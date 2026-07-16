//! Runtime-only provider traits, registries, lifecycle, and authenticated RPC
//! proxies for d2b 2.0.
//!
//! Serialized values remain owned by `d2b-contracts`. This crate contains no
//! provider implementation, SDK, dynamic loading, or ambient fallback path.

mod context;
mod error;
mod instance;
mod registry;
mod rpc;

pub use context::{CancellationToken, OwnedOperationContext};
pub use error::{FactoryError, ProviderRuntimeError, RegistryBuildError, RegistryShutdownReport};
pub use instance::{
    ProviderFactory, ProviderInstance, provider_capabilities_are_dispatchable,
    provider_inspection_method, provider_method_is_dispatchable,
};
pub use registry::{
    AdmissionOptions, AdmittedProvider, InFlightPermit, ProviderRegistry, ProviderRegistryBuilder,
    ProviderRegistryManager, RegistryLimits,
};
pub use rpc::{
    AuthenticatedProviderRpc, ProviderClock, RpcCall, RpcOperation, RpcPayload, RpcProviderProxy,
    RpcResponse, SessionIdentity, SystemProviderClock,
};

pub use d2b_contracts::v2_provider::{
    AudioProvider, CredentialProvider, DeviceProvider, DisplayProvider, InfrastructureProvider,
    NetworkProvider, ObservabilityProvider, Provider, RuntimeProvider, StorageProvider,
    SubstrateProvider, TransportProvider,
};
