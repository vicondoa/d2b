//! Redacted qemu-media audit events.

use serde::Serialize;
use sha2::{Digest, Sha256};

/// Closed audit event kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditEventKind {
    /// Guest row created.
    GuestCreated,
    /// Runner launch started.
    RunnerLaunching,
    /// QMP became ready.
    QmpReady,
    /// Guest is running.
    GuestRunning,
    /// Guest remains paused at boot.
    GuestPausedAtBoot,
    /// Guest resumed.
    GuestResumed,
    /// Media was hotplugged.
    MediaHotplugAttached,
    /// Media was detached.
    MediaHotplugDetached,
    /// Guest stop started.
    GuestStopping,
    /// Runner exited.
    RunnerExited,
    /// Guest finalization completed.
    GuestDeleted,
    /// Dependency degraded.
    DependencyDegraded,
    /// Provider phase failed.
    ProviderPhaseFailed,
    /// WaylandSession was created.
    WaylandSessionCreated,
    /// WaylandSession was deleted.
    WaylandSessionDeleted,
}

impl AuditEventKind {
    /// All event kinds.
    pub const ALL: [Self; 15] = [
        Self::GuestCreated,
        Self::RunnerLaunching,
        Self::QmpReady,
        Self::GuestRunning,
        Self::GuestPausedAtBoot,
        Self::GuestResumed,
        Self::MediaHotplugAttached,
        Self::MediaHotplugDetached,
        Self::GuestStopping,
        Self::RunnerExited,
        Self::GuestDeleted,
        Self::DependencyDegraded,
        Self::ProviderPhaseFailed,
        Self::WaylandSessionCreated,
        Self::WaylandSessionDeleted,
    ];

    /// Return the stable wire name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::GuestCreated => "guest/created",
            Self::RunnerLaunching => "guest/runner-launching",
            Self::QmpReady => "guest/qmp-ready",
            Self::GuestRunning => "guest/running",
            Self::GuestPausedAtBoot => "guest/paused-at-boot",
            Self::GuestResumed => "guest/resumed",
            Self::MediaHotplugAttached => "guest/media-hotplug-attached",
            Self::MediaHotplugDetached => "guest/media-hotplug-detached",
            Self::GuestStopping => "guest/stopping",
            Self::RunnerExited => "guest/runner-exited",
            Self::GuestDeleted => "guest/deleted",
            Self::DependencyDegraded => "guest/dependency-degraded",
            Self::ProviderPhaseFailed => "guest/provider-phase-failed",
            Self::WaylandSessionCreated => "guest/wayland-session-created",
            Self::WaylandSessionDeleted => "guest/wayland-session-deleted",
        }
    }
}

/// Closed audit outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AuditOutcome {
    /// Mutation or observation succeeded.
    Success,
    /// Operation remains pending.
    Pending,
    /// Operation is degraded.
    Degraded,
    /// Operation failed.
    Failed,
    /// Operation was denied.
    Denied,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AuditWire<'a> {
    kind: &'static str,
    outcome: AuditOutcome,
    zone: &'a str,
    guest: &'a str,
    operation: &'a str,
}

/// Redacted audit record.
#[derive(Clone, PartialEq, Eq)]
pub struct AuditRecord {
    kind: AuditEventKind,
    outcome: AuditOutcome,
    zone_digest: String,
    guest_digest: String,
    operation_digest: String,
}

impl AuditRecord {
    /// Construct an audit record from untrusted identity inputs.
    pub fn new(
        kind: AuditEventKind,
        outcome: AuditOutcome,
        zone: &str,
        guest: &str,
        operation: &str,
    ) -> Self {
        Self {
            kind,
            outcome,
            zone_digest: digest(zone),
            guest_digest: digest(guest),
            operation_digest: digest(operation),
        }
    }

    /// Render the bounded redacted JSON payload.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(&AuditWire {
            kind: self.kind.as_str(),
            outcome: self.outcome,
            zone: &self.zone_digest,
            guest: &self.guest_digest,
            operation: &self.operation_digest,
        })
    }
}

impl core::fmt::Debug for AuditRecord {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("AuditRecord(<redacted>)")
    }
}

fn digest(value: &str) -> String {
    format!("sha256:{:x}", Sha256::digest(value.as_bytes()))
}
