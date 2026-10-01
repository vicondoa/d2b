//! The provider-owned implementation of the `EndpointBinding` family's
//! driver effects (U6): the family serves its effects from this crate
//! instead of a daemon-built port.
//!
//! Two surfaces share one implementation value:
//!
//! - the driver's typed seam, [`EndpointBindingDriverEffects`], which the
//!   family's driver holds (the factory builds it from the same facets);
//! - the declared zone-plane service [`BINDING_EFFECTS_SERVICE`], hosted
//!   per zone by the daemon through
//!   [`EndpointBindingEffectsServiceFactory`]. Its one method
//!   (`inspect-binding`) answers the family's committed serving contract -
//!   the exact `Endpoint` one binding delivers, the realization facets the
//!   family serves, and the surfaces its effects drive - served from inside
//!   the owning crate.
//!
//! Everything the effects read or mutate crosses the provider boundary as
//! declared facets ([`crate::facets`]). Nothing here names a daemon state
//! type, and the hosted report is hermetic: no host state is read or
//! mutated by it.

use std::sync::Arc;

use async_trait::async_trait;
use d2b_provider_endpoint::{EndpointAccessObservation, EndpointSocketIdentity};
use d2b_provider_toolkit::{
    EffectResponse, EffectService, EffectServiceError, EffectServiceFactory, ServiceInvocation,
};
use d2b_resource_types::{ServiceDecl, ServiceMethod};

use crate::driver::{
    ENDPOINT_BINDING_CREATIONS, ENDPOINT_BINDING_READS, ENDPOINT_BINDING_TYPE_NAME,
    EndpointBindingDelivery, EndpointBindingDriverEffects, EndpointDeliveryTarget,
};
use crate::facets::EndpointBindingEffectFacets;

/// The `EndpointBinding` family's declared effects service.
///
/// One zone-plane method, `inspect-binding`: it answers this family's
/// committed serving contract - the exact endpoint one binding delivers and
/// the realization facets the family's effects realize. The report is served
/// from the crate's own declaration, so it proves the family's serving
/// effects run inside the owning crate (U6), and it is hermetic: no host
/// state is read or mutated.
pub const BINDING_EFFECTS_SERVICE: ServiceDecl = ServiceDecl {
    id: "endpoint-binding.d2bus.org/effects",
    methods: &[ServiceMethod::zone_plane("inspect-binding")],
    attach_kinds: &[],
    streams: &[],
    endpoint_policy: None,
};

/// The realization facets this family serves, as they render in the
/// `inspect-binding` report.
const SERVED_FACETS: [&str; 2] = ["endpoint-descriptor", "endpoint-pathname"];

/// The one `inspect-binding` response payload: the family's committed
/// serving contract. The payload is built through the canonical JSON object
/// path, so a structural character in a committed value yields a correctly
/// escaped report rather than an unparseable one; the refusal is
/// unreachable and names its own code.
fn inspect_binding_response() -> Result<EffectResponse, EffectServiceError> {
    let reads = ENDPOINT_BINDING_READS
        .iter()
        .map(|known| known.to_resource_type_name().as_str().to_owned())
        .collect::<Vec<_>>();
    let creations = ENDPOINT_BINDING_CREATIONS
        .iter()
        .map(|creation| {
            serde_json::json!({
                "child": creation.child.to_resource_type_name().as_str(),
                "providerRef": creation.provider_ref,
                "order": creation.order,
            })
        })
        .collect::<Vec<_>>();
    let payload = serde_json::from_value(serde_json::json!({
        "family": "endpoint-binding",
        "resourceType": ENDPOINT_BINDING_TYPE_NAME,
        "sourceType": "Endpoint",
        "reads": reads,
        "creations": creations,
        "facets": SERVED_FACETS,
        "serving": ["verify", "deliver", "fence", "attached", "release"],
    }))
    .map_err(|_| EffectServiceError::Declined {
        service: BINDING_EFFECTS_SERVICE.id.to_owned(),
        reason: "inspect-binding-response-invalid".to_owned(),
    })?;
    Ok(EffectResponse::new(payload))
}

/// The provider-owned `EndpointBinding` effects (U6), built from the
/// daemon-supplied facets.
///
/// One value serves both the driver's typed seam and the declared hosted
/// service: the factory constructs it from the same
/// [`EndpointBindingEffectFacets`] the composition root supplies, so the
/// hosted surface and the driver observe the same serving adapter.
pub struct EndpointBindingEffectsService {
    verify: Arc<dyn crate::facets::EndpointVerifySource>,
    deliver: Arc<dyn crate::facets::EndpointDeliverSource>,
    fence: Arc<dyn crate::facets::EndpointFenceSource>,
    attached: Arc<dyn crate::facets::EndpointAttachmentSource>,
    release: Arc<dyn crate::facets::EndpointReleaseSource>,
}

impl EndpointBindingEffectsService {
    /// Build the effects from one zone's daemon-supplied facet set (R2):
    /// every daemon-structural read and mutation rides the facets, never a
    /// daemon handle.
    pub fn new(facets: EndpointBindingEffectFacets) -> Self {
        Self {
            verify: facets.verify,
            deliver: facets.deliver,
            fence: facets.fence,
            attached: facets.attached,
            release: facets.release,
        }
    }
}

#[async_trait]
impl EndpointBindingDriverEffects for EndpointBindingEffectsService {
    async fn verify(
        &self,
        target: &EndpointDeliveryTarget,
    ) -> Result<EndpointAccessObservation, String> {
        self.verify.verify(target).await
    }

    async fn deliver(
        &self,
        target: &EndpointDeliveryTarget,
        delivery: &EndpointBindingDelivery,
    ) -> Result<EndpointSocketIdentity, String> {
        self.deliver.deliver(target, delivery).await
    }

    async fn fence(&self, target: &EndpointDeliveryTarget) -> Result<(), String> {
        self.fence.fence(target).await
    }

    async fn consumer_attached(&self, target: &EndpointDeliveryTarget) -> Result<bool, String> {
        self.attached.attached(target).await
    }

    async fn release(&self, target: &EndpointDeliveryTarget) -> Result<(), String> {
        self.release.release(target).await
    }
}

#[async_trait]
impl EffectService for EndpointBindingEffectsService {
    async fn handle(
        &self,
        _invocation: ServiceInvocation<'_>,
    ) -> Result<EffectResponse, EffectServiceError> {
        // The declaration's method gates admission at the hosting side; the
        // service serves its one declared zone-plane method from the crate's
        // own committed serving contract.
        inspect_binding_response()
    }
}

/// The composition-root factory that hosts the `EndpointBinding` effects
/// service in one zone (R5): the daemon registers one per zone, carrying
/// that zone's facet set, and the host rebuilds the service from it on
/// respawn.
pub struct EndpointBindingEffectsServiceFactory {
    facets: EndpointBindingEffectFacets,
}

impl EndpointBindingEffectsServiceFactory {
    /// Build the factory from one zone's facet set.
    pub fn new(facets: EndpointBindingEffectFacets) -> Self {
        Self { facets }
    }
}

impl EffectServiceFactory for EndpointBindingEffectsServiceFactory {
    fn build(&self) -> Arc<dyn EffectService> {
        Arc::new(EndpointBindingEffectsService::new(self.facets.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use d2b_contracts_resource::v3::endpoint_binding::EndpointBindingSpec;
    use d2b_contracts_resource::v3::{
        BindingArbitration, BindingRealizationFacet, BindingSourceDecision, EndpointAttachmentKind,
        RequestedRights, ResourceGeneration, ResourceRef, ResourceUid, ZoneId,
        execution_policy::BoundedToken,
    };

    use crate::test_support::FakeEndpointEffects;

    /// The endpoint row's own purpose, which is the purpose the delivery
    /// carries: a binding row does not restate it.
    fn purpose() -> BoundedToken {
        BoundedToken::parse("display".to_owned()).expect("purpose")
    }

    fn row() -> EndpointBindingSpec {
        EndpointBindingSpec::new(
            ResourceRef::parse("Endpoint/display").expect("endpoint reference"),
            ResourceRef::parse("Process/consumer").expect("consumer reference"),
            EndpointAttachmentKind::Connect,
            BoundedToken::parse("primary".to_owned()).expect("slot"),
            BindingSourceDecision::new(
                vec![RequestedRights::Consume],
                BindingArbitration::Shared,
                vec![BindingRealizationFacet::EndpointDescriptor],
            )
            .expect("source decision"),
        )
        .expect("binding row")
    }

    fn target() -> EndpointDeliveryTarget {
        EndpointDeliveryTarget::derive(
            &ZoneId::parse("work").expect("zone"),
            &row(),
            ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").expect("uid"),
            ResourceGeneration::new(3).expect("generation"),
            &purpose(),
        )
    }

    /// The service's typed seam delegates onto the facets, one call per
    /// effect, in the order the driver makes them.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn the_service_delegates_onto_the_facets() {
        let fake = FakeEndpointEffects::new();
        let service = EndpointBindingEffectsService::new(fake.facet_set());
        let target = target();
        let delivery = EndpointBindingDelivery::for_row(&row());

        let observation = service.verify(&target).await.expect("verify");
        assert!(
            observation.traversable(),
            "the scripted ancestor traverse bit is applied"
        );
        assert_eq!(fake.call_order(), ["verify"]);

        let socket = service.deliver(&target, &delivery).await.expect("deliver");
        assert_eq!(
            socket,
            fake.pinned_socket(),
            "the delivery answers the identity the adapter pinned"
        );
        service.fence(&target).await.expect("fence");
        assert!(!service.consumer_attached(&target).await.expect("attached"));
        service.release(&target).await.expect("release");
        assert_eq!(
            fake.call_order(),
            ["verify", "deliver", "fence", "attached", "release"]
        );
    }

    /// The hosted report is built from this family's own declaration, so it
    /// names the exact endpoint the family reads, the facets it serves, and
    /// the absence of any child creation it licenses.
    #[test]
    fn the_hosted_report_answers_this_familys_own_serving_contract() {
        let response = inspect_binding_response().expect("the report is canonical");
        let payload = serde_json::to_value(&response.payload).expect("report json");
        assert_eq!(payload["family"], serde_json::json!("endpoint-binding"));
        assert_eq!(
            payload["resourceType"],
            serde_json::json!(ENDPOINT_BINDING_TYPE_NAME)
        );
        assert_eq!(payload["sourceType"], serde_json::json!("Endpoint"));
        assert_eq!(
            payload["reads"],
            serde_json::json!(["Endpoint"]),
            "the exact endpoint is the one row the family reads"
        );
        assert_eq!(
            payload["creations"],
            serde_json::json!([]),
            "the family licenses no child creation"
        );
        assert_eq!(
            payload["facets"],
            serde_json::json!(["endpoint-descriptor", "endpoint-pathname"])
        );
        assert_eq!(
            payload["serving"],
            serde_json::json!(["verify", "deliver", "fence", "attached", "release"])
        );
    }

    /// The declared service carries this family's one zone-plane method and
    /// no attach kinds, streams, or endpoint policy.
    #[test]
    fn the_declared_service_carries_only_its_own_method() {
        assert_eq!(
            BINDING_EFFECTS_SERVICE.id,
            "endpoint-binding.d2bus.org/effects"
        );
        assert_eq!(BINDING_EFFECTS_SERVICE.methods.len(), 1);
        assert_eq!(
            BINDING_EFFECTS_SERVICE.methods[0].name,
            "inspect-binding"
        );
        assert!(BINDING_EFFECTS_SERVICE.attach_kinds.is_empty());
        assert!(BINDING_EFFECTS_SERVICE.streams.is_empty());
        assert!(BINDING_EFFECTS_SERVICE.endpoint_policy.is_none());
    }

    /// A failed facet surfaces as an operational error on the typed seam and
    /// never as a fabricated observation.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn a_failed_facet_surfaces_rather_than_fabricating_an_observation() {
        let fake = FakeEndpointEffects::new();
        fake.set_fail_verify(true);
        let service = EndpointBindingEffectsService::new(fake.facet_set());
        let error = service
            .verify(&target())
            .await
            .expect_err("the adapter failure is reported");
        assert!(error.contains("scripted verify failure"), "{error}");
    }
}
