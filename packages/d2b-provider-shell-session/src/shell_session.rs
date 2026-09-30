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
use d2b_contracts_resource::v3::process::ProcessSpec;
use d2b_provider_shell_terminal::{
    SHELL_REPAIR_INTERVAL_SECS, SUPERVISOR_PROCESS_PROVIDER_REF, supervisor_execution_spec,
};
use d2b_resource_runtime::context::{ChildEnsure, SpecDecoder};
use d2b_resource_runtime::identity::ResourceTypeName;
use d2b_resource_types::{AllowedSources, CONVERTED_TYPE_VERBS, DriverDescriptor, WellKnownType};
use serde_json::{Value, json};

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

/// The optional terminal stream `Endpoint` this session's stream rides.
///
/// The relationship is optional in the spec only so an older row still
/// decodes; when it is declared it must name an `Endpoint`, because the
/// stream's admission is scoped to that exact endpoint rather than to the
/// directory that happens to contain it.
fn session_terminal_endpoint(
    envelope: &InteractionSpecEnvelope,
) -> Result<Option<ResourceRef>, InteractionEffectError> {
    envelope
        .base()
        .get("endpointRef")
        .and_then(Value::as_str)
        .map(|reference| {
            ResourceRef::parse(reference)
                .ok()
                .filter(|endpoint| endpoint.resource_type().as_str() == "Endpoint")
                .ok_or(InteractionEffectError::InvalidResource)
        })
        .transpose()
}

/// The supervisor Process child spec, derived from the Provider's declared
/// launch contract rather than composed by this driver.
///
/// The execution target, workload `User`, executable template, and Process
/// provider all come from the shell Provider's own contract, so the session
/// driver has no argv, executable, or Process provider of its own to
/// configure.
fn supervisor_child_spec(
    execution_ref: &ResourceRef,
    user_ref: &ResourceRef,
    pool_ref: &ResourceRef,
) -> Result<Vec<u8>, InteractionEffectError> {
    let invalid = || InteractionEffectError::InvalidResource;
    let execution = supervisor_execution_spec(execution_ref.clone(), user_ref.clone())
        .map_err(|_| invalid())?;
    let spec = ProcessSpec::minimal(execution);
    let mut value = serde_json::to_value(&spec).map_err(|_| invalid())?;
    let object = value.as_object_mut().ok_or_else(invalid)?;
    object.insert(
        "providerRef".to_owned(),
        Value::String(SUPERVISOR_PROCESS_PROVIDER_REF.to_owned()),
    );
    object.insert(
        "dependencies".to_owned(),
        json!([pool_ref.to_canonical_string()]),
    );
    serde_json::to_vec(&value).map_err(|_| invalid())
}

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

    /// The session's execution, user, login-shell, pool, and terminal
    /// endpoint reference shapes.
    fn validate(
        &self,
        envelope: &InteractionSpecEnvelope,
    ) -> Result<(), InteractionEffectError> {
        shell_session_execution(envelope.base(), envelope.provider_ref())?;
        shell_session_pool_ref(envelope.base(), envelope.provider_ref())?;
        session_terminal_endpoint(envelope).map(|_| ())
    }

    /// The graph relationships the session requests: its pool, its Process
    /// execution target, its `User`, and the terminal stream endpoint its
    /// interactive stream is admitted on.
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
        if let Some(endpoint) = session_terminal_endpoint(envelope)? {
            dependencies.push(endpoint);
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
        let process_spec = supervisor_child_spec(&execution_ref, &user_ref, &pool_ref)?;
        Ok(vec![ChildEnsure {
            type_name: ResourceTypeName::new(process_ref.resource_type().as_str()),
            name: process_ref.name().as_str().to_owned(),
            spec: process_spec,
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
