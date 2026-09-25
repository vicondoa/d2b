//! Semantic controller policy for `Provider/device-usbip`.
//!
//! The Provider owns USB Service and Binding reconciliation plus the lifecycle
//! of its shared Host backend, per-Network relay, and per-Binding proxy. Every
//! privileged mutation remains behind an injected [`UsbipEffectPort`]; this
//! crate has no broker connection, host path, device identifier, bus id, or raw
//! firewall representation.

#![deny(missing_docs)]

pub mod broker;
pub mod core_adapter;
mod arbitration;
mod busid;
mod controller;
mod driver;
pub mod effects_service;
pub mod facets;
mod firewall;
mod lifecycle;
mod process;
mod production;
pub mod reconcile_state;
mod state_machine;
pub mod vocabulary;
mod workers;

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

pub use arbitration::{UsbipArbitrator, UsbipClaim, UsbipClaimError};
pub use busid::{BusId, FirewallOwnershipMarker, MAX_BUS_ID_BYTES, PhysicalUsbBackingToken};
pub use controller::{
    NetworkDependency, ScopedResourceUid, USBIP_BINDING_FINALIZER, USBIP_MAX_REPAIR_INTERVAL_SECS,
    USBIP_REPAIR_INTERVAL_SECS, USBIP_SERVICE_FINALIZER, UsbipBindingAdmission,
    UsbipBindingController, UsbipBindingControllerError, UsbipBindingPhase,
    UsbipBindingReconcileResult, UsbipController, UsbipControllerError, UsbipMetricLabels,
    UsbipOperation, UsbipOutcome, UsbipRunnerContract, UsbipServicePhase, usbip_runner_contract,
};
pub use d2b_contracts::usbip::validate_bus_id;
pub use driver::{
    USBIP_BINDING_CONTROLLER_REF, USBIP_REGISTRATIONS, USBIP_RESYNC, USBIP_SERVICE_CONTROLLER_REF,
    UsbipComponent, UsbipDriverArgs, UsbipDriverEffects, declared_dependency_refs,
    usbip_descriptors,
};
pub use effects_service::USBIP_EFFECTS_SERVICE;
pub use firewall::{
    FirewallConfirmation, FirewallConfirmationKind, FirewallDigest, FirewallGenerationFence,
    FirewallObservation, FirewallProjectionAction, FirewallProjectionIntent, FirewallToken,
    RelayAuthorityLease, UsbipEffectError, UsbipEffectPort,
};
pub use lifecycle::{
    AttachProcessIdentity, AttachmentObservation, BindingIdentity, BindingLifecycle,
    BindingLifecycleError, BindingPhase, BindingPort, BindingProxyLease, BindingSlotLease,
    OwnedBusBinding, PhysicalAuthorityLease, ServiceLifecycle, ServiceLifecycleError, ServicePhase,
    ServicePort, ServiceRelayLease, SupervisorFinalizeError, UsbipSupervisor,
    binding_child_resources,
};
pub use process::{AttachSource, EphemeralProcessIntent, EphemeralProcessKind, UsbipDaemonProcess};
pub use production::{ProductionPort, UsbipBrokerDispatcher};
pub use state_machine::{
    CANONICAL_STEPS, UsbipBusidPlan, UsbipBusidStep, UsbipClaimSource, UsbipExecutionReport,
    UsbipPlanError, UsbipStepExecutor, build_usbip_explicit_plan, build_usbip_plan,
    execute_usbip_plan,
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
