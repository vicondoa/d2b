//! Pure status and health projection for the Secret Service controller.

use d2b_contracts::v3::credential::{
    CredentialInteractionState, CredentialLeaseStatus, CredentialStatus, PlacementBinding,
};
use d2b_credential_service::{CredentialMetadata, CredentialServiceError};

use crate::{LockPolicy, SecretServiceConfig, SecretServiceState};

/// Closed controller health state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretServiceControllerHealth {
    /// The injected port is ready.
    Ready,
    /// The keyring is locked under degraded policy.
    Degraded,
    /// Operations fail closed while the keyring is locked.
    Unavailable,
}

/// Status projection plus closed health.
#[derive(Clone, PartialEq, Eq)]
pub struct SecretServiceStatusProjection {
    /// Common Credential status.
    pub status: CredentialStatus,
    /// Closed controller health.
    pub health: SecretServiceControllerHealth,
}

impl core::fmt::Debug for SecretServiceStatusProjection {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("SecretServiceStatusProjection(<redacted>)")
    }
}

/// Stateless status-first controller projection.
#[derive(Debug, Clone)]
pub struct SecretServiceController {
    config: SecretServiceConfig,
}

impl SecretServiceController {
    /// Construct the projection controller.
    pub const fn new(config: SecretServiceConfig) -> Self {
        Self { config }
    }

    /// Project current lease metadata and port state without credential bytes.
    pub fn reconcile(
        &self,
        state: SecretServiceState,
        metadata: Option<&CredentialMetadata>,
    ) -> Result<SecretServiceStatusProjection, CredentialServiceError> {
        let health = match (state, self.config.lock_policy()) {
            (SecretServiceState::Unlocked, _) => SecretServiceControllerHealth::Ready,
            (SecretServiceState::Locked, LockPolicy::FailClosed) => {
                SecretServiceControllerHealth::Unavailable
            }
            (SecretServiceState::Locked, LockPolicy::FailDegraded) => {
                SecretServiceControllerHealth::Degraded
            }
        };
        let lease = metadata
            .map(|metadata| {
                CredentialLeaseStatus::new(
                    metadata.lease_handle.clone(),
                    metadata.state,
                    metadata.rotation_generation,
                    metadata.source_version.clone(),
                    metadata.expires_at_unix_ms,
                    1,
                    None,
                    None,
                    PlacementBinding::UserAgent,
                )
            })
            .transpose()
            .map_err(|_| {
                super::SecretServiceCredentialProvider::map_port_error(
                    super::SecretServicePortError::CompletionUnknown,
                )
            })?;
        let status =
            CredentialStatus::new(CredentialInteractionState::NotRequired, None, None, lease)
                .map_err(|_| {
                    super::SecretServiceCredentialProvider::map_port_error(
                        super::SecretServicePortError::CompletionUnknown,
                    )
                })?;
        Ok(SecretServiceStatusProjection { status, health })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_policy_drives_closed_controller_health() {
        let closed = SecretServiceController::new(
            SecretServiceConfig::new("login", 64, LockPolicy::FailClosed).unwrap(),
        );
        let degraded = SecretServiceController::new(
            SecretServiceConfig::new("login", 64, LockPolicy::FailDegraded).unwrap(),
        );
        assert_eq!(
            closed
                .reconcile(SecretServiceState::Locked, None)
                .unwrap()
                .health,
            SecretServiceControllerHealth::Unavailable
        );
        assert_eq!(
            degraded
                .reconcile(SecretServiceState::Locked, None)
                .unwrap()
                .health,
            SecretServiceControllerHealth::Degraded
        );
        assert_eq!(
            closed
                .reconcile(SecretServiceState::Unlocked, None)
                .unwrap()
                .health,
            SecretServiceControllerHealth::Ready
        );
    }
}
