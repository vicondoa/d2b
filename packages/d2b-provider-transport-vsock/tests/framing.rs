use std::any::Any;

use d2b_contracts::v3::component_session::AttachmentDescriptor;
use d2b_provider_transport_vsock::{FramedVsockTransport, TransportError};
use d2b_session::{
    AttachmentPayload, AttachmentValidationError, OwnedAttachment, OwnedTransport, TransportPacket,
};
use tokio::{
    io::{AsyncWriteExt, duplex},
    runtime::Builder,
};

struct TestAttachment;

impl AttachmentPayload for TestAttachment {
    fn close(self: Box<Self>) {}

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn into_any(self: Box<Self>) -> Box<dyn Any + Send> {
        self
    }

    fn validate_descriptor(
        &self,
        _descriptor: &AttachmentDescriptor,
    ) -> Result<(), AttachmentValidationError> {
        Ok(())
    }
}

#[test]
fn framed_transport_handles_partial_and_coalesced_records() {
    let runtime = Builder::new_current_thread()
        .enable_io()
        .build()
        .expect("runtime");
    runtime.block_on(async {
        let (mut sender, receiver) = duplex(128);
        let writer = tokio::spawn(async move {
            sender.write_all(&[0, 5, b'h', b'e']).await.unwrap();
            sender.write_all(b"llo\0\x05world").await.unwrap();
        });

        let mut transport = FramedVsockTransport::new(receiver);
        assert_eq!(
            transport.vsock_descriptor(),
            d2b_provider_transport_vsock::VsockTransportDescriptor::default()
        );
        assert_eq!(transport.receive(64).await.unwrap().as_bytes(), b"hello");
        assert_eq!(transport.receive(64).await.unwrap().as_bytes(), b"world");
        writer.await.unwrap();
    });
}

#[test]
fn framed_transport_rejects_oversized_records_before_allocation() {
    let runtime = Builder::new_current_thread()
        .enable_io()
        .build()
        .expect("runtime");
    runtime.block_on(async {
        let (mut sender, receiver) = duplex(16);
        sender.write_all(&[0, 8, 1, 2]).await.unwrap();
        let mut transport = FramedVsockTransport::with_limit(receiver, 4);
        assert_eq!(
            transport.read_frame().await,
            Err(TransportError::FrameTooLarge)
        );
    });
}

#[test]
fn framed_transport_rejects_attachments() {
    let runtime = Builder::new_current_thread()
        .enable_io()
        .build()
        .expect("runtime");
    runtime.block_on(async {
        let (_sender, receiver) = duplex(64);
        let mut transport = FramedVsockTransport::new(receiver);
        let packet = TransportPacket::with_attachments(
            b"payload".to_vec(),
            vec![OwnedAttachment::unbound(Box::new(TestAttachment))],
        );
        assert_eq!(
            transport.send(packet).await,
            Err(d2b_session::TransportError::InvalidAttachment)
        );
    });
}
