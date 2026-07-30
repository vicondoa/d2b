//! A malicious Provider, refused at every boundary it attacks.
//!
//! This is the negative half of the `ADR046-provider-001` validation
//! obligation "fake and malicious Provider". Each case takes the honest
//! Provider from `fake_provider.rs`, changes exactly one thing an attacker
//! controls, and asserts the exact closed refusal. The point is not that
//! something failed; it is that the specific boundary the change attacks is
//! the one that refuses.
//!
//! Helpers are restated here rather than shared, because each Cargo
//! integration test is its own crate and a shared module would run the
//! honest suite twice.

use d2b_contracts::v3::identity::BindingDigest;
use d2b_contracts::v3::{
    Locality, TransportBinding,
    execution_policy::{BoundedToken, ExecutionDomain},
    identity::{ResourceTypeName, SchemaFingerprint, SessionPurpose},
    provider::{
        ArtifactDigest, ArtifactDigestSet, ArtifactId, BindingTargetType, CapabilitySupport,
        CompatibilityRange, ComponentDescriptor, ComponentType, DependencyAlias,
        DependencyDeclaration, Exportability, ExtensionSchemaRegistration, PolicyEvaluation,
        ProjectionFactory, ProviderContractError, ProviderManifest, ResourceApiBinding,
        RevocationState, SignatureState, StandardCapabilityMatrix, TrustEvidence,
        UpgradeDisposition, UpgradePolicy,
    },
    resource_ref::ResourceRef,
    resource_schema::{ExtensionSchemaId, SchemaVersion},
    zone_routing::ZonePath,
};
use d2b_provider_toolkit::conformance::{CapabilityMatrix, ConformanceError};
use d2b_provider_toolkit::fakes::{FakeBus, FakePortError, FakeResourceStore};
use d2b_provider_toolkit::{AllocatorSessionBinding, ProviderAgentBootstrap, ProviderToolkitError};

const DIGEST: &str = "sha256:0000000000000000000000000000000000000000000000000000000000000001";

fn fingerprint(tail: &str) -> SchemaFingerprint {
    SchemaFingerprint::parse(format!("sha256:{}{tail}", "0".repeat(63))).expect("valid digest")
}

fn trusted() -> TrustEvidence {
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

fn digests() -> ArtifactDigestSet {
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

fn controller(component_id: &str) -> ComponentDescriptor {
    ComponentDescriptor::new(
        BoundedToken::parse(component_id).expect("valid id"),
        ComponentType::Controller,
        [ResourceTypeName::parse("Volume").expect("standard type")],
        [BoundedToken::parse("assess-update").expect("valid method")],
        [ExecutionDomain::System],
        1,
        ArtifactDigest::parse(DIGEST).expect("valid digest"),
        [],
        false,
    )
    .expect("a controller owning one ResourceType is valid")
}

fn binding_for(resource_type: &str) -> ResourceApiBinding {
    ResourceApiBinding::new(
        ResourceTypeName::parse(resource_type).expect("valid type"),
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
    .expect("a binding is valid")
}

fn manifest_with(
    trust: TrustEvidence,
    components: Vec<ComponentDescriptor>,
    bindings: Vec<ResourceApiBinding>,
    factories: Vec<ProjectionFactory>,
) -> Result<ProviderManifest, ProviderContractError> {
    ProviderManifest::new(
        ArtifactId::parse("provider-volume-local").expect("valid artifact id"),
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
        factories,
        UpgradePolicy {
            drain_before_upgrade: true,
            max_automatic_disposition: UpgradeDisposition::InPlace,
            preserves_durable_state: true,
        },
    )
}

fn honest() -> ProviderManifest {
    manifest_with(
        trusted(),
        vec![controller("volume-controller")],
        vec![binding_for("Volume")],
        vec![],
    )
    .expect("the honest manifest is valid")
}

#[test]
fn a_self_attested_artifact_is_never_admitted() {
    // Each mutation is a separate way of claiming trust the Zone did not
    // grant: no signature at all, a signature the Zone cannot check, a
    // revoked key, an emergency-denied artifact, and a conformance
    // attestation that was simply never evaluated.
    let attacks: [fn(&mut TrustEvidence); 5] = [
        |trust| trust.signature = SignatureState::Absent,
        |trust| trust.publisher_trusted = false,
        |trust| trust.revocation = RevocationState::Revoked,
        |trust| trust.emergency_deny = true,
        |trust| trust.conformance = PolicyEvaluation::Unevaluated,
    ];
    for mutate in attacks {
        let mut trust = trusted();
        mutate(&mut trust);
        let manifest = manifest_with(
            trust,
            vec![controller("volume-controller")],
            vec![binding_for("Volume")],
            vec![],
        )
        .expect("the manifest is structurally valid, only its trust is not");
        assert_eq!(
            manifest.admit(3, 4, &fingerprint("1")),
            Err(ProviderContractError::TrustNotEstablished)
        );
    }
}

#[test]
fn an_artifact_cannot_negotiate_its_way_past_the_required_api() {
    let manifest = honest();
    // A different major, a minor the artifact does not implement, and a
    // descriptor fingerprint that is not the advertised one are three
    // distinct refusals, and none of them degrades into a downgrade.
    assert_eq!(
        manifest.admit(2, 0, &fingerprint("1")),
        Err(ProviderContractError::ApiMajorMismatch)
    );
    assert_eq!(
        manifest.admit(3, 5, &fingerprint("1")),
        Err(ProviderContractError::ApiMinorTooNew)
    );
    assert_eq!(
        manifest.admit(3, 4, &fingerprint("8")),
        Err(ProviderContractError::DescriptorFingerprintMismatch)
    );
}

#[test]
fn a_provider_cannot_claim_a_resource_type_twice_or_bind_one_it_does_not_own() {
    assert_eq!(
        manifest_with(
            trusted(),
            vec![
                controller("volume-controller"),
                controller("shadow-controller")
            ],
            vec![binding_for("Volume")],
            vec![],
        ),
        Err(ProviderContractError::DuplicateDeclaration)
    );
    assert_eq!(
        manifest_with(
            trusted(),
            vec![controller("volume-controller")],
            vec![binding_for("Volume"), binding_for("Network")],
            vec![],
        ),
        Err(ProviderContractError::MissingRequiredField)
    );
}

#[test]
fn a_provider_cannot_register_an_extension_schema_for_a_foreign_resource_type() {
    let squatted = ExtensionSchemaRegistration {
        schema_id: ExtensionSchemaId::parse("volume-local.d2bus.org/Credential/spec")
            .expect("valid schema id"),
        schema_version: SchemaVersion::new(1, 0).expect("valid version"),
        schema_fingerprint: fingerprint("6"),
    };
    assert_eq!(
        ResourceApiBinding::new(
            ResourceTypeName::parse("Volume").expect("standard type"),
            SchemaVersion::new(1, 0).expect("valid version"),
            fingerprint("2"),
            SchemaVersion::new(1, 0).expect("valid version"),
            fingerprint("3"),
            StandardCapabilityMatrix::default(),
            Some(squatted),
            None,
        ),
        Err(ProviderContractError::WrongResourceType)
    );
}

#[test]
fn a_worker_cannot_grant_itself_a_dependency_portal_or_a_method_surface() {
    let escalated = |methods: Vec<BoundedToken>, dependencies: Vec<DependencyDeclaration>| {
        ComponentDescriptor::new(
            BoundedToken::parse("virtiofsd-worker").expect("valid id"),
            ComponentType::Worker,
            [],
            methods,
            [ExecutionDomain::System],
            1,
            ArtifactDigest::parse(DIGEST).expect("valid digest"),
            dependencies,
            false,
        )
    };
    assert_eq!(
        escalated(
            vec![BoundedToken::parse("exfiltrate").expect("valid method")],
            vec![],
        ),
        Err(ProviderContractError::ConflictingFields)
    );
    assert_eq!(
        escalated(
            vec![],
            vec![DependencyDeclaration {
                alias: DependencyAlias::Credential,
                required: true,
            }],
        ),
        Err(ProviderContractError::ConflictingFields)
    );
    // A worker that claims a ResourceType is claiming controller authority.
    assert_eq!(
        ComponentDescriptor::new(
            BoundedToken::parse("virtiofsd-worker").expect("valid id"),
            ComponentType::Worker,
            [ResourceTypeName::parse("Volume").expect("standard type")],
            [],
            [ExecutionDomain::System],
            1,
            ArtifactDigest::parse(DIGEST).expect("valid digest"),
            [],
            false,
        ),
        Err(ProviderContractError::ConflictingFields)
    );
}

#[test]
fn a_provider_cannot_smuggle_a_backing_device_or_binding_across_a_zone() {
    let service = ResourceTypeName::parse("audio.d2bus.org.AudioService").expect("valid type");
    let binding_type = ResourceTypeName::parse("audio.d2bus.org.AudioBinding").expect("valid type");
    let factory = ProjectionFactory::new(
        service,
        binding_type,
        [ResourceTypeName::parse("Device").expect("standard type")],
        [BindingTargetType::Guest],
        fingerprint("4"),
        fingerprint("5"),
        Exportability::ExplicitExport,
    )
    .expect("an explicit-export factory is valid");
    // The one legal export target is the owner Service. A backing Device,
    // the Binding expressing local consumer intent, and an unrelated
    // Credential are all refused.
    for smuggled in [
        "Device/headset",
        "audio.d2bus.org.AudioBinding/desk",
        "Credential/keyring",
    ] {
        assert_eq!(
            factory.admits_export_target(&ResourceRef::parse(smuggled).expect("valid ref")),
            Err(ProviderContractError::ProjectionFactoryInvalid)
        );
    }
    // A factory cannot be declared for a capability whose exportability is
    // forbidden, so a forbidden capability has no export machinery at all.
    assert_eq!(
        ProjectionFactory::new(
            ResourceTypeName::parse("usb.d2bus.org.UsbService").expect("valid type"),
            ResourceTypeName::parse("usb.d2bus.org.UsbBinding").expect("valid type"),
            [ResourceTypeName::parse("Device").expect("standard type")],
            [BindingTargetType::Zone],
            fingerprint("4"),
            fingerprint("5"),
            Exportability::Forbidden,
        ),
        Err(ProviderContractError::ExportForbidden)
    );
}

#[test]
fn a_projection_factory_must_be_backed_by_a_resource_type_the_provider_binds() {
    let orphan = ProjectionFactory::new(
        ResourceTypeName::parse("audio.d2bus.org.AudioService").expect("valid type"),
        ResourceTypeName::parse("audio.d2bus.org.AudioBinding").expect("valid type"),
        [ResourceTypeName::parse("Device").expect("standard type")],
        [BindingTargetType::Guest],
        fingerprint("4"),
        fingerprint("5"),
        Exportability::ExplicitExport,
    )
    .expect("the factory itself is well formed");
    assert_eq!(
        manifest_with(
            trusted(),
            vec![controller("volume-controller")],
            vec![binding_for("Volume")],
            vec![orphan],
        ),
        Err(ProviderContractError::ProjectionFactoryInvalid)
    );
}

#[test]
fn an_impostor_binding_cannot_tell_an_agent_it_is_a_different_provider() {
    let agent = ProviderAgentBootstrap::new(
        ResourceRef::parse("Provider/volume-local").expect("valid ref"),
        ZonePath::local_root(),
        SessionPurpose::parse("provider-agent").expect("valid purpose"),
    );
    let transport = || {
        TransportBinding::new(
            Locality::Local,
            BindingDigest::parse(DIGEST).expect("valid digest"),
        )
    };
    assert_eq!(
        agent.admit(AllocatorSessionBinding::new(
            ZonePath::local_root(),
            ResourceRef::parse("Provider/credential-entra").expect("valid ref"),
            SessionPurpose::parse("provider-agent").expect("valid purpose"),
            transport(),
        )),
        Err(ProviderToolkitError::BootstrapProviderMismatch)
    );
    assert_eq!(
        agent.admit(AllocatorSessionBinding::new(
            ZonePath::local_root(),
            ResourceRef::parse("Volume/volume-local").expect("valid ref"),
            SessionPurpose::parse("provider-agent").expect("valid purpose"),
            transport(),
        )),
        Err(ProviderToolkitError::BootstrapRefWrongType)
    );
    assert_eq!(
        agent.admit(AllocatorSessionBinding::new(
            ZonePath::local_root(),
            ResourceRef::parse("Provider/volume-local").expect("valid ref"),
            SessionPurpose::parse("resource-api").expect("valid purpose"),
            transport(),
        )),
        Err(ProviderToolkitError::BootstrapPurposeMismatch)
    );
}

#[test]
fn probing_the_bus_yields_a_refusal_and_never_the_binding_table() {
    let mut bus = FakeBus::with_bindings([(
        DependencyAlias::Volume,
        ResourceRef::parse("Provider/volume-backend").expect("valid ref"),
    )]);
    // A Provider that walks the alias set learns only which of its own
    // declared aliases are bound. It never receives a route table, and an
    // unbound alias is a refusal rather than a fallback endpoint.
    for alias in DependencyAlias::ALL {
        match bus.resolve_alias(alias) {
            Ok(resolved) => assert_eq!(alias, DependencyAlias::Volume, "{resolved:?}"),
            Err(error) => assert_eq!(error, FakePortError::AliasNotBound),
        }
    }
}

#[test]
fn a_controller_cannot_write_status_for_a_resource_type_it_does_not_own() {
    let mut store = FakeResourceStore::owning(vec!["Volume".to_owned()]);
    for foreign in ["Credential/keyring", "Network/lan", "Provider/other"] {
        assert_eq!(
            store.write_status(&ResourceRef::parse(foreign).expect("valid ref")),
            Err(FakePortError::NotOwned)
        );
    }
}

#[test]
fn a_capability_cannot_be_refused_unless_the_signed_matrix_declares_it_unsupported() {
    let matrix = CapabilityMatrix::new(
        [BoundedToken::parse("expedited-reconcile").expect("valid capability")],
        [BoundedToken::parse("in-place-resize").expect("valid capability")],
    )
    .expect("a disjoint matrix is valid");
    assert_eq!(
        matrix
            .refuse(&BoundedToken::parse("in-place-resize").expect("valid capability"))
            .expect("a declared unsupported capability is refusable"),
        "unsupported-capability"
    );
    // Refusing a supported capability, or one the matrix never mentions, is
    // exactly the silent decline the base-conformance rule forbids.
    for undeclared in ["expedited-reconcile", "drain-on-upgrade"] {
        assert_eq!(
            matrix.refuse(&BoundedToken::parse(undeclared).expect("valid capability")),
            Err(ConformanceError::CapabilityUndeclared)
        );
    }
    // The signed manifest matrix agrees: absence is not support.
    let signed = StandardCapabilityMatrix::new([(
        BoundedToken::parse("expedited-reconcile").expect("valid capability"),
        CapabilitySupport::Unsupported,
    )])
    .expect("a single-entry matrix is valid");
    assert!(
        !signed.supports(&BoundedToken::parse("expedited-reconcile").expect("valid capability"))
    );
    assert!(!signed.supports(&BoundedToken::parse("never-declared").expect("valid capability")));
}

#[test]
fn a_hostile_identifier_never_reaches_a_diagnostic_surface() {
    // A publisher, artifact identifier, and component identifier are all
    // attacker-authored in a third-party artifact, so none of them may
    // render through Debug into a log line.
    let hostile = ComponentDescriptor::new(
        BoundedToken::parse("evil-log-injection").expect("valid id"),
        ComponentType::Controller,
        [ResourceTypeName::parse("Volume").expect("standard type")],
        [BoundedToken::parse("assess-update").expect("valid method")],
        [ExecutionDomain::System],
        1,
        ArtifactDigest::parse(DIGEST).expect("valid digest"),
        [],
        false,
    )
    .expect("valid component");
    let manifest = manifest_with(
        trusted(),
        vec![hostile],
        vec![binding_for("Volume")],
        vec![],
    )
    .expect("valid manifest");
    for rendered in [
        format!("{:?}", manifest),
        format!("{:?}", manifest.components()),
        format!("{:?}", manifest.trust()),
        format!("{:?}", manifest.artifact_id()),
    ] {
        assert!(!rendered.contains("evil-log-injection"));
        assert!(!rendered.contains("first-party"));
        assert!(!rendered.contains("provider-volume-local"));
        assert!(!rendered.contains("sha256:"));
    }
}
