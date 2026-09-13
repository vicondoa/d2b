#![allow(clippy::result_large_err)]

pub(crate) mod endpoint_effects;
pub(crate) mod activation_effects;
pub mod process_effects;
pub(crate) mod credential_effects;
pub(crate) mod binding_effects;
pub(crate) mod volume_effects;
pub(crate) mod shared_provider_effects;
pub(crate) mod guest_effects;
pub(crate) mod system_core_effects;
pub(crate) mod interaction_child_sources;

/// The daemon-side half of the Guest target-control seam: the family crate
/// owns the channel, this module offers it the authenticated session.
pub(crate) mod guest_target_session;
pub(crate) mod resource_plane_v3;

include!("composition.rs");
