pub mod wayland;

use std::{
    future::Future,
    os::fd::OwnedFd,
    sync::atomic::{AtomicU64, Ordering},
};

use crate::{
    bridge::{
        ACCEPT_TRANSFER_METHOD_ID, AuthenticatedBridgeError, AuthenticatedBridgeOwner,
        AuthenticatedBridgeTransfer, AuthenticatedTransferDescriptor, BridgeTransferKind,
        BridgeTransferMetadata, TRANSFER_DESCRIPTOR_CREDITS, TransferDescriptorAccess,
        TransferDescriptorKind, TransferDescriptorObject, TransferDescriptorPurpose,
    },
    clipboard::{ClipboardTransferLimits, TransferCreditUsage},
    filter::{ClipboardDescriptorBridge, DescriptorHandoff},
    terminal::SessionDisplayFds,
};

use self::wayland::{
    ControlError, ControlMethod, ControlRequest, ControlResponse, DisplayProviderPort,
    ObservationSink, OpaqueId, WaylandControlService,
};

#[derive(Debug, thiserror::Error)]
pub enum CompositionError {
    #[error("Wayland control rejected the display")]
    Control(#[from] ControlError),
    #[error("ComponentSession supplied invalid display descriptors")]
    Descriptor(#[from] std::io::Error),
    #[error("Wayland control returned an invalid response")]
    ResponseMismatch,
    #[error("ComponentSession control failed")]
    Session,
}

#[derive(Debug)]
pub struct ComposedDisplay {
    pub resource_handle: OpaqueId,
    pub fds: SessionDisplayFds,
}

pub fn open_authenticated_display<P, O>(
    service: &mut WaylandControlService<P, O>,
    request: ControlRequest,
    upstream: OwnedFd,
    listener: OwnedFd,
) -> Result<ComposedDisplay, CompositionError>
where
    P: DisplayProviderPort,
    O: ObservationSink,
{
    if request.method != ControlMethod::OpenDisplay {
        return Err(CompositionError::ResponseMismatch);
    }

    let fds = SessionDisplayFds::from_component_session(upstream, listener)?;
    match service.dispatch(request)? {
        ControlResponse::Opened { resource_handle } => Ok(ComposedDisplay {
            resource_handle,
            fds,
        }),
        _ => Err(CompositionError::ResponseMismatch),
    }
}

pub trait AuthenticatedComponentSessionControl {
    fn generation(&self) -> u64;
    fn register_inbound_call(
        &self,
        request_id: [u8; 16],
    ) -> impl Future<Output = Result<(), CompositionError>>;
    fn complete_inbound_call(
        &self,
        request_id: [u8; 16],
    ) -> impl Future<Output = Result<bool, CompositionError>>;
    fn remove_inbound_call(
        &self,
        request_id: [u8; 16],
    ) -> impl Future<Output = Result<bool, CompositionError>>;
}

pub async fn open_component_session_display<S, P, O>(
    session: &S,
    service: &mut WaylandControlService<P, O>,
    request: ControlRequest,
    upstream: OwnedFd,
    listener: OwnedFd,
) -> Result<ComposedDisplay, CompositionError>
where
    S: AuthenticatedComponentSessionControl,
    P: DisplayProviderPort,
    O: ObservationSink,
{
    if session.generation() != request.session_generation {
        return Err(CompositionError::ResponseMismatch);
    }
    let request_id = request.request_id;
    session.register_inbound_call(request_id).await?;
    let result = open_authenticated_display(service, request, upstream, listener);
    match result {
        Ok(display) => {
            if !session.complete_inbound_call(request_id).await? {
                return Err(CompositionError::ResponseMismatch);
            }
            Ok(display)
        }
        Err(error) => {
            let _ = session.remove_inbound_call(request_id).await?;
            Err(error)
        }
    }
}

pub enum ClipboardDispatch {
    Delivered,
    Backpressure(Box<AuthenticatedBridgeTransfer>),
    Failed,
}

impl std::fmt::Debug for ClipboardDispatch {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Delivered => "ClipboardDispatch::Delivered",
            Self::Backpressure(_) => "ClipboardDispatch::Backpressure(<transfer>)",
            Self::Failed => "ClipboardDispatch::Failed",
        })
    }
}

pub trait AuthenticatedClipboardDispatcher: std::fmt::Debug {
    fn dispatch(&mut self, transfer: AuthenticatedBridgeTransfer) -> ClipboardDispatch;
}

#[derive(Debug)]
pub struct ComponentSessionClipboardBridge<D> {
    owner: AuthenticatedBridgeOwner,
    dispatcher: D,
    generation: u64,
    sequence: AtomicU64,
    limits: ClipboardTransferLimits,
}

impl<D> ComponentSessionClipboardBridge<D> {
    pub fn new(
        owner: AuthenticatedBridgeOwner,
        dispatcher: D,
        generation: u64,
        limits: ClipboardTransferLimits,
    ) -> Result<Self, AuthenticatedBridgeError> {
        if generation == 0 {
            return Err(AuthenticatedBridgeError::DescriptorMismatch);
        }
        Ok(Self {
            owner,
            dispatcher,
            generation,
            sequence: AtomicU64::new(1),
            limits,
        })
    }

    fn descriptor(
        &self,
        metadata: &BridgeTransferMetadata,
        sequence: u64,
    ) -> AuthenticatedTransferDescriptor {
        let (access, purpose) = match metadata.kind {
            BridgeTransferKind::CopySelection => (
                TransferDescriptorAccess::Read,
                TransferDescriptorPurpose::CopyMaterialization,
            ),
            BridgeTransferKind::PasteRequest => (
                TransferDescriptorAccess::Write,
                TransferDescriptorPurpose::PasteTarget,
            ),
        };
        let mut request_id = [0_u8; 16];
        request_id[..8].copy_from_slice(&sequence.to_be_bytes());
        request_id[8..].copy_from_slice(&metadata.source_id.to_be_bytes());
        let mut operation_id = request_id;
        operation_id[0] ^= 0x80;
        AuthenticatedTransferDescriptor {
            index: 0,
            descriptor_count: 1,
            kind: TransferDescriptorKind::FileDescriptor,
            object: TransferDescriptorObject::ClipboardTransfer,
            access,
            purpose,
            service_package: "d2b.clipboard.v2",
            method_id: ACCEPT_TRANSFER_METHOD_ID,
            request_id,
            operation_id,
            packet_sequence: sequence,
            reconnect_generation: self.generation,
            cloexec_required: true,
            duplicate_object_allowed: false,
            credit_classes: TRANSFER_DESCRIPTOR_CREDITS,
            declared_payload_bytes: 0,
        }
    }
}

impl<D> ClipboardDescriptorBridge for ComponentSessionClipboardBridge<D>
where
    D: AuthenticatedClipboardDispatcher,
{
    fn handoff(&mut self, fd: OwnedFd, metadata: &BridgeTransferMetadata) -> DescriptorHandoff {
        let sequence = self.sequence.fetch_add(1, Ordering::Relaxed);
        if sequence == u64::MAX {
            return DescriptorHandoff::Failed;
        }
        let descriptor = self.descriptor(metadata, sequence);
        let transfer = match AuthenticatedBridgeTransfer::new(
            &self.owner,
            fd,
            metadata.clone(),
            descriptor,
            TransferCreditUsage::default(),
            self.limits,
        ) {
            Ok(transfer) => transfer,
            Err(_) => return DescriptorHandoff::Failed,
        };
        match self.dispatcher.dispatch(transfer) {
            ClipboardDispatch::Delivered => DescriptorHandoff::Delivered,
            ClipboardDispatch::Backpressure(transfer) => {
                let (fd, _, _) = (*transfer).into_parts();
                DescriptorHandoff::Backpressure(fd)
            }
            ClipboardDispatch::Failed => DescriptorHandoff::Failed,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use d2b_core::workload_identity::WorkloadTarget;
    use d2b_realm_core::WorkloadProviderKind;

    use super::*;
    use crate::{
        bridge::BridgeTransferMetadata,
        identity::ProxyIdentity,
        services::wayland::{OpaqueId, SessionIdentity},
    };

    #[derive(Debug)]
    struct RecordingDispatcher(Arc<AtomicUsize>);

    impl AuthenticatedClipboardDispatcher for RecordingDispatcher {
        fn dispatch(&mut self, _: AuthenticatedBridgeTransfer) -> ClipboardDispatch {
            self.0.fetch_add(1, Ordering::Relaxed);
            ClipboardDispatch::Delivered
        }
    }

    fn authenticated_identity() -> ProxyIdentity {
        let target = WorkloadTarget::parse("work.host.d2b").unwrap();
        let component = ProxyIdentity::canonical(target.clone(), WorkloadProviderKind::LocalVm)
            .bridge_component();
        ProxyIdentity::from_component_session(
            target,
            WorkloadProviderKind::LocalVm,
            SessionIdentity {
                realm_id: OpaqueId::parse("local").unwrap(),
                workload_id: OpaqueId::parse(component).unwrap(),
                provider_id: OpaqueId::parse("local-vm").unwrap(),
                role_id: OpaqueId::parse("wayland-proxy").unwrap(),
            },
        )
    }

    #[test]
    fn clipboard_handoff_is_owned_and_descriptor_authenticated() {
        let identity = authenticated_identity();
        let owner = AuthenticatedBridgeOwner::from_component_session(identity.clone(), 7).unwrap();
        let delivered = Arc::new(AtomicUsize::new(0));
        let mut bridge = ComponentSessionClipboardBridge::new(
            owner,
            RecordingDispatcher(delivered.clone()),
            7,
            ClipboardTransferLimits::default(),
        )
        .unwrap();
        let (read, _write) = rustix::pipe::pipe_with(rustix::pipe::PipeFlags::CLOEXEC).unwrap();
        let metadata = BridgeTransferMetadata {
            identity,
            mime_type: "text/plain".to_owned(),
            source_id: 11,
            kind: BridgeTransferKind::CopySelection,
        };

        assert!(matches!(
            bridge.handoff(read, &metadata),
            DescriptorHandoff::Delivered
        ));
        assert_eq!(delivered.load(Ordering::Relaxed), 1);
    }
}
