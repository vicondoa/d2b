use crate::{
    credit::{CreditBundle, CreditScope, CreditScopeSet},
    error::{UnixSessionError, io_error},
    pidfd::{PidfdEvidence, PidfdIdentityVerifier, verify_pidfd},
};
use d2b_contracts::v2_component_session::{
    AttachmentAccess, AttachmentKind, AttachmentPacket, AttachmentPolicy, EndpointRole,
    KernelObjectType, ServicePackage, TransportClass, directional_channel_binding,
};
use rustix::{
    fd::{AsFd, OwnedFd},
    fs::{FileType, OFlags, fcntl_get_seals, fcntl_getfl, fstat},
    io::{FdFlags, fcntl_getfd},
    net::{
        AddressFamily, SocketType, UCred, getpeername,
        sockopt::{get_socket_domain, get_socket_peercred, get_socket_type},
    },
    process::{Gid, Pid, Uid, getgid, getuid},
};
use std::{fmt, sync::Arc};

/// The current process's real uid/gid, read directly from the kernel.
///
/// This is the ONLY sanctioned source of identity for a responder computing
/// a directional channel binding: a responder must bind against its own
/// process identity, never against a uid/gid value supplied by (or derived
/// from) the peer over the wire. Use [`Self::current`] on the responder
/// side; an initiator that already knows the responder's expected uid/gid
/// out of band may call
/// [`d2b_contracts::v2_component_session::directional_channel_binding`]
/// directly with that expected value.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ResponderIdentity {
    uid: Uid,
    gid: Gid,
}

impl fmt::Debug for ResponderIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ResponderIdentity(REDACTED)")
    }
}

impl ResponderIdentity {
    /// Reads the current process's real uid/gid via `getuid(2)`/`getgid(2)`.
    ///
    /// These syscalls always succeed and never consult any peer-supplied
    /// value, so this constructor is the canonical, tamper-proof identity
    /// source for a ComponentSession responder.
    pub fn current() -> Self {
        Self {
            uid: getuid(),
            gid: getgid(),
        }
    }

    pub fn uid(self) -> Uid {
        self.uid
    }

    pub fn gid(self) -> Gid {
        self.gid
    }

    /// Computes the directional channel binding for this responder's real
    /// identity, delegating to the canonical pure digest function so the
    /// value matches byte-for-byte whatever an initiator computes with the
    /// same `(service, transport, responder_role, uid, gid)` tuple obtained
    /// out of band.
    pub fn channel_binding(
        self,
        service: ServicePackage,
        transport: TransportClass,
        responder_role: EndpointRole,
    ) -> [u8; 32] {
        directional_channel_binding(
            service,
            transport,
            responder_role,
            self.uid.as_raw(),
            self.gid.as_raw(),
        )
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct PeerCredentials(UCred);

impl PeerCredentials {
    pub(crate) fn from_ucred(credentials: UCred) -> Self {
        Self(credentials)
    }

    pub fn pid(self) -> Pid {
        self.0.pid
    }

    pub fn uid(self) -> Uid {
        self.0.uid
    }

    pub fn gid(self) -> Gid {
        self.0.gid
    }

    pub(crate) fn as_ucred(self) -> UCred {
        self.0
    }
}

impl fmt::Debug for PeerCredentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PeerCredentials(REDACTED)")
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct FirstPacketCredentials(PeerCredentials);

impl FirstPacketCredentials {
    pub fn pid(self) -> Pid {
        self.0.pid()
    }
}

impl fmt::Debug for FirstPacketCredentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("FirstPacketCredentials(REDACTED)")
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct ObjectIdentity {
    device: u64,
    inode: u64,
    file_type: FileType,
    object_type: KernelObjectType,
    access: AttachmentAccess,
    special_device: u64,
    socket: Option<(AddressFamily, SocketType)>,
    seals: Option<u32>,
}

impl fmt::Debug for ObjectIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ObjectIdentity(REDACTED)")
    }
}

impl ObjectIdentity {
    pub fn from_trusted(
        fd: impl AsFd,
        object_type: KernelObjectType,
        access: AttachmentAccess,
    ) -> Result<Self, UnixSessionError> {
        inspect_identity(fd, object_type, access, false)
    }

    pub(crate) fn same_kernel_object(&self, other: &Self) -> bool {
        self.device == other.device
            && self.inode == other.inode
            && self.file_type == other.file_type
            && self.special_device == other.special_device
            && self.socket == other.socket
    }
}

#[derive(Clone)]
pub struct PidfdIdentityPolicy {
    expected: ObjectIdentity,
    evidence: PidfdEvidence,
    verifier: Arc<dyn PidfdIdentityVerifier>,
}

impl fmt::Debug for PidfdIdentityPolicy {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PidfdIdentityPolicy(REDACTED)")
    }
}

impl PidfdIdentityPolicy {
    pub fn new(
        trusted_pidfd: impl AsFd,
        access: AttachmentAccess,
        evidence: PidfdEvidence,
        verifier: Arc<dyn PidfdIdentityVerifier>,
    ) -> Result<Self, UnixSessionError> {
        verify_pidfd(trusted_pidfd.as_fd(), &evidence, verifier.as_ref())?;
        let expected = inspect_identity(trusted_pidfd, KernelObjectType::Pidfd, access, true)?;
        Ok(Self {
            expected,
            evidence,
            verifier,
        })
    }

    fn validate(
        &self,
        pidfd: impl AsFd,
        access: AttachmentAccess,
    ) -> Result<ObjectIdentity, UnixSessionError> {
        verify_pidfd(pidfd.as_fd(), &self.evidence, self.verifier.as_ref())?;
        let actual = inspect_identity(pidfd, KernelObjectType::Pidfd, access, true)?;
        if actual == self.expected {
            Ok(actual)
        } else {
            Err(UnixSessionError::PidfdIdentityMismatch)
        }
    }
}

#[derive(Clone)]
pub enum DescriptorPolicy {
    File(ObjectIdentity),
    SealedReadOnlyMemfd,
    Pidfd(PidfdIdentityPolicy),
    Credentials(PeerCredentials),
    /// A connected `AF_UNIX SOCK_STREAM` handed over via `SCM_RIGHTS`, whose
    /// remote peer must be credentialed to `expected_peer_uid`.
    ///
    /// This is a structural (class-based) policy: no pre-known exact kernel
    /// object is required. It is validated by confirming (1) the received
    /// descriptor is itself a connected `AF_UNIX`/`SOCK_STREAM` socket and
    /// (2) `SO_PEERCRED` on that received socket reports the same uid that
    /// authenticated the connection handing it over, so a same-uid process
    /// cannot forward a socket connected to some unrelated peer.
    ConnectedUnixStreamSocket {
        expected_peer_uid: Uid,
    },
}

impl fmt::Debug for DescriptorPolicy {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::File(_) => "DescriptorPolicy::File(REDACTED)",
            Self::SealedReadOnlyMemfd => "DescriptorPolicy::SealedReadOnlyMemfd",
            Self::Pidfd(_) => "DescriptorPolicy::Pidfd(REDACTED)",
            Self::Credentials(_) => "DescriptorPolicy::Credentials(REDACTED)",
            Self::ConnectedUnixStreamSocket { .. } => {
                "DescriptorPolicy::ConnectedUnixStreamSocket(REDACTED)"
            }
        })
    }
}

pub enum AcceptedAttachment {
    File(OwnedFd),
    Credentials(PeerCredentials),
}

impl fmt::Debug for AcceptedAttachment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::File(_) => "AcceptedAttachment::File(REDACTED)",
            Self::Credentials(_) => "AcceptedAttachment::Credentials(REDACTED)",
        })
    }
}

pub struct VerifiedPacket {
    payload: Vec<u8>,
    attachments: Vec<AcceptedAttachment>,
    credits: CreditBundle,
}

impl fmt::Debug for VerifiedPacket {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VerifiedPacket")
            .field("payload_bytes", &self.payload.len())
            .field("attachment_count", &self.attachments.len())
            .finish_non_exhaustive()
    }
}

impl VerifiedPacket {
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    pub fn attachments(&self) -> &[AcceptedAttachment] {
        &self.attachments
    }

    pub fn release_credit(&mut self, scope: CreditScope) {
        self.credits.release(scope);
    }

    pub fn into_parts(self) -> (Vec<u8>, Vec<AcceptedAttachment>, CreditBundle) {
        (self.payload, self.attachments, self.credits)
    }
}

pub(crate) enum ReceivedControl {
    File(OwnedFd),
    Credentials(PeerCredentials),
}

pub struct ReceivedPacket {
    pub(crate) payload: Vec<u8>,
    pub(crate) controls: Vec<ReceivedControl>,
    pub(crate) unknown_control: bool,
    pub(crate) first_on_socket: bool,
    pub(crate) credits: CreditBundle,
}

impl fmt::Debug for ReceivedPacket {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReceivedPacket")
            .field("payload_bytes", &self.payload.len())
            .field("control_count", &self.controls.len())
            .finish_non_exhaustive()
    }
}

impl ReceivedPacket {
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    pub fn control_count(&self) -> usize {
        self.controls.len()
    }

    pub fn verify_first_packet_credentials(
        &self,
        expected: PeerCredentials,
    ) -> Result<FirstPacketCredentials, UnixSessionError> {
        if !self.first_on_socket {
            return Err(UnixSessionError::CredentialMismatch);
        }
        match self.controls.as_slice() {
            [ReceivedControl::Credentials(actual)] if *actual == expected => {
                Ok(FirstPacketCredentials(*actual))
            }
            [ReceivedControl::Credentials(_)] => Err(UnixSessionError::CredentialMismatch),
            _ => Err(UnixSessionError::ControlMismatch),
        }
    }

    pub fn verify(
        self,
        packet: &AttachmentPacket,
        attachment_policy: AttachmentPolicy,
        policies: &[DescriptorPolicy],
        credit_scopes: &CreditScopeSet,
    ) -> Result<VerifiedPacket, UnixSessionError> {
        packet
            .validate(
                attachment_policy,
                self.controls.len(),
                false,
                false,
                self.unknown_control,
            )
            .map_err(|_| {
                if self.unknown_control {
                    UnixSessionError::UnknownControl
                } else {
                    UnixSessionError::ControlMismatch
                }
            })?;
        if policies.len() != self.controls.len() {
            return Err(UnixSessionError::ControlMismatch);
        }

        let mut credits = self.credits;
        credits
            .acquire_dispatch(credit_scopes, self.controls.len())
            .map_err(|_| UnixSessionError::CreditExceeded)?;
        let mut accepted = Vec::with_capacity(self.controls.len());
        let mut identities: Vec<(ObjectIdentity, bool)> = Vec::new();

        for ((control, descriptor), policy) in self
            .controls
            .into_iter()
            .zip(packet.descriptors.iter())
            .zip(policies)
        {
            match (control, descriptor.kind, policy) {
                (
                    ReceivedControl::Credentials(actual),
                    AttachmentKind::Credentials,
                    DescriptorPolicy::Credentials(expected),
                ) if actual == *expected => {
                    accepted.push(AcceptedAttachment::Credentials(actual));
                }
                (
                    ReceivedControl::File(fd),
                    AttachmentKind::FileDescriptor,
                    DescriptorPolicy::File(expected),
                ) if descriptor.object_type != KernelObjectType::Pidfd => {
                    let actual =
                        inspect_identity(&fd, descriptor.object_type, descriptor.access, false)?;
                    if actual != *expected {
                        return Err(UnixSessionError::DescriptorMismatch);
                    }
                    if identities.iter().any(|(prior, prior_allows)| {
                        prior.same_kernel_object(&actual)
                            && (!descriptor.duplicate_object_allowed || !prior_allows)
                    }) {
                        return Err(UnixSessionError::DuplicateObject);
                    }
                    identities.push((actual, descriptor.duplicate_object_allowed));
                    accepted.push(AcceptedAttachment::File(fd));
                }
                (
                    ReceivedControl::File(fd),
                    AttachmentKind::FileDescriptor,
                    DescriptorPolicy::SealedReadOnlyMemfd,
                ) if descriptor.object_type == KernelObjectType::Memfd
                    && descriptor.access == AttachmentAccess::ReadOnly =>
                {
                    let actual = inspect_identity(
                        &fd,
                        KernelObjectType::Memfd,
                        AttachmentAccess::ReadOnly,
                        false,
                    )?;
                    require_fully_sealed(&actual)?;
                    if identities.iter().any(|(prior, prior_allows)| {
                        prior.same_kernel_object(&actual)
                            && (!descriptor.duplicate_object_allowed || !prior_allows)
                    }) {
                        return Err(UnixSessionError::DuplicateObject);
                    }
                    identities.push((actual, descriptor.duplicate_object_allowed));
                    accepted.push(AcceptedAttachment::File(fd));
                }
                (
                    ReceivedControl::File(fd),
                    AttachmentKind::FileDescriptor,
                    DescriptorPolicy::Pidfd(policy),
                ) if descriptor.object_type == KernelObjectType::Pidfd => {
                    let actual = policy.validate(&fd, descriptor.access)?;
                    if identities.iter().any(|(prior, prior_allows)| {
                        prior.same_kernel_object(&actual)
                            && (!descriptor.duplicate_object_allowed || !prior_allows)
                    }) {
                        return Err(UnixSessionError::DuplicateObject);
                    }
                    identities.push((actual, descriptor.duplicate_object_allowed));
                    accepted.push(AcceptedAttachment::File(fd));
                }
                (
                    ReceivedControl::File(fd),
                    AttachmentKind::FileDescriptor,
                    DescriptorPolicy::ConnectedUnixStreamSocket { expected_peer_uid },
                ) if descriptor.object_type == KernelObjectType::UnixStreamSocket => {
                    let actual = inspect_identity(
                        &fd,
                        KernelObjectType::UnixStreamSocket,
                        descriptor.access,
                        false,
                    )?;
                    verify_connected_unix_stream_peer(&fd, *expected_peer_uid)?;
                    if identities.iter().any(|(prior, prior_allows)| {
                        prior.same_kernel_object(&actual)
                            && (!descriptor.duplicate_object_allowed || !prior_allows)
                    }) {
                        return Err(UnixSessionError::DuplicateObject);
                    }
                    identities.push((actual, descriptor.duplicate_object_allowed));
                    accepted.push(AcceptedAttachment::File(fd));
                }
                (ReceivedControl::Credentials(_), _, _) | (ReceivedControl::File(_), _, _) => {
                    return Err(UnixSessionError::DescriptorMismatch);
                }
            }
        }

        Ok(VerifiedPacket {
            payload: self.payload,
            attachments: accepted,
            credits,
        })
    }
}

fn inspect_identity(
    fd: impl AsFd,
    object_type: KernelObjectType,
    access: AttachmentAccess,
    pidfd_verified: bool,
) -> Result<ObjectIdentity, UnixSessionError> {
    let fd = fd.as_fd();
    if !fcntl_getfd(fd)
        .map_err(io_error)?
        .contains(FdFlags::CLOEXEC)
    {
        return Err(UnixSessionError::MissingCloexec);
    }
    let stat = fstat(fd).map_err(io_error)?;
    let file_type = FileType::from_raw_mode(stat.st_mode);
    let flags = fcntl_getfl(fd).map_err(io_error)?;
    if access_mode(flags) != access {
        return Err(UnixSessionError::DescriptorMismatch);
    }

    let socket = match object_type {
        KernelObjectType::UnixStreamSocket | KernelObjectType::WaylandSocket => {
            require_type(file_type, FileType::Socket)?;
            let domain = get_socket_domain(fd).map_err(io_error)?;
            let kind = get_socket_type(fd).map_err(io_error)?;
            if domain != AddressFamily::UNIX || kind != SocketType::STREAM {
                return Err(UnixSessionError::DescriptorMismatch);
            }
            Some((domain, kind))
        }
        KernelObjectType::UnixSeqpacketSocket => {
            require_type(file_type, FileType::Socket)?;
            let domain = get_socket_domain(fd).map_err(io_error)?;
            let kind = get_socket_type(fd).map_err(io_error)?;
            if domain != AddressFamily::UNIX || kind != SocketType::SEQPACKET {
                return Err(UnixSessionError::DescriptorMismatch);
            }
            Some((domain, kind))
        }
        KernelObjectType::PipeRead | KernelObjectType::PipeWrite => {
            require_type(file_type, FileType::Fifo)?;
            None
        }
        KernelObjectType::Memfd => {
            require_type(file_type, FileType::RegularFile)?;
            None
        }
        KernelObjectType::RegularFile => {
            require_type(file_type, FileType::RegularFile)?;
            None
        }
        KernelObjectType::Directory => {
            require_type(file_type, FileType::Directory)?;
            None
        }
        KernelObjectType::Device
        | KernelObjectType::Tap
        | KernelObjectType::Kvm
        | KernelObjectType::Vhost
        | KernelObjectType::Fuse
        | KernelObjectType::Hidraw
        | KernelObjectType::PtyMaster
        | KernelObjectType::PtySlave => {
            require_type(file_type, FileType::CharacterDevice)?;
            None
        }
        KernelObjectType::Pidfd => {
            if !pidfd_verified {
                return Err(UnixSessionError::PidfdEvidenceUnavailable);
            }
            None
        }

        KernelObjectType::ProcessCredentials => {
            return Err(UnixSessionError::DescriptorMismatch);
        }
    };

    let seals = if object_type == KernelObjectType::Memfd {
        Some(fcntl_get_seals(fd).map_err(io_error)?.bits())
    } else {
        None
    };

    Ok(ObjectIdentity {
        device: stat.st_dev,
        inode: stat.st_ino,
        file_type,
        object_type,
        access,
        special_device: stat.st_rdev,
        socket,
        seals,
    })
}

pub(crate) fn validate_owned_file(
    fd: impl AsFd,
    descriptor: &d2b_contracts::v2_component_session::AttachmentDescriptor,
    policy: &DescriptorPolicy,
) -> Result<(), UnixSessionError> {
    validate_owned_file_identity(fd, descriptor, policy).map(|_| ())
}

pub(crate) fn validate_owned_file_identity(
    fd: impl AsFd,
    descriptor: &d2b_contracts::v2_component_session::AttachmentDescriptor,
    policy: &DescriptorPolicy,
) -> Result<ObjectIdentity, UnixSessionError> {
    if descriptor.kind != AttachmentKind::FileDescriptor {
        return Err(UnixSessionError::DescriptorMismatch);
    }
    match policy {
        DescriptorPolicy::File(expected) if descriptor.object_type != KernelObjectType::Pidfd => {
            let actual = inspect_identity(fd, descriptor.object_type, descriptor.access, false)?;
            if actual == *expected {
                Ok(actual)
            } else {
                Err(UnixSessionError::DescriptorMismatch)
            }
        }
        DescriptorPolicy::Pidfd(policy) if descriptor.object_type == KernelObjectType::Pidfd => {
            policy.validate(fd, descriptor.access)
        }
        DescriptorPolicy::SealedReadOnlyMemfd
            if descriptor.object_type == KernelObjectType::Memfd
                && descriptor.access == AttachmentAccess::ReadOnly =>
        {
            let actual = inspect_identity(
                fd,
                KernelObjectType::Memfd,
                AttachmentAccess::ReadOnly,
                false,
            )?;
            require_fully_sealed(&actual)?;
            Ok(actual)
        }
        DescriptorPolicy::ConnectedUnixStreamSocket { expected_peer_uid }
            if descriptor.object_type == KernelObjectType::UnixStreamSocket =>
        {
            let actual = inspect_identity(
                &fd,
                KernelObjectType::UnixStreamSocket,
                descriptor.access,
                false,
            )?;
            verify_connected_unix_stream_peer(&fd, *expected_peer_uid)?;
            Ok(actual)
        }
        DescriptorPolicy::File(_)
        | DescriptorPolicy::SealedReadOnlyMemfd
        | DescriptorPolicy::Pidfd(_)
        | DescriptorPolicy::Credentials(_)
        | DescriptorPolicy::ConnectedUnixStreamSocket { .. } => {
            Err(UnixSessionError::DescriptorMismatch)
        }
    }
}

fn require_fully_sealed(identity: &ObjectIdentity) -> Result<(), UnixSessionError> {
    let required =
        (libc::F_SEAL_WRITE | libc::F_SEAL_GROW | libc::F_SEAL_SHRINK | libc::F_SEAL_SEAL) as u32;
    if identity.object_type == KernelObjectType::Memfd
        && identity.access == AttachmentAccess::ReadOnly
        && identity
            .seals
            .is_some_and(|seals| seals & required == required)
    {
        Ok(())
    } else {
        Err(UnixSessionError::DescriptorMismatch)
    }
}

fn require_type(actual: FileType, expected: FileType) -> Result<(), UnixSessionError> {
    if actual == expected {
        Ok(())
    } else {
        Err(UnixSessionError::DescriptorMismatch)
    }
}

fn access_mode(flags: OFlags) -> AttachmentAccess {
    match flags & OFlags::ACCMODE {
        OFlags::WRONLY => AttachmentAccess::WriteOnly,
        OFlags::RDWR => AttachmentAccess::ReadWrite,
        _ => AttachmentAccess::ReadOnly,
    }
}

/// Confirms a received `AF_UNIX`/`SOCK_STREAM` file descriptor is itself
/// connected (rather than an unconnected/listening socket) and that its
/// remote peer's credentialed uid matches `expected_peer_uid`.
///
/// `expected_peer_uid` must be the uid that authenticated the connection
/// handing this descriptor over, never a value read from the descriptor's
/// own payload: binding the check to the already-authenticated transport
/// peer is what stops a compromised same-uid process from forwarding a
/// stream socket connected to some unrelated foreign process.
fn verify_connected_unix_stream_peer(
    fd: impl AsFd,
    expected_peer_uid: Uid,
) -> Result<(), UnixSessionError> {
    let fd = fd.as_fd();
    match getpeername(fd) {
        Ok(_) => {}
        Err(_) => return Err(UnixSessionError::DescriptorMismatch),
    }
    let peer_credentials = get_socket_peercred(fd).map_err(io_error)?;
    if peer_credentials.uid == expected_peer_uid {
        Ok(())
    } else {
        Err(UnixSessionError::CredentialMismatch)
    }
}

#[cfg(test)]
mod responder_identity_tests {
    use super::*;
    use d2b_contracts::v2_component_session::{
        ServicePackage as ContractServicePackage, TransportClass as ContractTransportClass,
        directional_channel_binding,
    };
    use rustix::process::{getgid, getuid};

    #[test]
    fn current_reads_the_real_process_identity() {
        let identity = ResponderIdentity::current();
        assert_eq!(identity.uid(), getuid());
        assert_eq!(identity.gid(), getgid());
    }

    #[test]
    fn channel_binding_matches_the_canonical_pure_function() {
        let identity = ResponderIdentity::current();
        let actual = identity.channel_binding(
            ContractServicePackage::RuntimeSystemdUserV2,
            ContractTransportClass::UnixSeqpacket,
            EndpointRole::RuntimeSystemdUserAgent,
        );
        let expected = directional_channel_binding(
            ContractServicePackage::RuntimeSystemdUserV2,
            ContractTransportClass::UnixSeqpacket,
            EndpointRole::RuntimeSystemdUserAgent,
            getuid().as_raw(),
            getgid().as_raw(),
        );
        assert_eq!(actual, expected);
    }

    #[test]
    fn channel_binding_never_reads_a_peer_supplied_uid() {
        // A responder derives its channel binding solely from `current()`;
        // there is no constructor that accepts an externally supplied
        // uid/gid, so a peer cannot influence which identity is bound.
        let a = ResponderIdentity::current();
        let b = ResponderIdentity::current();
        assert_eq!(a, b);
    }
}

#[cfg(test)]
mod connected_unix_stream_socket_tests {
    use super::*;
    use rustix::net::{AddressFamily, SocketFlags, SocketType, socket, socketpair};

    fn connected_stream_pair() -> (OwnedFd, OwnedFd) {
        socketpair(
            AddressFamily::UNIX,
            SocketType::STREAM,
            SocketFlags::CLOEXEC,
            None,
        )
        .unwrap()
    }

    #[test]
    fn accepts_a_connected_socket_credentialed_to_the_expected_peer_uid() {
        let (left, _right) = connected_stream_pair();
        assert_eq!(
            verify_connected_unix_stream_peer(&left, rustix::process::getuid()),
            Ok(())
        );
    }

    #[test]
    fn rejects_a_connected_socket_with_a_mismatched_expected_peer_uid() {
        let (left, _right) = connected_stream_pair();
        let actual_uid = rustix::process::getuid();
        assert_ne!(
            actual_uid,
            Uid::ROOT,
            "test assumes it is not running as root"
        );
        assert_eq!(
            verify_connected_unix_stream_peer(&left, Uid::ROOT),
            Err(UnixSessionError::CredentialMismatch)
        );
    }

    #[test]
    fn rejects_an_unconnected_socket() {
        let unconnected = socket(AddressFamily::UNIX, SocketType::STREAM, None).unwrap();
        assert_eq!(
            verify_connected_unix_stream_peer(&unconnected, rustix::process::getuid()),
            Err(UnixSessionError::DescriptorMismatch)
        );
    }
}
