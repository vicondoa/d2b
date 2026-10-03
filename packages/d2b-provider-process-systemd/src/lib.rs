//! The `system-systemd` Process Provider controller.
//!
//! A process is a **non-forking transient system unit or scope**. Identity is
//! the unit InvocationID bound together
//! with the cgroup, the unit main process, that process's start time, and
//! the Provider, template, and generation triple. A unit name alone is
//! never identity, so it is neither an identity binding nor public status.
//! systemd owns `wait` and reap; this Provider holds only a locally
//! verified pidfd.
//!
//! Adapted from the earlier non-forking `systemd-run` transient-unit launch
//! contract.
//!
//! This crate performs no privileged mutation: it opens no D-Bus or systemd
//! socket, spawns no process, and resolves no unit name or path. It
//! validates the ticket and calls the injected
//! [`ProcessLaunchEffectPort`], which the fixed core process effect adapter
//! implements.
//!
//! The library target exports the surface production composes today: the
//! Provider controller, the `lifecycle` root re-exports, [`effects_service`],
//! and [`operations`]. The controller family (`controller`), `drain`,
//! `metrics`, `audit`, `launch`, and `sandbox` have no production consumer:
//! this crate's conformance tests exercise the first three, and the daemon
//! reconcile composition has not landed on any of the six, so they compile
//! behind the `test-support` feature instead of being exported by the
//! library.

#![deny(missing_docs)]

// The gated surface above has no production consumer: this crate's tests and
// the conformance suites reach it, and consumers opt in through the
// `test-support` feature. Gating on `any(test, feature = "test-support")`
// makes it available automatically to this crate's unit tests; the
// integration tests that consume it declare `required-features`, so run those
// with `--features test-support` (or let the Bazel `*_test_support` target
// compile them).
#[cfg(any(test, feature = "test-support"))]
pub mod audit;
#[cfg(any(test, feature = "test-support"))]
pub mod controller;
#[cfg(any(test, feature = "test-support"))]
pub mod drain;
pub mod effects_service;
#[cfg(any(test, feature = "test-support"))]
pub mod launch;
mod lifecycle;
#[cfg(any(test, feature = "test-support"))]
pub mod metrics;
pub mod operations;
#[cfg(any(test, feature = "test-support"))]
pub mod sandbox;

pub use lifecycle::{
    EphemeralProcessController, RestartPolicy, SystemdConfigError, SystemdProviderConfig,
};

use std::collections::BTreeSet;

use d2b_contracts_resource::v3::binding::BindingRealizationFacet;
use d2b_contracts_resource::v3::execution_policy::{BoundedToken, ExecutionDomain};
use d2b_contracts_resource::v3::{BackendSupport, ConfinementFacet};
use d2b_process_conformance::{
    AdoptionCandidate, AdoptionCondition, AdoptionOutcome, CancellationBinding, IdentityBinding,
    LaunchTicket, LaunchedProcess, ProcessConformanceError, ProcessIdentityDigest,
    ProcessLaunchEffectPort, ProcessPhaseClass, ProcessProvider, ProcessProviderProfile,
    ProcessStatusReport, ReadinessExpectation, StopClass, WaitReapOwner,
};
use tracing::{debug, warn};

/// The Provider name this controller implements.
pub const PROVIDER_NAME: &str = "system-systemd";

/// The canonical `Provider/<name>` reference this controller implements.
pub const PROVIDER_REF: &str = "Provider/system-systemd";

/// The confinement facets the `system-systemd` transient-unit path enforces.
///
/// This is the family's whole declared support and it is deliberately small.
/// The launch sets `NoNewPrivileges=true` and `ProtectSystem=strict`, and
/// nothing else: a transient unit gets no namespace, no capability bounding
/// set, no `SystemCallFilter`, and no private devices. Declaring more here
/// would be exactly the "a declared field is not a declared support" failure
/// this conversion removes, so the set states the truth and both
/// enforcement points - the controller's plan check and the operation
/// request fence - refuse everything else (R27, R50).
pub fn enforced_confinement_facets() -> BackendSupport {
    BackendSupport::new(vec![
        ConfinementFacet::NoNewPrivileges,
        ConfinementFacet::ReadOnlyRoot,
    ])
    .expect("the family's declared support set is well formed")
}

/// The presentation facets this backend realizes.
///
/// None. The properties this family sets carry no `BindPaths`,
/// `ReadWritePaths`, or `TemporaryFileSystem`, so it cannot confine a
/// workload to an exact source view at a destination. A launch that depends
/// on a presentation therefore refuses before a unit is started rather than
/// starting without the confinement it asked for (R20, AE19).
pub fn realized_presentation_facets() -> BTreeSet<BindingRealizationFacet> {
    BTreeSet::new()
}

/// Whether this backend realizes one presentation facet.
pub fn realizes_presentation(facet: BindingRealizationFacet) -> bool {
    realized_presentation_facets().contains(&facet)
}

/// The `system-systemd` Process Provider controller.
#[derive(Debug)]
pub struct SystemdProcessProvider<P: ProcessLaunchEffectPort> {
    port: P,
    profile: ProcessProviderProfile,
}

impl<P: ProcessLaunchEffectPort> SystemdProcessProvider<P> {
    /// Build the controller over an injected process effect port.
    ///
    /// The broker verifies both system and authenticated user-manager
    /// transient-unit paths before the effect reaches systemd.
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

    /// The one policy path this Provider enforces.
    ///
    /// A ticket that carries the resolved plan is validated against the
    /// plan's own admitted execution and its prepared relationships rather
    /// than against a separately authored posture. Two refusals matter here
    /// and both are about what this backend can *do*:
    ///
    /// * an admitted execution whose isolation classes, capability set,
    ///   syscall filter, or user namespace this family does not enforce is
    ///   refused - the family's declared support is the whole truth about
    ///   what a transient unit applies, and an unsupported requirement fails
    ///   closed instead of being ignored (R27);
    /// * a prepared relationship whose presentation this family cannot
    ///   realize is refused before a unit is started. This backend has no
    ///   mount namespace, so a destination or a named view it cannot confine
    ///   a workload to is refused rather than launched without it (R20,
    ///   AE19).
    ///
    /// A ticket with no plan is the pre-plan path and is validated as it
    /// always was; U34 deletes that branch with the rest of the ticket
    /// authority.
    fn validate_plan(&self, ticket: &LaunchTicket) -> Result<(), ProcessConformanceError> {
        let Some(plan) = ticket.resolved_plan() else {
            return Ok(());
        };
        if !plan.admits_start() {
            warn!(
                provider = PROVIDER_NAME,
                resource = %ticket.process_ref().to_canonical_string(),
                "assignment rejected: a required binding is not prepared"
            );
            return Err(ProcessConformanceError::ResolutionFailed);
        }
        if !plan
            .prepared_against(plan.subject())
            .is_ok_and(|()| ticket.process_ref() == plan.subject().process_ref())
        {
            warn!(
                provider = PROVIDER_NAME,
                resource = %ticket.process_ref().to_canonical_string(),
                "assignment rejected: the plan was not prepared for this row"
            );
            return Err(ProcessConformanceError::ResolutionFailed);
        }
        if let Some(facet) = plan
            .bindings()
            .flat_map(d2b_process_conformance::PreparedBinding::presentation)
            .copied()
            .find(|facet| !realizes_presentation(*facet))
        {
            warn!(
                provider = PROVIDER_NAME,
                resource = %ticket.process_ref().to_canonical_string(),
                presentation = ?facet,
                "assignment rejected: this backend cannot realize the required presentation"
            );
            return Err(ProcessConformanceError::SandboxRejected);
        }
        let support = enforced_confinement_facets();
        let execution = plan.execution();
        let unenforceable = execution
            .namespace_classes()
            .iter()
            .any(|class| !support.enforces(ConfinementFacet::from_namespace(*class)))
            || !execution.capability_classes().is_empty()
            && !support.enforces(ConfinementFacet::CapabilityCeiling)
            || execution.seccomp_profile_ref().is_some()
            && !support.enforces(ConfinementFacet::SyscallFilter)
            || execution.no_new_privileges() && !support.enforces(ConfinementFacet::NoNewPrivileges);
        if unenforceable {
            warn!(
                provider = PROVIDER_NAME,
                resource = %ticket.process_ref().to_canonical_string(),
                "assignment rejected: the admitted execution is not enforceable here"
            );
            return Err(ProcessConformanceError::SandboxRejected);
        }
        Ok(())
    }

    fn validate(&self, ticket: &LaunchTicket) -> Result<(), ProcessConformanceError> {
        if let Err(error) = ticket.validate() {
            warn!(
                provider = PROVIDER_NAME,
                resource = %ticket.process_ref().to_canonical_string(),
                error = %error,
                "launch ticket validation rejected"
            );
            return Err(error);
        }
        if ticket.has_controller_launch_binding()
            && let Err(error) = ticket.validate_controller_launch()
        {
            warn!(
                provider = PROVIDER_NAME,
                resource = %ticket.process_ref().to_canonical_string(),
                error = %error,
                "controller launch binding validation rejected"
            );
            return Err(error);
        }
        if ticket.has_assignment_binding()
            && let Err(error) = ticket.validate_assignment()
        {
            warn!(
                provider = PROVIDER_NAME,
                resource = %ticket.process_ref().to_canonical_string(),
                error = %error,
                "assignment binding validation rejected"
            );
            return Err(error);
        }
        if ticket.selected_provider().as_str() != PROVIDER_NAME {
            warn!(
                provider = PROVIDER_NAME,
                resource = %ticket.process_ref().to_canonical_string(),
                selected = ticket.selected_provider().as_str(),
                "assignment rejected: selected provider mismatch"
            );
            return Err(ProcessConformanceError::ProviderMismatch);
        }
        if !self.profile.supported_domains().contains(&ticket.domain()) {
            warn!(
                provider = PROVIDER_NAME,
                resource = %ticket.process_ref().to_canonical_string(),
                "assignment rejected: execution domain not supported"
            );
            return Err(ProcessConformanceError::DomainNotSupported);
        }
        if ticket.operation().cancellation() == CancellationBinding::Cancelled {
            debug!(
                provider = PROVIDER_NAME,
                resource = %ticket.process_ref(),
                "assignment rejected: operation cancelled"
            );
            return Err(ProcessConformanceError::Cancelled);
        }
        if ticket.domain() == ExecutionDomain::User && ticket.user_ref().is_none() {
            warn!(
                provider = PROVIDER_NAME,
                resource = %ticket.process_ref().to_canonical_string(),
                "assignment rejected: user domain requires a user ref"
            );
            return Err(ProcessConformanceError::UserRefRequired);
        }
        self.validate_plan(ticket)
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
            Err(stop_error) => {
                warn!(
                    provider = PROVIDER_NAME,
                    identity = %launched.identity.to_hex(),
                    stop_error = %stop_error,
                    "stop of failed launch unavailable; reporting stop-unavailable"
                );
                ProcessConformanceError::StopUnavailable
            }
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
                    warn!(
                        provider = PROVIDER_NAME,
                        resource = %ticket.process_ref().to_canonical_string(),
                        "readiness probe found no candidate before deadline"
                    );
                    return Err(ProcessConformanceError::DeadlineExceeded);
                };
                if !self.candidate_matches(ticket, &candidate, identity) {
                    warn!(
                        provider = PROVIDER_NAME,
                        resource = %ticket.process_ref().to_canonical_string(),
                        "readiness probe candidate identity mismatch"
                    );
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
            && candidate.wait_reap_owner == WaitReapOwner::ServiceManager
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
        let launched = self.port.launch(ticket).await.inspect_err(|error| {
            warn!(
                provider = PROVIDER_NAME,
                resource = %ticket.process_ref().to_canonical_string(),
                error = %error,
                "process start failed"
            );
        })?;
        if launched.wait_reap_owner != WaitReapOwner::ServiceManager {
            warn!(
                provider = PROVIDER_NAME,
                identity = %launched.identity.to_hex(),
                "launched process wait/reap owner mismatch"
            );
            return Err(ProcessConformanceError::WaitOwnerMismatch);
        }
        launched
            .validate(self.profile.required_identity_bindings())
            .inspect_err(|error| {
                warn!(
                    provider = PROVIDER_NAME,
                    identity = %launched.identity.to_hex(),
                    error = %error,
                    "launched process identity binding validation failed"
                );
            })?;
        ticket
            .validate_process_identity(&launched.identity)
            .inspect_err(|error| {
                warn!(
                    provider = PROVIDER_NAME,
                    identity = %launched.identity.to_hex(),
                    error = %error,
                    "launched process identity does not match the ticket"
                );
            })?;
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
        // Revalidate every stable identity property before the pidfd is
        // opened. Ambiguity quarantines; it never broadly kills or reuses.
        let identity_ok = candidate.wait_reap_owner == WaitReapOwner::ServiceManager
            && candidate
                .validate(self.profile.required_identity_bindings())
                .is_ok()
            && ticket
                .validate_process_identity(&candidate.identity)
                .is_ok();
        if !identity_ok {
            warn!(
                provider = PROVIDER_NAME,
                resource = %ticket.process_ref().to_canonical_string(),
                identity = %candidate.identity.to_hex(),
                "adoption refused: identity binding verification failed"
            );
            return Ok(AdoptionOutcome::Quarantined(self.report(
                ticket,
                candidate.identity,
                ProcessPhaseClass::Unknown,
                AdoptionCondition::Quarantined,
            )));
        }
        let phase = match self.readiness_phase(ticket, candidate.identity).await {
            Ok(phase) => phase,
            Err(readiness_error) => {
                warn!(
                    provider = PROVIDER_NAME,
                    resource = %ticket.process_ref().to_canonical_string(),
                    identity = %candidate.identity.to_hex(),
                    error = %readiness_error,
                    "adoption quarantined: readiness probe failed"
                );
                return Ok(AdoptionOutcome::Quarantined(self.report(
                    ticket,
                    candidate.identity,
                    ProcessPhaseClass::Unknown,
                    AdoptionCondition::Quarantined,
                )));
            }
        };
        let _pidfd = self.port.open_pidfd(&candidate).await.inspect_err(|error| {
            warn!(
                provider = PROVIDER_NAME,
                identity = %candidate.identity.to_hex(),
                error = %error,
                "pidfd open failed during adoption"
            );
        })?;
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
            warn!(
                provider = PROVIDER_NAME,
                "stop rejected: identity unverified"
            );
            return Err(ProcessConformanceError::IdentityUnverified);
        }
        self.port.stop(identity, class).await.inspect_err(|error| {
            warn!(
                provider = PROVIDER_NAME,
                identity = %identity.to_hex(),
                error = %error,
                "process stop failed"
            );
        })
    }

    async fn stop_stale(
        &self,
        candidate: &AdoptionCandidate,
    ) -> Result<(), ProcessConformanceError> {
        if candidate.identity.is_zero()
            || candidate.wait_reap_owner != WaitReapOwner::ServiceManager
        {
            warn!(
                provider = PROVIDER_NAME,
                "stale-candidate stop rejected: identity unverified"
            );
            return Err(ProcessConformanceError::IdentityUnverified);
        }
        self.port.open_pidfd(candidate).await?;
        self.port
            .stop(&candidate.identity, StopClass::Terminate)
            .await
            .inspect_err(|error| {
                warn!(
                    provider = PROVIDER_NAME,
                    identity = %candidate.identity.to_hex(),
                    error = %error,
                    "stale candidate stop failed"
                );
            })
    }
}
