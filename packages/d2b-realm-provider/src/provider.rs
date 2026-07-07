//! The v2 provider trait surface (ADR 0032). All provider/transport
//! traits are `async` via `async_trait` — Azure Relay, QUIC, SSH,
//! provider APIs, remote daemon sessions, and stream muxing must never
//! block the daemon reactor. Sync wrappers over blocking host code must
//! use `spawn_blocking`/dedicated threads (the no-blocking gate).

use async_trait::async_trait;
use d2b_realm_core::{
    ConstellationError, ConstellationFrame, NodeId, ProviderId, StreamOpen, WorkloadId,
    WorkloadSummary,
};
use serde::{Deserialize, Serialize};

use crate::capabilities::{
    DisplayCapabilitySet, NodeCapabilitySet, RuntimeCapabilitySet, WorkloadCapabilitySet,
};
use crate::error::ProviderResult;
use crate::types::{
    DaemonAccessMode, DisplaySessionHandle, DisplaySessionId, DisplaySessionRequest,
    ExecStartRequest, GuestControlEndpointStatus, IncomingStream, ListSelector, NodeRegistration,
    PersistentShellAttachProviderRequest, PersistentShellAttachProviderResponse,
    PersistentShellDetachProviderRequest, PersistentShellKillProviderRequest,
    PersistentShellListProviderRequest, PersistentShellListProviderResponse, PersistentShellStatus,
    RuntimeHandle, RuntimePlan, RuntimeStatus, StreamHandle, TransportSession, TransportTarget,
    WorkloadSpec, WorkloadStatus,
};

/// Installs/checks/prepares d2b on a host OS (NixOS, Ubuntu, generic
/// Linux). Reports host capabilities and typed remediation.
#[async_trait]
pub trait HostSubstrateProvider: Send + Sync {
    /// Provider id.
    fn provider_id(&self) -> ProviderId;
    /// Check host prerequisites (kernel, cgroup v2, KVM, userns, …).
    async fn check(&self) -> ProviderResult<NodeCapabilitySet>;
}

/// Runs a local workload on a full d2b host (Cloud Hypervisor,
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
///
/// Workloads do NOT own mux stream lifecycle: streams are opened/accepted
/// by the [`StreamMux`] against an already-authorized [`StreamOpen`]. A
/// workload that presents streams binds to those authorized substreams;
/// it never authorizes a stream itself.
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
    async fn exec(
        &self,
        req: ExecStartRequest,
    ) -> ProviderResult<d2b_realm_core::ExecutionId>;
}

/// Durable execution adapter over an already-addressed workload. This is the
/// constellation-facing seam for guest-control exec, provider-managed exec, or
/// future remote-node exec implementations; it carries only bounded metadata
/// and opaque stream/payload contracts from `d2b-realm-core`.
#[async_trait]
pub trait DurableExecutionProvider: Send + Sync {
    /// Provider id.
    fn provider_id(&self) -> ProviderId;
    /// Start or rediscover a durable execution.
    async fn start(
        &self,
        req: d2b_realm_core::ExecStartRequest,
    ) -> ProviderResult<d2b_realm_core::ExecutionSummary>;
    /// Attach or reconnect to a durable execution after generation validation.
    async fn attach(
        &self,
        req: d2b_realm_core::ExecAttachRequest,
    ) -> ProviderResult<d2b_realm_core::ExecutionSummary>;
    /// Fetch retained log metadata under a bounded request.
    async fn logs(
        &self,
        req: d2b_realm_core::ExecLogsRequest,
    ) -> ProviderResult<d2b_realm_core::ExecutionSummary>;
    /// Cancel an execution idempotently. `false` means it was already terminal
    /// or unknown to this provider scope.
    async fn cancel(&self, req: d2b_realm_core::ExecCancelRequest) -> ProviderResult<bool>;
}

/// Discovery seam for provider-managed workloads that run a
/// guestd-compatible d2b agent. This reports bounded capability and
/// generation metadata only; it never returns relay URLs, sockets, vsock
/// coordinates, credentials, or raw guest-control frames.
#[async_trait]
pub trait GuestControlEndpointProvider: Send + Sync {
    /// Provider id.
    fn provider_id(&self) -> ProviderId;
    /// Node this provider serves.
    fn node_id(&self) -> NodeId;
    /// Advertised workload-agent capabilities.
    fn capabilities(&self) -> WorkloadCapabilitySet;
    /// Resolve non-secret guest-control agent metadata for one workload.
    async fn endpoint_status(
        &self,
        workload: WorkloadId,
    ) -> ProviderResult<GuestControlEndpointStatus>;
}

/// Persistent named shell operations for guestd-compatible provider-managed
/// workloads. This is deliberately separate from [`WorkloadProvider::exec`]
/// and [`DurableExecutionProvider`]: one-shot provider exec APIs do not
/// satisfy ADR 0039 shell persistence, attach/detach, generation, audit, or
/// shell-authorized PTY stream semantics.
#[async_trait]
pub trait PersistentShellProvider: Send + Sync {
    /// Provider id.
    fn provider_id(&self) -> ProviderId;
    /// Node this provider serves.
    fn node_id(&self) -> NodeId;
    /// Advertised workload-agent capabilities.
    fn capabilities(&self) -> WorkloadCapabilitySet;
    /// List persistent shells for a workload.
    async fn list_shells(
        &self,
        req: PersistentShellListProviderRequest,
    ) -> ProviderResult<PersistentShellListProviderResponse>;
    /// Attach to a persistent shell and bind the authorized shell PTY stream.
    async fn attach_shell(
        &self,
        req: PersistentShellAttachProviderRequest,
    ) -> ProviderResult<PersistentShellAttachProviderResponse>;
    /// Detach a shell attach handle without killing the named shell.
    async fn detach_shell(
        &self,
        req: PersistentShellDetachProviderRequest,
    ) -> ProviderResult<PersistentShellStatus>;
    /// Kill a named persistent shell.
    async fn kill_shell(
        &self,
        req: PersistentShellKillProviderRequest,
    ) -> ProviderResult<PersistentShellStatus>;
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

/// An accepted inbound transport session and the node that registered the
/// listener it arrived on.
#[async_trait]
pub trait TransportListener: Send + Sync {
    /// The node this listener is registered for.
    fn node(&self) -> NodeId;
    /// Accept the next inbound session (outbound-only relays still expose
    /// an accept path here after the rendezvous completes).
    async fn accept(&self) -> ProviderResult<TransportSession>;
}

/// Byte transport below the constellation peer session/mux. Sessions carry
/// real bidirectional bytes.
#[async_trait]
pub trait TransportProvider: Send + Sync {
    /// Transport id.
    fn transport_id(&self) -> ProviderId;
    /// Connect to a transport target (sender side).
    async fn connect(&self, target: TransportTarget) -> ProviderResult<TransportSession>;
    /// Listen for inbound rendezvous (listener side).
    async fn listen(
        &self,
        registration: NodeRegistration,
    ) -> ProviderResult<Box<dyn TransportListener>>;
}

/// Named-stream multiplexing over a transport session. The mux is the
/// single owner of stream open/accept/close lifecycle and capability
/// gating: it opens a stream only against an already-validated
/// [`StreamOpen`] whose authz capability matches the descriptor kind and
/// whose required capability is advertised.
///
/// The **router** owns issuing the `StreamOpen.operation_id` binding (it
/// ties the open to the single authorizing operation and its principal);
/// the mux does not re-authorize the principal — it enforces capability
/// consistency + advertisement and rejects everything else fail-closed.
#[async_trait]
pub trait StreamMux: Send + Sync {
    /// Open a named stream. Implementations MUST reject the open
    /// (`CapabilityDenied`) when `open.is_consistent()` is false or the
    /// peer does not advertise `open.authz.capability` (fail-closed).
    async fn open_stream(&self, open: StreamOpen) -> ProviderResult<StreamHandle>;
    /// Accept the next inbound stream (already authorized by the peer).
    async fn accept_stream(&self) -> ProviderResult<IncomingStream>;
    /// Close a stream by id.
    async fn close_stream(&self, id: d2b_realm_core::StreamId) -> ProviderResult<()>;
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

/// How the `d2b` CLI reaches a specific `d2bd` (local Unix, direct
/// mTLS/QUIC/WebSocket, relay-backed, or explicit SSH bootstrap). Only
/// [`DaemonAccessMode::LocalUnix`] is implemented today; other modes fail
/// closed with `UnsupportedFeature`.
#[async_trait]
pub trait DaemonAccessTransport: Send + Sync {
    /// Transport id.
    fn transport_id(&self) -> ProviderId;
    /// The access mode this transport implements.
    fn mode(&self) -> DaemonAccessMode;
    /// Open a daemon byte session to the endpoint.
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

/// Non-secret lifecycle status of a realm/node enrollment credential. It
/// never carries key material.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum CredentialStatus {
    /// A valid, unexpired, unrevoked enrollment.
    Valid,
    /// No enrollment is present.
    Absent,
    /// The enrollment has expired.
    Expired,
    /// The enrollment was revoked.
    Revoked,
}

impl CredentialStatus {
    /// Whether the enrollment is currently usable.
    pub fn is_valid(self) -> bool {
        matches!(self, CredentialStatus::Valid)
    }
}

/// Realm/node enrollment and relay/provider credential handling. Never
/// exposes plaintext credentials to the host; lifecycle/proof hooks return
/// only opaque, non-secret values.
#[async_trait]
pub trait CredentialProvider: Send + Sync {
    /// Provider id.
    fn provider_id(&self) -> ProviderId;
    /// The non-secret enrollment status.
    async fn status(&self) -> ProviderResult<CredentialStatus>;
    /// Convenience: whether an enrollment is currently valid.
    async fn enrollment_valid(&self) -> ProviderResult<bool> {
        Ok(self.status().await?.is_valid())
    }
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
    async fn open_listener(&self, node: NodeId) -> ProviderResult<Box<dyn TransportListener>>;
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
