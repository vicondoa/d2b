use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};

use async_trait::async_trait;
use d2b_contracts::{
    v2_component_session::{GuestIdentityBindingV1, GuestSessionCredentialV1},
    v2_guest_configured_launches::GuestConfiguredLaunchesV1,
    v2_identity::{RealmId, WorkloadId},
    v2_services::{SERVICE_INVENTORY, service_schema_fingerprint},
};
use d2b_session::ComponentSessionDriver;

use crate::{
    controller_static_identity::{ControllerIdentityAuthority, ControllerProcessBinding},
    daemon_terminal::TerminalFailure,
    guest_terminal::{GuestProxySession, GuestTerminalConnector, GuestTerminalSession},
    production_guest_runtime::{
        BrokerRealmGuestMaterialPort, BundleGuestAuthorityPort, VsockDirectGuestSessionPort,
    },
    supervisor::pidfd_table::PidfdTable,
};

const GUEST_CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_CACHED_GUEST_SESSIONS: usize = 64;
const MAX_RECONNECT_ATTEMPTS: usize = 2;

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct GuestSessionAuthority {
    pub realm_id: RealmId,
    pub workload_id: WorkloadId,
    pub broker_realm_id: String,
    pub broker_workload_id: String,
    pub broker_endpoint: PathBuf,
    pub broker_uid: u32,
    pub broker_gid: u32,
    pub controller_uid: u32,
    pub controller_gid: u32,
    pub controller_generation: u64,
    pub workload_name: String,
    pub vsock_cid: u32,
    pub vsock_port: u32,
    pub runtime_instance_digest: [u8; 32],
    pub direct_schema_fingerprint: [u8; 32],
}

impl std::fmt::Debug for GuestSessionAuthority {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("GuestSessionAuthority(REDACTED)")
    }
}

pub struct AppliedGuestSessionMaterial {
    pub credential: GuestSessionCredentialV1,
    pub configured_launches: GuestConfiguredLaunchesV1,
}

impl std::fmt::Debug for AppliedGuestSessionMaterial {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("AppliedGuestSessionMaterial(REDACTED)")
    }
}

pub struct BootstrapGuestSession {
    pub driver: Arc<dyn ComponentSessionDriver>,
    pub guest_identity_digest: [u8; 32],
    pub guest_static_public_key: [u8; 32],
}

impl std::fmt::Debug for BootstrapGuestSession {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("BootstrapGuestSession(REDACTED)")
    }
}

#[async_trait]
pub trait GuestAuthorityPort: Send + Sync + std::fmt::Debug {
    async fn resolve(&self, workload: &str) -> Result<GuestSessionAuthority, TerminalFailure>;
}

#[async_trait]
pub trait RealmGuestMaterialPort: Send + Sync + std::fmt::Debug {
    async fn apply(
        &self,
        authority: &GuestSessionAuthority,
    ) -> Result<AppliedGuestSessionMaterial, TerminalFailure>;

    async fn persist_enrolled(
        &self,
        authority: &GuestSessionAuthority,
        credential: GuestSessionCredentialV1,
    ) -> Result<(), TerminalFailure>;
}

#[async_trait]
pub trait DirectGuestSessionPort: Send + Sync + std::fmt::Debug {
    async fn bootstrap(
        &self,
        authority: &GuestSessionAuthority,
        material: &AppliedGuestSessionMaterial,
    ) -> Result<BootstrapGuestSession, TerminalFailure>;

    async fn reconnect(
        &self,
        authority: &GuestSessionAuthority,
        material: &AppliedGuestSessionMaterial,
    ) -> Result<Arc<dyn ComponentSessionDriver>, TerminalFailure>;

    async fn is_live(&self, session: &GuestTerminalSession) -> bool;

    async fn close_bootstrap(&self, driver: Arc<dyn ComponentSessionDriver>);
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct CacheKey {
    realm_id: String,
    workload_id: String,
    generation: u64,
    runtime_instance_digest: [u8; 32],
    channel_binding: [u8; 32],
}

pub(crate) struct ProductionGuestTerminalConnector {
    authority: Arc<dyn GuestAuthorityPort>,
    material: Arc<dyn RealmGuestMaterialPort>,
    direct: Arc<dyn DirectGuestSessionPort>,
    cache: Mutex<BTreeMap<CacheKey, Arc<GuestTerminalSession>>>,
}

#[derive(Clone)]
pub struct ProductionGuestRuntimePorts {
    authority: Arc<BundleGuestAuthorityPort>,
    material: Arc<BrokerRealmGuestMaterialPort>,
    direct: Arc<VsockDirectGuestSessionPort>,
    binding: ControllerProcessBinding,
}

impl std::fmt::Debug for ProductionGuestRuntimePorts {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ProductionGuestRuntimePorts(REDACTED)")
    }
}

impl ProductionGuestRuntimePorts {
    pub fn construct(
        bundle_path: PathBuf,
        pidfd_table: Arc<PidfdTable>,
        binding: ControllerProcessBinding,
    ) -> Self {
        let identity = ControllerIdentityAuthority::load(binding.clone());
        let authority = BundleGuestAuthorityPort::new(bundle_path, binding.clone(), pidfd_table);
        let material = BrokerRealmGuestMaterialPort::new(identity.clone());
        let direct = VsockDirectGuestSessionPort::production(identity, Arc::clone(&authority));
        Self {
            authority,
            material,
            direct,
            binding,
        }
    }

    #[cfg(test)]
    fn concrete_type_names(&self) -> (&'static str, &'static str) {
        (
            std::any::type_name_of_val(self.material.as_ref()),
            std::any::type_name_of_val(self.direct.as_ref()),
        )
    }
}

static PRODUCTION_GUEST_RUNTIME_PORTS: OnceLock<ProductionGuestRuntimePorts> = OnceLock::new();

pub fn install_production_guest_runtime_ports(
    ports: ProductionGuestRuntimePorts,
) -> Result<(), TerminalFailure> {
    if ports.binding.generation() == 0 {
        return Err(TerminalFailure::GenerationMismatch);
    }
    PRODUCTION_GUEST_RUNTIME_PORTS
        .set(ports)
        .map_err(|_| TerminalFailure::Conflict)
}

impl std::fmt::Debug for ProductionGuestTerminalConnector {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let cached = self.cache.lock().map(|cache| cache.len()).unwrap_or(0);
        formatter
            .debug_struct("ProductionGuestTerminalConnector")
            .field("cached_sessions", &cached)
            .finish()
    }
}

impl ProductionGuestTerminalConnector {
    pub(crate) fn new(
        authority: Arc<dyn GuestAuthorityPort>,
        material: Arc<dyn RealmGuestMaterialPort>,
        direct: Arc<dyn DirectGuestSessionPort>,
    ) -> Arc<Self> {
        Arc::new(Self {
            authority,
            material,
            direct,
            cache: Mutex::new(BTreeMap::new()),
        })
    }

    pub(crate) fn production() -> Result<Arc<Self>, TerminalFailure> {
        let ports = PRODUCTION_GUEST_RUNTIME_PORTS
            .get()
            .ok_or(TerminalFailure::Internal)?;
        let authority: Arc<dyn GuestAuthorityPort> = ports.authority.clone();
        let material: Arc<dyn RealmGuestMaterialPort> = ports.material.clone();
        let direct: Arc<dyn DirectGuestSessionPort> = ports.direct.clone();
        Ok(Self::new(authority, material, direct))
    }

    pub(crate) async fn connect_scoped_session(
        &self,
        workload: &str,
    ) -> Result<(GuestSessionAuthority, Arc<GuestTerminalSession>), TerminalFailure> {
        let authority =
            tokio::time::timeout(GUEST_CONNECT_TIMEOUT, self.authority.resolve(workload))
                .await
                .map_err(|_| TerminalFailure::Unavailable)??;
        let mut material =
            tokio::time::timeout(GUEST_CONNECT_TIMEOUT, self.material.apply(&authority))
                .await
                .map_err(|_| TerminalFailure::Unavailable)??;
        validate_material(&authority, &material)?;

        let key = CacheKey {
            realm_id: authority.realm_id.as_str().to_owned(),
            workload_id: authority.workload_id.as_str().to_owned(),
            generation: material.credential.session_generation(),
            runtime_instance_digest: authority.runtime_instance_digest,
            channel_binding: *material.credential.channel_binding(),
        };
        let cached = self
            .cache
            .lock()
            .map_err(|_| TerminalFailure::Internal)?
            .get(&key)
            .cloned();
        if let Some(cached) = cached {
            if self.direct.is_live(&cached).await {
                return Ok((authority, cached));
            }
            self.remove_authority_sessions(&authority).await?;
        } else {
            self.remove_stale_sessions(&authority, &key).await?;
        }

        if material.credential.guest_identity_is_unbound() {
            material = self.bootstrap_and_enroll(&authority, material).await?;
        }
        let driver = self.reconnect(&authority, &material).await?;
        if driver.generation() != material.credential.session_generation() {
            return Err(TerminalFailure::GenerationMismatch);
        }
        let session = GuestTerminalSession::from_driver(driver);
        self.insert_cache(key, Arc::clone(&session))?;
        Ok((authority, session))
    }

    async fn connect_session(
        &self,
        workload: &str,
    ) -> Result<Arc<GuestTerminalSession>, TerminalFailure> {
        self.connect_scoped_session(workload)
            .await
            .map(|(_, session)| session)
    }

    async fn bootstrap_and_enroll(
        &self,
        authority: &GuestSessionAuthority,
        material: AppliedGuestSessionMaterial,
    ) -> Result<AppliedGuestSessionMaterial, TerminalFailure> {
        if material.credential.bootstrap().is_none() {
            return Err(TerminalFailure::Protocol);
        }
        let generation = material.credential.session_generation();
        let parent_static = *material.credential.parent_static_public_key();
        let channel_binding = *material.credential.channel_binding();
        let established = tokio::time::timeout(
            GUEST_CONNECT_TIMEOUT,
            self.direct.bootstrap(authority, &material),
        )
        .await
        .map_err(|_| TerminalFailure::Unavailable)??;
        if authority.direct_schema_fingerprint != direct_guest_schema_fingerprint()? {
            return Err(TerminalFailure::GenerationMismatch);
        }
        if established.driver.generation() != generation
            || established.guest_identity_digest == [0; 32]
            || established.guest_static_public_key == [0; 32]
        {
            self.direct.close_bootstrap(established.driver).await;
            return Err(TerminalFailure::GenerationMismatch);
        }
        let enrolled = GuestSessionCredentialV1::new(
            generation,
            parent_static,
            channel_binding,
            GuestIdentityBindingV1::Enrolled {
                guest_identity_digest: established.guest_identity_digest,
                guest_static_public_key: established.guest_static_public_key,
            },
            None,
        )
        .map_err(|_| TerminalFailure::Protocol)?;
        tokio::time::timeout(
            GUEST_CONNECT_TIMEOUT,
            self.material.persist_enrolled(authority, enrolled),
        )
        .await
        .map_err(|_| TerminalFailure::Unavailable)??;
        self.direct.close_bootstrap(established.driver).await;

        let enrolled = tokio::time::timeout(GUEST_CONNECT_TIMEOUT, self.material.apply(authority))
            .await
            .map_err(|_| TerminalFailure::Unavailable)??;
        validate_material(authority, &enrolled)?;
        if enrolled.credential.guest_identity_is_unbound()
            || enrolled.credential.session_generation() != generation
            || enrolled.credential.parent_static_public_key() != &parent_static
            || enrolled.credential.channel_binding() != &channel_binding
            || enrolled.credential.guest_identity_digest()
                != Some(&established.guest_identity_digest)
            || enrolled.credential.guest_static_public_key()
                != Some(&established.guest_static_public_key)
        {
            return Err(TerminalFailure::GenerationMismatch);
        }
        Ok(enrolled)
    }

    async fn reconnect(
        &self,
        authority: &GuestSessionAuthority,
        material: &AppliedGuestSessionMaterial,
    ) -> Result<Arc<dyn ComponentSessionDriver>, TerminalFailure> {
        let mut last = TerminalFailure::Unavailable;
        for _ in 0..MAX_RECONNECT_ATTEMPTS {
            match tokio::time::timeout(
                GUEST_CONNECT_TIMEOUT,
                self.direct.reconnect(authority, material),
            )
            .await
            {
                Ok(Ok(driver)) => return Ok(driver),
                Ok(Err(error)) => last = error,
                Err(_) => last = TerminalFailure::Unavailable,
            }
        }
        Err(last)
    }

    async fn remove_stale_sessions(
        &self,
        authority: &GuestSessionAuthority,
        current: &CacheKey,
    ) -> Result<(), TerminalFailure> {
        let removed = remove_cached_where(&self.cache, |key| {
            key.realm_id != authority.realm_id.as_str()
                || key.workload_id != authority.workload_id.as_str()
                || key == current
        })?;
        close_cached_sessions(removed).await;
        Ok(())
    }

    async fn remove_authority_sessions(
        &self,
        authority: &GuestSessionAuthority,
    ) -> Result<(), TerminalFailure> {
        let removed = remove_cached_where(&self.cache, |key| {
            key.realm_id != authority.realm_id.as_str()
                || key.workload_id != authority.workload_id.as_str()
        })?;
        close_cached_sessions(removed).await;
        Ok(())
    }

    fn insert_cache(
        &self,
        key: CacheKey,
        session: Arc<GuestTerminalSession>,
    ) -> Result<(), TerminalFailure> {
        let mut cache = self.cache.lock().map_err(|_| TerminalFailure::Internal)?;
        while cache.len() >= MAX_CACHED_GUEST_SESSIONS {
            let Some(oldest) = cache.keys().next().cloned() else {
                break;
            };
            if let Some(removed) = cache.remove(&oldest) {
                tokio::spawn(async move {
                    removed.close_session().await;
                });
            }
        }

        cache.insert(key, session);
        Ok(())
    }
}

fn remove_cached_where(
    cache: &Mutex<BTreeMap<CacheKey, Arc<GuestTerminalSession>>>,
    keep: impl Fn(&CacheKey) -> bool,
) -> Result<Vec<Arc<GuestTerminalSession>>, TerminalFailure> {
    let mut cache = cache.lock().map_err(|_| TerminalFailure::Internal)?;
    let removed_keys = cache
        .keys()
        .filter(|key| !keep(key))
        .cloned()
        .collect::<Vec<_>>();
    Ok(removed_keys
        .into_iter()
        .filter_map(|key| cache.remove(&key))
        .collect())
}

async fn close_cached_sessions(sessions: Vec<Arc<GuestTerminalSession>>) {
    for session in sessions {
        session.close_session().await;
    }
}

#[async_trait]
impl GuestTerminalConnector for ProductionGuestTerminalConnector {
    async fn acquire_material(&self, _: &str) -> Result<GuestSessionCredentialV1, TerminalFailure> {
        Err(TerminalFailure::Internal)
    }

    async fn connect_with_material(
        &self,
        _: &str,
        _: GuestSessionCredentialV1,
    ) -> Result<Arc<GuestTerminalSession>, TerminalFailure> {
        Err(TerminalFailure::Internal)
    }

    async fn connect(&self, workload: &str) -> Result<Arc<GuestTerminalSession>, TerminalFailure> {
        self.connect_session(workload).await
    }

    async fn connect_proxy(
        &self,
        workload: &str,
    ) -> Result<Arc<dyn GuestProxySession>, TerminalFailure> {
        let session: Arc<dyn GuestProxySession> = self.connect_session(workload).await?;
        Ok(session)
    }
}

fn validate_material(
    authority: &GuestSessionAuthority,
    material: &AppliedGuestSessionMaterial,
) -> Result<(), TerminalFailure> {
    if material.credential.session_generation() == 0
        || material.configured_launches.realm_id() != &authority.realm_id
        || material.configured_launches.workload_id() != &authority.workload_id
        || material.configured_launches.workload_digest() == &[0; 32]
    {
        return Err(TerminalFailure::GenerationMismatch);
    }
    Ok(())
}

pub(crate) fn direct_guest_schema_fingerprint() -> Result<[u8; 32], TerminalFailure> {
    SERVICE_INVENTORY
        .iter()
        .find(|service| service.package == "d2b.guest.v2")
        .map(service_schema_fingerprint)
        .ok_or(TerminalFailure::Internal)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use d2b_contracts::{
        v2_component_session::{
            BootstrapPskBinding, CloseReason, GuestBootstrapCredentialV1, GuestBootstrapPsk,
            OperationId, Remediation, RequestId, SessionErrorCode,
        },
        v2_guest_configured_launches::{GuestConfiguredLaunchEntryV1, GuestConfiguredLaunchesV1},
        v2_identity::{RealmPath, WorkloadName},
    };
    use d2b_core::configured_argv::ConfiguredArgv;
    use d2b_realm_core::ProtocolToken;
    use d2b_session::{
        Cancellation, OwnedAttachment, RequestRegistry, SessionError, SessionEvent, StreamEvent,
        StreamId,
    };

    use super::*;

    const GENERATION: u64 = 9;

    fn authority() -> GuestSessionAuthority {
        let realm_path = RealmPath::parse("work.local-root").unwrap();
        let realm_id = RealmId::derive(&realm_path);
        let workload_id = WorkloadId::derive(&realm_id, &WorkloadName::parse("browser").unwrap());
        GuestSessionAuthority {
            realm_id,
            workload_id,
            broker_realm_id: "work".to_owned(),
            broker_workload_id: "editor".to_owned(),
            broker_endpoint: PathBuf::from("realm-broker"),
            broker_uid: 1001,
            broker_gid: 1001,
            controller_uid: 1000,
            controller_gid: 1000,
            controller_generation: GENERATION,
            workload_name: "corp-vm".to_owned(),
            vsock_cid: 42,
            vsock_port: 14_318,
            runtime_instance_digest: [9; 32],
            direct_schema_fingerprint: direct_guest_schema_fingerprint().unwrap(),
        }
    }

    #[derive(Debug)]
    struct FakeAuthority {
        runtime_instance_digest: Mutex<[u8; 32]>,
    }

    impl FakeAuthority {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                runtime_instance_digest: Mutex::new([9; 32]),
            })
        }

        fn restart_same_generation(&self) {
            *self.runtime_instance_digest.lock().unwrap() = [0x99; 32];
        }
    }

    #[async_trait]
    impl GuestAuthorityPort for FakeAuthority {
        async fn resolve(&self, workload: &str) -> Result<GuestSessionAuthority, TerminalFailure> {
            if workload == "corp-vm" {
                let mut authority = authority();
                authority.runtime_instance_digest = *self.runtime_instance_digest.lock().unwrap();
                Ok(authority)
            } else {
                Err(TerminalFailure::NotFound)
            }
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum MaterialFault {
        None,
        ReplayBootstrap,
        MismatchedEnrollment,
        NoBroker,
    }

    struct FakeMaterial {
        fault: MaterialFault,
        enrolled: Mutex<Option<([u8; 32], [u8; 32])>>,
        applies: AtomicUsize,
        persists: AtomicUsize,
    }

    impl std::fmt::Debug for FakeMaterial {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("FakeMaterial(REDACTED)")
        }
    }

    impl FakeMaterial {
        fn new(fault: MaterialFault) -> Arc<Self> {
            Arc::new(Self {
                fault,
                enrolled: Mutex::new(None),
                applies: AtomicUsize::new(0),
                persists: AtomicUsize::new(0),
            })
        }

        fn launches(authority: &GuestSessionAuthority) -> GuestConfiguredLaunchesV1 {
            let entry = GuestConfiguredLaunchEntryV1::new(
                ProtocolToken::parse("browser").unwrap(),
                ConfiguredArgv::new(vec!["browser-bin".to_owned()]).unwrap(),
                true,
            )
            .unwrap();
            GuestConfiguredLaunchesV1::new(
                authority.realm_id.clone(),
                authority.workload_id.clone(),
                [8; 32],
                vec![entry],
            )
            .unwrap()
        }

        fn bootstrap_credential() -> GuestSessionCredentialV1 {
            let mut psk = [5; 32];
            let bootstrap = GuestBootstrapCredentialV1::new(
                BootstrapPskBinding {
                    operation_id: OperationId::new(vec![3; 16]).unwrap(),
                    replay_nonce: [4; 32],
                    expires_at_unix_ms: 200,
                },
                100,
                GuestBootstrapPsk::copy_from_and_zeroize(&mut psk).unwrap(),
            )
            .unwrap();
            GuestSessionCredentialV1::new(
                GENERATION,
                [1; 32],
                [2; 32],
                GuestIdentityBindingV1::UnboundBootstrap,
                Some(bootstrap),
            )
            .unwrap()
        }

        fn enrolled_credential(identity: [u8; 32], key: [u8; 32]) -> GuestSessionCredentialV1 {
            GuestSessionCredentialV1::new(
                GENERATION,
                [1; 32],
                [2; 32],
                GuestIdentityBindingV1::Enrolled {
                    guest_identity_digest: identity,
                    guest_static_public_key: key,
                },
                None,
            )
            .unwrap()
        }
    }

    #[async_trait]
    impl RealmGuestMaterialPort for FakeMaterial {
        async fn apply(
            &self,
            authority: &GuestSessionAuthority,
        ) -> Result<AppliedGuestSessionMaterial, TerminalFailure> {
            assert_eq!(authority.broker_endpoint, PathBuf::from("realm-broker"));
            self.applies.fetch_add(1, Ordering::AcqRel);
            if self.fault == MaterialFault::NoBroker {
                return Err(TerminalFailure::Unavailable);
            }
            let enrolled = *self.enrolled.lock().unwrap();
            let credential = match (self.fault, enrolled) {
                (MaterialFault::ReplayBootstrap, _) | (_, None) => Self::bootstrap_credential(),
                (MaterialFault::MismatchedEnrollment, Some(_)) => {
                    Self::enrolled_credential([0x66; 32], [0x77; 32])
                }
                (MaterialFault::None, Some((identity, key))) => {
                    Self::enrolled_credential(identity, key)
                }
                (MaterialFault::NoBroker, _) => unreachable!(),
            };
            Ok(AppliedGuestSessionMaterial {
                credential,
                configured_launches: Self::launches(authority),
            })
        }

        async fn persist_enrolled(
            &self,
            authority: &GuestSessionAuthority,
            credential: GuestSessionCredentialV1,
        ) -> Result<(), TerminalFailure> {
            assert_eq!(authority.broker_endpoint, PathBuf::from("realm-broker"));
            let identity = *credential
                .guest_identity_digest()
                .ok_or(TerminalFailure::Protocol)?;
            let key = *credential
                .guest_static_public_key()
                .ok_or(TerminalFailure::Protocol)?;
            *self.enrolled.lock().unwrap() = Some((identity, key));
            self.persists.fetch_add(1, Ordering::AcqRel);
            Ok(())
        }
    }

    struct FakeDriver {
        generation: u64,
        registry: Mutex<RequestRegistry>,
        closed: Option<Arc<AtomicUsize>>,
    }

    impl FakeDriver {
        fn new(generation: u64) -> Arc<Self> {
            Arc::new(Self {
                generation,
                registry: Mutex::new(RequestRegistry::new(generation).unwrap()),
                closed: None,
            })
        }

        fn tracked(generation: u64, closed: Arc<AtomicUsize>) -> Arc<Self> {
            Arc::new(Self {
                generation,
                registry: Mutex::new(RequestRegistry::new(generation).unwrap()),
                closed: Some(closed),
            })
        }
    }

    impl std::fmt::Debug for FakeDriver {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("FakeDriver(REDACTED)")
        }
    }

    #[async_trait]
    impl ComponentSessionDriver for FakeDriver {
        fn generation(&self) -> u64 {
            self.generation
        }

        async fn start_ttrpc(&self, _: RequestId, _: Vec<u8>) -> d2b_session::Result<()> {
            Ok(())
        }

        async fn complete_ttrpc(&self, _: RequestId) -> d2b_session::Result<bool> {
            Ok(true)
        }

        async fn cancel(&self, _: u64, _: RequestId) -> d2b_session::Result<()> {
            Ok(())
        }

        async fn send_ttrpc(&self, _: Vec<u8>) -> d2b_session::Result<()> {
            Ok(())
        }

        async fn receive_ttrpc(&self) -> d2b_session::Result<Vec<u8>> {
            Err(SessionError::new(SessionErrorCode::Cancelled))
        }

        async fn register_inbound_call(
            &self,
            request_id: RequestId,
        ) -> d2b_session::Result<Cancellation> {
            self.registry.lock().unwrap().register(request_id)
        }

        async fn complete_inbound_call(&self, request_id: RequestId) -> d2b_session::Result<bool> {
            Ok(self.registry.lock().unwrap().complete(&request_id))
        }

        async fn remove_inbound_call(&self, request_id: RequestId) -> d2b_session::Result<bool> {
            Ok(self.registry.lock().unwrap().remove(&request_id))
        }

        async fn send_attachments(&self, _: Vec<OwnedAttachment>) -> d2b_session::Result<()> {
            Ok(())
        }

        async fn receive_attachments(&self) -> d2b_session::Result<Vec<OwnedAttachment>> {
            Ok(Vec::new())
        }

        async fn open_named_stream(&self, _: StreamId, _: u32, _: u32) -> d2b_session::Result<()> {
            Ok(())
        }

        async fn send_named_stream(&self, _: StreamId, _: Vec<u8>) -> d2b_session::Result<()> {
            Ok(())
        }

        async fn receive_named_stream(&self) -> d2b_session::Result<StreamEvent> {
            Err(SessionError::new(SessionErrorCode::Cancelled))
        }

        async fn grant_named_stream_credit(&self, _: StreamId, _: u32) -> d2b_session::Result<()> {
            Ok(())
        }

        async fn close_named_stream(&self, _: StreamId) -> d2b_session::Result<()> {
            Ok(())
        }

        async fn reset_named_stream(&self, _: StreamId) -> d2b_session::Result<()> {
            Ok(())
        }

        async fn drive_keepalive(&self, _: std::time::Instant) -> d2b_session::Result<()> {
            Ok(())
        }

        async fn receive_control(&self) -> d2b_session::Result<SessionEvent> {
            Err(SessionError::new(SessionErrorCode::Cancelled))
        }

        async fn close(&self, _: CloseReason, _: Remediation) -> d2b_session::Result<()> {
            if let Some(closed) = &self.closed {
                closed.fetch_add(1, Ordering::AcqRel);
            }
            Ok(())
        }
    }

    #[derive(Debug)]
    struct FakeDirect {
        live: AtomicBool,
        bootstrap: AtomicUsize,
        reconnect: AtomicUsize,
        closed: AtomicUsize,
        session_closed: Arc<AtomicUsize>,
    }

    impl FakeDirect {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                live: AtomicBool::new(true),
                bootstrap: AtomicUsize::new(0),
                reconnect: AtomicUsize::new(0),
                closed: AtomicUsize::new(0),
                session_closed: Arc::new(AtomicUsize::new(0)),
            })
        }
    }

    #[async_trait]
    impl DirectGuestSessionPort for FakeDirect {
        async fn bootstrap(
            &self,
            authority: &GuestSessionAuthority,
            material: &AppliedGuestSessionMaterial,
        ) -> Result<BootstrapGuestSession, TerminalFailure> {
            assert_eq!(
                authority.direct_schema_fingerprint,
                direct_guest_schema_fingerprint().unwrap()
            );
            if !material.credential.guest_identity_is_unbound() {
                return Err(TerminalFailure::Protocol);
            }
            self.bootstrap.fetch_add(1, Ordering::AcqRel);
            Ok(BootstrapGuestSession {
                driver: FakeDriver::new(GENERATION),
                guest_identity_digest: [6; 32],
                guest_static_public_key: [7; 32],
            })
        }

        async fn reconnect(
            &self,
            authority: &GuestSessionAuthority,
            material: &AppliedGuestSessionMaterial,
        ) -> Result<Arc<dyn ComponentSessionDriver>, TerminalFailure> {
            assert_eq!(
                authority.direct_schema_fingerprint,
                direct_guest_schema_fingerprint().unwrap()
            );
            if material.credential.guest_identity_is_unbound() {
                return Err(TerminalFailure::Protocol);
            }
            self.reconnect.fetch_add(1, Ordering::AcqRel);
            let driver: Arc<dyn ComponentSessionDriver> =
                FakeDriver::tracked(GENERATION, Arc::clone(&self.session_closed));
            Ok(driver)
        }

        async fn is_live(&self, _: &GuestTerminalSession) -> bool {
            self.live.load(Ordering::Acquire)
        }

        async fn close_bootstrap(&self, _: Arc<dyn ComponentSessionDriver>) {
            self.closed.fetch_add(1, Ordering::AcqRel);
        }
    }

    fn connector(
        material: Arc<FakeMaterial>,
        direct: Arc<FakeDirect>,
    ) -> Arc<ProductionGuestTerminalConnector> {
        ProductionGuestTerminalConnector::new(FakeAuthority::new(), material, direct)
    }

    #[tokio::test]
    async fn bootstrap_persists_exact_enrollment_then_reconnects_and_caches() {
        let material = FakeMaterial::new(MaterialFault::None);
        let direct = FakeDirect::new();
        let connector = connector(Arc::clone(&material), Arc::clone(&direct));

        let first = connector.connect("corp-vm").await.unwrap();
        assert_eq!(first.generation(), GENERATION);
        assert_eq!(material.applies.load(Ordering::Acquire), 2);
        assert_eq!(material.persists.load(Ordering::Acquire), 1);
        assert_eq!(direct.bootstrap.load(Ordering::Acquire), 1);
        assert_eq!(direct.reconnect.load(Ordering::Acquire), 1);
        assert_eq!(direct.closed.load(Ordering::Acquire), 1);

        let second = connector.connect("corp-vm").await.unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(material.applies.load(Ordering::Acquire), 3);
        assert_eq!(direct.reconnect.load(Ordering::Acquire), 1);
    }

    #[tokio::test]
    async fn disconnected_cache_entry_reconnects_with_current_generation() {
        let material = FakeMaterial::new(MaterialFault::None);
        let direct = FakeDirect::new();
        let connector = connector(Arc::clone(&material), Arc::clone(&direct));
        connector.connect("corp-vm").await.unwrap();
        direct.live.store(false, Ordering::Release);
        connector.connect("corp-vm").await.unwrap();
        assert_eq!(direct.reconnect.load(Ordering::Acquire), 2);
    }

    #[tokio::test]
    async fn same_generation_runtime_restart_closes_and_replaces_cached_session() {
        let material = FakeMaterial::new(MaterialFault::None);
        let direct = FakeDirect::new();
        let authority = FakeAuthority::new();
        let connector = ProductionGuestTerminalConnector::new(
            authority.clone(),
            Arc::clone(&material) as Arc<dyn RealmGuestMaterialPort>,
            Arc::clone(&direct) as Arc<dyn DirectGuestSessionPort>,
        );
        let first = connector.connect("corp-vm").await.unwrap();
        authority.restart_same_generation();
        let second = connector.connect("corp-vm").await.unwrap();

        assert!(!Arc::ptr_eq(&first, &second));
        assert_eq!(first.generation(), second.generation());
        assert_eq!(direct.reconnect.load(Ordering::Acquire), 2);
        assert_eq!(direct.session_closed.load(Ordering::Acquire), 1);
        let cache = connector.cache.lock().unwrap();
        assert_eq!(cache.len(), 1);
        assert_eq!(
            cache.keys().next().unwrap().runtime_instance_digest,
            [0x99; 32]
        );
    }

    #[tokio::test]
    async fn replay_mismatch_and_missing_broker_fail_closed() {
        for fault in [
            MaterialFault::ReplayBootstrap,
            MaterialFault::MismatchedEnrollment,
            MaterialFault::NoBroker,
        ] {
            let material = FakeMaterial::new(fault);
            let direct = FakeDirect::new();
            let connector = connector(material, direct);
            assert!(connector.connect("corp-vm").await.is_err(), "{fault:?}");
        }
    }

    #[test]
    fn production_constructor_owns_concrete_broker_and_vsock_ports() {
        let root = tempfile::tempdir().unwrap();
        let binding = ControllerProcessBinding::from_process(
            GENERATION,
            rustix::process::getuid().as_raw(),
            rustix::process::getgid().as_raw(),
        )
        .unwrap();
        let ports = ProductionGuestRuntimePorts::construct(
            root.path().join("missing-bundle.json"),
            Arc::new(PidfdTable::new(root.path().join("pidfd-table.json"))),
            binding,
        );
        let (material, direct) = ports.concrete_type_names();
        assert!(material.ends_with("BrokerRealmGuestMaterialPort"));
        assert!(direct.ends_with("VsockDirectGuestSessionPort"));
        assert!(!format!("{ports:?}").contains("Unavailable"));
    }
}
