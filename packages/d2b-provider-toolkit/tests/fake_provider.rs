//! A well-behaved Provider, built end to end on the toolkit's fakes.
//!
//! This is the positive half of the `ADR046-provider-001` validation
//! obligation "fake and malicious Provider". It stands up a complete signed
//! manifest for an imaginary Provider, drives it through bootstrap, alias
//! resolution, component launch, effect release, and status write against
//! the fake core, store, bus, supervisor, and effect ports, and asserts that
//! the honest path is admitted at every step.
//!
//! `malicious_provider.rs` is the negative half: it reuses this same
//! Provider and mutates one thing at a time.

use std::collections::BTreeMap;

use d2b_contracts::v3::identity::BindingDigest;
use d2b_contracts::v3::{
    Locality, TransportBinding,
    execution_policy::{BoundedToken, ExecutionDomain},
    identity::{ResourceTypeName, SchemaFingerprint, SessionPurpose},
    provider::{
        ArtifactDigest, ArtifactDigestSet, ArtifactId, CapabilitySupport, CompatibilityRange,
        ComponentDescriptor, ComponentStateKind, ComponentStateNamespace, ComponentStateView,
        ComponentType, DependencyAlias, DependencyDeclaration, PolicyEvaluation, ProviderManifest,
        ProviderSpec, ResourceApiBinding, RevocationState, SignatureState,
        StandardCapabilityMatrix, StorageNeed, TrustEvidence, UpgradeDisposition, UpgradePolicy,
    },
    resource_ref::ResourceRef,
    resource_schema::SchemaVersion,
    volume::ViewRight,
    volume_state::{MigrationPolicy, PersistenceClass, SensitivityClass, VolumeStateSchemaId},
    zone_routing::ZonePath,
};
use d2b_provider_toolkit::fakes::{
    FakeBus, FakeCoreClient, FakeEffectPort, FakePortError, FakeResourceStore, FakeSupervisor,
    FaultPlan,
};
use d2b_provider_toolkit::{AllocatorSessionBinding, ProviderAgentBootstrap};

pub const DIGEST: &str = "sha256:0000000000000000000000000000000000000000000000000000000000000001";

pub fn fingerprint(tail: &str) -> SchemaFingerprint {
    SchemaFingerprint::parse(format!("sha256:{}{tail}", "0".repeat(63))).expect("valid digest")
}

pub fn trusted() -> TrustEvidence {
    TrustEvidence {
        publisher: BoundedToken::parse("first-party").expect("valid publisher"),
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
        support_channel: BoundedToken::parse("stable").expect("valid channel"),
    }
}

pub fn digests() -> ArtifactDigestSet {
    let digest = ArtifactDigest::parse(DIGEST).expect("valid digest");
    ArtifactDigestSet {
        package: digest.clone(),
        executable: digest.clone(),
        manifest: digest.clone(),
        config: digest.clone(),
        schema: digest.clone(),
        service: digest,
    }
}

pub fn controller() -> ComponentDescriptor {
    ComponentDescriptor::new(
        BoundedToken::parse("volume-controller").expect("valid id"),
        ComponentType::Controller,
        [ResourceTypeName::parse("Volume").expect("standard type")],
        [BoundedToken::parse("assess-update").expect("valid method")],
        [ExecutionDomain::System],
        1,
        ArtifactDigest::parse(DIGEST).expect("valid digest"),
        [DependencyDeclaration {
            alias: DependencyAlias::Volume,
            required: true,
        }],
        false,
    )
    .expect("a controller owning one ResourceType is valid")
}

fn state_namespace() -> ComponentStateNamespace {
    ComponentStateNamespace::new(
        BoundedToken::parse("main-state").expect("valid namespace id"),
        ComponentStateKind::State,
        VolumeStateSchemaId::parse("example-provider.d2bus.org/controller/main-state")
            .expect("valid state schema id"),
        SchemaVersion::new(1, 0).expect("valid version"),
        fingerprint("4"),
        PersistenceClass::Persistent,
        SensitivityClass::Private,
        MigrationPolicy::PreLaunchRequired,
        4096,
        Some(StorageNeed::Secret),
        false,
        None,
        false,
        BTreeMap::from([(
            "main".to_owned(),
            ComponentStateView::new(
                String::new(),
                vec![ViewRight::Read, ViewRight::Write, ViewRight::Traverse],
            )
            .expect("valid state view"),
        )]),
    )
    .expect("a complete persistent state namespace is valid")
}

pub fn binding() -> ResourceApiBinding {
    ResourceApiBinding::new(
        ResourceTypeName::parse("Volume").expect("standard type"),
        SchemaVersion::new(1, 0).expect("valid version"),
        fingerprint("2"),
        SchemaVersion::new(1, 0).expect("valid version"),
        fingerprint("3"),
        StandardCapabilityMatrix::new([(
            BoundedToken::parse("expedited-reconcile").expect("valid capability"),
            CapabilitySupport::Supported,
        )])
        .expect("a single-entry matrix is valid"),
        None,
        None,
    )
    .expect("a binding for an owned ResourceType is valid")
}

pub fn manifest_with(
    trust: TrustEvidence,
    components: Vec<ComponentDescriptor>,
    bindings: Vec<ResourceApiBinding>,
) -> Result<ProviderManifest, d2b_contracts::v3::provider::ProviderContractError> {
    ProviderManifest::new(
        artifact_id(),
        digests(),
        trust,
        CompatibilityRange {
            api_major: 3,
            api_minor: 4,
            descriptor_fingerprint: fingerprint("1"),
            state_schema_version: SchemaVersion::new(1, 2).expect("valid version"),
        },
        components,
        bindings,
        [],
        UpgradePolicy {
            drain_before_upgrade: true,
            max_automatic_disposition: UpgradeDisposition::InPlace,
            preserves_durable_state: true,
        },
    )
}

pub fn manifest() -> ProviderManifest {
    manifest_with(trusted(), vec![controller()], vec![binding()])
        .expect("the honest manifest is valid")
}

pub fn artifact_id() -> ArtifactId {
    ArtifactId::parse("provider-volume-local").expect("valid artifact id")
}

pub fn provider_ref() -> ResourceRef {
    ResourceRef::parse("Provider/volume-local").expect("valid provider ref")
}

pub fn transport() -> TransportBinding {
    TransportBinding::new(
        Locality::Local,
        BindingDigest::parse(DIGEST).expect("valid digest"),
    )
}

#[test]
fn an_honest_provider_runs_the_whole_admission_path() {
    // Bootstrap: the allocator names the agent, and the agent's compiled
    // expectation agrees.
    let agent = ProviderAgentBootstrap::new(
        provider_ref(),
        ZonePath::local_root(),
        SessionPurpose::parse("provider-agent").expect("valid purpose"),
    );
    let identity = agent
        .admit(AllocatorSessionBinding::new(
            ZonePath::local_root(),
            provider_ref(),
            SessionPurpose::parse("provider-agent").expect("valid purpose"),
            transport(),
        ))
        .expect("the allocator binding matches the agent");
    assert_eq!(identity.provider_ref(), &provider_ref());

    // Catalog: the resource row's artifactId selects exactly this manifest,
    // and the artifact is admissible at the exact required API.
    let manifest = manifest();
    let spec = ProviderSpec::minimal(artifact_id());
    let mut core = FakeCoreClient::with_artifact(&artifact_id(), manifest.clone());
    let resolved = core
        .resolve_artifact(spec.artifact_id())
        .expect("the artifact is in the catalog");
    assert_eq!(resolved.artifact_id(), manifest.artifact_id());
    assert!(manifest.admit(3, 4, &fingerprint("1")).is_ok());

    // Package presence is not installation: providerRef does not resolve
    // until core marks the row Ready.
    assert_eq!(
        core.resolve_provider_ref(&provider_ref()),
        Err(FakePortError::ProviderNotReady)
    );
    core.mark_ready();
    assert!(core.resolve_provider_ref(&provider_ref()).is_ok());

    // Dependencies: the component asks for its declared alias by name.
    let mut bus = FakeBus::with_bindings([(
        DependencyAlias::Volume,
        ResourceRef::parse("Provider/volume-backend").expect("valid ref"),
    )]);
    for dependency in controller().dependencies() {
        assert!(bus.resolve_alias(dependency.alias).is_ok());
    }

    // Launch: core creates each component's Process through the supervisor
    // port, and the Provider itself spawns nothing.
    let mut supervisor = FakeSupervisor::with_faults(FaultPlan::healthy());
    for component in manifest.components() {
        supervisor
            .launch(component.component_id())
            .expect("the honest component launches");
    }
    assert_eq!(supervisor.recorder().count_of("launch"), 1);

    // Effect and status: the effect is released through the injected port,
    // and status is written only for an owned ResourceType.
    let mut effects = FakeEffectPort::with_faults(FaultPlan::healthy());
    effects
        .apply(&BoundedToken::parse("provision-volume").expect("valid effect"))
        .expect("the effect port accepts the intent");
    let mut store = FakeResourceStore::owning(vec!["Volume".to_owned()]);
    assert!(
        store
            .write_status(&ResourceRef::parse("Volume/scratch").expect("valid ref"))
            .is_ok()
    );
}

#[test]
fn a_declared_state_volume_is_visible_and_a_stateless_provider_declares_none() {
    let stateful = controller()
        .with_state_namespaces([state_namespace()])
        .expect("a controller with a complete namespace is stateful");
    let manifest = manifest_with(trusted(), vec![stateful], vec![binding()])
        .expect("a stateful manifest is valid");
    assert!(manifest.declares_state_volume());

    let stateless = ComponentDescriptor::new(
        BoundedToken::parse("volume-controller").expect("valid id"),
        ComponentType::Controller,
        [ResourceTypeName::parse("Volume").expect("standard type")],
        [BoundedToken::parse("assess-update").expect("valid method")],
        [ExecutionDomain::System],
        1,
        ArtifactDigest::parse(DIGEST).expect("valid digest"),
        [],
        false,
    )
    .expect("a stateless controller is valid");
    let manifest = manifest_with(trusted(), vec![stateless], vec![binding()])
        .expect("a stateless manifest is valid");
    // An empty ProviderStateSet is a normal outcome, not a defect.
    assert!(!manifest.declares_state_volume());
}

#[test]
fn a_fault_on_one_port_does_not_release_the_effect_behind_it() {
    let mut effects = FakeEffectPort::with_faults(FaultPlan::failing_first(2));
    let effect = BoundedToken::parse("provision-volume").expect("valid effect");
    for _ in 0..2 {
        assert_eq!(effects.apply(&effect), Err(FakePortError::InjectedFault));
    }
    // Nothing was recorded while the port was failing, so a retry loop
    // cannot mistake a refused call for a released effect.
    assert!(effects.recorder().is_empty());
    assert!(effects.apply(&effect).is_ok());
    assert_eq!(effects.recorder().count_of("apply-effect"), 1);
}
