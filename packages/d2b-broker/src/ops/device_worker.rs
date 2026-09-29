//! The trusted scope of one Device-owned worker launch.
//!
//! Every Device-owned worker row (`Process/swtpm-<device>`,
//! `Process/gpu-<device>`, `Process/video-<device>`,
//! `EphemeralProcess/swtpm-flush-<device>`) is a declared child of the
//! `Device` that owns the physical function, and the launch presents both the
//! row (`resource_ref`) and the Device it claims to serve (`owner_ref` /
//! `owner_uid`). Every path the launch derives - the swtpm state identity, the
//! per-Guest socket directory the broker opens to the worker - hangs off that
//! Device, so the Device is the trust anchor and the request's claim about
//! which Device it is must be pinned against the verified Zone resource
//! bundle: the bundle's row for the launched Process row names the Device that
//! owns it, and the durable uid that Device's row carries is the deterministic
//! derivation of its key. [`resolve_launch_scope`] performs that pin; a launch
//! whose claim names another Device (or another Device's uid) is refused by
//! name before any Device-derived identity is trusted.
//!
//! Nothing here reads the launch arguments or any other caller-supplied path:
//! the pinned Device, the Guest it declares, and the per-Guest socket
//! directory are all bundle-derived.

use std::os::fd::AsFd;
use std::path::{Path, PathBuf};

use d2b_contracts_broker::broker_wire::RunnerRole;
use d2b_contracts_resource::v3::{ResourceRef, ResourceUid};
use d2b_core::bundle_resolver::{device_worker_posture, BundleResolver, DEVICE_TPM_PROVIDER_REF};
use d2b_core::processes::ProcessRole;
use d2b_core::storage::StoragePathKind;

/// The numeric posture the trusted `path:vm-run:<guest>` storage row declares
/// for one Guest's per-Guest runtime socket directory, resolved against this
/// host: owner uid, owner gid, and mode. Every field comes from the verified
/// storage contract; the broker invents none of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuestRuntimeDirPosture {
    /// The row's declared owner user.
    pub(crate) owner_uid: u32,
    /// The row's declared owning group.
    pub(crate) owner_gid: u32,
    /// The row's declared mode (`1770` for the per-Guest runtime tree: the
    /// worker principal reaches the directory through an explicit grant, and
    /// the sticky bit keeps one principal from renaming another's socket).
    pub(crate) mode: u32,
}

/// The trusted posture of one Guest's per-Guest runtime socket directory
/// (`<runtime_root>/vms/<guest>`), taken from the verified storage contract's
/// `path:vm-run:<guest>` row.
///
/// The row must be scoped to that very Guest and declare a directory: a
/// storage id that resolved to another scope or another kind is a contract
/// this broker refuses rather than a posture it borrows. `None` for a Guest
/// the contract names no such row for, or one whose declared principals and
/// mode do not resolve on this host - the caller then fails closed instead
/// of creating the directory with a posture of its own.
pub(crate) fn guest_runtime_dir_posture(
    resolver: &BundleResolver,
    guest: &str,
) -> Option<GuestRuntimeDirPosture> {
    let spec = resolver.find_storage_path_spec(&format!("path:vm-run:{guest}"))?;
    if spec.scope.as_str() != format!("vm:{guest}") || spec.kind != StoragePathKind::Directory {
        return None;
    }
    let (owner_uid, owner_gid, mode) = crate::ops::storage_contract::row_posture(spec)?;
    Some(GuestRuntimeDirPosture {
        owner_uid,
        owner_gid,
        mode,
    })
}

/// The runtime directory layout the per-Guest device sockets live under:
/// `<runtime_root>/vms/<guest>/<socket>` (the convention the guest VMM's
/// `--tpm socket=` / `--gpu socket=` arguments name).
const RUNTIME_VM_DIR: &str = "vms";

/// The Device scope one Device-owned worker launch is pinned to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceWorkerScope {
    /// The Zone identity the launched row was resolved under; the same Zone
    /// uid the request carried, now the Zone the bundle row was read from.
    pub(crate) zone_uid: ResourceUid,
    /// The `Device` row that owns the launched Process row, per the verified
    /// Zone resource bundle (`Process.metadata.ownerRef`, cross-checked with
    /// the request's claim).
    pub(crate) device_ref: ResourceRef,
    /// That Device's durable row uid: the deterministic derivation of its
    /// `(zone, "Device", name)` key, which is the uid the manager mints and
    /// the request must carry.
    pub(crate) device_uid: ResourceUid,
    /// The Guest the Device declares as its owner
    /// (`Device.metadata.ownerRef == Guest/<guest>`): the VM scope every
    /// runtime path of the worker hangs off.
    pub(crate) guest: String,
}

impl DeviceWorkerScope {
    /// The Guest name this scope pins.
    pub(crate) fn guest(&self) -> &str {
        &self.guest
    }

    /// The per-Guest runtime socket directory the worker binds its socket in
    /// (`<runtime_root>/vms/<guest>`), validated to stay strictly inside the
    /// broker's own runtime root.
    pub(crate) fn socket_directory(&self, runtime_root: &Path) -> Result<PathBuf, GuestSocketError> {
        guest_socket_directory(runtime_root, &self.guest)
    }
}

/// The closed reason one launch's Device scope could not be pinned.
///
/// Each reason names the field the launch is refused under, so the refusal is
/// filed against the request field that disagreed with the verified bundle
/// rather than collapsing into an opaque launch failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceWorkerScopeError {
    /// The verified bundles name no row for the launched Process row: the
    /// launch's row/zone identity is not one this host's bundle declares.
    RowUnresolved,
    /// The bundle's row for the launched Process row carries no authored
    /// owner, so the owning Device cannot be resolved.
    RowOwnerMissing,
    /// The bundle's row for the launched Process row is owned by a reference
    /// that is not a `Device`.
    RowOwnerNotDevice { owning: String },
    /// The request claims no owning Device reference at all.
    OwnerMissing,
    /// The request names a Device that is not the one owning the launched row.
    OwnerMismatch { claimed: String, owning: String },
    /// The request carries no semantic-owner uid.
    OwnerUidMissing { owning: String },
    /// The request's owner uid is not the pinned Device's durable row uid.
    OwnerUidMismatch { claimed: String, owning: String },
    /// The owning Device declares no Guest owner, so the worker has no VM
    /// scope to derive its runtime paths from.
    GuestUnresolved { owning: String },
    /// The launch asserts a Device scope the verified bundle does not pin to
    /// its launched row.
    ScopeMismatch,
}

impl std::fmt::Display for DeviceWorkerScopeError {
    /// The closed, path-free slug a launch refusal and its audit record carry.
    /// A raw reference or uid is deliberately absent: the field it disagrees
    /// on is already reported by [`Self::field`], and the claim is recorded
    /// nowhere else.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::RowUnresolved => "device-worker-row-unresolved",
            Self::RowOwnerMissing => "device-worker-row-owner-missing",
            Self::RowOwnerNotDevice { .. } => "device-worker-row-owner-not-a-device",
            Self::OwnerMissing => "device-worker-owner-missing",
            Self::OwnerMismatch { .. } => "device-worker-owner-mismatch",
            Self::OwnerUidMissing { .. } => "device-worker-owner-uid-missing",
            Self::OwnerUidMismatch { .. } => "device-worker-owner-uid-mismatch",
            Self::GuestUnresolved { .. } => "device-worker-guest-unresolved",
            Self::ScopeMismatch => "device-worker-scope-mismatch",
        })
    }
}

impl DeviceWorkerScopeError {
    /// The request field the refusal is filed under.
    pub(crate) fn field(&self) -> &'static str {
        match self {
            Self::OwnerUidMissing { .. } | Self::OwnerUidMismatch { .. } => "owner_uid",
            Self::OwnerMissing | Self::OwnerMismatch { .. } => "owner_ref",
            Self::RowUnresolved
            | Self::RowOwnerMissing
            | Self::RowOwnerNotDevice { .. }
            | Self::GuestUnresolved { .. } => "resource_ref",
            Self::ScopeMismatch => "deviceWorker.scope",
        }
    }

    /// What the request claimed (or `missing`), for the refusal record.
    pub(crate) fn requested(&self) -> String {
        match self {
            Self::RowUnresolved | Self::RowOwnerMissing | Self::OwnerMissing => {
                "missing".to_owned()
            }
            Self::RowOwnerNotDevice { owning } => owning.clone(),
            Self::OwnerMismatch { claimed, .. } | Self::OwnerUidMismatch { claimed, .. } => {
                claimed.clone()
            }
            Self::OwnerUidMissing { .. } => "missing".to_owned(),
            Self::GuestUnresolved { owning } => owning.clone(),
            Self::ScopeMismatch => "device-worker-scope-claim".to_owned(),
        }
    }

    /// What the verified bundle resolved instead.
    pub(crate) fn resolved(&self) -> String {
        match self {
            Self::RowUnresolved => "verified-bundle-row".to_owned(),
            Self::RowOwnerMissing => "bundle-row-owner".to_owned(),
            Self::RowOwnerNotDevice { .. } => "bundle-row-owner-is-not-a-device".to_owned(),
            Self::OwnerMissing => "owning-device-of-launched-row".to_owned(),
            Self::OwnerMismatch { owning, .. } => owning.clone(),
            Self::OwnerUidMissing { owning } | Self::OwnerUidMismatch { owning, .. } => {
                owning.clone()
            }
            Self::GuestUnresolved { .. } => "device-guest-owner".to_owned(),
            Self::ScopeMismatch => "bundle-pinned-device-worker-scope".to_owned(),
        }
    }
}

/// Pin the Device scope of one Device-owned worker launch.
///
/// The launched row is resolved from the verified Zone resource bundle the
/// request's `zone_uid` names (`Process.metadata.ownerRef`), that owner must
/// be a `Device`, `owner_ref` must be exactly it, and `owner_uid` must be that
/// Device row's durable uid. Only then is the Device's declared Guest read.
///
/// Every refusal is fail-closed: an unresolvable or disagreeing identity is
/// refused instead of falling back to the request's claim.
pub(crate) fn resolve_launch_scope(
    resolver: &BundleResolver,
    resource_ref: &ResourceRef,
    zone_uid: &ResourceUid,
    owner_ref: Option<&ResourceRef>,
    owner_uid: Option<&ResourceUid>,
) -> Result<DeviceWorkerScope, DeviceWorkerScopeError> {
    let (zone, bundle_bytes) =
        zone_bundle_for_uid(resolver, zone_uid).ok_or(DeviceWorkerScopeError::RowUnresolved)?;
    let owning = row_owner_ref(
        bundle_bytes,
        resource_ref.resource_type().as_str(),
        resource_ref.name().as_str(),
    )
    .ok_or(DeviceWorkerScopeError::RowOwnerMissing)?;
    if owning.resource_type().as_str() != "Device" {
        return Err(DeviceWorkerScopeError::RowOwnerNotDevice {
            owning: owning.to_canonical_string(),
        });
    }
    let Some(owner_ref) = owner_ref else {
        return Err(DeviceWorkerScopeError::OwnerMissing);
    };
    if owner_ref != &owning {
        return Err(DeviceWorkerScopeError::OwnerMismatch {
            claimed: owner_ref.to_canonical_string(),
            owning: owning.to_canonical_string(),
        });
    }
    let owning_uid = deterministic_resource_uid(&zone, "Device", owning.name().as_str());
    match owner_uid {
        None => {
            return Err(DeviceWorkerScopeError::OwnerUidMissing {
                owning: owning_uid.to_canonical_string(),
            });
        }
        Some(claimed) if claimed != &owning_uid => {
            return Err(DeviceWorkerScopeError::OwnerUidMismatch {
                claimed: claimed.to_canonical_string(),
                owning: owning_uid.to_canonical_string(),
            });
        }
        Some(_) => {}
    }
    let guest = device_guest_owner(bundle_bytes, owning.name().as_str()).ok_or(
        DeviceWorkerScopeError::GuestUnresolved {
            owning: owning.to_canonical_string(),
        },
    )?;
    Ok(DeviceWorkerScope {
        zone_uid: zone_uid.clone(),
        device_ref: owning,
        device_uid: owning_uid,
        guest,
    })
}

/// Re-pin one launch's asserted Device scope against the verified bundle.
///
/// [`resolve_launch_scope`] answers for the *launched row* alone: which `Device`
/// row owns it, that Device's durable uid, and the Guest that Device declares.
/// A launch that arrives over the wire also *asserts* a scope, and that claim
/// is what a naive caller would select every Device-derived path from - so it
/// is never trusted, only checked. The claim is compared field by field
/// against the pin and the launch proceeds on the pin itself, so nothing
/// downstream reads the payload's copy; a claim that does not reproduce the
/// pin - another declared Device with its publicly derivable uid, or another
/// declared Guest - is refused, never repaired.
///
/// `resource_ref` and `zone_uid` are the launched row's own identity, which
/// the pin resolves; `owner_ref` is the launch's claim about the Device that
/// owns it, and `owner_uid` the uid it claims for that Device, so both remain
/// cross-checks the bundle decides.
pub(crate) fn repin_launch_scope(
    resolver: &BundleResolver,
    claimed: &DeviceWorkerScope,
    resource_ref: &ResourceRef,
    zone_uid: &ResourceUid,
    owner_ref: Option<&ResourceRef>,
    owner_uid: Option<&ResourceUid>,
) -> Result<DeviceWorkerScope, DeviceWorkerScopeError> {
    let pinned = resolve_launch_scope(resolver, resource_ref, zone_uid, owner_ref, owner_uid)?;
    if &pinned != claimed {
        return Err(DeviceWorkerScopeError::ScopeMismatch);
    }
    Ok(pinned)
}

/// Whether a Device-owned worker row must find its state directory already
/// provisioned when it launches, or may be admitted before it lands.
///
/// The long-lived worker opens the NVRAM inside that directory by pathname,
/// so for that row an absent directory is a launch it cannot complete. The
/// one-shot flush only connects to a control socket that arrives with the
/// directory, and the row exists precisely to be admitted before that socket
/// does - refusing it there would turn a provisioning race into a refusal
/// the row can only escape by racing the worker.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum StateVolumeLeaf {
    /// The row opens the state directory itself: an absent one refuses.
    #[default]
    MustExist,
    /// The row waits for a socket another actor provides: an absent
    /// directory is not this launch's refusal.
    MayNotHaveLanded,
}

/// What one launch arm resolved for a Device-owned worker row.
///
/// The default - no scope, no runtime socket, the strict state-directory
/// policy - is what every launch whose intent is not a Device-owned worker
/// role resolves.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeviceWorkerLaunch {
    /// The pinned owning-Device scope of a typed launch (the launched row is a
    /// declared Process/EphemeralProcess resource row).
    pub(crate) scope: Option<DeviceWorkerScope>,
    /// Whether the launched role binds a socket under the broker runtime
    /// root's per-Guest directory ([`binds_runtime_socket`]).
    pub(crate) binds_runtime_socket: bool,
    /// What this row needs of its state directory's presence.
    pub(crate) state_volume_leaf: StateVolumeLeaf,
}

/// The state Volume directory of one Guest's TPM Device under the trusted TPM
/// state policy root, when the bundles name exactly one TPM Device for it:
/// `<root>/device-<32hex>-tpm-state`, the TPM Provider's naming from the
/// Device's durable uid.
///
/// `None` when the bundle names no such Device (a Device committed through the
/// Resource API appears in no bundle) or several (the Guest owns several state
/// Volumes): the caller must then keep the shared policy root it resolved
/// rather than inventing one Device's directory.
pub(crate) fn unique_tpm_state_dir(
    devices: &[(ResourceRef, ResourceUid)],
    state_root: &Path,
) -> Option<PathBuf> {
    match devices {
        [(_, device_uid)] => Some(
            state_root.join(crate::ops::swtpm_identity::state_volume_name(device_uid)),
        ),
        _ => None,
    }
}

/// Whether one Device-owned worker role binds its socket under the broker
/// runtime root's per-Guest directory.
///
/// The long-lived swtpm worker (`--server ...path=<root>/vms/<guest>/tpm.sock`)
/// and both GPU sidecars (`--socket <root>/vms/<guest>/gpu.sock`) do. The
/// one-shot flush binds its ctrl socket inside the Device's state Volume, and
/// the video sidecar's socket lives in the video module's own `/run/d2b-video`
/// runtime directory: neither is a directory the broker owns or opens.
pub(crate) const fn binds_runtime_socket(role: &ProcessRole) -> bool {
    matches!(
        role,
        ProcessRole::Swtpm | ProcessRole::Gpu | ProcessRole::GpuRenderNode
    )
}

/// What one Device-owned worker role needs of its state directory's
/// presence when it launches.
///
/// The long-lived worker opens the directory by pathname, so an absent one
/// refuses; the one-shot flush connects to a socket that lands with the
/// directory, so an absent one is a race the row is admitted ahead of.
pub(crate) const fn state_volume_leaf_for_role(role: &ProcessRole) -> StateVolumeLeaf {
    match role {
        ProcessRole::SwtpmPreStartFlush => StateVolumeLeaf::MayNotHaveLanded,
        _ => StateVolumeLeaf::MustExist,
    }
}

/// The slug a launch whose claimed role disagrees with its launched row's
/// own state-directory presence policy is refused by.
const LEAF_ROW_MISMATCH: &str = "device-worker-leaf-row-mismatch";

/// What one *launched row* needs of its state directory's presence, decided
/// by the row the verified Zone resource bundle declares rather than by the
/// role a launch payload asserts.
///
/// The long-lived worker opens the state directory by pathname, so an absent
/// one refuses; the one-shot pre-start flush connects to a control socket
/// that lands with the directory, so an absent one is a race the row is
/// admitted ahead of.
///
/// The launched row is the row [`resolve_launch_scope`] pins against the
/// bundle, so every fact read here is the bundle's: the `Device` row that owns
/// it names the Provider that declares it, and the row names the template it
/// declares. The closed [`device_worker_posture`] table - the one the trusted
/// runner intents are minted from - turns that declared pair into the worker
/// role, and the presence policy follows the role exactly as
/// [`state_volume_leaf_for_role`] does for every other Device worker. A row
/// the bundle declares no Device worker for keeps the strict variant.
pub(crate) fn state_volume_leaf_for_row(
    resolver: &BundleResolver,
    scope: &DeviceWorkerScope,
    row_ref: &ResourceRef,
) -> StateVolumeLeaf {
    declared_worker_role(resolver, scope, row_ref)
        .map(|role| state_volume_leaf_for_role(&role))
        .unwrap_or(StateVolumeLeaf::MustExist)
}

/// The state-directory presence policy of one pinned Device-worker launch,
/// cross-checked against the role the launch claims over the wire.
///
/// The kernel receives the presence policy from the wire, so deriving it from
/// an asserted role would let any launch claim the one-shot row's lenient
/// variant. The launched row is the fact and the claim is only a check: a
/// launch that claims the one-shot posture for a row that is not one, or
/// denies it to the row that is, is refused by [`LEAF_ROW_MISMATCH`] rather
/// than repaired.
pub(crate) fn state_volume_leaf_for_launch(
    resolver: &BundleResolver,
    scope: &DeviceWorkerScope,
    row_ref: &ResourceRef,
    claimed_role: &RunnerRole,
) -> Result<StateVolumeLeaf, &'static str> {
    let leaf = state_volume_leaf_for_row(resolver, scope, row_ref);
    if matches!(claimed_role, RunnerRole::SwtpmFlush)
        != (leaf == StateVolumeLeaf::MayNotHaveLanded)
    {
        return Err(LEAF_ROW_MISMATCH);
    }
    Ok(leaf)
}

/// The Device-worker role the verified Zone resource bundle declares for one
/// pinned launch row, or `None` when it declares no Device worker there.
///
/// `scope` is [`repin_launch_scope`]'s own answer, so the owning `Device` it
/// names is one the bundle itself pinned, and `row_ref` is the row that
/// Device's bundle row owns. The two are found by identity - the Device by
/// its canonical name, the worker by its own name under that Device - so
/// nothing here reads a resource-type name, and a row that resolves to no
/// declared template, or to a template the closed table does not know, is
/// `None` rather than a borrowed kind.
fn declared_worker_role(
    resolver: &BundleResolver,
    scope: &DeviceWorkerScope,
    row_ref: &ResourceRef,
) -> Option<ProcessRole> {
    let (_, bundle_bytes) = zone_bundle_for_uid(resolver, &scope.zone_uid)?;
    let bundle: serde_json::Value = serde_json::from_slice(bundle_bytes).ok()?;
    let device_name = scope.device_ref.name().as_str();
    let provider_ref = find_resource_row(&bundle, |row| {
        row.get("type").and_then(serde_json::Value::as_str) == Some("Device")
            && row
                .get("metadata")
                .and_then(|metadata| metadata.get("name"))
                .and_then(serde_json::Value::as_str)
                == Some(device_name)
    })
    .and_then(|row| spec_field(row, "providerRef"))?;
    let row_name = row_ref.name().as_str();
    let owner = scope.device_ref.to_canonical_string();
    let template = find_resource_row(&bundle, |row| {
        row.get("metadata")
            .and_then(|metadata| metadata.get("name"))
            .and_then(serde_json::Value::as_str)
            == Some(row_name)
            && row
                .get("metadata")
                .and_then(|metadata| metadata.get("ownerRef"))
                .and_then(serde_json::Value::as_str)
                == Some(owner.as_str())
    })
    .and_then(|row| spec_field(row, "template"))?;
    device_worker_posture(provider_ref, template).map(|posture| posture.role().clone())
}

/// One declared `spec` field of a bundle row, as text.
fn spec_field<'a>(row: &'a serde_json::Value, field: &str) -> Option<&'a str> {
    row.get("spec")
        .and_then(|spec| spec.get(field))
        .and_then(serde_json::Value::as_str)
}

/// Why one Guest's runtime socket directory could not be derived.
///
/// The Display of each refusal keeps the stable static-code spelling the
/// broker's launch-failure envelope surfaces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum GuestSocketError {
    /// The runtime root is not an anchored absolute path.
    RuntimeRootNotAnchored,
    /// The Guest name is not one plain path component.
    GuestNotAPlainName,
    /// The derived directory escapes the runtime root.
    DirectoryOutsideRuntimeRoot,
}

impl std::fmt::Display for GuestSocketError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::RuntimeRootNotAnchored => "device-worker-runtime-root-not-anchored",
            Self::GuestNotAPlainName => "device-worker-guest-not-a-plain-name",
            Self::DirectoryOutsideRuntimeRoot => "device-worker-socket-dir-outside-runtime-root",
        })
    }
}

/// The per-Guest runtime socket directory of one trusted Guest name.
///
/// The name must be one plain component (never empty, `.`, `..`, or a
/// multi-component path), the runtime root an anchored absolute path, and the
/// resulting directory strictly inside the runtime root - a launch must never
/// make the broker open a directory above the tree it owns.
pub(crate) fn guest_socket_directory(
    runtime_root: &Path,
    guest: &str,
) -> Result<PathBuf, GuestSocketError> {
    if !is_anchored_absolute(runtime_root) || runtime_root.parent().is_none() {
        return Err(GuestSocketError::RuntimeRootNotAnchored);
    }
    let mut components = Path::new(guest).components();
    let plain_name = !guest.is_empty()
        && !guest.contains('\0')
        && matches!(components.next(), Some(std::path::Component::Normal(_)))
        && components.next().is_none();
    if !plain_name {
        return Err(GuestSocketError::GuestNotAPlainName);
    }
    let directory = runtime_root.join(RUNTIME_VM_DIR).join(guest);
    if directory == runtime_root || !directory.starts_with(runtime_root) {
        return Err(GuestSocketError::DirectoryOutsideRuntimeRoot);
    }
    Ok(directory)
}

/// Why one Guest's per-Guest runtime directory could not be created and
/// postured to the posture its trusted `path:vm-run:<guest>` row declares.
///
/// A closed set of path-free slugs: both the launch refusal and the audit
/// record render the slug and the errno class, never a raw path or a raw
/// I/O message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RuntimeDirPostureError {
    /// The verified storage contract declares no `path:vm-run:<guest>` row
    /// for this Guest, so the directory has no declared identity and the
    /// broker refuses to invent one.
    RowUnresolved,
    /// The `vms` parent the per-Guest directory lives in does not exist.
    /// Host activation's tmpfiles rule owns that parent
    /// (`d /run/d2b/vms 1770 d2bd d2b`), so its absence is a host posture
    /// gap the broker refuses rather than one it invents a posture for.
    ParentAbsent,
    /// The directory already exists and its owner is not the one the row
    /// declares, so it is a directory another principal planted inside the
    /// sticky per-Guest parent rather than this launch's own.
    OwnerMismatch,
    /// The directory already exists, its owner is the one the row declares,
    /// its mode is not the declared one, and the DECLARED mode carries no
    /// group bits. POSIX rewrites an ACL mask from the group bits, so
    /// stamping that mode would nullify the very named entries the worker
    /// binds its socket through: the broker refuses to trade a wrong mode
    /// for a revoked grant.
    DeclaredModeZeroesAclMask,
    /// The directory exists or was created, but could not be brought to
    /// the declared mode and ownership. Carries the `io::ErrorKind` so the
    /// refusal stays path-free.
    PostureFailed(std::io::ErrorKind),
}

impl std::fmt::Display for RuntimeDirPostureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RowUnresolved => f.write_str("per-guest-dir-posture-row-unresolved"),
            Self::ParentAbsent => f.write_str("per-guest-dir-posture-parent-absent"),
            Self::OwnerMismatch => f.write_str("per-guest-dir-posture-owner-mismatch"),
            Self::DeclaredModeZeroesAclMask => {
                f.write_str("per-guest-dir-posture-declared-mode-zeroes-acl-mask")
            }
            Self::PostureFailed(kind) => {
                write!(f, "per-guest-dir-posture-failed:{kind:?}")
            }
        }
    }
}

/// Create one Guest's per-Guest runtime socket directory
/// (`<runtime_root>/vms/<guest>`) and posture it to what its trusted
/// `path:vm-run:<guest>` storage row declares, returning the `(dev, ino)` of
/// the exact inode that was postured so an audit record can tie itself to it
/// without naming a path.
///
/// A directory this call creates gets the declared mode and ownership. A
/// directory that already exists is accepted only when its owner is the one
/// the row declares, and its MODE is then reconciled to the declared one: the
/// same directory is shared with the sibling workers of that Guest, so a leaf
/// that reached the filesystem before this grant - the daemon's own
/// serving-socket realize creates it first on a host where the two race -
/// must still end up carrying the sticky posture the row declares, or a
/// principal holding a named rwx entry on it can rename another worker's
/// socket and bind its own at that name. The reconcile is safe precisely
/// because the DECLARED mode carries group bits: POSIX rewrites an ACL mask
/// from the group bits, so stamping `1770` leaves every named entry effective
/// where stamping `0700` would nullify the lot. A declared mode with no group
/// bits is refused rather than applied. Every step is fd-relative to the
/// parent opened with
/// `openat2(RESOLVE_NO_SYMLINKS | RESOLVE_BENEATH)`, so no component can be
/// swapped for a symlink between the check and the `mkdirat`.
///
/// The create is exclusive ([`crate::sys::path_safe::mkdir_at_exclusive`],
/// issue #64): the sticky per-Guest parent is writable by a `d2b`-group
/// principal, so a leaf can be planted between the absence check and the
/// create. A leaf planted by ANOTHER principal is refused on the evidence of
/// its owner and never adopted, because re-owning it would hand the planter a
/// directory the broker then believes it provisioned; a leaf the row's own
/// owner holds is this launch's to posture.
///
/// Refuses rather than degrades: a directory it cannot create or posture is
/// a launch the broker has no honest way to complete, because the ACL grant
/// the worker needs has nowhere to land.
///
pub(crate) fn create_guest_runtime_dir(
    directory: &Path,
    posture: GuestRuntimeDirPosture,
) -> Result<(u64, u64), RuntimeDirPostureError> {
    let parent = directory
        .parent()
        .ok_or(RuntimeDirPostureError::ParentAbsent)?;
    let name = directory
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(RuntimeDirPostureError::PostureFailed(
            std::io::ErrorKind::InvalidInput,
        ))?;
    let parent_fd = crate::sys::path_safe::open_dir_path_safe(parent).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            RuntimeDirPostureError::ParentAbsent
        } else {
            RuntimeDirPostureError::PostureFailed(error.kind())
        }
    })?;
    let fd = match crate::sys::path_safe::mkdir_at_exclusive(
        parent_fd.as_fd(),
        Path::new(name),
        posture.mode,
    ) {
        Ok(()) => {
            let fd = open_leaf(&parent_fd, name)?;
            crate::sys::path_safe::fchmod(fd.as_fd(), posture.mode)
                .and_then(|()| {
                    crate::sys::path_safe::fchown(
                        fd.as_fd(),
                        Some(posture.owner_uid),
                        Some(posture.owner_gid),
                    )
                })
                .map_err(|error| RuntimeDirPostureError::PostureFailed(error.kind()))?;
            fd
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            // Accepted on the evidence of its own owner, never adopted from
            // another principal: once the owner matches, the leaf is in this
            // row's trust domain, and the posture the row declares is the
            // one it must end up carrying however it got there.
            let fd = open_leaf(&parent_fd, name)?;
            let stat = crate::sys::path_safe::fstat_fd(fd.as_fd())
                .map_err(|error| RuntimeDirPostureError::PostureFailed(error.kind()))?;
            if stat.st_uid != posture.owner_uid {
                return Err(RuntimeDirPostureError::OwnerMismatch);
            }
            if stat.st_mode & 0o7777 != posture.mode {
                // Safe only for a declared mode that carries group bits: the
                // `chmod` below rewrites the ACL mask from them, so a
                // group-bitless declared mode would nullify the named entries
                // the worker binds its socket through instead of repairing
                // the posture.
                if posture.mode & 0o070 == 0 {
                    return Err(RuntimeDirPostureError::DeclaredModeZeroesAclMask);
                }
                crate::sys::path_safe::fchmod(fd.as_fd(), posture.mode)
                    .map_err(|error| RuntimeDirPostureError::PostureFailed(error.kind()))?;
            }
            fd
        }
        Err(error) => return Err(RuntimeDirPostureError::PostureFailed(error.kind())),
    };
    let stat = crate::sys::path_safe::fstat_fd(fd.as_fd())
        .map_err(|error| RuntimeDirPostureError::PostureFailed(error.kind()))?;
    Ok((stat.st_dev, stat.st_ino))
}

/// Open the leaf directory itself beneath an already-open safe parent: no
/// symlink, no magic link, and nothing above the parent.
fn open_leaf(
    parent_fd: &std::os::fd::OwnedFd,
    name: &str,
) -> Result<std::os::fd::OwnedFd, RuntimeDirPostureError> {
    use rustix::fs::OFlags;

    crate::sys::path_safe::open_at(
        parent_fd.as_fd(),
        Path::new(name),
        OFlags::RDONLY | OFlags::DIRECTORY,
    )
    .map_err(|error| RuntimeDirPostureError::PostureFailed(error.kind()))
}

/// Whether `path` is absolute and carries no `.`/`..` component.
fn is_anchored_absolute(path: &Path) -> bool {
    path.is_absolute()
        && !path.components().any(|component| {
            matches!(
                component,
                std::path::Component::CurDir | std::path::Component::ParentDir
            )
        })
}

/// The verified Zone resource bundle of one Zone uid, with the Zone name the
/// resolver indexes it under.
pub(crate) fn zone_bundle_for_uid<'a>(
    resolver: &'a BundleResolver,
    zone_uid: &ResourceUid,
) -> Option<(String, &'a [u8])> {
    let zone = resolver
        .zone_resource_bundle_zones()
        .ok()?
        .into_iter()
        .find(|zone| resolver.zone_uid(zone).as_ref() == Some(zone_uid))?;
    let bytes = resolver.zone_resource_bundle_bytes(zone.as_str())?;
    Some((zone.as_str().to_owned(), bytes))
}

/// The first row of a verified Zone resource bundle's `resources` array
/// that satisfies `pred`, keeping the row-walk in one place for every
/// bundle-shape consumer.
fn find_resource_row<'a>(
    bundle: &'a serde_json::Value,
    pred: impl FnMut(&&'a serde_json::Value) -> bool,
) -> Option<&'a serde_json::Value> {
    bundle
        .get("resources")?
        .as_array()?
        .iter()
        .find(pred)
}

/// The authored `metadata.ownerRef` of one row of a verified Zone resource
/// bundle, parsed into a canonical reference.
pub(crate) fn row_owner_ref(
    bundle_bytes: &[u8],
    resource_type: &str,
    name: &str,
) -> Option<ResourceRef> {
let bundle: serde_json::Value = serde_json::from_slice(bundle_bytes).ok()?;
    let resource = find_resource_row(&bundle, |row| {
        row.get("type").and_then(serde_json::Value::as_str) == Some(resource_type)
            && row
                .get("metadata")
                .and_then(|metadata| metadata.get("name"))
                .and_then(serde_json::Value::as_str)
                == Some(name)
    })?;
    resource
        .get("metadata")?
        .get("ownerRef")
        .and_then(serde_json::Value::as_str)
        .and_then(|owner| ResourceRef::parse(owner).ok())
}

/// `Device.metadata.ownerRef == Guest/<guest>` for one Device row of a
/// verified Zone resource bundle.
///
/// The same derivation the daemon's Device-worker ticket uses to name the
/// worker's VM scope.
pub(crate) fn device_guest_owner(bundle_bytes: &[u8], device: &str) -> Option<String> {
    let bundle: serde_json::Value = serde_json::from_slice(bundle_bytes).ok()?;
    let resource = find_resource_row(&bundle, |row| {
        row.get("type").and_then(serde_json::Value::as_str) == Some("Device")
            && row
                .get("metadata")
                .and_then(|metadata| metadata.get("name"))
                .and_then(serde_json::Value::as_str)
                == Some(device)
    })?;
    let owner = resource
        .get("metadata")?
        .get("ownerRef")
        .and_then(serde_json::Value::as_str)?;
    owner
        .strip_prefix("Guest/")
        .map(str::to_owned)
        .filter(|guest| !guest.is_empty())
}

/// Every TPM Device the verified bundles declare for one Guest, as
/// `(Device ref, durable uid)`: the Devices whose authored owner is
/// `Guest/<guest>` and whose spec selects the TPM Device Provider.
///
/// The bundle is the only artifact that names a Device's Guest owner, so this
/// is the resolution the worker's per-Device state directory hangs off. A
/// Device committed through the Resource API is not in any bundle and is
/// therefore never returned: callers must treat an empty or multi-entry result
/// as "the bundle does not name one Device", never as "the Guest has none".
pub(crate) fn tpm_devices_of_guest(
    resolver: &BundleResolver,
    guest: &str,
) -> Vec<(ResourceRef, ResourceUid)> {
    let Some(zones) = resolver.zone_resource_bundle_zones().ok() else {
        return Vec::new();
    };
    let mut devices = Vec::new();
    let expected_owner = format!("Guest/{guest}");
    for zone in zones {
        let Some(bytes) = resolver.zone_resource_bundle_bytes(zone.as_str()) else {
            continue;
        };
        let Ok(bundle) = serde_json::from_slice::<serde_json::Value>(bytes) else {
            continue;
        };
        let Some(resource) = find_resource_row(&bundle, |row| {
            row.get("type").and_then(serde_json::Value::as_str) == Some("Device")
                && row
                    .get("metadata")
                    .and_then(|metadata| metadata.get("ownerRef"))
                    .and_then(serde_json::Value::as_str)
                    == Some(expected_owner.as_str())
                && row
                    .pointer("/spec/providerRef")
                    .and_then(serde_json::Value::as_str)
                    == Some(DEVICE_TPM_PROVIDER_REF)
        }) else {
            continue;
        };
        let Some(name) = resource
            .get("metadata")
            .and_then(|metadata| metadata.get("name"))
            .and_then(serde_json::Value::as_str) else {
            continue;
        };
        let Some(device_ref) = ResourceRef::parse(&format!("Device/{name}")).ok() else {
            continue;
        };
        devices.push((
            device_ref,
            deterministic_resource_uid(zone.as_str(), "Device", name),
        ));
    }
    devices
}

/// The durable uid of one manager row key `(zone, type, name)`, as the wire
/// `ResourceUid`.
///
/// Cross-crate contract: `d2b_resource_runtime::manager::deterministic_uid`
/// derives this digest for every manager row (`commit_verified` cross-checks
/// it on every commit), and `d2b_resource_api`'s `manager_uid` replicates it
/// for the API. The broker reconstructs it to pin a Device-derived identity to
/// the Device the verified bundle names. A change to the derivation fails
/// closed here - the uids stop matching and every Device-worker launch is
/// refused - rather than letting the broker trust an owner identity the store
/// would not mint.
pub(crate) fn deterministic_resource_uid(
    zone: &str,
    resource_type: &str,
    name: &str,
) -> ResourceUid {
    use sha2::Digest as _;
    let mut hasher = sha2::Sha256::new();
    hasher.update(b"d2b-resource-uid/v1\x00");
    hasher.update(zone.as_bytes());
    hasher.update([0u8]);
    hasher.update(resource_type.as_bytes());
    hasher.update([0u8]);
    hasher.update(name.as_bytes());
    let digest = hasher.finalize();
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    // The wire identity is the UUIDv4-shaped rendering of the stable digest:
    // the shape bits are forced so the identity survives the closed
    // `ResourceUid` contract (the same rendering every manager row uses).
    ResourceUid::from_bytes(&bytes).expect("shaped row uids satisfy the UUIDv4 contract")
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_core::bundle::{Bundle, BundleGeneration};
    use d2b_core::host::HostJson;
    use d2b_core::manifest_v04::ManifestV04;
    use d2b_core::processes::ProcessesJson;
    use std::collections::BTreeMap;

    const ZONE_UID: &str = "123e4567-e89b-42d3-a456-426614174000";
    /// The derivation the manager mints for the fixture's Device row
    /// `("work", "Device", "tpm0")`, computed independently of this crate
    /// (SHA-256 over `d2b-resource-uid/v1\0work\0Device\0tpm0`, first 16 bytes
    /// rendered as a UUIDv4). If the cross-crate derivation ever changes, this
    /// vector and the manager disagree and the pin below fails closed.
    const TPM0_UID: &str = "0940f65d-b4f7-427f-992b-a65d545ec479";

    /// One authored bundle row, carrying the `spec` fields the Device-worker
    /// lookup reads: the Provider a `Device` row declares itself to be, and
    /// the template a worker row declares.
    fn row(
        resource_type: &str,
        name: &str,
        owner_ref: Option<&str>,
        provider_ref: Option<&str>,
        template: Option<&str>,
    ) -> serde_json::Value {
        let mut spec = serde_json::Map::new();
        if let Some(provider_ref) = provider_ref {
            spec.insert(
                "providerRef".to_owned(),
                serde_json::Value::String(provider_ref.to_owned()),
            );
        }
        if let Some(template) = template {
            spec.insert(
                "template".to_owned(),
                serde_json::Value::String(template.to_owned()),
            );
        }
        let mut metadata = serde_json::Map::new();
        metadata.insert(
            "name".to_owned(),
            serde_json::Value::String(name.to_owned()),
        );
        metadata.insert(
            "zone".to_owned(),
            serde_json::Value::String("work".to_owned()),
        );
        if let Some(owner_ref) = owner_ref {
            metadata.insert(
                "ownerRef".to_owned(),
                serde_json::Value::String(owner_ref.to_owned()),
            );
        }
        serde_json::json!({
            "apiVersion": "resources.d2bus.org/v3",
            "type": resource_type,
            "metadata": serde_json::Value::Object(metadata),
            "spec": serde_json::Value::Object(spec),
        })
    }

    /// The canonical content hash of one fixture resource array, computed the
    /// way `ResourceBundle` computes it (`d2b:v3:resource-bundle` over the
    /// canonical JSON of the sorted rows), so the fixture bundle verifies.
    fn fixture_content_hash(resources: &[serde_json::Value]) -> String {
        use d2b_contracts_resource::v3::resource_schema::{
            CanonicalJsonValue, canonical_json_bytes, framed_canonical_digest,
        };
        let array = serde_json::Value::Array(resources.to_vec());
        let canonical = CanonicalJsonValue::parse(
            &serde_json::to_vec(&array).expect("fixture resources serialize"),
        )
        .expect("fixture resources are canonical JSON");
        framed_canonical_digest(
            "d2b:v3:resource-bundle",
            &canonical_json_bytes(&canonical).expect("fixture resources encode"),
        )
    }

    /// The fixture topology: two Devices owned by two Guests, each with the
    /// long-lived worker row the bundle declares as its child, plus the one-shot
    /// pre-start flush `tpm0` declares. Each Device declares the Provider that
    /// owns its rows and each row declares its template, so the closed
    /// Device-worker posture table resolves every one of them. The rows are
    /// sorted by `(type, name)`, and the content hash is the one
    /// `ResourceBundle` computes for them.
    fn resolver() -> BundleResolver {
        let resources = vec![
            row(
                "Device",
                "gpu0",
                Some("Guest/other-guest"),
                Some("Provider/device-gpu"),
                None,
            ),
            row(
                "Device",
                "tpm0",
                Some("Guest/acceptance-guest"),
                Some("Provider/device-tpm"),
                None,
            ),
            row(
                "EphemeralProcess",
                "swtpm-flush-tpm0",
                Some("Device/tpm0"),
                None,
                Some("swtpm-init-flush"),
            ),
            row(
                "Process",
                "gpu-gpu0",
                Some("Device/gpu0"),
                None,
                Some("gpu-worker"),
            ),
            row(
                "Process",
                "swtpm-tpm0",
                Some("Device/tpm0"),
                None,
                Some("swtpm-socket"),
            ),
        ];
        let bundle = serde_json::json!({
            "schemaVersion": 3,
            "bundleVersion": 1,
            "zone": "work",
            "zoneUid": ZONE_UID,
            "contentHash": fixture_content_hash(&resources),
            "artifactCatalogDigest": format!("sha256:{}", "c".repeat(64)),
            "schemaFingerprints": {},
            "providerSchemaDigests": {},
            "resources": resources,
            "generatedAt": "1970-01-01T00:00:00.000Z",
        });
        let bundle_bytes =
            serde_json::to_vec(&bundle).expect("fixture zone resource bundle serializes");
        let bundle_manifest = Bundle {
            bundle_version: 11,
            schema_version: "v2".to_owned(),
            privileges_path: "privileges.json".to_owned(),
            storage_path: None,
            realm_workloads_launcher_v2_path: None,
            generation: BundleGeneration {
                generator: "test".to_owned(),
                source_revision: None,
                generated_at: None,
            },
            bundle_hash: None,
            artifact_hashes: None,
        };
        let host: HostJson = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/deny-unknown/host-valid.json"
        ))
        .expect("host fixture parses");
        let manifest = ManifestV04::from_slice(
            include_str!("../../../../tests/golden/manifest_v04/baseline-vms.json").as_bytes(),
        )
        .expect("manifest fixture parses");
        BundleResolver::from_artifacts_with_zone_resource_bundles(
            bundle_manifest,
            host,
            ProcessesJson {
                schema_version: "v2".to_owned(),
                vms: Vec::new(),
            },
            manifest,
            BTreeMap::from([("work".to_owned(), bundle_bytes)]),
        )
    }

    /// The broker's `resolver()` fixture plus the storage contract's
    /// `path:vm-run:<guest>` row: the declared posture of the per-Guest
    /// runtime directory. `scope` is a parameter so a test can hand the row
    /// another Guest's scope and see the fence refuse it.
    fn resolver_with_vm_run_row(guest: &str, scope: &str) -> BundleResolver {
        use d2b_contracts::contract_id::{ContractId, PathTemplate};
        use d2b_core::storage::{
            ActorKind, ActorRef, CleanupPolicy, LeaseClass, PrincipalKind, PrincipalRef,
            RepairPolicy, SensitivityClass, StorageAdoptionPolicy, StorageInvariant, StorageJson,
            StorageLifecycle, StoragePathKind, StoragePathSpec, StoragePersistence,
            StorageRestartPolicy,
        };
        let principal = |kind, value: &str| PrincipalRef {
            kind,
            value: ContractId::parse(value).unwrap(),
        };
        let actor = |kind, value: &str| ActorRef {
            kind,
            value: ContractId::parse(value).unwrap(),
        };
        let mut resolver = resolver();
        resolver.set_storage(StorageJson {
            schema_version: "v2".to_owned(),
            roots: Vec::new(),
            paths: vec![StoragePathSpec {
                id: ContractId::parse(&format!("path:vm-run:{guest}")).unwrap(),
                scope: ContractId::parse(scope).unwrap(),
                path_template: PathTemplate::parse(&format!("/run/d2b/vms/{guest}")).unwrap(),
                kind: StoragePathKind::Directory,
                lifecycle: StorageLifecycle::BootScopedReadoptable,
                persistence: StoragePersistence::BootScoped,
                owner: principal(PrincipalKind::Uid, "64025"),
                group: principal(PrincipalKind::Gid, "64025"),
                mode: "1770".to_owned(),
                access_acl: Vec::new(),
                default_acl: Vec::new(),
                creator: actor(ActorKind::NixModule, "tmpfiles"),
                writers: Vec::new(),
                readers: Vec::new(),
                cleanup_policy: CleanupPolicy::Boot,
                repair_policy: RepairPolicy::NixActivation,
                restart_policy: StorageRestartPolicy::PreserveAcrossDaemonRestart,
                adoption_policy: StorageAdoptionPolicy::QuarantineOnAmbiguity,
                lease_class: LeaseClass::ProcessPidfd,
                sensitivity: SensitivityClass::Private,
                no_follow: true,
                recursive: false,
                invariants: vec![StorageInvariant::NoSymlink],
            }],
            restart_policies: Vec::new(),
            degraded_states: Vec::new(),
            remediations: Vec::new(),
        });
        resolver
    }

    /// The posture the broker creates a per-Guest runtime directory with is
    /// the one the trusted `path:vm-run:<guest>` row declares - read out of
    /// the verified contract, never invented - and a row scoped to another
    /// Guest is refused rather than borrowed, because a borrowed posture
    /// would be another directory's identity.
    #[test]
    fn the_guest_runtime_dir_posture_is_the_one_its_trusted_row_declares() {
        assert_eq!(
            guest_runtime_dir_posture(
                &resolver_with_vm_run_row("acceptance-guest", "vm:acceptance-guest"),
                "acceptance-guest"
            ),
            Some(GuestRuntimeDirPosture {
                owner_uid: 64_025,
                owner_gid: 64_025,
                mode: 0o1770,
            }),
        );
        assert_eq!(
            guest_runtime_dir_posture(
                &resolver_with_vm_run_row("acceptance-guest", "vm:other-guest"),
                "acceptance-guest"
            ),
            None,
            "a row scoped to another Guest must not posture this one"
        );
        assert_eq!(
            guest_runtime_dir_posture(&resolver(), "acceptance-guest"),
            None,
            "a Guest the contract names no runtime row for has no declared posture"
        );
    }

    /// The sticky per-Guest parent is writable by a `d2b`-group principal, so
    /// a leaf can be planted there before the launch reaches the create, or
    /// raced into it between the create and the open. A leaf planted by
    /// another principal is refused on the evidence of its owner and keeps
    /// the metadata it was planted with - it is never adopted, and never
    /// re-stamped into looking provisioned. A leaf the row's OWN owner holds
    /// is this launch's, and is reconciled to the declared posture.
    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn a_planted_runtime_directory_owned_by_another_principal_is_refused() {
        use std::os::unix::fs::PermissionsExt as _;

        let root = std::env::current_dir()
            .expect("cwd")
            .join("target")
            .join(format!(
                "device-worker-planted-leaf-{}",
                std::process::id()
            ));
        let vms = root.join("vms");
        std::fs::create_dir_all(&vms).expect("create the vms parent");
        let leaf = vms.join("acceptance-guest");
        let posture = GuestRuntimeDirPosture {
            // The uid a planted directory cannot hold in this test without
            // privileges: the fixture declares an owner that is not the test's
            // own principal, and the fixture plants the leaf as the test's.
            owner_uid: u32::MAX,
            owner_gid: u32::MAX,
            mode: 0o1770,
        };
        let planted_mode = 0o700;
        std::fs::create_dir(&leaf).expect("plant the leaf");
        std::fs::set_permissions(&leaf, std::fs::Permissions::from_mode(planted_mode))
            .expect("set the planted mode");

        assert_eq!(
            create_guest_runtime_dir(&leaf, posture),
            Err(RuntimeDirPostureError::OwnerMismatch),
            "a leaf another principal planted must be refused, not adopted"
        );
        assert_eq!(
            std::fs::metadata(&leaf)
                .expect("stat the planted leaf")
                .permissions()
                .mode()
                & 0o777,
            planted_mode,
            "a refused leaf keeps the metadata it was planted with"
        );
        // The declaration that matches the leaf is accepted, and a leaf that
        // reached the filesystem first - the daemon's own serving-socket
        // realize creates this very directory when the two race - is
        // reconciled to the declared mode rather than used as it stands.
        let accepted = create_guest_runtime_dir(
            &leaf,
            GuestRuntimeDirPosture {
                owner_uid: std::os::unix::fs::MetadataExt::uid(
                    &std::fs::metadata(&leaf).expect("stat the leaf"),
                ),
                owner_gid: std::os::unix::fs::MetadataExt::gid(
                    &std::fs::metadata(&leaf).expect("stat the leaf"),
                ),
                mode: 0o1770,
            },
        )
        .expect("a leaf the row's own owner holds is this launch's to posture");
        assert!(accepted.1 > 0, "the accepted leaf's inode is reported");
        assert_eq!(
            std::fs::metadata(&leaf)
                .expect("stat the reconciled leaf")
                .permissions()
                .mode()
                & 0o7777,
            0o1770,
            "an accepted leaf is reconciled to the declared posture: the sticky \
             bit is what stops one principal renaming another's socket, and a \
             leaf left at the mode it was created with does not carry it"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// The per-Guest runtime directory is SHARED: the daemon's own
    /// serving-socket realize creates it when a host lets the two race, and
    /// that create is where a mode with no group bits, and so no sticky bit,
    /// would land. The posture type claims the sticky bit is what stops one
    /// principal renaming another's socket there, so the EEXIST path has to
    /// make the claim true: a leaf the row's own owner already holds is
    /// reconciled to the DECLARED mode.
    ///
    /// The second assertion is the one that matters. POSIX rewrites a
    /// directory's ACL mask from its group bits on every `chmod`, so a
    /// reconcile to a group-bitless mode would leave the directory looking
    /// postured while the named rwx entry every sibling worker binds its
    /// socket through has silently become `#effective:---`.
    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn an_adopted_runtime_dir_is_brought_to_the_declared_mode_with_its_grant_intact() {
        use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

        let root = std::env::current_dir()
            .expect("cwd")
            .join("target")
            .join(format!(
                "device-worker-adopted-leaf-{}",
                std::process::id()
            ));
        let vms = root.join("vms");
        let leaf = vms.join("acceptance-guest");
        std::fs::create_dir_all(&vms).expect("create the vms parent");
        std::fs::create_dir(&leaf).expect("the leaf another actor created first");
        // The exact artefact the competing create leaves: mode 0700, no
        // sticky bit, so the posture the row declares is simply absent.
        std::fs::set_permissions(&leaf, std::fs::Permissions::from_mode(0o700))
            .expect("stamp the competing create's mode");
        if ![
            "/run/current-system/sw/bin/setfacl",
            "/usr/bin/setfacl",
            "/bin/setfacl",
        ]
        .iter()
        .any(|candidate| Path::new(candidate).exists())
        {
            eprintln!("skipping adopted runtime-directory ACL test: no setfacl binary");
            let _ = std::fs::remove_dir_all(&root);
            return;
        }
        let uid = 50_123;
        let fd = crate::sys::path_safe::open_dir_path_safe(&leaf).expect("open the adopted leaf");
        crate::sys::pidfd_sys::run_setfacl_op_on_fd(fd.as_fd(), "-m", &format!("u:{uid}:rwx"))
            .expect("grant the sibling worker principal its named entry");

        let posture = GuestRuntimeDirPosture {
            owner_uid: std::fs::metadata(&leaf).expect("stat").uid(),
            owner_gid: std::fs::metadata(&leaf).expect("stat").gid(),
            mode: 0o1770,
        };
        create_guest_runtime_dir(&leaf, posture).expect("reconcile the adopted leaf");

        let mode = std::fs::metadata(&leaf).expect("stat").permissions().mode();
        assert_eq!(
            mode & 0o7777,
            0o1770,
            "an adopted leaf must carry the DECLARED posture, sticky bit included"
        );
        assert_eq!(
            mode & 0o070,
            0o070,
            "the reconcile must leave the sibling worker's named entry effective: \
             a group-bitless fchmod would zero the ACL mask this reads back and \
             the Device worker could no longer bind its socket in this tree"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// The reconcile is a `chmod`, and a `chmod` rewrites the ACL mask from
    /// the group bits. So a DECLARED mode with no group bits cannot be
    /// stamped onto an adopted leaf without nullifying the very grants the
    /// worker reaches the directory through: that is a posture the broker
    /// refuses rather than one it applies, under a typed path-free slug.
    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn an_adopted_runtime_dir_refuses_a_declared_mode_that_would_zero_the_acl_mask() {
        use std::os::unix::fs::PermissionsExt as _;

        let root = std::env::current_dir()
            .expect("cwd")
            .join("target")
            .join(format!(
                "device-worker-mask-zeroing-declared-mode-{}",
                std::process::id()
            ));
        let vms = root.join("vms");
        let leaf = vms.join("acceptance-guest");
        std::fs::create_dir_all(&vms).expect("create the vms parent");
        std::fs::create_dir(&leaf).expect("create the leaf");
        std::fs::set_permissions(&leaf, std::fs::Permissions::from_mode(0o1770))
            .expect("stamp the leaf with the declared sticky mode");

        let error = create_guest_runtime_dir(
            &leaf,
            GuestRuntimeDirPosture {
                owner_uid: nix::unistd::Uid::current().as_raw(),
                owner_gid: nix::unistd::Gid::current().as_raw(),
                mode: 0o700,
            },
        )
        .expect_err("a group-bitless declared mode must be refused, not applied");
        assert_eq!(error, RuntimeDirPostureError::DeclaredModeZeroesAclMask);
        assert!(
            error
                .to_string()
                .starts_with("per-guest-dir-posture-")
                && !error.to_string().contains('/'),
            "the refusal must stay a typed path-free slug, got: {error}"
        );
        assert_eq!(
            std::fs::metadata(&leaf)
                .expect("stat the leaf")
                .permissions()
                .mode()
                & 0o7777,
            0o1770,
            "a refused leaf keeps the posture it already carried"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    fn fixture() -> (BundleResolver, ResourceRef, ResourceUid, ResourceUid) {
        (
            resolver(),
            ResourceRef::parse("Process/swtpm-tpm0").expect("row ref"),
            ResourceUid::parse(ZONE_UID).expect("zone uid"),
            ResourceUid::parse(TPM0_UID).expect("device uid"),
        )
    }

    #[test]
    fn the_owning_device_scope_is_pinned_to_the_bundle_row() {
        let (resolver, row_ref, zone_uid, device_uid) = fixture();
        let owner = ResourceRef::parse("Device/tpm0").expect("owner ref");
        let scope = resolve_launch_scope(
            &resolver,
            &row_ref,
            &zone_uid,
            Some(&owner),
            Some(&device_uid),
        )
        .expect("the bundle's owning Device is the request's owner");
        assert_eq!(scope.device_ref, owner);
        assert_eq!(scope.device_uid, device_uid);
        assert_eq!(scope.guest(), "acceptance-guest");
        assert_eq!(
            scope
                .socket_directory(Path::new("/run/d2b"))
                .expect("socket directory"),
            PathBuf::from("/run/d2b/vms/acceptance-guest")
        );
        assert_eq!(
            deterministic_resource_uid("work", "Device", "tpm0"),
            device_uid,
            "the fixture vector is the derivation the manager mints"
        );
    }

    #[test]
    fn a_foreign_owning_device_is_refused_by_name() {
        let (resolver, row_ref, zone_uid, device_uid) = fixture();
        // The launched row is `Process/swtpm-tpm0` (owned by `Device/tpm0`);
        // claiming `Device/gpu0` would aim every Device-derived path at
        // another Guest's runtime tree.
        let foreign = ResourceRef::parse("Device/gpu0").expect("owner ref");
        let error = resolve_launch_scope(
            &resolver,
            &row_ref,
            &zone_uid,
            Some(&foreign),
            Some(&device_uid),
        )
        .expect_err("a foreign Device must be refused");
        assert_eq!(error.field(), "owner_ref");
        assert_eq!(error.requested(), "Device/gpu0");
        assert_eq!(error.resolved(), "Device/tpm0");

        // The same launch with no owner at all is refused, too.
        let error = resolve_launch_scope(&resolver, &row_ref, &zone_uid, None, Some(&device_uid))
            .expect_err("a missing owner must be refused");
        assert_eq!(error.field(), "owner_ref");
        assert_eq!(error.requested(), "missing");
    }

    #[test]
    fn a_foreign_owner_uid_is_refused_by_name() {
        let (resolver, row_ref, zone_uid, device_uid) = fixture();
        let owner = ResourceRef::parse("Device/tpm0").expect("owner ref");
        let foreign_uid = deterministic_resource_uid("work", "Device", "gpu0");
        let error = resolve_launch_scope(
            &resolver,
            &row_ref,
            &zone_uid,
            Some(&owner),
            Some(&foreign_uid),
        )
        .expect_err("another Device's uid must be refused");
        assert_eq!(error.field(), "owner_uid");
        assert_eq!(error.requested(), foreign_uid.as_str());
        assert_eq!(error.resolved(), device_uid.as_str());

        let error = resolve_launch_scope(&resolver, &row_ref, &zone_uid, Some(&owner), None)
            .expect_err("a missing owner uid must be refused");
        assert_eq!(error.field(), "owner_uid");
        assert_eq!(error.requested(), "missing");
    }

    #[test]
    fn a_row_the_bundle_does_not_own_is_refused() {
        let (resolver, _row_ref, zone_uid, device_uid) = fixture();
        let owner = ResourceRef::parse("Device/tpm0").expect("owner ref");
        let undeclared = ResourceRef::parse("Process/swtpm-tpm9").expect("row ref");
        let error = resolve_launch_scope(
            &resolver,
            &undeclared,
            &zone_uid,
            Some(&owner),
            Some(&device_uid),
        )
        .expect_err("a row no verified bundle names must be refused");
        assert_eq!(error.field(), "resource_ref");

        // A zone the bundles do not carry is refused the same way.
        let unknown_zone =
            ResourceUid::parse("123e4567-e89b-42d3-a456-426614174099").expect("zone uid");
        let row_ref = ResourceRef::parse("Process/swtpm-tpm0").expect("row ref");
        let error = resolve_launch_scope(
            &resolver,
            &row_ref,
            &unknown_zone,
            Some(&owner),
            Some(&device_uid),
        )
        .expect_err("an unknown zone must be refused");
        assert_eq!(error.field(), "resource_ref");
    }

    /// The scope a launch asserts over the wire, built from the four fields
    /// the payload carries: the Zone, the Device, that Device's durable uid,
    /// and the Guest the runtime directories are derived from.
    fn claimed_scope(
        device_ref: &ResourceRef,
        device_uid: ResourceUid,
        guest: &str,
    ) -> DeviceWorkerScope {
        DeviceWorkerScope {
            zone_uid: ResourceUid::parse(ZONE_UID).expect("zone uid"),
            device_ref: device_ref.clone(),
            device_uid,
            guest: guest.to_owned(),
        }
    }

    /// A payload that names ANOTHER declared Device is refused. The uid it
    /// carries is that Device's own durable uid, which is publicly derivable
    /// from `(zone, "Device", name)`, so a cross-check on the uid alone agrees
    /// with it - only the launched row's own owning Device, which the verified
    /// bundle names, tells the two apart. Without that pin the grant the launch
    /// receives is the sibling Device's TPM state Volume under the same shared
    /// root, with `rwx` on it.
    #[test]
    fn a_payload_naming_another_declared_device_is_refused() {
        let (resolver, row_ref, zone_uid, device_uid) = fixture();
        let foreign_ref = ResourceRef::parse("Device/gpu0").expect("device ref");
        let foreign_uid = deterministic_resource_uid("work", "Device", "gpu0");
        let owner = ResourceRef::parse("Device/tpm0").expect("owner ref");
        // The claim is fully self-consistent - another declared Device, with
        // that Device's own derivable uid, and the Guest that Device declares.
        let claimed = claimed_scope(&foreign_ref, foreign_uid.clone(), "other-guest");

        // Telling the truth about the launched row's owner while naming the
        // foreign Device in the scope: the uid the launch claims for the
        // owning Device is not that Device's durable uid.
        let error = repin_launch_scope(
            &resolver,
            &claimed,
            &row_ref,
            &zone_uid,
            Some(&owner),
            Some(&foreign_uid),
        )
        .expect_err("a payload naming another declared Device must be refused, not downgraded");
        assert_eq!(error.field(), "owner_uid");
        assert_eq!(
            error.to_string(),
            "device-worker-owner-uid-mismatch",
            "the refusal is a typed, path-free slug"
        );

        // Lying about the owner in the same direction is refused the same way.
        let error = repin_launch_scope(
            &resolver,
            &claimed,
            &row_ref,
            &zone_uid,
            Some(&foreign_ref),
            Some(&foreign_uid),
        )
        .expect_err("a payload naming a Device that does not own the launched row is refused");
        assert_eq!(error.field(), "owner_ref");
        assert_eq!(error.to_string(), "device-worker-owner-mismatch");

        // And the launched row's own scope is the one that resolves, so the
        // Guest whose runtime tree the launch may open is the pinned one.
        let pinned = repin_launch_scope(
            &resolver,
            &claimed_scope(&owner, device_uid.clone(), "acceptance-guest"),
            &row_ref,
            &zone_uid,
            Some(&owner),
            Some(&device_uid),
        )
        .expect("the scope that reproduces the pin resolves");
        assert_eq!(
            pinned.guest(),
            "acceptance-guest",
            "the launched row is still `Process/swtpm-tpm0`, whose own Guest \
             the pin names - this is the grant the refused claim aimed away from"
        );
    }

    /// A payload that names ANOTHER declared Guest is refused too: the Device
    /// and its uid are the pinned ones, so every identity cross-check agrees,
    /// and only the Guest - which every runtime directory is derived from -
    /// is the payload's own.
    #[test]
    fn a_payload_naming_another_declared_guest_is_refused() {
        let (resolver, row_ref, zone_uid, device_uid) = fixture();
        let owner = ResourceRef::parse("Device/tpm0").expect("owner ref");
        let claimed = claimed_scope(&owner, device_uid.clone(), "other-guest");
        let error = repin_launch_scope(
            &resolver,
            &claimed,
            &row_ref,
            &zone_uid,
            Some(&owner),
            Some(&device_uid),
        )
        .expect_err("a payload naming another declared Guest must be refused");
        assert_eq!(error, DeviceWorkerScopeError::ScopeMismatch);
    }

    /// A scope that reproduces the pin is admitted as the pin, so no
    /// downstream derivation reads the payload's own copy of it.
    #[test]
    fn a_scope_that_reproduces_the_pin_resolves_to_the_pin() {
        let (resolver, row_ref, zone_uid, device_uid) = fixture();
        let owner = ResourceRef::parse("Device/tpm0").expect("owner ref");
        let pinned = resolve_launch_scope(
            &resolver,
            &row_ref,
            &zone_uid,
            Some(&owner),
            Some(&device_uid),
        )
        .expect("the pin resolves");
        assert_eq!(
            repin_launch_scope(
                &resolver,
                &pinned,
                &row_ref,
                &zone_uid,
                Some(&owner),
                Some(&device_uid),
            )
            .expect("a claim that reproduces the pin is admitted"),
            pinned
        );
    }

    /// What a row needs of its state directory's presence is the row's own
    /// fact, not the role a launch claims: the long-lived worker row opens
    /// the directory itself, and only the one-shot row is admitted ahead of
    /// it. A payload that claims the one-shot role for the long-lived row
    /// therefore cannot reach the lenient variant - it is refused by name
    /// instead, on both sides of the disagreement.
    #[test]
    fn the_state_leaf_policy_comes_from_the_launched_row() {
        use d2b_contracts_broker::broker_wire::RunnerRole;

        let (resolver, row_ref, zone_uid, device_uid) = fixture();
        let owner = ResourceRef::parse("Device/tpm0").expect("owner ref");
        let pinned = resolve_launch_scope(
            &resolver,
            &row_ref,
            &zone_uid,
            Some(&owner),
            Some(&device_uid),
        )
        .expect("the pin resolves");
        assert_eq!(pinned.device_ref, owner);

        assert_eq!(
            state_volume_leaf_for_row(&resolver, &pinned, &row_ref),
            StateVolumeLeaf::MustExist,
            "the long-lived worker row opens the state directory itself"
        );

        // The one-shot row is pinned by the very same Device, and the bundle
        // is what says which of the two it is: its declared template resolves
        // through the closed Device-worker posture table to the one-shot
        // role, and only that role is admitted ahead of the directory.
        let flush = ResourceRef::parse("EphemeralProcess/swtpm-flush-tpm0").expect("row ref");
        let flush_pinned = resolve_launch_scope(
            &resolver,
            &flush,
            &zone_uid,
            Some(&owner),
            Some(&device_uid),
        )
        .expect("the one-shot row pins to the same Device");
        assert_eq!(
            state_volume_leaf_for_row(&resolver, &flush_pinned, &flush),
            StateVolumeLeaf::MayNotHaveLanded,
            "the one-shot row waits for the socket that lands with the directory"
        );

        for (role, row, scope) in [
            (RunnerRole::Swtpm, &row_ref, &pinned),
            (RunnerRole::SwtpmFlush, &flush, &flush_pinned),
        ] {
            assert_eq!(
                state_volume_leaf_for_launch(&resolver, scope, row, &role),
                Ok(if role == RunnerRole::SwtpmFlush {
                    StateVolumeLeaf::MayNotHaveLanded
                } else {
                    StateVolumeLeaf::MustExist
                }),
                "a launch that tells the truth about its own row is admitted on the row's policy"
            );
        }

        // The wire role is a claim the bundle overrules in both directions:
        // claiming the one-shot posture for the long-lived row would hand it
        // a grant the row cannot use, and denying it to the one-shot row
        // would turn a provisioning race into a refusal.
        for (role, row, scope) in [
            (RunnerRole::SwtpmFlush, &row_ref, &pinned),
            (RunnerRole::Swtpm, &flush, &flush_pinned),
        ] {
            assert_eq!(
                state_volume_leaf_for_launch(&resolver, scope, row, &role),
                Err(LEAF_ROW_MISMATCH),
                "a role that disagrees with the launched row is refused, never repaired"
            );
        }

        // A row the bundle declares no Device-worker template for keeps the
        // strict variant rather than borrowing another row's posture.
        assert_eq!(
            state_volume_leaf_for_row(
                &resolver,
                &pinned,
                &ResourceRef::parse("Process/undeclared-tpm0").expect("row ref"),
            ),
            StateVolumeLeaf::MustExist
        );
    }

    #[test]
    fn the_socket_directory_stays_inside_the_runtime_root() {
        let guest = "acceptance-guest";
        assert_eq!(
            guest_socket_directory(Path::new("/run/d2b"), guest).expect("socket dir"),
            PathBuf::from("/run/d2b/vms/acceptance-guest")
        );
        for guest in ["", ".", "..", "/etc", "a/b", "a\0b"] {
            assert!(
                guest_socket_directory(Path::new("/run/d2b"), guest).is_err(),
                "guest {guest:?} must not name a socket directory"
            );
        }
        assert!(guest_socket_directory(Path::new("run/d2b"), guest).is_err());
        assert!(guest_socket_directory(Path::new("/run/d2b/../etc"), guest).is_err());
        assert!(guest_socket_directory(Path::new("/"), guest).is_err());
    }

    #[test]
    fn tpm_devices_of_guest_reads_the_bundles_owned_devices() {
        let (resolver, _row_ref, _zone_uid, device_uid) = fixture();
        assert_eq!(
            tpm_devices_of_guest(&resolver, "acceptance-guest"),
            vec![(
                ResourceRef::parse("Device/tpm0").expect("device ref"),
                device_uid
            )],
            "the TPM Device the bundle declares for the Guest is the one the op names"
        );
        assert!(
            tpm_devices_of_guest(&resolver, "other-guest").is_empty(),
            "the GPU Device's Guest owns no TPM Device"
        );
    }

    #[test]
    fn one_bundle_named_tpm_device_names_the_worker_state_dir() {
        let root = Path::new("/var/lib/d2b/tpm-state");
        let device_uid = deterministic_resource_uid("work", "Device", "tpm0");
        let state_volume = format!(
            "device-{}-tpm-state",
            device_uid
                .as_str()
                .bytes()
                .filter(|byte| byte.is_ascii_hexdigit())
                .take(32)
                .map(char::from)
                .collect::<String>()
        );
        assert_eq!(
            unique_tpm_state_dir(
                &[(
                    ResourceRef::parse("Device/tpm0").expect("device ref"),
                    device_uid.clone()
                )],
                root
            ),
            Some(root.join(&state_volume)),
            "the op records the Volume directory the worker opens"
        );
        // No bundle-named Device (an API-created Device), or several of them:
        // the op keeps the shared policy root instead of inventing one.
        assert_eq!(unique_tpm_state_dir(&[], root), None);
        assert_eq!(
            unique_tpm_state_dir(
                &[
                    (
                        ResourceRef::parse("Device/tpm0").expect("device ref"),
                        device_uid.clone()
                    ),
                    (
                        ResourceRef::parse("Device/tpm1").expect("device ref"),
                        deterministic_resource_uid("work", "Device", "tpm1")
                    ),
                ],
                root
            ),
            None
        );
    }
}
