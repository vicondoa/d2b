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
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use d2b_contracts_broker::broker_wire::{
    BrokerCallerRole, BrokerRequest, BrokerResponse, StoreSyncRequest,
};
use d2b_contracts_resource::v3::{
    ControllerGeneration, ResourceGeneration, ResourceRef, ResourceUid, ZoneId, ZoneRevision,
    execution_policy::BoundedToken,
    volume::{SourceKind, VolumeSpec},
    volume_binding::VolumeBindingSpec,
};
use d2b_contracts_zone_session::v3::resource_bundle::{BundleResource, ResourceBundle};
use d2b_core::bundle_resolver::{BundleResolver, ResolvedStoreViewIntent, intent_id_store_view};
use d2b_provider_activation_nixos::{
    ACTIVATION_EFFECTS_SERVICE, ActivationDriverArgs, ActivationEffectFacets,
    ActivationEffectsServiceFactory, activation_descriptor,
};
use d2b_provider_endpoint::{
    ENDPOINT_EFFECTS_SERVICE, DeviceWorkerEvidenceSource, EndpointDriverArgs,
    EndpointEffectFacets, EndpointEffectsServiceFactory, EndpointSocketSource,
    GuestControlProducer, GuestVmmEvidenceSource, device_worker_purpose, endpoint_descriptor,
    guest_control_producer,
};
use d2b_provider_guest::{
    GUEST_EFFECTS_SERVICE, GuestDriverArgs, GuestEffectFacets, GuestEffectsServiceFactory,
    guest_descriptor,
};
use d2b_provider_host::{HOST_EFFECTS_SERVICE, HostEffectFacets, HostEffectsServiceFactory, host_descriptor};
use d2b_provider_user::{USER_EFFECTS_SERVICE, UserEffectFacets, UserEffectsServiceFactory, user_descriptor};
use d2b_provider_process::{
    CommittedProviderIdentitySource, GuestOwnerIdentitySource, PROCESS_EFFECTS_SERVICE,
    ProcessDriverArgs, ProcessEffectFacets, ProcessEffectsServiceFactory, ProcessProviderRuntime,
    decode_metadata_owner_ref, process_family_descriptors,
};
use d2b_provider_telemetry_binding::telemetry_binding_descriptor;
use d2b_provider_telemetry_service::telemetry_service_descriptor;
// The registered provider families and their declared services, generated
// from the per-crate `registrations.json` declarations (the composition
// root composes the table instead of naming families).
include!("generated/provider_registrations.rs");
use d2b_provider_volume::{
    VOLUME_EFFECTS_SERVICE, VolumeDriverArgs, VolumeEffectFacets, VolumeEffectsServiceFactory,
    VolumeRuntime, volume_descriptor,
};
use d2b_provider_volume_binding::{
    BINDING_EFFECTS_SERVICE, BindingDriverArgs, BindingEffectFacets, BindingEffectsServiceFactory,
    GuestMountSource, SocketReadySource, SocketRemoveSource, binding_descriptor,
};
use d2b_provider_volume_local::{
    AnchoredVolumeEffectAdapter, VolumeLocalController, VolumeLocalProfile,
};
use d2b_provider_volume_virtiofs::{MAX_SOCKET_PATH_BYTES, SocketIdentity, StoredBinding};
use d2b_resource_api::manager_backend::nix_bundle_subject;
use d2b_resource_runtime::context::{ManagerEndpoint, SpecDecoder};
use d2b_resource_runtime::error::ResourceError;
use d2b_resource_runtime::identity::{ResourceKey, ResourceTypeName};
use d2b_resource_runtime::resource::ResourceStatus;
use d2b_resource_runtime::manager::{
    DesiredResource, ManagerActorEndpoint, ResourceManager, ResourceManagerArgs,
    ResourceManagerClient, ResourceManagerMsg, ResourceSelector, ResourceView,
};
use d2b_resource_runtime::revision::RuntimeRevision;
use d2b_resource_runtime::spec_store::{SpecSelector, SpecStore, StoredDesiredResource};
use d2b_resource_runtime::GuestTargetControl;
use d2b_resource_runtime::target::{TargetDirectory, TargetRef, TargetResolver};
use d2b_resource_runtime::watch::{
    ChangeSource, DEFAULT_RING_CAPACITY, RevisionExpired, WatchDelivery, WatchHub, WatchRegistration,
    WatchSelector,
};
use d2b_resource_types::DriverDescriptor;
use d2bd_runtime::resource_runtime_support::NewPlaneReadinessState;
use d2bd_runtime::target_runtime::DaemonMode;
use rustix::fs::{Mode, OFlags, ResolveFlags, open, openat2};
use sha2::{Digest, Sha256};

use d2b_provider_credential::{
    CREDENTIAL_EFFECTS_SERVICE, CredentialDriverArgs, CredentialEffectFacets,
    CredentialEffectsServiceFactory, CredentialRuntime, credential_descriptor,
};
use crate::process_provider_runtime::PlaneCommittedProviderIdentitySource;
use crate::provider_lifecycle::{
    ProviderRuntime, ProviderSet, ProviderStartupError, TrustedContextPublication,
    family_declaration,
};
use d2b_provider_toolkit::EffectServiceFactory;
use d2b_provider_device::{
    DEVICE_EFFECTS_SERVICE, DeviceDriverArgs, device_descriptor,
};
use d2b_provider_device_security_key::{
    SECURITY_KEY_EFFECTS_SERVICE, SecurityKeyDriverArgs, security_key_descriptors,
};
use d2b_provider_device_usbip::{
    USBIP_EFFECTS_SERVICE, UsbipDriverArgs, usbip_descriptors,
};

use d2b_provider_network_local::{
    NETWORK_EFFECTS_SERVICE, NetworkDriverArgs, NetworkEffectFacets, NetworkEffectsServiceFactory,
    network_descriptor,
};
use d2b_provider_process_systemd::effects_service::{
    PROCESS_SYSTEMD_EFFECTS_SERVICE, SystemdEffectsServiceFactory,
};
use crate::shared_provider_effects::ProductionSharedProviderEffects;
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
use d2b_provider_audio_binding::{
    AudioBinding, audio_binding_descriptor,
};
use d2b_provider_audio_service::{AudioService, audio_service_descriptor};
use d2b_provider_shell_pool::{ShellPool, shell_pool_descriptor};
use d2b_provider_shell_session::{ShellSession, shell_session_descriptor};
use d2b_provider_audio_pipewire::AudioMediator;
use d2b_provider_wayland_policy::{
    AudioMediatorSource, InteractionDriverArgs, InteractionEffectFacets, InteractionEffectsService,
    InteractionIdentitySource, InteractionPlaneRead, WaylandPolicy, wayland_policy_descriptor,
};
use d2b_provider_wayland_session::{WaylandSession, wayland_session_descriptor};

/// The construction arguments every interaction driver of this plane shares.
///
/// Construction is infallible by contract: the zone was validated at plane
/// construction and the effects are the family's own implementation built
/// from the plane's facet set (U12) - the same shared value every type of the
/// family drives, so the family's per-zone controller state is shared across
/// the six types.
fn interaction_driver_args<T: d2b_provider_wayland_policy::InteractionType>(
    inputs: &ConstructionInputs,
    behavior: T,
) -> InteractionDriverArgs<T> {
    InteractionDriverArgs {
        zone: inputs.zone.as_str().to_owned(),
        controller_generation: inputs.authority.controller_generation,
        effects: Arc::new(InteractionEffectsService::new(
            inputs.interaction_facets.clone(),
        )),
        behavior,
    }
}

/// Preserved reconcile backoff for the plane's resource actors (R13).
const PLANE_BACKOFF: Duration = d2b_resource_runtime::DEFAULT_REQUEUE_BACKOFF;

/// Bounded wait budget for the binding-owned virtiofsd socket bind: the
/// worker Process child binds the private socket after its launch, and the
/// daemon's socket facet waits this budget before reporting a retryable
/// failure (the actor owns the retry, R13). The endpoint family's own
/// evidence budget lives in the family crate.
const SOCKET_BIND_BUDGET: Duration = Duration::from_secs(5);

/// The anchor projection drain window: one bounded re-materialization per
/// drain, at most one window after the first notice of the drain. The window
/// is measured from the first notice, not from the stream emptying, so a
/// burst coalesces into one re-materialization and sustained traffic cannot
/// starve it.
///
/// The window is short on purpose. A committing effect returns before its
/// notice is drained, so this window is the delay before that commit's
/// anchor exists; a lone commit must not wait longer than the synchronous
/// refresh it replaced. A tight burst still publishes well inside it.
const ANCHOR_DRAIN_WINDOW: Duration = Duration::from_millis(3);

/// Consecutive busy drains before the subscription reports that it is
/// falling behind: one busy drain is a burst, several in a row is overload.
const ANCHOR_BUSY_DRAINS_BEFORE_WARN: u64 = 3;

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
    inner: tokio::sync::Mutex<RegistryInner>,
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

    async fn with_inner<R>(&self, run: impl FnOnce(&mut RegistryInner) -> R) -> R {
        let mut inner = self.inner.lock().await;
        run(&mut inner)
    }

    /// Synchronous cache access: non-blocking `try_lock` per plan U4. A
    /// collision reports a miss (fail-closed) - the async socket-target
    /// lookups fall through to the authority on a miss, and the fences
    /// treat an unbound identity as unavailable.
    fn with_inner_sync<R>(&self, run: impl FnOnce(&mut RegistryInner) -> R) -> Option<R> {
        let mut inner = self.inner.try_lock().ok()?;
        Some(run(&mut inner))
    }

    fn lookup_anchor(&self, volume_uid: &ResourceUid) -> Option<VolumeAnchor> {
        self.with_inner_sync(|inner| {
            let volume_name = inner.volume_names_by_uid.get(volume_uid.as_str()).cloned()?;
            inner.volume_anchors_by_name.get(&volume_name).cloned()
        })
       .flatten()
    }

    async fn lookup_socket_target_by_identity(
        &self,
        socket: &SocketIdentity,
    ) -> Option<SocketTarget> {
        self.with_inner(|inner| inner.socket_targets_by_identity.get(&socket.to_hex()).cloned())
           .await
    }

    async fn lookup_socket_target_by_ref(&self, producer_ref: &ResourceRef) -> Option<SocketTarget> {
        self.with_inner(|inner| {
            inner
               .socket_targets_by_ref
               .get(&producer_ref.to_canonical_string())
               .cloned()
        })
       .await
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
                    register_binding_row(self, zone_token, row).await;
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
        if let Some(target) = self.lookup_socket_target_by_identity(socket).await {
            return Some(target);
        }
        self.load_binding_targets(zone_token).await;
        self.lookup_socket_target_by_identity(socket).await
    }

    /// Socket target for one serving-pair ref (worker Process or Endpoint):
    /// cache first, then the authority on a miss.
    async fn socket_target_by_ref(
        &self,
        zone_token: &BoundedToken,
        producer_ref: &ResourceRef,
    ) -> Option<SocketTarget> {
        if let Some(target) = self.lookup_socket_target_by_ref(producer_ref).await {
            return Some(target);
        }
        self.load_binding_targets(zone_token).await;
        self.lookup_socket_target_by_ref(producer_ref).await
    }

    async fn register_volume(&self, volume_uid: &str, volume_name: &str, anchor: VolumeAnchor) {
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
                    // name-only registration, and a later identity wins over
                    // an earlier one: an Updated Volume whose attachment moved
                    // must not keep the previous guest's identity, or every
                    // later reload would re-register the same stale anchor.
                    // A name-only anchor still never downgrades one that
                    // already carries a role.
                    if anchor.role.is_some() || existing.role.is_none() {
                        *existing = anchor.clone();
                    }
                })
               .or_insert(anchor);
        })
       .await;
    }

    async fn register_binding(
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
        })
       .await;
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
                    )
                   .await;
                }
                "VolumeBinding" => register_binding_row(self, zone_token, &row).await,
                _ => {}
            }
        }
        Ok(())
    }

    /// Publish one committed `Provider` row's identity (KTD7): the production
    /// Process effects bind it to controller rows that Provider owns. Fed by
    /// the plane's construction path from
    /// [`ConstructionInputs::committed_provider_identities`].
    pub(crate) async fn register_committed_provider_identity(
        &self,
        provider_ref: &ResourceRef,
        uid: ResourceUid,
        generation: d2b_contracts_resource::v3::ResourceGeneration,
    ) {
        self.with_inner(|inner| {
            inner
               .committed_provider_identities
               .insert(provider_ref.to_canonical_string(), (uid, generation));
        })
       .await;
    }

    /// The committed-`Provider` identity view the production Process effects
    /// consult (KTD7), published by [`PlaneResourceRegistry`].
    ///
    /// Synchronous surface over the `tokio::sync` inner (plan U4): the
    /// non-blocking `try_lock` reports unbound on a collision (fail-closed);
    /// the fences refuse as unavailable and the effects retry.
    pub(crate) fn committed_provider_identity(
        &self,
        provider_ref: &ResourceRef,
    ) -> Option<(ResourceUid, d2b_contracts_resource::v3::ResourceGeneration)> {
        self.with_inner_sync(|inner| {
            inner
               .committed_provider_identities
               .get(&provider_ref.to_canonical_string())
               .cloned()
        })
       .flatten()
    }
}

async fn register_binding_row(registry: &PlaneResourceRegistry, zone_token: &BoundedToken, row: &StoredDesiredResource) {
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
    registry
       .register_binding(
            &socket.to_hex(),
            &worker_ref,
            &endpoint_ref,
            SocketTarget {
                volume_ref: stored.spec().volume_ref().clone(),
                execution_ref: stored.spec().execution_ref().clone(),
            },
        )
       .await;
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
            if volume_name == format!(
                    "{}{}",
                    d2b_provider_volume_local::STORE_VIEW_VOLUME_NAME_PREFIX,
                    guest.name().as_str()
                ) =>
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
    ResourceUid::from_bytes(bytes).map_err(|_| ())
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
// Anchor projection subscription
// ---------------------------------------------------------------------------

/// The subscription's selector: Volume and VolumeBinding rows, the durable
/// rows the anchor projection holds. The hub matches on the resource key
/// alone, so status transitions for these rows arrive on the same
/// subscription and are filtered to [`ChangeSource::Desired`] before the
/// pending flag is set.
fn anchor_projection_selector() -> WatchSelector {
    WatchSelector::with_predicate(|key| {
        key.type_name == "Volume" || key.type_name == "VolumeBinding"
    })
}

/// Shared, observable state of the anchor projection subscription: the
/// coalescing pending flag (KTD2) and the counters this module's tests
/// assert against.
#[derive(Debug, Default)]
struct AnchorSubscriptionState {
    /// Set by a drained Desired notice, cleared when the drain acts: the
    /// reconciler's single-pending-flag coalescing shape. The flag itself is
    /// transient, so `pending_sets` is its deterministic observable form.
    pending: AtomicBool,
    /// How often a drained notice set `pending` (one per drain).
    pending_sets: AtomicU64,
    /// Completed drain actions (one bounded re-materialization each).
    rematerializations: AtomicU64,
    /// Completed recovery actions (relist + reload + fresh registration).
    relists: AtomicU64,
    /// Consecutive drains whose bounded window passed while notices were
    /// still arriving. A burst is normal; a drain that never quiesces is a
    /// consumer falling behind, so only the sustained case is reported.
    consecutive_busy: AtomicU64,
}


/// Spawn the plane's anchor projection subscription: one long-lived task
/// consuming the manager's durable-change stream for Volume and
/// VolumeBinding rows. The plane owns no other long-lived task - the manager
/// actor supervises itself - so a caller that needs to stop or replace the
/// subscription keeps the returned handle, and a restart is a fresh spawn
/// anchored after the commit the reload must heal.
fn spawn_anchor_subscription(
    hub: Arc<WatchHub>,
    registry: Arc<PlaneResourceRegistry>,
    store: Arc<SpecStore>,
    zone_token: BoundedToken,
    anchor: RuntimeRevision,
    window: Duration,
    state: Arc<AnchorSubscriptionState>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(run_anchor_subscription(
        hub,
        anchor_projection_selector(),
        registry,
        store,
        zone_token,
        state,
        anchor,
        window,
    ))
}

/// Run the anchor projection subscription: drain each registration's
/// retained replay, then its live stream, coalescing Desired notices into
/// one bounded re-materialization per drain, and relist on the hub's
/// unservable signals.
///
/// This subscription is the projection's only law: the writer-side refreshes
/// are retired, so a commit path never owes the projection a call. A
/// resolution that races a commit can therefore miss the anchor, and that miss
/// is a *retryable* failure rather than a permanent one - the row's actor
/// requeues it, on the delay the driver named or on its own ladder, and the
/// anchor this subscription registers is there by the next pass.
#[allow(clippy::too_many_arguments)]
async fn run_anchor_subscription(
    hub: Arc<WatchHub>,
    selector: WatchSelector,
    registry: Arc<PlaneResourceRegistry>,
    store: Arc<SpecStore>,
    zone_token: BoundedToken,
    state: Arc<AnchorSubscriptionState>,
    anchor: RuntimeRevision,
    window: Duration,
) {
    // The first registration is anchored at the snapshot revision taken
    // before the manager spawned, so the initial load and the subscription
    // are paired through it (R1): the load covers everything at or before
    // its own store read, the registration replays everything after the
    // anchor, and the live stream covers the rest. Registering live-only
    // after the load would leave the window uncovered, because a no-cursor
    // registration serves no replay.
    let mut cursor = anchor;
    loop {
        cursor = anchor_subscription_phase(
            &hub,
            &selector,
            &registry,
            &store,
            &zone_token,
            &state,
            cursor,
            window,
        )
       .await;
        // Recovery (R5): the type-scoped reload rebuilds the projection
        // from the store, covering rows committed while the subscription
        // was between streams; the next phase's fresh registration then
        // replays the interval after the handed-over cursor.
        state.pending.store(false, Ordering::Relaxed);
        // A recovery counts only once its reload read every row set: a failed
        // reload leaves the rows it could not read to heal on their next
        // notice, and reporting the recovery complete would hide that.
        if reload_anchor_rows(&registry, &zone_token, &store).await {
            state.relists.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// One registration and its live phase: drains the retained replay, then
/// the live stream, coalescing Desired notices into one bounded
/// re-materialization per drain. Returns the revision the next phase must
/// relist from when the stream ends (a terminal missed-data signal or the
/// hub reaping the subscriber) or the registration is Expired.
#[allow(clippy::too_many_arguments)]
async fn anchor_subscription_phase(
    hub: &WatchHub,
    selector: &WatchSelector,
    registry: &PlaneResourceRegistry,
    store: &SpecStore,
    zone_token: &BoundedToken,
    state: &AnchorSubscriptionState,
    cursor: RuntimeRevision,
    window: Duration,
) -> RuntimeRevision {
    let (snapshot, mut stream) = match hub.register(selector.clone(), Some(cursor)).await {
        WatchRegistration::Live { snapshot, replay, stream } => {
            // Drain the retained replay before treating the subscription as
            // live: it covers the interval between the cursor and the
            // registration, and the projection must reflect it (R1).
            let mut keys: Vec<ResourceKey> = Vec::new();
            for change in replay {
                if change.source == ChangeSource::Desired
                    && selector.matches(&change.key)
                    && !keys.contains(&change.key)
                {
                    keys.push(change.key);
                }
            }
            if !keys.is_empty() {
                state.pending.store(true, Ordering::Relaxed);
                state.pending_sets.fetch_add(1, Ordering::Relaxed);
                rematerialize_anchor_rows(registry, zone_token, store, &keys).await;
                state.rematerializations.fetch_add(1, Ordering::Relaxed);
                state.pending.store(false, Ordering::Relaxed);
            }
            (snapshot, stream)
        }
        WatchRegistration::Expired(RevisionExpired { snapshot,.. }) => {
            // The cursor is unservable: relist from the handed-over
            // snapshot revision (R5).
            return snapshot;
        }
    };
    let mut keys: Vec<ResourceKey> = Vec::new();
    loop {
        match stream.recv().await {
            Some(WatchDelivery::Change(change)) => {
                if change.source == ChangeSource::Desired && selector.matches(&change.key) {
                    // A durable mutation of a covered row: set the pending
                    // flag and drain the batch for the bounded window
                    // measured from this notice (R3). A burst coalesces into
                    // one re-materialization, and sustained traffic cannot
                    // starve it.
                    state.pending.store(true, Ordering::Relaxed);
                    state.pending_sets.fetch_add(1, Ordering::Relaxed);
                    if !keys.contains(&change.key) {
                        keys.push(change.key);
                    }
                    let deadline = tokio::time::Instant::now() + window;
                    let mut busy = false;
                    loop {
                        match tokio::time::timeout_at(deadline, stream.recv()).await {
                            Ok(Some(WatchDelivery::Change(change))) => {
                                if change.source == ChangeSource::Desired
                                    && selector.matches(&change.key)
                                {
                                    busy = true;
                                    if !keys.contains(&change.key) {
                                        keys.push(change.key);
                                    }
                                }
                            }
                            Ok(Some(WatchDelivery::Missed { last_delivered })) => {
                                return last_delivered;
                            }
                            Ok(None) => return snapshot,
                            Err(_elapsed) => break,
                        }
                    }
                    if busy {
                        let consecutive =
                            state.consecutive_busy.fetch_add(1, Ordering::Relaxed) + 1;
                        if consecutive == ANCHOR_BUSY_DRAINS_BEFORE_WARN {
                            tracing::warn!(
                                zone = %zone_token.as_str(),
                                drains = consecutive,
                                "anchor projection drain window passed repeatedly while the stream stayed busy; the projection is falling behind"
                            );
                        }
                    } else {
                        state.consecutive_busy.store(0, Ordering::Relaxed);
                    }
                    // Clear the flag, re-check the stream for anything queued
                    // between the last recv and the window, then perform one
                    // re-materialization (R3).
                    state.pending.store(false, Ordering::Relaxed);
                    loop {
                        match stream.try_recv() {
                            Some(WatchDelivery::Change(change)) => {
                                if change.source == ChangeSource::Desired
                                        && selector.matches(&change.key)
                                        && !keys.contains(&change.key)
                                    {
                                        keys.push(change.key);
                                    }
                            }
                            Some(WatchDelivery::Missed { last_delivered }) => {
                                return last_delivered;
                            }
                            None => break,
                        }
                    }
                    rematerialize_anchor_rows(registry, zone_token, store, &keys).await;
                    state.rematerializations.fetch_add(1, Ordering::Relaxed);
                    keys.clear();
                }
            }
            Some(WatchDelivery::Missed { last_delivered }) => {
                // Terminal missed-data signal: the cursor cannot be served,
                // so relist from the handed-over revision (R5).
                return last_delivered;
            }
            None => {
                // The stream ended without a Missed (the hub reaped the
                // subscriber): relist from the registration's snapshot.
                return snapshot;
            }
        }
    }
}

/// Register one durable row's anchor. The only row types the projection
/// holds are Volume and VolumeBinding; every other type is not the
/// projection's to carry.
async fn register_anchor_row(
    registry: &PlaneResourceRegistry,
    zone_token: &BoundedToken,
    row: &StoredDesiredResource,
) {
    match row.key.type_name.as_str() {
        "Volume" => {
            registry
               .register_volume(
                    &resource_uid_string(&row.uid),
                    &row.key.name,
                    volume_anchor_from_row(row),
                )
               .await;
        }
        "VolumeBinding" => register_binding_row(registry, zone_token, row).await,
        _ => {}
    }
}

/// Re-materialize the anchor projection for one drain: re-register the
/// committed rows the drain collected, keyed by the committed row. This is
/// the per-commit shape - per-row registration, never the store-wide sweep.
async fn rematerialize_anchor_rows(
    registry: &PlaneResourceRegistry,
    zone_token: &BoundedToken,
    store: &SpecStore,
    keys: &[ResourceKey],
) {
    for key in keys {
        let row = match store.get(key.clone()).await {
            Ok(row) => row,
            // A row the store no longer holds is gone, not failed: the drain
            // collected a notice whose row was retired in between.
            Err(d2b_resource_runtime::spec_store::SpecStoreError::NotFound {.. }) => continue,
            Err(error) => {
                tracing::warn!(
                    zone = %zone_token.as_str(),
                    row_type = %key.type_name,
                    error = %error,
                    "anchor projection: a committed row could not be read back; its anchor stays unregistered until the next notice for that row"
                );
                continue;
            }
        };
        register_anchor_row(registry, zone_token, &row).await;
    }
}

/// Type-scoped reload of the rows the anchor projection holds (Volume and
/// VolumeBinding): the recovery, relist, and commit-path refresh shape, never
/// the store-wide sweep. The zone scope is left open because the durable load
/// the retired reload ran was zone-open too: a system-homed row a Zone plane
/// resolves against lives outside its own Zone, and a zone-scoped refresh
/// would leave that row's anchor to the subscription's delivery instead.
/// Reports whether every row set was read back, so a refresh is not counted
/// as complete while the projection is still stale.
async fn reload_anchor_rows(
    registry: &PlaneResourceRegistry,
    zone_token: &BoundedToken,
    store: &SpecStore,
) -> bool {
    let mut complete = true;
    for type_name in ["Volume", "VolumeBinding"] {
        let selector = SpecSelector {
            zone: None,
            type_name: Some(type_name.to_owned()),
            owner_uid: None,
        };
        match store.list(selector).await {
            Ok(rows) => {
                for row in &rows {
                    register_anchor_row(registry, zone_token, row).await;
                }
            }
            Err(error) => {
                complete = false;
                tracing::warn!(
                    zone = %zone_token.as_str(),
                    row_type = type_name,
                    error = %error,
                    "anchor projection reload failed; rows committed while the subscription was down stay unregistered until the next notice for them"
                );
            }
        }
    }
    complete
}

// ---------------------------------------------------------------------------
// Production socket effect closures (binding/endpoint legs)
// ---------------------------------------------------------------------------

/// Mirror of the provider's `PrivateSocketPath::derive` (frozen v1 worker
/// contract): the private virtiofs socket path for one (volume, guest)
/// serving pair. The rendered accessor is crate-private in the provider
/// today; U14 collapses this mirror behind a provider-owned probe.
pub(crate) fn serving_socket_path(
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

async fn socket_is_present(path: &Path) -> bool {
    tokio::fs::metadata(path)
       .await
       .map(|metadata| metadata.file_type().is_socket())
       .unwrap_or(false)
}

async fn remove_socket_file(path: &Path) -> Result<(), String> {
    match tokio::fs::remove_file(path).await {
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
        serving_socket_path(
            &self.socket_runtime_dir,
            &self.zone_token,
            &target.volume_ref,
            &target.execution_ref,
        )
    }
}

/// The binding family's serving-socket probe facet (U6): the daemon's
/// socket-target registry and runtime directory answer the family's
/// `socket_ready` through the derived private path.
#[async_trait::async_trait]
impl SocketReadySource for BindingSocketProbe {
    async fn ready(&self, socket: &SocketIdentity) -> bool {
        match self.path_for(socket).await {
            Some(path) => socket_is_present(&path).await,
            None => false,
        }
    }
}

/// The binding family's socket-removal facet (U6): the endpoint-first half
/// of the teardown, idempotent under retry (R10).
#[async_trait::async_trait]
impl SocketRemoveSource for BindingSocketProbe {
    async fn remove(&self, socket: &SocketIdentity) -> Result<(), String> {
        match self.path_for(socket).await {
            Some(path) => remove_socket_file(&path).await,
            // Unknown socket: nothing was realized on this target.
            None => Ok(()),
        }
    }
}

/// The binding family's guest-mount observation facet (U6): the daemon's
/// Zone target directory answers the family's drain gate through the live
/// authenticated ComponentSession (KTD6/U13) - the same boundary every
/// other target-local observation crosses.
struct PlaneGuestMountSource {
    state: Arc<crate::ServerState>,
    zone: ZoneId,
}

#[async_trait::async_trait]
impl GuestMountSource for PlaneGuestMountSource {
    async fn guest_mount_ready(&self, key: &ResourceKey) -> Result<bool, String> {
        Ok(crate::binding_guest_mount_ready(&self.state, &self.zone, key).await)
    }
}

/// Production Endpoint socket surface (transport-unix virtiofsd case):
/// the daemon's host socket facet the Endpoint family's effects service
/// drives (U6). The worker Process child binds the private socket; the
/// facet's `ensure` waits a bounded budget for the bind and reports a
/// retryable failure otherwise (the actor owns the retry, R13), and the
/// same path resolution answers the family's presence probe and the
/// endpoint-first removal. The family's own dispatch routes the evidence
/// purposes onto the evidence facets, so this surface only ever sees the
/// virtiofsd purpose; the surface still refuses any other purpose with the
/// preserved pre-move message rather than weakening the old port's refusal.
#[derive(Clone)]
struct PlaneEndpointSocketSource {
    registry: Arc<PlaneResourceRegistry>,
    socket_runtime_dir: PathBuf,
    zone_token: BoundedToken,
}

impl PlaneEndpointSocketSource {
    /// Resolve the producer's private socket path; a registry miss loads the
    /// derived-child rows from the authority (the spec store) first.
    async fn path_for(&self, producer_ref: &ResourceRef) -> Option<PathBuf> {
        let target = self
           .registry
           .socket_target_by_ref(&self.zone_token, producer_ref)
           .await?;
        serving_socket_path(
            &self.socket_runtime_dir,
            &self.zone_token,
            &target.volume_ref,
            &target.execution_ref,
        )
    }
}

#[async_trait::async_trait]
impl EndpointSocketSource for PlaneEndpointSocketSource {
    async fn present(&self, producer_ref: &ResourceRef, purpose: &str) -> bool {
        if purpose != d2b_provider_endpoint::VIRTIOFSD_PURPOSE {
            return false;
        }
        let Some(path) = self.path_for(producer_ref).await else {
            return false;
        };
        socket_is_present(&path).await
    }

    async fn ensure(&self, producer_ref: &ResourceRef, purpose: &str) -> Result<(), String> {
        if purpose != d2b_provider_endpoint::VIRTIOFSD_PURPOSE {
            return Err(format!(
                "endpoint purpose {purpose:?} is not realized by the v3 plane"
            ));
        }
        // Resolve the target once (a miss consults the authority); the poll
        // below only re-checks the bound socket on the host target.
        let path = self.path_for(producer_ref).await;
        let deadline = tokio::time::Instant::now() + SOCKET_BIND_BUDGET;
        loop {
            if let Some(path) = path.as_deref()
                && socket_is_present(path).await
            {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err("virtiofsd socket not bound within its realize budget".to_owned());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    async fn remove(&self, producer_ref: &ResourceRef, purpose: &str) -> Result<(), String> {
        if purpose != d2b_provider_endpoint::VIRTIOFSD_PURPOSE {
            return Err(format!(
                "endpoint purpose {purpose:?} is not realized by the v3 plane"
            ));
        }
        match self.path_for(producer_ref).await {
            Some(path) => remove_socket_file(&path).await,
            // Unknown producer: nothing was realized on this target.
            None => Ok(()),
        }
    }
}

/// Presence evidence for the guest-runtime control endpoints (`ch-api`,
/// `guest-control`): the daemon's guest-VMM evidence facet the Endpoint
/// family's effects service drives (U6).
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
    planes: Arc<tokio::sync::Mutex<HashMap<String, Arc<ResourcePlaneV3>>>>,
    zone: ZoneId,
}

impl GuestControlEndpointProbe {
    fn new(
        planes: Arc<tokio::sync::Mutex<HashMap<String, Arc<ResourcePlaneV3>>>>,
        zone: ZoneId,
    ) -> Self {
        Self { planes, zone }
    }
}

#[async_trait::async_trait]
impl GuestVmmEvidenceSource for GuestControlEndpointProbe {
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
        let Some(plane) = self.planes.lock().await.get(self.zone.as_str()).cloned() else {
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
/// (`swtpm-tpm-socket`, `swtpm-control-socket`): the daemon's device-worker
/// evidence facet the Endpoint family's effects service drives (U6).
///
/// One swtpm launch composes both sockets (`--server` and `--ctrl` of the same
/// argv) and the declaring Device TPM Provider's worker Process row reports
/// `Ready` exactly while that launch is live, so the producer row is the
/// evidence row - read the same way the guest-runtime control family reads its
/// VMM row. The daemon owns nothing here: the worker creates the sockets and a
/// Device delete retires them with the row.
struct DeviceWorkerEndpointProbe {
    planes: Arc<tokio::sync::Mutex<HashMap<String, Arc<ResourcePlaneV3>>>>,
    zone: ZoneId,
}

impl DeviceWorkerEndpointProbe {
    fn new(
        planes: Arc<tokio::sync::Mutex<HashMap<String, Arc<ResourcePlaneV3>>>>,
        zone: ZoneId,
    ) -> Self {
        Self { planes, zone }
    }
}

#[async_trait::async_trait]
impl DeviceWorkerEvidenceSource for DeviceWorkerEndpointProbe {
    /// Whether the producer worker row reports `Ready` at its current
    /// generation.
    async fn present(&self, producer_ref: &ResourceRef, purpose: &str) -> bool {
        if !device_worker_purpose(purpose) || producer_ref.resource_type().as_str() != "Process" {
            return false;
        }
        let Some(plane) = self.planes.lock().await.get(self.zone.as_str()).cloned() else {
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
    ) -> Result<d2b_provider_volume_local::ResolvedVolumeRoot, d2b_provider_volume_local::VolumeLocalError> {
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
        Ok(d2b_provider_volume_local::ResolvedVolumeRoot::new(file, volume_uid.clone())?
           .with_marker_root(marker_root)?
           .with_preexisting_state())
    }
}

impl d2b_provider_volume_local::VolumeRootResolver for ZoneVolumeRootResolver {
    fn resolve_root(
        &self,
        volume_uid: &ResourceUid,
        source_policy_id: Option<&BoundedToken>,
        system_artifact_id: Option<&BoundedToken>,
        kind: SourceKind,
    ) -> Result<d2b_provider_volume_local::ResolvedVolumeRoot, d2b_provider_volume_local::VolumeLocalError> {
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
        d2b_provider_volume_local::ResolvedVolumeRoot::new(file, volume_uid.clone())?
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

/// The daemon-hosted Volume runtime (U7): the reconcile and cleanup
/// orchestration the family's effects service delegates to, over the
/// anchored adapters ([`AnchoredVolumeEffectAdapter`]) and the daemon's own
/// trusted root resolver, plus the durable layout probe recover reads. The
/// controller is rebuilt over the anchored adapters per call (exactly the
/// old `reconcile_volume` construction); the anchored-fd implementation
/// itself lives in the declaring crate, so the daemon holds no volume-local
/// mutation code.
struct PlaneVolumeRuntime {
    resolver: ZoneVolumeRootResolver,
    marker_root: PathBuf,
}

impl PlaneVolumeRuntime {
    /// Rebuild the preserved `VolumeLocalController` over the anchored
    /// adapters for one call.
    fn controller(
        &self,
    ) -> VolumeLocalController<
        AnchoredVolumeEffectAdapter<ZoneVolumeRootResolver>,
        AnchoredVolumeEffectAdapter<ZoneVolumeRootResolver>,
    > {
        let source = AnchoredVolumeEffectAdapter::new(self.resolver.clone());
        let layout = AnchoredVolumeEffectAdapter::new(self.resolver.clone());
        VolumeLocalController::new(VolumeLocalProfile::shipped(), source, layout)
    }
}

#[async_trait::async_trait]
impl VolumeRuntime for PlaneVolumeRuntime {
    async fn reconcile_volume(
        &self,
        volume_uid: &ResourceUid,
        spec: &VolumeSpec,
        provider: Option<&serde_json::Value>,
        owner_ref: Option<&ResourceRef>,
    ) -> Result<bool, String> {
        let report = self
            .controller()
            .reconcile(volume_uid, spec, provider, owner_ref)
            .await
            .map_err(|error| error.to_string())?;
        Ok(report.layout_phase == d2b_provider_volume_local::LayoutPhase::Ready)
    }

    async fn cleanup_volume(
        &self,
        volume_uid: &ResourceUid,
        spec: &VolumeSpec,
    ) -> Result<(), String> {
        self.controller()
            .cleanup(volume_uid, spec)
            .await
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    fn has_layout(&self, volume_uid: &ResourceUid) -> bool {
        // Layout-state probe (old recover): the volume-local marker is the
        // durable evidence an initialized layout left behind.
        self.marker_root.join(volume_uid.as_str()).exists()
    }
}

/// The production volume facet set: the daemon-hosted [`PlaneVolumeRuntime`]
/// over the per-zone resolver and marker root, supplied to the family's
/// effects service and driver factories through the composition root (U7).
pub(crate) fn production_volume_facets(
    state: &Arc<crate::ServerState>,
    zone: ZoneId,
    resolver: BundleResolver,
    registry: Arc<PlaneResourceRegistry>,
) -> VolumeEffectFacets {
    let marker_root = state
       .daemon_state_dir
       .parent()
       .unwrap_or(state.daemon_state_dir.as_path())
       .join("volume-local-markers");
    VolumeEffectFacets {
        runtime: Arc::new(PlaneVolumeRuntime {
            resolver: ZoneVolumeRootResolver {
                state: Arc::clone(state),
                resolver,
                zone,
                marker_root: marker_root.clone(),
                registry,
            },
            marker_root,
        }),
    }
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
    /// The daemon-supplied facet set the Process family's effects
    /// implementation is built from (U1): the composed fixed providers and
    /// the committed/Guest-owner identity sources, supplied through the
    /// composition root. The family never receives a daemon-built effect
    /// port (R2).
    pub process_facets: ProcessEffectFacets,
    /// The daemon-supplied facet set the Host family's effects
    /// implementation is built from (U5): the minijail platform gate source
    /// (the daemon's own bounded kernel/cgroup posture probe), supplied
    /// through the composition root. The family never receives a
    /// daemon-built effect port (R2).
    pub host_facets: HostEffectFacets,
    /// The daemon-supplied facet set the Network family's effects
    /// implementation is built from (U14): the daemon's Network runtime and
    /// the resolved bundle intents, supplied through the composition root.
    /// The family never receives a daemon-built effect port (R2).
    pub network_facets: NetworkEffectFacets,
/// The daemon-supplied facet set the Activation family's effects
    /// implementation is built from: the broker dispatch source (the
    /// daemon's own dispatch over its authenticated origination socket,
    /// presented as the daemon's admin-uid caller authority), supplied
    /// through the composition root. The family never receives a
    /// daemon-built effect port (R2).
    pub activation_facets: ActivationEffectFacets,
    /// The daemon-supplied facet set the User family's effects implementation
    /// is built from (U5): the crate's own bounded local-account probe,
    /// supplied through the composition root. Every probe input is host
    /// state the crate reads itself, so the family never receives a
    /// daemon-built effect port (R2).
pub user_facets: UserEffectFacets,
    /// The daemon-supplied facet set the VolumeBinding family's effects
    /// implementation is built from (U6):the serving-socket probe,the
    /// socket removal,and the guest-mount observation,supplied through the
    /// composition root. The family never receives a daemon-built effect
    /// port (R2).
    pub binding_facets: BindingEffectFacets,
    /// The daemon-supplied facet set the Endpoint family's effects
    /// implementation is built from (U6):the host socket surface and the
    /// two row-evidence probes,supplied through the composition root. The
    /// family never receives a daemon-built effect port (R2).
    pub endpoint_facets: EndpointEffectFacets,
    /// The daemon-supplied facet set the Credential family's effects
    /// implementation is built from (U8):the daemon's Credential runtime
    /// (the preserved Provider and execution-target reads,the lease-facts
    /// read,the managed-identity agent probe,and the authenticated
    /// Provider session handoff registry),supplied through the composition
    /// root. The family never receives a daemon-built effect port (R2).
    pub credential_facets: CredentialEffectFacets,
    /// The daemon-supplied facet set the Volume family's effects
    /// implementation is built from (U7): the daemon's Volume runtime over
    /// the per-zone trusted root resolver and durable layout state,
    /// supplied through the composition root. The family never receives a
    /// daemon-built effect port (R2).
    pub volume_facets: VolumeEffectFacets,
    /// The daemon-supplied facet set the Guest family's effects
    /// implementation is built from (U10):the zone's manager view (live
    /// rows, committed Provider identities,and the controller-session
    /// generation)andthe Cloud Hypervisor controller session,supplied
    /// through the composition root. The family never receives a
    /// daemon-built effect port (R2).
    pub guest_facets: GuestEffectFacets,
    /// The daemon-supplied facet sets the device families' effects
    /// implementations are built from (U12): each family's driver never
    /// receives a daemon-built effect port (R2);the family crates serve
    /// their own effects over these facets.
    pub usbip_facets: d2b_provider_device_usbip::facets::UsbipEffectFacets,
    pub security_key_facets: d2b_provider_device_security_key::facets::SecurityKeyEffectFacets,
    pub device_facets: d2b_provider_device::facets::DeviceEffectFacets,
    /// The daemon-supplied facet set the interaction family's effects
    /// implementation is built from (U12): the committed interaction
    /// identity, the zone's manager-plane reads, and the broker-backed audio
    /// mediator source, supplied through the composition root. The family
    /// never receives a daemon-built effect port (R2).
    pub interaction_facets: InteractionEffectFacets,
    /// The origination-leg publication binding for this Zone's
    /// trusted-context values: the broker socket, the daemon's caller role,
    /// and the generations the Zone serves. The production constructor
    /// binds the set; a test or context-free deployment leaves it unbound
    /// and nothing is published.
    pub trusted_context_publication: Option<TrustedContextPublication>,
    /// The hosting factories the composition root registered for the
    /// services the plane's providers declare (U3, R5), keyed by service
    /// identity. The composition point applies every entry to the provider
    /// set, so a provider that declares a service is hosted; a declared
    /// service with no entry still refuses startup by name.
    pub effect_service_factories: BTreeMap<&'static str, Arc<dyn EffectServiceFactory>>,
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
        credential_runtime: Arc<dyn CredentialRuntime>,
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
        // U1: the Process family's effects ride the declared facets, and the
        // composition root hosts the family's declared effects service from
        // the same facet set the driver factories are built from.
        let process_facets = ProcessEffectFacets {
            runtime: Arc::clone(&process_providers) as Arc<dyn ProcessProviderRuntime>,
            committed: Some(
                Arc::new(PlaneCommittedProviderIdentitySource {
                    registry: registry_source,
                }) as Arc<dyn CommittedProviderIdentitySource>,
            ),
            guest_owners: Some(Arc::new(PlaneGuestOwnerIdentities {
                state: Arc::clone(state),
            })),
        };
        // U14:the Network family's effects ride the declared facets,and
        // the composition root hosts the family's declared effects service
        // from the same facet set the driver factories are built from. The
        // shared-provider adapter serves as the daemon's Network runtime
        // over the plane's trusted bundle; the composed resolver is only
        // the last-verified seed, because every invocation (the runtime
        // facet's bundle read and the kernel intent source's per-call loader)
        // reloads and re-verifies the on-disk bundle (the retired adapter's
        // per-call behaviour).
        // U12 (device families): each device family's effects ride the
        // declared facets, and the composition root hosts the family's
        // declared effects service from the same facet set the driver
        // factories are built from. The shared-provider adapter serves as
        // every family's runtime over the plane's trusted bundle, admission,
        // and broker seam; the sub-family facet sets the Device runtime
        // drives (the TPM and GPU ports, the USBIP kernel dispatcher) are
        // built here from the same adapter and attached to it (the facet
        // sets are circular with the adapter, which is their runtime).
        let shared_provider_effects = Arc::new(ProductionSharedProviderEffects::new(
            Arc::clone(state),
            zone.clone(),
            controller_generation,
            resolver.clone(),
        ));
        let usbip_facets = d2b_provider_device_usbip::facets::UsbipEffectFacets {
            runtime: Arc::clone(&shared_provider_effects)
                as Arc<dyn d2b_provider_device_usbip::facets::UsbipRuntime>,
            broker: d2b_provider_device_usbip::facets::UsbipBrokerFacets {
                dispatch: Arc::new(crate::DaemonUsbipBrokerDispatch::new(
                    Arc::clone(state),
                )),
            },
        };
        let security_key_facets =
            d2b_provider_device_security_key::facets::SecurityKeyEffectFacets {
                runtime: Arc::clone(&shared_provider_effects)
                    as Arc<
                        dyn d2b_provider_device_security_key::facets::SecurityKeyRuntime,
                    >,
            };
        let device_facets = d2b_provider_device::facets::DeviceEffectFacets {
            runtime: Arc::clone(&shared_provider_effects)
                as Arc<dyn d2b_provider_device::facets::DeviceRuntime>,
        };
        let tpm_facets = d2b_provider_device_tpm::facets::TpmEffectFacets {
            runtime: Arc::clone(&shared_provider_effects)
                as Arc<dyn d2b_provider_device_tpm::facets::TpmRuntime>,
        };
        let gpu_facets = d2b_provider_device_gpu::facets::GpuEffectFacets {
            runtime: Arc::clone(&shared_provider_effects)
                as Arc<dyn d2b_provider_device_gpu::facets::GpuRuntime>,
        };
        shared_provider_effects.attach_device_facets(
            Arc::clone(&usbip_facets.broker.dispatch),
            tpm_facets.clone(),
            gpu_facets.clone(),
        );
        let network_facets = NetworkEffectFacets {
            runtime: Arc::clone(&shared_provider_effects)
                as Arc<dyn d2b_provider_network_local::NetworkRuntime>,
        };
// U5: the Host family's effects ride the declared facets too: the
        // daemon's own minijail platform gate probe is the one daemon-owned
        // read the family's probe needs, supplied through the composition
        // root; every other probe input is host state the family crate
        // reads itself. The facet set is composed beside the gate probe it
        // wraps (in the process-provider runtime module, whose minijail
        // vocabulary is already measured there).
        let host_facets = crate::process_provider_runtime::production_host_facets();
        // The Activation family's effects ride the declared facets too: the
        // one daemon-structural capability the family needs - the dispatch
        // of the preserved `ApplyHostGenerationHandoff` broker request over
        // the daemon's authenticated origination socket, presented as the
        // daemon's admin-uid caller authority - crosses the provider
        // boundary as the declared broker dispatch facet, supplied through
        // the composition root.
        let activation_facets = ActivationEffectFacets {
            broker: Arc::new(ProductionActivationBrokerDispatch {
                state: Arc::clone(state),
            }),
        };
// U10: the Guest family's effects ride the declared facets, and the
        // composition root hosts the family's declared effects service from
        // the same facet set the driver factories are built from. The
        // manager view and the Cloud Hypervisor controller session are
        // per-zone daemon-supplied facets over `ServerState`.
        let guest_facets = GuestEffectFacets {
            zone: zone.clone(),
            controller_generation,
            manager: Arc::new(PlaneGuestManagerView {
                state: Arc::clone(state),
                zone: zone.clone(),
            }),
            cloud_hypervisor: Arc::new(PlaneCloudHypervisorGuestRuntime {
                state: Arc::clone(state),
                zone: zone.clone(),
            }),
        };
        // U12: the interaction family's effects ride the declared facets
        // too: the committed identity, the zone's manager-plane reads, and
        // the broker-backed audio mediator source all cross the provider
        // boundary as daemon-supplied facets, never as a daemon-built port
        // (R2).
        let interaction_facets = InteractionEffectFacets::new(
            zone.clone(),
            Arc::new(ProductionInteractionIdentitySource {
                state: Arc::clone(state),
                zone: zone.clone(),
            }),
            Arc::new(ProductionInteractionPlaneRead {
                state: Arc::clone(state),
                zone: zone.clone(),
            }),
Arc::new(DaemonAudioMediatorSource {
                state: Arc::clone(state),
            }),
);
        // The User family's effects build from the facet set carrying the
        // crate's own probe: the probe reads host state the crate reads
        // itself (U5), so the composition root supplies no externally built
        // port.
        let user_facets = UserEffectFacets::production();
        // U6: the VolumeBinding and Endpoint families' effects ride the
        // declared facets too: the daemon's socket-target registry, runtime
        // directory, plane table, and target directory answer the families'
        // facet traits, and the families' own implementations
        // (effects_service) build their driver effects and hosted services
        // from the same facet sets.
        let binding_facets = BindingEffectFacets {
            ready: Arc::new(probe.clone()),
            remove: Arc::new(probe),
            // U13/KTD6: the guest-mount gate reads the Zone target
            // directory for the row's assignment (the plane is resolved at
            // call time - it is registered on `state` after this provider
            // directory is built).
            guest_mount: Arc::new(PlaneGuestMountSource {
                state: Arc::clone(state),
                zone: zone.clone(),
            }),
        };
        let endpoint_facets = EndpointEffectFacets {
            socket: Arc::new(PlaneEndpointSocketSource {
                registry: Arc::clone(&registry),
                socket_runtime_dir: endpoint_socket_runtime_dir.clone(),
                zone_token: endpoint_zone_token.clone(),
            }),
            guest_vmm: Arc::new(GuestControlEndpointProbe::new(
                Arc::clone(&state.v3_planes),
                zone.clone(),
            )),
            device_worker: Arc::new(DeviceWorkerEndpointProbe::new(
                Arc::clone(&state.v3_planes),
                zone.clone(),
            )),
        };
        // U8: the Credential family's effects ride the declared facets too:
        // the daemon's Credential runtime (the preserved provider reads and
        // the ProviderSupervisor session handoff registry) is supplied
        // through the composition root, and the composition root hosts the
        // family's declared effects service from the same facet set the
        // driver factories are built from.
        let credential_facets = CredentialEffectFacets { runtime: credential_runtime };
        // U7:the Volume family's effects ride the declared facets
        // composition root hosts the family's declared effects service from
        // the same facet set the driver factories are built from. The
        // daemon's Volume runtime is the reconcile/cleanup orchestration
        // over the anchored adapters and the per-zone trusted root resolver.
        let volume_facets = production_volume_facets(
            state,
            zone.clone(),
            resolver,
            Arc::clone(&registry),
        );
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
            process_facets: process_facets.clone(),
            host_facets: host_facets.clone(),
            network_facets: network_facets.clone(),
            user_facets: user_facets.clone(),
            binding_facets: binding_facets.clone(),
            endpoint_facets: endpoint_facets.clone(),
            volume_facets: volume_facets.clone(),
            activation_facets: activation_facets.clone(),
            credential_facets: credential_facets.clone(),
            guest_facets: guest_facets.clone(),
            usbip_facets: usbip_facets.clone(),
            security_key_facets: security_key_facets.clone(),
            device_facets: device_facets.clone(),
            interaction_facets: interaction_facets.clone(),
            trusted_context_publication: Some(
                crate::provider_lifecycle::TrustedContextPublication::production(
                    process_providers.mode(),
                    broker_socket,
                    state.daemon_uid,
                    controller_generation.get(),
                ),
            ),
            // The registered families' declared effects services are hosted
            // from the families' own implementations over this zone's facet
            // sets, one entry per service the registration table declares.
            effect_service_factories: registered_service_factories(
                &process_facets,
                &network_facets,
                &host_facets,
                &guest_facets,
                &binding_facets,
                &endpoint_facets,
                &activation_facets,
                &interaction_facets,
                &user_facets,
                &usbip_facets,
                &security_key_facets,
                &device_facets,
                &credential_facets,
                &volume_facets,
            ),
            foundation: None,
        })
    }
}

/// The hosting factories the composition root registers for the services
/// the registration table declares (U3, R5): one entry per declared service
/// identity, built from the families' own implementations over this zone's
/// facet sets. A declared service with no entry still refuses startup by
/// name.
#[allow(clippy::too_many_arguments)]
fn registered_service_factories(
    process_facets: &ProcessEffectFacets,
    network_facets: &NetworkEffectFacets,
    host_facets: &HostEffectFacets,
    guest_facets: &GuestEffectFacets,
    binding_facets: &BindingEffectFacets,
    endpoint_facets: &EndpointEffectFacets,
    activation_facets: &ActivationEffectFacets,
    interaction_facets: &InteractionEffectFacets,
    user_facets: &UserEffectFacets,
    usbip_facets: &d2b_provider_device_usbip::facets::UsbipEffectFacets,
    security_key_facets: &d2b_provider_device_security_key::facets::SecurityKeyEffectFacets,
    device_facets: &d2b_provider_device::facets::DeviceEffectFacets,
    credential_facets: &CredentialEffectFacets,
    volume_facets: &VolumeEffectFacets,
) -> BTreeMap<&'static str, Arc<dyn EffectServiceFactory>> {
    let mut factories = BTreeMap::new();
    for registration in PROVIDER_REGISTRATIONS {
        for &service in registration.services {
            let factory = if service == PROCESS_EFFECTS_SERVICE.id {
                Arc::new(ProcessEffectsServiceFactory::new(process_facets.clone()))
                    as Arc<dyn EffectServiceFactory>
            } else if service == NETWORK_EFFECTS_SERVICE.id {
                Arc::new(NetworkEffectsServiceFactory::new(network_facets.clone()))
                    as Arc<dyn EffectServiceFactory>
} else if service == HOST_EFFECTS_SERVICE.id {
                Arc::new(HostEffectsServiceFactory::new(host_facets.clone()))
as Arc<dyn EffectServiceFactory>
            } else if service == ACTIVATION_EFFECTS_SERVICE.id {
                Arc::new(ActivationEffectsServiceFactory::new(activation_facets.clone()))
            } else if service == USER_EFFECTS_SERVICE.id {
                Arc::new(UserEffectsServiceFactory::new(user_facets.clone()))
            } else if service == USBIP_EFFECTS_SERVICE.id {
                Arc::new(d2b_provider_device_usbip::effects_service::
                    UsbipEffectsServiceFactory::new(usbip_facets.clone()))
                    as Arc<dyn EffectServiceFactory>
            } else if service == SECURITY_KEY_EFFECTS_SERVICE.id {
                Arc::new(d2b_provider_device_security_key::effects_service::
                    SecurityKeyEffectsServiceFactory::new(security_key_facets.clone()))
                    as Arc<dyn EffectServiceFactory>
            } else if service == DEVICE_EFFECTS_SERVICE.id {
                Arc::new(d2b_provider_device::effects_service::
                    DeviceEffectsServiceFactory::new(device_facets.clone()))
                    as Arc<dyn EffectServiceFactory>
            } else if service == CREDENTIAL_EFFECTS_SERVICE.id {
                Arc::new(CredentialEffectsServiceFactory::new(credential_facets.clone()))
                    as Arc<dyn EffectServiceFactory>
            } else if service == VOLUME_EFFECTS_SERVICE.id {
                Arc::new(VolumeEffectsServiceFactory::new(volume_facets.clone()))
                    as Arc<dyn EffectServiceFactory>
            } else if service == d2b_provider_wayland_policy::INTERACTION_EFFECTS_SERVICE.id {
                Arc::new(
                    d2b_provider_wayland_policy::InteractionEffectsServiceFactory::new(
                        interaction_facets.clone(),
                    ),
                ) as Arc<dyn EffectServiceFactory>
            } else if service == PROCESS_SYSTEMD_EFFECTS_SERVICE.id {
                // U15:the family's service carries no facet set (R2), so
                // the composition root hosts its factory from crate-owned
                // constants alone, over the registered service identity - the
                // family itself is never named here.

                Arc::new(SystemdEffectsServiceFactory::new()) as Arc<dyn EffectServiceFactory>
            } else if service == GUEST_EFFECTS_SERVICE.id {
                Arc::new(GuestEffectsServiceFactory::new(guest_facets.clone()))
                    as Arc<dyn EffectServiceFactory>
            } else if service == BINDING_EFFECTS_SERVICE.id {
                Arc::new(BindingEffectsServiceFactory::new(binding_facets.clone()))
                    as Arc<dyn EffectServiceFactory>
            } else if service == ENDPOINT_EFFECTS_SERVICE.id {
                Arc::new(EndpointEffectsServiceFactory::new(endpoint_facets.clone()))
                    as Arc<dyn EffectServiceFactory>
            } else {
                continue;
            };
            factories.insert(service, factory);
        }
    }
    factories
}

/// Production `ActivationBrokerDispatch`: the daemon's own dispatch over its
/// authenticated origination socket, presented as the daemon's admin-uid
/// caller authority - the same authority the retired daemon adapter
/// presented. The family crate receives the dispatch result and never calls
/// a daemon function or reads a daemon path.
struct ProductionActivationBrokerDispatch {
    state: Arc<crate::ServerState>,
}

impl d2b_provider_activation_nixos::ActivationBrokerDispatch
    for ProductionActivationBrokerDispatch
{
    fn dispatch_handoff(
        &self,
        request: d2b_contracts_broker::host_generation::ApplyHostGenerationHandoff,
    ) -> Result<d2b_contracts_broker::broker_wire::ApplyHostGenerationHandoffResponse, String> {
        match crate::dispatch_broker_request_as(
            &self.state,
            BrokerRequest::ApplyHostGenerationHandoff(request),
            BrokerCallerRole::AdminUid {
                uid: self.state.daemon_uid,
            },
        ) {
            Ok(BrokerResponse::ApplyHostGenerationHandoff(response)) => Ok(response),
            Ok(BrokerResponse::Error(_)) | Ok(_) => {
                Err("activation-handoff-unexpected-response".to_owned())
            }
            Err(error) => Err(format!("{}: {}", error.kind(), error.message())),
        }
    }
}
/// Production `InteractionIdentitySource` (U12): the daemon's committed
/// interaction identity, resolved through the old plane's Zone runtime at
/// call time exactly as the retired effects resolved it.
struct ProductionInteractionIdentitySource {
    state: Arc<crate::ServerState>,
    zone: ZoneId,
}

#[async_trait::async_trait]
impl InteractionIdentitySource for ProductionInteractionIdentitySource {
    async fn identity(&self) -> Option<d2b_provider_wayland_policy::InteractionEffectIdentity> {
        let runtime = self
            .state
            .resource_plane
            .try_lock()
            .ok()?
            .as_ref()?
            .zone(&self.zone)
            .ok()?;
        let identity = runtime.interaction_identity()?;
        Some(d2b_provider_wayland_policy::InteractionEffectIdentity {
            wayland_session_ref: identity.wayland_session_ref().clone(),
            wayland_session_uid: identity.wayland_session_uid().clone(),
            subject_ref: identity.subject_ref().clone(),
            host_execution_ref: identity.host_execution_ref().clone(),
            user_ref: identity.user_ref().clone(),
        })
    }
}

/// Production `InteractionPlaneRead` (U12): the zone's v3 manager-plane row
/// reads, reached through the old plane's Zone runtime exactly as the
/// retired effects reached them.
struct ProductionInteractionPlaneRead {
    state: Arc<crate::ServerState>,
    zone: ZoneId,
}

impl ProductionInteractionPlaneRead {
    fn plane(&self) -> Result<Arc<crate::resource_plane_v3::ResourcePlaneV3>, ()> {
        self.state
            .resource_plane
            .try_lock()
            .map_err(|_| ())?
            .as_ref()
            .ok_or(())?
            .zone(&self.zone)
            .map_err(|_| ())?
            .v3_plane()
            .map_err(|_| ())
    }
}

#[async_trait::async_trait]
impl InteractionPlaneRead for ProductionInteractionPlaneRead {
    async fn get(
        &self,
        key: &ResourceKey,
    ) -> Result<Option<ResourceView>, ()> {
        self.plane()?.client().get(key.clone()).await.map_err(|_| ())
    }

    async fn list(&self, selector: &ResourceSelector) -> Result<Vec<ResourceView>, ()> {
        self.plane()?
            .client()
            .list(selector.clone())
            .await
            .map_err(|_| ())
    }
}

/// Production `AudioMediatorSource` (U12): the daemon's broker-backed audio
/// mediator, built from the target's capability row exactly as the retired
/// registry built it.
struct DaemonAudioMediatorSource {
    state: Arc<crate::ServerState>,
}

impl AudioMediatorSource for DaemonAudioMediatorSource {
    fn build(&self, vm_name: &str, projection: bool) -> Option<Box<dyn AudioMediator>> {
        let manifest = crate::load_json::<d2b_core::manifest_v04::ManifestV04>(
            &self.state.config.artifacts.public_manifest_path,
        )
        .ok()?;
        let mut capability = manifest
            .vms
            .get(vm_name)
            .and_then(crate::audio_dispatch::audio_capability_for_vm)?;
        if projection {
            capability.host_enforcement =
                d2b_core::provider_capabilities::AudioHostEnforcementKind::None;
        }
        Some(Box::new(crate::audio_dispatch::DaemonAudioMediator::new(
            &self.state,
            vm_name,
            capability,
            d2b_contracts_broker::broker_wire::BrokerCallerRole::AdminUid {
                uid: self.state.daemon_uid,
            },
        )))
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
           .await
           .as_ref()
           .and_then(|plane| plane.zone(zone).ok())?;
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

/// Production `GuestManagerView` (U10): the zone's v3 plane manager view,
/// committed Provider identities, and live controller-session generation,
/// over `ServerState`.
struct PlaneGuestManagerView {
    state: Arc<crate::ServerState>,
    zone: ZoneId,
}

impl PlaneGuestManagerView {
    /// The zone runtime, resolved the same way the retired daemon effect
    /// resolved it: non-blocking `try_lock` per plan U4; a collision
    /// reports unavailable (fail-closed), never a stall.
    fn runtime(&self) -> Result<Arc<crate::resource_runtime::ZoneResourceRuntime>, ()> {
        self.state
            .resource_plane
            .try_lock()
            .ok()
            .and_then(|plane| plane.as_ref().and_then(|plane| plane.zone(&self.zone).ok()))
            .ok_or(())
    }

    /// The published v3 plane (manager rows and their live status).
    fn plane(&self) -> Result<Arc<ResourcePlaneV3>, ()> {
        self.runtime()?
            .v3_plane()
            .map_err(|_| ())
    }
}

#[async_trait::async_trait]
impl d2b_provider_guest::GuestManagerView for PlaneGuestManagerView {
    async fn row_view(
        &self,
        key: &d2b_resource_runtime::identity::ResourceKey,
    ) -> Result<Option<d2b_resource_runtime::manager::ResourceView>, ()> {
        let plane = self.plane()?;
        plane.client().get(key.clone()).await.map_err(|_| ())
    }

    fn committed_provider_identity(
        &self,
        provider_ref: &ResourceRef,
    ) -> Result<Option<(ResourceUid, ResourceGeneration)>, ()> {
        let plane = self.plane()?;
        Ok(plane.registry().committed_provider_identity(provider_ref))
    }

    fn controller_session_generation(
        &self,
    ) -> Result<
        Option<d2b_contracts_resource::v3::identity::ReconnectGeneration>,
        (),
    > {
        Ok(self.runtime()?.controller_session_generation())
    }
}

/// Production `CloudHypervisorGuestRuntime` (U10): the daemon's controller
/// session for one zone - target-session establishment and the
/// controller-owned reconcile of one Cloud Hypervisor Guest - over
/// `ServerState`.
struct PlaneCloudHypervisorGuestRuntime {
    state: Arc<crate::ServerState>,
    zone: ZoneId,
}

#[async_trait::async_trait]
impl d2b_provider_guest::CloudHypervisorGuestRuntime for PlaneCloudHypervisorGuestRuntime {
    async fn ensure_target_session(&self, guest_ref: &ResourceRef) -> Result<(), String> {
        crate::ensure_guest_target_session(&self.state, &self.zone, guest_ref).await
    }

    async fn reconcile_guest(
        &self,
        guest_ref: &ResourceRef,
        status_sink: Option<d2b_provider_guest::GuestStatusSink>,
    ) -> Result<d2b_provider_guest::GuestCloudHypervisorOutcome, String> {
        let runtime = self
            .state
            .resource_plane
            .try_lock()
            .ok()
            .and_then(|plane| plane.as_ref().and_then(|plane| plane.zone(&self.zone).ok()))
            .ok_or_else(|| "guest-effect:zone-runtime-unavailable".to_owned())?;
        let outcome = runtime
            .reconcile_cloud_hypervisor_guest_with_status(
                Arc::clone(&self.state),
                guest_ref,
                status_sink,
            )
            .await
            .map_err(|error| error.to_string())?;
        Ok(match outcome {
            crate::resource_runtime::CloudHypervisorReconcileOutcome::Ready => {
                d2b_provider_guest::GuestCloudHypervisorOutcome::Ready
            }
            crate::resource_runtime::CloudHypervisorReconcileOutcome::Pending => {
                d2b_provider_guest::GuestCloudHypervisorOutcome::Pending
            }
        })
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
    /// attach, which registers the drivers it declared. The registered
    /// families start through the generated registration table first, in
    /// declaration order; the remaining families are wired below in the
    /// order the registry has been assembled in since the family moves
    /// landed.
    fn provider_set(inputs: &ConstructionInputs) -> ProviderSet {
        // The registered families start through the generated registration
        // table: the composition root composes each row's provider identity
        // and declared services from the table instead of naming the family,
        // so a new family's registration needs no edit here. The drivers the
        // daemon wires for the families it carries are the family's own
        // implementation (the crate reference and family id are the
        // dependency itself); a registered family the daemon does not carry
        // yet starts with no drivers until its lane wires them.
        let mut set = ProviderSet::new(inputs.zone.clone(), inputs.spec_store_dir.clone());
        for registration in PROVIDER_REGISTRATIONS {
            set = set.with(
                family_declaration(registration.provider_ref),
                Self::registered_drivers(registration, inputs),
            );
        }

        // The trusted-context publication rides the set: when production
        // bound one, the rendezvous publishes this Zone's attestation
        // values over the origination leg the moment the set is published.
        set = set.with_trusted_context_publication(inputs.trusted_context_publication.clone());
        // The composition root's registered service factories ride the set
        // too (U3, R5): a provider that declares a service is hosted behind
        // its factory, and a declared service with no registered factory
        // still refuses startup by name.
        set = set.with_effect_service_factories(&inputs.effect_service_factories);
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
// The Guest family starts through the generated registration table
        // above (its row carries the family's declared effects service); the
        // descriptor construction lives in `registered_drivers` beside the
        // other registered families.
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
        // The six interaction types start through the generated registration
        // table (U12): each type's row names its provider identity, and the
        // drivers are wired in `registered_drivers` from the family's own
        // descriptor construction. No daemon table names the family.
        set
    }

    /// The drivers the daemon wires for one registered family: the family's
    /// own descriptor construction over the daemon-supplied facets (the
    /// crate reference and family id are the dependency itself). A
    /// registered family the daemon does not carry yet registers with no
    /// drivers, so a new family's registration needs no edit here.
    fn registered_drivers(
        registration: &ProviderRegistration,
        inputs: &ConstructionInputs,
    ) -> Vec<DriverDescriptor> {
        match registration.provider_ref {
            // The Process family: one descriptor per member type, both over
            // the family's shared decoder and factory. The family's verbs,
            // execution domains, exportability, and reads travel on the
            // descriptor.
            "process" => Vec::from(process_family_descriptors(ProcessDriverArgs {
                zone: inputs.zone.clone(),
                facets: inputs.process_facets.clone(),
                zone_uid: inputs.authority.zone_uid.clone(),
                policy_revision: inputs.authority.policy_revision,
                provider_assignment_generation: inputs.authority.provider_assignment_generation,
                controller_generation: inputs.authority.controller_generation,
                guest_execution: inputs.authority.guest_execution.clone(),
                mode: crate::process_provider_runtime::execution_mode(inputs.authority.mode),
            })),
            // The Network family: the driver builds its effects from the
            // declared facets; no externally built port appears here (R2).
            "network-local" => vec![network_descriptor(NetworkDriverArgs {
                zone: inputs.zone.as_str().to_owned(),
                controller_generation: inputs.authority.controller_generation,
                facets: inputs.network_facets.clone(),
            })],
// The Host family (U5): the driver builds its effects from the
            // daemon-supplied facet set; no externally built port appears at
            // this construction site (R2).
            "host" => vec![host_descriptor(inputs.host_facets.clone())],
// The Activation family: the driver builds its effects from the
            // daemon-supplied facet set; no externally built port appears at
            // this construction site (R2).
            "activation-nixos" => vec![activation_descriptor(ActivationDriverArgs {
                zone: inputs.zone.as_str().to_owned(),
                facets: inputs.activation_facets.clone(),
            })],
            // The six interaction types (U12): each type's driver is built
            // over the family's shared effects value (the family's own
            // implementation from the declared facets), so the six types
            // reconcile one per-zone controller state. The session and
            // binding behaviors are the crates' own child-intent sources.
            "wayland-policy" => vec![wayland_policy_descriptor(interaction_driver_args(
                inputs,
                WaylandPolicy,
            ))],
            "wayland-session" => {
                vec![wayland_session_descriptor(interaction_driver_args(
                    inputs,
                    WaylandSession::default(),
                ))]
            }
            "audio-service" => vec![audio_service_descriptor(interaction_driver_args(
                inputs,
                AudioService,
            ))],
            "audio-binding" => {
                vec![audio_binding_descriptor(interaction_driver_args(
                    inputs,
                    AudioBinding::default(),
                ))]
            }
            "shell-pool" => vec![shell_pool_descriptor(interaction_driver_args(
                inputs,
                ShellPool,
            ))],
            "shell-session" => vec![shell_session_descriptor(interaction_driver_args(
                inputs,
                ShellSession,
            ))],
            // The User family: one descriptor over the family's shared
            // decoder and factory, whose effects come from the crate's own
            // implementation over the daemon-supplied facet set (U5); no
            // externally built port appears here (R2).
            "user" => vec![user_descriptor(inputs.user_facets.clone())],
            // The Guest family: the descriptor builds its effects from the
            // declared facets; no externally built port appears here (R2).
            "guest" => vec![guest_descriptor(GuestDriverArgs {
                zone: inputs.zone.as_str().to_owned(),
                controller_generation: inputs.authority.controller_generation,
                facets: inputs.guest_facets.clone(),
            })],
            // U12 (device families): each device family's driver builds its
            // effects from the declared facets; no externally built port
            // appears here (R2). The Device and USBIP families serve the
            // two USB and two security-key types through their own
            // declarations.
            "device-usbip" => Vec::from(usbip_descriptors(UsbipDriverArgs {
                zone: inputs.zone.as_str().to_owned(),
                controller_generation: inputs.authority.controller_generation,
                facets: inputs.usbip_facets.clone(),
            })),
            "device-security-key" => Vec::from(security_key_descriptors(SecurityKeyDriverArgs {
                zone: inputs.zone.as_str().to_owned(),
                controller_generation: inputs.authority.controller_generation,
                facets: inputs.security_key_facets.clone(),
            })),
            "device" => vec![device_descriptor(DeviceDriverArgs {
                zone: inputs.zone.as_str().to_owned(),
                controller_generation: inputs.authority.controller_generation,
                facets: inputs.device_facets.clone(),
            })],
            // The VolumeBinding family (U6): the driver builds its effects
            // from the daemon-supplied facet set; no externally built port
            // appears at this construction site (R2).
            "volume-binding" => vec![binding_descriptor(BindingDriverArgs {
                zone: inputs.zone.as_str().to_owned(),
                facets: inputs.binding_facets.clone(),
                vcpu_count: inputs.authority.vcpu_count,
            })],
            // The Endpoint family (U6): the driver builds its effects from
            // the daemon-supplied facet set; no externally built port
            // appears at this construction site (R2).
            "endpoint" => vec![endpoint_descriptor(EndpointDriverArgs {
                zone: inputs.zone.as_str().to_owned(),
                facets: inputs.endpoint_facets.clone(),
            })],
            // The Credential family (U8): the driver builds its effects from
            // the daemon-supplied facet set; no externally built port
            // appears at this construction site (R2).
            "credential" => vec![credential_descriptor(CredentialDriverArgs {
                zone: inputs.zone.as_str().to_owned(),
                controller_generation: inputs.authority.controller_generation,
                facets: inputs.credential_facets.clone(),
            })],
            // The Volume family (U7): the driver builds its effects from the
            // declared facets; no externally built port appears here (R2).
            "volume" => vec![volume_descriptor(VolumeDriverArgs {
                zone: inputs.zone.as_str().to_owned(),
                facets: inputs.volume_facets.clone(),
            })],
            _ => Vec::new(),
        }
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
        // Stage 1: durable spec store. The directory create is async
        // (`tokio::fs`);the SQLite open + migration has no async form and runs
        // once on the daemon's reused bounded loader seat (plan KTD2: zero
        // new seats;d2bd already drives bundle resolution on the same
        // shipped bounded worker). A saturated seat refuses the plane start
        // with a named Authority error instead of parking the worker.
        let store_path = Self::spec_store_path(&inputs.spec_store_dir);
        if let Some(parent) = store_path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|error| {
                PlaneError::Authority(format!("spec store dir create failed: {error}"))
            })?;
        }
        let store = Arc::new(
            d2b_core::loader_worker::run(move || {
                SpecStore::open(store_path.clone()).map_err(PlaneError::from)
            })
           .await
           .map_err(|error| {
                PlaneError::Authority(format!("spec store open refused: {error:?}"))
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
               .register_committed_provider_identity(provider_ref, uid.clone(), *generation).await;
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
        // The anchor projection subscription's first registration is paired
        // with the initial load through this snapshot revision (R1): nothing
        // publishes before the manager spawns, so the anchor is the empty
        // cursor, and the load covers everything at or before its read while
        // the registration replays everything after the anchor.
        let anchor_revision = hub.snapshot_revision();
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
        // The anchor projection subscription: one long-lived consumer of the
        // manager's durable-change stream for Volume and VolumeBinding rows,
        // additive on the write side. The registry's store is attached above,
        // before the manager spawns, so the task can rebuild the projection
        // from it. A plane restart spawns a fresh subscription with a fresh
        // anchor, which is the restart path; this handle is dropped.
        let _anchor_subscription = spawn_anchor_subscription(
            Arc::clone(&hub),
            Arc::clone(&inputs.registry),
            Arc::clone(&store),
            inputs.zone_token.clone(),
            anchor_revision,
            ANCHOR_DRAIN_WINDOW,
            Arc::new(AnchorSubscriptionState::default()),
        );
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

    /// The zone's manager endpoint: the surface the service driver context
    /// reads resource state through (U3, R7). The composition point wires
    /// it into the rendezvous alongside the kernel seam.
    pub(crate) fn manager_endpoint(&self) -> Arc<dyn ManagerEndpoint> {
        Arc::new(ManagerActorEndpoint::new(self.client.actor().clone()))
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


    /// The per-zone registry the production effects resolve per-resource
    /// anchors from.
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
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
use d2b_provider_system_core::MinijailPlatformGate;
    use d2b_contracts_resource::v3::ResourceName;
    use d2b_contracts_zone_session::v3::resource_bundle::BundleResourceMetadata;
    use d2b_process_conformance::ProcessIdentityDigest;
    use d2b_provider_system_core::UserIdentityDigest;
    use d2b_resource_runtime::revision::ManualClock;
    use d2b_resource_runtime::watch::{ChangeKind, ChangeNotice, WatchHubConfig};

    /// One machinery-test rig for the anchor projection subscription: a
    /// small hub (so Missed and Expired are reachable), a store, and a
    /// registry the subscription rebuilds. The `_dir` keeps the SQLite
    /// store alive for the rig's lifetime.
    struct AnchorSubscriptionRig {
        _dir: tempfile::TempDir,
        hub: Arc<WatchHub>,
        store: Arc<SpecStore>,
        registry: Arc<PlaneResourceRegistry>,
        zone_token: BoundedToken,
    }

    fn anchor_subscription_rig() -> AnchorSubscriptionRig {
        anchor_subscription_rig_with(WatchHubConfig {
            ring_capacity: 32,
            delivery_buffer: 16,
        })
    }

    fn anchor_subscription_rig_with(config: WatchHubConfig) -> AnchorSubscriptionRig {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Arc::new(
            SpecStore::open(dir.path().join("spec-store.sqlite3")).expect("store"),
        );
        let registry = Arc::new(PlaneResourceRegistry::new());
        registry.attach_store(Arc::clone(&store));
        let hub = Arc::new(WatchHub::with_config(&ManualClock::at(1_000), config));
        let zone_token = BoundedToken::parse("test".to_owned()).expect("token");
        AnchorSubscriptionRig {
            _dir: dir,
            hub,
            store,
            registry,
            zone_token,
        }
    }

    /// One committed Volume row the subscription's per-row registration reads.
    async fn commit_volume_row(store: &SpecStore, key: &ResourceKey) {
        store
           .ensure(StoredDesiredResource {
                key: key.clone(),
                uid: d2b_resource_runtime::manager::deterministic_uid(key),
                generation: 1,
                owner_uid: None,
                provenance: d2b_resource_runtime::identity::ResourceProvenance::Api,
                deleting: false,
                spec: serde_json::to_vec(&serde_json::json!({"providerRef": "Provider/volume-local"}))
                   .expect("volume spec"),
                metadata: br#"{"annotations":{},"labels":{},"ownerRef":null}"#.to_vec(),
                created_at: 0,
            })
           .await
           .expect("volume row committed");
    }

    /// The binding spec the binding row's socket identity derives from.
    fn binding_spec() -> serde_json::Value {
        serde_json::json!({
            "volumeRef": "Volume/state",
            "executionRef": "Guest/acceptance-guest",
            "view": "controller",
            "access": "read-only",
            "mountPath": "/state",
        })
    }

    /// One committed VolumeBinding row, as the Volume driver mints it
    /// (the serving Provider reference rides in the stored envelope).
    async fn commit_binding_row(store: &SpecStore, key: &ResourceKey) {
        let mut envelope = binding_spec().as_object().cloned().expect("object");
        envelope.insert(
            "providerRef".to_owned(),
            serde_json::Value::String("Provider/volume-virtiofs".to_owned()),
        );
        store
           .ensure(StoredDesiredResource {
                key: key.clone(),
                uid: d2b_resource_runtime::manager::deterministic_uid(key),
                generation: 1,
                owner_uid: Some([0x11; 16]),
                provenance: d2b_resource_runtime::identity::ResourceProvenance::Resource,
                deleting: false,
                spec: serde_json::to_vec(&envelope).expect("envelope"),
                metadata: Vec::new(),
                created_at: 0,
            })
           .await
           .expect("binding row committed");
    }

    /// Poll `condition` until it holds or the bounded wait elapses.
    async fn wait_for(mut condition: impl FnMut() -> bool) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while !condition() {
            assert!(
                tokio::time::Instant::now() < deadline,
                "condition not met within the bounded wait"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// A test subscriber writer that appends every formatted record to a
    /// shared buffer, so a test can assert a stall line was logged.
    #[derive(Clone)]
    struct CapturedWriter(Arc<std::sync::Mutex<Vec<u8>>>);

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for CapturedWriter {
        type Writer = CapturedWriter;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    /// Spawn the anchor projection subscription loop for a machinery test.
    fn spawn_subscription(
        rig: &AnchorSubscriptionRig,
        state: &Arc<AnchorSubscriptionState>,
        anchor: RuntimeRevision,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(run_anchor_subscription(
            Arc::clone(&rig.hub),
            anchor_projection_selector(),
            Arc::clone(&rig.registry),
            Arc::clone(&rig.store),
            rig.zone_token.clone(),
            Arc::clone(state),
            anchor,
            ANCHOR_DRAIN_WINDOW,
        ))
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    impl std::io::Write for CapturedWriter {

        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().expect("capture lock").extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

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

    fn test_inputs() -> (tempfile::TempDir, ConstructionInputs, Arc<NewPlaneReadinessState>) {
        // U12: the plane tests build the interaction family's facet set from
        // the scripted sources, exactly as the production composition root
        // builds it from the daemon's.
        test_inputs_with_interaction_facets(
            d2b_provider_wayland_policy::test_support::scripted_facets(
                ZoneId::parse("test").unwrap(),
            ),
        )
    }

    /// The plane inputs over a caller-chosen interaction facet set, so a
    /// test can seed the family's audio registry through the same facets the
    /// plane hosts the declared service from.
    fn test_inputs_with_interaction_facets(
        interaction_facets: d2b_provider_wayland_policy::InteractionEffectFacets,
    ) -> (tempfile::TempDir, ConstructionInputs, Arc<NewPlaneReadinessState>) {
        let dir = tempfile::tempdir().expect("tempdir");
        let spec_store_dir = dir.path().join("daemon-state/zones/test");
        let readiness = Arc::new(NewPlaneReadinessState::new());
        let process_facets = {
            let effects = Arc::new(
                d2b_provider_process::test_support::FakeFacets::new(Default::default()),
            );
            // The old plane fake reported no retained identity
            // (has_active false); the shared double's default reports
            // one (active true), so script it back so the launch path
            // (and only it) is what the plane tests observe.
            effects.set_active(false);
            effects.facet_set()
        };
        let network_facets = d2b_provider_network_local::test_support::recording_facets(
            Arc::new(d2b_provider_network_local::test_support::RecordingRuntime::default()),
        );
// U5: the plane tests build the Host family's facet set from the
        // scripted minijail gate double, exactly as the production
        // composition root builds it from the daemon's gate probe.
        let host_facets = d2b_provider_host::test_support::recording_facets(
            d2b_provider_host::test_support::RecordingMinijailGate::new(
                MinijailPlatformGate::new(6, 9, true),
            ),
        );
        let volume_facets = d2b_provider_volume::test_support::recording_facets(
            d2b_provider_volume::test_support::RecordingRuntime::new(),
        );
        // The plane tests build the Activation family's facet set from the
        // scripted broker dispatch double, exactly as the production
        // composition root builds it from the daemon's dispatch.
        let activation_facets = d2b_provider_activation_nixos::test_support::recording_facets(
            d2b_provider_activation_nixos::test_support::RecordingBrokerDispatch::new(),
        );
        let user_facets = d2b_provider_user::test_support::recording_facets(
            d2b_provider_user::test_support::ScriptedProbe::new(),
        );
        // U10: the plane tests build the Guest family's facet set from the
        // scripted facets double, exactly as the production composition
        // root builds it from the daemon's runtime.
        let guest_facets = d2b_provider_guest::test_support::ScriptedFacets::new().facet_set();
        // U1/U5/U10/U14: the plane hosts the Process, Network, Host,
        // Activation, and Guest families' declared effects services from the
        // same facet sets their driver factories are built from, exactly as
        // the production composition root does.
        // U12 (device families): the plane tests build each device
        // family's facet set from the recording runtime, exactly as
        // the production composition root builds it from the
        // daemon's runtime.
        let usbip_facets = d2b_provider_device_usbip::test_support::recording_facets(
            Arc::new(d2b_provider_device_usbip::test_support::RecordingRuntime::default()),
        );
        let security_key_facets = d2b_provider_device_security_key::test_support::recording_facets(
            Arc::new(d2b_provider_device_security_key::test_support::RecordingRuntime::default()),
        );
        let device_facets = d2b_provider_device::test_support::recording_facets(
            Arc::new(d2b_provider_device::test_support::RecordingRuntime::default()),
        );
        // U6: the plane tests build the VolumeBinding and Endpoint families'
        // facet sets from the scripted doubles, exactly as the production
        // composition root builds them from the daemon's registry, plane
        // table, and target directory.
        let binding_facets = {
            let effects =
                d2b_provider_volume_binding::test_support::FakeServingEffects::new();
            // The old plane fake reported the serving socket present
            // (socket_ready true); the shared double starts absent.
            effects.make_ready();
            effects.facet_set()
        };
        let endpoint_facets = {
            let effects = d2b_provider_endpoint::test_support::FakeSocketEffects::new();
            // The old plane fake reported the socket present
            // (socket_present true); the shared double starts absent.
            effects.make_present();
            effects.facet_set()
        };
        let credential_facets = {
            let runtime = d2b_provider_credential::test_support::RecordingRuntime::new(
                d2b_provider_credential::test_support::log(),
            );
            // The old plane fake answered no provider/execution facts, no
            // live agent, and no bound session; the shared double's defaults
            // differ, so script them back.
            runtime.set_facts(None);
            runtime.set_agent_ready(false);
            runtime.set_session(None);
            d2b_provider_credential::test_support::recording_facets(runtime)
        };
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
                process_facets: process_facets.clone(),
host_facets: host_facets.clone(),
                // U7: the plane tests build the Volume family's facet set
                // from the recording runtime, exactly as the production
                // composition root builds it from the daemon's runtime.
                volume_facets: volume_facets.clone(),
                binding_facets: binding_facets.clone(),
                endpoint_facets: endpoint_facets.clone(),
                activation_facets: activation_facets.clone(),
                usbip_facets: usbip_facets.clone(),
                security_key_facets: security_key_facets.clone(),
                device_facets: device_facets.clone(),
                // U8:the plane tests build the Credential family's facet set
                // from the recording runtime, exactly as the production
                // composition root builds it from the daemon's runtime.
                credential_facets: credential_facets.clone(),
                // U14: the plane tests build the Network family's facet set
                // from the recording runtime, exactly as the production
                // composition root builds it from the daemon's runtime.
                network_facets: network_facets.clone(),
user_facets: user_facets.clone(),
                guest_facets: guest_facets.clone(),
                interaction_facets: interaction_facets.clone(),
                trusted_context_publication: None,
// U1/U14/U5/U7: the plane hosts the Process, Network, Host,
                // Activation, and Volume families' declared effects services
// U1/U14/U5/U8: the plane hosts the Process, Network, Host,
                // Activation, and Credential families' declared effects
                // services from the same facet sets their driver factories
                // are built from, exactly as the production composition
                // root does.
                // U1/U14/U5/U6: the plane hosts the Process, Network, Host,
                // Activation, VolumeBinding, and Endpoint families'
                // declared effects services from the same facet sets their
                // driver factories are built from, exactly as the production
                // composition root does.
                // U1/U14/U5/U10: the plane hosts the Process, Network, Host,
                // Activation, and Guest families' declared effects services
                // from the same facet sets their driver factories are built
                // from, exactly as the production composition root does.
                // Activation, and User families' declared effects services
                // from the same facet sets their driver factories are built
                // from, exactly as the production composition root does.
                effect_service_factories: BTreeMap::from([
                    (
                        PROCESS_EFFECTS_SERVICE.id,
                        Arc::new(ProcessEffectsServiceFactory::new(process_facets))
                            as Arc<dyn EffectServiceFactory>,
                    ),
                    (
                        NETWORK_EFFECTS_SERVICE.id,
                        Arc::new(NetworkEffectsServiceFactory::new(
                            network_facets.clone(),
                        )) as Arc<dyn EffectServiceFactory>,
                    ),
                    (
HOST_EFFECTS_SERVICE.id,
                        Arc::new(HostEffectsServiceFactory::new(host_facets))
                            as Arc<dyn EffectServiceFactory>,
                    ),
                    (
                        VOLUME_EFFECTS_SERVICE.id,
                        Arc::new(VolumeEffectsServiceFactory::new(
                            volume_facets.clone(),
                        )) as Arc<dyn EffectServiceFactory>,
                    ),
                    (
                        d2b_provider_wayland_policy::INTERACTION_EFFECTS_SERVICE.id,
                        Arc::new(
                            d2b_provider_wayland_policy::InteractionEffectsServiceFactory::new(
                                interaction_facets,
                            ),
                        ) as Arc<dyn EffectServiceFactory>,
                    ),
                    (
                        ACTIVATION_EFFECTS_SERVICE.id,
                        Arc::new(ActivationEffectsServiceFactory::new(activation_facets))
                            as Arc<dyn EffectServiceFactory>,
                    ),
                    // U15: the family's service carries no facet set (R2),
                    // so the plane tests host its factory from crate-owned
                    // constants alone, exactly as the production composition
                    // root does.
                    (
                        PROCESS_SYSTEMD_EFFECTS_SERVICE.id,
                        Arc::new(SystemdEffectsServiceFactory::new())
                            as Arc<dyn EffectServiceFactory>,
                    ),
                    (
                        USER_EFFECTS_SERVICE.id,
                        Arc::new(UserEffectsServiceFactory::new(user_facets.clone()))
                            as Arc<dyn EffectServiceFactory>,
                    ),
                    (
                        GUEST_EFFECTS_SERVICE.id,
                        Arc::new(GuestEffectsServiceFactory::new(guest_facets))
                            as Arc<dyn EffectServiceFactory>,
                    ),
                    (
                        USBIP_EFFECTS_SERVICE.id,
                        Arc::new(d2b_provider_device_usbip::effects_service::
                            UsbipEffectsServiceFactory::new(
                                usbip_facets,
                            )) as Arc<dyn EffectServiceFactory>,
                    ),
                    (
                        SECURITY_KEY_EFFECTS_SERVICE.id,
                        Arc::new(d2b_provider_device_security_key::effects_service::
                            SecurityKeyEffectsServiceFactory::new(
                                security_key_facets,
                            )) as Arc<dyn EffectServiceFactory>,
                    ),
                    (
                        DEVICE_EFFECTS_SERVICE.id,
                        Arc::new(d2b_provider_device::effects_service::
                            DeviceEffectsServiceFactory::new(device_facets))
                            as Arc<dyn EffectServiceFactory>,
                    ),
                    (
                        d2b_provider_volume_binding::BINDING_EFFECTS_SERVICE.id,
                        Arc::new(d2b_provider_volume_binding::BindingEffectsServiceFactory::new(
                            binding_facets.clone(),
                        )) as Arc<dyn EffectServiceFactory>,
                    ),
                    (
                        d2b_provider_endpoint::ENDPOINT_EFFECTS_SERVICE.id,
                        Arc::new(d2b_provider_endpoint::EndpointEffectsServiceFactory::new(
                            endpoint_facets.clone(),
                        )) as Arc<dyn EffectServiceFactory>,
                    ),
                    (
                        CREDENTIAL_EFFECTS_SERVICE.id,
                        Arc::new(CredentialEffectsServiceFactory::new(
                            credential_facets.clone(),
                        )) as Arc<dyn EffectServiceFactory>,
                    ),
                ]),
                foundation: None,
            },
            readiness,
        )
    }

    // ---- U3 composition-root service hosting (R5) ----

    use crate::effect_service_actors::ServiceCallData;
    use d2b_contracts_resource::v3::CanonicalJsonObject;
    use d2b_provider_toolkit::{
        EffectResponse, EffectService, EffectServiceError, EffectServiceFactory, ServiceDecl,
        ServiceInvocation,
    };
    use d2b_resource_runtime::context::ServiceResourceContext;
    use d2b_resource_runtime::driver::{DynResourceDriver, ResourceDriverFactory};
    use d2b_provider_wayland_policy::InteractionDriverEffects;
    use d2b_resource_types::{AllowedSources, DriverDescriptor, ServiceMethod, WellKnownType};

    /// The declared service the composition tests host.
    const COMPOSITION_SERVICE: ServiceDecl = ServiceDecl {
        id: "fixture.echo",
        methods: &[ServiceMethod::serving("fixture-echo-ping", "ping")],
        attach_kinds: &[],
        streams: &[],
        endpoint_policy: None,
    };

    /// Echo fixture served by the composition-hosted actor.
    struct EchoService;

    #[async_trait::async_trait]
    impl EffectService for EchoService {
        async fn handle(
            &self,
            invocation: ServiceInvocation<'_>,
        ) -> Result<EffectResponse, EffectServiceError> {
            Ok(EffectResponse::new(invocation.payload.clone()))
        }
    }

    /// Builds one echo service per respawn.
    struct EchoFactory;

    impl EffectServiceFactory for EchoFactory {
        fn build(&self) -> Arc<dyn EffectService> {
            Arc::new(EchoService)
        }
    }

    /// A driver that registers cleanly beside its service declaration; its
    /// spec/driver methods are unreachable at the composition site.
    fn serving_descriptor(services: &'static [ServiceDecl]) -> DriverDescriptor {
        DriverDescriptor {
            resource_type: WellKnownType::PROCESS,
            allowed_sources: AllowedSources::STARTUP,
            verbs: &[],
            execution: &[],
            exportable: false,
            reads: &[],
            operations: &[],
            creations: &[],
            startup: &[],
            services,
            decoder: Arc::new(NoSpecs),
            factory: Arc::new(NoDrivers),
        }
    }

    struct NoSpecs;

    impl SpecDecoder for NoSpecs {
        fn decode(
            &self,
            _envelope: &[u8],
        ) -> Result<Box<dyn std::any::Any + Send>, Box<dyn std::error::Error + Send + Sync>> {
            unreachable!("the composition site decodes no specs")
        }
    }

    struct NoDrivers;

    #[async_trait::async_trait]
    impl ResourceDriverFactory for NoDrivers {
        fn resource_types(&self) -> &[ResourceTypeName] {
            &[]
        }

        async fn create(&self, _key: &ResourceKey) -> Box<dyn DynResourceDriver> {
            unreachable!("the composition site creates no resource drivers")
        }
    }

    /// U3 composition root (R5): a provider that declares a service is
    /// hosted when the composition inputs registered its factory - the
    /// composition root's factory application (`provider_set`'s
    /// `with_effect_service_factories` call over the inputs table) hosts
    /// the declared service and it answers an invocation carrying the real
    /// envelope payload.
    ///
    /// The set is built with the fixture provider alone: the plane's own
    /// providers register every well-known type, so a fixture driver cannot
    /// attach beside them; the composition application call is the same one
    /// `provider_set` makes over the inputs table.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread")]
    async fn the_composition_root_hosts_a_declared_service_with_its_registered_factory() {
        let (_dir, mut inputs, _readiness) = test_inputs();
        inputs
            .effect_service_factories
            .insert(COMPOSITION_SERVICE.id, Arc::new(EchoFactory));
        let runtime = ProviderSet::new(inputs.zone.clone(), inputs.spec_store_dir.clone())
            .with(
                family_declaration("fixture"),
                vec![serving_descriptor(&[COMPOSITION_SERVICE])],
            )
            .with_effect_service_factories(&inputs.effect_service_factories)
            .start()
            .await
            .expect("the composition root hosts the declared service");
        let binding = runtime
            .resolve_effect_service(COMPOSITION_SERVICE.id)
            .await
            .expect("the declared service resolves");
        assert_eq!(binding.revision(), 1, "first generation");
        let call = ServiceCallData {
            zone: "test".to_owned(),
            invocation_id: "invocation-7".to_owned(),
            payload: serde_json::from_value(serde_json::json!({ "echo": "ping" }))
                .expect("canonical payload"),
            resources: ServiceResourceContext::fail_closed(),
            method: COMPOSITION_SERVICE.methods[0],
            kernel: None,
            request_fds: Vec::new(),
            chain_identities: Vec::new(),
        };
        let response = binding.call(call).await.expect("call");
        assert_eq!(
            response.payload,
            serde_json::from_value::<CanonicalJsonObject>(serde_json::json!({ "echo": "ping" }))
                .expect("canonical payload"),
            "the composition-hosted service answered the real payload"
        );
    }

    /// U3 composition root (AE4): a declared service with no factory in the
    /// composition inputs refuses startup by name - the composition path
    /// keeps the fail-closed refusal.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread")]
    async fn the_composition_root_refuses_a_declared_service_without_a_factory() {
        let (_dir, inputs, _readiness) = test_inputs();
        let error = ResourcePlaneV3::provider_set(&inputs)
            .with(
                family_declaration("fixture"),
                vec![serving_descriptor(&[COMPOSITION_SERVICE])],
            )
            .start()
            .await
            .expect_err("a declared service needs a registered factory");
        assert_eq!(error.code(), "effect-service-factory-missing");
        assert_eq!(
            error.message(),
            "effect-service-factory-missing:fixture:fixture.echo"
        );
    }


    /// U1: the composition root hosts the Process family's declared effects
    /// service from the family's own factory over the plane's facet set, and
    /// the hosted service answers `has-active` through the real invocation
    /// capability object carrying the real envelope payload - the same
    /// implementation value the driver factory is built from.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread")]
    async fn the_process_effects_service_answers_has_active_through_the_binding() {
        let (_dir, inputs, _readiness) = test_inputs();
        let runtime = ResourcePlaneV3::provider_set(&inputs)
            .start()
            .await
            .expect("the plane starts the declared process service");
        let binding = runtime
            .resolve_effect_service(PROCESS_EFFECTS_SERVICE.id)
            .await
            .expect("the declared effects service resolves");
        let call = ServiceCallData {
            zone: "test".to_owned(),
            invocation_id: "invocation-u1-has-active".to_owned(),
            payload: serde_json::from_value(serde_json::json!({
                "zone": "test",
                "zoneUid": serde_json::Value::Null,
                "resourceRef": "Process/worker",
            }))
            .expect("canonical payload"),
            resources: ServiceResourceContext::fail_closed(),
            method: PROCESS_EFFECTS_SERVICE.methods[0],
            kernel: None,
            request_fds: Vec::new(),
            chain_identities: Vec::new(),
        };
        let response = binding.call(call).await.expect("call");
        assert_eq!(
            response.payload,
            serde_json::from_value::<CanonicalJsonObject>(serde_json::json!({ "active": false }))
                .expect("canonical payload"),
            "the hosted service answers the runtime facet's report"
        );
    }

    /// U14: the composition root hosts the Network family's declared effects
    /// service from the family's own factory over the plane's facet set, and
    /// the hosted service answers `inspect-network` through the real
    /// invocation capability object carrying the real envelope payload -
    /// the same implementation value the driver factory is built from. The
    /// report is served from the daemon-supplied bundle facet, so the trusted
    /// bundle and the installed generation identity cross the provider
    /// boundary as declared facets.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread")]
    async fn the_network_effects_service_answers_inspect_network_through_the_binding() {
        let (_dir, inputs, _readiness) = test_inputs();
        let runtime = ResourcePlaneV3::provider_set(&inputs)
            .start()
            .await
            .expect("the plane starts the declared network service");
        let binding = runtime
            .resolve_effect_service(NETWORK_EFFECTS_SERVICE.id)
            .await
            .expect("the declared effects service resolves");
        let call = ServiceCallData {
            zone: "test".to_owned(),
            invocation_id: "invocation-u14-inspect-network".to_owned(),
            payload: serde_json::from_value(serde_json::json!({})).expect("canonical payload"),
            resources: ServiceResourceContext::fail_closed(),
            method: NETWORK_EFFECTS_SERVICE.methods[0],
            kernel: None,
            request_fds: Vec::new(),
            chain_identities: Vec::new(),
        };
        let response = binding.call(call).await.expect("call");
        assert_eq!(
            response.payload,
            serde_json::from_value::<CanonicalJsonObject>(serde_json::json!({
                "family": "network-local",
                "resourceType": "Network",
                "installedGenerationId":
                    "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
                "hostNftables": { "family": "inet", "table": "d2b" },
                "eastWestOptIn": false,
                "operations": [
                    "ApplyNftables", "ApplyNftablesProjection", "ApplyNmUnmanaged",
                    "ApplyRoute", "ApplySysctl", "CreateBridge", "DeleteBridge",
                    "CreatePersistentTap", "DeletePersistentTap", "CreateTapFd",
                    "SetBridgePortFlags", "UpdateHostsFile", "SeedDnsmasqLease",
                ],
            }))
            .expect("canonical payload"),
            "the hosted service answers the trusted-bundle report from the bundle facet"
        );
    }

    /// The kernel-visible names the host family's probe asserts are the
    /// same names the sibling families declare: the pipewire runtime socket
    /// and the usbip kernel modules. The host probe spells them locally
    /// (it needs no sibling crate), so this composition-root test pins the
    /// shared-by-contract agreement: a rename on either side fails here.
    #[test]
    fn the_host_probe_and_sibling_families_agree_on_kernel_visible_names() {
        assert_eq!(
            d2b_provider_host::PIPEWIRE_RUNTIME_SOCKET,
            d2b_provider_audio_pipewire::PIPEWIRE_RUNTIME_SOCKET,
            "the host probe and the audio-pipewire family assert the same pipewire socket name"
        );
        assert_eq!(
            d2b_provider_host::USBIP_CORE_MODULE,
            d2b_provider_device_usbip::vocabulary::USBIP_CORE_MODULE,
            "the host probe and the device-usbip family assert the same usbip core module name"
        );
        assert_eq!(
            d2b_provider_host::USBIP_HOST_MODULE,
            d2b_provider_device_usbip::vocabulary::USBIP_HOST_MODULE,
            "the host probe and the device-usbip family assert the same usbip host module name"
        );
    }

    /// U5: the composition root hosts the Host family's declared effects
    /// service from the family's own factory over the plane's facet set, and
    /// the hosted service answers `inspect-host` through the real invocation
    /// capability object carrying the real envelope payload - the same
    /// implementation value the driver factory is built from. The report is
    /// the family's bounded probe running inside the owning crate: the
    /// capability classes present, the bounded metadata, and the minijail
    /// platform gate from the daemon-supplied facet.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread")]
    async fn the_host_effects_service_answers_inspect_host_through_the_binding() {
        let (_dir, inputs, _readiness) = test_inputs();
        let runtime = ResourcePlaneV3::provider_set(&inputs)
            .start()
            .await
            .expect("the plane starts the declared host service");
        let binding = runtime
            .resolve_effect_service(HOST_EFFECTS_SERVICE.id)
            .await
            .expect("the declared effects service resolves");
        let call = ServiceCallData {
            zone: "test".to_owned(),
            invocation_id: "invocation-u5-inspect-host".to_owned(),
            payload: serde_json::from_value(serde_json::json!({})).expect("canonical payload"),
            resources: ServiceResourceContext::fail_closed(),
            method: HOST_EFFECTS_SERVICE.methods[0],
            kernel: None,
            request_fds: Vec::new(),
            chain_identities: Vec::new(),
        };
        let response = binding.call(call).await.expect("call");
        let string_field = |key: &str| -> String {
            match response.payload.get(key) {
                Some(d2b_contracts_resource::v3::CanonicalJsonValue::String(value)) => {
                    value.clone()
                }
                other => panic!("field {key} is not a canonical string: {other:?}"),
            }
        };
        assert_eq!(string_field("family"), "host");
        assert_eq!(string_field("resourceType"), "Host");
        match response.payload.get("capabilities") {
            Some(d2b_contracts_resource::v3::CanonicalJsonValue::Array(capabilities)) => {
                assert!(
                    capabilities.len()
                        <= d2b_provider_system_core::HostCapabilityClass::ALL.len(),
                    "the report carries the bounded capability class set"
                );
            }
            other => panic!("capabilities is not a canonical array: {other:?}"),
        }
        let integer_field = |key: &str| -> i64 {
            match response
                .payload
                .get("minijail")
                .and_then(d2b_contracts_resource::v3::CanonicalJsonValue::as_object)
                .and_then(|gate| gate.get(key))
            {
                Some(d2b_contracts_resource::v3::CanonicalJsonValue::Integer(value)) => *value,
                other => panic!("minijail.{key} is not a canonical integer: {other:?}"),
            }
        };
        assert_eq!(
            integer_field("kernelMajor"),
            6,
            "the platform gate comes from the daemon-supplied facet"
        );
        assert_eq!(
            integer_field("kernelMinor"),
            9,
            "the platform gate comes from the daemon-supplied facet"
        );
        let kernel_release = string_field("kernelRelease");
        assert!(!kernel_release.is_empty());
        assert!(kernel_release.len() <= 64, "the observation is bounded");
        // Non-degenerate guard, not a value check: `activeProcessCount`
        // comes from the real probe's live `/proc` enumeration, so a
        // regression that degenerates the count to a constant zero would
        // otherwise pass this binding test. Every running machine has at
        // least one process - the daemon under test is one of them - so a
        // genuine count is never below 1 and the assertion cannot flake;
        // no exact or machine-tied range is asserted, since that would
        // trade this blind spot for a flake.
        let active_process_count = match response.payload.get("activeProcessCount") {
            Some(d2b_contracts_resource::v3::CanonicalJsonValue::Integer(count)) => *count,
            other => panic!("activeProcessCount is not a canonical integer: {other:?}"),
        };
        assert!(
            active_process_count >= 1,
            "process count is degenerate: {active_process_count}"
        );
    }

    /// KTD8 restart adoption for the rows this lane moves: an
    /// already-provisioned plane restarts and re-hosts the Host family's
    /// declared effects service from the same facet set, and the fresh
    /// generation answers the same `inspect-host` surface - the surface this
    /// lane moved adopts on restart, with no daemon-side effect module.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread")]
    async fn a_restarted_plane_rehosts_the_host_effects_service() {
        let (_dir, inputs, _readiness) = test_inputs();
        let started = ResourcePlaneV3::provider_set(&inputs)
            .start()
            .await
            .expect("the plane starts with the declared host service");
        let binding = started
            .resolve_effect_service(HOST_EFFECTS_SERVICE.id)
            .await
            .expect("the declared effects service resolves");
        let call = ServiceCallData {
            zone: "test".to_owned(),
            invocation_id: "invocation-u5-before-restart".to_owned(),
            payload: serde_json::from_value(serde_json::json!({})).expect("canonical payload"),
            resources: ServiceResourceContext::fail_closed(),
            method: HOST_EFFECTS_SERVICE.methods[0],
            kernel: None,
            request_fds: Vec::new(),
            chain_identities: Vec::new(),
        };
        let before = binding.call(call).await.expect("call before restart");
        drop(started);

        // The daemon restarts: the provider set is rebuilt from the same
        // declaration and factory, and the Host service is hosted again.
        let restarted = ResourcePlaneV3::provider_set(&inputs)
            .start()
            .await
            .expect("the restarted plane starts");
        let adopted = restarted
            .resolve_effect_service(HOST_EFFECTS_SERVICE.id)
            .await
            .expect("the restarted plane re-hosts the declared host service");
        let call = ServiceCallData {
            zone: "test".to_owned(),
            invocation_id: "invocation-u5-after-restart".to_owned(),
            payload: serde_json::from_value(serde_json::json!({})).expect("canonical payload"),
            resources: ServiceResourceContext::fail_closed(),
            method: HOST_EFFECTS_SERVICE.methods[0],
            kernel: None,
            request_fds: Vec::new(),
            chain_identities: Vec::new(),
        };
        let after = adopted.call(call).await.expect("call after restart");
        // `activeProcessCount` is computed from the live `/proc` process
        // count, so any process starting or exiting between the two calls
        // changes it; the restart-adoption meaning lives in the stable
        // fields. Normalize the volatile field on both sides (asserting it
        // is present and well-formed on both) and compare the remainders
        // exactly.
        let volatile_count = |payload: &d2b_contracts_resource::v3::CanonicalJsonObject| {
            match payload.get("activeProcessCount") {
                Some(d2b_contracts_resource::v3::CanonicalJsonValue::Integer(count)) => *count,
                other => panic!("activeProcessCount is not a canonical integer: {other:?}"),
            }
        };
        let before_count = volatile_count(&before.payload);
        let after_count = volatile_count(&after.payload);
        assert!(
            before_count >= 1 && after_count >= 1,
            "the process count is a non-degenerate observation (at least one process)"
        );
        let mut before_fields = before.payload.clone().into_inner();
        let mut after_fields = after.payload.clone().into_inner();
        before_fields.remove("activeProcessCount");
        after_fields.remove("activeProcessCount");
        assert_eq!(
            after_fields, before_fields,
            "the adopted generation answers the same bounded observations (the volatile process count normalized out)"
        );
    }

    /// U7: the composition root hosts the Volume family's declared effects
    /// service from the family's own factory over the plane's facet set, and
    /// the hosted service answers `has-layout` through the real invocation
    /// capability object carrying the real envelope payload - the same
    /// implementation value the driver factory is built from. The answer is
    /// served from the daemon-supplied runtime facet (the durable layout
    /// probe), so the daemon's layout state crosses the provider boundary as
    /// a declared facet.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread")]
    async fn the_volume_effects_service_answers_has_layout_through_the_binding() {
        let (_dir, inputs, _readiness) = test_inputs();
        let runtime = ResourcePlaneV3::provider_set(&inputs)
            .start()
            .await
            .expect("the plane starts the declared volume service");
        let binding = runtime
            .resolve_effect_service(VOLUME_EFFECTS_SERVICE.id)
            .await
            .expect("the declared effects service resolves");
        let call = ServiceCallData {
            zone: "test".to_owned(),
            invocation_id: "invocation-u7-has-layout".to_owned(),
            payload: serde_json::from_value(serde_json::json!({
                "volumeUid": "6f9619ff-8b86-4d01-b42d-00cf4fc964ff",
            }))
            .expect("canonical payload"),
            resources: ServiceResourceContext::fail_closed(),
            method: VOLUME_EFFECTS_SERVICE.methods[0],
            kernel: None,
            request_fds: Vec::new(),
            chain_identities: Vec::new(),
        };
        let response = binding.call(call).await.expect("call");
        assert_eq!(
            response.payload,
            serde_json::from_value::<CanonicalJsonObject>(serde_json::json!({
                "hasLayout": false,
            }))
            .expect("canonical payload"),
            "the hosted service answers the durable layout probe from the runtime facet"
        );
    }

    /// U12: the composition root hosts the interaction family's declared
    /// effects service from the family's own factory over the plane's facet
    /// set, and the hosted service answers `audio-binding-statuses` through
    /// the real invocation capability object - the same shared per-zone
    /// audio registry the six drivers reconcile. The method's answer
    /// hand-mirrors the frozen wire schema, so the exact payload is pinned:
    /// an empty registry answers the empty bindings list, and a binding
    /// reconciled through the family's own effects over the same facet set
    /// appears as its typed status row (the scripted audio source publishes
    /// the degraded, host-and-guest unavailable status exactly as a target
    /// without an audio capability does).
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread")]
    async fn the_interaction_effects_service_answers_audio_binding_statuses_through_the_binding() {
        let zone = ZoneId::parse("test").unwrap();
        let service_ref = ResourceRef::parse("audio.d2bus.org.AudioService/host-audio").unwrap();
        let guest_ref = ResourceRef::parse("Guest/work").unwrap();
        let binding_ref = ResourceRef::parse("audio.d2bus.org.AudioBinding/mic").unwrap();
        let facets = d2b_provider_wayland_policy::test_support::scripted_facets_with_rows(
            zone.clone(),
            vec![
                audio_seeded_row(
                    &zone,
                    &service_ref,
                    serde_json::json!({
                        "serviceRole": "owner",
                        "implementationEndpointRefs": ["Endpoint/audio"],
                        "operations": ["playback", "capture"],
                    }),
                ),
                audio_seeded_row(&zone, &guest_ref, serde_json::json!({})),
                audio_seeded_row(
                    &zone,
                    &binding_ref,
                    serde_json::json!({
                        "serviceRef": service_ref.to_canonical_string(),
                        "targetRef": guest_ref.to_canonical_string(),
                        "grants": {"mic": "off", "speaker": "off"},
                    }),
                ),
            ],
        );
        let (_dir, inputs, _readiness) = test_inputs_with_interaction_facets(facets.clone());
        let runtime = ResourcePlaneV3::provider_set(&inputs)
            .start()
            .await
            .expect("the plane starts the declared interaction service");
        let binding = runtime
            .resolve_effect_service(d2b_provider_wayland_policy::INTERACTION_EFFECTS_SERVICE.id)
            .await
            .expect("the declared effects service resolves");
        let call = |invocation_id: &str| ServiceCallData {
            zone: "test".to_owned(),
            invocation_id: invocation_id.to_owned(),
            payload: serde_json::from_value(serde_json::json!({})).expect("canonical payload"),
            resources: ServiceResourceContext::fail_closed(),
            method: d2b_provider_wayland_policy::INTERACTION_EFFECTS_SERVICE.methods[0],
            kernel: None,
            request_fds: Vec::new(),
            chain_identities: Vec::new(),
        };

        // The empty registry answers the empty bindings list.
        let response = binding.call(call("invocation-u12-audio-binding-statuses")).await.expect("call");
        assert_eq!(
            response.payload,
            serde_json::from_value::<CanonicalJsonObject>(serde_json::json!({
                "family": "interaction",
                "bindings": [],
            }))
            .expect("canonical payload"),
            "the hosted service answers the empty registry report"
        );

        // Seed the shared registry through the family's own effects over the
        // same facet set the plane hosts the service from: the service row
        // reconciles, then the binding row publishes its typed status (the
        // scripted audio source carries no capability, so the binding is
        // degraded with both readinesses unavailable, exactly as a target
        // without an audio capability is).
        let effects = d2b_provider_wayland_policy::InteractionEffectsService::new(facets);
        effects
            .reconcile(
                d2b_provider_wayland_policy::InteractionKind::AudioService,
                &d2b_provider_wayland_policy::InteractionEffectRequest {
                    target: ResourceKey::new(
                        "test",
                        "audio.d2bus.org.AudioService",
                        "host-audio",
                    ),
                    uid: resource_uid(&[0x51; 16]).unwrap(),
                    generation: 1,
                    controller_generation: 1,
                    spec: serde_json::json!({}),
                    provider_ref: Some(
                        ResourceRef::parse("Provider/audio-pipewire").unwrap(),
                    ),
                    children: &[],
                },
            )
            .await
            .expect("the service row reconciles");
        effects
            .reconcile(
                d2b_provider_wayland_policy::InteractionKind::AudioBinding,
                &d2b_provider_wayland_policy::InteractionEffectRequest {
                    target: ResourceKey::new("test", "audio.d2bus.org.AudioBinding", "mic"),
                    uid: resource_uid(&[0x52; 16]).unwrap(),
                    generation: 1,
                    controller_generation: 1,
                    spec: serde_json::json!({
                        "serviceRef": service_ref.to_canonical_string(),
                        "targetRef": guest_ref.to_canonical_string(),
                        "grants": {"mic": "off", "speaker": "off"},
                    }),
                    provider_ref: Some(
                        ResourceRef::parse("Provider/audio-pipewire").unwrap(),
                    ),
                    children: &[],
                },
            )
            .await
            .expect("the binding row reconciles");

        let response = binding
            .call(call("invocation-u12-audio-binding-statuses-seeded"))
            .await
            .expect("call");
        assert_eq!(
            response.payload,
            serde_json::from_value::<CanonicalJsonObject>(serde_json::json!({
                "family": "interaction",
                "bindings": [{
                    "resource": binding_ref.to_canonical_string(),
                    "phase": "Degraded",
                    "hostReadiness": "Unavailable",
                    "guestReadiness": "Unavailable",
                    "channels": {
                        "speaker": {"grant": "off", "level": null, "liveEnforced": false},
                        "mic": {
                            "grant": "off",
                            "gain": null,
                            "liveEnforced": false,
                            "arbitrationState": "inactive",
                        },
                    },
                    "enforcementPosture": "None",
                    "lastSetApplied": "OfflineOnly",
                }],
            }))
            .expect("canonical payload"),
            "the hosted service answers the seeded binding's typed status row"
        );
    }

    /// One manager row the scripted interaction facet set serves: the
    /// envelope-shaped spec is rendered through the manager's canonical
    /// projection with the row's live status, so the effects' dependency
    /// reads validate it exactly as a committed manager row.
    fn audio_seeded_row(
        zone: &ZoneId,
        resource_ref: &ResourceRef,
        base_spec: serde_json::Value,
    ) -> ResourceView {
        let mut spec = base_spec;
        if let Some(object) = spec.as_object_mut() {
            object.insert(
                "providerRef".to_owned(),
                serde_json::Value::String("Provider/audio-pipewire".to_owned()),
            );
        }
        ResourceView {
            key: ResourceKey::new(
                zone.as_str(),
                resource_ref.resource_type().as_str(),
                resource_ref.name().as_str(),
            ),
            uid: [0x61; 16],
            generation: 1,
            deleting: false,
            provenance: d2b_resource_runtime::identity::ResourceProvenance::Nix,
            spec: serde_json::to_vec(&spec).expect("seeded row spec"),
            metadata: Vec::new(),
            owner_key: None,
            status: Some(ResourceStatus::Ready),
            status_generation: Some(1),
            status_projection: None,
        }
    }

    /// The composition root hosts the Activation family's declared effects
    /// service from the family's own factory over the plane's facet set, and
    /// the hosted service answers `inspect-activation` through the real
    /// invocation capability object carrying the real envelope payload - the
    /// same implementation value the driver factory is built from. The
    /// report is the family's committed surface, so it proves the family's
    /// effects run inside the owning crate.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread")]
    async fn the_activation_effects_service_answers_inspect_activation_through_the_binding() {
        let (_dir, inputs, _readiness) = test_inputs();
        let runtime = ResourcePlaneV3::provider_set(&inputs)
            .start()
            .await
            .expect("the plane starts the declared activation service");
        let binding = runtime
            .resolve_effect_service(ACTIVATION_EFFECTS_SERVICE.id)
            .await
            .expect("the declared effects service resolves");
        let call = ServiceCallData {
            zone: "test".to_owned(),
            invocation_id: "invocation-inspect-activation".to_owned(),
            payload: serde_json::from_value(serde_json::json!({})).expect("canonical payload"),
            resources: ServiceResourceContext::fail_closed(),
            method: ACTIVATION_EFFECTS_SERVICE.methods[0],
            kernel: None,
            request_fds: Vec::new(),
            chain_identities: Vec::new(),
        };
        let response = binding.call(call).await.expect("call");
        assert_eq!(
            response.payload,
            serde_json::from_value::<CanonicalJsonObject>(serde_json::json!({
                "family": "activation-nixos",
                "resourceType": d2b_provider_activation_nixos::ACTIVATION_TYPE_NAME,
                "runner": {
                    "providerRef": "Provider/system-minijail",
                    "type": "EphemeralProcess",
                },
                "handoffOperation": "ApplyHostGenerationHandoff",
                "runnerSteps": ["switch", "boot", "test"],
            }))
            .expect("canonical payload"),
            "the hosted service answers the family's committed surface from the crate's own vocabulary"
        );
    }

    /// A restarted plane re-hosts the Activation family's declared effects
    /// service from the same facet set, and the fresh generation answers the
    /// same `inspect-activation` surface - the surface this lane moved
    /// adopts on restart, with no daemon-side effect module.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread")]
    async fn a_restarted_plane_rehosts_the_activation_effects_service() {
        let (_dir, inputs, _readiness) = test_inputs();
        let started = ResourcePlaneV3::provider_set(&inputs)
            .start()
            .await
            .expect("the plane starts with the declared activation service");
        let binding = started
            .resolve_effect_service(ACTIVATION_EFFECTS_SERVICE.id)
            .await
            .expect("the declared effects service resolves");
        let call = ServiceCallData {
            zone: "test".to_owned(),
            invocation_id: "invocation-activation-before-restart".to_owned(),
            payload: serde_json::from_value(serde_json::json!({})).expect("canonical payload"),
            resources: ServiceResourceContext::fail_closed(),
            method: ACTIVATION_EFFECTS_SERVICE.methods[0],
            kernel: None,
            request_fds: Vec::new(),
            chain_identities: Vec::new(),
        };
        let before = binding.call(call).await.expect("call before restart");
        drop(started);

        // The daemon restarts: the provider set is rebuilt from the same
        // declaration and factory, and the Activation service is hosted
        // again.
        let restarted = ResourcePlaneV3::provider_set(&inputs)
            .start()
            .await
            .expect("the restarted plane starts");
        let adopted = restarted
            .resolve_effect_service(ACTIVATION_EFFECTS_SERVICE.id)
            .await
            .expect("the restarted plane re-hosts the declared activation service");
        let call = ServiceCallData {
            zone: "test".to_owned(),
            invocation_id: "invocation-activation-after-restart".to_owned(),
            payload: serde_json::from_value(serde_json::json!({})).expect("canonical payload"),
            resources: ServiceResourceContext::fail_closed(),
            method: ACTIVATION_EFFECTS_SERVICE.methods[0],
            kernel: None,
            request_fds: Vec::new(),
            chain_identities: Vec::new(),
        };
        let after = adopted.call(call).await.expect("call after restart");
        assert_eq!(
            after.payload, before.payload,
            "the restarted plane re-hosts the same committed surface"
        );
    }

    /// U15:the composition root hosts the process-systemd family's
    /// declared effects service from the family's own factory over the
    /// registered service identity (U3,R5: the registration table
    /// carries the row;the daemon names no family string, only the
    /// crate's declared service id),and the hosted service answers
    /// `inspect-process-systemd` through the real invocation capability
    /// object carrying the real envelope payload - hermetic, served from
    /// the crate's own handler table, reaching no daemon state.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread")]
    async fn the_process_systemd_effects_service_answers_inspect_process_systemd_through_the_binding() {
        let (_dir, inputs, _readiness) = test_inputs();
        let runtime = ResourcePlaneV3::provider_set(&inputs)
            .start()
            .await
            .expect("the plane starts the declared process-systemd service");
        let binding = runtime
            .resolve_effect_service(PROCESS_SYSTEMD_EFFECTS_SERVICE.id)
            .await
            .expect("the declared effects service resolves");
        let method = PROCESS_SYSTEMD_EFFECTS_SERVICE
            .methods
            .iter()
            .find(|method| method.name == "inspect-process-systemd")
            .copied()
            .expect("the zone-plane inventory method is declared");
        let call = ServiceCallData {
            zone:"test".to_owned(),
            invocation_id:"invocation-u15-inspect-process-systemd".to_owned(),
            payload: serde_json::from_value(serde_json::json!({})).expect("canonical payload"),
            resources: ServiceResourceContext::fail_closed(),
            method,
            kernel: None,
            request_fds: Vec::new(),
            chain_identities: Vec::new(),
        };
        let response = binding.call(call).await.expect("call");
        assert_eq!(
            response.payload,
            serde_json::from_value::<CanonicalJsonObject>(serde_json::json!({
                "family": "process-systemd",
                "declaringProvider": "d2b-provider-process-systemd",
                "operations": [
                    "StartSystemdUnit", "CheckSystemdUserManager", "ObserveSystemdUnit",
                    "OpenSystemdUnitPidfd", "StopSystemdUnit",
                ],
                "declaredOperations": 5,
                "service": "process-systemd.d2bus.org/effects",
            }))
            .expect("canonical payload"),
            "the hosted service answers the crate-owned operation inventory"
        );
    }

    /// U5: the composition root hosts the User family's declared effects
    /// service from the family's own factory over the plane's facet set, and
    /// the hosted service answers `inspect-user` through the real invocation
    /// capability object carrying the real envelope payload - the same
    /// implementation value the driver factory is built from. The plane's
    /// test inputs carry the scripted probe, so the report is unconditional:
    /// the declared identity resolves as discovered with the scripted
    /// opaque identity digest, on any host.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread")]
    async fn the_user_effects_service_answers_inspect_user_through_the_binding() {
        let (_dir, inputs, _readiness) = test_inputs();
        let runtime = ResourcePlaneV3::provider_set(&inputs)
            .start()
            .await
            .expect("the plane starts the declared user service");
        let binding = runtime
            .resolve_effect_service(USER_EFFECTS_SERVICE.id)
            .await
            .expect("the declared effects service resolves");
        let call = ServiceCallData {
            zone: "test".to_owned(),
            invocation_id: "invocation-u5-inspect-user".to_owned(),
            payload: serde_json::from_value(serde_json::json!({
                "userRef": "User/inspect-user-u5",
                "osUsername": "alice",
                "groups": [],
            }))
            .expect("canonical payload"),
            resources: ServiceResourceContext::fail_closed(),
            method: USER_EFFECTS_SERVICE.methods[0],
            kernel: None,
            request_fds: Vec::new(),
            chain_identities: Vec::new(),
        };
        let response = binding.call(call).await.expect("call");
        let string_field = |key: &str| -> String {
            match response.payload.get(key) {
                Some(d2b_contracts_resource::v3::CanonicalJsonValue::String(value)) => {
                    value.clone()
                }
                other => panic!("field {key} is not a canonical string: {other:?}"),
            }
        };
        assert_eq!(string_field("family"), "user");
        assert_eq!(string_field("resourceType"), "User");
        assert_eq!(string_field("userRef"), "User/inspect-user-u5");
        assert_eq!(string_field("username"), "alice");
        assert_eq!(string_field("provider"), "system-core");
        assert_eq!(
            string_field("phase"),
            "Ready",
            "the scripted probe resolves the declared identity"
        );
        assert_eq!(
            string_field("discovery"),
            "discovered",
            "the scripted probe resolves the declared identity"
        );
        assert_eq!(
            string_field("identity"),
            UserIdentityDigest::from_bytes([0x5a; 32]).to_hex(),
            "the observation carries the scripted opaque identity digest"
        );
    }

    /// KTD8 restart adoption for the rows this lane moves: an
    /// already-provisioned plane restarts and re-hosts the User family's
    /// declared effects service from the same facet set, and the fresh
    /// generation answers the same `inspect-user` surface - the surface this
    /// lane moved adopts on restart, with no daemon-side effect module.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread")]
    async fn a_restarted_plane_rehosts_the_user_effects_service() {
        let payload: d2b_contracts_resource::v3::CanonicalJsonObject =
            serde_json::from_value(serde_json::json!({
                "userRef": "User/inspect-user-u5",
                "osUsername": "alice",
                "groups": [],
            }))
            .expect("canonical payload");
        let (_dir, inputs, _readiness) = test_inputs();
        let started = ResourcePlaneV3::provider_set(&inputs)
            .start()
            .await
            .expect("the plane starts with the declared user service");
        let binding = started
            .resolve_effect_service(USER_EFFECTS_SERVICE.id)
            .await
            .expect("the declared effects service resolves");
        let call = ServiceCallData {
            zone: "test".to_owned(),
            invocation_id: "invocation-u5-before-restart".to_owned(),
            payload: payload.clone(),
            resources: ServiceResourceContext::fail_closed(),
            method: USER_EFFECTS_SERVICE.methods[0],
            kernel: None,
            request_fds: Vec::new(),
            chain_identities: Vec::new(),
        };
        let before = binding.call(call).await.expect("call before restart");
        drop(started);

        // The daemon restarts: the provider set is rebuilt from the same
        // declarations and factories, and the User service is hosted again.
        let restarted = ResourcePlaneV3::provider_set(&inputs)
            .start()
            .await
            .expect("the restarted plane starts");
        let adopted = restarted
            .resolve_effect_service(USER_EFFECTS_SERVICE.id)
            .await
            .expect("the restarted plane re-hosts the declared user service");
        let call = ServiceCallData {
            zone: "test".to_owned(),
            invocation_id: "invocation-u5-after-restart".to_owned(),
            payload,
            resources: ServiceResourceContext::fail_closed(),
            method: USER_EFFECTS_SERVICE.methods[0],
            kernel: None,
            request_fds: Vec::new(),
            chain_identities: Vec::new(),
        };
        let after = adopted.call(call).await.expect("call after restart");
        assert_eq!(
            after.payload, before.payload,
            "the adopted generation answers the same bounded observations"
        );
    }

    /// U10: the composition root hosts the Guest family's declared effects
    /// service from the family's own factory over the plane's facet set,
    /// and the hosted service answers `guest-phase` through the real
    /// invocation capability object carrying the real envelope payload -
    /// the same manager-view read the driver's effects gate on. The
    /// scripted facets double holds no row for the payload's Guest, so the
    /// honest answer is `Absent`.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread")]
    async fn the_guest_effects_service_answers_guest_phase_through_the_binding() {
        let (_dir, inputs, _readiness) = test_inputs();
        let runtime = ResourcePlaneV3::provider_set(&inputs)
            .start()
            .await
            .expect("the plane starts the declared guest service");
        let binding = runtime
            .resolve_effect_service(GUEST_EFFECTS_SERVICE.id)
            .await
            .expect("the declared effects service resolves");
        let call = ServiceCallData {
            zone: "test".to_owned(),
            invocation_id: "invocation-u10-guest-phase".to_owned(),
            payload: serde_json::from_value(serde_json::json!({
                "zone": "test",
                "zoneUid": serde_json::Value::Null,
                "resourceRef": "Guest/worker",
            }))
            .expect("canonical payload"),
            resources: ServiceResourceContext::fail_closed(),
            method: GUEST_EFFECTS_SERVICE.methods[0],
            kernel: None,
            request_fds: Vec::new(),
            chain_identities: Vec::new(),
        };
        let response = binding.call(call).await.expect("call");
        assert_eq!(
            response.payload,
            serde_json::from_value::<CanonicalJsonObject>(serde_json::json!({
                "family": "guest",
                "resourceType": "Guest",
                "guestRef": "Guest/worker",
                "phase": "Absent",
            }))
            .expect("canonical payload"),
            "the hosted service answers the manager view's honest absent report"
        );
    }

    /// U6: the composition root hosts the Endpoint family's declared effects
    /// service from the family's own factory over the plane's facet set, and
    /// the hosted service answers `inspect-endpoint` through the real
    /// invocation capability object carrying the real envelope payload - the
    /// same implementation value the driver factory is built from. The report
    /// is served from the crate's own purpose derivations, so it proves the
    /// purpose vocabulary runs inside the owning crate.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread")]
    async fn the_endpoint_effects_service_answers_inspect_endpoint_through_the_binding() {
        let (_dir, inputs, _readiness) = test_inputs();
        let runtime = ResourcePlaneV3::provider_set(&inputs)
            .start()
            .await
            .expect("the plane starts the declared endpoint service");
        let binding = runtime
            .resolve_effect_service(d2b_provider_endpoint::ENDPOINT_EFFECTS_SERVICE.id)
            .await
            .expect("the declared effects service resolves");
        let call = ServiceCallData {
            zone: "test".to_owned(),
            invocation_id: "invocation-u6-inspect-endpoint".to_owned(),
            payload: serde_json::from_value(serde_json::json!({})).expect("canonical payload"),
            resources: ServiceResourceContext::fail_closed(),
            method: d2b_provider_endpoint::ENDPOINT_EFFECTS_SERVICE.methods[0],
            kernel: None,
            request_fds: Vec::new(),
            chain_identities: Vec::new(),
        };
        let response = binding.call(call).await.expect("call");
        let string_field = |key: &str| -> String {
            match response.payload.get(key) {
                Some(d2b_contracts_resource::v3::CanonicalJsonValue::String(value)) => {
                    value.clone()
                }
                other => panic!("field {key} is not a canonical string: {other:?}"),
            }
        };
        assert_eq!(string_field("family"), "endpoint");
        assert_eq!(string_field("resourceType"), "Endpoint");
        let purposes = match response.payload.get("purposes") {
            Some(d2b_contracts_resource::v3::CanonicalJsonValue::Object(purposes)) => purposes,
            other => panic!("purposes is not a canonical object: {other:?}"),
        };
        assert!(
            purposes.contains_key("ch-api"),
            "the report carries the Cloud Hypervisor child-role purpose"
        );
        assert!(
            purposes.contains_key("swtpm-tpm-socket"),
            "the report carries the Device TPM worker-socket purpose"
        );
        match response.payload.get("realizations") {
            Some(d2b_contracts_resource::v3::CanonicalJsonValue::Array(realizations)) => {
                assert_eq!(realizations.len(), 3, "the closed realization inventory");
            }
            other => panic!("realizations is not a canonical array: {other:?}"),
        }
    }

    /// U6: the composition root hosts the VolumeBinding family's declared
    /// effects service from the family's own factory over the plane's facet
    /// set, and the hosted service answers `inspect-binding` through the real
    /// invocation capability object carrying the real envelope payload - the
    /// same implementation value the driver factory is built from. The report
    /// is served from the crate's own committed serving contract, so it
    /// proves the family's serving effects run inside the owning crate.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread")]
    async fn the_binding_effects_service_answers_inspect_binding_through_the_binding() {
        let (_dir, inputs, _readiness) = test_inputs();
        let runtime = ResourcePlaneV3::provider_set(&inputs)
            .start()
            .await
            .expect("the plane starts the declared binding service");
        let binding = runtime
            .resolve_effect_service(d2b_provider_volume_binding::BINDING_EFFECTS_SERVICE.id)
            .await
            .expect("the declared effects service resolves");
        let call = ServiceCallData {
            zone: "test".to_owned(),
            invocation_id: "invocation-u6-inspect-binding".to_owned(),
            payload: serde_json::from_value(serde_json::json!({})).expect("canonical payload"),
            resources: ServiceResourceContext::fail_closed(),
            method: d2b_provider_volume_binding::BINDING_EFFECTS_SERVICE.methods[0],
            kernel: None,
            request_fds: Vec::new(),
            chain_identities: Vec::new(),
        };
        let response = binding.call(call).await.expect("call");
        let string_field = |key: &str| -> String {
            match response.payload.get(key) {
                Some(d2b_contracts_resource::v3::CanonicalJsonValue::String(value)) => {
                    value.clone()
                }
                other => panic!("field {key} is not a canonical string: {other:?}"),
            }
        };
        assert_eq!(string_field("family"), "volume-binding");
        assert_eq!(string_field("resourceType"), "VolumeBinding");
        match response.payload.get("creations") {
            Some(d2b_contracts_resource::v3::CanonicalJsonValue::Array(creations)) => {
                assert_eq!(
                    creations.len(),
                    2,
                    "the report carries the worker Process and Endpoint children"
                );
            }
            other => panic!("creations is not a canonical array: {other:?}"),
        }
        match response.payload.get("serving") {
            Some(d2b_contracts_resource::v3::CanonicalJsonValue::Array(serving)) => {
                assert_eq!(
                    serving.len(),
                    3,
                    "the report carries the three serving surfaces"
                );
            }
            other => panic!("serving is not a canonical array: {other:?}"),
        }
    }

    /// U8: the composition root hosts the Credential family's declared
    /// effects service from the family's own factory over the plane's facet
    /// set, and the hosted service answers `inspect-credential` through the
    /// real invocation capability object carrying the real envelope payload
    /// - the same implementation value the driver factory is built from.
    /// The report is served from the crate's own committed identities, so
    /// it proves the family's surface lives in the owning crate and answers
    /// no credential material.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread")]
    async fn the_credential_effects_service_answers_inspect_credential_through_the_binding() {
        let (_dir, inputs, _readiness) = test_inputs();
        let runtime = ResourcePlaneV3::provider_set(&inputs)
            .start()
            .await
            .expect("the plane starts the declared credential service");
        let binding = runtime
            .resolve_effect_service(CREDENTIAL_EFFECTS_SERVICE.id)
            .await
            .expect("the declared effects service resolves");
        let call = ServiceCallData {
            zone: "test".to_owned(),
            invocation_id: "invocation-u8-inspect-credential".to_owned(),
            payload: serde_json::from_value(serde_json::json!({})).expect("canonical payload"),
            resources: ServiceResourceContext::fail_closed(),
            method: CREDENTIAL_EFFECTS_SERVICE.methods[0],
            kernel: None,
            request_fds: Vec::new(),
            chain_identities: Vec::new(),
        };
        let response = binding.call(call).await.expect("call");
        assert_eq!(
            response.payload,
            serde_json::from_value::<CanonicalJsonObject>(serde_json::json!({
                "family": "credential",
                "resourceType": "Credential",
                "providers": [
                    "Provider/credential-secret-service",
                    "Provider/credential-entra",
                    "Provider/credential-managed-identity",
                ],
                "agentBinary": "d2b-managed-identity-agent",
            }))
            .expect("canonical payload"),
            "the hosted service answers the committed family surface"
        );
    }

    /// The providers the plane starts register exactly the converted-type
    /// authority list: no listed type is missing a driver, no driver serves a
    /// type outside the list, and every provider drains through the base in
    /// the reverse of the order it started.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
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
    /// assembled in since the family moves landed, with the registered
    /// families first in the generated table's declaration order; drain is
    /// its mirror.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread")]
    async fn providers_start_in_the_committed_order_and_drain_in_reverse() {
        let (_dir, inputs, _readiness) = test_inputs();
        let runtime = ResourcePlaneV3::start_providers(&inputs).await.expect("providers");
// The registered families start first, in the generated table's
        // declaration order; the families wired below the table keep the
        // order the composition root has assembled them in since the moves
        // landed. Deriving the registered prefix from the table keeps this
        // pin authoritative: a family that moves into the table starts at
        // the front in crate-name order without a hand-maintained edit
        // here, and the composed order still fails the assert if the
        // composition diverges from the table.
        let mut expected: Vec<&'static str> = PROVIDER_REGISTRATIONS
            .iter()
            .map(|registration| registration.provider_ref)
            .collect();
        expected.extend([
            "telemetry-service",
            "telemetry-binding",
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
        ]);
        assert_eq!(runtime.startup_order(), expected);
        runtime.drain().await.expect("the providers drain");
        let mut reversed = runtime.startup_order().to_vec();
        reversed.reverse();
        assert_eq!(runtime.drain_order(), reversed);
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
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread")]
    async fn committed_provider_identities_publish_before_the_manager_starts() {
        let (_dir, mut inputs, _readiness) = test_inputs();
        let provider_uid =
            ResourceUid::parse("123e4567-e89b-42d3-a456-426614174010").expect("provider uid");
        let provider_generation =
            d2b_contracts_resource::v3::ResourceGeneration::new(4).expect("generation");
        inputs.committed_provider_identities = BTreeMap::from([(
            ResourceRef::parse(d2b_provider_network_local::NETWORK_PROVIDER_REF).expect("provider ref"),
            (provider_uid.clone(), provider_generation),
        )]);
        let registry = Arc::clone(&inputs.registry);
        let plane = ResourcePlaneV3::open(inputs).await.expect("plane");
        let source = &*registry;
        assert_eq!(
            source.committed_provider_identity(
                &ResourceRef::parse(d2b_provider_network_local::NETWORK_PROVIDER_REF).expect("provider ref")
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
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
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
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
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
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
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
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
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
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
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
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
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
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread")]
    async fn controller_committed_process_child_reaches_the_process_driver() {
        let (_dir, mut inputs, _readiness) = test_inputs();
        // The plane's canonical Process effects double: the shared recording
        // fake, kept with a successful one-shot launch (the old plane fake's
        // launch always succeeded), so the committed row is observed at the
        // driver's launch effect.
        let effects = Arc::new(d2b_provider_process::test_support::FakeFacets::new(
            Default::default(),
        ));
        effects.set_active(false);
        inputs.process_facets = effects.facet_set();
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
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
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

        // Both owners' rows really are committed in the Zone...
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

        //... and the session-scoped relist read answers only its own child.
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
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread")]
    async fn guest_control_endpoints_are_realized_with_the_committed_vmm_process() {
        let (_dir, mut inputs, _readiness) = test_inputs();
        // The plane's canonical Process effects double adopts the committed
        // VMM row after the first launch (`Absent` comes first from the
        // default queue, then the scripted `Adopted` report): the driver's
        // `Ready` - and only `Ready` - publishes the evidence row, exactly
        // as the production provider's retained identity does.
        let effects = Arc::new(d2b_provider_process::test_support::FakeFacets::new(
            Default::default(),
        ));
        effects.set_active(false);
        effects.push_adoption(d2b_provider_process::ProviderAdoption::Adopted(
            adopted_report(),
        ));
        inputs.process_facets = effects.facet_set();
        let zone = ZoneId::parse("test").expect("zone");
        let planes: Arc<tokio::sync::Mutex<HashMap<String, Arc<ResourcePlaneV3>>>> =
            Arc::new(tokio::sync::Mutex::new(HashMap::new()));
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
           .await
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
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
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
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
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
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
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
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
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

    // ---------------------------------------------------------------------
    // Anchor projection subscription
    // ---------------------------------------------------------------------

    /// A published Volume notice sets the pending flag and, after the drain,
    /// performs exactly one re-materialization.

    #[tokio::test(flavor = "multi_thread")]
    async fn a_volume_notice_sets_the_pending_flag_and_drains_into_one_rematerialization() {
        let rig = anchor_subscription_rig();
        let key = ResourceKey::new("test", "Volume", "state");
        commit_volume_row(&rig.store, &key).await;
        let state = Arc::new(AnchorSubscriptionState::default());
        let task = spawn_subscription(&rig, &state, rig.hub.snapshot_revision());
        rig.hub
           .publish(ChangeNotice {
                key: key.clone(),
                kind: ChangeKind::Upsert,
                source: ChangeSource::Desired,
            })
           .await;
        // The notice sets the pending flag. The flag is cleared as soon as
        // the drain acts, so the deterministic observable is the set count.
        wait_for(|| state.pending_sets.load(Ordering::Relaxed) >= 1).await;
        // One drain performs exactly one re-materialization, not more within
        // the following windows.

        wait_for(|| state.rematerializations.load(Ordering::Relaxed) == 1).await;
        tokio::time::sleep(ANCHOR_DRAIN_WINDOW * 3).await;
        assert_eq!(state.rematerializations.load(Ordering::Relaxed), 1);
        // The projection reflects the committed row.

        let uid = resource_uid(&d2b_resource_runtime::manager::deterministic_uid(&key)).expect("uid");
        assert!(rig.registry.lookup_anchor(&uid).is_some(), "the projection reflects the committed row");
        task.abort();
    }

    /// A commit published between the initial materialization and the
    /// subscription handoff is not lost: it appears in the registration's
    /// retained replay and the projection reflects it once the replay is
    /// drained (R1).
    #[tokio::test(flavor = "multi_thread")]
    async fn a_commit_between_the_initial_load_and_the_handoff_is_not_lost() {
        let rig = anchor_subscription_rig();
        // The registration's snapshot revision pairs the subscription with the
        // initial materialization: the anchor is taken before the load, so
        // the replay covers everything after it.
        let anchor = rig.hub.snapshot_revision();
        rig.registry.load_from_store(&rig.zone_token, &rig.store).await.expect("initial load");
        // A commit between the initial load and the subscription handoff.

        let key = ResourceKey::new("test", "Volume", "state");
        commit_volume_row(&rig.store, &key).await;
        rig.hub
           .publish(ChangeNotice {
                key: key.clone(),
                kind: ChangeKind::Upsert,
                source: ChangeSource::Desired,
            })
           .await;
        let state = Arc::new(AnchorSubscriptionState::default());
        let task = spawn_subscription(&rig, &state, anchor);
        // The commit appears in the registration's retained replay, and the
        // projection reflects it once the replay is drained.



        let uid = resource_uid(&d2b_resource_runtime::manager::deterministic_uid(&key)).expect("uid");
        wait_for(|| rig.registry.lookup_anchor(&uid).is_some()).await;
        task.abort();
    }

    /// A burst of Volume and VolumeBinding notices drains into one bounded
    /// re-materialization, not one per notice (AE3): every row of the burst
    /// lands in the projection, and the projection needed strictly fewer
    /// re-materializations than there were notices.
    ///
    /// The coalescing claim is asserted on the observable outcome - the
    /// projection reflecting the whole burst after a bounded drain count -
    /// never on how long the burst took. The drain window coalesces whatever
    /// the stream delivers inside it, so how many windows a burst spans is
    /// the delivery latency's, not the drain's; pinning a fixed count after
    /// a fixed sleep would read the machine rather than the code.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_burst_of_volume_and_binding_notices_drains_into_one_rematerialization() {
        let rig = anchor_subscription_rig();
        let volume_keys: Vec<ResourceKey> = (0..4)
           .map(|i| ResourceKey::new("test", "Volume", format!("vol-{i}")))
           .collect();
        let binding_key = ResourceKey::new("test", "VolumeBinding", "binding-0");
        let notices = volume_keys.len() + 1;
        for key in &volume_keys {
            commit_volume_row(&rig.store, key).await;
        }
        commit_binding_row(&rig.store, &binding_key).await;
        let state = Arc::new(AnchorSubscriptionState::default());
        let task = spawn_subscription(&rig, &state, rig.hub.snapshot_revision());
        // A burst of Volume and VolumeBinding notices in one pass.
        for key in volume_keys.iter().chain(std::iter::once(&binding_key)) {
            rig.hub
               .publish(ChangeNotice {
                    key: key.clone(),
                    kind: ChangeKind::Upsert,
                    source: ChangeSource::Desired,
                })
               .await;
        }
        // Wait for the burst's observable outcome - the projection reflecting
        // every row, by at least one drain - never for a count within a
        // wall-clock window. A Volume row's projection is its anchor; a
        // VolumeBinding row's projection is its socket target.
        let binding_socket = {
            let stored = StoredBinding::new(
                serde_json::from_slice(&serde_json::to_vec(&binding_spec()).expect("binding spec"))
                   .expect("binding spec"),
                resource_uid(&d2b_resource_runtime::manager::deterministic_uid(&binding_key))
                   .expect("uid"),
                d2b_contracts_resource::v3::ResourceGeneration::new(1).expect("generation"),
                ZoneRevision::new(0),
            );
            stored.socket_identity(&rig.zone_token)
        };
        wait_for(|| {
            state.rematerializations.load(Ordering::Relaxed) >= 1
                && volume_keys.iter().all(|key| {
                    let uid = resource_uid(
                        &d2b_resource_runtime::manager::deterministic_uid(key),
                    )
                    .expect("uid");
                    rig.registry.lookup_anchor(&uid).is_some()
                })
        })
       .await;
        // The binding's projection is its socket target; poll it with the
        // same condition-wait (bounded, not a fixed window).
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while !rig
            .registry
            .lookup_socket_target_by_identity(&binding_socket)
            .await
            .is_some()
        {
            assert!(
                tokio::time::Instant::now() < deadline,
                "the burst's binding target not projected within the bounded wait"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        // Coalesced, not one per notice (AE3): the burst drained in strictly
        // fewer re-materializations than there were notices. A drain that
        // re-materialized once per notice would equal the notice count, so
        // this bound is the invariant that fails on the un-coalesced path and
        // cannot read how long the burst took.
        assert!(
            state.rematerializations.load(Ordering::Relaxed) < notices as u64,
            "a burst drains into one bounded re-materialization, not one per notice"
        );
        task.abort();
    }

    /// A notice for a type the selector does not cover leaves the pending
    /// flag clear.

    #[tokio::test(flavor = "multi_thread")]
    async fn a_notice_for_an_uncovered_type_leaves_the_pending_flag_clear() {
        let rig = anchor_subscription_rig();
        let state = Arc::new(AnchorSubscriptionState::default());
        let task = spawn_subscription(&rig, &state, rig.hub.snapshot_revision());
        // A Process row lies outside the Volume/VolumeBinding selector,so
        // the hub filters it before the stream and never sets the flag.

        rig.hub
           .publish(ChangeNotice {
                key: ResourceKey::new("test", "Process", "worker"),
                kind: ChangeKind::Upsert,
                source: ChangeSource::Desired,
            })
           .await;
        tokio::time::sleep(ANCHOR_DRAIN_WINDOW * 2).await;
        assert!(!state.pending.load(Ordering::Relaxed));
        assert_eq!(state.rematerializations.load(Ordering::Relaxed), 0);
        task.abort();
    }

    /// A terminal missed-data delivery causes a relist, a fresh
    /// registration, a replay drain, and the subscription continues
    /// (AE4).
    #[tokio::test(flavor = "current_thread")]
    async fn a_terminal_missed_delivery_relists_and_the_subscription_continues() {
        let rig = anchor_subscription_rig_with(WatchHubConfig {
            ring_capacity: 32,
            delivery_buffer: 1,
        });
        let key_b = ResourceKey::new("test", "Volume", "b");
        commit_volume_row(&rig.store, &key_b).await;
        let key_c = ResourceKey::new("test", "Volume", "c");
        commit_volume_row(&rig.store, &key_c).await;
        let state = Arc::new(AnchorSubscriptionState::default());
        let task = spawn_subscription(&rig, &state, rig.hub.snapshot_revision());
        // The subscription is live before the burst.



        let mut live = false;
        for _ in 0..50 {
            if rig.hub.subscriber_count().await == 1 {
                live = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(live, "the subscription registered");
        // A one-deep delivery buffer forces a Missed on the second back-to-back
        // publish: the subscription task cannot run between them on this
        // single-threaded runtime.



        rig.hub
           .publish(ChangeNotice {
                key: key_b.clone(),
                kind: ChangeKind::Upsert,
                source: ChangeSource::Desired,
            })
           .await;
        rig.hub
           .publish(ChangeNotice {
                key: key_c.clone(),
                kind: ChangeKind::Upsert,
                source: ChangeSource::Desired,
            })
           .await;
        // The terminal missed-data delivery caused a relist, a fresh
        // registration, and both rows are reflected (row b is durable, row c
        // is durable, and the interval after the Missed is replayed).



        wait_for(|| state.relists.load(Ordering::Relaxed) >= 1).await;
        for key in [&key_b, &key_c] {
            let uid = resource_uid(&d2b_resource_runtime::manager::deterministic_uid(key)).expect("uid");
            assert!(rig.registry.lookup_anchor(&uid).is_some(), "relist reflects row {key}");
        }
        // The subscription continues: a later notice is still drained.


        let key_d = ResourceKey::new("test", "Volume", "d");
        commit_volume_row(&rig.store, &key_d).await;
        rig.hub
           .publish(ChangeNotice {
                key: key_d.clone(),
                kind: ChangeKind::Upsert,
                source: ChangeSource::Desired,
            })
           .await;
        let uid_d = resource_uid(&d2b_resource_runtime::manager::deterministic_uid(&key_d)).expect("uid");
        wait_for(|| rig.registry.lookup_anchor(&uid_d).is_some()).await;
        task.abort();
    }

    /// An expired registration causes the same recovery path and does not end
    /// the subscription.



    #[tokio::test(flavor = "multi_thread")]
    async fn an_expired_registration_relists_and_does_not_end_the_subscription() {
        let rig = anchor_subscription_rig_with(WatchHubConfig {
            ring_capacity: 4,
            delivery_buffer: 16,
        });
        // The anchor is taken before the ring churns past it, so the
        // registration is Expired.


        let anchor = rig.hub.snapshot_revision();
        let keys: Vec<ResourceKey> = (0..8)
           .map(|i| ResourceKey::new("test", "Volume", format!("vol-{i}")))
           .collect();
        for key in &keys {
            commit_volume_row(&rig.store, key).await;
        }
        for key in &keys {
            rig.hub
               .publish(ChangeNotice {
                    key: key.clone(),
                    kind: ChangeKind::Upsert,
                    source: ChangeSource::Desired,
                })
               .await;
        }
        let state = Arc::new(AnchorSubscriptionState::default());
        let task = spawn_subscription(&rig, &state, anchor);
        // The recovery relists from the handed-over snapshot revision and the
        // type-scoped reload rebuilds the projection from the store.


        wait_for(|| state.relists.load(Ordering::Relaxed) >= 1).await;
        for key in &keys {
            let uid = resource_uid(&d2b_resource_runtime::manager::deterministic_uid(key)).expect("uid");
            assert!(rig.registry.lookup_anchor(&uid).is_some(), "relist rebuild reflects row {key}");
        }
        // The subscription continues: a later notice is acted on.



        let later = ResourceKey::new("test", "Volume", "later");
        commit_volume_row(&rig.store, &later).await;
        rig.hub
           .publish(ChangeNotice {
                key: later.clone(),
                kind: ChangeKind::Upsert,
                source: ChangeSource::Desired,
            })
           .await;
        let uid_later = resource_uid(&d2b_resource_runtime::manager::deterministic_uid(&later)).expect("uid");
        wait_for(|| rig.registry.lookup_anchor(&uid_later).is_some()).await;
        assert!(
            state.rematerializations.load(Ordering::Relaxed) >= 1,
            "the subscription continues after the expired registration"
        );
        task.abort();
    }

    /// The registry rebuild after a relist reflects the durable rows,
    /// including a row committed while the subscription was between streams.



    #[tokio::test(flavor = "current_thread")]
    async fn a_relist_rebuild_reflects_durable_rows_including_one_committed_between_streams() {
        let rig = anchor_subscription_rig_with(WatchHubConfig {
            ring_capacity: 32,
            delivery_buffer: 1,
        });
        let keys: Vec<ResourceKey> = ["b", "c", "between"]
           .iter()
           .map(|name| ResourceKey::new("test", "Volume", *name))
           .collect();
        for key in &keys[..2] {
            commit_volume_row(&rig.store, key).await;
        }
        let state = Arc::new(AnchorSubscriptionState::default());
        let task = spawn_subscription(&rig, &state, rig.hub.snapshot_revision());
        let mut live = false;
        for _ in 0..50 {
            if rig.hub.subscriber_count().await == 1 {
                live = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(live, "the subscription registered");
        // A one-deep delivery buffer forces a Missed on the second back-to-back
        // publish, ending the live stream. Then a row is committed with no
        // notice: only the recovery's type-scoped reload can register it,
        // and its ensure is queued before that reload's list, so the row is
        // visible to the store read (committed between streams).



        rig.hub
           .publish(ChangeNotice {
                key: keys[0].clone(),
                kind: ChangeKind::Upsert,
                source: ChangeSource::Desired,
            })
           .await;
        rig.hub
           .publish(ChangeNotice {
                key: keys[1].clone(),
                kind: ChangeKind::Upsert,
                source: ChangeSource::Desired,
            })
           .await;
        commit_volume_row(&rig.store, &keys[2]).await;
        wait_for(|| state.relists.load(Ordering::Relaxed) >= 1).await;
        for key in &keys {
            let uid = resource_uid(&d2b_resource_runtime::manager::deterministic_uid(key)).expect("uid");
            assert!(rig.registry.lookup_anchor(&uid).is_some(), "the relist rebuild reflects row {key}");
        }
        task.abort();
    }

    /// A status-source notice for a Volume row does not set the pending
    /// flag, so only durable changes trigger a re-materialization.



    #[tokio::test(flavor = "multi_thread")]
    async fn a_status_source_notice_does_not_set_the_pending_flag() {
        let rig = anchor_subscription_rig();
        let key = ResourceKey::new("test", "Volume", "state");
        commit_volume_row(&rig.store, &key).await;
        let state = Arc::new(AnchorSubscriptionState::default());
        let task = spawn_subscription(&rig, &state, rig.hub.snapshot_revision());
        // A status transition for a Volume row arrives on the same
        // subscription (the selector matches the key alone) but must not
        // trigger a drain.



        rig.hub
           .publish(ChangeNotice {
                key: key.clone(),
                kind: ChangeKind::Upsert,
                source: ChangeSource::RuntimeStatus,
            })
           .await;
        tokio::time::sleep(ANCHOR_DRAIN_WINDOW * 2).await;
        assert!(!state.pending.load(Ordering::Relaxed));
        assert_eq!(state.rematerializations.load(Ordering::Relaxed), 0);
        task.abort();
    }

    /// A sustained stream that never empties still performs a re-materialization
    /// within the bounded window, and the stall line is logged.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "current_thread")]
    async fn a_sustained_stream_still_rematerializes_within_the_bounded_window() {
        let rig = anchor_subscription_rig();
        let key = ResourceKey::new("test", "Volume", "state");
        commit_volume_row(&rig.store, &key).await;
        let state = Arc::new(AnchorSubscriptionState::default());
        let captured = Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
        let writer = CapturedWriter(Arc::clone(&captured));
        let subscriber = tracing_subscriber::fmt()
           .with_writer(writer)
           .with_max_level(tracing::Level::WARN)
           .with_ansi(false)
           .finish();
        // The drain task must run on this thread for the thread-local
        // subscriber to see its warning, so this test uses the current-thread
        // runtime and holds the guard for its whole body.
        let guard = tracing::subscriber::set_default(subscriber);
        let task = tokio::spawn(run_anchor_subscription(
            Arc::clone(&rig.hub),
            anchor_projection_selector(),
            Arc::clone(&rig.registry),
            Arc::clone(&rig.store),
            rig.zone_token.clone(),
            Arc::clone(&state),
            rig.hub.snapshot_revision(),
            ANCHOR_DRAIN_WINDOW,
        ));
        // A stream that never empties: publish a matching notice every few
        // milliseconds, faster than the bounded drain window.



        let publisher_hub = Arc::clone(&rig.hub);
        let publisher_key = key.clone();
        let publisher = tokio::spawn(async move {
            loop {
                publisher_hub
                   .publish(ChangeNotice {
                        key: publisher_key.clone(),
                        kind: ChangeKind::Upsert,
                        source: ChangeSource::Desired,
                    })
                   .await;
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        });
        // The bounded window still re-materializes under sustained traffic.


        wait_for(|| {
            state.rematerializations.load(Ordering::Relaxed)
                >= ANCHOR_BUSY_DRAINS_BEFORE_WARN
        })
        .await;
        publisher.abort();
        task.abort();
        // The stall line was logged: a stalled consumer is observable.


        let lines = String::from_utf8(captured.lock().expect("capture lock").clone()).expect("utf-8"); // async-gate-allow: synchronous lock acquisition, no await while the guard is held
        assert!(
            lines.contains("anchor projection drain window passed"),
            "stall line logged with the sustained stream: {lines}"
        );
        drop(guard);
    }

    /// A restart abandons the old registration and relists from a fresh one
    /// anchored after the commit it must heal, rather than resuming a stale
    /// cursor: a row committed in the window between the restart's reload and
    /// its fresh registration still resolves.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread")]
    async fn a_restart_relists_instead_of_resuming_a_stale_cursor() {
        let rig = anchor_subscription_rig();
        let state = Arc::new(AnchorSubscriptionState::default());
        let first = ResourceKey::new("test", "Volume", "first");
        commit_volume_row(&rig.store, &first).await;
        let task = spawn_subscription(&rig, &state, rig.hub.snapshot_revision());
        rig.hub
            .publish(ChangeNotice {
                key: first.clone(),
                kind: ChangeKind::Upsert,
                source: ChangeSource::Desired,
            })
            .await;
        let first_uid =
            resource_uid(&d2b_resource_runtime::manager::deterministic_uid(&first)).expect("uid");
        wait_for(|| rig.registry.lookup_anchor(&first_uid).is_some()).await;
        // The subscription is stopped: nothing covers the stream from here.
        task.abort();
        // A restart anchors the fresh registration BEFORE the reload, so a
        // commit published after the anchor is replayed into the fresh
        // registration and a commit read by the reload is covered by it. A
        // row that is in neither would stay unregistered.
        let anchor = rig.hub.snapshot_revision();
        let reloaded = reload_anchor_rows(&rig.registry, &rig.zone_token, &rig.store).await;
        assert!(reloaded, "the restart's reload read every row set");
        let window_row = ResourceKey::new("test", "Volume", "window");
        commit_volume_row(&rig.store, &window_row).await;
        rig.hub
            .publish(ChangeNotice {
                key: window_row.clone(),
                kind: ChangeKind::Upsert,
                source: ChangeSource::Desired,
            })
            .await;
        let restarted = spawn_subscription(&rig, &state, anchor);
        let window_uid = resource_uid(&d2b_resource_runtime::manager::deterministic_uid(&window_row))
            .expect("uid");
        wait_for(|| rig.registry.lookup_anchor(&window_uid).is_some()).await;
        restarted.abort();
    }

    /// The projection's law: a Volume row committed through the manager client
    /// resolves with no writer-side refresh call and no bridge, because the
    /// manager's own Desired notice drives the projection. The registration is
    /// eventual by construction, which is why a resolution that races a commit
    /// must classify its miss as retryable - this test waits for the anchor the
    /// retry finds.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread")]
    async fn an_api_applied_volume_resolves_through_the_projection() {
        let (_dir, inputs, _readiness) = test_inputs();
        let plane = ResourcePlaneV3::prepare(inputs).await.expect("plane prepare");
        plane.complete_initial_load().await.expect("initial load");
        let key = ResourceKey::new("test", "Volume", "api-applied");
        let uid =
            resource_uid(&d2b_resource_runtime::manager::deterministic_uid(&key)).expect("uid");
        assert!(
            plane.registry().lookup_anchor(&uid).is_none(),
            "no anchor is registered before the commit"
        );
        plane
           .client()
           .apply(
                d2b_resource_runtime::manager::MutationSubject {
                    principal: "test".to_owned(),
                    origin: d2b_resource_runtime::identity::ResourceProvenance::Api,
                },
                DesiredResource {
                    key: key.clone(),
                    spec: serde_json::to_vec(
                        &serde_json::json!({ "providerRef": "Provider/volume-local" }),
                    )
                   .expect("volume spec"),
                    metadata: br#"{"annotations":{},"labels":{},"ownerRef":null}"#.to_vec(),
                    provenance: d2b_resource_runtime::identity::ResourceProvenance::Api,
                },
            )
           .await
           .expect("the api apply commits");
        wait_for(|| plane.registry().lookup_anchor(&uid).is_some()).await;
        plane.shutdown().await;
    }


    /// The controller child-mutation bridge route: a Volume row committed
    /// through the bridge resolves with no refresh call from the bridge - the
    /// manager's own Desired notice drives the projection. A resolution that
    /// races the commit misses, so the row's actor must treat that miss as
    /// retryable rather than permanent; the eventual registration this test
    /// waits for is what makes the retry converge.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread")]
    async fn a_bridge_committed_volume_resolves_without_the_bridge_refresh() {
        let (_dir, inputs, _readiness) = test_inputs();
        let plane = Arc::new(ResourcePlaneV3::open(inputs).await.expect("plane"));
        plane
           .ingest_nix_bundle(&test_bundle(vec![bundle_row(
                "Guest",
                "acceptance-guest",
                serde_json::json!({"systemArtifactId": "acceptance-system"}),
            )]))
           .await
           .expect("owner ingest");
        let zone = ZoneId::parse("test").expect("zone");
        let target = ResourceRef::parse("Volume/bridge-committed").expect("target");
        let envelope = serde_json::to_vec(&serde_json::json!({
            "apiVersion": "resources.d2bus.org/v3",
            "type": "Volume",
            "metadata": {
                "name": "bridge-committed",
                "zone": "test",
                "ownerRef": "Guest/acceptance-guest",
                "finalizers": [],
                "deletionRequestedAt": null,
                "createdAt": "1970-01-01T00:00:00.000Z",
                "updatedAt": "1970-01-01T00:00:00.000Z",
                "generation": 1,
                "revision": 1,
                "managedBy": "controller",
            },
            "spec": { "providerRef": "Provider/volume-local" },
            "status": { "observedGeneration": 0, "phase": "Pending", "conditions": [], "resource": {} },
        }))
       .expect("child envelope");
        let port = crate::resource_runtime::plane_controller_bridge::PlaneChildMutations::new(
            Arc::clone(&plane),
            zone,
            ResourceRef::parse("Guest/acceptance-guest").expect("owner"),
        );
        port.ensure(&target, &envelope)
           .await
           .expect("the bridge commits the volume");
        let key = ResourceKey::new("test", "Volume", "bridge-committed");
        let uid =
            resource_uid(&d2b_resource_runtime::manager::deterministic_uid(&key)).expect("uid");
        wait_for(|| plane.registry().lookup_anchor(&uid).is_some()).await;
        plane.shutdown().await;
    }


    /// A later role-ful anchor replaces an earlier one, so a Volume whose
    /// NixClosure attachment moved heals on its next re-registration, while a
    /// name-only registration never downgrades an anchor that already
    /// carries a role.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread")]
    async fn a_later_role_ful_anchor_replaces_an_earlier_one() {
        let registry = PlaneResourceRegistry::new();
        let uid = [0x42u8; 16];
        let uid_str = resource_uid_string(&uid);
        registry
            .register_volume(
                &uid_str,
                "state",
                VolumeAnchor {
                    volume_name: "state".to_owned(),
                    guest_ref: Some(ResourceRef::parse("Guest/first").expect("guest")),
                    role: Some(ZoneNixClosureVolumeRole::StoreView),
                },
            )
            .await;
        registry
            .register_volume(
                &uid_str,
                "state",
                VolumeAnchor {
                    volume_name: "state".to_owned(),
                    guest_ref: Some(ResourceRef::parse("Guest/second").expect("guest")),
                    role: Some(ZoneNixClosureVolumeRole::StoreView),
                },
            )
            .await;
        let anchor = registry
            .lookup_anchor(&resource_uid(&uid).expect("uid"))
            .expect("anchor");
        assert_eq!(
            anchor.guest_ref.as_ref().map(ResourceRef::to_canonical_string),
            Some("Guest/second".to_owned()),
            "the later attachment replaced the earlier one"
        );
        registry
            .register_volume(
                &uid_str,
                "state",
                VolumeAnchor {
                    volume_name: "state".to_owned(),
                    guest_ref: None,
                    role: None,
                },
            )
            .await;
        let anchor = registry
            .lookup_anchor(&resource_uid(&uid).expect("uid"))
            .expect("anchor");
        assert!(
            anchor.role.is_some(),
            "a name-only registration did not downgrade the anchor"
        );
    }

}
