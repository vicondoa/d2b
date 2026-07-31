//! Secret-free managed identity controller projections.

use d2b_contracts::v3::credential::{
    CredentialInteractionState, CredentialLeaseStatus, CredentialStatus,
};
use d2b_credential_service::{
    CredentialMetadata, CredentialServiceError, CredentialServiceErrorCode,
};

use crate::{ManagedIdentityClientState, ManagedIdentityPlacement};

/// Common status plus closed client state.
#[derive(Clone, PartialEq, Eq)]
pub struct ManagedIdentityStatusProjection {
    /// Credential common status.
    pub status: CredentialStatus,
    /// Closed client state.
    pub client_state: ManagedIdentityClientState,
}

impl core::fmt::Debug for ManagedIdentityStatusProjection {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("ManagedIdentityStatusProjection(<redacted>)")
    }
}

/// Stateless status-first controller that holds no IMDS client.
#[derive(Debug, Clone)]
pub struct ManagedIdentityController {
    placement: ManagedIdentityPlacement,
}

impl ManagedIdentityController {
    /// Bind the secret-free controller to machine placement.
    pub const fn new(placement: ManagedIdentityPlacement) -> Self {
        Self { placement }
    }

    /// Project bounded non-secret lease state.
    pub fn reconcile(
        &self,
        client_state: ManagedIdentityClientState,
        metadata: Option<&CredentialMetadata>,
    ) -> Result<ManagedIdentityStatusProjection, CredentialServiceError> {
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
                    self.placement.binding(),
                )
            })
            .transpose()
            .map_err(|_| invariant())?;
        let status =
            CredentialStatus::new(CredentialInteractionState::NotRequired, None, None, lease)
                .map_err(|_| invariant())?;
        Ok(ManagedIdentityStatusProjection {
            status,
            client_state,
        })
    }
}

fn invariant() -> CredentialServiceError {
    CredentialServiceError::new(CredentialServiceErrorCode::InvariantFailure)
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts::v3::ResourceRef;
    use d2b_contracts::v3::credential::PlacementBinding;

    #[test]
    fn ready_and_unavailable_are_closed_status_observations() {
        let controller = ManagedIdentityController::new(
            ManagedIdentityPlacement::new(
                PlacementBinding::HostSystem,
                ResourceRef::parse("Host/azure-vm").unwrap(),
            )
            .unwrap(),
        );
        assert_eq!(
            controller
                .reconcile(ManagedIdentityClientState::Ready, None)
                .unwrap()
                .client_state,
            ManagedIdentityClientState::Ready
        );
        assert_eq!(
            controller
                .reconcile(ManagedIdentityClientState::Unavailable, None)
                .unwrap()
                .client_state,
            ManagedIdentityClientState::Unavailable
        );
    }
}
