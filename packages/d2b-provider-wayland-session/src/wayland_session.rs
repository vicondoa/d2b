//! The `WaylandSession` resource type: one admitted display session.
//!
//! A session is the display Provider's unit of realization. It reads the
//! Guest, Host, User, and policy rows its spec references, owns the host proxy
//! and guest frontend worker children (each with its private endpoint), and
//! drives the display Provider's admission through the family effect port.
//!
//! The session's children are the display supervisor's realization, so the
//! crate asks for them through [`DisplayChildSource`]: the daemon authors the
//! child intents with the launch material only it holds, and this crate turns
//! them into manager child rows.

use std::sync::Arc;
use std::time::Duration;

use d2b_contracts_resource::v3::{ResourceRef, ResourceUid, ZoneId};
use d2b_core_controller::OwnedChildIntent;
use d2b_provider_display_wayland::{DISPLAY_REPAIR_INTERVAL_SECS, WaylandSessionSpec};
use d2b_resource_runtime::context::{ChildEnsure, SpecDecoder};
use d2b_resource_types::{AllowedSources, DriverDescriptor, WellKnownType};

use d2b_provider_wayland_policy::interaction::{
    InteractionChildContext, InteractionDriver, InteractionDriverArgs, InteractionDriverFactory,
    InteractionEffectError, InteractionKind, InteractionSpecEnvelope, InteractionType, key_ref,
    owned_child_ensure, resource_uid, spec_decoder,
};

/// The canonical ResourceType name of a display session.
pub const WAYLAND_SESSION_TYPE: &str = "display-wayland.d2bus.org.WaylandSession";

/// The Provider reference the type's rows select.
pub const WAYLAND_SESSION_PROVIDER_REF: &str = d2b_provider_display_wayland::PROVIDER_REF;

/// The preserved reconcile resync cadence of the type.
pub const WAYLAND_SESSION_RESYNC: Duration =
    Duration::from_secs(DISPLAY_REPAIR_INTERVAL_SECS);

/// Everything the display supervisor needs to author one session's children.
pub struct DisplayChildRequest<'a> {
    /// The Zone the session belongs to.
    pub zone: &'a ZoneId,
    /// The session's resource reference.
    pub session_ref: &'a ResourceRef,
    /// The session's durable uid (the child names derive from it).
    pub session_uid: &'a ResourceUid,
    /// The session's decoded spec.
    pub spec: &'a WaylandSessionSpec,
    /// The session row's generation (the preserved policy generation).
    pub process_generation: u64,
    /// The controller generation the children bind.
    pub controller_generation: u64,
}

/// The daemon-owned source of one session's child intents.
///
/// The display supervisor owns the worker launch material, so the intents are
/// authored on the daemon side of the port and this crate owns the manager
/// child shape they materialize into.
pub trait DisplayChildSource: Send + Sync + 'static {
    /// The Process and Endpoint intents one session owns, in the family's
    /// preserved order (host proxy, guest frontend; each with its endpoint).
    fn display_children(
        &self,
        request: &DisplayChildRequest<'_>,
    ) -> Result<Vec<OwnedChildIntent>, InteractionEffectError>;
}

/// The `WaylandSession` driver behavior and declaration.
#[derive(Clone)]
pub struct WaylandSession {
    children: Arc<dyn DisplayChildSource>,
}

impl WaylandSession {
    /// Build the behavior over the daemon's child-intent source.
    pub fn new(children: Arc<dyn DisplayChildSource>) -> Self {
        Self { children }
    }
}

impl InteractionType for WaylandSession {
    const KIND: InteractionKind = InteractionKind::DisplayWaylandSession;
    const RESOURCE_TYPE: &'static str = WAYLAND_SESSION_TYPE;
    const PROVIDER_REF: &'static str = WAYLAND_SESSION_PROVIDER_REF;
    /// Display rows are envelope-only: the old descriptor kept no exact
    /// Provider selector for them.
    const SPEC_PROVIDER_SELECTOR: bool = false;

    fn resync(&self) -> Duration {
        WAYLAND_SESSION_RESYNC
    }

    /// The session spec is the whole contract beyond the envelope decode.
    fn validate(
        &self,
        envelope: &InteractionSpecEnvelope,
    ) -> Result<(), InteractionEffectError> {
        envelope.base_spec::<WaylandSessionSpec>().map(|_| ())
    }

    /// The session's cross-domain trust: the Guest, Host, User, and policy
    /// rows its spec names.
    fn dependencies(
        &self,
        envelope: &InteractionSpecEnvelope,
    ) -> Result<Vec<ResourceRef>, InteractionEffectError> {
        let spec = envelope.base_spec::<WaylandSessionSpec>()?;
        Ok(vec![
            spec.guest_ref().clone(),
            spec.host_ref().clone(),
            spec.user_ref().clone(),
            spec.policy_ref().clone(),
        ])
    }

    /// The two workers and their private endpoints, as manager child rows.
    fn desired_children(
        &self,
        children: &InteractionChildContext<'_>,
        envelope: &InteractionSpecEnvelope,
    ) -> Result<Vec<ChildEnsure>, InteractionEffectError> {
        let spec = envelope.base_spec::<WaylandSessionSpec>()?;
        let session_ref = key_ref(children.key);
        let session_uid = resource_uid(children.uid)
            .ok_or(InteractionEffectError::InvalidResource)?;
        let intents = self.children.display_children(&DisplayChildRequest {
            zone: children.zone,
            session_ref: &session_ref,
            session_uid: &session_uid,
            spec: &spec,
            process_generation: children.generation,
            controller_generation: children.controller_generation,
        })?;
        intents.iter().map(owned_child_ensure).collect()
    }
}

/// The driver for one `WaylandSession` row.
pub type WaylandSessionDriver = InteractionDriver<WaylandSession>;

/// The factory the registry serves for `WaylandSession`.
pub type WaylandSessionFactory = InteractionDriverFactory<WaylandSession>;

/// The manager-wired decode hook for `WaylandSession` rows.
pub fn wayland_session_spec_decoder() -> Arc<dyn SpecDecoder> {
    spec_decoder()
}

/// The `WaylandSession` driver declaration.
///
/// The registry keys the type by this declaration, so the type reaches the
/// plane only through it.
pub fn wayland_session_descriptor(
    args: InteractionDriverArgs<WaylandSession>,
) -> DriverDescriptor {
    DriverDescriptor {
        resource_type: WellKnownType::WAYLAND_SESSION,
        allowed_sources: AllowedSources::BUILTIN | AllowedSources::STARTUP | AllowedSources::RUNTIME,
        verbs: d2b_provider_wayland_policy::INTERACTION_VERBS,
        execution: d2b_provider_wayland_policy::INTERACTION_EXECUTION_DOMAINS,
        exportable: false,
        reads: &[
            WellKnownType::GUEST,
            WellKnownType::HOST,
            WellKnownType::USER,
            WellKnownType::WAYLAND_POLICY,
        ],
        operations: &[],
        creations: &[],
        startup: &[],
        services: &[],
        decoder: wayland_session_spec_decoder(),
        factory: Arc::new(WaylandSessionFactory::new(args)),
    }
}
