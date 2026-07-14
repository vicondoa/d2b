use std::{any::Any, fmt};

use d2b_contracts::v2_component_session::AttachmentDescriptor;

pub trait AttachmentPayload: Any + Send {
    fn close(self: Box<Self>);

    fn as_any(&self) -> &dyn Any;
}

pub struct OwnedAttachment {
    descriptor: AttachmentDescriptor,
    payload: Option<Box<dyn AttachmentPayload>>,
}

impl OwnedAttachment {
    pub fn new(descriptor: AttachmentDescriptor, payload: Box<dyn AttachmentPayload>) -> Self {
        Self {
            descriptor,
            payload: Some(payload),
        }
    }

    pub fn descriptor(&self) -> &AttachmentDescriptor {
        &self.descriptor
    }

    pub fn payload(&self) -> Option<&dyn Any> {
        self.payload.as_ref().map(|payload| payload.as_any())
    }

    pub fn close(mut self) {
        self.close_once();
    }

    pub(crate) fn bind(&mut self, index: u16, packet_sequence: u64, generation: u64) {
        self.descriptor.index = index;
        self.descriptor.packet_sequence = packet_sequence;
        self.descriptor.reconnect_generation = generation;
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
            .field("kind", &self.descriptor.kind.as_str())
            .field("object_type", &self.descriptor.object_type.as_str())
            .field("payload", &"<opaque>")
            .finish()
    }
}
