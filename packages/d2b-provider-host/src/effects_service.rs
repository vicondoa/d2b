//! The provider-owned implementation of the Host family's driver effects
//! (U5): the family serves its effects from this crate instead of a
//! daemon-built port.
//!
//! Two surfaces share one implementation value:
//!
//! - the driver's typed seam, [`HostDriverEffects`], which the family's
//!   driver holds (the factory builds it from the same facets). Its one
//!   method (`observe_host`) runs the bounded probe through the preserved
//!   `HostReconciler` with the preserved degraded fallback: a probe that
//!   cannot complete still publishes the spec decision as a degraded
//!   observation rather than failing the resource;
//! - the declared zone-plane service [`HOST_EFFECTS_SERVICE`], hosted per
//!   zone by the daemon through [`HostEffectsServiceFactory`]. Its one
//!   method (`inspect-host`) answers the family's bounded host observations:
//!   the capability classes, the bounded metadata, and the minijail platform
//!   gate - the same probe the driver effects reconcile over.
//!
//! Everything the effects read crosses the provider boundary as declared
//! facets ([`crate::facets`]): the one daemon-owned read - the minijail
//! platform gate - arrives through the composition-supplied facet, and every
//! other probe input is host state this crate reads itself. Nothing here
//! names a daemon state type.

use std::collections::BTreeSet;
use std::sync::Arc;

use async_trait::async_trait;
use d2b_contracts_resource::v3::host::HostSpec;
use d2b_contracts_resource::v3::{ResourcePhase, ResourceRef};
use d2b_provider_system_core::{
    HostCapabilityClass, HostObservationReport, HostProbeEffectPort, HostProbeMetadata,
    HostReconciler, MinijailPlatformGate,
};
use d2b_provider_toolkit::{
    EffectResponse, EffectService, EffectServiceError, EffectServiceFactory, ServiceInvocation,
};
use d2b_resource_types::{ServiceDecl, ServiceMethod};

use crate::driver::{HostDriverEffects, ObserveError};
use crate::facets::HostEffectFacets;

/// The Host family's declared effects service.
///
/// One zone-plane method, `inspect-host`: it answers this family's bounded
/// host observations - the capability classes present, the bounded
/// kernel/os-release metadata, the user-manager and process-count
/// observations, and the minijail platform gate - the same probe the
/// driver's `observe_host` reconciles over. The report is served from the
/// crate's own probe, so it proves the family's probe runs inside the
/// owning crate (U5); a probe that cannot complete refuses with its own
/// closed code instead of answering a half-built report.
///
/// The service is declared on the `Host` descriptor alone; the family's
/// driver effects (the typed seam) stay the driver's object, not a hosted
/// method surface.
pub const HOST_EFFECTS_SERVICE: ServiceDecl = ServiceDecl {
    id: "host.d2bus.org/effects",
    methods: &[ServiceMethod::zone_plane("inspect-host")],
    attach_kinds: &[],
    streams: &[],
    endpoint_policy: None,
};

/// The one `inspect-host` response payload: the family's bounded host
/// observations. The payload is built through the canonical JSON object
/// path, so a structural character in a bounded observation yields a
/// correctly escaped report rather than an unparseable one; the refusal is
/// unreachable and names its own code.
fn inspect_host_response(
    capabilities: &[HostCapabilityClass],
    metadata: &HostProbeMetadata,
    gate: MinijailPlatformGate,
) -> Result<EffectResponse, EffectServiceError> {
    let payload = serde_json::from_value(serde_json::json!({
        "family": "host",
        "resourceType": "Host",
        "kernelRelease": metadata.kernel_release,
        "osName": metadata.os_name,
        "userManagerAvailable": metadata.user_manager_available,
        "activeProcessCount": metadata.active_process_count,
        "minijail": {
            "kernelMajor": gate.kernel_major,
            "kernelMinor": gate.kernel_minor,
            "cgroupKillAvailable": gate.cgroup_kill_available,
        },
        "minijailReady": gate.kernel_supported() && gate.cgroup_kill_available,
        "capabilities": capabilities,
    }))
    .map_err(|_| EffectServiceError::Declined {
        service: HOST_EFFECTS_SERVICE.id.to_owned(),
        reason: "inspect-host-response-invalid".to_owned(),
    })?;
    Ok(EffectResponse::new(payload))
}

/// Serve the `inspect-host` method: run the family's bounded probe and
/// answer its observations. A probe that cannot complete refuses with its
/// own closed code instead of answering a half-built report (the degraded
/// fallback stays on the driver seam, where the row's spec decision is
/// available).
async fn serve_inspect_host(
    probe: &dyn HostProbeEffectPort,
) -> Result<EffectResponse, EffectServiceError> {
    let declined = |reason: &'static str| EffectServiceError::Declined {
        service: HOST_EFFECTS_SERVICE.id.to_owned(),
        reason: reason.to_owned(),
    };
    let mut capabilities = Vec::new();
    for capability in HostCapabilityClass::ALL {
        if probe
            .probe(capability)
            .await
            .map_err(|_| declined("inspect-host-probe-failed"))?
        {
            capabilities.push(capability);
        }
    }
    let metadata = probe
        .metadata()
        .await
        .map_err(|_| declined("inspect-host-probe-failed"))?;
    let gate = probe
        .platform()
        .await
        .map_err(|_| declined("inspect-host-probe-failed"))?;
    inspect_host_response(&capabilities, &metadata, gate)
}

/// The provider-owned Host effects (U5), built from the daemon-supplied
/// facets.
///
/// One value serves both the driver's typed seam and the declared hosted
/// service: the factory constructs it from the same [`HostEffectFacets`]
/// the composition root supplies, so the hosted surface and the driver
/// observe the same probe.
///
/// The probe is the facet-carried [`HostProbeEffectPort`] trait object: the
/// crate's production probe in production, a scripted double in tests.
pub struct HostEffectsService {
    probe: Arc<dyn HostProbeEffectPort>,
}

impl HostEffectsService {
    /// Build the effects from one zone's daemon-supplied facet set (R2):
    /// every daemon-structural read rides the facets, never a daemon handle.
    pub fn new(facets: HostEffectFacets) -> Self {
        Self {
            probe: facets.probe,
        }
    }
}

#[async_trait]
impl HostDriverEffects for HostEffectsService {
    async fn observe_host(
        &self,
        host_ref: &ResourceRef,
        provider_ref: &ResourceRef,
        spec: &HostSpec,
    ) -> Result<HostObservationReport, ObserveError> {
        match HostReconciler::new()
            .reconcile_with_probe(
                host_ref,
                provider_ref,
                spec,
                &*self.probe,
                &BTreeSet::new(),
                false,
            )
            .await
        {
            Ok(report) => Ok(report),
            Err(probe_error) => {
                // Preserved fallback: a probe that cannot complete still
                // publishes the spec decision as a degraded observation
                // rather than failing the resource.
                let mut status = HostReconciler::new()
                    .reconcile(host_ref, provider_ref, spec)
                    .map_err(|error| ObserveError { probe: probe_error, fallback: Some(error) })?;
                status.phase = ResourcePhase::Degraded;
                Ok(HostObservationReport {
                    status,
                    capabilities: Vec::new(),
                    kernel_release: "unknown".to_owned(),
                    os_name: "unknown".to_owned(),
                    user_manager_available: false,
                    active_process_count: 0,
                    minijail_ready: false,
                })
            }
        }
    }
}

#[async_trait]
impl EffectService for HostEffectsService {
    async fn handle(
        &self,
        _invocation: ServiceInvocation<'_>,
    ) -> Result<EffectResponse, EffectServiceError> {
        // The declaration's method gates admission at the hosting side; the
        // service serves its one declared zone-plane method from the
        // family's bounded probe.
        serve_inspect_host(&*self.probe).await
    }
}

/// The composition-root factory that hosts the Host effects service in one
/// zone (R5): the daemon registers one per zone, carrying that zone's facet
/// set, and the host rebuilds the service from it on respawn.
pub struct HostEffectsServiceFactory {
    facets: HostEffectFacets,
}

impl HostEffectsServiceFactory {
    /// Build the factory from one zone's facet set.
    pub fn new(facets: HostEffectFacets) -> Self {
        Self { facets }
    }
}

impl EffectServiceFactory for HostEffectsServiceFactory {
    fn build(&self) -> Arc<dyn EffectService> {
        Arc::new(HostEffectsService::new(self.facets.clone()))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    use d2b_contracts_resource::v3::host::{HOST_PROVIDER_REF, HostSpec};
    use d2b_provider_system_core::HostCapabilityClass;

    use crate::test_support::{RecordingProbe, scripted_facets};

    fn host_ref() -> ResourceRef {
        ResourceRef::parse("Host/host-system").expect("host ref")
    }

    fn provider_ref() -> ResourceRef {
        ResourceRef::parse(HOST_PROVIDER_REF).expect("provider ref")
    }

    fn system_spec() -> HostSpec {
        HostSpec::system_default()
    }

    /// The service over a scripted probe carried by the facet set, exactly
    /// as the composition root builds it from the production probe: the
    /// same `HostProbeEffectPort` surface production's `HostProbe`
    /// implements.
    fn service(probe: Arc<RecordingProbe>) -> HostEffectsService {
        HostEffectsService::new(scripted_facets(probe))
    }

    // -- driver seam: happy and degraded observation parity -------------------

    /// A host row reconciles from the probe inside the provider crate and
    /// publishes the same observation as before: the report is the
    /// reconciler's typed projection over the probe's bounded observations.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn observe_host_publishes_the_probe_observation() {
        let probe = RecordingProbe::new(vec![HostCapabilityClass::Kvm]);
        let report = service(probe.clone())
            .observe_host(&host_ref(), &provider_ref(), &system_spec())
            .await
            .expect("observe succeeds");
        assert_eq!(report.capabilities, vec![HostCapabilityClass::Kvm]);
        assert_eq!(report.kernel_release, "6.9.0-test");
        assert_eq!(report.os_name, "Linux");
        assert!(report.user_manager_available);
        assert_eq!(report.active_process_count, 3);
        assert!(report.minijail_ready);
        assert_eq!(report.status.phase, ResourcePhase::Ready);
        assert_eq!(
            probe.probed_classes(),
            HostCapabilityClass::ALL.to_vec(),
            "the probe is run once per capability class, exactly as the preserved reconciler does"
        );
    }

    /// Edge: a probe that cannot complete still publishes the degraded
    /// observation rather than failing the resource: the spec decision with
    /// the preserved fallback fields.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn observe_host_publishes_degraded_when_the_probe_cannot_complete() {
        let probe = RecordingProbe::new(Vec::new());
        probe.set_failing(true);
        let report = service(probe)
            .observe_host(&host_ref(), &provider_ref(), &system_spec())
            .await
            .expect("the degraded observation still publishes");
        assert_eq!(report.status.phase, ResourcePhase::Degraded);
        assert!(report.capabilities.is_empty());
        assert_eq!(report.kernel_release, "unknown");
        assert_eq!(report.os_name, "unknown");
        assert!(!report.user_manager_available);
        assert_eq!(report.active_process_count, 0);
        assert!(!report.minijail_ready);
    }

    // -- hosted surface: inspect-host -----------------------------------------

    fn canonical(payload: serde_json::Value) -> d2b_contracts_resource::v3::CanonicalJsonObject {
        serde_json::from_value(payload).expect("canonical payload")
    }

    fn invocation<'a>(
        payload: &'a d2b_contracts_resource::v3::CanonicalJsonObject,
        resources: &'a mut d2b_resource_runtime::context::ServiceResourceContext,
    ) -> ServiceInvocation<'a> {
        ServiceInvocation {
            zone: "work",
            method: HOST_EFFECTS_SERVICE.methods[0].name,
            invocation_id: "invocation-u5",
            payload,
            resources,
            state_cells: &[],
            kernel: None,
            request_fds: &[],
            response_fds: HOST_EFFECTS_SERVICE.methods[0].response_fds,
            payload_schema: None,
            chain_identities: &[],
        }
    }

    /// The hosted `inspect-host` method answers the family's bounded host
    /// observations from the crate's own probe.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn inspect_host_answers_the_bounded_host_observations() {
        let service = service(RecordingProbe::new(vec![
            HostCapabilityClass::Kvm,
            HostCapabilityClass::Pidfd,
        ]));
        let payload = canonical(serde_json::json!({}));
        let mut resources = d2b_resource_runtime::context::ServiceResourceContext::fail_closed();
        let response = service
            .handle(invocation(&payload, &mut resources))
            .await
            .expect("served");
        assert_eq!(
            response.payload,
            canonical(serde_json::json!({
                "family": "host",
                "resourceType": "Host",
                "kernelRelease": "6.9.0-test",
                "osName": "Linux",
                "userManagerAvailable": true,
                "activeProcessCount": 3,
                "minijail": {
                    "kernelMajor": 6,
                    "kernelMinor": 9,
                    "cgroupKillAvailable": true,
                },
                "minijailReady": true,
                "capabilities": ["kvm", "pidfd"],
            })),
        );
    }

    /// A probe that cannot complete refuses with its own closed code
    /// instead of answering a half-built report.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn inspect_host_refuses_when_the_probe_cannot_complete() {
        let probe = RecordingProbe::new(Vec::new());
        probe.set_failing(true);
        let payload = canonical(serde_json::json!({}));
        let mut resources = d2b_resource_runtime::context::ServiceResourceContext::fail_closed();
        let error = service(probe)
            .handle(invocation(&payload, &mut resources))
            .await
            .expect_err("refused");
        assert_eq!(
            error,
            EffectServiceError::Declined {
                service: HOST_EFFECTS_SERVICE.id.to_owned(),
                reason: "inspect-host-probe-failed".to_owned(),
            }
        );
    }

    // -- factory ---------------------------------------------------------------

    /// The composition-root factory rebuilds the same implementation value
    /// from the facet set the driver factory is built from: the built
    /// service answers through the crate's probe over the supplied facets.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn the_factory_builds_the_service_over_the_facets() {
        let facets = crate::test_support::recording_facets(
            crate::test_support::RecordingMinijailGate::new(MinijailPlatformGate::new(6, 9, true)),
        );
        let factory = HostEffectsServiceFactory::new(facets);
        let service = factory.build();
        let payload = canonical(serde_json::json!({}));
        let mut resources = d2b_resource_runtime::context::ServiceResourceContext::fail_closed();
        assert!(
            service.handle(invocation(&payload, &mut resources)).await.is_ok(),
            "the factory-built service serves the declared method"
        );
    }
}
