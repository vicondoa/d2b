//! The `ProcessLaunchEffectPort` seam.
//!
//! A Process Provider validates semantics and calls this injected typed
//! port. It never imports the broker crate, receives a broker socket or
//! DTO, opens a systemd or D-Bus socket, resolves a host path, or issues
//! `clone3` or `pidfd_open`. The fixed core effect adapter
//! (ProviderSupervisor) alone maps each call onto the broker or the systemd
//! effect owner, and the broker stays the sole privileged executor and
//! independent audit owner of the mutation.

use std::future::Future;

use crate::error::ProcessConformanceError;
use crate::identity::{ObservedIdentity, PidfdEvidence, ProcessIdentityDigest, WaitReapOwner};
use crate::ticket::LaunchTicket;

/// What the effect adapter returns from a successful launch: the
/// provider-specific stable process identity, the identity properties it
/// verified, mandatory pidfd evidence, and the wait and reap owner.
#[derive(Debug)]
pub struct LaunchedProcess {
    /// The opaque stable process identity.
    pub identity: ProcessIdentityDigest,
    /// The identity properties the adapter verified.
    pub observed: ObservedIdentity,
    /// Proof that a verified pidfd is held locally.
    pub pidfd: PidfdEvidence,
    /// Who calls `wait` and reaps this process.
    pub wait_reap_owner: WaitReapOwner,
}

impl LaunchedProcess {
    /// Validate the effect adapter's launch evidence.
    pub fn validate(
        &self,
        required: &std::collections::BTreeSet<crate::identity::IdentityBinding>,
    ) -> Result<(), ProcessConformanceError> {
        if self.identity.is_zero() {
            return Err(ProcessConformanceError::IdentityUnverified);
        }
        if !self.observed.covers(required) {
            return Err(ProcessConformanceError::IdentityUnverified);
        }
        Ok(())
    }
}

/// A process the adapter found still running for this ticket, before any
/// pidfd is opened for it.
///
/// Adoption verifies identity from this observation first; a pidfd is
/// opened only afterwards, and only when every required binding is
/// covered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdoptionCandidate {
    /// The opaque stable identity the adapter derived.
    pub identity: ProcessIdentityDigest,
    /// The identity properties the adapter verified for the candidate.
    pub observed: ObservedIdentity,
    /// Who owns `wait` and reap for the candidate.
    pub wait_reap_owner: WaitReapOwner,
}

impl AdoptionCandidate {
    /// Validate the candidate before a pidfd may be opened.
    pub fn validate(
        &self,
        required: &std::collections::BTreeSet<crate::identity::IdentityBinding>,
    ) -> Result<(), ProcessConformanceError> {
        if self.identity.is_zero() {
            return Err(ProcessConformanceError::IdentityUnverified);
        }
        if !self.observed.covers(required) {
            return Err(ProcessConformanceError::IdentityUnverified);
        }
        Ok(())
    }
}

/// How a stop request is delivered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum StopClass {
    /// Ask the process to drain within its configured drain timeout.
    Drain,
    /// Terminate the exact identity after the drain timeout elapsed.
    Terminate,
}

/// The typed async effect port for the Process domain.
///
/// Implemented only by the fixed core process effect adapter and by test
/// doubles. Every method acts on exactly one process identity; there is no
/// broad sweep, no reuse, and no operation that names anything but the
/// ticket and the derived opaque identity.
pub trait ProcessLaunchEffectPort: Send + Sync {
    /// Launch the ticket's process and return its verified identity and
    /// mandatory pidfd evidence.
    fn launch(
        &self,
        ticket: &LaunchTicket,
    ) -> impl Future<Output = Result<LaunchedProcess, ProcessConformanceError>> + Send;

    /// Observe whether a process for this ticket is already running,
    /// without opening a pidfd for it.
    fn observe(
        &self,
        ticket: &LaunchTicket,
    ) -> impl Future<Output = Result<Option<AdoptionCandidate>, ProcessConformanceError>> + Send;

    /// Open a verified pidfd for a candidate whose identity the caller has
    /// already fully verified.
    fn open_pidfd(
        &self,
        candidate: &AdoptionCandidate,
    ) -> impl Future<Output = Result<PidfdEvidence, ProcessConformanceError>> + Send;

    /// Stop exactly the named identity.
    fn stop(
        &self,
        identity: &ProcessIdentityDigest,
        class: StopClass,
    ) -> impl Future<Output = Result<(), ProcessConformanceError>> + Send;
}
