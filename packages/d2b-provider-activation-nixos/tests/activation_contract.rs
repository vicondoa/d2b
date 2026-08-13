use d2b_contracts::v3::ActivationOutcomeCode;
use d2b_contracts::v3::{ActivationMode, NixosGenerationSpec, ResourcePhase, ResourceRef};
use d2b_provider_activation_nixos::{
    ActivationCaller, ActivationController, CallerRole, GenerationObservation, GenerationPhase,
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
    assert_eq!(result.phase(), ResourcePhase::Pending);
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
