//! Closed clipboard policy and MIME model.

/// The only MIME values admitted to clipboard history.
pub const ALLOWED_MIME_TYPES: &[&str] = &[
    "text/plain;charset=utf-8",
    "text/plain",
    "text/html",
    "image/png",
];

/// MIME hints that suppress capture before any attachment is read.
pub const SECRET_HINT_MIME_TYPES: &[&str] = &[
    "x-kde-passwordmanagerhint",
    "application/x-password",
    "x-secret-content",
];

/// Policy construction failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardPolicyError {
    /// A numeric policy bound is invalid.
    InvalidBounds,
}

impl core::fmt::Display for ClipboardPolicyError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("clipboard-policy-invalid")
    }
}

impl std::error::Error for ClipboardPolicyError {}

/// Zone-local clipboard policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Policy {
    allow_host_capture: bool,
    allow_guest_capture: bool,
    require_picker_for_paste: bool,
    suppress_echo: bool,
    cross_zone_enabled: bool,
    max_history_entries: usize,
    max_item_bytes: usize,
    max_total_bytes: usize,
    max_concurrent_fds: usize,
    max_guest_rate_per_min: u32,
    fd_write_timeout_seconds: u64,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            allow_host_capture: true,
            allow_guest_capture: true,
            require_picker_for_paste: true,
            suppress_echo: true,
            cross_zone_enabled: false,
            max_history_entries: 20,
            max_item_bytes: 8 * 1024 * 1024,
            max_total_bytes: 64 * 1024 * 1024,
            max_concurrent_fds: 32,
            max_guest_rate_per_min: 60,
            fd_write_timeout_seconds: 30,
        }
    }
}

impl Policy {
    /// Validate and construct a custom policy.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        allow_host_capture: bool,
        allow_guest_capture: bool,
        require_picker_for_paste: bool,
        suppress_echo: bool,
        cross_zone_enabled: bool,
        max_history_entries: usize,
        max_item_bytes: usize,
        max_total_bytes: usize,
        max_concurrent_fds: usize,
        max_guest_rate_per_min: u32,
    ) -> Result<Self, ClipboardPolicyError> {
        Self::new_with_fd_write_timeout_seconds(
            allow_host_capture,
            allow_guest_capture,
            require_picker_for_paste,
            suppress_echo,
            cross_zone_enabled,
            max_history_entries,
            max_item_bytes,
            max_total_bytes,
            max_concurrent_fds,
            max_guest_rate_per_min,
            30,
        )
    }

    /// Validate and construct a custom policy with the configured FD
    /// transfer deadline.
    #[allow(clippy::too_many_arguments)]
    pub fn new_with_fd_write_timeout_seconds(
        allow_host_capture: bool,
        allow_guest_capture: bool,
        require_picker_for_paste: bool,
        suppress_echo: bool,
        cross_zone_enabled: bool,
        max_history_entries: usize,
        max_item_bytes: usize,
        max_total_bytes: usize,
        max_concurrent_fds: usize,
        max_guest_rate_per_min: u32,
        fd_write_timeout_seconds: u64,
    ) -> Result<Self, ClipboardPolicyError> {
        if !(1..=200).contains(&max_history_entries)
            || !(4096..=64 * 1024 * 1024).contains(&max_item_bytes)
            || max_total_bytes < max_item_bytes
            || max_total_bytes > 64 * 1024 * 1024
            || !(1..=256).contains(&max_concurrent_fds)
            || max_guest_rate_per_min == 0
            || !(5..=120).contains(&fd_write_timeout_seconds)
        {
            return Err(ClipboardPolicyError::InvalidBounds);
        }
        Ok(Self {
            allow_host_capture,
            allow_guest_capture,
            require_picker_for_paste,
            suppress_echo,
            cross_zone_enabled,
            max_history_entries,
            max_item_bytes,
            max_total_bytes,
            max_concurrent_fds,
            max_guest_rate_per_min,
            fd_write_timeout_seconds,
        })
    }

    /// Check one MIME value against the closed allowlist.
    pub fn allows_mime(&self, mime: &str) -> bool {
        let normalized = normalize_mime(mime);
        ALLOWED_MIME_TYPES.contains(&normalized.as_str())
    }

    /// Check whether any MIME value carries a secret hint.
    pub fn is_secret_hint(mime: &str) -> bool {
        SECRET_HINT_MIME_TYPES.contains(&normalize_mime(mime).as_str())
    }

    /// Whether host capture is enabled.
    pub const fn allow_host_capture(&self) -> bool {
        self.allow_host_capture
    }

    /// Whether guest capture is enabled.
    pub const fn allow_guest_capture(&self) -> bool {
        self.allow_guest_capture
    }

    /// Whether a picker is required before a paste.
    pub const fn require_picker_for_paste(&self) -> bool {
        self.require_picker_for_paste
    }

    /// Whether same-entry echo suppression is enabled.
    pub const fn suppress_echo(&self) -> bool {
        self.suppress_echo
    }

    /// Whether cross-Zone transfer is explicitly enabled.
    pub const fn cross_zone_enabled(&self) -> bool {
        self.cross_zone_enabled
    }

    /// Maximum history entry count.
    pub const fn max_history_entries(&self) -> usize {
        self.max_history_entries
    }

    /// Maximum bytes in one item.
    pub const fn max_item_bytes(&self) -> usize {
        self.max_item_bytes
    }

    /// Maximum total in-memory history bytes.
    pub const fn max_total_bytes(&self) -> usize {
        self.max_total_bytes
    }

    /// Maximum concurrent attachment FDs.
    pub const fn max_concurrent_fds(&self) -> usize {
        self.max_concurrent_fds
    }

    /// Per-Guest materialization rate bound.
    pub const fn max_guest_rate_per_min(&self) -> u32 {
        self.max_guest_rate_per_min
    }

    /// Return the configured FD transfer deadline in seconds.
    pub const fn fd_write_timeout_seconds(&self) -> u64 {
        self.fd_write_timeout_seconds
    }
}

/// Normalize a MIME token without accepting parameters beyond the allowlist.
pub fn normalize_mime(mime: &str) -> String {
    mime.trim().to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::Policy;

    #[test]
    fn fd_transfer_deadline_is_bounded_and_configured() {
        let policy = Policy::new_with_fd_write_timeout_seconds(
            true, true, true, true, false, 3, 4096, 4096, 32, 60, 45,
        )
        .expect("valid timeout");
        assert_eq!(policy.fd_write_timeout_seconds(), 45);
        assert!(
            Policy::new_with_fd_write_timeout_seconds(
                true, true, true, true, false, 3, 4096, 4096, 32, 60, 4,
            )
            .is_err()
        );
        assert!(
            Policy::new_with_fd_write_timeout_seconds(
                true, true, true, true, false, 3, 4096, 4096, 32, 60, 121,
            )
            .is_err()
        );
    }
}
