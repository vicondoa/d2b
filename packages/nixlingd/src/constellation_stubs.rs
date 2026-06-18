//! Initial compile-only peer-module skeletons (ADR 0032, s8).
//!
//! These wire the v2 provider/router/transport trait surface into
//! `nixlingd` so future gateway work can fill them in, but they are **not**
//! called from the running daemon — the local CLI→daemon path is unchanged
//! (zero behavior change). The module exists to prove the constellation
//! contract compiles against the daemon's dependency set and to give the
//! later gateway-mode work a concrete set of seams:
//!
//! - [`ApiFrontend`] — terminates a peer session and hands decoded frames in.
//! - [`ApiService`] — the transport-neutral CLI-facing daemon API surface.
//! - [`TargetResolver`] — resolves a realm-path target to a node/provider.
//! - [`PeerOperationRouter`] — binds the codec-neutral
//!   [`OperationRouter`](nixling_constellation_router::OperationRouter). It
//!   holds a **shared** node/gateway-scoped router (see [`SharedRouter`]) so
//!   reconnecting peer sessions share one dedup owner; a fresh per-session
//!   router would let reconnect retries bypass dedup and double-dispatch.
//! - [`ProviderExecutor`] — dispatches an accepted operation to a provider.
//! - [`LocalExecutor`] — the current local execution path (unchanged).
//! - [`PeerDaemon`] — a remote-node peer session (future gateway work).
//!
//! Everything here is `dead_code`-allowed until the gateway-mode work wires
//! it.

#![allow(dead_code)]

use std::sync::{Arc, Mutex};

use nixling_constellation_core::{OperationRequest, PrincipalId, RealmPath};
use nixling_constellation_provider::error::ProviderResult;
use nixling_constellation_provider::provider::{ProtocolCodec, WorkloadProvider};
use nixling_constellation_router::{OperationRouter, RouteDecision};

/// A node/gateway-scoped [`OperationRouter`] shared across peer sessions.
/// Constructed once per node and injected into every [`PeerOperationRouter`]
/// so dedup state survives session reconnects.
pub type SharedRouter = Arc<Mutex<OperationRouter>>;

/// Build a fresh node/gateway-scoped shared router.
pub fn new_shared_router() -> SharedRouter {
    Arc::new(Mutex::new(OperationRouter::new()))
}

/// Terminates a peer session: decodes wire bytes through a [`ProtocolCodec`]
/// into the semantic frame layer and forwards them to the API service. The
/// initial skeleton carries the seam only.
pub struct ApiFrontend {
    codec: Box<dyn ProtocolCodec>,
}

impl ApiFrontend {
    /// Build the frontend around a negotiated codec.
    pub fn new(codec: Box<dyn ProtocolCodec>) -> Self {
        Self { codec }
    }

    /// The negotiated codec id (diagnostics).
    pub fn codec_id(&self) -> &str {
        self.codec.codec_id()
    }
}

/// The transport-neutral CLI-facing daemon API surface (skeleton).
pub struct ApiService;

impl ApiService {
    /// Build the API service.
    pub fn new() -> Self {
        Self
    }
}

impl Default for ApiService {
    fn default() -> Self {
        Self::new()
    }
}

/// Resolves a realm-path target to the node/provider that serves it
/// (skeleton). Resolves to the local node until the realm/node registry
/// lands.
pub struct TargetResolver;

impl TargetResolver {
    /// Build the resolver.
    pub fn new() -> Self {
        Self
    }

    /// Resolve a target realm path. Returns the input unchanged (local-only)
    /// until later ADR 0032 work consults the realm/node registry.
    pub fn resolve(&self, realm: RealmPath) -> RealmPath {
        realm
    }
}

impl Default for TargetResolver {
    fn default() -> Self {
        Self::new()
    }
}

/// Binds the codec-neutral [`OperationRouter`] for a peer session against a
/// **shared** node/gateway-scoped router so reconnecting sessions share one
/// dedup owner.
pub struct PeerOperationRouter {
    router: SharedRouter,
}

impl PeerOperationRouter {
    /// Bind a peer session to the injected node-scoped shared router.
    pub fn new(router: SharedRouter) -> Self {
        Self { router }
    }

    /// Route one operation against the authenticated session principal.
    pub fn route(&self, req: &OperationRequest, principal: &PrincipalId) -> RouteDecision {
        self.router
            .lock()
            .expect("shared operation router mutex poisoned")
            .route(req, principal)
    }
}

/// Dispatches an accepted operation to a workload provider (skeleton).
pub struct ProviderExecutor {
    provider: Box<dyn WorkloadProvider>,
}

impl ProviderExecutor {
    /// Build the executor around a workload provider.
    pub fn new(provider: Box<dyn WorkloadProvider>) -> Self {
        Self { provider }
    }

    /// List workloads through the bound provider (skeleton path).
    pub async fn list(&self) -> ProviderResult<usize> {
        Ok(self
            .provider
            .list(nixling_constellation_core::WorkloadSelector::All)
            .await?
            .len())
    }
}

/// The current local execution path (unchanged). The marker records that
/// the local path remains the default; later ADR 0032 work routes through
/// the router + provider executor instead.
pub struct LocalExecutor;

impl LocalExecutor {
    /// Build the local executor.
    pub fn new() -> Self {
        Self
    }
}

impl Default for LocalExecutor {
    fn default() -> Self {
        Self::new()
    }
}

/// A remote-node peer session (future gateway work). Carries the seam only.
pub struct PeerDaemon;

impl PeerDaemon {
    /// Build the peer-daemon skeleton.
    pub fn new() -> Self {
        Self
    }
}

impl Default for PeerDaemon {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nixling_constellation_core::{
        IdempotencyKey, NodeId, OpaquePayload, OperationId, OperationKind,
    };

    fn list_req(principal: &PrincipalId) -> OperationRequest {
        OperationRequest {
            operation_id: OperationId::parse("op-1").unwrap(),
            idempotency_key: None,
            realm: RealmPath::local(),
            node: NodeId::parse("gw").unwrap(),
            workload: None,
            principal: principal.clone(),
            kind: OperationKind::WorkloadList,
            trace: None,
            body: OpaquePayload::empty(),
        }
    }

    fn start_req(principal: &PrincipalId, op_id: &str, key: &str) -> OperationRequest {
        OperationRequest {
            operation_id: OperationId::parse(op_id).unwrap(),
            idempotency_key: Some(IdempotencyKey::parse(key).unwrap()),
            realm: RealmPath::local(),
            node: NodeId::parse("gw").unwrap(),
            workload: None,
            principal: principal.clone(),
            kind: OperationKind::WorkloadStart,
            trace: None,
            body: OpaquePayload::new(b"start".to_vec()).unwrap(),
        }
    }

    #[test]
    fn peer_router_routes_through_the_codec_neutral_router() {
        let r = PeerOperationRouter::new(new_shared_router());
        let principal = PrincipalId::parse("alice").unwrap();
        let req = list_req(&principal);
        assert!(matches!(
            r.route(&req, &principal),
            RouteDecision::Accept { .. }
        ));
    }

    #[test]
    fn reconnecting_sessions_share_one_dedup_owner() {
        // A single node-scoped shared router injected into two distinct peer
        // sessions: a mutating op accepted on the first session must be seen
        // as in-progress by a reconnect on the second session (no
        // double-dispatch across session boundaries).
        let shared = new_shared_router();
        let principal = PrincipalId::parse("alice").unwrap();

        let session_a = PeerOperationRouter::new(shared.clone());
        let session_b = PeerOperationRouter::new(shared.clone());

        let req = start_req(&principal, "op-1", "k1");
        assert!(matches!(
            session_a.route(&req, &principal),
            RouteDecision::Accept { .. }
        ));
        // Reconnect retry on a different session hits the same dedup state.
        match session_b.route(&req, &principal) {
            RouteDecision::InProgress {
                original_operation_id,
            } => assert_eq!(original_operation_id, OperationId::parse("op-1").unwrap()),
            other => panic!("expected InProgress across sessions, got {other:?}"),
        }
    }

    #[test]
    fn target_resolver_resolves_local_target_unchanged() {
        let resolver = TargetResolver::new();
        let realm = RealmPath::local();
        assert_eq!(resolver.resolve(realm.clone()), realm);
    }
}
