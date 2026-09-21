//! The ShellSession resource type's driver crate.
//!
//! The crate owns the `shell-terminal.d2bus.org.ShellSession` type completely:
//! the driver, its spec decoder, its supervisor-child derivation, and the
//! driver declaration the resource plane registers the type by. The driver
//! verbs and the shared spec reference checks come from the interaction
//! family's shared engine (`d2b-provider-wayland-policy`), which also owns
//! the family's production effects.

#![deny(missing_docs)]

mod shell_session;

pub use shell_session::{
    SHELL_SESSION_PROVIDER_REF, SHELL_SESSION_RESYNC,
    SHELL_SESSION_TYPE, ShellSession, ShellSessionDriver, ShellSessionFactory,
    shell_session_descriptor, shell_session_spec_decoder,
};
