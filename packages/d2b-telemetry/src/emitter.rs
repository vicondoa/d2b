//! Bounded, best-effort Unix datagram telemetry emitter.

use std::{
    collections::{BTreeMap, VecDeque},
    io,
    os::unix::net::UnixDatagram,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use crate::metric_label_policy::{
    IdentityCanaries, MetricDescriptor, MetricPolicyError, validate_data_point, validate_labels,
};

/// Default frame limit for core-process telemetry.
pub const MAX_FRAME_BYTES: usize = 64 * 1024;
/// Default bounded ring capacity.
pub const DEFAULT_RING_CAPACITY_BYTES: usize = 4 * 1024 * 1024;

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
}

impl core::fmt::Display for EmitterError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::FrameTooLarge => "telemetry-frame-too-large",
            Self::StatePoisoned => "telemetry-emitter-state-poisoned",
            Self::MetricPolicy(_) => "telemetry-metric-policy-rejected",
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
}

struct State {
    socket: Option<UnixDatagram>,
    queue: VecDeque<QueuedFrame>,
    queued_bytes: usize,
}

/// A bounded FIFO emitter which never blocks the caller on export.
#[derive(Clone)]
pub struct BoundedEmitter {
    path: Arc<PathBuf>,
    capacity_bytes: usize,
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
        if capacity_bytes == 0 {
            return Err(EmitterError::StatePoisoned);
        }
        Ok(Self {
            path: Arc::new(path.into()),
            capacity_bytes,
            state: Arc::new(Mutex::new(State {
                socket: None,
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
        if signal == Signal::Metric {
            validate_metric_frame(frame)?;
        }
        if frame.len() > MAX_FRAME_BYTES {
            self.drops.increment(signal);
            return Err(EmitterError::FrameTooLarge);
        }
        let mut state = self.state.lock().map_err(|_| EmitterError::StatePoisoned)?;
        self.try_connect(&mut state);
        if let Some(socket) = &state.socket {
            if socket.send(frame).is_ok() {
                return Ok(EmitOutcome::Sent);
            }
            state.socket = None;
        }

        let bytes = frame.to_vec();
        if bytes.len() > self.capacity_bytes {
            self.drops.increment(signal);
            return Ok(EmitOutcome::Dropped);
        }
        while state.queued_bytes.saturating_add(bytes.len()) > self.capacity_bytes {
            let Some(oldest) = state.queue.pop_front() else {
                break;
            };
            state.queued_bytes = state.queued_bytes.saturating_sub(oldest.bytes.len());
            self.drops.increment(oldest.signal);
        }
        state.queued_bytes += bytes.len();
        state.queue.push_back(QueuedFrame { signal, bytes });
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
        self.try_connect(&mut state);
        let mut sent = 0;
        while let Some(frame) = state.queue.front() {
            let Some(socket) = &state.socket else {
                break;
            };
            if socket.send(&frame.bytes).is_err() {
                state.socket = None;
                break;
            }
            let frame = state.queue.pop_front().expect("front was present");
            state.queued_bytes = state.queued_bytes.saturating_sub(frame.bytes.len());
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
        let Ok(socket) = UnixDatagram::unbound() else {
            return;
        };
        // Export is deliberately best-effort. A blocking datagram send can
        // otherwise pin a core process behind a stalled collector, defeating
        // the bounded-ring failure mode.
        if socket.set_nonblocking(true).is_err() {
            return;
        }
        if socket.connect(self.path.as_path()).is_ok() {
            state.socket = Some(socket);
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

fn validate_metric_frame(frame: &[u8]) -> Result<(), EmitterError> {
    let value = serde_json::from_slice::<serde_json::Value>(frame)
        .map_err(|_| EmitterError::MetricPolicy(MetricPolicyError::DescriptorMalformed))?;
    let labels = value
        .get("labels")
        .or_else(|| value.get("value").and_then(|value| value.get("labels")));
    let Some(labels) = labels else {
        return Ok(());
    };
    let labels = labels.as_object().ok_or(EmitterError::MetricPolicy(
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
    validate_labels(&labels, &IdentityCanaries::default()).map_err(EmitterError::MetricPolicy)
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
        std::env::temp_dir().join(format!("d2b-telemetry-{label}-{nonce}.sock"))
    }

    #[test]
    fn frames_buffer_then_drain_fifo_when_socket_appears() {
        let path = socket_path("fifo");
        let emitter = BoundedEmitter::new(&path, 128).unwrap();
        assert_eq!(
            emitter.emit(Signal::Trace, b"one").unwrap(),
            EmitOutcome::Buffered
        );
        assert_eq!(
            emitter.emit(Signal::Trace, b"two").unwrap(),
            EmitOutcome::Buffered
        );

        let receiver = UnixDatagram::bind(&path).unwrap();
        assert_eq!(emitter.drain().unwrap(), 2);
        let mut first = [0_u8; 16];
        let mut second = [0_u8; 16];
        assert_eq!(receiver.recv(&mut first).unwrap(), 3);
        assert_eq!(&first[..3], b"one");
        assert_eq!(receiver.recv(&mut second).unwrap(), 3);
        assert_eq!(&second[..3], b"two");
        drop(receiver);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn ring_full_drops_oldest_frame_and_counts_its_signal() {
        let path = socket_path("drop");
        let emitter = BoundedEmitter::new(&path, 6).unwrap();
        emitter.emit(Signal::Metric, b"1234").unwrap();
        emitter.emit(Signal::Log, b"5678").unwrap();
        assert_eq!(emitter.buffered_frames().unwrap(), 1);
        assert_eq!(emitter.drops().metric, 1);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn metric_emission_rejects_an_out_of_policy_label() {
        let path = socket_path("policy");
        let emitter = BoundedEmitter::new(&path, 128).unwrap();
        let descriptor = MetricDescriptor::new(
            "d2b_test_total",
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
        let _ = fs::remove_file(path);
    }

    #[test]
    fn raw_metric_frames_are_checked_before_buffer_admission() {
        let path = socket_path("raw-policy");
        let emitter = BoundedEmitter::new(&path, 128).unwrap();
        let frame = encode_frame(
            Signal::Metric,
            &serde_json::json!({
                "name": "d2b_test_total",
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
        let _ = fs::remove_file(path);
    }
}
