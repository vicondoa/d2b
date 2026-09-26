//! The `ShellSession` resource type: one persistent shell attached to a pool.
//!
//! A session names the pool it belongs to, its execution target, the host user
//! it runs as, and the login shell artifact it starts; it owns one supervisor
//! Process child, which the Process driver launches. The session drives the
//! shell Provider's attachment and output-ring lifecycle through the family
//! effect port, and its teardown drains the supervisor child before the
//! Provider stage runs.

use std::sync::Arc;
use std::time::Duration;

use d2b_contracts_resource::v3::ResourceRef;
use d2b_provider_shell_terminal::SHELL_REPAIR_INTERVAL_SECS;
use d2b_resource_runtime::context::{ChildEnsure, SpecDecoder};
use d2b_resource_runtime::identity::ResourceTypeName;
use d2b_resource_types::{AllowedSources, CONVERTED_TYPE_VERBS, DriverDescriptor, WellKnownType};
use serde_json::json;

use d2b_provider_wayland_policy::{
    shell_session_execution, shell_session_pool_ref,
    interaction::{
        InteractionChildContext, InteractionDriver, InteractionDriverArgs,
        InteractionDriverFactory, InteractionEffectError, InteractionKind,
        InteractionSpecEnvelope, InteractionType, key_ref, spec_decoder,
    },
};

/// The canonical ResourceType name of a shell session.
pub const SHELL_SESSION_TYPE: &str = "shell-terminal.d2bus.org.ShellSession";

/// The Provider reference the type's rows select.
pub const SHELL_SESSION_PROVIDER_REF: &str = d2b_provider_shell_terminal::PROVIDER_REF;

/// The preserved reconcile resync cadence of the type.
pub const SHELL_SESSION_RESYNC: Duration = Duration::from_secs(SHELL_REPAIR_INTERVAL_SECS);

/// The Process Provider that launches the session's supervisor child.
const SHELL_SUPERVISOR_PROVIDER_REF: &str = "Provider/system-systemd";

/// The supervisor Process template the session's child is launched from.
const SHELL_SUPERVISOR_TEMPLATE: &str = "shell-supervisor-main";

/// The `ShellSession` driver behavior and declaration.
#[derive(Debug, Clone, Copy, Default)]
pub struct ShellSession;

impl InteractionType for ShellSession {
    const KIND: InteractionKind = InteractionKind::ShellSession;
    const RESOURCE_TYPE: &'static str = SHELL_SESSION_TYPE;
    const PROVIDER_REF: &'static str = SHELL_SESSION_PROVIDER_REF;
    /// The shell Provider's specs carry the universal `providerRef`
    /// themselves, so the row must select this Provider.
    const SPEC_PROVIDER_SELECTOR: bool = true;

    fn resync(&self) -> Duration {
        SHELL_SESSION_RESYNC
    }

    /// The session's execution, user, login-shell, and pool reference shapes.
    fn validate(
        &self,
        envelope: &InteractionSpecEnvelope,
    ) -> Result<(), InteractionEffectError> {
        shell_session_execution(envelope.base(), envelope.provider_ref())?;
        shell_session_pool_ref(envelope.base(), envelope.provider_ref()).map(|_| ())
    }

    /// The pool, execution target, and user the session runs against.
    fn dependencies(
        &self,
        envelope: &InteractionSpecEnvelope,
    ) -> Result<Vec<ResourceRef>, InteractionEffectError> {
        let (execution_ref, user_ref) =
            shell_session_execution(envelope.base(), envelope.provider_ref())?;
        let mut dependencies =
            vec![shell_session_pool_ref(envelope.base(), envelope.provider_ref())?];
        dependencies.push(execution_ref);
        if let Some(user_ref) = user_ref {
            dependencies.push(user_ref);
        }
        Ok(dependencies)
    }

    /// The session's supervisor Process child, as a manager child row.
    ///
    /// The child carries only signed template metadata; the Process driver
    /// owns the launch.
    fn desired_children(
        &self,
        children: &InteractionChildContext<'_>,
        envelope: &InteractionSpecEnvelope,
    ) -> Result<Vec<ChildEnsure>, InteractionEffectError> {
        let invalid = || InteractionEffectError::InvalidResource;
        let (execution_ref, user_ref) =
            shell_session_execution(envelope.base(), envelope.provider_ref())?;
        let user_ref = user_ref.ok_or_else(invalid)?;
        let pool_ref = shell_session_pool_ref(envelope.base(), envelope.provider_ref())?;
        let process_ref = ResourceRef::parse(&format!(
            "Process/shell-session-{}",
            children.key.name
        ))
        .map_err(|_| invalid())?;
        let process_spec = json!({
            "providerRef": SHELL_SUPERVISOR_PROVIDER_REF,
            "executionRef": execution_ref.to_canonical_string(),
            "domain": "user",
            "userRef": user_ref.to_canonical_string(),
            "processClass": "service",
            "template": SHELL_SUPERVISOR_TEMPLATE,
            "desiredLifecycle": "running",
            "deviceUsage": [],
            "networkUsage": null,
            "dependencies": [pool_ref.to_canonical_string()],
        });
        Ok(vec![ChildEnsure {
            type_name: ResourceTypeName::new(process_ref.resource_type().as_str()),
            name: process_ref.name().as_str().to_owned(),
            spec: serde_json::to_vec(&process_spec).map_err(|_| invalid())?,
            metadata: serde_json::to_vec(&json!({
                "ownerRef": key_ref(children.key)
                    .map_err(|_| invalid())?
                    .to_canonical_string(),
                "labels": {},
                "annotations": {},
            }))
            .map_err(|_| invalid())?,
        }])
    }
}

/// The driver for one `ShellSession` row.
pub type ShellSessionDriver = InteractionDriver<ShellSession>;

/// The factory the registry serves for `ShellSession`.
pub type ShellSessionFactory = InteractionDriverFactory<ShellSession>;

/// The manager-wired decode hook for `ShellSession` rows.
pub fn shell_session_spec_decoder() -> Arc<dyn SpecDecoder> {
    spec_decoder()
}

/// The `ShellSession` driver declaration.
///
/// The registry keys the type by this declaration, so the type reaches the
/// plane only through it.
pub fn shell_session_descriptor(args: InteractionDriverArgs<ShellSession>) -> DriverDescriptor {
    DriverDescriptor {
        resource_type: WellKnownType::SHELL_SESSION,
        allowed_sources: AllowedSources::BUILTIN | AllowedSources::STARTUP | AllowedSources::RUNTIME,
        verbs: CONVERTED_TYPE_VERBS,
        execution: d2b_provider_wayland_policy::INTERACTION_EXECUTION_DOMAINS,
        exportable: false,
        reads: &[
            WellKnownType::SHELL_POOL,
            WellKnownType::HOST,
            WellKnownType::GUEST,
            WellKnownType::USER,
        ],
        operations: &[],
        creations: &[],
        startup: &[],
        services: &[],
        decoder: shell_session_spec_decoder(),
        factory: Arc::new(ShellSessionFactory::new(args)),
    }
}
