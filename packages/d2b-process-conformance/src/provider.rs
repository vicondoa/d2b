//! The neutral Process Provider trait and its conformance profile.

use std::collections::BTreeSet;
use std::os::fd::OwnedFd;
use std::{future::Future, future::ready};

use d2b_contracts_resource::v3::execution_policy::{BoundedToken, ExecutionDomain};

use crate::error::ProcessConformanceError;
use crate::identity::{IdentityBinding, WaitReapOwner};
use crate::port::StopClass;
use crate::status::ProcessStatusReport;
use crate::ticket::LaunchTicket;

/// The fixed conformance profile of one Process Provider implementation.
///
/// Both shipped Providers implement identical ResourceTypes and identical
/// status and error conformance; the profile is the exact, declared set of
/// differences the shared suite is allowed to see.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessProviderProfile {
    provider: BoundedToken,
    wait_reap_owner: WaitReapOwner,
    supported_domains: BTreeSet<ExecutionDomain>,
    required_identity_bindings: BTreeSet<IdentityBinding>,
}

impl ProcessProviderProfile {
    /// Declare a profile.
    ///
    /// A Provider that supports no domain, or that requires no identity
    /// binding, is rejected: every Process Provider has a locally verified
    /// identity.
    pub fn new(
        provider: BoundedToken,
        wait_reap_owner: WaitReapOwner,
        supported_domains: BTreeSet<ExecutionDomain>,
        required_identity_bindings: BTreeSet<IdentityBinding>,
    ) -> Result<Self, ProcessConformanceError> {
        if supported_domains.is_empty() || required_identity_bindings.is_empty() {
            return Err(ProcessConformanceError::InvalidTicket);
        }
        Ok(Self {
            provider,
            wait_reap_owner,
            supported_domains,
            required_identity_bindings,
        })
    }

    /// Borrow the Provider name.
    pub const fn provider(&self) -> &BoundedToken {
        &self.provider
    }

    /// Return who owns `wait` and reap for this Provider.
    pub const fn wait_reap_owner(&self) -> WaitReapOwner {
        self.wait_reap_owner
    }

    /// Borrow the supported execution domains.
    pub const fn supported_domains(&self) -> &BTreeSet<ExecutionDomain> {
        &self.supported_domains
    }

    /// Borrow the identity bindings adoption must verify.
    pub const fn required_identity_bindings(&self) -> &BTreeSet<IdentityBinding> {
        &self.required_identity_bindings
    }
}

/// The outcome of an adoption attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdoptionOutcome {
    /// No process for this ticket is running.
    Absent,
    /// A running process was adopted after full identity verification.
    Adopted(ProcessStatusReport),
    /// One uniquely identified stale process is available for exact
    /// replacement through the effect port.
    Stale {
        /// The private effect-port evidence for the exact stale process.
        candidate: crate::port::AdoptionCandidate,
    },
    /// Identity was ambiguous. The process is quarantined and reported as
    /// `Unknown`; it is never broadly killed or reused.
    Quarantined(ProcessStatusReport),
}

/// The provider-neutral Process Provider controller surface.
pub trait ProcessProvider: Send + Sync {
    /// Borrow this Provider's declared conformance profile.
    fn profile(&self) -> &ProcessProviderProfile;

    /// Validate the ticket and launch through the injected effect port.
    fn launch(
        &self,
        ticket: &LaunchTicket,
    ) -> impl Future<Output = Result<ProcessStatusReport, ProcessConformanceError>> + Send;

    /// Launch with owned descriptors for a Provider-specific child bootstrap.
    ///
    /// Providers that do not own a descriptor-bearing launch path reject a
    /// non-empty vector by default, preserving the ordinary launch contract.
    fn launch_with_inherited_fds(
        &self,
        ticket: &LaunchTicket,
        inherited_fds: Vec<OwnedFd>,
    ) -> impl Future<Output = Result<ProcessStatusReport, ProcessConformanceError>> + Send {
        async move {
            if inherited_fds.is_empty() {
                self.launch(ticket).await
            } else {
                drop(inherited_fds);
                Err(ProcessConformanceError::InvalidTicket)
            }
        }
    }

    /// Re-establish ownership of an already running process after a
    /// controller restart, verifying identity before any pidfd is opened.
    fn adopt(
        &self,
        ticket: &LaunchTicket,
    ) -> impl Future<Output = Result<AdoptionOutcome, ProcessConformanceError>> + Send;

    /// Stop exactly one verified process identity.
    ///
    /// The default is deliberately unavailable: a Provider must opt into the
    /// provider-specific stop proof rather than silently pretending that a
    /// generic signal completed teardown.
    fn stop(
        &self,
        _identity: &crate::identity::ProcessIdentityDigest,
        _class: StopClass,
    ) -> impl Future<Output = Result<(), ProcessConformanceError>> + Send {
        ready(Err(ProcessConformanceError::StopUnavailable))
    }

    /// Stop and reap one uniquely identified stale process before replacement.
    ///
    /// Providers must keep this path narrow: the candidate's exact identity
    /// evidence is supplied by the effect adapter, and ambiguity is never
    /// converted into a stop request.
    fn stop_stale(
        &self,
        _candidate: &crate::port::AdoptionCandidate,
    ) -> impl Future<Output = Result<(), ProcessConformanceError>> + Send {
        ready(Err(ProcessConformanceError::StopUnavailable))
    }
}
