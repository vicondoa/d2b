//! No-op / mock providers for conformance tests and local model
//! validation. The display + mux mocks return typed capability denials so
//! callers exercise fail-closed routing.

use async_trait::async_trait;
use nixling_constellation_core::{
    Capability, CapabilitySet, ExecutionId, NodeId, ProviderId, StreamOpen, WorkloadId,
    WorkloadSummary,
};

use crate::capabilities::{DisplayCapabilitySet, WorkloadCapabilitySet};
use crate::error::{ProviderError, ProviderResult};
use crate::provider::{DisplayProvider, StreamMux, WorkloadProvider};
use crate::types::{
    DisplaySessionHandle, DisplaySessionId, DisplaySessionRequest, ExecStartRequest,
    IncomingStream, ListSelector, StreamHandle, WorkloadStatus,
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
            ProviderError::new(
                nixling_constellation_core::ErrorKind::MalformedFrame,
                "mock exec id",
            )
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

    async fn close_stream(&self, _id: nixling_constellation_core::StreamId) -> ProviderResult<()> {
        Ok(())
    }
}
