use std::{
    fmt,
    io::Read,
    os::fd::{AsFd, OwnedFd},
    sync::atomic::{AtomicU64, Ordering},
};

use d2b_contracts::v2_state::{
    AtomicWritePhase, AtomicWriteReceipt, AuthorityRef, CanonicalPayloadVerifier, Digest,
    Generation, MAX_JSON_DOCUMENT_BYTES, OwnershipEpoch, QuarantineReason, Remediation,
    STATE_SCHEMA_GENERATION, STATE_SCHEMA_VERSION, StateContractError, StateEnvelope,
    state_payload_digest,
};
use rustix::fs::{AtFlags, FileType, Mode, OFlags};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::value::RawValue;
use sha2::{Digest as _, Sha256};

use crate::{AnchoredResource, Error, ErrorCode, LeafName, LockGuard, Result};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MetadataExpectation {
    pub uid: u32,
    pub gid: u32,
    pub mode: u32,
}

impl MetadataExpectation {
    pub fn validate(self) -> Result<()> {
        if self.mode & !0o7777 != 0 {
            return Err(Error::Code(ErrorCode::MetadataMismatch));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GenerationPolicy {
    Exact(Generation),
    AtLeast(Generation),
}

impl GenerationPolicy {
    fn accepts(self, observed: Generation) -> bool {
        match self {
            Self::Exact(expected) => observed == expected,
            Self::AtLeast(minimum) => observed >= minimum,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadPolicy {
    pub metadata: MetadataExpectation,
    pub writer: AuthorityRef,
    pub config_generation: Generation,
    pub state_generation: GenerationPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WritePolicy {
    pub metadata: MetadataExpectation,
    pub writer: AuthorityRef,
    pub config_generation: Generation,
    pub state_generation: Generation,
    pub expected_previous: Option<Generation>,
    pub lock_id: d2b_contracts::v2_state::ResourceId,
    pub ownership_epoch: OwnershipEpoch,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DurableState<T> {
    pub config_generation: Generation,
    pub state_generation: Generation,
    pub writer: AuthorityRef,
    pub checksum: Digest,
    pub payload: T,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuarantineRecord {
    pub resource_id: d2b_contracts::v2_state::ResourceId,
    pub lock_id: d2b_contracts::v2_state::ResourceId,
    pub writer: AuthorityRef,
    pub ownership_epoch: OwnershipEpoch,
    pub reason: QuarantineReason,
    pub remediation: Remediation,
    pub observed_document_digest: Option<Digest>,
}

impl QuarantineRecord {
    pub fn for_error(
        resource_id: d2b_contracts::v2_state::ResourceId,
        lock_id: d2b_contracts::v2_state::ResourceId,
        writer: AuthorityRef,
        ownership_epoch: OwnershipEpoch,
        error: &Error,
        bytes: Option<&[u8]>,
    ) -> Self {
        let (reason, remediation) = match error {
            Error::Quarantine {
                reason,
                remediation,
            } => (*reason, *remediation),
            _ => (
                match error.code() {
                    ErrorCode::GenerationRollback | ErrorCode::GenerationGap => {
                        QuarantineReason::GenerationMismatch
                    }
                    ErrorCode::MetadataMismatch | ErrorCode::PathRejected => {
                        QuarantineReason::OwnerAmbiguous
                    }
                    _ => QuarantineReason::CorruptState,
                },
                Remediation::InspectQuarantine,
            ),
        };
        Self {
            resource_id,
            lock_id,
            writer,
            ownership_epoch,
            reason,
            remediation,
            observed_document_digest: bytes.map(document_digest),
        }
    }
}

pub struct CanonicalJson;

impl CanonicalJson {
    pub fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>> {
        let encoded =
            serde_json::to_vec(value).map_err(|_| Error::Code(ErrorCode::NonCanonical))?;
        validate_document_bound(&encoded)?;
        Ok(encoded)
    }

    pub fn decode<T: DeserializeOwned + Serialize>(bytes: &[u8]) -> Result<T> {
        validate_document_bound(bytes)?;
        let value: T =
            serde_json::from_slice(bytes).map_err(|_| Error::Code(ErrorCode::NonCanonical))?;
        let canonical =
            serde_json::to_vec(&value).map_err(|_| Error::Code(ErrorCode::NonCanonical))?;
        if canonical != bytes {
            return Err(Error::Code(ErrorCode::NonCanonical));
        }
        Ok(value)
    }
}

impl<T> CanonicalPayloadVerifier<T> for CanonicalJson
where
    T: DeserializeOwned + Serialize,
{
    fn decode_canonical(&self, raw_payload: &[u8]) -> std::result::Result<T, StateContractError> {
        let value: T = serde_json::from_slice(raw_payload)
            .map_err(|_| StateContractError::EnvelopePayloadMismatch)?;
        let canonical =
            serde_json::to_vec(&value).map_err(|_| StateContractError::EnvelopePayloadMismatch)?;
        if canonical != raw_payload {
            return Err(StateContractError::EnvelopePayloadMismatch);
        }
        Ok(value)
    }
}

fn validate_document_bound(bytes: &[u8]) -> Result<()> {
    if bytes.is_empty() {
        return Err(Error::Code(ErrorCode::Empty));
    }
    if bytes.len() as u64 > MAX_JSON_DOCUMENT_BYTES {
        return Err(Error::Code(ErrorCode::TooLarge));
    }
    Ok(())
}

fn document_digest(bytes: &[u8]) -> Digest {
    let mut hasher = Sha256::new();
    hasher.update(b"d2b.v2.state.document.sha256\0");
    hasher.update((bytes.len() as u64).to_be_bytes());
    hasher.update(bytes);
    let encoded = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Digest::parse(encoded).expect("SHA-256 is a valid contract digest")
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawEnvelope {
    schema_version: u32,
    schema_generation: u32,
    config_generation: Generation,
    state_generation: Generation,
    writer: AuthorityRef,
    encoded_bytes: u64,
    checksum: Digest,
    payload: Box<RawValue>,
}

impl RawEnvelope {
    fn decode<T>(self, policy: &ReadPolicy) -> Result<DurableState<T>>
    where
        T: DeserializeOwned + Serialize + PartialEq,
    {
        if self.writer != policy.writer {
            return Err(Error::Code(ErrorCode::InvalidWriter));
        }
        if self.config_generation != policy.config_generation {
            return Err(Error::Code(ErrorCode::GenerationRollback));
        }
        if !policy.state_generation.accepts(self.state_generation) {
            return Err(Error::Code(ErrorCode::GenerationRollback));
        }
        let payload_bytes = self.payload.get().as_bytes();
        let payload = CanonicalJson::decode(payload_bytes)?;
        let envelope = StateEnvelope {
            schema_version: self.schema_version,
            schema_generation: self.schema_generation,
            config_generation: self.config_generation,
            state_generation: self.state_generation,
            writer: self.writer.clone(),
            encoded_bytes: self.encoded_bytes,
            checksum: self.checksum.clone(),
            payload,
        };
        envelope.validate_payload_bytes(payload_bytes, &CanonicalJson)?;
        Ok(DurableState {
            config_generation: envelope.config_generation,
            state_generation: envelope.state_generation,
            writer: envelope.writer,
            checksum: envelope.checksum,
            payload: envelope.payload,
        })
    }
}

pub trait AtomicFilesystem {
    type Temp;

    fn resource_id(&self) -> &d2b_contracts::v2_state::ResourceId;
    fn read_target(&mut self, maximum: u64) -> Result<(Vec<u8>, MetadataExpectation)>;
    fn inspect_target_metadata(&mut self) -> Result<MetadataExpectation>;
    fn create_temp(&mut self, metadata: MetadataExpectation) -> Result<Self::Temp>;
    fn write_temp(&mut self, temp: &mut Self::Temp, bytes: &[u8]) -> Result<usize>;
    fn sync_temp(&mut self, temp: &mut Self::Temp) -> Result<()>;
    fn rename_temp(&mut self, temp: &mut Self::Temp) -> Result<()>;
    fn sync_parent(&mut self) -> Result<()>;
    fn remove_temp(&mut self, temp: &mut Self::Temp);
    fn quarantine_target(&mut self, quarantine_name: &LeafName) -> Result<()>;
}

pub struct AtomicWrite<F> {
    filesystem: F,
}

impl<F: AtomicFilesystem> AtomicWrite<F> {
    pub fn new(filesystem: F) -> Self {
        Self { filesystem }
    }

    pub fn read<T>(&mut self, policy: &ReadPolicy) -> Result<DurableState<T>>
    where
        T: DeserializeOwned + Serialize + PartialEq,
    {
        policy.metadata.validate()?;
        let (bytes, observed) = self.filesystem.read_target(MAX_JSON_DOCUMENT_BYTES + 1)?;
        if observed != policy.metadata {
            return Err(Error::Code(ErrorCode::MetadataMismatch));
        }
        validate_document_bound(&bytes)?;
        let raw: RawEnvelope =
            serde_json::from_slice(&bytes).map_err(|_| Error::Code(ErrorCode::NonCanonical))?;
        let canonical =
            serde_json::to_vec(&raw).map_err(|_| Error::Code(ErrorCode::NonCanonical))?;
        if canonical != bytes {
            return Err(Error::Code(ErrorCode::NonCanonical));
        }
        if raw.schema_version != STATE_SCHEMA_VERSION
            || raw.schema_generation != STATE_SCHEMA_GENERATION
        {
            return Err(Error::Code(ErrorCode::InvalidSchema));
        }
        raw.decode(policy)
    }

    pub fn write<T>(
        &mut self,
        payload: &T,
        policy: &WritePolicy,
        guard: Option<&LockGuard>,
    ) -> Result<AtomicWriteReceipt>
    where
        T: DeserializeOwned + Serialize + PartialEq,
    {
        self.validate_lock(
            guard,
            &policy.lock_id,
            &policy.writer,
            policy.ownership_epoch,
        )?;
        policy.metadata.validate()?;
        self.validate_generation_before_write::<T>(policy)?;
        let payload_bytes = CanonicalJson::encode(payload)?;
        let checksum = state_payload_digest(&payload_bytes)?;
        let raw_payload = RawValue::from_string(
            String::from_utf8(payload_bytes).map_err(|_| Error::Code(ErrorCode::NonCanonical))?,
        )
        .map_err(|_| Error::Code(ErrorCode::NonCanonical))?;
        let envelope = StateEnvelope {
            schema_version: STATE_SCHEMA_VERSION,
            schema_generation: STATE_SCHEMA_GENERATION,
            config_generation: policy.config_generation,
            state_generation: policy.state_generation,
            writer: policy.writer.clone(),
            encoded_bytes: raw_payload.get().len() as u64,
            checksum: checksum.clone(),
            payload: raw_payload.as_ref(),
        };
        let document = CanonicalJson::encode(&envelope)?;

        let mut phase = AtomicWritePhase::Initial;
        let mut temp = self.filesystem.create_temp(policy.metadata)?;
        phase = phase.transition(AtomicWritePhase::TemporaryCreated)?;
        let result = (|| {
            let mut written = 0;
            while written < document.len() {
                let count = self
                    .filesystem
                    .write_temp(&mut temp, &document[written..])?;
                if count == 0 {
                    return Err(Error::Code(ErrorCode::Io));
                }
                written = written
                    .checked_add(count)
                    .ok_or(Error::Code(ErrorCode::TooLarge))?;
            }
            phase = phase.transition(AtomicWritePhase::CompleteDocumentWritten)?;
            self.filesystem.sync_temp(&mut temp)?;
            phase = phase.transition(AtomicWritePhase::TemporaryFileSynced)?;
            self.filesystem.rename_temp(&mut temp)?;
            phase = phase.transition(AtomicWritePhase::Renamed)?;
            self.filesystem.sync_parent()?;
            phase = phase.transition(AtomicWritePhase::ParentDirectorySynced)?;
            Ok(())
        })();
        if result.is_err() {
            self.filesystem.remove_temp(&mut temp);
            if phase == AtomicWritePhase::Renamed {
                return Err(Error::Quarantine {
                    reason: QuarantineReason::OwnerAmbiguous,
                    remediation: Remediation::InspectQuarantine,
                });
            }
            return result.map(|()| unreachable!());
        }
        let receipt = AtomicWriteReceipt {
            resource_id: self.filesystem.resource_id().clone(),
            generation: policy.state_generation,
            phase,
            checksum,
            success: true,
        };
        receipt.validate()?;
        Ok(receipt)
    }

    fn validate_generation_before_write<T>(&mut self, policy: &WritePolicy) -> Result<()>
    where
        T: DeserializeOwned + Serialize + PartialEq,
    {
        match policy.expected_previous {
            Some(previous) => {
                let expected_next = Generation::new(
                    previous
                        .get()
                        .checked_add(1)
                        .ok_or(Error::Code(ErrorCode::GenerationGap))?,
                )?;
                if policy.state_generation != expected_next {
                    return Err(Error::Code(ErrorCode::GenerationGap));
                }
                self.read::<T>(&ReadPolicy {
                    metadata: policy.metadata,
                    writer: policy.writer.clone(),
                    config_generation: policy.config_generation,
                    state_generation: GenerationPolicy::Exact(previous),
                })?;
            }
            None => {
                if policy.state_generation.get() != 1 {
                    return Err(Error::Code(ErrorCode::GenerationGap));
                }
                match self.filesystem.inspect_target_metadata() {
                    Err(error) if error.code() == ErrorCode::Missing => {}
                    Err(error) => return Err(error),
                    Ok(_) => return Err(Error::Code(ErrorCode::AlreadyExists)),
                }
            }
        }
        Ok(())
    }

    pub fn quarantine(
        &mut self,
        record: &QuarantineRecord,
        guard: Option<&LockGuard>,
    ) -> Result<()> {
        if record.resource_id != *self.filesystem.resource_id() {
            return Err(Error::Code(ErrorCode::LockMismatch));
        }
        self.validate_lock(
            guard,
            &record.lock_id,
            &record.writer,
            record.ownership_epoch,
        )?;
        let name = LeafName::parse(format!(
            "quarantine-{}-{}",
            record.resource_id.as_str(),
            TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ))?;
        self.filesystem.quarantine_target(&name)?;
        self.filesystem.sync_parent()
    }

    pub fn into_inner(self) -> F {
        self.filesystem
    }

    fn validate_lock(
        &self,
        guard: Option<&LockGuard>,
        lock_id: &d2b_contracts::v2_state::ResourceId,
        writer: &AuthorityRef,
        ownership_epoch: OwnershipEpoch,
    ) -> Result<()> {
        guard
            .ok_or(Error::Code(ErrorCode::LockRequired))?
            .validate_state_binding(
                lock_id,
                self.filesystem.resource_id(),
                writer,
                ownership_epoch,
            )
    }
}

pub struct RealTemp {
    fd: OwnedFd,
    name: LeafName,
    renamed: bool,
}

impl fmt::Debug for RealTemp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RealTemp")
    }
}

pub struct RealAtomicFilesystem<'a> {
    resource: AnchoredResource<'a>,
    quarantine_directory: Option<&'a crate::AnchoredDir>,
}

impl fmt::Debug for RealAtomicFilesystem<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RealAtomicFilesystem")
            .field("resource_id", &self.resource.resource_id)
            .finish_non_exhaustive()
    }
}

impl<'a> RealAtomicFilesystem<'a> {
    pub fn new(resource: AnchoredResource<'a>) -> Self {
        Self {
            resource,
            quarantine_directory: None,
        }
    }

    pub fn with_quarantine(
        resource: AnchoredResource<'a>,
        quarantine_directory: &'a crate::AnchoredDir,
    ) -> Self {
        Self {
            resource,
            quarantine_directory: Some(quarantine_directory),
        }
    }

    fn open_target(&self) -> Result<OwnedFd> {
        let path = crate::RelativePath::from_components([self.resource.leaf.as_str()])?;
        self.resource.directory.open_beneath(
            &path,
            OFlags::RDONLY | OFlags::NONBLOCK,
            Mode::empty(),
        )
    }

    fn metadata_for_fd(fd: impl AsFd) -> Result<MetadataExpectation> {
        let stat = rustix::fs::fstat(fd).map_err(|error| Error::io(ErrorCode::Io, error))?;
        if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile {
            return Err(Error::Code(ErrorCode::NotRegularFile));
        }
        Ok(MetadataExpectation {
            uid: stat.st_uid,
            gid: stat.st_gid,
            mode: stat.st_mode & 0o7777,
        })
    }
}

impl AtomicFilesystem for RealAtomicFilesystem<'_> {
    type Temp = RealTemp;

    fn resource_id(&self) -> &d2b_contracts::v2_state::ResourceId {
        &self.resource.resource_id
    }

    fn read_target(&mut self, maximum: u64) -> Result<(Vec<u8>, MetadataExpectation)> {
        let fd = self.open_target()?;
        let metadata = Self::metadata_for_fd(&fd)?;
        let capacity = rustix::fs::fstat(&fd)
            .map(|stat| stat.st_size.max(0) as usize)
            .unwrap_or(0)
            .min(maximum as usize);
        let mut bytes = Vec::with_capacity(capacity);
        let mut file = std::fs::File::from(fd);
        file.by_ref()
            .take(maximum)
            .read_to_end(&mut bytes)
            .map_err(|error| Error::std_io(ErrorCode::Io, &error))?;
        if bytes.len() as u64 >= maximum {
            return Err(Error::Code(ErrorCode::TooLarge));
        }
        Ok((bytes, metadata))
    }

    fn inspect_target_metadata(&mut self) -> Result<MetadataExpectation> {
        Self::metadata_for_fd(self.open_target()?)
    }

    fn create_temp(&mut self, metadata: MetadataExpectation) -> Result<Self::Temp> {
        for _ in 0..128 {
            let name = LeafName::parse(format!(
                ".d2b-state-{}-{}",
                std::process::id(),
                TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
            ))?;
            let fd = match rustix::fs::openat(
                self.resource.directory.fd(),
                name.as_str(),
                OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::CLOEXEC | OFlags::NOFOLLOW,
                Mode::from_raw_mode(metadata.mode),
            ) {
                Ok(fd) => fd,
                Err(rustix::io::Errno::EXIST) => continue,
                Err(error) => return Err(Error::io(ErrorCode::Io, error)),
            };
            if let Err(error) = rustix::fs::fchmod(&fd, Mode::from_raw_mode(metadata.mode)) {
                let _ = rustix::fs::unlinkat(
                    self.resource.directory.fd(),
                    name.as_str(),
                    AtFlags::empty(),
                );
                return Err(Error::io(ErrorCode::Io, error));
            }
            match Self::metadata_for_fd(&fd) {
                Ok(observed) if observed == metadata => {}
                Ok(_) => {
                    let _ = rustix::fs::unlinkat(
                        self.resource.directory.fd(),
                        name.as_str(),
                        AtFlags::empty(),
                    );
                    return Err(Error::Code(ErrorCode::MetadataMismatch));
                }
                Err(error) => {
                    let _ = rustix::fs::unlinkat(
                        self.resource.directory.fd(),
                        name.as_str(),
                        AtFlags::empty(),
                    );
                    return Err(error);
                }
            }
            return Ok(RealTemp {
                fd,
                name,
                renamed: false,
            });
        }
        Err(Error::Code(ErrorCode::AlreadyExists))
    }

    fn write_temp(&mut self, temp: &mut Self::Temp, bytes: &[u8]) -> Result<usize> {
        rustix::io::write(&temp.fd, bytes).map_err(|error| Error::io(ErrorCode::Io, error))
    }

    fn sync_temp(&mut self, temp: &mut Self::Temp) -> Result<()> {
        rustix::fs::fsync(&temp.fd).map_err(|error| Error::io(ErrorCode::Io, error))
    }

    fn rename_temp(&mut self, temp: &mut Self::Temp) -> Result<()> {
        rustix::fs::renameat(
            self.resource.directory.fd(),
            temp.name.as_str(),
            self.resource.directory.fd(),
            self.resource.leaf_os(),
        )
        .map_err(|error| Error::io(ErrorCode::Io, error))?;
        temp.renamed = true;
        Ok(())
    }

    fn sync_parent(&mut self) -> Result<()> {
        rustix::fs::fsync(self.resource.directory.fd())
            .map_err(|error| Error::io(ErrorCode::Io, error))
    }

    fn remove_temp(&mut self, temp: &mut Self::Temp) {
        if !temp.renamed {
            let _ = rustix::fs::unlinkat(
                self.resource.directory.fd(),
                temp.name.as_str(),
                AtFlags::empty(),
            );
        }
    }

    fn quarantine_target(&mut self, quarantine_name: &LeafName) -> Result<()> {
        let quarantine_directory = self
            .quarantine_directory
            .ok_or(Error::Code(ErrorCode::PathRejected))?;
        rustix::fs::renameat_with(
            self.resource.directory.fd(),
            self.resource.leaf_os(),
            quarantine_directory.fd(),
            quarantine_name.as_str(),
            rustix::fs::RenameFlags::NOREPLACE,
        )
        .map_err(|error| Error::io(ErrorCode::Io, error))?;
        rustix::fs::fsync(quarantine_directory.fd())
            .map_err(|error| Error::io(ErrorCode::Io, error))
    }
}
