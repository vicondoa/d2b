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

mod bootstrap;
mod error;
mod host;
mod nss;
mod user;

pub mod ownership;
pub mod testing;

pub use bootstrap::{BootstrapCapability, BootstrapError, BootstrapSequence, BootstrapStage};
pub use error::SystemCoreError;
pub use host::{
    BudgetReservation, HostCapabilityClass, HostObservationReport, HostProbeEffectPort,
    HostProbeMetadata, HostProbeSnapshot, HostReconciler, HostStatusReport,
    ISOLATION_POSTURE_MESSAGE, MinijailPlatformGate, NO_ISOLATION_STATUS_FIELDS,
};
pub use nss::{
    MAX_OBSERVED_GROUPS, NssUserEffectPort, NssUserReconciler, NssUserRecord, NssUserStatus,
};
pub use ownership::{DISOWNED_RESOURCE_TYPES, OWNED_RESOURCE_TYPES};
pub use user::{
    DiscoveredUser, UserBinding, UserDiscoveryCondition, UserDiscoveryEffectPort,
    UserIdentityDigest, UserObservation, UserReconciler, UserStatusReport,
};

/// The Provider name this bootstrap controller implements.
pub const PROVIDER_NAME: &str = "system-core";

/// The canonical `Provider/system-core` reference.
///
/// This is the only value admitted by `Host.spec.providerRef`, and it is
/// the same constant the Host primitive contract pins.
pub const PROVIDER_REF: &str = d2b_contracts::v3::host::HOST_PROVIDER_REF;
