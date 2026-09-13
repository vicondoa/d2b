//! Status-first restart state with no Provider state Volume.

use std::collections::BTreeMap;

use crate::types::ProviderPhase;

/// Maximum retained per-Guest observations.
pub const MAX_GUEST_OBSERVATIONS: usize = 256;
/// Maximum retained error detail bytes.
pub const MAX_ERROR_DETAIL_BYTES: usize = 256;

/// One redacted observation re-derived from the resource store and process ledger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestObservation {
    /// Current provider phase.
    pub provider_phase: ProviderPhase,
    /// Whether a matching process was observed.
    pub process_present: bool,
    /// Whether QMP health was successful.
    pub qmp_ready: bool,
    /// Bounded error code, if any.
    pub error_code: Option<&'static str>,
}

/// Status-first Provider operational state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeState {
    observations: BTreeMap<String, GuestObservation>,
    reconcile_count: u64,
    last_error: Option<String>,
}

impl Default for RuntimeState {
    fn default() -> Self {
        Self::new()
    }
}

impl RuntimeState {
    /// Construct empty state. No state Volume or path is involved.
    pub fn new() -> Self {
        Self {
            observations: BTreeMap::new(),
            reconcile_count: 0,
            last_error: None,
        }
    }

    /// Record a bounded Guest observation.
    pub fn record(
        &mut self,
        guest_key: impl Into<String>,
        observation: GuestObservation,
    ) -> Result<(), StateError> {
        let guest_key = guest_key.into();
        if self.observations.len() >= MAX_GUEST_OBSERVATIONS
            && !self.observations.contains_key(&guest_key)
        {
            return Err(StateError::TooManyGuests);
        }
        if guest_key.is_empty() || guest_key.len() > 128 || guest_key.contains('/') {
            return Err(StateError::InvalidGuestKey);
        }
        self.observations.insert(guest_key, observation);
        self.reconcile_count = self.reconcile_count.saturating_add(1);
        Ok(())
    }

    /// Record a bounded error code without attacker-authored text.
    pub fn record_error(&mut self, code: impl Into<String>) -> Result<(), StateError> {
        let code = code.into();
        if code.is_empty()
            || code.len() > MAX_ERROR_DETAIL_BYTES
            || !code.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
            })
        {
            return Err(StateError::InvalidErrorCode);
        }
        self.last_error = Some(code);
        Ok(())
    }

    /// Borrow the status observation for a Guest.
    pub fn observation(&self, guest_key: &str) -> Option<&GuestObservation> {
        self.observations.get(guest_key)
    }

    /// Return the bounded reconcile count.
    pub const fn reconcile_count(&self) -> u64 {
        self.reconcile_count
    }

    /// Return whether a Provider state Volume is required.
    pub const fn requires_state_volume(&self) -> bool {
        false
    }

    /// Return whether the controller can reconstruct state from external observations.
    pub const fn restart_rehydratable(&self) -> bool {
        true
    }
}

/// Status-first state failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateError {
    /// Too many Guest observations are retained.
    TooManyGuests,
    /// Guest key is not a bounded opaque key.
    InvalidGuestKey,
    /// Error code is not a closed bounded code.
    InvalidErrorCode,
}
