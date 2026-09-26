#![allow(clippy::result_large_err)]

//! The `d2bd` daemon composition wires the static Provider deployment and
//! effect adapter (U12), the Zone resource planes and their controller
//! runtimes, the interaction families (display-wayland, audio,
//! clipboard, notification, shell-pool),the guest target-control seam,
//! and the operator dispatch over the public socket. [`serve`] runs the
//! daemon accept loop; [`lock_only`] holds the state lock alone. The
//! CLI contract the daemon serves is `docs/reference/cli-contract.md`,
//! and its error surface is `docs/reference/error-codes.md`.

pub(crate) mod shared_provider_effects;


/// The daemon-side half of the Guest target-control seam: the family crate
/// owns the channel, this module offers it the authenticated session.
pub(crate) mod guest_target_session;
pub(crate) mod zone_enrollment;
pub(crate) mod foundation_seed;
pub(crate) mod forward_rendezvous;
pub(crate) mod effect_service_actors;
pub(crate) mod plane_port;
pub mod principal_allocation;
pub(crate) mod provider_lifecycle;
pub(crate) mod resource_plane_v3;

include!("composition.rs");
