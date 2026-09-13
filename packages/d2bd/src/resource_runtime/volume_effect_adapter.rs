//! Production anchored-fd Volume effect adapter.
//!
//! The adapter is deliberately downstream of the pure controller. A trusted
//! root resolver supplies an already anchored directory descriptor; this
//! module never accepts a caller path. All mutations are single-entry,
//! marker-checked, OFD-locked, and fd-relative.

use std::{
    collections::BTreeSet,
    fs::File,
    future::Future,
    io::{Read, Write},
    mem::MaybeUninit,
    os::fd::{AsRawFd, OwnedFd},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use nix::{
    fcntl::{FcntlArg, fcntl},
    libc,
    unistd::{Gid, Uid, fchown},
};
use rustix::{
    fs::{
        AtFlags, FileType, Mode, OFlags, RawDir, ResolveFlags, XattrFlags, fchmod, fstat, fsync,
        mkdirat, openat2, renameat, symlinkat, unlinkat,
    },
    io::fcntl_dupfd_cloexec,
};

use d2b_contracts_resource::v3::{
    ResourceRef, ResourceUid, SchemaFingerprint, SchemaVersion, VolumeStateSchemaId,
    execution_policy::BoundedToken,
    volume::{EntryType, RepairPolicy, SourceKind},
};

use d2b_provider_volume_local::{
    ContentFile, ContentMaterializationEvidence, ContentProjection, DriftClass, EntryRequest,
    MarkerState, NetworkConfigContentProjection, NetworkConfigMaterializationEvidence,
    ObservedContentFile, ObservedEntry, OwnerProof, QuotaCapability, StoreViewMarkerEvidence,
    VolumeLayoutEffectPort, VolumeLocalError, VolumeRootHandle, VolumeSourceEffectPort,
    atomic::{AtomicFilesystem, AtomicWriteError, replace_bytes},
    lock::{
        LockError, LockGuard, LockId, LockSet, LockSpec, LockTransferPolicy, OfdLockBackend,
        OfdLockHandle,
    },
    marker::{
        MarkerBinding, MarkerDisposition, MarkerError, MarkerStore, VolumeRootIdentity,
        provision_marker, verify_marker,
    },
};

const MARKER_NAME: &str = ".d2b-volume-marker";
const LOCK_NAME: &str = ".d2b-volume.lock";
const MARKER_SCHEMA_ID: &str = "volume-local.d2bus.org/controller/volume-root";
const MARKER_SCHEMA_DIGEST: &str =
    "sha256:0000000000000000000000000000000000000000000000000000000000000000";
const MARKER_MODE: u32 = 0o600;
const LOCK_MODE: u32 = 0o600;
const MAX_MARKER_BYTES: usize = 64 * 1024;

/// A root descriptor resolved by a trusted broker/core boundary.
pub struct ResolvedVolumeRoot {
    fd: OwnedFd,
    marker_root_fd: Option<OwnedFd>,
    volume_uid: ResourceUid,
    marker_name: String,
    lock_name: String,
    identity: VolumeRootIdentity,
    marker_binding: MarkerBinding,
    marker_owner_uid: u32,
    marker_group_gid: u32,
    quota: QuotaCapability,
    preexisting_state: bool,
}

impl ResolvedVolumeRoot {
    /// Bind an anchored directory descriptor to one Volume identity.
    pub fn new(fd: OwnedFd, volume_uid: ResourceUid) -> Result<Self, VolumeLocalError> {
        let identity = root_identity(&fd)?;
        let stat = fstat(&fd).map_err(|_| VolumeLocalError::EffectFailed)?;
        let schema_id = VolumeStateSchemaId::parse(MARKER_SCHEMA_ID)
            .map_err(|_| VolumeLocalError::InvalidSpec)?;
        let schema_version = SchemaVersion::new(1, 0).map_err(|_| VolumeLocalError::InvalidSpec)?;
        let schema_digest = SchemaFingerprint::parse(MARKER_SCHEMA_DIGEST)
            .map_err(|_| VolumeLocalError::InvalidSpec)?;
        let marker_binding = MarkerBinding::new(
            volume_uid.clone(),
            identity,
            schema_id,
            schema_version,
            schema_digest,
        );
        Ok(Self {
            fd,
            marker_root_fd: None,
            volume_uid,
            marker_name: MARKER_NAME.to_owned(),
            lock_name: LOCK_NAME.to_owned(),
            identity,
            marker_binding,
            marker_owner_uid: stat.st_uid,
            marker_group_gid: stat.st_gid,
            quota: QuotaCapability::Enforceable,
            preexisting_state: false,
        })
    }

    /// Override the trusted quota capability observation.
    pub const fn with_quota(mut self, quota: QuotaCapability) -> Self {
        self.quota = quota;
        self
    }

    /// Admit a broker-materialized root whose contents predate this
    /// Volume-local reconcile.
    pub const fn with_preexisting_state(mut self) -> Self {
        self.preexisting_state = true;
        self
    }

    /// Conditionally mark the opened root as already materialized.
    pub const fn with_preexisting_state_if(mut self, preexisting: bool) -> Self {
        self.preexisting_state = preexisting;
        self
    }

    /// Move marker storage to an already anchored broker-owned marker root.
    pub fn with_marker_root(mut self, marker_root_fd: OwnedFd) -> Result<Self, VolumeLocalError> {
        validate_component(self.volume_uid.as_str())?;
        let marker_root_stat =
            fstat(&marker_root_fd).map_err(|_| VolumeLocalError::EffectFailed)?;
        self.marker_root_fd = Some(marker_root_fd);
        self.marker_name = self.volume_uid.as_str().to_owned();
        self.marker_owner_uid = marker_root_stat.st_uid;
        self.marker_group_gid = marker_root_stat.st_gid;
        Ok(self)
    }
}

impl core::fmt::Debug for ResolvedVolumeRoot {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("ResolvedVolumeRoot(<redacted>)")
    }
}

/// Trusted source and principal resolution used by the production adapter.
///
/// Implementations are expected to be backed by broker-resolved bundle
/// policy. They return descriptors and numeric identities only inside this
/// adapter; neither reaches a Provider Resource or status projection.
pub trait VolumeRootResolver: Send + Sync {
    /// Resolve one opaque source selection to an anchored root descriptor.
    fn resolve_root(
        &self,
        volume_uid: &ResourceUid,
        source_policy_id: Option<&BoundedToken>,
        system_artifact_id: Option<&BoundedToken>,
        kind: SourceKind,
    ) -> Result<ResolvedVolumeRoot, VolumeLocalError>;

    /// Resolve one typed User reference to its host UID.
    fn resolve_principal(&self, reference: &ResourceRef) -> Result<u32, VolumeLocalError>;

    /// Resolve one typed User reference to its host GID.
    fn resolve_group(&self, reference: &ResourceRef) -> Result<u32, VolumeLocalError> {
        self.resolve_principal(reference)
    }
}

/// A descriptor-backed resolver useful for host composition and hermetic
/// production-path tests.
pub struct FdRootResolver {
    root: Arc<OwnedFd>,
    volume_uid: ResourceUid,
    quota: QuotaCapability,
    preexisting_state: bool,
}

impl FdRootResolver {
    /// Bind one already opened directory descriptor to a Volume.
    pub fn new(root: File, volume_uid: ResourceUid) -> Result<Self, VolumeLocalError> {
        let fd: OwnedFd = root.into();
        ensure_directory(&fd)?;
        Ok(Self {
            root: Arc::new(fd),
            volume_uid,
            quota: QuotaCapability::Enforceable,
            preexisting_state: false,
        })
    }

    /// Override the trusted quota observation.
    pub const fn with_quota(mut self, quota: QuotaCapability) -> Self {
        self.quota = quota;
        self
    }

    /// Mark the opened root as already materialized by a trusted effect.
    pub const fn with_preexisting_state(mut self) -> Self {
        self.preexisting_state = true;
        self
    }
}

impl core::fmt::Debug for FdRootResolver {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("FdRootResolver(<redacted>)")
    }
}

impl VolumeRootResolver for FdRootResolver {
    fn resolve_root(
        &self,
        volume_uid: &ResourceUid,
        _source_policy_id: Option<&BoundedToken>,
        _system_artifact_id: Option<&BoundedToken>,
        _kind: SourceKind,
    ) -> Result<ResolvedVolumeRoot, VolumeLocalError> {
        if volume_uid != &self.volume_uid {
            return Err(VolumeLocalError::SourceUnresolved);
        }
        let fd = fcntl_dupfd_cloexec(self.root.as_ref(), 0)
            .map_err(|_| VolumeLocalError::EffectFailed)?;
        Ok(ResolvedVolumeRoot::new(fd, volume_uid.clone())?
            .with_quota(self.quota)
            .with_preexisting_state_if(self.preexisting_state))
    }

    fn resolve_principal(&self, reference: &ResourceRef) -> Result<u32, VolumeLocalError> {
        if reference.resource_type().as_str() != "User" {
            return Err(VolumeLocalError::InvalidSpec);
        }
        Ok(Uid::current().as_raw())
    }

    fn resolve_group(&self, reference: &ResourceRef) -> Result<u32, VolumeLocalError> {
        if reference.resource_type().as_str() != "User" {
            return Err(VolumeLocalError::InvalidSpec);
        }
        Ok(Gid::current().as_raw())
    }
}

/// The production source/layout adapter over a trusted root resolver.
pub struct AnchoredVolumeEffectAdapter<R> {
    resolver: R,
}

impl<R> AnchoredVolumeEffectAdapter<R> {
    /// Build an adapter over one trusted root resolver.
    pub const fn new(resolver: R) -> Self {
        Self { resolver }
    }

    /// Borrow the trusted resolver.
    pub const fn resolver(&self) -> &R {
        &self.resolver
    }
}

impl<R: core::fmt::Debug> core::fmt::Debug for AnchoredVolumeEffectAdapter<R> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("AnchoredVolumeEffectAdapter")
            .field("resolver", &self.resolver)
            .finish()
    }
}

impl<R: VolumeRootResolver> VolumeSourceEffectPort for AnchoredVolumeEffectAdapter<R> {
    fn resolve_root(
        &self,
        source_policy_id: Option<&BoundedToken>,
        system_artifact_id: Option<&BoundedToken>,
        kind: SourceKind,
    ) -> impl Future<Output = Result<VolumeRootHandle, VolumeLocalError>> + Send {
        let _ = (source_policy_id, system_artifact_id, kind);
        async { Err(VolumeLocalError::SourceUnresolved) }
    }

    fn resolve_root_for(
        &self,
        volume_uid: &ResourceUid,
        source_policy_id: Option<&BoundedToken>,
        system_artifact_id: Option<&BoundedToken>,
        kind: SourceKind,
    ) -> impl Future<Output = Result<VolumeRootHandle, VolumeLocalError>> + Send {
        let root =
            self.resolver
                .resolve_root(volume_uid, source_policy_id, system_artifact_id, kind);
        async move {
            let root = root?;
            if root.volume_uid != *volume_uid {
                // Cardinality: per distinct resolve attempt; resolver
                // mismatch is never expected in steady state.
                tracing::warn!(
                    volume = ?volume_uid,
                    "volume root resolver returned a different volume uid",
                );
                return Err(VolumeLocalError::SourceUnresolved);
            }
            Ok(VolumeRootHandle::from_anchored(
                root.fd,
                root.marker_root_fd,
                root.volume_uid,
                root.marker_name,
                root.lock_name,
                root.identity,
                root.marker_binding,
                root.marker_owner_uid,
                root.marker_group_gid,
                root.preexisting_state,
            ))
        }
    }

    fn quota_capability(
        &self,
        root: &VolumeRootHandle,
    ) -> impl Future<Output = Result<QuotaCapability, VolumeLocalError>> + Send {
        let result = match root_fd(root) {
            Ok(Some(fd)) => ensure_directory(fd).map(|_| QuotaCapability::Enforceable),
            Ok(None) | Err(_) => {
                // Cardinality: per distinct quota probe; fires only while
                // the root is unusable.
                tracing::warn!(
                    volume = ?root.volume_uid(),
                    "quota capability probe failed for volume root",
                );
                Err(VolumeLocalError::EffectFailed)
            }
        };
        async move { result }
    }
}

impl<R: VolumeRootResolver> VolumeLayoutEffectPort for AnchoredVolumeEffectAdapter<R> {
    fn observe(
        &self,
        root: &VolumeRootHandle,
        entry: &EntryRequest,
    ) -> impl Future<Output = Result<ObservedEntry, VolumeLocalError>> + Send {
        let result = self.observe_sync(root, entry);
        async move { result }
    }

    fn provision(
        &self,
        root: &VolumeRootHandle,
        entry: &EntryRequest,
    ) -> impl Future<Output = Result<(), VolumeLocalError>> + Send {
        let result = self.provision_sync(root, entry);
        async move { result }
    }

    fn repair(
        &self,
        root: &VolumeRootHandle,
        entry: &EntryRequest,
        drift: &BTreeSet<DriftClass>,
    ) -> impl Future<Output = Result<(), VolumeLocalError>> + Send {
        let result = self.repair_sync(root, entry, drift);
        async move { result }
    }

    fn apply_acl(
        &self,
        root: &VolumeRootHandle,
        entry: &EntryRequest,
    ) -> impl Future<Output = Result<(), VolumeLocalError>> + Send {
        let result = self.apply_acl_sync(root, entry);
        async move { result }
    }

    fn cleanup(
        &self,
        root: &VolumeRootHandle,
        entry: &EntryRequest,
    ) -> impl Future<Output = Result<(), VolumeLocalError>> + Send {
        let result = self.cleanup_sync(root, entry);
        async move { result }
    }

    fn marker_state(
        &self,
        root: &VolumeRootHandle,
    ) -> impl Future<Output = Result<MarkerState, VolumeLocalError>> + Send {
        let result = self.marker_state_sync(root);
        async move { result }
    }

    fn publish_marker(
        &self,
        root: &VolumeRootHandle,
    ) -> impl Future<Output = Result<(), VolumeLocalError>> + Send {
        let result = self.publish_marker_sync(root);
        async move { result }
    }

    fn materialize_content(
        &self,
        root: &VolumeRootHandle,
        projection: &ContentProjection,
    ) -> impl Future<Output = Result<ContentMaterializationEvidence, VolumeLocalError>> + Send {
        let result = self.materialize_content_sync(root, projection);
        async move { result }
    }

    fn materialize_network_config(
        &self,
        root: &VolumeRootHandle,
        projection: &NetworkConfigContentProjection,
    ) -> impl Future<Output = Result<NetworkConfigMaterializationEvidence, VolumeLocalError>> + Send
    {
        let result = self.materialize_network_config_sync(root, projection);
        async move { result }
    }

    fn observe_store_view_marker(
        &self,
        root: &VolumeRootHandle,
        marker_path: &str,
    ) -> impl Future<Output = Result<StoreViewMarkerEvidence, VolumeLocalError>> + Send {
        let result = self.observe_store_view_marker_sync(root, marker_path);
        async move { result }
    }
}

impl<R: VolumeRootResolver> VolumeSourceEffectPort for &AnchoredVolumeEffectAdapter<R> {
    fn resolve_root(
        &self,
        source_policy_id: Option<&BoundedToken>,
        system_artifact_id: Option<&BoundedToken>,
        kind: SourceKind,
    ) -> impl Future<Output = Result<VolumeRootHandle, VolumeLocalError>> + Send {
        (*self).resolve_root(source_policy_id, system_artifact_id, kind)
    }

    fn resolve_root_for(
        &self,
        volume_uid: &ResourceUid,
        source_policy_id: Option<&BoundedToken>,
        system_artifact_id: Option<&BoundedToken>,
        kind: SourceKind,
    ) -> impl Future<Output = Result<VolumeRootHandle, VolumeLocalError>> + Send {
        (*self).resolve_root_for(volume_uid, source_policy_id, system_artifact_id, kind)
    }

    fn quota_capability(
        &self,
        root: &VolumeRootHandle,
    ) -> impl Future<Output = Result<QuotaCapability, VolumeLocalError>> + Send {
        (*self).quota_capability(root)
    }
}

impl<R: VolumeRootResolver> VolumeLayoutEffectPort for &AnchoredVolumeEffectAdapter<R> {
    fn observe(
        &self,
        root: &VolumeRootHandle,
        entry: &EntryRequest,
    ) -> impl Future<Output = Result<ObservedEntry, VolumeLocalError>> + Send {
        (*self).observe(root, entry)
    }

    fn provision(
        &self,
        root: &VolumeRootHandle,
        entry: &EntryRequest,
    ) -> impl Future<Output = Result<(), VolumeLocalError>> + Send {
        (*self).provision(root, entry)
    }

    fn repair(
        &self,
        root: &VolumeRootHandle,
        entry: &EntryRequest,
        drift: &BTreeSet<DriftClass>,
    ) -> impl Future<Output = Result<(), VolumeLocalError>> + Send {
        (*self).repair(root, entry, drift)
    }

    fn apply_acl(
        &self,
        root: &VolumeRootHandle,
        entry: &EntryRequest,
    ) -> impl Future<Output = Result<(), VolumeLocalError>> + Send {
        (*self).apply_acl(root, entry)
    }

    fn cleanup(
        &self,
        root: &VolumeRootHandle,
        entry: &EntryRequest,
    ) -> impl Future<Output = Result<(), VolumeLocalError>> + Send {
        (*self).cleanup(root, entry)
    }

    fn marker_state(
        &self,
        root: &VolumeRootHandle,
    ) -> impl Future<Output = Result<MarkerState, VolumeLocalError>> + Send {
        (*self).marker_state(root)
    }

    fn publish_marker(
        &self,
        root: &VolumeRootHandle,
    ) -> impl Future<Output = Result<(), VolumeLocalError>> + Send {
        (*self).publish_marker(root)
    }

    fn materialize_content(
        &self,
        root: &VolumeRootHandle,
        projection: &ContentProjection,
    ) -> impl Future<Output = Result<ContentMaterializationEvidence, VolumeLocalError>> + Send {
        (*self).materialize_content(root, projection)
    }

    fn materialize_network_config(
        &self,
        root: &VolumeRootHandle,
        projection: &NetworkConfigContentProjection,
    ) -> impl Future<Output = Result<NetworkConfigMaterializationEvidence, VolumeLocalError>> + Send
    {
        (*self).materialize_network_config(root, projection)
    }

    fn observe_store_view_marker(
        &self,
        root: &VolumeRootHandle,
        marker_path: &str,
    ) -> impl Future<Output = Result<StoreViewMarkerEvidence, VolumeLocalError>> + Send {
        (*self).observe_store_view_marker(root, marker_path)
    }
}

impl<R: VolumeRootResolver> AnchoredVolumeEffectAdapter<R> {
    fn observe_sync(
        &self,
        root: &VolumeRootHandle,
        entry: &EntryRequest,
    ) -> Result<ObservedEntry, VolumeLocalError> {
        self.with_lock(root, |guard| {
            let Some(fd) = root_fd(root)? else {
                return Err(VolumeLocalError::EffectFailed);
            };
            ensure_root_identity(root)?;
            let Some((target, _parent)) = open_entry(
                fd,
                entry.declared().path(),
                false,
                entry.entry_type() == EntryType::Symlink,
            )?
            else {
                return Ok(ObservedEntry::absent());
            };
            let stat = fstat(&target).map_err(|_| VolumeLocalError::EffectFailed)?;
            let mut drift = BTreeSet::new();
            if !entry_type_matches(stat.st_mode, entry.entry_type()) {
                drift.insert(DriftClass::EntryType);
            }
            let observer_only_symlink = entry.entry_type() == EntryType::Symlink
                && entry.repair_policy() == RepairPolicy::None;
            if !observer_only_symlink {
                let owner = self
                    .resolver
                    .resolve_principal(entry.declared().owner_ref())?;
                let group = self.resolver.resolve_group(entry.declared().group_ref())?;
                if stat.st_uid != owner || stat.st_gid != group {
                    drift.insert(DriftClass::Owner);
                }
                if (stat.st_mode as u32 & 0o777) != parse_mode(entry.declared().mode())? {
                    drift.insert(DriftClass::Mode);
                }
            }
            if entry.has_acl() {
                drift.insert(DriftClass::Acl);
            }
            guard
                .validate_resource(root_uid(root)?)
                .map_err(|_| VolumeLocalError::EffectFailed)?;
            let owner_proof = match entry.lease_class() {
                // No declared lease: nothing to prove about a live owner.
                d2b_contracts_resource::v3::volume::LeaseClass::None => {
                    OwnerProof::NotApplicable
                }
                // `file-record`: the file must carry the live owner's record.
                // The broker (or the daemon itself, for a lock it created)
                // writes it under the exclusive lock; ownership is verified
                // against the kernel, never against a caller-supplied name.
                d2b_contracts_resource::v3::volume::LeaseClass::FileRecord => {
                    match d2b_host::hardlink_farm::SyncLockOwnerRecord::read_locked(&target)
                        .and_then(|record| record.verify_live())
                    {
                        Ok(()) => OwnerProof::Live,
                        Err(reason) => {
                            tracing::debug!(
                                volume = ?root_uid(root)?,
                                path = entry.declared().path(),
                                reason = %reason,
                                "volume file-record lease owner is not verifiable; quarantining",
                            );
                            OwnerProof::Unknown
                        }
                    }
                }
                // No live-lease reader exists for the other classes: an
                // entry that declares one never adopts on an unproven owner.
                _ => OwnerProof::Unknown,
            };
            Ok(ObservedEntry {
                present: true,
                drift,
                symlink_encountered: false,
                foreign_children: false,
                owner_proof,
            })
        })
    }

    fn provision_sync(
        &self,
        root: &VolumeRootHandle,
        entry: &EntryRequest,
    ) -> Result<(), VolumeLocalError> {
        self.with_lock(root, |_guard| {
            let fd = root_fd(root)?.ok_or(VolumeLocalError::EffectFailed)?;
            ensure_root_identity(root)?;
            let (parent, leaf) = parent_for(fd, entry.declared().path(), false)?;
            match entry.entry_type() {
                EntryType::Directory => {
                    mkdirat(
                        &parent,
                        leaf.as_str(),
                        Mode::from_raw_mode(parse_mode(entry.declared().mode())?),
                    )
                    .map_err(|_| VolumeLocalError::EffectFailed)?;
                    let target = openat2(
                        &parent,
                        leaf.as_str(),
                        directory_flags(),
                        Mode::empty(),
                        resolve_flags(),
                    )
                    .map_err(|_| VolumeLocalError::EffectFailed)?;
                    apply_metadata(&target, &self.resolver, entry)?;
                    fsync(&target).map_err(|_| VolumeLocalError::EffectFailed)?;
                }
                EntryType::File => {
                    let target = openat2(
                        &parent,
                        leaf.as_str(),
                        OFlags::WRONLY
                            | OFlags::CREATE
                            | OFlags::EXCL
                            | OFlags::CLOEXEC
                            | OFlags::NOFOLLOW,
                        Mode::from_raw_mode(parse_mode(entry.declared().mode())?),
                        resolve_flags(),
                    )
                    .map_err(|_| VolumeLocalError::EffectFailed)?;
                    apply_metadata(&target, &self.resolver, entry)?;
                    if entry.lease_class()
                        == d2b_contracts_resource::v3::volume::LeaseClass::FileRecord
                    {
                        // The creator of a file-record lock is its first live
                        // owner: record the identity the kernel reports for
                        // this process so a later pass can verify (or refute)
                        // that ownership. A broker StoreSync that takes the
                        // lock overwrites the record with its own identity.
                        d2b_host::hardlink_farm::SyncLockOwnerRecord::record_current_process(&target)
                            .map_err(|_| VolumeLocalError::EffectFailed)?;
                    }
                    fsync(&target).map_err(|_| VolumeLocalError::EffectFailed)?;
                }
                EntryType::Symlink => {
                    let target = entry
                        .declared()
                        .target()
                        .ok_or(VolumeLocalError::InvalidSpec)?;
                    symlinkat(target, &parent, leaf.as_str())
                        .map_err(|_| VolumeLocalError::EffectFailed)?;
                }
                EntryType::UnixSocket => return Err(VolumeLocalError::EffectFailed),
            }
            fsync(&parent).map_err(|_| VolumeLocalError::EffectFailed)
        })
    }

    fn repair_sync(
        &self,
        root: &VolumeRootHandle,
        entry: &EntryRequest,
        drift: &BTreeSet<DriftClass>,
    ) -> Result<(), VolumeLocalError> {
        if drift
            .iter()
            .any(|class| matches!(class, DriftClass::EntryType | DriftClass::SameFilesystem))
        {
            // Cardinality: per distinct repair request; unrecoverable drift
            // repeats until the spec is corrected.
            tracing::warn!(
                volume = ?root.volume_uid(),
                path = entry.declared().path(),
                drift = ?drift,
                "volume repair refused: unrecoverable drift classes",
            );
            return Err(VolumeLocalError::InvariantViolated);
        }
        // ACL drift is applied by the separate `apply_acl` pass, not here: a
        // repair set that names the class converges through that pass instead
        // of failing this one.
        self.with_lock(root, |_guard| {
            let fd = root_fd(root)?.ok_or(VolumeLocalError::EffectFailed)?;
            ensure_root_identity(root)?;
            let Some((target, _)) = open_entry(fd, entry.declared().path(), false, false)? else {
                return Err(VolumeLocalError::EntryMissing);
            };
            if drift.contains(&DriftClass::Owner) || drift.contains(&DriftClass::Mode) {
                apply_metadata(&target, &self.resolver, entry)?;
                fsync(&target).map_err(|_| VolumeLocalError::EffectFailed)?;
            }
            Ok(())
        })
    }

    /// Apply the declared access and default ACLs of one entry.
    ///
    /// Every grant principal is a `User` reference resolved through the
    /// trusted resolver; the daemon owns the entry it created, so the xattrs
    /// are applied without any capability. The mask carries the group-class
    /// permissions, so the declared mode's group bits must cover it for the
    /// declaration to be idempotent (`setfacl` semantics: the mask is
    /// mirrored in the file's group bits).
    fn apply_acl_sync(
        &self,
        root: &VolumeRootHandle,
        entry: &EntryRequest,
    ) -> Result<(), VolumeLocalError> {
        if !entry.has_acl() {
            return Ok(());
        }
        self.with_lock(root, |_guard| {
            let fd = root_fd(root)?.ok_or(VolumeLocalError::EffectFailed)?;
            ensure_root_identity(root)?;
            let Some((target, _)) = open_entry(fd, entry.declared().path(), false, false)? else {
                return Err(VolumeLocalError::EntryMissing);
            };
            let mode = parse_mode(entry.declared().mode())? & 0o777;
            let access = acl_entries(mode, declared_acl_entries(&self.resolver, entry, "accessAcl")?);
            set_acl_xattr(&target, POSIX_ACL_ACCESS_XATTR, &access)?;
            if entry.entry_type() == EntryType::Directory {
                let default =
                    acl_entries(mode, declared_acl_entries(&self.resolver, entry, "defaultAcl")?);
                set_acl_xattr(&target, POSIX_ACL_DEFAULT_XATTR, &default)?;
            }
            fsync(&target).map_err(|_| VolumeLocalError::EffectFailed)
        })
    }

    fn cleanup_sync(
        &self,
        root: &VolumeRootHandle,
        entry: &EntryRequest,
    ) -> Result<(), VolumeLocalError> {
        self.with_lock(root, |_guard| {
            let fd = root_fd(root)?.ok_or(VolumeLocalError::EffectFailed)?;
            ensure_root_identity(root)?;
            let (parent, leaf) = parent_for(fd, entry.declared().path(), false)?;
            let flags = if entry.entry_type() == EntryType::Directory {
                AtFlags::REMOVEDIR
            } else {
                AtFlags::empty()
            };
            match unlinkat(&parent, leaf.as_str(), flags) {
                Ok(()) => fsync(&parent).map_err(|_| VolumeLocalError::EffectFailed),
                Err(error) if error == rustix::io::Errno::NOENT => Ok(()),
                Err(error) => {
                    // Cardinality: per distinct cleanup failure.
                    tracing::warn!(
                        volume = ?root.volume_uid(),
                        path = entry.declared().path(),
                        error = ?error,
                        "volume entry cleanup unlink failed",
                    );
                    Err(VolumeLocalError::EffectFailed)
                }
            }
        })
    }

    fn marker_state_sync(&self, root: &VolumeRootHandle) -> Result<MarkerState, VolumeLocalError> {
        self.with_lock(root, |_guard| marker_state_unlocked(root))
    }

    fn publish_marker_sync(&self, root: &VolumeRootHandle) -> Result<(), VolumeLocalError> {
        self.with_lock(root, |_guard| {
            ensure_root_identity(root)?;
            if marker_state_unlocked(root)? == MarkerState::Provisioned {
                return Ok(());
            }
            let mut store = FdMarkerStore::new(root)?;
            provision_marker(
                &mut store,
                root.marker_binding()
                    .ok_or(VolumeLocalError::EffectFailed)?,
            )
            .map_err(|_| VolumeLocalError::EffectFailed)
        })
    }

    fn materialize_content_sync(
        &self,
        root: &VolumeRootHandle,
        projection: &ContentProjection,
    ) -> Result<ContentMaterializationEvidence, VolumeLocalError> {
        projection.validate()?;
        if root_uid(root)? != projection.volume_uid() {
            return Err(VolumeLocalError::InvariantViolated);
        }
        self.with_lock(root, |_guard| {
            let observed = self.materialize_files_locked(root, projection.files())?;
            ContentMaterializationEvidence::from_readback(projection, &observed)
        })
    }

    fn materialize_network_config_sync(
        &self,
        root: &VolumeRootHandle,
        projection: &NetworkConfigContentProjection,
    ) -> Result<NetworkConfigMaterializationEvidence, VolumeLocalError> {
        projection.validate()?;
        if root_uid(root)? != projection.volume_uid() {
            return Err(VolumeLocalError::InvariantViolated);
        }
        let files = [
            ContentFile::new(
                "dnsmasq.conf",
                projection.file_owner().clone(),
                projection.file_group().clone(),
                projection.file_mode(),
                projection.dnsmasq().to_vec(),
            )?,
            ContentFile::new(
                "nftables.rules",
                projection.file_owner().clone(),
                projection.file_group().clone(),
                projection.file_mode(),
                projection.nftables().to_vec(),
            )?,
            ContentFile::new(
                "routing.conf",
                projection.file_owner().clone(),
                projection.file_group().clone(),
                projection.file_mode(),
                projection.routing().to_vec(),
            )?,
            ContentFile::new(
                "attachments.json",
                projection.file_owner().clone(),
                projection.file_group().clone(),
                projection.file_mode(),
                projection.attachments().to_vec(),
            )?,
        ];
        self.with_lock(root, |_guard| {
            let observed = self.materialize_files_locked(root, &files)?;
            NetworkConfigMaterializationEvidence::from_observed_files(
                projection,
                &observed[0].bytes(),
                &observed[1].bytes(),
                &observed[2].bytes(),
                &observed[3].bytes(),
            )
        })
    }

    fn observe_store_view_marker_sync(
        &self,
        root: &VolumeRootHandle,
        marker_path: &str,
    ) -> Result<StoreViewMarkerEvidence, VolumeLocalError> {
        validate_store_view_marker_path(marker_path)?;
        self.with_lock(root, |_guard| {
            let fd = root_fd(root)?.ok_or(VolumeLocalError::EffectFailed)?;
            ensure_root_identity(root)?;
            let Some((target, _parent)) = open_entry(fd, marker_path, false, false)? else {
                return Ok(StoreViewMarkerEvidence {
                    present: false,
                    zero_length: false,
                });
            };
            let stat = fstat(&target).map_err(|_| VolumeLocalError::EffectFailed)?;
            let present = FileType::from_raw_mode(stat.st_mode) == FileType::RegularFile
                && stat.st_nlink == 1;
            Ok(StoreViewMarkerEvidence {
                present,
                zero_length: present && stat.st_size == 0,
            })
        })
    }

    fn materialize_files_locked(
        &self,
        root: &VolumeRootHandle,
        files: &[ContentFile],
    ) -> Result<Vec<ObservedContentFile>, VolumeLocalError> {
        ensure_root_identity(root)?;
        if marker_state_unlocked(root)? != MarkerState::Provisioned {
            // Cardinality: per distinct materialization attempt; repeats
            // while the volume stays unprovisioned.
            tracing::warn!(
                volume = ?root.volume_uid(),
                "volume content materialization refused: marker not provisioned",
            );
            return Err(VolumeLocalError::InvariantViolated);
        }
        for file in files {
            let existing = inspect_content_file(
                root,
                file.path(),
                file.owner(),
                file.group(),
                self.resolver.resolve_principal(file.owner())?,
                self.resolver.resolve_group(file.group())?,
            )?;
            if let Some(existing) = existing {
                if existing.owner() != file.owner()
                    || existing.group() != file.group()
                    || existing.mode() != file.mode()
                {
                    // Cardinality: per distinct mismatched file; repeats
                    // until the on-disk file is corrected.
                    tracing::warn!(
                        volume = ?root.volume_uid(),
                        path = file.path(),
                        "volume content file metadata diverges from projection",
                    );
                    return Err(VolumeLocalError::InvariantViolated);
                }
                if existing.bytes() != file.bytes() {
                    let mut filesystem = AnchoredAtomicFilesystem::new(root, file.path())?;
                    replace_bytes(
                        &mut filesystem,
                        file.bytes(),
                        self.resolver.resolve_principal(file.owner())?,
                        self.resolver.resolve_group(file.group())?,
                        parse_mode(file.mode())?,
                    )
                    .map_err(|error| {
                        // Cardinality: per distinct replacement failure.
                        tracing::warn!(
                            volume = ?root.volume_uid(),
                            path = file.path(),
                            error = ?error,
                            "volume content file replacement failed (existing)",
                        );
                        VolumeLocalError::EffectFailed
                    })?;
                }
            } else {
                let mut filesystem = AnchoredAtomicFilesystem::new(root, file.path())?;
                replace_bytes(
                    &mut filesystem,
                    file.bytes(),
                    self.resolver.resolve_principal(file.owner())?,
                    self.resolver.resolve_group(file.group())?,
                    parse_mode(file.mode())?,
                )
                .map_err(|error| {
                    // Cardinality: per distinct creation failure.
                    tracing::warn!(
                        volume = ?root.volume_uid(),
                        path = file.path(),
                        error = ?error,
                        "volume content file replacement failed (new)",
                    );
                    VolumeLocalError::EffectFailed
                })?;
            }
        }
        files
            .iter()
            .map(|file| {
                inspect_content_file(
                    root,
                    file.path(),
                    file.owner(),
                    file.group(),
                    self.resolver.resolve_principal(file.owner())?,
                    self.resolver.resolve_group(file.group())?,
                )?
                .ok_or(VolumeLocalError::EffectFailed)
            })
            .collect()
    }

    fn with_lock<T>(
        &self,
        root: &VolumeRootHandle,
        operation: impl FnOnce(&LockGuard) -> Result<T, VolumeLocalError>,
    ) -> Result<T, VolumeLocalError> {
        let fd = root_fd(root)?.ok_or(VolumeLocalError::EffectFailed)?;
        let lock_name = root.lock_name().ok_or(VolumeLocalError::EffectFailed)?;
        validate_component(lock_name)?;
        let lock_fd = openat2(
            fd,
            lock_name,
            OFlags::RDWR | OFlags::CREATE | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::from_raw_mode(LOCK_MODE),
            resolve_flags(),
        )
        .map_err(|_| VolumeLocalError::EffectFailed)?;
        let backend = FdLockBackend { fd: lock_fd };
        let spec = LockSpec::new(
            LockId::parse(format!("volume-lock-{}", root_uid(root)?.as_str()))
                .map_err(|_| VolumeLocalError::EffectFailed)?,
            root_uid(root)?.clone(),
            1,
            Vec::new(),
            5_000,
            LockTransferPolicy::Never,
        )
        .map_err(|_| VolumeLocalError::EffectFailed)?;
        let mut locks = LockSet::new();
        let guard = locks.acquire(&backend, &spec).map_err(|error| {
            // Cardinality: per distinct lock acquisition failure; the
            // effect that needed the lock fails closed.
            tracing::warn!(
                volume = ?root.volume_uid(),
                error = ?error,
                "volume OFD lock acquisition failed",
            );
            VolumeLocalError::EffectFailed
        })?;
        operation(guard)
    }
}

fn root_fd(root: &VolumeRootHandle) -> Result<Option<&OwnedFd>, VolumeLocalError> {
    Ok(root.anchored_fd())
}

fn root_uid(root: &VolumeRootHandle) -> Result<&ResourceUid, VolumeLocalError> {
    root.volume_uid().ok_or(VolumeLocalError::EffectFailed)
}

fn ensure_root_identity(root: &VolumeRootHandle) -> Result<(), VolumeLocalError> {
    let fd = root.anchored_fd().ok_or(VolumeLocalError::EffectFailed)?;
    let expected = root.root_identity().ok_or(VolumeLocalError::EffectFailed)?;
    if root_identity(fd)? != expected {
        // Cardinality: per distinct identity check; a mismatch means the
        // root fd no longer points at the provisioned directory.
        tracing::warn!(
            volume = ?root.volume_uid(),
            "volume root identity mismatch (fd detached from provisioned directory)",
        );
        return Err(VolumeLocalError::InvariantViolated);
    }
    Ok(())
}

fn ensure_directory(fd: &OwnedFd) -> Result<(), VolumeLocalError> {
    let stat = fstat(fd).map_err(|_| VolumeLocalError::EffectFailed)?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::Directory {
        return Err(VolumeLocalError::InvariantViolated);
    }
    Ok(())
}

fn root_identity(fd: &OwnedFd) -> Result<VolumeRootIdentity, VolumeLocalError> {
    let stat = fstat(fd).map_err(|_| VolumeLocalError::EffectFailed)?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::Directory {
        return Err(VolumeLocalError::InvariantViolated);
    }
    Ok(VolumeRootIdentity {
        device: stat.st_dev,
        inode: stat.st_ino,
    })
}

fn parse_mode(mode: &str) -> Result<u32, VolumeLocalError> {
    if mode.len() != 4
        || !mode.starts_with('0')
        || !mode[1..].bytes().all(|byte| (b'0'..=b'7').contains(&byte))
    {
        return Err(VolumeLocalError::InvalidSpec);
    }
    u32::from_str_radix(&mode[1..], 8).map_err(|_| VolumeLocalError::InvalidSpec)
}

pub(crate) fn validate_component(value: &str) -> Result<(), VolumeLocalError> {
    if value.is_empty()
        || value == "."
        || value == ".."
        || value.contains('/')
        || value.contains('\\')
        || value.contains('\0')
    {
        return Err(VolumeLocalError::InvariantViolated);
    }
    Ok(())
}

fn validate_store_view_marker_path(path: &str) -> Result<(), VolumeLocalError> {
    if path.is_empty()
        || path.starts_with('/')
        || path.contains('\\')
        || path.contains('\0')
        || path
            .split('/')
            .any(|component| component.is_empty() || component == "." || component == "..")
    {
        return Err(VolumeLocalError::InvalidSpec);
    }
    Ok(())
}

fn resolve_flags() -> ResolveFlags {
    ResolveFlags::BENEATH
        | ResolveFlags::NO_SYMLINKS
        | ResolveFlags::NO_MAGICLINKS
        | ResolveFlags::NO_XDEV
}

fn directory_flags() -> OFlags {
    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW
}

fn parent_for(
    root: &OwnedFd,
    path: &str,
    _create_missing: bool,
) -> Result<(OwnedFd, String), VolumeLocalError> {
    let (parent, leaf) = path.rsplit_once('/').unwrap_or(("", path));
    validate_component(leaf)?;
    let parent_fd = if parent.is_empty() {
        fcntl_dupfd_cloexec(root, 0).map_err(|_| VolumeLocalError::EffectFailed)?
    } else {
        openat2(
            root,
            parent,
            directory_flags(),
            Mode::empty(),
            resolve_flags(),
        )
        .map_err(|error| {
            if error == rustix::io::Errno::LOOP || error == rustix::io::Errno::NOTDIR {
                VolumeLocalError::SymlinkTraversalRejected
            } else {
                VolumeLocalError::EffectFailed
            }
        })?
    };
    Ok((parent_fd, leaf.to_owned()))
}

fn open_entry(
    root: &OwnedFd,
    path: &str,
    create_missing: bool,
    allow_final_symlink: bool,
) -> Result<Option<(OwnedFd, OwnedFd)>, VolumeLocalError> {
    if path.is_empty() {
        return Ok(Some((
            fcntl_dupfd_cloexec(root, 0).map_err(|_| VolumeLocalError::EffectFailed)?,
            fcntl_dupfd_cloexec(root, 0).map_err(|_| VolumeLocalError::EffectFailed)?,
        )));
    }
    let (parent, leaf) = parent_for(root, path, create_missing)?;
    let (flags, resolve) = if allow_final_symlink {
        (
            OFlags::PATH | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            ResolveFlags::BENEATH | ResolveFlags::NO_MAGICLINKS | ResolveFlags::NO_XDEV,
        )
    } else {
        (
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            resolve_flags(),
        )
    };
    match openat2(&parent, leaf.as_str(), flags, Mode::empty(), resolve) {
        Ok(fd) => Ok(Some((fd, parent))),
        Err(error) if error == rustix::io::Errno::NOENT => Ok(None),
        Err(error) if error == rustix::io::Errno::LOOP => {
            Err(VolumeLocalError::SymlinkTraversalRejected)
        }
        Err(_) => Err(VolumeLocalError::EffectFailed),
    }
}

fn entry_type_matches(mode: u32, expected: EntryType) -> bool {
    let file_type = FileType::from_raw_mode(mode);
    match expected {
        EntryType::Directory => file_type == FileType::Directory,
        EntryType::File => file_type == FileType::RegularFile,
        EntryType::Symlink => file_type == FileType::Symlink,
        EntryType::UnixSocket => file_type == FileType::Socket,
    }
}

fn apply_metadata<R: VolumeRootResolver>(
    target: &OwnedFd,
    resolver: &R,
    entry: &EntryRequest,
) -> Result<(), VolumeLocalError> {
    let owner = resolver.resolve_principal(entry.declared().owner_ref())?;
    let group = resolver.resolve_group(entry.declared().group_ref())?;
    fchmod(
        target,
        Mode::from_raw_mode(parse_mode(entry.declared().mode())?),
    )
    .map_err(|_| VolumeLocalError::EffectFailed)?;
    fchown(
        target.as_raw_fd(),
        Some(Uid::from_raw(owner)),
        Some(Gid::from_raw(group)),
    )
    .map_err(|_| VolumeLocalError::EffectFailed)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// POSIX ACL application
//
// The declared grants are real kernel ACLs: the daemon owns the entry it
// creates, so it applies them through the `system.posix_acl_*` xattrs without
// any capability. The encoding is the kernel's `posix_acl_xattr_header` +
// `posix_acl_xattr_entry` layout, and the mask carries the group-class
// permissions exactly as `setfacl` computes it - the kernel requires the mask
// whenever named entries exist, and mirrors it in the file's group bits.
// ---------------------------------------------------------------------------

/// `POSIX_ACL_XATTR_VERSION`.
const POSIX_ACL_XATTR_VERSION: u32 = 2;
/// The id of every base ACL entry (`owner`/`group`/`mask`/`other`).
const ACL_UNDEFINED_ID: u32 = u32::MAX;
// ACL entry tags (`linux/posix_acl.h`).
const ACL_USER_OBJ_TAG: u16 = 0x01;
const ACL_USER_TAG: u16 = 0x02;
const ACL_GROUP_OBJ_TAG: u16 = 0x04;
const ACL_MASK_TAG: u16 = 0x10;
const ACL_OTHER_TAG: u16 = 0x20;
// ACL permission bits.
const ACL_READ: u16 = 0x04;
const ACL_WRITE: u16 = 0x02;
const ACL_EXECUTE: u16 = 0x01;
/// The POSIX ACL xattr names.
const POSIX_ACL_ACCESS_XATTR: &str = "system.posix_acl_access";
const POSIX_ACL_DEFAULT_XATTR: &str = "system.posix_acl_default";

/// One encoded ACL entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AclEntry {
    tag: u16,
    perm: u16,
    id: u32,
}

/// Parse one declared `permissions` spelling into ACL permission bits.
fn acl_permission_bits(permissions: &str) -> Result<u16, VolumeLocalError> {
    if permissions.is_empty() || permissions.len() > 3 {
        return Err(VolumeLocalError::InvalidSpec);
    }
    let mut bits = 0u16;
    for byte in permissions.bytes() {
        bits |= match byte {
            b'r' => ACL_READ,
            b'w' => ACL_WRITE,
            b'x' => ACL_EXECUTE,
            _ => return Err(VolumeLocalError::InvalidSpec),
        };
    }
    Ok(bits)
}

/// Build the full entry set for one declared mode and named-user grants.
///
/// The owner, group-object, and other entries come from the declared mode;
/// every declared grant is one named-user entry; the mask is the union of the
/// group-class permissions (what `setfacl` computes), and it is present
/// exactly when named entries are.
fn acl_entries(mode: u32, mut named: Vec<AclEntry>) -> Vec<AclEntry> {
    let class = |shift: u32| u16::try_from((mode >> shift) & 0o7).unwrap_or(0);
    let (owner, group, other) = (class(6), class(3), class(0));
    named.sort_by_key(|entry| entry.id);
    let mut mask = group;
    for entry in &named {
        mask |= entry.perm;
    }
    let mut entries = vec![AclEntry {
        tag: ACL_USER_OBJ_TAG,
        perm: owner,
        id: ACL_UNDEFINED_ID,
    }];
    entries.extend(named.iter().copied());
    entries.push(AclEntry {
        tag: ACL_GROUP_OBJ_TAG,
        perm: group,
        id: ACL_UNDEFINED_ID,
    });
    if !named.is_empty() {
        entries.push(AclEntry {
            tag: ACL_MASK_TAG,
            perm: mask,
            id: ACL_UNDEFINED_ID,
        });
    }
    entries.push(AclEntry {
        tag: ACL_OTHER_TAG,
        perm: other,
        id: ACL_UNDEFINED_ID,
    });
    entries
}

/// Encode one entry set as the `system.posix_acl_*` xattr value.
fn acl_xattr_bytes(entries: &[AclEntry]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(4 + entries.len() * 8);
    bytes.extend_from_slice(&POSIX_ACL_XATTR_VERSION.to_le_bytes());
    for entry in entries {
        bytes.extend_from_slice(&entry.tag.to_le_bytes());
        bytes.extend_from_slice(&entry.perm.to_le_bytes());
        bytes.extend_from_slice(&entry.id.to_le_bytes());
    }
    bytes
}

/// Decode one declared grant list into named-user ACL entries.
///
/// Every grant principal is a `User` reference, so each grant is one
/// `ACL_USER` entry and the resolver turns the reference into the host uid
/// the account carries.
fn declared_acl_entries<R: VolumeRootResolver>(
    resolver: &R,
    entry: &EntryRequest,
    field: &str,
) -> Result<Vec<AclEntry>, VolumeLocalError> {
    let rendered =
        serde_json::to_value(entry.declared()).map_err(|_| VolumeLocalError::InvalidSpec)?;
    let Some(grants) = rendered.get(field).and_then(serde_json::Value::as_array) else {
        return Ok(Vec::new());
    };
    let mut entries = Vec::with_capacity(grants.len());
    for grant in grants {
        let reference = grant
            .get("principal")
            .and_then(|principal| principal.get("ref"))
            .and_then(serde_json::Value::as_str)
            .ok_or(VolumeLocalError::InvalidSpec)
            .and_then(|reference| {
                ResourceRef::parse(reference).map_err(|_| VolumeLocalError::InvalidSpec)
            })?;
        if reference.resource_type().as_str() != "User" {
            return Err(VolumeLocalError::InvalidSpec);
        }
        let permissions = grant
            .get("permissions")
            .and_then(serde_json::Value::as_str)
            .ok_or(VolumeLocalError::InvalidSpec)?;
        entries.push(AclEntry {
            tag: ACL_USER_TAG,
            perm: acl_permission_bits(permissions)?,
            id: resolver.resolve_principal(&reference)?,
        });
    }
    Ok(entries)
}

/// Write one encoded ACL xattr onto an open target.
fn set_acl_xattr(
    target: &OwnedFd,
    name: &str,
    entries: &[AclEntry],
) -> Result<(), VolumeLocalError> {
    let bytes = acl_xattr_bytes(entries);
    rustix::fs::fsetxattr(target, name, &bytes, XattrFlags::empty())
        .map_err(|_| VolumeLocalError::EffectFailed)
}

fn marker_state_unlocked(root: &VolumeRootHandle) -> Result<MarkerState, VolumeLocalError> {
    ensure_root_identity(root)?;
    let mut store = FdMarkerStore::new(root)?;
    let binding = root
        .marker_binding()
        .ok_or(VolumeLocalError::EffectFailed)?;
    if store
        .read_marker(root_uid(root)?)
        .map_err(|_| VolumeLocalError::EffectFailed)?
        .is_none()
    {
        return if root_has_unprovisioned_entries(root)? {
            if root.preexisting_state() {
                Ok(MarkerState::NeverProvisioned)
            } else {
                // Cardinality: per distinct marker observation; repeats
                // while state is inconsistent.
                tracing::warn!(
                    volume = ?root_uid(root)?,
                    "volume previously provisioned but marker state missing",
                );
                Err(VolumeLocalError::PreviouslyProvisionedStateMissing)
            }
        } else {
            Ok(MarkerState::NeverProvisioned)
        };
    }
    match verify_marker(&mut store, root.root_identity(), binding) {
        Ok(MarkerDisposition::Verified) => Ok(MarkerState::Provisioned),
        disposition => {
            // Cardinality: per distinct marker verification failure.
            tracing::warn!(
                volume = ?root_uid(root)?,
                disposition = ?disposition.err(),
                "volume marker verification failed",
            );
            Err(VolumeLocalError::EffectFailed)
        }
    }
}

fn root_has_unprovisioned_entries(root: &VolumeRootHandle) -> Result<bool, VolumeLocalError> {
    let fd = root.anchored_fd().ok_or(VolumeLocalError::EffectFailed)?;
    let lock_name = root.lock_name().unwrap_or(LOCK_NAME);
    let marker_name = root.marker_name().unwrap_or(MARKER_NAME);
    let mut buffer = [MaybeUninit::uninit(); 4096];
    let mut directory = RawDir::new(fd, &mut buffer);
    while let Some(entry) = directory.next() {
        let entry = entry.map_err(|_| VolumeLocalError::EffectFailed)?;
        let name = entry.file_name().to_bytes();
        if name != b"."
            && name != b".."
            && name != lock_name.as_bytes()
            && name != marker_name.as_bytes()
        {
            return Ok(true);
        }
    }
    Ok(false)
}

struct FdMarkerStore<'a> {
    root: &'a OwnedFd,
    name: &'a str,
    owner_uid: u32,
    group_gid: u32,
}

impl<'a> FdMarkerStore<'a> {
    fn new(root: &'a VolumeRootHandle) -> Result<Self, VolumeLocalError> {
        Ok(Self {
            root: root
                .marker_root_fd()
                .or(root.anchored_fd())
                .ok_or(VolumeLocalError::EffectFailed)?,
            name: root.marker_name().ok_or(VolumeLocalError::EffectFailed)?,
            owner_uid: root
                .marker_owner_uid()
                .ok_or(VolumeLocalError::EffectFailed)?,
            group_gid: root
                .marker_group_gid()
                .ok_or(VolumeLocalError::EffectFailed)?,
        })
    }
}

impl MarkerStore for FdMarkerStore<'_> {
    fn read_marker(
        &mut self,
        volume_uid: &ResourceUid,
    ) -> Result<Option<d2b_provider_volume_local::marker::VerifiedMarkerFile>, MarkerError> {
        let fd = match openat2(
            self.root,
            self.name,
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::empty(),
            resolve_flags(),
        ) {
            Ok(fd) => fd,
            Err(error) if error == rustix::io::Errno::NOENT => return Ok(None),
            Err(error) if error == rustix::io::Errno::LOOP => {
                return Err(MarkerError::MarkerInvalid);
            }
            Err(_) => return Err(MarkerError::MarkerReadFailed),
        };
        let stat = fstat(&fd).map_err(|_| MarkerError::MarkerReadFailed)?;
        if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile
            || stat.st_nlink != 1
            || stat.st_mode as u32 & 0o777 != MARKER_MODE
            || stat.st_uid != self.owner_uid
            || stat.st_gid != self.group_gid
        {
            return Err(MarkerError::MarkerInvalid);
        }
        let file = File::from(fd);
        let mut bytes = Vec::new();
        file.take((MAX_MARKER_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|_| MarkerError::MarkerReadFailed)?;
        if bytes.is_empty() || bytes.len() > MAX_MARKER_BYTES {
            return Err(MarkerError::MarkerInvalid);
        }
        let _ = volume_uid;
        Ok(Some(
            d2b_provider_volume_local::marker::VerifiedMarkerFile::from_verified_regular_file(
                bytes,
            ),
        ))
    }

    fn create_marker_exclusive(
        &mut self,
        _volume_uid: &ResourceUid,
        bytes: &[u8],
    ) -> Result<(), MarkerError> {
        if bytes.is_empty() || bytes.len() > MAX_MARKER_BYTES {
            return Err(MarkerError::MarkerWriteFailed);
        }
        let fd = openat2(
            self.root,
            self.name,
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::from_raw_mode(MARKER_MODE),
            resolve_flags(),
        )
        .map_err(|_| MarkerError::MarkerWriteFailed)?;
        let mut file = File::from(fd);
        fchmod(&file, Mode::from_raw_mode(MARKER_MODE))
            .map_err(|_| MarkerError::MarkerWriteFailed)?;
        fchown(
            file.as_raw_fd(),
            Some(Uid::from_raw(self.owner_uid)),
            Some(Gid::from_raw(self.group_gid)),
        )
        .map_err(|_| MarkerError::MarkerWriteFailed)?;
        file.write_all(bytes)
            .and_then(|_| file.sync_all())
            .map_err(|_| MarkerError::MarkerWriteFailed)?;
        fsync(self.root).map_err(|_| MarkerError::MarkerWriteFailed)
    }
}

struct FdLockBackend {
    fd: OwnedFd,
}

struct FdLockHandle {
    fd: Option<OwnedFd>,
}

impl OfdLockBackend for FdLockBackend {
    fn acquire(&self, _spec: &LockSpec) -> Result<Box<dyn OfdLockHandle>, LockError> {
        let fd = fcntl_dupfd_cloexec(&self.fd, 0).map_err(|_| LockError::AcquisitionFailed)?;
        let lock = libc::flock {
            l_type: libc::F_WRLCK as _,
            l_whence: libc::SEEK_SET as _,
            l_start: 0,
            l_len: 0,
            l_pid: 0,
        };
        fcntl(fd.as_raw_fd(), FcntlArg::F_OFD_SETLK(&lock))
            .map_err(|_| LockError::AcquisitionFailed)?;
        Ok(Box::new(FdLockHandle { fd: Some(fd) }))
    }
}

impl OfdLockHandle for FdLockHandle {
    fn release(&mut self) -> Result<(), LockError> {
        let Some(fd) = self.fd.take() else {
            return Ok(());
        };
        let lock = libc::flock {
            l_type: libc::F_UNLCK as _,
            l_whence: libc::SEEK_SET as _,
            l_start: 0,
            l_len: 0,
            l_pid: 0,
        };
        fcntl(fd.as_raw_fd(), FcntlArg::F_OFD_SETLK(&lock))
            .map(|_| ())
            .map_err(|_| LockError::AdapterFailed)
    }

    fn commit_transfer(&mut self) -> Result<(), LockError> {
        Err(LockError::TransferDenied)
    }
}

struct TempFile {
    name: String,
    file: File,
}

struct AnchoredAtomicFilesystem {
    parent: OwnedFd,
    target: String,
    uid: ResourceUid,
}

impl AnchoredAtomicFilesystem {
    fn new(root: &VolumeRootHandle, path: &str) -> Result<Self, VolumeLocalError> {
        let root_fd = root.anchored_fd().ok_or(VolumeLocalError::EffectFailed)?;
        let (parent, target) = parent_for(root_fd, path, false)?;
        Ok(Self {
            parent,
            target,
            uid: root_uid(root)?.clone(),
        })
    }

    fn open_target(&self) -> Result<OwnedFd, AtomicWriteError> {
        openat2(
            &self.parent,
            self.target.as_str(),
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::empty(),
            resolve_flags(),
        )
        .map_err(|_| AtomicWriteError::EffectFailed)
    }
}

impl AtomicFilesystem for AnchoredAtomicFilesystem {
    type Temp = TempFile;

    fn resource_uid(&self) -> &ResourceUid {
        &self.uid
    }

    fn read_target(&mut self, maximum: usize) -> Result<Vec<u8>, AtomicWriteError> {
        let fd = self.open_target()?;
        let file = File::from(fd);
        let mut bytes = Vec::new();
        file.take((maximum + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|_| AtomicWriteError::EffectFailed)?;
        if bytes.len() > maximum {
            return Err(AtomicWriteError::EffectFailed);
        }
        Ok(bytes)
    }

    fn current_generation(&mut self) -> Result<Option<u64>, AtomicWriteError> {
        let bytes = self.read_target(64 * 1024)?;
        Ok(serde_json::from_slice::<serde_json::Value>(&bytes)
            .ok()
            .and_then(|value| value.get("generation").and_then(serde_json::Value::as_u64)))
    }

    fn current_charged_bytes(&mut self) -> Result<u64, AtomicWriteError> {
        self.current_target_bytes()
    }

    fn current_target_bytes(&mut self) -> Result<u64, AtomicWriteError> {
        let fd = self.open_target()?;
        let stat = fstat(&fd).map_err(|_| AtomicWriteError::EffectFailed)?;
        Ok(stat.st_size as u64)
    }

    fn create_temp(&mut self) -> Result<Self::Temp, AtomicWriteError> {
        static NEXT_TEMP: AtomicU64 = AtomicU64::new(1);
        let name = format!(
            ".{}.d2b-tmp-{}-{}",
            self.target,
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        );
        let fd = openat2(
            &self.parent,
            name.as_str(),
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::from_raw_mode(0o600),
            resolve_flags(),
        )
        .map_err(|_| AtomicWriteError::EffectFailed)?;
        Ok(TempFile {
            name,
            file: File::from(fd),
        })
    }

    fn write_temp(
        &mut self,
        temp: &mut Self::Temp,
        bytes: &[u8],
    ) -> Result<usize, AtomicWriteError> {
        temp.file
            .write(bytes)
            .map_err(|_| AtomicWriteError::EffectFailed)
    }

    fn set_temp_metadata(
        &mut self,
        temp: &mut Self::Temp,
        owner: u32,
        group: u32,
        mode: u32,
    ) -> Result<(), AtomicWriteError> {
        fchmod(&temp.file, Mode::from_raw_mode(mode))
            .map_err(|_| AtomicWriteError::EffectFailed)?;
        fchown(
            temp.file.as_raw_fd(),
            Some(Uid::from_raw(owner)),
            Some(Gid::from_raw(group)),
        )
        .map_err(|_| AtomicWriteError::EffectFailed)
    }

    fn sync_temp(&mut self, temp: &mut Self::Temp) -> Result<(), AtomicWriteError> {
        temp.file
            .sync_all()
            .map_err(|_| AtomicWriteError::EffectFailed)
    }

    fn replace_temp(&mut self, temp: &mut Self::Temp) -> Result<(), AtomicWriteError> {
        renameat(
            &self.parent,
            temp.name.as_str(),
            &self.parent,
            self.target.as_str(),
        )
        .map_err(|_| AtomicWriteError::EffectFailed)
    }

    fn sync_parent(&mut self) -> Result<(), AtomicWriteError> {
        let fd =
            fcntl_dupfd_cloexec(&self.parent, 0).map_err(|_| AtomicWriteError::EffectFailed)?;
        File::from(fd)
            .sync_all()
            .map_err(|_| AtomicWriteError::EffectFailed)
    }

    fn remove_temp(&mut self, temp: &mut Self::Temp) {
        let _ = unlinkat(&self.parent, temp.name.as_str(), AtFlags::empty());
    }
}

fn inspect_content_file(
    root: &VolumeRootHandle,
    path: &str,
    owner: &ResourceRef,
    group: &ResourceRef,
    owner_uid: u32,
    group_gid: u32,
) -> Result<Option<ObservedContentFile>, VolumeLocalError> {
    let fd = root.anchored_fd().ok_or(VolumeLocalError::EffectFailed)?;
    let Some((target, _parent)) = open_entry(fd, path, false, false)? else {
        return Ok(None);
    };
    let stat = fstat(&target).map_err(|_| VolumeLocalError::EffectFailed)?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile || stat.st_nlink != 1 {
        return Err(VolumeLocalError::InvariantViolated);
    }
    if stat.st_uid != owner_uid || stat.st_gid != group_gid {
        return Err(VolumeLocalError::InvariantViolated);
    }
    let file = File::from(target);
    let mut bytes = Vec::new();
    file.take((d2b_provider_volume_local::MAX_CONTENT_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| VolumeLocalError::EffectFailed)?;
    if bytes.len() > d2b_provider_volume_local::MAX_CONTENT_BYTES {
        return Err(VolumeLocalError::EffectFailed);
    }
    Ok(Some(ObservedContentFile::new(
        path,
        owner.clone(),
        group.clone(),
        format!("0{:03o}", stat.st_mode as u32 & 0o777),
        bytes,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt as _;

    #[test]
    fn marker_root_fd_supplies_marker_file_ownership() {
        let volume_root = tempfile::tempdir().expect("volume root");
        let marker_root = tempfile::tempdir().expect("marker root");
        let volume_fd = rustix::fs::open(
            volume_root.path(),
            OFlags::DIRECTORY | OFlags::RDONLY | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .expect("open volume root");
        let marker_fd = rustix::fs::open(
            marker_root.path(),
            OFlags::DIRECTORY | OFlags::RDONLY | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .expect("open marker root");
        let volume_uid =
            ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").expect("volume UID");
        let root = ResolvedVolumeRoot::new(volume_fd, volume_uid)
            .expect("bind volume root")
            .with_marker_root(marker_fd)
            .expect("bind marker root");
        let marker_stat = fstat(root.marker_root_fd.as_ref().expect("marker root fd"))
            .expect("stat marker root");
        assert_eq!(root.marker_owner_uid, marker_stat.st_uid);
        assert_eq!(root.marker_group_gid, marker_stat.st_gid);
    }

    #[test]
    fn component_validation_rejects_traversal_names() {
        assert!(validate_component("state").is_ok());
        assert!(validate_component("vol-state-vfd-0123456789abcdef").is_ok());
        for bad in ["", ".", "..", "a/b", "a\\b", "a\0b"] {
            assert!(
                validate_component(bad).is_err(),
                "{bad:?} must not become a volume subdirectory"
            );
        }
    }

    /// A resolver that binds one root descriptor and maps declared `User`
    /// names to arbitrary uids, so a test can name principals other than the
    /// one running it (the kernel accepts named ACL entries for any uid).
    struct NamedUidResolver {
        root: Arc<OwnedFd>,
        volume_uid: ResourceUid,
        uids: std::collections::BTreeMap<String, u32>,
    }

    impl VolumeRootResolver for NamedUidResolver {
        fn resolve_root(
            &self,
            volume_uid: &ResourceUid,
            _source_policy_id: Option<&BoundedToken>,
            _system_artifact_id: Option<&BoundedToken>,
            _kind: SourceKind,
        ) -> Result<ResolvedVolumeRoot, VolumeLocalError> {
            if volume_uid != &self.volume_uid {
                return Err(VolumeLocalError::SourceUnresolved);
            }
            let fd = fcntl_dupfd_cloexec(self.root.as_ref(), 0)
                .map_err(|_| VolumeLocalError::EffectFailed)?;
            ResolvedVolumeRoot::new(fd, volume_uid.clone())
        }

        fn resolve_principal(&self, reference: &ResourceRef) -> Result<u32, VolumeLocalError> {
            self.uids
                .get(reference.name().as_str())
                .copied()
                .ok_or(VolumeLocalError::EffectFailed)
        }

        fn resolve_group(&self, reference: &ResourceRef) -> Result<u32, VolumeLocalError> {
            self.resolve_principal(reference)
        }
    }

    /// One directory entry that declares the same access/default grants the
    /// Device TPM Provider's state Volume declares.
    fn acl_layout_entry() -> d2b_contracts_resource::v3::volume::LayoutEntry {
        serde_json::from_value(serde_json::json!({
            "path": "",
            "type": "directory",
            "ownerRef": "User/owner",
            "groupRef": "User/owner",
            "mode": "0770",
            "target": null,
            "accessAcl": [
                { "principal": { "ref": "User/worker" }, "permissions": "rwx" },
                { "principal": { "ref": "User/flush" }, "permissions": "rx" }
            ],
            "defaultAcl": [
                { "principal": { "ref": "User/worker" }, "permissions": "rwx" },
                { "principal": { "ref": "User/flush" }, "permissions": "rw" }
            ],
            "foreignChildPolicy": "preserve",
            "noFollow": true,
            "recursive": false,
            "sensitivity": "private",
            "createPolicy": "create-if-never-provisioned",
            "repairPolicy": "exact-owner",
            "cleanupPolicy": "never",
            "adoptionPolicy": "quarantine-on-ambiguity",
            "restartPolicy": "preserve-across-controller-restart",
            "leaseClass": "none",
            "invariants": ["no-symlink"]
        }))
        .expect("layout entry")
    }

    fn read_xattr(path: &std::path::Path, name: &str) -> Option<Vec<u8>> {
        let mut buffer = vec![0u8; 256];
        let size = rustix::fs::lgetxattr(path, name, &mut buffer).ok()?;
        buffer.truncate(size);
        Some(buffer)
    }

    /// Regression (U17 TPM state Volume): a declared ACL entry must be
    /// applied for real - the daemon owns the entry it creates, so no
    /// capability is needed, and the encoded access/default ACLs must be what
    /// the kernel stores (mask = the group-class union, mirrored in the mode).
    #[test]
    fn declared_acls_are_applied_to_the_owned_entry() {
        let dir = tempfile::tempdir().expect("volume root");
        let root = File::open(dir.path()).expect("open root");
        let volume_uid =
            ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").expect("volume UID");
        let own = Uid::current().as_raw();
        let resolver = NamedUidResolver {
            root: Arc::new(root.into()),
            volume_uid: volume_uid.clone(),
            uids: std::collections::BTreeMap::from([
                ("owner".to_owned(), own),
                ("worker".to_owned(), own + 1),
                ("flush".to_owned(), own + 2),
            ]),
        };
        let adapter = AnchoredVolumeEffectAdapter::new(resolver);
        let handle = d2b_provider_volume_local::testing::block_on(adapter.resolve_root_for(
            &volume_uid,
            None,
            None,
            SourceKind::LocalPath,
        ))
        .expect("root handle");
        let entry = EntryRequest::resolve(&volume_uid, &acl_layout_entry()).expect("entry");
        d2b_provider_volume_local::testing::block_on(adapter.apply_acl(&handle, &entry))
            .expect("acl applied");

        let expected_access = acl_entries(
            0o770,
            vec![
                AclEntry {
                    tag: ACL_USER_TAG,
                    perm: ACL_READ | ACL_WRITE | ACL_EXECUTE,
                    id: own + 1,
                },
                AclEntry {
                    tag: ACL_USER_TAG,
                    perm: ACL_READ | ACL_EXECUTE,
                    id: own + 2,
                },
            ],
        );
        assert_eq!(
            read_xattr(dir.path(), POSIX_ACL_ACCESS_XATTR).expect("access acl xattr"),
            acl_xattr_bytes(&expected_access)
        );
        let expected_default = acl_entries(
            0o770,
            vec![
                AclEntry {
                    tag: ACL_USER_TAG,
                    perm: ACL_READ | ACL_WRITE | ACL_EXECUTE,
                    id: own + 1,
                },
                AclEntry {
                    tag: ACL_USER_TAG,
                    perm: ACL_READ | ACL_WRITE,
                    id: own + 2,
                },
            ],
        );
        assert_eq!(
            read_xattr(dir.path(), POSIX_ACL_DEFAULT_XATTR).expect("default acl xattr"),
            acl_xattr_bytes(&expected_default)
        );
        // The kernel mirrors the mask into the group bits.
        let mode = std::fs::metadata(dir.path())
            .expect("stat root")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o770);
    }

    /// Entries without declared grants are a no-op, and an unresolvable
    /// principal is a spec failure rather than a silent skip.
    #[test]
    fn acl_application_fails_closed_on_unresolved_principals() {
        let dir = tempfile::tempdir().expect("volume root");
        let root = File::open(dir.path()).expect("open root");
        let volume_uid =
            ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").expect("volume UID");
        let resolver = NamedUidResolver {
            root: Arc::new(root.into()),
            volume_uid: volume_uid.clone(),
            uids: std::collections::BTreeMap::from([("owner".to_owned(), Uid::current().as_raw())]),
        };
        let adapter = AnchoredVolumeEffectAdapter::new(resolver);
        let handle = d2b_provider_volume_local::testing::block_on(adapter.resolve_root_for(
            &volume_uid,
            None,
            None,
            SourceKind::LocalPath,
        ))
        .expect("root handle");
        let mut rendered = serde_json::to_value(acl_layout_entry()).expect("entry renders");
        rendered["accessAcl"] =
            serde_json::json!([{ "principal": { "ref": "User/absent" }, "permissions": "rx" }]);
        let entry = EntryRequest::resolve(
            &volume_uid,
            &serde_json::from_value(rendered).expect("entry stays valid"),
        )
        .expect("entry");
        assert!(
            d2b_provider_volume_local::testing::block_on(adapter.apply_acl(&handle, &entry))
                .is_err()
        );
    }
}
