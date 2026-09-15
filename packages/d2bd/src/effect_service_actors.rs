//! Effect-service actors (U8, KTD5): one ractor actor per declared effect
//! service, linked under a per-zone supervisor that mirrors `ResourceManager`
//! supervision (`packages/d2b-resource-runtime/src/manager.rs:1187-1211`):
//! a crashed or killed service actor is respawned from its durable row, so a
//! stop/crash, kill, or unexpected stop never leaves a service unhosted;
//! requeue timing is `ractor::time` timers (never threads); and each respawn
//! or provider-set republish bumps the service's generational binding
//! revision so bindings made stale by the bump refuse instead of dispatching
//! against a dead or superseded actor.
//!
//! This module ships the actor machinery around a small FIXTURE service trait
//! ([`EffectService`]); no real provider integration lives here. Hosting is
//! wired at the provider composition site: `ProviderSet::start`
//! (`provider_lifecycle.rs`) hosts one actor per DECLARED effect service -
//! every `ServiceDecl` a started provider declares becomes an
//! [`EffectServiceRow`] on the zone's supervisor, rebuilt from that durable
//! row on every respawn.
//!
//! The rendezvous binding consumes [`EffectServiceBinding`]: the forwarded
//! operation resolves to its declaring service through the declared
//! methods' `operation` facets (KD6, U7), the rendezvous captures
//! [`EffectServiceBinding::revision`] when the call starts and dispatches
//! through [`EffectServiceBinding::call_expected`], so an in-flight call
//! against a revision that a respawn or republish moved past refuses with a
//! dedicated code ([`EffectServiceError::StaleRevision`], surfaced on the
//! forward carrier as the rendezvous's `stale-revision` refusal) instead of
//! hanging or hitting the wrong generation. The hosting seam
//! (`resolve_effect_service`/`publish_effect_service` and the
//! operation-resolution query on `ProviderRuntime`) is the composition
//! face. `kill` and `actor_id` stay test-only harness surface and carry the
//! dead-code allowance as the tree's marker for exactly that state, with
//! U9+ notes.


use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use d2b_provider_toolkit::ServiceDecl;
use ractor::{Actor, ActorCell, ActorProcessingErr, ActorRef, SupervisionEvent};
use thiserror::Error;
use tokio::sync::oneshot;

/// Fixture request payload. The rendezvous will swap this for the wire type
/// at U8 integration.
pub type EffectRequest = Vec<u8>;

/// Fixture response payload. The rendezvous will swap this for the wire type
/// at U8 integration.
pub type EffectResponse = Vec<u8>;

/// Fixture-level refusal set for effect-service calls (KTD7 shape: a
/// machine-actionable closed code set, never a hang and never flattened to a
/// generic `unregistered` outcome).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum EffectServiceError {
    #[error("no service `{service}` is published in zone `{zone}`")]
    UnboundService { zone: String, service: String },
    #[error("no hosted effect service declares an operation `{operation}`")]
    OperationUnserved { operation: String },
    #[error("row for `{service}` belongs to zone `{row_zone}`, not `{zone}`")]
    WrongZone { zone: String, service: String, row_zone: String },
    #[error("binding for `{service}` is stale: revision {current} != expected {expected} (KTD5)")]
    StaleRevision { service: String, expected: u64, current: u64 },
    #[error("service `{service}` is not running; its binding targets a dead actor (KTD5)")]
    ServiceUnavailable { service: String },
    #[error("service `{service}` died mid-call; the caller's revision is stale (KTD5)")]
    InFlightStale { service: String },
    #[error("`{service}` declined: {reason}")]
    Declined { service: String, reason: String },
}

/// A fixture effect service: one handle-loop shape plus an optional
/// timer-driven poll tick. Real provider services implement this over
/// carrier-delivered invocations; the rendezvous wiring is out of scope
/// here.
#[async_trait]
pub trait EffectService: Send + Sync + 'static {
    /// Handle one request.
    async fn handle(&self, request: EffectRequest) -> Result<EffectResponse, EffectServiceError>;

    /// Timer-driven poll tick (default: nothing). The actor's requeue timer
    /// drives this on the declared interval - never a thread.
    async fn poll(&self) {}
}

/// Rebuilds a service from a durable row; a respawn calls `build` again,
/// exactly like `ResourceManager` re-creates its drivers from the committed
/// spec row.
pub trait EffectServiceFactory: Send + Sync + 'static {
    fn build(&self) -> Arc<dyn EffectService>;
}

/// The durable declaration row for one effect service (U8): the production
/// declaration the declaring provider made about the service, plus the
/// factory that rebuilds the service instance from this row. The supervisor
/// treats this struct as its durable source and respawns a service
/// exclusively from it (mirroring `ResourceManager` re-creating its drivers
/// from the committed spec row).
#[derive(Clone)]
pub struct EffectServiceRow {
    /// The zone that hosts this service.
    pub zone: String,
    /// The service identity the session layer addresses.
    pub service: String,
    /// The production declaration facets (methods, attach kinds, streams,
    /// endpoint policy). The rendezvous validates method calls against
    /// these at U8b.
    pub decl: ServiceDecl,
    /// Rebuilds the service on respawn.
    pub factory: Arc<dyn EffectServiceFactory>,
}

impl EffectServiceRow {
    /// One declared service of a started provider, hosted in `zone`.
    pub fn declared(
        zone: &str,
        service: &ServiceDecl,
        factory: Arc<dyn EffectServiceFactory>,
    ) -> Self {
        Self {
            zone: zone.to_owned(),
            service: service.id.to_owned(),
            decl: *service,
            factory,
        }
    }
}

impl fmt::Debug for EffectServiceRow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EffectServiceRow")
            .field("zone", &self.zone)
            .field("service", &self.service)
            .finish_non_exhaustive()
    }
}

/// Generational binding for one effect service: the supervisor bumps the
/// shared revision on every respawn or republish, so a caller that captured
/// `revision()` at bind time can refuse stale traffic (KTD5, U8). The
/// rendezvous binding consumes this counter at U8 integration.
#[derive(Clone, Debug)]
pub struct EffectServiceBinding {
    service: String,
    actor: ActorRef<EffectServiceMsg>,
    revision: Arc<AtomicU64>,
    /// The production declaration facets the binding serves. The rendezvous
    /// validates the method an operation names against these before it
    /// dispatches (U8b).
    decl: ServiceDecl,
}

impl EffectServiceBinding {
    fn new(
        service: String,
        actor: ActorRef<EffectServiceMsg>,
        revision: u64,
        decl: ServiceDecl,
    ) -> Self {
        Self {
            service,
            actor,
            revision: Arc::new(AtomicU64::new(revision)),
            decl,
        }
    }

    /// The service this binding names.
    pub fn service(&self) -> &str {
        &self.service
    }

    /// The declared facets the service serves (request methods, attach
    /// kinds, streams, endpoint policy). An operation resolved to this
    /// binding through one of these methods; the rendezvous names the
    /// resolving method in its records.
    pub fn decl(&self) -> &ServiceDecl {
        &self.decl
    }

    /// The current generational revision. A respawn or republish bumps it;
    /// compare a captured value against this to detect staleness.
    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::SeqCst)
    }

    /// The actor generation this binding currently names. After a respawn
    /// the supervisor's live binding names a different actor - resolve again
    /// to re-bind.
    ///
    /// Test-only harness surface (the supervision tests observe the fresh
    /// generation); no production path reads actor ids. U9+ note: a
    /// telemetry surface may consume this.
    #[allow(dead_code)]
    pub fn actor_id(&self) -> ractor::ActorId {
        self.actor.get_id()
    }

    /// Kill the bound actor (crash a service mid-supervision). The
    /// supervisor respawns it from the durable row and bumps the revision.
    ///
    /// Test-only harness surface (the supervision tests kill actors
    /// mid-call); no production path kills a hosted service. U9+ note: the
    /// operator control plane may consume this.
    #[allow(dead_code)]
    pub fn kill(&self) {
        self.actor.get_cell().kill();
    }

    /// Call through the binding at its current revision; a mid-flight death
    /// of the actor surfaces as [`EffectServiceError::InFlightStale`] - the
    /// caller sees a refusal, never a hang.
    pub async fn call(&self, request: EffectRequest) -> Result<EffectResponse, EffectServiceError> {
        self.send(request).await
    }

    /// Call guarded by a captured revision (KTD5): if a respawn or republish
    /// bumped the revision since the caller captured `expected`, refuse
    /// before dispatch. The rendezvous uses this admission check.
    pub async fn call_expected(
        &self,
        expected: u64,
        request: EffectRequest,
    ) -> Result<EffectResponse, EffectServiceError> {
        let current = self.revision();
        if current != expected {
            return Err(EffectServiceError::StaleRevision {
                service: self.service.clone(),
                expected,
                current,
            });
        }
        self.call(request).await
    }

    async fn send(&self, request: EffectRequest) -> Result<EffectResponse, EffectServiceError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.actor
            .send_message(EffectServiceMsg::Call { request, reply: reply_tx })
            .map_err(|_| EffectServiceError::ServiceUnavailable { service: self.service.clone() })?;
        reply_rx
            .await
            .map_err(|_| EffectServiceError::InFlightStale { service: self.service.clone() })?
    }
}

/// Messages an effect-service actor handles.
pub(crate) enum EffectServiceMsg {
    Call {
        request: EffectRequest,
        reply: oneshot::Sender<Result<EffectResponse, EffectServiceError>>,
    },
    /// Requeue tick: the poll loop schedules the next one with
    /// `ractor::time::send_after` - no threads (KTD9).
    Poll,
}

pub(crate) struct EffectServiceActorArgs {
    service: Arc<dyn EffectService>,
    poll_interval: Duration,
}

pub(crate) struct EffectServiceActorState {
    service: Arc<dyn EffectService>,
    poll_interval: Duration,
}

/// One hosted effect service. `handle` awaits the service's own future
/// inline - the fixture shape; production services forward long effects onto
/// an unbounded channel pump like `ResourceActor` (KTD12) so the mailbox
/// never blocks.
pub(crate) struct EffectServiceActor;

impl EffectServiceActor {
    pub const fn new() -> Self {
        Self
    }
}

impl Default for EffectServiceActor {
    fn default() -> Self {
        Self::new()
    }
}

impl Actor for EffectServiceActor {
    type Msg = EffectServiceMsg;
    type State = EffectServiceActorState;
    type Arguments = EffectServiceActorArgs;

    async fn pre_start(
        &self,
        _myself: ActorRef<EffectServiceMsg>,
        args: EffectServiceActorArgs,
    ) -> Result<EffectServiceActorState, ActorProcessingErr> {
        Ok(EffectServiceActorState {
            service: args.service,
            poll_interval: args.poll_interval,
        })
    }

    async fn post_start(
        &self,
        myself: ActorRef<EffectServiceMsg>,
        state: &mut EffectServiceActorState,
    ) -> Result<(), ActorProcessingErr> {
        // First poll after the timeout; every poll handler reschedules the
        // next (ractor::time timers, never threads). The timer future must
        // be driven, not dropped - a bare `let _` would cancel the poll.
        tokio::spawn(ractor::time::send_after(
            state.poll_interval,
            myself.get_cell(),
            || EffectServiceMsg::Poll,
        ));
        Ok(())
    }

    async fn handle(
        &self,
        myself: ActorRef<EffectServiceMsg>,
        message: EffectServiceMsg,
        state: &mut EffectServiceActorState,
    ) -> Result<(), ActorProcessingErr> {
        match message {
            EffectServiceMsg::Call { request, reply } => {
                let result = state.service.handle(request).await;
                // If the actor dies before this sends, the caller's receiver
                // closes and surfaces `InFlightStale` - never a hang.
                let _ = reply.send(result);
            }
            EffectServiceMsg::Poll => {
                state.service.poll().await;
                tokio::spawn(ractor::time::send_after(
                    state.poll_interval,
                    myself.get_cell(),
                    || EffectServiceMsg::Poll,
                ));
            }
        }
        Ok(())
    }
}

/// Supervisor messages (crate-visible; U8 integration publishes declared
/// services and resolves live bindings through these).
pub enum EffectServiceSupervisorMsg {
    /// Publish (or republish) a service row. A republish bumps the binding
    /// revision and rebuilds the actor from the new row. Reply: the
    /// resulting binding.
    Publish {
        row: EffectServiceRow,
        reply: oneshot::Sender<Result<EffectServiceBinding, EffectServiceError>>,
    },
    /// Resolve the live binding for a service. Reply: the current binding.
    ///
    /// Test-only harness surface: the rendezvous resolves by operation
    /// ([`Self::ResolveOperation`]), and the supervision tests resolve by
    /// name after a kill. U9+ note: a composition/operator dialogue
    /// re-drives rows by name and consumes this.
    #[allow(dead_code)]
    Resolve {
        service: String,
        reply: oneshot::Sender<Result<EffectServiceBinding, EffectServiceError>>,
    },
    /// Resolve the live binding of the service that declares one operation
    /// (KD6, U7): the operation the forwarded call names is served exactly
    /// when a declared method's `operation` facet names it, and the
    /// declaring service answers it. Reply: the declaring service's current
    /// binding, or [`EffectServiceError::OperationUnserved`] when no hosted
    /// service declares the operation.
    ResolveOperation {
        operation: String,
        reply: oneshot::Sender<Result<EffectServiceBinding, EffectServiceError>>,
    },
}

/// Supervisor arguments.
pub(crate) struct EffectServiceSupervisorArgs {
    /// The zone this supervisor owns; rows from any other zone are refused.
    pub zone: String,
    /// Durable rows to recover at start (restart recovery: one actor per
    /// row, mirroring `ResourceManager::pre_start`).
    pub rows: Vec<EffectServiceRow>,
    /// Requeue cadence for the hosted services' poll timers.
    pub poll_interval: Duration,
}

pub(crate) struct EffectServiceSupervisorState {
    zone: String,
    poll_interval: Duration,
    rows: HashMap<String, EffectServiceRow>,
    bindings: HashMap<String, EffectServiceBinding>,
    actors_by_id: HashMap<ractor::ActorId, String>,
}

/// Per-zone supervisor for effect services (U8, KTD5): service actors are
/// linked children, respawned from their durable rows on failure.
pub(crate) struct EffectServiceSupervisor;

impl EffectServiceSupervisor {
    pub const fn new() -> Self {
        Self
    }
}

impl Default for EffectServiceSupervisor {
    fn default() -> Self {
        Self::new()
    }
}

impl EffectServiceSupervisorState {
    fn resolve(&self, service: &str) -> Result<EffectServiceBinding, EffectServiceError> {
        self.bindings.get(service).cloned().ok_or_else(|| EffectServiceError::UnboundService {
            zone: self.zone.clone(),
            service: service.to_string(),
        })
    }

    /// Resolve the live binding of the service that declares one operation
    /// (KD6, U7).
    ///
    /// A declared method whose `operation` facet names the operation is the
    /// declaration that the operation is the service's surface; the lowest
    /// service identity among the declaring rows wins, so the resolution is
    /// deterministic even when the row map iteration order is not. A row
    /// whose actor never spawned (a refused build) resolves through the
    /// bindings, which refuse it as unbound.
    fn resolve_for_operation(
        &self,
        operation: &str,
    ) -> Result<EffectServiceBinding, EffectServiceError> {
        let mut serving: Vec<&EffectServiceRow> = self
            .rows
            .values()
            .filter(|row| {
                row.decl
                    .methods
                    .iter()
                    .any(|method| method.operation == Some(operation))
            })
            .collect();
        serving.sort_by(|left, right| left.service.cmp(&right.service));
        match serving.first() {
            Some(row) => self.resolve(&row.service),
            None => Err(EffectServiceError::OperationUnserved {
                operation: operation.to_owned(),
            }),
        }
    }

    async fn publish(
        &mut self,
        myself: &ActorRef<EffectServiceSupervisorMsg>,
        row: EffectServiceRow,
    ) -> Result<EffectServiceBinding, EffectServiceError> {
        if row.zone != self.zone {
            return Err(EffectServiceError::WrongZone {
                zone: self.zone.clone(),
                service: row.service.clone(),
                row_zone: row.zone.clone(),
            });
        }
        if self.bindings.contains_key(&row.service) {
            // Republish (provider-set republish, KTD5): deregister the old
            // actor id before killing it. Supervision events race regular
            // mailbox traffic, so the old id must not be found when the
            // event arrives - otherwise the exit would respawn the old
            // generation on top of the new one (same reasoning as
            // `ResourceManager::handle`'s `DeletionComplete`).
            let binding = self.bindings.get(&row.service).expect("checked above");
            self.actors_by_id.remove(&binding.actor.get_id());
            binding.actor.get_cell().kill();
        }
        self.rows.insert(row.service.clone(), row.clone());
        self.spawn_service_actor(myself, &row).await
    }

    /// The row is durable in `rows` before the actor exists (F1-shaped
    /// boundary): a failed spawn leaves the row for the next publish or
    /// restart to retry.
    async fn spawn_service_actor(
        &mut self,
        myself: &ActorRef<EffectServiceSupervisorMsg>,
        row: &EffectServiceRow,
    ) -> Result<EffectServiceBinding, EffectServiceError> {
        let service = row.factory.build();
        let args = EffectServiceActorArgs { service, poll_interval: self.poll_interval };
        let (actor, _join) = EffectServiceActor::spawn_linked(
            None,
            EffectServiceActor::new(),
            args,
            myself.get_cell(),
        )
        .await
        .map_err(|error| EffectServiceError::Declined {
            service: row.service.clone(),
            reason: error.to_string(),
        })?;
        let binding = match self.bindings.get_mut(&row.service) {
            // Respawn or republish of a live service: bump its generation
            // and point the shared binding at the fresh actor.
            Some(existing) => {
                existing.revision.fetch_add(1, Ordering::SeqCst);
                existing.actor = actor.clone();
                existing.decl = row.decl;
                existing.clone()
            }
            None => {
                let binding =
                    EffectServiceBinding::new(row.service.clone(), actor.clone(), 1, row.decl);
                self.bindings.insert(row.service.clone(), binding.clone());
                binding
            }
        };
        self.actors_by_id.insert(actor.get_id(), row.service.clone());
        Ok(binding)
    }
}

impl Actor for EffectServiceSupervisor {
    type Msg = EffectServiceSupervisorMsg;
    type State = EffectServiceSupervisorState;
    type Arguments = EffectServiceSupervisorArgs;

    async fn pre_start(
        &self,
        myself: ActorRef<EffectServiceSupervisorMsg>,
        args: EffectServiceSupervisorArgs,
    ) -> Result<EffectServiceSupervisorState, ActorProcessingErr> {
        let mut state = EffectServiceSupervisorState {
            zone: args.zone,
            poll_interval: args.poll_interval,
            rows: HashMap::new(),
            bindings: HashMap::new(),
            actors_by_id: HashMap::new(),
        };
        // Restart recovery: spawn one actor per durable row, mirroring
        // `ResourceManager::pre_start`. The row is recorded before the
        // actor exists (F1-shaped boundary, same as `publish`): a respawn
        // after a later kill must find the durable row in `rows` even when
        // the supervisor started from that row.
        let zone = state.zone.clone();
        for row in args.rows.into_iter().filter(|row| row.zone == zone) {
            state.rows.insert(row.service.clone(), row.clone());
            let _ = state.spawn_service_actor(&myself, &row).await;
        }
        Ok(state)
    }

    async fn handle(
        &self,
        myself: ActorRef<EffectServiceSupervisorMsg>,
        message: EffectServiceSupervisorMsg,
        state: &mut EffectServiceSupervisorState,
    ) -> Result<(), ActorProcessingErr> {
        match message {
            EffectServiceSupervisorMsg::Publish { row, reply } => {
                reply.send(state.publish(&myself, row).await).ok();
            }
            EffectServiceSupervisorMsg::Resolve { service, reply } => {
                reply.send(state.resolve(&service)).ok();
            }
            EffectServiceSupervisorMsg::ResolveOperation { operation, reply } => {
                reply.send(state.resolve_for_operation(&operation)).ok();
            }
        }
        Ok(())
    }

    async fn handle_supervisor_evt(
        &self,
        myself: ActorRef<EffectServiceSupervisorMsg>,
        message: SupervisionEvent,
        state: &mut EffectServiceSupervisorState,
    ) -> Result<(), ActorProcessingErr> {
        match message {
            SupervisionEvent::ActorFailed(who, _panic) => {
                supervise_exit(state, who, &myself).await
            }
            SupervisionEvent::ActorTerminated(who, _last_state, _reason) => {
                supervise_exit(state, who, &myself).await
            }
            _ => {}
        }
        Ok(())
    }
}

/// Supervision mirror of `manager.rs::supervise_exit`: a crashed or killed
/// service actor is respawned from its durable row. Bookkeeping is removed
/// first so a racing exit event cannot double-spawn.
async fn supervise_exit(
    state: &mut EffectServiceSupervisorState,
    who: ActorCell,
    myself: &ActorRef<EffectServiceSupervisorMsg>,
) {
    let Some(service) = state.actors_by_id.remove(&who.get_id()) else {
        return;
    };
    let Some(row) = state.rows.get(&service).cloned() else {
        return;
    };
    // Respawn bumps the generational binding revision (KTD5).
    let _ = state.spawn_service_actor(myself, &row).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU64;

    use d2b_resource_types::ServiceMethod;
    use tokio::sync::Notify;

    /// Echo fixture: answers with the request payload.
    struct EchoService;

    #[async_trait]
    impl EffectService for EchoService {
        async fn handle(&self, request: EffectRequest) -> Result<EffectResponse, EffectServiceError> {
            Ok(request)
        }
    }

    /// Counts rebuilds from the durable row, so a respawn is observable.
    struct EchoFactory {
        builds: Arc<AtomicU64>,
    }

    impl EffectServiceFactory for EchoFactory {
        fn build(&self) -> Arc<dyn EffectService> {
            self.builds.fetch_add(1, Ordering::SeqCst);
            Arc::new(EchoService)
        }
    }

    /// Returns one pre-built service (gated/polling fixtures).
    struct OnceFactory(Arc<dyn EffectService>);

    impl EffectServiceFactory for OnceFactory {
        fn build(&self) -> Arc<dyn EffectService> {
            self.0.clone()
        }
    }

    /// Polling fixture: counts every timer-driven poll tick.
    struct TickerService {
        polls: Arc<AtomicU64>,
    }

    #[async_trait]
    impl EffectService for TickerService {
        async fn handle(&self, _request: EffectRequest) -> Result<EffectResponse, EffectServiceError> {
            Err(EffectServiceError::Declined {
                service: "ticker".to_string(),
                reason: "poll-only fixture".to_string(),
            })
        }

        async fn poll(&self) {
            self.polls.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// Gated fixture: parks inside `handle` until killed, signalling that
    /// the call is genuinely in flight.
    struct GatedService {
        entered: Arc<Notify>,
        release: Arc<Notify>,
    }

    #[async_trait]
    impl EffectService for GatedService {
        async fn handle(&self, _request: EffectRequest) -> Result<EffectResponse, EffectServiceError> {
            self.entered.notify_one();
            self.release.notified().await;
            Ok(b"released".to_vec())
        }
    }

    /// The zone-plane ping method of the fixture services.
    const PING_METHOD: ServiceMethod = ServiceMethod::zone_plane("ping");

    fn service_decl(id: &'static str) -> ServiceDecl {
        ServiceDecl {
            id,
            methods: &[PING_METHOD],
            attach_kinds: &[],
            streams: &[],
            endpoint_policy: None,
        }
    }

    fn echo_row(zone: &str, service: &'static str, builds: Arc<AtomicU64>) -> EffectServiceRow {
        EffectServiceRow::declared(zone, &service_decl(service), Arc::new(EchoFactory { builds }))
    }

    fn once_row(zone: &str, service: &'static str, svc: Arc<dyn EffectService>) -> EffectServiceRow {
        EffectServiceRow::declared(zone, &service_decl(service), Arc::new(OnceFactory(svc)))
    }

    async fn spawn_supervisor(
        zone: &str,
        rows: Vec<EffectServiceRow>,
        poll_interval: Duration,
    ) -> ActorRef<EffectServiceSupervisorMsg> {
        let args =
            EffectServiceSupervisorArgs { zone: zone.to_string(), rows, poll_interval };
        let (actor, _join) = Actor::spawn(None, EffectServiceSupervisor::new(), args)
            .await
            .expect("supervisor spawn");
        actor
    }

    async fn publish(
        supervisor: &ActorRef<EffectServiceSupervisorMsg>,
        row: EffectServiceRow,
    ) -> Result<EffectServiceBinding, EffectServiceError> {
        let (tx, rx) = oneshot::channel();
        supervisor
            .send_message(EffectServiceSupervisorMsg::Publish { row, reply: tx })
            .expect("supervisor accepts publishes");
        rx.await.expect("supervisor answered")
    }

    async fn resolve(
        supervisor: &ActorRef<EffectServiceSupervisorMsg>,
        service: &str,
    ) -> Result<EffectServiceBinding, EffectServiceError> {
        let (tx, rx) = oneshot::channel();
        supervisor
            .send_message(EffectServiceSupervisorMsg::Resolve {
                service: service.to_string(),
                reply: tx,
            })
            .expect("supervisor accepts resolves");
        rx.await.expect("supervisor answered")
    }

    async fn until(condition: impl Fn() -> bool) {
        for _ in 0..200 {
            if condition() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("condition never became true within the deadline");
    }

    /// Happy path: a published service answers through its binding, and a
    /// provider-set republish bumps the generational revision while the
    /// service keeps answering from the fresh instance (KTD5).
    #[tokio::test]
    async fn bound_service_answers_calls_through_its_supervisor() {
        let supervisor = spawn_supervisor("z", Vec::new(), Duration::from_secs(60)).await;
        let builds = Arc::new(AtomicU64::new(0));
        let binding = publish(&supervisor, echo_row("z", "echo", builds.clone()))
            .await
            .expect("publish");
        assert_eq!(binding.revision(), 1);

        let response = binding.call(b"ping".to_vec()).await.expect("call");
        assert_eq!(response, b"ping".to_vec(), "service answered");

        // Republish: revision bumps, and the new generation answers.
        let rebound = publish(&supervisor, echo_row("z", "echo", builds.clone()))
            .await
            .expect("republish");
        assert_eq!(rebound.revision(), 2, "republish bumped the revision");
        assert_eq!(builds.load(Ordering::SeqCst), 2, "fresh instance from the new row");
        let response = rebound.call(b"again".to_vec()).await.expect("call after republish");
        assert_eq!(response, b"again".to_vec());
    }

    /// Error: killing an actor mid-supervision respawns the service from its
    /// durable row; the next call succeeds and the revision bumped. The
    /// pre-crash binding refuses as stale (KTD5).
    #[tokio::test]
    async fn killed_actor_respawns_from_its_durable_row_and_bumps_revision() {
        let supervisor = spawn_supervisor("z", Vec::new(), Duration::from_secs(60)).await;
        let builds = Arc::new(AtomicU64::new(0));
        let binding = publish(&supervisor, echo_row("z", "echo", builds.clone()))
            .await
            .expect("publish");
        let response = binding.call(b"one".to_vec()).await.expect("call");
        assert_eq!(response, b"one".to_vec());
        let revision_before = binding.revision();
        let id_before = binding.actor_id();

        // Kill the actor mid-supervision (aborts any in-flight work).
        binding.kill();

        // The supervisor respawns from the durable row and bumps the
        // generational revision - observable through the shared counter.
        until(|| binding.revision() != revision_before).await;
        let respawned = resolve(&supervisor, "echo").await.expect("resolve after respawn");
        assert_eq!(respawned.revision(), revision_before + 1, "respawn bumped the revision");
        assert_ne!(respawned.actor_id(), id_before, "respawned actor is a fresh generation");
        assert_eq!(builds.load(Ordering::SeqCst), 2, "rebuilt from the durable row");

        // Stale bindings refuse (KTD5): the captured revision is stale, and
        // the pre-crash handle targets the dead actor.
        let stale = binding
            .call_expected(revision_before, b"stale".to_vec())
            .await
            .expect_err("stale revision must refuse");
        assert!(
            matches!(stale, EffectServiceError::StaleRevision { .. }),
            "got {stale:?}"
        );
        let dead = binding.call(b"dead".to_vec()).await.expect_err("dead actor must refuse");
        assert!(
            matches!(dead, EffectServiceError::ServiceUnavailable { .. }),
            "got {dead:?}"
        );

        // The next call succeeds against the respawned generation.
        let response = respawned.call(b"two".to_vec()).await.expect("call after respawn");
        assert_eq!(response, b"two".to_vec());
    }

    /// Edge: an in-flight call when the actor dies sees a refusal, not a
    /// hang, and the supervisor still respawns the service.
    #[tokio::test]
    async fn in_flight_call_against_a_killed_actor_refuses_instead_of_hanging() {
        let supervisor = spawn_supervisor("z", Vec::new(), Duration::from_secs(60)).await;
        let entered = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let binding = publish(
            &supervisor,
            once_row(
                "z",
                "gate",
                Arc::new(GatedService { entered: entered.clone(), release: release.clone() }),
            ),
        )
        .await
        .expect("publish");

        let caller = tokio::spawn({
            let binding = binding.clone();
            async move { binding.call(b"in-flight".to_vec()).await }
        });
        // Wait until the call is genuinely parked inside the service.
        entered.notified().await;

        binding.kill();

        let outcome = tokio::time::timeout(Duration::from_secs(2), caller)
            .await
            .expect("in-flight call must refuse, not hang");
        let error = outcome.expect("call task completed").expect_err("call must refuse");
        assert!(
            matches!(error, EffectServiceError::InFlightStale { .. }),
            "got {error:?}"
        );

        // The service still respawns from its durable row afterwards.
        until(|| binding.revision() != 1).await;
        let respawned = resolve(&supervisor, "gate").await.expect("resolve after respawn");
        assert_eq!(respawned.revision(), 2);
    }

    /// Edge: a requeue timer schedules the next poll after a timeout - no
    /// poll before the delay, polls arrive afterwards, and the handler
    /// reschedules the next tick (ractor::time timers, never threads).
    #[tokio::test]
    async fn requeue_timer_schedules_the_next_poll_after_a_timeout() {
        let polls = Arc::new(AtomicU64::new(0));
        let interval = Duration::from_millis(50);
        let supervisor = spawn_supervisor("z", Vec::new(), interval).await;
        let _binding = publish(
            &supervisor,
            once_row("z", "ticker", Arc::new(TickerService { polls: polls.clone() })),
        )
        .await
        .expect("publish");

        // Nothing before the first timeout: the requeue is a delayed timer,
        // not an immediate callback or a thread.
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(polls.load(Ordering::SeqCst), 0, "poll fired before its timeout");

        // The first poll arrives after the timeout...
        until(|| polls.load(Ordering::SeqCst) >= 1).await;
        // ...and the poll handler requeued the next one.
        until(|| polls.load(Ordering::SeqCst) >= 2).await;
    }

    /// Restart recovery: durable rows present at supervisor start spawn one
    /// actor per row (mirrors `manager.rs::ensure_commits_before_spawn_and_
    /// restart_recovers`).
    #[tokio::test]
    async fn durable_rows_recover_on_supervisor_start() {
        let builds = Arc::new(AtomicU64::new(0));
        let supervisor =
            spawn_supervisor("z", vec![echo_row("z", "echo", builds.clone())], Duration::from_secs(60))
                .await;
        let binding = resolve(&supervisor, "echo").await.expect("recovered service resolves");
        assert_eq!(binding.revision(), 1);

        let response = binding.call(b"recovered".to_vec()).await.expect("call");
        assert_eq!(response, b"recovered".to_vec());
        assert_eq!(builds.load(Ordering::SeqCst), 1, "built from the durable row");
    }
}