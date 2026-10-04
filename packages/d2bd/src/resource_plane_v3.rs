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
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use d2b_contracts_broker::broker_wire::{
    BrokerCallerRole, BrokerRequest, BrokerResponse, StoreSyncRequest,
};
use d2b_contracts_resource::v3::{
    AuthoritySubject, AuthoritySubjectKind, ControllerGeneration, ResourceGeneration, ResourceRef,
    ResourceUid, ZoneId, ZoneRevision,
    execution_policy::BoundedToken,
    volume::{SourceKind, VolumeSpec},
    volume_binding::VolumeBindingSpec,
};
use d2b_contracts_zone_session::v3::resource_bundle::{BundleResource, ResourceBundle};
use d2b_core::bundle_resolver::{BundleResolver, ResolvedStoreViewIntent, intent_id_store_view};
use d2b_core::resource_authority::AuthorityRowKind;
use d2b_provider_activation_nixos::{
    ACTIVATION_EFFECTS_SERVICE, ActivationDriverArgs, ActivationEffectFacets,
    ActivationEffectsServiceFactory, activation_descriptor,
};
use d2b_provider_endpoint::{
    CommittedEndpointShapeSource, DeviceWorkerEvidenceSource, ENDPOINT_EFFECTS_SERVICE,
    EndpointDriverArgs, EndpointEffectFacets, EndpointEffectsServiceFactory, EndpointSocketSource,
    GuestControlProducer, GuestVmmEvidenceSource, HostSocketEvidenceSource, RealizationHandle,
    device_worker_purpose, endpoint_descriptor, guest_control_producer,
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
// from the per-crate `registrations.json` declarations and staged under the
// repository's `generated/new-graph/` closure - the one byte the daemon's
// composition root compiles (it composes the table instead of naming
// families). `cargo xtask gen-new-graph` renders it and drift-gates it byte
// for byte; `cargo xtask check-provider-crate-layout --fix` installs it.
include!("../../../generated/new-graph/provider_registrations.rs");
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
use d2b_provider_volume_virtiofs::{SocketIdentity, StoredBinding};
use d2b_resource_api::manager_backend::nix_bundle_subject;
use d2b_resource_runtime::AuthorityPublisher;
use d2b_resource_runtime::context::{ManagerEndpoint, SpecDecoder};
use d2b_resource_runtime::error::ResourceError;
use d2b_resource_runtime::identity::{ResourceKey, ResourceTypeName};
use d2b_resource_runtime::resource::ResourceStatus;
use d2b_resource_runtime::manager::{
    AdmissionDecision, AdmissionOp, DesiredResource, ManagerActorEndpoint, MutationAdmission,
    MutationRequest, MutationSubject, ResourceManager, ResourceManagerArgs,
    ResourceManagerClient, ResourceManagerMsg, ResourceSelector, ResourceView,
};
use d2b_resource_runtime::revision::RuntimeRevision;
use d2b_resource_runtime::spec_store::{SpecSelector, SpecStore, StoredDesiredResource};
use d2b_resource_runtime::GuestTargetControl;
use d2b_resource_runtime::target::{TargetDirectory, TargetRef, TargetResolver};
use d2b_resource_runtime::relations::RelationExtractors;
use d2b_resource_runtime::watch::{
    ChangeSource, DEFAULT_RING_CAPACITY, RevisionExpired, WatchDelivery, WatchHub, WatchRegistration,
    WatchSelector,
};
use d2b_resource_types::DriverDescriptor;
use d2bd_runtime::resource_runtime_support::NewPlaneReadinessState;
use d2bd_runtime::target_runtime::DaemonMode;
use rustix::fs::{Mode, OFlags, ResolveFlags, open, openat2};

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
    DeviceDriverArgs, device_descriptor, effects_service::DEVICE_EFFECTS_SERVICE,
};
use d2b_provider_device_security_key::{
    SecurityKeyDriverArgs, security_key_descriptors,
    effects_service::SECURITY_KEY_EFFECTS_SERVICE,
};
use d2b_provider_device_usbip::{
    UsbipDriverArgs, usbip_descriptors, effects_service::USBIP_EFFECTS_SERVICE,
};

use d2b_provider_network_local::{
    NETWORK_EFFECTS_SERVICE, NetworkDriverArgs, NetworkEffectFacets, NetworkEffectsServiceFactory,
    network_descriptor,
};
use d2b_provider_process_systemd::effects_service::{
    PROCESS_SYSTEMD_EFFECTS_SERVICE, SystemdEffectsServiceFactory,
};
use crate::shared_provider_effects::ProductionSharedProviderEffects;
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
use d2b_provider_execution_policy::execution_policy_descriptor;
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
    AudioMediatorSource, InteractionDriverArgs, InteractionEffectError, InteractionEffectFacets,
    InteractionEffectsService, InteractionIdentitySource, InteractionPlaneRead,
    InteractionSpecEnvelope, WaylandPolicy, spec_decoder, wayland_policy_descriptor,
};
use d2b_provider_wayland_session::{
    DisplayChildRequest, DisplayChildSource, WAYLAND_SESSION_TYPE, WaylandSession,
    wayland_session_descriptor,
};

use d2b_provider_display_wayland::{
    SharedDisplayEndpointVocabulary, WaylandSessionSpec, session_children,
};

/// The `WaylandSession` child-intent source this plane installs (U12, KTD5).
///
/// The display Provider authors one admitted session's children - the two
/// worker Process rows and each worker's private Endpoint - through its own
/// durable derivation, and the manager turns them into child rows. This
/// composition supplies one thing and authors nothing: the derivation is the
/// display Provider's own function over the session's row identity and spec,
/// and the shapes it commits are handed to the display Provider's own
/// vocabulary so the Endpoint family admits them by that Provider's exact
/// match.
///
/// The commit happens BEFORE the intents are returned, so a session's shapes
/// are in the vocabulary by the time the manager holds the rows they describe
/// and the Endpoint actor can be asked to classify them. The child intents are
/// the ones the display Provider derived, unchanged.
///
/// That ordering covers a session admitted while the plane is open. A restart
/// is covered by [`restore_display_endpoint_vocabulary`], which commits the
/// same shapes from the durable rows before the manager spawns anything.
#[derive(Clone)]
struct PlaneDisplayChildSource {
    vocabulary: Arc<SharedDisplayEndpointVocabulary>,
}

impl DisplayChildSource for PlaneDisplayChildSource {
    fn display_children(
        &self,
        request: &DisplayChildRequest<'_>,
    ) -> Result<Vec<d2b_core_controller::OwnedChildIntent>, InteractionEffectError> {
        let refuse = |error: d2b_provider_display_wayland::WorkerEffectError| {
            tracing::warn!(
                provider = d2b_provider_display_wayland::PROVIDER_REF,
                session = %request.session_ref.to_canonical_string(),
                reason = %error,
                "display child derivation failed for wayland session"
            );
            InteractionEffectError::InvalidResource
        };
        let intents = session_children::display_owned_child_intents(
            request.zone,
            request.session_ref,
            request.session_uid,
            request.spec,
            request.process_generation,
        )
        .map_err(refuse)?;
        // The shapes are committed from the SAME derivation the rows were
        // built from, so a session that cannot derive them commits nothing and
        // the rows it did derive are never admitted by this Provider.
        self.vocabulary
            .commit_session(request.session_uid, request.spec)
            .map_err(refuse)?;
        Ok(intents)
    }
}

/// Decode one durable `WaylandSession` row's spec the way that row's own
/// driver opens it: the interaction family's envelope decode, then the typed
/// base spec. The restore and the driver therefore read the same spec out of
/// the same bytes, and the shapes the restore commits are derived from the
/// spec the durable child rows were built from.
fn restore_session_spec(
    decoder: &dyn SpecDecoder,
    spec: &[u8],
) -> Result<WaylandSessionSpec, String> {
    let envelope = decoder
        .decode(spec)
        .map_err(|error| error.to_string())?
        .downcast::<InteractionSpecEnvelope>()
        .map_err(|_| "the row's spec is not an interaction spec envelope".to_owned())?;
    envelope
        .base_spec::<WaylandSessionSpec>()
        .map_err(|error| error.to_string())
}

/// Rebuild the display Provider's committed-shape vocabulary from the durable
/// `WaylandSession` rows this store holds (F5).
///
/// The live admission path commits a session's shapes from
/// [`PlaneDisplayChildSource`] as that session's child intents are derived,
/// which orders the shapes correctly for every session admitted after the
/// plane is open. It orders nothing on a restart: the manager spawns one
/// actor per durable row at once, so an `Endpoint` child row whose
/// `WaylandSession` has not reconciled yet asks a vocabulary holding nothing,
/// is refused `ShapeUnsupported`, and that refusal is terminal - the row never
/// requeues and display readiness never republishes for it.
///
/// The shapes are a pure function of the session's own row uid and its durable
/// spec - the same two values the child intents are derived from - so a
/// restart rebuilds them from the durable rows themselves, into the same
/// registry, before the spawn. A restart therefore no longer depends on
/// reconcile order.
///
/// Every durable row of this Zone contributes, a row already marked deleting
/// included: that row is still a session this Provider commits shapes for, and
/// its children are torn down with it rather than refused for a shape that
/// exists. A row this Provider cannot derive contributes nothing and is named
/// in the log - its own session actor refuses that row on the same derivation,
/// so the restart reproduces the live verdict rather than inventing one. A
/// store that cannot be read at all IS this plane's failure: an unreadable
/// vocabulary is the terminal-refusal window this closes, moved earlier and
/// made loud rather than left to the actors that would hit it first.
async fn restore_display_endpoint_vocabulary(
    store: &SpecStore,
    zone: &ZoneId,
    vocabulary: &SharedDisplayEndpointVocabulary,
) -> Result<usize, PlaneError> {
    let rows = store
        .list(SpecSelector {
            zone: Some(zone.as_str().to_owned()),
            type_name: Some(WAYLAND_SESSION_TYPE.to_owned()),
            owner_uid: None,
        })
        .await?;
    let decoder = spec_decoder();
    let mut committed = 0;
    for row in &rows {
        let uid = match resource_uid(&row.uid) {
            Ok(uid) => uid,
            Err(()) => {
                tracing::warn!(
                    zone = %zone.as_str(),
                    session = %row.key.name.as_str(),
                    "the durable display session's row identity is not UUIDv4-shaped; its endpoint rows stay unadmitted"
                );
                continue;
            }
        };
        let spec = match restore_session_spec(decoder.as_ref(), &row.spec) {
            Ok(spec) => spec,
            Err(reason) => {
                tracing::warn!(
                    zone = %zone.as_str(),
                    session = %row.key.name.as_str(),
                    reason = %reason,
                    "the durable display session's spec does not decode into a committed shape; its endpoint rows stay unadmitted until its own actor refuses the row"
                );
                continue;
            }
        };
        match vocabulary.commit_session(&uid, &spec) {
            Ok(()) => committed += 1,
            Err(error) => {
                tracing::warn!(
                    provider = d2b_provider_display_wayland::PROVIDER_REF,
                    zone = %zone.as_str(),
                    session = %row.key.name.as_str(),
                    reason = %error,
                    "the durable display session derives no committed shape; its endpoint rows stay unadmitted until its own actor refuses the row"
                );
            }
        }
    }
    tracing::info!(
        zone = %zone.as_str(),
        sessions = committed,
        durable_sessions = rows.len(),
        "the display endpoint vocabulary was rebuilt from the zone's durable sessions before the manager spawned"
    );
    Ok(committed)
}

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
        zone: inputs.zone.clone(),
        controller_generation: inputs.authority.controller_generation,
        effects: Arc::new(InteractionEffectsService::new(
            inputs.interaction_facets.clone(),
        )),
        behavior,
    }
}

/// Preserved reconcile backoff for the plane's resource actors (R13).
const PLANE_BACKOFF: Duration = d2b_resource_runtime::DEFAULT_REQUEUE_BACKOFF;

/// The whole-exchange budget one authority publication round trip gets. It
/// matches the coordinator the providers already publish their bootstrap
/// authority over, so one Zone presents one bound to the broker.
const AUTHORITY_PUBLICATION_ROUND_TRIP: Duration = Duration::from_secs(20);

/// The identity every authority publication of this daemon runs under.
///
/// It is the verified deployment identity this daemon authenticates as, never
/// the row a candidate mutates: a publisher that presented a candidate's own
/// identity would be letting a row authorize its own introduction.
fn authority_publication_subject() -> AuthoritySubject {
    AuthoritySubject::unresourced(AuthoritySubjectKind::Bootstrap)
}

/// The id a `User/<name>` or `Group/<name>` principal resolves to.
///
/// The closed Volume contract requires these principals to be real host
/// accounts, resolved through NSS: `build_tpm_state_volume_spec` documents
/// that "each principal must be a real host account the state-layout effect
/// resolves through NSS", and `host-users.nix` materializes exactly those
/// accounts from `d2bLib.deviceTpmPrincipals`, with the uid the worker row
/// actually runs as - the id the host pinned on that account, which the
/// runtime now reads back through the same NSS lookup rather than deriving a
/// second time (`mint_template_intent`, `d2b-core`). A name that does not
/// resolve is a provisioning gap, not something to paper over.
///
/// An earlier revision fell back to a name-derived stable id here. That was
/// wrong twice over: the accounts do exist, so the fallback never ran; and on
/// the path it was meant to cover it would have granted a uid no process ever
/// holds, so a missing account would have become a silent permission grant
/// rather than the loud refusal the contract wants. It is reverted.
fn principal_id_for(
    name: &str,
    group: bool,
) -> Result<u32, d2b_provider_volume_local::VolumeLocalError> {
    let id = if group {
        nix::unistd::Group::from_name(name).map(|entry| entry.map(|g| g.gid.as_raw()))
    } else {
        nix::unistd::User::from_name(name).map(|entry| entry.map(|u| u.uid.as_raw()))
    };
    id.map_err(|_| d2b_provider_volume_local::VolumeLocalError::EffectFailed)?
        .ok_or(d2b_provider_volume_local::VolumeLocalError::EffectFailed)
}

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
    /// Construct an empty registry (equivalent to `Default`).
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
            let uid = resource_uid_string(&row.uid);
            // Log after the insert, not before: an earlier placement made a
            // registration look like it had already landed while the write
            // was still queued behind the registry lock, and the resulting
            // log ordering read as "registered, then missed".
            registry
               .register_volume(&uid, &row.key.name, volume_anchor_from_row(row))
               .await;
            tracing::info!(
                zone = %zone_token.as_str(),
                volume = %row.key.name.as_str(),
                uid = %uid,
                "anchor projection registered a Volume"
            );
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

/// The private virtiofs socket path for one (volume, guest) serving pair.
///
/// The derivation itself belongs to the Provider that owns the frozen v1
/// worker socket contract, and both sides of one relationship go through
/// it: the daemon's serving launch composes `--socket-path` from the same
/// function this probe, presence check, and removal use, so the socket the
/// worker binds and the socket this surface waits for cannot be two
/// different paths.
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
    let volume = BoundedToken::parse(volume_ref.name().as_str().to_owned()).ok()?;
    let guest = BoundedToken::parse(execution_ref.name().as_str().to_owned()).ok()?;
    d2b_provider_volume_virtiofs::derive_serving_socket_path(
        socket_runtime_dir,
        zone,
        &volume,
        &guest,
    )
    .ok()
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

/// Production Endpoint socket surface (the local Unix virtiofsd case):
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

/// The daemon's private host observation: the exact socket standing behind one
/// committed endpoint, and the minted handle that names it (KTD5, KTD8).
///
/// A host socket realization is daemon state. The daemon knows the locator the
/// endpoint owner committed - through the same socket-target registry the
/// host socket facet resolves over - the exact socket that locator currently
/// stands for, and whether that socket accepts a connection. None of that
/// crosses the provider boundary: the only value that leaves is a
/// [`RealizationHandle`], and only for an observation that proved all three.
///
/// The three ways a socket can fail to be the one this endpoint named -
/// nothing bound at the locator, something bound that does not accept a
/// connection, and a socket replaced under the same locator - are ONE answer.
/// They read the same way to a consumer, so the facet answers the same way and
/// the exact socket identity stays inside the daemon that compared it. An
/// endpoint the daemon holds no committed locator for is the same answer: the
/// daemon privately observed no exact socket, so the shape stays unrealized
/// rather than reporting a readiness nothing proved.
///
/// The handle is minted from fresh randomness, never from the locator, the
/// device, or the inode: the `(dev, ino)` pair is what decides whether a
/// socket was REPLACED, and a token derived from it could be recomputed by
/// anyone who read one. It is minted when the socket becomes current, kept
/// while that same socket stands (a pass that re-observes the same
/// realization must not invalidate a dependent that read the earlier token),
/// and re-minted at a higher rotation when the socket behind the endpoint is
/// replaced. The whole table lives in this process, so a daemon restart
/// re-mints every handle from fresh randomness (KTD8).
#[derive(Clone)]
struct PlaneHostSocketEvidence {
    registry: Arc<PlaneResourceRegistry>,
    socket_runtime_dir: PathBuf,
    zone_token: BoundedToken,
    minted: Arc<tokio::sync::Mutex<MintedHostSockets>>,
}

/// The handles this daemon has minted, and the rotation counter they were
/// minted at.
///
/// The counter is this process's own: it starts at zero on every daemon start
/// and moves once per handle minted, so a replacement carries a different
/// rotation than the realization it replaced and a restart mints from scratch.
#[derive(Default)]
struct MintedHostSockets {
    /// Endpoint reference to the exact socket identity the live handle names,
    /// and the handle itself.
    current: BTreeMap<String, ((u64, u64), RealizationHandle)>,
    rotations: u64,
}

impl PlaneHostSocketEvidence {
    fn new(
        registry: Arc<PlaneResourceRegistry>,
        socket_runtime_dir: PathBuf,
        zone_token: BoundedToken,
    ) -> Self {
        Self {
            registry,
            socket_runtime_dir,
            zone_token,
            minted: Arc::new(tokio::sync::Mutex::new(MintedHostSockets::default())),
        }
    }

    /// The locator the endpoint owner committed, resolved exactly as the host
    /// socket facet resolves one.
    async fn path_for(&self, endpoint_ref: &ResourceRef) -> Option<PathBuf> {
        let target = self
            .registry
            .socket_target_by_ref(&self.zone_token, endpoint_ref)
            .await?;
        serving_socket_path(
            &self.socket_runtime_dir,
            &self.zone_token,
            &target.volume_ref,
            &target.execution_ref,
        )
    }

    /// The live handle for `identity`, minting one when the socket behind this
    /// endpoint is not the one the previous handle named.
    async fn handle_for(
        &self,
        endpoint_ref: &ResourceRef,
        identity: (u64, u64),
    ) -> Option<RealizationHandle> {
        let mut minted = self.minted.lock().await;
        let key = endpoint_ref.to_canonical_string();
        if let Some((minted_identity, handle)) = minted.current.get(&key)
            && *minted_identity == identity
        {
            return Some(handle.clone());
        }
        let rotations = minted.rotations + 1;
        let handle = RealizationHandle::mint(realization_nonce()?, rotations)?;
        minted.rotations = rotations;
        minted.current.insert(key, (identity, handle.clone()));
        Some(handle)
    }
}

/// One fresh incarnation nonce: 128 bits of kernel randomness, rendered as the
/// lowercase bounded token a [`RealizationHandle`] takes (KTD8).
///
/// The leading letter is a fixed prefix so the value is a bounded token; the
/// entropy is the 32 hex characters behind it, which is why the handle's own
/// floor of 32 characters is met rather than merely rounded at.
fn realization_nonce() -> Option<BoundedToken> {
    let mut bytes = [0_u8; 16];
    getrandom::getrandom(&mut bytes).ok()?;
    let hex = bytes.iter().map(|byte| format!("{byte:02x}")).collect::<String>();
    BoundedToken::parse(format!("r{hex}")).ok()
}

#[async_trait::async_trait]
impl HostSocketEvidenceSource for PlaneHostSocketEvidence {
    /// The realization standing behind `endpoint_ref`, or `None` when nothing
    /// proved one.
    async fn observe(&self, endpoint_ref: &ResourceRef, _purpose: &str) -> Option<RealizationHandle> {
        let path = self.path_for(endpoint_ref).await?;
        let metadata = tokio::fs::metadata(&path).await.ok()?;
        if !metadata.file_type().is_socket() {
            return None;
        }
        // Connectability is part of the proof, not a nicety: a socket that is
        // bound and refusing every connection publishes the closed
        // `unavailable` state, and the Endpoint driver then publishes no
        // realization at all.
        tokio::net::UnixStream::connect(&path).await.ok()?;
        self.handle_for(endpoint_ref, (metadata.dev(), metadata.ino())).await
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

    /// [`Self::source_unresolved`] carrying the provider's own error code.
    /// Constructing the anchored root reports through `?` without a stage, so
    /// without this the error code that distinguishes "the root would not
    /// stat" from "the marker root would not stat" never reaches a log.
    fn source_unresolved_err(
        &self,
        stage: &'static str,
        volume_uid: &d2b_contracts_resource::v3::ResourceUid,
        error: d2b_provider_volume_local::VolumeLocalError,
    ) -> d2b_provider_volume_local::VolumeLocalError {
        tracing::warn!(
            zone = %self.zone.as_str(),
            volume = %volume_uid.as_str(),
            stage,
            error = ?error,
            "v3 Volume source resolution failed"
        );
        error
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
        // Name the uid that missed, not "?". A registration gap and a
        // uid-representation gap both surface here, and the uid is the only
        // datum that tells them apart - without it the stage name is all a
        // reader has, and this failure is otherwise undiagnosable.
        let Some(anchor) = self.registry.lookup_anchor(volume_uid) else {
            return Err(self.source_unresolved("volume-anchor", volume_uid.as_str()));
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
        // Every other refusal in this function names its stage through
        // `source_unresolved`, but these two propagate bare. A root that will
        // not construct reached the driver as a plain "a provider layout
        // effect failed" with no stage at all, so it was indistinguishable
        // from a layout entry that would not provision.
        d2b_provider_volume_local::ResolvedVolumeRoot::new(file, volume_uid.clone())
            .map_err(|error| self.source_unresolved_err("volume-root-construct", volume_uid, error))?
            .with_marker_root(marker_file)
            .map_err(|error| self.source_unresolved_err("marker-root-construct", volume_uid, error))
    }

    fn resolve_principal(
        &self,
        reference: &ResourceRef,
    ) -> Result<u32, d2b_provider_volume_local::VolumeLocalError> {
        if reference.resource_type().as_str() != "User" {
            return Err(d2b_provider_volume_local::VolumeLocalError::InvalidSpec);
        }
        principal_id_for(reference.name().as_str(), false)
    }

    fn resolve_group(
        &self,
        reference: &ResourceRef,
    ) -> Result<u32, d2b_provider_volume_local::VolumeLocalError> {
        // A layout entry names its group with a `User/<name>` reference as
        // often as a `Group/<name>` one - the closed contract's own fixtures
        // declare `groupRef: "User/d2bd"`. Refusing anything but `Group` here
        // rejected every Volume that spells it that way, which is all of them.
        let kind = reference.resource_type().as_str();
        if kind != "Group" && kind != "User" {
            return Err(d2b_provider_volume_local::VolumeLocalError::InvalidSpec);
        }
        principal_id_for(reference.name().as_str(), kind == "Group")
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
    /// The controller generation for the zone authority's process rows
    /// (KTD7: from the bundle resolver, never the spec store).
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
    /// U31: the verified deployment graph the composition published for this
    /// process, when it published one.
    ///
    /// A family's driver reads the accepted graph to confirm this deployment
    /// published its own compiled implementation before it plans a runner
    /// or dispatches an effect. `None` is the pre-cutover construction; the
    /// cutover makes the publication mandatory in the same step.
    pub deployment_graph:
        Option<std::sync::Arc<d2b_provider_activation_nixos::AcceptedDeploymentGraph>>,
    /// U18: the daemon state a family's privileged broker dispatch is built
    /// over.
    ///
    /// The Endpoint family's delivery reaches the exact-endpoint ACL helpers
    /// over the broker socket, which only the daemon holds. Handing the
    /// driver the daemon's own state keeps the socket, the caller role, and
    /// the broker path daemon-hosted: the family receives a dispatch facet
    /// and never a socket, a path, or a numerical principal (R2).
    pub server_state: Option<std::sync::Arc<crate::ServerState>>,
    /// The daemon-supplied facet set the User family's effects implementation
    /// is built from (U5): the crate's own bounded local-account probe,
    /// supplied through the composition root. Every probe input is host
    /// state the crate reads itself, so the family never receives a
    /// daemon-built effect port (R2).
pub user_facets: UserEffectFacets,
    /// The daemon-supplied facet set the VolumeBinding family's effects
    /// implementation is built from (U6):the serving-socket probe, the
    /// socket removal, and the guest-mount observation, supplied through the
    /// composition root. The family never receives a daemon-built effect
    /// port (R2).
    pub binding_facets: BindingEffectFacets,
    /// The daemon-supplied facet set the Endpoint family's effects
    /// implementation is built from (U6):the host socket surface and the
    /// two row-evidence probes, supplied through the composition root. The
    /// family never receives a daemon-built effect port (R2).
    pub endpoint_facets: EndpointEffectFacets,
    /// The display Provider's Zone-wide committed-shape vocabulary, injected
    /// into the Endpoint family's facet set above (KTD5).
    ///
    /// The display Provider owns the endpoint shapes it commits and every
    /// field of its own exact match; this plane installs that object and adds
    /// nothing to it. The session admission path commits each admitted
    /// session's shapes into it, so a committed display `Endpoint` row is
    /// admitted by the Provider that committed it and by nothing else.
    pub display_endpoint_vocabulary: Arc<SharedDisplayEndpointVocabulary>,
    /// The daemon-supplied facet set the Credential family's effects
    /// implementation is built from (U8):the daemon's Credential runtime
    /// (the preserved Provider and execution-target reads, the lease-facts
    /// read, the managed-identity agent probe, and the authenticated
    /// Provider session handoff registry), supplied through the composition
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
    /// rows, committed Provider identities, and the controller-session
    /// generation)andthe Cloud Hypervisor controller session, supplied
    /// through the composition root. The family never receives a
    /// daemon-built effect port (R2).
    pub guest_facets: GuestEffectFacets,
    /// The daemon-supplied facet sets the device families' effects
    /// implementations are built from (U12): each family's driver never
    /// receives a daemon-built effect port (R2); the family crates serve
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
    /// The broker half of this Zone's durable authority mutations, when a
    /// caller binds one itself.
    ///
    /// `None` is the production composition: the plane binds the coordinator
    /// over its own origination leg and its own store incarnation. A test or
    /// a context-free deployment binds a publisher directly, because a plane
    /// with no fence has no way to commit a desired mutation at all.
    pub authority_publisher: Option<Arc<dyn AuthorityPublisher>>,
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
    /// The Zone's verified Nix bundle, applied before the manager spawns.
    ///
    /// The plane commits the rows this graph reads as authority - the
    /// `Role` and `RoleBinding` rows an accepted graph is built from -
    /// through the same fenced store path the foundation seed uses, so they
    /// are durable, published, and acknowledged before the manager exists
    /// (F1). The manager's own `pre_start` then loads them, so a Zone never
    /// reaches a reader - the broker's per-Zone projection included - as an
    /// empty Zone while its own declared authority is still in flight.
    ///
    /// Every other bundle row keeps arriving through
    /// [`ResourcePlaneV3::ingest_nix_bundle`] after the plane opens: only the
    /// authority rows are read by anything that exists before the manager
    /// spawns, and staging the rest here would move every resource actor's
    /// first reconcile without any boundary being ready to decide it.
    ///
    /// `None` is a plane that was given no bundle to apply, which is the
    /// test shape: a plane with no declared authority commits no authority
    /// row of its own.
    pub bundle: Option<ResourceBundle>,
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
        let zone_token = BoundedToken::parse(zone.as_str().to_owned())
            .map_err(|error| PlaneError::Authority(error.into()))?;
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
                state
                    .provider_runtime
                    .attach_process_providers(Arc::clone(&providers))
                    .map_err(|error| PlaneError::Authority(error.into()))?;
                providers
            }
        };
        let registry = Arc::new(PlaneResourceRegistry::new());
        let controller_generation = ControllerGeneration::new(1)
           .map_err(|error| PlaneError::Authority(error.into()))?;
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
            // U20: the bounded helper leg and the claim port. Both refuse by
            // name and grant nothing, because the daemon has no legitimate
            // source of `BindingAuthorization` or a `FreshnessTuple` at the
            // effects seam yet: the verified deployment graph is per-deployment
            // and this admission is per-Zone. A USB binding whose device
            // cannot be proven is visibly unadmitted rather than served from a
            // fabricated grant.
            admission: Arc::new(d2b_provider_device_usbip::facets::UnwiredUsbipHelperLegs),
            claims: Arc::new(d2b_provider_device_usbip::facets::UnwiredUsbipClaimPorts),
        };
        let security_key_facets =
            d2b_provider_device_security_key::facets::SecurityKeyEffectFacets {
                runtime: Arc::clone(&shared_provider_effects)
                    as Arc<
                        dyn d2b_provider_device_security_key::facets::SecurityKeyRuntime,
                    >,
                // U20: as for USB, the helper leg and claim port refuse by
                // name until the daemon can produce real binding authority.
                admission: Arc::new(
                    d2b_provider_device_security_key::facets::UnwiredSecurityKeyHelperLegs,
                ),
                claims: Arc::new(
                    d2b_provider_device_security_key::facets::UnwiredSecurityKeyClaimPorts,
                ),
            };
        let device_facets = d2b_provider_device::facets::DeviceEffectFacets {
            runtime: Arc::clone(&shared_provider_effects)
                as Arc<dyn d2b_provider_device::facets::DeviceRuntime>,
            inventory: Arc::clone(&shared_provider_effects)
                as Arc<dyn d2b_provider_device::facets::DeviceInventorySource>,
            // U16: the authority evidence a committed DeviceBinding's
            // presence is decided against. The shared provider effects are
            // the daemon's own read of the authority journal and the standing
            // relationships, so the family never reads them itself.
            authority: Arc::clone(&shared_provider_effects)
                as Arc<dyn d2b_provider_device::facets::DeviceBindingAuthoritySource>,
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
        // U6/KTD5: the Endpoint family's provider seam is wired here too. The
        // display Provider owns the endpoint shapes it commits and this plane
        // installs that one object and adds nothing to it, and the private
        // host observation is the daemon's own: it resolves the locator the
        // endpoint owner committed, compares the exact socket standing there,
        // and mints a handle only for an observation that proved it.
        let display_endpoint_vocabulary = Arc::new(SharedDisplayEndpointVocabulary::new());
        let endpoint_facets = EndpointEffectFacets::new(
            Arc::new(PlaneEndpointSocketSource {
                registry: Arc::clone(&registry),
                socket_runtime_dir: endpoint_socket_runtime_dir.clone(),
                zone_token: endpoint_zone_token.clone(),
            }),
            Arc::new(GuestControlEndpointProbe::new(
                Arc::clone(&state.v3_planes),
                zone.clone(),
            )),
            Arc::new(DeviceWorkerEndpointProbe::new(
                Arc::clone(&state.v3_planes),
                zone.clone(),
            )),
        )
        .with_committed_shapes(
            Arc::clone(&display_endpoint_vocabulary) as Arc<dyn CommittedEndpointShapeSource>
        )
        .with_host_socket_observation(Arc::new(PlaneHostSocketEvidence::new(
            Arc::clone(&registry),
            endpoint_socket_runtime_dir.clone(),
            endpoint_zone_token.clone(),
        )) as Arc<dyn HostSocketEvidenceSource>);
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
        deployment_graph: None,
            server_state: None,
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
            display_endpoint_vocabulary: Arc::clone(&display_endpoint_vocabulary),
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
            // Production binds no publisher here: the plane builds the
            // coordinator over its own origination leg and store incarnation.
            authority_publisher: None,
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
            bundle: None,
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
            let Some(factory) = (match service {
                x if x == PROCESS_EFFECTS_SERVICE.id => Some(Arc::new(ProcessEffectsServiceFactory::new(process_facets.clone())) as Arc<dyn EffectServiceFactory>),
                x if x == NETWORK_EFFECTS_SERVICE.id => Some(Arc::new(NetworkEffectsServiceFactory::new(network_facets.clone())) as Arc<dyn EffectServiceFactory>),
                x if x == HOST_EFFECTS_SERVICE.id => Some(Arc::new(HostEffectsServiceFactory::new(host_facets.clone())) as Arc<dyn EffectServiceFactory>),
                x if x == ACTIVATION_EFFECTS_SERVICE.id => Some(Arc::new(ActivationEffectsServiceFactory::new(activation_facets.clone())) as Arc<dyn EffectServiceFactory>),
                x if x == USER_EFFECTS_SERVICE.id => Some(Arc::new(UserEffectsServiceFactory::new(user_facets.clone())) as Arc<dyn EffectServiceFactory>),
                x if x == USBIP_EFFECTS_SERVICE.id => Some(Arc::new(d2b_provider_device_usbip::effects_service::
                    UsbipEffectsServiceFactory::new(usbip_facets.clone())) as Arc<dyn EffectServiceFactory>),
                x if x == SECURITY_KEY_EFFECTS_SERVICE.id => Some(Arc::new(d2b_provider_device_security_key::effects_service::
                    SecurityKeyEffectsServiceFactory::new(security_key_facets.clone())) as Arc<dyn EffectServiceFactory>),
                x if x == DEVICE_EFFECTS_SERVICE.id => Some(Arc::new(d2b_provider_device::effects_service::
                    DeviceEffectsServiceFactory::new(device_facets.clone())) as Arc<dyn EffectServiceFactory>),
                x if x == CREDENTIAL_EFFECTS_SERVICE.id => Some(Arc::new(CredentialEffectsServiceFactory::new(credential_facets.clone())) as Arc<dyn EffectServiceFactory>),
                x if x == VOLUME_EFFECTS_SERVICE.id => Some(Arc::new(VolumeEffectsServiceFactory::new(volume_facets.clone())) as Arc<dyn EffectServiceFactory>),
                x if x == d2b_provider_wayland_policy::INTERACTION_EFFECTS_SERVICE.id => Some(Arc::new(
                    d2b_provider_wayland_policy::InteractionEffectsServiceFactory::new(
                        interaction_facets.clone(),
                    ),
                ) as Arc<dyn EffectServiceFactory>),
                x if x == PROCESS_SYSTEMD_EFFECTS_SERVICE.id => {
                    // U15:the family's service carries no facet set (R2), so
                    // the composition root hosts its factory from crate-owned
                    // constants alone, over the registered service identity - the
                    // family itself is never named here.


                    Some(Arc::new(SystemdEffectsServiceFactory::new()) as Arc<dyn EffectServiceFactory>)
                },
                x if x == GUEST_EFFECTS_SERVICE.id => Some(Arc::new(GuestEffectsServiceFactory::new(guest_facets.clone())) as Arc<dyn EffectServiceFactory>),
                x if x == BINDING_EFFECTS_SERVICE.id => Some(Arc::new(BindingEffectsServiceFactory::new(binding_facets.clone())) as Arc<dyn EffectServiceFactory>),
                x if x == ENDPOINT_EFFECTS_SERVICE.id => Some(Arc::new(EndpointEffectsServiceFactory::new(endpoint_facets.clone())) as Arc<dyn EffectServiceFactory>),
                _ => None,
            }) else {
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
        // U25: the v3 composition reaches the host audio session only
        // through the endpoint relationships the Zone graph admitted. This
        // surface has no admitted audio relationship to hand the mediator
        // yet, so it passes none and the host side reports unavailable
        // rather than reaching for an ambient PipeWire environment.
        let _ = &self.state;
        Some(Box::new(crate::audio_dispatch::DaemonAudioMediator::new(
            vm_name,
            capability,
            None,
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
// the only gate on which types can spawn actors. The plane therefore still
// installs the system-homed write fence, which is the whole policy on the
// unchanged entry point (U34 replaces it with [`GraphMutationAdmission`] and
// deletes the string-subject path atomically).

// ---------------------------------------------------------------------------
// New-graph mutation admission (U6, KTD4)
// ---------------------------------------------------------------------------

/// The manager-boundary admission of the new graph construction (U6, KTD4).
///
/// This is the composition's whole contribution to the decision: it names the
/// Zone, the transport, and the subject the manager boundary already
/// established, and defers every rule to the one pure evaluator in
/// `d2b_core::resource_authority`. It holds no policy of its own, and it never
/// reads a name out of the request to decide anything - `principal` is used
/// only to recover the initiating subject's exact reference, and a request
/// whose principal names no resolvable reference is refused rather than
/// evaluated.
///
/// U40 installed this in place of the system-homed write fence
/// for the [`ResourceManagerMsg::AuthenticatedApply`] entry point, and deletes
/// the unchanged string-subject entry points in the same step.
pub struct GraphMutationAdmission {
    accepted: std::sync::Arc<d2b_core::resource_authority::AcceptedGraph>,
    zone: d2b_contracts_resource::v3::ZoneId,
    transport: d2b_core::resource_authority::TransportIdentity,
}

impl GraphMutationAdmission {
    /// Construct the plane's admission from the prior accepted graph.
    pub fn new(
        accepted: std::sync::Arc<d2b_core::resource_authority::AcceptedGraph>,
        zone: d2b_contracts_resource::v3::ZoneId,
        transport: d2b_core::resource_authority::TransportIdentity,
    ) -> Self {
        Self { accepted, zone, transport }
    }

    /// The prior accepted graph this admission evaluates against.
    pub fn accepted(&self) -> &d2b_core::resource_authority::AcceptedGraph {
        &self.accepted
    }
}

impl MutationAdmission for GraphMutationAdmission {
    fn admit(&self, subject: &MutationSubject, request: &MutationRequest) -> AdmissionDecision {
        use d2b_contracts_resource::v3::{
            AdmissionDecision as GraphDecision, AuthoritySubject, AuthoritySubjectKind,
        };
        use d2b_core::resource_authority::{
            GraphAuthority, GraphMutation, MutationKind, MutationSubjectEvidence,
        };

        // The authenticated entry points render the subject either as an exact
        // reference or as the one of the two bootstrap-class tokens the
        // runtime renders for a subject that names no resource. Anything else
        // is display text an in-process caller wrote, and display text decides
        // nothing.
        let initiating = match d2b_contracts_resource::v3::ResourceRef::parse(&subject.principal)
        {
            Ok(reference) => match
                d2b_resource_runtime::manager::authority_subject_kind(&reference)
            {
                Some(kind) => AuthoritySubject::named(kind, reference.clone()),
                None => {
                    return AdmissionDecision::Deny(format!(
                        "graph admission: {} has no admitted authority class",
                        subject.principal
                    ));
                }
            },
            Err(_) => match subject.principal.as_str() {
                "bootstrap" => AuthoritySubject::unresourced(AuthoritySubjectKind::Bootstrap),
                "operator" => AuthoritySubject::unresourced(AuthoritySubjectKind::Operator),
                // The Nix bundle subject. `nix_bundle_subject`
                // (`d2b-resource-api/src/manager_backend.rs:255`) is the only
                // producer of this spelling, it renders the prefix from a
                // bundle identity rather than from a caller's words, and it
                // pairs the prefix with `ResourceProvenance::Nix`. Requiring
                // BOTH is what makes the prefix unforgeable from the API: an
                // API caller's principal must parse as a `ResourceRef`, and
                // `nix:<identity>` has no `/` so it never can. A subject that
                // spells the prefix without the matching origin is display
                // text and decides nothing.
                _ if subject.origin
                    == d2b_resource_runtime::spec_store::ResourceProvenance::Nix
                    && subject.principal.starts_with("nix:")
                    && subject.principal.len() > "nix:".len() =>
                {
                    AuthoritySubject::unresourced(AuthoritySubjectKind::Bootstrap)
                }
                _ => {
                    return AdmissionDecision::Deny(
                        "graph admission: the caller principal is neither an exact resource \
                         reference nor a bootstrap-class token"
                            .to_owned(),
                    );
                }
            },
        };
        let kind = match request.op {
            AdmissionOp::Ensure => MutationKind::Create,
            AdmissionOp::Remove => MutationKind::Delete,
        };
        // A key the typed reference cannot spell names no row the graph could
        // authorize, so it is refused rather than approximated with a different
        // target.
        let Ok(target) = d2b_contracts_resource::v3::ResourceRef::parse(&format!(
            "{}/{}",
            request.key.type_name, request.key.name
        )) else {
            return AdmissionDecision::Deny(format!(
                "graph admission: {}/{} is not an exact resource reference",
                request.key.type_name, request.key.name
            ));
        };
        let mutation = GraphMutation::new(
            self.zone.clone(),
            MutationSubjectEvidence::new(initiating, self.transport),
            kind,
            target,
        );
        match GraphAuthority::admit_mutation(&mutation, &self.accepted) {
            GraphDecision::Admitted => AdmissionDecision::Allow,
            GraphDecision::Refused { stage, reason } => AdmissionDecision::Deny(format!(
                "graph admission refused at {}: {}",
                serde_json::to_string(&stage).unwrap_or_else(|_| "authorize".to_owned()),
                serde_json::to_string(&reason).unwrap_or_else(|_| "identity-not-authorized".to_owned()),
            )),
        }
    }
}

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
    FoundationSeed(#[from] crate::foundation_seed::SeedError),
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
    ManagerSpawn(#[from] ractor::SpawnErr),
    #[error("manager rpc failed: {0}")]
    ManagerRpc(#[from] ResourceError),
    /// The Zone still owed an outcome for a transaction a previous boot left
    /// outstanding, and recovery could not resolve it. The Zone stays fenced
    /// and the start is refused; nothing is released against authority the
    /// broker has not accepted.
    #[error("zone recovery refused: {0}")]
    ZoneRecovery(String),
    #[error("zone authority inputs invalid: {0}")]
    Authority(#[source] Box<dyn std::error::Error + Send + Sync>),
    #[error("target layer refused: {0}")]
    Target(#[source] Box<dyn std::error::Error + Send + Sync>),
    #[error("bundle invalid: {0}")]
    Bundle(#[from] d2b_contracts_zone_session::v3::resource_bundle::ResourceBundleError),
    /// The authority refused a row the plane applies before its manager
    /// spawns. The plane does not open without it: a Zone whose declared
    /// authority was not accepted is a Zone every authority reader would see
    /// as unestablished.
    #[error("declared authority row refused: {0}")]
    AuthorityPublication(
        #[source] d2b_resource_runtime::authority_publish::PublishError,
    ),
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
///
/// A declared reference that is not an execution TARGET is not an anchor
/// either, and is treated exactly as an absent one is. The target directory's
/// closed vocabulary is `Host/<name>` and `Guest/<name>`, while several
/// binding specs carry an `executionRef` naming the CONSUMER of the
/// relationship instead: an `EndpointBinding` delivers to a `Guest` or a
/// `Process` helper, and that helper is a row of its own with its own anchor.
/// Handing such a reference to the directory cannot place the row - it fails
/// the directory's own parse, the row commits, and no actor is ever spawned
/// for it, so the whole relationship converges for ever behind a deferred
/// answer that never says why. The anchor of the type is the anchor of the
/// row.
struct DeclaredExecutionRef;

impl TargetResolver for DeclaredExecutionRef {
    fn execution_ref(&self, _key: &ResourceKey, spec: &[u8]) -> Option<String> {
        let value: serde_json::Value = serde_json::from_slice(spec).ok()?;
        let reference = value.get("executionRef")?.as_str()?;
        TargetRef::parse(reference).ok()?;
        Some(reference.to_owned())
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
        // U7: the verified deployment graph's authority rides the same
        // origination leg, so the broker socket is the one the
        // trusted-context publication already resolved.
        set = set.with_authority_publication(inputs.trusted_context_publication.as_ref().map(|publication| publication.broker_socket().to_path_buf()));
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
        set = set.with(
            family_declaration("quota"),
            vec![quota_descriptor(inputs.zone.clone())],
        );
        set = set.with(
            family_declaration("emergency-policy"),
            vec![emergency_policy_descriptor(inputs.zone.clone())],
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
        set = set.with(
            family_declaration("operation"),
            vec![operation_descriptor()],
        );
        set = set.with(
            family_declaration("seccomp-profile"),
            vec![seccomp_profile_descriptor()],
        );
        set = set.with(
            family_declaration("execution-policy"),
            vec![execution_policy_descriptor()],
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
            "network-local" => vec![
                // The NetworkBinding row type this family also serves (U17):
                // the relationships whose membership the fabric render
                // writes, read back from the plane's own committed rows.
                d2b_provider_network_local::network_binding_descriptor(
                    d2b_provider_network_local::NetworkBindingDriverArgs {
                        zone: inputs.zone.clone(),
                    },
                ),
                network_descriptor(NetworkDriverArgs {
                    zone: inputs.zone.clone(),
                    controller_generation: inputs.authority.controller_generation,
                    facets: inputs.network_facets.clone(),
                }),
            ],
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
                // U31: the verified deployment graph this plane published,
                // when the composition published one. A graph that does not
                // name this family's implementation refuses the family's
                // reconcile before any runner is planned; a plane built
                // before the cutover carries `None` and keeps the
                // pre-cutover behaviour until the cutover installs it.
                deployment_graph: inputs.deployment_graph.clone(),
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
                    // The session's child intents are the display Provider's
                    // own derivation, and admitting them is this Provider's
                    // own vocabulary: the child source commits the session's
                    // committed shapes into the same registry the Endpoint
                    // family's facet set reads (KTD5).
                    WaylandSession::new(Arc::new(PlaneDisplayChildSource {
                        vocabulary: Arc::clone(&inputs.display_endpoint_vocabulary),
                    })),
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
                zone: inputs.zone.clone(),
                controller_generation: inputs.authority.controller_generation,
                facets: inputs.guest_facets.clone(),
            })],
            // U12 (device families): each device family's driver builds its
            // effects from the declared facets; no externally built port
            // appears here (R2). The Device and USBIP families serve the
            // two USB and two security-key types through their own
            // declarations.
            "device-usbip" => Vec::from(usbip_descriptors(UsbipDriverArgs {
                zone: inputs.zone.clone(),
                controller_generation: inputs.authority.controller_generation,
                facets: inputs.usbip_facets.clone(),
            })),
            "device-security-key" => Vec::from(security_key_descriptors(SecurityKeyDriverArgs {
                zone: inputs.zone.clone(),
                controller_generation: inputs.authority.controller_generation,
                facets: inputs.security_key_facets.clone(),
            })),
            "device" => vec![
                device_descriptor(DeviceDriverArgs {
                    zone: inputs.zone.clone(),
                    controller_generation: inputs.authority.controller_generation,
                    facets: inputs.device_facets.clone(),
                }),
                // The DeviceBinding row type this family also serves (U16):
                // built from the same daemon-supplied facet set, so the row
                // and its parent are driven through one set (R2).
                d2b_provider_device::device_binding_descriptor(
                    d2b_provider_device::DeviceBindingDriverArgs {
                        zone: inputs.zone.clone(),
                        controller_generation: inputs.authority.controller_generation,
                        facets: inputs.device_facets.clone(),
                    },
                ),
            ],
            // The VolumeBinding family (U6): the driver builds its effects
            // from the daemon-supplied facet set; no externally built port
            // appears at this construction site (R2).
            "volume-binding" => vec![binding_descriptor(BindingDriverArgs {
                zone: inputs.zone.clone(),
                facets: inputs.binding_facets.clone(),
                vcpu_count: inputs.authority.vcpu_count,
            })],
            // The Endpoint family (U6): the driver builds its effects from
            // the daemon-supplied facet set; no externally built port
            // appears at this construction site (R2).
            "endpoint" => {
                // The Endpoint family (U6): the driver builds its effects from
                // the daemon-supplied facet set; no externally built port
                // appears at this construction site (R2).
                let mut drivers = vec![endpoint_descriptor(EndpointDriverArgs {
                    zone: inputs.zone.as_str().to_owned(),
                    facets: inputs.endpoint_facets.clone(),
                })];
                // The EndpointBinding row type this family also serves (U18).
                //
                // Its privileged delivery reaches the exact-endpoint ACL
                // helpers over the broker socket, which only the daemon holds,
                // so the driver is always registered and the dispatch is the
                // daemon's when the plane carries one. A plane without a
                // daemon gets the family's own unwired dispatch, which refuses
                // every verb by name: the type stays covered by the registry
                // and the relationship reports undelivered rather than
                // silently having no driver (R2).
                drivers.push(
                    d2b_provider_endpoint::endpoint_binding_descriptor(
                        d2b_provider_endpoint::EndpointBindingDriverArgs {
                            zone: inputs.zone.clone(),
                            access: match inputs.server_state.clone() {
                                Some(state) => Arc::new(
                                    crate::DaemonEndpointAccessDispatch::new(state),
                                ),
                                None => Arc::new(d2b_provider_endpoint::UnwiredEndpointAccess),
                            },
                        },
                    ),
                );
                drivers
            }
            // The Credential family (U8): the driver builds its effects from
            // the daemon-supplied facet set; no externally built port
            // appears at this construction site (R2).
            "credential" => vec![
                // The CredentialBinding row type this family also serves (U37).
                d2b_provider_credential::credential_binding_descriptor(
                    d2b_provider_credential::CredentialBindingDriverArgs {
                        zone: inputs.zone.clone(),
                        controller_generation: inputs.authority.controller_generation,
                        facets: inputs.credential_facets.clone(),
                    },
                ),
                credential_descriptor(CredentialDriverArgs {
                    zone: inputs.zone.clone(),
                    controller_generation: inputs.authority.controller_generation,
                    facets: inputs.credential_facets.clone(),
                }),
            ],
            // The Volume family (U7): the driver builds its effects from the
            // declared facets; no externally built port appears here (R2).
            "volume" => vec![volume_descriptor(VolumeDriverArgs {
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

    /// Open the store, register the converted-type factories, apply the
    /// Zone's declared authority rows, and spawn the manager.
    /// Initial-load completion is a separate step so the readiness checklist
    /// is observable stage by stage; [`Self::open`] composes both.
    ///
    /// The declared authority rows are applied HERE, before the spawn, and
    /// not after it: a `Role` or `RoleBinding` row is an input an accepted
    /// graph is built from, so a manager that spawned first and ingested
    /// afterwards left every authority reader that runs before the ingest
    /// looking at an empty Zone.
    ///
    /// Every stage here is async work the constructor awaits; the only
    /// synchronous work left is the store's own half (a directory create and
    /// SQLite's open + migration, neither of which has an async form), which
    /// runs on the blocking pool so it is bounded by that pool rather than by
    /// the worker this call would otherwise park.
    pub async fn prepare(inputs: ConstructionInputs) -> Result<Self, PlaneError> {
        let readiness = Arc::new(NewPlaneReadinessState::new());
        // Stage 1: durable spec store. The directory create is async
        // (`tokio::fs`); the SQLite open + migration has no async form and runs
        // once on the daemon's reused bounded loader seat (plan KTD2: zero
        // new seats; d2bd already drives bundle resolution on the same
        // shipped bounded worker). A saturated seat refuses the plane start
        // with a named Authority error instead of parking the worker.
        let store_path = Self::spec_store_path(&inputs.spec_store_dir);
        if let Some(parent) = store_path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|error| {
                PlaneError::Authority(error.into())
            })?;
        }
        let store = Arc::new(
            d2b_core::loader_worker::run(move || {
                SpecStore::open(store_path.clone()).map_err(PlaneError::from)
            })
           .await
           .map_err(|error| PlaneError::Authority(error.into()))??,
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
        // F5: the display Provider's committed-shape vocabulary is rebuilt
        // from the same durable rows, here, before the manager spawns any
        // actor. The live path commits a session's shapes when that session's
        // own actor reconciles, which orders them correctly for a session
        // admitted after the plane is open and orders nothing at all on a
        // restart - where the manager starts one actor per durable row at
        // once, and an `Endpoint` child row would be refused for a shape its
        // session does commit. A restart reads its shapes from the store
        // instead of from reconcile order.
        restore_display_endpoint_vocabulary(
            &store,
            &inputs.zone,
            &inputs.display_endpoint_vocabulary,
        )
        .await?;
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
        // KTD6: the manager's fence. It is bound from this plane's own store
        // incarnation and the origination leg, not from the deployment
        // document's generation: the broker refuses a publication session
        // whose store generation is not the store the rows live in, and a
        // document generation would silently fence every candidate this store
        // ever stages. The publication subject is the verified deployment
        // identity this daemon authenticates as, never the row being mutated -
        // a candidate that presented its own identity would be authorizing its
        // own introduction.
        let incarnation = store.store_incarnation().await.map_err(PlaneError::from)?;
        let authority: Arc<dyn AuthorityPublisher> = match &inputs.authority_publisher {
            Some(bound) => Arc::clone(bound),
            None => {
                let broker_socket = inputs
                    .trusted_context_publication
                    .as_ref()
                    .map(|publication| publication.broker_socket().to_path_buf())
                    .ok_or_else(|| PlaneError::Authority(
                        "this plane has no origination leg and no bound publisher, so no \
                         desired mutation has a fence to commit against"
                            .into(),
                    ))?;
                let coordinator = Arc::new(
                    crate::authority_publication::AuthorityPublicationCoordinator::new(
                        inputs.zone.as_str(),
                        incarnation.clone(),
                        authority_publication_subject(),
                        Arc::new(crate::authority_publication::OriginationPublicationLink::new(
                            broker_socket.clone(),
                            AUTHORITY_PUBLICATION_ROUND_TRIP,
                        )),
                    ),
                );
                let publisher = crate::authority_publication::CoordinatorPublisher::new(
                    coordinator,
                    incarnation.clone(),
                    authority_publication_subject(),
                );
                // The seed homes its rows in the system Zone, so that Zone
                // needs its own coordinator: a coordinator carries the Zone
                // its session is bound to and that Zone's accepted cursor, and
                // a seeded row fenced under the plane's own Zone would leave
                // the store's per-Zone sequence and the broker's per-Zone
                // projection describing different authorities.
                if inputs.foundation.is_some() {
                    publisher.with_zone(Arc::new(
                        crate::authority_publication::AuthorityPublicationCoordinator::new(
                            crate::foundation_seed::SYSTEM_ZONE,
                            incarnation,
                            authority_publication_subject(),
                            Arc::new(crate::authority_publication::OriginationPublicationLink::new(
                                broker_socket,
                                AUTHORITY_PUBLICATION_ROUND_TRIP,
                            )),
                        ),
                    ))
                } else {
                    publisher
                }
            }
        };
        if inputs.foundation.is_some() {
            // The seed homes its rows in the system Zone, which is not this
            // plane's own Zone, so the manager's own restart adoption never
            // covers it. A previous boot that died between staging a seeded
            // row and settling it would otherwise leave that Zone owing an
            // outcome forever, and every boot after it would be refused by
            // the one-outstanding-transaction rule before the seed wrote
            // anything. Adopt first, through the same recovery the manager
            // uses, so the Zone is settled or explicitly refused here rather
            // than wedged at the seed's first publish.
            d2b_resource_runtime::authority_publish::adopt_outstanding(
                &store,
                crate::foundation_seed::SYSTEM_ZONE,
                authority.as_ref(),
            )
            .await
            .map_err(|error| PlaneError::ZoneRecovery(error.to_string()))?;
            // The seed's Zone is reconciled the same way the manager reconciles
            // its own, and for the same reason: a broker restart moves it to
            // reconciling too, and the plane rather than any manager is what
            // publishes for it. Adoption first, then the reconciliation, and
            // only then does the seed write anything.
            d2b_resource_runtime::authority_publish::resynchronize(
                &store,
                crate::foundation_seed::SYSTEM_ZONE,
                authority.as_ref(),
            )
            .await
            .map_err(|error| PlaneError::ZoneRecovery(error.to_string()))?;
        }
        if let Some(foundation) = &inputs.foundation {
            let seed = crate::foundation_seed::FoundationSeed::new(
                foundation.declarations.clone(),
                foundation.allocation.clone(),
            );
            let report = seed.run(&store, &providers, authority.as_ref()).await?;
            tracing::info!(
                zone = %inputs.zone.as_str(),
                committed = report.committed.len(),
                materialized = report.materialized.len(),
                unchanged = report.unchanged,
                "foundation seed committed the policy rows"
            );
        }
        // The Zone's own declared authority is applied before the manager
        // spawns, in the same place and through the same fenced store path
        // the foundation seed just used. Ordering is the whole point: a
        // `Role` or `RoleBinding` row is the input an accepted graph is
        // built from, so a plane that spawned its manager first and ingested
        // afterwards published the Zone while every authority reader that
        // runs before the ingest - the broker's per-Zone projection, the
        // session layer, the manager-boundary admission - still saw an empty
        // Zone. The rows are durable and acknowledged here, and the manager's
        // own `pre_start` loads them, so its actors start from a committed row
        // (F1) exactly as the seeded rows do.
        let committed_authority = Self::commit_declared_authority_rows(
            &store,
            inputs.bundle.as_ref(),
            &inputs.zone,
            authority.as_ref(),
        )
        .await?;
        if !committed_authority.is_empty() {
            tracing::info!(
                zone = %inputs.zone.as_str(),
                committed = committed_authority.len(),
                "the zone's declared authority rows committed before the manager spawn"
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
           .map_err(|error| PlaneError::Target(error.into()))?;
        // Every registered driver's declaration carries its type's decoder,
        // so the registry is the authority: the plane wires no decoder table
        // of its own.
        let decoders = providers.decoders();
        // U40: the manager-boundary admission, and the two facts it would
        // read: the Zone's own prior accepted graph, and the Zone's accepted
        // ceilings and reduction, which the two family drivers republish as
        // their rows commit. A limits snapshot taken here would hold only
        // what existed at the spawn and would never enforce a ceiling an
        // operator commits afterwards, which is why the holder below is the
        // live one.
        //
        // The two per-Zone runtimes are installed BEFORE the manager spawns,
        // so a family's driver finds its runtime on its first reconcile. The
        // census is counted from the plane's own committed rows by the Quota
        // driver, because `MutationAdmission::admit` is synchronous and cannot
        // read the store per mutation.
        d2b_provider_quota::install(Arc::new(d2b_provider_quota::ZoneQuotaRuntime::new(
            inputs.zone.clone(),
            Arc::new(crate::PlaneZoneUsage { store: Arc::clone(&store), zone: inputs.zone.clone() }),
        )));
        d2b_provider_emergency_policy::install(Arc::new(
            d2b_provider_emergency_policy::ZoneEmergencyRuntime::new(
                inputs.zone.clone(),
                Arc::new(crate::PlaneZoneOpenUse {
                    store: Arc::clone(&store),
                    zone: inputs.zone.clone(),
                }),
            ),
        ));
        // U40: the manager-boundary admission is still NOT installed, and the
        // reason is no longer the graph.
        //
        // The per-Zone accepted graph this admission needs now EXISTS at this
        // point: `commit_declared_authority_rows` committed the Zone's own
        // `Role` and `RoleBinding` rows above, through the store's fenced
        // path, before this manager spawns, so the graph built from them is
        // rooted at THIS Zone and is not the deployment's system-Zone graph.
        //
        // What still blocks the install is the subject every other mutating
        // entry point presents. `GraphMutationAdmission` decides a mutation
        // from the initiating subject alone, and it refuses a principal that
        // is neither an exact `Type/name` reference nor one of the
        // bootstrap-class tokens. The owned-cascade boundaries render their
        // subject as `ResourceKey`'s display form - `zone/Type/name` - which
        // is not an exact reference (both the child bridge and the manager's
        // own cascade do this), and no Zone bundle declares the RoleBindings
        // that would grant a cascade subject even once it parsed. Installing
        // the identity arm today therefore refuses every owned-child commit in
        // every Zone, which is strictly worse than the gap it closes.
        //
        // What IS installed is only the fence below. The per-Zone runtimes
        // and the drain finalizer the families drive are real and wired, but
        // no committed ceiling or reduction reaches a mutation until the
        // admission that reads them is installed, so nothing here should be
        // read as enforcement that is live today.
        // The foundation plane is the one that carries the seeded system-homed
        // rows; every other plane is zone-local and the fence refuses those
        // three types on it.
        let admission: Arc<dyn MutationAdmission> = Arc::new(
            crate::foundation_seed::SystemZoneWriteFence::new(inputs.foundation.is_some()),
        );
        let args = ResourceManagerArgs {
            zone: inputs.zone.as_str().to_owned(),
            store: Arc::clone(&store),
            authority,
            providers,
            hub: Arc::clone(&hub),
            admission,
            decoders,
            default_decoder: Arc::new(PassthroughDecoder),
            targets: Arc::clone(&targets),
            host_target,
            target_resolver: Arc::new(DeclaredExecutionRef),
            backoff: PLANE_BACKOFF,
            // Relation indexing (U6, R3/R4): the plane registers no per-type
            // projection yet, so the manager's derived index carries ownership
            // only - exactly the relationship class the unchanged entry point
            // already relies on. The canonical projections arrive with each
            // converted declaration family and the new admission construction
            // is installed with them at the U34 cutover.
            relation_extractors: RelationExtractors::new(),
        };
        let (actor, _join) = ractor::Actor::spawn(None, ResourceManager::new(), args).await?;
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

    /// Commit the Zone bundle's declared authority rows, before the manager
    /// exists.
    ///
    /// These are the rows an accepted graph is built from: a `Role` states
    /// the rules one grant draws on and a `RoleBinding` states which subject
    /// holds them, so anything that decides a mutation reads them. The class
    /// comes from [`AuthorityRowKind::of_reference`] rather than from a local
    /// table of type names, so the plane and the evaluator cannot disagree
    /// about what counts as authority.
    ///
    /// The write is the store's only fenced path - staged, frozen, committed,
    /// published, acknowledged - and it is the same call the foundation seed
    /// makes for the rows it homes in the system Zone, so a seeded row and a
    /// Zone-declared row are committed by one authority rather than two.
    ///
    /// Nothing is overwritten. A row this store already holds is left exactly
    /// as it stands, whatever its provenance: an API-created row keeps the
    /// provenance it was created under (R26), a row already retiring keeps
    /// that mark, and [`ResourcePlaneV3::ingest_nix_bundle`] partitions both
    /// the same way once the plane is open. An authority row that declares an
    /// owner is left to that ingest too, because linking an owned child is
    /// the manager's own deferral sweep and this step does not second-guess
    /// it.
    async fn commit_declared_authority_rows(
        store: &SpecStore,
        bundle: Option<&ResourceBundle>,
        zone: &ZoneId,
        authority: &dyn AuthorityPublisher,
    ) -> Result<Vec<ResourceKey>, PlaneError> {
        let Some(bundle) = bundle else {
            return Ok(Vec::new());
        };
        bundle.verify()?;
        let mut committed = Vec::new();
        for row in &bundle.resources {
            let key = bundle_row_key(zone, row);
            let Ok(reference) = ResourceRef::parse(&format!(
                "{}/{}",
                key.type_name, key.name
            )) else {
                continue;
            };
            if !AuthorityRowKind::of_reference(&reference).is_authority()
                || row.metadata().owner_ref().is_some()
            {
                continue;
            }
            // Any row this store already holds is left exactly as it stands,
            // including one already retiring: an ensure against a deleting
            // row is refused by the store, and the ingest partitions a
            // retiring row as a skip anyway.
            if store
                .list(SpecSelector {
                    zone: Some(zone.as_str().to_owned()),
                    type_name: Some(key.type_name.clone()),
                    owner_uid: None,
                })
                .await?
                .iter()
                .any(|held| held.key.name == key.name)
            {
                continue;
            }
            let desired = bundle_desired(zone, row);
            store
                .publish(
                    d2b_resource_runtime::DesiredMutation::Ensure(StoredDesiredResource {
                        uid: d2b_resource_runtime::manager::deterministic_uid(&key),
                        key: key.clone(),
                        generation: 1,
                        owner_uid: None,
                        provenance: d2b_resource_runtime::identity::ResourceProvenance::Nix,
                        deleting: false,
                        spec: desired.spec,
                        metadata: desired.metadata,
                        created_at: 0,
                    }),
                    authority,
                )
                .await
                .map_err(|error| {
                    tracing::error!(
                        zone = %zone.as_str(),
                        reference = %key,
                        error = ?error,
                        "the zone's declared authority row was refused"
                    );
                    PlaneError::AuthorityPublication(error)
                })?;
            committed.push(key);
        }
        Ok(committed)
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
    pub fn targets(&self) -> Arc<TargetDirectory> {
        Arc::clone(&self.targets)
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
           .map_err(|error| PlaneError::Target(error.into()))?;
        self.client
           .actor()
           .send_message(ResourceManagerMsg::TargetReconnected {
                guest: guest.clone(),
                session_generation: outcome.session_generation(),
                pending_adoption: outcome.pending_adoption().to_vec(),
            })
           .map_err(|error| PlaneError::Target(error.into()))?;
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
           .map_err(|error| PlaneError::Target(error.into()))?;
        self.client
           .actor()
           .send_message(ResourceManagerMsg::TargetUnavailable {
                guest: guest.clone(),
                session_generation: outcome.session_generation(),
                affected: outcome.affected().to_vec(),
            })
           .map_err(|error| PlaneError::Target(error.into()))?;
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
    pub fn hub(&self) -> Arc<WatchHub> {
        Arc::clone(&self.hub)
    }

    /// See [`Self::readiness`]: read by this module's tests.
    #[cfg(test)]
    pub fn store(&self) -> &SpecStore {
        &self.store
    }


    /// The per-zone registry the production effects resolve per-resource
    /// anchors from.
    pub fn registry(&self) -> &PlaneResourceRegistry {
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
    bundle.verify()?;
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
    /// The rows this ingestion applied.
    pub applied: Vec<ResourceKey>,
    /// The rows this ingestion removed.
    pub removed: Vec<ResourceKey>,
    /// The rows this ingestion protected from management-plane mutation.
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
    use d2b_contracts_resource::{resource_proto as wire, v3::ResourceName};
    use d2b_contracts_zone_session::v3::resource_bundle::BundleResourceMetadata;
    use d2b_process_conformance::ProcessIdentityDigest;
    use d2b_provider_system_core::UserIdentityDigest;
    use d2b_resource_runtime::revision::ManualClock;
    use d2b_resource_runtime::watch::{ChangeKind, ChangeNotice, WatchHubConfig};
    use d2b_contracts_broker::broker_wire::{
        BrokerErrorResponse, BrokerRequestEnvelope, EndpointAccessResponse, EndpointAccessVerb,
    };
    use std::os::fd::AsRawFd;

    use d2b_core::resource_authority::{AcceptedGraph, ProjectionRow, TransportIdentity};

    /// A principal that is not a real host account must be refused, not
    /// resolved to a guessed id. The closed contract requires these
    /// principals to be real accounts, and `host-users.nix` materializes the
    /// Device TPM ones from `d2bLib.deviceTpmPrincipals` with the uid the
    /// worker row runs as. A name that does not resolve is therefore a
    /// provisioning gap; answering it with a name-derived hash would turn a
    /// loud refusal into a silent permission grant for a uid no process
    /// holds, which is the harder failure to diagnose later.
    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn a_principal_without_an_account_is_refused() {
        for name in [
            "d2b-acceptance-guest-swtpm",
            "d2b-acceptance-guest-swtpm-flush",
            "d2b-no-such-account-probe",
        ] {
            assert!(
                principal_id_for(name, false).is_err(),
                "{name} has no host account and must not resolve to a guessed id"
            );
        }
    }

    /// A principal that is a real account resolves to the account's own id.
    /// These are the same ids `host-users.nix` assigns the Device TPM
    /// accounts, so this is the path the state Volume's ACL grants ride.
    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn a_principal_that_is_an_account_resolves_to_its_real_id() {
        let Some(user) = nix::unistd::User::from_name("root").ok().flatten() else {
            return;
        };
        assert_eq!(
            principal_id_for("root", false).expect("root is a host account"),
            user.uid.as_raw()
        );
    }

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
        // No broker in these fixtures: the recording publisher fences and
        // accepts what the store publishes.
        let publisher = d2b_resource_runtime::test_support::RecordingPublisher::new();
        commit_volume_row_with(store, key, publisher.as_ref()).await
    }

    async fn commit_binding_row(store: &SpecStore, key: &ResourceKey) {
        let publisher = d2b_resource_runtime::test_support::RecordingPublisher::new();
        commit_binding_row_with(store, key, publisher.as_ref()).await
    }

    async fn commit_volume_row_with(
        store: &SpecStore,
        key: &ResourceKey,
        publisher: &dyn d2b_resource_runtime::AuthorityPublisher,
    ) {
        store
           .publish(d2b_resource_runtime::DesiredMutation::Ensure(StoredDesiredResource {
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
            }), publisher)
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
            "presentation": { "presentation": "filesystem", "destination": "/state" },
            "slot": "state",
            "source": {
                "admittedRights": ["consume"],
                "arbitration": "shared",
                "realizedFacets": ["filesystem-presentation"],
            },
        })
    }

    /// One committed VolumeBinding row, as the Volume driver mints it
    /// (the serving Provider reference rides in the stored envelope).
    async fn commit_binding_row_with(
        store: &SpecStore,
        key: &ResourceKey,
        publisher: &dyn d2b_resource_runtime::AuthorityPublisher,
    ) {
        let mut envelope = binding_spec().as_object().cloned().expect("object");
        envelope.insert(
            "providerRef".to_owned(),
            serde_json::Value::String("Provider/volume-virtiofs".to_owned()),
        );
        store
           .publish(d2b_resource_runtime::DesiredMutation::Ensure(StoredDesiredResource {
                key: key.clone(),
                uid: d2b_resource_runtime::manager::deterministic_uid(key),
                generation: 1,
                owner_uid: Some([0x11; 16]),
                provenance: d2b_resource_runtime::identity::ResourceProvenance::Resource,
                deleting: false,
                spec: serde_json::to_vec(&envelope).expect("envelope"),
                metadata: Vec::new(),
                created_at: 0,
            }), publisher)
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

    /// The plane inputs over a caller-chosen Guest facet set, so a test can
    /// observe the Cloud Hypervisor controller-session calls the composed
    /// Guest driver's effects make through the same facets the plane builds
    /// the family's effects service from.
    fn test_inputs_with_guest_facets(
        guest_facets: d2b_provider_guest::facets::GuestEffectFacets,
    ) -> (tempfile::TempDir, ConstructionInputs, Arc<NewPlaneReadinessState>) {
        test_inputs_over(
            d2b_provider_wayland_policy::test_support::scripted_facets(
                ZoneId::parse("test").unwrap(),
            ),
            guest_facets,
        )
    }


    /// The plane inputs over a caller-chosen interaction facet set, so a
    /// test can seed the family's audio registry through the same facets the
    /// plane hosts the declared service from.
    fn test_inputs_with_interaction_facets(
        interaction_facets: d2b_provider_wayland_policy::InteractionEffectFacets,
    ) -> (tempfile::TempDir, ConstructionInputs, Arc<NewPlaneReadinessState>) {
        test_inputs_over(
            interaction_facets,
            d2b_provider_guest::test_support::ScriptedFacets::new().facet_set(),
        )
    }

    /// The plane inputs over both caller-chosen facet sets: the interaction
    /// family's and the Guest family's, each the value the production
    /// composition root builds its family's effects service from.
    fn test_inputs_over(
        interaction_facets: d2b_provider_wayland_policy::InteractionEffectFacets,
        guest_facets: d2b_provider_guest::facets::GuestEffectFacets,
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
        // caller's scripted facets double, exactly as the production
        // composition root builds it from the daemon's runtime.
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
        // The production composition installs ONE display vocabulary into both
        // the Endpoint family's committed-shape seam and the session's
        // child-intent source. The fixture wires that same object into both: a
        // fixture that handed the Endpoint family its own empty registry would
        // refuse every display endpoint row for a shape this plane does
        // commit, and no test over it could see why.
        let display_endpoint_vocabulary = Arc::new(SharedDisplayEndpointVocabulary::new());
        let endpoint_facets = {
            let effects = d2b_provider_endpoint::test_support::FakeSocketEffects::new();
            // The old plane fake reported the socket present
            // (socket_present true); the shared double starts absent.
            effects.make_present();
            effects
                .facet_set()
                .with_committed_shapes(
                    Arc::clone(&display_endpoint_vocabulary)
                        as Arc<dyn CommittedEndpointShapeSource>,
                )
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
                display_endpoint_vocabulary,
                activation_facets: activation_facets.clone(),
            deployment_graph: None,
            server_state: None,
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
                authority_publisher: Some(d2b_resource_runtime::test_support::RecordingPublisher::new()),
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
                bundle: None,
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
                    "lastSetApplied": "NotApplied",
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
    /// registered service identity (U3, R5: the registration table
    /// carries the row; the daemon names no family string, only the
    /// crate's declared service id), and the hosted service answers
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
            "operation",
            "seccomp-profile",
            "execution-policy",
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

    /// Every service the generated registration closure declares is hosted by
    /// the production composition, and it hosts nothing else.
    ///
    /// The table is the committed `generated/new-graph/` byte the daemon
    /// compiles, and the factories come from `ConstructionInputs::production`,
    /// the same construction a real Zone's plane takes. A declaration that
    /// adds a service makes this fail before any Zone starts, because the
    /// hosting side would otherwise refuse the service by name at startup and
    /// the composition would have drifted from the generated closure.
    #[test]
    fn every_service_the_generated_closure_declares_is_hosted_by_the_composition() {
        let (_dir, inputs, _readiness) = test_inputs();
        let declared: BTreeSet<(&'static str, &'static str)> = PROVIDER_REGISTRATIONS
            .iter()
            .flat_map(|registration| {
                registration
                    .services
                    .iter()
                    .map(move |service| (registration.provider_ref, *service))
            })
            .collect();
        let declared_services: BTreeSet<&'static str> =
            declared.iter().map(|(_, service)| *service).collect();

        let unhosted: Vec<&str> = declared_services
            .iter()
            .filter(|service| !inputs.effect_service_factories.contains_key(*service))
            .copied()
            .collect();
        assert!(
            unhosted.is_empty(),
            "the generated closure declares services the composition does not host: {unhosted:?}",
        );

        let undeclared: Vec<&str> = inputs
            .effect_service_factories
            .keys()
            .filter(|service| !declared_services.contains(*service))
            .copied()
            .collect();
        assert!(
            undeclared.is_empty(),
            "the composition hosts services the generated closure does not declare: {undeclared:?}",
        );

        let families: BTreeSet<&str> = PROVIDER_REGISTRATIONS
            .iter()
            .map(|registration| registration.provider_ref)
            .collect();
        assert_eq!(
            families.len(),
            PROVIDER_REGISTRATIONS.len(),
            "a generated registration row names a family identity twice",
        );
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
            "presentation": { "presentation": "filesystem", "destination": "/state" },
            "slot": "state",
            "source": {
                "admittedRights": ["consume"],
                "arbitration": "shared",
                "realizedFacets": ["filesystem-presentation"],
            },
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
        // No broker in this fixture: the recording publisher fences and
        // accepts what the store publishes.
        let publisher = d2b_resource_runtime::test_support::RecordingPublisher::new();
        plane
           .store()
           .publish(d2b_resource_runtime::DesiredMutation::Ensure(StoredDesiredResource {
                key: ResourceKey::new("test", "VolumeBinding", "vol-binding-derived"),
                uid,
                generation: 1,
                owner_uid: Some([0x11; 16]),
                provenance: d2b_resource_runtime::identity::ResourceProvenance::Resource,
                deleting: false,
                spec: serde_json::to_vec(&envelope).expect("envelope"),
                metadata: Vec::new(),
                created_at: 0,
            }), publisher.as_ref())
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
    fn operation_desired(zone: &str, name: &str) -> DesiredResource {
        let spec = d2b_contracts_resource::v3::canonical_json_bytes(&serde_json::json!({
            "ownerRef": "Role/worker",
            "payloadSchema": {
                "type": "object",
                "additionalProperties": false,
                "properties": { "socketPath": { "type": "string" } }
            },
            "secretAccess": "None",
            "audit": {
                "enabled": false,
                "reasons": [],
                "retentionDays": 0,
                "fields": []
            }
        }))
       .expect("canonical operation spec");
        let metadata = serde_json::to_vec(&serde_json::json!({
            "annotations": {},
            "labels": {},
            "ownerRef": null
        }))
       .expect("metadata");
        DesiredResource {
            key: ResourceKey::new(zone, "Operation", name),
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
           .apply(api_subject("User/alice"), operation_desired("system", "worker"))
           .await
           .expect("the foundation plane admits the write");
    }

    /// The authority rows a Zone bundle declares, in the Zone's own words: one
    /// `Role` over the types that Zone's operator may write, and one
    /// `RoleBinding` naming that operator. This is the exact pair an accepted
    /// graph is built from, and the pair the manager boundary's identity arm
    /// decides against.
    fn declared_authority_rows(role_name: &str, subject: &str) -> Vec<BundleResource> {
        let zone = ZoneId::parse("test").expect("the fixture zone");
        let role = d2b_contracts_zone_session::v3::role::AuthorizedRole::new(
            vec![d2b_contracts_zone_session::v3::RoleRule::new(
                vec![d2b_contracts_resource::v3::ResourceTypeName::parse("Volume")
                    .expect("a standard type")],
                vec![
                    d2b_contracts_zone_session::v3::RoleResourceVerb::Create,
                    d2b_contracts_zone_session::v3::RoleResourceVerb::Delete,
                ],
                Vec::new(),
                Vec::new(),
                vec![zone],
                Vec::new(),
                Vec::new(),
            )
            .expect("the role rule validates")],
            Vec::new(),
        )
        .expect("the role validates");
        let binding = d2b_contracts_zone_session::v3::RoleBindingSpec::new(
            ResourceRef::parse(&format!("Role/{role_name}")).expect("a canonical role"),
            vec![ResourceRef::parse(subject).expect("a canonical subject")],
            None,
            None,
        )
        .expect("the role binding validates");
        vec![
            bundle_row(
                "Role",
                role_name,
                serde_json::to_value(&role).expect("role row bytes"),
            ),
            bundle_row(
                "RoleBinding",
                &format!("{role_name}-holders"),
                serde_json::to_value(&binding).expect("binding row bytes"),
            ),
        ]
    }

    /// The prior accepted graph one Zone's admission reads, rebuilt from the
    /// rows this plane's own store committed - through the same
    /// [`AcceptedGraph::from_canonical_rows`] call, over the same stored
    /// bytes, that the broker's `prior_graph` makes.
    async fn plane_accepted_graph(plane: &ResourcePlaneV3) -> AcceptedGraph {
        let zone = ZoneId::parse("test").expect("the fixture zone");
        let rows = plane
           .store()
           .list(SpecSelector {
                zone: Some(zone.as_str().to_owned()),
                type_name: None,
                owner_uid: None,
            })
           .await
           .expect("store rows");
        let mut decoded: Vec<(ResourceRef, CanonicalJsonObject)> = Vec::new();
        for row in rows {
            let Ok(reference) =
                ResourceRef::parse(&format!("{}/{}", row.key.type_name, row.key.name))
            else {
                continue;
            };
            if !AuthorityRowKind::of_reference(&reference).is_authority() {
                continue;
            }
            let Ok(admitted) = serde_json::from_slice::<CanonicalJsonObject>(&row.spec) else {
                continue;
            };
            decoded.push((reference, admitted));
        }
        let projection = decoded
           .iter()
           .map(|(reference, admitted)| ProjectionRow::new(reference, admitted));
        AcceptedGraph::from_canonical_rows(
            zone,
            plane
               .store()
               .store_incarnation()
               .await
               .expect("store incarnation"),
            AuthoritySubject::unresourced(AuthoritySubjectKind::Bootstrap),
            projection,
        )
        .expect("the committed authority rows decode")
    }

    /// One API mutation of `Type/name` in the fixture Zone, as the manager
    /// boundary presents it.
    fn api_mutation(type_name: &str, name: &str) -> MutationRequest {
        MutationRequest {
            key: ResourceKey::new("test", type_name, name),
            op: AdmissionOp::Ensure,
            spec: Vec::new(),
            metadata: br#"{"annotations":{},"labels":{},"ownerRef":null}"#.to_vec(),
        }
    }

    /// A cold start in a Zone-local plane applies that Zone's declared
    /// authority BEFORE the manager spawns, so the per-Zone accepted graph
    /// the manager boundary reads already carries the Zone's own grants the
    /// first time it reads it - and a mutation the Zone granted is admitted
    /// rather than refused for want of a graph.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread")]
    async fn a_zone_local_cold_start_applies_its_declared_authority_before_the_manager_spawns() {
        let (_dir, mut inputs, _readiness) = test_inputs();
        inputs.bundle = Some(test_bundle(declared_authority_rows("volume-writer", "User/alice")));
        // No ingest call: everything asserted below was committed by the
        // plane's own open, before its manager existed.
        let plane = ResourcePlaneV3::prepare(inputs).await.expect("plane");
        plane.complete_initial_load().await.expect("initial load");

        for (type_name, name) in
            [("Role", "volume-writer"), ("RoleBinding", "volume-writer-holders")]
        {
            let committed = plane
               .store()
               .list(SpecSelector {
                    zone: Some("test".to_owned()),
                    type_name: Some(type_name.to_owned()),
                    owner_uid: None,
                })
               .await
               .expect("store rows")
               .into_iter()
               .any(|row| row.key.name == name);
            assert!(committed, "{type_name}/{name} was not committed before the manager spawned");
            // The manager's own pre_start loaded it, so the row is served
            // from the manager and not only from the store.
            assert!(
                plane
                   .client()
                   .get_row(ResourceKey::new("test", type_name, name))
                   .await
                   .expect("manager row")
                   .is_some(),
                "{type_name}/{name} is durable but the manager never loaded it"
            );
        }

        let graph = plane_accepted_graph(&plane).await;
        assert_eq!(
            graph.zone().as_str(),
            "test",
            "the graph is rooted at the Zone whose rows it holds, not the deployment's"
        );
        let admission = GraphMutationAdmission::new(
            Arc::new(graph),
            ZoneId::parse("test").expect("the fixture zone"),
            TransportIdentity::ComponentSession,
        );
        assert!(
            matches!(
                admission.admit(&api_subject("User/alice"), &api_mutation("Volume", "state")),
                AdmissionDecision::Allow
            ),
            "a Zone-local cold start refused a mutation its own declared RoleBinding grants"
        );
        plane.shutdown().await;
    }

    /// The same admission is not a rubber stamp: a mutation naming a row the
    /// accepted graph does not cover is refused, on the target axis and on
    /// the subject axis, while the granted one still passes beside them.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread")]
    async fn the_zone_local_admission_refuses_a_row_its_graph_does_not_cover() {
        let (_dir, mut inputs, _readiness) = test_inputs();
        inputs.bundle = Some(test_bundle(declared_authority_rows("volume-writer", "User/alice")));
        let plane = ResourcePlaneV3::open(inputs).await.expect("plane");
        let admission = GraphMutationAdmission::new(
            Arc::new(plane_accepted_graph(&plane).await),
            ZoneId::parse("test").expect("the fixture zone"),
            TransportIdentity::ComponentSession,
        );

        // The granted mutation, so neither refusal below can be an admission
        // that denies everything.
        assert!(matches!(
            admission.admit(&api_subject("User/alice"), &api_mutation("Volume", "state")),
            AdmissionDecision::Allow
        ));
        // The target the Zone's Role does not name.
        let refused_type = admission.admit(
            &api_subject("User/alice"),
            &api_mutation("Guest", "work"),
        );
        assert!(
            matches!(&refused_type, AdmissionDecision::Deny(refusal) if refusal.contains("identity-not-authorized")),
            "a type the graph grants nothing was admitted: {refused_type:?}"
        );
        // The subject no RoleBinding in this Zone names.
        let refused_subject =
            admission.admit(&api_subject("User/mallory"), &api_mutation("Volume", "state"));
        assert!(
            matches!(&refused_subject, AdmissionDecision::Deny(refusal) if refusal.contains("identity-not-authorized")),
            "a subject the graph never names was admitted: {refused_subject:?}"
        );
        plane.shutdown().await;
    }

    /// A publication transaction the system Zone still owes an outcome for is
    /// resolved before the foundation seed publishes.
    ///
    /// The seed homes its rows in the system Zone, which is not this plane's
    /// own Zone, so the manager's own restart adoption does not cover it. A
    /// previous boot that died between staging a seeded row and settling it
    /// therefore has to be adopted here: without it the seed's first publish
    /// is refused by the one-outstanding-transaction rule and the plane never
    /// opens, which is a permanent wedge rather than a retryable refusal.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread")]
    async fn the_foundation_plane_adopts_the_system_zone_before_it_seeds() {
        let (_dir, mut inputs, _readiness) = test_inputs();
        inputs.foundation = Some(FoundationInputs {
            declarations: crate::foundation_seed::core_declarations(),
            allocation: crate::principal_allocation::PrincipalAllocation::committed()
               .expect("committed allocation"),
        });
        let store_path = ResourcePlaneV3::spec_store_path(&inputs.spec_store_dir);
        {
            let previous_boot = SpecStore::open(&store_path).expect("the store the previous boot opened");
            previous_boot
                .stage_mutation(d2b_resource_runtime::DesiredMutation::Ensure(
                    StoredDesiredResource {
                        key: ResourceKey::new(
                            crate::foundation_seed::SYSTEM_ZONE,
                            "SeccompProfile",
                            "left-outstanding",
                        ),
                        uid: d2b_resource_runtime::manager::deterministic_uid(&ResourceKey::new(
                            crate::foundation_seed::SYSTEM_ZONE,
                            "SeccompProfile",
                            "left-outstanding",
                        )),
                        generation: 0,
                        owner_uid: None,
                        provenance: d2b_resource_runtime::spec_store::ResourceProvenance::Nix,
                        deleting: false,
                        spec: b"{}".to_vec(),
                        metadata: b"{}".to_vec(),
                        created_at: 0,
                    },
                ))
                .await
                .expect("the previous boot stages its seeded candidate");
            // The boot dies here: the candidate is durable, nothing settled it,
            // and the store above went out of scope with its writer thread.
        }

        // The production open path is the restart. It must recover the system
        // Zone and go on to seed, not refuse the start.
        let plane = ResourcePlaneV3::open(inputs)
            .await
            .expect("the plane recovers the outstanding system-Zone transaction and opens");
        let recovery = plane
            .store()
            .zone_recovery(crate::foundation_seed::SYSTEM_ZONE)
            .await
            .expect("the system Zone answers what it owes");
        assert!(
            !recovery.has_outstanding(),
            "the Zone owes nothing once the seed has published: {recovery:?}"
        );
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
        // No broker in this fixture: the recording publisher fences and
        // accepts what the store publishes.
        let publisher = d2b_resource_runtime::test_support::RecordingPublisher::new();
        store
           .publish(d2b_resource_runtime::DesiredMutation::Ensure(StoredDesiredResource {
                uid,
                key,
                generation: 1,
                owner_uid: None,
                provenance: d2b_resource_runtime::spec_store::ResourceProvenance::Nix,
                deleting: false,
                spec: b"{}".to_vec(),
                metadata: br#"{"annotations":{},"labels":{},"ownerRef":null}"#.to_vec(),
                created_at: 0,
            }), publisher.as_ref())
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

        let (_dir, mut inputs, _readiness) = test_inputs();
        // The readers resolve the Zone the seed homes its rows under: two
        // declarations of the reserved name would put the commit and the read
        // back out of agreement.
        assert_eq!(
            crate::foundation_seed::SYSTEM_ZONE,
            d2b_contracts::identity::SYSTEM_ZONE_NAME,
            "the seed homes its rows in the Zone the readers select",
        );
        let declarations = crate::foundation_seed::core_declarations();
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
        // self-bound provider creates Operation rows. The seeded role narrows
        // the grant to the `create` subresource and names no resource.
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
                    resource_name: None,
                    verb: ResourceVerb::Create,
                    subresource: Some("create".to_owned()),
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
        // One publisher for the whole Zone: a Zone has at most one outstanding
        // publication transaction, so three publishers would refuse two of
        // these rows rather than commit them.
        let publisher = d2b_resource_runtime::test_support::RecordingPublisher::new();
        for key in &keys[..2] {
            commit_volume_row_with(&rig.store, key, publisher.as_ref()).await;
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
        // The Missed that ends this live phase is proved by the recovery
        // reload that follows it, so the third row is committed after that
        // reload rather than racing it: a row committed while the reload was
        // already reading would prove nothing about a row committed between
        // two streams.
        wait_for(|| state.relists.load(Ordering::Relaxed) >= 1).await;
        commit_volume_row_with(&rig.store, &keys[2], publisher.as_ref()).await;
        // Now the row is durable with no notice naming it. Ending the live
        // phase again is what makes the next recovery reload the only path
        // that can register it.
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
        let third = resource_uid(&d2b_resource_runtime::manager::deterministic_uid(&keys[2]))
            .expect("uid");
        wait_for(|| rig.registry.lookup_anchor(&third).is_some()).await;
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

    // -----------------------------------------------------------------------
    // Production-path coverage for the Guest runtime Providers.
    //
    // The plane registers one `Guest` driver for four runtime Providers, and
    // which Provider serves a row is decided from that row's own
    // `spec.providerRef`. Nothing outside that split names it, so a Provider
    // whose branch stopped being taken would leave the row converging as some
    // other Provider - or not at all - with no failure anywhere. Each test
    // below starts the plane's own provider set (the `provider_set`
    // composition the daemon runs), takes the `Guest` driver and the `Guest`
    // decoder that set registered, and drives stored rows through them, so the
    // assertions are the composed driver's own behavior.
    // -----------------------------------------------------------------------

    use d2b_provider_guest::driver::{GUEST_REGISTRATIONS, GuestKind};
    use d2b_provider_toolkit::testing::fakes::{RecordingManagerEndpoint, RecordingRequeue};
    use d2b_resource_runtime::ResourceStatus;
    use d2b_resource_runtime::context::{ResourceContext, SpecDecoder};
    use d2b_resource_runtime::identity::{ResourceKey, ResourceProvenance, StoredDesiredResource};

    /// The Provider reference one runtime-Provider row of the Guest family's
    /// own table names.
    fn runtime_provider_ref(kind: GuestKind) -> &'static str {
        GUEST_REGISTRATIONS[kind.index()].provider_ref
    }

    /// The uid the Guest rows these passes drive, and the uid the composed
    /// driver commits their children under.
    const GUEST_ROW_UID: [u8; 16] = [
        0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x41, 0x11, 0x81, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11,
        0x11,
    ];
    /// The `Guest` row name the runtime-Provider passes below drive.
    const GUEST_ROW_NAME: &str = "worker";

    /// The `Guest` driver factory and spec decoder, as the plane's production
    /// composition registered them.
    async fn production_guest_driver(
        inputs: &ConstructionInputs,
    ) -> (Arc<dyn ResourceDriverFactory>, Arc<dyn SpecDecoder>) {
        let mut runtime = ResourcePlaneV3::start_providers(inputs)
            .await
            .expect("the production composition starts its providers");
        let directory = runtime.take_directory();
        let type_name = d2b_resource_types::WellKnownType::GUEST.to_resource_type_name();
        let factory = directory
            .lookup(&type_name)
            .expect("the production composition registers the Guest type");
        let decoder = directory
            .decoders()
            .get(&type_name)
            .cloned()
            .expect("the production composition registers the Guest decoder");
        runtime.drain().await.expect("the providers drain");
        (factory, decoder)
    }

    fn stored_row(type_name: &str, name: &str, spec: serde_json::Value) -> StoredDesiredResource {
        StoredDesiredResource {
            key: ResourceKey::new("test", type_name, name),
            uid: GUEST_ROW_UID,
            generation: 1,
            owner_uid: None,
            provenance: ResourceProvenance::Nix,
            deleting: false,
            spec: serde_json::to_vec(&spec).expect("canonical spec bytes"),
            metadata: br#"{"ownerRef":null,"labels":{},"annotations":{}}"#.to_vec(),
            created_at: 0,
        }
    }

    /// The child surface the composed driver commits through: the manager
    /// double, keyed into the plane's own zone, with the row's uid as the
    /// owner its committed children carry.
    fn guest_manager() -> Arc<RecordingManagerEndpoint> {
        Arc::new(
            RecordingManagerEndpoint::new()
                .with_zone("test")
                .with_owner_uid(GUEST_ROW_UID),
        )
    }

    fn guest_context(
        row: StoredDesiredResource,
        decoder: Arc<dyn SpecDecoder>,
        manager: Arc<RecordingManagerEndpoint>,
    ) -> ResourceContext {
        let (effects_tx, _effects_rx) = tokio::sync::mpsc::unbounded_channel();
        let (notify_tx, _notify_rx) = tokio::sync::mpsc::unbounded_channel();
        ResourceContext::new(
            row,
            decoder,
            manager,
            Arc::new(RecordingRequeue::default()),
            effects_tx,
            notify_tx,
        )
    }

    /// The committed children of the driven row, read back through the
    /// family's own child surface.
    async fn committed_children(
        ctx: &mut ResourceContext,
    ) -> Vec<(String, serde_json::Value)> {
        ctx.children()
            .await
            .expect("the manager answers the owned set")
            .into_iter()
            .map(|row| {
                (
                    format!("{}/{}", row.key.type_name, row.key.name),
                    serde_json::from_slice(&row.spec).expect("committed child spec decodes"),
                )
            })
            .collect()
    }

    /// The Plane inputs whose Guest effects service is built over a scripted
    /// facet set seeded for the runtime Providers the test drives: the plane's
    /// own controller generation, a committed identity per driven Provider row,
    /// and one enrolled controller-session generation.
    ///
    /// The three are the runtime fence the family's effects validate before
    /// any Provider work runs, so without them the effects refuse the row
    /// before the runtime branch is ever taken.
    async fn production_guest_inputs(
        provider_refs: &[&str],
    ) -> (
        tempfile::TempDir,
        ConstructionInputs,
        Arc<NewPlaneReadinessState>,
        Arc<d2b_provider_guest::test_support::ScriptedFacets>,
    ) {
        let scripted = d2b_provider_guest::test_support::ScriptedFacets::new();
        for (index, provider_ref) in provider_refs.iter().enumerate() {
            scripted.add_committed_provider(
                d2b_contracts_resource::v3::ResourceRef::parse(provider_ref)
                    .expect("typed provider reference"),
                d2b_contracts_resource::v3::ResourceUid::parse(format!(
                    "00000000-0000-4000-8000-0000000000{index:02x}"
                ))
                .expect("bounded resource uid"),
                d2b_contracts_resource::v3::ResourceGeneration::new(1)
                    .expect("bounded generation"),
            );
        }
        scripted.set_session_generation(Some(
            d2b_contracts_resource::v3::identity::ReconnectGeneration::new(1)
                .expect("bounded reconnect generation"),
        ));
        for provider_ref in provider_refs {
            let name = provider_ref
                .strip_prefix("Provider/")
                .expect("a runtime Provider reference");
            scripted
                .add_row(d2b_provider_guest::test_support::row_fixture(
                    "test",
                    "Provider",
                    name,
                    serde_json::json!({}),
                    ResourceStatus::Ready,
                ))
                .await;
        }
        let inputs = test_inputs_with_guest_facets(d2b_provider_guest::facets::GuestEffectFacets {
            zone: ZoneId::parse("test").expect("bounded zone"),
            controller_generation: ControllerGeneration::new(1)
                .expect("bounded controller generation"),
            manager: Arc::clone(&scripted)
                as Arc<dyn d2b_provider_guest::facets::GuestManagerView>,
            cloud_hypervisor: Arc::clone(&scripted)
                as Arc<dyn d2b_provider_guest::facets::CloudHypervisorGuestRuntime>,
        });
        (inputs.0, inputs.1, inputs.2, scripted)
    }

    /// The Cloud Hypervisor runtime Provider: the composed driver's effects
    /// reach this Zone's Cloud Hypervisor controller session for the row, and
    /// the kind commits no manager children (its fixed child roles belong to
    /// the controller session).
    ///
    /// The facet set is the value the composition root builds the family's
    /// effects service from, so the recorded calls are the composed driver's
    /// own: the target session it establishes and the controller reconcile it
    /// drives.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread")]
    async fn the_production_guest_driver_drives_the_cloud_hypervisor_controller_session() {
        let (_dir, inputs, _readiness, scripted) = production_guest_inputs(&[
            runtime_provider_ref(GuestKind::CloudHypervisor),
        ])
        .await;
        let (factory, decoder) = production_guest_driver(&inputs).await;

        let manager = guest_manager();
        manager.add(
            stored_row("Provider", "runtime-cloud-hypervisor", serde_json::json!({})),
            ResourceStatus::Ready,
        );
        let mut ctx = guest_context(
            stored_row(
                "Guest",
                GUEST_ROW_NAME,
                serde_json::json!({
                    "providerRef": runtime_provider_ref(GuestKind::CloudHypervisor),
                }),
            ),
            decoder,
            Arc::clone(&manager),
        );
        let mut driver = factory.create(ctx.key()).await;
        driver
            .reconcile(&mut ctx)
            .await
            .expect("the composed driver reconciles the Cloud Hypervisor row");

        let calls = scripted.call_order();
        assert!(
            calls.contains(&"ensure-session:Guest/worker".to_owned()),
            "the composed driver established no Cloud Hypervisor target session: {calls:?}"
        );
        assert!(
            calls.contains(&"reconcile-ch:Guest/worker".to_owned()),
            "the composed driver never reached the Cloud Hypervisor controller: {calls:?}"
        );
        assert_eq!(
            committed_children(&mut ctx).await,
            Vec::new(),
            "the Cloud Hypervisor kind commits no manager children: its fixed child roles \
             belong to the controller session"
        );
    }

    /// The qemu-media runtime Provider: the composed driver materializes the
    /// runtime Volume and the qemu worker Process the qemu-media Provider
    /// derives, and the Process spec is that Provider's own worker template
    /// over the committed execution reference and runtime mount.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread")]
    async fn the_production_guest_driver_commits_the_qemu_media_runtime_children() {
        let (_dir, inputs, _readiness, _scripted) =
            production_guest_inputs(&[runtime_provider_ref(GuestKind::QemuMedia)]).await;
        let (factory, decoder) = production_guest_driver(&inputs).await;
        let manager = guest_manager();
        manager.add(
            stored_row(
                "Provider",
                "runtime-qemu-media",
                serde_json::json!({
                    "config": serde_json::to_value(
                        d2b_provider_guest_qemu_media::ProviderConfig::new(
                            "Host/host-system",
                            "qemu-system-x86-64",
                            "Provider/network-local",
                            "Provider/volume-local",
                            None,
                        )
                        .expect("qemu provider config"),
                    )
                    .expect("provider config document"),
                }),
            ),
            ResourceStatus::Ready,
        );
        let mut ctx = guest_context(
            stored_row(
                "Guest",
                GUEST_ROW_NAME,
                serde_json::json!({
                    "providerRef": runtime_provider_ref(GuestKind::QemuMedia),
                }),
            ),
            decoder,
            Arc::clone(&manager),
        );
        let mut driver = factory.create(ctx.key()).await;
        driver
            .reconcile(&mut ctx)
            .await
            .expect("the composed driver reconciles the qemu-media row");

        assert_eq!(
            manager.ensure_order(),
            vec![
                "ensure:Volume/worker-runtime".to_owned(),
                "ensure:Process/worker-qemu".to_owned(),
            ],
            "the qemu-media kind commits the runtime Volume before the Process that consumes it"
        );
        let children = committed_children(&mut ctx).await;
        let volume = &children
            .iter()
            .find(|(key, _)| key == "Volume/worker-runtime")
            .expect("the runtime Volume child")
            .1;
        assert_eq!(volume["providerRef"], "Provider/volume-local");
        assert_eq!(volume["kind"], "ephemeral");
        let process = &children
            .iter()
            .find(|(key, _)| key == "Process/worker-qemu")
            .expect("the qemu worker Process child")
            .1;
        assert_eq!(
            process["sandbox"]["seccompClass"], "qemu-media-runner",
            "the worker Process is the qemu-media Provider's own seccomp template"
        );
        assert_eq!(process["processClass"], "worker");
        assert_eq!(process["providerRef"], "Provider/system-minijail");
        assert_eq!(process["executionRef"], "Host/host-system");
        assert_eq!(
            process["mounts"][0]["volumeRef"], "Volume/worker-runtime",
            "the worker mounts the runtime Volume the pass committed first"
        );
        assert_eq!(process["mounts"][0]["mountPath"], "/run/qemu");
        assert_eq!(process["mounts"][0]["access"], "read-write");
    }

    /// The two Azure runtime Providers take different branches of the same
    /// composed driver: azure-container-apps owns the sandbox-agent control
    /// Endpoint its provider identity serves, and azure-virtual-machine
    /// realizes itself through no manager child at all. A row dispatched to
    /// the wrong branch would swap those two outcomes.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread")]
    async fn the_production_guest_driver_separates_the_two_azure_runtime_providers() {
        let (_dir, inputs, _readiness, _scripted) = production_guest_inputs(&[
            runtime_provider_ref(GuestKind::AzureContainerApps),
            runtime_provider_ref(GuestKind::AzureVirtualMachine),
        ])
        .await;
        let (factory, decoder) = production_guest_driver(&inputs).await;

        let container_apps = guest_manager();
        container_apps.add(
            stored_row(
                "Provider",
                "runtime-azure-container-apps",
                serde_json::json!({
                    "config": runtime_provider_ref(GuestKind::AzureContainerApps),
                }),
            ),
            ResourceStatus::Ready,
        );
        let mut ctx = guest_context(
            stored_row(
                "Guest",
                GUEST_ROW_NAME,
                serde_json::json!({
                    "providerRef": runtime_provider_ref(GuestKind::AzureContainerApps),
                }),
            ),
            Arc::clone(&decoder),
            Arc::clone(&container_apps),
        );
        let mut driver = factory.create(ctx.key()).await;
        let _ = driver.reconcile(&mut ctx).await;
        let children = committed_children(&mut ctx).await;
        let control = children
            .iter()
            .find(|(key, _)| key == "Endpoint/worker-sandbox-agent")
            .map(|(_, spec)| spec.clone())
            .expect("the azure-container-apps kind commits its sandbox-agent control Endpoint");
        assert_eq!(
            control["providerRef"],
            runtime_provider_ref(GuestKind::AzureContainerApps)
        );
        assert_eq!(control["endpointClass"], "control");
        assert_eq!(control["purpose"], "aca-sandbox-agent");
        assert_eq!(
            control["consumerPolicy"]["allowedSubjects"],
            serde_json::json!([runtime_provider_ref(GuestKind::AzureContainerApps)]),
            "the control Endpoint admits exactly its own Provider identity"
        );

        let virtual_machine = guest_manager();
        virtual_machine.add(
            stored_row(
                "Provider",
                "runtime-azure-virtual-machine",
                serde_json::json!({}),
            ),
            ResourceStatus::Ready,
        );
        let mut ctx = guest_context(
            stored_row(
                "Guest",
                GUEST_ROW_NAME,
                serde_json::json!({
                    "providerRef": runtime_provider_ref(GuestKind::AzureVirtualMachine),
                }),
            ),
            Arc::clone(&decoder),
            Arc::clone(&virtual_machine),
        );
        let mut driver = factory.create(ctx.key()).await;
        let _ = driver.reconcile(&mut ctx).await;
        assert_eq!(
            virtual_machine.ensure_order(),
            Vec::<String>::new(),
            "the azure-virtual-machine kind realizes itself through no manager child"
        );
        assert_eq!(
            committed_children(&mut ctx).await,
            Vec::new(),
            "the azure-virtual-machine kind committed the container-apps control Endpoint"
        );
    }

    /// A Guest row whose `providerRef` names a Provider this family does not
    /// own is refused by the composed driver, not guessed onto one of the four
    /// runtime branches.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread")]
    async fn the_production_guest_driver_refuses_a_provider_the_family_does_not_own() {
        let (_dir, inputs, _readiness) = test_inputs();
        let (factory, decoder) = production_guest_driver(&inputs).await;
        let manager = guest_manager();
        let mut ctx = guest_context(
            stored_row(
                "Guest",
                GUEST_ROW_NAME,
                serde_json::json!({ "providerRef": "Provider/volume-local" }),
            ),
            decoder,
            Arc::clone(&manager),
        );
        let mut driver = factory.create(ctx.key()).await;
        let failure = driver
            .reconcile(&mut ctx)
            .await
            .expect_err("a Provider the Guest family does not own is refused");
        assert!(failure.to_string().contains("providerRef"), "{failure}");
        assert_eq!(
            manager.ensure_order(),
            Vec::<String>::new(),
            "a refused row commits nothing"
        );
    }

    // -----------------------------------------------------------------------
    // Production-path coverage for the telemetry rows.
    //
    // The plane wires the two telemetry types through the per-type
    // declarations below the generated registration table rather than through
    // it, so nothing in the generated closure names them and a family that
    // stopped being wired would leave both rows unserved with no failure
    // anywhere. These tests take the drivers that wiring registered and drive
    // stored rows through them.
    // -----------------------------------------------------------------------

    /// The driver factory and spec decoder one resource type's production
    /// registration served.
    async fn production_driver_for(
        inputs: &ConstructionInputs,
        type_name: &str,
    ) -> (Arc<dyn ResourceDriverFactory>, Arc<dyn SpecDecoder>) {
        let mut runtime = ResourcePlaneV3::start_providers(inputs)
            .await
            .expect("the production composition starts its providers");
        let directory = runtime.take_directory();
        let name = ResourceTypeName::new(type_name);
        let factory = directory
            .lookup(&name)
            .unwrap_or_else(|| panic!("the production composition registers {type_name}"));
        let decoder = directory
            .decoders()
            .get(&name)
            .cloned()
            .unwrap_or_else(|| panic!("the production composition decodes {type_name}"));
        runtime.drain().await.expect("the providers drain");
        (factory, decoder)
    }

    fn telemetry_context(
        row: StoredDesiredResource,
        decoder: Arc<dyn SpecDecoder>,
        manager: Arc<RecordingManagerEndpoint>,
    ) -> ResourceContext {
        let (effects_tx, _effects_rx) = tokio::sync::mpsc::unbounded_channel();
        let (notify_tx, _notify_rx) = tokio::sync::mpsc::unbounded_channel();
        ResourceContext::new(
            row,
            decoder,
            manager,
            Arc::new(RecordingRequeue::default()),
            effects_tx,
            notify_tx,
        )
    }

    /// The `TelemetryBinding` row the production-registered driver realizes:
    /// the Serving Provider's own collector Process and its ingest Endpoint,
    /// named by that Provider's declared child set.
    ///
    /// The Service row the Binding admits must itself be observed Ready
    /// through the manager, so an absent or unready Service leaves the row
    /// fenced with no child committed.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread")]
    async fn the_production_telemetry_binding_driver_materializes_the_serving_provider_children() {
        const BINDING_TYPE: &str = "telemetry.d2bus.org.TelemetryBinding";
        const SERVICE_TYPE: &str = "telemetry.d2bus.org.TelemetryService";
        let (_dir, inputs, _readiness) = test_inputs();
        let (factory, decoder) = production_driver_for(&inputs, BINDING_TYPE).await;

        let manager = Arc::new(
            RecordingManagerEndpoint::new()
                .with_zone("test")
                .with_owner_uid(GUEST_ROW_UID),
        );
        manager.add(
            stored_row(
                SERVICE_TYPE,
                "ingest",
                serde_json::json!({
                    "providerRef": "Provider/observability-otel",
                    "serviceRole": "authority",
                    "ingestEndpointRefs": ["Endpoint/ingest"],
                    "signals": ["metrics"],
                    "quota": {},
                    "policy": {},
                }),
            ),
            ResourceStatus::Ready,
        );
        manager.add(
            stored_row("Zone", "test", serde_json::json!({})),
            ResourceStatus::Ready,
        );
        let mut ctx = telemetry_context(
            stored_row(
                BINDING_TYPE,
                "metrics",
                serde_json::json!({
                    "providerRef": "Provider/observability-otel",
                    "serviceRef": format!("{SERVICE_TYPE}/ingest"),
                    "producerRef": "Zone/test",
                }),
            ),
            decoder,
            Arc::clone(&manager),
        );
        let mut driver = factory.create(ctx.key()).await;
        driver
            .reconcile(&mut ctx)
            .await
            .expect("the composed driver reconciles the telemetry Binding row");

        let children = committed_children(&mut ctx).await;
        let endpoint = children
            .iter()
            .find(|(key, _)| key.starts_with("Endpoint/"))
            .map(|(key, spec)| (key.clone(), spec.clone()))
            .expect("the collector's ingest Endpoint child");
        assert!(
            endpoint.0.ends_with("-ingest-endpoint"),
            "the Endpoint child is the Serving Provider's declared ingest endpoint: {}",
            endpoint.0
        );
        assert_eq!(
            endpoint.1["providerRef"], "Provider/observability-otel",
            "the ingest Endpoint is served by the telemetry Serving Provider"
        );
        assert_eq!(endpoint.1["endpointClass"], "service");
        assert_eq!(endpoint.1["purpose"], "ingest-endpoint");
        let collector = children
            .iter()
            .find(|(key, _)| key.starts_with("Process/"))
            .map(|(key, spec)| (key.clone(), spec.clone()))
            .expect("the collector Process child");
        assert!(
            collector.0.ends_with("-collector"),
            "the Process child is the Serving Provider's declared collector: {}",
            collector.0
        );
        assert_eq!(
            collector.1["providerRef"], "Provider/system-minijail",
            "the collector is launched by the Process controller under the minijail Provider"
        );
        assert_eq!(
            collector.1["executionRef"], "Host/host-system",
            "the collector runs on the host the Serving Provider declares"
        );
    }

    /// A Service the Serving Provider has not made Ready admits no ingest
    /// route, so the composed Binding driver fences instead of committing a
    /// collector whose endpoint relationship was never admitted.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread")]
    async fn the_production_telemetry_binding_driver_fences_on_an_unadmitted_service() {
        const BINDING_TYPE: &str = "telemetry.d2bus.org.TelemetryBinding";
        const SERVICE_TYPE: &str = "telemetry.d2bus.org.TelemetryService";
        let (_dir, inputs, _readiness) = test_inputs();
        let (factory, decoder) = production_driver_for(&inputs, BINDING_TYPE).await;

        let manager = Arc::new(
            RecordingManagerEndpoint::new()
                .with_zone("test")
                .with_owner_uid(GUEST_ROW_UID),
        );
        manager.add(
            stored_row(
                SERVICE_TYPE,
                "ingest",
                serde_json::json!({
                    "providerRef": "Provider/observability-otel",
                    "serviceRole": "authority",
                    "ingestEndpointRefs": ["Endpoint/ingest"],
                    "signals": ["metrics"],
                    "quota": {},
                    "policy": {},
                }),
            ),
            ResourceStatus::Pending,
        );
        manager.add(
            stored_row("Zone", "test", serde_json::json!({})),
            ResourceStatus::Ready,
        );
        let mut ctx = telemetry_context(
            stored_row(
                BINDING_TYPE,
                "metrics",
                serde_json::json!({
                    "providerRef": "Provider/observability-otel",
                    "serviceRef": format!("{SERVICE_TYPE}/ingest"),
                    "producerRef": "Zone/test",
                }),
            ),
            decoder,
            Arc::clone(&manager),
        );
        let mut driver = factory.create(ctx.key()).await;
        driver
            .reconcile(&mut ctx)
            .await
            .expect("the composed driver fences the row");
        assert_eq!(
            manager.ensure_order(),
            Vec::<String>::new(),
            "an unadmitted Service admits no ingest route, so no collector is materialized"
        );
    }

    /// The `TelemetryService` row: a projection Service publishes its own
    /// Ready projection from the composed driver, without reading any
    /// dependency the ResourceContext surface cannot answer.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread")]
    async fn the_production_telemetry_service_driver_publishes_the_projection_phase() {
        const SERVICE_TYPE: &str = "telemetry.d2bus.org.TelemetryService";
        let (_dir, inputs, _readiness) = test_inputs();
        let (factory, decoder) = production_driver_for(&inputs, SERVICE_TYPE).await;

        let manager = Arc::new(
            RecordingManagerEndpoint::new()
                .with_zone("test")
                .with_owner_uid(GUEST_ROW_UID),
        );
        let mut ctx = telemetry_context(
            stored_row(
                SERVICE_TYPE,
                "edge",
                serde_json::json!({
                    "providerRef": "Provider/observability-otel",
                    "serviceRole": "projection",
                    "ingestEndpointRefs": ["Endpoint/edge"],
                    "signals": ["traces"],
                    "quota": {},
                    "policy": {},
                }),
            ),
            decoder,
            Arc::clone(&manager),
        );
        let mut driver = factory.create(ctx.key()).await;
        driver
            .reconcile(&mut ctx)
            .await
            .expect("the composed driver reconciles the telemetry Service row");
        let status = ctx
            .status::<d2b_provider_telemetry_service::TelemetryServiceStatus>()
            .expect("the composed driver published the Service status");
        assert_eq!(
            status.phase,
            d2b_provider_telemetry_service::TelemetryServicePhase::Ready,
            "a projection Service needs no ingest route to be Ready"
        );
        assert_eq!(
            manager.ensure_order(),
            Vec::<String>::new(),
            "a Service realizes nothing on a target, so the pass commits no child"
        );
    }


    // -----------------------------------------------------------------------
    // Production-path coverage for the display Wayland policy row.
    //
    // The display rows carry no Provider selector and no resource children,
    // so the only thing that decides which family branch a row takes is the
    // `InteractionType` the composition built its descriptor over. A policy
    // row wired to the session branch would report Pending against a
    // dependency set it never declares, with no failure anywhere; a session
    // row wired to the policy branch would report Ready while realizing
    // nothing.
    // -----------------------------------------------------------------------

    /// The composed display policy driver publishes the family's own Ready
    /// projection and mutates no row.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread")]
    async fn the_production_wayland_policy_driver_publishes_its_own_family_projection() {
        const POLICY_TYPE: &str = "display-wayland.d2bus.org.WaylandPolicy";
        let (_dir, inputs, _readiness) = test_inputs();
        let (factory, decoder) = production_driver_for(&inputs, POLICY_TYPE).await;

        let manager = Arc::new(
            RecordingManagerEndpoint::new()
                .with_zone("test")
                .with_owner_uid(GUEST_ROW_UID),
        );
        let mut ctx = telemetry_context(
            stored_row(POLICY_TYPE, "default", serde_json::json!({})),
            decoder,
            Arc::clone(&manager),
        );
        let mut driver = factory.create(ctx.key()).await;
        driver
            .reconcile(&mut ctx)
            .await
            .expect("the composed driver reconciles the display policy row");
        let status = ctx
            .status::<d2b_provider_wayland_policy::interaction::InteractionDriverStatus>()
            .expect("the composed driver published the row status");
        assert!(
            status.ready,
            "a display policy row reaches its own family branch, which realizes nothing and              reports Ready"
        );
        assert_eq!(
            manager.ensure_order(),
            Vec::<String>::new(),
            "a display policy row realizes no resource children"
        );
    }

    // -----------------------------------------------------------------------
    // Restart ordering for the display committed-shape vocabulary (F5).
    //
    // The vocabulary was committed by the session's own reconcile and by
    // nothing else, and a restart starts one actor per durable row at once: an
    // `Endpoint` child row whose `WaylandSession` had not reconciled yet asked
    // a vocabulary holding nothing, was refused `ShapeUnsupported` for good,
    // and never requeued. The plane now rebuilds the vocabulary from the
    // durable rows before the spawn, so a restart does not depend on reconcile
    // order.
    // -----------------------------------------------------------------------

    /// One admitted display session, exactly as a durable row carries it.
    fn display_session_spec() -> WaylandSessionSpec {
        WaylandSessionSpec::new(
            ResourceRef::parse("Guest/work").expect("guest ref"),
            ResourceRef::parse("Host/host-system").expect("host ref"),
            ResourceRef::parse("User/alice").expect("user ref"),
            ResourceRef::parse("display-wayland.d2bus.org.WaylandPolicy/default")
                .expect("policy ref"),
            d2b_provider_display_wayland::DisplayIdentity::new(
                "work",
                "#112233",
                "#223344",
                "#334455",
            )
            .expect("display identity"),
            true,
        )
        .expect("a cross-domain session")
    }

    /// The durable row identity one admitted session is keyed by.
    fn display_session_row(zone: &str, name: &str) -> (ResourceKey, ResourceUid) {
        let key = ResourceKey::new(zone, WAYLAND_SESSION_TYPE, name);
        let uid = resource_uid(&d2b_resource_runtime::manager::deterministic_uid(&key))
            .expect("the manager's deterministic uid is UUIDv4-shaped");
        (key, uid)
    }

    /// The `Endpoint` child rows one session's durable derivation builds, each
    /// with the manager row the store holds for it and the spec that row
    /// carries. This is the very derivation the live admission path commits
    /// the vocabulary from, so the rows and the shapes cannot drift apart.
    fn display_endpoint_child_rows(
        session_ref: &ResourceRef,
        session_uid: &ResourceUid,
        spec: &WaylandSessionSpec,
    ) -> Vec<(ResourceKey, d2b_provider_display_wayland::EndpointSpec)> {
        session_children::display_owned_child_intents(
            &ZoneId::parse("test").expect("zone"),
            session_ref,
            session_uid,
            spec,
            1,
        )
        .expect("the durable child derivation")
        .into_iter()
        .filter(|intent| intent.target().resource_type().as_str() == "Endpoint")
        .map(|intent| {
            let key = ResourceKey::new(
                "test",
                intent.target().resource_type().as_str(),
                intent.target().name().as_str(),
            );
            let value: serde_json::Value =
                serde_json::from_slice(intent.canonical_resource()).expect("child envelope");
            let endpoint: d2b_provider_display_wayland::EndpointSpec =
                serde_json::from_value(value["spec"].clone()).expect("endpoint spec");
            (key, endpoint)
        })
        .collect()
    }

    /// Commit one durable row the way a previous boot's manager left it. No
    /// broker in this fixture: the recording publisher fences and accepts what
    /// the store publishes.
    async fn commit_durable_row(
        store: &SpecStore,
        key: &ResourceKey,
        owner_uid: Option<[u8; 16]>,
        spec: &[u8],
    ) {
        let publisher = d2b_resource_runtime::test_support::RecordingPublisher::new();
        store
           .publish(
                d2b_resource_runtime::DesiredMutation::Ensure(StoredDesiredResource {
                    uid: d2b_resource_runtime::manager::deterministic_uid(key),
                    key: key.clone(),
                    generation: 1,
                    owner_uid,
                    provenance: d2b_resource_runtime::identity::ResourceProvenance::Nix,
                    deleting: false,
                    spec: spec.to_vec(),
                    metadata: br#"{"annotations":{},"labels":{},"ownerRef":null}"#.to_vec(),
                    created_at: 0,
                }),
                publisher.as_ref(),
            )
           .await
            .expect("the durable row committed");
    }

    /// Every status one row published, polled until its first pass leaves the
    /// pre-pass phases.
    ///
    /// `Pending`, `Recovering`, and `Reconciling` are what an actor publishes
    /// while its pass is still running, so a test that stopped there has
    /// observed nothing about the failure it is about - and a terminal refusal
    /// publishes once and never requeues, so it stays readable for as long as
    /// the row does. The budget is this test's own, not the actor's.
    async fn observed_statuses(
        plane: &ResourcePlaneV3,
        key: &ResourceKey,
        budget: Duration,
    ) -> Vec<ResourceStatus> {
        let deadline = tokio::time::Instant::now() + budget;
        let mut seen: Vec<ResourceStatus> = Vec::new();
        loop {
            let view = plane
               .client()
               .get(key.clone())
               .await
               .expect("the manager serves the row")
               .unwrap_or_else(|| panic!("the manager holds {key}"));
            if let Some(status) = view.observed_status()
                && !seen.contains(&status)
            {
                let converged = matches!(
                    status,
                    ResourceStatus::Ready | ResourceStatus::Failed(_) | ResourceStatus::Deleting
                );
                seen.push(status);
                if converged {
                    return seen;
                }
            }
            if tokio::time::Instant::now() >= deadline {
                return seen;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// The restore rebuilds the vocabulary from the durable rows ALONE: no
    /// actor runs here and no session reconciles, so every admitted shape can
    /// only have come from the store. The Zone scope is exact - a session
    /// homed in another Zone is not this plane's vocabulary - and a row whose
    /// spec no longer decodes is skipped rather than failing the restore,
    /// because that row's own actor refuses it on the same derivation.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread")]
    async fn the_display_vocabulary_is_rebuilt_from_the_durable_rows_alone() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = SpecStore::open(dir.path().join("spec-store.sqlite3")).expect("store");
        let zone = ZoneId::parse("test").expect("zone");
        let spec = display_session_spec();
        let spec_bytes = serde_json::to_vec(&spec).expect("spec bytes");

        let (session_key, session_uid) = display_session_row("test", "display-work");
        commit_durable_row(&store, &session_key, None, &spec_bytes).await;
        let (elsewhere_key, elsewhere_uid) = display_session_row("elsewhere", "display-other");
        commit_durable_row(&store, &elsewhere_key, None, &spec_bytes).await;
        let (broken_key, _) = display_session_row("test", "display-broken");
        commit_durable_row(
            &store,
            &broken_key,
            None,
            br#"{"guestRef":"Guest/not-a-session"}"#,
        )
        .await;

        let vocabulary = SharedDisplayEndpointVocabulary::new();
        let restored = restore_display_endpoint_vocabulary(&store, &zone, &vocabulary)
            .await
            .expect("the durable rows read back");
        assert_eq!(
            restored, 1,
            "only this Zone's decodable session contributes shapes"
        );

        let admitted = |reference: &ResourceRef, uid: &ResourceUid| {
            display_endpoint_child_rows(reference, uid, &spec)
                .into_iter()
                .all(|(_, endpoint)| {
                    d2b_provider_endpoint::endpoint_realization(&endpoint, &vocabulary)
                        .is_some()
                })
        };
        assert!(
            admitted(
                &ResourceRef::parse("display-wayland.d2bus.org.WaylandSession/display-work")
                    .expect("session ref"),
                &session_uid
            ),
            "this Zone's durable session has its committed shapes admitted"
        );
        assert!(
            !admitted(
                &ResourceRef::parse("display-wayland.d2bus.org.WaylandSession/display-other")
                    .expect("session ref"),
                &elsewhere_uid
            ),
            "another Zone's session is not this plane's vocabulary"
        );
    }

    /// A restart does not depend on reconcile order: the plane's production
    /// open admits a display `Endpoint` row whose `WaylandSession` has not
    /// reconciled. Before the plane starts, the exact admission question the
    /// Endpoint driver's `check_shape` asks refuses every one of those rows -
    /// that is the premise, and a refusal there is terminal. After the plane
    /// opens the same question admits all of them, and no endpoint actor ends
    /// up in a `Failed` status that requeues nothing.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread")]
    async fn a_restart_admits_display_endpoint_rows_before_their_session_reconciles() {
        let (_dir, inputs, _readiness) = test_inputs();
        let vocabulary = Arc::clone(&inputs.display_endpoint_vocabulary);
        let session_ref =
            ResourceRef::parse("display-wayland.d2bus.org.WaylandSession/display-work")
                .expect("session ref");
        let spec = display_session_spec();
        let (session_key, session_uid) =
            display_session_row("test", session_ref.name().as_str());
        let rows = display_endpoint_child_rows(&session_ref, &session_uid, &spec);
        assert_eq!(rows.len(), 3, "one session derives three endpoint rows");

        // The previous boot left the session and its children durable; nothing
        // has admitted their shapes yet, which is the whole failure mode.
        let store = SpecStore::open(ResourcePlaneV3::spec_store_path(&inputs.spec_store_dir))
            .expect("store");
        commit_durable_row(
            &store,
            &session_key,
            None,
            &serde_json::to_vec(&spec).expect("spec bytes"),
        )
        .await;
        for (key, endpoint) in &rows {
            assert!(
                d2b_provider_endpoint::endpoint_realization(endpoint, &*vocabulary).is_none(),
                "nothing has committed {key} yet: the premise"
            );
            commit_durable_row(
                &store,
                key,
                Some(d2b_resource_runtime::manager::deterministic_uid(
                    &session_key,
                )),
                &serde_json::to_vec(endpoint).expect("endpoint spec bytes"),
            )
            .await;
        }
        drop(store);

        let plane = ResourcePlaneV3::open(inputs)
            .await
            .expect("the plane opens over the durable rows");
        for (key, endpoint) in &rows {
            assert!(
                d2b_provider_endpoint::endpoint_realization(endpoint, &*vocabulary).is_some(),
                "the plane rebuilt {key}'s committed shape before the manager spawned an actor"
            );
            // A terminal refusal publishes at `validate`, the first step of
            // the actor's first pass, and never requeues - so it is readable
            // within milliseconds and stays readable. The budget only has to
            // outlast that first pass, not the row's convergence: these rows
            // keep reconciling in this fixture and never settle on their own.
            let observed = observed_statuses(&plane, key, Duration::from_secs(2)).await;
            assert!(
                !observed.is_empty(),
                "{key} published no status the plane can read"
            );
            for status in observed {
                if let ResourceStatus::Failed(failure) = status {
                    assert!(
                        failure.defers(),
                        "{key} ended in a terminal failure with no requeue: {}",
                        failure.report().code()
                    );
                }
            }
        }
        plane.shutdown().await;
    }
    // -----------------------------------------------------------------------
    // Production-composition acceptance for the display actor graph.
    //
    // One manager-owned `WaylandSession` row, admitted through the plane's own
    // Nix ingest, reconciled by the real per-Zone manager, the real
    // ProviderSet, the real driver factories, the real interaction effects
    // service, and the real Endpoint-family committed-shape seam. Nothing here
    // stands in for the plane: the composition supplies the facet sets the
    // production composition root supplies, each built by its own family
    // crate, and the plane assembles and runs every actor itself.
    //
    // Why this test exists at all (F5): the display Provider owns the endpoint
    // shapes it commits, and the ONE vocabulary it commits them into is
    // installed in two seams - the session driver's child-intent source and
    // the Endpoint family's committed-shape source. Until the plane wired that
    // one object into both, a fixture could hand the Endpoint family its own
    // always-empty registry, and every display `Endpoint` row would be refused
    // `ShapeUnsupported` for a shape the display Provider does commit, with no
    // test over the composition able to see why. Every assertion below runs
    // through both seams at once, because a fixture that wires only one of
    // them proves nothing about the graph production realizes.
    // -----------------------------------------------------------------------

    /// The admitted session's row name in this scene.
    const DISPLAY_SESSION_NAME: &str = "display-work";

    /// The scene's Guest row name, which the session spec names as its
    /// subject.
    const DISPLAY_GUEST_NAME: &str = "work";

    /// The session's committed interaction identity: the row's own reference
    /// and durable uid, and the Guest, Host, and User references its spec
    /// must name. This is the bounded subset the daemon resolves from its
    /// durable Zone authority and hands the family as a facet.
    fn display_session_identity() -> d2b_provider_wayland_policy::InteractionEffectIdentity {
        let (session_key, session_uid) = display_session_row("test", DISPLAY_SESSION_NAME);
        assert_eq!(session_key.type_name, WAYLAND_SESSION_TYPE);
        d2b_provider_wayland_policy::InteractionEffectIdentity {
            wayland_session_ref: ResourceRef::parse(&format!(
                "{WAYLAND_SESSION_TYPE}/{DISPLAY_SESSION_NAME}"
            ))
            .expect("the session's own canonical reference"),
            wayland_session_uid: session_uid,
            subject_ref: ResourceRef::parse("Guest/work").expect("guest ref"),
            host_execution_ref: ResourceRef::parse("Host/host-system").expect("host ref"),
            user_ref: ResourceRef::parse("User/alice").expect("user ref"),
        }
    }

    /// The committed identity facet, resolved from the row identities the
    /// manager itself derives rather than from anything the reconcile asked
    /// for.
    struct CommittedSessionIdentity(d2b_provider_wayland_policy::InteractionEffectIdentity);

    #[async_trait::async_trait]
    impl InteractionIdentitySource for CommittedSessionIdentity {
        async fn identity(&self) -> Option<d2b_provider_wayland_policy::InteractionEffectIdentity> {
            Some(self.0.clone())
        }
    }

    /// The zone's manager-plane row reads, over the very client the plane
    /// hands its own callers: this facet is the same manager, reached the way
    /// the production composition reaches it. The cell is filled the moment
    /// the plane is open and before any display row is admitted, so no
    /// reconcile can read a manager this facet does not yet hold.
    struct ManagerPlaneRead(Arc<std::sync::OnceLock<ResourceManagerClient>>);

    #[async_trait::async_trait]
    impl InteractionPlaneRead for ManagerPlaneRead {
        async fn get(&self, key: &ResourceKey) -> Result<Option<ResourceView>, ()> {
            self.0
                .get()
                .ok_or(())?
                .get(key.clone())
                .await
                .map_err(|_| ())
        }

        async fn list(&self, selector: &ResourceSelector) -> Result<Vec<ResourceView>, ()> {
            self.0
                .get()
                .ok_or(())?
                .list(selector.clone())
                .await
                .map_err(|_| ())
        }
    }

    /// A Zone whose targets declare no audio capability, which is what the
    /// display path needs and nothing more.
    struct NoAudioCapability;

    impl AudioMediatorSource for NoAudioCapability {
        fn build(&self, _vm_name: &str, _projection: bool) -> Option<Box<dyn AudioMediator>> {
            None
        }
    }

    /// One exact-endpoint request this scene's broker end received.
    ///
    /// The call log is the only thing this scene keeps about its own wire.
    /// It exists because a delivery a test cannot see is a delivery it cannot
    /// order: the revoke-before-retire claim is about frames on this socket,
    /// so the frames have to be readable.
    #[derive(Debug, Clone, PartialEq, Eq)]
    struct BrokerEndpointCall {
        verb: EndpointAccessVerb,
        endpoint: String,
        consumer: String,
        /// Whether this end ANSWERED the request rather than refusing it.
        ///
        /// A refused request is still a request that crossed the wire, and the
        /// two are not the same fact: a scene that holds one endpoint row has
        /// to be able to tell "this relationship was delivered" from "this
        /// relationship asked and was refused", or a reader waiting for the
        /// delivery would be satisfied by the refusal.
        answered: bool,
        at: std::time::Instant,
    }

    /// The scene's broker end of the Endpoint family's privileged dispatch.
    ///
    /// The delivery the display graph publishes rides the daemon's REAL
    /// dispatch: [`crate::DaemonEndpointAccessDispatch`] over
    /// [`crate::ServerState`], a `SOCK_SEQPACKET` connection, a length-prefixed
    /// [`BrokerRequestEnvelope`], and the one [`BrokerResponse::EndpointAccess`]
    /// the Endpoint family reads. This object is the far end of that socket
    /// and nothing more - it does not stub the dispatch, does not name a
    /// relationship's verdict, and holds no plane state. What it does own is
    /// the ONE thing a broker owns that a fixture may script: whether a named
    /// endpoint's requests are answered or refused, and the record of what
    /// arrived.
    struct EndpointAccessBroker {
        socket_path: PathBuf,
        /// Endpoints whose requests are refused rather than answered. A held
        /// endpoint answers with the broker's own closed refusal frame, which
        /// is what a real broker does for an admission it will not perform.
        held: std::sync::Arc<std::sync::Mutex<std::collections::BTreeSet<String>>>,
        calls: std::sync::Arc<std::sync::Mutex<Vec<BrokerEndpointCall>>>,
    }

    impl EndpointAccessBroker {
        /// Refuse every request naming this endpoint row from now on.
        fn hold(&self, endpoint: &ResourceKey) {
            self.held
                .lock()
                .expect("the held-endpoint set is not poisoned")
                .insert(endpoint.name.clone());
        }

        /// Answer every request naming this endpoint row again.
        fn release(&self, endpoint: &ResourceKey) {
            self.held
                .lock()
                .expect("the held-endpoint set is not poisoned")
                .remove(&endpoint.name);
        }

        /// Every request this broker end received, in arrival order.
        fn calls(&self) -> Vec<BrokerEndpointCall> {
            self.calls
                .lock()
                .expect("the call log is not poisoned")
                .clone()
        }

        /// Every request naming `endpoint` this end ADMITTED, in arrival
        /// order: the non-observe requests it answered rather than refused.
        ///
        /// A refusal is a fact about the broker's own posture and not about
        /// what the graph published, so every reader that is asking whether an
        /// access was delivered reads this rather than the whole log.
        fn admissions(&self, endpoint: &ResourceKey) -> Vec<BrokerEndpointCall> {
            self.calls()
                .into_iter()
                .filter(|call| {
                    call.endpoint == *endpoint.name
                        && call.verb != EndpointAccessVerb::Observe
                        && call.answered
                })
                .collect()
        }

        /// The arrival record of the LAST request this end admitted for
        /// `endpoint`.
        fn last_call(&self, endpoint: &ResourceKey) -> Option<BrokerEndpointCall> {
            self.admissions(endpoint).into_iter().next_back()
        }
    }

    /// Bind the scene's broker socket and answer every exact-endpoint request
    /// that arrives on it.
    ///
    /// The frame is the production frame: the daemon connects a `SOCK_SEQPACKET`
    /// socket to this path, writes the length-prefixed
    /// [`BrokerRequestEnvelope`] through [`crate::write_json_frame`], and reads
    /// the answer with [`crate::read_frame`]. This end serves them in that same
    /// order and shape, on its own thread, because the socket it accepts on is
    /// a blocking descriptor and the plane's actors must not park on it.
    ///
    /// The answer carries the request's own endpoint, consumer, socket name,
    /// and admitted rights back: this broker grants exactly what was asked
    /// for and never hands back the directory authority R23 removed. The
    /// pinned `(device, inode)` is a fixed synthetic pair - a fixture never
    /// reads a host device number, and the value only has to be stable so a
    /// grant that is already standing reads back as the same standing one.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn start_endpoint_access_broker(dir: &Path) -> EndpointAccessBroker {
        use nix::sys::socket::{
            UnixAddr, accept4, bind, listen, socket, AddressFamily, Backlog, SockFlag, SockType,
        };

        let socket_path = dir.join("broker.sock");
        let listener = socket(
            AddressFamily::Unix,
            SockType::SeqPacket,
            SockFlag::SOCK_CLOEXEC,
            None,
        )
        .expect("create the broker listener");
        let address = UnixAddr::new(&socket_path).expect("the broker socket address");
        bind(listener.as_raw_fd(), &address).expect("bind the broker listener");
        listen(&listener, Backlog::new(16).expect("the broker backlog")).expect("listen");

        let held: Arc<std::sync::Mutex<std::collections::BTreeSet<String>>> =
            Arc::new(std::sync::Mutex::new(std::collections::BTreeSet::new()));
        let calls: Arc<std::sync::Mutex<Vec<BrokerEndpointCall>>> =
            Arc::new(std::sync::Mutex::new(Vec::new()));
        let answers = Arc::clone(&held);
        let log = Arc::clone(&calls);
        std::thread::spawn(move || {
            while let Ok(peer) = accept4(listener.as_raw_fd(), SockFlag::SOCK_CLOEXEC) {
                let Ok(frame) = crate::read_frame(&peer) else {
                    continue;
                };
                let Ok(envelope) = serde_json::from_slice::<BrokerRequestEnvelope>(&frame) else {
                    continue;
                };
                let (verb, request) = match &envelope.request {
                    BrokerRequest::EndpointObserve(request) => (EndpointAccessVerb::Observe, request),
                    BrokerRequest::EndpointGrantAccess(request) => (EndpointAccessVerb::Grant, request),
                    BrokerRequest::EndpointRevokeAccess(request) => (EndpointAccessVerb::Revoke, request),
                    _ => {
                        let _ = crate::write_json_frame(
                            &peer,
                            &BrokerResponse::Error(BrokerErrorResponse {
                                kind: "broker-unsupported-request".to_owned(),
                                operation: "EndpointAccess".to_owned(),
                                target_wave: None,
                                message: "this broker end serves exact-endpoint requests only"
                                    .to_owned(),
                                action: "none".to_owned(),
                            }),
                        );
                        continue;
                    }
                };
                let endpoint_name = request.endpoint_ref.name().as_str().to_owned();
                let held_now = answers
                    .lock()
                    .expect("the held-endpoint set is not poisoned")
                    .contains(&endpoint_name);
                log.lock()
                    .expect("the call log is not poisoned")
                    .push(BrokerEndpointCall {
                        verb,
                        endpoint: endpoint_name.clone(),
                        consumer: request.consumer_ref.to_canonical_string(),
                        answered: !held_now,
                        at: std::time::Instant::now(),
                    });
                let response = if held_now {
                    BrokerResponse::Error(BrokerErrorResponse {
                        kind: "endpoint-access-refused".to_owned(),
                        operation: verb.as_str().to_owned(),
                        target_wave: None,
                        message: "this broker end does not perform this admission".to_owned(),
                        action: "none".to_owned(),
                    })
                } else {
                    BrokerResponse::EndpointAccess(EndpointAccessResponse {
                        endpoint_ref: request.endpoint_ref.clone(),
                        consumer_ref: request.consumer_ref.clone(),
                        socket: request.socket.clone(),
                        socket_device: FIXTURE_ENDPOINT_DEVICE,
                        socket_inode: FIXTURE_ENDPOINT_INODE,
                        socket_effective_rights: u32::from(request.socket_rights),
                        ancestors_traversable: true,
                        parent_listable: false,
                        consumer_uid: request
                            .claimed_principal
                            .map_or(0, |claim| claim.uid),
                        consumer_gid: request
                            .claimed_principal
                            .map_or(0, |claim| claim.gid),
                    })
                };
                let _ = crate::write_json_frame(&peer, &response);
            }
        });
        EndpointAccessBroker {
            socket_path,
            held,
            calls,
        }
    }

    /// The synthetic `(device, inode)` pair every answer in this scene pins.
    ///
    /// It is not read from the host and never reaches a published projection -
    /// the Endpoint family redacts it - so a fixed pair is the honest fixture
    /// value: it says "this broker pins one endpoint socket", which is all the
    /// delivery verdict reads.
    const FIXTURE_ENDPOINT_DEVICE: u64 = 0x00d2;
    const FIXTURE_ENDPOINT_INODE: u64 = 0x0b00_0002;

    /// The daemon state the Endpoint family's privileged dispatch is built
    /// from when a plane carries one.
    ///
    /// This is the crate's own daemon state, field for field, with only the
    /// broker socket pointed at this scene's broker end: the production
    /// composition supplies exactly this object to the same construction site,
    /// and a plane built without one gets the family's own unwired dispatch
    /// that refuses every verb by name - which is a delivery that never
    /// exists, not a delivery this scene is choosing to fake.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn test_server_state(broker_socket_path: PathBuf, dir: &Path) -> Arc<crate::ServerState> {
        let daemon_state_dir = dir.join("daemon-state");
        std::fs::create_dir_all(&daemon_state_dir).expect("create the daemon state dir");
        let broker_reap_log = crate::BrokerReapLog::new();
        Arc::new(crate::ServerState {
            config: crate::DaemonConfig {
                broker_socket_path,
                ..crate::DaemonConfig::default()
            },
            daemon_uid: 0,
            daemon_audit: Arc::new(d2bd_runtime::daemon_audit::DaemonAuditLog::no_op()),
            daemon_state_dir: daemon_state_dir.clone(),
            pidfd_table: Arc::new(
                crate::PidfdTable::new(daemon_state_dir.join("pidfd-table.json"))
                    .with_broker_reap_log(Arc::clone(&broker_reap_log)),
            ),
            broker_reap_log,
            metrics_registry: Arc::new(d2bd_runtime::metrics::Registry::new()),
            exec_sessions: Arc::new(crate::exec_session::SessionTable::new(
                crate::exec_session::ExecSessionCaps::default(),
            )),
            console_sessions: Arc::new(tokio::sync::Mutex::new(
                crate::console_session::ConsoleSessionTable::default(),
            )),
            conn_semaphore: d2bd_runtime::concurrency::ConnSemaphore::new(8),
            op_locks: d2bd_runtime::concurrency::OpLockManager::new(),
            public_status_read_model: Arc::new(
                d2bd_runtime::public_read_model::PublicStatusReadModel::new(),
            ),
            provider_runtime: Arc::new(crate::provider_registry::ProviderRuntime::new()),
            resource_plane: Arc::new(tokio::sync::Mutex::new(None)),
            interaction_runtime: Arc::new(tokio::sync::Mutex::new(None)),
            interaction_listeners: Arc::new(tokio::sync::Mutex::new(None)),
            typed_shell_session_targets: d2bd_runtime::typed_shell_targets::new_cache(),
            zone_coordinator: d2bd_runtime::zone_authority::new_coordinator(),
            config_staging: Arc::new(tokio::sync::Mutex::new(Default::default())),
            guest_component_sessions: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            guest_component_session_locks: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            security_key_sessions: Arc::new(tokio::sync::Mutex::new(
                d2b_provider_device_security_key::SkSessionTable::default(),
            )),
            unsafe_local_helpers: Arc::new(d2bd_runtime::unsafe_local_helper::HelperRegistry::new(
                0,
                [],
            )),
            v3_planes: std::sync::Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            runtime_handle: crate::test_runtime_handle(),
        })
    }

    /// The display scene's construction, with the doubles the tests script and
    /// the broker end the production dispatch delivers over, so a test can
    /// move the delivery evidence without rebuilding the composition.
    struct DisplayComposition {
        dir: tempfile::TempDir,
        inputs: ConstructionInputs,
        client: Arc<std::sync::OnceLock<ResourceManagerClient>>,
        broker: EndpointAccessBroker,
        processes: Arc<d2b_provider_process::test_support::FakeFacets>,
    }

    /// The composition the display graph reconciles over, rooted at the
    /// caller's durable directory so a restart can reopen the same Zone.
    ///
    /// Every facet set here is built by its own family crate over that
    /// family's own scripted effect port, exactly as the production
    /// composition root builds them from the daemon's runtimes. The Guest
    /// family is given the committed runtime-Provider identity, the enrolled
    /// controller-session generation, and the committed `Provider` row its
    /// effects validate before any Provider work runs; the interaction family
    /// is given the REAL manager-plane reads and this Zone's committed
    /// identity.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn display_composition() -> DisplayComposition {
        // The Process family's own effects double, scripted the way the
        // plane's other worker-row tests script it: no retained identity
        // before the first launch, then the retained identity a real Provider
        // records once the launch it issued is serving. Without the second
        // answer the host proxy's actor adopts nothing on every pass and
        // relaunches for ever, which is a state the production provider this
        // double stands for is never in.
        let processes = adopting_process_facets();
        display_composition_with_processes(processes).await
    }

    /// The one Process double this scene admits a graph to converge over: a
    /// launch that finds no retained identity, then the retained identity the
    /// same Provider records once that launch is serving.
    fn adopting_process_facets() -> Arc<d2b_provider_process::test_support::FakeFacets> {
        let processes =
            Arc::new(d2b_provider_process::test_support::FakeFacets::new(
                Default::default(),
            ));
        processes.set_active(false);
        processes.push_adoption(d2b_provider_process::ProviderAdoption::Absent);
        processes.push_adoption(d2b_provider_process::ProviderAdoption::Adopted(
            adopted_report(),
        ));
        processes
    }

    /// The composition over a caller-chosen Process double.
    ///
    /// Every other seam is the same real one: the same scripted Guest
    /// controller session, the same scripted private host observation, the
    /// same REAL broker socket the production dispatch delivers over, and the
    /// same real manager-plane reads the interaction family answers from. Only
    /// the Process family's own scripted provider runtime moves, so a test can
    /// ask what the graph does while the provider admits no standing worker
    /// without standing up a different plane to ask it.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn display_composition_with_processes(
        processes: Arc<d2b_provider_process::test_support::FakeFacets>,
    ) -> DisplayComposition {
        let client = Arc::new(std::sync::OnceLock::new());
        let runtime_provider = runtime_provider_ref(GuestKind::CloudHypervisor);
        let provider_name = runtime_provider
            .strip_prefix("Provider/")
            .expect("a runtime Provider reference");
        let scripted = d2b_provider_guest::test_support::ScriptedFacets::new();
        scripted.add_committed_provider(
            ResourceRef::parse(runtime_provider).expect("runtime Provider reference"),
            ResourceUid::from_bytes(&[0x11; 16]).expect("bounded resource uid"),
            d2b_contracts_resource::v3::ResourceGeneration::new(1).expect("bounded generation"),
        );
        scripted.set_session_generation(Some(
            d2b_contracts_resource::v3::identity::ReconnectGeneration::new(1)
                .expect("bounded reconnect generation"),
        ));
        // The Guest family's effects read the committed runtime `Provider`
        // row through their manager facet before any Provider work runs, so
        // the scene's Guest row has one to find.
        scripted
            .add_row(d2b_provider_guest::test_support::row_fixture(
                "test",
                "Provider",
                provider_name,
                serde_json::json!({}),
                ResourceStatus::Ready,
            ))
            .await;
        let guest_facets = d2b_provider_guest::facets::GuestEffectFacets {
            zone: ZoneId::parse("test").expect("bounded zone"),
            controller_generation: ControllerGeneration::new(1).expect("bounded generation"),
            manager: Arc::clone(&scripted)
                as Arc<dyn d2b_provider_guest::facets::GuestManagerView>,
            cloud_hypervisor: Arc::clone(&scripted)
                as Arc<dyn d2b_provider_guest::facets::CloudHypervisorGuestRuntime>,
        };
        let (dir, mut inputs, _readiness) =
            test_inputs_over(
                InteractionEffectFacets::new(
                    ZoneId::parse("test").expect("bounded zone"),
                    Arc::new(CommittedSessionIdentity(display_session_identity())),
                    Arc::new(ManagerPlaneRead(Arc::clone(&client))),
                    Arc::new(NoAudioCapability),
                ),
                guest_facets,
            );
        // The serving-socket probe and the host socket surface are the two
        // facets the display path's delivery and realization evidence ride,
        // and this test scripts them directly. Both are re-bound over the SAME
        // display vocabulary the session driver commits its shapes into, so
        // the composition keeps one object in both seams.
        let serving = d2b_provider_volume_binding::test_support::FakeServingEffects::new();
        serving.make_ready();
        inputs.binding_facets = serving.facet_set();
    /// The scripted private host observation the display scene installs for
    /// the daemon's own socket facet.
    ///
    /// The production facet ([`PlaneHostSocketEvidence`]) resolves the locator
    /// the endpoint owner committed, compares the exact socket standing there
    /// and whether it accepts a connection, and mints a handle only for an
    /// observation that proved it. A scene hosts no socket to compare, so this
    /// one answers the same scripted presence the rest of the Endpoint family's
    /// double answers - and mints ONE handle per endpoint reference, because a
    /// fresh nonce on every pass is a replacement the scene never made and
    /// would rotate the realization under the binding that already read it.
    struct ScriptedHostSocketObservation {
        present: std::sync::atomic::AtomicBool,
        minted: tokio::sync::Mutex<std::collections::HashMap<String, RealizationHandle>>,
    }

    impl ScriptedHostSocketObservation {
        fn new() -> Self {
            Self {
                present: std::sync::atomic::AtomicBool::new(false),
                minted: tokio::sync::Mutex::new(std::collections::HashMap::new()),
            }
        }

        /// Script the observation as proving a realization, as a bound and
        /// connectable socket does.
        fn make_present(&self) {
            self.present.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }

    #[async_trait::async_trait]
    impl HostSocketEvidenceSource for ScriptedHostSocketObservation {
        async fn observe(
            &self,
            endpoint_ref: &ResourceRef,
            _purpose: &str,
        ) -> Option<RealizationHandle> {
            if !self.present.load(std::sync::atomic::Ordering::SeqCst) {
                return None;
            }
            let mut minted = self.minted.lock().await;
            let handle = minted
                .entry(endpoint_ref.to_canonical_string())
                .or_insert_with(|| {
                    RealizationHandle::mint(
                        realization_nonce().expect("128 bits of kernel randomness"),
                        1,
                    )
                    .expect("a full-width nonce clears the incarnation floor")
                })
                .clone();
            Some(handle)
        }
    }

        let sockets = d2b_provider_endpoint::test_support::FakeSocketEffects::new();
        sockets.make_present();
        let host_sockets = ScriptedHostSocketObservation::new();
        host_sockets.make_present();
        inputs.endpoint_facets = sockets
            .facet_set()
            .with_committed_shapes(
                Arc::clone(&inputs.display_endpoint_vocabulary)
                    as Arc<dyn CommittedEndpointShapeSource>,
            )
            .with_host_socket_observation(Arc::new(host_sockets) as Arc<dyn HostSocketEvidenceSource>);
        // The Endpoint family's privileged delivery reaches the exact-endpoint
        // ACL helpers over a broker socket only the daemon holds, so this scene
        // carries the daemon's own state pointed at this scene's broker end.
        // The EndpointBinding driver is then built with the PRODUCTION
        // dispatch rather than the family's unwired double, and every delivery
        // verdict a test below reads is one this wire answered.
        let broker = start_endpoint_access_broker(dir.path());
        inputs.server_state = Some(test_server_state(broker.socket_path.clone(), dir.path()));
        inputs.process_facets = processes.facet_set();
        DisplayComposition {
            dir,
            inputs,
            client,
            broker,
            processes,
        }
    }

    /// The durable directory the plane's own fixture created is the one this
    /// composition keeps, so a restart can reopen the same Zone over the same
    /// store.
    /// The Host row's own closed contract: the family's canonical Host spec
    /// behind the Provider selector the Host driver fences on. A row without
    /// that selector is not a Host row this family admits.
    fn display_host_spec() -> serde_json::Value {
        let mut spec =
            spec_value(&d2b_contracts_resource::v3::host::HostSpec::system_default());
        spec["providerRef"] = serde_json::Value::String(
            d2b_contracts_resource::v3::host::HOST_PROVIDER_REF.to_owned(),
        );
        spec
    }

    /// The scene's Guest side of the cross-domain session.
    ///
    /// The scene binds the crate's own [`GuestTargetRuntime`] - the runtime
    /// the daemon binds when a Guest session connects - and adds the one
    /// thing the daemon never adds: the GUEST-LOCAL effect a production
    /// Guest runs. A realization the Host asked for is not serving the
    /// moment it is recorded; the Guest's own local effect reports it with
    /// [`GuestTargetRuntime::mark_ready`] when it is. Without that report the
    /// guest frontend's row would report a realization that never stops
    /// converging, which is a state no production Guest is ever in and no
    /// assertion about this graph could be made over.
    #[derive(Debug)]
    struct GuestLocalEffect {
        control: Arc<dyn GuestTargetControl>,
        runtime: Arc<d2b_resource_runtime::guest_target::GuestTargetRuntime>,
        realized: GuestRealizations,
    }

    /// The realize frames this scene's Guest applied, in arrival order.
    ///
    /// The Guest runs the runtime's own target control behind
    /// [`GuestLocalEffect`], so this is a RECORD of the frames the production
    /// Process family wrote, read back through the family's own decoder. It
    /// observes; it decides nothing, and no actor reads it.
    #[derive(Clone, Debug, Default)]
    struct GuestRealizations(
        Arc<tokio::sync::Mutex<Vec<d2b_provider_process::worker_launch::GuestProcessRealization>>>,
    );

    impl GuestRealizations {
        /// Every frame this Guest applied, in arrival order.
        async fn frames(
            &self,
        ) -> Vec<d2b_provider_process::worker_launch::GuestProcessRealization> {
            self.0.lock().await.clone()
        }

        /// Every frame this Guest applied for one worker row, in arrival order.
        async fn frames_for(
            &self,
            process_ref: &str,
        ) -> Vec<d2b_provider_process::worker_launch::GuestProcessRealization> {
            self.frames()
                .await
                .into_iter()
                .filter(|frame| frame.process_ref() == process_ref)
                .collect()
        }
    }

    #[async_trait::async_trait]
    impl GuestTargetControl for GuestLocalEffect {
        async fn realize(
            &self,
            request: d2b_resource_runtime::guest_target::GuestRealizeRequest,
        ) -> Result<
            d2b_resource_runtime::guest_target::TargetResourceInstance,
            d2b_resource_runtime::guest_target::GuestTargetError,
        > {
            // The exact realization the Process family wrote, taken from the
            // frame itself. A frame this Guest cannot decode is not one the
            // production Guest would apply either, so the decoder's refusal
            // is the fixture's own failure, not a recorded verdict.
            self.realized
                .0
                .lock()
                .await
                .push(d2b_provider_process::worker_launch::GuestProcessRealization::decode(
                    request.spec(),
                )
                .expect("the Process family writes a decodable realization"));
            let admitted = self.control.realize(request).await?;
            self.runtime.mark_ready(admitted.source());
            // The answer a Guest gives is the instance it HOLDS once its own
            // local effect has run, not the one the request was admitted
            // into: the production `GuestTargetService::realize` re-reads it
            // for exactly this reason, and falls back to the admitted one only
            // when that read collided. Answering with the admitted instance
            // instead reported `Realizing` over a realization this Guest had
            // already served, so every guest launch in this scene was refused
            // as "the target-local effect has not converged" and the worker
            // row spent a further `PROCESS_RESYNC` pass discovering what the
            // pass that realized it had already done.
            Ok(self
                .runtime
                .instance(admitted.source())
                .unwrap_or(admitted))
        }

        async fn observe(
            &self,
            assignment: &d2b_resource_runtime::guest_target::TargetControlAssignment,
        ) -> Result<
            d2b_resource_runtime::target::TargetObservation,
            d2b_resource_runtime::guest_target::GuestTargetError,
        > {
            self.control.observe(assignment).await
        }

        async fn delete(
            &self,
            assignment: &d2b_resource_runtime::guest_target::TargetControlAssignment,
        ) -> Result<(), d2b_resource_runtime::guest_target::GuestTargetError> {
            self.control.delete(assignment).await
        }

        async fn adopt(
            &self,
            assignment: &d2b_resource_runtime::guest_target::TargetControlAssignment,
        ) -> Result<
            d2b_resource_runtime::guest_target::GuestAdoption,
            d2b_resource_runtime::guest_target::GuestTargetError,
        > {
            self.control.adopt(assignment).await
        }
    }

    /// Bind the scene's Guest target-control runtime to this plane, exactly as
    /// the daemon binds it when the Guest session connects: the runtime the
    /// runtime serves, one bound session generation, the Guest's own local
    /// effect behind it, and the plane's own target directory told to notify
    /// the affected actors.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn bind_display_guest_target(plane: &ResourcePlaneV3) -> GuestRealizations {
        let guest = TargetRef::guest(DISPLAY_GUEST_NAME).expect("the Guest target reference");
        let runtime = Arc::new(
            d2b_resource_runtime::guest_target::GuestTargetRuntime::new(guest.clone()),
        );
        runtime.bind_session(1).expect("the guest session binds");
        let realized = GuestRealizations::default();
        let control: Arc<dyn GuestTargetControl> = Arc::new(GuestLocalEffect {
            control: runtime.control(1).expect("the guest target control"),
            runtime: Arc::clone(&runtime),
            realized: realized.clone(),
        });
        plane
            .bind_guest_target(&guest, 1, control)
            .expect("the plane binds the guest target");
        realized
    }

    /// One spec rendered as the canonical JSON a durable row carries.
    fn spec_value<T: serde::Serialize>(spec: &T) -> serde_json::Value {
        serde_json::to_value(spec).expect("the canonical spec document")
    }

    /// The rows this scene's Nix bundle declares: the session's own Guest,
    /// Host, User, and policy dependencies, plus the session itself.
    ///
    /// Each dependency row carries its family's own canonical spec - the
    /// closed Host and User contracts and the runtime Provider's own Guest
    /// selector - because the session's admission refuses a dependency whose
    /// row the dependency's own driver refuses, and a hand-written stand-in
    /// would prove nothing about the graph production admits.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn display_scene_bundle(with_session: bool, reconnect_generation: u64) -> ResourceBundle {
        let mut spec = display_session_spec();
        if reconnect_generation > 0 {
            spec = spec
                .with_reconnect_generation(reconnect_generation)
                .expect("a bounded reconnect generation");
        }
        let mut rows = vec![
            // The Zone self row, exactly as the foundation seed files it. The
            // EndpointBinding driver resolves the authority the broker's
            // verified bundle is filed under by reading this committed row, so
            // a scene without one defers every derived relationship for ever.
            bundle_row("Zone", "test", serde_json::json!({})),
            bundle_row(
                "Guest",
                DISPLAY_GUEST_NAME,
                serde_json::json!({"providerRef": runtime_provider_ref(GuestKind::CloudHypervisor)}),
            ),
            bundle_row("Host", "host-system", display_host_spec()),
            bundle_row(
                "User",
                "alice",
                spec_value(&d2b_contracts_resource::v3::user::UserSpec::minimal(
                    d2b_contracts_resource::v3::user::OsUsername::parse("alice")
                        .expect("bounded username"),
                )),
            ),
            bundle_row(
                "display-wayland.d2bus.org.WaylandPolicy",
                "default",
                serde_json::json!({"providerRef": "Provider/display-wayland"}),
            ),
        ];
        if with_session {
            rows.push(bundle_row(WAYLAND_SESSION_TYPE, DISPLAY_SESSION_NAME, spec_value(&spec)));
        }
        test_bundle(rows)
    }

    /// One admitted display scene over the real composition.
    struct DisplayScene {
        _dir: tempfile::TempDir,
        plane: Arc<ResourcePlaneV3>,
        vocabulary: Arc<SharedDisplayEndpointVocabulary>,
        session_key: ResourceKey,
        session_uid: ResourceUid,
        broker: EndpointAccessBroker,
        processes: Arc<d2b_provider_process::test_support::FakeFacets>,
        /// The realize frames the Guest applied, read back out of the bytes
        /// the production Process family wrote to it.
        realized: GuestRealizations,
    }

    /// One admitted display scene over the composition that converges.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn display_scene() -> DisplayScene {
        display_scene_with_processes(adopting_process_facets()).await
    }

    /// One admitted display scene over a caller-chosen Process double.
    ///
    /// Everything else is the same real scene: the same manager, the same
    /// provider set, the same driver factories, the same REAL broker socket,
    /// and the same bound Guest target. Only the Process family's own scripted
    /// provider runtime moves, which is what lets a test ask what the graph
    /// does while no worker is standing - the one state in which a dependent
    /// has to wait for its source to prove a realization.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn display_scene_with_processes(
        processes: Arc<d2b_provider_process::test_support::FakeFacets>,
    ) -> DisplayScene {
        let DisplayComposition { dir, inputs, client, broker, processes } =
            display_composition_with_processes(processes).await;
        let vocabulary = Arc::clone(&inputs.display_endpoint_vocabulary);
        let plane = Arc::new(
            ResourcePlaneV3::open(inputs)
                .await
                .expect("the display composition opens"),
        );
        client
            .set(plane.client().clone())
            .expect("the plane's client is bound once");
        let realized = bind_display_guest_target(&plane).await;
        let (session_key, session_uid) = display_session_row("test", DISPLAY_SESSION_NAME);
        let scene = DisplayScene {
            _dir: dir,
            plane,
            vocabulary,
            session_key,
            session_uid,
            broker,
            processes,
            realized,
        };
        // The first Nix apply: the rows commit, and the actors reconcile over
        // them from here.
        scene
            .plane
            .ingest_nix_bundle(&display_scene_bundle(true, 0))
            .await
            .expect("the scene's rows commit");
        scene
    }

    /// The manager's view of one row, or a panic naming the row.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn view_of(plane: &ResourcePlaneV3, key: &ResourceKey) -> ResourceView {
        plane
            .client()
            .get(key.clone())
            .await
            .expect("the manager serves the row")
            .unwrap_or_else(|| panic!("the manager holds no row {key}"))
    }

    /// Every row of one ResourceType this Zone's manager holds, by name.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn rows_of_type(plane: &ResourcePlaneV3, type_name: &str) -> BTreeMap<String, ResourceView> {
        plane
            .client()
            .list(ResourceSelector {
                zone: Some("test".to_owned()),
                type_name: Some(type_name.to_owned()),
                owner: None,
            })
            .await
            .expect("the manager serves the Zone")
            .into_iter()
            .map(|view| (view.key.name.as_str().to_owned(), view))
            .collect()
    }

    /// One tick of the fastest reconcile cadence in this graph, in seconds:
    /// the self-requeue a `Process` row defers on (`PROCESS_RESYNC`), an
    /// unrealized `Endpoint` row on (`ENDPOINT_REALIZE_RESYNC`), and an
    /// undelivered `EndpointBinding` row on (`ENDPOINT_BINDING_RESYNC`). All
    /// three live in the driver crates and are private to them, so the number
    /// is restated here with the three it stands for named above.
    const DISPLAY_LINK_RESYNC_SECS: u64 = 5;

    /// One tick of the session row's own cadence, in seconds:
    /// `WAYLAND_SESSION_RESYNC`, the preserved display repair interval. An
    /// unconverged session row re-enters its pass on this one, so it is the
    /// slowest clock in this graph and the fallback for the aggregate read
    /// that ends every chain.
    const WAYLAND_SESSION_RESYNC_SECS: u64 = 30;

    /// How much wall clock one of those ticks costs on a machine that is not
    /// idle, as a multiple of its nominal length.
    ///
    /// Measured on this twelve-core host rather than chosen. Idle, the restart
    /// test's own convergence - the last evidence to the session row's own
    /// answer - takes 20-27s of the 60s its chain nominal, because a child
    /// publication wakes its watchers ahead of the next tick. Under load the
    /// same measurement was taken with the twelve cores oversubscribed by
    /// spinners: 60s with twelve, and 25s, 35s, 45s, 25s and 45s with
    /// twenty-four and forty-eight. Every one of those runs converged; none of
    /// them stalled. Three is the tail's measured stretch, and four is the
    /// whole first-boot chain's (25s idle against 136s under twelve spinners).
    /// Five carries the aggregate gate, which runs every suite in this
    /// workspace at once, with the tick of slack a saturated machine adds to
    /// any single pass.
    const DISPLAY_LOAD_FACTOR: u64 = 5;

    /// One tick of the fastest reconcile cadence in this graph.
    const DISPLAY_LINK_RESYNC: Duration = Duration::from_secs(DISPLAY_LINK_RESYNC_SECS);

    /// One tick of the session row's own cadence.
    const WAYLAND_SESSION_RESYNC: Duration =
        Duration::from_secs(WAYLAND_SESSION_RESYNC_SECS);

    /// The nominal cost of one display chain end to end, in seconds.
    ///
    /// Six `DISPLAY_LINK_RESYNC` links - each `Endpoint` realizing, each
    /// `EndpointBinding` delivering, and each consumer `Process` launching, in
    /// the order the graph derives them - and one `WAYLAND_SESSION_RESYNC` for
    /// the session row's aggregate read that the chain ends in.
    const DISPLAY_CHAIN_SECS: u64 = DISPLAY_LINK_RESYNC_SECS * 6 + WAYLAND_SESSION_RESYNC_SECS;

    /// The budget one EVIDENCE wait gets.
    ///
    /// An evidence wait ends when the graph has actually done the thing the
    /// assertions read, so it returns as soon as that happened and only spends
    /// this bound when the graph did not do it at all. That is the whole
    /// difference from a window: the bound stops deciding whether the test
    /// passes and goes back to deciding only how long a real failure takes to
    /// be reported.
    const DISPLAY_EVIDENCE_BUDGET: Duration =
        Duration::from_secs(DISPLAY_CHAIN_SECS * DISPLAY_LOAD_FACTOR);

    /// The budget one status read gets AFTER the evidence it is derived from
    /// has already landed.
    ///
    /// Nothing is left to chain: the actor whose effect completed publishes
    /// its own status in that same pass, and the row that aggregates it is
    /// woken by the watch on that publication. What remains is one pass plus
    /// one hop, with `WAYLAND_SESSION_RESYNC` as the fallback clock for the
    /// hop if the watch is missed, at the same load factor as the chain.
    const DISPLAY_AGGREGATE_BUDGET: Duration = Duration::from_secs(
        (DISPLAY_LINK_RESYNC_SECS + WAYLAND_SESSION_RESYNC_SECS) * DISPLAY_LOAD_FACTOR,
    );

    /// The window a NEGATIVE observation holds open: one deferral cycle of
    /// the actor whose pass is being fenced, at the load factor.
    ///
    /// A negative window has to span at least one retry to mean anything, and
    /// it is strictly stricter the longer it runs, so it is sized by the
    /// cadence rather than by how long this machine happens to take to fire
    /// one tick.
    const DISPLAY_RETRY_WINDOW: Duration =
        Duration::from_secs(DISPLAY_LINK_RESYNC_SECS * DISPLAY_LOAD_FACTOR);

    /// The window a RESTART's negative status trail holds open.
    ///
    /// The claim it has to support is that a restarted actor's answer STAYS
    /// replaced rather than drifting back to the previous boot's cached one,
    /// and the only way to observe that is across a further pass. An
    /// unconverged session row re-enters its pass on `WAYLAND_SESSION_RESYNC`,
    /// so a whole resync plus half of one covers the pass and a margin; the
    /// window opens when the row first speaks, so what it costs is the
    /// observation and not the wait for the restarted actor to be scheduled.
    const DISPLAY_RESTART_WINDOW: Duration = Duration::from_secs(
        WAYLAND_SESSION_RESYNC_SECS + WAYLAND_SESSION_RESYNC_SECS / 2,
    );

    /// Wait until one row publishes `wanted`, and answer what it published.
    ///
    /// The budget is this test's own. `Pending`, `Recovering`, and
    /// `Reconciling` are what an actor publishes while its pass is still
    /// running, so a test that stopped there has observed nothing about the
    /// convergence it is about to assert.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn settled(
        plane: &ResourcePlaneV3,
        key: &ResourceKey,
        wanted: ResourceStatus,
        budget: Duration,
    ) -> ResourceStatus {
        let deadline = tokio::time::Instant::now() + budget;
        let mut last = ResourceStatus::Pending;
        loop {
            if let Some(view) = plane
                .client()
                .get(key.clone())
                .await
                .expect("the manager serves the row")
                && let Some(status) = view.observed_status()
            {
                last = status.clone();
                if status == wanted {
                    return status;
                }
            }
            if tokio::time::Instant::now() >= deadline {
                return last;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    /// Every frame this scene's Guest applied for one worker row, waiting
    /// until it applied at least one.
    ///
    /// The counterpart to [`frames_over`]: where that one holds a window open
    /// because the claim is that NO frame was written, this one ends the moment
    /// the Guest has applied one. A realization frame is the LAST effect in
    /// this graph - the worker row publishes its own readiness only once the
    /// Guest has applied the frame its launch wrote - so this is the closest
    /// observable evidence to the convergence a session row's aggregate gate
    /// then reports, and the one wait that does not sit on a tick chain.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn frames_until(
        realized: &GuestRealizations,
        process_ref: &str,
        budget: Duration,
    ) -> Vec<d2b_provider_process::worker_launch::GuestProcessRealization> {
        let deadline = tokio::time::Instant::now() + budget;
        loop {
            let frames = realized.frames_for(process_ref).await;
            if !frames.is_empty() || tokio::time::Instant::now() >= deadline {
                return frames;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    /// Every frame this scene's Guest applied for one worker row over a fixed
    /// window, in arrival order.
    ///
    /// This is the NEGATIVE observation a fence needs, and it cannot be made
    /// by a single read: one read cannot tell "no frame was ever written"
    /// from "the read landed before the write". The row's own status is no
    /// help either - a worker row publishes its own classification while its
    /// launch is still being prepared, so a reader that stopped at `Pending`
    /// would be reading a status that says nothing about what reached the
    /// Guest. Every frame this Guest has ever applied is recorded from the
    /// moment the scene bound it, so a window opened after the fact still
    /// answers for the passes that ran before it; what the window adds is the
    /// retries that run during it.
    ///
    /// The window is the test's own. A launch a source has not proven is
    /// deferred and retried on the Process family's own resync cadence, so a
    /// window spanning more than one of those cadences is what covers the
    /// retries a fence has to hold across.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn frames_over(
        realized: &GuestRealizations,
        process_ref: &str,
        window: Duration,
    ) -> Vec<d2b_provider_process::worker_launch::GuestProcessRealization> {
        let deadline = tokio::time::Instant::now() + window;
        loop {
            let frames = realized.frames_for(process_ref).await;
            if tokio::time::Instant::now() >= deadline {
                return frames;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    /// Every distinct status one row publishes, from the moment it publishes
    /// its first one and over a further window, in the order each was first
    /// observed.
    ///
    /// Unlike [`observed_statuses`] this reads PAST the first converged status.
    /// A restart leaves the previous boot's `Ready` on the durable row, so what
    /// has to be shown about a restarted actor is that it REPLACES that answer
    /// - and a reader that stopped at the stale first read would show nothing.
    ///
    /// The window opens on the row's FIRST publication rather than at the
    /// call. A restarted plane spawns its actors on open, and a spawn that has
    /// not been scheduled yet publishes nothing at all, so a window that began
    /// before this row had ever spoken could expire over a row that had never
    /// acted - and a reader that stopped there would report "it never left
    /// `Ready`" about a row that had not yet been given the chance. The
    /// window is what still has to follow that publication: the claim is that
    /// the answer STAYS replaced, which is only observable across a further
    /// pass.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn status_trail(
        plane: &ResourcePlaneV3,
        key: &ResourceKey,
        window: Duration,
    ) -> Vec<ResourceStatus> {
        // The call is bounded twice over: once for the window that opens when
        // the row first speaks, and once for the wait that precedes it, so a
        // row that never speaks ends the call instead of hanging the suite.
        let opened_at = tokio::time::Instant::now();
        let hard = opened_at + window + window;
        let mut opened: Option<tokio::time::Instant> = None;
        let mut seen: Vec<ResourceStatus> = Vec::new();
        loop {
            if let Some(view) = plane
                .client()
                .get(key.clone())
                .await
                .expect("the manager serves the row")
                && let Some(status) = view.observed_status()
            {
                if !seen.contains(&status) {
                    seen.push(status);
                }
                opened.get_or_insert_with(tokio::time::Instant::now);
            }
            let now = tokio::time::Instant::now();
            if now >= hard || opened.is_some_and(|at| now >= at + window) {
                return seen;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// Every row of one ResourceType this Zone still holds once the budget
    /// elapses; empty when the type drained inside it.
    ///
    /// The manager holds a row through its own teardown - the durable deleting
    /// mark stays observable until the cleanup completes - so a teardown test
    /// has to wait for RETIREMENT, not for the delete request.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn drained(
        plane: &ResourcePlaneV3,
        type_name: &str,
        budget: Duration,
    ) -> BTreeMap<String, ResourceView> {
        let deadline = tokio::time::Instant::now() + budget;
        loop {
            let held = rows_of_type(plane, type_name).await;
            if held.is_empty() || tokio::time::Instant::now() >= deadline {
                return held;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    /// The two worker rows of one scene's derivation, in the family's own
    /// preserved order: the host proxy, then the guest frontend.
    fn display_worker_rows(processes: &[ResourceKey]) -> (ResourceKey, ResourceKey) {
        let proxy = processes
            .iter()
            .find(|key| key.name.starts_with("display-host-proxy-"))
            .expect("the host proxy row");
        let frontend = processes
            .iter()
            .find(|key| key.name.starts_with("display-guest-frontend-"))
            .expect("the guest frontend row");
        (proxy.clone(), frontend.clone())
    }

    /// The published status projection of one committed row, read once its
    /// owning actor has published one.
    ///
    /// An owning actor republishes its status on every pass, and the window
    /// between a pass's own status and its own projection is one in which the
    /// row publishes nothing at all. A reader that sampled that window would
    /// report a row that said nothing, which is a moment in one actor's cadence
    /// rather than anything this graph did. The wait ends on the first
    /// projection the actor publishes; a projection that carries no layer
    /// under test is still that actor's own answer, and is returned as it is.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn published_projection(
        plane: &ResourcePlaneV3,
        key: &ResourceKey,
        budget: Duration,
    ) -> serde_json::Value {
        let deadline = tokio::time::Instant::now() + budget;
        loop {
            let projection = view_of(plane, key)
                .await
                .observed_status_projection()
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            if !projection.is_null() || tokio::time::Instant::now() >= deadline {
                return projection;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    /// The realization token one committed endpoint row published, or an
    /// empty string where it published none.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn endpoint_token(
        plane: &ResourcePlaneV3,
        key: &ResourceKey,
        budget: Duration,
    ) -> String {
        published_projection(plane, key, budget)
            .await
            .pointer("/endpoint/incarnation")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned()
    }

    /// The one canonical relationship row one committed endpoint row
    /// publishes, named by the Endpoint family's own derivation - the same
    /// derivation the `Endpoint` actor commits the row from.
    fn published_relationship(
        endpoint: &ResourceKey,
        spec: &d2b_provider_display_wayland::EndpointSpec,
    ) -> ResourceKey {
        let rows = d2b_provider_display_wayland::display_canonical_bindings(
            &ZoneId::parse("test").expect("zone"),
            &ResourceRef::parse(&format!("Endpoint/{}", endpoint.name)).expect("endpoint ref"),
            spec,
        )
        .expect("the Endpoint family derives this row's published relationships");
        assert_eq!(
            rows.len(),
            1,
            "a row that publishes a relationship publishes exactly one: {endpoint}"
        );
        ResourceKey::new("test", "EndpointBinding", rows[0].name().as_str())
    }

    /// The realization token one delivered relationship published, or an
    /// empty string where it published none.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn relationship_token(
        plane: &ResourcePlaneV3,
        key: &ResourceKey,
        budget: Duration,
    ) -> String {
        published_projection(plane, key, budget)
            .await
            .pointer("/binding")
            .and_then(|layer| layer.pointer("/incarnation"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
            .unwrap_or_default()
    }

    /// Every grant this scene's broker end was asked to admit, waiting until
    /// each named endpoint row has had its own grant cross the wire.
    ///
    /// A grant is the deepest evidence a relationship publishes: the
    /// `EndpointBinding` actor behind it has read the endpoint's own
    /// realization, asked the production dispatch for exactly that consumer's
    /// access, and been answered over the real socket. How long the LAST of
    /// those takes is a property of the graph and not of the test - each link
    /// is one `ENDPOINT_BINDING_RESYNC`, and each link only starts once the
    /// source above it has published - so a wait that ends on the evidence
    /// covers the whole chain however slowly this machine runs it, where a
    /// window covers whatever fraction of it the machine happened to finish.
    ///
    /// The wait names the endpoints rather than counting calls, and counts
    /// only what this end ANSWERED: a scene that holds one row still records
    /// that row's grants, and a reader that counted them would be satisfied by
    /// the refusal it is waiting for the absence of.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn granted_endpoints(
        broker: &EndpointAccessBroker,
        endpoints: &[ResourceKey],
        budget: Duration,
    ) -> Vec<BrokerEndpointCall> {
        let deadline = tokio::time::Instant::now() + budget;
        loop {
            let grants: Vec<BrokerEndpointCall> = broker
                .calls()
                .into_iter()
                .filter(|call| call.verb == EndpointAccessVerb::Grant && call.answered)
                .collect();
            let delivered = endpoints
                .iter()
                .all(|key| grants.iter().any(|call| call.endpoint == key.name));
            if delivered || tokio::time::Instant::now() >= deadline {
                return grants;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    /// The realization token one committed endpoint row published, waiting
    /// until it published a STANDING one rather than any publication at all.
    ///
    /// [`endpoint_token`] answers at the row's first projection, which for a
    /// `ProducerRow` shape is the unrealized class its own actor publishes
    /// before it can prove a realization. A reader that wants the standing
    /// answer has to keep reading until one is published.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn standing_endpoint_token(
        plane: &ResourcePlaneV3,
        key: &ResourceKey,
        budget: Duration,
    ) -> String {
        let deadline = tokio::time::Instant::now() + budget;
        loop {
            let token = published_projection(
                plane,
                key,
                deadline.saturating_duration_since(tokio::time::Instant::now()),
            )
            .await
            .pointer("/endpoint/incarnation")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned();
            if !token.is_empty() || tokio::time::Instant::now() >= deadline {
                return token;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    /// Every revoke this broker end ADMITTED for one endpoint row, in arrival
    /// order.
    fn revokes_for(
        broker: &EndpointAccessBroker,
        endpoint: &ResourceKey,
    ) -> Vec<BrokerEndpointCall> {
        broker
            .admissions(endpoint)
            .into_iter()
            .filter(|call| call.verb == EndpointAccessVerb::Revoke)
            .collect()
    }

    /// The scene's display graph, exactly as the display Provider's own
    /// durable derivation builds it: the three `Endpoint` rows in the
    /// family's preserved order (the host compositor source, the host proxy's
    /// private carriage, the guest frontend's own endpoint), and the two
    /// worker `Process` rows.
    fn derived_display_rows(
        session_uid: &ResourceUid,
    ) -> (
        Vec<(ResourceKey, d2b_provider_display_wayland::EndpointSpec)>,
        Vec<ResourceKey>,
    ) {
        let session_ref = ResourceRef::parse(&format!(
            "{WAYLAND_SESSION_TYPE}/{DISPLAY_SESSION_NAME}"
        ))
        .expect("the session's own canonical reference");
        let spec = display_session_spec();
        let endpoints = display_endpoint_child_rows(&session_ref, session_uid, &spec);
        let processes = d2b_provider_display_wayland::session_children::display_owned_child_intents(
            &ZoneId::parse("test").expect("zone"),
            &session_ref,
            session_uid,
            &spec,
            1,
        )
        .expect("the durable child derivation")
        .into_iter()
        .filter(|intent| intent.target().resource_type().as_str() == "Process")
        .map(|intent| ResourceKey::new("test", "Process", intent.target().name().as_str()))
        .collect();
        (endpoints, processes)
    }

    /// The production-composition acceptance for the display actor graph: one
    /// manager-owned `WaylandSession` row, admitted through the plane's own
    /// Nix ingest and reconciled by the real manager, the real ProviderSet,
    /// the real driver factories, the real interaction effects service over
    /// this plane's own manager-plane reads, and the real Endpoint-family
    /// committed-shape seam.
    ///
    /// The graph it realizes is the display Provider's own durable derivation
    /// and nothing else: two worker `Process` rows, three `Endpoint` rows, and
    /// exactly two `EndpointBinding` rows derived by the `Endpoint` driver
    /// from each committed endpoint row's own publication intent.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread")]
    async fn an_admitted_display_session_realizes_its_whole_actor_graph() {
        let scene = display_scene().await;
        let (endpoints, processes) = derived_display_rows(&scene.session_uid);
        // The evidence this graph has to produce before its session row can
        // answer: both of the relationships the display Provider's own
        // derivation publishes, granted over this scene's real broker socket.
        // A grant is the deepest thing a relationship publishes, so waiting
        // for both of them covers the whole chain - each `Endpoint` realizing,
        // each `EndpointBinding` delivering, each consumer `Process` launching
        // and the Guest applying its frame - however many of this machine's
        // resync ticks that takes, and only the session row's own aggregate
        // answer is left on a budget.
        granted_endpoints(
            &scene.broker,
            &[endpoints[0].0.clone(), endpoints[1].0.clone()],
            DISPLAY_EVIDENCE_BUDGET,
        )
        .await;
        let session = settled(
            &scene.plane,
            &scene.session_key,
            ResourceStatus::Ready,
            DISPLAY_AGGREGATE_BUDGET,
        )
        .await;
        assert_eq!(
            session,
            ResourceStatus::Ready,
            "the manager-owned session row is reconciled by the interaction family's real effects \
             over this plane's own manager reads"
        );

        // The ONE display vocabulary the plane installs in both seams admitted
        // every committed endpoint row. A fixture that handed the Endpoint
        // family its own always-empty registry would have refused all three
        // for `ShapeUnsupported`, which no d2bd test could see why.
        for (key, spec) in &endpoints {
            assert!(
                d2b_provider_endpoint::endpoint_realization(spec, &*scene.vocabulary).is_some(),
                "{key} is a shape the display Provider commits, admitted by the one vocabulary the \
                 session driver and the Endpoint family share"
            );
        }

        // The graph is exactly the derivation's, and every row in it is a
        // manager-owned child of the session row.
        let held_processes = rows_of_type(&scene.plane, "Process").await;
        let held_endpoints = rows_of_type(&scene.plane, "Endpoint").await;
        let held_bindings = rows_of_type(&scene.plane, "EndpointBinding").await;
        assert_eq!(held_processes.len(), 2, "one session derives two worker rows: {held_processes:?}");
        assert_eq!(held_endpoints.len(), 3, "one session derives three endpoint rows: {held_endpoints:?}");
        assert_eq!(held_bindings.len(), 2, "one session publishes two relationships: {held_bindings:?}");
        for key in processes.iter().chain(endpoints.iter().map(|(key, _)| key)) {
            let view = view_of(&scene.plane, key).await;
            assert_eq!(
                view.owner_key.as_ref(),
                Some(&scene.session_key),
                "{key} is a manager-owned child of the session row, not a display-local row"
            );
        }

        // The two relationships are the ones the committed endpoint rows'
        // OWN publication intent derives, through the Endpoint family's own
        // derivation - and the guest frontend's own endpoint publishes
        // nothing, so it derives none.
        let zone = ZoneId::parse("test").expect("zone");
        let mut derived = Vec::new();
        for (index, (key, spec)) in endpoints.iter().enumerate() {
            let endpoint_ref =
                ResourceRef::parse(&format!("Endpoint/{}", key.name)).expect("endpoint ref");
            let rows = d2b_provider_display_wayland::display_canonical_bindings(
                &zone,
                &endpoint_ref,
                spec,
            )
            .expect("the Endpoint family derives this row's published relationships");
            if index == endpoints.len() - 1 {
                assert!(
                    rows.is_empty(),
                    "the guest frontend's own endpoint publishes no in-Zone relationship, so no row \
                     is derived for it"
                );
            } else {
                assert_eq!(
                    rows.len(),
                    1,
                    "the host compositor source and the host proxy's private carriage each publish \
                     exactly one relationship: {key} derived {rows:?}"
                );
            }
            derived.extend(rows.into_iter().map(|row| row.name().as_str().to_owned()));
        }
        derived.sort();
        let mut held: Vec<String> = held_bindings.keys().cloned().collect();
        held.sort();
        assert_eq!(
            held, derived,
            "the committed binding rows are exactly what the endpoint rows' own publication intent \
             derives"
        );
        // The delivery those two rows report is the one this scene's broker
        // wire produced: the PRODUCTION `DaemonEndpointAccessDispatch`
        // answered both grants over the daemon's own broker socket, and both
        // relationships publish a delivery at their own current row
        // generation. Nothing here scripts a verdict - the verdict is read
        // back through the display Provider's own published-layer parser.
        for (name, view) in &held_bindings {
            let layer = view.observed_status_projection().and_then(|p| p.pointer("/binding").cloned());
            assert!(
                d2b_provider_display_wayland::session_children::display_binding_delivered(
                    layer.as_ref(),
                    view.generation,
                ),
                "{name} publishes no delivery at generation {}: {layer:?}",
                view.generation
            );
        }
        let calls = scene.broker.calls();
        let granted: Vec<&BrokerEndpointCall> = calls
            .iter()
            .filter(|call| call.verb == EndpointAccessVerb::Grant)
            .collect();
        assert_eq!(
            granted.len(),
            2,
            "one grant per published relationship crossed the real broker socket: {granted:?}"
        );

        // With both deliveries standing the two worker rows leave the launch
        // gate and publish readiness, which is what the session's aggregate
        // gate reads.
        for key in &processes {
            let status = settled(
                &scene.plane,
                key,
                ResourceStatus::Ready,
                DISPLAY_AGGREGATE_BUDGET,
            )
            .await;
            assert_eq!(
                status,
                ResourceStatus::Ready,
                "{key} is released by its own delivered relationship, not by the session"
            );
        }
        // The claim is WHICH rows take the host launch, not how many times
        // the scripted provider is asked before it records the identity its
        // own launch produced. Those are different facts, and this scene's
        // double scripts the second one: it reports no retained identity
        // until its launch is serving, and the launch gate now answers
        // before a row acts at all, so the pass that finds its sources
        // unproven never reaches the provider to be asked. Counting launches
        // therefore counted this double's pacing rather than this graph's
        // shape. Which row launched is stated the way the two sibling
        // acceptance tests over this same scene state it.
        let (host_proxy, _frontend) = display_worker_rows(&processes);
        let launches = scene.processes.launch_calls();
        assert!(
            !launches.is_empty()
                && launches
                    .iter()
                    .all(|launch| launch.resource_ref.ends_with(&host_proxy.name)),
            "the host proxy is the one worker the Process family launches over the host Provider; \
             the guest frontend realizes through its own Guest target: {launches:?}"
        );
        scene.plane.shutdown().await;
    }

    /// The authenticated client the daemon's own Resource API session holds:
    /// the real service over the real manager-backed store, bound to a subject
    /// the real authorizer admitted.
    type DisplayApiClient = d2b_resource_api::ResourceApiClient<
        d2b_resource_api::manager_backend::ManagerBackend,
        d2b_resource_api::service::UnavailableUpgradeDispatcher,
    >;

    /// The controller generation this Zone's daemon session is bound to, and
    /// the one its authorization state is admitted under.
    ///
    /// A status mutation is routed to the store only for a session whose
    /// controller generation the evaluator still reads as current, so the two
    /// are declared as one value here rather than two that could drift.
    const DISPLAY_API_CONTROLLER_GENERATION: u64 = 11;

    /// The policy revision [`display_api_authorizer`] installs, and the
    /// revision [`display_api_authorization_state`] is admitted under; the
    /// evaluator refuses a session whose snapshot names another one.
    const DISPLAY_API_POLICY_REVISION: u64 = 7;

    /// The identity the daemon's own system-core Resource API session carries
    /// into a display Zone: a locally bound `Provider` session, admitted by a
    /// policy that names exactly this Zone.
    ///
    /// This is the identity the daemon's own update path is authenticated
    /// with, not a stand-in for it. What the path then reaches is this scene's
    /// own plane.
    fn display_api_subject(
        zone: &ZoneId,
    ) -> d2b_contracts_resource::v3::identity::AuthenticatedSubjectContext {
        use d2b_contracts_resource::v3::identity::{
            BindingDigest, EvidenceClass, Locality, ReconnectGeneration, ServiceName,
            SessionBinding, SessionPurpose, TranscriptHash,
        };

        d2b_contracts_resource::v3::identity::AuthenticatedSubjectContext::new(
            ResourceRef::parse("Provider/system-core").expect("the system-core Provider reference"),
            ResourceUid::parse("123e4567-e89b-42d3-a456-426614174001")
                .expect("the subject's own committed uid"),
            // The Zone's own Display is redacted; the ref carries its canonical
            // name, which is what the authorizer matches the request Zone by.
            ResourceRef::parse(&format!("Zone/{}", zone.as_str()))
                .expect("the Zone self reference"),
            EvidenceClass::UnixPeer,
            SessionPurpose::parse("resource-api").expect("a bounded session purpose"),
            ServiceName::parse("d2b.resource.v3").expect("a bounded service name"),
            SessionBinding::new(
                d2b_contracts_resource::v3::SchemaFingerprint::parse(format!(
                    "sha256:{}",
                    "1".repeat(64)
                ))
                .expect("a bounded schema fingerprint"),
                d2b_contracts_resource::v3::identity::TransportBinding::new(
                    Locality::Local,
                    BindingDigest::parse(format!("sha256:{}", "2".repeat(64)))
                        .expect("a bounded binding digest"),
                ),
                ReconnectGeneration::new(1).expect("a bounded reconnect generation"),
                TranscriptHash::from_bytes([3; 32]),
            ),
        )
        .with_controller_generation(
            ControllerGeneration::new(DISPLAY_API_CONTROLLER_GENERATION)
                .expect("a bounded controller generation"),
        )
    }

    /// The authorization state this session's policy revision is admitted
    /// under; its revision is the one [`display_api_authorizer`] installs.
    fn display_api_authorization_state() -> d2b_resource_api::authz::AuthorizationState {
        d2b_resource_api::authz::AuthorizationState {
            snapshot: d2b_contracts_resource::v3::PolicySnapshot {
                policy_revision: DISPLAY_API_POLICY_REVISION,
                api_catalog_revision: 1,
                active_configuration_revision:
                    d2b_contracts_resource::v3::ConfigurationGeneration::new(7)
                        .expect("a bounded configuration generation"),
                controller_generation: Some(
                    ControllerGeneration::new(DISPLAY_API_CONTROLLER_GENERATION)
                        .expect("a bounded controller generation"),
                ),
            },
            zone_policy_revision: ZoneRevision::new(DISPLAY_API_POLICY_REVISION),
            bootstrap_phase: d2b_resource_api::authz::BootstrapPhase::Disabled,
            now_tick: 1,
        }
    }

    /// The real authorizer this Zone's session is admitted by, and the store
    /// seal its manager backend answers to.
    ///
    /// The rule grants every resource verb the graph's own types carry,
    /// `UpdateStatus` among them, so the refusal the test asserts is the one
    /// the mutation reaches AFTER authorization rather than a denial standing
    /// in for it.
    fn display_api_authorizer(
        zone: &ZoneId,
    ) -> (
        Arc<d2b_resource_api::authz::NativeAuthorizer>,
        d2b_contracts_resource::v3::operations::seal::MutationSealAcceptor,
    ) {
        use d2b_resource_api::authz::{
            ApiCatalog, BindingScope, BoundSubject, CompiledRole, CompiledRoleBinding,
            NativeAuthorizer, PolicyRule, PolicySet, RelayGrantAuthority, ResourceVerb, SessionVerb,
        };

        let catalog = ApiCatalog::with_extensions([
            d2b_contracts_resource::v3::ResourceTypeName::parse(WAYLAND_SESSION_TYPE)
                .expect("the display session type"),
        ])
        .expect("the display catalog extends the standard one");
        let subject = display_api_subject(zone);
        let rule = PolicyRule::new(
            &catalog,
            [
                d2b_contracts_resource::v3::ResourceTypeName::parse(WAYLAND_SESSION_TYPE)
                    .expect("the display session type"),
                d2b_contracts_resource::v3::ResourceTypeName::parse("Process")
                    .expect("the worker type"),
                d2b_contracts_resource::v3::ResourceTypeName::parse("Endpoint")
                    .expect("the endpoint type"),
                d2b_contracts_resource::v3::ResourceTypeName::parse("EndpointBinding")
                    .expect("the relationship type"),
            ],
            [
                ResourceVerb::Get,
                ResourceVerb::Create,
                ResourceVerb::UpdateSpec,
                ResourceVerb::UpdateStatus,
                ResourceVerb::UpdateMetadata,
                ResourceVerb::UpdateFinalizers,
                ResourceVerb::Delete,
            ],
            [SessionVerb::Connect],
            [],
            [],
            [zone.clone()],
            [],
        )
        .expect("the display status rule compiles");
        let role =
            CompiledRole::new(ResourceRef::parse("Role/system-core").expect("role ref"), vec![rule])
                .expect("the system-core role compiles");
        let binding = CompiledRoleBinding::new(
            role.role_ref.clone(),
            [BoundSubject {
                subject_ref: subject.subject_ref().clone(),
                subject_uid: subject.subject_uid().clone(),
            }],
            BindingScope::default(),
            RelayGrantAuthority::None,
        )
        .expect("the system-core role binding compiles");
        let policy =
            PolicySet::new(&catalog, DISPLAY_API_POLICY_REVISION, vec![role], vec![binding])
                .expect("the system-core policy set compiles");
        let authorizer =
            Arc::new(NativeAuthorizer::new(catalog, Some(policy)).expect("authorizer binds"));
        let seal = d2b_contracts_resource::v3::StoreSealIdentity::new(
            d2b_contracts_resource::v3::StoreSlot::new(0).expect("the manager plane's slot"),
            zone.clone(),
            ResourceUid::parse("11111111-1111-4111-8111-111111111111")
                .expect("the sealed store's own uid"),
        );
        let acceptor = authorizer
            .take_store_seal(seal)
            .expect("the manager plane takes the store seal");
        (authorizer, acceptor)
    }

    /// The operation metadata the Resource API requires of every request.
    fn display_api_request_meta(operation: &str) -> wire::RequestMeta {
        let mut meta = wire::RequestMeta::new();
        meta.operation_id = operation.to_owned();
        meta.idempotency_key = operation.to_owned();
        meta.correlation_id = operation.to_owned();
        meta.trace_id = operation.to_owned();
        meta.deadline_ms = 10_000;
        meta
    }

    /// The authenticated read this Zone's client issues for one row.
    fn display_api_get_request(
        zone: &ZoneId,
        key: &ResourceKey,
        operation: &str,
    ) -> wire::GetRequest {
        let mut request = wire::GetRequest::new();
        request.meta = protobuf::MessageField::some(display_api_request_meta(operation));
        let mut target = wire::ResourceIdentity::new();
        target.zone = zone.as_str().to_owned();
        target.resource_type = key.type_name.clone();
        target.name = key.name.clone();
        request.target = protobuf::MessageField::some(target);
        // The service refuses a read that does not name its projection; this
        // one wants the whole envelope, published status layer included.
        let mut projection = wire::Projection::new();
        projection.kind =
            protobuf::EnumOrUnknown::new(wire::ProjectionKind::PROJECTION_KIND_FULL);
        request.projection = protobuf::MessageField::some(projection);
        request
    }

    /// The canonical envelope this Zone's Resource API serves for one row,
    /// read through the same authenticated client the daemon holds.
    async fn display_api_served(
        client: &DisplayApiClient,
        zone: &ZoneId,
        key: &ResourceKey,
        operation: &str,
    ) -> wire::ResourceEnvelopeBytes {
        let response = client.get(display_api_get_request(zone, key, operation)).await;
        assert!(
            response.error.is_none(),
            "the Resource API serves {key}: {:?}",
            response.error.as_ref().map(|error| (
                error.kind.enum_value_or_default(),
                error.reason.clone()
            ))
        );
        *response
            .resource
            .0
            .expect("a served envelope for the read")
    }

    /// The daemon's own status mutation for one row: the canonical envelope
    /// the plane serves for it with its published phase replaced, carried
    /// under the exact revision and uid that same read reported.
    ///
    /// Nothing here is malformed on purpose. A status write the API cannot
    /// even parse would prove nothing about the refusal under test, so this
    /// is a well-formed `UpdateStatus` over a valid canonical envelope, and
    /// the only thing that can answer it is `ManagerBackend`.
    fn display_api_status_write_request(
        served: &wire::ResourceEnvelopeBytes,
        phase: &str,
        operation: &str,
    ) -> wire::UpdateStatusRequest {
        let mut envelope: serde_json::Value =
            serde_json::from_slice(&served.canonical_json).expect("a canonical envelope");
        envelope["status"]["phase"] = serde_json::Value::String(phase.to_owned());
        let payload = d2b_contracts_resource::v3::CanonicalJsonValue::parse(
            &serde_json::to_vec(&envelope).expect("the canonical status document"),
        )
        .expect("the status document parses")
        .to_canonical_bytes();
        let identity = served
            .identity
            .as_ref()
            .expect("a served identity for the read")
            .clone();
        let mut body = wire::ResourceEnvelopeBytes::new();
        body.identity = protobuf::MessageField::some(identity.clone());
        body.canonical_json = payload.clone();
        body.payload_digest = d2b_contracts_resource::v3::canonical_digest(
            d2b_contracts_resource::v3::RESOURCE_ENVELOPE_DOMAIN_TAG,
            &payload,
        );
        let mut precondition = wire::Precondition::new();
        precondition.kind = protobuf::EnumOrUnknown::new(
            wire::PreconditionKind::PRECONDITION_KIND_EXACT_REVISION,
        );
        precondition.expected_revision = served.identity.revision;
        precondition.expected_uid = served.identity.uid.clone();
        let mut mutation = wire::Mutation::new();
        mutation.kind = protobuf::EnumOrUnknown::new(wire::MutationKind::MUTATION_KIND_UPDATE_STATUS);
        mutation.target = protobuf::MessageField::some(identity);
        mutation.precondition = protobuf::MessageField::some(precondition);
        mutation.resource = protobuf::MessageField::some(body);
        let mut request = wire::UpdateStatusRequest::new();
        request.meta = protobuf::MessageField::some(display_api_request_meta(operation));
        request.mutation = protobuf::MessageField::some(mutation);
        request
    }

    /// The daemon's own authenticated Resource API cannot durably write a
    /// status, and the whole converged display graph is where that shows.
    ///
    /// The client is the daemon's own, assembled the way the composition
    /// assembles it: `ResourceBusAdapter::bind_component_session`
    /// over a `ResourceService` whose store IS the `ManagerBackend` this
    /// scene's own plane handed over, authorized by a real `NativeAuthorizer`
    /// over a real compiled policy that grants `UpdateStatus` on every type in
    /// the graph. So the mutation is admitted all the way to the production
    /// refusal in `ManagerBackend::commit_mutation`: it is not a double's
    /// answer, and it is not unconstructible either - it is a well-formed
    /// status mutation over a valid canonical envelope.
    ///
    /// The closed code is the load-bearing part of that answer, not the class.
    /// The service refuses a status mutation whose session carries no current
    /// controller generation under `status controller generation does not
    /// match` - the same class, a different code, and a refusal that happens
    /// before the store is ever asked. Only `resource-status-owner-mismatch`
    /// is `ManagerBackend` answering a mutation that was authorized, parsed
    /// and sealed on its way in.
    ///
    /// What it cannot be is a write. Every row of the graph - the session and
    /// all seven children its own durable derivation commits - still carries
    /// exactly the status its owning `ResourceActor` published, at exactly the
    /// revision it held before the attempt.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_daemons_own_api_cannot_durably_write_a_status() {
        let scene = display_scene().await;
        let zone = ZoneId::parse("test").expect("bounded zone");
        let (endpoints, processes) = derived_display_rows(&scene.session_uid);
        // The evidence the graph has to produce first: both of the
        // relationships the display Provider's own derivation publishes,
        // granted over this scene's real broker socket. The session row's
        // readiness is what the whole graph converges behind, and it is what
        // the relationships are derived behind, so both waits are on the
        // graph's own output rather than on a clock.
        granted_endpoints(
            &scene.broker,
            &[endpoints[0].0.clone(), endpoints[1].0.clone()],
            DISPLAY_EVIDENCE_BUDGET,
        )
        .await;
        assert_eq!(
            settled(
                &scene.plane,
                &scene.session_key,
                ResourceStatus::Ready,
                DISPLAY_AGGREGATE_BUDGET
            )
            .await,
            ResourceStatus::Ready,
            "the session row is driven to readiness by its own owning actor, before this test \
             writes anything"
        );
        // The whole graph, in the display Provider's own derivation: the
        // session row, its two worker children, its three endpoint children,
        // and the two relationships the endpoint rows publish.
        let mut graph = vec![scene.session_key.clone()];
        graph.extend(processes.iter().cloned());
        graph.extend(endpoints.iter().map(|(key, _)| key.clone()));
        graph.extend(
            rows_of_type(&scene.plane, "EndpointBinding")
                .await
                .keys()
                .map(|name| ResourceKey::new("test", "EndpointBinding", name.as_str())),
        );
        assert_eq!(
            graph.len(),
            8,
            "the session row and the seven children its durable derivation commits: {graph:?}"
        );
        // Every child row is standing before anything is attempted: a status
        // write refused against a row that is still converging would say
        // nothing about the status its owning actor owns.
        for key in &graph {
            assert_eq!(
                settled(
                    &scene.plane,
                    key,
                    ResourceStatus::Ready,
                    DISPLAY_AGGREGATE_BUDGET
                )
                .await,
                ResourceStatus::Ready,
                "{key} is driven to readiness by its own owning actor, before this test writes \
                 anything"
            );
        }

        let (authorizer, acceptor) = display_api_authorizer(&zone);
        let backend = d2b_resource_api::manager_backend::ManagerBackend::new(
            scene.plane.client().clone(),
            scene.plane.hub(),
            acceptor,
        );
        let service = Arc::new(
            d2b_resource_api::ResourceService::new_with_zone_uid(
                Arc::new(backend),
                Arc::clone(&authorizer),
                None,
            )
            .expect("the display Resource service binds"),
        );
        let capability = authorizer
            .issue_authenticated_subject(
                display_api_subject(&zone),
                display_api_authorization_state(),
            )
            .expect("the policy grants the system-core session its display status verb");
        let client = d2b_resource_api::ResourceBusAdapter::bind_component_session(
            service,
            capability,
        )
        .expect("the session binds to the Resource service")
        .client();

        for (index, key) in graph.iter().enumerate() {
            let before = display_api_served(
                &client,
                &zone,
                key,
                &format!("display-status-read-{index}"),
            )
            .await;
            let envelope: serde_json::Value =
                serde_json::from_slice(&before.canonical_json).expect("a canonical envelope");
            let status_before = envelope["status"].clone();
            let published = status_before["phase"]
                .as_str()
                .expect("the served envelope publishes a phase")
                .to_owned();
            // A status the daemon would have to accept as a change, so an
            // unchanged row after the attempt is the actor's answer and not a
            // value the write happened to agree with.
            let attempted = if published == "Failed" { "Degraded" } else { "Failed" };
            assert_ne!(
                attempted, published,
                "{key} is attempted with a phase its own actor never published"
            );
            let response = client
                .update_status(display_api_status_write_request(
                    &before,
                    attempted,
                    &format!("display-status-write-{index}"),
                ))
                .await;
            let error = response.error.as_ref().unwrap_or_else(|| {
                panic!("{key} published {published} and admitted a daemon status write")
            });
            assert_eq!(
                error.kind.enum_value_or_default(),
                wire::ResourceErrorKind::RESOURCE_ERROR_KIND_RESOURCE_STATUS_OWNER_MISMATCH,
                "{key} is refused because the daemon's API is not the status owner: {:?} {}",
                error.kind.enum_value_or_default(),
                error.reason,
            );
            assert_eq!(
                error.reason, "resource-status-owner-mismatch",
                "the refusal is the manager backend's own closed code, over the real policy that \
                 grants the verb: {key} answered {:?} {}",
                error.kind.enum_value_or_default(),
                error.reason,
            );
            // The row the attempt named still carries the actor's own status,
            // at the actor's own revision.
            let after = display_api_served(
                &client,
                &zone,
                key,
                &format!("display-status-reread-{index}"),
            )
            .await;
            let envelope: serde_json::Value =
                serde_json::from_slice(&after.canonical_json).expect("a canonical envelope");
            assert_eq!(
                envelope["status"], status_before,
                "{key} kept the status its owning actor published across the refused write"
            );
            assert_eq!(
                after.identity.revision, before.identity.revision,
                "{key} carries the revision it held before the refused write"
            );
            // The row is still driven to readiness by its owning actor. That is
            // read over a window rather than at one instant: every actor in
            // this graph republishes on its own resync cadence, so a single
            // sample can land inside a pass that has not published its own
            // classification yet, and would report a row mid-pass as one the
            // daemon had taken over.
            assert_eq!(
                settled(
                    &scene.plane,
                    key,
                    ResourceStatus::Ready,
                    WAYLAND_SESSION_RESYNC * 2
                )
                .await,
                ResourceStatus::Ready,
                "{key} is still driven by its owning actor, read back through the manager the \
                 refused write would have had to reach"
            );
        }
        scene.plane.shutdown().await;
    }

    /// The host proxy reaches readiness BEFORE the guest frontend, and the
    /// order is the graph's evidence rather than an order this graph imposed.
    ///
    /// The host proxy's private carriage is a `ProducerRow` shape realized
    /// behind the host proxy's own `Process` row, so the relationship the
    /// guest frontend launches against cannot be delivered - and the frontend
    /// therefore cannot be released - while that row is not standing. This
    /// test withholds exactly that one relationship at the broker, watches the
    /// host proxy converge with the frontend still blocked, then lets the
    /// broker answer and watches the frontend follow. The Process family's
    /// own recorded effects say what happened in between: no launch effect
    /// was issued for the frontend while its delivery did not exist.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread")]
    async fn the_host_proxy_reaches_readiness_before_the_guest_frontend() {
        let scene = display_scene().await;
        let (endpoints, processes) = derived_display_rows(&scene.session_uid);
        // The scene's own derivation order: the host compositor source, the
        // host proxy's private carriage, the guest frontend's own endpoint
        // (which publishes nothing). The carriage is the second of the three.
        let (compositor, carriage) = (endpoints[0].0.clone(), endpoints[1].0.clone());
        let (host_proxy, frontend) = {
            let proxy = processes
                .iter()
                .find(|key| key.name.starts_with("display-host-proxy-"))
                .expect("the host proxy row");
            let frontend = processes
                .iter()
                .find(|key| key.name.starts_with("display-guest-frontend-"))
                .expect("the guest frontend row");
            (proxy.clone(), frontend.clone())
        };

        // The carriage's relationship is refused at the broker for as long as
        // this test holds it. Everything else is answered normally.
        scene.broker.hold(&carriage);
        // The evidence the host proxy needs before it can be released: the
        // compositor relationship, the one this scene still answers, granted
        // over the real broker socket. The carriage is named nowhere in this
        // wait, because a broker end that refuses a row still records its
        // grant - counting calls would be satisfied by the refusal this test
        // is holding.
        granted_endpoints(
            &scene.broker,
            std::slice::from_ref(&compositor),
            DISPLAY_EVIDENCE_BUDGET,
        )
        .await;
        let proxy = settled(
            &scene.plane,
            &host_proxy,
            ResourceStatus::Ready,
            DISPLAY_AGGREGATE_BUDGET,
        )
        .await;
        assert_eq!(
            proxy,
            ResourceStatus::Ready,
            "the host proxy is released by the compositor relationship, which this scene answers"
        );
        let held = settled(
            &scene.plane,
            &frontend,
            ResourceStatus::Pending,
            DISPLAY_LINK_RESYNC,
        )
        .await;
        assert_ne!(
            held,
            ResourceStatus::Ready,
            "the guest frontend is NOT released while the relationship its launch gate reads has \
             no delivery: the ordering is its evidence, not an order this graph issued"
        );
        assert!(
            scene
                .processes
                .launch_calls()
                .iter()
                .all(|launch| launch.resource_ref.ends_with(&host_proxy.name)),
            "no launch effect was issued for a consumer whose own delivery does not exist: {:?}",
            scene.processes.launch_calls()
        );

        scene.broker.release(&carriage);
        // The evidence the frontend needs before it can answer: the carriage
        // relationship granted over the real broker socket now that this scene
        // answers it, and the Guest having applied the frontend's realization.
        // A frame is the last effect in this graph, so neither wait can expire
        // in the middle of a chain that was still making progress.
        granted_endpoints(
            &scene.broker,
            &[compositor.clone(), carriage.clone()],
            DISPLAY_EVIDENCE_BUDGET,
        )
        .await;
        let _ = frames_until(
            &scene.realized,
            &ResourceRef::parse(&format!("Process/{}", frontend.name))
                .expect("the frontend's own canonical reference")
                .to_canonical_string(),
            DISPLAY_EVIDENCE_BUDGET,
        )
        .await;
        let released = settled(
            &scene.plane,
            &frontend,
            ResourceStatus::Ready,
            DISPLAY_AGGREGATE_BUDGET,
        )
        .await;
        assert_eq!(
            released,
            ResourceStatus::Ready,
            "the guest frontend follows once its own relationship is delivered"
        );
        let Some(carriage_grant) = scene.broker.last_call(&carriage) else {
            panic!("the carriage relationship reached the broker socket");
        };
        let Some(compositor_grant) = scene.broker.last_call(&compositor) else {
            panic!("the compositor relationship reached the broker socket");
        };
        assert!(
            compositor_grant.at <= carriage_grant.at,
            "the host proxy's own delivery crossed the wire first: compositor {compositor_grant:?} \
             carriage {carriage_grant:?}"
        );
        scene.plane.shutdown().await;
    }

    /// A source that cannot prove a realization hands out no access through
    /// it, and nothing downstream reads `Ready` (R18, R21).
    ///
    /// The carriage is a `ProducerRow` shape: its realization IS the host
    /// proxy's `Process` row, so while that row is not standing the carriage
    /// publishes the unrealized class and NO token - and the guest frontend,
    /// whose admission is gated on exactly that token, can be handed nothing
    /// through it. The Process provider is therefore given a runtime that
    /// admits nothing for the host proxy, which is the state its own driver
    /// reaches whenever a launch does not come back serving.
    ///
    /// What is asserted is the fence and nothing weaker: the frontend's row
    /// does not read `Ready`, NO realization the Guest applied names it at
    /// all, and the session is not `Ready` either. The startup pass itself is
    /// blocked too, and that is the point rather than an accident: while the
    /// carriage names no realization there is nothing the frontend's gate can
    /// be satisfied over, so a launch in that state could only ever carry an
    /// EMPTY delivery set - and a process with no endpoint access still runs,
    /// so that was never a fence. The frame itself is what must not be
    /// written.
    ///
    /// The observation is over the FRAMES, across a window spanning more than
    /// the Process family's own resync cadence so the assertion covers the
    /// retries as well as the first pass. It cannot be over the row's status:
    /// a worker row publishes its own classification while its launch is
    /// still being prepared, so whether the pass has run at any given instant
    /// is decided by where that read lands against the source's own
    /// republication cadence.
    ///
    /// Nothing here scripts a verdict. The Endpoint and Process drivers derive
    /// every class, the relationship is delivered by the PRODUCTION dispatch
    /// over the real broker socket, and the frames are read back out of the
    /// bytes the production Process family wrote to the Guest.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread")]
    async fn an_unproven_endpoint_realization_blocks_the_guest_frontend() {
        // The Process provider admits nothing while this test says so, so the
        // host proxy row is never standing. Everything else is the same real
        // scene: the same manager, the same provider set, the same driver
        // factories, the same REAL broker socket, and the same bound Guest
        // target.
        let silent = Arc::new(d2b_provider_process::test_support::FakeFacets::new(
            Default::default(),
        ));
        silent.set_active(false);
        let scene = display_scene_with_processes(Arc::clone(&silent)).await;
        let (endpoints, processes) = derived_display_rows(&scene.session_uid);
        let carriage = endpoints[1].0.clone();
        let (host_proxy, frontend) = display_worker_rows(&processes);
        let frontend_ref = ResourceRef::parse(&format!("Process/{}", frontend.name))
            .expect("the frontend's own canonical reference");
        let blocked = settled(
            &scene.plane,
            &host_proxy,
            ResourceStatus::Pending,
            Duration::from_secs(60),
        )
        .await;
        assert_ne!(
            blocked,
            ResourceStatus::Ready,
            "the host proxy row is not standing while its provider admits nothing for it"
        );
        assert!(
            scene
                .processes
                .launch_calls()
                .iter()
                .all(|launch| launch.resource_ref.ends_with(&host_proxy.name)),
            "only the host proxy row ever reached its own launch effect: {:?}",
            scene.processes.launch_calls()
        );

        // The carriage is a `ProducerRow` shape realized behind THAT row, so
        // with no standing producer it publishes the unrealized class and NO
        // token at all.
        // A positive wait: it ends when the endpoint actor publishes, so the
        // bound only decides how long a graph that never published takes to
        // be reported. The publication itself is one `Endpoint` actor's first
        // pass over a row the session actor committed, which is one chain away.
        let layer = published_projection(&scene.plane, &carriage, DISPLAY_EVIDENCE_BUDGET).await;
        assert_eq!(
            layer
                .pointer("/endpoint/readiness")
                .and_then(serde_json::Value::as_str),
            Some("realizing"),
            "a ProducerRow shape with no standing producer row is not realized: {layer}"
        );
        assert!(
            layer.pointer("/endpoint/incarnation").is_none(),
            "a realization this row cannot prove publishes NO token, so no launch can be gated on \
             one: {layer}"
        );

        let frontend_status = settled(
            &scene.plane,
            &frontend,
            ResourceStatus::Pending,
            Duration::from_secs(30),
        )
        .await;
        assert_ne!(
            frontend_status,
            ResourceStatus::Ready,
            "the guest frontend is not released while its source names no realization"
        );
        // The frontend's own startup pass is BLOCKED, and what proves it is
        // the absence of a frame rather than the row's status: a worker row
        // publishes its own classification while its launch is still being
        // prepared, so `Pending` there is an answer that says nothing about
        // what reached the Guest. An empty delivery set was the old fence - a
        // realized row carrying no access - and it was not a fence at all,
        // because a process with no endpoint access still runs. The frame
        // itself is the thing that must never be written.
        //
        // Every frame this Guest has applied is recorded from the moment the
        // scene bound it, so this window answers for the pass that raced the
        // carriage's own first publication - the one a source that had
        // published nothing at all used to let through - as well as for the
        // retries that run during it.
        let before = frames_over(
            &scene.realized,
            &frontend_ref.to_canonical_string(),
            DISPLAY_RETRY_WINDOW,
        )
        .await;
        assert!(
            before.is_empty(),
            "no realization reached the guest frontend while its source names no realization: \
             {before:?}"
        );

        let session = settled(
            &scene.plane,
            &scene.session_key,
            ResourceStatus::Pending,
            Duration::from_secs(30),
        )
        .await;
        assert_ne!(
            session,
            ResourceStatus::Ready,
            "a session whose frontend has no admitted carriage is not a ready display session"
        );

        // Admit the worker the host proxy launched, and the graph converges:
        // the frame the frontend is realized with carries EXACTLY the token
        // the carriage published and EXACTLY the one its delivered
        // relationship was granted over. That equality is the fence - a launch
        // over any other token would be a launch over a realization this graph
        // cannot prove.
        silent.push_adoption(d2b_provider_process::ProviderAdoption::Adopted(
            adopted_report(),
        ));
        // What the convergence has to produce is the evidence first and the
        // session's own answer second: both relationships granted over this
        // scene's real broker socket, and the Guest this plane is bound to
        // having applied the frontend's realization. Neither is a status any
        // row publishes, so waiting on them is waiting on the graph rather
        // than on a clock, and only the aggregate read that follows them is
        // left on a budget.
        granted_endpoints(
            &scene.broker,
            &[endpoints[0].0.clone(), carriage.clone()],
            DISPLAY_EVIDENCE_BUDGET,
        )
        .await;
        assert_eq!(
            settled(
                &scene.plane,
                &scene.session_key,
                ResourceStatus::Ready,
                DISPLAY_AGGREGATE_BUDGET,
            )
            .await,
            ResourceStatus::Ready,
            "the whole graph converges once the source can prove its realization"
        );
        let token =
            standing_endpoint_token(&scene.plane, &carriage, DISPLAY_AGGREGATE_BUDGET).await;
        assert!(
            !token.is_empty(),
            "the standing carriage publishes the token its dependents gate on"
        );
        let relationship = published_relationship(&carriage, &endpoints[1].1);
        assert_eq!(
            relationship_token(&scene.plane, &relationship, DISPLAY_AGGREGATE_BUDGET).await,
            token,
            "the delivered relationship was granted over exactly the realization the endpoint \
             published"
        );
        let frames = frames_until(
            &scene.realized,
            &frontend_ref.to_canonical_string(),
            DISPLAY_AGGREGATE_BUDGET,
        )
        .await;
        let frame = frames.last().expect(
            "the frontend realize frame the Process family wrote",
        );
        assert_eq!(
            frame.deliveries().len(),
            1,
            "the frontend is realized with exactly the one relationship it requires: {frame:?}"
        );
        assert_eq!(
            frame.deliveries()[0].incarnation(),
            token,
            "the launch was gated on the realization this endpoint row currently proves"
        );
        scene.plane.shutdown().await;
    }

    /// A restart RE-OBSERVES: the session comes back to `Ready` from evidence
    /// a restarted provider produced, never from the status the previous boot
    /// left on the durable row.
    ///
    /// The durable store still carries the first boot's `Ready`, so the second
    /// half of this test is the decisive one: the restarted actors must REPLACE
    /// it. The restarted Process provider is given a runtime that admits
    /// nothing - the same double, scripted differently - and the session has to
    /// leave `Ready` and stay off it. Only when that provider admits the
    /// worker it launched does the graph return, and it returns over grants
    /// that crossed the RESTARTED daemon's own broker socket.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread")]
    async fn a_restart_re_observation_returns_the_session_to_ready_from_fresh_evidence() {
        let DisplayComposition {
            dir: _first_dir,
            inputs,
            client,
            broker: first_broker,
            processes: _first_processes,
        } = display_composition().await;
        let store_dir = inputs.spec_store_dir.clone();
        let plane = Arc::new(
            ResourcePlaneV3::open(inputs)
                .await
                .expect("the first boot opens"),
        );
        client
            .set(plane.client().clone())
            .expect("the first plane's client is bound once");
        bind_display_guest_target(&plane).await;
        plane
            .ingest_nix_bundle(&display_scene_bundle(true, 0))
            .await
            .expect("the scene's rows commit");
        let (session_key, session_uid) = display_session_row("test", DISPLAY_SESSION_NAME);
        let (endpoints, processes) = derived_display_rows(&session_uid);
        let (compositor, carriage) = (endpoints[0].0.clone(), endpoints[1].0.clone());
        let frontend = display_worker_rows(&processes).1;
        // The evidence the first boot has to produce before its session row
        // can answer: both of the relationships the display Provider's own
        // derivation publishes, granted over this scene's real broker socket.
        // A grant is the deepest thing this graph publishes about a
        // relationship, so waiting for both of them covers the whole chain -
        // each `Endpoint` realizing, each `EndpointBinding` delivering, each
        // consumer `Process` launching - however many of this machine's
        // five-second ticks that takes.
        granted_endpoints(
            &first_broker,
            &[compositor.clone(), carriage.clone()],
            DISPLAY_EVIDENCE_BUDGET,
        )
        .await;
        assert_eq!(
            settled(&plane, &session_key, ResourceStatus::Ready, DISPLAY_AGGREGATE_BUDGET).await,
            ResourceStatus::Ready,
            "the first boot realizes the whole graph before it is torn down"
        );
        let before = standing_endpoint_token(&plane, &carriage, DISPLAY_AGGREGATE_BUDGET).await;
        assert!(
            !before.is_empty(),
            "the first boot published the realization the restart must re-prove: {before}"
        );
        plane.shutdown().await;
        drop(plane);

        // -- the second boot, over the same durable rows -------------------
        let silent = Arc::new(d2b_provider_process::test_support::FakeFacets::new(
            Default::default(),
        ));
        silent.set_active(false);
        let DisplayComposition {
            dir: _second_dir,
            mut inputs,
            client: second_client,
            broker: second_broker,
            processes: second_processes,
        } = display_composition_with_processes(Arc::clone(&silent)).await;
        inputs.spec_store_dir = store_dir;
        let plane = Arc::new(
            ResourcePlaneV3::open(inputs)
                .await
                .expect("the second boot reopens the same Zone over the same store"),
        );
        second_client
            .set(plane.client().clone())
            .expect("the second plane's client is bound once");
        let second_realized = bind_display_guest_target(&plane).await;

        // The restarted provider admits nothing, so the graph has no standing
        // worker - and the durable rows still carry the first boot's `Ready`.
        let trail = status_trail(&plane, &session_key, DISPLAY_RESTART_WINDOW).await;
        assert!(
            trail.iter().any(|status| *status != ResourceStatus::Ready),
            "a restart that adopted the cached status would leave the session `Ready` for ever: \
             {trail:?}"
        );
        assert_ne!(
            trail.last(),
            Some(&ResourceStatus::Ready),
            "the session came back to `Ready` over no freshly observed evidence: {trail:?}"
        );
        // The carriage is a `ProducerRow` shape realized behind a `Process` row
        // the restarted provider admits nothing for, so the restarted endpoint
        // actor re-publishes the unrealized class over the realization the
        // previous boot left on the durable row. That re-publication is one
        // `ENDPOINT_REALIZE_RESYNC` away, so the wait is one such link at the
        // load factor rather than a fixed span.
        let mut carriage_token = endpoint_token(&plane, &carriage, DISPLAY_RETRY_WINDOW).await;
        let reobserved = tokio::time::Instant::now() + DISPLAY_RETRY_WINDOW;
        while !carriage_token.is_empty() && tokio::time::Instant::now() < reobserved {
            tokio::time::sleep(Duration::from_millis(50)).await;
            carriage_token = endpoint_token(&plane, &carriage, DISPLAY_RETRY_WINDOW).await;
        }
        assert!(
            carriage_token.is_empty(),
            "the restarted endpoint actor kept the cached realization instead of re-observing: \
             {carriage_token}"
        );

        // Now let the restarted provider admit what it launched, and the graph
        // has to return on its own.
        second_processes.push_adoption(d2b_provider_process::ProviderAdoption::Adopted(
            adopted_report(),
        ));
        // What that convergence has to produce is the evidence first: both
        // relationships re-granted over the RESTARTED daemon's own broker
        // socket, and the Guest this plane is bound to having applied the
        // frontend's realization. A frame is the last effect in this graph -
        // the worker row publishes its own readiness only once the Guest
        // applied the frame its launch wrote - so neither wait sits on a clock
        // that could expire mid-chain, and only the session row's own
        // aggregate answer is left on a budget.
        granted_endpoints(
            &second_broker,
            &[compositor.clone(), carriage.clone()],
            DISPLAY_EVIDENCE_BUDGET,
        )
        .await;
        let frontend_ref = ResourceRef::parse(&format!("Process/{}", frontend.name))
            .expect("the frontend's own canonical reference");
        let _ = frames_until(
            &second_realized,
            &frontend_ref.to_canonical_string(),
            DISPLAY_EVIDENCE_BUDGET,
        )
        .await;
        assert_eq!(
            settled(&plane, &session_key, ResourceStatus::Ready, DISPLAY_AGGREGATE_BUDGET).await,
            ResourceStatus::Ready,
            "the restarted session returns to `Ready` from freshly re-observed evidence"
        );
        let grants: Vec<BrokerEndpointCall> = second_broker
            .calls()
            .into_iter()
            .filter(|call| call.verb == EndpointAccessVerb::Grant)
            .collect();
        assert_eq!(
            grants.len(),
            2,
            "both relationships were re-proved over the restarted daemon's own broker socket: \
             {grants:?}"
        );
        let after = standing_endpoint_token(&plane, &carriage, DISPLAY_AGGREGATE_BUDGET).await;
        assert!(
            !after.is_empty(),
            "the restarted endpoint actor re-observed a standing realization: {after}"
        );
        assert!(
            !silent.launch_calls().is_empty(),
            "the restarted Process actor re-proved against its provider instead of adopting the \
             previous boot's status: {:?}",
            silent.launch_calls()
        );
        plane.shutdown().await;
    }

    /// Teardown closes endpoint access BEFORE it retires any row, and leaves
    /// no child of the withdrawn session behind.
    ///
    /// The ordering is only observable if the revoke really travels: a
    /// relationship actor derives the broker entry it removes from its OWN
    /// row, so a teardown that retired the row first could not issue a revoke
    /// at all. Both revokes are therefore asserted on the real socket, each
    /// naming the exact endpoint row and the exact consumer its grant admitted,
    /// and each landing after the delivery it releases. Nothing re-admits
    /// access afterwards, and the Zone is left holding no worker, no endpoint,
    /// and no relationship of this session.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread")]
    async fn a_session_teardown_revokes_on_the_wire_before_it_retires_any_child() {
        let scene = display_scene().await;
        let (endpoints, processes) = derived_display_rows(&scene.session_uid);
        let (compositor, carriage) = (endpoints[0].0.clone(), endpoints[1].0.clone());
        let (host_proxy, frontend) = display_worker_rows(&processes);
        // The graph has to be standing before a teardown runs over it, and
        // what "standing" means here is the evidence: both relationships
        // granted over this scene's real broker socket. Only the session
        // row's own aggregate answer is left on a budget.
        granted_endpoints(
            &scene.broker,
            &[compositor.clone(), carriage.clone()],
            DISPLAY_EVIDENCE_BUDGET,
        )
        .await;
        assert_eq!(
            settled(
                &scene.plane,
                &scene.session_key,
                ResourceStatus::Ready,
                DISPLAY_AGGREGATE_BUDGET,
            )
            .await,
            ResourceStatus::Ready,
            "the graph is standing, so the teardown below runs over live rows"
        );
        let grants: Vec<BrokerEndpointCall> = scene
            .broker
            .calls()
            .into_iter()
            .filter(|call| call.verb == EndpointAccessVerb::Grant)
            .collect();
        assert_eq!(
            grants.len(),
            2,
            "both relationships are delivered before the teardown begins: {grants:?}"
        );
        let delivered = grants.last().expect("a delivered relationship").at;

        // Withdraw the session row from the Zone's desired state, exactly as a
        // Nix apply that no longer declares it does.
        scene
            .plane
            .ingest_nix_bundle(&display_scene_bundle(false, 0))
            .await
            .expect("the withdrawal commits");

        // The withdrawal only commits the durable mark; each relationship's
        // own actor issues its revoke on a later pass, so the wire is read
        // until both releases have actually crossed it.
        let mut released = 0;
        let deadline = tokio::time::Instant::now() + DISPLAY_EVIDENCE_BUDGET;
        while released < 2 && tokio::time::Instant::now() < deadline {
            released = usize::from(!revokes_for(&scene.broker, &compositor).is_empty())
                + usize::from(!revokes_for(&scene.broker, &carriage).is_empty());
            if released == 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }

        for (endpoint, consumer) in [(&compositor, &host_proxy), (&carriage, &frontend)] {
            let revokes = revokes_for(&scene.broker, endpoint);
            assert!(
                !revokes.is_empty(),
                "{endpoint} released its access on the real broker socket: no revoke crossed the \
                 wire, which is what a retired-first teardown would produce"
            );
            assert_eq!(
                revokes[0].consumer,
                ResourceRef::parse(&format!("Process/{}", consumer.name))
                    .expect("the consumer's own canonical reference")
                    .to_canonical_string(),
                "the revoke names the consumer its own grant admitted"
            );
            assert!(
                revokes[0].at > delivered,
                "the revoke is issued after the delivery it releases, never before: {revokes:?}"
            );
            assert_eq!(
                scene.broker.last_call(endpoint).map(|call| call.verb),
                Some(EndpointAccessVerb::Revoke),
                "the last thing this broker hears about {endpoint} is its release"
            );
        }
        let first_revoke = scene
            .broker
            .calls()
            .iter()
            .position(|call| call.verb == EndpointAccessVerb::Revoke)
            .expect("a revoke crossed the wire");
        let after_revoke = &scene.broker.calls()[first_revoke..];
        assert!(
            !after_revoke
                .iter()
                .any(|call| call.verb == EndpointAccessVerb::Grant),
            "no access is re-admitted once it has been released: {after_revoke:?}"
        );

        for type_name in ["EndpointBinding", "Endpoint", "Process"] {
            let held = drained(&scene.plane, type_name, DISPLAY_EVIDENCE_BUDGET).await;
            assert!(
                held.is_empty(),
                "the teardown retired every {type_name} row the session derived: {held:?}"
            );
        }
        let session = drained(&scene.plane, WAYLAND_SESSION_TYPE, DISPLAY_EVIDENCE_BUDGET).await;
        assert!(
            session.is_empty(),
            "the withdrawn session row retired with its children: {session:?}"
        );
        scene.plane.shutdown().await;
    }

}
