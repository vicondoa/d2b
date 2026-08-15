//! Bootstrap service boundary.

use crate::{bootstrap::BootstrapAdmission, error::AzureVmError};
use serde::{Deserialize, Serialize};

/// Bootstrap service session state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BootstrapServiceState {
    /// Waiting for one IKpsk2 enrollment.
    Waiting,
    /// Enrollment completed and KK may be used.
    Enrolled,
    /// The service failed closed.
    Failed,
}

/// Gateway Guest bootstrap service.
pub struct BootstrapService {
    state: BootstrapServiceState,
}

impl Default for BootstrapService {
    fn default() -> Self {
        Self {
            state: BootstrapServiceState::Waiting,
        }
    }
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
            self.state = BootstrapServiceState::Failed;
            return Err(AzureVmError::BootstrapPskReplayed);
        }
        match admission.consume(presented, now_unix_ms) {
            Ok(_psk) => {
                self.state = BootstrapServiceState::Enrolled;
                Ok(())
            }
            Err(error) => {
                self.state = BootstrapServiceState::Failed;
                Err(error)
            }
        }
    }
}
