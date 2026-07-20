use std::{
    fs::File,
    io::Read,
    os::fd::{AsFd, OwnedFd},
    path::Path,
    sync::Arc,
};

use d2b_contracts::v2_component_session::GuestSessionCredentialBytes;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::guest_session_material::{GuestMaterialError, GuestMaterialTarget};

const MAX_MATERIAL_BYTES: usize = 2 * 1024 * 1024;
const RECOVERY_MAGIC: &[u8; 8] = b"D2BGRJ1\0";
const RECOVERY_PREPARED: u8 = 1;
const RECOVERY_PAIR_LEDGER_COMMITTED: u8 = 2;
const RECOVERY_AUDIT_COMMITTED: u8 = 3;
const RECOVERY_JOURNAL: &str = ".d2b-enrollment-recovery-v1";
const PRIOR_SESSION: &str = ".d2b-enrollment-prior-session-v1";
const PRIOR_CONFIGURED: &str = ".d2b-enrollment-prior-configured-v1";
const NEW_SESSION: &str = ".d2b-enrollment-new-session-v1";
const NEW_CONFIGURED: &str = ".d2b-enrollment-new-configured-v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EnrollmentSuccessAuditIdentity {
    pub session_generation: u64,
    pub request_digest: [u8; 32],
    pub credential_digest: [u8; 32],
    pub configured_digest: [u8; 32],
    pub outcome: u8,
    pub dedup_key: [u8; 32],
}

impl EnrollmentSuccessAuditIdentity {
    pub fn new(
        session_generation: u64,
        request_digest: [u8; 32],
        credential_digest: [u8; 32],
        configured_digest: [u8; 32],
    ) -> Result<Self, GuestMaterialError> {
        if session_generation == 0
            || request_digest == [0; 32]
            || credential_digest == [0; 32]
            || configured_digest == [0; 32]
        {
            return Err(GuestMaterialError::StorageContractMismatch);
        }
        let outcome = 1;
        let dedup_key = success_audit_dedup(
            session_generation,
            request_digest,
            credential_digest,
            configured_digest,
            outcome,
        );
        Ok(Self {
            session_generation,
            request_digest,
            credential_digest,
            configured_digest,
            outcome,
            dedup_key,
        })
    }

    pub(crate) fn validate(self) -> Result<(), GuestMaterialError> {
        if self.outcome != 1
            || self.session_generation == 0
            || self.request_digest == [0; 32]
            || self.credential_digest == [0; 32]
            || self.configured_digest == [0; 32]
            || self.dedup_key
                != success_audit_dedup(
                    self.session_generation,
                    self.request_digest,
                    self.credential_digest,
                    self.configured_digest,
                    self.outcome,
                )
        {
            return Err(GuestMaterialError::StorageContractMismatch);
        }
        Ok(())
    }
}

fn success_audit_dedup(
    session_generation: u64,
    request_digest: [u8; 32],
    credential_digest: [u8; 32],
    configured_digest: [u8; 32],
    outcome: u8,
) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"d2b-guest-material-success-audit-v1\0");
    digest.update(session_generation.to_be_bytes());
    digest.update(request_digest);
    digest.update(credential_digest);
    digest.update(configured_digest);
    digest.update([outcome]);
    digest.finalize().into()
}

pub trait EnrollmentSuccessAudit: Send + Sync {
    fn ensure_success(
        &self,
        identity: EnrollmentSuccessAuditIdentity,
    ) -> Result<(), GuestMaterialError>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EnrollmentRecoveryIdentity {
    pub replay_digest: [u8; 32],
    pub credential_digest: [u8; 32],
    pub configured_digest: [u8; 32],
    pub success_audit: EnrollmentSuccessAuditIdentity,
}

pub trait EnrollmentCommitLookup: Send + Sync {
    fn enrollment_committed(
        &self,
        identity: EnrollmentRecoveryIdentity,
    ) -> Result<bool, GuestMaterialError>;
}

pub trait GuestMaterialTransaction: Send {
    /// Complete fallible durability work while retaining rollback authority.
    fn commit(&mut self) -> Result<(), GuestMaterialError>;
    /// Persist the cross-file transaction commit marker.
    fn mark_committed(&mut self) -> Result<(), GuestMaterialError>;
    /// Persist that the success audit record is durable.
    fn mark_audit_committed(&mut self) -> Result<(), GuestMaterialError>;
    /// Disarm rollback after every participant has committed.
    fn finalize(&mut self);
    fn rollback(&mut self) -> Result<(), GuestMaterialError>;
}

pub trait GuestMaterialStore: Send + Sync {
    fn stage_pair(
        &self,
        session_target: &GuestMaterialTarget,
        session_bytes: &GuestSessionCredentialBytes,
        configured_target: &GuestMaterialTarget,
        configured_bytes: &[u8],
    ) -> Result<Box<dyn GuestMaterialTransaction>, GuestMaterialError>;

    fn stage_enrollment_pair(
        &self,
        session_target: &GuestMaterialTarget,
        session_bytes: &GuestSessionCredentialBytes,
        configured_target: &GuestMaterialTarget,
        configured_bytes: &[u8],
        _: EnrollmentRecoveryIdentity,
    ) -> Result<Box<dyn GuestMaterialTransaction>, GuestMaterialError> {
        self.stage_pair(
            session_target,
            session_bytes,
            configured_target,
            configured_bytes,
        )
    }
}

pub struct FilesystemGuestMaterialStore {
    backend: Arc<dyn PairMutationBackend>,
    require_root: bool,
}

impl Default for FilesystemGuestMaterialStore {
    fn default() -> Self {
        Self {
            backend: Arc::new(LivePairMutationBackend),
            require_root: true,
        }
    }
}

impl FilesystemGuestMaterialStore {
    pub fn realm_child() -> Self {
        Self {
            backend: Arc::new(LivePairMutationBackend),
            require_root: true,
        }
    }
}

impl GuestMaterialStore for FilesystemGuestMaterialStore {
    fn stage_pair(
        &self,
        session_target: &GuestMaterialTarget,
        session_bytes: &GuestSessionCredentialBytes,
        configured_target: &GuestMaterialTarget,
        configured_bytes: &[u8],
    ) -> Result<Box<dyn GuestMaterialTransaction>, GuestMaterialError> {
        stage_pair_with_backend(
            session_target,
            session_bytes.as_slice(),
            configured_target,
            configured_bytes,
            self.require_root,
            Arc::clone(&self.backend),
        )
        .map(|transaction| Box::new(transaction) as Box<dyn GuestMaterialTransaction>)
    }

    fn stage_enrollment_pair(
        &self,
        session_target: &GuestMaterialTarget,
        session_bytes: &GuestSessionCredentialBytes,
        configured_target: &GuestMaterialTarget,
        configured_bytes: &[u8],
        identity: EnrollmentRecoveryIdentity,
    ) -> Result<Box<dyn GuestMaterialTransaction>, GuestMaterialError> {
        stage_enrollment_pair_with_backend(
            session_target,
            session_bytes.as_slice(),
            configured_target,
            configured_bytes,
            identity,
            self.require_root,
            Arc::clone(&self.backend),
        )
        .map(|transaction| Box::new(transaction) as Box<dyn GuestMaterialTransaction>)
    }
}

pub struct NoopGuestMaterialStore;

impl GuestMaterialStore for NoopGuestMaterialStore {
    fn stage_pair(
        &self,
        _: &GuestMaterialTarget,
        _: &GuestSessionCredentialBytes,
        _: &GuestMaterialTarget,
        _: &[u8],
    ) -> Result<Box<dyn GuestMaterialTransaction>, GuestMaterialError> {
        Ok(Box::new(NoopTransaction { pending: true }))
    }
}

struct NoopTransaction {
    pending: bool,
}

impl GuestMaterialTransaction for NoopTransaction {
    fn commit(&mut self) -> Result<(), GuestMaterialError> {
        Ok(())
    }

    fn mark_committed(&mut self) -> Result<(), GuestMaterialError> {
        Ok(())
    }

    fn mark_audit_committed(&mut self) -> Result<(), GuestMaterialError> {
        Ok(())
    }

    fn finalize(&mut self) {
        self.pending = false;
    }

    fn rollback(&mut self) -> Result<(), GuestMaterialError> {
        self.pending = false;
        Ok(())
    }
}

impl Drop for NoopTransaction {
    fn drop(&mut self) {
        self.pending = false;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PairMember {
    Session,
    Configured,
}

trait PairMutationBackend: Send + Sync {
    fn replace(
        &self,
        parent_fd: &OwnedFd,
        member: PairMember,
        name: &str,
        target: &GuestMaterialTarget,
        bytes: &[u8],
    ) -> Result<(), GuestMaterialError>;
}

struct LivePairMutationBackend;

impl PairMutationBackend for LivePairMutationBackend {
    fn replace(
        &self,
        parent_fd: &OwnedFd,
        _: PairMember,
        name: &str,
        target: &GuestMaterialTarget,
        bytes: &[u8],
    ) -> Result<(), GuestMaterialError> {
        crate::sys::path_safe::atomic_replace_fd_with_owner(
            parent_fd,
            name,
            bytes,
            target.mode,
            Some(target.owner_uid),
            Some(target.owner_gid),
        )
        .map_err(|_| GuestMaterialError::MaterializationFailed)
    }
}

struct PriorMaterial {
    bytes: Option<Zeroizing<Vec<u8>>>,
}

struct FilesystemPairTransaction {
    parent_fd: OwnedFd,
    session_name: String,
    configured_name: String,
    session_target: GuestMaterialTarget,
    configured_target: GuestMaterialTarget,
    prior_session: PriorMaterial,
    prior_configured: PriorMaterial,
    backend: Arc<dyn PairMutationBackend>,
    pending: bool,
}

impl GuestMaterialTransaction for FilesystemPairTransaction {
    fn commit(&mut self) -> Result<(), GuestMaterialError> {
        Ok(())
    }

    fn mark_committed(&mut self) -> Result<(), GuestMaterialError> {
        Ok(())
    }

    fn mark_audit_committed(&mut self) -> Result<(), GuestMaterialError> {
        Ok(())
    }

    fn finalize(&mut self) {
        self.pending = false;
    }

    fn rollback(&mut self) -> Result<(), GuestMaterialError> {
        if !self.pending {
            return Ok(());
        }
        let session = restore_target(
            &self.parent_fd,
            PairMember::Session,
            &self.session_name,
            &self.session_target,
            &self.prior_session,
            self.backend.as_ref(),
        );
        let configured = restore_target(
            &self.parent_fd,
            PairMember::Configured,
            &self.configured_name,
            &self.configured_target,
            &self.prior_configured,
            self.backend.as_ref(),
        );
        if session.is_err() || configured.is_err() {
            return Err(GuestMaterialError::MaterializationFailed);
        }
        self.pending = false;
        Ok(())
    }
}

impl Drop for FilesystemPairTransaction {
    fn drop(&mut self) {
        let _ = self.rollback();
    }
}

struct RecoveryRecord {
    state: u8,
    identity: EnrollmentRecoveryIdentity,
    session_name: String,
    configured_name: String,
    prior_session: Option<[u8; 32]>,
    prior_configured: Option<[u8; 32]>,
}

struct FilesystemEnrollmentTransaction {
    parent_fd: OwnedFd,
    session_name: String,
    configured_name: String,
    session_target: GuestMaterialTarget,
    configured_target: GuestMaterialTarget,
    prior_session: PriorMaterial,
    prior_configured: PriorMaterial,
    new_session: Zeroizing<Vec<u8>>,
    new_configured: Zeroizing<Vec<u8>>,
    recovery: RecoveryRecord,
    backend: Arc<dyn PairMutationBackend>,
    pending: bool,
}

impl GuestMaterialTransaction for FilesystemEnrollmentTransaction {
    fn commit(&mut self) -> Result<(), GuestMaterialError> {
        if !self.pending {
            return Err(GuestMaterialError::HandlerDropped);
        }
        self.backend.replace(
            &self.parent_fd,
            PairMember::Session,
            &self.session_name,
            &self.session_target,
            &self.new_session,
        )?;
        self.backend.replace(
            &self.parent_fd,
            PairMember::Configured,
            &self.configured_name,
            &self.configured_target,
            &self.new_configured,
        )?;
        rustix::fs::fsync(&self.parent_fd).map_err(|_| GuestMaterialError::MaterializationFailed)
    }

    fn mark_committed(&mut self) -> Result<(), GuestMaterialError> {
        if !self.pending {
            return Err(GuestMaterialError::HandlerDropped);
        }
        self.recovery.state = RECOVERY_PAIR_LEDGER_COMMITTED;
        write_recovery_record(&self.parent_fd, &self.recovery)
    }

    fn mark_audit_committed(&mut self) -> Result<(), GuestMaterialError> {
        if !self.pending {
            return Err(GuestMaterialError::HandlerDropped);
        }
        self.recovery.state = RECOVERY_AUDIT_COMMITTED;
        write_recovery_record(&self.parent_fd, &self.recovery)
    }

    fn finalize(&mut self) {
        let _ = cleanup_recovery(&self.parent_fd);
        self.pending = false;
    }

    fn rollback(&mut self) -> Result<(), GuestMaterialError> {
        if !self.pending {
            return Ok(());
        }
        restore_pair(
            &self.parent_fd,
            &self.session_name,
            &self.session_target,
            &self.prior_session,
            &self.configured_name,
            &self.configured_target,
            &self.prior_configured,
            self.backend.as_ref(),
        )?;
        cleanup_recovery(&self.parent_fd)?;
        self.pending = false;
        Ok(())
    }
}

impl Drop for FilesystemEnrollmentTransaction {
    fn drop(&mut self) {
        let _ = self.rollback();
    }
}

impl FilesystemGuestMaterialStore {
    pub fn recover_enrollment_pair(
        &self,
        session_target: &GuestMaterialTarget,
        configured_target: &GuestMaterialTarget,
        expected_configured_digest: [u8; 32],
        lookup: &dyn EnrollmentCommitLookup,
        audit: &dyn EnrollmentSuccessAudit,
    ) -> Result<(), GuestMaterialError> {
        if session_target.path.parent() != configured_target.path.parent() {
            return Err(GuestMaterialError::StorageContractMismatch);
        }
        let parent = session_target
            .path
            .parent()
            .ok_or(GuestMaterialError::StorageContractMismatch)?;
        let parent_fd = crate::sys::path_safe::open_dir_path_safe(parent)
            .map_err(|_| GuestMaterialError::MaterializationFailed)?;
        let record = match read_recovery_record(&parent_fd)? {
            Some(record) => record,
            None => {
                cleanup_recovery(&parent_fd)?;
                return Ok(());
            }
        };
        let session_name = target_name(&session_target.path)?;
        let configured_name = target_name(&configured_target.path)?;
        if record.session_name != session_name
            || record.configured_name != configured_name
            || record.identity.configured_digest != expected_configured_digest
        {
            return Err(GuestMaterialError::StorageContractMismatch);
        }
        if record.state == RECOVERY_AUDIT_COMMITTED {
            if !lookup.enrollment_committed(record.identity)? {
                return Err(GuestMaterialError::StorageContractMismatch);
            }
            audit.ensure_success(record.identity.success_audit)?;
            return cleanup_recovery(&parent_fd);
        }
        let prior_session =
            read_recovery_material(&parent_fd, PRIOR_SESSION, record.prior_session)?;
        let prior_configured =
            read_recovery_material(&parent_fd, PRIOR_CONFIGURED, record.prior_configured)?;
        let new_session = read_required_recovery_material(
            &parent_fd,
            NEW_SESSION,
            record.identity.credential_digest,
        )?;
        let new_configured = read_required_recovery_material(
            &parent_fd,
            NEW_CONFIGURED,
            record.identity.configured_digest,
        )?;
        if lookup.enrollment_committed(record.identity)? {
            self.backend.replace(
                &parent_fd,
                PairMember::Session,
                session_name,
                session_target,
                &new_session,
            )?;
            self.backend.replace(
                &parent_fd,
                PairMember::Configured,
                configured_name,
                configured_target,
                &new_configured,
            )?;
            audit.ensure_success(record.identity.success_audit)?;
            let mut audited = record;
            audited.state = RECOVERY_AUDIT_COMMITTED;
            write_recovery_record(&parent_fd, &audited)?;
        } else {
            if record.state == RECOVERY_AUDIT_COMMITTED {
                return Err(GuestMaterialError::StorageContractMismatch);
            }
            restore_pair(
                &parent_fd,
                session_name,
                session_target,
                &PriorMaterial {
                    bytes: prior_session,
                },
                configured_name,
                configured_target,
                &PriorMaterial {
                    bytes: prior_configured,
                },
                self.backend.as_ref(),
            )?;
        }
        rustix::fs::fsync(&parent_fd).map_err(|_| GuestMaterialError::MaterializationFailed)?;
        cleanup_recovery(&parent_fd)
    }
}

#[allow(clippy::too_many_arguments)]
fn stage_enrollment_pair_with_backend(
    session_target: &GuestMaterialTarget,
    session_bytes: &[u8],
    configured_target: &GuestMaterialTarget,
    configured_bytes: &[u8],
    identity: EnrollmentRecoveryIdentity,
    require_root: bool,
    backend: Arc<dyn PairMutationBackend>,
) -> Result<FilesystemEnrollmentTransaction, GuestMaterialError> {
    identity.success_audit.validate()?;
    if session_target.path.parent() != configured_target.path.parent()
        || (require_root && (session_target.owner_uid != 0 || configured_target.owner_uid != 0))
        || <[u8; 32]>::from(Sha256::digest(session_bytes)) != identity.credential_digest
        || <[u8; 32]>::from(Sha256::digest(configured_bytes)) != identity.configured_digest
        || identity.success_audit.credential_digest != identity.credential_digest
        || identity.success_audit.configured_digest != identity.configured_digest
    {
        return Err(GuestMaterialError::StorageContractMismatch);
    }
    let parent = session_target
        .path
        .parent()
        .ok_or(GuestMaterialError::StorageContractMismatch)?;
    let session_name = target_name(&session_target.path)?.to_owned();
    let configured_name = target_name(&configured_target.path)?.to_owned();
    let parent_fd = crate::sys::path_safe::open_dir_path_safe(parent)
        .map_err(|_| GuestMaterialError::MaterializationFailed)?;
    let parent_stat = rustix::fs::fstat(parent_fd.as_fd())
        .map_err(|_| GuestMaterialError::MaterializationFailed)?;
    validate_parent_metadata(parent_stat.st_uid, parent_stat.st_mode, require_root)?;
    if read_recovery_record(&parent_fd)?.is_some() {
        return Err(GuestMaterialError::ResourceExhausted);
    }
    let prior_session = read_prior(&parent_fd, &session_name, session_target)?;
    let prior_configured = read_prior(&parent_fd, &configured_name, configured_target)?;
    let recovery = RecoveryRecord {
        state: RECOVERY_PREPARED,
        identity,
        session_name: session_name.clone(),
        configured_name: configured_name.clone(),
        prior_session: prior_digest(&prior_session),
        prior_configured: prior_digest(&prior_configured),
    };
    stage_recovery_material(
        &parent_fd,
        PRIOR_SESSION,
        prior_session.bytes.as_ref().map(|bytes| bytes.as_slice()),
    )?;
    stage_recovery_material(
        &parent_fd,
        PRIOR_CONFIGURED,
        prior_configured
            .bytes
            .as_ref()
            .map(|bytes| bytes.as_slice()),
    )?;
    stage_recovery_material(&parent_fd, NEW_SESSION, Some(session_bytes))?;
    stage_recovery_material(&parent_fd, NEW_CONFIGURED, Some(configured_bytes))?;
    if let Err(error) = write_recovery_record(&parent_fd, &recovery) {
        let _ = cleanup_recovery(&parent_fd);
        return Err(error);
    }
    Ok(FilesystemEnrollmentTransaction {
        parent_fd,
        session_name,
        configured_name,
        session_target: session_target.clone(),
        configured_target: configured_target.clone(),
        prior_session,
        prior_configured,
        new_session: Zeroizing::new(session_bytes.to_vec()),
        new_configured: Zeroizing::new(configured_bytes.to_vec()),
        recovery,
        backend,
        pending: true,
    })
}

fn stage_pair_with_backend(
    session_target: &GuestMaterialTarget,
    session_bytes: &[u8],
    configured_target: &GuestMaterialTarget,
    configured_bytes: &[u8],
    require_root: bool,
    backend: Arc<dyn PairMutationBackend>,
) -> Result<FilesystemPairTransaction, GuestMaterialError> {
    if session_target.path.parent() != configured_target.path.parent()
        || (require_root && (session_target.owner_uid != 0 || configured_target.owner_uid != 0))
    {
        return Err(GuestMaterialError::StorageContractMismatch);
    }
    let parent = session_target
        .path
        .parent()
        .ok_or(GuestMaterialError::StorageContractMismatch)?;
    let session_name = target_name(&session_target.path)?.to_owned();
    let configured_name = target_name(&configured_target.path)?.to_owned();
    let parent_fd = crate::sys::path_safe::open_dir_path_safe(parent)
        .map_err(|_| GuestMaterialError::MaterializationFailed)?;
    let parent_stat = rustix::fs::fstat(parent_fd.as_fd())
        .map_err(|_| GuestMaterialError::MaterializationFailed)?;
    validate_parent_metadata(parent_stat.st_uid, parent_stat.st_mode, require_root)?;
    let prior_session = read_prior(&parent_fd, &session_name, session_target)?;
    let prior_configured = read_prior(&parent_fd, &configured_name, configured_target)?;
    let mut transaction = FilesystemPairTransaction {
        parent_fd,
        session_name,
        configured_name,
        session_target: session_target.clone(),
        configured_target: configured_target.clone(),
        prior_session,
        prior_configured,
        backend,
        pending: true,
    };
    if transaction
        .backend
        .replace(
            &transaction.parent_fd,
            PairMember::Session,
            &transaction.session_name,
            &transaction.session_target,
            session_bytes,
        )
        .is_err()
    {
        transaction.rollback()?;
        return Err(GuestMaterialError::MaterializationFailed);
    }
    if transaction
        .backend
        .replace(
            &transaction.parent_fd,
            PairMember::Configured,
            &transaction.configured_name,
            &transaction.configured_target,
            configured_bytes,
        )
        .is_err()
    {
        transaction.rollback()?;
        return Err(GuestMaterialError::MaterializationFailed);
    }
    Ok(transaction)
}

fn validate_parent_metadata(
    uid: u32,
    mode: u32,
    require_root: bool,
) -> Result<(), GuestMaterialError> {
    if (require_root && uid != 0) || mode & 0o022 != 0 {
        return Err(GuestMaterialError::StorageContractMismatch);
    }
    Ok(())
}

fn target_name(path: &Path) -> Result<&str, GuestMaterialError> {
    path.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty() && !name.contains('/'))
        .ok_or(GuestMaterialError::StorageContractMismatch)
}

fn read_prior(
    parent_fd: &OwnedFd,
    name: &str,
    target: &GuestMaterialTarget,
) -> Result<PriorMaterial, GuestMaterialError> {
    let fd = match rustix::fs::openat(
        parent_fd.as_fd(),
        name,
        rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::CLOEXEC | rustix::fs::OFlags::NOFOLLOW,
        rustix::fs::Mode::empty(),
    ) {
        Ok(fd) => fd,
        Err(rustix::io::Errno::NOENT) => return Ok(PriorMaterial { bytes: None }),
        Err(_) => return Err(GuestMaterialError::MaterializationFailed),
    };
    let stat =
        rustix::fs::fstat(fd.as_fd()).map_err(|_| GuestMaterialError::MaterializationFailed)?;
    if rustix::fs::FileType::from_raw_mode(stat.st_mode) != rustix::fs::FileType::RegularFile
        || stat.st_size < 0
        || stat.st_uid != target.owner_uid
        || stat.st_gid != target.owner_gid
        || stat.st_mode & 0o777 != target.mode
        || usize::try_from(stat.st_size)
            .ok()
            .is_none_or(|size| size > MAX_MATERIAL_BYTES)
    {
        return Err(GuestMaterialError::MaterializationFailed);
    }
    let mut bytes = Zeroizing::new(Vec::with_capacity(stat.st_size as usize));
    File::from(fd)
        .take((MAX_MATERIAL_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| GuestMaterialError::MaterializationFailed)?;
    if bytes.len() > MAX_MATERIAL_BYTES {
        return Err(GuestMaterialError::MaterializationFailed);
    }
    Ok(PriorMaterial { bytes: Some(bytes) })
}

fn restore_target(
    parent_fd: &OwnedFd,
    member: PairMember,
    name: &str,
    target: &GuestMaterialTarget,
    prior: &PriorMaterial,
    backend: &dyn PairMutationBackend,
) -> Result<(), GuestMaterialError> {
    match prior.bytes.as_ref() {
        Some(bytes) => backend.replace(parent_fd, member, name, target, bytes),
        None => match rustix::fs::unlinkat(parent_fd.as_fd(), name, rustix::fs::AtFlags::empty()) {
            Ok(()) | Err(rustix::io::Errno::NOENT) => Ok(()),
            Err(_) => Err(GuestMaterialError::MaterializationFailed),
        },
    }
}

#[allow(clippy::too_many_arguments)]
fn restore_pair(
    parent_fd: &OwnedFd,
    session_name: &str,
    session_target: &GuestMaterialTarget,
    prior_session: &PriorMaterial,
    configured_name: &str,
    configured_target: &GuestMaterialTarget,
    prior_configured: &PriorMaterial,
    backend: &dyn PairMutationBackend,
) -> Result<(), GuestMaterialError> {
    let session = restore_target(
        parent_fd,
        PairMember::Session,
        session_name,
        session_target,
        prior_session,
        backend,
    );
    let configured = restore_target(
        parent_fd,
        PairMember::Configured,
        configured_name,
        configured_target,
        prior_configured,
        backend,
    );
    if session.is_err() || configured.is_err() {
        return Err(GuestMaterialError::MaterializationFailed);
    }
    rustix::fs::fsync(parent_fd).map_err(|_| GuestMaterialError::MaterializationFailed)
}

fn prior_digest(prior: &PriorMaterial) -> Option<[u8; 32]> {
    prior
        .bytes
        .as_ref()
        .map(|bytes| Sha256::digest(bytes).into())
}

fn stage_recovery_material(
    parent_fd: &OwnedFd,
    name: &str,
    bytes: Option<&[u8]>,
) -> Result<(), GuestMaterialError> {
    match bytes {
        Some(bytes) => crate::sys::path_safe::atomic_replace_fd_with_owner(
            parent_fd, name, bytes, 0o600, None, None,
        )
        .map_err(|_| GuestMaterialError::MaterializationFailed),
        None => unlink_recovery_file(parent_fd, name),
    }
}

fn write_recovery_record(
    parent_fd: &OwnedFd,
    record: &RecoveryRecord,
) -> Result<(), GuestMaterialError> {
    let encoded = encode_recovery_record(record)?;
    crate::sys::path_safe::atomic_replace_fd_with_owner(
        parent_fd,
        RECOVERY_JOURNAL,
        &encoded,
        0o600,
        None,
        None,
    )
    .map_err(|_| GuestMaterialError::MaterializationFailed)?;
    rustix::fs::fsync(parent_fd).map_err(|_| GuestMaterialError::MaterializationFailed)
}

fn encode_recovery_record(record: &RecoveryRecord) -> Result<Vec<u8>, GuestMaterialError> {
    record.identity.success_audit.validate()?;
    if !matches!(
        record.state,
        RECOVERY_PREPARED | RECOVERY_PAIR_LEDGER_COMMITTED | RECOVERY_AUDIT_COMMITTED
    ) || record.identity.replay_digest == [0; 32]
        || record.identity.credential_digest == [0; 32]
        || record.identity.configured_digest == [0; 32]
        || record.identity.success_audit.credential_digest != record.identity.credential_digest
        || record.identity.success_audit.configured_digest != record.identity.configured_digest
    {
        return Err(GuestMaterialError::StorageContractMismatch);
    }
    let session_len = u16::try_from(record.session_name.len())
        .map_err(|_| GuestMaterialError::StorageContractMismatch)?;
    let configured_len = u16::try_from(record.configured_name.len())
        .map_err(|_| GuestMaterialError::StorageContractMismatch)?;
    let mut encoded = Vec::with_capacity(256);
    encoded.extend_from_slice(RECOVERY_MAGIC);
    encoded.push(record.state);
    encoded.extend_from_slice(&record.identity.replay_digest);
    encoded.extend_from_slice(&record.identity.credential_digest);
    encoded.extend_from_slice(&record.identity.configured_digest);
    encoded.extend_from_slice(
        &record
            .identity
            .success_audit
            .session_generation
            .to_be_bytes(),
    );
    encoded.extend_from_slice(&record.identity.success_audit.request_digest);
    encoded.extend_from_slice(&record.identity.success_audit.credential_digest);
    encoded.extend_from_slice(&record.identity.success_audit.configured_digest);
    encoded.push(record.identity.success_audit.outcome);
    encoded.extend_from_slice(&record.identity.success_audit.dedup_key);
    encoded.extend_from_slice(&session_len.to_be_bytes());
    encoded.extend_from_slice(record.session_name.as_bytes());
    encoded.extend_from_slice(&configured_len.to_be_bytes());
    encoded.extend_from_slice(record.configured_name.as_bytes());
    encode_optional_digest(&mut encoded, record.prior_session);
    encode_optional_digest(&mut encoded, record.prior_configured);
    Ok(encoded)
}

fn encode_optional_digest(encoded: &mut Vec<u8>, digest: Option<[u8; 32]>) {
    encoded.push(u8::from(digest.is_some()));
    encoded.extend_from_slice(&digest.unwrap_or([0; 32]));
}

fn read_recovery_record(parent_fd: &OwnedFd) -> Result<Option<RecoveryRecord>, GuestMaterialError> {
    let encoded = match read_internal_file(parent_fd, RECOVERY_JOURNAL, 1024)? {
        Some(encoded) => encoded,
        None => return Ok(None),
    };
    let mut reader = RecoveryReader::new(&encoded);
    if reader.take(RECOVERY_MAGIC.len())? != RECOVERY_MAGIC {
        return Err(GuestMaterialError::StorageContractMismatch);
    }
    let state = reader.byte()?;
    let identity = EnrollmentRecoveryIdentity {
        replay_digest: reader.array()?,
        credential_digest: reader.array()?,
        configured_digest: reader.array()?,
        success_audit: EnrollmentSuccessAuditIdentity {
            session_generation: reader.u64()?,
            request_digest: reader.array()?,
            credential_digest: reader.array()?,
            configured_digest: reader.array()?,
            outcome: reader.byte()?,
            dedup_key: reader.array()?,
        },
    };
    let session_name = reader.string()?;
    let configured_name = reader.string()?;
    let prior_session = reader.optional_digest()?;
    let prior_configured = reader.optional_digest()?;
    if !reader.done()
        || !matches!(
            state,
            RECOVERY_PREPARED | RECOVERY_PAIR_LEDGER_COMMITTED | RECOVERY_AUDIT_COMMITTED
        )
        || identity.replay_digest == [0; 32]
        || identity.credential_digest == [0; 32]
        || identity.configured_digest == [0; 32]
        || identity.success_audit.validate().is_err()
        || identity.success_audit.credential_digest != identity.credential_digest
        || identity.success_audit.configured_digest != identity.configured_digest
        || target_name(Path::new(&session_name))? != session_name
        || target_name(Path::new(&configured_name))? != configured_name
    {
        return Err(GuestMaterialError::StorageContractMismatch);
    }
    Ok(Some(RecoveryRecord {
        state,
        identity,
        session_name,
        configured_name,
        prior_session,
        prior_configured,
    }))
}

fn read_recovery_material(
    parent_fd: &OwnedFd,
    name: &str,
    expected: Option<[u8; 32]>,
) -> Result<Option<Zeroizing<Vec<u8>>>, GuestMaterialError> {
    match (
        read_internal_file(parent_fd, name, MAX_MATERIAL_BYTES)?,
        expected,
    ) {
        (None, None) => Ok(None),
        (Some(bytes), Some(digest)) if <[u8; 32]>::from(Sha256::digest(&bytes)) == digest => {
            Ok(Some(bytes))
        }
        _ => Err(GuestMaterialError::StorageContractMismatch),
    }
}

fn read_required_recovery_material(
    parent_fd: &OwnedFd,
    name: &str,
    expected: [u8; 32],
) -> Result<Zeroizing<Vec<u8>>, GuestMaterialError> {
    read_recovery_material(parent_fd, name, Some(expected))?
        .ok_or(GuestMaterialError::StorageContractMismatch)
}

fn read_internal_file(
    parent_fd: &OwnedFd,
    name: &str,
    max_bytes: usize,
) -> Result<Option<Zeroizing<Vec<u8>>>, GuestMaterialError> {
    let fd = match rustix::fs::openat(
        parent_fd.as_fd(),
        name,
        rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::CLOEXEC | rustix::fs::OFlags::NOFOLLOW,
        rustix::fs::Mode::empty(),
    ) {
        Ok(fd) => fd,
        Err(rustix::io::Errno::NOENT) => return Ok(None),
        Err(_) => return Err(GuestMaterialError::MaterializationFailed),
    };
    let stat =
        rustix::fs::fstat(fd.as_fd()).map_err(|_| GuestMaterialError::MaterializationFailed)?;
    let parent_stat = rustix::fs::fstat(parent_fd.as_fd())
        .map_err(|_| GuestMaterialError::MaterializationFailed)?;
    if rustix::fs::FileType::from_raw_mode(stat.st_mode) != rustix::fs::FileType::RegularFile
        || stat.st_size < 0
        || stat.st_uid != parent_stat.st_uid
        || stat.st_mode & 0o777 != 0o600
        || usize::try_from(stat.st_size)
            .ok()
            .is_none_or(|size| size > max_bytes)
    {
        return Err(GuestMaterialError::StorageContractMismatch);
    }
    let mut bytes = Zeroizing::new(Vec::with_capacity(stat.st_size as usize));
    File::from(fd)
        .take((max_bytes + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| GuestMaterialError::MaterializationFailed)?;
    if bytes.len() > max_bytes {
        return Err(GuestMaterialError::StorageContractMismatch);
    }
    Ok(Some(bytes))
}

fn cleanup_recovery(parent_fd: &OwnedFd) -> Result<(), GuestMaterialError> {
    unlink_recovery_file(parent_fd, RECOVERY_JOURNAL)?;
    rustix::fs::fsync(parent_fd).map_err(|_| GuestMaterialError::MaterializationFailed)?;
    for name in [PRIOR_SESSION, PRIOR_CONFIGURED, NEW_SESSION, NEW_CONFIGURED] {
        unlink_recovery_file(parent_fd, name)?;
        rustix::fs::fsync(parent_fd).map_err(|_| GuestMaterialError::MaterializationFailed)?;
    }
    Ok(())
}

fn unlink_recovery_file(parent_fd: &OwnedFd, name: &str) -> Result<(), GuestMaterialError> {
    match rustix::fs::unlinkat(parent_fd.as_fd(), name, rustix::fs::AtFlags::empty()) {
        Ok(()) | Err(rustix::io::Errno::NOENT) => Ok(()),
        Err(_) => Err(GuestMaterialError::MaterializationFailed),
    }
}

struct RecoveryReader<'a> {
    encoded: &'a [u8],
    offset: usize,
}

impl<'a> RecoveryReader<'a> {
    const fn new(encoded: &'a [u8]) -> Self {
        Self { encoded, offset: 0 }
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8], GuestMaterialError> {
        let end = self
            .offset
            .checked_add(length)
            .filter(|end| *end <= self.encoded.len())
            .ok_or(GuestMaterialError::StorageContractMismatch)?;
        let bytes = &self.encoded[self.offset..end];
        self.offset = end;
        Ok(bytes)
    }

    fn byte(&mut self) -> Result<u8, GuestMaterialError> {
        self.take(1)?
            .first()
            .copied()
            .ok_or(GuestMaterialError::StorageContractMismatch)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], GuestMaterialError> {
        self.take(N)?
            .try_into()
            .map_err(|_| GuestMaterialError::StorageContractMismatch)
    }

    fn u64(&mut self) -> Result<u64, GuestMaterialError> {
        Ok(u64::from_be_bytes(self.array()?))
    }

    fn string(&mut self) -> Result<String, GuestMaterialError> {
        let length = usize::from(u16::from_be_bytes(self.array()?));
        if length == 0 || length > 255 {
            return Err(GuestMaterialError::StorageContractMismatch);
        }
        std::str::from_utf8(self.take(length)?)
            .map(str::to_owned)
            .map_err(|_| GuestMaterialError::StorageContractMismatch)
    }

    fn optional_digest(&mut self) -> Result<Option<[u8; 32]>, GuestMaterialError> {
        let present = self.byte()?;
        let digest = self.array()?;
        match (present, digest) {
            (0, digest) if digest == [0; 32] => Ok(None),
            (1, digest) if digest != [0; 32] => Ok(Some(digest)),
            _ => Err(GuestMaterialError::StorageContractMismatch),
        }
    }

    fn done(&self) -> bool {
        self.offset == self.encoded.len()
    }
}

#[cfg(test)]
mod tests {
    use std::{
        os::unix::fs::{MetadataExt, PermissionsExt},
        sync::{
            Arc, Mutex,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
    };

    use d2b_contracts::v2_component_session::{GuestIdentityBindingV1, GuestSessionCredentialV1};

    use super::*;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum FailurePoint {
        FirstRename,
        FirstFsync,
        SecondRename,
        SecondFsync,
    }

    struct FaultBackend {
        point: FailurePoint,
        fired: AtomicBool,
        live: LivePairMutationBackend,
    }

    struct RecoveryLookup {
        expected: EnrollmentRecoveryIdentity,
        committed: bool,
    }

    impl EnrollmentCommitLookup for RecoveryLookup {
        fn enrollment_committed(
            &self,
            identity: EnrollmentRecoveryIdentity,
        ) -> Result<bool, GuestMaterialError> {
            assert_eq!(identity, self.expected);
            Ok(self.committed)
        }
    }

    #[derive(Default)]
    struct RecoveryAudit {
        keys: Mutex<Vec<[u8; 32]>>,
        appends: AtomicUsize,
    }

    impl EnrollmentSuccessAudit for RecoveryAudit {
        fn ensure_success(
            &self,
            identity: EnrollmentSuccessAuditIdentity,
        ) -> Result<(), GuestMaterialError> {
            identity.validate()?;
            let mut keys = self.keys.lock().unwrap();
            if !keys.contains(&identity.dedup_key) {
                keys.push(identity.dedup_key);
                self.appends.fetch_add(1, Ordering::SeqCst);
            }
            Ok(())
        }
    }

    impl PairMutationBackend for FaultBackend {
        fn replace(
            &self,
            parent_fd: &OwnedFd,
            member: PairMember,
            name: &str,
            target: &GuestMaterialTarget,
            bytes: &[u8],
        ) -> Result<(), GuestMaterialError> {
            let is_member = matches!(
                (self.point, member),
                (
                    FailurePoint::FirstRename | FailurePoint::FirstFsync,
                    PairMember::Session
                ) | (
                    FailurePoint::SecondRename | FailurePoint::SecondFsync,
                    PairMember::Configured
                )
            );
            let fail_before = matches!(
                self.point,
                FailurePoint::FirstRename | FailurePoint::SecondRename
            );
            if is_member && fail_before && !self.fired.swap(true, Ordering::SeqCst) {
                return Err(GuestMaterialError::MaterializationFailed);
            }
            self.live.replace(parent_fd, member, name, target, bytes)?;
            if is_member && !fail_before && !self.fired.swap(true, Ordering::SeqCst) {
                return Err(GuestMaterialError::MaterializationFailed);
            }
            Ok(())
        }
    }

    fn encoded_session() -> GuestSessionCredentialBytes {
        GuestSessionCredentialV1::new(
            1,
            [1; 32],
            [2; 32],
            GuestIdentityBindingV1::Enrolled {
                guest_identity_digest: [3; 32],
                guest_static_public_key: [4; 32],
            },
            None,
        )
        .unwrap()
        .encode()
        .unwrap()
    }

    #[test]
    fn parent_must_be_root_owned_and_not_group_or_world_writable() {
        assert_eq!(
            validate_parent_metadata(1, 0o700, true),
            Err(GuestMaterialError::StorageContractMismatch)
        );
        assert_eq!(
            validate_parent_metadata(0, 0o720, true),
            Err(GuestMaterialError::StorageContractMismatch)
        );
        assert_eq!(
            validate_parent_metadata(0, 0o702, true),
            Err(GuestMaterialError::StorageContractMismatch)
        );
        validate_parent_metadata(0, 0o750, true).unwrap();
    }

    #[test]
    fn every_rename_and_fsync_failure_restores_both_members() {
        for point in [
            FailurePoint::FirstRename,
            FailurePoint::FirstFsync,
            FailurePoint::SecondRename,
            FailurePoint::SecondFsync,
        ] {
            let root = crate::test_tempdir("guest-material-store");
            std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
            let uid = rustix::process::getuid().as_raw();
            let gid = rustix::process::getgid().as_raw();
            let session_path = root.path().join("session");
            let configured_path = root.path().join("configured");
            std::fs::write(&session_path, b"prior-session").unwrap();
            std::fs::write(&configured_path, b"prior-configured").unwrap();
            std::fs::set_permissions(&session_path, std::fs::Permissions::from_mode(0o400))
                .unwrap();
            std::fs::set_permissions(&configured_path, std::fs::Permissions::from_mode(0o400))
                .unwrap();
            assert_eq!(std::fs::metadata(root.path()).unwrap().uid(), uid);
            let target = |storage_ref: &str, path| GuestMaterialTarget {
                storage_ref: storage_ref.to_owned(),
                path,
                owner_uid: uid,
                owner_gid: gid,
                mode: 0o400,
            };
            let session_target = target("session", session_path.clone());
            let configured_target = target("configured", configured_path.clone());
            let backend: Arc<dyn PairMutationBackend> = Arc::new(FaultBackend {
                point,
                fired: AtomicBool::new(false),
                live: LivePairMutationBackend,
            });
            assert!(
                stage_pair_with_backend(
                    &session_target,
                    encoded_session().as_slice(),
                    &configured_target,
                    b"new-configured",
                    false,
                    backend,
                )
                .is_err()
            );
            assert_eq!(std::fs::read(session_path).unwrap(), b"prior-session");
            assert_eq!(std::fs::read(configured_path).unwrap(), b"prior-configured");
        }
    }

    #[derive(Clone, Copy, Debug)]
    enum CrashBoundary {
        Prepared,
        FirstReplacement,
        PairReplaced,
        LedgerCommitted,
        PairLedgerMarked,
        AuditDurable,
        AuditMarked,
    }

    #[test]
    fn enrollment_recovery_resolves_every_crash_boundary_before_serving() {
        for boundary in [
            CrashBoundary::Prepared,
            CrashBoundary::FirstReplacement,
            CrashBoundary::PairReplaced,
            CrashBoundary::LedgerCommitted,
            CrashBoundary::PairLedgerMarked,
            CrashBoundary::AuditDurable,
            CrashBoundary::AuditMarked,
        ] {
            let root = crate::test_tempdir("guest-enrollment-recovery");
            std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
            let uid = rustix::process::getuid().as_raw();
            let gid = rustix::process::getgid().as_raw();
            let session_path = root.path().join("session");
            let configured_path = root.path().join("configured");
            std::fs::write(&session_path, b"prior-session").unwrap();
            std::fs::write(&configured_path, b"prior-configured").unwrap();
            std::fs::set_permissions(&session_path, std::fs::Permissions::from_mode(0o400))
                .unwrap();
            std::fs::set_permissions(&configured_path, std::fs::Permissions::from_mode(0o400))
                .unwrap();
            let target = |storage_ref: &str, path| GuestMaterialTarget {
                storage_ref: storage_ref.to_owned(),
                path,
                owner_uid: uid,
                owner_gid: gid,
                mode: 0o400,
            };
            let session_target = target("session", session_path.clone());
            let configured_target = target("configured", configured_path.clone());
            let new_session = encoded_session();
            let new_configured = b"new-configured";
            let credential_digest = Sha256::digest(new_session.as_slice()).into();
            let configured_digest = Sha256::digest(new_configured).into();
            let identity = EnrollmentRecoveryIdentity {
                replay_digest: [9; 32],
                credential_digest,
                configured_digest,
                success_audit: EnrollmentSuccessAuditIdentity::new(
                    1,
                    [8; 32],
                    credential_digest,
                    configured_digest,
                )
                .unwrap(),
            };
            let backend: Arc<dyn PairMutationBackend> = Arc::new(LivePairMutationBackend);
            let mut transaction = stage_enrollment_pair_with_backend(
                &session_target,
                new_session.as_slice(),
                &configured_target,
                new_configured,
                identity,
                false,
                Arc::clone(&backend),
            )
            .unwrap();
            assert_eq!(std::fs::read(&session_path).unwrap(), b"prior-session");
            assert_eq!(
                std::fs::read(&configured_path).unwrap(),
                b"prior-configured"
            );
            let prepared = read_recovery_record(&transaction.parent_fd)
                .unwrap()
                .unwrap();
            assert_eq!(prepared.state, RECOVERY_PREPARED);
            assert_eq!(prepared.identity, identity);

            let committed = matches!(
                boundary,
                CrashBoundary::LedgerCommitted
                    | CrashBoundary::PairLedgerMarked
                    | CrashBoundary::AuditDurable
                    | CrashBoundary::AuditMarked
            );
            let audit = RecoveryAudit::default();
            match boundary {
                CrashBoundary::Prepared => {}
                CrashBoundary::FirstReplacement => {
                    transaction
                        .backend
                        .replace(
                            &transaction.parent_fd,
                            PairMember::Session,
                            &transaction.session_name,
                            &transaction.session_target,
                            &transaction.new_session,
                        )
                        .unwrap();
                }
                CrashBoundary::PairReplaced | CrashBoundary::LedgerCommitted => {
                    transaction.commit().unwrap();
                }
                CrashBoundary::PairLedgerMarked => {
                    transaction.commit().unwrap();
                    transaction.mark_committed().unwrap();
                }
                CrashBoundary::AuditDurable => {
                    transaction.commit().unwrap();
                    transaction.mark_committed().unwrap();
                    audit.ensure_success(identity.success_audit).unwrap();
                }
                CrashBoundary::AuditMarked => {
                    transaction.commit().unwrap();
                    transaction.mark_committed().unwrap();
                    audit.ensure_success(identity.success_audit).unwrap();
                    transaction.mark_audit_committed().unwrap();
                }
            }
            std::mem::forget(transaction);

            let store = FilesystemGuestMaterialStore {
                backend,
                require_root: false,
            };
            store
                .recover_enrollment_pair(
                    &session_target,
                    &configured_target,
                    identity.configured_digest,
                    &RecoveryLookup {
                        expected: identity,
                        committed,
                    },
                    &audit,
                )
                .unwrap();
            if committed {
                assert_eq!(
                    std::fs::read(&session_path).unwrap(),
                    new_session.as_slice()
                );
                assert_eq!(std::fs::read(&configured_path).unwrap(), new_configured);
            } else {
                assert_eq!(std::fs::read(&session_path).unwrap(), b"prior-session");
                assert_eq!(
                    std::fs::read(&configured_path).unwrap(),
                    b"prior-configured"
                );
            }
            assert_eq!(audit.appends.load(Ordering::SeqCst), usize::from(committed));
            store
                .recover_enrollment_pair(
                    &session_target,
                    &configured_target,
                    identity.configured_digest,
                    &RecoveryLookup {
                        expected: identity,
                        committed,
                    },
                    &audit,
                )
                .unwrap();
            assert_eq!(audit.appends.load(Ordering::SeqCst), usize::from(committed));
            assert!(!root.path().join(RECOVERY_JOURNAL).exists());
        }
    }

    #[test]
    fn terminal_recovery_needs_no_sidecars_and_reaps_each_cleanup_crash_state() {
        let root = crate::test_tempdir("guest-enrollment-terminal-cleanup");
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let uid = rustix::process::getuid().as_raw();
        let gid = rustix::process::getgid().as_raw();
        let session_target = GuestMaterialTarget {
            storage_ref: "session".to_owned(),
            path: root.path().join("session"),
            owner_uid: uid,
            owner_gid: gid,
            mode: 0o400,
        };
        let configured_target = GuestMaterialTarget {
            storage_ref: "configured".to_owned(),
            path: root.path().join("configured"),
            owner_uid: uid,
            owner_gid: gid,
            mode: 0o400,
        };
        let credential_digest = [2; 32];
        let configured_digest = [3; 32];
        let identity = EnrollmentRecoveryIdentity {
            replay_digest: [1; 32],
            credential_digest,
            configured_digest,
            success_audit: EnrollmentSuccessAuditIdentity::new(
                7,
                [4; 32],
                credential_digest,
                configured_digest,
            )
            .unwrap(),
        };
        let parent_fd = crate::sys::path_safe::open_dir_path_safe(root.path()).unwrap();
        write_recovery_record(
            &parent_fd,
            &RecoveryRecord {
                state: RECOVERY_AUDIT_COMMITTED,
                identity,
                session_name: "session".to_owned(),
                configured_name: "configured".to_owned(),
                prior_session: Some([5; 32]),
                prior_configured: Some([6; 32]),
            },
        )
        .unwrap();
        let audit = RecoveryAudit::default();
        let lookup = RecoveryLookup {
            expected: identity,
            committed: true,
        };
        let store = FilesystemGuestMaterialStore {
            backend: Arc::new(LivePairMutationBackend),
            require_root: false,
        };
        store
            .recover_enrollment_pair(
                &session_target,
                &configured_target,
                configured_digest,
                &lookup,
                &audit,
            )
            .unwrap();
        assert_eq!(audit.appends.load(Ordering::SeqCst), 1);

        let sidecars = [PRIOR_SESSION, PRIOR_CONFIGURED, NEW_SESSION, NEW_CONFIGURED];
        for remaining in 0..=sidecars.len() {
            for name in sidecars {
                let _ = unlink_recovery_file(&parent_fd, name);
            }
            for name in sidecars.iter().take(remaining) {
                stage_recovery_material(&parent_fd, name, Some(b"orphan")).unwrap();
            }
            store
                .recover_enrollment_pair(
                    &session_target,
                    &configured_target,
                    configured_digest,
                    &lookup,
                    &audit,
                )
                .unwrap();
            assert!(sidecars.iter().all(|name| !root.path().join(name).exists()));
        }
        assert_eq!(audit.appends.load(Ordering::SeqCst), 1);
    }
}
