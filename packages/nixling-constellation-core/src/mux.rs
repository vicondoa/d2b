//! A pure, synchronous stream-mux state machine (ADR 0032).
//!
//! No I/O lives here: the transport layer drives the mux by feeding it
//! decoded [`crate::frame`] frames and emitting the control frames it asks
//! for. The mux enforces the fail-closed multiplexing contract the P0
//! design panel required *before* any Relay/Waypipe wiring exists:
//!
//! - a stream must be **opened** (authz-consistent, operation-bound) before
//!   any data flows on it;
//! - inbound chunks are **strictly sequential** — a gap or a replay is
//!   rejected, never silently reordered;
//! - a sender may only spend **credit the receiver granted** (credit-based
//!   backpressure), counted in frames;
//! - a chunk's [`StreamChannel`] must be **valid for the stream kind**
//!   (only `Stdio` carries `Stdout`/`Stderr`; every other kind is
//!   `Primary`-only);
//! - a resume [`crate::ids::StreamCursor`] only rides a `Logs` stream;
//! - data after a close, or a double close, is rejected.
//!
//! The state machine is deliberately codec- and transport-neutral so the
//! same enforcement runs identically over AF_VSOCK (host↔gateway) and over
//! an Azure Relay peer session (gateway↔container).

use std::collections::HashMap;

use crate::error::{ConstellationError, ErrorKind};
use crate::frame::{StreamClose, StreamData, StreamFlow, StreamOpen};
use crate::ids::{OperationId, StreamId};
use crate::stream::{StreamChannel, StreamCloseReason, StreamKind};

/// Default cap on concurrently-open streams in one peer session. Bounds the
/// mux's memory against a peer that opens streams without closing them.
pub const DEFAULT_MAX_OPEN_STREAMS: usize = 256;

/// Lifecycle state of a single multiplexed stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StreamState {
    Open,
    Closed,
}

/// Per-stream bookkeeping. Tracks both the inbound (receive-and-enforce)
/// and outbound (send-budget) halves so one [`StreamMux`] models one
/// endpoint's full view of a bidirectional stream.
#[derive(Debug, Clone)]
struct StreamEntry {
    kind: StreamKind,
    operation_id: OperationId,
    state: StreamState,
    close_reason: Option<StreamCloseReason>,
    /// Next inbound sequence number this endpoint will accept.
    expected_seq: u64,
    /// Inbound frames this endpoint has granted the peer and not yet seen
    /// consumed (credit the peer may still spend sending to us).
    recv_credit: u64,
    /// Next outbound sequence number this endpoint will assign.
    next_send_seq: u64,
    /// Outbound frames this endpoint may still send (credit the peer
    /// granted us).
    send_credit: u64,
}

/// A pure stream-mux state machine for one peer-session endpoint.
#[derive(Debug, Clone)]
pub struct StreamMux {
    streams: HashMap<StreamId, StreamEntry>,
    max_open: usize,
}

impl Default for StreamMux {
    fn default() -> Self {
        Self::new()
    }
}

impl StreamMux {
    /// A mux with the default open-stream cap.
    pub fn new() -> Self {
        Self::with_max_open(DEFAULT_MAX_OPEN_STREAMS)
    }

    /// A mux with an explicit open-stream cap (must be non-zero).
    pub fn with_max_open(max_open: usize) -> Self {
        Self {
            streams: HashMap::new(),
            max_open: max_open.max(1),
        }
    }

    /// Number of streams currently in the `Open` state.
    pub fn open_stream_count(&self) -> usize {
        self.streams
            .values()
            .filter(|e| e.state == StreamState::Open)
            .count()
    }

    /// Whether `stream` is known and currently open.
    pub fn is_open(&self, stream: &StreamId) -> bool {
        self.streams
            .get(stream)
            .is_some_and(|e| e.state == StreamState::Open)
    }

    /// The recorded close reason for a closed stream, if any.
    pub fn close_reason(&self, stream: &StreamId) -> Option<StreamCloseReason> {
        self.streams.get(stream).and_then(|e| e.close_reason)
    }

    /// Register a newly-opened stream.
    ///
    /// Fails closed when the open is authz-inconsistent (capability does
    /// not match the descriptor kind), when the stream id is already in
    /// use, or when the open-stream cap would be exceeded. The stream
    /// starts with **zero** credit in both directions: the receiver must
    /// [`grant_inbound`](Self::grant_inbound) before the peer may send, and
    /// this endpoint may only send after it [`receive_flow`](Self::receive_flow)s.
    pub fn open(&mut self, open: &StreamOpen) -> Result<(), ConstellationError> {
        if !open.is_consistent() {
            return Err(ConstellationError::new(
                ErrorKind::Unauthorized,
                "stream open authz capability does not match the descriptor kind",
            ));
        }
        let id = &open.descriptor.id;
        if self.streams.contains_key(id) {
            return Err(ConstellationError::new(
                ErrorKind::MalformedFrame,
                "stream id is already open",
            ));
        }
        if self.open_stream_count() >= self.max_open {
            return Err(ConstellationError::new(
                ErrorKind::Backpressure,
                "peer session exceeds the open-stream cap",
            ));
        }
        self.streams.insert(
            id.clone(),
            StreamEntry {
                kind: open.descriptor.kind,
                operation_id: open.operation_id.clone(),
                state: StreamState::Open,
                close_reason: None,
                expected_seq: 0,
                recv_credit: 0,
                next_send_seq: 0,
                send_credit: 0,
            },
        );
        Ok(())
    }

    /// Grant the peer additional inbound credit on `stream` and produce the
    /// [`StreamFlow`] frame to send. The grant must be non-zero.
    pub fn grant_inbound(
        &mut self,
        stream: &StreamId,
        credits: u32,
    ) -> Result<StreamFlow, ConstellationError> {
        if credits == 0 {
            return Err(ConstellationError::new(
                ErrorKind::MalformedFrame,
                "credit grant must be non-zero",
            ));
        }
        let entry = self.open_entry_mut(stream)?;
        entry.recv_credit = entry.recv_credit.saturating_add(u64::from(credits));
        Ok(StreamFlow {
            stream: stream.clone(),
            credits,
        })
    }

    /// Validate and account for an inbound data chunk.
    ///
    /// Enforces (fail-closed): the stream is open; the channel is valid for
    /// the kind; a cursor only rides a `Logs` stream; the sequence is
    /// exactly the next expected (no gap, no replay); and the peer holds
    /// credit. On success, one credit unit is consumed and the expected
    /// sequence advances.
    pub fn accept_data(&mut self, data: &StreamData) -> Result<(), ConstellationError> {
        // Validate kind-dependent shape before touching credit/sequence so a
        // malformed chunk never perturbs accounting.
        let kind = self.open_entry(&data.stream)?.kind;
        if !channel_valid_for_kind(kind, data.channel) {
            return Err(ConstellationError::new(
                ErrorKind::MalformedFrame,
                "stream channel is not valid for the stream kind",
            ));
        }
        if data.cursor.is_some() && kind != StreamKind::Logs {
            return Err(ConstellationError::new(
                ErrorKind::MalformedFrame,
                "only a logs stream may carry a resume cursor",
            ));
        }
        let entry = self.open_entry_mut(&data.stream)?;
        if data.sequence != entry.expected_seq {
            return Err(ConstellationError::new(
                ErrorKind::MalformedFrame,
                "stream data sequence is not the next expected value",
            ));
        }
        if entry.recv_credit == 0 {
            return Err(ConstellationError::new(
                ErrorKind::Backpressure,
                "stream data exceeds granted credit",
            ));
        }
        entry.recv_credit -= 1;
        entry.expected_seq += 1;
        Ok(())
    }

    /// Account for a [`StreamFlow`] the peer sent us (outbound credit we may
    /// now spend on `stream`).
    pub fn receive_flow(&mut self, flow: &StreamFlow) -> Result<(), ConstellationError> {
        if flow.credits == 0 {
            return Err(ConstellationError::new(
                ErrorKind::MalformedFrame,
                "credit grant must be non-zero",
            ));
        }
        let entry = self.open_entry_mut(&flow.stream)?;
        entry.send_credit = entry.send_credit.saturating_add(u64::from(flow.credits));
        Ok(())
    }

    /// Reserve the next outbound chunk on `stream`: validates the channel
    /// for the kind, spends one outbound credit, and returns the sequence
    /// number to stamp on the [`StreamData`]. Fails closed when the stream
    /// is not open, the channel is invalid, or no outbound credit remains.
    pub fn reserve_send(
        &mut self,
        stream: &StreamId,
        channel: StreamChannel,
    ) -> Result<u64, ConstellationError> {
        let kind = self.open_entry(stream)?.kind;
        if !channel_valid_for_kind(kind, channel) {
            return Err(ConstellationError::new(
                ErrorKind::MalformedFrame,
                "stream channel is not valid for the stream kind",
            ));
        }
        let entry = self.open_entry_mut(stream)?;
        if entry.send_credit == 0 {
            return Err(ConstellationError::new(
                ErrorKind::Backpressure,
                "no outbound credit remains on the stream",
            ));
        }
        entry.send_credit -= 1;
        let seq = entry.next_send_seq;
        entry.next_send_seq += 1;
        Ok(seq)
    }

    /// Close a stream, recording the reason. A double close is rejected.
    pub fn close(&mut self, close: &StreamClose) -> Result<(), ConstellationError> {
        let entry = self.streams.get_mut(&close.stream).ok_or_else(|| {
            ConstellationError::new(ErrorKind::MalformedFrame, "close of an unknown stream")
        })?;
        if entry.state == StreamState::Closed {
            return Err(ConstellationError::new(
                ErrorKind::MalformedFrame,
                "stream is already closed",
            ));
        }
        entry.state = StreamState::Closed;
        entry.close_reason = Some(close.reason);
        Ok(())
    }

    /// The operation id a stream is bound to (for audit/correlation).
    pub fn operation_id(&self, stream: &StreamId) -> Option<&OperationId> {
        self.streams.get(stream).map(|e| &e.operation_id)
    }

    fn open_entry(&self, stream: &StreamId) -> Result<&StreamEntry, ConstellationError> {
        match self.streams.get(stream) {
            Some(e) if e.state == StreamState::Open => Ok(e),
            Some(_) => Err(ConstellationError::new(
                ErrorKind::Cancelled,
                "stream is closed",
            )),
            None => Err(ConstellationError::new(
                ErrorKind::MalformedFrame,
                "reference to an unknown stream",
            )),
        }
    }

    fn open_entry_mut(
        &mut self,
        stream: &StreamId,
    ) -> Result<&mut StreamEntry, ConstellationError> {
        match self.streams.get_mut(stream) {
            Some(e) if e.state == StreamState::Open => Ok(e),
            Some(_) => Err(ConstellationError::new(
                ErrorKind::Cancelled,
                "stream is closed",
            )),
            None => Err(ConstellationError::new(
                ErrorKind::MalformedFrame,
                "reference to an unknown stream",
            )),
        }
    }
}

/// Whether `channel` is a legal sub-channel for `kind`. A non-TTY `Stdio`
/// stream multiplexes `Stdout`/`Stderr` and MUST NOT use `Primary`; every
/// other kind carries a single `Primary` channel.
fn channel_valid_for_kind(kind: StreamKind, channel: StreamChannel) -> bool {
    match kind {
        StreamKind::Stdio => matches!(channel, StreamChannel::Stdout | StreamChannel::Stderr),
        _ => channel == StreamChannel::Primary,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::StreamOpen;
    use crate::ids::StreamCursor;
    use crate::payload::OpaquePayload;
    use crate::realm::RealmPath;
    use crate::stream::{StreamAuthz, StreamDescriptor};

    fn sid(s: &str) -> StreamId {
        StreamId::parse(s).unwrap()
    }

    fn opid() -> OperationId {
        OperationId::parse("op-1").unwrap()
    }

    fn open_frame(id: &str, kind: StreamKind) -> StreamOpen {
        let principal = crate::ids::PrincipalId::parse("principal-1").unwrap();
        StreamOpen {
            descriptor: StreamDescriptor { id: sid(id), kind },
            operation_id: opid(),
            authz: StreamAuthz::for_kind(principal, RealmPath::local(), kind),
        }
    }

    fn data(id: &str, sequence: u64, channel: StreamChannel) -> StreamData {
        StreamData {
            stream: sid(id),
            sequence,
            channel,
            cursor: None,
            data: OpaquePayload::new(b"x".to_vec()).unwrap(),
        }
    }

    #[test]
    fn open_then_credit_then_sequential_data_is_accepted() {
        let mut mux = StreamMux::new();
        mux.open(&open_frame("s1", StreamKind::Display)).unwrap();
        assert!(mux.is_open(&sid("s1")));
        // No credit yet: data is refused (backpressure).
        let err = mux
            .accept_data(&data("s1", 0, StreamChannel::Primary))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Backpressure);
        // Grant 2 credits, then two sequential chunks flow.
        let flow = mux.grant_inbound(&sid("s1"), 2).unwrap();
        assert_eq!(flow.credits, 2);
        mux.accept_data(&data("s1", 0, StreamChannel::Primary))
            .unwrap();
        mux.accept_data(&data("s1", 1, StreamChannel::Primary))
            .unwrap();
        // Credit is now exhausted again.
        assert_eq!(
            mux.accept_data(&data("s1", 2, StreamChannel::Primary))
                .unwrap_err()
                .kind(),
            ErrorKind::Backpressure
        );
    }

    #[test]
    fn data_before_open_is_rejected() {
        let mut mux = StreamMux::new();
        let err = mux
            .accept_data(&data("ghost", 0, StreamChannel::Primary))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::MalformedFrame);
    }

    #[test]
    fn sequence_gap_and_replay_are_rejected() {
        let mut mux = StreamMux::new();
        mux.open(&open_frame("s1", StreamKind::Display)).unwrap();
        mux.grant_inbound(&sid("s1"), 8).unwrap();
        mux.accept_data(&data("s1", 0, StreamChannel::Primary))
            .unwrap();
        // A gap (seq 2 when 1 expected) fails closed.
        assert_eq!(
            mux.accept_data(&data("s1", 2, StreamChannel::Primary))
                .unwrap_err()
                .kind(),
            ErrorKind::MalformedFrame
        );
        // A replay of seq 0 also fails closed.
        assert_eq!(
            mux.accept_data(&data("s1", 0, StreamChannel::Primary))
                .unwrap_err()
                .kind(),
            ErrorKind::MalformedFrame
        );
    }

    #[test]
    fn channel_must_match_kind() {
        let mut mux = StreamMux::new();
        mux.open(&open_frame("disp", StreamKind::Display)).unwrap();
        mux.grant_inbound(&sid("disp"), 4).unwrap();
        // Display is Primary-only: stdout is rejected.
        assert_eq!(
            mux.accept_data(&data("disp", 0, StreamChannel::Stdout))
                .unwrap_err()
                .kind(),
            ErrorKind::MalformedFrame
        );

        mux.open(&open_frame("io", StreamKind::Stdio)).unwrap();
        mux.grant_inbound(&sid("io"), 4).unwrap();
        // Stdio must use Stdout/Stderr, not Primary.
        assert_eq!(
            mux.accept_data(&data("io", 0, StreamChannel::Primary))
                .unwrap_err()
                .kind(),
            ErrorKind::MalformedFrame
        );
        // Stdout/Stderr on a Stdio stream are accepted.
        mux.accept_data(&data("io", 0, StreamChannel::Stdout))
            .unwrap();
        mux.accept_data(&data("io", 1, StreamChannel::Stderr))
            .unwrap();
    }

    #[test]
    fn cursor_only_on_logs() {
        let mut mux = StreamMux::new();
        mux.open(&open_frame("disp", StreamKind::Display)).unwrap();
        mux.grant_inbound(&sid("disp"), 4).unwrap();
        let mut chunk = data("disp", 0, StreamChannel::Primary);
        chunk.cursor = Some(StreamCursor::parse("cur-1").unwrap());
        assert_eq!(
            mux.accept_data(&chunk).unwrap_err().kind(),
            ErrorKind::MalformedFrame
        );

        mux.open(&open_frame("logs", StreamKind::Logs)).unwrap();
        mux.grant_inbound(&sid("logs"), 4).unwrap();
        let mut log_chunk = data("logs", 0, StreamChannel::Primary);
        log_chunk.cursor = Some(StreamCursor::parse("cur-1").unwrap());
        mux.accept_data(&log_chunk).unwrap();
    }

    #[test]
    fn duplicate_open_and_inconsistent_open_are_rejected() {
        let mut mux = StreamMux::new();
        mux.open(&open_frame("s1", StreamKind::Display)).unwrap();
        assert_eq!(
            mux.open(&open_frame("s1", StreamKind::Display))
                .unwrap_err()
                .kind(),
            ErrorKind::MalformedFrame
        );
        // An authz/kind-inconsistent open is refused.
        let mut bad = open_frame("s2", StreamKind::Display);
        bad.authz.capability = crate::capability::Capability::Clipboard;
        assert_eq!(mux.open(&bad).unwrap_err().kind(), ErrorKind::Unauthorized);
    }

    #[test]
    fn open_stream_cap_is_enforced() {
        let mut mux = StreamMux::with_max_open(1);
        mux.open(&open_frame("s1", StreamKind::Display)).unwrap();
        assert_eq!(
            mux.open(&open_frame("s2", StreamKind::Display))
                .unwrap_err()
                .kind(),
            ErrorKind::Backpressure
        );
        // Closing s1 frees a slot.
        mux.close(&StreamClose {
            stream: sid("s1"),
            reason: StreamCloseReason::Completed,
        })
        .unwrap();
        mux.open(&open_frame("s2", StreamKind::Display)).unwrap();
    }

    #[test]
    fn close_is_terminal_and_data_after_close_is_rejected() {
        let mut mux = StreamMux::new();
        mux.open(&open_frame("s1", StreamKind::Display)).unwrap();
        mux.grant_inbound(&sid("s1"), 4).unwrap();
        mux.close(&StreamClose {
            stream: sid("s1"),
            reason: StreamCloseReason::Cancelled,
        })
        .unwrap();
        assert_eq!(
            mux.close_reason(&sid("s1")),
            Some(StreamCloseReason::Cancelled)
        );
        // Data after close fails closed (stream no longer open).
        assert_eq!(
            mux.accept_data(&data("s1", 0, StreamChannel::Primary))
                .unwrap_err()
                .kind(),
            ErrorKind::Cancelled
        );
        // A double close is rejected.
        assert_eq!(
            mux.close(&StreamClose {
                stream: sid("s1"),
                reason: StreamCloseReason::Completed,
            })
            .unwrap_err()
            .kind(),
            ErrorKind::MalformedFrame
        );
    }

    #[test]
    fn outbound_send_respects_credit_and_assigns_sequence() {
        let mut mux = StreamMux::new();
        mux.open(&open_frame("s1", StreamKind::Display)).unwrap();
        // No outbound credit yet.
        assert_eq!(
            mux.reserve_send(&sid("s1"), StreamChannel::Primary)
                .unwrap_err()
                .kind(),
            ErrorKind::Backpressure
        );
        // Peer grants us 2 outbound credits.
        mux.receive_flow(&StreamFlow {
            stream: sid("s1"),
            credits: 2,
        })
        .unwrap();
        assert_eq!(
            mux.reserve_send(&sid("s1"), StreamChannel::Primary)
                .unwrap(),
            0
        );
        assert_eq!(
            mux.reserve_send(&sid("s1"), StreamChannel::Primary)
                .unwrap(),
            1
        );
        assert_eq!(
            mux.reserve_send(&sid("s1"), StreamChannel::Primary)
                .unwrap_err()
                .kind(),
            ErrorKind::Backpressure
        );
    }
}
