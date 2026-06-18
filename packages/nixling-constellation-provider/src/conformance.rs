//! Conformance fixtures (ADR 0032). Any provider implementation can be
//! run through these to prove it advertises capabilities as data and
//! fails closed when a capability is absent.

use nixling_constellation_core::{
    Capability, ErrorKind, OperationId, PrincipalId, RealmPath, StreamAuthz, StreamDescriptor,
    StreamId, StreamKind, StreamOpen, WorkloadSelector,
};

use crate::error::ProviderResult;
use crate::provider::{DisplayProvider, StreamMux, WorkloadProvider};
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
/// typed [`ErrorKind::CapabilityDenied`] (not a silent fallback) carrying
/// the structured `WindowForwarding` missing capability.
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
        Err(e) => {
            e.kind() == ErrorKind::CapabilityDenied
                && e.missing_capability() == Some(Capability::WindowForwarding)
        }
        Ok(_) => false,
    }
}

fn stream_open(kind: StreamKind, authz_capability: Option<Capability>) -> StreamOpen {
    let principal = PrincipalId::parse("conformance-principal").expect("valid");
    let realm = RealmPath::local();
    let authz = match authz_capability {
        // A forged authz whose capability is downgraded from the kind.
        Some(cap) => StreamAuthz {
            principal,
            realm,
            capability: cap,
        },
        None => StreamAuthz::for_kind(principal, realm, kind),
    };
    StreamOpen {
        descriptor: StreamDescriptor {
            id: StreamId::parse("conformance-stream").expect("valid"),
            kind,
        },
        operation_id: OperationId::parse("conformance-op").expect("valid"),
        authz,
    }
}

/// Assert that a [`StreamMux`] fails closed (`CapabilityDenied`, naming the
/// missing capability) when asked to open a stream whose required
/// capability is not advertised. The `kind` MUST be one the mux does not
/// support.
pub async fn mux_fails_closed_on_unsupported_stream(mux: &dyn StreamMux, kind: StreamKind) -> bool {
    match mux.open_stream(stream_open(kind, None)).await {
        Err(e) => {
            e.kind() == ErrorKind::CapabilityDenied
                && e.missing_capability() == Some(kind.required_capability())
        }
        Ok(_) => false,
    }
}

/// Assert that a [`StreamMux`] fails closed when handed an in-memory
/// `StreamOpen` whose authz capability is downgraded from the descriptor
/// kind (a forged, inconsistent open that bypassed the wire decoder).
pub async fn mux_rejects_inconsistent_open(
    mux: &dyn StreamMux,
    kind: StreamKind,
    forged: Capability,
) -> bool {
    assert_ne!(forged, kind.required_capability(), "forged must differ");
    match mux.open_stream(stream_open(kind, Some(forged))).await {
        Err(e) => e.kind() == ErrorKind::CapabilityDenied,
        Ok(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::{HeadlessDisplayProvider, LoopbackStreamMux, MockWorkloadProvider};
    use crate::types::DisplaySessionRequest;
    use nixling_constellation_core::{
        OperationId, PrincipalId, RealmPath, StreamAuthz, StreamId, StreamKind, WorkloadId,
    };

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
            operation_id: OperationId::parse("op-display-1").unwrap(),
            display_stream: StreamId::parse("disp-1").unwrap(),
            authz: StreamAuthz::for_kind(
                PrincipalId::parse("principal-1").unwrap(),
                RealmPath::local(),
                StreamKind::Display,
            ),
        };
        assert!(display_fails_closed_when_unsupported(&p, req).await);
    }

    #[tokio::test]
    async fn mux_denies_unadvertised_stream() {
        // The default loopback mux advertises lifecycle/exec/window-forwarding
        // but NOT clipboard, so a clipboard stream open must fail closed
        // naming the missing capability.
        let mux = LoopbackStreamMux::default();
        assert!(mux_fails_closed_on_unsupported_stream(&mux, StreamKind::Clipboard).await);
        // An advertised kind (Display -> WindowForwarding) is allowed.
        assert!(!mux_fails_closed_on_unsupported_stream(&mux, StreamKind::Display).await);
    }

    #[tokio::test]
    async fn mux_rejects_forged_inconsistent_open() {
        // A mux that advertises BOTH WindowForwarding (the Display-required
        // capability) AND Clipboard (the forged authz capability). A naive
        // mux that only checked the authz capability against its advertised
        // set would wrongly ALLOW this; the correct mux rejects it because
        // the Display descriptor is paired with a downgraded Clipboard authz.
        let mux = LoopbackStreamMux::new(
            nixling_constellation_core::CapabilitySet::empty()
                .with(Capability::WindowForwarding)
                .with(Capability::Clipboard),
        );
        assert!(
            mux_rejects_inconsistent_open(&mux, StreamKind::Display, Capability::Clipboard).await
        );
    }
}
