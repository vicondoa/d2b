//! Transactional composition for authenticated clipboard services.
//!
//! Endpoint discovery and ComponentSession establishment are caller-owned.
//! This module accepts only negotiated session evidence, keeps policy, picker,
//! and transfer descriptors behind one lifecycle, and drops all bounded state
//! when any required session is lost.

pub mod bridge;
pub mod control;
pub mod picker;

use std::{
    collections::{BTreeMap, BTreeSet},
    os::fd::{AsFd, OwnedFd},
};

use crate::{
    audit::{AuditEvent, MetricEvent, MetricName, MetricsQueue},
    fd::{
        AcceptedTransferFdKind, AuthenticatedTransferOwner, ComponentSessionFdClaim, FdCapModel,
        FdSafetyError, validate_component_session_transfer_fd, validate_fd_cap,
    },
    framing::PickerProjectionBounds,
    policy::ReasonCode,
    protocol::{OpaquePickerId, PickerOffer},
};
use control::{
    AdmittedCall, ClipboardControl, ClipboardControlConfig, ControlError, ControlInput,
    ControlOutcome, ControlPeer, ControlResponse, ControlSession, ControlTransport,
};
use picker::{
    ClipboardPickerService, PickerCall, PickerOperation, PickerResponse, PickerServiceError,
    PickerSession, PickerTransport,
};

/// Transport selected by the established ComponentSession.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardSessionTransport {
    UnixStream,
    UnixSeqpacket,
}

/// Immutable evidence supplied by a ComponentSession endpoint adapter.
///
/// Implementations must expose negotiated session state, not request fields or
/// caller-provided presentation metadata.
pub trait EstablishedClipboardSession {
    fn service_package(&self) -> &str;
    fn endpoint_purpose(&self) -> &str;
    fn endpoint_role(&self) -> &str;
    fn generation(&self) -> u64;
    fn transport(&self) -> ClipboardSessionTransport;
    fn authenticated_realm(&self) -> Option<&str>;
    fn is_established(&self) -> bool;
    fn is_authenticated(&self) -> bool;
    fn is_host_local(&self) -> bool;
    fn uses_pre_authorized_transport(&self) -> bool;
    fn attachments_present(&self) -> bool;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardServicesConfig {
    pub control: ClipboardControlConfig,
    pub transfer_fd_cap: FdCapModel,
    pub metrics_capacity: usize,
}

impl Default for ClipboardServicesConfig {
    fn default() -> Self {
        Self {
            control: ClipboardControlConfig::default(),
            transfer_fd_cap: FdCapModel {
                requested_cap: 64,
                rlimit_nofile: 1_024,
                base_reserved: 64,
                max_fds_per_recvmsg: 1,
            },
            metrics_capacity: 1_024,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardServicePhase {
    Active,
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardServiceCloseReason {
    SessionUnavailable,
    Requested,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ClipboardStartupError {
    #[error("clipboard-session-not-established")]
    SessionNotEstablished,
    #[error("clipboard-session-unauthenticated")]
    Unauthenticated,
    #[error("clipboard-transport-untrusted")]
    UntrustedTransport,
    #[error("clipboard-session-contract-mismatch")]
    ContractMismatch,
    #[error("clipboard-session-generation-mismatch")]
    GenerationMismatch,
    #[error("clipboard-session-attachment-denied")]
    AttachmentDenied,
    #[error("clipboard-service-config-invalid")]
    InvalidConfig,
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum ClipboardServiceError {
    #[error("clipboard-services-unavailable")]
    Unavailable,
    #[error("clipboard-control-error: {0:?}")]
    Control(ControlError),
    #[error("clipboard-picker-error: {0}")]
    Picker(PickerServiceError),
    #[error("clipboard-transfer-fd-error: {0}")]
    TransferFd(FdSafetyError),
    #[error("clipboard-transfer-generation-mismatch")]
    TransferGenerationMismatch,
    #[error("clipboard-transfer-capacity-exhausted")]
    TransferCapacityExhausted,
    #[error("clipboard-transfer-not-found")]
    TransferNotFound,
    #[error("clipboard-transfer-not-authorized")]
    TransferNotAuthorized,
    #[error("clipboard-transfer-picker-confirmation-required")]
    PickerConfirmationRequired,
    #[error("clipboard-picker-offer-policy-denied: {0:?}")]
    PickerOfferPolicy(ReasonCode),
}

impl From<ControlError> for ClipboardServiceError {
    fn from(error: ControlError) -> Self {
        Self::Control(error)
    }
}

impl From<PickerServiceError> for ClipboardServiceError {
    fn from(error: PickerServiceError) -> Self {
        Self::Picker(error)
    }
}

impl From<FdSafetyError> for ClipboardServiceError {
    fn from(error: FdSafetyError) -> Self {
        Self::TransferFd(error)
    }
}

/// Opaque handle for a descriptor held by the authenticated service lifecycle.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct TransferHandle(u64);

impl std::fmt::Debug for TransferHandle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("TransferHandle(<redacted>)")
    }
}

struct ActiveClipboardServices {
    generation: u64,
    command_session: ControlSession,
    bridge_session: ControlSession,
    picker_session: PickerSession,
    control: ClipboardControl,
    picker: ClipboardPickerService,
    policy: crate::policy::ClipboardPolicy,
    transfer_fd_cap: usize,
    next_transfer_handle: u64,
    transfer_fds: BTreeMap<TransferHandle, OwnedFd>,
    policy_accepted_offers: BTreeSet<String>,
    picker_required_offers: BTreeSet<String>,
    picker_confirmed_offers: BTreeSet<String>,
    picker_selections: BTreeMap<OpaquePickerId, OpaquePickerId>,
    metrics: MetricsQueue,
}

/// Control, bridge, and picker state bound to one session generation.
///
/// Startup is transactional. No service is exposed until all three sessions,
/// the policy store, and the descriptor cap are admitted. Session loss drops
/// control receipts, picker state, and every held descriptor in one operation.
pub struct ClipboardServices {
    active: Option<ActiveClipboardServices>,
    close_reason: Option<ClipboardServiceCloseReason>,
}

#[derive(Debug, Clone, Copy)]
enum ControlTransition {
    Accepted,
    Closed,
}

impl std::fmt::Debug for ClipboardServices {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ClipboardServices")
            .field("phase", &self.phase())
            .field("close_reason", &self.close_reason)
            .finish()
    }
}

impl ClipboardServices {
    pub fn start<C, B, P>(
        control_session: &C,
        bridge_session: &B,
        picker_session: &P,
        config: ClipboardServicesConfig,
    ) -> Result<Self, ClipboardStartupError>
    where
        C: EstablishedClipboardSession,
        B: EstablishedClipboardSession,
        P: EstablishedClipboardSession,
    {
        validate_session(
            control_session,
            control::SERVICE_PACKAGE,
            control::ENDPOINT_PURPOSE,
            control::ENDPOINT_ROLE,
        )?;
        validate_session(
            bridge_session,
            bridge::SERVICE_PACKAGE,
            bridge::ENDPOINT_PURPOSE,
            bridge::ENDPOINT_ROLE,
        )?;
        validate_session(
            picker_session,
            picker::SERVICE_PACKAGE,
            picker::ENDPOINT_PURPOSE,
            picker::ENDPOINT_ROLE,
        )?;

        let generation = control_session.generation();
        if generation == 0
            || bridge_session.generation() != generation
            || picker_session.generation() != generation
        {
            return Err(ClipboardStartupError::GenerationMismatch);
        }
        if control_session.authenticated_realm().is_some()
            || bridge_session.authenticated_realm().is_none()
            || picker_session.authenticated_realm().is_some()
        {
            return Err(ClipboardStartupError::ContractMismatch);
        }
        if control_session.attachments_present()
            || picker_session.attachments_present()
            || bridge_session.attachments_present()
        {
            return Err(ClipboardStartupError::AttachmentDenied);
        }

        let command_session = ControlSession::admit(
            generation,
            ControlPeer::CommandClient,
            None,
            control_transport(control_session.transport()),
            true,
            true,
        )
        .map_err(map_control_startup)?;
        let bridge_control_session = ControlSession::admit(
            generation,
            ControlPeer::ClipboardBridge,
            bridge_session.authenticated_realm(),
            control_transport(bridge_session.transport()),
            true,
            true,
        )
        .map_err(map_control_startup)?;
        let picker_control_session = PickerSession::admit(
            generation,
            picker_transport(picker_session.transport()),
            true,
            true,
            false,
        )
        .map_err(map_picker_startup)?;

        if config.metrics_capacity == 0 {
            return Err(ClipboardStartupError::InvalidConfig);
        }
        let validated_cap = validate_fd_cap(config.transfer_fd_cap)
            .map_err(|_| ClipboardStartupError::InvalidConfig)?;
        let transfer_fd_cap =
            usize::try_from(validated_cap).map_err(|_| ClipboardStartupError::InvalidConfig)?;
        if transfer_fd_cap == 0 || transfer_fd_cap > config.control.policy.max_held_transfers {
            return Err(ClipboardStartupError::InvalidConfig);
        }
        let policy = config.control.policy.clone();
        let control = ClipboardControl::new(config.control)
            .map_err(|_| ClipboardStartupError::InvalidConfig)?;

        Ok(Self {
            active: Some(ActiveClipboardServices {
                generation,
                command_session,
                bridge_session: bridge_control_session,
                picker_session: picker_control_session,
                control,
                picker: ClipboardPickerService::default(),
                policy,
                transfer_fd_cap,
                next_transfer_handle: 1,
                transfer_fds: BTreeMap::new(),
                policy_accepted_offers: BTreeSet::new(),
                picker_required_offers: BTreeSet::new(),
                picker_confirmed_offers: BTreeSet::new(),
                picker_selections: BTreeMap::new(),
                metrics: MetricsQueue::new(config.metrics_capacity),
            }),
            close_reason: None,
        })
    }

    pub fn phase(&self) -> ClipboardServicePhase {
        if self.active.is_some() {
            ClipboardServicePhase::Active
        } else {
            ClipboardServicePhase::Closed
        }
    }

    pub fn close_reason(&self) -> Option<ClipboardServiceCloseReason> {
        self.close_reason
    }

    pub fn generation(&self) -> Result<u64, ClipboardServiceError> {
        Ok(self.active()?.generation)
    }

    pub fn handle_control(
        &mut self,
        peer: ControlPeer,
        call: AdmittedCall,
        input: ControlInput,
        now_unix_ms: u64,
    ) -> Result<ControlResponse, ClipboardServiceError> {
        let active = self.active_mut()?;
        let transition = match &input {
            ControlInput::AcceptTransfer { offer_id } => {
                Some((offer_id.clone(), ControlTransition::Accepted))
            }
            ControlInput::CompleteTransfer { offer_id }
            | ControlInput::CancelTransfer { offer_id } => {
                Some((offer_id.clone(), ControlTransition::Closed))
            }
            _ => None,
        };
        let session = match peer {
            ControlPeer::CommandClient => active.command_session.clone(),
            ControlPeer::ClipboardBridge => active.bridge_session.clone(),
        };
        let response = active
            .control
            .handle(session, call, input, now_unix_ms)
            .map_err(|error| {
                match error {
                    ControlError::Policy(ReasonCode::AuditFailure) => {
                        active.metrics.enqueue_droppable(MetricEvent {
                            name: MetricName::AuditQueueOverflow,
                            reason: Some(ReasonCode::AuditFailure),
                        });
                    }
                    ControlError::Policy(reason) => {
                        active.metrics.enqueue_droppable(MetricEvent {
                            name: MetricName::PolicyDenied,
                            reason: Some(reason),
                        });
                    }
                    _ => {}
                }
                ClipboardServiceError::from(error)
            })?;
        if response.outcome == ControlOutcome::Denied {
            active.metrics.enqueue_droppable(MetricEvent {
                name: MetricName::PolicyDenied,
                reason: Some(response.reason),
            });
        }
        if matches!(
            response.outcome,
            ControlOutcome::Succeeded | ControlOutcome::AlreadyApplied
        ) && let Some((offer_id, transition)) = transition
        {
            match transition {
                ControlTransition::Accepted => {
                    active.policy_accepted_offers.insert(offer_id);
                }
                ControlTransition::Closed => {
                    active.policy_accepted_offers.remove(&offer_id);
                }
            }
        }
        Ok(response)
    }

    pub fn publish_picker_offer(
        &mut self,
        offer: PickerOffer,
    ) -> Result<(), ClipboardServiceError> {
        let active = self.active_mut()?;
        let Some(byte_count) = offer.byte_count() else {
            active.metrics.enqueue_droppable(MetricEvent {
                name: MetricName::PolicyDenied,
                reason: Some(ReasonCode::MemoryCapExceeded),
            });
            return Err(ClipboardServiceError::PickerOfferPolicy(
                ReasonCode::MemoryCapExceeded,
            ));
        };
        if let Err(reason) = active.policy.validate_offer(byte_count, offer.mime_type()) {
            active.metrics.enqueue_droppable(MetricEvent {
                name: MetricName::PolicyDenied,
                reason: Some(reason),
            });
            return Err(ClipboardServiceError::PickerOfferPolicy(reason));
        }
        let offer_id = offer.offer_id().as_str().to_owned();
        let confirmation_required = offer.confirmation_required();
        active.picker.publish_offer(offer)?;
        if confirmation_required {
            active.picker_required_offers.insert(offer_id);
        } else {
            active.picker_required_offers.remove(&offer_id);
            active.picker_confirmed_offers.remove(&offer_id);
        }
        Ok(())
    }

    pub fn remove_picker_offer(
        &mut self,
        offer_id: &OpaquePickerId,
    ) -> Result<(), ClipboardServiceError> {
        let active = self.active_mut()?;
        active.picker.remove_offer(offer_id);
        active.picker_required_offers.remove(offer_id.as_str());
        active.picker_confirmed_offers.remove(offer_id.as_str());
        active
            .picker_selections
            .retain(|_, selected_offer| selected_offer != offer_id);
        Ok(())
    }

    pub fn handle_picker(
        &mut self,
        call: &PickerCall,
        projection: PickerProjectionBounds,
        operation: PickerOperation,
        now_unix_ms: u64,
    ) -> Result<PickerResponse, ClipboardServiceError> {
        let active = self.active_mut()?;
        if let PickerOperation::Select(selection) = &operation
            && !active
                .picker_selections
                .contains_key(&selection.selection_id)
            && active.picker_selections.len() >= active.transfer_fd_cap
        {
            return Err(ClipboardServiceError::Picker(
                PickerServiceError::ResourceExhausted,
            ));
        }
        let cancelled_selection = match &operation {
            PickerOperation::CancelSelection(cancel) => Some(cancel.selection_id.clone()),
            PickerOperation::Cancel(selection_id) => Some(selection_id.clone()),
            _ => None,
        };
        let picker_opened = matches!(operation, PickerOperation::List(_));
        let response = active
            .picker
            .invoke(
                &active.picker_session,
                call,
                projection,
                operation,
                now_unix_ms,
            )
            .map_err(|error| {
                if error == PickerServiceError::DeadlineExpired {
                    active.metrics.enqueue_droppable(MetricEvent {
                        name: MetricName::PickerTimeout,
                        reason: Some(ReasonCode::PickerTimeout),
                    });
                }
                ClipboardServiceError::from(error)
            })?;
        if picker_opened {
            active.metrics.enqueue_droppable(MetricEvent {
                name: MetricName::PickerOpened,
                reason: None,
            });
        }
        if let PickerResponse::Selected(receipt) = &response {
            active
                .picker_confirmed_offers
                .insert(receipt.offer_id.as_str().to_owned());
            active
                .picker_selections
                .insert(receipt.selection_id.clone(), receipt.offer_id.clone());
        } else if let Some(selection_id) = cancelled_selection
            && let Some(offer_id) = active.picker_selections.remove(&selection_id)
            && !active
                .picker_selections
                .values()
                .any(|selected_offer| selected_offer == &offer_id)
        {
            active.picker_confirmed_offers.remove(offer_id.as_str());
        }
        Ok(response)
    }

    pub fn accept_transfer_fd(
        &mut self,
        offer_id: &OpaquePickerId,
        fd: OwnedFd,
        owner: &AuthenticatedTransferOwner,
        claim: &ComponentSessionFdClaim,
    ) -> Result<(TransferHandle, AcceptedTransferFdKind), ClipboardServiceError> {
        let active = self.active_mut()?;
        if claim.reconnect_generation != active.generation {
            return Err(ClipboardServiceError::TransferGenerationMismatch);
        }
        if active.transfer_fds.len() >= active.transfer_fd_cap {
            return Err(ClipboardServiceError::TransferCapacityExhausted);
        }
        if !active.policy_accepted_offers.contains(offer_id.as_str()) {
            return Err(ClipboardServiceError::TransferNotAuthorized);
        }
        if active.picker_required_offers.contains(offer_id.as_str())
            && !active.picker_confirmed_offers.contains(offer_id.as_str())
        {
            return Err(ClipboardServiceError::PickerConfirmationRequired);
        }
        let kind = validate_component_session_transfer_fd(fd.as_fd(), owner, claim)?;
        let handle = TransferHandle(active.next_transfer_handle);
        active.next_transfer_handle = active
            .next_transfer_handle
            .checked_add(1)
            .ok_or(ClipboardServiceError::TransferCapacityExhausted)?;
        active.transfer_fds.insert(handle, fd);
        active.policy_accepted_offers.remove(offer_id.as_str());
        active.picker_confirmed_offers.remove(offer_id.as_str());
        Ok((handle, kind))
    }

    pub fn transfer_fd(&self, handle: TransferHandle) -> Result<&OwnedFd, ClipboardServiceError> {
        self.active()?
            .transfer_fds
            .get(&handle)
            .ok_or(ClipboardServiceError::TransferNotFound)
    }

    pub fn finish_transfer(&mut self, handle: TransferHandle) -> Result<(), ClipboardServiceError> {
        self.active_mut()?
            .transfer_fds
            .remove(&handle)
            .map(drop)
            .ok_or(ClipboardServiceError::TransferNotFound)
    }

    pub fn active_transfer_count(&self) -> Result<usize, ClipboardServiceError> {
        Ok(self.active()?.transfer_fds.len())
    }

    pub fn drain_audit(
        &mut self,
        max_events: usize,
    ) -> Result<Vec<AuditEvent>, ClipboardServiceError> {
        Ok(self.active_mut()?.control.drain_audit(max_events))
    }

    pub fn drain_metrics(
        &mut self,
        max_events: usize,
    ) -> Result<(Vec<MetricEvent>, u64), ClipboardServiceError> {
        let active = self.active_mut()?;
        let dropped = active.metrics.take_dropped_count();
        Ok((active.metrics.drain_bounded(max_events), dropped))
    }

    pub fn session_unavailable(&mut self) {
        self.close(ClipboardServiceCloseReason::SessionUnavailable);
    }

    pub fn shutdown(&mut self) {
        self.close(ClipboardServiceCloseReason::Requested);
    }

    fn active(&self) -> Result<&ActiveClipboardServices, ClipboardServiceError> {
        self.active
            .as_ref()
            .ok_or(ClipboardServiceError::Unavailable)
    }

    fn active_mut(&mut self) -> Result<&mut ActiveClipboardServices, ClipboardServiceError> {
        self.active
            .as_mut()
            .ok_or(ClipboardServiceError::Unavailable)
    }

    fn close(&mut self, reason: ClipboardServiceCloseReason) {
        if self.active.take().is_some() {
            self.close_reason = Some(reason);
        }
    }
}

fn validate_session<S: EstablishedClipboardSession>(
    session: &S,
    package: &str,
    purpose: &str,
    role: &str,
) -> Result<(), ClipboardStartupError> {
    if !session.is_established() {
        return Err(ClipboardStartupError::SessionNotEstablished);
    }
    if !session.is_authenticated() || !session.is_host_local() {
        return Err(ClipboardStartupError::Unauthenticated);
    }
    if !session.uses_pre_authorized_transport() {
        return Err(ClipboardStartupError::UntrustedTransport);
    }
    if session.service_package() != package
        || session.endpoint_purpose() != purpose
        || session.endpoint_role() != role
    {
        return Err(ClipboardStartupError::ContractMismatch);
    }
    Ok(())
}

const fn control_transport(transport: ClipboardSessionTransport) -> ControlTransport {
    match transport {
        ClipboardSessionTransport::UnixStream => ControlTransport::ComponentSessionUnixStream,
        ClipboardSessionTransport::UnixSeqpacket => ControlTransport::ComponentSessionUnixSeqpacket,
    }
}

const fn picker_transport(transport: ClipboardSessionTransport) -> PickerTransport {
    match transport {
        ClipboardSessionTransport::UnixStream => PickerTransport::ComponentSessionUnixStream,
        ClipboardSessionTransport::UnixSeqpacket => PickerTransport::ComponentSessionUnixSeqpacket,
    }
}

fn map_control_startup(error: ControlError) -> ClipboardStartupError {
    match error {
        ControlError::UnauthenticatedSession => ClipboardStartupError::Unauthenticated,
        ControlError::GenerationMismatch => ClipboardStartupError::GenerationMismatch,
        ControlError::Unauthorized => ClipboardStartupError::ContractMismatch,
        _ => ClipboardStartupError::InvalidConfig,
    }
}

fn map_picker_startup(error: PickerServiceError) -> ClipboardStartupError {
    match error {
        PickerServiceError::Unauthenticated => ClipboardStartupError::Unauthenticated,
        PickerServiceError::GenerationMismatch => ClipboardStartupError::GenerationMismatch,
        PickerServiceError::AttachmentDenied => ClipboardStartupError::AttachmentDenied,
        _ => ClipboardStartupError::InvalidConfig,
    }
}

#[cfg(test)]
mod tests {
    use d2b_realm_core::WorkloadProviderKind;
    use rustix::pipe::{PipeFlags, pipe_with};

    use super::*;
    use crate::{
        protocol::{
            AttributionQuality, CapabilityPreflight, ClipboardTarget, OfferQuery, OfferSelection,
            PickerOfferInput,
        },
        services::control::{ControlOutcome, OfferIntent},
    };

    #[derive(Clone)]
    struct Session {
        package: &'static str,
        purpose: &'static str,
        role: &'static str,
        generation: u64,
        realm: Option<&'static str>,
        established: bool,
        authenticated: bool,
        host_local: bool,
        pre_authorized: bool,
        attachments: bool,
    }

    impl EstablishedClipboardSession for Session {
        fn service_package(&self) -> &str {
            self.package
        }

        fn endpoint_purpose(&self) -> &str {
            self.purpose
        }

        fn endpoint_role(&self) -> &str {
            self.role
        }

        fn generation(&self) -> u64 {
            self.generation
        }

        fn transport(&self) -> ClipboardSessionTransport {
            ClipboardSessionTransport::UnixSeqpacket
        }

        fn authenticated_realm(&self) -> Option<&str> {
            self.realm
        }

        fn is_established(&self) -> bool {
            self.established
        }

        fn is_authenticated(&self) -> bool {
            self.authenticated
        }

        fn is_host_local(&self) -> bool {
            self.host_local
        }

        fn uses_pre_authorized_transport(&self) -> bool {
            self.pre_authorized
        }

        fn attachments_present(&self) -> bool {
            self.attachments
        }
    }

    fn session(package: &'static str, purpose: &'static str, role: &'static str) -> Session {
        Session {
            package,
            purpose,
            role,
            generation: 7,
            realm: None,
            established: true,
            authenticated: true,
            host_local: true,
            pre_authorized: true,
            attachments: false,
        }
    }

    fn sessions() -> (Session, Session, Session) {
        let control = session(
            control::SERVICE_PACKAGE,
            control::ENDPOINT_PURPOSE,
            control::ENDPOINT_ROLE,
        );
        let mut bridge = session(
            bridge::SERVICE_PACKAGE,
            bridge::ENDPOINT_PURPOSE,
            bridge::ENDPOINT_ROLE,
        );
        bridge.realm = Some("work");
        let picker = session(
            picker::SERVICE_PACKAGE,
            picker::ENDPOINT_PURPOSE,
            picker::ENDPOINT_ROLE,
        );
        (control, bridge, picker)
    }

    fn services() -> ClipboardServices {
        let (control, bridge, picker) = sessions();
        ClipboardServices::start(
            &control,
            &bridge,
            &picker,
            ClipboardServicesConfig::default(),
        )
        .unwrap()
    }

    fn call(id: u8) -> AdmittedCall {
        AdmittedCall::new([id; 16], Some(&[id; 16]), 7, 1_000, 9_000).unwrap()
    }

    fn picker_offer(byte_count: Option<u64>) -> PickerOffer {
        PickerOffer::new(PickerOfferInput {
            offer_id: OpaquePickerId::parse("offer-1").unwrap(),
            source: ClipboardTarget::Host,
            destination: ClipboardTarget::workload(
                "workload.work.d2b",
                WorkloadProviderKind::LocalVm,
            )
            .unwrap(),
            mime_type: "text/plain".to_owned(),
            preview: Some("safe preview".to_owned()),
            thumbnail_png: None,
            source_application: None,
            attribution: AttributionQuality::ExactClient,
            capability_preflight: CapabilityPreflight::Satisfied,
            byte_count,
            observed_at_unix_ms: 1_000,
            expires_at_unix_ms: 9_000,
            confirmation_required: true,
        })
        .unwrap()
    }

    fn projection() -> PickerProjectionBounds {
        PickerProjectionBounds {
            encoded_bytes: 1_024,
            offer_count: 4,
            thumbnail_bytes: 0,
            attachment_count: 0,
        }
    }

    fn authorize_transfer(services: &mut ClipboardServices) {
        services
            .handle_control(
                ControlPeer::ClipboardBridge,
                call(3),
                ControlInput::Offer(OfferIntent {
                    offer_id: "offer-1".to_owned(),
                    operation_id: "operation-1".to_owned(),
                    source_realm: "work".to_owned(),
                    destination_realm: "personal".to_owned(),
                    mime_type: "text/plain".to_owned(),
                    byte_count: 12,
                    request_digest: [9; 32],
                    expires_at_unix_ms: 9_000,
                    explicit_cross_realm_allow: true,
                    trusted_paste_intent: true,
                }),
                2_000,
            )
            .unwrap();
        services
            .handle_control(
                ControlPeer::CommandClient,
                call(4),
                ControlInput::AcceptTransfer {
                    offer_id: "offer-1".to_owned(),
                },
                2_000,
            )
            .unwrap();
    }

    #[test]
    fn starts_control_bridge_and_picker_transactionally() {
        let mut services = services();
        let response = services
            .handle_control(
                ControlPeer::ClipboardBridge,
                call(1),
                ControlInput::Offer(OfferIntent {
                    offer_id: "offer-1".to_owned(),
                    operation_id: "operation-1".to_owned(),
                    source_realm: "work".to_owned(),
                    destination_realm: "personal".to_owned(),
                    mime_type: "text/plain".to_owned(),
                    byte_count: 12,
                    request_digest: [9; 32],
                    expires_at_unix_ms: 9_000,
                    explicit_cross_realm_allow: true,
                    trusted_paste_intent: true,
                }),
                2_000,
            )
            .unwrap();
        assert_eq!(response.outcome, ControlOutcome::Succeeded);

        let offer = picker_offer(Some(12));
        let destination = offer.destination().clone();
        services.publish_picker_offer(offer).unwrap();
        let response = services
            .handle_picker(
                &PickerCall::new([2; 16], None, 7, 1_000, 9_000).unwrap(),
                projection(),
                PickerOperation::List(OfferQuery::new(destination, 4).unwrap()),
                2_000,
            )
            .unwrap();
        assert!(matches!(response, PickerResponse::Offers(offers) if offers.len() == 1));
        assert_eq!(services.phase(), ClipboardServicePhase::Active);
    }

    #[test]
    fn startup_rejects_untrusted_or_incomplete_session_sets() {
        let (control, bridge, picker) = sessions();
        let cases = [
            (
                Session {
                    established: false,
                    ..control.clone()
                },
                ClipboardStartupError::SessionNotEstablished,
            ),
            (
                Session {
                    authenticated: false,
                    ..control.clone()
                },
                ClipboardStartupError::Unauthenticated,
            ),
            (
                Session {
                    pre_authorized: false,
                    ..control.clone()
                },
                ClipboardStartupError::UntrustedTransport,
            ),
            (
                Session {
                    purpose: bridge::ENDPOINT_PURPOSE,
                    ..control.clone()
                },
                ClipboardStartupError::ContractMismatch,
            ),
        ];
        for (invalid, expected) in cases {
            assert_eq!(
                ClipboardServices::start(
                    &invalid,
                    &bridge,
                    &picker,
                    ClipboardServicesConfig::default(),
                )
                .unwrap_err(),
                expected
            );
        }

        let wrong_generation = Session {
            generation: 8,
            ..picker.clone()
        };
        assert_eq!(
            ClipboardServices::start(
                &control,
                &bridge,
                &wrong_generation,
                ClipboardServicesConfig::default(),
            )
            .unwrap_err(),
            ClipboardStartupError::GenerationMismatch
        );
    }

    #[test]
    fn descriptor_admission_is_exact_bounded_and_generation_bound() {
        let mut config = ClipboardServicesConfig::default();
        config.transfer_fd_cap.requested_cap = 1;
        let (control, bridge, picker) = sessions();
        let mut services = ClipboardServices::start(&control, &bridge, &picker, config).unwrap();
        authorize_transfer(&mut services);
        let offer_id = OpaquePickerId::parse("offer-1").unwrap();
        let owner =
            AuthenticatedTransferOwner::from_component_session([4; 16], [2; 16], 7).unwrap();
        let claim = ComponentSessionFdClaim {
            request_id: [4; 16],
            operation_id: [2; 16],
            reconnect_generation: 7,
            packet_sequence: 1,
            descriptor_index: 0,
            descriptor_count: 1,
            packet_atomic: true,
            cloexec_required: true,
        };
        let (read_end, _write_end) = pipe_with(PipeFlags::CLOEXEC).unwrap();
        let (handle, kind) = services
            .accept_transfer_fd(&offer_id, read_end, &owner, &claim)
            .unwrap();
        assert_eq!(kind, AcceptedTransferFdKind::Pipe);
        assert_eq!(services.active_transfer_count().unwrap(), 1);
        assert!(services.transfer_fd(handle).is_ok());

        let (second_read, _second_write) = pipe_with(PipeFlags::CLOEXEC).unwrap();
        assert_eq!(
            services
                .accept_transfer_fd(&offer_id, second_read, &owner, &claim)
                .unwrap_err(),
            ClipboardServiceError::TransferCapacityExhausted
        );
        services.finish_transfer(handle).unwrap();

        let wrong_generation = ComponentSessionFdClaim {
            reconnect_generation: 8,
            ..claim
        };
        let (third_read, _third_write) = pipe_with(PipeFlags::CLOEXEC).unwrap();
        assert_eq!(
            services
                .accept_transfer_fd(&offer_id, third_read, &owner, &wrong_generation)
                .unwrap_err(),
            ClipboardServiceError::TransferGenerationMismatch
        );
    }

    #[test]
    fn picker_projection_cannot_bypass_transfer_policy() {
        let mut services = services();
        assert_eq!(
            services
                .publish_picker_offer(picker_offer(None))
                .unwrap_err(),
            ClipboardServiceError::PickerOfferPolicy(ReasonCode::MemoryCapExceeded)
        );
        assert_eq!(
            services
                .publish_picker_offer(picker_offer(Some(9 * 1024 * 1024)))
                .unwrap_err(),
            ClipboardServiceError::PickerOfferPolicy(ReasonCode::MemoryCapExceeded)
        );
    }

    #[test]
    fn required_picker_confirmation_gates_policy_accepted_descriptor() {
        let mut services = services();
        authorize_transfer(&mut services);
        let offer = picker_offer(Some(12));
        let offer_id = offer.offer_id().clone();
        let destination = offer.destination().clone();
        services.publish_picker_offer(offer).unwrap();
        let owner =
            AuthenticatedTransferOwner::from_component_session([4; 16], [2; 16], 7).unwrap();
        let claim = ComponentSessionFdClaim {
            request_id: [4; 16],
            operation_id: [2; 16],
            reconnect_generation: 7,
            packet_sequence: 1,
            descriptor_index: 0,
            descriptor_count: 1,
            packet_atomic: true,
            cloexec_required: true,
        };
        let (unconfirmed_read, _unconfirmed_write) = pipe_with(PipeFlags::CLOEXEC).unwrap();
        assert_eq!(
            services
                .accept_transfer_fd(&offer_id, unconfirmed_read, &owner, &claim)
                .unwrap_err(),
            ClipboardServiceError::PickerConfirmationRequired
        );

        services
            .handle_picker(
                &PickerCall::new([5; 16], Some(&[5; 16]), 7, 1_000, 9_000).unwrap(),
                projection(),
                PickerOperation::Select(
                    OfferSelection::new("selection-1", offer_id.as_str(), destination).unwrap(),
                ),
                2_000,
            )
            .unwrap();
        let (confirmed_read, _confirmed_write) = pipe_with(PipeFlags::CLOEXEC).unwrap();
        services
            .accept_transfer_fd(&offer_id, confirmed_read, &owner, &claim)
            .unwrap();
    }

    #[test]
    fn session_loss_drops_all_service_and_descriptor_state() {
        let mut services = services();
        authorize_transfer(&mut services);
        let offer_id = OpaquePickerId::parse("offer-1").unwrap();
        let owner =
            AuthenticatedTransferOwner::from_component_session([4; 16], [2; 16], 7).unwrap();
        let claim = ComponentSessionFdClaim {
            request_id: [4; 16],
            operation_id: [2; 16],
            reconnect_generation: 7,
            packet_sequence: 1,
            descriptor_index: 0,
            descriptor_count: 1,
            packet_atomic: true,
            cloexec_required: true,
        };
        let (read_end, _write_end) = pipe_with(PipeFlags::CLOEXEC).unwrap();
        services
            .accept_transfer_fd(&offer_id, read_end, &owner, &claim)
            .unwrap();
        services
            .publish_picker_offer(picker_offer(Some(12)))
            .unwrap();

        services.session_unavailable();

        assert_eq!(services.phase(), ClipboardServicePhase::Closed);
        assert_eq!(
            services.close_reason(),
            Some(ClipboardServiceCloseReason::SessionUnavailable)
        );
        assert_eq!(
            services.active_transfer_count().unwrap_err(),
            ClipboardServiceError::Unavailable
        );
        assert_eq!(
            services
                .handle_picker(
                    &PickerCall::new([2; 16], None, 7, 1_000, 9_000).unwrap(),
                    projection(),
                    PickerOperation::List(OfferQuery::new(ClipboardTarget::Host, 1).unwrap()),
                    2_000,
                )
                .unwrap_err(),
            ClipboardServiceError::Unavailable
        );

        services.shutdown();
        assert_eq!(
            services.close_reason(),
            Some(ClipboardServiceCloseReason::SessionUnavailable)
        );
    }

    #[test]
    fn debug_output_exposes_only_lifecycle_state() {
        let mut services = services();
        assert_eq!(
            format!("{services:?}"),
            "ClipboardServices { phase: Active, close_reason: None }"
        );
        services.shutdown();
        assert_eq!(
            format!("{services:?}"),
            "ClipboardServices { phase: Closed, close_reason: Some(Requested) }"
        );
    }

    #[test]
    fn composition_exposes_bounded_audit_and_closed_metric_events() {
        let mut config = ClipboardServicesConfig::default();
        config.control.audit_per_realm_quota = 1;
        let (control, bridge, picker) = sessions();
        let mut services = ClipboardServices::start(&control, &bridge, &picker, config).unwrap();

        for (id, request) in [(1, 21), (2, 22)] {
            services
                .handle_control(
                    ControlPeer::ClipboardBridge,
                    call(request),
                    ControlInput::Offer(OfferIntent {
                        offer_id: format!("offer-{id}"),
                        operation_id: format!("operation-{id}"),
                        source_realm: "work".to_owned(),
                        destination_realm: "personal".to_owned(),
                        mime_type: "text/plain".to_owned(),
                        byte_count: 12,
                        request_digest: [id; 32],
                        expires_at_unix_ms: 9_000,
                        explicit_cross_realm_allow: true,
                        trusted_paste_intent: true,
                    }),
                    2_000,
                )
                .unwrap();
        }
        services
            .handle_control(
                ControlPeer::CommandClient,
                call(23),
                ControlInput::AcceptTransfer {
                    offer_id: "offer-1".to_owned(),
                },
                2_000,
            )
            .unwrap();
        assert_eq!(
            services
                .handle_control(
                    ControlPeer::CommandClient,
                    call(24),
                    ControlInput::AcceptTransfer {
                        offer_id: "offer-2".to_owned(),
                    },
                    2_000,
                )
                .unwrap_err(),
            ClipboardServiceError::Control(ControlError::Policy(ReasonCode::AuditFailure))
        );

        assert_eq!(services.drain_audit(1).unwrap().len(), 1);
        assert!(services.drain_audit(1).unwrap().is_empty());
        assert_eq!(
            services
                .handle_control(
                    ControlPeer::CommandClient,
                    call(25),
                    ControlInput::AcceptTransfer {
                        offer_id: "offer-2".to_owned(),
                    },
                    2_000,
                )
                .unwrap()
                .outcome,
            ControlOutcome::Succeeded
        );

        services
            .handle_picker(
                &PickerCall::new([31; 16], None, 7, 1_000, 9_000).unwrap(),
                projection(),
                PickerOperation::List(OfferQuery::new(ClipboardTarget::Host, 1).unwrap()),
                2_000,
            )
            .unwrap();
        assert_eq!(
            services
                .handle_picker(
                    &PickerCall::new([32; 16], None, 7, 1_000, 1_500).unwrap(),
                    projection(),
                    PickerOperation::List(OfferQuery::new(ClipboardTarget::Host, 1).unwrap()),
                    2_000,
                )
                .unwrap_err(),
            ClipboardServiceError::Picker(PickerServiceError::DeadlineExpired)
        );
        assert!(matches!(
            services.publish_picker_offer(picker_offer(Some(9 * 1024 * 1024))),
            Err(ClipboardServiceError::PickerOfferPolicy(_))
        ));

        let (first, dropped) = services.drain_metrics(2).unwrap();
        assert_eq!(dropped, 0);
        assert_eq!(first.len(), 2);
        let (remaining, dropped) = services.drain_metrics(8).unwrap();
        assert_eq!(dropped, 0);
        let names = first
            .into_iter()
            .chain(remaining)
            .map(|event| event.name)
            .collect::<Vec<_>>();
        assert!(names.contains(&MetricName::AuditQueueOverflow));
        assert!(names.contains(&MetricName::PickerOpened));
        assert!(names.contains(&MetricName::PickerTimeout));
        assert!(names.contains(&MetricName::PolicyDenied));
    }
}
