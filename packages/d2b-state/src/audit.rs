use std::{fmt, io::Read, os::fd::OwnedFd};

use d2b_contracts::v2_identity::ProviderId;
use d2b_contracts::v2_state::{
    AuditActor, AuditCheckpoint, AuditCheckpointSignatureVerifier, AuditCorrelation, AuditEvent,
    AuditOutcome, AuditOwner, AuditReason, AuditRecord, AuditRetentionDecision,
    AuditRetentionEvidence, AuditRetentionPolicy, AuditSegment, AuditSegmentSummary, AuditStream,
    CorrelationId, Digest, Generation, MAX_AUDIT_RECORD_BYTES, MAX_AUDIT_RECORDS_PER_SEGMENT,
    MAX_AUDIT_SEGMENT_BYTES, MAX_SAFE_JSON_INTEGER, PruneStatus, ResourceId, STATE_SCHEMA_VERSION,
};
use rustix::fs::{FileType, Mode, OFlags};

use crate::{
    AnchoredResource, CanonicalJson, Error, ErrorCode, MetadataExpectation, RelativePath, Result,
};

fn zero_digest() -> Digest {
    Digest::parse("0".repeat(64)).expect("fixed zero digest is valid")
}

#[derive(Debug, Clone)]
pub struct AuditRecordInput {
    pub stream: AuditStream,
    pub sequence: u64,
    pub occurred_at_unix_ms: u64,
    pub operation_id: CorrelationId,
    pub session_id: Option<CorrelationId>,
    pub provider_id: Option<ProviderId>,
    pub actor: AuditActor,
    pub event: AuditEvent,
    pub outcome: AuditOutcome,
    pub reason: AuditReason,
}

impl AuditRecordInput {
    pub fn build(self, previous_hash: Digest) -> Result<AuditRecord> {
        let mut record = AuditRecord {
            schema_version: STATE_SCHEMA_VERSION,
            stream: self.stream,
            sequence: self.sequence,
            occurred_at_unix_ms: self.occurred_at_unix_ms,
            correlation: AuditCorrelation {
                operation_id: self.operation_id,
                session_id: self.session_id,
                provider_id: self.provider_id,
            },
            actor: self.actor,
            event: self.event,
            outcome: self.outcome,
            reason: self.reason,
            previous_hash,
            record_hash: zero_digest(),
            encoded_bytes: 1,
        };
        for _ in 0..4 {
            record.record_hash = record.computed_hash();
            let encoded = CanonicalJson::encode(&record)?;
            let encoded_bytes =
                u32::try_from(encoded.len()).map_err(|_| Error::Code(ErrorCode::TooLarge))?;
            if encoded_bytes > MAX_AUDIT_RECORD_BYTES {
                return Err(Error::Code(ErrorCode::TooLarge));
            }
            if record.encoded_bytes == encoded_bytes {
                record.validate_integrity()?;
                return Ok(record);
            }
            record.encoded_bytes = encoded_bytes;
        }
        Err(Error::Code(ErrorCode::AuditInvalid))
    }
}

#[derive(Debug, Clone)]
pub struct SegmentBuilder {
    stream: AuditStream,
    owner: AuditOwner,
    segment_id: ResourceId,
    previous_segment_digest: Digest,
    controller_generation: Generation,
    created_at_unix_ms: u64,
    next_sequence: u64,
    records: Vec<AuditRecord>,
    encoded_bytes: u64,
}

impl SegmentBuilder {
    pub fn new(
        stream: AuditStream,
        owner: AuditOwner,
        segment_id: ResourceId,
        previous_segment_digest: Digest,
        controller_generation: Generation,
        created_at_unix_ms: u64,
        first_sequence: u64,
    ) -> Result<Self> {
        let zero_previous = previous_segment_digest
            .as_str()
            .bytes()
            .all(|byte| byte == b'0');
        if first_sequence == 0 || (first_sequence == 1) != zero_previous {
            return Err(Error::Code(ErrorCode::AuditInvalid));
        }
        Ok(Self {
            stream,
            owner,
            segment_id,
            previous_segment_digest,
            controller_generation,
            created_at_unix_ms,
            next_sequence: first_sequence,
            records: Vec::new(),
            encoded_bytes: 0,
        })
    }

    pub fn push(&mut self, record: AuditRecord) -> Result<()> {
        if self.records.len() >= MAX_AUDIT_RECORDS_PER_SEGMENT
            || record.stream != self.stream
            || record.sequence != self.next_sequence
            || record.previous_hash
                != self.records.last().map_or_else(
                    || self.previous_segment_digest.clone(),
                    |previous| previous.record_hash.clone(),
                )
        {
            return Err(Error::Code(ErrorCode::AuditInvalid));
        }
        record.validate_integrity()?;
        let line_bytes = u64::from(record.encoded_bytes)
            .checked_add(1)
            .ok_or(Error::Code(ErrorCode::TooLarge))?;
        let encoded_bytes = self
            .encoded_bytes
            .checked_add(line_bytes)
            .ok_or(Error::Code(ErrorCode::TooLarge))?;
        if encoded_bytes > MAX_AUDIT_SEGMENT_BYTES {
            return Err(Error::Code(ErrorCode::TooLarge));
        }
        self.encoded_bytes = encoded_bytes;
        self.records.push(record);
        self.next_sequence = self
            .next_sequence
            .checked_add(1)
            .ok_or(Error::Code(ErrorCode::TooLarge))?;
        Ok(())
    }

    pub fn seal(self, sealed_at_unix_ms: u64, prune_status: PruneStatus) -> Result<AuditSegment> {
        let first = self
            .records
            .first()
            .ok_or(Error::Code(ErrorCode::AuditInvalid))?
            .sequence;
        let last = self
            .records
            .last()
            .ok_or(Error::Code(ErrorCode::AuditInvalid))?
            .sequence;
        let mut segment = AuditSegment {
            summary: AuditSegmentSummary {
                stream: self.stream,
                owner: self.owner,
                segment_id: self.segment_id,
                first_sequence: first,
                last_sequence: last,
                previous_segment_digest: self.previous_segment_digest,
                segment_digest: zero_digest(),
                controller_generation: self.controller_generation,
                created_at_unix_ms: self.created_at_unix_ms,
                sealed_at_unix_ms,
                encoded_bytes: self.encoded_bytes,
                prune_status,
            },
            records: self.records,
        };
        segment.summary.segment_digest = segment.computed_digest();
        verify_segment_internal(&segment)?;
        Ok(segment)
    }
}

pub fn read_audit_segment(
    summary: AuditSegmentSummary,
    bytes: &[u8],
    previous: Option<&AuditSegment>,
) -> Result<AuditSegment> {
    if bytes.is_empty()
        || bytes.len() as u64 > MAX_AUDIT_SEGMENT_BYTES
        || bytes.last() != Some(&b'\n')
        || summary.encoded_bytes != bytes.len() as u64
    {
        return Err(Error::Code(ErrorCode::AuditInvalid));
    }
    let mut records = Vec::new();
    for line in bytes[..bytes.len() - 1].split(|byte| *byte == b'\n') {
        if line.is_empty() || records.len() >= MAX_AUDIT_RECORDS_PER_SEGMENT {
            return Err(Error::Code(ErrorCode::AuditInvalid));
        }
        let record: AuditRecord = CanonicalJson::decode(line)?;
        if record.encoded_bytes as usize != line.len() {
            return Err(Error::Code(ErrorCode::AuditInvalid));
        }
        records.push(record);
    }
    let segment = AuditSegment { summary, records };
    match previous {
        Some(previous) => segment.validate_after(previous)?,
        None => segment.validate()?,
    }
    Ok(segment)
}

pub fn checkpoint(
    segment: &AuditSegment,
    checkpoint_id: ResourceId,
    previous: Option<&AuditCheckpoint>,
    created_at_unix_ms: u64,
    realm_signature_digest: Option<Digest>,
) -> Result<AuditCheckpoint> {
    verify_segment_internal(segment)?;
    if created_at_unix_ms < segment.summary.sealed_at_unix_ms
        || created_at_unix_ms > MAX_SAFE_JSON_INTEGER
        || matches!(
            (&segment.summary.stream, &realm_signature_digest),
            (AuditStream::LocalRoot, Some(_)) | (AuditStream::Realm { .. }, None)
        )
    {
        return Err(Error::Code(ErrorCode::AuditInvalid));
    }
    match previous {
        Some(previous)
            if previous.stream == segment.summary.stream
                && previous.owner == segment.summary.owner
                && previous.checkpoint_digest == previous.computed_digest()
                && previous.segment_digest == segment.summary.previous_segment_digest
                && previous
                    .through_sequence
                    .checked_add(1)
                    .is_some_and(|next| next == segment.summary.first_sequence) => {}
        Some(_) => return Err(Error::Code(ErrorCode::AuditInvalid)),
        None if segment.summary.first_sequence == 1
            && segment
                .summary
                .previous_segment_digest
                .as_str()
                .bytes()
                .all(|byte| byte == b'0') => {}
        None => return Err(Error::Code(ErrorCode::AuditInvalid)),
    }
    let mut checkpoint = AuditCheckpoint {
        stream: segment.summary.stream.clone(),
        owner: segment.summary.owner.clone(),
        checkpoint_id,
        through_sequence: segment.summary.last_sequence,
        segment_digest: segment.summary.segment_digest.clone(),
        previous_checkpoint_digest: previous.map_or_else(zero_digest, |checkpoint| {
            checkpoint.checkpoint_digest.clone()
        }),
        checkpoint_digest: zero_digest(),
        controller_generation: segment.summary.controller_generation,
        created_at_unix_ms,
        realm_signature_digest,
    };
    checkpoint.checkpoint_digest = checkpoint.computed_digest();
    Ok(checkpoint)
}

pub fn decide_retention<V: AuditCheckpointSignatureVerifier>(
    policy: &AuditRetentionPolicy,
    evidence: AuditRetentionEvidence<'_, V>,
) -> Result<AuditRetentionDecision> {
    Ok(policy.decide(evidence)?)
}

pub fn detect_gap(
    stream: AuditStream,
    expected_sequence: u64,
    observed_sequence: u64,
    detected_at_unix_ms: u64,
) -> Result<Option<d2b_contracts::v2_state::AuditGap>> {
    Ok(d2b_contracts::v2_state::detect_audit_gap(
        stream,
        expected_sequence,
        observed_sequence,
        detected_at_unix_ms,
    )?)
}

pub struct AuditAppender {
    resource: AnchoredResource,
    fd: OwnedFd,
    stream: AuditStream,
    next_sequence: u64,
    previous_hash: Digest,
    encoded_bytes: u64,
    record_count: usize,
    poisoned: bool,
}

impl fmt::Debug for AuditAppender {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AuditAppender")
            .field("resource_id", &self.resource.resource_id)
            .field("next_sequence", &self.next_sequence)
            .field("record_count", &self.record_count)
            .finish_non_exhaustive()
    }
}

impl AuditAppender {
    pub fn create(
        resource: AnchoredResource,
        metadata: MetadataExpectation,
        stream: AuditStream,
        previous_segment_digest: Digest,
        first_sequence: u64,
    ) -> Result<Self> {
        metadata.validate()?;
        if first_sequence == 0 {
            return Err(Error::Code(ErrorCode::AuditInvalid));
        }
        let zero_previous = previous_segment_digest
            .as_str()
            .bytes()
            .all(|byte| byte == b'0');
        if (first_sequence == 1) != zero_previous {
            return Err(Error::Code(ErrorCode::AuditInvalid));
        }
        let path = RelativePath::from_components([resource.leaf.as_str()])?;
        let fd = resource.directory.open_beneath(
            &path,
            OFlags::WRONLY | OFlags::APPEND | OFlags::CREATE | OFlags::EXCL,
            Mode::from_raw_mode(metadata.mode),
        )?;
        rustix::fs::fchmod(&fd, Mode::from_raw_mode(metadata.mode))
            .map_err(|error| Error::io(ErrorCode::Io, error))?;
        validate_metadata(&fd, metadata)?;
        Ok(Self {
            resource,
            fd,
            stream,
            next_sequence: first_sequence,
            previous_hash: previous_segment_digest,
            encoded_bytes: 0,
            record_count: 0,
            poisoned: false,
        })
    }

    pub fn resume(
        resource: AnchoredResource,
        metadata: MetadataExpectation,
        stream: AuditStream,
        previous_segment_digest: Digest,
        first_sequence: u64,
    ) -> Result<Self> {
        metadata.validate()?;
        let path = RelativePath::from_components([resource.leaf.as_str()])?;
        let fd =
            resource
                .directory
                .open_beneath(&path, OFlags::RDWR | OFlags::APPEND, Mode::empty())?;
        validate_metadata(&fd, metadata)?;
        let stat = rustix::fs::fstat(&fd).map_err(|error| Error::io(ErrorCode::Io, error))?;
        if stat.st_size <= 0 || stat.st_size as u64 > MAX_AUDIT_SEGMENT_BYTES {
            return Err(Error::Code(ErrorCode::AuditInvalid));
        }
        let mut bytes = Vec::with_capacity(stat.st_size as usize);
        let mut file = std::fs::File::from(fd);
        file.by_ref()
            .take(MAX_AUDIT_SEGMENT_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| Error::std_io(ErrorCode::Io, &error))?;
        if bytes.len() as u64 != stat.st_size as u64 || bytes.last() != Some(&b'\n') {
            return Err(Error::Code(ErrorCode::AuditInvalid));
        }
        let records =
            parse_unsealed_records(&stream, &previous_segment_digest, first_sequence, &bytes)?;
        let last = records
            .last()
            .expect("resume parser rejects empty segments");
        let next_sequence = last
            .sequence
            .checked_add(1)
            .ok_or(Error::Code(ErrorCode::TooLarge))?;
        let previous_hash = last.record_hash.clone();
        let record_count = records.len();
        let fd = OwnedFd::from(file);
        Ok(Self {
            resource,
            fd,
            stream,
            next_sequence,
            previous_hash,
            encoded_bytes: bytes.len() as u64,
            record_count,
            poisoned: false,
        })
    }

    pub fn append(&mut self, input: AuditRecordInput) -> Result<AuditRecord> {
        if self.poisoned || input.stream != self.stream || input.sequence != self.next_sequence {
            return Err(Error::Code(ErrorCode::AuditInvalid));
        }
        let record = input.build(self.previous_hash.clone())?;
        if self.record_count >= MAX_AUDIT_RECORDS_PER_SEGMENT {
            return Err(Error::Code(ErrorCode::TooLarge));
        }
        let mut line = CanonicalJson::encode(&record)?;
        if line.len() != record.encoded_bytes as usize {
            return Err(Error::Code(ErrorCode::AuditInvalid));
        }
        line.push(b'\n');
        let next_bytes = self
            .encoded_bytes
            .checked_add(line.len() as u64)
            .ok_or(Error::Code(ErrorCode::TooLarge))?;
        if next_bytes > MAX_AUDIT_SEGMENT_BYTES {
            return Err(Error::Code(ErrorCode::TooLarge));
        }
        let mut written = 0;
        while written < line.len() {
            let count = match rustix::io::write(&self.fd, &line[written..]) {
                Ok(count) => count,
                Err(error) => {
                    self.poisoned = true;
                    return Err(Error::io(ErrorCode::Io, error));
                }
            };
            if count == 0 {
                self.poisoned = true;
                return Err(Error::Code(ErrorCode::Io));
            }
            written += count;
        }
        if let Err(error) = rustix::fs::fsync(&self.fd) {
            self.poisoned = true;
            return Err(Error::io(ErrorCode::Io, error));
        }
        self.previous_hash = record.record_hash.clone();
        self.next_sequence = self
            .next_sequence
            .checked_add(1)
            .ok_or(Error::Code(ErrorCode::TooLarge))?;
        self.encoded_bytes = next_bytes;
        self.record_count += 1;
        Ok(record)
    }

    pub fn encoded_bytes(&self) -> u64 {
        self.encoded_bytes
    }

    pub fn record_count(&self) -> usize {
        self.record_count
    }
}

fn validate_metadata(fd: &OwnedFd, expected: MetadataExpectation) -> Result<()> {
    let stat = rustix::fs::fstat(fd).map_err(|error| Error::io(ErrorCode::Io, error))?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile
        || stat.st_uid != expected.uid
        || stat.st_gid != expected.gid
        || stat.st_mode & 0o7777 != expected.mode
        || !rustix::fs::fcntl_getfl(fd)
            .map_err(|error| Error::io(ErrorCode::Io, error))?
            .contains(OFlags::APPEND)
    {
        return Err(Error::Code(ErrorCode::MetadataMismatch));
    }
    Ok(())
}

fn verify_segment_internal(segment: &AuditSegment) -> Result<()> {
    if segment.records.is_empty()
        || segment.records.len() > MAX_AUDIT_RECORDS_PER_SEGMENT
        || !valid_owner(&segment.summary.stream, &segment.summary.owner)
        || segment.summary.first_sequence == 0
        || segment.summary.last_sequence > MAX_SAFE_JSON_INTEGER
        || segment.summary.created_at_unix_ms > segment.summary.sealed_at_unix_ms
        || segment.summary.sealed_at_unix_ms > MAX_SAFE_JSON_INTEGER
        || segment.summary.encoded_bytes == 0
        || segment.summary.encoded_bytes > MAX_AUDIT_SEGMENT_BYTES
        || segment.summary.first_sequence
            != segment
                .records
                .first()
                .expect("non-empty checked above")
                .sequence
        || segment.summary.last_sequence
            != segment
                .records
                .last()
                .expect("non-empty checked above")
                .sequence
        || segment.summary.segment_digest != segment.computed_digest()
    {
        return Err(Error::Code(ErrorCode::AuditInvalid));
    }
    let mut sequence = segment.summary.first_sequence;
    let mut previous = &segment.summary.previous_segment_digest;
    let mut encoded_bytes = 0_u64;
    for record in &segment.records {
        record.validate_integrity()?;
        let canonical = CanonicalJson::encode(record)?;
        if record.stream != segment.summary.stream
            || record.sequence != sequence
            || &record.previous_hash != previous
            || canonical.len() != record.encoded_bytes as usize
        {
            return Err(Error::Code(ErrorCode::AuditInvalid));
        }
        encoded_bytes = encoded_bytes
            .checked_add(canonical.len() as u64 + 1)
            .ok_or(Error::Code(ErrorCode::AuditInvalid))?;
        sequence = sequence
            .checked_add(1)
            .ok_or(Error::Code(ErrorCode::AuditInvalid))?;
        previous = &record.record_hash;
    }
    if encoded_bytes != segment.summary.encoded_bytes {
        return Err(Error::Code(ErrorCode::AuditInvalid));
    }
    Ok(())
}

fn parse_unsealed_records(
    stream: &AuditStream,
    previous_segment_digest: &Digest,
    first_sequence: u64,
    bytes: &[u8],
) -> Result<Vec<AuditRecord>> {
    if first_sequence == 0 || bytes.is_empty() || bytes.last() != Some(&b'\n') {
        return Err(Error::Code(ErrorCode::AuditInvalid));
    }
    let zero_previous = previous_segment_digest
        .as_str()
        .bytes()
        .all(|byte| byte == b'0');
    if (first_sequence == 1) != zero_previous {
        return Err(Error::Code(ErrorCode::AuditInvalid));
    }
    let mut records = Vec::new();
    let mut next_sequence = first_sequence;
    let mut previous_hash = previous_segment_digest.clone();
    for line in bytes[..bytes.len() - 1].split(|byte| *byte == b'\n') {
        if line.is_empty() || records.len() >= MAX_AUDIT_RECORDS_PER_SEGMENT {
            return Err(Error::Code(ErrorCode::AuditInvalid));
        }
        let record: AuditRecord = CanonicalJson::decode(line)?;
        record.validate_integrity()?;
        if &record.stream != stream
            || record.sequence != next_sequence
            || record.previous_hash != previous_hash
            || record.encoded_bytes as usize != line.len()
        {
            return Err(Error::Code(ErrorCode::AuditInvalid));
        }
        next_sequence = next_sequence
            .checked_add(1)
            .ok_or(Error::Code(ErrorCode::AuditInvalid))?;
        previous_hash = record.record_hash.clone();
        records.push(record);
    }
    if records.is_empty() {
        return Err(Error::Code(ErrorCode::AuditInvalid));
    }
    Ok(records)
}

fn valid_owner(stream: &AuditStream, owner: &AuditOwner) -> bool {
    match (stream, owner) {
        (AuditStream::LocalRoot, AuditOwner::LocalRootBroker) => true,
        (AuditStream::Realm { realm_id: stream }, AuditOwner::RealmBroker { realm_id: owner }) => {
            stream == owner
        }
        _ => false,
    }
}
