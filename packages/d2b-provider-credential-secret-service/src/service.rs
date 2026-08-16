//! Credential service dispatch for Secret Service.

use d2b_contracts::v3::credential::{
    CredentialAuthorization, CredentialLeaseState, CredentialMethod, CredentialOutcomeCode,
    CredentialProvider, CredentialRequest, CredentialResponse, CredentialServiceError,
    CredentialServiceErrorCode, DeliveryResponse, MetadataResponse,
};

use crate::{
    LeaseRecord, SecretServiceCredentialProvider, SecretServiceLeaseRef, SecretServiceLeaseRequest,
    SessionKey,
};

impl CredentialProvider for SecretServiceCredentialProvider {
    fn dispatch(
        &self,
        method: CredentialMethod,
        request: &CredentialRequest,
        authorization: &CredentialAuthorization,
    ) -> Result<CredentialResponse, CredentialServiceError> {
        let _lifecycle = self.mutation_guard()?;
        let session_key = self.authorize_session_locked(authorization)?;
        match method {
            CredentialMethod::AcquireToken => self.acquire(request, authorization, session_key),
            CredentialMethod::RefreshToken => self.refresh(request, authorization, session_key),
            CredentialMethod::RevokeToken => self.revoke(request, session_key),
            CredentialMethod::InspectMetadata => self.inspect(request, session_key),
            CredentialMethod::SignChallenge => Err(CredentialServiceError::new(
                CredentialServiceErrorCode::Malformed,
            )),
        }
    }
}

impl SecretServiceCredentialProvider {
    fn acquire(
        &self,
        request: &CredentialRequest,
        authorization: &CredentialAuthorization,
        session_key: SessionKey,
    ) -> Result<CredentialResponse, CredentialServiceError> {
        let delivery = authorization
            .delivery_session_params()
            .cloned()
            .ok_or_else(invariant)?;
        let deadline = Self::operation_deadline(request.deadline_unix_ms())?;
        let key = request.credential_ref().to_canonical_string();
        let lease_key = (session_key, key.clone());
        {
            let mut leases = self.leases.lock().map_err(|_| invariant())?;
            leases.retain(|_, record| record.metadata.state == CredentialLeaseState::Active);
            if let Some(existing) = leases.get(&lease_key)
                && existing.idempotency_key == request.idempotency_key()
            {
                return Ok(CredentialResponse::AcquireToken(DeliveryResponse {
                    metadata: existing.metadata.clone(),
                    delivery_session_params: delivery,
                }));
            }
            if leases.len() >= self.config.max_leases() as usize {
                return Err(CredentialServiceError::new(
                    CredentialServiceErrorCode::ProviderUnavailable,
                ));
            }
        }
        let port_request = SecretServiceLeaseRequest {
            credential_ref: request.credential_ref().clone(),
            operation_id: request.operation_id().to_owned(),
            idempotency_key: request.idempotency_key().to_owned(),
            requested_expiry_unix_ms: request.requested_expiry_unix_ms(),
        };
        let grant = Self::poll_port(self.port.issue_lease(&port_request), deadline)?;
        let metadata = Self::grant_metadata(grant, request.requested_expiry_unix_ms())?;
        self.leases.lock().map_err(|_| invariant())?.insert(
            lease_key,
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
        session_key: SessionKey,
    ) -> Result<CredentialResponse, CredentialServiceError> {
        let delivery = authorization
            .delivery_session_params()
            .cloned()
            .ok_or_else(invariant)?;
        let deadline = Self::operation_deadline(request.deadline_unix_ms())?;
        let key = request.credential_ref().to_canonical_string();
        let lease_key = (session_key, key);
        let current = self
            .leases
            .lock()
            .map_err(|_| invariant())?
            .get(&lease_key)
            .cloned()
            .ok_or_else(expired)?;
        if current.metadata.state != CredentialLeaseState::Active {
            return Err(expired());
        }
        let lease = SecretServiceLeaseRef {
            credential_ref: request.credential_ref().clone(),
            metadata: current.metadata,
        };
        let inspected = Self::poll_port(self.port.inspect_lease(&lease), deadline)?;
        if inspected.state != CredentialLeaseState::Active
            || inspected.rotation_generation != lease.metadata.rotation_generation
        {
            return Err(invariant());
        }
        let grant = Self::poll_port(self.port.refresh_lease(&lease), deadline)?;
        let metadata = Self::grant_metadata(grant, request.requested_expiry_unix_ms())?;
        self.leases.lock().map_err(|_| invariant())?.insert(
            lease_key,
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
        session_key: SessionKey,
    ) -> Result<CredentialResponse, CredentialServiceError> {
        let deadline = Self::operation_deadline(request.deadline_unix_ms())?;
        let key = request.credential_ref().to_canonical_string();
        let lease_key = (session_key, key);
        let mut leases = self.leases.lock().map_err(|_| invariant())?;
        let record = leases.get_mut(&lease_key).ok_or_else(expired)?;
        let outcome = if record.metadata.state == CredentialLeaseState::Revoked {
            CredentialOutcomeCode::AlreadyRevoked
        } else {
            let lease = SecretServiceLeaseRef {
                credential_ref: request.credential_ref().clone(),
                metadata: record.metadata.clone(),
            };
            let result = Self::poll_port(self.port.revoke_lease(&lease), deadline)?;
            record.metadata.state = CredentialLeaseState::Revoked;
            match result {
                crate::SecretServiceLeaseRevocation::Revoked => CredentialOutcomeCode::Revoked,
                crate::SecretServiceLeaseRevocation::AlreadyRevoked => {
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
        session_key: SessionKey,
    ) -> Result<CredentialResponse, CredentialServiceError> {
        let deadline = Self::operation_deadline(request.deadline_unix_ms())?;
        let key = request.credential_ref().to_canonical_string();
        let lease_key = (session_key, key);
        let record = self
            .leases
            .lock()
            .map_err(|_| invariant())?
            .get(&lease_key)
            .cloned()
            .ok_or_else(expired)?;
        let lease = SecretServiceLeaseRef {
            credential_ref: request.credential_ref().clone(),
            metadata: record.metadata,
        };
        let inspection = Self::poll_port(self.port.inspect_lease(&lease), deadline)?;
        let mut metadata = lease.metadata;
        metadata.state = inspection.state;
        metadata.source_version = inspection.source_version;
        metadata.rotation_generation = inspection.rotation_generation;
        metadata.expires_at_unix_ms = inspection.expires_at_unix_ms;
        Ok(CredentialResponse::InspectMetadata(MetadataResponse {
            metadata,
        }))
    }

    /// Revoke every active lease owned by one admitted session and release its
    /// capability authority.
    pub fn disconnect(
        &self,
        authorization: &CredentialAuthorization,
    ) -> Result<(), CredentialServiceError> {
        let _mutation = self.blocking_mutation_guard()?;
        let session_key = self.session_capability(authorization)?.session_key();
        if !self
            .sessions
            .lock()
            .map_err(|_| invariant())?
            .contains_key(&session_key)
        {
            self.discard_session_key(session_key)?;
            return Ok(());
        }
        let deadline = Self::operation_deadline(1_000)?;
        self.close_session_locked(session_key, deadline)
    }

    /// Finalize one admitted session using the same revocation semantics as a
    /// transport disconnect, then prevent further capability minting.
    pub fn finalize_session(
        &self,
        authorization: &CredentialAuthorization,
    ) -> Result<(), CredentialServiceError> {
        let _mutation = self.blocking_mutation_guard()?;
        self.session_capability(authorization)?;
        self.finalized
            .store(true, std::sync::atomic::Ordering::Release);
        self.close_all_sessions_locked(Self::operation_deadline(1_000)?)?;
        self.authority.clear().map_err(|_| invariant())?;
        Ok(())
    }

    /// Finalize every admitted session and prevent later capability minting.
    pub fn drain(&self) -> Result<(), CredentialServiceError> {
        let _mutation = self.blocking_mutation_guard()?;
        self.finalized
            .store(true, std::sync::atomic::Ordering::Release);
        let keys = self
            .sessions
            .lock()
            .map_err(|_| invariant())?
            .keys()
            .copied()
            .collect::<Vec<_>>();
        let deadline = Self::operation_deadline(1_000)?;
        for session_key in keys {
            self.close_session_locked(session_key, deadline)?;
        }
        self.authority.clear().map_err(|_| invariant())?;
        Ok(())
    }

    fn close_all_sessions_locked(
        &self,
        deadline: std::time::Instant,
    ) -> Result<(), CredentialServiceError> {
        let keys = self
            .sessions
            .lock()
            .map_err(|_| invariant())?
            .keys()
            .copied()
            .collect::<Vec<_>>();
        for session_key in keys {
            self.close_session_locked(session_key, deadline)?;
        }
        Ok(())
    }

    fn close_session_locked(
        &self,
        session_key: SessionKey,
        deadline: std::time::Instant,
    ) -> Result<(), CredentialServiceError> {
        let records = self
            .leases
            .lock()
            .map_err(|_| invariant())?
            .iter()
            .filter(|((key, _), record)| {
                *key == session_key && record.metadata.state == CredentialLeaseState::Active
            })
            .map(|((_, credential), record)| (credential.clone(), record.clone()))
            .collect::<Vec<_>>();

        for (credential, record) in records {
            let lease = SecretServiceLeaseRef {
                credential_ref: d2b_contracts::v3::ResourceRef::parse(&credential)
                    .map_err(|_| invariant())?,
                metadata: record.metadata,
            };
            Self::poll_port(self.port.revoke_lease(&lease), deadline)?;
        }

        self.leases
            .lock()
            .map_err(|_| invariant())?
            .retain(|(key, _), _| *key != session_key);
        self.release_session_key(session_key)?;
        self.sessions
            .lock()
            .map_err(|_| invariant())?
            .remove(&session_key);
        Ok(())
    }
}

fn invariant() -> CredentialServiceError {
    CredentialServiceError::new(CredentialServiceErrorCode::InvariantFailure)
}

fn expired() -> CredentialServiceError {
    CredentialServiceError::new(CredentialServiceErrorCode::LeaseExpired)
}
