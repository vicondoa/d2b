//! Scheme-keyed transport fabric (ADR 0032/0045).
//!
//! [`TransportFabric`] composes existing [`TransportProvider`] impls (the
//! in-crate [`crate::LoopbackTransport`], [`crate::LocalTcpTransport`], and
//! any future transport) behind one [`TransportProvider`] facade, keyed by a
//! bounded, validated URI-style scheme parsed from
//! [`TransportTarget::endpoint`]. It is itself just another
//! `TransportProvider`: it adds no new endpoint kind, credential, or
//! address-parsing rule beyond delegating to whichever already-validated
//! transport owns the scheme.
//!
//! - `connect()` parses the scheme prefix (the substring before the first
//!   `"://"`, or the whole endpoint when there is no `"://"`, which is how
//!   [`crate::LoopbackTransport`]'s bare `"loopback"` target reads) and
//!   dispatches to the transport registered for that scheme. An
//!   unregistered scheme fails closed with
//!   [`d2b_realm_core::ErrorKind::InvalidTarget`] — there is no default
//!   transport.
//! - `listen()` fans out to every registered transport and returns one
//!   [`TransportListener`] whose `accept()` races every sub-listener's
//!   `accept()` (via a bounded [`tokio::task::JoinSet`], the "rt" tokio
//!   feature already enabled by this crate — no new dependency) and
//!   resolves to the first session accepted on ANY of them. A sub-listener
//!   that errors does not fail the whole fan-out: the race keeps waiting on
//!   the remaining listeners and only surfaces an error once every
//!   registered transport has failed. On success every other in-flight
//!   accept task is aborted (bounded, explicit cancellation — no leaked
//!   background accept loops).
//!
//! Registration (`register`) is bounded ([`MAX_FABRIC_TRANSPORTS`]) and
//! rejects a duplicate scheme or a malformed scheme literal, so the fabric's
//! scheme table can never grow without bound or become ambiguous.
//!
//! This module carries no realm relay/session/provider credentials, no
//! remote node registry, and no free-form path/argv construction — it is
//! strictly a byte-transport composition, matching the dependency-direction
//! and ADR 0032/0045 boundaries documented at the crate root.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use d2b_realm_core::{ErrorKind, NodeId, ProviderId};
use d2b_realm_provider::error::{ProviderError, ProviderResult};
use d2b_realm_provider::provider::{TransportListener, TransportProvider};
use d2b_realm_provider::types::{NodeRegistration, TransportSession, TransportTarget};
use tokio::task::JoinSet;

/// Maximum number of distinct schemes one fabric may register. Bounds the
/// scheme table and the per-`listen()` fan-out width.
pub const MAX_FABRIC_TRANSPORTS: usize = 16;

/// Maximum length of a [`FabricScheme`] literal.
pub const MAX_FABRIC_SCHEME_LEN: usize = 32;

/// A validated, bounded transport scheme literal (the part of a
/// [`TransportTarget::endpoint`] before `"://"`, or the whole endpoint for a
/// bare, delimiter-free target). Grammar: `ALPHA *( ALPHA / DIGIT / "+" /
/// "-" / "." )`, matching the URI scheme production (RFC 3986 §3.1) closely
/// enough to reject anything surprising while still accepting the crate's
/// own `"loopback"` and `"tcp+local"` literals. Comparison is
/// case-insensitive; the stored form is lowercased.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FabricScheme(String);

impl FabricScheme {
    /// Parse and validate a scheme literal.
    pub fn parse(raw: &str) -> Result<Self, FabricError> {
        if raw.is_empty() || raw.len() > MAX_FABRIC_SCHEME_LEN {
            return Err(FabricError::InvalidScheme);
        }
        let mut chars = raw.chars();
        let first = chars.next().ok_or(FabricError::InvalidScheme)?;
        if !first.is_ascii_alphabetic() {
            return Err(FabricError::InvalidScheme);
        }
        if !chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')) {
            return Err(FabricError::InvalidScheme);
        }
        Ok(Self(raw.to_ascii_lowercase()))
    }

    /// Borrow the normalized (lowercased) scheme literal.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Extract the scheme from a transport endpoint: the substring before
    /// the first `"://"`, or the whole endpoint when no `"://"` is present
    /// (covering a bare loopback-style target).
    fn from_endpoint(endpoint: &str) -> Result<Self, FabricError> {
        let raw = match endpoint.split_once("://") {
            Some((scheme, _)) => scheme,
            None => endpoint,
        };
        Self::parse(raw)
    }
}

impl std::fmt::Display for FabricScheme {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Why a fabric registration or scheme lookup was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FabricError {
    /// The scheme literal was empty, too long, or contained a character
    /// outside the validated scheme grammar.
    InvalidScheme,
    /// A transport is already registered for this scheme.
    DuplicateScheme(FabricScheme),
    /// The fabric already holds [`MAX_FABRIC_TRANSPORTS`] entries.
    TooManyTransports,
}

impl std::fmt::Display for FabricError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FabricError::InvalidScheme => write!(f, "fabric scheme literal is invalid"),
            FabricError::DuplicateScheme(scheme) => {
                write!(f, "a transport is already registered for scheme `{scheme}`")
            }
            FabricError::TooManyTransports => {
                write!(f, "fabric already holds the maximum registered transports")
            }
        }
    }
}

impl std::error::Error for FabricError {}

/// A scheme-keyed composition of [`TransportProvider`] impls behind one
/// `TransportProvider` facade. See the module documentation for the
/// `connect`/`listen` dispatch contract.
pub struct TransportFabric {
    id: ProviderId,
    transports: HashMap<FabricScheme, Arc<dyn TransportProvider>>,
}

impl TransportFabric {
    /// An empty fabric. `connect`/`listen` fail closed until at least one
    /// transport is registered.
    pub fn new() -> Self {
        Self {
            id: ProviderId::parse("realm-transport-fabric").expect("valid provider id"),
            transports: HashMap::new(),
        }
    }

    /// Register `transport` under `scheme`. Fails closed on an invalid
    /// scheme literal, a duplicate scheme, or capacity exhaustion.
    pub fn register(
        &mut self,
        scheme: &str,
        transport: Arc<dyn TransportProvider>,
    ) -> Result<(), FabricError> {
        let scheme = FabricScheme::parse(scheme)?;
        if self.transports.contains_key(&scheme) {
            return Err(FabricError::DuplicateScheme(scheme));
        }
        if self.transports.len() >= MAX_FABRIC_TRANSPORTS {
            return Err(FabricError::TooManyTransports);
        }
        self.transports.insert(scheme, transport);
        Ok(())
    }

    /// Number of registered schemes.
    pub fn len(&self) -> usize {
        self.transports.len()
    }

    /// Whether no transport is registered yet.
    pub fn is_empty(&self) -> bool {
        self.transports.is_empty()
    }

    /// The registered scheme literals, in no particular order.
    pub fn schemes(&self) -> impl Iterator<Item = &str> {
        self.transports.keys().map(FabricScheme::as_str)
    }

    fn resolve(&self, endpoint: &str) -> ProviderResult<Arc<dyn TransportProvider>> {
        let scheme = FabricScheme::from_endpoint(endpoint)
            .map_err(|_| ProviderError::new(ErrorKind::InvalidTarget, "fabric-scheme-invalid"))?;
        self.transports.get(&scheme).cloned().ok_or_else(|| {
            ProviderError::new(ErrorKind::InvalidTarget, "fabric-scheme-unregistered")
        })
    }
}

impl Default for TransportFabric {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl TransportProvider for TransportFabric {
    fn transport_id(&self) -> ProviderId {
        self.id.clone()
    }

    async fn connect(&self, target: TransportTarget) -> ProviderResult<TransportSession> {
        let transport = self.resolve(&target.endpoint)?;
        transport.connect(target).await
    }

    async fn listen(
        &self,
        registration: NodeRegistration,
    ) -> ProviderResult<Box<dyn TransportListener>> {
        if self.transports.is_empty() {
            return Err(ProviderError::new(
                ErrorKind::InvalidTarget,
                "fabric-no-registered-transports",
            ));
        }
        // Deterministic fan-out order (by scheme) so behaviour does not
        // depend on hash-map iteration order.
        let mut entries: Vec<_> = self.transports.iter().collect();
        entries.sort_by_key(|(scheme, _)| (*scheme).clone());
        let mut listeners = Vec::with_capacity(entries.len());
        for (_, transport) in entries {
            let listener: Arc<dyn TransportListener> =
                Arc::from(transport.listen(registration.clone()).await?);
            listeners.push(listener);
        }
        Ok(Box::new(FabricListener {
            node: registration.node,
            listeners,
        }))
    }
}

/// The accept side of a [`TransportFabric`]: fans in every registered
/// transport's listener behind one `accept()` race.
struct FabricListener {
    node: NodeId,
    listeners: Vec<Arc<dyn TransportListener>>,
}

#[async_trait]
impl TransportListener for FabricListener {
    fn node(&self) -> NodeId {
        self.node.clone()
    }

    async fn accept(&self) -> ProviderResult<TransportSession> {
        if self.listeners.len() == 1 {
            return self.listeners[0].accept().await;
        }
        let mut set: JoinSet<ProviderResult<TransportSession>> = JoinSet::new();
        for listener in &self.listeners {
            let listener = Arc::clone(listener);
            set.spawn(async move { listener.accept().await });
        }
        // Race every sub-listener's accept. A sub-listener erroring does not
        // fail the fan-out: keep waiting on the rest and only surface an
        // error once every registered transport has failed. On the first
        // success, abort the remaining in-flight tasks (bounded, explicit
        // cancellation).
        let mut last_err = None;
        while let Some(joined) = set.join_next().await {
            match joined {
                Ok(Ok(session)) => {
                    set.abort_all();
                    return Ok(session);
                }
                Ok(Err(err)) => last_err = Some(err),
                Err(join_err) => {
                    last_err = Some(ProviderError::new(
                        ErrorKind::RelayUnavailable,
                        format!("fabric-accept-task-failed:{join_err}"),
                    ));
                }
            }
        }
        Err(last_err.unwrap_or_else(|| {
            ProviderError::new(ErrorKind::RelayUnavailable, "fabric-listener-closed")
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{LocalTcpTransport, LoopbackTransport, conformance};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn registration() -> NodeRegistration {
        NodeRegistration {
            node: NodeId::parse("gw").unwrap(),
        }
    }

    #[test]
    fn scheme_rejects_empty_and_oversize_and_bad_chars() {
        assert_eq!(FabricScheme::parse(""), Err(FabricError::InvalidScheme));
        assert_eq!(
            FabricScheme::parse(&"a".repeat(MAX_FABRIC_SCHEME_LEN + 1)),
            Err(FabricError::InvalidScheme)
        );
        assert_eq!(FabricScheme::parse("1abc"), Err(FabricError::InvalidScheme));
        assert_eq!(
            FabricScheme::parse("tcp local"),
            Err(FabricError::InvalidScheme)
        );
        assert_eq!(
            FabricScheme::parse("tcp/local"),
            Err(FabricError::InvalidScheme)
        );
    }

    #[test]
    fn scheme_accepts_valid_literals_case_insensitively() {
        assert_eq!(
            FabricScheme::parse("Tcp+Local").unwrap().as_str(),
            "tcp+local"
        );
        assert_eq!(FabricScheme::parse("a").unwrap().as_str(), "a");
        assert_eq!(
            FabricScheme::parse("loop-back.v1").unwrap().as_str(),
            "loop-back.v1"
        );
    }

    #[test]
    fn endpoint_scheme_extraction_handles_bare_and_delimited_forms() {
        assert_eq!(
            FabricScheme::from_endpoint("loopback").unwrap().as_str(),
            "loopback"
        );
        assert_eq!(
            FabricScheme::from_endpoint("tcp+local://127.0.0.1:5000")
                .unwrap()
                .as_str(),
            "tcp+local"
        );
    }

    #[test]
    fn duplicate_scheme_registration_is_rejected() {
        let mut fabric = TransportFabric::new();
        fabric
            .register("loopback", Arc::new(LoopbackTransport::new()))
            .unwrap();
        let err = fabric
            .register("loopback", Arc::new(LoopbackTransport::new()))
            .unwrap_err();
        assert!(matches!(err, FabricError::DuplicateScheme(_)));
        assert_eq!(fabric.len(), 1);
    }

    #[test]
    fn invalid_scheme_registration_is_rejected_without_mutating_table() {
        let mut fabric = TransportFabric::new();
        let err = fabric
            .register("1bad", Arc::new(LoopbackTransport::new()))
            .unwrap_err();
        assert_eq!(err, FabricError::InvalidScheme);
        assert!(fabric.is_empty());
    }

    #[test]
    fn capacity_bound_is_enforced() {
        let mut fabric = TransportFabric::new();
        for i in 0..MAX_FABRIC_TRANSPORTS {
            fabric
                .register(&format!("scheme-{i}"), Arc::new(LoopbackTransport::new()))
                .unwrap();
        }
        assert_eq!(fabric.len(), MAX_FABRIC_TRANSPORTS);
        let err = fabric
            .register("one-too-many", Arc::new(LoopbackTransport::new()))
            .unwrap_err();
        assert_eq!(err, FabricError::TooManyTransports);
        assert_eq!(fabric.len(), MAX_FABRIC_TRANSPORTS);
    }

    #[tokio::test]
    async fn unregistered_scheme_connect_fails_closed() {
        let fabric = TransportFabric::new();
        let err = fabric
            .connect(TransportTarget {
                endpoint: "loopback".to_owned(),
            })
            .await
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidTarget);
    }

    #[tokio::test]
    async fn empty_fabric_listen_fails_closed() {
        let fabric = TransportFabric::new();
        let err = match fabric.listen(registration()).await {
            Ok(_) => panic!("an empty fabric must not produce a listener"),
            Err(err) => err,
        };
        assert_eq!(err.kind(), ErrorKind::InvalidTarget);
    }

    #[tokio::test]
    async fn bare_endpoint_routes_to_loopback_scheme() {
        let mut fabric = TransportFabric::new();
        fabric
            .register("loopback", Arc::new(LoopbackTransport::new()))
            .unwrap();
        conformance::accepts_and_round_trips(&fabric).await.unwrap();
    }

    #[tokio::test]
    async fn scheme_delimited_endpoint_routes_to_local_tcp() {
        let local_tcp = LocalTcpTransport::bind_loopback_v4().await.unwrap();
        let target = local_tcp.target();
        let mut fabric = TransportFabric::new();
        fabric
            .register(crate::local_tcp::LOCAL_TCP_SCHEME_NAME, Arc::new(local_tcp))
            .unwrap();
        conformance::accepts_and_round_trips_with_target(&fabric, target)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn fan_out_listener_serves_whichever_registered_transport_is_used() {
        let mut fabric = TransportFabric::new();
        fabric
            .register("loopback", Arc::new(LoopbackTransport::new()))
            .unwrap();
        let local_tcp = LocalTcpTransport::bind_loopback_v4().await.unwrap();
        let tcp_target = local_tcp.target();
        fabric
            .register(crate::local_tcp::LOCAL_TCP_SCHEME_NAME, Arc::new(local_tcp))
            .unwrap();

        let listener = fabric.listen(registration()).await.unwrap();
        // Connect on the tcp+local side only; the fan-out listener must
        // still resolve the accept even though the loopback side is idle.
        let (mut sender, mut accepted) =
            tokio::try_join!(fabric.connect(tcp_target), listener.accept()).unwrap();
        sender.stream_mut().write_all(b"fanout").await.unwrap();
        let mut buf = [0_u8; 6];
        accepted.stream_mut().read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"fanout");
    }

    #[tokio::test]
    async fn listener_survives_one_dead_transport_and_serves_the_other() {
        let dead = LoopbackTransport::new();
        dead.close();
        let mut fabric = TransportFabric::new();
        fabric.register("dead", Arc::new(dead)).unwrap();
        let healthy = LoopbackTransport::new();
        fabric.register("healthy", Arc::new(healthy)).unwrap();

        let listener = fabric.listen(registration()).await.unwrap();
        let (_sender, _accepted) = tokio::try_join!(
            fabric.connect(TransportTarget {
                endpoint: "healthy".to_owned(),
            }),
            listener.accept()
        )
        .expect("healthy transport still accepts despite the dead sibling erroring");
    }

    #[tokio::test]
    async fn all_transports_dead_fails_closed() {
        let dead_a = LoopbackTransport::new();
        dead_a.close();
        let dead_b = LoopbackTransport::new();
        dead_b.close();
        let mut fabric = TransportFabric::new();
        fabric.register("dead-a", Arc::new(dead_a)).unwrap();
        fabric.register("dead-b", Arc::new(dead_b)).unwrap();

        let listener = fabric.listen(registration()).await.unwrap();
        let err = listener.accept().await.unwrap_err();
        assert_eq!(err.kind(), ErrorKind::RelayUnavailable);
    }
}
