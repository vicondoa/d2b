//! The `DeviceBinding` provider crate: the `DeviceBinding` resource type's
//! driver, its spec decoder, its read-side row helpers, its driver
//! declaration, and the implementation of the family's driver effects.
//!
//! The crate owns the binding type's complete row-side resource knowledge:
//! the strict decode of the canonical `DeviceBindingRequest` the Device source
//! commits, the claim-and-attach drive that realizes one admitted device
//! capability for one consumer slot, the fenced readiness projection the
//! binding actor publishes, the drain gate and the attachment-first teardown
//! that give the claim back, the driver's validate, recover, reconcile,
//! pre-drain, and delete verbs, and the
//! [`DriverDescriptor`](d2b_resource_types::DriverDescriptor) the plane
//! registers the type by.
//!
//! The row is realized against the Device provider's trusted inventory, never
//! against a device node path: the family mints no child, and the only row it
//! reads is the parent `Device` it names, so the physical authority, the
//! presence of the named capability, and every device-side surface stay with
//! the source that arbitrated them.
//!
//! The family's driver effects (U6) are implemented by this crate itself
//! ([`crate::effects_service`]): the claim-and-attach drive, the two releases,
//! and the two observations the daemon realizes cross the provider boundary
//! as the declared [`crate::facets`] the composition root supplies. The daemon
//! hosts the family's declared effects service
//! ([`crate::effects_service::DEVICE_BINDING_EFFECTS_SERVICE`]) per zone from
//! the family's registered factory; no externally built port appears at any
//! construction site (R2).

#![deny(missing_docs)]

#[cfg(any(test, feature = "test-support"))]
/// Recording test doubles shared with downstream crates' unit tests, gated
/// behind the `test-support` Cargo feature so production consumers never
/// pull them in.
pub mod test_support;

mod driver;
mod effects_service;
mod facets;
mod row_readers;

pub use driver::{
    DEVICE_BINDING_CREATIONS, DEVICE_BINDING_READS, DEVICE_BINDING_TYPE_NAME,
    DeviceBindingDriverArgs, DeviceBindingDriverEffects, binding_descriptor, binding_spec_decoder,
};
pub use effects_service::{
    DEVICE_BINDING_EFFECTS_SERVICE, DeviceBindingEffectsService, DeviceBindingEffectsServiceFactory,
};
pub use facets::{
    AttachmentMediation, AttachmentObservation, DeviceAttachment, DeviceBindingEffectFacets,
    DeviceBindingFence, DeviceEstablishOutcome, DeviceRefusal,
};
pub use row_readers::{BindingReadiness, binding_readiness_current, parsed_binding_spec};