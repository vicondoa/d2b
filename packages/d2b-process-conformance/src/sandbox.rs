//! Provider-neutral semantic sandbox compilation.
//!
//! Providers receive a digest of the compiled plan, never raw minijail
//! arguments, systemd properties, paths, capabilities, or environment
//! values.  The fixed effect adapter owns the implementation plan.

use d2b_contracts::v3::{
    canonical_digest, canonical_json_bytes, execution_policy::ExecutionDomain, process::SandboxSpec,
};

use crate::{ConfigurationDigest, ProcessConformanceError, identity::WaitReapOwner};

/// A compiled semantic sandbox plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompiledSandbox {
    digest: ConfigurationDigest,
    domain: ExecutionDomain,
    requires_cgroup_kill: bool,
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

    /// Whether intentional teardown needs the cgroup.kill proof.
    pub const fn requires_cgroup_kill(&self) -> bool {
        self.requires_cgroup_kill
    }
}

/// The provider-neutral semantic sandbox compiler.
#[derive(Debug, Clone, Copy, Default)]
pub struct SandboxCompiler;

impl SandboxCompiler {
    /// Compile one public SandboxSpec into an opaque digest.
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
            requires_cgroup_kill: true,
        })
    }
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
    use d2b_contracts::v3::process::{EnvironmentClass, SandboxSpec};

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
            d2b_contracts::v3::execution_policy::BoundedToken::parse("strict").unwrap(),
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
