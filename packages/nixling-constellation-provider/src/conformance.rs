//! Conformance fixtures (ADR 0032). Any provider implementation can be
//! run through these to prove it advertises capabilities as data and
//! fails closed when a capability is absent.

use nixling_constellation_core::{Capability, ErrorKind, WorkloadSelector};

use crate::error::ProviderResult;
use crate::provider::{DisplayProvider, WorkloadProvider};
use crate::types::DisplaySessionRequest;

/// Exercise the basic [`WorkloadProvider`] surface: listing must succeed
/// and capabilities must be queryable. Returns the advertised capability
/// presence for `Lifecycle`.
pub async fn workload_lists_and_advertises(
    provider: &dyn WorkloadProvider,
) -> ProviderResult<bool> {
    let _ = provider.list(WorkloadSelector::All).await?;
    Ok(provider.capabilities().has(Capability::Lifecycle))
}

/// Assert that a display provider lacking `window-forwarding` returns a
/// typed [`ErrorKind::CapabilityDenied`] (not a silent fallback) when a
/// display session is requested.
pub async fn display_fails_closed_when_unsupported(
    provider: &dyn DisplayProvider,
    req: DisplaySessionRequest,
) -> bool {
    if provider.capabilities().has(Capability::WindowForwarding) {
        // Provider claims support; this fixture only checks the
        // unsupported path.
        return true;
    }
    match provider.open_display_session(req).await {
        Err(e) => e.kind() == ErrorKind::CapabilityDenied,
        Ok(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::{HeadlessDisplayProvider, MockWorkloadProvider};
    use crate::types::DisplaySessionRequest;
    use nixling_constellation_core::WorkloadId;

    #[tokio::test]
    async fn mock_workload_passes_conformance() {
        let p = MockWorkloadProvider::default();
        assert!(workload_lists_and_advertises(&p).await.unwrap());
    }

    #[tokio::test]
    async fn headless_display_fails_closed() {
        let p = HeadlessDisplayProvider;
        let req = DisplaySessionRequest {
            workload: WorkloadId::parse("demo").unwrap(),
        };
        assert!(display_fails_closed_when_unsupported(&p, req).await);
    }
}
