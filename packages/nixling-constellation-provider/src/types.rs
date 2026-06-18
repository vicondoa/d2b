//! Provider DTOs (ADR 0032). Pure-data DTOs are `serde`; runtime handles
//! that carry live byte channels (transport sessions, mux substreams) are
//! deliberately NOT `serde`/`Clone`/`Eq` and redact their contents in
//! `Debug`.

use nixling_constellation_core::{
    ExecutionId, NodeId, OpaquePayload, OperationId, ProviderId, StreamAuthz, StreamCursor,
    StreamId, WorkloadId, WorkloadSelector,
};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncWrite};

pub use nixling_constellation_core::{StreamKind, StreamOpen};

/// A request to plan/run a workload, addressed by a stable alias.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkloadSpec {
    /// Stable operator-facing alias for the workload.
    pub alias: WorkloadId,
}

/// An opaque, provider-resolved runtime plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimePlan {
    /// Provider that produced the plan.
    pub provider: ProviderId,
    /// Workload the plan is for.
    pub workload: WorkloadId,
}

/// An opaque handle to a running runtime instance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeHandle {
    /// Workload the handle refers to.
    pub workload: WorkloadId,
}

/// Coarse runtime status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeStatus {
    /// Workload.
    pub workload: WorkloadId,
    /// Whether the runtime is currently running.
    pub running: bool,
}

/// Coarse workload status returned by a [`crate::WorkloadProvider`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkloadStatus {
    /// Workload.
    pub workload: WorkloadId,
    /// Whether the workload is currently running.
    pub running: bool,
}

/// A request to start an execution in a workload. The `command` is an
/// opaque, codec-defined payload (argv + env + stdio policy) so the
/// provider trait never has to model shell semantics; its bytes are never
/// logged/audited as content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecStartRequest {
    /// Workload to exec in.
    pub workload: WorkloadId,
    /// Whether a TTY is requested.
    pub tty: bool,
    /// Opaque, bounded command payload (argv/env/stdio descriptor).
    pub command: OpaquePayload,
}

/// A request to fetch (and optionally resume) the logs of an execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecLogsRequest {
    /// Workload the execution belongs to.
    pub workload: WorkloadId,
    /// Execution whose logs are requested.
    pub execution: ExecutionId,
    /// Resume from this durable cursor; `None` streams from the start.
    pub cursor: Option<StreamCursor>,
}

/// A request to cancel a running execution (idempotent at the provider).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecCancelRequest {
    /// Workload the execution belongs to.
    pub workload: WorkloadId,
    /// Execution to cancel.
    pub execution: ExecutionId,
}

/// A display-session id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisplaySessionId(pub String);

/// A request to open a display session for a workload. The request carries
/// the **authorized display-stream binding** the mux must already hold: a
/// Waypipe byte never flows until there is an accepted `StreamOpen` for
/// `display_stream` under `authz`, bound to `operation_id`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisplaySessionRequest {
    /// Workload presenting the UI.
    pub workload: WorkloadId,
    /// The operation that authorized this display session (audit +
    /// idempotency binding).
    pub operation_id: OperationId,
    /// The authorized display stream this session drives.
    pub display_stream: StreamId,
    /// The authorization context (principal/realm/derived capability) the
    /// gateway validated for the display stream.
    pub authz: StreamAuthz,
}

/// An opaque handle to an open display session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisplaySessionHandle {
    /// Session id.
    pub id: DisplaySessionId,
}

/// A selector used by [`crate::WorkloadProvider::list`].
pub type ListSelector = WorkloadSelector;

/// A node registration handle (transport listener side).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeRegistration {
    /// Node being registered.
    pub node: NodeId,
}

/// A transport-level target to connect to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransportTarget {
    /// Opaque transport endpoint reference (e.g. a relay rendezvous id).
    pub endpoint: String,
}

/// Maximum length of a [`SafeLabel`].
pub const MAX_LABEL_LEN: usize = 64;

/// A bounded, low-cardinality, non-secret diagnostic label. It MUST carry
/// a stable classification (e.g. `relay-session`, `loopback`), never an
/// endpoint, store path, argv, or secret. The length is bounded so it can
/// never become an unbounded/high-cardinality side channel.
#[derive(Clone, PartialEq, Eq)]
pub struct SafeLabel(String);

impl SafeLabel {
    /// Build a bounded label (truncated to [`MAX_LABEL_LEN`]).
    pub fn new(label: impl Into<String>) -> Self {
        let mut s = label.into();
        if s.len() > MAX_LABEL_LEN {
            let mut end = MAX_LABEL_LEN;
            while end > 0 && !s.is_char_boundary(end) {
                end -= 1;
            }
            s.truncate(end);
        }
        Self(s)
    }

    /// The label text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Debug for SafeLabel {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "SafeLabel({:?})", self.0)
    }
}

/// A bidirectional byte channel: a connected transport session or an
/// accepted mux substream. Implemented by any `AsyncRead + AsyncWrite`
/// (e.g. a `tokio::io::DuplexStream` for the loopback mock, or a relay
/// WebSocket adapter later).
pub trait ByteStream: AsyncRead + AsyncWrite + Send + Unpin {}
impl<T: AsyncRead + AsyncWrite + Send + Unpin + ?Sized> ByteStream for T {}

/// A connected transport session: a bidirectional byte channel below the
/// mux, plus a bounded non-secret label. Not `Clone`/`Eq`/`serde` (it owns
/// a live stream); `Debug` reveals only the label.
pub struct TransportSession {
    label: SafeLabel,
    stream: Box<dyn ByteStream>,
}

impl TransportSession {
    /// Wrap a connected byte stream with a bounded diagnostic label.
    pub fn new(label: SafeLabel, stream: Box<dyn ByteStream>) -> Self {
        Self { label, stream }
    }

    /// The non-secret diagnostic label.
    pub fn label(&self) -> &str {
        self.label.as_str()
    }

    /// Borrow the underlying byte channel.
    pub fn stream_mut(&mut self) -> &mut dyn ByteStream {
        &mut *self.stream
    }

    /// Take ownership of the underlying byte channel.
    pub fn into_stream(self) -> Box<dyn ByteStream> {
        self.stream
    }
}

impl core::fmt::Debug for TransportSession {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TransportSession")
            .field("label", &self.label)
            .finish_non_exhaustive()
    }
}

/// An opened mux substream: the authorized open plus its byte channel.
pub struct StreamHandle {
    /// The stream id.
    pub id: StreamId,
    stream: Box<dyn ByteStream>,
}

impl StreamHandle {
    /// Wrap an opened substream.
    pub fn new(id: StreamId, stream: Box<dyn ByteStream>) -> Self {
        Self { id, stream }
    }

    /// Borrow the underlying byte channel.
    pub fn stream_mut(&mut self) -> &mut dyn ByteStream {
        &mut *self.stream
    }

    /// Take ownership of the underlying byte channel.
    pub fn into_stream(self) -> Box<dyn ByteStream> {
        self.stream
    }
}

impl core::fmt::Debug for StreamHandle {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("StreamHandle")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

/// An accepted inbound mux substream: the (already-authorized) open and
/// its byte channel.
pub struct IncomingStream {
    /// The authorized stream open (descriptor + authz).
    pub open: StreamOpen,
    stream: Box<dyn ByteStream>,
}

impl IncomingStream {
    /// Wrap an accepted substream.
    pub fn new(open: StreamOpen, stream: Box<dyn ByteStream>) -> Self {
        Self { open, stream }
    }

    /// Take ownership of the underlying byte channel.
    pub fn into_stream(self) -> Box<dyn ByteStream> {
        self.stream
    }
}

impl core::fmt::Debug for IncomingStream {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("IncomingStream")
            .field("open", &self.open)
            .finish_non_exhaustive()
    }
}

/// A daemon-access transport mode (which `nixlingd` transport the CLI uses).
/// Only [`DaemonAccessMode::LocalUnix`] is implemented today; the others are
/// declared slots that fail closed with `UnsupportedFeature` until a later
/// wave implements them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum DaemonAccessMode {
    /// Local `public.sock` Unix-domain socket (current behavior).
    LocalUnix,
    /// Relay-backed (Azure Relay hybrid connection); later wave.
    Relay,
    /// Direct mTLS/QUIC/WebSocket; later wave.
    DirectTls,
    /// Explicit SSH bootstrap; later wave.
    SshBootstrap,
}

impl DaemonAccessMode {
    /// Whether this mode is implemented today.
    pub fn is_implemented(self) -> bool {
        matches!(self, DaemonAccessMode::LocalUnix)
    }
}
