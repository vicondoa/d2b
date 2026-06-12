//! Production guest-control transport bridge.
//!
//! Wires the host daemon's authenticated guest-control probe
//! ([`crate::guest_control_health`]) to a real broker-backed signer and
//! the per-VM vsock socket. The W11 layer shipped the probe, the ttRPC
//! client, the vsock connector, and the broker's HMAC signing op, but no
//! production [`GuestControlSigner`] and nothing that drives the probe.
//! This module supplies both:
//!
//! * [`BrokerSigner`] — the production signer. It forwards the
//!   probe-built [`GuestControlSignRequest`] verbatim to the privileged
//!   broker and returns the broker-minted tag. It owns only the broker
//!   socket path so it is `Send + Sync` and can move into the blocking
//!   probe worker without borrowing `ServerState`.
//! * an orchestration seam ([`GuestControlProbe`]) plus its production
//!   implementation ([`RealGuestControlProbe`]) that resolves the
//!   connection, builds the ttRPC client, and runs the probe / config
//!   read on a dedicated current-thread runtime.
//!
//! Runtime boundary: every probe runs inside a fresh
//! `new_current_thread().enable_all()` runtime with owned parameters.
//! Callers on the multi-threaded daemon runtime MUST invoke the probe
//! from `tokio::task::spawn_blocking` — never `Handle::current()`,
//! `block_in_place`, or a nested runtime.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use nixling_ipc::broker_wire::{
    BrokerCallerRole, BrokerRequest, BrokerResponse, GuestControlSignRequest,
    GuestControlSignResponse,
};
use nixling_ipc::guest_auth::AUTH_NONCE_LEN;

use crate::guest_control_health::{
    connected_stream_to_ttrpc_socket, guest_control_health_ready, probe_guest_control_health,
    read_guest_config_authenticated, GuestControlHealthError, GuestControlHealthEvidence,
    GuestControlSigner, GuestFileReadError, TtrpcGuestControlClient,
};
use crate::guest_control_vsock::{connect_guest_control_vsock, GuestControlTransportProbeResult};
use crate::typed_error::TypedError;

/// Well-known `VMADDR_CID_HOST`. The host side of an `AF_VSOCK` pair is
/// always CID 2; the sign request binds the host proof to this CID so a
/// captured proof cannot be replayed from a different CID.
pub const VMADDR_CID_HOST: u32 = libc::VMADDR_CID_HOST;

/// Per-attempt cap applied to connect / CONNECT-ACK / each ttRPC / each
/// broker-sign. The effective per-attempt timeout is
/// `min(this, remaining_deadline)`.
pub const GUEST_CONTROL_ATTEMPT_CAP: Duration = Duration::from_secs(3);

/// Backoff between readiness-loop attempts while the guest is still
/// booting / the socket is not yet present.
pub const GUEST_CONTROL_RETRY_BACKOFF: Duration = Duration::from_millis(250);

/// Single-attempt timeout for the config-read verb (the VM is already
/// up by the time config-sync runs, so no readiness retry loop).
pub const GUEST_CONTROL_CONFIG_READ_TIMEOUT: Duration = Duration::from_secs(10);

/// 32 fresh CSPRNG bytes for the host nonce. No time-seeded fallback:
/// an entropy failure fails the probe closed.
pub fn host_nonce() -> Result<[u8; AUTH_NONCE_LEN], getrandom::Error> {
    let mut nonce = [0u8; AUTH_NONCE_LEN];
    getrandom::getrandom(&mut nonce)?;
    Ok(nonce)
}

/// Map a broker dispatch result for a `GuestControlSign` request to the
/// signer's typed result. Any transport error (incl. timeout / refusal),
/// a broker `Error` response, or any non-`GuestControlSign` response
/// collapses to [`GuestControlHealthError::Signer`]. Extracted as a pure
/// function so the mapping is unit-testable without a live broker.
fn map_broker_sign_response(
    result: Result<BrokerResponse, TypedError>,
) -> Result<GuestControlSignResponse, GuestControlHealthError> {
    match result {
        Ok(BrokerResponse::GuestControlSign(response)) => Ok(response),
        Ok(_) | Err(_) => Err(GuestControlHealthError::Signer),
    }
}

/// Production [`GuestControlSigner`] backed by the privileged broker.
///
/// Holds only the broker socket path and a per-call timeout so it is
/// `Send + Sync` and movable into the blocking probe worker. `sign`
/// forwards the probe-built request verbatim — it never mints nonces,
/// roles, or boot ids; the broker remains the sole minter of the tag.
#[derive(Clone)]
pub struct BrokerSigner {
    broker_socket_path: PathBuf,
    timeout: Duration,
}

impl BrokerSigner {
    pub fn new(broker_socket_path: PathBuf, timeout: Duration) -> Self {
        Self {
            broker_socket_path,
            timeout,
        }
    }
}

impl GuestControlSigner for BrokerSigner {
    fn sign(
        &self,
        request: GuestControlSignRequest,
    ) -> Result<GuestControlSignResponse, GuestControlHealthError> {
        let result = crate::dispatch_broker_request_to_socket(
            &self.broker_socket_path,
            BrokerRequest::GuestControlSign(request),
            BrokerCallerRole::default(),
            Some(self.timeout),
        );
        map_broker_sign_response(result)
    }
}

/// Fully-resolved, owned parameters for one guest-control probe /
/// config read. Every field is owned so the struct can move into the
/// blocking probe worker without borrowing `ServerState`.
#[derive(Clone, Debug)]
pub struct ProbeParams {
    pub vm_id: String,
    pub socket_path: PathBuf,
    pub state_root: PathBuf,
    pub expected_state_root_uid: u32,
    pub expected_state_root_gid: u32,
    pub expected_peer_uid: u32,
    pub expected_peer_gid: u32,
}

/// Seam over the orchestration so the readiness loop and the config-sync
/// verb can be unit-tested with scripted outcomes without a live guest.
pub trait GuestControlProbe: Send + Sync {
    fn probe_health(
        &self,
        params: &ProbeParams,
        attempt_timeout: Duration,
    ) -> Result<GuestControlHealthEvidence, GuestControlHealthError>;

    fn read_config(
        &self,
        params: &ProbeParams,
        attempt_timeout: Duration,
    ) -> Result<Vec<u8>, GuestFileReadError>;
}

/// Production probe: connects the vsock socket, builds the ttRPC client,
/// and runs the authenticated probe / config read on a dedicated
/// current-thread runtime. Owns only the broker socket path.
pub struct RealGuestControlProbe {
    broker_socket_path: PathBuf,
}

impl RealGuestControlProbe {
    pub fn new(broker_socket_path: PathBuf) -> Self {
        Self { broker_socket_path }
    }
}

impl GuestControlProbe for RealGuestControlProbe {
    fn probe_health(
        &self,
        params: &ProbeParams,
        attempt_timeout: Duration,
    ) -> Result<GuestControlHealthEvidence, GuestControlHealthError> {
        run_health_probe_once(params, &self.broker_socket_path, attempt_timeout)
    }

    fn read_config(
        &self,
        params: &ProbeParams,
        attempt_timeout: Duration,
    ) -> Result<Vec<u8>, GuestFileReadError> {
        run_config_read_once(params, &self.broker_socket_path, attempt_timeout)
    }
}

fn build_probe_runtime() -> Result<tokio::runtime::Runtime, GuestControlHealthError> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| GuestControlHealthError::TransportIo)
}

/// Synchronously connect the per-VM vsock socket and wrap it in a ttRPC
/// client. MUST be called inside the probe runtime: the returned client
/// holds a `tokio::net::UnixStream` that needs the reactor.
fn connect_and_build_client(
    params: &ProbeParams,
    attempt_timeout: Duration,
) -> Result<TtrpcGuestControlClient, GuestControlHealthError> {
    let connected = match connect_guest_control_vsock(
        &params.socket_path,
        &params.state_root,
        params.expected_state_root_uid,
        params.expected_state_root_gid,
        params.expected_peer_uid,
        params.expected_peer_gid,
        attempt_timeout,
    ) {
        GuestControlTransportProbeResult::Connected(connected) => connected,
        GuestControlTransportProbeResult::Failed(_) => {
            return Err(GuestControlHealthError::TransportIo);
        }
    };
    let socket = connected_stream_to_ttrpc_socket(connected)?;
    Ok(TtrpcGuestControlClient::new(socket, attempt_timeout))
}

/// One authenticated Health probe attempt. Builds a fresh host nonce, a
/// per-attempt broker signer, and a dedicated current-thread runtime.
pub fn run_health_probe_once(
    params: &ProbeParams,
    broker_socket_path: &Path,
    attempt_timeout: Duration,
) -> Result<GuestControlHealthEvidence, GuestControlHealthError> {
    let signer = BrokerSigner::new(broker_socket_path.to_path_buf(), attempt_timeout);
    let nonce = host_nonce().map_err(|_| GuestControlHealthError::Signer)?;
    let runtime = build_probe_runtime()?;
    runtime.block_on(async {
        let client = connect_and_build_client(params, attempt_timeout)?;
        probe_guest_control_health(
            &params.vm_id,
            Some(VMADDR_CID_HOST),
            nonce,
            &client,
            &signer,
        )
        .await
    })
}

/// One authenticated config-read attempt over the same handshake.
pub fn run_config_read_once(
    params: &ProbeParams,
    broker_socket_path: &Path,
    attempt_timeout: Duration,
) -> Result<Vec<u8>, GuestFileReadError> {
    let signer = BrokerSigner::new(broker_socket_path.to_path_buf(), attempt_timeout);
    let nonce = host_nonce()
        .map_err(|_| GuestFileReadError::Probe(GuestControlHealthError::Signer))?;
    let runtime = build_probe_runtime().map_err(GuestFileReadError::Probe)?;
    runtime.block_on(async {
        let client =
            connect_and_build_client(params, attempt_timeout).map_err(GuestFileReadError::Probe)?;
        read_guest_config_authenticated(
            &params.vm_id,
            Some(VMADDR_CID_HOST),
            nonce,
            &client,
            &signer,
        )
        .await
    })
}

/// Run a single config read on a DEDICATED OS thread so the probe's
/// current-thread runtime is never nested inside a caller's Tokio runtime
/// (the public.sock dispatch path runs synchronously on a multi-threaded
/// runtime worker; calling `Runtime::block_on` there would panic). This is the
/// BR13 runtime boundary for the synchronous verb path: nothing is borrowed
/// across the thread, and the spawned thread starts with no runtime context.
pub fn run_config_read_on_dedicated_thread(
    params: ProbeParams,
    broker_socket_path: PathBuf,
    timeout: Duration,
) -> Result<Vec<u8>, GuestFileReadError> {
    std::thread::spawn(move || run_config_read_once(&params, &broker_socket_path, timeout))
        .join()
        .map_err(|_| GuestFileReadError::Probe(GuestControlHealthError::TransportIo))?
}

/// Injectable clock for deterministic retry-loop tests. The real
/// implementation uses a monotonic `Instant` and `thread::sleep`; fakes
/// advance a logical clock on `sleep`.
pub trait ProbeClock {
    fn elapsed(&self) -> Duration;
    fn sleep(&self, duration: Duration);
}

pub struct RealProbeClock {
    start: Instant,
}

impl RealProbeClock {
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
        }
    }
}

impl Default for RealProbeClock {
    fn default() -> Self {
        Self::new()
    }
}

impl ProbeClock for RealProbeClock {
    fn elapsed(&self) -> Duration {
        self.start.elapsed()
    }

    fn sleep(&self, duration: Duration) {
        std::thread::sleep(duration);
    }
}

/// State-aware guest-control readiness loop. Retries the authenticated
/// Health probe until [`guest_control_health_ready`] returns true or the
/// `deadline` elapses, applying a per-attempt timeout of
/// `min(attempt_cap, remaining_deadline)` to connect / CONNECT-ACK /
/// ttRPC / broker-sign. Fails CLOSED: on deadline it returns the last
/// (not-ready) outcome.
pub fn run_guest_control_readiness_loop(
    probe: &dyn GuestControlProbe,
    params: &ProbeParams,
    deadline: Duration,
    attempt_cap: Duration,
    retry_backoff: Duration,
    clock: &dyn ProbeClock,
) -> Result<GuestControlHealthEvidence, GuestControlHealthError> {
    loop {
        let remaining = deadline.saturating_sub(clock.elapsed());
        let attempt_timeout = attempt_cap
            .min(remaining)
            .max(Duration::from_millis(1));
        let outcome = probe.probe_health(params, attempt_timeout);
        if guest_control_health_ready(&outcome) {
            return outcome;
        }
        // Stop if there is no room for another attempt + backoff before
        // the deadline. Returns the last not-ready outcome.
        if clock.elapsed().saturating_add(retry_backoff) >= deadline {
            return outcome;
        }
        clock.sleep(retry_backoff);
    }
}

/// Closed-enum label for the guest-reported health state of a probe
/// outcome. Used as a metric/span label, so the range is a small fixed
/// vocabulary — never free-form text and never guest-supplied content.
pub fn health_state_label(evidence: &GuestControlHealthEvidence) -> &'static str {
    use nixling_ipc::guest_proto::HealthState;
    match evidence.health.state.enum_value() {
        Ok(HealthState::HEALTH_STATE_HEALTHY) => "healthy",
        Ok(HealthState::HEALTH_STATE_DEGRADED) => "degraded",
        Ok(HealthState::HEALTH_STATE_UNAVAILABLE_OLD_GENERATION) => "unavailable-old-generation",
        Ok(HealthState::HEALTH_STATE_LISTENER_ABSENT) => "listener-absent",
        Ok(HealthState::HEALTH_STATE_TRANSPORT_UNREACHABLE) => "transport-unreachable",
        Ok(HealthState::HEALTH_STATE_AUTH_FAILED) => "auth-failed",
        Ok(HealthState::HEALTH_STATE_PROTOCOL_MISMATCH) => "protocol-mismatch",
        Ok(HealthState::HEALTH_STATE_STALE_SESSION) => "stale-session",
        Ok(HealthState::HEALTH_STATE_UNSPECIFIED) | Err(_) => "unspecified",
    }
}

/// Closed-enum label for a guest-control probe error. Used as a
/// metric/span label, so the range is a small fixed vocabulary.
pub fn error_kind_label(error: &GuestControlHealthError) -> &'static str {
    match error {
        GuestControlHealthError::TransportIo => "transport-io",
        GuestControlHealthError::Ttrpc => "ttrpc",
        GuestControlHealthError::Signer => "signer",
        GuestControlHealthError::Protocol => "protocol",
        GuestControlHealthError::AuthFailed => "auth-failed",
        GuestControlHealthError::StaleSession => "stale-session",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nixling_ipc::broker_wire::{
        BrokerErrorResponse, GuestControlProofRole, GuestControlSignRequest,
    };
    use nixling_ipc::guest_auth::AUTH_TAG_LEN;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Mutex;

    /// Records every `GuestControlSignRequest` the probe forwards so the
    /// test can assert the host built it verbatim (correct CID, roles,
    /// nonces, boot id).
    struct RecordingSigner {
        recorded: Mutex<Vec<GuestControlSignRequest>>,
        fail: bool,
    }

    impl RecordingSigner {
        fn new(fail: bool) -> Self {
            Self {
                recorded: Mutex::new(Vec::new()),
                fail,
            }
        }
    }

    impl GuestControlSigner for RecordingSigner {
        fn sign(
            &self,
            request: GuestControlSignRequest,
        ) -> Result<GuestControlSignResponse, GuestControlHealthError> {
            self.recorded.lock().unwrap().push(request.clone());
            if self.fail {
                return Err(GuestControlHealthError::Signer);
            }
            let fill = match request.role {
                GuestControlProofRole::HostProof => 0x55,
                GuestControlProofRole::GuestProof => 0x77,
            };
            Ok(GuestControlSignResponse {
                tag: vec![fill; AUTH_TAG_LEN],
            })
        }
    }

    #[test]
    fn broker_sign_response_mapping_is_fail_closed() {
        // Happy path: a GuestControlSign response forwards through.
        let ok = map_broker_sign_response(Ok(BrokerResponse::GuestControlSign(
            GuestControlSignResponse {
                tag: vec![0u8; AUTH_TAG_LEN],
            },
        )));
        assert!(matches!(ok, Ok(resp) if resp.tag.len() == AUTH_TAG_LEN));

        // Broker Error response -> Signer.
        let broker_error = map_broker_sign_response(Ok(BrokerResponse::Error(BrokerErrorResponse {
            kind: "guest-control-auth".to_owned(),
            operation: "GuestControlSign".to_owned(),
            target_wave: None,
            message: "refused".to_owned(),
            action: "n/a".to_owned(),
        })));
        assert_eq!(broker_error, Err(GuestControlHealthError::Signer));

        // Wrong (non-sign) response variant -> Signer.
        let wrong = map_broker_sign_response(Ok(BrokerResponse::PollChildReaped(
            nixling_ipc::broker_wire::PollChildReapedResponse {
                notifications: vec![],
            },
        )));
        assert_eq!(wrong, Err(GuestControlHealthError::Signer));

        // Transport/timeout error -> Signer.
        let timeout = map_broker_sign_response(Err(TypedError::InternalIo {
            context: "recv seqpacket frame".to_owned(),
            detail: "timed out".to_owned(),
        }));
        assert_eq!(timeout, Err(GuestControlHealthError::Signer));
    }

    #[test]
    fn broker_signer_maps_missing_broker_to_signer_error() {
        // A signer pointed at a non-existent broker socket fails the
        // connect and must map to a Signer error (fail closed).
        let signer = BrokerSigner::new(
            PathBuf::from("/nonexistent-nixling-broker.sock"),
            Duration::from_millis(50),
        );
        let request = GuestControlSignRequest {
            vm_id: nixling_ipc::types::VmId::new("corp-vm"),
            role: GuestControlProofRole::HostProof,
            protocol_version: nixling_ipc::guest_wire::GUEST_CONTROL_PROTOCOL_VERSION,
            direction: nixling_ipc::broker_wire::GuestControlDirection::HostToGuest,
            purpose: nixling_ipc::broker_wire::GuestControlAuthPurpose::GuestControlAuthV1,
            guest_control_port: nixling_ipc::guest_auth::GUEST_CONTROL_AUTH_PORT,
            peer_cid: Some(VMADDR_CID_HOST),
            host_nonce: vec![0x11; AUTH_NONCE_LEN],
            guest_nonce: vec![0x22; AUTH_NONCE_LEN],
            guest_boot_id: nixling_ipc::broker_wire::GuestBootIdWire::new("boot-1"),
            capabilities_hash: None,
            tracing_span_id: None,
        };
        assert_eq!(signer.sign(request), Err(GuestControlHealthError::Signer));
    }

    #[test]
    fn host_nonce_is_fresh_and_full_length() {
        let a = host_nonce().expect("entropy");
        let b = host_nonce().expect("entropy");
        assert_eq!(a.len(), AUTH_NONCE_LEN);
        // Two draws must (overwhelmingly likely) differ; a constant draw
        // would indicate a broken CSPRNG.
        assert_ne!(a, b);
    }

    /// Build a minimal Healthy evidence so the readiness loop's
    /// ready-decision can be exercised without a live guest.
    fn healthy_evidence() -> GuestControlHealthEvidence {
        use nixling_ipc::guest_proto as pb;
        let mut health = pb::HealthResponse::new();
        health.origin =
            protobuf::EnumOrUnknown::new(pb::HealthOrigin::HEALTH_ORIGIN_GUEST_REPORTED);
        health.state = protobuf::EnumOrUnknown::new(pb::HealthState::HEALTH_STATE_HEALTHY);
        health.reason = protobuf::EnumOrUnknown::new(pb::HealthReason::HEALTH_REASON_NONE);
        health.remediation =
            protobuf::EnumOrUnknown::new(pb::HealthRemediation::HEALTH_REMEDIATION_NONE);
        health.protocol_version = nixling_ipc::guest_wire::GUEST_CONTROL_PROTOCOL_VERSION;
        GuestControlHealthEvidence {
            vm_id: "corp-vm".to_owned(),
            guest_boot_id: "boot-1".to_owned(),
            protocol_version: nixling_ipc::guest_wire::GUEST_CONTROL_PROTOCOL_VERSION,
            capabilities_hash: "caps-sha256".to_owned(),
            health,
        }
    }

    fn test_params() -> ProbeParams {
        ProbeParams {
            vm_id: "corp-vm".to_owned(),
            socket_path: PathBuf::from("/var/lib/nixling/vms/corp-vm/vsock.sock"),
            state_root: PathBuf::from("/var/lib/nixling/vms/corp-vm"),
            expected_state_root_uid: 990,
            expected_state_root_gid: 100,
            expected_peer_uid: 31000,
            expected_peer_gid: 31000,
        }
    }

    /// Fake clock that advances a logical millisecond counter on sleep,
    /// so the retry loop's deadline behaviour is fully deterministic.
    struct FakeClock {
        elapsed_ms: AtomicU64,
    }

    impl FakeClock {
        fn new() -> Self {
            Self {
                elapsed_ms: AtomicU64::new(0),
            }
        }
    }

    impl ProbeClock for FakeClock {
        fn elapsed(&self) -> Duration {
            Duration::from_millis(self.elapsed_ms.load(Ordering::SeqCst))
        }

        fn sleep(&self, duration: Duration) {
            self.elapsed_ms
                .fetch_add(duration.as_millis() as u64, Ordering::SeqCst);
        }
    }

    /// Probe that returns a scripted sequence of outcomes, recording the
    /// per-attempt timeout passed by the loop.
    struct ScriptedProbe {
        outcomes: Mutex<Vec<Result<GuestControlHealthEvidence, GuestControlHealthError>>>,
        attempt_timeouts: Mutex<Vec<Duration>>,
    }

    impl ScriptedProbe {
        fn new(outcomes: Vec<Result<GuestControlHealthEvidence, GuestControlHealthError>>) -> Self {
            Self {
                outcomes: Mutex::new(outcomes),
                attempt_timeouts: Mutex::new(Vec::new()),
            }
        }
    }

    impl GuestControlProbe for ScriptedProbe {
        fn probe_health(
            &self,
            _params: &ProbeParams,
            attempt_timeout: Duration,
        ) -> Result<GuestControlHealthEvidence, GuestControlHealthError> {
            self.attempt_timeouts.lock().unwrap().push(attempt_timeout);
            let mut outcomes = self.outcomes.lock().unwrap();
            if outcomes.is_empty() {
                // Past the script: keep returning the persistent failure.
                return Err(GuestControlHealthError::TransportIo);
            }
            outcomes.remove(0)
        }

        fn read_config(
            &self,
            _params: &ProbeParams,
            _attempt_timeout: Duration,
        ) -> Result<Vec<u8>, GuestFileReadError> {
            unreachable!("readiness loop never reads config")
        }
    }

    #[test]
    fn readiness_loop_succeeds_after_transient_failures() {
        let probe = ScriptedProbe::new(vec![
            Err(GuestControlHealthError::TransportIo),
            Err(GuestControlHealthError::Ttrpc),
            Ok(healthy_evidence()),
        ]);
        let clock = FakeClock::new();
        let outcome = run_guest_control_readiness_loop(
            &probe,
            &test_params(),
            Duration::from_secs(30),
            GUEST_CONTROL_ATTEMPT_CAP,
            GUEST_CONTROL_RETRY_BACKOFF,
            &clock,
        );
        assert!(guest_control_health_ready(&outcome));
        // Three attempts: two transient, one healthy.
        assert_eq!(probe.attempt_timeouts.lock().unwrap().len(), 3);
    }

    #[test]
    fn readiness_loop_per_attempt_timeout_is_capped() {
        let probe = ScriptedProbe::new(vec![Ok(healthy_evidence())]);
        let clock = FakeClock::new();
        let _ = run_guest_control_readiness_loop(
            &probe,
            &test_params(),
            Duration::from_secs(30),
            GUEST_CONTROL_ATTEMPT_CAP,
            GUEST_CONTROL_RETRY_BACKOFF,
            &clock,
        );
        // With a 30s deadline and a 3s cap, the per-attempt timeout is
        // the cap, not the full remaining deadline.
        let timeouts = probe.attempt_timeouts.lock().unwrap();
        assert_eq!(timeouts[0], GUEST_CONTROL_ATTEMPT_CAP);
    }

    #[test]
    fn readiness_loop_persistent_failure_hits_deadline_and_fails_closed() {
        // Empty script -> ScriptedProbe yields persistent TransportIo.
        let probe = ScriptedProbe::new(vec![]);
        let clock = FakeClock::new();
        let outcome = run_guest_control_readiness_loop(
            &probe,
            &test_params(),
            Duration::from_secs(2),
            GUEST_CONTROL_ATTEMPT_CAP,
            GUEST_CONTROL_RETRY_BACKOFF,
            &clock,
        );
        assert!(!guest_control_health_ready(&outcome));
        // The fake clock advanced past the deadline via backoff sleeps.
        assert!(clock.elapsed() >= Duration::from_secs(2) - GUEST_CONTROL_RETRY_BACKOFF);
        // Many attempts occurred before giving up (2s / 250ms backoff).
        assert!(probe.attempt_timeouts.lock().unwrap().len() >= 5);
    }

    #[tokio::test]
    async fn probe_forwards_sign_requests_verbatim_with_host_cid() {
        // Drive the real probe with a local happy fake client via a
        // recording signer to assert the host built each
        // GuestControlSignRequest verbatim (CID 2, HostProof then
        // GuestProof, matching nonces + boot id).
        use crate::guest_control_health::probe_guest_control_health;

        let signer = RecordingSigner::new(false);
        let host_nonce = [0x11u8; AUTH_NONCE_LEN];
        let evidence = probe_guest_control_health(
            "corp-vm",
            Some(VMADDR_CID_HOST),
            host_nonce,
            &HappyFakeClient,
            &signer,
        )
        .await
        .expect("probe succeeds");
        assert_eq!(evidence.vm_id, "corp-vm");

        let recorded = signer.recorded.lock().unwrap();
        assert_eq!(recorded.len(), 2, "host + guest proof signed");
        assert_eq!(recorded[0].role, GuestControlProofRole::HostProof);
        assert_eq!(recorded[1].role, GuestControlProofRole::GuestProof);
        for request in recorded.iter() {
            assert_eq!(request.peer_cid, Some(VMADDR_CID_HOST));
            assert_eq!(request.host_nonce, host_nonce.to_vec());
            assert_eq!(request.guest_boot_id.as_str(), "boot-1");
            assert_eq!(
                request.protocol_version,
                nixling_ipc::guest_wire::GUEST_CONTROL_PROTOCOL_VERSION
            );
        }
        // The GuestProof carries the capabilities hash; the HostProof
        // does not.
        assert!(recorded[0].capabilities_hash.is_none());
        assert_eq!(
            recorded[1].capabilities_hash.as_deref(),
            Some("caps-sha256")
        );
    }

    #[tokio::test]
    async fn probe_maps_signer_failure_to_signer_error() {
        use crate::guest_control_health::probe_guest_control_health;
        let signer = RecordingSigner::new(true);
        let outcome = probe_guest_control_health(
            "corp-vm",
            Some(VMADDR_CID_HOST),
            [0x11u8; AUTH_NONCE_LEN],
            &HappyFakeClient,
            &signer,
        )
        .await;
        assert!(matches!(outcome, Err(GuestControlHealthError::Signer)));
    }

    /// Minimal happy-path RPC fake for the verbatim-forward tests. The
    /// guest tag (0x77) matches `RecordingSigner`'s GuestProof fill so
    /// the constant-time tag comparison in the probe passes.
    struct HappyFakeClient;

    #[async_trait::async_trait]
    impl crate::guest_control_health::GuestControlRpc for HappyFakeClient {
        async fn hello(
            &self,
            _request: nixling_ipc::guest_proto::HelloRequest,
        ) -> Result<nixling_ipc::guest_proto::HelloResponse, GuestControlHealthError> {
            use nixling_ipc::guest_proto as pb;
            let mut response = pb::HelloResponse::new();
            response.guest_nonce = vec![0x22; AUTH_NONCE_LEN];
            response.guest_boot_id = "boot-1".to_owned();
            response.protocol_version = nixling_ipc::guest_wire::GUEST_CONTROL_PROTOCOL_VERSION;
            Ok(response)
        }

        async fn authenticate(
            &self,
            _request: nixling_ipc::guest_proto::AuthenticateRequest,
        ) -> Result<nixling_ipc::guest_proto::AuthenticateResponse, GuestControlHealthError>
        {
            use nixling_ipc::guest_proto as pb;
            let mut response = pb::AuthenticateResponse::new();
            response.guest_auth_tag = Some(vec![0x77; AUTH_TAG_LEN]);
            response.capabilities_hash = Some("caps-sha256".to_owned());
            Ok(response)
        }

        async fn health(
            &self,
            _request: nixling_ipc::guest_proto::HealthRequest,
        ) -> Result<nixling_ipc::guest_proto::HealthResponse, GuestControlHealthError> {
            Ok(healthy_evidence().health)
        }

        async fn read_guest_file(
            &self,
            _request: nixling_ipc::guest_proto::ReadGuestFileRequest,
        ) -> Result<nixling_ipc::guest_proto::ReadGuestFileResponse, GuestControlHealthError>
        {
            Ok(nixling_ipc::guest_proto::ReadGuestFileResponse::new())
        }
    }
}
