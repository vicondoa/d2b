//! The provider-owned implementation of the VolumeBinding family's driver
//! effects (U6): the family serves its effects from this crate instead of a
//! daemon-built port.
//!
//! Two surfaces share one implementation value:
//!
//! - the driver's typed seam, [`BindingDriverEffects`], which the family's
//!   driver holds (the factory builds it from the same facets);
//! - the declared zone-plane service [`BINDING_EFFECTS_SERVICE`], hosted
//!   per zone by the daemon through [`BindingEffectsServiceFactory`]. Its
//!   one method (`inspect-binding`) answers the family's committed serving
//!   contract - the owned children the binding mints and the serving
//!   surfaces its effects drive - served from inside the owning crate.
//!
//! Everything the effects read crosses the provider boundary as declared
//! facets ([`crate::facets`]): the serving-socket probe, the socket
//! removal, and the guest-mount observation the daemon supplies. Nothing
//! here names a daemon state type.

use std::sync::Arc;

use async_trait::async_trait;
use d2b_provider_toolkit::{
    EffectResponse, EffectService, EffectServiceError, EffectServiceFactory, ServiceInvocation,
};
use d2b_provider_volume_virtiofs::{SocketIdentity, StoredBinding};
use d2b_resource_runtime::identity::ResourceKey;
use d2b_resource_types::{ServiceDecl, ServiceMethod};

use crate::driver::BindingDriverEffects;
use crate::facets::BindingEffectFacets;

/// The VolumeBinding family's declared effects service.
///
/// One zone-plane method, `inspect-binding`: it answers this family's
/// committed serving contract - the owned children one binding mints (the
/// worker Process and the Endpoint socket) and the serving surfaces the
/// driver's effects drive. The report is served from the crate's own
/// declaration, so it proves the family's serving effects run inside the
/// owning crate (U6), and it is hermetic: no host state is read or mutated.
///
/// The service is declared on the `VolumeBinding` descriptor alone; the
/// family's driver effects (the typed seam) stay the driver's object, not a
/// hosted method surface.
pub const BINDING_EFFECTS_SERVICE: ServiceDecl = ServiceDecl {
    id: "volume-binding.d2bus.org/effects",
    methods: &[ServiceMethod::zone_plane("inspect-binding")],
    attach_kinds: &[],
    streams: &[],
    endpoint_policy: None,
};

/// The one `inspect-binding` response payload: the family's committed
/// serving contract. The payload is built through the canonical JSON object
/// path, so a structural character in a committed value yields a correctly
/// escaped report rather than an unparseable one; the refusal is
/// unreachable and names its own code.
fn inspect_binding_response() -> Result<EffectResponse, EffectServiceError> {
    let payload = serde_json::from_value(serde_json::json!({
        "family": "volume-binding",
        "resourceType": "VolumeBinding",
        "creations": [
            {"child": "Process", "providerRef": crate::driver::WORKER_PROVIDER_REF, "order": 0},
            {"child": "Endpoint", "providerRef": crate::driver::BINDING_PROVIDER_REF, "order": 1},
        ],
        "serving": ["socket-ready", "remove-socket", "guest-mount"],
    }))
    .map_err(|_| EffectServiceError::Declined {
        service: BINDING_EFFECTS_SERVICE.id.to_owned(),
        reason: "inspect-binding-response-invalid".to_owned(),
    })?;
    Ok(EffectResponse::new(payload))
}

/// The provider-owned VolumeBinding effects (U6), built from the
/// daemon-supplied facets.
///
/// One value serves both the driver's typed seam and the declared hosted
/// service: the factory constructs it from the same [`BindingEffectFacets`]
/// the composition root supplies, so the hosted surface and the driver
/// observe the same serving adapter.
pub struct BindingEffectsService {
    ready: Arc<dyn crate::facets::SocketReadySource>,
    remove: Arc<dyn crate::facets::SocketRemoveSource>,
    guest_mount: Arc<dyn crate::facets::GuestMountSource>,
}

impl BindingEffectsService {
    /// Build the effects from one zone's daemon-supplied facet set (R2):
    /// every daemon-structural read rides the facets, never a daemon handle.
    pub fn new(facets: BindingEffectFacets) -> Self {
        Self {
            ready: facets.ready,
            remove: facets.remove,
            guest_mount: facets.guest_mount,
        }
    }
}

#[async_trait]
impl BindingDriverEffects for BindingEffectsService {
    async fn socket_ready(&self, socket: &SocketIdentity) -> bool {
        self.ready.ready(socket).await
    }

    async fn remove_socket(&self, socket: &SocketIdentity) -> Result<(), String> {
        self.remove.remove(socket).await
    }

    async fn guest_mount_ready(
        &self,
        key: &ResourceKey,
        _binding: &StoredBinding,
    ) -> Result<bool, String> {
        self.guest_mount.guest_mount_ready(key).await
    }
}

#[async_trait]
impl EffectService for BindingEffectsService {
    async fn handle(
        &self,
        _invocation: ServiceInvocation<'_>,
    ) -> Result<EffectResponse, EffectServiceError> {
        // The declaration's method gates admission at the hosting side; the
        // service serves its one declared zone-plane method from the
        // crate's own committed serving contract.
        inspect_binding_response()
    }
}

/// The composition-root factory that hosts the VolumeBinding effects
/// service in one zone (R5): the daemon registers one per zone, carrying
/// that zone's facet set, and the host rebuilds the service from it on
/// respawn.
pub struct BindingEffectsServiceFactory {
    facets: BindingEffectFacets,
}

impl BindingEffectsServiceFactory {
    /// Build the factory from one zone's facet set.
    pub fn new(facets: BindingEffectFacets) -> Self {
        Self { facets }
    }
}

impl EffectServiceFactory for BindingEffectsServiceFactory {
    fn build(&self) -> Arc<dyn EffectService> {
        Arc::new(BindingEffectsService::new(self.facets.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::test_support::FakeServingEffects;

    /// The service's typed seam delegates onto the facets, and the hosted
    /// report answers the family's committed serving contract.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn the_service_delegates_onto_the_facets() {
        use d2b_contracts_resource::v3::{
            ResourceRef, ResourceUid, execution_policy::BoundedToken, volume::AttachmentAccess,
            volume_binding::VolumeBindingSpec,
        };

        let fake = FakeServingEffects::new();
        let service = BindingEffectsService::new(fake.facet_set());
        let socket = SocketIdentity::derive(
            &BoundedToken::parse("work".to_owned()).expect("zone"),
            &ResourceRef::parse("Volume/work").expect("volume"),
            &ResourceRef::parse("Guest/acceptance-guest").expect("guest"),
        );
        let key = ResourceKey::new("work", "VolumeBinding", "binding");
        let binding = StoredBinding::new(
            VolumeBindingSpec::new(
                ResourceRef::parse("Volume/work").expect("volume"),
                ResourceRef::parse("Guest/acceptance-guest").expect("guest"),
                "named",
                AttachmentAccess::ReadWrite,
                "/mnt/work",
            )
            .expect("binding spec"),
            ResourceUid::parse("00000000-0000-4000-8000-000000000000").expect("uid"),
            d2b_contracts_resource::v3::ResourceGeneration::new(1).expect("generation"),
            d2b_contracts_resource::v3::ZoneRevision::new(1),
        );

        assert!(!service.socket_ready(&socket).await);
        assert_eq!(fake.call_order(), ["socket-ready"]);
        service.remove_socket(&socket).await.expect("remove");
        assert_eq!(fake.call_order(), ["socket-ready", "remove-socket"]);
        service
            .guest_mount_ready(&key, &binding)
            .await
            .expect("guest-mount");
        assert_eq!(fake.call_order(), ["socket-ready", "remove-socket", "guest-mount"]);
    }
}