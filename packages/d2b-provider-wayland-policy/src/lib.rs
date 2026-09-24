//! The WaylandPolicy resource type and the interaction family's shared engine.
//!
//! This crate is the display wayland policy's home: the `WaylandPolicy`
//! driver, its spec decoder, and the driver declaration the resource plane
//! registers the type by. It is also the family's root crate, so the six
//! interaction types' shared engine lives here beside it: the reconcile,
//! recover, finalize, and delete verbs, the spec-envelope decode, the
//! manager-child plumbing, the shared spec vocabulary ([`vocabulary`]), and
//! the family's driver effects ([`effects_service`]) with their declared
//! facets ([`facets`]).
//!
//! The other five types - `WaylandSession`, `AudioService`, `AudioBinding`,
//! `ShellPool`, and `ShellSession` - each live in their own crate and build
//! their behavior on the same engine. Every one of them owns its row, its
//! decoder, its factory, and its descriptor; none of them is declared by the
//! daemon.
//!
//! The family's driver effects are implemented by this crate itself
//! ([`crate::effects_service`]): the display session admission against the
//! committed interaction identity, the audio-pipewire controller registry
//! ([`crate::audio_registry`]), and the shell pool/session reference checks
//! run inside the crate over the daemon-supplied facet set
//! ([`crate::facets`]). The daemon hosts the family's declared effects
//! service (`interaction.d2bus.org/effects`) per zone from the family's
//! registered factory; no externally built port appears at any construction
//! site (R2).

#![deny(missing_docs)]

mod audio_registry;
pub mod effects_service;
pub mod facets;
pub mod interaction;
mod vocabulary;
mod wayland_policy;

// The scripted InteractionDriverEffects double. Needed both by
// external crates (d2bd's plane tests, which opt in via the `test-support`
// feature) and by this crate's own tests. Gating on
// `any(test, feature = "test-support")` makes it available automatically to
// this crate's unit tests. Integration tests that need it declare
// `required-features`, so run those with `--features test-support` (or let
// the Bazel `*_test_support` target compile them).
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

pub use effects_service::{
    INTERACTION_EFFECTS_SERVICE, InteractionEffectsService, InteractionEffectsServiceFactory,
};
pub use facets::{
    AudioMediatorSource, InteractionEffectFacets, InteractionEffectIdentity,
    InteractionIdentitySource, InteractionPlaneRead,
};
pub use interaction::{
    InteractionChild, InteractionChildContext, InteractionDriver, InteractionDriverArgs,
    InteractionDriverEffects, InteractionDriverError, InteractionDriverFactory,
    InteractionDriverStatus, InteractionEffectError, InteractionEffectOutcome,
    InteractionEffectPhase, InteractionEffectRequest, InteractionFinalize, InteractionKind,
    InteractionSpecDecodeError, InteractionSpecEnvelope, InteractionType, binding_child_ensure,
    key_ref, owned_child_ensure, resource_uid, spec_decoder,
};
pub use vocabulary::{
    AUDIO_BINDING_TYPE, AUDIO_SERVICE_TYPE, shell_pool_spec, shell_session_execution,
    shell_session_pool_ref,
};
pub use wayland_policy::{
    WAYLAND_POLICY_PROVIDER_REF, WAYLAND_POLICY_RESYNC,
    WAYLAND_POLICY_TYPE, WaylandPolicy, WaylandPolicyDriver, WaylandPolicyFactory,
    wayland_policy_descriptor, wayland_policy_spec_decoder,
};

/// The execution domains every interaction type is driven in.
///
/// A display session's workers, an audio binding's guest target, and a shell
/// session's execution reference all cross the Host/Guest boundary, so the
/// family declares both domains.
pub const INTERACTION_EXECUTION_DOMAINS: &[&str] = &["host", "guest"];
