//! The provider-owned implementation of the USBIP family's driver effects
//! (U12 usbip step): the family serves its effects over the daemon-supplied
//! facets instead of a daemon-built port.
//!
//! Two surfaces share one implementation value:
//!
//! - the driver's typed seam, [`UsbipDriverEffects`], which the family's
//!   driver holds (the factory builds it from the same facets);
//! - the declared zone-plane service [`USBIP_EFFECTS_SERVICE`], hosted per
//!   zone by the daemon through [`UsbipEffectsServiceFactory`]. Its one
//!   method (`inspect-usbip`) answers the family's committed surface: the
//!   provider identity, the resource types it serves, and the typed
//!   bind/unbind operations its dispatcher invokes.
//!
//! Everything the effects read crosses the provider boundary as declared
//! facets ([`crate::facets`]): the daemon's USBIP runtime (the reconcile
//! and finalize orchestration over the daemon's own admission, child rows,
//! and readiness state) and the privileged typed broker dispatch. Nothing
//! here names a daemon state type.

use std::sync::Arc;

use async_trait::async_trait;
use d2b_provider_toolkit::{
    EffectResponse, EffectService, EffectServiceError, EffectServiceFactory, ServiceInvocation,
    SharedProviderEffectError, SharedProviderEffectOutcome, SharedProviderEffectRequest,
    SharedProviderFinalize,
};
use d2b_resource_types::{ServiceDecl, ServiceMethod};

use crate::driver::{UsbipComponent, UsbipDriverEffects};
use crate::facets::UsbipEffectFacets;

/// The USBIP family's declared effects service.
///
/// One zone-plane method, `inspect-usbip`: it answers the family's
/// committed surface - the provider identity, the two resource types it
/// serves, and the typed bind/unbind operations its kernel dispatcher
/// invokes. The report is hermetic: no host state is read or mutated.
///
/// The service is declared on the USB descriptors; the family's driver
/// effects (the typed seam) stay the driver's object, not a hosted method
/// surface.
pub const USBIP_EFFECTS_SERVICE: ServiceDecl = ServiceDecl {
    id: "usbip.d2bus.org/effects",
    methods: &[ServiceMethod::zone_plane("inspect-usbip")],
    attach_kinds: &[],
    streams: &[],
    endpoint_policy: None,
};

/// The one `inspect-usbip` response payload: the family's committed
/// surface. The payload is built through the canonical JSON object path, so
/// a structural character in a trusted value yields a correctly escaped
/// report rather than an unparseable one; the refusal is unreachable and
/// names its own code.
fn inspect_usbip_response() -> Result<EffectResponse, EffectServiceError> {
    let payload = serde_json::from_value(serde_json::json!({
        "family": "device-usbip",
        "provider": crate::PROVIDER_REF,
        "resourceType": crate::USB_SERVICE_RESOURCE_TYPE,
        "types": [
            crate::USB_SERVICE_RESOURCE_TYPE,
            crate::USB_BINDING_RESOURCE_TYPE,
        ],
        "operations": ["UsbipBind", "UsbipUnbind"],
    }))
    .map_err(|_| EffectServiceError::Declined {
        service: USBIP_EFFECTS_SERVICE.id.to_owned(),
        reason: "inspect-usbip-response-invalid".to_owned(),
    })?;
    Ok(EffectResponse::new(payload))
}

/// The provider-owned USBIP effects (U12 usbip step), built from the
/// daemon-supplied facets.
///
/// One value serves both the driver's typed seam and the declared hosted
/// service: the factory constructs it from the same [`UsbipEffectFacets`]
/// the driver factories are built from, so the hosted surface and the
/// driver observe the same runtime.
pub struct UsbipEffects {
    runtime: Arc<dyn crate::facets::UsbipRuntime>,
}

impl UsbipEffects {
    /// Build the effects from one zone's daemon-supplied facet set (R2):
    /// every daemon-structural read rides the facets, never a daemon handle.
    pub fn new(facets: UsbipEffectFacets) -> Self {
        Self {
            runtime: facets.runtime,
        }
    }
}

#[async_trait]
impl UsbipDriverEffects for UsbipEffects {
    async fn reconcile_usbip(
        &self,
        component: UsbipComponent,
        request: &SharedProviderEffectRequest<'_>,
    ) -> Result<SharedProviderEffectOutcome, SharedProviderEffectError> {
        self.runtime.reconcile_usbip(component, request).await
    }

    async fn finalize(
        &self,
        component: UsbipComponent,
        request: &SharedProviderEffectRequest<'_>,
    ) -> Result<SharedProviderFinalize, SharedProviderEffectError> {
        self.runtime.finalize(component, request).await
    }
}

/// The hosted `inspect-usbip` service: answers the family's committed
/// surface report. The report is static (the family's own vocabulary), so
/// the service holds no runtime state.
struct UsbipEffectsService;

#[async_trait]
impl EffectService for UsbipEffectsService {
    async fn handle(
        &self,
        _invocation: ServiceInvocation<'_>,
    ) -> Result<EffectResponse, EffectServiceError> {
        inspect_usbip_response()
    }
}

/// The composition-root factory that hosts the USBIP effects service in one
/// zone (R5): the daemon registers one per zone, carrying that zone's facet
/// set for the respawn path, which is not yet wired.
pub struct UsbipEffectsServiceFactory {
    // The facet set is carried for the R5 respawn contract: the daemon
    // registers one factory per zone with that zone's facet set. The
    // respawn path that rebuilds the service from the facets is not wired
    // yet, and the current static inspect service does not read them.
    #[allow(dead_code)]
    facets: UsbipEffectFacets,
}

impl UsbipEffectsServiceFactory {
    /// Build the factory from one zone's facet set.
    pub fn new(facets: UsbipEffectFacets) -> Self {
        Self { facets }
    }
}

impl EffectServiceFactory for UsbipEffectsServiceFactory {
    fn build(&self) -> Arc<dyn EffectService> {
        Arc::new(UsbipEffectsService)
    }
}