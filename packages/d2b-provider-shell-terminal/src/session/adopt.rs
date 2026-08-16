//! Strict supervisor identity and restart-adoption decisions.

/// Opaque supervisor identity fields verified by the fixed process adapter.
#[derive(Clone, PartialEq, Eq)]
pub struct SupervisorIdentity {
    invocation_digest: [u8; 32],
    cgroup_digest: [u8; 32],
    generation: u64,
}

impl SupervisorIdentity {
    /// Construct a verified identity with a nonzero generation and digests.
    pub fn new(
        invocation_digest: [u8; 32],
        cgroup_digest: [u8; 32],
        generation: u64,
    ) -> Result<Self, crate::ShellTerminalError> {
        if invocation_digest == [0; 32] || cgroup_digest == [0; 32] || generation == 0 {
            return Err(crate::ShellTerminalError::SupervisorAmbiguous);
        }
        Ok(Self {
            invocation_digest,
            cgroup_digest,
            generation,
        })
    }

    /// Return the externally visible supervisor generation.
    pub const fn generation(&self) -> u64 {
        self.generation
    }
}

impl std::fmt::Debug for SupervisorIdentity {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SupervisorIdentity")
            .field("generation", &self.generation)
            .finish_non_exhaustive()
    }
}

/// One process candidate observed during controller restart.
#[derive(Clone)]
pub struct SupervisorCandidate {
    owner_session: String,
    identity: SupervisorIdentity,
}

impl SupervisorCandidate {
    /// Construct a candidate from an owner session and verified identity.
    pub fn new(owner_session: impl Into<String>, identity: SupervisorIdentity) -> Self {
        Self {
            owner_session: owner_session.into(),
            identity,
        }
    }
}

impl std::fmt::Debug for SupervisorCandidate {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SupervisorCandidate(<redacted>)")
    }
}

/// Controller decision after scanning supervisor candidates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdoptionDecision {
    /// Exactly one process proved ownership and identity.
    Adopted,
    /// No process exists for the session.
    Missing,
    /// The observed process belongs to an obsolete supervisor generation.
    StaleGeneration,
    /// More than one candidate could own the session.
    Ambiguous,
}

/// Decide whether controller restart may adopt one exact supervisor.
pub fn adopt_supervisor(
    session_name: &str,
    expected: &SupervisorIdentity,
    candidates: &[SupervisorCandidate],
) -> AdoptionDecision {
    let matching_owner: Vec<_> = candidates
        .iter()
        .filter(|candidate| candidate.owner_session == session_name)
        .collect();
    match matching_owner.as_slice() {
        [] => AdoptionDecision::Missing,
        [candidate] if candidate.identity == *expected => AdoptionDecision::Adopted,
        [candidate] if candidate.identity.generation() != expected.generation() => {
            AdoptionDecision::StaleGeneration
        }
        [_] => AdoptionDecision::Ambiguous,
        _ => AdoptionDecision::Ambiguous,
    }
}
