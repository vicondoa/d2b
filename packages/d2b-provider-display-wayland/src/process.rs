//! Process and attachment projections for display workers.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

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

    /// Return the readiness generation, when the worker is Ready.
    pub const fn generation(self) -> Option<u64> {
        match self {
            Self::Ready { generation } => Some(generation),
            _ => None,
        }
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

/// Core-observed timing and teardown evidence used for restart fencing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkerRestartEvidence {
    /// Monotonic observation time in milliseconds.
    pub(crate) observed_at_ms: u64,
    /// Last proxy failure time in the current observation window.
    pub(crate) proxy_last_failure_ms: Option<u64>,
    /// Last frontend failure time in the current observation window.
    pub(crate) frontend_last_failure_ms: Option<u64>,
    /// Monotonic teardown generation fencing stale launch actions.
    pub(crate) teardown_generation: u64,
}

impl WorkerRestartEvidence {
    /// Construct restart evidence emitted by the Core-owned worker
    /// supervisor.
    ///
    /// The controller consumes this typed observation for retry-window and
    /// teardown fencing. It is not a readiness or identity grant; those
    /// remain bound to the authenticated ComponentSession and worker
    /// handshakes.
    pub const fn from_supervisor(
        observed_at_ms: u64,
        proxy_last_failure_ms: Option<u64>,
        frontend_last_failure_ms: Option<u64>,
        teardown_generation: u64,
    ) -> Self {
        Self {
            observed_at_ms,
            proxy_last_failure_ms,
            frontend_last_failure_ms,
            teardown_generation,
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    /// Construct bounded retry evidence for hermetic model tests.
    pub const fn for_test(
        observed_at_ms: u64,
        proxy_last_failure_ms: Option<u64>,
        frontend_last_failure_ms: Option<u64>,
        teardown_generation: u64,
    ) -> Self {
        Self::from_supervisor(
            observed_at_ms,
            proxy_last_failure_ms,
            frontend_last_failure_ms,
            teardown_generation,
        )
    }
}

#[cfg(any(test, feature = "test-support"))]
impl Default for WorkerRestartEvidence {
    fn default() -> Self {
        Self::for_test(0, None, None, 1)
    }
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
    retry_window_ms: u64,
    retry_backoff_ms: u64,
}

impl WorkerSupervisor {
    /// Default consecutive restart bound for one worker.
    pub const DEFAULT_MAX_ATTEMPTS: u8 = 5;
    /// Default bounded retry window.
    pub const DEFAULT_RETRY_WINDOW_MS: u64 = 30_000;
    /// Default delay between failed restart attempts.
    pub const DEFAULT_RETRY_BACKOFF_MS: u64 = 250;

    /// Construct a bounded worker supervisor.
    pub const fn new(max_attempts: u8) -> Option<Self> {
        if max_attempts == 0 {
            None
        } else {
            Some(Self {
                max_attempts,
                retry_window_ms: Self::DEFAULT_RETRY_WINDOW_MS,
                retry_backoff_ms: Self::DEFAULT_RETRY_BACKOFF_MS,
            })
        }
    }

    /// Construct a bounded supervisor with explicit retry timing.
    pub const fn with_policy(
        max_attempts: u8,
        retry_window_ms: u64,
        retry_backoff_ms: u64,
    ) -> Option<Self> {
        if max_attempts == 0 || retry_window_ms == 0 {
            None
        } else {
            Some(Self {
                max_attempts,
                retry_window_ms,
                retry_backoff_ms,
            })
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

    /// Plan actions with Core-observed retry timing and teardown fencing.
    pub fn plan_with_evidence(
        &self,
        observation: ProcessObservation,
        policy_changed: bool,
        evidence: WorkerRestartEvidence,
    ) -> Result<Vec<WorkerAction>, WorkerSupervisorError> {
        let actions = Self::plan(observation, policy_changed);
        let mut admissible = Vec::with_capacity(actions.len());
        for action in actions {
            let (role, attempts, last_failure) = match action {
                WorkerAction::EnsureProxy => (
                    DisplayProcessRole::HostProxy,
                    observation.proxy.failure_count(),
                    evidence.proxy_last_failure_ms,
                ),
                WorkerAction::EnsureFrontend => (
                    DisplayProcessRole::GuestFrontend,
                    observation.frontend.failure_count(),
                    evidence.frontend_last_failure_ms,
                ),
            };
            let attempts_in_window = if last_failure.is_some_and(|failure| {
                failure <= evidence.observed_at_ms
                    && evidence.observed_at_ms.saturating_sub(failure) <= self.retry_window_ms
            }) {
                attempts
            } else {
                0
            };
            if attempts_in_window >= self.max_attempts {
                return Err(WorkerSupervisorError::RetryExhausted(role));
            }
            if last_failure.is_some_and(|failure| {
                evidence.observed_at_ms < failure.saturating_add(self.retry_backoff_ms)
            }) {
                continue;
            }
            admissible.push(action);
        }
        Ok(admissible)
    }
}

/// Opaque attachment grant handle resolved by ProviderSupervisor.
#[derive(PartialEq, Eq)]
pub struct AttachmentGrantHandle([u8; 32]);

impl AttachmentGrantHandle {
    /// Construct a handle at the private Core/Supervisor boundary.
    pub(crate) const fn from_supervisor(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Construct an opaque fixture handle for daemon conformance tests.
    #[cfg(any(feature = "daemon-support", test))]
    pub const fn from_daemon(bytes: [u8; 32]) -> Self {
        Self::from_supervisor(bytes)
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
    teardown_generation: u64,
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
            teardown_generation: 1,
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
            teardown_generation: 1,
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
        teardown_generation: u64,
    ) -> Self {
        Self {
            compositor,
            gpu,
            frontend_gpu: Some(frontend_gpu),
            session_digest,
            reconnect_generation,
            teardown_generation,
        }
    }

    /// Construct one daemon-issued grant bundle.
    ///
    /// This constructor is available only to the daemon adapter feature (and
    /// hermetic tests).  The values are opaque grant commitments; they are
    /// consumed into a single [`LaunchTicket`] before a process effect is
    /// attempted.
    #[cfg(any(feature = "daemon-support", feature = "test-support"))]
    pub const fn from_daemon(
        compositor: [u8; 32],
        gpu: [u8; 32],
        frontend_gpu: [u8; 32],
        session_digest: [u8; 32],
        reconnect_generation: u64,
        teardown_generation: u64,
    ) -> Self {
        Self::from_supervisor_for_session_with_frontend(
            AttachmentGrantHandle::from_supervisor(compositor),
            AttachmentGrantHandle::from_supervisor(gpu),
            AttachmentGrantHandle::from_supervisor(frontend_gpu),
            session_digest,
            reconnect_generation,
            teardown_generation,
        )
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

    #[allow(dead_code)]
    pub(crate) fn into_worker_tickets(
        self,
        expected_session_digest: [u8; 32],
        expected_reconnect_generation: u64,
        policy_digest: &str,
        policy_generation: u64,
        identity_label: &str,
        actions: &[WorkerAction],
    ) -> Option<Vec<LaunchTicket>> {
        self.into_worker_tickets_with_fence(
            expected_session_digest,
            expected_reconnect_generation,
            1,
            policy_digest,
            policy_generation,
            identity_label,
            actions,
        )
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "the sealed launch boundary keeps all session and fence evidence explicit"
    )]
    pub(crate) fn into_worker_tickets_with_fence(
        self,
        expected_session_digest: [u8; 32],
        expected_reconnect_generation: u64,
        expected_teardown_generation: u64,
        policy_digest: &str,
        policy_generation: u64,
        identity_label: &str,
        actions: &[WorkerAction],
    ) -> Option<Vec<LaunchTicket>> {
        if self.session_digest != expected_session_digest
            || self.reconnect_generation != expected_reconnect_generation
            || self.teardown_generation != expected_teardown_generation
            || self.reconnect_generation == 0
            || self.teardown_generation == 0
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
                    expected_teardown_generation,
                )
                .ok()?,
            );
        }
        Some(tickets)
    }
}

/// Daemon-only, grant-free binding produced by consuming a display launch
/// ticket.
///
/// The attachment grants are reduced to commitments before crossing into the
/// daemon composition layer.  No file descriptor, path, process handle, or
/// raw attachment authority is exposed.
#[cfg(any(feature = "daemon-support", feature = "test-support"))]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct DisplayLaunchBinding {
    role: DisplayProcessRole,
    attachment_digest: [u8; 32],
    policy_digest: [u8; 32],
    policy_generation: u64,
    teardown_generation: u64,
}

#[cfg(any(feature = "daemon-support", feature = "test-support"))]
impl DisplayLaunchBinding {
    /// Consume one ticket into a daemon-owned commitment.
    pub fn from_ticket(ticket: LaunchTicket) -> Self {
        let mut digest = Sha256::new();
        digest.update(b"d2b-display-launch-binding-v1");
        digest.update((ticket.role as u8).to_be_bytes());
        digest.update(ticket.gpu_grant.0);
        if let Some(compositor) = ticket.compositor_grant {
            digest.update(compositor.0);
        }
        let attachment_digest = digest.finalize().into();
        let policy_digest = digest_policy(ticket.policy_digest.as_bytes());
        Self {
            role: ticket.role,
            attachment_digest,
            policy_digest,
            policy_generation: ticket.policy_generation,
            teardown_generation: ticket.teardown_generation,
        }
    }

    /// Return the worker role.
    pub const fn role(self) -> DisplayProcessRole {
        self.role
    }

    /// Return the opaque attachment commitment.
    pub const fn attachment_digest(self) -> [u8; 32] {
        self.attachment_digest
    }

    /// Return the compiled policy digest.
    pub const fn policy_digest(self) -> [u8; 32] {
        self.policy_digest
    }

    /// Return the policy generation.
    pub const fn policy_generation(self) -> u64 {
        self.policy_generation
    }

    /// Return the teardown generation.
    pub const fn teardown_generation(self) -> u64 {
        self.teardown_generation
    }
}

#[cfg(any(feature = "daemon-support", feature = "test-support"))]
impl core::fmt::Debug for DisplayLaunchBinding {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("DisplayLaunchBinding(<redacted>)")
    }
}

#[cfg(any(feature = "daemon-support", feature = "test-support"))]
fn digest_policy(policy_digest: &[u8]) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"d2b-display-policy-binding-v1");
    digest.update(policy_digest);
    digest.finalize().into()
}

impl core::fmt::Debug for LaunchGrants {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("LaunchGrants(<redacted>)")
    }
}

/// Display worker role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
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
    pub(crate) proxy: WorkerState,
    /// Guest frontend lifecycle evidence.
    pub(crate) frontend: WorkerState,
    /// Runtime Volume lifecycle evidence.
    pub(crate) volume: VolumeState,
    /// Policy generation proved by the worker readiness handshakes.
    pub(crate) policy_generation: u64,
    /// Teardown generation proved by the worker readiness handshakes.
    pub(crate) teardown_generation: u64,
    /// Session binding digest proved by the worker readiness handshakes.
    pub(crate) session_digest: [u8; 32],
}

#[cfg(any(test, feature = "test-support"))]
impl Default for ProcessObservation {
    fn default() -> Self {
        Self {
            proxy: WorkerState::Starting,
            frontend: WorkerState::Starting,
            volume: VolumeState::Present,
            policy_generation: 0,
            teardown_generation: 0,
            session_digest: [0; 32],
        }
    }
}

impl ProcessObservation {
    #[allow(dead_code)]
    pub(crate) const fn from_supervisor(
        proxy: WorkerState,
        frontend: WorkerState,
        volume: VolumeState,
        policy_generation: u64,
        teardown_generation: u64,
        session_digest: [u8; 32],
    ) -> Self {
        Self {
            proxy,
            frontend,
            volume,
            policy_generation,
            teardown_generation,
            session_digest,
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    /// Construct a fully Ready observation.
    pub const fn ready() -> Self {
        Self::ready_for(0, 0)
    }

    #[cfg(any(test, feature = "test-support"))]
    /// Construct a Ready observation bound to one policy and teardown
    /// generation.
    pub const fn ready_for(policy_generation: u64, teardown_generation: u64) -> Self {
        Self::from_supervisor(
            WorkerState::Ready { generation: 1 },
            WorkerState::Ready { generation: 1 },
            VolumeState::Present,
            policy_generation,
            teardown_generation,
            [0; 32],
        )
    }

    #[cfg(any(test, feature = "test-support"))]
    /// Construct a Ready observation bound to the exact display session.
    pub fn ready_for_session(
        spec: &crate::WaylandSessionSpec,
        policy_generation: u64,
        teardown_generation: u64,
    ) -> Self {
        Self {
            session_digest: crate::controller::session_digest(spec, 0),
            ..Self::ready_for(policy_generation, teardown_generation)
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    /// Construct a failed observation after the supplied retry count.
    pub const fn proxy_failed(proxy_failure_count: u8) -> Self {
        Self::from_supervisor(
            WorkerState::Failed {
                attempts: proxy_failure_count,
            },
            WorkerState::Failed {
                attempts: proxy_failure_count,
            },
            VolumeState::Present,
            0,
            0,
            [0; 32],
        )
    }

    /// Whether both workers proved the requested policy and teardown fence.
    pub fn workers_ready_for(
        &self,
        policy_generation: u64,
        teardown_generation: u64,
        session_digest: [u8; 32],
    ) -> bool {
        policy_generation != 0
            && teardown_generation != 0
            && session_digest != [0; 32]
            && self.policy_generation == policy_generation
            && self.teardown_generation == teardown_generation
            && self.session_digest == session_digest
            && self.proxy.is_ready()
            && self.frontend.is_ready()
    }

    /// Whether both supervised workers hold a non-zero current fence.
    pub fn is_ready(&self) -> bool {
        self.policy_generation != 0
            && self.teardown_generation != 0
            && self.session_digest != [0; 32]
            && self.proxy.is_ready()
            && self.frontend.is_ready()
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
    teardown_generation: u64,
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
            1,
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
        teardown_generation: u64,
    ) -> Result<Self, &'static str> {
        let policy_digest = policy_digest.into();
        let identity_label = identity_label.into();
        if !policy_digest.starts_with("sha256:")
            || identity_label.is_empty()
            || identity_label.len() > 64
            || teardown_generation == 0
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
            teardown_generation,
        })
    }

    /// Construct an opaque role ticket for daemon conformance tests.
    #[cfg(any(feature = "daemon-support", test))]
    pub fn new_for_daemon(
        role: DisplayProcessRole,
        compositor_grant: Option<AttachmentGrantHandle>,
        gpu_grant: AttachmentGrantHandle,
        policy_digest: impl Into<String>,
        policy_generation: u64,
        identity_label: impl Into<String>,
        teardown_generation: u64,
    ) -> Result<Self, &'static str> {
        Self::new_for_role(
            role,
            compositor_grant,
            gpu_grant,
            policy_digest,
            policy_generation,
            identity_label,
            teardown_generation,
        )
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

    /// Return the teardown generation fencing this launch.
    pub const fn teardown_generation(&self) -> u64 {
        self.teardown_generation
    }

    /// Whether the ticket is current for one Core teardown generation.
    pub const fn is_current(&self, teardown_generation: u64) -> bool {
        self.teardown_generation == teardown_generation && teardown_generation != 0
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
            policy_generation: 1,
            teardown_generation: 1,
            session_digest: [0; 32],
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
            policy_generation: 0,
            teardown_generation: 0,
            session_digest: [0; 32],
        };
        assert_eq!(
            supervisor.plan_with_budget(observation, false),
            Err(WorkerSupervisorError::RetryExhausted(
                DisplayProcessRole::GuestFrontend
            ))
        );
    }

    #[test]
    fn restart_backoff_and_window_are_core_observed() {
        let supervisor = WorkerSupervisor::with_policy(3, 1_000, 250).unwrap();
        let observation = ProcessObservation {
            proxy: WorkerState::Failed { attempts: 1 },
            frontend: WorkerState::Ready { generation: 1 },
            volume: VolumeState::Present,
            policy_generation: 1,
            teardown_generation: 1,
            session_digest: [0; 32],
        };
        let evidence = WorkerRestartEvidence {
            observed_at_ms: 100,
            proxy_last_failure_ms: Some(0),
            frontend_last_failure_ms: None,
            teardown_generation: 4,
        };
        assert!(
            supervisor
                .plan_with_evidence(observation, false, evidence)
                .unwrap()
                .is_empty()
        );
        let recovered = WorkerRestartEvidence {
            observed_at_ms: 300,
            ..evidence
        };
        assert_eq!(
            supervisor
                .plan_with_evidence(observation, false, recovered)
                .unwrap(),
            vec![WorkerAction::EnsureProxy]
        );
        let outside_window = WorkerRestartEvidence {
            observed_at_ms: 2_000,
            ..evidence
        };
        assert_eq!(
            supervisor
                .plan_with_evidence(
                    ProcessObservation {
                        proxy: WorkerState::Failed { attempts: 3 },
                        ..observation
                    },
                    false,
                    outside_window,
                )
                .unwrap(),
            vec![WorkerAction::EnsureProxy]
        );
    }

    #[test]
    fn future_failure_evidence_does_not_exhaust_the_retry_window() {
        let supervisor = WorkerSupervisor::with_policy(3, 1_000, 250).unwrap();
        let observation = ProcessObservation {
            proxy: WorkerState::Failed { attempts: 3 },
            frontend: WorkerState::Ready { generation: 1 },
            volume: VolumeState::Present,
            policy_generation: 1,
            teardown_generation: 1,
            session_digest: [0; 32],
        };
        let evidence = WorkerRestartEvidence {
            observed_at_ms: 100,
            proxy_last_failure_ms: Some(200),
            frontend_last_failure_ms: None,
            teardown_generation: 1,
        };
        assert!(
            supervisor
                .plan_with_evidence(observation, false, evidence)
                .is_ok()
        );
    }

    #[test]
    fn launch_tickets_are_fenced_to_the_teardown_generation() {
        let grants = LaunchGrants::from_supervisor_for_session_with_frontend(
            AttachmentGrantHandle::from_supervisor([1; 32]),
            AttachmentGrantHandle::from_supervisor([2; 32]),
            AttachmentGrantHandle::from_supervisor([3; 32]),
            [4; 32],
            9,
            2,
        );
        assert!(
            grants
                .into_worker_tickets_with_fence(
                    [4; 32],
                    9,
                    1,
                    &format!("sha256:{}", "a".repeat(64)),
                    2,
                    "demo",
                    &[WorkerAction::EnsureProxy],
                )
                .is_none()
        );
        let grants = LaunchGrants::from_supervisor_for_session_with_frontend(
            AttachmentGrantHandle::from_supervisor([1; 32]),
            AttachmentGrantHandle::from_supervisor([2; 32]),
            AttachmentGrantHandle::from_supervisor([3; 32]),
            [4; 32],
            9,
            2,
        );
        let tickets = grants
            .into_worker_tickets_with_fence(
                [4; 32],
                9,
                2,
                &format!("sha256:{}", "a".repeat(64)),
                2,
                "demo",
                &[WorkerAction::EnsureProxy],
            )
            .unwrap();
        assert!(tickets[0].is_current(2));
        assert!(!tickets[0].is_current(1));
    }

    #[test]
    fn worker_grants_are_consumed_into_independent_role_tickets() {
        let grants = LaunchGrants::from_supervisor_for_session_with_frontend(
            AttachmentGrantHandle::from_supervisor([1; 32]),
            AttachmentGrantHandle::from_supervisor([2; 32]),
            AttachmentGrantHandle::from_supervisor([3; 32]),
            [4; 32],
            9,
            1,
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
