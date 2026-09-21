//! The ShellPool resource type's driver crate.
//!
//! The crate owns the `shell-terminal.d2bus.org.ShellPool` type completely:
//! the driver, its spec decoder, its spec reference checks, and the driver
//! declaration the resource plane registers the type by. The driver verbs come
//! from the interaction family's shared engine
//! (`d2b-provider-wayland-policy`); the daemon keeps the production shell
//! effects behind the family's effect port.

#![deny(missing_docs)]

mod shell_pool;

pub use shell_pool::{
    SHELL_POOL_PROVIDER_REF, SHELL_POOL_RESYNC, SHELL_POOL_TYPE,
    ShellPool, ShellPoolDriver, ShellPoolFactory, shell_pool_descriptor,
    shell_pool_spec_decoder,
};
