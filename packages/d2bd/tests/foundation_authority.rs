//! The initial verified graph over the core metadata providers (U19; R1-R15,
//! AE1, AE14).
//!
//! Three things are proven here, each against the canonical contracts rather
//! than a test-local imitation of them:
//!
//! 1. **The initial verified graph identifies the declared providers and
//!    execution targets.** `Provider/system-core` is the one Provider that
//!    owns `Host` and `User`, and it declares four components: a reconciler
//!    and a hosted effects service per type, all placed at the Host
//!    execution target. The declared service and method identities are read
//!    from the same constants the bound descriptors carry, so the two halves
//!    cannot disagree, and `Provider/host` and `Provider/user` name no
//!    identity at all.
//! 2. **Untrusted artifact selection cannot add implementation authority.**
//!    A `Provider` row selects an artifact id and nothing else: an id the
//!    verified deployment admitted no manifest for resolves to nothing, and
//!    an admitted manifest that declares no such method cannot serve it. The
//!    resolved identity always names the declaring provider's own method
//!    (AE14).
//! 3. **A wrong identity or missing execution-parent support refuses a child
//!    request.** A parent's defaults belong to exactly the child they name,
//!    and only a support ceiling admits a child's binding request; a parent's
//!    own consumption and a parent's defaults are inputs to someone else's
//!    decision and grant nothing by themselves.
//!
//! The unchanged production foundation publication still writes the
//! pre-cutover posture and Command-materialized rows. That path is the
//! cutover's to replace and is deliberately not exercised here.

use std::collections::BTreeSet;
use std::sync::{Arc, LazyLock};

use d2b_contracts_provider::v3::projection::{
    BuiltArtifact, GRAPH_PROJECTION_CONTRACT_VERSION, GraphProjectionError, project_provider_graph,
};
use d2b_contracts_provider::v3::{
    AdmittedProviderArtifact, ArtifactDigest, ArtifactDigestSet, CompatibilityRange,
    PolicyEvaluation, ProviderManifest, ProviderSpec, ResourceApiBinding, RevocationState,
    SignatureState, StandardCapabilityMatrix, TargetRuntimeArtifacts, TrustEvidence,
    UpgradeDisposition, UpgradePolicy,
};

use d2b_provider_toolkit::declaration::provider::ProviderDeclarationError;
use d2b_contracts_resource::v3::execution_policy::BoundedToken;
use d2b_contracts_resource::v3::{
    AdmissionStage, BindingContractError, BindingKind, BindingRealizationFacet,
    BindingRealizationSupport, BindingRefusal, BindingSupportEntry,
    CanonicalJsonObject, ChildBindingRequest, ChildRequestDefaults, ChildSupportCeiling,
    DefaultedSource, ExecutionParentInput, OperationImplementation, RefusalReason, RequestedRights,
    ResourceRef, ResourceTypeName, SchemaFingerprint, SchemaVersion, StoreIncarnation, ZoneId,
    resource_schema::PlacementAnchor,
};
use d2b_contracts_resource::v3::ArtifactId;
use d2b_resource_types::{AllowedSources, CONVERTED_TYPE_VERBS};
use d2b_provider_system_core::{
    HOST_EFFECTS_SERVICE as CORE_HOST_SERVICE, OWNED_RESOURCE_TYPES, PROVIDER_REF,
    USER_EFFECTS_SERVICE as CORE_USER_SERVICE, system_core_declaration,
};
use d2b_provider_toolkit::{
    DriverDescriptor, ProviderImplementationBindings, ServiceDecl, ServiceMethod,
    UnifiedProviderDeclaration, WellKnownType,
};

const ZONE: &str = "foundation-authority";
const STORE: &str = "store-generation-1";
const DIGEST_A: &str =
    "sha256:0000000000000000000000000000000000000000000000000000000000000001";
const DIGEST_B: &str =
    "sha256:0000000000000000000000000000000000000000000000000000000000000002";

fn zone() -> ZoneId {
    ZoneId::parse(ZONE).expect("the fixture zone is canonical")
}

fn store() -> StoreIncarnation {
    StoreIncarnation::parse(STORE).expect("the fixture incarnation is a bounded token")
}

fn reference(value: &str) -> ResourceRef {
    ResourceRef::parse(value).expect("the fixture reference is canonical")
}

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

fn artifact_id(value: &str) -> ArtifactId {
    ArtifactId::parse(value).expect("artifact identifier")
}

// ---------------------------------------------------------------------------
// The composition root's binding half
// ---------------------------------------------------------------------------

/// The descriptor table this Provider's declaration is bound to.
///
/// The composition root binds the two family descriptors; this suite binds a
/// table of the same shape so the declaration's two halves are exercised
/// without standing up a plane. The ResourceTypes and the declared services
/// are the contracts' own, and the family crates' own `ServiceDecl` constants
/// are compared against the declared services below - that comparison, not
/// this table, is what holds the two halves together.
static DRIVERS: LazyLock<[DriverDescriptor; 2]> = LazyLock::new(|| {
    static SERVICES: [ServiceDecl; 2] = [HOST_SERVICE, USER_SERVICE];
    [
        DriverDescriptor {
            resource_type: WellKnownType::HOST,
            allowed_sources: AllowedSources::BUILTIN
                | AllowedSources::STARTUP,
            verbs: CONVERTED_TYPE_VERBS,
            execution: &["host"],
            exportable: false,
            reads: &[],
            operations: &[],
            creations: &[],
            startup: &[],
            services: &SERVICES[0..1],
            decoder: Arc::new(NoDecoder),
            factory: Arc::new(NoFactory),
        },
        DriverDescriptor {
            resource_type: WellKnownType::USER,
            allowed_sources: AllowedSources::BUILTIN
                | AllowedSources::STARTUP,
            verbs: CONVERTED_TYPE_VERBS,
            execution: &["host"],
            exportable: false,
            reads: &[],
            operations: &[],
            creations: &[],
            startup: &[],
            services: &SERVICES[1..2],
            decoder: Arc::new(NoDecoder),
            factory: Arc::new(NoFactory),
        },
    ]
});

/// The two zone-plane service declarations, one per family, each carrying the
/// identity the family crate's own `ServiceDecl` publishes.
static HOST_SERVICE: ServiceDecl = ServiceDecl {
    id: d2b_provider_host::HOST_EFFECTS_SERVICE.id,
    methods: &[ServiceMethod::zone_plane("inspect-host")],
    attach_kinds: &[],
    streams: &[],
    endpoint_policy: None,
};

static USER_SERVICE: ServiceDecl = ServiceDecl {
    id: d2b_provider_user::USER_EFFECTS_SERVICE.id,
    methods: &[ServiceMethod::zone_plane("inspect-user")],
    attach_kinds: &[],
    streams: &[],
    endpoint_policy: None,
};

/// A descriptor realizes nothing: the declaration contract is decided before
/// and around the row, not by the driver, so a driver that holds no privilege
/// is exactly what keeps this fixture honest.
struct NoDecoder;

impl d2b_resource_runtime::context::SpecDecoder for NoDecoder {
    fn decode(
        &self,
        _envelope: &[u8],
    ) -> Result<Box<dyn std::any::Any + Send>, Box<dyn std::error::Error + Send + Sync>> {
        Ok(Box::new(()))
    }
}

struct NoFactory;

#[async_trait::async_trait]
impl d2b_resource_runtime::driver::ResourceDriverFactory for NoFactory {
    fn resource_types(&self) -> &[d2b_resource_runtime::identity::ResourceTypeName] {
        &[]
    }

    async fn create(
        &self,
        _key: &d2b_resource_runtime::identity::ResourceKey,
    ) -> Box<dyn d2b_resource_runtime::driver::DynResourceDriver> {
        unreachable!("the declaration contract never creates a driver")
    }
}

fn declared_with_bindings() -> UnifiedProviderDeclaration {
    UnifiedProviderDeclaration::new(
        system_core_declaration(),
        ProviderImplementationBindings::new(&DRIVERS[..]),
    )
}

/// The one implementation identity a declared method resolves to, as the
/// three parts the session layer addresses it by.
fn implementation_parts(
    implementation: &OperationImplementation,
) -> (String, String, String) {
    match implementation {
        OperationImplementation::ProviderMethod {
            provider,
            component,
            method,
        } => (
            provider.to_canonical_string(),
            component.as_str().to_owned(),
            method.as_str().to_owned(),
        ),
        OperationImplementation::TrustedExecutableTemplate { .. } => {
            panic!("a declared method resolves to a provider method, not a template")
        }
    }
}

// ---------------------------------------------------------------------------
// Scenario 1: the initial verified graph identifies the declared provider
// and its execution targets
// ---------------------------------------------------------------------------

/// The two halves agree, and what they agree on is the identity the committed
/// rows already pin: one `Provider/system-core` owning exactly `Host` and
/// `User`, with `Provider/host` and `Provider/user` naming nothing.
#[test]
fn the_verified_graph_identifies_the_declared_provider_and_its_execution_targets() {
    let declaration = declared_with_bindings();
    let identities = declaration
        .identities()
        .expect("the declaration projects its identities");
    assert_eq!(
        declaration.provider().to_canonical_string(),
        PROVIDER_REF,
        "the declared identity is the one the Host and User rows pin"
    );
    let owned: BTreeSet<&str> = identities
        .resource_types
        .iter()
        .map(String::as_str)
        .collect();
    assert_eq!(
        owned,
        OWNED_RESOURCE_TYPES.iter().copied().collect::<BTreeSet<&str>>(),
        "the declaration owns exactly what ADR-0046 gives this Provider"
    );

    // The four identities the deployment must be able to place: two
    // reconcilers and the effects service each answers through.
    let components: Vec<&str> = declaration
        .spec()
        .components()
        .iter()
        .map(|component| component.component().component_id().as_str())
        .collect();
    assert_eq!(
        components,
        [
            "host-controller",
            "host-effects",
            "user-controller",
            "user-effects"
        ]
    );

    // Every one of them is placed at the Host execution target, and each
    // names the signed Host target artifact - so an execution target cannot
    // be introduced by the declaration alone.
    for component in declaration.spec().components() {
        assert_eq!(
            component.placement().targets(),
            &BTreeSet::from([d2b_contracts_provider::v3::ControllerTargetKind::Host]),
            "{} runs at the Host execution target",
            component.component().component_id().as_str()
        );
        assert!(
            component
                .component()
                .target_capabilities()
                .iter()
                .any(|capability| {
                    capability.target_kind()
                        == d2b_contracts_provider::v3::ControllerTargetKind::Host
                }),
            "{} names the signed Host target artifact",
            component.component().component_id().as_str()
        );
    }

    // The services the session layer routes are the family crates' own
    // constants, so the plane's generated registration and this declaration
    // cannot name different providers for one service.
    let services: BTreeSet<&str> = identities
        .services
        .iter()
        .map(|service| service.as_str())
        .collect();
    assert_eq!(
        services,
        BTreeSet::from([
            d2b_provider_host::HOST_EFFECTS_SERVICE.id,
            d2b_provider_user::USER_EFFECTS_SERVICE.id,
        ])
    );
    assert_eq!(CORE_HOST_SERVICE.id, d2b_provider_host::HOST_EFFECTS_SERVICE.id);
    assert_eq!(CORE_USER_SERVICE.id, d2b_provider_user::USER_EFFECTS_SERVICE.id);
}

/// The methods the verified graph publishes resolve to the declaring
/// provider's own identity, and they are exactly the two the bound services
/// answer - no more, so a session cannot address a method no code serves.
#[test]
fn the_declared_methods_resolve_to_the_declaring_provider_only() {
    let declaration = declared_with_bindings();
    let identities = declaration
        .identities()
        .expect("the declaration projects its identities");
    let published: Vec<(String, String, String)> = identities
        .operations
        .iter()
        .map(implementation_parts)
        .collect();
    assert_eq!(
        published,
        vec![
            (PROVIDER_REF.to_owned(), "host-effects".to_owned(), "inspect-host".to_owned()),
            (PROVIDER_REF.to_owned(), "user-effects".to_owned(), "inspect-user".to_owned()),
        ],
        "every implementation identity names the declaring provider"
    );
}

// ---------------------------------------------------------------------------
// Scenario 2: untrusted artifact selection cannot add implementation
// authority (AE14)
// ---------------------------------------------------------------------------

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

fn api_binding(name: &str) -> ResourceApiBinding {
    ResourceApiBinding::new_with_placement(
        resource_type(name),
        SchemaVersion::new(1, 0).expect("spec version"),
        fingerprint('2'),
        SchemaVersion::new(1, 0).expect("status version"),
        fingerprint('3'),
        StandardCapabilityMatrix::new(Vec::new()).expect("capability matrix"),
        None,
        None,
        PlacementAnchor::ExecutionRef,
    )
    .expect("resource api binding")
}

/// The signed manifest a verified deployment admits for this artifact: the
/// declaration's own components, so the admitted method set is the declared
/// one by construction.
fn manifest() -> ProviderManifest {
    let spec = system_core_declaration();
    ProviderManifest::new(
        artifact_id("system-core"),
        digests(),
        trusted(),
        compatibility(),
        spec.components()
            .iter()
            .map(|component| component.component().clone()),
        [api_binding("Host"), api_binding("User")],
        [],
        upgrade_policy(),
    )
    .expect("provider manifest")
    .with_target_runtime_artifacts([TargetRuntimeArtifacts::new(
        d2b_contracts_provider::v3::ControllerTargetKind::Host,
        digest(DIGEST_A),
        digest(DIGEST_A),
    )
    .expect("host runtime artifacts")])
    .expect("runtime artifacts")
}

fn admitted_artifact() -> AdmittedProviderArtifact {
    AdmittedProviderArtifact::new(manifest()).expect("the deployment admitted the artifact")
}

fn row(name: &str) -> ProviderSpec {
    ProviderSpec::minimal(artifact_id(name))
}

/// A `Provider` row selects an artifact id and nothing else. An id no
/// verified deployment admitted resolves to no implementation at all, and
/// the row's own configuration cannot redirect the identity it gets.
#[test]
fn an_untrusted_artifact_selection_cannot_add_implementation_authority() {
    let declaration = declared_with_bindings();
    let admitted = admitted_artifact();

    // The admitted selection resolves, and resolves to the declaring
    // provider's own method - never to code the row introduced.
    let resolved = declaration
        .resolve_implementation(
            std::slice::from_ref(&admitted),
            &row("system-core"),
            &token("host-effects"),
            &token("inspect-host"),
        )
        .expect("the admitted artifact serves its declared method");
    assert_eq!(
        implementation_parts(&resolved),
        (
            PROVIDER_REF.to_owned(),
            "host-effects".to_owned(),
            "inspect-host".to_owned()
        )
    );

    // An artifact id the verified deployment admitted no manifest for: the
    // row cannot mint a compiled privileged handler by naming one. This is
    // where `Provider/host` and `Provider/user` land.
    for unadmitted in ["host", "user", "system-core-forged"] {
        let error = declaration
            .resolve_implementation(
                std::slice::from_ref(&admitted),
                &row(unadmitted),
                &token("host-effects"),
                &token("inspect-host"),
            )
            .expect_err("an unadmitted artifact selects nothing");
        assert_eq!(
            error,
            ProviderDeclarationError::ArtifactSelectionUnverified,
            "artifact {unadmitted} is refused by name"
        );
    }

    // An admitted artifact that declares no such method cannot serve it
    // either: the manifest is the authority, not the row.
    let error = declaration
        .resolve_implementation(
            std::slice::from_ref(&admitted),
            &row("system-core"),
            &token("host-effects"),
            &token("inspect-guest"),
        )
        .expect_err("an undeclared method has no implementation");
    assert_eq!(
        error,
        ProviderDeclarationError::ImplementationNotAdmitted {
            component: "host-effects".to_owned(),
            method: "inspect-guest".to_owned(),
        }
    );

    // And the row's configuration is not a lever: the implementation
    // identity is derived from the declaration, so an arbitrary config
    // object changes nothing about which code answers.
    let reconfigured = ProviderSpec::new(
        artifact_id("system-core"),
        CanonicalJsonObject::parse(br#"{"hostPath":"/etc/shadow","exec":"anything"}"#)
            .expect("canonical config object"),
    );
    let with_config = declaration
        .resolve_implementation(
            std::slice::from_ref(&admitted),
            &reconfigured,
            &token("host-effects"),
            &token("inspect-host"),
        )
        .expect("the identity does not depend on the row's configuration");
    assert_eq!(with_config, resolved);
}

// ---------------------------------------------------------------------------
// Scenario 3: a wrong identity or missing execution-parent support refuses a
// child request
// ---------------------------------------------------------------------------

fn ceiling(entries: &[(BindingKind, &[RequestedRights])]) -> ChildSupportCeiling {
    ChildSupportCeiling::new(
        entries
            .iter()
            .map(|(kind, rights)| {
                BindingSupportEntry::new(*kind, rights.to_vec()).expect("support entry")
            })
            .collect(),
    )
    .expect("child support ceiling")
}

/// The child's own request: it names its own source and right, so a parent's
/// default can fill nothing it already declared.
fn child_request() -> ChildBindingRequest {
    ChildBindingRequest::new(reference("Process/worker"), BindingKind::Volume)
        .expect("the child's own request")
        // `Mutate` differs from the right a defaulted request would take, so
        // a default that overwrote it would be visible in the assertion.
        .declaring(reference("Volume/data"), None, RequestedRights::Mutate)
        .expect("the declared request")
}

fn defaulted_source() -> DefaultedSource {
    DefaultedSource::new(BindingKind::Volume, reference("Volume/defaults"), None)
        .expect("the parent's default source")
}

/// A parent's defaults shape exactly the child they name. Applied to another
/// child's request they are refused outright, so a parent cannot widen a
/// sibling's request by naming a different consumer.
#[test]
fn a_parents_defaults_refuse_to_shape_another_childs_request() {
    let request = child_request();
    let other = ChildRequestDefaults::new(reference("Process/sibling"), defaulted_source())
        .expect("defaults naming another child");
    let refused = request.apply_defaults(&other);
    assert_eq!(
        refused.err(),
        Some(BindingContractError::WrongConsumer),
        "defaults belong to exactly one child"
    );

    // The same defaults applied to the child they name do shape it, which is
    // what makes the refusal about the consumer and nothing else.
    let own = ChildRequestDefaults::new(reference("Process/worker"), defaulted_source())
        .expect("defaults naming this child");
    let shaped = request
        .apply_defaults(&own)
        .expect("the named child's request is shaped");
    assert_eq!(
        shaped
            .source_ref()
            .map(ResourceRef::to_canonical_string),
        Some("Volume/data".to_owned()),
        "a default never overrides what the child declared"
    );
    assert_eq!(shaped.rights(), Some(RequestedRights::Mutate));
}

/// Only a support ceiling admits a child's binding request. A parent's own
/// consumption and a parent's defaults are inputs to someone else's decision
/// and grant nothing by themselves, so a child reaching for a kind the parent
/// never declared support for is refused by name.
#[test]
fn missing_execution_parent_support_refuses_a_child_request() {
    let missing = BindingRefusal::new(
        AdmissionStage::Authorize,
        RefusalReason::TargetSupportMissing,
    );
    let supported: ExecutionParentInput<ChildBindingRequest> =
        ExecutionParentInput::ChildSupportCeiling(ceiling(&[(
            BindingKind::Volume,
            &[RequestedRights::Observe],
        )]));
    assert_eq!(
        supported.admits_child_request(BindingKind::Volume, RequestedRights::Observe),
        Ok(()),
        "a ceiling admitting this kind and right admits the child request"
    );
    assert_eq!(
        supported.admits_child_request(BindingKind::Device, RequestedRights::Observe),
        Err(missing),
        "a kind the ceiling does not cover is refused, never silently admitted"
    );
    assert_eq!(
        supported.admits_child_request(BindingKind::Volume, RequestedRights::Exclusive),
        Err(missing),
        "a right the ceiling does not cover is refused"
    );

    // A parent's own use of a binding is not a support ceiling for its
    // children: it is a different relationship entirely.
    let parent_use = ExecutionParentInput::<ChildBindingRequest>::ParentUse(child_request());
    assert_eq!(
        parent_use.admits_child_request(BindingKind::Volume, RequestedRights::Observe),
        Err(missing),
        "a parent's own consumption grants a child nothing"
    );

    // A parent's defaults shape one child's request; they are not a ceiling.
    let defaults = ExecutionParentInput::<ChildBindingRequest>::ChildRequestDefaults(
        ChildRequestDefaults::new(reference("Process/worker"), defaulted_source())
            .expect("the parent's defaults"),
    );
    assert_eq!(
        defaults.admits_child_request(BindingKind::Volume, RequestedRights::Observe),
        Err(missing),
        "a default shapes one request; it does not admit one"
    );
    assert!(
        !defaults.yields_binding(),
        "a default produces no relationship of its own"
    );
    assert!(parent_use.yields_binding());
}

/// A realization's declared support is not permission. The facet set a
/// provider implementation declares says what it can enforce; the accepted
/// graph's source decision is what admits a relationship, and the initial
/// verified graph carries none.
#[test]
fn declared_realization_support_is_not_admission() {
    let support =
        BindingRealizationSupport::new(vec![BindingRealizationFacet::FilesystemPresentation])
            .expect("the realization support set validates");
    assert!(support.realizes(BindingRealizationFacet::FilesystemPresentation));
    assert!(!support.realizes(BindingRealizationFacet::CredentialDelivery));

    let ceiling = ceiling(&[(
        BindingKind::Volume,
        &[RequestedRights::Observe],
    )]);
    assert!(ceiling.admits(BindingKind::Volume, RequestedRights::Observe));
    assert!(
        !ceiling.admits(BindingKind::Credential, RequestedRights::Observe),
        "a ceiling covers exactly the kinds it declares"
    );
}

// ---------------------------------------------------------------------------
// The projection: what the verified deployment admits from these declarations
// ---------------------------------------------------------------------------

/// The projection consumes the declaration and the build's own output, and
/// refuses by name when the two disagree. A placeholder configuration digest
/// therefore cannot pass as a verified build identity, which is what keeps a
/// declared component from being admitted on the strength of its own bytes.
#[test]
fn the_projection_refuses_a_declaration_no_build_output_admits() {
    let spec = system_core_declaration();
    let built = BuiltArtifact::new(
        artifact_id("system-core"),
        GRAPH_PROJECTION_CONTRACT_VERSION,
        digest(DIGEST_A),
        digest(DIGEST_A),
        br#"{"type":"object"}"#.to_vec(),
    );
    let error = project_provider_graph(&[&spec], &[built], &[])
        .expect_err("a placeholder digest is not a build identity");
    assert!(
        matches!(
            error,
            GraphProjectionError::ExecutableDigestMismatch { .. }
                | GraphProjectionError::SchemaMalformed { .. }
        ),
        "the projection refuses by name: {error:?}"
    );

    // A build output for no declared artifact at all is refused too: the
    // declaration is the only thing that may introduce an artifact id.
    let orphan = BuiltArtifact::new(
        artifact_id("system-core-forged"),
        GRAPH_PROJECTION_CONTRACT_VERSION,
        digest(DIGEST_A),
        digest(DIGEST_A),
        br#"{"type":"object"}"#.to_vec(),
    );
    let error = project_provider_graph(&[&spec], &[orphan], &[])
        .expect_err("a build output admits no artifact the tree did not declare");
    assert_eq!(
        error,
        GraphProjectionError::BuildOutputMissing {
            artifact_id: "system-core".to_owned(),
        }
    );
    let _ = zone();
    let _ = store();
}
