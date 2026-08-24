use d2b_contracts_resource::v3::ActivationOutcomeCode;
use d2b_contracts_resource::v3::{
    ActivationMode,
    NixosGenerationSpec,
    ResourcePhase,
    ResourceRef,
};
use d2b_provider_activation_nixos::{
    activation_runner_name, activation_runner_ref, ActivationCaller, ActivationController,
    CallerRole, GenerationObservation, GenerationPhase,
};

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
    let controller = ActivationController::new(3);
    let result = controller
        .reconcile(
            &spec(),
            &caller(),
            &[],
            GenerationObservation::new("gen-7", GenerationPhase::Pending),
        )
        .unwrap();
    assert_eq!(result.runner_requests().len(), 1);
    assert!(result.runner_requests()[0].start_root);
    assert_eq!(
        result.runner_requests()[0].runner_name,
        activation_runner_name(
            &ResourceRef::parse(
                "activation-nixos.d2bus.org.NixosGeneration/gen-7"
            )
            .unwrap()
        )
    );
    assert_eq!(result.phase(), ResourcePhase::Pending);
}

#[test]
fn activation_runner_reference_is_stable_and_target_local() {
    let generation =
        ResourceRef::parse("activation-nixos.d2bus.org.NixosGeneration/gen-7").unwrap();
    assert_eq!(activation_runner_ref(&generation), activation_runner_ref(&generation));
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
    let controller = ActivationController::new(3);
    let planned = controller
        .reconcile(
            &spec(),
            &caller(),
            &[],
            GenerationObservation::new("gen-7", GenerationPhase::Pending),
        )
        .unwrap();
    let runner = d2b_provider_activation_nixos::activation_runner_spec(
        &planned.runner_requests()[0],
    );
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
    assert_eq!(runner.execution().template().as_str(), "activation-nixos-runner");
    assert_eq!(runner.execution().process_class(), d2b_contracts_resource::v3::ProcessClass::Worker);
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
    let controller = ActivationController::new(3);
    let foreign = ActivationCaller::new(
        CallerRole::User,
        ResourceRef::parse("Guest/dev-vm").unwrap(),
    );
    let result = controller.reconcile(
        &spec(),
        &foreign,
        &[],
        GenerationObservation::new("gen-7", GenerationPhase::Pending),
    );
    assert!(result.is_err());
}

#[test]
fn runner_failure_preserves_the_source_generation_and_audits_one_code() {
    let controller = ActivationController::new(3);
    let failed = controller
        .apply_runner_result(
            &spec(),
            ActivationOutcomeCode::HelperFailed,
            GenerationObservation::new("gen-6", GenerationPhase::Ready),
        )
        .unwrap();
    assert!(failed.source_generation_preserved());
    assert_eq!(failed.audit_codes(), &[ActivationOutcomeCode::HelperFailed]);
}

#[test]
fn adopted_outcome_is_rejected_for_switch_mode() {
    let controller = ActivationController::new(3);
    let result = controller.apply_runner_result(
        &spec(),
        ActivationOutcomeCode::Adopted,
        GenerationObservation::new("gen-6", GenerationPhase::Ready),
    );
    assert_eq!(
        result.unwrap_err(),
        d2b_provider_activation_nixos::ActivationError::OutcomeMismatch
    );
}

#[test]
fn adopt_mode_accepts_adoption_without_starting_a_runner() {
    let controller = ActivationController::new(3);
    let adopt = spec_with_mode(ActivationMode::Adopt);
    let pending = controller
        .reconcile(
            &adopt,
            &caller(),
            &[],
            GenerationObservation::new("gen-7", GenerationPhase::Pending),
        )
        .unwrap();
    assert!(pending.runner_requests().is_empty());

    let result = controller
        .apply_runner_result(
            &adopt,
            ActivationOutcomeCode::Adopted,
            GenerationObservation::new("gen-6", GenerationPhase::Ready),
        )
        .unwrap();
    assert_eq!(result.phase(), ResourcePhase::Ready);
    assert!(!result.source_generation_preserved());
}

#[test]
fn test_mode_succeeds_without_preserving_the_source_generation() {
    let controller = ActivationController::new(3);
    let result = controller
        .apply_runner_result(
            &spec_with_mode(ActivationMode::Test),
            ActivationOutcomeCode::Succeeded,
            GenerationObservation::new("gen-6", GenerationPhase::Ready),
        )
        .unwrap();
    assert_eq!(result.phase(), ResourcePhase::Succeeded);
    assert!(!result.source_generation_preserved());
}

#[test]
fn successful_switch_reports_ready_and_replaces_the_source_generation() {
    let controller = ActivationController::new(3);
    let result = controller
        .apply_runner_result(
            &spec(),
            ActivationOutcomeCode::Succeeded,
            GenerationObservation::new("gen-6", GenerationPhase::Ready),
        )
        .unwrap();
    assert_eq!(result.phase(), ResourcePhase::Ready);
    assert!(!result.source_generation_preserved());
}

#[test]
fn deleted_generation_is_not_restarted() {
    let controller = ActivationController::new(3);
    let result = controller.reconcile(
        &spec(),
        &caller(),
        &[],
        GenerationObservation::new("gen-7", GenerationPhase::Deleted),
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
    let controller = ActivationController::new(3);
    let result = controller.reconcile(
        &spec,
        &caller(),
        &[],
        GenerationObservation::new("gen-7", GenerationPhase::Pending),
    );
    assert_eq!(
        result.unwrap_err(),
        d2b_provider_activation_nixos::ActivationError::InvalidSpec
    );
    let result = controller
        .reconcile(
            &spec,
            &caller(),
            &[GenerationObservation::new("gen-6", GenerationPhase::Ready)],
            GenerationObservation::new("gen-7", GenerationPhase::Pending),
        )
        .unwrap();
    assert_eq!(result.runner_requests().len(), 1);
}

#[test]
fn retention_prunes_only_old_terminal_generations_without_ttl() {
    let controller = ActivationController::new(2);
    let observations = vec![
        GenerationObservation::terminal("gen-1", GenerationPhase::Succeeded, 1),
        GenerationObservation::terminal("gen-2", GenerationPhase::Failed, 2),
        GenerationObservation::terminal("gen-3", GenerationPhase::Ready, 3),
    ];
    let plan = controller.retention_plan(&observations);
    assert_eq!(plan.delete_names(), &["gen-1".to_owned()]);
    assert!(!plan.uses_ttl());
}
