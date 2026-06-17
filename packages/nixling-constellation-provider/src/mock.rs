//! No-op / mock providers for conformance tests and local model
//! validation. A `HeadlessDisplayProvider` returns typed capability
//! denials so callers exercise fail-closed routing.

use async_trait::async_trait;
use nixling_constellation_core::{
    Capability, ExecutionId, NodeId, ProviderId, WorkloadId, WorkloadSummary,
};

use crate::capabilities::{DisplayCapabilitySet, WorkloadCapabilitySet};
use crate::error::{ProviderError, ProviderResult};
use crate::provider::{DisplayProvider, WorkloadProvider};
use crate::types::{
    DisplaySessionHandle, DisplaySessionId, DisplaySessionRequest, ExecStartRequest, ListSelector,
    StreamHandle, StreamOpenRequest, WorkloadStatus,
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
                caps: nixling_constellation_core::CapabilitySet::empty()
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
    async fn open_stream(&self, req: StreamOpenRequest) -> ProviderResult<StreamHandle> {
        Ok(StreamHandle { id: req.id })
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
        Err(ProviderError::capability_denied(Capability::WindowForwarding))
    }
    async fn close_display_session(&self, _id: DisplaySessionId) -> ProviderResult<()> {
        Ok(())
    }
}
