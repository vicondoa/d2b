//! Accepted-file-descriptor admission for the local transport portal.

use crate::graph_binding::{AdmittedTransportRoute, KernelPeerPin};
use d2b_contracts_resource::v3::EndpointAttachmentKind;
use rustix::{
    fd::AsFd,
    fs::{fcntl_getfd, fcntl_setfd},
    io::FdFlags,
    net::{
        AddressFamily, SocketType,
        sockopt::{get_socket_domain, get_socket_type, set_socket_passcred},
    },
};
use std::{error::Error, fmt};

/// The requested Unix socket type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketKind {
    /// A packet-preserving Unix seqpacket socket.
    Seqpacket,
    /// A byte-stream Unix socket.
    Stream,
}

/// The caller's closed transport route class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteClass {
    /// A child ZoneLink route that cannot carry descriptor attachments.
    ZoneLink,
    /// A same-Zone portal route that may carry seqpacket attachments.
    LocalPortal,
}

/// Validated arguments for one transport-open request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenTransportRequest {
    socket_kind: SocketKind,
    route_class: RouteClass,
    attachments_enabled: bool,
}

impl OpenTransportRequest {
    /// Construct a closed transport request.
    pub const fn new(
        socket_kind: SocketKind,
        route_class: RouteClass,
        attachments_enabled: bool,
    ) -> Self {
        Self {
            socket_kind,
            route_class,
            attachments_enabled,
        }
    }

    /// Return the requested socket kind.
    pub const fn socket_kind(self) -> SocketKind {
        self.socket_kind
    }

    /// Return the route class.
    pub const fn route_class(self) -> RouteClass {
        self.route_class
    }

    /// Return whether descriptor attachments are requested.
    pub const fn attachments_enabled(self) -> bool {
        self.attachments_enabled
    }
}

/// Fail-closed admission outcome for an accepted descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportAdmissionError {
    /// The route policy prohibits descriptor attachments.
    AttachmentPolicyConflict,
    /// The descriptor is not an AF_UNIX socket of the declared type.
    SocketKindMismatch,
    /// The descriptor cannot be made close-on-exec.
    Cloexec,
    /// The descriptor's peer credentials could not be read.
    PeerCredentials,
    /// The peer is not the one the admitted relationship is pinned to.
    PeerPolicyMismatch,
    /// The open requests descriptor attachments the admitted relationship
    /// was not admitted for.
    AttachmentKindConflict,
}

impl fmt::Display for TransportAdmissionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::AttachmentPolicyConflict => "attachment-policy-conflict",
            Self::SocketKindMismatch => "socket-kind-mismatch",
            Self::Cloexec => "cloexec-set-failed",
            Self::PeerCredentials => "peer-credentials-unavailable",
            Self::PeerPolicyMismatch => "peer-policy-mismatch",
            Self::AttachmentKindConflict => "attachment-kind-conflict",
        })
    }
}

impl Error for TransportAdmissionError {}

pub(crate) fn validate_route_class(
    request: OpenTransportRequest,
) -> Result<(), TransportAdmissionError> {
    if (request.route_class == RouteClass::ZoneLink || request.socket_kind == SocketKind::Stream)
        && request.attachments_enabled
    {
        return Err(TransportAdmissionError::AttachmentPolicyConflict);
    }
    Ok(())
}

pub(crate) fn validate_and_prepare(
    fd: impl AsFd,
    request: OpenTransportRequest,
) -> Result<(), TransportAdmissionError> {
    validate_route_class(request)?;
    let fd = fd.as_fd();
    if get_socket_domain(fd).ok() != Some(AddressFamily::UNIX)
        || get_socket_type(fd).ok()
            != Some(match request.socket_kind {
                SocketKind::Seqpacket => SocketType::SEQPACKET,
                SocketKind::Stream => SocketType::STREAM,
            })
    {
        return Err(TransportAdmissionError::SocketKindMismatch);
    }
    let flags = fcntl_getfd(fd).map_err(|_| TransportAdmissionError::Cloexec)?;
    fcntl_setfd(fd, flags | FdFlags::CLOEXEC).map_err(|_| TransportAdmissionError::Cloexec)?;
    if request.socket_kind == SocketKind::Seqpacket {
        set_socket_passcred(fd, true).map_err(|_| TransportAdmissionError::PeerCredentials)?;
    }
    Ok(())
}

/// Validate one accepted descriptor for a graph-bound open.
///
/// This runs the same descriptor rules the legacy path runs, and then binds
/// the request to the admitted relationship: an open that asks for
/// descriptor attachments must have been admitted as an `Attach` route, and
/// the kernel peer identity this open enforces must be the relationship's own
/// pin rather than one supplied beside the request.
pub(crate) fn validate_under_relationship(
    fd: impl AsFd,
    request: OpenTransportRequest,
    route: &AdmittedTransportRoute,
    pin: Option<&KernelPeerPin>,
) -> Result<(), TransportAdmissionError> {
    validate_route_class(request)?;
    if request.attachments_enabled() && route.attachment() != EndpointAttachmentKind::Attach {
        return Err(TransportAdmissionError::AttachmentKindConflict);
    }
    if pin.is_some() && pin != route.kernel_peer_pin().as_ref() {
        return Err(TransportAdmissionError::PeerPolicyMismatch);
    }
    validate_and_prepare(fd, request)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph_binding::{
        AdmittedTransportBinding, AdmittedTransportRoute, RelationshipFence,
        TransportBindingRegistry,
    };
    use d2b_contracts_resource::v3::{
        BindingSlot, BoundedToken, DesiredRevision, EndpointBindingRequest, ResourceGeneration,
        ResourceRef, ResourceUid, StoreIncarnation, ZoneDesiredSequence, ZoneId,
        identity::ReconnectGeneration,
    };
    use rustix::net::{SocketFlags, SocketType, socketpair};

    const SOURCE_UID: &str = "1b4e28ba-2fa1-41d2-883f-0016d3cca427";
    const CONSUMER_UID: &str = "2b4e28ba-2fa1-41d2-883f-0016d3cca428";

    fn admitted_route(
        attachment: EndpointAttachmentKind,
        pin: KernelPeerPin,
    ) -> AdmittedTransportRoute {
        let request = EndpointBindingRequest::new(
            ResourceRef::parse("Endpoint/portal").expect("endpoint"),
            ResourceRef::parse("Process/session").expect("consumer"),
            BindingSlot::parse("primary").expect("slot"),
            attachment,
            BoundedToken::parse("session-transport").expect("purpose"),
        )
        .expect("endpoint binding request");
        let key = request
            .key(
                ZoneId::parse("local-root").expect("zone"),
                ResourceUid::parse(SOURCE_UID).expect("source uid"),
                ResourceUid::parse(CONSUMER_UID).expect("consumer uid"),
            )
            .expect("binding key");
        let fence = RelationshipFence::new(
            StoreIncarnation::parse("store-one").expect("incarnation"),
            DesiredRevision::INITIAL.try_next().expect("revision"),
            ZoneDesiredSequence::INITIAL.try_next().expect("sequence"),
            ResourceGeneration::new(1).expect("source generation"),
            ResourceGeneration::new(1).expect("consumer generation"),
            ReconnectGeneration::new(1).expect("reconnect generation"),
        );
        let binding = AdmittedTransportBinding::new(key, request, fence).with_kernel_peer_pin(pin);
        TransportBindingRegistry::new()
            .admit(binding)
            .expect("admitted route")
    }

    fn pair(kind: SocketType) -> (rustix::fd::OwnedFd, rustix::fd::OwnedFd) {
        socketpair(
            AddressFamily::UNIX,
            kind,
            SocketFlags::CLOEXEC | SocketFlags::NONBLOCK,
            None,
        )
        .expect("socketpair")
    }

    #[test]
    fn descriptor_attachments_need_an_attach_relationship() {
        let route = admitted_route(EndpointAttachmentKind::Connect, KernelPeerPin::new(1, 1));
        let (seqpacket, _peer) = pair(SocketType::SEQPACKET);

        assert_eq!(
            validate_under_relationship(
                &seqpacket,
                OpenTransportRequest::new(SocketKind::Seqpacket, RouteClass::LocalPortal, true),
                &route,
                None,
            ),
            Err(TransportAdmissionError::AttachmentKindConflict)
        );
    }

    #[test]
    fn the_enforced_peer_pin_must_be_the_relationships_own() {
        let pinned = KernelPeerPin::new(4_244, 4_244);
        let route = admitted_route(EndpointAttachmentKind::Connect, pinned);
        let (stream, _peer) = pair(SocketType::STREAM);
        let request = OpenTransportRequest::new(SocketKind::Stream, RouteClass::LocalPortal, false);

        assert_eq!(
            validate_under_relationship(
                &stream,
                request,
                &route,
                Some(&KernelPeerPin::new(4_245, 4_244)),
            ),
            Err(TransportAdmissionError::PeerPolicyMismatch)
        );
        assert_eq!(
            validate_under_relationship(&stream, request, &route, Some(&pinned)),
            Ok(())
        );
    }
}
