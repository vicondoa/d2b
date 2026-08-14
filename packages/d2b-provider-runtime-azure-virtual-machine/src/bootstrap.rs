//! One-time bootstrap PSK admission and enrollment state.

use std::fmt;

use zeroize::{Zeroize, Zeroizing};

use crate::error::AzureVmError;

/// A one-time bootstrap PSK held only during delivery.
pub struct BootstrapPsk(Zeroizing<Vec<u8>>);

impl BootstrapPsk {
    /// Construct a bounded PSK.
    pub fn from_bytes(bytes: impl Into<Vec<u8>>) -> Result<Self, AzureVmError> {
        let mut bytes = bytes.into();
        if bytes.is_empty() || bytes.len() > 8_192 {
            bytes.zeroize();
            return Err(AzureVmError::InvalidConfiguration);
        }
        Ok(Self(Zeroizing::new(bytes)))
    }

    /// Compare against a presented PSK without exposing it.
    pub fn matches(&self, presented: &[u8]) -> bool {
        self.0.as_slice() == presented
    }

    /// Consume the secret for a single delivery.
    pub fn consume(self) -> Zeroizing<Vec<u8>> {
        self.0
    }

    /// Copy the bounded secret for an effect attempt without consuming it.
    pub(crate) fn copy_for_delivery(&self) -> Zeroizing<Vec<u8>> {
        Zeroizing::new(self.0.to_vec())
    }
}

impl fmt::Debug for BootstrapPsk {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("BootstrapPsk(<redacted>)")
    }
}

/// Controller-side one-time admission record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootstrapAdmissionState {
    /// Admission can be consumed.
    Pending,
    /// Admission has been consumed.
    Consumed,
    /// Admission expired.
    Expired,
}

/// A single-use bootstrap admission.
pub struct BootstrapAdmission {
    psk: Option<BootstrapPsk>,
    expires_at_unix_ms: u64,
    state: BootstrapAdmissionState,
}

impl BootstrapAdmission {
    /// Create an admission record.
    pub fn new(psk: BootstrapPsk, expires_at_unix_ms: u64) -> Self {
        Self {
            psk: Some(psk),
            expires_at_unix_ms,
            state: BootstrapAdmissionState::Pending,
        }
    }

    /// Consume the PSK if the nonce is fresh and the deadline is valid.
    pub fn consume(
        &mut self,
        presented: &[u8],
        now_unix_ms: u64,
    ) -> Result<Zeroizing<Vec<u8>>, AzureVmError> {
        if now_unix_ms >= self.expires_at_unix_ms {
            self.state = BootstrapAdmissionState::Expired;
            self.psk = None;
            return Err(AzureVmError::BootstrapPskExpired);
        }
        let Some(psk) = self.psk.take() else {
            self.state = BootstrapAdmissionState::Consumed;
            return Err(AzureVmError::BootstrapPskReplayed);
        };
        if !psk.matches(presented) {
            self.psk = Some(psk);
            return Err(AzureVmError::BootstrapEnrollmentFailed);
        }
        self.state = BootstrapAdmissionState::Consumed;
        Ok(psk.consume())
    }

    /// Return the current admission state.
    pub const fn state(&self) -> BootstrapAdmissionState {
        self.state
    }
}
