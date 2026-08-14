//! clipd-host service boundary and display dependency.

use crate::{
    DependencyStatus, DisplayDependencyEvidence,
    audit::{ClipboardAuditEvent, ClipboardAuditQueue, ClipboardReason, SizeBucket},
    fd::{AttachmentClass, FdSafetyError, ReceivedFdBatch},
    history::{ClipboardEntry, ClipboardHistory},
    picker::PickerReceipt,
    policy::Policy,
};
use d2b_contracts::v3::{ResourceRef, ZoneId};
use d2b_session::{AuthenticatedComponentSession, AuthenticatedSessionRouteBinding};
use d2b_session_unix::{AcceptedAttachment, VerifiedPacket};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

/// A routing identity projected from an authenticated ComponentSession.
///
/// The only public constructor consumes the canonical session route binding,
/// whose fields are private and can only be obtained from a verified session.
#[derive(PartialEq, Eq)]
pub struct AuthenticatedClipboardSession {
    subject_ref: ResourceRef,
    zone: ZoneId,
    reconnect_generation: u64,
}

impl AuthenticatedClipboardSession {
    /// Derive clipboard identity from a verified ComponentSession.
    pub fn from_component_session<C>(
        session: &AuthenticatedComponentSession<C>,
    ) -> Result<Self, ClipboardServiceError> {
        Self::from_route_binding(session.route_binding())
    }

    /// Derive clipboard identity from a canonical authenticated route.
    pub(crate) fn from_route_binding(
        route: AuthenticatedSessionRouteBinding,
    ) -> Result<Self, ClipboardServiceError> {
        let provider_matches = route
            .provider_ref()
            .is_some_and(|provider| provider.to_canonical_string() == crate::PROVIDER_REF);
        let service_matches = matches!(
            route.service().as_str(),
            crate::MANAGEMENT_SERVICE | crate::BRIDGE_SERVICE | crate::PICKER_SERVICE
        );
        if !provider_matches || !service_matches || route.provider_generation().is_none() {
            return Err(ClipboardServiceError::SessionUnauthenticated);
        }
        let subject_type = route.subject_ref().resource_type().as_str();
        if !matches!(subject_type, "Guest" | "User" | "Provider") {
            return Err(ClipboardServiceError::SessionUnauthenticated);
        }
        Ok(Self {
            subject_ref: route.subject_ref().clone(),
            zone: route.zone().clone(),
            reconnect_generation: route.reconnect_generation().get(),
        })
    }

    /// Borrow the authenticated subject reference.
    pub fn subject_ref(&self) -> &ResourceRef {
        &self.subject_ref
    }

    /// Borrow the authenticated Zone.
    pub fn zone(&self) -> &str {
        self.zone.as_str()
    }

    /// Borrow the authenticated Guest/User/Provider identity.
    pub fn guest_ref(&self) -> String {
        self.subject_ref.to_canonical_string()
    }

    /// Return the reconnect generation used for replay fencing.
    pub const fn reconnect_generation(&self) -> u64 {
        self.reconnect_generation
    }

    /// Whether the subject is a Guest.
    pub fn is_guest(&self) -> bool {
        self.subject_ref.resource_type().as_str() == "Guest"
    }
}

impl core::fmt::Debug for AuthenticatedClipboardSession {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("AuthenticatedClipboardSession(REDACTED)")
    }
}

/// A paste route bound to two authenticated session identities.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthenticatedPasteRoute {
    operation_id: String,
    source_zone: String,
    source_reconnect_generation: u64,
    destination_zone: String,
    destination_guest: String,
    reconnect_generation: u64,
}

impl AuthenticatedPasteRoute {
    /// Bind a source and destination session without accepting lexical IDs.
    pub fn from_sessions(
        source: &AuthenticatedClipboardSession,
        destination: &AuthenticatedClipboardSession,
    ) -> Result<Self, ClipboardServiceError> {
        if !destination.is_guest() {
            return Err(ClipboardServiceError::SessionUnauthenticated);
        }
        Ok(Self {
            operation_id: operation_id_for_sessions(source, destination),
            source_zone: source.zone().to_owned(),
            source_reconnect_generation: source.reconnect_generation(),
            destination_zone: destination.zone().to_owned(),
            destination_guest: destination.subject_ref.to_canonical_string(),
            reconnect_generation: destination.reconnect_generation(),
        })
    }

    /// Borrow the operation binding minted for this authenticated route.
    pub fn operation_id(&self) -> &str {
        &self.operation_id
    }

    pub(crate) fn source_zone(&self) -> &str {
        &self.source_zone
    }

    pub(crate) fn destination_zone(&self) -> &str {
        &self.destination_zone
    }

    pub(crate) fn source_reconnect_generation(&self) -> u64 {
        self.source_reconnect_generation
    }

    pub(crate) fn destination_guest(&self) -> &str {
        &self.destination_guest
    }

    pub(crate) fn reconnect_generation(&self) -> u64 {
        self.reconnect_generation
    }
}

/// Typed display dependency observation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayDependency {
    /// Whether the display Provider is absent or Ready.
    pub status: DependencyStatus,
    /// The typed service contract consumed by clipd-host.
    pub service_contract: &'static str,
    /// Authenticated display evidence, when the dependency is Ready.
    pub evidence: Option<DisplayDependencyEvidence>,
}

/// Authenticated Guest-selection metadata used to suppress a host echo.
///
/// The receipt is issued only for a live entry owned by the supplied Guest
/// session and is consumed by host capture.  Clipboard bytes never cross this
/// boundary.
pub struct GuestSelectionEvent {
    source_zone: ZoneId,
    source_guest: ResourceRef,
    source_generation: u64,
    entry_digest: String,
    expires_at: u64,
}

impl core::fmt::Debug for GuestSelectionEvent {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("GuestSelectionEvent(REDACTED)")
    }
}

/// Bridge effect port. Clipboard payloads are represented by attachments in
/// the real adapter; this trait never accepts a path or a raw compositor
/// socket.
pub trait ClipboardBridgePort {
    /// Notify the display bridge of a Guest selection without payload bytes.
    fn notify_guest_selection(
        &mut self,
        guest: &str,
        mime: &str,
    ) -> Result<(), ClipboardServiceError>;
    /// Cancel one opaque entry.
    fn cancel_entry(&mut self, token: &str) -> Result<(), ClipboardServiceError>;
}

/// Service failures with stable content-free codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardServiceError {
    /// Display dependency is absent or not Ready.
    DependencyUnavailable,
    /// Cross-Zone transfer is denied.
    CrossZoneDenied,
    /// Guest is suspended.
    GuestSuspended,
    /// Audit queue is full.
    AuditUnavailable,
    /// History rejected the item.
    HistoryRejected,
    /// A picker is required before materialization.
    PickerRequired,
    /// The ComponentSession route was not authenticated for this Provider.
    SessionUnauthenticated,
    /// A one-use picker receipt did not match the route or entry.
    PickerReceiptInvalid,
    /// Host capture was suppressed as a recent Guest echo.
    EchoSuppressed,
    /// Host capture was supplied by a Guest session.
    HostSessionInvalid,
    /// A received attachment failed mandatory kernel metadata checks.
    AttachmentRejected,
}

impl core::fmt::Display for ClipboardServiceError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::DependencyUnavailable => "dependency-unavailable",
            Self::CrossZoneDenied => "cross-zone-denied",
            Self::GuestSuspended => "zone-suspended",
            Self::AuditUnavailable => "audit-unavailable",
            Self::HistoryRejected => "clipboard-history-rejected",
            Self::PickerRequired => "picker-required",
            Self::SessionUnauthenticated => "session-unauthenticated",
            Self::PickerReceiptInvalid => "picker-receipt-invalid",
            Self::EchoSuppressed => "echo-suppressed",
            Self::HostSessionInvalid => "host-session-invalid",
            Self::AttachmentRejected => "attachment-rejected",
        })
    }
}

impl std::error::Error for ClipboardServiceError {}

/// In-memory clipboard service.
pub struct ClipdHost {
    policy: Policy,
    history: ClipboardHistory,
    audit: ClipboardAuditQueue,
    dependency: DisplayDependency,
    echo_window: BTreeMap<String, u64>,
}

impl ClipdHost {
    /// Construct clipd-host with an optional display dependency.
    pub fn new(
        policy: Policy,
        audit_capacity: usize,
        display: Option<DisplayDependencyEvidence>,
    ) -> Result<Self, ClipboardServiceError> {
        if display.as_ref().is_some_and(|evidence| {
            evidence.provider_ref().to_canonical_string() != "Provider/display-wayland"
                || evidence.generation() == 0
        }) {
            return Err(ClipboardServiceError::DependencyUnavailable);
        }
        let history = ClipboardHistory::new(crate::ClipboardConfig::from_policy(policy.clone()))
            .map_err(|_| ClipboardServiceError::HistoryRejected)?;
        let status = if display.is_some() {
            DependencyStatus::Ready
        } else {
            DependencyStatus::Absent
        };
        Ok(Self {
            policy,
            history,
            audit: ClipboardAuditQueue::new(audit_capacity),
            dependency: DisplayDependency {
                status,
                service_contract: "d2b.display.host-clipboard.v3",
                evidence: display,
            },
            echo_window: BTreeMap::new(),
        })
    }

    /// Return the typed display dependency state.
    pub const fn dependency(&self) -> &DisplayDependency {
        &self.dependency
    }

    /// Validate all descriptors from one authenticated attachment packet.
    ///
    /// Control truncation, descriptor metadata, operation direction, and the
    /// configured concurrent-FD bound are checked before ownership escapes the
    /// receive adapter.
    pub fn accept_received_fds<F>(
        &self,
        session: &AuthenticatedClipboardSession,
        batch: ReceivedFdBatch<F>,
        attachment_class: AttachmentClass,
    ) -> Result<Vec<F>, ClipboardServiceError>
    where
        F: std::os::fd::AsFd,
    {
        if self.dependency.status != DependencyStatus::Ready {
            return Err(ClipboardServiceError::DependencyUnavailable);
        }
        let expected_guest = matches!(attachment_class, AttachmentClass::GuestTransfer);
        if session.is_guest() != expected_guest {
            return Err(if expected_guest {
                ClipboardServiceError::SessionUnauthenticated
            } else {
                ClipboardServiceError::HostSessionInvalid
            });
        }
        if batch.len() > self.policy.max_concurrent_fds() {
            return Err(ClipboardServiceError::AttachmentRejected);
        }
        batch
            .validate_control(attachment_class, self.policy.max_item_bytes() as u64)
            .map_err(|_: FdSafetyError| ClipboardServiceError::AttachmentRejected)
    }

    /// Admit attachments from the audited Unix session adapter.
    ///
    /// `VerifiedPacket` can only be produced after the transport has checked
    /// the authenticated packet descriptor policy.  This boundary performs
    /// the Provider-specific size, mode, link-count, CLOEXEC, and direction
    /// checks before any descriptor is returned to the service.
    pub fn accept_verified_packet(
        &self,
        session: &AuthenticatedClipboardSession,
        packet: VerifiedPacket,
        attachment_class: AttachmentClass,
    ) -> Result<Vec<std::os::fd::OwnedFd>, ClipboardServiceError> {
        let (_payload, attachments, _credits) = packet.into_parts();
        let descriptors = attachments
            .into_iter()
            .map(|attachment| match attachment {
                AcceptedAttachment::File(fd) => Ok(fd),
                AcceptedAttachment::Credentials(_) => {
                    Err(ClipboardServiceError::AttachmentRejected)
                }
            })
            .collect::<Result<Vec<_>, _>>()?;
        self.accept_received_fds(
            session,
            ReceivedFdBatch::new(descriptors, false),
            attachment_class,
        )
    }

    /// Capture one Guest selection after audit admission.
    pub fn capture_guest(
        &mut self,
        session: &AuthenticatedClipboardSession,
        mime: &str,
        bytes: &[u8],
        now_secs: u64,
    ) -> Result<String, ClipboardServiceError> {
        if !session.is_guest() {
            return Err(ClipboardServiceError::SessionUnauthenticated);
        }
        if self.dependency.status != DependencyStatus::Ready {
            return Err(ClipboardServiceError::DependencyUnavailable);
        }
        if !self.policy.allow_guest_capture() {
            return Err(ClipboardServiceError::HistoryRejected);
        }
        if bytes.len() > self.policy.max_item_bytes() {
            return Err(ClipboardServiceError::HistoryRejected);
        }
        let guest = session.subject_ref().to_canonical_string();
        self.history
            .check_guest_request(&guest, now_secs)
            .map_err(|_| ClipboardServiceError::HistoryRejected)?;
        let entry = ClipboardEntry::new(&guest, mime, bytes, now_secs)
            .map_err(|_| ClipboardServiceError::HistoryRejected)?;
        let token = entry.token().to_owned();
        if self.audit.is_full() {
            return Err(ClipboardServiceError::AuditUnavailable);
        }
        self.history
            .insert(entry)
            .map_err(|_| ClipboardServiceError::HistoryRejected)?;
        self.history
            .record_guest_request(&guest, now_secs)
            .map_err(|_| ClipboardServiceError::HistoryRejected)?;
        let event = ClipboardAuditEvent::new(
            "guest",
            "host",
            ClipboardReason::Allowed,
            SizeBucket::from_len(bytes.len()),
        )
        .with_event_type(crate::ClipboardEventType::GuestCapture);
        self.audit
            .push(event)
            .map_err(|_| ClipboardServiceError::AuditUnavailable)?;
        self.echo_window
            .insert(token.clone(), now_secs.saturating_add(5));
        Ok(token)
    }

    /// Issue authenticated metadata for a Guest selection event.
    pub fn guest_selection_event(
        &self,
        session: &AuthenticatedClipboardSession,
        entry_digest: &str,
        now_secs: u64,
    ) -> Result<GuestSelectionEvent, ClipboardServiceError> {
        if !session.is_guest() {
            return Err(ClipboardServiceError::SessionUnauthenticated);
        }
        let Some(expires_at) = self.echo_window.get(entry_digest).copied() else {
            return Err(ClipboardServiceError::HistoryRejected);
        };
        if expires_at <= now_secs {
            return Err(ClipboardServiceError::HistoryRejected);
        }
        Ok(GuestSelectionEvent {
            source_zone: session.zone.clone(),
            source_guest: session.subject_ref.clone(),
            source_generation: session.reconnect_generation,
            entry_digest: entry_digest.to_owned(),
            expires_at,
        })
    }

    /// Capture one host selection through an authenticated host session.
    pub fn capture_host(
        &mut self,
        session: &AuthenticatedClipboardSession,
        mime: &str,
        bytes: &[u8],
        source_event: Option<GuestSelectionEvent>,
        now_secs: u64,
    ) -> Result<String, ClipboardServiceError> {
        if session.is_guest() {
            return Err(ClipboardServiceError::HostSessionInvalid);
        }
        if self.dependency.status != DependencyStatus::Ready {
            return Err(ClipboardServiceError::DependencyUnavailable);
        }
        if !self.policy.allow_host_capture() {
            return Err(ClipboardServiceError::HistoryRejected);
        }
        self.echo_window
            .retain(|_, expires_at| *expires_at > now_secs);
        if self.policy.suppress_echo()
            && source_event.as_ref().is_some_and(|event| {
                event.expires_at > now_secs
                    && event.source_zone.as_str() == session.zone()
                    && event.source_generation == session.reconnect_generation
                    && event.source_guest.resource_type().as_str() == "Guest"
                    && self.echo_window.contains_key(&event.entry_digest)
            })
        {
            if !self.audit.is_full() {
                let source_zone = source_event
                    .as_ref()
                    .map_or_else(|| session.zone.clone(), |event| event.source_zone.clone());
                let _ = self.audit.push(
                    ClipboardAuditEvent::new(
                        source_zone.as_str(),
                        session.zone(),
                        ClipboardReason::EchoSuppressed,
                        SizeBucket::from_len(bytes.len()),
                    )
                    .with_event_type(crate::ClipboardEventType::EchoSuppressed),
                );
            }
            return Err(ClipboardServiceError::EchoSuppressed);
        }
        if bytes.len() > self.policy.max_item_bytes() {
            return Err(ClipboardServiceError::HistoryRejected);
        }
        let owner = format!("Host/{}", session.zone());
        let entry = ClipboardEntry::new(owner, mime, bytes, now_secs)
            .map_err(|_| ClipboardServiceError::HistoryRejected)?;
        let token = entry.token().to_owned();
        if self.audit.is_full() {
            return Err(ClipboardServiceError::AuditUnavailable);
        }
        self.history
            .insert(entry)
            .map_err(|_| ClipboardServiceError::HistoryRejected)?;
        self.audit
            .push(
                ClipboardAuditEvent::new(
                    session.zone(),
                    session.zone(),
                    ClipboardReason::Allowed,
                    SizeBucket::from_len(bytes.len()),
                )
                .with_event_type(crate::ClipboardEventType::HostCapture),
            )
            .map_err(|_| ClipboardServiceError::AuditUnavailable)?;
        Ok(token)
    }

    /// Suspend a Guest and revoke its paste authority.
    pub fn suspend_guest(&mut self, guest: &str) {
        self.history.suspend_guest(guest);
    }

    /// Resume a Guest.
    pub fn resume_guest(&mut self, guest: &str) {
        self.history.resume_guest(guest);
    }

    /// Purge all Guest-owned entries on lifecycle destruction.
    pub fn purge_guest(&mut self, guest: &str) {
        self.history.purge_guest(guest);
    }

    /// Check whether a cross-Zone route is allowed.
    pub const fn cross_zone_allowed(&self) -> bool {
        self.policy.cross_zone_enabled()
    }

    /// Check a paste route before any attachment is requested.
    pub fn authorize_paste(
        &self,
        route: &AuthenticatedPasteRoute,
    ) -> Result<(), ClipboardServiceError> {
        self.authorize_paste_inner(route, false)
    }

    /// Check a paste route after the authenticated picker completed.
    pub fn authorize_paste_after_picker(
        &self,
        route: &AuthenticatedPasteRoute,
        receipt: PickerReceipt,
        entry_digest: &str,
    ) -> Result<(), ClipboardServiceError> {
        if !receipt.matches_and_consume(route, entry_digest) {
            return Err(ClipboardServiceError::PickerReceiptInvalid);
        }
        self.authorize_paste_inner(route, true)
    }

    fn authorize_paste_inner(
        &self,
        route: &AuthenticatedPasteRoute,
        picker_completed: bool,
    ) -> Result<(), ClipboardServiceError> {
        if self.dependency.status != DependencyStatus::Ready {
            return Err(ClipboardServiceError::DependencyUnavailable);
        }

        if route.source_zone() != route.destination_zone() && !self.policy.cross_zone_enabled() {
            return Err(ClipboardServiceError::CrossZoneDenied);
        }
        self.history
            .authorize_guest(route.destination_guest())
            .map_err(|_| ClipboardServiceError::GuestSuspended)
            .and_then(|()| {
                if self.policy.require_picker_for_paste() && !picker_completed {
                    Err(ClipboardServiceError::PickerRequired)
                } else {
                    Ok(())
                }
            })
    }

    /// Return bounded history size.
    pub fn history_len(&self) -> usize {
        self.history.len()
    }
}

pub(crate) fn operation_id_for_sessions(
    source: &AuthenticatedClipboardSession,
    destination: &AuthenticatedClipboardSession,
) -> String {
    let mut digest = Sha256::new();
    digest.update(source.subject_ref.to_canonical_string().as_bytes());
    digest.update([0]);
    digest.update(source.zone.as_str().as_bytes());
    digest.update([0]);
    digest.update(source.reconnect_generation.to_be_bytes());
    digest.update([0]);
    digest.update(destination.subject_ref.to_canonical_string().as_bytes());
    digest.update([0]);
    digest.update(destination.zone.as_str().as_bytes());
    digest.update([0]);
    digest.update(destination.reconnect_generation.to_be_bytes());
    format!(
        "sha256:{}",
        digest
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}

/// Provider configuration used by history and service components.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardConfig {
    policy: Policy,
    host_entry_ttl_secs: u64,
    guest_entry_ttl_secs: u64,
}

impl Default for ClipboardConfig {
    fn default() -> Self {
        Self {
            policy: Policy::default(),
            host_entry_ttl_secs: 3600,
            guest_entry_ttl_secs: 3600,
        }
    }
}

impl ClipboardConfig {
    /// Construct configuration from a policy.
    pub fn from_policy(policy: Policy) -> Self {
        Self {
            policy,
            ..Self::default()
        }
    }

    /// Return the policy.
    pub const fn policy(&self) -> &Policy {
        &self.policy
    }

    /// Return item byte limit.
    pub const fn max_item_bytes(&self) -> usize {
        self.policy.max_item_bytes()
    }

    /// Return total byte limit.
    pub const fn max_total_bytes(&self) -> usize {
        self.policy.max_total_bytes()
    }

    /// Return history entry bound.
    pub const fn max_history_entries(&self) -> usize {
        self.policy.max_history_entries()
    }

    /// Return per-Guest rate limit.
    pub const fn max_guest_rate_per_min(&self) -> u32 {
        self.policy.max_guest_rate_per_min()
    }

    /// Return Host entry TTL.
    pub const fn host_entry_ttl_secs(&self) -> u64 {
        self.host_entry_ttl_secs
    }

    /// Return Guest entry TTL.
    pub const fn guest_entry_ttl_secs(&self) -> u64 {
        self.guest_entry_ttl_secs
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::picker::{PickerAuthority, PickerRequest};

    fn display() -> DisplayDependencyEvidence {
        DisplayDependencyEvidence {
            provider_ref: ResourceRef::parse("Provider/display-wayland").unwrap(),
            zone: ZoneId::parse("zone-a").unwrap(),
            user_ref: ResourceRef::parse("User/alice").unwrap(),
            generation: 1,
        }
    }

    fn guest(name: &str, zone: &str, generation: u64) -> AuthenticatedClipboardSession {
        AuthenticatedClipboardSession {
            subject_ref: ResourceRef::parse(&format!("Guest/{name}")).unwrap(),
            zone: ZoneId::parse(zone).unwrap(),
            reconnect_generation: generation,
        }
    }

    fn user(zone: &str, generation: u64) -> AuthenticatedClipboardSession {
        AuthenticatedClipboardSession {
            subject_ref: ResourceRef::parse("User/alice").unwrap(),
            zone: ZoneId::parse(zone).unwrap(),
            reconnect_generation: generation,
        }
    }

    #[test]
    fn paste_routes_are_bound_to_authenticated_sessions_and_one_use_picker_receipts() {
        let host = ClipdHost::new(Policy::default(), 4, Some(display())).unwrap();
        let source = user("zone-a", 7);
        let destination = guest("work", "zone-a", 8);
        let route = AuthenticatedPasteRoute::from_sessions(&source, &destination).unwrap();
        assert_eq!(
            host.authorize_paste(&route),
            Err(ClipboardServiceError::PickerRequired)
        );

        let request = PickerRequest::new(
            route.operation_id(),
            "zone-a",
            "Guest/work",
            vec!["text/plain".to_owned()],
        )
        .unwrap();
        let digest = format!("sha256:{}", "a".repeat(64));
        let receipt = PickerAuthority::complete(
            &source,
            &destination,
            &request,
            crate::picker::PickerResult::Selected(digest.clone()),
            digest.clone(),
        )
        .expect("picker receipt");
        assert!(
            host.authorize_paste_after_picker(&route, receipt, &digest)
                .is_ok()
        );
    }

    #[test]
    fn guest_capture_requires_ready_display_and_authenticated_guest() {
        let guest = guest("work", "zone-a", 1);
        let user = user("zone-a", 1);
        let mut absent = ClipdHost::new(Policy::default(), 4, None).unwrap();
        assert_eq!(
            absent.capture_guest(&guest, "text/plain", b"hello", 100),
            Err(ClipboardServiceError::DependencyUnavailable)
        );
        let mut host = ClipdHost::new(Policy::default(), 4, Some(display())).unwrap();
        assert_eq!(
            host.capture_guest(&user, "text/plain", b"hello", 100),
            Err(ClipboardServiceError::SessionUnauthenticated)
        );
        assert!(
            host.capture_guest(&guest, "text/plain", b"hello", 100)
                .is_ok()
        );
    }

    #[test]
    fn host_capture_enforces_policy_and_audits_suppressed_echo() {
        let policy = Policy::new(true, true, true, true, true, 3, 4096, 4096, 32, 60).unwrap();
        let mut host = ClipdHost::new(policy, 4, Some(display())).unwrap();
        let guest = guest("work", "zone-a", 1);
        let token = host
            .capture_guest(&guest, "text/plain", b"hello", 100)
            .unwrap();
        let event = host.guest_selection_event(&guest, &token, 101).unwrap();
        assert_eq!(
            host.capture_host(&user("zone-a", 1), "text/plain", b"hello", Some(event), 101),
            Err(ClipboardServiceError::EchoSuppressed)
        );
        assert_eq!(host.history_len(), 1);
    }
}
