//! One-time bootstrap PSK admission and enrollment state.

use std::fmt;

use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, Zeroizing};

use crate::error::AzureVmError;

/// A one-time bootstrap PSK held only during delivery.
pub struct BootstrapPsk(Zeroizing<Vec<u8>>);

impl BootstrapPsk {
    /// Construct a bounded PSK.
    ///
    /// # Errors
    ///
    /// Returns [`AzureVmError::InvalidConfiguration`] when the secret is
    /// empty or exceeds the 8192-byte bound.
    pub fn from_bytes(bytes: impl Into<Vec<u8>>) -> Result<Self, AzureVmError> {
        let mut bytes = bytes.into();
        if bytes.is_empty() || bytes.len() > 8_192 {
            bytes.zeroize();
            return Err(AzureVmError::InvalidConfiguration);
        }
        Ok(Self(Zeroizing::new(bytes)))
    }

    /// Compare against a presented PSK without exposing it.
    ///
    /// The comparison is constant-time in the presented length: it walks
    /// the longer of the two secrets with zero padding and never exits
    /// early on a mismatch.
    pub fn matches(&self, presented: &[u8]) -> bool {
        let mut difference = self.0.len() ^ presented.len();
        let length = self.0.len().max(presented.len());
        for index in 0..length {
            let expected = self.0.get(index).copied().unwrap_or(0);
            let actual = presented.get(index).copied().unwrap_or(0);
            difference |= usize::from(expected ^ actual);
        }
        difference == 0
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
///
/// The admission is one of three closed states: a `Pending` admission
/// carries its secret and deadline; `Consumed` and `Expired` carry neither,
/// so an expired or replayed admission cannot be constructed with a
/// still-present secret.
pub enum BootstrapAdmission {
    /// The admission has not been consumed yet.
    Pending {
        /// The one-time secret held only during delivery.
        psk: BootstrapPsk,
        /// The deadline unix timestamp (milliseconds) after which the
        /// admission refuses consumption.
        expires_at_unix_ms: u64,
    },
    /// The admission has been consumed or exhausted by a failed attempt.
    Consumed,
    /// The admission refused consumption because its deadline elapsed.
    Expired,
}

impl BootstrapAdmission {
    /// Create an admission record.
    pub fn new(psk: BootstrapPsk, expires_at_unix_ms: u64) -> Self {
        Self::Pending {
            psk,
            expires_at_unix_ms,
        }
    }

    /// Consume the PSK when the presented bytes match and the deadline is
    /// valid. The admission becomes `Consumed` (or `Expired` when the
    /// deadline elapsed) whether or not the PSK matches, so each admission
    /// is single-use.
    ///
    /// # Errors
    ///
    /// Returns [`AzureVmError::BootstrapPskExpired`] when the deadline
    /// elapsed, [`AzureVmError::BootstrapPskReplayed`] when the admission
    /// was already consumed, and
    /// [`AzureVmError::BootstrapEnrollmentFailed`] when the presented PSK
    /// does not match.
    pub fn consume(
        &mut self,
        presented: &[u8],
        now_unix_ms: u64,
    ) -> Result<Zeroizing<Vec<u8>>, AzureVmError> {
        match std::mem::replace(self, Self::Consumed) {
            Self::Pending {
                psk,
                expires_at_unix_ms,
            } => {
                if now_unix_ms >= expires_at_unix_ms {
                    tracing::warn!(
                        provider = "runtime-azure-virtual-machine",
                        "bootstrap PSK admission refused: admission expired"
                    );
                    *self = Self::Expired;
                    return Err(AzureVmError::BootstrapPskExpired);
                }
                if !psk.matches(presented) {
                    tracing::warn!(
                        provider = "runtime-azure-virtual-machine",
                        "bootstrap handshake failed: presented PSK does not match admission"
                    );
                    return Err(AzureVmError::BootstrapEnrollmentFailed);
                }
                Ok(psk.consume())
            }
            Self::Consumed => {
                tracing::warn!(
                    provider = "runtime-azure-virtual-machine",
                    "bootstrap PSK admission refused: PSK replayed"
                );
                Err(AzureVmError::BootstrapPskReplayed)
            }
            Self::Expired => {
                tracing::warn!(
                    provider = "runtime-azure-virtual-machine",
                    "bootstrap PSK admission refused: admission expired"
                );
                Err(AzureVmError::BootstrapPskExpired)
            }
        }
    }

    /// Return the current admission state.
    pub const fn state(&self) -> BootstrapAdmissionState {
        match self {
            Self::Pending { .. } => BootstrapAdmissionState::Pending,
            Self::Consumed => BootstrapAdmissionState::Consumed,
            Self::Expired => BootstrapAdmissionState::Expired,
        }
    }
}

/// Bootstrap service session state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum BootstrapServiceState {
    /// Waiting for one IKpsk2 enrollment.
    #[default]
    Waiting,
    /// Enrollment completed and KK may be used.
    Enrolled,
    /// The service failed closed.
    Failed,
}

/// Gateway Guest bootstrap service.
#[derive(Default)]
pub struct BootstrapService {
    state: BootstrapServiceState,
}

impl BootstrapService {
    /// Restore a service state from the sealed controller recovery record.
    pub const fn from_state(state: BootstrapServiceState) -> Self {
        Self { state }
    }

    /// Return the current state.
    pub const fn state(&self) -> BootstrapServiceState {
        self.state
    }

    /// Consume one admission and transition to enrolled.
    pub fn complete_enrollment(
        &mut self,
        admission: &mut BootstrapAdmission,
        presented: &[u8],
        now_unix_ms: u64,
    ) -> Result<(), AzureVmError> {
        if self.state != BootstrapServiceState::Waiting {
            tracing::warn!(
                provider = "runtime-azure-virtual-machine",
                state = ?self.state,
                "bootstrap enrollment refused: service is not waiting for admission"
            );
            self.state = BootstrapServiceState::Failed;
            return Err(AzureVmError::BootstrapPskReplayed);
        }
        match admission.consume(presented, now_unix_ms) {
            Ok(_psk) => {
                self.state = BootstrapServiceState::Enrolled;
                Ok(())
            }
            Err(error) => {
                tracing::warn!(
                    provider = "runtime-azure-virtual-machine",
                    code = error.code(),
                    "bootstrap enrollment failed; service failed closed"
                );
                self.state = BootstrapServiceState::Failed;
                Err(error)
            }
        }
    }
}
