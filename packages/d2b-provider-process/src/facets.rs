//! The declared facets the provider-owned Process effects service reaches
//! daemon state through (U1).
//!
//! The Process family's driver effects are served by this crate's own
//! implementation (see [`crate::effects_service`]). The daemon state that
//! implementation holds - the composed process providers, the committed
//! controller-provider identities, the Guest-owner durable identities, and
//! the daemon runtime roots - crosses the provider boundary as declared
//! facets rather than as a daemon handle: every facet here is a type the
//! provider crate declares, an implementation of it is supplied by the
//! daemon host through the composition root (never derived from caller
//! input),and the family crate holds no daemon state type.



use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use d2b_contracts_resource::v3::process::{EphemeralProcessSpec, ProcessSpec};
use d2b_contracts_resource::v3::{
    ControllerGeneration, ResourceGeneration, ResourceRef, ResourceUid, SchemaFingerprint,
    ZoneId, ZoneRevision,
};
use d2b_core::bundle_resolver::BundleResolver;
use d2b_process_conformance::{AdoptionCandidate, GuestExecutionBinding, LaunchIdentity};
use d2b_resource_runtime::context::ResourceContext;

use crate::identity::{ProcessFamilySpec, ProcessResourceIdentity};
use crate::worker_launch::ServingWorkerLaunch;

use crate::effects::{ProviderAdoption, ProviderLiveness};
use crate::worker_launch::DeviceWorkerLaunch;

/// Result of a Provider-backed launch, carrying only opaque process identity.
///
/// The identity is established by the effect adapter and carries no host
/// handle;the broker's audit continuity keys on it alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderLaunch {
    /// Opaque identity established by the effect adapter.
    pub identity: d2b_process_conformance::ProcessIdentityDigest,
}

/// The provider-layer context of one resource row, consumed by the
/// composed process Providers.
///
/// The ticket machinery is entirely inside the provider layer:the
/// Process driver never assembles a ticket;the effects service builds
/// this context from the row identity and the daemon-supplied facets (the
/// committed controller-provider identity, the Guest-owner uid, and the
/// catalog-bound Guest setup descriptor digest),and the composed Providers
/// consume it. Every daemon-structural read rides the facets, never a
/// daemon state handle.
///
/// This type is owned by the family crate so its implementation can live
/// inside it; its fields are all cross-crate types the family already imports.
#[derive(Debug, Clone)]
pub struct ProcessResourceContext<'a> {
    /// The zone the row lives in.
    pub zone: ZoneId,
    /// The canonical resource reference of the row.
    pub resource_ref: &'a ResourceRef,
    /// The durable uid of the row.
    pub resource_uid: &'a ResourceUid,
    /// The durable generation of the row.
    pub resource_generation: ResourceGeneration,
    /// The row revision (the row generation, per the ticket binding).
    pub resource_revision: ZoneRevision,
    /// The canonical provider reference the row selects.
    pub provider_ref: &'a ResourceRef,
    /// The provider identity bound by the KTD7 committed source, if any.
    pub provider_uid: Option<ResourceUid>,
    /// The provider generation bound by the KTD7 committed source, if any.
    pub provider_generation: Option<ResourceGeneration>,
    /// The controller generation the launch ticket binds.
    pub controller_generation: ControllerGeneration,
    /// Binding-declared Guest execution inputs, when the row is guest-executing.
    pub guest_execution: Option<GuestExecutionBinding>,
    /// The zone-authority uid, if the plane serves one.
    pub zone_uid: Option<ResourceUid>,
    /// The zone policy revision, if the authority serves one.
    pub policy_revision: Option<u64>,
    /// The provider assignment generation, if one is assigned.
    pub provider_assignment_generation: Option<ResourceGeneration>,
    /// Semantic owner used to bind static Provider controller templates.
    pub owner_ref: Option<ResourceRef>,
    /// Immutable identity of the semantic owner.
    pub owner_uid: Option<ResourceUid>,
    /// Provider that owns the supervised controller route.
    pub controller_provider_ref: Option<ResourceRef>,
    /// Optional Guest selector for a shared Host execution reference.
    pub target_ref: Option<ResourceRef>,
    /// Exact execution reference from the Process spec.
    pub execution_ref: Option<ResourceRef>,
    /// Exact User scope from the Process execution spec.
    pub user_ref: Option<ResourceRef>,
    /// Catalog-bound private Guest setup descriptor digest.
    pub guest_descriptor_digest: Option<SchemaFingerprint>,
    /// Binding-declared serving-worker launch inputs, when this Process is a
    /// VolumeBinding-owned serving worker.
    pub worker_launch: Option<ServingWorkerLaunch>,
    /// Device-declared worker launch parameters, when this Process is one of
    /// the declared Device-owned worker rows (`Process/swtpm-<device>`,
    /// `EphemeralProcess/swtpm-flush-<device>`, `Process/gpu-<device>`,
    /// `Process/video-<device>`). Derived by the Process controller from the
    /// owning Device row,the trusted declared template,and the daemon's own
    /// runtime paths.
    pub device_worker_launch: Option<DeviceWorkerLaunch>,
    /// The canonical launch identity the owning row resolved (KTD7).The
    /// ticket builder consumes this value instead of re-deriving the owner,
    /// target, VM, or legacy role from the fields above.
    pub launch: Option<LaunchIdentity>,
}

impl<'a> ProcessResourceContext<'a> {
    /// Build the row context from the row identity and the zone authority.
    pub const fn new(
        zone: ZoneId,
        identity: (
            &'a ResourceRef,
            &'a ResourceUid,
            ResourceGeneration,
            ZoneRevision,
        ),
        provider_ref: &'a ResourceRef,
        controller_generation: ControllerGeneration,
        target_ref: Option<ResourceRef>,
    ) -> Self {
        let (resource_ref, resource_uid, resource_generation, resource_revision) = identity;
        Self {
            zone,
            resource_ref,
            resource_uid,
            resource_generation,
            resource_revision,
            provider_ref,
            provider_uid: None,
            provider_generation: None,
            controller_generation,
            guest_execution: None,
            zone_uid: None,
            policy_revision: None,
            provider_assignment_generation: None,
            owner_ref: None,
            owner_uid: None,
            controller_provider_ref: None,
            target_ref,
            execution_ref: None,
            user_ref: None,
            guest_descriptor_digest: None,
            worker_launch: None,
            device_worker_launch: None,
            launch: None,
        }
    }

    /// Attach the row-resolved canonical launch identity (KTD7).
    pub fn with_launch_identity(mut self, launch: LaunchIdentity) -> Self {
        self.launch = Some(launch);
        self
    }

    /// Attach the binding-declared serving-worker launch inputs.
    pub fn with_worker_launch(mut self, launch: Option<ServingWorkerLaunch>) -> Self {
        self.worker_launch = launch;
        self
    }

    /// Attach the Device-declared worker launch parameters.
    pub fn with_device_worker_launch(
        mut self,
        launch: Option<DeviceWorkerLaunch>,
    ) -> Self {
        self.device_worker_launch = launch;
        self
    }

    /// Attach the binding-declared Guest execution inputs.
    pub fn with_guest_execution(mut self, binding: Option<&GuestExecutionBinding>) -> Self {
        self.guest_execution = binding.cloned();
        self
    }

    /// Attach the zone-authority lifecycle identity.
    pub fn with_lifecycle_identity(
        mut self,
        zone_uid: Option<ResourceUid>,
        policy_revision: Option<u64>,
        provider_assignment_generation: Option<ResourceGeneration>,
    ) -> Self {
        self.zone_uid = zone_uid;
        self.policy_revision = policy_revision;
        self.provider_assignment_generation = provider_assignment_generation;
        self
    }

    /// Attach the semantic owner reference.
    pub fn with_owner_ref(mut self, owner_ref: Option<ResourceRef>) -> Self {
        self.owner_ref = owner_ref;
        self
    }

    /// Attach the semantic owner uid.
    pub fn with_owner_uid(mut self, owner_uid: Option<ResourceUid>) -> Self {
        self.owner_uid = owner_uid;
        self
    }

    /// Retained seam:the controller-provider reference a Guest-local launch
    /// binds into its controller bootstrap context. The U12 conversion
    /// retired the only production writer (the Guest-local Process runtime),so
    /// this stays available for the Guest-side realization follow-on and
    /// its tests;the bootstrap context falls back to the Provider owner ref
/// meanwhile。
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_controller_provider_ref(
        mut self,
        provider_ref: Option<ResourceRef>,
    ) -> Self {
        self.controller_provider_ref = provider_ref;
        self
    }

    /// Attach the provider uid and generation (the KTD7 committed binding).
    pub fn with_provider_identity(
        mut self,
        provider_uid: Option<&ResourceUid>,
        provider_generation: Option<ResourceGeneration>,
    ) -> Self {
        self.provider_uid = provider_uid.cloned();
        self.provider_generation = provider_generation;
        self
    }

    /// Attach the catalog-bound Guest setup descriptor digest.
    pub fn with_guest_descriptor_digest(
        mut self,
        descriptor_digest: Option<&SchemaFingerprint>,
    ) -> Self {
        self.guest_descriptor_digest = descriptor_digest.cloned();
        self
    }

    /// Attach the exact execution reference from the Process spec.
    pub fn with_execution_ref(mut self, execution_ref: &ResourceRef) -> Self {
        self.execution_ref = Some(execution_ref.clone());
        self
    }

    /// Attach the exact User scope from the Process execution spec.
    pub fn with_user_ref(mut self, user_ref: Option<&ResourceRef>) -> Self {
        self.user_ref = user_ref.cloned();
        self
    }
}

/// The composed fixed process Providers, as the family crate sees them.
///
/// The daemon owns the concrete composed runtime (`ProductionProcessProviders`,
/// in `d2bd`),which implements this provider-declared facet trait;the
/// composition root supplies it to the family's effects service through the
/// service factory, never as a daemon-side effect port and never derived
/// from caller input. The context type ([`ProcessResourceContext`]) is
/// family-owned too,so the implementation the crate serves can build and
/// consume it without naming a daemon state type.
#[async_trait::async_trait]
pub trait ProcessProviderRuntime: Send + Sync + 'static {
    /// The trusted bundle the composed Providers resolve their intent from.
    fn bundle(&self) -> &BundleResolver;

    /// The daemon's per-VM device socket root under its runtime root.
    fn socket_runtime_dir(&self) -> &Path;

    /// The catalog-bound Guest setup descriptor digest for one guest.
    fn guest_setup_descriptor_digest(
        &self,
        zone: &ZoneId,
        guest_ref: &ResourceRef,
    ) -> Option<SchemaFingerprint>;

    /// Resolve the typed launch parameters of one declared Device-owned
    /// worker row (U17 gap closure).
    ///
    /// The daemon host owns the Device-family-specific resolution - the
    /// owning Device's declared settings, the controller-created state
    /// Volume, the projected Wayland socket - and may name the device
    /// families; the family crate receives the already-resolved typed
    /// parameters and never names a device family. Returns `None` for every
    /// row that is not one of the declared Device worker templates, and a
    /// declared template whose trusted inputs cannot be resolved refuses
    /// with the named code.
    async fn resolve_device_worker_launch(
        &self,
        ctx: &mut ResourceContext,
        identity: &ProcessResourceIdentity,
        spec: &ProcessFamilySpec,
    ) -> Result<Option<DeviceWorkerLaunch>, &'static str>;

    /// Launch through the signed provider-ticket path.
    async fn launch_resource(
        &self,
        context: ProcessResourceContext<'_>,
        spec: &ProcessSpec,
        timeout: Duration,
    ) -> Result<ProviderLaunch, String>;

    /// Launch one one-shot process through the preserved ephemeral ticket.
    async fn launch_ephemeral_resource(
        &self,
        context: ProcessResourceContext<'_>,
        spec: &EphemeralProcessSpec,
        timeout: Duration,
    ) -> Result<ProviderLaunch, String>;

    /// Probe-and-adopt over pidfd/proc evidence with the preserved classification.
    async fn adopt_resource(
        &self,
        context: ProcessResourceContext<'_>,
        spec: &ProcessSpec,
    ) -> Result<ProviderAdoption, String>;

    /// Probe one already-started durable process.
    async fn probe_resource(
        &self,
        context: ProcessResourceContext<'_>,
        spec: &ProcessSpec,
    ) -> Result<ProviderLiveness, String>;

    /// Probe-and-adopt one one-shot process.
    async fn adopt_ephemeral_resource(
        &self,
        context: ProcessResourceContext<'_>,
        spec: &EphemeralProcessSpec,
    ) -> Result<ProviderAdoption, String>;

    /// Probe one already-started one-shot identity.
    async fn probe_ephemeral_resource(
        &self,
        context: ProcessResourceContext<'_>,
        spec: &EphemeralProcessSpec,
    ) -> Result<ProviderLiveness, String>;

    /// Preserved term-then-kill escalation with pidfd retry for a durable row.
    async fn stop_resource(
        &self,
        context: ProcessResourceContext<'_>,
        spec: &ProcessSpec,
        term_timeout: Duration,
        kill_timeout: Duration,
    ) -> Result<bool, String>;

    /// Preserved term-then-kill escalation for one one-shot row.
    async fn stop_ephemeral_resource(
        &self,
        context: ProcessResourceContext<'_>,
        spec: &EphemeralProcessSpec,
        term_timeout: Duration,
        kill_timeout: Duration,
    ) -> Result<bool, String>;

    /// Stop one exactly-identified stale candidate before a fresh launch.
    async fn stop_stale_resource(
        &self,
        provider_ref: &ResourceRef,
        candidate: &AdoptionCandidate,
    ) -> Result<(), String>;

    /// Remove the provider's exact local authority after a terminal exit.
    async fn finalize_resource(
        &self,
        context: ProcessResourceContext<'_>,
    ) -> Result<(), String>;

    /// Whether this zone retains a verified identity for the resource.
    fn has_active_resource_in_zone(
        &self,
        zone: &ZoneId,
        zone_uid: Option<&ResourceUid>,
        resource_ref: &ResourceRef,
    ) -> bool;
}

/// KTD7 committed controller-provider identity source:the committed
/// `Provider` row's durable uid/generation for one canonical Provider reference.
/// as the daemon's plane publishes it.
///
/// A controller Process owned by a `Provider` takes that Provider's
/// committed uid/generation when the driver left the identity unbound.A
/// reference the daemon retains no committed row for keeps the slot
/// unbound,so a genuinely missing row still refuses closed.
pub trait CommittedProviderIdentitySource: Send + Sync + 'static {
    /// The committed `(uid, generation)` for one Provider reference.
    fn committed_provider_identity(
        &self,
        provider: &ResourceRef,
    ) -> Option<(ResourceUid, ResourceGeneration)>;
}

/// The daemon-supplied facet set one Zone's Process effects service (and
/// the driver factory that serves it) is built from.
///
/// Every facet is a provider-declared trait object the daemon host supplies
/// through the composition root; none is derived from caller input,and
/// none is a daemon state type.(R2, KTD7)
#[derive(Clone)]
pub struct ProcessEffectFacets {
    /// The composed fixed process Providers.
    pub runtime: Arc<dyn ProcessProviderRuntime>,
    /// Committed controller-provider identities (KTD7), when wired.
    pub committed: Option<Arc<dyn CommittedProviderIdentitySource>>,
    /// Guest-owner durable identities (KTD7), when wired.
    pub guest_owners: Option<Arc<dyn crate::GuestOwnerIdentitySource>>,
}