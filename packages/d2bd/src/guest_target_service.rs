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
//! Consumer situation (2026-09-10): no converted type registers target-local
//! effect code in Guest mode yet. Process and EphemeralProcess effects run
//! through the preserved Guest-local Resource API path
//! (`run_guest_process_reconciliation`), which this service deliberately does
//! not replace; every other converted type has no Guest-side consumer, so its
//! realize frames are refused here instead of recording phantom state.
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
pub(crate) trait GuestTargetEffect: Send + Sync + 'static {
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
pub(crate) enum GuestTargetEffectError {
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
pub(crate) enum GuestTargetRefusal {
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
/// note). Today the map is empty - no converted type has Guest-side effect
/// code yet - so every type is refused rather than recorded.
pub(crate) type GuestTargetEffects = BTreeMap<ResourceTypeName, Arc<dyn GuestTargetEffect>>;

/// The Guest-mode target-control service.
///
/// One service owns the Guest's [`GuestTargetRuntime`], the authority Zone the
/// Guest realizes for, and the registered target-local effect code. The
/// service survives reconnects: realizations stay on the target runtime and a
/// reconnected session re-adopts them (F5).
pub(crate) struct GuestTargetService {
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
    pub(crate) fn new(
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
    pub(crate) fn bind_session(&self, session_generation: u64) -> Result<(), GuestTargetError> {
        self.runtime.bind_session(session_generation)
    }

    /// Serve one target-control request: admit, fence, consume, answer.
    pub(crate) async fn handle(&self, request: TargetControlRequest) -> TargetControlResponse {
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
pub(crate) fn target_control_services(
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

#[cfg(test)]
mod tests {
    use std::sync::Mutex as StdMutex;

    use d2b_contracts_resource::v3::{
        ResourceRef, ResourceUid, SchemaFingerprint, ZoneId,
        identity::{ReconnectGeneration, SessionPurpose},
    };
    use d2b_resource_runtime::guest_target::{
        TargetControlAssignment, TargetResourceInstance, TargetInstanceState,
    };
    use d2b_resource_runtime::target::{TargetObservation, TargetRef};
    use d2b_session::{
        HandshakeCredentials, Secret32, SessionEngine, SessionTtrpcClient, x25519_public_key,
    };
    use d2b_session_unix::FramedVsockTransport;
    use d2bd_runtime::guest_mode::{
        BootIdentity, GUEST_COMPONENT_SESSION_PURPOSE, GuestIdentity, GuestRuntime,
    };
    use d2bd_runtime::target_runtime::AdmissionLimits;

    use super::*;

    const AUTHORITY_ZONE: &str = "work";
    const TARGET_TYPE: &str = "Endpoint";

    /// One target-local effect under test: records the exact spec bytes it
    /// was asked to apply and answers the discovery question.
    #[derive(Default)]
    struct RecordingEffect {
        realized: StdMutex<Vec<(ResourceKey, Vec<u8>, String)>>,
        deleted: StdMutex<Vec<ResourceKey>>,
        adopted: StdMutex<Vec<ResourceKey>>,
        present: StdMutex<bool>,
        outcome: StdMutex<Option<GuestTargetEffectError>>,
        discovery: StdMutex<Option<GuestTargetEffectError>>,
    }

    impl RecordingEffect {
        fn new() -> Arc<Self> {
            Arc::new(Self { present: StdMutex::new(true), ..Self::default() })
        }

        fn absent() -> Arc<Self> {
            let effect = Self::new();
            *effect.present.lock().expect("present") = false;
            effect
        }

        fn failing(error: GuestTargetEffectError) -> Arc<Self> {
            let effect = Self::new();
            *effect.outcome.lock().expect("outcome") = Some(error);
            effect
        }

        fn realized(&self) -> Vec<(ResourceKey, Vec<u8>, String)> {
            self.realized.lock().expect("realized").clone()
        }

        fn deleted(&self) -> Vec<ResourceKey> {
            self.deleted.lock().expect("deleted").clone()
        }

        fn adopted(&self) -> Vec<ResourceKey> {
            self.adopted.lock().expect("adopted").clone()
        }
    }

    #[async_trait]
    impl GuestTargetEffect for RecordingEffect {
        async fn realize(
            &self,
            request: &GuestRealizeRequest,
        ) -> Result<(), GuestTargetEffectError> {
            if let Some(error) = *self.outcome.lock().expect("outcome") {
                return Err(error);
            }
            self.realized.lock().expect("realized").push((
                request.source().clone(),
                request.spec().to_vec(),
                request.spec_digest().to_owned(),
            ));
            Ok(())
        }

        async fn delete(&self, source: &ResourceKey) -> Result<(), GuestTargetEffectError> {
            self.deleted.lock().expect("deleted").push(source.clone());
            Ok(())
        }

        async fn adopt(&self, source: &ResourceKey) -> Result<bool, GuestTargetEffectError> {
            self.adopted.lock().expect("adopted").push(source.clone());
            if let Some(error) = *self.discovery.lock().expect("discovery") {
                return Err(error);
            }
            Ok(*self.present.lock().expect("present"))
        }
    }

    /// The service under test with its runtime kept for assertions.
    struct Fixture {
        runtime: Arc<GuestTargetRuntime>,
        effect: Arc<RecordingEffect>,
        service: Arc<GuestTargetService>,
    }

    impl Fixture {
        fn new(effect: Arc<RecordingEffect>, generation: u64) -> Self {
            let runtime = Arc::new(GuestTargetRuntime::new(guest()));
            let effects = GuestTargetEffects::from([(
                ResourceTypeName::new(TARGET_TYPE),
                Arc::clone(&effect) as Arc<dyn GuestTargetEffect>,
            )]);
            let service = Arc::new(GuestTargetService::new(
                Arc::clone(&runtime),
                zone(),
                effects,
            ));
            service.bind_session(generation).expect("bind session");
            Self { runtime, effect, service }
        }

        fn recording(generation: u64) -> Self {
            Self::new(RecordingEffect::new(), generation)
        }

        async fn realize(&self, request: GuestRealizeRequest) -> TargetControlResponse {
            self.service
                .handle(TargetControlRequest::Realize(request))
                .await
        }

        async fn observe(&self, name: &str, generation: u64) -> TargetControlResponse {
            self.service
                .handle(TargetControlRequest::Observe {
                    assignment: assignment(name, generation),
                })
                .await
        }

        async fn delete(&self, name: &str, generation: u64) -> TargetControlResponse {
            self.service
                .handle(TargetControlRequest::Delete {
                    assignment: assignment(name, generation),
                })
                .await
        }

        async fn adopt(&self, name: &str, generation: u64) -> TargetControlResponse {
            self.service
                .handle(TargetControlRequest::Adopt {
                    assignment: assignment(name, generation),
                })
                .await
        }

        fn instance(&self, name: &str) -> Option<TargetResourceInstance> {
            self.runtime.instance(&source(name))
        }
    }

    fn guest() -> TargetRef {
        TargetRef::guest("workload").expect("guest ref")
    }

    fn zone() -> ZoneId {
        ZoneId::parse(AUTHORITY_ZONE).expect("zone")
    }

    fn source(name: &str) -> ResourceKey {
        ResourceKey::new(AUTHORITY_ZONE, TARGET_TYPE, name)
    }

    fn spec() -> Vec<u8> {
        br#"{"endpoint":{"kind":"relay"}}"#.to_vec()
    }

    fn assignment(name: &str, generation: u64) -> TargetControlAssignment {
        TargetControlAssignment::new(source(name), [7; 16], 3, generation)
    }

    fn realize_request(name: &str, generation: u64, spec: Vec<u8>) -> GuestRealizeRequest {
        GuestRealizeRequest::new(
            assignment(name, generation),
            spec.clone(),
            target_local_spec_digest(&spec),
            format!("/run/d2b/{name}.sock"),
        )
    }

    #[tokio::test]
    async fn realize_records_once_applies_the_exact_spec_and_reports_ready() {
        let f = Fixture::recording(1);
        let request = realize_request("relay", 1, spec());

        let first = f.realize(request.clone()).await;
        let TargetControlResponse::Realized { realization } = &first else {
            panic!("first realize: {first:?}");
        };
        assert_eq!(realization.state(), TargetInstanceState::Ready, "the effect is serving");
        assert_eq!(realization.local_handle(), "/run/d2b/relay.sock");
        assert_eq!(realization.spec_digest(), request.spec_digest());

        let second = f.realize(realize_request("relay", 1, spec())).await;
        let TargetControlResponse::Realized { realization } = &second else {
            panic!("second realize: {second:?}");
        };
        assert_eq!(realization.source(), &source("relay"));
        assert_eq!(f.runtime.instances().len(), 1, "one realization per host-owned source");
        assert_eq!(f.effect.realized().len(), 2, "the effect is re-applied idempotently");
        assert_eq!(f.effect.realized()[0].1, spec(), "the exact bytes reach the effect");
        assert_eq!(f.effect.realized()[0].2, request.spec_digest());
        assert_eq!(
            f.observe("relay", 1).await,
            TargetControlResponse::Observed(TargetObservation::Ready { session_generation: 1 })
        );
    }

    #[tokio::test]
    async fn a_stale_generation_performs_no_effect_and_answers_session_unavailable() {
        let f = Fixture::recording(2);

        assert_eq!(
            f.realize(realize_request("relay", 1, spec())).await,
            TargetControlResponse::SessionUnavailable
        );
        assert_eq!(
            f.observe("relay", 1).await,
            TargetControlResponse::SessionUnavailable
        );
        assert_eq!(f.delete("relay", 1).await, TargetControlResponse::SessionUnavailable);
        assert_eq!(f.adopt("relay", 1).await, TargetControlResponse::SessionUnavailable);
        assert!(f.runtime.instances().is_empty(), "no stale request created state");
        assert!(f.effect.realized().is_empty(), "the effect never saw the stale spec");
        assert!(f.effect.deleted().is_empty());
        assert!(f.effect.adopted().is_empty());
    }

    #[tokio::test]
    async fn a_live_realization_survives_the_reconnect_and_the_old_generation_stops_working() {
        let f = Fixture::recording(1);
        f.realize(realize_request("relay", 1, spec())).await;

        f.service.bind_session(2).expect("reconnect generation");
        assert_eq!(
            f.observe("relay", 1).await,
            TargetControlResponse::SessionUnavailable,
            "the lost generation cannot observe its old realization"
        );
        assert!(f.instance("relay").is_some(), "desired realization state is untouched");
        assert_eq!(
            f.observe("relay", 2).await,
            TargetControlResponse::Observed(TargetObservation::Ready { session_generation: 1 }),
            "the live generation sees the stored realization; only adoption re-binds it"
        );
        let adopted = f.adopt("relay", 2).await;
        let TargetControlResponse::Adopted(GuestAdoption::Adopted(instance)) = &adopted else {
            panic!("adopt: {adopted:?}");
        };
        assert_eq!(instance.session_generation(), 2, "adoption re-binds to the live generation");
    }

    #[tokio::test]
    async fn delete_removes_only_its_own_instance_and_its_own_effect() {
        let f = Fixture::recording(1);
        f.realize(realize_request("relay", 1, spec())).await;
        f.realize(realize_request("other", 1, spec())).await;

        assert_eq!(f.delete("relay", 1).await, TargetControlResponse::Deleted);
        assert_eq!(f.effect.deleted(), vec![source("relay")]);
        assert!(f.instance("relay").is_none());
        assert!(f.instance("other").is_some(), "the sibling realization stays");
        assert_eq!(f.runtime.instances().len(), 1);

        // Deleting an absent source is still `deleted` and runs no effect.
        assert_eq!(f.delete("relay", 1).await, TargetControlResponse::Deleted);
        assert_eq!(f.effect.deleted(), vec![source("relay")], "an absent delete runs no effect");
    }

    #[tokio::test]
    async fn a_foreign_zone_key_is_refused_without_creating_state() {
        let f = Fixture::recording(1);
        let foreign = GuestRealizeRequest::new(
            TargetControlAssignment::new(
                ResourceKey::new("other-zone", TARGET_TYPE, "relay"),
                [7; 16],
                3,
                1,
            ),
            spec(),
            target_local_spec_digest(&spec()),
            "/run/d2b/relay.sock",
        );
        assert_eq!(
            f.service.handle(TargetControlRequest::Realize(foreign)).await,
            TargetControlResponse::SessionUnavailable
        );
        assert!(f.runtime.instances().is_empty());
        assert!(f.effect.realized().is_empty());
    }

    #[tokio::test]
    async fn a_key_with_no_target_local_effect_is_refused_without_creating_state() {
        let f = Fixture::recording(1);
        let unknown = GuestRealizeRequest::new(
            TargetControlAssignment::new(
                ResourceKey::new(AUTHORITY_ZONE, "Role", "admin"),
                [7; 16],
                3,
                1,
            ),
            spec(),
            target_local_spec_digest(&spec()),
            "/run/d2b/admin.sock",
        );
        assert_eq!(
            f.service.handle(TargetControlRequest::Realize(unknown)).await,
            TargetControlResponse::SessionUnavailable
        );
        assert!(f.runtime.instances().is_empty(), "an unknown type never becomes a realization");
    }

    #[tokio::test]
    async fn a_substituted_spec_is_refused_before_the_effect_sees_it() {
        let f = Fixture::recording(1);
        let request = realize_request("relay", 1, spec());
        let substituted = GuestRealizeRequest::new(
            request.assignment().clone(),
            br#"{"endpoint":{"kind":"hostile"}}"#.to_vec(),
            request.spec_digest().to_owned(),
            request.local_handle().to_owned(),
        );
        assert_eq!(
            f.service.handle(TargetControlRequest::Realize(substituted)).await,
            TargetControlResponse::SessionUnavailable
        );
        assert!(f.runtime.instances().is_empty(), "nothing is recorded for a mismatched spec");
        assert!(f.effect.realized().is_empty(), "the effect never sees a mismatched spec");

        // The same bytes with the right commitment are accepted: the refusal
        // is the commitment, not the transport.
        assert!(matches!(
            f.realize(request).await,
            TargetControlResponse::Realized { .. }
        ));
        assert_eq!(f.effect.realized().len(), 1);
    }

    #[tokio::test]
    async fn a_failed_effect_leaves_the_realization_realizing_instead_of_ready() {
        let f = Fixture::new(RecordingEffect::failing(GuestTargetEffectError::Unavailable), 1);
        let response = f.realize(realize_request("relay", 1, spec())).await;
        let TargetControlResponse::Realized { realization } = &response else {
            panic!("realize: {response:?}");
        };
        assert_eq!(realization.state(), TargetInstanceState::Realizing);
        assert_eq!(
            f.observe("relay", 1).await,
            TargetControlResponse::Observed(TargetObservation::Realizing { session_generation: 1 })
        );
    }

    #[tokio::test]
    async fn adoption_rebinds_and_recovers_the_target_local_effect() {
        let f = Fixture::recording(1);
        f.realize(realize_request("relay", 1, spec())).await;
        f.service.bind_session(2).expect("reconnect generation");

        let response = f.adopt("relay", 2).await;
        let TargetControlResponse::Adopted(GuestAdoption::Adopted(instance)) = &response else {
            panic!("adopt: {response:?}");
        };
        assert_eq!(instance.session_generation(), 2, "adoption re-binds, never inherits");
        assert_eq!(instance.state(), TargetInstanceState::Ready, "the present effect re-serves");
        assert_eq!(f.effect.adopted(), vec![source("relay")]);
        assert_eq!(f.runtime.instances().len(), 1);
    }

    #[tokio::test]
    async fn adoption_reports_missing_when_the_target_local_effect_is_gone() {
        let f = Fixture::new(RecordingEffect::absent(), 1);
        f.realize(realize_request("relay", 1, spec())).await;
        f.service.bind_session(2).expect("reconnect generation");

        assert_eq!(
            f.adopt("relay", 2).await,
            TargetControlResponse::Adopted(GuestAdoption::Missing),
            "an absent effect is never reported as adopted"
        );
        assert!(f.instance("relay").is_none(), "the stale record is forgotten");
    }

    #[tokio::test]
    async fn adoption_reports_missing_when_the_effect_cannot_confirm_the_realization() {
        let effect = RecordingEffect::new();
        *effect.discovery.lock().expect("discovery") = Some(GuestTargetEffectError::Unavailable);
        let f = Fixture::new(Arc::clone(&effect), 1);
        f.realize(realize_request("relay", 1, spec())).await;
        f.service.bind_session(2).expect("reconnect generation");

        assert_eq!(
            f.adopt("relay", 2).await,
            TargetControlResponse::Adopted(GuestAdoption::Missing),
            "an unconfirmed realization is never reported as adopted"
        );
        assert!(f.instance("relay").is_none(), "the unconfirmed record is forgotten");
    }

    #[test]
    fn the_registered_service_publishes_exactly_the_protocol_method() {
        let f = Fixture::recording(1);
        let services = target_control_services(Arc::clone(&f.service));
        assert_eq!(services.len(), 1);
        let surface = services.get(TARGET_CONTROL_SERVICE).expect("registered service");
        assert_eq!(
            surface.methods.keys().collect::<Vec<_>>(),
            vec![TARGET_CONTROL_METHOD],
            "the service publishes exactly the one protocol method"
        );
        assert!(surface.streams.is_empty());
    }

    #[tokio::test]
    async fn frames_round_trip_over_a_real_authenticated_session() {
        let state_dir = tempfile::tempdir().expect("state directory");
        let guest_runtime = GuestRuntime::new(
            guest_identity(1),
            "/run/d2b/guest-broker.sock".into(),
            997,
            AdmissionLimits::guest_default(),
            state_dir.path(),
        )
        .await
        .expect("Guest runtime");
        let parent_private_bytes = [2_u8; 32];
        let guest_private_bytes = [3_u8; 32];
        let parent_public = x25519_public_key(&parent_private_bytes).expect("parent public key");
        let guest_public = x25519_public_key(&guest_private_bytes).expect("guest public key");
        let parent_private = Secret32::new(parent_private_bytes).expect("parent private key");
        let guest_private = Secret32::new(guest_private_bytes).expect("guest private key");
        let parent_policy =
            d2b_contracts_zone_session::v3::component_session::EndpointPolicyIdentity::from(
                &guest_identity(1).endpoint_policy(),
            );
        let (left, right) = tokio::io::duplex(64 * 1024);
        let parent = tokio::spawn(async move {
            SessionEngine::establish_initiator_with_generation_discovery(
                FramedVsockTransport::new(left),
                parent_policy,
                HandshakeCredentials::Kk {
                    local_private: parent_private,
                    remote_public: guest_public,
                },
                std::time::Instant::now(),
            )
            .await
        });
        let (session, lease) = guest_runtime
            .establish_component_session(
                FramedVsockTransport::new(right),
                guest_private,
                parent_public,
            )
            .await
            .expect("guest accepts the parent session");
        let generation = lease.generation();
        let parent_engine = parent.await.expect("parent task").expect("parent session");

        let f = Fixture::new(RecordingEffect::new(), generation);
        let serving = tokio::spawn(
            session
                .into_ttrpc_handle()
                .serve_ttrpc_services(target_control_services(Arc::clone(&f.service))),
        );
        let driver: Arc<dyn d2b_session::ComponentSessionDriver> =
            Arc::new(parent_engine.into_driver());
        let transport = SessionTtrpcClient::new(driver);
        let client = transport.client();

        // Realize over the wire: the service applies the exact spec and the
        // reply carries the ready realization back.
        let request = realize_request("relay", generation, spec());
        let response = call(&client, TargetControlRequest::Realize(request.clone())).await;
        let TargetControlResponse::Realized { realization } = &response else {
            panic!("wire realize: {response:?}");
        };
        assert_eq!(realization.state(), TargetInstanceState::Ready);
        assert_eq!(realization.source(), &source("relay"));
        assert_eq!(f.effect.realized()[0].1, spec());

        // A stale generation over the same wire is fenced by the daemon's
        // live generation: SessionUnavailable, no effect, no state.
        let stale = TargetControlRequest::Realize(GuestRealizeRequest::new(
            assignment("stale", generation + 7),
            spec(),
            target_local_spec_digest(&spec()),
            "/run/d2b/stale.sock",
        ));
        assert_eq!(call(&client, stale).await, TargetControlResponse::SessionUnavailable);
        assert!(f.instance("stale").is_none());

        // Observe, adopt and delete round-trip on the same registration.
        assert_eq!(
            call(
                &client,
                TargetControlRequest::Observe { assignment: assignment("relay", generation) }
            )
            .await,
            TargetControlResponse::Observed(TargetObservation::Ready {
                session_generation: generation
            })
        );
        assert!(matches!(
            call(
                &client,
                TargetControlRequest::Adopt { assignment: assignment("relay", generation) }
            )
            .await,
            TargetControlResponse::Adopted(GuestAdoption::Adopted(_))
        ));
        assert_eq!(
            call(
                &client,
                TargetControlRequest::Delete { assignment: assignment("relay", generation) }
            )
            .await,
            TargetControlResponse::Deleted
        );
        assert_eq!(f.effect.deleted(), vec![source("relay")]);

        // A payload that is not a frame, and a frame with a foreign protocol
        // token, are both refused without touching the service.
        assert!(raw_call(&client, b"not-a-frame".to_vec()).await.is_err());
        let mut foreign: serde_json::Value = serde_json::from_slice(
            &TargetControlFrame::new(TargetControlRequest::Observe {
                assignment: assignment("relay", generation),
            })
            .encode(),
        )
        .expect("frame json");
        foreign["protocol"] = serde_json::Value::String("d2b.target-control.v0".to_owned());
        assert!(
            raw_call(&client, serde_json::to_vec(&foreign).expect("foreign frame"))
                .await
                .is_err()
        );

        serving.abort();
        drop(lease);
        let _ = serving.await;
    }

    /// One encoded round trip over the real service registration.
    async fn call(
        client: &ttrpc::r#async::Client,
        request: TargetControlRequest,
    ) -> TargetControlResponse {
        let payload = TargetControlFrame::new(request).encode();
        let response = raw_call(client, payload).await.expect("target-control reply");
        TargetControlResponse::decode(&response).expect("decodable reply")
    }

    async fn raw_call(
        client: &ttrpc::r#async::Client,
        payload: Vec<u8>,
    ) -> ttrpc::Result<Vec<u8>> {
        let response = client
            .request(ttrpc::Request {
                service: TARGET_CONTROL_SERVICE.to_owned(),
                method: TARGET_CONTROL_METHOD.to_owned(),
                payload,
                ..Default::default()
            })
            .await?;
        Ok(response.payload)
    }

    fn guest_identity(generation: u64) -> GuestIdentity {
        GuestIdentity::new(
            ResourceRef::parse("Guest/workload").expect("Guest ref"),
            ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").expect("Guest UID"),
            zone(),
            BootIdentity::from_kernel_boot_id("guest-target-service-test").expect("boot identity"),
            SessionPurpose::parse(GUEST_COMPONENT_SESSION_PURPOSE).expect("purpose"),
            SchemaFingerprint::parse(format!("sha256:{}", "1".repeat(64))).expect("schema"),
            ReconnectGeneration::new(generation).expect("generation"),
            1,
            1,
            1,
        )
        .expect("Guest identity")
    }
}
