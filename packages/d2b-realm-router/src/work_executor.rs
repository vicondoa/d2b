//! Work executor: the single typed dispatch surface that ties the realm
//! entrypoint resolver, the host-resident authorization/idempotency router,
//! the host-resident durable execution table, and the bounded gateway
//! session lifecycle together behind one [`WorkExecutor::dispatch`] entry
//! point (ADR 0032/0045).
//!
//! [`WorkExecutor`] is host-side: it runs where the realm entrypoint table
//! decides whether a target is [`DispatchTarget::HostResident`] or
//! [`DispatchTarget::GatewayBacked`]. It holds no realm relay/session/
//! provider credential and no remote-node registry itself -- that state is
//! gateway-owned per ADR 0032. Concretely:
//!
//! 1. Resolve the request's realm target with the entrypoint table.
//! 2. `HostResident` → validate the addressed node against this executor's
//!    own local node identity, then authorize + dedup the request through
//!    this executor's *own* [`crate::OperationRouter`] (the host's
//!    authorization/idempotency owner for its scope) before ever touching
//!    the durable execution table. A capability-denied, missing-idempotency,
//!    principal-mismatch, or other refusal never reaches the table.
//! 3. `GatewayBacked` → the realm entrypoint table resolves a canonical
//!    `gateway` [`RealmTarget`] (the gateway guest boundary) and a canonical
//!    `target` [`RealmTarget`]. `WorkExecutor` hands both, unmodified, to an
//!    *injected* [`GatewayPort`] -- an already-authorized port the caller
//!    supplies per dispatch. `WorkExecutor` never constructs or owns a
//!    [`crate::remote_node::RemoteFullHostAdapter`] itself: that adapter
//!    (and the remote-node registry it wraps) remains gateway-side; see
//!    [`crate::remote_node::SingleGatewayPort`] for the reference port meant
//!    to run *inside* the gateway guest process it fronts, never embedded in
//!    a host-resident `WorkExecutor`. For a `DisplaySessionOpen` operation
//!    specifically, `WorkExecutor` also drives a
//!    [`crate::session_lifecycle::SessionLifecycle`] for that operation id,
//!    bounded by [`DEFAULT_MAX_GATEWAY_SESSIONS`] tracked sessions --
//!    lifecycle state is allocated only *after* the gateway port reports a
//!    successful dispatch, and a failed/rejected attempt evicts any
//!    lingering entry rather than consuming bounded capacity for an
//!    operation id that never actually succeeded.
//!
//! This module depends only on the router's own already-public types
//! (`crate::{...}`) plus `d2b_realm_core`/`serde`/`serde_json` (already
//! direct dependencies of this crate). It never depends on a transport or
//! codec impl in production code -- the dependency-direction gate still
//! applies -- and it holds no realm relay/session/provider credential and no
//! remote node registry: routing and dedup state for the host-resident scope
//! live in this executor's own embedded [`crate::OperationRouter`]; routing
//! and dedup state for the gateway-backed scope live entirely behind the
//! injected [`GatewayPort`], never inside this type.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use d2b_realm_core::{
    CapabilitySet, ConstellationError, ExecAttachRequest, ExecCancelRequest, ExecLogsRequest,
    ExecStartRequest, ExecutionId, ExecutionSummary, NodeId, OpaquePayload, OperationId,
    OperationKind, OperationRequest, PrincipalId, ProtocolToken, RealmTarget,
};

use crate::remote_node::{
    GatewayDispatchClassification, GatewayPort, RemoteDispatchOutcome, RemoteNodeError,
    RemotePeerClient,
};
use crate::session_lifecycle::{SessionLifecycle, SessionPhase};
use crate::target_resolver::{DispatchTarget, RealmEntrypointTable, ResolveError};
use crate::{Clock, OperationRouter, RouteDecision, SystemClock, route_decision_error};

/// Default bound on the number of gateway-backed display sessions one
/// `WorkExecutor` tracks concurrently. Bounded so a stream of
/// `DisplaySessionOpen` operations can never grow the session map without
/// limit.
pub const DEFAULT_MAX_GATEWAY_SESSIONS: usize = 4096;

/// The natural outcome of a host-resident exec-family dispatch, one variant
/// per [`OperationKind`] the durable execution table understands. Encoded to
/// an [`OpaquePayload`] for the host router's own dedup replay cache.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HostResidentOutcome {
    /// `ExecStart` was accepted and a new durable execution is tracked.
    Started(ExecutionSummary),
    /// `ExecAttach` resolved to the execution's current summary.
    Attached(ExecutionSummary),
    /// `ExecLogs` resolved to the execution's current summary (log bytes
    /// themselves are never part of this metadata).
    Logs(ExecutionSummary),
    /// `ExecCancel` completed; `true` if cancellation was newly requested.
    Cancelled(bool),
}

/// The result of one [`WorkExecutor::dispatch`] call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkDispatchOutcome {
    /// The realm entrypoint resolved host-resident; the durable execution
    /// table handled the request directly, after this executor's own
    /// authorization/idempotency gate accepted it. `original_operation_id`
    /// is this executor's own [`crate::OperationRouter`]'s dedup-lease
    /// owner: `req.operation_id` itself for a fresh `Accept`, or the
    /// first-attempt id for a `Replay` -- never a retry's own id. Callers
    /// MUST use it for any further in-progress status query keyed to the
    /// same logical operation.
    HostResident {
        original_operation_id: OperationId,
        outcome: HostResidentOutcome,
    },
    /// The realm entrypoint resolved gateway-backed; the injected
    /// [`GatewayPort`] handled the request. `original_operation_id` is
    /// [`RemoteDispatchOutcome::original_operation_id`] for `remote` --
    /// the gateway-side dedup lease owner, never a retry's own id.
    /// `session_phase` is set only for a `DisplaySessionOpen` operation
    /// (the lifecycle this executor tracks, keyed by
    /// `original_operation_id`); every other gateway-backed kind carries
    /// `None`.
    GatewayBacked {
        original_operation_id: OperationId,
        session_phase: Option<SessionPhase>,
        remote: RemoteDispatchOutcome,
    },
}

/// Why a [`WorkExecutor::dispatch`] call failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkExecutorError {
    /// The request carried no target workload (fail-closed: every dispatch
    /// must address a workload).
    MissingWorkload,
    /// The realm entrypoint table could not resolve a dispatch decision.
    Resolve(ResolveError),
    /// The operation body did not decode to the request shape its kind
    /// requires.
    MalformedBody,
    /// The operation kind has no host-resident durable-execution handling
    /// (the durable execution table only understands the exec family).
    UnsupportedHostResidentOperation(OperationKind),
    /// The host-resident durable execution table rejected the request.
    Durable(ConstellationError),
    /// The injected gateway port rejected the request.
    Remote(RemoteNodeError),
    /// The bounded gateway session table is at capacity. Reserved/checked
    /// *before* the gateway port is ever called, so the peer is never
    /// reached for a request this bound refuses.
    GatewaySessionCapacityExceeded,
    /// A host-resident request addressed a node other than this executor's
    /// own local node identity.
    WrongNode,
    /// An exec-family operation's body/envelope workload did not match the
    /// workload already recorded for that execution (or the envelope's own
    /// declared workload, for `ExecStart`).
    WorkloadMismatch,
    /// This executor's own authorization/idempotency router refused the
    /// host-resident request (capability denial, principal mismatch,
    /// missing/conflicting/expired idempotency key, or dedup capacity).
    Router(ConstellationError),
    /// A same-key host-resident mutation is still in progress at the host;
    /// the caller must retry later rather than treat this as a failure.
    /// Carries the [`OperationId`] of the attempt that actually owns the
    /// in-progress dedup lease -- the caller's own retry `operation_id` may
    /// differ and must never be used for a subsequent status query.
    HostOperationInProgress { original_operation_id: OperationId },
}

impl std::fmt::Display for WorkExecutorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WorkExecutorError::MissingWorkload => {
                write!(f, "operation is missing its target workload")
            }
            WorkExecutorError::Resolve(err) => write!(f, "realm entrypoint resolution: {err}"),
            WorkExecutorError::MalformedBody => write!(
                f,
                "operation body did not decode to the shape its kind requires"
            ),
            WorkExecutorError::UnsupportedHostResidentOperation(kind) => write!(
                f,
                "operation kind `{}` has no host-resident durable-execution handling",
                kind.code()
            ),
            WorkExecutorError::Durable(err) => write!(f, "durable execution table: {err}"),
            WorkExecutorError::Remote(err) => write!(f, "gateway port dispatch: {err}"),
            WorkExecutorError::GatewaySessionCapacityExceeded => {
                write!(f, "gateway session table is at capacity")
            }
            WorkExecutorError::WrongNode => {
                write!(f, "operation addressed a node other than this host")
            }
            WorkExecutorError::WorkloadMismatch => write!(
                f,
                "operation body/envelope workload did not match the execution's recorded workload"
            ),
            WorkExecutorError::Router(err) => write!(f, "host authorization router: {err}"),
            WorkExecutorError::HostOperationInProgress {
                original_operation_id,
            } => write!(
                f,
                "a same-key host-resident operation is still in progress under operation id `{}`",
                original_operation_id.as_str()
            ),
        }
    }
}

impl std::error::Error for WorkExecutorError {}

fn realm_target_from_request(req: &OperationRequest) -> Result<RealmTarget, WorkExecutorError> {
    let workload = req
        .workload
        .clone()
        .ok_or(WorkExecutorError::MissingWorkload)?;
    Ok(RealmTarget::new(workload, req.realm.clone()))
}

fn decode_body<T: serde::de::DeserializeOwned>(
    req: &OperationRequest,
) -> Result<T, WorkExecutorError> {
    serde_json::from_slice(req.body.as_bytes()).map_err(|_| WorkExecutorError::MalformedBody)
}

fn encode_host_resident_outcome(
    outcome: &HostResidentOutcome,
) -> Result<OpaquePayload, WorkExecutorError> {
    let bytes = serde_json::to_vec(outcome).map_err(|_| WorkExecutorError::MalformedBody)?;
    OpaquePayload::new(bytes).map_err(|_| WorkExecutorError::MalformedBody)
}

fn decode_host_resident_outcome(
    payload: &OpaquePayload,
) -> Result<HostResidentOutcome, WorkExecutorError> {
    serde_json::from_slice(payload.as_bytes()).map_err(|_| WorkExecutorError::MalformedBody)
}

/// Convert a host-router refusal for `req` into a [`WorkExecutorError`].
/// Every non-`Accept`/`Replay`/`InProgress` decision maps to `Some` from
/// [`route_decision_error`]; the fallback only guards a future decision
/// variant this module has not been updated for.
fn host_router_error(req: &OperationRequest, decision: &RouteDecision) -> WorkExecutorError {
    match route_decision_error(req, decision) {
        Some(err) => WorkExecutorError::Router(err),
        None => WorkExecutorError::UnsupportedHostResidentOperation(req.kind),
    }
}

/// The one typed dispatch surface tying realm resolution, host-resident
/// authorization + durable execution, and gateway session lifecycle
/// tracking together. See the module documentation for the dispatch
/// contract.
pub struct WorkExecutor<C = SystemClock>
where
    C: Clock,
{
    entrypoints: RealmEntrypointTable,
    durable: crate::execution::DurableExecTable,
    router: OperationRouter<C>,
    local_node: NodeId,
    session_principal: PrincipalId,
    capabilities: CapabilitySet,
    sessions: HashMap<OperationId, SessionLifecycle>,
    max_gateway_sessions: usize,
}

impl WorkExecutor<SystemClock> {
    /// Build with default-bounded tables and the default clock.
    ///
    /// `local_node` is the node identity this host answers to for
    /// host-resident dispatch (a host-resident request addressed to any
    /// other node fails closed with [`WorkExecutorError::WrongNode`]).
    /// `session_principal`/`capabilities` are the negotiated identity this
    /// executor authorizes host-resident requests against, mirroring
    /// [`crate::remote_node::RemoteFullHostAdapter::new`]'s own
    /// `gateway_principal`/`negotiated_capabilities` construction.
    pub fn new(
        local_node: NodeId,
        session_principal: PrincipalId,
        capabilities: CapabilitySet,
    ) -> Self {
        Self::with_parts(
            RealmEntrypointTable::new(),
            crate::execution::DurableExecTable::new(),
            OperationRouter::new(),
            local_node,
            session_principal,
            capabilities,
        )
    }
}

impl<C> WorkExecutor<C>
where
    C: Clock,
{
    /// Build from already-constructed parts (tests, or a host wiring
    /// injecting a manual clock / pre-populated entrypoint table).
    pub fn with_parts(
        entrypoints: RealmEntrypointTable,
        durable: crate::execution::DurableExecTable,
        router: OperationRouter<C>,
        local_node: NodeId,
        session_principal: PrincipalId,
        capabilities: CapabilitySet,
    ) -> Self {
        Self {
            entrypoints,
            durable,
            router,
            local_node,
            session_principal,
            capabilities,
            sessions: HashMap::new(),
            max_gateway_sessions: DEFAULT_MAX_GATEWAY_SESSIONS,
        }
    }

    /// Override the bounded gateway session-table capacity.
    pub fn with_max_gateway_sessions(mut self, max_gateway_sessions: usize) -> Self {
        self.max_gateway_sessions = max_gateway_sessions.max(1);
        self
    }

    /// Borrow the realm entrypoint table (e.g. to register realms before
    /// serving traffic).
    pub fn entrypoints_mut(&mut self) -> &mut RealmEntrypointTable {
        &mut self.entrypoints
    }

    /// Borrow the realm entrypoint table.
    ///
    /// Along with the other unused-in-this-crate's-own-tests accessors
    /// below (`durable_mut`, `router`, `router_mut`, `local_node`), this is
    /// real integrator-facing API for the production wiring documented in
    /// `docs/reference/realm-work-executor.md` (this module only compiles
    /// under this crate's own `#[cfg(test)]` today), not dead code.
    #[allow(dead_code)]
    pub fn entrypoints(&self) -> &RealmEntrypointTable {
        &self.entrypoints
    }

    /// Borrow the host-resident durable execution table.
    pub fn durable(&self) -> &crate::execution::DurableExecTable {
        &self.durable
    }

    /// Borrow the host-resident durable execution table mutably.
    #[allow(dead_code)]
    pub fn durable_mut(&mut self) -> &mut crate::execution::DurableExecTable {
        &mut self.durable
    }

    /// Borrow this executor's own host-resident authorization/idempotency
    /// router.
    #[allow(dead_code)]
    pub fn router(&self) -> &OperationRouter<C> {
        &self.router
    }

    /// Borrow this executor's own host-resident authorization/idempotency
    /// router mutably (e.g. to `gc()` on a maintenance tick).
    #[allow(dead_code)]
    pub fn router_mut(&mut self) -> &mut OperationRouter<C> {
        &mut self.router
    }

    /// This executor's own local node identity.
    #[allow(dead_code)]
    pub fn local_node(&self) -> &NodeId {
        &self.local_node
    }

    /// Number of gateway-backed sessions currently tracked.
    pub fn gateway_session_count(&self) -> usize {
        self.sessions.len()
    }

    /// The tracked session-lifecycle phase for `operation_id`, if any.
    pub fn gateway_session_phase(&self, operation_id: &OperationId) -> Option<SessionPhase> {
        self.sessions.get(operation_id).map(SessionLifecycle::phase)
    }

    /// Drive an orderly teardown of a tracked gateway session, returning its
    /// final phase. Idempotent: a second call after the session is already
    /// removed returns `None`. Once the session reaches `Stopped` it is
    /// evicted from the bounded map.
    pub fn stop_gateway_session(&mut self, operation_id: &OperationId) -> Option<SessionPhase> {
        let lifecycle = self.sessions.get_mut(operation_id)?;
        lifecycle.stop();
        lifecycle.finish_stop();
        let phase = lifecycle.phase();
        if lifecycle.is_stopped() {
            self.sessions.remove(operation_id);
        }
        Some(phase)
    }

    /// Dispatch one operation request. Resolves the realm entrypoint and
    /// routes host-resident vs gateway-backed per the module documentation.
    pub fn dispatch(
        &mut self,
        req: &OperationRequest,
        generation: &ProtocolToken,
        client: &mut dyn RemotePeerClient,
        gateway_port: &mut dyn GatewayPort,
    ) -> Result<WorkDispatchOutcome, WorkExecutorError> {
        let target = realm_target_from_request(req)?;
        match self
            .entrypoints
            .resolve(&target)
            .map_err(WorkExecutorError::Resolve)?
        {
            DispatchTarget::HostResident { .. } => {
                let (original_operation_id, outcome) = self.dispatch_host_resident(req)?;
                Ok(WorkDispatchOutcome::HostResident {
                    original_operation_id,
                    outcome,
                })
            }
            DispatchTarget::GatewayBacked { gateway, target } => self.dispatch_gateway_backed(
                &gateway,
                &target,
                req,
                generation,
                client,
                gateway_port,
            ),
        }
    }

    fn dispatch_host_resident(
        &mut self,
        req: &OperationRequest,
    ) -> Result<(OperationId, HostResidentOutcome), WorkExecutorError> {
        // Cheap, side-effect-free node validation first -- mirrors
        // `RemoteFullHostAdapter::dispatch`'s own "resolve/validate the
        // target before ever touching router/dedup state" ordering.
        if req.node != self.local_node {
            return Err(WorkExecutorError::WrongNode);
        }

        match self
            .router
            .route_with_capabilities(req, &self.session_principal, &self.capabilities)
        {
            RouteDecision::Accept { .. } => {
                let outcome = self.perform_host_resident(req);
                if req.kind.is_mutating() {
                    match &outcome {
                        Ok(value) => match encode_host_resident_outcome(value) {
                            Ok(payload) => {
                                self.router.mark_completed(req, payload);
                            }
                            Err(_) => {
                                self.router.mark_failed(req);
                            }
                        },
                        Err(_) => {
                            self.router.mark_failed(req);
                        }
                    }
                }
                // This attempt's own `operation_id` established the dedup
                // lease (a fresh `Accept`), so it *is* the original id.
                outcome.map(|value| (req.operation_id.clone(), value))
            }
            RouteDecision::Replay {
                original_operation_id,
                result,
            } => decode_host_resident_outcome(&result).map(|value| (original_operation_id, value)),
            RouteDecision::InProgress {
                original_operation_id,
            } => Err(WorkExecutorError::HostOperationInProgress {
                original_operation_id,
            }),
            decision => Err(host_router_error(req, &decision)),
        }
    }

    /// The exec-family body's own workload (`ExecStart`) or the workload
    /// already recorded for an existing execution (`ExecAttach`/`ExecLogs`/
    /// `ExecCancel`) must equal the envelope's `req.workload`. No existing
    /// record is not itself a mismatch -- the table's own not-found error
    /// surfaces naturally once the table call is attempted.
    fn check_execution_workload(
        &self,
        req: &OperationRequest,
        execution_id: &ExecutionId,
    ) -> Result<(), WorkExecutorError> {
        if let Some(existing) = self.durable.summary(execution_id)
            && Some(&existing.workload) != req.workload.as_ref()
        {
            return Err(WorkExecutorError::WorkloadMismatch);
        }
        Ok(())
    }

    fn perform_host_resident(
        &mut self,
        req: &OperationRequest,
    ) -> Result<HostResidentOutcome, WorkExecutorError> {
        match req.kind {
            OperationKind::ExecStart => {
                let decoded: ExecStartRequest = decode_body(req)?;
                if Some(&decoded.workload) != req.workload.as_ref() {
                    return Err(WorkExecutorError::WorkloadMismatch);
                }
                let summary = self
                    .durable
                    .start(decoded)
                    .map_err(WorkExecutorError::Durable)?;
                Ok(HostResidentOutcome::Started(summary))
            }
            OperationKind::ExecAttach => {
                let decoded: ExecAttachRequest = decode_body(req)?;
                self.check_execution_workload(req, &decoded.execution_id)?;
                let summary = self
                    .durable
                    .attach(&decoded)
                    .map_err(WorkExecutorError::Durable)?;
                Ok(HostResidentOutcome::Attached(summary))
            }
            OperationKind::ExecLogs => {
                let decoded: ExecLogsRequest = decode_body(req)?;
                self.check_execution_workload(req, &decoded.execution_id)?;
                let summary = self
                    .durable
                    .logs(&decoded)
                    .map_err(WorkExecutorError::Durable)?;
                Ok(HostResidentOutcome::Logs(summary))
            }
            OperationKind::ExecCancel => {
                let decoded: ExecCancelRequest = decode_body(req)?;
                self.check_execution_workload(req, &decoded.execution_id)?;
                let cancelled = self
                    .durable
                    .cancel(&decoded)
                    .map_err(WorkExecutorError::Durable)?;
                Ok(HostResidentOutcome::Cancelled(cancelled))
            }
            other => Err(WorkExecutorError::UnsupportedHostResidentOperation(other)),
        }
    }

    fn dispatch_gateway_backed(
        &mut self,
        gateway: &RealmTarget,
        _target: &RealmTarget,
        req: &OperationRequest,
        generation: &ProtocolToken,
        client: &mut dyn RemotePeerClient,
        gateway_port: &mut dyn GatewayPort,
    ) -> Result<WorkDispatchOutcome, WorkExecutorError> {
        let tracks_session = req.kind == OperationKind::DisplaySessionOpen;

        // Classify BEFORE ever checking/reserving local bounded capacity.
        // Classification is a pure peek (see `GatewayPort::classify_dispatch`):
        // it never mutates gateway dedup/session state and never contacts
        // the remote peer, so a capacity refusal computed from it is always
        // provable as "no remote side effect occurred". A retry that will
        // resolve against an ALREADY-tracked session (a fresh
        // `req.operation_id` reusing the same idempotency key) must never
        // be rejected by a capacity bound that was never actually needed --
        // only a genuinely new, side-effecting session consumes a
        // reservation.
        let classification = if tracks_session {
            Some(gateway_port.classify_dispatch(gateway, req))
        } else {
            None
        };

        // Reserve bounded capacity *before* the gateway port is ever
        // called, keyed provisionally by this attempt's own
        // `req.operation_id` -- but only for a classification that resolves
        // to a genuinely new lease. An at-capacity refusal here must be
        // provable as "the peer was never contacted for this request", not
        // merely "the outcome was not tracked". The reservation is rolled
        // back (this exact entry only, see below) if the attempt fails,
        // turns out to be a replay of a different pre-existing session, or
        // resolves ambiguously. `reserved_new_slot` remembers whether this
        // call itself created the entry -- never true for an id that was
        // already tracked before this call, and never true for a
        // classification that already resolved to an existing session --
        // so a failed/conflicting retry can never evict a running session
        // it does not own.
        let needs_new_reservation = tracks_session
            && !matches!(
                classification,
                Some(GatewayDispatchClassification::Existing { .. })
            )
            && !self.sessions.contains_key(&req.operation_id);
        let reserved_new_slot = if !needs_new_reservation {
            false
        } else if self.sessions.len() >= self.max_gateway_sessions {
            return Err(WorkExecutorError::GatewaySessionCapacityExceeded);
        } else {
            self.sessions
                .insert(req.operation_id.clone(), SessionLifecycle::new());
            true
        };

        let result = gateway_port.dispatch_via_gateway(gateway, req, generation, client);

        let session_phase = if tracks_session {
            match &result {
                Ok(outcome) => match outcome {
                    RemoteDispatchOutcome::Sent { .. } | RemoteDispatchOutcome::Replayed { .. } => {
                        let original_id = outcome.original_operation_id(&req.operation_id).clone();
                        if original_id != req.operation_id && reserved_new_slot {
                            // This attempt's own provisional reservation
                            // turned out to be unnecessary -- the gateway
                            // resolved it as a replay of a different,
                            // already-canonical id. Release only the
                            // reservation this call itself just created,
                            // never any pre-existing entry.
                            self.sessions.remove(&req.operation_id);
                        }
                        if self.sessions.contains_key(&original_id) {
                            let lifecycle = self
                                .sessions
                                .get_mut(&original_id)
                                .expect("just checked contains_key");
                            while lifecycle.advance().is_ok() {}
                            Some(lifecycle.phase())
                        } else if self.sessions.len() < self.max_gateway_sessions {
                            let lifecycle = self.sessions.entry(original_id.clone()).or_default();
                            while lifecycle.advance().is_ok() {}
                            Some(lifecycle.phase())
                        } else {
                            // The gateway dispatch already succeeded (the
                            // peer was legitimately contacted or the dedup
                            // lease already existed there); a purely local
                            // tracking gap under bounded capacity must not
                            // be reported as an error for an operation that
                            // already succeeded remotely.
                            None
                        }
                    }
                    RemoteDispatchOutcome::QueryRemoteState { .. } => {
                        let original_id = outcome.original_operation_id(&req.operation_id).clone();
                        // Ambiguous outcome: never finalize new bounded
                        // state for it. Release only this attempt's own
                        // provisional reservation; report an
                        // already-tracked session's phase, if any, without
                        // advancing it.
                        if reserved_new_slot {
                            self.sessions.remove(&req.operation_id);
                        }
                        self.sessions.get(&original_id).map(SessionLifecycle::phase)
                    }
                },
                Err(_) => {
                    // Release ONLY the reservation this exact call created.
                    // A pre-existing entry for `req.operation_id` (e.g. a
                    // conflicting retry of an already-running session) is
                    // never touched here -- only an explicit stop/teardown
                    // removes it.
                    if reserved_new_slot {
                        self.sessions.remove(&req.operation_id);
                    }
                    None
                }
            }
        } else {
            None
        };

        let remote = result.map_err(WorkExecutorError::Remote)?;
        // Every successful outcome exposes its dedup-lease owner id --
        // never a retry's own `req.operation_id` -- regardless of whether
        // this operation kind tracks local session-lifecycle state.
        let original_operation_id = remote.original_operation_id(&req.operation_id).clone();
        Ok(WorkDispatchOutcome::GatewayBacked {
            original_operation_id,
            session_phase,
            remote,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_realm_core::{
        Capability, CapabilitySet, CorrelationId, ExecAttachMode, ExecutionGeneration, ExecutionId,
        IdempotencyKey, NodeId, NodeKind, NodeSummary, OpaquePayload, PrincipalId, ProviderId,
        RealmId, RealmPath, WorkloadId,
    };

    use crate::remote_node::{
        RemoteFullHostAdapter, RemoteNodeError, RemoteNodeErrorKind, RemoteNodeRegistration,
        RemotePeerStatus, RemoteRoute, SingleGatewayPort,
    };

    /// The gateway-backed test realm (`work`).
    fn work_realm() -> RealmPath {
        RealmPath::new(vec![RealmId::parse("work").unwrap()]).unwrap()
    }

    fn local_node() -> NodeId {
        NodeId::parse("this-host").unwrap()
    }

    fn principal() -> PrincipalId {
        PrincipalId::parse("gateway-principal").unwrap()
    }

    fn gateway_node() -> NodeId {
        NodeId::parse("gateway").unwrap()
    }

    fn gateway_target() -> RealmTarget {
        RealmTarget::new(WorkloadId::parse("gateway-vm").unwrap(), RealmPath::local())
    }

    fn remote_registration(generation: &str, caps: CapabilitySet) -> RemoteNodeRegistration {
        RemoteNodeRegistration {
            summary: NodeSummary {
                id: NodeId::parse("remote-host").unwrap(),
                realm: work_realm(),
                kind: NodeKind::FullHost,
                capabilities: caps,
            },
            gateway_principal: principal(),
            gateway_node: gateway_node(),
            substrate_adapter: ProviderId::parse("nixos-host-substrate").unwrap(),
            generation: ProtocolToken::parse(generation).unwrap(),
        }
    }

    fn exec_start_request(
        kind: OperationKind,
        execution_id: &str,
        key: Option<&str>,
    ) -> OperationRequest {
        let start = ExecStartRequest {
            execution_id: ExecutionId::parse(execution_id).unwrap(),
            workload: WorkloadId::parse("vm-a").unwrap(),
            generation: ExecutionGeneration {
                guest_boot_id: ProtocolToken::parse("boot-1").unwrap(),
                workload_generation: ProtocolToken::parse("workload-gen-1").unwrap(),
            },
            attach_mode: ExecAttachMode::Detached,
            tty: false,
        };
        OperationRequest {
            operation_id: OperationId::parse("op-1").unwrap(),
            correlation_id: CorrelationId::parse("corr-1").unwrap(),
            idempotency_key: key.map(|raw| IdempotencyKey::parse(raw).unwrap()),
            realm: RealmPath::local(),
            node: local_node(),
            workload: Some(WorkloadId::parse("vm-a").unwrap()),
            principal: principal(),
            kind,
            trace: None,
            body: OpaquePayload::new(serde_json::to_vec(&start).unwrap()).unwrap(),
        }
    }

    fn display_open_request(realm: RealmPath, key: Option<&str>) -> OperationRequest {
        OperationRequest {
            operation_id: OperationId::parse("op-display-1").unwrap(),
            correlation_id: CorrelationId::parse("corr-1").unwrap(),
            idempotency_key: key.map(|raw| IdempotencyKey::parse(raw).unwrap()),
            realm,
            node: NodeId::parse("remote-host").unwrap(),
            workload: Some(WorkloadId::parse("vm-a").unwrap()),
            principal: principal(),
            kind: OperationKind::DisplaySessionOpen,
            trace: None,
            body: OpaquePayload::new(Capability::WindowForwarding.code().as_bytes().to_vec())
                .unwrap(),
        }
    }

    #[derive(Default)]
    struct FakePeer {
        sends: usize,
        fail_send: bool,
    }

    impl RemotePeerClient for FakePeer {
        fn send_operation(
            &mut self,
            _route: &RemoteRoute,
            _req: &OperationRequest,
        ) -> Result<OpaquePayload, RemoteNodeError> {
            self.sends += 1;
            if self.fail_send {
                return Err(RemoteNodeError {
                    kind: RemoteNodeErrorKind::NodeUnavailable,
                    missing_capability: None,
                    capability_fingerprint: None,
                    correlation_id: None,
                });
            }
            Ok(OpaquePayload::new(b"remote-ok".to_vec()).unwrap())
        }

        fn query_operation(
            &mut self,
            _route: &RemoteRoute,
            _req: &OperationRequest,
        ) -> Result<RemotePeerStatus, RemoteNodeError> {
            Ok(RemotePeerStatus::InProgress)
        }
    }

    /// A `WorkExecutor` for host-resident tests, authorized with
    /// `capabilities` for `session_principal = principal()`.
    fn host_resident_executor(capabilities: CapabilitySet) -> WorkExecutor<SystemClock> {
        let mut executor = WorkExecutor::new(local_node(), principal(), capabilities);
        executor.entrypoints_mut().host_resident(RealmPath::local());
        executor
    }

    /// A fully-authorized host-resident executor (the common case for tests
    /// that are not specifically exercising the authorization gate).
    fn authorized_host_resident_executor() -> WorkExecutor<SystemClock> {
        host_resident_executor(
            CapabilitySet::empty()
                .with(Capability::Exec)
                .with(Capability::Logs),
        )
    }

    fn gateway_backed_executor(
        caps: CapabilitySet,
    ) -> (WorkExecutor<SystemClock>, SingleGatewayPort<SystemClock>) {
        let gateway_realm = work_realm();
        let boundary = gateway_target();
        let adapter = RemoteFullHostAdapter::with_router(
            crate::remote_node::RemoteNodeRegistry::for_realm(gateway_realm.clone()),
            crate::OperationRouter::new(),
            principal(),
            caps.clone(),
        );
        let mut port = SingleGatewayPort::new(boundary.clone(), adapter);
        port.adapter_mut()
            .registry_mut()
            .register(
                remote_registration("gen-1", caps),
                std::time::Instant::now(),
            )
            .unwrap();

        // Gateway-backed dispatch is authorized entirely by the injected
        // `GatewayPort` (here, the gateway-side adapter's own embedded
        // router, seeded with `caps` above) -- `WorkExecutor`'s own
        // `session_principal`/`capabilities` only gate the host-resident
        // path, so an empty set here is deliberate and does not affect
        // these gateway-backed tests.
        let mut executor = WorkExecutor::new(local_node(), principal(), CapabilitySet::empty());
        executor
            .entrypoints_mut()
            .gateway_backed(gateway_realm, boundary);
        (executor, port)
    }

    #[test]
    fn host_resident_exec_start_reaches_the_durable_table() {
        let mut executor = authorized_host_resident_executor();
        let req = exec_start_request(OperationKind::ExecStart, "exec-1", Some("idem-1"));
        let mut peer = FakePeer::default();
        let mut port = NoopGatewayPort;
        let outcome = executor
            .dispatch(
                &req,
                &ProtocolToken::parse("boot-1").unwrap(),
                &mut peer,
                &mut port,
            )
            .unwrap();
        match outcome {
            WorkDispatchOutcome::HostResident {
                original_operation_id,
                outcome: HostResidentOutcome::Started(summary),
            } => {
                assert_eq!(summary.id, ExecutionId::parse("exec-1").unwrap());
                assert_eq!(original_operation_id, req.operation_id);
            }
            other => panic!("expected host-resident start outcome, got {other:?}"),
        }
        assert_eq!(executor.durable().len(), 1);
        assert_eq!(
            peer.sends, 0,
            "host-resident dispatch must never call the remote peer"
        );
    }

    #[test]
    fn missing_workload_fails_closed() {
        let mut executor = authorized_host_resident_executor();
        let mut req = exec_start_request(OperationKind::ExecStart, "exec-1", Some("idem-1"));
        req.workload = None;
        let mut peer = FakePeer::default();
        let mut port = NoopGatewayPort;
        let err = executor
            .dispatch(
                &req,
                &ProtocolToken::parse("boot-1").unwrap(),
                &mut peer,
                &mut port,
            )
            .unwrap_err();
        assert_eq!(err, WorkExecutorError::MissingWorkload);
    }

    #[test]
    fn unresolved_realm_fails_closed() {
        let mut executor = WorkExecutor::new(local_node(), principal(), CapabilitySet::empty());
        let req = exec_start_request(OperationKind::ExecStart, "exec-1", Some("idem-1"));
        let mut peer = FakePeer::default();
        let mut port = NoopGatewayPort;
        let err = executor
            .dispatch(
                &req,
                &ProtocolToken::parse("boot-1").unwrap(),
                &mut peer,
                &mut port,
            )
            .unwrap_err();
        assert!(matches!(err, WorkExecutorError::Resolve(_)));
    }

    #[test]
    fn wrong_node_fails_closed_before_touching_the_durable_table() {
        let mut executor = authorized_host_resident_executor();
        let mut req = exec_start_request(OperationKind::ExecStart, "exec-1", Some("idem-1"));
        req.node = NodeId::parse("some-other-host").unwrap();
        let mut peer = FakePeer::default();
        let mut port = NoopGatewayPort;
        let err = executor
            .dispatch(
                &req,
                &ProtocolToken::parse("boot-1").unwrap(),
                &mut peer,
                &mut port,
            )
            .unwrap_err();
        assert_eq!(err, WorkExecutorError::WrongNode);
        assert!(executor.durable().is_empty());
    }

    #[test]
    fn empty_capabilities_reject_a_mutating_host_resident_operation() {
        // ExecStart is mutating and requires `Capability::Exec`; an executor
        // authorized with no capabilities must refuse it before the durable
        // table is ever touched (finding: reject empty capabilities for
        // mutations).
        let mut executor = host_resident_executor(CapabilitySet::empty());
        let req = exec_start_request(OperationKind::ExecStart, "exec-1", Some("idem-1"));
        let mut peer = FakePeer::default();
        let mut port = NoopGatewayPort;
        let err = executor
            .dispatch(
                &req,
                &ProtocolToken::parse("boot-1").unwrap(),
                &mut peer,
                &mut port,
            )
            .unwrap_err();
        assert!(
            matches!(err, WorkExecutorError::Router(_)),
            "expected a router capability refusal, got {err:?}"
        );
        assert!(executor.durable().is_empty());
    }

    #[test]
    fn host_operation_in_progress_error_exposes_the_original_operation_id() {
        // `RouteDecision::InProgress` carries the id of the attempt that
        // actually owns the in-progress dedup lease -- which can genuinely
        // differ from a later concurrent caller's own retry id (dedup keys
        // deliberately exclude `operation_id`; see
        // `d2b_realm_core::OperationRequest::dedup_fingerprint_input`).
        // `WorkExecutor` must plumb it through unchanged rather than
        // substituting the caller's own operation id.
        let original = OperationId::parse("op-original").unwrap();
        let err = WorkExecutorError::HostOperationInProgress {
            original_operation_id: original.clone(),
        };
        assert_eq!(
            err.to_string(),
            format!(
                "a same-key host-resident operation is still in progress under operation id `{}`",
                original.as_str()
            )
        );
        match err {
            WorkExecutorError::HostOperationInProgress {
                original_operation_id,
            } => {
                assert_eq!(original_operation_id, original);
            }
            other => panic!("expected HostOperationInProgress, got {other:?}"),
        }
    }

    #[test]
    fn missing_idempotency_key_rejects_a_mutating_host_resident_operation() {
        // ExecStart is mutating; an idempotency key is mandatory even when
        // fully authorized by capability (finding: reject missing
        // idempotency for mutations).
        let mut executor = authorized_host_resident_executor();
        let req = exec_start_request(OperationKind::ExecStart, "exec-1", None);
        let mut peer = FakePeer::default();
        let mut port = NoopGatewayPort;
        let err = executor
            .dispatch(
                &req,
                &ProtocolToken::parse("boot-1").unwrap(),
                &mut peer,
                &mut port,
            )
            .unwrap_err();
        assert!(
            matches!(err, WorkExecutorError::Router(_)),
            "expected a router idempotency refusal, got {err:?}"
        );
        assert!(executor.durable().is_empty());
    }

    #[test]
    fn non_mutating_exec_logs_does_not_require_an_idempotency_key() {
        let mut executor = authorized_host_resident_executor();
        let start_req = exec_start_request(OperationKind::ExecStart, "exec-1", Some("idem-1"));
        let mut peer = FakePeer::default();
        let mut port = NoopGatewayPort;
        executor
            .dispatch(
                &start_req,
                &ProtocolToken::parse("boot-1").unwrap(),
                &mut peer,
                &mut port,
            )
            .unwrap();

        let logs = ExecLogsRequest {
            execution_id: ExecutionId::parse("exec-1").unwrap(),
            generation: ExecutionGeneration {
                guest_boot_id: ProtocolToken::parse("boot-1").unwrap(),
                workload_generation: ProtocolToken::parse("workload-gen-1").unwrap(),
            },
            cursor: None,
            max_bytes: std::num::NonZero::new(4096).unwrap(),
        };
        let mut req = start_req.clone();
        req.kind = OperationKind::ExecLogs;
        req.idempotency_key = None;
        req.body = OpaquePayload::new(serde_json::to_vec(&logs).unwrap()).unwrap();

        let outcome = executor
            .dispatch(
                &req,
                &ProtocolToken::parse("boot-1").unwrap(),
                &mut peer,
                &mut port,
            )
            .unwrap();
        assert!(matches!(
            outcome,
            WorkDispatchOutcome::HostResident {
                outcome: HostResidentOutcome::Logs(_),
                ..
            }
        ));
    }

    #[test]
    fn duplicate_idempotency_key_replays_instead_of_restarting() {
        let mut executor = authorized_host_resident_executor();
        let req = exec_start_request(OperationKind::ExecStart, "exec-1", Some("idem-1"));
        let mut peer = FakePeer::default();
        let mut port = NoopGatewayPort;
        let first = executor
            .dispatch(
                &req,
                &ProtocolToken::parse("boot-1").unwrap(),
                &mut peer,
                &mut port,
            )
            .unwrap();
        let second = executor
            .dispatch(
                &req,
                &ProtocolToken::parse("boot-1").unwrap(),
                &mut peer,
                &mut port,
            )
            .unwrap();
        assert_eq!(first, second);
        assert_eq!(
            executor.durable().len(),
            1,
            "a replayed retry must never re-run the durable start"
        );
    }

    #[test]
    fn malformed_body_is_rejected_before_touching_the_durable_table() {
        let mut executor = authorized_host_resident_executor();
        let mut req = exec_start_request(OperationKind::ExecStart, "exec-1", Some("idem-1"));
        req.body = OpaquePayload::new(b"not-json".to_vec()).unwrap();
        let mut peer = FakePeer::default();
        let mut port = NoopGatewayPort;
        let err = executor
            .dispatch(
                &req,
                &ProtocolToken::parse("boot-1").unwrap(),
                &mut peer,
                &mut port,
            )
            .unwrap_err();
        assert_eq!(err, WorkExecutorError::MalformedBody);
        assert!(executor.durable().is_empty());
    }

    #[test]
    fn body_workload_mismatch_is_rejected_before_touching_the_durable_table() {
        let mut executor = authorized_host_resident_executor();
        let mut req = exec_start_request(OperationKind::ExecStart, "exec-1", Some("idem-1"));
        // The envelope addresses `vm-a`, but rewrite the body to start a
        // different workload -- the envelope/body workload must agree.
        let mismatched = ExecStartRequest {
            execution_id: ExecutionId::parse("exec-1").unwrap(),
            workload: WorkloadId::parse("vm-b").unwrap(),
            generation: ExecutionGeneration {
                guest_boot_id: ProtocolToken::parse("boot-1").unwrap(),
                workload_generation: ProtocolToken::parse("workload-gen-1").unwrap(),
            },
            attach_mode: ExecAttachMode::Detached,
            tty: false,
        };
        req.body = OpaquePayload::new(serde_json::to_vec(&mismatched).unwrap()).unwrap();
        let mut peer = FakePeer::default();
        let mut port = NoopGatewayPort;
        let err = executor
            .dispatch(
                &req,
                &ProtocolToken::parse("boot-1").unwrap(),
                &mut peer,
                &mut port,
            )
            .unwrap_err();
        assert_eq!(err, WorkExecutorError::WorkloadMismatch);
        assert!(executor.durable().is_empty());
    }

    #[test]
    fn existing_execution_workload_mismatch_is_rejected_for_attach() {
        let mut executor = authorized_host_resident_executor();
        let start_req = exec_start_request(OperationKind::ExecStart, "exec-1", Some("idem-1"));
        let mut peer = FakePeer::default();
        let mut port = NoopGatewayPort;
        executor
            .dispatch(
                &start_req,
                &ProtocolToken::parse("boot-1").unwrap(),
                &mut peer,
                &mut port,
            )
            .unwrap();

        let attach = ExecAttachRequest {
            execution_id: ExecutionId::parse("exec-1").unwrap(),
            generation: ExecutionGeneration {
                guest_boot_id: ProtocolToken::parse("boot-1").unwrap(),
                workload_generation: ProtocolToken::parse("workload-gen-1").unwrap(),
            },
            stdout_cursor: None,
            stderr_cursor: None,
        };
        let mut req = start_req.clone();
        req.kind = OperationKind::ExecAttach;
        req.idempotency_key = Some(IdempotencyKey::parse("idem-2").unwrap());
        // Envelope now claims a different workload than the one recorded at
        // start time.
        req.workload = Some(WorkloadId::parse("vm-b").unwrap());
        req.body = OpaquePayload::new(serde_json::to_vec(&attach).unwrap()).unwrap();

        let err = executor
            .dispatch(
                &req,
                &ProtocolToken::parse("boot-1").unwrap(),
                &mut peer,
                &mut port,
            )
            .unwrap_err();
        assert_eq!(err, WorkExecutorError::WorkloadMismatch);
    }

    #[test]
    fn unsupported_host_resident_kind_is_rejected() {
        let mut executor = authorized_host_resident_executor();
        let mut req = exec_start_request(OperationKind::ExecStart, "exec-1", Some("idem-1"));
        req.kind = OperationKind::WorkloadStart;
        let mut peer = FakePeer::default();
        let mut port = NoopGatewayPort;
        let err = executor
            .dispatch(
                &req,
                &ProtocolToken::parse("boot-1").unwrap(),
                &mut peer,
                &mut port,
            )
            .unwrap_err();
        // `WorkloadStart` is a supported, capability-gated, mutating kind;
        // without a `Capability::Lifecycle` grant the router refuses it
        // before the durable table's own "unsupported kind" branch would
        // ever run.
        assert!(matches!(err, WorkExecutorError::Router(_)));
    }

    #[test]
    fn gateway_backed_display_session_advances_to_running_on_success() {
        let caps = CapabilitySet::empty().with(Capability::WindowForwarding);
        let (mut executor, mut port) = gateway_backed_executor(caps);
        let realm = work_realm();
        let req = display_open_request(realm, Some("idem-1"));
        let mut peer = FakePeer::default();

        let outcome = executor
            .dispatch(
                &req,
                &ProtocolToken::parse("gen-1").unwrap(),
                &mut peer,
                &mut port,
            )
            .unwrap();
        match outcome {
            WorkDispatchOutcome::GatewayBacked {
                original_operation_id,
                session_phase,
                remote,
            } => {
                assert_eq!(session_phase, Some(SessionPhase::Running));
                assert!(matches!(remote, RemoteDispatchOutcome::Sent { .. }));
                assert_eq!(original_operation_id, req.operation_id);
            }
            other => panic!("expected gateway-backed outcome, got {other:?}"),
        }
        assert_eq!(peer.sends, 1);
        assert_eq!(executor.gateway_session_count(), 1);
        assert_eq!(
            executor.gateway_session_phase(&req.operation_id),
            Some(SessionPhase::Running)
        );
    }

    #[test]
    fn gateway_backed_display_session_never_allocates_lifecycle_on_remote_error() {
        let caps = CapabilitySet::empty().with(Capability::WindowForwarding);
        let (mut executor, mut port) = gateway_backed_executor(caps);
        let realm = work_realm();
        let req = display_open_request(realm, Some("idem-1"));
        let mut peer = FakePeer {
            fail_send: true,
            ..Default::default()
        };

        let err = executor
            .dispatch(
                &req,
                &ProtocolToken::parse("gen-1").unwrap(),
                &mut peer,
                &mut port,
            )
            .unwrap_err();
        assert!(matches!(err, WorkExecutorError::Remote(_)));
        // Lifecycle state is only allocated after a successful
        // auth/preflight-gated dispatch outcome: a failed attempt must
        // never leave a lingering entry that counts against the bounded
        // capacity.
        assert_eq!(executor.gateway_session_phase(&req.operation_id), None);
        assert_eq!(executor.gateway_session_count(), 0);
    }

    #[test]
    fn rejected_gateway_attempts_cannot_exhaust_bounded_session_capacity() {
        // A gateway port that always refuses (e.g. the request never
        // clears the gateway's own auth/preflight) must never be able to
        // fill the bounded session table with entries for ids that never
        // actually succeeded.
        let caps = CapabilitySet::empty().with(Capability::WindowForwarding);
        let (mut executor, mut port) = gateway_backed_executor(caps);
        executor = executor.with_max_gateway_sessions(2);
        let realm = work_realm();
        let mut peer = FakePeer {
            fail_send: true,
            ..Default::default()
        };

        for i in 0..8 {
            let mut req = display_open_request(realm.clone(), Some("idem-1"));
            req.operation_id = OperationId::parse(format!("op-display-{i}")).unwrap();
            let err = executor
                .dispatch(
                    &req,
                    &ProtocolToken::parse("gen-1").unwrap(),
                    &mut peer,
                    &mut port,
                )
                .unwrap_err();
            assert!(matches!(err, WorkExecutorError::Remote(_)));
        }
        assert_eq!(
            executor.gateway_session_count(),
            0,
            "repeatedly-rejected attempts must never consume bounded capacity"
        );

        // A subsequent legitimately-succeeding attempt must still fit.
        peer.fail_send = false;
        let ok_req = display_open_request(realm, Some("idem-1"));
        let outcome = executor
            .dispatch(
                &ok_req,
                &ProtocolToken::parse("gen-1").unwrap(),
                &mut peer,
                &mut port,
            )
            .unwrap();
        assert!(matches!(outcome, WorkDispatchOutcome::GatewayBacked { .. }));
        assert_eq!(executor.gateway_session_count(), 1);
    }

    #[test]
    fn stop_gateway_session_evicts_once_stopped() {
        let caps = CapabilitySet::empty().with(Capability::WindowForwarding);
        let (mut executor, mut port) = gateway_backed_executor(caps);
        let realm = work_realm();
        let req = display_open_request(realm, Some("idem-1"));
        let mut peer = FakePeer::default();
        executor
            .dispatch(
                &req,
                &ProtocolToken::parse("gen-1").unwrap(),
                &mut peer,
                &mut port,
            )
            .unwrap();

        let phase = executor.stop_gateway_session(&req.operation_id).unwrap();
        assert_eq!(phase, SessionPhase::Stopped);
        assert_eq!(executor.gateway_session_count(), 0);
        // Idempotent: a second stop on an already-evicted session is `None`.
        assert_eq!(executor.stop_gateway_session(&req.operation_id), None);
    }

    #[test]
    fn non_display_gateway_operation_does_not_track_a_session() {
        let caps = CapabilitySet::empty().with(Capability::Lifecycle);
        let (mut executor, mut port) = gateway_backed_executor(caps);
        let realm = work_realm();
        let req = OperationRequest {
            operation_id: OperationId::parse("op-workload-1").unwrap(),
            correlation_id: CorrelationId::parse("corr-1").unwrap(),
            idempotency_key: Some(IdempotencyKey::parse("idem-2").unwrap()),
            realm,
            node: NodeId::parse("remote-host").unwrap(),
            workload: Some(WorkloadId::parse("vm-a").unwrap()),
            principal: principal(),
            kind: OperationKind::WorkloadStart,
            trace: None,
            body: OpaquePayload::new(Capability::Lifecycle.code().as_bytes().to_vec()).unwrap(),
        };
        let mut peer = FakePeer::default();
        let outcome = executor
            .dispatch(
                &req,
                &ProtocolToken::parse("gen-1").unwrap(),
                &mut peer,
                &mut port,
            )
            .unwrap();
        match outcome {
            WorkDispatchOutcome::GatewayBacked { session_phase, .. } => {
                assert_eq!(session_phase, None);
            }
            other => panic!("expected gateway-backed outcome, got {other:?}"),
        }
        assert_eq!(executor.gateway_session_count(), 0);
    }

    #[test]
    fn gateway_session_capacity_is_enforced_for_legitimate_sessions() {
        let caps = CapabilitySet::empty().with(Capability::WindowForwarding);
        let (mut executor, mut port) = gateway_backed_executor(caps);
        executor = executor.with_max_gateway_sessions(1);
        let realm = work_realm();
        let first = display_open_request(realm.clone(), Some("idem-1"));
        let mut peer = FakePeer::default();
        executor
            .dispatch(
                &first,
                &ProtocolToken::parse("gen-1").unwrap(),
                &mut peer,
                &mut port,
            )
            .unwrap();

        let mut second = display_open_request(realm, Some("idem-2"));
        second.operation_id = OperationId::parse("op-display-2").unwrap();
        let err = executor
            .dispatch(
                &second,
                &ProtocolToken::parse("gen-1").unwrap(),
                &mut peer,
                &mut port,
            )
            .unwrap_err();
        assert_eq!(err, WorkExecutorError::GatewaySessionCapacityExceeded);
        // The at-capacity refusal must be provable as "the peer was never
        // reached for this request": exactly the first request's send, and
        // none from the refused second attempt.
        assert_eq!(
            peer.sends, 1,
            "an at-capacity refusal must never reach the gateway peer"
        );
    }

    #[test]
    fn a_fresh_id_replay_of_an_existing_session_succeeds_even_at_capacity_one() {
        // The bounded session table is exhausted by one legitimate session.
        // A retry that carries a fresh `operation_id` but the SAME
        // idempotency key must resolve against that already-tracked
        // session -- it must never be rejected by the capacity bound,
        // because it never needs a NEW reservation in the first place.
        let caps = CapabilitySet::empty().with(Capability::WindowForwarding);
        let (mut executor, mut port) = gateway_backed_executor(caps);
        executor = executor.with_max_gateway_sessions(1);
        let realm = work_realm();
        let first = display_open_request(realm, Some("idem-1"));
        let mut peer = FakePeer::default();
        executor
            .dispatch(
                &first,
                &ProtocolToken::parse("gen-1").unwrap(),
                &mut peer,
                &mut port,
            )
            .unwrap();
        assert_eq!(executor.gateway_session_count(), 1);

        let mut retry = first.clone();
        retry.operation_id = OperationId::parse("op-display-retry").unwrap();
        let outcome = executor
            .dispatch(
                &retry,
                &ProtocolToken::parse("gen-1").unwrap(),
                &mut peer,
                &mut port,
            )
            .expect("a same-key replay must never be rejected by session capacity");
        match outcome {
            WorkDispatchOutcome::GatewayBacked {
                original_operation_id,
                session_phase,
                remote,
            } => {
                assert_eq!(original_operation_id, first.operation_id);
                assert_eq!(session_phase, Some(SessionPhase::Running));
                assert!(matches!(remote, RemoteDispatchOutcome::Replayed { .. }));
            }
            other => panic!("expected gateway-backed outcome, got {other:?}"),
        }
        // Still exactly one tracked session -- the retry's own id never
        // consumed a second slot.
        assert_eq!(executor.gateway_session_count(), 1);
        assert_eq!(
            peer.sends, 1,
            "a same-key replay must never re-send to the gateway peer"
        );
    }

    #[test]
    fn at_capacity_new_operation_is_rejected_with_no_gateway_side_effect() {
        // A genuinely new session (a distinct idempotency key) at capacity
        // must still be refused -- and, critically, must never reach the
        // gateway peer: classification distinguishing it from an existing
        // session's replay must never itself cause a remote side effect.
        let caps = CapabilitySet::empty().with(Capability::WindowForwarding);
        let (mut executor, mut port) = gateway_backed_executor(caps);
        executor = executor.with_max_gateway_sessions(1);
        let realm = work_realm();
        let first = display_open_request(realm.clone(), Some("idem-1"));
        let mut peer = FakePeer::default();
        executor
            .dispatch(
                &first,
                &ProtocolToken::parse("gen-1").unwrap(),
                &mut peer,
                &mut port,
            )
            .unwrap();
        assert_eq!(peer.sends, 1);

        let mut new_op = display_open_request(realm, Some("idem-brand-new"));
        new_op.operation_id = OperationId::parse("op-display-new").unwrap();
        let err = executor
            .dispatch(
                &new_op,
                &ProtocolToken::parse("gen-1").unwrap(),
                &mut peer,
                &mut port,
            )
            .unwrap_err();
        assert_eq!(err, WorkExecutorError::GatewaySessionCapacityExceeded);
        assert_eq!(
            peer.sends, 1,
            "a genuinely new operation refused for capacity must never reach the gateway peer"
        );
        assert_eq!(executor.gateway_session_count(), 1);
    }

    #[test]
    fn replay_under_a_different_operation_id_keys_the_session_by_the_original_id() {
        // A retry commonly carries a fresh per-attempt `operation_id` while
        // reusing the same idempotency key -- the dedup fingerprint
        // deliberately excludes `operation_id` (see
        // `d2b_realm_core::OperationRequest::dedup_fingerprint_input`), so
        // the gateway's own router resolves this as a same-key replay
        // carrying the *first* attempt's id, never the retry's own.
        let caps = CapabilitySet::empty().with(Capability::WindowForwarding);
        let (mut executor, mut port) = gateway_backed_executor(caps);
        let realm = work_realm();
        let first = display_open_request(realm, Some("idem-1"));
        let mut peer = FakePeer::default();
        let outcome = executor
            .dispatch(
                &first,
                &ProtocolToken::parse("gen-1").unwrap(),
                &mut peer,
                &mut port,
            )
            .unwrap();
        let original_id = match outcome {
            WorkDispatchOutcome::GatewayBacked {
                original_operation_id,
                ..
            } => original_operation_id,
            other => panic!("expected gateway-backed outcome, got {other:?}"),
        };
        assert_eq!(original_id, first.operation_id);

        let mut retry = first.clone();
        retry.operation_id = OperationId::parse("op-display-retry").unwrap();
        let outcome = executor
            .dispatch(
                &retry,
                &ProtocolToken::parse("gen-1").unwrap(),
                &mut peer,
                &mut port,
            )
            .unwrap();
        match outcome {
            WorkDispatchOutcome::GatewayBacked {
                original_operation_id,
                session_phase,
                remote,
            } => {
                assert_eq!(
                    original_operation_id, original_id,
                    "session lifecycle must be keyed/exposed by the original id, never the retry's own"
                );
                assert_eq!(session_phase, Some(SessionPhase::Running));
                assert!(matches!(remote, RemoteDispatchOutcome::Replayed { .. }));
            }
            other => panic!("expected gateway-backed outcome, got {other:?}"),
        }
        // No new entry is created under the retry's own operation id -- the
        // bounded session table must never grow for a same-key replay.
        assert_eq!(executor.gateway_session_count(), 1);
        assert_eq!(
            executor.gateway_session_phase(&retry.operation_id),
            None,
            "a replay must never be tracked under the retry's own operation id"
        );
        assert_eq!(
            peer.sends, 1,
            "a dedup replay must never re-send to the gateway peer"
        );
    }

    #[test]
    fn failed_conflicting_retry_reusing_operation_id_never_evicts_a_running_session() {
        // Reusing the same literal `operation_id` with a *different*
        // idempotency key is, from the dedup owner's perspective, an
        // entirely separate logical operation (dedup keys are scoped by
        // idempotency key, never by operation id) -- not a legitimate
        // replay of the first. If that second, conflicting attempt fails
        // at the gateway, the failure must never evict the first
        // request's already-running, successfully-established session.
        let caps = CapabilitySet::empty().with(Capability::WindowForwarding);
        let (mut executor, mut port) = gateway_backed_executor(caps);
        let realm = work_realm();
        let first = display_open_request(realm, Some("idem-1"));
        let mut peer = FakePeer::default();
        executor
            .dispatch(
                &first,
                &ProtocolToken::parse("gen-1").unwrap(),
                &mut peer,
                &mut port,
            )
            .unwrap();
        assert_eq!(
            executor.gateway_session_phase(&first.operation_id),
            Some(SessionPhase::Running)
        );

        let mut second = first.clone();
        second.idempotency_key = Some(IdempotencyKey::parse("idem-2").unwrap());
        peer.fail_send = true;
        let err = executor
            .dispatch(
                &second,
                &ProtocolToken::parse("gen-1").unwrap(),
                &mut peer,
                &mut port,
            )
            .unwrap_err();
        assert!(matches!(err, WorkExecutorError::Remote(_)));
        assert_eq!(
            executor.gateway_session_phase(&first.operation_id),
            Some(SessionPhase::Running),
            "a failed conflicting retry must never evict a prior successfully-established session"
        );
        assert_eq!(executor.gateway_session_count(), 1);
    }

    #[test]
    fn a_failed_new_reservation_never_disturbs_other_tracked_sessions() {
        let caps = CapabilitySet::empty().with(Capability::WindowForwarding);
        let (mut executor, mut port) = gateway_backed_executor(caps);
        let realm = work_realm();
        let established = display_open_request(realm.clone(), Some("idem-1"));
        let mut peer = FakePeer::default();
        executor
            .dispatch(
                &established,
                &ProtocolToken::parse("gen-1").unwrap(),
                &mut peer,
                &mut port,
            )
            .unwrap();
        assert_eq!(executor.gateway_session_count(), 1);

        let mut failing = display_open_request(realm, Some("idem-2"));
        failing.operation_id = OperationId::parse("op-display-failing").unwrap();
        peer.fail_send = true;
        let err = executor
            .dispatch(
                &failing,
                &ProtocolToken::parse("gen-1").unwrap(),
                &mut peer,
                &mut port,
            )
            .unwrap_err();
        assert!(matches!(err, WorkExecutorError::Remote(_)));

        assert_eq!(
            executor.gateway_session_phase(&established.operation_id),
            Some(SessionPhase::Running),
            "a failed, unrelated new reservation must never disturb an already-established session"
        );
        assert_eq!(executor.gateway_session_count(), 1);
        assert_eq!(
            executor.gateway_session_phase(&failing.operation_id),
            None,
            "the failed reservation itself must be released"
        );
    }

    #[test]
    fn gateway_port_boundary_mismatch_fails_closed() {
        // A port constructed for one gateway boundary must refuse a
        // dispatch resolved against a different one -- the injected port
        // is the sole authority for "does this gateway match the boundary
        // I front", never re-derived independently by `WorkExecutor`.
        let caps = CapabilitySet::empty().with(Capability::WindowForwarding);
        let (mut executor, mut port) = gateway_backed_executor(caps);
        // Rewire the entrypoint table so the resolved gateway target no
        // longer matches the port's own boundary.
        let realm = work_realm();
        let mut entrypoints = RealmEntrypointTable::new();
        entrypoints.gateway_backed(
            realm.clone(),
            RealmTarget::new(
                WorkloadId::parse("different-gateway-vm").unwrap(),
                RealmPath::local(),
            ),
        );
        *executor.entrypoints_mut() = entrypoints;

        let req = display_open_request(realm, Some("idem-1"));
        let mut peer = FakePeer::default();
        let err = executor
            .dispatch(
                &req,
                &ProtocolToken::parse("gen-1").unwrap(),
                &mut peer,
                &mut port,
            )
            .unwrap_err();
        assert!(matches!(err, WorkExecutorError::Remote(_)));
        assert_eq!(executor.gateway_session_count(), 0);
    }

    /// A [`GatewayPort`] that should never be invoked (host-resident tests
    /// must never cross the gateway boundary).
    struct NoopGatewayPort;

    impl GatewayPort for NoopGatewayPort {
        fn dispatch_via_gateway(
            &mut self,
            _gateway: &RealmTarget,
            _req: &OperationRequest,
            _generation: &ProtocolToken,
            _client: &mut dyn RemotePeerClient,
        ) -> Result<RemoteDispatchOutcome, RemoteNodeError> {
            panic!("host-resident dispatch must never reach the gateway port");
        }

        fn classify_dispatch(
            &self,
            _gateway: &RealmTarget,
            _req: &OperationRequest,
        ) -> GatewayDispatchClassification {
            panic!("host-resident dispatch must never reach the gateway port");
        }
    }
}
