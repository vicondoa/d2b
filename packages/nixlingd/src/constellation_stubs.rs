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

use nixling_constellation_core::{OperationRequest, PrincipalId, TargetName};
use nixling_constellation_provider::error::ProviderResult;
use nixling_constellation_provider::provider::{ProtocolCodec, WorkloadProvider};
use nixling_constellation_router::{
    DispatchTarget, OperationRouter, RealmEntrypointTable, ResolveError, RouteDecision,
};

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

/// Resolves a constellation [`TargetName`] to the [`DispatchTarget`] that
/// serves it, by consulting the node's [`RealmEntrypointTable`] (ADR 0032
/// `TargetResolver`). The table is seeded with the reserved `local` realm as
/// host-resident; gateway-mode config wiring populates the rest. Resolution
/// is fail-closed — an unknown realm is rejected rather than defaulted to
/// local dispatch.
pub struct TargetResolver {
    table: RealmEntrypointTable,
}

impl TargetResolver {
    /// Build a resolver over an entrypoint table.
    pub fn new(table: RealmEntrypointTable) -> Self {
        Self { table }
    }

    /// A resolver that only knows the local (host-resident) realm — the
    /// host-mode default until realm config lands.
    pub fn local_only() -> Self {
        Self::new(RealmEntrypointTable::with_local_default())
    }

    /// Resolve a target to its dispatch decision.
    pub fn resolve(&self, target: &TargetName) -> Result<DispatchTarget, ResolveError> {
        self.table.resolve(target)
    }

    /// Borrow the underlying entrypoint table (e.g. to extend it from config).
    pub fn table(&self) -> &RealmEntrypointTable {
        &self.table
    }
}

impl Default for TargetResolver {
    fn default() -> Self {
        Self::local_only()
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

/// Which role a single `nixlingd` instance plays. There is exactly one
/// binary; the mode is selected from resolved config, never a separate
/// program. ADR 0015 keeps the host daemon as the sole local lifecycle
/// authority, while a realm gateway runs its own `nixlingd` in gateway mode
/// inside the gateway guest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaemonMode {
    /// The host daemon: supervises local VMs through the broker and is the
    /// only lifecycle authority for the host. Holds no realm
    /// provider/relay/entrypoint config.
    Host,
    /// A realm-scoped gateway daemon inside a gateway guest: terminates peer
    /// sessions and dispatches accepted operations to providers. Holds no
    /// host-broker / local-VM-lifecycle responsibility.
    Gateway,
}

/// The mode-relevant slice of a `nixlingd` instance's resolved config. It is
/// used both to **select** the mode (a realm entrypoint ⇒ gateway) and to
/// **guard** that the rest of the config matches the selected mode, so the
/// daemon refuses to start cross-wired (host mode carrying realm config, or
/// gateway mode carrying host-lifecycle responsibility).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DaemonModeConfig {
    /// A realm entrypoint is configured. Its presence selects gateway mode.
    pub has_realm_entrypoint: bool,
    /// Realm provider/relay config is present (only legal in gateway mode).
    pub has_provider_or_relay_config: bool,
    /// Host-broker / local-VM-lifecycle responsibility is present (only legal
    /// in host mode).
    pub has_host_lifecycle: bool,
}

/// Why a [`DaemonModeConfig`] was rejected at startup (fail-closed).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaemonModeError {
    /// Host mode carries realm provider/relay config it must not own.
    HostCarriesRealmConfig,
    /// Gateway mode carries host-broker / local-lifecycle responsibility it
    /// must not own.
    GatewayCarriesHostLifecycle,
}

impl core::fmt::Display for DaemonModeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            DaemonModeError::HostCarriesRealmConfig => write!(
                f,
                "host-mode nixlingd must not carry realm provider/relay config"
            ),
            DaemonModeError::GatewayCarriesHostLifecycle => write!(
                f,
                "gateway-mode nixlingd must not carry host-broker/local-lifecycle responsibility"
            ),
        }
    }
}

impl std::error::Error for DaemonModeError {}

impl DaemonModeConfig {
    /// The mode this config implies: a realm entrypoint selects gateway,
    /// otherwise host.
    pub fn selected_mode(&self) -> DaemonMode {
        if self.has_realm_entrypoint {
            DaemonMode::Gateway
        } else {
            DaemonMode::Host
        }
    }

    /// Validate the config against its selected mode (fail-closed). Host mode
    /// rejects realm provider/relay config; gateway mode rejects
    /// host-broker/local-lifecycle responsibility.
    pub fn validate(&self) -> Result<DaemonMode, DaemonModeError> {
        match self.selected_mode() {
            DaemonMode::Host => {
                if self.has_provider_or_relay_config {
                    return Err(DaemonModeError::HostCarriesRealmConfig);
                }
                Ok(DaemonMode::Host)
            }
            DaemonMode::Gateway => {
                if self.has_host_lifecycle {
                    return Err(DaemonModeError::GatewayCarriesHostLifecycle);
                }
                Ok(DaemonMode::Gateway)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nixling_constellation_core::{
        IdempotencyKey, NodeId, OpaquePayload, OperationId, OperationKind, RealmPath,
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
    fn target_resolver_resolves_local_target_host_resident() {
        let resolver = TargetResolver::local_only();
        let target = TargetName::parse("demo.nixling").unwrap();
        assert!(matches!(
            resolver.resolve(&target),
            Ok(DispatchTarget::HostResident { .. })
        ));
    }

    #[test]
    fn target_resolver_unknown_realm_fails_closed() {
        // The local-only table has no entry for a named realm: resolution
        // must fail closed rather than silently default to local dispatch.
        let resolver = TargetResolver::local_only();
        let target = TargetName::parse("demo.aca.work.nixling").unwrap();
        assert!(matches!(
            resolver.resolve(&target),
            Err(ResolveError::NoEntrypoint(_))
        ));
    }

    #[test]
    fn daemon_mode_is_selected_by_realm_entrypoint() {
        let host = DaemonModeConfig::default();
        assert_eq!(host.selected_mode(), DaemonMode::Host);
        let gateway = DaemonModeConfig {
            has_realm_entrypoint: true,
            ..Default::default()
        };
        assert_eq!(gateway.selected_mode(), DaemonMode::Gateway);
    }

    #[test]
    fn host_mode_refuses_realm_provider_relay_config() {
        // A bare host config validates as Host.
        assert_eq!(DaemonModeConfig::default().validate(), Ok(DaemonMode::Host));
        // Realm provider/relay config without an entrypoint is host-mode
        // carrying realm config — rejected fail-closed.
        let cross_wired = DaemonModeConfig {
            has_provider_or_relay_config: true,
            ..Default::default()
        };
        assert_eq!(
            cross_wired.validate(),
            Err(DaemonModeError::HostCarriesRealmConfig)
        );
    }

    #[test]
    fn gateway_mode_refuses_host_lifecycle_responsibility() {
        // A gateway with provider/relay config but no host lifecycle is fine.
        let gateway = DaemonModeConfig {
            has_realm_entrypoint: true,
            has_provider_or_relay_config: true,
            has_host_lifecycle: false,
        };
        assert_eq!(gateway.validate(), Ok(DaemonMode::Gateway));
        // A gateway that also claims host lifecycle is cross-wired — rejected.
        let cross_wired = DaemonModeConfig {
            has_realm_entrypoint: true,
            has_host_lifecycle: true,
            ..Default::default()
        };
        assert_eq!(
            cross_wired.validate(),
            Err(DaemonModeError::GatewayCarriesHostLifecycle)
        );
    }
}
