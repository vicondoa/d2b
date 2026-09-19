//! The `system-core` bootstrap Provider.
//!
//! `system-core` is the one fixed core-controller process per Zone, and it
//! is also `Provider/system-core`. It and the fixed `system-minijail`
//! controller are the only Providers not represented by Process resources
//! (`ADR-046-provider-model-and-packaging`, section "system-core
//! bootstrap").
//!
//! It owns exactly two things:
//!
//! - Host reconciliation, including the non-negotiable no-isolation posture
//!   the user-only Host carries;
//! - local User discovery and status.
//!
//! It owns nothing else. Process and EphemeralProcess belong to
//! `system-systemd` and `system-minijail`; Volume, Network, Device,
//! Credential, and every semantic runtime, desktop, or cloud ResourceType
//! belong to their own Providers. That negative list is enforced here as an
//! allowlist rather than documented as a convention, so a later caller
//! cannot hand this Provider a ResourceType the specification denied it:
//! see [`ownership`].
//!
//! Like every Provider, `system-core` performs no privileged mutation. It
//! resolves no host path, opens no socket, and calls neither NSS nor the
//! broker. Local User discovery reaches the host only through the injected
//! [`UserDiscoveryEffectPort`], whose sole implementor is the fixed core
//! effect adapter; the broker remains the sole privileged executor and
//! audit owner.
//!
//! No raw UID, GID, home directory, shell, unit name, cgroup path, or OS
//! username appears in any type here. Identity travels as an opaque digest
//! and as typed resource references.

#![deny(missing_docs)]

mod error;
mod host;
mod user;

/// The Host ResourceType spec and status shapes owned by the system-core
/// Provider (the census-resolved home of the host primitive contract).
pub mod host_spec;
/// The User ResourceType spec and status shapes owned by the system-core
/// Provider (the census-resolved home of the user primitive contract).
pub mod user_spec;

pub mod ownership;
pub mod testing;

pub use error::SystemCoreError;
pub use host::{
    HostCapabilityClass, HostObservationReport, HostProbeEffectPort,
    HostProbeMetadata, HostProbeSnapshot, HostReconciler, HostStatusReport,
    ISOLATION_POSTURE_MESSAGE, MinijailPlatformGate, NO_ISOLATION_STATUS_FIELDS,
};
pub use host_spec::*;
pub use ownership::{DISOWNED_RESOURCE_TYPES, OWNED_RESOURCE_TYPES};
pub use user::{
    DiscoveredUser, UserBinding, UserDiscoveryCondition, UserDiscoveryEffectPort,
    UserIdentityDigest, UserObservation, UserReconciler, UserStatusReport,
};
pub use user_spec::*;

/// The Provider name this bootstrap controller implements.
pub const PROVIDER_NAME: &str = "system-core";

/// The canonical `Provider/system-core` reference.
///
/// This is the only value admitted by `Host.spec.providerRef`, and it is
/// the same constant the Host primitive contract pins.
pub const PROVIDER_REF: &str = host_spec::HOST_PROVIDER_REF;

/// The canonical `Provider/system-core` resource UID.
///
/// This is the fixed UID the daemon's bootstrap admits for the bootstrap
/// Provider's own subject row. It is not part of any wire contract; the
/// bus keeps its own copy until the daemon's composition re-homes its
/// subject installation onto this constant.
pub const PROVIDER_UID: &str = "11111111-1111-4111-8111-111111111111";
