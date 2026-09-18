//! Test-support recording double for the [`GuestDriverEffects`] port.
//!
//! The scripted effect fake for the Guest family: records every effect call
//! order-preservingly, keeps an optional shared order log (the recording
//! manager's) so tests can compare the provider stage against the child
//! mutations, and scripts the reported phase, resource projection, and
//! finalize stage. Gated behind the `test-support` Cargo feature (available
//! automatically under `cargo test`), so production consumers never pull it
//! in. The plane tests in `d2bd` reach it through the same public surface.

use std::sync::Arc;

use crate::driver::{
    GuestDriverEffects, GuestEffectError, GuestEffectOutcome, GuestEffectPhase,
    GuestEffectRequest, GuestFinalizeStage, GuestKind,
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