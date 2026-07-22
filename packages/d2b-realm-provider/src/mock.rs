//! No-op / mock providers for conformance tests and local model
//! validation. The display + mux mocks return typed capability denials so
//! callers exercise fail-closed routing.
//!
//! This module also hosts the provider parity inventory/proof
//! ([`parity`]) as a nested module. It lives here (rather than as a
//! top-level `lib.rs` module) purely so the parity proof's stand-in
//! coverage of every mock defined below stays a single-file review
//! surface; `crate::mock::parity` is its public path.

use async_trait::async_trait;
use d2b_realm_core::{
    Capability, CapabilitySet, ConstellationError, ConstellationFrame, ErrorKind, ExecutionId,
    NodeId, ProviderId, StreamOpen, WorkloadId, WorkloadSummary,
};

use crate::capabilities::{
    DisplayCapabilitySet, HostSubstrateKind, NodeCapabilitySet, RuntimeCapabilitySet,
    WorkloadCapabilitySet,
};
use crate::error::{ProviderError, ProviderResult};
use crate::provider::{
    CredentialProvider, CredentialStatus, DaemonAccessApi, DaemonAccessTransport, DisplayProvider,
    DurableExecutionProvider, GuestControlEndpointProvider, HostSubstrateProvider,
    InfrastructureProvider, NodeProvider, ObservabilitySinkProvider, PersistentShellProvider,
    ProtocolCodec, RelayProvider, RuntimeProvider, StreamMux, TransportListener, TransportProvider,
    WorkloadProvider,
};
use crate::types::{
    DaemonAccessMode, DisplaySessionHandle, DisplaySessionId, DisplaySessionRequest,
    ExecStartRequest, GuestControlEndpointStatus, IncomingStream, ListSelector, NodeRegistration,
    PersistentShellAttachProviderRequest, PersistentShellAttachProviderResponse,
    PersistentShellDetachProviderRequest, PersistentShellKillProviderRequest,
    PersistentShellListProviderRequest, PersistentShellListProviderResponse, PersistentShellStatus,
    RuntimeHandle, RuntimePlan, RuntimeStatus, SafeLabel, StreamHandle, TransportSession,
    TransportTarget, WorkloadStatus,
};

#[path = "parity.rs"]
pub mod parity;

fn id(label: &str) -> ProviderId {
    ProviderId::parse(label).expect("mock label is valid")
}

/// A mock workload provider that records nothing and succeeds trivially.
#[derive(Debug, Clone)]
pub struct MockWorkloadProvider {
    provider: ProviderId,
    node: NodeId,
    caps: WorkloadCapabilitySet,
}

impl Default for MockWorkloadProvider {
    fn default() -> Self {
        Self {
            provider: id("mock"),
            node: NodeId::parse("mock").expect("valid"),
            caps: WorkloadCapabilitySet {
                caps: CapabilitySet::empty()
                    .with(Capability::Lifecycle)
                    .with(Capability::Exec),
            },
        }
    }
}

#[async_trait]
impl WorkloadProvider for MockWorkloadProvider {
    fn provider_id(&self) -> ProviderId {
        self.provider.clone()
    }
    fn node_id(&self) -> NodeId {
        self.node.clone()
    }
    fn capabilities(&self) -> WorkloadCapabilitySet {
        self.caps.clone()
    }
    async fn list(&self, _selector: ListSelector) -> ProviderResult<Vec<WorkloadSummary>> {
        Ok(vec![])
    }
    async fn create(&self, spec: crate::types::WorkloadSpec) -> ProviderResult<WorkloadId> {
        Ok(spec.alias)
    }
    async fn start(&self, id: WorkloadId) -> ProviderResult<WorkloadStatus> {
        Ok(WorkloadStatus {
            workload: id,
            running: true,
        })
    }
    async fn stop(&self, id: WorkloadId) -> ProviderResult<WorkloadStatus> {
        Ok(WorkloadStatus {
            workload: id,
            running: false,
        })
    }
    async fn exec(&self, _req: ExecStartRequest) -> ProviderResult<ExecutionId> {
        ExecutionId::parse("mock-exec-1").map_err(|_| {
            ProviderError::new(d2b_realm_core::ErrorKind::MalformedFrame, "mock exec id")
        })
    }
}

/// A display provider that cannot present windows: every request returns a
/// typed `WindowForwarding` capability denial (fail-closed).
#[derive(Debug, Clone, Default)]
pub struct HeadlessDisplayProvider;

#[async_trait]
impl DisplayProvider for HeadlessDisplayProvider {
    fn provider_id(&self) -> ProviderId {
        id("headless")
    }
    fn capabilities(&self) -> DisplayCapabilitySet {
        DisplayCapabilitySet::default()
    }
    async fn open_display_session(
        &self,
        _req: DisplaySessionRequest,
    ) -> ProviderResult<DisplaySessionHandle> {
        Err(ProviderError::capability_denied(
            Capability::WindowForwarding,
        ))
    }
    async fn close_display_session(&self, _id: DisplaySessionId) -> ProviderResult<()> {
        Ok(())
    }
}

/// A provider-managed workload surface with no guestd-compatible agent.
#[derive(Debug, Clone, Default)]
pub struct NoGuestControlEndpointProvider;

#[async_trait]
impl GuestControlEndpointProvider for NoGuestControlEndpointProvider {
    fn provider_id(&self) -> ProviderId {
        id("no-guest-control")
    }

    fn node_id(&self) -> NodeId {
        NodeId::parse("mock").expect("valid")
    }

    fn capabilities(&self) -> WorkloadCapabilitySet {
        WorkloadCapabilitySet::default()
    }

    async fn endpoint_status(
        &self,
        _workload: WorkloadId,
    ) -> ProviderResult<GuestControlEndpointStatus> {
        Err(ProviderError::capability_denied(
            Capability::PersistentShell,
        ))
    }
}

/// A provider-managed workload surface that cannot run persistent shells.
#[derive(Debug, Clone, Default)]
pub struct HeadlessPersistentShellProvider;

#[async_trait]
impl PersistentShellProvider for HeadlessPersistentShellProvider {
    fn provider_id(&self) -> ProviderId {
        id("headless-shell")
    }

    fn node_id(&self) -> NodeId {
        NodeId::parse("mock").expect("valid")
    }

    fn capabilities(&self) -> WorkloadCapabilitySet {
        WorkloadCapabilitySet::default()
    }

    async fn list_shells(
        &self,
        _req: PersistentShellListProviderRequest,
    ) -> ProviderResult<PersistentShellListProviderResponse> {
        Err(ProviderError::capability_denied(
            Capability::PersistentShell,
        ))
    }

    async fn attach_shell(
        &self,
        _req: PersistentShellAttachProviderRequest,
    ) -> ProviderResult<PersistentShellAttachProviderResponse> {
        Err(ProviderError::capability_denied(
            Capability::PersistentShell,
        ))
    }

    async fn detach_shell(
        &self,
        _req: PersistentShellDetachProviderRequest,
    ) -> ProviderResult<PersistentShellStatus> {
        Err(ProviderError::capability_denied(
            Capability::PersistentShell,
        ))
    }

    async fn kill_shell(
        &self,
        _req: PersistentShellKillProviderRequest,
    ) -> ProviderResult<PersistentShellStatus> {
        Err(ProviderError::capability_denied(
            Capability::PersistentShell,
        ))
    }
}

/// A provider-managed workload surface that advertises persistent shell support
/// but still rejects forged attach streams before doing any work.
#[derive(Debug, Clone, Default)]
pub struct StrictPersistentShellProvider;

#[async_trait]
impl PersistentShellProvider for StrictPersistentShellProvider {
    fn provider_id(&self) -> ProviderId {
        id("strict-shell")
    }

    fn node_id(&self) -> NodeId {
        NodeId::parse("mock").expect("valid")
    }

    fn capabilities(&self) -> WorkloadCapabilitySet {
        WorkloadCapabilitySet {
            caps: CapabilitySet::empty().with(Capability::PersistentShell),
        }
    }

    async fn list_shells(
        &self,
        _req: PersistentShellListProviderRequest,
    ) -> ProviderResult<PersistentShellListProviderResponse> {
        Err(ProviderError::unsupported(
            "strict mock does not list shells",
        ))
    }

    async fn attach_shell(
        &self,
        req: PersistentShellAttachProviderRequest,
    ) -> ProviderResult<PersistentShellAttachProviderResponse> {
        if !req.shell_pty_stream_is_authorized() {
            return Err(ProviderError::capability_denied(
                Capability::PersistentShell,
            ));
        }
        Err(ProviderError::unsupported(
            "strict mock does not attach shells",
        ))
    }

    async fn detach_shell(
        &self,
        _req: PersistentShellDetachProviderRequest,
    ) -> ProviderResult<PersistentShellStatus> {
        Err(ProviderError::unsupported(
            "strict mock does not detach shells",
        ))
    }

    async fn kill_shell(
        &self,
        _req: PersistentShellKillProviderRequest,
    ) -> ProviderResult<PersistentShellStatus> {
        Err(ProviderError::unsupported(
            "strict mock does not kill shells",
        ))
    }
}

/// A mux that advertises a fixed capability set and opens a stream only
/// when (a) the open's authz capability matches the descriptor kind and
/// (b) the required capability is advertised — otherwise it fails closed
/// with a typed `CapabilityDenied`. On success it returns
/// a loopback substream so conformance can exercise the byte path.
#[derive(Debug, Clone)]
pub struct LoopbackStreamMux {
    caps: CapabilitySet,
}

impl LoopbackStreamMux {
    /// Build a mux advertising exactly `caps`.
    pub fn new(caps: CapabilitySet) -> Self {
        Self { caps }
    }
}

impl Default for LoopbackStreamMux {
    fn default() -> Self {
        // Advertises lifecycle/exec/window-forwarding by default.
        Self::new(
            CapabilitySet::empty()
                .with(Capability::Lifecycle)
                .with(Capability::Exec)
                .with(Capability::WindowForwarding),
        )
    }
}

#[async_trait]
impl StreamMux for LoopbackStreamMux {
    async fn open_stream(&self, open: StreamOpen) -> ProviderResult<StreamHandle> {
        // Fail closed: the authz capability must match the kind, and the
        // required capability must be advertised.
        if !open.is_consistent() {
            return Err(ProviderError::capability_denied(
                open.descriptor.kind.required_capability(),
            ));
        }
        let required = open.descriptor.kind.required_capability();
        if !self.caps.has(required) {
            return Err(ProviderError::capability_denied(required));
        }
        let (near, _far) = tokio::io::duplex(64);
        Ok(StreamHandle::new(open.descriptor.id, Box::new(near)))
    }

    async fn accept_stream(&self) -> ProviderResult<IncomingStream> {
        Err(ProviderError::unsupported(
            "mock mux does not accept inbound streams",
        ))
    }

    async fn close_stream(&self, _id: d2b_realm_core::StreamId) -> ProviderResult<()> {
        Ok(())
    }
}

// ---- Remaining trait-family stand-ins ---------------------------------
//
// The families above (workload/display/guest-control-endpoint/persistent-
// shell/stream-mux) are exercised by conformance.rs today. The stand-ins
// below give every other `provider::*` trait family an in-memory,
// non-networked implementation so [`parity`] can build a full inventory
// without ever dialing a real host, relay, or provider API.

/// A generic-Linux host substrate mock: reports fixed, low-cardinality
/// capability data without probing a real host.
#[derive(Debug, Clone, Default)]
pub struct MockHostSubstrateProvider;

#[async_trait]
impl HostSubstrateProvider for MockHostSubstrateProvider {
    fn provider_id(&self) -> ProviderId {
        id("mock-host-substrate")
    }
    async fn check(&self) -> ProviderResult<NodeCapabilitySet> {
        Ok(NodeCapabilitySet {
            caps: CapabilitySet::empty().with(Capability::Lifecycle),
            substrate: Some(HostSubstrateKind::GenericLinux),
            substrate_version: None,
            userns_available: true,
            vhost_acceleration: false,
            lsm: None,
        })
    }
}

/// A runtime provider mock that plans/starts/stops/inspects trivially and
/// advertises `Lifecycle` only.
#[derive(Debug, Clone)]
pub struct MockRuntimeProvider {
    provider: ProviderId,
    caps: RuntimeCapabilitySet,
}

impl Default for MockRuntimeProvider {
    fn default() -> Self {
        Self {
            provider: id("mock-runtime"),
            caps: RuntimeCapabilitySet {
                caps: CapabilitySet::empty().with(Capability::Lifecycle),
            },
        }
    }
}

#[async_trait]
impl RuntimeProvider for MockRuntimeProvider {
    fn provider_id(&self) -> ProviderId {
        self.provider.clone()
    }
    fn capabilities(&self) -> RuntimeCapabilitySet {
        self.caps.clone()
    }
    async fn plan_workload(&self, spec: crate::types::WorkloadSpec) -> ProviderResult<RuntimePlan> {
        Ok(RuntimePlan {
            provider: self.provider.clone(),
            workload: spec.alias,
        })
    }
    async fn start(&self, plan: RuntimePlan) -> ProviderResult<RuntimeHandle> {
        Ok(RuntimeHandle {
            workload: plan.workload,
        })
    }
    async fn stop(&self, _handle: RuntimeHandle) -> ProviderResult<()> {
        Ok(())
    }
    async fn inspect(&self, handle: RuntimeHandle) -> ProviderResult<RuntimeStatus> {
        Ok(RuntimeStatus {
            workload: handle.workload,
            running: true,
        })
    }
}

/// A durable-execution provider mock that rediscovers/attaches/logs/cancels
/// deterministically without a real guest-control or ACA peer.
#[derive(Debug, Clone, Default)]
pub struct MockDurableExecutionProvider;

#[async_trait]
impl DurableExecutionProvider for MockDurableExecutionProvider {
    fn provider_id(&self) -> ProviderId {
        id("mock-durable-exec")
    }
    async fn start(
        &self,
        req: d2b_realm_core::ExecStartRequest,
    ) -> ProviderResult<d2b_realm_core::ExecutionSummary> {
        Ok(d2b_realm_core::ExecutionSummary {
            id: req.execution_id,
            workload: req.workload,
            state: d2b_realm_core::ExecState::Running,
            exit_code: None,
            tty: req.tty,
            generation: req.generation,
            attach_mode: req.attach_mode,
            stdout_cursor: None,
            stderr_cursor: None,
        })
    }
    async fn attach(
        &self,
        req: d2b_realm_core::ExecAttachRequest,
    ) -> ProviderResult<d2b_realm_core::ExecutionSummary> {
        Ok(d2b_realm_core::ExecutionSummary {
            id: req.execution_id,
            workload: WorkloadId::parse("mock-workload").expect("valid"),
            state: d2b_realm_core::ExecState::Running,
            exit_code: None,
            tty: false,
            generation: req.generation,
            attach_mode: d2b_realm_core::ExecAttachMode::Attached,
            stdout_cursor: req.stdout_cursor,
            stderr_cursor: req.stderr_cursor,
        })
    }
    async fn logs(
        &self,
        req: d2b_realm_core::ExecLogsRequest,
    ) -> ProviderResult<d2b_realm_core::ExecutionSummary> {
        Ok(d2b_realm_core::ExecutionSummary {
            id: req.execution_id,
            workload: WorkloadId::parse("mock-workload").expect("valid"),
            state: d2b_realm_core::ExecState::Exited,
            exit_code: Some(0),
            tty: false,
            generation: req.generation,
            attach_mode: d2b_realm_core::ExecAttachMode::Detached,
            stdout_cursor: req.cursor,
            stderr_cursor: None,
        })
    }
    async fn cancel(&self, _req: d2b_realm_core::ExecCancelRequest) -> ProviderResult<bool> {
        // Idempotent by contract: the mock always reports a successful
        // cancel, mirroring "already terminal or unknown" being a valid
        // `false` outcome only when the execution was never started here.
        Ok(true)
    }
}

/// A transport listener stand-in that never receives an inbound loopback
/// session. Shared by [`LoopbackTransportProvider::listen`] and
/// [`MockRelayProvider::open_listener`]: both hand back a listener object
/// without any real accept path.
#[derive(Debug, Clone)]
pub struct NoAcceptTransportListener {
    node: NodeId,
}

#[async_trait]
impl TransportListener for NoAcceptTransportListener {
    fn node(&self) -> NodeId {
        self.node.clone()
    }
    async fn accept(&self) -> ProviderResult<TransportSession> {
        Err(ProviderError::unsupported(
            "mock transport listener does not accept inbound sessions",
        ))
    }
}

/// A loopback transport: `connect` opens an in-memory duplex byte channel;
/// `listen` returns a listener that never accepts. No network I/O.
#[derive(Debug, Clone, Default)]
pub struct LoopbackTransportProvider;

#[async_trait]
impl TransportProvider for LoopbackTransportProvider {
    fn transport_id(&self) -> ProviderId {
        id("mock-loopback-transport")
    }
    async fn connect(&self, _target: TransportTarget) -> ProviderResult<TransportSession> {
        let (near, _far) = tokio::io::duplex(64);
        Ok(TransportSession::new(
            SafeLabel::new("loopback"),
            Box::new(near),
        ))
    }
    async fn listen(
        &self,
        registration: NodeRegistration,
    ) -> ProviderResult<Box<dyn TransportListener>> {
        Ok(Box::new(NoAcceptTransportListener {
            node: registration.node,
        }))
    }
}

/// A codec placeholder proving the [`ProtocolCodec`] seam without a real
/// wire format: every encode/decode fails closed with a typed
/// `UnsupportedFeature` refusal (this crate MUST NOT depend on a real
/// protocol codec; see the crate-level dependency-direction note).
#[derive(Debug, Clone, Default)]
pub struct NoOpProtocolCodec;

impl ProtocolCodec for NoOpProtocolCodec {
    fn codec_id(&self) -> &str {
        "mock-noop"
    }
    fn encode_frame(&self, _frame: &ConstellationFrame) -> Result<Vec<u8>, ConstellationError> {
        Err(ConstellationError::new(
            ErrorKind::UnsupportedFeature,
            "mock codec does not encode",
        ))
    }
    fn decode_frame(&self, _bytes: &[u8]) -> Result<ConstellationFrame, ConstellationError> {
        Err(ConstellationError::new(
            ErrorKind::UnsupportedFeature,
            "mock codec does not decode",
        ))
    }
    fn schema_fingerprint(&self) -> String {
        "mock-noop-v0".to_owned()
    }
}

/// The only implemented daemon-access transport mode today: a local
/// loopback stand-in (no real `public.sock`).
#[derive(Debug, Clone, Default)]
pub struct LocalUnixDaemonAccessTransport;

#[async_trait]
impl DaemonAccessTransport for LocalUnixDaemonAccessTransport {
    fn transport_id(&self) -> ProviderId {
        id("mock-local-unix")
    }
    fn mode(&self) -> DaemonAccessMode {
        DaemonAccessMode::LocalUnix
    }
    async fn connect(&self, _endpoint: TransportTarget) -> ProviderResult<TransportSession> {
        let (near, _far) = tokio::io::duplex(64);
        Ok(TransportSession::new(
            SafeLabel::new("local-unix"),
            Box::new(near),
        ))
    }
}

/// A declared-but-unimplemented daemon-access transport mode (relay, direct
/// mTLS/QUIC/WebSocket, SSH bootstrap): fails closed with a typed
/// `UnsupportedFeature` refusal rather than an undocumented fallback.
#[derive(Debug, Clone, Copy)]
pub struct UnimplementedDaemonAccessTransport {
    mode: DaemonAccessMode,
}

impl UnimplementedDaemonAccessTransport {
    /// Build a stand-in for a not-yet-implemented `mode`.
    ///
    /// # Panics
    /// Panics (debug builds only) if `mode` is already implemented; callers
    /// must reach for [`LocalUnixDaemonAccessTransport`] instead.
    pub fn new(mode: DaemonAccessMode) -> Self {
        debug_assert!(!mode.is_implemented(), "mode is already implemented");
        Self { mode }
    }
}

#[async_trait]
impl DaemonAccessTransport for UnimplementedDaemonAccessTransport {
    fn transport_id(&self) -> ProviderId {
        id("mock-unimplemented-transport")
    }
    fn mode(&self) -> DaemonAccessMode {
        self.mode
    }
    async fn connect(&self, _endpoint: TransportTarget) -> ProviderResult<TransportSession> {
        Err(ProviderError::unsupported(
            "daemon access mode not implemented in this build",
        ))
    }
}

/// A daemon-access API mock that reports an empty local workload inventory.
#[derive(Debug, Clone, Default)]
pub struct MockDaemonAccessApi;

#[async_trait]
impl DaemonAccessApi for MockDaemonAccessApi {
    async fn vm_list(&self) -> ProviderResult<Vec<WorkloadSummary>> {
        Ok(vec![])
    }
}

/// An infrastructure provider mock that plans trivially and mutates nothing.
#[derive(Debug, Clone, Default)]
pub struct MockInfrastructureProvider;

#[async_trait]
impl InfrastructureProvider for MockInfrastructureProvider {
    fn provider_id(&self) -> ProviderId {
        id("mock-infrastructure")
    }
    async fn plan_infrastructure(&self, _node: NodeId) -> ProviderResult<()> {
        Ok(())
    }
}

/// A credential provider mock with a fixed, caller-selected status. Never
/// carries key material.
#[derive(Debug, Clone)]
pub struct FixedCredentialProvider {
    status: CredentialStatus,
}

impl FixedCredentialProvider {
    /// A valid, unexpired, unrevoked enrollment.
    pub fn valid() -> Self {
        Self {
            status: CredentialStatus::Valid,
        }
    }
    /// No enrollment present.
    pub fn absent() -> Self {
        Self {
            status: CredentialStatus::Absent,
        }
    }
}

#[async_trait]
impl CredentialProvider for FixedCredentialProvider {
    fn provider_id(&self) -> ProviderId {
        id("mock-credential")
    }
    async fn status(&self) -> ProviderResult<CredentialStatus> {
        Ok(self.status.clone())
    }
}

/// An observability sink mock with a fixed, caller-selected health.
#[derive(Debug, Clone, Copy)]
pub struct FixedObservabilitySinkProvider {
    is_healthy: bool,
}

impl FixedObservabilitySinkProvider {
    /// A reachable sink.
    pub fn healthy() -> Self {
        Self { is_healthy: true }
    }
    /// An unreachable sink (degraded handling, not an error).
    pub fn unreachable() -> Self {
        Self { is_healthy: false }
    }
}

#[async_trait]
impl ObservabilitySinkProvider for FixedObservabilitySinkProvider {
    fn provider_id(&self) -> ProviderId {
        id("mock-observability-sink")
    }
    async fn healthy(&self) -> ProviderResult<bool> {
        Ok(self.is_healthy)
    }
}

/// A relay provider mock whose listener never accepts (see
/// [`NoAcceptTransportListener`]).
#[derive(Debug, Clone, Default)]
pub struct MockRelayProvider;

#[async_trait]
impl RelayProvider for MockRelayProvider {
    fn provider_id(&self) -> ProviderId {
        id("mock-relay")
    }
    async fn open_listener(&self, node: NodeId) -> ProviderResult<Box<dyn TransportListener>> {
        Ok(Box::new(NoAcceptTransportListener { node }))
    }
}

/// A node provider mock advertising `Lifecycle` and an empty workload
/// inventory.
#[derive(Debug, Clone)]
pub struct MockNodeProvider {
    node: NodeId,
    caps: NodeCapabilitySet,
}

impl Default for MockNodeProvider {
    fn default() -> Self {
        Self {
            node: NodeId::parse("mock").expect("valid"),
            caps: NodeCapabilitySet {
                caps: CapabilitySet::empty().with(Capability::Lifecycle),
                ..NodeCapabilitySet::default()
            },
        }
    }
}

#[async_trait]
impl NodeProvider for MockNodeProvider {
    fn node_id(&self) -> NodeId {
        self.node.clone()
    }
    fn capabilities(&self) -> NodeCapabilitySet {
        self.caps.clone()
    }
    async fn list_workloads(&self) -> ProviderResult<Vec<WorkloadSummary>> {
        Ok(vec![])
    }
}
