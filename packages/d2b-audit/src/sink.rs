//! Durable audit sink with class-specific failure behavior.

use std::path::Path;
use std::sync::Mutex;

use crate::{
    rate_limit::{
        AuditRateLimiter, AuditWriteClass, DEFAULT_AUDIT_WRITES_PER_SECOND, RateDecision,
    },
    record_types::AuditRecord,
    segment::{DEFAULT_MAX_SEGMENT_BYTES, DEFAULT_RETENTION_DAYS, SegmentWriter},
};

/// Sink result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditWriteOutcome {
    /// The record reached the segment and was synchronized.
    Written,
    /// A standard or best-effort record was dropped by rate limiting.
    RateLimited,
}

/// Sink failure. It intentionally does not contain paths or record payloads.
#[derive(Debug)]
pub enum AuditSinkError {
    /// The audit segment could not be opened or synchronized.
    Unavailable,
    /// Internal lock state was poisoned.
    StatePoisoned,
    /// A record could not be serialized.
    Serialization,
}

impl core::fmt::Display for AuditSinkError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::Unavailable => "audit-unavailable",
            Self::StatePoisoned => "audit-sink-state-poisoned",
            Self::Serialization => "audit-record-serialization-failed",
        })
    }
}

impl std::error::Error for AuditSinkError {}

/// A synchronized append-only audit sink.
pub struct AuditSink {
    writer: Mutex<SegmentWriter>,
    limiter: Mutex<AuditRateLimiter>,
}

impl core::fmt::Debug for AuditSink {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("AuditSink(<redacted>)")
    }
}

impl AuditSink {
    /// Open the default 64 MiB, 30-day sink.
    pub fn open(directory: impl AsRef<Path>) -> Result<Self, AuditSinkError> {
        Self::open_with_limits(
            directory,
            DEFAULT_MAX_SEGMENT_BYTES,
            DEFAULT_RETENTION_DAYS,
            DEFAULT_AUDIT_WRITES_PER_SECOND,
        )
    }

    /// Open with explicit segment, retention, and rate limits.
    pub fn open_with_limits(
        directory: impl AsRef<Path>,
        max_segment_bytes: u64,
        retention_days: u64,
        writes_per_second: u32,
    ) -> Result<Self, AuditSinkError> {
        let writer = SegmentWriter::open(directory, max_segment_bytes, retention_days)
            .map_err(|_| AuditSinkError::Unavailable)?;
        Ok(Self {
            writer: Mutex::new(writer),
            limiter: Mutex::new(AuditRateLimiter::new(writes_per_second)),
        })
    }

    /// Append one record under its durability class.
    pub fn append(
        &self,
        class: AuditWriteClass,
        record: &AuditRecord,
    ) -> Result<AuditWriteOutcome, AuditSinkError> {
        let decision = self
            .limiter
            .lock()
            .map_err(|_| AuditSinkError::StatePoisoned)?
            .admit(class);
        if decision == RateDecision::Limited {
            return Ok(AuditWriteOutcome::RateLimited);
        }
        let mut writer = self
            .writer
            .lock()
            .map_err(|_| AuditSinkError::StatePoisoned)?;
        writer
            .append(record)
            .map_err(|_| AuditSinkError::Unavailable)?;
        if class == AuditWriteClass::Privileged {
            writer.sync().map_err(|_| AuditSinkError::Unavailable)?;
        }
        Ok(AuditWriteOutcome::Written)
    }

    /// Prune old immutable segments.
    pub fn prune_old(&self, now_ms: u64) -> Result<usize, AuditSinkError> {
        self.writer
            .lock()
            .map_err(|_| AuditSinkError::StatePoisoned)?
            .prune_old(now_ms)
            .map_err(|_| AuditSinkError::Unavailable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        hash_chain::genesis_hash,
        record_types::{AuditRecord, AuditRecordFields, ProcessEffectFields},
    };

    fn sample() -> AuditRecord {
        AuditRecord::new(
            1,
            "work",
            "op",
            "corr",
            None,
            "test",
            genesis_hash(),
            AuditRecordFields::ProcessEffect(ProcessEffectFields {
                event: "launch".to_owned(),
                provider: "systemd".to_owned(),
                domain: "system".to_owned(),
                no_isolation: false,
                execution_ref_digest: "sha256:exec".to_owned(),
                process_uid: "uid".to_owned(),
                outcome: "ok".to_owned(),
                exit_class: None,
            }),
        )
        .unwrap()
    }

    #[test]
    fn privileged_writes_are_not_rate_limited() {
        let directory = std::env::temp_dir().join(format!("d2b-audit-sink-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        let sink = AuditSink::open_with_limits(&directory, 1024, 30, 1).unwrap();
        for _ in 0..8 {
            assert_eq!(
                sink.append(AuditWriteClass::Privileged, &sample()).unwrap(),
                AuditWriteOutcome::Written
            );
        }
        let _ = std::fs::remove_dir_all(directory);
    }
}
