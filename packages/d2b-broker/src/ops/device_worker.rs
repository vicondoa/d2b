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

use std::path::{Path, PathBuf};

use d2b_contracts_resource::v3::{ResourceRef, ResourceUid};
use d2b_core::bundle_resolver::{BundleResolver, DEVICE_TPM_PROVIDER_REF};
use d2b_core::processes::ProcessRole;

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
    pub(crate) fn socket_directory(
        &self,
        runtime_root: &Path,
    ) -> Result<PathBuf, &'static str> {
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

/// What one launch arm resolved for a Device-owned worker row.
///
/// The default - no scope, no runtime socket - is what every launch whose
/// intent is not a Device-owned worker role resolves.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeviceWorkerLaunch {
    /// The pinned owning-Device scope of a typed launch (the launched row is a
    /// declared Process/EphemeralProcess resource row).
    pub(crate) scope: Option<DeviceWorkerScope>,
    /// Whether the launched role binds a socket under the broker runtime
    /// root's per-Guest directory ([`binds_runtime_socket`]).
    pub(crate) binds_runtime_socket: bool,
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
            state_root.join(crate::ops::swtpm_dir::state_volume_name(device_uid)),
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

/// The per-Guest runtime socket directory of one trusted Guest name.
///
/// The name must be one plain component (never empty, `.`, `..`, or a
/// multi-component path), the runtime root an anchored absolute path, and the
/// resulting directory strictly inside the runtime root - a launch must never
/// make the broker open a directory above the tree it owns.
pub(crate) fn guest_socket_directory(
    runtime_root: &Path,
    guest: &str,
) -> Result<PathBuf, &'static str> {
    if !is_anchored_absolute(runtime_root) || runtime_root.parent().is_none() {
        return Err("device-worker-runtime-root-not-anchored");
    }
    let mut components = Path::new(guest).components();
    let plain_name = !guest.is_empty()
        && !guest.contains('\0')
        && matches!(components.next(), Some(std::path::Component::Normal(_)))
        && components.next().is_none();
    if !plain_name {
        return Err("device-worker-guest-not-a-plain-name");
    }
    let directory = runtime_root.join(RUNTIME_VM_DIR).join(guest);
    if directory == runtime_root || !directory.starts_with(runtime_root) {
        return Err("device-worker-socket-dir-outside-runtime-root");
    }
    Ok(directory)
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

/// The authored `metadata.ownerRef` of one row of a verified Zone resource
/// bundle, parsed into a canonical reference.
pub(crate) fn row_owner_ref(
    bundle_bytes: &[u8],
    resource_type: &str,
    name: &str,
) -> Option<ResourceRef> {
    let bundle: serde_json::Value = serde_json::from_slice(bundle_bytes).ok()?;
    for resource in bundle.get("resources")?.as_array()? {
        if resource.get("type").and_then(serde_json::Value::as_str) != Some(resource_type) {
            continue;
        }
        let metadata = resource.get("metadata")?;
        if metadata.get("name").and_then(serde_json::Value::as_str) != Some(name) {
            continue;
        }
        return metadata
            .get("ownerRef")
            .and_then(serde_json::Value::as_str)
            .and_then(|owner| ResourceRef::parse(owner).ok());
    }
    None
}

/// `Device.metadata.ownerRef == Guest/<guest>` for one Device row of a
/// verified Zone resource bundle.
///
/// The same derivation the daemon's Device-worker ticket uses to name the
/// worker's VM scope.
pub(crate) fn device_guest_owner(bundle_bytes: &[u8], device: &str) -> Option<String> {
    let bundle: serde_json::Value = serde_json::from_slice(bundle_bytes).ok()?;
    for resource in bundle.get("resources")?.as_array()? {
        if resource.get("type").and_then(serde_json::Value::as_str) != Some("Device") {
            continue;
        }
        let Some(metadata) = resource.get("metadata") else {
            continue;
        };
        if metadata.get("name").and_then(serde_json::Value::as_str) != Some(device) {
            continue;
        }
        let owner = metadata
            .get("ownerRef")
            .and_then(serde_json::Value::as_str)?;
        return owner
            .strip_prefix("Guest/")
            .map(str::to_owned)
            .filter(|guest| !guest.is_empty());
    }
    None
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
    for zone in zones {
        let Some(bytes) = resolver.zone_resource_bundle_bytes(zone.as_str()) else {
            continue;
        };
        let Ok(bundle) = serde_json::from_slice::<serde_json::Value>(bytes) else {
            continue;
        };
        let Some(resources) = bundle.get("resources").and_then(|rows| rows.as_array()) else {
            continue;
        };
        for resource in resources {
            if resource.get("type").and_then(serde_json::Value::as_str) != Some("Device") {
                continue;
            }
            let Some(metadata) = resource.get("metadata") else {
                continue;
            };
            let expected_owner = format!("Guest/{guest}");
            if metadata.get("ownerRef").and_then(serde_json::Value::as_str)
                != Some(expected_owner.as_str())
            {
                continue;
            }
            if resource
                .pointer("/spec/providerRef")
                .and_then(serde_json::Value::as_str)
                != Some(DEVICE_TPM_PROVIDER_REF)
            {
                continue;
            }
            let Some(name) = metadata.get("name").and_then(serde_json::Value::as_str) else {
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
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let text = format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7], bytes[8],
        bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15],
    );
    ResourceUid::parse(text).expect("shaped row uids satisfy the UUIDv4 contract")
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

    /// One authored bundle row.
    fn row(
        resource_type: &str,
        name: &str,
        owner_ref: Option<&str>,
        provider_ref: Option<&str>,
    ) -> serde_json::Value {
        let mut spec = serde_json::Map::new();
        if let Some(provider_ref) = provider_ref {
            spec.insert(
                "providerRef".to_owned(),
                serde_json::Value::String(provider_ref.to_owned()),
            );
        }
        let mut metadata = serde_json::Map::new();
        metadata.insert("name".to_owned(), serde_json::Value::String(name.to_owned()));
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
    /// worker row the bundle declares as its child. Only `tpm0` declares the
    /// TPM Device Provider. The rows are sorted by `(type, name)`, and the
    /// content hash is the one `ResourceBundle` computes for them.
    fn resolver() -> BundleResolver {
        let resources = vec![
            row(
                "Device",
                "gpu0",
                Some("Guest/other-guest"),
                Some("Provider/device-gpu"),
            ),
            row(
                "Device",
                "tpm0",
                Some("Guest/acceptance-guest"),
                Some("Provider/device-tpm"),
            ),
            row("Process", "gpu-gpu0", Some("Device/gpu0"), None),
            row("Process", "swtpm-tpm0", Some("Device/tpm0"), None),
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
            public_manifest_path: "vms.json".to_owned(),
            host_path: "host.json".to_owned(),
            processes_path: "processes.json".to_owned(),
            privileges_path: "privileges.json".to_owned(),
            storage_path: None,
            sync_path: None,
            allocator_path: None,
            realm_controllers_path: None,
            realm_identity_path: None,
            realm_workloads_launcher_v2_path: None,
            unsafe_local_workloads_path: None,
            closures: Vec::new(),
            minijail_profiles: Vec::new(),
            managed_keys: Default::default(),
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
