//! Typed Process-conformance lifecycle for session supervisors.

use std::collections::BTreeSet;

use crate::ShellSession;
use d2b_contracts::v3::{
    ResourceRef,
    execution_policy::{BoundedToken, ExecutionDomain},
};
use d2b_process_conformance::{
    AdoptionCondition, AdoptionOutcome, CancellationBinding, IdentityBinding, LaunchTicket,
    ProcessConformanceError, ProcessIdentityDigest, ProcessLaunchEffectPort, ProcessPhaseClass,
    ProcessProvider, ProcessProviderProfile, ProcessStatusReport, StopClass, WaitReapOwner,
};

/// User-domain supervisor lifecycle delegated to `Provider/system-systemd`.
///
/// This type validates only the semantic process request and calls a typed
/// effect port. It neither opens a broker or system-manager connection nor
/// receives raw process identifiers, credentials, paths, or descriptors.
pub struct SupervisorProcessLifecycle<P: ProcessLaunchEffectPort> {
    port: P,
    profile: ProcessProviderProfile,
    execution_ref: ResourceRef,
    user_ref: ResourceRef,
}

impl<P: ProcessLaunchEffectPort> std::fmt::Debug for SupervisorProcessLifecycle<P> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SupervisorProcessLifecycle(<redacted>)")
    }
}

impl<P: ProcessLaunchEffectPort> SupervisorProcessLifecycle<P> {
    /// Construct a workload-user supervisor lifecycle bound to one session.
    pub fn for_session(port: P, session: &ShellSession) -> Self {
        let profile = ProcessProviderProfile::new(
            BoundedToken::parse("system-systemd")
                .expect("the fixed system-systemd provider token is valid"),
            WaitReapOwner::ServiceManager,
            BTreeSet::from([ExecutionDomain::User]),
            BTreeSet::from([
                IdentityBinding::UnitInvocationId,
                IdentityBinding::Cgroup,
                IdentityBinding::UnitMainPid,
                IdentityBinding::ProcessStartTime,
                IdentityBinding::Template,
                IdentityBinding::Generation,
            ]),
        )
        .expect("the fixed supervisor process profile is valid");
        let target_type = if session.execution_target().is_host() {
            "Host"
        } else {
            "Guest"
        };
        let execution_ref = ResourceRef::parse(&format!(
            "{target_type}/{}",
            session.execution_target().name()
        ))
        .expect("validated ShellSession target has a valid resource reference");
        let user_ref = ResourceRef::parse(&format!("User/{}", session.workload_user()))
            .expect("validated ShellSession workload user has a valid resource reference");
        Self {
            port,
            profile,
            execution_ref,
            user_ref,
        }
    }

    fn validate(&self, ticket: &LaunchTicket) -> Result<(), ProcessConformanceError> {
        if ticket.selected_provider().as_str() != "system-systemd" {
            return Err(ProcessConformanceError::ProviderMismatch);
        }
        if ticket.domain() != ExecutionDomain::User || ticket.user_ref().is_none() {
            return Err(ProcessConformanceError::UserRefRequired);
        }
        if ticket.execution_ref() != &self.execution_ref
            || ticket.user_ref() != Some(&self.user_ref)
        {
            return Err(ProcessConformanceError::InvalidTicket);
        }
        if ticket.operation().cancellation() == CancellationBinding::Cancelled {
            return Err(ProcessConformanceError::Cancelled);
        }
        Ok(())
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
            wait_reap_owner: WaitReapOwner::ServiceManager,
            execution_ref: ticket.execution_ref().clone(),
            domain: ExecutionDomain::User,
            user_ref: ticket.user_ref().cloned(),
            digests: *ticket.digests(),
            phase,
            last_exit: None,
            adoption,
        }
    }

    async fn cleanup_failed_launch(
        &self,
        launched: &d2b_process_conformance::LaunchedProcess,
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
}

impl<P: ProcessLaunchEffectPort> ProcessProvider for SupervisorProcessLifecycle<P> {
    fn profile(&self) -> &ProcessProviderProfile {
        &self.profile
    }

    async fn launch(
        &self,
        ticket: &LaunchTicket,
    ) -> Result<ProcessStatusReport, ProcessConformanceError> {
        self.validate(ticket)?;
        let launched = self.port.launch(ticket).await?;
        if let Err(error) = launched.validate(self.profile.required_identity_bindings()) {
            return Err(self.cleanup_failed_launch(&launched, error).await);
        }
        if launched.wait_reap_owner != WaitReapOwner::ServiceManager {
            return Err(
                self.cleanup_failed_launch(&launched, ProcessConformanceError::WaitOwnerMismatch)
                    .await,
            );
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
        let identity_verified = candidate.wait_reap_owner == WaitReapOwner::ServiceManager
            && candidate
                .validate(self.profile.required_identity_bindings())
                .is_ok();
        let expected_identity_matches = ticket
            .expected_identity_digest()
            .is_none_or(|expected| *expected == candidate.identity);
        if !identity_verified || !expected_identity_matches {
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

    async fn stop(
        &self,
        identity: &ProcessIdentityDigest,
        class: StopClass,
    ) -> Result<(), ProcessConformanceError> {
        self.port.stop(identity, class).await
    }
}
