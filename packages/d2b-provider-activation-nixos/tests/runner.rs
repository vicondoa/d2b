use d2b_contracts_resource::v3::{
    ActivationMode,
    ArtifactId,
    ResourceRef,
};
use d2b_provider_activation_nixos::{
    ActivationHelper, ActivationRunner, ActivationRunnerError, ActivationRunnerRequest,
    RunnerOutcomeCode,
};

#[derive(Debug)]
struct FixedHelper(Result<RunnerOutcomeCode, ActivationRunnerError>);

impl ActivationHelper for FixedHelper {
    fn activate(
        &self,
        _request: &ActivationRunnerRequest,
    ) -> Result<RunnerOutcomeCode, ActivationRunnerError> {
        self.0
    }
}

fn request(execution_ref: &str) -> ActivationRunnerRequest {
    ActivationRunnerRequest {
        system_artifact_id: ArtifactId::parse("dev-vm-system").unwrap(),
        execution_ref: ResourceRef::parse(execution_ref).expect("valid execution reference"),
        activation_mode: ActivationMode::Switch,
    }
}

#[test]
fn runner_rejects_invalid_requests_before_helper_dispatch() {
    let runner = ActivationRunner;
    let invalid_target = request("User/alice");
    assert_eq!(
        runner.run(
            &invalid_target,
            &FixedHelper(Ok(RunnerOutcomeCode::Succeeded))
        ),
        Err(ActivationRunnerError::InvalidRequest)
    );
}

#[test]
fn runner_preserves_source_on_refusal_and_failure() {
    let runner = ActivationRunner;
    for outcome in [
        RunnerOutcomeCode::HelperRefused,
        RunnerOutcomeCode::HelperFailed,
    ] {
        let result = runner
            .run(&request("Host/dev"), &FixedHelper(Ok(outcome)))
            .expect("helper outcome");
        assert_eq!(result.outcome, outcome);
        assert!(result.source_generation_preserved);
    }
}

#[test]
fn runner_success_does_not_preserve_the_source_generation() {
    let result = ActivationRunner
        .run(
            &request("Host/dev"),
            &FixedHelper(Ok(RunnerOutcomeCode::Succeeded)),
        )
        .expect("helper outcome");
    assert_eq!(result.outcome, RunnerOutcomeCode::Succeeded);
    assert!(!result.source_generation_preserved);
}

#[test]
fn runner_rejects_untrusted_helper_output() {
    assert_eq!(
        ActivationRunner::parse_helper_output(br#"{"outcome":"failed"}"#),
        Err(ActivationRunnerError::InvalidHelperOutput)
    );
    assert_eq!(
        ActivationRunner::parse_helper_output(br#"{"outcome":"helper-failed","path":"/nix"}"#),
        Err(ActivationRunnerError::InvalidHelperOutput)
    );
    assert_eq!(
        ActivationRunner::parse_helper_output(&vec![b'x'; 513]),
        Err(ActivationRunnerError::InvalidHelperOutput)
    );
    assert_eq!(
        ActivationRunner::parse_helper_output(br#"{"outcome":"helper-failed"}"#),
        Ok(RunnerOutcomeCode::HelperFailed)
    );
}
