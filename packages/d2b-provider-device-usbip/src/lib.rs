//! Semantic controller policy for `Provider/device-usbip`.
//!
//! The Provider owns USB Service and Binding reconciliation plus the lifecycle
//! of its shared Host backend, per-Network relay, and per-Binding proxy. Every
//! privileged mutation remains behind an injected [`UsbipEffectPort`]; this
//! crate has no broker connection, host path, device identifier, bus id, or raw
//! firewall representation.

#![deny(missing_docs)]

mod controller;
mod firewall;
mod workers;

pub use controller::{
    NetworkDependency, ScopedResourceUid, UsbipController, UsbipControllerError, UsbipMetricLabels,
    UsbipOperation, UsbipOutcome, UsbipServicePhase,
};
pub use firewall::{
    FirewallConfirmation, FirewallConfirmationKind, FirewallDigest, FirewallGenerationFence,
    FirewallObservation, FirewallProjectionAction, FirewallProjectionIntent, FirewallToken,
    RelayAuthorityLease, UsbipEffectError, UsbipEffectPort,
};
pub use workers::{
    AttachmentActivation, AttachmentCommand, UsbipWorkerClass, UsbipWorkerDeclaration,
};

/// Provider resource reference used by descriptors and RBAC bindings.
pub const PROVIDER_REF: &str = "Provider/device-usbip";
/// Provider-neutral USB authority Service ResourceType.
pub const USB_SERVICE_RESOURCE_TYPE: &str = "usb.d2bus.org.UsbService";
/// Provider-neutral per-Guest USB Binding ResourceType.
pub const USB_BINDING_RESOURCE_TYPE: &str = "usb.d2bus.org.UsbBinding";
/// Conflict reason for a second relay owner on one Network.
pub const USBIP_NETWORK_RELAY_AUTHORITY_CONFLICT: &str = "usbip-network-relay-authority-conflict";
