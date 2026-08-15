//! In-memory host notification sink and observer projection.

use crate::{
    GuestSource, NotificationProviderConfig,
    action_nonce::{ActionNonceError, ActionNonceStore},
    admission::SessionEvidence,
    redact::SanitizedNotification,
    types::NotificationRequest,
};
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
    /// Present one sanitized notification and return an opaque desktop ID.
    fn notify(&mut self, notification: &SanitizedNotification) -> Result<u32, SinkError>;
}

impl<T: DesktopNotificationPort + ?Sized> DesktopNotificationPort for Box<T> {
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
        source_session
            .admit_source()
            .map_err(|_| crate::types::NotificationError::InvalidOpaqueKey)?;
        if !self.observer_enabled {
            return Err(crate::types::NotificationError::ObserverDisabled);
        }
        observer_session
            .admit_observer()
            .map_err(|_| crate::types::NotificationError::InvalidOpaqueKey)?;
        if source_session.zone() != observer_session.zone() {
            return Err(crate::types::NotificationError::InvalidOpaqueKey);
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
            Err(_) => {
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
        self.projection_nonces.insert(request_id, issued_keys);
        self.projection_sessions
            .insert(format!("notification-{notification_id}"), observer_session);
        self.projection_deadlines.insert(
            format!("notification-{notification_id}"),
            now_secs.saturating_add(self.acknowledge_timeout_secs),
        );
        let result = NotificationResult::Accepted {
            notification_id,
            action_nonces,
        };
        if let Some(key) = idempotency_key {
            self.idempotency.insert(
                key.clone(),
                (format!("notification-{notification_id}"), result.clone()),
            );
            self.projection_idempotency
                .insert(format!("notification-{notification_id}"), key);
        }
        Ok(result)
    }

    /// Deliver after the configured Guest-source category admission.
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
            .map_err(|_| crate::types::NotificationError::InvalidOpaqueKey)?;
        self.deliver(port, source_session, observer_session, request, now_secs)
    }

    /// Consume one observer action capability.
    pub fn invoke_action(
        &mut self,
        action_key: &str,
        observer_session: &SessionEvidence,
        now_secs: u64,
    ) -> Result<String, ActionNonceError> {
        observer_session
            .admit_observer()
            .map_err(|_| ActionNonceError::SessionMismatch)?;
        let observer_session = observer_session.session_key();
        let result = self.nonces.consume(action_key, &observer_session, now_secs);
        if result.is_ok() {
            self.forget_consumed_nonce(action_key);
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
        observer_session
            .admit_observer()
            .map_err(|_| ActionNonceError::SessionMismatch)?;
        let observer_session = observer_session.session_key();
        let result = self.nonces.consume_for_action(
            action_key,
            &observer_session,
            Some(action_id),
            now_secs,
        );
        if result.is_ok() {
            self.forget_consumed_nonce(action_key);
        }
        result
    }

    /// Evict a projection when its desktop notification closes.
    pub fn close(&mut self, notification_id: u32) {
        let request_id = format!("notification-{notification_id}");
        self.projections.remove(&request_id);
        self.revoke_projection_nonces(&request_id);
        self.remove_projection_idempotency(&request_id);
        self.projection_sessions.remove(&request_id);
        self.projection_deadlines.remove(&request_id);
        self.order.retain(|value| value != &request_id);
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
            if let Some(notification_id) = request_id
                .strip_prefix("notification-")
                .and_then(|value| value.parse::<u32>().ok())
            {
                self.close(notification_id);
            }
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
            if let Some(notification_id) = request_id
                .strip_prefix("notification-")
                .and_then(|value| value.parse::<u32>().ok())
            {
                self.close(notification_id);
            }
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
            Err(crate::types::NotificationError::InvalidOpaqueKey)
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
            Err(crate::types::NotificationError::InvalidOpaqueKey)
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
