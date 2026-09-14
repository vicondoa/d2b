//! ShellSession registration and behavior boundary: the declaration the plane
//! registers, the reference shapes the driver validates, and the supervisor
//! Process child the session derives.

use std::sync::Arc;

use d2b_contracts_resource::v3::{ControllerGeneration, ResourceRef, ZoneId};
use d2b_provider_shell_session::{
    SHELL_SESSION_TYPE, ShellSession, shell_session_descriptor, shell_session_execution,
    shell_session_pool_ref, shell_session_spec_decoder,
};
use d2b_provider_wayland_policy::{
    InteractionChildContext, InteractionDriverArgs, InteractionDriverEffects,
    InteractionEffectError, InteractionEffectOutcome, InteractionEffectPhase,
    InteractionEffectRequest, InteractionFinalize, InteractionKind, InteractionSpecEnvelope,
    InteractionType,
};
use d2b_resource_runtime::identity::{ResourceKey, ResourceTypeName};
use d2b_resource_runtime::provider::{ProviderDirectory, ProviderDirectoryError};
use d2b_resource_types::{AllowedSources, WellKnownType};
use serde_json::{Value, json};

/// The port instance the declaration carries; the boundary never runs an
/// effect.
struct UnusedEffects;

#[async_trait::async_trait]
impl InteractionDriverEffects for UnusedEffects {
    async fn reconcile(
        &self,
        _kind: InteractionKind,
        _request: &InteractionEffectRequest<'_>,
    ) -> Result<InteractionEffectOutcome, InteractionEffectError> {
        Ok(InteractionEffectOutcome::phase(InteractionEffectPhase::Pending))
    }

    async fn finalize(
        &self,
        _kind: InteractionKind,
        _request: &InteractionEffectRequest<'_>,
    ) -> Result<InteractionFinalize, InteractionEffectError> {
        Ok(InteractionFinalize::Complete)
    }
}

fn descriptor() -> d2b_resource_types::DriverDescriptor {
    shell_session_descriptor(InteractionDriverArgs {
        zone: "work".to_owned(),
        controller_generation: ControllerGeneration::new(3).expect("generation"),
        effects: Arc::new(UnusedEffects),
        behavior: ShellSession,
    })
}

fn session_spec() -> Value {
    json!({
        "providerRef": "Provider/shell-terminal",
        "executionRef": "Host/host-system",
        "userRef": "User/alice",
        "loginShellRef": "artifact://shell",
        "poolRef": "shell-terminal.d2bus.org.ShellPool/work-shell",
    })
}

fn envelope(value: &Value) -> InteractionSpecEnvelope {
    let bytes = serde_json::to_vec(value).expect("spec bytes");
    let decoded = shell_session_spec_decoder()
        .decode(&bytes)
        .expect("the row decodes");
    *decoded
        .downcast::<InteractionSpecEnvelope>()
        .expect("the decoder yields the family envelope")
}

fn child_context<'a>(
    zone: &'a ZoneId,
    key: &'a ResourceKey,
    uid: &'a [u8; 16],
) -> InteractionChildContext<'a> {
    InteractionChildContext {
        zone,
        key,
        uid,
        generation: 4,
        controller_generation: 3,
    }
}

/// The declaration carries the session row: the type, its runtime-admitted
/// source mask, and the rows a session reads.
#[test]
fn the_declaration_serves_the_session_row() {
    let descriptor = descriptor();
    assert_eq!(descriptor.resource_type, WellKnownType::SHELL_SESSION);
    assert_eq!(
        descriptor.allowed_sources,
        AllowedSources::BUILTIN | AllowedSources::STARTUP | AllowedSources::RUNTIME
    );
    assert!(!descriptor.exportable);
    assert_eq!(
        descriptor.reads,
        &[
            WellKnownType::SHELL_POOL,
            WellKnownType::HOST,
            WellKnownType::GUEST,
            WellKnownType::USER,
        ]
    );
    assert!(descriptor.operations.is_empty() && descriptor.creations.is_empty());
    const { assert!(ShellSession::SPEC_PROVIDER_SELECTOR); };
    assert_eq!(ShellSession::RESOURCE_TYPE, SHELL_SESSION_TYPE);
}

/// The registry serves the session's decoder and factory from the
/// declaration, and a second registration of the type is refused.
#[test]
fn the_registry_serves_the_declaration_and_refuses_a_duplicate() {
    let mut providers = ProviderDirectory::new();
    providers
        .register_driver(&descriptor())
        .expect("the declaration registers");
    assert!(
        providers
            .decoders()
            .contains_key(&ResourceTypeName::new(SHELL_SESSION_TYPE))
    );
    assert!(matches!(
        providers.register_driver(&descriptor()),
        Err(ProviderDirectoryError::DuplicateType(resource_type))
            if resource_type.as_str() == SHELL_SESSION_TYPE
    ));
}

/// The session validates and reads its pool, execution target, and user.
#[test]
fn the_session_reads_its_pool_execution_and_user() {
    let envelope = envelope(&session_spec());
    ShellSession.validate(&envelope).expect("the session validates");
    assert_eq!(
        ShellSession.dependencies(&envelope).expect("dependencies"),
        vec![
            ResourceRef::parse("shell-terminal.d2bus.org.ShellPool/work-shell").expect("pool"),
            ResourceRef::parse("Host/host-system").expect("execution"),
            ResourceRef::parse("User/alice").expect("user"),
        ]
    );
}

/// The session's supervisor Process child: named after the row, launched by
/// the Process Provider from the preserved template, with the pool in its
/// dependencies.
#[test]
fn the_session_derives_its_supervisor_child() {
    let envelope = envelope(&session_spec());
    let zone = ZoneId::parse("work").expect("zone");
    let key = ResourceKey::new("work", SHELL_SESSION_TYPE, "session-1");
    let uid = [0x42u8; 16];
    let children = ShellSession
        .desired_children(&child_context(&zone, &key, &uid), &envelope)
        .expect("children");

    assert_eq!(children.len(), 1);
    assert_eq!(children[0].type_name.as_str(), "Process");
    assert_eq!(children[0].name, "shell-session-session-1");
    let spec: Value = serde_json::from_slice(&children[0].spec).expect("child spec");
    assert_eq!(spec["providerRef"], "Provider/system-systemd");
    assert_eq!(spec["template"], "shell-supervisor-main");
    assert_eq!(spec["processClass"], "service");
    assert_eq!(spec["executionRef"], "Host/host-system");
    assert_eq!(spec["userRef"], "User/alice");
    assert_eq!(
        spec["dependencies"],
        json!(["shell-terminal.d2bus.org.ShellPool/work-shell"])
    );
    let metadata: Value = serde_json::from_slice(&children[0].metadata).expect("child metadata");
    assert_eq!(
        metadata["ownerRef"],
        "shell-terminal.d2bus.org.ShellSession/session-1"
    );
}

/// The session's reference shapes are refused exactly as they were: a pool
/// reference that is not a ShellPool, and a row with no user that cannot
/// derive its supervisor child.
#[test]
fn malformed_references_are_refused() {
    let mut bad_pool = session_spec();
    bad_pool["poolRef"] = json!("shell-terminal.d2bus.org.ShellSession/other");
    let bad_pool_envelope = envelope(&bad_pool);
    assert_eq!(
        ShellSession.validate(&bad_pool_envelope),
        Err(InteractionEffectError::InvalidResource)
    );
    assert_eq!(
        shell_session_pool_ref(bad_pool_envelope.base(), bad_pool_envelope.provider_ref()),
        Err(InteractionEffectError::InvalidResource)
    );

    let mut no_user = session_spec();
    no_user.as_object_mut().expect("object").remove("userRef");
    let no_user_envelope = envelope(&no_user);
    ShellSession
        .validate(&no_user_envelope)
        .expect("a user-less row still validates");
    assert!(
        shell_session_execution(no_user_envelope.base(), no_user_envelope.provider_ref())
            .expect("execution")
            .1
            .is_none()
    );
    let zone = ZoneId::parse("work").expect("zone");
    let key = ResourceKey::new("work", SHELL_SESSION_TYPE, "session-1");
    let uid = [0x42u8; 16];
    assert!(
        matches!(
            ShellSession.desired_children(&child_context(&zone, &key, &uid), &no_user_envelope),
            Err(InteractionEffectError::InvalidResource)
        ),
        "the supervisor child needs the session's user"
    );
}
