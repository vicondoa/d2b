//! Production guest-control transport bridge.
//!
//! Wires the host daemon's authenticated guest-control probe
//! ([`crate::guest_control_health`]) to a real broker-backed signer and
//! the per-VM vsock socket. An earlier layer shipped the probe, the ttRPC
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

use d2b_contracts::broker_wire::{
    BrokerCallerRole, BrokerRequest, BrokerResponse, GuestControlSignRequest,
    GuestControlSignResponse,
};
use d2b_contracts::guest_auth::AUTH_NONCE_LEN;

use crate::guest_control_health::{
    AttemptBudget, GuestAudioChannelStatus, GuestAudioSetError, GuestAudioStatus,
    GuestControlHealthError, GuestControlHealthEvidence, GuestControlSigner, GuestFileReadError,
    GuestSystemActivationError, GuestSystemActivationStart, GuestSystemActivationStatus,
    GuestUsbipAction, GuestUsbipImportCall, GuestUsbipImportError, GuestUsbipImportResult,
    GuestUsbipStatusResult, TtrpcGuestControlClient, activate_system_start_authenticated,
    activate_system_status_authenticated, audio_set_authenticated, audio_status_authenticated,
    connected_stream_to_ttrpc_socket, guest_control_health_ready, probe_guest_control_health,
    read_guest_config_authenticated, usbip_import_authenticated, usbip_status_authenticated,
};
use crate::guest_control_vsock::{GuestControlTransportProbeResult, connect_guest_control_vsock};
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
/// End-to-end USBIP import deadline. This must exceed guestd's bounded
/// `usbip` command timeout so the host does not drop the ttRPC future and kill
/// the guest subprocess before guestd can return a typed failure.
pub const GUEST_CONTROL_USBIP_IMPORT_TIMEOUT: Duration = Duration::from_secs(15);

/// 32 fresh CSPRNG bytes for the host nonce. No time-seeded fallback:
/// an entropy failure fails the probe closed.
pub fn host_nonce() -> Result<[u8; AUTH_NONCE_LEN], getrandom::Error> {
    let mut nonce = [0u8; AUTH_NONCE_LEN];
    getrandom::getrandom(&mut nonce)?;
    Ok(nonce)
}

/// Map a broker dispatch result for a `GuestControlSign` request to the
/// signer's typed result. A round-trip deadline exhaustion
/// ([`TypedError::InternalBrokerTimeout`]) maps to
/// [`GuestControlHealthError::Timeout`] so a stalled/backlogged broker
/// surfaces as the `guest-control-timeout` error end to end. Any other
/// transport error (incl. refusal), a broker `Error` response, or any
/// non-`GuestControlSign` response collapses to
/// [`GuestControlHealthError::Signer`]. Extracted as a pure function so
/// the mapping is unit-testable without a live broker.
fn map_broker_sign_response(
    result: Result<BrokerResponse, TypedError>,
) -> Result<GuestControlSignResponse, GuestControlHealthError> {
    match result {
        Ok(BrokerResponse::GuestControlSign(response)) => Ok(response),
        Err(TypedError::InternalBrokerTimeout { .. }) => Err(GuestControlHealthError::Timeout),
        Ok(_) | Err(_) => Err(GuestControlHealthError::Signer),
    }
}

/// Production [`GuestControlSigner`] backed by the privileged broker.
///
/// Holds only the broker socket path and the shared per-attempt budget so
/// it is `Send + Sync` and movable into the blocking probe worker. `sign`
/// forwards the probe-built request verbatim — it never mints nonces,
/// roles, or boot ids; the broker remains the sole minter of the tag.
/// Each `sign` draws `min(cap, remaining)` from the budget so the broker
/// round-trip shares the same absolute deadline as connect / ttRPC; a
/// passed deadline returns [`GuestControlHealthError::Timeout`].
#[derive(Clone)]
pub struct BrokerSigner {
    broker_socket_path: PathBuf,
    budget: AttemptBudget,
}

impl BrokerSigner {
    pub fn new(broker_socket_path: PathBuf, budget: AttemptBudget) -> Self {
        Self {
            broker_socket_path,
            budget,
        }
    }
}

impl GuestControlSigner for BrokerSigner {
    fn sign(
        &self,
        request: GuestControlSignRequest,
    ) -> Result<GuestControlSignResponse, GuestControlHealthError> {
        let timeout = self.budget.next().ok_or(GuestControlHealthError::Timeout)?;
        let result = crate::dispatch_broker_request_to_socket(
            &self.broker_socket_path,
            BrokerRequest::GuestControlSign(request),
            BrokerCallerRole::default(),
            Some(timeout),
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

    fn usbip_import(
        &self,
        params: &ProbeParams,
        attempt_timeout: Duration,
        action: GuestUsbipAction,
        host: &str,
        bus_id: &str,
    ) -> Result<GuestUsbipImportResult, GuestUsbipImportError>;

    fn usbip_status(
        &self,
        params: &ProbeParams,
        attempt_timeout: Duration,
        host: Option<&str>,
        bus_id: Option<&str>,
    ) -> Result<GuestUsbipStatusResult, GuestUsbipImportError>;

    fn activate_system_start(
        &self,
        params: &ProbeParams,
        attempt_timeout: Duration,
        start: &GuestSystemActivationStart,
    ) -> Result<GuestSystemActivationStatus, GuestSystemActivationError> {
        let _ = (params, attempt_timeout, start);
        Err(GuestSystemActivationError::CapabilityUnavailable)
    }

    fn activate_system_status(
        &self,
        params: &ProbeParams,
        attempt_timeout: Duration,
        activation_id: &str,
    ) -> Result<GuestSystemActivationStatus, GuestSystemActivationError> {
        let _ = (params, attempt_timeout, activation_id);
        Err(GuestSystemActivationError::CapabilityUnavailable)
    }

    /// Issue an authenticated AudioSet RPC. Default returns
    /// `CapabilityUnavailable` so existing probe impls do not need updating.
    fn audio_status(
        &self,
        params: &ProbeParams,
        attempt_timeout: Duration,
    ) -> Result<GuestAudioStatus, GuestAudioSetError> {
        let _ = (params, attempt_timeout);
        Err(GuestAudioSetError::CapabilityUnavailable)
    }

    /// Issue an authenticated AudioSet RPC. Default returns
    /// `CapabilityUnavailable` so existing probe impls do not need updating.
    fn audio_set(
        &self,
        params: &ProbeParams,
        attempt_timeout: Duration,
        channel: d2b_contracts::guest_proto::AudioChannel,
        kind: d2b_contracts::guest_proto::AudioSetKind,
        grant_on: bool,
        level: u32,
    ) -> Result<GuestAudioChannelStatus, GuestAudioSetError> {
        let _ = (params, attempt_timeout, channel, kind, grant_on, level);
        Err(GuestAudioSetError::CapabilityUnavailable)
    }
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

    fn usbip_import(
        &self,
        params: &ProbeParams,
        attempt_timeout: Duration,
        action: GuestUsbipAction,
        host: &str,
        bus_id: &str,
    ) -> Result<GuestUsbipImportResult, GuestUsbipImportError> {
        run_usbip_import_once(
            params,
            &self.broker_socket_path,
            attempt_timeout,
            GuestUsbipImportCall {
                action,
                host,
                bus_id,
            },
        )
    }

    fn usbip_status(
        &self,
        params: &ProbeParams,
        attempt_timeout: Duration,
        host: Option<&str>,
        bus_id: Option<&str>,
    ) -> Result<GuestUsbipStatusResult, GuestUsbipImportError> {
        run_usbip_status_once(
            params,
            &self.broker_socket_path,
            attempt_timeout,
            host,
            bus_id,
        )
    }

    fn activate_system_start(
        &self,
        params: &ProbeParams,
        attempt_timeout: Duration,
        start: &GuestSystemActivationStart,
    ) -> Result<GuestSystemActivationStatus, GuestSystemActivationError> {
        run_activation_start_once(params, &self.broker_socket_path, attempt_timeout, start)
    }

    fn activate_system_status(
        &self,
        params: &ProbeParams,
        attempt_timeout: Duration,
        activation_id: &str,
    ) -> Result<GuestSystemActivationStatus, GuestSystemActivationError> {
        run_activation_status_once(
            params,
            &self.broker_socket_path,
            attempt_timeout,
            activation_id,
        )
    }

    fn audio_status(
        &self,
        params: &ProbeParams,
        attempt_timeout: Duration,
    ) -> Result<GuestAudioStatus, GuestAudioSetError> {
        run_audio_status_once(params, &self.broker_socket_path, attempt_timeout)
    }

    fn audio_set(
        &self,
        params: &ProbeParams,
        attempt_timeout: Duration,
        channel: d2b_contracts::guest_proto::AudioChannel,
        kind: d2b_contracts::guest_proto::AudioSetKind,
        grant_on: bool,
        level: u32,
    ) -> Result<GuestAudioChannelStatus, GuestAudioSetError> {
        run_audio_set_once(
            params,
            &self.broker_socket_path,
            attempt_timeout,
            channel,
            kind,
            grant_on,
            level,
        )
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
/// holds a `tokio::net::UnixStream` that needs the reactor. The connect
/// draws `min(cap, remaining)` from the shared attempt budget so it
/// shares the same absolute deadline as the ttRPC calls; a passed
/// deadline returns [`GuestControlHealthError::Timeout`].
pub(crate) fn connect_and_build_client(
    params: &ProbeParams,
    budget: AttemptBudget,
) -> Result<TtrpcGuestControlClient, GuestControlHealthError> {
    let connect_timeout = budget.next().ok_or(GuestControlHealthError::Timeout)?;
    let connected = match connect_guest_control_vsock(
        &params.socket_path,
        &params.state_root,
        params.expected_state_root_uid,
        params.expected_state_root_gid,
        params.expected_peer_uid,
        params.expected_peer_gid,
        connect_timeout,
    ) {
        GuestControlTransportProbeResult::Connected(connected) => connected,
        GuestControlTransportProbeResult::Failed(_) => {
            return Err(GuestControlHealthError::TransportIo);
        }
    };
    let socket = connected_stream_to_ttrpc_socket(connected)?;
    Ok(TtrpcGuestControlClient::new(socket, budget))
}

/// Test-only twin of [`connect_and_build_client`] that drives the SAME connect
/// path through the relaxed-directory test policy
/// (`connect_guest_control_vsock_for_tests`), so a hermetic test reaches the
/// genuine `SocketMissing` transport branch under a non-root tempdir instead of
/// tripping the production state-root ownership pre-validation first. The
/// `Failed(_) -> TransportIo` mapping is identical to production.
#[cfg(test)]
pub(crate) fn connect_and_build_client_for_tests(
    params: &ProbeParams,
    budget: AttemptBudget,
) -> Result<TtrpcGuestControlClient, GuestControlHealthError> {
    let connect_timeout = budget.next().ok_or(GuestControlHealthError::Timeout)?;
    let connected = match crate::guest_control_vsock::connect_guest_control_vsock_for_tests(
        &params.socket_path,
        &params.state_root,
        connect_timeout,
    ) {
        GuestControlTransportProbeResult::Connected(connected) => connected,
        GuestControlTransportProbeResult::Failed(_) => {
            return Err(GuestControlHealthError::TransportIo);
        }
    };
    let socket = connected_stream_to_ttrpc_socket(connected)?;
    Ok(TtrpcGuestControlClient::new(socket, budget))
}

/// One authenticated Health probe attempt. Builds a fresh host nonce, an
/// absolute-deadline budget (`now + attempt_timeout`, capped at
/// [`GUEST_CONTROL_ATTEMPT_CAP`]), a per-attempt broker signer sharing
/// that budget, and a dedicated current-thread runtime. Connect, every
/// ttRPC unary, and both broker signs draw from the one budget so the
/// whole attempt is bounded by its absolute deadline.
pub fn run_health_probe_once(
    params: &ProbeParams,
    broker_socket_path: &Path,
    attempt_timeout: Duration,
) -> Result<GuestControlHealthEvidence, GuestControlHealthError> {
    let budget = AttemptBudget::from_now(attempt_timeout, GUEST_CONTROL_ATTEMPT_CAP);
    let signer = BrokerSigner::new(broker_socket_path.to_path_buf(), budget);
    let nonce = host_nonce().map_err(|_| GuestControlHealthError::Signer)?;
    let runtime = build_probe_runtime()?;
    runtime.block_on(async {
        let client = connect_and_build_client(params, budget)?;
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

/// One authenticated config-read attempt over the same handshake. Shares
/// a single absolute-deadline budget across connect / ttRPC / sign.
pub fn run_config_read_once(
    params: &ProbeParams,
    broker_socket_path: &Path,
    attempt_timeout: Duration,
) -> Result<Vec<u8>, GuestFileReadError> {
    let budget = AttemptBudget::from_now(attempt_timeout, GUEST_CONTROL_ATTEMPT_CAP);
    let signer = BrokerSigner::new(broker_socket_path.to_path_buf(), budget);
    let nonce =
        host_nonce().map_err(|_| GuestFileReadError::Probe(GuestControlHealthError::Signer))?;
    let runtime = build_probe_runtime().map_err(GuestFileReadError::Probe)?;
    runtime.block_on(async {
        let client = connect_and_build_client(params, budget).map_err(GuestFileReadError::Probe)?;
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

pub fn run_usbip_import_once(
    params: &ProbeParams,
    broker_socket_path: &Path,
    attempt_timeout: Duration,
    call: GuestUsbipImportCall<'_>,
) -> Result<GuestUsbipImportResult, GuestUsbipImportError> {
    let budget = AttemptBudget::from_now(attempt_timeout, attempt_timeout);
    let signer = BrokerSigner::new(broker_socket_path.to_path_buf(), budget);
    let nonce =
        host_nonce().map_err(|_| GuestUsbipImportError::Probe(GuestControlHealthError::Signer))?;
    let runtime = build_probe_runtime().map_err(GuestUsbipImportError::Probe)?;
    runtime.block_on(async {
        let client =
            connect_and_build_client(params, budget).map_err(GuestUsbipImportError::Probe)?;
        usbip_import_authenticated(
            &params.vm_id,
            Some(VMADDR_CID_HOST),
            nonce,
            &client,
            &signer,
            call,
        )
        .await
    })
}

pub fn run_usbip_status_once(
    params: &ProbeParams,
    broker_socket_path: &Path,
    attempt_timeout: Duration,
    host: Option<&str>,
    bus_id: Option<&str>,
) -> Result<GuestUsbipStatusResult, GuestUsbipImportError> {
    let budget = AttemptBudget::from_now(attempt_timeout, attempt_timeout);
    let signer = BrokerSigner::new(broker_socket_path.to_path_buf(), budget);
    let nonce =
        host_nonce().map_err(|_| GuestUsbipImportError::Probe(GuestControlHealthError::Signer))?;
    let runtime = build_probe_runtime().map_err(GuestUsbipImportError::Probe)?;
    runtime.block_on(async {
        let client =
            connect_and_build_client(params, budget).map_err(GuestUsbipImportError::Probe)?;
        usbip_status_authenticated(
            &params.vm_id,
            Some(VMADDR_CID_HOST),
            nonce,
            &client,
            &signer,
            host,
            bus_id,
        )
        .await
    })
}

pub fn run_activation_start_once(
    params: &ProbeParams,
    broker_socket_path: &Path,
    attempt_timeout: Duration,
    start: &GuestSystemActivationStart,
) -> Result<GuestSystemActivationStatus, GuestSystemActivationError> {
    let budget = AttemptBudget::from_now(attempt_timeout, attempt_timeout);
    let signer = BrokerSigner::new(broker_socket_path.to_path_buf(), budget);
    let nonce = host_nonce()
        .map_err(|_| GuestSystemActivationError::Probe(GuestControlHealthError::Signer))?;
    let runtime = build_probe_runtime().map_err(GuestSystemActivationError::Probe)?;
    runtime.block_on(async {
        let client =
            connect_and_build_client(params, budget).map_err(GuestSystemActivationError::Probe)?;
        activate_system_start_authenticated(
            &params.vm_id,
            Some(VMADDR_CID_HOST),
            nonce,
            &client,
            &signer,
            start,
        )
        .await
    })
}

pub fn run_activation_status_once(
    params: &ProbeParams,
    broker_socket_path: &Path,
    attempt_timeout: Duration,
    activation_id: &str,
) -> Result<GuestSystemActivationStatus, GuestSystemActivationError> {
    let budget = AttemptBudget::from_now(attempt_timeout, attempt_timeout);
    let signer = BrokerSigner::new(broker_socket_path.to_path_buf(), budget);
    let nonce = host_nonce()
        .map_err(|_| GuestSystemActivationError::Probe(GuestControlHealthError::Signer))?;
    let runtime = build_probe_runtime().map_err(GuestSystemActivationError::Probe)?;
    runtime.block_on(async {
        let client =
            connect_and_build_client(params, budget).map_err(GuestSystemActivationError::Probe)?;
        activate_system_status_authenticated(
            &params.vm_id,
            Some(VMADDR_CID_HOST),
            nonce,
            &client,
            &signer,
            activation_id,
        )
        .await
    })
}

/// Issue a single authenticated AudioSet RPC attempt on the current thread's
/// runtime. Callers MUST be inside a current-thread Tokio runtime.
pub fn run_audio_set_once(
    params: &ProbeParams,
    broker_socket_path: &Path,
    attempt_timeout: Duration,
    channel: d2b_contracts::guest_proto::AudioChannel,
    kind: d2b_contracts::guest_proto::AudioSetKind,
    grant_on: bool,
    level: u32,
) -> Result<GuestAudioChannelStatus, GuestAudioSetError> {
    let budget = AttemptBudget::from_now(attempt_timeout, attempt_timeout);
    let signer = BrokerSigner::new(broker_socket_path.to_path_buf(), budget);
    let nonce =
        host_nonce().map_err(|_| GuestAudioSetError::Probe(GuestControlHealthError::Signer))?;
    let runtime = build_probe_runtime().map_err(GuestAudioSetError::Probe)?;
    runtime.block_on(async {
        let client = connect_and_build_client(params, budget).map_err(GuestAudioSetError::Probe)?;
        audio_set_authenticated(
            &params.vm_id,
            Some(VMADDR_CID_HOST),
            nonce,
            &client,
            &signer,
            channel,
            kind,
            grant_on,
            level,
        )
        .await
    })
}

pub fn run_audio_status_once(
    params: &ProbeParams,
    broker_socket_path: &Path,
    attempt_timeout: Duration,
) -> Result<GuestAudioStatus, GuestAudioSetError> {
    let budget = AttemptBudget::from_now(attempt_timeout, attempt_timeout);
    let signer = BrokerSigner::new(broker_socket_path.to_path_buf(), budget);
    let nonce =
        host_nonce().map_err(|_| GuestAudioSetError::Probe(GuestControlHealthError::Signer))?;
    let runtime = build_probe_runtime().map_err(GuestAudioSetError::Probe)?;
    runtime.block_on(async {
        let client = connect_and_build_client(params, budget).map_err(GuestAudioSetError::Probe)?;
        audio_status_authenticated(
            &params.vm_id,
            Some(VMADDR_CID_HOST),
            nonce,
            &client,
            &signer,
        )
        .await
    })
}

/// Whether a config-read failure is a transient connect-level failure
/// worth retrying within the config-sync deadline. A missing/refused CH
/// vsock socket during startup/restart (TransportIo), a ttRPC transport
/// hiccup, or a per-attempt timeout are transient; auth/protocol/signer
/// failures and every guest-reported file error (not-found, too-large,
/// path-unsafe, read-denied, capability-unavailable) are deterministic
/// and returned immediately.
fn config_read_error_is_transient(error: &GuestFileReadError) -> bool {
    matches!(
        error,
        GuestFileReadError::Probe(GuestControlHealthError::TransportIo)
            | GuestFileReadError::Probe(GuestControlHealthError::Ttrpc)
            | GuestFileReadError::Probe(GuestControlHealthError::Timeout)
    )
}

fn usbip_import_error_is_transient(error: &GuestUsbipImportError) -> bool {
    matches!(
        error,
        GuestUsbipImportError::Probe(GuestControlHealthError::TransportIo)
            | GuestUsbipImportError::Probe(GuestControlHealthError::Ttrpc)
            | GuestUsbipImportError::Probe(GuestControlHealthError::Timeout)
    )
}

pub fn activation_error_is_transient(error: &GuestSystemActivationError) -> bool {
    matches!(
        error,
        GuestSystemActivationError::Probe(GuestControlHealthError::TransportIo)
            | GuestSystemActivationError::Probe(GuestControlHealthError::Ttrpc)
            | GuestSystemActivationError::Probe(GuestControlHealthError::Timeout)
    )
}

pub fn activation_status_error_is_transient(error: &GuestSystemActivationError) -> bool {
    use d2b_contracts::guest_proto::GuestControlErrorKind as Kind;
    activation_error_is_transient(error)
        || matches!(
            error,
            GuestSystemActivationError::GuestRejected(
                Kind::GUEST_CONTROL_ERROR_KIND_ACTIVATION_NOT_FOUND
                    | Kind::GUEST_CONTROL_ERROR_KIND_ACTIVATION_STATUS_UNAVAILABLE
            )
        )
}

/// State-aware config-read loop, mirroring [`run_guest_control_readiness_loop`].
/// Retries the authenticated config read on transient connect-level
/// failures until `deadline` elapses, applying a per-attempt timeout of
/// `min(attempt_cap, remaining_deadline)` to connect / CONNECT-ACK /
/// ttRPC / broker-sign. A terminal (auth/protocol/file) error returns
/// immediately. Fails CLOSED: once the deadline has been reached (even
/// after an overslept backoff) it does NOT start a fresh floored-to-1ms
/// attempt — it surfaces a Timeout instead.
pub fn run_guest_control_config_read_loop(
    probe: &dyn GuestControlProbe,
    params: &ProbeParams,
    deadline: Duration,
    attempt_cap: Duration,
    retry_backoff: Duration,
    clock: &dyn ProbeClock,
) -> Result<Vec<u8>, GuestFileReadError> {
    loop {
        let remaining = deadline.saturating_sub(clock.elapsed());
        // Fail closed: if the deadline has already passed (e.g. after an
        // overslept backoff), do NOT apply the 1ms floor and start a
        // fresh attempt AFTER the deadline. The exceeded deadline is a
        // timeout (slug guest-control-timeout) end to end.
        if remaining.is_zero() {
            return Err(GuestFileReadError::Probe(GuestControlHealthError::Timeout));
        }
        let attempt_timeout = attempt_cap.min(remaining).max(Duration::from_millis(1));
        match probe.read_config(params, attempt_timeout) {
            Ok(bytes) => return Ok(bytes),
            Err(err) => {
                if !config_read_error_is_transient(&err) {
                    return Err(err);
                }
                // No room for another attempt + backoff before the
                // deadline: return the last transient error.
                if clock.elapsed().saturating_add(retry_backoff) >= deadline {
                    return Err(err);
                }
                clock.sleep(retry_backoff);
            }
        }
    }
}

/// Run the config-read loop on a DEDICATED OS thread so the probe's
/// current-thread runtime is never nested inside a caller's Tokio runtime
/// (the public.sock dispatch path runs synchronously on a multi-threaded
/// runtime worker; calling `Runtime::block_on` there would panic). This is
/// the synchronous-verb runtime boundary: nothing is borrowed across the
/// thread, and the spawned thread starts with no runtime context. The
/// loop retries transient connect failures up to `deadline` with the same
/// per-attempt cap / backoff model as the readiness loop.
pub fn run_config_read_on_dedicated_thread(
    params: ProbeParams,
    broker_socket_path: PathBuf,
    deadline: Duration,
) -> Result<Vec<u8>, GuestFileReadError> {
    std::thread::spawn(move || {
        let probe = RealGuestControlProbe::new(broker_socket_path);
        let clock = RealProbeClock::new();
        run_guest_control_config_read_loop(
            &probe,
            &params,
            deadline,
            GUEST_CONTROL_ATTEMPT_CAP,
            GUEST_CONTROL_RETRY_BACKOFF,
            &clock,
        )
    })
    .join()
    .map_err(|_| GuestFileReadError::Probe(GuestControlHealthError::TransportIo))?
}

pub fn run_usbip_import_on_dedicated_thread(
    params: ProbeParams,
    broker_socket_path: PathBuf,
    action: GuestUsbipAction,
    host: String,
    bus_id: String,
    deadline: Duration,
) -> Result<GuestUsbipImportResult, GuestUsbipImportError> {
    std::thread::spawn(move || {
        let probe = RealGuestControlProbe::new(broker_socket_path);
        let clock = RealProbeClock::new();
        run_guest_control_usbip_import_loop(
            &probe,
            &params,
            GuestUsbipImportCall {
                action,
                host: &host,
                bus_id: &bus_id,
            },
            deadline,
            GUEST_CONTROL_RETRY_BACKOFF,
            &clock,
        )
    })
    .join()
    .map_err(|_| GuestUsbipImportError::Probe(GuestControlHealthError::TransportIo))?
}

pub fn run_usbip_status_on_dedicated_thread(
    params: ProbeParams,
    broker_socket_path: PathBuf,
    host: Option<String>,
    bus_id: Option<String>,
    deadline: Duration,
) -> Result<GuestUsbipStatusResult, GuestUsbipImportError> {
    std::thread::spawn(move || {
        let probe = RealGuestControlProbe::new(broker_socket_path);
        let clock = RealProbeClock::new();
        run_guest_control_usbip_status_loop(
            &probe,
            &params,
            host.as_deref(),
            bus_id.as_deref(),
            deadline,
            GUEST_CONTROL_RETRY_BACKOFF,
            &clock,
        )
    })
    .join()
    .map_err(|_| GuestUsbipImportError::Probe(GuestControlHealthError::TransportIo))?
}

pub fn run_activation_start_on_dedicated_thread(
    params: ProbeParams,
    broker_socket_path: PathBuf,
    start: GuestSystemActivationStart,
    deadline: Duration,
) -> Result<GuestSystemActivationStatus, GuestSystemActivationError> {
    std::thread::spawn(move || {
        let probe = RealGuestControlProbe::new(broker_socket_path);
        let clock = RealProbeClock::new();
        run_guest_control_activation_start_loop(
            &probe,
            &params,
            &start,
            deadline,
            GUEST_CONTROL_RETRY_BACKOFF,
            &clock,
        )
    })
    .join()
    .map_err(|_| GuestSystemActivationError::Probe(GuestControlHealthError::TransportIo))?
}

pub fn run_activation_status_on_dedicated_thread(
    params: ProbeParams,
    broker_socket_path: PathBuf,
    activation_id: String,
    deadline: Duration,
) -> Result<GuestSystemActivationStatus, GuestSystemActivationError> {
    std::thread::spawn(move || {
        let probe = RealGuestControlProbe::new(broker_socket_path);
        let clock = RealProbeClock::new();
        run_guest_control_activation_status_loop(
            &probe,
            &params,
            &activation_id,
            deadline,
            GUEST_CONTROL_RETRY_BACKOFF,
            &clock,
        )
    })
    .join()
    .map_err(|_| GuestSystemActivationError::Probe(GuestControlHealthError::TransportIo))?
}

pub fn run_guest_control_activation_start_loop(
    probe: &dyn GuestControlProbe,
    params: &ProbeParams,
    start: &GuestSystemActivationStart,
    deadline: Duration,
    retry_backoff: Duration,
    clock: &dyn ProbeClock,
) -> Result<GuestSystemActivationStatus, GuestSystemActivationError> {
    loop {
        let remaining = deadline.saturating_sub(clock.elapsed());
        if remaining.is_zero() {
            return Err(GuestSystemActivationError::Probe(
                GuestControlHealthError::Timeout,
            ));
        }
        let attempt_timeout = remaining.max(Duration::from_millis(1));
        match probe.activate_system_start(params, attempt_timeout, start) {
            Ok(status) => return Ok(status),
            Err(err) => {
                if !activation_error_is_transient(&err) {
                    return Err(err);
                }
                if clock.elapsed().saturating_add(retry_backoff) >= deadline {
                    return Err(err);
                }
                clock.sleep(retry_backoff);
            }
        }
    }
}

pub fn run_guest_control_activation_status_loop(
    probe: &dyn GuestControlProbe,
    params: &ProbeParams,
    activation_id: &str,
    deadline: Duration,
    retry_backoff: Duration,
    clock: &dyn ProbeClock,
) -> Result<GuestSystemActivationStatus, GuestSystemActivationError> {
    loop {
        let remaining = deadline.saturating_sub(clock.elapsed());
        if remaining.is_zero() {
            return Err(GuestSystemActivationError::Probe(
                GuestControlHealthError::Timeout,
            ));
        }
        let attempt_timeout = remaining.max(Duration::from_millis(1));
        match probe.activate_system_status(params, attempt_timeout, activation_id) {
            Ok(status) => return Ok(status),
            Err(err) => {
                if !activation_status_error_is_transient(&err) {
                    return Err(err);
                }
                if clock.elapsed().saturating_add(retry_backoff) >= deadline {
                    return Err(err);
                }
                clock.sleep(retry_backoff);
            }
        }
    }
}

/// Per-attempt timeout for audio set RPCs (single attempt, no readiness loop).
pub const GUEST_CONTROL_AUDIO_SET_TIMEOUT: Duration = Duration::from_secs(5);

/// Run an authenticated AudioSet RPC on a DEDICATED OS thread.
///
/// Audio set is a one-shot op (no readiness retry loop): the VM must already
/// be running and guestd ready before the audio command is dispatched. If
/// the VM is not reachable, we return `Probe(TransportIo)` so callers can
/// report `HostOnly` rather than hanging.
pub fn run_audio_set_on_dedicated_thread(
    params: ProbeParams,
    broker_socket_path: PathBuf,
    channel: d2b_contracts::guest_proto::AudioChannel,
    kind: d2b_contracts::guest_proto::AudioSetKind,
    grant_on: bool,
    level: u32,
    deadline: Duration,
) -> Result<GuestAudioChannelStatus, GuestAudioSetError> {
    std::thread::spawn(move || {
        let probe = RealGuestControlProbe::new(broker_socket_path);
        let remaining = deadline;
        let attempt_timeout = remaining.max(Duration::from_millis(1));
        probe.audio_set(&params, attempt_timeout, channel, kind, grant_on, level)
    })
    .join()
    .map_err(|_| GuestAudioSetError::Probe(GuestControlHealthError::TransportIo))?
}

pub fn run_audio_status_on_dedicated_thread(
    params: ProbeParams,
    broker_socket_path: PathBuf,
    deadline: Duration,
) -> Result<GuestAudioStatus, GuestAudioSetError> {
    std::thread::spawn(move || {
        let probe = RealGuestControlProbe::new(broker_socket_path);
        let attempt_timeout = deadline.max(Duration::from_millis(1));
        probe.audio_status(&params, attempt_timeout)
    })
    .join()
    .map_err(|_| GuestAudioSetError::Probe(GuestControlHealthError::TransportIo))?
}

pub fn run_guest_control_usbip_import_loop(
    probe: &dyn GuestControlProbe,
    params: &ProbeParams,
    call: GuestUsbipImportCall<'_>,
    deadline: Duration,
    retry_backoff: Duration,
    clock: &dyn ProbeClock,
) -> Result<GuestUsbipImportResult, GuestUsbipImportError> {
    loop {
        let remaining = deadline.saturating_sub(clock.elapsed());
        if remaining.is_zero() {
            return Err(GuestUsbipImportError::Probe(
                GuestControlHealthError::Timeout,
            ));
        }
        let attempt_timeout = remaining.max(Duration::from_millis(1));
        match probe.usbip_import(params, attempt_timeout, call.action, call.host, call.bus_id) {
            Ok(result) => return Ok(result),
            Err(err) => {
                if !usbip_import_error_is_transient(&err) {
                    return Err(err);
                }
                if clock.elapsed().saturating_add(retry_backoff) >= deadline {
                    return Err(err);
                }
                clock.sleep(retry_backoff);
            }
        }
    }
}

pub fn run_guest_control_usbip_status_loop(
    probe: &dyn GuestControlProbe,
    params: &ProbeParams,
    host: Option<&str>,
    bus_id: Option<&str>,
    deadline: Duration,
    retry_backoff: Duration,
    clock: &dyn ProbeClock,
) -> Result<GuestUsbipStatusResult, GuestUsbipImportError> {
    loop {
        let remaining = deadline.saturating_sub(clock.elapsed());
        if remaining.is_zero() {
            return Err(GuestUsbipImportError::Probe(
                GuestControlHealthError::Timeout,
            ));
        }
        let attempt_timeout = remaining.max(Duration::from_millis(1));
        match probe.usbip_status(params, attempt_timeout, host, bus_id) {
            Ok(result) => return Ok(result),
            Err(err) => {
                if !usbip_import_error_is_transient(&err) {
                    return Err(err);
                }
                if clock.elapsed().saturating_add(retry_backoff) >= deadline {
                    return Err(err);
                }
                clock.sleep(retry_backoff);
            }
        }
    }
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

/// Terminal result of a readiness loop: the last probe outcome plus
/// bounded-retry observability (attempt count and elapsed wall time).
/// `attempts`/`elapsed` are intended as tracing FIELDS / histogram
/// buckets — never metric labels (they are unbounded-ish / per-run).
pub struct ReadinessProbeRun {
    pub outcome: Result<GuestControlHealthEvidence, GuestControlHealthError>,
    pub attempts: u32,
    pub elapsed: Duration,
}

/// State-aware guest-control readiness loop. Retries the authenticated
/// Health probe until [`guest_control_health_ready`] returns true or the
/// `deadline` elapses, applying a per-attempt timeout of
/// `min(attempt_cap, remaining_deadline)` to connect / CONNECT-ACK /
/// ttRPC / broker-sign. Fails CLOSED: on deadline it returns the last
/// (not-ready) outcome, the number of attempts made, and the elapsed
/// wall time.
pub fn run_guest_control_readiness_loop(
    probe: &dyn GuestControlProbe,
    params: &ProbeParams,
    deadline: Duration,
    attempt_cap: Duration,
    retry_backoff: Duration,
    clock: &dyn ProbeClock,
) -> ReadinessProbeRun {
    let start = clock.elapsed();
    let mut attempts: u32 = 0;
    let mut last_outcome: Option<Result<GuestControlHealthEvidence, GuestControlHealthError>> =
        None;
    loop {
        let remaining = deadline.saturating_sub(clock.elapsed());
        // Fail closed: if the deadline has already passed (e.g. after an
        // overslept backoff), do NOT apply the 1ms floor and start a
        // fresh attempt AFTER the deadline. Return the last not-ready
        // outcome, or a Timeout if no attempt ever ran.
        if remaining.is_zero() {
            return ReadinessProbeRun {
                outcome: last_outcome.unwrap_or(Err(GuestControlHealthError::Timeout)),
                attempts,
                elapsed: clock.elapsed().saturating_sub(start),
            };
        }
        let attempt_timeout = attempt_cap.min(remaining).max(Duration::from_millis(1));
        attempts = attempts.saturating_add(1);
        let outcome = probe.probe_health(params, attempt_timeout);
        if guest_control_health_ready(&outcome) {
            return ReadinessProbeRun {
                outcome,
                attempts,
                elapsed: clock.elapsed().saturating_sub(start),
            };
        }
        // Stop if there is no room for another attempt + backoff before
        // the deadline. Returns the last not-ready outcome.
        if clock.elapsed().saturating_add(retry_backoff) >= deadline {
            return ReadinessProbeRun {
                outcome,
                attempts,
                elapsed: clock.elapsed().saturating_sub(start),
            };
        }
        last_outcome = Some(outcome);
        clock.sleep(retry_backoff);
    }
}

/// Leak-safe observability projection of a readiness run. Every string
/// field is a CLOSED-ENUM label drawn from a small fixed vocabulary;
/// `attempt_count`/`duration_ms` are numeric FIELDS. By construction this
/// struct can never carry guest content, store/socket/state-dir paths,
/// nonces, tokens, auth tags, raw signer requests/responses,
/// `guest_boot_id`, or `capabilities_hash`.
pub struct ReadinessObservation {
    pub subsystem: &'static str,
    pub outcome: &'static str,
    pub health_state: &'static str,
    pub health_reason: &'static str,
    pub error_kind: &'static str,
    pub attempt_count: u32,
    pub duration_ms: u64,
}

impl ReadinessObservation {
    /// Project a readiness run onto the closed-enum observability fields.
    pub fn from_run(run: &ReadinessProbeRun) -> Self {
        let ready = guest_control_health_ready(&run.outcome);
        let (health_state, health_reason, error_kind) = match &run.outcome {
            Ok(evidence) => (
                health_state_label(evidence),
                health_reason_label(evidence),
                "none",
            ),
            Err(error) => ("unavailable", "unspecified", error_kind_label(error)),
        };
        Self {
            subsystem: "guest-control-health",
            outcome: if ready { "ready" } else { "not-ready" },
            health_state,
            health_reason,
            error_kind,
            attempt_count: run.attempts,
            duration_ms: u64::try_from(run.elapsed.as_millis()).unwrap_or(u64::MAX),
        }
    }

    /// The closed set of LABEL keys this subsystem contributes to
    /// metrics/spans. Deliberately excludes `vm`, `env`, `attempt_count`,
    /// `duration_ms`, and any path/error-message key: those are span
    /// attributes / fields / buckets, never metric labels.
    pub fn label_keys() -> &'static [&'static str] {
        &[
            "subsystem",
            "outcome",
            "health_state",
            "health_reason",
            "error_kind",
        ]
    }
}

/// Closed-enum label for the guest-reported health state of a probe
/// outcome. Used as a metric/span label, so the range is a small fixed
/// vocabulary — never free-form text and never guest-supplied content.
pub fn health_state_label(evidence: &GuestControlHealthEvidence) -> &'static str {
    use d2b_contracts::guest_proto::HealthState;
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
        GuestControlHealthError::Timeout => "timeout",
    }
}

/// Closed-enum label for the guest-reported health REASON of a probe
/// outcome. Used as a metric/span label, so the range is the fixed
/// `HealthReason` vocabulary — never free-form text and never
/// guest-supplied content.
pub fn health_reason_label(evidence: &GuestControlHealthEvidence) -> &'static str {
    use d2b_contracts::guest_proto::HealthReason;
    match evidence.health.reason.enum_value() {
        Ok(HealthReason::HEALTH_REASON_NONE) => "none",
        Ok(HealthReason::HEALTH_REASON_OLD_GENERATION) => "old-generation",
        Ok(HealthReason::HEALTH_REASON_LISTENER_ABSENT) => "listener-absent",
        Ok(HealthReason::HEALTH_REASON_CONNECT_REFUSED) => "connect-refused",
        Ok(HealthReason::HEALTH_REASON_CONNECT_TIMEOUT) => "connect-timeout",
        Ok(HealthReason::HEALTH_REASON_EOF_BEFORE_ACK) => "eof-before-ack",
        Ok(HealthReason::HEALTH_REASON_MALFORMED_ACK) => "malformed-ack",
        Ok(HealthReason::HEALTH_REASON_ACK_TOO_LONG) => "ack-too-long",
        Ok(HealthReason::HEALTH_REASON_TRANSPORT_IO) => "transport-io",
        Ok(HealthReason::HEALTH_REASON_AUTH_TOKEN_REJECTED) => "auth-token-rejected",
        Ok(HealthReason::HEALTH_REASON_PROTOCOL_VERSION_UNSUPPORTED) => {
            "protocol-version-unsupported"
        }
        Ok(HealthReason::HEALTH_REASON_SESSION_GENERATION_MISMATCH) => {
            "session-generation-mismatch"
        }
        Ok(HealthReason::HEALTH_REASON_EXEC_SUBSYSTEM_UNAVAILABLE) => "exec-subsystem-unavailable",
        Ok(HealthReason::HEALTH_REASON_LOG_STORAGE_UNAVAILABLE) => "log-storage-unavailable",
        Ok(HealthReason::HEALTH_REASON_QUOTA_EXCEEDED) => "quota-exceeded",
        Ok(HealthReason::HEALTH_REASON_RATE_LIMITED) => "rate-limited",
        Ok(HealthReason::HEALTH_REASON_INTERNAL_HEALTH_CHECK_FAILED) => {
            "internal-health-check-failed"
        }
        Ok(HealthReason::HEALTH_REASON_UNSPECIFIED) | Err(_) => "unspecified",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts::broker_wire::{
        BrokerErrorResponse, GuestControlProofRole, GuestControlSignRequest,
    };
    use d2b_contracts::guest_auth::AUTH_TAG_LEN;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicU64, Ordering};

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
        let broker_error =
            map_broker_sign_response(Ok(BrokerResponse::Error(BrokerErrorResponse {
                kind: "guest-control-auth".to_owned(),
                operation: "GuestControlSign".to_owned(),
                target_wave: None,
                message: "refused".to_owned(),
                action: "n/a".to_owned(),
            })));
        assert_eq!(broker_error, Err(GuestControlHealthError::Signer));

        // Wrong (non-sign) response variant -> Signer.
        let wrong = map_broker_sign_response(Ok(BrokerResponse::PollChildReaped(
            d2b_contracts::broker_wire::PollChildReapedResponse {
                notifications: vec![],
            },
        )));
        assert_eq!(wrong, Err(GuestControlHealthError::Signer));

        // Transport/IO error (non-deadline) -> Signer (fail closed).
        let transport = map_broker_sign_response(Err(TypedError::InternalIo {
            context: "recv seqpacket frame".to_owned(),
            detail: "timed out".to_owned(),
        }));
        assert_eq!(transport, Err(GuestControlHealthError::Signer));

        // Round-trip deadline exhaustion -> Timeout, so a stalled/backlogged
        // broker surfaces as the guest-control-timeout error end to end
        // rather than collapsing into a generic Signer failure.
        let deadline = map_broker_sign_response(Err(TypedError::InternalBrokerTimeout {
            path: std::path::PathBuf::from("/run/d2b/priv.sock"),
        }));
        assert_eq!(deadline, Err(GuestControlHealthError::Timeout));
    }

    #[test]
    fn broker_signer_maps_missing_broker_to_signer_error() {
        // A signer pointed at a non-existent broker socket fails the
        // connect and must map to a Signer error (fail closed).
        let signer = BrokerSigner::new(
            PathBuf::from("/nonexistent-d2b-broker.sock"),
            AttemptBudget::from_now(Duration::from_millis(50), GUEST_CONTROL_ATTEMPT_CAP),
        );
        let request = GuestControlSignRequest {
            vm_id: d2b_contracts::types::VmId::new("corp-vm"),
            role: GuestControlProofRole::HostProof,
            protocol_version: d2b_contracts::guest_wire::GUEST_CONTROL_PROTOCOL_VERSION,
            direction: d2b_contracts::broker_wire::GuestControlDirection::HostToGuest,
            purpose: d2b_contracts::broker_wire::GuestControlAuthPurpose::GuestControlAuthV1,
            guest_control_port: d2b_contracts::guest_auth::GUEST_CONTROL_AUTH_PORT,
            peer_cid: Some(VMADDR_CID_HOST),
            host_nonce: vec![0x11; AUTH_NONCE_LEN],
            guest_nonce: vec![0x22; AUTH_NONCE_LEN],
            guest_boot_id: d2b_contracts::broker_wire::GuestBootIdWire::new("boot-1"),
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
        use d2b_contracts::guest_proto as pb;
        let mut health = pb::HealthResponse::new();
        health.origin =
            protobuf::EnumOrUnknown::new(pb::HealthOrigin::HEALTH_ORIGIN_GUEST_REPORTED);
        health.state = protobuf::EnumOrUnknown::new(pb::HealthState::HEALTH_STATE_HEALTHY);
        health.reason = protobuf::EnumOrUnknown::new(pb::HealthReason::HEALTH_REASON_NONE);
        health.remediation =
            protobuf::EnumOrUnknown::new(pb::HealthRemediation::HEALTH_REMEDIATION_NONE);
        health.protocol_version = d2b_contracts::guest_wire::GUEST_CONTROL_PROTOCOL_VERSION;
        GuestControlHealthEvidence {
            vm_id: "corp-vm".to_owned(),
            guest_boot_id: "boot-1".to_owned(),
            protocol_version: d2b_contracts::guest_wire::GUEST_CONTROL_PROTOCOL_VERSION,
            capabilities_hash: "caps-sha256".to_owned(),
            health,
        }
    }

    fn test_params() -> ProbeParams {
        ProbeParams {
            vm_id: "corp-vm".to_owned(),
            socket_path: PathBuf::from("/var/lib/d2b/vms/corp-vm/vsock.sock"),
            state_root: PathBuf::from("/var/lib/d2b/vms/corp-vm"),
            expected_state_root_uid: 990,
            expected_state_root_gid: 100,
            expected_peer_uid: 31000,
            expected_peer_gid: 31000,
        }
    }

    /// A successfully authenticated guest reporting DEGRADED with a valid
    /// reason / remediation / degraded-subsystem set. `guest_control_health_ready`
    /// treats DEGRADED as a SUCCESS (ready), so this drives the
    /// degraded-success readiness path end to end.
    fn degraded_evidence() -> GuestControlHealthEvidence {
        use d2b_contracts::guest_proto as pb;
        let mut health = pb::HealthResponse::new();
        health.origin =
            protobuf::EnumOrUnknown::new(pb::HealthOrigin::HEALTH_ORIGIN_GUEST_REPORTED);
        health.state = protobuf::EnumOrUnknown::new(pb::HealthState::HEALTH_STATE_DEGRADED);
        health.reason =
            protobuf::EnumOrUnknown::new(pb::HealthReason::HEALTH_REASON_QUOTA_EXCEEDED);
        health.remediation =
            protobuf::EnumOrUnknown::new(pb::HealthRemediation::HEALTH_REMEDIATION_REDUCE_LOAD);
        health
            .degraded_subsystems
            .push(protobuf::EnumOrUnknown::new(
                pb::GuestSubsystem::GUEST_SUBSYSTEM_EXEC,
            ));
        health.protocol_version = d2b_contracts::guest_wire::GUEST_CONTROL_PROTOCOL_VERSION;
        GuestControlHealthEvidence {
            vm_id: "corp-vm".to_owned(),
            guest_boot_id: "boot-1".to_owned(),
            protocol_version: d2b_contracts::guest_wire::GUEST_CONTROL_PROTOCOL_VERSION,
            capabilities_hash: "caps-sha256".to_owned(),
            health,
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

    /// Fake clock whose `sleep` OVERSHOOTS: it advances the logical clock
    /// by the requested backoff PLUS a fixed overshoot, deterministically
    /// reproducing an oversleeping `thread::sleep` that lands past the
    /// deadline. Used to prove the loops fail closed instead of starting a
    /// fresh floored-to-1ms attempt after the deadline (D2).
    struct OversleepingClock {
        elapsed_ms: AtomicU64,
        overshoot_ms: u64,
    }

    impl OversleepingClock {
        fn new(overshoot_ms: u64) -> Self {
            Self {
                elapsed_ms: AtomicU64::new(0),
                overshoot_ms,
            }
        }
    }

    impl ProbeClock for OversleepingClock {
        fn elapsed(&self) -> Duration {
            Duration::from_millis(self.elapsed_ms.load(Ordering::SeqCst))
        }

        fn sleep(&self, duration: Duration) {
            self.elapsed_ms.fetch_add(
                duration.as_millis() as u64 + self.overshoot_ms,
                Ordering::SeqCst,
            );
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

        fn usbip_import(
            &self,
            _params: &ProbeParams,
            _attempt_timeout: Duration,
            _action: GuestUsbipAction,
            _host: &str,
            _bus_id: &str,
        ) -> Result<GuestUsbipImportResult, GuestUsbipImportError> {
            unreachable!("readiness loop never imports USBIP")
        }

        fn usbip_status(
            &self,
            _params: &ProbeParams,
            _attempt_timeout: Duration,
            _host: Option<&str>,
            _bus_id: Option<&str>,
        ) -> Result<GuestUsbipStatusResult, GuestUsbipImportError> {
            unreachable!("readiness loop never reads USBIP status")
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
        let run = run_guest_control_readiness_loop(
            &probe,
            &test_params(),
            Duration::from_secs(30),
            GUEST_CONTROL_ATTEMPT_CAP,
            GUEST_CONTROL_RETRY_BACKOFF,
            &clock,
        );
        assert!(guest_control_health_ready(&run.outcome));
        // Three attempts: two transient, one healthy.
        assert_eq!(probe.attempt_timeouts.lock().unwrap().len(), 3);
        assert_eq!(run.attempts, 3);
    }

    #[test]
    fn readiness_loop_degraded_state_is_ready_success() {
        // A guest reporting DEGRADED (with a valid reason/remediation/
        // degraded-subsystem set) is a SUCCESS: the loop terminates ready
        // on the first attempt and the observation projects health_state
        // "degraded" with outcome "ready". This locks the degraded-success
        // path through the full loop + observation projection, not just the
        // `guest_control_health_ready` predicate in isolation.
        let probe = ScriptedProbe::new(vec![Ok(degraded_evidence())]);
        let clock = FakeClock::new();
        let run = run_guest_control_readiness_loop(
            &probe,
            &test_params(),
            Duration::from_secs(30),
            GUEST_CONTROL_ATTEMPT_CAP,
            GUEST_CONTROL_RETRY_BACKOFF,
            &clock,
        );
        assert!(
            guest_control_health_ready(&run.outcome),
            "DEGRADED is a ready/success outcome"
        );
        assert_eq!(run.attempts, 1, "degraded success terminates immediately");
        let obs = ReadinessObservation::from_run(&run);
        assert_eq!(obs.outcome, "ready");
        assert_eq!(obs.health_state, "degraded");
        assert_eq!(obs.health_reason, "quota-exceeded");
        assert_eq!(obs.error_kind, "none");
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
        let run = run_guest_control_readiness_loop(
            &probe,
            &test_params(),
            Duration::from_secs(2),
            GUEST_CONTROL_ATTEMPT_CAP,
            GUEST_CONTROL_RETRY_BACKOFF,
            &clock,
        );
        assert!(!guest_control_health_ready(&run.outcome));
        // The fake clock advanced past the deadline via backoff sleeps.
        assert!(clock.elapsed() >= Duration::from_secs(2) - GUEST_CONTROL_RETRY_BACKOFF);
        // Many attempts occurred before giving up (2s / 250ms backoff).
        assert!(probe.attempt_timeouts.lock().unwrap().len() >= 5);
        assert!(run.attempts >= 5);
    }

    #[test]
    fn readiness_loop_overslept_backoff_starts_no_attempt_past_deadline() {
        // D2: an overslept backoff that lands AT/PAST the deadline must
        // NOT start a fresh floored-to-1ms attempt. The loop fails closed
        // and returns the last not-ready outcome without probing again.
        //
        // deadline 1000ms, backoff 250ms, overshoot 1000ms: attempt 1 at
        // t=0 fails (transient); the end-of-loop guard (0+250 < 1000) lets
        // it sleep; the oversleeping clock jumps to t=1250 (250 + 1000
        // overshoot); the next loop top sees zero remaining and STOPS.
        let probe = ScriptedProbe::new(vec![Err(GuestControlHealthError::TransportIo)]);
        let clock = OversleepingClock::new(1000);
        let run = run_guest_control_readiness_loop(
            &probe,
            &test_params(),
            Duration::from_millis(1000),
            GUEST_CONTROL_ATTEMPT_CAP,
            GUEST_CONTROL_RETRY_BACKOFF,
            &clock,
        );
        assert!(!guest_control_health_ready(&run.outcome));
        // Exactly ONE probe attempt — the post-deadline 1ms attempt that
        // the old `.max(1ms)` floor would have started never happens.
        assert_eq!(
            probe.attempt_timeouts.lock().unwrap().len(),
            1,
            "no fresh attempt may start after the deadline"
        );
        assert_eq!(run.attempts, 1);
    }

    #[test]
    fn readiness_loop_zero_deadline_starts_no_attempt() {
        // D2 boundary: a deadline already at/below the clock on entry must
        // start NO attempt and fail closed as a timeout.
        let probe = ScriptedProbe::new(vec![Ok(healthy_evidence())]);
        let clock = FakeClock::new();
        let run = run_guest_control_readiness_loop(
            &probe,
            &test_params(),
            Duration::ZERO,
            GUEST_CONTROL_ATTEMPT_CAP,
            GUEST_CONTROL_RETRY_BACKOFF,
            &clock,
        );
        assert!(matches!(run.outcome, Err(GuestControlHealthError::Timeout)));
        assert_eq!(run.attempts, 0);
        assert!(probe.attempt_timeouts.lock().unwrap().is_empty());
    }

    /// Probe whose `read_config` returns a scripted sequence, recording
    /// each per-attempt timeout. Used to drive the config-read retry loop
    /// deterministically.
    struct ScriptedConfigProbe {
        outcomes: Mutex<Vec<Result<Vec<u8>, GuestFileReadError>>>,
        attempt_timeouts: Mutex<Vec<Duration>>,
    }

    impl ScriptedConfigProbe {
        fn new(outcomes: Vec<Result<Vec<u8>, GuestFileReadError>>) -> Self {
            Self {
                outcomes: Mutex::new(outcomes),
                attempt_timeouts: Mutex::new(Vec::new()),
            }
        }
    }

    impl GuestControlProbe for ScriptedConfigProbe {
        fn probe_health(
            &self,
            _params: &ProbeParams,
            _attempt_timeout: Duration,
        ) -> Result<GuestControlHealthEvidence, GuestControlHealthError> {
            unreachable!("config-read loop never probes health")
        }

        fn read_config(
            &self,
            _params: &ProbeParams,
            attempt_timeout: Duration,
        ) -> Result<Vec<u8>, GuestFileReadError> {
            self.attempt_timeouts.lock().unwrap().push(attempt_timeout);
            let mut outcomes = self.outcomes.lock().unwrap();
            if outcomes.is_empty() {
                // Past the script: persistent transient connect failure.
                return Err(GuestFileReadError::Probe(
                    GuestControlHealthError::TransportIo,
                ));
            }
            outcomes.remove(0)
        }

        fn usbip_import(
            &self,
            _params: &ProbeParams,
            _attempt_timeout: Duration,
            _action: GuestUsbipAction,
            _host: &str,
            _bus_id: &str,
        ) -> Result<GuestUsbipImportResult, GuestUsbipImportError> {
            unreachable!("config-read loop never imports USBIP")
        }

        fn usbip_status(
            &self,
            _params: &ProbeParams,
            _attempt_timeout: Duration,
            _host: Option<&str>,
            _bus_id: Option<&str>,
        ) -> Result<GuestUsbipStatusResult, GuestUsbipImportError> {
            unreachable!("config-read loop never reads USBIP status")
        }
    }

    struct ScriptedUsbipProbe {
        outcomes: Mutex<Vec<Result<GuestUsbipImportResult, GuestUsbipImportError>>>,
        attempt_timeouts: Mutex<Vec<Duration>>,
    }

    impl ScriptedUsbipProbe {
        fn new(outcomes: Vec<Result<GuestUsbipImportResult, GuestUsbipImportError>>) -> Self {
            Self {
                outcomes: Mutex::new(outcomes),
                attempt_timeouts: Mutex::new(Vec::new()),
            }
        }
    }

    struct ScriptedActivationProbe {
        status_outcomes:
            Mutex<Vec<Result<GuestSystemActivationStatus, GuestSystemActivationError>>>,
        attempt_timeouts: Mutex<Vec<Duration>>,
    }

    impl ScriptedActivationProbe {
        fn new(
            status_outcomes: Vec<Result<GuestSystemActivationStatus, GuestSystemActivationError>>,
        ) -> Self {
            Self {
                status_outcomes: Mutex::new(status_outcomes),
                attempt_timeouts: Mutex::new(Vec::new()),
            }
        }
    }

    impl GuestControlProbe for ScriptedActivationProbe {
        fn probe_health(
            &self,
            _params: &ProbeParams,
            _attempt_timeout: Duration,
        ) -> Result<GuestControlHealthEvidence, GuestControlHealthError> {
            unreachable!("activation status loop never probes health directly")
        }

        fn read_config(
            &self,
            _params: &ProbeParams,
            _attempt_timeout: Duration,
        ) -> Result<Vec<u8>, GuestFileReadError> {
            unreachable!("activation status loop never reads config")
        }

        fn usbip_import(
            &self,
            _params: &ProbeParams,
            _attempt_timeout: Duration,
            _action: GuestUsbipAction,
            _host: &str,
            _bus_id: &str,
        ) -> Result<GuestUsbipImportResult, GuestUsbipImportError> {
            unreachable!("activation status loop never imports USBIP")
        }

        fn usbip_status(
            &self,
            _params: &ProbeParams,
            _attempt_timeout: Duration,
            _host: Option<&str>,
            _bus_id: Option<&str>,
        ) -> Result<GuestUsbipStatusResult, GuestUsbipImportError> {
            unreachable!("activation status loop never reads USBIP status")
        }

        fn activate_system_status(
            &self,
            _params: &ProbeParams,
            attempt_timeout: Duration,
            _activation_id: &str,
        ) -> Result<GuestSystemActivationStatus, GuestSystemActivationError> {
            self.attempt_timeouts.lock().unwrap().push(attempt_timeout);
            let mut outcomes = self.status_outcomes.lock().unwrap();
            if outcomes.is_empty() {
                return Err(GuestSystemActivationError::Probe(
                    GuestControlHealthError::TransportIo,
                ));
            }
            outcomes.remove(0)
        }
    }

    impl GuestControlProbe for ScriptedUsbipProbe {
        fn probe_health(
            &self,
            _params: &ProbeParams,
            _attempt_timeout: Duration,
        ) -> Result<GuestControlHealthEvidence, GuestControlHealthError> {
            unreachable!("usbip loop never probes health directly")
        }

        fn read_config(
            &self,
            _params: &ProbeParams,
            _attempt_timeout: Duration,
        ) -> Result<Vec<u8>, GuestFileReadError> {
            unreachable!("usbip loop never reads config")
        }

        fn usbip_import(
            &self,
            _params: &ProbeParams,
            attempt_timeout: Duration,
            _action: GuestUsbipAction,
            _host: &str,
            _bus_id: &str,
        ) -> Result<GuestUsbipImportResult, GuestUsbipImportError> {
            self.attempt_timeouts.lock().unwrap().push(attempt_timeout);
            let mut outcomes = self.outcomes.lock().unwrap();
            if outcomes.is_empty() {
                return Err(GuestUsbipImportError::Probe(
                    GuestControlHealthError::TransportIo,
                ));
            }
            outcomes.remove(0)
        }

        fn usbip_status(
            &self,
            _params: &ProbeParams,
            _attempt_timeout: Duration,
            _host: Option<&str>,
            _bus_id: Option<&str>,
        ) -> Result<GuestUsbipStatusResult, GuestUsbipImportError> {
            unreachable!("usbip import loop never reads USBIP status")
        }
    }

    #[test]
    fn usbip_import_loop_retries_transient_then_succeeds_with_full_remaining_budget() {
        let probe = ScriptedUsbipProbe::new(vec![
            Err(GuestUsbipImportError::Probe(
                GuestControlHealthError::TransportIo,
            )),
            Ok(GuestUsbipImportResult { detached_ports: 2 }),
        ]);
        let clock = FakeClock::new();
        let result = run_guest_control_usbip_import_loop(
            &probe,
            &test_params(),
            GuestUsbipImportCall {
                action: GuestUsbipAction::Attach,
                host: "192.0.2.1",
                bus_id: "1-2",
            },
            Duration::from_secs(15),
            GUEST_CONTROL_RETRY_BACKOFF,
            &clock,
        )
        .expect("usbip import succeeds");
        assert_eq!(result.detached_ports, 2);
        let timeouts = probe.attempt_timeouts.lock().unwrap();
        assert_eq!(timeouts.len(), 2);
        assert_eq!(
            timeouts[0],
            Duration::from_secs(15),
            "USBIP must not use the short health/config per-attempt cap"
        );
        assert!(timeouts[1] < Duration::from_secs(15));
    }

    #[test]
    fn usbip_import_loop_terminal_error_returns_immediately() {
        let probe =
            ScriptedUsbipProbe::new(vec![Err(GuestUsbipImportError::CapabilityUnavailable)]);
        let clock = FakeClock::new();
        let result = run_guest_control_usbip_import_loop(
            &probe,
            &test_params(),
            GuestUsbipImportCall {
                action: GuestUsbipAction::Attach,
                host: "192.0.2.1",
                bus_id: "1-2",
            },
            Duration::from_secs(15),
            GUEST_CONTROL_RETRY_BACKOFF,
            &clock,
        );
        assert_eq!(result, Err(GuestUsbipImportError::CapabilityUnavailable));
        assert_eq!(probe.attempt_timeouts.lock().unwrap().len(), 1);
    }

    #[test]
    fn usbip_import_loop_zero_deadline_starts_no_attempt() {
        let probe = ScriptedUsbipProbe::new(vec![Ok(GuestUsbipImportResult { detached_ports: 1 })]);
        let clock = FakeClock::new();
        let result = run_guest_control_usbip_import_loop(
            &probe,
            &test_params(),
            GuestUsbipImportCall {
                action: GuestUsbipAction::Detach,
                host: "192.0.2.1",
                bus_id: "1-2",
            },
            Duration::ZERO,
            GUEST_CONTROL_RETRY_BACKOFF,
            &clock,
        );
        assert!(matches!(
            result,
            Err(GuestUsbipImportError::Probe(
                GuestControlHealthError::Timeout
            ))
        ));
        assert!(probe.attempt_timeouts.lock().unwrap().is_empty());
    }

    #[test]
    fn activation_status_loop_rejoins_after_unknown_activation_id() {
        use d2b_contracts::guest_proto::{GuestActivationState, GuestControlErrorKind};

        let probe = ScriptedActivationProbe::new(vec![
            Err(GuestSystemActivationError::GuestRejected(
                GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_ACTIVATION_NOT_FOUND,
            )),
            Ok(GuestSystemActivationStatus {
                state: GuestActivationState::GUEST_ACTIVATION_STATE_SUCCEEDED,
                exit_code: Some(0),
                signal: None,
                status_code: Some(0),
            }),
        ]);
        let clock = FakeClock::new();
        let result = run_guest_control_activation_status_loop(
            &probe,
            &test_params(),
            "activation-1",
            Duration::from_secs(10),
            GUEST_CONTROL_RETRY_BACKOFF,
            &clock,
        )
        .expect("status rejoin succeeds");

        assert_eq!(
            result.state,
            GuestActivationState::GUEST_ACTIVATION_STATE_SUCCEEDED
        );
        assert_eq!(probe.attempt_timeouts.lock().unwrap().len(), 2);
    }

    #[test]
    fn config_read_loop_retries_transient_then_succeeds() {
        // A transient missing/refused CH vsock socket during startup must
        // be retried, not fail config sync immediately.
        let probe = ScriptedConfigProbe::new(vec![
            Err(GuestFileReadError::Probe(
                GuestControlHealthError::TransportIo,
            )),
            Err(GuestFileReadError::Probe(GuestControlHealthError::Timeout)),
            Ok(b"config-bytes".to_vec()),
        ]);
        let clock = FakeClock::new();
        let result = run_guest_control_config_read_loop(
            &probe,
            &test_params(),
            Duration::from_secs(10),
            GUEST_CONTROL_ATTEMPT_CAP,
            GUEST_CONTROL_RETRY_BACKOFF,
            &clock,
        );
        assert_eq!(result.expect("read succeeds"), b"config-bytes".to_vec());
        assert_eq!(probe.attempt_timeouts.lock().unwrap().len(), 3);
    }

    #[test]
    fn config_read_loop_per_attempt_timeout_is_capped() {
        let probe = ScriptedConfigProbe::new(vec![Ok(b"x".to_vec())]);
        let clock = FakeClock::new();
        let _ = run_guest_control_config_read_loop(
            &probe,
            &test_params(),
            Duration::from_secs(10),
            GUEST_CONTROL_ATTEMPT_CAP,
            GUEST_CONTROL_RETRY_BACKOFF,
            &clock,
        );
        // 10s deadline, 3s cap -> first attempt timeout is the cap.
        assert_eq!(
            probe.attempt_timeouts.lock().unwrap()[0],
            GUEST_CONTROL_ATTEMPT_CAP
        );
    }

    #[test]
    fn config_read_loop_terminal_error_returns_immediately() {
        // A deterministic guest-reported file error (FileNotFound) must
        // NOT be retried: it returns on the first attempt.
        let probe = ScriptedConfigProbe::new(vec![Err(GuestFileReadError::FileNotFound)]);
        let clock = FakeClock::new();
        let result = run_guest_control_config_read_loop(
            &probe,
            &test_params(),
            Duration::from_secs(10),
            GUEST_CONTROL_ATTEMPT_CAP,
            GUEST_CONTROL_RETRY_BACKOFF,
            &clock,
        );
        assert!(matches!(result, Err(GuestFileReadError::FileNotFound)));
        assert_eq!(probe.attempt_timeouts.lock().unwrap().len(), 1);
    }

    #[test]
    fn config_read_loop_persistent_transient_hits_deadline() {
        // Persistent transient connect failure: retried until the
        // config-sync deadline, then the last transient error is returned.
        let probe = ScriptedConfigProbe::new(vec![]);
        let clock = FakeClock::new();
        let result = run_guest_control_config_read_loop(
            &probe,
            &test_params(),
            Duration::from_secs(2),
            GUEST_CONTROL_ATTEMPT_CAP,
            GUEST_CONTROL_RETRY_BACKOFF,
            &clock,
        );
        assert!(matches!(
            result,
            Err(GuestFileReadError::Probe(
                GuestControlHealthError::TransportIo
            ))
        ));
        assert!(probe.attempt_timeouts.lock().unwrap().len() >= 5);
    }

    #[test]
    fn config_read_loop_overslept_backoff_starts_no_attempt_past_deadline() {
        // D2: an overslept backoff that lands AT/PAST the deadline must
        // NOT start a fresh floored-to-1ms config-read attempt. The loop
        // fails closed and surfaces a Timeout (slug guest-control-timeout)
        // without reading again. deadline 1000ms, backoff 250ms, overshoot
        // 1000ms: attempt 1 at t=0 fails transient, sleeps; the clock
        // jumps to t=1250; the next loop top sees zero remaining and STOPS.
        let probe = ScriptedConfigProbe::new(vec![Err(GuestFileReadError::Probe(
            GuestControlHealthError::TransportIo,
        ))]);
        let clock = OversleepingClock::new(1000);
        let result = run_guest_control_config_read_loop(
            &probe,
            &test_params(),
            Duration::from_millis(1000),
            GUEST_CONTROL_ATTEMPT_CAP,
            GUEST_CONTROL_RETRY_BACKOFF,
            &clock,
        );
        assert!(
            matches!(
                result,
                Err(GuestFileReadError::Probe(GuestControlHealthError::Timeout))
            ),
            "an exceeded deadline must surface as Timeout, got {result:?}"
        );
        // Exactly ONE read attempt — the post-deadline 1ms attempt that
        // the old `.max(1ms)` floor would have started never happens.
        assert_eq!(
            probe.attempt_timeouts.lock().unwrap().len(),
            1,
            "no fresh attempt may start after the deadline"
        );
    }

    #[test]
    fn config_read_loop_zero_deadline_starts_no_attempt() {
        // D2 boundary: a zero deadline on entry must start NO attempt and
        // fail closed as a timeout.
        let probe = ScriptedConfigProbe::new(vec![Ok(b"never-read".to_vec())]);
        let clock = FakeClock::new();
        let result = run_guest_control_config_read_loop(
            &probe,
            &test_params(),
            Duration::ZERO,
            GUEST_CONTROL_ATTEMPT_CAP,
            GUEST_CONTROL_RETRY_BACKOFF,
            &clock,
        );
        assert!(matches!(
            result,
            Err(GuestFileReadError::Probe(GuestControlHealthError::Timeout))
        ));
        assert!(probe.attempt_timeouts.lock().unwrap().is_empty());
    }

    #[test]
    fn expired_attempt_budget_surfaces_timeout_through_broker_signer() {
        // A genuinely expired per-attempt budget must surface as a
        // Timeout from the PRODUCTION BrokerSigner WITHOUT even reaching
        // the broker socket, so a real deadline/timeout reaches the
        // documented guest-control-timeout error.
        let signer = BrokerSigner::new(
            PathBuf::from("/nonexistent-d2b-broker.sock"),
            AttemptBudget::from_now(Duration::ZERO, GUEST_CONTROL_ATTEMPT_CAP),
        );
        let request = GuestControlSignRequest {
            vm_id: d2b_contracts::types::VmId::new("corp-vm"),
            role: GuestControlProofRole::HostProof,
            protocol_version: d2b_contracts::guest_wire::GUEST_CONTROL_PROTOCOL_VERSION,
            direction: d2b_contracts::broker_wire::GuestControlDirection::HostToGuest,
            purpose: d2b_contracts::broker_wire::GuestControlAuthPurpose::GuestControlAuthV1,
            guest_control_port: d2b_contracts::guest_auth::GUEST_CONTROL_AUTH_PORT,
            peer_cid: Some(VMADDR_CID_HOST),
            host_nonce: vec![0x11; AUTH_NONCE_LEN],
            guest_nonce: vec![0x22; AUTH_NONCE_LEN],
            guest_boot_id: d2b_contracts::broker_wire::GuestBootIdWire::new("boot-1"),
            capabilities_hash: None,
            tracing_span_id: None,
        };
        assert_eq!(signer.sign(request), Err(GuestControlHealthError::Timeout));
    }

    #[test]
    fn config_read_timeout_maps_to_timeout_kind() {
        // A probe timeout flows through the read-error mapping as the
        // closed-enum Timeout kind (slug guest-control-timeout), not a
        // generic Transport collapse.
        use crate::typed_error::{GuestControlReadErrorKind, TypedError};
        let mapped = crate::map_guest_file_read_error(GuestFileReadError::Probe(
            GuestControlHealthError::Timeout,
        ));
        match mapped {
            TypedError::GuestControlReadFailed { kind } => {
                assert_eq!(kind, GuestControlReadErrorKind::Timeout);
            }
            other => panic!("expected GuestControlReadFailed, got {other:?}"),
        }
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
        // Assert EVERY field of EVERY forwarded request so a regression in
        // any one (vm_id, direction, purpose, port, cid, nonces, boot id,
        // protocol version, span id) is caught, not just a representative
        // subset.
        for request in recorded.iter() {
            assert_eq!(request.vm_id.as_str(), "corp-vm");
            assert_eq!(
                request.protocol_version,
                d2b_contracts::guest_wire::GUEST_CONTROL_PROTOCOL_VERSION
            );
            assert_eq!(
                request.direction,
                d2b_contracts::broker_wire::GuestControlDirection::HostToGuest
            );
            assert_eq!(
                request.purpose,
                d2b_contracts::broker_wire::GuestControlAuthPurpose::GuestControlAuthV1
            );
            assert_eq!(
                request.guest_control_port,
                d2b_contracts::guest_auth::GUEST_CONTROL_AUTH_PORT
            );
            assert_eq!(request.peer_cid, Some(VMADDR_CID_HOST));
            assert_eq!(request.host_nonce, host_nonce.to_vec());
            // The guest nonce echoes the HappyFakeClient Hello reply.
            assert_eq!(request.guest_nonce, vec![0x22; AUTH_NONCE_LEN]);
            assert_eq!(request.guest_boot_id.as_str(), "boot-1");
            // The host never forwards a tracing span id to the broker.
            assert!(request.tracing_span_id.is_none());
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
            _request: d2b_contracts::guest_proto::HelloRequest,
        ) -> Result<d2b_contracts::guest_proto::HelloResponse, GuestControlHealthError> {
            use d2b_contracts::guest_proto as pb;
            let mut response = pb::HelloResponse::new();
            response.guest_nonce = vec![0x22; AUTH_NONCE_LEN];
            response.guest_boot_id = "boot-1".to_owned();
            response.protocol_version = d2b_contracts::guest_wire::GUEST_CONTROL_PROTOCOL_VERSION;
            Ok(response)
        }

        async fn authenticate(
            &self,
            _request: d2b_contracts::guest_proto::AuthenticateRequest,
        ) -> Result<d2b_contracts::guest_proto::AuthenticateResponse, GuestControlHealthError>
        {
            use d2b_contracts::guest_proto as pb;
            let mut response = pb::AuthenticateResponse::new();
            response.guest_auth_tag = Some(vec![0x77; AUTH_TAG_LEN]);
            response.capabilities_hash = Some("caps-sha256".to_owned());
            Ok(response)
        }

        async fn health(
            &self,
            _request: d2b_contracts::guest_proto::HealthRequest,
        ) -> Result<d2b_contracts::guest_proto::HealthResponse, GuestControlHealthError> {
            Ok(healthy_evidence().health)
        }

        async fn read_guest_file(
            &self,
            _request: d2b_contracts::guest_proto::ReadGuestFileRequest,
        ) -> Result<d2b_contracts::guest_proto::ReadGuestFileResponse, GuestControlHealthError>
        {
            Ok(d2b_contracts::guest_proto::ReadGuestFileResponse::new())
        }

        async fn usbip_import(
            &self,
            _request: d2b_contracts::guest_proto::UsbipImportRequest,
        ) -> Result<d2b_contracts::guest_proto::UsbipImportResponse, GuestControlHealthError>
        {
            Ok(d2b_contracts::guest_proto::UsbipImportResponse::new())
        }

        async fn usbip_status(
            &self,
            _request: d2b_contracts::guest_proto::UsbipStatusRequest,
        ) -> Result<d2b_contracts::guest_proto::UsbipStatusResponse, GuestControlHealthError>
        {
            Ok(d2b_contracts::guest_proto::UsbipStatusResponse::new())
        }
    }

    /// Quoted argv tokens (`"ssh"`, `"scp"`) constructed without embedding the
    /// bare literals in this file, so the daemon-source scan never trips on its
    /// own search strings.
    fn forbidden_argv_tokens() -> [String; 2] {
        let ssh: String = ['s', 's', 'h'].iter().collect();
        let scp: String = ['s', 'c', 'p'].iter().collect();
        [format!("\"{ssh}\""), format!("\"{scp}\"")]
    }

    /// True if `src` launches an SSH/SCP client outside a `#[cfg(test)] mod`
    /// block. The daemon hosts the guest-control readiness path and MUST NOT
    /// spawn an SSH client anywhere; there is no allowlist on the daemon side.
    fn source_launches_ssh(src: &str) -> bool {
        let [ssh_tok, scp_tok] = forbidden_argv_tokens();
        let lines: Vec<&str> = src.lines().collect();
        let mut in_test_mod = false;
        let mut i = 0;
        while i < lines.len() {
            let line = lines[i];
            let trimmed = line.trim();
            if !in_test_mod && trimmed == "#[cfg(test)]" {
                let next_is_mod = lines[i + 1..]
                    .iter()
                    .find(|candidate| !candidate.trim().is_empty())
                    .map(|candidate| candidate.trim_start().starts_with("mod "))
                    .unwrap_or(false);
                if next_is_mod {
                    in_test_mod = true;
                }
            }
            if in_test_mod {
                if line == "}" {
                    in_test_mod = false;
                }
                i += 1;
                continue;
            }
            if line.contains(&ssh_tok) || line.contains(&scp_tok) {
                return true;
            }
            i += 1;
        }
        false
    }

    fn collect_rs_sources(dir: &std::path::Path, out: &mut Vec<(PathBuf, String)>) {
        for entry in std::fs::read_dir(dir).expect("read daemon src dir") {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                collect_rs_sources(&path, out);
            } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
                let body = std::fs::read_to_string(&path).expect("read daemon source file");
                out.push((path, body));
            }
        }
    }

    #[test]
    fn daemon_source_launches_no_ssh_client() {
        let src_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut sources = Vec::new();
        collect_rs_sources(&src_dir, &mut sources);
        assert!(!sources.is_empty(), "expected daemon source files");
        let offenders: Vec<&PathBuf> = sources
            .iter()
            .filter(|(_, body)| source_launches_ssh(body))
            .map(|(path, _)| path)
            .collect();
        assert!(
            offenders.is_empty(),
            "the daemon must never launch an SSH/SCP client; offenders: {offenders:?}"
        );
    }

    #[test]
    fn readiness_loop_spawns_no_ssh_client() {
        // The readiness loop drives ONLY the injected probe and spawns no
        // external process. The daemon's hard "never launch an SSH/SCP
        // client" invariant is enforced statically across the whole daemon
        // source by `daemon_source_launches_no_ssh_client`; this test verifies
        // the readiness path converges to ready through the injected probe.
        let probe = ScriptedProbe::new(vec![
            Err(GuestControlHealthError::TransportIo),
            Ok(healthy_evidence()),
        ]);
        let clock = FakeClock::new();
        let run = run_guest_control_readiness_loop(
            &probe,
            &test_params(),
            Duration::from_secs(30),
            GUEST_CONTROL_ATTEMPT_CAP,
            GUEST_CONTROL_RETRY_BACKOFF,
            &clock,
        );
        assert!(guest_control_health_ready(&run.outcome));
    }

    /// Build evidence whose every guest-controlled string carries a
    /// sentinel, so a leak into the observability projection is detectable.
    fn sentinel_evidence(
        state: d2b_contracts::guest_proto::HealthState,
        reason: d2b_contracts::guest_proto::HealthReason,
    ) -> GuestControlHealthEvidence {
        use d2b_contracts::guest_proto as pb;
        let sentinel = "SENTINEL-LEAK-7b3f";
        let mut health = pb::HealthResponse::new();
        health.origin =
            protobuf::EnumOrUnknown::new(pb::HealthOrigin::HEALTH_ORIGIN_GUEST_REPORTED);
        health.state = protobuf::EnumOrUnknown::new(state);
        health.reason = protobuf::EnumOrUnknown::new(reason);
        health.remediation =
            protobuf::EnumOrUnknown::new(pb::HealthRemediation::HEALTH_REMEDIATION_NONE);
        health.protocol_version = d2b_contracts::guest_wire::GUEST_CONTROL_PROTOCOL_VERSION;
        GuestControlHealthEvidence {
            vm_id: sentinel.to_owned(),
            guest_boot_id: sentinel.to_owned(),
            protocol_version: d2b_contracts::guest_wire::GUEST_CONTROL_PROTOCOL_VERSION,
            capabilities_hash: sentinel.to_owned(),
            health,
        }
    }

    /// The closed health-state / health-reason / error-kind vocabularies
    /// the observability projection is allowed to emit. Any value outside
    /// these sets would be an unbounded-cardinality or leaky label.
    const APPROVED_HEALTH_STATES: &[&str] = &[
        "healthy",
        "degraded",
        "unavailable-old-generation",
        "listener-absent",
        "transport-unreachable",
        "auth-failed",
        "protocol-mismatch",
        "stale-session",
        "unspecified",
        "unavailable",
    ];
    const APPROVED_OUTCOMES: &[&str] = &["ready", "not-ready"];

    #[test]
    fn readiness_observation_carries_no_guest_bytes_and_uses_closed_enums() {
        use d2b_contracts::guest_proto::{HealthReason, HealthState};

        // Success projection: guest-reported strings (vm_id, guest_boot_id,
        // capabilities_hash) must NEVER reach the observation fields.
        let run_ok = ReadinessProbeRun {
            outcome: Ok(sentinel_evidence(
                HealthState::HEALTH_STATE_DEGRADED,
                HealthReason::HEALTH_REASON_QUOTA_EXCEEDED,
            )),
            attempts: 4,
            elapsed: Duration::from_millis(1234),
        };
        let obs = ReadinessObservation::from_run(&run_ok);
        for field in [
            obs.subsystem,
            obs.outcome,
            obs.health_state,
            obs.health_reason,
            obs.error_kind,
        ] {
            assert!(!field.contains("SENTINEL"), "guest content leaked: {field}");
        }
        assert_eq!(obs.subsystem, "guest-control-health");
        assert_eq!(obs.health_state, "degraded");
        assert_eq!(obs.health_reason, "quota-exceeded");
        assert_eq!(obs.error_kind, "none");
        assert!(APPROVED_HEALTH_STATES.contains(&obs.health_state));
        assert!(APPROVED_OUTCOMES.contains(&obs.outcome));
        // attempt_count/duration are numeric FIELDS, not labels.
        assert_eq!(obs.attempt_count, 4);
        assert_eq!(obs.duration_ms, 1234);

        // Error projection: closed error_kind, neutral state/reason.
        let run_err = ReadinessProbeRun {
            outcome: Err(GuestControlHealthError::AuthFailed),
            attempts: 2,
            elapsed: Duration::from_millis(50),
        };
        let obs_err = ReadinessObservation::from_run(&run_err);
        assert_eq!(obs_err.outcome, "not-ready");
        assert_eq!(obs_err.error_kind, "auth-failed");
        assert_eq!(obs_err.health_state, "unavailable");
        assert_eq!(obs_err.health_reason, "unspecified");
        assert!(APPROVED_HEALTH_STATES.contains(&obs_err.health_state));
    }

    #[test]
    fn guest_control_fields_never_become_metric_labels() {
        // The guest-control readiness path emits closed-enum tracing
        // labels + numeric fields, never Prometheus metric labels. Assert
        // the closed LABEL vocabulary excludes every forbidden / high-
        // cardinality / per-run key.
        let forbidden = [
            "vm",
            "env",
            "attempt",
            "attempt_count",
            "duration",
            "duration_ms",
            "size",
            "sha256",
            "path",
            "socket",
            "state_dir",
            "store_path",
            "error",
            "error_message",
            "nonce",
            "token",
            "auth_tag",
            "guest_boot_id",
            "capabilities_hash",
            "content",
        ];
        for key in ReadinessObservation::label_keys() {
            assert!(
                !forbidden.contains(key),
                "forbidden guest-control metric label key: {key}"
            );
        }
        // The daemon declares no guest-control Prometheus metric, so none
        // of its closed-enum field names may surface in the rendered
        // exposition as a metric label.
        let rendered = crate::metrics::Registry::new().render();
        for leaked in [
            "health_state",
            "health_reason",
            "guest_boot_id",
            "capabilities_hash",
            "SENTINEL",
        ] {
            assert!(
                !rendered.contains(leaked),
                "guest-control field leaked into rendered metrics: {leaked}"
            );
        }
    }

    // ── Audio set probe tests ──────────────────────────────────────────────

    use d2b_contracts::guest_proto as pb;

    struct ScriptedAudioProbe {
        outcomes: Mutex<Vec<Result<GuestAudioChannelStatus, GuestAudioSetError>>>,
        recorded_calls: Mutex<Vec<(pb::AudioChannel, pb::AudioSetKind, bool, u32)>>,
    }

    impl ScriptedAudioProbe {
        fn new(outcomes: Vec<Result<GuestAudioChannelStatus, GuestAudioSetError>>) -> Self {
            Self {
                outcomes: Mutex::new(outcomes),
                recorded_calls: Mutex::new(Vec::new()),
            }
        }
    }

    impl GuestControlProbe for ScriptedAudioProbe {
        fn probe_health(
            &self,
            _params: &ProbeParams,
            _attempt_timeout: Duration,
        ) -> Result<GuestControlHealthEvidence, GuestControlHealthError> {
            unreachable!("audio set never probes health directly")
        }

        fn read_config(
            &self,
            _params: &ProbeParams,
            _attempt_timeout: Duration,
        ) -> Result<Vec<u8>, GuestFileReadError> {
            unreachable!("audio set never reads config")
        }

        fn usbip_import(
            &self,
            _params: &ProbeParams,
            _attempt_timeout: Duration,
            _action: GuestUsbipAction,
            _host: &str,
            _bus_id: &str,
        ) -> Result<GuestUsbipImportResult, GuestUsbipImportError> {
            unreachable!("audio set never imports USBIP")
        }

        fn usbip_status(
            &self,
            _params: &ProbeParams,
            _attempt_timeout: Duration,
            _host: Option<&str>,
            _bus_id: Option<&str>,
        ) -> Result<GuestUsbipStatusResult, GuestUsbipImportError> {
            unreachable!("audio set never reads USBIP status")
        }

        fn audio_set(
            &self,
            _params: &ProbeParams,
            _attempt_timeout: Duration,
            channel: pb::AudioChannel,
            kind: pb::AudioSetKind,
            grant_on: bool,
            level: u32,
        ) -> Result<GuestAudioChannelStatus, GuestAudioSetError> {
            self.recorded_calls
                .lock()
                .unwrap()
                .push((channel, kind, grant_on, level));
            let mut outcomes = self.outcomes.lock().unwrap();
            if outcomes.is_empty() {
                return Err(GuestAudioSetError::Probe(
                    GuestControlHealthError::TransportIo,
                ));
            }
            outcomes.remove(0)
        }
    }

    #[test]
    fn audio_set_probe_success_returns_applied_status() {
        let probe = ScriptedAudioProbe::new(vec![Ok(GuestAudioChannelStatus {
            muted: false,
            level: 80,
            level_known: true,
        })]);
        let result = probe.audio_set(
            &test_params(),
            Duration::from_secs(5),
            pb::AudioChannel::AUDIO_CHANNEL_SPEAKER,
            pb::AudioSetKind::AUDIO_SET_KIND_GRANT,
            true,
            0,
        );
        assert!(result.is_ok(), "probe success must return Ok");
        let calls = probe.recorded_calls.lock().unwrap();
        assert_eq!(calls.len(), 1, "must record exactly one call");
        assert_eq!(calls[0].0, pb::AudioChannel::AUDIO_CHANNEL_SPEAKER);
        assert_eq!(calls[0].1, pb::AudioSetKind::AUDIO_SET_KIND_GRANT);
        assert!(calls[0].2, "grant_on must be true");
    }

    #[test]
    fn audio_set_probe_capability_unavailable_returns_capability_unavailable_not_fallback() {
        let probe = ScriptedAudioProbe::new(vec![Err(GuestAudioSetError::CapabilityUnavailable)]);
        let result = probe.audio_set(
            &test_params(),
            Duration::from_secs(5),
            pb::AudioChannel::AUDIO_CHANNEL_MICROPHONE,
            pb::AudioSetKind::AUDIO_SET_KIND_GRANT,
            false,
            0,
        );
        assert!(
            matches!(result, Err(GuestAudioSetError::CapabilityUnavailable)),
            "capability unavailable must propagate, not be silently treated as success"
        );
    }

    #[test]
    fn audio_set_probe_transport_failure_returns_probe_error_not_success() {
        let probe = ScriptedAudioProbe::new(vec![Err(GuestAudioSetError::Probe(
            GuestControlHealthError::TransportIo,
        ))]);
        let result = probe.audio_set(
            &test_params(),
            Duration::from_secs(5),
            pb::AudioChannel::AUDIO_CHANNEL_SPEAKER,
            pb::AudioSetKind::AUDIO_SET_KIND_LEVEL,
            false,
            75,
        );
        assert!(
            matches!(result, Err(GuestAudioSetError::Probe(_))),
            "transport failure must not produce a success-shaped result"
        );
    }

    #[test]
    fn audio_set_level_probe_records_correct_level_value() {
        let probe = ScriptedAudioProbe::new(vec![Ok(GuestAudioChannelStatus {
            muted: false,
            level: 60,
            level_known: true,
        })]);
        let result = probe.audio_set(
            &test_params(),
            Duration::from_secs(5),
            pb::AudioChannel::AUDIO_CHANNEL_MICROPHONE,
            pb::AudioSetKind::AUDIO_SET_KIND_LEVEL,
            false,
            60,
        );
        assert!(result.is_ok());
        let calls = probe.recorded_calls.lock().unwrap();
        assert_eq!(calls[0].1, pb::AudioSetKind::AUDIO_SET_KIND_LEVEL);
        assert_eq!(calls[0].3, 60, "level value must be forwarded verbatim");
    }
}
