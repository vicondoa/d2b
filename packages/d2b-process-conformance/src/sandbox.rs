//! Provider-neutral semantic sandbox compilation.
//!
//! Providers receive a digest of the compiled plan, never raw minijail
//! arguments, systemd properties, paths, capabilities, or environment
//! values.  The fixed effect adapter owns the implementation plan.

use d2b_contracts_resource::v3::execution_policy::BoundedToken;
use d2b_contracts_resource::v3::execution_policy_resource::AdmittedExecution;
use d2b_contracts_resource::v3::process::{
    CapabilityClass, EnvironmentClass, MappingClass, NamespaceClass, SandboxSpec,
    UserNamespaceSpec,
};
use d2b_contracts_resource::v3::{
    canonical_digest, canonical_json_bytes, execution_policy::ExecutionDomain,
};

use crate::{ConfigurationDigest, ProcessConformanceError, identity::WaitReapOwner};

/// A compiled semantic sandbox plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompiledSandbox {
    digest: ConfigurationDigest,
    domain: ExecutionDomain,
}

/// The compiled semantic plan retained by a launch ticket so the privileged
/// adapter can enforce every public sandbox requirement, not just its digest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxPlan {
    compiled: CompiledSandbox,
    spec: SandboxSpec,
}

impl SandboxPlan {
    /// Construct a plan from the validated public specification.
    pub fn new(spec: &SandboxSpec, compiled: CompiledSandbox) -> Self {
        Self {
            compiled,
            spec: spec.clone(),
        }
    }

    /// Borrow the compiled digest and domain binding.
    pub const fn compiled(&self) -> &CompiledSandbox {
        &self.compiled
    }

    /// Borrow every semantic sandbox requirement.
    pub const fn spec(&self) -> &SandboxSpec {
        &self.spec
    }

    /// Build the compiled sandbox from one admitted execution.
    ///
    /// This is the one policy path a Process Provider uses. The effective
    /// namespaces, capabilities, identity, root restrictions, filter, umask,
    /// and budget are the [`AdmittedExecution`]'s - the result of the
    /// `ExecutionPolicy` contract evaluating the instance against an authorized
    /// policy and the selected backend's declared support - so the provider
    /// cannot be handed a posture the policy did not admit, and the same
    /// admission drives a long-running and a run-to-completion instance
    /// (R25, R26, AE28).
    ///
    /// A `SandboxSpec` on the caller's side is a *declared requirement*, not an
    /// authority: a provider that compiled one directly would be reading a
    /// row-authored posture, which is the second authority this replaces.
    ///
    /// # Errors
    ///
    /// Returns [`ProcessConformanceError::SandboxRejected`] when the admitted
    /// execution's canonical rendering fails, which is the only failure this
    /// step can have: the policy contract already refused everything else.
    pub fn from_admitted_execution(
        execution: &AdmittedExecution,
        domain: ExecutionDomain,
        provider_allows_root: bool,
    ) -> Result<Self, ProcessConformanceError> {
        let spec = admitted_execution_spec(execution, provider_allows_root)?;
        let compiled = SandboxCompiler.compile(&spec, domain, provider_allows_root)?;
        Ok(Self::new(&spec, compiled))
    }
}

impl CompiledSandbox {
    /// Borrow the opaque compiled-plan digest.
    pub const fn digest(&self) -> ConfigurationDigest {
        self.digest
    }

    /// Return the resolved execution domain.
    pub const fn domain(&self) -> ExecutionDomain {
        self.domain
    }
}

/// The provider-neutral semantic sandbox compiler.
#[derive(Debug, Clone, Copy, Default)]
pub struct SandboxCompiler;

impl SandboxCompiler {
    /// Compile one public SandboxSpec into an opaque digest.
    ///
    /// # Errors
    ///
    /// Returns [`ProcessConformanceError::SandboxRejected`] when the spec
    /// starts as root in a user domain or against a provider that does not
    /// allow root, or when its canonical JSON rendering fails.
    pub fn compile(
        &self,
        sandbox: &SandboxSpec,
        domain: ExecutionDomain,
        provider_allows_root: bool,
    ) -> Result<CompiledSandbox, ProcessConformanceError> {
        if sandbox.start_root() && (!provider_allows_root || domain == ExecutionDomain::User) {
            return Err(ProcessConformanceError::SandboxRejected);
        }

        let bytes =
            canonical_json_bytes(sandbox).map_err(|_| ProcessConformanceError::SandboxRejected)?;
        let mut input = Vec::with_capacity(bytes.len() + 1);
        input.push(match domain {
            ExecutionDomain::System => 0,
            ExecutionDomain::User => 1,
        });
        input.extend_from_slice(&bytes);
        let rendered = canonical_digest("d2b:v3:sandbox-plan", &input);
        let mut digest = [0_u8; 32];
        for (index, pair) in rendered.as_bytes()[7..].chunks_exact(2).enumerate() {
            digest[index] = (hex(pair[0]) << 4) | hex(pair[1]);
        }
        Ok(CompiledSandbox {
            digest: ConfigurationDigest::from_bytes(digest),
            domain,
        })
    }

    /// Compile and retain the typed semantic plan for privileged admission.
    pub fn compile_plan(
        &self,
        sandbox: &SandboxSpec,
        domain: ExecutionDomain,
        provider_allows_root: bool,
    ) -> Result<SandboxPlan, ProcessConformanceError> {
        let compiled = self.compile(sandbox, domain, provider_allows_root)?;
        Ok(SandboxPlan::new(sandbox, compiled))
    }
}

/// Project one admitted execution onto the public `SandboxSpec` a backend
/// compiler consumes.
///
/// The projection is total and widening-free: every field is the admitted
/// execution's own value, so the compiled plan cannot ask for a class, a
/// capability, or a root posture the `ExecutionPolicy` contract did not admit
/// for this instance (R25, R27, R50). `start_root` is the one derived fact -
/// an admitted execution that resolves no `User` is the only shape a system
/// domain root launch can take, and the caller's `provider_allows_root` still
/// has the final say in [`SandboxCompiler::compile`].
fn admitted_execution_spec(
    execution: &AdmittedExecution,
    provider_allows_root: bool,
) -> Result<SandboxSpec, ProcessConformanceError> {
    let namespace_classes: Vec<NamespaceClass> = execution.namespace_classes().to_vec();
    let capability_classes: Vec<CapabilityClass> = execution.capability_classes().to_vec();
    let seccomp_class = BoundedToken::parse(match execution.seccomp_profile_ref() {
        Some(_) => "profile",
        None => "off",
    })
    .map_err(|_| ProcessConformanceError::SandboxRejected)?;
    let user_namespace_requested = namespace_classes.contains(&NamespaceClass::User);
    let environment_class = if user_namespace_requested {
        EnvironmentClass::Minimal
    } else {
        EnvironmentClass::SafeInherited
    };
    let user_namespace = user_namespace_requested.then_some(UserNamespaceSpec {
        mapping_class: MappingClass::ProcessPrincipalRoot,
    });
    let umask = execution.umask().map(|value| format!("{value:04o}"));
    SandboxSpec::new(
        namespace_classes,
        capability_classes,
        seccomp_class,
        execution.no_new_privileges(),
        provider_allows_root
            && execution.user_ref().is_none()
            && !user_namespace_requested,
        environment_class,
        execution.no_new_privileges(),
        umask,
        0,
        user_namespace,
    )
    .map_err(|_| ProcessConformanceError::SandboxRejected)
}

fn hex(value: u8) -> u8 {
    match value {
        b'0'..=b'9' => value - b'0',
        b'a'..=b'f' => value - b'a' + 10,
        _ => 0,
    }
}

/// Proofs required before a Process finalizer may clear its row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StopProof {
    /// Exact-main SIGTERM or equivalent was delivered to the verified owner.
    pub exact_main_signaled: bool,
    /// The owning broker parent supplied a wait/reap result.
    pub broker_reaped: bool,
    /// The anchored cgroup leaf reports `populated 0`.
    pub cgroup_empty: bool,
    /// The systemd manager supplied a terminal unit transition.
    pub manager_terminal: bool,
}

/// Validate provider-specific intentional stop proofs.
pub fn validate_stop_proof(
    owner: WaitReapOwner,
    proof: StopProof,
) -> Result<(), ProcessConformanceError> {
    if !proof.exact_main_signaled {
        return Err(ProcessConformanceError::StopProofMissing);
    }
    match owner {
        WaitReapOwner::Local if !proof.broker_reaped || !proof.cgroup_empty => {
            Err(ProcessConformanceError::StopProofMissing)
        }
        WaitReapOwner::ServiceManager if !proof.manager_terminal || !proof.cgroup_empty => {
            Err(ProcessConformanceError::StopProofMissing)
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts_resource::v3::process::{EnvironmentClass, SandboxSpec};

    #[test]
    fn sandbox_digest_is_opaque_and_domain_bound() {
        let compiler = SandboxCompiler;
        let system = compiler
            .compile(&SandboxSpec::default(), ExecutionDomain::System, false)
            .unwrap();
        let user = compiler
            .compile(&SandboxSpec::default(), ExecutionDomain::User, false)
            .unwrap();
        assert_ne!(system.digest(), user.digest());
        assert_eq!(
            format!("{:?}", system.digest()),
            "ConfigurationDigest(<redacted>)"
        );
    }

    #[test]
    fn root_and_stop_proofs_fail_closed() {
        let sandbox = SandboxSpec::new(
            Vec::new(),
            Vec::new(),
            d2b_contracts_resource::v3::execution_policy::BoundedToken::parse("strict").unwrap(),
            true,
            true,
            EnvironmentClass::Minimal,
            true,
            None,
            0,
            None,
        )
        .unwrap();
        assert_eq!(
            SandboxCompiler
                .compile(&sandbox, ExecutionDomain::System, false)
                .unwrap_err(),
            ProcessConformanceError::SandboxRejected
        );
        assert_eq!(
            validate_stop_proof(WaitReapOwner::Local, StopProof::default()).unwrap_err(),
            ProcessConformanceError::StopProofMissing
        );
    }
}
