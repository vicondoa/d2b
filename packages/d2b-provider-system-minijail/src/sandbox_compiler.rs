//! Semantic SandboxSpec compilation for system-minijail.

use d2b_contracts::v3::{
    execution_policy::ExecutionDomain,
    process::{SandboxSpec, UserNamespaceSpec},
};
use d2b_process_conformance::{CompiledSandbox, ProcessConformanceError, SandboxCompiler};

use crate::user_ns::UserNamespacePlan;

/// The compiled minijail plan visible to the core effect adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MinijailSandboxPlan {
    /// Provider-neutral plan digest.
    pub compiled: CompiledSandbox,
    /// Optional semantic user-namespace mapping.
    pub user_namespace: Option<UserNamespacePlan>,
}

/// Compile semantic sandbox fields without producing minijail argv.
#[derive(Debug, Clone, Copy, Default)]
pub struct MinijailSandboxCompiler {
    inner: SandboxCompiler,
}

impl MinijailSandboxCompiler {
    /// Compile one SandboxSpec for a resolved domain.
    pub fn compile(
        &self,
        spec: &SandboxSpec,
        domain: ExecutionDomain,
    ) -> Result<MinijailSandboxPlan, ProcessConformanceError> {
        self.compile_with_root_authorization(spec, domain, false)
    }

    /// Compile after the signed Provider descriptor explicitly authorizes
    /// starting as in-namespace root.
    pub fn compile_with_root_authorization(
        &self,
        spec: &SandboxSpec,
        domain: ExecutionDomain,
        provider_allows_root: bool,
    ) -> Result<MinijailSandboxPlan, ProcessConformanceError> {
        let compiled = self.inner.compile(spec, domain, provider_allows_root)?;
        let user_namespace = spec
            .user_namespace()
            .map(UserNamespacePlan::from_spec)
            .transpose()?;
        Ok(MinijailSandboxPlan {
            compiled,
            user_namespace,
        })
    }
}

impl UserNamespacePlan {
    fn from_spec(spec: &UserNamespaceSpec) -> Result<Self, ProcessConformanceError> {
        Self::new(spec.mapping_class)
    }
}
