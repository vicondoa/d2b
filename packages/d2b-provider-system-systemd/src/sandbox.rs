//! Semantic SandboxSpec validation for the systemd Process Provider.

use d2b_contracts::v3::{execution_policy::ExecutionDomain, process::SandboxSpec};
use d2b_process_conformance::{CompiledSandbox, ProcessConformanceError, SandboxCompiler};

/// The systemd Provider's semantic sandbox compiler.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemdSandboxCompiler {
    inner: SandboxCompiler,
}

impl SystemdSandboxCompiler {
    /// Compile a public SandboxSpec without constructing a unit fragment.
    pub fn compile(
        &self,
        spec: &SandboxSpec,
        domain: ExecutionDomain,
    ) -> Result<CompiledSandbox, ProcessConformanceError> {
        self.inner.compile(spec, domain, false)
    }
}
