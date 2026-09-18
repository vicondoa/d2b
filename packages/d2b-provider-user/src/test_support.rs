//! Test-support recording double for the [`UserDriverEffects`] port.
//!
//! The scripted effect fake for the User family: records every discovery call
//! order-preservingly and can script the reported phase or a probe refusal.

//! Gated behind the `test-support` Cargo feature (available automatically
//! under `cargo test`), so production consumers never pull it in. The plane
//! tests in `d2bd` reach it through the same public surface.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use d2b_contracts_resource::v3::{ResourcePhase, ResourceRef};
use d2b_contracts_resource::v3::user::UserSpec;
use d2b_provider_system_core::{UserDiscoveryCondition, UserStatusReport};

use crate::UserDriverEffects;

/// Scripted discovery port: records every call order-preservingly and can
/// fail discovery.
pub struct RecordingEffects {
    calls: parking_lot::Mutex<Vec<String>>,
    phase: parking_lot::Mutex<ResourcePhase>,
    /// Script whether the next discovery refuses.

    pub fail: AtomicBool,
}

impl RecordingEffects {
    /// Construct a fresh double with the default Ready phase and no recorded
    /// calls.
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            calls: parking_lot::Mutex::new(Vec::new()),
            phase: parking_lot::Mutex::new(ResourcePhase::Ready),
            fail: AtomicBool::new(false),
        })
    }

    /// The observed call labels in arrival order.

    pub fn call_order(&self) -> Vec<String> {
        self.calls.lock().clone()
    }

    /// Script the phase the next discovery reports.

    pub fn set_phase(&self, phase: ResourcePhase) {
        *self.phase.lock() = phase;
    }
}

#[async_trait::async_trait]
impl UserDriverEffects for RecordingEffects {
    async fn observe_user(
        &self,
        user_ref: &ResourceRef,
        _spec: &UserSpec,
    ) -> Result<UserStatusReport, String> {
        self.calls.lock().push("observe-user".to_owned());
        if self.fail.load(Ordering::SeqCst) {
            return Err("the scripted discovery refused".to_owned());
        }
        Ok(UserStatusReport {
            user_ref: user_ref.clone(),
            provider: "system-core",
            phase: *self.phase.lock(),
            discovery: UserDiscoveryCondition::Discovered,
            identity: None,
        })
    }
}