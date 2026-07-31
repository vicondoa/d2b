//! Credential service dispatch for the injected managed identity client.

use d2b_contracts::v3::credential::CredentialLeaseState;
use d2b_credential_service::{
    CredentialAuthorization, CredentialMethod, CredentialOutcomeCode, CredentialProvider,
    CredentialRequest, CredentialResponse, CredentialServiceError, CredentialServiceErrorCode,
    DeliveryResponse, MetadataResponse,
};

use crate::{
    LeaseRecord, ManagedIdentityCredentialProvider, ManagedIdentityLeaseRef,
    ManagedIdentityLeaseRequest,
};

impl CredentialProvider for ManagedIdentityCredentialProvider {
    fn dispatch(
        &self,
        method: CredentialMethod,
        request: &CredentialRequest,
        authorization: &CredentialAuthorization,
    ) -> Result<CredentialResponse, CredentialServiceError> {
        match method {
            CredentialMethod::AcquireToken => self.acquire(request, authorization),
            CredentialMethod::RefreshToken => self.refresh(request, authorization),
            CredentialMethod::RevokeToken => self.revoke(request),
            CredentialMethod::InspectMetadata => self.inspect(request),
            CredentialMethod::SignChallenge => Err(CredentialServiceError::new(
                CredentialServiceErrorCode::Malformed,
            )),
        }
    }
}

impl ManagedIdentityCredentialProvider {
    fn acquire(
        &self,
        request: &CredentialRequest,
        authorization: &CredentialAuthorization,
    ) -> Result<CredentialResponse, CredentialServiceError> {
        let delivery = authorization
            .delivery_session_params()
            .cloned()
            .ok_or_else(invariant)?;
        let key = request.credential_ref().to_canonical_string();
        {
            let leases = self.leases.lock().map_err(|_| invariant())?;
            if let Some(existing) = leases.get(&key) {
                if existing.idempotency_key == request.idempotency_key() {
                    return Ok(CredentialResponse::AcquireToken(DeliveryResponse {
                        metadata: existing.metadata.clone(),
                        delivery_session_params: delivery,
                    }));
                }
            }
            if leases.len() >= self.config.max_leases() as usize {
                return Err(CredentialServiceError::new(
                    CredentialServiceErrorCode::ProviderUnavailable,
                ));
            }
        }
        let client_request = ManagedIdentityLeaseRequest {
            credential_ref: request.credential_ref().clone(),
            operation_id: request.operation_id().to_owned(),
            idempotency_key: request.idempotency_key().to_owned(),
            requested_expiry_unix_ms: request.requested_expiry_unix_ms(),
        };
        let grant = Self::poll_client(self.client.issue_lease(&client_request))?;
        let metadata = Self::grant_metadata(grant, request.requested_expiry_unix_ms())?;
        self.leases.lock().map_err(|_| invariant())?.insert(
            key,
            LeaseRecord {
                idempotency_key: request.idempotency_key().to_owned(),
                metadata: metadata.clone(),
            },
        );
        Ok(CredentialResponse::AcquireToken(DeliveryResponse {
            metadata,
            delivery_session_params: delivery,
        }))
    }

    fn refresh(
        &self,
        request: &CredentialRequest,
        authorization: &CredentialAuthorization,
    ) -> Result<CredentialResponse, CredentialServiceError> {
        let delivery = authorization
            .delivery_session_params()
            .cloned()
            .ok_or_else(invariant)?;
        let key = request.credential_ref().to_canonical_string();
        let record = self
            .leases
            .lock()
            .map_err(|_| invariant())?
            .get(&key)
            .cloned()
            .ok_or_else(expired)?;
        let lease = ManagedIdentityLeaseRef {
            credential_ref: request.credential_ref().clone(),
            metadata: record.metadata,
        };
        let inspection = Self::poll_client(self.client.inspect_lease(&lease))?;
        if inspection.state != CredentialLeaseState::Active
            || inspection.rotation_generation != lease.metadata.rotation_generation
        {
            return Err(invariant());
        }
        let grant = Self::poll_client(self.client.refresh_lease(&lease))?;
        let metadata = Self::grant_metadata(grant, request.requested_expiry_unix_ms())?;
        self.leases.lock().map_err(|_| invariant())?.insert(
            key,
            LeaseRecord {
                idempotency_key: request.idempotency_key().to_owned(),
                metadata: metadata.clone(),
            },
        );
        Ok(CredentialResponse::RefreshToken(DeliveryResponse {
            metadata,
            delivery_session_params: delivery,
        }))
    }

    fn revoke(
        &self,
        request: &CredentialRequest,
    ) -> Result<CredentialResponse, CredentialServiceError> {
        let key = request.credential_ref().to_canonical_string();
        let mut leases = self.leases.lock().map_err(|_| invariant())?;
        let record = leases.get_mut(&key).ok_or_else(expired)?;
        let outcome = if record.metadata.state == CredentialLeaseState::Revoked {
            CredentialOutcomeCode::AlreadyRevoked
        } else {
            let lease = ManagedIdentityLeaseRef {
                credential_ref: request.credential_ref().clone(),
                metadata: record.metadata.clone(),
            };
            let revocation = Self::poll_client(self.client.revoke_lease(&lease))?;
            record.metadata.state = CredentialLeaseState::Revoked;
            match revocation {
                crate::ManagedIdentityLeaseRevocation::Revoked => CredentialOutcomeCode::Revoked,
                crate::ManagedIdentityLeaseRevocation::AlreadyRevoked => {
                    CredentialOutcomeCode::AlreadyRevoked
                }
            }
        };
        record.metadata.outcome = outcome;
        Ok(CredentialResponse::RevokeToken(MetadataResponse {
            metadata: record.metadata.clone(),
        }))
    }

    fn inspect(
        &self,
        request: &CredentialRequest,
    ) -> Result<CredentialResponse, CredentialServiceError> {
        let record = self
            .leases
            .lock()
            .map_err(|_| invariant())?
            .get(&request.credential_ref().to_canonical_string())
            .cloned()
            .ok_or_else(expired)?;
        let lease = ManagedIdentityLeaseRef {
            credential_ref: request.credential_ref().clone(),
            metadata: record.metadata,
        };
        let inspection = Self::poll_client(self.client.inspect_lease(&lease))?;
        let mut metadata = lease.metadata;
        metadata.state = inspection.state;
        metadata.source_version = inspection.source_version;
        metadata.rotation_generation = inspection.rotation_generation;
        metadata.expires_at_unix_ms = inspection.expires_at_unix_ms;
        Ok(CredentialResponse::InspectMetadata(MetadataResponse {
            metadata,
        }))
    }
}

fn invariant() -> CredentialServiceError {
    CredentialServiceError::new(CredentialServiceErrorCode::InvariantFailure)
}

fn expired() -> CredentialServiceError {
    CredentialServiceError::new(CredentialServiceErrorCode::LeaseExpired)
}
