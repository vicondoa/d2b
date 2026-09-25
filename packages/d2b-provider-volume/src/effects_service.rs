//! The provider-owned implementation of the Volume family's driver effects
//! (U7): the family serves its effects over the daemon-supplied facets
//! instead of a daemon-built port.
//!
//! Two surfaces share one implementation value:
//!
//! - the driver's typed seam, [`VolumeDriverEffects`], which the family's
//!   driver holds (the factory builds it from the same facets);
//! - the declared zone-plane service [`VOLUME_EFFECTS_SERVICE`], hosted
//!   per zone by the daemon through [`VolumeEffectsServiceFactory`]. Its one
//!   method (`has-layout`) answers the family's durable layout probe for a
//!   named volume - the same recover evidence the driver reads - from the
//!   daemon-supplied runtime facet.
//!
//! Everything the effects read crosses the provider boundary as declared
//! facets ([`crate::facets`]): the daemon's Volume runtime (the reconcile
//! and cleanup orchestration over the daemon's own trusted root resolver
//! and durable layout state). Nothing here names a daemon state type.

use std::sync::Arc;

use async_trait::async_trait;
use d2b_contracts_resource::v3::{
    CanonicalJsonObject, CanonicalJsonValue, ResourceRef, ResourceUid, volume::VolumeSpec,
};
use d2b_provider_toolkit::{
    EffectResponse, EffectService, EffectServiceError, EffectServiceFactory, ServiceInvocation,
};
use d2b_resource_types::{ServiceDecl, ServiceMethod};

use crate::driver::VolumeDriverEffects;
use crate::facets::{VolumeEffectFacets, VolumeRuntime};

/// The Volume family's declared effects service.
///
/// One zone-plane method, `has-layout`: it answers whether this zone's
/// durable layout state holds an initialized layout for one volume
/// (`volumeUid`). Payload:
///
/// ```json
/// { "volumeUid": "<uid>" }
/// ```
///
/// Response: `{ "hasLayout": true|false }`.
///
/// The answer is served from the daemon-supplied runtime facet - the same
/// durable layout probe the driver's recover reads - so it proves the
/// daemon's layout state crosses the provider boundary as a declared facet
/// (U7), and it is read-only: no host state is mutated.
///
/// The service is declared on the `Volume` descriptor alone; the family's
/// driver effects (the typed seam) stay the driver's object, not a hosted
/// method surface.
pub const VOLUME_EFFECTS_SERVICE: ServiceDecl = ServiceDecl {
    id: "volume.d2bus.org/effects",
    methods: &[ServiceMethod::zone_plane("has-layout")],
    attach_kinds: &[],
    streams: &[],
    endpoint_policy: None,
};

/// The one `has-layout` response payload: whether the zone retains an
/// initialized layout for the requested volume. The two literals are
/// canonical by construction; the parse refusal is unreachable and names
/// its own code.
fn has_layout_response(has_layout: bool) -> Result<EffectResponse, EffectServiceError> {
    let bytes: &[u8] = if has_layout {
        b"{\"hasLayout\":true}"
    } else {
        b"{\"hasLayout\":false}"
    };
    let payload = CanonicalJsonObject::parse(bytes).map_err(|_| {
        EffectServiceError::Declined {
            service: VOLUME_EFFECTS_SERVICE.id.to_owned(),
            reason: "has-layout-response-invalid".to_owned(),
        }
    })?;
    Ok(EffectResponse::new(payload))
}

/// The declared `has-layout` payload contract: `volumeUid` names the volume
/// the caller asks about.
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

/// Serve the `has-layout` method: parse the volume uid from the canonical
/// payload and answer from the runtime facet's durable layout probe. A
/// payload that does not match the contract refuses with its own closed
/// code instead of answering a half-built report.
async fn serve_has_layout(
    runtime: &dyn VolumeRuntime,
    payload: &CanonicalJsonObject,
) -> Result<EffectResponse, EffectServiceError> {
    let declined = |reason: &'static str| EffectServiceError::Declined {
        service: VOLUME_EFFECTS_SERVICE.id.to_owned(),
        reason: reason.to_owned(),
    };
    let volume_uid = payload_string(payload, "volumeUid", "has-layout-volume-uid-missing")
        .map_err(declined)?;
    let volume_uid =
        ResourceUid::parse(volume_uid).map_err(|_| declined("has-layout-volume-uid-invalid"))?;
    has_layout_response(runtime.has_layout(&volume_uid))
}

/// The provider-owned Volume effects (U7), built from the daemon-supplied
/// facets.
///
/// One value serves both the driver's typed seam and the declared hosted
/// service: the factory constructs it from the same [`VolumeEffectFacets`]
/// the composition root supplies, so the hosted surface and the driver
/// observe the same runtime.
pub struct VolumeEffectsService {
    runtime: Arc<dyn VolumeRuntime>,
}

impl VolumeEffectsService {
    /// Build the effects from one zone's daemon-supplied facet set (R2):
    /// every daemon-structural read rides the facets, never a daemon handle.
    pub fn new(facets: VolumeEffectFacets) -> Self {
        Self {
            runtime: facets.runtime,
        }
    }
}

#[async_trait]
impl VolumeDriverEffects for VolumeEffectsService {
    async fn ensure_layout(
        &self,
        volume_uid: &ResourceUid,
        spec: &VolumeSpec,
        provider: Option<&serde_json::Value>,
        owner_ref: Option<&ResourceRef>,
    ) -> Result<bool, String> {
        self.runtime
            .reconcile_volume(volume_uid, spec, provider, owner_ref)
            .await
    }

    async fn remove_layout(
        &self,
        volume_uid: &ResourceUid,
        spec: &VolumeSpec,
    ) -> Result<(), String> {
        self.runtime.cleanup_volume(volume_uid, spec).await
    }

    fn has_layout(&self, volume_uid: &ResourceUid) -> bool {
        self.runtime.has_layout(volume_uid)
    }
}

#[async_trait]
impl EffectService for VolumeEffectsService {
    async fn handle(
        &self,
        invocation: ServiceInvocation<'_>,
    ) -> Result<EffectResponse, EffectServiceError> {
        // The declaration's method gates admission at the hosting side; the
        // service serves its one declared zone-plane method from the
        // daemon-supplied runtime facet.
        serve_has_layout(&*self.runtime, invocation.payload).await
    }
}

/// The composition-root factory that hosts the Volume effects service in
/// one zone (R5): the daemon registers one per zone, carrying that zone's
/// facet set, and the host rebuilds the service from it on respawn.
pub struct VolumeEffectsServiceFactory {
    facets: VolumeEffectFacets,
}

impl VolumeEffectsServiceFactory {
    /// Build the factory from one zone's facet set.
    pub fn new(facets: VolumeEffectFacets) -> Self {
        Self { facets }
    }
}

impl EffectServiceFactory for VolumeEffectsServiceFactory {
    fn build(&self) -> Arc<dyn EffectService> {
        Arc::new(VolumeEffectsService::new(self.facets.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use d2b_provider_toolkit::ServiceInvocation;
    use d2b_resource_runtime::context::ServiceResourceContext;

    /// A runtime double answering the durable layout probe.
    struct ScriptedRuntime {
        has_layout: bool,
    }

    fn facets(runtime: Arc<ScriptedRuntime>) -> VolumeEffectFacets {
        VolumeEffectFacets { runtime }
    }

    #[async_trait]
    impl VolumeRuntime for ScriptedRuntime {
        async fn reconcile_volume(
            &self,
            _volume_uid: &ResourceUid,
            _spec: &VolumeSpec,
            _provider: Option<&serde_json::Value>,
            _owner_ref: Option<&ResourceRef>,
        ) -> Result<bool, String> {
            unreachable!("the has-layout surface never reconciles")
        }
        async fn cleanup_volume(
            &self,
            _volume_uid: &ResourceUid,
            _spec: &VolumeSpec,
        ) -> Result<(), String> {
            unreachable!("the has-layout surface never cleans up")
        }
        fn has_layout(&self, _volume_uid: &ResourceUid) -> bool {
            self.has_layout
        }
    }

    fn canonical(payload: serde_json::Value) -> CanonicalJsonObject {
        serde_json::from_value(payload).expect("canonical payload")
    }

    fn invocation<'a>(
        payload: &'a CanonicalJsonObject,
        resources: &'a mut ServiceResourceContext,
        invocation_id: &'a str,
    ) -> ServiceInvocation<'a> {
        ServiceInvocation {
            zone: "work",
            method: VOLUME_EFFECTS_SERVICE.methods[0].name,
            invocation_id,
            payload,
            resources,
            state_cells: &[],
            kernel: None,
            request_fds: &[],
            response_fds: VOLUME_EFFECTS_SERVICE.methods[0].response_fds,
            payload_schema: None,
            chain_identities: &[],
        }
    }

    /// The hosted `has-layout` method answers the durable layout probe from
    /// the runtime facet: an initialized layout reports true, a missing one
    /// false, and a payload that does not name a valid volume uid refuses
    /// with its own closed code.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn has_layout_answers_the_durable_layout_probe() {
        let service = VolumeEffectsService::new(facets(Arc::new(ScriptedRuntime {
            has_layout: true,
        })));
        let payload = canonical(serde_json::json!({
            "volumeUid": "6f9619ff-8b86-4d01-b42d-00cf4fc964ff",
        }));
        let mut resources = ServiceResourceContext::fail_closed();
        let response = service
            .handle(invocation(&payload, &mut resources, "invocation-u7"))
            .await
            .expect("call");
        assert_eq!(
            response.payload,
            canonical(serde_json::json!({ "hasLayout": true })),
            "the hosted method answers the probe from the runtime facet"
        );
    }

    /// A volume uid that is absent from the payload refuses with the
    /// method's own closed code instead of answering a half-built report.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn has_layout_refuses_a_payload_without_a_volume_uid() {
        let service = VolumeEffectsService::new(facets(Arc::new(ScriptedRuntime {
            has_layout: false,
        })));
        let payload = canonical(serde_json::json!({}));
        let mut resources = ServiceResourceContext::fail_closed();
        let error = service
            .handle(invocation(&payload, &mut resources, "invocation-u7"))
            .await
            .expect_err("refused");
        assert_eq!(
            error,
            EffectServiceError::Declined {
                service: VOLUME_EFFECTS_SERVICE.id.to_owned(),
                reason: "has-layout-volume-uid-missing".to_owned(),
            },
            "a payload without a volume uid refuses with its own code"
        );
    }

    /// A volume uid that is not a canonical uid refuses with the method's
    /// own closed code.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn has_layout_refuses_an_invalid_volume_uid() {
        let service = VolumeEffectsService::new(facets(Arc::new(ScriptedRuntime {
            has_layout: false,
        })));
        let payload = canonical(serde_json::json!({ "volumeUid": "not-a-uid" }));
        let mut resources = ServiceResourceContext::fail_closed();
        let error = service
            .handle(invocation(&payload, &mut resources, "invocation-u7"))
            .await
            .expect_err("refused");
        assert_eq!(
            error,
            EffectServiceError::Declined {
                service: VOLUME_EFFECTS_SERVICE.id.to_owned(),
                reason: "has-layout-volume-uid-invalid".to_owned(),
            },
            "a payload with an invalid volume uid refuses with its own code"
        );
    }

    }