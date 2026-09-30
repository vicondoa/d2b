//! The declaration-derived graph projection (KTD1, U4).
//!
//! Packaging, the registration table, the service catalog, the compiler's
//! private plan, and the Nix projection used to be five authored views of one
//! provider, and nothing checked that they agreed. This suite pins the
//! replacement: one declaration, one build, and the canonical consumer
//! requests a configuration author's shorthand compiles into.
//!
//! The four plan scenarios are the four boundaries:
//!
//! 1. Equivalent declaration and configuration produce byte-stable manifest
//!    inputs and graph rows.
//! 2. A mismatched executable digest, a malformed schema, a duplicate
//!    consumer slot, and a retired contract version are each refused as the
//!    specific failure they are.
//! 3. One changed provider method moves every generated surface at once, with
//!    no handwritten shared list to update.
//! 4. The signed presentation capability survives declaration, package
//!    manifest, compiler graph row, and private plan - and nothing infers it
//!    from a role name, a seccomp label, or a serving-worker role.
//!
//! Every rendered artifact is written into an isolated output directory the
//! test target declares, never into a committed generated file.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};

use d2b_contracts_provider::v3::{
    ArtifactDigest, BinaryRef, ComponentDescriptor, ComponentExecution, ComponentTargetCapability,
    ComponentType, ControllerInstanceScope, ControllerTargetKind, DeclaredComponent,
    DeclaredMethod, DeclaredPlacement, DeclaredService, EffectPortClass, PresentationCapability,
    ProviderDeclarationSpec, RequiredResourceCapability, SetupRestriction,
};
use d2b_contracts_resource::v3::{
    ArtifactId, BindingKind, BindingSlot, ResourceRef, ResourceTypeName, ResourceUid,
    VolumeBindingRequest, VolumePresentation, ZoneId, canonical_json_bytes,
    execution_policy::{BoundedToken, ExecutionDomain},
    resource_schema::PlacementAnchor,
    volume::AttachmentAccess,
};
use d2b_contracts_provider::v3::projection::{
    BindingSlotDecisionWire, BuiltArtifact, ConsumerRequestInput, GRAPH_PROJECTION_CONTRACT_VERSION,
    GRAPH_PROJECTION_INPUTS, GraphProjectionError, PrivatePlanProjection, project_provider_graph,
};
use d2b_resource_compiler::{build_artifact, executable_set_digest, sha256_digest};

const ARTIFACT: &str = "provider-volume-virtiofs";
const CONTROLLER: &str = "volume-binding";
const SERVICE: &str = "volume-virtiofs";
const SERVICE_ID: &str = "volume-virtiofs.d2bus.org/export";
const EXPORT_METHOD: &str = "export";
const CLOSE_METHOD: &str = "close";
const REOPEN_METHOD: &str = "reopen";
const DIGEST_B: &str = "sha256:0000000000000000000000000000000000000000000000000000000000000002";
const ZONE: &str = "alpha";
const VOLUME_UID: &str = "018f0000-0000-4000-8000-000000000001";
const PROCESS_UID: &str = "018f0000-0000-4000-8000-000000000002";
const GUEST_UID: &str = "018f0000-0000-4000-8000-000000000003";

/// The canonical root configuration schema every fixture component digests.
const CONFIG_SCHEMA: &[u8] = br#"{"type":"object"}"#;

/// Serializes the one test that moves the process working directory.
static WORKING_DIRECTORY: Mutex<()> = Mutex::new(());

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

fn artifact_id() -> ArtifactId {
    ArtifactId::parse(ARTIFACT).expect("artifact identifier")
}

fn config_digest() -> ArtifactDigest {
    sha256_digest(CONFIG_SCHEMA)
}

fn zone() -> ZoneId {
    ZoneId::parse(ZONE).expect("zone identifier")
}

fn uid(value: &str) -> ResourceUid {
    ResourceUid::parse(value).expect("store-assigned identity")
}

fn host_target_capability() -> ComponentTargetCapability {
    ComponentTargetCapability::new(
        ControllerTargetKind::Host,
        digest(DIGEST_B),
        [EffectPortClass::Volume],
    )
    .expect("host target capability")
}

fn build_output(executable_bytes: &[u8]) -> BuiltArtifact {
    let mut executables = BTreeMap::new();
    executables.insert(SERVICE.to_owned(), sha256_digest(executable_bytes));
    let declared = executable_set_digest(&executables).expect("canonical executable set digest");
    build_artifact(
        artifact_id(),
        GRAPH_PROJECTION_CONTRACT_VERSION,
        executables,
        declared,
        CONFIG_SCHEMA.to_vec(),
    )
    .expect("the fixture build output is canonical")
}

/// The build's one executable, with the bytes the fixture package shipped.
fn built_artifact() -> BuiltArtifact {
    build_output(b"volume-virtiofs")
}

/// The same build, re-pointed at a substituted executable. The signed manifest
/// still pins the digest of the bytes the package originally shipped, so the
/// build's output and the manifest's pin disagree.
fn substituted_build() -> BuiltArtifact {
    let mut executables = BTreeMap::new();
    executables.insert(SERVICE.to_owned(), sha256_digest(b"substituted-binary"));
    let substituted = executable_set_digest(&executables).expect("canonical set digest");
    BuiltArtifact::new(
        artifact_id(),
        GRAPH_PROJECTION_CONTRACT_VERSION,
        substituted,
        built_artifact().executable_set_digest().clone(),
        CONFIG_SCHEMA.to_vec(),
    )
}

/// One build whose signed manifest pins the same set, with a different
/// configuration schema installed.
fn build_with_schema(schema: &[u8]) -> BuiltArtifact {
    let honest = built_artifact();
    BuiltArtifact::new(
        artifact_id(),
        GRAPH_PROJECTION_CONTRACT_VERSION,
        honest.executable_set_digest().clone(),
        honest.declared_executable_set_digest().clone(),
        schema.to_vec(),
    )
}

/// The same build under a retired contract version.
fn build_with_version(version: &str) -> BuiltArtifact {
    let honest = built_artifact();
    BuiltArtifact::new(
        artifact_id(),
        version,
        honest.executable_set_digest().clone(),
        honest.declared_executable_set_digest().clone(),
        CONFIG_SCHEMA.to_vec(),
    )
}

fn controller_descriptor() -> ComponentDescriptor {
    ComponentDescriptor::new(
        token(CONTROLLER),
        ComponentType::Controller,
        [resource_type("VolumeBinding")],
        [],
        [ExecutionDomain::System],
        1,
        config_digest(),
        [],
    )
    .expect("controller descriptor")
    .with_execution(ComponentExecution::Launchable {
        binary_ref: BinaryRef::parse(CONTROLLER).expect("binary reference"),
    })
    .with_controller_placement(
        ControllerInstanceScope::ZoneSingleton,
        [ControllerTargetKind::Zone],
    )
    .expect("zone singleton placement")
    .with_target_capabilities([ComponentTargetCapability::new(
        ControllerTargetKind::Zone,
        digest(DIGEST_B),
        [],
    )
    .expect("zone target capability")])
    .expect("zone target capabilities")
}

/// The service component's signed code identity.
///
/// Its identity and its binary reference both read as though they mounted a
/// host path: `volume-virtiofs` is the classic mount-helper spelling and
/// `virtiofsd-mount-helper` is the classic privileged helper name. Neither is
/// a capability, and the projection must report the declared one.
fn service_descriptor_with(methods: &[&str]) -> ComponentDescriptor {
    ComponentDescriptor::new(
        token(SERVICE),
        ComponentType::Service,
        [],
        methods.iter().map(|method| token(method)),
        [ExecutionDomain::System],
        1,
        config_digest(),
        [],
    )
    .expect("service descriptor")
    .with_execution(ComponentExecution::Launchable {
        binary_ref: BinaryRef::parse("virtiofsd-mount-helper").expect("binary reference"),
    })
    .with_target_capabilities([host_target_capability()])
    .expect("host target capabilities")
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

fn service_component(
    presentation: PresentationCapability,
    methods: &[&str],
) -> DeclaredComponent {
    DeclaredComponent::new(
        service_descriptor_with(methods),
        DeclaredPlacement::new([ControllerTargetKind::Host], None).expect("host placement"),
        presentation,
        SetupRestriction::required_for(presentation).iter().copied(),
        methods
            .iter()
            .map(|method| DeclaredMethod::new(token(method), None, presentation))
            .collect(),
        vec![DeclaredService::new(
            SERVICE_ID,
            methods.iter().map(|method| token(method)),
        )
        .expect("declared service")],
        [],
    )
    .expect("declared service component")
}

fn spec(components: Vec<DeclaredComponent>) -> ProviderDeclarationSpec {
    ProviderDeclarationSpec::new(artifact_id(), components, []).expect("declaration spec")
}

/// The fixture declaration: a Zone-singleton `VolumeBinding` controller and a
/// namespace-first volume service.
fn fixture_spec() -> ProviderDeclarationSpec {
    spec(vec![
        controller_component(),
        service_component(PresentationCapability::NamespaceFirstServiceSource, BASE_METHODS),
    ])
}

const BASE_METHODS: &[&str] = &[EXPORT_METHOD, CLOSE_METHOD];
const EXTENDED_METHODS: &[&str] = &[EXPORT_METHOD, CLOSE_METHOD, REOPEN_METHOD];

fn project(
    declaration: &ProviderDeclarationSpec,
    built: &[BuiltArtifact],
    requests: &[ConsumerRequestInput],
) -> Result<PrivatePlanProjection, GraphProjectionError> {
    project_provider_graph(&[declaration], built, requests)
}

/// One canonical consumer request: a view of the `data` Volume presented at a
/// destination inside the named consumer, under a stable local slot.
fn request(
    consumer_ref: &str,
    consumer_uid: &str,
    slot: &str,
    access: AttachmentAccess,
    destination: &str,
) -> ConsumerRequestInput {
    let request = VolumeBindingRequest::new(
        ResourceRef::parse("Volume/data").expect("volume reference"),
        ResourceRef::parse(consumer_ref).expect("consumer reference"),
        BindingSlot::parse(slot).expect("stable consumer slot"),
        token("root"),
        access,
        VolumePresentation::filesystem(destination).expect("filesystem presentation"),
    )
    .expect("canonical consumer request");
    ConsumerRequestInput::new(zone(), uid(VOLUME_UID), uid(consumer_uid), request)
}

fn web_request(slot: &str, access: AttachmentAccess, destination: &str) -> ConsumerRequestInput {
    request("Process/web", PROCESS_UID, slot, access, destination)
}

/// Every rendered artifact lands in one directory the test target owns.
///
/// Under Bazel the render lands in the test sandbox's scratch directory; a
/// plain `cargo test` run gets a fresh temporary tree. Either way the
/// projection writes outside the source tree, so a generation action can
/// never overwrite a committed generated artifact.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
fn output_dir() -> PathBuf {
    let base = std::env::var_os("TEST_TMPDIR")
        .or_else(|| std::env::var_os("CARGO_TARGET_TMPDIR"))
        .map_or_else(
            || {
                std::env::temp_dir().join(format!(
                    "d2b-provider-graph-projection-{}",
                    std::process::id()
                ))
            },
            |scratch| PathBuf::from(scratch).join("provider-graph-projection"),
        );
    fs::create_dir_all(&base).expect("isolated projection output directory");
    base
}

/// Write one rendered artifact and return the bytes that landed.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
fn write_artifact(dir: &Path, name: &str, bytes: &[u8]) {
    let path = dir.join(name);
    fs::create_dir_all(path.parent().expect("artifact parent")).expect("artifact directory");
    fs::write(&path, bytes).expect("rendered artifact write");
    assert_eq!(
        fs::read(&path).expect("rendered artifact read"),
        bytes,
        "the rendered artifact on disk must be the bytes the generator produced"
    );
}

// ---------------------------------------------------------------------------
// Scenario 1: equivalent declaration and configuration are byte-stable
// ---------------------------------------------------------------------------

#[test]
fn equivalent_declaration_produces_byte_stable_manifest_inputs_and_graph_rows() {
    let declaration = fixture_spec();
    let built = built_artifact();

    let first = project(&declaration, std::slice::from_ref(&built), &[]).expect("first projection");
    let second = project(&fixture_spec(), std::slice::from_ref(&built), &[]).expect("second projection");

    assert_eq!(
        first.canonical_bytes().expect("first plan bytes"),
        second.canonical_bytes().expect("second plan bytes"),
        "one declaration must render one byte-stable private plan"
    );
    assert_eq!(
        first.digest().expect("first plan digest"),
        second.digest().expect("second digest")
    );
    assert_eq!(first.operations(), second.operations(), "graph rows are stable");
    assert_eq!(first.manifest_inputs(), second.manifest_inputs());

    // The manifest input carries exactly the canonical projection U3's
    // `emit_declaration_canonical` emits, so the bytes a provider signs and
    // the bytes a generator consumes cannot differ.
    let input = &first.manifest_inputs()[0];
    assert_eq!(
        input.declaration_bytes(),
        canonical_json_bytes(&declaration)
            .expect("canonical declaration bytes")
            .as_slice(),
        "the manifest input is the declaration's own canonical projection"
    );
    assert_eq!(input.artifact_id(), ARTIFACT);
    assert_eq!(input.provider_ref(), format!("Provider/{ARTIFACT}"));
    assert_eq!(
        input.executable_set_digest(),
        built.executable_set_digest().as_str(),
        "the executable digest comes from the build output, not the declaration"
    );

    // One declaration produces the four identities from one source: the
    // controller owns the ResourceType, the service owns the methods, and the
    // catalog routes the one declared service.
    let methods: Vec<&str> = first.operations().iter().map(|row| row.method()).collect();
    assert_eq!(methods, vec![CLOSE_METHOD, EXPORT_METHOD], "sorted declared methods");
    assert!(
        first
            .operations()
            .iter()
            .all(|row| row.resource_types().is_empty()),
        "a service exports no ResourceType, so no method claims one"
    );
    assert_eq!(first.services().len(), 1, "one declared service, one catalog row");
    assert_eq!(first.services()[0].service_id(), SERVICE_ID);
    assert_eq!(first.services()[0].provider_ref(), format!("Provider/{ARTIFACT}"));
    assert_eq!(first.services()[0].methods(), BASE_METHODS);
    assert_eq!(first.registrations().len(), 1, "one declaration, one registration row");
    assert_eq!(first.registrations()[0].artifact_id(), ARTIFACT);
    assert_eq!(first.registrations()[0].services(), &[SERVICE_ID.to_owned()]);
}

#[test]
fn equivalent_configuration_coalesces_one_consumer_slot() {
    let declaration = fixture_spec();
    let built = built_artifact();
    let first = web_request("state", AttachmentAccess::ReadWrite, "/state");
    let equivalent = web_request("state", AttachmentAccess::ReadWrite, "/state");

    let plan = project(&declaration, &[built], &[first, equivalent])
        .expect("two equivalent requests for one slot");

    assert_eq!(plan.consumer_requests().len(), 2, "both requests are projected");
    let (first_row, second_row) = (&plan.consumer_requests()[0], &plan.consumer_requests()[1]);
    assert_eq!(
        first_row.fingerprint(),
        second_row.fingerprint(),
        "equivalent configuration digests the same desired bytes"
    );
    assert_eq!(
        first_row.slot(),
        second_row.slot(),
        "both occupy the one stable slot"
    );
    assert_eq!(first_row.slot(), "state");
    assert_eq!(first_row.consumer_ref(), "Process/web");
    assert_eq!(
        (first_row.kind(), first_row.required_facets()),
        (second_row.kind(), second_row.required_facets()),
        "the two rows differ only in how the slot index resolved them"
    );
    assert_eq!(first_row.decision(), BindingSlotDecisionWire::Claimed);
    assert_eq!(
        second_row.decision(),
        BindingSlotDecisionWire::Coalesced,
        "the second identical declaration coalesces onto the first"
    );
}

// ---------------------------------------------------------------------------
// Scenario 2: the four specific refusals
// ---------------------------------------------------------------------------

#[test]
fn a_mismatched_executable_digest_is_refused() {
    let error = project(&fixture_spec(), &[substituted_build()], &[])
        .expect_err("a substituted binary is refused");
    assert_eq!(
        error.code(),
        "provider-graph-executable-digest-mismatch",
        "the build's bytes must match the digest the manifest pins: {error}"
    );
    assert!(
        matches!(error, GraphProjectionError::ExecutableDigestMismatch { .. }),
        "the refusal names the executable digest specifically"
    );
}

#[test]
fn a_malformed_schema_is_refused() {
    let honest = built_artifact();
    // Unsorted keys: valid JSON, not the canonical bytes a `configDigest` can
    // name.
    let _ = honest;
    let non_canonical = build_with_schema(br#"{"type":"object","title":"keys out of order"}"#);
    let error = project(&fixture_spec(), &[non_canonical], &[])
        .expect_err("a non-canonical schema is refused");
    assert_eq!(
        error.code(),
        "provider-graph-schema-malformed",
        "the installed schema must be the canonical schema the manifest digests: {error}"
    );
    assert!(
        matches!(
            error,
            GraphProjectionError::SchemaMalformed {
                reason: "not-canonical",
                ..
            }
        ),
        "the refusal names canonicality specifically: {error:?}"
    );

    let unparseable = build_with_schema(b"{\"type\":");
    assert!(
        matches!(
            project(&fixture_spec(), &[unparseable], &[]),
            Err(GraphProjectionError::SchemaMalformed {
                reason: "not-valid-canonical-json",
                ..
            })
        ),
        "a truncated schema is refused as a schema defect, not parsed"
    );
}

#[test]
fn a_duplicate_consumer_slot_is_refused() {
    let first = web_request("state", AttachmentAccess::ReadWrite, "/state");
    let conflicting = web_request("state", AttachmentAccess::ReadOnly, "/srv/state");

    let error = project(&fixture_spec(), &[built_artifact()], &[first, conflicting])
        .expect_err("two different declarations for one slot are refused");
    assert_eq!(
        error.code(),
        "provider-graph-consumer-slot-duplicate",
        "one slot cannot carry two relationships: {error}"
    );
    assert!(
        matches!(
            error,
            GraphProjectionError::ConsumerSlotDuplicate {
                kind: "volume",
                ref slot,
                ..
            } if slot == "state"
        ),
        "the refusal names the consumer slot specifically: {error:?}"
    );
}

#[test]
fn a_retired_contract_version_is_refused() {
    let honest = built_artifact();
    let _ = honest;
    let retired = build_with_version("d2b.zone.v2");

    let error = project(&fixture_spec(), &[retired], &[])
        .expect_err("an old contract version is refused");
    assert_eq!(
        error.code(),
        "provider-graph-contract-version-retired",
        "the release has no compatibility parser for an old version: {error}"
    );
    assert!(
        matches!(error, GraphProjectionError::ContractVersionRetired { .. }),
        "the refusal names the contract version specifically"
    );
}

// ---------------------------------------------------------------------------
// Scenario 3: one changed method moves every surface
// ---------------------------------------------------------------------------

#[test]
fn one_changed_provider_method_moves_every_generated_surface() {
    let built = built_artifact();
    let before =
        project(&fixture_spec(), std::slice::from_ref(&built), &[]).expect("baseline projection");

    // The only edit is the declaration: the service answers one more method.
    let after = project(
        &spec(vec![
            controller_component(),
            service_component(
                PresentationCapability::NamespaceFirstServiceSource,
                EXTENDED_METHODS,
            ),
        ]),
        &[built],
        &[],
    )
    .expect("changed projection");

    // Surface one: the compiler's resource-and-operation graph rows.
    let methods: Vec<&str> = after.operations().iter().map(|row| row.method()).collect();
    assert_eq!(
        methods,
        vec![CLOSE_METHOD, EXPORT_METHOD, REOPEN_METHOD],
        "the added method became exactly one graph row"
    );
    assert_eq!(
        before.operations().len() + 1,
        after.operations().len(),
        "the edit added a row rather than rewriting the projection"
    );

    // Surface two: the package manifest input.
    let manifest_methods = after.manifest_inputs()[0]
        .components()
        .iter()
        .find(|component| component.component_id() == SERVICE)
        .expect("service manifest input row")
        .exported_methods()
        .to_vec();
    assert_eq!(manifest_methods, vec![CLOSE_METHOD, EXPORT_METHOD, REOPEN_METHOD]);

    // Surface three: the service catalog and the provider registration row.
    assert_eq!(
        after.services()[0].service_id(),
        SERVICE_ID,
        "the catalog still routes the one declared service"
    );
    assert_eq!(
        after.services()[0].methods(),
        EXTENDED_METHODS,
        "the catalog row's method list moved with the declaration"
    );
    assert_eq!(after.registrations()[0].artifact_id(), ARTIFACT);
    assert_eq!(after.registrations()[0].services(), &[SERVICE_ID.to_owned()]);
    assert_eq!(
        after.registrations()[0].methods(),
        ["close", "export", "reopen"],
        "the registration row's method list is the sorted declared set"
    );

    // Surface four: the private plan projection.
    assert_ne!(
        before.digest().expect("baseline digest"),
        after.digest().expect("changed digest"),
        "the private plan moved without a handwritten shared list"
    );
    assert_ne!(
        before.canonical_bytes().expect("baseline bytes"),
        after.canonical_bytes().expect("changed bytes")
    );
    assert!(
        after
            .operations()
            .iter()
            .all(|row| row.presentation_token() == "namespace-first-service-source"),
        "the projection still speaks the declared capability's wire token"
    );

    // Every rendered artifact lands in the isolated output directory, never
    // beside a committed generated file.
    let out = output_dir();
    assert!(
        !out.starts_with(std::env::current_dir().expect("current directory")),
        "a generation action must write outside the source tree: {}",
        out.display()
    );
    write_artifact(
        &out,
        "private-plan.json",
        &after.canonical_bytes().expect("plan bytes"),
    );
    assert!(out.join("private-plan.json").is_file());
}

// ---------------------------------------------------------------------------
// Scenario 4: the signed presentation capability survives every projection
// ---------------------------------------------------------------------------

#[test]
fn a_signed_presentation_capability_survives_every_projection() {
    let declaration = fixture_spec();
    let plan = project(&declaration, &[built_artifact()], &[]).expect("projection");

    // Stage one: the declaration itself carries it.
    let component = declaration
        .component(&token(SERVICE))
        .expect("declared service component");
    assert_eq!(
        component.presentation(),
        PresentationCapability::NamespaceFirstServiceSource
    );

    // Stage two: the package manifest input carries it, with the setup
    // restrictions that capability requires.
    let manifest_component = plan.manifest_inputs()[0]
        .components()
        .iter()
        .find(|row| row.component_id() == SERVICE)
        .expect("service manifest input row");
    assert_eq!(
        manifest_component.presentation_capability(),
        PresentationCapability::NamespaceFirstServiceSource,
        "the signed manifest input carries the declared capability"
    );
    assert_eq!(
        manifest_component.setup_restrictions(),
        &[
            SetupRestriction::SteadyStateMountNamespace,
            SetupRestriction::ZeroHostCapability
        ],
        "a namespace-first service source keeps ADR 0021's zero-host-capability launch"
    );

    // Stage three: the compiler's graph row carries it.
    for row in plan.operations() {
        assert_eq!(
            row.presentation_capability(),
            PresentationCapability::NamespaceFirstServiceSource,
            "every declared method needs the capability its component declares"
        );
    }

    // Stage four: the private plan projection round-trips it.
    let bytes = plan.canonical_bytes().expect("plan bytes");
    let parsed: PrivatePlanProjection =
        serde_json::from_slice(&bytes).expect("private plan round trip");
    assert_eq!(
        parsed.operations(),
        plan.operations(),
        "the capability survives the private plan projection byte-for-byte"
    );
    assert_eq!(
        parsed.manifest_inputs(),
        plan.manifest_inputs(),
        "the signed manifest inputs survive the private plan projection"
    );
    assert!(
        parsed
            .operations()
            .iter()
            .all(|row| row.presentation_token() == "namespace-first-service-source"),
        "the wire token survives the round trip too"
    );

    // The private plan has no field a role name, a seccomp label, or a
    // serving-worker role could arrive in, so there is no inference fallback
    // to fall back to.
    let wire = String::from_utf8(bytes).expect("canonical JSON is UTF-8");
    for forbidden in ["\"role\"", "seccomp", "\"label\"", "workerRole"] {
        assert!(
            !wire.contains(forbidden),
            "the private plan must carry no `{forbidden}` field to infer a capability from"
        );
    }
}

#[test]
fn a_serving_worker_name_does_not_buy_a_presentation_capability() {
    // A component whose whole vocabulary reads like a privileged mount
    // worker, declaring that it presents nothing.
    let declared = service_component(PresentationCapability::None, BASE_METHODS);
    let plan = project(
        &spec(vec![controller_component(), declared.clone()]),
        &[built_artifact()],
        &[],
    )
    .expect("projection");

    for row in plan.operations() {
        assert_eq!(
            row.presentation_capability(),
            PresentationCapability::None,
            "a `virtiofsd-mount-helper` name and a serving-worker role buy no capability: {}",
            row.method()
        );
    }
    let wire = String::from_utf8(plan.canonical_bytes().expect("plan bytes")).expect("UTF-8");
    assert!(
        !wire.contains("filesystem-presentation"),
        "no surface may report a capability the declaration did not make"
    );

    // A method that needs a presentation its component cannot realize is
    // refused by the declaration's own contract, before any projection runs.
    assert!(
        DeclaredComponent::new(
            declared.component().clone(),
            DeclaredPlacement::new([ControllerTargetKind::Host], None).expect("host placement"),
            PresentationCapability::None,
            [],
            vec![DeclaredMethod::new(
                token(EXPORT_METHOD),
                None,
                PresentationCapability::FilesystemPresentation,
            )],
            Vec::new(),
            Vec::new(),
        )
        .is_err(),
        "a None-presentation component cannot declare a filesystem-presenting method"
    );
}

// ---------------------------------------------------------------------------
// The generator reads no authored merge input
// ---------------------------------------------------------------------------

#[test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
fn the_projection_reads_no_authored_merge_input() {
    // The closed input list names the one source the generator reads. A
    // regression that reintroduced the retired broker-operation merge
    // documents, the handwritten privilege or role-scope tables, or a
    // declaration-free Nix inventory row would have to declare it here.
    assert_eq!(
        GRAPH_PROJECTION_INPUTS,
        &["provider-declaration"],
        "the projection reads the declaration and nothing else"
    );

    // The rendered bytes do not depend on the working directory, because the
    // projection takes no path and opens no file. Running it from a directory
    // that holds none of the retired inputs is indistinguishable from running
    // it beside the repository, which is what a relative read of any of those
    // documents could not be.
    let _guard = WORKING_DIRECTORY.lock().expect("working-directory guard");
    let expected = project(&fixture_spec(), &[built_artifact()], &[])
        .expect("projection")
        .canonical_bytes()
        .expect("plan bytes");
    let isolated = output_dir().join("empty-cwd");
    fs::create_dir_all(&isolated).expect("isolated working directory");
    let previous = std::env::current_dir().expect("current directory");
    std::env::set_current_dir(&isolated).expect("enter the isolated directory");
    let rerendered = project(&fixture_spec(), &[built_artifact()], &[])
        .expect("projection from an empty directory")
        .canonical_bytes()
        .expect("plan bytes");
    std::env::set_current_dir(previous).expect("restore the working directory");
    assert_eq!(
        expected, rerendered,
        "the projection is a function of its arguments, not of the files beside it"
    );

    // The declared graph rows are joined from the declaration's own exports: a
    // required resource capability rides the manifest input, and a controller
    // that declares no method projects no operation row, with no family table
    // anywhere in the projection.
    let controller_plan = project(
        &ProviderDeclarationSpec::new(
            artifact_id(),
            [controller_component()],
            [RequiredResourceCapability::new(
                resource_type("VolumeBinding"),
                token("expedited-reconcile"),
            )],
        )
        .expect("declaration spec with a required capability"),
        &[built_artifact()],
        &[],
    )
    .expect("controller projection");
    assert!(
        controller_plan.operations().is_empty(),
        "a controller that declares no method projects no operation row"
    );
    let required = &controller_plan.manifest_inputs()[0];
    assert_eq!(required.artifact_id(), ARTIFACT);
    let wire = String::from_utf8(required.declaration_bytes().to_vec()).expect("UTF-8");
    assert!(
        wire.contains("expedited-reconcile"),
        "the required resource capability is declared, not tabulated elsewhere"
    );
}

// ---------------------------------------------------------------------------
// A Guest consumer request
// ---------------------------------------------------------------------------

#[test]
fn a_guest_consumer_request_projects_its_presentation_facets() {
    let guest = request(
        "Guest/vm",
        GUEST_UID,
        "boot-disk",
        AttachmentAccess::ReadOnly,
        "/boot",
    );
    let plan = project(&fixture_spec(), &[built_artifact()], &[guest]).expect("projection");
    let row = &plan.consumer_requests()[0];
    assert_eq!(row.consumer_ref(), "Guest/vm");
    assert_eq!(row.slot(), "boot-disk");
    assert_eq!(row.fingerprint().len(), 71, "a framed canonical digest");
    assert_eq!(row.kind(), BindingKind::Volume);

    let wire = String::from_utf8(plan.canonical_bytes().expect("plan bytes")).expect("UTF-8");
    assert!(
        wire.contains("\"requiredFacets\":[\"filesystem-presentation\"]"),
        "the consumer's typed presentation decides the realization facets: {wire}"
    );
    assert!(
        wire.contains("\"sourceRef\":\"Volume/data\"") && !wire.contains("hostname"),
        "a request names its source by exact reference, never by a host path"
    );
}
