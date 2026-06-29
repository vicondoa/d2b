use std::collections::{BTreeMap, VecDeque};

use serde::{Deserialize, Serialize};

use crate::policy::{AttributionQuality, ReasonCode};

const MAX_AUDIT_MIME_BYTES: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditEvent {
    pub request_id: String,
    pub source_realm: String,
    pub destination_realm: String,
    #[serde(serialize_with = "serialize_bounded_mime")]
    pub mime_type: String,
    pub byte_count: u64,
    pub decision: AuditDecision,
    pub attribution: AttributionQuality,
    pub reason: ReasonCode,
    pub timestamp_unix_ms: u64,
}

pub fn bounded_mime(mime_type: &str) -> String {
    let normalized = crate::policy::normalize_mime(mime_type);
    if crate::policy::is_mime_allowed(&normalized) {
        return normalized;
    }
    let mut out = String::new();
    for ch in mime_type.chars() {
        if out.len() + ch.len_utf8() > MAX_AUDIT_MIME_BYTES {
            out.push('…');
            break;
        }
        if ch.is_ascii_graphic() || ch == ' ' {
            out.push(ch);
        } else {
            out.push('?');
        }
    }
    out
}

fn serialize_bounded_mime<S>(mime_type: &str, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(&bounded_mime(mime_type))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditDecision {
    Allow,
    Deny,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditQueueConfig {
    pub per_realm_quota: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditQueue {
    config: AuditQueueConfig,
    per_realm: BTreeMap<String, VecDeque<AuditEvent>>,
}

impl AuditQueue {
    pub fn new(config: AuditQueueConfig) -> Self {
        Self {
            config,
            per_realm: BTreeMap::new(),
        }
    }

    pub fn enqueue_fail_closed(&mut self, event: AuditEvent) -> Result<(), ReasonCode> {
        let queue = self
            .per_realm
            .entry(event.source_realm.clone())
            .or_default();
        if queue.len() >= self.config.per_realm_quota {
            return Err(ReasonCode::AuditFailure);
        }
        queue.push_back(event);
        Ok(())
    }

    pub fn len_for_realm(&self, realm: &str) -> usize {
        self.per_realm.get(realm).map_or(0, VecDeque::len)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetricEvent {
    pub name: MetricName,
    pub reason: Option<ReasonCode>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricName {
    PickerOpened,
    PickerTimeout,
    PolicyDenied,
    AuditQueueOverflow,
    DroppedDiagnostic,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetricsQueue {
    capacity: usize,
    queue: VecDeque<MetricEvent>,
    dropped: u64,
}

impl MetricsQueue {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            queue: VecDeque::new(),
            dropped: 0,
        }
    }

    pub fn enqueue_droppable(&mut self, event: MetricEvent) {
        if self.queue.len() >= self.capacity {
            self.dropped = self.dropped.saturating_add(1);
        } else {
            self.queue.push_back(event);
        }
    }

    pub fn dropped_count(&self) -> u64 {
        self.dropped
    }

    pub fn take_dropped_count(&mut self) -> u64 {
        let dropped = self.dropped;
        self.dropped = 0;
        dropped
    }

    pub fn len(&self) -> usize {
        self.queue.len()
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(realm: &str, request_id: &str) -> AuditEvent {
        AuditEvent {
            request_id: request_id.to_owned(),
            source_realm: realm.to_owned(),
            destination_realm: "host".to_owned(),
            mime_type: "text/plain".to_owned(),
            byte_count: 12,
            decision: AuditDecision::Allow,
            attribution: AttributionQuality::ExactClient,
            reason: ReasonCode::Allowed,
            timestamp_unix_ms: 1,
        }
    }

    #[test]
    fn audit_queue_is_fail_closed_per_realm() {
        let mut queue = AuditQueue::new(AuditQueueConfig { per_realm_quota: 1 });
        assert_eq!(queue.enqueue_fail_closed(event("vm-a", "r1")), Ok(()));
        assert_eq!(
            queue.enqueue_fail_closed(event("vm-a", "r2")),
            Err(ReasonCode::AuditFailure)
        );
        assert_eq!(queue.enqueue_fail_closed(event("vm-b", "r3")), Ok(()));
        assert_eq!(queue.len_for_realm("vm-a"), 1);
        assert_eq!(queue.len_for_realm("vm-b"), 1);
    }

    #[test]
    fn metrics_queue_drops_without_blocking() {
        let mut queue = MetricsQueue::new(1);
        queue.enqueue_droppable(MetricEvent {
            name: MetricName::PickerOpened,
            reason: None,
        });
        queue.enqueue_droppable(MetricEvent {
            name: MetricName::PickerTimeout,
            reason: Some(ReasonCode::PickerTimeout),
        });
        assert_eq!(queue.len(), 1);
        assert_eq!(queue.dropped_count(), 1);
    }

    #[test]
    fn audit_json_contains_metadata_not_payload_fields() {
        let json = serde_json::to_string(&event("vm-a", "r1")).expect("json");
        assert!(json.contains("source_realm"));
        assert!(!json.contains("preview"));
        assert!(!json.contains("payload"));
        assert!(!json.contains("clipboard"));
    }

    #[test]
    fn audit_mime_is_bounded_for_unrecognized_values() {
        let mut event = event("vm-a", "r1");
        event.mime_type = format!("application/x-secret;payload={}", "x".repeat(256));
        let json = serde_json::to_string(&event).expect("json");
        assert!(json.contains("application/x-secret"));
        assert!(!json.contains(&"x".repeat(128)));
    }

    #[test]
    fn metrics_queue_take_dropped_count_resets_counter() {
        let mut queue = MetricsQueue::new(0);
        queue.enqueue_droppable(MetricEvent {
            name: MetricName::PickerTimeout,
            reason: Some(ReasonCode::PickerTimeout),
        });
        assert_eq!(queue.take_dropped_count(), 1);
        assert_eq!(queue.take_dropped_count(), 0);
    }
}
