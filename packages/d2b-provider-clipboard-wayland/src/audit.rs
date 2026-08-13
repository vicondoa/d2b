//! Fail-closed, payload-free clipboard audit.

use sha2::{Digest, Sha256};
use std::collections::VecDeque;

/// Closed clipboard operation event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardEventType {
    /// Host selection captured.
    HostCapture,
    /// Guest selection captured.
    GuestCapture,
    /// Paste was authorized.
    PasteAuthorized,
    /// Paste was rejected.
    PasteRejected,
    /// A selection was suppressed as an echo.
    EchoSuppressed,
    /// A history entry expired.
    EntryExpired,
    /// Guest lifecycle purged entries.
    EntryPurged,
    /// A picker session started.
    PickerSessionStarted,
    /// A picker session completed.
    PickerSessionCompleted,
    /// A picker session failed.
    PickerSessionFailed,
}

/// Closed clipboard reason code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardReason {
    /// The operation was allowed.
    Allowed,
    /// MIME was not in the allowlist.
    MimeRejected,
    /// A secret-hint MIME suppressed capture.
    SecretHintMime,
    /// The item exceeded the bound.
    ItemTooLarge,
    /// History quota could not be satisfied.
    TotalQuotaExceeded,
    /// FD capacity was exhausted.
    FdCountExceeded,
    /// FD transfer timed out.
    FdWriteTimeout,
    /// Ancillary data was truncated.
    MsgCtrunc,
    /// FD metadata failed validation.
    FdSafetyViolation,
    /// Guest rate limit was exceeded.
    RateLimitExceeded,
    /// Picker timed out.
    PickerTimedOut,
    /// Picker was cancelled.
    PickerCancelled,
    /// Picker failed to start.
    PickerStartFailed,
    /// Display dependency was absent.
    DependencyAbsent,
    /// Display dependency was degraded.
    DependencyDegraded,
    /// Guest is suspended.
    ZoneSuspended,
    /// Authorization failed.
    Unauthorized,
    /// Cross-Zone transfer is denied.
    CrossZoneDenied,
}

impl ClipboardReason {
    /// Return the stable reason label.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Allowed => "allowed",
            Self::MimeRejected => "mime-rejected",
            Self::SecretHintMime => "secret-hint-mime",
            Self::ItemTooLarge => "item-too-large",
            Self::TotalQuotaExceeded => "total-quota-exceeded",
            Self::FdCountExceeded => "fd-count-exceeded",
            Self::FdWriteTimeout => "fd-write-timeout",
            Self::MsgCtrunc => "msg-ctrunc",
            Self::FdSafetyViolation => "fd-safety-violation",
            Self::RateLimitExceeded => "rate-limit-exceeded",
            Self::PickerTimedOut => "picker-timed-out",
            Self::PickerCancelled => "picker-cancelled",
            Self::PickerStartFailed => "picker-start-failed",
            Self::DependencyAbsent => "dependency-absent",
            Self::DependencyDegraded => "dependency-degraded",
            Self::ZoneSuspended => "zone-suspended",
            Self::Unauthorized => "unauthorized",
            Self::CrossZoneDenied => "cross-zone-denied",
        }
    }
}

/// Discretized clipboard size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SizeBucket {
    /// Less than one KiB.
    Lt1K,
    /// One to 64 KiB.
    K1To64K,
    /// 64 KiB to one MiB.
    K64ToM1,
    /// Greater than one MiB.
    GtM1,
}

impl SizeBucket {
    /// Classify a byte count without retaining the exact value.
    pub const fn from_len(length: usize) -> Self {
        match length {
            0..=1023 => Self::Lt1K,
            1024..=65_535 => Self::K1To64K,
            65_536..=1_048_575 => Self::K64ToM1,
            _ => Self::GtM1,
        }
    }
}

/// Content-free clipboard audit event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardAuditEvent {
    event_type: ClipboardEventType,
    source_zone_digest: String,
    dest_zone_digest: String,
    reason: ClipboardReason,
    size_bucket: SizeBucket,
}

impl ClipboardAuditEvent {
    /// Construct an event with identity digests and a size bucket.
    pub fn new(
        source_zone: &str,
        dest_zone: &str,
        reason: ClipboardReason,
        size_bucket: SizeBucket,
    ) -> Self {
        Self {
            event_type: ClipboardEventType::PasteAuthorized,
            source_zone_digest: digest(source_zone),
            dest_zone_digest: digest(dest_zone),
            reason,
            size_bucket,
        }
    }

    /// Set the event type.
    pub const fn with_event_type(mut self, event_type: ClipboardEventType) -> Self {
        self.event_type = event_type;
        self
    }

    /// Render the bounded wire record.
    pub fn to_wire(&self) -> String {
        format!(
            "event={} source={} dest={} reason={} size={:?}",
            format!("{:?}", self.event_type).to_ascii_lowercase(),
            self.source_zone_digest,
            self.dest_zone_digest,
            self.reason.as_str(),
            self.size_bucket
        )
    }
}

/// Bounded fail-closed audit queue.
pub struct ClipboardAuditQueue {
    capacity: usize,
    entries: VecDeque<ClipboardAuditEvent>,
}

impl ClipboardAuditQueue {
    /// Construct a queue with a fixed capacity.
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            entries: VecDeque::new(),
        }
    }

    /// Append an event, refusing the operation when the queue is full.
    pub fn push(&mut self, event: ClipboardAuditEvent) -> Result<(), ClipboardReason> {
        if self.entries.len() >= self.capacity {
            return Err(ClipboardReason::FdCountExceeded);
        }
        self.entries.push_back(event);
        Ok(())
    }

    /// Render all queued events.
    pub fn to_wire(&self) -> String {
        self.entries
            .iter()
            .map(ClipboardAuditEvent::to_wire)
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Return queue length.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the queue is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Whether the queue is at capacity.
    pub fn is_full(&self) -> bool {
        self.entries.len() >= self.capacity
    }
}

fn digest(value: &str) -> String {
    format!("sha256:{:x}", Sha256::digest(value.as_bytes()))
}
