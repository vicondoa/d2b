//! The provider-owned implementation of the Process family's driver effects
//! (U1): the family serves its effects over the daemon-supplied facets
//! instead of a daemon-built port.
//!
//! Two surfaces share one implementation value:
//!
//! - the driver's typed seam, [`ProcessDriverEffects`], which the family's
//!   driver holds (the factory builds it from the same facets);
//! - the declared zone-plane service [`PROCESS_EFFECTS_SERVICE`], hosted
//!   per zone by the daemon through [`ProcessEffectsServiceFactory`]. Its one
//!   method (`has-active`) answers whether the zone retains a verified
//!   identity for one resource - the same report the provider-side
//!   admission fence reads.
//!
//! Everything the effects read crosses the provider boundary as declared
//! facets ([`crate::facets`]): the composed fixed process providers, the
//! committed Provider and Guest-owner identities the plane publishes (KTD7),
//! the trusted bundle's intents and projected site artifacts, the daemon's
//! own runtime roots, and the already-resolved Device-worker launch
//! parameters (the daemon host owns the Device-family-specific resolution
//! and may name the device families). Nothing here names a daemon state type
//! or a device family.

use std::sync::Arc;
use std::time::Duration;

use d2b_contracts_resource::v3::process::{EphemeralProcessSpec, ProcessClass, ProcessSpec};
use d2b_contracts_resource::v3::{
    CanonicalJsonObject, CanonicalJsonValue, ResourceRef, ResourceUid, SchemaFingerprint, ZoneId,
    ZoneRevision,
};
use d2b_process_conformance::{AdoptionCandidate, ProcessIdentityDigest};
use d2b_provider_toolkit::{
    EffectResponse, EffectService, EffectServiceError, EffectServiceFactory, ServiceInvocation,
};
use d2b_resource_runtime::context::ResourceContext;
use d2b_resource_types::{ServiceDecl, ServiceMethod};

use crate::effects::{ProcessDriverEffects, ProviderAdoption, ProviderLiveness};
use crate::facets::{
    CommittedProviderIdentitySource, ProcessEffectFacets, ProcessProviderRuntime,
    ProcessResourceContext,
};
use crate::identity::{ProcessFamilySpec, ProcessResourceIdentity};
use crate::worker_launch::DeviceWorkerLaunch;
use crate::{GuestOwnerIdentitySource, device_worker_family, resolve_guest_owner_uid};

/// The Process family's declared effects service.
///
/// One zone-plane method, `has-active`: it answers whether this zone retains
/// a verified identity for one resource (`resourceRef`). Payload:
///
/// ```json
/// { "zone": "<zone>", "zoneUid": null, "resourceRef": "<ref>" }
/// ```
///
/// The invocation's zone is the authoritative one the host addressed: a
/// payload naming another zone refuses with its own closed code. The
/// zone-authority uid is host-supplied scope, not a caller assertion - the
/// capability object carries none today, so a payload that asserts one
/// refuses instead of being trusted.
///
/// Response: `{ "active": true|false }`.
///
/// The service is declared on the `Process` descriptor alone; the family's
/// driver effects (the typed seam) stay the driver's object, not a hosted
/// method surface.
pub const PROCESS_EFFECTS_SERVICE: ServiceDecl = ServiceDecl {
    id: "process.d2bus.org/effects",
    methods: &[ServiceMethod::zone_plane("has-active")],
    attach_kinds: &[],
    streams: &[],
    endpoint_policy: None,
};

/// The provider-owned Process effects (U1), built from the daemon-supplied
/// facets.
///
/// One value serves both the driver's typed seam and the declared hosted
/// service: the factory constructs it from the same [`ProcessEffectFacets`]
/// the composition root supplies, so the hosted surface and the driver
/// observe the same runtime.
pub struct ProcessEffectsService {
    runtime: Arc<dyn ProcessProviderRuntime>,
    /// Committed Provider identities (KTD7), wired by the plane's
    /// construction path from the composition-resolved snapshot.
    committed_provider_identities: Option<Arc<dyn CommittedProviderIdentitySource>>,
    /// Guest-owner durable identities (KTD7), wired by the plane's
    /// construction path from the pre-v3 plane that owns `Guest` rows.
    guest_owner_identities: Option<Arc<dyn GuestOwnerIdentitySource>>,
}

impl ProcessEffectsService {
    /// Build the effects from one zone's daemon-supplied facet set (R2):
    /// every daemon-structural read rides the facets, never a daemon handle.
    pub fn new(facets: ProcessEffectFacets) -> Self {
        Self {
            runtime: facets.runtime,
            committed_provider_identities: facets.committed,
            guest_owner_identities: facets.guest_owners,
        }
    }

    /// The provider-layer context for one row: the committed
    /// controller-provider identity (KTD7), the owning Guest's durable uid
    /// for a guest-owned row, and the catalog-bound Guest setup descriptor
    /// digest.
    async fn resource_context<'a>(
        &self,
        identity: &'a ProcessResourceIdentity,
    ) -> ProcessResourceContext<'a> {
        let guest_owner_uid =
            resolve_guest_owner_uid(self.guest_owner_identities.as_deref(), identity).await;
        process_resource_context(
            identity,
            self.committed_provider_identities.as_deref(),
            guest_owner_uid.as_ref(),
            |zone, guest| self.runtime.guest_setup_descriptor_digest(zone, guest),
        )
    }
}

/// Build the provider-layer context for one row: the committed
/// controller-provider identity (KTD7) first, then the owning Guest's
/// durable uid and the catalog-bound `Guest` setup descriptor digest for a
/// guest-owned row.
///
/// The digest is the old runner's `set_guest_descriptor_digests` input and the
/// private guest VMM intent lookup (`find_guest_vmm_intent`) refuses a ticket
/// without it (`provider-ticket:guest-descriptor-unbound`), so a
/// controller-minted `Process/<guest>-vmm` row cannot launch end to end until
/// the bundle's descriptor digest is bound. The owner uid is the linkage the
/// old composer read from the durable row (the broker refuses a Cloud
/// Hypervisor launch without it). Rows without a Guest owner bind nothing, and
/// a Guest the plane or bundle does not retain stays unbound - the ticket
/// path still refuses closed.
fn process_resource_context<'a>(
    identity: &'a ProcessResourceIdentity,
    committed_provider_identities: Option<&dyn CommittedProviderIdentitySource>,
    guest_owner_uid: Option<&ResourceUid>,
    guest_descriptor_digest: impl Fn(&ZoneId, &ResourceRef) -> Option<SchemaFingerprint>,
) -> ProcessResourceContext<'a> {
    let context =
        bind_committed_controller_provider_identity(identity, committed_provider_identities);
    let Some(guest) = identity
        .launch
        .owner_ref()
        .filter(|owner| owner.resource_type().as_str() == "Guest")
    else {
        return context;
    };
    let context = match guest_owner_uid {
        Some(guest_owner_uid) => context.with_owner_uid(Some(guest_owner_uid.clone())),
        None => context,
    };
    match guest_descriptor_digest(&identity.zone, guest) {
        Some(digest) => context.with_guest_descriptor_digest(Some(&digest)),
        None => context,
    }
}

/// Build the borrowed provider-layer context of one row from its identity
/// alone. The ticket machinery is entirely inside the provider layer; the
/// driver never assembles a ticket.
///
/// A free function rather than an inherent method because
/// [`ProcessResourceIdentity`] lives in the identity module and no inherent
/// impl can be written for a foreign type.
fn identity_resource_context(identity: &ProcessResourceIdentity) -> ProcessResourceContext<'_> {
    ProcessResourceContext::new(
        identity.zone.clone(),
        (
            &identity.resource_ref,
            &identity.resource_uid,
            identity.resource_generation,
            // The new store has no zone-wide commit revision: the durable
            // revision of a row is its generation. The launch ticket requires
            // a non-zero resource revision, and the provider identity fence
            // compares generations, not revisions, so the row generation is
            // the honest binding here.
            ZoneRevision::new(identity.resource_generation.get()),
        ),
        &identity.provider_ref,
        identity.controller_generation,
        identity.launch.target_ref().cloned(),
    )
    .with_guest_execution(identity.guest_execution.as_ref())
    .with_lifecycle_identity(
        identity.zone_uid.clone(),
        identity.policy_revision,
        identity.provider_assignment_generation,
    )
    .with_owner_ref(identity.launch.owner_ref().cloned())
    .with_owner_uid(identity.launch.owner_uid().cloned())
    .with_provider_identity(
        identity.controller_provider_uid.as_ref(),
        identity.controller_provider_generation,
    )
    .with_worker_launch(identity.worker_launch.clone())
    .with_device_worker_launch(identity.device_worker_launch.clone())
    .with_launch_identity(identity.launch.clone())
}

/// Bind the committed Provider row's identity (KTD7) onto one controller
/// row's provider context: a controller Process owned by a `Provider` takes
/// that Provider's committed uid/generation when the driver left the identity
/// unbound. Every other row - another process class, another owner type, an
/// already-bound identity, or a Provider with no committed row - keeps the
/// driver-derived context, so a genuinely missing row still refuses closed.
fn bind_committed_controller_provider_identity<'a>(
    identity: &'a ProcessResourceIdentity,
    source: Option<&dyn CommittedProviderIdentitySource>,
) -> ProcessResourceContext<'a> {
    let context = identity_resource_context(identity);
    if identity.process_class != ProcessClass::Controller
        || identity.controller_provider_uid.is_some()
        || identity.controller_provider_generation.is_some()
    {
        return context;
    }
    let Some(provider_owner) = identity
        .launch
        .owner_ref()
        .filter(|owner| owner.resource_type().as_str() == "Provider")
    else {
        return context;
    };
    match source.and_then(|source| source.committed_provider_identity(provider_owner)) {
        Some((uid, generation)) => context.with_provider_identity(Some(&uid), Some(generation)),
        None => context,
    }
}

/// The one `has-active` response payload: whether the Zone retains a verified
/// identity for the requested resource. The two literals are canonical by
/// construction; the parse refusal is unreachable and names its own code.
fn has_active_response(active: bool) -> Result<EffectResponse, EffectServiceError> {
    let bytes: &[u8] = if active {
        b"{\"active\":true}"
    } else {
        b"{\"active\":false}"
    };
    let payload = CanonicalJsonObject::parse(bytes).map_err(|_| {
        EffectServiceError::Declined {
            service: PROCESS_EFFECTS_SERVICE.id.to_owned(),
            reason: "has-active-response-invalid".to_owned(),
        }
    })?;
    Ok(EffectResponse::new(payload))
}

/// The declared `has-active` payload contract: `zone` names the zone and
/// `resourceRef` the resource the caller asks about.
fn payload_string<'a>(
    payload: &'a CanonicalJsonObject,
    key: &str,
    missing_code: &'static str,
) -> Result<&'a str, &'static str> {
    match payload.get(key) {
        Some(CanonicalJsonValue::String(value)) if !value.is_empty() => Ok(value),
        _ => Err(missing_code),
    }
}

/// Serve the `has-active` method: parse the zone and resource reference from
/// the canonical payload, bind the query to the invocation's authoritative
/// zone (a payload naming another zone refuses with its own closed code),
/// and answer from the runtime facet with the host-supplied zone-authority
/// scope. A payload that does not match the contract refuses with its own
/// closed code instead of answering a half-built report.
async fn serve_has_active(
    runtime: &dyn ProcessProviderRuntime,
    invocation_zone: &str,
    payload: &CanonicalJsonObject,
) -> Result<EffectResponse, EffectServiceError> {
    let declined = |reason: &'static str| EffectServiceError::Declined {
        service: PROCESS_EFFECTS_SERVICE.id.to_owned(),
        reason: reason.to_owned(),
    };
    let zone = payload_string(payload, "zone", "has-active-zone-missing")
        .map_err(declined)?;
    let zone = ZoneId::parse(zone).map_err(|_| declined("has-active-zone-invalid"))?;
    // The invocation's zone is the authoritative one the host addressed; a
    // payload naming a different zone is refused instead of answered.
    if zone.as_str() != invocation_zone {
        return Err(declined("has-active-zone-mismatch"));
    }
    // The zone-authority uid is host-supplied scope, never a caller
    // assertion: the capability object carries none today, so a payload
    // that asserts one is refused rather than trusted.
    match payload.get("zoneUid") {
        None | Some(CanonicalJsonValue::Null) => {}
        _ => return Err(declined("has-active-zone-uid-unsupplied")),
    }
    let resource_ref = payload_string(payload, "resourceRef", "has-active-resource-ref-missing")
        .map_err(declined)?;
    let resource_ref =
        ResourceRef::parse(resource_ref).map_err(|_| declined("has-active-resource-ref-invalid"))?;
    let active = runtime.has_active_resource_in_zone(&zone, None, &resource_ref);
    has_active_response(active)
}

#[async_trait::async_trait]
impl ProcessDriverEffects for ProcessEffectsService {
    async fn launch(
        &self,
        identity: &ProcessResourceIdentity,
        spec: &ProcessSpec,
        timeout: Duration,
    ) -> Result<ProcessIdentityDigest, String> {
        let context = self.resource_context(identity).await;
        self.runtime
            .launch_resource(context, spec, timeout)
            .await
            .map(|launch| launch.identity)
    }

    async fn launch_ephemeral(
        &self,
        identity: &ProcessResourceIdentity,
        spec: &EphemeralProcessSpec,
        timeout: Duration,
    ) -> Result<ProcessIdentityDigest, String> {
        let context = self.resource_context(identity).await;
        self.runtime
            .launch_ephemeral_resource(context, spec, timeout)
            .await
            .map(|launch| launch.identity)
    }

    async fn adopt(
        &self,
        identity: &ProcessResourceIdentity,
        spec: &ProcessSpec,
    ) -> Result<ProviderAdoption, String> {
        self.runtime
            .adopt_resource(self.resource_context(identity).await, spec)
            .await
    }

    async fn probe(
        &self,
        identity: &ProcessResourceIdentity,
        spec: &ProcessSpec,
    ) -> Result<ProviderLiveness, String> {
        self.runtime
            .probe_resource(self.resource_context(identity).await, spec)
            .await
    }

    async fn adopt_ephemeral(
        &self,
        identity: &ProcessResourceIdentity,
        spec: &EphemeralProcessSpec,
    ) -> Result<ProviderAdoption, String> {
        self.runtime
            .adopt_ephemeral_resource(self.resource_context(identity).await, spec)
            .await
    }

    async fn probe_ephemeral(
        &self,
        identity: &ProcessResourceIdentity,
        spec: &EphemeralProcessSpec,
    ) -> Result<ProviderLiveness, String> {
        self.runtime
            .probe_ephemeral_resource(self.resource_context(identity).await, spec)
            .await
    }

    async fn stop(
        &self,
        identity: &ProcessResourceIdentity,
        spec: &ProcessSpec,
        term_timeout: Duration,
        kill_timeout: Duration,
    ) -> Result<bool, String> {
        self.runtime
            .stop_resource(
                self.resource_context(identity).await,
                spec,
                term_timeout,
                kill_timeout,
            )
            .await
    }

    async fn stop_ephemeral(
        &self,
        identity: &ProcessResourceIdentity,
        spec: &EphemeralProcessSpec,
        term_timeout: Duration,
        kill_timeout: Duration,
    ) -> Result<bool, String> {
        self.runtime
            .stop_ephemeral_resource(
                self.resource_context(identity).await,
                spec,
                term_timeout,
                kill_timeout,
            )
            .await
    }

    async fn stop_stale(
        &self,
        provider_ref: &ResourceRef,
        candidate: &AdoptionCandidate,
    ) -> Result<(), String> {
        self.runtime.stop_stale_resource(provider_ref, candidate).await
    }

    async fn finalize(&self, identity: &ProcessResourceIdentity) -> Result<(), String> {
        self.runtime
            .finalize_resource(self.resource_context(identity).await)
            .await
    }

    fn has_active(
        &self,
        zone: &ZoneId,
        zone_uid: Option<&ResourceUid>,
        resource_ref: &ResourceRef,
    ) -> bool {
        self.runtime
            .has_active_resource_in_zone(zone, zone_uid, resource_ref)
    }

    /// Derive the typed launch parameters of one declared Device-owned worker
    /// row (`U17` gap closure).
    ///
    /// The Device Providers declare their worker rows path-free and the
    /// Process spec is argv-free by contract, so the inputs the device argv
    /// generators need are resolved by the daemon host and cross the facet
    /// boundary as the already-resolved typed launch parameters: the daemon
    /// owns the Device-family-specific resolution (the owning Device's
    /// declared settings, the controller-created state Volume, the projected
    /// Wayland socket) and may name the device families. This crate keeps
    /// the process mechanics - the declared worker template vocabulary, the
    /// typed launch shapes, and the driver seam - and never names a device
    /// family.
    ///
    /// Returns `None` for every row that is not one of the declared Device
    /// worker templates. A declared template whose trusted inputs cannot be
    /// resolved refuses the launch (the named code is the diagnosis) instead
    /// of launching bare.
    async fn device_worker_launch(
        &self,
        ctx: &mut ResourceContext,
        identity: &ProcessResourceIdentity,
        spec: &ProcessFamilySpec,
    ) -> Result<Option<DeviceWorkerLaunch>, &'static str> {
        // The declared Device-worker template vocabulary is this family's
        // own; a row no template declares is not a Device worker at all.
        if device_worker_family(spec.execution().template().as_str()).is_none() {
            return Ok(None);
        }
        self.runtime
            .resolve_device_worker_launch(ctx, identity, spec)
            .await
    }
}

#[async_trait::async_trait]
impl EffectService for ProcessEffectsService {
    async fn handle(
        &self,
        invocation: ServiceInvocation<'_>,
    ) -> Result<EffectResponse, EffectServiceError> {
        // The declaration's method gates admission at the hosting side; the
        // service serves its one declared zone-plane method bound to the
        // invocation's authoritative zone and refuses a payload that does
        // not match the method's contract.
        serve_has_active(&*self.runtime, invocation.zone, invocation.payload).await
    }
}

/// The composition-root factory that hosts the Process effects service in
/// one zone (R5): the daemon registers one per zone, carrying that zone's
/// facet set, and the host rebuilds the service from it on respawn.
pub struct ProcessEffectsServiceFactory {
    facets: ProcessEffectFacets,
}

impl ProcessEffectsServiceFactory {
    /// Build the factory from one zone's facet set。
    pub fn new(facets: ProcessEffectFacets) -> Self {
        Self { facets }
    }
}

impl EffectServiceFactory for ProcessEffectsServiceFactory {
    fn build(&self) -> Arc<dyn EffectService> {
        Arc::new(ProcessEffectsService::new(self.facets.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::BTreeMap;

    use parking_lot::Mutex;

    use d2b_contracts_resource::v3::process::ProcessClass;
    use d2b_contracts_resource::v3::{
        CanonicalJsonObject, ControllerGeneration, ResourceGeneration, ResourceUid,
        SchemaFingerprint,
    };
    use d2b_process_conformance::LaunchIdentity;
    use d2b_provider_toolkit::ServiceInvocation;
    use d2b_resource_runtime::context::ServiceResourceContext;
    use crate::{LaunchRow, ProcessResourceIdentity, resolve_launch_identity};

    // -- fixtures ------------------------------------------------------------

    /// The committed Provider uid the test source publishes.
    const COMMITTED_PROVIDER_UID: &str = "123e4567-e89b-42d3-a456-426614174010";
    /// The committed Provider generation the test source publishes.
    const COMMITTED_PROVIDER_GENERATION: u64 = 4;
    /// The durable uid the pre-v3 plane publishes for the owning Guest.
    const GUEST_UID: &str = "323e4567-e89b-42d3-a456-426614174001";
    /// The canonical provider reference every test row selects.
    const PROVIDER_REF: &str = "Provider/system-minijail";

    fn zone() -> ZoneId {
        ZoneId::parse("work").expect("zone")
    }

    /// The one canonical launch identity of a row, resolved by the family's
    /// own resolver: the context builders read only what this resolves, so
    /// every field below is the field the ticket path would see.
    fn launch_identity(
        owner: &str,
        process_name: &str,
        template: &str,
    ) -> LaunchIdentity {
        let owner = ResourceRef::parse(owner).expect("owner ref");
        let execution_ref = ResourceRef::parse("Host/host-system").expect("execution ref");
        resolve_launch_identity(&LaunchRow {
            owner_ref: Some(&owner),
            owner_uid: None,
            execution_ref: &execution_ref,
            process_name,
            template,
            declared_target: None,
        })
        .expect("complete launch identity")
    }

    fn identity(
        resource: &str,
        process_name: &str,
        template: &str,
        owner: &str,
        process_class: ProcessClass,
    ) -> ProcessResourceIdentity {
        ProcessResourceIdentity {
            zone: zone(),
            resource_ref: ResourceRef::parse(resource).expect("resource ref"),
            resource_uid: ResourceUid::parse("423e4567-e89b-42d3-a456-426614174002")
                .expect("resource uid"),
            resource_generation: ResourceGeneration::new(7).expect("generation"),
            process_class,
            provider_ref: ResourceRef::parse(PROVIDER_REF).expect("provider ref"),
            launch: launch_identity(owner, process_name, template),
            zone_uid: None,
            policy_revision: None,
            provider_assignment_generation: None,
            controller_generation: ControllerGeneration::new(1).expect("controller generation"),
            controller_provider_uid: None,
            controller_provider_generation: None,
            guest_execution: None,
            worker_launch: None,
            device_worker_launch: None,
        }
    }

    /// A controller-class row owned by a Provider: the row shape the
    /// controller ticket needs its owner's committed identity for.
    fn controller_identity() -> ProcessResourceIdentity {
        identity(
            "Process/controller",
            "controller",
            "reaction",
            "Provider/network-local",
            ProcessClass::Controller,
        )
    }

    /// A Guest-owned guest-runtime row: its launch targets its owning Guest.
    fn guest_vmm_identity() -> ProcessResourceIdentity {
        identity(
            "Process/acceptance-guest-vmm",
            "acceptance-guest-vmm",
            "cloud-hypervisor-runner",
            "Guest/acceptance-guest",
            ProcessClass::Worker,
        )
    }

    /// A committed-Provider identity source double for the context-builder
    /// tests: publishes the rows the test registers, exactly like the
    /// daemon's `PlaneResourceRegistry` view does.
    #[derive(Default)]
    struct TestCommittedIdentities {
        rows: Mutex<BTreeMap<String, (ResourceUid, ResourceGeneration)>>,
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    impl TestCommittedIdentities {
        fn publish(&self, provider: &str, uid: &str, generation: u64) {
            self.rows.lock().insert(
                provider.to_owned(),
                (
                    ResourceUid::parse(uid).expect("provider uid"),
                    ResourceGeneration::new(generation).expect("provider generation"),
                ),
            );
        }
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    impl CommittedProviderIdentitySource for TestCommittedIdentities {
        fn committed_provider_identity(
            &self,
            provider: &ResourceRef,
        ) -> Option<(ResourceUid, ResourceGeneration)> {
            self.rows
                .lock()
                .get(&provider.to_canonical_string())
                .cloned()
        }
    }

    // -- the hosted `has-active` method --------------------------------------

    /// A runtime-facet double answering the retained-identity report.
    struct ScriptedRuntime {
        active: bool,
    }

    impl ScriptedRuntime {
        fn shared(active: bool) -> Arc<Self> {
            Arc::new(Self { active })
        }
    }

    fn facets(runtime: Arc<ScriptedRuntime>) -> ProcessEffectFacets {
        ProcessEffectFacets {
            runtime,
            committed: None,
            guest_owners: None,
        }
    }

    #[async_trait::async_trait]
    impl ProcessProviderRuntime for ScriptedRuntime {
        fn bundle(&self) -> &d2b_core::bundle_resolver::BundleResolver {
            unreachable!("the has-active surface reads no bundle")
        }
        fn socket_runtime_dir(&self) -> &std::path::Path {
            unreachable!("the has-active surface reads no socket runtime dir")
        }
        fn guest_setup_descriptor_digest(
            &self,
            _zone: &ZoneId,
            _guest_ref: &ResourceRef,
        ) -> Option<SchemaFingerprint> {
            None
        }
        async fn resolve_device_worker_launch(
            &self,
            _ctx: &mut ResourceContext,
            _identity: &ProcessResourceIdentity,
            _spec: &ProcessFamilySpec,
        ) -> Result<Option<DeviceWorkerLaunch>, &'static str> {
            unreachable!("the has-active surface never resolves a device worker")
        }
        async fn launch_resource(
            &self,
            _context: ProcessResourceContext<'_>,
            _spec: &ProcessSpec,
            _timeout: Duration,
        ) -> Result<crate::facets::ProviderLaunch, String> {
            unreachable!("the has-active surface never launches")
        }
        async fn launch_ephemeral_resource(
            &self,
            _context: ProcessResourceContext<'_>,
            _spec: &EphemeralProcessSpec,
            _timeout: Duration,
        ) -> Result<crate::facets::ProviderLaunch, String> {
            unreachable!("the has-active surface never launches")
        }
        async fn adopt_resource(
            &self,
            _context: ProcessResourceContext<'_>,
            _spec: &ProcessSpec,
        ) -> Result<ProviderAdoption, String> {
            unreachable!("the has-active surface never adopts")
        }
        async fn probe_resource(
            &self,
            _context: ProcessResourceContext<'_>,
            _spec: &ProcessSpec,
        ) -> Result<ProviderLiveness, String> {
            unreachable!("the has-active surface never probes")
        }
        async fn adopt_ephemeral_resource(
            &self,
            _context: ProcessResourceContext<'_>,
            _spec: &EphemeralProcessSpec,
        ) -> Result<ProviderAdoption, String> {
            unreachable!("the has-active surface never adopts")
        }
        async fn probe_ephemeral_resource(
            &self,
            _context: ProcessResourceContext<'_>,
            _spec: &EphemeralProcessSpec,
        ) -> Result<ProviderLiveness, String> {
            unreachable!("the has-active surface never probes")
        }
        async fn stop_resource(
            &self,
            _context: ProcessResourceContext<'_>,
            _spec: &ProcessSpec,
            _term_timeout: Duration,
            _kill_timeout: Duration,
        ) -> Result<bool, String> {
            unreachable!("the has-active surface never stops")
        }
        async fn stop_ephemeral_resource(
            &self,
            _context: ProcessResourceContext<'_>,
            _spec: &EphemeralProcessSpec,
            _term_timeout: Duration,
            _kill_timeout: Duration,
        ) -> Result<bool, String> {
            unreachable!("the has-active surface never stops")
        }
        async fn stop_stale_resource(
            &self,
            _provider_ref: &ResourceRef,
            _candidate: &AdoptionCandidate,
        ) -> Result<(), String> {
            unreachable!("the has-active surface never stops")
        }
        async fn finalize_resource(
            &self,
            _context: ProcessResourceContext<'_>,
        ) -> Result<(), String> {
            unreachable!("the has-active surface never finalizes")
        }
        fn has_active_resource_in_zone(
            &self,
            zone: &ZoneId,
            zone_uid: Option<&ResourceUid>,
            resource_ref: &ResourceRef,
        ) -> bool {
            assert_eq!(zone.as_str(), "work");
            assert_eq!(
                zone_uid, None,
                "the host supplies no zone-authority uid on the capability"
            );
            assert_eq!(resource_ref.to_canonical_string(), "Process/acceptance-guest-vmm");
            self.active
        }
    }

    fn has_active_invocation<'a>(
        payload: &'a CanonicalJsonObject,
        resources: &'a mut ServiceResourceContext,
        invocation_id: &'a str,
    ) -> ServiceInvocation<'a> {
        ServiceInvocation {
            zone: "work",
            method: "has-active",
            invocation_id,
            payload,
            resources,
            state_cells: &[],
            kernel: None,
            request_fds: &[],
            response_fds: PROCESS_EFFECTS_SERVICE.methods[0].response_fds,
            payload_schema: None,
            chain_identities: &[],
        }
    }

    fn canonical(payload: serde_json::Value) -> CanonicalJsonObject {
        serde_json::from_value(payload).expect("canonical payload")
    }

    /// The hosted `has-active` method answers from the runtime facet for the
    /// invocation's authoritative zone and the resource reference the
    /// payload names, with the host-supplied (un-scoped) zone-authority
    /// uid - the same report the driver's admission fence reads.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn has_active_answers_from_the_runtime_facet() {
        let service = ProcessEffectsService::new(facets(ScriptedRuntime::shared(true)));
        let payload = canonical(serde_json::json!({
            "zone": "work",
            "zoneUid": serde_json::Value::Null,
            "resourceRef": "Process/acceptance-guest-vmm",
        }));
        let mut resources = ServiceResourceContext::fail_closed();
        let response = service
            .handle(has_active_invocation(&payload, &mut resources, "invocation-9"))
            .await
            .expect("served");
        assert_eq!(
            response.payload,
            canonical(serde_json::json!({ "active": true })),
        );
    }

    /// A payload that does not match the method's contract refuses with its
    /// own closed code instead of answering a half-built report.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn has_active_refuses_an_invalid_payload() {
        let service = ProcessEffectsService::new(facets(ScriptedRuntime::shared(true)));
        for (payload, code) in [
            (serde_json::json!({}), "has-active-zone-missing"),
            (
                serde_json::json!({ "zone": 7 }),
                "has-active-zone-missing",
            ),
            (
                serde_json::json!({ "zone": "not a zone" }),
                "has-active-zone-invalid",
            ),
            (
                serde_json::json!({ "zone": "work", "zoneUid": "bogus" }),
                "has-active-zone-uid-unsupplied",
            ),
            (
                serde_json::json!({ "zone": "work", "zoneUid": 7 }),
                "has-active-zone-uid-unsupplied",
            ),
            (
                serde_json::json!({ "zone": "work" }),
                "has-active-resource-ref-missing",
            ),
            (
                serde_json::json!({ "zone": "work", "resourceRef": "not a ref" }),
                "has-active-resource-ref-invalid",
            ),
        ] {
            let payload = canonical(payload);
            let mut resources = ServiceResourceContext::fail_closed();
            let error = service
                .handle(has_active_invocation(&payload, &mut resources, "invocation-10"))
                .await
                .expect_err("refused");
            assert_eq!(
                error,
                EffectServiceError::Declined {
                    service: PROCESS_EFFECTS_SERVICE.id.to_owned(),
                    reason: code.to_owned(),
                },
                "payload {payload:?}"
            );
        }
    }

    /// A payload naming another zone than the invocation is addressed to is
    /// refused by name: the invocation's zone is the authoritative one the
    /// host addressed, never a caller-chosen zone.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn has_active_refuses_a_payload_naming_another_zone() {
        let service = ProcessEffectsService::new(facets(ScriptedRuntime::shared(true)));
        let payload = canonical(serde_json::json!({
            "zone": "other",
            "zoneUid": serde_json::Value::Null,
            "resourceRef": "Process/acceptance-guest-vmm",
        }));
        let mut resources = ServiceResourceContext::fail_closed();
        let error = service
            .handle(has_active_invocation(&payload, &mut resources, "invocation-11"))
            .await
            .expect_err("refused");
        assert_eq!(
            error,
            EffectServiceError::Declined {
                service: PROCESS_EFFECTS_SERVICE.id.to_owned(),
                reason: "has-active-zone-mismatch".to_owned(),
            }
        );
    }

    // -- committed controller-provider identity -------------------------------

    /// A controller row owned by a Provider takes the owner's committed
    /// uid/generation into the provider context, so the controller bootstrap
    /// ticket forms instead of refusing with
    /// `provider-controller-provider-identity-missing`.
    #[test]
    fn controller_provider_identity_binds_the_committed_provider_row() {
        let identity = controller_identity();
        let source = TestCommittedIdentities::default();
        source.publish(
            "Provider/network-local",
            COMMITTED_PROVIDER_UID,
            COMMITTED_PROVIDER_GENERATION,
        );
        let context =
            bind_committed_controller_provider_identity(&identity, Some(&source));
        assert_eq!(
            context.provider_uid.as_ref().map(ResourceUid::as_str),
            Some(COMMITTED_PROVIDER_UID)
        );
        assert_eq!(
            context.provider_generation,
            Some(ResourceGeneration::new(COMMITTED_PROVIDER_GENERATION).expect("generation"))
        );
    }

    /// A Provider the daemon retains no committed row for - and an unwired
    /// production effects value - leaves the identity unbound, so the ticket
    /// path still refuses closed instead of inventing an identity.
    #[test]
    fn controller_provider_identity_stays_unbound_without_a_committed_row() {
        let identity = controller_identity();
        let empty = TestCommittedIdentities::default();
        let unretained = bind_committed_controller_provider_identity(
            &identity,
            Some(&empty),
        );
        assert_eq!(unretained.provider_uid, None);
        assert_eq!(unretained.provider_generation, None);

        let unwired = bind_committed_controller_provider_identity(&identity, None);
        assert_eq!(unwired.provider_uid, None);
        assert_eq!(unwired.provider_generation, None);
    }

    // -- catalog-bound Guest setup descriptor digest -------------------------

    /// The catalog digest the bundle resolves for one guest.
    fn guest_descriptor_digest() -> SchemaFingerprint {
        SchemaFingerprint::parse(format!("sha256:{}", "a".repeat(64))).expect("guest digest")
    }

    /// The launch ticket for a Guest-owned guest-runtime Process must carry
    /// the owner identity the old descriptor composer produced for the same
    /// row: the authored `owner_ref`, the durable owner uid the pre-v3 plane
    /// resolved from it, and the owning Guest as the cross-target selector.
    ///
    /// The manager row cannot carry the linkage for an unconverted owner
    /// (`Guest` stays on the pre-v3 plane), so the process effects resolve the
    /// same durable uid from that plane.
    #[test]
    fn guest_vmm_ticket_carries_the_old_descriptor_owner_identity() {
        let guest_ref = ResourceRef::parse("Guest/acceptance-guest").expect("guest ref");
        let guest_uid = ResourceUid::parse(GUEST_UID).expect("guest uid");
        let identity = guest_vmm_identity();
        assert_eq!(
            identity.launch.owner_uid(),
            None,
            "a manager row cannot link an unconverted Guest owner"
        );

        let context = process_resource_context(&identity, None, Some(&guest_uid), |_, _| None);

        assert_eq!(context.owner_ref.as_ref(), Some(&guest_ref));
        assert_eq!(context.owner_uid.as_ref(), Some(&guest_uid));
        assert_eq!(context.target_ref.as_ref(), Some(&guest_ref));
    }

    /// The old runner bound the bundle's Guest setup descriptor digest for
    /// guest-owned rows (`set_guest_descriptor_digests`); the private guest VMM
    /// intent lookup refuses a ticket without it
    /// (`provider-ticket:guest-descriptor-unbound`), so the descriptor must
    /// reach the provider context.
    #[test]
    fn guest_owned_row_binds_the_catalog_guest_descriptor_digest() {
        let identity = guest_vmm_identity();
        let digest = guest_descriptor_digest();
        let consulted = std::cell::Cell::new(false);
        let context = process_resource_context(&identity, None, None, |zone, guest| {
            consulted.set(true);
            assert_eq!(zone.as_str(), "work");
            assert_eq!(guest.name().as_str(), "acceptance-guest");
            Some(digest.clone())
        });
        assert!(
            consulted.get(),
            "a guest-owned row must resolve its descriptor from the bundle"
        );
        assert_eq!(context.guest_descriptor_digest.as_ref(), Some(&digest));
    }

    /// Non-guest rows never consult the bundle descriptor source, so the
    /// context keeps the descriptor slot unbound.
    #[test]
    fn non_guest_rows_keep_the_guest_descriptor_digest_unbound() {
        let identity = controller_identity();
        let context = process_resource_context(&identity, None, None, |_, _| {
            panic!("a Provider-owned row must not consult a Guest descriptor")
        });
        assert_eq!(context.guest_descriptor_digest, None);
    }

    /// A Guest the bundle retains no descriptor for stays unbound - the
    /// ticket path still refuses closed instead of inventing a digest.
    #[test]
    fn missing_catalog_descriptor_keeps_the_guest_digest_unbound() {
        let identity = guest_vmm_identity();
        let context = process_resource_context(&identity, None, None, |_, _| None);
        assert_eq!(context.guest_descriptor_digest, None);
    }

    // -- Device-worker derivation --------------------------------------------
    //
    // The Device-family-specific resolution (the owning Device's declared
    // settings, the controller-created state Volume, the projected Wayland
    // socket) is owned by the daemon host behind the facet boundary and is
    // tested in `d2bd`'s `process_provider_runtime` module, where the daemon
    // may name the device families. This crate's seam delegates and never
    // names a device family.
}