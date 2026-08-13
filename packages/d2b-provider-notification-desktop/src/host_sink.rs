//! In-memory host notification sink and observer projection.

use crate::{
    action_nonce::{ActionNonceError, ActionNonceStore},
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
    projections: BTreeMap<String, NotificationProjection>,
    order: VecDeque<String>,
    nonces: ActionNonceStore,
}

impl NotificationSink {
    /// Construct a host sink with bounded queue and nonce state.
    pub fn new(max_pending: usize, nonce_capacity: usize, nonce_ttl_secs: u64) -> Self {
        Self {
            max_pending,
            projections: BTreeMap::new(),
            order: VecDeque::new(),
            nonces: ActionNonceStore::new(nonce_capacity, nonce_ttl_secs),
        }
    }

    /// Deliver one request through the effect port.
    pub fn deliver<P: DesktopNotificationPort>(
        &mut self,
        port: &mut P,
        source_session: &str,
        request: NotificationRequest,
        now_secs: u64,
    ) -> Result<NotificationResult, crate::types::NotificationError> {
        if self.projections.len() >= self.max_pending {
            self.evict_oldest();
        }
        let notification = request.sanitize()?;
        if notification.actions().len() > self.nonces.available_capacity() {
            return Ok(NotificationResult::CapacityExceeded);
        }
        let notification_id = match port.notify(&notification) {
            Ok(id) => id,
            Err(_) => return Ok(NotificationResult::SinkUnavailable),
        };
        let request_id = format!("notification-{notification_id}");
        let mut action_nonces = BTreeMap::new();
        for (action_id, _) in notification.actions() {
            let nonce = self
                .nonces
                .register(source_session, action_id, now_secs)
                .map_err(|error| match error {
                    ActionNonceError::Capacity => crate::types::NotificationError::InvalidActions,
                    _ => crate::types::NotificationError::InvalidOpaqueKey,
                })?;
            action_nonces.insert(action_id.clone(), nonce.action_key());
        }
        self.order.push_back(request_id.clone());
        self.projections.insert(
            request_id.clone(),
            NotificationProjection {
                request_id,
                notification,
            },
        );
        Ok(NotificationResult::Accepted {
            notification_id,
            action_nonces,
        })
    }

    /// Consume one observer action capability.
    pub fn invoke_action(
        &mut self,
        action_key: &str,
        observer_session: &str,
        now_secs: u64,
    ) -> Result<String, ActionNonceError> {
        self.nonces.consume(action_key, observer_session, now_secs)
    }

    /// Consume an action capability with an explicit action ID check.
    pub fn invoke_action_for(
        &mut self,
        action_key: &str,
        observer_session: &str,
        action_id: &str,
        now_secs: u64,
    ) -> Result<String, ActionNonceError> {
        self.nonces
            .consume_for_action(action_key, observer_session, Some(action_id), now_secs)
    }

    /// Evict a projection when its desktop notification closes.
    pub fn close(&mut self, notification_id: u32) {
        let request_id = format!("notification-{notification_id}");
        self.projections.remove(&request_id);
        self.order.retain(|value| value != &request_id);
    }

    /// Drain all transient state during restart or shutdown.
    pub fn drain(&mut self) {
        self.projections.clear();
        self.order.clear();
        self.nonces.clear();
    }

    /// Return the current projection size.
    pub fn projection_len(&self) -> usize {
        self.projections.len()
    }

    fn evict_oldest(&mut self) {
        if let Some(request_id) = self.order.pop_front() {
            self.projections.remove(&request_id);
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
