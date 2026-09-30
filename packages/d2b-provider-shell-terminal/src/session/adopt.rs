//! Strict supervisor identity and graph-bound restart-adoption decisions.
//!
//! Adoption is a measurement of live observation against one admitted
//! relationship, never a name comparison. A controller restart may only
//! retain a supervisor that proves all three of the exact `Process` row the
//! session owns, the exact `Endpoint` its terminal stream was admitted on,
//! and a reconnect generation that is not older than the relationship's
//! fence. Anything else is refused by decision rather than repaired by
//! assumption, because a wrong adoption would hand an interactive terminal to
//! a process the graph never admitted for it.

use d2b_contracts_resource::v3::ResourceRef;
use d2b_contracts_resource::v3::identity::ReconnectGeneration;

use crate::ShellSession;
use crate::service::TerminalStreamBinding;

/// Opaque supervisor identity fields verified by the fixed process adapter.
#[derive(Clone, PartialEq, Eq)]
pub struct SupervisorIdentity {
    invocation_digest: [u8; 32],
    cgroup_digest: [u8; 32],
    generation: u64,
}

impl SupervisorIdentity {
    /// Construct a verified identity with a nonzero generation and digests.
    ///
    /// # Errors
    ///
    /// Returns [`crate::ShellTerminalError::SupervisorAmbiguous`] when the
    /// invocation or cgroup digest is zero or the generation is zero.
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

/// The exact `Process` row and terminal `Endpoint` one supervisor observation
/// was made through.
///
/// The observation carries no authority of its own: it is the evidence the
/// graph measured, and [`adopt_supervisor`] decides it against the admitted
/// [`TerminalStreamBinding`] and the session's own supervisor `Process`.
#[derive(Clone, PartialEq, Eq)]
pub struct SupervisorObservation {
    process_ref: ResourceRef,
    endpoint_ref: ResourceRef,
    reconnect: ReconnectGeneration,
}

impl SupervisorObservation {
    /// Construct one observation from the row, endpoint, and reconnect
    /// generation the supervisor was seen through.
    pub const fn observed(
        process_ref: ResourceRef,
        endpoint_ref: ResourceRef,
        reconnect: ReconnectGeneration,
    ) -> Self {
        Self {
            process_ref,
            endpoint_ref,
            reconnect,
        }
    }

    /// Borrow the `Process` row the supervisor was observed as.
    pub const fn process_ref(&self) -> &ResourceRef {
        &self.process_ref
    }

    /// Borrow the exact `Endpoint` the terminal stream rides.
    pub const fn endpoint_ref(&self) -> &ResourceRef {
        &self.endpoint_ref
    }

    /// Return the reconnect generation the observation was made in.
    pub const fn reconnect(&self) -> ReconnectGeneration {
        self.reconnect
    }
}

impl std::fmt::Debug for SupervisorObservation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SupervisorObservation(<redacted>)")
    }
}

/// One process candidate observed during controller restart.
#[derive(Clone)]
pub struct SupervisorCandidate {
    owner_session: String,
    observation: SupervisorObservation,
    identity: SupervisorIdentity,
}

impl SupervisorCandidate {
    /// Construct a candidate from an owner session, the exact admitted
    /// relationship it was observed through, and its verified identity.
    pub fn observed(
        owner_session: impl Into<String>,
        observation: SupervisorObservation,
        identity: SupervisorIdentity,
    ) -> Self {
        Self {
            owner_session: owner_session.into(),
            observation,
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
    /// Exactly one process proved ownership, identity, Process row, and endpoint.
    Adopted,
    /// No process exists for the session.
    Missing,
    /// The observed process belongs to an obsolete supervisor generation.
    StaleGeneration,
    /// The observed process runs under a `Process` row the session does not own.
    ForeignProcess,
    /// The observed terminal stream is not the admitted `Endpoint`.
    ForeignEndpoint,
    /// The observation was made in a reconnect generation the fence excludes.
    StaleReconnect,
    /// More than one candidate could own the session.
    Ambiguous,
}

/// Decide whether controller restart may retain one exact supervisor.
///
/// The candidate is retained only when the single process claiming this session
/// is running as the session's own supervisor `Process`, is observed on the
/// admitted terminal `Endpoint`, was observed in a reconnect generation the
/// binding still admits, and carries exactly the expected identity. Every
/// other observation is a distinct refusal so a caller can report which
/// relationship failed rather than that "adoption was ambiguous".
pub fn adopt_supervisor(
    session: &ShellSession,
    stream: &TerminalStreamBinding,
    expected: &SupervisorIdentity,
    candidates: &[SupervisorCandidate],
) -> AdoptionDecision {
    let matching_owner: Vec<_> = candidates
        .iter()
        .filter(|candidate| candidate.owner_session == session.name())
        .collect();
    let [candidate] = matching_owner.as_slice() else {
        return if matching_owner.is_empty() {
            AdoptionDecision::Missing
        } else {
            AdoptionDecision::Ambiguous
        };
    };
    if candidate.observation.process_ref() != stream.consumer()
        || candidate.observation.process_ref() != session.supervisor_process_ref()
    {
        return AdoptionDecision::ForeignProcess;
    }
    if candidate.observation.endpoint_ref() != stream.endpoint() {
        return AdoptionDecision::ForeignEndpoint;
    }
    if candidate.observation.reconnect() < stream.minimum_reconnect() {
        return AdoptionDecision::StaleReconnect;
    }
    if candidate.identity == *expected {
        return AdoptionDecision::Adopted;
    }
    if candidate.identity.generation() != expected.generation() {
        AdoptionDecision::StaleGeneration
    } else {
        AdoptionDecision::Ambiguous
    }
}