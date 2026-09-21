//! The provider-owned implementation of the security-key family's driver
//! effects (U12 security-key step): the family serves its effects over the
//! daemon-supplied facets instead of a daemon-built port.
//!
//! Two surfaces share one implementation value:
//!
//! - the driver's typed seam, [`SecurityKeyDriverEffects`], which the
//!   family's driver holds (the factory builds it from the same facets);
//! - the declared zone-plane service [`SECURITY_KEY_EFFECTS_SERVICE`],
//!   hosted per zone by the daemon through
//!   [`SecurityKeyEffectsServiceFactory`]. Its one method
//!   (`inspect-security-key`) answers the family's committed surface: the
//!   provider identity, the resource types it serves, and the relay roles
//!   its workers realize.
//!
//! Everything the effects read crosses the provider boundary as declared
//! facets ([`crate::facets`]): the daemon's security-key runtime (the
//! reconcile and finalize orchestration over the daemon's own admission,
//! child rows, and readiness state). Nothing here names a daemon state
//! type.

use std::sync::Arc;

use async_trait::async_trait;
use d2b_provider_toolkit::{
    EffectResponse, EffectService, EffectServiceError, EffectServiceFactory, ServiceInvocation,
    SharedProviderEffectError, SharedProviderEffectOutcome, SharedProviderEffectRequest,
    SharedProviderFinalize,
};
use d2b_resource_types::{ServiceDecl, ServiceMethod};

use crate::driver::{SecurityKeyComponent, SecurityKeyDriverEffects};
use crate::facets::SecurityKeyEffectFacets;

/// The security-key family's declared effects service.
///
/// One zone-plane method, `inspect-security-key`: it answers the family's
/// committed surface - the provider identity, the two resource types it
/// serves, and the relay roles its workers realize. The report is hermetic:
/// no host state is read or mutated.
///
/// The service is declared on the security-key descriptors; the family's
/// driver effects (the typed seam) stay the driver's object, not a hosted
/// method surface.
pub const SECURITY_KEY_EFFECTS_SERVICE: ServiceDecl = ServiceDecl {
    id: "security-key.d2bus.org/effects",
    methods: &[ServiceMethod::zone_plane("inspect-security-key")],
    attach_kinds: &[],
    streams: &[],
    endpoint_policy: None,
};

/// The one `inspect-security-key` response payload: the family's committed
/// surface. The payload is built through the canonical JSON object path, so
/// a structural character in a trusted value yields a correctly escaped
/// report rather than an unparseable one; the refusal is unreachable and
/// names its own code.
fn inspect_security_key_response() -> Result<EffectResponse, EffectServiceError> {
    let payload = serde_json::from_value(serde_json::json!({
        "family": "device-security-key",
        "provider": crate::PROVIDER_REF,
        "resourceType": crate::SECURITY_KEY_SERVICE_RESOURCE_TYPE,
        "types": [
            crate::SECURITY_KEY_SERVICE_RESOURCE_TYPE,
            crate::SECURITY_KEY_BINDING_RESOURCE_TYPE,
        ],
        "relayRoles": ["host-relay", "guest-frontend"],
    }))
    .map_err(|_| EffectServiceError::Declined {
        service: SECURITY_KEY_EFFECTS_SERVICE.id.to_owned(),
        reason: "inspect-security-key-response-invalid".to_owned(),
    })?;
    Ok(EffectResponse::new(payload))
}

/// The provider-owned security-key effects (U12 security-key step), built
/// from the daemon-supplied facets.
///
/// One value serves both the driver's typed seam and the declared hosted
/// service: the factory constructs it from the same
/// [`SecurityKeyEffectFacets`] the driver factories are built from, so the
/// hosted surface and the driver observe the same runtime.
pub struct SecurityKeyEffects {
    runtime: Arc<dyn crate::facets::SecurityKeyRuntime>,
}

impl SecurityKeyEffects {
    /// Build the effects from one zone's daemon-supplied facet set (R2):
    /// every daemon-structural read rides the facets, never a daemon handle.
    pub fn new(facets: SecurityKeyEffectFacets) -> Self {
        Self {
            runtime: facets.runtime,
        }
    }
}

#[async_trait]
impl SecurityKeyDriverEffects for SecurityKeyEffects {
    async fn reconcile_security_key(
        &self,
        component: SecurityKeyComponent,
        request: &SharedProviderEffectRequest<'_>,
    ) -> Result<SharedProviderEffectOutcome, SharedProviderEffectError> {
        self.runtime
            .reconcile_security_key(component, request)
            .await
    }

    async fn finalize(
        &self,
        component: SecurityKeyComponent,
        request: &SharedProviderEffectRequest<'_>,
    ) -> Result<SharedProviderFinalize, SharedProviderEffectError> {
        self.runtime.finalize(component, request).await
    }
}

/// The hosted `inspect-security-key` service: answers the family's
/// committed surface report. The report is static (the family's own
/// vocabulary), so the service holds no runtime state.
struct SecurityKeyEffectsService;

#[async_trait]
impl EffectService for SecurityKeyEffectsService {
    async fn handle(
        &self,
        _invocation: ServiceInvocation<'_>,
    ) -> Result<EffectResponse, EffectServiceError> {
        inspect_security_key_response()
    }
}

/// The composition-root factory that hosts the security-key effects service
/// in one zone (R5): the daemon registers one per zone, carrying that
/// zone's facet set for the respawn path, which is not yet wired.
pub struct SecurityKeyEffectsServiceFactory {
    // The facet set is carried for the R5 respawn contract: the daemon
    // registers one factory per zone with that zone's facet set. The
    // respawn path that rebuilds the service from the facets is not wired
    // yet, and the current static inspect service does not read them.
    #[allow(dead_code)]
    facets: SecurityKeyEffectFacets,
}

impl SecurityKeyEffectsServiceFactory {
    /// Build the factory from one zone's facet set.
    pub fn new(facets: SecurityKeyEffectFacets) -> Self {
        Self { facets }
    }
}

impl EffectServiceFactory for SecurityKeyEffectsServiceFactory {
    fn build(&self) -> Arc<dyn EffectService> {
        Arc::new(SecurityKeyEffectsService)
    }
}