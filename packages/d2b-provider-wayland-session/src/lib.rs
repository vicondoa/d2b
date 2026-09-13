//! The WaylandSession resource type's driver crate.
//!
//! The crate owns the `display-wayland.d2bus.org.WaylandSession` type
//! completely: the driver, its spec decoder, its child-intent port, and the
//! driver declaration the resource plane registers the type by. The session's
//! driver verbs come from the interaction family's shared engine
//! (`d2b-provider-wayland-policy`), and the daemon keeps the production effect
//! implementation and the display supervisor's child-intent source behind the
//! ports this crate declares.

#![deny(missing_docs)]

mod wayland_session;

pub use wayland_session::{
    DisplayChildRequest, DisplayChildSource, WAYLAND_SESSION_CONTROLLER_REF,
    WAYLAND_SESSION_PROVIDER_REF, WAYLAND_SESSION_RESYNC, WAYLAND_SESSION_TYPE, WaylandSession,
    WaylandSessionDriver, WaylandSessionFactory, wayland_session_descriptor,
    wayland_session_spec_decoder,
};
