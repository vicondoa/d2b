//! Driver-declared startup obligations, executed in derived order.
//!
//! A startup step declares the rows it consumes as inputs and the rows or
//! services it publishes as outputs. The order is therefore derived rather
//! than chosen: a step runs only after every input it names has been
//! committed as an output of a predecessor, and a step whose input no
//! predecessor commits is a declaration refusal, named.
//!
//! The base derives and validates the plan and executes it through a
//! [`StartupStepExecutor`]. The bodies stay with the composition root: a
//! provider-side step is the provider's own hook, and the daemon-side
//! sequence that runs host-plane steps belongs to the composition root.

use std::collections::BTreeMap;
use std::fmt;

use async_trait::async_trait;
use d2b_resource_types::{DriverDescriptor, StartupStep, WellKnownType};

/// The derived execution order of every declared startup step.
#[derive(Debug, Clone, Default)]
pub struct StartupPlan {
    steps: Vec<PlannedStep>,
}

/// One step in the derived order, with the driver that declared it.
#[derive(Debug, Clone, Copy)]
pub struct PlannedStep {
    /// The step's identity.
    pub id: &'static str,
    /// The driver that declared the step.
    pub declaring: WellKnownType,
    /// The declaration itself.
    pub declaration: &'static StartupStep,
}

impl StartupPlan {
    /// Derive the order from the drivers' declared steps.
    ///
    /// # Errors
    ///
    /// Returns [`StartupPlanRefusal::MissingInput`] when a step names an
    /// input no predecessor commits, [`StartupPlanRefusal::DuplicateOutput`]
    /// when two steps commit the same output, and
    /// [`StartupPlanRefusal::Cycle`] when the declared inputs and outputs
    /// form a cycle.
    pub fn derive(drivers: &[DriverDescriptor]) -> Result<Self, StartupPlanRefusal> {
        let mut declared: Vec<(WellKnownType, &'static StartupStep)> = Vec::new();
        for driver in drivers {
            for step in driver.startup {
                declared.push((driver.resource_type, step));
            }
        }
        Self::order(declared)
    }

    /// Derive the order from explicit declaration rows.
    ///
    /// A provider crate assembles its `DriverDescriptor`s once; a test or a
    /// composition root that already holds the rows states them directly.
    /// Both feed one derivation.
    ///
    /// # Errors
    ///
    /// Returns the same refusals as [`Self::derive`]: `MissingInput`,
    /// `DuplicateOutput`, or `Cycle`.
    pub fn declare(
        rows: &'static [(WellKnownType, &'static [StartupStep])],
    ) -> Result<Self, StartupPlanRefusal> {
        let mut declared: Vec<(WellKnownType, &'static StartupStep)> = Vec::new();
        for (declaring, steps) in rows {
            for step in *steps {
                declared.push((*declaring, step));
            }
        }
        Self::order(declared)
    }

    fn order(
        declared: Vec<(WellKnownType, &'static StartupStep)>,
    ) -> Result<Self, StartupPlanRefusal> {
        let mut producers: BTreeMap<&'static str, usize> = BTreeMap::new();
        for (index, (declaring, step)) in declared.iter().enumerate() {
            for output in step.outputs {
                if producers.insert(output, index).is_some() {
                    return Err(StartupPlanRefusal::DuplicateOutput {
                        step: step.id,
                        declaring: *declaring,
                        output,
                    });
                }
            }
        }
        let mut dependencies: Vec<Vec<usize>> = Vec::with_capacity(declared.len());
        for (index, (declaring, step)) in declared.iter().enumerate() {
            let mut required = Vec::new();
            for input in step.inputs {
                match producers.get(input) {
                    Some(producer) if *producer != index => required.push(*producer),
                    Some(_) => {}
                    None => {
                        return Err(StartupPlanRefusal::MissingInput {
                            step: step.id,
                            declaring: *declaring,
                            input,
                        });
                    }
                }
            }
            required.sort_unstable();
            required.dedup();
            dependencies.push(required);
        }
        let mut steps = Vec::with_capacity(declared.len());
        let mut placed = vec![false; declared.len()];
        while steps.len() < declared.len() {
            let mut progressed = false;
            for (index, (declaring, step)) in declared.iter().enumerate() {
                if placed[index]
                    || dependencies[index]
                        .iter()
                        .any(|required| !placed[*required])
                {
                    continue;
                }
                placed[index] = true;
                steps.push(PlannedStep {
                    id: step.id,
                    declaring: *declaring,
                    declaration: step,
                });
                progressed = true;
            }
            if !progressed {
                let (declaring, step) = declared
                    .iter()
                    .zip(placed.iter())
                    .find(|(_, placed)| !**placed)
                    .map(|((declaring, step), _)| (*declaring, *step))
                    .expect("an unplaced step exists when no step progressed");
                return Err(StartupPlanRefusal::Cycle {
                    step: step.id,
                    declaring,
                });
            }
        }
        Ok(Self { steps })
    }

    /// The derived order.
    pub fn steps(&self) -> &[PlannedStep] {
        &self.steps
    }

    /// Whether no driver declared a startup step.
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    /// The derived step identities, in order.
    pub fn ids(&self) -> Vec<&'static str> {
        self.steps.iter().map(|step| step.id).collect()
    }
}

/// Why a declared startup plan cannot run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartupPlanRefusal {
    /// A step names an input no predecessor commits.
    MissingInput {
        /// The step that names it.
        step: &'static str,
        /// The driver that declared the step.
        declaring: WellKnownType,
        /// The uncommitted input.
        input: &'static str,
    },
    /// Two steps commit the same output, so the order is ambiguous.
    DuplicateOutput {
        /// The step that commits it second.
        step: &'static str,
        /// The driver that declared the step.
        declaring: WellKnownType,
        /// The doubly committed output.
        output: &'static str,
    },
    /// The declared inputs and outputs form a cycle.
    Cycle {
        /// A step on the cycle.
        step: &'static str,
        /// The driver that declared the step.
        declaring: WellKnownType,
    },
}

impl StartupPlanRefusal {
    /// The stable lower-kebab code for this refusal.
    pub const fn code(&self) -> &'static str {
        match self {
            Self::MissingInput { .. } => "startup-input-uncommitted",
            Self::DuplicateOutput { .. } => "startup-output-duplicated",
            Self::Cycle { .. } => "startup-cycle",
        }
    }
}

impl fmt::Display for StartupPlanRefusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for StartupPlanRefusal {}

/// Why one startup step failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StartupStepError {
    /// The closed failure code the executor reports.
    pub code: &'static str,
}

impl StartupStepError {
    /// Refuse one step with a closed code.
    pub const fn new(code: &'static str) -> Self {
        Self { code }
    }
}

impl fmt::Display for StartupStepError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code)
    }
}

impl std::error::Error for StartupStepError {}

/// The body of one declared startup step.
///
/// The composition root supplies the executor. A provider process that
/// declares steps and supplies none does not start: the base refuses with
/// `startup-executor-missing` rather than skipping obligations it cannot
/// perform.
#[async_trait]
pub trait StartupStepExecutor: Send + Sync {
    /// Execute one step, in the derived order.
    async fn execute(&self, step: &'static StartupStep) -> Result<(), StartupStepError>;
}
