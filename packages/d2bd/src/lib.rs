#![allow(clippy::result_large_err)]

pub mod endpoint_driver;
pub mod process_driver;
pub mod credential_driver;
pub mod binding_driver;
pub mod volume_driver;
pub mod activation_driver;
pub(crate) mod shared_provider_driver;
pub(crate) mod shared_provider_effects;
pub(crate) mod system_core_driver;

pub(crate) mod resource_plane_v3;

include!("composition.rs");
