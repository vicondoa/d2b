//! The v2 provider trait surface (ADR 0032). All provider/transport
//! traits are `async` via `async_trait` — Azure Relay, QUIC, SSH,
//! provider APIs, remote daemon sessions, and stream muxing must never
//! block the daemon reactor. Sync wrappers over blocking host code must
//! use `spawn_blocking`/dedicated threads (Wave 0 no-blocking gate).

use async_trait::async_trait;
use nixling_constellation_core::{
    ConstellationError, ConstellationFrame, ExecutionId, NodeId, ProviderId, WorkloadId,
    WorkloadSummary,
};

use crate::capabilities::{
    DisplayCapabilitySet, NodeCapabilitySet, RuntimeCapabilitySet, WorkloadCapabilitySet,
};
use crate::error::ProviderResult;
use crate::types::{
    DisplaySessionHandle, DisplaySessionId, DisplaySessionRequest, ExecStartRequest,
    IncomingStream, ListSelector, NodeRegistration, RuntimeHandle, RuntimePlan, RuntimeStatus,
    StreamHandle, StreamOpenRequest, TransportListener, TransportSession, TransportTarget,
    WorkloadSpec, WorkloadStatus,
};

/// Installs/checks/prepares nixling on a host OS (NixOS, Ubuntu, generic
/// Linux). Reports host capabilities and typed remediation.
#[async_trait]
pub trait HostSubstrateProvider: Send + Sync {
    /// Provider id.
    fn provider_id(&self) -> ProviderId;
    /// Check host prerequisites (kernel, cgroup v2, KVM, userns, …).
    async fn check(&self) -> ProviderResult<NodeCapabilitySet>;
}

/// Runs a local workload on a full nixling host (Cloud Hypervisor,
/// crosvm, …). Narrow and data-driven.
#[async_trait]
pub trait RuntimeProvider: Send + Sync {
    /// Provider id.
    fn provider_id(&self) -> ProviderId;
    /// Advertised runtime capabilities.
    fn capabilities(&self) -> RuntimeCapabilitySet;
    /// Resolve a workload spec into a runtime plan.
    async fn plan_workload(&self, spec: WorkloadSpec) -> ProviderResult<RuntimePlan>;
    /// Start a planned workload.
    async fn start(&self, plan: RuntimePlan) -> ProviderResult<RuntimeHandle>;
    /// Stop a running workload.
    async fn stop(&self, handle: RuntimeHandle) -> ProviderResult<()>;
    /// Inspect a running workload.
    async fn inspect(&self, handle: RuntimeHandle) -> ProviderResult<RuntimeStatus>;
}

/// Workload lifecycle when the isolation boundary is not the local host
/// runtime (e.g. Azure Container Apps sessions) or a local runtime-backed
/// microVM, behind one operation API.
#[async_trait]
pub trait WorkloadProvider: Send + Sync {
    /// Provider id.
    fn provider_id(&self) -> ProviderId;
    /// Node this provider serves.
    fn node_id(&self) -> NodeId;
    /// Advertised workload capabilities (positive assertions).
    fn capabilities(&self) -> WorkloadCapabilitySet;
    /// List workloads matching a selector.
    async fn list(&self, selector: ListSelector) -> ProviderResult<Vec<WorkloadSummary>>;
    /// Create a workload (mutating; idempotency owned by the caller/gateway).
    async fn create(&self, spec: WorkloadSpec) -> ProviderResult<WorkloadId>;
    /// Start a workload.
    async fn start(&self, id: WorkloadId) -> ProviderResult<WorkloadStatus>;
    /// Stop a workload.
    async fn stop(&self, id: WorkloadId) -> ProviderResult<WorkloadStatus>;
    /// Start an execution.
    async fn exec(&self, req: ExecStartRequest) -> ProviderResult<ExecutionId>;
    /// Open a named stream against the workload.
    async fn open_stream(&self, req: StreamOpenRequest) -> ProviderResult<StreamHandle>;
}

/// Window/display forwarding for workloads that can present UI. A provider
/// that cannot present windows returns a typed capability denial.
#[async_trait]
pub trait DisplayProvider: Send + Sync {
    /// Provider id.
    fn provider_id(&self) -> ProviderId;
    /// Advertised display capabilities (window-forwarding, SHM, dmabuf, …).
    fn capabilities(&self) -> DisplayCapabilitySet;
    /// Open a display session over an authorized `display` stream.
    async fn open_display_session(
        &self,
        req: DisplaySessionRequest,
    ) -> ProviderResult<DisplaySessionHandle>;
    /// Close a display session.
    async fn close_display_session(&self, id: DisplaySessionId) -> ProviderResult<()>;
}

/// Byte transport below the constellation peer session/mux.
#[async_trait]
pub trait TransportProvider: Send + Sync {
    /// Transport id.
    fn transport_id(&self) -> ProviderId;
    /// Connect to a transport target (sender side).
    async fn connect(&self, target: TransportTarget) -> ProviderResult<TransportSession>;
    /// Listen for inbound rendezvous (listener side).
    async fn listen(&self, registration: NodeRegistration)
        -> ProviderResult<TransportListener>;
}

/// Named-stream multiplexing over a transport session.
#[async_trait]
pub trait StreamMux: Send + Sync {
    /// Open a named stream with the given authorization context.
    async fn open_stream(&self, req: StreamOpenRequest) -> ProviderResult<StreamHandle>;
    /// Accept the next inbound stream.
    async fn accept_stream(&self) -> ProviderResult<IncomingStream>;
}

/// Encodes/decodes the semantic [`ConstellationFrame`]. The first codec is
/// protobuf; the operation layer never depends on the encoding.
pub trait ProtocolCodec: Send + Sync {
    /// Stable codec id negotiated in the handshake.
    fn codec_id(&self) -> &str;
    /// Encode a semantic frame to bytes.
    fn encode_frame(&self, frame: &ConstellationFrame) -> Result<Vec<u8>, ConstellationError>;
    /// Decode bytes to a semantic frame (fail-closed on unknown shapes).
    fn decode_frame(&self, bytes: &[u8]) -> Result<ConstellationFrame, ConstellationError>;
    /// A stable fingerprint of the codec's schema.
    fn schema_fingerprint(&self) -> String;
}

/// How the `nixling` CLI reaches a specific `nixlingd` (local Unix, direct
/// mTLS/QUIC/WebSocket, relay-backed, or explicit SSH bootstrap).
#[async_trait]
pub trait DaemonAccessTransport: Send + Sync {
    /// Transport id.
    fn transport_id(&self) -> ProviderId;
    /// Open a daemon connection label (opaque) to the endpoint.
    async fn connect(&self, endpoint: TransportTarget) -> ProviderResult<TransportSession>;
}

/// The transport-neutral CLI-facing daemon API surface.
#[async_trait]
pub trait DaemonAccessApi: Send + Sync {
    /// List local workloads (current CLI behavior over the local binding).
    async fn vm_list(&self) -> ProviderResult<Vec<WorkloadSummary>>;
}

/// Provision infrastructure and bootstrap nodes (separate from node
/// control).
#[async_trait]
pub trait InfrastructureProvider: Send + Sync {
    /// Provider id.
    fn provider_id(&self) -> ProviderId;
    /// Plan infrastructure for a node.
    async fn plan_infrastructure(&self, node: NodeId) -> ProviderResult<()>;
}

/// Realm/node enrollment and relay/provider credential handling. Never
/// exposes plaintext credentials to the host.
#[async_trait]
pub trait CredentialProvider: Send + Sync {
    /// Provider id.
    fn provider_id(&self) -> ProviderId;
    /// Whether an enrollment is currently valid (not expired/revoked).
    async fn enrollment_valid(&self) -> ProviderResult<bool>;
}

/// Observability export target (local, gateway, observer, or external).
#[async_trait]
pub trait ObservabilitySinkProvider: Send + Sync {
    /// Provider id.
    fn provider_id(&self) -> ProviderId;
    /// Whether the sink is currently reachable (for degraded handling).
    async fn healthy(&self) -> ProviderResult<bool>;
}

/// Rendezvous/listener/sender mechanics (e.g. Azure Relay Hybrid
/// Connections).
#[async_trait]
pub trait RelayProvider: Send + Sync {
    /// Provider id.
    fn provider_id(&self) -> ProviderId;
    /// Open the listener side (outbound-only).
    async fn open_listener(&self, node: NodeId) -> ProviderResult<TransportListener>;
}

/// A registered node that dispatches operations.
#[async_trait]
pub trait NodeProvider: Send + Sync {
    /// Node id.
    fn node_id(&self) -> NodeId;
    /// Advertised node capabilities.
    fn capabilities(&self) -> NodeCapabilitySet;
    /// List workloads on the node.
    async fn list_workloads(&self) -> ProviderResult<Vec<WorkloadSummary>>;
}
