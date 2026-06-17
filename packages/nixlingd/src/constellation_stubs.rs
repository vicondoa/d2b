//! Wave 0 compile-only peer-module skeletons (ADR 0032, s8).
//!
//! These wire the v2 provider/router/transport trait surface into
//! `nixlingd` so later waves can fill them in, but they are **not** called
//! from the running daemon — the local CLI→daemon path is unchanged (zero
//! behavior change). The module exists to prove the constellation contract
//! compiles against the daemon's dependency set and to give the later
//! gateway-mode work a concrete set of seams:
//!
//! - [`ApiFrontend`] — terminates a peer session and hands decoded frames in.
//! - [`ApiService`] — the transport-neutral CLI-facing daemon API surface.
//! - [`TargetResolver`] — resolves a realm-path target to a node/provider.
//! - [`PeerOperationRouter`] — binds the codec-neutral
//!   [`OperationRouter`](nixling_constellation_router::OperationRouter).
//! - [`ProviderExecutor`] — dispatches an accepted operation to a provider.
//! - [`LocalExecutor`] — the current local execution path (unchanged).
//! - [`PeerDaemon`] — a remote-node peer session (later wave).
//!
//! Everything here is `dead_code`-allowed until the gateway waves wire it.

#![allow(dead_code)]

use nixling_constellation_core::{OperationRequest, PrincipalId, RealmPath};
use nixling_constellation_provider::error::ProviderResult;
use nixling_constellation_provider::provider::{ProtocolCodec, WorkloadProvider};
use nixling_constellation_router::{OperationRouter, RouteDecision};

/// Terminates a peer session: decodes wire bytes through a [`ProtocolCodec`]
/// into the semantic frame layer and forwards them to the API service. Wave
/// 0 carries the seam only.
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
/// (skeleton). Wave 0 always resolves to the local node.
pub struct TargetResolver;

impl TargetResolver {
    /// Build the resolver.
    pub fn new() -> Self {
        Self
    }

    /// Resolve a target realm path. Wave 0 returns the input unchanged
    /// (local-only); later waves consult the realm/node registry.
    pub fn resolve(&self, realm: RealmPath) -> RealmPath {
        realm
    }
}

impl Default for TargetResolver {
    fn default() -> Self {
        Self::new()
    }
}

/// Binds the codec-neutral [`OperationRouter`] for a peer session.
pub struct PeerOperationRouter {
    router: OperationRouter,
}

impl PeerOperationRouter {
    /// Build the per-session router.
    pub fn new() -> Self {
        Self {
            router: OperationRouter::new(),
        }
    }

    /// Route one operation against the authenticated session principal.
    pub fn route(&mut self, req: &OperationRequest, principal: &PrincipalId) -> RouteDecision {
        self.router.route(req, principal)
    }
}

impl Default for PeerOperationRouter {
    fn default() -> Self {
        Self::new()
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

/// The current local execution path (unchanged in Wave 0). The marker
/// records that the local path remains the default; later waves route
/// through the router + provider executor instead.
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

/// A remote-node peer session (later wave). Wave 0 carries the seam only.
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
    use nixling_constellation_core::{NodeId, OpaquePayload, OperationId, OperationKind};

    #[test]
    fn peer_router_routes_through_the_codec_neutral_router() {
        let mut r = PeerOperationRouter::new();
        let principal = PrincipalId::parse("alice").unwrap();
        let req = OperationRequest {
            operation_id: OperationId::parse("op-1").unwrap(),
            idempotency_key: None,
            realm: RealmPath::local(),
            node: NodeId::parse("gw").unwrap(),
            workload: None,
            principal: principal.clone(),
            kind: OperationKind::WorkloadList,
            trace: None,
            body: OpaquePayload::empty(),
        };
        assert!(matches!(
            r.route(&req, &principal),
            RouteDecision::Accept { .. }
        ));
    }

    #[test]
    fn target_resolver_is_local_only_in_wave0() {
        let resolver = TargetResolver::new();
        let realm = RealmPath::local();
        assert_eq!(resolver.resolve(realm.clone()), realm);
    }
}
