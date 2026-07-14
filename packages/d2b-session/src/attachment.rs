use std::{any::Any, fmt};

use d2b_contracts::v2_component_session::AttachmentDescriptor;

pub trait AttachmentPayload: Any + Send {
    fn close(self: Box<Self>);

    fn as_any(&self) -> &dyn Any;
}

pub struct OwnedAttachment {
    descriptor: Option<AttachmentDescriptor>,
    payload: Option<Box<dyn AttachmentPayload>>,
}

impl OwnedAttachment {
    pub fn new(descriptor: AttachmentDescriptor, payload: Box<dyn AttachmentPayload>) -> Self {
        Self {
            descriptor: Some(descriptor),
            payload: Some(payload),
        }
    }

    /// Creates an attachment received from a transport before its encrypted
    /// descriptor has been decoded and authenticated by ComponentSession.
    pub fn received(payload: Box<dyn AttachmentPayload>) -> Self {
        Self {
            descriptor: None,
            payload: Some(payload),
        }
    }

    pub fn descriptor(&self) -> Option<&AttachmentDescriptor> {
        self.descriptor.as_ref()
    }

    pub fn payload(&self) -> Option<&dyn Any> {
        self.payload.as_ref().map(|payload| payload.as_any())
    }

    /// Transfers the opaque payload without closing it.
    ///
    /// The recipient becomes the sole close owner. Dropping this attachment
    /// after extraction does not close the transferred payload.
    pub fn into_payload(mut self) -> Option<Box<dyn AttachmentPayload>> {
        self.payload.take()
    }

    pub fn close(mut self) {
        self.close_once();
    }

    pub(crate) fn bind_outbound(
        &mut self,
        index: u16,
        packet_sequence: u64,
        generation: u64,
    ) -> Option<&AttachmentDescriptor> {
        let descriptor = self.descriptor.as_mut()?;
        descriptor.index = index;
        descriptor.packet_sequence = packet_sequence;
        descriptor.reconnect_generation = generation;
        Some(descriptor)
    }

    pub(crate) fn bind_received(mut self, descriptor: AttachmentDescriptor) -> Option<Self> {
        if self.descriptor.is_some() {
            return None;
        }
        self.descriptor = Some(descriptor);
        Some(self)
    }

    fn close_once(&mut self) {
        if let Some(payload) = self.payload.take() {
            payload.close();
        }
    }
}

impl Drop for OwnedAttachment {
    fn drop(&mut self) {
        self.close_once();
    }
}

impl fmt::Debug for OwnedAttachment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OwnedAttachment")
            .field(
                "kind",
                &self
                    .descriptor
                    .as_ref()
                    .map(|descriptor| descriptor.kind.as_str()),
            )
            .field(
                "object_type",
                &self
                    .descriptor
                    .as_ref()
                    .map(|descriptor| descriptor.object_type.as_str()),
            )
            .field("payload", &"<opaque>")
            .finish()
    }
}
