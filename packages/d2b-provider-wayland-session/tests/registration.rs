//! WaylandSession registration and behavior boundary: the declaration the
//! plane registers, the row vocabulary the driver validates, and the manager
//! child rows the session's provider realization materializes into.

use std::sync::Arc;

use d2b_contracts_resource::v3::{ControllerGeneration, ResourceRef, ZoneId};
use d2b_core_controller::OwnedChildIntent;
use d2b_provider_display_wayland::{DisplayIdentity, WaylandSessionSpec};
use d2b_provider_wayland_policy::{
    InteractionChildContext, InteractionDriverArgs, InteractionDriverEffects,
    InteractionEffectError, InteractionEffectOutcome, InteractionEffectPhase,
    InteractionEffectRequest, InteractionFinalize, InteractionKind, InteractionSpecEnvelope,
    InteractionType,
};
use d2b_provider_wayland_session::{
    DisplayChildRequest, DisplayChildSource, WAYLAND_SESSION_TYPE, WaylandSession,
    wayland_session_descriptor, wayland_session_spec_decoder,
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

/// The display supervisor's child intents, as the daemon authors them.
struct TwoWorkers;

impl DisplayChildSource for TwoWorkers {
    fn display_children(
        &self,
        _request: &DisplayChildRequest<'_>,
    ) -> Result<Vec<OwnedChildIntent>, InteractionEffectError> {
        Ok(vec![
            intent(
                "Process/display-host-proxy-session",
                json!({"providerRef": "Provider/system-minijail", "template": "display-host-proxy"}),
            ),
            intent(
                "Endpoint/display-endpoint-session",
                json!({"producerRef": "Process/display-host-proxy-session"}),
            ),
        ])
    }
}

fn intent(target: &str, spec: Value) -> OwnedChildIntent {
    let body = json!({
        "apiVersion": "d2b.v3",
        "metadata": {"ownerRef": "display-wayland.d2bus.org.WaylandSession/display"},
        "spec": spec,
    });
    OwnedChildIntent::new(
        ResourceRef::parse(target).expect("child ref"),
        serde_json::to_vec(&body).expect("child body"),
        "sha256:display-session-child",
    )
    .expect("child intent")
}

fn descriptor() -> d2b_resource_types::DriverDescriptor {
    wayland_session_descriptor(InteractionDriverArgs {
        zone: "work".to_owned(),
        controller_generation: ControllerGeneration::new(3).expect("generation"),
        effects: Arc::new(UnusedEffects),
        behavior: WaylandSession::new(Arc::new(TwoWorkers)),
    })
}

/// One stored session row, as the compiled spec document the store holds.
fn session_spec() -> Value {
    let spec = WaylandSessionSpec::new(
        ResourceRef::parse("Guest/workstation").expect("guest"),
        ResourceRef::parse("Host/host-system").expect("host"),
        ResourceRef::parse("User/alice").expect("user"),
        ResourceRef::parse("display-wayland.d2bus.org.WaylandPolicy/policy").expect("policy"),
        DisplayIdentity::new("display", "#112233", "#223344", "#334455").expect("identity"),
        true,
    )
    .expect("session spec");
    serde_json::to_value(&spec).expect("spec json")
}

fn envelope(value: &Value) -> InteractionSpecEnvelope {
    let bytes = serde_json::to_vec(value).expect("spec bytes");
    let decoded = wayland_session_spec_decoder()
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
    assert_eq!(descriptor.resource_type, WellKnownType::WAYLAND_SESSION);
    assert_eq!(
        descriptor.allowed_sources,
        AllowedSources::BUILTIN | AllowedSources::STARTUP | AllowedSources::RUNTIME
    );
    assert!(!descriptor.exportable);
    assert_eq!(descriptor.execution, &["host", "guest"]);
    assert_eq!(
        descriptor.reads,
        &[
            WellKnownType::GUEST,
            WellKnownType::HOST,
            WellKnownType::USER,
            WellKnownType::WAYLAND_POLICY,
        ]
    );
    assert!(descriptor.operations.is_empty());
    assert!(descriptor.creations.is_empty());
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
            .contains_key(&ResourceTypeName::new(WAYLAND_SESSION_TYPE))
    );
    assert!(matches!(
        providers.register_driver(&descriptor()),
        Err(ProviderDirectoryError::DuplicateType(resource_type))
            if resource_type.as_str() == WAYLAND_SESSION_TYPE
    ));
}

/// The session's cross-domain trust is validated and its four dependencies
/// are watched in the preserved order.
#[test]
fn the_session_reads_its_domain_dependencies() {
    let envelope = envelope(&session_spec());
    let behavior = WaylandSession::new(Arc::new(TwoWorkers));
    assert_eq!(WaylandSession::RESOURCE_TYPE, WAYLAND_SESSION_TYPE);
    behavior.validate(&envelope).expect("the session validates");
    assert_eq!(
        behavior.dependencies(&envelope).expect("dependencies"),
        vec![
            ResourceRef::parse("Guest/workstation").expect("guest"),
            ResourceRef::parse("Host/host-system").expect("host"),
            ResourceRef::parse("User/alice").expect("user"),
            ResourceRef::parse("display-wayland.d2bus.org.WaylandPolicy/policy").expect("policy"),
        ]
    );
}

/// A row that is not a session spec is refused.
#[test]
fn a_foreign_row_is_refused() {
    let envelope = envelope(&json!({"providerRef": "Provider/display-wayland"}));
    let behavior = WaylandSession::new(Arc::new(TwoWorkers));
    assert_eq!(
        behavior.validate(&envelope),
        Err(InteractionEffectError::InvalidResource)
    );
}

/// The session's provider realization becomes manager child rows: the manager
/// owns the child identity, the intent body supplies the spec and the
/// authored metadata.
#[test]
fn the_provider_intents_become_manager_child_rows() {
    let envelope = envelope(&session_spec());
    let behavior = WaylandSession::new(Arc::new(TwoWorkers));
    let zone = ZoneId::parse("work").expect("zone");
    let key = ResourceKey::new("work", WAYLAND_SESSION_TYPE, "display");
    let uid = [0x42u8; 16];
    let children = behavior
        .desired_children(&child_context(&zone, &key, &uid), &envelope)
        .expect("children");

    assert_eq!(children.len(), 2);
    assert_eq!(children[0].type_name.as_str(), "Process");
    assert_eq!(children[0].name, "display-host-proxy-session");
    let spec: Value = serde_json::from_slice(&children[0].spec).expect("child spec");
    assert_eq!(spec["template"], "display-host-proxy");
    let metadata: Value = serde_json::from_slice(&children[0].metadata).expect("child metadata");
    assert_eq!(
        metadata["ownerRef"],
        "display-wayland.d2bus.org.WaylandSession/display"
    );
    assert_eq!(children[1].type_name.as_str(), "Endpoint");
    assert_eq!(children[1].name, "display-endpoint-session");
}

/// A child source that refuses is a spec failure the engine classifies.
#[test]
fn a_refusing_child_source_fails_the_derivation() {
    struct Refusing;

    impl DisplayChildSource for Refusing {
        fn display_children(
            &self,
            _request: &DisplayChildRequest<'_>,
        ) -> Result<Vec<OwnedChildIntent>, InteractionEffectError> {
            Err(InteractionEffectError::InvalidResource)
        }
    }

    let envelope = envelope(&session_spec());
    let behavior = WaylandSession::new(Arc::new(Refusing));
    let zone = ZoneId::parse("work").expect("zone");
    let key = ResourceKey::new("work", WAYLAND_SESSION_TYPE, "display");
    let uid = [0x42u8; 16];
    assert!(matches!(
        behavior.desired_children(&child_context(&zone, &key, &uid), &envelope),
        Err(InteractionEffectError::InvalidResource)
    ));
}
