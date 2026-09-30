use d2b_contracts_resource::v3::ActivationOutcomeCode;
use d2b_contracts_resource::v3::{ActivationMode, NixosGenerationSpec, ResourcePhase, ResourceRef};
use d2b_provider_activation_nixos::{
    ActivationCaller, ActivationController, ActivationTrust, ActivationTrustExpectation,
    ActivationApplicationVerifier, CallerRole, GenerationObservation, GenerationPhase,
    SignedActivationApplicationVerifier, TrustStatus, activation_runner_name, activation_runner_ref,
};
use ring::signature::{Ed25519KeyPair, KeyPair};
use sha2::{Digest, Sha256};

fn spec() -> NixosGenerationSpec {
    NixosGenerationSpec::new(
        ResourceRef::parse("Provider/activation-nixos").unwrap(),
        ResourceRef::parse("Guest/dev-vm").unwrap(),
        "dev-vm-system",
        ActivationMode::Switch,
        None,
    )
    .unwrap()
}

fn spec_with_mode(mode: ActivationMode) -> NixosGenerationSpec {
    NixosGenerationSpec::new(
        ResourceRef::parse("Provider/activation-nixos").unwrap(),
        ResourceRef::parse("Guest/dev-vm").unwrap(),
        "dev-vm-system",
        mode,
        None,
    )
    .unwrap()
}

fn caller() -> ActivationCaller {
    ActivationCaller::new(
        CallerRole::Lifecycle,
        ResourceRef::parse("Guest/dev-vm").unwrap(),
    )
}

#[test]
fn compatible_generation_starts_one_typed_runner() {
    let controller = ActivationController::new();
    let result = controller
        .reconcile(
            &spec(),
            &caller(),
            &[],
            GenerationObservation::new("gen-7", GenerationPhase::Pending).unwrap(),
        )
        .unwrap();
    assert_eq!(result.runner_requests().len(), 1);
    assert!(result.runner_requests()[0].start_root);
    assert_eq!(
        result.runner_requests()[0].runner_name,
        activation_runner_name(
            &ResourceRef::parse("activation-nixos.d2bus.org.NixosGeneration/gen-7").unwrap()
        )
    );
    assert_eq!(result.phase(), ResourcePhase::Pending);
}

#[test]
fn activation_runner_reference_is_stable_and_target_local() {
    let generation =
        ResourceRef::parse("activation-nixos.d2bus.org.NixosGeneration/gen-7").unwrap();
    assert_eq!(
        activation_runner_ref(&generation),
        activation_runner_ref(&generation)
    );
    assert_eq!(
        activation_runner_ref(&generation).resource_type().as_str(),
        "EphemeralProcess"
    );
    assert_ne!(
        activation_runner_ref(&generation),
        activation_runner_ref(
            &ResourceRef::parse("activation-nixos.d2bus.org.NixosGeneration/gen-8").unwrap()
        )
    );
}

#[test]
fn activation_runner_spec_is_closed_and_bounded() {
    let generation =
        ResourceRef::parse("activation-nixos.d2bus.org.NixosGeneration/gen-7").unwrap();
    let controller = ActivationController::new();
    let planned = controller
        .reconcile(
            &spec(),
            &caller(),
            &[],
            GenerationObservation::new("gen-7", GenerationPhase::Pending).unwrap(),
        )
        .unwrap();
    let runner =
        d2b_provider_activation_nixos::activation_runner_spec(&planned.runner_requests()[0]);
    let rendered = serde_json::to_value(&runner).expect("runner spec is serializable");
    assert_eq!(
        rendered["activationInput"]["systemArtifactId"],
        "dev-vm-system"
    );
    assert_eq!(rendered["activationInput"]["targetGeneration"], 7);
    assert_eq!(rendered["activationInput"]["activationMode"], "switch");
    assert_eq!(
        runner.execution().execution_ref(),
        &ResourceRef::parse("Guest/dev-vm").unwrap()
    );
    assert_eq!(
        runner.execution().template().as_str(),
        "activation-nixos-runner"
    );
    assert_eq!(
        runner.execution().process_class(),
        d2b_contracts_resource::v3::ProcessClass::Worker
    );
    assert!(runner.execution().sandbox().start_root());
    assert!(runner.execution().sandbox().no_new_privileges());
    assert_eq!(runner.start_deadline().as_str(), "120s");
    assert_eq!(runner.runtime_deadline().as_str(), "600s");
    assert_eq!(
        activation_runner_name(&generation).as_str(),
        "activation-nixos--runner--gen-7"
    );
}

#[test]
fn unauthorized_or_foreign_callers_refuse_before_runner_creation() {
    let controller = ActivationController::new();
    let foreign = ActivationCaller::new(
        CallerRole::User,
        ResourceRef::parse("Guest/dev-vm").unwrap(),
    );
    let result = controller.reconcile(
        &spec(),
        &foreign,
        &[],
        GenerationObservation::new("gen-7", GenerationPhase::Pending).unwrap(),
    );
    assert!(result.is_err());
}

#[test]
fn runner_failure_preserves_the_source_generation_and_audits_one_code() {
    let controller = ActivationController::new();
    let failed = controller
        .apply_runner_result(
            &spec(),
            ActivationOutcomeCode::HelperFailed,
            GenerationObservation::new("gen-6", GenerationPhase::Ready).unwrap(),
        )
        .unwrap();
    assert!(failed.source_generation_preserved());
    assert_eq!(failed.audit_codes(), &[ActivationOutcomeCode::HelperFailed]);
}

#[test]
fn adopted_outcome_is_rejected_for_switch_mode() {
    let controller = ActivationController::new();
    let result = controller.apply_runner_result(
        &spec(),
        ActivationOutcomeCode::Adopted,
        GenerationObservation::new("gen-6", GenerationPhase::Ready).unwrap(),
    );
    assert_eq!(
        result.unwrap_err(),
        d2b_provider_activation_nixos::ActivationError::OutcomeMismatch
    );
}

#[test]
fn activation_refuses_a_runner_step_outside_the_declared_set() {
    let controller = ActivationController::new();
    // The runner performs only the steps the family declares: switch, boot,
    // and test. Adoption records an already-active generation and is not a
    // runner step, and a rollback is a daemon-side operation, not a runner
    // step - each is outside the declared set and refused.
    assert_eq!(
        d2b_provider_activation_nixos::declared_runner_step(ActivationMode::Adopt),
        None
    );
    assert!(
        controller
            .refuse_undeclared_runner_step("adopt")
            .is_err(),
        "the adopt step is outside the declared runner step set"
    );
    assert!(
        controller
            .refuse_undeclared_runner_step("rollback")
            .is_err(),
        "the rollback step is outside the declared runner step set"
    );
    assert_eq!(controller.refuse_undeclared_runner_step("switch"), Ok(()));
    assert_eq!(controller.refuse_undeclared_runner_step("boot"), Ok(()));
    assert_eq!(controller.refuse_undeclared_runner_step("test"), Ok(()));
    assert_eq!(
        d2b_provider_activation_nixos::declared_runner_step(ActivationMode::Switch)
            .map(|step| step.label),
        Some("switch")
    );
    assert_eq!(
        d2b_provider_activation_nixos::declared_runner_step(ActivationMode::Switch)
            .map(|step| step.generation_suffix),
        Some("gen")
    );
}

#[test]
fn adopt_mode_accepts_adoption_without_starting_a_runner() {
    let controller = ActivationController::new();
    let adopt = spec_with_mode(ActivationMode::Adopt);
    let pending = controller
        .reconcile(
            &adopt,
            &caller(),
            &[],
            GenerationObservation::new("gen-7", GenerationPhase::Pending).unwrap(),
        )
        .unwrap();
    assert!(pending.runner_requests().is_empty());

    let result = controller
        .apply_runner_result(
            &adopt,
            ActivationOutcomeCode::Adopted,
            GenerationObservation::new("gen-6", GenerationPhase::Ready).unwrap(),
        )
        .unwrap();
    assert_eq!(result.phase(), ResourcePhase::Ready);
    assert!(!result.source_generation_preserved());
}

#[test]
fn test_mode_succeeds_without_preserving_the_source_generation() {
    let controller = ActivationController::new();
    let result = controller
        .apply_runner_result(
            &spec_with_mode(ActivationMode::Test),
            ActivationOutcomeCode::Succeeded,
            GenerationObservation::new("gen-6", GenerationPhase::Ready).unwrap(),
        )
        .unwrap();
    assert_eq!(result.phase(), ResourcePhase::Succeeded);
    assert!(!result.source_generation_preserved());
}

#[test]
fn successful_switch_reports_ready_and_replaces_the_source_generation() {
    let controller = ActivationController::new();
    let result = controller
        .apply_runner_result(
            &spec(),
            ActivationOutcomeCode::Succeeded,
            GenerationObservation::new("gen-6", GenerationPhase::Ready).unwrap(),
        )
        .unwrap();
    assert_eq!(result.phase(), ResourcePhase::Ready);
    assert!(!result.source_generation_preserved());
}

#[test]
fn deleted_generation_is_not_restarted() {
    let controller = ActivationController::new();
    let result = controller.reconcile(
        &spec(),
        &caller(),
        &[],
        GenerationObservation::new("gen-7", GenerationPhase::Deleted).unwrap(),
    );
    assert_eq!(
        result.unwrap_err(),
        d2b_provider_activation_nixos::ActivationError::AlreadyDeleted
    );
}

#[test]
fn malformed_generation_observation_cannot_start_a_zero_generation_runner() {
    let controller = ActivationController::new();
    let result = controller.reconcile(
        &spec(),
        &caller(),
        &[],
        GenerationObservation::new("generation", GenerationPhase::Pending).unwrap(),
    );

    assert_eq!(
        result.unwrap_err(),
        d2b_provider_activation_nixos::ActivationError::InvalidSpec
    );
}

#[test]
fn stale_deleted_source_cannot_project_a_successful_activation() {
    let controller = ActivationController::new();
    let result = controller.apply_runner_result(
        &spec(),
        ActivationOutcomeCode::Succeeded,
        GenerationObservation::new("gen-6", GenerationPhase::Deleted).unwrap(),
    );

    assert_eq!(
        result.unwrap_err(),
        d2b_provider_activation_nixos::ActivationError::AlreadyDeleted
    );
}

#[test]
fn prior_generation_reference_must_be_present_in_observations() {
    let spec = NixosGenerationSpec::new(
        ResourceRef::parse("Provider/activation-nixos").unwrap(),
        ResourceRef::parse("Guest/dev-vm").unwrap(),
        "dev-vm-system",
        ActivationMode::Switch,
        Some(ResourceRef::parse("activation-nixos.d2bus.org.NixosGeneration/gen-6").unwrap()),
    )
    .unwrap();
    let controller = ActivationController::new();
    let result = controller.reconcile(
        &spec,
        &caller(),
        &[],
        GenerationObservation::new("gen-7", GenerationPhase::Pending).unwrap(),
    );
    assert_eq!(
        result.unwrap_err(),
        d2b_provider_activation_nixos::ActivationError::InvalidSpec
    );
    let result = controller
        .reconcile(
            &spec,
            &caller(),
            &[GenerationObservation::new("gen-6", GenerationPhase::Ready).unwrap()],
            GenerationObservation::new("gen-7", GenerationPhase::Pending).unwrap(),
        )
        .unwrap();
    assert_eq!(result.runner_requests().len(), 1);
}

fn trust_fixture() -> (ActivationTrust, ActivationTrustExpectation, Vec<u8>, String) {
    let rng = ring::rand::SystemRandom::new();
    let key_pair = Ed25519KeyPair::generate_pkcs8(&rng).unwrap();
    let key_pair = Ed25519KeyPair::from_pkcs8(key_pair.as_ref()).unwrap();
    let payload = b"activation-envelope".to_vec();
    let artifact = b"verified-system".to_vec();
    let signature = key_pair.sign(&payload);
    let artifact_digest = format!("sha256:{:x}", Sha256::digest(&artifact));
    let catalog_digest = format!("sha256:{}", "1".repeat(64));
    let trust = ActivationTrust::new(
        7,
        Some("revocation/7".to_owned()),
        TrustStatus::Clear,
        TrustStatus::Clear,
        "publisher-root",
        "signing-key-7",
        key_pair.public_key().as_ref().to_vec(),
        signature.as_ref().to_vec(),
    );
    let expected = ActivationTrustExpectation::new(
        7,
        Some("revocation/7".to_owned()),
        "publisher-root",
        "signing-key-7",
        artifact_digest,
        catalog_digest.clone(),
        payload,
    );
    (trust, expected, artifact, catalog_digest)
}

#[test]
fn activation_verification_requires_all_trust_and_digest_fences() {
    let (trust, expected, artifact, catalog_digest) = trust_fixture();
    ActivationController::new()
        .verify_application(&trust, &expected, &artifact, &catalog_digest)
        .expect("trusted activation verifies");

    let mut cases = Vec::new();
    cases.push(ActivationTrust::new(
        8,
        Some("revocation/7".to_owned()),
        TrustStatus::Clear,
        TrustStatus::Clear,
        "publisher-root",
        "signing-key-7",
        vec![0; 32],
        vec![0; 64],
    ));
    cases.push(ActivationTrust::new(
        7,
        Some("revocation/8".to_owned()),
        TrustStatus::Clear,
        TrustStatus::Clear,
        "publisher-root",
        "signing-key-7",
        vec![0; 32],
        vec![0; 64],
    ));
    cases.push(ActivationTrust::new(
        7,
        Some("revocation/7".to_owned()),
        TrustStatus::Denied,
        TrustStatus::Clear,
        "publisher-root",
        "signing-key-7",
        vec![0; 32],
        vec![0; 64],
    ));
    cases.push(ActivationTrust::new(
        7,
        Some("revocation/7".to_owned()),
        TrustStatus::Unknown,
        TrustStatus::Clear,
        "publisher-root",
        "signing-key-7",
        vec![0; 32],
        vec![0; 64],
    ));
    cases.push(ActivationTrust::new(
        7,
        Some("revocation/7".to_owned()),
        TrustStatus::Clear,
        TrustStatus::Clear,
        "other-root",
        "signing-key-7",
        vec![0; 32],
        vec![0; 64],
    ));
    cases.push(ActivationTrust::new(
        7,
        Some("revocation/7".to_owned()),
        TrustStatus::Clear,
        TrustStatus::Clear,
        "publisher-root",
        "other-key",
        vec![0; 32],
        vec![0; 64],
    ));

    for (i, (trust, expected_error)) in cases.into_iter().zip([
        d2b_provider_activation_nixos::ActivationVerificationError::TrustEpochMismatch,
        d2b_provider_activation_nixos::ActivationVerificationError::RevocationRefMismatch,
        d2b_provider_activation_nixos::ActivationVerificationError::TrustDenied,
        d2b_provider_activation_nixos::ActivationVerificationError::TrustDenied,
        d2b_provider_activation_nixos::ActivationVerificationError::PublisherRootMismatch,
        d2b_provider_activation_nixos::ActivationVerificationError::SignatureIdMismatch,
    ]).enumerate() {
        assert_eq!(
            trust.verify(&expected, &artifact, &catalog_digest),
            Err(expected_error),
            "case {i}: expected {expected_error:?}",
        );
    }
}

#[test]
fn activation_verification_rejects_digest_catalog_and_signature_changes() {
    let (trust, expected, artifact, catalog_digest) = trust_fixture();
    assert_eq!(
        trust.verify(&expected, b"changed", &catalog_digest),
        Err(d2b_provider_activation_nixos::ActivationVerificationError::ArtifactDigestMismatch)
    );
    assert_eq!(
        trust.verify(
            &expected,
            &artifact,
            &("sha256:".to_owned() + &"2".repeat(64))
        ),
        Err(d2b_provider_activation_nixos::ActivationVerificationError::ArtifactCatalogDigestMismatch)
    );
    let payload = b"changed-envelope".to_vec();
    let changed = ActivationTrustExpectation::new(
        7,
        Some("revocation/7".to_owned()),
        "publisher-root",
        "signing-key-7",
        format!("sha256:{:x}", Sha256::digest(&artifact)),
        catalog_digest.clone(),
        payload,
    );
    assert_eq!(
        trust.verify(&changed, &artifact, &catalog_digest),
        Err(d2b_provider_activation_nixos::ActivationVerificationError::SignatureInvalid)
    );
}

#[test]
fn signed_activation_verifier_binds_verification_to_the_exact_runner_request() {
    let (trust, expected, artifact, catalog_digest) = trust_fixture();
    let request = d2b_provider_activation_nixos::RunnerRequest {
        runner_name: d2b_contracts_resource::v3::ResourceName::parse("runner").unwrap(),
        execution_ref: ResourceRef::parse("Host/host-system").unwrap(),
        system_artifact_id: d2b_contracts_resource::v3::ArtifactId::parse("system").unwrap(),
        activation_mode: ActivationMode::Switch,
        target_generation: 1,
        start_root: true,
    };
    let verifier = SignedActivationApplicationVerifier::new(
        request.clone(),
        trust,
        expected,
        artifact,
        catalog_digest,
    );
    verifier
        .verify_application(&ActivationController::new(), &request)
        .expect("exact request verifies");
    let mut changed = request;
    changed.target_generation = 2;
    assert_eq!(
        verifier.verify_application(&ActivationController::new(), &changed),
        Err(d2b_provider_activation_nixos::ActivationVerificationError::InvalidEvidence)
    );
}

// ---------------------------------------------------------------------------
// The verified deployment graph (U31)
// ---------------------------------------------------------------------------

/// The implementation identities a build compiles, as the generated
/// registration table spells them.
const COMPILED: &[&str] = &["activation-nixos", "system-minijail", "volume-local"];

/// Render a deployment graph document the way the publisher writes it, with
/// the self-hash computed over the canonical bytes of everything else.
fn deployment_document(
    schema_version: &str,
    implementations: &[&str],
    tamper: bool,
) -> Vec<u8> {
    use d2b_contracts_resource::v3::{canonical_json_bytes, framed_canonical_digest};
    use d2b_provider_activation_nixos::{DEPLOYMENT_GRAPH_DIGEST_DOMAIN, DEPLOYMENT_GRAPH_SCHEMA};

    let mut document = serde_json::Map::new();
    document.insert(
        "schemaVersion".to_owned(),
        serde_json::Value::String(if schema_version.is_empty() {
            DEPLOYMENT_GRAPH_SCHEMA.to_owned()
        } else {
            schema_version.to_owned()
        }),
    );
    document.insert(
        "implementations".to_owned(),
        serde_json::Value::Array(
            implementations
                .iter()
                .map(|value| serde_json::Value::String((*value).to_owned()))
                .collect(),
        ),
    );
    // A field this family never reads, so the digest demonstrably covers the
    // whole document rather than only the family's own view of it.
    document.insert(
        "stateVolume".to_owned(),
        serde_json::Value::String("Volume/d2b-state".to_owned()),
    );
    // The publisher hashes the document it is about to write, which has no
    // digest field yet.
    let bytes =
        canonical_json_bytes(&serde_json::Value::Object(document.clone())).expect("canonical bytes");
    let digest = framed_canonical_digest(DEPLOYMENT_GRAPH_DIGEST_DOMAIN, &bytes);
    if tamper {
        // Edit a field the family does not read, after the hash was taken.
        document.insert(
            "stateVolume".to_owned(),
            serde_json::Value::String("Volume/someone-elses-state".to_owned()),
        );
    }
    document.insert(
        "graphDigest".to_owned(),
        serde_json::Value::String(digest),
    );
    serde_json::to_vec(&serde_json::Value::Object(document)).expect("render document")
}

fn accept(bytes: &[u8]) -> Result<d2b_provider_activation_nixos::AcceptedDeploymentGraph, d2b_provider_activation_nixos::ActivationVerificationError> {
    ActivationController::new().accept_deployment_graph(bytes, COMPILED)
}

#[test]
fn a_verified_deployment_graph_binds_the_compiled_implementations() {
    let graph = accept(&deployment_document("", &["activation-nixos"], false))
        .expect("a self-consistent document verifies");
    assert!(graph.publishes("activation-nixos"));
    assert!(
        !graph.publishes("audio-pipewire"),
        "a family that was not published is not bound"
    );
}

#[test]
fn a_tampered_deployment_graph_is_refused() {
    assert_eq!(
        accept(&deployment_document("", &["activation-nixos"], true)),
        Err(
            d2b_provider_activation_nixos::ActivationVerificationError::DeploymentGraphDigestMismatch
        ),
        "a document edited after verification refuses, even where the edit is outside this family's view"
    );
}

#[test]
fn an_old_deployment_artifact_is_refused() {
    assert_eq!(
        accept(&deployment_document("d2b-deployment-bootstrap/0", &["activation-nixos"], false)),
        Err(
            d2b_provider_activation_nixos::ActivationVerificationError::DeploymentGraphSchemaUnsupported
        ),
        "another contract version is refused even when its bytes are self-consistent"
    );
}

#[test]
fn an_unknown_implementation_is_refused() {
    assert_eq!(
        accept(&deployment_document("", &["provider-from-another-release"], false)),
        Err(
            d2b_provider_activation_nixos::ActivationVerificationError::UnknownDeploymentImplementation
        ),
        "an implementation no compiled declaration binds is refused with no allowlist to extend"
    );
}

#[test]
fn an_absent_deployment_graph_is_refused() {
    assert_eq!(
        accept(b""),
        Err(
            d2b_provider_activation_nixos::ActivationVerificationError::DeploymentGraphUnreadable
        ),
        "no document means no accepted deployment, and the family plans no runner"
    );
}
