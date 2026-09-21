//! The provider-owned implementation of the Device family's driver effects
//! (U12 device step): the family serves its effects over the daemon-supplied
//! facets instead of a daemon-built port.
//!
//! Two surfaces share one implementation value:
//!
//! - the driver's typed seam, [`DeviceDriverEffects`], which the family's
//!   driver holds (the factory builds it from the same facets);
//! - the declared zone-plane service [`DEVICE_EFFECTS_SERVICE`], hosted per
//!   zone by the daemon through [`DeviceEffectsServiceFactory`]. Its one
//!   method (`inspect-device`) answers the family's committed surface: the
//!   four hardware components the Device type serves and their controller
//!   rows.
//!
//! Everything the effects read crosses the provider boundary as declared
//! facets ([`crate::facets`]): the daemon's Device runtime (the reconcile
//! and finalize orchestration over the daemon's own admission, child rows,
//! and readiness state). Nothing here names a daemon state type.

use std::sync::Arc;

use async_trait::async_trait;
use d2b_provider_toolkit::{
    EffectResponse, EffectService, EffectServiceError, EffectServiceFactory, ServiceInvocation,
    SharedProviderEffectError, SharedProviderEffectOutcome, SharedProviderEffectRequest,
    SharedProviderFinalize,
};
use d2b_resource_types::{ServiceDecl, ServiceMethod};

use crate::driver::{DeviceComponent, DeviceDriverEffects, DeviceResourceState};
use crate::facets::DeviceEffectFacets;

/// The Device family's declared effects service.
///
/// One zone-plane method, `inspect-device`: it answers the family's
/// committed surface - the Device resource type it serves and the four
/// hardware components (and their controller rows) the Device row
/// dispatches over. The report is hermetic: no host state is read or
/// mutated.
///
/// The service is declared on the Device descriptor; the family's driver
/// effects (the typed seam) stay the driver's object, not a hosted method
/// surface.
pub const DEVICE_EFFECTS_SERVICE: ServiceDecl = ServiceDecl {
    id: "device.d2bus.org/effects",
    methods: &[ServiceMethod::zone_plane("inspect-device")],
    attach_kinds: &[],
    streams: &[],
    endpoint_policy: None,
};

/// The one `inspect-device` response payload: the family's committed
/// surface. The payload is built through the canonical JSON object path, so
/// a structural character in a trusted value yields a correctly escaped
/// report rather than an unparseable one; the refusal is unreachable and
/// names its own code.
fn inspect_device_response() -> Result<EffectResponse, EffectServiceError> {
    let payload = serde_json::from_value(serde_json::json!({
        "family": "device",
        "resourceType": crate::driver::DEVICE_TYPE_NAME,
        "components": ["tpm", "usbip", "security-key", "gpu"],
        "controllers": [
            crate::driver::TPM_CONTROLLER_REF,
            crate::driver::USBIP_CONTROLLER_REF,
            crate::driver::SECURITY_KEY_CONTROLLER_REF,
            crate::driver::GPU_CONTROLLER_REF,
        ],
    }))
    .map_err(|_| EffectServiceError::Declined {
        service: DEVICE_EFFECTS_SERVICE.id.to_owned(),
        reason: "inspect-device-response-invalid".to_owned(),
    })?;
    Ok(EffectResponse::new(payload))
}

/// The provider-owned Device effects (U12 device step), built from the
/// daemon-supplied facets.
///
/// One value serves both the driver's typed seam and the declared hosted
/// service: the factory constructs it from the same [`DeviceEffectFacets`]
/// the driver factories are built from, so the hosted surface and the
/// driver observe the same runtime.
pub struct DeviceEffects {
    runtime: Arc<dyn crate::facets::DeviceRuntime>,
}

impl DeviceEffects {
    /// Build the effects from one zone's daemon-supplied facet set (R2):
    /// every daemon-structural read rides the facets, never a daemon handle.
    pub fn new(facets: DeviceEffectFacets) -> Self {
        Self {
            runtime: facets.runtime,
        }
    }
}

#[async_trait]
impl DeviceDriverEffects for DeviceEffects {
    async fn reconcile_device(
        &self,
        component: DeviceComponent,
        request: &SharedProviderEffectRequest<'_>,
        state: &DeviceResourceState,
    ) -> Result<SharedProviderEffectOutcome, SharedProviderEffectError> {
        self.runtime
            .reconcile_device(component, request, state)
            .await
    }

    async fn finalize_device(
        &self,
        component: DeviceComponent,
        request: &SharedProviderEffectRequest<'_>,
        state: &DeviceResourceState,
    ) -> Result<SharedProviderFinalize, SharedProviderEffectError> {
        self.runtime
            .finalize_device(component, request, state)
            .await
    }
}

/// The hosted `inspect-device` service: answers the family's committed
/// surface report. The report is static (the family's own vocabulary), so
/// the service holds no runtime state.
struct DeviceEffectsService;

#[async_trait]
impl EffectService for DeviceEffectsService {
    async fn handle(
        &self,
        _invocation: ServiceInvocation<'_>,
    ) -> Result<EffectResponse, EffectServiceError> {
        inspect_device_response()
    }
}

/// The composition-root factory that hosts the Device effects service in one
/// zone (R5): the daemon registers one per zone, carrying that zone's facet
/// set, and the host rebuilds the service from it on respawn.
pub struct DeviceEffectsServiceFactory {
    // The facet set is carried for the R5 respawn contract even though the
    // current inspect service is static; `build()` rebuilds from it.
    #[allow(dead_code)]
    facets: DeviceEffectFacets,
}

impl DeviceEffectsServiceFactory {
    /// Build the factory from one zone's facet set.
    pub fn new(facets: DeviceEffectFacets) -> Self {
        Self { facets }
    }
}

impl EffectServiceFactory for DeviceEffectsServiceFactory {
    fn build(&self) -> Arc<dyn EffectService> {
        Arc::new(DeviceEffectsService)
    }
}