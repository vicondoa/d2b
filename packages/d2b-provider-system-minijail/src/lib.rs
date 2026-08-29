//! The `system-minijail` Process Provider controller.
//!
//! The controller calls the `ProcessLaunchEffectPort` with the resource UID
//! and the compiled sandbox and resource digests; the effect adapter
//! resolves the plan and **the broker** performs the spawn, preferring
//! `clone3(CLONE_PIDFD)` so the process is born directly in its final
//! cgroup. d2b owns `wait` and reap. Adoption verifies pid, process start
//! time, cgroup, executable, template, and generation *before* any
//! `pidfd_open`; ambiguity yields Unknown and quarantine, never a broad
//! kill or a reuse.
//!
//! Adapted from the existing broker `SpawnRunner` plan and pidfd path
//! (`packages/d2b-broker/src/ops/spawn_runner.rs` and its
//! `clone3_pidfd_or_fork_fallback`) and from the daemon supervisor's pidfd
//! table (`packages/d2bd/src/supervisor/pidfd_table.rs`), whose
//! `(pid, start_time_ticks)` recheck is exactly the pid-reuse guard modelled
//! here as the [`IdentityBinding::Pid`] plus
//! [`IdentityBinding::ProcessStartTime`] pair.
//!
//! There is no direct Provider broker access: this crate never imports the
//! broker crate, receives a broker socket or DTO, or issues `clone3` or
//! `pidfd_open` itself.

#![deny(missing_docs)]

pub mod adoption;
pub mod effect_port;
pub mod effect_result;
pub mod ephemeral;
pub mod finalize;
pub mod launch;
pub mod manifest;
pub mod sandbox_compiler;
pub mod user_ns;

use std::collections::BTreeSet;

use d2b_contracts_resource::v3::execution_policy::{BoundedToken, ExecutionDomain};
use d2b_process_conformance::{
    AdoptionCandidate, AdoptionCondition, AdoptionOutcome, CancellationBinding, IdentityBinding,
    LaunchTicket, LaunchedProcess, ProcessConformanceError, ProcessIdentityDigest,
    ProcessLaunchEffectPort, ProcessPhaseClass, ProcessProvider, ProcessProviderProfile,
    ProcessStatusReport, ReadinessExpectation, StopClass, WaitReapOwner,
};

/// The Provider name this controller implements.
pub const PROVIDER_NAME: &str = "system-minijail";

/// The `system-minijail` Process Provider controller.
#[derive(Debug)]
pub struct MinijailProcessProvider<P: ProcessLaunchEffectPort> {
    port: P,
    profile: ProcessProviderProfile,
    platform_gate: Option<launch::PlatformGate>,
}

impl<P: ProcessLaunchEffectPort> MinijailProcessProvider<P> {
    /// Build the system-domain controller over an injected effect port.
    ///
    /// The user domain is **not** supported by default: `system-minijail`
    /// may support it only where its descriptor and conformance say so, so
    /// it is opted into explicitly through
    /// [`MinijailProcessProvider::with_user_domain`].
    pub fn new(port: P) -> Self {
        Self::with_user_domain(port, false)
    }

    /// Build the controller, declaring whether the Provider descriptor
    /// admits user-domain processes.
    pub fn with_user_domain(port: P, descriptor_admits_user_domain: bool) -> Self {
        Self::with_user_domain_and_platform_gate(port, descriptor_admits_user_domain, None)
    }

    /// Build the controller with an injected production platform snapshot.
    ///
    /// The plain constructors remain useful for conformance tests. Daemon
    /// composition must use this constructor so production launches are
    /// rejected unless the runtime kernel and delegated cgroup posture were
    /// actually observed.
    pub fn with_platform_gate(port: P, platform_gate: launch::PlatformGate) -> Self {
        Self::with_user_domain_and_platform_gate(port, false, Some(platform_gate))
    }

    /// Build the controller with an optional platform gate and user-domain
    /// descriptor admission.
    pub fn with_user_domain_and_platform_gate(
        port: P,
        descriptor_admits_user_domain: bool,
        platform_gate: Option<launch::PlatformGate>,
    ) -> Self {
        let mut domains = BTreeSet::from([ExecutionDomain::System]);
        if descriptor_admits_user_domain {
            domains.insert(ExecutionDomain::User);
        }
        let profile = ProcessProviderProfile::new(
            BoundedToken::parse(PROVIDER_NAME).expect("the frozen provider name is a valid token"),
            WaitReapOwner::Local,
            domains,
            BTreeSet::from([
                IdentityBinding::Pid,
                IdentityBinding::ProcessStartTime,
                IdentityBinding::Cgroup,
                IdentityBinding::Executable,
                IdentityBinding::Template,
                IdentityBinding::Generation,
            ]),
        )
        .expect("the frozen system-minijail profile is well formed");
        Self {
            port,
            profile,
            platform_gate,
        }
    }

    /// Borrow the injected effect port.
    pub const fn port(&self) -> &P {
        &self.port
    }

    fn validate(&self, ticket: &LaunchTicket) -> Result<(), ProcessConformanceError> {
        ticket.validate()?;
        if ticket.has_controller_launch_binding() {
            ticket.validate_controller_launch()?;
        }
        if ticket.has_assignment_binding() {
            ticket.validate_assignment()?;
        }
        if ticket.selected_provider().as_str() != PROVIDER_NAME {
            return Err(ProcessConformanceError::ProviderMismatch);
        }
        if !self.profile.supported_domains().contains(&ticket.domain()) {
            return Err(ProcessConformanceError::DomainNotSupported);
        }
        if ticket.operation().cancellation() == CancellationBinding::Cancelled {
            return Err(ProcessConformanceError::Cancelled);
        }
        if ticket.domain() == ExecutionDomain::User && ticket.user_ref().is_none() {
            return Err(ProcessConformanceError::UserRefRequired);
        }
        if let Some(gate) = self.platform_gate {
            launch::validate_launch_ticket(ticket, gate)?;
        }
        Ok(())
    }

    async fn cleanup_failed_launch(
        &self,
        launched: &LaunchedProcess,
        error: ProcessConformanceError,
    ) -> ProcessConformanceError {
        if launched.identity.is_zero() {
            return error;
        }
        match self
            .port
            .stop(&launched.identity, StopClass::Terminate)
            .await
        {
            Ok(()) => error,
            Err(_) => ProcessConformanceError::StopUnavailable,
        }
    }

    async fn readiness_phase(
        &self,
        ticket: &LaunchTicket,
        identity: ProcessIdentityDigest,
    ) -> Result<ProcessPhaseClass, ProcessConformanceError> {
        match ticket.readiness() {
            ReadinessExpectation::None => Ok(ProcessPhaseClass::Running),
            ReadinessExpectation::Condition { .. } => {
                // The fixed adapter's probe is the readiness observation;
                // it does not open or retain another pidfd.
                let Some(candidate) = self.port.probe(ticket).await? else {
                    return Err(ProcessConformanceError::DeadlineExceeded);
                };
                if !self.candidate_matches(ticket, &candidate, identity) {
                    return Err(ProcessConformanceError::AdoptionAmbiguous);
                }
                Ok(ProcessPhaseClass::Ready)
            }
        }
    }

    fn candidate_matches(
        &self,
        ticket: &LaunchTicket,
        candidate: &AdoptionCandidate,
        identity: ProcessIdentityDigest,
    ) -> bool {
        candidate.identity == identity
            && candidate.wait_reap_owner == WaitReapOwner::Local
            && candidate
                .validate(self.profile.required_identity_bindings())
                .is_ok()
            && ticket
                .validate_process_identity(&candidate.identity)
                .is_ok()
    }

    fn report(
        &self,
        ticket: &LaunchTicket,
        identity: ProcessIdentityDigest,
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

impl<P: ProcessLaunchEffectPort> ProcessProvider for MinijailProcessProvider<P> {
    fn profile(&self) -> &ProcessProviderProfile {
        &self.profile
    }

    async fn launch(
        &self,
        ticket: &LaunchTicket,
    ) -> Result<ProcessStatusReport, ProcessConformanceError> {
        self.validate(ticket)?;
        let launched = self.port.launch(ticket).await?;
        // d2b owns wait and reap for every minijail-launched process.
        if launched.wait_reap_owner != WaitReapOwner::Local {
            return Err(ProcessConformanceError::WaitOwnerMismatch);
        }
        launched.validate(self.profile.required_identity_bindings())?;
        ticket.validate_process_identity(&launched.identity)?;
        match self.readiness_phase(ticket, launched.identity).await {
            Ok(phase) => Ok(self.report(
                ticket,
                launched.identity,
                phase,
                AdoptionCondition::NotApplicable,
            )),
            Err(error) => Err(self.cleanup_failed_launch(&launched, error).await),
        }
    }

    async fn adopt(
        &self,
        ticket: &LaunchTicket,
    ) -> Result<AdoptionOutcome, ProcessConformanceError> {
        self.validate(ticket)?;
        let Some(candidate) = self.port.observe(ticket).await? else {
            return Ok(AdoptionOutcome::Absent);
        };
        if adoption::is_stale_candidate(ticket, &candidate, &self.profile) {
            return Ok(AdoptionOutcome::Stale { candidate });
        }
        let identity_ok = candidate.wait_reap_owner == WaitReapOwner::Local
            && candidate
                .validate(self.profile.required_identity_bindings())
                .is_ok()
            && ticket
                .validate_process_identity(&candidate.identity)
                .is_ok();
        if !identity_ok {
            return Ok(AdoptionOutcome::Quarantined(self.report(
                ticket,
                candidate.identity,
                ProcessPhaseClass::Unknown,
                AdoptionCondition::Quarantined,
            )));
        }
        let phase = match self.readiness_phase(ticket, candidate.identity).await {
            Ok(phase) => phase,
            Err(_) => {
                return Ok(AdoptionOutcome::Quarantined(self.report(
                    ticket,
                    candidate.identity,
                    ProcessPhaseClass::Unknown,
                    AdoptionCondition::Quarantined,
                )));
            }
        };
        let _pidfd = self.port.open_pidfd(&candidate).await?;
        Ok(AdoptionOutcome::Adopted(self.report(
            ticket,
            candidate.identity,
            phase,
            AdoptionCondition::Adopted,
        )))
    }

    async fn stop(
        &self,
        identity: &ProcessIdentityDigest,
        class: StopClass,
    ) -> Result<(), ProcessConformanceError> {
        if identity.is_zero() {
            return Err(ProcessConformanceError::IdentityUnverified);
        }
        self.port.stop(identity, class).await
    }

    async fn stop_stale(
        &self,
        candidate: &AdoptionCandidate,
    ) -> Result<(), ProcessConformanceError> {
        if candidate.identity.is_zero() {
            return Err(ProcessConformanceError::IdentityUnverified);
        }
        self.port.open_pidfd(candidate).await?;
        self.port
            .stop(&candidate.identity, StopClass::Terminate)
            .await
    }
}
