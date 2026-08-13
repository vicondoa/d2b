//! Bounded systemd Provider lifecycle policy.

use d2b_process_conformance::{ProcessExitClass, ProcessOutcome};

/// Provider-level systemd settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SystemdProviderConfig {
    /// Launch deadline in seconds.
    pub launch_timeout_sec: u32,
    /// Termination grace in seconds.
    pub termination_grace_sec: u32,
    /// User-manager reachability deadline in seconds.
    pub user_manager_check_timeout: u32,
    /// Maximum concurrent launch effects.
    pub max_concurrent_launches: u32,
}

impl Default for SystemdProviderConfig {
    fn default() -> Self {
        Self {
            launch_timeout_sec: 30,
            termination_grace_sec: 30,
            user_manager_check_timeout: 5,
            max_concurrent_launches: 64,
        }
    }
}

impl SystemdProviderConfig {
    /// Construct a validated Provider config.
    pub fn new(
        launch_timeout_sec: u32,
        termination_grace_sec: u32,
        user_manager_check_timeout: u32,
        max_concurrent_launches: u32,
    ) -> Result<Self, SystemdConfigError> {
        if !(1..=3600).contains(&launch_timeout_sec)
            || termination_grace_sec > 3600
            || !(1..=60).contains(&user_manager_check_timeout)
            || !(1..=256).contains(&max_concurrent_launches)
        {
            return Err(SystemdConfigError::OutOfRange);
        }
        Ok(Self {
            launch_timeout_sec,
            termination_grace_sec,
            user_manager_check_timeout,
            max_concurrent_launches,
        })
    }

    /// Systemd units are transient and never Provider-owned persistent units.
    pub const fn no_persistent_unit(self) -> bool {
        true
    }
}

/// Invalid systemd Provider configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemdConfigError {
    /// A field exceeded its fixed bound.
    OutOfRange,
}

impl core::fmt::Display for SystemdConfigError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("systemd-provider-config-out-of-range")
    }
}

impl std::error::Error for SystemdConfigError {}

/// Restart-on-failure policy with a bounded counter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RestartPolicy {
    restart_on_failure: bool,
    max_restarts: u32,
    attempts: u32,
    reset_after_ticks: u64,
}

impl RestartPolicy {
    /// Construct a bounded restart-on-failure policy.
    pub const fn on_failure(max_restarts: u32, reset_after_ticks: u64) -> Self {
        Self {
            restart_on_failure: true,
            max_restarts,
            attempts: 0,
            reset_after_ticks,
        }
    }

    /// Decide whether a terminal result may restart the process.
    pub fn should_restart(&mut self, outcome: ProcessOutcome) -> bool {
        if !self.restart_on_failure || outcome.exit_class == ProcessExitClass::CleanExit {
            return false;
        }
        if self.attempts >= self.max_restarts {
            return false;
        }
        self.attempts += 1;
        true
    }

    /// Reset the bounded counter after a healthy interval.
    pub const fn reset_after_ticks(&self) -> u64 {
        self.reset_after_ticks
    }

    /// Return the current attempt count.
    pub const fn attempts(&self) -> u32 {
        self.attempts
    }
}

/// One-shot EphemeralProcess lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EphemeralProcessController {
    ttl_seconds: u64,
    ttl_remaining: Option<u64>,
    incident_hold: bool,
    terminal: Option<ProcessExitClass>,
}

impl EphemeralProcessController {
    /// Construct a one-shot process with a bounded terminal TTL.
    pub const fn new(ttl_seconds: u64, incident_hold: bool) -> Self {
        Self {
            ttl_seconds,
            ttl_remaining: None,
            incident_hold,
            terminal: None,
        }
    }

    /// Consume a typed terminal outcome.
    pub fn observe(&mut self, outcome: ProcessOutcome) -> ProcessExitClass {
        self.terminal = Some(outcome.exit_class);
        self.ttl_remaining = Some(self.ttl_seconds);
        outcome.exit_class
    }

    /// Advance the deterministic TTL clock.
    pub fn tick(&mut self, seconds: u64) {
        if let Some(remaining) = &mut self.ttl_remaining {
            *remaining = remaining.saturating_sub(seconds);
        }
    }

    /// Return the current TTL.
    pub const fn ttl_remaining(&self) -> Option<u64> {
        self.ttl_remaining
    }

    /// Whether core cleanup may issue the Delete request.
    pub const fn cleanup_eligible(&self) -> bool {
        self.terminal.is_some() && !self.incident_hold && matches!(self.ttl_remaining, Some(0))
    }

    /// Ephemeral processes never own persistent systemd units.
    pub const fn owns_persistent_unit(&self) -> bool {
        false
    }
}
