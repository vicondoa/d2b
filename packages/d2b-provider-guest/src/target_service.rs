//! Guest-side target-control service (U13 guest half; R18, R19, R21, F5).
//!
//! The protocol vocabulary, the frame codec, the live-session fence, and the
//! target-local bookkeeping all live in `d2b_resource_runtime::guest_target`
//! (U13's published contract); this module adds the two pieces only the Guest
//! daemon can supply:
//!
//! - **The served transport.** One ttrpc method - the published
//!   [`TARGET_CONTROL_SERVICE`] `TARGET_CONTROL_METHOD` - registered on the
//!   same authenticated ComponentSession that already carries the Guest-local
//!   Resource API. Its payload is one [`TargetControlFrame`]; an unreadable
//!   frame or a foreign protocol token is refused before any dispatch.
//! - **The consumption path.** Every realize the runtime accepted is applied
//!   through the target-local effect code registered for its resource type
//!   ([`GuestTargetEffect`]), and the realization is reported `ready` only
//!   after that effect is serving. A target-local effect is the Guest half of
//!   a converted type's driver: it applies exactly the host-resolved spec
//!   bytes, never a shape it invents.
//!
//! Fail-closed rules, all enforced before any state exists:
//!
//! - a request whose source belongs to another Zone's authority is refused
//!   (a Guest only realizes for the Zone that owns its session);
//! - a request whose resource type has no target-local effect registered here
//!   is refused - the Guest never records a realization it cannot apply;
//! - a realize whose `specDigest` does not match `target_local_spec_digest` of
//!   the exact bytes carried is refused before the effect sees the spec.
//!
//! Every refusal answers [`TargetControlResponse::SessionUnavailable`], the
//! only closed negative in the protocol, and performs no effect: the host
//! treats the target as unavailable and retries or re-realizes.
//!
//! The live session generation is bound by [`GuestTargetService::bind_session`]
//! from the accepted session's authenticated route, so the fence is the
//! daemon's actual live generation rather than a value a request carries.
//!
//! Consumer situation (2026-09-11): no converted type registers target-local
//! effect code in Guest mode yet. The U12 conversion moved Process and
//! EphemeralProcess onto the manager plane and retired the Guest-local typed
//! runner, so no type has a Guest-side realization path here; every converted
//! type's realize frames are refused instead of recording phantom state.
//! Registering the effect code is what admits a type.

use std::{collections::BTreeMap, collections::HashMap, fmt, sync::Arc};

use async_trait::async_trait;
use d2b_contracts_resource::v3::ZoneId;
use d2b_resource_runtime::guest_target::{
    target_local_spec_digest, GuestAdoption, GuestRealizeRequest, GuestTargetError,
    GuestTargetRuntime, TargetControlFrame, TargetControlRequest, TargetControlResponse,
    TARGET_CONTROL_METHOD, TARGET_CONTROL_SERVICE,
};
use d2b_resource_runtime::identity::{ResourceKey, ResourceTypeName};

/// Target-local effect code for one converted resource type: the Guest half
/// of that type's driver.
///
/// One implementation serves one resource type and owns the target-local
/// effect for every source of that type on this Guest. Realize and delete are
/// idempotent per source: a repeated realize converges on the same effect
/// (never a second one), and a repeated delete of an absent effect succeeds.
#[async_trait]
pub trait GuestTargetEffect: Send + Sync + 'static {
    /// Apply (or re-apply) the host-resolved target-local spec. Returning
    /// `Ok` means the effect is serving; the realization is then reported
    /// `ready`. Any error leaves the realization `realizing` - the owning
    /// Host driver's next realize retries it.
    async fn realize(
        &self,
        request: &GuestRealizeRequest,
    ) -> Result<(), GuestTargetEffectError>;

    /// Remove the target-local effect for one source.
    async fn delete(&self, source: &ResourceKey) -> Result<(), GuestTargetEffectError>;

    /// Re-discover the target-local effect after a reconnect (F5) and report
    /// whether it is present and serving.
    async fn adopt(&self, source: &ResourceKey) -> Result<bool, GuestTargetEffectError>;
}

/// Closed failure of one type's target-local effect.
///
/// The effect could not be applied (or removed) on the target right now: the
/// realization stays `realizing` and the owning Host driver's next realize
/// retries it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestTargetEffectError {
    /// The target-local machinery cannot run right now.
    Unavailable,
}

impl GuestTargetEffectError {
    const fn code(self) -> &'static str {
        match self {
            Self::Unavailable => "guest-target-effect-unavailable",
        }
    }
}

/// Why a target-control request was refused before any state changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestTargetRefusal {
    /// The named resource belongs to another Zone's authority.
    ForeignZone,
    /// This Guest has no target-local effect code for the resource type.
    UnknownType,
}

impl GuestTargetRefusal {
    const fn code(self) -> &'static str {
        match self {
            Self::ForeignZone => "guest-target-foreign-zone",
            Self::UnknownType => "guest-target-unknown-type",
        }
    }
}

/// The target-local effect code this Guest serves, keyed by resource type.
///
/// The composition owns the map: a type appears here exactly when this Guest
/// has the effect code to apply its target-local spec (see the module consumer
/// note).
pub type GuestTargetEffects = BTreeMap<ResourceTypeName, Arc<dyn GuestTargetEffect>>;

/// The production Guest-side effect map (U13).
///
/// **This map is intentionally empty, and that is the gap, not an oversight.**
/// Registering a type here is what makes the service record and serve its
/// realize frames; no converted type has Guest-side effect code in this tree,
/// so a placeholder entry would have the Guest record a realization it cannot
/// apply - strictly worse than the honest refusal the empty map produces.
///
/// What exists instead, and what this map deliberately does not replace:
///
/// - the Cloud Hypervisor controller's guest-local lifecycle (the seed batch
///   and drain the U6 path drives over the Guest Resource API);
/// - a Host-zone resource whose spec targets a Guest is assigned to the Zone
///   target directory and reaches the Guest through the authenticated
///   ComponentSession ([`GuestTargetService`]); until its type is registered
///   here, that realize frame is refused with no state.
///
/// (The old Guest-local typed Process/EphemeralProcess runner was retired when
/// those types converted in U12; no path serves those rows on the Guest side
/// today.)
///
/// Guest-side realization for converted types (the Guest half of those
/// drivers, applying the host-resolved spec inside the Guest) is follow-on
/// work to U13; it is not on the Cloud Hypervisor acceptance path, which this
/// map does not serve either way.
pub fn production_guest_target_effects() -> GuestTargetEffects {
    GuestTargetEffects::new()
}

/// The Guest-mode target-control service.
///
/// One service owns the Guest's [`GuestTargetRuntime`], the authority Zone the
/// Guest realizes for, and the registered target-local effect code. The
/// service survives reconnects: realizations stay on the target runtime and a
/// reconnected session re-adopts them (F5).
pub struct GuestTargetService {
    runtime: Arc<GuestTargetRuntime>,
    zone: ZoneId,
    effects: GuestTargetEffects,
}

impl fmt::Debug for GuestTargetService {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GuestTargetService")
            .field("zone", &self.zone)
            .field(
                "effect_types",
                &self.effects.keys().map(ResourceTypeName::as_str).collect::<Vec<_>>(),
            )
            .field("session_generation", &self.runtime.session_generation())
            .finish()
    }
}

impl GuestTargetService {
    /// Own the target runtime for one authority Zone and the effect code this
    /// Guest serves. Only registered types are served: an unregistered type is
    /// refused rather than recorded (see the module consumer note).
    pub fn new(
        runtime: Arc<GuestTargetRuntime>,
        zone: ZoneId,
        effects: GuestTargetEffects,
    ) -> Self {
        Self { runtime, zone, effects }
    }

    /// Bind the authenticated session generation the fence validates against.
    ///
    /// Called once per accepted ComponentSession from the session's own
    /// authenticated route: this value is the live generation, never a value
    /// a request carries.
    pub fn bind_session(&self, session_generation: u64) -> Result<(), GuestTargetError> {
        self.runtime.bind_session(session_generation)
    }

    /// Serve one target-control request: admit, fence, consume, answer.
    pub async fn handle(&self, request: TargetControlRequest) -> TargetControlResponse {
        if let Err(refusal) = self.admit(request.source()) {
            tracing::warn!(
                code = refusal.code(),
                source = %request.source(),
                "Guest target-control request refused before any effect",
            );
            return TargetControlResponse::SessionUnavailable;
        }
        match request {
            TargetControlRequest::Realize(realize) => self.realize(realize).await,
            TargetControlRequest::Delete { .. } => self.delete(request).await,
            TargetControlRequest::Adopt { .. } => self.adopt(request).await,
            TargetControlRequest::Observe { .. } => self.runtime.handle(request),
        }
    }

    /// Realize one target-local realization: the runtime records it under the
    /// owning Host-zone source, then the type's effect code applies the exact
    /// spec bytes and the realization is reported `ready` once serving.
    async fn realize(&self, realize: GuestRealizeRequest) -> TargetControlResponse {
        // The commitment is recomputed from the carried bytes: a spec that was
        // substituted or truncated on the way is refused before the effect
        // sees it, and nothing is recorded.
        if target_local_spec_digest(realize.spec()) != realize.spec_digest() {
            tracing::warn!(
                code = "guest-target-spec-commitment-mismatch",
                source = %realize.source(),
                "Guest target-control realize refused: the spec does not match its commitment",
            );
            return TargetControlResponse::SessionUnavailable;
        }
        let response = self.runtime.handle(TargetControlRequest::Realize(realize.clone()));
        let TargetControlResponse::Realized { realization } = &response else {
            return response;
        };
        let source = realization.source().clone();
        let Some(effect) = self.effect(&source) else {
            // Admission and dispatch read the same registry, so this is
            // unreachable while the service is composed; if it ever happens,
            // the effect is not applied and the realization honestly stays
            // realizing instead of being reported ready.
            tracing::warn!(
                code = GuestTargetEffectError::Unavailable.code(),
                source = %source,
                "Guest target-local effect code is missing after admission",
            );
            return response;
        };
        match effect.realize(&realize).await {
            Ok(()) => {
                self.runtime.mark_ready(&source);
            }
            Err(error) => {
                tracing::warn!(
                    code = error.code(),
                    source = %source,
                    "Guest target-local effect did not converge; the realization stays realizing",
                );
            }
        }
        match self.runtime.instance(&source) {
            Some(instance) => TargetControlResponse::Realized { realization: instance },
            None => response,
        }
    }

    /// Delete one target-local realization: the runtime removes exactly the
    /// named source's instance, then its effect code removes the target-local
    /// effect. An absent source is still answered `deleted`, with no effect.
    async fn delete(&self, request: TargetControlRequest) -> TargetControlResponse {
        let source = request.source().clone();
        let present = self.runtime.instance(&source).is_some();
        let response = self.runtime.handle(request);
        if present
            && matches!(response, TargetControlResponse::Deleted)
            && let Some(effect) = self.effect(&source)
            && let Err(error) = effect.delete(&source).await
        {
            tracing::warn!(
                code = error.code(),
                source = %source,
                "Guest target-local effect delete did not converge",
            );
        }
        response
    }

    /// Adopt one realization after a reconnect (F5): the runtime re-binds the
    /// instance to the live session generation, and the type's effect code
    /// re-discovers the target-local effect. An effect that is gone - or whose
    /// discovery cannot answer - answers `missing`, so the owning Host actor
    /// realizes the resource again (idempotently) instead of inheriting a
    /// record of something that may not be there.
    async fn adopt(&self, request: TargetControlRequest) -> TargetControlResponse {
        let source = request.source().clone();
        let response = self.runtime.handle(request);
        let TargetControlResponse::Adopted(GuestAdoption::Adopted(instance)) = &response else {
            return response;
        };
        let Some(effect) = self.effect(&source) else {
            return response;
        };
        let recovery = effect.adopt(&source).await;
        match recovery {
            Ok(true) => {
                return match self.runtime.mark_ready(&source) {
                    Some(instance) => {
                        TargetControlResponse::Adopted(GuestAdoption::Adopted(instance))
                    }
                    None => response,
                };
            }
            Ok(false) => {
                tracing::warn!(
                    code = "guest-target-effect-absent",
                    source = %source,
                    "Guest target-local effect is absent at adoption; reporting missing",
                );
            }
            Err(error) => {
                tracing::warn!(
                    code = error.code(),
                    source = %source,
                    "Guest target-local effect discovery did not converge; reporting missing",
                );
            }
        }
        // Forgetting the unresolved record is the fenced delete the protocol
        // already serves, at the same live generation the adoption named, so
        // the host sees `missing` and realizes the resource again instead of
        // holding a realization nobody could confirm.
        self.runtime.handle(TargetControlRequest::Delete {
            assignment: d2b_resource_runtime::guest_target::TargetControlAssignment::new(
                source,
                *instance.source_uid(),
                instance.assignment_generation(),
                instance.session_generation(),
            ),
        });
        TargetControlResponse::Adopted(GuestAdoption::Missing)
    }

    /// Admit one source before any state or effect exists.
    fn admit(&self, source: &ResourceKey) -> Result<(), GuestTargetRefusal> {
        if source.zone != self.zone.as_str() {
            return Err(GuestTargetRefusal::ForeignZone);
        }
        if !self.effects.contains_key(&ResourceTypeName::new(source.type_name.clone())) {
            return Err(GuestTargetRefusal::UnknownType);
        }
        Ok(())
    }

    fn effect(&self, source: &ResourceKey) -> Option<&Arc<dyn GuestTargetEffect>> {
        self.effects.get(&ResourceTypeName::new(source.type_name.clone()))
    }
}

/// The Guest-side ttrpc method serving [`TARGET_CONTROL_SERVICE`].
struct TargetControlMethod {
    service: Arc<GuestTargetService>,
}

#[async_trait]
impl ttrpc::r#async::MethodHandler for TargetControlMethod {
    async fn handler(
        &self,
        _context: ttrpc::r#async::TtrpcContext,
        request: ttrpc::Request,
    ) -> ttrpc::Result<ttrpc::Response> {
        let frame = TargetControlFrame::decode(&request.payload).map_err(refused_frame)?;
        let request = frame.into_request().map_err(refused_frame)?;
        let response = self.service.handle(request).await;
        let mut reply = ttrpc::Response::new();
        reply.set_status(ttrpc::get_status(ttrpc::Code::OK, ""));
        reply.payload = response.encode();
        Ok(reply)
    }
}

fn refused_frame(error: GuestTargetError) -> ttrpc::Error {
    tracing::warn!(
        code = %error,
        "Guest target-control frame refused before dispatch",
    );
    ttrpc::Error::Others(error.to_string())
}

/// Build the Guest-side target-control service map for one accepted session.
///
/// The map merges into the same ttrpc surface that already carries the
/// Guest-local Resource API (U6) and the Guest configuration services.
pub fn target_control_services(
    service: Arc<GuestTargetService>,
) -> HashMap<String, ttrpc::r#async::Service> {
    let methods = HashMap::from([(
        TARGET_CONTROL_METHOD.to_owned(),
        Box::new(TargetControlMethod { service })
            as Box<dyn ttrpc::r#async::MethodHandler + Send + Sync>,
    )]);
    HashMap::from([(
        TARGET_CONTROL_SERVICE.to_owned(),
        ttrpc::r#async::Service { methods, streams: HashMap::new() },
    )])
}
