//! Redacted authoritative display audit records.

use sha2::{Digest, Sha256};

/// Closed display audit operation kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayAuditKind {
    /// A session was created.
    SessionCreated,
    /// A session finalizer completed.
    SessionFinalized,
    /// A proxy started.
    ProxyStarted,
    /// A proxy exited.
    ProxyExited,
    /// A policy advisory was produced.
    PolicyAdvisory,
    /// A policy was compiled.
    PolicyCompiled,
}

impl DisplayAuditKind {
    /// Return the stable audit kind string.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SessionCreated => "display-wayland/session-created",
            Self::SessionFinalized => "display-wayland/session-finalized",
            Self::ProxyStarted => "display-wayland/proxy-started",
            Self::ProxyExited => "display-wayland/proxy-exited",
            Self::PolicyAdvisory => "display-wayland/policy-advisory",
            Self::PolicyCompiled => "display-wayland/policy-compiled",
        }
    }
}

/// Closed audit outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayAuditOutcome {
    /// Operation succeeded.
    Success,
    /// Operation was rejected.
    Denied,
    /// Operation remains pending.
    Pending,
    /// Operation degraded.
    Degraded,
    /// Operation failed.
    Failed,
}

impl DisplayAuditOutcome {
    /// Return the stable outcome string.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Denied => "denied",
            Self::Pending => "pending",
            Self::Degraded => "degraded",
            Self::Failed => "failed",
        }
    }
}

/// Audit record with all identity values reduced to bounded digests.
pub struct DisplayAuditRecord {
    kind: DisplayAuditKind,
    outcome: DisplayAuditOutcome,
    zone_digest: String,
    resource_digest: String,
    subject_digest: String,
    operation_digest: String,
    warning: Option<String>,
}

impl DisplayAuditRecord {
    /// Construct a redacted record.
    pub fn new(
        kind: DisplayAuditKind,
        outcome: DisplayAuditOutcome,
        zone: &str,
        resource: &str,
        subject: &str,
        operation_id: &str,
    ) -> Self {
        Self {
            kind,
            outcome,
            zone_digest: digest(zone),
            resource_digest: digest(resource),
            subject_digest: digest(subject),
            operation_digest: digest(operation_id),
            warning: None,
        }
    }

    /// Add a bounded closed warning code and interface name.
    pub fn with_warning(mut self, warning: &str, interface: &str) -> Self {
            let warning = sanitize_component(warning);
            let interface = sanitize_component(interface);
            self.warning = Some(format!("{warning}:{interface}"));
            self
    }

    /// Render a path-free audit payload.
    pub fn to_wire_record(&self) -> String {
            format!(
                "kind={} outcome={} zone={} resource={} subject={} operation={} warning={}",
                self.kind.as_str(),
                self.outcome.as_str(),
                self.zone_digest,
                self.resource_digest,
                self.subject_digest,
                self.operation_digest,
                self.warning.as_deref().unwrap_or("none")
            )
    }
}

fn sanitize_component(value: &str) -> String {
    value
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric()
                    || matches!(character, '-' | '_' | '.' | '/')
                {
                    character
                } else {
                    '_'
                }
            })
            .take(63)
            .collect::<String>()
}

impl core::fmt::Debug for DisplayAuditRecord {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("DisplayAuditRecord(<redacted>)")
    }
}

fn digest(value: &str) -> String {
    format!("sha256:{:x}", Sha256::digest(value.as_bytes()))
}
