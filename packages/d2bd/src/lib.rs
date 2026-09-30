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
/// The manager-side authority publication coordinator (U7, KTD6-KTD7).
///
/// Re-exported so this package's owning integration test drives the same
/// construction production will install, rather than a test-local imitation of
/// it. U34 wires it into the resource plane's mutation path and removes the
/// old graph-construction entry point in the same cutover.
pub mod authority_publication;
pub(crate) mod effect_service_actors;
pub(crate) mod plane_port;
pub mod principal_allocation;
pub(crate) mod provider_lifecycle;
pub(crate) mod resource_plane_v3;

/// The new-graph limit and emergency admission (U40, KTD6-KTD10).
///
/// Re-exported so this package's owning integration test drives the same
/// construction production will install, rather than a test-local imitation of
/// it. U34 wires it into the plane's admission and effect boundaries and
/// removes the old fence and allow-all paths in the same cutover.
pub mod graph_limits_admission;

/// The plane's new-graph mutation admission construction (U6, KTD4).
///
/// Re-exported so this package's owning integration test drives the same
/// construction production will install, rather than a test-local imitation of
/// it. U34 removes this export together with the construction it names.
pub use provider_lifecycle::AuthorityPublication;
pub use graph_limits_admission::{AcceptedLimits, GraphLimitsAdmission};
pub use resource_plane_v3::GraphMutationAdmission;

include!("composition.rs");
