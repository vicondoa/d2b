//! Local-root allocation and paired realm-child dispatch.

use std::collections::BTreeMap;
use std::ffi::CString;
use std::fmt;
use std::fs::File;
use std::io::{Seek, SeekFrom, Write};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd};
use std::os::unix::fs::FileExt;
use std::sync::Arc;
use std::time::Duration;

use d2b_contracts::v2_component_session::AttachmentAccess;
use d2b_contracts::v2_services::{StrictWireMessage, broker, common};
use d2b_host::realm_broker_bootstrap::{
    REALM_BROKER_AUTHORITY_RESOURCE_ID, RealmBrokerChildAuthority,
};
use d2b_host::realm_children::{
    PidfdEvidence, PidfdIdentityPolicy, PidfdIdentityVerifier, RealmChildBootstrapEndpoints,
    RealmChildCredentialError, RealmChildDescriptorSet, RealmChildFdBinding, RealmChildFdKind,
    RealmChildFdTable, RealmChildIdentity, RealmChildLaunchRecord, RealmChildPlanError,
    RealmChildRole, UnixSessionError,
};
use d2b_host::realm_controller_bootstrap::{
    REALM_CONTROLLER_AUTHORITY_RESOURCE_ID, RealmControllerChildAuthority,
};
use d2b_realm_core::allocator::{
    AllocatorLease, AllocatorReasonCode, GrantedHostResource as CoreGrantedResource,
    HostResourceKind as CoreResourceKind, LeaseAllocationRequest, LeaseAllocationResult,
    LeaseOwner, LeaseResourceRequest, ResourceAcquisitionOrder as CoreAcquisitionOrder,
    ResourceDelegation, ResourceShareMode as CoreShareMode,
};
use d2b_realm_core::allocator_engine::{
    AllocatorEngineDecision, AllocatorEngineError, AllocatorLedger, AllocatorLiveness,
    LocalRootAllocatorEngine, ObservedAllocatorState,
};
use d2b_realm_core::ids::{
    ControllerGenerationId, CorrelationId, HostResourceId, IdempotencyKey, NodeId, OperationId,
    RealmId,
};
use d2b_realm_core::realm::RealmPath;
use nix::libc;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::sys::pidfd_sys::{SpawnOutcome, clone3_spawn_realm_child, pidfd_send_signal};

const MAX_REALM_GUEST_RUNTIME_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Debug)]
pub struct ServiceReply<T, A = Vec<OwnedFd>> {
    pub message: T,
    pub attachments: A,
}

pub struct VerifiedPidfdAttachment {
    pidfd: OwnedFd,
    evidence: PidfdEvidence,
    role: RealmChildRole,
    process_id: String,
    pid: u32,
    executable_digest: [u8; 32],
}

impl fmt::Debug for VerifiedPidfdAttachment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("VerifiedPidfdAttachment(REDACTED)")
    }
}

impl VerifiedPidfdAttachment {
    fn from_spawned(child: SpawnedRealmChild) -> Result<Self, AllocatorServiceError> {
        if child.evidence.expected_pid().as_raw_nonzero().get() as u32 != child.pid {
            return Err(AllocatorServiceError::PidfdEvidenceCorrelation);
        }
        let flags = rustix::io::fcntl_getfd(&child.pidfd)
            .map_err(|error| AllocatorServiceError::Descriptor(error.into()))?;
        if !flags.contains(rustix::io::FdFlags::CLOEXEC) {
            return Err(AllocatorServiceError::PidfdMissingCloexec);
        }
        Ok(Self {
            pidfd: child.pidfd,
            evidence: child.evidence,
            role: child.identity.role,
            process_id: child.identity.process_id,
            pid: child.pid,
            executable_digest: child.identity.executable_digest,
        })
    }

    pub const fn role(&self) -> RealmChildRole {
        self.role
    }

    pub const fn attachment_index(&self) -> u32 {
        match self.role {
            RealmChildRole::Controller => 0,
            RealmChildRole::Broker => 1,
        }
    }

    pub fn process_id(&self) -> &str {
        &self.process_id
    }

    pub const fn pid(&self) -> u32 {
        self.pid
    }

    pub fn as_fd(&self) -> BorrowedFd<'_> {
        self.pidfd.as_fd()
    }

    pub fn bind_policy(
        self,
        verifier: Arc<dyn PidfdIdentityVerifier>,
    ) -> Result<PolicyBoundPidfdAttachment, AllocatorServiceError> {
        let policy = PidfdIdentityPolicy::new(
            &self.pidfd,
            AttachmentAccess::ReadWrite,
            self.evidence,
            verifier,
        )
        .map_err(AllocatorServiceError::PidfdPolicy)?;
        Ok(PolicyBoundPidfdAttachment {
            pidfd: self.pidfd,
            policy,
            role: self.role,
            process_id: self.process_id,
            pid: self.pid,
        })
    }
}

pub struct PolicyBoundPidfdAttachment {
    pidfd: OwnedFd,
    policy: PidfdIdentityPolicy,
    role: RealmChildRole,
    process_id: String,
    pid: u32,
}

impl fmt::Debug for PolicyBoundPidfdAttachment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PolicyBoundPidfdAttachment(REDACTED)")
    }
}

impl PolicyBoundPidfdAttachment {
    pub const fn role(&self) -> RealmChildRole {
        self.role
    }

    pub const fn attachment_index(&self) -> u32 {
        match self.role {
            RealmChildRole::Controller => 0,
            RealmChildRole::Broker => 1,
        }
    }

    pub fn process_id(&self) -> &str {
        &self.process_id
    }

    pub const fn pid(&self) -> u32 {
        self.pid
    }

    pub fn as_fd(&self) -> BorrowedFd<'_> {
        self.pidfd.as_fd()
    }

    pub fn into_transport_parts(self) -> (OwnedFd, PidfdIdentityPolicy) {
        (self.pidfd, self.policy)
    }
}

pub struct VerifiedPidfdAttachments {
    controller: VerifiedPidfdAttachment,
    broker: VerifiedPidfdAttachment,
}

impl fmt::Debug for VerifiedPidfdAttachments {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("VerifiedPidfdAttachments(REDACTED)")
    }
}

impl VerifiedPidfdAttachments {
    fn from_spawned(pair: SpawnedRealmPair) -> Result<Self, AllocatorServiceError> {
        let controller = VerifiedPidfdAttachment::from_spawned(pair.controller)?;
        let broker = VerifiedPidfdAttachment::from_spawned(pair.broker)?;
        if controller.role != RealmChildRole::Controller
            || broker.role != RealmChildRole::Broker
            || controller.pid == broker.pid
            || controller.process_id == broker.process_id
        {
            return Err(AllocatorServiceError::PidfdEvidenceCorrelation);
        }
        Ok(Self { controller, broker })
    }

    pub const fn len(&self) -> usize {
        2
    }

    pub const fn is_empty(&self) -> bool {
        false
    }

    pub const fn controller(&self) -> &VerifiedPidfdAttachment {
        &self.controller
    }

    pub const fn broker(&self) -> &VerifiedPidfdAttachment {
        &self.broker
    }

    pub fn bind_policies(
        self,
        controller_verifier: Arc<dyn PidfdIdentityVerifier>,
        broker_verifier: Arc<dyn PidfdIdentityVerifier>,
    ) -> Result<PolicyBoundPidfdAttachments, AllocatorServiceError> {
        Ok(PolicyBoundPidfdAttachments {
            controller: self.controller.bind_policy(controller_verifier)?,
            broker: self.broker.bind_policy(broker_verifier)?,
        })
    }

    fn validate_response_correlation(
        &self,
        response: &broker::SpawnRealmChildrenResponse,
    ) -> Result<(), AllocatorServiceError> {
        if response.children.len() != 2 {
            return Err(AllocatorServiceError::PidfdEvidenceCorrelation);
        }
        let mut controller_seen = false;
        let mut broker_seen = false;
        for child in &response.children {
            match child.role.value() {
                1 if !controller_seen
                    && child.pidfd_attachment_index == 0
                    && child.process_id == self.controller.process_id
                    && child.executable_digest == self.controller.executable_digest
                    && child.pid == self.controller.pid =>
                {
                    controller_seen = true;
                }
                2 if !broker_seen
                    && child.pidfd_attachment_index == 1
                    && child.process_id == self.broker.process_id
                    && child.executable_digest == self.broker.executable_digest
                    && child.pid == self.broker.pid =>
                {
                    broker_seen = true;
                }
                _ => return Err(AllocatorServiceError::PidfdEvidenceCorrelation),
            }
        }
        if !controller_seen || !broker_seen {
            return Err(AllocatorServiceError::PidfdEvidenceCorrelation);
        }
        Ok(())
    }
}

pub struct PolicyBoundPidfdAttachments {
    controller: PolicyBoundPidfdAttachment,
    broker: PolicyBoundPidfdAttachment,
}

impl fmt::Debug for PolicyBoundPidfdAttachments {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PolicyBoundPidfdAttachments(REDACTED)")
    }
}

impl PolicyBoundPidfdAttachments {
    pub const fn controller(&self) -> &PolicyBoundPidfdAttachment {
        &self.controller
    }

    pub const fn broker(&self) -> &PolicyBoundPidfdAttachment {
        &self.broker
    }

    pub fn into_transport_order(self) -> [PolicyBoundPidfdAttachment; 2] {
        [self.controller, self.broker]
    }
}

pub trait AllocatedResourceBackend {
    fn materialize(
        &mut self,
        lease: &AllocatorLease,
        resource: &CoreGrantedResource,
    ) -> Result<Option<OwnedFd>, AllocatorServiceError>;
}

#[derive(Debug, Default)]
pub struct NoFdResourceBackend;

impl AllocatedResourceBackend for NoFdResourceBackend {
    fn materialize(
        &mut self,
        _lease: &AllocatorLease,
        _resource: &CoreGrantedResource,
    ) -> Result<Option<OwnedFd>, AllocatorServiceError> {
        Ok(None)
    }
}

#[derive(Debug, Default)]
pub struct LinuxAllocatedResourceBackend {
    descriptors: BTreeMap<String, OwnedFd>,
}

impl LinuxAllocatedResourceBackend {
    pub fn insert(
        &mut self,
        resource_id: impl Into<String>,
        fd: OwnedFd,
    ) -> Result<(), AllocatorServiceError> {
        let resource_id = resource_id.into();
        if self.descriptors.insert(resource_id, fd).is_some() {
            return Err(AllocatorServiceError::Invariant(
                "duplicate live resource descriptor",
            ));
        }
        Ok(())
    }
}

impl AllocatedResourceBackend for LinuxAllocatedResourceBackend {
    fn materialize(
        &mut self,
        _lease: &AllocatorLease,
        resource: &CoreGrantedResource,
    ) -> Result<Option<OwnedFd>, AllocatorServiceError> {
        self.descriptors
            .get(resource.resource_id.as_str())
            .map(OwnedFd::try_clone)
            .transpose()
            .map_err(AllocatorServiceError::Descriptor)
    }
}

pub trait RealmLaunchRecordResolver {
    fn resolve(
        &self,
        realm_id: &str,
        controller_generation_id: &str,
    ) -> Result<RealmChildLaunchRecord, AllocatorServiceError>;
}

pub trait RealmChildSpawner {
    fn spawn_pair(
        &self,
        record: &RealmChildLaunchRecord,
        controller_fds: RealmChildDescriptorSet,
        broker_fds: RealmChildDescriptorSet,
        bootstrap: RealmChildBootstrapEndpoints,
    ) -> Result<PendingSpawnedRealmPair, AllocatorServiceError>;

    fn terminate_pair(&self, pair: &PendingSpawnedRealmPair) {
        let _ = pidfd_send_signal(pair.controller.pidfd.as_fd(), libc::SIGKILL);
        let _ = pidfd_send_signal(pair.broker.pidfd.as_fd(), libc::SIGKILL);
    }
}

#[derive(Debug)]
struct SpawnedRealmPair {
    controller: SpawnedRealmChild,
    broker: SpawnedRealmChild,
}

#[derive(Debug)]
struct SpawnedRealmChild {
    identity: RealmChildIdentity,
    pid: u32,
    pidfd: OwnedFd,
    evidence: PidfdEvidence,
}

#[derive(Debug)]
pub struct PendingSpawnedRealmPair {
    pub controller: PendingSpawnedRealmChild,
    pub broker: PendingSpawnedRealmChild,
    pub bootstrap: RealmChildBootstrapEndpoints,
}

struct PendingSpawnedRealmPairGuard<'a, S>
where
    S: RealmChildSpawner + ?Sized,
{
    spawner: &'a S,
    pair: PendingSpawnedRealmPair,
    armed: bool,
}

impl<'a, S> PendingSpawnedRealmPairGuard<'a, S>
where
    S: RealmChildSpawner + ?Sized,
{
    fn new(spawner: &'a S, pair: PendingSpawnedRealmPair) -> Self {
        Self {
            spawner,
            pair,
            armed: true,
        }
    }

    fn pair(&self) -> &PendingSpawnedRealmPair {
        &self.pair
    }

    fn disarm(mut self) {
        self.armed = false;
    }
}

impl<S> Drop for PendingSpawnedRealmPairGuard<'_, S>
where
    S: RealmChildSpawner + ?Sized,
{
    fn drop(&mut self) {
        if self.armed {
            self.spawner.terminate_pair(&self.pair);
        }
    }
}

#[derive(Debug)]
pub struct PendingSpawnedRealmChild {
    pub identity: RealmChildIdentity,
    pub pid: u32,
    pub pidfd: OwnedFd,
}

#[derive(Debug, Default)]
pub struct LinuxRealmChildSpawner;

impl RealmChildSpawner for LinuxRealmChildSpawner {
    fn spawn_pair(
        &self,
        record: &RealmChildLaunchRecord,
        mut controller_fds: RealmChildDescriptorSet,
        mut broker_fds: RealmChildDescriptorSet,
        bootstrap: RealmChildBootstrapEndpoints,
    ) -> Result<PendingSpawnedRealmPair, AllocatorServiceError> {
        controller_fds.install_allocator_resource(
            REALM_CONTROLLER_AUTHORITY_RESOURCE_ID,
            realm_controller_authority_fd(record)?,
        )?;
        let guest_runtime_digest = realm_broker_guest_runtime_digest(&broker_fds)?;
        broker_fds.install_allocator_resource(
            REALM_BROKER_AUTHORITY_RESOURCE_ID,
            realm_broker_authority_fd(record, guest_runtime_digest)?,
        )?;
        let controller = spawn_one(
            &record.controller,
            Some(&record.broker),
            &record.realm_id,
            &record.controller_generation_id,
            controller_fds,
        )?;
        match spawn_one(
            &record.broker,
            Some(&record.controller),
            &record.realm_id,
            &record.controller_generation_id,
            broker_fds,
        ) {
            Ok(broker) => Ok(PendingSpawnedRealmPair {
                controller,
                broker,
                bootstrap,
            }),
            Err(error) => {
                let _ = pidfd_send_signal(controller.pidfd.as_fd(), libc::SIGKILL);
                Err(error)
            }
        }
    }
}

fn spawn_one(
    identity: &RealmChildIdentity,
    controller_peer: Option<&RealmChildIdentity>,
    realm_id: &str,
    controller_generation_id: &str,
    descriptors: RealmChildDescriptorSet,
) -> Result<PendingSpawnedRealmChild, AllocatorServiceError> {
    if descriptors.role() != identity.role {
        return Err(AllocatorServiceError::Invariant(
            "descriptor role does not match child identity",
        ));
    }
    let (cgroup_leaf, inherited_fds, mut env) = descriptors.into_launch_parts()?;
    env.extend([
        format!("D2B_REALM_ID={realm_id}"),
        format!("D2B_CONTROLLER_GENERATION={controller_generation_id}"),
        format!(
            "{}={}",
            d2b_host::guest_runtime::CONTROLLER_SESSION_GENERATION_ENV,
            d2b_host::guest_runtime::controller_session_generation(
                realm_id,
                controller_generation_id
            )
        ),
        format!("D2B_PROCESS_ID={}", identity.process_id),
        format!("D2B_CHILD_ROLE={}", identity.role.as_str()),
        format!("D2B_CGROUP_DIGEST={}", hex_digest(&identity.cgroup_digest)),
        "PATH=/run/current-system/sw/bin".to_owned(),
    ]);
    let binary = path_cstring(&identity.executable)?;
    let argv = realm_child_argv(identity.role, realm_id)?;
    let env = env
        .into_iter()
        .map(|entry| {
            CString::new(entry).map_err(|_| {
                AllocatorServiceError::InvalidRequest("launch environment contains NUL")
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    let SpawnOutcome {
        pid,
        pidfd,
        used_fork_fallback,
    } = clone3_spawn_realm_child(
        binary,
        argv,
        env,
        identity.uid,
        identity.gid,
        controller_peer.map(|peer| (peer.uid, peer.gid)),
        cgroup_leaf,
        inherited_fds,
    )
    .map_err(AllocatorServiceError::Spawn)?;
    if used_fork_fallback || pid <= 0 {
        let _ = pidfd_send_signal(pidfd.as_fd(), libc::SIGKILL);
        return Err(AllocatorServiceError::Invariant(
            "realm child did not use clone3 with a pidfd",
        ));
    }

    Ok(PendingSpawnedRealmChild {
        identity: identity.clone(),
        pid: pid as u32,
        pidfd,
    })
}

fn hex_digest(digest: &[u8; 32]) -> String {
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn realm_child_argv(
    role: RealmChildRole,
    realm_id: &str,
) -> Result<Vec<CString>, AllocatorServiceError> {
    let title = CString::new(format!(
        "{}-r-{realm_id}",
        match role {
            RealmChildRole::Controller => "d2bd",
            RealmChildRole::Broker => "d2bbr",
        }
    ))
    .map_err(|_| AllocatorServiceError::InvalidRequest("child process title contains NUL"))?;
    let mode = match role {
        RealmChildRole::Controller => "serve",
        RealmChildRole::Broker => "serve-child-realm",
    };
    let mode = CString::new(mode)
        .map_err(|_| AllocatorServiceError::InvalidRequest("child process mode contains NUL"))?;
    let argv = vec![title, mode];
    Ok(argv)
}

fn realm_broker_authority_fd(
    record: &RealmChildLaunchRecord,
    guest_runtime_digest: [u8; 32],
) -> Result<OwnedFd, AllocatorServiceError> {
    let authority = RealmBrokerChildAuthority {
        realm_id: record.realm_id.clone(),
        controller_generation: record.controller_generation_id.clone(),
        broker_process_id: record.broker.process_id.clone(),
        session_generation: d2b_host::guest_runtime::controller_session_generation(
            &record.realm_id,
            &record.controller_generation_id,
        ),
        controller_uid: record.controller.uid,
        controller_gid: record.controller.gid,
        broker_uid: record.broker.uid,
        broker_gid: record.broker.gid,
        cgroup_digest: record.broker.cgroup_digest,
        guest_runtime_digest,
    };
    let encoded = authority
        .encode()
        .map_err(|_| AllocatorServiceError::Invariant("realm broker authority encoding failed"))?;
    sealed_authority_fd("realm-broker-authority-v1", &encoded)
}

fn realm_controller_authority_fd(
    record: &RealmChildLaunchRecord,
) -> Result<OwnedFd, AllocatorServiceError> {
    let authority = RealmControllerChildAuthority {
        realm_id: record.realm_id.clone(),
        controller_generation: record.controller_generation_id.clone(),
        controller_process_id: record.controller.process_id.clone(),
        session_generation: d2b_host::guest_runtime::controller_session_generation(
            &record.realm_id,
            &record.controller_generation_id,
        ),
        controller_host_uid: record.controller.uid,
        controller_host_gid: record.controller.gid,
        broker_host_uid: record.broker.uid,
        broker_host_gid: record.broker.gid,
        broker_namespace_uid: u32::from(record.controller.uid != record.broker.uid),
        broker_namespace_gid: u32::from(record.controller.gid != record.broker.gid),
        cgroup_digest: record.controller.cgroup_digest,
    };
    let encoded = authority.encode().map_err(|_| {
        AllocatorServiceError::Invariant("realm controller authority encoding failed")
    })?;
    sealed_authority_fd("realm-controller-authority-v1", &encoded)
}

fn sealed_authority_fd(name: &str, encoded: &[u8]) -> Result<OwnedFd, AllocatorServiceError> {
    let fd = rustix::fs::memfd_create(
        name,
        rustix::fs::MemfdFlags::CLOEXEC | rustix::fs::MemfdFlags::ALLOW_SEALING,
    )
    .map_err(|error| AllocatorServiceError::Descriptor(error.into()))?;
    let mut writer = File::from(fd);
    writer
        .write_all(encoded)
        .and_then(|()| writer.flush())
        .and_then(|()| writer.seek(SeekFrom::Start(0)).map(|_| ()))
        .map_err(AllocatorServiceError::Descriptor)?;
    nix::fcntl::fcntl(
        writer.as_raw_fd(),
        nix::fcntl::FcntlArg::F_ADD_SEALS(
            nix::fcntl::SealFlag::F_SEAL_WRITE
                | nix::fcntl::SealFlag::F_SEAL_GROW
                | nix::fcntl::SealFlag::F_SEAL_SHRINK
                | nix::fcntl::SealFlag::F_SEAL_SEAL,
        ),
    )
    .map_err(|error| AllocatorServiceError::Descriptor(error.into()))?;
    let readonly = rustix::fs::open(
        format!("/proc/self/fd/{}", writer.as_raw_fd()),
        rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )
    .map_err(|error| AllocatorServiceError::Descriptor(error.into()))?;
    drop(writer);
    Ok(readonly)
}

fn realm_broker_guest_runtime_digest(
    descriptors: &RealmChildDescriptorSet,
) -> Result<[u8; 32], AllocatorServiceError> {
    let fd = descriptors
        .resource_fd(d2b_host::realm_broker_bootstrap::REALM_BROKER_GUEST_RUNTIME_RESOURCE_ID)
        .ok_or(AllocatorServiceError::Invariant(
            "realm broker guest runtime authority missing",
        ))?
        .try_clone_to_owned()
        .map_err(AllocatorServiceError::Descriptor)?;
    let file = File::from(fd);
    let metadata = file.metadata().map_err(AllocatorServiceError::Descriptor)?;
    let length = metadata.len();
    if length == 0 || length > MAX_REALM_GUEST_RUNTIME_BYTES {
        return Err(AllocatorServiceError::Invariant(
            "realm broker guest runtime authority size invalid",
        ));
    }
    let mut encoded = Zeroizing::new(vec![0_u8; length as usize]);
    file.read_exact_at(&mut encoded, 0)
        .map_err(AllocatorServiceError::Descriptor)?;
    Ok(Sha256::digest(&encoded).into())
}

fn path_cstring(path: &std::path::Path) -> Result<CString, AllocatorServiceError> {
    use std::os::unix::ffi::OsStrExt;
    CString::new(path.as_os_str().as_bytes())
        .map_err(|_| AllocatorServiceError::InvalidRequest("executable path contains NUL"))
}

pub struct AllocatorChildBrokerService<R, L, S, EL, EO, EV>
where
    EL: AllocatorLedger,
    EO: ObservedAllocatorState,
    EV: AllocatorLiveness,
{
    engine: LocalRootAllocatorEngine<EL, EO, EV>,
    resources: R,
    launch_records: L,
    spawner: S,
    credential_timeout: Duration,
}

impl<R, L, S, EL, EO, EV> AllocatorChildBrokerService<R, L, S, EL, EO, EV>
where
    R: AllocatedResourceBackend,
    L: RealmLaunchRecordResolver,
    S: RealmChildSpawner,
    EL: AllocatorLedger,
    EO: ObservedAllocatorState,
    EV: AllocatorLiveness,
{
    pub fn new(
        engine: LocalRootAllocatorEngine<EL, EO, EV>,
        resources: R,
        launch_records: L,
        spawner: S,
    ) -> Self {
        Self {
            engine,
            resources,
            launch_records,
            spawner,
            credential_timeout: Duration::from_secs(5),
        }
    }

    pub fn with_credential_timeout(mut self, timeout: Duration) -> Self {
        self.credential_timeout = timeout;
        self
    }

    pub fn engine(&self) -> &LocalRootAllocatorEngine<EL, EO, EV> {
        &self.engine
    }

    pub fn allocate(
        &mut self,
        request: &broker::AllocateRequest,
    ) -> Result<ServiceReply<broker::AllocateResponse>, AllocatorServiceError> {
        request
            .validate_wire(true)
            .map_err(|error| AllocatorServiceError::Contract(error.to_string()))?;
        let core_request = allocation_request(request)?;

        let reconciliation = self.engine.reconcile(
            core_request.operation_id.clone(),
            core_request.correlation_id.clone(),
        )?;
        if reconciliation.actions.iter().any(|action| {
            matches!(
                action.decision,
                AllocatorEngineDecision::Quarantine { .. }
                    | AllocatorEngineDecision::Preserve { .. }
                    | AllocatorEngineDecision::DenyConflict { .. }
            )
        }) {
            return Ok(ServiceReply {
                message: denied_allocate_response(
                    &request.operation_id,
                    AllocatorReasonCode::ReconcileMismatch,
                    Vec::new(),
                ),
                attachments: Vec::new(),
            });
        }

        let allocation = self.engine.allocate(core_request)?;
        match allocation.response.result {
            LeaseAllocationResult::Denied { reason, conflicts } => Ok(ServiceReply {
                message: denied_allocate_response(&request.operation_id, reason, conflicts),
                attachments: Vec::new(),
            }),
            LeaseAllocationResult::Granted { lease } => {
                let mut response = broker::AllocateResponse::new();
                response.outcome = common::Outcome::OUTCOME_SUCCEEDED.into();
                response.operation_id = request.operation_id.clone();
                response.status = broker::AllocationStatus::ALLOCATION_STATUS_GRANTED.into();
                response.lease_id = lease.lease_id.to_string();
                response.reason = broker::AllocatorReason::ALLOCATOR_REASON_UNSPECIFIED.into();

                let mut attachments = Vec::new();
                for resource in &lease.resources {
                    let materialized = self.resources.materialize(&lease, resource)?;
                    let attachment_index = materialized.as_ref().map(|_| attachments.len() as u32);
                    if let Some(fd) = materialized {
                        attachments.push(fd);
                    }
                    response
                        .resources
                        .push(granted_resource(resource, attachment_index));
                }
                response.validate_wire(false).map_err(|_| {
                    AllocatorServiceError::Invariant("allocator produced an invalid response")
                })?;
                Ok(ServiceReply {
                    message: response,
                    attachments,
                })
            }
        }
    }

    pub async fn spawn(
        &self,
        request: &broker::SpawnRealmChildrenRequest,
        attachments: Vec<OwnedFd>,
        bootstrap: RealmChildBootstrapEndpoints,
    ) -> Result<
        ServiceReply<broker::SpawnRealmChildrenResponse, VerifiedPidfdAttachments>,
        AllocatorServiceError,
    > {
        request
            .validate_wire(true)
            .map_err(|error| AllocatorServiceError::Contract(error.to_string()))?;
        if request.fds.len() != attachments.len() {
            return Err(AllocatorServiceError::InvalidRequest(
                "Spawn attachment count mismatch",
            ));
        }

        let record = self
            .launch_records
            .resolve(&request.realm_id, &request.controller_generation_id)?;
        record.validate_for_request(
            &request.realm_id,
            &request.controller_generation_id,
            &request.controller_process_id,
            &request.broker_process_id,
            &request.launch_record_digest,
        )?;

        let bindings = request
            .fds
            .iter()
            .map(fd_binding)
            .collect::<Result<Vec<_>, _>>()?;
        let table = RealmChildFdTable::correlate(bindings, attachments)?;
        let (controller_fds, broker_fds) = table.into_pair()?;
        validate_descriptor_types(&controller_fds)?;
        validate_descriptor_types(&broker_fds)?;
        bootstrap
            .validate_child_descriptors(&controller_fds, &broker_fds)
            .map_err(AllocatorServiceError::Credential)?;
        let pending = PendingSpawnedRealmPairGuard::new(
            &self.spawner,
            self.spawner
                .spawn_pair(&record, controller_fds, broker_fds, bootstrap)?,
        );
        validate_pending_spawn_correlation(pending.pair(), &record)?;
        let pair = authenticate_spawned_pair(pending.pair(), self.credential_timeout).await?;
        let mut response = broker::SpawnRealmChildrenResponse::new();
        response.outcome = common::Outcome::OUTCOME_SUCCEEDED.into();
        response.operation_id = request.operation_id.clone();
        response.launch_record_digest = request.launch_record_digest.clone();
        response.children = vec![
            spawned_child_message(&pair.controller, 0),
            spawned_child_message(&pair.broker, 1),
        ];
        response.validate_wire(false).map_err(|_| {
            AllocatorServiceError::Invariant("spawner produced an invalid response")
        })?;
        let attachments = VerifiedPidfdAttachments::from_spawned(pair)?;
        attachments.validate_response_correlation(&response)?;
        pending.disarm();
        Ok(ServiceReply {
            message: response,
            attachments,
        })
    }
}

fn validate_pending_spawn_correlation(
    pending: &PendingSpawnedRealmPair,
    record: &RealmChildLaunchRecord,
) -> Result<(), AllocatorServiceError> {
    if pending.controller.identity != record.controller
        || pending.broker.identity != record.broker
        || pending.controller.pid == pending.broker.pid
    {
        return Err(AllocatorServiceError::PidfdEvidenceCorrelation);
    }
    for child in [&pending.controller, &pending.broker] {
        let flags = rustix::io::fcntl_getfd(&child.pidfd)
            .map_err(|error| AllocatorServiceError::Descriptor(error.into()))?;
        if !flags.contains(rustix::io::FdFlags::CLOEXEC) {
            return Err(AllocatorServiceError::PidfdMissingCloexec);
        }
    }
    Ok(())
}

async fn authenticate_spawned_pair(
    pending: &PendingSpawnedRealmPair,
    timeout: Duration,
) -> Result<SpawnedRealmPair, AllocatorServiceError> {
    let controller_evidence = pending
        .bootstrap
        .controller
        .receive_pidfd_evidence(
            &pending.controller.identity,
            pending.controller.pid,
            timeout,
        )
        .await
        .map_err(AllocatorServiceError::Credential)?;
    let broker_evidence = pending
        .bootstrap
        .broker
        .receive_pidfd_evidence(&pending.broker.identity, pending.broker.pid, timeout)
        .await
        .map_err(AllocatorServiceError::Credential)?;
    Ok(SpawnedRealmPair {
        controller: SpawnedRealmChild {
            identity: pending.controller.identity.clone(),
            pid: pending.controller.pid,
            pidfd: pending
                .controller
                .pidfd
                .try_clone()
                .map_err(AllocatorServiceError::Descriptor)?,
            evidence: controller_evidence,
        },
        broker: SpawnedRealmChild {
            identity: pending.broker.identity.clone(),
            pid: pending.broker.pid,
            pidfd: pending
                .broker
                .pidfd
                .try_clone()
                .map_err(AllocatorServiceError::Descriptor)?,
            evidence: broker_evidence,
        },
    })
}

fn allocation_request(
    request: &broker::AllocateRequest,
) -> Result<LeaseAllocationRequest, AllocatorServiceError> {
    let metadata = request
        .metadata
        .as_ref()
        .ok_or(AllocatorServiceError::InvalidRequest("missing metadata"))?;
    let owner = request
        .owner
        .as_ref()
        .ok_or(AllocatorServiceError::InvalidRequest("missing lease owner"))?;
    let realm = RealmPath::new(
        owner
            .realm_path
            .split('.')
            .map(|part| RealmId::parse(part.to_owned()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| AllocatorServiceError::InvalidRequest("invalid realm path"))?,
    )
    .ok_or(AllocatorServiceError::InvalidRequest("invalid realm path"))?;
    let owner = LeaseOwner {
        realm,
        controller_generation: ControllerGenerationId::parse(
            owner.controller_generation_id.clone(),
        )
        .map_err(|_| AllocatorServiceError::InvalidRequest("invalid controller generation"))?,
        node: owner
            .node_id
            .as_ref()
            .map(|node| NodeId::parse(node.clone()))
            .transpose()
            .map_err(|_| AllocatorServiceError::InvalidRequest("invalid owner node"))?,
    };
    let resources = request
        .resources
        .iter()
        .map(|resource| {
            let order = resource.acquisition_order.as_ref().ok_or(
                AllocatorServiceError::InvalidRequest("missing acquisition order"),
            )?;
            Ok(LeaseResourceRequest {
                resource_id: HostResourceId::parse(resource.resource_id.clone()).map_err(|_| {
                    AllocatorServiceError::InvalidRequest("invalid host resource id")
                })?,
                kind: core_resource_kind(resource.kind.value())?,
                share: core_share_mode(resource.share.value())?,
                acquisition_order: CoreAcquisitionOrder {
                    phase: order.phase as u16,
                    ordinal: order.ordinal as u16,
                },
            })
        })
        .collect::<Result<Vec<_>, AllocatorServiceError>>()?;
    Ok(LeaseAllocationRequest {
        operation_id: OperationId::parse(request.operation_id.clone())
            .map_err(|_| AllocatorServiceError::InvalidRequest("invalid operation id"))?,
        correlation_id: CorrelationId::parse(metadata.correlation_id.clone())
            .map_err(|_| AllocatorServiceError::InvalidRequest("invalid correlation id"))?,
        idempotency_key: IdempotencyKey::parse(hex_bytes(&metadata.idempotency_key))
            .map_err(|_| AllocatorServiceError::InvalidRequest("invalid idempotency key"))?,
        owner,
        resources,
        trace: None,
    })
}

fn hex_bytes(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

fn core_resource_kind(value: i32) -> Result<CoreResourceKind, AllocatorServiceError> {
    Ok(match value {
        1 => CoreResourceKind::Bridge,
        2 => CoreResourceKind::Tap,
        3 => CoreResourceKind::VethPair,
        4 => CoreResourceKind::NftablesTable,
        5 => CoreResourceKind::NftablesPartition,
        6 => CoreResourceKind::CgroupSubtree,
        7 => CoreResourceKind::HostFilePartition,
        8 => CoreResourceKind::NamespaceBoundary,
        _ => {
            return Err(AllocatorServiceError::InvalidRequest(
                "invalid resource kind",
            ));
        }
    })
}

fn core_share_mode(value: i32) -> Result<CoreShareMode, AllocatorServiceError> {
    match value {
        1 => Ok(CoreShareMode::Exclusive),
        2 => Ok(CoreShareMode::SharedPartition),
        _ => Err(AllocatorServiceError::InvalidRequest(
            "invalid resource share mode",
        )),
    }
}

fn wire_resource_kind(kind: CoreResourceKind) -> broker::HostResourceKind {
    match kind {
        CoreResourceKind::Bridge => broker::HostResourceKind::HOST_RESOURCE_KIND_BRIDGE,
        CoreResourceKind::Tap => broker::HostResourceKind::HOST_RESOURCE_KIND_TAP,
        CoreResourceKind::VethPair => broker::HostResourceKind::HOST_RESOURCE_KIND_VETH_PAIR,
        CoreResourceKind::NftablesTable => {
            broker::HostResourceKind::HOST_RESOURCE_KIND_NFTABLES_TABLE
        }
        CoreResourceKind::NftablesPartition => {
            broker::HostResourceKind::HOST_RESOURCE_KIND_NFTABLES_PARTITION
        }
        CoreResourceKind::CgroupSubtree => {
            broker::HostResourceKind::HOST_RESOURCE_KIND_CGROUP_SUBTREE
        }
        CoreResourceKind::HostFilePartition => {
            broker::HostResourceKind::HOST_RESOURCE_KIND_HOST_FILE_PARTITION
        }
        CoreResourceKind::NamespaceBoundary => {
            broker::HostResourceKind::HOST_RESOURCE_KIND_NAMESPACE_BOUNDARY
        }
    }
}

fn wire_share_mode(share: CoreShareMode) -> broker::ResourceShareMode {
    match share {
        CoreShareMode::Exclusive => broker::ResourceShareMode::RESOURCE_SHARE_MODE_EXCLUSIVE,
        CoreShareMode::SharedPartition => {
            broker::ResourceShareMode::RESOURCE_SHARE_MODE_SHARED_PARTITION
        }
    }
}

fn granted_resource(
    resource: &CoreGrantedResource,
    attachment_index: Option<u32>,
) -> broker::GrantedHostResource {
    let (delegation, id) = if attachment_index.is_some() {
        (
            broker::ResourceDelegationKind::RESOURCE_DELEGATION_KIND_FILE_DESCRIPTOR,
            resource.resource_id.to_string(),
        )
    } else {
        match &resource.delegation {
            ResourceDelegation::OpaqueName { id } => (
                broker::ResourceDelegationKind::RESOURCE_DELEGATION_KIND_OPAQUE_NAME,
                id.to_string(),
            ),
            ResourceDelegation::FileDescriptor { id } => (
                broker::ResourceDelegationKind::RESOURCE_DELEGATION_KIND_FILE_DESCRIPTOR,
                id.to_string(),
            ),
            ResourceDelegation::PartitionId { id } => (
                broker::ResourceDelegationKind::RESOURCE_DELEGATION_KIND_PARTITION_ID,
                id.to_string(),
            ),
            ResourceDelegation::NamespaceHandle { id } => (
                broker::ResourceDelegationKind::RESOURCE_DELEGATION_KIND_NAMESPACE_HANDLE,
                id.to_string(),
            ),
        }
    };
    let mut result = broker::GrantedHostResource::new();
    result.resource_id = resource.resource_id.to_string();
    result.kind = wire_resource_kind(resource.kind).into();
    result.share = wire_share_mode(resource.share).into();
    result.delegation = delegation.into();
    result.delegation_id = id;
    let mut order = broker::ResourceAcquisitionOrder::new();
    order.phase = u32::from(resource.acquisition_order.phase);
    order.ordinal = u32::from(resource.acquisition_order.ordinal);
    result.acquisition_order = Some(order).into();
    result.attachment_index = attachment_index;
    result
}

fn denied_allocate_response(
    operation_id: &str,
    reason: AllocatorReasonCode,
    conflicts: Vec<d2b_realm_core::allocator::AllocatorConflict>,
) -> broker::AllocateResponse {
    let mut response = broker::AllocateResponse::new();
    response.outcome = common::Outcome::OUTCOME_DENIED.into();
    response.operation_id = operation_id.to_owned();
    response.status = broker::AllocationStatus::ALLOCATION_STATUS_DENIED.into();
    response.reason = wire_reason(reason).into();
    response.conflicts = conflicts
        .into_iter()
        .map(|conflict| {
            let mut wire = broker::AllocatorConflict::new();
            wire.resource_id = conflict.resource_id.to_string();
            wire.kind = wire_resource_kind(conflict.kind).into();
            wire.reason = wire_reason(conflict.reason).into();
            wire.existing_lease_id = conflict.existing_lease.map(|id| id.to_string());
            wire
        })
        .collect();
    response
}

fn wire_reason(reason: AllocatorReasonCode) -> broker::AllocatorReason {
    use broker::AllocatorReason as W;
    match reason {
        AllocatorReasonCode::ResourceConflict => W::ALLOCATOR_REASON_RESOURCE_CONFLICT,
        AllocatorReasonCode::OwnershipConflict => W::ALLOCATOR_REASON_OWNERSHIP_CONFLICT,
        AllocatorReasonCode::AcquisitionOrderViolation => {
            W::ALLOCATOR_REASON_ACQUISITION_ORDER_VIOLATION
        }
        AllocatorReasonCode::InvalidRequest => W::ALLOCATOR_REASON_INVALID_REQUEST,
        AllocatorReasonCode::CapacityExhausted => W::ALLOCATOR_REASON_CAPACITY_EXHAUSTED,
        AllocatorReasonCode::DriftDetected => W::ALLOCATOR_REASON_DRIFT_DETECTED,
        AllocatorReasonCode::ReconcileMismatch => W::ALLOCATOR_REASON_RECONCILE_MISMATCH,
        AllocatorReasonCode::OwnerNotLive => W::ALLOCATOR_REASON_OWNER_NOT_LIVE,
        AllocatorReasonCode::PolicyDenied => W::ALLOCATOR_REASON_POLICY_DENIED,
        AllocatorReasonCode::UnsupportedKind => W::ALLOCATOR_REASON_UNSUPPORTED_KIND,
        AllocatorReasonCode::StorageContractViolation => {
            W::ALLOCATOR_REASON_STORAGE_CONTRACT_VIOLATION
        }
        AllocatorReasonCode::KernelStateUnknown => W::ALLOCATOR_REASON_KERNEL_STATE_UNKNOWN,
    }
}

fn fd_binding(fd: &broker::RealmChildFd) -> Result<RealmChildFdBinding, AllocatorServiceError> {
    let role = match fd.role.value() {
        1 => RealmChildRole::Controller,
        2 => RealmChildRole::Broker,
        _ => return Err(AllocatorServiceError::InvalidRequest("invalid child role")),
    };
    let kind = match fd.kind.value() {
        1 => RealmChildFdKind::PublicListener,
        2 => RealmChildFdKind::BrokerListener,
        3 => RealmChildFdKind::UserNamespace,
        4 => RealmChildFdKind::MountNamespace,
        5 => RealmChildFdKind::NetworkNamespace,
        6 => RealmChildFdKind::IpcNamespace,
        7 => RealmChildFdKind::PidNamespace,
        8 => RealmChildFdKind::CgroupNamespace,
        9 => RealmChildFdKind::CgroupLeaf,
        10 => RealmChildFdKind::StateRoot,
        11 => RealmChildFdKind::AuditRoot,
        12 => RealmChildFdKind::Resource,
        13 => RealmChildFdKind::Lease,
        14 => RealmChildFdKind::BootstrapSession,
        _ => {
            return Err(AllocatorServiceError::InvalidRequest(
                "invalid child fd kind",
            ));
        }
    };
    Ok(RealmChildFdBinding {
        role,
        kind,
        attachment_index: fd.attachment_index,
        resource_id: fd.resource_id.clone(),
    })
}

fn validate_descriptor_types(
    descriptors: &RealmChildDescriptorSet,
) -> Result<(), AllocatorServiceError> {
    for entry in descriptors.entries() {
        crate::sys::validate_realm_child_fd(entry.fd.as_fd(), entry.binding.kind)
            .map_err(AllocatorServiceError::Descriptor)?;
    }
    Ok(())
}

fn spawned_child_message(
    child: &SpawnedRealmChild,
    pidfd_attachment_index: u32,
) -> broker::SpawnedRealmChild {
    let mut result = broker::SpawnedRealmChild::new();
    result.role = match child.identity.role {
        RealmChildRole::Controller => broker::RealmChildRole::REALM_CHILD_ROLE_CONTROLLER,
        RealmChildRole::Broker => broker::RealmChildRole::REALM_CHILD_ROLE_BROKER,
    }
    .into();
    result.process_id = child.identity.process_id.clone();
    result.pidfd_attachment_index = pidfd_attachment_index;
    result.executable_digest = child.identity.executable_digest.to_vec();
    result.pid = child.pid;
    result
}

#[derive(Debug)]
pub enum AllocatorServiceError {
    InvalidRequest(&'static str),
    Invariant(&'static str),
    LaunchRecord(String),
    Contract(String),
    Descriptor(std::io::Error),
    Spawn(std::io::Error),
    Credential(RealmChildCredentialError),
    AllocatorTransaction,
    PidfdEvidenceCorrelation,
    PidfdMissingCloexec,
    PidfdPolicy(UnixSessionError),
    Plan(RealmChildPlanError),
}

impl fmt::Display for AllocatorServiceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRequest(message) | Self::Invariant(message) => {
                formatter.write_str(message)
            }
            Self::LaunchRecord(message) => write!(formatter, "launch record: {message}"),
            Self::Contract(message) => write!(formatter, "service contract: {message}"),
            Self::Descriptor(error) => write!(formatter, "invalid delegated descriptor: {error}"),
            Self::Spawn(error) => write!(formatter, "realm child spawn failed: {error}"),
            Self::Credential(error) => {
                write!(formatter, "realm child credential evidence failed: {error}")
            }
            Self::AllocatorTransaction => formatter.write_str("allocator transaction failed"),
            Self::PidfdEvidenceCorrelation => {
                formatter.write_str("spawned pidfd evidence correlation failed")
            }
            Self::PidfdMissingCloexec => formatter.write_str("spawned pidfd is missing CLOEXEC"),
            Self::PidfdPolicy(error) => {
                write!(
                    formatter,
                    "spawned pidfd policy construction failed: {error}"
                )
            }
            Self::Plan(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for AllocatorServiceError {}

impl From<AllocatorEngineError> for AllocatorServiceError {
    fn from(_: AllocatorEngineError) -> Self {
        Self::AllocatorTransaction
    }
}

impl From<RealmChildPlanError> for AllocatorServiceError {
    fn from(value: RealmChildPlanError) -> Self {
        Self::Plan(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocator_uses_closed_role_titles_and_explicit_child_broker_mode() {
        let controller =
            realm_child_argv(RealmChildRole::Controller, "aaaaaaaaaaaaaaaaaaaa").unwrap();
        assert_eq!(controller.len(), 2);
        assert_eq!(
            controller[0].to_str().unwrap(),
            "d2bd-r-aaaaaaaaaaaaaaaaaaaa"
        );
        assert_eq!(controller[1].to_str().unwrap(), "serve");

        let broker = realm_child_argv(RealmChildRole::Broker, "aaaaaaaaaaaaaaaaaaaa").unwrap();
        assert_eq!(broker.len(), 2);
        assert_eq!(broker[0].to_str().unwrap(), "d2bbr-r-aaaaaaaaaaaaaaaaaaaa");
        assert_eq!(broker[1].to_str().unwrap(), "serve-child-realm");
    }
}
