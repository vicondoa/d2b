//! Declared activation-family vocabulary.
//!
//! The activation runner step set, the wire labels the generation spec and
//! the target-local helper share, and the generation-name suffixes the
//! daemon derives for each mode. The family declares these facts here; the
//! daemon and the broker read them from this crate instead of spelling the
//! vocabulary themselves.

use d2b_contracts_resource::v3::ActivationMode;

/// One declared activation runner step.
///
/// The `label` is the wire spelling the `NixosGeneration` spec's
/// `activationMode` and the target-local helper share; the
/// `generation_suffix` is the resource-name suffix the daemon derives for
/// the mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActivationRunnerStep {
    /// The wire activation-mode label.
    pub label: &'static str,
    /// The generation-name suffix the daemon derives for this step.
    pub generation_suffix: &'static str,
}

/// The runner steps the activation family's runner may perform.
///
/// The runner never performs a step outside this set: an activation that
/// requests one is refused before any runner is planned. `Adopt` is
/// deliberately absent - adoption records an already-active generation
/// without running the helper.
pub const ACTIVATION_RUNNER_STEPS: &[ActivationRunnerStep] = &[
    ActivationRunnerStep {
        label: "switch",
        generation_suffix: "gen",
    },
    ActivationRunnerStep {
        label: "boot",
        generation_suffix: "boot",
    },
    ActivationRunnerStep {
        label: "test",
        generation_suffix: "test",
    },
];

/// The declared runner step for one activation mode.
///
/// `None` when the mode is outside the declared set: the runner never
/// performs it, and an activation that requests it is refused.
pub const fn declared_runner_step(mode: ActivationMode) -> Option<&'static ActivationRunnerStep> {
    match mode {
        ActivationMode::Switch => Some(&ACTIVATION_RUNNER_STEPS[0]),
        ActivationMode::Boot => Some(&ACTIVATION_RUNNER_STEPS[1]),
        ActivationMode::Test => Some(&ACTIVATION_RUNNER_STEPS[2]),
        ActivationMode::Adopt => None,
    }
}

/// Whether one step label is inside the family's declared runner step set.
pub fn is_declared_runner_step(step: &str) -> bool {
    ACTIVATION_RUNNER_STEPS
        .iter()
        .any(|declared| declared.label == step)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declared_steps_carry_the_wire_labels_and_generation_suffixes() {
        assert_eq!(ACTIVATION_RUNNER_STEPS.len(), 3);
        assert_eq!(
            declared_runner_step(ActivationMode::Switch),
            Some(&ActivationRunnerStep {
                label: "switch",
                generation_suffix: "gen",
            })
        );
        assert_eq!(
            declared_runner_step(ActivationMode::Boot),
            Some(&ActivationRunnerStep {
                label: "boot",
                generation_suffix: "boot",
            })
        );
        assert_eq!(
            declared_runner_step(ActivationMode::Test),
            Some(&ActivationRunnerStep {
                label: "test",
                generation_suffix: "test",
            })
        );
    }

    #[test]
    fn adopt_is_outside_the_declared_runner_step_set() {
        // Adoption records an already-active generation; the runner never
        // performs it, and the step is not declared.
        assert_eq!(declared_runner_step(ActivationMode::Adopt), None);
        assert!(!is_declared_runner_step("adopt"));
        assert!(!is_declared_runner_step("rollback"));
        assert!(is_declared_runner_step("switch"));
        assert!(is_declared_runner_step("boot"));
        assert!(is_declared_runner_step("test"));
    }
}