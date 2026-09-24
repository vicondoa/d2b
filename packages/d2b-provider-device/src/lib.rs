//! The `Device` resource type's driver.
//!
//! The `Device` ResourceType is served by four hardware Providers (TPM,
//! USBIP, security-key, GPU) and the plane keys one driver per ResourceType,
//! so the type's driver is declared once - here - over the four Provider
//! rows. The rows take their identity from the realizer crates' exported
//! `PROVIDER_REF` constants, and the typed effect behind
//! [`DeviceDriverEffects`] dispatches on the row's declared component.
//!
//! The crate holds no host authority: the production effect implementation
//! (the TPM controller, the GPU authority fence, the USBIP and security-key
//! device admission) stays in the daemon behind the port, and the per-resource
//! Provider state the effects keep travels back through the driver's own
//! state.

#![deny(missing_docs)]

mod driver;
pub mod effects_service;
pub mod facets;

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

pub use driver::{
    DEVICE_REGISTRATIONS, DEVICE_RESYNC, DEVICE_TYPE_NAME, DeviceComponent, DeviceDriverArgs,
    DeviceDriverEffects, DeviceResourceState, GPU_CONTROLLER_REF, SECURITY_KEY_CONTROLLER_REF,
    TPM_CONTROLLER_REF, USBIP_CONTROLLER_REF, declared_dependency_refs, device_descriptor,
};
pub use effects_service::DEVICE_EFFECTS_SERVICE;
