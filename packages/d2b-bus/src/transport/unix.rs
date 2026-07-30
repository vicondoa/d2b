//! Zone bus Unix transport.
//!
//! This module adapts the audited Unix transport substrate to the v3 Zone
//! model. Two things change relative to the substrate it is built on.
//!
//! The first is how a transport descriptor arrives. There is no socket
//! activation here: the Zone allocator pre-binds the edge and hands the Zone
//! runtime an already-connected descriptor, which arrives as an
//! [`AllocatedTransportFd`] and is consumed by value exactly once. This module
//! reads no environment variable and consults no activation protocol; a
//! descriptor that was not handed to it does not exist as far as it is
//! concerned.
//!
//! The second is peer identity. The v3 shapes are a pathname endpoint whose
//! provenance a caller-supplied verifier proves, and an inherited socketpair
//! whose expected peer is read from the kernel through the descriptor itself.
//! Neither shape accepts caller-asserted credentials, and neither maps a peer
//! to a Zone subject: authoritative subject resolution is exclusively the Zone
//! registrar's, and doing it here would be a second, unaudited path to naming a
//! principal.
//!
//! Descriptor discipline is delegated to the substrate and is not weakened
//! here. Admission requires a close-on-exec, non-blocking Unix socket of the
//! exact expected kind; anything else is refused with a typed reason and the
//! descriptor is closed by the same move that presented it.

use crate::transport::credit::{AttachmentCreditPlan, CreditError, RouteClass};
use d2b_contracts::v3::component_session::{LimitProfile, Locality, TransportClass};
use d2b_session::OwnedTransport;
use d2b_session_unix::{
    PeerIdentityPolicy, SeqpacketSocket, StreamSocket, UnixSeqpacketTransport, UnixSessionError,
    UnixStreamTransport,
};
use std::{error::Error, fmt, os::fd::OwnedFd};

pub use d2b_session_unix::{
    DescriptorPolicy, DescriptorPolicyResolver, PathnamePeerVerifier, UnixTransportEvent,
    UnixTransportFailure, UnixTransportObserver,
};

/// The socket kind an allocator-issued descriptor is declared to be.
///
/// The declaration is a routing hint only. The substrate independently
/// re-derives the real kind from the kernel and refuses a mismatch, so a wrong
/// declaration cannot widen anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AllocatedSocketKind {
    /// A connected `AF_UNIX` seqpacket endpoint, which can carry attachments.
    Seqpacket,
    /// A connected `AF_UNIX` stream endpoint, which cannot.
    Stream,
}

impl AllocatedSocketKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Seqpacket => "seqpacket",
            Self::Stream => "stream",
        }
    }
}

/// One allocator-issued, already-connected transport descriptor.
///
/// This is the v3 replacement for socket-activation inheritance. The allocator
/// pre-binds the edge and transfers the descriptor explicitly; the receiving
/// runtime wraps it here and hands it to exactly one transport.
///
/// The descriptor is owned. There is no accessor that borrows or duplicates it,
/// no clone, and no default, so a single handoff can be opened exactly once:
/// opening consumes the value, and dropping it without opening closes the
/// descriptor. It is never retained as authority - the authority is the
/// authenticated session the transport later carries, not the descriptor.
pub struct AllocatedTransportFd {
    descriptor: OwnedFd,
    kind: AllocatedSocketKind,
}

impl fmt::Debug for AllocatedTransportFd {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AllocatedTransportFd")
            .field("kind", &self.kind)
            .field("descriptor", &"<redacted>")
            .finish()
    }
}

impl AllocatedTransportFd {
    /// Receives an allocator-issued connected seqpacket endpoint.
    pub fn seqpacket(descriptor: OwnedFd) -> Self {
        Self {
            descriptor,
            kind: AllocatedSocketKind::Seqpacket,
        }
    }

    /// Receives an allocator-issued connected stream endpoint.
    pub fn stream(descriptor: OwnedFd) -> Self {
        Self {
            descriptor,
            kind: AllocatedSocketKind::Stream,
        }
    }

    pub const fn kind(&self) -> AllocatedSocketKind {
        self.kind
    }

    fn take(self, expected: AllocatedSocketKind) -> Result<OwnedFd, TransportAdmissionError> {
        if self.kind != expected {
            return Err(TransportAdmissionError::SocketKindMismatch);
        }
        Ok(self.descriptor)
    }
}

/// How the peer on an allocator-issued descriptor is proven.
///
/// This deliberately carries no user, group, or process identifier. Expected
/// credentials come from the kernel through the descriptor; provenance for a
/// pathname endpoint comes from a verifier the endpoint owner supplies.
pub enum ZonePeerIdentity {
    /// A pathname endpoint whose provenance the supplied verifier proves.
    Pathname(PathnamePeerVerifier),
    /// An inherited socketpair. The expected peer is read from the kernel
    /// through the descriptor itself and pinned before the first packet, so a
    /// peer cannot later assert a different identity.
    InheritedSocketpair,
}

impl fmt::Debug for ZonePeerIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Pathname(_) => "ZonePeerIdentity::Pathname",
            Self::InheritedSocketpair => "ZonePeerIdentity::InheritedSocketpair",
        })
    }
}

impl ZonePeerIdentity {
    const fn transport_class(&self) -> TransportClass {
        match self {
            Self::Pathname(_) => TransportClass::UnixSeqpacket,
            Self::InheritedSocketpair => TransportClass::InheritedSocketpair,
        }
    }
}

/// Typed refusal reasons for admitting an allocator-issued descriptor.
///
/// Every variant names a decision, not a location. No path, descriptor number,
/// socket name, or peer identifier appears in any variant or its rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportAdmissionError {
    /// The descriptor is not a Unix socket of the expected kind.
    SocketKindMismatch,
    /// The descriptor is not close-on-exec.
    MissingCloexec,
    /// The descriptor is in blocking mode.
    BlockingDescriptor,
    /// An inherited socketpair was not pre-armed for credential passing by its
    /// parent, so first-packet credentials cannot be trusted.
    PasscredNotPrearmed,
    /// Peer identity could not be proven.
    PeerIdentityRejected,
    /// The attachment policy is not admissible for this transport class.
    AttachmentPolicyRejected,
    /// The route class of the credit plan does not match the transport being
    /// opened.
    RouteClassMismatch,
    /// Attachment credit admission refused.
    Credit(CreditError),
    /// The descriptor is already closed.
    Closed,
    /// A framing, truncation, or fairness rule was violated.
    Protocol,
    /// The descriptor could not be interrogated or driven.
    Io,
}

impl fmt::Display for TransportAdmissionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SocketKindMismatch => formatter.write_str("transport-socket-kind-mismatch"),
            Self::MissingCloexec => formatter.write_str("transport-missing-cloexec"),
            Self::BlockingDescriptor => formatter.write_str("transport-blocking-descriptor"),
            Self::PasscredNotPrearmed => formatter.write_str("transport-passcred-not-prearmed"),
            Self::PeerIdentityRejected => formatter.write_str("transport-peer-identity-rejected"),
            Self::AttachmentPolicyRejected => {
                formatter.write_str("transport-attachment-policy-rejected")
            }
            Self::RouteClassMismatch => formatter.write_str("transport-route-class-mismatch"),
            Self::Credit(inner) => write!(formatter, "transport-credit({inner})"),
            Self::Closed => formatter.write_str("transport-closed"),
            Self::Protocol => formatter.write_str("transport-protocol-violation"),
            Self::Io => formatter.write_str("transport-io"),
        }
    }
}

impl Error for TransportAdmissionError {}

impl TransportAdmissionError {
    /// Classifies a substrate error into a Zone-level refusal reason.
    ///
    /// The mapping is total and fails closed: there is no permissive fallback
    /// variant, and an errno is dropped rather than surfaced.
    fn classify(error: UnixSessionError) -> Self {
        match error {
            UnixSessionError::InvalidSocket => Self::SocketKindMismatch,
            UnixSessionError::MissingCloexec => Self::MissingCloexec,
            UnixSessionError::BlockingSocket => Self::BlockingDescriptor,
            UnixSessionError::PasscredNotPrearmed => Self::PasscredNotPrearmed,
            UnixSessionError::CredentialMismatch
            | UnixSessionError::DescriptorMismatch
            | UnixSessionError::DuplicateObject
            | UnixSessionError::PidfdEvidenceUnavailable
            | UnixSessionError::PidfdIdentityMismatch => Self::PeerIdentityRejected,
            UnixSessionError::AncillaryCapacity => Self::AttachmentPolicyRejected,
            UnixSessionError::CreditExceeded => {
                Self::Credit(CreditError::AttachmentAllowanceExceeded)
            }
            UnixSessionError::Closed => Self::Closed,
            UnixSessionError::EmptyPacket
            | UnixSessionError::PayloadLimit
            | UnixSessionError::MessageTruncated
            | UnixSessionError::ControlTruncated
            | UnixSessionError::UnknownControl
            | UnixSessionError::ControlMismatch
            | UnixSessionError::PacketNotAtomic
            | UnixSessionError::FairnessBudget => Self::Protocol,
            UnixSessionError::Io { .. } => Self::Io,
        }
    }
}

impl From<CreditError> for TransportAdmissionError {
    fn from(error: CreditError) -> Self {
        Self::Credit(error)
    }
}

/// Opens a within-Zone seqpacket transport over an allocator-issued descriptor.
///
/// The plan must be a within-Zone plan: a ZoneLink plan owns no credit pools
/// and is refused here rather than silently downgraded to an attachment-free
/// transport.
pub fn open_within_zone_seqpacket(
    allocated: AllocatedTransportFd,
    locality: Locality,
    limits: LimitProfile,
    plan: &AttachmentCreditPlan,
    resolver: DescriptorPolicyResolver,
    peer_identity: ZonePeerIdentity,
) -> Result<Box<dyn OwnedTransport>, TransportAdmissionError> {
    if plan.route_class() != RouteClass::WithinZone {
        return Err(TransportAdmissionError::RouteClassMismatch);
    }
    let class = peer_identity.transport_class();
    let descriptor = allocated.take(AllocatedSocketKind::Seqpacket)?;
    let (socket, policy) = match peer_identity {
        ZonePeerIdentity::Pathname(verifier) => {
            let socket = SeqpacketSocket::from_owned(descriptor)
                .map_err(TransportAdmissionError::classify)?;
            (socket, PeerIdentityPolicy::pathname(verifier))
        }
        ZonePeerIdentity::InheritedSocketpair => {
            let socket = SeqpacketSocket::from_parent_prearmed(descriptor)
                .map_err(TransportAdmissionError::classify)?;
            // The expected peer is read from the kernel through this exact
            // descriptor. It is never supplied by a caller and never parsed
            // from a payload.
            let expected_peer = socket
                .acceptor_peer_credentials()
                .map_err(TransportAdmissionError::classify)?;
            (
                socket,
                PeerIdentityPolicy::inherited_socketpair(expected_peer),
            )
        }
    };
    let attachment_policy = plan.attachment_policy();
    attachment_policy
        .validate(class)
        .map_err(|_| TransportAdmissionError::AttachmentPolicyRejected)?;
    let credits = plan.credit_scopes()?;
    let transport = UnixSeqpacketTransport::new(
        socket,
        locality,
        limits,
        attachment_policy,
        credits,
        resolver,
        policy,
    )
    .map_err(TransportAdmissionError::classify)?;
    Ok(Box::new(transport))
}

/// Opens a ZoneLink transport over an allocator-issued descriptor.
///
/// A ZoneLink hop is always attachment-free, so it is carried by a framed
/// stream transport that has no ancillary-data path at all. The absence of
/// descriptor passing is therefore structural rather than a policy the caller
/// could relax.
pub fn open_zone_link_stream(
    allocated: AllocatedTransportFd,
    locality: Locality,
    limits: LimitProfile,
    plan: &AttachmentCreditPlan,
) -> Result<Box<dyn OwnedTransport>, TransportAdmissionError> {
    if plan.route_class() != RouteClass::ZoneLink {
        return Err(TransportAdmissionError::RouteClassMismatch);
    }
    plan.admit(0)?;
    let descriptor = allocated.take(AllocatedSocketKind::Stream)?;
    let socket = StreamSocket::from_owned(descriptor).map_err(TransportAdmissionError::classify)?;
    let transport = UnixStreamTransport::new(socket, locality, limits);
    Ok(Box::new(transport))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::credit::ProcessCreditLimit;
    use d2b_contracts::v3::component_session::{AttachmentPolicy, AttachmentPolicyKind};
    use d2b_session_unix::prearmed_seqpacket_pair;
    use std::{os::unix::net::UnixStream, sync::Arc};

    fn packet_atomic(max_per_packet: u16) -> AttachmentPolicy {
        AttachmentPolicy {
            kind: AttachmentPolicyKind::PacketAtomic,
            max_per_packet,
            max_per_request: max_per_packet,
            max_per_operation: max_per_packet,
            max_per_session: max_per_packet,
            credentials_allowed: true,
        }
    }

    fn within_zone_plan() -> AttachmentCreditPlan {
        AttachmentCreditPlan::within_zone(
            TransportClass::InheritedSocketpair,
            packet_atomic(2),
            ProcessCreditLimit::derive(1024, 64).expect("derive process capacity"),
            256,
        )
        .expect("build a within-Zone plan")
    }

    fn resolver() -> DescriptorPolicyResolver {
        Arc::new(|_| Err(UnixSessionError::DescriptorMismatch))
    }

    fn allocated_stream_pair() -> (OwnedFd, OwnedFd) {
        let (left, right) = UnixStream::pair().expect("create a Unix stream pair");
        left.set_nonblocking(true).expect("arm the local endpoint");
        right.set_nonblocking(true).expect("arm the peer endpoint");
        (OwnedFd::from(left), OwnedFd::from(right))
    }

    #[tokio::test]
    async fn an_allocator_issued_seqpacket_opens_an_inherited_within_zone_transport() {
        let (local, peer) = prearmed_seqpacket_pair().expect("create a prearmed pair");
        let plan = within_zone_plan();
        let transport = open_within_zone_seqpacket(
            AllocatedTransportFd::seqpacket(local),
            Locality::HostLocal,
            LimitProfile::local_default(),
            &plan,
            resolver(),
            ZonePeerIdentity::InheritedSocketpair,
        )
        .expect("open the allocator-issued transport");
        let descriptor = transport.descriptor();
        assert_eq!(descriptor.class, TransportClass::InheritedSocketpair);
        assert_eq!(descriptor.locality, Locality::HostLocal);
        assert!(descriptor.packet_atomic);
        assert!(descriptor.supports_attachments);
        drop(peer);
    }

    #[tokio::test]
    async fn an_allocator_issued_stream_opens_an_attachment_free_zone_link_transport() {
        let (local, peer) = allocated_stream_pair();
        let plan = AttachmentCreditPlan::zone_link();
        let transport = open_zone_link_stream(
            AllocatedTransportFd::stream(local),
            Locality::Remote,
            LimitProfile::local_default(),
            &plan,
        )
        .expect("open the allocator-issued ZoneLink transport");
        let descriptor = transport.descriptor();
        assert_eq!(descriptor.class, TransportClass::UnixStream);
        assert!(
            !descriptor.supports_attachments,
            "a ZoneLink hop must have no descriptor-passing path"
        );
        assert!(!descriptor.packet_atomic);
        drop(peer);
    }

    #[tokio::test]
    async fn a_declared_kind_that_contradicts_the_request_is_refused() {
        let (local, peer) = prearmed_seqpacket_pair().expect("create a prearmed pair");
        let plan = within_zone_plan();
        let refusal = open_within_zone_seqpacket(
            AllocatedTransportFd::stream(local),
            Locality::HostLocal,
            LimitProfile::local_default(),
            &plan,
            resolver(),
            ZonePeerIdentity::InheritedSocketpair,
        )
        .err();
        assert_eq!(refusal, Some(TransportAdmissionError::SocketKindMismatch));
        drop(peer);
    }

    #[tokio::test]
    async fn a_stream_descriptor_presented_as_a_seqpacket_is_refused_by_the_kernel_kind() {
        let (local, peer) = allocated_stream_pair();
        let plan = AttachmentCreditPlan::within_zone(
            TransportClass::UnixSeqpacket,
            packet_atomic(2),
            ProcessCreditLimit::derive(1024, 64).expect("derive process capacity"),
            256,
        )
        .expect("build a pathname within-Zone plan");
        let verifier: PathnamePeerVerifier = Arc::new(|_| Ok(()));
        let refusal = open_within_zone_seqpacket(
            AllocatedTransportFd::seqpacket(local),
            Locality::HostLocal,
            LimitProfile::local_default(),
            &plan,
            resolver(),
            ZonePeerIdentity::Pathname(verifier),
        )
        .err();
        assert_eq!(refusal, Some(TransportAdmissionError::SocketKindMismatch));
        drop(peer);
    }

    #[tokio::test]
    async fn an_inherited_descriptor_that_was_not_prearmed_is_refused() {
        let (local, peer) = allocated_stream_pair();
        let plan = within_zone_plan();
        let refusal = open_within_zone_seqpacket(
            AllocatedTransportFd::seqpacket(local),
            Locality::HostLocal,
            LimitProfile::local_default(),
            &plan,
            resolver(),
            ZonePeerIdentity::InheritedSocketpair,
        )
        .err();
        assert_eq!(
            refusal,
            Some(TransportAdmissionError::PasscredNotPrearmed),
            "an inherited endpoint whose parent did not prearm credential passing is refused"
        );
        drop(peer);
    }

    #[tokio::test]
    async fn a_zone_link_plan_never_opens_a_within_zone_seqpacket() {
        let (local, peer) = prearmed_seqpacket_pair().expect("create a prearmed pair");
        let plan = AttachmentCreditPlan::zone_link();
        let refusal = open_within_zone_seqpacket(
            AllocatedTransportFd::seqpacket(local),
            Locality::HostLocal,
            LimitProfile::local_default(),
            &plan,
            resolver(),
            ZonePeerIdentity::InheritedSocketpair,
        )
        .err();
        assert_eq!(refusal, Some(TransportAdmissionError::RouteClassMismatch));
        drop(peer);
    }

    #[tokio::test]
    async fn a_within_zone_plan_never_opens_a_zone_link_stream() {
        let (local, peer) = allocated_stream_pair();
        let plan = within_zone_plan();
        let refusal = open_zone_link_stream(
            AllocatedTransportFd::stream(local),
            Locality::Remote,
            LimitProfile::local_default(),
            &plan,
        )
        .err();
        assert_eq!(refusal, Some(TransportAdmissionError::RouteClassMismatch));
        drop(peer);
    }

    #[tokio::test]
    async fn a_blocking_allocator_descriptor_is_refused_rather_than_repaired() {
        let (left, right) = UnixStream::pair().expect("create a Unix stream pair");
        let plan = AttachmentCreditPlan::zone_link();
        let refusal = open_zone_link_stream(
            AllocatedTransportFd::stream(OwnedFd::from(left)),
            Locality::Remote,
            LimitProfile::local_default(),
            &plan,
        )
        .err();
        assert_eq!(
            refusal,
            Some(TransportAdmissionError::BlockingDescriptor),
            "the transport layer never silently rearms a descriptor it was handed"
        );
        drop(right);
    }

    #[test]
    fn descriptor_discipline_failures_map_to_their_own_typed_reason() {
        for (substrate, expected) in [
            (
                UnixSessionError::MissingCloexec,
                TransportAdmissionError::MissingCloexec,
            ),
            (
                UnixSessionError::BlockingSocket,
                TransportAdmissionError::BlockingDescriptor,
            ),
            (
                UnixSessionError::InvalidSocket,
                TransportAdmissionError::SocketKindMismatch,
            ),
            (
                UnixSessionError::PasscredNotPrearmed,
                TransportAdmissionError::PasscredNotPrearmed,
            ),
            (
                UnixSessionError::CredentialMismatch,
                TransportAdmissionError::PeerIdentityRejected,
            ),
            (
                UnixSessionError::DuplicateObject,
                TransportAdmissionError::PeerIdentityRejected,
            ),
            (
                UnixSessionError::PidfdIdentityMismatch,
                TransportAdmissionError::PeerIdentityRejected,
            ),
            (
                UnixSessionError::AncillaryCapacity,
                TransportAdmissionError::AttachmentPolicyRejected,
            ),
            (
                UnixSessionError::CreditExceeded,
                TransportAdmissionError::Credit(CreditError::AttachmentAllowanceExceeded),
            ),
            (UnixSessionError::Closed, TransportAdmissionError::Closed),
            (
                UnixSessionError::PacketNotAtomic,
                TransportAdmissionError::Protocol,
            ),
            (
                UnixSessionError::Io { errno: Some(9) },
                TransportAdmissionError::Io,
            ),
        ] {
            assert_eq!(TransportAdmissionError::classify(substrate), expected);
        }
        assert_eq!(
            TransportAdmissionError::Io.to_string(),
            "transport-io",
            "no errno reaches the rendered reason"
        );
        assert_eq!(
            TransportAdmissionError::MissingCloexec.to_string(),
            "transport-missing-cloexec"
        );
    }

    #[test]
    fn the_allocator_handoff_consults_no_socket_activation_protocol() {
        // The v3 handoff is an explicit descriptor transfer. Any residual
        // socket-activation path would reintroduce an ambient, unowned source
        // of descriptors, so this module must name none of it. The needles are
        // assembled at runtime so this assertion does not match itself.
        let source = include_str!("unix.rs");
        for needle in [
            ["LIS", "TEN_FDS"].concat(),
            ["LIS", "TEN_PID"].concat(),
            ["LIS", "TEN_FDNAMES"].concat(),
            ["SD_", "LIS", "TEN_FDS"].concat(),
            ["std::", "env"].concat(),
            ["Listen", "Fd"].concat(),
            ["from_", "systemd"].concat(),
        ] {
            assert!(
                !source.contains(&needle),
                "the allocator handoff must not reference a socket-activation symbol"
            );
        }
    }

    #[test]
    fn an_unopened_handoff_reports_only_its_declared_kind() {
        let (local, peer) = allocated_stream_pair();
        let handoff = AllocatedTransportFd::stream(local);
        assert_eq!(handoff.kind(), AllocatedSocketKind::Stream);
        assert_eq!(handoff.kind().as_str(), "stream");
        assert_eq!(AllocatedSocketKind::Seqpacket.as_str(), "seqpacket");
        let rendered = format!("{handoff:?}");
        assert!(rendered.contains("<redacted>"));
        assert!(rendered.contains("Stream"));
        assert!(
            !rendered.chars().any(|character| character.is_ascii_digit()),
            "no descriptor number is rendered"
        );
        drop(handoff);
        drop(peer);
    }

    #[test]
    fn peer_identity_debug_names_the_shape_and_nothing_else() {
        assert_eq!(
            format!("{:?}", ZonePeerIdentity::InheritedSocketpair),
            "ZonePeerIdentity::InheritedSocketpair"
        );
        let verifier: PathnamePeerVerifier = Arc::new(|_| Ok(()));
        assert_eq!(
            format!("{:?}", ZonePeerIdentity::Pathname(verifier)),
            "ZonePeerIdentity::Pathname"
        );
    }
}
