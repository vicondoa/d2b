//! The provider-owned implementation of the Activation family's driver
//! effects: the family serves its effects from this crate instead of a
//! daemon-built port.
//!
//! Two surfaces share one implementation value:
//!
//! - the driver's typed seam, [`ActivationDriverEffects`], which the
//!   family's driver holds (the factory builds it from the same facets). Its
//!   one method (`apply_host_generation_handoff`) dispatches the preserved
//!   `ApplyHostGenerationHandoff` broker request - caller role `Lifecycle`
//!   on the typed request, admin-uid daemon caller on the dispatch -
//!   through the daemon-supplied dispatch facet and reduces the response to
//!   the closed result the driver's outcome mapping reads;
//! - the declared zone-plane service [`ACTIVATION_EFFECTS_SERVICE`], hosted
//!   per zone by the daemon through [`ActivationEffectsServiceFactory`]. Its
//!   one method (`inspect-activation`) answers the family's committed
//!   surface: the resource type it serves, the runner creation it declares,
//!   the handoff operation it drives, and the declared runner steps.
//!
//! Everything the effects read crosses the provider boundary as declared
//! facets ([`crate::facets`]): the one daemon-owned capability - the broker
//! dispatch over the daemon's authenticated origination socket - arrives
//! through the composition-supplied facet, and every other input is the
//! family's own committed vocabulary. Nothing here names a daemon state type.

use std::sync::Arc;

use async_trait::async_trait;
use d2b_contracts_broker::broker_wire::ApplyHostGenerationHandoffResponse;
use d2b_contracts_broker::host_generation::{
    ApplyHostGenerationHandoff, HandoffCallerRole, HandoffState, HostGenerationHandoffIntent,
};
use d2b_contracts_resource::v3::ResourceRef;
use d2b_provider_toolkit::{
    EffectResponse, EffectService, EffectServiceError, EffectServiceFactory, ServiceInvocation,
};
use d2b_resource_types::{ServiceDecl, ServiceMethod};

use crate::driver::{ActivationDriverEffects, HostHandoffResult};
use crate::facets::{ActivationBrokerDispatch, ActivationEffectFacets};
use crate::{ACTIVATION_RUNNER_STEPS, ACTIVATION_TYPE_NAME, RUNNER_PROVIDER_REF, RUNNER_TYPE_NAME};

/// The Activation family's declared effects service.
///
/// One zone-plane method, `inspect-activation`: it answers the family's
/// committed surface - the resource type it serves, the runner creation it
/// declares, the handoff operation it drives, and the declared runner steps.
/// The report is served from the crate's own committed vocabulary, so it
/// proves the family's effects run inside the owning crate; it is hermetic:
/// no host state is read or mutated.
///
/// The service is declared on the `NixosGeneration` descriptor alone; the
/// family's driver effects (the typed seam) stay the driver's object, not a
/// hosted method surface.
pub const ACTIVATION_EFFECTS_SERVICE: ServiceDecl = ServiceDecl {
    id: "activation.d2bus.org/effects",
    methods: &[ServiceMethod::zone_plane("inspect-activation")],
    attach_kinds: &[],
    streams: &[],
    endpoint_policy: None,
};

/// The one `inspect-activation` response payload: the family's committed
/// surface. The payload is built through the canonical JSON object path, so
/// a structural character in a committed value yields a correctly escaped
/// report rather than an unparseable one; the refusal is unreachable and
/// names its own code.
fn inspect_activation_response() -> Result<EffectResponse, EffectServiceError> {
    let payload = serde_json::from_value(serde_json::json!({
        "family": "activation-nixos",
        "resourceType": ACTIVATION_TYPE_NAME,
        "runner": {
            "providerRef": RUNNER_PROVIDER_REF,
            "type": RUNNER_TYPE_NAME,
        },
        "handoffOperation": "ApplyHostGenerationHandoff",
        "runnerSteps": ACTIVATION_RUNNER_STEPS
            .iter()
            .map(|step| step.label)
            .collect::<Vec<_>>(),
    }))
    .map_err(|_| EffectServiceError::Declined {
        service: ACTIVATION_EFFECTS_SERVICE.id.to_owned(),
        reason: "inspect-activation-response-invalid".to_owned(),
    })?;
    Ok(EffectResponse::new(payload))
}

/// Serve the `inspect-activation` method: answer the family's committed
/// surface from its own declared vocabulary.
async fn serve_inspect_activation() -> Result<EffectResponse, EffectServiceError> {
    inspect_activation_response()
}

/// The provider-owned Activation effects, built from the daemon-supplied
/// facets.
///
/// One value serves both the driver's typed seam and the declared hosted
/// service: the factory constructs it from the same [`ActivationEffectFacets`]
/// the composition root supplies, so the hosted surface and the driver
/// dispatch through the same broker boundary.
pub struct ActivationEffectsService {
    broker: Arc<dyn ActivationBrokerDispatch>,
}

impl ActivationEffectsService {
    /// Build the effects over the daemon-supplied dispatch facet (R2): every
    /// daemon-structural read rides the facets, never a daemon handle.
    pub fn new(facets: ActivationEffectFacets) -> Self {
        Self {
            broker: facets.broker,
        }
    }

    /// Build the effects over a scripted broker dispatch (test-support
    /// only): the same [`ActivationBrokerDispatch`] surface the daemon's
    /// production dispatch implements, so the completed, refused, rolled
    /// back, and failed handoff paths are testable hermetically.
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_broker(broker: Arc<dyn ActivationBrokerDispatch>) -> Self {
        Self { broker }
    }
}

#[async_trait]
impl ActivationDriverEffects for ActivationEffectsService {
    async fn apply_host_generation_handoff(
        &self,
        target: ResourceRef,
        intent: HostGenerationHandoffIntent,
    ) -> HostHandoffResult {
        let request = ApplyHostGenerationHandoff {
            caller_role: HandoffCallerRole::Lifecycle,
            target,
            intent,
        };
        match self.broker.dispatch_handoff(request) {
            Ok(response) => host_handoff_result(&response),
            Err(_) => HostHandoffResult::Incomplete,
        }
    }
}

/// Preserved outcome mapping: a recorded completion is success unless the
/// coordinator reported no strict generation transition (old
/// `host_handoff_result` in the daemon's activation effects).
fn host_handoff_result(response: &ApplyHostGenerationHandoffResponse) -> HostHandoffResult {
    match response.state {
        HandoffState::Completed => HostHandoffResult::Completed {
            source_generation: response.source_generation,
            target_generation: response.target_generation,
        },
        HandoffState::Refused => HostHandoffResult::Refused,
        HandoffState::RolledBack => HostHandoffResult::RolledBack,
        _ => HostHandoffResult::Incomplete,
    }
}

#[async_trait]
impl EffectService for ActivationEffectsService {
    async fn handle(
        &self,
        _invocation: ServiceInvocation<'_>,
    ) -> Result<EffectResponse, EffectServiceError> {
        // The declaration's method gates admission at the hosting side; the
        // service serves its one declared zone-plane method from the
        // family's committed surface.
        serve_inspect_activation().await
    }
}

/// The composition-root factory that hosts the Activation effects service in
/// one zone (R5): the daemon registers one per zone, carrying that zone's
/// facet set, and the host rebuilds the service from it on respawn.
pub struct ActivationEffectsServiceFactory {
    facets: ActivationEffectFacets,
}

impl ActivationEffectsServiceFactory {
    /// Build the factory from one zone's facet set.
    pub fn new(facets: ActivationEffectFacets) -> Self {
        Self { facets }
    }
}

impl EffectServiceFactory for ActivationEffectsServiceFactory {
    fn build(&self) -> Arc<dyn EffectService> {
        Arc::new(ActivationEffectsService::new(self.facets.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use d2b_contracts_broker::host_generation::SourceGenerationCompatibilityFloorV1;
    use d2b_contracts_resource::v3::{ActivationMode, ArtifactId, CanonicalJsonObject};

    use crate::test_support::{RecordingBrokerDispatch, recording_facets};

    fn handoff_intent() -> HostGenerationHandoffIntent {
        HostGenerationHandoffIntent {
            source_generation: 1,
            target_generation: 2,
            system_artifact_id: ArtifactId::parse("system-artifact").expect("artifact"),
            activation_mode: ActivationMode::Switch,
            compatibility: SourceGenerationCompatibilityFloorV1::new(1, [1u8; 32])
                .expect("compatibility floor"),
        }
    }

    fn handoff_response(
        state: HandoffState,
        source_generation: u64,
        target_generation: u64,
    ) -> ApplyHostGenerationHandoffResponse {
        ApplyHostGenerationHandoffResponse {
            target: ResourceRef::parse("Host/host-system").expect("ref"),
            state,
            source_generation,
            target_generation,
            source_remains_usable: false,
            summary: "scripted".to_owned(),
        }
    }

    #[tokio::test]
    async fn a_completed_handoff_reduces_to_the_closed_completed_result() {
        let broker = RecordingBrokerDispatch::with_responses(vec![Ok(handoff_response(
            HandoffState::Completed,
            3,
            4,
        ))]);
        let service = ActivationEffectsService::with_broker(broker.clone());
        let result = service
            .apply_host_generation_handoff(
                ResourceRef::parse("Host/host-system").expect("ref"),
                handoff_intent(),
            )
            .await;
        assert_eq!(
            result,
            HostHandoffResult::Completed {
                source_generation: 3,
                target_generation: 4,
            }
        );
    }

    #[tokio::test]
    async fn a_refused_handoff_projects_helper_refused_without_a_completion() {
        let broker = RecordingBrokerDispatch::with_responses(vec![Ok(handoff_response(
            HandoffState::Refused,
            0,
            0,
        ))]);
        let service = ActivationEffectsService::with_broker(broker.clone());
        let result = service
            .apply_host_generation_handoff(
                ResourceRef::parse("Host/host-system").expect("ref"),
                handoff_intent(),
            )
            .await;
        assert_eq!(result, HostHandoffResult::Refused);
    }

    #[tokio::test]
    async fn a_rolled_back_handoff_projects_rolled_back() {
        let broker = RecordingBrokerDispatch::with_responses(vec![Ok(handoff_response(
            HandoffState::RolledBack,
            0,
            0,
        ))]);
        let service = ActivationEffectsService::with_broker(broker.clone());
        let result = service
            .apply_host_generation_handoff(
                ResourceRef::parse("Host/host-system").expect("ref"),
                handoff_intent(),
            )
            .await;
        assert_eq!(result, HostHandoffResult::RolledBack);
    }

    #[tokio::test]
    async fn a_non_terminal_or_failed_dispatch_projects_incomplete() {
        for scripted in [
            Ok(handoff_response(HandoffState::Recorded, 0, 0)),
            Err("dispatch-failed".to_owned()),
        ] {
            let broker = RecordingBrokerDispatch::with_responses(vec![scripted]);
            let service = ActivationEffectsService::with_broker(broker.clone());
            let result = service
                .apply_host_generation_handoff(
                    ResourceRef::parse("Host/host-system").expect("ref"),
                    handoff_intent(),
                )
                .await;
            assert_eq!(result, HostHandoffResult::Incomplete);
        }
    }

    #[tokio::test]
    async fn the_dispatched_request_carries_the_lifecycle_caller_role() {
        let broker = RecordingBrokerDispatch::with_responses(vec![Err("unused".to_owned())]);
        let service = ActivationEffectsService::with_broker(broker.clone());
        let target = ResourceRef::parse("Host/host-system").expect("ref");
        let intent = handoff_intent();
        service
            .apply_host_generation_handoff(target.clone(), intent.clone())
            .await;
        let requests = broker.requests();
        assert_eq!(requests.len(), 1);
        let handoff = &requests[0];
        assert_eq!(handoff.caller_role, HandoffCallerRole::Lifecycle);
        assert_eq!(handoff.target, target);
        assert_eq!(handoff.intent, intent);
    }

    /// The hosted surface serves the family's committed surface: the plane
    /// hosts the service through the factory and the one declared method
    /// answers the report from the crate's own vocabulary.
    #[tokio::test]
    async fn the_hosted_method_answers_the_family_committed_surface() {
        let facets = recording_facets(RecordingBrokerDispatch::new());
        let factory = ActivationEffectsServiceFactory::new(facets);
        let service = factory.build();
        let response = service
            .handle(ServiceInvocation {
                zone: "test",
                method: "inspect-activation",
                invocation_id: "invocation-inspect-activation",
                payload: &serde_json::from_value::<CanonicalJsonObject>(serde_json::json!({}))
                    .expect("canonical payload"),
                resources: &mut d2b_resource_runtime::context::ServiceResourceContext::fail_closed(),
                state_cells: &[],
                kernel: None,
                request_fds: &[],
                response_fds: d2b_resource_types::MethodFdContract::NONE,
                payload_schema: None,
                chain_identities: &[],
            })
            .await
            .expect("the report serves");
        let payload = serde_json::from_value::<CanonicalJsonObject>(serde_json::json!({
            "family": "activation-nixos",
            "resourceType": ACTIVATION_TYPE_NAME,
            "runner": {
                "providerRef": RUNNER_PROVIDER_REF,
                "type": RUNNER_TYPE_NAME,
            },
            "handoffOperation": "ApplyHostGenerationHandoff",
            "runnerSteps": ["switch", "boot", "test"],
        }))
        .expect("expected payload");
        assert_eq!(response.payload, payload);
    }
}