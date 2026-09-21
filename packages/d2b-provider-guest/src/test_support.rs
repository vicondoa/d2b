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

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use d2b_contracts_resource::v3::identity::ReconnectGeneration;
use d2b_contracts_resource::v3::{
    ControllerGeneration, ResourceGeneration, ResourceRef, ResourceUid, ZoneId,
};
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
    calls: parking_lot::Mutex<Vec<String>>,
    /// Optional shared order log (the recording manager's), so tests can
    /// compare the provider stage against the child mutations.
    shared: Option<Arc<parking_lot::Mutex<Vec<String>>>>,
    observations: parking_lot::Mutex<Vec<EffectObservation>>,
    phase: parking_lot::Mutex<GuestEffectPhase>,
    projection: parking_lot::Mutex<Option<serde_json::Value>>,
    finalize: parking_lot::Mutex<GuestFinalizeStage>,
}

impl ScriptedEffects {
    /// Construct a fresh double with the default Ready phase, no projection,
    /// and a Complete finalize stage.
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            calls: parking_lot::Mutex::new(Vec::new()),
            shared: None,
            observations: parking_lot::Mutex::new(Vec::new()),
            phase: parking_lot::Mutex::new(GuestEffectPhase::Ready),
            projection: parking_lot::Mutex::new(None),
            finalize: parking_lot::Mutex::new(GuestFinalizeStage::Complete),
        })
    }

    /// Construct a double that appends every recorded call to `log` as well
    /// as its own call log, so the provider stage can be compared against
    /// the child mutations of the recording manager that owns the log.
    pub fn with_shared_log(log: Arc<parking_lot::Mutex<Vec<String>>>) -> Arc<Self> {
        let mut effects = Arc::into_inner(Self::new()).expect("fresh effects");
        effects.shared = Some(log);
        Arc::new(effects)
    }

    fn record(&self, entry: String) {
        if let Some(shared) = &self.shared {
            shared.lock().push(entry.clone());
        }
        self.calls.lock().push(entry);
    }

    /// Script the phase the next `reconcile` reports.
    pub fn set_phase(&self, phase: GuestEffectPhase) {
        *self.phase.lock() = phase;
    }

    /// Script the `status.resource` projection the next `reconcile` reports.
    pub fn set_projection(&self, projection: Option<serde_json::Value>) {
        *self.projection.lock() = projection;
    }

    /// Script the finalize stage the next `finalize` reports.
    pub fn set_finalize(&self, stage: GuestFinalizeStage) {
        *self.finalize.lock() = stage;
    }

    /// The observed call labels in arrival order.
    pub fn call_order(&self) -> Vec<String> {
        self.calls.lock().clone()
    }

    /// The recorded `reconcile` observations in arrival order.
    pub fn observations(&self) -> Vec<EffectObservation> {
        self.observations.lock().clone()
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
        self.observations.lock().push(EffectObservation {
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
            phase: *self.phase.lock(),
            resource_projection: self.projection.lock().clone(),
        })
    }

    async fn finalize(
        &self,
        kind: GuestKind,
        _request: &GuestEffectRequest<'_>,
    ) -> Result<GuestFinalizeStage, GuestEffectError> {
        self.record(format!("finalize:{}", kind.effect_id()));
        Ok(*self.finalize.lock())
    }
}

/// Scripted [`GuestManagerView`] + [`CloudHypervisorGuestRuntime`] facets
/// double: answers the manager rows, committed identities, session
/// generation, and Cloud Hypervisor outcomes the production effects service
/// reads, and records every read order-preservingly.
pub struct ScriptedFacets {
    zone: ZoneId,
    controller_generation: ControllerGeneration,
    rows: parking_lot::Mutex<HashMap<ResourceKey, ResourceView>>,
    committed: parking_lot::Mutex<BTreeMap<ResourceRef, (ResourceUid, ResourceGeneration)>>,
    session_generation: parking_lot::Mutex<Option<ReconnectGeneration>>,
    cloud_hypervisor_outcome: parking_lot::Mutex<GuestCloudHypervisorOutcome>,
    fail_reads: parking_lot::Mutex<bool>,
    calls: parking_lot::Mutex<Vec<String>>,
}

impl ScriptedFacets {
    /// Construct a fresh double for the `work` zone: no rows, no committed
    /// identities, no enrolled session, and a Ready Cloud Hypervisor
    /// outcome.
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            zone: ZoneId::parse("work").expect("zone"),
            controller_generation: ControllerGeneration::new(3).expect("generation"),
            rows: parking_lot::Mutex::new(HashMap::new()),
            committed: parking_lot::Mutex::new(BTreeMap::new()),
            session_generation: parking_lot::Mutex::new(None),
            cloud_hypervisor_outcome: parking_lot::Mutex::new(GuestCloudHypervisorOutcome::Ready),
            fail_reads: parking_lot::Mutex::new(false),
            calls: parking_lot::Mutex::new(Vec::new()),
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
    pub fn add_row(&self, row: ResourceView) {
        self.rows.lock().insert(row.key.clone(), row);
    }

    /// Seed one committed Provider identity (KTD7).
    pub fn add_committed_provider(
        &self,
        provider_ref: ResourceRef,
        uid: ResourceUid,
        generation: ResourceGeneration,
    ) {
        self.committed
            .lock()
            .insert(provider_ref, (uid, generation));
    }

    /// Enroll (or clear) the live controller-session generation.
    pub fn set_session_generation(&self, generation: Option<ReconnectGeneration>) {
        *self.session_generation.lock() = generation;
    }

    /// Script the Cloud Hypervisor reconcile outcome.
    pub fn set_cloud_hypervisor_outcome(&self, outcome: GuestCloudHypervisorOutcome) {
        *self.cloud_hypervisor_outcome.lock() = outcome;
    }

    /// Script the manager view as unanswerable: every read refuses, the
    /// same fail-closed surface the effects treat as `Unavailable`.
    pub fn set_fail_reads(&self, fail: bool) {
        *self.fail_reads.lock() = fail;
    }

    /// The observed read labels in arrival order.
    pub fn call_order(&self) -> Vec<String> {
        self.calls.lock().clone()
    }
}

#[async_trait::async_trait]
impl GuestManagerView for ScriptedFacets {
    async fn row_view(&self, key: &ResourceKey) -> Result<Option<ResourceView>, ()> {
        self.calls.lock().push(format!("row:{key}"));
        if *self.fail_reads.lock() {
            return Err(());
        }
        Ok(self.rows.lock().get(key).cloned())
    }

    fn committed_provider_identity(
        &self,
        provider_ref: &ResourceRef,
    ) -> Result<Option<(ResourceUid, ResourceGeneration)>, ()> {
        self.calls
            .lock()
            .push(format!("committed:{}", provider_ref.to_canonical_string()));
        if *self.fail_reads.lock() {
            return Err(());
        }
        Ok(self.committed.lock().get(provider_ref).cloned())
    }

    fn controller_session_generation(&self) -> Result<Option<ReconnectGeneration>, ()> {
        self.calls.lock().push("session-generation".to_owned());
        if *self.fail_reads.lock() {
            return Err(());
        }
        Ok(*self.session_generation.lock())
    }
}

#[async_trait::async_trait]
impl CloudHypervisorGuestRuntime for ScriptedFacets {
    async fn ensure_target_session(&self, guest_ref: &ResourceRef) -> Result<(), String> {
        self.calls
            .lock()
            .push(format!("ensure-session:{}", guest_ref.to_canonical_string()));
        Ok(())
    }

    async fn reconcile_guest(
        &self,
        guest_ref: &ResourceRef,
        _status_sink: Option<crate::driver::GuestStatusSink>,
    ) -> Result<GuestCloudHypervisorOutcome, String> {
        self.calls
            .lock()
            .push(format!("reconcile-ch:{}", guest_ref.to_canonical_string()));
        Ok(*self.cloud_hypervisor_outcome.lock())
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