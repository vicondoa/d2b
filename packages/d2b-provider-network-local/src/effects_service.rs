//! The provider-owned implementation of the Network family's driver effects
//! (U14): the family serves its effects over the daemon-supplied facets
//! instead of a daemon-built port.
//!
//! Two surfaces share one implementation value:
//!
//! - the driver's typed seam, [`NetworkDriverEffects`], which the family's
//!   driver holds (the factory builds it from the same facets);
//! - the declared zone-plane service [`NETWORK_EFFECTS_SERVICE`], hosted
//!   per zone by the daemon through [`NetworkEffectsServiceFactory`]. Its one
//!   method (`inspect-network`) answers the family's trusted-bundle report:
//!   the installed generation identity, the host nftables table the family
//!   projects into, the site's east-west acknowledgement, and the family's
//!   committed operation inventory - the same bundle facts the driver
//!   effects reconcile over.
//!
//! Everything the effects read crosses the provider boundary as declared
//! facets ([`crate::facets`]): the daemon's Network runtime (the reconcile
//! and finalize orchestration over the daemon's own admission, child rows,
//! and readiness state), the trusted bundle, and the kernel-invoking
//! broker's intent source ([`crate::broker::NetworkIntentSource`]). Nothing
//! here names a daemon state type.

use std::sync::Arc;

use async_trait::async_trait;
use d2b_provider_toolkit::{
    EffectResponse, EffectService, EffectServiceError, EffectServiceFactory, ServiceInvocation,
    SharedProviderEffectError, SharedProviderEffectOutcome, SharedProviderEffectRequest,
    SharedProviderFinalize,
};
use d2b_resource_types::{ServiceDecl, ServiceMethod};

use crate::driver::NetworkDriverEffects;
use crate::facets::{NetworkEffectFacets, NetworkRuntime};

/// The Network family's declared effects service.
///
/// One zone-plane method, `inspect-network`: it answers this zone's
/// trusted-bundle report - the installed generation identity, the host
/// nftables table the family's projections land in, the site's east-west
/// acknowledgement, and the family's committed operation inventory. The
/// report is served from the daemon-supplied bundle facet, so it proves the
/// trusted bundle and the installed generation identity cross the provider
/// boundary as declared facets (U14), and it is hermetic: no host state is
/// read or mutated.
///
/// The service is declared on the `Network` descriptor alone; the family's
/// driver effects (the typed seam) stay the driver's object, not a hosted
/// method surface.
pub const NETWORK_EFFECTS_SERVICE: ServiceDecl = ServiceDecl {
    id: "network.d2bus.org/effects",
    methods: &[ServiceMethod::zone_plane("inspect-network")],
    attach_kinds: &[],
    streams: &[],
    endpoint_policy: None,
};

/// The one `inspect-network` response payload: the family's trusted-bundle
/// report. The payload is built through the canonical JSON object path, so
/// a structural character in a trusted value yields a correctly escaped
/// report rather than an unparseable one; the refusal is unreachable and
/// names its own code.
fn inspect_network_response(
    installed_generation_id: &str,
    nftables_family: &str,
    nftables_table: &str,
    east_west_opt_in: bool,
) -> Result<EffectResponse, EffectServiceError> {
    let payload = serde_json::from_value(serde_json::json!({
        "family": "network-local",
        "resourceType": "Network",
        "installedGenerationId": installed_generation_id,
        "hostNftables": {
            "family": nftables_family,
            "table": nftables_table,
        },
        "eastWestOptIn": east_west_opt_in,
        "operations": [
            "ApplyNftables", "ApplyNftablesProjection", "ApplyNmUnmanaged",
            "ApplyRoute", "ApplySysctl", "CreateBridge", "DeleteBridge",
            "CreatePersistentTap", "DeletePersistentTap", "CreateTapFd",
            "SetBridgePortFlags", "UpdateHostsFile", "SeedDnsmasqLease",
        ],
    }))
    .map_err(|_| EffectServiceError::Declined {
        service: NETWORK_EFFECTS_SERVICE.id.to_owned(),
        reason: "inspect-network-response-invalid".to_owned(),
    })?;
    Ok(EffectResponse::new(payload))
}

/// Serve the `inspect-network` method: answer the family's trusted-bundle
/// report from the daemon-supplied bundle facet. A bundle that carries no
/// installed generation identity refuses with its own closed code instead
/// of answering a half-built report.
async fn serve_inspect_network(
    runtime: &dyn NetworkRuntime,
) -> Result<EffectResponse, EffectServiceError> {
    let declined = |reason: &'static str| EffectServiceError::Declined {
        service: NETWORK_EFFECTS_SERVICE.id.to_owned(),
        reason: reason.to_owned(),
    };
    let bundle = runtime.bundle();
    let installed = bundle
        .installed_generation_identity()
        .ok_or_else(|| declined("inspect-network-installed-generation-unavailable"))?;
    inspect_network_response(
        installed.as_str(),
        &bundle.host.nftables.family,
        &bundle.host.nftables.table,
        bundle.host.site.allow_unsafe_east_west,
    )
}

/// The provider-owned Network effects (U14), built from the daemon-supplied
/// facets.
///
/// One value serves both the driver's typed seam and the declared hosted
/// service: the factory constructs it from the same [`NetworkEffectFacets`]
/// the composition root supplies, so the hosted surface and the driver
/// observe the same runtime.
pub struct NetworkEffectsService {
    runtime: Arc<dyn NetworkRuntime>,
}

impl NetworkEffectsService {
    /// Build the effects from one zone's daemon-supplied facet set (R2):
    /// every daemon-structural read rides the facets, never a daemon handle.
    pub fn new(facets: NetworkEffectFacets) -> Self {
        Self {
            runtime: facets.runtime,
        }
    }
}

#[async_trait]
impl NetworkDriverEffects for NetworkEffectsService {
    async fn reconcile_network(
        &self,
        request: &SharedProviderEffectRequest<'_>,
    ) -> Result<SharedProviderEffectOutcome, SharedProviderEffectError> {
        self.runtime.reconcile_network(request).await
    }

    async fn finalize(
        &self,
        request: &SharedProviderEffectRequest<'_>,
    ) -> Result<SharedProviderFinalize, SharedProviderEffectError> {
        self.runtime.finalize_network(request).await
    }
}

#[async_trait]
impl EffectService for NetworkEffectsService {
    async fn handle(
        &self,
        _invocation: ServiceInvocation<'_>,
    ) -> Result<EffectResponse, EffectServiceError> {
        // The declaration's method gates admission at the hosting side; the
        // service serves its one declared zone-plane method from the
        // daemon-supplied bundle facet.
        serve_inspect_network(&*self.runtime).await
    }
}

/// The composition-root factory that hosts the Network effects service in
/// one zone (R5): the daemon registers one per zone, carrying that zone's
/// facet set, and the host rebuilds the service from it on respawn.
pub struct NetworkEffectsServiceFactory {
    facets: NetworkEffectFacets,
}

impl NetworkEffectsServiceFactory {
    /// Build the factory from one zone's facet set.
    pub fn new(facets: NetworkEffectFacets) -> Self {
        Self { facets }
    }
}

impl EffectServiceFactory for NetworkEffectsServiceFactory {
    fn build(&self) -> Arc<dyn EffectService> {
        Arc::new(NetworkEffectsService::new(self.facets.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use d2b_contracts_resource::v3::{canonical_json_bytes, CanonicalJsonObject, CanonicalJsonValue};
    use d2b_provider_toolkit::ServiceInvocation;
    use d2b_resource_runtime::context::ServiceResourceContext;

    /// A bundle-facet double answering the trusted-bundle report.
    struct ScriptedRuntime {
        bundle: d2b_core::bundle_resolver::BundleResolver,
    }

    fn facets(runtime: Arc<ScriptedRuntime>) -> NetworkEffectFacets {
        NetworkEffectFacets { runtime }
    }

    #[async_trait]
    impl NetworkRuntime for ScriptedRuntime {
        fn bundle(&self) -> std::sync::Arc<d2b_core::bundle_resolver::BundleResolver> {
            std::sync::Arc::new(self.bundle.clone())
        }
        fn broker_socket_path(&self) -> &std::path::Path {
            unreachable!("the inspect-network surface reads no broker socket")
        }
        fn caller_role(&self) -> d2b_contracts_broker::broker_wire::BrokerCallerRole {
            unreachable!("the inspect-network surface invokes no kernel")
        }
        async fn reconcile_network(
            &self,
            _request: &SharedProviderEffectRequest<'_>,
        ) -> Result<SharedProviderEffectOutcome, SharedProviderEffectError> {
            unreachable!("the inspect-network surface never reconciles")
        }
        async fn finalize_network(
            &self,
            _request: &SharedProviderEffectRequest<'_>,
        ) -> Result<SharedProviderFinalize, SharedProviderEffectError> {
            unreachable!("the inspect-network surface never finalizes")
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
            response_fds: NETWORK_EFFECTS_SERVICE.methods[0].response_fds,
            payload_schema: None,
        }
    }

    /// The hosted `inspect-network` method answers the trusted-bundle report
    /// from the bundle facet: the installed generation identity, the host
    /// nftables table, the east-west acknowledgement, and the family's
    /// committed operation inventory.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn inspect_network_answers_the_trusted_bundle_report() {
        let service = NetworkEffectsService::new(facets(Arc::new(ScriptedRuntime {
            bundle: test_bundle(),
        })));
        let payload = canonical(serde_json::json!({}));
        let mut resources = ServiceResourceContext::fail_closed();
        let response = service
            .handle(invocation(&payload, &mut resources, "invocation-u14"))
            .await
            .expect("served");
        assert_eq!(
            response.payload,
            canonical(serde_json::json!({
                "family": "network-local",
                "resourceType": "Network",
                "installedGenerationId":
                    "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
                "hostNftables": { "family": "inet", "table": "d2b" },
                "eastWestOptIn": false,
                "operations": [
                    "ApplyNftables", "ApplyNftablesProjection", "ApplyNmUnmanaged",
                    "ApplyRoute", "ApplySysctl", "CreateBridge", "DeleteBridge",
                    "CreatePersistentTap", "DeletePersistentTap", "CreateTapFd",
                    "SetBridgePortFlags", "UpdateHostsFile", "SeedDnsmasqLease",
                ],
            })),
        );
    }

    /// A bundle that carries no installed generation identity refuses with
    /// its own closed code instead of answering a half-built report.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn inspect_network_refuses_without_an_installed_generation() {
        let service = NetworkEffectsService::new(facets(Arc::new(ScriptedRuntime {
            bundle: test_bundle_without_installed_generation(),
        })));
        let payload = canonical(serde_json::json!({}));
        let mut resources = ServiceResourceContext::fail_closed();
        let error = service
            .handle(invocation(&payload, &mut resources, "invocation-u14-b"))
            .await
            .expect_err("refused");
        assert_eq!(
            error,
            EffectServiceError::Declined {
                service: NETWORK_EFFECTS_SERVICE.id.to_owned(),
                reason: "inspect-network-installed-generation-unavailable".to_owned(),
            }
        );
    }

    /// The committed family operation inventory the report carries, in the
    /// committed catalog order, byte for byte.
    #[test]
    fn inspect_network_operations_match_the_committed_inventory() {
        let committed = crate::operations::network_family_operations()
            .iter()
            .map(|operation| operation.operation_ref.name().as_str().to_owned())
            .collect::<Vec<_>>();
        let expected = [
            "apply-nftables",
            "apply-nftables-projection",
            "apply-nm-unmanaged",
            "apply-route",
            "apply-sysctl",
            "create-bridge",
            "delete-bridge",
            "create-persistent-tap",
            "delete-persistent-tap",
            "create-tap-fd",
            "set-bridge-port-flags",
            "update-hosts-file",
            "seed-dnsmasq-lease",
        ];
        assert_eq!(committed, expected);
    }

    /// The report's operation spellings are the committed PascalCase wire
    /// names, byte for byte (the catalog's spellings, not the lowercase
    /// operation references).
    #[test]
    fn inspect_network_report_spells_the_catalog_wire_names() {
        let report = inspect_network_response("sha256:abc", "inet", "d2b", false)
            .expect("report")
            .payload;
        let bytes = canonical_json_bytes(&report).expect("canonical");
        let text = String::from_utf8(bytes).expect("utf8");
        for wire_name in [
            "ApplyNftables",
            "ApplyNftablesProjection",
            "ApplyNmUnmanaged",
            "ApplyRoute",
            "ApplySysctl",
            "CreateBridge",
            "CreatePersistentTap",
            "CreateTapFd",
            "DeleteBridge",
            "DeletePersistentTap",
            "SeedDnsmasqLease",
            "SetBridgePortFlags",
            "UpdateHostsFile",
        ] {
            assert!(text.contains(wire_name), "missing {wire_name} in {text}");
        }
    }

    /// A structural character inside a trusted value yields a correctly
    /// escaped report instead of an unparseable one: the payload is built
    /// through the canonical JSON object path, never string interpolation.
    #[test]
    fn inspect_network_escapes_a_structural_character_in_a_trusted_value() {
        let report = inspect_network_response("sha256:abc\"def", "inet", "d2b", false)
            .expect("report")
            .payload;
        assert_eq!(
            report.get("installedGenerationId"),
            Some(&CanonicalJsonValue::String("sha256:abc\"def".to_owned())),
        );
        let bytes = canonical_json_bytes(&report).expect("canonical");
        assert!(
            String::from_utf8(bytes).expect("utf8").contains(r#"sha256:abc\"def"#),
            "the escaped value survives the canonical round-trip"
        );
    }

    /// The installed generation identity the fixture bundle carries.
    const INSTALLED_GENERATION: &str =
        "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn fixture_bundle(bundle_hash: Option<&str>) -> d2b_core::bundle_resolver::BundleResolver {
        use d2b_core::bundle::{Bundle, BundleGeneration};
        use d2b_core::host::HostJson;
        use d2b_core::manifest_v04::ManifestV04;
        use d2b_core::processes::ProcessesJson;
        use std::collections::BTreeMap;

        let host = serde_json::from_str::<HostJson>(include_str!(
            "../../../tests/fixtures/deny-unknown/host-valid.json"
        ))
        .expect("host fixture");
        let manifest = ManifestV04::from_slice(
            include_str!("../../../tests/golden/manifest_v04/baseline-vms.json").as_bytes(),
        )
        .expect("manifest fixture");
        d2b_core::bundle_resolver::BundleResolver::from_artifacts_with_zone_resource_bundles(
            Bundle {
                bundle_version: 1,
                schema_version: "v3".to_owned(),
                privileges_path: "privileges.json".to_owned(),
                storage_path: None,
                realm_workloads_launcher_v2_path: None,
                generation: BundleGeneration {
                    generator: "test".to_owned(),
                    source_revision: None,
                    generated_at: None,
                },
                bundle_hash: bundle_hash.map(str::to_owned),
                artifact_hashes: None,
            },
            host,
            ProcessesJson {
                schema_version: "v2".to_owned(),
                vms: Vec::new(),
            },
            manifest,
            BTreeMap::new(),
        )
    }

    fn test_bundle() -> d2b_core::bundle_resolver::BundleResolver {
        fixture_bundle(Some(INSTALLED_GENERATION))
    }

    fn test_bundle_without_installed_generation(
    ) -> d2b_core::bundle_resolver::BundleResolver {
        fixture_bundle(None)
    }
}