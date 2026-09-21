//! Test-support scripted double for the [`InteractionDriverEffects`] port,
//! and the scripted facet set the plane tests build the family's effects
//! from.
//!
//! The scripted effect fake records every effect call with the caller's
//! shared ordered log and replays a scripted ready/finalize outcome
//! sequence. The scripted facet set supplies the daemon-side facet sources
//! (identity, plane reads, audio mediator) with fixed test answers, so the
//! plane tests construct the family's real effects over scripted sources
//! exactly as the production composition root constructs them over the
//! daemon's. Gated behind the `test-support` Cargo feature (available
//! automatically under `cargo test`), so production consumers never pull it
//! in. The plane tests in `d2bd` reach it through the same public surface.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use d2b_contracts_resource::v3::{ResourceRef, ResourceUid, ZoneId};
use d2b_provider_audio_pipewire::AudioMediator;
use d2b_resource_runtime::identity::ResourceKey;
use d2b_resource_runtime::manager::{ResourceSelector, ResourceView};
use serde_json::json;

use crate::{
    AudioMediatorSource, InteractionDriverEffects, InteractionEffectError, InteractionEffectFacets,
    InteractionEffectIdentity, InteractionEffectOutcome, InteractionEffectPhase,
    InteractionEffectRequest, InteractionIdentitySource, InteractionPlaneRead, InteractionFinalize,
    InteractionKind,
};

/// The fixed test identity the scripted facet set serves.
///
/// The refs mirror the committed identity the daemon's plane tests use
/// elsewhere, so a driver test that drives a session row over the scripted
/// facets can match the admission fence.
pub fn scripted_identity() -> InteractionEffectIdentity {
    InteractionEffectIdentity {
        wayland_session_ref: ResourceRef::parse(
            "display-wayland.d2bus.org.WaylandSession/display-wayland",
        )
        .expect("fixed test WaylandSession reference"),
        wayland_session_uid: ResourceUid::parse("33333333-3333-4333-8333-333333333333")
            .expect("fixed test WaylandSession UID"),
        subject_ref: ResourceRef::parse("Guest/work").expect("fixed test Guest reference"),
        host_execution_ref: ResourceRef::parse("Host/host-system")
            .expect("fixed test Host reference"),
        user_ref: ResourceRef::parse("User/alice").expect("fixed test User reference"),
    }
}

/// The scripted identity source: always answers the fixed test identity.
struct ScriptedIdentitySource;

#[async_trait]
impl InteractionIdentitySource for ScriptedIdentitySource {
    async fn identity(&self) -> Option<InteractionEffectIdentity> {
        Some(scripted_identity())
    }
}

/// The scripted plane read source: no row is ever committed.
struct ScriptedPlaneRead;

#[async_trait]
impl InteractionPlaneRead for ScriptedPlaneRead {
    async fn get(&self, _key: &ResourceKey) -> Result<Option<ResourceView>, ()> {
        Ok(None)
    }

    async fn list(&self, _selector: &ResourceSelector) -> Result<Vec<ResourceView>, ()> {
        Ok(Vec::new())
    }
}

/// The scripted audio mediator source: no target carries an audio
/// capability.
struct ScriptedAudioSource;

impl AudioMediatorSource for ScriptedAudioSource {
    fn build(&self, _vm_name: &str, _projection: bool) -> Option<Box<dyn AudioMediator>> {
        None
    }
}

/// The scripted facet set for one test Zone: the fixed identity, an empty
/// plane, and no audio capability, so the effects built from it answer the
/// same pending outcomes the scripted port double answered before the move.
pub fn scripted_facets(zone: ZoneId) -> InteractionEffectFacets {
    InteractionEffectFacets::new(
        zone,
        Arc::new(ScriptedIdentitySource),
        Arc::new(ScriptedPlaneRead),
        Arc::new(ScriptedAudioSource),
    )
}

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