#![allow(clippy::result_large_err)]

pub(crate) mod endpoint_effects;
pub mod process_effects;
pub mod credential_driver;
pub mod binding_driver;
pub mod volume_driver;
pub mod activation_driver;
pub(crate) mod shared_provider_driver;
pub(crate) mod shared_provider_effects;
pub(crate) mod guest_effects;
pub(crate) mod system_core_driver;
/// U12: the core-family `ResourceDriver`. Registered on the v3 plane by
/// `resource_plane_v3`; until that registration lands the module is only
/// exercised by its tests.
pub(crate) mod core_driver;
pub(crate) mod interaction_driver;

/// The daemon-side half of the Guest target-control seam: the family crate
/// owns the channel, this module offers it the authenticated session.
pub(crate) mod guest_target_session;
pub(crate) mod resource_plane_v3;

include!("composition.rs");
