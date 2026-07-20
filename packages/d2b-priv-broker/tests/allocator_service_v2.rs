use std::fs::File;
use std::io::{IoSliceMut, Seek, SeekFrom, Write};
use std::os::fd::{AsRawFd, BorrowedFd, OwnedFd};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

use d2b_contracts::v2_services::{broker, common};
use d2b_host::realm_children::{
    PidfdEvidence, PidfdIdentityVerifier, RealmChildBootstrapEndpoint,
    RealmChildBootstrapEndpoints, RealmChildDescriptorSet, RealmChildIdentity,
    RealmChildLaunchRecord, RealmChildRole, UnixSessionError,
};
use d2b_priv_broker::allocator_service::{
    AllocatedResourceBackend, AllocatorChildBrokerService, AllocatorServiceError,
    PendingSpawnedRealmChild, PendingSpawnedRealmPair, RealmChildSpawner,
    RealmLaunchRecordResolver,
};
use d2b_priv_broker::live_handlers::prebind_realm_listeners;
use d2b_realm_core::allocator::{
    AllocatorLease, AllocatorLeaseState, GrantedHostResource, HostResourceKind, LeaseOwner,
};
use d2b_realm_core::allocator_engine::{
    AllocatorEngineError, AllocatorLedger, AllocatorLedgerCommit, AllocatorLedgerCommitResult,
    AllocatorLedgerGeneration, AllocatorLedgerSnapshot, FakeAllocatorLedger, FakeAllocatorLiveness,
    FakeObservedAllocatorState, LocalRootAllocatorEngine,
};
use d2b_realm_core::ids::{AllocatorLeaseId, ControllerGenerationId, RealmId};
use d2b_realm_core::realm::RealmPath;
use nix::sys::socket::{
    AddressFamily, Backlog, MsgFlags, SockFlag, SockType, UnixAddr, bind, connect, listen, recv,
    send, setsockopt, socket, socketpair, sockopt::PassCred,
};
use nix::unistd::getpid;
use rustix::net::{
    RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags, SocketFlags as RustixSocketFlags,
    accept_with, recvmsg,
};

const RUNTIME_REALM_ID: &str = "aaaaaaaaaaaaaaaaaaaa";

fn metadata() -> common::RequestMetadata {
    let mut value = common::RequestMetadata::new();
    value.request_id = vec![1; 16];
    value.correlation_id = "correlation-1".into();
    value.idempotency_key = vec![2; 16];
    value.issued_at_unix_ms = 100;
    value.expires_at_unix_ms = 200;
    value.session_generation = 1;
    value
}

fn scope() -> common::IdentityScope {
    let mut value = common::IdentityScope::new();
    value.realm_id = RUNTIME_REALM_ID.into();
    value
}

fn owner() -> LeaseOwner {
    LeaseOwner {
        realm: RealmPath::new(vec![RealmId::parse("work").unwrap()]).unwrap(),
        controller_generation: ControllerGenerationId::parse("generation-1").unwrap(),
        node: None,
    }
}

#[derive(Default)]
struct TestResources;

impl AllocatedResourceBackend for TestResources {
    fn materialize(
        &mut self,
        _lease: &d2b_realm_core::allocator::AllocatorLease,
        resource: &GrantedHostResource,
    ) -> Result<Option<OwnedFd>, AllocatorServiceError> {
        if resource.kind == HostResourceKind::NamespaceBoundary {
            Ok(Some(File::open("/dev/null").unwrap().into()))
        } else {
            Ok(None)
        }
    }
}

#[derive(Clone, Copy, Default)]
enum LedgerFailurePoint {
    #[default]
    None,
    ReconcileRead,
    ReconcileIntegrity,
    AllocationRead,
    Commit,
}

struct FailingLedger {
    inner: FakeAllocatorLedger,
    failure: LedgerFailurePoint,
    loads: Arc<AtomicUsize>,
    commits: Arc<AtomicUsize>,
}

impl FailingLedger {
    fn new(failure: LedgerFailurePoint) -> Self {
        Self {
            inner: FakeAllocatorLedger::default(),
            failure,
            loads: Arc::new(AtomicUsize::new(0)),
            commits: Arc::new(AtomicUsize::new(0)),
        }
    }
}

impl AllocatorLedger for FailingLedger {
    fn load(&self) -> Result<AllocatorLedgerSnapshot, AllocatorEngineError> {
        let call = self.loads.fetch_add(1, Ordering::SeqCst) + 1;
        match (self.failure, call) {
            (LedgerFailurePoint::ReconcileRead, 1) => Err(AllocatorEngineError::LedgerIo),
            (LedgerFailurePoint::ReconcileIntegrity, 1) => {
                let lease = AllocatorLease {
                    lease_id: AllocatorLeaseId::parse("lease-duplicate").unwrap(),
                    owner: owner(),
                    state: AllocatorLeaseState::Granted,
                    resources: Vec::new(),
                };
                Ok(AllocatorLedgerSnapshot::new(
                    AllocatorLedgerGeneration::default(),
                    vec![lease.clone(), lease],
                    Vec::new(),
                ))
            }
            (LedgerFailurePoint::AllocationRead, 2) => {
                Err(AllocatorEngineError::LedgerLockUnavailable)
            }
            _ => self.inner.load(),
        }
    }

    fn commit_allocation(
        &mut self,
        commit: AllocatorLedgerCommit,
    ) -> Result<AllocatorLedgerCommitResult, AllocatorEngineError> {
        self.commits.fetch_add(1, Ordering::SeqCst);
        if matches!(self.failure, LedgerFailurePoint::Commit) {
            Err(AllocatorEngineError::LedgerGenerationConflict)
        } else {
            self.inner.commit_allocation(commit)
        }
    }
}

struct CountingResources {
    calls: Arc<AtomicUsize>,
}

impl AllocatedResourceBackend for CountingResources {
    fn materialize(
        &mut self,
        _lease: &d2b_realm_core::allocator::AllocatorLease,
        resource: &GrantedHostResource,
    ) -> Result<Option<OwnedFd>, AllocatorServiceError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if resource.kind == HostResourceKind::NamespaceBoundary {
            Ok(Some(File::open("/dev/null").unwrap().into()))
        } else {
            Ok(None)
        }
    }
}

#[derive(Clone)]
struct TestLaunches(RealmChildLaunchRecord);

impl RealmLaunchRecordResolver for TestLaunches {
    fn resolve(
        &self,
        _realm_id: &str,
        _controller_generation_id: &str,
    ) -> Result<RealmChildLaunchRecord, AllocatorServiceError> {
        Ok(self.0.clone())
    }
}

#[derive(Clone, Default)]
struct TestSpawner {
    terminated: Arc<AtomicUsize>,
    spawned: Arc<AtomicUsize>,
    pids: Option<(u32, u32)>,
    corruption: Option<SpawnCorruption>,
    spawn_thread: Option<Arc<Mutex<Option<std::thread::ThreadId>>>>,
    spawn_delay: Option<Duration>,
}

#[derive(Clone, Copy)]
enum SpawnCorruption {
    SwappedIdentities,
    DuplicateControllerIdentity,
    MissingCloexec,
}

impl RealmChildSpawner for TestSpawner {
    fn spawn_pair(
        &self,
        record: &RealmChildLaunchRecord,
        controller_fds: RealmChildDescriptorSet,
        broker_fds: RealmChildDescriptorSet,
        bootstrap: RealmChildBootstrapEndpoints,
    ) -> Result<PendingSpawnedRealmPair, AllocatorServiceError> {
        if let Some(thread) = &self.spawn_thread {
            *thread.lock().unwrap() = Some(std::thread::current().id());
        }
        self.spawned.fetch_add(1, Ordering::SeqCst);
        if let Some(delay) = self.spawn_delay {
            std::thread::sleep(delay);
        }
        assert_eq!(controller_fds.role(), RealmChildRole::Controller);
        assert_eq!(broker_fds.role(), RealmChildRole::Broker);
        let (controller_pid, broker_pid) = self.pids.unwrap_or_else(|| {
            let pid = getpid().as_raw() as u32;
            (pid, pid + 1)
        });
        let (mut controller_identity, mut broker_identity) =
            (record.controller.clone(), record.broker.clone());
        match self.corruption {
            Some(SpawnCorruption::SwappedIdentities) => {
                std::mem::swap(&mut controller_identity, &mut broker_identity);
            }
            Some(SpawnCorruption::DuplicateControllerIdentity) => {
                broker_identity = controller_identity.clone();
            }
            Some(SpawnCorruption::MissingCloexec) | None => {}
        }
        let controller_pidfd = rustix::process::pidfd_open(
            rustix::process::Pid::from_raw(controller_pid as i32).unwrap(),
            rustix::process::PidfdFlags::empty(),
        )
        .unwrap();
        let broker_pidfd = rustix::process::pidfd_open(
            rustix::process::Pid::from_raw(broker_pid as i32).unwrap(),
            rustix::process::PidfdFlags::empty(),
        )
        .unwrap();
        if matches!(self.corruption, Some(SpawnCorruption::MissingCloexec)) {
            use nix::fcntl::{FcntlArg, FdFlag, fcntl};
            fcntl(
                controller_pidfd.as_raw_fd(),
                FcntlArg::F_SETFD(FdFlag::empty()),
            )
            .unwrap();
        }
        Ok(PendingSpawnedRealmPair {
            controller: PendingSpawnedRealmChild {
                identity: controller_identity,
                pid: controller_pid,
                pidfd: controller_pidfd,
            },
            broker: PendingSpawnedRealmChild {
                identity: broker_identity,
                pid: broker_pid,
                pidfd: broker_pidfd,
            },
            bootstrap,
        })
    }

    fn terminate_pair(&self, _pair: &PendingSpawnedRealmPair) {
        self.terminated.fetch_add(1, Ordering::SeqCst);
    }
}

fn record() -> RealmChildLaunchRecord {
    RealmChildLaunchRecord {
        realm_id: RUNTIME_REALM_ID.into(),
        controller_generation_id: "generation-1".into(),
        launch_record_digest: [7; 32],
        controller: RealmChildIdentity {
            role: RealmChildRole::Controller,
            process_id: "controller-1".into(),
            executable: PathBuf::from("/bin/true"),
            executable_digest: [8; 32],
            cgroup_digest: [10; 32],
            uid: 1001,
            gid: 1001,
        },
        broker: RealmChildIdentity {
            role: RealmChildRole::Broker,
            process_id: "broker-1".into(),
            executable: PathBuf::from("/bin/true"),
            executable_digest: [9; 32],
            cgroup_digest: [11; 32],
            uid: 1002,
            gid: 1002,
        },
    }
}

type TestService = AllocatorChildBrokerService<
    TestResources,
    TestLaunches,
    TestSpawner,
    FakeAllocatorLedger,
    FakeObservedAllocatorState,
    FakeAllocatorLiveness,
>;

fn service() -> TestService {
    service_with_spawner(TestSpawner::default())
}

fn service_with_spawner(spawner: TestSpawner) -> TestService {
    let owner = owner();
    AllocatorChildBrokerService::new(
        LocalRootAllocatorEngine::new(
            owner.clone(),
            FakeAllocatorLedger::default(),
            FakeObservedAllocatorState::default(),
            FakeAllocatorLiveness::new(vec![owner]),
        ),
        TestResources,
        TestLaunches(record()),
        spawner,
    )
}

type TransactionTestService = AllocatorChildBrokerService<
    CountingResources,
    TestLaunches,
    TestSpawner,
    FailingLedger,
    FakeObservedAllocatorState,
    FakeAllocatorLiveness,
>;

fn transaction_service(failure: LedgerFailurePoint) -> (TransactionTestService, Arc<AtomicUsize>) {
    let owner = owner();
    let resource_calls = Arc::new(AtomicUsize::new(0));
    (
        AllocatorChildBrokerService::new(
            LocalRootAllocatorEngine::new(
                owner.clone(),
                FailingLedger::new(failure),
                FakeObservedAllocatorState::default(),
                FakeAllocatorLiveness::new(vec![owner]),
            ),
            CountingResources {
                calls: resource_calls.clone(),
            },
            TestLaunches(record()),
            TestSpawner::default(),
        ),
        resource_calls,
    )
}

static CREDENTIAL_SOCKET_COUNTER: AtomicUsize = AtomicUsize::new(0);

fn socket_tempdir() -> tempfile::TempDir {
    let Some(root) = std::env::var_os("D2B_VALIDATION_SOCKET_DIR").map(PathBuf::from) else {
        return tempfile::tempdir().expect("create allocator socket tempdir");
    };
    std::fs::create_dir_all(&root).expect("create allocator test socket root");
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))
        .expect("harden allocator test socket root");
    tempfile::tempdir_in(root).expect("create allocator socket tempdir")
}

fn recv_child_endpoint(control: &OwnedFd) -> OwnedFd {
    let mut payload = [0_u8; 32];
    let mut control_bytes = vec![0_u8; rustix::cmsg_space!(ScmRights(1))];
    let mut ancillary = RecvAncillaryBuffer::new(&mut control_bytes);
    let mut iov = [IoSliceMut::new(&mut payload)];
    recvmsg(control, &mut iov, &mut ancillary, RecvFlags::CMSG_CLOEXEC).unwrap();
    ancillary
        .drain()
        .find_map(|message| match message {
            RecvAncillaryMessage::ScmRights(mut files) => files.next(),
            _ => None,
        })
        .expect("credential helper transferred its child endpoint")
}

fn bootstrap_endpoint_process(
    mode: &str,
) -> (
    RealmChildBootstrapEndpoint,
    OwnedFd,
    Child,
    tempfile::TempDir,
) {
    let ordinal = CREDENTIAL_SOCKET_COUNTER.fetch_add(1, Ordering::Relaxed);
    let root = socket_tempdir();
    let path = root.path().join(format!("credential-{ordinal}.sock"));
    let listener = socket(
        AddressFamily::Unix,
        SockType::SeqPacket,
        SockFlag::SOCK_CLOEXEC,
        None,
    )
    .unwrap();
    bind(
        listener.as_raw_fd(),
        &UnixAddr::new(path.as_path()).unwrap(),
    )
    .unwrap();
    listen(&listener, Backlog::new(1).unwrap()).unwrap();
    let (control_parent, control_child) = socketpair(
        AddressFamily::Unix,
        SockType::SeqPacket,
        None,
        SockFlag::SOCK_CLOEXEC,
    )
    .unwrap();
    use nix::fcntl::{FcntlArg, FdFlag, fcntl};
    fcntl(
        control_child.as_raw_fd(),
        FcntlArg::F_SETFD(FdFlag::empty()),
    )
    .unwrap();
    let child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "credential_peer_helper", "--nocapture"])
        .env("D2B_CREDENTIAL_HELPER", mode)
        .env("D2B_CREDENTIAL_SOCKET", &path)
        .env(
            "D2B_CREDENTIAL_CONTROL_FD",
            control_child.as_raw_fd().to_string(),
        )
        .spawn()
        .unwrap();
    drop(control_child);
    let accepted = accept_with(
        &listener,
        RustixSocketFlags::CLOEXEC | RustixSocketFlags::NONBLOCK,
    )
    .unwrap();
    let child_endpoint = recv_child_endpoint(&control_parent);
    setsockopt(&accepted, PassCred, &true).unwrap();
    send(accepted.as_raw_fd(), b"go", MsgFlags::MSG_NOSIGNAL).unwrap();
    (
        RealmChildBootstrapEndpoint::from_parent_prearmed(accepted, &child_endpoint).unwrap(),
        child_endpoint,
        child,
        root,
    )
}

struct TestBootstrapProcesses {
    endpoints: RealmChildBootstrapEndpoints,
    sessions: Vec<OwnedFd>,
    helpers: Vec<Child>,
    roots: Vec<tempfile::TempDir>,
    pids: (u32, u32),
}

fn bootstrap_processes(controller_mode: &str, broker_mode: &str) -> TestBootstrapProcesses {
    let (controller, controller_endpoint, controller_child, controller_root) =
        bootstrap_endpoint_process(controller_mode);
    let (broker, broker_endpoint, broker_child, broker_root) =
        bootstrap_endpoint_process(broker_mode);
    let pids = (controller_child.id(), broker_child.id());
    TestBootstrapProcesses {
        endpoints: RealmChildBootstrapEndpoints { controller, broker },
        sessions: vec![controller_endpoint, broker_endpoint],
        helpers: vec![controller_child, broker_child],
        roots: vec![controller_root, broker_root],
        pids,
    }
}

fn cleanup_helpers(mut helpers: Vec<Child>, roots: Vec<tempfile::TempDir>) {
    for helper in &mut helpers {
        let _ = helper.kill();
        let _ = helper.wait();
    }
    for root in roots {
        root.close().expect("remove allocator socket tempdir");
    }
}

struct ExpectedPidVerifier(u32);

fn pidfd_process_id(raw_fd: i32) -> Option<u32> {
    std::fs::read_to_string(format!("/proc/self/fdinfo/{raw_fd}"))
        .ok()?
        .lines()
        .find_map(|line| line.strip_prefix("Pid:"))
        .and_then(|pid| pid.trim().parse::<u32>().ok())
}

impl PidfdIdentityVerifier for ExpectedPidVerifier {
    fn verify(
        &self,
        pidfd: BorrowedFd<'_>,
        evidence: &PidfdEvidence,
    ) -> Result<(), UnixSessionError> {
        let expected = rustix::process::Pid::from_raw(self.0 as i32)
            .ok_or(UnixSessionError::PidfdIdentityMismatch)?;
        let observed = pidfd_process_id(pidfd.as_raw_fd())
            .ok_or(UnixSessionError::PidfdEvidenceUnavailable)?;
        if evidence.expected_pid() == expected && observed == self.0 {
            Ok(())
        } else {
            Err(UnixSessionError::PidfdIdentityMismatch)
        }
    }
}

#[test]
fn credential_peer_helper() {
    let Ok(mode) = std::env::var("D2B_CREDENTIAL_HELPER") else {
        return;
    };
    if mode == "send-inherited" {
        let fd = std::env::var("D2B_CREDENTIAL_FD")
            .unwrap()
            .parse::<i32>()
            .unwrap();
        let _ = send(fd, b"peer-ready", MsgFlags::MSG_NOSIGNAL);
        return;
    }
    let path = PathBuf::from(std::env::var_os("D2B_CREDENTIAL_SOCKET").unwrap());
    let socket = socket(
        AddressFamily::Unix,
        SockType::SeqPacket,
        SockFlag::SOCK_CLOEXEC,
        None,
    )
    .unwrap();
    connect(socket.as_raw_fd(), &UnixAddr::new(path.as_path()).unwrap()).unwrap();
    let control_fd = std::env::var("D2B_CREDENTIAL_CONTROL_FD")
        .unwrap()
        .parse::<i32>()
        .unwrap();
    d2b_priv_broker::fd_passing::send_fds(control_fd, b"child-endpoint", &[socket.as_raw_fd()])
        .unwrap();
    let mut go = [0_u8; 2];
    recv(socket.as_raw_fd(), &mut go, MsgFlags::empty()).unwrap();
    match mode.as_str() {
        "valid" => {
            let _ = send(socket.as_raw_fd(), b"peer-ready", MsgFlags::MSG_NOSIGNAL);
        }
        "truncated" => {
            let _ = send(
                socket.as_raw_fd(),
                &vec![
                    0x41;
                    d2b_host::realm_children::REALM_CHILD_BOOTSTRAP_MAX_PACKET_BYTES as usize + 1
                ],
                MsgFlags::MSG_NOSIGNAL,
            );
        }
        "timeout" => std::thread::sleep(Duration::from_secs(30)),
        "mismatch" => {
            use nix::fcntl::{FcntlArg, FdFlag, fcntl};
            fcntl(socket.as_raw_fd(), FcntlArg::F_SETFD(FdFlag::empty())).unwrap();
            Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "credential_peer_helper", "--nocapture"])
                .env("D2B_CREDENTIAL_HELPER", "send-inherited")
                .env("D2B_CREDENTIAL_FD", socket.as_raw_fd().to_string())
                .status()
                .unwrap();
        }
        other => panic!("unexpected credential helper mode: {other}"),
    }
}

fn allocate_request(operation_id: &str) -> broker::AllocateRequest {
    let mut request = broker::AllocateRequest::new();
    request.metadata = Some(metadata()).into();
    request.scope = Some(scope()).into();
    request.operation_id = operation_id.into();
    let mut wire_owner = broker::LeaseOwner::new();
    wire_owner.realm_path = "work".into();
    wire_owner.controller_generation_id = "generation-1".into();
    request.owner = Some(wire_owner).into();
    request.request_digest = vec![3; 32];

    for (id, kind, ordinal) in [
        (
            "bridge-1",
            broker::HostResourceKind::HOST_RESOURCE_KIND_BRIDGE,
            1,
        ),
        (
            "namespace-1",
            broker::HostResourceKind::HOST_RESOURCE_KIND_NAMESPACE_BOUNDARY,
            0,
        ),
    ] {
        let mut resource = broker::LeaseResourceRequest::new();
        resource.resource_id = id.into();
        resource.kind = kind.into();
        resource.share = broker::ResourceShareMode::RESOURCE_SHARE_MODE_EXCLUSIVE.into();
        let mut order = broker::ResourceAcquisitionOrder::new();
        order.phase = 1;
        order.ordinal = ordinal;
        resource.acquisition_order = Some(order).into();
        request.resources.push(resource);
    }
    request
}

#[test]
fn allocate_uses_engine_order_and_exact_fd_indexes() {
    let request = allocate_request("allocate-1");
    let reply = service().allocate(&request).unwrap();
    assert_eq!(reply.attachments.len(), 1);
    assert_eq!(reply.message.resources[0].resource_id, "namespace-1");
    assert_eq!(reply.message.resources[0].attachment_index, Some(0));
    assert_eq!(reply.message.resources[1].resource_id, "bridge-1");
    assert_eq!(reply.message.resources[1].attachment_index, None);
}

fn assert_allocator_transaction_failure(
    failure: LedgerFailurePoint,
    expected_loads: usize,
    expected_commits: usize,
) {
    let (mut service, resource_calls) = transaction_service(failure);
    let error = service
        .allocate(&allocate_request("allocate-failure"))
        .unwrap_err();
    assert!(matches!(error, AllocatorServiceError::AllocatorTransaction));
    assert_eq!(error.to_string(), "allocator transaction failed");
    assert_eq!(format!("{error:?}"), "AllocatorTransaction");
    assert_eq!(resource_calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        service.engine().ledger().loads.load(Ordering::SeqCst),
        expected_loads
    );
    assert_eq!(
        service.engine().ledger().commits.load(Ordering::SeqCst),
        expected_commits
    );
}

#[test]
fn allocate_propagates_reconcile_read_failure_without_reply_or_attachments() {
    assert_allocator_transaction_failure(LedgerFailurePoint::ReconcileRead, 1, 0);
}

#[test]
fn allocate_propagates_reconcile_integrity_failure_without_reply_or_attachments() {
    assert_allocator_transaction_failure(LedgerFailurePoint::ReconcileIntegrity, 1, 0);
}

#[test]
fn allocate_propagates_allocation_read_failure_without_reply_or_attachments() {
    assert_allocator_transaction_failure(LedgerFailurePoint::AllocationRead, 2, 0);
}

#[test]
fn allocate_propagates_commit_failure_without_reply_or_attachments() {
    assert_allocator_transaction_failure(LedgerFailurePoint::Commit, 2, 1);
}

#[test]
fn allocate_idempotent_replay_reuses_committed_grant_without_second_commit() {
    let (mut service, resource_calls) = transaction_service(LedgerFailurePoint::None);
    let request = allocate_request("allocate-replay");
    let first = service.allocate(&request).unwrap();
    let replay = service.allocate(&request).unwrap();

    assert_eq!(first.message.lease_id, replay.message.lease_id);
    assert_eq!(first.message.resources, replay.message.resources);
    assert_eq!(first.attachments.len(), 1);
    assert_eq!(replay.attachments.len(), 1);
    assert_eq!(resource_calls.load(Ordering::SeqCst), 4);
    assert_eq!(service.engine().ledger().commits.load(Ordering::SeqCst), 1);
}

fn listener(name: &[u8]) -> OwnedFd {
    let fd = socket(
        AddressFamily::Unix,
        SockType::SeqPacket,
        SockFlag::SOCK_CLOEXEC,
        None,
    )
    .unwrap();
    bind(fd.as_raw_fd(), &UnixAddr::new_abstract(name).unwrap()).unwrap();
    listen(&fd, Backlog::new(4).unwrap()).unwrap();
    fd
}

fn binding(
    role: broker::RealmChildRole,
    kind: broker::RealmChildFdKind,
    index: u32,
) -> broker::RealmChildFd {
    let mut fd = broker::RealmChildFd::new();
    fd.role = role.into();
    fd.kind = kind.into();
    fd.attachment_index = index;
    fd
}

fn controller_static_identity() -> OwnedFd {
    let fd = rustix::fs::memfd_create(
        "controller-static-identity",
        rustix::fs::MemfdFlags::CLOEXEC | rustix::fs::MemfdFlags::ALLOW_SEALING,
    )
    .unwrap();
    let mut writer = File::from(fd);
    writer.write_all(&[7; 32]).unwrap();
    writer.seek(SeekFrom::Start(0)).unwrap();
    nix::fcntl::fcntl(
        writer.as_raw_fd(),
        nix::fcntl::FcntlArg::F_ADD_SEALS(
            nix::fcntl::SealFlag::F_SEAL_WRITE
                | nix::fcntl::SealFlag::F_SEAL_GROW
                | nix::fcntl::SealFlag::F_SEAL_SHRINK
                | nix::fcntl::SealFlag::F_SEAL_SEAL,
        ),
    )
    .unwrap();
    let readonly = rustix::fs::open(
        format!("/proc/self/fd/{}", writer.as_raw_fd()),
        rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )
    .unwrap();
    drop(writer);
    readonly
}

fn spawn_request(
    operation_id: &str,
    mut bootstrap_sessions: Vec<OwnedFd>,
) -> (broker::SpawnRealmChildrenRequest, Vec<OwnedFd>) {
    assert_eq!(bootstrap_sessions.len(), 2);
    let mut request = broker::SpawnRealmChildrenRequest::new();
    request.metadata = Some(metadata()).into();
    request.scope = Some(scope()).into();
    request.operation_id = operation_id.into();
    request.realm_id = RUNTIME_REALM_ID.into();
    request.controller_generation_id = "generation-1".into();
    request.controller_process_id = "controller-1".into();
    request.broker_process_id = "broker-1".into();
    request.launch_record_digest = vec![7; 32];
    let controller_session = bootstrap_sessions.remove(0);
    let broker_session = bootstrap_sessions.remove(0);
    let socket_ordinal = CREDENTIAL_SOCKET_COUNTER.fetch_add(1, Ordering::Relaxed);
    let attachments: Vec<OwnedFd> = vec![
        listener(format!("w5-controller-{socket_ordinal}").as_bytes()),
        listener(format!("w5-broker-{socket_ordinal}").as_bytes()),
        controller_session,
        broker_session,
        File::open("/sys/fs/cgroup").unwrap().into(),
        File::open("/sys/fs/cgroup").unwrap().into(),
        controller_static_identity(),
    ];
    let mut static_identity = binding(
        broker::RealmChildRole::REALM_CHILD_ROLE_CONTROLLER,
        broker::RealmChildFdKind::REALM_CHILD_FD_KIND_RESOURCE,
        6,
    );
    static_identity.resource_id =
        Some(d2b_host::guest_runtime::CONTROLLER_STATIC_IDENTITY_RESOURCE_ID.to_owned());
    request.fds = vec![
        binding(
            broker::RealmChildRole::REALM_CHILD_ROLE_CONTROLLER,
            broker::RealmChildFdKind::REALM_CHILD_FD_KIND_PUBLIC_LISTENER,
            0,
        ),
        binding(
            broker::RealmChildRole::REALM_CHILD_ROLE_BROKER,
            broker::RealmChildFdKind::REALM_CHILD_FD_KIND_BROKER_LISTENER,
            1,
        ),
        binding(
            broker::RealmChildRole::REALM_CHILD_ROLE_CONTROLLER,
            broker::RealmChildFdKind::REALM_CHILD_FD_KIND_BOOTSTRAP_SESSION,
            2,
        ),
        binding(
            broker::RealmChildRole::REALM_CHILD_ROLE_BROKER,
            broker::RealmChildFdKind::REALM_CHILD_FD_KIND_BOOTSTRAP_SESSION,
            3,
        ),
        binding(
            broker::RealmChildRole::REALM_CHILD_ROLE_CONTROLLER,
            broker::RealmChildFdKind::REALM_CHILD_FD_KIND_CGROUP_LEAF,
            4,
        ),
        binding(
            broker::RealmChildRole::REALM_CHILD_ROLE_BROKER,
            broker::RealmChildFdKind::REALM_CHILD_FD_KIND_CGROUP_LEAF,
            5,
        ),
        static_identity,
    ];
    (request, attachments)
}

#[tokio::test(flavor = "current_thread")]
async fn spawn_correlates_inputs_and_returns_fixed_pidfd_slots_after_credentials() {
    let executor_thread = std::thread::current().id();
    let spawn_thread = Arc::new(Mutex::new(None));
    let peers = bootstrap_processes("valid", "valid");
    let (request, attachments) = spawn_request("spawn-1", peers.sessions);
    let spawner = TestSpawner {
        terminated: Arc::new(AtomicUsize::new(0)),
        spawned: Arc::new(AtomicUsize::new(0)),
        pids: Some(peers.pids),
        corruption: None,
        spawn_thread: Some(Arc::clone(&spawn_thread)),
        spawn_delay: None,
    };
    let reply = service_with_spawner(spawner)
        .spawn(&request, attachments, peers.endpoints)
        .await
        .unwrap();
    cleanup_helpers(peers.helpers, peers.roots);
    assert_eq!(reply.attachments.len(), 2);
    assert_eq!(reply.attachments.controller().attachment_index(), 0);
    assert_eq!(
        reply.attachments.controller().role(),
        RealmChildRole::Controller
    );
    assert_eq!(reply.attachments.broker().attachment_index(), 1);
    assert_eq!(reply.attachments.broker().role(), RealmChildRole::Broker);
    for attachment in [reply.attachments.controller(), reply.attachments.broker()] {
        let flags = nix::fcntl::fcntl(
            attachment.as_fd().as_raw_fd(),
            nix::fcntl::FcntlArg::F_GETFD,
        )
        .unwrap();
        assert!(
            nix::fcntl::FdFlag::from_bits_truncate(flags).contains(nix::fcntl::FdFlag::FD_CLOEXEC)
        );
    }
    assert_eq!(
        format!("{:?}", reply.attachments),
        "VerifiedPidfdAttachments(REDACTED)"
    );
    assert_eq!(reply.message.children[0].pidfd_attachment_index, 0);
    assert_eq!(reply.message.children[1].pidfd_attachment_index, 1);
    assert_ne!(
        spawn_thread.lock().unwrap().expect("spawn thread recorded"),
        executor_thread
    );
}

#[tokio::test(flavor = "current_thread")]
async fn verified_pidfd_attachments_bind_only_to_exact_process_policies() {
    let peers = bootstrap_processes("valid", "valid");
    let pids = peers.pids;
    let (request, attachments) = spawn_request("spawn-policy", peers.sessions);
    let spawner = TestSpawner {
        pids: Some(pids),
        ..TestSpawner::default()
    };
    let reply = service_with_spawner(spawner)
        .spawn(&request, attachments, peers.endpoints)
        .await
        .unwrap();
    let policies = reply
        .attachments
        .bind_policies(
            Arc::new(ExpectedPidVerifier(pids.0)),
            Arc::new(ExpectedPidVerifier(pids.1)),
        )
        .unwrap();
    assert_eq!(policies.controller().attachment_index(), 0);
    assert_eq!(policies.controller().role(), RealmChildRole::Controller);
    assert_eq!(policies.broker().attachment_index(), 1);
    assert_eq!(policies.broker().role(), RealmChildRole::Broker);
    assert_eq!(
        format!("{policies:?}"),
        "PolicyBoundPidfdAttachments(REDACTED)"
    );
    cleanup_helpers(peers.helpers, peers.roots);
}

#[tokio::test(flavor = "current_thread")]
async fn swapped_policy_evidence_fails_closed_and_consumes_both_pidfds() {
    let peers = bootstrap_processes("valid", "valid");
    let pids = peers.pids;
    let (request, attachments) = spawn_request("spawn-swapped-policy", peers.sessions);
    let reply = service_with_spawner(TestSpawner {
        pids: Some(pids),
        ..TestSpawner::default()
    })
    .spawn(&request, attachments, peers.endpoints)
    .await
    .unwrap();
    let controller_fd = reply.attachments.controller().as_fd().as_raw_fd();
    let broker_fd = reply.attachments.broker().as_fd().as_raw_fd();
    let error = reply
        .attachments
        .bind_policies(
            Arc::new(ExpectedPidVerifier(pids.1)),
            Arc::new(ExpectedPidVerifier(pids.0)),
        )
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("pidfd policy construction failed")
    );
    assert_ne!(pidfd_process_id(controller_fd), Some(pids.0));
    assert_ne!(pidfd_process_id(broker_fd), Some(pids.1));
    cleanup_helpers(peers.helpers, peers.roots);
}

async fn assert_spawn_correlation_failure(corruption: SpawnCorruption) {
    let peers = bootstrap_processes("valid", "valid");
    let (request, attachments) = spawn_request("spawn-correlation-failure", peers.sessions);
    let terminated = Arc::new(AtomicUsize::new(0));
    let error = service_with_spawner(TestSpawner {
        terminated: terminated.clone(),
        pids: Some(peers.pids),
        corruption: Some(corruption),
        ..TestSpawner::default()
    })
    .spawn(&request, attachments, peers.endpoints)
    .await
    .unwrap_err();
    assert!(
        error.to_string().contains("pidfd evidence correlation")
            || error.to_string().contains("missing CLOEXEC")
    );
    assert_eq!(terminated.load(Ordering::SeqCst), 1);
    cleanup_helpers(peers.helpers, peers.roots);
}

#[tokio::test(flavor = "current_thread")]
async fn spawn_rejects_swapped_and_duplicate_child_evidence_before_acceptance() {
    assert_spawn_correlation_failure(SpawnCorruption::SwappedIdentities).await;
    assert_spawn_correlation_failure(SpawnCorruption::DuplicateControllerIdentity).await;
}

#[tokio::test(flavor = "current_thread")]
async fn spawn_rejects_pidfd_without_cloexec_and_cleans_pair() {
    assert_spawn_correlation_failure(SpawnCorruption::MissingCloexec).await;
}

#[test]
fn spawn_result_structurally_requires_both_evidence_bearing_pidfds() {
    let source = include_str!("../src/allocator_service.rs");
    assert!(source.contains("struct VerifiedPidfdAttachments {\n    controller:"));
    assert!(source.contains("broker: VerifiedPidfdAttachment,"));
    assert!(source.contains("evidence: PidfdEvidence,"));
    assert!(source.contains("pub fn bind_policies(\n        self,"));
    assert!(!source.contains("impl Clone for VerifiedPidfdAttachment"));
}

async fn assert_credential_failure_cleans_pair(
    operation_id: &str,
    controller_mode: &str,
    timeout: Duration,
) -> AllocatorServiceError {
    let peers = bootstrap_processes(controller_mode, "valid");
    let (request, attachments) = spawn_request(operation_id, peers.sessions);
    let terminated = Arc::new(AtomicUsize::new(0));
    let spawner = TestSpawner {
        terminated: terminated.clone(),
        spawned: Arc::new(AtomicUsize::new(0)),
        pids: Some(peers.pids),
        corruption: None,
        spawn_thread: None,
        spawn_delay: None,
    };
    let error = service_with_spawner(spawner)
        .with_credential_timeout(timeout)
        .spawn(&request, attachments, peers.endpoints)
        .await
        .unwrap_err();
    assert_eq!(terminated.load(Ordering::SeqCst), 1);
    cleanup_helpers(peers.helpers, peers.roots);
    error
}

#[tokio::test(flavor = "current_thread")]
async fn spawn_rejects_first_packet_credential_mismatch_and_cleans_pair() {
    let error = assert_credential_failure_cleans_pair(
        "spawn-credential-mismatch",
        "mismatch",
        Duration::from_secs(2),
    )
    .await;
    assert!(error.to_string().contains("credential-mismatch"));
}

#[tokio::test(flavor = "current_thread")]
async fn spawn_rejects_truncated_first_packet_and_cleans_pair() {
    let error = assert_credential_failure_cleans_pair(
        "spawn-credential-truncated",
        "truncated",
        Duration::from_secs(2),
    )
    .await;
    assert!(error.to_string().contains("message-truncated"));
}

#[tokio::test(flavor = "current_thread")]
async fn spawn_times_out_waiting_for_credentials_and_cleans_pair() {
    let error = assert_credential_failure_cleans_pair(
        "spawn-credential-timeout",
        "timeout",
        Duration::from_millis(25),
    )
    .await;
    assert!(error.to_string().contains("credential-timeout"));
}

#[tokio::test(flavor = "current_thread")]
async fn cancelling_spawn_during_credential_authentication_cleans_pair_once() {
    let peers = bootstrap_processes("timeout", "valid");
    let (request, attachments) = spawn_request("spawn-cancelled-authentication", peers.sessions);
    let terminated = Arc::new(AtomicUsize::new(0));
    let spawned = Arc::new(AtomicUsize::new(0));
    let service = service_with_spawner(TestSpawner {
        terminated: terminated.clone(),
        spawned: spawned.clone(),
        pids: Some(peers.pids),
        corruption: None,
        spawn_thread: None,
        spawn_delay: None,
    })
    .with_credential_timeout(Duration::from_secs(30));

    tokio::time::timeout(
        Duration::from_millis(25),
        service.spawn(&request, attachments, peers.endpoints),
    )
    .await
    .unwrap_err();

    assert_eq!(spawned.load(Ordering::SeqCst), 1);
    assert_eq!(terminated.load(Ordering::SeqCst), 1);
    cleanup_helpers(peers.helpers, peers.roots);
}

#[tokio::test(flavor = "current_thread")]
async fn cancelling_spawn_during_blocking_worker_cleans_pair_once() {
    let peers = bootstrap_processes("valid", "valid");
    let (request, attachments) = spawn_request("spawn-cancelled-worker", peers.sessions);
    let terminated = Arc::new(AtomicUsize::new(0));
    let spawned = Arc::new(AtomicUsize::new(0));
    let service = service_with_spawner(TestSpawner {
        terminated: Arc::clone(&terminated),
        spawned: Arc::clone(&spawned),
        pids: Some(peers.pids),
        corruption: None,
        spawn_thread: None,
        spawn_delay: Some(Duration::from_millis(100)),
    });

    tokio::time::timeout(
        Duration::from_millis(10),
        service.spawn(&request, attachments, peers.endpoints),
    )
    .await
    .unwrap_err();
    tokio::time::timeout(Duration::from_secs(2), async {
        while terminated.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();

    assert_eq!(spawned.load(Ordering::SeqCst), 1);
    assert_eq!(terminated.load(Ordering::SeqCst), 1);
    cleanup_helpers(peers.helpers, peers.roots);
}

#[tokio::test(flavor = "current_thread")]
async fn spawn_rejects_mismatched_bootstrap_child_endpoints_before_clone() {
    let mut peers = bootstrap_processes("valid", "valid");
    peers.sessions.swap(0, 1);
    let (request, attachments) = spawn_request("spawn-endpoint-mismatch", peers.sessions);
    let spawned = Arc::new(AtomicUsize::new(0));
    let spawner = TestSpawner {
        terminated: Arc::new(AtomicUsize::new(0)),
        spawned: spawned.clone(),
        pids: Some(peers.pids),
        corruption: None,
        spawn_thread: None,
        spawn_delay: None,
    };
    let error = service_with_spawner(spawner)
        .spawn(&request, attachments, peers.endpoints)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("bootstrap-endpoint-mismatch"));
    assert_eq!(spawned.load(Ordering::SeqCst), 0);
    cleanup_helpers(peers.helpers, peers.roots);
}

#[tokio::test(flavor = "current_thread")]
async fn spawn_rejects_attachment_metadata_mismatch_before_spawning() {
    let mut request = broker::SpawnRealmChildrenRequest::new();
    request.metadata = Some(metadata()).into();
    request.scope = Some(scope()).into();
    request.operation_id = "spawn-2".into();
    request.realm_id = RUNTIME_REALM_ID.into();
    request.controller_generation_id = "generation-1".into();
    request.controller_process_id = "controller-1".into();
    request.broker_process_id = "broker-1".into();
    request.launch_record_digest = vec![7; 32];
    request.fds = vec![
        binding(
            broker::RealmChildRole::REALM_CHILD_ROLE_CONTROLLER,
            broker::RealmChildFdKind::REALM_CHILD_FD_KIND_PUBLIC_LISTENER,
            0,
        ),
        binding(
            broker::RealmChildRole::REALM_CHILD_ROLE_BROKER,
            broker::RealmChildFdKind::REALM_CHILD_FD_KIND_BROKER_LISTENER,
            1,
        ),
        binding(
            broker::RealmChildRole::REALM_CHILD_ROLE_CONTROLLER,
            broker::RealmChildFdKind::REALM_CHILD_FD_KIND_BOOTSTRAP_SESSION,
            2,
        ),
        binding(
            broker::RealmChildRole::REALM_CHILD_ROLE_BROKER,
            broker::RealmChildFdKind::REALM_CHILD_FD_KIND_BOOTSTRAP_SESSION,
            3,
        ),
    ];
    let peers = bootstrap_processes("valid", "valid");
    assert!(
        service()
            .spawn(&request, Vec::new(), peers.endpoints)
            .await
            .is_err()
    );
    cleanup_helpers(peers.helpers, peers.roots);
}

#[test]
fn listeners_are_prebound_as_a_pair_and_never_replaced() {
    let root = socket_tempdir();
    let listeners = prebind_realm_listeners(root.path(), "work").unwrap();
    let public_path = root.path().join("work/public.sock");
    assert!(public_path.exists());
    assert!(root.path().join("work/broker.sock").exists());
    let public_inode = std::fs::symlink_metadata(&public_path).unwrap().ino();
    assert!(prebind_realm_listeners(root.path(), "work").is_err());
    assert_eq!(
        std::fs::symlink_metadata(&public_path).unwrap().ino(),
        public_inode,
        "a failed bind must not unlink or replace the existing listener"
    );
    drop(listeners);
    root.close().expect("remove listener fixture");
}

#[test]
fn listeners_refuse_a_group_or_world_writable_runtime_root() {
    let root = socket_tempdir();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o777)).unwrap();

    assert!(prebind_realm_listeners(root.path(), "work").is_err());

    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    root.close().expect("remove listener fixture");
}
