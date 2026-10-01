//! The provider-owned implementation of the CredentialBinding family's driver
//! effects: the family serves its effects from this crate instead of a
//! daemon-built port.
//!
//! Two surfaces share one implementation value:
//!
//! - the driver's typed seam, [`CredentialBindingDriverEffects`], which the
//!   family's driver holds (the factory builds it from the same facets);
//! - the declared zone-plane service
//!   [`CREDENTIAL_BINDING_EFFECTS_SERVICE`], hosted per zone by the daemon
//!   through [`CredentialBindingEffectsServiceFactory`]. Its one method
//!   (`inspect-binding`) answers the family's committed serving contract - the
//!   deliveries it establishes and the surfaces its effects drive - served
//!   from inside the owning crate.
//!
//! Everything the effects read or write crosses the provider boundary as
//! declared facets ([`crate::facets`]): the delivery, the observation of a
//! live delivery, the revocation, and the clock. Nothing here names a daemon
//! state type, and nothing here can carry credential material: the delivery
//! request holds identity, vocabulary, counters, and bounds, and the session
//! it answers with is the non-secret identity of what was established.

use std::sync::Arc;

use async_trait::async_trait;
use d2b_provider_toolkit::{
    EffectResponse, EffectService, EffectServiceError, EffectServiceFactory, ServiceInvocation,
};
use d2b_resource_types::{ServiceDecl, ServiceMethod};

use crate::driver::{CredentialBindingDriverEffects, CredentialDelivery, CredentialRevocation, DeliveredSession};
use crate::facets::CredentialBindingEffectFacets;

/// The CredentialBinding family's declared effects service.
///
/// One zone-plane method, `inspect-binding`: it answers this family's
/// committed serving contract - the deliveries one binding establishes, the
/// realization it drives, and the surfaces its effects reach. The report is
/// served from the crate's own declaration, so it proves the family's serving
/// effects run inside the owning crate, and it is hermetic: no host state is
/// read or mutated and no credential material is involved.
///
/// The service is declared on the `CredentialBinding` descriptor alone; the
/// family's driver effects (the typed seam) stay the driver's object, not a
/// hosted method surface.
pub const CREDENTIAL_BINDING_EFFECTS_SERVICE: ServiceDecl = ServiceDecl {
    id: "credential-binding.d2bus.org/effects",
    methods: &[ServiceMethod::zone_plane("inspect-binding")],
    attach_kinds: &[],
    streams: &[],
    endpoint_policy: None,
};

/// The one `inspect-binding` response payload: the family's committed serving
/// contract.
///
/// The payload is built through the JSON object path, so a structural
/// character in a committed value yields a correctly escaped report rather
/// than an unparseable one; the refusal is unreachable and names its own
/// code.
fn inspect_binding_response() -> Result<EffectResponse, EffectServiceError> {
    let payload = serde_json::from_value(serde_json::json!({
        "family": "credential-binding",
        "resourceType": crate::driver::CREDENTIAL_BINDING_TYPE_NAME,
        // A delivery realizes inside an admitted session at an existing
        // execution target: the family mints no child rows.
        "creations": [],
        "serving": ["deliver", "observe", "revoke", "now"],
    }))
    .map_err(|_| EffectServiceError::Declined {
        service: CREDENTIAL_BINDING_EFFECTS_SERVICE.id.to_owned(),
        reason: "inspect-binding-response-invalid".to_owned(),
    })?;
    Ok(EffectResponse::new(payload))
}

/// The provider-owned CredentialBinding effects, built from the
/// daemon-supplied facets.
///
/// One value serves both the driver's typed seam and the declared hosted
/// service: the factory constructs it from the same
/// [`CredentialBindingEffectFacets`] the composition root supplies, so the
/// hosted surface and the driver observe the same adapter.
pub struct CredentialBindingEffectsService {
    facets: CredentialBindingEffectFacets,
}

impl CredentialBindingEffectsService {
    /// Build the effects from one zone's daemon-supplied facet set: every
    /// daemon-structural read and write rides the facets, never a daemon
    /// handle.
    pub fn new(facets: CredentialBindingEffectFacets) -> Self {
        Self { facets }
    }
}

#[async_trait]
impl CredentialBindingDriverEffects for CredentialBindingEffectsService {
    async fn deliver(&self, delivery: &CredentialDelivery) -> Result<DeliveredSession, String> {
        self.facets.delivery.deliver(delivery).await
    }

    async fn observe(
        &self,
        delivery: &CredentialDelivery,
    ) -> Result<Option<DeliveredSession>, String> {
        self.facets.delivery.observe(delivery).await
    }

    async fn revoke(&self, revocation: &CredentialRevocation) -> Result<(), String> {
        self.facets.revocation.revoke(revocation).await
    }

    fn now_unix_ms(&self) -> u64 {
        self.facets.clock.now_unix_ms()
    }
}

#[async_trait]
impl EffectService for CredentialBindingEffectsService {
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

/// The composition-root factory that hosts the CredentialBinding effects
/// service in one zone: the daemon registers one per zone, carrying that
/// zone's facet set, and the host rebuilds the service from it on respawn.
pub struct CredentialBindingEffectsServiceFactory {
    facets: CredentialBindingEffectFacets,
}

impl CredentialBindingEffectsServiceFactory {
    /// Build the factory from one zone's facet set.
    pub fn new(facets: CredentialBindingEffectFacets) -> Self {
        Self { facets }
    }
}

impl EffectServiceFactory for CredentialBindingEffectsServiceFactory {
    fn build(&self) -> Arc<dyn EffectService> {
        Arc::new(CredentialBindingEffectsService::new(self.facets.clone()))
    }
}

#[cfg(test)]
mod tests {
    use d2b_contracts_resource::v3::{ResourceGeneration, ResourceUid};
    use d2b_provider_toolkit::testing::SharedLog;
    use d2b_resource_runtime::identity::ResourceKey;

    use super::*;
    use crate::driver::CREDENTIAL_BINDING_TYPE_NAME;
    use crate::test_support::FakeDeliveryEffects;

    fn identity() -> ResourceUid {
        ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").expect("fixture uid")
    }

    fn delivery() -> CredentialDelivery {
        CredentialDelivery::new(
            ResourceKey::new("work", CREDENTIAL_BINDING_TYPE_NAME, "binding"),
            ResourceKey::new("work", "Credential", "data"),
            identity(),
            ResourceGeneration::new(1).expect("generation"),
            ResourceKey::new("work", "Guest", "work-vm"),
            identity(),
            ResourceGeneration::new(2).expect("generation"),
            "operator",
            vec!["acquire-token"],
            1_760_000_060_000,
        )
    }

    /// The service's typed seam delegates onto the facets: the delivery, the
    /// observation, the revocation, and the clock each reach the double the
    /// composition root supplied, and the hosted report answers the family's
    /// committed serving contract.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn the_service_delegates_onto_the_facets() {
        let double = Arc::new(FakeDeliveryEffects::with_log(SharedLog::new()));
        double.set_now_unix_ms(1_760_000_030_000);
        let service = CredentialBindingEffectsService::new(double.facet_set());

        assert_eq!(service.now_unix_ms(), 1_760_000_030_000);
        assert_eq!(service.observe(&delivery()).await.expect("observe"), None);

        let session = service.deliver(&delivery()).await.expect("deliver");
        assert_eq!(session.destination(), delivery().destination());
        assert_eq!(session.sequence(), 1);
        assert_eq!(
            service
                .revoke(&CredentialRevocation::at_destination(
                    delivery().binding(),
                    delivery().destination()
                ))
                .await,
            Ok(())
        );
        assert_eq!(
            double.call_order(),
            vec![
                "observe:work/Guest/work-vm".to_owned(),
                "deliver:work/Guest/work-vm".to_owned(),
                "revoke:work/Guest/work-vm".to_owned(),
            ],
            "every effect the driver drove reaches the composition root's adapter"
        );
    }

    /// The hosted report answers the family's serving contract, and it is
    /// hermetic: it reads and mutates nothing and names no material.
    #[test]
    fn the_hosted_report_answers_the_serving_contract() {
        let double = FakeDeliveryEffects::new();
        // The response builder is the service's one declared method body: it
        // takes no invocation input and reaches no facet.
        let _service = CredentialBindingEffectsService::new(double.facet_set());
        let response = inspect_binding_response().expect("the declared report is served");
        let payload: serde_json::Value =
            serde_json::from_slice(&response.payload.to_canonical_bytes())
                .expect("the canonical payload parses");
        assert_eq!(
            payload,
            serde_json::json!({
                "family": "credential-binding",
                "resourceType": CREDENTIAL_BINDING_TYPE_NAME,
                "creations": [],
                "serving": ["deliver", "observe", "revoke", "now"],
            })
        );
        assert!(
            double.call_order().is_empty(),
            "the hosted report reads and mutates nothing"
        );
    }

    /// A failing facet surfaces as the adapter's own error: the service adds
    /// no retry policy of its own, because the actor owns that.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn a_failing_delivery_surfaces_the_adapters_error() {
        let double = FakeDeliveryEffects::new();
        double.set_fail_deliver(true);
        let service = CredentialBindingEffectsService::new(double.facet_set());
        let error = service
            .deliver(&delivery())
            .await
            .expect_err("the scripted failure surfaces");
        assert_eq!(error, "scripted delivery failure");
        assert_eq!(double.deliveries(), 0, "a failed delivery mints nothing");
    }
}
