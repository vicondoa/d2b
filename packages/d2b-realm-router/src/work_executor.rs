//! Work executor: the single typed dispatch surface that ties the realm
//! entrypoint resolver, the host-resident durable execution table, the
//! gateway-side remote full-host adapter, and the gateway session lifecycle
//! together behind one [`WorkExecutor::dispatch`] entry point (ADR 0032).
//!
//! [`WorkExecutor`] adds no new state machine of its own: it is pure
//! composition/glue over the router's already-owned, already-tested state
//! (`RealmEntrypointTable`, `DurableExecTable`, `RemoteFullHostAdapter`,
//! `SessionLifecycle`). Its job is exactly the routing decision an
//! integrator would otherwise have to hand-wire at every call site:
//!
//! 1. Resolve the request's realm target with the entrypoint table.
//! 2. `HostResident` → decode the operation's exec-family body and drive the
//!    local durable execution table directly. This table is host-resident
//!    metadata only; a gateway-backed dispatch never touches it.
//! 3. `GatewayBacked` → hand the request to the remote full-host adapter
//!    (codec/transport neutral: the caller supplies a
//!    [`d2b_realm_router::RemotePeerClient`] object). For a
//!    `DisplaySessionOpen` operation specifically, also drive a
//!    [`SessionLifecycle`] for that operation id, bounded by
//!    [`DEFAULT_MAX_GATEWAY_SESSIONS`] tracked sessions, so the
//!    gateway-owned display session state machine advances/fails in lockstep
//!    with the remote dispatch outcome. Every other gateway-backed operation
//!    kind is dispatched without session-lifecycle tracking (the phases
//!    model workload/display session establishment, not generic exec).
//!
//! This module depends only on the router's own already-public types
//! (`crate::{...}`) plus `d2b_realm_core`/`serde_json` (both already direct
//! dependencies of this crate). It never depends on a transport or codec
//! impl in production code — the dependency-direction gate still applies —
//! and it holds no realm relay/session/provider credential and no remote
//! node registry: routing and dedup state live entirely in the existing
//! router tables it composes.

use std::collections::HashMap;

use d2b_realm_core::{
    ConstellationError, ExecAttachRequest, ExecCancelRequest, ExecLogsRequest, ExecStartRequest,
    ExecutionSummary, OperationId, OperationKind, OperationRequest, ProtocolToken, RealmTarget,
};

use crate::remote_node::{
    RemoteDispatchOutcome, RemoteFullHostAdapter, RemoteNodeError, RemotePeerClient,
};
use crate::session_lifecycle::{SessionLifecycle, SessionPhase};
use crate::target_resolver::{DispatchTarget, RealmEntrypointTable, ResolveError};
use crate::{Clock, SystemClock};

/// Default bound on the number of gateway-backed display sessions one
/// `WorkExecutor` tracks concurrently. Bounded so a stream of
/// `DisplaySessionOpen` operations can never grow the session map without
/// limit.
pub const DEFAULT_MAX_GATEWAY_SESSIONS: usize = 4096;

/// The natural outcome of a host-resident exec-family dispatch, one variant
/// per [`OperationKind`] the durable execution table understands.
#[derive(Debug, Clone, PartialEq, Eq)]
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
    /// table handled the request directly.
    HostResident(HostResidentOutcome),
    /// The realm entrypoint resolved gateway-backed; the remote full-host
    /// adapter handled the request. `session_phase` is set only for a
    /// `DisplaySessionOpen` operation (the lifecycle this executor tracks);
    /// every other gateway-backed kind carries `None`.
    GatewayBacked {
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
    /// The gateway-backed remote full-host adapter rejected the request.
    Remote(RemoteNodeError),
    /// The bounded gateway session table is at capacity.
    GatewaySessionCapacityExceeded,
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
            WorkExecutorError::Remote(err) => write!(f, "remote full-host dispatch: {err}"),
            WorkExecutorError::GatewaySessionCapacityExceeded => {
                write!(f, "gateway session table is at capacity")
            }
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

/// The one typed dispatch surface tying realm resolution, host-resident
/// durable execution, gateway-backed remote dispatch, and gateway session
/// lifecycle tracking together. See the module documentation for the
/// dispatch contract.
pub struct WorkExecutor<C = SystemClock>
where
    C: Clock,
{
    entrypoints: RealmEntrypointTable,
    durable: crate::execution::DurableExecTable,
    remote: RemoteFullHostAdapter<C>,
    sessions: HashMap<OperationId, SessionLifecycle>,
    max_gateway_sessions: usize,
}

impl WorkExecutor<SystemClock> {
    /// Build with default-bounded tables and the default clock.
    pub fn new(
        gateway_principal: d2b_realm_core::PrincipalId,
        negotiated_capabilities: d2b_realm_core::CapabilitySet,
    ) -> Self {
        Self::with_parts(
            RealmEntrypointTable::new(),
            crate::execution::DurableExecTable::new(),
            RemoteFullHostAdapter::new(gateway_principal, negotiated_capabilities),
        )
    }
}

impl<C> WorkExecutor<C>
where
    C: Clock,
{
    /// Build from already-constructed parts (tests, or a gateway wiring
    /// injecting a manual clock / pre-populated entrypoint table).
    pub fn with_parts(
        entrypoints: RealmEntrypointTable,
        durable: crate::execution::DurableExecTable,
        remote: RemoteFullHostAdapter<C>,
    ) -> Self {
        Self {
            entrypoints,
            durable,
            remote,
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
    pub fn entrypoints(&self) -> &RealmEntrypointTable {
        &self.entrypoints
    }

    /// Borrow the host-resident durable execution table.
    pub fn durable(&self) -> &crate::execution::DurableExecTable {
        &self.durable
    }

    /// Borrow the host-resident durable execution table mutably.
    pub fn durable_mut(&mut self) -> &mut crate::execution::DurableExecTable {
        &mut self.durable
    }

    /// Borrow the gateway-side remote full-host adapter (e.g. to register
    /// remote nodes before serving traffic).
    pub fn remote_mut(&mut self) -> &mut RemoteFullHostAdapter<C> {
        &mut self.remote
    }

    /// Borrow the gateway-side remote full-host adapter.
    pub fn remote(&self) -> &RemoteFullHostAdapter<C> {
        &self.remote
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
    ) -> Result<WorkDispatchOutcome, WorkExecutorError> {
        let target = realm_target_from_request(req)?;
        match self
            .entrypoints
            .resolve(&target)
            .map_err(WorkExecutorError::Resolve)?
        {
            DispatchTarget::HostResident { .. } => Ok(WorkDispatchOutcome::HostResident(
                self.dispatch_host_resident(req)?,
            )),
            DispatchTarget::GatewayBacked { .. } => {
                self.dispatch_gateway_backed(req, generation, client)
            }
        }
    }

    fn dispatch_host_resident(
        &mut self,
        req: &OperationRequest,
    ) -> Result<HostResidentOutcome, WorkExecutorError> {
        match req.kind {
            OperationKind::ExecStart => {
                let decoded: ExecStartRequest = decode_body(req)?;
                let summary = self
                    .durable
                    .start(decoded)
                    .map_err(WorkExecutorError::Durable)?;
                Ok(HostResidentOutcome::Started(summary))
            }
            OperationKind::ExecAttach => {
                let decoded: ExecAttachRequest = decode_body(req)?;
                let summary = self
                    .durable
                    .attach(&decoded)
                    .map_err(WorkExecutorError::Durable)?;
                Ok(HostResidentOutcome::Attached(summary))
            }
            OperationKind::ExecLogs => {
                let decoded: ExecLogsRequest = decode_body(req)?;
                let summary = self
                    .durable
                    .logs(&decoded)
                    .map_err(WorkExecutorError::Durable)?;
                Ok(HostResidentOutcome::Logs(summary))
            }
            OperationKind::ExecCancel => {
                let decoded: ExecCancelRequest = decode_body(req)?;
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
        req: &OperationRequest,
        generation: &ProtocolToken,
        client: &mut dyn RemotePeerClient,
    ) -> Result<WorkDispatchOutcome, WorkExecutorError> {
        let tracks_session = req.kind == OperationKind::DisplaySessionOpen;
        if tracks_session && !self.sessions.contains_key(&req.operation_id) {
            if self.sessions.len() >= self.max_gateway_sessions {
                return Err(WorkExecutorError::GatewaySessionCapacityExceeded);
            }
            self.sessions
                .insert(req.operation_id.clone(), SessionLifecycle::new());
        }

        let result = self.remote.dispatch(req, generation, client);

        let session_phase = if tracks_session {
            let lifecycle = self
                .sessions
                .get_mut(&req.operation_id)
                .expect("inserted above when absent");
            match &result {
                Ok(RemoteDispatchOutcome::Sent { .. })
                | Ok(RemoteDispatchOutcome::Replayed { .. }) => {
                    // Drive the session all the way to `Running`: dispatch
                    // succeeding means the remote peer accepted/replayed the
                    // display-session-open operation.
                    while lifecycle.advance().is_ok() {}
                }
                Ok(RemoteDispatchOutcome::QueryRemoteState { .. }) => {
                    // Ambiguous outcome: leave the phase where it is until
                    // the caller resolves the query.
                }
                Err(_) => lifecycle.fail(),
            }
            Some(lifecycle.phase())
        } else {
            None
        };

        let remote = result.map_err(WorkExecutorError::Remote)?;
        Ok(WorkDispatchOutcome::GatewayBacked {
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
        RemoteNodeError, RemoteNodeErrorKind, RemoteNodeRegistration, RemotePeerStatus, RemoteRoute,
    };

    /// The gateway-backed test realm (`work`).
    fn work_realm() -> RealmPath {
        RealmPath::new(vec![RealmId::parse("work").unwrap()]).unwrap()
    }

    fn principal() -> PrincipalId {
        PrincipalId::parse("gateway-principal").unwrap()
    }

    fn gateway_node() -> NodeId {
        NodeId::parse("gateway").unwrap()
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
            node: NodeId::parse("this-host").unwrap(),
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

    fn host_resident_executor() -> WorkExecutor<SystemClock> {
        let mut executor = WorkExecutor::new(principal(), CapabilitySet::empty());
        executor.entrypoints_mut().host_resident(RealmPath::local());
        executor
    }

    fn gateway_backed_executor(caps: CapabilitySet) -> WorkExecutor<SystemClock> {
        let gateway_realm = work_realm();
        let remote = RemoteFullHostAdapter::with_router(
            crate::remote_node::RemoteNodeRegistry::for_realm(gateway_realm.clone()),
            crate::OperationRouter::new(),
            principal(),
            caps.clone(),
        );
        let mut entrypoints = RealmEntrypointTable::new();
        entrypoints.gateway_backed(
            gateway_realm,
            RealmTarget::new(WorkloadId::parse("gateway-vm").unwrap(), RealmPath::local()),
        );
        let mut executor = WorkExecutor::with_parts(
            entrypoints,
            crate::execution::DurableExecTable::new(),
            remote,
        );
        executor
            .remote_mut()
            .registry_mut()
            .register(
                remote_registration("gen-1", caps),
                std::time::Instant::now(),
            )
            .unwrap();
        executor
    }

    #[test]
    fn host_resident_exec_start_reaches_the_durable_table() {
        let mut executor = host_resident_executor();
        let req = exec_start_request(OperationKind::ExecStart, "exec-1", None);
        let mut peer = FakePeer::default();
        let outcome = executor
            .dispatch(&req, &ProtocolToken::parse("boot-1").unwrap(), &mut peer)
            .unwrap();
        match outcome {
            WorkDispatchOutcome::HostResident(HostResidentOutcome::Started(summary)) => {
                assert_eq!(summary.id, ExecutionId::parse("exec-1").unwrap());
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
        let mut executor = host_resident_executor();
        let mut req = exec_start_request(OperationKind::ExecStart, "exec-1", None);
        req.workload = None;
        let mut peer = FakePeer::default();
        let err = executor
            .dispatch(&req, &ProtocolToken::parse("boot-1").unwrap(), &mut peer)
            .unwrap_err();
        assert_eq!(err, WorkExecutorError::MissingWorkload);
    }

    #[test]
    fn unresolved_realm_fails_closed() {
        let mut executor = WorkExecutor::new(principal(), CapabilitySet::empty());
        let req = exec_start_request(OperationKind::ExecStart, "exec-1", None);
        let mut peer = FakePeer::default();
        let err = executor
            .dispatch(&req, &ProtocolToken::parse("boot-1").unwrap(), &mut peer)
            .unwrap_err();
        assert!(matches!(err, WorkExecutorError::Resolve(_)));
    }

    #[test]
    fn malformed_body_is_rejected_before_touching_the_durable_table() {
        let mut executor = host_resident_executor();
        let mut req = exec_start_request(OperationKind::ExecStart, "exec-1", None);
        req.body = OpaquePayload::new(b"not-json".to_vec()).unwrap();
        let mut peer = FakePeer::default();
        let err = executor
            .dispatch(&req, &ProtocolToken::parse("boot-1").unwrap(), &mut peer)
            .unwrap_err();
        assert_eq!(err, WorkExecutorError::MalformedBody);
        assert!(executor.durable().is_empty());
    }

    #[test]
    fn unsupported_host_resident_kind_is_rejected() {
        let mut executor = host_resident_executor();
        let mut req = exec_start_request(OperationKind::ExecStart, "exec-1", None);
        req.kind = OperationKind::WorkloadStart;
        let mut peer = FakePeer::default();
        let err = executor
            .dispatch(&req, &ProtocolToken::parse("boot-1").unwrap(), &mut peer)
            .unwrap_err();
        assert_eq!(
            err,
            WorkExecutorError::UnsupportedHostResidentOperation(OperationKind::WorkloadStart)
        );
    }

    #[test]
    fn gateway_backed_display_session_advances_to_running_on_success() {
        let caps = CapabilitySet::empty().with(Capability::WindowForwarding);
        let mut executor = gateway_backed_executor(caps);
        let realm = work_realm();
        let req = display_open_request(realm, Some("idem-1"));
        let mut peer = FakePeer::default();

        let outcome = executor
            .dispatch(&req, &ProtocolToken::parse("gen-1").unwrap(), &mut peer)
            .unwrap();
        match outcome {
            WorkDispatchOutcome::GatewayBacked {
                session_phase,
                remote,
            } => {
                assert_eq!(session_phase, Some(SessionPhase::Running));
                assert!(matches!(remote, RemoteDispatchOutcome::Sent { .. }));
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
    fn gateway_backed_display_session_fails_the_lifecycle_on_remote_error() {
        let caps = CapabilitySet::empty().with(Capability::WindowForwarding);
        let mut executor = gateway_backed_executor(caps);
        let realm = work_realm();
        let req = display_open_request(realm, Some("idem-1"));
        let mut peer = FakePeer {
            fail_send: true,
            ..Default::default()
        };

        let err = executor
            .dispatch(&req, &ProtocolToken::parse("gen-1").unwrap(), &mut peer)
            .unwrap_err();
        assert!(matches!(err, WorkExecutorError::Remote(_)));
        // The lifecycle is still tracked, rolled into teardown by the
        // failure (never silently dropped).
        assert_eq!(
            executor.gateway_session_phase(&req.operation_id),
            Some(SessionPhase::Stopping)
        );
    }

    #[test]
    fn stop_gateway_session_evicts_once_stopped() {
        let caps = CapabilitySet::empty().with(Capability::WindowForwarding);
        let mut executor = gateway_backed_executor(caps);
        let realm = work_realm();
        let req = display_open_request(realm, Some("idem-1"));
        let mut peer = FakePeer::default();
        executor
            .dispatch(&req, &ProtocolToken::parse("gen-1").unwrap(), &mut peer)
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
        let mut executor = gateway_backed_executor(caps);
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
            .dispatch(&req, &ProtocolToken::parse("gen-1").unwrap(), &mut peer)
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
    fn gateway_session_capacity_is_enforced() {
        let caps = CapabilitySet::empty().with(Capability::WindowForwarding);
        let mut executor = gateway_backed_executor(caps).with_max_gateway_sessions(1);
        let realm = work_realm();
        let first = display_open_request(realm.clone(), Some("idem-1"));
        let mut peer = FakePeer::default();
        executor
            .dispatch(&first, &ProtocolToken::parse("gen-1").unwrap(), &mut peer)
            .unwrap();

        let mut second = display_open_request(realm, Some("idem-2"));
        second.operation_id = OperationId::parse("op-display-2").unwrap();
        let err = executor
            .dispatch(&second, &ProtocolToken::parse("gen-1").unwrap(), &mut peer)
            .unwrap_err();
        assert_eq!(err, WorkExecutorError::GatewaySessionCapacityExceeded);
    }
}
