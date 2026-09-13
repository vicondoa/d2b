//! The WaylandPolicy resource type and the interaction family's shared engine.
//!
//! This crate is the display wayland policy's home: the `WaylandPolicy`
//! driver, its spec decoder, and the driver declaration the resource plane
//! registers the type by. It is also the family's root crate, so the six
//! interaction types' shared engine lives here beside it: the reconcile,
//! recover, finalize, and delete verbs, the spec-envelope decode, the
//! manager-child plumbing, and the effect port the daemon implements
//! ([`interaction`]).
//!
//! The other five types - `WaylandSession`, `AudioService`, `AudioBinding`,
//! `ShellPool`, and `ShellSession` - each live in their own crate and build
//! their behavior on the same engine. Every one of them owns its row, its
//! decoder, its factory, and its descriptor; none of them is declared by the
//! daemon.

#![deny(missing_docs)]

pub mod interaction;
mod wayland_policy;

pub use interaction::{
    InteractionChild, InteractionChildContext, InteractionDriver, InteractionDriverArgs,
    InteractionDriverEffects, InteractionDriverError, InteractionDriverFactory,
    InteractionDriverStatus, InteractionEffectError, InteractionEffectOutcome,
    InteractionEffectPhase, InteractionEffectRequest, InteractionFinalize, InteractionKind,
    InteractionSpecDecodeError, InteractionSpecEnvelope, InteractionType, binding_child_ensure,
    key_ref, owned_child_ensure, resource_uid, spec_decoder,
};
pub use wayland_policy::{
    WAYLAND_POLICY_PROVIDER_REF, WAYLAND_POLICY_RESYNC,
    WAYLAND_POLICY_TYPE, WaylandPolicy, WaylandPolicyDriver, WaylandPolicyFactory,
    wayland_policy_descriptor, wayland_policy_spec_decoder,
};

/// The resource verbs every interaction type supports.
///
/// The family's rows are ordinary managed resources: they are read, watched,
/// authored, and deleted, and their status and metadata are updated by the
/// plane. No type of the family is created by a broker operation.
pub const INTERACTION_VERBS: &[&str] = &[
    "get",
    "list",
    "watch",
    "create",
    "update-spec",
    "update-status",
    "update-metadata",
    "update-finalizers",
    "delete",
];

/// The execution domains every interaction type is driven in.
///
/// A display session's workers, an audio binding's guest target, and a shell
/// session's execution reference all cross the Host/Guest boundary, so the
/// family declares both domains.
pub const INTERACTION_EXECUTION_DOMAINS: &[&str] = &["host", "guest"];
