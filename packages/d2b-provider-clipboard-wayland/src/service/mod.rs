//! clipd-host service boundary and display dependency.

use crate::{
    DependencyStatus,
    audit::{ClipboardAuditEvent, ClipboardAuditQueue, ClipboardReason, SizeBucket},
    history::{ClipboardEntry, ClipboardHistory},
    policy::Policy,
};

/// Typed display dependency observation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayDependency {
    /// Whether the display Provider is absent or Ready.
    pub status: DependencyStatus,
    /// The typed service contract consumed by clipd-host.
    pub service_contract: &'static str,
}

/// Bridge effect port. Clipboard payloads are represented by attachments in
/// the real adapter; this trait never accepts a path or a raw compositor
/// socket.
pub trait ClipboardBridgePort {
    /// Notify the display bridge of a Guest selection without payload bytes.
    fn notify_guest_selection(
        &mut self,
        guest: &str,
        mime: &str,
    ) -> Result<(), ClipboardServiceError>;
    /// Cancel one opaque entry.
    fn cancel_entry(&mut self, token: &str) -> Result<(), ClipboardServiceError>;
}

/// Service failures with stable content-free codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardServiceError {
    /// Display dependency is absent or not Ready.
    DependencyUnavailable,
    /// Cross-Zone transfer is denied.
    CrossZoneDenied,
    /// Guest is suspended.
    GuestSuspended,
    /// Audit queue is full.
    AuditUnavailable,
    /// History rejected the item.
    HistoryRejected,
    /// A picker is required before materialization.
    PickerRequired,
}

impl core::fmt::Display for ClipboardServiceError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::DependencyUnavailable => "dependency-unavailable",
            Self::CrossZoneDenied => "cross-zone-denied",
            Self::GuestSuspended => "zone-suspended",
            Self::AuditUnavailable => "audit-unavailable",
            Self::HistoryRejected => "clipboard-history-rejected",
            Self::PickerRequired => "picker-required",
        })
    }
}

impl std::error::Error for ClipboardServiceError {}

/// In-memory clipboard service.
pub struct ClipdHost {
    policy: Policy,
    history: ClipboardHistory,
    audit: ClipboardAuditQueue,
    dependency: DisplayDependency,
}

impl ClipdHost {
    /// Construct clipd-host with an optional display dependency.
    pub fn new(
        policy: Policy,
        audit_capacity: usize,
        display_ready: Option<bool>,
    ) -> Result<Self, ClipboardServiceError> {
        let history = ClipboardHistory::new(crate::ClipboardConfig::from_policy(policy.clone()))
            .map_err(|_| ClipboardServiceError::HistoryRejected)?;
        let status = match display_ready {
            None => DependencyStatus::Absent,
            Some(true) => DependencyStatus::Ready,
            Some(false) => DependencyStatus::Degraded,
        };
        Ok(Self {
            policy,
            history,
            audit: ClipboardAuditQueue::new(audit_capacity),
            dependency: DisplayDependency {
                status,
                service_contract: "d2b.display.host-clipboard.v3",
            },
        })
    }

    /// Return the typed display dependency state.
    pub const fn dependency(&self) -> &DisplayDependency {
        &self.dependency
    }

    /// Capture one Guest selection after audit admission.
    pub fn capture_guest(
        &mut self,
        guest: &str,
        mime: &str,
        bytes: &[u8],
        now_secs: u64,
    ) -> Result<String, ClipboardServiceError> {
        if self.dependency.status != DependencyStatus::Ready {
            return Err(ClipboardServiceError::DependencyUnavailable);
        }
        if !self.policy.allow_guest_capture() {
            return Err(ClipboardServiceError::HistoryRejected);
        }
        self.history
            .record_guest_request(guest, now_secs)
            .map_err(|_| ClipboardServiceError::HistoryRejected)?;
        let entry = ClipboardEntry::new(guest, mime, bytes, now_secs)
            .map_err(|_| ClipboardServiceError::HistoryRejected)?;
        let token = entry.token().to_owned();
        if self.audit.is_full() {
            return Err(ClipboardServiceError::AuditUnavailable);
        }
        self.history
            .insert(entry)
            .map_err(|_| ClipboardServiceError::HistoryRejected)?;
        let event = ClipboardAuditEvent::new(
            "guest",
            "host",
            ClipboardReason::Allowed,
            SizeBucket::from_len(bytes.len()),
        );
        self.audit
            .push(event)
            .map_err(|_| ClipboardServiceError::AuditUnavailable)?;
        Ok(token)
    }

    /// Suspend a Guest and revoke its paste authority.
    pub fn suspend_guest(&mut self, guest: &str) {
        self.history.suspend_guest(guest);
    }

    /// Resume a Guest.
    pub fn resume_guest(&mut self, guest: &str) {
        self.history.resume_guest(guest);
    }

    /// Purge all Guest-owned entries on lifecycle destruction.
    pub fn purge_guest(&mut self, guest: &str) {
        self.history.purge_guest(guest);
    }

    /// Check whether a cross-Zone route is allowed.
    pub const fn cross_zone_allowed(&self) -> bool {
        self.policy.cross_zone_enabled()
    }

    /// Check a paste route before any attachment is requested.
    pub fn authorize_paste(
        &self,
        source_zone: &str,
        destination_zone: &str,
        guest: &str,
    ) -> Result<(), ClipboardServiceError> {
        self.authorize_paste_inner(source_zone, destination_zone, guest, false)
    }

    /// Check a paste route after the authenticated picker completed.
    pub fn authorize_paste_after_picker(
        &self,
        source_zone: &str,
        destination_zone: &str,
        guest: &str,
    ) -> Result<(), ClipboardServiceError> {
        self.authorize_paste_inner(source_zone, destination_zone, guest, true)
    }

    fn authorize_paste_inner(
        &self,
        source_zone: &str,
        destination_zone: &str,
        guest: &str,
        picker_completed: bool,
    ) -> Result<(), ClipboardServiceError> {
        if self.dependency.status != DependencyStatus::Ready {
            return Err(ClipboardServiceError::DependencyUnavailable);
        }
        if source_zone != destination_zone && !self.policy.cross_zone_enabled() {
            return Err(ClipboardServiceError::CrossZoneDenied);
        }
        self.history
            .authorize_guest(guest)
            .map_err(|_| ClipboardServiceError::GuestSuspended)
            .and_then(|()| {
                if self.policy.require_picker_for_paste() && !picker_completed {
                    Err(ClipboardServiceError::PickerRequired)
                } else {
                    Ok(())
                }
            })
    }

    /// Return bounded history size.
    pub fn history_len(&self) -> usize {
        self.history.len()
    }
}

/// Provider configuration used by history and service components.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardConfig {
    policy: Policy,
    host_entry_ttl_secs: u64,
    guest_entry_ttl_secs: u64,
}

impl Default for ClipboardConfig {
    fn default() -> Self {
        Self {
            policy: Policy::default(),
            host_entry_ttl_secs: 3600,
            guest_entry_ttl_secs: 3600,
        }
    }
}

impl ClipboardConfig {
    /// Construct configuration from a policy.
    pub fn from_policy(policy: Policy) -> Self {
        Self {
            policy,
            ..Self::default()
        }
    }

    /// Return the policy.
    pub const fn policy(&self) -> &Policy {
        &self.policy
    }

    /// Return item byte limit.
    pub const fn max_item_bytes(&self) -> usize {
        self.policy.max_item_bytes()
    }

    /// Return total byte limit.
    pub const fn max_total_bytes(&self) -> usize {
        self.policy.max_total_bytes()
    }

    /// Return history entry bound.
    pub const fn max_history_entries(&self) -> usize {
        self.policy.max_history_entries()
    }

    /// Return per-Guest rate limit.
    pub const fn max_guest_rate_per_min(&self) -> u32 {
        self.policy.max_guest_rate_per_min()
    }

    /// Return Host entry TTL.
    pub const fn host_entry_ttl_secs(&self) -> u64 {
        self.host_entry_ttl_secs
    }

    /// Return Guest entry TTL.
    pub const fn guest_entry_ttl_secs(&self) -> u64 {
        self.guest_entry_ttl_secs
    }
}
