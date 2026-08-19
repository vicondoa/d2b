//! Durable audit sink with class-specific failure behavior.

use std::{
    collections::BTreeMap,
    fs,
    io::{self, BufRead},
    os::unix::fs::OpenOptionsExt,
    path::Path,
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::{
    export::is_segment_name,
    hash_chain::AuditHash,
    operation::ZoneOperationKey,
    rate_limit::{
        AuditRateLimiter, AuditWriteClass, DEFAULT_AUDIT_WRITES_PER_SECOND, RateDecision,
    },
    record_types::AuditRecord,
    segment::{DEFAULT_MAX_SEGMENT_BYTES, DEFAULT_RETENTION_DAYS, FailureInjector, SegmentWriter},
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
    /// The sink was poisoned after an ambiguous post-write failure.
    Poisoned,
}

impl core::fmt::Display for AuditSinkError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::Unavailable => "audit-unavailable",
            Self::StatePoisoned => "audit-sink-state-poisoned",
            Self::Serialization => "audit-record-serialization-failed",
            Self::ChainMismatch => "audit-chain-mismatch",
            Self::Poisoned => "audit-sink-poisoned",
        })
    }
}

impl std::error::Error for AuditSinkError {}

/// A synchronized append-only audit sink.
pub struct AuditSink {
    state: Mutex<SinkState>,
}

const MAX_AUDIT_DIRECTORY_ENTRIES: usize = 4096;

struct SinkState {
    writer: SegmentWriter,
    limiter: AuditRateLimiter,
    chain_head: AuditHash,
    durable_mutations: BTreeMap<(ZoneOperationKey, String), AuditHash>,
    mutation_predecessors: BTreeMap<(ZoneOperationKey, String), AuditHash>,
    poisoned: bool,
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
        Self::open_with_writer(
            directory,
            max_segment_bytes,
            retention_days,
            writes_per_second,
            None,
        )
    }

    /// Open with deterministic segment durability fault injection.
    pub fn open_with_injector(
        directory: impl AsRef<Path>,
        max_segment_bytes: u64,
        retention_days: u64,
        writes_per_second: u32,
        injector: FailureInjector,
    ) -> Result<Self, AuditSinkError> {
        Self::open_with_writer(
            directory,
            max_segment_bytes,
            retention_days,
            writes_per_second,
            Some(injector),
        )
    }

    fn open_with_writer(
        directory: impl AsRef<Path>,
        max_segment_bytes: u64,
        retention_days: u64,
        writes_per_second: u32,
        injector: Option<FailureInjector>,
    ) -> Result<Self, AuditSinkError> {
        let directory = directory.as_ref();
        if !directory.is_absolute() {
            return Err(AuditSinkError::Unavailable);
        }
        fs::create_dir_all(directory).map_err(|_| AuditSinkError::Unavailable)?;
        let writer = match injector {
            Some(injector) => SegmentWriter::open_at_with_injector(
                directory,
                max_segment_bytes,
                retention_days,
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
                    .unwrap_or(0),
                injector,
            ),
            None => SegmentWriter::open(directory, max_segment_bytes, retention_days),
        }
        .map_err(|_| AuditSinkError::Unavailable)?;
        // SegmentWriter acquires the directory lock and runs retention before
        // returning. Rebuild every derived index only after that point so a
        // prune or restart cannot leave a stale chain head behind.
        let scan = scan_chain_state(writer.directory())?;
        Ok(Self {
            state: Mutex::new(SinkState {
                writer,
                limiter: AuditRateLimiter::new(writes_per_second),
                chain_head: scan.chain_head,
                durable_mutations: scan.durable_mutations,
                mutation_predecessors: scan.mutation_predecessors,
                poisoned: false,
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
        if state.poisoned {
            return Err(AuditSinkError::Poisoned);
        }
        let mutation_key = record.mutation_id().and_then(|mutation_id| {
            record
                .zone_operation_key()
                .ok()
                .map(|key| (key, mutation_id.to_owned()))
        });
        if let Some(key) = &mutation_key
            && let Some(existing) = state.durable_mutations.get(key)
        {
            if existing == record.record_hash() {
                return Ok(AuditWriteOutcome::Written);
            }
            return Err(AuditSinkError::ChainMismatch);
        }
        if record.previous_hash() != &state.chain_head {
            return Err(AuditSinkError::ChainMismatch);
        }
        let decision = state.limiter.admit(class);
        if decision == RateDecision::Limited {
            return Ok(AuditWriteOutcome::RateLimited);
        }
        let line = record
            .to_json_line()
            .map_err(|_| AuditSinkError::Serialization)?;
        if state.writer.append_serialized(&line).is_err() {
            if !state.writer.append_state_consistent() {
                state.poisoned = true;
            }
            if state.poisoned {
                return Err(AuditSinkError::Poisoned);
            }
            return if class == AuditWriteClass::Privileged {
                Err(AuditSinkError::Unavailable)
            } else {
                Ok(AuditWriteOutcome::DroppedUnavailable)
            };
        }
        if class == AuditWriteClass::Privileged && state.writer.sync().is_err() {
            state.poisoned = true;
            return Err(AuditSinkError::Unavailable);
        }
        state.chain_head = record.record_hash().clone();
        if let Some(key) = mutation_key {
            let chain_head = state.chain_head.clone();
            state
                .mutation_predecessors
                .insert(key.clone(), record.previous_hash().clone());
            state.durable_mutations.insert(key, chain_head);
        }
        Ok(AuditWriteOutcome::Written)
    }

    /// Prune old immutable segments.
    pub fn prune_old(&self, now_ms: u64) -> Result<usize, AuditSinkError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| AuditSinkError::StatePoisoned)?;
        let removed = match state.writer.prune_old(now_ms) {
            Ok(removed) => removed,
            Err(_error) => return Err(AuditSinkError::Unavailable),
        };
        if removed > 0 {
            let scan = scan_chain_state(state.writer.directory()).map_err(|_| {
                state.poisoned = true;
                AuditSinkError::Poisoned
            })?;
            state.chain_head = scan.chain_head;
            state.durable_mutations = scan.durable_mutations;
            state.mutation_predecessors = scan.mutation_predecessors;
        }
        Ok(removed)
    }

    /// Export while holding the same writer/pruner lock used by append and
    /// retention. This prevents a page from observing a half-published
    /// checkpoint or a segment set that changes during the read.
    pub fn export_segments(
        &self,
        after: Option<&str>,
        before: Option<&str>,
    ) -> Result<Vec<crate::ExportLine>, AuditSinkError> {
        let state = self
            .state
            .lock()
            .map_err(|_| AuditSinkError::StatePoisoned)?;
        if state.poisoned {
            return Err(AuditSinkError::Poisoned);
        }
        crate::export::export_segments_range(state.writer.directory(), after, before)
            .map_err(|_| AuditSinkError::Unavailable)
    }

    /// Return the hash of the record currently at the end of the sink.
    pub fn chain_head(&self) -> Result<AuditHash, AuditSinkError> {
        self.state
            .lock()
            .map(|state| state.chain_head.clone())
            .map_err(|_| AuditSinkError::StatePoisoned)
    }

    /// Whether automatic retention needs a retry.
    pub fn retention_degraded(&self) -> Result<bool, AuditSinkError> {
        self.state
            .lock()
            .map(|state| state.writer.retention_degraded())
            .map_err(|_| AuditSinkError::StatePoisoned)
    }

    /// Return the durable hash recorded for one resource mutation identity.
    ///
    /// Recovery uses this lookup after an append-before-clear crash so it can
    /// advance from the already durable record instead of rebuilding it with
    /// the current chain head.
    pub fn mutation_record_hash(
        &self,
        key: &ZoneOperationKey,
        mutation_id: &str,
    ) -> Result<Option<AuditHash>, AuditSinkError> {
        self.state
            .lock()
            .map(|state| {
                state
                    .durable_mutations
                    .get(&(key.clone(), mutation_id.to_owned()))
                    .cloned()
            })
            .map_err(|_| AuditSinkError::StatePoisoned)
    }

    /// Return the predecessor hash recorded for one durable mutation.
    pub fn mutation_record_predecessor(
        &self,
        key: &ZoneOperationKey,
        mutation_id: &str,
    ) -> Result<Option<AuditHash>, AuditSinkError> {
        self.state
            .lock()
            .map(|state| {
                state
                    .mutation_predecessors
                    .get(&(key.clone(), mutation_id.to_owned()))
                    .cloned()
            })
            .map_err(|_| AuditSinkError::StatePoisoned)
    }
}

struct ScanState {
    chain_head: AuditHash,
    durable_mutations: BTreeMap<(ZoneOperationKey, String), AuditHash>,
    mutation_predecessors: BTreeMap<(ZoneOperationKey, String), AuditHash>,
}

fn scan_chain_state(directory: &Path) -> Result<ScanState, AuditSinkError> {
    let mut paths = Vec::new();
    for (index, entry) in fs::read_dir(directory)
        .map_err(|_| AuditSinkError::Unavailable)?
        .enumerate()
    {
        if index >= MAX_AUDIT_DIRECTORY_ENTRIES {
            return Err(AuditSinkError::Unavailable);
        }
        let entry = entry.map_err(|_| AuditSinkError::Unavailable)?;
        let path = entry.path();
        if path
            .file_name()
            .and_then(|value| value.to_str())
            .is_some_and(is_segment_name)
        {
            paths.push(path);
        }
    }
    paths.sort();

    if crate::segment::checkpoint_pending(directory).map_err(|_| AuditSinkError::ChainMismatch)? {
        return Err(AuditSinkError::ChainMismatch);
    }
    let mut previous =
        crate::segment::checkpoint_anchor(directory).map_err(|_| AuditSinkError::ChainMismatch)?;
    let mut durable_mutations = BTreeMap::new();
    let mut mutation_predecessors = BTreeMap::new();
    for path in paths {
        let file = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
            .open(path)
            .map_err(|_| AuditSinkError::Unavailable)?;
        let mut reader = io::BufReader::new(file);
        while let Some(bytes) =
            read_bounded_line(&mut reader).map_err(|_| AuditSinkError::ChainMismatch)?
        {
            let line = String::from_utf8(bytes).map_err(|_| AuditSinkError::ChainMismatch)?;
            let record = serde_json::from_str::<AuditRecord>(&line)
                .map_err(|_| AuditSinkError::ChainMismatch)?;
            record
                .verify(&previous)
                .map_err(|_| AuditSinkError::ChainMismatch)?;
            previous = record.record_hash().clone();
            if let Some(mutation_id) = record.mutation_id()
                && let Ok(key) = record.zone_operation_key()
            {
                let key = (key, mutation_id.to_owned());
                if durable_mutations
                    .insert(key, record.record_hash().clone())
                    .is_some()
                {
                    return Err(AuditSinkError::ChainMismatch);
                }
                if let Some(mutation_id) = record.mutation_id()
                    && let Ok(key) = record.zone_operation_key()
                {
                    mutation_predecessors.insert(
                        (key, mutation_id.to_owned()),
                        record.previous_hash().clone(),
                    );
                }
            }
        }
    }
    Ok(ScanState {
        chain_head: previous,
        durable_mutations,
        mutation_predecessors,
    })
}

fn read_bounded_line<R: BufRead>(reader: &mut R) -> io::Result<Option<Vec<u8>>> {
    let mut bytes = Vec::new();
    loop {
        let chunk = reader.fill_buf()?;
        if chunk.is_empty() {
            return if bytes.is_empty() {
                Ok(None)
            } else {
                Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "audit-scan-line-truncated",
                ))
            };
        }
        let newline = chunk.iter().position(|byte| *byte == b'\n');
        let take = newline.map_or(chunk.len(), |index| index + 1);
        if bytes.len().saturating_add(take) > crate::export::MAX_EXPORT_LINE_BYTES {
            return Err(io::Error::other("audit-scan-line-limit"));
        }
        bytes.extend_from_slice(&chunk[..take]);
        reader.consume(take);
        if newline.is_some() {
            bytes.pop();
            return Ok(Some(bytes));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        hash_chain::genesis_hash,
        record_types::{
            AuditRecord, AuditRecordFields, ProcessEffectFields, ResourceMutationFields,
            test_support,
        },
        segment::{FailureInjector, FailurePoint},
    };

    fn writable_manifest_dir() -> std::path::PathBuf {
        std::env::var_os("TEST_TMPDIR")
            .map(std::path::PathBuf::from)
            .or_else(|| std::env::var_os("CARGO_MANIFEST_DIR").map(std::path::PathBuf::from))
            .or_else(|| std::env::current_dir().ok())
            .expect("resolve test writable directory")
    }

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
                execution_ref_digest:
                    "sha256:0000000000000000000000000000000000000000000000000000000000000001"
                        .to_owned(),
                process_uid: "uid".to_owned(),
                outcome: "ok".to_owned(),
                exit_class: None,
            }),
        )
        .unwrap()
    }

    #[test]
    fn privileged_writes_are_not_rate_limited() {
        let directory = writable_manifest_dir()
            .join("target")
            .join(format!("d2b-audit-sink-{}", std::process::id()));
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
    fn sink_serializes_each_append_once() {
        let directory = writable_manifest_dir().join("target").join(format!(
            "d2b-audit-sink-serialization-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        let sink = AuditSink::open_with_limits(&directory, 1024, 30, 8).unwrap();
        let record = sample(genesis_hash());
        test_support::reset_json_line_serialization_count();

        assert_eq!(
            sink.append(AuditWriteClass::Privileged, &record).unwrap(),
            AuditWriteOutcome::Written
        );
        assert_eq!(
            test_support::json_line_serialization_count(),
            1,
            "one serialized line should serve the sink and segment writer"
        );
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn sink_rejects_an_invalid_predecessor_chain() {
        let directory = writable_manifest_dir()
            .join("target")
            .join(format!("d2b-audit-chain-{}", std::process::id()));
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

    #[test]
    fn privileged_success_requires_every_segment_durability_step() {
        for point in [
            FailurePoint::Append,
            FailurePoint::DataSync,
            FailurePoint::ParentSync,
        ] {
            let directory = writable_manifest_dir().join("target").join(format!(
                "d2b-audit-sink-fault-{point:?}-{}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&directory);
            let injector = FailureInjector::default();
            injector.fail_next(point);
            let sink = AuditSink::open_with_injector(&directory, 1024, 30, 1, injector).unwrap();
            let record = sample(genesis_hash());
            assert_eq!(
                sink.append(AuditWriteClass::Privileged, &record)
                    .unwrap_err(),
                AuditSinkError::Unavailable
            );
            let _ = std::fs::remove_dir_all(directory);
        }
    }

    #[test]
    fn startup_retention_rebuilds_chain_head_from_retained_segments() {
        let directory = writable_manifest_dir().join("target").join(format!(
            "d2b-audit-startup-retention-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        let record = sample(genesis_hash());
        {
            let mut writer = SegmentWriter::open_at(&directory, 1024, 1, 0).unwrap();
            writer.append_at(&record, 0).unwrap();
        }
        let sink = AuditSink::open_with_limits(&directory, 1024, 1, 8).unwrap();
        assert_eq!(sink.chain_head(), Ok(record.record_hash().clone()));
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn post_write_failure_rolls_back_chain_and_allows_retry() {
        for point in [FailurePoint::DataSync, FailurePoint::ParentSync] {
            let directory = writable_manifest_dir()
                .join("target")
                .join(format!("d2b-audit-retry-{point:?}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&directory);
            let injector = FailureInjector::default();
            injector.fail_next(point);
            let sink = AuditSink::open_with_injector(&directory, 1024, 30, 1, injector).unwrap();
            let record = sample(genesis_hash());
            assert_eq!(
                sink.append(AuditWriteClass::Privileged, &record)
                    .unwrap_err(),
                AuditSinkError::Unavailable
            );
            assert_eq!(
                sink.append(AuditWriteClass::Privileged, &record).unwrap(),
                AuditWriteOutcome::Written
            );
            let _ = std::fs::remove_dir_all(directory);
        }
    }

    #[test]
    fn durable_append_is_success_even_when_automatic_retention_degrades() {
        let directory = writable_manifest_dir().join("target").join(format!(
            "d2b-audit-retention-success-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        let injector = FailureInjector::default();
        let sink = AuditSink::open_with_injector(&directory, 1, 1, 8, injector.clone()).unwrap();
        let old = directory.join("audit-19700101000000000000.jsonl");
        std::fs::write(&old, b"").unwrap();
        let first = sample(genesis_hash());
        assert_eq!(
            sink.append(AuditWriteClass::Privileged, &first).unwrap(),
            AuditWriteOutcome::Written
        );
        let second = sample(first.record_hash().clone());
        injector.fail_next(FailurePoint::PruneCheckpoint);
        assert_eq!(
            sink.append(AuditWriteClass::Privileged, &second).unwrap(),
            AuditWriteOutcome::Written
        );
        assert_eq!(sink.chain_head(), Ok(second.record_hash().clone()));
        assert!(sink.retention_degraded().unwrap());
        assert!(old.exists());
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        assert_eq!(sink.prune_old(now_ms).unwrap(), 1);
        assert!(!sink.retention_degraded().unwrap());
        assert!(!old.exists());
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn restart_refuses_a_corrupt_hash_chain() {
        let directory = writable_manifest_dir()
            .join("target")
            .join(format!("d2b-audit-corrupt-chain-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        let sink = AuditSink::open(&directory).unwrap();
        let first = sample(genesis_hash());
        sink.append(AuditWriteClass::Privileged, &first).unwrap();
        let path = sink
            .chain_head()
            .and_then(|_| {
                std::fs::read_dir(&directory)
                    .map_err(|_| AuditSinkError::Unavailable)
                    .and_then(|entries| {
                        entries
                            .filter_map(Result::ok)
                            .map(|entry| entry.path())
                            .find(|path| {
                                path.file_name()
                                    .and_then(|name| name.to_str())
                                    .is_some_and(crate::export::is_segment_name)
                            })
                            .ok_or(AuditSinkError::Unavailable)
                    })
            })
            .unwrap();
        let mut bytes = std::fs::read(&path).unwrap();
        bytes.extend_from_slice(b"{\"corrupt\":true}\n");
        std::fs::write(&path, bytes).unwrap();
        drop(sink);
        assert_eq!(
            AuditSink::open(&directory).unwrap_err(),
            AuditSinkError::ChainMismatch
        );
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn replay_after_append_before_clear_is_idempotent() {
        let directory = writable_manifest_dir()
            .join("target")
            .join(format!("d2b-audit-idempotent-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        let sink = AuditSink::open(&directory).unwrap();
        let record = AuditRecord::new(
            1_700_000_000_000,
            "work",
            "operation",
            "correlation",
            None,
            "resource-store",
            genesis_hash(),
            AuditRecordFields::ResourceMutation(ResourceMutationFields {
                verb: "create".to_owned(),
                resource_type: "Host".to_owned(),
                resource_uid: "uid".to_owned(),
                generation: 1,
                expected_revision: 0,
                resulting_revision: 1,
                subject_digest:
                    "sha256:0000000000000000000000000000000000000000000000000000000000000002"
                        .to_owned(),
                policy_revision: 1,
                outcome: "ok".to_owned(),
                error_code: None,
                mutation_id: Some(
                    "sha256:0000000000000000000000000000000000000000000000000000000000000001"
                        .to_owned(),
                ),
                mutation_ordinal: Some(0),
            }),
        )
        .unwrap();
        sink.append(AuditWriteClass::Privileged, &record).unwrap();
        drop(sink);
        let restarted = AuditSink::open(&directory).unwrap();
        assert_eq!(
            restarted
                .append(AuditWriteClass::Privileged, &record)
                .unwrap(),
            AuditWriteOutcome::Written
        );
        let lines = std::fs::read_dir(&directory)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| std::fs::read_to_string(entry.path()).unwrap())
            .map(|text| text.lines().count())
            .sum::<usize>();
        assert_eq!(lines, 1);
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn sink_uses_one_lifetime_writer_lock_per_directory() {
        let directory = writable_manifest_dir()
            .join("target")
            .join(format!("d2b-audit-single-writer-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        let sink = AuditSink::open(&directory).unwrap();
        assert_eq!(
            AuditSink::open(&directory).unwrap_err(),
            AuditSinkError::Unavailable
        );
        drop(sink);
        assert!(AuditSink::open(&directory).is_ok());
        let _ = std::fs::remove_dir_all(directory);
    }
}
