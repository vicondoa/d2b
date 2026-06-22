//! `nixling-constellation-provider` defines the v2 provider trait surface
//! plus typed capability descriptors and conformance fixtures (ADR 0032).
//!
//! Public command semantics and constellation operation DTOs stay stable
//! while providers plug in below them. Every provider advertises
//! capabilities as **data** and returns a typed denial
//! ([`error::ProviderError`]) rather than falling back to SSH, generic TCP
//! tunnels, or undocumented behavior.
//!
//! Dependency direction: this crate depends only on
//! `nixling-constellation-core` + `async-trait`. It MUST NOT depend on a
//! protocol codec, a transport implementation, or host-only broker/daemon
//! internals.

pub mod capabilities;
pub mod conformance;
pub mod credential;
pub mod error;
pub mod mock;
pub mod provider;
pub mod rate_limit;
pub mod types;

pub use credential::{
    AzureControlPlaneRef, CredentialPlane, ManagedIdentityRef, OpaqueAzureRef,
    SessionCredentialBinding,
};
pub use error::{ProviderDiagnostic, ProviderError, RetryHint};
pub use provider::{
    CredentialProvider, CredentialStatus, DaemonAccessApi, DaemonAccessTransport, DisplayProvider,
    DurableExecutionProvider, GuestControlEndpointProvider, HostSubstrateProvider,
    InfrastructureProvider, NodeProvider, ObservabilitySinkProvider, PersistentShellProvider,
    ProtocolCodec, RelayProvider, RuntimeProvider, StreamMux, TransportListener, TransportProvider,
    WorkloadProvider,
};
pub use rate_limit::{CircuitBreakerConfig, CircuitBreakerSnapshot, ProviderCircuitBreaker};
