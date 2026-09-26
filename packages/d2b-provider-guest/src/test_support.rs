//! Test-support recording doubles for the Guest family's effect surfaces.
//!
//! Two doubles share this module:
//!
//! - [`ScriptedEffects`] implements the typed [`GuestDriverEffects`] port
//!   directly: records every effect call order-preservingly, keeps an
//!   optional shared order log (the recording manager's) so tests can
//!   compare the provider stage against the child mutations, and scripts
//!   the reported phase, resource projection, and finalize stage. The
//!   driver tests script this surface; the production composition never
//!   takes it.
//! - [`ScriptedFacets`] implements the declared facet traits
//!   ([`GuestManagerView`], [`CloudHypervisorGuestRuntime`]) the production
//!   [`crate::effects_service::GuestEffectsService`] is built from, so the
//!   plane tests build the family's facet set from it exactly as the
//!   production composition root builds it from the daemon's runtime.
//!
//! Both are gated behind the `test-support` Cargo feature (available
//! automatically under `cargo test`), so production consumers never pull
//! them in. The plane tests in `d2bd` reach them through the same public
//! surface.
//!
//! The doubles' ordered call recorders are the toolkit's `SharedLog`
//! (`d2b_provider_toolkit::testing`), the canonical recorder shape every
//! family crate's test-support module shares.
//!
//! Every other recorded field is behind a lock: the async lock
//! (`tokio::sync::Mutex`, awaited) where only async methods touch it, and
//! the blocking lock (`std::sync::Mutex`) with its recorded per-site
//! exception where the facet trait's synchronous accessors read it.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use d2b_contracts_resource::v3::identity::ReconnectGeneration;
use d2b_contracts_resource::v3::{
    ControllerGeneration, ResourceGeneration, ResourceRef, ResourceUid, ZoneId,
};
use d2b_provider_toolkit::testing::SharedLog;
use d2b_resource_runtime::identity::ResourceKey;
use d2b_resource_runtime::manager::ResourceView;
use d2b_resource_runtime::ResourceStatus;

use crate::driver::{
    GuestDriverEffects, GuestEffectError, GuestEffectOutcome, GuestEffectPhase,
    GuestEffectRequest, GuestFinalizeStage, GuestKind,
};
use crate::facets::{
    CloudHypervisorGuestRuntime, GuestCloudHypervisorOutcome, GuestEffectFacets, GuestManagerView,
};

/// One `reconcile` call as the scripted effect observed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectObservation {
    /// The Guest kind the call drove.
    pub kind: GuestKind,
    /// The Provider row's spec the driver resolved, when the manager held it.
    pub provider_spec: Option<serde_json::Value>,
    /// The driver's last published `status.resource` projection, when one
    /// was published.
    pub status: Option<serde_json::Value>,
    /// The owned child rows the call read, with their live phase.
    pub children: Vec<(String, String, bool)>,
}

/// Scripted [`GuestDriverEffects`] double: records every call and answers
/// with the configured outcome.
pub struct ScriptedEffects {
    calls: SharedLog,
    /// Optional shared order log (the recording manager's), so tests can
    /// compare the provider stage against the child mutations.
    shared: Option<SharedLog>,
    observations: tokio::sync::Mutex<Vec<EffectObservation>>,
    phase: tokio::sync::Mutex<GuestEffectPhase>,
    projection: tokio::sync::Mutex<Option<serde_json::Value>>,
    finalize: tokio::sync::Mutex<GuestFinalizeStage>,
}

impl ScriptedEffects {
    /// Construct a fresh double with the default Ready phase, no projection,
    /// and a Complete finalize stage.
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            calls: SharedLog::new(),
            shared: None,
            observations: tokio::sync::Mutex::new(Vec::new()),
            phase: tokio::sync::Mutex::new(GuestEffectPhase::Ready),
            projection: tokio::sync::Mutex::new(None),
            finalize: tokio::sync::Mutex::new(GuestFinalizeStage::Complete),
        })
    }

    /// Construct a double that appends every recorded call to `log` as well
    /// as its own call log, so the provider stage can be compared against
    /// the child mutations of the recording manager that owns the log.
    pub fn with_shared_log(log: SharedLog) -> Arc<Self> {
        let mut effects = Arc::into_inner(Self::new()).expect("fresh effects");
        effects.shared = Some(log);
        Arc::new(effects)
    }

    fn record(&self, entry: String) {
        if let Some(shared) = &self.shared {
            shared.record(entry.clone());
        }
        self.calls.record(entry);
    }

    /// Script the phase the next `reconcile` reports.
    pub async fn set_phase(&self, phase: GuestEffectPhase) {
        *self.phase.lock().await = phase;
    }

    /// Script the `status.resource` projection the next `reconcile` reports.
    pub async fn set_projection(&self, projection: Option<serde_json::Value>) {
        *self.projection.lock().await = projection;
    }

    /// Script the finalize stage the next `finalize` reports.
    pub async fn set_finalize(&self, stage: GuestFinalizeStage) {
        *self.finalize.lock().await = stage;
    }

    /// The observed call labels in arrival order.
    pub fn call_order(&self) -> Vec<String> {
        self.calls.entries()
    }

    /// The recorded `reconcile` observations in arrival order.
    pub async fn observations(&self) -> Vec<EffectObservation> {
        self.observations.lock().await.clone()
    }
}

#[async_trait::async_trait]
impl GuestDriverEffects for ScriptedEffects {
    async fn reconcile(
        &self,
        kind: GuestKind,
        request: &GuestEffectRequest<'_>,
    ) -> Result<GuestEffectOutcome, GuestEffectError> {
        self.record(format!("reconcile:{}", kind.effect_id()));
        let children = request.children.owned().await?;
        self.observations.lock().await.push(EffectObservation {
            kind,
            provider_spec: request.provider_spec.clone(),
            status: request.status.clone(),
            children: children
                .iter()
                .map(|child| {
                    (
                        child.key.type_name.clone(),
                        child.key.name.clone(),
                        child.ready(),
                    )
                })
                .collect(),
        });
        Ok(GuestEffectOutcome {
            phase: *self.phase.lock().await,
            resource_projection: self.projection.lock().await.clone(),
        })
    }

    async fn finalize(
        &self,
        kind: GuestKind,
        _request: &GuestEffectRequest<'_>,
    ) -> Result<GuestFinalizeStage, GuestEffectError> {
        self.record(format!("finalize:{}", kind.effect_id()));
        Ok(*self.finalize.lock().await)
    }
}

/// Scripted [`GuestManagerView`] + [`CloudHypervisorGuestRuntime`] facets
/// double: answers the manager rows, committed identities, session
/// generation, and Cloud Hypervisor outcomes the production effects service
/// reads, and records every read order-preservingly.
pub struct ScriptedFacets {
    zone: ZoneId,
    controller_generation: ControllerGeneration,
    rows: tokio::sync::Mutex<HashMap<ResourceKey, ResourceView>>,
    committed: std::sync::Mutex<BTreeMap<ResourceRef, (ResourceUid, ResourceGeneration)>>,
    session_generation: std::sync::Mutex<Option<ReconnectGeneration>>,
    cloud_hypervisor_outcome: tokio::sync::Mutex<GuestCloudHypervisorOutcome>,
    fail_reads: std::sync::Mutex<bool>,
    calls: SharedLog,
}

impl ScriptedFacets {
    /// Construct a fresh double for the `work` zone: no rows, no committed
    /// identities, no enrolled session, and a Ready Cloud Hypervisor
    /// outcome.
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            zone: ZoneId::parse("work").expect("zone"),
            controller_generation: ControllerGeneration::new(3).expect("generation"),
            rows: tokio::sync::Mutex::new(HashMap::new()),
            committed: std::sync::Mutex::new(BTreeMap::new()),
            session_generation: std::sync::Mutex::new(None),
            cloud_hypervisor_outcome: tokio::sync::Mutex::new(GuestCloudHypervisorOutcome::Ready),
            fail_reads: std::sync::Mutex::new(false),
            calls: SharedLog::new(),
        })
    }

    /// The facet set the effects service is built from, over this double.
    pub fn facet_set(self: &Arc<Self>) -> GuestEffectFacets {
        GuestEffectFacets {
            zone: self.zone.clone(),
            controller_generation: self.controller_generation,
            manager: Arc::clone(self) as Arc<dyn GuestManagerView>,
            cloud_hypervisor: Arc::clone(self) as Arc<dyn CloudHypervisorGuestRuntime>,
        }
    }

    /// Seed one manager row the effects read.
    pub async fn add_row(&self, row: ResourceView) {
        self.rows.lock().await.insert(row.key.clone(), row);
    }

    /// Seed one committed Provider identity (KTD7).
    pub fn add_committed_provider(
        &self,
        provider_ref: ResourceRef,
        uid: ResourceUid,
        generation: ResourceGeneration,
    ) {
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        {
            self.committed
                .lock()
                .unwrap()
                .insert(provider_ref, (uid, generation));
        }
    }

    /// Enroll (or clear) the live controller-session generation.
    pub fn set_session_generation(&self, generation: Option<ReconnectGeneration>) {
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        {
            *self.session_generation.lock().unwrap() = generation;
        }
    }

    /// Script the Cloud Hypervisor reconcile outcome.
    pub async fn set_cloud_hypervisor_outcome(&self, outcome: GuestCloudHypervisorOutcome) {
        *self.cloud_hypervisor_outcome.lock().await = outcome;
    }

    /// Script the manager view as unanswerable: every read refuses, the
    /// same fail-closed surface the effects treat as `Unavailable`.
    pub fn set_fail_reads(&self, fail: bool) {
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        {
            *self.fail_reads.lock().unwrap() = fail;
        }
    }

    /// The observed read labels in arrival order.
    pub fn call_order(&self) -> Vec<String> {
        self.calls.entries()
    }
}

#[async_trait::async_trait]
impl GuestManagerView for ScriptedFacets {
    async fn row_view(&self, key: &ResourceKey) -> Result<Option<ResourceView>, ()> {
        self.calls.record(format!("row:{key}"));
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        if *self.fail_reads.lock().unwrap() { // async-gate-allow: test-support recorder lock
            return Err(());
        }
        Ok(self.rows.lock().await.get(key).cloned())
    }

    fn committed_provider_identity(
        &self,
        provider_ref: &ResourceRef,
    ) -> Result<Option<(ResourceUid, ResourceGeneration)>, ()> {
        self.calls
            .record(format!("committed:{}", provider_ref.to_canonical_string()));
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        if *self.fail_reads.lock().unwrap() {
            return Err(());
        }
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        Ok(self.committed.lock().unwrap().get(provider_ref).cloned())
    }

    fn controller_session_generation(&self) -> Result<Option<ReconnectGeneration>, ()> {
        self.calls.record("session-generation".to_owned());
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        if *self.fail_reads.lock().unwrap() {
            return Err(());
        }
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        Ok(*self.session_generation.lock().unwrap())
    }
}

#[async_trait::async_trait]
impl CloudHypervisorGuestRuntime for ScriptedFacets {
    async fn ensure_target_session(&self, guest_ref: &ResourceRef) -> Result<(), String> {
        self.calls
            .record(format!("ensure-session:{}", guest_ref.to_canonical_string()));
        Ok(())
    }

    async fn reconcile_guest(
        &self,
        guest_ref: &ResourceRef,
        _status_sink: Option<crate::driver::GuestStatusSink>,
    ) -> Result<GuestCloudHypervisorOutcome, String> {
        self.calls
            .record(format!("reconcile-ch:{}", guest_ref.to_canonical_string()));
        Ok(*self.cloud_hypervisor_outcome.lock().await)
    }
}

/// A manager row fixture for one resource: the canonical view the effects
/// read, with the given live status.
pub fn row_fixture(
    zone: &str,
    type_name: &str,
    name: &str,
    spec: serde_json::Value,
    status: ResourceStatus,
) -> ResourceView {
    row_fixture_with_metadata(zone, type_name, name, spec, status, serde_json::json!({}))
}

/// A manager row fixture with explicit metadata (the gateway-custody
/// validation reads the gateway Guest's `metadata.zone`).
pub fn row_fixture_with_metadata(
    zone: &str,
    type_name: &str,
    name: &str,
    spec: serde_json::Value,
    status: ResourceStatus,
    metadata: serde_json::Value,
) -> ResourceView {
    ResourceView {
        key: ResourceKey::new(zone, type_name, name),
        uid: [0x42; 16],
        generation: 1,
        deleting: false,
        provenance: d2b_resource_runtime::identity::ResourceProvenance::Resource,
        spec: serde_json::to_vec(&spec).expect("spec bytes"),
        metadata: serde_json::to_vec(&metadata).expect("metadata bytes"),
        owner_key: None,
        status: Some(status),
        status_generation: Some(1),
        status_projection: None,
    }
}