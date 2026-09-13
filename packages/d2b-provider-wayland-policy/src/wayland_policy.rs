//! The `WaylandPolicy` resource type: the display policy a Zone commits.
//!
//! The row is the family's root: it owns no children and reads nothing, and
//! its envelope is the whole contract. It exists so that every
//! `WaylandSession` in the Zone can reference the policy it was compiled
//! under, and so that the display Provider's committed rows are readable
//! before any session is admitted.

use std::sync::Arc;
use std::time::Duration;

use d2b_provider_display_wayland::DISPLAY_REPAIR_INTERVAL_SECS;
use d2b_resource_runtime::context::{ChildEnsure, SpecDecoder};
use d2b_resource_types::{AllowedSources, DriverDescriptor, WellKnownType};
use serde_json::Value;

use crate::interaction::{
    InteractionChildContext, InteractionDriver, InteractionDriverArgs, InteractionDriverFactory,
    InteractionEffectError, InteractionKind, InteractionSpecEnvelope, InteractionType,
    spec_decoder,
};

/// The canonical ResourceType name of the display policy.
pub const WAYLAND_POLICY_TYPE: &str = "display-wayland.d2bus.org.WaylandPolicy";

/// The Provider reference the type's rows select.
pub const WAYLAND_POLICY_PROVIDER_REF: &str = d2b_provider_display_wayland::PROVIDER_REF;

/// The preserved reconcile resync cadence of the type.
pub const WAYLAND_POLICY_RESYNC: Duration =
    Duration::from_secs(DISPLAY_REPAIR_INTERVAL_SECS);

/// The `WaylandPolicy` driver behavior and declaration.
#[derive(Debug, Clone, Copy, Default)]
pub struct WaylandPolicy;

impl InteractionType for WaylandPolicy {
    const KIND: InteractionKind = InteractionKind::DisplayWaylandPolicy;
    const RESOURCE_TYPE: &'static str = WAYLAND_POLICY_TYPE;
    const PROVIDER_REF: &'static str = WAYLAND_POLICY_PROVIDER_REF;
    /// Display rows are envelope-only: the old descriptor kept no exact
    /// Provider selector for them, so the universal `spec.providerRef` stays
    /// optional.
    const SPEC_PROVIDER_SELECTOR: bool = false;

    fn resync(&self) -> Duration {
        WAYLAND_POLICY_RESYNC
    }

    /// The policy envelope is the whole contract: the spec must decode as a
    /// JSON object and nothing else is checked.
    fn validate(
        &self,
        envelope: &InteractionSpecEnvelope,
    ) -> Result<(), InteractionEffectError> {
        envelope.base_spec::<Value>().map(|_| ())
    }

    /// A policy reads nothing while reconciling.
    fn dependencies(
        &self,
        _envelope: &InteractionSpecEnvelope,
    ) -> Result<Vec<d2b_contracts_resource::v3::ResourceRef>, InteractionEffectError> {
        Ok(Vec::new())
    }

    /// A policy realizes nothing through resource rows.
    fn desired_children(
        &self,
        _children: &InteractionChildContext<'_>,
        _envelope: &InteractionSpecEnvelope,
    ) -> Result<Vec<ChildEnsure>, InteractionEffectError> {
        Ok(Vec::new())
    }
}

/// The driver for one `WaylandPolicy` row.
pub type WaylandPolicyDriver = InteractionDriver<WaylandPolicy>;

/// The factory the registry serves for `WaylandPolicy`.
pub type WaylandPolicyFactory = InteractionDriverFactory<WaylandPolicy>;

/// The manager-wired decode hook for `WaylandPolicy` rows.
pub fn wayland_policy_spec_decoder() -> Arc<dyn SpecDecoder> {
    spec_decoder()
}

/// The `WaylandPolicy` driver declaration.
///
/// The registry keys the type by this declaration, so the type reaches the
/// plane only through it.
pub fn wayland_policy_descriptor(
    args: InteractionDriverArgs<WaylandPolicy>,
) -> DriverDescriptor {
    DriverDescriptor {
        resource_type: WellKnownType::WAYLAND_POLICY,
        allowed_sources: AllowedSources::BUILTIN | AllowedSources::STARTUP | AllowedSources::RUNTIME,
        verbs: crate::INTERACTION_VERBS,
        execution: crate::INTERACTION_EXECUTION_DOMAINS,
        exportable: false,
        reads: &[],
        operations: &[],
        creations: &[],
        startup: &[],
        services: &[],
        decoder: wayland_policy_spec_decoder(),
        factory: Arc::new(WaylandPolicyFactory::new(args)),
    }
}
