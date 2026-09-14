//! The VolumeBinding provider crate: the VolumeBinding resource type's
//! driver, its spec decoder, its read-side row helpers, and its driver
//! declaration.
//!
//! The crate owns the binding type's complete resource knowledge: the
//! derived virtiofsd worker plan the frozen `volume-virtiofs` contract
//! admits, the binding-owned worker Process and Endpoint children one
//! binding mints, the fenced readiness projection its actor publishes, the
//! driver's validate, recover, reconcile, finalize, and delete verbs, and the
//! [`DriverDescriptor`](d2b_resource_types::DriverDescriptor) the plane
//! registers the type by.
//!
//! Everything the driver needs from outside arrives through the driver
//! effect port ([`BindingDriverEffects`]): the serving socket probe, the
//! socket removal, and the guest-mount observation the daemon realizes. The
//! production implementation lives in the daemon behind that port, so this
//! crate carries no host state.

#![deny(missing_docs)]

mod driver;
mod row_readers;

pub use driver::{
    BINDING_CREATIONS, BINDING_TYPE_NAME, BindingDriverArgs, BindingDriverEffects,
    binding_descriptor, binding_spec_decoder,
};
pub use row_readers::{binding_readiness_current, parsed_binding_spec};
