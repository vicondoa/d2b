//! In-memory host notification sink and observer projection.

use crate::{
    GuestSource, NotificationProviderConfig,
    action_nonce::{ActionNonceError, ActionNonceStore},
    admission::{
        NotificationEndpointGate, NotificationEndpointRole, SessionEvidence,
        admit_notification_endpoint,
    },
    redact::SanitizedNotification,
    types::NotificationRequest,
};
use tracing::{debug, warn};
use std::collections::{BTreeMap, VecDeque};

/// D-Bus/presentation sink failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SinkError {
    /// The desktop notification service is unavailable.
    Unavailable,
    /// The operation timed out.
    Timeout,
}

impl core::fmt::Display for SinkError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::Unavailable => "sink-unavailable",
            Self::Timeout => "sink-timeout",
        })
    }
}

impl std::error::Error for SinkError {}

/// Presentation effect port. Implementations own the pre-opened desktop
/// session connection and never receive an address or path.
pub trait DesktopNotificationPort {
    /// Confirm that the host presentation boundary accepted sink activation.
    fn activate(&mut self) -> Result<(), SinkError>;
    /// Confirm that the host presentation boundary accepted sink deactivation.
    fn deactivate(&mut self) -> Result<(), SinkError>;
    /// Present one sanitized notification and return an opaque desktop ID.
    fn notify(&mut self, notification: &SanitizedNotification) -> Result<u32, SinkError>;
}

impl<T: DesktopNotificationPort + ?Sized> DesktopNotificationPort for Box<T> {
    fn activate(&mut self) -> Result<(), SinkError> {
        (**self).activate()
    }

    fn deactivate(&mut self) -> Result<(), SinkError> {
        (**self).deactivate()
    }

    fn notify(&mut self, notification: &SanitizedNotification) -> Result<u32, SinkError> {
        (**self).notify(notification)
    }
}

/// The result returned to the source stream.
#[derive(Clone, PartialEq, Eq)]
pub enum NotificationResult {
    /// Notification was accepted and action capabilities were issued.
    Accepted {
        /// Opaque desktop notification ID.
        notification_id: u32,
        /// Action capability keys keyed by stable action ID.
        action_nonces: BTreeMap<String, String>,
    },
    /// Sink could not present the request.
    SinkUnavailable,
    /// The bounded pending queue was full.
    CapacityExceeded,
    /// A request was rejected by validation.
    Rejected,
}

impl core::fmt::Debug for NotificationResult {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Accepted {
                notification_id,
                action_nonces,
            } => formatter
                .debug_struct("NotificationResult::Accepted")
                .field("notification_id", notification_id)
                .field("action_count", &action_nonces.len())
                .finish(),
            Self::SinkUnavailable => formatter.write_str("NotificationResult::SinkUnavailable"),
            Self::CapacityExceeded => formatter.write_str("NotificationResult::CapacityExceeded"),
            Self::Rejected => formatter.write_str("NotificationResult::Rejected"),
        }
    }
}

/// One observer projection entry held only for the session lifetime.
#[derive(Clone, PartialEq, Eq)]
pub struct NotificationProjection {
    /// Opaque request handle.
    pub request_id: String,
    /// Sanitized presentation content.
    pub notification: SanitizedNotification,
}

impl core::fmt::Debug for NotificationProjection {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("NotificationProjection(<redacted>)")
    }
}

/// Host sink with bounded projection and action state.
pub struct NotificationSink {
    max_pending: usize,
    acknowledge_timeout_secs: u64,
    observer_enabled: bool,
    projections: BTreeMap<String, NotificationProjection>,
    order: VecDeque<String>,
    projection_nonces: BTreeMap<String, Vec<String>>,
    projection_idempotency: BTreeMap<String, (String, String)>,
    projection_sessions: BTreeMap<String, String>,
    projection_deadlines: BTreeMap<String, u64>,
    idempotency: BTreeMap<(String, String), (String, NotificationResult)>,
    nonces: ActionNonceStore,
}

impl NotificationSink {
    /// Construct a host sink with bounded queue and nonce state.
    pub fn new(max_pending: usize, nonce_capacity: usize, nonce_ttl_secs: u64) -> Self {
        Self::new_with_policy(
            max_pending,
            nonce_capacity,
            nonce_ttl_secs,
            crate::DEFAULT_ACKNOWLEDGE_TIMEOUT_SECS,
            true,
        )
    }

    /// Construct a host sink with the complete Provider policy.
    pub fn new_with_policy(
        max_pending: usize,
        nonce_capacity: usize,
        nonce_ttl_secs: u64,
        acknowledge_timeout_secs: u64,
        observer_enabled: bool,
    ) -> Self {
        Self {
            max_pending,
            acknowledge_timeout_secs,
            observer_enabled,
            projections: BTreeMap::new(),
            order: VecDeque::new(),
            projection_nonces: BTreeMap::new(),
            projection_idempotency: BTreeMap::new(),
            projection_sessions: BTreeMap::new(),
            projection_deadlines: BTreeMap::new(),
            idempotency: BTreeMap::new(),
            nonces: ActionNonceStore::new(nonce_capacity, nonce_ttl_secs),
        }
    }

    /// Construct a host sink from the validated Provider configuration.
    pub fn from_config(config: &NotificationProviderConfig) -> Self {
        Self::new_with_policy(
            config.max_pending_notifications(),
            config.action_nonce_store_size(),
            config.action_nonce_ttl_secs(),
            config.acknowledge_timeout_secs(),
            config.observer_enabled(),
        )
    }

    /// Deliver one authenticated Guest-source request through the effect port
    /// to one authenticated desktop observer.
    pub(crate) fn deliver<P: DesktopNotificationPort + ?Sized>(
        &mut self,
        port: &mut P,
        source_session: &SessionEvidence,
        observer_session: &SessionEvidence,
        request: NotificationRequest,
        now_secs: u64,
    ) -> Result<NotificationResult, crate::types::NotificationError> {
        source_session.admit_source().map_err(|_| {
            debug!(
                provider = "notification-desktop",
                "delivery refused: source session not authenticated"
            );
            crate::types::NotificationError::Denied
        })?;
        if !self.observer_enabled {
            return Err(crate::types::NotificationError::ObserverDisabled);
        }
        observer_session.admit_observer().map_err(|_| {
            debug!(
                provider = "notification-desktop",
                "delivery refused: observer session not authenticated"
            );
            crate::types::NotificationError::Denied
        })?;
        if source_session.zone() != observer_session.zone() {
            debug!(
                provider = "notification-desktop",
                "delivery refused: source and observer zone mismatch"
            );
            return Err(crate::types::NotificationError::Denied);
        }
        let observer_session = observer_session.session_key();
        self.nonces.gc(now_secs);
        self.gc_projections(now_secs);
        self.prune_idempotency_nonces();
        let idempotency_key = request
            .idempotency_key()
            .map(|key| (observer_session.to_owned(), key.to_owned()));
        if let Some(key) = &idempotency_key
            && let Some((_, result)) = self.idempotency.get(key)
        {
            return Ok(result.clone());
        }
        if self.max_pending == 0 {
            return Ok(NotificationResult::CapacityExceeded);
        }
        let notification = request.sanitize()?;
        if notification.actions().len() > self.nonces.available_capacity() {
            return Ok(NotificationResult::CapacityExceeded);
        }
        let mut action_nonces = BTreeMap::new();
        let mut issued_keys: Vec<String> = Vec::with_capacity(notification.actions().len());
        for (action_id, _) in notification.actions() {
            let nonce = match self.nonces.register(&observer_session, action_id, now_secs) {
                Ok(nonce) => nonce,
                Err(error) => {
                    for action_key in &issued_keys {
                        self.nonces.revoke(action_key);
                    }
                    return Err(match error {
                        ActionNonceError::Capacity => {
                            crate::types::NotificationError::InvalidActions
                        }
                        _ => crate::types::NotificationError::InvalidOpaqueKey,
                    });
                }
            };
            let action_key = nonce.action_key();
            issued_keys.push(action_key.clone());
            action_nonces.insert(action_id.clone(), action_key);
        }
        // D-Bus must receive only the opaque capabilities that were issued
        // for this observer session; stable caller action IDs never cross the
        // host presentation boundary.
        let presentation = notification.clone().with_action_keys(&action_nonces)?;
        let notification_id = match port.notify(&presentation) {
            Ok(id) => id,
            Err(notify_error) => {
                warn!(
                    provider = "notification-desktop",
                    notify_error = ?notify_error,
                    "desktop notification delivery failed; revoking issued action capabilities"
                );
                for action_key in &issued_keys {
                    self.nonces.revoke(action_key);
                }
                return Ok(NotificationResult::SinkUnavailable);
            }
        };
        if self.projections.len() >= self.max_pending {
            self.evict_oldest();
        }
        let request_id = format!("notification-{notification_id}");
        self.order.push_back(request_id.clone());
        self.projections.insert(
            request_id.clone(),
            NotificationProjection {
                request_id: request_id.clone(),
                notification,
            },
        );
        self.projection_nonces.insert(request_id.clone(), issued_keys);
        self.projection_sessions
            .insert(request_id.clone(), observer_session);
        self.projection_deadlines.insert(
            request_id.clone(),
            now_secs.saturating_add(self.acknowledge_timeout_secs),
        );
        let result = NotificationResult::Accepted {
            notification_id,
            action_nonces,
        };
        if let Some(key) = idempotency_key {
            self.idempotency.insert(
                key.clone(),
                (request_id.clone(), result.clone()),
            );
            self.projection_idempotency.insert(request_id, key);
        }
        Ok(result)
    }

    /// Deliver one Guest-source request over an admitted desktop-presentation
    /// endpoint relationship.
    ///
    /// The declared endpoint gate runs first, so a missing, revoked, draining,
    /// or superseded relationship refuses the delivery before the desktop
    /// presentation port is touched. There is no second channel: the refusal
    /// is the delivery's only outcome, and it does not fall through to the
    /// observer stream or to an unadmitted presentation.
    ///
    /// Once admitted, the delivery is the unchanged one: the source and
    /// observer sessions must both be admitted, the Zone must match, the
    /// observer must be enabled, the action nonces are minted per observer
    /// session, and only the opaque action keys cross the presentation
    /// boundary. The request's content is not an input to the gate, so a
    /// summary, body, or action label can neither widen the relationship nor
    /// stand in for it.
    ///
    /// # Errors
    ///
    /// Returns [`crate::types::NotificationError::Denied`] when the desktop
    /// presentation channel carries no admitted relationship, when the
    /// presented evidence is not the committed evidence, or when either
    /// session is refused. The refusal code is recorded at the debug level.
    pub fn deliver_over_endpoint<P: DesktopNotificationPort + ?Sized>(
        &mut self,
        port: &mut P,
        source_session: &SessionEvidence,
        observer_session: &SessionEvidence,
        request: NotificationRequest,
        now_secs: u64,
        gate: &NotificationEndpointGate<'_>,
    ) -> Result<NotificationResult, crate::types::NotificationError> {
        let admitted = match admit_notification_endpoint(gate) {
            Ok(admitted) if admitted.role() == NotificationEndpointRole::DesktopSink => {
                admitted
            }
            Ok(_) => {
                debug!(
                    provider = "notification-desktop",
                    "delivery refused: admitted relationship is not the desktop presentation channel"
                );
                return Err(crate::types::NotificationError::Denied);
            }
            Err(refusal) => {
                debug!(
                    provider = "notification-desktop",
                    refusal = refusal.code(),
                    stage = ?refusal.stage(),
                    reason = ?refusal.reason(),
                    "delivery refused: desktop presentation endpoint not admitted"
                );
                return Err(crate::types::NotificationError::Denied);
            }
        };
        if !admitted.admits_delivery() {
            debug!(
                provider = "notification-desktop",
                "delivery refused: desktop presentation relationship no longer admits delivery"
            );
            return Err(crate::types::NotificationError::Denied);
        }
        self.deliver(port, source_session, observer_session, request, now_secs)
    }

    /// Deliver after the configured Guest-source category admission.
    ///
    /// # Errors
    ///
    /// Returns [`crate::types::NotificationError::Denied`] when the Guest
    /// source rejects the session or request, and the delivery validation
    /// errors (`FieldBounds`, `InvalidIcon`, `InvalidActions`,
    /// `InvalidTimeout`, `InvalidOpaqueKey`, `ObserverDisabled`) when the
    /// request or observer stream fails its bounded validation.
    pub fn deliver_from_guest_source<P: DesktopNotificationPort + ?Sized>(
        &mut self,
        port: &mut P,
        source: &GuestSource,
        source_session: &SessionEvidence,
        observer_session: &SessionEvidence,
        request: NotificationRequest,
        now_secs: u64,
    ) -> Result<NotificationResult, crate::types::NotificationError> {
        source
            .validate_authenticated(source_session, &request)
            .map_err(|_| crate::types::NotificationError::Denied)?;
        self.deliver(port, source_session, observer_session, request, now_secs)
    }

    /// Consume one observer action capability.
    pub fn invoke_action(
        &mut self,
        action_key: &str,
        observer_session: &SessionEvidence,
        now_secs: u64,
    ) -> Result<String, ActionNonceError> {
        observer_session.admit_observer().map_err(|_| {
            debug!(
                provider = "notification-desktop",
                "action invoke refused: observer session not authenticated"
            );
            ActionNonceError::SessionMismatch
        })?;
        let observer_session = observer_session.session_key();
        let result = self.nonces.consume(action_key, &observer_session, now_secs);
        if result.is_ok() {
            self.forget_consumed_nonce(action_key);
        } else {
            debug!(
                provider = "notification-desktop",
                action = action_key,
                "action capability rejected"
            );
        }
        result
    }

    /// Consume an action capability with an explicit action ID check.
    pub fn invoke_action_for(
        &mut self,
        action_key: &str,
        observer_session: &SessionEvidence,
        action_id: &str,
        now_secs: u64,
    ) -> Result<String, ActionNonceError> {
        observer_session.admit_observer().map_err(|_| {
            debug!(
                provider = "notification-desktop",
                "action invoke refused: observer session not authenticated"
            );
            ActionNonceError::SessionMismatch
        })?;
        let observer_session = observer_session.session_key();
        let result = self.nonces.consume_for_action(
            action_key,
            &observer_session,
            Some(action_id),
            now_secs,
        );
        if result.is_ok() {
            self.forget_consumed_nonce(action_key);
        } else {
            debug!(
                provider = "notification-desktop",
                action = action_key,
                "action capability rejected for the requested action id"
            );
        }
        result
    }

    /// Evict a projection when its desktop notification closes.
    pub fn close(&mut self, notification_id: u32) {
        self.close_by_request_id(&format!("notification-{notification_id}"));
    }

    /// Evict a projection by its internal request id.
    fn close_by_request_id(&mut self, request_id: &str) {
        self.projections.remove(request_id);
        self.revoke_projection_nonces(request_id);
        self.remove_projection_idempotency(request_id);
        self.projection_sessions.remove(request_id);
        self.projection_deadlines.remove(request_id);
        self.order.retain(|value| value != request_id);
    }

    /// Revoke all projections and action capabilities for a closed session.
    pub fn close_session(&mut self, observer_session: &SessionEvidence) {
        let session_key = observer_session.session_key();
        let request_ids = self
            .projection_sessions
            .iter()
            .filter(|(_, owner)| owner.as_str() == session_key.as_str())
            .map(|(request_id, _)| request_id.clone())
            .collect::<Vec<_>>();
        for request_id in request_ids {
            self.close_by_request_id(&request_id);
        }
        self.nonces.revoke_session(&session_key);
    }

    /// Drain all transient state during restart or shutdown.
    pub fn drain(&mut self) {
        self.projections.clear();
        self.order.clear();
        self.projection_nonces.clear();
        self.projection_idempotency.clear();
        self.projection_sessions.clear();
        self.projection_deadlines.clear();
        self.idempotency.clear();
        self.nonces.clear();
    }

    /// Return the current projection size.
    pub fn projection_len(&self) -> usize {
        self.projections.len()
    }

    fn evict_oldest(&mut self) {
        if let Some(request_id) = self.order.pop_front() {
            self.projections.remove(&request_id);
            self.revoke_projection_nonces(&request_id);
            self.remove_projection_idempotency(&request_id);
            self.projection_sessions.remove(&request_id);
            self.projection_deadlines.remove(&request_id);
        }
    }

    fn revoke_projection_nonces(&mut self, request_id: &str) {
        if let Some(action_keys) = self.projection_nonces.remove(request_id) {
            for action_key in action_keys {
                self.nonces.revoke(&action_key);
            }
        }
    }

    fn remove_projection_idempotency(&mut self, request_id: &str) {
        if let Some(key) = self.projection_idempotency.remove(request_id) {
            self.idempotency.remove(&key);
        }
    }

    fn forget_consumed_nonce(&mut self, action_key: &str) {
        for action_keys in self.projection_nonces.values_mut() {
            action_keys.retain(|key| key != action_key);
        }
        for (_, result) in self.idempotency.values_mut() {
            if let NotificationResult::Accepted { action_nonces, .. } = result {
                action_nonces.retain(|_, key| key != action_key);
            }
        }
    }

    fn prune_idempotency_nonces(&mut self) {
        let stale = self
            .idempotency
            .iter()
            .filter_map(|(key, (request_id, result))| {
                let NotificationResult::Accepted { action_nonces, .. } = result else {
                    return None;
                };
                action_nonces
                    .values()
                    .any(|action_key| !self.nonces.contains(action_key))
                    .then_some((key.clone(), request_id.clone()))
            })
            .collect::<Vec<_>>();
        for (key, request_id) in stale {
            self.idempotency.remove(&key);
            self.projections.remove(&request_id);
            self.revoke_projection_nonces(&request_id);
            self.projection_idempotency.remove(&request_id);
            self.projection_sessions.remove(&request_id);
            self.projection_deadlines.remove(&request_id);
            self.order.retain(|value| value != &request_id);
        }
    }

    fn gc_projections(&mut self, now_secs: u64) {
        let expired = self
            .projection_deadlines
            .iter()
            .filter_map(|(request_id, deadline)| {
                (*deadline <= now_secs).then_some(request_id.clone())
            })
            .collect::<Vec<_>>();
        for request_id in expired {
            self.close_by_request_id(&request_id);
        }
    }
}

impl core::fmt::Debug for NotificationSink {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("NotificationSink")
            .field("max_pending", &self.max_pending)
            .field("projection_len", &self.projections.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        admission::{test_observer, test_source, test_source_at_zone},
        types::{ActionSpec, Category},
    };

    #[derive(Default)]
    struct TestPort {
        next_id: u32,
        summaries: Vec<String>,
        actions: Vec<Vec<String>>,
    }

    impl DesktopNotificationPort for TestPort {
        fn activate(&mut self) -> Result<(), SinkError> {
            Ok(())
        }

        fn deactivate(&mut self) -> Result<(), SinkError> {
            Ok(())
        }

        fn notify(&mut self, notification: &SanitizedNotification) -> Result<u32, SinkError> {
            self.next_id = self.next_id.saturating_add(1);
            self.summaries.push(notification.summary().to_owned());
            self.actions.push(
                notification
                    .actions()
                    .iter()
                    .map(|(action, _)| action.clone())
                    .collect(),
            );
            Ok(self.next_id)
        }
    }

    struct FailingPort;

    impl DesktopNotificationPort for FailingPort {
        fn activate(&mut self) -> Result<(), SinkError> {
            Ok(())
        }

        fn deactivate(&mut self) -> Result<(), SinkError> {
            Ok(())
        }

        fn notify(&mut self, _notification: &SanitizedNotification) -> Result<u32, SinkError> {
            Err(SinkError::Unavailable)
        }
    }

    fn request_with_action() -> NotificationRequest {
        NotificationRequest::new("summary", "body", Category::SystemInfo)
            .unwrap()
            .with_actions(vec![ActionSpec::new("open", "Open").unwrap()])
            .unwrap()
    }

    #[test]
    fn delivery_requires_observer_purpose_and_returns_opaque_action_state() {
        let mut sink = NotificationSink::new(2, 2, 10);
        let mut port = TestPort::default();
        let source = test_source("guest");
        assert_eq!(
            sink.deliver(&mut port, &source, &source, request_with_action(), 100),
            Err(crate::types::NotificationError::Denied)
        );

        let observer = test_observer("alice");
        let result = sink
            .deliver(&mut port, &source, &observer, request_with_action(), 100)
            .unwrap();
        let action_key = match result {
            NotificationResult::Accepted { action_nonces, .. } => action_nonces["open"].clone(),
            other => panic!("unexpected result: {other:?}"),
        };
        assert_eq!(port.summaries, vec!["summary"]);
        assert_ne!(port.actions, vec![vec!["open".to_owned()]]);
        assert_eq!(
            sink.invoke_action(&action_key, &test_observer("bob"), 101),
            Err(ActionNonceError::SessionMismatch)
        );
        assert_eq!(
            sink.invoke_action_for(&action_key, &observer, "open", 101),
            Ok("open".to_owned())
        );
        assert_eq!(
            sink.invoke_action(&action_key, &observer, 101),
            Err(ActionNonceError::Unavailable)
        );
    }

    #[test]
    fn delivery_rejects_cross_zone_source_and_observer_sessions() {
        let mut sink = NotificationSink::new(2, 2, 10);
        let mut port = TestPort::default();
        assert_eq!(
            sink.deliver(
                &mut port,
                &test_source_at_zone("guest", 1, "other"),
                &test_observer("alice"),
                request_with_action(),
                100,
            ),
            Err(crate::types::NotificationError::Denied)
        );
    }

    #[test]
    fn session_close_revokes_projection_nonces_and_idempotency() {
        let mut sink = NotificationSink::new(2, 4, 10);
        let mut port = TestPort::default();
        let observer = test_observer("alice");
        let source = test_source("guest");
        let request = request_with_action().with_idempotency_key("same").unwrap();
        let result = sink
            .deliver(&mut port, &source, &observer, request.clone(), 100)
            .unwrap();
        let action_key = match result {
            NotificationResult::Accepted { action_nonces, .. } => action_nonces["open"].clone(),
            other => panic!("unexpected result: {other:?}"),
        };

        sink.close_session(&observer);
        assert_eq!(sink.projection_len(), 0);
        assert_eq!(
            sink.invoke_action(&action_key, &observer, 101),
            Err(ActionNonceError::Unavailable)
        );
        let replacement = sink
            .deliver(&mut port, &source, &observer, request, 102)
            .unwrap();
        let replacement_key = match replacement {
            NotificationResult::Accepted {
                notification_id,
                action_nonces,
            } => {
                assert_eq!(notification_id, 2);
                action_nonces["open"].clone()
            }
            other => panic!("unexpected replacement result: {other:?}"),
        };
        assert_ne!(replacement_key, action_key);
        assert_eq!(
            sink.invoke_action(&replacement_key, &observer, 103),
            Ok("open".to_owned())
        );
        assert_eq!(port.summaries, vec!["summary", "summary"]);
    }

    #[test]
    fn idempotent_retry_does_not_return_expired_action_capabilities() {
        let mut sink = NotificationSink::new(2, 4, 1);
        let mut port = TestPort::default();
        let source = test_source("guest");
        let observer = test_observer("alice");
        let request = request_with_action().with_idempotency_key("same").unwrap();
        let first = sink
            .deliver(&mut port, &source, &observer, request.clone(), 100)
            .unwrap();
        assert!(matches!(first, NotificationResult::Accepted { .. }));
        let second = sink
            .deliver(&mut port, &source, &observer, request, 102)
            .unwrap();
        assert!(matches!(
            second,
            NotificationResult::Accepted {
                notification_id: 2,
                ..
            }
        ));
    }

    #[test]
    fn failed_delivery_does_not_evict_the_previous_projection() {
        let mut sink = NotificationSink::new(1, 2, 10);
        let mut port = TestPort::default();
        let source = test_source("guest");
        let observer = test_observer("alice");
        let first = sink
            .deliver(&mut port, &source, &observer, request_with_action(), 100)
            .unwrap();
        let action_key = match first {
            NotificationResult::Accepted { action_nonces, .. } => action_nonces["open"].clone(),
            other => panic!("unexpected result: {other:?}"),
        };

        assert_eq!(
            sink.deliver(
                &mut FailingPort,
                &source,
                &observer,
                request_with_action(),
                101,
            )
            .unwrap(),
            NotificationResult::SinkUnavailable
        );
        assert_eq!(sink.projection_len(), 1);
        assert_eq!(
            sink.invoke_action_for(&action_key, &observer, "open", 102),
            Ok("open".to_owned())
        );
    }

    #[test]
    fn observer_policy_and_acknowledgement_timeout_are_enforced() {
        let mut disabled = NotificationSink::new_with_policy(2, 2, 10, 5, false);
        let mut port = TestPort::default();
        assert_eq!(
            disabled.deliver(
                &mut port,
                &test_source("guest"),
                &test_observer("alice"),
                request_with_action(),
                100,
            ),
            Err(crate::types::NotificationError::ObserverDisabled)
        );

        let mut sink = NotificationSink::new_with_policy(2, 2, 100, 5, true);
        let source = test_source("guest");
        let observer = test_observer("alice");
        let first = sink
            .deliver(&mut port, &source, &observer, request_with_action(), 100)
            .unwrap();
        let action_key = match first {
            NotificationResult::Accepted { action_nonces, .. } => action_nonces["open"].clone(),
            other => panic!("unexpected result: {other:?}"),
        };
        let second = sink
            .deliver(&mut port, &source, &observer, request_with_action(), 105)
            .unwrap();
        assert!(matches!(
            second,
            NotificationResult::Accepted {
                notification_id: 2,
                ..
            }
        ));
        assert_eq!(sink.projection_len(), 1);
        assert_eq!(
            sink.invoke_action(&action_key, &observer, 105),
            Err(ActionNonceError::Unavailable)
        );
    }
}

#[cfg(test)]
mod endpoint_tests {
    use super::*;
    use crate::admission::{
        NotificationEndpointBinding, NotificationEndpointError, NotificationEndpointEvidence,
        NotificationEndpointFence, NotificationEndpointPhase, NotificationEndpointRefusal,
        NotificationHostEndpoints, admit_notification_endpoint, notification_endpoint_bindings,
        test_observer, test_source,
    };
    use crate::types::{ActionSpec, Category, NotificationError};
    use d2b_contracts_resource::v3::{
        DesiredRevision, EndpointAttachmentKind, ResourceGeneration, ResourceRef, ResourceUid,
        StoreIncarnation, ZoneDesiredSequence, ZoneId, identity::ReconnectGeneration,
    };

    const SOURCE_UID: &str = "aaaaaaaa-0000-4000-8000-000000000001";
    const CONSUMER_UID: &str = "aaaaaaaa-0000-4000-8000-000000000002";
    const SOURCE_ENDPOINT: &str = "Endpoint/notification-guest-source";
    const DESKTOP_ENDPOINT: &str = "Endpoint/notification-desktop-sink";
    const CONSUMER: &str = "Process/notification-sink";

    /// A presentation port that records every notification it is asked to
    /// show, so a refused delivery is observable as "nothing was presented".
    #[derive(Default)]
    struct RecordingPortForEndpoint {
        presented: Vec<String>,
    }

    impl DesktopNotificationPort for RecordingPortForEndpoint {
        fn activate(&mut self) -> Result<(), SinkError> {
            Ok(())
        }

        fn deactivate(&mut self) -> Result<(), SinkError> {
            Ok(())
        }

        fn notify(&mut self, notification: &SanitizedNotification) -> Result<u32, SinkError> {
            self.presented
                .push(notification.summary().to_owned());
            Ok(u32::try_from(self.presented.len()).unwrap_or(u32::MAX))
        }
    }

    fn endpoints() -> NotificationHostEndpoints {
        NotificationHostEndpoints::new(
            ResourceRef::parse(SOURCE_ENDPOINT).expect("endpoint"),
            ResourceRef::parse(DESKTOP_ENDPOINT).expect("endpoint"),
        )
        .expect("declared endpoints")
    }

    fn consumer() -> ResourceRef {
        ResourceRef::parse(CONSUMER).expect("consumer")
    }

    fn fence() -> NotificationEndpointFence {
        NotificationEndpointFence::new(
            ZoneId::parse("dev").expect("zone"),
            StoreIncarnation::parse("store-one").expect("store"),
            DesiredRevision::INITIAL.try_next().expect("revision"),
            ZoneDesiredSequence::INITIAL.try_next().expect("sequence"),
            ResourceGeneration::new(3).expect("source generation"),
            ResourceGeneration::new(5).expect("consumer generation"),
            ReconnectGeneration::new(2).expect("reconnect"),
        )
    }

    fn evidence(fence: &NotificationEndpointFence) -> NotificationEndpointEvidence {
        NotificationEndpointEvidence {
            zone: fence.zone().clone(),
            store: fence.store().clone(),
            source_generation: fence.source_generation(),
            consumer_generation: fence.consumer_generation(),
            desired_revision: fence.desired_revision(),
            sequence: fence.sequence(),
            reconnect: ReconnectGeneration::new(7).expect("reconnect"),
        }
    }

    struct Gate {
        endpoints: NotificationHostEndpoints,
        consumer: ResourceRef,
        fence: NotificationEndpointFence,
        source_uid: ResourceUid,
        consumer_uid: ResourceUid,
    }

    impl Gate {
        fn new() -> Self {
            Self {
                endpoints: endpoints(),
                consumer: consumer(),
                fence: fence(),
                source_uid: ResourceUid::parse(SOURCE_UID).expect("source uid"),
                consumer_uid: ResourceUid::parse(CONSUMER_UID).expect("consumer uid"),
            }
        }

        fn gate<'a>(
            &'a self,
            binding: &'a NotificationEndpointBinding,
            fence: &'a NotificationEndpointFence,
            evidence: &'a NotificationEndpointEvidence,
        ) -> NotificationEndpointGate<'a> {
            NotificationEndpointGate {
                endpoints: &self.endpoints,
                consumer: &self.consumer,
                binding,
                fence,
                evidence,
                source_uid: &self.source_uid,
                consumer_uid: &self.consumer_uid,
            }
        }
    }

    fn desktop_binding() -> NotificationEndpointBinding {
        notification_endpoint_bindings(&endpoints(), &consumer())
            .expect("bindings")
            .into_iter()
            .find(|binding| binding.role() == NotificationEndpointRole::DesktopSink)
            .expect("desktop binding")
    }

    #[test]
    fn an_admitted_presentation_endpoint_delivers_the_unchanged_request() {
        let owned = Gate::new();
        let binding = desktop_binding();
        let live = evidence(&owned.fence);
        let gate = owned.gate(&binding, &owned.fence, &live);
        let mut sink = NotificationSink::new(8, 8, 120);
        let mut port = RecordingPortForEndpoint::default();
        let request = NotificationRequest::new("summary", "body", Category::SystemInfo)
            .expect("request");

        let result = sink
            .deliver_over_endpoint(
                &mut port,
                &test_source("one"),
                &test_observer("alice"),
                request,
                10,
                &gate,
            )
            .expect("delivery");

        assert!(matches!(result, NotificationResult::Accepted { .. }));
        assert_eq!(port.presented, vec!["summary".to_owned()]);
    }

    #[test]
    fn a_withdrawn_presentation_endpoint_stops_delivery_and_presents_nothing() {
        let owned = Gate::new();
        let binding = desktop_binding();
        let revoked = owned.fence.clone().revoke();
        let stale = evidence(&revoked);
        let gate = owned.gate(&binding, &revoked, &stale);
        let mut sink = NotificationSink::new(8, 8, 120);
        let mut port = RecordingPortForEndpoint::default();
        let request = NotificationRequest::new("summary", "body", Category::SystemInfo)
            .expect("request");

        let result = sink.deliver_over_endpoint(
            &mut port,
            &test_source("one"),
            &test_observer("alice"),
            request,
            10,
            &gate,
        );

        assert_eq!(result, Err(NotificationError::Denied));
        assert!(
            port.presented.is_empty(),
            "a withdrawn endpoint presented on another channel"
        );
    }

    #[test]
    fn a_guest_source_relationship_cannot_present_on_the_desktop_channel() {
        let owned = Gate::new();
        let guest_binding = notification_endpoint_bindings(&endpoints(), &consumer())
            .expect("bindings")
            .into_iter()
            .find(|binding| binding.role() == NotificationEndpointRole::GuestSource)
            .expect("guest binding");
        let live = evidence(&owned.fence);
        let gate = owned.gate(&guest_binding, &owned.fence, &live);
        let mut sink = NotificationSink::new(8, 8, 120);
        let mut port = RecordingPortForEndpoint::default();
        let request = NotificationRequest::new("summary", "body", Category::SystemInfo)
            .expect("request");

        let result = sink.deliver_over_endpoint(
            &mut port,
            &test_source("one"),
            &test_observer("alice"),
            request,
            10,
            &gate,
        );

        assert_eq!(result, Err(NotificationError::Denied));
        assert!(port.presented.is_empty());
    }

    #[test]
    fn notification_content_cannot_become_the_relationship_authority() {
        const CANARY: &str = "notif-authority-canary-3c91";
        let owned = Gate::new();
        let declared = desktop_binding();
        let live = evidence(&owned.fence);
        let gate = owned.gate(&declared, &owned.fence, &live);
        let mut sink = NotificationSink::new(8, 8, 120);
        let mut port = RecordingPortForEndpoint::default();
        let hostile = NotificationRequest::new(
            NotificationEndpointRole::DesktopSink.purpose(),
            NotificationEndpointRole::DesktopSink.slot(),
            Category::SystemInfo,
        )
        .expect("request")
        .with_actions(vec![
            ActionSpec::new(CANARY, NotificationEndpointRole::DesktopSink.slot())
                .expect("action"),
        ])
        .expect("actions")
        .with_idempotency_key(NotificationEndpointRole::DesktopSink.slot())
        .expect("idempotency key");

        let result = sink
            .deliver_over_endpoint(
                &mut port,
                &test_source("one"),
                &test_observer("alice"),
                hostile,
                10,
                &gate,
            )
            .expect("delivery");

        // The relationship the delivery ran under is the declared one, byte for
        // byte, and no content field reached it.
        let again = desktop_binding();
        assert_eq!(declared.request(), again.request());
        assert_eq!(
            declared.request().purpose().as_str(),
            NotificationEndpointRole::DesktopSink.purpose()
        );
        assert_eq!(
            declared.request().slot().as_str(),
            NotificationEndpointRole::DesktopSink.slot()
        );
        let rendered = format!("{:?}", declared.request());
        assert!(!rendered.contains(CANARY));
        // The opaque action capability is issued for the observer session, not
        // for the action's own label.
        let NotificationResult::Accepted { action_nonces, .. } = result else {
            panic!("delivery accepted");
        };
        let issued = action_nonces.get(CANARY).expect("action key");
        assert!(!issued.contains(CANARY));
        assert_ne!(issued, NotificationEndpointRole::DesktopSink.slot());
    }

    #[test]
    fn the_gate_refuses_foreign_and_stale_evidence_before_any_presentation() {
        let owned = Gate::new();
        let binding = desktop_binding();
        let mut refused = Vec::new();

        let foreign = NotificationEndpointEvidence {
            zone: ZoneId::parse("other").expect("zone"),
            ..evidence(&owned.fence)
        };
        let gate = owned.gate(&binding, &owned.fence, &foreign);
        refused.push(
            admit_notification_endpoint(&gate)
                .expect_err("foreign zone")
                .code()
                .to_owned(),
        );

        let mut stale_reconnect = evidence(&owned.fence);
        stale_reconnect.reconnect = ReconnectGeneration::new(1).expect("reconnect");
        let gate = owned.gate(&binding, &owned.fence, &stale_reconnect);
        refused.push(
            admit_notification_endpoint(&gate)
                .expect_err("stale reconnect")
                .code()
                .to_owned(),
        );

        let draining = owned.fence.clone().drain();
        let draining_evidence = evidence(&draining);
        let gate = owned.gate(&binding, &draining, &draining_evidence);
        refused.push(
            admit_notification_endpoint(&gate)
                .expect_err("draining")
                .code()
                .to_owned(),
        );

        assert_eq!(
            refused,
            vec![
                "notification-endpoint-foreign-zone",
                "notification-endpoint-stale-reconnect-generation",
                "notification-endpoint-relationship-draining",
            ]
        );
    }

    #[test]
    fn a_relationship_over_another_endpoint_is_a_conflicting_declaration() {
        let owned = Gate::new();
        let live = evidence(&owned.fence);
        let declared = desktop_binding();
        // Swap the declared presentation endpoint for the guest-source one.
        let forged_endpoints = NotificationHostEndpoints::new(
            ResourceRef::parse(SOURCE_ENDPOINT).expect("endpoint"),
            ResourceRef::parse(SOURCE_ENDPOINT).expect("endpoint"),
        )
        .expect("endpoints");
        let forged = notification_endpoint_bindings(&forged_endpoints, &consumer())
            .expect("bindings")
            .into_iter()
            .find(|binding| binding.role() == NotificationEndpointRole::DesktopSink)
            .expect("desktop binding");
        assert_ne!(forged.request(), declared.request());

        let gate = NotificationEndpointGate {
            endpoints: &owned.endpoints,
            consumer: &owned.consumer,
            binding: &forged,
            fence: &owned.fence,
            evidence: &live,
            source_uid: &owned.source_uid,
            consumer_uid: &owned.consumer_uid,
        };
        assert_eq!(
            admit_notification_endpoint(&gate).map_err(NotificationEndpointRefusal::code),
            Err("notification-endpoint-request-mismatch")
        );
        assert!(owned.endpoints.source_ref(NotificationEndpointRole::DesktopSink) != forged.source_ref());
    }

    #[test]
    fn every_declared_stream_carries_its_own_slot_and_purpose() {
        let bindings = notification_endpoint_bindings(&endpoints(), &consumer()).expect("bindings");
        let slots: Vec<&str> = bindings
            .iter()
            .map(|binding| binding.request().slot().as_str())
            .collect();
        assert_eq!(
            slots,
            vec![
                NotificationEndpointRole::GuestSource.slot(),
                NotificationEndpointRole::DesktopSink.slot()
            ]
        );
        for binding in &bindings {
            assert_eq!(binding.request().purpose().as_str(), binding.role().purpose());
            assert_eq!(binding.request().slot().as_str(), binding.role().slot());
            assert_eq!(binding.role().attachment(), EndpointAttachmentKind::Connect);
            assert_eq!(
                NotificationEndpointRole::ALL
                    .iter()
                    .filter(|role| role.stream() == binding.role().stream())
                    .count(),
                1
            );
        }
        assert_eq!(
            NotificationEndpointPhase::Admitted,
            NotificationEndpointPhase::Admitted
        );
        assert_eq!(
            NotificationEndpointError::SourceNotEndpoint.to_string(),
            "notification-endpoint-source-invalid"
        );
    }
}
