//! Bounded named-stream bridge with byte credits and round-robin delivery.

use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Arc, Mutex, MutexGuard},
};

use crate::operations::SessionId;

/// Default maximum bytes credited to one stream.
pub const DEFAULT_MAX_STREAM_CREDIT: usize = 256 * 1024;
/// Default maximum queued bytes across the bus.
pub const DEFAULT_MAX_AGGREGATE_BYTES: usize = 4 * 1024 * 1024;
/// Default maximum named streams.
pub const DEFAULT_MAX_STREAMS: usize = 1024;
/// Default maximum bytes in one stream frame.
pub const DEFAULT_MAX_FRAME_BYTES: usize = 64 * 1024;

/// A bounded named-stream identifier.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StreamName(String);

impl StreamName {
    /// Parse a canonical stream name.
    pub fn parse(value: impl Into<String>) -> Result<Self, StreamError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > 128
            || !value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':')
            })
        {
            return Err(StreamError::InvalidName);
        }
        Ok(Self(value))
    }

    /// Borrow the exact name for an authorized encoding.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Debug for StreamName {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("StreamName(<redacted>)")
    }
}

/// Frozen limits for one stream bridge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamLimits {
    pub max_stream_credit: usize,
    pub max_aggregate_bytes: usize,
    pub max_streams: usize,
    pub max_frame_bytes: usize,
}

impl Default for StreamLimits {
    fn default() -> Self {
        Self {
            max_stream_credit: DEFAULT_MAX_STREAM_CREDIT,
            max_aggregate_bytes: DEFAULT_MAX_AGGREGATE_BYTES,
            max_streams: DEFAULT_MAX_STREAMS,
            max_frame_bytes: DEFAULT_MAX_FRAME_BYTES,
        }
    }
}

impl StreamLimits {
    pub(crate) fn validate(self) -> Result<Self, StreamError> {
        if self.max_stream_credit == 0
            || self.max_aggregate_bytes == 0
            || self.max_streams == 0
            || self.max_frame_bytes == 0
            || self.max_frame_bytes > self.max_stream_credit
        {
            return Err(StreamError::InvalidLimits);
        }
        Ok(self)
    }
}

struct StreamState {
    source: SessionId,
    destination: SessionId,
    credit: usize,
    frames: VecDeque<Vec<u8>>,
}

struct BridgeState {
    aggregate_bytes: usize,
    aggregate_credit: usize,
    streams: BTreeMap<StreamName, StreamState>,
    ready: VecDeque<StreamName>,
}

/// Shared bridge state. Handles expose only their direction-specific operations.
pub(crate) struct StreamBridge {
    limits: StreamLimits,
    state: Mutex<BridgeState>,
}

impl StreamBridge {
    pub(crate) fn new(limits: StreamLimits) -> Result<Arc<Self>, StreamError> {
        Ok(Arc::new(Self {
            limits: limits.validate()?,
            state: Mutex::new(BridgeState {
                aggregate_bytes: 0,
                aggregate_credit: 0,
                streams: BTreeMap::new(),
                ready: VecDeque::new(),
            }),
        }))
    }

    pub(crate) fn open(
        self: &Arc<Self>,
        name: StreamName,
        source: SessionId,
        destination: SessionId,
        initial_credit: usize,
    ) -> Result<(OutgoingStream, IncomingStream), StreamError> {
        if initial_credit == 0 || initial_credit > self.limits.max_stream_credit {
            return Err(StreamError::CreditExceeded);
        }
        let mut state = self.lock();
        if state.streams.len() >= self.limits.max_streams {
            return Err(StreamError::StreamCapacityExceeded);
        }
        if state.streams.contains_key(&name) {
            return Err(StreamError::DuplicateStream);
        }
        if state.aggregate_credit.saturating_add(initial_credit) > self.limits.max_aggregate_bytes {
            return Err(StreamError::AggregateBackpressure);
        }
        state.aggregate_credit += initial_credit;
        state.streams.insert(
            name.clone(),
            StreamState {
                source,
                destination,
                credit: initial_credit,
                frames: VecDeque::new(),
            },
        );
        drop(state);
        Ok((
            OutgoingStream {
                bridge: Arc::clone(self),
                name: name.clone(),
                source,
                closed: false,
            },
            IncomingStream {
                bridge: Arc::clone(self),
                owner_name: name,
                destination,
                closed: false,
            },
        ))
    }

    fn send(
        &self,
        name: &StreamName,
        source: SessionId,
        payload: Vec<u8>,
    ) -> Result<(), StreamError> {
        if payload.is_empty() || payload.len() > self.limits.max_frame_bytes {
            return Err(StreamError::FrameBounds);
        }
        let frame_len = payload.len();
        let mut state = self.lock();
        if state.aggregate_bytes.saturating_add(frame_len) > self.limits.max_aggregate_bytes {
            return Err(StreamError::AggregateBackpressure);
        }
        let stream = state
            .streams
            .get_mut(name)
            .ok_or(StreamError::StreamClosed)?;
        if stream.source != source {
            return Err(StreamError::DirectionMismatch);
        }
        if stream.credit < frame_len {
            return Err(StreamError::CreditExceeded);
        }
        let was_empty = stream.frames.is_empty();
        stream.credit -= frame_len;
        stream.frames.push_back(payload);
        state.aggregate_credit -= frame_len;
        state.aggregate_bytes += frame_len;
        if was_empty {
            state.ready.push_back(name.clone());
        }
        Ok(())
    }

    fn receive(&self, destination: SessionId) -> Result<ReceivedFrame, StreamError> {
        let mut state = self.lock();
        let candidates = state.ready.len();
        for _ in 0..candidates {
            let name = state
                .ready
                .pop_front()
                .expect("candidate count came from ready queue");
            let Some(stream) = state.streams.get_mut(&name) else {
                continue;
            };
            if stream.destination != destination {
                state.ready.push_back(name);
                continue;
            }
            let payload = stream
                .frames
                .pop_front()
                .expect("ready streams contain a frame");
            let has_more = !stream.frames.is_empty();
            state.aggregate_bytes -= payload.len();
            if has_more {
                state.ready.push_back(name.clone());
            }
            return Ok(ReceivedFrame {
                stream: name,
                payload,
            });
        }
        Err(StreamError::NoFrameAvailable)
    }

    fn grant(
        &self,
        name: &StreamName,
        destination: SessionId,
        bytes: usize,
    ) -> Result<(), StreamError> {
        if bytes == 0 {
            return Err(StreamError::CreditExceeded);
        }
        let mut state = self.lock();
        if state.aggregate_credit.saturating_add(bytes) > self.limits.max_aggregate_bytes {
            return Err(StreamError::AggregateBackpressure);
        }
        let stream = state
            .streams
            .get_mut(name)
            .ok_or(StreamError::StreamClosed)?;
        if stream.destination != destination {
            return Err(StreamError::DirectionMismatch);
        }
        stream.credit = stream
            .credit
            .checked_add(bytes)
            .filter(|credit| *credit <= self.limits.max_stream_credit)
            .ok_or(StreamError::CreditExceeded)?;
        state.aggregate_credit += bytes;
        Ok(())
    }

    fn close(&self, name: &StreamName) {
        let mut state = self.lock();
        if let Some(stream) = state.streams.remove(name) {
            let queued = stream.frames.iter().map(Vec::len).sum::<usize>();
            state.aggregate_bytes = state.aggregate_bytes.saturating_sub(queued);
            state.aggregate_credit = state.aggregate_credit.saturating_sub(stream.credit);
            state.ready.retain(|ready| ready != name);
        }
    }

    pub(crate) fn cancel_session(&self, session: SessionId) {
        let names = {
            let state = self.lock();
            state
                .streams
                .iter()
                .filter(|(_, stream)| stream.source == session || stream.destination == session)
                .map(|(name, _)| name.clone())
                .collect::<Vec<_>>()
        };
        for name in names {
            self.close(&name);
        }
    }

    fn lock(&self) -> MutexGuard<'_, BridgeState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// One frame selected by destination-wide round-robin scheduling.
#[derive(Clone, PartialEq, Eq)]
pub struct ReceivedFrame {
    stream: StreamName,
    payload: Vec<u8>,
}

impl ReceivedFrame {
    /// Borrow the stream name.
    pub const fn stream(&self) -> &StreamName {
        &self.stream
    }

    /// Borrow the frame bytes.
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }
}

impl core::fmt::Debug for ReceivedFrame {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ReceivedFrame")
            .field("stream", &self.stream)
            .field("payload_bytes", &self.payload.len())
            .finish()
    }
}

/// Source-side stream handle.
pub(crate) struct OutgoingStream {
    bridge: Arc<StreamBridge>,
    name: StreamName,
    source: SessionId,
    closed: bool,
}

impl OutgoingStream {
    pub(crate) fn name(&self) -> &StreamName {
        &self.name
    }

    pub(crate) fn send(&self, payload: Vec<u8>) -> Result<(), StreamError> {
        self.bridge.send(&self.name, self.source, payload)
    }

    pub(crate) fn close(&mut self) {
        if !self.closed {
            self.bridge.close(&self.name);
            self.closed = true;
        }
    }
}

impl Drop for OutgoingStream {
    fn drop(&mut self) {
        self.close();
    }
}

/// Destination-side bridge handle delivered to a registered endpoint.
pub struct IncomingStream {
    bridge: Arc<StreamBridge>,
    owner_name: StreamName,
    destination: SessionId,
    closed: bool,
}

impl IncomingStream {
    /// Borrow the stream that established this destination reader.
    pub const fn stream_name(&self) -> &StreamName {
        &self.owner_name
    }

    /// Receive the next fair frame for this destination across all named streams.
    pub async fn receive_next(&self) -> Result<ReceivedFrame, StreamError> {
        self.bridge.receive(self.destination)
    }

    /// Grant more byte credit to one named stream at this destination.
    pub async fn grant(&self, stream: &StreamName, bytes: usize) -> Result<(), StreamError> {
        self.bridge.grant(stream, self.destination, bytes)
    }

    /// Close the stream that established this reader.
    pub fn close(&mut self) {
        if !self.closed {
            self.bridge.close(&self.owner_name);
            self.closed = true;
        }
    }
}

impl core::fmt::Debug for IncomingStream {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("IncomingStream")
            .field("stream", &self.owner_name)
            .finish()
    }
}

impl Drop for IncomingStream {
    fn drop(&mut self) {
        self.close();
    }
}

/// Closed named-stream failure classes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamError {
    InvalidName,
    InvalidLimits,
    DuplicateStream,
    StreamCapacityExceeded,
    FrameBounds,
    CreditExceeded,
    AggregateBackpressure,
    DirectionMismatch,
    StreamClosed,
    NoFrameAvailable,
}

impl core::fmt::Display for StreamError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::InvalidName => "stream name is invalid",
            Self::InvalidLimits => "stream limits are invalid",
            Self::DuplicateStream => "stream name is already active",
            Self::StreamCapacityExceeded => "stream capacity is exhausted",
            Self::FrameBounds => "stream frame is outside the byte bounds",
            Self::CreditExceeded => "stream byte credit is insufficient",
            Self::AggregateBackpressure => "aggregate stream backpressure limit was reached",
            Self::DirectionMismatch => "stream operation came from the wrong endpoint",
            Self::StreamClosed => "stream is closed",
            Self::NoFrameAvailable => "no stream frame is available",
        })
    }
}

impl std::error::Error for StreamError {}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::{Barrier, mpsc};

    fn bridge(aggregate: usize) -> Arc<StreamBridge> {
        StreamBridge::new(StreamLimits {
            max_stream_credit: 16,
            max_aggregate_bytes: aggregate,
            max_streams: 4,
            max_frame_bytes: 8,
        })
        .unwrap()
    }

    #[test]
    fn stream_names_limits_and_duplicate_opens_are_closed() {
        assert_eq!(StreamName::parse(""), Err(StreamError::InvalidName));
        assert!(
            StreamBridge::new(StreamLimits {
                max_stream_credit: 1,
                max_aggregate_bytes: 1,
                max_streams: 1,
                max_frame_bytes: 2,
            })
            .is_err()
        );

        let bridge = bridge(16);
        let name = StreamName::parse("watch:one").unwrap();
        let (_outgoing, _incoming) = bridge
            .open(name.clone(), SessionId(1), SessionId(2), 8)
            .unwrap();
        assert!(matches!(
            bridge.open(name, SessionId(1), SessionId(2), 8),
            Err(StreamError::DuplicateStream)
        ));
    }

    #[tokio::test]
    async fn per_stream_and_aggregate_credit_apply_before_enqueue() {
        let bridge = bridge(6);
        let (outgoing, incoming) = bridge
            .open(
                StreamName::parse("watch:credit").unwrap(),
                SessionId(1),
                SessionId(2),
                5,
            )
            .unwrap();
        assert!(matches!(
            bridge.open(
                StreamName::parse("watch:aggregate-credit").unwrap(),
                SessionId(1),
                SessionId(2),
                2,
            ),
            Err(StreamError::AggregateBackpressure)
        ));
        outgoing.send(vec![1; 5]).unwrap();
        assert_eq!(outgoing.send(vec![2]), Err(StreamError::CreditExceeded));
        incoming.grant(outgoing.name(), 1).await.unwrap();
        assert_eq!(
            outgoing.send(vec![2; 2]),
            Err(StreamError::AggregateBackpressure)
        );
        let frame = incoming.receive_next().await.unwrap();
        assert_eq!(frame.payload(), &[1; 5]);
        outgoing.send(vec![2]).unwrap();
    }

    #[tokio::test]
    async fn ready_streams_are_served_round_robin() {
        let bridge = bridge(32);
        let (first_out, first_in) = bridge
            .open(
                StreamName::parse("watch:first").unwrap(),
                SessionId(1),
                SessionId(2),
                8,
            )
            .unwrap();
        let (second_out, _second_in) = bridge
            .open(
                StreamName::parse("watch:second").unwrap(),
                SessionId(1),
                SessionId(2),
                8,
            )
            .unwrap();
        first_out.send(vec![1]).unwrap();
        first_out.send(vec![2]).unwrap();
        second_out.send(vec![3]).unwrap();

        let observed = [
            first_in.receive_next().await.unwrap(),
            first_in.receive_next().await.unwrap(),
            first_in.receive_next().await.unwrap(),
        ];
        assert_eq!(observed[0].stream().as_str(), "watch:first");
        assert_eq!(observed[1].stream().as_str(), "watch:second");
        assert_eq!(observed[2].stream().as_str(), "watch:first");
    }

    #[tokio::test]
    async fn session_cancellation_closes_both_stream_directions() {
        let bridge = bridge(32);
        let (outgoing, incoming) = bridge
            .open(
                StreamName::parse("watch:cancel").unwrap(),
                SessionId(1),
                SessionId(2),
                8,
            )
            .unwrap();
        bridge.cancel_session(SessionId(2));
        assert_eq!(outgoing.send(vec![1]), Err(StreamError::StreamClosed));
        assert_eq!(
            incoming.receive_next().await,
            Err(StreamError::NoFrameAvailable)
        );
    }

    #[tokio::test]
    async fn concurrent_stream_opens_saturate_aggregate_credit() {
        const ATTEMPTS: usize = 16;
        let bridge = StreamBridge::new(StreamLimits {
            max_stream_credit: 8,
            max_aggregate_bytes: 32,
            max_streams: ATTEMPTS,
            max_frame_bytes: 8,
        })
        .unwrap();
        let release = Arc::new(Barrier::new(ATTEMPTS + 1));
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut tasks = Vec::new();

        for index in 0..ATTEMPTS {
            let bridge = Arc::clone(&bridge);
            let release = Arc::clone(&release);
            let tx = tx.clone();
            tasks.push(tokio::spawn(async move {
                let opened = bridge.open(
                    StreamName::parse(format!("stream:{index}")).unwrap(),
                    SessionId(1),
                    SessionId(2),
                    8,
                );
                tx.send(opened.as_ref().map(|_| ()).map_err(|error| *error))
                    .unwrap();
                release.wait().await;
                drop(opened);
            }));
        }
        drop(tx);

        let mut opened = 0;
        let mut backpressured = 0;
        for _ in 0..ATTEMPTS {
            match rx.recv().await.unwrap() {
                Ok(()) => opened += 1,
                Err(StreamError::AggregateBackpressure) => backpressured += 1,
                Err(error) => panic!("unexpected concurrent open error: {error}"),
            }
        }
        assert_eq!(opened, 4);
        assert_eq!(backpressured, ATTEMPTS - opened);
        release.wait().await;
        for task in tasks {
            task.await.unwrap();
        }
    }

    #[tokio::test]
    async fn concurrent_senders_exhaust_exact_byte_credit() {
        const ATTEMPTS: usize = 32;
        let bridge = StreamBridge::new(StreamLimits {
            max_stream_credit: 16,
            max_aggregate_bytes: 32,
            max_streams: 1,
            max_frame_bytes: 1,
        })
        .unwrap();
        let (outgoing, _incoming) = bridge
            .open(
                StreamName::parse("stream:contention").unwrap(),
                SessionId(1),
                SessionId(2),
                16,
            )
            .unwrap();
        let outgoing = Arc::new(outgoing);
        let start = Arc::new(Barrier::new(ATTEMPTS));
        let mut tasks = Vec::new();
        for _ in 0..ATTEMPTS {
            let outgoing = Arc::clone(&outgoing);
            let start = Arc::clone(&start);
            tasks.push(tokio::spawn(async move {
                start.wait().await;
                outgoing.send(vec![1])
            }));
        }

        let mut accepted = 0;
        let mut rejected = 0;
        for task in tasks {
            match task.await.unwrap() {
                Ok(()) => accepted += 1,
                Err(StreamError::CreditExceeded) => rejected += 1,
                Err(error) => panic!("unexpected concurrent send error: {error}"),
            }
        }
        assert_eq!(accepted, 16);
        assert_eq!(rejected, ATTEMPTS - accepted);
    }
}
