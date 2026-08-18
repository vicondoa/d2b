//! Bounded, best-effort Unix datagram telemetry emitter.

use std::{
    collections::{BTreeMap, VecDeque},
    io,
    os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt},
    os::unix::net::UnixDatagram,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use crate::metric_label_policy::{
    IdentityCanaries, MetricDescriptor, MetricPolicyError, canonical_descriptor,
    validate_canonical_data_point, validate_data_point,
};
use d2b_contracts::v3::{
    TelemetryFrame, TelemetryFrameError, TelemetrySignal, parse_raw_frame, redact_parsed_frame,
    validate_frame,
};

#[cfg(test)]
use std::cell::Cell;

#[cfg(test)]
thread_local! {
    static RAW_FRAME_PARSE_COUNT: Cell<usize> = const { Cell::new(0) };
}

#[cfg(test)]
fn count_raw_frame_parse() {
    RAW_FRAME_PARSE_COUNT.with(|count| count.set(count.get().saturating_add(1)));
}

#[cfg(test)]
fn raw_frame_parse_count() -> usize {
    RAW_FRAME_PARSE_COUNT.with(Cell::get)
}

/// Default frame limit for core-process telemetry.
pub const MAX_FRAME_BYTES: usize = 64 * 1024;
/// Default bounded ring capacity.
pub const DEFAULT_RING_CAPACITY_BYTES: usize = 4 * 1024 * 1024;
/// Maximum number of retained frames in one ring.
pub const DEFAULT_RING_CAPACITY_FRAMES: usize = 1024;
/// Maximum age of a retained frame.
pub const DEFAULT_RING_MAX_AGE: Duration = Duration::from_secs(30);
/// Maximum send attempts for one retained frame.
pub const DEFAULT_MAX_RETRY_ATTEMPTS: u8 = 3;

/// Signal class used by drop accounting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    /// Metric frame.
    Metric,
    /// Trace or span frame.
    Trace,
    /// Structured log frame.
    Log,
}

impl Signal {
    /// Stable metric label.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Metric => "metric",
            Self::Trace => "trace",
            Self::Log => "log",
        }
    }
}

/// Emitter outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmitOutcome {
    /// The frame was sent directly.
    Sent,
    /// The frame was retained in the bounded FIFO.
    Buffered,
    /// The frame could not fit and was dropped.
    Dropped,
}

/// Emitter failures which do not expose a socket path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmitterError {
    /// The frame exceeded the bounded frame limit.
    FrameTooLarge,
    /// The emitter lock was poisoned.
    StatePoisoned,
    /// A metric frame did not satisfy the closed label policy.
    MetricPolicy(MetricPolicyError),
    /// A non-metric frame was not a bounded structured observation.
    FrameRedaction,
    /// The configured socket path was not a trusted absolute socket.
    SocketPathInvalid,
}

impl core::fmt::Display for EmitterError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::FrameTooLarge => "telemetry-frame-too-large",
            Self::StatePoisoned => "telemetry-emitter-state-poisoned",
            Self::MetricPolicy(_) => "telemetry-metric-policy-rejected",
            Self::FrameRedaction => "telemetry-frame-redaction-rejected",
            Self::SocketPathInvalid => "telemetry-socket-path-invalid",
        })
    }
}

impl std::error::Error for EmitterError {}

#[derive(Default)]
struct DropCounters {
    metric: AtomicU64,
    trace: AtomicU64,
    log: AtomicU64,
}

impl DropCounters {
    fn increment(&self, signal: Signal) {
        let counter = match signal {
            Signal::Metric => &self.metric,
            Signal::Trace => &self.trace,
            Signal::Log => &self.log,
        };
        counter.fetch_add(1, Ordering::Relaxed);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Snapshot of bounded emitter drops.
pub struct DropSnapshot {
    /// Metric frames dropped.
    pub metric: u64,
    /// Trace frames dropped.
    pub trace: u64,
    /// Log frames dropped.
    pub log: u64,
}

struct QueuedFrame {
    signal: Signal,
    bytes: Vec<u8>,
    enqueued_at: Instant,
    attempts: u8,
}

struct State {
    socket: Option<UnixDatagram>,
    socket_identity: Option<(u64, u64, u32, u32)>,
    queue: VecDeque<QueuedFrame>,
    queued_bytes: usize,
}

/// A bounded FIFO emitter which never blocks the caller on export.
#[derive(Clone)]
pub struct BoundedEmitter {
    path: Arc<PathBuf>,
    capacity_bytes: usize,
    capacity_frames: usize,
    max_age: Duration,
    max_retry_attempts: u8,
    state: Arc<Mutex<State>>,
    drops: Arc<DropCounters>,
}

impl core::fmt::Debug for BoundedEmitter {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("BoundedEmitter")
            .field("capacity_bytes", &self.capacity_bytes)
            .field("socket_configured", &true)
            .finish()
    }
}

impl BoundedEmitter {
    /// Construct an emitter for a private Unix datagram path.
    pub fn new(path: impl Into<PathBuf>, capacity_bytes: usize) -> Result<Self, EmitterError> {
        Self::new_with_limits(
            path,
            capacity_bytes,
            DEFAULT_RING_CAPACITY_FRAMES,
            DEFAULT_RING_MAX_AGE,
            DEFAULT_MAX_RETRY_ATTEMPTS,
        )
    }

    /// Construct an emitter with explicit count, age, and retry bounds.
    pub fn new_with_limits(
        path: impl Into<PathBuf>,
        capacity_bytes: usize,
        capacity_frames: usize,
        max_age: Duration,
        max_retry_attempts: u8,
    ) -> Result<Self, EmitterError> {
        if capacity_bytes == 0 {
            return Err(EmitterError::StatePoisoned);
        }
        if capacity_frames == 0 || max_age.is_zero() || max_retry_attempts == 0 {
            return Err(EmitterError::StatePoisoned);
        }
        let path = path.into();
        if !path.is_absolute() {
            return Err(EmitterError::SocketPathInvalid);
        }
        Ok(Self {
            path: Arc::new(path),
            capacity_bytes,
            capacity_frames,
            max_age,
            max_retry_attempts,
            state: Arc::new(Mutex::new(State {
                socket: None,
                socket_identity: None,
                queue: VecDeque::new(),
                queued_bytes: 0,
            })),
            drops: Arc::new(DropCounters::default()),
        })
    }

    /// Construct an emitter with the default ring capacity.
    pub fn with_default_capacity(path: impl Into<PathBuf>) -> Result<Self, EmitterError> {
        Self::new(path, DEFAULT_RING_CAPACITY_BYTES)
    }

    /// Emit one bounded frame.
    pub fn emit(&self, signal: Signal, frame: &[u8]) -> Result<EmitOutcome, EmitterError> {
        if frame.len() > MAX_FRAME_BYTES {
            self.drops.increment(signal);
            return Err(EmitterError::FrameTooLarge);
        }
        #[cfg(test)]
        count_raw_frame_parse();
        let shared = parse_raw_frame(frame).map_err(|_| EmitterError::FrameRedaction)?;
        let expected_signal = match signal {
            Signal::Metric => TelemetrySignal::Metric,
            Signal::Trace => TelemetrySignal::Trace,
            Signal::Log => TelemetrySignal::Log,
        };
        if shared.signal != expected_signal {
            return Err(EmitterError::FrameRedaction);
        }
        if signal == Signal::Metric {
            validate_metric_frame(&shared)?;
        }
        validate_frame(&shared).map_err(|_| EmitterError::FrameRedaction)?;
        let frame = match redact_parsed_frame(shared) {
            Ok(frame) => frame,
            Err(TelemetryFrameError::RedactedOversize) => {
                self.drops.increment(signal);
                return Err(EmitterError::FrameTooLarge);
            }
            Err(_) => return Err(EmitterError::FrameRedaction),
        };
        if frame.len() > MAX_FRAME_BYTES {
            self.drops.increment(signal);
            return Err(EmitterError::FrameTooLarge);
        }
        let mut state = self.state.lock().map_err(|_| EmitterError::StatePoisoned)?;
        self.prune_expired(&mut state);
        self.try_connect(&mut state);
        if state.socket.is_some()
            && state.socket_identity.is_none_or(|identity| {
                Self::trusted_socket_identity(self.path.as_path()) != Some(identity)
            })
        {
            state.socket = None;
            state.socket_identity = None;
        }
        if let Some(socket) = &state.socket {
            if socket.send(&frame).is_ok() {
                return Ok(EmitOutcome::Sent);
            }
            state.socket = None;
            state.socket_identity = None;
        }

        let bytes = frame;
        if bytes.len() > self.capacity_bytes {
            self.drops.increment(signal);
            return Ok(EmitOutcome::Dropped);
        }
        while state.queued_bytes.saturating_add(bytes.len()) > self.capacity_bytes
            || state.queue.len() >= self.capacity_frames
        {
            let Some(oldest) = state.queue.pop_front() else {
                break;
            };
            state.queued_bytes = state.queued_bytes.saturating_sub(oldest.bytes.len());
            self.drops.increment(oldest.signal);
        }
        state.queued_bytes += bytes.len();
        state.queue.push_back(QueuedFrame {
            signal,
            bytes,
            enqueued_at: Instant::now(),
            attempts: 0,
        });
        Ok(EmitOutcome::Buffered)
    }

    /// Validate and emit one compact metric frame.
    ///
    /// The descriptor and identity canaries are checked before serialization,
    /// queue admission, or socket I/O. This is the emitter-side defense in
    /// depth for callers that have a typed metric descriptor.
    pub fn emit_metric<T: serde::Serialize>(
        &self,
        descriptor: &MetricDescriptor,
        labels: &BTreeMap<String, String>,
        canaries: &IdentityCanaries,
        value: &T,
    ) -> Result<EmitOutcome, EmitterError> {
        validate_data_point(descriptor, labels, canaries).map_err(EmitterError::MetricPolicy)?;
        let frame = encode_frame(
            Signal::Metric,
            &serde_json::json!({
                "name": descriptor.name(),
                "labels": labels,
                "value": value,
            }),
        )
        .map_err(|_| EmitterError::MetricPolicy(MetricPolicyError::DescriptorMalformed))?;
        self.emit(Signal::Metric, &frame)
    }

    /// Try to reconnect and drain buffered frames in FIFO order.
    pub fn drain(&self) -> Result<usize, EmitterError> {
        let mut state = self.state.lock().map_err(|_| EmitterError::StatePoisoned)?;
        self.prune_expired(&mut state);
        self.try_connect(&mut state);
        if state.socket.is_some()
            && state.socket_identity.is_none_or(|identity| {
                Self::trusted_socket_identity(self.path.as_path()) != Some(identity)
            })
        {
            state.socket = None;
            state.socket_identity = None;
        }
        let mut sent = 0;
        let mut drained_bytes = 0usize;
        while let Some(frame) = state.queue.front() {
            if sent >= self.capacity_frames || drained_bytes >= self.capacity_bytes {
                break;
            }
            let Some(socket) = &state.socket else {
                break;
            };
            if socket.send(&frame.bytes).is_err() {
                if let Some(frame) = state.queue.front_mut() {
                    frame.attempts = frame.attempts.saturating_add(1);
                    if frame.attempts >= self.max_retry_attempts {
                        let frame = state.queue.pop_front().expect("front was present");
                        state.queued_bytes = state.queued_bytes.saturating_sub(frame.bytes.len());
                        self.drops.increment(frame.signal);
                    }
                }
                state.socket = None;
                state.socket_identity = None;
                break;
            }
            let frame = state.queue.pop_front().expect("front was present");
            state.queued_bytes = state.queued_bytes.saturating_sub(frame.bytes.len());
            drained_bytes = drained_bytes.saturating_add(frame.bytes.len());
            sent += 1;
        }
        Ok(sent)
    }

    /// Number of frames currently buffered.
    pub fn buffered_frames(&self) -> Result<usize, EmitterError> {
        self.state
            .lock()
            .map(|state| state.queue.len())
            .map_err(|_| EmitterError::StatePoisoned)
    }

    /// Number of bytes currently retained in the ring.
    pub fn buffered_bytes(&self) -> Result<usize, EmitterError> {
        self.state
            .lock()
            .map(|state| state.queued_bytes)
            .map_err(|_| EmitterError::StatePoisoned)
    }

    /// Maximum retained frame count.
    pub const fn max_frames(&self) -> usize {
        self.capacity_frames
    }

    /// Maximum retained frame age.
    pub const fn max_age(&self) -> Duration {
        self.max_age
    }

    /// Snapshot drop counters.
    pub fn drops(&self) -> DropSnapshot {
        DropSnapshot {
            metric: self.drops.metric.load(Ordering::Relaxed),
            trace: self.drops.trace.load(Ordering::Relaxed),
            log: self.drops.log.load(Ordering::Relaxed),
        }
    }

    /// Return the configured path for integration-owned socket setup.
    pub fn socket_path(&self) -> &Path {
        self.path.as_path()
    }

    fn try_connect(&self, state: &mut State) {
        if state.socket.is_some() {
            return;
        }
        let Some(identity) = Self::trusted_socket_identity(self.path.as_path()) else {
            return;
        };
        let Ok(socket) = UnixDatagram::unbound() else {
            return;
        };
        // Export is deliberately best-effort. A blocking datagram send can
        // otherwise pin a core process behind a stalled collector, defeating
        // the bounded-ring failure mode.
        if socket.set_nonblocking(true).is_err() {
            return;
        }
        if socket.connect(self.path.as_path()).is_ok()
            && Self::trusted_socket_identity(self.path.as_path())
                .is_some_and(|current| current == identity)
        {
            state.socket_identity = Some(identity);
            state.socket = Some(socket);
        }
    }

    fn trusted_socket_identity(path: &Path) -> Option<(u64, u64, u32, u32)> {
        let metadata = std::fs::symlink_metadata(path).ok()?;
        let parent = path
            .parent()
            .and_then(|parent| std::fs::symlink_metadata(parent).ok())?;
        if parent.file_type().is_symlink()
            || !parent.is_dir()
            || parent.permissions().mode() & 0o022 != 0
        {
            return None;
        }
        if !metadata.file_type().is_socket()
            || metadata.permissions().mode() & 0o777 != 0o660
            || metadata.uid() != parent.uid()
            || metadata.gid() != parent.gid()
        {
            return None;
        }
        Some((
            metadata.dev(),
            metadata.ino(),
            metadata.uid(),
            metadata.gid(),
        ))
    }

    fn prune_expired(&self, state: &mut State) {
        while state
            .queue
            .front()
            .is_some_and(|frame| frame.enqueued_at.elapsed() >= self.max_age)
        {
            let frame = state.queue.pop_front().expect("front was present");
            state.queued_bytes = state.queued_bytes.saturating_sub(frame.bytes.len());
            self.drops.increment(frame.signal);
        }
    }
}

/// Encode a compact JSON telemetry frame without exposing transport details.
pub fn encode_frame<T: serde::Serialize>(signal: Signal, value: &T) -> Result<Vec<u8>, io::Error> {
    serde_json::to_vec(&serde_json::json!({
        "signal": signal.as_str(),
        "value": value,
    }))
    .map_err(io::Error::other)
}

fn validate_metric_frame(frame: &TelemetryFrame) -> Result<(), EmitterError> {
    if frame.signal != TelemetrySignal::Metric {
        return Err(EmitterError::FrameRedaction);
    }
    let Some(object) = frame.value.as_object() else {
        return Err(EmitterError::MetricPolicy(
            MetricPolicyError::DescriptorMalformed,
        ));
    };
    let name = object
        .get("name")
        .and_then(serde_json::Value::as_str)
        .ok_or(EmitterError::MetricPolicy(
            MetricPolicyError::DescriptorMalformed,
        ))?;
    if name.is_empty()
        || name.len() > 128
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    {
        return Err(EmitterError::MetricPolicy(
            MetricPolicyError::DescriptorMalformed,
        ));
    }
    let descriptor = canonical_descriptor(name).ok_or(EmitterError::MetricPolicy(
        MetricPolicyError::DescriptorNotAllowlisted,
    ))?;
    let labels = object
        .get("labels")
        .and_then(serde_json::Value::as_object)
        .ok_or(EmitterError::MetricPolicy(
            MetricPolicyError::DescriptorMalformed,
        ))?;
    let labels = labels
        .iter()
        .map(|(key, value)| {
            value
                .as_str()
                .map(|value| (key.clone(), value.to_owned()))
                .ok_or(EmitterError::MetricPolicy(
                    MetricPolicyError::ValueNotAllowlisted,
                ))
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    validate_canonical_data_point(&descriptor, &labels, &IdentityCanaries::default())
        .map_err(EmitterError::MetricPolicy)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        os::unix::net::UnixDatagram,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn socket_path(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!("d2b-t-{}-{nonce}", std::process::id()));
        fs::create_dir(&directory).unwrap();
        directory.join(format!("e-{label}.sock"))
    }

    fn cleanup_socket(path: PathBuf) {
        let parent = path.parent().map(Path::to_path_buf);
        let _ = fs::remove_file(path);
        if let Some(parent) = parent {
            let _ = fs::remove_dir(parent);
        }
    }

    fn redact_frame(frame: &[u8]) -> Result<Vec<u8>, EmitterError> {
        let parsed = parse_raw_frame(frame).map_err(|_| EmitterError::FrameRedaction)?;
        validate_frame(&parsed).map_err(|_| EmitterError::FrameRedaction)?;
        redact_parsed_frame(parsed).map_err(|_| EmitterError::FrameRedaction)
    }

    #[test]
    fn frames_buffer_then_drain_fifo_when_socket_appears() {
        let path = socket_path("fifo");
        let emitter = BoundedEmitter::new(&path, 512).unwrap();
        let one = encode_frame(Signal::Trace, &serde_json::json!({"event": "accepted"})).unwrap();
        let two = encode_frame(Signal::Trace, &serde_json::json!({"event": "rejected"})).unwrap();
        let one_redacted = redact_frame(&one).unwrap();
        let two_redacted = redact_frame(&two).unwrap();
        assert_eq!(
            emitter.emit(Signal::Trace, &one).unwrap(),
            EmitOutcome::Buffered
        );
        assert_eq!(
            emitter.emit(Signal::Trace, &two).unwrap(),
            EmitOutcome::Buffered
        );

        let receiver = UnixDatagram::bind(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o660)).unwrap();
        assert_eq!(emitter.drain().unwrap(), 2);
        let mut first = [0_u8; 128];
        let mut second = [0_u8; 128];
        let first_len = receiver.recv(&mut first).unwrap();
        let second_len = receiver.recv(&mut second).unwrap();
        assert_eq!(&first[..first_len], &one_redacted);
        assert_eq!(&second[..second_len], &two_redacted);
        drop(receiver);
        cleanup_socket(path);
    }

    #[test]
    fn ring_full_drops_oldest_frame_and_counts_its_signal() {
        let path = socket_path("drop");
        let emitter = BoundedEmitter::new(&path, 100).unwrap();
        let metric = encode_frame(
            Signal::Metric,
            &serde_json::json!({
                "name": "d2b_api_watch_active",
                "labels": {},
                "value": 1,
            }),
        )
        .unwrap();
        let log = encode_frame(Signal::Log, &serde_json::json!({"event": "buffer"})).unwrap();
        emitter.emit(Signal::Metric, &metric).unwrap();
        emitter.emit(Signal::Log, &log).unwrap();
        assert_eq!(emitter.buffered_frames().unwrap(), 1);
        assert_eq!(emitter.drops().metric, 1);
        cleanup_socket(path);
    }

    #[test]
    fn metric_emission_rejects_an_out_of_policy_label() {
        let path = socket_path("policy");
        let emitter = BoundedEmitter::new(&path, 128).unwrap();
        let descriptor = MetricDescriptor::new(
            "d2b_api_watch_active",
            [crate::meter_registry::label("vm", &["work"])],
        );
        let labels = BTreeMap::from([("vm".to_owned(), "work".to_owned())]);
        assert_eq!(
            emitter
                .emit_metric(&descriptor, &labels, &IdentityCanaries::default(), &1_u64)
                .unwrap_err(),
            EmitterError::MetricPolicy(MetricPolicyError::KeyForbidden)
        );
        assert_eq!(emitter.buffered_frames().unwrap(), 0);
        cleanup_socket(path);
    }

    #[test]
    fn raw_metric_frames_are_checked_before_buffer_admission() {
        let path = socket_path("raw-policy");
        let emitter = BoundedEmitter::new(&path, 128).unwrap();
        let frame = encode_frame(
            Signal::Metric,
            &serde_json::json!({
                "name": "d2b_api_watch_active",
                "labels": {"zone": "work"},
                "value": 1,
            }),
        )
        .unwrap();
        assert_eq!(
            emitter.emit(Signal::Metric, &frame).unwrap_err(),
            EmitterError::MetricPolicy(MetricPolicyError::KeyForbidden)
        );
        assert_eq!(emitter.buffered_frames().unwrap(), 0);
        cleanup_socket(path);
    }

    #[test]
    fn raw_metric_frames_preserve_the_max_label_guard() {
        let path = socket_path("max-labels");
        let emitter = BoundedEmitter::new(&path, 512).unwrap();
        let labels = d2b_contracts::v3::telemetry_policy::METRIC_LABEL_POLICY
            .iter()
            .take(17)
            .map(|(key, values)| {
                (
                    (*key).to_owned(),
                    serde_json::Value::String(values[0].to_owned()),
                )
            })
            .collect::<serde_json::Map<_, _>>();
        let frame = encode_frame(
            Signal::Metric,
            &serde_json::json!({
                "name": "d2b_api_request_total",
                "labels": labels,
                "value": 1,
            }),
        )
        .unwrap();

        assert_eq!(
            emitter.emit(Signal::Metric, &frame),
            Err(EmitterError::MetricPolicy(
                MetricPolicyError::DescriptorMalformed
            ))
        );
        assert_eq!(emitter.buffered_frames().unwrap(), 0);
        cleanup_socket(path);
    }

    #[test]
    fn canonical_metric_frames_are_admitted() {
        let path = socket_path("canonical-metric");
        let emitter = BoundedEmitter::new(&path, 512).unwrap();
        let frame = encode_frame(
            Signal::Metric,
            &serde_json::json!({
                "name": "d2b_api_watch_active",
                "labels": {},
                "value": 1,
            }),
        )
        .unwrap();
        assert_eq!(
            emitter.emit(Signal::Metric, &frame).unwrap(),
            EmitOutcome::Buffered
        );
        cleanup_socket(path);
    }

    #[test]
    fn raw_metric_frames_require_a_canonical_descriptor() {
        let path = socket_path("invalid-descriptor");
        let emitter = BoundedEmitter::new(&path, 512).unwrap();
        let frame = encode_frame(
            Signal::Metric,
            &serde_json::json!({
                "name": "d2b_unregistered_total",
                "labels": {"outcome": "ok"},
                "value": 1,
            }),
        )
        .unwrap();
        assert!(matches!(
            emitter.emit(Signal::Metric, &frame),
            Err(EmitterError::MetricPolicy(_))
        ));
        assert_eq!(emitter.buffered_frames().unwrap(), 0);
        cleanup_socket(path);
    }

    #[test]
    fn expected_signal_is_checked_before_metric_shape() {
        let path = socket_path("wrong-signal");
        let emitter = BoundedEmitter::new(&path, 512).unwrap();
        let frame = encode_frame(Signal::Trace, &serde_json::json!({"event": "accepted"})).unwrap();
        assert_eq!(
            emitter.emit(Signal::Metric, &frame),
            Err(EmitterError::FrameRedaction)
        );
        assert_eq!(emitter.buffered_frames().unwrap(), 0);
        cleanup_socket(path);
    }

    #[test]
    fn emit_parses_one_shared_frame_for_metric_admission_and_redaction() {
        let path = socket_path("single-parse");
        let emitter = BoundedEmitter::new(&path, 512).unwrap();
        let frame = encode_frame(
            Signal::Metric,
            &serde_json::json!({
                "name": "d2b_api_watch_active",
                "labels": {},
                "value": 1,
            }),
        )
        .unwrap();
        RAW_FRAME_PARSE_COUNT.with(|count| count.set(0));
        assert_eq!(
            emitter.emit(Signal::Metric, &frame).unwrap(),
            EmitOutcome::Buffered
        );
        assert_eq!(raw_frame_parse_count(), 1);
        cleanup_socket(path);
    }

    #[test]
    fn raw_observation_frames_are_rejected_before_retention() {
        let path = socket_path("raw-redaction");
        let emitter = BoundedEmitter::new(&path, 128).unwrap();
        assert_eq!(
            emitter.emit(Signal::Log, b"attacker-canary"),
            Err(EmitterError::FrameRedaction)
        );
        assert_eq!(emitter.buffered_frames().unwrap(), 0);
        cleanup_socket(path);
    }

    #[test]
    fn forbidden_observation_fields_are_rejected_before_retention() {
        let path = socket_path("forbidden-field");
        let emitter = BoundedEmitter::new(&path, 128).unwrap();
        let frame = encode_frame(
            Signal::Trace,
            &serde_json::json!({
                "event": "accepted",
                "extra": "forbidden",
            }),
        )
        .unwrap();
        assert_eq!(
            emitter.emit(Signal::Trace, &frame),
            Err(EmitterError::FrameRedaction)
        );
        assert_eq!(emitter.buffered_frames().unwrap(), 0);
        cleanup_socket(path);
    }

    #[test]
    fn raw_oversize_is_rejected_before_parse_or_queue_eviction() {
        let path = socket_path("raw-oversize");
        let emitter = BoundedEmitter::new(&path, 128).unwrap();
        let valid = encode_frame(Signal::Trace, &serde_json::json!({"event": "accepted"})).unwrap();
        emitter.emit(Signal::Trace, &valid).unwrap();
        let oversized = vec![b'x'; MAX_FRAME_BYTES + 1];
        assert_eq!(
            emitter.emit(Signal::Trace, &oversized),
            Err(EmitterError::FrameTooLarge)
        );
        assert_eq!(emitter.buffered_frames().unwrap(), 1);
        cleanup_socket(path);
    }

    #[test]
    fn identity_values_are_redacted_before_socket_export() {
        let path = socket_path("redaction");
        let emitter = BoundedEmitter::new(&path, 512).unwrap();
        let frame = encode_frame(
            Signal::Trace,
            &serde_json::json!({
                "d2b.zone": "zone-secret-canary",
                "path": "/private/host/path",
                "env": {"TOKEN": "secret-token-canary"},
                "event": "accepted",
            }),
        )
        .unwrap();
        let receiver = UnixDatagram::bind(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o660)).unwrap();
        assert_eq!(
            emitter.emit(Signal::Trace, &frame).unwrap(),
            EmitOutcome::Sent
        );
        let mut bytes = [0_u8; 512];
        let length = receiver.recv(&mut bytes).unwrap();
        let rendered = String::from_utf8(bytes[..length].to_vec()).unwrap();
        assert!(!rendered.contains("zone-secret-canary"));
        assert!(!rendered.contains("/private/host/path"));
        assert!(!rendered.contains("secret-token-canary"));
        drop(receiver);
        cleanup_socket(path);
    }

    #[test]
    fn count_and_age_bounds_prune_deterministically() {
        let path = socket_path("bounds");
        let emitter =
            BoundedEmitter::new_with_limits(&path, 512, 1, Duration::from_millis(1), 1).unwrap();
        let first = encode_frame(Signal::Log, &serde_json::json!({"event": "accepted"})).unwrap();
        let second = encode_frame(Signal::Log, &serde_json::json!({"event": "rejected"})).unwrap();
        emitter.emit(Signal::Log, &first).unwrap();
        emitter.emit(Signal::Log, &second).unwrap();
        assert_eq!(emitter.buffered_frames().unwrap(), 1);
        std::thread::sleep(Duration::from_millis(2));
        assert_eq!(emitter.drain().unwrap(), 0);
        assert_eq!(emitter.buffered_frames().unwrap(), 0);
        assert!(emitter.drops().log >= 2);
        cleanup_socket(path);
    }
}
