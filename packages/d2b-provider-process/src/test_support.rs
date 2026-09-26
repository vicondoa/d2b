//! Test-support recording double for the [`ProcessProviderRuntime`] facet
//! (U1).
//!
//! The canonical scripted runtime facet for the Process family: records
//! every call with the exact ticket inputs the driver derived (KTD7) and
//! replays a scripted adoption/liveness/launch sequence. The family's
//! effects implementation runs over it exactly as over the composed
//! production runtime, so driver and plane tests observe the real seam.
//! Gated behind the `test-support` Cargo feature (available automatically
//! under `cargo test`), so production consumers never pull it in. The plane
//! tests in `d2bd` reach it through the same public surface.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use std::collections::BTreeMap;

use d2b_contracts_resource::v3::process::{EphemeralProcessSpec, ExecutionSpec, ProcessSpec};
use d2b_contracts_resource::v3::{ResourceRef, ResourceUid, SchemaFingerprint, ZoneId};
use d2b_core::bundle::{Bundle, BundleGeneration};
use d2b_core::bundle_resolver::BundleResolver;
use d2b_core::processes::ProcessesJson;
use d2b_process_conformance::{AdoptionCandidate, ProcessIdentityDigest};
use parking_lot::Mutex;

use crate::effects::{ProviderAdoption, ProviderLiveness};
use crate::facets::{
    ProcessEffectFacets, ProcessProviderRuntime, ProcessResourceContext, ProviderLaunch,
};

/// One recorded launch with the ticket inputs the driver derived.
#[derive(Clone, Debug)]
pub struct RecordedLaunch {
    /// Which family arm launched: `Process` or `EphemeralProcess`.
    pub kind: &'static str,
    /// The canonical resource reference the launch was recorded against.
    pub resource_ref: String,
    /// The recorded resource uid.
    pub resource_uid: String,
    /// The recorded resource generation.
    pub generation: u64,
    /// The zone the launch was recorded against.
    pub zone: ZoneId,
    /// The zone-authority uid the driver derived, if any.
    pub zone_uid: Option<ResourceUid>,
    /// The zone policy revision the driver derived, if any.
    pub policy_revision: Option<u64>,
    /// The canonical provider reference the launch was recorded against.
    pub provider_ref: String,
    /// The launch template from the spec.
    pub template: String,
    /// The canonical execution reference the launch was recorded against.
    pub execution_ref: String,
    /// The one-shot launch budget (`startDeadline`) the driver derived.
    pub start_deadline_ms: Option<u64>,
}

/// One recorded stop with the timeouts the driver derived.
#[derive(Clone, Debug)]
pub struct RecordedStop {
    /// Which family arm stopped: `Process` or `EphemeralProcess`.
    pub kind: &'static str,
    /// The term timeout the stop was issued with.
    pub term_timeout: Duration,
    /// The kill timeout the stop was issued with.
    pub kill_timeout: Duration,
}

/// Configuration for [`FakeFacets`]: the scripted adoption/liveness/launch
/// outcomes the double replays in order.
#[derive(Clone)]
pub struct FakeFacetsConfig {
    /// Scripted adoption results; the last one repeats once exhausted.
    pub adoption: VecDeque<ProviderAdoption>,
    /// When set, one-shot adoption refuses with this provider error.
    pub adopt_error: Option<String>,
    /// Scripted liveness results; the last one repeats once exhausted
    /// (default `Alive`).
    pub liveness: VecDeque<ProviderLiveness>,
    /// The scripted launch/launch-ephemeral result.
    pub launch: Result<ProcessIdentityDigest, String>,
    /// Whether the fake reports a live retained identity.
    pub active: bool,
}

impl Default for FakeFacetsConfig {
    fn default() -> Self {
        Self {
            adoption: VecDeque::from([ProviderAdoption::Absent]),
            adopt_error: None,
            liveness: VecDeque::new(),
            launch: Ok(ProcessIdentityDigest::from_bytes([0x51; 32])),
            active: true,
        }
    }
}

/// Scripted [`ProcessProviderRuntime`] facet double: records every call with
/// the exact ticket inputs the driver derived (KTD7) and replays a scripted
/// adoption sequence.
pub struct FakeFacets {
    config: Mutex<FakeFacetsConfig>,
    calls: Mutex<Vec<&'static str>>,
    launches: Mutex<Vec<RecordedLaunch>>,
    stops: Mutex<Vec<RecordedStop>>,
    finalizes: Mutex<usize>,
    bundle: BundleResolver,
    socket_runtime_dir: std::path::PathBuf,
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
impl FakeFacets {
    /// Build a double from the given scripted configuration.
    pub fn new(config: FakeFacetsConfig) -> Self {
        Self {
            config: Mutex::new(config),
            calls: Mutex::new(Vec::new()),
            launches: Mutex::new(Vec::new()),
            stops: Mutex::new(Vec::new()),
            finalizes: Mutex::new(0),
            bundle: fixture_bundle(),
            socket_runtime_dir: std::path::PathBuf::from("/run/d2b"),
        }
    }

    /// The facet set the driver and the service factory are built from.
    pub fn facet_set(self: &Arc<Self>) -> ProcessEffectFacets {
        ProcessEffectFacets {
            runtime: Arc::clone(self) as Arc<dyn ProcessProviderRuntime>,
            committed: None,
            guest_owners: None,
        }
    }

    /// Queue one adoption result; the queue empties toward the default.
    pub fn push_adoption(&self, adoption: ProviderAdoption) {
        self.config.lock().adoption.push_back(adoption);
    }

    /// Queue one liveness result; the queue empties toward `Alive`.
    pub fn push_liveness(&self, liveness: ProviderLiveness) {
        self.config.lock().liveness.push_back(liveness);
    }

    /// Script the launch/launch-ephemeral result.
    pub fn set_launch(&self, result: Result<ProcessIdentityDigest, String>) {
        self.config.lock().launch = result;
    }

    /// Flip the retained-identity report (the Provider records one on a
    /// successful launch, so a row can be launched first and then read as
    /// live).
    pub fn set_active(&self, active: bool) {
        self.config.lock().active = active;
    }

    /// The recorded launch calls, in order.
    pub fn launch_calls(&self) -> Vec<RecordedLaunch> {
        self.launches.lock().clone()
    }

    /// The recorded stop calls, in order.
    pub fn stop_calls(&self) -> Vec<RecordedStop> {
        self.stops.lock().clone()
    }

    /// The recorded finalize count.
    pub fn finalize_calls(&self) -> usize {
        *self.finalizes.lock()
    }

    /// The recorded effect call order.
    pub fn call_order(&self) -> Vec<&'static str> {
        self.calls.lock().clone()
    }
}

/// Build a [`RecordedLaunch`] for one recorded launch of the given family
/// arm, from the provider-layer context and execution spec the driver
/// derived.
pub fn recorded_launch(
    kind: &'static str,
    context: &ProcessResourceContext<'_>,
    execution: &ExecutionSpec,
    start_deadline_ms: Option<u64>,
) -> RecordedLaunch {
    RecordedLaunch {
        kind,
        resource_ref: context.resource_ref.to_canonical_string(),
        resource_uid: context.resource_uid.as_str().to_owned(),
        generation: context.resource_generation.get(),
        zone: context.zone.clone(),
        zone_uid: context.zone_uid.clone(),
        policy_revision: context.policy_revision,
        provider_ref: context.provider_ref.to_canonical_string(),
        template: execution.template().as_str().to_owned(),
        execution_ref: execution.execution_ref().to_canonical_string(),
        start_deadline_ms,
    }
}

#[async_trait::async_trait]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
impl ProcessProviderRuntime for FakeFacets {
    fn bundle(&self) -> &BundleResolver {
        &self.bundle
    }

    fn socket_runtime_dir(&self) -> &Path {
        &self.socket_runtime_dir
    }

    fn guest_setup_descriptor_digest(
        &self,
        _zone: &ZoneId,
        _guest_ref: &ResourceRef,
    ) -> Option<SchemaFingerprint> {
        None
    }

    async fn launch_resource(
        &self,
        context: ProcessResourceContext<'_>,
        spec: &ProcessSpec,
        _timeout: Duration,
    ) -> Result<ProviderLaunch, String> {
        self.calls.lock().push("launch"); // async-gate-allow: test-support recorder lock
        self.launches
            .lock() // async-gate-allow: test-support recorder lock
            .push(recorded_launch("Process", &context, spec.execution(), None));
        self.config.lock().launch.clone().map(launch_identity) // async-gate-allow: test-support recorder lock
    }

    async fn launch_ephemeral_resource(
        &self,
        context: ProcessResourceContext<'_>,
        spec: &EphemeralProcessSpec,
        timeout: Duration,
    ) -> Result<ProviderLaunch, String> {
        self.calls.lock().push("launch-ephemeral"); // async-gate-allow: test-support recorder lock
        assert_eq!(
            timeout,
            Duration::from_millis(spec.start_deadline().as_millis()),
            "the one-shot launch budget is the spec's startDeadline",
        );
        self.launches.lock().push(recorded_launch( // async-gate-allow: test-support recorder lock
            "EphemeralProcess",
            &context,
            spec.execution(),
            Some(spec.start_deadline().as_millis()),
        ));
        self.config.lock().launch.clone().map(launch_identity) // async-gate-allow: test-support recorder lock
    }

    async fn adopt_resource(
        &self,
        _context: ProcessResourceContext<'_>,
        _spec: &ProcessSpec,
    ) -> Result<ProviderAdoption, String> {
        self.calls.lock().push("adopt"); // async-gate-allow: test-support recorder lock
        let mut config = self.config.lock(); // async-gate-allow: test-support recorder lock
        Ok(config
            .adoption
            .pop_front()
            .unwrap_or(ProviderAdoption::Absent))
    }

    async fn probe_resource(
        &self,
        _context: ProcessResourceContext<'_>,
        _spec: &ProcessSpec,
    ) -> Result<ProviderLiveness, String> {
        self.calls.lock().push("probe"); // async-gate-allow: test-support recorder lock
        let mut config = self.config.lock(); // async-gate-allow: test-support recorder lock
        Ok(config
            .liveness
            .pop_front()
            .unwrap_or(ProviderLiveness::Alive))
    }

    async fn adopt_ephemeral_resource(
        &self,
        _context: ProcessResourceContext<'_>,
        _spec: &EphemeralProcessSpec,
    ) -> Result<ProviderAdoption, String> {
        self.calls.lock().push("adopt-ephemeral"); // async-gate-allow: test-support recorder lock
        let mut config = self.config.lock(); // async-gate-allow: test-support recorder lock
        if let Some(error) = config.adopt_error.clone() {
            return Err(error);
        }
        Ok(config
            .adoption
            .pop_front()
            .unwrap_or(ProviderAdoption::Absent))
    }

    async fn probe_ephemeral_resource(
        &self,
        _context: ProcessResourceContext<'_>,
        _spec: &EphemeralProcessSpec,
    ) -> Result<ProviderLiveness, String> {
        self.calls.lock().push("probe-ephemeral"); // async-gate-allow: test-support recorder lock
        let mut config = self.config.lock(); // async-gate-allow: test-support recorder lock
        Ok(config
            .liveness
            .pop_front()
            .unwrap_or(ProviderLiveness::Alive))
    }

    async fn stop_resource(
        &self,
        _context: ProcessResourceContext<'_>,
        _spec: &ProcessSpec,
        term_timeout: Duration,
        kill_timeout: Duration,
    ) -> Result<bool, String> {
        self.calls.lock().push("stop"); // async-gate-allow: test-support recorder lock
        self.stops.lock().push(RecordedStop { // async-gate-allow: test-support recorder lock
            kind: "Process",
            term_timeout,
            kill_timeout,
        });
        Ok(true)
    }

    async fn stop_ephemeral_resource(
        &self,
        _context: ProcessResourceContext<'_>,
        _spec: &EphemeralProcessSpec,
        term_timeout: Duration,
        kill_timeout: Duration,
    ) -> Result<bool, String> {
        self.calls.lock().push("stop-ephemeral"); // async-gate-allow: test-support recorder lock
        self.stops.lock().push(RecordedStop { // async-gate-allow: test-support recorder lock
            kind: "EphemeralProcess",
            term_timeout,
            kill_timeout,
        });
        Ok(true)
    }

    async fn stop_stale_resource(
        &self,
        _provider_ref: &ResourceRef,
        _candidate: &AdoptionCandidate,
    ) -> Result<(), String> {
        self.calls.lock().push("stop-stale"); // async-gate-allow: test-support recorder lock
        Ok(())
    }

    async fn finalize_resource(
        &self,
        _context: ProcessResourceContext<'_>,
    ) -> Result<(), String> {
        self.calls.lock().push("finalize"); // async-gate-allow: test-support recorder lock
        *self.finalizes.lock() += 1; // async-gate-allow: test-support recorder lock
        Ok(())
    }

    fn has_active_resource_in_zone(
        &self,
        _zone: &ZoneId,
        _zone_uid: Option<&ResourceUid>,
        _resource_ref: &ResourceRef,
    ) -> bool {
        self.config.lock().active
    }

    async fn resolve_device_worker_launch(
        &self,
        _ctx: &mut d2b_resource_runtime::context::ResourceContext,
        _identity: &crate::identity::ProcessResourceIdentity,
        _spec: &crate::identity::ProcessFamilySpec,
    ) -> Result<Option<crate::worker_launch::DeviceWorkerLaunch>, &'static str> {
        // The double never models a Device-owned worker row: the driver
        // tests exercise the seam's family gate (which returns `None` for
        // every non-device template before the facet is consulted), and the
        // Device-family-specific resolution is owned by the daemon host.
        Ok(None)
    }
}

/// Map the scripted launch digest onto the adapter's opaque launch result.
fn launch_identity(identity: ProcessIdentityDigest) -> ProviderLaunch {
    ProviderLaunch { identity }
}

/// The trusted-bundle fixture the double reports: the same artifact shapes
/// the daemon's own tests load (host fixture + golden v04 manifest), with no
/// zone resource bundles. The family's effects only read the bundle's
/// projected site and storage/intent tables for Device-worker rows, which
/// the driver tests never reach.
fn fixture_bundle() -> BundleResolver {
    let host = serde_json::from_str::<d2b_core::host::HostJson>(include_str!(
        "../../../tests/fixtures/deny-unknown/host-valid.json"
    ))
    .expect("host fixture");
    let manifest = d2b_core::manifest_v04::ManifestV04::from_slice(
        include_str!("../../../tests/golden/manifest_v04/baseline-vms.json").as_bytes(),
    )
    .expect("manifest fixture");
    BundleResolver::from_artifacts_with_zone_resource_bundles(
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
            bundle_hash: Some("sha256:bundle".to_owned()),
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