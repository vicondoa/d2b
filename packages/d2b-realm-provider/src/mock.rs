//! No-op / mock providers for conformance tests and local model
//! validation. The display + mux mocks return typed capability denials so
//! callers exercise fail-closed routing.

use async_trait::async_trait;
use d2b_realm_core::{
    Capability, CapabilitySet, ExecutionId, NodeId, ProviderId, StreamOpen, WorkloadId,
    WorkloadSummary,
};

use crate::capabilities::{DisplayCapabilitySet, WorkloadCapabilitySet};
use crate::error::{ProviderError, ProviderResult};
use crate::provider::{
    DisplayProvider, GuestControlEndpointProvider, PersistentShellProvider, StreamMux,
    WorkloadProvider,
};
use crate::types::{
    DisplaySessionHandle, DisplaySessionId, DisplaySessionRequest, ExecStartRequest,
    GuestControlEndpointStatus, IncomingStream, ListSelector, PersistentShellAttachProviderRequest,
    PersistentShellAttachProviderResponse, PersistentShellDetachProviderRequest,
    PersistentShellKillProviderRequest, PersistentShellListProviderRequest,
    PersistentShellListProviderResponse, PersistentShellStatus, StreamHandle, WorkloadStatus,
};

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
