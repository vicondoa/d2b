//! Contract coverage for the unified provider declaration (U3).
//!
//! The declaration replaces separately authored metadata with one typed
//! source. The suite walks the ways that source can lie about what a
//! provider actually runs:
//!
//! 1. AE1: one declaration produces matching operation, resource, component,
//!    and service identities.
//! 2. A method with no implementation, an implementation with no declaration,
//!    a duplicate identity, and an unsupported placement are all refused.
//! 3. AE14: a mutable `Provider` row cannot create a compiled privileged
//!    handler, because the implementation identity resolves against the
//!    deployment's admitted artifacts.
//! 4. A namespace-first service cannot advertise filesystem presentation, and
//!    no role name or seccomp label supplies the missing capability.
//!
//! The fixture is a namespace-first volume-serving provider: a
//! `VolumeBinding` controller and a `volume-virtiofs` service whose admitted
//! source is realized inside its own verified sandbox rather than by a
//! Process mount the broker never applied.

use std::sync::{Arc, LazyLock};

use async_trait::async_trait;
use d2b_contracts_provider::v3::{
    AdmittedProviderArtifact, ArtifactDigest, ArtifactDigestSet, BinaryRef,
    CapabilitySupport, ComponentDescriptor, ComponentExecution, ComponentTargetCapability,
    ComponentType, CompatibilityRange, ControllerInstanceScope, ControllerTargetKind,
    DeclaredComponent, DeclaredMethod, DeclaredPlacement, DeclaredService, EffectPortClass,
    PolicyEvaluation, PresentationCapability, ProviderContractError, ProviderDeclarationSpec,
    ProviderManifest, ProviderSpec, RequiredResourceCapability, ResourceApiBinding,
    RevocationState, SetupRestriction, SignatureState, StandardCapabilityMatrix,
    TargetRuntimeArtifacts, TrustEvidence, UpgradeDisposition, UpgradePolicy,
};
use d2b_contracts_resource::v3::{
    ArtifactId, CanonicalJsonObject, OperationImplementation, ResourceRef, ResourceTypeName,
    SchemaFingerprint, SchemaVersion, canonical_json_bytes,
    execution_policy::{BoundedText, BoundedToken, ExecutionDomain},
    resource_schema::PlacementAnchor,
};
use d2b_provider_toolkit::declaration::provider::{
    ProviderDeclaration, ProviderDeclarationError,
};
use d2b_provider_toolkit::emit_declaration_canonical;
use d2b_resource_runtime::context::SpecDecoder;
use d2b_resource_runtime::driver::{DynResourceDriver, ResourceDriverFactory};
use d2b_resource_runtime::identity::{ResourceKey, ResourceTypeName as RuntimeResourceTypeName};
use d2b_resource_types::{
    AllowedSources, CONVERTED_TYPE_VERBS, DriverDescriptor, OperationCtx,
    OperationDef, OperationFailure, OperationHandler, OperationResult,
    ProviderImplementationBindings, ServiceDecl, ServiceMethod, ValidatedPayload, WellKnownType,
};

const ARTIFACT: &str = "provider-volume-virtiofs";
const CONTROLLER: &str = "volume-binding";
const SERVICE: &str = "volume-virtiofs";
const SERVICE_ID: &str = "volume-virtiofs.d2bus.org/export";
const EXPORT_METHOD: &str = "export";
const CLOSE_METHOD: &str = "close";
const EXPORT_OPERATION: &str = "export-volume";
const CLOSE_OPERATION: &str = "close-volume";
const DIGEST_A: &str =
    "sha256:0000000000000000000000000000000000000000000000000000000000000001";
const DIGEST_B: &str =
    "sha256:0000000000000000000000000000000000000000000000000000000000000002";

// ---------------------------------------------------------------------------
// Fixture
// ---------------------------------------------------------------------------

fn token(value: &str) -> BoundedToken {
    BoundedToken::parse(value).expect("bounded token")
}

fn resource_type(value: &str) -> ResourceTypeName {
    ResourceTypeName::parse(value).expect("registered resource type")
}

fn digest(value: &str) -> ArtifactDigest {
    ArtifactDigest::parse(value).expect("canonical digest")
}

fn fingerprint(tail: char) -> SchemaFingerprint {
    SchemaFingerprint::parse(format!("sha256:{}{}", "0".repeat(63), tail)).expect("fingerprint")
}

fn binary_ref(value: &str) -> BinaryRef {
    BinaryRef::parse(value).expect("component binary reference")
}

fn artifact_id() -> ArtifactId {
    ArtifactId::parse(ARTIFACT).expect("artifact identifier")
}

fn digests() -> ArtifactDigestSet {
    ArtifactDigestSet {
        executable: digest(DIGEST_A),
        config: digest(DIGEST_B),
        schema: digest(DIGEST_B),
        service: digest(DIGEST_B),
    }
}

fn trusted() -> TrustEvidence {
    TrustEvidence {
        publisher: token("first-party"),
        root_epoch: 1,
        publisher_trusted: true,
        signature: SignatureState::Valid,
        revocation: RevocationState::Clear,
        emergency_deny: false,
        provenance: PolicyEvaluation::Accepted,
        sbom: PolicyEvaluation::Accepted,
        license: PolicyEvaluation::Accepted,
        vulnerability: PolicyEvaluation::Accepted,
        conformance: PolicyEvaluation::Accepted,
        support_channel: token("stable"),
    }
}

fn compatibility() -> CompatibilityRange {
    CompatibilityRange {
        api_major: 3,
        api_minor: 4,
        descriptor_fingerprint: fingerprint('1'),
        state_schema_version: SchemaVersion::new(2, 3).expect("state schema version"),
    }
}

fn upgrade_policy() -> UpgradePolicy {
    UpgradePolicy {
        drain_before_upgrade: true,
        max_automatic_disposition: UpgradeDisposition::InPlace,
        preserves_durable_state: true,
    }
}

/// The `VolumeBinding` controller: a Zone singleton that owns no method and
/// publishes one service through the provider's descriptor.
fn controller_descriptor() -> ComponentDescriptor {
    ComponentDescriptor::new(
        token(CONTROLLER),
        ComponentType::Controller,
        [resource_type("VolumeBinding")],
        [],
        [ExecutionDomain::System],
        1,
        digest(DIGEST_B),
        [],
    )
    .expect("controller descriptor")
    .with_execution(ComponentExecution::Launchable {
        binary_ref: binary_ref(CONTROLLER),
    })
    .with_controller_placement(ControllerInstanceScope::ZoneSingleton, [ControllerTargetKind::Zone])
    .expect("zone singleton placement")
    .with_target_capabilities([ComponentTargetCapability::new(
        ControllerTargetKind::Zone,
        digest(DIGEST_B),
        [],
    )
    .expect("zone target capability")])
    .expect("zone target capabilities")
}

/// The namespace-first service: it realizes the admitted source inside its own
/// verified sandbox, so it declares no host capability and never claims a
/// Process mount.
fn service_descriptor() -> ComponentDescriptor {
    ComponentDescriptor::new(
        token(SERVICE),
        ComponentType::Service,
        [],
        [token(EXPORT_METHOD), token(CLOSE_METHOD)],
        [ExecutionDomain::System],
        1,
        digest(DIGEST_B),
        [],
    )
    .expect("service descriptor")
    .with_execution(ComponentExecution::Launchable {
        binary_ref: binary_ref(SERVICE),
    })
    .with_target_capabilities([ComponentTargetCapability::new(
        ControllerTargetKind::Host,
        digest(DIGEST_B),
        [EffectPortClass::Volume],
    )
    .expect("host target capability")])
    .expect("host target capabilities")
}

fn declared_component(
    presentation: PresentationCapability,
    method_presentation: PresentationCapability,
) -> DeclaredComponent {
    let component = service_descriptor();
    DeclaredComponent::new(
        component,
        DeclaredPlacement::new([ControllerTargetKind::Host], None).expect("host placement"),
        presentation,
        SetupRestriction::required_for(presentation).iter().copied(),
        vec![
            DeclaredMethod::new(token(EXPORT_METHOD), None, method_presentation),
            DeclaredMethod::new(token(CLOSE_METHOD), None, method_presentation),
        ],
        vec![DeclaredService::new(SERVICE_ID, [token(EXPORT_METHOD), token(CLOSE_METHOD)])
            .expect("declared service")],
        [],
    )
    .expect("declared component")
}

fn controller_component() -> DeclaredComponent {
    DeclaredComponent::new(
        controller_descriptor(),
        DeclaredPlacement::new([ControllerTargetKind::Zone], Some(PlacementAnchor::Zone))
            .expect("zone placement"),
        PresentationCapability::None,
        [],
        Vec::new(),
        Vec::new(),
        Vec::new(),
    )
    .expect("declared controller component")
}

fn spec(components: Vec<DeclaredComponent>) -> ProviderDeclarationSpec {
    ProviderDeclarationSpec::new(artifact_id(), components, []).expect("declaration spec")
}

fn fixture_spec() -> ProviderDeclarationSpec {
    spec(vec![
        controller_component(),
        declared_component(
            PresentationCapability::NamespaceFirstServiceSource,
            PresentationCapability::NamespaceFirstServiceSource,
        ),
    ])
}

fn required_capability_spec() -> ProviderDeclarationSpec {
    ProviderDeclarationSpec::new(
        artifact_id(),
        [controller_component()],
        [RequiredResourceCapability::new(
            resource_type("VolumeBinding"),
            token("expedited-reconcile"),
        )],
    )
    .expect("declaration spec with a required capability")
}

fn unsupported_capability_spec() -> ProviderDeclarationSpec {
    ProviderDeclarationSpec::new(
        artifact_id(),
        [controller_component()],
        [RequiredResourceCapability::new(
            resource_type("VolumeBinding"),
            token("rescheduling"),
        )],
    )
    .expect("declaration spec with an unsupported required capability")
}

// ---------------------------------------------------------------------------
// The binding half: the local constructors and functions
// ---------------------------------------------------------------------------

struct NoDecoder;

impl SpecDecoder for NoDecoder {
    fn decode(
        &self,
        _envelope: &[u8],
    ) -> Result<Box<dyn std::any::Any + Send>, Box<dyn std::error::Error + Send + Sync>> {
        Ok(Box::new(()))
    }
}

struct NoFactory;

#[async_trait]
impl ResourceDriverFactory for NoFactory {
    fn resource_types(&self) -> &[RuntimeResourceTypeName] {
        &[]
    }

    async fn create(&self, _key: &ResourceKey) -> Box<dyn DynResourceDriver> {
        unreachable!("the declaration contract never creates a driver")
    }
}

struct DeclaredHandler;

#[async_trait]
impl OperationHandler for DeclaredHandler {
    async fn execute(
        &self,
        _ctx: OperationCtx<'_>,
        _payload: ValidatedPayload,
    ) -> Result<OperationResult, OperationFailure> {
        Ok(OperationResult::new(CanonicalJsonObject::empty()))
    }
}

static EXPORT_SERVICE: ServiceDecl = ServiceDecl {
    id: SERVICE_ID,
    methods: &[
        // The operation facet carries the committed operation row's own
        // name, which is the spelling the envelope resolves a forwarded
        // invocation with.
        ServiceMethod::serving(EXPORT_OPERATION, EXPORT_METHOD),
        ServiceMethod::serving(CLOSE_OPERATION, CLOSE_METHOD),
    ],
    attach_kinds: &[],
    streams: &[],
    endpoint_policy: None,
};

static OPERATIONS: LazyLock<[OperationDef; 2]> = LazyLock::new(|| {
    [
        OperationDef {
            operation_ref: ResourceRef::parse("Operation/export-volume")
                .expect("declared operation reference"),
            handler: &DeclaredHandler,
        },
        OperationDef {
            operation_ref: ResourceRef::parse("Operation/close-volume")
                .expect("declared operation reference"),
            handler: &DeclaredHandler,
        },
    ]
});

/// One driver descriptor, carrying the given service and operation tables.
fn driver_descriptor(
    services: &'static [ServiceDecl],
    operations: &'static [OperationDef],
) -> DriverDescriptor {
    DriverDescriptor {
        resource_type: WellKnownType::VOLUME_BINDING,
        allowed_sources: AllowedSources::BUILTIN,
        verbs: CONVERTED_TYPE_VERBS,
        execution: &["system"],
        exportable: true,
        reads: &[],
        operations,
        creations: &[],
        startup: &[],
        services,
        decoder: Arc::new(NoDecoder),
        factory: Arc::new(NoFactory),
    }
}

static EXPORT_SERVICES: &[ServiceDecl] = &[EXPORT_SERVICE];

static DRIVERS: LazyLock<[DriverDescriptor; 1]> =
    LazyLock::new(|| [driver_descriptor(EXPORT_SERVICES, export_operations())]);

/// The two declared operation rows and their handlers.
fn export_operations() -> &'static [OperationDef] {
    &OPERATIONS[..]
}

fn bindings() -> ProviderImplementationBindings {
    ProviderImplementationBindings::new(&DRIVERS[..])
}

fn declaration(spec: ProviderDeclarationSpec) -> ProviderDeclaration {
    ProviderDeclaration::new(spec, bindings())
}

fn fixture_declaration() -> ProviderDeclaration {
    declaration(fixture_spec())
}

// ---------------------------------------------------------------------------
// The signed half: the deployment-admitted artifact
// ---------------------------------------------------------------------------

fn manifest(
    artifact: &str,
    components: Vec<ComponentDescriptor>,
    matrix: StandardCapabilityMatrix,
) -> ProviderManifest {
    let binding = ResourceApiBinding::new_with_placement(
        resource_type("VolumeBinding"),
        SchemaVersion::new(1, 0).expect("spec version"),
        fingerprint('2'),
        SchemaVersion::new(1, 0).expect("status version"),
        fingerprint('3'),
        matrix,
        None,
        None,
        PlacementAnchor::Zone,
    )
    .expect("resource api binding");
    ProviderManifest::new(
        ArtifactId::parse(artifact).expect("artifact identifier"),
        digests(),
        trusted(),
        compatibility(),
        components,
        [binding],
        [],
        upgrade_policy(),
    )
    .expect("provider manifest")
    .with_target_runtime_artifacts([TargetRuntimeArtifacts::new(
        ControllerTargetKind::Host,
        digest(DIGEST_A),
        digest(DIGEST_A),
    )
    .expect("host runtime artifacts")])
    .expect("runtime artifacts")
}

fn admitted_artifact() -> AdmittedProviderArtifact {
    AdmittedProviderArtifact::new(manifest(
        ARTIFACT,
        vec![
            controller_descriptor(),
            service_descriptor(),
        ],
        StandardCapabilityMatrix::new([(
            token("expedited-reconcile"),
            CapabilitySupport::Supported,
        )])
        .expect("capability matrix"),
    ))
    .expect("the deployment admitted the artifact")
}

fn row(artifact: &str) -> ProviderSpec {
    ProviderSpec::minimal(ArtifactId::parse(artifact).expect("artifact identifier"))
}

// ---------------------------------------------------------------------------
// AE1: one declaration, four matching identities
// ---------------------------------------------------------------------------

#[test]
fn one_declaration_produces_matching_operations_resources_components_and_services() {
    let declaration = fixture_declaration();
    declaration.validate().expect("the two halves agree");

    let identities = declaration
        .identities()
        .expect("the declaration projects its identities");

    assert_eq!(
        identities.components.iter().map(BoundedToken::as_str).collect::<Vec<_>>(),
        vec![CONTROLLER, SERVICE],
        "the component identities come from the one declaration"
    );
    assert_eq!(
        identities.resource_types.iter().map(String::as_str).collect::<Vec<_>>(),
        vec!["VolumeBinding"],
        "the owned resource types come from the same declaration"
    );
    assert_eq!(
        identities
            .services
            .iter()
            .map(BoundedText::as_str)
            .collect::<Vec<_>>(),
        vec![SERVICE_ID],
        "the service identity comes from the same declaration"
    );

    // Every declared method resolves to an implementation whose provider is
    // the declaring artifact and whose component and method are the declared
    // identities, so the operation identity and the service identity name the
    // same method rather than two spellings of it.
    let resolved = identities.operations;
    assert_eq!(resolved.len(), 2);
    for implementation in &resolved {
        assert_eq!(
            implementation.provider().to_canonical_string(),
            format!("Provider/{ARTIFACT}"),
            "an implementation identity names the declaring provider, nothing else"
        );
    }
    assert!(
        resolved.iter().all(OperationImplementation::is_provider_method),
        "both methods are served by the declaring component, not by a Command row"
    );

    // The operation identity the declaration projects is exactly the
    // operation the bound service method names, so the broker's operation ->
    // service resolution and the provider's own handler table agree.
    for method in [EXPORT_METHOD, CLOSE_METHOD] {
        let implementation = declaration
            .spec()
            .implementation(&token(SERVICE), &token(method))
            .expect("the declared method resolves");
        let operation = EXPORT_SERVICE
            .method(method)
            .and_then(ServiceMethod::operation)
            .expect("the bound service method names an operation");
        assert!(
            OPERATIONS
                .iter()
                .any(|handler| handler.serves_named_operation(operation)),
            "the declared method `{method}` resolves to the bound operation `{operation}`"
        );
        assert!(matches!(
            implementation,
            OperationImplementation::ProviderMethod { .. }
        ));
    }
}

#[test]
fn a_resource_type_no_binding_serves_is_refused() {
    // The declaration owns `VolumeBinding`; the bindings serve no descriptor
    // at all, so the declared type has no implementation.
    let declaration = ProviderDeclaration::new(
        spec(vec![controller_component()]),
        ProviderImplementationBindings::new(&[]),
    );
    assert_eq!(
        declaration.validate(),
        Err(ProviderDeclarationError::ResourceTypeUndeclared(
            "VolumeBinding".to_owned()
        ))
    );
}

// ---------------------------------------------------------------------------
// Scenario 2: the refusals
// ---------------------------------------------------------------------------

#[test]
fn a_method_with_no_implementation_is_refused() {
    // The service declares `export` and `close`; the bound service answers
    // only `export`, so `close` has no handler to reach.
    static PARTIAL_SERVICE: ServiceDecl = ServiceDecl {
        id: SERVICE_ID,
        methods: &[ServiceMethod::serving(EXPORT_OPERATION, EXPORT_METHOD)],
        attach_kinds: &[],
        streams: &[],
        endpoint_policy: None,
    };
    static PARTIAL_OPERATIONS: LazyLock<[OperationDef; 1]> = LazyLock::new(|| {
        [OperationDef {
            operation_ref: ResourceRef::parse("Operation/export-volume")
                .expect("declared operation reference"),
            handler: &DeclaredHandler,
        }]
    });
    static PARTIAL_SERVICES: &[ServiceDecl] = &[PARTIAL_SERVICE];
    static PARTIAL_DRIVERS: LazyLock<[DriverDescriptor; 1]> =
        LazyLock::new(|| [driver_descriptor(PARTIAL_SERVICES, &PARTIAL_OPERATIONS[..])]);
    let declaration = ProviderDeclaration::new(
        fixture_spec(),
        ProviderImplementationBindings::new(&PARTIAL_DRIVERS[..]),
    );
    assert_eq!(
        declaration.validate(),
        Err(ProviderDeclarationError::MethodUnimplemented {
            component: SERVICE.to_owned(),
            method: CLOSE_METHOD.to_owned(),
        })
    );
}

#[test]
fn a_method_naming_an_operation_with_no_bound_handler_is_refused() {
    // The service method names a committed operation row, but the crate bound
    // no handler for it: a declared method that resolves to no code is still
    // a method with no implementation.
    static UNBOUND_SERVICE: ServiceDecl = ServiceDecl {
        id: SERVICE_ID,
        methods: &[
            ServiceMethod::serving(EXPORT_OPERATION, EXPORT_METHOD),
            ServiceMethod::serving("detach-volume", CLOSE_METHOD),
        ],
        attach_kinds: &[],
        streams: &[],
        endpoint_policy: None,
    };
    static UNBOUND_SERVICES: &[ServiceDecl] = &[UNBOUND_SERVICE];
    static UNBOUND_DRIVERS: LazyLock<[DriverDescriptor; 1]> =
        LazyLock::new(|| [driver_descriptor(UNBOUND_SERVICES, &OPERATIONS[..1])]);
    let declaration = ProviderDeclaration::new(
        fixture_spec(),
        ProviderImplementationBindings::new(&UNBOUND_DRIVERS[..]),
    );
    assert_eq!(
        declaration.validate(),
        Err(ProviderDeclarationError::MethodUnimplemented {
            component: SERVICE.to_owned(),
            method: CLOSE_METHOD.to_owned(),
        })
    );
}

#[test]
fn an_implementation_with_no_declaration_is_refused() {
    // The bound service answers a method the declaration never names.
    static EXTRA_SERVICE: ServiceDecl = ServiceDecl {
        id: SERVICE_ID,
        methods: &[
            ServiceMethod::serving(EXPORT_OPERATION, EXPORT_METHOD),
            ServiceMethod::serving(CLOSE_OPERATION, CLOSE_METHOD),
            ServiceMethod::serving("open-volume", "open"),
        ],
        attach_kinds: &[],
        streams: &[],
        endpoint_policy: None,
    };
    static EXTRA_OPERATIONS: LazyLock<[OperationDef; 3]> = LazyLock::new(|| {
        [
            OperationDef {
                operation_ref: ResourceRef::parse("Operation/export-volume")
                    .expect("declared operation reference"),
                handler: &DeclaredHandler,
            },
            OperationDef {
                operation_ref: ResourceRef::parse("Operation/close-volume")
                    .expect("declared operation reference"),
                handler: &DeclaredHandler,
            },
            OperationDef {
                operation_ref: ResourceRef::parse("Operation/open-volume")
                    .expect("declared operation reference"),
                handler: &DeclaredHandler,
            },
        ]
    });
    static EXTRA_SERVICES: &[ServiceDecl] = &[EXTRA_SERVICE];
    static EXTRA_DRIVERS: LazyLock<[DriverDescriptor; 1]> =
        LazyLock::new(|| [driver_descriptor(EXTRA_SERVICES, &EXTRA_OPERATIONS[..])]);
    let declaration = ProviderDeclaration::new(
        fixture_spec(),
        ProviderImplementationBindings::new(&EXTRA_DRIVERS[..]),
    );
    assert_eq!(
        declaration.validate(),
        Err(ProviderDeclarationError::ImplementationUndeclared {
            service: SERVICE_ID.to_owned(),
            method: "open".to_owned(),
        })
    );
}

#[test]
fn a_duplicate_component_identity_is_refused() {
    let duplicate = controller_component();
    let error = ProviderDeclarationSpec::new(
        artifact_id(),
        [controller_component(), duplicate],
        [],
    )
    .expect_err("two components cannot share one identity");
    assert_eq!(error, ProviderContractError::DuplicateDeclaration);
}

#[test]
fn a_duplicate_method_identity_is_refused() {
    // The session layer addresses a method by name, so two components
    // declaring one name would leave the bindings unable to say which
    // implementation a call reaches.
    let error = ProviderDeclarationSpec::new(
        artifact_id(),
        [
            controller_component(),
            declared_component(
                PresentationCapability::NamespaceFirstServiceSource,
                PresentationCapability::NamespaceFirstServiceSource,
            ),
            declared_component(
                PresentationCapability::NamespaceFirstServiceSource,
                PresentationCapability::NamespaceFirstServiceSource,
            ),
        ],
        [],
    )
    .expect_err("one method identity cannot be declared twice");
    assert_eq!(error, ProviderContractError::DuplicateDeclaration);
}

#[test]
fn an_unsupported_placement_is_refused() {
    // A namespace-first service is a narrow Host component, never a Zone
    // singleton: the Zone target is not in its signed target set.
    let descriptor = service_descriptor();
    let error = DeclaredComponent::new(
        descriptor,
        DeclaredPlacement::new(
            [ControllerTargetKind::Host, ControllerTargetKind::Zone],
            None,
        )
        .expect("placement declares two targets"),
        PresentationCapability::NamespaceFirstServiceSource,
        SetupRestriction::required_for(PresentationCapability::NamespaceFirstServiceSource)
            .iter()
            .copied(),
        vec![
            DeclaredMethod::new(
                token(EXPORT_METHOD),
                None,
                PresentationCapability::NamespaceFirstServiceSource,
            ),
            DeclaredMethod::new(
                token(CLOSE_METHOD),
                None,
                PresentationCapability::NamespaceFirstServiceSource,
            ),
        ],
        vec![DeclaredService::new(SERVICE_ID, [token(EXPORT_METHOD), token(CLOSE_METHOD)])
            .expect("declared service")],
        [],
    )
    .expect_err("a service cannot be placed at the Zone");
    assert_eq!(error, ProviderContractError::PlacementUnsupported);
}

// ---------------------------------------------------------------------------
// AE14: the mutable row cannot introduce code
// ---------------------------------------------------------------------------

#[test]
fn a_row_naming_an_unadmitted_artifact_cannot_resolve_an_implementation() {
    let declaration = fixture_declaration();
    let admitted = [admitted_artifact()];

    // A row that names the admitted artifact resolves to the declared method.
    let resolved = declaration
        .resolve_implementation(
            &admitted,
            &row(ARTIFACT),
            &token(SERVICE),
            &token(EXPORT_METHOD),
        )
        .expect("the deployment-admitted artifact serves the declared method");
    assert_eq!(
        resolved.provider().to_canonical_string(),
        format!("Provider/{ARTIFACT}")
    );

    // A mutable row naming an artifact no verified deployment admitted
    // resolves to nothing: it cannot select a compiled privileged handler.
    let unadmitted = row("provider-untrusted");
    assert_eq!(
        declaration.resolve_implementation(
            &admitted,
            &unadmitted,
            &token(SERVICE),
            &token(EXPORT_METHOD),
        ),
        Err(ProviderDeclarationError::ArtifactSelectionUnverified)
    );
    assert_eq!(
        row(ARTIFACT).artifact_id(),
        admitted[0].artifact_id(),
        "the deployment admitted exactly the artifact the row selects"
    );
}

#[test]
fn an_admitted_artifact_that_declares_no_such_method_cannot_serve_it() {
    // A second, validly signed artifact of the same provider that declares no
    // service component. The row selects it; the declaration still refuses,
    // because the signed manifest is what says the method exists.
    let other = AdmittedProviderArtifact::new(manifest(
        ARTIFACT,
        vec![controller_descriptor()],
        StandardCapabilityMatrix::new([]).expect("empty capability matrix"),
    ))
    .expect("the deployment admitted the second artifact");
    let declaration = fixture_declaration();
    assert_eq!(
        declaration.resolve_implementation(
            &[other],
            &row(ARTIFACT),
            &token(SERVICE),
            &token(EXPORT_METHOD),
        ),
        Err(ProviderDeclarationError::ImplementationNotAdmitted {
            component: SERVICE.to_owned(),
            method: EXPORT_METHOD.to_owned(),
        })
    );
}

#[test]
fn the_row_config_cannot_change_the_implementation_identity() {
    // `ProviderSpec` carries `artifactId` and `config`; only the artifact
    // selects code. Two rows with different configuration resolve to the same
    // implementation identity, so a mutable row cannot widen a call.
    let declaration = fixture_declaration();
    let admitted = [admitted_artifact()];
    let configured = ProviderSpec::new(
        artifact_id(),
        CanonicalJsonObject::parse(br#"{"concurrency":64}"#).expect("canonical config"),
    );
    let plain = declaration
        .resolve_implementation(
            &admitted,
            &row(ARTIFACT),
            &token(SERVICE),
            &token(EXPORT_METHOD),
        )
        .expect("the plain row resolves");
    let configured = declaration
        .resolve_implementation(
            &admitted,
            &configured,
            &token(SERVICE),
            &token(EXPORT_METHOD),
        )
        .expect("the configured row resolves to the same identity");
    assert_eq!(plain, configured);
}

#[test]
fn a_required_capability_the_binding_does_not_support_is_refused() {
    let admitted = AdmittedProviderArtifact::new(manifest(
        ARTIFACT,
        vec![
            controller_descriptor(),
            service_descriptor(),
        ],
        StandardCapabilityMatrix::new([(
            token("expedited-reconcile"),
            CapabilitySupport::Supported,
        )])
        .expect("capability matrix"),
    ))
    .expect("the deployment admitted the artifact");

    // Absence is not support: a capability the signed matrix does not list is
    // refused at deployment rather than discovered when an effect does
    // nothing.
    let unsupported = declaration(unsupported_capability_spec());
    assert_eq!(
        unsupported.admit_required_capabilities(&admitted),
        Err(ProviderDeclarationError::RequiredCapabilityUnsupported {
            resource_type: "VolumeBinding".to_owned(),
            capability: "rescheduling".to_owned(),
        })
    );
    // The same declaration with a capability the matrix supports admits.
    let supported = declaration(required_capability_spec());
    assert_eq!(supported.admit_required_capabilities(&admitted), Ok(()));
    // The same bound matrix admits the supported capability and refuses the
    // unsupported one, so the refusal came from the signed declaration rather
    // than from the projection.
    let matrix = admitted.manifest().binding_for(&resource_type("VolumeBinding"))
        .expect("the artifact binds VolumeBinding")
        .capability_matrix()
        .clone();
    assert!(matrix.supports(&token("expedited-reconcile")));
    assert!(!matrix.supports(&token("rescheduling")));
}

// ---------------------------------------------------------------------------
// Scenario 4: the closed presentation capability
// ---------------------------------------------------------------------------

#[test]
fn a_namespace_first_service_cannot_advertise_filesystem_presentation() {
    let error = DeclaredComponent::new(
        service_descriptor(),
        DeclaredPlacement::new([ControllerTargetKind::Host], None).expect("host placement"),
        PresentationCapability::NamespaceFirstServiceSource,
        SetupRestriction::required_for(PresentationCapability::NamespaceFirstServiceSource)
            .iter()
            .copied(),
        vec![
            DeclaredMethod::new(
                token(EXPORT_METHOD),
                None,
                PresentationCapability::FilesystemPresentation,
            ),
            DeclaredMethod::new(
                token(CLOSE_METHOD),
                None,
                PresentationCapability::NamespaceFirstServiceSource,
            ),
        ],
        vec![DeclaredService::new(SERVICE_ID, [token(EXPORT_METHOD), token(CLOSE_METHOD)])
            .expect("declared service")],
        [],
    )
    .expect_err("a namespace-first service cannot present a filesystem");
    assert_eq!(error, ProviderContractError::PresentationUnsupported);
    assert_eq!(error.code(), "presentation-unsupported");
}

#[test]
fn a_namespace_first_service_without_its_setup_restrictions_is_refused() {
    // A namespace-first service keeps ADR 0021's zero-host-capability launch
    // and never changes the broker's steady-state mount namespace. Dropping
    // either restriction is a promise the implementation does not make.
    let restrictions: Vec<SetupRestriction> =
        SetupRestriction::required_for(PresentationCapability::NamespaceFirstServiceSource)
            .to_vec();
    for dropped in &restrictions {
        let descriptor = service_descriptor();
        let remaining: Vec<_> = restrictions
            .iter()
            .copied()
            .filter(|restriction| *restriction != *dropped)
            .collect();
        let error = DeclaredComponent::new(
            descriptor,
            DeclaredPlacement::new([ControllerTargetKind::Host], None).expect("host placement"),
            PresentationCapability::NamespaceFirstServiceSource,
            remaining,
            vec![
                DeclaredMethod::new(
                    token(EXPORT_METHOD),
                    None,
                    PresentationCapability::NamespaceFirstServiceSource,
                ),
                DeclaredMethod::new(
                    token(CLOSE_METHOD),
                    None,
                    PresentationCapability::NamespaceFirstServiceSource,
                ),
            ],
            vec![DeclaredService::new(SERVICE_ID, [token(EXPORT_METHOD), token(CLOSE_METHOD)])
                .expect("declared service")],
            [],
        )
        .expect_err("the setup restriction is required by the presentation");
        assert_eq!(error, ProviderContractError::SetupRestrictionIncompatible);
    }
    assert_eq!(
        restrictions.len(),
        2,
        "both source-facing restrictions exist, so the loop covered each"
    );
}

#[test]
fn no_role_or_seccomp_name_can_supply_the_missing_presentation() {
    // The presentation facet is closed: the wire form accepts the declared
    // fields and refuses anything else, so a `role`, `seccompProfile`,
    // `workerRole`, or `mountPolicy` member has nowhere to arrive. A name
    // cannot become a capability the implementation does not have.
    for member in [
        r#""role":"Role/virtiofs-export""#,
        r#""seccompProfile":"SeccompProfile/desktop""#,
        r#""workerRole":"virtiofsd""#,
        r#""mountPolicy":{"destination":"/state"}"#,
    ] {
        let document = format!(
            r#"{{"name":"export","presentation":"namespace-first-service-source",{member}}}"#
        );
        assert!(
            serde_json::from_str::<DeclaredMethod>(&document).is_err(),
            "`{member}` must not be accepted on a declared method"
        );
    }
    // The same method without those members decodes, and its presentation is
    // the closed facet value - not a name.
    let decoded: DeclaredMethod = serde_json::from_str(
        r#"{"name":"export","presentation":"namespace-first-service-source"}"#,
    )
    .expect("the closed facet form decodes");
    assert_eq!(
        decoded.presentation(),
        PresentationCapability::NamespaceFirstServiceSource
    );
    assert!(
        !PresentationCapability::NamespaceFirstServiceSource
            .realizes(PresentationCapability::FilesystemPresentation)
    );
}

// ---------------------------------------------------------------------------
// Verification: the projection is deterministic and carries no key or state
// ---------------------------------------------------------------------------

#[test]
fn the_declaration_projection_is_deterministic() {
    let first = emit_declaration_canonical(&fixture_spec());
    let second = emit_declaration_canonical(&fixture_spec());
    assert_eq!(first, second, "the same declaration projects the same bytes");

    // The set-shaped facets canonicalize independently of their authoring
    // order, so two spellings of the same provider project the same bytes.
    // Component order is declaration order and is part of the projection.
    let mut spec = fixture_spec();
    let components = spec.components_mut();
    components.reverse();
    assert_eq!(
        emit_declaration_canonical(&spec),
        canonical_json_bytes(&spec).expect("canonical bytes"),
        "the projection is exactly the canonical serialization of the spec"
    );
}

#[test]
fn the_declaration_projection_carries_no_signing_key_or_runtime_state() {
    let bytes = emit_declaration_canonical(&fixture_spec());
    let document: serde_json::Value =
        serde_json::from_slice(&bytes).expect("the projection is JSON");

    // Walk every member name in the projection. A signing key, a handler, a
    // decoder or factory reference, and a runtime-state field would all have
    // to appear as a member name to reach the generated output, so the closed
    // list below is the whole way such a value could get there.
    const FORBIDDEN_MEMBERS: &[&str] = &[
        "privateKey",
        "private_key",
        "publicKey",
        "public_key",
        "signingKey",
        "signing_key",
        "signature",
        "sign",
        "trust",
        "handler",
        "factory",
        "decoder",
        "serviceHandler",
        "runtimeState",
        "runtime_state",
        "pids",
        "fds",
        "stateCells",
    ];
    fn assert_clean(value: &serde_json::Value) {
        match value {
            serde_json::Value::Object(members) => {
                for (name, member) in members {
                    assert!(
                        !FORBIDDEN_MEMBERS.contains(&name.as_str()),
                        "the projection carries a forbidden member `{name}`"
                    );
                    assert_clean(member);
                }
            }
            serde_json::Value::Array(items) => {
                for item in items {
                    assert_clean(item);
                }
            }
            _ => {}
        }
    }
    assert_clean(&document);

    // The projection is a function of the serializable half alone. Two
    // declarations that share that half and differ in their local
    // constructors and functions project byte-identical inputs, so changing
    // a handler cannot change a generated manifest, a schema, or a Nix
    // projection.
    let rebound = ProviderDeclaration::new(
        fixture_spec(),
        ProviderImplementationBindings::new(&[]),
    );
    assert_eq!(
        emit_declaration_canonical(rebound.spec()),
        bytes,
        "the data projection does not depend on the binding half"
    );
    assert_ne!(
        rebound.validate(),
        Ok(()),
        "the same bindings change what the declaration is allowed to claim"
    );
}
