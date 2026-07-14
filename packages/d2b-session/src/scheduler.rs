use std::{
    collections::{BTreeMap, VecDeque},
    fmt,
};

use d2b_contracts::v2_component_session::{LimitProfile, SessionErrorCode};

use crate::{Result, SessionError, StreamId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueClass {
    SessionControl,
    TtrpcControl,
    AttachmentControl,
    NamedStream,
}

pub struct OutboundFrame {
    class: QueueClass,
    stream: Option<StreamId>,
    bytes: Vec<u8>,
}

impl OutboundFrame {
    pub fn control(class: QueueClass, bytes: Vec<u8>) -> Result<Self> {
        if class == QueueClass::NamedStream || bytes.is_empty() {
            return Err(SessionError::new(SessionErrorCode::InvalidChannel));
        }
        Ok(Self {
            class,
            stream: None,
            bytes,
        })
    }

    pub fn named(stream: StreamId, bytes: Vec<u8>) -> Result<Self> {
        if bytes.is_empty() {
            return Err(SessionError::new(SessionErrorCode::InvalidChannel));
        }
        Ok(Self {
            class: QueueClass::NamedStream,
            stream: Some(stream),
            bytes,
        })
    }

    pub fn class(&self) -> QueueClass {
        self.class
    }

    pub fn stream(&self) -> Option<StreamId> {
        self.stream
    }

    pub fn channel(&self) -> d2b_contracts::v2_component_session::ChannelId {
        match self.class {
            QueueClass::SessionControl => {
                d2b_contracts::v2_component_session::ChannelId::SESSION_CONTROL
            }
            QueueClass::TtrpcControl => {
                d2b_contracts::v2_component_session::ChannelId::TTRPC_CONTROL
            }
            QueueClass::AttachmentControl => {
                d2b_contracts::v2_component_session::ChannelId::ATTACHMENT_CONTROL
            }
            QueueClass::NamedStream => self
                .stream
                .map(StreamId::channel)
                .unwrap_or(d2b_contracts::v2_component_session::ChannelId::SESSION_CONTROL),
        }
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }
}

impl fmt::Debug for OutboundFrame {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OutboundFrame")
            .field("class", &self.class)
            .field("stream", &self.stream.map(|_| "<redacted>"))
            .field("bytes", &"<redacted>")
            .field("len", &self.bytes.len())
            .finish()
    }
}

struct NamedQueue {
    frames: VecDeque<OutboundFrame>,
    bytes: usize,
    credit: u32,
}

pub struct FairScheduler {
    limits: LimitProfile,
    session: VecDeque<OutboundFrame>,
    session_bytes: usize,
    ttrpc: VecDeque<OutboundFrame>,
    ttrpc_bytes: usize,
    attachments: VecDeque<OutboundFrame>,
    attachment_bytes: usize,
    named: BTreeMap<StreamId, NamedQueue>,
    named_order: VecDeque<StreamId>,
    named_bytes: usize,
}

impl FairScheduler {
    pub fn new(limits: LimitProfile) -> Result<Self> {
        limits.validate()?;
        Ok(Self {
            limits,
            session: VecDeque::new(),
            session_bytes: 0,
            ttrpc: VecDeque::new(),
            ttrpc_bytes: 0,
            attachments: VecDeque::new(),
            attachment_bytes: 0,
            named: BTreeMap::new(),
            named_order: VecDeque::new(),
            named_bytes: 0,
        })
    }

    pub fn register_stream(&mut self, stream: StreamId, initial_credit: u32) -> Result<()> {
        if self.named.len() >= self.limits.active_named_streams as usize
            || initial_credit > self.limits.named_stream_queue_bytes
            || self.named.contains_key(&stream)
        {
            return Err(SessionError::new(SessionErrorCode::QueueBackpressure));
        }
        self.named.insert(
            stream,
            NamedQueue {
                frames: VecDeque::new(),
                bytes: 0,
                credit: initial_credit,
            },
        );
        self.named_order.push_back(stream);
        Ok(())
    }

    pub fn grant_stream_credit(&mut self, stream: StreamId, bytes: u32) -> Result<()> {
        let queue = self
            .named
            .get_mut(&stream)
            .ok_or_else(|| SessionError::new(SessionErrorCode::InvalidChannel))?;
        let next = queue
            .credit
            .checked_add(bytes)
            .ok_or_else(|| SessionError::new(SessionErrorCode::ArithmeticOverflow))?;
        if next > self.limits.named_stream_queue_bytes {
            return Err(SessionError::new(SessionErrorCode::QueueBackpressure));
        }
        queue.credit = next;
        Ok(())
    }

    pub fn enqueue(&mut self, frame: OutboundFrame) -> Result<()> {
        let len = frame.bytes.len();
        match frame.class {
            QueueClass::SessionControl => enqueue_bounded(
                &mut self.session,
                &mut self.session_bytes,
                frame,
                self.limits.session_control_queue_bytes as usize,
            ),
            QueueClass::TtrpcControl => enqueue_bounded(
                &mut self.ttrpc,
                &mut self.ttrpc_bytes,
                frame,
                self.limits.ttrpc_control_queue_bytes as usize,
            ),
            QueueClass::AttachmentControl => enqueue_bounded(
                &mut self.attachments,
                &mut self.attachment_bytes,
                frame,
                self.limits.session_control_queue_bytes as usize,
            ),
            QueueClass::NamedStream => {
                let stream = frame
                    .stream
                    .ok_or_else(|| SessionError::new(SessionErrorCode::InvalidChannel))?;
                let aggregate = self
                    .named_bytes
                    .checked_add(len)
                    .ok_or_else(|| SessionError::new(SessionErrorCode::ArithmeticOverflow))?;
                if aggregate > self.limits.aggregate_named_stream_queue_bytes as usize {
                    return Err(SessionError::new(SessionErrorCode::QueueBackpressure));
                }
                let queue = self
                    .named
                    .get_mut(&stream)
                    .ok_or_else(|| SessionError::new(SessionErrorCode::InvalidChannel))?;
                let stream_bytes = queue
                    .bytes
                    .checked_add(len)
                    .ok_or_else(|| SessionError::new(SessionErrorCode::ArithmeticOverflow))?;
                if stream_bytes > self.limits.named_stream_queue_bytes as usize {
                    return Err(SessionError::new(SessionErrorCode::QueueBackpressure));
                }
                queue.frames.push_back(frame);
                queue.bytes = stream_bytes;
                self.named_bytes = aggregate;
                Ok(())
            }
        }
    }

    pub fn dequeue(&mut self) -> Option<OutboundFrame> {
        if let Some(frame) = pop_bounded(&mut self.session, &mut self.session_bytes) {
            return Some(frame);
        }
        if let Some(frame) = pop_bounded(&mut self.ttrpc, &mut self.ttrpc_bytes) {
            return Some(frame);
        }
        if let Some(frame) = pop_bounded(&mut self.attachments, &mut self.attachment_bytes) {
            return Some(frame);
        }
        let attempts = self.named_order.len();
        for _ in 0..attempts {
            let stream = self.named_order.pop_front()?;
            self.named_order.push_back(stream);
            let queue = self.named.get_mut(&stream)?;
            let Some(front) = queue.frames.front() else {
                continue;
            };
            let len = u32::try_from(front.bytes.len()).ok()?;
            if len > queue.credit {
                continue;
            }
            queue.credit -= len;
            let frame = queue.frames.pop_front()?;
            queue.bytes -= frame.bytes.len();
            self.named_bytes -= frame.bytes.len();
            return Some(frame);
        }
        None
    }

    pub fn remove_stream(&mut self, stream: StreamId) -> bool {
        let Some(queue) = self.named.remove(&stream) else {
            return false;
        };
        self.named_bytes = self.named_bytes.saturating_sub(queue.bytes);
        self.named_order.retain(|candidate| *candidate != stream);
        true
    }

    pub fn queued_bytes(&self, class: QueueClass) -> usize {
        match class {
            QueueClass::SessionControl => self.session_bytes,
            QueueClass::TtrpcControl => self.ttrpc_bytes,
            QueueClass::AttachmentControl => self.attachment_bytes,
            QueueClass::NamedStream => self.named_bytes,
        }
    }
}

fn enqueue_bounded(
    queue: &mut VecDeque<OutboundFrame>,
    used: &mut usize,
    frame: OutboundFrame,
    limit: usize,
) -> Result<()> {
    let next = used
        .checked_add(frame.bytes.len())
        .ok_or_else(|| SessionError::new(SessionErrorCode::ArithmeticOverflow))?;
    if next > limit {
        return Err(SessionError::new(
            SessionErrorCode::ControlResourceExhausted,
        ));
    }
    queue.push_back(frame);
    *used = next;
    Ok(())
}

fn pop_bounded(queue: &mut VecDeque<OutboundFrame>, used: &mut usize) -> Option<OutboundFrame> {
    let frame = queue.pop_front()?;
    *used -= frame.bytes.len();
    Some(frame)
}

impl fmt::Debug for FairScheduler {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FairScheduler")
            .field("session_bytes", &self.session_bytes)
            .field("ttrpc_bytes", &self.ttrpc_bytes)
            .field("attachment_bytes", &self.attachment_bytes)
            .field("named_bytes", &self.named_bytes)
            .field("named_streams", &self.named.len())
            .field("stream_ids", &"<redacted>")
            .finish()
    }
}
