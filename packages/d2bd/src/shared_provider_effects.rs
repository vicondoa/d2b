//! Production effects for the U12 shared host-provider drivers.
//!
//! Every typed Provider effect the driver family dispatches lives here: the
//! Network-local controller over a manager-routed child port, the persistent
//! TPM controller, the USBIP lifecycle over the broker authority ledger, the
//! SecurityKey relay lifecycle, and the authority-fenced GPU lifecycle. The
//! port is the dyn-erased [`SharedProviderDriverEffects`] boundary; the
//! daemon owns every side effect behind it (broker dispatch, authority
//! leases, the child-row ensures and the phase gates their controllers
//! publish) and the drivers own the child rows.
//!
//! U17 (KTD13): no effect here launches a process. The TPM and GPU effects
//! realize their workers as the Device Providers' declared Process rows and
//! read the phases their Process controllers publish; the launch, restart,
//! adoption, drain and teardown of every one of them is the Process
//! controller's.
//!
//! Live readiness is read through the manager view for converted rows (a
//! converted row's actor status is the only status there is, R11) and through
//! the durable store for unconverted rows - the same split the old effects
//! got from `/status/phase`. The driver never sees either.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use d2b_contracts::types::{BundleOpId, VmId};
use d2b_contracts_broker::broker_wire::BrokerCallerRole;
use d2b_contracts_resource::v3::{
    ControllerGeneration, ResourceGeneration, ResourceRef, ResourceUid, ZoneId, ZoneRevision,
    identity::ReconnectGeneration, network::NetworkProvenance, volume::VolumeSpec,
};
use d2b_core::bundle_resolver::BundleResolver;
use d2b_core_controller::authority::AuthorityRequest;
use d2b_provider_network_local::{
    artifact::{ArtifactCatalogEntry, ArtifactKind},
    controller::{
        AttachmentRealization, NetworkAdmissionIntent, NetworkAdmissionKey, NetworkAdmissionProof,
        NetworkEffectError, NetworkReconciler, NetworkResourcePort, ReconcileInput,
        ReconcileProgress,
    },
    observe::{HostNetworkOccupancy, observe_host_network},
};
use d2b_resource_runtime::context::ChildEnsure;
use d2b_resource_runtime::identity::{ResourceKey, ResourceTypeName};
use d2b_resource_runtime::manager::ResourceView;
use d2b_resource_runtime::ResourceStatus;
use d2b_contracts_resource::v3::{ResourceAssignmentFence, ResourceAssignmentScope};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::ServerState;
use crate::resource_plane_v3::ResourcePlaneV3;
use crate::resource_runtime::{ASSIGNMENT_EPOCH, ZoneResourceRuntime};
use d2b_provider_device::{DeviceComponent, DeviceResourceState};
use d2b_provider_device_gpu::facets::GpuRuntime;
use d2b_provider_device_security_key::SecurityKeyComponent;
use d2b_provider_device_tpm::facets::TpmRuntime;
use d2b_provider_device_usbip::facets::UsbipRuntime;
use d2b_provider_device_usbip::UsbipComponent;
use d2b_provider_network_local::NetworkComponent;
use d2b_provider_toolkit::{
    HOST_REF, SharedProviderEffectError, SharedProviderEffectOutcome, SharedProviderEffectPhase,
    SharedProviderEffectRequest, SharedProviderFinalize, key_ref, resource_uid,
};

/// The daemon-side handler key of one shared-provider row.
///
/// Every row's identity (its ResourceType, Provider reference, controller
/// reference, and dependency declaration) is owned by the family crate that
/// declares it; this key only selects the handler the daemon implements, so
/// the daemon and the declaring crate can never disagree on a string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SharedProviderKind {
    Network,
    UsbipDevice,
    UsbipService,
    UsbipBinding,
    SecurityKeyDevice,
    SecurityKeyService,
    SecurityKeyBinding,
    GpuDevice,
}

/// The admission mode one shared-provider effect request carries on its
/// spec (`/mode`). The wire spelling is kebab-case; an unknown or misspelled
/// mode is refused at the effect boundary rather than silently taking the
/// non-authority / non-projection branch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum SharedProviderEffectMode {
    /// The authority admission posture.
    Authority,
    /// The projection admission posture.
    Projection,
}

impl SharedProviderEffectMode {
    /// Parse the mode from the request spec, refusing unknown spellings.
    fn parse(request: &SharedProviderEffectRequest<'_>) -> Result<Self, SharedProviderEffectError> {
        let mode = request
            .spec
            .pointer("/mode")
            .and_then(Value::as_str)
            .ok_or(SharedProviderEffectError::InvalidResource)?;
        match mode {
            "authority" => Ok(Self::Authority),
            "projection" => Ok(Self::Projection),
            _ => Err(SharedProviderEffectError::InvalidResource),
        }
    }
}

impl SharedProviderKind {
    /// The Provider reference the row's spec must name.
    const fn provider_ref(self) -> &'static str {
        match self {
            Self::Network => d2b_provider_network_local::NETWORK_PROVIDER_REF,
            Self::UsbipDevice | Self::UsbipService | Self::UsbipBinding => {
                d2b_provider_device_usbip::PROVIDER_REF
            }
            Self::SecurityKeyDevice | Self::SecurityKeyService | Self::SecurityKeyBinding => {
                d2b_provider_device_security_key::PROVIDER_REF
            }
            Self::GpuDevice => d2b_provider_device_gpu::PROVIDER_REF,
        }
    }

    /// The controller reference the row's effects bind.
    const fn controller_ref(self) -> &'static str {
        match self {
            Self::Network => d2b_provider_network_local::NETWORK_CONTROLLER_REF,
            Self::UsbipDevice => d2b_provider_device::USBIP_CONTROLLER_REF,
            Self::UsbipService => d2b_provider_device_usbip::USBIP_SERVICE_CONTROLLER_REF,
            Self::UsbipBinding => d2b_provider_device_usbip::USBIP_BINDING_CONTROLLER_REF,
            Self::SecurityKeyDevice => d2b_provider_device::SECURITY_KEY_CONTROLLER_REF,
            Self::SecurityKeyService => {
                d2b_provider_device_security_key::SECURITY_KEY_SERVICE_CONTROLLER_REF
            }
            Self::SecurityKeyBinding => {
                d2b_provider_device_security_key::SECURITY_KEY_BINDING_CONTROLLER_REF
            }
            Self::GpuDevice => d2b_provider_device::GPU_CONTROLLER_REF,
        }
    }

    /// The dependency references one row declares (the declaring crate's own
    /// declaration).
    fn declared_dependency_refs(self, spec: &Value, metadata: &Value) -> Vec<ResourceRef> {
        match self {
            Self::Network => d2b_provider_network_local::declared_dependency_refs(
                NetworkComponent::Network,
                spec,
                metadata,
            ),
            Self::UsbipDevice => {
                d2b_provider_device::declared_dependency_refs(DeviceComponent::Usbip, spec, metadata)
            }
            Self::UsbipService => d2b_provider_device_usbip::declared_dependency_refs(
                UsbipComponent::Service,
                spec,
                metadata,
            ),
            Self::UsbipBinding => d2b_provider_device_usbip::declared_dependency_refs(
                UsbipComponent::Binding,
                spec,
                metadata,
            ),
            Self::SecurityKeyDevice => d2b_provider_device::declared_dependency_refs(
                DeviceComponent::SecurityKey,
                spec,
                metadata,
            ),
            Self::SecurityKeyService => d2b_provider_device_security_key::declared_dependency_refs(
                SecurityKeyComponent::Service,
                spec,
                metadata,
            ),
            Self::SecurityKeyBinding => d2b_provider_device_security_key::declared_dependency_refs(
                SecurityKeyComponent::Binding,
                spec,
                metadata,
            ),
            Self::GpuDevice => {
                d2b_provider_device::declared_dependency_refs(DeviceComponent::Gpu, spec, metadata)
            }
        }
    }
}

/// Production composition adapter for the closed shared-provider family.
///
/// The adapter performs the Provider-owned typed admission before any effect
/// call. A missing live broker/resource binding is returned as a retryable
/// refusal; it is never converted into generic convergence.
///
/// U12 (device families): the adapter no longer implements the device
/// families' driver effect traits. Each family's effects service lives in
/// its declaring crate and delegates to the runtime trait this adapter
/// implements; the sub-family facet sets the Device runtime drives (the TPM
/// and GPU ports, the USBIP dispatcher) are supplied through the
/// composition root and held here for the Device row's per-component
/// drives.
pub(crate) struct ProductionSharedProviderEffects {
    state: Arc<ServerState>,
    zone: ZoneId,
    controller_generation: ControllerGeneration,
    /// The daemon-supplied Network intent-resolution facet (U14): every
    /// method resolves against a resolver the daemon's loader fresh from the
    /// on-disk bundle, so an on-disk bundle replacement is picked up without
    /// a daemon restart (the retired `network_effect_port` reloaded the
    /// resolver on every reconcile, finalize, and effect).
    intents: Arc<dyn d2b_provider_network_local::broker::NetworkIntentSource>,
    /// The last verified trusted bundle the runtime facet serves
    /// ([`NetworkRuntime::bundle`]): `bundle()` reloads the on-disk bundle
    /// per invocation and falls back to the last verified resolver when
    /// the load fails, so an unreadable bundle never mints facts while the
    /// reconcile, finalize, and kernel paths refuse closed. Seeded at
    /// plane composition, which already verified the same bundle file.
    bundle: tokio::sync::Mutex<Arc<BundleResolver>>,
    /// The authenticated daemon-to-broker origination socket (U14).
    broker_socket: PathBuf,
    /// The daemon-supplied USBIP broker dispatch facet (U12 usbip step):
    /// the runtime builds the crate's kernel dispatcher from it. The facet
    /// set is circular with this adapter (this adapter is the family's
    /// runtime), so the composition root attaches it once after
    /// construction.
    usbip_dispatch: std::sync::OnceLock<
        Arc<dyn d2b_provider_device_usbip::facets::UsbipBrokerDispatch>,
    >,
    /// The daemon-supplied TPM facet set (U12 tpm step): the Device
    /// runtime builds the TPM crate's port from it for every TPM Device
    /// row. Circular with this adapter (this adapter is the family's
    /// runtime); attached once by the composition root after construction.
    tpm_facets: std::sync::OnceLock<d2b_provider_device_tpm::facets::TpmEffectFacets>,
    /// The daemon-supplied GPU facet set (U12 gpu step): the Device runtime
    /// builds the GPU crate's port from it for every GPU Device row.
    /// Circular with this adapter (this adapter is the family's runtime);
    /// attached once by the composition root after construction.
    gpu_facets: std::sync::OnceLock<d2b_provider_device_gpu::facets::GpuEffectFacets>,
    /// Zone-wide USBIP authority ledger (old `usbip_ledger`), shared by every
    /// USBIP Service and Binding dispatcher in the zone.
    usbip_ledger: d2b_provider_device_usbip::broker::AuthorityLedgerHandle,
    /// Zone-wide activated USBIP services (old `usbip_services`).
    usbip_services: Arc<tokio::sync::Mutex<BTreeSet<ResourceUid>>>,
    /// Scripted host-network occupancy (test-support only): a test installs a
    /// fixed snapshot so the admission path is hermetic (the observed host
    /// state would otherwise leak the machine running the suite). Production
    /// leaves this empty and always observes the live host.
    #[cfg(any(test, feature = "test-support"))]
    scripted_host_occupancy: Option<HostNetworkOccupancy>,
}

impl ProductionSharedProviderEffects {
    pub(crate) fn new(
        state: Arc<ServerState>,
        zone: ZoneId,
        controller_generation: ControllerGeneration,
        resolver: BundleResolver,
    ) -> Self {
        let broker_socket = crate::broker_socket_path(&state);
        // Per-invocation freshness (the retired adapter reloaded the trusted
        // bundle on every reconcile, finalize, and effect): the daemon's
        // neutral loader yields a fresh, fully re-verified resolver per call,
        // and the provider crate's intent source resolves every intent against
        // that per-call resolver.

        let daemon_state = Arc::clone(&state);
        let intents: Arc<dyn d2b_provider_network_local::broker::NetworkIntentSource> =
            Arc::new(
                d2b_provider_network_local::broker::LoaderNetworkIntentSource::new(move || {
                    match crate::load_bundle_resolver(&daemon_state) {
                        Ok(resolver) => Some(resolver),
                        Err(error) => {
                            tracing::warn!(
                                error = ?error,
                                "Network intent resolution: the trusted bundle load failed; \
                                 refusing closed without an intent",
                            );
                            None
                        }
                    }
                }),
            );
        Self {
            state,
            zone,
            controller_generation,
            intents,
            bundle: tokio::sync::Mutex::new(Arc::new(resolver)),
            broker_socket,
            usbip_dispatch: std::sync::OnceLock::new(),
            tpm_facets: std::sync::OnceLock::new(),
            gpu_facets: std::sync::OnceLock::new(),
            usbip_ledger: d2b_provider_device_usbip::broker::new_authority_ledger(),
            usbip_services: Arc::new(tokio::sync::Mutex::new(BTreeSet::new())),
            #[cfg(any(test, feature = "test-support"))]
            scripted_host_occupancy: None,
        }
    }

    /// Install a scripted host-network occupancy (test-support only): the
    /// admission path then observes this snapshot instead of the live host,
    /// so tests that drive the real `network_admission` are hermetic
    /// regardless of the machine they run on. Production never calls this.
    #[cfg(test)]
    pub(crate) fn with_scripted_host_occupancy(
        mut self,
        occupancy: HostNetworkOccupancy,
    ) -> Self {
        self.scripted_host_occupancy = Some(occupancy);
        self
    }

    /// The host-network occupancy admission observes: the scripted snapshot
    /// a test installed, or the live host observation.
    async fn observed_host_occupancy(
        &self,
    ) -> Result<HostNetworkOccupancy, d2b_provider_network_local::observe::HostNetworkObservationError>
    {
        #[cfg(any(test, feature = "test-support"))]
        if let Some(occupancy) = &self.scripted_host_occupancy {
            return Ok(occupancy.clone());
        }
        observe_host_network().await
    }

    /// Attach the device-family facet sets the composition root built from
    /// this adapter (U12): the USBIP broker dispatch, and the TPM and GPU
    /// facet sets whose runtime is this adapter itself. Called once, before
    /// the plane starts; a Device or USBIP effect that runs before the
    /// attachment refuses closed.
    pub(crate) fn attach_device_facets(
        &self,
        usbip_dispatch: Arc<dyn d2b_provider_device_usbip::facets::UsbipBrokerDispatch>,
        tpm_facets: d2b_provider_device_tpm::facets::TpmEffectFacets,
        gpu_facets: d2b_provider_device_gpu::facets::GpuEffectFacets,
    ) {
        let _ = self.usbip_dispatch.set(usbip_dispatch);
        let _ = self.tpm_facets.set(tpm_facets);
        let _ = self.gpu_facets.set(gpu_facets);
    }

    fn runtime(&self) -> Result<Arc<ZoneResourceRuntime>, SharedProviderEffectError> {
        // Synchronous caller on a tokio Mutex (plan U10): the slot's
        // critical sections are single Assignment/attach operations
        // (microseconds) and the pre-conversion std Mutex::lock serialized
        // instead of refusing, so a collision spins on try_lock (lock_sync
        // pattern, same as the broker rate limiter) rather than failing
        // closed - concurrent reconcile/attach traffic must be serialized,
        // never refused.
        let plane = loop {
            match self.state.resource_plane.try_lock() {
                Ok(guard) => break guard,
                Err(_) => std::hint::spin_loop(),
            }
        };
        plane
            .as_ref()
            .and_then(|plane| plane.zone(&self.zone).ok())
            .ok_or(SharedProviderEffectError::Unavailable)
    }

    /// The published v3 plane (manager-backed live rows and status).
    fn plane(&self) -> Result<Arc<ResourcePlaneV3>, SharedProviderEffectError> {
        let runtime = self.runtime()?;
        runtime
            .v3_plane()
            .map_err(|_| SharedProviderEffectError::Unavailable)
    }

    /// The live phase of one resource from the manager view.
    async fn live_phase(
        &self,
        target: &ResourceRef,
    ) -> Result<Option<&'static str>, SharedProviderEffectError> {
        let plane = self.plane()?;
        let key = ResourceKey::new(
            self.zone.as_str(),
            target.resource_type().as_str(),
            target.name().as_str(),
        );
        let view = plane
            .client()
            .get(key)
            .await
            .map_err(|_| SharedProviderEffectError::Unavailable)?;
        Ok(view.map(|view| view_phase(&view)))
    }

    /// The old-shape document of one resource (`spec`, `metadata`, live
    /// `status.phase`) from the manager view.
    async fn resource_value(
        &self,
        target: &ResourceRef,
    ) -> Result<Option<Value>, SharedProviderEffectError> {
        let plane = self.plane()?;
        let key = ResourceKey::new(
            self.zone.as_str(),
            target.resource_type().as_str(),
            target.name().as_str(),
        );
        let view = plane
            .client()
            .get(key)
            .await
            .map_err(|_| SharedProviderEffectError::Unavailable)?;
        let Some(view) = view else {
            return Ok(None);
        };
        let spec = serde_json::from_slice::<Value>(&view.spec)
            .map_err(|_| SharedProviderEffectError::InvalidResource)?;
        let metadata = if view.metadata.is_empty() {
            json!({})
        } else {
            serde_json::from_slice::<Value>(&view.metadata)
                .map_err(|_| SharedProviderEffectError::InvalidResource)?
        };
        let uid = resource_uid(&view.uid)?;
        let phase = view_phase(&view);
        Ok(Some(json!({
            "spec": spec,
            "metadata": metadata,
            "status": {"phase": phase},
            "uid": uid.as_str(),
            "generation": view.generation,
        })))
    }

    /// Whether one resource is present, not deleting, and live-Ready.
    async fn resource_ready(&self, target: &ResourceRef) -> bool {
        matches!(self.live_phase(target).await, Ok(Some("Ready")))
    }

    /// The old `dependencies_ready` barrier over the row's declared
    /// dependency references.
    async fn dependencies_ready(
        &self,
        kind: SharedProviderKind,
        request: &SharedProviderEffectRequest<'_>,
    ) -> Result<bool, SharedProviderEffectError> {
        for dependency in kind.declared_dependency_refs(&request.spec, &request.metadata) {
            if !self.resource_ready(&dependency).await {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// The Provider generation the old shared Runner resolved from the
    /// Provider row; never guessed.
    async fn provider_generation(
        &self,
        kind: SharedProviderKind,
    ) -> Result<ResourceGeneration, SharedProviderEffectError> {
        let provider_ref = ResourceRef::parse(kind.provider_ref())
            .map_err(|_| SharedProviderEffectError::InvalidResource)?;
        let plane = self.plane()?;
        let key = ResourceKey::new(
            self.zone.as_str(),
            provider_ref.resource_type().as_str(),
            provider_ref.name().as_str(),
        );
        let view = plane
            .client()
            .get(key)
            .await
            .map_err(|_| SharedProviderEffectError::Unavailable)?;
        match view.and_then(|view| ResourceGeneration::new(view.generation).ok()) {
            Some(generation) if generation.get() != 0 => Ok(generation),
            _ => Err(SharedProviderEffectError::Unavailable),
        }
    }

    fn session_generation(
        &self,
        runtime: &ZoneResourceRuntime,
    ) -> Result<ReconnectGeneration, SharedProviderEffectError> {
        runtime
            .controller_session_generation()
            .ok_or(SharedProviderEffectError::Unavailable)
    }

    /// The assignment fence the old Runner's `AssignmentFenceResolver` minted
    /// for one row: identity from the Provider row, the plane's controller
    /// generation, the live controller session, and the fixed Host target.
    async fn assignment_fence(
        &self,
        kind: SharedProviderKind,
        runtime: &ZoneResourceRuntime,
        request: &SharedProviderEffectRequest<'_>,
    ) -> Result<ResourceAssignmentFence, SharedProviderEffectError> {
        Ok(ResourceAssignmentFence {
            resource_uid: request.uid.clone(),
            resource_revision: ZoneRevision::new(request.generation.get()),
            provider_generation: self.provider_generation(kind).await?,
            controller_generation: self.controller_generation,
            controller_role: ResourceRef::parse(kind.controller_ref())
                .map_err(|_| SharedProviderEffectError::InvalidResource)?,
            target: ResourceRef::parse(HOST_REF)
                .map_err(|_| SharedProviderEffectError::InvalidResource)?,
            session_generation: self.session_generation(runtime)?,
            epoch: ASSIGNMENT_EPOCH,
            scope: ResourceAssignmentScope::Primary,
        })
    }

    /// The old-shape owner-envelope document of one effect request.
    fn envelope(request: &SharedProviderEffectRequest<'_>) -> Value {
        json!({
            "spec": request.spec.clone(),
            "metadata": request.metadata.clone(),
        })
    }
}

// ---------------------------------------------------------------------------
// Network
// ---------------------------------------------------------------------------

const NETWORK_CONFIG_VOLUME_SCHEMA_ID: &str = d2b_provider_volume_local::VOLUME_CONTENT_SCHEMA_ID;
const NETWORK_CONFIG_VOLUME_SCHEMA_VERSION: &str =
    d2b_provider_volume_local::VOLUME_CONTENT_SCHEMA_VERSION;
const NETWORK_CONFIG_CONTENT_KIND: &str = d2b_provider_volume_local::NETWORK_CONFIG_CONTENT_KIND;
const NETWORK_CONFIG_FILE_OWNER: &str = d2b_provider_volume_local::NETWORK_CONFIG_FILE_OWNER;
const NETWORK_CONFIG_FILE_MODE: &str = d2b_provider_volume_local::NETWORK_CONFIG_FILE_MODE;

/// The content fence the old effects threaded into every config-Volume
/// projection (assignment and provenance only: the identity fields were
/// consumed by the old stored-fence comparison the minted fence replaces).
#[derive(Clone)]
struct NetworkContentFence {
    provenance: NetworkProvenance,
    assignment: ResourceAssignmentFence,
}

/// Network readiness facts the controller's barriers consume (old
/// `SharedRunnerNetworkReadiness`): row presence plus the child's live phase,
/// which is the only status a converted child has (R11).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct NetworkReadiness {
    volume_ready: bool,
    guest_ready: bool,
    attachment_ready: bool,
}

/// The Network child port over the manager-routed surface: every upsert is a
/// `ctx.ensure_child` (F1) and every read is the live row.
struct NetworkChildPort<'a> {
    effects: &'a ProductionSharedProviderEffects,
    request: &'a SharedProviderEffectRequest<'a>,
    owner_ref: ResourceRef,
    uid: ResourceUid,
    fence: NetworkContentFence,
    volume_ref: ResourceRef,
    guest_ref: ResourceRef,
    agent_ref: ResourceRef,
}

impl<'a> NetworkChildPort<'a> {
    fn new(
        effects: &'a ProductionSharedProviderEffects,
        request: &'a SharedProviderEffectRequest<'a>,
        owner_ref: ResourceRef,
        uid: ResourceUid,
        fence: NetworkContentFence,
    ) -> Self {
        let vm_name =
            d2b_provider_network_local::ifname::derive_network_child_name(&uid, "vm");
        let agent_name =
            d2b_provider_network_local::ifname::derive_network_child_name(&uid, "agent");
        Self {
            effects,
            request,
            owner_ref,
            uid,
            fence,
            volume_ref: child_ref("Volume", "net-config"),
            guest_ref: child_ref("Guest", &vm_name),
            agent_ref: child_ref("Process", &agent_name),
        }
    }

    async fn current(&self, target: &ResourceRef) -> Result<Option<Value>, NetworkEffectError> {
        self.effects
            .resource_value(target)
            .await
            .map_err(|_| NetworkEffectError::ConfigVolume)
    }

    async fn upsert(&self, target: &ResourceRef, spec: Value) -> Result<(), NetworkEffectError> {
        let metadata = json!({
            "ownerRef": self.owner_ref.to_canonical_string(),
            "labels": {},
            "annotations": {},
        });
        self.request
            .children
            .ensure(ChildEnsure {
                type_name: ResourceTypeName::new(target.resource_type().as_str()),
                name: target.name().as_str().to_owned(),
                spec: serde_json::to_vec(&spec).map_err(|_| NetworkEffectError::ConfigVolume)?,
                metadata: serde_json::to_vec(&metadata)
                    .map_err(|_| NetworkEffectError::ConfigVolume)?,
            })
            .await
            .map(|_| ())
            .map_err(|_| NetworkEffectError::ConfigVolume)
    }

    async fn delete(&self, target: &ResourceRef) -> Result<(), NetworkEffectError> {
        let key = ResourceKey::new(
            self.effects.zone.as_str(),
            target.resource_type().as_str(),
            target.name().as_str(),
        );
        self.request
            .children
            .delete(&key)
            .await
            .map_err(|_| NetworkEffectError::ConfigVolume)
    }

    /// The controller's readiness barriers over the live child rows.
    async fn readiness(&self) -> Result<NetworkReadiness, NetworkEffectError> {
        let volume = self.current(&self.volume_ref).await?;
        let guest = self.current(&self.guest_ref).await?;
        let volume_phase = self
            .effects
            .live_phase(&self.volume_ref)
            .await
            .map_err(|_| NetworkEffectError::ConfigVolume)?;
        let guest_phase = self
            .effects
            .live_phase(&self.guest_ref)
            .await
            .map_err(|_| NetworkEffectError::ConfigVolume)?;
        let volume_ready = volume_phase == Some("Ready")
            && volume
                .as_ref()
                .is_some_and(|value| network_config_projection_present(value, &self.uid));
        let guest_ready = guest_phase == Some("Ready") && guest.is_some();
        let attachment_ready = volume_phase == Some("Ready")
            && volume.as_ref().is_some_and(|value| {
                value
                    .pointer("/spec/attachments")
                    .and_then(Value::as_array)
                    .is_some_and(|attachments| {
                        attachments.iter().any(|attachment| {
                            attachment.get("executionRef").and_then(Value::as_str)
                                == Some(self.guest_ref.to_canonical_string().as_str())
                        })
                    })
            });
        Ok(NetworkReadiness {
            volume_ready,
            guest_ready,
            attachment_ready,
        })
    }
}

/// The row's live phase: the canonical wire phase of the row's observed
/// status (issue #515). `ResourceStatus::wire_phase` owns the vocabulary
/// (`Deleting` renders as the `Deleted` tombstone); an unpublished or stale
/// status is not observed state of the current row and reads `Pending`.
///
/// The U17 row-owned effect ports (`tpm_effect_port`) gate on this same
/// projection, so a Provider effect and a driver read one phase vocabulary.
pub(crate) fn view_phase(view: &ResourceView) -> &'static str {
    view.observed_status()
        .as_ref()
        .map(ResourceStatus::wire_phase)
        .unwrap_or("Pending")
}

fn child_ref(resource_type: &str, name: &str) -> ResourceRef {
    ResourceRef::parse(&format!("{resource_type}/{name}"))
        .expect("derived Network child ref is canonical")
}

/// The spec-level content projection check (old
/// `network_config_content_projection_ready` without its persisted-status
/// terms: the materialization evidence lived in `status.resource`, which R11
/// deletes; the Volume actor's live Ready phase is the equivalent gate).
fn network_config_projection_present(value: &Value, volume_uid: &ResourceUid) -> bool {
    let Some(provider) = value.pointer("/spec/provider") else {
        return false;
    };
    let Some(spec) = value.get("spec") else {
        return false;
    };
    if validate_network_config_volume_spec(&mut spec.clone()).is_err() {
        return false;
    }
    provider.get("schemaId").and_then(Value::as_str) == Some(NETWORK_CONFIG_VOLUME_SCHEMA_ID)
        && provider.get("schemaVersion").and_then(Value::as_str)
            == Some(NETWORK_CONFIG_VOLUME_SCHEMA_VERSION)
        && provider.pointer("/settings/kind").and_then(Value::as_str)
            == Some(NETWORK_CONFIG_CONTENT_KIND)
        && provider
            .pointer("/settings/content")
            .and_then(|content| {
                d2b_provider_volume_local::NetworkConfigContentProjection::from_settings(content)
                    .ok()
            })
            .is_some_and(|content| content.volume_uid() == volume_uid)
}

fn validate_network_config_volume_spec(spec: &mut Value) -> Result<(), NetworkEffectError> {
    let provider_ref = spec
        .get("providerRef")
        .and_then(Value::as_str)
        .ok_or(NetworkEffectError::ConfigVolume)?;
    if provider_ref != "Provider/volume-local" {
        return Err(NetworkEffectError::NetworkAdmissionMismatch);
    }
    // Strip the wire-only fields in place (the `VolumeSpec` wire shape
    // denies unknown fields), parse the rest directly from the borrow, and
    // restore them so the caller's document is unchanged by validation.
    let mut removed = [None, None, None];
    if let Some(base) = spec.as_object_mut() {
        removed[0] = base.remove("providerRef");
        removed[1] = base.remove("updatePolicy");
        removed[2] = base.remove("provider");
    }
    let volume: VolumeSpec =
        VolumeSpec::deserialize(&*spec).map_err(|_| NetworkEffectError::ConfigVolume)?;
    let required = [
        d2b_provider_network_local::controller::NETWORK_CONFIG_FILE_DNSMASQ,
        d2b_provider_network_local::controller::NETWORK_CONFIG_FILE_NFTABLES,
        d2b_provider_network_local::controller::NETWORK_CONFIG_FILE_ROUTING,
        d2b_provider_network_local::controller::NETWORK_CONFIG_FILE_ATTACHMENTS,
    ];
    if !required.iter().all(|path| {
        volume.layout().iter().any(|entry| {
            entry.path() == *path
                && entry.entry_type() == d2b_contracts_resource::v3::volume::EntryType::File
                && entry.owner_ref().to_canonical_string() == NETWORK_CONFIG_FILE_OWNER
                && entry.group_ref().to_canonical_string() == NETWORK_CONFIG_FILE_OWNER
                && entry.mode() == NETWORK_CONFIG_FILE_MODE
        })
    }) {
        return Err(NetworkEffectError::ConfigVolume);
    }
    if let Some(base) = spec.as_object_mut() {
        if let Some(value) = std::mem::take(&mut removed[0]) {
            base.insert("providerRef".to_owned(), value);
        }
        if let Some(value) = std::mem::take(&mut removed[1]) {
            base.insert("updatePolicy".to_owned(), value);
        }
        if let Some(value) = std::mem::take(&mut removed[2]) {
            base.insert("provider".to_owned(), value);
        }
    }
    Ok(())
}

fn network_config_provider_matches(
    provider: &Value,
    volume_uid: &ResourceUid,
    owner_ref: &ResourceRef,
    marker: &str,
) -> bool {
    provider.get("schemaId").and_then(Value::as_str) == Some(NETWORK_CONFIG_VOLUME_SCHEMA_ID)
        && provider.get("schemaVersion").and_then(Value::as_str)
            == Some(NETWORK_CONFIG_VOLUME_SCHEMA_VERSION)
        && provider.pointer("/settings/kind").and_then(Value::as_str)
            == Some(NETWORK_CONFIG_CONTENT_KIND)
        && provider
            .pointer("/settings/content")
            .and_then(|content| {
                d2b_provider_volume_local::NetworkConfigContentProjection::from_settings(content)
                    .ok()
            })
            .is_some_and(|content| {
                content.volume_uid() == volume_uid
                    && content.network_ref() == owner_ref
                    && content.ownership_marker() == marker
            })
}

fn network_config_legacy_provider_matches(
    provider: &Value,
    owner_ref: &ResourceRef,
    marker: &str,
) -> bool {
    provider.get("schemaId").and_then(Value::as_str) == Some(NETWORK_CONFIG_VOLUME_SCHEMA_ID)
        && provider.get("schemaVersion").and_then(Value::as_str)
            == Some(NETWORK_CONFIG_VOLUME_SCHEMA_VERSION)
        && provider.pointer("/settings/kind").and_then(Value::as_str)
            == Some(NETWORK_CONFIG_CONTENT_KIND)
        && provider
            .pointer("/settings/ownershipMarker")
            .and_then(Value::as_str)
            == Some(marker)
        && provider
            .pointer("/settings/networkRef")
            .and_then(Value::as_str)
            == Some(owner_ref.to_canonical_string().as_str())
        && provider
            .pointer("/settings/fileOwner")
            .and_then(Value::as_str)
            == Some(NETWORK_CONFIG_FILE_OWNER)
        && provider
            .pointer("/settings/fileGroup")
            .and_then(Value::as_str)
            == Some(NETWORK_CONFIG_FILE_OWNER)
        && provider
            .pointer("/settings/fileMode")
            .and_then(Value::as_str)
            == Some(NETWORK_CONFIG_FILE_MODE)
        && provider.pointer("/settings/files").is_some()
}

fn network_config_spec_with_content(
    mut spec: Value,
    volume_uid: &ResourceUid,
    content: &d2b_provider_network_local::controller::NetworkConfigContent,
    fence: &NetworkContentFence,
    owner_ref: &ResourceRef,
) -> Result<Value, NetworkEffectError> {
    // The caller validates the document before handing it over; the only flow
    // into this helper validates the document, unchanged, immediately
    // before the call, so re-validating here would double the parse per
    // reconcile.
    let marker = d2b_contracts_resource::v3::derive_network_ownership_marker(
        &fence.provenance,
        "network-config",
    );
    if spec.get("provider").is_some_and(|provider| {
        !network_config_provider_matches(provider, volume_uid, owner_ref, &marker)
            && !network_config_legacy_provider_matches(provider, owner_ref, &marker)
    }) {
        return Err(NetworkEffectError::NetworkAdmissionMismatch);
    }
    let provider = network_config_provider_extension(volume_uid, content, owner_ref, fence, &marker)?;
    spec.as_object_mut()
        .ok_or(NetworkEffectError::ConfigVolume)?
        .insert("provider".to_owned(), provider);
    Ok(spec)
}

fn network_config_provider_extension(
    volume_uid: &ResourceUid,
    content: &d2b_provider_network_local::controller::NetworkConfigContent,
    owner_ref: &ResourceRef,
    fence: &NetworkContentFence,
    marker: &str,
) -> Result<Value, NetworkEffectError> {
    let file_owner =
        ResourceRef::parse(NETWORK_CONFIG_FILE_OWNER).map_err(|_| NetworkEffectError::ConfigVolume)?;
    let projection = d2b_provider_volume_local::NetworkConfigContentProjection::new(
        volume_uid.clone(),
        owner_ref.clone(),
        fence.provenance.clone(),
        marker,
        file_owner.clone(),
        file_owner,
        NETWORK_CONFIG_FILE_MODE,
        content.dhcp_bytes().to_vec(),
        content.firewall_bytes().to_vec(),
        content.routing.clone(),
        content.attachments.clone(),
        content.digest(),
    )
    .map_err(|_| NetworkEffectError::NetworkAdmissionMismatch)?;
    let content = serde_json::to_value(&projection).map_err(|_| NetworkEffectError::ConfigVolume)?;
    Ok(json!({
        "schemaId": NETWORK_CONFIG_VOLUME_SCHEMA_ID,
        "schemaVersion": NETWORK_CONFIG_VOLUME_SCHEMA_VERSION,
        "settings": {
            "kind": NETWORK_CONFIG_CONTENT_KIND,
            "content": content,
            "assignmentFence": {
                "resourceUid": fence.assignment.resource_uid,
                "resourceRevision": fence.assignment.resource_revision,
                "providerGeneration": fence.assignment.provider_generation,
                "controllerGeneration": fence.assignment.controller_generation,
                "controllerRole": fence.assignment.controller_role,
                "target": fence.assignment.target,
                "sessionGeneration": fence.assignment.session_generation,
                "epoch": fence.assignment.epoch,
                "scope": "primary",
            },
        },
    }))
}

impl NetworkResourcePort for NetworkChildPort<'_> {
    async fn upsert_volume_backing(&self, spec: &VolumeSpec) -> Result<(), NetworkEffectError> {
        let mut value = serde_json::to_value(spec).map_err(|_| NetworkEffectError::ConfigVolume)?;
        if let Some(current) = self.current(&self.volume_ref).await?
            && let Some(provider) = current.pointer("/spec/provider")
        {
            value
                .as_object_mut()
                .ok_or(NetworkEffectError::ConfigVolume)?
                .insert("provider".to_owned(), provider.clone());
        }
        value
            .as_object_mut()
            .ok_or(NetworkEffectError::ConfigVolume)?
            .insert(
                "providerRef".to_owned(),
                Value::String("Provider/volume-local".to_owned()),
            );
        self.upsert(&self.volume_ref, value).await
    }

    async fn upsert_volume_content(
        &self,
        content: &d2b_provider_network_local::controller::NetworkConfigContent,
    ) -> Result<(), NetworkEffectError> {
        if content.provenance() != Some(&self.fence.provenance) {
            return Err(NetworkEffectError::NetworkAdmissionMismatch);
        }
        let current = self
            .current(&self.volume_ref)
            .await?
            .ok_or(NetworkEffectError::ConfigVolume)?;
        if current.pointer("/metadata/ownerRef").and_then(Value::as_str)
            != Some(self.owner_ref.to_canonical_string().as_str())
            || current.pointer("/spec/providerRef").and_then(Value::as_str)
                != Some("Provider/volume-local")
        {
            return Err(NetworkEffectError::NetworkAdmissionMismatch);
        }
        let mut spec = current
            .get("spec")
            .cloned()
            .ok_or(NetworkEffectError::ConfigVolume)?;
        validate_network_config_volume_spec(&mut spec)?;
        let volume_uid = current
            .pointer("/uid")
            .and_then(Value::as_str)
            .and_then(|value| ResourceUid::parse(value.to_owned()).ok())
            .ok_or(NetworkEffectError::ConfigVolume)?;
        spec = network_config_spec_with_content(spec, &volume_uid, content, &self.fence, &self.owner_ref)?;
        self.upsert(&self.volume_ref, spec).await
    }

    async fn upsert_guest(
        &self,
        spec: &d2b_provider_guest::GuestSpec,
    ) -> Result<(), NetworkEffectError> {
        let mut value = serde_json::to_value(spec).map_err(|_| NetworkEffectError::ConfigVolume)?;
        value
            .as_object_mut()
            .ok_or(NetworkEffectError::ConfigVolume)?
            .insert(
                "providerRef".to_owned(),
                Value::String("Provider/runtime-cloud-hypervisor".to_owned()),
            );
        self.upsert(&self.guest_ref, value).await
    }

    async fn attach_volume(
        &self,
        attachment: &d2b_contracts_resource::v3::volume::VolumeAttachment,
    ) -> Result<(), NetworkEffectError> {
        let current = self
            .current(&self.volume_ref)
            .await?
            .ok_or(NetworkEffectError::ConfigVolume)?;
        let mut spec = current
            .get("spec")
            .cloned()
            .ok_or(NetworkEffectError::ConfigVolume)?;
        let attachments = spec
            .as_object_mut()
            .ok_or(NetworkEffectError::ConfigVolume)?
            .entry("attachments")
            .or_insert_with(|| Value::Array(Vec::new()));
        let attachments = attachments
            .as_array_mut()
            .ok_or(NetworkEffectError::ConfigVolume)?;
        let attachment =
            serde_json::to_value(attachment).map_err(|_| NetworkEffectError::ConfigVolume)?;
        if !attachments.iter().any(|current| current == &attachment) {
            attachments.push(attachment);
        }
        self.upsert(&self.volume_ref, spec).await
    }

    async fn upsert_agent(
        &self,
        spec: &d2b_contracts_resource::v3::process::ProcessSpec,
    ) -> Result<(), NetworkEffectError> {
        let mut value = serde_json::to_value(spec).map_err(|_| NetworkEffectError::ConfigVolume)?;
        value
            .as_object_mut()
            .ok_or(NetworkEffectError::ConfigVolume)?
            .insert(
                "providerRef".to_owned(),
                Value::String("Provider/system-minijail".to_owned()),
            );
        self.upsert(&self.agent_ref, value).await
    }

    async fn reconcile_mdns(&self, enabled: bool) -> Result<(), NetworkEffectError> {
        if enabled {
            return Err(NetworkEffectError::ConfigVolume);
        }
        Ok(())
    }

    async fn delete_processes(&self) -> Result<(), NetworkEffectError> {
        self.delete(&self.agent_ref).await
    }

    async fn detach_volume(&self) -> Result<(), NetworkEffectError> {
        let Some(current) = self.current(&self.volume_ref).await? else {
            return Ok(());
        };
        let mut spec = current
            .get("spec")
            .cloned()
            .ok_or(NetworkEffectError::ConfigVolume)?;
        if let Some(attachments) = spec
            .as_object_mut()
            .and_then(|spec| spec.get_mut("attachments"))
            .and_then(Value::as_array_mut)
        {
            attachments.retain(|attachment| {
                attachment.get("executionRef").and_then(Value::as_str)
                    != Some(self.guest_ref.to_canonical_string().as_str())
            });
        }
        self.upsert(&self.volume_ref, spec).await
    }

    async fn delete_guest(&self) -> Result<(), NetworkEffectError> {
        self.delete(&self.guest_ref).await
    }

    async fn delete_volume(&self) -> Result<(), NetworkEffectError> {
        self.delete(&self.volume_ref).await
    }
}

impl ProductionSharedProviderEffects {
    /// Old `network_admission`: root-owned admission evidence for one
    /// Network row.
    async fn network_admission(
        &self,
        runtime: &ZoneResourceRuntime,
        request: &SharedProviderEffectRequest<'_>,
        spec: &d2b_contracts_resource::v3::network::NetworkSpec,
        resolver: &d2b_core::bundle_resolver::BundleResolver,
    ) -> Result<NetworkAdmissionProof, SharedProviderEffectError> {
        let zone_uid = runtime
            .authority_zone_uid()
            .cloned()
            .ok_or(SharedProviderEffectError::Unavailable)?;
        let network_generation = request.generation;
        let network_ref = key_ref(&request.target).to_canonical_string();
        let mut guest_uids = Vec::with_capacity(spec.attachments().len());
        let mut attachment_generation = network_generation.get();
        for attachment in spec.attachments() {
            let attached = self
                .resource_value(attachment.execution_ref())
                .await?
                .ok_or(SharedProviderEffectError::InvalidResource)?;
            // The attached row's authoritative zone is the zone its key
            // resolved under (`resource_value` reads the plane for
            // `self.zone`); the stored metadata carries no zone, so the old
            // `/metadata/zone` read was always `None` and refused every
            // attachment unconditionally. `request.zone` is external input,
            // so the fence stays: a request naming a zone the resolved rows
            // cannot be in is refused.
            if self.zone.as_str() != request.zone.as_str() {
                return Err(SharedProviderEffectError::InvalidResource);
            }
            attachment_generation = attachment_generation.max(
                attached
                    .get("generation")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
            );
            if attachment.execution_ref().resource_type().as_str() == "Guest" {
                guest_uids.push(
                    attached
                        .get("uid")
                        .and_then(Value::as_str)
                        .and_then(|value| ResourceUid::parse(value.to_owned()).ok())
                        .ok_or(SharedProviderEffectError::InvalidResource)?,
                );
            }
            let reciprocal = attached
                .pointer("/spec/networkAttachments")
                .and_then(Value::as_array)
                .is_some_and(|attachments| {
                    attachments.iter().any(|candidate| {
                        candidate.get("networkRef").and_then(Value::as_str)
                            == Some(network_ref.as_str())
                    })
                });
            if !reciprocal {
                return Err(SharedProviderEffectError::InvalidResource);
            }
        }
        for guest in runtime
            .committed_resources_of_type("Guest")
            .await
            .map_err(|_| SharedProviderEffectError::Unavailable)?
        {
            let attached = guest
                .pointer("/spec/networkAttachments")
                .and_then(Value::as_array)
                .is_some_and(|attachments| {
                    attachments.iter().any(|candidate| {
                        candidate.get("networkRef").and_then(Value::as_str)
                            == Some(network_ref.as_str())
                    })
                });
            if !attached {
                continue;
            }
            // The committed Guest rows were resolved under `self.zone` (the
            // type-scoped manager list selects the plane's own zone), so the
            // row's authoritative zone is `self.zone`; the fence compares it
            // against the request zone instead of a projection-synthesised
            // field.
            if self.zone.as_str() != request.zone.as_str() {
                return Err(SharedProviderEffectError::InvalidResource);
            }
            let guest_uid = guest
                .pointer("/metadata/uid")
                .and_then(Value::as_str)
                .and_then(|value| ResourceUid::parse(value.to_owned()).ok())
                .ok_or(SharedProviderEffectError::InvalidResource)?;
            guest_uids.push(guest_uid);
            let generation = guest
                .pointer("/metadata/generation")
                .and_then(Value::as_u64)
                .ok_or(SharedProviderEffectError::InvalidResource)?;
            attachment_generation = attachment_generation.max(generation);
        }
        let installed_generation = resolver
            .installed_generation_identity()
            .and_then(|identity| {
                d2b_contracts_resource::v3::ResourceBundleGenerationId::parse(
                    identity.as_str().to_owned(),
                )
                .ok()
            })
            .ok_or(SharedProviderEffectError::Unavailable)?;
        let attachment_generation =
            ResourceGeneration::new(attachment_generation).map_err(|_| SharedProviderEffectError::InvalidResource)?;
        let intent = NetworkAdmissionIntent::new(
            NetworkAdmissionKey::new(
                zone_uid,
                request.uid.clone(),
                network_generation,
                attachment_generation,
                installed_generation,
            ),
            spec.clone(),
            guest_uids,
        )
        .map_err(|_| SharedProviderEffectError::InvalidResource)?;
        let plane = self
            .state
            .resource_plane
            .lock()
            .await
            .as_ref()
            .map(Arc::clone)
            .ok_or(SharedProviderEffectError::Unavailable)?;
        let occupancy = self
            .observed_host_occupancy()
            .await
            .map_err(|_| SharedProviderEffectError::Unavailable)?;
        plane
            .network_admission_index()
            .lock()
            .await
            .admit(intent, &occupancy)
            .map_err(|_| SharedProviderEffectError::Unavailable)
    }

    /// The Network content fence of one effect call (old
    /// `SharedRunnerNetworkContentFence`).
    async fn network_content_fence(
        &self,
        kind: SharedProviderKind,
        runtime: &ZoneResourceRuntime,
        request: &SharedProviderEffectRequest<'_>,
        admission: &NetworkAdmissionProof,
    ) -> Result<NetworkContentFence, SharedProviderEffectError> {
        let assignment = self.assignment_fence(kind, runtime, request).await?;
        let provenance = NetworkProvenance::new(
            admission.key().zone_uid().clone(),
            admission.key().network_uid().clone(),
            admission.key().network_generation(),
            admission.key().attachment_generation(),
            admission.key().bundle_generation().clone(),
        );
        Ok(NetworkContentFence {
            provenance,
            assignment,
        })
    }

    /// The Network reconcile input over the live child rows.
    fn network_input(
        &self,
        spec: &d2b_contracts_resource::v3::network::NetworkSpec,
        request: &SharedProviderEffectRequest<'_>,
        admission: NetworkAdmissionProof,
        readiness: NetworkReadiness,
        attachments: Vec<AttachmentRealization>,
    ) -> ReconcileInput {
        let mdns_enabled = request
            .spec
            .pointer("/mdns/enable")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        ReconcileInput {
            spec: spec.clone(),
            mdns_enabled,
            network_uid: request.uid.clone(),
            network_generation: request.generation,
            attachment_generation: admission.key().attachment_generation(),
            installed_generation: admission.key().bundle_generation().clone(),
            admission,
            artifact_catalog: vec![ArtifactCatalogEntry::new(
                spec.net_vm_system_artifact_id().clone(),
                ArtifactKind::NixosSystem,
            )],
            user_ready: true,
            host_memory_budget_available:
                d2b_provider_network_local::controller::CONFIG_VOLUME_MAX_BYTES,
            volume_ready: readiness.volume_ready,
            guest_ready: readiness.guest_ready,
            volume_attachment_ready: readiness.attachment_ready,
            workload_fds_closed: true,
            agent_deleted: true,
            mdns_deleted: !mdns_enabled,
            volume_attachment_removed: true,
            guest_deleted: true,
            volume_deleted: true,
            attachments,
        }
    }

    fn network_spec(
        &self,
        request: &SharedProviderEffectRequest<'_>,
    ) -> Result<d2b_contracts_resource::v3::network::NetworkSpec, SharedProviderEffectError> {
        let mut spec_value = request
            .spec
            .clone();
        if let Some(spec) = spec_value.as_object_mut() {
            for field in ["providerRef", "updatePolicy", "provider"] {
                spec.remove(field);
            }
        }
        serde_json::from_value(spec_value).map_err(|_| SharedProviderEffectError::InvalidResource)
    }
}

// ---------------------------------------------------------------------------
// USBIP
// ---------------------------------------------------------------------------

/// The production USBIP port over the zone-wide authority ledger (old
/// `SharedRunnerUsbipPort`).
type SharedRunnerUsbipPort<'a> =
    d2b_provider_device_usbip::ProductionPort<d2b_provider_device_usbip::broker::KernelUsbipDispatcher<'a>>;

impl ProductionSharedProviderEffects {
    /// Old `usbip_service_port`: the broker-backed dispatcher for one USBIP
    /// Service, over the zone-wide authority ledger.
    async fn usbip_service_port<'a>(
        &'a self,
        runtime: &ZoneResourceRuntime,
        request: &SharedProviderEffectRequest<'_>,
    ) -> Result<
        (ResourceUid, bool, SharedRunnerUsbipPort<'a>),
        SharedProviderEffectError,
    > {
        let zone_uid = runtime
            .authority_zone_uid()
            .cloned()
            .ok_or(SharedProviderEffectError::Unavailable)?;
        let device_ref = ResourceRef::parse(
            request
                .spec
                .pointer("/backingDeviceRef")
                .and_then(Value::as_str)
                .ok_or(SharedProviderEffectError::InvalidResource)?,
        )
        .map_err(|_| SharedProviderEffectError::InvalidResource)?;
        let device = self
            .resource_value(&device_ref)
            .await?
            .ok_or(SharedProviderEffectError::InvalidResource)?;
        if device.pointer("/spec/providerRef").and_then(Value::as_str)
            != Some(d2b_provider_device_usbip::PROVIDER_REF)
            || device.pointer("/status/phase").and_then(Value::as_str) != Some("Ready")
        {
            return Err(SharedProviderEffectError::Unavailable);
        }
        let device_uid = device
            .pointer("/metadata/uid")
            .and_then(Value::as_str)
            .and_then(|value| ResourceUid::parse(value.to_owned()).ok())
            .ok_or(SharedProviderEffectError::InvalidResource)?;
        let env = request
            .spec
            .pointer("/env")
            .and_then(Value::as_str)
            .unwrap_or(request.target.name.as_str());
        let physical_key =
            d2b_provider_device_usbip::core_adapter::UsbipCoreAdapter::physical_usb_backing_key(
                device_uid.as_str().as_bytes(),
            )
            .as_bytes();
        let binding_context = d2b_provider_device_usbip::broker::UsbipBindingContext::new(
            request.target.name.as_str(),
            env,
            format!("shared-usbip-bind-{}", request.uid.as_str()),
            format!("shared-usbip-runner-{}", request.uid.as_str()),
            physical_key,
        )
        .map_err(|_| SharedProviderEffectError::InvalidResource)?;
        // U12 usbip step: the kernel dispatcher is the declaring crate's own
        // `KernelUsbipDispatcher`, built from the daemon-supplied broker
        // dispatch facet and the zone's shared authority ledger.
        let dispatch = self
            .usbip_dispatch
            .get()
            .ok_or(SharedProviderEffectError::Unavailable)?;
        let port = d2b_provider_device_usbip::broker::KernelUsbipDispatcher::new(
            dispatch.as_ref(),
            binding_context,
            self.usbip_ledger.clone(),
        )
        .into_port();
        let mode = SharedProviderEffectMode::parse(request)?;
        let opted_in = mode == SharedProviderEffectMode::Authority;
        Ok((zone_uid, opted_in, port))
    }
}

impl ProductionSharedProviderEffects {
    /// One GPU authority digest (old `DaemonSharedProviderEffects::gpu_digest`
    /// with the driver's controller generation in place of the old context).
    fn gpu_digest(
        &self,
        domain: &str,
        request: &SharedProviderEffectRequest<'_>,
        assignment_epoch: u64,
        extra: &str,
    ) -> [u8; 32] {
        let mut digest = Sha256::new();
        digest.update(domain.as_bytes());
        digest.update([0]);
        digest.update(request.uid.as_str().as_bytes());
        digest.update([0]);
        digest.update(request.generation.get().to_be_bytes());
        digest.update(self.controller_generation.get().to_be_bytes());
        digest.update(assignment_epoch.to_be_bytes());
        digest.update(extra.as_bytes());
        digest.finalize().into()
    }

    /// Old `gpu_admission`: the authority-fenced admission for one GPU Device.
    async fn gpu_admission(
        &self,
        request: &SharedProviderEffectRequest<'_>,
    ) -> Result<
        (
            Arc<ZoneResourceRuntime>,
            d2b_provider_device_gpu::GpuAuthorityAdmission,
            d2b_provider_device_gpu::GpuEffectTokenSet,
            d2b_provider_device_gpu::GpuSettings,
            ResourceRef,
        ),
        SharedProviderEffectError,
    > {
        let runtime = self.runtime()?;
        if runtime.authority_zone_uid().is_none() {
            return Err(SharedProviderEffectError::Unavailable);
        }
        let holder_ref = request.owner_ref()?;
        if !matches!(holder_ref.resource_type().as_str(), "Guest" | "Host") {
            return Err(SharedProviderEffectError::InvalidResource);
        }
        if self.resource_value(&holder_ref).await?.is_none() {
            return Err(SharedProviderEffectError::InvalidResource);
        }
        let hosts = runtime
            .committed_resources_of_type("Host")
            .await
            .map_err(|_| SharedProviderEffectError::Unavailable)?;
        let [host] = hosts.as_slice() else {
            return Err(SharedProviderEffectError::InvalidResource);
        };
        let host_uid = host
            .pointer("/metadata/uid")
            .and_then(Value::as_str)
            .and_then(|value| ResourceUid::parse(value.to_owned()).ok())
            .ok_or(SharedProviderEffectError::InvalidResource)?;
        let settings: d2b_provider_device_gpu::GpuSettings =
            match request.spec.pointer("/provider/settings") {
                Some(settings) => serde_json::from_value(settings.clone())
                    .map_err(|_| SharedProviderEffectError::InvalidResource)?,
                None => d2b_provider_device_gpu::GpuSettings::default(),
            };
        let arbitration: d2b_contracts_resource::v3::device::DeviceArbitration =
            serde_json::from_value(
                request
                    .spec
                    .pointer("/arbitration")
                    .cloned()
                    .unwrap_or_else(|| Value::String("exclusive".to_owned())),
            )
            .map_err(|_| SharedProviderEffectError::InvalidResource)?;
        let max_holders = request
            .spec
            .pointer("/maxConcurrentClaims")
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .unwrap_or(1);
        let assignment = self
            .assignment_fence(SharedProviderKind::GpuDevice, &runtime, request)
            .await?;
        let assignment_epoch = assignment.epoch;
        let session_generation = assignment.session_generation.get();
        let mut backing_digest =
            self.gpu_digest("d2b:gpu-backing/v2", request, assignment_epoch, "backing");
        backing_digest[..8].copy_from_slice(&session_generation.to_be_bytes());
        let platform_digest =
            self.gpu_digest("d2b:gpu-platform/v2", request, assignment_epoch, host_uid.as_str());
        let gpu_principal_digest =
            self.gpu_digest("d2b:gpu-principal/v2", request, assignment_epoch, "gpu");
        let owner = d2b_provider_device_gpu::GpuOwnerProof::new(
            ResourceRef::parse(&format!("Zone/{}", self.zone.as_str()))
                .map_err(|_| SharedProviderEffectError::InvalidResource)?,
            holder_ref.clone(),
            request.uid.clone(),
            host_uid,
            request.generation,
        )
        .map_err(|_| SharedProviderEffectError::InvalidResource)?;
        let mut admission = d2b_provider_device_gpu::GpuAuthorityAdmission::new(
            owner,
            d2b_provider_device_gpu::GpuBackingToken::from_core(backing_digest),
            d2b_provider_device_gpu::GpuPlatformToken::from_core(platform_digest),
            arbitration,
            u32::try_from(max_holders).map_err(|_| SharedProviderEffectError::InvalidResource)?,
            settings.render_node_only,
            d2b_provider_device_gpu::GpuPrincipalToken::from_core(gpu_principal_digest),
        )
        .map_err(|_| SharedProviderEffectError::InvalidResource)?;
        if settings.video_sidecar {
            admission = admission
                .with_video_principal(d2b_provider_device_gpu::GpuPrincipalToken::from_core(
                    self.gpu_digest("d2b:gpu-principal/v2", request, assignment_epoch, "video"),
                ))
                .map_err(|_| SharedProviderEffectError::InvalidResource)?;
        }
        let mut token_values = if settings.render_node_only {
            d2b_provider_device_gpu::vocabulary::GPU_RENDER_NODE_GRANT_CLASSES.to_vec()
        } else {
            d2b_provider_device_gpu::vocabulary::GPU_GRANT_CLASSES.to_vec()
        };
        if settings.video_sidecar && settings.video_nvidia_decode {
            token_values.extend(d2b_provider_device_gpu::vocabulary::GPU_VIDEO_GRANT_CLASSES);
        }
        let tokens = d2b_provider_device_gpu::GpuEffectTokenSet::from_core(
            token_values
                .into_iter()
                .map(|device_class| {
                    d2b_provider_device_gpu::GpuEffectToken::from_core(self.gpu_digest(
                        "d2b:gpu-device-grant/v2",
                        request,
                        assignment_epoch,
                        device_class,
                    ))
                })
                .collect(),
        )
        .map_err(|_| SharedProviderEffectError::InvalidResource)?;
        Ok((runtime, admission, tokens, settings, holder_ref))
    }
}

// ---------------------------------------------------------------------------
// Effects
// ---------------------------------------------------------------------------

impl ProductionSharedProviderEffects {
    /// Reconcile one Network row through the Network-local controller.
    async fn reconcile_network(
        &self,
        request: &SharedProviderEffectRequest<'_>,
    ) -> Result<SharedProviderEffectOutcome, SharedProviderEffectError> {
        if !self
            .dependencies_ready(SharedProviderKind::Network, request)
            .await?
        {
            return Ok(SharedProviderEffectOutcome::phase(
                SharedProviderEffectPhase::Pending,
            ));
        }
        let spec = self.network_spec(request)?;
        // Per-invocation freshness (the retired adapter reloaded the trusted
        // bundle on every reconcile): the worker-side load re-verifies every
        // artifact hash, so a replaced bundle is admitted under its new
        // generation without a daemon restart, and a missing, tampered, or
        // unreadable bundle fails this reconcile closed.
        let resolver = crate::load_bundle_resolver_on_worker(&self.state)
            .await
            .map_err(|_| SharedProviderEffectError::Unavailable)?;
        let runtime = self.runtime()?;
        let admission = self
            .network_admission(&runtime, request, &spec, &resolver)
            .await?;
        let fence = self
            .network_content_fence(SharedProviderKind::Network, &runtime, request, &admission)
            .await?;
        let owner_ref = key_ref(&request.target).clone();
        let children = NetworkChildPort::new(self, request, owner_ref, request.uid.clone(), fence);
        let readiness = children
            .readiness()
            .await
            .map_err(|_| SharedProviderEffectError::Unavailable)?;
        let broker_context = crate::resolve_network_effect_context(
            &Self::envelope(request),
            &resolver,
            &admission,
        )
        .map_err(|_| SharedProviderEffectError::Unavailable)?
        .with_host_global_nic_admission();
        // U14: the kernel-invoking adapter is the declaring crate's own
        // `KernelNetworkBroker`, built from the daemon-supplied facets (the
        // origination socket, the AdminUid authority, and the resolved
        // bundle intents).
        let effects = d2b_provider_network_local::broker::BrokerNetworkEffectPort::new(
            d2b_provider_network_local::broker::KernelNetworkBroker::new(
                d2b_provider_network_local::broker::NetworkBrokerFacets::new(
                    crate::broker_socket_path(&self.state),
                    BrokerCallerRole::AdminUid {
                        uid: self.state.daemon_uid,
                    },
                    Arc::clone(&self.intents),
                ),
            ),
            broker_context,
        );
        let input = self.network_input(&spec, request, admission, readiness, Vec::new());
        match NetworkReconciler::new(effects, children)
            .reconcile(&input)
            .await
        {
            Ok(ReconcileProgress::Ready) => Ok(SharedProviderEffectOutcome::phase(
                SharedProviderEffectPhase::Ready,
            )),
            Ok(
                ReconcileProgress::Pending(_)
                | ReconcileProgress::Requeue(_)
                | ReconcileProgress::Blocked(_),
            ) => Ok(SharedProviderEffectOutcome::phase(
                SharedProviderEffectPhase::Pending,
            )),
            Err(_) => Err(SharedProviderEffectError::Unavailable),
        }
    }

    /// Reconcile one TPM Device through the persistent TPM controller.
    #[allow(clippy::disallowed_methods, reason = "synchronous path")]
    async fn reconcile_tpm(
        &self,
        request: &SharedProviderEffectRequest<'_>,
        state: &DeviceResourceState,
    ) -> Result<SharedProviderEffectOutcome, SharedProviderEffectError> {
        let execution_ref = request
            .spec
            .pointer("/provider/settings/executionRef")
            .and_then(Value::as_str)
            .and_then(|value| ResourceRef::parse(value).ok())
            .unwrap_or_else(|| ResourceRef::parse(HOST_REF).expect("Host ref"));
        let holder = request.owner_ref()?;
        if holder.resource_type().as_str() != "Guest" {
            tracing::warn!(
                device = %key_ref(&request.target).to_canonical_string(),
                owner = %holder.to_canonical_string(),
                "TPM device reconcile refused: the Device is not owned by a Guest",
            );
            return Err(SharedProviderEffectError::InvalidResource);
        }
        let device_ref = key_ref(&request.target).clone();
        let runtime = self.runtime().inspect_err(|_| {
            tracing::warn!(
                device = %device_ref.to_canonical_string(),
                "TPM device reconcile refused: the Zone resource runtime is not attached",
            );
        })?;
        let vm_id = VmId::new(holder.name().as_str());
        let migration_intent = BundleOpId::new(format!(
            "{}{}",
            d2b_provider_device_tpm::vocabulary::TPM_LEGACY_MIGRATION_INTENT_PREFIX,
            vm_id.as_str()
        ));
        let decision = runtime
            .tpm_device_is_admitted(
                &request.uid,
                &device_ref,
                vm_id.as_str(),
                &request.operation_id,
                None,
            )
            .await
            .map_err(|error| {
                tracing::warn!(
                    device = %device_ref.to_canonical_string(),
                    error = %error,
                    "TPM device admission refused",
                );
                SharedProviderEffectError::Unavailable
            })?;
        let mut controller = {
            let mut controllers = state
                .tpm_controllers
                .lock() // async-gate-allow: synchronous lock acquisition, no await while the guard is held
                .map_err(|_| SharedProviderEffectError::Unavailable)?;
            match controllers.remove(&request.uid) {
                Some(controller) => controller,
                None => d2b_provider_device_tpm::TpmResourceController::new(
                    request.uid.clone(),
                    key_ref(&request.target).clone(),
                    execution_ref.clone(),
                )
                .map_err(|_| SharedProviderEffectError::InvalidResource)?,
            }
        };
        // U12 tpm step: the resource effect port is the TPM crate's own
        // implementation, built from the daemon-supplied facet set.
        let tpm_facets = self
            .tpm_facets
            .get()
            .cloned()
            .ok_or(SharedProviderEffectError::Unavailable)?;
        let result = d2b_provider_device_tpm::effects_service::reconcile_device_tpm_controller(
            tpm_facets,
            vm_id.clone(),
            migration_intent,
            decision,
            d2b_provider_device_tpm::effects_service::AdmittedTpmDevice::from_row(
                request.uid.clone(),
                key_ref(&request.target).clone(),
                self.zone.as_str(),
                execution_ref,
                request.operation_id.clone(),
            ),
            request.children,
            &mut controller,
        )
        .await
        .map_err(|error| {
            tracing::warn!(
                error = ?error,
                device = %key_ref(&request.target).to_canonical_string(),
                "TPM device controller reconcile failed",
            );
            SharedProviderEffectError::Unavailable
        });
        match result {
            Ok(outcome) => {
                {
                    let mut controllers = state
                        .tpm_controllers
                        .lock() // async-gate-allow: synchronous lock acquisition, no await while the guard is held
                        .map_err(|_| SharedProviderEffectError::Unavailable)?;
                    controllers.insert(request.uid.clone(), controller);
                }
                match outcome {
                    d2b_provider_device_tpm::TpmResourceOutcome::Ready => {
                        Ok(SharedProviderEffectOutcome::phase(
                            SharedProviderEffectPhase::Ready,
                        ))
                    }
                    d2b_provider_device_tpm::TpmResourceOutcome::Retry => {
                        Ok(SharedProviderEffectOutcome::phase(
                            SharedProviderEffectPhase::Pending,
                        ))
                    }
                    d2b_provider_device_tpm::TpmResourceOutcome::Failed
                    | d2b_provider_device_tpm::TpmResourceOutcome::VolumeRetained => {
                        Err(SharedProviderEffectError::Unavailable)
                    }
                }
            }
            Err(error) => {
                {
                    let mut controllers = state
                        .tpm_controllers
                        .lock() // async-gate-allow: synchronous lock acquisition, no await while the guard is held
                        .map_err(|_| SharedProviderEffectError::Unavailable)?;
                    controllers.insert(request.uid.clone(), controller);
                }
                Err(error)
            }
        }
    }

    /// Reconcile one USBIP Device row: Ready once a Service for the backing
    /// Device is live.
    async fn reconcile_usbip_device(
        &self,
        request: &SharedProviderEffectRequest<'_>,
    ) -> Result<SharedProviderEffectOutcome, SharedProviderEffectError> {
        if !self
            .dependencies_ready(SharedProviderKind::UsbipDevice, request)
            .await?
        {
            return Ok(SharedProviderEffectOutcome::phase(
                SharedProviderEffectPhase::Pending,
            ));
        }
        let runtime = self.runtime()?;
        let services = runtime
            .committed_resources_of_type(
                d2b_provider_device_usbip::USB_SERVICE_RESOURCE_TYPE,
            )
            .await
            .map_err(|_| SharedProviderEffectError::Unavailable)?;
        let device_ref = key_ref(&request.target).to_canonical_string();
        let ready = services.iter().any(|service| {
            service.pointer("/spec/providerRef").and_then(Value::as_str)
                == Some(d2b_provider_device_usbip::PROVIDER_REF)
                && service
                    .pointer("/spec/backingDeviceRef")
                    .and_then(Value::as_str)
                    == Some(device_ref.as_str())
                && service.pointer("/status/phase").and_then(Value::as_str)
                    == Some("Ready")
        });
        Ok(SharedProviderEffectOutcome::phase(if ready {
            SharedProviderEffectPhase::Ready
        } else {
            SharedProviderEffectPhase::Pending
        }))
    
    }

    /// Reconcile one USBIP Service or Binding row through its typed
    /// lifecycle controller.
    async fn reconcile_usbip(
        &self,
        component: UsbipComponent,
        request: &SharedProviderEffectRequest<'_>,
    ) -> Result<SharedProviderEffectOutcome, SharedProviderEffectError> {
        let kind = match component {
            UsbipComponent::Service => SharedProviderKind::UsbipService,
            UsbipComponent::Binding => SharedProviderKind::UsbipBinding,
        };
        if !self.dependencies_ready(kind, request).await? {
            return Ok(SharedProviderEffectOutcome::phase(
                SharedProviderEffectPhase::Pending,
            ));
        }
        match component {
            UsbipComponent::Service => {
                if self
                    .usbip_services
                    .lock()
                    .await
                    .contains(&request.uid)
                {
                    return Ok(SharedProviderEffectOutcome::phase(
                        SharedProviderEffectPhase::Ready,
                    ));
                }
                let runtime = self.runtime()?;
                let (zone_uid, zone_opted_in, mut port) =
                    self.usbip_service_port(&runtime, request).await?;
                let mut lifecycle = d2b_provider_device_usbip::ServiceLifecycle::new(
                    zone_uid.clone(),
                    request.uid.clone(),
                );
                lifecycle
                    .activate(zone_opted_in, zone_uid, &mut port)
                    .map_err(|_| SharedProviderEffectError::Unavailable)?;
                self.usbip_services.lock().await.insert(request.uid.clone());
                Ok(SharedProviderEffectOutcome::phase(
                    SharedProviderEffectPhase::Ready,
                ))
            }
            UsbipComponent::Binding => {
                let service_ref = ResourceRef::parse(
                    request
                        .spec
                        .pointer("/serviceRef")
                        .and_then(Value::as_str)
                        .ok_or(SharedProviderEffectError::InvalidResource)?,
                )
                .map_err(|_| SharedProviderEffectError::InvalidResource)?;
                let guest_ref = ResourceRef::parse(
                    request
                        .spec
                        .pointer("/guestRef")
                        .and_then(Value::as_str)
                        .ok_or(SharedProviderEffectError::InvalidResource)?,
                )
                .map_err(|_| SharedProviderEffectError::InvalidResource)?;
                let runtime = self.runtime()?;
                let zone_uid = runtime
                    .authority_zone_uid()
                    .cloned()
                    .ok_or(SharedProviderEffectError::Unavailable)?;
                let service = self
                    .resource_value(&service_ref)
                    .await?
                    .ok_or(SharedProviderEffectError::Unavailable)?;
                if service.pointer("/spec/providerRef").and_then(Value::as_str)
                    != Some(d2b_provider_device_usbip::PROVIDER_REF)
                {
                    return Err(SharedProviderEffectError::InvalidResource);
                }
                if service.pointer("/status/phase").and_then(Value::as_str) != Some("Ready") {
                    return Ok(SharedProviderEffectOutcome::phase(
                        SharedProviderEffectPhase::Pending,
                    ));
                }
                let service_uid = service
                    .pointer("/metadata/uid")
                    .and_then(Value::as_str)
                    .and_then(|value| ResourceUid::parse(value.to_owned()).ok())
                    .ok_or(SharedProviderEffectError::InvalidResource)?;
                let service_generation = service
                    .pointer("/metadata/generation")
                    .and_then(Value::as_u64)
                    .and_then(|value| ResourceGeneration::new(value).ok())
                    .ok_or(SharedProviderEffectError::InvalidResource)?;
                let guest = self
                    .resource_value(&guest_ref)
                    .await?
                    .ok_or(SharedProviderEffectError::Unavailable)?;
                if guest.pointer("/status/phase").and_then(Value::as_str) != Some("Ready") {
                    return Ok(SharedProviderEffectOutcome::phase(
                        SharedProviderEffectPhase::Pending,
                    ));
                }
                let guest_uid = guest
                    .pointer("/metadata/uid")
                    .and_then(Value::as_str)
                    .and_then(|value| ResourceUid::parse(value.to_owned()).ok())
                    .ok_or(SharedProviderEffectError::InvalidResource)?;
                let admission = self
                    .assignment_fence(SharedProviderKind::UsbipBinding, &runtime, request)
                    .await?;
                let admission = d2b_provider_device_usbip::UsbipBindingAdmission::new(
                    zone_uid,
                    request.uid.clone(),
                    service_uid,
                    guest_uid,
                    service_generation,
                    admission.epoch,
                )
                .map_err(|_| SharedProviderEffectError::InvalidResource)?;
                let mut controller =
                    d2b_provider_device_usbip::UsbipBindingController::new_admitted(
                        &key_ref(&request.target),
                        &service_ref,
                        &guest_ref,
                        admission,
                    )
                    .map_err(|_| SharedProviderEffectError::InvalidResource)?;
                let desired = d2b_provider_device_usbip::binding_child_resources(
                    &key_ref(&request.target),
                    &service_ref,
                    &guest_ref,
                )
                .map_err(|_| SharedProviderEffectError::InvalidResource)?;
                let ready = self
                    .child_set_ready(
                        &desired,
                        &request.target,
                        &request.zone,
                    )
                    .await?;
                if ready {
                    controller
                        .observe_children(true)
                        .map_err(|_| SharedProviderEffectError::Unavailable)?;
                }
                Ok(SharedProviderEffectOutcome::phase(if ready {
                    SharedProviderEffectPhase::Ready
                } else {
                    SharedProviderEffectPhase::Pending
                }))
            }
        }
    }

    /// Reconcile one security-key Device row: Ready once the Device's own
    /// status projection reports the key present and confirmed.
    async fn reconcile_security_key_device(
        &self,
        request: &SharedProviderEffectRequest<'_>,
    ) -> Result<SharedProviderEffectOutcome, SharedProviderEffectError> {
        if !self
            .dependencies_ready(SharedProviderKind::SecurityKeyDevice, request)
            .await?
        {
            return Ok(SharedProviderEffectOutcome::phase(
                SharedProviderEffectPhase::Pending,
            ));
        }
        let projection = request.status.clone().unwrap_or_else(|| json!({}));
        let admitted = projection.get("devicePresent").and_then(Value::as_bool) == Some(true)
            && projection.get("fidoConfirmed").and_then(Value::as_bool) == Some(true);
        Ok(SharedProviderEffectOutcome::projection(
            if admitted {
                SharedProviderEffectPhase::Ready
            } else {
                SharedProviderEffectPhase::Pending
            },
            projection,
        ))
    
    }

    /// Reconcile one security-key Service or Binding row through its
    /// typed lifecycle controller.
    async fn reconcile_security_key(
        &self,
        component: SecurityKeyComponent,
        request: &SharedProviderEffectRequest<'_>,
    ) -> Result<SharedProviderEffectOutcome, SharedProviderEffectError> {
        let kind = match component {
            SecurityKeyComponent::Service => SharedProviderKind::SecurityKeyService,
            SecurityKeyComponent::Binding => SharedProviderKind::SecurityKeyBinding,
        };
        if !self.dependencies_ready(kind, request).await? {
            return Ok(SharedProviderEffectOutcome::phase(
                SharedProviderEffectPhase::Pending,
            ));
        }
        match component {
            SecurityKeyComponent::Service => {
                let runtime = self.runtime()?;
                let mode = SharedProviderEffectMode::parse(request)?;
                if mode == SharedProviderEffectMode::Projection {
                    let endpoint_ref = request
                        .status
                        .as_ref()
                        .and_then(|status| status.get("relayEndpointRef"))
                        .or_else(|| {
                            request
                                .status
                                .as_ref()
                                .and_then(|status| status.get("details"))
                                .and_then(|details| details.get("relayEndpointRef"))
                        })
                        .and_then(Value::as_str)
                        .and_then(|value| ResourceRef::parse(value).ok())
                        .ok_or(SharedProviderEffectError::Unavailable)?;
                    let endpoint = self
                        .resource_value(&endpoint_ref)
                        .await?
                        .ok_or(SharedProviderEffectError::Unavailable)?;
                    return Ok(SharedProviderEffectOutcome::projection(
                        if endpoint.pointer("/status/phase").and_then(Value::as_str) == Some("Ready")
                        {
                            SharedProviderEffectPhase::Ready
                        } else {
                            SharedProviderEffectPhase::Pending
                        },
                        request.status.clone().unwrap_or_else(|| json!({})),
                    ));
                }
                let settings = request
                    .spec
                    .pointer("/provider/settings")
                    .ok_or(SharedProviderEffectError::InvalidResource)?;
                let device_ref = settings
                    .get("deviceRef")
                    .and_then(Value::as_str)
                    .and_then(|value| ResourceRef::parse(value).ok())
                    .ok_or(SharedProviderEffectError::InvalidResource)?;
                let relay_endpoint_ref = settings
                    .get("relayEndpointRef")
                    .and_then(Value::as_str)
                    .and_then(|value| ResourceRef::parse(value).ok())
                    .ok_or(SharedProviderEffectError::InvalidResource)?;
                if device_ref.resource_type().as_str() != "Device"
                    || relay_endpoint_ref.resource_type().as_str() != "Endpoint"
                {
                    return Err(SharedProviderEffectError::InvalidResource);
                }
                let device = self
                    .resource_value(&device_ref)
                    .await?
                    .ok_or(SharedProviderEffectError::Unavailable)?;
                if device.pointer("/spec/providerRef").and_then(Value::as_str)
                    != Some(d2b_provider_device_security_key::PROVIDER_REF)
                {
                    return Err(SharedProviderEffectError::InvalidResource);
                }
                if device.pointer("/status/phase").and_then(Value::as_str) != Some("Ready") {
                    return Ok(SharedProviderEffectOutcome::phase(
                        SharedProviderEffectPhase::Pending,
                    ));
                }
                let device_uid = device
                    .pointer("/metadata/uid")
                    .and_then(Value::as_str)
                    .and_then(|value| ResourceUid::parse(value.to_owned()).ok())
                    .ok_or(SharedProviderEffectError::InvalidResource)?;
                let relay_process_name =
                    d2b_provider_device_security_key::security_key_process_name(
                        &device_uid,
                        d2b_provider_device_security_key::SecurityKeyProcessRole::HostRelay,
                    )
                    .map_err(|_| SharedProviderEffectError::InvalidResource)?;
                let relay_process_ref =
                    ResourceRef::parse(&format!("Process/{relay_process_name}"))
                        .map_err(|_| SharedProviderEffectError::InvalidResource)?;
                let process = self
                    .resource_value(&relay_process_ref)
                    .await?
                    .ok_or(SharedProviderEffectError::Unavailable)?;
                let endpoint = self
                    .resource_value(&relay_endpoint_ref)
                    .await?
                    .ok_or(SharedProviderEffectError::Unavailable)?;
                let ready =
                    process.pointer("/status/phase").and_then(Value::as_str) == Some("Ready")
                        && endpoint.pointer("/status/phase").and_then(Value::as_str)
                            == Some("Ready");
                let _ = runtime;
                Ok(SharedProviderEffectOutcome::phase(if ready {
                    SharedProviderEffectPhase::Ready
                } else {
                    SharedProviderEffectPhase::Pending
                }))
            }
            SecurityKeyComponent::Binding => {
                let service_ref = ResourceRef::parse(
                    request
                        .spec
                        .pointer("/serviceRef")
                        .and_then(Value::as_str)
                        .ok_or(SharedProviderEffectError::InvalidResource)?,
                )
                .map_err(|_| SharedProviderEffectError::InvalidResource)?;
                let target_ref = request
                    .spec
                    .pointer("/target/guestRef")
                    .or_else(|| request.spec.pointer("/guestRef"))
                    .and_then(Value::as_str)
                    .and_then(|value| ResourceRef::parse(value).ok())
                    .ok_or(SharedProviderEffectError::InvalidResource)?;
                let service = self
                    .resource_value(&service_ref)
                    .await?
                    .ok_or(SharedProviderEffectError::Unavailable)?;
                if service.pointer("/spec/providerRef").and_then(Value::as_str)
                    != Some(d2b_provider_device_security_key::PROVIDER_REF)
                {
                    return Err(SharedProviderEffectError::InvalidResource);
                }
                if service.pointer("/status/phase").and_then(Value::as_str) != Some("Ready") {
                    return Ok(SharedProviderEffectOutcome::phase(
                        SharedProviderEffectPhase::Pending,
                    ));
                }
                let guest = self
                    .resource_value(&target_ref)
                    .await?
                    .ok_or(SharedProviderEffectError::Unavailable)?;
                if guest.pointer("/status/phase").and_then(Value::as_str) != Some("Ready") {
                    return Ok(SharedProviderEffectOutcome::phase(
                        SharedProviderEffectPhase::Pending,
                    ));
                }
                let desired = if let Some(user_ref) = request
                    .spec
                    .pointer("/target/userRef")
                    .or_else(|| request.spec.pointer("/userRef"))
                    .and_then(Value::as_str)
                    .and_then(|value| ResourceRef::parse(value).ok())
                {
                    d2b_provider_device_security_key::SecurityKeyController::child_resources_for_user(
                        &key_ref(&request.target),
                        &service_ref,
                        &target_ref,
                        &user_ref,
                    )
                } else {
                    d2b_provider_device_security_key::SecurityKeyController::child_resources(
                        &key_ref(&request.target),
                        &service_ref,
                        &target_ref,
                    )
                }
                .map_err(|_| SharedProviderEffectError::InvalidResource)?;
                let ready = self
                    .child_set_ready(&desired, &request.target, &request.zone)
                    .await?;
                Ok(SharedProviderEffectOutcome::phase(if ready {
                    SharedProviderEffectPhase::Ready
                } else {
                    SharedProviderEffectPhase::Pending
                }))
            }
        }
    }

    /// Reconcile one GPU Device through the authority-fenced lifecycle.
    #[allow(clippy::disallowed_methods, reason = "synchronous path")]
    async fn reconcile_gpu(
        &self,
        request: &SharedProviderEffectRequest<'_>,
        state: &DeviceResourceState,
    ) -> Result<SharedProviderEffectOutcome, SharedProviderEffectError> {
        if !self
            .dependencies_ready(SharedProviderKind::GpuDevice, request)
            .await?
        {
            return Ok(SharedProviderEffectOutcome::phase(
                SharedProviderEffectPhase::Pending,
            ));
        }
        let (_runtime, admission, tokens, settings, holder_ref) = self.gpu_admission(request).await?;
        let mut controllers = state.gpu_controllers
            .lock() // async-gate-allow: synchronous lock acquisition, no await while the guard is held
            .map_err(|_| SharedProviderEffectError::Unavailable)?;
        let mut controller = match controllers.remove(&request.uid) {
            Some(controller) => controller,
            None => d2b_provider_device_gpu::GpuController::new_authorized(
                admission.clone(),
                settings.clone(),
                tokens.clone(),
            )
            .map_err(|_| SharedProviderEffectError::InvalidResource)?,
        };
        if controller
            .admission()
            .is_some_and(|current| current != &admission)
        {
            controllers.insert(request.uid.clone(), controller);
            return Err(SharedProviderEffectError::InvalidResource);
        }
        // U12 gpu step: the lifecycle port is the GPU crate's own
        // implementation, built from the daemon-supplied facet set and the
        // driver's per-resource lease cache.
        let gpu_facets = self
            .gpu_facets
            .get()
            .cloned()
            .ok_or(SharedProviderEffectError::Unavailable)?;
        let mut port = d2b_provider_device_gpu::effects_service::DeclaredWorkerGpuPort::new(
            d2b_provider_device_gpu::effects_service::DeclaredWorkerGpuPortArgs::new(
                d2b_provider_device_gpu::effects_service::DeclaredWorkerGpuPortDeps::new(
                    Arc::clone(&gpu_facets.runtime),
                    Arc::clone(&state.gpu_authority_leases),
                    tokio::runtime::Handle::current(),
                    request.children,
                ),
                self.zone.as_str().to_owned(),
                key_ref(&request.target).clone(),
                request.uid.clone(),
                holder_ref,
                request.generation,
                request.operation_id.clone(),
            ),
        );
        let result = controller
            .reconcile_lifecycle(&mut port)
            .map_err(|_| SharedProviderEffectError::Unavailable)
            .map(|outcome| match outcome {
                d2b_provider_device_gpu::GpuReconcileOutcome::Converged => {
                    SharedProviderEffectPhase::Ready
                }
                d2b_provider_device_gpu::GpuReconcileOutcome::Retry => {
                    SharedProviderEffectPhase::Pending
                }
            });
        controllers.insert(request.uid.clone(), controller);
        result.map(SharedProviderEffectOutcome::phase)
    }

    /// The Network row's teardown stage (the staged fabric finalizer).
    async fn finalize_network_row(
        &self,
        request: &SharedProviderEffectRequest<'_>,
    ) -> Result<SharedProviderFinalize, SharedProviderEffectError> {
        self.finalize_network(request).await
    }

    /// The USBIP Service row's teardown stage.
    async fn finalize_usbip_service_row(
        &self,
        request: &SharedProviderEffectRequest<'_>,
    ) -> Result<SharedProviderFinalize, SharedProviderEffectError> {
        self.finalize_usbip_service(request).await
    }

    /// The USBIP Binding row's teardown stage.
    async fn finalize_usbip_binding(
        &self,
        request: &SharedProviderEffectRequest<'_>,
    ) -> Result<SharedProviderFinalize, SharedProviderEffectError> {
        let service_ref = ResourceRef::parse(
            request
                .spec
                .pointer("/serviceRef")
                .and_then(Value::as_str)
                .ok_or(SharedProviderEffectError::InvalidResource)?,
        )
        .map_err(|_| SharedProviderEffectError::InvalidResource)?;
        let guest_ref = ResourceRef::parse(
            request
                .spec
                .pointer("/guestRef")
                .and_then(Value::as_str)
                .ok_or(SharedProviderEffectError::InvalidResource)?,
        )
        .map_err(|_| SharedProviderEffectError::InvalidResource)?;
        let mut controller = d2b_provider_device_usbip::UsbipBindingController::new(
            &key_ref(&request.target),
            &service_ref,
            &guest_ref,
        )
        .map_err(|_| SharedProviderEffectError::InvalidResource)?;
        controller.finalize();
        Ok(SharedProviderFinalize::Complete)
    }

    /// The USBIP Device row's teardown stage: no Service may still reference
    /// the backing Device.
    async fn finalize_usbip_device(
        &self,
        request: &SharedProviderEffectRequest<'_>,
    ) -> Result<SharedProviderFinalize, SharedProviderEffectError> {
        let device_ref = key_ref(&request.target).to_canonical_string();
        let runtime = self.runtime()?;
        let children = runtime
            .committed_resources_of_type(d2b_provider_device_usbip::USB_SERVICE_RESOURCE_TYPE)
            .await
            .map_err(|_| SharedProviderEffectError::Unavailable)?;
        if children.iter().any(|child| {
            child
                .pointer("/spec/backingDeviceRef")
                .and_then(Value::as_str)
                == Some(device_ref.as_str())
        }) {
            return Ok(SharedProviderFinalize::Pending);
        }
        Ok(SharedProviderFinalize::Complete)
    }

    /// The security-key Service row's teardown stage: no Binding may still
    /// reference it.
    async fn finalize_security_key_service(
        &self,
        request: &SharedProviderEffectRequest<'_>,
    ) -> Result<SharedProviderFinalize, SharedProviderEffectError> {
        let runtime = self.runtime()?;
        let service_ref = key_ref(&request.target).to_canonical_string();
        let bindings = runtime
            .committed_resources_of_type(
                d2b_provider_device_security_key::SECURITY_KEY_BINDING_RESOURCE_TYPE,
            )
            .await
            .map_err(|_| SharedProviderEffectError::Unavailable)?;
        if bindings.iter().any(|binding| {
            binding.pointer("/spec/serviceRef").and_then(Value::as_str) == Some(service_ref.as_str())
        }) {
            return Ok(SharedProviderFinalize::Pending);
        }
        Ok(SharedProviderFinalize::Complete)
    }

    /// The security-key Device row's teardown stage: no Service may still
    /// reference the Device.
    async fn finalize_security_key_device(
        &self,
        request: &SharedProviderEffectRequest<'_>,
    ) -> Result<SharedProviderFinalize, SharedProviderEffectError> {
        let runtime = self.runtime()?;
        let device_ref = key_ref(&request.target).to_canonical_string();
        let services = runtime
            .committed_resources_of_type(
                d2b_provider_device_security_key::SECURITY_KEY_SERVICE_RESOURCE_TYPE,
            )
            .await
            .map_err(|_| SharedProviderEffectError::Unavailable)?;
        if services.iter().any(|service| {
            service
                .pointer("/spec/provider/settings/deviceRef")
                .and_then(Value::as_str)
                == Some(device_ref.as_str())
                || service
                    .pointer("/metadata/ownerRef")
                    .and_then(Value::as_str)
                    == Some(device_ref.as_str())
        }) {
            return Ok(SharedProviderFinalize::Pending);
        }
        Ok(SharedProviderFinalize::Complete)
    }

    /// The security-key Binding row's teardown stage.
    async fn finalize_security_key_binding(
        &self,
        _request: &SharedProviderEffectRequest<'_>,
    ) -> Result<SharedProviderFinalize, SharedProviderEffectError> {
        Ok(SharedProviderFinalize::Complete)
    }

    /// The TPM Device row's teardown stage.
    async fn finalize_tpm_row(
        &self,
        request: &SharedProviderEffectRequest<'_>,
        state: &DeviceResourceState,
    ) -> Result<SharedProviderFinalize, SharedProviderEffectError> {
        self.finalize_tpm(request, state).await
    }

    /// The GPU Device row's teardown stage.
    async fn finalize_gpu_row(
        &self,
        request: &SharedProviderEffectRequest<'_>,
        state: &DeviceResourceState,
    ) -> Result<SharedProviderFinalize, SharedProviderEffectError> {
        self.finalize_gpu(request, state).await
    }
}

impl ProductionSharedProviderEffects {
    /// The children of one Provider-declared Binding set, live-Ready?
    async fn child_set_ready(
        &self,
        desired: &d2b_contracts_provider::v3::semantic_services::child_resources::BindingChildSet,
        owner: &ResourceKey,
        zone: &ZoneId,
    ) -> Result<bool, SharedProviderEffectError> {
        for intent in desired.iter() {
            if *intent.owner_ref() != key_ref(owner) || zone.as_str() != self.zone.as_str() {
                return Err(SharedProviderEffectError::InvalidResource);
            }
            if !self.resource_ready(intent.resource_ref()).await {
                return Ok(false);
            }
        }
        Ok(true)
    }

    async fn finalize_network(
        &self,
        request: &SharedProviderEffectRequest<'_>,
    ) -> Result<SharedProviderFinalize, SharedProviderEffectError> {
        let spec = self.network_spec(request)?;
        // Per-invocation freshness (the retired adapter reloaded the trusted
        // bundle on every finalize): the worker-side load re-verifies every
        // artifact hash, so a replaced bundle is torn down under its new
        // generation without a daemon restart, and a missing, tampered, or
        // unreadable bundle fails this finalize closed.
        let resolver = crate::load_bundle_resolver_on_worker(&self.state)
            .await
            .map_err(|_| SharedProviderEffectError::Unavailable)?;
        let runtime = self.runtime()?;
        let admission = self
            .network_admission(&runtime, request, &spec, &resolver)
            .await?;
        let fence = self
            .network_content_fence(SharedProviderKind::Network, &runtime, request, &admission)
            .await?;
        let owner_ref = key_ref(&request.target).clone();
        let children = NetworkChildPort::new(self, request, owner_ref, request.uid.clone(), fence);
        let volume = children
            .current(&children.volume_ref)
            .await
            .map_err(|_| SharedProviderEffectError::Unavailable)?;
        let guest = children
            .current(&children.guest_ref)
            .await
            .map_err(|_| SharedProviderEffectError::Unavailable)?;
        let agent = children
            .current(&children.agent_ref)
            .await
            .map_err(|_| SharedProviderEffectError::Unavailable)?;
        let volume_phase = self
            .live_phase(&children.volume_ref)
            .await
            .map_err(|_| SharedProviderEffectError::Unavailable)?;
        let guest_phase = self
            .live_phase(&children.guest_ref)
            .await
            .map_err(|_| SharedProviderEffectError::Unavailable)?;
        let agent_phase = self
            .live_phase(&children.agent_ref)
            .await
            .map_err(|_| SharedProviderEffectError::Unavailable)?;
        let volume_attachment_removed = volume.as_ref().is_none_or(|value| {
            value
                .pointer("/spec/attachments")
                .and_then(Value::as_array)
                .is_none_or(Vec::is_empty)
        });
        let mdns_enabled = request
            .spec
            .pointer("/mdns/enable")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if mdns_enabled {
            return Err(SharedProviderEffectError::Unavailable);
        }
        let broker_context = crate::resolve_network_effect_context(
            &Self::envelope(request),
            &resolver,
            &admission,
        )
        .map_err(|_| SharedProviderEffectError::Unavailable)?
        .with_host_global_nic_admission();
        // U14: the kernel-invoking adapter is the declaring crate's own
        // `KernelNetworkBroker`, built from the daemon-supplied facets.
        let effects = d2b_provider_network_local::broker::BrokerNetworkEffectPort::new(
            d2b_provider_network_local::broker::KernelNetworkBroker::new(
                d2b_provider_network_local::broker::NetworkBrokerFacets::new(
                    crate::broker_socket_path(&self.state),
                    BrokerCallerRole::AdminUid {
                        uid: self.state.daemon_uid,
                    },
                    Arc::clone(&self.intents),
                ),
            ),
            broker_context,
        );
        let input = ReconcileInput {
            spec,
            mdns_enabled,
            network_uid: request.uid.clone(),
            network_generation: request.generation,
            attachment_generation: admission.key().attachment_generation(),
            installed_generation: admission.key().bundle_generation().clone(),
            admission,
            artifact_catalog: Vec::new(),
            user_ready: true,
            host_memory_budget_available:
                d2b_provider_network_local::controller::CONFIG_VOLUME_MAX_BYTES,
            volume_ready: volume_phase == Some("Ready"),
            guest_ready: guest_phase == Some("Ready"),
            volume_attachment_ready: volume_phase == Some("Ready"),
            workload_fds_closed: true,
            agent_deleted: agent.is_none() || agent_phase == Some("Deleted"),
            mdns_deleted: true,
            volume_attachment_removed,
            guest_deleted: guest.is_none() || guest_phase == Some("Deleted"),
            volume_deleted: volume.is_none() || volume_phase == Some("Deleted"),
            attachments: Vec::<AttachmentRealization>::new(),
        };
        let stage = NetworkReconciler::new(effects, children)
            .finalize(&input)
            .await
            .map_err(|_| SharedProviderEffectError::Unavailable)?;
        if stage != d2b_provider_network_local::controller::FinalizerStage::Complete {
            return Ok(SharedProviderFinalize::Pending);
        }
        let zone_uid = runtime.authority_zone_uid().cloned();
        let plane = self
            .state
            .resource_plane
            .lock()
            .await
            .as_ref()
            .map(Arc::clone);
        if let (Some(zone_uid), Some(plane)) = (zone_uid, plane) {
            plane
                .network_admission_index()
                .lock()
                .await
                .release_owner_after_finalizer(&zone_uid, &request.uid, true);
        }
        Ok(SharedProviderFinalize::Complete)
    }

    #[allow(clippy::disallowed_methods, reason = "synchronous path")]
    async fn finalize_tpm(
        &self,
        request: &SharedProviderEffectRequest<'_>,
        state: &DeviceResourceState,
    ) -> Result<SharedProviderFinalize, SharedProviderEffectError> {
        let holder = request.owner_ref()?;
        if holder.resource_type().as_str() != "Guest" {
            return Err(SharedProviderEffectError::InvalidResource);
        }
        let execution_ref = request
            .spec
            .pointer("/provider/settings/executionRef")
            .and_then(Value::as_str)
            .and_then(|value| ResourceRef::parse(value).ok())
            .unwrap_or_else(|| ResourceRef::parse(HOST_REF).expect("Host ref"));
        let runtime = self.runtime()?;
        let vm_id = VmId::new(holder.name().as_str());
        let migration_intent = BundleOpId::new(format!(
            "{}{}",
            d2b_provider_device_tpm::vocabulary::TPM_LEGACY_MIGRATION_INTENT_PREFIX,
            vm_id.as_str()
        ));
        let decision = runtime
            .tpm_device_is_admitted(
                &request.uid,
                &key_ref(&request.target),
                vm_id.as_str(),
                &request.operation_id,
                None,
            )
            .await
            .map_err(|_| SharedProviderEffectError::Unavailable)?;
        let mut controller = {
            let mut controllers = state
                .tpm_controllers
                .lock() // async-gate-allow: synchronous lock acquisition, no await while the guard is held
                .map_err(|_| SharedProviderEffectError::Unavailable)?;
            controllers
                .remove(&request.uid)
                .ok_or(SharedProviderEffectError::Unavailable)?
        };
        let tpm_facets = self
            .tpm_facets
            .get()
            .cloned()
            .ok_or(SharedProviderEffectError::Unavailable)?;
        let result = d2b_provider_device_tpm::effects_service::finalize_device_tpm_controller(
            tpm_facets,
            vm_id.clone(),
            migration_intent,
            decision,
            d2b_provider_device_tpm::effects_service::AdmittedTpmDevice::from_row(
                request.uid.clone(),
                key_ref(&request.target).clone(),
                self.zone.as_str(),
                execution_ref,
                request.operation_id.clone(),
            ),
            request.children,
            &mut controller,
        )
        .await;
        match result {
            Ok(_) => Ok(SharedProviderFinalize::Complete),
            Err(error) => {
                {
                    let mut controllers = state
                        .tpm_controllers
                        .lock() // async-gate-allow: synchronous lock acquisition, no await while the guard is held
                        .map_err(|_| SharedProviderEffectError::Unavailable)?;
                    controllers.insert(request.uid.clone(), controller);
                }
                tracing::warn!(
                    error = ?error,
                    device = %key_ref(&request.target).to_canonical_string(),
                    "TPM device controller finalize failed",
                );
                Err(SharedProviderEffectError::Unavailable)
            }
        }
    }

    async fn finalize_usbip_service(
        &self,
        request: &SharedProviderEffectRequest<'_>,
    ) -> Result<SharedProviderFinalize, SharedProviderEffectError> {
        let runtime = self.runtime()?;
        let service_ref = key_ref(&request.target).to_canonical_string();
        let bindings = runtime
            .committed_resources_of_type(d2b_provider_device_usbip::USB_BINDING_RESOURCE_TYPE)
            .await
            .map_err(|_| SharedProviderEffectError::Unavailable)?;
        if bindings.iter().any(|binding| {
            binding.pointer("/spec/serviceRef").and_then(Value::as_str)
                == Some(service_ref.as_str())
        }) {
            return Ok(SharedProviderFinalize::Pending);
        }
        let (zone_uid, opted_in, mut port) = self.usbip_service_port(&runtime, request).await?;
        if !opted_in {
            return Ok(SharedProviderFinalize::Complete);
        }
        let mut lifecycle = d2b_provider_device_usbip::ServiceLifecycle::new(
            zone_uid.clone(),
            request.uid.clone(),
        );
        lifecycle
            .activate(true, zone_uid, &mut port)
            .map_err(|_| SharedProviderEffectError::Unavailable)?;
        let mut supervisor = d2b_provider_device_usbip::UsbipSupervisor::new(lifecycle);
        supervisor
            .finalize(&mut port)
            .map_err(|_| SharedProviderEffectError::Unavailable)?;
        self.usbip_services.lock().await.remove(&request.uid);
        Ok(SharedProviderFinalize::Complete)
    }

    #[allow(clippy::disallowed_methods, reason = "synchronous path")]
    async fn finalize_gpu(
        &self,
        request: &SharedProviderEffectRequest<'_>,
        state: &DeviceResourceState,
    ) -> Result<SharedProviderFinalize, SharedProviderEffectError> {
        let mut controllers = state.gpu_controllers
            .lock() // async-gate-allow: synchronous lock acquisition, no await while the guard is held
            .map_err(|_| SharedProviderEffectError::Unavailable)?;
        let admission = controllers
            .get(&request.uid)
            .and_then(|controller| controller.admission().cloned())
            .ok_or(SharedProviderEffectError::Unavailable)?;
        let mut controller = controllers
            .remove(&request.uid)
            .ok_or(SharedProviderEffectError::Unavailable)?;
        let gpu_facets = self
            .gpu_facets
            .get()
            .cloned()
            .ok_or(SharedProviderEffectError::Unavailable)?;
        let mut port = d2b_provider_device_gpu::effects_service::DeclaredWorkerGpuPort::new(
            d2b_provider_device_gpu::effects_service::DeclaredWorkerGpuPortArgs::new(
                d2b_provider_device_gpu::effects_service::DeclaredWorkerGpuPortDeps::new(
                    Arc::clone(&gpu_facets.runtime),
                    Arc::clone(&state.gpu_authority_leases),
                    tokio::runtime::Handle::current(),
                    request.children,
                ),
                self.zone.as_str().to_owned(),
                key_ref(&request.target).clone(),
                request.uid.clone(),
                admission.owner().holder_ref().clone(),
                admission.owner().generation(),
                request.operation_id.clone(),
            ),
        );
        let result = controller.finalize_lifecycle(&mut port).map_err(|error| {
            tracing::debug!(
                error = ?error,
                device = %key_ref(&request.target).to_canonical_string(),
                "GPU lifecycle finalize failed",
            );
            SharedProviderEffectError::Unavailable
        });
        match result {
            Ok(()) => Ok(SharedProviderFinalize::Complete),
            Err(error) => {
                controllers.insert(request.uid.clone(), controller);
                Err(error)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Family effect ports: the daemon implements each declared family's port; the
// drivers live in the family crates.
// ---------------------------------------------------------------------------

// U14: the Network family's driver effects are no longer a daemon port. The
// daemon supplies the declared facet implementation the family's own
// effects service delegates to: the reconcile/finalize orchestration over
// the daemon's admission, child rows, and readiness state, plus the
// daemon-resolved bundle intents ([`NetworkIntentSource`]).
#[async_trait]
impl d2b_provider_network_local::NetworkRuntime for ProductionSharedProviderEffects {
    fn bundle(&self) -> Arc<d2b_core::bundle_resolver::BundleResolver> {
        // Per-invocation freshness, mirroring the retired adapter's per-call
        // reload: re-verify the on-disk bundle before serving any bundle
        // fact, so a replaced bundle is observed without a daemon restart.
        // The owned `Arc` keeps the served resolver valid for the caller's
        // synchronous read even when a later invocation refreshes the slot.
        // A bundle that fails verification keeps the last verified resolver
        // (an unreadable bundle never mints facts), while the reconcile,
        // finalize, and kernel paths refuse closed.
        let mut slot = loop {
            match self.bundle.try_lock() {
                Ok(guard) => break guard,
                Err(_) => std::hint::spin_loop(),
            }
        };
        if let Ok(resolver) = crate::load_bundle_resolver(&self.state) {
            *slot = Arc::new(resolver);
        }
        Arc::clone(&slot)
    }

    fn broker_socket_path(&self) -> &std::path::Path {
        &self.broker_socket
    }

    fn caller_role(&self) -> d2b_contracts_broker::broker_wire::BrokerCallerRole {
        BrokerCallerRole::AdminUid {
            uid: self.state.daemon_uid,
        }
    }

    async fn reconcile_network(
        &self,
        request: &SharedProviderEffectRequest<'_>,
    ) -> Result<SharedProviderEffectOutcome, SharedProviderEffectError> {
        ProductionSharedProviderEffects::reconcile_network(self, request).await
    }

    async fn finalize_network(
        &self,
        request: &SharedProviderEffectRequest<'_>,
    ) -> Result<SharedProviderFinalize, SharedProviderEffectError> {
        self.finalize_network_row(request).await
    }
}

// U12 (device families): the device families' driver effects are no longer
// daemon ports. Each family's effects service lives in its declaring crate
// and delegates to the runtime trait this adapter implements: the reconcile
// and finalize orchestration over the daemon's admission, child rows, and
// readiness state stays daemon-hosted, and the per-component drives (the
// TPM controller over the TPM crate's port, the authority-fenced GPU
// lifecycle over the GPU crate's port, the USBIP kernel dispatcher) run
// inside the Device and USBIP runtimes over those crates' declared facets.
#[async_trait]
impl UsbipRuntime for ProductionSharedProviderEffects {
    async fn reconcile_usbip(
        &self,
        component: UsbipComponent,
        request: &SharedProviderEffectRequest<'_>,
    ) -> Result<SharedProviderEffectOutcome, SharedProviderEffectError> {
        ProductionSharedProviderEffects::reconcile_usbip(self, component, request).await
    }

    async fn finalize(
        &self,
        component: UsbipComponent,
        request: &SharedProviderEffectRequest<'_>,
    ) -> Result<SharedProviderFinalize, SharedProviderEffectError> {
        match component {
            UsbipComponent::Service => self.finalize_usbip_service_row(request).await,
            UsbipComponent::Binding => self.finalize_usbip_binding(request).await,
        }
    }
}

#[async_trait]
impl d2b_provider_device_security_key::facets::SecurityKeyRuntime
    for ProductionSharedProviderEffects
{
    async fn reconcile_security_key(
        &self,
        component: SecurityKeyComponent,
        request: &SharedProviderEffectRequest<'_>,
    ) -> Result<SharedProviderEffectOutcome, SharedProviderEffectError> {
        ProductionSharedProviderEffects::reconcile_security_key(self, component, request).await
    }

    async fn finalize(
        &self,
        component: SecurityKeyComponent,
        request: &SharedProviderEffectRequest<'_>,
    ) -> Result<SharedProviderFinalize, SharedProviderEffectError> {
        match component {
            SecurityKeyComponent::Service => self.finalize_security_key_service(request).await,
            SecurityKeyComponent::Binding => self.finalize_security_key_binding(request).await,
        }
    }
}

#[async_trait]
impl d2b_provider_device::facets::DeviceRuntime for ProductionSharedProviderEffects {
    async fn reconcile_device(
        &self,
        component: DeviceComponent,
        request: &SharedProviderEffectRequest<'_>,
        state: &DeviceResourceState,
    ) -> Result<SharedProviderEffectOutcome, SharedProviderEffectError> {
        match component {
            DeviceComponent::Tpm => self.reconcile_tpm(request, state).await,
            DeviceComponent::Usbip => self.reconcile_usbip_device(request).await,
            DeviceComponent::SecurityKey => self.reconcile_security_key_device(request).await,
            DeviceComponent::Gpu => self.reconcile_gpu(request, state).await,
        }
    }

    async fn finalize_device(
        &self,
        component: DeviceComponent,
        request: &SharedProviderEffectRequest<'_>,
        state: &DeviceResourceState,
    ) -> Result<SharedProviderFinalize, SharedProviderEffectError> {
        match component {
            DeviceComponent::Tpm => self.finalize_tpm_row(request, state).await,
            DeviceComponent::Usbip => self.finalize_usbip_device(request).await,
            DeviceComponent::SecurityKey => self.finalize_security_key_device(request).await,
            DeviceComponent::Gpu => self.finalize_gpu_row(request, state).await,
        }
    }
}

// The TPM runtime facet (U12 tpm step): the TPM crate's port resolves its
// daemon-state reads through this adapter.
#[async_trait]
impl TpmRuntime for ProductionSharedProviderEffects {
    fn broker_socket_path(&self) -> &std::path::Path {
        &self.broker_socket
    }

    fn caller_role(&self) -> BrokerCallerRole {
        BrokerCallerRole::AdminUid {
            uid: self.state.daemon_uid,
        }
    }

    fn kernel_io_timeout(&self) -> std::time::Duration {
        crate::KERNEL_IO_TIMEOUT
    }

    async fn load_bundle(
        &self,
    ) -> Result<Arc<BundleResolver>, d2b_provider_device_tpm::TpmResourceEffectError> {
        crate::load_bundle_resolver_on_worker(&self.state)
            .await
            .map(Arc::new)
            .map_err(|_| d2b_provider_device_tpm::TpmResourceEffectError::Transient)
    }

    async fn consume_lifecycle_lease(
        &self,
        vm_id: &str,
        operation_id: &str,
    ) -> Result<(), d2b_provider_device_tpm::TpmResourceEffectError> {
        use crate::provider_effects::{
            GuestLifecycleOperation, LifecycleAuthorization, ProviderEffectError,
        };
        let zone = ZoneId::parse(self.zone.as_str())
            .map_err(|_| d2b_provider_device_tpm::TpmResourceEffectError::InvalidDevice)?;
        // Non-blocking `try_lock` per plan U4: a collision reports Transient
        // (fail-closed), never a stall.
        let runtime = self
            .state
            .resource_plane
            .try_lock()
            .ok()
            .and_then(|plane| plane.as_ref().and_then(|plane| plane.zone(&zone).ok()))
            .ok_or(d2b_provider_device_tpm::TpmResourceEffectError::Transient)?;
        let guest_ref = ResourceRef::parse(&format!("Guest/{vm_id}"))
            .map_err(|_| d2b_provider_device_tpm::TpmResourceEffectError::InvalidDevice)?;
        let admission = runtime
            .admit_internal_guest_lifecycle(guest_ref.clone(), operation_id)
            .await
            .map_err(|_| d2b_provider_device_tpm::TpmResourceEffectError::Transient)?;
        let authorization = LifecycleAuthorization::from_lease(
            admission.lease,
            guest_ref,
            admission.guest_uid,
            admission.guest_generation,
            admission.provider_assignment_generation,
        )
        .map_err(|_| d2b_provider_device_tpm::TpmResourceEffectError::StateIntegrity)?;
        crate::consume_lifecycle_lease(
            &self.state,
            &authorization,
            GuestLifecycleOperation::Start,
            &BrokerCallerRole::AdminUid {
                uid: self.state.daemon_uid,
            },
        )
        .map_err(|error: ProviderEffectError| match error {
            ProviderEffectError::StateUnavailable
            | ProviderEffectError::MutationPending
            | ProviderEffectError::MutationTableFull => {
                d2b_provider_device_tpm::TpmResourceEffectError::Transient
            }
            _ => d2b_provider_device_tpm::TpmResourceEffectError::EffectRejected,
        })
    }
}

// The GPU runtime facet (U12 gpu step): the GPU crate's port admits and
// releases its Host-global authority leases through this adapter.
impl GpuRuntime for ProductionSharedProviderEffects {
    #[allow(clippy::disallowed_methods, reason = "synchronous path")]
    fn admit_authority(
        &self,
        request: AuthorityRequest,
    ) -> Result<d2b_core_controller::authority::AuthorityLease, d2b_provider_device_gpu::GpuEffectError>
    {
        let runtime = self
            .runtime()
            .map_err(|_| d2b_provider_device_gpu::GpuEffectError::Transient)?;
        crate::drive_sync(&tokio::runtime::Handle::current(), async {
            runtime
                .authority_index()
                .lock()
                .await
                .admit_authority(request)
        })
        .map_err(|_| d2b_provider_device_gpu::GpuEffectError::AuthorityConflict)
    }

    #[allow(clippy::disallowed_methods, reason = "synchronous path")]
    fn release_authority(
        &self,
        lease: &d2b_core_controller::authority::AuthorityLease,
    ) -> Result<(), d2b_provider_device_gpu::GpuEffectError> {
        let runtime = self
            .runtime()
            .map_err(|_| d2b_provider_device_gpu::GpuEffectError::Transient)?;
        crate::drive_sync(&tokio::runtime::Handle::current(), async {
            runtime
                .authority_index()
                .lock()
                .await
                .release_authority(lease)
        })
        .map_err(|_| d2b_provider_device_gpu::GpuEffectError::AuthorityConflict)
    }
}

#[cfg(test)]
mod tests {
    use d2b_resource_runtime::error::{DriverFailure, DriverOp};
    use d2b_resource_runtime::identity::{ResourceKey, ResourceProvenance};
    use d2b_resource_runtime::manager::ResourceView;
    use d2b_resource_runtime::resource::ResourceStatus;

    use super::*;

    use super::view_phase;

    /// One manager view carrying the given status classification, published
    /// generation, and durable deleting mark.
    fn phase_view(
        status: Option<ResourceStatus>,
        status_generation: Option<u64>,
        deleting: bool,
    ) -> ResourceView {
        ResourceView {
            key: ResourceKey::new("work", "Volume", "phase-view"),
            uid: [0x42; 16],
            generation: 2,
            deleting,
            provenance: ResourceProvenance::Api,
            spec: b"{}".to_vec(),
            metadata: b"{}".to_vec(),
            owner_key: None,
            status,
            status_generation,
            status_projection: None,
        }
    }

    /// Issue #515: `view_phase` delegates to the canonical wire producer.
    /// Across the closed status vocabulary - and across the runtime flags
    /// (the durable deleting mark, an ungenerationed status, a stale
    /// generation) - the gate's phase equals the phase
    /// `ResourceView::wire_status` serves, so a future divergence fails here.
    #[test]
    fn view_phase_delegates_to_the_canonical_wire_phase() {
        let statuses = [
            None,
            Some(ResourceStatus::Pending),
            Some(ResourceStatus::Recovering),
            Some(ResourceStatus::Reconciling),
            Some(ResourceStatus::Ready),
            Some(ResourceStatus::Failed(DriverFailure::retryable(DriverOp::Reconcile))),
            Some(ResourceStatus::Failed(DriverFailure::terminal(DriverOp::Delete))),
            Some(ResourceStatus::Deleting),
        ];
        for deleting in [false, true] {
            for status in &statuses {
                for status_generation in [None, Some(1), Some(2), Some(3)] {
                    let view = phase_view(status.clone(), status_generation, deleting);
                    let canonical = view.wire_status()["phase"]
                        .as_str()
                        .expect("the canonical status always carries a phase")
                        .to_owned();
                    assert_eq!(
                        view_phase(&view),
                        canonical,
                        "phase divergence: status={status:?} status_generation={status_generation:?} deleting={deleting}",
                    );
                }
            }
        }
    }

    /// The Network runtime facet ([`NetworkRuntime::bundle`]) and the
    /// intent source re-verify the on-disk bundle on every invocation (the
    /// retired `network_effect_port`'s per-call reload), so a bundle
    /// generation change is observed without a daemon restart.
    ///
    /// Red observation (the snapshot behaviour this restores): the resolver
    /// captured at plane composition was frozen into the intent source and
    /// served by `bundle()`, so after the on-disk bundle was replaced both
    /// reads kept answering the old generation - the family reconciled
    /// against stale intents and the broker's generation fence refused
    /// every projection as stale until a daemon restart.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread")]
    async fn network_effects_observe_a_replaced_bundle_without_a_daemon_restart() {
        use d2b_provider_network_local::NetworkRuntime as _;

        let (state, _dir) = test_state_with_bundle("composed");
        let effects = ProductionSharedProviderEffects::new(
            Arc::new(state.clone()),
            ZoneId::parse("test").unwrap(),
            ControllerGeneration::new(1).unwrap(),
            d2b_core::bundle_resolver::BundleResolver::load_with_policy(
                &state.config.artifacts.bundle_path,
                &d2b_core::bundle_resolver::BundleVerifyPolicy::for_tests(),
            )
            .expect("load the composed bundle"),
        );

        let composed = effects
            .bundle()
            .installed_generation_identity()
            .expect("composed bundle generation")
            .as_str()
            .to_owned();
        let composed_fence = d2b_provider_network_local::broker::NetworkIntentSource::
            installed_generation_identity(&*effects.intents)
            .expect("composed generation fence");
        assert_eq!(
            composed_fence.as_str(),
            composed,
            "the runtime facet and the kernel generation fence resolve one bundle state"
        );

        // Replace the on-disk bundle (a new installed generation) without a
        // daemon restart, then observe the next invocations.
        write_v3_native_bundle(&state.config.artifacts.bundle_path, "replaced");
        let replaced = effects
            .bundle()
            .installed_generation_identity()
            .expect("replaced bundle generation")
            .as_str()
            .to_owned();
        let replaced_fence = d2b_provider_network_local::broker::NetworkIntentSource::
            installed_generation_identity(&*effects.intents)
            .expect("replaced generation fence");
        assert_ne!(
            composed, replaced,
            "the runtime facet re-verifies the on-disk bundle per invocation: \
             a bundle replacement is observed without a daemon restart (the \
             composition snapshot keeps answering the old generation)"
        );
        assert_eq!(
            replaced_fence.as_str(),
            replaced,
            "the kernel generation fence follows the replaced bundle"
        );
    }

    /// A bundle-problem refusal is fail-closed, but never silent:the
    /// daemon-supplied loader collapses the trusted bundle load into no
    /// intent, and the kernel broker maps that to its generic closed code,
    /// so the typed error that distinguishes a tampered bundle from a missing
    /// or unreadable one has to reach the journal at the point of collapse.
    #[tokio::test(flavor = "multi_thread")]
    async fn intent_loader_logs_the_typed_error_when_the_bundle_is_tampered() {
        let (state, _dir) = test_state_with_bundle("composed");
        let effects = ProductionSharedProviderEffects::new(
            Arc::new(state.clone()),
            ZoneId::parse("test").unwrap(),
            ControllerGeneration::new(1).unwrap(),
            d2b_core::bundle_resolver::BundleResolver::load_with_policy(
                &state.config.artifacts.bundle_path,
                &d2b_core::bundle_resolver::BundleVerifyPolicy::for_tests(),
            )
            .expect("load the composed bundle"),
        );

        // Tamper the on-disk bundle after composition:the next intent
        // resolution re-verifies it per invocation and refuses closed, and
        // the tamper reason must be journaled at the point of collapse.

        tokio::fs::write(
            &state.config.artifacts.bundle_path,
            br#"{ "schemaVersion": "v3" }"#,
        )
        .await
        .expect("tamper the bundle");

        let output = capture_journal_output(|| {
            let provenance = NetworkProvenance::new(
                ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap(),
                ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap(),
                ResourceGeneration::new(1).unwrap(),
                ResourceGeneration::new(1).unwrap(),
                d2b_contracts_resource::v3::ResourceBundleGenerationId::parse(format!(
                    "sha256:{}",
                    "ab".repeat(32),
                ))
                .unwrap(),
            );
            let intent = effects.intents.resolve_bridge_intent("bridge-0", &provenance);
            assert!(
                intent.is_none(),
                "a tampered bundle yields no intent:the effect refuses closed",
            );
        });
        assert!(
            output.contains("BundleTampered"),
            "the typed tamper error must reach the journal: {output:?}",
        );
    }

    /// Capture everything one action emits with the daemon's default filter
    /// applied (`main.rs` initializes `info`): an event below that level never
    /// reaches the host journal.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn capture_journal_output(action: impl FnOnce()) -> String {
        #[derive(Clone)]
        struct Buffer(Arc<std::sync::Mutex<Vec<u8>>>);

        // Synchronous by construction (the tracing writer surface is sync):
        // stays a `std::sync::Mutex` test fake under the plan's sanctioned
        // cfg(test)-helper survivor class.


        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        impl std::io::Write for Buffer {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.0.lock().expect("journal buffer").extend_from_slice(bytes);
                Ok(bytes.len())
            }

            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Buffer {
            type Writer = Buffer;

            fn make_writer(&'a self) -> Self::Writer {
                self.clone()
            }
        }

        let buffer = Buffer(Arc::new(std::sync::Mutex::new(Vec::new())));
        let subscriber = tracing_subscriber::fmt()
            .with_writer(buffer.clone())
            .with_max_level(tracing::Level::INFO)
            .with_ansi(false)
            .without_time()
            .finish();
        tracing::subscriber::with_default(subscriber, action);
        String::from_utf8(buffer.0.lock().expect("journal buffer").clone())
            .expect("journal output is utf-8")
    }

    /// A daemon state whose trusted bundle path names a freshly written
    /// self-hashed v3 zone-native bundle carrying the given generator
    /// marker.
    fn test_state_with_bundle(generator: &str) -> (ServerState, tempfile::TempDir) {
        use std::collections::HashMap;
        let dir = tempfile::tempdir().expect("network freshness test state");
        let daemon_state_dir = dir.path().join("daemon-state");
        let bundle_path = dir.path().join("bundle.json");
        write_v3_native_bundle(&bundle_path, generator);
        let broker_reap_log = d2bd_runtime::supervisor::pidfd_table::BrokerReapLog::new();
        let pidfd_table = Arc::new(
            d2bd_runtime::supervisor::pidfd_table::PidfdTable::new(
                daemon_state_dir.join("pidfd-table.json"),
            )
            .with_broker_reap_log(Arc::clone(&broker_reap_log)),
        );
        let state = ServerState {
            config: d2bd_runtime::daemon_config::DaemonConfig {
                artifacts: d2bd_runtime::daemon_config::ArtifactPaths {
                    bundle_path,
                    ..d2bd_runtime::daemon_config::ArtifactPaths::default()
                },
                ..d2bd_runtime::daemon_config::DaemonConfig::default()
            },
            daemon_uid: 0,
            daemon_state_dir,
            pidfd_table,
            broker_reap_log,
            metrics_registry: Arc::new(d2bd_runtime::metrics::Registry::new()),
            daemon_audit: Arc::new(d2bd_runtime::daemon_audit::DaemonAuditLog::no_op()),
            exec_sessions: Arc::new(crate::exec_session::SessionTable::new(
                crate::exec_session::ExecSessionCaps::default(),
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
            console_sessions: Arc::new(tokio::sync::Mutex::new(
                crate::console_session::ConsoleSessionTable::default(),
            )),
            security_key_sessions: Arc::new(tokio::sync::Mutex::new(
                d2b_provider_device_security_key::SkSessionTable::default(),
            )),
            unsafe_local_helpers: Arc::new(d2bd_runtime::unsafe_local_helper::HelperRegistry::new(
                0,
                [],
            )),
            v3_planes: std::sync::Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            runtime_handle: tokio::runtime::Handle::try_current()
                .expect("the network freshness test runs on a tokio runtime"),
        };
        (state, dir)
    }

    /// Write a minimal self-hashed v3 zone-native bundle whose installed
    /// generation identity is the `bundleHash` digest (the same contract
    /// `verify_bundle_hash` enforces for `schemaVersion >= 2`).
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn write_v3_native_bundle(bundle_path: &std::path::Path, generator: &str) {
        use std::os::unix::fs::PermissionsExt as _;

        let mut bundle = json!({
            "bundleVersion": 1,
            "schemaVersion": "v3",
            "privilegesPath": "privileges.json",
            "zones": [],
            "artifactHashes": {},
            "generation": {
                "generator": generator,
                "sourceRevision": null,
                "generatedAt": null
            }
        });
        // `verify_bundle_hash` re-derives the digest over the serialization
        // with `bundleHash` absent and `artifactHashes` nullified.
        let mut preimage_value = bundle.clone();
        if let Some(obj) = preimage_value.as_object_mut() {
            obj.remove("bundleHash");
            obj.insert("artifactHashes".to_owned(), serde_json::Value::Null);
        }
        let preimage =
            serde_json::to_vec(&preimage_value).expect("serialize v3 bundle hash preimage");
        let digest = {
            use sha2::Digest as _;
            let mut hasher = sha2::Sha256::new();
            hasher.update(&preimage);
            let hex: String = hasher
                .finalize()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect();
            format!("sha256:{hex}")
        };
        bundle["bundleHash"] = json!(digest);
        std::fs::write(
            bundle_path,
            serde_json::to_vec(&bundle).expect("serialize v3 native bundle"),
        )
        .expect("write v3 native bundle");
        std::fs::set_permissions(bundle_path, std::fs::Permissions::from_mode(0o640))
            .expect("chmod test bundle");
    }

    // -------------------------------------------------------------------
    // Network admission (same-zone Guest attachments): the attached row's
    // `resource_value` document carries the manager's authoritative `uid`
    // and `generation` at the top level, but the old reads looked for them
    // under `/metadata/*` - the raw stored metadata (ownerRef, labels,
    // annotations) carries neither, so the generation silently contributed
    // zero and the uid read refused every Guest attachment. These tests
    // drive the real `network_admission` over a real manager plane with
    // Nix-ingested rows, reproducing the exact raw-metadata shape the
    // admission reads.
    // -------------------------------------------------------------------

    use std::collections::BTreeMap;
    use std::sync::Arc;

    use d2b_contracts_resource::v3::{
        ControllerGeneration, ResourceGeneration, ResourceRef, ResourceUid, ZoneId,
    };

    use super::ProductionSharedProviderEffects;

    /// One no-op child surface: `network_admission` never consults children.
    struct UnusedChildSurface;

    #[async_trait::async_trait]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    impl d2b_provider_toolkit::SharedProviderChildSurface for UnusedChildSurface {
        async fn ensure(
            &self,
            _child: d2b_resource_runtime::context::ChildEnsure,
        ) -> Result<
            d2b_resource_runtime::spec_store::EnsureOutcome,
            d2b_provider_toolkit::SharedProviderEffectError,
        > {
            Err(d2b_provider_toolkit::SharedProviderEffectError::Unavailable)
        }

        async fn delete(
            &self,
            _key: &d2b_resource_runtime::identity::ResourceKey,
        ) -> Result<(), d2b_provider_toolkit::SharedProviderEffectError> {
            Err(d2b_provider_toolkit::SharedProviderEffectError::Unavailable)
        }

        async fn view(
            &self,
            _key: &d2b_resource_runtime::identity::ResourceKey,
        ) -> Result<
            Option<d2b_resource_runtime::manager::ResourceView>,
            d2b_provider_toolkit::SharedProviderEffectError,
        > {
            Ok(None)
        }
    }

    /// The same fake plane inputs the plane tests own, for the given zone.
    fn network_admission_plane_inputs(
        zone: d2b_contracts_resource::v3::ZoneId,
    ) -> (tempfile::TempDir, crate::resource_plane_v3::ConstructionInputs) {
        use d2bd_runtime::resource_runtime_support::NewPlaneReadinessState;

        let dir = tempfile::tempdir().expect("tempdir");
        let spec_store_dir = dir.path().join("daemon-state/zones").join(zone.as_str());
        let _readiness = Arc::new(NewPlaneReadinessState::new());
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
                d2b_provider_system_core::MinijailPlatformGate::new(6, 9, true),
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
        let tpm_facets = d2b_provider_device_tpm::test_support::recording_facets();
        // U6:the plane tests build the VolumeBinding and Endpoint families'
        // facet sets from the scripted doubles, exactly as the production
        // composition root builds them from the daemon's registry, plane
        // table, and target directory.
        let binding_facets = {
            let effects = d2b_provider_volume_binding::test_support::FakeServingEffects::new();
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
        // U12: the plane tests build the interaction family's facet set from
        // the scripted sources, exactly as the production composition root
        // builds it from the daemon's.
        let interaction_facets = d2b_provider_wayland_policy::test_support::scripted_facets(
            zone.clone(),
        );
        (
            dir,
            crate::resource_plane_v3::ConstructionInputs {
                zone: zone.clone(),
                zone_token: d2b_contracts_resource::v3::execution_policy::BoundedToken::parse(
                    zone.as_str().to_owned(),
                )
                .unwrap(),
                spec_store_dir,
                authority: crate::resource_plane_v3::ZoneAuthorityInputs {
                    zone_uid: None,
                    policy_revision: Some(1),
                    provider_assignment_generation: None,
                    controller_generation:
                        d2b_contracts_resource::v3::ControllerGeneration::new(1).unwrap(),
                    guest_execution: None,
                    mode: d2bd_runtime::target_runtime::DaemonMode::Host,
                    vcpu_count: 1,
                },
                committed_provider_identities: BTreeMap::new(),
                registry: Arc::new(crate::resource_plane_v3::PlaneResourceRegistry::new()),
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
                // U8: the plane tests build the Credential family's facet set
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
                // U1/U14/U5/U7/U8/U6/U10/U12/U15: the plane hosts every converted
                // family's declared effects services from the same facet
                // sets their driver factories are built from, exactly as the
                // production composition root does (the systemd service
                // carries no facet set, R2).
                effect_service_factories: BTreeMap::from([
                    (
                        d2b_provider_process::PROCESS_EFFECTS_SERVICE.id,
                        Arc::new(d2b_provider_process::ProcessEffectsServiceFactory::new(
                            process_facets,
                        )) as Arc<dyn d2b_provider_toolkit::EffectServiceFactory>,
                    ),
                    (
                        d2b_provider_network_local::NETWORK_EFFECTS_SERVICE.id,
                        Arc::new(d2b_provider_network_local::NetworkEffectsServiceFactory::new(
                            network_facets.clone(),
                        )) as Arc<dyn d2b_provider_toolkit::EffectServiceFactory>,
                    ),
                    (
                        d2b_provider_host::HOST_EFFECTS_SERVICE.id,
                        Arc::new(d2b_provider_host::HostEffectsServiceFactory::new(
                            host_facets,
                        )) as Arc<dyn d2b_provider_toolkit::EffectServiceFactory>,
                    ),
                    (
                        d2b_provider_volume::VOLUME_EFFECTS_SERVICE.id,
                        Arc::new(d2b_provider_volume::VolumeEffectsServiceFactory::new(
                            volume_facets.clone(),
                        )) as Arc<dyn d2b_provider_toolkit::EffectServiceFactory>,
                    ),
                    (
                        d2b_provider_wayland_policy::INTERACTION_EFFECTS_SERVICE.id,
                        Arc::new(
                            d2b_provider_wayland_policy::InteractionEffectsServiceFactory::new(
                                interaction_facets,
                            ),
                        ) as Arc<dyn d2b_provider_toolkit::EffectServiceFactory>,
                    ),
                    (
                        d2b_provider_activation_nixos::ACTIVATION_EFFECTS_SERVICE.id,
                        Arc::new(d2b_provider_activation_nixos::ActivationEffectsServiceFactory::new(
                            activation_facets,
                        )) as Arc<dyn d2b_provider_toolkit::EffectServiceFactory>,
                    ),
                    // U15: the family's service carries no facet set (R2),
                    // so the plane tests host its factory from crate-owned
                    // constants alone, exactly as the production composition
                    // root does.
                    (
                        d2b_provider_process_systemd::effects_service::PROCESS_SYSTEMD_EFFECTS_SERVICE
                            .id,
                        Arc::new(
                            d2b_provider_process_systemd::effects_service::SystemdEffectsServiceFactory::new(),
                        ) as Arc<dyn d2b_provider_toolkit::EffectServiceFactory>,
                    ),
                    (
                        d2b_provider_user::USER_EFFECTS_SERVICE.id,
                        Arc::new(d2b_provider_user::UserEffectsServiceFactory::new(
                            user_facets.clone(),
                        )) as Arc<dyn d2b_provider_toolkit::EffectServiceFactory>,
                    ),
                    (
                        d2b_provider_guest::GUEST_EFFECTS_SERVICE.id,
                        Arc::new(d2b_provider_guest::GuestEffectsServiceFactory::new(
                            guest_facets,
                        )) as Arc<dyn d2b_provider_toolkit::EffectServiceFactory>,
                    ),
                    (
                        d2b_provider_device_usbip::USBIP_EFFECTS_SERVICE.id,
                        Arc::new(d2b_provider_device_usbip::effects_service::
                            UsbipEffectsServiceFactory::new(
                                usbip_facets,
                            )) as Arc<dyn d2b_provider_toolkit::EffectServiceFactory>,
                    ),
                    (
                        d2b_provider_device_security_key::SECURITY_KEY_EFFECTS_SERVICE.id,
                        Arc::new(d2b_provider_device_security_key::effects_service::
                            SecurityKeyEffectsServiceFactory::new(
                                security_key_facets,
                            )) as Arc<dyn d2b_provider_toolkit::EffectServiceFactory>,
                    ),
                    (
                        d2b_provider_device::DEVICE_EFFECTS_SERVICE.id,
                        Arc::new(d2b_provider_device::effects_service::
                            DeviceEffectsServiceFactory::new(device_facets))
                            as Arc<dyn d2b_provider_toolkit::EffectServiceFactory>,
                    ),
                    (
                        d2b_provider_device_tpm::effects_service::TPM_EFFECTS_SERVICE.id,
                        Arc::new(d2b_provider_device_tpm::effects_service::TpmEffectsServiceFactory::new(
                            tpm_facets.clone(),
                        )) as Arc<dyn d2b_provider_toolkit::EffectServiceFactory>,
                    ),
                    (
                        d2b_provider_volume_binding::BINDING_EFFECTS_SERVICE.id,
                        Arc::new(d2b_provider_volume_binding::BindingEffectsServiceFactory::new(
                            binding_facets.clone(),
                        )) as Arc<dyn d2b_provider_toolkit::EffectServiceFactory>,
                    ),
                    (
                        d2b_provider_endpoint::ENDPOINT_EFFECTS_SERVICE.id,
                        Arc::new(d2b_provider_endpoint::EndpointEffectsServiceFactory::new(
                            endpoint_facets.clone(),
                        )) as Arc<dyn d2b_provider_toolkit::EffectServiceFactory>,
                    ),
                    (
                        d2b_provider_credential::CREDENTIAL_EFFECTS_SERVICE.id,
                        Arc::new(d2b_provider_credential::CredentialEffectsServiceFactory::new(
                            credential_facets.clone(),
                        )) as Arc<dyn d2b_provider_toolkit::EffectServiceFactory>,
                    ),
                ]),
                foundation: None,
            },
        )
    }

    /// One Network row's authored spec: one attached Guest.
    fn network_admission_spec(guest_name: &str) -> d2b_contracts_resource::v3::network::NetworkSpec {
        use d2b_contracts_resource::v3::execution_policy::BoundedToken;
        use d2b_contracts_resource::v3::network::{
            DhcpSpec, DnsSpec, Ipv4Cidr, IsolationSpec, MdnsSpec, NetworkAttachmentEntry,
            NetworkSpec, RoutingSpec,
        };
        NetworkSpec::new(
            // TEST-NET-2 LAN and a benchmark-range uplink: the admission
            // observes the real host, so the intent's CIDRs must not overlap
            // any address the host actually configures (a host on
            // TEST-NET-3, for example, collides with the classic
            // `203.0.113.0/30` uplink choice).
            Ipv4Cidr::parse("198.51.100.0/24").unwrap(),
            Ipv4Cidr::parse("198.18.0.0/30").unwrap(),
            None,
            false,
            IsolationSpec::default(),
            RoutingSpec::default(),
            DhcpSpec::default(),
            DnsSpec::default(),
            None,
            MdnsSpec::default(),
            None,
            BoundedToken::parse("net-vm-base").unwrap(),
            vec![NetworkAttachmentEntry::new(
                ResourceRef::parse(&format!("Guest/{guest_name}")).unwrap(),
                2,
                None,
            )
            .unwrap()],
        )
        .unwrap()
    }

    /// One Guest row committed into the plane, carrying the reciprocal
    /// `networkAttachments` the Network row's admission requires.
    fn network_admission_guest_row(
        zone: &d2b_contracts_resource::v3::ZoneId,
        network_ref: &str,
    ) -> d2b_contracts_zone_session::v3::resource_bundle::BundleResource {
        use d2b_contracts_resource::v3::execution_policy::{
            BudgetSpec, ExecutionDomain, ExecutionPolicy, NetworkAttachment,
        };
        use d2b_contracts_resource::v3::resource_schema::CanonicalJsonObject;

        let policy = ExecutionPolicy::new(
            ExecutionDomain::System,
            vec![ExecutionDomain::System],
            None,
            BudgetSpec::default(),
            vec![NetworkAttachment::new(
                ResourceRef::parse(network_ref).unwrap(),
                true,
            )
            .unwrap()],
            Vec::new(),
            Vec::new(),
        )
        .unwrap();
        let spec = d2b_provider_guest::GuestSpec::new(policy, None);
        d2b_contracts_zone_session::v3::resource_bundle::BundleResource::new(
            d2b_contracts_resource::v3::ResourceTypeName::parse("Guest").unwrap(),
            d2b_contracts_zone_session::v3::resource_bundle::BundleResourceMetadata::new(
                d2b_contracts_resource::v3::ResourceName::parse("gateway").unwrap(),
                zone.clone(),
                None,
                BTreeMap::new(),
                BTreeMap::new(),
            ),
            serde_json::from_value::<CanonicalJsonObject>(
                serde_json::to_value(&spec).expect("serialize Guest spec"),
            )
            .expect("canonical Guest spec"),
        )
        .expect("Guest bundle row")
    }

    /// One Network row committed into the plane with the given authored spec.
    fn network_admission_network_row(
        zone: &d2b_contracts_resource::v3::ZoneId,
        spec: &d2b_contracts_resource::v3::network::NetworkSpec,
    ) -> d2b_contracts_zone_session::v3::resource_bundle::BundleResource {
        use d2b_contracts_resource::v3::resource_schema::CanonicalJsonObject;

        d2b_contracts_zone_session::v3::resource_bundle::BundleResource::new(
            d2b_contracts_resource::v3::ResourceTypeName::parse("Network").unwrap(),
            d2b_contracts_zone_session::v3::resource_bundle::BundleResourceMetadata::new(
                d2b_contracts_resource::v3::ResourceName::parse("zone-net").unwrap(),
                zone.clone(),
                None,
                BTreeMap::new(),
                BTreeMap::new(),
            ),
            serde_json::from_value::<CanonicalJsonObject>(
                serde_json::to_value(spec).expect("serialize Network spec"),
            )
            .expect("canonical Network spec"),
        )
        .expect("Network bundle row")
    }

    /// One canonical per-Zone storage row binding the bundle's Zone UID.
    fn network_admission_storage_row(
        zone: &str,
        zone_uid: &d2b_contracts_resource::v3::ResourceUid,
        store_uid: &d2b_contracts_resource::v3::ResourceUid,
    ) -> d2b_contracts_resource::v3::storage::ZoneStoreStorageRow {
        let store_identity = d2b_contracts_resource::v3::storage::ZoneStoreIdentity::new(
            zone_uid.clone(),
            store_uid.clone(),
            1,
        )
        .expect("valid store identity");
        serde_json::from_value(serde_json::json!({
            "identity": store_identity,
            "zoneStoreId": format!("zone-store-{zone}"),
            "storageOwnerPrincipal": "d2b-zonert",
            "parentDirectoryId": format!("zone-store-parent-{zone}"),
            "ownership": {
                "owner": "d2b-zonert", "group": "d2b-zonert",
                "mode": "0640", "linkCount": 1
            },
            "auxiliaryDirectories": {
                "audit": {
                    "directoryId": format!("zone-store-audit-{zone}"),
                    "owner": "d2bd", "group": "d2bd",
                    "mode": "0700", "repairOwner": "privileged-broker"
                },
                "telemetry": {
                    "directoryId": format!("zone-store-telemetry-{zone}"),
                    "owner": "d2bd", "group": "d2bd",
                    "mode": "0700", "repairOwner": "privileged-broker"
                }
            },
            "filesystem": "regular-file-anchored-fd-relative-no-follow",
            "locking": "ofd-close-on-exec",
            "marker": {
                "identityMarkerId": format!("zone-store-marker-{zone}")
            },
            "replacementDetection": "fail-closed-on-missing-replaced-or-identity-mismatch",
            "fsync": "database-and-parent-directory",
            "publication": {
                "descriptor": "owned-descriptor-close-on-exec-verified-before-concurrency",
                "replacement": "atomic-rename-retain-prior-quarantine-ambiguity"
            }
        }))
        .expect("valid storage row")
    }

    /// A fixture resolver the admission's installed-generation read needs.
    fn network_admission_resolver() -> d2b_core::bundle_resolver::BundleResolver {
        use d2b_core::bundle::{Bundle, BundleGeneration};
        use d2b_core::manifest_v04::ManifestV04;
        use d2b_core::processes::ProcessesJson;

        let host = serde_json::from_str(include_str!(
            "../../../tests/fixtures/deny-unknown/host-valid.json"
        ))
        .unwrap();
        let manifest = ManifestV04::from_slice(
            include_str!("../../../tests/golden/manifest_v04/baseline-vms.json").as_bytes(),
        )
        .unwrap();
        d2b_core::bundle_resolver::BundleResolver::from_artifacts_with_zone_resource_bundles(
            Bundle {
                bundle_version: 1,
                schema_version: "v3".to_owned(),
                privileges_path: "privileges.json".to_owned(),
                storage_path: None,
                realm_workloads_launcher_v2_path: None,
                generation: BundleGeneration {
                    generator: "test".to_owned(),
                    source_revision: None,
                    generated_at: None,
                },
                bundle_hash: Some(format!("sha256:{}", "a".repeat(64))),
                artifact_hashes: None,
            },
            host,
            ProcessesJson {
                schema_version: "v2".to_owned(),
                vms: Vec::new(),
            },
            manifest,
            BTreeMap::new(),
        )
    }

    /// Every live piece `network_admission` needs, over one committed
    /// Network row and one committed Guest row in the same zone.
    struct NetworkAdmissionHarness {
        runtime: std::sync::Arc<crate::resource_runtime::ZoneResourceRuntime>,
        effects: ProductionSharedProviderEffects,
        network_uid: d2b_contracts_resource::v3::ResourceUid,
        network_generation: d2b_contracts_resource::v3::ResourceGeneration,
        resolver: d2b_core::bundle_resolver::BundleResolver,
        spec: d2b_contracts_resource::v3::network::NetworkSpec,
        _plane_dir: tempfile::TempDir,
    }

    async fn network_admission_harness() -> NetworkAdmissionHarness {
        let zone = ZoneId::parse("work").unwrap();
        let zone_uid = ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap();
        let store_uid = ResourceUid::parse("223e4567-e89b-42d3-a456-426614174001").unwrap();

        let spec = network_admission_spec("gateway");
        let guest_row = network_admission_guest_row(&zone, "Network/zone-net");
        let network_row = network_admission_network_row(&zone, &spec);
        let bundle = d2b_contracts_zone_session::v3::resource_bundle::ResourceBundle::new(
            zone.clone(),
            vec![guest_row, network_row],
            format!("sha256:{}", "a".repeat(64)),
            BTreeMap::new(),
            BTreeMap::new(),
            d2b_contracts_resource::v3::Timestamp::parse("2026-01-01T00:00:00.000Z").unwrap(),
        )
        .unwrap()
        .with_zone_uid(zone_uid.clone());
        let storage_row = network_admission_storage_row(zone.as_str(), &zone_uid, &store_uid);
        let authority =
            d2bd_runtime::zone_authority::ZoneAuthorityIdentity::from_bundle_and_storage(
                &zone,
                &bundle,
                &storage_row,
            )
            .expect("bundle authority");

        let state = crate::detached_exec_routing_tests::test_state(
            crate::exec_session::ExecSessionCaps::default(),
        );
        let (plane_dir, inputs) = network_admission_plane_inputs(zone.clone());
        let plane = Arc::new(
            crate::resource_plane_v3::ResourcePlaneV3::open(inputs)
                .await
                .expect("open plane"),
        );
        plane
            .ingest_nix_bundle(&bundle)
            .await
            .expect("ingest Network and Guest rows");

        let runtime = crate::resource_runtime::ZoneResourceRuntime::open_production_with_identity(
            zone.clone(),
            bundle,
            authority,
        )
        .await
        .expect("open zone runtime");

        runtime.attach_v3_planes(Arc::new(tokio::sync::Mutex::new(
            std::collections::HashMap::from([(
                zone.as_str().to_owned(),
                Arc::clone(&plane),
            )]),
        )));
        let mut composition = crate::resource_runtime::ResourcePlane::new();
        composition.set_topology_root(zone.clone());
        let runtime_arc = composition.insert(runtime).expect("insert zone runtime");
        let state_arc = Arc::new(state);
        *state_arc.resource_plane.lock().await = Some(Arc::new(composition));

        let resolver = network_admission_resolver();
        let network_key = ResourceKey::new("work", "Network", "zone-net");
        let view = plane
            .client()
            .get(network_key.clone())
            .await
            .expect("manager read")
            .expect("Network row committed");
        let network_uid = resource_uid(&view.uid).expect("committed Network uid");
        let network_generation =
            ResourceGeneration::new(view.generation).expect("committed Network generation");

        NetworkAdmissionHarness {
            runtime: runtime_arc,
            effects: ProductionSharedProviderEffects::new(
                Arc::clone(&state_arc),
                zone,
                ControllerGeneration::new(3).unwrap(),
                resolver.clone(),
            )
            .with_scripted_host_occupancy(HostNetworkOccupancy::from_parts(
                Vec::new(),
                Vec::new(),
                Vec::new(),
            )),
            network_uid,
            network_generation,
            resolver,
            spec,
            _plane_dir: plane_dir,
        }
    }

    /// One effect request shaped exactly as the driver would build it.
    fn network_admission_request<'a>(
        zone: ZoneId,
        network_name: &str,
        uid: d2b_contracts_resource::v3::ResourceUid,
        generation: d2b_contracts_resource::v3::ResourceGeneration,
        children: &'a UnusedChildSurface,
    ) -> d2b_provider_toolkit::SharedProviderEffectRequest<'a> {
        d2b_provider_toolkit::SharedProviderEffectRequest {
            zone,
            target: ResourceKey::new("work", "Network", network_name),
            uid,
            generation,
            operation_id: "network-admission-test".to_owned(),
            spec: serde_json::json!({}),
            metadata: serde_json::json!({}),
            status: None,
            children: children as &dyn d2b_provider_toolkit::SharedProviderChildSurface,
        }
    }

    /// The attached Guest row is in the same zone as the Network row (both
    /// resolved under the plane's own zone): the admission must complete.
    ///
    /// Pre-fix this refused deterministically: the attached row's raw stored
    /// metadata carries no uid, so the old `/metadata/uid` read refused
    /// every Guest attachment (and the `/metadata/generation` read silently
    /// contributed zero). The fix reads the manager-authoritative
    /// `uid`/`generation` the `resource_value` document carries at the top
    /// level.
    #[tokio::test(flavor = "multi_thread")]
    async fn network_admission_admits_a_same_zone_attached_guest() {
        let harness = network_admission_harness().await;
        let children = UnusedChildSurface;

        let request = network_admission_request(
            ZoneId::parse("work").unwrap(),
            "zone-net",
            harness.network_uid.clone(),
            harness.network_generation,
            &children,
        );
        let result = harness
            .effects
            .network_admission(&harness.runtime, &request, &harness.spec, &harness.resolver)
            .await;
        // The refusal reason, not the result: the error Display is a closed
        // vocabulary and never carries the network or guest identity, so the
        // assertion message logs no uid.
        let verdict = match &result {
            Ok(_) => "admitted".to_owned(),
            Err(error) => error.to_string(),
        };
        assert!(
            result.is_ok(),
            "a same-zone attached Guest must be admitted: {verdict}",
        );
    }
}

