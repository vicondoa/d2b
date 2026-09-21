//! Security-key Device Provider contracts.
//!
//! This crate owns the unprivileged relay/frontend Process declarations and
//! the bounded lease/session protocol. Core alone resolves the physical
//! hidraw effect and places the returned fd in the relay LaunchTicket.

#![deny(missing_docs)]

mod authority;
mod controller;
mod driver;
pub mod effects_service;
pub mod facets;
mod lease;
mod process;
pub mod relay;
mod relay_service;
pub mod vocabulary;

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

pub use authority::{
    PhysicalAuthorityLease, PhysicalUsbBackingClaim, PhysicalUsbBackingToken, RelayLaunchTicket,
    SecurityKeyAdmission, SecurityKeyEffectError, SecurityKeyEffectPort, SecurityKeyOpenIntent,
};
pub use controller::{
    SECURITY_KEY_BINDING_FINALIZER, SECURITY_KEY_MAX_REPAIR_INTERVAL_SECS,
    SECURITY_KEY_REPAIR_INTERVAL_SECS, SECURITY_KEY_SERVICE_FINALIZER, SecurityKeyController,
    SecurityKeyControllerError, SecurityKeyPhase, SecurityKeyReconcileOutcome,
    SecurityKeyRunnerContract, security_key_runner_contract,
};
pub use driver::{
    SECURITY_KEY_BINDING_CONTROLLER_REF, SECURITY_KEY_BINDING_CREATIONS,
    SECURITY_KEY_REGISTRATIONS, SECURITY_KEY_RESYNC, SECURITY_KEY_SERVICE_CONTROLLER_REF,
    SECURITY_KEY_SERVICE_CREATIONS, SecurityKeyComponent, SecurityKeyDriverArgs,
    SecurityKeyDriverEffects, declared_dependency_refs, security_key_descriptors,
};
pub use effects_service::SECURITY_KEY_EFFECTS_SERVICE;
pub use lease::{LeaseState, SecurityKeyLease, SecurityKeyLeaseError, SecurityKeySessionId};
pub use process::{
    FrontendProcessDeclaration, ProcessDeclarationError, RelayProcessDeclaration,
    SecurityKeyProcessRole, security_key_process_name,
};
pub use relay::{
    CEREMONY_TIMEOUT, CTAPHID_BROADCAST_CID, CTAPHID_CANCEL, CTAPHID_ERROR,
    CTAPHID_ERR_CHANNEL_BUSY, CTAPHID_ERR_INVALID_CMD, CTAPHID_INIT, CTAPHID_INIT_PKT_BIT,
    CTAPHID_REPORT_SIZE, CidTranslator, CtaphidContPacket, CtaphidInitPacket, CtaphidPacket,
    CtaphidReport, LeaseId, QUEUE_WAIT_TIMEOUT, SecurityKeyState, build_cancel_packet,
    build_error_report, build_init_packet, parse_ctaphid_report,
};
pub use relay_service::{
    AsyncHidrawDevice, HidrawDevice, PeerAuthError, SkAcceptAbort, SkAcceptHandle, SkSessionTable,
    authenticate_peer, bind_accept_socket, spawn_accept_loop,
};

/// Provider identity.
pub const PROVIDER_REF: &str = "Provider/device-security-key";
/// Device extension schema identifier.
pub const DEVICE_SECURITY_KEY_SCHEMA_ID: &str = "device-security-key.d2bus.org/Device/spec";
/// Device Provider finalizer.
pub const DEVICE_SECURITY_KEY_FINALIZER: &str = "device-security-key.d2bus.org/lease-released";
/// The provider-neutral security-key Service ResourceType.
pub const SECURITY_KEY_SERVICE_RESOURCE_TYPE: &str = "security-key.d2bus.org.SecurityKeyService";
/// The provider-neutral security-key Binding ResourceType.
pub const SECURITY_KEY_BINDING_RESOURCE_TYPE: &str = "security-key.d2bus.org.SecurityKeyBinding";
/// Minimum bounded recent-session ring size the controller admits.
pub const MIN_SESSION_RING_SIZE: usize = 8;
/// Maximum bounded recent-session ring size the controller admits.
pub const MAX_SESSION_RING_SIZE: usize = 256;
/// Default bounded recent-session ring size.
pub const DEFAULT_SESSION_RING_SIZE: usize = 32;
