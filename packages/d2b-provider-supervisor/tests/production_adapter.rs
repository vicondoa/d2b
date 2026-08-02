//! Shared conformance, fault, and latency coverage for ProviderSupervisor.

use std::num::NonZeroU32;
use std::os::fd::AsFd;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use std::time::{Duration, Instant};

use d2b_process::{
    AdoptionCandidate, BackendLaunch, BackendObservation, IdentityBinding, ObservedIdentity,
    ProcessEffectBackend, ProcessEffectError, ProcessIdentityDigest, ProcessRequest,
    ProcessStopClass, StopClass, WaitReapOwner,
};
use d2b_process_conformance::suite;
use d2b_process_conformance::testing::{ScriptedEffectPort, block_on, fixtures};
use d2b_process_conformance::{
    AdoptionOutcome, ProcessConformanceError, ProcessLaunchEffectPort, ProcessProvider,
};
use d2b_provider_supervisor::{
    BrokerLaunchIntent, BrokerLaunchResolver, BrokerObservedProcess, BrokerProcessBackend,
    ProviderSupervisor, SystemdEffectLaunch, SystemdEffectOwner, SystemdInvocationIdentity,
    SystemdProcessBackend,
};
use d2b_provider_system_minijail::{MinijailProcessProvider, PROVIDER_NAME as MINIJAIL};
use d2b_provider_system_systemd::{PROVIDER_NAME as SYSTEMD, SystemdProcessProvider};

fn minijail_bindings() -> Vec<IdentityBinding> {
    vec![
        IdentityBinding::Pid,
        IdentityBinding::ProcessStartTime,
        IdentityBinding::Cgroup,
        IdentityBinding::Executable,
        IdentityBinding::Template,
        IdentityBinding::Generation,
    ]
}

fn systemd_bindings() -> Vec<IdentityBinding> {
    vec![
        IdentityBinding::UnitInvocationId,
        IdentityBinding::Cgroup,
        IdentityBinding::UnitMainPid,
        IdentityBinding::ProcessStartTime,
        IdentityBinding::Template,
        IdentityBinding::Generation,
    ]
}

#[derive(Clone, Copy)]
enum Mode {
    Good,
    LaunchFailure,
    Vanished,
    Reused,
    WrongOwner,
}

struct DeterministicBackend {
    bindings: Vec<IdentityBinding>,
    owner: WaitReapOwner,
    mode: Mode,
    calls: Arc<Mutex<Vec<&'static str>>>,
    delay: Duration,
}

impl DeterministicBackend {
    fn new(bindings: Vec<IdentityBinding>, owner: WaitReapOwner) -> Self {
        Self {
            bindings,
            owner,
            mode: Mode::Good,
            calls: Arc::new(Mutex::new(Vec::new())),
            delay: Duration::ZERO,
        }
    }

    fn mode(mut self, mode: Mode) -> Self {
        self.mode = mode;
        self
    }

    fn delay(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }

    fn observation(&self) -> BackendObservation {
        let bindings = match self.mode {
            Mode::Reused => self
                .bindings
                .iter()
                .copied()
                .filter(|binding| *binding != IdentityBinding::ProcessStartTime)
                .collect::<Vec<_>>(),
            _ => self.bindings.clone(),
        };
        let owner = match self.mode {
            Mode::WrongOwner if self.owner == WaitReapOwner::Local => WaitReapOwner::ServiceManager,
            Mode::WrongOwner => WaitReapOwner::Local,
            _ => self.owner,
        };
        BackendObservation::new(
            ProcessIdentityDigest::from_bytes([0x44; 32]),
            ObservedIdentity::from_verified(bindings),
            owner,
        )
    }

    fn record(&self, call: &'static str) {
        self.calls.lock().unwrap().push(call);
    }
}

impl ProcessEffectBackend for DeterministicBackend {
    type Handle = ();

    fn launch(
        &self,
        _request: ProcessRequest,
    ) -> Result<BackendLaunch<Self::Handle>, ProcessEffectError> {
        self.record("launch");
        if !self.delay.is_zero() {
            std::thread::sleep(self.delay);
        }
        if matches!(self.mode, Mode::LaunchFailure) {
            return Err(ProcessEffectError::LaunchFailed);
        }
        Ok(BackendLaunch::new(self.observation(), ()))
    }

    fn observe(
        &self,
        _request: ProcessRequest,
    ) -> Result<Option<BackendObservation>, ProcessEffectError> {
        self.record("observe");
        if matches!(self.mode, Mode::Vanished) {
            return Ok(None);
        }
        Ok(Some(self.observation()))
    }

    fn open_pidfd(
        &self,
        _observation: BackendObservation,
    ) -> Result<Self::Handle, ProcessEffectError> {
        self.record("open-pidfd");
        Ok(())
    }

    fn stop(
        &self,
        _handle: &Self::Handle,
        _class: ProcessStopClass,
    ) -> Result<(), ProcessEffectError> {
        self.record("stop");
        Ok(())
    }
}

fn minijail(
    backend: DeterministicBackend,
) -> MinijailProcessProvider<ProviderSupervisor<DeterministicBackend>> {
    MinijailProcessProvider::new(ProviderSupervisor::new(backend))
}

fn systemd(
    backend: DeterministicBackend,
) -> SystemdProcessProvider<ProviderSupervisor<DeterministicBackend>> {
    SystemdProcessProvider::new(ProviderSupervisor::new(backend))
}

fn minijail_good() -> MinijailProcessProvider<ProviderSupervisor<DeterministicBackend>> {
    minijail(DeterministicBackend::new(
        minijail_bindings(),
        WaitReapOwner::Local,
    ))
}

fn systemd_good() -> SystemdProcessProvider<ProviderSupervisor<DeterministicBackend>> {
    systemd(DeterministicBackend::new(
        systemd_bindings(),
        WaitReapOwner::ServiceManager,
    ))
}

struct ScriptedBackend {
    port: ScriptedEffectPort,
}

impl ProcessEffectBackend for ScriptedBackend {
    type Handle = ();

    fn launch(
        &self,
        request: ProcessRequest,
    ) -> Result<BackendLaunch<Self::Handle>, ProcessEffectError> {
        let launched = block_on(self.port.launch(request.ticket())).map_err(effect_error)?;
        Ok(BackendLaunch::new(
            BackendObservation::new(
                launched.identity,
                launched.observed,
                launched.wait_reap_owner,
            ),
            (),
        ))
    }

    fn observe(
        &self,
        request: ProcessRequest,
    ) -> Result<Option<BackendObservation>, ProcessEffectError> {
        let candidate = block_on(self.port.observe(request.ticket())).map_err(effect_error)?;
        Ok(candidate.map(|candidate| {
            BackendObservation::new(
                candidate.identity,
                candidate.observed,
                candidate.wait_reap_owner,
            )
        }))
    }

    fn open_pidfd(
        &self,
        observation: BackendObservation,
    ) -> Result<Self::Handle, ProcessEffectError> {
        block_on(self.port.open_pidfd(&AdoptionCandidate {
            identity: observation.identity(),
            observed: observation.observed().clone(),
            wait_reap_owner: observation.wait_reap_owner(),
        }))
        .map_err(effect_error)?;
        Ok(())
    }

    fn stop(
        &self,
        _handle: &Self::Handle,
        class: ProcessStopClass,
    ) -> Result<(), ProcessEffectError> {
        let class = match class {
            ProcessStopClass::Drain => StopClass::Drain,
            ProcessStopClass::Terminate => StopClass::Terminate,
        };
        block_on(
            self.port
                .stop(&ProcessIdentityDigest::from_bytes([0x44; 32]), class),
        )
        .map_err(effect_error)
    }
}

fn effect_error(error: ProcessConformanceError) -> ProcessEffectError {
    match error {
        ProcessConformanceError::WaitOwnerMismatch => ProcessEffectError::WaitOwnerMismatch,
        ProcessConformanceError::IdentityUnverified
        | ProcessConformanceError::AdoptionAmbiguous => ProcessEffectError::IdentityChanged,
        ProcessConformanceError::PidfdUnavailable => ProcessEffectError::PidfdUnavailable,
        ProcessConformanceError::DeadlineExceeded => ProcessEffectError::DeadlineExceeded,
        _ => ProcessEffectError::LaunchFailed,
    }
}

fn scripted_minijail(
    port: ScriptedEffectPort,
) -> MinijailProcessProvider<ProviderSupervisor<ScriptedBackend>> {
    MinijailProcessProvider::new(ProviderSupervisor::new(ScriptedBackend { port }))
}

fn scripted_systemd(
    port: ScriptedEffectPort,
) -> SystemdProcessProvider<ProviderSupervisor<ScriptedBackend>> {
    SystemdProcessProvider::new(ProviderSupervisor::new(ScriptedBackend { port }))
}

#[test]
fn shared_conformance_runs_through_the_production_adapter() {
    suite::assert_launch_is_locality_neutral(&minijail_good(), MINIJAIL);
    suite::assert_foreign_provider_selection_is_rejected(&minijail_good());
    suite::assert_domain_support_matches_the_profile(&minijail_good(), MINIJAIL);
    suite::assert_status_is_redacted(&minijail_good(), MINIJAIL);
    suite::assert_incomplete_launch_identity_fails_closed(scripted_minijail, MINIJAIL);
    suite::assert_adoption_verifies_identity_before_opening_a_pidfd(scripted_minijail, MINIJAIL);

    suite::assert_launch_is_locality_neutral(&systemd_good(), SYSTEMD);
    suite::assert_foreign_provider_selection_is_rejected(&systemd_good());
    suite::assert_domain_support_matches_the_profile(&systemd_good(), SYSTEMD);
    suite::assert_status_is_redacted(&systemd_good(), SYSTEMD);
    suite::assert_incomplete_launch_identity_fails_closed(scripted_systemd, SYSTEMD);
    suite::assert_adoption_verifies_identity_before_opening_a_pidfd(scripted_systemd, SYSTEMD);
}

#[test]
fn fault_matrix_fails_closed() {
    let ticket = fixtures::ticket_builder()
        .selected_provider(MINIJAIL)
        .expected_identity(minijail_bindings())
        .build()
        .unwrap();

    let failed = minijail(
        DeterministicBackend::new(minijail_bindings(), WaitReapOwner::Local)
            .mode(Mode::LaunchFailure),
    );
    assert_eq!(
        block_on(failed.launch(&ticket)).unwrap_err(),
        ProcessConformanceError::LaunchFailed
    );

    let vanished = minijail(
        DeterministicBackend::new(minijail_bindings(), WaitReapOwner::Local).mode(Mode::Vanished),
    );
    assert_eq!(
        block_on(vanished.adopt(&ticket)).unwrap(),
        AdoptionOutcome::Absent
    );

    let reused_backend =
        DeterministicBackend::new(minijail_bindings(), WaitReapOwner::Local).mode(Mode::Reused);
    let reused_calls = Arc::clone(&reused_backend.calls);
    let reused = minijail(reused_backend);
    assert!(matches!(
        block_on(reused.adopt(&ticket)).unwrap(),
        AdoptionOutcome::Quarantined(_)
    ));
    assert_eq!(*reused_calls.lock().unwrap(), vec!["observe"]);

    let wrong_owner = minijail(
        DeterministicBackend::new(minijail_bindings(), WaitReapOwner::Local).mode(Mode::WrongOwner),
    );
    assert_eq!(
        block_on(wrong_owner.launch(&ticket)).unwrap_err(),
        ProcessConformanceError::WaitOwnerMismatch
    );
}

#[derive(Clone)]
struct SystemdOwner {
    first: SystemdInvocationIdentity,
    reopened: SystemdInvocationIdentity,
}

impl SystemdEffectOwner for SystemdOwner {
    type Handle = ();

    fn launch(
        &self,
        _request: ProcessRequest,
    ) -> Result<SystemdEffectLaunch<Self::Handle>, ProcessEffectError> {
        Ok(SystemdEffectLaunch::new(self.first.clone(), ()))
    }

    fn observe(
        &self,
        _request: ProcessRequest,
    ) -> Result<Option<SystemdInvocationIdentity>, ProcessEffectError> {
        Ok(Some(self.first.clone()))
    }

    fn reopen(
        &self,
        _expected: &SystemdInvocationIdentity,
    ) -> Result<SystemdEffectLaunch<Self::Handle>, ProcessEffectError> {
        Ok(SystemdEffectLaunch::new(self.reopened.clone(), ()))
    }

    fn stop(
        &self,
        _handle: &Self::Handle,
        _class: ProcessStopClass,
    ) -> Result<(), ProcessEffectError> {
        Ok(())
    }
}

fn invocation(start_time: u64) -> SystemdInvocationIdentity {
    SystemdInvocationIdentity::new(
        [1; 16],
        [2; 32],
        NonZeroU32::new(100).unwrap(),
        start_time,
        [3; 32],
        [4; 32],
        1,
    )
    .unwrap()
}

#[test]
fn systemd_reopen_rejects_process_reuse() {
    let backend = SystemdProcessBackend::new(SystemdOwner {
        first: invocation(7),
        reopened: invocation(8),
    });
    let supervisor = ProviderSupervisor::new(backend);
    let provider = SystemdProcessProvider::new(supervisor);
    let ticket = fixtures::ticket_builder()
        .selected_provider(SYSTEMD)
        .expected_identity(systemd_bindings())
        .build()
        .unwrap();
    assert!(matches!(
        block_on(provider.adopt(&ticket)),
        Err(ProcessConformanceError::AdoptionAmbiguous)
    ));
}

#[derive(Clone)]
struct MissingSystemdOwner;

impl SystemdEffectOwner for MissingSystemdOwner {
    type Handle = ();

    fn launch(
        &self,
        _request: ProcessRequest,
    ) -> Result<SystemdEffectLaunch<Self::Handle>, ProcessEffectError> {
        Err(ProcessEffectError::Vanished)
    }

    fn observe(
        &self,
        _request: ProcessRequest,
    ) -> Result<Option<SystemdInvocationIdentity>, ProcessEffectError> {
        Ok(None)
    }

    fn reopen(
        &self,
        _expected: &SystemdInvocationIdentity,
    ) -> Result<SystemdEffectLaunch<Self::Handle>, ProcessEffectError> {
        Err(ProcessEffectError::Vanished)
    }

    fn stop(
        &self,
        _handle: &Self::Handle,
        _class: ProcessStopClass,
    ) -> Result<(), ProcessEffectError> {
        Err(ProcessEffectError::Vanished)
    }
}

#[test]
fn a_vanished_transient_unit_is_absent_not_adopted() {
    let provider = SystemdProcessProvider::new(ProviderSupervisor::new(
        SystemdProcessBackend::new(MissingSystemdOwner),
    ));
    let ticket = fixtures::ticket_builder()
        .selected_provider(SYSTEMD)
        .expected_identity(systemd_bindings())
        .build()
        .unwrap();
    assert_eq!(
        block_on(provider.adopt(&ticket)).unwrap(),
        AdoptionOutcome::Absent
    );
}

#[derive(Clone)]
struct FixedBrokerResolver {
    intent: BrokerLaunchIntent,
}

impl BrokerLaunchResolver for FixedBrokerResolver {
    fn resolve(&self, _request: &ProcessRequest) -> Result<BrokerLaunchIntent, ProcessEffectError> {
        Ok(self.intent.clone())
    }

    fn observe(
        &self,
        _request: &ProcessRequest,
    ) -> Result<Option<BrokerObservedProcess>, ProcessEffectError> {
        Ok(None)
    }
}

#[test]
fn broker_backend_uses_the_production_spawn_wire_and_pidfd_handoff() {
    use std::io::{IoSlice, IoSliceMut};

    use d2b_contracts::broker_wire::{
        BrokerRequest, BrokerRequestEnvelope, BrokerResponse, RunnerRole, SpawnRunnerResponse,
    };
    use d2b_contracts::types::{BundleOpId, RoleId, VmId};
    use rustix::net::{
        AddressFamily, RecvAncillaryBuffer, RecvFlags, SendAncillaryBuffer, SendAncillaryMessage,
        SendFlags, SocketAddrUnix, SocketFlags, SocketType, accept, bind_unix, listen, recvmsg,
        sendmsg, socket_with,
    };

    let directory = tempfile::tempdir().unwrap();
    let socket_path = directory.path().join("broker.sock");
    let listener = socket_with(
        AddressFamily::UNIX,
        SocketType::SEQPACKET,
        SocketFlags::CLOEXEC,
        None,
    )
    .unwrap();
    let address = SocketAddrUnix::new(&socket_path).unwrap();
    bind_unix(&listener, &address).unwrap();
    listen(&listener, 1).unwrap();

    let vm_id = VmId::new("corp-vm");
    let role_id = RoleId::new("worker");
    let server_vm = vm_id.clone();
    let server_role = role_id.clone();
    let server = std::thread::spawn(move || {
        let connection = accept(&listener).unwrap();
        let mut payload = vec![0_u8; d2b_contracts::MAX_FRAME_SIZE + 4];
        let mut iov = [IoSliceMut::new(&mut payload)];
        let mut ancillary_bytes = [];
        let mut ancillary = RecvAncillaryBuffer::new(&mut ancillary_bytes);
        let received = recvmsg(
            &connection,
            &mut iov,
            &mut ancillary,
            RecvFlags::CMSG_CLOEXEC,
        )
        .unwrap();
        let request: BrokerRequestEnvelope =
            d2b_contracts::decode_frame("BrokerRequestEnvelope", &payload[..received.bytes])
                .unwrap();
        match request.request {
            BrokerRequest::SpawnRunner(request) => {
                assert_eq!(request.vm_id, server_vm);
                assert_eq!(request.role_id, server_role);
                assert_eq!(request.role, RunnerRole::Virtiofsd);
            }
            _ => panic!("expected SpawnRunner"),
        }

        let pid = i32::try_from(std::process::id()).unwrap();
        let start_time_ticks = read_self_start_time();
        let pid = rustix::process::Pid::from_raw(pid).unwrap();
        let pidfd = rustix::process::pidfd_open(pid, rustix::process::PidfdFlags::empty())
            .expect("open self pidfd");
        let response = BrokerResponse::SpawnRunner(SpawnRunnerResponse {
            vm_id: server_vm,
            role_id: server_role,
            role: RunnerRole::Virtiofsd,
            pid: pid.as_raw_nonzero().get(),
            start_time_ticks,
            pidfd_index: 0,
            console_fd_index: None,
        });
        let frame = d2b_contracts::encode_frame(&response).unwrap();
        let iov = [IoSlice::new(&frame)];
        let descriptors = [pidfd.as_fd()];
        let mut control_bytes = [0_u8; rustix::cmsg_space!(ScmRights(1))];
        let mut control = SendAncillaryBuffer::new(&mut control_bytes);
        assert!(control.push(SendAncillaryMessage::ScmRights(&descriptors)));
        assert_eq!(
            sendmsg(&connection, &iov, &mut control, SendFlags::empty()).unwrap(),
            frame.len()
        );
    });

    let intent = BrokerLaunchIntent {
        vm_id,
        role_id,
        role: RunnerRole::Virtiofsd,
        bundle_runner_intent_ref: BundleOpId::new("runner:vm:corp-vm:role:worker"),
        provider_identity: [0x11; 32],
        template_identity: [0x22; 32],
        generation: 1,
    };
    let backend = BrokerProcessBackend::with_socket(
        FixedBrokerResolver { intent },
        &socket_path,
        Duration::from_secs(2),
    );
    let provider = MinijailProcessProvider::new(ProviderSupervisor::new(backend));
    let ticket = fixtures::ticket_builder()
        .selected_provider(MINIJAIL)
        .expected_identity(minijail_bindings())
        .build()
        .unwrap();
    let report = block_on(provider.launch(&ticket)).unwrap();
    assert_eq!(report.wait_reap_owner, WaitReapOwner::Local);
    drop(provider);
    server.join().unwrap();
}

fn read_self_start_time() -> u64 {
    let stat = std::fs::read_to_string("/proc/self/stat").unwrap();
    let close = stat.rfind(')').unwrap();
    stat[close + 1..]
        .split_whitespace()
        .nth(19)
        .unwrap()
        .parse()
        .unwrap()
}

#[tokio::test(flavor = "current_thread")]
async fn blocking_effects_do_not_stall_the_async_executor() {
    let supervisor = ProviderSupervisor::with_limits(
        DeterministicBackend::new(minijail_bindings(), WaitReapOwner::Local)
            .delay(Duration::from_millis(50)),
        16,
        Duration::from_secs(2),
    );
    let ticket = fixtures::ticket_builder()
        .selected_provider(MINIJAIL)
        .expected_identity(minijail_bindings())
        .build()
        .unwrap();
    let started = Instant::now();
    let launch = supervisor.launch(&ticket);
    let heartbeat = async {
        let mut ticks = Vec::new();
        for _ in 0..5 {
            tokio::time::sleep(Duration::from_millis(10)).await;
            ticks.push(started.elapsed());
        }
        ticks
    };
    let (result, ticks) = tokio::join!(launch, heartbeat);
    result.unwrap();
    let max_gap = ticks
        .windows(2)
        .map(|pair| pair[1] - pair[0])
        .max()
        .unwrap();
    assert!(
        max_gap < Duration::from_millis(40),
        "blocking backend stalled the async executor"
    );
}

struct ParallelLaunchBackend {
    started: Arc<Mutex<Vec<Instant>>>,
    active: Arc<AtomicUsize>,
    max_active: Arc<AtomicUsize>,
    next_identity: AtomicUsize,
    delay: Duration,
}

impl ParallelLaunchBackend {
    fn new(delay: Duration) -> (Self, Arc<Mutex<Vec<Instant>>>, Arc<AtomicUsize>) {
        let started = Arc::new(Mutex::new(Vec::new()));
        let max_active = Arc::new(AtomicUsize::new(0));
        (
            Self {
                started: Arc::clone(&started),
                active: Arc::new(AtomicUsize::new(0)),
                max_active: Arc::clone(&max_active),
                next_identity: AtomicUsize::new(1),
                delay,
            },
            started,
            max_active,
        )
    }
}

impl ProcessEffectBackend for ParallelLaunchBackend {
    type Handle = ();

    fn launch(
        &self,
        _request: ProcessRequest,
    ) -> Result<BackendLaunch<Self::Handle>, ProcessEffectError> {
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_active.fetch_max(active, Ordering::SeqCst);
        self.started
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(Instant::now());
        std::thread::sleep(self.delay);
        self.active.fetch_sub(1, Ordering::SeqCst);

        let identity_seed = self.next_identity.fetch_add(1, Ordering::Relaxed) as u8;
        Ok(BackendLaunch::new(
            BackendObservation::new(
                ProcessIdentityDigest::from_bytes([identity_seed; 32]),
                ObservedIdentity::from_verified(minijail_bindings()),
                WaitReapOwner::Local,
            ),
            (),
        ))
    }

    fn observe(
        &self,
        _request: ProcessRequest,
    ) -> Result<Option<BackendObservation>, ProcessEffectError> {
        Ok(None)
    }

    fn open_pidfd(
        &self,
        _observation: BackendObservation,
    ) -> Result<Self::Handle, ProcessEffectError> {
        Ok(())
    }

    fn stop(
        &self,
        _handle: &Self::Handle,
        _class: ProcessStopClass,
    ) -> Result<(), ProcessEffectError> {
        Ok(())
    }
}

fn parallel_ticket(index: usize) -> d2b_process::LaunchTicket {
    use d2b_contracts::v3::execution_policy::{BoundedToken, ExecutionDomain};
    use d2b_contracts::v3::{ControllerGeneration, ResourceGeneration, ResourceRef, ResourceUid};
    use d2b_process::{LaunchTicket, OperationBinding};

    let uid = ResourceUid::parse(format!("123e4567-e89b-42d3-a456-42661417{index:04x}"))
        .expect("parallel fixture UID is valid");
    LaunchTicket::new(
        ResourceRef::parse(&format!("Process/parallel-{index}")).unwrap(),
        uid.clone(),
        ResourceGeneration::new(1).unwrap(),
        ControllerGeneration::new(1).unwrap(),
        BoundedToken::parse("system-minijail").unwrap(),
        BoundedToken::parse("controller").unwrap(),
        BoundedToken::parse("controller-main").unwrap(),
        ResourceRef::parse("Host/host-system").unwrap(),
        ExecutionDomain::System,
        None,
        BoundedToken::parse(MINIJAIL).unwrap(),
        fixtures::compiled_digests(),
        OperationBinding::new(uid, 30_000).unwrap(),
        minijail_bindings().into_iter().collect(),
    )
    .unwrap()
}

#[tokio::test(flavor = "current_thread")]
async fn ready_process_launches_reach_the_provider_adapter_in_parallel() {
    use tokio::sync::Semaphore;

    for count in [1_usize, 10, 100] {
        let (backend, started, max_active) = ParallelLaunchBackend::new(Duration::from_millis(25));
        let supervisor = ProviderSupervisor::with_limits(backend, 16, Duration::from_secs(2));
        let provider = Arc::new(MinijailProcessProvider::new(supervisor));
        let admission = Arc::new(Semaphore::new(16));
        let mut launches = Vec::with_capacity(count);

        for index in 0..count {
            let provider = Arc::clone(&provider);
            let admission = Arc::clone(&admission);
            launches.push(tokio::spawn(async move {
                let permit = admission.acquire_owned().await.unwrap();
                let result = provider.launch(&parallel_ticket(index)).await;
                drop(permit);
                result
            }));
        }

        for launch in launches {
            launch.await.unwrap().unwrap();
        }
        assert_eq!(
            started
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .len(),
            count
        );
        if count > 1 {
            assert!(
                max_active.load(Ordering::SeqCst) >= 2,
                "ready Process launches were serialized by the provider adapter"
            );
        }
    }
}
