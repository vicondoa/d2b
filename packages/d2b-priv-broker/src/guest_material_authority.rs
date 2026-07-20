use std::{
    collections::BTreeMap,
    fmt,
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    os::fd::OwnedFd,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use d2b_contracts::v2_component_session::GuestSessionCredentialV1;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::guest_material_store::{
    EnrollmentCommitLookup, EnrollmentRecoveryIdentity, EnrollmentSuccessAuditIdentity,
};
use crate::guest_session_material::{
    GuestAuthorityLookup, GuestBootstrapAuthority, GuestMaterialError, GuestSessionAuthority,
    GuestSessionAuthorityPort,
};

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct BootstrapReplayKey {
    realm_id: String,
    workload_id: String,
    operation_id: Vec<u8>,
    replay_nonce: [u8; 32],
}

impl BootstrapReplayKey {
    fn from_authority(
        realm_id: &str,
        workload_id: &str,
        bootstrap: &GuestBootstrapAuthority,
    ) -> Self {
        Self {
            realm_id: realm_id.to_owned(),
            workload_id: workload_id.to_owned(),
            operation_id: bootstrap.binding.operation_id.as_bytes().to_vec(),
            replay_nonce: bootstrap.binding.replay_nonce,
        }
    }

    fn digest(&self) -> [u8; 32] {
        let mut digest = Sha256::new();
        digest.update(b"d2b-guest-bootstrap-replay-v1\0");
        digest_field(&mut digest, self.realm_id.as_bytes());
        digest_field(&mut digest, self.workload_id.as_bytes());
        digest_field(&mut digest, &self.operation_id);
        digest.update(self.replay_nonce);
        digest.finalize().into()
    }
}

impl fmt::Debug for BootstrapReplayKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("BootstrapReplayKey(REDACTED)")
    }
}

pub trait EnrollmentLedgerTransaction: Send {
    fn commit(&mut self) -> Result<(), GuestMaterialError>;
    fn finalize(&mut self);
    fn rollback(&mut self) -> Result<(), GuestMaterialError>;
}

pub trait BootstrapReplayLedger: Send + Sync {
    fn is_consumed(&self, key: &BootstrapReplayKey) -> Result<bool, GuestMaterialError>;

    fn stage_enrollment(
        &self,
        key: &BootstrapReplayKey,
        realm_id: &str,
        workload_id: &str,
        credential: &[u8],
    ) -> Result<Box<dyn EnrollmentLedgerTransaction>, GuestMaterialError>;

    fn restore_enrollment(
        &self,
        realm_id: &str,
        workload_id: &str,
    ) -> Result<Option<GuestSessionCredentialV1>, GuestMaterialError>;
}

#[derive(Clone)]
struct EnrollmentRecord {
    realm_id: String,
    workload_id: String,
    credential: Zeroizing<Vec<u8>>,
}

#[derive(Default)]
struct MemoryLedgerState {
    records: BTreeMap<[u8; 32], EnrollmentRecord>,
    active: bool,
}

#[derive(Default)]
pub struct InMemoryBootstrapReplayLedger {
    state: Arc<Mutex<MemoryLedgerState>>,
}

impl BootstrapReplayLedger for InMemoryBootstrapReplayLedger {
    fn is_consumed(&self, key: &BootstrapReplayKey) -> Result<bool, GuestMaterialError> {
        Ok(self
            .state
            .lock()
            .map_err(|_| GuestMaterialError::ResourceExhausted)?
            .records
            .contains_key(&key.digest()))
    }

    fn stage_enrollment(
        &self,
        key: &BootstrapReplayKey,
        realm_id: &str,
        workload_id: &str,
        credential: &[u8],
    ) -> Result<Box<dyn EnrollmentLedgerTransaction>, GuestMaterialError> {
        validate_enrolled_bytes(credential)?;
        let digest = key.digest();
        let mut state = self
            .state
            .lock()
            .map_err(|_| GuestMaterialError::ResourceExhausted)?;
        if state.active || state.records.contains_key(&digest) {
            return Err(GuestMaterialError::Replay);
        }
        state.active = true;
        Ok(Box::new(MemoryLedgerTransaction {
            state: Arc::clone(&self.state),
            digest,
            record: Some(EnrollmentRecord {
                realm_id: realm_id.to_owned(),
                workload_id: workload_id.to_owned(),
                credential: Zeroizing::new(credential.to_vec()),
            }),
            committed: false,
            active: true,
        }))
    }

    fn restore_enrollment(
        &self,
        realm_id: &str,
        workload_id: &str,
    ) -> Result<Option<GuestSessionCredentialV1>, GuestMaterialError> {
        restore_record(
            &self
                .state
                .lock()
                .map_err(|_| GuestMaterialError::ResourceExhausted)?
                .records,
            realm_id,
            workload_id,
        )
    }
}

impl EnrollmentCommitLookup for InMemoryBootstrapReplayLedger {
    fn enrollment_committed(
        &self,
        identity: EnrollmentRecoveryIdentity,
    ) -> Result<bool, GuestMaterialError> {
        enrollment_record_matches(
            &self
                .state
                .lock()
                .map_err(|_| GuestMaterialError::ResourceExhausted)?
                .records,
            identity,
        )
    }
}

struct MemoryLedgerTransaction {
    state: Arc<Mutex<MemoryLedgerState>>,
    digest: [u8; 32],
    record: Option<EnrollmentRecord>,
    committed: bool,
    active: bool,
}

impl EnrollmentLedgerTransaction for MemoryLedgerTransaction {
    fn commit(&mut self) -> Result<(), GuestMaterialError> {
        if !self.active || self.committed {
            return Err(GuestMaterialError::HandlerDropped);
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| GuestMaterialError::ResourceExhausted)?;
        let record = self
            .record
            .take()
            .ok_or(GuestMaterialError::HandlerDropped)?;
        if state.records.insert(self.digest, record).is_some() {
            return Err(GuestMaterialError::Replay);
        }
        self.committed = true;
        Ok(())
    }

    fn finalize(&mut self) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.active = false;
        self.active = false;
    }

    fn rollback(&mut self) -> Result<(), GuestMaterialError> {
        if !self.active {
            return Ok(());
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| GuestMaterialError::ResourceExhausted)?;
        if self.committed {
            state.records.remove(&self.digest);
        }
        self.record = None;
        self.committed = false;
        state.active = false;
        self.active = false;
        Ok(())
    }
}

impl Drop for MemoryLedgerTransaction {
    fn drop(&mut self) {
        let _ = self.rollback();
    }
}

pub struct FileBootstrapReplayLedger {
    state: Arc<Mutex<FileReplayState>>,
}

const LEDGER_MAGIC: &[u8; 8] = b"D2BER2\0\0";
const LEDGER_PREPARE: u8 = 1;
const LEDGER_COMMIT: u8 = 2;

struct FileReplayState {
    file: File,
    records: BTreeMap<[u8; 32], EnrollmentRecord>,
    active: bool,
}

impl fmt::Debug for FileBootstrapReplayLedger {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("FileBootstrapReplayLedger(REDACTED)")
    }
}

impl FileBootstrapReplayLedger {
    pub fn open(
        path: PathBuf,
        expected_uid: u32,
        expected_gid: u32,
    ) -> Result<Self, GuestMaterialError> {
        let parent = path
            .parent()
            .ok_or(GuestMaterialError::StorageContractMismatch)?;
        let parent_fd = validate_parent(parent, expected_uid)?;
        let mut options = OpenOptions::new();
        options
            .read(true)
            .append(true)
            .create(true)
            .mode(0o600)
            .custom_flags(nix::libc::O_CLOEXEC | nix::libc::O_NOFOLLOW);
        let file = options
            .open(&path)
            .map_err(|_| GuestMaterialError::AuthorityUnavailable)?;
        let metadata = file
            .metadata()
            .map_err(|_| GuestMaterialError::AuthorityUnavailable)?;
        if !metadata.is_file()
            || metadata.uid() != expected_uid
            || metadata.gid() != expected_gid
            || metadata.mode() & 0o777 != 0o600
        {
            return Err(GuestMaterialError::StorageContractMismatch);
        }
        file.sync_all()
            .map_err(|_| GuestMaterialError::AuthorityUnavailable)?;
        rustix::fs::fsync(&parent_fd).map_err(|_| GuestMaterialError::AuthorityUnavailable)?;
        let records = read_records(&file)?;
        Ok(Self {
            state: Arc::new(Mutex::new(FileReplayState {
                file,
                records,
                active: false,
            })),
        })
    }
}

impl BootstrapReplayLedger for FileBootstrapReplayLedger {
    fn is_consumed(&self, key: &BootstrapReplayKey) -> Result<bool, GuestMaterialError> {
        Ok(self
            .state
            .lock()
            .map_err(|_| GuestMaterialError::ResourceExhausted)?
            .records
            .contains_key(&key.digest()))
    }

    fn stage_enrollment(
        &self,
        key: &BootstrapReplayKey,
        realm_id: &str,
        workload_id: &str,
        credential: &[u8],
    ) -> Result<Box<dyn EnrollmentLedgerTransaction>, GuestMaterialError> {
        validate_enrolled_bytes(credential)?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| GuestMaterialError::ResourceExhausted)?;
        let digest = key.digest();
        if state.active || state.records.contains_key(&digest) {
            return Err(GuestMaterialError::Replay);
        }
        let record = EnrollmentRecord {
            realm_id: realm_id.to_owned(),
            workload_id: workload_id.to_owned(),
            credential: Zeroizing::new(credential.to_vec()),
        };
        let encoded = encode_prepare_record(digest, &record)?;
        let prior_len = state
            .file
            .seek(SeekFrom::End(0))
            .map_err(|_| GuestMaterialError::AuthorityUnavailable)?;
        state
            .file
            .write_all(&encoded)
            .and_then(|()| state.file.sync_data())
            .map_err(|_| GuestMaterialError::AuthorityUnavailable)?;
        state.active = true;
        Ok(Box::new(FileLedgerTransaction {
            state: Arc::clone(&self.state),
            digest,
            record: Some(record),
            prior_len,
            committed: false,
            active: true,
        }))
    }

    fn restore_enrollment(
        &self,
        realm_id: &str,
        workload_id: &str,
    ) -> Result<Option<GuestSessionCredentialV1>, GuestMaterialError> {
        restore_record(
            &self
                .state
                .lock()
                .map_err(|_| GuestMaterialError::ResourceExhausted)?
                .records,
            realm_id,
            workload_id,
        )
    }
}

impl EnrollmentCommitLookup for FileBootstrapReplayLedger {
    fn enrollment_committed(
        &self,
        identity: EnrollmentRecoveryIdentity,
    ) -> Result<bool, GuestMaterialError> {
        enrollment_record_matches(
            &self
                .state
                .lock()
                .map_err(|_| GuestMaterialError::ResourceExhausted)?
                .records,
            identity,
        )
    }
}

struct FileLedgerTransaction {
    state: Arc<Mutex<FileReplayState>>,
    digest: [u8; 32],
    record: Option<EnrollmentRecord>,
    prior_len: u64,
    committed: bool,
    active: bool,
}

impl EnrollmentLedgerTransaction for FileLedgerTransaction {
    fn commit(&mut self) -> Result<(), GuestMaterialError> {
        if !self.active || self.committed {
            return Err(GuestMaterialError::HandlerDropped);
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| GuestMaterialError::ResourceExhausted)?;
        let record = self
            .record
            .as_ref()
            .ok_or(GuestMaterialError::HandlerDropped)?;
        let commit = encode_commit_record(self.digest);
        state
            .file
            .write_all(&commit)
            .and_then(|()| state.file.sync_data())
            .map_err(|_| GuestMaterialError::AuthorityUnavailable)?;
        if state.records.insert(self.digest, record.clone()).is_some() {
            return Err(GuestMaterialError::Replay);
        }
        self.record = None;
        self.committed = true;
        Ok(())
    }

    fn finalize(&mut self) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.active = false;
        self.active = false;
    }

    fn rollback(&mut self) -> Result<(), GuestMaterialError> {
        if !self.active {
            return Ok(());
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| GuestMaterialError::ResourceExhausted)?;
        state
            .file
            .set_len(self.prior_len)
            .and_then(|()| state.file.seek(SeekFrom::End(0)).map(|_| ()))
            .and_then(|()| state.file.sync_data())
            .map_err(|_| GuestMaterialError::AuthorityUnavailable)?;
        state.records.remove(&self.digest);
        self.record = None;
        self.committed = false;
        state.active = false;
        self.active = false;
        Ok(())
    }
}

impl Drop for FileLedgerTransaction {
    fn drop(&mut self) {
        let _ = self.rollback();
    }
}

fn validate_parent(parent: &Path, expected_uid: u32) -> Result<OwnedFd, GuestMaterialError> {
    let fd = crate::sys::path_safe::open_dir_path_safe(parent)
        .map_err(|_| GuestMaterialError::AuthorityUnavailable)?;
    let stat = rustix::fs::fstat(&fd).map_err(|_| GuestMaterialError::AuthorityUnavailable)?;
    if stat.st_uid != expected_uid || stat.st_mode & 0o022 != 0 {
        return Err(GuestMaterialError::StorageContractMismatch);
    }
    Ok(fd)
}

fn read_records(file: &File) -> Result<BTreeMap<[u8; 32], EnrollmentRecord>, GuestMaterialError> {
    let mut reader = file
        .try_clone()
        .map_err(|_| GuestMaterialError::AuthorityUnavailable)?;
    reader
        .seek(SeekFrom::Start(0))
        .map_err(|_| GuestMaterialError::AuthorityUnavailable)?;
    let mut encoded = Zeroizing::new(Vec::new());
    reader
        .read_to_end(&mut encoded)
        .map_err(|_| GuestMaterialError::AuthorityUnavailable)?;
    let mut records = BTreeMap::new();
    let mut prepared: Option<([u8; 32], EnrollmentRecord, usize)> = None;
    let mut offset = 0;
    while offset != encoded.len() {
        let remaining = &encoded[offset..];
        if remaining.len() < 12 {
            truncate_ledger(file, offset)?;
            break;
        }
        if &remaining[..8] != LEDGER_MAGIC {
            return Err(GuestMaterialError::StorageContractMismatch);
        }
        let payload_len = u32::from_be_bytes(
            remaining[8..12]
                .try_into()
                .map_err(|_| GuestMaterialError::StorageContractMismatch)?,
        ) as usize;
        if 12_usize
            .checked_add(payload_len)
            .is_none_or(|length| length > remaining.len())
        {
            truncate_ledger(file, offset)?;
            break;
        }
        let (record, consumed) = decode_record(&encoded[offset..])?;
        match record {
            DecodedLedgerRecord::Prepare { digest, record } => {
                if prepared.is_some() || records.contains_key(&digest) {
                    return Err(GuestMaterialError::StorageContractMismatch);
                }
                prepared = Some((digest, record, offset));
            }
            DecodedLedgerRecord::Commit { digest } => {
                let (prepared_digest, record, _) = prepared
                    .take()
                    .ok_or(GuestMaterialError::StorageContractMismatch)?;
                if digest != prepared_digest || records.insert(digest, record).is_some() {
                    return Err(GuestMaterialError::StorageContractMismatch);
                }
            }
        }
        offset = offset
            .checked_add(consumed)
            .ok_or(GuestMaterialError::StorageContractMismatch)?;
    }
    if let Some((_, _, prepare_offset)) = prepared {
        truncate_ledger(file, prepare_offset)?;
    }
    Ok(records)
}

fn truncate_ledger(file: &File, offset: usize) -> Result<(), GuestMaterialError> {
    file.set_len(offset as u64)
        .and_then(|()| file.sync_data())
        .map_err(|_| GuestMaterialError::AuthorityUnavailable)
}

fn encode_prepare_record(
    digest: [u8; 32],
    record: &EnrollmentRecord,
) -> Result<Zeroizing<Vec<u8>>, GuestMaterialError> {
    let realm_len =
        u16::try_from(record.realm_id.len()).map_err(|_| GuestMaterialError::AuthorityMismatch)?;
    let workload_len = u16::try_from(record.workload_id.len())
        .map_err(|_| GuestMaterialError::AuthorityMismatch)?;
    let credential_len = u16::try_from(record.credential.len())
        .map_err(|_| GuestMaterialError::AuthorityMismatch)?;
    let payload_len = 1_usize
        .checked_add(32)
        .and_then(|value| value.checked_add(2 + record.realm_id.len()))
        .and_then(|value| value.checked_add(2 + record.workload_id.len()))
        .and_then(|value| value.checked_add(2 + record.credential.len()))
        .ok_or(GuestMaterialError::AuthorityMismatch)?;
    let mut encoded = Zeroizing::new(Vec::with_capacity(12 + payload_len));
    encoded.extend_from_slice(LEDGER_MAGIC);
    encoded.extend_from_slice(
        &u32::try_from(payload_len)
            .map_err(|_| GuestMaterialError::AuthorityMismatch)?
            .to_be_bytes(),
    );
    encoded.push(LEDGER_PREPARE);
    encoded.extend_from_slice(&digest);
    encoded.extend_from_slice(&realm_len.to_be_bytes());
    encoded.extend_from_slice(record.realm_id.as_bytes());
    encoded.extend_from_slice(&workload_len.to_be_bytes());
    encoded.extend_from_slice(record.workload_id.as_bytes());
    encoded.extend_from_slice(&credential_len.to_be_bytes());
    encoded.extend_from_slice(&record.credential);
    Ok(encoded)
}

fn encode_commit_record(digest: [u8; 32]) -> Vec<u8> {
    let payload_len = 1_u32 + 32;
    let mut encoded = Vec::with_capacity(12 + payload_len as usize);
    encoded.extend_from_slice(LEDGER_MAGIC);
    encoded.extend_from_slice(&payload_len.to_be_bytes());
    encoded.push(LEDGER_COMMIT);
    encoded.extend_from_slice(&digest);
    encoded
}

enum DecodedLedgerRecord {
    Prepare {
        digest: [u8; 32],
        record: EnrollmentRecord,
    },
    Commit {
        digest: [u8; 32],
    },
}

fn decode_record(encoded: &[u8]) -> Result<(DecodedLedgerRecord, usize), GuestMaterialError> {
    if encoded.len() < 12 || &encoded[..8] != LEDGER_MAGIC {
        return Err(GuestMaterialError::StorageContractMismatch);
    }
    let payload_len = u32::from_be_bytes(
        encoded[8..12]
            .try_into()
            .map_err(|_| GuestMaterialError::StorageContractMismatch)?,
    ) as usize;
    let end = 12_usize
        .checked_add(payload_len)
        .filter(|end| *end <= encoded.len())
        .ok_or(GuestMaterialError::StorageContractMismatch)?;
    let mut offset = 12;
    let kind = *take(encoded, &mut offset, 1, end)?
        .first()
        .ok_or(GuestMaterialError::StorageContractMismatch)?;
    let digest = take_array::<32>(encoded, &mut offset, end)?;
    if kind == LEDGER_COMMIT {
        if offset != end {
            return Err(GuestMaterialError::StorageContractMismatch);
        }
        return Ok((DecodedLedgerRecord::Commit { digest }, end));
    }
    if kind != LEDGER_PREPARE {
        return Err(GuestMaterialError::StorageContractMismatch);
    }
    let realm_id = take_string(encoded, &mut offset, end)?;
    let workload_id = take_string(encoded, &mut offset, end)?;
    let credential_len = usize::from(take_u16(encoded, &mut offset, end)?);
    let credential = Zeroizing::new(take(encoded, &mut offset, credential_len, end)?.to_vec());
    if offset != end {
        return Err(GuestMaterialError::StorageContractMismatch);
    }
    validate_enrolled_bytes(&credential)?;
    Ok((
        DecodedLedgerRecord::Prepare {
            digest,
            record: EnrollmentRecord {
                realm_id,
                workload_id,
                credential,
            },
        },
        end,
    ))
}

fn take<'a>(
    encoded: &'a [u8],
    offset: &mut usize,
    len: usize,
    end: usize,
) -> Result<&'a [u8], GuestMaterialError> {
    let next = offset
        .checked_add(len)
        .filter(|next| *next <= end)
        .ok_or(GuestMaterialError::StorageContractMismatch)?;
    let bytes = encoded
        .get(*offset..next)
        .ok_or(GuestMaterialError::StorageContractMismatch)?;
    *offset = next;
    Ok(bytes)
}

fn take_u16(encoded: &[u8], offset: &mut usize, end: usize) -> Result<u16, GuestMaterialError> {
    Ok(u16::from_be_bytes(
        take(encoded, offset, 2, end)?
            .try_into()
            .map_err(|_| GuestMaterialError::StorageContractMismatch)?,
    ))
}

fn take_array<const N: usize>(
    encoded: &[u8],
    offset: &mut usize,
    end: usize,
) -> Result<[u8; N], GuestMaterialError> {
    take(encoded, offset, N, end)?
        .try_into()
        .map_err(|_| GuestMaterialError::StorageContractMismatch)
}

fn take_string(
    encoded: &[u8],
    offset: &mut usize,
    end: usize,
) -> Result<String, GuestMaterialError> {
    let len = usize::from(take_u16(encoded, offset, end)?);
    if len == 0 || len > 128 {
        return Err(GuestMaterialError::StorageContractMismatch);
    }
    std::str::from_utf8(take(encoded, offset, len, end)?)
        .map(str::to_owned)
        .map_err(|_| GuestMaterialError::StorageContractMismatch)
}

fn validate_enrolled_bytes(encoded: &[u8]) -> Result<GuestSessionCredentialV1, GuestMaterialError> {
    let credential = GuestSessionCredentialV1::decode(encoded)
        .map_err(|_| GuestMaterialError::AuthorityMismatch)?;
    if credential.guest_identity_is_unbound() || credential.bootstrap().is_some() {
        return Err(GuestMaterialError::AuthorityMismatch);
    }
    Ok(credential)
}

fn enrollment_record_matches(
    records: &BTreeMap<[u8; 32], EnrollmentRecord>,
    identity: EnrollmentRecoveryIdentity,
) -> Result<bool, GuestMaterialError> {
    if identity.replay_digest == [0; 32]
        || identity.credential_digest == [0; 32]
        || identity.configured_digest == [0; 32]
        || identity.success_audit.credential_digest != identity.credential_digest
        || identity.success_audit.configured_digest != identity.configured_digest
    {
        return Err(GuestMaterialError::StorageContractMismatch);
    }
    Ok(records.get(&identity.replay_digest).is_some_and(|record| {
        <[u8; 32]>::from(Sha256::digest(&record.credential)) == identity.credential_digest
    }))
}

fn restore_record(
    records: &BTreeMap<[u8; 32], EnrollmentRecord>,
    realm_id: &str,
    workload_id: &str,
) -> Result<Option<GuestSessionCredentialV1>, GuestMaterialError> {
    let mut restored = None;
    for record in records
        .values()
        .filter(|record| record.realm_id == realm_id && record.workload_id == workload_id)
    {
        let credential = validate_enrolled_bytes(&record.credential)?;
        if restored
            .as_ref()
            .is_some_and(|prior: &GuestSessionCredentialV1| {
                prior.session_generation() == credential.session_generation()
                    && (prior.parent_static_public_key() != credential.parent_static_public_key()
                        || prior.channel_binding() != credential.channel_binding()
                        || prior.guest_identity_digest() != credential.guest_identity_digest()
                        || prior.guest_static_public_key() != credential.guest_static_public_key())
            })
        {
            return Err(GuestMaterialError::AuthorityMismatch);
        }
        if restored
            .as_ref()
            .is_none_or(|prior| prior.session_generation() < credential.session_generation())
        {
            restored = Some(credential);
        }
    }
    Ok(restored)
}

struct StoredAuthority {
    session_generation: u64,
    parent_static_public_key: [u8; 32],
    channel_binding: [u8; 32],
    guest_identity_digest: [u8; 32],
    guest_static_public_key: [u8; 32],
    bootstrap: Option<GuestBootstrapAuthority>,
}

pub struct RealmGuestSessionAuthorityConnector {
    realm_id: String,
    ledger: Arc<dyn BootstrapReplayLedger>,
    authorities: Arc<Mutex<BTreeMap<String, StoredAuthority>>>,
    pending: Arc<Mutex<BTreeMap<String, BootstrapReplayKey>>>,
}

impl fmt::Debug for RealmGuestSessionAuthorityConnector {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RealmGuestSessionAuthorityConnector")
            .field("realm_id", &"<closed-id>")
            .field("authorities", &"<redacted>")
            .finish()
    }
}

impl RealmGuestSessionAuthorityConnector {
    pub fn new(realm_id: String, ledger: Arc<dyn BootstrapReplayLedger>) -> Self {
        Self {
            realm_id,
            ledger,
            authorities: Arc::new(Mutex::new(BTreeMap::new())),
            pending: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    pub fn install(&self, mut authority: GuestSessionAuthority) -> Result<(), GuestMaterialError> {
        if authority.realm_id != self.realm_id
            || authority.workload_id.is_empty()
            || authority.session_generation == 0
        {
            return Err(GuestMaterialError::AuthorityMismatch);
        }
        if let Some(restored) = self
            .ledger
            .restore_enrollment(&self.realm_id, &authority.workload_id)?
        {
            if restored.session_generation() > authority.session_generation {
                return Err(GuestMaterialError::AuthorityMismatch);
            }
            if restored.session_generation() == authority.session_generation {
                if restored.parent_static_public_key() != &authority.parent_static_public_key
                    || restored.channel_binding() != &authority.channel_binding
                {
                    return Err(GuestMaterialError::AuthorityMismatch);
                }
                authority.guest_identity_digest = *restored
                    .guest_identity_digest()
                    .ok_or(GuestMaterialError::AuthorityMismatch)?;
                authority.guest_static_public_key = *restored
                    .guest_static_public_key()
                    .ok_or(GuestMaterialError::AuthorityMismatch)?;
                authority.bootstrap = None;
            }
        }
        let mut authorities = self
            .authorities
            .lock()
            .map_err(|_| GuestMaterialError::ResourceExhausted)?;
        if let Some(existing) = authorities.get(&authority.workload_id) {
            if existing.session_generation > authority.session_generation {
                return Err(GuestMaterialError::GenerationMismatch);
            }
            if existing.session_generation == authority.session_generation
                && (existing.parent_static_public_key != authority.parent_static_public_key
                    || existing.channel_binding != authority.channel_binding
                    || existing.guest_identity_digest != authority.guest_identity_digest
                    || existing.guest_static_public_key != authority.guest_static_public_key
                    || existing.bootstrap.as_ref().map(|value| &value.binding)
                        != authority.bootstrap.as_ref().map(|value| &value.binding))
            {
                return Err(GuestMaterialError::AuthorityMismatch);
            }
        }
        let workload_id = authority.workload_id;
        authorities.insert(
            workload_id.clone(),
            StoredAuthority {
                session_generation: authority.session_generation,
                parent_static_public_key: authority.parent_static_public_key,
                channel_binding: authority.channel_binding,
                guest_identity_digest: authority.guest_identity_digest,
                guest_static_public_key: authority.guest_static_public_key,
                bootstrap: authority.bootstrap,
            },
        );
        self.pending
            .lock()
            .map_err(|_| GuestMaterialError::ResourceExhausted)?
            .remove(&workload_id);
        Ok(())
    }

    pub fn bind_established_identity(
        &self,
        realm_id: &str,
        workload_id: &str,
        session_generation: u64,
        guest_identity_digest: [u8; 32],
        guest_static_public_key: [u8; 32],
    ) -> Result<(), GuestMaterialError> {
        if realm_id != self.realm_id
            || guest_identity_digest == [0; 32]
            || guest_static_public_key == [0; 32]
        {
            return Err(GuestMaterialError::AuthorityMismatch);
        }
        let mut authorities = self
            .authorities
            .lock()
            .map_err(|_| GuestMaterialError::ResourceExhausted)?;
        let authority = authorities
            .get_mut(workload_id)
            .ok_or(GuestMaterialError::AuthorityUnavailable)?;
        if authority.session_generation != session_generation {
            return Err(GuestMaterialError::GenerationMismatch);
        }
        let unbound = authority.guest_identity_digest == [0; 32]
            && authority.guest_static_public_key == [0; 32];
        let already_bound = authority.guest_identity_digest == guest_identity_digest
            && authority.guest_static_public_key == guest_static_public_key;
        if !unbound && !already_bound {
            return Err(GuestMaterialError::AuthorityMismatch);
        }
        authority.guest_identity_digest = guest_identity_digest;
        authority.guest_static_public_key = guest_static_public_key;
        Ok(())
    }

    pub fn stage_enrolled_credential(
        &self,
        realm_id: &str,
        workload_id: &str,
        credential: &GuestSessionCredentialV1,
        encoded: &[u8],
    ) -> Result<AuthorityEnrollmentTransaction, GuestMaterialError> {
        if realm_id != self.realm_id
            || credential.guest_identity_is_unbound()
            || credential.bootstrap().is_some()
        {
            return Err(GuestMaterialError::AuthorityMismatch);
        }
        let guest_identity_digest = *credential
            .guest_identity_digest()
            .ok_or(GuestMaterialError::AuthorityMismatch)?;
        let guest_static_public_key = *credential
            .guest_static_public_key()
            .ok_or(GuestMaterialError::AuthorityMismatch)?;
        if guest_identity_digest != Sha256::digest(guest_static_public_key).as_slice() {
            return Err(GuestMaterialError::AuthorityMismatch);
        }
        let authorities = self
            .authorities
            .lock()
            .map_err(|_| GuestMaterialError::ResourceExhausted)?;
        let authority = authorities
            .get(workload_id)
            .ok_or(GuestMaterialError::AuthorityUnavailable)?;
        if authority.session_generation != credential.session_generation() {
            return Err(GuestMaterialError::GenerationMismatch);
        }
        if authority.parent_static_public_key != *credential.parent_static_public_key()
            || authority.channel_binding != *credential.channel_binding()
            || authority.bootstrap.is_some()
        {
            return Err(GuestMaterialError::AuthorityMismatch);
        }
        let prior_identity = (
            authority.guest_identity_digest,
            authority.guest_static_public_key,
        );
        drop(authorities);
        let key = self
            .pending
            .lock()
            .map_err(|_| GuestMaterialError::ResourceExhausted)?
            .get(workload_id)
            .cloned()
            .ok_or(GuestMaterialError::Replay)?;
        let ledger = self
            .ledger
            .stage_enrollment(&key, realm_id, workload_id, encoded)?;
        Ok(AuthorityEnrollmentTransaction {
            authorities: Arc::clone(&self.authorities),
            pending: Arc::clone(&self.pending),
            ledger,
            replay_digest: key.digest(),
            workload_id: workload_id.to_owned(),
            session_generation: credential.session_generation(),
            prior_identity,
            enrolled_identity: (guest_identity_digest, guest_static_public_key),
            memory_applied: false,
            active: true,
        })
    }
}

pub struct AuthorityEnrollmentTransaction {
    authorities: Arc<Mutex<BTreeMap<String, StoredAuthority>>>,
    pending: Arc<Mutex<BTreeMap<String, BootstrapReplayKey>>>,
    ledger: Box<dyn EnrollmentLedgerTransaction>,
    replay_digest: [u8; 32],
    workload_id: String,
    session_generation: u64,
    prior_identity: ([u8; 32], [u8; 32]),
    enrolled_identity: ([u8; 32], [u8; 32]),
    memory_applied: bool,
    active: bool,
}

impl fmt::Debug for AuthorityEnrollmentTransaction {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AuthorityEnrollmentTransaction(REDACTED)")
    }
}

impl AuthorityEnrollmentTransaction {
    pub fn recovery_identity(
        &self,
        credential_digest: [u8; 32],
        configured_digest: [u8; 32],
        success_audit: EnrollmentSuccessAuditIdentity,
    ) -> EnrollmentRecoveryIdentity {
        EnrollmentRecoveryIdentity {
            replay_digest: self.replay_digest,
            credential_digest,
            configured_digest,
            success_audit,
        }
    }

    pub fn apply_memory(&mut self) -> Result<(), GuestMaterialError> {
        if !self.active || self.memory_applied {
            return Err(GuestMaterialError::Replay);
        }
        let mut authorities = self
            .authorities
            .lock()
            .map_err(|_| GuestMaterialError::ResourceExhausted)?;
        let authority = authorities
            .get_mut(&self.workload_id)
            .ok_or(GuestMaterialError::AuthorityUnavailable)?;
        if authority.session_generation != self.session_generation
            || (
                authority.guest_identity_digest,
                authority.guest_static_public_key,
            ) != self.prior_identity
        {
            return Err(GuestMaterialError::GenerationMismatch);
        }
        authority.guest_identity_digest = self.enrolled_identity.0;
        authority.guest_static_public_key = self.enrolled_identity.1;
        self.memory_applied = true;
        Ok(())
    }

    pub fn commit(&mut self) -> Result<(), GuestMaterialError> {
        if !self.active || !self.memory_applied {
            return Err(GuestMaterialError::HandlerDropped);
        }
        let pending = self
            .pending
            .lock()
            .map_err(|_| GuestMaterialError::ResourceExhausted)?;
        if !pending.contains_key(&self.workload_id) {
            return Err(GuestMaterialError::Replay);
        }
        self.ledger.commit()?;
        Ok(())
    }

    pub fn finalize(&mut self) {
        self.ledger.finalize();
        self.pending
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&self.workload_id);
        self.active = false;
    }

    pub fn rollback(&mut self) -> Result<(), GuestMaterialError> {
        if !self.active {
            return Ok(());
        }
        if self.memory_applied {
            let mut authorities = self
                .authorities
                .lock()
                .map_err(|_| GuestMaterialError::ResourceExhausted)?;
            let authority = authorities
                .get_mut(&self.workload_id)
                .ok_or(GuestMaterialError::AuthorityUnavailable)?;
            authority.guest_identity_digest = self.prior_identity.0;
            authority.guest_static_public_key = self.prior_identity.1;
            self.memory_applied = false;
        }
        self.ledger.rollback()?;
        self.active = false;
        Ok(())
    }
}

impl Drop for AuthorityEnrollmentTransaction {
    fn drop(&mut self) {
        let _ = self.rollback();
    }
}

#[async_trait]
impl GuestSessionAuthorityPort for RealmGuestSessionAuthorityConnector {
    async fn resolve(
        &self,
        lookup: GuestAuthorityLookup,
    ) -> Result<GuestSessionAuthority, GuestMaterialError> {
        if lookup.realm_id != self.realm_id {
            return Err(GuestMaterialError::AuthorityMismatch);
        }
        let mut authorities = self
            .authorities
            .lock()
            .map_err(|_| GuestMaterialError::ResourceExhausted)?;
        let authority = authorities
            .get_mut(&lookup.workload_id)
            .ok_or(GuestMaterialError::AuthorityUnavailable)?;
        if authority.session_generation != lookup.session_generation {
            return Err(GuestMaterialError::GenerationMismatch);
        }
        let bootstrap = authority.bootstrap.take();
        if let Some(bootstrap_ref) = bootstrap.as_ref() {
            let key = BootstrapReplayKey::from_authority(
                &self.realm_id,
                &lookup.workload_id,
                bootstrap_ref,
            );
            let consumed = self.ledger.is_consumed(&key)?;
            let mut pending = self
                .pending
                .lock()
                .map_err(|_| GuestMaterialError::ResourceExhausted)?;
            if consumed || pending.contains_key(&lookup.workload_id) {
                authority.bootstrap = bootstrap;
                return Err(GuestMaterialError::Replay);
            }
            pending.insert(lookup.workload_id.clone(), key);
        }
        Ok(GuestSessionAuthority {
            realm_id: self.realm_id.clone(),
            workload_id: lookup.workload_id,
            session_generation: authority.session_generation,
            parent_static_public_key: authority.parent_static_public_key,
            channel_binding: authority.channel_binding,
            guest_identity_digest: authority.guest_identity_digest,
            guest_static_public_key: authority.guest_static_public_key,
            bootstrap,
        })
    }
}

fn digest_field(digest: &mut Sha256, field: &[u8]) {
    digest.update(u32::try_from(field.len()).unwrap_or(u32::MAX).to_be_bytes());
    digest.update(field);
}

fn hex_digest(digest: &[u8; 32]) -> String {
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use d2b_contracts::v2_component_session::{
        BootstrapPskBinding, GuestBootstrapPsk, OperationId,
    };

    use super::*;

    fn authority(realm: &str, workload: &str, request_marker: u8) -> GuestSessionAuthority {
        GuestSessionAuthority {
            realm_id: realm.to_owned(),
            workload_id: workload.to_owned(),
            session_generation: 7,
            parent_static_public_key: [1; 32],
            channel_binding: [2; 32],
            guest_identity_digest: [3; 32],
            guest_static_public_key: [4; 32],
            bootstrap: Some(GuestBootstrapAuthority {
                binding: BootstrapPskBinding {
                    operation_id: OperationId::new(vec![0x55; 16]).unwrap(),
                    replay_nonce: [0x66; 32],
                    expires_at_unix_ms: 20_000,
                },
                issued_at_unix_ms: 10_000,
                psk: GuestBootstrapPsk::generate_with(|psk| {
                    psk.fill(request_marker);
                    Ok(())
                })
                .unwrap(),
            }),
        }
    }

    fn lookup(operation: &str) -> GuestAuthorityLookup {
        GuestAuthorityLookup {
            realm_id: "work".to_owned(),
            workload_id: "editor".to_owned(),
            operation_id: operation.to_owned(),
            storage_ref: "storage".to_owned(),
            request_digest: [9; 32],
            session_generation: 7,
        }
    }

    fn enrolled_credential(generation: u64) -> GuestSessionCredentialV1 {
        let guest_public = [0x44; 32];
        GuestSessionCredentialV1::new(
            generation,
            [1; 32],
            [2; 32],
            d2b_contracts::v2_component_session::GuestIdentityBindingV1::Enrolled {
                guest_identity_digest: Sha256::digest(guest_public).into(),
                guest_static_public_key: guest_public,
            },
            None,
        )
        .unwrap()
    }

    #[tokio::test]
    async fn uncommitted_bootstrap_is_not_durably_consumed_across_recreation() {
        let ledger: Arc<dyn BootstrapReplayLedger> =
            Arc::new(InMemoryBootstrapReplayLedger::default());
        let first =
            RealmGuestSessionAuthorityConnector::new("work".to_owned(), Arc::clone(&ledger));
        first.install(authority("work", "editor", 0x77)).unwrap();
        first.resolve(lookup("request-one")).await.unwrap();

        let recreated =
            RealmGuestSessionAuthorityConnector::new("work".to_owned(), Arc::clone(&ledger));
        let mut rekeyed = authority("work", "editor", 0x88);
        rekeyed.session_generation = 8;
        recreated.install(rekeyed).unwrap();
        let mut second_lookup = lookup("request-two");
        second_lookup.session_generation = 8;
        assert!(recreated.resolve(second_lookup).await.is_ok());
    }

    #[tokio::test]
    async fn connector_rejects_cross_realm_authority_and_lookup() {
        let connector = RealmGuestSessionAuthorityConnector::new(
            "work".to_owned(),
            Arc::new(InMemoryBootstrapReplayLedger::default()),
        );
        assert_eq!(
            connector
                .install(authority("personal", "editor", 0x77))
                .unwrap_err(),
            GuestMaterialError::AuthorityMismatch
        );
        connector
            .install(authority("work", "editor", 0x77))
            .unwrap();
        let mut wrong = lookup("request");
        wrong.realm_id = "personal".to_owned();
        assert_eq!(
            connector.resolve(wrong).await.unwrap_err(),
            GuestMaterialError::AuthorityMismatch
        );
    }

    #[tokio::test]
    async fn unbound_bootstrap_persists_established_identity_for_enrolled_use() {
        let ledger: Arc<dyn BootstrapReplayLedger> =
            Arc::new(InMemoryBootstrapReplayLedger::default());
        let connector =
            RealmGuestSessionAuthorityConnector::new("work".to_owned(), Arc::clone(&ledger));
        let mut unbound = authority("work", "editor", 0x77);
        unbound.guest_identity_digest = [0; 32];
        unbound.guest_static_public_key = [0; 32];
        connector.install(unbound).unwrap();
        let bootstrap = connector.resolve(lookup("bootstrap")).await.unwrap();
        assert_eq!(bootstrap.guest_identity_digest, [0; 32]);
        assert_eq!(bootstrap.guest_static_public_key, [0; 32]);

        let credential = enrolled_credential(7);
        let encoded = credential.encode().unwrap();
        let mut enrollment = connector
            .stage_enrolled_credential("work", "editor", &credential, encoded.as_slice())
            .unwrap();
        enrollment.apply_memory().unwrap();
        enrollment.commit().unwrap();
        enrollment.finalize();
        let enrolled = connector.resolve(lookup("enrolled")).await.unwrap();
        assert_eq!(
            enrolled.guest_identity_digest,
            Sha256::digest([0x44; 32]).as_slice()
        );
        assert_eq!(enrolled.guest_static_public_key, [0x44; 32]);
        assert!(enrolled.bootstrap.is_none());

        let restored =
            RealmGuestSessionAuthorityConnector::new("work".to_owned(), Arc::clone(&ledger));
        let mut restart_authority = authority("work", "editor", 0x99);
        restart_authority.guest_identity_digest = [0; 32];
        restart_authority.guest_static_public_key = [0; 32];
        restored.install(restart_authority).unwrap();
        let after_restart = restored.resolve(lookup("after-restart")).await.unwrap();
        assert_eq!(
            after_restart.guest_identity_digest,
            Sha256::digest([0x44; 32]).as_slice()
        );
        assert!(after_restart.bootstrap.is_none());
    }

    #[test]
    fn file_ledger_rejects_consumed_binding_after_reopen() {
        let root = crate::test_tempdir("guest-bootstrap-replay-ledger");
        let uid = rustix::process::getuid().as_raw();
        let gid = rustix::process::getgid().as_raw();
        let path = root.path().join("replay.ledger");
        let bootstrap = authority("work", "editor", 0x77)
            .bootstrap
            .expect("bootstrap");
        let key = BootstrapReplayKey::from_authority("work", "editor", &bootstrap);
        let ledger = FileBootstrapReplayLedger::open(path.clone(), uid, gid).unwrap();
        let credential = enrolled_credential(7);
        let encoded = credential.encode().unwrap();
        let mut transaction = ledger
            .stage_enrollment(&key, "work", "editor", encoded.as_slice())
            .unwrap();
        transaction.commit().unwrap();
        transaction.finalize();
        let reopened = FileBootstrapReplayLedger::open(path, uid, gid).unwrap();
        assert!(reopened.is_consumed(&key).unwrap());
        assert!(
            reopened
                .restore_enrollment("work", "editor")
                .unwrap()
                .is_some()
        );
        assert!(matches!(
            reopened.stage_enrollment(&key, "work", "editor", encoded.as_slice()),
            Err(GuestMaterialError::Replay)
        ));
    }

    #[test]
    fn file_ledger_rollback_is_durable_across_reopen() {
        let root = crate::test_tempdir("guest-bootstrap-replay-rollback");
        let uid = rustix::process::getuid().as_raw();
        let gid = rustix::process::getgid().as_raw();
        let path = root.path().join("replay.ledger");
        let bootstrap = authority("work", "editor", 0x77)
            .bootstrap
            .expect("bootstrap");
        let key = BootstrapReplayKey::from_authority("work", "editor", &bootstrap);
        let ledger = FileBootstrapReplayLedger::open(path.clone(), uid, gid).unwrap();
        let credential = enrolled_credential(7);
        let encoded = credential.encode().unwrap();
        let transaction = ledger
            .stage_enrollment(&key, "work", "editor", encoded.as_slice())
            .unwrap();
        assert!(!ledger.is_consumed(&key).unwrap());
        assert!(
            ledger
                .restore_enrollment("work", "editor")
                .unwrap()
                .is_none()
        );
        drop(transaction);
        let reopened = FileBootstrapReplayLedger::open(path, uid, gid).unwrap();
        assert!(!reopened.is_consumed(&key).unwrap());
        assert!(
            reopened
                .restore_enrollment("work", "editor")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn file_ledger_restart_discards_fsynced_prepare_without_commit_marker() {
        let root = crate::test_tempdir("guest-bootstrap-replay-crash-prepare");
        let uid = rustix::process::getuid().as_raw();
        let gid = rustix::process::getgid().as_raw();
        let path = root.path().join("replay.ledger");
        let bootstrap = authority("work", "editor", 0x77)
            .bootstrap
            .expect("bootstrap");
        let key = BootstrapReplayKey::from_authority("work", "editor", &bootstrap);
        let ledger = FileBootstrapReplayLedger::open(path.clone(), uid, gid).unwrap();
        let credential = enrolled_credential(7);
        let encoded = credential.encode().unwrap();
        let transaction = ledger
            .stage_enrollment(&key, "work", "editor", encoded.as_slice())
            .unwrap();
        assert!(std::fs::metadata(&path).unwrap().len() > 0);
        std::mem::forget(transaction);
        drop(ledger);

        let reopened = FileBootstrapReplayLedger::open(path.clone(), uid, gid).unwrap();
        assert!(!reopened.is_consumed(&key).unwrap());
        assert!(
            reopened
                .restore_enrollment("work", "editor")
                .unwrap()
                .is_none()
        );
        assert_eq!(std::fs::metadata(path).unwrap().len(), 0);
    }

    #[test]
    fn file_ledger_restart_truncates_only_torn_uncommitted_tail() {
        let root = crate::test_tempdir("guest-bootstrap-replay-torn-tail");
        let uid = rustix::process::getuid().as_raw();
        let gid = rustix::process::getgid().as_raw();
        let path = root.path().join("replay.ledger");
        let bootstrap = authority("work", "editor", 0x77)
            .bootstrap
            .expect("bootstrap");
        let key = BootstrapReplayKey::from_authority("work", "editor", &bootstrap);
        let ledger = FileBootstrapReplayLedger::open(path.clone(), uid, gid).unwrap();
        let credential = enrolled_credential(7);
        let encoded = credential.encode().unwrap();
        let mut transaction = ledger
            .stage_enrollment(&key, "work", "editor", encoded.as_slice())
            .unwrap();
        transaction.commit().unwrap();
        transaction.finalize();
        drop(transaction);
        drop(ledger);

        let committed_len = std::fs::metadata(&path).unwrap().len();
        let mut file = OpenOptions::new().append(true).open(&path).unwrap();
        file.write_all(&LEDGER_MAGIC[..6]).unwrap();
        file.sync_data().unwrap();
        drop(file);

        let reopened = FileBootstrapReplayLedger::open(path.clone(), uid, gid).unwrap();
        assert!(
            reopened
                .restore_enrollment("work", "editor")
                .unwrap()
                .is_some()
        );
        assert_eq!(std::fs::metadata(path).unwrap().len(), committed_len);
    }
}
