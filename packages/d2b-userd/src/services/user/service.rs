use std::{fmt, sync::Arc};

use d2b_contracts::{
    v2_identity::ProviderId,
    v2_services::{StrictWireMessage, admit_metadata, common, method_spec},
};

use super::{
    ENDPOINT_ROLE, SERVICE_PACKAGE,
    export::{ExportHandle, ExportInspection, ScopedExportManager},
    identity::{
        AuthenticatedUser, OwnerBinding, SecretMetricSink, SecretOperation, UserSecretError,
        record_result,
    },
    secret_service::{
        OwnedSecretMetadata, OwnedSecretSelector, SecretStore, SecretStoreError, SystemUserdClock,
        UserdClock,
    },
};

const SERVICE_NAME: &str = "UserService";

pub struct AdmittedUserRequest<'a> {
    authenticated_user: &'a AuthenticatedUser,
    request: &'a common::ServiceRequest,
    method: UserSecretMethod,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UserSecretMethod {
    DeleteCredential,
    RevokeExport,
    Inspect,
}

impl<'a> AdmittedUserRequest<'a> {
    pub fn authenticated_user(&self) -> &'a AuthenticatedUser {
        self.authenticated_user
    }

    pub fn request(&self) -> &'a common::ServiceRequest {
        self.request
    }
}

impl fmt::Debug for AdmittedUserRequest<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AdmittedUserRequest")
            .field("authenticated_user", &"<redacted>")
            .field("request", &"<redacted>")
            .finish()
    }
}

pub struct UserSecretService {
    owner: OwnerBinding,
    store: Arc<dyn SecretStore>,
    exports: Arc<ScopedExportManager>,
    clock: Arc<dyn UserdClock>,
    metrics: Arc<dyn SecretMetricSink>,
}

impl fmt::Debug for UserSecretService {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UserSecretService")
            .field("service_package", &SERVICE_PACKAGE)
            .field("endpoint_role", &ENDPOINT_ROLE)
            .field("owner", &"<redacted>")
            .finish_non_exhaustive()
    }
}

impl UserSecretService {
    pub fn new(
        owner: OwnerBinding,
        store: Arc<dyn SecretStore>,
        exports: Arc<ScopedExportManager>,
        metrics: Arc<dyn SecretMetricSink>,
    ) -> Result<Self, UserSecretError> {
        Self::new_with_clock(owner, store, exports, Arc::new(SystemUserdClock), metrics)
    }

    pub fn new_with_clock(
        owner: OwnerBinding,
        store: Arc<dyn SecretStore>,
        exports: Arc<ScopedExportManager>,
        clock: Arc<dyn UserdClock>,
        metrics: Arc<dyn SecretMetricSink>,
    ) -> Result<Self, UserSecretError> {
        if exports.owner() != &owner {
            return Err(UserSecretError::InvariantViolation);
        }
        Ok(Self {
            owner,
            store,
            exports,
            clock,
            metrics,
        })
    }

    pub fn owner(&self) -> &OwnerBinding {
        &self.owner
    }

    pub fn admit<'a>(
        &self,
        method: &str,
        authenticated_user: &'a AuthenticatedUser,
        request: &'a common::ServiceRequest,
    ) -> Result<AdmittedUserRequest<'a>, UserSecretError> {
        let specification = method_spec(SERVICE_PACKAGE, SERVICE_NAME, method)
            .ok_or(UserSecretError::InvalidRequest)?;
        let method = match method {
            "DeleteCredential" => UserSecretMethod::DeleteCredential,
            "RevokeExport" => UserSecretMethod::RevokeExport,
            "Inspect" => UserSecretMethod::Inspect,
            _ => return Err(UserSecretError::InvalidRequest),
        };
        request
            .validate_wire(specification.requires_idempotency)
            .map_err(|_| UserSecretError::InvalidRequest)?;
        self.owner.authorize(authenticated_user)?;

        let metadata = request
            .metadata
            .as_ref()
            .ok_or(UserSecretError::InvalidRequest)?;
        if metadata.session_generation != authenticated_user.session_generation().get() {
            return Err(UserSecretError::Unauthorized);
        }
        admit_metadata(
            metadata,
            specification.requires_idempotency,
            self.clock.now_unix_ms(),
            u64::from(specification.max_lifetime_ms),
            None,
            None,
        )
        .map_err(|_| UserSecretError::DeadlineExpired)?;

        let scope = request
            .scope
            .as_ref()
            .ok_or(UserSecretError::InvalidRequest)?;
        if scope.realm_id != self.owner.realm_id().as_str()
            || !scope.workload_id.is_empty()
            || !scope.provider_id.is_empty()
            || !scope.role_id.is_empty()
        {
            return Err(UserSecretError::Unauthorized);
        }
        if request.resource_id.is_empty()
            || !request.attachment_indexes.is_empty()
            || !request.stream_id.is_empty()
            || request.page_size != 0
            || !request.page_cursor.is_empty()
            || request.desired_state.value() != 0
        {
            return Err(UserSecretError::InvalidRequest);
        }
        if specification.mutating
            && (request.operation_id.is_empty() || request.request_digest.len() != 32)
        {
            return Err(UserSecretError::InvalidRequest);
        }
        Ok(AdmittedUserRequest {
            authenticated_user,
            request,
            method,
        })
    }

    pub async fn status(
        &self,
        authenticated_user: &AuthenticatedUser,
    ) -> Result<d2b_provider_credential_secret_service::SecretServiceState, UserSecretError> {
        let result = async {
            self.owner.authorize(authenticated_user)?;
            self.store.state().await.map_err(Self::map_store)
        }
        .await;
        record_result(&self.metrics, SecretOperation::Status, &result);
        result
    }

    pub async fn inspect_credential(
        &self,
        admitted: &AdmittedUserRequest<'_>,
    ) -> Result<OwnedSecretMetadata, UserSecretError> {
        let result = async {
            self.require_method(admitted, UserSecretMethod::Inspect)?;
            let provider_id = self.provider_id(admitted)?;
            self.store
                .metadata(&OwnedSecretSelector::new(self.owner.clone(), provider_id))
                .await
                .map_err(Self::map_store)
        }
        .await;
        record_result(&self.metrics, SecretOperation::Status, &result);
        result
    }

    pub async fn delete_credential(
        &self,
        admitted: &AdmittedUserRequest<'_>,
    ) -> Result<bool, UserSecretError> {
        let result = async {
            self.require_method(admitted, UserSecretMethod::DeleteCredential)?;
            let provider_id = self.provider_id(admitted)?;
            self.store
                .delete_owned(&OwnedSecretSelector::new(self.owner.clone(), provider_id))
                .await
                .map_err(Self::map_store)
        }
        .await;
        record_result(&self.metrics, SecretOperation::DeleteCredential, &result);
        result
    }

    pub async fn inspect_export(
        &self,
        admitted: &AdmittedUserRequest<'_>,
    ) -> Result<ExportInspection, UserSecretError> {
        self.require_method(admitted, UserSecretMethod::Inspect)?;
        let handle = self.export_handle(admitted)?;
        self.exports
            .inspect(admitted.authenticated_user(), &handle)
            .await
    }

    pub async fn revoke_export(
        &self,
        admitted: &AdmittedUserRequest<'_>,
    ) -> Result<bool, UserSecretError> {
        self.require_method(admitted, UserSecretMethod::RevokeExport)?;
        let handle = self.export_handle(admitted)?;
        self.exports
            .revoke(admitted.authenticated_user(), &handle)
            .await
    }

    fn provider_id(
        &self,
        admitted: &AdmittedUserRequest<'_>,
    ) -> Result<ProviderId, UserSecretError> {
        ProviderId::parse(admitted.request().resource_id.clone())
            .map_err(|_| UserSecretError::InvalidRequest)
    }

    fn export_handle(
        &self,
        admitted: &AdmittedUserRequest<'_>,
    ) -> Result<ExportHandle, UserSecretError> {
        ExportHandle::parse(admitted.request().resource_id.clone())
    }

    fn require_method(
        &self,
        admitted: &AdmittedUserRequest<'_>,
        expected: UserSecretMethod,
    ) -> Result<(), UserSecretError> {
        if admitted.method == expected {
            Ok(())
        } else {
            Err(UserSecretError::InvalidRequest)
        }
    }

    fn map_store(error: SecretStoreError) -> UserSecretError {
        match error {
            SecretStoreError::Locked => UserSecretError::Locked,
            SecretStoreError::NotFound => UserSecretError::NotFound,
            SecretStoreError::Denied => UserSecretError::Unauthorized,
            SecretStoreError::Duplicate | SecretStoreError::InvalidData => {
                UserSecretError::InvariantViolation
            }
            SecretStoreError::Unavailable => UserSecretError::Unavailable,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::services::user::{
        InMemoryExportCommitPort, NoopSecretMetrics, Tpm2SealContext, Tpm2SealError, Tpm2Sealer,
        secret_service::{
            SecretMaterial,
            tests::{FakeClock, FixedEntropy, MemoryStore, owner},
        },
    };
    use async_trait::async_trait;
    use d2b_contracts::v2_services::common::{IdentityScope, RequestMetadata};
    use zeroize::Zeroizing;

    struct FakeSealer;

    #[async_trait]
    impl Tpm2Sealer for FakeSealer {
        async fn seal(
            &self,
            _: &Tpm2SealContext,
            _: SecretMaterial,
        ) -> Result<Zeroizing<Vec<u8>>, Tpm2SealError> {
            Ok(Zeroizing::new(vec![1]))
        }
    }

    fn service(clock: Arc<FakeClock>) -> UserSecretService {
        let owner = owner();
        let store = Arc::new(MemoryStore::default());
        let metrics = Arc::new(NoopSecretMetrics);
        let exports = Arc::new(ScopedExportManager::new_with_runtime(
            owner.clone(),
            store.clone(),
            Arc::new(FakeSealer),
            Arc::new(InMemoryExportCommitPort::default()),
            Arc::new(FixedEntropy::default()),
            clock.clone(),
            metrics.clone(),
        ));
        UserSecretService::new_with_clock(owner, store, exports, clock, metrics).expect("service")
    }

    fn request(service: &UserSecretService, realm: String) -> common::ServiceRequest {
        common::ServiceRequest {
            metadata: Some(RequestMetadata {
                request_id: vec![1; 16],
                correlation_id: "correlation".to_owned(),
                trace_id: Vec::new(),
                idempotency_key: vec![2; 16],
                issued_at_unix_ms: 100,
                expires_at_unix_ms: 1_000,
                session_generation: service.owner().agent_generation().get(),
                special_fields: Default::default(),
            })
            .into(),
            scope: Some(IdentityScope {
                realm_id: realm,
                workload_id: String::new(),
                provider_id: String::new(),
                role_id: String::new(),
                special_fields: Default::default(),
            })
            .into(),
            resource_id: "e11111111111111111111111111111111".to_owned(),
            operation_id: "operation-one".to_owned(),
            request_digest: vec![3; 32],
            stream_id: String::new(),
            attachment_indexes: Vec::new(),
            page_size: 0,
            page_cursor: String::new(),
            desired_state: Default::default(),
            special_fields: Default::default(),
        }
    }

    #[test]
    fn admission_uses_session_uid_and_exact_realm_not_payload_authority() {
        let clock = Arc::new(FakeClock(Mutex::new(200)));
        let service = service(clock);
        let authenticated = AuthenticatedUser::command_client(
            service.owner().uid(),
            service.owner().realm_id().clone(),
            service.owner().agent_generation(),
        );
        let exact = request(&service, service.owner().realm_id().as_str().to_owned());
        assert!(
            service
                .admit("RevokeExport", &authenticated, &exact)
                .is_ok()
        );

        let wrong_uid = AuthenticatedUser::command_client(
            service.owner().uid() + 1,
            service.owner().realm_id().clone(),
            service.owner().agent_generation(),
        );
        assert_eq!(
            service
                .admit("RevokeExport", &wrong_uid, &exact)
                .unwrap_err(),
            UserSecretError::Unauthorized
        );

        let foreign_realm = request(&service, "aaaaaaaaaaaaaaaaaaaa".to_owned());
        assert_eq!(
            service
                .admit("RevokeExport", &authenticated, &foreign_realm)
                .unwrap_err(),
            UserSecretError::Unauthorized
        );
    }
}
