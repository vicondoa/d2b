//! Process and attachment projections for display workers.

use serde::{Deserialize, Serialize};

/// Lifecycle evidence for one independently supervised worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerState {
    /// The worker has not completed its readiness handshake.
    Starting,
    /// The worker completed a fresh readiness handshake.
    Ready {
        /// Monotonic worker generation.
        generation: u64,
    },
    /// The worker failed and may be restarted.
    Failed {
        /// Consecutive failures in the current bounded retry window.
        attempts: u8,
    },
    /// The worker reached a verified terminal state.
    Terminal {
        /// Whether Process deletion has been confirmed.
        deleted: bool,
    },
}

impl WorkerState {
    /// Whether this worker has a current readiness proof.
    pub const fn is_ready(self) -> bool {
        matches!(self, Self::Ready { .. })
    }

    /// Return the current retry count.
    pub const fn failure_count(self) -> u8 {
        match self {
            Self::Failed { attempts } => attempts,
            _ => 0,
        }
    }

    /// Whether the worker is terminal.
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Terminal { .. })
    }

    /// Whether Process deletion was confirmed.
    pub const fn is_deleted(self) -> bool {
        matches!(self, Self::Terminal { deleted: true })
    }
}

/// Runtime Volume cleanup evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VolumeState {
    /// The runtime Volume remains owned by the Provider.
    Present,
    /// The runtime Volume deletion was confirmed.
    Deleted,
}

impl VolumeState {
    /// Whether the Volume is gone.
    pub const fn is_deleted(self) -> bool {
        matches!(self, Self::Deleted)
    }
}

/// One bounded action emitted by the display worker supervisor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerAction {
    /// Start or restart the Host proxy.
    EnsureProxy,
    /// Start or restart the Guest frontend independently of the proxy.
    EnsureFrontend,
}

/// Stable worker supervision failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerSupervisorError {
    /// The worker exhausted its bounded restart budget.
    RetryExhausted(DisplayProcessRole),
}

/// Bounded worker supervision planner.
///
/// Proxy and frontend readiness are evaluated independently.  A restart
/// budget is carried by this controller rather than inferred from the other
/// worker's state, so a healthy proxy cannot suppress a failed frontend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkerSupervisor {
    max_attempts: u8,
}

impl WorkerSupervisor {
    /// Default consecutive restart bound for one worker.
    pub const DEFAULT_MAX_ATTEMPTS: u8 = 5;

    /// Construct a bounded worker supervisor.
    pub const fn new(max_attempts: u8) -> Option<Self> {
        if max_attempts == 0 {
            None
        } else {
            Some(Self { max_attempts })
        }
    }

    /// Plan independent worker actions from the latest authenticated proof.
    pub fn plan(observation: ProcessObservation, policy_changed: bool) -> Vec<WorkerAction> {
        let mut actions = Vec::new();
        if policy_changed || !observation.proxy.is_ready() {
            actions.push(WorkerAction::EnsureProxy);
        }
        if policy_changed || !observation.frontend.is_ready() {
            actions.push(WorkerAction::EnsureFrontend);
        }
        actions
    }

    /// Plan actions while enforcing each worker's independent retry budget.
    pub fn plan_with_budget(
        &self,
        observation: ProcessObservation,
        policy_changed: bool,
    ) -> Result<Vec<WorkerAction>, WorkerSupervisorError> {
        let actions = Self::plan(observation, policy_changed);
        for action in &actions {
            let (role, attempts) = match action {
                WorkerAction::EnsureProxy => (
                    DisplayProcessRole::HostProxy,
                    observation.proxy.failure_count(),
                ),
                WorkerAction::EnsureFrontend => (
                    DisplayProcessRole::GuestFrontend,
                    observation.frontend.failure_count(),
                ),
            };
            if attempts >= self.max_attempts {
                return Err(WorkerSupervisorError::RetryExhausted(role));
            }
        }
        Ok(actions)
    }
}

/// Opaque attachment grant handle resolved by ProviderSupervisor.
#[derive(PartialEq, Eq)]
pub struct AttachmentGrantHandle([u8; 32]);

impl AttachmentGrantHandle {
    /// Construct a handle at the private Core/Supervisor boundary.
    #[allow(dead_code)]
    pub(crate) const fn from_supervisor(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
}

impl core::fmt::Debug for AttachmentGrantHandle {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("AttachmentGrantHandle(REDACTED)")
    }
}

/// Opaque per-session grants required to launch display workers.
#[derive(PartialEq, Eq)]
pub struct LaunchGrants {
    compositor: AttachmentGrantHandle,
    gpu: AttachmentGrantHandle,
    frontend_gpu: Option<AttachmentGrantHandle>,
    session_digest: [u8; 32],
    reconnect_generation: u64,
}

impl LaunchGrants {
    /// Construct launch grants at the private Core/Supervisor boundary.
    #[allow(dead_code)]
    pub(crate) const fn from_supervisor(
        compositor: AttachmentGrantHandle,
        gpu: AttachmentGrantHandle,
    ) -> Self {
        Self {
            compositor,
            gpu,
            frontend_gpu: None,
            session_digest: [0; 32],
            reconnect_generation: 0,
        }
    }

    /// Construct grants bound to one authenticated display session.
    #[allow(dead_code)]
    pub(crate) const fn from_supervisor_for_session(
        compositor: AttachmentGrantHandle,
        gpu: AttachmentGrantHandle,
        session_digest: [u8; 32],
        reconnect_generation: u64,
    ) -> Self {
        Self {
            compositor,
            gpu,
            frontend_gpu: None,
            session_digest,
            reconnect_generation,
        }
    }

    /// Construct grants for both independently supervised display workers.
    #[allow(dead_code)]
    pub(crate) const fn from_supervisor_for_session_with_frontend(
        compositor: AttachmentGrantHandle,
        gpu: AttachmentGrantHandle,
        frontend_gpu: AttachmentGrantHandle,
        session_digest: [u8; 32],
        reconnect_generation: u64,
    ) -> Self {
        Self {
            compositor,
            gpu,
            frontend_gpu: Some(frontend_gpu),
            session_digest,
            reconnect_generation,
        }
    }

    #[allow(dead_code)]
    pub(crate) fn into_parts(
        self,
        expected_session_digest: [u8; 32],
        expected_reconnect_generation: u64,
    ) -> Option<(AttachmentGrantHandle, AttachmentGrantHandle)> {
        if self.session_digest != expected_session_digest
            || self.reconnect_generation != expected_reconnect_generation
            || self.reconnect_generation == 0
        {
            return None;
        }
        Some((self.compositor, self.gpu))
    }

    pub(crate) fn into_worker_tickets(
        self,
        expected_session_digest: [u8; 32],
        expected_reconnect_generation: u64,
        policy_digest: &str,
        policy_generation: u64,
        identity_label: &str,
        actions: &[WorkerAction],
    ) -> Option<Vec<LaunchTicket>> {
        if self.session_digest != expected_session_digest
            || self.reconnect_generation != expected_reconnect_generation
            || self.reconnect_generation == 0
        {
            return None;
        }
        let mut compositor = Some(self.compositor);
        let mut gpu = Some(self.gpu);
        let mut frontend_gpu = self.frontend_gpu;
        let mut tickets = Vec::with_capacity(actions.len());
        for action in actions {
            let (role, compositor_grant, gpu_grant) = match action {
                WorkerAction::EnsureProxy => {
                    (DisplayProcessRole::HostProxy, compositor.take(), gpu.take())
                }
                WorkerAction::EnsureFrontend => {
                    (DisplayProcessRole::GuestFrontend, None, frontend_gpu.take())
                }
            };
            let gpu_grant = gpu_grant?;
            tickets.push(
                LaunchTicket::new_for_role(
                    role,
                    compositor_grant,
                    gpu_grant,
                    policy_digest,
                    policy_generation,
                    identity_label,
                )
                .ok()?,
            );
        }
        Some(tickets)
    }
}

impl core::fmt::Debug for LaunchGrants {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("LaunchGrants(<redacted>)")
    }
}

/// Display worker role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DisplayProcessRole {
    /// Jailed Host proxy worker.
    HostProxy,
    /// Guest cross-domain frontend worker.
    GuestFrontend,
}

/// Canonical proxy readiness stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProxyReadinessStage {
    /// Upstream compositor attachment was checked.
    Upstream,
    /// The proxy listener was created.
    Listener,
    /// The first client was accepted.
    FirstClient,
}

/// Canonical proxy readiness state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProxyReadinessState {
    /// The stage is ready.
    Ready,
    /// The stage failed.
    Failed,
}

/// Closed proxy readiness failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProxyReadinessFailure {
    /// The compositor attachment was unavailable.
    UpstreamUnavailable,
    /// The proxy listener could not be created.
    ListenerUnavailable,
    /// No first client arrived before the deadline.
    FirstClientTimeout,
    /// The client failed policy admission.
    ClientRejected,
}

/// Bounded process observation supplied by the Process controller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessObservation {
    /// Host proxy lifecycle evidence.
    pub proxy: WorkerState,
    /// Guest frontend lifecycle evidence.
    pub frontend: WorkerState,
    /// Runtime Volume lifecycle evidence.
    pub volume: VolumeState,
}

impl Default for ProcessObservation {
    fn default() -> Self {
        Self {
            proxy: WorkerState::Starting,
            frontend: WorkerState::Starting,
            volume: VolumeState::Present,
        }
    }
}

impl ProcessObservation {
    /// Construct a fully Ready observation.
    pub const fn ready() -> Self {
        Self {
            proxy: WorkerState::Ready { generation: 1 },
            frontend: WorkerState::Ready { generation: 1 },
            volume: VolumeState::Present,
        }
    }

    /// Construct a failed observation after the supplied retry count.
    pub const fn proxy_failed(proxy_failure_count: u8) -> Self {
        Self {
            proxy: WorkerState::Failed {
                attempts: proxy_failure_count,
            },
            frontend: WorkerState::Failed {
                attempts: proxy_failure_count,
            },
            volume: VolumeState::Present,
        }
    }
}

/// Canonical process template projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProxyProcessTemplate {
    /// Process role.
    pub role: DisplayProcessRole,
    /// Fixed binary name.
    pub binary: &'static str,
    /// Execution domain.
    pub domain: &'static str,
    /// Whether the process has any broker or bus authority after launch.
    pub bus_authority_after_launch: bool,
}

impl ProxyProcessTemplate {
    /// Host proxy template.
    pub const fn host_proxy() -> Self {
        Self {
            role: DisplayProcessRole::HostProxy,
            binary: "d2b-display-wayland-host-proxy",
            domain: "system",
            bus_authority_after_launch: false,
        }
    }

    /// Guest frontend template.
    pub const fn guest_frontend() -> Self {
        Self {
            role: DisplayProcessRole::GuestFrontend,
            binary: "wl-cross-domain-proxy",
            domain: "system",
            bus_authority_after_launch: false,
        }
    }
}

/// Sealed launch ticket composed from opaque attachment handles.
#[derive(PartialEq, Eq)]
pub struct LaunchTicket {
    role: DisplayProcessRole,
    compositor_grant: Option<AttachmentGrantHandle>,
    gpu_grant: AttachmentGrantHandle,
    policy_digest: String,
    policy_generation: u64,
    identity_label: String,
}

impl LaunchTicket {
    /// Construct a launch ticket without accepting paths or raw file
    /// descriptors.
    #[allow(dead_code)]
    pub(crate) fn new(
        compositor_grant: AttachmentGrantHandle,
        gpu_grant: AttachmentGrantHandle,
        policy_digest: impl Into<String>,
        identity_label: impl Into<String>,
    ) -> Result<Self, &'static str> {
        Self::new_with_generation(
            compositor_grant,
            gpu_grant,
            policy_digest,
            0,
            identity_label,
        )
    }

    /// Construct a launch ticket bound to a Core policy generation.
    #[allow(dead_code)]
    pub(crate) fn new_with_generation(
        compositor_grant: AttachmentGrantHandle,
        gpu_grant: AttachmentGrantHandle,
        policy_digest: impl Into<String>,
        policy_generation: u64,
        identity_label: impl Into<String>,
    ) -> Result<Self, &'static str> {
        let policy_digest = policy_digest.into();
        let identity_label = identity_label.into();
        if !policy_digest.starts_with("sha256:")
            || identity_label.is_empty()
            || identity_label.len() > 64
        {
            return Err("display-launch-ticket-invalid");
        }
        Self::new_for_role(
            DisplayProcessRole::HostProxy,
            Some(compositor_grant),
            gpu_grant,
            policy_digest,
            policy_generation,
            identity_label,
        )
    }

    /// Construct one role-specific launch ticket from supervisor grants.
    pub(crate) fn new_for_role(
        role: DisplayProcessRole,
        compositor_grant: Option<AttachmentGrantHandle>,
        gpu_grant: AttachmentGrantHandle,
        policy_digest: impl Into<String>,
        policy_generation: u64,
        identity_label: impl Into<String>,
    ) -> Result<Self, &'static str> {
        let policy_digest = policy_digest.into();
        let identity_label = identity_label.into();
        if !policy_digest.starts_with("sha256:")
            || identity_label.is_empty()
            || identity_label.len() > 64
            || (role == DisplayProcessRole::HostProxy && compositor_grant.is_none())
        {
            return Err("display-launch-ticket-invalid");
        }
        Ok(Self {
            role,
            compositor_grant,
            gpu_grant,
            policy_digest,
            policy_generation,
            identity_label,
        })
    }

    /// Return the independently supervised worker role.
    pub const fn role(&self) -> DisplayProcessRole {
        self.role
    }

    /// Borrow the compositor attachment grant.
    #[allow(dead_code)]
    pub(crate) const fn compositor_grant(&self) -> Option<&AttachmentGrantHandle> {
        self.compositor_grant.as_ref()
    }

    /// Borrow the GPU attachment grant.
    #[allow(dead_code)]
    pub(crate) const fn gpu_grant(&self) -> &AttachmentGrantHandle {
        &self.gpu_grant
    }

    /// Borrow the sealed policy digest.
    pub fn policy_digest(&self) -> &str {
        &self.policy_digest
    }

    /// Return the authenticated policy generation.
    pub const fn policy_generation(&self) -> u64 {
        self.policy_generation
    }

    /// Borrow the bounded identity label.
    pub fn identity_label(&self) -> &str {
        &self.identity_label
    }
}

impl core::fmt::Debug for LaunchTicket {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("LaunchTicket(<redacted>)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supervisor_grants_are_non_cloneable_and_bind_one_launch_ticket() {
        let grants = LaunchGrants::from_supervisor(
            AttachmentGrantHandle::from_supervisor([7; 32]),
            AttachmentGrantHandle::from_supervisor([8; 32]),
        );
        let ticket = LaunchTicket::new_with_generation(
            grants.compositor,
            grants.gpu,
            format!("sha256:{}", "a".repeat(64)),
            3,
            "session",
        )
        .unwrap();
        assert_eq!(ticket.policy_generation(), 3);
        assert_eq!(ticket.identity_label(), "session");
    }

    #[test]
    fn frontend_restart_is_independent_from_a_ready_proxy() {
        let observation = ProcessObservation {
            proxy: WorkerState::Ready { generation: 4 },
            frontend: WorkerState::Failed { attempts: 1 },
            volume: VolumeState::Present,
        };
        assert_eq!(
            WorkerSupervisor::plan(observation, false),
            vec![WorkerAction::EnsureFrontend]
        );
    }

    #[test]
    fn worker_restart_budget_is_independent_per_role() {
        let supervisor = WorkerSupervisor::new(3).unwrap();
        let observation = ProcessObservation {
            proxy: WorkerState::Ready { generation: 4 },
            frontend: WorkerState::Failed { attempts: 3 },
            volume: VolumeState::Present,
        };
        assert_eq!(
            supervisor.plan_with_budget(observation, false),
            Err(WorkerSupervisorError::RetryExhausted(
                DisplayProcessRole::GuestFrontend
            ))
        );
    }

    #[test]
    fn worker_grants_are_consumed_into_independent_role_tickets() {
        let grants = LaunchGrants::from_supervisor_for_session_with_frontend(
            AttachmentGrantHandle::from_supervisor([1; 32]),
            AttachmentGrantHandle::from_supervisor([2; 32]),
            AttachmentGrantHandle::from_supervisor([3; 32]),
            [4; 32],
            9,
        );
        let tickets = grants
            .into_worker_tickets(
                [4; 32],
                9,
                &format!("sha256:{}", "a".repeat(64)),
                2,
                "demo",
                &[WorkerAction::EnsureProxy, WorkerAction::EnsureFrontend],
            )
            .expect("both role grants");
        assert_eq!(tickets.len(), 2);
        assert_eq!(tickets[0].role(), DisplayProcessRole::HostProxy);
        assert_eq!(tickets[1].role(), DisplayProcessRole::GuestFrontend);
        assert!(tickets[0].compositor_grant().is_some());
        assert!(tickets[1].compositor_grant().is_none());
    }
}
