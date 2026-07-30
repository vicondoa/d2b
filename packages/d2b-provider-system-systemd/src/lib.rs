//! The `system-systemd` Process Provider controller.
//!
//! A process is a **non-forking transient system unit or scope**, or - for
//! the user domain - a verified transient user scope created through the
//! fixed user supervisor. Identity is the unit InvocationID bound together
//! with the cgroup, the unit main process, that process's start time, and
//! the Provider, template, and generation triple. A unit name alone is
//! never identity, so it is neither an identity binding nor public status.
//! systemd owns `wait` and reap; this Provider holds only a locally
//! verified pidfd.
//!
//! Adapted from the existing unsafe-local helper's `VerifiedScope`
//! (`packages/d2b-unsafe-local-helper/src/systemd.rs`), which already binds
//! a transient user scope by unit, InvocationID, and ControlGroup and fails
//! closed on a mismatch, and from the guest exec runner's non-forking
//! `systemd-run` transient-unit launch
//! (`packages/d2b-guestd/src/detached.rs`).
//!
//! This crate performs no privileged mutation: it opens no D-Bus or systemd
//! socket, spawns no process, and resolves no unit name or path. It
//! validates the ticket and calls the injected
//! [`ProcessLaunchEffectPort`], which the fixed core process effect adapter
//! implements.

#![deny(missing_docs)]

use std::collections::BTreeSet;

use d2b_contracts::v3::execution_policy::{BoundedToken, ExecutionDomain};
use d2b_process_conformance::{
    AdoptionCondition, AdoptionOutcome, IdentityBinding, LaunchTicket, ProcessConformanceError,
    ProcessLaunchEffectPort, ProcessPhaseClass, ProcessProvider, ProcessProviderProfile,
    ProcessStatusReport, WaitReapOwner,
};

/// The Provider name this controller implements.
pub const PROVIDER_NAME: &str = "system-systemd";

/// The `system-systemd` Process Provider controller.
#[derive(Debug)]
pub struct SystemdProcessProvider<P: ProcessLaunchEffectPort> {
    port: P,
    profile: ProcessProviderProfile,
}

impl<P: ProcessLaunchEffectPort> SystemdProcessProvider<P> {
    /// Build the controller over an injected process effect port.
    ///
    /// The profile is fixed: both execution domains are supported, because
    /// the user domain is served by a verified transient user scope through
    /// the fixed user supervisor.
    pub fn new(port: P) -> Self {
        let profile = ProcessProviderProfile::new(
            BoundedToken::parse(PROVIDER_NAME).expect("the frozen provider name is a valid token"),
            WaitReapOwner::ServiceManager,
            BTreeSet::from([ExecutionDomain::System, ExecutionDomain::User]),
            BTreeSet::from([
                IdentityBinding::UnitInvocationId,
                IdentityBinding::Cgroup,
                IdentityBinding::UnitMainPid,
                IdentityBinding::ProcessStartTime,
                IdentityBinding::Template,
                IdentityBinding::Generation,
            ]),
        )
        .expect("the frozen system-systemd profile is well formed");
        Self { port, profile }
    }

    /// Borrow the injected effect port.
    pub const fn port(&self) -> &P {
        &self.port
    }

    fn validate(&self, ticket: &LaunchTicket) -> Result<(), ProcessConformanceError> {
        if ticket.selected_provider().as_str() != PROVIDER_NAME {
            return Err(ProcessConformanceError::ProviderMismatch);
        }
        if !self.profile.supported_domains().contains(&ticket.domain()) {
            return Err(ProcessConformanceError::DomainNotSupported);
        }
        // A user-domain process runs in a verified transient user scope, so
        // the exact identity must already be resolved on the ticket.
        if ticket.domain() == ExecutionDomain::User && ticket.user_ref().is_none() {
            return Err(ProcessConformanceError::UserRefRequired);
        }
        Ok(())
    }

    fn report(
        &self,
        ticket: &LaunchTicket,
        identity: d2b_process_conformance::ProcessIdentityDigest,
        phase: ProcessPhaseClass,
        adoption: AdoptionCondition,
    ) -> ProcessStatusReport {
        ProcessStatusReport {
            provider: self.profile.provider().clone(),
            identity,
            wait_reap_owner: self.profile.wait_reap_owner(),
            execution_ref: ticket.execution_ref().clone(),
            domain: ticket.domain(),
            user_ref: ticket.user_ref().cloned(),
            digests: *ticket.digests(),
            phase,
            last_exit: None,
            adoption,
        }
    }
}

impl<P: ProcessLaunchEffectPort> ProcessProvider for SystemdProcessProvider<P> {
    fn profile(&self) -> &ProcessProviderProfile {
        &self.profile
    }

    async fn launch(
        &self,
        ticket: &LaunchTicket,
    ) -> Result<ProcessStatusReport, ProcessConformanceError> {
        self.validate(ticket)?;
        let launched = self.port.launch(ticket).await?;
        if launched.wait_reap_owner != WaitReapOwner::ServiceManager {
            return Err(ProcessConformanceError::WaitOwnerMismatch);
        }
        if !launched
            .observed
            .covers(self.profile.required_identity_bindings())
        {
            return Err(ProcessConformanceError::IdentityUnverified);
        }
        Ok(self.report(
            ticket,
            launched.identity,
            ProcessPhaseClass::Running,
            AdoptionCondition::NotApplicable,
        ))
    }

    async fn adopt(
        &self,
        ticket: &LaunchTicket,
    ) -> Result<AdoptionOutcome, ProcessConformanceError> {
        self.validate(ticket)?;
        let Some(candidate) = self.port.observe(ticket).await? else {
            return Ok(AdoptionOutcome::Absent);
        };
        // Revalidate every stable identity property before the pidfd is
        // opened. Ambiguity quarantines; it never broadly kills or reuses.
        let identity_ok = candidate.wait_reap_owner == WaitReapOwner::ServiceManager
            && candidate
                .observed
                .covers(self.profile.required_identity_bindings());
        if !identity_ok {
            return Ok(AdoptionOutcome::Quarantined(self.report(
                ticket,
                candidate.identity,
                ProcessPhaseClass::Unknown,
                AdoptionCondition::Quarantined,
            )));
        }
        let _pidfd = self.port.open_pidfd(&candidate).await?;
        Ok(AdoptionOutcome::Adopted(self.report(
            ticket,
            candidate.identity,
            ProcessPhaseClass::Running,
            AdoptionCondition::Adopted,
        )))
    }
}
