//! Test-support recording doubles for `d2b-provider-network-local`.
//!
//! The crate's canonical recording [`NetworkRuntime`] double lives here so
//! the crate's own unit tests and `d2bd`'s plane tests share one shape,
//! rather than each defining its own bespoke fake.

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use d2b_contracts_broker::broker_wire::BrokerCallerRole;
use d2b_provider_toolkit::{
    SharedProviderEffectError, SharedProviderEffectOutcome, SharedProviderEffectPhase,
    SharedProviderEffectRequest, SharedProviderFinalize,
};

use crate::facets::{NetworkEffectFacets, NetworkRuntime};

/// The installed generation the fixture bundle carries: sha256 plus 64
/// lowercase hex digits, so the bundle resolves an installed generation
/// identity.
const INSTALLED_GENERATION: &str =
    "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// Recording [`NetworkRuntime`] double.
///
/// Every effect call is appended to an ordered [`Self::call_order`] log while
/// the per-verb counters (`reconciled`, `finalized`) keep counting, so the
/// plane can assert both event ordering and invocation counts. The double
/// answers the trusted-bundle report from a fixture bundle (the same
/// artifact shapes the daemon's own tests load); the broker facets are
/// unreachable because the driver tests never invoke a kernel.
#[derive(Default)]
pub struct RecordingRuntime {
    /// Ordered log of every effect call, oldest first.
    calls: parking_lot::Mutex<Vec<&'static str>>,
    /// Number of [`NetworkRuntime::reconcile_network`] invocations.
    pub reconciled: parking_lot::Mutex<usize>,
    /// Number of [`NetworkRuntime::finalize_network`] invocations.
    pub finalized: parking_lot::Mutex<usize>,
}

impl RecordingRuntime {
    /// The ordered effect calls, oldest first.
    pub fn call_order(&self) -> Vec<&'static str> {
        self.calls.lock().clone()
    }
}

#[async_trait]
impl NetworkRuntime for RecordingRuntime {
    fn bundle(&self) -> Arc<d2b_core::bundle_resolver::BundleResolver> {
        Arc::new(FIXTURE_BUNDLE.clone())
    }

    fn broker_socket_path(&self) -> &Path {
        unreachable!("the driver tests never invoke a kernel")
    }

    fn caller_role(&self) -> BrokerCallerRole {
        unreachable!("the driver tests never invoke a kernel")
    }

    async fn reconcile_network(
        &self,
        _request: &SharedProviderEffectRequest<'_>,
    ) -> Result<SharedProviderEffectOutcome, SharedProviderEffectError> {
        self.calls.lock().push("reconcile"); // async-gate-allow: test-support recorder lock
        *self.reconciled.lock() += 1; // async-gate-allow: test-support recorder lock
        Ok(SharedProviderEffectOutcome::phase(
            SharedProviderEffectPhase::Ready,
        ))
    }

    async fn finalize_network(
        &self,
        _request: &SharedProviderEffectRequest<'_>,
    ) -> Result<SharedProviderFinalize, SharedProviderEffectError> {
        self.calls.lock().push("finalize"); // async-gate-allow: test-support recorder lock
        *self.finalized.lock() += 1; // async-gate-allow: test-support recorder lock
        Ok(SharedProviderFinalize::Complete)
    }
}

/// The facet set one recording runtime serves.
pub fn recording_facets(runtime: Arc<RecordingRuntime>) -> NetworkEffectFacets {
    NetworkEffectFacets { runtime }
}

/// The fixture bundle the recording runtime reports.
static FIXTURE_BUNDLE: std::sync::LazyLock<d2b_core::bundle_resolver::BundleResolver> =
    std::sync::LazyLock::new(fixture_bundle);

fn fixture_bundle() -> d2b_core::bundle_resolver::BundleResolver {
    use d2b_core::bundle::Bundle;
    use std::collections::BTreeMap;

    let host = serde_json::from_str::<d2b_core::host::HostJson>(include_str!(
        "../../../tests/fixtures/deny-unknown/host-valid.json"
    ))
    .expect("host fixture");
    let manifest = d2b_core::manifest_v04::ManifestV04::from_slice(
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
            generation: d2b_core::bundle::BundleGeneration {
                generator: "test".to_owned(),
                source_revision: None,
                generated_at: None,
            },
            bundle_hash: Some(INSTALLED_GENERATION.to_owned()),
            artifact_hashes: None,
        },
        host,
        d2b_core::processes::ProcessesJson {
            schema_version: "v2".to_owned(),
            vms: Vec::new(),
        },
        manifest,
        BTreeMap::new(),
    )
}