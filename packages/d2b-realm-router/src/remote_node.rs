//! Remote full-host node routing state.
//!
//! This module is intentionally pure router state. It tracks enrolled
//! full-host nodes and prepares semantic remote operation routing decisions;
//! it never carries transport endpoints, host paths, file descriptors, pidfds,
//! broker operations, or guest-control frames.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use d2b_realm_core::{
    Capability, CapabilitySet, CorrelationId, ErrorKind, ExecutionGeneration, NodeId, NodeKind,
    NodeSummary, OperationKind, OperationRequest, PrincipalId, ProtocolToken, ProviderId,
    RealmPath, ShellGeneration, TraceContext,
};

/// Default maximum number of remote full-host nodes retained by one registry.
pub const DEFAULT_MAX_REMOTE_NODES: usize = 4096;
/// Default heartbeat timeout used when a peer disconnect event was not seen.
pub const DEFAULT_HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(30);

/// Lifecycle state for a registered remote full-host node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteNodeAvailability {
    /// A peer session is registered and not stale.
    Available,
    /// The peer session closed; refuse routes immediately.
    Disconnected,
    /// The heartbeat fallback elapsed without a refresh.
    StaleHeartbeat,
}

/// Why a remote node registration or route was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteNodeErrorKind {
    WrongRealm,
    WrongNode,
    NotFullHost,
    UnauthorizedGateway,
    DuplicateRegistration,
    StaleGeneration,
    RegistryCapacityExceeded,
    DedupCapacityExceeded,
    MissingIdempotencyKey,
    IdempotencyConflict,
    IdempotencyExpired,
    RemoteOperationUnknown,
    NodeUnavailable,
    CapabilityDenied,
    MissingWorkload,
    UnsupportedOperation,
}

impl RemoteNodeErrorKind {
    /// Stable low-cardinality code safe for audit/error labels.
    pub fn code(self) -> &'static str {
        match self {
            Self::WrongRealm => "wrong-realm",
            Self::WrongNode => "wrong-node",
            Self::NotFullHost => "not-full-host",
            Self::UnauthorizedGateway => "unauthorized-gateway",
            Self::DuplicateRegistration => "duplicate-registration",
            Self::StaleGeneration => "stale-node-generation",
            Self::RegistryCapacityExceeded => "registry-capacity-exceeded",
            Self::DedupCapacityExceeded => "dedup-capacity-exceeded",
            Self::MissingIdempotencyKey => "missing-idempotency-key",
            Self::IdempotencyConflict => "idempotency-key-conflict",
            Self::IdempotencyExpired => "idempotency-key-expired",
            Self::RemoteOperationUnknown => "remote-operation-unknown",
            Self::NodeUnavailable => "remote-node-unavailable",
            Self::CapabilityDenied => "capability-denied",
            Self::MissingWorkload => "missing-workload",
            Self::UnsupportedOperation => "unsupported-operation",
        }
    }

    /// Map to the existing typed constellation error vocabulary.
    pub fn error_kind(self) -> ErrorKind {
        match self {
            Self::CapabilityDenied => ErrorKind::CapabilityDenied,
            Self::RegistryCapacityExceeded | Self::DedupCapacityExceeded => ErrorKind::Backpressure,
            Self::NodeUnavailable => ErrorKind::GatewayUnavailable,
            Self::RemoteOperationUnknown => ErrorKind::GatewayUnavailable,
            Self::UnsupportedOperation => ErrorKind::UnsupportedFeature,
            Self::IdempotencyConflict => ErrorKind::IdempotencyKeyConflict,
            Self::IdempotencyExpired => ErrorKind::IdempotencyKeyExpired,
            Self::UnauthorizedGateway => ErrorKind::Unauthorized,
            Self::WrongRealm
            | Self::WrongNode
            | Self::NotFullHost
            | Self::DuplicateRegistration
            | Self::StaleGeneration
            | Self::MissingIdempotencyKey
            | Self::MissingWorkload => ErrorKind::InvalidTarget,
        }
    }

    /// Operator-facing remediation text. This is bounded static prose and
    /// deliberately avoids endpoints, credentials, paths, and provider data.
    pub fn remediation(self) -> &'static str {
        match self {
            Self::WrongRealm => "check the target realm and remote node enrollment",
            Self::WrongNode => "check the target node id and remote node enrollment",
            Self::NotFullHost => "register only full d2b hosts through this adapter",
            Self::UnauthorizedGateway => "enroll the gateway principal for this realm and node",
            Self::DuplicateRegistration => {
                "re-register with the current generation or remove the conflicting node"
            }
            Self::StaleGeneration => "refresh remote node registration before retrying",
            Self::RegistryCapacityExceeded => {
                "remove stale remote nodes or raise the registry capacity"
            }
            Self::DedupCapacityExceeded => {
                "wait for in-flight operations to finish or raise the dedup capacity"
            }
            Self::MissingIdempotencyKey => "retry the mutating operation with an idempotency key",
            Self::IdempotencyConflict => {
                "use a fresh idempotency key or retry the original request"
            }
            Self::IdempotencyExpired => "start a new operation with a fresh idempotency key",
            Self::RemoteOperationUnknown => {
                "retry after the remote node reconciles operation state"
            }
            Self::NodeUnavailable => {
                "check the remote daemon peer session and re-register the node"
            }
            Self::CapabilityDenied => {
                "check the substrate provider report and re-register after resolving capability gaps"
            }
            Self::MissingWorkload => "target a workload for workload or execution operations",
            Self::UnsupportedOperation => "use a supported remote full-host operation",
        }
    }
}

/// A typed refusal from the remote-node registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteNodeError {
    /// Stable reason.
    pub kind: RemoteNodeErrorKind,
    /// Missing capability, when `kind == CapabilityDenied`.
    pub missing_capability: Option<Capability>,
    /// Bounded capability fingerprint of the registered node.
    pub capability_fingerprint: Option<String>,
    /// Operation correlation id when this refusal came from an operation path.
    pub correlation_id: Option<CorrelationId>,
}

impl RemoteNodeError {
    fn new(kind: RemoteNodeErrorKind) -> Self {
        Self {
            kind,
            missing_capability: None,
            capability_fingerprint: None,
            correlation_id: None,
        }
    }

    fn capability_denied(capability: Capability, fingerprint: String) -> Self {
        Self {
            kind: RemoteNodeErrorKind::CapabilityDenied,
            missing_capability: Some(capability),
            capability_fingerprint: Some(fingerprint),
            correlation_id: None,
        }
    }

    fn with_correlation_id(mut self, correlation_id: CorrelationId) -> Self {
        self.correlation_id = Some(correlation_id);
        self
    }

    /// Stable reason code.
    pub fn code(&self) -> &'static str {
        self.kind.code()
    }

    /// Operator-safe remediation text.
    pub fn remediation(&self) -> &'static str {
        self.kind.remediation()
    }
}

impl core::fmt::Display for RemoteNodeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}: {}", self.code(), self.remediation())
    }
}

impl std::error::Error for RemoteNodeError {}

/// Registration request for one remote full-host node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteNodeRegistration {
    /// The full-host node summary being registered.
    pub summary: NodeSummary,
    /// Authenticated gateway principal for the peer session.
    pub gateway_principal: PrincipalId,
    /// Authenticated gateway node for the peer session, when bound.
    pub gateway_node: NodeId,
    /// Host substrate adapter id advertised by the remote host.
    pub substrate_adapter: ProviderId,
    /// Bounded non-secret remote boot/generation token.
    pub generation: ProtocolToken,
}

/// Bounded metadata retained for a registered remote node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteNodeEntry {
    /// Advertised full-host node summary.
    pub summary: NodeSummary,
    /// Gateway principal authenticated on the peer session.
    pub gateway_principal: PrincipalId,
    /// Gateway node authenticated on the peer session.
    pub gateway_node: NodeId,
    /// Host substrate adapter id advertised by the remote host.
    pub substrate_adapter: ProviderId,
    /// Current remote node generation.
    pub generation: ProtocolToken,
    /// Capability fingerprint used for low-cardinality audit correlation.
    pub capability_fingerprint: String,
    availability: RemoteNodeAvailability,
    last_heartbeat: Instant,
}

impl RemoteNodeEntry {
    /// Current availability.
    pub fn availability(&self) -> RemoteNodeAvailability {
        self.availability
    }

    /// Whether an operation for `generation` is known stale.
    pub fn is_stale_generation(&self, generation: &ProtocolToken) -> bool {
        generation != &self.generation
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct RemoteNodeKey {
    realm: RealmPath,
    node: NodeId,
}

impl RemoteNodeKey {
    fn new(realm: RealmPath, node: NodeId) -> Self {
        Self { realm, node }
    }
}

/// Pure registry and route-preflight owner for remote full-host nodes.
#[derive(Debug, Clone)]
pub struct RemoteNodeRegistry {
    managed_realm: RealmPath,
    nodes: BTreeMap<RemoteNodeKey, RemoteNodeEntry>,
    max_nodes: usize,
    heartbeat_timeout: Duration,
}

impl Default for RemoteNodeRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl RemoteNodeRegistry {
    /// Construct with default bounds.
    pub fn new() -> Self {
        Self::for_realm(RealmPath::local())
    }

    /// Construct for a specific gateway-managed realm.
    pub fn for_realm(managed_realm: RealmPath) -> Self {
        Self {
            managed_realm,
            nodes: BTreeMap::new(),
            max_nodes: DEFAULT_MAX_REMOTE_NODES,
            heartbeat_timeout: DEFAULT_HEARTBEAT_TIMEOUT,
        }
    }

    /// Override retained node capacity.
    pub fn with_max_nodes(mut self, max_nodes: usize) -> Self {
        self.max_nodes = max_nodes;
        self
    }

    /// Override heartbeat timeout.
    pub fn with_heartbeat_timeout(mut self, timeout: Duration) -> Self {
        self.heartbeat_timeout = timeout;
        self
    }

    /// Register or re-register a remote full-host node.
    pub fn register(
        &mut self,
        registration: RemoteNodeRegistration,
        now: Instant,
    ) -> Result<RemoteNodeEntry, RemoteNodeError> {
        let node_label = registration.summary.id.as_str().to_owned();
        let realm_label = registration.summary.realm.target_form();
        let substrate_label = registration.substrate_adapter.as_str().to_owned();
        tracing::info!(
            event = "remote-node-register",
            realm = %realm_label,
            node = %node_label,
            substrate_adapter = %substrate_label,
            "remote full-host node registration preflight"
        );
        if registration.summary.kind != NodeKind::FullHost {
            return Err(RemoteNodeError::new(RemoteNodeErrorKind::NotFullHost));
        }
        if registration.summary.realm != self.managed_realm {
            return Err(RemoteNodeError::new(RemoteNodeErrorKind::WrongRealm));
        }
        if registration.gateway_node == registration.summary.id {
            return Err(RemoteNodeError::new(
                RemoteNodeErrorKind::UnauthorizedGateway,
            ));
        }
        let key = RemoteNodeKey::new(
            registration.summary.realm.clone(),
            registration.summary.id.clone(),
        );
        let fingerprint = registration.summary.capabilities.stable_fingerprint();
        if let Some(existing) = self.nodes.get_mut(&key) {
            if existing.gateway_principal != registration.gateway_principal {
                return Err(RemoteNodeError::new(
                    RemoteNodeErrorKind::DuplicateRegistration,
                ));
            }
            if existing.generation == registration.generation {
                if existing.summary != registration.summary
                    || existing.gateway_node != registration.gateway_node
                    || existing.substrate_adapter != registration.substrate_adapter
                {
                    return Err(RemoteNodeError::new(
                        RemoteNodeErrorKind::DuplicateRegistration,
                    ));
                }
            } else {
                existing.generation = registration.generation;
            }
            existing.summary = registration.summary;
            existing.gateway_node = registration.gateway_node;
            existing.substrate_adapter = registration.substrate_adapter;
            existing.capability_fingerprint = fingerprint;
            existing.availability = RemoteNodeAvailability::Available;
            existing.last_heartbeat = now;
            return Ok(existing.clone());
        }

        if self.nodes.len() >= self.max_nodes {
            return Err(RemoteNodeError::new(
                RemoteNodeErrorKind::RegistryCapacityExceeded,
            ));
        }

        let entry = RemoteNodeEntry {
            summary: registration.summary,
            gateway_principal: registration.gateway_principal,
            gateway_node: registration.gateway_node,
            substrate_adapter: registration.substrate_adapter,
            generation: registration.generation,
            capability_fingerprint: fingerprint,
            availability: RemoteNodeAvailability::Available,
            last_heartbeat: now,
        };
        self.nodes.insert(key, entry.clone());
        Ok(entry)
    }

    /// Refresh heartbeat for the current generation.
    pub fn heartbeat(
        &mut self,
        realm: &RealmPath,
        node: &NodeId,
        generation: &ProtocolToken,
        now: Instant,
    ) -> Result<RemoteNodeEntry, RemoteNodeError> {
        tracing::info!(
            event = "remote-node-heartbeat",
            realm = %realm.target_form(),
            node = %node.as_str(),
            "remote full-host node heartbeat"
        );
        let entry = self
            .entry_mut(realm, node)
            .ok_or_else(|| RemoteNodeError::new(RemoteNodeErrorKind::WrongNode))?;
        if &entry.generation != generation {
            return Err(RemoteNodeError::new(RemoteNodeErrorKind::StaleGeneration));
        }
        if entry.availability == RemoteNodeAvailability::Disconnected {
            return Err(RemoteNodeError::new(RemoteNodeErrorKind::NodeUnavailable));
        }
        entry.availability = RemoteNodeAvailability::Available;
        entry.last_heartbeat = now;
        Ok(entry.clone())
    }

    /// Mark a peer-session disconnect immediately.
    pub fn peer_disconnected(&mut self, realm: &RealmPath, node: &NodeId) -> bool {
        tracing::info!(
            event = "remote-node-disconnected",
            realm = %realm.target_form(),
            node = %node.as_str(),
            "remote full-host node marked unavailable after peer disconnect"
        );
        let Some(entry) = self.entry_mut(realm, node) else {
            return false;
        };
        entry.availability = RemoteNodeAvailability::Disconnected;
        true
    }

    /// Mark nodes stale when heartbeat fallback expires.
    pub fn expire_heartbeats(&mut self, now: Instant) {
        for entry in self.nodes.values_mut() {
            if entry.availability == RemoteNodeAvailability::Available
                && now.duration_since(entry.last_heartbeat) > self.heartbeat_timeout
            {
                entry.availability = RemoteNodeAvailability::StaleHeartbeat;
            }
        }
    }

    /// Borrow a registered node.
    pub fn get(&self, realm: &RealmPath, node: &NodeId) -> Option<&RemoteNodeEntry> {
        self.nodes
            .get(&RemoteNodeKey::new(realm.clone(), node.clone()))
    }

    /// Remove a registered remote node explicitly.
    pub fn unregister(&mut self, realm: &RealmPath, node: &NodeId) -> Option<RemoteNodeEntry> {
        self.nodes
            .remove(&RemoteNodeKey::new(realm.clone(), node.clone()))
    }

    /// Number of registered nodes.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Check whether a remote operation may be sent.
    pub fn prepare_route(
        &self,
        req: &OperationRequest,
        generation: &ProtocolToken,
        gateway_principal: &PrincipalId,
    ) -> Result<RemoteRoute, RemoteNodeError> {
        if req.realm != self.managed_realm {
            return Err(RemoteNodeError::new(RemoteNodeErrorKind::WrongRealm)
                .with_correlation_id(req.correlation_id.clone()));
        }
        let entry = self.get(&req.realm, &req.node).ok_or_else(|| {
            RemoteNodeError::new(RemoteNodeErrorKind::WrongNode)
                .with_correlation_id(req.correlation_id.clone())
        })?;
        if req.node != entry.summary.id {
            return Err(RemoteNodeError::new(RemoteNodeErrorKind::WrongNode)
                .with_correlation_id(req.correlation_id.clone()));
        }
        if &entry.gateway_principal != gateway_principal {
            return Err(
                RemoteNodeError::new(RemoteNodeErrorKind::UnauthorizedGateway)
                    .with_correlation_id(req.correlation_id.clone()),
            );
        }
        if entry.availability != RemoteNodeAvailability::Available {
            return Err(RemoteNodeError::new(RemoteNodeErrorKind::NodeUnavailable)
                .with_correlation_id(req.correlation_id.clone()));
        }
        if entry.is_stale_generation(generation) {
            return Err(RemoteNodeError::new(RemoteNodeErrorKind::StaleGeneration)
                .with_correlation_id(req.correlation_id.clone()));
        }
        let required_capability = required_remote_capability(req.kind)
            .map_err(|err| err.with_correlation_id(req.correlation_id.clone()))?;
        if let Some(capability) = required_capability
            && !entry.summary.capabilities.has(capability)
        {
            return Err(RemoteNodeError::capability_denied(
                capability,
                entry.capability_fingerprint.clone(),
            )
            .with_correlation_id(req.correlation_id.clone()));
        }
        if req.workload.is_none() && remote_operation_requires_workload(req.kind) {
            return Err(RemoteNodeError::new(RemoteNodeErrorKind::MissingWorkload)
                .with_correlation_id(req.correlation_id.clone()));
        }
        Ok(RemoteRoute {
            realm: req.realm.clone(),
            node: req.node.clone(),
            generation: entry.generation.clone(),
            operation: req.kind,
            correlation_id: req.correlation_id.clone(),
            required_capability,
            capability_fingerprint: entry.capability_fingerprint.clone(),
            principal: entry.gateway_principal.clone(),
            trace: req.trace.clone(),
            mutating: req.kind.is_mutating(),
        })
    }

    fn entry_mut(&mut self, realm: &RealmPath, node: &NodeId) -> Option<&mut RemoteNodeEntry> {
        self.nodes
            .get_mut(&RemoteNodeKey::new(realm.clone(), node.clone()))
    }
}

/// Route metadata that is safe to audit and pass to a semantic peer client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteRoute {
    /// Target realm.
    pub realm: RealmPath,
    /// Target node.
    pub node: NodeId,
    /// Registered node generation.
    pub generation: ProtocolToken,
    /// Trusted operation kind.
    pub operation: OperationKind,
    /// Cross-realm correlation id shared across route and audit hops.
    pub correlation_id: CorrelationId,
    /// Capability required by the operation, if any.
    pub required_capability: Option<Capability>,
    /// Bounded capability fingerprint.
    pub capability_fingerprint: String,
    /// Authenticated gateway principal.
    pub principal: PrincipalId,
    /// Bounded trace context propagated for audit correlation.
    pub trace: Option<TraceContext>,
    /// Whether this route is for a mutating operation.
    pub mutating: bool,
}

impl RemoteRoute {
    /// Bounded labels safe for audit, traces, and tests.
    pub fn audit_labels(&self, outcome: &'static str) -> RemoteNodeAuditLabels<'_> {
        RemoteNodeAuditLabels {
            realm: self.realm.target_form(),
            node: self.node.as_str(),
            operation: self.operation,
            correlation_id: self.correlation_id.as_str(),
            principal: self.principal.as_str(),
            capability_fingerprint: &self.capability_fingerprint,
            trace_id: self.trace.as_ref().map(TraceContext::trace_id),
            span_id: self.trace.as_ref().map(TraceContext::span_id),
            outcome,
        }
    }
}

/// Low-cardinality route labels suitable for audit/telemetry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteNodeAuditLabels<'a> {
    pub realm: String,
    pub node: &'a str,
    pub operation: OperationKind,
    pub correlation_id: &'a str,
    pub principal: &'a str,
    pub capability_fingerprint: &'a str,
    pub trace_id: Option<&'a str>,
    pub span_id: Option<&'a str>,
    pub outcome: &'static str,
}

/// Recovering an unacknowledged mutating operation must query remote state
/// before retrying.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteRetryAction {
    QueryRemoteState,
    NoSideEffectToRetry,
}

/// Decide how to recover after a disconnect.
pub fn retry_action_after_disconnect(req: &OperationRequest) -> RemoteRetryAction {
    if req.kind.is_mutating() {
        RemoteRetryAction::QueryRemoteState
    } else {
        RemoteRetryAction::NoSideEffectToRetry
    }
}

/// Result from a semantic remote peer client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemotePeerStatus {
    /// The remote host recorded a completed semantic result.
    Completed(d2b_realm_core::OpaquePayload),
    /// The remote host still has the same operation in progress.
    InProgress,
    /// The remote host has no matching side effect for this idempotency key.
    Unknown,
}

/// Transport-neutral remote peer seam.
///
/// Implementations exchange semantic operation envelopes. They must not expose
/// bytes, frames, sockets, endpoints, broker ops, guest-control frames, or file
/// descriptors through this trait.
pub trait RemotePeerClient: Send {
    /// Send a newly accepted operation to the remote host.
    fn send_operation(
        &mut self,
        route: &RemoteRoute,
        req: &OperationRequest,
    ) -> Result<d2b_realm_core::OpaquePayload, RemoteNodeError>;

    /// Query remote state for an unacknowledged mutating operation before any
    /// retry can be considered.
    fn query_operation(
        &mut self,
        route: &RemoteRoute,
        req: &OperationRequest,
    ) -> Result<RemotePeerStatus, RemoteNodeError>;
}

/// Outcome of gateway-side remote full-host dispatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteDispatchOutcome {
    /// A semantic operation was sent and completed.
    Sent {
        route: RemoteRoute,
        result: d2b_realm_core::OpaquePayload,
    },
    /// A same-request retry was satisfied from gateway-side dedup state.
    Replayed {
        result: d2b_realm_core::OpaquePayload,
    },
    /// The operation may have reached the remote host; query remote state
    /// before retrying.
    QueryRemoteState { route: RemoteRoute },
}

/// Gateway-side remote full-host adapter. It gates through the shared router
/// and registry before a semantic peer client can see an operation.
pub struct RemoteFullHostAdapter<C = super::SystemClock>
where
    C: super::Clock,
{
    registry: RemoteNodeRegistry,
    router: super::OperationRouter<C>,
    gateway_principal: PrincipalId,
    negotiated_capabilities: CapabilitySet,
}

impl RemoteFullHostAdapter<super::SystemClock> {
    /// Build with default registry/router bounds.
    pub fn new(gateway_principal: PrincipalId, negotiated_capabilities: CapabilitySet) -> Self {
        Self::with_router(
            RemoteNodeRegistry::new(),
            super::OperationRouter::new(),
            gateway_principal,
            negotiated_capabilities,
        )
    }
}

impl<C> RemoteFullHostAdapter<C>
where
    C: super::Clock,
{
    /// Build with injected state for tests or gateway composition.
    pub fn with_router(
        registry: RemoteNodeRegistry,
        router: super::OperationRouter<C>,
        gateway_principal: PrincipalId,
        negotiated_capabilities: CapabilitySet,
    ) -> Self {
        Self {
            registry,
            router,
            gateway_principal,
            negotiated_capabilities,
        }
    }

    /// Mutable access to the registry for enrollment/heartbeat paths.
    pub fn registry_mut(&mut self) -> &mut RemoteNodeRegistry {
        &mut self.registry
    }

    /// Borrow registry state.
    pub fn registry(&self) -> &RemoteNodeRegistry {
        &self.registry
    }

    /// Route and send one operation through the semantic peer client.
    pub fn dispatch(
        &mut self,
        req: &OperationRequest,
        generation: &ProtocolToken,
        client: &mut dyn RemotePeerClient,
    ) -> Result<RemoteDispatchOutcome, RemoteNodeError> {
        let route = self
            .registry
            .prepare_route(req, generation, &self.gateway_principal)?;
        let labels = route.audit_labels("preflight-ok");
        tracing::info!(
            event = "remote-node-dispatch",
            realm = %labels.realm,
            node = %labels.node,
            operation_kind = ?labels.operation,
            correlation_id = %labels.correlation_id,
            principal = %labels.principal,
            capability_fingerprint = %labels.capability_fingerprint,
            trace_id = labels.trace_id,
            span_id = labels.span_id,
            outcome = labels.outcome,
            "remote full-host operation dispatch preflight accepted"
        );
        match self.router.route_with_capabilities(
            req,
            &self.gateway_principal,
            &self.negotiated_capabilities,
        ) {
            super::RouteDecision::Accept { .. } => {
                let result = match client.send_operation(&route, req) {
                    Ok(result) => result,
                    Err(err) => {
                        if req.kind.is_mutating() {
                            self.router.mark_failed(req);
                        }
                        return Err(err);
                    }
                };
                let labels = route.audit_labels("sent");
                tracing::info!(
                    event = "remote-node-dispatch",
                    realm = %labels.realm,
                    node = %labels.node,
                    operation_kind = ?labels.operation,
                    correlation_id = %labels.correlation_id,
                    principal = %labels.principal,
                    capability_fingerprint = %labels.capability_fingerprint,
                    trace_id = labels.trace_id,
                    span_id = labels.span_id,
                    outcome = labels.outcome,
                    "remote full-host operation sent"
                );
                if req.kind.is_mutating() {
                    self.router.mark_completed(req, result.clone());
                }
                Ok(RemoteDispatchOutcome::Sent { route, result })
            }
            super::RouteDecision::Replay { result, .. } => {
                Ok(RemoteDispatchOutcome::Replayed { result })
            }
            super::RouteDecision::InProgress { .. } => match client.query_operation(&route, req) {
                Err(err) => {
                    if req.kind.is_mutating() {
                        self.router.mark_failed(req);
                    }
                    Err(err)
                }
                Ok(status) => match status {
                    RemotePeerStatus::Completed(result) => {
                        self.router.mark_completed(req, result.clone());
                        Ok(RemoteDispatchOutcome::Replayed { result })
                    }
                    RemotePeerStatus::InProgress => {
                        let labels = route.audit_labels("query-remote-state");
                        tracing::info!(
                            event = "remote-node-dispatch",
                            realm = %labels.realm,
                            node = %labels.node,
                            operation_kind = ?labels.operation,
                            correlation_id = %labels.correlation_id,
                            principal = %labels.principal,
                            capability_fingerprint = %labels.capability_fingerprint,
                            trace_id = labels.trace_id,
                            span_id = labels.span_id,
                            outcome = labels.outcome,
                            "remote full-host operation requires remote state query"
                        );
                        Ok(RemoteDispatchOutcome::QueryRemoteState { route })
                    }
                    RemotePeerStatus::Unknown => {
                        if req.kind.is_mutating() {
                            self.router.mark_failed(req);
                        }
                        Err(
                            RemoteNodeError::new(RemoteNodeErrorKind::RemoteOperationUnknown)
                                .with_correlation_id(req.correlation_id.clone()),
                        )
                    }
                },
            },
            decision => Err(remote_error_from_route_decision(req, decision)),
        }
    }
}

fn remote_error_from_route_decision(
    req: &OperationRequest,
    decision: super::RouteDecision,
) -> RemoteNodeError {
    let err = match decision {
        super::RouteDecision::PrincipalMismatch => {
            RemoteNodeError::new(RemoteNodeErrorKind::UnauthorizedGateway)
        }
        super::RouteDecision::CapabilityDenied {
            capability,
            negotiated_fingerprint,
        } => RemoteNodeError::capability_denied(capability, negotiated_fingerprint),
        super::RouteDecision::MissingIdempotencyKey => {
            RemoteNodeError::new(RemoteNodeErrorKind::MissingIdempotencyKey)
        }
        super::RouteDecision::IdempotencyKeyConflict => {
            RemoteNodeError::new(RemoteNodeErrorKind::IdempotencyConflict)
        }
        super::RouteDecision::IdempotencyKeyExpired => {
            RemoteNodeError::new(RemoteNodeErrorKind::IdempotencyExpired)
        }
        super::RouteDecision::DedupCapacityExceeded => {
            RemoteNodeError::new(RemoteNodeErrorKind::DedupCapacityExceeded)
        }
        super::RouteDecision::InProgress { .. }
        | super::RouteDecision::Replay { .. }
        | super::RouteDecision::Accept { .. } => {
            RemoteNodeError::new(RemoteNodeErrorKind::UnsupportedOperation)
        }
    };
    err.with_correlation_id(req.correlation_id.clone())
}

fn required_remote_capability(kind: OperationKind) -> Result<Option<Capability>, RemoteNodeError> {
    match kind {
        OperationKind::NodeRegister
        | OperationKind::NodeHeartbeat
        | OperationKind::NodeCapabilities
        | OperationKind::WorkloadList
        | OperationKind::WorkloadStart
        | OperationKind::WorkloadStop
        | OperationKind::ExecStart
        | OperationKind::ExecAttach
        | OperationKind::ExecLogs
        | OperationKind::ExecCancel
        | OperationKind::ShellList
        | OperationKind::ShellAttach
        | OperationKind::ShellDetach
        | OperationKind::ShellKill
        | OperationKind::GuestHealth => Ok(kind.required_capability()),
        OperationKind::FileCopyStart
        | OperationKind::PortForwardOpen
        | OperationKind::DisplaySessionOpen => Err(RemoteNodeError::new(
            RemoteNodeErrorKind::UnsupportedOperation,
        )),
    }
}

fn remote_operation_requires_workload(kind: OperationKind) -> bool {
    matches!(
        kind,
        OperationKind::WorkloadStart
            | OperationKind::WorkloadStop
            | OperationKind::ExecStart
            | OperationKind::ExecAttach
            | OperationKind::ExecLogs
            | OperationKind::ExecCancel
            | OperationKind::ShellList
            | OperationKind::ShellAttach
            | OperationKind::ShellDetach
            | OperationKind::ShellKill
    )
}

/// Validate remote execution generation before opening a stream.
pub fn ensure_remote_execution_generation(
    route: &RemoteRoute,
    generation: &ExecutionGeneration,
) -> Result<(), RemoteNodeError> {
    if generation.guest_boot_id == route.generation {
        Ok(())
    } else {
        Err(RemoteNodeError::new(RemoteNodeErrorKind::StaleGeneration))
    }
}

/// Validate remote shell generation before opening a shell PTY stream.
pub fn ensure_remote_shell_generation(
    route: &RemoteRoute,
    generation: &ShellGeneration,
) -> Result<(), RemoteNodeError> {
    if generation.guest_boot_id == route.generation {
        Ok(())
    } else {
        Err(RemoteNodeError::new(RemoteNodeErrorKind::StaleGeneration))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_realm_core::{IdempotencyKey, OpaquePayload, OperationId, WorkloadId};

    fn instant(offset: u64) -> Instant {
        Instant::now() + Duration::from_secs(offset)
    }

    fn principal() -> PrincipalId {
        PrincipalId::parse("gateway-principal").unwrap()
    }

    fn gateway_node() -> NodeId {
        NodeId::parse("gateway").unwrap()
    }

    fn full_host_summary(caps: CapabilitySet) -> NodeSummary {
        NodeSummary {
            id: NodeId::parse("remote-host").unwrap(),
            realm: RealmPath::local(),
            kind: NodeKind::FullHost,
            capabilities: caps,
        }
    }

    fn registration(generation: &str, caps: CapabilitySet) -> RemoteNodeRegistration {
        RemoteNodeRegistration {
            summary: full_host_summary(caps),
            gateway_principal: principal(),
            gateway_node: gateway_node(),
            substrate_adapter: ProviderId::parse("nixos-host-substrate").unwrap(),
            generation: ProtocolToken::parse(generation).unwrap(),
        }
    }

    fn req(kind: OperationKind, capability: Capability, key: Option<&str>) -> OperationRequest {
        req_with_trace(kind, capability, key, None)
    }

    fn req_without_workload(
        kind: OperationKind,
        capability: Capability,
        key: Option<&str>,
    ) -> OperationRequest {
        let mut request = req(kind, capability, key);
        request.workload = None;
        request
    }

    fn req_with_trace(
        kind: OperationKind,
        capability: Capability,
        key: Option<&str>,
        trace: Option<TraceContext>,
    ) -> OperationRequest {
        OperationRequest {
            operation_id: OperationId::parse("op-1").unwrap(),
            correlation_id: CorrelationId::parse("corr-1").unwrap(),
            idempotency_key: key.map(|raw| IdempotencyKey::parse(raw).unwrap()),
            realm: RealmPath::local(),
            node: NodeId::parse("remote-host").unwrap(),
            workload: Some(WorkloadId::parse("vm-a").unwrap()),
            principal: principal(),
            kind,
            trace,
            body: OpaquePayload::new(capability.code().as_bytes().to_vec()).unwrap(),
        }
    }

    #[test]
    fn full_host_registers_with_bounded_fingerprint() {
        let caps = CapabilitySet::empty().with(Capability::Lifecycle);
        let mut registry = RemoteNodeRegistry::new();
        let entry = registry
            .register(registration("gen-1", caps.clone()), instant(0))
            .unwrap();
        assert_eq!(entry.summary.kind, NodeKind::FullHost);
        assert_eq!(entry.capability_fingerprint, caps.stable_fingerprint());
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn provider_managed_node_cannot_register_as_full_host() {
        let mut reg = registration("gen-1", CapabilitySet::empty());
        reg.summary.kind = NodeKind::ProviderManaged;
        let err = RemoteNodeRegistry::new()
            .register(reg, instant(0))
            .unwrap_err();
        assert_eq!(err.kind, RemoteNodeErrorKind::NotFullHost);
        assert_eq!(err.code(), "not-full-host");
    }

    #[test]
    fn gateway_identity_cannot_be_the_registered_full_host() {
        let mut reg = registration("gen-1", CapabilitySet::empty());
        reg.gateway_node = reg.summary.id.clone();
        let err = RemoteNodeRegistry::new()
            .register(reg, instant(0))
            .unwrap_err();
        assert_eq!(err.kind, RemoteNodeErrorKind::UnauthorizedGateway);
        assert_eq!(err.kind.error_kind(), ErrorKind::Unauthorized);
    }

    #[test]
    fn conflicting_duplicate_registration_fails() {
        let mut registry = RemoteNodeRegistry::new();
        let caps = CapabilitySet::empty().with(Capability::Lifecycle);
        registry
            .register(registration("gen-1", caps.clone()), instant(0))
            .unwrap();
        let mut conflicting = registration("gen-1", caps);
        conflicting.gateway_principal = PrincipalId::parse("other-gateway").unwrap();
        let err = registry.register(conflicting, instant(1)).unwrap_err();
        assert_eq!(err.kind, RemoteNodeErrorKind::DuplicateRegistration);
    }

    #[test]
    fn wrong_realm_registration_fails_before_state_is_retained() {
        let mut reg = registration("gen-1", CapabilitySet::empty());
        reg.summary.realm =
            RealmPath::new(vec![d2b_realm_core::RealmId::parse("work").unwrap()]).unwrap();
        let mut registry = RemoteNodeRegistry::new();
        let err = registry.register(reg, instant(0)).unwrap_err();
        assert_eq!(err.kind, RemoteNodeErrorKind::WrongRealm);
        assert!(registry.is_empty());
    }

    #[test]
    fn new_generation_supersedes_old_and_old_generation_is_stale() {
        let mut registry = RemoteNodeRegistry::new();
        let caps = CapabilitySet::empty().with(Capability::Lifecycle);
        registry
            .register(registration("gen-1", caps.clone()), instant(0))
            .unwrap();
        let entry = registry
            .register(registration("gen-2", caps), instant(1))
            .unwrap();
        assert_eq!(entry.generation, ProtocolToken::parse("gen-2").unwrap());

        let old = ProtocolToken::parse("gen-1").unwrap();
        let request = req(
            OperationKind::WorkloadStart,
            Capability::Lifecycle,
            Some("idem-1"),
        );
        let err = registry
            .prepare_route(&request, &old, &principal())
            .unwrap_err();
        assert_eq!(err.kind, RemoteNodeErrorKind::StaleGeneration);
    }

    #[test]
    fn wrong_realm_route_fails_before_lookup() {
        let registry = RemoteNodeRegistry::new();
        let mut request = req(OperationKind::WorkloadList, Capability::Lifecycle, None);
        request.realm =
            RealmPath::new(vec![d2b_realm_core::RealmId::parse("work").unwrap()]).unwrap();
        let err = registry
            .prepare_route(
                &request,
                &ProtocolToken::parse("gen-1").unwrap(),
                &principal(),
            )
            .unwrap_err();
        assert_eq!(err.kind, RemoteNodeErrorKind::WrongRealm);
    }

    #[test]
    fn heartbeat_and_disconnect_drive_availability() {
        let mut registry = RemoteNodeRegistry::new().with_heartbeat_timeout(Duration::from_secs(2));
        let caps = CapabilitySet::empty().with(Capability::Lifecycle);
        registry
            .register(registration("gen-1", caps), instant(0))
            .unwrap();
        let generation = ProtocolToken::parse("gen-1").unwrap();
        let node = NodeId::parse("remote-host").unwrap();
        registry
            .heartbeat(&RealmPath::local(), &node, &generation, instant(1))
            .unwrap();
        assert_eq!(
            registry
                .get(&RealmPath::local(), &node)
                .unwrap()
                .availability(),
            RemoteNodeAvailability::Available
        );
        assert!(registry.peer_disconnected(&RealmPath::local(), &node));
        assert_eq!(
            registry
                .get(&RealmPath::local(), &node)
                .unwrap()
                .availability(),
            RemoteNodeAvailability::Disconnected
        );
        let err = registry
            .heartbeat(&RealmPath::local(), &node, &generation, instant(2))
            .unwrap_err();
        assert_eq!(err.kind, RemoteNodeErrorKind::NodeUnavailable);
        registry
            .register(
                registration("gen-1", CapabilitySet::empty().with(Capability::Lifecycle)),
                instant(3),
            )
            .unwrap();
        registry.expire_heartbeats(instant(10));
        assert_eq!(
            registry
                .get(&RealmPath::local(), &node)
                .unwrap()
                .availability(),
            RemoteNodeAvailability::StaleHeartbeat
        );
    }

    #[test]
    fn unavailable_node_refuses_before_route() {
        let mut registry = RemoteNodeRegistry::new();
        let caps = CapabilitySet::empty().with(Capability::Lifecycle);
        registry
            .register(registration("gen-1", caps), instant(0))
            .unwrap();
        let node = NodeId::parse("remote-host").unwrap();
        registry.peer_disconnected(&RealmPath::local(), &node);
        let request = req(OperationKind::WorkloadList, Capability::Lifecycle, None);
        let err = registry
            .prepare_route(
                &request,
                &ProtocolToken::parse("gen-1").unwrap(),
                &principal(),
            )
            .unwrap_err();
        assert_eq!(err.kind, RemoteNodeErrorKind::NodeUnavailable);
        assert!(err.remediation().contains("remote daemon"));
    }

    #[test]
    fn missing_capability_denies_before_remote_send() {
        let mut registry = RemoteNodeRegistry::new();
        registry
            .register(registration("gen-1", CapabilitySet::empty()), instant(0))
            .unwrap();
        let request = req(
            OperationKind::WorkloadStart,
            Capability::Lifecycle,
            Some("idem-1"),
        );
        let err = registry
            .prepare_route(
                &request,
                &ProtocolToken::parse("gen-1").unwrap(),
                &principal(),
            )
            .unwrap_err();
        assert_eq!(err.kind, RemoteNodeErrorKind::CapabilityDenied);
        assert_eq!(err.missing_capability, Some(Capability::Lifecycle));
        assert!(err.capability_fingerprint.is_some());
        assert_eq!(
            err.correlation_id.as_ref().map(CorrelationId::as_str),
            Some("corr-1")
        );
    }

    #[test]
    fn remote_operation_maps_existing_kind_and_capability() {
        let mut registry = RemoteNodeRegistry::new();
        let caps = CapabilitySet::empty()
            .with(Capability::Lifecycle)
            .with(Capability::Exec)
            .with(Capability::Logs);
        registry
            .register(registration("gen-1", caps.clone()), instant(0))
            .unwrap();
        for (kind, cap) in [
            (OperationKind::WorkloadList, Some(Capability::Lifecycle)),
            (OperationKind::WorkloadStart, Some(Capability::Lifecycle)),
            (OperationKind::WorkloadStop, Some(Capability::Lifecycle)),
            (OperationKind::ExecStart, Some(Capability::Exec)),
            (OperationKind::ExecAttach, Some(Capability::Exec)),
            (OperationKind::ExecCancel, Some(Capability::Exec)),
            (OperationKind::ExecLogs, Some(Capability::Logs)),
        ] {
            let request = req(kind, cap.unwrap_or(Capability::Lifecycle), Some("idem-1"));
            let route = registry
                .prepare_route(
                    &request,
                    &ProtocolToken::parse("gen-1").unwrap(),
                    &principal(),
                )
                .unwrap();
            assert_eq!(route.operation, kind);
            assert_eq!(route.required_capability, cap);
            assert_eq!(route.capability_fingerprint, caps.stable_fingerprint());
        }
    }

    #[test]
    fn remote_shell_operations_require_persistent_shell_capability() {
        let mut registry = RemoteNodeRegistry::new();
        let caps = CapabilitySet::empty().with(Capability::PersistentShell);
        registry
            .register(registration("gen-1", caps.clone()), instant(0))
            .unwrap();
        for (kind, key, mutating) in [
            (OperationKind::ShellList, None, false),
            (OperationKind::ShellAttach, Some("shell-attach-1"), true),
            (OperationKind::ShellDetach, Some("shell-detach-1"), true),
            (OperationKind::ShellKill, Some("shell-kill-1"), true),
        ] {
            let request = req(kind, Capability::PersistentShell, key);
            let route = registry
                .prepare_route(
                    &request,
                    &ProtocolToken::parse("gen-1").unwrap(),
                    &principal(),
                )
                .unwrap();
            assert_eq!(route.operation, kind);
            assert_eq!(route.required_capability, Some(Capability::PersistentShell));
            assert_eq!(route.mutating, mutating);
            assert_eq!(route.capability_fingerprint, caps.stable_fingerprint());
        }
    }

    #[test]
    fn remote_shell_capability_denied_when_node_does_not_advertise_it() {
        let mut registry = RemoteNodeRegistry::new();
        registry
            .register(
                registration("gen-1", CapabilitySet::empty().with(Capability::Exec)),
                instant(0),
            )
            .unwrap();
        let request = req(
            OperationKind::ShellAttach,
            Capability::PersistentShell,
            Some("shell-attach-1"),
        );
        let err = registry
            .prepare_route(
                &request,
                &ProtocolToken::parse("gen-1").unwrap(),
                &principal(),
            )
            .unwrap_err();
        assert_eq!(err.kind, RemoteNodeErrorKind::CapabilityDenied);
        assert_eq!(err.missing_capability, Some(Capability::PersistentShell));
    }

    #[test]
    fn remote_shell_operations_require_workload_targets() {
        let mut registry = RemoteNodeRegistry::new();
        registry
            .register(
                registration(
                    "gen-1",
                    CapabilitySet::empty().with(Capability::PersistentShell),
                ),
                instant(0),
            )
            .unwrap();
        for (kind, key) in [
            (OperationKind::ShellList, None),
            (OperationKind::ShellAttach, Some("shell-attach-1")),
            (OperationKind::ShellDetach, Some("shell-detach-1")),
            (OperationKind::ShellKill, Some("shell-kill-1")),
        ] {
            let request = req_without_workload(kind, Capability::PersistentShell, key);
            let err = registry
                .prepare_route(
                    &request,
                    &ProtocolToken::parse("gen-1").unwrap(),
                    &principal(),
                )
                .unwrap_err();
            assert_eq!(err.kind, RemoteNodeErrorKind::MissingWorkload);
        }
    }

    #[test]
    fn unsupported_remote_operation_is_refused() {
        let mut registry = RemoteNodeRegistry::new();
        registry
            .register(
                registration(
                    "gen-1",
                    CapabilitySet::empty().with(Capability::PortForward),
                ),
                instant(0),
            )
            .unwrap();
        let request = req(
            OperationKind::PortForwardOpen,
            Capability::PortForward,
            Some("idem-1"),
        );
        let err = registry
            .prepare_route(
                &request,
                &ProtocolToken::parse("gen-1").unwrap(),
                &principal(),
            )
            .unwrap_err();
        assert_eq!(err.kind, RemoteNodeErrorKind::UnsupportedOperation);
    }

    #[test]
    fn unregister_allows_capacity_recovery() {
        let caps = CapabilitySet::empty().with(Capability::Lifecycle);
        let mut registry = RemoteNodeRegistry::new().with_max_nodes(1);
        registry
            .register(registration("gen-1", caps.clone()), instant(0))
            .unwrap();
        let mut second = registration("gen-1", caps.clone());
        second.summary.id = NodeId::parse("other-host").unwrap();
        assert_eq!(
            registry
                .register(second.clone(), instant(1))
                .unwrap_err()
                .kind,
            RemoteNodeErrorKind::RegistryCapacityExceeded
        );
        assert!(
            registry
                .unregister(&RealmPath::local(), &NodeId::parse("remote-host").unwrap())
                .is_some()
        );
        assert!(registry.register(second, instant(2)).is_ok());
    }

    #[test]
    fn mutating_disconnect_recovery_queries_remote_state() {
        let start = req(
            OperationKind::WorkloadStart,
            Capability::Lifecycle,
            Some("idem-1"),
        );
        assert_eq!(
            retry_action_after_disconnect(&start),
            RemoteRetryAction::QueryRemoteState
        );
        let list = req(OperationKind::WorkloadList, Capability::Lifecycle, None);
        assert_eq!(
            retry_action_after_disconnect(&list),
            RemoteRetryAction::NoSideEffectToRetry
        );
        let attach = req(
            OperationKind::ShellAttach,
            Capability::PersistentShell,
            Some("shell-attach-1"),
        );
        assert_eq!(
            retry_action_after_disconnect(&attach),
            RemoteRetryAction::QueryRemoteState
        );
        let shell_list = req(OperationKind::ShellList, Capability::PersistentShell, None);
        assert_eq!(
            retry_action_after_disconnect(&shell_list),
            RemoteRetryAction::NoSideEffectToRetry
        );
    }

    #[test]
    fn remote_execution_generation_is_checked_before_stream_open() {
        let route = RemoteRoute {
            realm: RealmPath::local(),
            node: NodeId::parse("remote-host").unwrap(),
            generation: ProtocolToken::parse("boot-a").unwrap(),
            operation: OperationKind::ExecAttach,
            correlation_id: CorrelationId::parse("corr-1").unwrap(),
            required_capability: Some(Capability::Exec),
            capability_fingerprint: CapabilitySet::empty()
                .with(Capability::Exec)
                .stable_fingerprint(),
            principal: principal(),
            trace: None,
            mutating: false,
        };
        let good = ExecutionGeneration {
            guest_boot_id: ProtocolToken::parse("boot-a").unwrap(),
            workload_generation: ProtocolToken::parse("workload-gen").unwrap(),
        };
        assert!(ensure_remote_execution_generation(&route, &good).is_ok());
        let stale = ExecutionGeneration {
            guest_boot_id: ProtocolToken::parse("boot-b").unwrap(),
            workload_generation: ProtocolToken::parse("workload-gen").unwrap(),
        };
        let err = ensure_remote_execution_generation(&route, &stale).unwrap_err();
        assert_eq!(err.kind, RemoteNodeErrorKind::StaleGeneration);
    }

    #[test]
    fn remote_shell_generation_is_checked_before_shell_pty_open() {
        let route = RemoteRoute {
            realm: RealmPath::local(),
            node: NodeId::parse("remote-host").unwrap(),
            generation: ProtocolToken::parse("boot-a").unwrap(),
            operation: OperationKind::ShellAttach,
            correlation_id: CorrelationId::parse("corr-1").unwrap(),
            required_capability: Some(Capability::PersistentShell),
            capability_fingerprint: CapabilitySet::empty()
                .with(Capability::PersistentShell)
                .stable_fingerprint(),
            principal: principal(),
            trace: None,
            mutating: true,
        };
        let good = ShellGeneration {
            guest_boot_id: ProtocolToken::parse("boot-a").unwrap(),
            guestd_instance_id: ProtocolToken::parse("guestd-a").unwrap(),
            shell_daemon_instance_id: ProtocolToken::parse("shell-a").unwrap(),
        };
        assert!(ensure_remote_shell_generation(&route, &good).is_ok());
        let stale = ShellGeneration {
            guest_boot_id: ProtocolToken::parse("boot-b").unwrap(),
            guestd_instance_id: ProtocolToken::parse("guestd-a").unwrap(),
            shell_daemon_instance_id: ProtocolToken::parse("shell-a").unwrap(),
        };
        let err = ensure_remote_shell_generation(&route, &stale).unwrap_err();
        assert_eq!(err.kind, RemoteNodeErrorKind::StaleGeneration);
    }

    #[test]
    fn debug_output_does_not_expose_forbidden_shapes() {
        let mut registry = RemoteNodeRegistry::new();
        registry
            .register(
                registration("gen-1", CapabilitySet::empty().with(Capability::Lifecycle)),
                instant(0),
            )
            .unwrap();
        let debug = format!("{registry:?}");
        for forbidden in [
            "TOKEN=",
            "secret",
            "/nix/store",
            "stdout",
            "pidfd",
            "fd=",
            "https://",
            "ssh://",
        ] {
            assert!(
                !debug.contains(forbidden),
                "debug output leaked {forbidden}: {debug}"
            );
        }
    }

    #[derive(Default)]
    struct FakePeer {
        sends: usize,
        queries: usize,
        query_status: Option<RemotePeerStatus>,
        fail_send: bool,
        fail_query: bool,
    }

    impl RemotePeerClient for FakePeer {
        fn send_operation(
            &mut self,
            _route: &RemoteRoute,
            _req: &OperationRequest,
        ) -> Result<OpaquePayload, RemoteNodeError> {
            self.sends += 1;
            if self.fail_send {
                return Err(RemoteNodeError::new(RemoteNodeErrorKind::NodeUnavailable));
            }
            Ok(OpaquePayload::new(b"remote-ok".to_vec()).unwrap())
        }

        fn query_operation(
            &mut self,
            _route: &RemoteRoute,
            _req: &OperationRequest,
        ) -> Result<RemotePeerStatus, RemoteNodeError> {
            self.queries += 1;
            if self.fail_query {
                return Err(RemoteNodeError::new(RemoteNodeErrorKind::NodeUnavailable));
            }
            Ok(self
                .query_status
                .clone()
                .unwrap_or(RemotePeerStatus::InProgress))
        }
    }

    fn adapter(caps: CapabilitySet) -> RemoteFullHostAdapter {
        let mut adapter = RemoteFullHostAdapter::new(principal(), caps.clone());
        adapter
            .registry_mut()
            .register(registration("gen-1", caps), instant(0))
            .unwrap();
        adapter
    }

    #[test]
    fn gateway_adapter_sends_only_after_router_and_registry_accept() {
        let caps = CapabilitySet::empty().with(Capability::Lifecycle);
        let mut adapter = adapter(caps);
        let mut peer = FakePeer::default();
        let request = req(
            OperationKind::WorkloadStart,
            Capability::Lifecycle,
            Some("idem-1"),
        );
        let outcome = adapter
            .dispatch(&request, &ProtocolToken::parse("gen-1").unwrap(), &mut peer)
            .unwrap();
        assert!(matches!(outcome, RemoteDispatchOutcome::Sent { .. }));
        assert_eq!(peer.sends, 1);
    }

    #[test]
    fn gateway_adapter_replays_without_second_send() {
        let caps = CapabilitySet::empty().with(Capability::Lifecycle);
        let mut adapter = adapter(caps);
        let mut peer = FakePeer::default();
        let request = req(
            OperationKind::WorkloadStart,
            Capability::Lifecycle,
            Some("idem-1"),
        );
        adapter
            .dispatch(&request, &ProtocolToken::parse("gen-1").unwrap(), &mut peer)
            .unwrap();
        let outcome = adapter
            .dispatch(&request, &ProtocolToken::parse("gen-1").unwrap(), &mut peer)
            .unwrap();
        assert!(matches!(outcome, RemoteDispatchOutcome::Replayed { .. }));
        assert_eq!(peer.sends, 1);
        assert_eq!(peer.queries, 0);
    }

    #[test]
    fn missing_negotiated_capability_denies_before_remote_send() {
        let mut adapter = adapter(CapabilitySet::empty().with(Capability::Lifecycle));
        adapter.negotiated_capabilities = CapabilitySet::empty();
        let mut peer = FakePeer::default();
        let request = req(
            OperationKind::WorkloadStart,
            Capability::Lifecycle,
            Some("idem-1"),
        );
        let err = adapter
            .dispatch(&request, &ProtocolToken::parse("gen-1").unwrap(), &mut peer)
            .unwrap_err();
        assert_eq!(err.kind, RemoteNodeErrorKind::CapabilityDenied);
        assert_eq!(
            err.correlation_id.as_ref().map(CorrelationId::as_str),
            Some("corr-1")
        );
        assert_eq!(peer.sends, 0);
    }

    #[test]
    fn shell_mutations_use_remote_idempotency_and_shell_list_does_not() {
        let caps = CapabilitySet::empty().with(Capability::PersistentShell);
        let mut adapter = adapter(caps);
        let mut peer = FakePeer::default();

        let list = req(OperationKind::ShellList, Capability::PersistentShell, None);
        let outcome = adapter
            .dispatch(&list, &ProtocolToken::parse("gen-1").unwrap(), &mut peer)
            .unwrap();
        assert!(matches!(outcome, RemoteDispatchOutcome::Sent { .. }));
        assert_eq!(peer.sends, 1);

        let missing_key = req(
            OperationKind::ShellAttach,
            Capability::PersistentShell,
            None,
        );
        let err = adapter
            .dispatch(
                &missing_key,
                &ProtocolToken::parse("gen-1").unwrap(),
                &mut peer,
            )
            .unwrap_err();
        assert_eq!(err.kind, RemoteNodeErrorKind::MissingIdempotencyKey);

        for (kind, key) in [
            (OperationKind::ShellAttach, "shell-attach-1"),
            (OperationKind::ShellDetach, "shell-detach-1"),
            (OperationKind::ShellKill, "shell-kill-1"),
        ] {
            let request = req(kind, Capability::PersistentShell, Some(key));
            let outcome = adapter
                .dispatch(&request, &ProtocolToken::parse("gen-1").unwrap(), &mut peer)
                .unwrap();
            assert!(matches!(outcome, RemoteDispatchOutcome::Sent { .. }));
            let replay = adapter
                .dispatch(&request, &ProtocolToken::parse("gen-1").unwrap(), &mut peer)
                .unwrap();
            assert!(matches!(replay, RemoteDispatchOutcome::Replayed { .. }));
        }
        assert_eq!(peer.sends, 4);
        assert_eq!(peer.queries, 0);
    }

    #[test]
    fn gateway_principal_must_match_remote_registration() {
        let caps = CapabilitySet::empty().with(Capability::Lifecycle);
        let mut adapter = adapter(caps);
        adapter.gateway_principal = PrincipalId::parse("other-gateway").unwrap();
        let mut peer = FakePeer::default();
        let request = req(
            OperationKind::WorkloadStart,
            Capability::Lifecycle,
            Some("idem-1"),
        );
        let err = adapter
            .dispatch(&request, &ProtocolToken::parse("gen-1").unwrap(), &mut peer)
            .unwrap_err();
        assert_eq!(err.kind, RemoteNodeErrorKind::UnauthorizedGateway);
        assert_eq!(peer.sends, 0);
    }

    #[test]
    fn unacknowledged_mutation_queries_remote_state_before_retry() {
        let caps = CapabilitySet::empty().with(Capability::Lifecycle);
        let mut adapter = RemoteFullHostAdapter::with_router(
            RemoteNodeRegistry::new(),
            super::super::OperationRouter::new(),
            principal(),
            caps.clone(),
        );
        adapter
            .registry_mut()
            .register(registration("gen-1", caps), instant(0))
            .unwrap();
        let request = req(
            OperationKind::WorkloadStart,
            Capability::Lifecycle,
            Some("idem-1"),
        );
        assert!(matches!(
            adapter.router.route_with_capabilities(
                &request,
                &principal(),
                &adapter.negotiated_capabilities
            ),
            super::super::RouteDecision::Accept { .. }
        ));

        let mut peer = FakePeer::default();
        let outcome = adapter
            .dispatch(&request, &ProtocolToken::parse("gen-1").unwrap(), &mut peer)
            .unwrap();
        assert!(matches!(
            outcome,
            RemoteDispatchOutcome::QueryRemoteState { .. }
        ));
        assert_eq!(peer.sends, 0);
        assert_eq!(peer.queries, 1);
    }

    #[test]
    fn unknown_remote_state_clears_local_lock_and_returns_typed_error() {
        let caps = CapabilitySet::empty().with(Capability::Lifecycle);
        let mut adapter = RemoteFullHostAdapter::with_router(
            RemoteNodeRegistry::new(),
            super::super::OperationRouter::new(),
            principal(),
            caps.clone(),
        );
        adapter
            .registry_mut()
            .register(registration("gen-1", caps), instant(0))
            .unwrap();
        let request = req(
            OperationKind::WorkloadStart,
            Capability::Lifecycle,
            Some("idem-1"),
        );
        assert!(matches!(
            adapter.router.route_with_capabilities(
                &request,
                &principal(),
                &adapter.negotiated_capabilities
            ),
            super::super::RouteDecision::Accept { .. }
        ));
        let mut peer = FakePeer {
            query_status: Some(RemotePeerStatus::Unknown),
            ..Default::default()
        };
        let err = adapter
            .dispatch(&request, &ProtocolToken::parse("gen-1").unwrap(), &mut peer)
            .unwrap_err();
        assert_eq!(err.kind, RemoteNodeErrorKind::RemoteOperationUnknown);
        let mut next_peer = FakePeer::default();
        let outcome = adapter
            .dispatch(
                &request,
                &ProtocolToken::parse("gen-1").unwrap(),
                &mut next_peer,
            )
            .unwrap();
        assert!(matches!(outcome, RemoteDispatchOutcome::Sent { .. }));
        assert_eq!(next_peer.sends, 1);
    }

    #[test]
    fn send_error_clears_local_idempotency_lock() {
        let caps = CapabilitySet::empty().with(Capability::Lifecycle);
        let mut adapter = adapter(caps);
        let request = req(
            OperationKind::WorkloadStart,
            Capability::Lifecycle,
            Some("idem-1"),
        );
        let mut failing_peer = FakePeer {
            fail_send: true,
            ..Default::default()
        };
        let err = adapter
            .dispatch(
                &request,
                &ProtocolToken::parse("gen-1").unwrap(),
                &mut failing_peer,
            )
            .unwrap_err();
        assert_eq!(err.kind, RemoteNodeErrorKind::NodeUnavailable);

        let mut next_peer = FakePeer::default();
        let outcome = adapter
            .dispatch(
                &request,
                &ProtocolToken::parse("gen-1").unwrap(),
                &mut next_peer,
            )
            .unwrap();
        assert!(matches!(outcome, RemoteDispatchOutcome::Sent { .. }));
        assert_eq!(next_peer.sends, 1);
    }

    #[test]
    fn query_error_clears_local_idempotency_lock() {
        let caps = CapabilitySet::empty().with(Capability::Lifecycle);
        let mut adapter = RemoteFullHostAdapter::with_router(
            RemoteNodeRegistry::new(),
            super::super::OperationRouter::new(),
            principal(),
            caps.clone(),
        );
        adapter
            .registry_mut()
            .register(registration("gen-1", caps), instant(0))
            .unwrap();
        let request = req(
            OperationKind::WorkloadStart,
            Capability::Lifecycle,
            Some("idem-1"),
        );
        assert!(matches!(
            adapter.router.route_with_capabilities(
                &request,
                &principal(),
                &adapter.negotiated_capabilities
            ),
            super::super::RouteDecision::Accept { .. }
        ));

        let mut failing_peer = FakePeer {
            fail_query: true,
            ..Default::default()
        };
        let err = adapter
            .dispatch(
                &request,
                &ProtocolToken::parse("gen-1").unwrap(),
                &mut failing_peer,
            )
            .unwrap_err();
        assert_eq!(err.kind, RemoteNodeErrorKind::NodeUnavailable);

        let mut next_peer = FakePeer::default();
        let outcome = adapter
            .dispatch(
                &request,
                &ProtocolToken::parse("gen-1").unwrap(),
                &mut next_peer,
            )
            .unwrap();
        assert!(matches!(outcome, RemoteDispatchOutcome::Sent { .. }));
        assert_eq!(next_peer.sends, 1);
    }

    #[test]
    fn route_audit_labels_propagate_trace_context() {
        let mut registry = RemoteNodeRegistry::new();
        let caps = CapabilitySet::empty().with(Capability::Lifecycle);
        registry
            .register(registration("gen-1", caps.clone()), instant(0))
            .unwrap();
        let trace = TraceContext::new("trace-1", "span-1").unwrap();
        let request = req_with_trace(
            OperationKind::WorkloadStart,
            Capability::Lifecycle,
            Some("idem-1"),
            Some(trace),
        );
        let route = registry
            .prepare_route(
                &request,
                &ProtocolToken::parse("gen-1").unwrap(),
                &principal(),
            )
            .unwrap();
        let labels = route.audit_labels("accepted");
        assert_eq!(labels.realm, "local");
        assert_eq!(labels.node, "remote-host");
        assert_eq!(labels.correlation_id, "corr-1");
        assert_eq!(labels.principal, "gateway-principal");
        assert_eq!(labels.capability_fingerprint, caps.stable_fingerprint());
        assert_eq!(labels.trace_id, Some("trace-1"));
        assert_eq!(labels.span_id, Some("span-1"));
        assert_eq!(labels.outcome, "accepted");
    }
}
