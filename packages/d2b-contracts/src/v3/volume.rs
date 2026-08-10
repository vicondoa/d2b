//! Volume primitive ResourceType base spec.
//!
//! `Volume` is the single shareable-storage ResourceType. It folds the
//! fine-grained layout, ownership, ACL, lifecycle, named-view, and same-Zone
//! Host or Guest attachment policy that separate file, directory, ACL, and
//! filesystem-view types would otherwise carry.
//!
//! `source.settings` never carries a raw host path in the authored spec: the
//! `local-path` and `block-image` source kinds name an opaque bounded
//! `sourcePolicyId` that resolves, only inside the Volume Provider's private
//! authority, against that Provider's allowlisted root policy. Layout paths
//! are relative to the anchored Volume root, and ACL principals are always
//! typed `User/<name>` references; no numeric UID or GID form is accepted.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};

use super::{
    ResourceRef,
    execution_policy::{
        BoundedToken, PrimitiveSpecError, redacted_debug, require_execution_ref,
        require_resource_type,
    },
    process::validate_octal_mode,
};

/// The canonical ResourceType name for this module.
pub const VOLUME_RESOURCE_TYPE: &str = "Volume";
/// Maximum layout entries on one Volume.
pub const MAX_LAYOUT_ENTRIES: usize = 1024;
/// Maximum named views on one Volume.
pub const MAX_VIEWS: usize = 64;
/// Maximum attachments on one Volume.
pub const MAX_ATTACHMENTS: usize = 64;
/// Maximum bytes in one anchored relative layout path.
pub const MAX_LAYOUT_PATH_BYTES: usize = 255;
/// Maximum ACL grants on one layout entry list.
pub const MAX_ACL_GRANTS: usize = 64;
/// Maximum virtiofs worker threads requested by one attachment.
pub const MAX_THREAD_POOL_SIZE: u32 = 256;

/// Semantic persistence class of a Volume.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum VolumeKind {
    Durable,
    Ephemeral,
    State,
    Tmp,
    Cache,
}

/// Backing class of the Volume source.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum SourceKind {
    LocalPath,
    BlockImage,
    Tmpfs,
}

/// On-disk format managed for a `block-image` source.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum BlockImageFormat {
    Raw,
    Qcow2,
}

/// The Volume base source settings.
///
/// `sourcePolicyId` is an opaque bounded ID, never a raw host path.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SourceSettings {
    kind: SourceKind,
    source_policy_id: Option<BoundedToken>,
    #[serde(skip_serializing_if = "Option::is_none")]
    image_format: Option<BlockImageFormat>,
    #[serde(default, skip_serializing_if = "is_false")]
    preallocate: bool,
}

impl SourceSettings {
    /// Construct source settings, requiring the opaque policy ID exactly for
    /// the two host-backed source kinds and rejecting it for `tmpfs`.
    pub fn new(
        kind: SourceKind,
        source_policy_id: Option<BoundedToken>,
    ) -> Result<Self, PrimitiveSpecError> {
        Self::new_with_image(kind, source_policy_id, None, false)
    }

    /// Construct source settings with the block-image lifecycle options.
    pub fn new_with_image(
        kind: SourceKind,
        source_policy_id: Option<BoundedToken>,
        image_format: Option<BlockImageFormat>,
        preallocate: bool,
    ) -> Result<Self, PrimitiveSpecError> {
        let required = matches!(kind, SourceKind::LocalPath | SourceKind::BlockImage);
        match (required, source_policy_id.is_some()) {
            (true, false) => Err(PrimitiveSpecError::MissingRequiredField),
            (false, true) => Err(PrimitiveSpecError::ConflictingFields),
            _ if kind != SourceKind::BlockImage && (image_format.is_some() || preallocate) => {
                Err(PrimitiveSpecError::ConflictingFields)
            }
            _ => Ok(Self {
                kind,
                source_policy_id,
                image_format,
                preallocate,
            }),
        }
    }

    /// Return the backing class.
    pub const fn kind(&self) -> SourceKind {
        self.kind
    }

    /// Borrow the opaque allowlist policy ID.
    pub const fn source_policy_id(&self) -> Option<&BoundedToken> {
        self.source_policy_id.as_ref()
    }

    /// Return the selected block-image format, defaulting to raw.
    pub const fn image_format(&self) -> BlockImageFormat {
        match self.image_format {
            Some(format) => format,
            None => BlockImageFormat::Raw,
        }
    }

    /// Whether a block-image is preallocated at creation.
    pub const fn preallocate(&self) -> bool {
        self.preallocate
    }
}

redacted_debug!(SourceSettings);

impl<'de> Deserialize<'de> for SourceSettings {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            kind: SourceKind,
            #[serde(default)]
            source_policy_id: Option<BoundedToken>,
            #[serde(default)]
            image_format: Option<BlockImageFormat>,
            #[serde(default)]
            preallocate: bool,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new_with_image(
            wire.kind,
            wire.source_policy_id,
            wire.image_format,
            wire.preallocate,
        )
        .map_err(serde::de::Error::custom)
    }
}

const fn is_false(value: &bool) -> bool {
    !*value
}

/// Where the backing storage lives, and how it is selected.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct VolumeSource {
    execution_ref: ResourceRef,
    settings: SourceSettings,
}

impl VolumeSource {
    /// Construct a Volume source after checking the execution reference type.
    pub fn new(
        execution_ref: ResourceRef,
        settings: SourceSettings,
    ) -> Result<Self, PrimitiveSpecError> {
        require_execution_ref(&execution_ref)?;
        Ok(Self {
            execution_ref,
            settings,
        })
    }

    /// Borrow the Host or Guest that holds the backing storage.
    pub const fn execution_ref(&self) -> &ResourceRef {
        &self.execution_ref
    }

    /// Borrow the base source settings.
    pub const fn settings(&self) -> &SourceSettings {
        &self.settings
    }
}

redacted_debug!(VolumeSource);

impl<'de> Deserialize<'de> for VolumeSource {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            execution_ref: ResourceRef,
            settings: SourceSettings,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.execution_ref, wire.settings).map_err(serde::de::Error::custom)
    }
}

/// Layout entry class.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum EntryType {
    Directory,
    File,
    Symlink,
    UnixSocket,
}

/// Audit and log handling class of one layout entry.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum SensitivityClass {
    Public,
    Private,
    /// State whose path and size are restricted to the broker audit trail.
    SecretAdjacent,
    /// A tamper-evident audit segment.
    Audit,
    /// State whose visibility is bounded by the Zone boundary.
    ZoneScoped,
    /// Compatibility spelling retained for already-landed fixtures.
    Secret,
}

/// When a layout entry is created.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum CreatePolicy {
    CreateIfAbsent,
    CreateIfNeverProvisioned,
    AlwaysRecreate,
    ObserveOnly,
}

/// How drift from the declared layout state is reconciled.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum RepairPolicy {
    None,
    NixActivation,
    ExactOwner,
    FailClosed,
    OperatorOnly,
    /// Compatibility policy for callers that repair only mode bits.
    ExactMode,
    /// Compatibility policy for callers that repair owner and ACLs.
    ExactOwnerAndAcl,
}

/// When a layout entry is removed.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum CleanupPolicy {
    Never,
    Boot,
    ProcessExitWithProof,
    VmStopWithProof,
    CutoverOnly,
    External,
    /// Compatibility spelling for controller-owned cleanup.
    OwnerControlled,
    /// Compatibility spelling for process-scoped cleanup.
    ProcessExit,
}

/// How an existing entry is treated on first bind.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum EntryAdoptionPolicy {
    AdoptWithLiveOwnerProof,
    RecreateFromPersistent,
    QuarantineOnAmbiguity,
    DeleteIfOwnerDead,
    NotAdoptable,
    /// Compatibility spelling for a non-adoptable entry.
    NeverAdopt,
}

/// Behavior across a Volume controller restart.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum EntryRestartPolicy {
    PreserveAcrossControllerRestart,
    RecreateAfterOwnerDeath,
    CleanupAfterOwnerDeath,
    ManualRecovery,
    NotApplicable,
    /// Compatibility spelling for restart recreation.
    RecreateOnControllerRestart,
}

/// Type of live-ownership lease checked during adoption.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum LeaseClass {
    None,
    ProcessPidfd,
    CgroupLeaf,
    FileRecord,
    External,
    /// Compatibility spelling for a controller-owned lock.
    ControllerLock,
}

/// Additional fail-closed layout check.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum Invariant {
    NoSymlink,
    NoMagicLink,
    NoRecursiveMutation,
    SameFilesystem,
    HardlinkFarmNoRecursion,
    BrokerOpaqueIdOnly,
    RootOwnedParent,
    ScopeAuthorizationRequired,
}

/// How unlisted ACL entries on a directory's children are treated.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum ForeignChildPolicy {
    Preserve,
    Fail,
}

/// One named ACL principal.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AclPrincipal {
    #[serde(rename = "ref")]
    reference: ResourceRef,
}

impl AclPrincipal {
    /// Construct a principal after checking that it names a `User`.
    pub fn new(reference: ResourceRef) -> Result<Self, PrimitiveSpecError> {
        require_resource_type(&reference, "User")?;
        Ok(Self { reference })
    }

    /// Borrow the principal reference.
    pub const fn reference(&self) -> &ResourceRef {
        &self.reference
    }
}

redacted_debug!(AclPrincipal);

impl<'de> Deserialize<'de> for AclPrincipal {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            #[serde(rename = "ref")]
            reference: ResourceRef,
        }
        Self::new(Wire::deserialize(deserializer)?.reference).map_err(serde::de::Error::custom)
    }
}

/// One POSIX ACL grant.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AclGrant {
    principal: AclPrincipal,
    permissions: String,
}

impl AclGrant {
    /// Construct a grant after checking the permission spelling.
    pub fn new(
        principal: AclPrincipal,
        permissions: impl Into<String>,
    ) -> Result<Self, PrimitiveSpecError> {
        let permissions = permissions.into();
        if permissions.is_empty()
            || permissions.len() > 3
            || permissions
                .bytes()
                .any(|byte| !matches!(byte, b'r' | b'w' | b'x'))
        {
            return Err(PrimitiveSpecError::InvalidText);
        }
        Ok(Self {
            principal,
            permissions,
        })
    }

    /// Borrow the granted principal.
    pub const fn principal(&self) -> &AclPrincipal {
        &self.principal
    }

    /// Borrow the POSIX permission string.
    pub fn permissions(&self) -> &str {
        &self.permissions
    }
}

redacted_debug!(AclGrant);

impl<'de> Deserialize<'de> for AclGrant {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            principal: AclPrincipal,
            permissions: String,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.principal, wire.permissions).map_err(serde::de::Error::custom)
    }
}

/// One anchored layout entry relative to the Volume root.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LayoutEntry {
    path: String,
    #[serde(rename = "type")]
    entry_type: EntryType,
    owner_ref: ResourceRef,
    group_ref: ResourceRef,
    mode: String,
    target: Option<String>,
    access_acl: Vec<AclGrant>,
    default_acl: Vec<AclGrant>,
    foreign_child_policy: ForeignChildPolicy,
    no_follow: bool,
    recursive: bool,
    sensitivity: SensitivityClass,
    create_policy: CreatePolicy,
    repair_policy: RepairPolicy,
    cleanup_policy: CleanupPolicy,
    adoption_policy: EntryAdoptionPolicy,
    restart_policy: EntryRestartPolicy,
    lease_class: LeaseClass,
    invariants: Vec<Invariant>,
}

impl LayoutEntry {
    /// Construct a layout entry after checking every anchored-path, typed
    /// principal, and symlink invariant.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        path: impl Into<String>,
        entry_type: EntryType,
        owner_ref: ResourceRef,
        group_ref: ResourceRef,
        mode: impl Into<String>,
        target: Option<String>,
        access_acl: Vec<AclGrant>,
        default_acl: Vec<AclGrant>,
        foreign_child_policy: ForeignChildPolicy,
        no_follow: bool,
        recursive: bool,
        sensitivity: SensitivityClass,
        create_policy: CreatePolicy,
        repair_policy: RepairPolicy,
        cleanup_policy: CleanupPolicy,
        adoption_policy: EntryAdoptionPolicy,
        restart_policy: EntryRestartPolicy,
        lease_class: LeaseClass,
        invariants: Vec<Invariant>,
    ) -> Result<Self, PrimitiveSpecError> {
        let path = path.into();
        validate_anchored_path(&path)?;
        require_resource_type(&owner_ref, "User")?;
        require_resource_type(&group_ref, "User")?;
        let mode = mode.into();
        validate_octal_mode(&mode, 4, 4)?;
        let mut invariants = invariants;
        let declared_invariants = invariants.len();
        invariants.sort_unstable();
        invariants.dedup();
        if invariants.len() != declared_invariants {
            return Err(PrimitiveSpecError::DuplicateEntry);
        }
        if recursive
            && !matches!(
                repair_policy,
                RepairPolicy::ExactOwner
                    | RepairPolicy::FailClosed
                    | RepairPolicy::ExactOwnerAndAcl
            )
        {
            return Err(PrimitiveSpecError::ConflictingFields);
        }
        if recursive
            && invariants.iter().any(|invariant| {
                matches!(
                    invariant,
                    Invariant::NoRecursiveMutation | Invariant::HardlinkFarmNoRecursion
                )
            })
        {
            return Err(PrimitiveSpecError::ConflictingFields);
        }
        if matches!(
            cleanup_policy,
            CleanupPolicy::ProcessExitWithProof | CleanupPolicy::ProcessExit
        ) && !matches!(
            lease_class,
            LeaseClass::ProcessPidfd | LeaseClass::CgroupLeaf
        ) {
            return Err(PrimitiveSpecError::ConflictingFields);
        }
        if cleanup_policy == CleanupPolicy::VmStopWithProof && lease_class != LeaseClass::CgroupLeaf
        {
            return Err(PrimitiveSpecError::ConflictingFields);
        }
        if lease_class == LeaseClass::FileRecord
            && (entry_type != EntryType::File || cleanup_policy != CleanupPolicy::Never)
        {
            return Err(PrimitiveSpecError::ConflictingFields);
        }
        if create_policy == CreatePolicy::AlwaysRecreate
            && !(matches!(
                lease_class,
                LeaseClass::ProcessPidfd | LeaseClass::CgroupLeaf
            ) && matches!(
                cleanup_policy,
                CleanupPolicy::ProcessExitWithProof | CleanupPolicy::ProcessExit
            ))
        {
            return Err(PrimitiveSpecError::ConflictingFields);
        }
        match (entry_type, &target) {
            (EntryType::Symlink, None) => return Err(PrimitiveSpecError::MissingRequiredField),
            (EntryType::Symlink, Some(target)) => {
                validate_anchored_path(target)?;
                if target.is_empty() {
                    return Err(PrimitiveSpecError::InvalidPath);
                }
                if no_follow {
                    return Err(PrimitiveSpecError::ConflictingFields);
                }
            }
            (_, Some(_)) => return Err(PrimitiveSpecError::ConflictingFields),
            (_, None) => {
                if !no_follow {
                    return Err(PrimitiveSpecError::ConflictingFields);
                }
            }
        }
        if access_acl.len() > MAX_ACL_GRANTS || default_acl.len() > MAX_ACL_GRANTS {
            return Err(PrimitiveSpecError::TooManyEntries);
        }
        Ok(Self {
            path,
            entry_type,
            owner_ref,
            group_ref,
            mode,
            target,
            access_acl,
            default_acl,
            foreign_child_policy,
            no_follow,
            recursive,
            sensitivity,
            create_policy,
            repair_policy,
            cleanup_policy,
            adoption_policy,
            restart_policy,
            lease_class,
            invariants,
        })
    }

    /// Construct the canonical Volume-root directory entry.
    pub fn root_directory(
        owner_ref: ResourceRef,
        group_ref: ResourceRef,
        mode: impl Into<String>,
    ) -> Result<Self, PrimitiveSpecError> {
        Self::new(
            String::new(),
            EntryType::Directory,
            owner_ref,
            group_ref,
            mode,
            None,
            Vec::new(),
            Vec::new(),
            ForeignChildPolicy::Preserve,
            true,
            false,
            SensitivityClass::Private,
            CreatePolicy::CreateIfAbsent,
            RepairPolicy::ExactOwner,
            CleanupPolicy::Never,
            EntryAdoptionPolicy::AdoptWithLiveOwnerProof,
            EntryRestartPolicy::PreserveAcrossControllerRestart,
            LeaseClass::None,
            vec![Invariant::NoSymlink],
        )
    }

    /// Borrow the anchored relative path.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Return the entry class.
    pub const fn entry_type(&self) -> EntryType {
        self.entry_type
    }

    /// Borrow the typed owning principal.
    pub const fn owner_ref(&self) -> &ResourceRef {
        &self.owner_ref
    }

    /// Borrow the typed owning group principal.
    pub const fn group_ref(&self) -> &ResourceRef {
        &self.group_ref
    }

    /// Borrow the four-digit octal mode.
    pub fn mode(&self) -> &str {
        &self.mode
    }

    /// Borrow the symlink target.
    pub fn target(&self) -> Option<&str> {
        self.target.as_deref()
    }
}

redacted_debug!(LayoutEntry);

impl<'de> Deserialize<'de> for LayoutEntry {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            path: String,
            #[serde(rename = "type")]
            entry_type: EntryType,
            owner_ref: ResourceRef,
            group_ref: ResourceRef,
            mode: String,
            #[serde(default)]
            target: Option<String>,
            #[serde(default)]
            access_acl: Vec<AclGrant>,
            #[serde(default)]
            default_acl: Vec<AclGrant>,
            #[serde(default = "preserve")]
            foreign_child_policy: ForeignChildPolicy,
            #[serde(default = "yes")]
            no_follow: bool,
            #[serde(default)]
            recursive: bool,
            #[serde(default = "private")]
            sensitivity: SensitivityClass,
            #[serde(default = "create_if_absent")]
            create_policy: CreatePolicy,
            #[serde(default = "exact_owner")]
            repair_policy: RepairPolicy,
            #[serde(default = "never")]
            cleanup_policy: CleanupPolicy,
            #[serde(default = "adopt_with_live_owner_proof")]
            adoption_policy: EntryAdoptionPolicy,
            #[serde(default = "preserve_across_controller_restart")]
            restart_policy: EntryRestartPolicy,
            #[serde(default = "lease_none")]
            lease_class: LeaseClass,
            #[serde(default = "no_symlink")]
            invariants: Vec<Invariant>,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(
            wire.path,
            wire.entry_type,
            wire.owner_ref,
            wire.group_ref,
            wire.mode,
            wire.target,
            wire.access_acl,
            wire.default_acl,
            wire.foreign_child_policy,
            wire.no_follow,
            wire.recursive,
            wire.sensitivity,
            wire.create_policy,
            wire.repair_policy,
            wire.cleanup_policy,
            wire.adoption_policy,
            wire.restart_policy,
            wire.lease_class,
            wire.invariants,
        )
        .map_err(serde::de::Error::custom)
    }
}

/// One right granted by a named view.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum ViewRight {
    Read,
    Write,
    Create,
    Delete,
    Traverse,
    Execute,
}

/// One named view onto a Volume subtree.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ViewSpec {
    path: String,
    rights: Vec<ViewRight>,
}

impl ViewSpec {
    /// Construct a view after checking its anchored path and unique rights.
    pub fn new(
        path: impl Into<String>,
        mut rights: Vec<ViewRight>,
    ) -> Result<Self, PrimitiveSpecError> {
        let path = path.into();
        validate_anchored_path(&path)?;
        if rights.is_empty() {
            return Err(PrimitiveSpecError::MissingRequiredField);
        }
        let declared = rights.len();
        rights.sort_unstable();
        rights.dedup();
        if rights.len() != declared {
            return Err(PrimitiveSpecError::DuplicateEntry);
        }
        Ok(Self { path, rights })
    }

    /// Borrow the anchored subtree path.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Borrow the granted rights.
    pub fn rights(&self) -> &[ViewRight] {
        &self.rights
    }
}

redacted_debug!(ViewSpec);

impl<'de> Deserialize<'de> for ViewSpec {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            path: String,
            rights: Vec<ViewRight>,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.path, wire.rights).map_err(serde::de::Error::custom)
    }
}

/// Transport of one Volume attachment.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum AttachmentTransport {
    Virtiofs,
    VirtioBlk,
}

/// Access level of one Volume attachment.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum AttachmentAccess {
    ReadOnly,
    ReadWrite,
    SharedWrite,
}

/// Page-cache behavior of one attachment.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum AttachmentCache {
    Auto,
    Always,
    Never,
}

/// Inode file-handle behavior of one attachment.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum InodeFileHandles {
    Never,
    Prefer,
    Mandatory,
}

/// The typed base attachment options.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentSettings {
    posix_acl: bool,
    xattr: bool,
    cache: AttachmentCache,
    inode_file_handles: InodeFileHandles,
    thread_pool_size: Option<u32>,
    socket_group: Option<BoundedToken>,
}

impl AttachmentSettings {
    /// Construct attachment options after checking the thread-pool bound.
    pub fn new(
        posix_acl: bool,
        xattr: bool,
        cache: AttachmentCache,
        inode_file_handles: InodeFileHandles,
        thread_pool_size: Option<u32>,
        socket_group: Option<BoundedToken>,
    ) -> Result<Self, PrimitiveSpecError> {
        if thread_pool_size.is_some_and(|size| size == 0 || size > MAX_THREAD_POOL_SIZE) {
            return Err(PrimitiveSpecError::OutOfRange);
        }
        Ok(Self {
            posix_acl,
            xattr,
            cache,
            inode_file_handles,
            thread_pool_size,
            socket_group,
        })
    }

    /// Return the page-cache behavior.
    pub const fn cache(&self) -> AttachmentCache {
        self.cache
    }

    /// Return the inode file-handle behavior.
    pub const fn inode_file_handles(&self) -> InodeFileHandles {
        self.inode_file_handles
    }
}

impl Default for AttachmentSettings {
    fn default() -> Self {
        Self {
            posix_acl: false,
            xattr: false,
            cache: AttachmentCache::Auto,
            inode_file_handles: InodeFileHandles::Never,
            thread_pool_size: None,
            socket_group: None,
        }
    }
}

redacted_debug!(AttachmentSettings);

impl<'de> Deserialize<'de> for AttachmentSettings {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            #[serde(default)]
            posix_acl: bool,
            #[serde(default)]
            xattr: bool,
            #[serde(default = "cache_auto")]
            cache: AttachmentCache,
            #[serde(default = "handles_never")]
            inode_file_handles: InodeFileHandles,
            #[serde(default)]
            thread_pool_size: Option<u32>,
            #[serde(default)]
            socket_group: Option<BoundedToken>,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(
            wire.posix_acl,
            wire.xattr,
            wire.cache,
            wire.inode_file_handles,
            wire.thread_pool_size,
            wire.socket_group,
        )
        .map_err(serde::de::Error::custom)
    }
}

/// One same-Zone Host or Guest attachment.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct VolumeAttachment {
    execution_ref: ResourceRef,
    transport: AttachmentTransport,
    view: BoundedToken,
    access: AttachmentAccess,
    mount_path: String,
    settings: AttachmentSettings,
}

impl VolumeAttachment {
    /// Construct an attachment after checking the execution reference and
    /// guest-side mount path.
    pub fn new(
        execution_ref: ResourceRef,
        transport: AttachmentTransport,
        view: BoundedToken,
        access: AttachmentAccess,
        mount_path: impl Into<String>,
        settings: AttachmentSettings,
    ) -> Result<Self, PrimitiveSpecError> {
        require_execution_ref(&execution_ref)?;
        let mount_path = mount_path.into();
        if !validate_mount_path(&mount_path) {
            return Err(PrimitiveSpecError::InvalidPath);
        }
        Ok(Self {
            execution_ref,
            transport,
            view,
            access,
            mount_path,
            settings,
        })
    }

    /// Borrow the target Host or Guest.
    pub const fn execution_ref(&self) -> &ResourceRef {
        &self.execution_ref
    }

    /// Return the attachment transport.
    pub const fn transport(&self) -> AttachmentTransport {
        self.transport
    }

    /// Borrow the selected view name.
    pub const fn view(&self) -> &BoundedToken {
        &self.view
    }

    /// Return the attachment access level.
    pub const fn access(&self) -> AttachmentAccess {
        self.access
    }

    /// Borrow the guest-side mount path.
    pub fn mount_path(&self) -> &str {
        &self.mount_path
    }

    /// Borrow the typed base attachment options.
    pub const fn settings(&self) -> &AttachmentSettings {
        &self.settings
    }
}

redacted_debug!(VolumeAttachment);

impl<'de> Deserialize<'de> for VolumeAttachment {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            execution_ref: ResourceRef,
            transport: AttachmentTransport,
            view: BoundedToken,
            #[serde(default = "attachment_read_only")]
            access: AttachmentAccess,
            mount_path: String,
            #[serde(default)]
            settings: AttachmentSettings,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(
            wire.execution_ref,
            wire.transport,
            wire.view,
            wire.access,
            wire.mount_path,
            wire.settings,
        )
        .map_err(serde::de::Error::custom)
    }
}

/// Whether the backing filesystem must enforce the declared quota.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum QuotaEnforcement {
    None,
    Hard,
}

/// Storage limits for a Volume.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct QuotaSpec {
    max_bytes: Option<u64>,
    max_inodes: Option<u64>,
    enforcement: QuotaEnforcement,
}

impl QuotaSpec {
    /// Construct a quota, requiring both limits for hard enforcement.
    pub fn new(
        max_bytes: Option<u64>,
        max_inodes: Option<u64>,
        enforcement: QuotaEnforcement,
    ) -> Result<Self, PrimitiveSpecError> {
        if enforcement == QuotaEnforcement::Hard && (max_bytes.is_none() || max_inodes.is_none()) {
            return Err(PrimitiveSpecError::MissingRequiredField);
        }
        if max_bytes == Some(0) || max_inodes == Some(0) {
            return Err(PrimitiveSpecError::OutOfRange);
        }
        Ok(Self {
            max_bytes,
            max_inodes,
            enforcement,
        })
    }

    /// Return the byte ceiling.
    pub const fn max_bytes(&self) -> Option<u64> {
        self.max_bytes
    }

    /// Return the inode ceiling.
    pub const fn max_inodes(&self) -> Option<u64> {
        self.max_inodes
    }

    /// Return the enforcement class.
    pub const fn enforcement(&self) -> QuotaEnforcement {
        self.enforcement
    }
}

redacted_debug!(QuotaSpec);

impl<'de> Deserialize<'de> for QuotaSpec {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            #[serde(default)]
            max_bytes: Option<u64>,
            #[serde(default)]
            max_inodes: Option<u64>,
            #[serde(default = "quota_none")]
            enforcement: QuotaEnforcement,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.max_bytes, wire.max_inodes, wire.enforcement)
            .map_err(serde::de::Error::custom)
    }
}

/// The Volume ResourceType base spec.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct VolumeSpec {
    source: VolumeSource,
    kind: VolumeKind,
    layout: Vec<LayoutEntry>,
    views: BTreeMap<String, ViewSpec>,
    attachments: Vec<VolumeAttachment>,
    quota: Option<QuotaSpec>,
}

impl VolumeSpec {
    /// Construct a Volume base spec after checking every frozen bound and
    /// cross-field invariant.
    pub fn new(
        source: VolumeSource,
        kind: VolumeKind,
        layout: Vec<LayoutEntry>,
        views: BTreeMap<String, ViewSpec>,
        attachments: Vec<VolumeAttachment>,
        quota: Option<QuotaSpec>,
    ) -> Result<Self, PrimitiveSpecError> {
        if layout.len() > MAX_LAYOUT_ENTRIES
            || views.len() > MAX_VIEWS
            || attachments.len() > MAX_ATTACHMENTS
        {
            return Err(PrimitiveSpecError::TooManyEntries);
        }
        if views.is_empty() {
            return Err(PrimitiveSpecError::MissingRequiredField);
        }
        for name in views.keys() {
            BoundedToken::parse(name.clone())?;
        }
        let mut paths: Vec<&str> = layout.iter().map(LayoutEntry::path).collect();
        let declared = paths.len();
        paths.sort_unstable();
        paths.dedup();
        if paths.len() != declared {
            return Err(PrimitiveSpecError::DuplicateEntry);
        }
        for attachment in &attachments {
            if !views.contains_key(attachment.view.as_str()) {
                return Err(PrimitiveSpecError::MissingRequiredField);
            }
            let view = views
                .get(attachment.view.as_str())
                .ok_or(PrimitiveSpecError::MissingRequiredField)?;
            if attachment.access != AttachmentAccess::ReadOnly
                && !view.rights.contains(&ViewRight::Write)
            {
                return Err(PrimitiveSpecError::ConflictingFields);
            }
            let source_kind = source.settings().kind();
            let transport_matches_source = match source_kind {
                SourceKind::BlockImage => attachment.transport == AttachmentTransport::VirtioBlk,
                SourceKind::LocalPath | SourceKind::Tmpfs => {
                    attachment.transport == AttachmentTransport::Virtiofs
                }
            };
            if !transport_matches_source {
                return Err(PrimitiveSpecError::ConflictingFields);
            }
        }
        if attachments
            .iter()
            .filter(|attachment| attachment.access == AttachmentAccess::ReadWrite)
            .count()
            > 1
        {
            return Err(PrimitiveSpecError::ConflictingFields);
        }
        let mut execution_refs: Vec<String> = attachments
            .iter()
            .map(|attachment| attachment.execution_ref.to_canonical_string())
            .collect();
        let declared_execution_refs = execution_refs.len();
        execution_refs.sort_unstable();
        execution_refs.dedup();
        if execution_refs.len() != declared_execution_refs {
            return Err(PrimitiveSpecError::DuplicateEntry);
        }
        if source.settings().kind() == SourceKind::Tmpfs {
            if !matches!(kind, VolumeKind::Ephemeral | VolumeKind::Tmp) {
                return Err(PrimitiveSpecError::ConflictingFields);
            }
            let quota = quota.ok_or(PrimitiveSpecError::MissingRequiredField)?;
            if quota.max_bytes().is_none() || quota.max_inodes().is_none() {
                return Err(PrimitiveSpecError::MissingRequiredField);
            }
            if quota.enforcement() != QuotaEnforcement::Hard {
                return Err(PrimitiveSpecError::ConflictingFields);
            }
        }
        if source.settings().kind() == SourceKind::BlockImage
            && quota.as_ref().and_then(QuotaSpec::max_bytes).is_none()
        {
            return Err(PrimitiveSpecError::MissingRequiredField);
        }
        Ok(Self {
            source,
            kind,
            layout,
            views,
            attachments,
            quota,
        })
    }

    /// Borrow the Volume source.
    pub const fn source(&self) -> &VolumeSource {
        &self.source
    }

    /// Return the semantic persistence class.
    pub const fn kind(&self) -> VolumeKind {
        self.kind
    }

    /// Borrow the anchored layout entries.
    pub fn layout(&self) -> &[LayoutEntry] {
        &self.layout
    }

    /// Borrow the named views.
    pub const fn views(&self) -> &BTreeMap<String, ViewSpec> {
        &self.views
    }

    /// Borrow the same-Zone attachments.
    pub fn attachments(&self) -> &[VolumeAttachment] {
        &self.attachments
    }

    /// Borrow the declared quota.
    pub const fn quota(&self) -> Option<&QuotaSpec> {
        self.quota.as_ref()
    }
}

redacted_debug!(VolumeSpec);

impl<'de> Deserialize<'de> for VolumeSpec {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            source: VolumeSource,
            kind: VolumeKind,
            #[serde(default)]
            layout: Vec<LayoutEntry>,
            views: BTreeMap<String, ViewSpec>,
            #[serde(default)]
            attachments: Vec<VolumeAttachment>,
            #[serde(default)]
            quota: Option<QuotaSpec>,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(
            wire.source,
            wire.kind,
            wire.layout,
            wire.views,
            wire.attachments,
            wire.quota,
        )
        .map_err(serde::de::Error::custom)
    }
}

/// Accepts one anchored path in a single normal form.
///
/// Containment is the obvious job: a leading separator, a `..` segment, a
/// backslash or an embedded NUL is refused, so a layout entry cannot name a
/// location outside its Volume.
///
/// Requiring a normal form is the less obvious one, and it is what makes the
/// per-path uniqueness check downstream mean anything. Entry uniqueness is an
/// exact string comparison, so admitting `state`, `./state` and `state/` would
/// admit three distinct entries that resolve to one host path and may carry
/// conflicting create, repair, cleanup or ACL settings. Refusing the redundant
/// spellings is what turns that comparison into a real check. The empty string
/// stays admitted: it names the Volume root, which the canonical minimal spec
/// uses.
fn validate_anchored_path(value: &str) -> Result<(), PrimitiveSpecError> {
    if value.len() > MAX_LAYOUT_PATH_BYTES
        || value.starts_with('/')
        || value.contains('\0')
        || value.contains('\\')
        || value.contains(':')
        || value.chars().any(is_path_separator_homoglyph)
    {
        return Err(PrimitiveSpecError::InvalidPath);
    }

    if value.is_empty() {
        return Ok(());
    }
    if value
        .split('/')
        .any(|segment| segment.is_empty() || segment == "." || segment == "..")
    {
        return Err(PrimitiveSpecError::InvalidPath);
    }
    Ok(())
}

fn is_path_separator_homoglyph(character: char) -> bool {
    matches!(
        character,
        '\u{2044}'
            | '\u{2215}'
            | '\u{29F8}'
            | '\u{2AF8}'
            | '\u{FF0F}'
            | '\u{FF3C}'
            | '\u{FE68}'
            | '\u{FF0E}'
            | '\u{2024}'
    )
}

fn validate_mount_path(value: &str) -> bool {
    if value.len() > MAX_LAYOUT_PATH_BYTES
        || !value.starts_with('/')
        || value.contains('\0')
        || value.contains('\\')
        || value.chars().any(is_path_separator_homoglyph)
    {
        return false;
    }
    if value == "/" {
        return true;
    }
    value
        .split('/')
        .skip(1)
        .all(|component| !component.is_empty() && component != "." && component != "..")
}

const fn yes() -> bool {
    true
}

const fn preserve() -> ForeignChildPolicy {
    ForeignChildPolicy::Preserve
}

const fn private() -> SensitivityClass {
    SensitivityClass::Private
}

const fn create_if_absent() -> CreatePolicy {
    CreatePolicy::CreateIfAbsent
}

const fn exact_owner() -> RepairPolicy {
    RepairPolicy::ExactOwner
}

const fn never() -> CleanupPolicy {
    CleanupPolicy::Never
}

const fn adopt_with_live_owner_proof() -> EntryAdoptionPolicy {
    EntryAdoptionPolicy::AdoptWithLiveOwnerProof
}

const fn preserve_across_controller_restart() -> EntryRestartPolicy {
    EntryRestartPolicy::PreserveAcrossControllerRestart
}

const fn lease_none() -> LeaseClass {
    LeaseClass::None
}

fn no_symlink() -> Vec<Invariant> {
    vec![Invariant::NoSymlink]
}

const fn cache_auto() -> AttachmentCache {
    AttachmentCache::Auto
}

const fn handles_never() -> InodeFileHandles {
    InodeFileHandles::Never
}

const fn attachment_read_only() -> AttachmentAccess {
    AttachmentAccess::ReadOnly
}

const fn quota_none() -> QuotaEnforcement {
    QuotaEnforcement::None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v3::{execution_policy::to_base_object, resource_schema::canonical_json_bytes};

    fn views() -> BTreeMap<String, ViewSpec> {
        BTreeMap::from([(
            "controller".to_owned(),
            ViewSpec::new(
                String::new(),
                vec![
                    ViewRight::Read,
                    ViewRight::Write,
                    ViewRight::Create,
                    ViewRight::Delete,
                    ViewRight::Traverse,
                ],
            )
            .unwrap(),
        )])
    }

    fn local_source() -> VolumeSource {
        VolumeSource::new(
            ResourceRef::parse("Host/host-system").unwrap(),
            SourceSettings::new(
                SourceKind::LocalPath,
                Some(BoundedToken::parse("state-root").unwrap()),
            )
            .unwrap(),
        )
        .unwrap()
    }

    fn minimal_volume() -> VolumeSpec {
        VolumeSpec::new(
            local_source(),
            VolumeKind::State,
            vec![
                LayoutEntry::root_directory(
                    ResourceRef::parse("User/example-system").unwrap(),
                    ResourceRef::parse("User/example-system").unwrap(),
                    "0700",
                )
                .unwrap(),
            ],
            views(),
            Vec::new(),
            None,
        )
        .unwrap()
    }

    const MINIMAL_VOLUME_SPEC: &[u8] = br#"{"attachments":[],"kind":"state","layout":[{"accessAcl":[],"adoptionPolicy":"adopt-with-live-owner-proof","cleanupPolicy":"never","createPolicy":"create-if-absent","defaultAcl":[],"foreignChildPolicy":"preserve","groupRef":"User/example-system","invariants":["no-symlink"],"leaseClass":"none","mode":"0700","noFollow":true,"ownerRef":"User/example-system","path":"","recursive":false,"repairPolicy":"exact-owner","restartPolicy":"preserve-across-controller-restart","sensitivity":"private","target":null,"type":"directory"}],"quota":null,"source":{"executionRef":"Host/host-system","settings":{"kind":"local-path","sourcePolicyId":"state-root"}},"views":{"controller":{"path":"","rights":["read","write","create","delete","traverse"]}}}"#;

    #[test]
    fn schema_vector_pins_the_minimal_volume_base_spec() {
        let spec = minimal_volume();
        assert_eq!(canonical_json_bytes(&spec).unwrap(), MINIMAL_VOLUME_SPEC);
        let parsed: VolumeSpec = serde_json::from_slice(MINIMAL_VOLUME_SPEC).unwrap();
        assert_eq!(parsed, spec);
    }

    #[test]
    fn no_raw_host_path_is_admitted_in_the_authored_source() {
        assert!(
            serde_json::from_slice::<SourceSettings>(
                br#"{"kind":"local-path","path":"/var/lib/d2b"}"#
            )
            .is_err()
        );
        assert_eq!(
            SourceSettings::new(SourceKind::LocalPath, None),
            Err(PrimitiveSpecError::MissingRequiredField)
        );
        assert_eq!(
            SourceSettings::new(
                SourceKind::Tmpfs,
                Some(BoundedToken::parse("state-root").unwrap())
            ),
            Err(PrimitiveSpecError::ConflictingFields)
        );
        let base = to_base_object(&minimal_volume()).unwrap();
        for reserved in ["providerRef", "updatePolicy", "provider"] {
            assert!(base.get(reserved).is_none());
        }
    }

    #[test]
    fn layout_paths_are_anchored_and_principals_are_typed() {
        for rejected in ["/state", "../state", "state\\x"] {
            assert_eq!(
                LayoutEntry::root_directory(
                    ResourceRef::parse("User/a").unwrap(),
                    ResourceRef::parse("User/a").unwrap(),
                    "0700",
                )
                .and_then(|_| LayoutEntry::new(
                    rejected,
                    EntryType::Directory,
                    ResourceRef::parse("User/a").unwrap(),
                    ResourceRef::parse("User/a").unwrap(),
                    "0700",
                    None,
                    Vec::new(),
                    Vec::new(),
                    ForeignChildPolicy::Preserve,
                    true,
                    false,
                    SensitivityClass::Private,
                    CreatePolicy::CreateIfAbsent,
                    RepairPolicy::ExactOwner,
                    CleanupPolicy::Never,
                    EntryAdoptionPolicy::AdoptWithLiveOwnerProof,
                    EntryRestartPolicy::PreserveAcrossControllerRestart,
                    LeaseClass::None,
                    Vec::new(),
                )),
                Err(PrimitiveSpecError::InvalidPath)
            );
        }
        assert_eq!(
            LayoutEntry::root_directory(
                ResourceRef::parse("Guest/a").unwrap(),
                ResourceRef::parse("User/a").unwrap(),
                "0700",
            ),
            Err(PrimitiveSpecError::WrongResourceType)
        );
        assert_eq!(
            LayoutEntry::root_directory(
                ResourceRef::parse("User/a").unwrap(),
                ResourceRef::parse("User/a").unwrap(),
                "700",
            ),
            Err(PrimitiveSpecError::InvalidMode)
        );
    }

    #[test]
    fn acl_principals_reject_a_numeric_identity() {
        assert!(
            serde_json::from_slice::<AclGrant>(
                br#"{"principal":{"uid":1000},"permissions":"rwx"}"#
            )
            .is_err()
        );
        assert_eq!(
            AclPrincipal::new(ResourceRef::parse("Provider/x").unwrap()),
            Err(PrimitiveSpecError::WrongResourceType)
        );
        assert!(
            AclGrant::new(
                AclPrincipal::new(ResourceRef::parse("User/a").unwrap()).unwrap(),
                "rwz",
            )
            .is_err()
        );
    }

    #[test]
    fn attachment_view_membership_and_single_writer_fail_closed() {
        let attachment = VolumeAttachment::new(
            ResourceRef::parse("Guest/work-vm").unwrap(),
            AttachmentTransport::Virtiofs,
            BoundedToken::parse("absent").unwrap(),
            AttachmentAccess::ReadWrite,
            "/state",
            AttachmentSettings::default(),
        )
        .unwrap();
        assert_eq!(
            VolumeSpec::new(
                local_source(),
                VolumeKind::State,
                Vec::new(),
                views(),
                vec![attachment],
                None
            ),
            Err(PrimitiveSpecError::MissingRequiredField)
        );

        let writer = |name: &str| {
            VolumeAttachment::new(
                ResourceRef::parse(name).unwrap(),
                AttachmentTransport::Virtiofs,
                BoundedToken::parse("controller").unwrap(),
                AttachmentAccess::ReadWrite,
                "/state",
                AttachmentSettings::default(),
            )
            .unwrap()
        };
        assert_eq!(
            VolumeSpec::new(
                local_source(),
                VolumeKind::State,
                Vec::new(),
                views(),
                vec![writer("Guest/a"), writer("Guest/b")],
                None,
            ),
            Err(PrimitiveSpecError::ConflictingFields)
        );
    }

    #[test]
    fn tmpfs_requires_both_hard_limits_and_a_boot_scoped_kind() {
        let tmpfs = || {
            VolumeSource::new(
                ResourceRef::parse("Host/host-system").unwrap(),
                SourceSettings::new(SourceKind::Tmpfs, None).unwrap(),
            )
            .unwrap()
        };
        assert_eq!(
            VolumeSpec::new(
                tmpfs(),
                VolumeKind::Durable,
                Vec::new(),
                views(),
                Vec::new(),
                None
            ),
            Err(PrimitiveSpecError::ConflictingFields)
        );
        assert_eq!(
            VolumeSpec::new(
                tmpfs(),
                VolumeKind::Ephemeral,
                Vec::new(),
                views(),
                Vec::new(),
                None
            ),
            Err(PrimitiveSpecError::MissingRequiredField)
        );
        assert!(
            VolumeSpec::new(
                tmpfs(),
                VolumeKind::Ephemeral,
                Vec::new(),
                views(),
                Vec::new(),
                Some(QuotaSpec::new(Some(1024), Some(64), QuotaEnforcement::Hard).unwrap()),
            )
            .is_ok()
        );
    }

    #[test]
    fn a_volume_must_declare_at_least_one_named_view() {
        assert_eq!(
            VolumeSpec::new(
                local_source(),
                VolumeKind::State,
                Vec::new(),
                BTreeMap::new(),
                Vec::new(),
                None
            ),
            Err(PrimitiveSpecError::MissingRequiredField)
        );
        assert_eq!(
            VolumeSpec::new(
                local_source(),
                VolumeKind::State,
                Vec::new(),
                BTreeMap::from([(
                    "Controller".to_owned(),
                    ViewSpec::new(String::new(), vec![ViewRight::Read]).unwrap()
                )]),
                Vec::new(),
                None,
            ),
            Err(PrimitiveSpecError::InvalidToken)
        );
    }

    #[test]
    fn diagnostics_stay_redacted() {
        assert_eq!(format!("{:?}", minimal_volume()), "VolumeSpec(<redacted>)");
    }

    /// An anchored path is admitted in exactly one spelling.
    ///
    /// Entry uniqueness is an exact string comparison, so a redundant
    /// spelling would be a second entry resolving to the same host path,
    /// free to carry a conflicting policy. The root stays admitted as the
    /// empty string because the canonical minimal spec uses it.
    #[test]
    fn an_anchored_path_is_admitted_in_one_normal_form_only() {
        for admitted in ["", "state", "state/tpm", "a/b/c"] {
            assert!(
                validate_anchored_path(admitted).is_ok(),
                "normal form rejected: {admitted:?}"
            );
        }
        for refused in [
            "/state",
            "./state",
            "state/",
            "state//tpm",
            "state/./tpm",
            "state/../tpm",
            "..",
            ".",
            "state\\tpm",
        ] {
            assert!(
                validate_anchored_path(refused).is_err(),
                "redundant or unsafe spelling admitted: {refused:?}"
            );
        }
    }

    #[test]
    fn anchored_and_mount_paths_reject_drive_separator_and_homoglyph_forms() {
        let owner = ResourceRef::parse("User/a").unwrap();
        for refused in ["C:state", "state\u{2215}data", "state\u{FF0F}data"] {
            assert!(
                LayoutEntry::new(
                    refused,
                    EntryType::Directory,
                    owner.clone(),
                    owner.clone(),
                    "0700",
                    None,
                    Vec::new(),
                    Vec::new(),
                    ForeignChildPolicy::Preserve,
                    true,
                    false,
                    SensitivityClass::Private,
                    CreatePolicy::CreateIfAbsent,
                    RepairPolicy::ExactOwner,
                    CleanupPolicy::Never,
                    EntryAdoptionPolicy::AdoptWithLiveOwnerProof,
                    EntryRestartPolicy::PreserveAcrossControllerRestart,
                    LeaseClass::None,
                    vec![Invariant::NoSymlink],
                )
                .is_err(),
                "unsafe anchored path accepted: {refused:?}"
            );
        }
        for refused in [
            "/state//data",
            "/state/./data",
            "/state/../data",
            "/state\\data",
        ] {
            assert!(
                VolumeAttachment::new(
                    ResourceRef::parse("Guest/work-vm").unwrap(),
                    AttachmentTransport::Virtiofs,
                    BoundedToken::parse("controller").unwrap(),
                    AttachmentAccess::ReadOnly,
                    refused,
                    AttachmentSettings::default(),
                )
                .is_err(),
                "unsafe mount path accepted: {refused:?}"
            );
        }
    }

    #[test]
    fn source_kind_transport_and_quota_cross_fields_are_strict() {
        let mut block = serde_json::to_value(minimal_volume()).unwrap();
        block["source"]["settings"]["kind"] = serde_json::json!("block-image");
        block["source"]["settings"]["sourcePolicyId"] = serde_json::json!("disk-root");
        assert!(serde_json::from_value::<VolumeSpec>(block.clone()).is_err());

        block["quota"] = serde_json::json!({ "maxBytes": 4096, "enforcement": "none" });
        let no_attachment: VolumeSpec =
            serde_json::from_value(block.clone()).expect("block image quota is sufficient");
        assert_eq!(
            no_attachment.source().settings().kind(),
            SourceKind::BlockImage
        );

        block["attachments"] = serde_json::json!([{
            "executionRef": "Guest/work-vm",
            "transport": "virtiofs",
            "view": "controller",
            "access": "read-only",
            "mountPath": "/disk"
        }]);
        assert!(serde_json::from_value::<VolumeSpec>(block).is_err());

        let mut tmpfs = serde_json::to_value(minimal_volume()).unwrap();
        tmpfs["source"]["settings"]["kind"] = serde_json::json!("tmpfs");
        tmpfs["source"]["settings"]
            .as_object_mut()
            .unwrap()
            .remove("sourcePolicyId");
        tmpfs["kind"] = serde_json::json!("tmp");
        tmpfs["quota"] =
            serde_json::json!({ "maxBytes": 4096, "maxInodes": 32, "enforcement": "none" });
        assert!(serde_json::from_value::<VolumeSpec>(tmpfs).is_err());
    }

    #[test]
    fn block_image_format_and_preallocation_stay_typed_and_source_scoped() {
        let settings = SourceSettings::new_with_image(
            SourceKind::BlockImage,
            Some(BoundedToken::parse("disk-root").unwrap()),
            Some(BlockImageFormat::Qcow2),
            true,
        )
        .unwrap();
        let rendered = serde_json::to_value(&settings).unwrap();
        assert_eq!(rendered["imageFormat"], "qcow2");
        assert_eq!(rendered["preallocate"], true);
        assert_eq!(settings.image_format(), BlockImageFormat::Qcow2);
        assert!(settings.preallocate());
        assert!(
            SourceSettings::new_with_image(
                SourceKind::LocalPath,
                Some(BoundedToken::parse("state-root").unwrap()),
                Some(BlockImageFormat::Raw),
                false,
            )
            .is_err()
        );
    }

    #[test]
    fn duplicate_invariants_and_empty_acl_permissions_are_rejected() {
        let owner = ResourceRef::parse("User/a").unwrap();
        assert_eq!(
            LayoutEntry::new(
                "state",
                EntryType::Directory,
                owner.clone(),
                owner.clone(),
                "0700",
                None,
                Vec::new(),
                Vec::new(),
                ForeignChildPolicy::Preserve,
                true,
                false,
                SensitivityClass::Private,
                CreatePolicy::CreateIfAbsent,
                RepairPolicy::ExactOwner,
                CleanupPolicy::Never,
                EntryAdoptionPolicy::AdoptWithLiveOwnerProof,
                EntryRestartPolicy::PreserveAcrossControllerRestart,
                LeaseClass::None,
                vec![Invariant::NoSymlink, Invariant::NoSymlink],
            ),
            Err(PrimitiveSpecError::DuplicateEntry)
        );
        let principal = AclPrincipal::new(owner).unwrap();
        assert_eq!(
            AclGrant::new(principal, ""),
            Err(PrimitiveSpecError::InvalidText)
        );
    }
}
