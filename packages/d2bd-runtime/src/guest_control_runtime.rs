//! Provider-independent guest-control probe contracts and retry/readiness policy.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::guest_control_health::{
    GuestAudioChannelStatus, GuestAudioSetError, GuestAudioStatus, GuestControlHealthError,
    GuestControlHealthEvidence, GuestFileReadError, GuestSystemActivationError,
    GuestSystemActivationStart, GuestSystemActivationStatus, GuestUsbipAction,
    GuestUsbipImportCall, GuestUsbipImportError, GuestUsbipImportResult, GuestUsbipStatusResult,
    guest_control_health_ready,
};

/// Per-attempt cap applied to connect, ttRPC, and broker-sign operations.
pub const GUEST_CONTROL_ATTEMPT_CAP: Duration = Duration::from_secs(3);
pub const GUEST_CONTROL_RETRY_BACKOFF: Duration = Duration::from_millis(250);
pub const GUEST_CONTROL_CONFIG_READ_TIMEOUT: Duration = Duration::from_secs(10);
pub const GUEST_CONTROL_USBIP_IMPORT_TIMEOUT: Duration = Duration::from_secs(15);
pub const GUEST_CONTROL_AUDIO_SET_TIMEOUT: Duration = Duration::from_secs(5);
pub const VMADDR_CID_HOST: u32 = libc::VMADDR_CID_HOST;

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
        channel: d2b_contracts_control::guest_proto::AudioChannel,
        kind: d2b_contracts_control::guest_proto::AudioSetKind,
        grant_on: bool,
        level: u32,
    ) -> Result<GuestAudioChannelStatus, GuestAudioSetError> {
        let _ = (params, attempt_timeout, channel, kind, grant_on, level);
        Err(GuestAudioSetError::CapabilityUnavailable)
    }
}

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
    use d2b_contracts_control::guest_proto::GuestControlErrorKind as Kind;
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
/// attempt - it surfaces a Timeout instead.
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
/// buckets - never metric labels (they are unbounded-ish / per-run).
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
/// vocabulary - never free-form text and never guest-supplied content.
pub fn health_state_label(evidence: &GuestControlHealthEvidence) -> &'static str {
    use d2b_contracts_control::guest_proto::HealthState;
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
/// `HealthReason` vocabulary - never free-form text and never
/// guest-supplied content.
pub fn health_reason_label(evidence: &GuestControlHealthEvidence) -> &'static str {
    use d2b_contracts_control::guest_proto::HealthReason;
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
