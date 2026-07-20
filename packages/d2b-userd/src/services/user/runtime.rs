use std::{
    fmt,
    future::Future,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use async_trait::async_trait;
use d2b_contracts::{
    v2_component_session::{
        AttachmentPolicy, EndpointPolicy, EndpointPurpose, EndpointRole,
        IdentityEvidenceRequirement, LimitProfile, Locality, NoiseProfile, PurposeClass, RequestId,
        ServicePackage, TransportBinding, TransportClass,
    },
    v2_identity::{ProviderId, RealmId, RealmPath, RoleId, RoleKind, WorkloadId, WorkloadName},
    v2_provider::{Generation, MAX_SAFE_JSON_INTEGER},
    v2_services::{
        SERVICE_INVENTORY, common, service_schema_fingerprint,
        user_ttrpc::{UserService, create_user_service},
    },
};
use d2b_session::{
    ComponentSessionDriver, HandshakeCredentials, RequestRegistry, SessionEngine,
    serve_ttrpc_services,
};
use d2b_session_unix::{
    ActivatedSeqpacketListeners, CreditPool, CreditScopeSet, DescriptorPolicyResolver,
    PeerCredentials, PeerIdentityPolicy, SeqpacketSocket, UnixSeqpacketTransport, UnixSessionError,
};
use rustix::process::getuid;
use sha2::{Digest, Sha256};
use tokio::{
    sync::{Semaphore, watch},
    task::JoinSet,
    time::timeout,
};

use crate::services::user::{
    AuthenticatedUser, ExportInspection, ExportState, InMemoryExportCommitPort, NoopSecretMetrics,
    Oo7SecretStore, OsEntropy, OwnedSecretMetadata, OwnerBinding, ScopedExportManager, SecretStore,
    SecretStoreError, SystemdCredsTpm2Sealer, UserSecretError, UserSecretService,
};

const LISTENER_NAME: &str = "user-agent";
const MAX_CONCURRENT_SESSIONS: usize = 32;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);
const SYSTEMD_CREDS: &str = "/run/current-system/sw/bin/systemd-creds";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserdRuntimeError {
    Composition,
    Activation,
    Serve,
}

impl UserdRuntimeError {
    pub const fn exit_code(self) -> i32 {
        match self {
            Self::Composition | Self::Activation => 78,
            Self::Serve => 1,
        }
    }
}

impl fmt::Display for UserdRuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Composition => "user-agent-composition-unavailable",
            Self::Activation => "user-agent-activation-invalid",
            Self::Serve => "user-agent-session-failed",
        })
    }
}

impl std::error::Error for UserdRuntimeError {}

#[async_trait]
pub trait SecretStoreFactory: Send + Sync {
    async fn connect(&self, owner: OwnerBinding) -> Result<Arc<dyn SecretStore>, SecretStoreError>;
}

#[derive(Debug, Default)]
pub struct Oo7SecretStoreFactory;

#[async_trait]
impl SecretStoreFactory for Oo7SecretStoreFactory {
    async fn connect(&self, owner: OwnerBinding) -> Result<Arc<dyn SecretStore>, SecretStoreError> {
        Oo7SecretStore::connect(owner)
            .await
            .map(|store| Arc::new(store) as Arc<dyn SecretStore>)
    }
}

pub struct UserdComposition {
    owner: OwnerBinding,
    service: Arc<UserSecretService>,
}

impl fmt::Debug for UserdComposition {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UserdComposition")
            .field("owner", &"<redacted>")
            .finish_non_exhaustive()
    }
}

impl UserdComposition {
    pub async fn establish(
        factory: &dyn SecretStoreFactory,
        uid: u32,
        generation: Generation,
    ) -> Result<Self, UserdRuntimeError> {
        let owner = owner_binding(uid, generation)?;
        let store = factory
            .connect(owner.clone())
            .await
            .map_err(|_| UserdRuntimeError::Composition)?;
        Self::from_store(owner, store)
    }

    pub fn from_store(
        owner: OwnerBinding,
        store: Arc<dyn SecretStore>,
    ) -> Result<Self, UserdRuntimeError> {
        let metrics = Arc::new(NoopSecretMetrics);
        let sealer = Arc::new(
            SystemdCredsTpm2Sealer::new(SYSTEMD_CREDS)
                .map_err(|_| UserdRuntimeError::Composition)?,
        );
        let exports = Arc::new(ScopedExportManager::new(
            owner.clone(),
            Arc::clone(&store),
            sealer,
            Arc::new(InMemoryExportCommitPort::default()),
            metrics.clone(),
        ));
        let service = Arc::new(
            UserSecretService::new(owner.clone(), store, exports, metrics)
                .map_err(|_| UserdRuntimeError::Composition)?,
        );
        Ok(Self { owner, service })
    }

    pub fn owner(&self) -> &OwnerBinding {
        &self.owner
    }

    pub fn service(&self) -> Arc<UserSecretService> {
        Arc::clone(&self.service)
    }
}

pub struct UserServiceAdapter {
    service: Arc<UserSecretService>,
    authenticated: AuthenticatedUser,
    requests: Mutex<RequestRegistry>,
}

impl fmt::Debug for UserServiceAdapter {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UserServiceAdapter")
            .field("authenticated", &"<redacted>")
            .finish_non_exhaustive()
    }
}

impl UserServiceAdapter {
    pub fn new(
        service: Arc<UserSecretService>,
        authenticated: AuthenticatedUser,
    ) -> Result<Self, UserdRuntimeError> {
        let requests = RequestRegistry::new(service.owner().agent_generation().get())
            .map_err(|_| UserdRuntimeError::Composition)?;
        service
            .owner()
            .authorize(&authenticated)
            .map_err(|_| UserdRuntimeError::Composition)?;
        Ok(Self {
            service,
            authenticated,
            requests: Mutex::new(requests),
        })
    }

    pub async fn status(&self) -> Result<common::ServiceResponse, UserSecretError> {
        let state = self.service.status(&self.authenticated).await?;
        Ok(common::ServiceResponse {
            outcome: common::Outcome::OUTCOME_SUCCEEDED.into(),
            resource_handle: match state {
                d2b_provider_credential_secret_service::SecretServiceState::Locked => {
                    "locked".to_owned()
                }
                d2b_provider_credential_secret_service::SecretServiceState::Unlocked => {
                    "unlocked".to_owned()
                }
            },
            ..Default::default()
        })
    }

    pub async fn inspect(
        &self,
        request: &common::ServiceRequest,
    ) -> Result<common::ServiceResponse, UserSecretError> {
        let admitted = self
            .service
            .admit("Inspect", &self.authenticated, request)?;
        if request.resource_id == "status" {
            return self.status().await;
        }
        if ProviderId::parse(request.resource_id.clone()).is_ok() {
            self.service
                .inspect_credential(&admitted)
                .await
                .map(|metadata| credential_inspection_response(request, &metadata))
        } else {
            self.service
                .inspect_export(&admitted)
                .await
                .map(|inspection| export_inspection_response(request, &inspection))
        }
    }

    pub async fn delete_credential(
        &self,
        request: &common::ServiceRequest,
    ) -> Result<common::ServiceResponse, UserSecretError> {
        let admitted = self
            .service
            .admit("DeleteCredential", &self.authenticated, request)?;
        self.service
            .delete_credential(&admitted)
            .await
            .map(|applied| mutation_response(request, applied))
    }

    pub async fn revoke_export(
        &self,
        request: &common::ServiceRequest,
    ) -> Result<common::ServiceResponse, UserSecretError> {
        let admitted = self
            .service
            .admit("RevokeExport", &self.authenticated, request)?;
        self.service
            .revoke_export(&admitted)
            .await
            .map(|applied| mutation_response(request, applied))
    }

    async fn run_request<F>(
        &self,
        request: &common::ServiceRequest,
        operation: F,
    ) -> ttrpc::Result<common::ServiceResponse>
    where
        F: Future<Output = Result<common::ServiceResponse, UserSecretError>>,
    {
        let request_id = request_id(request).map_err(rpc_error)?;
        let cancellation = {
            let mut requests = self.requests.lock().map_err(|_| internal_rpc_error())?;
            let cancellation = requests
                .register(request_id.clone())
                .map_err(|_| invalid_rpc_error())?;
            requests
                .mark_dispatched(&request_id)
                .map_err(|_| invalid_rpc_error())?;
            cancellation
        };
        let result = tokio::select! {
            result = timeout(REQUEST_TIMEOUT, operation) => {
                result.map_err(|_| deadline_rpc_error()).and_then(|result| result.map_err(rpc_error))
            }
            () = cancellation.cancelled() => Err(cancelled_rpc_error()),
        };
        if let Ok(mut requests) = self.requests.lock() {
            requests.complete(&request_id);
        }
        result
    }

    pub fn cancel_all(&self) {
        if let Ok(mut requests) = self.requests.lock() {
            requests.cancel_all();
        }
    }
}

#[async_trait]
impl UserService for UserServiceAdapter {
    async fn delete_credential(
        &self,
        _: &ttrpc::r#async::TtrpcContext,
        request: common::ServiceRequest,
    ) -> ttrpc::Result<common::ServiceResponse> {
        self.run_request(&request, Self::delete_credential(self, &request))
            .await
    }

    async fn revoke_export(
        &self,
        _: &ttrpc::r#async::TtrpcContext,
        request: common::ServiceRequest,
    ) -> ttrpc::Result<common::ServiceResponse> {
        self.run_request(&request, Self::revoke_export(self, &request))
            .await
    }

    async fn inspect(
        &self,
        _: &ttrpc::r#async::TtrpcContext,
        request: common::ServiceRequest,
    ) -> ttrpc::Result<common::ServiceResponse> {
        self.run_request(&request, Self::inspect(self, &request))
            .await
    }

    async fn cancel(
        &self,
        _: &ttrpc::r#async::TtrpcContext,
        request: common::CancelRequest,
    ) -> ttrpc::Result<common::CancelResponse> {
        self.requests
            .lock()
            .map_err(|_| internal_rpc_error())?
            .cancel_generated(&request)
            .map_err(|_| invalid_rpc_error())
    }
}

pub async fn run_production() -> Result<(), UserdRuntimeError> {
    let listeners = ActivatedSeqpacketListeners::from_systemd(&[LISTENER_NAME])
        .map_err(|_| UserdRuntimeError::Activation)?;
    let generation = random_generation()?;
    let composition =
        UserdComposition::establish(&Oo7SecretStoreFactory, getuid().as_raw(), generation).await?;
    let (shutdown_sender, shutdown_receiver) = watch::channel(false);
    let signal = shutdown_signal()?;
    tokio::spawn(async move {
        signal.await;
        let _ = shutdown_sender.send(true);
    });
    serve_activated(listeners, Arc::new(composition), shutdown_receiver).await
}

async fn serve_activated(
    listeners: ActivatedSeqpacketListeners,
    composition: Arc<UserdComposition>,
    mut shutdown: watch::Receiver<bool>,
) -> Result<(), UserdRuntimeError> {
    let sessions = Arc::new(Semaphore::new(MAX_CONCURRENT_SESSIONS));
    let mut tasks = JoinSet::new();
    loop {
        tokio::select! {
            biased;
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    break;
                }
            }
            accepted = listeners.accept(LISTENER_NAME) => {
                let socket = accepted.map_err(|_| UserdRuntimeError::Serve)?;
                let Ok(permit) = Arc::clone(&sessions).try_acquire_owned() else {
                    drop(socket);
                    continue;
                };
                let composition = Arc::clone(&composition);
                let session_shutdown = shutdown.clone();
                tasks.spawn(async move {
                    let _permit = permit;
                    let _ = serve_socket(socket, composition, session_shutdown).await;
                });
            }
            Some(_) = tasks.join_next(), if !tasks.is_empty() => {}
        }
    }
    let joined = timeout(SHUTDOWN_TIMEOUT, async {
        while tasks.join_next().await.is_some() {}
    })
    .await;
    if joined.is_err() {
        tasks.abort_all();
    }
    Ok(())
}

async fn serve_socket(
    socket: SeqpacketSocket,
    composition: Arc<UserdComposition>,
    mut shutdown: watch::Receiver<bool>,
) -> Result<(), UserdRuntimeError> {
    let peer = socket
        .acceptor_peer_credentials()
        .map_err(|_| UserdRuntimeError::Serve)?;
    if peer.uid().as_raw() != composition.owner().uid() {
        return Err(UserdRuntimeError::Serve);
    }
    let policy = endpoint_policy(
        composition.owner().agent_generation(),
        channel_binding(peer),
    )?;
    let transport = UnixSeqpacketTransport::new(
        socket,
        Locality::HostLocal,
        policy.limits,
        policy.attachment_policy,
        credit_scopes(),
        reject_attachments(),
        PeerIdentityPolicy::accepted(peer),
    )
    .map_err(|_| UserdRuntimeError::Serve)?;
    let engine = SessionEngine::establish_responder(
        transport,
        policy,
        HandshakeCredentials::Nn,
        Instant::now(),
    )
    .await
    .map_err(|_| UserdRuntimeError::Serve)?;
    let authenticated = AuthenticatedUser::from_verified_peer(
        peer,
        composition.owner().realm_id().clone(),
        EndpointRole::CommandClient,
        composition.owner().agent_generation(),
    );
    let adapter = Arc::new(UserServiceAdapter::new(
        composition.service(),
        authenticated,
    )?);
    let driver: Arc<dyn ComponentSessionDriver> = Arc::new(engine.into_driver());
    let services = create_user_service(adapter.clone());
    let result = tokio::select! {
        result = serve_ttrpc_services(driver, services) => {
            result.map_err(|_| UserdRuntimeError::Serve)
        }
        changed = shutdown.changed() => {
            if changed.is_err() || *shutdown.borrow() {
                Ok(())
            } else {
                Err(UserdRuntimeError::Serve)
            }
        }
    };
    adapter.cancel_all();
    result
}

fn endpoint_policy(
    generation: Generation,
    channel_binding: [u8; 32],
) -> Result<EndpointPolicy, UserdRuntimeError> {
    let service = SERVICE_INVENTORY
        .iter()
        .find(|service| service.package == "d2b.user.v2" && service.service == "UserService")
        .ok_or(UserdRuntimeError::Composition)?;
    let policy = EndpointPolicy {
        purpose: EndpointPurpose::UserAgent,
        purpose_class: PurposeClass::Local,
        initiator_role: EndpointRole::CommandClient,
        responder_role: EndpointRole::UserAgent,
        service: ServicePackage::UserV2,
        schema_fingerprint: service_schema_fingerprint(service),
        noise_profile: NoiseProfile::Nn25519ChaChaPolySha256,
        limits: LimitProfile::local_default(),
        transport_binding: TransportBinding {
            transport: TransportClass::UnixSeqpacket,
            locality: Locality::HostLocal,
            channel_binding,
            identity_evidence: IdentityEvidenceRequirement::DirectionalUnix,
        },
        reconnect_generation: generation.get(),
        attachment_policy: AttachmentPolicy::disabled(),
    };
    d2b_contracts::v2_component_session::HandshakeOffer::from(policy.clone())
        .validate()
        .map_err(|_| UserdRuntimeError::Composition)?;
    Ok(policy)
}

fn owner_binding(uid: u32, generation: Generation) -> Result<OwnerBinding, UserdRuntimeError> {
    let realm = RealmId::derive(&RealmPath::root());
    let workload = WorkloadId::derive(
        &realm,
        &WorkloadName::parse("user-agent").map_err(|_| UserdRuntimeError::Composition)?,
    );
    Ok(OwnerBinding::new(
        uid,
        realm.clone(),
        RoleId::derive(&realm, &workload, RoleKind::WaylandProxy),
        generation,
    ))
}

fn random_generation() -> Result<Generation, UserdRuntimeError> {
    let mut bytes = [0_u8; 8];
    d2b_contracts::v2_provider::Generation::new(1).map_err(|_| UserdRuntimeError::Composition)?;
    crate::services::user::EntropySource::fill(&OsEntropy, &mut bytes)
        .map_err(|_| UserdRuntimeError::Composition)?;
    let value = (u64::from_be_bytes(bytes) % MAX_SAFE_JSON_INTEGER).max(1);
    Generation::new(value).map_err(|_| UserdRuntimeError::Composition)
}

fn channel_binding(peer: PeerCredentials) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"d2b.user.v2\0unix-seqpacket\0");
    digest.update(peer.uid().as_raw().to_be_bytes());
    digest.update(peer.gid().as_raw().to_be_bytes());
    digest.finalize().into()
}

fn credit_scopes() -> CreditScopeSet {
    let pool = || CreditPool::new(1).expect("positive disabled-attachment credit");
    CreditScopeSet::new(pool(), pool(), pool(), pool(), pool(), pool())
}

fn reject_attachments() -> DescriptorPolicyResolver {
    Arc::new(|_| Err(UnixSessionError::DescriptorMismatch))
}

fn request_id(request: &common::ServiceRequest) -> Result<RequestId, UserSecretError> {
    request
        .metadata
        .as_ref()
        .ok_or(UserSecretError::InvalidRequest)
        .and_then(|metadata| {
            RequestId::new(metadata.request_id.clone()).map_err(|_| UserSecretError::InvalidRequest)
        })
}

fn credential_inspection_response(
    request: &common::ServiceRequest,
    metadata: &OwnedSecretMetadata,
) -> common::ServiceResponse {
    let mut digest = Sha256::new();
    digest.update(b"d2b-user-credential-inspection-v2\0");
    digest.update(metadata.source_version.as_str().as_bytes());
    digest.update(metadata.rotation_generation.get().to_be_bytes());
    digest.update(metadata.expires_at_unix_ms.to_be_bytes());
    common::ServiceResponse {
        outcome: common::Outcome::OUTCOME_SUCCEEDED.into(),
        resource_handle: request.resource_id.clone(),
        result_digest: digest.finalize().to_vec(),
        ..Default::default()
    }
}

fn export_inspection_response(
    request: &common::ServiceRequest,
    inspection: &ExportInspection,
) -> common::ServiceResponse {
    let mut digest = Sha256::new();
    digest.update(b"d2b-user-export-inspection-v2\0");
    digest.update([match inspection.state {
        ExportState::Pending => 1,
        ExportState::Active => 2,
        ExportState::Revoked => 3,
        ExportState::Expired => 4,
    }]);
    digest.update(inspection.source_version.as_str().as_bytes());
    digest.update(inspection.export_generation.get().to_be_bytes());
    digest.update(inspection.expires_at_unix_ms.to_be_bytes());
    common::ServiceResponse {
        outcome: common::Outcome::OUTCOME_SUCCEEDED.into(),
        resource_handle: request.resource_id.clone(),
        result_digest: digest.finalize().to_vec(),
        ..Default::default()
    }
}

fn mutation_response(request: &common::ServiceRequest, applied: bool) -> common::ServiceResponse {
    common::ServiceResponse {
        outcome: if applied {
            common::Outcome::OUTCOME_SUCCEEDED
        } else {
            common::Outcome::OUTCOME_NOT_APPLICABLE
        }
        .into(),
        operation_id: request.operation_id.clone(),
        resource_handle: request.resource_id.clone(),
        ..Default::default()
    }
}

fn rpc_error(error: UserSecretError) -> ttrpc::Error {
    let code = match error {
        UserSecretError::InvalidRequest => ttrpc::Code::INVALID_ARGUMENT,
        UserSecretError::Unauthorized => ttrpc::Code::PERMISSION_DENIED,
        UserSecretError::Locked => ttrpc::Code::FAILED_PRECONDITION,
        UserSecretError::NotFound => ttrpc::Code::NOT_FOUND,
        UserSecretError::Conflict => ttrpc::Code::ALREADY_EXISTS,
        UserSecretError::ResourceExhausted => ttrpc::Code::RESOURCE_EXHAUSTED,
        UserSecretError::DeadlineExpired => ttrpc::Code::DEADLINE_EXCEEDED,
        UserSecretError::Unavailable => ttrpc::Code::UNAVAILABLE,
        UserSecretError::AmbiguousMutation => ttrpc::Code::ABORTED,
        UserSecretError::InvariantViolation => ttrpc::Code::INTERNAL,
    };
    ttrpc::Error::RpcStatus(ttrpc::get_status(code, error.to_string()))
}

fn invalid_rpc_error() -> ttrpc::Error {
    rpc_error(UserSecretError::InvalidRequest)
}

fn internal_rpc_error() -> ttrpc::Error {
    rpc_error(UserSecretError::InvariantViolation)
}

fn deadline_rpc_error() -> ttrpc::Error {
    rpc_error(UserSecretError::DeadlineExpired)
}

fn cancelled_rpc_error() -> ttrpc::Error {
    ttrpc::Error::RpcStatus(ttrpc::get_status(
        ttrpc::Code::CANCELLED,
        "user-secret-cancelled".to_owned(),
    ))
}

fn shutdown_signal() -> Result<impl Future<Output = ()>, UserdRuntimeError> {
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .map_err(|_| UserdRuntimeError::Composition)?;
    let mut interrupt = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())
        .map_err(|_| UserdRuntimeError::Composition)?;
    Ok(async move {
        tokio::select! {
            _ = terminate.recv() => {}
            _ = interrupt.recv() => {}
        }
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;
    use crate::services::user::{
        EntropySource, ExportCommitError, ExportCommitPort, ExportHandle, ExportRequest,
        OwnedSecretSelector, SecretMaterial, Tpm2SealContext, Tpm2SealError, Tpm2Sealer,
        UserSecretEntropyError, UserdClock,
    };
    use d2b_contracts::{
        v2_identity::ProviderId,
        v2_services::common::{IdentityScope, RequestMetadata},
    };
    use d2b_provider_credential_secret_service::SecretServiceState;
    use zeroize::Zeroizing;

    #[derive(Default)]
    struct FakeClock(Mutex<u64>);

    impl UserdClock for FakeClock {
        fn now_unix_ms(&self) -> u64 {
            *self.0.lock().expect("clock")
        }
    }

    #[derive(Default)]
    struct FixedEntropy(Mutex<u8>);

    impl EntropySource for FixedEntropy {
        fn fill(&self, destination: &mut [u8]) -> Result<(), UserSecretEntropyError> {
            let mut byte = self.0.lock().expect("entropy");
            destination.fill(*byte);
            *byte = byte.wrapping_add(1);
            Ok(())
        }
    }

    #[derive(Default)]
    struct MemoryStore {
        values: Mutex<BTreeMap<String, (OwnedSecretMetadata, Vec<u8>)>>,
        locked: Mutex<bool>,
    }

    #[async_trait]
    impl SecretStore for MemoryStore {
        async fn state(&self) -> Result<SecretServiceState, SecretStoreError> {
            Ok(if *self.locked.lock().expect("locked") {
                SecretServiceState::Locked
            } else {
                SecretServiceState::Unlocked
            })
        }

        async fn put_owned(
            &self,
            selector: &OwnedSecretSelector,
            metadata: &OwnedSecretMetadata,
            secret: SecretMaterial,
        ) -> Result<(), SecretStoreError> {
            self.values.lock().expect("values").insert(
                selector.provider_id().as_str().to_owned(),
                (metadata.clone(), secret.expose(<[u8]>::to_vec)),
            );
            Ok(())
        }

        async fn metadata(
            &self,
            selector: &OwnedSecretSelector,
        ) -> Result<OwnedSecretMetadata, SecretStoreError> {
            if *self.locked.lock().expect("locked") {
                return Err(SecretStoreError::Locked);
            }
            self.values
                .lock()
                .expect("values")
                .get(selector.provider_id().as_str())
                .map(|(metadata, _)| metadata.clone())
                .ok_or(SecretStoreError::NotFound)
        }

        async fn read_owned(
            &self,
            selector: &OwnedSecretSelector,
        ) -> Result<(OwnedSecretMetadata, SecretMaterial), SecretStoreError> {
            let (metadata, secret) = self
                .values
                .lock()
                .expect("values")
                .get(selector.provider_id().as_str())
                .cloned()
                .ok_or(SecretStoreError::NotFound)?;
            Ok((metadata, SecretMaterial::new(secret)?))
        }

        async fn delete_owned(
            &self,
            selector: &OwnedSecretSelector,
        ) -> Result<bool, SecretStoreError> {
            if *self.locked.lock().expect("locked") {
                return Err(SecretStoreError::Locked);
            }
            Ok(self
                .values
                .lock()
                .expect("values")
                .remove(selector.provider_id().as_str())
                .is_some())
        }
    }

    struct FakeFactory {
        store: Arc<dyn SecretStore>,
        owners: Mutex<Vec<OwnerBinding>>,
    }

    #[async_trait]
    impl SecretStoreFactory for FakeFactory {
        async fn connect(
            &self,
            owner: OwnerBinding,
        ) -> Result<Arc<dyn SecretStore>, SecretStoreError> {
            self.owners.lock().expect("owners").push(owner);
            Ok(Arc::clone(&self.store))
        }
    }

    struct FakeSealer;

    #[async_trait]
    impl Tpm2Sealer for FakeSealer {
        async fn seal(
            &self,
            _: &Tpm2SealContext,
            secret: SecretMaterial,
        ) -> Result<Zeroizing<Vec<u8>>, Tpm2SealError> {
            assert_eq!(secret.len(), 15);
            Ok(Zeroizing::new(b"sealed-only".to_vec()))
        }
    }

    #[derive(Default)]
    struct RecordingCommit {
        states: Mutex<BTreeMap<String, ExportState>>,
    }

    #[async_trait]
    impl ExportCommitPort for RecordingCommit {
        async fn commit(
            &self,
            export: crate::services::user::SealedExport,
        ) -> Result<(), ExportCommitError> {
            assert_eq!(export.sealed_credential(), b"sealed-only");
            self.states
                .lock()
                .expect("states")
                .insert(export.handle().as_str().to_owned(), ExportState::Active);
            Ok(())
        }

        async fn inspect(&self, handle: &ExportHandle) -> Result<ExportState, ExportCommitError> {
            self.states
                .lock()
                .expect("states")
                .get(handle.as_str())
                .copied()
                .ok_or(ExportCommitError::NotFound)
        }

        async fn revoke(&self, handle: &ExportHandle) -> Result<bool, ExportCommitError> {
            let mut states = self.states.lock().expect("states");
            let state = states
                .get_mut(handle.as_str())
                .ok_or(ExportCommitError::NotFound)?;
            let applied = *state != ExportState::Revoked;
            *state = ExportState::Revoked;
            Ok(applied)
        }
    }

    fn owner() -> OwnerBinding {
        owner_binding(1000, Generation::new(7).expect("generation")).expect("owner")
    }

    fn user(owner: &OwnerBinding) -> AuthenticatedUser {
        AuthenticatedUser::command_client(
            owner.uid(),
            owner.realm_id().clone(),
            owner.agent_generation(),
        )
    }

    fn provider(_: &OwnerBinding) -> ProviderId {
        ProviderId::parse("e222222222222222222a").expect("provider")
    }

    fn request(
        owner: &OwnerBinding,
        resource_id: String,
        operation_id: &str,
    ) -> common::ServiceRequest {
        let mutating = operation_id.starts_with("delete-") || operation_id.starts_with("revoke-");
        let now = now_unix_ms();
        common::ServiceRequest {
            metadata: Some(RequestMetadata {
                request_id: Sha256::digest(operation_id.as_bytes())[..16].to_vec(),
                correlation_id: "correlation".to_owned(),
                trace_id: Vec::new(),
                idempotency_key: if mutating { vec![9; 16] } else { Vec::new() },
                issued_at_unix_ms: now.saturating_sub(1_000),
                expires_at_unix_ms: now + 60_000,
                session_generation: owner.agent_generation().get(),
                special_fields: Default::default(),
            })
            .into(),
            scope: Some(IdentityScope {
                realm_id: owner.realm_id().as_str().to_owned(),
                workload_id: String::new(),
                provider_id: String::new(),
                role_id: String::new(),
                special_fields: Default::default(),
            })
            .into(),
            resource_id,
            operation_id: if mutating {
                operation_id.to_owned()
            } else {
                String::new()
            },
            request_digest: if mutating { vec![7; 32] } else { Vec::new() },
            ..Default::default()
        }
    }

    fn now_unix_ms() -> u64 {
        u64::try_from(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time")
                .as_millis(),
        )
        .expect("milliseconds fit")
    }

    #[tokio::test]
    async fn fake_factory_establishes_production_boundary_without_desktop_keyring() {
        let store: Arc<dyn SecretStore> = Arc::new(MemoryStore::default());
        let factory = FakeFactory {
            store,
            owners: Mutex::new(Vec::new()),
        };
        let composition =
            UserdComposition::establish(&factory, 1000, Generation::new(11).expect("generation"))
                .await
                .expect("composition");
        assert_eq!(composition.owner().uid(), 1000);
        assert_eq!(
            composition
                .service()
                .status(&user(composition.owner()))
                .await,
            Ok(SecretServiceState::Unlocked)
        );
        assert_eq!(factory.owners.lock().expect("owners").len(), 1);
        assert!(!format!("{composition:?}").contains("1000"));
    }

    #[tokio::test]
    async fn adapter_status_and_credential_inspect_delete_are_opaque() {
        let owner = owner();
        let provider = provider(&owner);
        let store = Arc::new(MemoryStore::default());
        let metadata = OwnedSecretMetadata {
            source_version: d2b_contracts::v2_provider::SourceVersion::parse("version-private")
                .expect("version"),
            rotation_generation: Generation::new(2).expect("generation"),
            expires_at_unix_ms: 10_000,
        };
        store
            .put_owned(
                &crate::services::user::OwnedSecretSelector::new(owner.clone(), provider.clone()),
                &metadata,
                SecretMaterial::new(b"never-in-response".to_vec()).expect("secret"),
            )
            .await
            .expect("put");
        let composition =
            UserdComposition::from_store(owner.clone(), store.clone()).expect("composition");
        let adapter =
            UserServiceAdapter::new(composition.service(), user(&owner)).expect("adapter");

        let status = adapter
            .inspect(&request(&owner, "status".to_owned(), "inspect-status"))
            .await
            .expect("status");
        assert_eq!(status.resource_handle, "unlocked");
        let inspect = adapter
            .inspect(&request(
                &owner,
                provider.as_str().to_owned(),
                "inspect-one",
            ))
            .await
            .expect("inspect");
        assert_eq!(inspect.result_digest.len(), 32);
        let encoded = format!("{inspect:?}");
        assert!(!encoded.contains("never-in-response"));
        assert!(!encoded.contains("version-private"));

        let deleted = adapter
            .delete_credential(&request(&owner, provider.as_str().to_owned(), "delete-one"))
            .await
            .expect("delete");
        assert_eq!(
            deleted.outcome.enum_value().expect("outcome"),
            common::Outcome::OUTCOME_SUCCEEDED
        );
        assert!(
            store
                .metadata(&crate::services::user::OwnedSecretSelector::new(
                    owner, provider
                ))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn scoped_export_commit_inspect_revoke_and_adapter_dispatch_are_closed() {
        let owner = owner();
        let provider = provider(&owner);
        let store = Arc::new(MemoryStore::default());
        let now = now_unix_ms();
        store
            .put_owned(
                &crate::services::user::OwnedSecretSelector::new(owner.clone(), provider.clone()),
                &OwnedSecretMetadata {
                    source_version: d2b_contracts::v2_provider::SourceVersion::parse(
                        "version-secret",
                    )
                    .expect("version"),
                    rotation_generation: Generation::new(3).expect("generation"),
                    expires_at_unix_ms: now + 60_000,
                },
                SecretMaterial::new(b"credential-data".to_vec()).expect("secret"),
            )
            .await
            .expect("put");
        let commit = Arc::new(RecordingCommit::default());
        let clock = Arc::new(FakeClock(Mutex::new(now)));
        let manager = Arc::new(ScopedExportManager::new_with_runtime(
            owner.clone(),
            store.clone(),
            Arc::new(FakeSealer),
            commit.clone(),
            Arc::new(FixedEntropy::default()),
            clock.clone(),
            Arc::new(NoopSecretMetrics),
        ));
        let exported = manager
            .export(&ExportRequest {
                authenticated_user: user(&owner),
                credential_provider_id: provider,
                target_service: "example.service".to_owned(),
                allowed_purpose: "authenticate".to_owned(),
                host_binding_digest: [4; 32],
                requested_expiry_unix_ms: now + 30_000,
                operation_id: "export-one".to_owned(),
                idempotency_key: "export-key-one".to_owned(),
            })
            .await
            .expect("export");
        assert_eq!(
            manager
                .inspect(&user(&owner), &exported.handle)
                .await
                .expect("inspect")
                .state,
            ExportState::Active
        );
        assert_eq!(
            commit
                .inspect(&exported.handle)
                .await
                .expect("committed export"),
            ExportState::Active
        );

        let service = Arc::new(
            UserSecretService::new_with_clock(
                owner.clone(),
                store,
                manager.clone(),
                clock,
                Arc::new(NoopSecretMetrics),
            )
            .expect("service"),
        );
        let adapter = UserServiceAdapter::new(service, user(&owner)).expect("adapter");
        let inspected = adapter
            .inspect(&request(
                &owner,
                exported.handle.as_str().to_owned(),
                "inspect-export",
            ))
            .await
            .expect("inspect export");
        assert_eq!(inspected.result_digest.len(), 32);
        assert!(!format!("{inspected:?}").contains("credential-data"));
        assert!(!format!("{inspected:?}").contains("version-secret"));

        let revoked = adapter
            .revoke_export(&request(
                &owner,
                exported.handle.as_str().to_owned(),
                "revoke-export",
            ))
            .await
            .expect("revoke");
        assert_eq!(
            revoked.outcome.enum_value().expect("outcome"),
            common::Outcome::OUTCOME_SUCCEEDED
        );
        assert!(
            !manager
                .revoke(&user(&owner), &exported.handle)
                .await
                .unwrap()
        );
        assert_eq!(
            manager
                .inspect(&user(&owner), &exported.handle)
                .await
                .expect("inspect revoked")
                .state,
            ExportState::Revoked
        );
    }

    #[tokio::test]
    async fn adapter_cancels_in_flight_requests_without_exposing_payloads() {
        let owner = owner();
        let composition =
            UserdComposition::from_store(owner.clone(), Arc::new(MemoryStore::default()))
                .expect("composition");
        let adapter = Arc::new(
            UserServiceAdapter::new(composition.service(), user(&owner)).expect("adapter"),
        );
        let pending_adapter = Arc::clone(&adapter);
        let pending_request = request(&owner, "status".to_owned(), "inspect-pending");
        let task = tokio::spawn(async move {
            pending_adapter
                .run_request(
                    &pending_request,
                    std::future::pending::<Result<common::ServiceResponse, UserSecretError>>(),
                )
                .await
        });

        tokio::task::yield_now().await;
        adapter.cancel_all();
        let error = task.await.expect("request task").expect_err("cancelled");
        assert!(matches!(
            error,
            ttrpc::Error::RpcStatus(ref status) if status.code() == ttrpc::Code::CANCELLED
        ));
        assert!(!format!("{error:?}").contains("status"));
    }
}
