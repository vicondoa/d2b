//! Durable audit sink with class-specific failure behavior.

use std::{
    fs,
    io::{self, BufRead},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    sync::Mutex,
};

use crate::{
    export::is_segment_name,
    hash_chain::{AuditHash, genesis_hash},
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
    /// A standard or best-effort record could not reach the segment.
    DroppedUnavailable,
}

/// Sink failure. It intentionally does not contain paths or record payloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditSinkError {
    /// The audit segment could not be opened or synchronized.
    Unavailable,
    /// Internal lock state was poisoned.
    StatePoisoned,
    /// A record could not be serialized.
    Serialization,
    /// The record does not name the sink's current chain head.
    ChainMismatch,
}

impl core::fmt::Display for AuditSinkError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::Unavailable => "audit-unavailable",
            Self::StatePoisoned => "audit-sink-state-poisoned",
            Self::Serialization => "audit-record-serialization-failed",
            Self::ChainMismatch => "audit-chain-mismatch",
        })
    }
}

impl std::error::Error for AuditSinkError {}

/// A synchronized append-only audit sink.
pub struct AuditSink {
    state: Mutex<SinkState>,
}

struct SinkState {
    writer: SegmentWriter,
    limiter: AuditRateLimiter,
    chain_head: AuditHash,
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
        let directory = directory.as_ref();
        fs::create_dir_all(directory).map_err(|_| AuditSinkError::Unavailable)?;
        let chain_head = scan_chain_head(directory)?;
        let writer = SegmentWriter::open(directory, max_segment_bytes, retention_days)
            .map_err(|_| AuditSinkError::Unavailable)?;
        Ok(Self {
            state: Mutex::new(SinkState {
                writer,
                limiter: AuditRateLimiter::new(writes_per_second),
                chain_head,
            }),
        })
    }

    /// Append one record under its durability class.
    pub fn append(
        &self,
        class: AuditWriteClass,
        record: &AuditRecord,
    ) -> Result<AuditWriteOutcome, AuditSinkError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| AuditSinkError::StatePoisoned)?;
        if record.previous_hash() != &state.chain_head {
            return Err(AuditSinkError::ChainMismatch);
        }
        let decision = state.limiter.admit(class);
        if decision == RateDecision::Limited {
            return Ok(AuditWriteOutcome::RateLimited);
        }
        record
            .to_json_line()
            .map_err(|_| AuditSinkError::Serialization)?;
        if state.writer.append(record).is_err() {
            return if class == AuditWriteClass::Privileged {
                Err(AuditSinkError::Unavailable)
            } else {
                Ok(AuditWriteOutcome::DroppedUnavailable)
            };
        }
        if class == AuditWriteClass::Privileged && state.writer.sync().is_err() {
            return Err(AuditSinkError::Unavailable);
        }
        state.chain_head = record.record_hash().clone();
        Ok(AuditWriteOutcome::Written)
    }

    /// Prune old immutable segments.
    pub fn prune_old(&self, now_ms: u64) -> Result<usize, AuditSinkError> {
        self.state
            .lock()
            .map_err(|_| AuditSinkError::StatePoisoned)?
            .writer
            .prune_old(now_ms)
            .map_err(|_| AuditSinkError::Unavailable)
    }

    /// Return the hash of the record currently at the end of the sink.
    pub fn chain_head(&self) -> Result<AuditHash, AuditSinkError> {
        self.state
            .lock()
            .map(|state| state.chain_head.clone())
            .map_err(|_| AuditSinkError::StatePoisoned)
    }
}

fn scan_chain_head(directory: &Path) -> Result<AuditHash, AuditSinkError> {
    let mut paths = fs::read_dir(directory)
        .map_err(|_| AuditSinkError::Unavailable)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|value| value.to_str())
                .is_some_and(is_segment_name)
        })
        .collect::<Vec<PathBuf>>();
    paths.sort();

    let mut previous = genesis_hash();
    for path in paths {
        let file = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
            .open(path)
            .map_err(|_| AuditSinkError::Unavailable)?;
        let mut reader = io::BufReader::new(file);
        loop {
            let mut bytes = Vec::new();
            let read = reader
                .read_until(b'\n', &mut bytes)
                .map_err(|_| AuditSinkError::Unavailable)?;
            if read == 0 {
                break;
            }
            if bytes.last() != Some(&b'\n') {
                return Err(AuditSinkError::ChainMismatch);
            }
            bytes.pop();
            let line = String::from_utf8(bytes).map_err(|_| AuditSinkError::ChainMismatch)?;
            let record = serde_json::from_str::<AuditRecord>(&line)
                .map_err(|_| AuditSinkError::ChainMismatch)?;
            record
                .verify(&previous)
                .map_err(|_| AuditSinkError::ChainMismatch)?;
            previous = record.record_hash().clone();
        }
    }
    Ok(previous)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        hash_chain::genesis_hash,
        record_types::{AuditRecord, AuditRecordFields, ProcessEffectFields},
    };

    fn sample(previous_hash: crate::AuditHash) -> AuditRecord {
        AuditRecord::new(
            1,
            "work",
            "op",
            "corr",
            None,
            "test",
            previous_hash,
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
        let mut previous = genesis_hash();
        for _ in 0..8 {
            let record = sample(previous);
            assert_eq!(
                sink.append(AuditWriteClass::Privileged, &record).unwrap(),
                AuditWriteOutcome::Written
            );
            previous = record.record_hash().clone();
        }
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn sink_rejects_an_invalid_predecessor_chain() {
        let directory =
            std::env::temp_dir().join(format!("d2b-audit-chain-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        let sink = AuditSink::open_with_limits(&directory, 1024, 30, 8).unwrap();
        let first = sample(genesis_hash());
        sink.append(AuditWriteClass::Privileged, &first).unwrap();
        let invalid = sample(genesis_hash());
        assert_eq!(
            sink.append(AuditWriteClass::Privileged, &invalid)
                .unwrap_err()
                .to_string(),
            "audit-chain-mismatch"
        );
        assert_eq!(sink.chain_head().unwrap(), *first.record_hash());
        let _ = std::fs::remove_dir_all(directory);
    }
}
