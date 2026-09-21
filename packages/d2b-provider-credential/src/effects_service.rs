//! The provider-owned implementation of the Credential family's driver
//! effects (U8): the family serves its effects from this crate instead of a
//! daemon-built port.
//!
//! Two surfaces share one implementation value:
//!
//! - the driver's typed seam, [`CredentialDriverEffects`], which the
//!   family's driver holds (the factory builds it from the same facets);
//! - the declared zone-plane service [`CREDENTIAL_EFFECTS_SERVICE`], hosted
//!   per zone by the daemon through [`CredentialEffectsServiceFactory`]. Its
//!   one method (`inspect-credential`) answers the family's committed
//!   surface: the three backend Provider references the admission set is
//!   built from and the signed managed-identity agent binary - the same
//!   identities the driver's validate and reconcile verbs classify over.
//!
//! Everything the effects read crosses the provider boundary as declared
//! facets ([`crate::facets`]): the daemon's Credential runtime (the
//! preserved Provider and execution-target reads, the lease-facts read,
//! the managed-identity agent probe, and the authenticated Provider
//! session handoff registry). Nothing here names a daemon state type, and
//! no credential material crosses the boundary: the runtime answers typed
//! facts and hands back the same session objects the daemon's
//! ProviderSupervisor registry holds.

use std::sync::Arc;

use async_trait::async_trait;
use d2b_contracts_resource::v3::ResourceRef;
use d2b_provider_toolkit::{
    EffectResponse, EffectService, EffectServiceError, EffectServiceFactory, ServiceInvocation,
};
use d2b_resource_types::{ServiceDecl, ServiceMethod};

use crate::driver::{CredentialDependencyFacts, CredentialDriverEffects, CredentialLeaseFacts};
use crate::facets::CredentialEffectFacets;
use crate::session::CredentialSession;

/// The Credential family's declared effects service.
///
/// One zone-plane method, `inspect-credential`: it answers this family's
/// committed surface - the three backend Provider references the admission
/// set is built from and the signed managed-identity agent binary the
/// driver mints - the same identities the driver's validate and reconcile
/// verbs classify over. The report is served from the crate's own declared
/// constants, so it proves the family's committed identities live in the
/// owning crate (U8), and it is hermetic: no host state is read and no
/// credential material is answered.
///
/// The service is declared on the `Credential` descriptor alone; the
/// family's driver effects (the typed seam) stay the driver's object, not a
/// hosted method surface.
pub const CREDENTIAL_EFFECTS_SERVICE: ServiceDecl = ServiceDecl {
    id: "credential.d2bus.org/effects",
    methods: &[ServiceMethod::zone_plane("inspect-credential")],
    attach_kinds: &[],
    streams: &[],
    endpoint_policy: None,
};

/// The one `inspect-credential` response payload: the family's committed
/// surface. The payload is built through the canonical JSON object path, so
/// a structural character in a committed identity yields a correctly
/// escaped report rather than an unparseable one; the refusal is
/// unreachable and names its own code.
fn inspect_credential_response() -> Result<EffectResponse, EffectServiceError> {
    let payload = serde_json::from_value(serde_json::json!({
        "family": "credential",
        "resourceType": "Credential",
        "providers": [
            crate::SECRET_SERVICE_PROVIDER_REF,
            crate::ENTRA_PROVIDER_REF,
            crate::MANAGED_IDENTITY_PROVIDER_REF,
        ],
        "agentBinary": crate::CREDENTIAL_AGENT_BINARY,
    }))
    .map_err(|_| EffectServiceError::Declined {
        service: CREDENTIAL_EFFECTS_SERVICE.id.to_owned(),
        reason: "inspect-credential-response-invalid".to_owned(),
    })?;
    Ok(EffectResponse::new(payload))
}

/// Serve the `inspect-credential` method: answer the family's committed
/// surface from the crate's own declared identities. The report is
/// hermetic - it reads no host state and answers no credential material -
/// so it needs no runtime facet.
async fn serve_inspect_credential() -> Result<EffectResponse, EffectServiceError> {
    inspect_credential_response()
}

/// The provider-owned Credential effects (U8), built from the daemon-
/// supplied facets.
///
/// One value serves both the driver's typed seam and the declared hosted
/// service: the factory constructs it from the same [`CredentialEffectFacets`]
/// the composition root supplies, so the hosted surface and the driver
/// observe the same runtime.
pub struct CredentialEffectsService {
    runtime: Arc<dyn crate::facets::CredentialRuntime>,
}

impl CredentialEffectsService {
    /// Build the effects from one zone's daemon-supplied facet set (R2):
    /// every daemon-structural read rides the facets, never a daemon handle.
    pub fn new(facets: CredentialEffectFacets) -> Self {
        Self {
            runtime: facets.runtime,
        }
    }
}

#[async_trait]
impl CredentialDriverEffects for CredentialEffectsService {
    async fn dependency_facts(
        &self,
        provider_ref: &ResourceRef,
        execution_ref: &ResourceRef,
    ) -> Option<CredentialDependencyFacts> {
        self.runtime.dependency_facts(provider_ref, execution_ref).await
    }

    async fn lease_facts(&self, credential_ref: &ResourceRef) -> Option<CredentialLeaseFacts> {
        self.runtime.lease_facts(credential_ref).await
    }

    async fn agent_ready(&self, agent_ref: &ResourceRef) -> bool {
        self.runtime.agent_ready(agent_ref).await
    }

    fn session(&self, provider_ref: &ResourceRef) -> Option<Arc<dyn CredentialSession>> {
        self.runtime.session(provider_ref)
    }
}

#[async_trait]
impl EffectService for CredentialEffectsService {
    async fn handle(
        &self,
        _invocation: ServiceInvocation<'_>,
    ) -> Result<EffectResponse, EffectServiceError> {
        // The declaration's method gates admission at the hosting side; the
        // service serves its one declared zone-plane method from the
        // crate's own committed identities.
        serve_inspect_credential().await
    }
}

/// The composition-root factory that hosts the Credential effects service in
/// one zone (R5): the daemon registers one per zone, carrying that zone's
/// facet set, and the host rebuilds the service from it on respawn.
pub struct CredentialEffectsServiceFactory {
    facets: CredentialEffectFacets,
}

impl CredentialEffectsServiceFactory {
    /// Build the factory from one zone's facet set.
    pub fn new(facets: CredentialEffectFacets) -> Self {
        Self { facets }
    }
}

impl EffectServiceFactory for CredentialEffectsServiceFactory {
    fn build(&self) -> Arc<dyn EffectService> {
        Arc::new(CredentialEffectsService::new(self.facets.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use d2b_contracts_resource::v3::CanonicalJsonObject;
    use d2b_provider_toolkit::ServiceInvocation;
    use d2b_resource_runtime::context::ServiceResourceContext;

    /// A runtime facet double: the `inspect-credential` surface never reads
    /// it, so every method is unreachable.
    struct UnusedRuntime;

    #[async_trait]
    impl crate::facets::CredentialRuntime for UnusedRuntime {
        async fn dependency_facts(
            &self,
            _provider_ref: &ResourceRef,
            _execution_ref: &ResourceRef,
        ) -> Option<CredentialDependencyFacts> {
            unreachable!("the inspect-credential surface reads no facts")
        }
        async fn lease_facts(
            &self,
            _credential_ref: &ResourceRef,
        ) -> Option<CredentialLeaseFacts> {
            unreachable!("the inspect-credential surface reads no lease facts")
        }
        async fn agent_ready(&self, _agent_ref: &ResourceRef) -> bool {
            unreachable!("the inspect-credential surface probes no agent")
        }
        fn session(&self, _provider_ref: &ResourceRef) -> Option<Arc<dyn CredentialSession>> {
            unreachable!("the inspect-credential surface binds no session")
        }
    }

    fn facets() -> CredentialEffectFacets {
        CredentialEffectFacets {
            runtime: Arc::new(UnusedRuntime),
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
            invocation_id,
            payload,
            resources,
            state_cells: &[],
            kernel: None,
            request_fds: &[],
            response_fds: CREDENTIAL_EFFECTS_SERVICE.methods[0].response_fds,
            payload_schema: None,
        }
    }

    /// The hosted `inspect-credential` method answers the family's committed
    /// surface from the crate's own declared identities: the three backend
    /// Provider references and the signed agent binary, with no host state
    /// and no credential material in the report.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn inspect_credential_answers_the_committed_family_surface() {
        let service = CredentialEffectsService::new(facets());
        let mut resources = ServiceResourceContext::fail_closed();
        let payload = canonical(serde_json::json!({}));
        let call = invocation(&payload, &mut resources, "invocation-u8");
        let response = service.handle(call).await.expect("call");
        assert_eq!(
            response.payload,
            serde_json::from_value::<CanonicalJsonObject>(serde_json::json!({
                "family": "credential",
                "resourceType": "Credential",
                "providers": [
                    "Provider/credential-secret-service",
                    "Provider/credential-entra",
                    "Provider/credential-managed-identity",
                ],
                "agentBinary": "d2b-managed-identity-agent",
            }))
            .expect("canonical payload"),
            "the hosted service answers the committed family surface"
        );
        assert!(
            response.fds.is_empty(),
            "the inspect-credential surface mints no descriptors"
        );
    }

    /// The factory rebuilds the service from the same facet set, so a
    /// respawn answers the same committed surface the driver factories are
    /// built from.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn factory_builds_the_service_over_the_same_facet_set() {
        let factory = CredentialEffectsServiceFactory::new(facets());
        let service = factory.build();
        let mut resources = ServiceResourceContext::fail_closed();
        let payload = canonical(serde_json::json!({}));
        let call = invocation(&payload, &mut resources, "invocation-u8-factory");
        let response = service.handle(call).await.expect("call");
        assert_eq!(
            response.payload,
            serde_json::from_value::<CanonicalJsonObject>(serde_json::json!({
                "family": "credential",
                "resourceType": "Credential",
                "providers": [
                    "Provider/credential-secret-service",
                    "Provider/credential-entra",
                    "Provider/credential-managed-identity",
                ],
                "agentBinary": "d2b-managed-identity-agent",
            }))
            .expect("canonical payload"),
            "the factory-built service answers the committed family surface"
        );
    }
}