//! Credential service dispatch for the injected managed identity client.

use d2b_contracts::v3::credential::{
    CredentialAuthorization, CredentialLeaseState, CredentialMethod, CredentialOutcomeCode,
    CredentialProvider, CredentialRequest, CredentialResponse, CredentialServiceError,
    CredentialServiceErrorCode, DeliveryResponse, MetadataResponse,
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
            CredentialMethod::RevokeToken => self.revoke(request, authorization),
            CredentialMethod::InspectMetadata => self.inspect(request, authorization),
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
            .ok_or_else(operation_denied)?;
        let session = self.validate_authenticated_session(
            CredentialMethod::AcquireToken,
            request,
            authorization,
        )?;
        let requested_expiry = ManagedIdentityCredentialProvider::bounded_expiry(
            request.requested_expiry_unix_ms(),
            session.expires_at_unix_ms(),
            delivery.expiry_unix_ms(),
        )?;
        let deadline = Self::operation_deadline(request.deadline_unix_ms())?;
        let _mutation = self.mutation_guard()?;
        let key = request.credential_ref().to_canonical_string();
        let now = Self::now_unix_ms();
        let rotation_generation = {
            let mut leases = self.leases.lock().map_err(|_| invariant())?;
            Self::mark_expired_locked(&mut leases, now);
            let records = leases.get(&key);
            if let Some(records) = records {
                if records.iter().any(|record| {
                    Self::same_owner(
                        &record.authenticated_subject,
                        session.authenticated_subject(),
                    )
                }) {
                    if let Some(existing) = records.iter().find(|record| {
                        record.metadata.state == CredentialLeaseState::Active
                            && record.idempotency_key == request.idempotency_key()
                            && Self::same_owner(
                                &record.authenticated_subject,
                                session.authenticated_subject(),
                            )
                    }) {
                        return Ok(CredentialResponse::AcquireToken(DeliveryResponse {
                            metadata: existing.metadata.clone(),
                            delivery_session_params: delivery,
                        }));
                    }
                } else if !records.is_empty() {
                    return Err(operation_denied());
                }
            }
            if Self::active_lease_count(&leases) >= self.config.max_leases() as usize {
                return Err(CredentialServiceError::new(
                    CredentialServiceErrorCode::ProviderUnavailable,
                ));
            }
            let prior_generation = records
                .into_iter()
                .flatten()
                .filter(|record| {
                    Self::same_owner(
                        &record.authenticated_subject,
                        session.authenticated_subject(),
                    )
                })
                .map(|record| record.metadata.rotation_generation)
                .max()
                .unwrap_or(0);
            prior_generation.checked_add(1).ok_or_else(|| invariant())?
        };
        let client_request = ManagedIdentityLeaseRequest {
            credential_ref: request.credential_ref().clone(),
            operation_id: request.operation_id().to_owned(),
            idempotency_key: request.idempotency_key().to_owned(),
            requested_expiry_unix_ms: requested_expiry,
            rotation_generation,
        };
        let grant = Self::poll_client(self.client.issue_lease(&client_request), deadline)?;
        let metadata = Self::grant_metadata(grant, requested_expiry, rotation_generation)?;
        self.leases
            .lock()
            .map_err(|_| invariant())?
            .entry(key)
            .or_default()
            .push(LeaseRecord {
                idempotency_key: request.idempotency_key().to_owned(),
                metadata: metadata.clone(),
                authenticated_subject: session.authenticated_subject().clone(),
                session_expires_at_unix_ms: session.expires_at_unix_ms(),
            });
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
            .ok_or_else(operation_denied)?;
        let session = self.validate_authenticated_session(
            CredentialMethod::RefreshToken,
            request,
            authorization,
        )?;
        let requested_expiry = ManagedIdentityCredentialProvider::bounded_expiry(
            request.requested_expiry_unix_ms(),
            session.expires_at_unix_ms(),
            delivery.expiry_unix_ms(),
        )?;
        let deadline = Self::operation_deadline(request.deadline_unix_ms())?;
        let _mutation = self.mutation_guard()?;
        let key = request.credential_ref().to_canonical_string();
        let now = Self::now_unix_ms();
        let record = {
            let mut leases = self.leases.lock().map_err(|_| invariant())?;
            Self::mark_expired_locked(&mut leases, now);
            let records = leases.get(&key).ok_or_else(expired)?;
            let record = records
                .iter()
                .find(|record| {
                    Self::same_owner(
                        &record.authenticated_subject,
                        session.authenticated_subject(),
                    )
                })
                .ok_or_else(operation_denied)?;
            if !Self::same_session(
                &record.authenticated_subject,
                session.authenticated_subject(),
            ) {
                return Err(operation_denied());
            }
            ensure_active(record)?;
            record.clone()
        };
        let lease = ManagedIdentityLeaseRef {
            credential_ref: request.credential_ref().clone(),
            metadata: record.metadata.clone(),
        };
        let inspection = Self::poll_client(self.client.inspect_lease(&lease), deadline)?;
        if inspection.state != CredentialLeaseState::Active {
            self.update_state(
                &key,
                &record,
                inspection.state,
                CredentialOutcomeCode::Success,
            )?;
            return Err(error_for_state(inspection.state));
        }
        if Self::is_expired(inspection.expires_at_unix_ms, Self::now_unix_ms()) {
            self.update_state(
                &key,
                &record,
                CredentialLeaseState::Expired,
                CredentialOutcomeCode::Success,
            )?;
            return Err(expired());
        }
        if inspection.rotation_generation < record.metadata.rotation_generation {
            return Err(invariant());
        }
        let grant = Self::poll_client(self.client.refresh_lease(&lease), deadline)?;
        let metadata =
            Self::grant_metadata(grant, requested_expiry, record.metadata.rotation_generation)?;
        self.replace_record(&key, &record, request.idempotency_key(), metadata.clone())?;
        Ok(CredentialResponse::RefreshToken(DeliveryResponse {
            metadata,
            delivery_session_params: delivery,
        }))
    }

    fn revoke(
        &self,
        request: &CredentialRequest,
        authorization: &CredentialAuthorization,
    ) -> Result<CredentialResponse, CredentialServiceError> {
        let session = self.validate_authenticated_session(
            CredentialMethod::RevokeToken,
            request,
            authorization,
        )?;
        let deadline = Self::operation_deadline(request.deadline_unix_ms())?;
        let _mutation = self.mutation_guard()?;
        let key = request.credential_ref().to_canonical_string();
        let now = Self::now_unix_ms();
        let record = {
            let mut leases = self.leases.lock().map_err(|_| invariant())?;
            Self::mark_expired_locked(&mut leases, now);
            let records = leases.get(&key).ok_or_else(expired)?;
            let record = records
                .iter()
                .find(|record| {
                    Self::same_owner(
                        &record.authenticated_subject,
                        session.authenticated_subject(),
                    )
                })
                .ok_or_else(operation_denied)?;
            if !Self::same_session(
                &record.authenticated_subject,
                session.authenticated_subject(),
            ) {
                return Err(operation_denied());
            }
            record.clone()
        };
        if record.metadata.state == CredentialLeaseState::Revoked {
            return Ok(CredentialResponse::RevokeToken(MetadataResponse {
                metadata: record.metadata,
            }));
        }
        ensure_active(&record)?;
        let lease = ManagedIdentityLeaseRef {
            credential_ref: request.credential_ref().clone(),
            metadata: record.metadata.clone(),
        };
        let revocation = Self::poll_client(self.client.revoke_lease(&lease), deadline)?;
        let outcome = match revocation {
            crate::ManagedIdentityLeaseRevocation::Revoked => CredentialOutcomeCode::Revoked,
            crate::ManagedIdentityLeaseRevocation::AlreadyRevoked => {
                CredentialOutcomeCode::AlreadyRevoked
            }
        };
        let mut metadata = record.metadata.clone();
        metadata.state = CredentialLeaseState::Revoked;
        metadata.outcome = outcome;
        self.replace_record(&key, &record, request.idempotency_key(), metadata.clone())?;
        Ok(CredentialResponse::RevokeToken(MetadataResponse {
            metadata,
        }))
    }

    fn inspect(
        &self,
        request: &CredentialRequest,
        authorization: &CredentialAuthorization,
    ) -> Result<CredentialResponse, CredentialServiceError> {
        let session = self.validate_authenticated_session(
            CredentialMethod::InspectMetadata,
            request,
            authorization,
        )?;
        let deadline = Self::operation_deadline(request.deadline_unix_ms())?;
        let _mutation = self.mutation_guard()?;
        let key = request.credential_ref().to_canonical_string();
        let now = Self::now_unix_ms();
        let record = {
            let mut leases = self.leases.lock().map_err(|_| invariant())?;
            Self::mark_expired_locked(&mut leases, now);
            let records = leases.get(&key).ok_or_else(expired)?;
            let record = records
                .iter()
                .find(|record| {
                    Self::same_owner(
                        &record.authenticated_subject,
                        session.authenticated_subject(),
                    )
                })
                .ok_or_else(operation_denied)?;
            if !Self::same_session(
                &record.authenticated_subject,
                session.authenticated_subject(),
            ) {
                return Err(operation_denied());
            }
            ensure_active_or_observable(record)?;
            record.clone()
        };
        let lease = ManagedIdentityLeaseRef {
            credential_ref: request.credential_ref().clone(),
            metadata: record.metadata.clone(),
        };
        let inspection = Self::poll_client(self.client.inspect_lease(&lease), deadline)?;
        let mut metadata = record.metadata.clone();
        metadata.state = inspection.state;
        metadata.source_version = inspection.source_version;
        metadata.rotation_generation = inspection
            .rotation_generation
            .max(metadata.rotation_generation);
        metadata.expires_at_unix_ms = inspection.expires_at_unix_ms;
        if Self::is_expired(metadata.expires_at_unix_ms, Self::now_unix_ms())
            && metadata.state == CredentialLeaseState::Active
        {
            metadata.state = CredentialLeaseState::Expired;
        }
        self.replace_record(&key, &record, request.idempotency_key(), metadata.clone())?;
        if metadata.state == CredentialLeaseState::Expired {
            return Err(expired());
        }
        if metadata.state == CredentialLeaseState::Revoked {
            return Err(error_for_state(metadata.state));
        }
        Ok(CredentialResponse::InspectMetadata(MetadataResponse {
            metadata,
        }))
    }

    fn update_state(
        &self,
        key: &str,
        record: &LeaseRecord,
        state: CredentialLeaseState,
        outcome: CredentialOutcomeCode,
    ) -> Result<(), CredentialServiceError> {
        let mut metadata = record.metadata.clone();
        metadata.state = state;
        metadata.outcome = outcome;
        self.replace_record(key, record, &record.idempotency_key, metadata)
    }

    fn replace_record(
        &self,
        key: &str,
        old: &LeaseRecord,
        idempotency_key: &str,
        metadata: d2b_contracts::v3::credential::CredentialMetadata,
    ) -> Result<(), CredentialServiceError> {
        let mut leases = self.leases.lock().map_err(|_| invariant())?;
        let records = leases.get_mut(key).ok_or_else(invariant)?;
        let record = records
            .iter_mut()
            .find(|record| {
                record.metadata == old.metadata
                    && record.authenticated_subject == old.authenticated_subject
            })
            .ok_or_else(invariant)?;
        record.idempotency_key = idempotency_key.to_owned();
        record.metadata = metadata;
        Ok(())
    }
}

fn ensure_active(record: &LeaseRecord) -> Result<(), CredentialServiceError> {
    match record.metadata.state {
        CredentialLeaseState::Active => Ok(()),
        state => Err(error_for_state(state)),
    }
}

fn ensure_active_or_observable(record: &LeaseRecord) -> Result<(), CredentialServiceError> {
    match record.metadata.state {
        CredentialLeaseState::Active => Ok(()),
        state => Err(error_for_state(state)),
    }
}

fn error_for_state(state: CredentialLeaseState) -> CredentialServiceError {
    match state {
        CredentialLeaseState::Expired => expired(),
        CredentialLeaseState::Revoked => {
            CredentialServiceError::new(CredentialServiceErrorCode::LeaseRevoked)
        }
        CredentialLeaseState::Active | CredentialLeaseState::Unknown => invariant(),
    }
}

fn invariant() -> CredentialServiceError {
    CredentialServiceError::new(CredentialServiceErrorCode::InvariantFailure)
}

fn operation_denied() -> CredentialServiceError {
    CredentialServiceError::new(CredentialServiceErrorCode::OperationDenied)
}

fn expired() -> CredentialServiceError {
    CredentialServiceError::new(CredentialServiceErrorCode::LeaseExpired)
}
