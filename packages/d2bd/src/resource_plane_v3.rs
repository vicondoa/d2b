//! Per-zone v3 resource plane assembly (U9/U10; KTD4, KTD5, R26, R27, R31).
//!
//! ## What this module owns
//!
//! - [`ResourcePlaneV3`] is the per-zone assembly (KTD5): it opens the
//!   per-zone SQLite spec store, registers the driver factories over the
//!   production effects (KTD7: ticket inputs from the bundle resolver /
//!   `ZoneAuthorityIdentity`, never from the spec store), spawns the
//!   per-zone manager, and reports the U9 readiness checklist. U14 retired
//!   the redb store and the Phase A type partition with it: the manager is
//!   the one plane every resource type is served by.
//! - [`ResourcePlaneV3::ingest_nix_bundle`] is U10: the Nix bundle flows
//!   into the manager as desired specs with provenance `Nix` under the
//!   bundle subject; Nix applies never clobber API-provenance rows (the
//!   ingest plan consults durable provenance, and removals only touch
//!   Nix-provenance rows).
//!
//! ## Spec store path decision
//!
//! The store is a plain daemon-owned file under
//! `<daemon-state>/zones/<zone>/spec-store.sqlite3`. It is never opened
//! through a broker fd handover: the broker-provisioned
//! `<state-root>/zones/<zone>` directory is owned by the zone-store
//! principal, so a spec store placed there fails to open with
//! `SQLITE_CANTOPEN`. [`d2b_resource_runtime::spec_store::SpecStore::open`]
//! enforces the 0600 file / 0700 directory posture and owns the WAL setup
//! itself, so no broker handover is needed; U14 retired the redb store and
//! its handover.

use std::any::Any;
use std::collections::{BTreeMap, BTreeSet, HashMap};
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
use d2b_provider_activation_nixos::{
    ActivationDriverArgs, ActivationDriverEffects, activation_descriptor,
};
use d2b_provider_endpoint::{
    EndpointDriverArgs, EndpointDriverEffects, GuestControlProducer, endpoint_descriptor,
};
use d2b_provider_guest::{GuestDriverArgs, GuestDriverEffects, guest_descriptor};
use d2b_provider_host::host_descriptor;
use d2b_provider_user::user_descriptor;
use d2b_provider_process::{
    GuestOwnerIdentitySource, ProcessDriverArgs, ProcessDriverEffects, decode_metadata_owner_ref,
    process_family_descriptors,
};
use d2b_provider_telemetry_binding::telemetry_binding_descriptor;
use d2b_provider_telemetry_service::telemetry_service_descriptor;
use d2b_provider_volume::{
    VolumeDriverArgs, VolumeDriverEffects, volume_descriptor,
};
use d2b_provider_volume_binding::{
    BindingDriverArgs, BindingDriverEffects, binding_descriptor,
};
use d2b_provider_volume_local::{VolumeLocalController, VolumeLocalProfile};
use d2b_provider_volume_virtiofs::{MAX_SOCKET_PATH_BYTES, SocketIdentity, StoredBinding};
use d2b_resource_api::manager_backend::nix_bundle_subject;
use d2b_resource_runtime::context::SpecDecoder;
use d2b_resource_runtime::error::ResourceError;
use d2b_resource_runtime::identity::{ResourceKey, ResourceTypeName};
use d2b_resource_runtime::resource::ResourceStatus;
use d2b_resource_runtime::manager::{
    DesiredResource, ResourceManager, ResourceManagerArgs, ResourceManagerClient,
    ResourceManagerMsg, ResourceSelector,
};
use d2b_resource_runtime::spec_store::{SpecSelector, SpecStore, StoredDesiredResource};
use d2b_resource_runtime::GuestTargetControl;
use d2b_resource_runtime::target::{TargetDirectory, TargetRef, TargetResolver};
use d2b_resource_runtime::watch::{DEFAULT_RING_CAPACITY, WatchHub};
use d2bd_runtime::resource_runtime_support::NewPlaneReadinessState;
use d2bd_runtime::target_runtime::DaemonMode;
use rustix::fs::{Mode, OFlags, ResolveFlags, open, openat2};
use sha2::{Digest, Sha256};

use crate::activation_effects::ProductionActivationDriverEffects;
use crate::binding_effects::ProductionBindingDriverEffects;
use d2b_provider_credential::{
    CredentialDriverArgs, CredentialDriverEffects, credential_descriptor,
};
use crate::endpoint_effects::{
    AsyncSocketEffect, ProductionEndpointDriverEffects, device_worker_purpose,
    guest_control_producer, guest_control_purpose,
};
use crate::process_effects::ProductionProcessDriverEffects;
use crate::volume_effects::ProductionVolumeDriverEffects;
use crate::provider_lifecycle::{
    ProviderRuntime, ProviderSet, ProviderStartupError, TrustedContextPublication,
    family_declaration,
};
use d2b_provider_device::{DeviceDriverArgs, device_descriptor};
use d2b_provider_device_security_key::{SecurityKeyDriverArgs, security_key_descriptors};
use d2b_provider_device_usbip::{UsbipDriverArgs, usbip_descriptors};
use d2b_provider_network_local::{NetworkDriverArgs, network_descriptor};
use crate::guest_effects::ProductionGuestDriverEffects;
use crate::shared_provider_effects::ProductionSharedProviderEffects;
use crate::system_core_effects::{ProductionHostDriverEffects, ProductionUserDriverEffects};
use d2b_provider_command::command_descriptor;
use d2b_provider_emergency_policy::emergency_policy_descriptor;
use d2b_provider_operation::operation_descriptor;
use d2b_provider_provider::{
    ProviderDriverArgs, ProviderDriverEffects, provider_descriptor,
};
use d2b_provider_quota::quota_descriptor;
use d2b_provider_resource_export::resource_export_descriptor;
use d2b_provider_resource_import::resource_import_descriptor;
use d2b_provider_role::role_descriptor;
use d2b_provider_role_binding::role_binding_descriptor;
use d2b_provider_seccomp_profile::seccomp_profile_descriptor;
use d2b_provider_zone::zone_descriptor;
use d2b_provider_zone_link::zone_link_descriptor;
use crate::interaction_child_sources::{
    ProductionAudioBindingChildSource, ProductionDisplayChildSource,
};
use crate::resource_runtime::ProductionInteractionDriverEffects;
use d2b_provider_audio_binding::{
    AudioBinding, audio_binding_descriptor,
};
use d2b_provider_audio_service::{AudioService, audio_service_descriptor};
use d2b_provider_shell_pool::{ShellPool, shell_pool_descriptor};
use d2b_provider_shell_session::{ShellSession, shell_session_descriptor};
use d2b_provider_wayland_policy::{
    InteractionDriverArgs, InteractionDriverEffects, WaylandPolicy, wayland_policy_descriptor,
};
use d2b_provider_wayland_session::{WaylandSession, wayland_session_descriptor};

/// The construction arguments every interaction driver of this plane shares.
///
/// Construction is infallible by contract: the zone was validated at plane
/// construction and the effect port is the daemon's production adapter.
fn interaction_driver_args<T: d2b_provider_wayland_policy::InteractionType>(
    inputs: &ConstructionInputs,
    behavior: T,
) -> InteractionDriverArgs<T> {
    InteractionDriverArgs {
        zone: inputs.zone.as_str().to_owned(),
        controller_generation: inputs.authority.controller_generation,
        effects: Arc::clone(&inputs.interaction_effects),
        behavior,
    }
}

/// Frozen purpose of the binding-owned virtiofsd socket.
const VIRTIOFSD_PURPOSE: &str = "virtiofsd";

/// Preserved reconcile backoff for the plane's resource actors (R13).
const PLANE_BACKOFF: Duration = d2b_resource_runtime::DEFAULT_REQUEUE_BACKOFF;

/// Bounded wait budget for endpoint socket realization.
const SOCKET_REALIZE_BUDGET: Duration = Duration::from_secs(5);

// ---------------------------------------------------------------------------
// Committed Provider identities (KTD7)
// ---------------------------------------------------------------------------
//
// The composition unit resolves the bundle's `Provider` rows through the
// Zone's durable authority and hands the identities to
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
    pub(crate) fn register_committed_provider_identity(
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

    /// The committed-`Provider` identity view the production Process effects
    /// consult (KTD7), published by [`PlaneResourceRegistry`].
    pub(crate) fn committed_provider_identity(
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

fn decode_volume_spec(spec_bytes: &[u8]) -> Option<VolumeSpec> {
    let spec = serde_json::from_slice::<d2b_contracts_resource::v3::ResourceSpec>(spec_bytes).ok()?;
    serde_json::from_slice::<VolumeSpec>(&spec.base().to_canonical_bytes()).ok()
}

/// Map the new store's 16-byte deterministic uid onto the contracts crate's
/// UUIDv4-shaped `ResourceUid` (same mapping the converted drivers use).
pub(crate) fn resource_uid(bytes: &[u8; 16]) -> Result<ResourceUid, ()> {
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

/// Re-resolve the provisional KTD7 seed from the plane's own store.
///
/// The manager is the `Provider` row's authority: a durable row already
/// carries the identity and generation its spec history reached, which the
/// pre-open seed (running before this store exists) cannot know. Rows this
/// store does not hold keep their seeded identity - the ingest is about to
/// create them exactly as seeded.
async fn corrected_committed_provider_identities(
    store: &Arc<SpecStore>,
    zone: &ZoneId,
    seeded: &BTreeMap<ResourceRef, (ResourceUid, d2b_contracts_resource::v3::ResourceGeneration)>,
) -> BTreeMap<ResourceRef, (ResourceUid, d2b_contracts_resource::v3::ResourceGeneration)> {
    let mut corrected = seeded.clone();
    for provider_ref in seeded.keys() {
        let key = ResourceKey::new(
            zone.as_str(),
            provider_ref.resource_type().as_str(),
            provider_ref.name().as_str(),
        );
        let row = store.get(key).await;
        if let Ok(row) = row
            && let Ok(uid) = resource_uid(&row.uid)
            && let Ok(generation) =
                d2b_contracts_resource::v3::ResourceGeneration::new(row.generation)
        {
            corrected.insert(provider_ref.clone(), (uid, generation));
        }
    }
    corrected
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
        virtiofs_socket_path(
            &self.socket_runtime_dir,
            &self.zone_token,
            &target.volume_ref,
            &target.execution_ref,
        )
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
        virtiofs_socket_path(
            &self.socket_runtime_dir,
            &self.zone_token,
            &target.volume_ref,
            &target.execution_ref,
        )
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

/// Presence evidence for the guest-runtime control endpoints (`ch-api`,
/// `guest-control`).
///
/// The guest's nested VMM carries both private rendezvous - the Cloud
/// Hypervisor API socket and the authenticated guest-control session - and
/// the guest's committed VMM Process row (`Process/<guest>-vmm`) reports
/// `Ready` exactly while the launch that carries them is live. The old daemon
/// stage published both endpoint rows `Ready` from that same evidence
/// (`reconcile_cloud_hypervisor_endpoints`); since U17 the row's actor owns
/// its status (R11, AE6), so the Endpoint actor reads the guest's VMM row and
/// publishes it. The plane table is resolved lazily per read - the
/// composition fills it only after its per-zone loop finishes.
struct GuestControlEndpointProbe {
    planes: Arc<parking_lot::Mutex<HashMap<String, Arc<ResourcePlaneV3>>>>,
    zone: ZoneId,
}

impl GuestControlEndpointProbe {
    fn new(
        planes: Arc<parking_lot::Mutex<HashMap<String, Arc<ResourcePlaneV3>>>>,
        zone: ZoneId,
    ) -> Self {
        Self { planes, zone }
    }

    /// Whether the producer Guest's committed VMM Process row reports
    /// `Ready` at its current generation.
    ///
    /// The evidence row follows the provider's own declaration for the
    /// purpose: `ch-api` is produced by the VMM Process itself (the producer
    /// row is the evidence row), `guest-control` by the Guest (the evidence
    /// row is the guest's deterministic VMM child).
    async fn present(&self, producer_ref: &ResourceRef, purpose: &str) -> bool {
        let Some(producer) = guest_control_producer(purpose) else {
            return false;
        };
        if producer_ref.resource_type().as_str() != producer.resource_type() {
            return false;
        }
        let vmm_ref = match producer {
            GuestControlProducer::VmmProcess => producer_ref.clone(),
            GuestControlProducer::Guest => {
                let Ok(vmm_ref) =
                    d2b_provider_guest_cloud_hypervisor::deterministic_child_ref(
                        producer_ref,
                        d2b_provider_guest_cloud_hypervisor::ChildRole::VmmProcess,
                    )
                else {
                    return false;
                };
                vmm_ref
            }
        };
        let Some(plane) = self.planes.lock().get(self.zone.as_str()).cloned() else {
            return false;
        };
        let key = ResourceKey::new(
            self.zone.as_str(),
            vmm_ref.resource_type().as_str(),
            vmm_ref.name().as_str(),
        );
        match plane.client().get(key).await {
            Ok(Some(view)) => view.observed_status() == Some(ResourceStatus::Ready),
            Ok(None) => false,
            Err(error) => {
                tracing::warn!(
                    zone = %self.zone.as_str(),
                    producer = %producer_ref.to_canonical_string(),
                    purpose,
                    error = %error,
                    "guest control endpoint probe: VMM row read failed",
                );
                false
            }
        }
    }
}

/// Presence evidence for the Device-owning worker endpoints
/// (`swtpm-tpm-socket`, `swtpm-control-socket`).
///
/// One swtpm launch composes both sockets (`--server` and `--ctrl` of the same
/// argv) and the declaring Device TPM Provider's worker Process row reports
/// `Ready` exactly while that launch is live, so the producer row is the
/// evidence row - read the same way the guest-runtime control family reads its
/// VMM row. The daemon owns nothing here: the worker creates the sockets and a
/// Device delete retires them with the row.
struct DeviceWorkerEndpointProbe {
    planes: Arc<parking_lot::Mutex<HashMap<String, Arc<ResourcePlaneV3>>>>,
    zone: ZoneId,
}

impl DeviceWorkerEndpointProbe {
    fn new(
        planes: Arc<parking_lot::Mutex<HashMap<String, Arc<ResourcePlaneV3>>>>,
        zone: ZoneId,
    ) -> Self {
        Self { planes, zone }
    }

    /// Whether the producer worker row reports `Ready` at its current
    /// generation.
    async fn present(&self, producer_ref: &ResourceRef, purpose: &str) -> bool {
        if !device_worker_purpose(purpose) || producer_ref.resource_type().as_str() != "Process" {
            return false;
        }
        let Some(plane) = self.planes.lock().get(self.zone.as_str()).cloned() else {
            return false;
        };
        let key = ResourceKey::new(
            self.zone.as_str(),
            producer_ref.resource_type().as_str(),
            producer_ref.name().as_str(),
        );
        match plane.client().get(key).await {
            Ok(Some(view)) => view.observed_status() == Some(ResourceStatus::Ready),
            Ok(None) => false,
            Err(error) => {
                tracing::warn!(
                    zone = %self.zone.as_str(),
                    producer = %producer_ref.to_canonical_string(),
                    purpose,
                    error = %error,
                    "device worker endpoint probe: producer row read failed",
                );
                false
            }
        }
    }
}

/// Dispatches each admitted Endpoint purpose onto the realization the plane
/// owns: `virtiofsd` onto the host socket effect, the guest-runtime control
/// purposes onto the guest's VMM evidence, the device-worker purposes onto the
/// producer worker row's evidence. The driver refuses every other purpose at
/// validate, so anything else is a retryable failure (R13).
struct EndpointEnsureEffect {
    socket: Arc<SocketWaitEffect>,
    control: Arc<GuestControlEndpointProbe>,
    device_worker: Arc<DeviceWorkerEndpointProbe>,
}

#[async_trait::async_trait]
impl AsyncSocketEffect for EndpointEnsureEffect {
    async fn run(&self, producer_ref: &ResourceRef, purpose: &str) -> Result<(), String> {
        if guest_control_purpose(purpose) || device_worker_purpose(purpose) {
            let probe = if guest_control_purpose(purpose) {
                EndpointEvidence::Control(&self.control)
            } else {
                EndpointEvidence::DeviceWorker(&self.device_worker)
            };
            let deadline = tokio::time::Instant::now() + SOCKET_REALIZE_BUDGET;
            loop {
                match probe {
                    EndpointEvidence::Control(probe) => {
                        if probe.present(producer_ref, purpose).await {
                            return Ok(());
                        }
                    }
                    EndpointEvidence::DeviceWorker(probe) => {
                        if probe.present(producer_ref, purpose).await {
                            return Ok(());
                        }
                    }
                }
                if tokio::time::Instant::now() >= deadline {
                    return Err(format!(
                        "endpoint {purpose:?} is not realized within its realize budget"
                    ));
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
        self.socket.run(producer_ref, purpose).await
    }
}

/// The row-evidenced probe one admitted non-socket family resolves to.
enum EndpointEvidence<'a> {
    Control(&'a GuestControlEndpointProbe),
    DeviceWorker(&'a DeviceWorkerEndpointProbe),
}

/// Removal for the same dispatch: the guest-runtime control endpoints are
/// owned by the guest's nested VMM (the daemon creates nothing to remove), so
/// their removal converges without effects; `virtiofsd` keeps the preserved
/// endpoint-first socket removal.
struct EndpointRemoveEffect {
    socket: Arc<SocketRemoveEffect>,
}

#[async_trait::async_trait]
impl AsyncSocketEffect for EndpointRemoveEffect {
    async fn run(&self, producer_ref: &ResourceRef, purpose: &str) -> Result<(), String> {
        if guest_control_purpose(purpose) || device_worker_purpose(purpose) {
            return Ok(());
        }
        self.socket.run(producer_ref, purpose).await
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
        virtiofs_socket_path(
            &self.socket_runtime_dir,
            &self.zone_token,
            &target.volume_ref,
            &target.execution_ref,
        )
    }
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

    /// [`Self::source_unresolved`] for an anchored open that failed with a
    /// concrete OS error. The errno is the only evidence that distinguishes a
    /// farm that does not exist yet, a mode/ownership denial, and a mount
    /// boundary, so it is logged rather than dropped.
    fn source_open_failed(
        &self,
        stage: &'static str,
        volume_name: &str,
        path: &Path,
        error: &std::io::Error,
    ) -> d2b_provider_volume_local::VolumeLocalError {
        tracing::warn!(
            zone = %self.zone.as_str(),
            volume = %volume_name,
            stage,
            path = %path.display(),
            error = %error,
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
            self.sync_store_view(guest_ref, intent, generation_token, &anchor.volume_name)?;
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
        let file = open_anchored_directory(&farm_path).map_err(|error| {
            self.source_open_failed("store-view-open", &anchor.volume_name, &farm_path, &error)
        })?;
        let marker_root = open_anchored_directory(&self.marker_root).map_err(|error| {
            self.source_open_failed("marker-root", &anchor.volume_name, &self.marker_root, &error)
        })?;
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
        crate::resource_runtime::ResolvedVolumeRoot::new(file, volume_uid.clone())?
            .with_marker_root(marker_file)
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
    let names: Vec<&std::ffi::OsStr> = path
        .components()
        .filter_map(|component| match component {
            std::path::Component::Normal(name) => Some(name),
            _ => None,
        })
        .collect();
    let Some((leaf, ancestors)) = names.split_last() else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "unanchored directory",
        ));
    };
    let mut current = open(
        Path::new("/"),
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::empty(),
    )
    .map_err(|error| std::io::Error::from_raw_os_error(error.raw_os_error()))?;
    for name in ancestors {
        // A pure anchor: `O_PATH` needs only search permission on the parent
        // and none on the component itself, which is exactly what the
        // store-view chain grants the daemon (`root:d2bd 0750` plus the
        // traversal-only `u:d2bd:--x` ACLs the runner paths add). Opening an
        // intermediate `O_RDONLY` would instead demand read on directories
        // that are traversal-only by design and fail EACCES.
        current = openat2(
            &current,
            *name,
            OFlags::PATH | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::empty(),
            resolve_beneath(),
        )
        .map_err(|error| std::io::Error::from_raw_os_error(error.raw_os_error()))?;
    }
    // The leaf is the handle the Volume driver keeps; it stays readable.
    let leaf = openat2(
        &current,
        *leaf,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::empty(),
        resolve_beneath(),
    )
    .map_err(|error| std::io::Error::from_raw_os_error(error.raw_os_error()))?;
    Ok(leaf)
}

/// Resolution flags shared by every component of an anchored walk: never
/// escape the directory the walk is anchored at, never follow a symlink or
/// magic link, never cross a mount boundary.
fn resolve_beneath() -> ResolveFlags {
    ResolveFlags::BENEATH
        | ResolveFlags::NO_SYMLINKS
        | ResolveFlags::NO_MAGICLINKS
        | ResolveFlags::NO_XDEV
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
    /// U12: the live controller-session evidence the Core `Provider` driver's
    /// observation and drain read. Production wires the zone runtime's
    /// controller-session coordinator (the same seam the G5 reader bridge
    /// uses); the default fails closed, exactly as the old handler did for a
    /// controller row without session evidence.
    pub provider_effects: Arc<dyn ProviderDriverEffects>,
    pub process_effects: Arc<dyn ProcessDriverEffects>,
    pub volume_effects: Arc<dyn VolumeDriverEffects>,
    pub binding_effects: Arc<dyn BindingDriverEffects>,
    pub endpoint_effects: Arc<dyn EndpointDriverEffects>,
    pub activation_effects: Arc<dyn ActivationDriverEffects>,
    pub credential_effects: Arc<dyn CredentialDriverEffects>,
    pub shared_provider_effects: crate::shared_provider_effects::SharedProviderEffects,
    pub guest_effects: Arc<dyn GuestDriverEffects>,
    pub interaction_effects: Arc<dyn InteractionDriverEffects>,
    /// The origination-leg publication binding for this Zone's
    /// trusted-context values: the broker socket, the daemon's caller role,
    /// and the generations the Zone serves. The production constructor
    /// binds the set; a test or context-free deployment leaves it unbound
    /// and nothing is published.
    pub trusted_context_publication: Option<TrustedContextPublication>,
    /// The committed policy rows this plane seeds before its manager spawns.
    ///
    /// The composition sets this for the foundation plane - the durable
    /// authority's home - and leaves it clear for every zone-local plane, so
    /// a system-homed row can never be written outside the seed.
    pub foundation: Option<FoundationInputs>,
}

/// The declarations one foundation plane seeds before its manager spawns.
pub struct FoundationInputs {
    /// The declared policy rows.
    pub declarations: crate::foundation_seed::FoundationDeclarations,
    /// The committed principal allocation the postures resolve through.
    pub allocation: crate::principal_allocation::PrincipalAllocation,
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
                        state.pidfd_table.clone(),
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
            provider_effects: Arc::new(d2b_provider_provider::FailClosedProviderDriverEffects),
            process_effects: Arc::new(
                ProductionProcessDriverEffects::new(Arc::clone(&process_providers))
                    .with_committed_provider_identities(registry_source)
                    .with_guest_owner_identities(Arc::new(PlaneGuestOwnerIdentities {
                        state: Arc::clone(state),
                    })),
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
                // U13/KTD6: the guest-mount gate reads the Zone target
                // directory for the row's assignment (the plane is resolved
                // at call time - it is registered on `state` after this
                // provider directory is built).
                Arc::new({
                    let state = Arc::clone(state);
                    let zone = zone.clone();
                    move |key: &ResourceKey| {
                        let state = Arc::clone(&state);
                        let zone = zone.clone();
                        let key = key.clone();
                        Box::pin(async move {
                            crate::binding_guest_mount_ready(&state, &zone, &key).await
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
                let control = Arc::new(GuestControlEndpointProbe::new(
                    Arc::clone(&state.v3_planes),
                    zone.clone(),
                ));
                let ensure_control = Arc::clone(&control);
                let device_worker = Arc::new(DeviceWorkerEndpointProbe::new(
                    Arc::clone(&state.v3_planes),
                    zone.clone(),
                ));
                let ensure_device_worker = Arc::clone(&device_worker);
                Arc::new(ProductionEndpointDriverEffects::new(
                    Arc::new(move |producer_ref: &ResourceRef, purpose: &str| {
                        let present = Arc::clone(&present);
                        let control = Arc::clone(&control);
                        let device_worker = Arc::clone(&device_worker);
                        Box::pin(async move {
                            if guest_control_purpose(purpose) {
                                control.present(producer_ref, purpose).await
                            } else if device_worker_purpose(purpose) {
                                device_worker.present(producer_ref, purpose).await
                            } else {
                                present.present(producer_ref, purpose).await
                            }
                        })
                    }),
                    Arc::new(EndpointEnsureEffect {
                        socket: Arc::clone(&wait),
                        control: ensure_control,
                        device_worker: ensure_device_worker,
                    }),
                    Arc::new(EndpointRemoveEffect {
                        socket: Arc::new(SocketRemoveEffect {
                            registry: Arc::clone(&registry),
                            socket_runtime_dir: endpoint_socket_runtime_dir,
                            zone_token: endpoint_zone_token,
                        }),
                    }),
                ))
            },
            activation_effects: Arc::new(ProductionActivationDriverEffects::new(Arc::clone(state))),
            credential_effects,
            shared_provider_effects: crate::shared_provider_effects::SharedProviderEffects::production(
                Arc::new(ProductionSharedProviderEffects::new(
                    Arc::clone(state),
                    zone.clone(),
                    controller_generation,
                )),
            ),
            guest_effects: Arc::new(ProductionGuestDriverEffects::new(
                Arc::clone(state),
                zone.clone(),
                controller_generation,
            )),
            interaction_effects: Arc::new(ProductionInteractionDriverEffects::new(
                Arc::clone(state),
                zone.clone(),
            )),
            trusted_context_publication: Some(
                crate::provider_lifecycle::TrustedContextPublication::production(
                    process_providers.mode(),
                    broker_socket,
                    state.daemon_uid,
                    controller_generation.get(),
                ),
            ),
            foundation: None,
        })
    }
}

/// Production `GuestOwnerIdentitySource` (KTD7): the pre-v3 plane owns `Guest`,
/// so its durable rows are the authority for a Guest-owned Process launch's
/// owner uid. The old store resolved every row's `metadata.ownerRef` to the
/// owner row's uid and the old descriptor composer read that linkage into the
/// launch ticket; the converted manager row cannot carry it for an
/// unconverted owner, so the Process effects resolve the same durable value
/// here.
struct PlaneGuestOwnerIdentities {
    state: Arc<crate::ServerState>,
}

#[async_trait::async_trait]
impl GuestOwnerIdentitySource for PlaneGuestOwnerIdentities {
    async fn guest_owner_uid(&self, zone: &ZoneId, guest_ref: &ResourceRef) -> Option<ResourceUid> {
        let runtime = self
            .state
            .resource_plane
            .lock()
            .ok()
            .and_then(|plane| plane.as_ref().and_then(|plane| plane.zone(zone).ok()))?;
        match runtime.guest_owner_uid(guest_ref).await {
            Ok(uid) => Some(uid),
            Err(error) => {
                tracing::warn!(
                    zone = %zone.as_str(),
                    guest = %guest_ref.to_canonical_string(),
                    error = ?error,
                    "guest owner identity read failed; the launch ticket keeps owner_uid unbound",
                );
                None
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Manager-boundary admission (U9/U10 subjects)
// ---------------------------------------------------------------------------

// Manager-boundary admission for the plane (KTD2 execution decision):
// Nix ingestion presents the bundle subject (`nix:<generation>`), the API
// path (U8) presents the api caller subject, owned cascades present the
// resource-owner subject. U14 retired the Phase A type partition, so the
// manager serves every type; the DriverFactory's registered directory is
// the only gate on which types can spawn actors. The manager's default
// [`AllowAll`] admission is therefore the whole policy.

/// Default spec decode hook for rows no per-type decoder covers (a row whose
/// type has no driver never spawns an actor, so this only ever sees
/// registered types in error paths).
struct PassthroughDecoder;

impl SpecDecoder for PassthroughDecoder {
    fn decode(
        &self,
        envelope: &[u8],
    ) -> Result<Box<dyn Any + Send>, Box<dyn std::error::Error + Send + Sync>> {
        Ok(Box::new(envelope.to_vec()))
    }
}

/// Compare the registry's registered types against the generated
/// converted-type catalog (R4) and fail startup when either side names a type
/// the other does not.
///
/// The catalog is the authority on which types the manager plane serves, so
/// the comparison is exact: a cataloged type with no registered driver is a
/// driver that never arrived (the presence obligation for a mask without
/// RUNTIME), and a registered type the catalog does not list is a driver
/// serving a type outside the plane's partition. Both sides are named, sorted.
fn check_registry_catalog(
    registered: Vec<ResourceTypeName>,
    catalog: &[&str],
) -> Result<(), PlaneError> {
    let registered = registered
        .into_iter()
        .map(|type_name| type_name.as_str().to_owned())
        .collect::<BTreeSet<_>>();
    let listed = catalog.iter().copied().collect::<BTreeSet<_>>();
    let missing = listed
        .iter()
        .filter(|entry| !registered.contains(**entry))
        .map(|entry| (*entry).to_owned())
        .collect::<Vec<_>>();
    let unexpected = registered
        .iter()
        .filter(|entry| !listed.contains(&entry.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    if missing.is_empty() && unexpected.is_empty() {
        return Ok(());
    }
    Err(PlaneError::RegistryCatalogMismatch { missing, unexpected })
}

// ---------------------------------------------------------------------------
// ResourcePlaneV3: the per-zone new plane (U9)
// ---------------------------------------------------------------------------

/// Assembly failures (U9).
#[derive(Debug, thiserror::Error)]
pub enum PlaneError {
    #[error("spec store open failed: {0}")]
    SpecStore(#[from] d2b_resource_runtime::spec_store::SpecStoreError),
    #[error("foundation seed failed: {0}")]
    FoundationSeed(String),
    #[error("provider registration failed: {0}")]
    ProviderRegistration(
        #[from] d2b_resource_runtime::provider::ProviderDirectoryError,
    ),
    /// A provider did not start through the base. The failure names the
    /// provider and the declared row it refused.
    #[error("provider startup refused: {0}")]
    ProviderStartup(#[from] crate::provider_lifecycle::ProviderStartupError),
    /// The registry and the generated converted-type catalog disagree (R4).
    /// Both sides are named: the catalog types with no registered driver, and
    /// the registered types the catalog does not list.
    #[error(
        "driver registry does not match the converted-type catalog: \
         in-catalog-but-not-registered {missing:?}, registered-but-not-in-catalog {unexpected:?}"
    )]
    RegistryCatalogMismatch {
        /// Catalog types no driver is registered for.
        missing: Vec<String>,
        /// Registered types the catalog does not list.
        unexpected: Vec<String>,
    },
    #[error("manager spawn failed: {0}")]
    ManagerSpawn(String),
    #[error("manager rpc failed: {0}")]
    ManagerRpc(#[from] ResourceError),
    #[error("zone authority inputs invalid: {0}")]
    Authority(String),
    #[error("target layer refused: {0}")]
    Target(String),
    #[error("bundle invalid: {0}")]
    Bundle(String),
}

/// The canonical core Host target every non-guest resource realizes on
/// (`CORE_CONTROLLER_HOST_REF` in the old plane).
const CORE_HOST_TARGET_NAME: &str = "host-system";

/// The canonical `spec.executionRef` resolver (U13).
///
/// A stored row's `spec` is the ResourceSpec object, so this reads exactly
/// the `executionRef` base field the resource contracts' `PlacementAnchor::
/// ExecutionRef` resolves. A row whose type has no execution anchor, or a
/// legacy row that carries none, returns `None` and realizes on the Zone's
/// Host target.
struct DeclaredExecutionRef;

impl TargetResolver for DeclaredExecutionRef {
    fn execution_ref(&self, _key: &ResourceKey, spec: &[u8]) -> Option<String> {
        let value: serde_json::Value = serde_json::from_slice(spec).ok()?;
        value.get("executionRef")?.as_str().map(str::to_owned)
    }
}

/// The per-zone v3 resource plane: spec store, provider directory, watch
/// hub, manager actor, and the readiness checklist (U9, KTD5, R27).
pub struct ResourcePlaneV3 {
    zone: ZoneId,
    zone_token: BoundedToken,
    store: Arc<SpecStore>,
    hub: Arc<WatchHub>,
    /// The per-Zone target directory (U13): every resource's assignment and
    /// every guest session generation lives here.
    targets: Arc<TargetDirectory>,
    registry: Arc<PlaneResourceRegistry>,
    client: ResourceManagerClient,
    /// The providers this zone started, in the order they started. The plane
    /// keeps them so it can report the order it ran and drain them in the
    /// mirror of it.
    providers: Arc<ProviderRuntime>,
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

    /// The providers one zone starts, in the committed startup order.
    ///
    /// Each family states its declaration and the drivers it serves; the base
    /// realizes the declared plane facts and then runs the family's own
    /// attach, which registers the drivers it declared. The order of the
    /// groups below is the order the registry has been assembled in since the
    /// family moves landed.
    fn provider_set(inputs: &ConstructionInputs) -> ProviderSet {
        // The Process family starts through its driver declarations: one
        // descriptor per member type, both over the family's shared decoder
        // and factory. The family's verbs, execution domains, exportability,
        // and reads travel on the descriptor.
        let mut set = ProviderSet::new(inputs.zone.clone(), inputs.spec_store_dir.clone()).with(
            family_declaration("process"),
            Vec::from(process_family_descriptors(ProcessDriverArgs {
                zone: inputs.zone.clone(),
                effects: Arc::clone(&inputs.process_effects),
                zone_uid: inputs.authority.zone_uid.clone(),
                policy_revision: inputs.authority.policy_revision,
                provider_assignment_generation: inputs.authority.provider_assignment_generation,
                controller_generation: inputs.authority.controller_generation,
                guest_execution: inputs.authority.guest_execution.clone(),
                mode: crate::process_provider_runtime::execution_mode(inputs.authority.mode),
            })),
        );
        // The Volume family states its own declaration; the Binding family
        // states its own. The registry serves each type's decoder and factory
        // from its driver declaration, and the declaration carries the
        // family's verbs, execution domains, exportability, reads, and the
        // children it may create.
        set = set.with(
            d2b_provider_volume::volume_provider_declaration(),
            vec![volume_descriptor(VolumeDriverArgs {
                zone: inputs.zone.as_str().to_owned(),
                effects: Arc::clone(&inputs.volume_effects),
            })],
        );
        set = set.with(
            family_declaration("volume-binding"),
            vec![binding_descriptor(BindingDriverArgs {
                zone: inputs.zone.as_str().to_owned(),
                effects: Arc::clone(&inputs.binding_effects),
                vcpu_count: inputs.authority.vcpu_count,
            })],
        );
        // The Endpoint type starts through its driver declaration: the
        // registry serves the type's decoder and factory from it, and the
        // declaration carries the family's verbs, execution domains,
        // exportability, and reads.
        set = set.with(
            family_declaration("endpoint"),
            vec![endpoint_descriptor(EndpointDriverArgs {
                zone: inputs.zone.as_str().to_owned(),
                effects: Arc::clone(&inputs.endpoint_effects),
            })],
        );
        // The Credential type starts through its driver declaration: the
        // registry serves the type's decoder and factory from it, and the
        // declaration carries the family's verbs, execution domains,
        // exportability, reads, and the one declared agent Process child.
        set = set.with(
            family_declaration("credential"),
            vec![credential_descriptor(CredentialDriverArgs {
                zone: inputs.zone.as_str().to_owned(),
                controller_generation: inputs.authority.controller_generation,
                effects: Arc::clone(&inputs.credential_effects),
            })],
        );
        // The trusted-context publication rides the set: when production
        // bound one, the rendezvous publishes this Zone's attestation
        // values over the origination leg the moment the set is published.
        set = set.with_trusted_context_publication(inputs.trusted_context_publication.clone());
        // The NixosGeneration type starts through its driver declaration: the
        // registry serves the type's decoder and factory from it, and the
        // declaration carries the family's verbs, execution domains,
        // exportability, reads, and its one declared child creation.
        set = set.with(
            family_declaration("activation-nixos"),
            vec![activation_descriptor(ActivationDriverArgs {
                zone: inputs.zone.as_str().to_owned(),
                effects: Arc::clone(&inputs.activation_effects),
                verifier: Arc::new(d2b_provider_activation_nixos::FailClosedActivationVerifier),
            })],
        );
        // The telemetry pair starts through its driver declarations: one per
        // type, each carrying that type's decoder, factory, verbs, execution
        // domains, exportability, reads, and (for the Binding) the
        // provider-declared child creations.
        set = set.with(
            family_declaration("telemetry-service"),
            vec![telemetry_service_descriptor()],
        );
        set = set.with(
            family_declaration("telemetry-binding"),
            vec![telemetry_binding_descriptor()],
        );
        // The Network family starts through its own declaration, the two USB
        // types through the USB family's, the two security-key types through
        // the security-key family's, and the Device type (four hardware
        // Providers) through the Device family's. Each declaration carries its
        // decoder, so the registry serves it for the type.
        set = set.with(
            family_declaration("network-local"),
            vec![network_descriptor(NetworkDriverArgs {
                zone: inputs.zone.as_str().to_owned(),
                controller_generation: inputs.authority.controller_generation,
                effects: Arc::clone(&inputs.shared_provider_effects.network),
            })],
        );
        set = set.with(
            family_declaration("device-usbip"),
            Vec::from(usbip_descriptors(UsbipDriverArgs {
                zone: inputs.zone.as_str().to_owned(),
                controller_generation: inputs.authority.controller_generation,
                effects: Arc::clone(&inputs.shared_provider_effects.usbip),
            })),
        );
        set = set.with(
            family_declaration("device-security-key"),
            Vec::from(security_key_descriptors(SecurityKeyDriverArgs {
                zone: inputs.zone.as_str().to_owned(),
                controller_generation: inputs.authority.controller_generation,
                effects: Arc::clone(&inputs.shared_provider_effects.security_key),
            })),
        );
        set = set.with(
            family_declaration("device"),
            vec![device_descriptor(DeviceDriverArgs {
                zone: inputs.zone.as_str().to_owned(),
                controller_generation: inputs.authority.controller_generation,
                effects: Arc::clone(&inputs.shared_provider_effects.device),
            })],
        );
        // The Guest type starts through its driver declaration: the registry
        // serves the type's decoder and factory from it, and the declaration
        // carries the family's verbs, execution domains, exportability, reads,
        // and the children its runtime Providers create.
        set = set.with(
            family_declaration("guest"),
            vec![guest_descriptor(GuestDriverArgs {
                zone: inputs.zone.as_str().to_owned(),
                controller_generation: inputs.authority.controller_generation,
                effects: Arc::clone(&inputs.guest_effects),
            })],
        );
        // The Host and User bootstrap types start through their driver
        // declarations: the registry serves each type's decoder and factory
        // from its declaration, and the declarations carry the types' verbs,
        // execution domains, exportability, and reads.
        set = set.with(
            family_declaration("host"),
            vec![host_descriptor(Arc::new(ProductionHostDriverEffects))],
        );
        set = set.with(
            family_declaration("user"),
            vec![user_descriptor(Arc::new(ProductionUserDriverEffects))],
        );
        // The controller family starts through its per-type declarations:
        // each crate serves exactly one type, and the registry resolves that
        // type's decoder, factory, verbs, execution domains, exportability,
        // and reads from the declaration.
        set = set.with(family_declaration("zone"), vec![zone_descriptor()]);
        set = set.with(
            family_declaration("zone-link"),
            vec![zone_link_descriptor()],
        );
        set = set.with(
            family_declaration("provider"),
            vec![provider_descriptor(ProviderDriverArgs {
                effects: Arc::clone(&inputs.provider_effects),
            })],
        );
        set = set.with(family_declaration("role"), vec![role_descriptor()]);
        set = set.with(
            family_declaration("role-binding"),
            vec![role_binding_descriptor()],
        );
        set = set.with(family_declaration("quota"), vec![quota_descriptor()]);
        set = set.with(
            family_declaration("emergency-policy"),
            vec![emergency_policy_descriptor()],
        );
        set = set.with(
            family_declaration("resource-export"),
            vec![resource_export_descriptor()],
        );
        set = set.with(
            family_declaration("resource-import"),
            vec![resource_import_descriptor()],
        );
        // The policy types are declared with their drivers; their rows commit
        // with the committed policy rows, and the presence obligation is what
        // keeps a plane from opening without their drivers.
        set = set.with(family_declaration("command"), vec![command_descriptor()]);
        set = set.with(
            family_declaration("operation"),
            vec![operation_descriptor()],
        );
        set = set.with(
            family_declaration("seccomp-profile"),
            vec![seccomp_profile_descriptor()],
        );
        // The six interaction types start through their driver declarations:
        // the registry serves each type's decoder and factory from its own
        // crate's descriptor, and no daemon table names them.
        set = set.with(
            family_declaration("wayland-policy"),
            vec![wayland_policy_descriptor(interaction_driver_args(
                inputs,
                WaylandPolicy,
            ))],
        );
        set = set.with(
            family_declaration("wayland-session"),
            vec![wayland_session_descriptor(interaction_driver_args(
                inputs,
                WaylandSession::new(Arc::new(ProductionDisplayChildSource)),
            ))],
        );
        set = set.with(
            family_declaration("audio-service"),
            vec![audio_service_descriptor(interaction_driver_args(
                inputs,
                AudioService,
            ))],
        );
        set = set.with(
            family_declaration("audio-binding"),
            vec![audio_binding_descriptor(interaction_driver_args(
                inputs,
                AudioBinding::new(Arc::new(ProductionAudioBindingChildSource)),
            ))],
        );
        set = set.with(
            family_declaration("shell-pool"),
            vec![shell_pool_descriptor(interaction_driver_args(
                inputs,
                ShellPool,
            ))],
        );
        set.with(
            family_declaration("shell-session"),
            vec![shell_session_descriptor(interaction_driver_args(
                inputs,
                ShellSession,
            ))],
        )
    }

    /// Start the zone's providers through the toolkit base.
    ///
    /// The base's attach sequence is async work, so the constructor awaits it
    /// like any other caller instead of driving it from a blocking section: a
    /// blocking section parks the worker that is running the plane's own
    /// start, and on a single-threaded runtime there is no other worker to
    /// park. A refusal names the provider and the declared row it refused.
    async fn start_providers(inputs: &ConstructionInputs) -> Result<ProviderRuntime, PlaneError> {
        let set = Self::provider_set(inputs);
        set.start().await.map_err(PlaneError::ProviderStartup)
    }

    /// Open the store, register the converted-type factories, and
    /// spawn the manager. Initial-load completion is a separate step so the
    /// readiness checklist is observable stage by stage; [`Self::open`]
    /// composes both.
    ///
    /// Every stage here is async work the constructor awaits; the only
    /// synchronous work left is the store's own half (a directory create and
    /// SQLite's open + migration, neither of which has an async form), which
    /// runs on the blocking pool so it is bounded by that pool rather than by
    /// the worker this call would otherwise park.
    pub async fn prepare(inputs: ConstructionInputs) -> Result<Self, PlaneError> {
        let readiness = Arc::new(NewPlaneReadinessState::new());
        // Stage 1: durable spec store.
        let store_path = Self::spec_store_path(&inputs.spec_store_dir);
        let store = Arc::new(
            tokio::task::spawn_blocking(move || {
                if let Some(parent) = store_path.parent() {
                    std::fs::create_dir_all(parent).map_err(|error| {
                        PlaneError::Authority(format!("spec store dir create failed: {error}"))
                    })?;
                }
                SpecStore::open(store_path.clone()).map_err(PlaneError::from)
            })
            .await
            .map_err(|error| {
                PlaneError::Authority(format!("spec store open join failed: {error}"))
            })??,
        );
        // The registry caches store-derived rows for the production effects;
        // the store is the authority its socket-target lookups load from on
        // a miss (the manager mints derived children after `open`).
        inputs.registry.attach_store(Arc::clone(&store));
        // KTD7: publish the committed Provider identities before the manager
        // spawns any resource actor (restart recovery spawns one per durable
        // row), so a controller row's first reconcile never observes its
        // owning Provider unbound. The composition's seed binds the identity
        // the manager allocates for the row the ingest is about to create; a
        // row this store already holds (a restart, possibly after a spec
        // change bumped its generation) answers with its own identity.
        let committed_provider_identities = corrected_committed_provider_identities(
            &store,
            &inputs.zone,
            &inputs.committed_provider_identities,
        )
        .await;
        for (provider_ref, (uid, generation)) in &committed_provider_identities {
            inputs
                .registry
                .register_committed_provider_identity(provider_ref, uid.clone(), *generation);
        }
        readiness.set_spec_store_ready(true);
        // Stage 2: start the zone's providers through the toolkit base. Each
        // provider states its declaration and drivers; the base realizes the
        // declared plane facts and runs the provider's own attach, which
        // registers the drivers it declared. The registry closes for late
        // required registration at plane open (R4), and the registered set is
        // cross-checked against the generated converted-type catalog before
        // the manager spawns.
        let mut provider_runtime = Self::start_providers(&inputs).await?;
        let mut providers = provider_runtime.take_directory();
        tracing::debug!(
            zone = %inputs.zone.as_str(),
            providers = provider_runtime.startup_order().len(),
            claimed_roots = provider_runtime.claimed_roots().len(),
            deployed_adapters = provider_runtime.deployed_adapters().len(),
            published_services = provider_runtime.published_services().len(),
            "the zone's providers started through the base"
        );
        // The foundation plane commits its declared policy rows - the system
        // zone itself, the postures, roles, commands, self-bindings, the
        // materialized spawn operations, and the operator bindings - before
        // its manager spawns, so every seeded row's actor starts from a
        // committed row (F1). The manager's pre_start loads them.
        if let Some(foundation) = &inputs.foundation {
            let seed = crate::foundation_seed::FoundationSeed::new(
                foundation.declarations.clone(),
                foundation.allocation.clone(),
            );
            let report = seed
                .run(&store, &providers)
                .await
                .map_err(|error| PlaneError::FoundationSeed(error.to_string()))?;
            tracing::info!(
                zone = %inputs.zone.as_str(),
                committed = report.committed.len(),
                materialized = report.materialized.len(),
                unchanged = report.unchanged,
                "foundation seed committed the policy rows"
            );
        }
        providers.mark_plane_open();
        check_registry_catalog(
            providers.registered_types(),
            &d2b_contracts::identity::V3_CONVERTED_RESOURCE_TYPES,
        )?;
        readiness.set_providers_registered(true);
        // Stage 3: per-zone manager spawn (KTD5), with the target layer
        // wired (U13): the directory is owned here, the composition registers
        // guest sessions on it, and the manager resolves each row's declared
        // execution reference through it.
        let hub = Arc::new(WatchHub::new(&d2b_resource_runtime::revision::SystemClock, DEFAULT_RING_CAPACITY));
        let targets = Arc::new(TargetDirectory::new());
        let host_target = TargetRef::host(CORE_HOST_TARGET_NAME)
            .map_err(|error| PlaneError::Target(error.to_string()))?;
        // Every registered driver's declaration carries its type's decoder,
        // so the registry is the authority: the plane wires no decoder table
        // of its own.
        let decoders = providers.decoders();
        let args = ResourceManagerArgs {
            zone: inputs.zone.as_str().to_owned(),
            store: Arc::clone(&store),
            providers,
            hub: Arc::clone(&hub),
            admission: Arc::new(crate::foundation_seed::SystemZoneWriteFence::new(
                inputs.foundation.is_some(),
            )),
            decoders,
            default_decoder: Arc::new(PassthroughDecoder),
            targets: Arc::clone(&targets),
            host_target,
            target_resolver: Arc::new(DeclaredExecutionRef),
            backoff: PLANE_BACKOFF,
        };
        let (actor, _join) = ractor::Actor::spawn(None, ResourceManager::new(), args)
            .await
            .map_err(|error| PlaneError::ManagerSpawn(error.to_string()))?;
        readiness.set_manager_started(true);
        readiness.set_spec_store_ready(true);
        Ok(Self {
            zone: inputs.zone.clone(),
            zone_token: inputs.zone_token,
            store,
            hub,
            targets,
            registry: inputs.registry,
            client: ResourceManagerClient::new(actor),
            providers: Arc::new(provider_runtime),
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
        let plane = Self::prepare(inputs).await?;
        plane.complete_initial_load().await?;
        Ok(plane)
    }

    /// The providers this zone started, in the order they started.
    pub(crate) fn providers(&self) -> &ProviderRuntime {
        &self.providers
    }

    /// The providers this zone started, behind a shared handle.
    ///
    /// The forwarding rendezvous holds this handle so a forwarded call
    /// resolves against the providers that are actually started, not against
    /// a copy taken when the plane opened.
    pub(crate) fn provider_runtime(&self) -> Arc<ProviderRuntime> {
        Arc::clone(&self.providers)
    }

    /// Drain the zone's providers in the reverse of the order they started.
    ///
    /// The daemon runs this once on shutdown, after the interaction
    /// providers have finalized, so a provider gives back the plane facts it
    /// claimed before the process that owns them goes away.
    pub(crate) async fn drain_providers(&self) -> Result<(), ProviderStartupError> {
        self.providers.drain().await
    }

    /// The plane's public surface: the U9 readiness checklist and the
    /// spec-store handle. Both are read by this module's tests; the daemon
    /// composition reads `client`/`hub`/`registry`/`targets`/
    /// `ready_zone_count` today, and the readiness reporting the checklist was
    /// sized for is the composition's outstanding wiring.
    #[cfg(test)]
    pub fn readiness(&self) -> d2bd_runtime::resource_runtime_support::NewPlaneReadiness {
        self.readiness.snapshot()
    }

    /// The per-Zone target directory (U13).
    pub fn targets(&self) -> &Arc<TargetDirectory> {
        &self.targets
    }

    /// Register one authenticated guest session generation and tell the
    /// manager to notify the affected actors so they re-run target-local
    /// discovery, adoption and reconcile (F5). Assignments are never
    /// inherited from an older generation (R28).
    pub fn bind_guest_target(
        &self,
        guest: &TargetRef,
        session_generation: u64,
        control: Arc<dyn GuestTargetControl>,
    ) -> Result<(), PlaneError> {
        let outcome = self
            .targets
            .connect_guest(guest, session_generation, control)
            .map_err(|error| PlaneError::Target(error.to_string()))?;
        self.client
            .actor()
            .send_message(ResourceManagerMsg::TargetReconnected {
                guest: guest.clone(),
                session_generation: outcome.session_generation(),
                pending_adoption: outcome.pending_adoption().to_vec(),
            })
            .map_err(|error| PlaneError::Target(error.to_string()))?;
        Ok(())
    }

    /// Mark one guest session generation lost and tell the manager to notify
    /// the affected actors that their target-dependent observed state is
    /// unavailable (R21). Nothing is deleted, moved, or forgotten.
    pub fn unbind_guest_target(
        &self,
        guest: &TargetRef,
        session_generation: u64,
    ) -> Result<(), PlaneError> {
        let outcome = self
            .targets
            .disconnect_guest(guest, session_generation)
            .map_err(|error| PlaneError::Target(error.to_string()))?;
        self.client
            .actor()
            .send_message(ResourceManagerMsg::TargetUnavailable {
                guest: guest.clone(),
                session_generation: outcome.session_generation(),
                affected: outcome.affected().to_vec(),
            })
            .map_err(|error| PlaneError::Target(error.to_string()))?;
        Ok(())
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

    /// See [`Self::readiness`]: read by this module's tests.
    #[cfg(test)]
    pub fn store(&self) -> &Arc<SpecStore> {
        &self.store
    }

    /// Re-register the durable rows the production effects resolve
    /// per-resource anchors from (U17: a provider controller session commits
    /// converted Volume/VolumeBinding children through the manager after the
    /// plane's durable loads, and a Volume root whose anchor is not registered
    /// stays unresolved until a reload). The manager stays the only writer:
    /// this reads its store, it never mutates it.
    pub async fn reload_registry(&self) -> Result<(), PlaneError> {
        self.registry
            .load_from_store(&self.zone_token, &self.store)
            .await
    }

    pub fn registry(&self) -> &Arc<PlaneResourceRegistry> {
        &self.registry
    }

    /// Stop the manager actor. Read by this module's tests; the daemon
    /// composition stops the manager through the runtime it owns.
    #[cfg(test)]
    pub async fn shutdown(&self) {
        self.client.actor().get_cell().stop(None);
    }
}

// ---------------------------------------------------------------------------
// U10: Nix ingestion into the manager (R26, F1)
// ---------------------------------------------------------------------------

/// The Nix bundle ingest plan (U10).
pub struct BundleIngestPlan {
    /// Bundle rows to route through `ResourceManager::Apply` with
    /// provenance `Nix` under the bundle subject.
    pub apply: Vec<DesiredResource>,
    /// Durable Nix-provenance rows the bundle no longer declares:
    /// configuration changes mark them deleting (R26).
    pub remove: Vec<ResourceKey>,
    /// Durable API-provenance rows the bundle names: never touched by a
    /// Nix apply (provenance respected; the API owns them).
    pub api_protected: Vec<ResourceKey>,
}

/// Plan one verified Zone bundle for the manager (U10/R26): every bundle row
/// is an apply or a provenance-protected skip, and durable Nix-provenance
/// rows the bundle no longer declares are removals. U14 retired the Phase A
/// type partition, so no row is passed through to a second plane.
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
    for row in &bundle.resources {
        let key = ResourceKey::new(
            zone.as_str(),
            row.resource_type().as_str(),
            row.metadata().name().as_str(),
        );
        match durable_by_key.get(&key) {
            // API-created rows are never clobbered by a Nix apply
            // (R26: provenance is respected).
            Some(existing)
                if existing.provenance
                    == d2b_resource_runtime::spec_store::ResourceProvenance::Api =>
            {
                api_protected.push(key);
            }
            Some(existing) if existing.deleting => {
                // Already retiring; a re-declare re-adds on the next
                // bundle once the deletion completed.
            }
            _ => apply.push(bundle_desired(zone, row)),
        }
    }
    // Removed Nix rows: mark deleting (R26).
    let declared: HashMap<ResourceKey, ()> = bundle
        .resources
        .iter()
        .map(|row| (bundle_row_key(zone, row), ()))
        .collect();
    let mut remove = Vec::new();
    for (key, existing) in &durable_by_key {
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
}

impl ResourcePlaneV3 {
    /// Flow the verified Nix bundle into the manager (U10/R26/F1): every
    /// apply commits before its actor exists (the manager's durability
    /// boundary), provenance is `Nix`, API-provenance rows survive, and the
    /// registry re-registers the durable anchors afterwards.
    pub async fn ingest_nix_bundle(&self, bundle: &ResourceBundle) -> Result<BundleIngestReport, PlaneError> {
        let plan = partition_nix_bundle(&self.zone, bundle, &self.store).await?;
        let subject = nix_bundle_subject(&bundle.integrity.content_hash);
        let mut report = BundleIngestReport::default();
        // Owners before owned. A bundle row that declares `metadata.ownerRef`
        // is ensured as that owner's child, so the manager links ownership by
        // uid the way R8 defines it: the Core `Provider` driver reads its
        // owned controller `Process` rows through that link (an unlinked row
        // leaves every Provider Pending forever - vmCheck fixtures,
        // 2026-09-11) and the owner cascade follows it. Canonical
        // `(type, name)` order puts `Process` before `Provider`, so rows
        // whose owner is not committed yet are deferred one sweep; an owner
        // that never appears keeps the pre-v3 top-level shape, where the
        // authored reference still renders into the row's metadata.
        let mut known: std::collections::HashSet<ResourceKey> = self
            .store
            .list(SpecSelector {
                zone: Some(self.zone.as_str().to_owned()),
                type_name: None,
                owner_uid: None,
            })
            .await?
            .into_iter()
            .map(|row| row.key)
            .collect();
        let mut pending = plan.apply;
        let mut applied = Vec::new();
        while !pending.is_empty() {
            let attempted = pending.len();
            let mut deferred = Vec::new();
            for desired in pending.drain(..) {
                let owner = decode_metadata_owner_ref(&desired.metadata).map(|owner| {
                    ResourceKey::new(
                        self.zone.as_str(),
                        owner.resource_type().as_str(),
                        owner.name().as_str(),
                    )
                });
                if let Some(owner) = owner.as_ref()
                    && !known.contains(owner)
                {
                    deferred.push(desired);
                    continue;
                }
                let key = desired.key.clone();
                match owner {
                    Some(owner) => {
                        self.client
                            .ensure(subject.clone(), Some(owner), desired)
                            .await?;
                    }
                    None => {
                        self.client.apply(subject.clone(), desired).await?;
                    }
                }
                known.insert(key.clone());
                applied.push(key);
            }
            if deferred.is_empty() {
                break;
            }
            // A sweep that committed nothing can only be rows whose owner
            // never appears in this plane: commit them top-level.
            if deferred.len() == attempted {
                for desired in deferred {
                    applied.push(desired.key.clone());
                    self.client.apply(subject.clone(), desired).await?;
                }
                break;
            }
            pending = deferred;
        }
        for key in &plan.remove {
            self.client.remove(subject.clone(), key.clone()).await?;
        }
        report.applied = applied;
        report.removed = plan.remove;
        report.api_protected = plan.api_protected;
        // The production effects resolve per-resource anchors durably.
        self.reload_registry().await?;
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_provider_activation_nixos::HostHandoffResult;
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

    struct FakeInteractionEffects;

    #[async_trait::async_trait]
    impl InteractionDriverEffects for FakeInteractionEffects {
        async fn reconcile(
            &self,
            _kind: d2b_provider_wayland_policy::InteractionKind,
            _request: &d2b_provider_wayland_policy::InteractionEffectRequest<'_>,
        ) -> Result<
            d2b_provider_wayland_policy::InteractionEffectOutcome,
            d2b_provider_wayland_policy::InteractionEffectError,
        > {
            Ok(d2b_provider_wayland_policy::InteractionEffectOutcome::phase(
                d2b_provider_wayland_policy::InteractionEffectPhase::Pending,
            ))
        }

        async fn finalize(
            &self,
            _kind: d2b_provider_wayland_policy::InteractionKind,
            _request: &d2b_provider_wayland_policy::InteractionEffectRequest<'_>,
        ) -> Result<
            d2b_provider_wayland_policy::InteractionFinalize,
            d2b_provider_wayland_policy::InteractionEffectError,
        > {
            Ok(d2b_provider_wayland_policy::InteractionFinalize::Complete)
        }
    }

    /// Guest effects that stay Pending: the plane tests only need the Guest
    /// driver registered, never a Guest reaching Ready.
    struct FakeGuestEffects;

    #[async_trait::async_trait]
    impl d2b_provider_guest::GuestDriverEffects for FakeGuestEffects {
        async fn reconcile(
            &self,
            _kind: d2b_provider_guest::GuestKind,
            _request: &d2b_provider_guest::GuestEffectRequest<'_>,
        ) -> Result<
            d2b_provider_guest::GuestEffectOutcome,
            d2b_provider_guest::GuestEffectError,
        > {
            Ok(d2b_provider_guest::GuestEffectOutcome::phase(
                d2b_provider_guest::GuestEffectPhase::Pending,
            ))
        }

        async fn finalize(
            &self,
            _kind: d2b_provider_guest::GuestKind,
            _request: &d2b_provider_guest::GuestEffectRequest<'_>,
        ) -> Result<
            d2b_provider_guest::GuestFinalizeStage,
            d2b_provider_guest::GuestEffectError,
        > {
            Ok(d2b_provider_guest::GuestFinalizeStage::Complete)
        }
    }

    fn test_inputs() -> (tempfile::TempDir, ConstructionInputs, Arc<NewPlaneReadinessState>) {
        let dir = tempfile::tempdir().expect("tempdir");
        let spec_store_dir = dir.path().join("daemon-state/zones/test");
        let readiness = Arc::new(NewPlaneReadinessState::new());
        (
            dir,
            ConstructionInputs {
                zone: ZoneId::parse("test").unwrap(),
                zone_token: BoundedToken::parse("test".to_owned()).unwrap(),
                spec_store_dir: spec_store_dir.clone(),
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
                provider_effects: Arc::new(d2b_provider_provider::FailClosedProviderDriverEffects),
                process_effects: {
                    let effects = Arc::new(
                        d2b_provider_process::test_support::FakeEffects::new(Default::default()),
                    );
                    // The old plane fake reported no retained identity
                    // (has_active false); the shared double's default reports
                    // one (active true), so script it back so the launch path
                    // (and only it) is what the plane tests observe.
                    effects.set_active(false);
                    effects as Arc<dyn ProcessDriverEffects>
                },
                volume_effects: d2b_provider_volume::test_support::FakeLayoutEffects::new(),
                binding_effects: {
                    let effects =
                        d2b_provider_volume_binding::test_support::FakeServingEffects::new();
                    // The old plane fake reported the serving socket present
                    // (socket_ready true); the shared double starts absent.
                    effects.make_ready();
                    effects
                },
                endpoint_effects: {
                    let effects = d2b_provider_endpoint::test_support::FakeSocketEffects::new();
                    // The old plane fake reported the socket present
                    // (socket_present true); the shared double starts absent.
                    effects.make_present();
                    effects
                },
                activation_effects: d2b_provider_activation_nixos::test_support::
                    FakeActivationEffects::new(HostHandoffResult::Incomplete),
                credential_effects: {
                    let effects = d2b_provider_credential::test_support::FakeEffects::new(
                        d2b_provider_credential::test_support::log(),
                    );
                    // The old plane fake answered no provider/execution
                    // facts, no live agent, and no bound session; the shared
                    // double's defaults differ, so script them back.
                    effects.set_facts(None);
                    effects.set_agent_ready(false);
                    effects.set_session(None);
                    effects
                },
                shared_provider_effects: crate::shared_provider_effects::SharedProviderEffects {
                    network: Arc::new(
                        d2b_provider_network_local::test_support::RecordingEffects::default(),
                    ),
                    usbip: Arc::new(
                        d2b_provider_device_usbip::test_support::RecordingEffects::default(),
                    ),
                    security_key: Arc::new(
                        d2b_provider_device_security_key::test_support::RecordingEffects::default(),
                    ),
                    device: Arc::new(
                        d2b_provider_device::test_support::RecordingEffects::default(),
                    ),
                },
                guest_effects: Arc::new(FakeGuestEffects),
                interaction_effects: Arc::new(FakeInteractionEffects),
                trusted_context_publication: None,
                foundation: None,
            },
            readiness,
        )
    }


    /// The providers the plane starts register exactly the converted-type
    /// authority list: no listed type is missing a driver, no driver serves a
    /// type outside the list, and every provider drains through the base in
    /// the reverse of the order it started.
    #[tokio::test(flavor = "multi_thread")]
    async fn started_providers_cover_the_converted_type_authority() {
        let (_dir, inputs, _readiness) = test_inputs();
        let mut runtime = ResourcePlaneV3::start_providers(&inputs).await.expect("providers");
        let providers = runtime.take_directory();
        check_registry_catalog(
            providers.registered_types(),
            &d2b_contracts::identity::V3_CONVERTED_RESOURCE_TYPES,
        )
        .expect("the assembled registry covers the converted-type authority list");
        runtime.drain().await.expect("the providers drain");
    }

    /// The committed startup order is the order the registry has been
    /// assembled in since the family moves landed; drain is its mirror.
    #[tokio::test(flavor = "multi_thread")]
    async fn providers_start_in_the_committed_order_and_drain_in_reverse() {
        let (_dir, inputs, _readiness) = test_inputs();
        let runtime = ResourcePlaneV3::start_providers(&inputs).await.expect("providers");
        assert_eq!(
            runtime.startup_order(),
            [
                "process",
                "volume",
                "volume-binding",
                "endpoint",
                "credential",
                "activation-nixos",
                "telemetry-service",
                "telemetry-binding",
                "network-local",
                "device-usbip",
                "device-security-key",
                "device",
                "guest",
                "host",
                "user",
                "zone",
                "zone-link",
                "provider",
                "role",
                "role-binding",
                "quota",
                "emergency-policy",
                "resource-export",
                "resource-import",
                "command",
                "operation",
                "seccomp-profile",
                "wayland-policy",
                "wayland-session",
                "audio-service",
                "audio-binding",
                "shell-pool",
                "shell-session",
            ]
        );
        runtime.drain().await.expect("the providers drain");
        let mut reversed = runtime.startup_order().to_vec();
        reversed.reverse();
        assert_eq!(runtime.drain_order(), reversed);
    }

    /// A plane's providers drain in the mirror of their startup order, and
    /// the plane reports the same sequence it ran.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_plane_drains_its_providers() {
        let (_dir, inputs, _readiness) = test_inputs();
        let plane = ResourcePlaneV3::prepare(inputs).await.expect("plane prepare");
        let mut reversed = plane.providers().startup_order().to_vec();
        reversed.reverse();
        plane
            .drain_providers()
            .await
            .expect("the providers drain through the base");
        assert_eq!(plane.providers().drain_order(), reversed);
        plane.shutdown().await;
    }

    /// The startup cross-check fails when the registry and the catalog
    /// disagree, naming both sides: the catalog type with no registered
    /// driver and the registered type the catalog does not list.
    #[test]
    fn registry_catalog_mismatch_names_both_sides() {
        let registered = vec![
            ResourceTypeName::new("Process"),
            ResourceTypeName::new("Endpoint"),
        ];
        let error = check_registry_catalog(registered, &["Process", "Volume"]).unwrap_err();
        match error {
            PlaneError::RegistryCatalogMismatch { missing, unexpected } => {
                assert_eq!(missing, vec!["Volume".to_owned()]);
                assert_eq!(unexpected, vec!["Endpoint".to_owned()]);
            }
            other => panic!("wrong failure: {other}"),
        }
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
        let source = &*registry;
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
        let plane = ResourcePlaneV3::prepare(inputs).await.expect("plane prepare");
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
        let plane = ResourcePlaneV3::prepare(inputs).await.expect("plane prepare");

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

    /// U10: every bundle row routes through the manager with provenance Nix;
    /// the retired Phase A partition leaves no row behind for a second plane.
    #[tokio::test(flavor = "multi_thread")]
    async fn bundle_ingest_applies_every_row_to_the_manager() {
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
        assert!(report.applied.contains(&ResourceKey::new("test", "Volume", "state")));
        assert!(report.applied.contains(&ResourceKey::new("test", "Guest", "work")));
        assert_eq!(report.applied.len(), 2, "every row routes through the manager");
        assert!(report.removed.is_empty());
        assert!(report.api_protected.is_empty());

        // Rows persist with provenance Nix before any actor effect ran (F1).
        for key in [
            ResourceKey::new("test", "Volume", "state"),
            ResourceKey::new("test", "Guest", "work"),
        ] {
            let row = plane
                .client()
                .get_row(key)
                .await
                .expect("get_row")
                .expect("committed row");
            assert_eq!(row.provenance, d2b_resource_runtime::identity::ResourceProvenance::Nix);
            assert_eq!(row.generation, 1);
        }

        // Re-ingest is idempotent: no new generation, nothing removed.
        let again = plane.ingest_nix_bundle(&bundle).await.expect("re-ingest");
        assert!(again.applied.contains(&ResourceKey::new("test", "Volume", "state")));
        let row = plane.client().get_row(ResourceKey::new("test", "Volume", "state")).await.unwrap().unwrap();
        assert_eq!(row.generation, 1);
        plane.shutdown().await;
    }

    /// The single plane has no second destination: a bundle row whose type has
    /// no registered driver still commits durably through the manager (its
    /// F1 durability boundary) but fails the ingest, where the deleted Phase A
    /// partition used to pass the row through to the redb store.
    #[tokio::test(flavor = "multi_thread")]
    async fn bundle_ingest_refuses_a_row_without_a_registered_driver() {
        let (_dir, inputs, _readiness) = test_inputs();
        let plane = ResourcePlaneV3::open(inputs).await.expect("plane");
        let key = ResourceKey::new("test", "vendor-extension.d2bus.org.Report", "runner");
        let bundle = test_bundle(vec![bundle_row(
            "vendor-extension.d2bus.org.Report",
            "runner",
            serde_json::json!({}),
        )]);

        let error = plane
            .ingest_nix_bundle(&bundle)
            .await
            .expect_err("no driver serves the type");
        assert!(matches!(error, PlaneError::ManagerRpc(_)), "got {error}");
        let row = plane
            .client()
            .get_row(key)
            .await
            .expect("get_row")
            .expect("the manager committed the row before the spawn failed");
        assert_eq!(row.provenance, d2b_resource_runtime::identity::ResourceProvenance::Nix);
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

    /// U17: a `Process` child a provider controller commits through the
    /// manager is owned and launched by the Process driver. The old plane
    /// wrote this row to the pre-v3 store, where no actor exists - the
    /// committed row now reaches the driver's launch effect, which is what
    /// the fixture's nested-VMM socket wait stands on.
    #[tokio::test(flavor = "multi_thread")]
    async fn controller_committed_process_child_reaches_the_process_driver() {
        let (_dir, mut inputs, _readiness) = test_inputs();
        // The plane's canonical Process effects double: the shared recording
        // fake, kept with a successful one-shot launch (the old plane fake's
        // launch always succeeded), so the committed row is observed at the
        // driver's launch effect.
        let effects = Arc::new(d2b_provider_process::test_support::FakeEffects::new(
            Default::default(),
        ));
        effects.set_active(false);
        let process_effects: Arc<dyn ProcessDriverEffects> = effects.clone();
        inputs.process_effects = process_effects;
        let plane = Arc::new(ResourcePlaneV3::open(inputs).await.expect("plane"));
        // The owner row the child commit is linked under: the production
        // manager holds the Guest (bundle ingest) before any provider
        // controller session commits its children.
        plane
            .ingest_nix_bundle(&test_bundle(vec![bundle_row(
                "Guest",
                "acceptance-guest",
                serde_json::json!({"systemArtifactId": "acceptance-system"}),
            )]))
            .await
            .expect("owner ingest");
        let owner = ResourceRef::parse("Guest/acceptance-guest").expect("owner");
        let target = ResourceRef::parse("Process/acceptance-guest-vmm").expect("target");
        let port = crate::resource_runtime::plane_controller_bridge::PlaneChildMutations::new(
            Arc::clone(&plane),
            ZoneId::parse("test").expect("zone"),
            owner.clone(),
        );

        let committed = port
            .ensure(&target, &controller_child_envelope("running"))
            .await
            .expect("child commit");
        assert_eq!(committed.resource_ref, target);
        assert_ne!(committed.uid.as_str(), "");

        // The driver runs for the committed row: the shared double records
        // the launch, and the row keeps the authored owner reference and
        // spec layer through the manager's rendering.
        let mut launched = false;
        for _ in 0..200 {
            if !effects.launch_calls().is_empty() {
                launched = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        assert!(
            launched,
            "the controller-committed Process child never reached a launch effect"
        );
        // R8: the child commit links this session's owner by uid - the
        // linkage the guest session identity fence compares against.
        let stored = plane
            .client()
            .get_row(ResourceKey::new("test", "Process", "acceptance-guest-vmm"))
            .await
            .expect("row")
            .expect("committed child");
        let owner_row = plane
            .client()
            .get_row(ResourceKey::new("test", "Guest", "acceptance-guest"))
            .await
            .expect("owner row")
            .expect("ingested owner");
        assert_eq!(
            stored.owner_uid,
            Some(owner_row.uid),
            "the controller-committed child must be linked to its owner by uid"
        );
        let view = plane
            .client()
            .get(ResourceKey::new("test", "Process", "acceptance-guest-vmm"))
            .await
            .expect("view")
            .expect("committed child");
        let row = d2b_resource_api::manager_backend::manager_row_stored(&view).expect("render");
        let envelope =
            d2b_contracts_resource::v3::ResourceEnvelope::from_json(&row.canonical_json)
                .expect("envelope");
        assert_eq!(envelope.metadata().owner_ref(), Some(&owner));
        let spec: serde_json::Value =
            serde_json::from_slice(&envelope.spec().base().to_canonical_bytes()).expect("spec");
        assert_eq!(
            spec.get("processClass").and_then(serde_json::Value::as_str),
            Some("worker")
        );
        assert_eq!(
            spec.get("template").and_then(serde_json::Value::as_str),
            Some("cloud-hypervisor-runner")
        );
        plane.shutdown().await;
    }

    /// Regression (P2): the child relist read is owner-scoped. A Zone whose
    /// converted rows exceed the relist's page bound - the 85-guest case,
    /// where the union of every guest's Process/Endpoint/Volume rows crossed
    /// 256 - must still answer exactly this session owner's children, and
    /// another owner's rows must never appear in (or bound) the answer.
    #[tokio::test(flavor = "multi_thread")]
    async fn session_child_rows_are_owner_scoped() {
        let (_dir, inputs, _readiness) = test_inputs();
        let plane = Arc::new(ResourcePlaneV3::open(inputs).await.expect("plane"));
        plane
            .ingest_nix_bundle(&test_bundle(vec![
                bundle_row(
                    "Guest",
                    "acceptance-guest",
                    serde_json::json!({"systemArtifactId": "acceptance-system"}),
                ),
                bundle_row(
                    "Guest",
                    "other-guest",
                    serde_json::json!({"systemArtifactId": "other-system"}),
                ),
            ]))
            .await
            .expect("owner ingest");
        let zone = ZoneId::parse("test").expect("zone");
        let owned = ResourceRef::parse("Process/acceptance-guest-vmm").expect("owned child");
        let port = crate::resource_runtime::plane_controller_bridge::PlaneChildMutations::new(
            Arc::clone(&plane),
            zone.clone(),
            ResourceRef::parse("Guest/acceptance-guest").expect("owner"),
        );
        port.ensure(&owned, &controller_child_envelope("running"))
            .await
            .expect("owned child commit");

        let foreign = ResourceRef::parse("Process/other-guest-vmm").expect("foreign child");
        crate::resource_runtime::plane_controller_bridge::PlaneChildMutations::new(
            Arc::clone(&plane),
            zone.clone(),
            ResourceRef::parse("Guest/other-guest").expect("foreign owner"),
        )
        .ensure(
            &foreign,
            &controller_child_envelope_for("Guest/other-guest", "other-guest-vmm", "running"),
        )
        .await
        .expect("foreign child commit");

        // Both owners' rows really are committed in the Zone ...
        let zone_rows = plane
            .client()
            .list(d2b_resource_runtime::manager::ResourceSelector {
                zone: Some(zone.as_str().to_owned()),
                type_name: Some("Process".to_owned()),
                owner: None,
            })
            .await
            .expect("zone rows");
        assert_eq!(zone_rows.len(), 2, "both owners' rows are committed");

        // ... and the session-scoped relist read answers only its own child.
        let rows = port
            .rows_of_types(&["Process", "Endpoint", "Volume"])
            .await
            .expect("owned rows");
        assert_eq!(
            rows.iter()
                .map(|row| row.resource_ref.to_canonical_string())
                .collect::<Vec<_>>(),
            vec![owned.to_canonical_string()],
            "the relist read must return exactly this owner's children",
        );
    }

    /// The retained-identity report the shared Process effects double
    /// replays as the scripted adoption: the driver's `Ready` (and only
    /// `Ready`) publishes the committed child row, exactly as the production
    /// provider's retained identity does. `ProviderAdoption::Adopted` with
    /// this report is queued on the double after the first launch.
    fn adopted_report() -> d2b_process_conformance::ProcessStatusReport {
        d2b_process_conformance::ProcessStatusReport {
            provider: BoundedToken::parse("system-minijail").expect("provider token"),
            identity: ProcessIdentityDigest::from_bytes([0x51; 32]),
            wait_reap_owner: d2b_process_conformance::WaitReapOwner::Local,
            execution_ref: ResourceRef::parse("Host/host-system").expect("execution ref"),
            domain: d2b_contracts_resource::v3::execution_policy::ExecutionDomain::System,
            user_ref: None,
            digests: d2b_process_conformance::testing::fixtures::compiled_digests(),
            phase: d2b_process_conformance::ProcessPhaseClass::Ready,
            last_exit: None,
            adoption: d2b_process_conformance::AdoptionCondition::Adopted,
        }
    }

    /// U17: the guest-runtime control endpoints (`ch-api`, `guest-control`)
    /// are realized by the guest's committed VMM Process row being `Ready` -
    /// the exact evidence the old daemon publication stage wrote both rows
    /// `Ready` from. The plane's probe reads that row through the published
    /// plane table, so the Endpoint actor publishes the same evidence the
    /// guest's provider controller gates its Guest readiness on.
    #[tokio::test(flavor = "multi_thread")]
    async fn guest_control_endpoints_are_realized_with_the_committed_vmm_process() {
        let (_dir, mut inputs, _readiness) = test_inputs();
        // The plane's canonical Process effects double adopts the committed
        // VMM row after the first launch (`Absent` comes first from the
        // default queue, then the scripted `Adopted` report): the driver's
        // `Ready` - and only `Ready` - publishes the evidence row, exactly
        // as the production provider's retained identity does.
        let effects = Arc::new(d2b_provider_process::test_support::FakeEffects::new(
            Default::default(),
        ));
        effects.set_active(false);
        effects.push_adoption(d2b_provider_process::ProviderAdoption::Adopted(
            adopted_report(),
        ));
        let process_effects: Arc<dyn ProcessDriverEffects> = effects.clone();
        inputs.process_effects = process_effects;
        let zone = ZoneId::parse("test").expect("zone");
        let planes: Arc<parking_lot::Mutex<HashMap<String, Arc<ResourcePlaneV3>>>> =
            Arc::new(parking_lot::Mutex::new(HashMap::new()));
        let plane = Arc::new(ResourcePlaneV3::open(inputs).await.expect("plane"));
        // The owner row the child commits are linked under, exactly as the
        // production ingest commits the Guest before its provider controller
        // session runs.
        plane
            .ingest_nix_bundle(&test_bundle(vec![bundle_row(
                "Guest",
                "acceptance-guest",
                serde_json::json!({"systemArtifactId": "acceptance-system"}),
            )]))
            .await
            .expect("owner ingest");
        planes
            .lock()
            .insert(zone.as_str().to_owned(), Arc::clone(&plane));
        let probe = GuestControlEndpointProbe::new(Arc::clone(&planes), zone.clone());
        let guest = ResourceRef::parse("Guest/acceptance-guest").expect("guest");
        // The producer the provider's own child-role vocabulary declares per
        // purpose: `ch-api` on the guest's VMM Process, `guest-control` on
        // the Guest.
        let vmm = ResourceRef::parse("Process/acceptance-guest-vmm").expect("vmm");

        // Before the guest's provider controller commits anything the
        // evidence is absent, and a purpose outside the control family is
        // never this probe's answer.
        assert!(!probe.present(&vmm, "ch-api").await);
        assert!(!probe.present(&guest, "guest-control").await);
        assert!(!probe.present(&guest, "virtiofsd").await);
        // The producer/purpose pair is the provider's declaration, not a
        // free choice: neither purpose is realized off the other's producer.
        assert!(!probe.present(&guest, "ch-api").await);
        assert!(!probe.present(&vmm, "guest-control").await);

        let port = crate::resource_runtime::plane_controller_bridge::PlaneChildMutations::new(
            Arc::clone(&plane),
            zone.clone(),
            guest.clone(),
        );
        port.ensure(
            &ResourceRef::parse("Process/acceptance-guest-vmm").expect("target"),
            &controller_child_envelope("running"),
        )
        .await
        .expect("child commit");

        let mut realized = false;
        for _ in 0..200 {
            if probe.present(&vmm, "ch-api").await
                && probe.present(&guest, "guest-control").await
            {
                realized = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        assert!(
            realized,
            "both control endpoints must report realized once the committed VMM Process row is Ready"
        );
        plane.shutdown().await;
    }

    /// The anchored walk that resolves the store-view farm must need only
    /// search permission on the ancestors (the chain is deliberately
    /// traversal-only for the daemon) and must still hand back the leaf.
    #[test]
    fn anchored_walk_needs_only_search_on_ancestors() {
        use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
        let dir = tempfile::tempdir().expect("tempdir");
        let farm = dir
            .path()
            .join("zones/work/guests/acceptance-guest/store-view");
        std::fs::create_dir_all(&farm).expect("farm");
        // Search-only intermediate: the daemon's grant on the chain is `--x`.
        let traversal_only = dir.path().join("zones/work/guests");
        std::fs::set_permissions(&traversal_only, std::fs::Permissions::from_mode(0o111))
            .expect("chmod traversal-only");

        let opened = open_anchored_directory(&farm).expect("search-only ancestors must open");

        let leaf = std::fs::symlink_metadata(&farm).expect("farm stat");
        let handle = rustix::fs::fstat(&opened).expect("fstat");
        assert_eq!(handle.st_ino, leaf.ino(), "the handle is the leaf inode");
    }

    /// One provider-authored child envelope as the Cloud Hypervisor controller
    /// renders it: the authored identity (no uid - the store stamps it), the
    /// spec layer, and the status the driver replaces.
    fn controller_child_envelope(desired_lifecycle: &str) -> Vec<u8> {
        controller_child_envelope_for(
            "Guest/acceptance-guest",
            "acceptance-guest-vmm",
            desired_lifecycle,
        )
    }

    /// The same envelope for one named owner/child pair: the owner-scoped
    /// relist regression commits a second owner's child beside the first.
    fn controller_child_envelope_for(owner: &str, name: &str, desired_lifecycle: &str) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "apiVersion": "resources.d2bus.org/v3",
            "type": "Process",
            "metadata": {
                "name": name,
                "zone": "test",
                "ownerRef": owner,
                "finalizers": [],
                "deletionRequestedAt": null,
                "createdAt": "1970-01-01T00:00:00.000Z",
                "updatedAt": "1970-01-01T00:00:00.000Z",
                "generation": 1,
                "revision": 1,
                "managedBy": "controller",
            },
            "spec": {
                "providerRef": "Provider/system-minijail",
                "executionRef": "Host/host-system",
                "processClass": "worker",
                "template": "cloud-hypervisor-runner",
                "desiredLifecycle": desired_lifecycle,
                "sandbox": {},
            },
            "status": {
                "observedGeneration": 0,
                "phase": "Pending",
                "conditions": [],
                "resource": {},
            },
        }))
        .expect("child envelope")
    }
    /// A system-homed policy row as a caller would submit it.
    fn command_desired(zone: &str, name: &str) -> DesiredResource {
        let spec = d2b_contracts_resource::v3::canonical_json_bytes(&serde_json::json!({
            "exec": "/usr/lib/d2b/libexec/virtiofsd",
            "argv": ["--socket-path", "{socketPath}"],
            "params": {
                "type": "object",
                "additionalProperties": false,
                "properties": { "socketPath": { "type": "string" } }
            },
            "roleRef": "Role/worker",
            "intent": { "grammar": "<zone>/<name>", "mint": "per-bundle-entry" }
        }))
        .expect("canonical command spec");
        let metadata = serde_json::to_vec(&serde_json::json!({
            "annotations": {},
            "labels": {},
            "ownerRef": null
        }))
        .expect("metadata");
        DesiredResource {
            key: ResourceKey::new(zone, "Command", name),
            spec,
            metadata,
            provenance: d2b_resource_runtime::spec_store::ResourceProvenance::Api,
        }
    }

    fn api_subject(principal: &str) -> d2b_resource_runtime::manager::MutationSubject {
        d2b_resource_runtime::manager::MutationSubject {
            principal: principal.to_owned(),
            origin: d2b_resource_runtime::spec_store::ResourceProvenance::Api,
        }
    }

    /// A zone-local plane is not the foundation plane: a system-homed row is
    /// refused terminally, naming the type and the caller.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_zone_local_plane_refuses_a_system_homed_row() {
        let (_dir, inputs, _readiness) = test_inputs();
        let plane = ResourcePlaneV3::open(inputs).await.expect("plane");

        let error = plane
            .client()
            .apply(api_subject("User/alice"), command_desired("test", "worker"))
            .await
            .expect_err("a system-homed write is refused");
        assert!(
            matches!(
                &error,
                d2b_resource_runtime::error::ResourceError::AdmissionDenied {
                    type_name,
                    principal,
                    ..
                } if type_name == "Command" && principal == "User/alice"
            ),
            "unexpected refusal: {error:?}"
        );
        assert!(
            error.to_string().contains("wrong plane"),
            "the refusal keeps the named shape: {error}"
        );
        // The refused row never reached the durable store.
        assert!(plane.store().list(SpecSelector::default()).await.expect("list")
            .iter()
            .all(|row| row.key.type_name != "Command"));
    }

    /// The foundation plane commits the seeded policy rows before its manager
    /// starts, and admits the writes only it may make.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_foundation_plane_commits_the_seed_and_admits_the_system_rows() {
        let (_dir, mut inputs, _readiness) = test_inputs();
        inputs.foundation = Some(FoundationInputs {
            declarations: crate::foundation_seed::core_declarations(),
            allocation: crate::principal_allocation::PrincipalAllocation::committed()
                .expect("committed allocation"),
        });
        let plane = ResourcePlaneV3::open(inputs).await.expect("plane");

        let rows = plane
            .store()
            .list(SpecSelector::default())
            .await
            .expect("list rows");
        let refs: Vec<String> = rows
            .iter()
            .map(|row| format!("{}/{}", row.key.type_name, row.key.name))
            .collect();
        assert!(refs.contains(&"Zone/system".to_owned()), "refs: {refs:?}");
        assert!(
            refs.contains(&"Role/operation-publisher".to_owned()),
            "refs: {refs:?}"
        );
        assert!(
            refs.iter().any(|reference| reference.starts_with("RoleBinding/")),
            "refs: {refs:?}"
        );
        // The system-homed write is admitted on this plane.
        plane
            .client()
            .apply(api_subject("User/alice"), command_desired("system", "worker"))
            .await
            .expect("the foundation plane admits the write");
    }

    /// The one declared spawn command, under the seed's publisher role.
    fn seeded_command(name: &str) -> crate::foundation_seed::SeedCommand {
        let spec = d2b_contracts_resource::v3::canonical_json_bytes(&serde_json::json!({
            "exec": "/usr/lib/d2b/libexec/virtiofsd",
            "argv": ["--socket-path", "{socketPath}"],
            "params": {
                "type": "object",
                "additionalProperties": false,
                "properties": { "socketPath": { "type": "string" } }
            },
            "roleRef": "Role/operation-publisher",
            "intent": { "grammar": "<zone>/<name>", "mint": "per-bundle-entry" }
        }))
        .expect("canonical command spec");
        crate::foundation_seed::SeedCommand {
            name: name.to_owned(),
            spec: serde_json::from_slice(&spec).expect("command spec"),
        }
    }

    /// The seed's own vocabulary, plus one declared command so the controller
    /// materializes an `Operation` row to resolve.
    fn seeded_declarations(command: &str) -> crate::foundation_seed::FoundationDeclarations {
        let mut declarations = crate::foundation_seed::core_declarations();
        let command = seeded_command(command);
        declarations.roles[0].spec = serde_json::from_value(serde_json::json!({
            "rules": [{
                "resourceTypes": ["Operation"],
                "verbs": ["create"],
                "subresources": [],
                "resourceNames": [],
                "zones": [],
                "executionRefs": [],
                "sessionVerbs": []
            }],
            "commandRefs": [format!("Command/{}", command.name)],
        }))
        .expect("publisher role scoped to the declared command");
        declarations.commands = vec![command];
        declarations
    }

    /// The `Provider` row the seeded self-binding's subject resolves through.
    ///
    /// The seed homes its rows in the reserved system Zone and publishes the
    /// provider identity without a row, so the plane's own Zone carries the
    /// subject row: the two-Zone shape the policy read path must span. The
    /// row is committed before the plane opens so the manager indexes it.
    async fn seed_provider_row(inputs: &ConstructionInputs, provider_ref: &ResourceRef) -> ResourceUid {
        let key = ResourceKey::new("test", "Provider", provider_ref.name().as_str());
        let uid = d2b_resource_runtime::manager::deterministic_uid(&key);
        let store =
            SpecStore::open(ResourcePlaneV3::spec_store_path(&inputs.spec_store_dir)).expect("store");
        store
            .ensure(StoredDesiredResource {
                uid,
                key,
                generation: 1,
                owner_uid: None,
                provenance: d2b_resource_runtime::spec_store::ResourceProvenance::Nix,
                deleting: false,
                spec: b"{}".to_vec(),
                metadata: br#"{"annotations":{},"labels":{},"ownerRef":null}"#.to_vec(),
                created_at: 0,
            })
            .await
            .expect("provider row committed");
        drop(store);
        d2b_provider_process::resource_uid_from_bytes(&uid).expect("UUIDv4-shaped provider uid")
    }

    /// The committed controller subject: the provider the seed self-bound.
    fn controller_subject(
        provider_ref: &ResourceRef,
        provider_uid: ResourceUid,
        zone: &ZoneId,
    ) -> d2b_contracts_resource::v3::identity::AuthenticatedSubjectContext {
        use d2b_contracts_resource::v3::SchemaFingerprint;
        use d2b_contracts_resource::v3::identity::{
            AuthenticatedSubjectContext, BindingDigest, EvidenceClass, Locality,
            ReconnectGeneration, ServiceName, SessionBinding, SessionPurpose, TranscriptHash,
            TransportBinding,
        };

        AuthenticatedSubjectContext::new(
            provider_ref.clone(),
            provider_uid,
            ResourceRef::parse(&format!("Zone/{}", zone.as_str())).expect("zone ref"),
            EvidenceClass::UnixPeer,
            SessionPurpose::parse("resource-api").expect("purpose"),
            ServiceName::parse("d2b.resource.v3").expect("service"),
            SessionBinding::new(
                SchemaFingerprint::parse(format!("sha256:{}", "1".repeat(64)))
                    .expect("schema fingerprint"),
                TransportBinding::new(
                    Locality::Local,
                    BindingDigest::parse(format!("sha256:{}", "2".repeat(64)))
                        .expect("binding digest"),
                ),
                ReconnectGeneration::new(1).expect("reconnect generation"),
                TranscriptHash::from_bytes([3; 32]),
            ),
        )
    }

    /// The whole committed authority chain resolves through the read path.
    ///
    /// The seed commits the built-in role, the provider self-binding, the
    /// declared command, and the operation it materializes into the reserved
    /// system Zone. A read that selected the plane's own Zone alone would see
    /// none of them, so this pins that the committed policy compile and the
    /// row reads reach the system Zone - and that the grant the chain exists
    /// for (the controller creating its operations) is installed.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_seeded_system_vocabulary_resolves_through_the_policy_read_path() {
        use d2b_resource_api::authz::{
            ApiCatalog, ApiMethod, AuthorizationRequest, AuthorizationTarget, NativeAuthorizer,
            ResourceVerb,
        };

        let command = "virtiofsd-worker";
        let (_dir, mut inputs, _readiness) = test_inputs();
        // The readers resolve the Zone the seed homes its rows under: two
        // declarations of the reserved name would put the commit and the read
        // back out of agreement.
        assert_eq!(
            crate::foundation_seed::SYSTEM_ZONE,
            d2b_contracts::identity::SYSTEM_ZONE_NAME,
            "the seed homes its rows in the Zone the readers select",
        );
        let declarations = seeded_declarations(command);
        let provider_ref = declarations
            .providers
            .first()
            .expect("the seed declares the process provider")
            .provider_ref
            .clone();
        let provider_uid = seed_provider_row(&inputs, &provider_ref).await;
        inputs.foundation = Some(FoundationInputs {
            declarations,
            allocation: crate::principal_allocation::PrincipalAllocation::committed()
                .expect("committed allocation"),
        });
        let zone = inputs.zone.clone();
        let plane = ResourcePlaneV3::open(inputs).await.expect("plane");
        let view = crate::resource_runtime::plane_controller_bridge::ManagerControllerPlaneView::new(
            plane.client().clone(),
            zone.clone(),
        );

        // Every row of the chain is read back, homed in the reserved Zone.
        let resources = crate::resource_runtime::committed_policy_resources(&view)
            .await
            .expect("committed policy read");
        let homed = |reference: &str| {
            resources
                .iter()
                .find(|row| row.resource_ref.to_canonical_string() == reference)
                .map(|row| row.zone.as_str().to_owned())
        };
        // Every policy row of the chain is read back, homed in the system Zone.
        let chain = [
            "Role/operation-publisher".to_owned(),
            "RoleBinding/system-minijail-self-operation-publisher".to_owned(),
            format!("Zone/{}", crate::foundation_seed::SYSTEM_ZONE),
        ];
        for reference in &chain {
            assert_eq!(
                homed(reference).as_deref(),
                Some(crate::foundation_seed::SYSTEM_ZONE),
                "the policy read resolves {reference}: {:?}",
                resources
                    .iter()
                    .map(|row| row.resource_ref.to_canonical_string())
                    .collect::<Vec<_>>(),
            );
        }
        // The declared command and the operation it materialized are read back
        // by reference, the shape an invocation resolves them in.
        let declared = [
            format!("Command/{command}"),
            format!("Operation/process-run-{command}"),
        ];
        for reference in &declared {
            let target = ResourceRef::parse(reference).expect("resource reference");
            let row = crate::resource_runtime::bridge_manager_row(&view, &target)
                .await
                .expect("row read")
                .unwrap_or_else(|| panic!("the read path resolves {reference}"));
            assert_eq!(row.zone.as_str(), crate::foundation_seed::SYSTEM_ZONE);
        }
        // The self-binding's subject resolves through the same read, so the
        // binding is not dropped as unresolved.
        let fingerprints =
            d2bd_runtime::resource_runtime_support::committed_policy_subject_fingerprints(
                &resources,
            )
            .expect("subject fingerprints");
        assert!(
            fingerprints.contains_key(&(
                ResourceRef::parse("RoleBinding/system-minijail-self-operation-publisher")
                    .expect("binding ref"),
                provider_ref.clone(),
            )),
            "the seeded binding resolved its subject, fingerprints: {}",
            fingerprints.len(),
        );

        // The compiled policy installs the grant the chain exists for: the
        // self-bound provider creates the operation its command materialized.
        let snapshot = d2bd_runtime::resource_runtime_support::initial_policy_snapshot()
            .expect("bootstrap snapshot");
        let (policy, state) =
            d2bd_runtime::resource_runtime_support::compile_committed_policy_with_subjects(
                &zone,
                snapshot,
                ZoneRevision::new(snapshot.policy_revision),
                &[],
                &resources,
                std::iter::empty(),
            )
            .expect("committed policy compiles");
        let authorizer = NativeAuthorizer::new(ApiCatalog::standard(), Some(policy))
            .expect("authorizer over the compiled policy");
        let grant = authorizer.authorize(
            &controller_subject(&provider_ref, provider_uid, &zone),
            &AuthorizationRequest {
                method: ApiMethod::Create,
                zone: zone.clone(),
                targets: vec![AuthorizationTarget {
                    resource_type: d2b_contracts_resource::v3::ResourceTypeName::parse(
                        "Operation".to_owned(),
                    )
                    .expect("operation type"),
                    resource_name: Some(
                        ResourceName::parse(format!("process-run-{command}"))
                            .expect("operation name"),
                    ),
                    verb: ResourceVerb::Create,
                    subresource: None,
                    execution_ref: None,
                }],
            },
            &state,
        );
        assert!(
            grant.is_ok(),
            "the seeded self-binding grants the controller its operation: {grant:?}"
        );
    }
}
