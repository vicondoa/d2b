//! Generated provider hosting: one declared implementation lookup (U9).
//!
//! The daemon used to register a provider three times in shared code: a
//! generated table of registered service ids, a handwritten match turning a
//! registered id into its `ServiceDecl`, and a second handwritten match
//! turning the same id into its hosting factory. A fourth table, the
//! per-provider operation envelope, resolved the same operation name under a
//! second spelling, so one invocation could reach a handler through one leg
//! and miss through the other.
//!
//! This suite covers the four things the generated composition must do before
//! the cutover installs it:
//!
//! 1. **AE1 and AE13.** A declaration hosts exactly its declared methods - no
//!    more, no fewer - and a declaration that asks for a facet the selected
//!    host cannot enforce is refused naming that facet.
//! 2. **Restart invalidates old invocation bindings.** A respawn advances the
//!    hosting generation, so a binding captured before it refuses instead of
//!    reaching a superseded implementation.
//! 3. **Duplicates fail before hosting.** A method mapped by two services and
//!    one composition site serving two methods are refused while the table is
//!    built, before any registry exists to host.
//! 4. **No registered service depends on a handwritten shared match.** The
//!    suite walks the generated registration and fails when any entry is
//!    served by a site another entry also uses, and every hosted method's
//!    handler is shown to run.
//!
//! Every input is the U3 declaration and the U4 graph projection. Nothing
//! here names a provider crate: the composition root passes in the
//! `ServiceDecl` rows and the executable factories, and the generator derives
//! the table from them.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock};

use async_trait::async_trait;
use d2b_contracts_provider::v3::projection::{
    BuiltArtifact, GRAPH_PROJECTION_CONTRACT_VERSION, PrivatePlanProjection, project_provider_graph,
};
use d2b_contracts_provider::v3::{
    ArtifactDigest, BinaryRef, ComponentDescriptor, ComponentExecution, ComponentTargetCapability,
    ComponentType, ControllerInstanceScope, ControllerTargetKind, DeclaredComponent, DeclaredMethod,
    DeclaredPlacement, DeclaredService, EffectPortClass, PresentationCapability,
    ProviderDeclarationSpec, SetupRestriction,
};
use d2b_contracts_resource::v3::{
    ArtifactId, CanonicalJsonObject, ResourceRef, ResourceTypeName,
    execution_policy::{BoundedToken, ExecutionDomain},
    resource_schema::PlacementAnchor,
};
use d2b_provider_toolkit::declaration::provider::ProviderDeclaration;
use d2b_provider_toolkit::hosting::{
    DeclaredBindings, HostSupport, HostingRefusal, ImplementationBinding, ImplementationFactory,
    InvocationTarget, MethodFacet, MethodKey, ProviderExport, generate_hosting_registry,
};
use d2b_provider_toolkit::service::{EffectResponse, EffectService, EffectServiceError, ServiceInvocation};
use d2b_resource_runtime::context::{ServiceResourceContext, SpecDecoder};
use d2b_resource_runtime::driver::{DynResourceDriver, ResourceDriverFactory};
use d2b_resource_runtime::identity::{ResourceKey, ResourceTypeName as RuntimeResourceTypeName};
use d2b_resource_types::{
    AllowedSources, CONVERTED_TYPE_VERBS, DriverDescriptor, MethodFdContract, OperationCtx,
    OperationDef, OperationFailure, OperationHandler, OperationResult,
    ProviderImplementationBindings, ServiceDecl, ServiceMethod, ValidatedPayload, WellKnownType,
};

const ARTIFACT: &str = "provider-volume-virtiofs";
const CONTROLLER: &str = "volume-binding";
const SERVICE: &str = "volume-virtiofs";
const SERVICE_ID: &str = "volume-virtiofs.d2bus.org/export";
const MIRROR_SERVICE_ID: &str = "volume-virtiofs.d2bus.org/mirror";
const EXPORT_METHOD: &str = "export";
const CLOSE_METHOD: &str = "close";
const INSPECT_METHOD: &str = "inspect";
const EXPORT_OPERATION: &str = "export-volume";
const CLOSE_OPERATION: &str = "close-volume";
const INSPECT_OPERATION: &str = "inspect-volume";
const DIGEST_B: &str = "sha256:0000000000000000000000000000000000000000000000000000000000000002";
const EXECUTABLE_SET: &str =
    "sha256:0000000000000000000000000000000000000000000000000000000000000003";
/// The `sha256` digest of the canonical root configuration schema below.
const CONFIG_DIGEST: &str =
    "sha256:a2c799262a3ce3c19ef5cdd983bf3d12b43ab3c426227091b909dcb7054738c0";
const CONFIG_SCHEMA: &[u8] = br#"{"type":"object"}"#;

// ---------------------------------------------------------------------------
// The composition root's own implementations
// ---------------------------------------------------------------------------

/// One provider's own implementation of a declared service.
///
/// Which declared method answers is provider-owned code inside the
/// provider's own service: the composition root binds the service once and
/// the generated table never routes by a family, a service, or a method
/// name of its own.
struct ServiceImpl {
    service: &'static str,
    methods: &'static [&'static str],
    served: &'static [&'static AtomicUsize],
}

#[async_trait]
impl EffectService for ServiceImpl {
    async fn handle(
        &self,
        invocation: ServiceInvocation<'_>,
    ) -> Result<EffectResponse, EffectServiceError> {
        let index =
            self.methods
                .iter()
                .position(|method| *method == invocation.method)
                .ok_or_else(|| EffectServiceError::Declined {
                    service: self.service.to_owned(),
                    reason: format!("method `{}` is outside the declaration", invocation.method),
                })?;
        self.served[index].fetch_add(1, Ordering::SeqCst);
        Ok(EffectResponse::new(CanonicalJsonObject::empty()))
    }
}

/// How many invocations each declared method's handler has answered.
static EXPORT_SERVED: AtomicUsize = AtomicUsize::new(0);
static CLOSE_SERVED: AtomicUsize = AtomicUsize::new(0);
static INSPECT_SERVED: AtomicUsize = AtomicUsize::new(0);

/// The methods the fixture's service declares, in declaration order, beside
/// the counter each one answers into.
const SERVICE_METHODS: &[&str] = &[EXPORT_METHOD, CLOSE_METHOD, INSPECT_METHOD];
const SERVICE_COUNTERS: &[&AtomicUsize] = &[&EXPORT_SERVED, &CLOSE_SERVED, &INSPECT_SERVED];

/// The composition-root site that binds one declared service.
struct VolumeVirtiofsSite;

impl ImplementationFactory for VolumeVirtiofsSite {
    fn build(&self) -> Arc<dyn EffectService> {
        Arc::new(ServiceImpl {
            service: SERVICE_ID,
            methods: SERVICE_METHODS,
            served: SERVICE_COUNTERS,
        })
    }
}

/// The generated binding for the fixture's declared service.
const SERVICE_BINDING: ImplementationBinding =
    ImplementationBinding::new("composition-root/volume-virtiofs.d2bus.org/export", &VolumeVirtiofsSite);

/// The composition-root site that binds the mirror service. It is a distinct
/// type and a distinct site, because one site per declared service is exactly
/// what the generated table enforces.
struct VolumeVirtiofsMirrorSite;

impl ImplementationFactory for VolumeVirtiofsMirrorSite {
    fn build(&self) -> Arc<dyn EffectService> {
        Arc::new(ServiceImpl {
            service: MIRROR_SERVICE_ID,
            methods: SERVICE_METHODS,
            served: SERVICE_COUNTERS,
        })
    }
}

/// The generated binding for the fixture's declared mirror service.
const MIRROR_BINDING: ImplementationBinding =
    ImplementationBinding::new("composition-root/volume-virtiofs.d2bus.org/mirror", &VolumeVirtiofsMirrorSite);

// ---------------------------------------------------------------------------
// Fixture: the declared contract
// ---------------------------------------------------------------------------

fn token(value: &str) -> BoundedToken {
    BoundedToken::parse(value).expect("bounded token")
}

fn digest(value: &str) -> ArtifactDigest {
    ArtifactDigest::parse(value).expect("canonical digest")
}

fn artifact_id() -> ArtifactId {
    ArtifactId::parse(ARTIFACT).expect("artifact identifier")
}

fn resource_type(value: &str) -> ResourceTypeName {
    ResourceTypeName::parse(value).expect("registered resource type")
}

/// The zone-singleton controller that owns the `VolumeBinding` driver
/// descriptor and declares no method of its own.
fn controller_descriptor() -> ComponentDescriptor {
    ComponentDescriptor::new(
        token(CONTROLLER),
        ComponentType::Controller,
        [resource_type("VolumeBinding")],
        [],
        [ExecutionDomain::System],
        1,
        digest(CONFIG_DIGEST),
        [],
    )
    .expect("controller descriptor")
    .with_execution(ComponentExecution::Launchable {
        binary_ref: BinaryRef::parse(CONTROLLER).expect("component binary reference"),
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

/// The namespace-first service component: it realizes the admitted source
/// inside its own verified sandbox, so it declares no host capability.
fn service_descriptor(methods: &[&str]) -> ComponentDescriptor {
    ComponentDescriptor::new(
        token(SERVICE),
        ComponentType::Service,
        [],
        methods.iter().copied().map(token),
        [ExecutionDomain::System],
        1,
        digest(CONFIG_DIGEST),
        [],
    )
    .expect("service descriptor")
    .with_execution(ComponentExecution::Launchable {
        binary_ref: BinaryRef::parse(SERVICE).expect("component binary reference"),
    })
    .with_target_capabilities([ComponentTargetCapability::new(
        ControllerTargetKind::Host,
        digest(DIGEST_B),
        [EffectPortClass::Volume],
    )
    .expect("host target capability")])
    .expect("host target capabilities")
}

fn in_process(name: &str) -> DeclaredMethod {
    DeclaredMethod::new(
        token(name),
        None,
        PresentationCapability::NamespaceFirstServiceSource,
    )
}

/// One namespace-first service component declaring `methods` and serving each
/// of them from the named services.
fn service_component(methods: &[&str], services: &[(&str, &[&str])]) -> DeclaredComponent {
    let presentation = PresentationCapability::NamespaceFirstServiceSource;
    let declared = methods.iter().copied().map(in_process).collect::<Vec<_>>();
    let services = services
        .iter()
        .map(|(id, answered)| {
            DeclaredService::new(*id, answered.iter().copied().map(token))
                .expect("declared service")
        })
        .collect::<Vec<_>>();
    DeclaredComponent::new(
        service_descriptor(methods),
        DeclaredPlacement::new([ControllerTargetKind::Host], None).expect("host placement"),
        presentation,
        SetupRestriction::required_for(presentation).iter().copied(),
        declared,
        services,
        [],
    )
    .expect("declared service component")
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

fn spec(services: &[DeclaredComponent]) -> ProviderDeclarationSpec {
    let mut components = vec![controller_component()];
    components.extend_from_slice(services);
    ProviderDeclarationSpec::new(artifact_id(), components, []).expect("declaration spec")
}

/// The fixture declaration: three in-process methods on one service.
fn fixture_spec() -> ProviderDeclarationSpec {
    spec(&[service_component(
        &[EXPORT_METHOD, CLOSE_METHOD, INSPECT_METHOD],
        &[(
            SERVICE_ID,
            &[EXPORT_METHOD, CLOSE_METHOD, INSPECT_METHOD],
        )],
    )])
}

// ---------------------------------------------------------------------------
// Fixture: the binding half
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
        unreachable!("the generated hosting contract never creates a driver")
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

/// One declared operation row, with the handler the declaration requires a
/// bound implementation for.
fn operation(name: &str) -> OperationDef {
    OperationDef {
        operation_ref: ResourceRef::parse(&format!("Operation/{name}"))
            .expect("declared operation reference"),
        handler: &DeclaredHandler,
    }
}

/// One service declaration carrying the given methods.
fn service_decl(id: &'static str, methods: &'static [ServiceMethod]) -> ServiceDecl {
    ServiceDecl {
        id,
        methods,
        attach_kinds: &[],
        streams: &[],
        endpoint_policy: None,
    }
}

/// The driver descriptor table one fixture's declaration binds.
///
/// The declaration's binding half is a real `&'static` slice with an owner:
/// the cell is declared next to the services and operations it carries, and
/// nothing leaks to manufacture the lifetime.
macro_rules! driver_table {
    ($cell:ident, $services:ident, $operations:ident) => {
        static $cell: LazyLock<[DriverDescriptor; 1]> = LazyLock::new(|| {
            [DriverDescriptor {
                resource_type: WellKnownType::VOLUME_BINDING,
                allowed_sources: AllowedSources::BUILTIN,
                verbs: CONVERTED_TYPE_VERBS,
                execution: &["system"],
                exportable: true,
                reads: &[],
                operations: &$operations,
                creations: &[],
                startup: &[],
                services: &$services,
                decoder: Arc::new(NoDecoder),
                factory: Arc::new(NoFactory),
            }]
        });
    };
}

/// Bind one declaration's serializable facets to its local constructors and
/// functions.
fn declaration(
    declaration_spec: ProviderDeclarationSpec,
    descriptor_table: &'static [DriverDescriptor],
) -> ProviderDeclaration {
    ProviderDeclaration::new(
        declaration_spec,
        ProviderImplementationBindings::new(descriptor_table),
    )
}

/// The U4 graph projection of one declaration, as the composition root reads
/// it before it hosts anything.
fn plan(declaration_spec: &ProviderDeclarationSpec) -> PrivatePlanProjection {
    project_provider_graph(
        &[declaration_spec],
        &[BuiltArtifact::new(
            artifact_id(),
            GRAPH_PROJECTION_CONTRACT_VERSION,
            digest(EXECUTABLE_SET),
            digest(EXECUTABLE_SET),
            CONFIG_SCHEMA.to_vec(),
        )],
        &[],
    )
    .expect("graph projection")
}

/// A hosting site that enforces nothing but the descriptor legs, which is
/// what a plain hosting site really applies.
fn plain_host() -> HostSupport {
    HostSupport::STANDARD
}

/// The canonical `Provider/...` reference the declaration derives.
fn provider_ref() -> String {
    ResourceRef::parse(&format!("Provider/{ARTIFACT}"))
        .expect("provider reference")
        .to_canonical_string()
}

fn method_key(component: &str, service: &str, method: &str) -> MethodKey {
    MethodKey::new(provider_ref(), component, Some(service.to_owned()), method)
}

// ---------------------------------------------------------------------------
// Scenario 1: a declaration hosts exactly its methods (AE1)
// ---------------------------------------------------------------------------

#[test]
fn a_declaration_hosts_exactly_its_methods() {
    static METHODS: LazyLock<Vec<ServiceMethod>> = LazyLock::new(|| vec![
        ServiceMethod::serving(EXPORT_OPERATION, EXPORT_METHOD),
        ServiceMethod::serving(CLOSE_OPERATION, CLOSE_METHOD),
        ServiceMethod::serving_with(
            INSPECT_OPERATION,
            INSPECT_METHOD,
            None,
            MethodFdContract { max_fds: 2, fd_kind: Some("any") },
            MethodFdContract::NONE,
            &[],
            &[],
            None,
        ),
    ]);
    static SERVICES: LazyLock<Vec<ServiceDecl>> =
        LazyLock::new(|| vec![service_decl(SERVICE_ID, &METHODS)]);
    static OPERATIONS: LazyLock<Vec<OperationDef>> = LazyLock::new(|| vec![
        operation(EXPORT_OPERATION),
        operation(CLOSE_OPERATION),
        operation(INSPECT_OPERATION),
    ]);
    driver_table!(DESCRIPTORS, SERVICES, OPERATIONS);
    let declaration_spec = fixture_spec();
    let declaration = declaration(
        declaration_spec.clone(),
        &DESCRIPTORS[..],
    );
    let exports = vec![ProviderExport {
        declaration: &declaration,
        bindings: &[DeclaredBindings { service: SERVICE_ID, binding: &SERVICE_BINDING }],
    }];
    let registry = generate_hosting_registry(&exports, &plain_host(), &plan(&declaration_spec))
        .expect("generated hosting");

    // Exactly the declared methods, in declaration order, each with the
    // committed Operation it is the service surface of.
    let hosted: Vec<(String, String, Option<String>)> = registry
        .entries()
        .iter()
        .map(|entry| {
            (
                entry.key().component().to_owned(),
                entry.key().method().to_owned(),
                entry.operation().map(str::to_owned),
            )
        })
        .collect();
    assert_eq!(
        hosted,
        vec![
            (SERVICE.to_owned(), EXPORT_METHOD.to_owned(), Some(EXPORT_OPERATION.to_owned())),
            (SERVICE.to_owned(), CLOSE_METHOD.to_owned(), Some(CLOSE_OPERATION.to_owned())),
            (
                SERVICE.to_owned(),
                INSPECT_METHOD.to_owned(),
                Some(INSPECT_OPERATION.to_owned())
            ),
        ],
        "the table hosts exactly the declared methods, no more and no fewer"
    );

    // The controller declares no method, so it hosts none.
    assert!(
        registry.entries().iter().all(|entry| entry.key().component() == SERVICE),
        "a component that declares no method hosts nothing"
    );

    // Registry identity and executable handler reachability agree by
    // generated construction: one entry per projected method.
    assert_eq!(
        registry.entries().len(),
        plan(&declaration_spec).operations().len(),
        "the generated table and the graph projection cover the same methods"
    );

    // Both spellings of one declared method resolve to one entry.
    for (method, operation) in [
        (EXPORT_METHOD, EXPORT_OPERATION),
        (CLOSE_METHOD, CLOSE_OPERATION),
        (INSPECT_METHOD, INSPECT_OPERATION),
    ] {
        let by_operation = registry
            .bind(&InvocationTarget::Operation(operation.to_owned()))
            .expect("the declared Operation resolves");
        let by_method = registry
            .bind(&InvocationTarget::DeclaredMethod(method_key(SERVICE, SERVICE_ID, method)))
            .expect("the declared method resolves");
        assert_eq!(
            by_operation, by_method,
            "the Operation spelling and the declared method spelling are one entry"
        );
    }

    // The admitted carriage is what the host checked, not a copy of the row.
    let inspect = registry
        .bind(&InvocationTarget::DeclaredMethod(method_key(
            SERVICE,
            SERVICE_ID,
            INSPECT_METHOD,
        )))
        .expect("resolved");
    assert_eq!(
        registry.admit(&inspect).expect("admitted").capabilities().request_fds().max_fds,
        2
    );
    let export = registry
        .bind(&InvocationTarget::DeclaredMethod(method_key(SERVICE, SERVICE_ID, EXPORT_METHOD)))
        .expect("resolved");
    assert_eq!(
        registry.admit(&export).expect("admitted").capabilities().request_fds().max_fds,
        0
    );

    // An Operation nothing declares has no route: there is no fallback
    // handler and no second namespace to fall through to.
    assert_eq!(
        registry.bind(&InvocationTarget::Operation("inspect-host".to_owned())),
        Err(HostingRefusal::Undeclared {
            target: InvocationTarget::Operation("inspect-host".to_owned()),
        })
    );
}

// ---------------------------------------------------------------------------
// Scenario 1b: an unenforceable required facet is refused (AE13)
// ---------------------------------------------------------------------------

#[test]
fn a_declaration_asking_for_an_unsupported_facet_is_refused_named() {
    static METHODS: LazyLock<Vec<ServiceMethod>> = LazyLock::new(|| {
        vec![ServiceMethod::serving_with(
            EXPORT_OPERATION,
            EXPORT_METHOD,
            None,
            MethodFdContract::NONE,
            MethodFdContract::NONE,
            &["lifecycle-leases"],
            &[],
            None,
        )]
    });
    static SERVICES: LazyLock<Vec<ServiceDecl>> =
        LazyLock::new(|| vec![service_decl(SERVICE_ID, &METHODS)]);
    static OPERATIONS: LazyLock<Vec<OperationDef>> =
        LazyLock::new(|| vec![operation(EXPORT_OPERATION)]);
    driver_table!(DESCRIPTORS, SERVICES, OPERATIONS);
    let declaration_spec = spec(&[service_component(
        &[EXPORT_METHOD],
        &[(SERVICE_ID, &[EXPORT_METHOD])],
    )]);
    let declaration = declaration(
        declaration_spec.clone(),
        &DESCRIPTORS[..],
    );
    let exports = vec![ProviderExport {
        declaration: &declaration,
        bindings: &[DeclaredBindings { service: SERVICE_ID, binding: &SERVICE_BINDING }],
    }];

    // The plain site provides no state cell, so a declaration that declares
    // one is refused by name instead of being hosted with a cell name the
    // site would hand straight back unenforced.
    assert_eq!(
        generate_hosting_registry(&exports, &plain_host(), &plan(&declaration_spec))
            .expect_err("the unenforceable facet is refused"),
        HostingRefusal::FacetUnenforced {
            key: method_key(SERVICE, SERVICE_ID, EXPORT_METHOD),
            facet: MethodFacet::StateCells,
            declared: Some("lifecycle-leases".to_owned()),
        }
    );

    // A site that does provide the cell admits it, and what reaches the
    // handler is the admitted capability.
    const CELLS: &[&str] = &["lifecycle-leases"];
    let capable = HostSupport::new(&[], CELLS, &[], u8::MAX, u8::MAX);
    let registry = generate_hosting_registry(&exports, &capable, &plan(&declaration_spec))
        .expect("the capable site admits");
    let binding = registry
        .bind(&InvocationTarget::DeclaredMethod(method_key(
            SERVICE,
            SERVICE_ID,
            EXPORT_METHOD,
        )))
        .expect("resolved");
    assert_eq!(
        registry.admit(&binding).expect("admitted").capabilities().state_cells(),
        ["lifecycle-leases"]
    );
}

// ---------------------------------------------------------------------------
// Scenario 2: a service restart invalidates old invocation bindings
// ---------------------------------------------------------------------------

#[test]
fn a_service_restart_invalidates_old_invocation_bindings() {
    static METHODS: LazyLock<Vec<ServiceMethod>> =
        LazyLock::new(|| vec![ServiceMethod::serving(EXPORT_OPERATION, EXPORT_METHOD)]);
    static SERVICES: LazyLock<Vec<ServiceDecl>> =
        LazyLock::new(|| vec![service_decl(SERVICE_ID, &METHODS)]);
    static OPERATIONS: LazyLock<Vec<OperationDef>> =
        LazyLock::new(|| vec![operation(EXPORT_OPERATION)]);
    driver_table!(DESCRIPTORS, SERVICES, OPERATIONS);
    let declaration_spec = spec(&[service_component(
        &[EXPORT_METHOD],
        &[(SERVICE_ID, &[EXPORT_METHOD])],
    )]);
    let declaration = declaration(
        declaration_spec.clone(),
        &DESCRIPTORS[..],
    );
    let exports = vec![ProviderExport {
        declaration: &declaration,
        bindings: &[DeclaredBindings { service: SERVICE_ID, binding: &SERVICE_BINDING }],
    }];
    let registry = generate_hosting_registry(&exports, &plain_host(), &plan(&declaration_spec))
        .expect("hosted");

    let before = registry
        .bind(&InvocationTarget::Operation(EXPORT_OPERATION.to_owned()))
        .expect("resolved");
    assert_eq!(before.generation(), 1);
    assert!(registry.admit(&before).is_ok(), "the fresh binding admits");

    let after = registry
        .restart(&InvocationTarget::Operation(EXPORT_OPERATION.to_owned()))
        .expect("the restart rebuilds the implementation");
    assert_eq!(after.generation(), 2);
    assert_eq!(
        registry.admit(&before).expect_err("the old binding refuses"),
        HostingRefusal::GenerationMoved {
            key: method_key(SERVICE, SERVICE_ID, EXPORT_METHOD),
            expected: 1,
            current: 2,
        },
        "a binding captured before the restart refuses"
    );
    assert!(
        registry.admit(&after).is_ok(),
        "the binding captured at the restart admits"
    );
}

// ---------------------------------------------------------------------------
// Scenario 3: duplicates fail before hosting
// ---------------------------------------------------------------------------

#[test]
fn a_duplicate_method_mapping_fails_before_hosting() {
    // One declared method, mapped by two declared services of one component:
    // the shape a handwritten service-id table produces when two registered
    // ids resolve to the same declaration row.
    static METHODS: LazyLock<Vec<ServiceMethod>> =
        LazyLock::new(|| vec![ServiceMethod::serving(EXPORT_OPERATION, EXPORT_METHOD)]);
    static SERVICES: LazyLock<Vec<ServiceDecl>> = LazyLock::new(|| vec![
        service_decl(SERVICE_ID, &METHODS),
        service_decl(MIRROR_SERVICE_ID, &METHODS),
    ]);
    static OPERATIONS: LazyLock<Vec<OperationDef>> =
        LazyLock::new(|| vec![operation(EXPORT_OPERATION)]);
    driver_table!(DESCRIPTORS, SERVICES, OPERATIONS);
    let declaration_spec = spec(&[service_component(
        &[EXPORT_METHOD],
        &[(SERVICE_ID, &[EXPORT_METHOD]), (MIRROR_SERVICE_ID, &[EXPORT_METHOD])],
    )]);
    let declaration = declaration(
        declaration_spec.clone(),
        &DESCRIPTORS[..],
    );
    let exports = vec![ProviderExport {
        declaration: &declaration,
        bindings: &[
            DeclaredBindings { service: SERVICE_ID, binding: &SERVICE_BINDING },
            DeclaredBindings { service: MIRROR_SERVICE_ID, binding: &MIRROR_BINDING },
        ],
    }];
    assert_eq!(
        generate_hosting_registry(&exports, &plain_host(), &plan(&declaration_spec))
            .expect_err("the duplicate mapping is refused"),
        HostingRefusal::DuplicateMethodMapping {
            provider: provider_ref(),
            component: SERVICE.to_owned(),
            method: EXPORT_METHOD.to_owned(),
            first: SERVICE_ID.to_owned(),
            second: MIRROR_SERVICE_ID.to_owned(),
        }
    );
}

#[test]
fn a_duplicate_handler_mapping_fails_before_hosting() {
    // Two declared services with disjoint methods, served by one composition
    // site: the shape of a shared entry several registered identities land
    // on.
    static PRIMARY_METHODS: LazyLock<Vec<ServiceMethod>> =
        LazyLock::new(|| vec![ServiceMethod::serving(EXPORT_OPERATION, EXPORT_METHOD)]);
    static MIRROR_METHODS: LazyLock<Vec<ServiceMethod>> =
        LazyLock::new(|| vec![ServiceMethod::serving(CLOSE_OPERATION, CLOSE_METHOD)]);
    static SERVICES: LazyLock<Vec<ServiceDecl>> = LazyLock::new(|| vec![
        service_decl(SERVICE_ID, &PRIMARY_METHODS),
        service_decl(MIRROR_SERVICE_ID, &MIRROR_METHODS),
    ]);
    static OPERATIONS: LazyLock<Vec<OperationDef>> =
        LazyLock::new(|| vec![operation(EXPORT_OPERATION), operation(CLOSE_OPERATION)]);
    driver_table!(DESCRIPTORS, SERVICES, OPERATIONS);
    let declaration_spec = spec(&[service_component(
        &[EXPORT_METHOD, CLOSE_METHOD],
        &[(SERVICE_ID, &[EXPORT_METHOD]), (MIRROR_SERVICE_ID, &[CLOSE_METHOD])],
    )]);
    let declaration = declaration(declaration_spec.clone(), &DESCRIPTORS[..]);
    let exports = vec![ProviderExport {
        declaration: &declaration,
        bindings: &[
            DeclaredBindings { service: SERVICE_ID, binding: &SERVICE_BINDING },
            DeclaredBindings { service: MIRROR_SERVICE_ID, binding: &SERVICE_BINDING },
        ],
    }];
    assert_eq!(
        generate_hosting_registry(&exports, &plain_host(), &plan(&declaration_spec))
            .expect_err("the shared site is refused"),
        HostingRefusal::DuplicateHandler {
            site: "composition-root/volume-virtiofs.d2bus.org/export",
            first: SERVICE_ID.to_owned(),
            second: MIRROR_SERVICE_ID.to_owned(),
        }
    );
}

// ---------------------------------------------------------------------------
// Scenario 4: no registered service depends on a handwritten shared match
// ---------------------------------------------------------------------------

#[tokio::test]
async fn no_registered_service_depends_on_a_handwritten_shared_match() {
    static METHODS: LazyLock<Vec<ServiceMethod>> = LazyLock::new(|| vec![
        ServiceMethod::serving(EXPORT_OPERATION, EXPORT_METHOD),
        ServiceMethod::serving(CLOSE_OPERATION, CLOSE_METHOD),
        ServiceMethod::serving(INSPECT_OPERATION, INSPECT_METHOD),
    ]);
    static SERVICES: LazyLock<Vec<ServiceDecl>> =
        LazyLock::new(|| vec![service_decl(SERVICE_ID, &METHODS)]);
    static OPERATIONS: LazyLock<Vec<OperationDef>> = LazyLock::new(|| vec![
        operation(EXPORT_OPERATION),
        operation(CLOSE_OPERATION),
        operation(INSPECT_OPERATION),
    ]);
    driver_table!(DESCRIPTORS, SERVICES, OPERATIONS);
    let declaration_spec = fixture_spec();
    let declaration = declaration(
        declaration_spec.clone(),
        &DESCRIPTORS[..],
    );
    let exports = vec![ProviderExport {
        declaration: &declaration,
        bindings: &[DeclaredBindings { service: SERVICE_ID, binding: &SERVICE_BINDING }],
    }];
    let registry = generate_hosting_registry(&exports, &plain_host(), &plan(&declaration_spec))
        .expect("hosted");

    // Walk the generated registration. Every entry resolves to the site its
    // own declared service is bound to, and no site answers for two declared
    // services: a shared entry several registered identities land on is the
    // handwritten table this generation removes, and the walk fails on it.
    let mut owner_of_site: BTreeMap<&str, &str> = BTreeMap::new();
    let mut services: BTreeSet<&str> = BTreeSet::new();
    for entry in registry.entries() {
        let binding = registry
            .bind(&InvocationTarget::DeclaredMethod(entry.key().clone()))
            .expect("every generated entry resolves by its declared identity");
        let admitted = registry.admit(&binding).expect("every generated entry admits");
        let service = entry.key().service().expect("a hosted method names its service");
        assert_eq!(
            admitted.site(),
            entry.site(),
            "the resolved implementation is the entry's own site"
        );
        if let Some(owner) = owner_of_site.insert(entry.site(), service) {
            assert_eq!(
                owner, service,
                "site `{}` answers for both service `{owner}` and service `{service}`: that is a handwritten shared entry, not a generated binding",
                entry.site()
            );
        }
        services.insert(service);
    }
    assert_eq!(
        owner_of_site.len(),
        services.len(),
        "every declared service has its own composition site, and no site answers for two"
    );

    // Every admitted entry's handler is reachable: it runs, under the
    // declared method it was bound for.
    for (method, served) in [
        (EXPORT_METHOD, &EXPORT_SERVED),
        (CLOSE_METHOD, &CLOSE_SERVED),
        (INSPECT_METHOD, &INSPECT_SERVED),
    ] {
        let binding = registry
            .bind(&InvocationTarget::DeclaredMethod(method_key(SERVICE, SERVICE_ID, method)))
            .expect("resolved");
        let admitted = registry.admit(&binding).expect("admitted");
        let service = admitted.build();
        let before = served.load(Ordering::SeqCst);
        let payload = CanonicalJsonObject::empty();
        let mut resources = ServiceResourceContext::fail_closed();
        service
            .handle(ServiceInvocation {
                zone: "alpha",
                method,
                invocation_id: "invocation-1",
                payload: &payload,
                resources: &mut resources,
                state_cells: &[],
                kernel: None,
                request_fds: &[],
                response_fds: admitted.capabilities().response_fds(),
                payload_schema: admitted.capabilities().payload_schema(),
                chain_identities: &[],
            })
            .await
            .unwrap_or_else(|error| panic!("the declared handler for `{method}` answered: {error}"));
        assert_eq!(
            served.load(Ordering::SeqCst),
            before + 1,
            "the handler behind `{method}` ran exactly once"
        );
    }
}