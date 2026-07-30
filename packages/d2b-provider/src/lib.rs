//! The v3 Provider model surface: descriptors, registry, session identity,
//! and forwarding admission.
//!
//! This crate is the Zone-side Provider registry. It holds one registry
//! generation per Zone, admits authenticated calls against it, drains and
//! retires a generation, and republishes a replacement live. It is adapted
//! from the ADR45 `d2b-provider` registry: the lifecycle, in-flight
//! accounting, RAII permit, drain-waiter notify race, and live-swap manager
//! are carried over, while the identity is now the Zone's
//! [`ZonePath`](d2b_contracts::v3::zone_routing::ZonePath) plus an
//! authenticated Zone principal rather than a realm and a peer role.
//!
//! What this crate deliberately does not do. It performs no host mutation and
//! opens no socket: a Provider never mutates host state directly, and every
//! privileged effect belongs to a typed, audited broker op reached through an
//! injected effect port in a Provider implementation crate. No public type
//! here carries a numeric UID or GID, a device node, a store path, a socket
//! path, or any host path; a Provider is named only by its Zone path and its
//! `Provider/<name>` reference. No type here carries authority: an
//! [`InFlightPermit`] is a concurrency slot, and the grants in
//! [`LocalHopGrants`] are the local RBAC engine's already-reached decisions,
//! not a transferable capability.
//!
//! It also does not name the Provider trait-object catalog. The ADR45
//! `ProviderInstance` sum type and the `RpcProviderProxy` payload and
//! response enums are built from Provider method DTOs owned by
//! `d2b-contracts`, and the v3 replacements for those DTOs do not exist yet.
//! Rather than invent them here, [`ProviderRegistry`] is generic over the
//! Zone runtime's own instance handle, and [`ProviderClass`] preserves the
//! eleven frozen Provider families as a discriminant.

#![deny(missing_docs)]

mod context;
mod descriptor;
mod error;
mod forwarding;
mod identity;
mod installation;
mod registry;
mod session;

pub use context::{CancellationToken, OwnedOperationContext};
pub use descriptor::ProviderDescriptor;
pub use error::{ProviderRuntimeError, RegistryBuildError};
pub use forwarding::{
    ForwardTarget, ForwardedCall, LocalHopGrants, ProviderForwardRequest,
    ZoneRouteFailClosedReason, admit_provider_forward,
};
pub use identity::{
    MAX_PROVIDER_CAPABILITIES, MAX_PROVIDER_REGISTRY_ENTRIES, PROVIDER_RESOURCE_TYPE,
    PROVIDER_SCHEMA_VERSION, ProviderCapabilitySet, ProviderClass, ProviderImplementationId,
    ProviderMethodName,
};
pub use installation::{
    InstalledProvider, ProviderReadiness, RequiredProviderApi, admit_installation,
};
pub use registry::{
    AdmissionOptions, AdmittedProvider, InFlightPermit, MAX_REGISTRY_DRAIN_MS, ProviderRegistry,
    ProviderRegistryBuilder, ProviderRegistryManager, ProviderRegistrySnapshot,
    RegistryDrainPolicy, RegistryLifecycle, RegistryLimits, RegistryShutdownReport,
};
pub use session::SessionIdentity;
