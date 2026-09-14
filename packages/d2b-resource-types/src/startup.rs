//! Startup steps a driver contributes to the composition root.

/// One startup step of the composition root.
///
/// The composition root derives the execution order from the declared inputs
/// and outputs: a step runs only after every input it names has been
/// committed as an output of a predecessor. Startup fails when a step's input
/// is not committed by any predecessor, so a declaration cannot silently skip
/// a prerequisite.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StartupStep {
    /// The step identity, used for ordering diagnostics.
    pub id: &'static str,
    /// The inputs the step requires, each named by a predecessor's output.
    pub inputs: &'static [&'static str],
    /// The outputs the step commits for later steps.
    pub outputs: &'static [&'static str],
}
