use std::time::Instant;

use async_trait::async_trait;
use d2b_contracts::v2_component_session::{CloseReason, Remediation, RequestId, SessionErrorCode};

use crate::{
    Cancellation, OwnedAttachment, OwnedTransport, Result, SessionEngine, SessionError,
    SessionEvent, StreamId,
};

/// Portable control surface for one established ComponentSession.
///
/// Ttrpc frames stay opaque here: generated ttrpc code owns request/reply
/// framing and correlation, while ComponentSession owns protection,
/// fragmentation, cancellation, attachments, and named-stream multiplexing.
#[async_trait]
pub trait ComponentSessionDriver: Send {
    fn generation(&self) -> u64;

    fn outstanding_attachment_credits(&self) -> u16;

    async fn invoke(&mut self, request_id: RequestId, frame: Vec<u8>) -> Result<Cancellation>;

    fn complete_invoke(&mut self, request_id: &RequestId) -> bool;

    async fn cancel(&mut self, generation: u64, request_id: &RequestId) -> Result<()>;

    async fn send_ttrpc(&mut self, frame: Vec<u8>) -> Result<()>;

    fn open_named_stream(
        &mut self,
        stream: StreamId,
        send_credit: u32,
        receive_credit: u32,
    ) -> Result<()>;

    async fn send_named_stream(&mut self, stream: StreamId, bytes: Vec<u8>) -> Result<()>;

    async fn grant_named_stream_credit(&mut self, stream: StreamId, bytes: u32) -> Result<()>;

    async fn close_named_stream(&mut self, stream: StreamId) -> Result<()>;

    async fn reset_named_stream(&mut self, stream: StreamId) -> Result<()>;

    async fn send_attachments(&mut self, attachments: Vec<OwnedAttachment>) -> Result<()>;

    async fn drive_keepalive(&mut self, now: Instant) -> Result<()>;

    async fn receive(&mut self) -> Result<SessionEvent>;

    async fn close(&mut self, reason: CloseReason, remediation: Remediation) -> Result<()>;
}

#[async_trait]
impl<T: OwnedTransport> ComponentSessionDriver for SessionEngine<T> {
    fn generation(&self) -> u64 {
        SessionEngine::generation(self)
    }

    fn outstanding_attachment_credits(&self) -> u16 {
        SessionEngine::outstanding_attachment_credits(self)
    }

    async fn invoke(&mut self, request_id: RequestId, frame: Vec<u8>) -> Result<Cancellation> {
        SessionEngine::call(self, request_id, frame).await
    }

    fn complete_invoke(&mut self, request_id: &RequestId) -> bool {
        SessionEngine::complete_call(self, request_id)
    }

    async fn cancel(&mut self, generation: u64, request_id: &RequestId) -> Result<()> {
        if generation != SessionEngine::generation(self) {
            return Err(SessionError::new(SessionErrorCode::GenerationMismatch));
        }
        SessionEngine::cancel_call(self, request_id).await
    }

    async fn send_ttrpc(&mut self, frame: Vec<u8>) -> Result<()> {
        SessionEngine::send_ttrpc(self, frame).await
    }

    fn open_named_stream(
        &mut self,
        stream: StreamId,
        send_credit: u32,
        receive_credit: u32,
    ) -> Result<()> {
        SessionEngine::open_named_stream(self, stream, send_credit, receive_credit)
    }

    async fn send_named_stream(&mut self, stream: StreamId, bytes: Vec<u8>) -> Result<()> {
        SessionEngine::send_named_stream(self, stream, bytes).await
    }

    async fn grant_named_stream_credit(&mut self, stream: StreamId, bytes: u32) -> Result<()> {
        SessionEngine::grant_named_stream_credit(self, stream, bytes).await
    }

    async fn close_named_stream(&mut self, stream: StreamId) -> Result<()> {
        SessionEngine::close_named_stream(self, stream).await
    }

    async fn reset_named_stream(&mut self, stream: StreamId) -> Result<()> {
        SessionEngine::reset_named_stream(self, stream).await
    }

    async fn send_attachments(&mut self, attachments: Vec<OwnedAttachment>) -> Result<()> {
        SessionEngine::send_attachments(self, attachments).await
    }

    async fn drive_keepalive(&mut self, now: Instant) -> Result<()> {
        SessionEngine::drive_keepalive(self, now).await
    }

    async fn receive(&mut self) -> Result<SessionEvent> {
        SessionEngine::receive(self).await
    }

    async fn close(&mut self, reason: CloseReason, remediation: Remediation) -> Result<()> {
        SessionEngine::close(self, reason, remediation).await
    }
}
