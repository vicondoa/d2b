//! Test-support scripted double for the [`InteractionDriverEffects`] port.
//!
//! The scripted effect fake for the interaction family: records every effect
//! call with the caller's shared ordered log and replays a scripted
//! ready/finalize outcome sequence. Gated behind the `test-support` Cargo
//! feature (available automatically under `cargo test`), so production
//! consumers never pull it in. The plane tests in `d2bd` reach it through
//! the same public surface.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::json;

use crate::{
    InteractionDriverEffects, InteractionEffectError, InteractionEffectOutcome, InteractionEffectPhase,
    InteractionEffectRequest, InteractionFinalize, InteractionKind,
};

/// One shared ordered log the scripted double records into.
pub type Log = Arc<tokio::sync::Mutex<Vec<String>>>;

/// Scripted typed effects over the caller's ordered log, so the tests assert
/// one sequence across manager calls and Provider effects.
pub struct ScriptedEffects {
    log: Log,
    ready: AtomicBool,
    finalize_pending: AtomicBool,
}

impl ScriptedEffects {
    /// A fresh double recording into its own private, unused log.
    pub fn new() -> Arc<Self> {
        Self::shared(Arc::new(tokio::sync::Mutex::new(Vec::new())))
    }

    /// A double recording into the caller's shared ordered log.
    pub fn shared(log: Log) -> Arc<Self> {
        Arc::new(Self {
            log,
            ready: AtomicBool::new(false),
            finalize_pending: AtomicBool::new(false),
        })
    }

    /// Script the next reconcile to project Ready.
    pub fn make_ready(&self) {
        self.ready.store(true, Ordering::SeqCst);
    }

    /// Script the next finalize to stay pending.
    pub fn hold_finalize(&self) {
        self.finalize_pending.store(true, Ordering::SeqCst);
    }
}

#[async_trait::async_trait]
impl InteractionDriverEffects for ScriptedEffects {
    async fn reconcile(
        &self,
        kind: InteractionKind,
        _request: &InteractionEffectRequest<'_>,
    ) -> Result<InteractionEffectOutcome, InteractionEffectError> {
        self.log
            .lock().await
            .push(format!("effect:{}", kind.effect_id()));
        if self.ready.load(Ordering::SeqCst) {
            Ok(InteractionEffectOutcome::projection(
                InteractionEffectPhase::Ready,
                json!({"phase": "Ready"}),
            ))
        } else {
            Ok(InteractionEffectOutcome::phase(InteractionEffectPhase::Pending))
        }
    }

    async fn finalize(
        &self,
        kind: InteractionKind,
        _request: &InteractionEffectRequest<'_>,
    ) -> Result<InteractionFinalize, InteractionEffectError> {
        self.log
            .lock().await
            .push(format!("finalize:{}", kind.effect_id()));
        if self.finalize_pending.load(Ordering::SeqCst) {
            Ok(InteractionFinalize::Pending)
        } else {
            Ok(InteractionFinalize::Complete)
        }
    }
}