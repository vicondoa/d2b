//! Process and attachment projections for display workers.

use serde::{Deserialize, Serialize};

/// Opaque attachment grant handle resolved by ProviderSupervisor.
#[derive(Clone, PartialEq, Eq)]
pub struct AttachmentGrantHandle([u8; 32]);

impl AttachmentGrantHandle {
    /// Construct a handle at the trusted Core/Supervisor boundary.
    pub const fn from_core(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
}

impl core::fmt::Debug for AttachmentGrantHandle {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("AttachmentGrantHandle(REDACTED)")
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ProcessObservation {
    /// Whether the Host proxy is Ready.
    pub proxy_ready: bool,
    /// Whether the Guest frontend is Ready.
    pub frontend_ready: bool,
    /// Consecutive proxy failures in the current retry window.
    pub proxy_failure_count: u8,
    /// Whether the proxy reached a verified terminal phase.
    pub proxy_terminal: bool,
    /// Whether the proxy Process was deleted by its owner.
    pub proxy_deleted: bool,
    /// Whether the runtime Volume was deleted by its owner.
    pub volume_deleted: bool,
}

impl ProcessObservation {
    /// Construct a fully Ready observation.
    pub const fn ready() -> Self {
        Self {
            proxy_ready: true,
            frontend_ready: true,
            proxy_failure_count: 0,
            proxy_terminal: false,
            proxy_deleted: false,
            volume_deleted: false,
        }
    }

    /// Construct a failed observation after the supplied retry count.
    pub const fn proxy_failed(proxy_failure_count: u8) -> Self {
        Self {
            proxy_ready: false,
            frontend_ready: false,
            proxy_failure_count,
            proxy_terminal: true,
            proxy_deleted: false,
            volume_deleted: false,
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
#[derive(Clone, PartialEq, Eq)]
pub struct LaunchTicket {
    compositor_grant: AttachmentGrantHandle,
    gpu_grant: AttachmentGrantHandle,
    policy_digest: String,
    identity_label: String,
}

impl LaunchTicket {
    /// Construct a launch ticket without accepting paths or raw file
    /// descriptors.
    pub fn new(
        compositor_grant: AttachmentGrantHandle,
        gpu_grant: AttachmentGrantHandle,
        policy_digest: impl Into<String>,
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
        Ok(Self {
            compositor_grant,
            gpu_grant,
            policy_digest,
            identity_label,
        })
    }

    /// Borrow the compositor attachment grant.
    pub const fn compositor_grant(&self) -> &AttachmentGrantHandle {
        &self.compositor_grant
    }

    /// Borrow the GPU attachment grant.
    pub const fn gpu_grant(&self) -> &AttachmentGrantHandle {
        &self.gpu_grant
    }

    /// Borrow the sealed policy digest.
    pub fn policy_digest(&self) -> &str {
        &self.policy_digest
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
