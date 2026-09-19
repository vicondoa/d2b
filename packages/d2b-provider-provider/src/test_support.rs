//! Test-support recording double for the [`ProviderDriverEffects`] port.
//!
//! The scripted effect fake for the Provider family: records every session
//! evidence read and replays a scripted evidence payload with the driver's
//! ticket inputs stamped in. Gated behind the `test-support` Cargo feature
//! (available automatically under `cargo test`), so production consumers
//! never pull it in. The plane tests in `d2bd` reach it through the same
//! public surface.

use std::sync::Arc;
use std::sync::Mutex;

use crate::ProviderDriverEffects;

/// Scripted session-evidence port.
pub struct RecordingEffects {
    calls: Mutex<Vec<&'static str>>,
    evidence: Mutex<Option<serde_json::Value>>,
}

impl RecordingEffects {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            calls: Mutex::new(Vec::new()),
            evidence: Mutex::new(None),
        })
    }

    /// The observed call labels in arrival order.
    pub fn call_order(&self) -> Vec<&'static str> {
        self.calls.try_lock().expect("uncontended test mutex").clone()
    }

    /// Script the evidence payload the next read stamps the ticket inputs into.
    pub fn set_evidence(&self, evidence: serde_json::Value) {
        *self.evidence.try_lock().expect("uncontended test mutex") = Some(evidence);
    }
}

impl ProviderDriverEffects for RecordingEffects {
    fn controller_session_evidence(
        &self,
        process_ref: &d2b_contracts_resource::v3::ResourceRef,
        process_uid: &d2b_contracts_resource::v3::ResourceUid,
        generation: d2b_contracts_resource::v3::ResourceGeneration,
    ) -> Option<serde_json::Value> {
        self.calls.try_lock().expect("uncontended test mutex").push("controller-session");
        let mut evidence = self.evidence.try_lock().expect("uncontended test mutex").clone()?;
        evidence["processRef"] = serde_json::Value::String(process_ref.to_canonical_string());
        evidence["processUid"] = serde_json::Value::String(process_uid.as_str().to_owned());
        evidence["processGeneration"] = serde_json::Value::from(generation.get());
        Some(evidence)
    }
}