//! The provider-owned implementation of the `DeviceBinding` family's driver
//! effects (U6): the family serves its effects from this crate instead of a
//! daemon-built port.
//!
//! Two surfaces share one implementation value:
//!
//! - the driver's typed seam, [`DeviceBindingDriverEffects`], which the
//!   family's driver holds (the factory builds it from the same facets);
//! - the declared zone-plane service
//!   [`DEVICE_BINDING_EFFECTS_SERVICE`], hosted per zone by the daemon
//!   through [`DeviceBindingEffectsServiceFactory`]. Its one method
//!   (`inspect-binding`) answers the family's committed serving contract -
//!   the claim-and-attach drive its effects make, the readiness and drain
//!   observations they read, and the two releases the teardown runs - served
//!   from inside the owning crate.
//!
//! Everything the effects touch crosses the provider boundary as declared
//! facets ([`crate::facets`]): the device mediation that takes and gives back
//! the physical authority, and the observation over the realized attachment.
//! Nothing here names a daemon state type.

use std::sync::Arc;

use async_trait::async_trait;
use d2b_provider_toolkit::{
    EffectResponse, EffectService, EffectServiceError, EffectServiceFactory, ServiceInvocation,
};
use d2b_resource_types::{ServiceDecl, ServiceMethod};

use crate::driver::{DEVICE_BINDING_CREATIONS, DeviceBindingDriverEffects};
use crate::facets::{
    AttachmentMediation, AttachmentObservation, DeviceAttachment, DeviceBindingEffectFacets,
    DeviceEstablishOutcome, DeviceRefusal,
};

/// The `DeviceBinding` family's declared effects service.
///
/// One zone-plane method, `inspect-binding`: it answers this family's
/// committed serving contract - the realization one binding drives and the
/// serving surfaces the driver's effects drive. The report is served from the
/// crate's own declaration, so it proves the family's serving effects run
/// inside the owning crate (U6), and it is hermetic: no host state is read or
/// mutated.
///
/// The service is declared on the `DeviceBinding` descriptor alone; the
/// family's driver effects (the typed seam) stay the driver's object, not a
/// hosted method surface.
pub const DEVICE_BINDING_EFFECTS_SERVICE: ServiceDecl = ServiceDecl {
    id: "device-binding.d2bus.org/effects",
    methods: &[ServiceMethod::zone_plane("inspect-binding")],
    attach_kinds: &[],
    streams: &[],
    endpoint_policy: None,
};

/// The one `inspect-binding` response payload: the family's committed serving
/// contract. The payload is built through the canonical JSON object path, so a
/// structural character in a committed value yields a correctly escaped report
/// rather than an unparseable one; the refusal is unreachable and names its
/// own code.
fn inspect_binding_response() -> Result<EffectResponse, EffectServiceError> {
    let payload = serde_json::from_value(serde_json::json!({
        "family": "device-binding",
        "resourceType": crate::driver::DEVICE_BINDING_TYPE_NAME,
        "creations": DEVICE_BINDING_CREATIONS
            .iter()
            .map(|creation| serde_json::json!({
                "child": creation.child.to_resource_type_name().as_str(),
                "providerRef": creation.provider_ref,
                "order": creation.order,
            }))
            .collect::<Vec<_>>(),
        "serving": [
            "establish-attachment",
            "attachment-ready",
            "attachment-held",
            "release-attachment",
            "release-slot",
        ],
    }))
    .map_err(|_| EffectServiceError::Declined {
        service: DEVICE_BINDING_EFFECTS_SERVICE.id.to_owned(),
        reason: "inspect-binding-response-invalid".to_owned(),
    })?;
    Ok(EffectResponse::new(payload))
}

/// The provider-owned `DeviceBinding` effects (U6), built from the
/// daemon-supplied facets.
///
/// One value serves both the driver's typed seam and the declared hosted
/// service: the factory constructs it from the same
/// [`DeviceBindingEffectFacets`] the composition root supplies, so the hosted
/// surface and the driver observe the same mediation adapter.
pub struct DeviceBindingEffectsService {
    mediation: Arc<dyn AttachmentMediation>,
    observation: Arc<dyn AttachmentObservation>,
}

impl DeviceBindingEffectsService {
    /// Build the effects from one zone's daemon-supplied facet set (R2):
    /// every daemon-structural read rides the facets, never a daemon handle.
    pub fn new(facets: DeviceBindingEffectFacets) -> Self {
        Self {
            mediation: facets.mediation,
            observation: facets.observation,
        }
    }
}

#[async_trait]
impl DeviceBindingDriverEffects for DeviceBindingEffectsService {
    async fn establish(
        &self,
        attachment: &DeviceAttachment,
    ) -> Result<DeviceEstablishOutcome, DeviceRefusal> {
        self.mediation.establish(attachment).await
    }

    async fn attachment_ready(&self, attachment: &DeviceAttachment) -> bool {
        self.observation.attachment_ready(attachment).await
    }

    async fn attachment_held(&self, attachment: &DeviceAttachment) -> Result<bool, String> {
        self.observation.attachment_held(attachment).await
    }

    async fn release_attachment(&self, attachment: &DeviceAttachment) -> Result<(), String> {
        self.mediation.release_attachment(attachment).await
    }

    async fn release_slot(&self, attachment: &DeviceAttachment) -> Result<(), String> {
        self.mediation.release_slot(attachment).await
    }
}

#[async_trait]
impl EffectService for DeviceBindingEffectsService {
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

/// The composition-root factory that hosts the `DeviceBinding` effects service
/// in one zone (R5): the daemon registers one per zone, carrying that zone's
/// facet set, and the host rebuilds the service from it on respawn.
pub struct DeviceBindingEffectsServiceFactory {
    facets: DeviceBindingEffectFacets,
}

impl DeviceBindingEffectsServiceFactory {
    /// Build the factory from one zone's facet set.
    pub fn new(facets: DeviceBindingEffectFacets) -> Self {
        Self { facets }
    }
}

impl EffectServiceFactory for DeviceBindingEffectsServiceFactory {
    fn build(&self) -> Arc<dyn EffectService> {
        Arc::new(DeviceBindingEffectsService::new(self.facets.clone()))
    }
}

#[cfg(test)]
mod tests {
    use d2b_contracts_resource::v3::{
        BindingArbitration, BindingRealizationFacet, BindingSourceDecision, DeviceClaimRequest,
        DeviceFunction,
        RequestedRights, ResourceGeneration, ResourceRef, ResourceUid, ZoneRevision,
        device_binding::DeviceBindingSpec, execution_policy::BoundedToken,
    };
    use d2b_resource_runtime::identity::ResourceKey;

    use super::*;
    use crate::facets::DeviceBindingFence;
    use crate::test_support::FakeAttachmentEffects;

    fn attachment() -> DeviceAttachment {
        let spec = DeviceBindingSpec::new(
            ResourceRef::parse("Device/gpu0").expect("device"),
            ResourceRef::parse("Process/render").expect("consumer"),
            DeviceFunction::parse("render").expect("function"),
            DeviceClaimRequest::Exclusive,
            BoundedToken::parse("gpu0").expect("slot"),
            BindingSourceDecision::new(
                vec![RequestedRights::Exclusive],
                BindingArbitration::Exclusive,
                vec![BindingRealizationFacet::DeviceAttachment],
            )
            .expect("the Device source admits an exclusive attachment"),
        )
        .expect("canonical row spec");
        DeviceAttachment::new(
            ResourceKey::new("work", "DeviceBinding", "dev-binding-0001"),
            None,
            spec,
            DeviceBindingFence::new(
                ResourceUid::parse("00000000-0000-4000-8000-000000000000").expect("uid"),
                ResourceGeneration::new(1).expect("generation"),
                ZoneRevision::new(1),
            ),
        )
    }

    /// The service's typed seam delegates onto the facets, and the hosted
    /// report answers the family's committed serving contract.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn the_service_delegates_onto_the_facets_and_serves_its_declaration() {
        let fake = FakeAttachmentEffects::new();
        let service = DeviceBindingEffectsService::new(fake.facet_set());
        let attachment = attachment();

        assert_eq!(
            service.establish(&attachment).await.expect("establish"),
            DeviceEstablishOutcome::Realized
        );
        assert!(!service.attachment_ready(&attachment).await);
        assert!(!service.attachment_held(&attachment).await.expect("held"));
        service
            .release_attachment(&attachment)
            .await
            .expect("release attachment");
        service.release_slot(&attachment).await.expect("release slot");
        assert_eq!(
            fake.call_order(),
            [
                "establish",
                "ready",
                "held",
                "release-attachment",
                "release-slot",
            ],
            "the driver's seam reaches the facets and nothing else"
        );

        let payload = inspect_binding_response()
            .expect("the served report")
            .payload
            .to_canonical_bytes();
        let payload = String::from_utf8(payload).expect("a canonical payload is utf-8");
        assert!(
            payload.contains("\"resourceType\":\"DeviceBinding\""),
            "{payload}"
        );
        assert!(payload.contains("release-slot"), "{payload}");
        // The family mints no children, and the report says so rather than
        // claiming an attachment surface this crate does not own.
        assert!(payload.contains("\"creations\":[]"), "{payload}");
        assert!(
            DEVICE_BINDING_EFFECTS_SERVICE.declares_method("inspect-binding"),
            "the declaration names the one method the family serves"
        );
    }

}