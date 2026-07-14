use std::{collections::BTreeMap, fmt};

use d2b_contracts::v2_component_session::{ChannelId, LimitProfile, SessionErrorCode};

use crate::{Result, SessionError};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StreamId(ChannelId);

impl StreamId {
    pub fn new(value: u16) -> Result<Self> {
        Ok(Self(ChannelId::named(value)?))
    }

    pub fn channel(self) -> ChannelId {
        self.0
    }
}

impl fmt::Debug for StreamId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("StreamId(<redacted>)")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamPhase {
    Open,
    HalfClosedLocal,
    HalfClosedRemote,
    Closed,
    Reset,
}

pub enum StreamEvent {
    Data { stream: StreamId, bytes: Vec<u8> },
    RemoteClosed { stream: StreamId },
    Reset { stream: StreamId },
}

impl fmt::Debug for StreamEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Data { bytes, .. } => formatter
                .debug_struct("StreamEvent::Data")
                .field("stream", &"<redacted>")
                .field("bytes", &"<redacted>")
                .field("len", &bytes.len())
                .finish(),
            Self::RemoteClosed { .. } => {
                formatter.write_str("StreamEvent::RemoteClosed(<redacted>)")
            }
            Self::Reset { .. } => formatter.write_str("StreamEvent::Reset(<redacted>)"),
        }
    }
}

struct StreamState {
    phase: StreamPhase,
    send_credit: u32,
    receive_credit: u32,
}

pub struct NamedStreamMux {
    limits: LimitProfile,
    streams: BTreeMap<StreamId, StreamState>,
}

impl NamedStreamMux {
    pub fn new(limits: LimitProfile) -> Result<Self> {
        limits.validate()?;
        Ok(Self {
            limits,
            streams: BTreeMap::new(),
        })
    }

    pub fn open(&mut self, stream: StreamId, send_credit: u32, receive_credit: u32) -> Result<()> {
        if self.streams.len() >= self.limits.active_named_streams as usize
            || send_credit > self.limits.named_stream_queue_bytes
            || receive_credit > self.limits.named_stream_queue_bytes
            || self.streams.contains_key(&stream)
        {
            return Err(SessionError::new(SessionErrorCode::QueueBackpressure));
        }
        self.streams.insert(
            stream,
            StreamState {
                phase: StreamPhase::Open,
                send_credit,
                receive_credit,
            },
        );
        Ok(())
    }

    pub fn phase(&self, stream: StreamId) -> Option<StreamPhase> {
        self.streams.get(&stream).map(|state| state.phase)
    }

    pub fn send_credit(&self, stream: StreamId) -> Option<u32> {
        self.streams.get(&stream).map(|state| state.send_credit)
    }

    pub fn reserve_send(&mut self, stream: StreamId, bytes: usize) -> Result<()> {
        let bytes = checked_message_len(bytes, self.limits.logical_named_stream_bytes)?;
        let state = self.stream_mut(stream)?;
        if !matches!(
            state.phase,
            StreamPhase::Open | StreamPhase::HalfClosedRemote
        ) || state.send_credit < bytes
        {
            return Err(SessionError::new(SessionErrorCode::QueueBackpressure));
        }
        state.send_credit -= bytes;
        Ok(())
    }

    pub fn grant_send_credit(&mut self, stream: StreamId, bytes: u32) -> Result<()> {
        let limit = self.limits.named_stream_queue_bytes;
        let state = self.stream_mut(stream)?;
        let next = state
            .send_credit
            .checked_add(bytes)
            .ok_or_else(|| SessionError::new(SessionErrorCode::ArithmeticOverflow))?;
        if next > limit {
            return Err(SessionError::new(SessionErrorCode::QueueBackpressure));
        }
        state.send_credit = next;
        Ok(())
    }

    pub fn receive_data(&mut self, stream: StreamId, bytes: Vec<u8>) -> Result<StreamEvent> {
        let len = checked_message_len(bytes.len(), self.limits.logical_named_stream_bytes)?;
        let state = self.stream_mut(stream)?;
        if !matches!(
            state.phase,
            StreamPhase::Open | StreamPhase::HalfClosedLocal
        ) || state.receive_credit < len
        {
            return Err(SessionError::new(SessionErrorCode::QueueBackpressure));
        }
        state.receive_credit -= len;
        Ok(StreamEvent::Data { stream, bytes })
    }

    pub fn release_receive_credit(&mut self, stream: StreamId, bytes: u32) -> Result<u32> {
        let limit = self.limits.named_stream_queue_bytes;
        let state = self.stream_mut(stream)?;
        let next = state
            .receive_credit
            .checked_add(bytes)
            .ok_or_else(|| SessionError::new(SessionErrorCode::ArithmeticOverflow))?;
        if next > limit {
            return Err(SessionError::new(SessionErrorCode::QueueBackpressure));
        }
        state.receive_credit = next;
        Ok(bytes)
    }

    pub fn close_local(&mut self, stream: StreamId) -> Result<StreamPhase> {
        let state = self.stream_mut(stream)?;
        state.phase = match state.phase {
            StreamPhase::Open => StreamPhase::HalfClosedLocal,
            StreamPhase::HalfClosedRemote => StreamPhase::Closed,
            StreamPhase::HalfClosedLocal | StreamPhase::Closed | StreamPhase::Reset => {
                return Err(SessionError::new(SessionErrorCode::UnknownControl));
            }
        };
        Ok(state.phase)
    }

    pub fn receive_close(&mut self, stream: StreamId) -> Result<StreamEvent> {
        let state = self.stream_mut(stream)?;
        state.phase = match state.phase {
            StreamPhase::Open => StreamPhase::HalfClosedRemote,
            StreamPhase::HalfClosedLocal => StreamPhase::Closed,
            StreamPhase::HalfClosedRemote | StreamPhase::Closed | StreamPhase::Reset => {
                return Err(SessionError::new(SessionErrorCode::UnknownControl));
            }
        };
        Ok(StreamEvent::RemoteClosed { stream })
    }

    pub fn reset(&mut self, stream: StreamId) -> Result<StreamEvent> {
        self.stream_mut(stream)?.phase = StreamPhase::Reset;
        Ok(StreamEvent::Reset { stream })
    }

    pub fn remove_terminal(&mut self, stream: StreamId) -> bool {
        if self
            .streams
            .get(&stream)
            .is_some_and(|state| matches!(state.phase, StreamPhase::Closed | StreamPhase::Reset))
        {
            self.streams.remove(&stream);
            true
        } else {
            false
        }
    }

    pub fn active(&self) -> usize {
        self.streams.len()
    }

    fn stream_mut(&mut self, stream: StreamId) -> Result<&mut StreamState> {
        self.streams
            .get_mut(&stream)
            .ok_or_else(|| SessionError::new(SessionErrorCode::InvalidChannel))
    }
}

fn checked_message_len(bytes: usize, logical_limit: u32) -> Result<u32> {
    let bytes = u32::try_from(bytes)
        .map_err(|_| SessionError::new(SessionErrorCode::ArithmeticOverflow))?;
    if bytes == 0 || bytes > logical_limit {
        return Err(SessionError::new(SessionErrorCode::ReassemblyLimitExceeded));
    }
    Ok(bytes)
}

impl fmt::Debug for NamedStreamMux {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NamedStreamMux")
            .field("active", &self.streams.len())
            .field("stream_ids", &"<redacted>")
            .finish()
    }
}
