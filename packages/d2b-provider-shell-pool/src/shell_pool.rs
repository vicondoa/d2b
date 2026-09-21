//! The `ShellPool` resource type: the durable pool persistent shells belong
//! to.
//!
//! A pool names the execution target its sessions run on, the host user they
//! run as, and the login shell artifact they start; sessions reference it. The
//! pool realizes nothing through resource rows of its own, so its desired
//! child set is empty and its teardown refuses while a session still
//! references it - that refusal is the Provider's, served through the family
//! effect port.
//!
//! The spec is authored as the Provider's reference document rather than a
//! typed struct, so the reference shapes are checked through the family
//! engine's shared vocabulary, in the same order and with the same refusals
//! the family has always used.

use std::sync::Arc;
use std::time::Duration;

use d2b_contracts_resource::v3::ResourceRef;
use d2b_provider_shell_terminal::SHELL_REPAIR_INTERVAL_SECS;
use d2b_resource_runtime::context::{ChildEnsure, SpecDecoder};
use d2b_resource_types::{AllowedSources, DriverDescriptor, WellKnownType};

use d2b_provider_wayland_policy::{
    shell_pool_spec,
    interaction::{
        InteractionChildContext, InteractionDriver, InteractionDriverArgs,
        InteractionDriverFactory, InteractionEffectError, InteractionKind,
        InteractionSpecEnvelope, InteractionType, spec_decoder,
    },
};

/// The canonical ResourceType name of a shell pool.
pub const SHELL_POOL_TYPE: &str = "shell-terminal.d2bus.org.ShellPool";

/// The Provider reference the type's rows select.
pub const SHELL_POOL_PROVIDER_REF: &str = d2b_provider_shell_terminal::PROVIDER_REF;

/// The preserved reconcile resync cadence of the type.
pub const SHELL_POOL_RESYNC: Duration = Duration::from_secs(SHELL_REPAIR_INTERVAL_SECS);

/// The `ShellPool` driver behavior and declaration.
#[derive(Debug, Clone, Copy, Default)]
pub struct ShellPool;

impl InteractionType for ShellPool {
    const KIND: InteractionKind = InteractionKind::ShellPool;
    const RESOURCE_TYPE: &'static str = SHELL_POOL_TYPE;
    const PROVIDER_REF: &'static str = SHELL_POOL_PROVIDER_REF;
    /// The shell Provider's specs carry the universal `providerRef`
    /// themselves, so the row must select this Provider.
    const SPEC_PROVIDER_SELECTOR: bool = true;

    fn resync(&self) -> Duration {
        SHELL_POOL_RESYNC
    }

    /// The pool's execution, user, and login-shell reference shapes.
    fn validate(
        &self,
        envelope: &InteractionSpecEnvelope,
    ) -> Result<(), InteractionEffectError> {
        shell_pool_spec(envelope.base(), envelope.provider_ref()).map(|_| ())
    }

    /// The execution target and user the pool's sessions run as.
    fn dependencies(
        &self,
        envelope: &InteractionSpecEnvelope,
    ) -> Result<Vec<ResourceRef>, InteractionEffectError> {
        let (execution_ref, user_ref) =
            shell_pool_spec(envelope.base(), envelope.provider_ref())?;
        Ok(vec![execution_ref, user_ref])
    }

    /// A pool realizes nothing through resource rows.
    fn desired_children(
        &self,
        _children: &InteractionChildContext<'_>,
        _envelope: &InteractionSpecEnvelope,
    ) -> Result<Vec<ChildEnsure>, InteractionEffectError> {
        Ok(Vec::new())
    }
}

/// The driver for one `ShellPool` row.
pub type ShellPoolDriver = InteractionDriver<ShellPool>;

/// The factory the registry serves for `ShellPool`.
pub type ShellPoolFactory = InteractionDriverFactory<ShellPool>;

/// The manager-wired decode hook for `ShellPool` rows.
pub fn shell_pool_spec_decoder() -> Arc<dyn SpecDecoder> {
    spec_decoder()
}

/// The `ShellPool` driver declaration.
///
/// The registry keys the type by this declaration, so the type reaches the
/// plane only through it.
pub fn shell_pool_descriptor(args: InteractionDriverArgs<ShellPool>) -> DriverDescriptor {
    DriverDescriptor {
        resource_type: WellKnownType::SHELL_POOL,
        allowed_sources: AllowedSources::BUILTIN | AllowedSources::STARTUP | AllowedSources::RUNTIME,
        verbs: d2b_provider_wayland_policy::INTERACTION_VERBS,
        execution: d2b_provider_wayland_policy::INTERACTION_EXECUTION_DOMAINS,
        exportable: false,
        reads: &[WellKnownType::HOST, WellKnownType::GUEST, WellKnownType::USER],
        operations: &[],
        creations: &[],
        startup: &[],
        services: &[],
        decoder: shell_pool_spec_decoder(),
        factory: Arc::new(ShellPoolFactory::new(args)),
    }
}
