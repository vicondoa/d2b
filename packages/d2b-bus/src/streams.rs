//! Bounded named-stream bridge with byte credits and round-robin delivery.

use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Arc, Mutex, MutexGuard},
};
use tokio::sync::Notify;

#[cfg(test)]
use crate::router::NoopBusObserver;
use crate::{
    operations::SessionId,
    registry::PrincipalId,
    router::{BusEvent, BusFailureReason, BusObserver},
};

// The bus owns addressing and admission, while the portable session crate
// owns the protocol stream FSM.  Re-exporting that canonical FSM here keeps
// callers from constructing a second half-close/reset implementation.
pub use d2b_session::{NamedStreamMux, StreamEvent, StreamId, StreamPhase};

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
    pub max_streams_per_principal: usize,
    pub max_credit_per_principal: usize,
    pub max_queued_bytes_per_principal: usize,
}

impl Default for StreamLimits {
    fn default() -> Self {
        Self {
            max_stream_credit: DEFAULT_MAX_STREAM_CREDIT,
            max_aggregate_bytes: DEFAULT_MAX_AGGREGATE_BYTES,
            max_streams: DEFAULT_MAX_STREAMS,
            max_frame_bytes: DEFAULT_MAX_FRAME_BYTES,
            max_streams_per_principal: 64,
            max_credit_per_principal: 1024 * 1024,
            max_queued_bytes_per_principal: 1024 * 1024,
        }
    }
}

impl StreamLimits {
    pub(crate) fn validate(self) -> Result<Self, StreamError> {
        if self.max_stream_credit == 0
            || self.max_aggregate_bytes == 0
            || self.max_streams == 0
            || self.max_frame_bytes == 0
            || self.max_streams_per_principal == 0
            || self.max_credit_per_principal == 0
            || self.max_queued_bytes_per_principal == 0
            || self.max_frame_bytes > self.max_stream_credit
            || self.max_streams_per_principal > self.max_streams
            || self.max_credit_per_principal > self.max_aggregate_bytes
            || self.max_queued_bytes_per_principal > self.max_aggregate_bytes
        {
            return Err(StreamError::InvalidLimits);
        }
        Ok(self)
    }
}

struct StreamState {
    source_principal: PrincipalId,
    destination_principal: PrincipalId,
    source: SessionId,
    destination: SessionId,
    credit: usize,
    frames: VecDeque<Vec<u8>>,
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct StreamKey {
    principal: PrincipalId,
    name: StreamName,
}

#[derive(Default)]
struct PrincipalUsage {
    streams: usize,
    credit: usize,
    queued_bytes: usize,
}

struct BridgeState {
    aggregate_bytes: usize,
    aggregate_credit: usize,
    streams: BTreeMap<StreamKey, StreamState>,
    usage: BTreeMap<PrincipalId, PrincipalUsage>,
    ready_principals: VecDeque<PrincipalId>,
    ready_streams: BTreeMap<PrincipalId, VecDeque<StreamKey>>,
}

/// Shared bridge state. Handles expose only their direction-specific operations.
pub(crate) struct StreamBridge {
    limits: StreamLimits,
    state: Mutex<BridgeState>,
    notify: Notify,
    observer: Arc<dyn BusObserver>,
}

impl StreamBridge {
    #[cfg(test)]
    pub(crate) fn new(limits: StreamLimits) -> Result<Arc<Self>, StreamError> {
        Self::with_observer(limits, Arc::new(NoopBusObserver))
    }

    pub(crate) fn with_observer(
        limits: StreamLimits,
        observer: Arc<dyn BusObserver>,
    ) -> Result<Arc<Self>, StreamError> {
        Ok(Arc::new(Self {
            limits: limits.validate()?,
            state: Mutex::new(BridgeState {
                aggregate_bytes: 0,
                aggregate_credit: 0,
                streams: BTreeMap::new(),
                usage: BTreeMap::new(),
                ready_principals: VecDeque::new(),
                ready_streams: BTreeMap::new(),
            }),
            notify: Notify::new(),
            observer,
        }))
    }

    pub(crate) fn open(
        self: &Arc<Self>,
        name: StreamName,
        source: SessionId,
        source_principal: PrincipalId,
        destination: SessionId,
        destination_principal: PrincipalId,
        initial_credit: usize,
    ) -> Result<(OutgoingStream, IncomingStream), StreamError> {
        if initial_credit == 0 || initial_credit > self.limits.max_stream_credit {
            return Err(StreamError::CreditExceeded);
        }
        let mut state = self.lock();
        let key = StreamKey {
            principal: source_principal.clone(),
            name: name.clone(),
        };
        if state.streams.len() >= self.limits.max_streams {
            return Err(StreamError::StreamCapacityExceeded);
        }
        if state.streams.contains_key(&key) {
            return Err(StreamError::DuplicateStream);
        }
        if state.aggregate_credit.saturating_add(initial_credit) > self.limits.max_aggregate_bytes {
            return Err(StreamError::AggregateBackpressure);
        }
        let usage = state.usage.get(&source_principal);
        if usage.is_some_and(|usage| usage.streams >= self.limits.max_streams_per_principal) {
            return Err(StreamError::PrincipalCapacityExceeded);
        }
        if usage
            .map_or(0, |usage| usage.credit)
            .saturating_add(initial_credit)
            > self.limits.max_credit_per_principal
        {
            return Err(StreamError::PrincipalBackpressure);
        }
        state.aggregate_credit += initial_credit;
        state.streams.insert(
            key.clone(),
            StreamState {
                source_principal: source_principal.clone(),
                destination_principal: destination_principal.clone(),
                source,
                destination,
                credit: initial_credit,
                frames: VecDeque::new(),
            },
        );
        let usage = state.usage.entry(source_principal).or_default();
        usage.streams += 1;
        usage.credit += initial_credit;
        drop(state);
        Ok((
            OutgoingStream {
                bridge: Arc::clone(self),
                key: key.clone(),
                source,
                closed: false,
            },
            IncomingStream {
                bridge: Arc::clone(self),
                owner: key,
                destination,
                destination_principal,
                closed: false,
            },
        ))
    }

    fn send(
        &self,
        name: &StreamName,
        principal: &PrincipalId,
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
        let key = StreamKey {
            principal: principal.clone(),
            name: name.clone(),
        };
        let principal_queued = state
            .usage
            .get(principal)
            .map_or(0, |usage| usage.queued_bytes);
        if principal_queued.saturating_add(frame_len) > self.limits.max_queued_bytes_per_principal {
            return Err(StreamError::PrincipalBackpressure);
        }
        let stream = state
            .streams
            .get_mut(&key)
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
        let usage = state
            .usage
            .get_mut(principal)
            .ok_or(StreamError::StreamClosed)?;
        usage.credit -= frame_len;
        usage.queued_bytes += frame_len;
        if was_empty {
            let principal_ready = state
                .ready_streams
                .get(principal)
                .is_some_and(|ready| !ready.is_empty());
            state
                .ready_streams
                .entry(principal.clone())
                .or_default()
                .push_back(key);
            if !principal_ready {
                state.ready_principals.push_back(principal.clone());
            }
        }
        drop(state);
        self.notify.notify_waiters();
        Ok(())
    }

    fn receive(
        &self,
        destination: SessionId,
        destination_principal: &PrincipalId,
        owner: &StreamKey,
    ) -> Result<ReceivedFrame, StreamError> {
        let mut state = self.lock();
        if !state.streams.contains_key(owner) {
            return Err(StreamError::StreamClosed);
        }
        let principal_candidates = state.ready_principals.len();
        for _ in 0..principal_candidates {
            let principal = state
                .ready_principals
                .pop_front()
                .expect("candidate count came from principal queue");
            let mut ready = state.ready_streams.remove(&principal).unwrap_or_default();
            let stream_candidates = ready.len();
            let mut selected = None;
            for _ in 0..stream_candidates {
                let key = ready.pop_front().expect("stream candidate exists");
                let matches = state.streams.get(&key).is_some_and(|stream| {
                    stream.destination == destination
                        && &stream.destination_principal == destination_principal
                });
                if matches {
                    selected = Some(key);
                    break;
                }
                ready.push_back(key);
            }
            let Some(key) = selected else {
                if !ready.is_empty() {
                    state.ready_streams.insert(principal.clone(), ready);
                    state.ready_principals.push_back(principal);
                }
                continue;
            };
            let stream = state
                .streams
                .get_mut(&key)
                .expect("ready stream remains registered");
            let payload = stream
                .frames
                .pop_front()
                .expect("ready streams contain a frame");
            let has_more = !stream.frames.is_empty();
            state.aggregate_bytes -= payload.len();
            let usage = state
                .usage
                .get_mut(&principal)
                .expect("stream principal usage exists");
            usage.queued_bytes -= payload.len();
            if has_more {
                ready.push_back(key.clone());
            }
            if !ready.is_empty() {
                state.ready_streams.insert(principal.clone(), ready);
                state.ready_principals.push_back(principal);
            }
            return Ok(ReceivedFrame { key, payload });
        }
        Err(StreamError::NoFrameAvailable)
    }

    fn grant(
        &self,
        name: &StreamName,
        principal: &PrincipalId,
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
        let key = StreamKey {
            principal: principal.clone(),
            name: name.clone(),
        };
        let principal_credit = state.usage.get(principal).map_or(0, |usage| usage.credit);
        if principal_credit.saturating_add(bytes) > self.limits.max_credit_per_principal {
            return Err(StreamError::PrincipalBackpressure);
        }
        let stream = state
            .streams
            .get_mut(&key)
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
        state
            .usage
            .get_mut(principal)
            .ok_or(StreamError::StreamClosed)?
            .credit += bytes;
        drop(state);
        self.notify.notify_waiters();
        Ok(())
    }

    fn close(&self, key: &StreamKey) {
        let mut state = self.lock();
        let mut shed = false;
        if let Some(stream) = state.streams.remove(key) {
            let queued = stream.frames.iter().map(Vec::len).sum::<usize>();
            shed = queued != 0;
            state.aggregate_bytes = state.aggregate_bytes.saturating_sub(queued);
            state.aggregate_credit = state.aggregate_credit.saturating_sub(stream.credit);
            if let Some(usage) = state.usage.get_mut(&stream.source_principal) {
                usage.streams -= 1;
                usage.credit = usage.credit.saturating_sub(stream.credit);
                usage.queued_bytes = usage.queued_bytes.saturating_sub(queued);
                if usage.streams == 0 {
                    state.usage.remove(&stream.source_principal);
                }
            }
            if let Some(ready) = state.ready_streams.get_mut(&stream.source_principal) {
                ready.retain(|ready| ready != key);
                if ready.is_empty() {
                    state.ready_streams.remove(&stream.source_principal);
                    state
                        .ready_principals
                        .retain(|principal| principal != &stream.source_principal);
                }
            }
        }
        drop(state);
        if shed {
            self.observer
                .record(BusEvent::Cleanup, BusFailureReason::StreamShed);
        }
        self.notify.notify_waiters();
    }

    pub(crate) fn cancel_session(&self, session: SessionId) {
        let keys = {
            let state = self.lock();
            state
                .streams
                .iter()
                .filter(|(_, stream)| stream.source == session || stream.destination == session)
                .map(|(key, _)| key.clone())
                .collect::<Vec<_>>()
        };
        for key in keys {
            self.close(&key);
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
    key: StreamKey,
    payload: Vec<u8>,
}

impl ReceivedFrame {
    /// Borrow the stream name.
    pub const fn stream(&self) -> &StreamName {
        &self.key.name
    }

    /// Borrow the frame bytes.
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }
}

impl core::fmt::Debug for ReceivedFrame {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ReceivedFrame")
            .field("stream", &self.key.name)
            .field("payload_bytes", &self.payload.len())
            .finish()
    }
}

/// Source-side stream handle.
pub(crate) struct OutgoingStream {
    bridge: Arc<StreamBridge>,
    key: StreamKey,
    source: SessionId,
    closed: bool,
}

impl OutgoingStream {
    pub(crate) fn name(&self) -> &StreamName {
        &self.key.name
    }

    pub(crate) fn send(&self, payload: Vec<u8>) -> Result<(), StreamError> {
        self.bridge
            .send(&self.key.name, &self.key.principal, self.source, payload)
    }

    pub(crate) async fn send_wait(&self, payload: Vec<u8>) -> Result<(), StreamError> {
        loop {
            let notified = self.bridge.notify.notified();
            let mut notified = std::pin::pin!(notified);
            notified.as_mut().enable();
            match self.send(payload.clone()) {
                Ok(()) => return Ok(()),
                Err(
                    StreamError::CreditExceeded
                    | StreamError::AggregateBackpressure
                    | StreamError::PrincipalBackpressure,
                ) => {
                    notified.await;
                }
                Err(error) => return Err(error),
            }
        }
    }

    pub(crate) fn close(&mut self) {
        if !self.closed {
            self.bridge.close(&self.key);
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
    owner: StreamKey,
    destination: SessionId,
    destination_principal: PrincipalId,
    closed: bool,
}

impl IncomingStream {
    /// Borrow the stream that established this destination reader.
    pub const fn stream_name(&self) -> &StreamName {
        &self.owner.name
    }

    /// Receive the next fair frame for this destination across all named streams.
    pub async fn receive_next(&self) -> Result<ReceivedFrame, StreamError> {
        loop {
            let notified = self.bridge.notify.notified();
            match self
                .bridge
                .receive(self.destination, &self.destination_principal, &self.owner)
            {
                Err(StreamError::NoFrameAvailable) => notified.await,
                result => return result,
            }
        }
    }

    /// Grant more byte credit to one named stream at this destination.
    pub async fn grant(&self, stream: &StreamName, bytes: usize) -> Result<(), StreamError> {
        self.bridge
            .grant(stream, &self.owner.principal, self.destination, bytes)
    }

    /// Grant credit to the exact authenticated principal and stream in a frame.
    pub async fn grant_frame(
        &self,
        frame: &ReceivedFrame,
        bytes: usize,
    ) -> Result<(), StreamError> {
        self.bridge.grant(
            &frame.key.name,
            &frame.key.principal,
            self.destination,
            bytes,
        )
    }

    /// Close the stream that established this reader.
    pub fn close(&mut self) {
        if !self.closed {
            self.bridge.close(&self.owner);
            self.closed = true;
        }
    }
}

impl core::fmt::Debug for IncomingStream {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("IncomingStream")
            .field("stream", &self.owner.name)
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
    PrincipalCapacityExceeded,
    FrameBounds,
    CreditExceeded,
    AggregateBackpressure,
    PrincipalBackpressure,
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
            Self::PrincipalCapacityExceeded => "principal stream capacity is exhausted",
            Self::FrameBounds => "stream frame is outside the byte bounds",
            Self::CreditExceeded => "stream byte credit is insufficient",
            Self::AggregateBackpressure => "aggregate stream backpressure limit was reached",
            Self::PrincipalBackpressure => "principal stream backpressure limit was reached",
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
    use d2b_contracts::v3::ResourceUid;
    use tokio::sync::{Barrier, mpsc};

    #[derive(Default)]
    struct RecordingObserver(Mutex<Vec<(BusEvent, BusFailureReason)>>);

    impl BusObserver for RecordingObserver {
        fn record(&self, event: BusEvent, reason: BusFailureReason) {
            self.0.lock().unwrap().push((event, reason));
        }
    }

    fn principal(value: u64) -> PrincipalId {
        PrincipalId::test(
            ResourceUid::parse(format!("00000000-0000-4000-8000-{value:012}")).unwrap(),
        )
    }

    impl StreamBridge {
        fn open_test(
            self: &Arc<Self>,
            name: StreamName,
            source: SessionId,
            destination: SessionId,
            initial_credit: usize,
        ) -> Result<(OutgoingStream, IncomingStream), StreamError> {
            self.open(
                name,
                source,
                principal(source.0),
                destination,
                principal(destination.0),
                initial_credit,
            )
        }
    }

    fn bridge(aggregate: usize) -> Arc<StreamBridge> {
        bridge_with_observer(aggregate, Arc::new(NoopBusObserver))
    }

    fn bridge_with_observer(aggregate: usize, observer: Arc<dyn BusObserver>) -> Arc<StreamBridge> {
        StreamBridge::with_observer(
            StreamLimits {
                max_stream_credit: 16,
                max_aggregate_bytes: aggregate,
                max_streams: 4,
                max_frame_bytes: 8,
                max_streams_per_principal: 4,
                max_credit_per_principal: aggregate,
                max_queued_bytes_per_principal: aggregate,
            },
            observer,
        )
        .unwrap()
    }

    #[test]
    fn queued_stream_shedding_is_observed_for_every_close_path() {
        let observer = Arc::new(RecordingObserver::default());
        let bridge = bridge_with_observer(32, observer.clone());

        let (mut explicit, explicit_incoming) = bridge
            .open_test(
                StreamName::parse("watch:explicit").unwrap(),
                SessionId(1),
                SessionId(2),
                8,
            )
            .unwrap();
        explicit.send(vec![1, 2]).unwrap();
        explicit.close();
        drop(explicit_incoming);

        let (dropped_outgoing, dropped_incoming) = bridge
            .open_test(
                StreamName::parse("watch:drop").unwrap(),
                SessionId(3),
                SessionId(4),
                8,
            )
            .unwrap();
        dropped_outgoing.send(vec![3, 4]).unwrap();
        drop(dropped_incoming);
        drop(dropped_outgoing);

        let (session_outgoing, session_incoming) = bridge
            .open_test(
                StreamName::parse("watch:session").unwrap(),
                SessionId(5),
                SessionId(6),
                8,
            )
            .unwrap();
        session_outgoing.send(vec![5, 6]).unwrap();
        bridge.cancel_session(SessionId(6));
        drop(session_incoming);
        drop(session_outgoing);

        assert_eq!(
            observer.0.lock().unwrap().as_slice(),
            &[
                (BusEvent::Cleanup, BusFailureReason::StreamShed),
                (BusEvent::Cleanup, BusFailureReason::StreamShed),
                (BusEvent::Cleanup, BusFailureReason::StreamShed),
            ]
        );
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
                max_streams_per_principal: 1,
                max_credit_per_principal: 1,
                max_queued_bytes_per_principal: 1,
            })
            .is_err()
        );

        let bridge = bridge(16);
        let name = StreamName::parse("watch:one").unwrap();
        let (_outgoing, _incoming) = bridge
            .open_test(name.clone(), SessionId(1), SessionId(2), 8)
            .unwrap();
        assert!(matches!(
            bridge.open_test(name, SessionId(1), SessionId(2), 8),
            Err(StreamError::DuplicateStream)
        ));
    }

    #[tokio::test]
    async fn per_stream_and_aggregate_credit_apply_before_enqueue() {
        let bridge = bridge(6);
        let (outgoing, incoming) = bridge
            .open_test(
                StreamName::parse("watch:credit").unwrap(),
                SessionId(1),
                SessionId(2),
                5,
            )
            .unwrap();
        assert!(matches!(
            bridge.open_test(
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
    async fn bounded_watch_delivery_waits_for_transport_credit() {
        let bridge = bridge(16);
        let (outgoing, incoming) = bridge
            .open_test(
                StreamName::parse("watch:wait-credit").unwrap(),
                SessionId(1),
                SessionId(2),
                2,
            )
            .unwrap();
        let outgoing = Arc::new(outgoing);
        let sender = {
            let outgoing = Arc::clone(&outgoing);
            tokio::spawn(async move { outgoing.send_wait(vec![1, 2, 3, 4]).await })
        };
        tokio::task::yield_now().await;
        incoming.grant(outgoing.name(), 2).await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(1), sender)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let frame = incoming.receive_next().await.unwrap();
        assert_eq!(frame.payload(), &[1, 2, 3, 4]);
    }

    #[tokio::test]
    async fn ready_streams_are_served_round_robin() {
        let bridge = bridge(32);
        let (first_out, first_in) = bridge
            .open_test(
                StreamName::parse("watch:first").unwrap(),
                SessionId(1),
                SessionId(2),
                8,
            )
            .unwrap();
        let (second_out, _second_in) = bridge
            .open_test(
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
    async fn principals_can_reuse_names_and_are_scheduled_before_their_streams() {
        let bridge = bridge(32);
        let destination = principal(9);
        let (first_out, first_in) = bridge
            .open(
                StreamName::parse("watch:shared").unwrap(),
                SessionId(1),
                principal(1),
                SessionId(9),
                destination.clone(),
                8,
            )
            .unwrap();
        let (second_out, _second_in) = bridge
            .open(
                StreamName::parse("watch:shared").unwrap(),
                SessionId(2),
                principal(2),
                SessionId(9),
                destination,
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
        assert_eq!(observed[0].payload(), &[1]);
        assert_eq!(observed[1].payload(), &[3]);
        assert_eq!(observed[2].payload(), &[2]);
    }

    #[test]
    fn per_principal_stream_quota_does_not_block_another_principal() {
        let bridge = StreamBridge::new(StreamLimits {
            max_stream_credit: 8,
            max_aggregate_bytes: 24,
            max_streams: 3,
            max_frame_bytes: 8,
            max_streams_per_principal: 1,
            max_credit_per_principal: 8,
            max_queued_bytes_per_principal: 8,
        })
        .unwrap();
        let destination = principal(9);
        let _first = bridge
            .open(
                StreamName::parse("one").unwrap(),
                SessionId(1),
                principal(1),
                SessionId(9),
                destination.clone(),
                8,
            )
            .unwrap();
        assert!(matches!(
            bridge.open(
                StreamName::parse("two").unwrap(),
                SessionId(1),
                principal(1),
                SessionId(9),
                destination.clone(),
                8,
            ),
            Err(StreamError::PrincipalCapacityExceeded)
        ));
        assert!(
            bridge
                .open(
                    StreamName::parse("one").unwrap(),
                    SessionId(2),
                    principal(2),
                    SessionId(9),
                    destination,
                    8,
                )
                .is_ok()
        );
    }

    #[tokio::test]
    async fn receive_waits_for_arrival_and_close_wakes_waiters() {
        let bridge = bridge(16);
        let (outgoing, incoming) = bridge
            .open_test(
                StreamName::parse("watch:wait").unwrap(),
                SessionId(1),
                SessionId(2),
                8,
            )
            .unwrap();
        let receive = incoming.receive_next();
        let send = async {
            tokio::task::yield_now().await;
            outgoing.send(vec![1]).unwrap();
        };
        let (frame, ()) = tokio::join!(receive, send);
        assert_eq!(frame.unwrap().payload(), &[1]);

        let receive = incoming.receive_next();
        let close = async {
            tokio::task::yield_now().await;
            drop(outgoing);
        };
        let (closed, ()) = tokio::join!(receive, close);
        assert_eq!(closed, Err(StreamError::StreamClosed));
    }

    #[tokio::test]
    async fn session_cancellation_closes_both_stream_directions() {
        let bridge = bridge(32);
        let (outgoing, incoming) = bridge
            .open_test(
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
            Err(StreamError::StreamClosed)
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
            max_streams_per_principal: ATTEMPTS,
            max_credit_per_principal: 32,
            max_queued_bytes_per_principal: 32,
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
                let opened = bridge.open_test(
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
            max_streams_per_principal: 1,
            max_credit_per_principal: 32,
            max_queued_bytes_per_principal: 32,
        })
        .unwrap();
        let (outgoing, _incoming) = bridge
            .open_test(
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
