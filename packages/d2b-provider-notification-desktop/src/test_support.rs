//! Test-support recording double for the [`SourceProcessEffectPort`] port.
//!
//! The scripted effect fake for the desktop notification family: records
//! every applied reconcile plan and completes every effect immediately.
//! Gated behind the `test-support` Cargo feature (available automatically
//! under `cargo test`), so production consumers never pull it in. The plane
//! tests in `d2bd` reach it through the same public surface.

use crate::{
    NotificationLifecyclePlan, SourceProcessEffectPort, SourceProcessEffectReceipt, SourceReconcileResult,
};

/// Scripted source-process effect port: records every applied plan and
/// completes every effect immediately.
pub struct RecordingEffects {
    /// The reconcile plans applied inorder.
    pub plans: Vec<SourceReconcileResult>,
}

impl SourceProcessEffectPort for RecordingEffects {
    fn apply(
        &mut self,
        plan: &SourceReconcileResult,
        _lifecycle: &NotificationLifecyclePlan,
    ) -> Result<SourceProcessEffectReceipt, &'static str> {
        self.plans.push(plan.clone());
        Ok(SourceProcessEffectReceipt::complete(plan))
    }
}