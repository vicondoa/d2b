//! Authenticated display runtime orchestration.
//!
//! The controller remains a pure reconciler.  This module is the production
//! boundary that consumes an admitted ComponentSession, obtains one-use
//! supervisor grants, dispatches role-specific workers through a daemon-owned
//! effect port, and drives ordered finalization.  No Provider-owned service
//! or process handle is retained here.

use std::collections::BTreeSet;

use d2b_provider_toolkit::{AuthenticatedComponentSession, AuthenticatedSessionRouteBinding};
use sha2::{Digest, Sha256};

use crate::{
    AuthenticatedDisplaySession, CleanupState, DependencyState, DisplayController,
    DisplayDependencyProof, DisplayProcessRole, FinalizationDecision, FinalizationInput,
    GraceState, LaunchGrants, ProcessObservation, StopRequest, VolumeState, WaylandPolicySnapshot,
    WaylandSessionSpec, WorkerState, process::WorkerRestartEvidence,
};

/// Failure returned by the daemon-owned display effect port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerEffectError {
    /// The effect owner could not issue a one-use grant.
    GrantUnavailable,
    /// The effect owner rejected the role-specific launch.
    LaunchRejected,
    /// The effect owner could not observe or stop the exact worker.
    WorkerUnavailable,
    /// Cleanup could not be confirmed.
    CleanupIncomplete,
}

impl core::fmt::Display for WorkerEffectError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::GrantUnavailable => "display-worker-grant-unavailable",
            Self::LaunchRejected => "display-worker-launch-rejected",
            Self::WorkerUnavailable => "display-worker-unavailable",
            Self::CleanupIncomplete => "display-worker-cleanup-incomplete",
        })
    }
}

impl std::error::Error for WorkerEffectError {}

/// Error returned by the authenticated display runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayRuntimeError {
    /// ComponentSession route admission failed.
    SessionUnauthenticated,
    /// The authenticated session does not match the desired display.
    SessionMismatch,
    /// Core supplied an invalid policy or retry fence.
    InvalidPolicy,
    /// A daemon-owned effect refused or failed.
    Effect(WorkerEffectError),
    /// A worker action could not be turned into a current observation.
    ObservationUnavailable,
    /// Finalization remains ambiguous and must retain ownership.
    FinalizationAmbiguous,
}

impl core::fmt::Display for DisplayRuntimeError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::SessionUnauthenticated => "display-runtime-session-unauthenticated",
            Self::SessionMismatch => "display-runtime-session-mismatch",
            Self::InvalidPolicy => "display-runtime-policy-invalid",
            Self::Effect(error) => error.code(),
            Self::ObservationUnavailable => "display-runtime-observation-unavailable",
            Self::FinalizationAmbiguous => "display-runtime-finalization-ambiguous",
        })
    }
}

impl std::error::Error for DisplayRuntimeError {}

impl WorkerEffectError {
    const fn code(self) -> &'static str {
        match self {
            Self::GrantUnavailable => "display-worker-grant-unavailable",
            Self::LaunchRejected => "display-worker-launch-rejected",
            Self::WorkerUnavailable => "display-worker-unavailable",
            Self::CleanupIncomplete => "display-worker-cleanup-incomplete",
        }
    }
}

/// Readiness and identity evidence returned by the daemon after one exact
/// worker effect.  The effect owner, not the Provider, is responsible for
/// pidfd/adoption verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkerLaunchReceipt {
    role: DisplayProcessRole,
    state: WorkerState,
    policy_generation: u64,
    teardown_generation: u64,
    session_digest: [u8; 32],
}

impl WorkerLaunchReceipt {
    /// Construct a verified worker receipt at the Core/effect boundary.
    pub const fn from_supervisor(
        role: DisplayProcessRole,
        state: WorkerState,
        policy_generation: u64,
        teardown_generation: u64,
        session_digest: [u8; 32],
    ) -> Self {
        Self {
            role,
            state,
            policy_generation,
            teardown_generation,
            session_digest,
        }
    }

    /// Return the worker role.
    pub const fn role(self) -> DisplayProcessRole {
        self.role
    }

    /// Return the verified worker state.
    pub const fn state(self) -> WorkerState {
        self.state
    }

    /// Return the policy generation proved by the worker.
    pub const fn policy_generation(self) -> u64 {
        self.policy_generation
    }

    /// Return the teardown generation proved by the worker.
    pub const fn teardown_generation(self) -> u64 {
        self.teardown_generation
    }

    /// Return the session digest proved by the worker.
    pub const fn session_digest(self) -> [u8; 32] {
        self.session_digest
    }
}

/// Daemon-owned effect port for display workers and finalization.
///
/// Implementations must route launch, observe, stop, and cleanup through
/// `d2b-provider-supervisor` and the existing typed process effect adapter.
/// The Provider receives no pidfd, socket path, argv, or broker handle.
pub trait DisplayProcessEffectPort {
    /// Read current daemon-owned retry/adoption evidence before reconciliation.
    ///
    /// Production effect owners obtain this from the persisted supervisor
    /// observation; hermetic effect ports may retain the bounded default.
    fn current_supervision(&mut self) -> WorkerRestartEvidence {
        WorkerRestartEvidence::from_supervisor(0, None, None, 1)
    }

    /// Issue one fresh, session-bound launch grant bundle.
    fn issue_launch_grants(
        &mut self,
        session: &AuthenticatedDisplaySession,
        spec: &WaylandSessionSpec,
        policy: &WaylandPolicySnapshot,
        proof: Option<&DisplayDependencyProof>,
        teardown_generation: u64,
    ) -> Result<LaunchGrants, WorkerEffectError>;

    /// Launch or adopt one exact worker ticket.
    fn launch(
        &mut self,
        ticket: crate::LaunchTicket,
    ) -> Result<WorkerLaunchReceipt, WorkerEffectError>;

    /// Stop one exact worker and return terminal deletion evidence.
    fn stop(&mut self, role: DisplayProcessRole) -> Result<WorkerLaunchReceipt, WorkerEffectError>;

    /// Delete the Provider's transient runtime volume after both workers are
    /// terminal and deleted.
    fn delete_runtime_volume(&mut self) -> Result<VolumeState, WorkerEffectError>;

    /// Revoke compositor portal authority.
    fn revoke_portal(&mut self) -> Result<CleanupState, WorkerEffectError>;

    /// Release the Provider principal.
    fn release_principal(&mut self) -> Result<CleanupState, WorkerEffectError>;

    /// Release the authenticated ComponentSession authority.
    fn release_authority(&mut self) -> Result<CleanupState, WorkerEffectError>;
}

/// One live authenticated display runtime.
pub struct DisplayRuntime<E> {
    controller: DisplayController,
    effects: E,
    observation: ProcessObservation,
    supervision: WorkerRestartEvidence,
    issued_grants: BTreeSet<[u8; 32]>,
    stop_requested: bool,
    authority: CleanupState,
    principal: CleanupState,
    portal: CleanupState,
    volume: VolumeState,
}

impl<E> DisplayRuntime<E>
where
    E: DisplayProcessEffectPort,
{
    /// Construct a runtime with daemon-owned effects.
    pub fn new(controller: DisplayController, effects: E) -> Self {
        Self {
            controller,
            effects,
            observation: ProcessObservation::from_supervisor(
                WorkerState::Starting,
                WorkerState::Starting,
                VolumeState::Present,
                0,
                0,
                [0; 32],
            ),
            supervision: WorkerRestartEvidence::from_supervisor(0, None, None, 1),
            issued_grants: BTreeSet::new(),
            stop_requested: false,
            authority: CleanupState::Pending,
            principal: CleanupState::Pending,
            portal: CleanupState::Pending,
            volume: VolumeState::Present,
        }
    }

    /// Borrow the current process observation.
    pub const fn observation(&self) -> ProcessObservation {
        self.observation
    }

    /// Borrow the latest supervisor retry/adoption evidence.
    pub const fn supervision(&self) -> WorkerRestartEvidence {
        self.supervision
    }

    /// Whether both supervised workers are ready for the current fence.
    pub fn is_ready(&self) -> bool {
        self.observation.is_ready()
    }

    /// Borrow the current controller.
    pub const fn controller(&self) -> &DisplayController {
        &self.controller
    }

    /// Mutably borrow the effect owner for daemon composition.
    pub const fn effects_mut(&mut self) -> &mut E {
        &mut self.effects
    }

    /// Refresh retry/adoption evidence from the daemon-owned effect owner.
    pub fn refresh_supervision(&mut self) {
        self.supervision = self.effects.current_supervision();
    }

    /// Project the authenticated display dependency only after both workers
    /// have supplied current readiness and identity evidence.
    pub fn dependency_proof<C>(
        &self,
        session: &AuthenticatedComponentSession<C>,
        spec: &WaylandSessionSpec,
        result: &crate::ReconcileResult,
        policy: &WaylandPolicySnapshot,
    ) -> Result<DisplayDependencyProof, DisplayRuntimeError> {
        self.controller
            .dependency_proof(session, spec, result, policy, self.observation)
            .map_err(|_| DisplayRuntimeError::ObservationUnavailable)
    }

    /// Project dependency evidence from a route retained after daemon
    /// registration.
    pub fn dependency_proof_from_route(
        &self,
        route: &AuthenticatedSessionRouteBinding,
        spec: &WaylandSessionSpec,
        result: &crate::ReconcileResult,
        policy: &WaylandPolicySnapshot,
    ) -> Result<DisplayDependencyProof, DisplayRuntimeError> {
        self.controller
            .dependency_proof_from_route(route, spec, result, policy, self.observation)
            .map_err(|_| DisplayRuntimeError::ObservationUnavailable)
    }

    /// Reconcile one authenticated session and dispatch any required workers.
    ///
    /// The first pass deliberately runs without grants.  This lets the
    /// controller determine whether a launch is needed before the effect port
    /// mints fresh, one-use grants.  A second pass consumes those grants into
    /// role-specific tickets, and the effect owner returns the only readiness
    /// evidence accepted into the next observation.
    pub fn reconcile<C>(
        &mut self,
        session: &AuthenticatedComponentSession<C>,
        spec: &WaylandSessionSpec,
        dependencies: DependencyState,
        supervision: WorkerRestartEvidence,
        policy: &WaylandPolicySnapshot,
    ) -> Result<crate::ReconcileResult, DisplayRuntimeError> {
        let authenticated = AuthenticatedDisplaySession::from_component_session(session)
            .map_err(|_| DisplayRuntimeError::SessionUnauthenticated)?;
        if authenticated.guest_ref() != spec.guest_ref()
            || authenticated.host_ref() != spec.host_ref()
            || authenticated.reconnect_generation() != spec.reconnect_generation()
            || authenticated.zone() != policy.zone()
        {
            return Err(DisplayRuntimeError::SessionMismatch);
        }
        self.supervision = supervision;
        let mut result = self
            .controller
            .reconcile_authenticated_session(
                session,
                spec,
                dependencies.clone(),
                self.observation,
                supervision,
                None,
                policy,
            )
            .map_err(|_| DisplayRuntimeError::InvalidPolicy)?;
        if !result.worker_actions.is_empty() {
            let fence = grant_fence(&authenticated, supervision);
            if self.issued_grants.contains(&fence) {
                return Err(DisplayRuntimeError::Effect(
                    WorkerEffectError::GrantUnavailable,
                ));
            }
            let grants = self
                .effects
                .issue_launch_grants(
                    &authenticated,
                    spec,
                    policy,
                    self.controller
                        .dependency_proof(session, spec, &result, policy, self.observation)
                        .ok()
                        .as_ref(),
                    supervision.teardown_generation,
                )
                .map_err(DisplayRuntimeError::Effect)?;
            let launch_tickets = self
                .controller
                .reconcile_authenticated_session(
                    session,
                    spec,
                    dependencies.clone(),
                    self.observation,
                    supervision,
                    Some(grants),
                    policy,
                )
                .map_err(|_| DisplayRuntimeError::InvalidPolicy)?
                .launch_tickets;
            if launch_tickets.is_empty() {
                return self
                    .controller
                    .reconcile_authenticated_session(
                        session,
                        spec,
                        dependencies,
                        self.observation,
                        supervision,
                        None,
                        policy,
                    )
                    .map_err(|_| DisplayRuntimeError::InvalidPolicy);
            }
            for ticket in launch_tickets {
                let receipt = self
                    .effects
                    .launch(ticket)
                    .map_err(DisplayRuntimeError::Effect)?;
                if receipt.teardown_generation() != supervision.teardown_generation
                    || receipt.policy_generation() != policy.generation()
                {
                    return Err(DisplayRuntimeError::ObservationUnavailable);
                }
                self.observe_receipt(receipt);
            }
            self.issued_grants.insert(fence);
            result = self
                .controller
                .reconcile_authenticated_session(
                    session,
                    spec,
                    dependencies,
                    self.observation,
                    supervision,
                    None,
                    policy,
                )
                .map_err(|_| DisplayRuntimeError::InvalidPolicy)?;
        }
        Ok(result)
    }

    /// Reconcile a session whose authority was consumed by the daemon's Zone
    /// registrar.  The retained route is authenticated metadata only; bus
    /// ingress remains the owner of cancellation and request authority.
    pub fn reconcile_registered(
        &mut self,
        route: &AuthenticatedSessionRouteBinding,
        spec: &WaylandSessionSpec,
        dependencies: DependencyState,
        supervision: WorkerRestartEvidence,
        policy: &WaylandPolicySnapshot,
    ) -> Result<crate::ReconcileResult, DisplayRuntimeError> {
        let authenticated = AuthenticatedDisplaySession::from_authenticated_route(route.clone())
            .map_err(|_| DisplayRuntimeError::SessionUnauthenticated)?;
        if authenticated.guest_ref() != spec.guest_ref()
            || authenticated.host_ref() != spec.host_ref()
            || authenticated.reconnect_generation() != spec.reconnect_generation()
            || authenticated.zone() != policy.zone()
        {
            return Err(DisplayRuntimeError::SessionMismatch);
        }
        self.supervision = supervision;
        let mut result = self
            .controller
            .reconcile_authenticated_route(
                route,
                spec,
                dependencies.clone(),
                self.observation,
                supervision,
                None,
                policy,
            )
            .map_err(|_| DisplayRuntimeError::InvalidPolicy)?;
        if !result.worker_actions.is_empty() {
            let fence = grant_fence(&authenticated, supervision);
            if self.issued_grants.contains(&fence) {
                return Err(DisplayRuntimeError::Effect(
                    WorkerEffectError::GrantUnavailable,
                ));
            }
            let proof = self
                .controller
                .dependency_proof_from_route(route, spec, &result, policy, self.observation)
                .ok();
            let grants = self
                .effects
                .issue_launch_grants(
                    &authenticated,
                    spec,
                    policy,
                    proof.as_ref(),
                    supervision.teardown_generation,
                )
                .map_err(DisplayRuntimeError::Effect)?;
            let launch_tickets = self
                .controller
                .reconcile_authenticated_route(
                    route,
                    spec,
                    dependencies.clone(),
                    self.observation,
                    supervision,
                    Some(grants),
                    policy,
                )
                .map_err(|_| DisplayRuntimeError::InvalidPolicy)?
                .launch_tickets;
            if launch_tickets.is_empty() {
                return self
                    .controller
                    .reconcile_authenticated_route(
                        route,
                        spec,
                        dependencies,
                        self.observation,
                        supervision,
                        None,
                        policy,
                    )
                    .map_err(|_| DisplayRuntimeError::InvalidPolicy);
            }
            for ticket in launch_tickets {
                let receipt = self
                    .effects
                    .launch(ticket)
                    .map_err(DisplayRuntimeError::Effect)?;
                if receipt.teardown_generation() != supervision.teardown_generation
                    || receipt.policy_generation() != policy.generation()
                {
                    return Err(DisplayRuntimeError::ObservationUnavailable);
                }
                self.observe_receipt(receipt);
            }
            self.issued_grants.insert(fence);
            result = self
                .controller
                .reconcile_authenticated_route(
                    route,
                    spec,
                    dependencies,
                    self.observation,
                    supervision,
                    None,
                    policy,
                )
                .map_err(|_| DisplayRuntimeError::InvalidPolicy)?;
        }
        Ok(result)
    }

    fn observe_receipt(&mut self, receipt: WorkerLaunchReceipt) {
        let (proxy, frontend) = match receipt.role() {
            DisplayProcessRole::HostProxy => (receipt.state(), self.observation.frontend),
            DisplayProcessRole::GuestFrontend => (self.observation.proxy, receipt.state()),
        };
        self.observation = ProcessObservation::from_supervisor(
            proxy,
            frontend,
            self.volume,
            receipt.policy_generation(),
            receipt.teardown_generation(),
            receipt.session_digest(),
        );
    }

    /// Finalize in the required order: stop workers, delete the transient
    /// volume, revoke portal authority, release the principal, then release
    /// the authenticated session authority.
    pub fn finalize(
        &mut self,
        grace: GraceState,
    ) -> Result<FinalizationReport, DisplayRuntimeError> {
        self.stop_requested = true;
        let stop_proxy =
            !(self.observation.proxy.is_terminal() && self.observation.proxy.is_deleted());
        let stop_frontend =
            !(self.observation.frontend.is_terminal() && self.observation.frontend.is_deleted());
        if stop_proxy {
            let receipt = self
                .effects
                .stop(DisplayProcessRole::HostProxy)
                .map_err(DisplayRuntimeError::Effect)?;
            self.observe_receipt(receipt);
        }
        if stop_frontend {
            let receipt = self
                .effects
                .stop(DisplayProcessRole::GuestFrontend)
                .map_err(DisplayRuntimeError::Effect)?;
            self.observe_receipt(receipt);
        }
        let decision = DisplayController::finalize(FinalizationInput::from_supervisor(
            StopRequest::Requested,
            self.observation.proxy,
            self.observation.frontend,
            self.volume,
            self.authority,
            self.principal,
            self.portal,
            grace,
        ));
        if decision.delete_runtime_volume {
            self.volume = self
                .effects
                .delete_runtime_volume()
                .map_err(DisplayRuntimeError::Effect)?;
        }
        if self.volume.is_deleted()
            && self.observation.proxy.is_terminal()
            && self.observation.proxy.is_deleted()
            && self.observation.frontend.is_terminal()
            && self.observation.frontend.is_deleted()
        {
            self.portal = self
                .effects
                .revoke_portal()
                .map_err(DisplayRuntimeError::Effect)?;
            self.principal = self
                .effects
                .release_principal()
                .map_err(DisplayRuntimeError::Effect)?;
            self.authority = self
                .effects
                .release_authority()
                .map_err(DisplayRuntimeError::Effect)?;
        }
        let final_decision = DisplayController::finalize(FinalizationInput::from_supervisor(
            StopRequest::Requested,
            self.observation.proxy,
            self.observation.frontend,
            self.volume,
            self.authority,
            self.principal,
            self.portal,
            grace,
        ));
        if final_decision.ambiguous && matches!(grace, GraceState::Expired) {
            return Err(DisplayRuntimeError::FinalizationAmbiguous);
        }
        Ok(FinalizationReport {
            decision: final_decision,
            stop_proxy,
            stop_frontend,
            volume: self.volume,
            authority: self.authority,
            principal: self.principal,
            portal: self.portal,
        })
    }
}

fn grant_fence(
    session: &AuthenticatedDisplaySession,
    supervision: WorkerRestartEvidence,
) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(session.guest_ref().to_canonical_string().as_bytes());
    digest.update(session.host_ref().to_canonical_string().as_bytes());
    digest.update(session.zone().as_str().as_bytes());
    digest.update(session.reconnect_generation().to_be_bytes());
    digest.update(session.controller_generation().to_be_bytes());
    digest.update(supervision.observed_at_ms.to_be_bytes());
    digest.update(
        supervision
            .proxy_last_failure_ms
            .unwrap_or_default()
            .to_be_bytes(),
    );
    digest.update(
        supervision
            .frontend_last_failure_ms
            .unwrap_or_default()
            .to_be_bytes(),
    );
    digest.update(supervision.teardown_generation.to_be_bytes());
    digest.finalize().into()
}

/// Ordered finalization evidence returned to daemon reconciliation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FinalizationReport {
    /// Finalizer decision after all newly observed cleanup.
    pub decision: FinalizationDecision,
    /// Whether the proxy stop effect ran.
    pub stop_proxy: bool,
    /// Whether the frontend stop effect ran.
    pub stop_frontend: bool,
    /// Runtime volume state.
    pub volume: VolumeState,
    /// ComponentSession authority state.
    pub authority: CleanupState,
    /// Principal state.
    pub principal: CleanupState,
    /// Portal state.
    pub portal: CleanupState,
}

impl FinalizationReport {
    /// Report a completed finalization for a session that never launched
    /// display workers.
    pub const fn empty() -> Self {
        Self {
            decision: FinalizationDecision {
                stop_proxy: false,
                stop_frontend: false,
                delete_runtime_volume: true,
                remove_finalizer: true,
                phase: crate::controller::Phase::Terminating,
                ambiguous: false,
            },
            stop_proxy: false,
            stop_frontend: false,
            volume: VolumeState::Deleted,
            authority: CleanupState::Complete,
            principal: CleanupState::Complete,
            portal: CleanupState::Complete,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DisplayIdentity, FilterInput, WorkerState, process::VolumeState};
    use d2b_contracts_zone_session::v3::{ResourceRef, ZoneId};

    #[derive(Default)]
    struct Effects {
        launches: Vec<DisplayProcessRole>,
        stops: Vec<DisplayProcessRole>,
        cleanup: Vec<&'static str>,
        nonce: u64,
    }

    impl DisplayProcessEffectPort for Effects {
        fn issue_launch_grants(
            &mut self,
            session: &AuthenticatedDisplaySession,
            _spec: &WaylandSessionSpec,
            _policy: &WaylandPolicySnapshot,
            _proof: Option<&DisplayDependencyProof>,
            teardown_generation: u64,
        ) -> Result<LaunchGrants, WorkerEffectError> {
            self.nonce = self.nonce.saturating_add(1);
            Ok(LaunchGrants::from_supervisor_for_session_with_frontend(
                crate::process::AttachmentGrantHandle::from_supervisor([1; 32]),
                crate::process::AttachmentGrantHandle::from_supervisor([2; 32]),
                crate::process::AttachmentGrantHandle::from_supervisor([3; 32]),
                [7; 32],
                session.reconnect_generation(),
                teardown_generation,
            ))
        }

        fn launch(
            &mut self,
            ticket: crate::LaunchTicket,
        ) -> Result<WorkerLaunchReceipt, WorkerEffectError> {
            self.launches.push(ticket.role());
            Ok(WorkerLaunchReceipt::from_supervisor(
                ticket.role(),
                WorkerState::Ready { generation: 1 },
                ticket.policy_generation(),
                ticket.teardown_generation(),
                [7; 32],
            ))
        }

        fn stop(
            &mut self,
            role: DisplayProcessRole,
        ) -> Result<WorkerLaunchReceipt, WorkerEffectError> {
            self.stops.push(role);
            Ok(WorkerLaunchReceipt::from_supervisor(
                role,
                WorkerState::Terminal { deleted: true },
                1,
                1,
                [7; 32],
            ))
        }

        fn delete_runtime_volume(&mut self) -> Result<VolumeState, WorkerEffectError> {
            self.cleanup.push("volume");
            Ok(VolumeState::Deleted)
        }

        fn revoke_portal(&mut self) -> Result<CleanupState, WorkerEffectError> {
            self.cleanup.push("portal");
            Ok(CleanupState::Complete)
        }

        fn release_principal(&mut self) -> Result<CleanupState, WorkerEffectError> {
            self.cleanup.push("principal");
            Ok(CleanupState::Complete)
        }

        fn release_authority(&mut self) -> Result<CleanupState, WorkerEffectError> {
            self.cleanup.push("authority");
            Ok(CleanupState::Complete)
        }
    }

    fn display() -> (WaylandSessionSpec, WaylandPolicySnapshot) {
        let spec = WaylandSessionSpec::new(
            ResourceRef::parse("Guest/work-vm").unwrap(),
            ResourceRef::parse("Host/host-system").unwrap(),
            ResourceRef::parse("User/alice").unwrap(),
            ResourceRef::parse("display-wayland.d2bus.org.WaylandPolicy/default").unwrap(),
            DisplayIdentity::new("work-vm", "#7fc8ff", "#45475a", "#f38ab8").unwrap(),
            true,
        )
        .unwrap();
        let policy = WaylandPolicySnapshot::from_test_core(
            spec.policy_ref().clone(),
            ZoneId::parse("dev").unwrap(),
            1,
            FilterInput::default(),
            FilterInput::default(),
        )
        .unwrap();
        (spec, policy)
    }

    #[test]
    fn effect_port_receives_independent_worker_launches_and_ordered_cleanup() {
        let (spec, policy) = display();
        // The production entrypoint accepts an AuthenticatedComponentSession,
        // so this test exercises the same effect port and finalizer ordering
        // through a directly seeded runtime observation.
        let mut runtime = DisplayRuntime::new(DisplayController::new(2), Effects::default());
        runtime.observation = ProcessObservation::from_supervisor(
            WorkerState::Terminal { deleted: false },
            WorkerState::Terminal { deleted: false },
            VolumeState::Present,
            policy.generation(),
            1,
            [7; 32],
        );
        let report = runtime.finalize(GraceState::Active).unwrap();
        assert!(report.decision.remove_finalizer);
        assert_eq!(
            runtime.effects.cleanup,
            vec!["volume", "portal", "principal", "authority"]
        );
        let _ = spec;
    }
}
