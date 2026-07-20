//! Bounded action service over an authenticated notify `ComponentSession`.
//!
//! Endpoint discovery, transport connection, and legacy CLI forwarding are
//! deliberately outside this module. Composition supplies a pre-authorized
//! session and adapts the frozen `NotifyService.InvokeAction` envelope into
//! [`InvokeActionRequest`].

use std::collections::VecDeque;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::nonce::{ActionNonce, ActionNonceStore, MAX_STORE_SIZE, NonceError};

pub const SERVICE_PACKAGE: &str = "d2b.notify.v2";
pub const ENDPOINT_PURPOSE: &str = "desktop-observer";
pub const ENDPOINT_ROLE: &str = "desktop-observer";
pub const SERVICE_NAME: &str = "NotifyService";
pub const INVOKE_ACTION_METHOD: &str = "InvokeAction";

pub const REQUEST_ID_BYTES: usize = 16;
pub const IDEMPOTENCY_KEY_BYTES: usize = 16;
pub const MAX_INVOKE_REQUEST_BYTES: usize = 512;
pub const MAX_REQUEST_LIFETIME_MS: u64 = 120_000;
pub const MAX_TARGET_BYTES: usize = 256;
pub const MAX_COMPLETED_ACTIONS: usize = MAX_STORE_SIZE;
pub const COMPLETED_TTL_MS: u64 = 120_000;
pub const MAX_ACTION_MEASURES: usize = 6;

pub trait EstablishedComponentSession {
    fn service_package(&self) -> &str;
    fn endpoint_purpose(&self) -> &str;
    fn endpoint_role(&self) -> &str;
    fn is_authenticated(&self) -> bool;
    fn uses_pre_authorized_transport(&self) -> bool;
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ActionSession {
    _private: (),
}

impl std::fmt::Debug for ActionSession {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ActionSession(<authenticated>)")
    }
}

impl ActionSession {
    pub fn admit(
        established: &impl EstablishedComponentSession,
    ) -> Result<Self, SessionAdmissionError> {
        if !established.is_authenticated() {
            return Err(SessionAdmissionError::Unauthenticated);
        }
        if !established.uses_pre_authorized_transport() {
            return Err(SessionAdmissionError::UntrustedTransport);
        }
        if established.service_package() != SERVICE_PACKAGE
            || established.endpoint_purpose() != ENDPOINT_PURPOSE
            || established.endpoint_role() != ENDPOINT_ROLE
        {
            return Err(SessionAdmissionError::ContractMismatch);
        }
        Ok(Self { _private: () })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionAdmissionError {
    Unauthenticated,
    UntrustedTransport,
    ContractMismatch,
}

impl std::fmt::Display for SessionAdmissionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let code = match self {
            Self::Unauthenticated => "desktop-action-session-unauthenticated",
            Self::UntrustedTransport => "desktop-action-transport-untrusted",
            Self::ContractMismatch => "desktop-action-session-contract-mismatch",
        };
        formatter.write_str(code)
    }
}

impl std::error::Error for SessionAdmissionError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum ActionKind {
    CancelSecurityKeyCeremony,
}

#[derive(Clone, PartialEq, Eq)]
struct ActionIntent {
    kind: ActionKind,
    target: String,
}

impl std::fmt::Debug for ActionIntent {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ActionIntent")
            .field("kind", &self.kind)
            .field("target", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActionOffer {
    pub kind: ActionKind,
    pub capability: ActionNonce,
}

impl std::fmt::Debug for ActionOffer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ActionOffer")
            .field("kind", &self.kind)
            .field("capability", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InvokeActionRequest {
    pub request_id: Vec<u8>,
    pub idempotency_key: Vec<u8>,
    pub capability: ActionNonce,
    pub issued_at_unix_ms: u64,
    pub expires_at_unix_ms: u64,
}

impl std::fmt::Debug for InvokeActionRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("InvokeActionRequest")
            .field("request_id", &"<redacted>")
            .field("idempotency_key", &"<redacted>")
            .field("capability", &"<redacted>")
            .field("issued_at_unix_ms", &self.issued_at_unix_ms)
            .field("expires_at_unix_ms", &self.expires_at_unix_ms)
            .finish()
    }
}

impl InvokeActionRequest {
    pub fn new(
        request_id: [u8; REQUEST_ID_BYTES],
        idempotency_key: [u8; IDEMPOTENCY_KEY_BYTES],
        capability: ActionNonce,
        issued_at_unix_ms: u64,
        expires_at_unix_ms: u64,
    ) -> Self {
        Self {
            request_id: request_id.to_vec(),
            idempotency_key: idempotency_key.to_vec(),
            capability,
            issued_at_unix_ms,
            expires_at_unix_ms,
        }
    }

    fn validate(&self, now_unix_ms: u64) -> Result<(), InvokeError> {
        let encoded_len = serde_json::to_vec(self)
            .map_err(|_| InvokeError::Malformed)?
            .len();
        if encoded_len > MAX_INVOKE_REQUEST_BYTES
            || self.request_id.len() != REQUEST_ID_BYTES
            || self.idempotency_key.len() != IDEMPOTENCY_KEY_BYTES
        {
            return Err(InvokeError::Malformed);
        }
        if self.issued_at_unix_ms > now_unix_ms
            || self.expires_at_unix_ms <= now_unix_ms
            || self.expires_at_unix_ms <= self.issued_at_unix_ms
            || self
                .expires_at_unix_ms
                .saturating_sub(self.issued_at_unix_ms)
                > MAX_REQUEST_LIFETIME_MS
        {
            return Err(InvokeError::Expired);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ActionOutcome {
    Succeeded,
    NotApplicable,
    Denied,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvokeError {
    Malformed,
    Expired,
    InvalidCapability,
    IdempotencyConflict,
}

impl std::fmt::Display for InvokeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let code = match self {
            Self::Malformed => "desktop-action-request-invalid",
            Self::Expired => "desktop-action-request-expired",
            Self::InvalidCapability => "desktop-action-capability-invalid",
            Self::IdempotencyConflict => "desktop-action-idempotency-conflict",
        };
        formatter.write_str(code)
    }
}

impl std::error::Error for InvokeError {}

#[derive(Clone, Copy)]
pub struct AuthorizedAction<'a> {
    intent: &'a ActionIntent,
}

impl AuthorizedAction<'_> {
    pub fn kind(self) -> ActionKind {
        self.intent.kind
    }

    pub fn target(&self) -> &str {
        &self.intent.target
    }
}

impl std::fmt::Debug for AuthorizedAction<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AuthorizedAction")
            .field("kind", &self.intent.kind)
            .field("target", &"<redacted>")
            .finish()
    }
}

pub trait ActionExecutor {
    fn execute(&mut self, action: AuthorizedAction<'_>) -> ActionOutcome;
}

struct CompletedAction {
    idempotency_key: Vec<u8>,
    capability: ActionNonce,
    outcome: ActionOutcome,
    expires_at_unix_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionMeasureKind {
    Issued,
    Invoked,
    Replayed,
    Rejected,
    Pending,
    Completed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActionMeasure {
    pub kind: ActionMeasureKind,
    pub value: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionObservability {
    measures: [ActionMeasure; MAX_ACTION_MEASURES],
}

impl ActionObservability {
    pub fn measures(&self) -> &[ActionMeasure] {
        &self.measures
    }
}

pub trait LocalObservabilitySink {
    type Error;

    fn project(&mut self, measures: &[ActionMeasure]) -> Result<(), Self::Error>;
}

pub struct ActionService {
    _session: ActionSession,
    pending: ActionNonceStore<ActionIntent>,
    completed: VecDeque<CompletedAction>,
    issued: u64,
    invoked: u64,
    replayed: u64,
    rejected: u64,
}

impl std::fmt::Debug for ActionService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ActionService")
            .field("pending", &self.pending.len())
            .field("completed", &self.completed.len())
            .field("issued", &self.issued)
            .field("invoked", &self.invoked)
            .field("replayed", &self.replayed)
            .field("rejected", &self.rejected)
            .finish()
    }
}

impl ActionService {
    pub fn new(session: ActionSession) -> Self {
        Self {
            _session: session,
            pending: ActionNonceStore::new(),
            completed: VecDeque::new(),
            issued: 0,
            invoked: 0,
            replayed: 0,
            rejected: 0,
        }
    }

    pub fn offer_cancel(
        &mut self,
        target: impl Into<String>,
        now_secs: u64,
    ) -> Result<ActionOffer, OfferError> {
        let target = target.into();
        if target.is_empty() || target.len() > MAX_TARGET_BYTES {
            return Err(OfferError::InvalidTarget);
        }
        let capability = self
            .pending
            .issue(
                ActionIntent {
                    kind: ActionKind::CancelSecurityKeyCeremony,
                    target,
                },
                now_secs,
            )
            .map_err(OfferError::Capability)?;
        self.issued = self.issued.saturating_add(1);
        Ok(ActionOffer {
            kind: ActionKind::CancelSecurityKeyCeremony,
            capability,
        })
    }

    pub fn invoke<E: ActionExecutor>(
        &mut self,
        request: &InvokeActionRequest,
        now_unix_ms: u64,
        executor: &mut E,
    ) -> Result<ActionOutcome, InvokeError> {
        self.gc_completed(now_unix_ms);
        if let Err(error) = request.validate(now_unix_ms) {
            self.rejected = self.rejected.saturating_add(1);
            return Err(error);
        }

        if let Some(completed) = self
            .completed
            .iter()
            .find(|completed| completed.idempotency_key == request.idempotency_key)
        {
            if completed.capability != request.capability {
                self.rejected = self.rejected.saturating_add(1);
                return Err(InvokeError::IdempotencyConflict);
            }
            self.replayed = self.replayed.saturating_add(1);
            return Ok(completed.outcome);
        }

        let intent = match self
            .pending
            .consume(&request.capability, now_unix_ms / 1_000)
        {
            Ok(intent) => intent,
            Err(NonceError::Invalid)
            | Err(NonceError::MissingOrConsumed)
            | Err(NonceError::Expired)
            | Err(NonceError::Capacity)
            | Err(NonceError::EntropyUnavailable) => {
                self.rejected = self.rejected.saturating_add(1);
                return Err(InvokeError::InvalidCapability);
            }
        };
        let outcome = executor.execute(AuthorizedAction { intent: &intent });
        self.invoked = self.invoked.saturating_add(1);
        if self.completed.len() == MAX_COMPLETED_ACTIONS {
            self.completed.pop_front();
        }
        self.completed.push_back(CompletedAction {
            idempotency_key: request.idempotency_key.clone(),
            capability: request.capability.clone(),
            outcome,
            expires_at_unix_ms: now_unix_ms.saturating_add(COMPLETED_TTL_MS),
        });
        Ok(outcome)
    }

    pub fn observability(&self) -> ActionObservability {
        ActionObservability {
            measures: [
                ActionMeasure {
                    kind: ActionMeasureKind::Issued,
                    value: self.issued,
                },
                ActionMeasure {
                    kind: ActionMeasureKind::Invoked,
                    value: self.invoked,
                },
                ActionMeasure {
                    kind: ActionMeasureKind::Replayed,
                    value: self.replayed,
                },
                ActionMeasure {
                    kind: ActionMeasureKind::Rejected,
                    value: self.rejected,
                },
                ActionMeasure {
                    kind: ActionMeasureKind::Pending,
                    value: self.pending.len() as u64,
                },
                ActionMeasure {
                    kind: ActionMeasureKind::Completed,
                    value: self.completed.len() as u64,
                },
            ],
        }
    }

    pub fn project_observability<S: LocalObservabilitySink>(
        &self,
        sink: &mut S,
    ) -> Result<(), S::Error> {
        sink.project(self.observability().measures())
    }

    fn gc_completed(&mut self, now_unix_ms: u64) {
        self.completed
            .retain(|completed| completed.expires_at_unix_ms > now_unix_ms);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OfferError {
    InvalidTarget,
    Capability(NonceError),
}

impl std::fmt::Display for OfferError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidTarget => formatter.write_str("desktop-action-target-invalid"),
            Self::Capability(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for OfferError {}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW_MS: u64 = 1_750_000_000_000;

    struct Session {
        package: &'static str,
        purpose: &'static str,
        role: &'static str,
        authenticated: bool,
        pre_authorized: bool,
    }

    impl EstablishedComponentSession for Session {
        fn service_package(&self) -> &str {
            self.package
        }
        fn endpoint_purpose(&self) -> &str {
            self.purpose
        }
        fn endpoint_role(&self) -> &str {
            self.role
        }
        fn is_authenticated(&self) -> bool {
            self.authenticated
        }
        fn uses_pre_authorized_transport(&self) -> bool {
            self.pre_authorized
        }
    }

    fn admitted() -> ActionSession {
        ActionSession::admit(&Session {
            package: SERVICE_PACKAGE,
            purpose: ENDPOINT_PURPOSE,
            role: ENDPOINT_ROLE,
            authenticated: true,
            pre_authorized: true,
        })
        .unwrap()
    }

    fn request(offer: &ActionOffer, key: u8) -> InvokeActionRequest {
        InvokeActionRequest::new(
            [key; REQUEST_ID_BYTES],
            [key; IDEMPOTENCY_KEY_BYTES],
            offer.capability.clone(),
            NOW_MS,
            NOW_MS + 10_000,
        )
    }

    #[derive(Default)]
    struct Executor {
        calls: usize,
        seen_target: Option<String>,
    }

    impl ActionExecutor for Executor {
        fn execute(&mut self, action: AuthorizedAction<'_>) -> ActionOutcome {
            self.calls += 1;
            self.seen_target = Some(action.target().to_owned());
            assert_eq!(action.kind(), ActionKind::CancelSecurityKeyCeremony);
            ActionOutcome::Succeeded
        }
    }

    #[test]
    fn admission_requires_authenticated_frozen_session() {
        let mut session = Session {
            package: SERVICE_PACKAGE,
            purpose: ENDPOINT_PURPOSE,
            role: ENDPOINT_ROLE,
            authenticated: false,
            pre_authorized: true,
        };
        assert_eq!(
            ActionSession::admit(&session),
            Err(SessionAdmissionError::Unauthenticated)
        );
        session.authenticated = true;
        session.pre_authorized = false;
        assert_eq!(
            ActionSession::admit(&session),
            Err(SessionAdmissionError::UntrustedTransport)
        );
        session.pre_authorized = true;
        session.package = "d2b.notify.v1";
        assert_eq!(
            ActionSession::admit(&session),
            Err(SessionAdmissionError::ContractMismatch)
        );
    }

    #[test]
    fn invoke_is_single_execution_and_replay_safe() {
        let mut service = ActionService::new(admitted());
        let offer = service
            .offer_cancel("private-ceremony-target", NOW_MS / 1_000)
            .unwrap();
        let request = request(&offer, 7);
        let mut executor = Executor::default();
        assert_eq!(
            service.invoke(&request, NOW_MS + 1, &mut executor),
            Ok(ActionOutcome::Succeeded)
        );
        assert_eq!(
            service.invoke(&request, NOW_MS + 2, &mut executor),
            Ok(ActionOutcome::Succeeded)
        );
        assert_eq!(executor.calls, 1);
        assert_eq!(
            executor.seen_target.as_deref(),
            Some("private-ceremony-target")
        );
    }

    #[test]
    fn capability_replay_with_a_new_key_is_rejected() {
        let mut service = ActionService::new(admitted());
        let offer = service.offer_cancel("target", NOW_MS / 1_000).unwrap();
        let mut executor = Executor::default();
        service
            .invoke(&request(&offer, 1), NOW_MS + 1, &mut executor)
            .unwrap();
        assert_eq!(
            service.invoke(&request(&offer, 2), NOW_MS + 2, &mut executor),
            Err(InvokeError::InvalidCapability)
        );
        assert_eq!(executor.calls, 1);
    }

    #[test]
    fn conflicting_idempotency_key_is_rejected() {
        let mut service = ActionService::new(admitted());
        let first = service.offer_cancel("first", NOW_MS / 1_000).unwrap();
        let second = service.offer_cancel("second", NOW_MS / 1_000).unwrap();
        let mut executor = Executor::default();
        service
            .invoke(&request(&first, 1), NOW_MS + 1, &mut executor)
            .unwrap();
        assert_eq!(
            service.invoke(&request(&second, 1), NOW_MS + 2, &mut executor),
            Err(InvokeError::IdempotencyConflict)
        );
        assert_eq!(executor.calls, 1);
    }

    #[test]
    fn request_bounds_and_expiry_fail_closed() {
        let mut service = ActionService::new(admitted());
        let offer = service.offer_cancel("target", NOW_MS / 1_000).unwrap();
        let mut malformed = request(&offer, 3);
        malformed.request_id.push(3);
        let mut executor = Executor::default();
        assert_eq!(
            service.invoke(&malformed, NOW_MS + 1, &mut executor),
            Err(InvokeError::Malformed)
        );
        let expired = InvokeActionRequest::new(
            [4; REQUEST_ID_BYTES],
            [4; IDEMPOTENCY_KEY_BYTES],
            offer.capability,
            NOW_MS,
            NOW_MS + 1,
        );
        assert_eq!(
            service.invoke(&expired, NOW_MS + 1, &mut executor),
            Err(InvokeError::Expired)
        );
        assert_eq!(executor.calls, 0);
    }

    #[test]
    fn wire_debug_errors_and_observability_do_not_leak_target_or_capability() {
        let mut service = ActionService::new(admitted());
        let offer = service
            .offer_cancel("secret-target", NOW_MS / 1_000)
            .unwrap();
        let request = request(&offer, 9);
        for output in [
            format!("{service:?}"),
            format!("{offer:?}"),
            format!("{request:?}"),
            format!("{:?}", service.observability()),
            InvokeError::InvalidCapability.to_string(),
        ] {
            assert!(!output.contains("secret-target"));
            assert!(!output.contains(offer.capability.expose()));
        }
        let wire = serde_json::to_string(&request).unwrap();
        assert!(!wire.contains("secret-target"));
        assert!(!wire.contains("cancel"));
        assert!(wire.len() <= MAX_INVOKE_REQUEST_BYTES);
    }

    #[test]
    fn observability_adapter_receives_only_closed_bounded_measures() {
        #[derive(Default)]
        struct Sink(Vec<ActionMeasure>);

        impl LocalObservabilitySink for Sink {
            type Error = std::convert::Infallible;

            fn project(&mut self, measures: &[ActionMeasure]) -> Result<(), Self::Error> {
                self.0.extend_from_slice(measures);
                Ok(())
            }
        }

        let service = ActionService::new(admitted());
        let mut sink = Sink::default();
        service.project_observability(&mut sink).unwrap();
        assert_eq!(sink.0.len(), MAX_ACTION_MEASURES);
    }

    #[test]
    fn owned_action_paths_have_no_legacy_endpoint_or_command_fallback() {
        let sources = [
            include_str!("../../nonce.rs"),
            include_str!("../../wlcontrol.rs"),
        ]
        .join("\n");
        for forbidden in [
            "/run/d2b/",
            "D2B_PUBLIC_SOCKET",
            "UnixStream::connect",
            "Command::new",
            "--action-token",
            "d2b-sk-",
        ] {
            assert!(
                !sources.contains(forbidden),
                "legacy desktop action fallback: {forbidden}"
            );
        }
    }

    #[test]
    fn service_keys_exist_in_all_frozen_contract_inputs() {
        let component_session =
            include_str!("../../../../d2b-contracts/src/v2_component_session.rs");
        let services = include_str!("../../../../d2b-contracts/src/v2_services.rs");
        let observability = include_str!("../../../../d2b-provider-observability-local/src/lib.rs");
        let transport = include_str!("../../../../d2b-provider-transport-local/src/lib.rs");

        assert!(component_session.contains("DesktopObserver = 15 => \"desktop-observer\""));
        assert!(component_session.contains("NotifyV2 = 11 => \"d2b.notify.v2\""));
        assert!(services.contains("\"InvokeAction\" => true"));
        assert!(observability.contains("pub struct BoundedProjection"));
        assert!(transport.contains("never discovers endpoints"));
    }
}
