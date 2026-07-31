//! Privileged-broker process backend using the production broker wire.

use std::collections::BTreeMap;
use std::fs;
use std::io::IoSliceMut;
use std::os::fd::{AsFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use d2b_contracts::broker_wire::{
    BrokerRequest, BrokerRequestEnvelope, BrokerResponse, DeregisterRunnerPidfdRequest,
    OpenPidfdRequest, RunnerRole, RunnerSignal, SignalRunnerRequest, SpawnRunnerRequest,
};
use d2b_contracts::types::{BundleOpId, RoleId, VmId};
use d2b_process::{
    BackendLaunch, BackendObservation, IdentityBinding, ObservedIdentity, ProcessEffectBackend,
    ProcessEffectError, ProcessIdentityDigest, ProcessRequest, ProcessStopClass, WaitReapOwner,
};
use rustix::event::{PollFd, PollFlags, poll};
use rustix::net::{
    AddressFamily, RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags, SocketFlags, SocketType,
    recvmsg, send, socket_with,
};
use sha2::{Digest, Sha256};
use socket2::Socket;

const MAX_PENDING_OBSERVATIONS: usize = 1024;

/// Trusted-bundle launch intent resolved for one generic Process ticket.
#[derive(Clone, PartialEq, Eq)]
pub struct BrokerLaunchIntent {
    /// Broker VM scope.
    pub vm_id: VmId,
    /// Broker role scope.
    pub role_id: RoleId,
    /// Existing closed broker runner role selecting its trusted argv compiler.
    pub role: RunnerRole,
    /// Opaque runner-intent row in the trusted broker bundle.
    pub bundle_runner_intent_ref: BundleOpId,
    /// Digest of the owning Provider identity resolved from trusted config.
    pub provider_identity: [u8; 32],
    /// Digest of the owning component template resolved from trusted config.
    pub template_identity: [u8; 32],
    /// Nonzero Process resource generation bound to this launch.
    pub generation: u64,
}

impl std::fmt::Debug for BrokerLaunchIntent {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("BrokerLaunchIntent(<redacted>)")
    }
}

/// Candidate discovered independently of the adapter's in-memory handle table.
#[derive(Clone, PartialEq, Eq)]
pub struct BrokerObservedProcess {
    /// Trusted launch intent identifying the broker-managed runner.
    pub intent: BrokerLaunchIntent,
    /// Observed process identifier used only inside the broker boundary.
    pub pid: i32,
    /// Observed process-start-time ticks used to reject identifier reuse.
    pub start_time_ticks: u64,
    /// Whether trusted observation also verified the declared cgroup leaf.
    pub cgroup_verified: bool,
    /// Whether trusted observation verified the executable behind the runner.
    pub executable_verified: bool,
}

impl std::fmt::Debug for BrokerObservedProcess {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("BrokerObservedProcess(<redacted>)")
    }
}

impl BrokerObservedProcess {
    fn validate(&self) -> Result<(), ProcessEffectError> {
        if self.pid <= 0
            || self.start_time_ticks == 0
            || !self.cgroup_verified
            || !self.executable_verified
            || self.intent.provider_identity == [0; 32]
            || self.intent.template_identity == [0; 32]
            || self.intent.generation == 0
        {
            return Err(ProcessEffectError::IdentityChanged);
        }
        Ok(())
    }

    fn digest(&self) -> ProcessIdentityDigest {
        let mut digest = Sha256::new();
        digest.update(b"d2b-broker-process-identity-v1");
        digest.update(self.intent.vm_id.as_str().as_bytes());
        digest.update([0]);
        digest.update(self.intent.role_id.as_str().as_bytes());
        digest.update([0]);
        digest.update(self.intent.role.as_str().as_bytes());
        digest.update(self.intent.provider_identity);
        digest.update(self.intent.template_identity);
        digest.update(self.intent.generation.to_le_bytes());
        digest.update(self.pid.to_le_bytes());
        digest.update(self.start_time_ticks.to_le_bytes());
        ProcessIdentityDigest::from_bytes(digest.finalize().into())
    }

    fn observation(&self) -> BackendObservation {
        BackendObservation::new(
            self.digest(),
            ObservedIdentity::from_verified([
                IdentityBinding::Pid,
                IdentityBinding::ProcessStartTime,
                IdentityBinding::Cgroup,
                IdentityBinding::Executable,
                IdentityBinding::Template,
                IdentityBinding::Generation,
            ]),
            WaitReapOwner::Local,
        )
    }
}

/// Trusted resolver for broker launch and independent adoption observation.
///
/// Generic v3 Process tickets do not carry a legacy [`RunnerRole`]. The
/// resolver must map a ticket to an existing trusted bundle row and may return
/// `UnsupportedProvider` when no exact role disposition exists yet.
pub trait BrokerLaunchResolver: Send + Sync + 'static {
    /// Resolve a validated ticket to one trusted broker runner intent.
    fn resolve(&self, request: &ProcessRequest) -> Result<BrokerLaunchIntent, ProcessEffectError>;

    /// Discover a running candidate and verify non-pid stable bindings.
    fn observe(
        &self,
        request: &ProcessRequest,
    ) -> Result<Option<BrokerObservedProcess>, ProcessEffectError>;
}

/// Core-local pidfd plus the identity tuple the broker verified.
pub struct BrokerPidfdHandle {
    pidfd: OwnedFd,
    observed: BrokerObservedProcess,
}

impl std::fmt::Debug for BrokerPidfdHandle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("BrokerPidfdHandle(<redacted>)")
    }
}

/// Production process backend for existing broker-managed runner roles.
///
/// It sends the repository's production `SpawnRunner`, `OpenPidfd`, and
/// `SignalRunner` wire requests. The broker performs trusted bundle resolution,
/// user-namespace pre-establishment, final cgroup placement, and audited spawn.
/// The pidfd returned via `SCM_RIGHTS` is retained inside this backend.
pub struct BrokerProcessBackend<R: BrokerLaunchResolver> {
    resolver: R,
    socket_path: PathBuf,
    io_timeout: Duration,
    observations: Mutex<BTreeMap<ProcessIdentityDigest, BrokerObservedProcess>>,
}

impl<R: BrokerLaunchResolver> BrokerProcessBackend<R> {
    /// Build a backend using the production broker socket path.
    pub fn new(resolver: R) -> Self {
        Self::with_socket(
            resolver,
            d2b_contracts::BROKER_SOCKET_PATH,
            Duration::from_secs(10),
        )
    }

    /// Build a backend with an explicit socket path and I/O timeout.
    pub fn with_socket(resolver: R, socket_path: impl Into<PathBuf>, io_timeout: Duration) -> Self {
        Self {
            resolver,
            socket_path: socket_path.into(),
            io_timeout,
            observations: Mutex::new(BTreeMap::new()),
        }
    }

    fn request(&self, request: BrokerRequest) -> Result<BrokerFrame, ProcessEffectError> {
        broker_round_trip(&self.socket_path, self.io_timeout, request)
    }

    fn record(&self, observed: BrokerObservedProcess) -> Result<(), ProcessEffectError> {
        let mut observations = self
            .observations
            .lock()
            .map_err(|_| ProcessEffectError::ObserveFailed)?;
        let identity = observed.digest();
        if observations.len() >= MAX_PENDING_OBSERVATIONS
            && !observations.contains_key(&identity)
            && let Some(candidate) = observations.keys().next().copied()
        {
            observations.remove(&candidate);
        }
        observations.insert(identity, observed);
        Ok(())
    }

    fn take_observation(
        &self,
        identity: &ProcessIdentityDigest,
    ) -> Result<BrokerObservedProcess, ProcessEffectError> {
        self.observations
            .lock()
            .map_err(|_| ProcessEffectError::ObserveFailed)?
            .remove(identity)
            .ok_or(ProcessEffectError::IdentityChanged)
    }
}

impl<R: BrokerLaunchResolver> std::fmt::Debug for BrokerProcessBackend<R> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("BrokerProcessBackend(<redacted>)")
    }
}

impl<R: BrokerLaunchResolver> ProcessEffectBackend for BrokerProcessBackend<R> {
    type Handle = BrokerPidfdHandle;

    fn launch(
        &self,
        request: ProcessRequest,
    ) -> Result<BackendLaunch<Self::Handle>, ProcessEffectError> {
        let intent = self.resolver.resolve(&request)?;
        let frame = self.request(BrokerRequest::SpawnRunner(SpawnRunnerRequest {
            vm_id: intent.vm_id.clone(),
            role_id: intent.role_id.clone(),
            role: intent.role,
            bundle_runner_intent_ref: intent.bundle_runner_intent_ref.clone(),
            runtime_allocations: Vec::new(),
            tracing_span_id: None,
            workload_identity: None,
        }))?;
        let BrokerResponse::SpawnRunner(ref response) = frame.response else {
            return Err(response_error(&frame.response, BrokerOperation::Other));
        };
        if response.vm_id != intent.vm_id
            || response.role_id != intent.role_id
            || response.role != intent.role
            || response.pid <= 0
            || response.start_time_ticks == 0
        {
            return Err(ProcessEffectError::IdentityChanged);
        }
        let pidfd = frame.take_fd(response.pidfd_index)?;
        if read_proc_start_time(response.pid)? != Some(response.start_time_ticks) {
            return Err(ProcessEffectError::IdentityChanged);
        }
        let observed = BrokerObservedProcess {
            intent,
            pid: response.pid,
            start_time_ticks: response.start_time_ticks,
            cgroup_verified: true,
            executable_verified: true,
        };
        observed.validate()?;
        let observation = observed.observation();
        Ok(BackendLaunch::new(
            observation,
            BrokerPidfdHandle { pidfd, observed },
        ))
    }

    fn observe(
        &self,
        request: ProcessRequest,
    ) -> Result<Option<BackendObservation>, ProcessEffectError> {
        let Some(observed) = self.resolver.observe(&request)? else {
            return Ok(None);
        };
        observed.validate()?;
        let observation = observed.observation();
        self.record(observed)?;
        Ok(Some(observation))
    }

    fn open_pidfd(
        &self,
        observation: BackendObservation,
    ) -> Result<Self::Handle, ProcessEffectError> {
        let observed = self.take_observation(&observation.identity())?;
        let frame = self.request(BrokerRequest::OpenPidfd(OpenPidfdRequest {
            vm_id: observed.intent.vm_id.clone(),
            role_id: observed.intent.role_id.clone(),
            pid: observed.pid,
            expected_start_time_ticks: observed.start_time_ticks,
            tracing_span_id: None,
        }))?;
        let BrokerResponse::OpenPidfd(ref response) = frame.response else {
            return Err(response_error(&frame.response, BrokerOperation::OpenPidfd));
        };
        if response.vm_id != observed.intent.vm_id
            || response.role_id != observed.intent.role_id
            || response.pid != observed.pid
            || response.verified_start_time_ticks != observed.start_time_ticks
        {
            return Err(ProcessEffectError::IdentityChanged);
        }
        let pidfd = frame.take_fd(response.pidfd_index)?;
        if read_proc_start_time(response.pid)? != Some(response.verified_start_time_ticks) {
            return Err(ProcessEffectError::IdentityChanged);
        }
        Ok(BrokerPidfdHandle { pidfd, observed })
    }

    fn stop(
        &self,
        handle: &Self::Handle,
        class: ProcessStopClass,
    ) -> Result<(), ProcessEffectError> {
        let signal = match class {
            ProcessStopClass::Drain => RunnerSignal::Term,
            ProcessStopClass::Terminate => RunnerSignal::Kill,
        };
        let frame = self.request(BrokerRequest::SignalRunner(SignalRunnerRequest {
            vm_id: handle.observed.intent.vm_id.clone(),
            role_id: handle.observed.intent.role_id.clone(),
            signal,
            pid: Some(handle.observed.pid),
            expected_start_time_ticks: Some(handle.observed.start_time_ticks),
            tracing_span_id: None,
        }))?;
        match frame.response {
            BrokerResponse::SignalRunner(response)
                if response.signaled
                    && response.vm_id == handle.observed.intent.vm_id
                    && response.role_id == handle.observed.intent.role_id =>
            {
                let _ = handle.pidfd.as_fd();
            }
            _ => return Err(ProcessEffectError::StopFailed),
        }
        if class == ProcessStopClass::Terminate {
            wait_pidfd_exit(&handle.pidfd, self.io_timeout)?;
            let frame = self.request(BrokerRequest::DeregisterRunnerPidfd(
                DeregisterRunnerPidfdRequest {
                    vm_id: handle.observed.intent.vm_id.clone(),
                    role_id: handle.observed.intent.role_id.clone(),
                    tracing_span_id: None,
                },
            ))?;
            match frame.response {
                BrokerResponse::DeregisterRunnerPidfd(response)
                    if response.vm_id == handle.observed.intent.vm_id
                        && response.role_id == handle.observed.intent.role_id => {}
                _ => return Err(ProcessEffectError::StopFailed),
            }
        }
        Ok(())
    }
}

fn wait_pidfd_exit(pidfd: &OwnedFd, timeout: Duration) -> Result<(), ProcessEffectError> {
    let timeout_ms = timeout.as_millis().min(i32::MAX as u128) as i32;
    let mut fds = [PollFd::new(
        pidfd,
        PollFlags::IN | PollFlags::ERR | PollFlags::HUP,
    )];
    match poll(&mut fds, timeout_ms) {
        Ok(0) | Err(_) => Err(ProcessEffectError::StopFailed),
        Ok(_) if fds[0].revents().intersects(PollFlags::IN | PollFlags::HUP) => Ok(()),
        Ok(_) => Err(ProcessEffectError::StopFailed),
    }
}

struct BrokerFrame {
    response: BrokerResponse,
    fds: Mutex<Vec<Option<OwnedFd>>>,
}

impl BrokerFrame {
    fn take_fd(&self, index: u32) -> Result<OwnedFd, ProcessEffectError> {
        self.fds
            .lock()
            .map_err(|_| ProcessEffectError::PidfdUnavailable)?
            .get_mut(usize::try_from(index).map_err(|_| ProcessEffectError::PidfdUnavailable)?)
            .and_then(Option::take)
            .ok_or(ProcessEffectError::PidfdUnavailable)
    }
}

#[derive(Clone, Copy)]
enum BrokerOperation {
    OpenPidfd,
    Other,
}

fn response_error(response: &BrokerResponse, operation: BrokerOperation) -> ProcessEffectError {
    match response {
        BrokerResponse::Error(error)
            if error.kind.contains("Pidfd")
                || (matches!(operation, BrokerOperation::OpenPidfd)
                    && error.kind == "Broker.LiveHandlerFailed") =>
        {
            ProcessEffectError::IdentityChanged
        }
        BrokerResponse::Error(_) => ProcessEffectError::LaunchFailed,
        _ => ProcessEffectError::LaunchFailed,
    }
}

#[cfg(test)]
mod tests {
    use d2b_contracts::broker_wire::BrokerErrorResponse;

    use super::*;

    struct Resolver;

    impl BrokerLaunchResolver for Resolver {
        fn resolve(
            &self,
            _request: &ProcessRequest,
        ) -> Result<BrokerLaunchIntent, ProcessEffectError> {
            Err(ProcessEffectError::ResolutionFailed)
        }

        fn observe(
            &self,
            _request: &ProcessRequest,
        ) -> Result<Option<BrokerObservedProcess>, ProcessEffectError> {
            Ok(None)
        }
    }

    fn observed(seed: u16) -> BrokerObservedProcess {
        BrokerObservedProcess {
            intent: BrokerLaunchIntent {
                vm_id: VmId::new("corp-vm"),
                role_id: RoleId::new("worker"),
                role: RunnerRole::Virtiofsd,
                bundle_runner_intent_ref: BundleOpId::new("runner:vm:corp-vm:role:worker"),
                provider_identity: [1; 32],
                template_identity: [2; 32],
                generation: 1,
            },
            pid: i32::from(seed) + 1,
            start_time_ticks: u64::from(seed) + 1,
            cgroup_verified: true,
            executable_verified: true,
        }
    }

    #[test]
    fn pending_broker_observations_are_bounded_and_consumed() {
        let backend =
            BrokerProcessBackend::with_socket(Resolver, "/unused", Duration::from_millis(1));
        for seed in 0..=MAX_PENDING_OBSERVATIONS {
            backend
                .record(observed(u16::try_from(seed).unwrap()))
                .unwrap();
        }
        assert_eq!(
            backend.observations.lock().unwrap().len(),
            MAX_PENDING_OBSERVATIONS
        );
        let identity = observed(u16::try_from(MAX_PENDING_OBSERVATIONS).unwrap()).digest();
        backend.take_observation(&identity).unwrap();
        assert_eq!(
            backend.observations.lock().unwrap().len(),
            MAX_PENDING_OBSERVATIONS - 1
        );
    }

    #[test]
    fn open_pidfd_live_handler_failures_are_identity_changes() {
        let response = BrokerResponse::Error(BrokerErrorResponse {
            kind: "Broker.LiveHandlerFailed".to_owned(),
            operation: "LiveHandler".to_owned(),
            target_wave: None,
            message: "pid 42 changed from start time 100 to 200".to_owned(),
            action: "inspect private audit".to_owned(),
        });
        let error = response_error(&response, BrokerOperation::OpenPidfd);
        assert_eq!(error, ProcessEffectError::IdentityChanged);
        assert_eq!(error.to_string(), "identity-changed");
        assert_eq!(
            response_error(&response, BrokerOperation::Other),
            ProcessEffectError::LaunchFailed
        );
    }

    #[test]
    fn broker_diagnostics_redact_process_identity_values() {
        let process = observed(41);
        assert_eq!(format!("{process:?}"), "BrokerObservedProcess(<redacted>)");
        assert_eq!(
            format!("{:?}", process.intent),
            "BrokerLaunchIntent(<redacted>)"
        );
    }
}

fn read_proc_start_time(pid: i32) -> Result<Option<u64>, ProcessEffectError> {
    let content = match fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(ProcessEffectError::ObserveFailed),
    };
    let close = content
        .trim_end_matches('\n')
        .rfind(')')
        .ok_or(ProcessEffectError::ObserveFailed)?;
    let mut fields = content[close + 1..].split_whitespace();
    let state = fields.next().ok_or(ProcessEffectError::ObserveFailed)?;
    if matches!(state, "Z" | "X") {
        return Ok(None);
    }
    fields
        .nth(18)
        .ok_or(ProcessEffectError::ObserveFailed)?
        .parse::<u64>()
        .map(Some)
        .map_err(|_| ProcessEffectError::ObserveFailed)
}

fn broker_round_trip(
    socket_path: &Path,
    io_timeout: Duration,
    request: BrokerRequest,
) -> Result<BrokerFrame, ProcessEffectError> {
    let fd = socket_with(
        AddressFamily::UNIX,
        SocketType::SEQPACKET,
        SocketFlags::CLOEXEC,
        None,
    )
    .map_err(|_| ProcessEffectError::LaunchFailed)?;
    let socket = Socket::from(fd);
    let address =
        socket2::SockAddr::unix(socket_path).map_err(|_| ProcessEffectError::LaunchFailed)?;
    socket
        .connect_timeout(&address, io_timeout)
        .map_err(|_| ProcessEffectError::LaunchFailed)?;
    socket
        .set_read_timeout(Some(io_timeout))
        .map_err(|_| ProcessEffectError::LaunchFailed)?;
    socket
        .set_write_timeout(Some(io_timeout))
        .map_err(|_| ProcessEffectError::LaunchFailed)?;
    let envelope = BrokerRequestEnvelope {
        request,
        caller_role: Default::default(),
        test_peer_uid: None,
    };
    let frame =
        d2b_contracts::encode_frame(&envelope).map_err(|_| ProcessEffectError::LaunchFailed)?;
    let written = send(&socket, &frame, rustix::net::SendFlags::empty())
        .map_err(|_| ProcessEffectError::LaunchFailed)?;
    if written != frame.len() {
        return Err(ProcessEffectError::LaunchFailed);
    }

    let mut payload = vec![0_u8; d2b_contracts::MAX_FRAME_SIZE + 4];
    let mut iov = [IoSliceMut::new(&mut payload)];
    let mut control_bytes = vec![0_u8; rustix::cmsg_space!(ScmRights(8))];
    let mut control = RecvAncillaryBuffer::new(&mut control_bytes);
    let message = recvmsg(&socket, &mut iov, &mut control, RecvFlags::CMSG_CLOEXEC)
        .map_err(|_| ProcessEffectError::LaunchFailed)?;
    let bytes = message.bytes;
    let mut fds = Vec::new();
    for message in control.drain() {
        if let RecvAncillaryMessage::ScmRights(received) = message {
            for owned in received {
                fds.push(Some(owned));
            }
        }
    }
    let response = d2b_contracts::decode_frame("BrokerResponse", &payload[..bytes])
        .map_err(|_| ProcessEffectError::LaunchFailed)?;
    Ok(BrokerFrame {
        response,
        fds: Mutex::new(fds),
    })
}
