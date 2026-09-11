//! Per-zone v3 resource plane assembly and the Phase A type partition
//! (U9/U10; KTD4, KTD5, R26, R27, R31).
//!
//! ## What this module owns
//!
//! - [`route_resource_type`] classifies a resource type onto the new plane
//!   (`Process`, `Volume`, `VolumeBinding`, `Endpoint`) or the old plane,
//!   and the public surface dispatches by that classification: converted
//!   types are served to callers ONLY by the new plane, unconverted types
//!   ONLY by the old one. Legacy in-daemon writers of converted types (the
//!   framework runners and controller sessions that U12 carries over) still
//!   write the redb plane until their providers convert, so the exclusive
//!   rule holds on the API surface, not yet inside the daemon. R29's
//!   "exactly one model" completes in Phase B when the remaining types
//!   convert and those writers are deleted.
//! - [`ResourcePlaneV3`] is the per-zone NEW assembly (KTD5): it opens the
//!   per-zone SQLite spec store, registers the four converted-type driver
//!   factories over the production effects the old reconcilers compose from
//!   (KTD7: ticket inputs from the bundle resolver / `ZoneAuthorityIdentity`,
//!   never from the spec store), spawns the per-zone manager, and reports
//!   the U9 readiness checklist.
//! - [`ResourcePlaneV3::ingest_nix_bundle`] is U10: the Nix bundle flows
//!   into the manager as desired specs with provenance `Nix` under the
//!   bundle subject; unconverted rows pass through to the existing
//!   `materialize_zone_resource_bundle` path unchanged; Nix applies never
//!   clobber API-provenance rows (the partition consults durable provenance,
//!   and removals only touch Nix-provenance rows).
//!
//! ## Spec store path decision
//!
//! The old redb store is opened through a broker fd handover
//! (`open_zone_store_from_broker`), so the daemon never derived its path and
//! never needed write access to the directory holding it. That directory
//! (`<state-root>/zones/<zone>`) is broker-provisioned and owned by the
//! zone-store principal; the daemon may traverse it but not create entries in
//! it, so a spec store placed there fails to open with `SQLITE_CANTOPEN`.
//! The new SQLite store is a plain daemon-owned file and therefore lives
//! under the daemon's own state root:
//! `<daemon-state>/zones/<zone>/spec-store.sqlite3`.
//! [`d2b_resource_runtime::spec_store::SpecStore::open`] enforces the 0600
//! file / 0700 directory posture and owns the WAL setup itself, so no broker
//! handover is needed; U14 retires the redb store.

#![allow(dead_code)]

use std::any::Any;
use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::os::unix::fs::FileTypeExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use d2b_contracts_broker::broker_wire::{
    BrokerCallerRole, BrokerRequest, BrokerResponse, StoreSyncRequest,
};
use d2b_contracts_resource::v3::{
    ControllerGeneration, ResourceRef, ResourceUid, ZoneId, ZoneRevision,
    execution_policy::BoundedToken,
    volume::{SourceKind, VolumeSpec},
    volume_binding::VolumeBindingSpec,
};
use d2b_contracts_zone_session::v3::resource_bundle::{BundleResource, ResourceBundle};
use d2b_core::bundle_resolver::{BundleResolver, ResolvedStoreViewIntent, intent_id_store_view};
use d2b_provider_volume_local::{VolumeLocalController, VolumeLocalProfile};
use d2b_provider_volume_virtiofs::{MAX_SOCKET_PATH_BYTES, SocketIdentity, StoredBinding};
use d2b_resource_api::manager_backend::nix_bundle_subject;
use d2b_resource_runtime::context::SpecDecoder;
use d2b_resource_runtime::error::ResourceError;
use d2b_resource_runtime::identity::{ResourceKey, ResourceTypeName};
use d2b_resource_runtime::manager::{
    AdmissionDecision, DesiredResource, MutationAdmission, MutationRequest,
    MutationSubject, ResourceManager, ResourceManagerArgs, ResourceManagerClient, ResourceSelector,
};
use d2b_resource_runtime::provider::ProviderDirectory;
use d2b_resource_runtime::spec_store::{SpecSelector, SpecStore, StoredDesiredResource};
use d2b_resource_runtime::watch::{DEFAULT_RING_CAPACITY, WatchHub};
use d2bd_runtime::resource_runtime_support::NewPlaneReadinessState;
use d2bd_runtime::target_runtime::DaemonMode;
use rustix::fs::{Mode, OFlags, ResolveFlags, open, openat2};
use sha2::{Digest, Sha256};

use crate::activation_driver::{
    ActivationDriverArgs, ActivationDriverEffects, ActivationDriverFactory,
    ProductionActivationDriverEffects, activation_spec_decoder,
};
use crate::binding_driver::{
    BindingDriverArgs, BindingDriverEffects, BindingDriverFactory, ProductionBindingDriverEffects,
    binding_spec_decoder,
};
use crate::credential_driver::{
    CredentialDriverArgs, CredentialDriverEffects, CredentialDriverFactory, credential_spec_decoder,
};
use crate::endpoint_driver::{AsyncSocketEffect, EndpointDriverArgs, EndpointDriverFactory, endpoint_spec_decoder};
use crate::process_driver::{
    ProcessDriverArgs, ProcessDriverEffects, ProcessDriverFactory, ProductionProcessDriverEffects,
    process_spec_decoder,
};
use crate::semantic_binding_resource_runtime::{
    TELEMETRY_BINDING_TYPE, TELEMETRY_SERVICE_TYPE, TelemetryDriverFactory,
    telemetry_spec_decoder,
};
use crate::volume_driver::{
    ProductionVolumeDriverEffects, VolumeDriverArgs, VolumeDriverEffects, volume_spec_decoder,
};
use crate::shared_provider_driver::{
    SharedProviderDriverArgs, SharedProviderDriverEffects, SharedProviderDriverFactory,
    shared_provider_spec_decoder,
};
use crate::shared_provider_effects::ProductionSharedProviderEffects;
use crate::system_core_driver::{SystemCoreDriverFactory, system_core_spec_decoder};

/// Frozen purpose of the binding-owned virtiofsd socket (old `VIRTIOFSD_PURPOSE`
/// in `endpoint_driver.rs`).
const VIRTIOFSD_PURPOSE: &str = "virtiofsd";

/// Preserved reconcile backoff for the plane's resource actors (R13).
const PLANE_BACKOFF: Duration = d2b_resource_runtime::DEFAULT_REQUEUE_BACKOFF;

/// Bounded wait budget for endpoint socket realization.
const SOCKET_REALIZE_BUDGET: Duration = Duration::from_secs(5);

// ---------------------------------------------------------------------------
// U9 type partition router (KTD4)
// ---------------------------------------------------------------------------

/// Converted types (KTD4 Phase A): served exclusively by the new plane.
pub const CONVERTED_TYPES: [&str; 16] =
    d2b_contracts_resource::v3::V3_CONVERTED_RESOURCE_TYPES;

/// Which runtime serves a resource type during Phase A.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaneRoute {
    /// The v3 runtime owns the type end to end (KTD4).
    NewPlane,
    /// The old reconciler plane serves the type until Phase B.
    OldPlane,
}

/// Classify one resource type onto its plane (U9 execution decision
/// implementing the P1 review finding): converted types are served ONLY by
/// the new plane, unconverted types ONLY by the old plane; no type is served
/// by both, so the spec's single-model invariant holds per type from the
/// moment this lands (R29 completes in Phase B).
pub fn route_resource_type(type_name: &str) -> PlaneRoute {
    for converted in CONVERTED_TYPES {
        if converted == type_name {
            return PlaneRoute::NewPlane;
        }
    }
    PlaneRoute::OldPlane
}

// ---------------------------------------------------------------------------
// Committed Provider identities (KTD7)
// ---------------------------------------------------------------------------
//
// The new store carries no `Provider` rows: bundle rows of unconverted types
// pass through to the old plane's redb store, so the per-zone v3 plane cannot
// resolve a controller row's owning Provider from its own store. The
// composition unit resolves the bundle's `Provider` rows through the old
// plane's durable authority and hands the identities to
// [`ConstructionInputs::committed_provider_identities`]; the plane publishes
// them into [`PlaneResourceRegistry`] before its manager spawns any resource
// actor, and the production Process effects bind them to controller rows. A
// reference the authority does not retain stays unpublished, so the
// controller ticket refuses closed.

// ---------------------------------------------------------------------------
// Per-zone plane registry: per-resource anchors for the production effects
// ---------------------------------------------------------------------------

/// Which privileged role one NixClosure volume serves (mirror of the old
/// private `NixClosureVolumeRole`; collapses with it at U14).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoneNixClosureVolumeRole {
    StoreView,
    SystemVolume,
}

/// Per-resource anchor for one Volume row: the storage subdir name plus the
/// NixClosure guest identity the old reconciler derived from the resource
/// value on every reconcile pass.
#[derive(Debug, Clone)]
struct VolumeAnchor {
    volume_name: String,
    guest_ref: Option<ResourceRef>,
    role: Option<ZoneNixClosureVolumeRole>,
}

/// The serving-pair inputs one virtiofs socket realization resolves from.
#[derive(Debug, Clone)]
struct SocketTarget {
    volume_ref: ResourceRef,
    execution_ref: ResourceRef,
}

/// Per-zone registry the production effects resolve per-resource anchors
/// from. The old plane rebuilt these from the resource JSON snapshot on
/// every pass; the new plane registers them once per durable row set (from
/// the spec store at open, after every bundle ingestion, and after API
/// applies via the merge owner's re-registration call).
///
/// The registry is a cache of store-derived rows, so a socket-target lookup
/// that misses consults the authority (the attached spec store) rather than
/// assuming the load-time snapshot was complete: the manager ensures the
/// Volume-minted `VolumeBinding` children *after* those durable loads, so
/// the socket targets they derive are only observable through the store.
#[derive(Default)]
pub struct PlaneResourceRegistry {
    inner: Mutex<RegistryInner>,
    /// The durable authority this registry caches rows from; attached by
    /// the plane once its spec store is open.
    store: OnceLock<Arc<SpecStore>>,
}

impl fmt::Debug for PlaneResourceRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PlaneResourceRegistry").finish_non_exhaustive()
    }
}

#[derive(Debug, Default)]
struct RegistryInner {
    volume_names_by_uid: BTreeMap<String, String>,
    volume_anchors_by_name: BTreeMap<String, VolumeAnchor>,
    socket_targets_by_identity: BTreeMap<String, SocketTarget>,
    socket_targets_by_ref: BTreeMap<String, SocketTarget>,
    /// Committed `Provider` row identities (KTD7) keyed by canonical ref.
    committed_provider_identities:
        BTreeMap<String, (ResourceUid, d2b_contracts_resource::v3::ResourceGeneration)>,
}

impl PlaneResourceRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    fn with_inner<R>(&self, run: impl FnOnce(&mut RegistryInner) -> R) -> R {
        let mut inner = self.inner.lock().expect("plane registry lock");
        run(&mut inner)
    }

    fn lookup_anchor(&self, volume_uid: &ResourceUid) -> Option<VolumeAnchor> {
        self.with_inner(|inner| {
            let volume_name = inner.volume_names_by_uid.get(volume_uid.as_str()).cloned()?;
            inner.volume_anchors_by_name.get(&volume_name).cloned()
        })
    }

    fn lookup_socket_target_by_identity(&self, socket: &SocketIdentity) -> Option<SocketTarget> {
        self.with_inner(|inner| inner.socket_targets_by_identity.get(&socket.to_hex()).cloned())
    }

    fn lookup_socket_target_by_ref(&self, producer_ref: &ResourceRef) -> Option<SocketTarget> {
        self.with_inner(|inner| {
            inner
                .socket_targets_by_ref
                .get(&producer_ref.to_canonical_string())
                .cloned()
        })
    }

    /// Attach the durable authority (the plane's spec store) the cache
    /// loads derived rows from. Idempotent.
    pub fn attach_store(&self, store: Arc<SpecStore>) {
        let _ = self.store.set(store);
    }

    /// Re-register the durable `VolumeBinding` rows (and the socket targets
    /// they derive) from the attached spec store. Bounded to the one type;
    /// never a full row sweep.
    async fn load_binding_targets(&self, zone_token: &BoundedToken) {
        let Some(store) = self.store.get() else {
            return;
        };
        let selector = SpecSelector {
            zone: Some(zone_token.as_str().to_owned()),
            type_name: Some("VolumeBinding".to_owned()),
            owner_uid: None,
        };
        match store.list(selector).await {
            Ok(rows) => {
                for row in &rows {
                    register_binding_row(self, zone_token, row);
                }
            }
            Err(error) => {
                tracing::debug!(
                    zone = %zone_token.as_str(),
                    error = %error,
                    "binding socket target load failed"
                );
            }
        }
    }

    /// Socket target for one binding socket identity: cache first, then the
    /// authority on a miss (the derived child may have been minted after the
    /// plane's last durable load).
    async fn socket_target_by_identity(
        &self,
        zone_token: &BoundedToken,
        socket: &SocketIdentity,
    ) -> Option<SocketTarget> {
        if let Some(target) = self.lookup_socket_target_by_identity(socket) {
            return Some(target);
        }
        self.load_binding_targets(zone_token).await;
        self.lookup_socket_target_by_identity(socket)
    }

    /// Socket target for one serving-pair ref (worker Process or Endpoint):
    /// cache first, then the authority on a miss.
    async fn socket_target_by_ref(
        &self,
        zone_token: &BoundedToken,
        producer_ref: &ResourceRef,
    ) -> Option<SocketTarget> {
        if let Some(target) = self.lookup_socket_target_by_ref(producer_ref) {
            return Some(target);
        }
        self.load_binding_targets(zone_token).await;
        self.lookup_socket_target_by_ref(producer_ref)
    }

    fn register_volume(&self, volume_uid: &str, volume_name: &str, anchor: VolumeAnchor) {
        self.with_inner(|inner| {
            inner
                .volume_names_by_uid
                .entry(volume_uid.to_owned())
                .or_insert_with(|| volume_name.to_owned());
            inner
                .volume_anchors_by_name
                .entry(volume_name.to_owned())
                .and_modify(|existing| {
                    // A bundle-derived NixClosure identity wins over a
                    // name-only registration; never downgrade an anchor.
                    if existing.role.is_none() {
                        *existing = anchor.clone();
                    }
                })
                .or_insert(anchor);
        });
    }

    fn register_binding(
        &self,
        socket_hex: &str,
        worker_ref: &ResourceRef,
        endpoint_ref: &ResourceRef,
        target: SocketTarget,
    ) {
        self.with_inner(|inner| {
            inner
                .socket_targets_by_identity
                .insert(socket_hex.to_owned(), target.clone());
            inner
                .socket_targets_by_ref
                .insert(worker_ref.to_canonical_string(), target.clone());
            inner
                .socket_targets_by_ref
                .insert(endpoint_ref.to_canonical_string(), target);
        });
    }

    /// Register every durable row this plane serves (U9 open, U10
    /// post-apply, and the merge owner's API-path re-registration).
    pub async fn load_from_store(
        &self,
        zone_token: &BoundedToken,
        store: &SpecStore,
    ) -> Result<(), PlaneError> {
        for row in store.list(SpecSelector::default()).await? {
            match row.key.type_name.as_str() {
                "Volume" => {
                    self.register_volume(
                        &resource_uid_string(&row.uid),
                        &row.key.name,
                        volume_anchor_from_row(&row),
                    );
                }
                "VolumeBinding" => register_binding_row(self, zone_token, &row),
                _ => {}
            }
        }
        Ok(())
    }

    /// Publish one committed `Provider` row's identity (KTD7): the production
    /// Process effects bind it to controller rows that Provider owns. Fed by
    /// the plane's construction path from
    /// [`ConstructionInputs::committed_provider_identities`].
    fn register_committed_provider_identity(
        &self,
        provider_ref: &ResourceRef,
        uid: ResourceUid,
        generation: d2b_contracts_resource::v3::ResourceGeneration,
    ) {
        self.with_inner(|inner| {
            inner
                .committed_provider_identities
                .insert(provider_ref.to_canonical_string(), (uid, generation));
        });
    }
}

/// The committed-`Provider` identity view the production Process effects
/// consult (KTD7), published by [`PlaneResourceRegistry`].
impl crate::process_driver::CommittedProviderIdentitySource for PlaneResourceRegistry {
    fn committed_provider_identity(
        &self,
        provider_ref: &ResourceRef,
    ) -> Option<(ResourceUid, d2b_contracts_resource::v3::ResourceGeneration)> {
        self.with_inner(|inner| {
            inner
                .committed_provider_identities
                .get(&provider_ref.to_canonical_string())
                .cloned()
        })
    }
}

fn register_binding_row(registry: &PlaneResourceRegistry, zone_token: &BoundedToken, row: &StoredDesiredResource) {
    let Ok(spec) = serde_json::from_slice::<d2b_contracts_resource::v3::ResourceSpec>(&row.spec)
    else {
        return;
    };
    let Ok(binding) = serde_json::from_slice::<VolumeBindingSpec>(&spec.base().to_canonical_bytes())
    else {
        return;
    };
    let Ok(uid) = resource_uid(&row.uid) else {
        return;
    };
    let Ok(generation) = d2b_contracts_resource::v3::ResourceGeneration::new(row.generation) else {
        return;
    };
    // The readiness fence does not compare revisions (the new store carries
    // none); revision zero matches the old fence behavior (KTD8).
    let stored = StoredBinding::new(binding, uid, generation, ZoneRevision::new(0));
    let socket = stored.socket_identity(zone_token);
    let (Ok(worker_ref), Ok(endpoint_ref)) = (stored.worker_process_ref(), stored.endpoint_ref())
    else {
        return;
    };
    registry.register_binding(
        &socket.to_hex(),
        &worker_ref,
        &endpoint_ref,
        SocketTarget {
            volume_ref: stored.spec().volume_ref().clone(),
            execution_ref: stored.spec().execution_ref().clone(),
        },
    );
}

/// Derive the per-volume anchor from one durable Volume row.
fn volume_anchor_from_row(row: &StoredDesiredResource) -> VolumeAnchor {
    let owner_ref = decode_metadata_owner_ref(&row.metadata);
    let volume_spec = decode_volume_spec(&row.spec);
    let nix_identity = volume_spec
        .as_ref()
        .filter(|spec| spec.source().settings().kind() == SourceKind::NixClosure)
        .and_then(|spec| {
            nix_closure_volume_anchor(&row.key.name, owner_ref.as_ref(), spec).ok()
        });
    VolumeAnchor {
        volume_name: row.key.name.clone(),
        guest_ref: nix_identity.as_ref().map(|(guest, _)| guest.clone()),
        role: nix_identity.map(|(_, role)| role),
    }
}

/// Mirror of the old `nix_closure_volume_identity` derivation over a durable
/// row: ownerRef from the metadata envelope plus the typed VolumeSpec.
fn nix_closure_volume_anchor(
    volume_name: &str,
    owner_ref: Option<&ResourceRef>,
    spec: &VolumeSpec,
) -> Result<(ResourceRef, ZoneNixClosureVolumeRole), String> {
    let mut attachment_guest = None;
    for attachment in spec.attachments() {
        if attachment.execution_ref().resource_type().as_str() != "Guest" {
            return Err("nix-closure attachment must target a Guest".to_owned());
        }
        if attachment_guest
            .replace(attachment.execution_ref().clone())
            .is_some_and(|previous| previous != *attachment.execution_ref())
        {
            return Err("nix-closure attachments must share one Guest".to_owned());
        }
    }
    match (owner_ref, attachment_guest) {
        (Some(owner), None)
            if owner.resource_type().as_str() == "Guest"
                && volume_name == format!("{}-system", owner.name().as_str()) =>
        {
            Ok((owner.clone(), ZoneNixClosureVolumeRole::SystemVolume))
        }
        (None, Some(guest))
            if volume_name == format!("store-view-{}", guest.name().as_str()) =>
        {
            Ok((guest, ZoneNixClosureVolumeRole::StoreView))
        }
        _ => Err("resource does not name a NixClosure volume identity".to_owned()),
    }
}

pub(crate) fn decode_metadata_owner_ref(metadata: &[u8]) -> Option<ResourceRef> {
    let value: serde_json::Value = serde_json::from_slice(metadata).ok()?;
    value
        .get("ownerRef")
        .and_then(serde_json::Value::as_str)
        .and_then(|owner| ResourceRef::parse(owner).ok())
}

fn decode_volume_spec(spec_bytes: &[u8]) -> Option<VolumeSpec> {
    let spec = serde_json::from_slice::<d2b_contracts_resource::v3::ResourceSpec>(spec_bytes).ok()?;
    serde_json::from_slice::<VolumeSpec>(&spec.base().to_canonical_bytes()).ok()
}

/// Map the new store's 16-byte deterministic uid onto the contracts crate's
/// UUIDv4-shaped `ResourceUid` (same mapping the converted drivers use).
fn resource_uid(bytes: &[u8; 16]) -> Result<ResourceUid, ()> {
    let mut bytes = *bytes;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let text = format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7], bytes[8],
        bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15],
    );
    ResourceUid::parse(text).map_err(|_| ())
}

/// The registry key for one volume uid: the canonical uid string.
fn resource_uid_string(bytes: &[u8; 16]) -> String {
    resource_uid(bytes)
        .map(|uid| uid.as_str().to_owned())
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Production socket effect closures (binding/endpoint legs)
// ---------------------------------------------------------------------------

/// Mirror of the provider's `PrivateSocketPath::derive` (frozen v1 worker
/// contract): the private virtiofs socket path for one (volume, guest)
/// serving pair. The rendered accessor is crate-private in the provider
/// today; U14 collapses this mirror behind a provider-owned probe.
pub(crate) fn virtiofs_socket_path(
    socket_runtime_dir: &Path,
    zone: &BoundedToken,
    volume_ref: &ResourceRef,
    execution_ref: &ResourceRef,
) -> Option<PathBuf> {
    if volume_ref.resource_type().as_str() != "Volume"
        || execution_ref.resource_type().as_str() != "Guest"
    {
        return None;
    }
    let root = socket_runtime_dir.to_string_lossy().into_owned();
    if !root.starts_with('/')
        || root.ends_with('/')
        || root.contains('\0')
        || root.contains('\\')
        || root
            .split('/')
            .skip(1)
            .any(|component| component.is_empty() || component == "." || component == "..")
    {
        return None;
    }
    let mut hasher = Sha256::new();
    hasher.update(zone.as_str().as_bytes());
    hasher.update([0u8]);
    hasher.update(volume_ref.name().as_str().as_bytes());
    hasher.update([0u8]);
    hasher.update(execution_ref.name().as_str().as_bytes());
    let digest = hasher.finalize();
    let mut tag = String::with_capacity(8);
    for byte in digest[..4].iter().copied() {
        tag.push(char::from_digit(u32::from(byte >> 4), 16).unwrap_or('0'));
        tag.push(char::from_digit(u32::from(byte & 0x0f), 16).unwrap_or('0'));
    }
    let rendered = format!(
        "{}/vms/{}/vol-{}.vfd.sock",
        root,
        execution_ref.name().as_str(),
        tag
    );
    if rendered.len() > MAX_SOCKET_PATH_BYTES {
        return None;
    }
    Some(PathBuf::from(rendered))
}

fn socket_is_present(path: &Path) -> bool {
    std::fs::metadata(path)
        .map(|metadata| metadata.file_type().is_socket())
        .unwrap_or(false)
}

fn remove_socket_file(path: &Path) -> Result<(), String> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

/// Production binding readiness: derive the private socket path from the
/// registered serving target and probe it on the host target.
#[derive(Clone)]
struct BindingSocketProbe {
    registry: Arc<PlaneResourceRegistry>,
    socket_runtime_dir: PathBuf,
    zone_token: BoundedToken,
}

impl BindingSocketProbe {
    /// Resolve the binding's private socket path; a registry miss loads the
    /// derived-child rows from the authority (the spec store) first.
    async fn path_for(&self, socket: &SocketIdentity) -> Option<PathBuf> {
        let target = self
            .registry
            .socket_target_by_identity(&self.zone_token, socket)
            .await?;
        Some(virtiofs_socket_path(
            &self.socket_runtime_dir,
            &self.zone_token,
            &target.volume_ref,
            &target.execution_ref,
        )?)
    }
}

/// Endpoint socket realization (transport-unix virtiofsd case): the worker
/// Process child binds the private socket; the endpoint's ensure waits a
/// bounded budget for the bind and reports a retryable failure otherwise
/// (the actor owns the retry, R13). The same effect answers the driver's
/// presence probe.
struct SocketWaitEffect {
    registry: Arc<PlaneResourceRegistry>,
    socket_runtime_dir: PathBuf,
    zone_token: BoundedToken,
}

impl SocketWaitEffect {
    /// Resolve the producer's private socket path; a registry miss loads the
    /// derived-child rows from the authority (the spec store) first.
    async fn path_for(&self, producer_ref: &ResourceRef) -> Option<PathBuf> {
        let target = self
            .registry
            .socket_target_by_ref(&self.zone_token, producer_ref)
            .await?;
        Some(virtiofs_socket_path(
            &self.socket_runtime_dir,
            &self.zone_token,
            &target.volume_ref,
            &target.execution_ref,
        )?)
    }

    /// Whether the producer's socket is resolved and bound on the host
    /// target.
    async fn present(&self, producer_ref: &ResourceRef, purpose: &str) -> bool {
        purpose == VIRTIOFSD_PURPOSE
            && self
                .path_for(producer_ref)
                .await
                .map(|path| socket_is_present(&path))
                .unwrap_or(false)
    }
}

#[async_trait::async_trait]
impl AsyncSocketEffect for SocketWaitEffect {
    async fn run(&self, producer_ref: &ResourceRef, purpose: &str) -> Result<(), String> {
        if purpose != VIRTIOFSD_PURPOSE {
            return Err(format!("endpoint purpose {purpose:?} is not realized by the v3 plane"));
        }
        // Resolve the target once (a miss consults the authority); the poll
        // below only re-checks the bound socket on the host target.
        let path = self.path_for(producer_ref).await;
        let deadline = tokio::time::Instant::now() + SOCKET_REALIZE_BUDGET;
        loop {
            if path.as_deref().map(socket_is_present).unwrap_or(false) {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err("virtiofsd socket not bound within its realize budget".to_owned());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}

/// Endpoint removal - endpoint-first teardown, idempotent under retry (R10).
struct SocketRemoveEffect {
    registry: Arc<PlaneResourceRegistry>,
    socket_runtime_dir: PathBuf,
    zone_token: BoundedToken,
}

#[async_trait::async_trait]
impl AsyncSocketEffect for SocketRemoveEffect {
    async fn run(&self, producer_ref: &ResourceRef, purpose: &str) -> Result<(), String> {
        if purpose != VIRTIOFSD_PURPOSE {
            return Err(format!("endpoint purpose {purpose:?} is not realized by the v3 plane"));
        }
        match self.path_for(producer_ref).await {
            Some(path) => remove_socket_file(&path),
            // Unknown producer: nothing was realized on this target.
            None => Ok(()),
        }
    }
}

impl SocketRemoveEffect {
    /// Resolve the producer's private socket path; a registry miss loads the
    /// derived-child rows from the authority (the spec store) first.
    async fn path_for(&self, producer_ref: &ResourceRef) -> Option<PathBuf> {
        let target = self
            .registry
            .socket_target_by_ref(&self.zone_token, producer_ref)
            .await?;
        Some(virtiofs_socket_path(
            &self.socket_runtime_dir,
            &self.zone_token,
            &target.volume_ref,
            &target.execution_ref,
        )?)
    }
}

// ---------------------------------------------------------------------------
// Production volume leg
// ---------------------------------------------------------------------------

/// Trusted daemon-side root resolver for the new plane: per-zone inputs,
/// per-resource anchors resolved through the [`PlaneResourceRegistry`].
/// Storage-policy and NixClosure legs mirror the old
/// `DaemonVolumeRootResolver` (U14 collapses the old one with the old
/// reconciler).
#[derive(Clone)]
pub struct ZoneVolumeRootResolver {
    state: Arc<crate::ServerState>,
    resolver: BundleResolver,
    zone: ZoneId,
    marker_root: PathBuf,
    registry: Arc<PlaneResourceRegistry>,
}

impl ZoneVolumeRootResolver {
    fn source_unresolved(
        &self,
        stage: &'static str,
        volume_name: &str,
    ) -> d2b_provider_volume_local::VolumeLocalError {
        tracing::warn!(
            zone = %self.zone.as_str(),
            volume = %volume_name,
            stage,
            "v3 Volume source resolution failed"
        );
        d2b_provider_volume_local::VolumeLocalError::SourceUnresolved
    }

    fn sync_store_view(
        &self,
        guest_ref: &ResourceRef,
        intent: &ResolvedStoreViewIntent,
        generation_token: u32,
        volume_name: &str,
    ) -> Result<d2b_contracts_broker::broker_wire::StoreSyncResponse, d2b_provider_volume_local::VolumeLocalError>
    {
        let response = crate::dispatch_broker_request_as(
            &self.state,
            BrokerRequest::StoreSync(StoreSyncRequest {
                vm_id: d2b_contracts::types::VmId::new(guest_ref.name().as_str()),
                bundle_closure_ref: d2b_contracts::types::BundleClosureRef::new(
                    intent.intent_id.clone(),
                ),
                generation_token,
                tracing_span_id: None,
            }),
            BrokerCallerRole::AdminUid {
                uid: self.state.daemon_uid,
            },
        )
        .map_err(|error| {
            tracing::warn!(
                zone = %self.zone.as_str(),
                volume = %volume_name,
                error = ?error,
                "v3 volume store sync broker dispatch failed"
            );
            d2b_provider_volume_local::VolumeLocalError::EffectFailed
        })?;
        match response {
            BrokerResponse::StoreSync(response) => Ok(response),
            _ => {
                tracing::warn!(
                    zone = %self.zone.as_str(),
                    volume = %volume_name,
                    "v3 volume store sync broker returned unexpected response type"
                );
                Err(d2b_provider_volume_local::VolumeLocalError::EffectFailed)
            }
        }
    }

    fn resolve_nix_closure_root(
        &self,
        volume_uid: &ResourceUid,
        anchor: &VolumeAnchor,
        system_artifact_id: &BoundedToken,
    ) -> Result<crate::resource_runtime::ResolvedVolumeRoot, d2b_provider_volume_local::VolumeLocalError> {
        let Some(guest_ref) = anchor.guest_ref.as_ref() else {
            return Err(self.source_unresolved("guest-reference", &anchor.volume_name));
        };
        let descriptor = self
            .resolver
            .guest_setup_descriptor_bytes(self.zone.as_str(), guest_ref.name().as_str())
            .ok_or_else(|| self.source_unresolved("guest-setup-descriptor", &anchor.volume_name))?;
        let descriptor: serde_json::Value = serde_json::from_slice(descriptor)
            .map_err(|_| self.source_unresolved("guest-setup-descriptor-decode", &anchor.volume_name))?;
        let descriptor_artifact_id = descriptor
            .get("systemArtifactId")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| self.source_unresolved("guest-setup-artifact-id", &anchor.volume_name))?;
        let intent = self
            .resolver
            .find_store_view_intent_for_zone(&self.zone, guest_ref.name().as_str())
            .ok_or_else(|| self.source_unresolved("store-view-intent", &anchor.volume_name))?;
        if guest_ref.resource_type().as_str() != "Guest"
            || intent.vm != guest_ref.name().as_str()
            || intent.intent_id != intent_id_store_view(&self.zone, guest_ref.name().as_str())
            || descriptor_artifact_id != system_artifact_id.as_str()
        {
            return Err(self.source_unresolved("store-view-identity", &anchor.volume_name));
        }
        let generation_token = u32::try_from(intent.generation)
            .map_err(|_| self.source_unresolved("store-view-generation", &anchor.volume_name))?;
        let response =
            self.sync_store_view(guest_ref, &intent, generation_token, &anchor.volume_name)?;
        let expected_generation_id = d2b_host::hardlink_farm::generation_id(
            &intent.closure_paths,
            d2b_host::hardlink_farm::system_store_path(&intent.closure_paths),
        );
        let farm_path = PathBuf::from(&response.hardlink_farm_path);
        if response.vm != intent.vm
            || response.generation_id != expected_generation_id
            || response.generation_token != generation_token
            || response.closure_count
                != u32::try_from(intent.closure_paths.len()).unwrap_or(u32::MAX)
            || farm_path != intent.hardlink_farm_path
        {
            return Err(self.source_unresolved("store-sync-response", &anchor.volume_name));
        }
        let file = open_anchored_directory(&farm_path)
            .map_err(|_| self.source_unresolved("store-view-open", &anchor.volume_name))?;
        let marker_root = open_anchored_directory(&self.marker_root)
            .map_err(|_| self.source_unresolved("marker-root", &anchor.volume_name))?;
        Ok(crate::resource_runtime::ResolvedVolumeRoot::new(file, volume_uid.clone())?
            .with_marker_root(marker_root)?
            .with_preexisting_state())
    }
}

impl crate::resource_runtime::VolumeRootResolver for ZoneVolumeRootResolver {
    fn resolve_root(
        &self,
        volume_uid: &ResourceUid,
        source_policy_id: Option<&BoundedToken>,
        system_artifact_id: Option<&BoundedToken>,
        kind: SourceKind,
    ) -> Result<crate::resource_runtime::ResolvedVolumeRoot, d2b_provider_volume_local::VolumeLocalError> {
        let Some(anchor) = self.registry.lookup_anchor(volume_uid) else {
            return Err(self.source_unresolved("volume-anchor", "?"));
        };
        if kind == SourceKind::NixClosure {
            if source_policy_id.is_some() {
                return Err(d2b_provider_volume_local::VolumeLocalError::InvalidSpec);
            }
            let Some(system_artifact_id) = system_artifact_id else {
                return Err(self.source_unresolved("system-artifact", &anchor.volume_name));
            };
            return self.resolve_nix_closure_root(volume_uid, &anchor, system_artifact_id);
        }
        let policy = source_policy_id
            .map(BoundedToken::as_str)
            .ok_or_else(|| self.source_unresolved("storage-policy", &anchor.volume_name))?;
        let storage_id = if policy == "state-root" || policy == "default-state" {
            "path:state-root".to_owned()
        } else {
            format!("path:{policy}")
        };
        let path = self
            .resolver
            .find_storage_path_spec(&storage_id)
            .map(|spec| spec.path_template.as_str().to_owned())
            .ok_or_else(|| self.source_unresolved("storage-path", &anchor.volume_name))?;
        let path = Path::new(&path);
        if !path.is_absolute()
            || path
                .components()
                .any(|component| matches!(component, std::path::Component::ParentDir))
        {
            return Err(self.source_unresolved("storage-path-safety", &anchor.volume_name));
        }
        let file = open_anchored_directory(path)
            .map_err(|_| self.source_unresolved("storage-path-open", &anchor.volume_name))?;
        let name = anchor.volume_name.as_str();
        if name.is_empty() || name == "." || name == ".." {
            return Err(self.source_unresolved("storage-subdir-name", &anchor.volume_name));
        }
        match rustix::fs::mkdirat(&file, name, Mode::from_raw_mode(0o700)) {
            Ok(()) => {}
            Err(error) if error == rustix::io::Errno::EXIST => {}
            Err(_) => return Err(self.source_unresolved("storage-subdir-create", &anchor.volume_name)),
        }
        let file = openat2(
            &file,
            name,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::empty(),
            ResolveFlags::BENEATH
                | ResolveFlags::NO_SYMLINKS
                | ResolveFlags::NO_MAGICLINKS
                | ResolveFlags::NO_XDEV,
        )
        .map_err(|_| self.source_unresolved("storage-subdir-open", &anchor.volume_name))?;
        let marker_file = open_anchored_directory(&self.marker_root)
            .map_err(|_| self.source_unresolved("marker-root", &anchor.volume_name))?;
        crate::resource_runtime::ResolvedVolumeRoot::new(file.into(), volume_uid.clone())?
            .with_marker_root(marker_file.into())
    }

    fn resolve_principal(
        &self,
        reference: &ResourceRef,
    ) -> Result<u32, d2b_provider_volume_local::VolumeLocalError> {
        if reference.resource_type().as_str() != "User" {
            return Err(d2b_provider_volume_local::VolumeLocalError::InvalidSpec);
        }
        nix::unistd::User::from_name(reference.name().as_str())
            .map_err(|_| d2b_provider_volume_local::VolumeLocalError::EffectFailed)?
            .map(|user| user.uid.as_raw())
            .ok_or(d2b_provider_volume_local::VolumeLocalError::EffectFailed)
    }
}

fn open_anchored_directory(path: &Path) -> std::io::Result<std::os::fd::OwnedFd> {
    if !path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "unanchored directory",
        ));
    }
    let mut current = open(
        Path::new("/"),
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::empty(),
    )
    .map_err(|error| std::io::Error::from_raw_os_error(error.raw_os_error()))?;
    for component in path.components() {
        let std::path::Component::Normal(name) = component else {
            continue;
        };
        current = openat2(
            &current,
            name,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::empty(),
            ResolveFlags::BENEATH
                | ResolveFlags::NO_SYMLINKS
                | ResolveFlags::NO_MAGICLINKS
                | ResolveFlags::NO_XDEV,
        )
        .map_err(|error| std::io::Error::from_raw_os_error(error.raw_os_error()))?;
    }
    Ok(current)
}

/// The production volume effects: the controller closure rebuilds the
/// preserved `VolumeLocalController` over anchored adapters per call
/// (exactly the old `reconcile_volume` construction) over the per-zone
/// resolver; the state closure probes the volume-local marker as the
/// durable layout evidence (old recover probe).
pub(crate) fn production_volume_effects(
    state: &Arc<crate::ServerState>,
    zone: ZoneId,
    resolver: BundleResolver,
    registry: Arc<PlaneResourceRegistry>,
) -> ProductionVolumeDriverEffects<
    crate::resource_runtime::AnchoredVolumeEffectAdapter<ZoneVolumeRootResolver>,
    crate::resource_runtime::AnchoredVolumeEffectAdapter<ZoneVolumeRootResolver>,
> {
    let marker_root = state
        .daemon_state_dir
        .parent()
        .unwrap_or(state.daemon_state_dir.as_path())
        .join("volume-local-markers");
    let closure_resolver = ZoneVolumeRootResolver {
        state: Arc::clone(state),
        resolver: resolver.clone(),
        zone: zone.clone(),
        marker_root: marker_root.clone(),
        registry,
    };
    ProductionVolumeDriverEffects::new(
        Arc::new(move || {
            let source = crate::resource_runtime::AnchoredVolumeEffectAdapter::new(
                closure_resolver.clone(),
            );
            let layout = crate::resource_runtime::AnchoredVolumeEffectAdapter::new(
                closure_resolver.clone(),
            );
            VolumeLocalController::new(VolumeLocalProfile::shipped(), source, layout)
        }),
        Arc::new(move |volume_uid: &ResourceUid| {
            // Layout-state probe (old recover): the volume-local marker is
            // the durable evidence an initialized layout left behind.
            marker_root.join(volume_uid.as_str()).exists()
        }),
    )
}

// ---------------------------------------------------------------------------
// Construction inputs
// ---------------------------------------------------------------------------

/// KTD7 zone-authority inputs: derived from the bundle resolver and
/// `ZoneAuthorityIdentity`, never from the spec store.
#[derive(Clone)]
pub struct ZoneAuthorityInputs {
    /// Zone authority uid (`ZoneAuthorityIdentity::zone_uid`).
    pub zone_uid: Option<ResourceUid>,
    /// Zone policy revision from the authority path (KTD6).
    pub policy_revision: Option<u64>,
    /// Provider assignment generation for guest execution sessions.
    pub provider_assignment_generation: Option<d2b_contracts_resource::v3::ResourceGeneration>,
    pub controller_generation: ControllerGeneration,
    pub guest_execution: Option<d2b_process_conformance::GuestExecutionBinding>,
    pub mode: DaemonMode,
    /// Target Guest vcpu count for binding worker thread pools (KTD7); the
    /// merge owner derives it from the zone bundle's Guest vcpus (the old
    /// path defaulted to one on a missing guest).
    pub vcpu_count: u32,
}

/// Everything U9 needs to assemble one zone's new plane. The production
/// constructor ([`ConstructionInputs::production`]) wires the production
/// effects from the same sources the old plane composes; tests inject the
/// U6/U7 fake effect ports instead.
pub struct ConstructionInputs {
    pub zone: ZoneId,
    pub zone_token: BoundedToken,
    /// The zone's daemon-owned spec-store directory
    /// (`<daemon-state>/zones/<zone>`); the spec store lives at
    /// `spec-store.sqlite3` underneath it.
    pub spec_store_dir: PathBuf,
    /// Absolute runtime root the virtiofs worker-template exports resolve
    /// under (`PrivateSocketPath::derive` input); production derives it
    /// from the broker socket's parent directory.
    pub socket_runtime_dir: PathBuf,
    pub authority: ZoneAuthorityInputs,
    /// Committed `Provider` identities (KTD7) keyed by canonical reference,
    /// resolved by the composition unit from the old plane's durable
    /// authority (unconverted `Provider` rows pass through to its store, so
    /// this plane's spec store never carries them). The plane publishes them
    /// into [`PlaneResourceRegistry`] before its manager spawns any resource
    /// actor; a reference absent here stays unpublished and its controller
    /// rows refuse closed.
    pub committed_provider_identities:
        BTreeMap<ResourceRef, (ResourceUid, d2b_contracts_resource::v3::ResourceGeneration)>,
    /// Shared per-zone registry the production effects resolve per-resource
    /// anchors from; the plane re-populates it from the spec store.
    pub registry: Arc<PlaneResourceRegistry>,
    pub process_effects: Arc<dyn ProcessDriverEffects>,
    pub volume_effects: Arc<dyn VolumeDriverEffects>,
    pub binding_effects: Arc<dyn BindingDriverEffects>,
    pub endpoint_effects: Arc<dyn crate::endpoint_driver::EndpointDriverEffects>,
    pub activation_effects: Arc<dyn ActivationDriverEffects>,
    pub credential_effects: Arc<dyn CredentialDriverEffects>,
    pub shared_provider_effects: Arc<dyn SharedProviderDriverEffects>,
}

/// Production construction for one zone under `open_resource_plane`: reuse
/// the daemon-attached fixed process providers, the bundle resolver and the
/// broker dispatch the old plane composes, and derive the zone state and
/// socket runtime roots the same way composition does.
impl ConstructionInputs {
    pub fn production(
        state: &Arc<crate::ServerState>,
        zone: ZoneId,
        authority: &d2bd_runtime::zone_authority::ZoneAuthorityIdentity,
        resolver: BundleResolver,
        credential_effects: Arc<dyn CredentialDriverEffects>,
        committed_provider_identities: BTreeMap<
            ResourceRef,
            (ResourceUid, d2b_contracts_resource::v3::ResourceGeneration),
        >,
    ) -> Result<Self, PlaneError> {
        // The spec store is daemon-owned, so it lives under the daemon's own
        // state root - never in the broker-provisioned
        // `<state-root>/zones/<zone>` directory, which the daemon may
        // traverse but not write.
        let spec_store_dir = state.daemon_state_dir.join("zones").join(zone.as_str());
        let broker_socket = crate::broker_socket_path(state);
        let socket_runtime_dir = broker_socket
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("/run/d2b"));
        let zone_token = BoundedToken::parse(zone.as_str().to_owned()).map_err(|_| {
            PlaneError::Authority(format!("zone {} is not a bounded token", zone.as_str()))
        })?;
        // Reuse the daemon's shared, already-composed fixed Process
        // Providers; compose and attach once when absent (identical inputs
        // to the old composition path at composition.rs:3653).
        let process_providers = match state.provider_runtime.process_providers() {
            Some(providers) => providers,
            None => {
                let providers = Arc::new(
                    crate::process_provider_runtime::ProductionProcessProviders::new(
                        resolver.clone(),
                        crate::broker_socket_path(state),
                        BrokerCallerRole::AdminUid {
                            uid: state.daemon_uid,
                        },
                    ),
                );
                let _ = state.provider_runtime.attach_process_providers(Arc::clone(&providers));
                providers
            }
        };
        let registry = Arc::new(PlaneResourceRegistry::new());
        let controller_generation = ControllerGeneration::new(1)
            .map_err(|error| PlaneError::Authority(error.to_string()))?;
        let endpoint_socket_runtime_dir = socket_runtime_dir.clone();
        let endpoint_zone_token = zone_token.clone();
        let probe = BindingSocketProbe {
            registry: Arc::clone(&registry),
            socket_runtime_dir: socket_runtime_dir.clone(),
            zone_token: zone_token.clone(),
        };
        let registry_source = Arc::clone(&registry);
        Ok(Self {
            zone: zone.clone(),
            zone_token,
            spec_store_dir,
            socket_runtime_dir,
            authority: ZoneAuthorityInputs {
                zone_uid: Some(authority.zone_uid().clone()),
                policy_revision: None,
                provider_assignment_generation: None,
                controller_generation,
                guest_execution: None,
                mode: DaemonMode::Host,
                vcpu_count: 1,
            },
            committed_provider_identities,
            registry: Arc::clone(&registry),
            process_effects: Arc::new(
                ProductionProcessDriverEffects::new(process_providers)
                    .with_committed_provider_identities(registry_source),
            ),
            volume_effects: Arc::new(production_volume_effects(state, zone.clone(), resolver, Arc::clone(&registry))),
            binding_effects: Arc::new(ProductionBindingDriverEffects::new(
                Arc::new({
                    let probe = probe.clone();
                    move |socket: &SocketIdentity| {
                        let probe = probe.clone();
                        Box::pin(async move {
                            probe
                                .path_for(socket)
                                .await
                                .map(|path| socket_is_present(&path))
                                .unwrap_or(false)
                        })
                    }
                }),
                Arc::new({
                    let probe = probe;
                    move |socket: &SocketIdentity| {
                        let probe = probe.clone();
                        Box::pin(async move {
                            match probe.path_for(socket).await {
                                Some(path) => remove_socket_file(&path),
                                None => Ok(()),
                            }
                        })
                    }
                }),
            )),
            endpoint_effects: {
                let wait = Arc::new(SocketWaitEffect {
                    registry: Arc::clone(&registry),
                    socket_runtime_dir: endpoint_socket_runtime_dir.clone(),
                    zone_token: endpoint_zone_token.clone(),
                });
                let present = Arc::clone(&wait);
                Arc::new(crate::endpoint_driver::ProductionEndpointDriverEffects::new(
                    Arc::new(move |producer_ref: &ResourceRef, purpose: &str| {
                        let present = Arc::clone(&present);
                        Box::pin(async move { present.present(producer_ref, purpose).await })
                    }),
                    wait,
                    Arc::new(SocketRemoveEffect {
                        registry: Arc::clone(&registry),
                        socket_runtime_dir: endpoint_socket_runtime_dir,
                        zone_token: endpoint_zone_token,
                    }),
                ))
            },
            activation_effects: Arc::new(ProductionActivationDriverEffects::new(Arc::clone(state))),
            credential_effects,
            shared_provider_effects: Arc::new(ProductionSharedProviderEffects::new(
                Arc::clone(state),
                zone.clone(),
                controller_generation,
            )),
        })
    }
}

// ---------------------------------------------------------------------------
// Manager-boundary admission (U9/U10 subjects)
// ---------------------------------------------------------------------------

/// Manager-boundary admission for the new plane (KTD2 execution decision):
/// Nix ingestion presents the bundle subject (`nix:<generation>`), the API
/// path (U8) presents the api caller subject, owned cascades present the
/// resource-owner subject. Defense in depth for the type partition:
/// mutations against unconverted types are denied - the new plane never
/// serves them (U9 notes the API/resource-owner subject call sites for the
/// merge owner's U8 wiring).
struct PlaneMutationAdmission;

impl MutationAdmission for PlaneMutationAdmission {
    fn admit(&self, _subject: &MutationSubject, request: &MutationRequest) -> AdmissionDecision {
        if route_resource_type(&request.key.type_name) != PlaneRoute::NewPlane {
            return AdmissionDecision::Deny(format!(
                "resource type {} is not served by the v3 plane",
                request.key.type_name
            ));
        }
        AdmissionDecision::Allow
    }
}

/// Default spec decode hook for rows no per-type decoder covers (unconverted
/// rows persist in the new store per KTD4 but never spawn actors, so this
/// only ever sees converted types in error paths).
struct PassthroughDecoder;

impl SpecDecoder for PassthroughDecoder {
    fn decode(
        &self,
        envelope: &[u8],
    ) -> Result<Box<dyn Any + Send>, Box<dyn std::error::Error + Send + Sync>> {
        Ok(Box::new(envelope.to_vec()))
    }
}

// ---------------------------------------------------------------------------
// ResourcePlaneV3: the per-zone new plane (U9)
// ---------------------------------------------------------------------------

/// Assembly failures (U9).
#[derive(Debug, thiserror::Error)]
pub enum PlaneError {
    #[error("spec store open failed: {0}")]
    SpecStore(#[from] d2b_resource_runtime::spec_store::SpecStoreError),
    #[error("provider registration failed: {0}")]
    ProviderRegistration(
        #[from] d2b_resource_runtime::provider::ProviderDirectoryError,
    ),
    #[error("manager spawn failed: {0}")]
    ManagerSpawn(String),
    #[error("manager rpc failed: {0}")]
    ManagerRpc(#[from] ResourceError),
    #[error("zone authority inputs invalid: {0}")]
    Authority(String),
    #[error("bundle invalid: {0}")]
    Bundle(String),
}

/// The per-zone v3 resource plane: spec store, provider directory, watch
/// hub, manager actor, and the readiness checklist (U9, KTD5, R27).
pub struct ResourcePlaneV3 {
    zone: ZoneId,
    zone_token: BoundedToken,
    store_path: PathBuf,
    store: Arc<SpecStore>,
    hub: Arc<WatchHub>,
    registry: Arc<PlaneResourceRegistry>,
    client: ResourceManagerClient,
    readiness: Arc<NewPlaneReadinessState>,
}

impl core::fmt::Debug for ResourcePlaneV3 {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("ResourcePlaneV3")
            .field("zone", &self.zone)
            .finish_non_exhaustive()
    }
}

impl ResourcePlaneV3 {
    /// The per-zone spec store path decision (documented in the module
    /// header): `spec-store.sqlite3` under the daemon-owned
    /// `<daemon-state>/zones/<zone>` directory.
    pub fn spec_store_path(spec_store_dir: &Path) -> PathBuf {
        spec_store_dir.join("spec-store.sqlite3")
    }

    fn build_providers(inputs: &ConstructionInputs) -> Result<ProviderDirectory, PlaneError> {
        let mut providers = ProviderDirectory::new();
        providers.register(Arc::new(ProcessDriverFactory::new(ProcessDriverArgs {
            zone: inputs.zone.clone(),
            effects: Arc::clone(&inputs.process_effects),
            zone_uid: inputs.authority.zone_uid.clone(),
            policy_revision: inputs.authority.policy_revision,
            provider_assignment_generation: inputs.authority.provider_assignment_generation,
            controller_generation: inputs.authority.controller_generation,
            guest_execution: inputs.authority.guest_execution.clone(),
            mode: inputs.authority.mode,
        })))?;
        providers.register(Arc::new(crate::volume_driver::VolumeDriverFactory::new(
            VolumeDriverArgs {
                zone: inputs.zone.as_str().to_owned(),
                effects: Arc::clone(&inputs.volume_effects),
            },
        )))?;
        providers.register(Arc::new(BindingDriverFactory::new(BindingDriverArgs {
            zone: inputs.zone.as_str().to_owned(),
            effects: Arc::clone(&inputs.binding_effects),
            vcpu_count: inputs.authority.vcpu_count,
        })))?;
        providers.register(Arc::new(EndpointDriverFactory::new(EndpointDriverArgs {
            zone: inputs.zone.as_str().to_owned(),
            effects: Arc::clone(&inputs.endpoint_effects),
        })))?;
        providers.register(Arc::new(CredentialDriverFactory::new(CredentialDriverArgs {
            zone: inputs.zone.as_str().to_owned(),
            controller_generation: inputs.authority.controller_generation,
            effects: Arc::clone(&inputs.credential_effects),
        })))?;
        providers.register(Arc::new(ActivationDriverFactory::new(ActivationDriverArgs {
            zone: inputs.zone.as_str().to_owned(),
            effects: Arc::clone(&inputs.activation_effects),
            verifier: Arc::new(d2b_provider_activation_nixos::FailClosedActivationVerifier),
        })))?;
        providers.register(Arc::new(TelemetryDriverFactory::new()))?;
        providers.register(Arc::new(SharedProviderDriverFactory::new(
            SharedProviderDriverArgs {
                zone: inputs.zone.as_str().to_owned(),
                controller_generation: inputs.authority.controller_generation,
                effects: Arc::clone(&inputs.shared_provider_effects),
            },
        )))?;
        providers.register(Arc::new(SystemCoreDriverFactory::new()))?;
        Ok(providers)
    }

    fn decoders() -> HashMap<ResourceTypeName, Arc<dyn SpecDecoder>> {
        let mut decoders = HashMap::new();
        decoders.insert(ResourceTypeName::new("Process"), process_spec_decoder());
        decoders.insert(ResourceTypeName::new("Volume"), volume_spec_decoder());
        decoders.insert(ResourceTypeName::new("VolumeBinding"), binding_spec_decoder());
        decoders.insert(ResourceTypeName::new("Endpoint"), endpoint_spec_decoder());
        decoders.insert(
            ResourceTypeName::new(crate::activation_driver::ACTIVATION_TYPE_NAME),
            activation_spec_decoder(),
        );
        decoders.insert(
            ResourceTypeName::new(TELEMETRY_SERVICE_TYPE),
            telemetry_spec_decoder(),
        );
        decoders.insert(
            ResourceTypeName::new(TELEMETRY_BINDING_TYPE),
            telemetry_spec_decoder(),
        );
        decoders.insert(
            ResourceTypeName::new("Credential"),
            credential_spec_decoder(),
        );
        for resource_type in crate::shared_provider_driver::SHARED_PROVIDER_TYPES {
            decoders.insert(
                ResourceTypeName::new(resource_type),
                shared_provider_spec_decoder(),
            );
        }
        decoders.insert(
            ResourceTypeName::new("Host"),
            system_core_spec_decoder(),
        );
        decoders.insert(
            ResourceTypeName::new("User"),
            system_core_spec_decoder(),
        );
        decoders
    }

    /// Open the store, register the converted-type factories, and
    /// spawn the manager. Initial-load completion is a separate step so the
    /// readiness checklist is observable stage by stage; [`Self::open`]
    /// composes both.
    pub fn prepare(inputs: ConstructionInputs) -> Result<Self, PlaneError> {
        let readiness = Arc::new(NewPlaneReadinessState::new());
        // Stage 1: durable spec store.
        let store_path = Self::spec_store_path(&inputs.spec_store_dir);
        if let Some(parent) = store_path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                PlaneError::Authority(format!("spec store dir create failed: {error}"))
            })?;
        }
        let store = Arc::new(SpecStore::open(store_path.clone())?);
        // The registry caches store-derived rows for the production effects;
        // the store is the authority its socket-target lookups load from on
        // a miss (the manager mints derived children after `open`).
        inputs.registry.attach_store(Arc::clone(&store));
        // KTD7: publish the committed Provider identities before the manager
        // spawns any resource actor (restart recovery spawns one per durable
        // row), so a controller row's first reconcile never observes its
        // owning Provider unbound.
        for (provider_ref, (uid, generation)) in &inputs.committed_provider_identities {
            inputs
                .registry
                .register_committed_provider_identity(provider_ref, uid.clone(), *generation);
        }
        readiness.set_spec_store_ready(true);
        // Stage 2: provider directory with production effects wired.
        let providers = Self::build_providers(&inputs)?;
        readiness.set_providers_registered(true);
        // Stage 3: per-zone manager spawn (KTD5).
        let hub = Arc::new(WatchHub::new(&d2b_resource_runtime::revision::SystemClock, DEFAULT_RING_CAPACITY));
        let args = ResourceManagerArgs {
            zone: inputs.zone.as_str().to_owned(),
            store: Arc::clone(&store),
            providers,
            hub: Arc::clone(&hub),
            admission: Arc::new(PlaneMutationAdmission),
            decoders: Self::decoders(),
            default_decoder: Arc::new(PassthroughDecoder),
            backoff: PLANE_BACKOFF,
        };
        let runtime = tokio::runtime::Handle::try_current()
            .map_err(|_| PlaneError::ManagerSpawn("resource plane requires a tokio runtime".into()))?;
        let (actor, _join) = tokio::task::block_in_place(|| {
            runtime.block_on(async {
                ractor::Actor::spawn(None, ResourceManager::new(), args).await
            })
        })
        .map_err(|error| PlaneError::ManagerSpawn(error.to_string()))?;
        readiness.set_manager_started(true);
        readiness.set_spec_store_ready(true);
        Ok(Self {
            zone: inputs.zone.clone(),
            zone_token: inputs.zone_token,
            store_path,
            store,
            hub,
            registry: inputs.registry,
            client: ResourceManagerClient::new(actor),
            readiness,
        })
    }

    /// The initial desired load (F2): the manager's pre_start loaded every
    /// durable row and spawned its actor; this round-trip confirms the
    /// manager is serving, then the plane re-registers the durable anchors
    /// for the production effects.
    pub async fn complete_initial_load(&self) -> Result<(), PlaneError> {
        self.registry
            .load_from_store(&self.zone_token, &self.store)
            .await?;
        let views = self
            .client
            .list(ResourceSelector {
                zone: Some(self.zone.as_str().to_owned()),
                ..ResourceSelector::default()
            })
            .await?;
        tracing::debug!(
            zone = %self.zone.as_str(),
            resources = views.len(),
            "v3 resource plane initial load complete"
        );
        self.readiness.set_initial_load_complete(true);
        Ok(())
    }

    /// One-shot assembly (production path): prepare + initial load.
    pub async fn open(inputs: ConstructionInputs) -> Result<Self, PlaneError> {
        let plane = Self::prepare(inputs)?;
        plane.complete_initial_load().await?;
        Ok(plane)
    }

    pub fn zone(&self) -> &ZoneId {
        &self.zone
    }

    pub fn readiness(&self) -> d2bd_runtime::resource_runtime_support::NewPlaneReadiness {
        self.readiness.snapshot()
    }

    /// The readiness checklist handle (U9): shared state the composition
    /// can observe while the plane opens.
    pub fn readiness_state(&self) -> Arc<NewPlaneReadinessState> {
        Arc::clone(&self.readiness)
    }

    /// The manager caller facade (U8 wires the Resource API onto this
    /// through `ManagerBackend::new`).
    pub fn client(&self) -> &ResourceManagerClient {
        &self.client
    }

    /// The in-memory watch hub (U8 pairs it with the client in
    /// `ManagerBackend`; ManagerWatch/ManagerWatchStreams hand off the
    /// external WATCH streams, KTD8).
    pub fn hub(&self) -> &Arc<WatchHub> {
        &self.hub
    }

    pub fn store(&self) -> &Arc<SpecStore> {
        &self.store
    }

    pub fn registry(&self) -> &Arc<PlaneResourceRegistry> {
        &self.registry
    }

    pub async fn shutdown(&self) {
        let _ = self.client.actor().get_cell().stop(None);
    }
}

// ---------------------------------------------------------------------------
// U10: Nix ingestion into the manager (R26, F1)
// ---------------------------------------------------------------------------

/// The type-partitioned Nix bundle ingest plan (U10).
pub struct BundleIngestPlan {
    /// Converted rows to route through `ResourceManager::Apply` with
    /// provenance `Nix` under the bundle subject.
    pub apply: Vec<DesiredResource>,
    /// Durable Nix-provenance rows of converted types that the new bundle
    /// no longer declares: configuration changes mark them deleting (R26).
    pub remove: Vec<ResourceKey>,
    /// Durable API-provenance rows the bundle names: never touched by a
    /// Nix apply (provenance respected; the API owns them).
    pub api_protected: Vec<ResourceKey>,
    /// Unconverted bundle rows for the existing materialization path.
    pub pass_through: Vec<BundleResource>,
}

impl BundleIngestPlan {
    /// The pass-through bundle for the old `materialize_zone_resource_bundle`
    /// path, rebuilt through the canonical bundle constructor so its
    /// integrity pin stays verifiable.
    pub fn old_bundle(&self, bundle: &ResourceBundle) -> Result<ResourceBundle, PlaneError> {
        let rebuilt = ResourceBundle::new(
            bundle.zone.clone(),
            self.pass_through.clone(),
            bundle.integrity.artifact_catalog_digest.clone(),
            bundle.integrity.schema_fingerprints.clone(),
            bundle.integrity.provider_schema_digests.clone(),
            bundle.generated_at.clone(),
        )
        .map_err(|error| PlaneError::Bundle(error.to_string()))?;
        Ok(match bundle.zone_uid.clone() {
            Some(zone_uid) => rebuilt.with_zone_uid(zone_uid),
            None => rebuilt,
        })
    }
}

/// The old-plane view of a bundle: converted types removed so the legacy
/// materialization path never sees them (exclusive partition without a
/// store handle - the manager is the only writer for converted types).
pub fn old_plane_bundle(bundle: &ResourceBundle) -> Result<ResourceBundle, PlaneError> {
    let pass_through: Vec<BundleResource> = bundle
        .resources
        .iter()
        .filter(|resource| {
            route_resource_type(resource.resource_type().as_str()) == PlaneRoute::OldPlane
        })
        .cloned()
        .collect();
    let rebuilt = ResourceBundle::new(
        bundle.zone.clone(),
        pass_through,
        bundle.integrity.artifact_catalog_digest.clone(),
        bundle.integrity.schema_fingerprints.clone(),
        bundle.integrity.provider_schema_digests.clone(),
        bundle.generated_at.clone(),
    )
    .map_err(|error| PlaneError::Bundle(error.to_string()))?;
    Ok(match bundle.zone_uid.clone() {
        Some(zone_uid) => rebuilt.with_zone_uid(zone_uid),
        None => rebuilt,
    })
}

/// Partition one verified Zone bundle per the Phase A type partition (U10):
/// converted rows become manager applies/removals; unconverted rows flow to
/// the old materialization path unchanged. API-provenance rows of converted
/// types are never clobbered by a Nix apply.
pub async fn partition_nix_bundle(
    zone: &ZoneId,
    bundle: &ResourceBundle,
    store: &SpecStore,
) -> Result<BundleIngestPlan, PlaneError> {
    bundle.verify().map_err(|error| PlaneError::Bundle(error.to_string()))?;
    let durable_by_key: HashMap<ResourceKey, StoredDesiredResource> = store
        .list(SpecSelector {
            zone: Some(zone.as_str().to_owned()),
            type_name: None,
            owner_uid: None,
        })
        .await?
        .into_iter()
        .map(|row| (row.key.clone(), row))
        .collect();
    let mut apply = Vec::new();
    let mut api_protected = Vec::new();
    let mut pass_through = Vec::new();
    for row in &bundle.resources {
        let key = ResourceKey::new(
            zone.as_str(),
            row.resource_type().as_str(),
            row.metadata().name().as_str(),
        );
        match route_resource_type(row.resource_type().as_str()) {
            PlaneRoute::NewPlane => match durable_by_key.get(&key) {
                // API-created rows are never clobbered by a Nix apply
                // (R26: provenance is respected).
                Some(existing) if existing.provenance == d2b_resource_runtime::spec_store::ResourceProvenance::Api => {
                    api_protected.push(key);
                }
                Some(existing) if existing.deleting => {
                    // Already retiring; a re-declare re-adds on the next
                    // bundle once the deletion completed.
                }
                _ => apply.push(bundle_desired(zone, row)),
            },
            PlaneRoute::OldPlane => pass_through.push(row.clone()),
        }
    }
    // Removed Nix rows of converted types: mark deleting (R26).
    let declared: HashMap<ResourceKey, ()> = bundle
        .resources
        .iter()
        .map(|row| (bundle_row_key(zone, row), ()))
        .collect();
    let mut remove = Vec::new();
    for (key, existing) in &durable_by_key {
        if route_resource_type(&key.type_name) != PlaneRoute::NewPlane {
            continue;
        }
        if existing.provenance != d2b_resource_runtime::spec_store::ResourceProvenance::Nix
            || existing.deleting
            || declared.contains_key(key)
        {
            continue;
        }
        remove.push(key.clone());
    }
    Ok(BundleIngestPlan {
        apply,
        remove,
        api_protected,
        pass_through,
    })

}

fn bundle_row_key(zone: &ZoneId, row: &BundleResource) -> ResourceKey {
    ResourceKey::new(
        zone.as_str(),
        row.resource_type().as_str(),
        row.metadata().name().as_str(),
    )
}


/// One desired spec under the bundle subject: the canonical spec envelope
/// bytes exactly as authored, plus the author metadata envelope (owner
/// reference, labels, annotations; KTD2 metadata fields minus status).
fn bundle_desired(zone: &ZoneId, row: &BundleResource) -> DesiredResource {
    let metadata = serde_json::json!({
        "ownerRef": row.metadata().owner_ref().map(|owner| owner.to_canonical_string()),
        "labels": row.metadata().labels(),
        "annotations": row.metadata().annotations(),
    });
    DesiredResource {
        key: bundle_row_key(zone, row),
        spec: row.spec().to_canonical_bytes(),
        metadata: serde_json::to_vec(&metadata).unwrap_or_default(),
        provenance: d2b_resource_runtime::spec_store::ResourceProvenance::Nix,
    }
}

/// What one bundle ingestion did (U10 report).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BundleIngestReport {
    pub applied: Vec<ResourceKey>,
    pub removed: Vec<ResourceKey>,
    pub api_protected: Vec<ResourceKey>,
    pub pass_through_count: usize,
}

impl ResourcePlaneV3 {
    /// Flow the verified Nix bundle into the manager (U10/R26/F1): every
    /// apply commits before its actor exists (the manager's durability
    /// boundary), provenance is `Nix`, API-provenance rows survive, and the
    /// registry re-registers the durable anchors afterwards.
    pub async fn ingest_nix_bundle(&self, bundle: &ResourceBundle) -> Result<BundleIngestReport, PlaneError> {
        let plan = partition_nix_bundle(&self.zone, bundle, &self.store).await?;
        let subject = nix_bundle_subject(&bundle.integrity.content_hash);
        let mut report = BundleIngestReport {
            pass_through_count: plan.pass_through.len(),
            ..BundleIngestReport::default()
        };
        for desired in &plan.apply {
            self.client.apply(subject.clone(), desired.clone()).await?;
        }
        for key in &plan.remove {
            self.client.remove(subject.clone(), key.clone()).await?;
        }
        report.applied = plan
            .apply
            .iter()
            .map(|desired| desired.key.clone())
            .collect();
        report.removed = plan.remove.clone();
        report.api_protected = plan.api_protected;
        // The production effects resolve per-resource anchors durably.
        self.registry
            .load_from_store(&self.zone_token, &self.store)
            .await?;
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activation_driver::{ActivationDriverEffects, HostHandoffResult};
    use d2b_contracts_broker::host_generation::HostGenerationHandoffIntent;
    use d2b_contracts_resource::v3::ResourceName;
    use d2b_contracts_zone_session::v3::resource_bundle::BundleResourceMetadata;
    use d2b_process_conformance::ProcessIdentityDigest;

    fn timestamp() -> d2b_contracts_resource::v3::Timestamp {
        d2b_contracts_resource::v3::Timestamp::parse("2026-01-01T00:00:00.000Z").expect("timestamp")
    }
    fn test_bundle(resources: Vec<BundleResource>) -> ResourceBundle {
        ResourceBundle::new(
            ZoneId::parse("test").unwrap(),
            resources,
            format!("sha256:{}", "a".repeat(64)),
            BTreeMap::new(),
            BTreeMap::new(),
            timestamp(),
        )
        .expect("bundle")
    }

    fn bundle_row(
        resource_type: &str,
        name: &str,
        spec: serde_json::Value,
    ) -> BundleResource {
        BundleResource::new(
            d2b_contracts_resource::v3::ResourceTypeName::parse(resource_type).unwrap(),
            BundleResourceMetadata::new(
                ResourceName::parse(name).unwrap(),
                ZoneId::parse("test").unwrap(),
                None,
                BTreeMap::new(),
                BTreeMap::new(),
            ),
            serde_json::from_value::<d2b_contracts_resource::v3::resource_schema::CanonicalJsonObject>(spec)
                .expect("canonical spec"),
        )
        .expect("bundle row")
    }

    struct FakeProcessEffects;

    #[async_trait::async_trait]
    impl ProcessDriverEffects for FakeProcessEffects {
        async fn launch(
            &self,
            _identity: &crate::process_driver::ProcessResourceIdentity,
            _spec: &d2b_contracts_resource::v3::process::ProcessSpec,
            _timeout: Duration,
        ) -> Result<ProcessIdentityDigest, String> {
            Ok(ProcessIdentityDigest::from_bytes([0u8; 32]))
        }

        async fn adopt(
            &self,
            _identity: &crate::process_driver::ProcessResourceIdentity,
            _spec: &d2b_contracts_resource::v3::process::ProcessSpec,
        ) -> Result<crate::process_provider_runtime::ProviderAdoption, String> {
            Ok(crate::process_provider_runtime::ProviderAdoption::Absent)
        }

        async fn stop(
            &self,
            _identity: &crate::process_driver::ProcessResourceIdentity,
            _spec: &d2b_contracts_resource::v3::process::ProcessSpec,
            _term_timeout: Duration,
            _kill_timeout: Duration,
        ) -> Result<bool, String> {
            Ok(true)
        }

        async fn stop_stale(
            &self,
            _provider_ref: &ResourceRef,
            _candidate: &d2b_process_conformance::AdoptionCandidate,
        ) -> Result<(), String> {
            Ok(())
        }

        async fn finalize(&self, _identity: &crate::process_driver::ProcessResourceIdentity) -> Result<(), String> {
            Ok(())
        }

        fn has_active(
            &self,
            _zone: &ZoneId,
            _zone_uid: Option<&ResourceUid>,
            _resource_ref: &ResourceRef,
        ) -> bool {
            false
        }
    }

    struct FakeVolumeEffects;

    #[async_trait::async_trait]
    impl VolumeDriverEffects for FakeVolumeEffects {
        async fn ensure_layout(
            &self,
            _volume_uid: &ResourceUid,
            _spec: &VolumeSpec,
            _provider: Option<&serde_json::Value>,
            _owner_ref: Option<&ResourceRef>,
        ) -> Result<bool, String> {
            Ok(true)
        }

        async fn remove_layout(&self, _volume_uid: &ResourceUid, _spec: &VolumeSpec) -> Result<(), String> {
            Ok(())
        }

        fn has_layout(&self, _volume_uid: &ResourceUid) -> bool {
            false
        }
    }

    struct FakeBindingEffects;

    #[async_trait::async_trait]
    impl BindingDriverEffects for FakeBindingEffects {
        async fn socket_ready(&self, _socket: &SocketIdentity) -> bool {
            true
        }

        async fn remove_socket(&self, _socket: &SocketIdentity) -> Result<(), String> {
            Ok(())
        }
    }

    struct FakeEndpointEffects;

    #[async_trait::async_trait]
    impl crate::endpoint_driver::EndpointDriverEffects for FakeEndpointEffects {
        async fn socket_present(&self, _producer_ref: &ResourceRef, _purpose: &str) -> bool {
            true
        }

        async fn ensure_socket(&self, _producer_ref: &ResourceRef, _purpose: &str) -> Result<(), String> {
            Ok(())
        }

        async fn remove_socket(&self, _producer_ref: &ResourceRef, _purpose: &str) -> Result<(), String> {
            Ok(())
        }
    }

    struct FakeActivationEffects;

    #[async_trait::async_trait]
    impl ActivationDriverEffects for FakeActivationEffects {
        async fn apply_host_generation_handoff(
            &self,
            _target: ResourceRef,
            _intent: HostGenerationHandoffIntent,
        ) -> HostHandoffResult {
            HostHandoffResult::Incomplete
        }
    }

    struct FakeCredentialEffects;

    #[async_trait::async_trait]
    impl CredentialDriverEffects for FakeCredentialEffects {
        async fn dependency_facts(
            &self,
            _provider_ref: &ResourceRef,
            _execution_ref: &ResourceRef,
        ) -> Option<crate::credential_driver::CredentialDependencyFacts> {
            None
        }

        async fn lease_facts(
            &self,
            _credential_ref: &ResourceRef,
        ) -> Option<crate::credential_driver::CredentialLeaseFacts> {
            None
        }

        async fn agent_ready(&self, _agent_ref: &ResourceRef) -> bool {
            false
        }

        fn session(
            &self,
            _provider_ref: &ResourceRef,
        ) -> Option<Arc<dyn crate::credential_resource_runtime::CredentialSession>> {
            None
        }
    }

    struct FakeSharedProviderEffects;

    #[async_trait::async_trait]
    impl crate::shared_provider_driver::SharedProviderDriverEffects for FakeSharedProviderEffects {
        async fn reconcile_network(
            &self,
            _request: &crate::shared_provider_driver::SharedProviderEffectRequest<'_>,
        ) -> Result<
            crate::shared_provider_driver::SharedProviderEffectOutcome,
            crate::shared_provider_driver::SharedProviderEffectError,
        > {
            Ok(crate::shared_provider_driver::SharedProviderEffectOutcome::phase(
                crate::shared_provider_driver::SharedProviderEffectPhase::Pending,
            ))
        }

        async fn reconcile_tpm(
            &self,
            _request: &crate::shared_provider_driver::SharedProviderEffectRequest<'_>,
        ) -> Result<
            crate::shared_provider_driver::SharedProviderEffectOutcome,
            crate::shared_provider_driver::SharedProviderEffectError,
        > {
            Ok(crate::shared_provider_driver::SharedProviderEffectOutcome::phase(
                crate::shared_provider_driver::SharedProviderEffectPhase::Pending,
            ))
        }

        async fn reconcile_usbip(
            &self,
            _component: crate::shared_provider_driver::UsbipComponent,
            _request: &crate::shared_provider_driver::SharedProviderEffectRequest<'_>,
        ) -> Result<
            crate::shared_provider_driver::SharedProviderEffectOutcome,
            crate::shared_provider_driver::SharedProviderEffectError,
        > {
            Ok(crate::shared_provider_driver::SharedProviderEffectOutcome::phase(
                crate::shared_provider_driver::SharedProviderEffectPhase::Pending,
            ))
        }

        async fn reconcile_security_key(
            &self,
            _component: crate::shared_provider_driver::SecurityKeyComponent,
            _request: &crate::shared_provider_driver::SharedProviderEffectRequest<'_>,
        ) -> Result<
            crate::shared_provider_driver::SharedProviderEffectOutcome,
            crate::shared_provider_driver::SharedProviderEffectError,
        > {
            Ok(crate::shared_provider_driver::SharedProviderEffectOutcome::phase(
                crate::shared_provider_driver::SharedProviderEffectPhase::Pending,
            ))
        }

        async fn reconcile_gpu(
            &self,
            _request: &crate::shared_provider_driver::SharedProviderEffectRequest<'_>,
        ) -> Result<
            crate::shared_provider_driver::SharedProviderEffectOutcome,
            crate::shared_provider_driver::SharedProviderEffectError,
        > {
            Ok(crate::shared_provider_driver::SharedProviderEffectOutcome::phase(
                crate::shared_provider_driver::SharedProviderEffectPhase::Pending,
            ))
        }

        async fn finalize(
            &self,
            _kind: crate::shared_provider_driver::SharedProviderKind,
            _request: &crate::shared_provider_driver::SharedProviderEffectRequest<'_>,
        ) -> Result<
            crate::shared_provider_driver::SharedProviderFinalize,
            crate::shared_provider_driver::SharedProviderEffectError,
        > {
            Ok(crate::shared_provider_driver::SharedProviderFinalize::Complete)
        }
    }

    fn test_inputs() -> (tempfile::TempDir, ConstructionInputs, Arc<NewPlaneReadinessState>) {
        let dir = tempfile::tempdir().expect("tempdir");
        let spec_store_dir = dir.path().join("daemon-state/zones/test");
        let socket_runtime_dir = dir.path().join("run");
        let readiness = Arc::new(NewPlaneReadinessState::new());
        (
            dir,
            ConstructionInputs {
                zone: ZoneId::parse("test").unwrap(),
                zone_token: BoundedToken::parse("test".to_owned()).unwrap(),
                spec_store_dir: spec_store_dir.clone(),
                socket_runtime_dir,
                authority: ZoneAuthorityInputs {
                    zone_uid: None,
                    policy_revision: Some(1),
                    provider_assignment_generation: None,
                    controller_generation: ControllerGeneration::new(1).unwrap(),
                    guest_execution: None,
                    mode: DaemonMode::Host,
                    vcpu_count: 1,
                },
                committed_provider_identities: BTreeMap::new(),
                registry: Arc::new(PlaneResourceRegistry::new()),
                process_effects: Arc::new(FakeProcessEffects),
                volume_effects: Arc::new(FakeVolumeEffects),
                binding_effects: Arc::new(FakeBindingEffects),
                endpoint_effects: Arc::new(FakeEndpointEffects),
                activation_effects: Arc::new(FakeActivationEffects),
                credential_effects: Arc::new(FakeCredentialEffects),
                shared_provider_effects: Arc::new(FakeSharedProviderEffects),
            },
            readiness,
        )
    }


    /// Partition router: the converted types route to the new plane,
    /// representative unconverted types to the old plane.
    #[test]
    fn partition_router_classifies_converted_and_unconverted_types() {
        for converted in CONVERTED_TYPES {
            assert_eq!(route_resource_type(converted), PlaneRoute::NewPlane);
        }
        for unconverted in ["Guest", "Provider", "Quota", "EphemeralProcess"] {
            assert_eq!(
                route_resource_type(unconverted),
                PlaneRoute::OldPlane,
                "{unconverted} must stay on the old plane"
            );
        }
        assert_eq!(CONVERTED_TYPES.len(), 16);
    }

    /// KTD7: the committed Provider identities the composition resolves are
    /// published into the registry the production Process effects consult
    /// before the manager spawns any resource actor; a Provider the authority
    /// did not retain stays unpublished so its controller rows refuse closed.
    #[tokio::test(flavor = "multi_thread")]
    async fn committed_provider_identities_publish_before_the_manager_starts() {
        let (_dir, mut inputs, _readiness) = test_inputs();
        let provider_uid =
            ResourceUid::parse("123e4567-e89b-42d3-a456-426614174010").expect("provider uid");
        let provider_generation =
            d2b_contracts_resource::v3::ResourceGeneration::new(4).expect("generation");
        inputs.committed_provider_identities = BTreeMap::from([(
            ResourceRef::parse("Provider/network-local").expect("provider ref"),
            (provider_uid.clone(), provider_generation),
        )]);
        let registry = Arc::clone(&inputs.registry);
        let plane = ResourcePlaneV3::open(inputs).await.expect("plane");
        let source = &*registry as &dyn crate::process_driver::CommittedProviderIdentitySource;
        assert_eq!(
            source.committed_provider_identity(
                &ResourceRef::parse("Provider/network-local").expect("provider ref")
            ),
            Some((provider_uid, provider_generation))
        );
        assert_eq!(
            source.committed_provider_identity(
                &ResourceRef::parse("Provider/unretained").expect("provider ref")
            ),
            None
        );
        plane.shutdown().await;
    }

    /// Assembly constructs with fake effects and the readiness gate opens
    /// only when the initial load completes.
    #[tokio::test(flavor = "multi_thread")]
    async fn assembly_constructs_and_readiness_follows_the_checklist() {
        let (_dir, inputs, readiness) = test_inputs();
        let plane = ResourcePlaneV3::prepare(inputs).expect("plane prepare");
        // Before the initial load completes, the gate stays closed.
        let snapshot = plane.readiness();
        assert!(snapshot.spec_store_ready);
        assert!(snapshot.manager_started);
        assert!(snapshot.providers_registered);
        assert!(!snapshot.initial_load_complete);
        assert!(!snapshot.is_ready());

        plane.complete_initial_load().await.expect("initial load");
        assert!(plane.readiness().is_ready());
        let _ = readiness;
        plane.shutdown().await;
    }

    /// The registry is a cache of store-derived rows: a socket target that
    /// belongs to a derived child committed after the plane's durable loads
    /// (the manager mints the VolumeBinding) still resolves, by identity and
    /// by producer ref, through the store on a lookup miss.
    #[tokio::test(flavor = "multi_thread")]
    async fn socket_target_lookup_loads_derived_binding_rows_from_the_store() {
        let (_dir, inputs, _readiness) = test_inputs();
        let zone_token = inputs.zone_token.clone();
        let registry = Arc::clone(&inputs.registry);
        let plane = ResourcePlaneV3::prepare(inputs).expect("plane prepare");

        let volume_ref = ResourceRef::parse("Volume/state").unwrap();
        let execution_ref = ResourceRef::parse("Guest/acceptance-guest").unwrap();
        let binding = serde_json::json!({
            "volumeRef": volume_ref.to_canonical_string(),
            "executionRef": execution_ref.to_canonical_string(),
            "view": "controller",
            "access": "read-only",
            "mountPath": "/state",
        });
        // The stored envelope is the neutral binding plus the serving
        // Provider reference, exactly as the Volume driver mints it.
        let mut envelope = binding.as_object().cloned().expect("object");
        envelope.insert(
            "providerRef".to_owned(),
            serde_json::Value::String("Provider/volume-virtiofs".to_owned()),
        );
        let uid = [0x42; 16];
        // A row the plane never loaded: exactly the state the manager leaves
        // behind when it ensures a derived child after `open`.
        plane
            .store()
            .ensure(StoredDesiredResource {
                key: ResourceKey::new("test", "VolumeBinding", "vol-binding-derived"),
                uid,
                generation: 1,
                owner_uid: Some([0x11; 16]),
                provenance: d2b_resource_runtime::identity::ResourceProvenance::Resource,
                deleting: false,
                spec: serde_json::to_vec(&envelope).expect("envelope"),
                metadata: Vec::new(),
                created_at: 0,
            })
            .await
            .expect("binding row");

        let stored = d2b_provider_volume_virtiofs::StoredBinding::new(
            serde_json::from_slice(&serde_json::to_vec(&binding).expect("binding")).expect("binding spec"),
            resource_uid(&uid).expect("uid"),
            d2b_contracts_resource::v3::ResourceGeneration::new(1).expect("generation"),
            ZoneRevision::new(0),
        );
        let by_identity = registry
            .socket_target_by_identity(&zone_token, &stored.socket_identity(&zone_token))
            .await
            .expect("identity lookup loads the derived row from the store");
        assert_eq!(by_identity.volume_ref, volume_ref);
        assert_eq!(by_identity.execution_ref, execution_ref);
        for producer_ref in [
            stored.worker_process_ref().expect("worker ref"),
            stored.endpoint_ref().expect("endpoint ref"),
        ] {
            let target = registry
                .socket_target_by_ref(&zone_token, &producer_ref)
                .await
                .expect("producer-ref lookup loads the derived row from the store");
            assert_eq!(target.volume_ref, volume_ref);
            assert_eq!(target.execution_ref, execution_ref);
        }
        plane.shutdown().await;
    }

    /// U10: converted rows route through the manager with provenance Nix;
    /// unconverted rows pass through to the old materialization path.
    #[tokio::test(flavor = "multi_thread")]
    async fn bundle_ingest_partitions_converted_and_unconverted_rows() {
        let (_dir, inputs, _readiness) = test_inputs();
        let plane = ResourcePlaneV3::open(inputs).await.expect("plane");
        let bundle = test_bundle(vec![
            bundle_row(
                "Volume",
                "state",
                serde_json::json!({"providerRef": "Provider/volume-local"}),
            ),
            bundle_row("Guest", "work", serde_json::json!({"systemArtifactId": "a"})),
        ]);

        let report = plane.ingest_nix_bundle(&bundle).await.expect("ingest");
        assert_eq!(report.applied, vec![ResourceKey::new("test", "Volume", "state")]);
        assert!(report.removed.is_empty());
        assert!(report.api_protected.is_empty());
        assert_eq!(report.pass_through_count, 1);

        // Converted row persisted with provenance Nix before any actor
        // effect ran (F1).
        let row = plane
            .client()
            .get_row(ResourceKey::new("test", "Volume", "state"))
            .await
            .expect("get_row")
            .expect("converted row");
        assert_eq!(row.provenance, d2b_resource_runtime::identity::ResourceProvenance::Nix);
        assert_eq!(row.generation, 1);

        // Unconverted rows pass through with the bundle's integrity intact.
        let plan = partition_nix_bundle(&ZoneId::parse("test").unwrap(), &bundle, plane.store())
            .await
            .expect("partition");
        let old_bundle = plan.old_bundle(&bundle).expect("old bundle");
        assert_eq!(old_bundle.resources.len(), 1);
        assert_eq!(old_bundle.resources[0].resource_type().as_str(), "Guest");
        old_bundle.verify().expect("rebuilt bundle verifies");

        // Re-ingest is idempotent: no new generation, nothing removed.
        let again = plane.ingest_nix_bundle(&bundle).await.expect("re-ingest");
        assert!(again.applied.contains(&ResourceKey::new("test", "Volume", "state")));
        let row = plane.client().get_row(ResourceKey::new("test", "Volume", "state")).await.unwrap().unwrap();
        assert_eq!(row.generation, 1);
        plane.shutdown().await;
    }

    /// Nix applies never clobber API-provenance rows (R26): the partition
    /// leaves them alone and the durable row keeps its provenance and spec.
    #[tokio::test(flavor = "multi_thread")]
    async fn api_provenance_rows_survive_a_nix_re_apply() {
        let (_dir, inputs, _readiness) = test_inputs();
        let plane = ResourcePlaneV3::open(inputs).await.expect("plane");
        let key = ResourceKey::new("test", "Volume", "state");
        let api_subject = d2b_resource_runtime::manager::MutationSubject {
            principal: "User/alice".to_owned(),
            origin: d2b_resource_runtime::identity::ResourceProvenance::Api,
        };
        plane
            .client()
            .apply(
                api_subject,
                DesiredResource {
                    key: key.clone(),
                    spec: serde_json::json!({"providerRef": "Provider/volume-local", "apiField": true})
                        .to_string()
                        .into_bytes(),
                    metadata: Vec::new(),
                    provenance: d2b_resource_runtime::identity::ResourceProvenance::Api,
                },
            )
            .await
            .expect("api apply");

        let bundle = test_bundle(vec![bundle_row(
            "Volume",
            "state",
            serde_json::json!({"providerRef": "Provider/volume-local", "nixField": true}),
        )]);
        let report = plane.ingest_nix_bundle(&bundle).await.expect("ingest");
        assert_eq!(report.api_protected, vec![key.clone()]);
        assert!(report.applied.is_empty());

        let row = plane.client().get_row(key.clone()).await.unwrap().expect("row");
        assert_eq!(row.provenance, d2b_resource_runtime::identity::ResourceProvenance::Api);
        assert!(String::from_utf8_lossy(&row.spec).contains("apiField"));
        plane.shutdown().await;
    }

    /// Configuration changes mark removed Nix resources deleting (R26).
    #[tokio::test(flavor = "multi_thread")]
    async fn removed_nix_rows_are_marked_deleting() {
        let (_dir, inputs, _readiness) = test_inputs();
        let plane = ResourcePlaneV3::open(inputs).await.expect("plane");
        let first = test_bundle(vec![bundle_row(
            "Volume",
            "state",
            serde_json::json!({"providerRef": "Provider/volume-local"}),
        )]);
        plane.ingest_nix_bundle(&first).await.expect("first ingest");
        assert!(plane.client().get_row(ResourceKey::new("test", "Volume", "state")).await.unwrap().is_some());

        let second = test_bundle(vec![]);
        let report = plane.ingest_nix_bundle(&second).await.expect("second ingest");
        assert_eq!(report.removed, vec![ResourceKey::new("test", "Volume", "state")]);

        // The deletion completes through the actor (fake effects converge).
        let mut gone = false;
        // The deletion completes through the actor (fake effects converge).

        for _ in 0..100 {
            if plane
                .client()
                .get_row(ResourceKey::new("test", "Volume", "state"))
                .await
                .unwrap()
                .is_none()
            {
                gone = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(gone, "removed Nix row must retire after cleanup");
        plane.shutdown().await;
    }
}