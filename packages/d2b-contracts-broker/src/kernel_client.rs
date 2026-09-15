//! The shared kernel-invocation client (U10).
//!
//! The U10 sandwich serves each process-family operation's privileged,
//! resource-agnostic kernel in-broker as a broker-generic committed row
//! while the family operation itself stays forwarded to the declaring
//! process. The daemon-side family handler validates the typed request
//! against its own resolver and invokes the kernel as a nested envelope
//! call; the daemon's own legacy sites invoke the kernels directly. Both
//! legs dial the broker's origination socket with one `EnvelopeInvoke`
//! frame, so the dialing machinery is shared here rather than duplicated
//! per caller crate.
//!
//! The frame is the envelope's generic invocation surface: the operation
//! names a committed row exactly as the catalog declares it, the payload
//! is the canonical object the envelope validates against the row's
//! declared shape, and a nested call carries the evidence chain (root
//! invocation id plus ordered identities) it was dispatched under, so the
//! graft rule authorizes the call against the chain's initiating principal
//! and the in-broker leg records the correlation leg (KTD6). An
//! `EnvelopeInvoke` frame carries no audit join: the envelope's own
//! invocation-id audit replaces the typed join, exactly as the broker's
//! admission reads it.

use std::io::IoSlice;
use std::os::fd::OwnedFd;
use std::path::Path;
use std::time::Duration;

use rustix::event::{PollFd, PollFlags, poll};
use rustix::net::{
    AddressFamily, RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags, SendAncillaryBuffer,
    SendAncillaryMessage, SendFlags, SocketFlags, SocketType, recvmsg, send, sendmsg, socket_with,
};
use socket2::Socket;

use crate::broker_wire::{
    BrokerCallerRole, BrokerRequest, BrokerRequestEnvelope, BrokerResponse, EnvelopeInvokeRequest,
    EnvelopeInvokeResponse, FdKind,
};

/// The failure of one kernel invocation, in the vocabulary the daemon's
/// other broker clients use for the same transport.
#[derive(Debug)]
pub enum KernelInvokeError {
    /// The transport step (dial, frame write, frame read, decode) failed.
    Transport(String),
    /// The broker answered a non-envelope response.
    Protocol(String),
    /// The broker refused the invocation with the envelope's closed code.
    Refused { code: String, detail: Option<String> },
}

impl std::fmt::Display for KernelInvokeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Transport(detail) => write!(formatter, "kernel transport: {detail}"),
            Self::Protocol(detail) => write!(formatter, "kernel protocol: {detail}"),
            Self::Refused { code, detail } => match detail {
                Some(detail) => write!(formatter, "kernel refused: {code} ({detail})"),
                None => write!(formatter, "kernel refused: {code}"),
            },
        }
    }
}

impl std::error::Error for KernelInvokeError {}

/// One kernel invocation: the committed kernel row name, the Zone it runs
/// in, the canonical payload, the descriptors to attach, and the evidence
/// chain a nested call presents.
#[derive(Debug, Clone)]
pub struct KernelInvocation<'a> {
    /// The committed kernel row name, exactly as the catalog declares it.
    pub operation: &'a str,
    /// The Zone the invocation runs in.
    pub zone: &'a str,
    /// The canonical payload object the envelope validates against the
    /// row's declared shape.
    pub payload: serde_json::Value,
    /// The SCM_RIGHTS request attachments, in frame order.
    pub fds: &'a [OwnedFd],
    /// The root invocation id of the evidence chain a nested call
    /// presents; absent for a root call.
    pub chain_root_invocation_id: Option<&'a str>,
    /// The ordered chain identities, root first; present exactly when
    /// [`Self::chain_root_invocation_id`] is, and never empty then.
    pub chain_identities: Option<&'a [String]>,
}

/// The reply to one kernel invocation: the envelope response plus the
/// descriptors the answering kernel minted, in frame order.
#[derive(Debug)]
pub struct KernelReply {
    /// The envelope's reply, carrying the invocation id, the canonical
    /// result, or the refusal code.
    pub response: EnvelopeInvokeResponse,
    /// The response frame's SCM_RIGHTS attachments this result owns, by
    /// index (the response's `fd_indexes` name positions in this vector).
    pub fds: Vec<OwnedFd>,
}

/// Invoke one committed kernel row over the broker's origination socket.
///
/// The frame is the generic `EnvelopeInvoke` carrier: the operation names
/// the committed kernel row, the payload is the canonical object the
/// envelope validates, and a nested call presents the evidence chain it was
/// dispatched under. The reply carries the invocation id the audit record
/// keys on and the descriptors the kernel minted; a refusal keeps its
/// closed code and detail.
pub fn envelope_invoke_kernel(
    socket_path: &Path,
    io_timeout: Duration,
    caller_role: BrokerCallerRole,
    invocation: KernelInvocation<'_>,
) -> Result<KernelReply, KernelInvokeError> {
    let fd = socket_with(
        AddressFamily::UNIX,
        SocketType::SEQPACKET,
        SocketFlags::CLOEXEC,
        None,
    )
    .map_err(|error| KernelInvokeError::Transport(format!("socket: {error}")))?;
    let socket = Socket::from(fd);
    let address = socket2::SockAddr::unix(socket_path)
        .map_err(|error| KernelInvokeError::Transport(format!("address: {error}")))?;
    socket
        .connect_timeout(&address, io_timeout)
        .map_err(|error| KernelInvokeError::Transport(format!("connect: {error}")))?;
    socket
        .set_read_timeout(Some(io_timeout))
        .map_err(|error| KernelInvokeError::Transport(format!("read timeout: {error}")))?;
    socket
        .set_write_timeout(Some(io_timeout))
        .map_err(|error| KernelInvokeError::Transport(format!("write timeout: {error}")))?;
    let request = BrokerRequest::EnvelopeInvoke(EnvelopeInvokeRequest {
        operation: invocation.operation.to_owned(),
        zone: invocation.zone.to_owned(),
        payload: invocation.payload,
        chain_root_invocation_id: invocation.chain_root_invocation_id.map(str::to_owned),
        chain_identities: invocation
            .chain_identities
            .map(|identities| identities.to_vec()),
        fd_indexes: (0..invocation.fds.len() as u32).collect(),
        // The kernel rows declare `Any` for their pidfd legs; the
        // receiving envelope re-checks the actual kernel kinds against the
        // row's declared facet, so the shared client declares the
        // permissive kind and never mislabels an anon-inode descriptor.
        fd_kinds: vec![FdKind::Any; invocation.fds.len()],
    });
    let envelope = BrokerRequestEnvelope {
        request,
        caller_role,
        test_peer_uid: None,
        audit_join: None,
    };
    let frame = d2b_contracts::encode_frame(&envelope)
        .map_err(|error| KernelInvokeError::Transport(format!("encode: {error}")))?;
    let written = if invocation.fds.is_empty() {
        send(&socket, &frame, SendFlags::empty())
            .map_err(|error| KernelInvokeError::Transport(format!("send: {error}")))?
    } else {
        let descriptors = invocation
            .fds
            .iter()
            .map(std::os::fd::AsFd::as_fd)
            .collect::<Vec<_>>();
        let mut control_bytes = vec![0_u8; rustix::cmsg_space!(ScmRights(256))];
        let mut control = SendAncillaryBuffer::new(&mut control_bytes);
        if !control.push(SendAncillaryMessage::ScmRights(&descriptors)) {
            return Err(KernelInvokeError::Transport(
                "send: descriptor control data rejected".to_owned(),
            ));
        }
        let iov = [IoSlice::new(&frame)];
        sendmsg(&socket, &iov, &mut control, SendFlags::empty())
            .map_err(|error| KernelInvokeError::Transport(format!("sendmsg: {error}")))?
    };
    if written != frame.len() {
        return Err(KernelInvokeError::Transport("short write".to_owned()));
    }
    // The reply must arrive within the io timeout: poll for readability so
    // a silent peer cannot hold the caller past its budget.
    let mut fds = [PollFd::new(&socket, PollFlags::IN | PollFlags::ERR | PollFlags::HUP)];
    match poll(&mut fds, io_timeout.as_millis().min(i32::MAX as u128) as i32) {
        Ok(0) => {
            return Err(KernelInvokeError::Transport("reply timeout".to_owned()));
        }
        Err(error) => {
            return Err(KernelInvokeError::Transport(format!("reply poll: {error}")));
        }
        Ok(_) => {}
    }
    let mut payload = vec![0_u8; d2b_contracts::MAX_FRAME_SIZE + 4];
    let mut iov = [std::io::IoSliceMut::new(&mut payload)];
    let mut control_bytes = vec![0_u8; rustix::cmsg_space!(ScmRights(256))];
    let mut control = RecvAncillaryBuffer::new(&mut control_bytes);
    let message = recvmsg(&socket, &mut iov, &mut control, RecvFlags::CMSG_CLOEXEC)
        .map_err(|error| KernelInvokeError::Transport(format!("recvmsg: {error}")))?;
    let bytes = message.bytes;
    let mut received = Vec::new();
    for message in control.drain() {
        if let RecvAncillaryMessage::ScmRights(descriptors) = message {
            for owned in descriptors {
                received.push(owned);
            }
        }
    }
    let response: BrokerResponse = d2b_contracts::decode_frame("BrokerResponse", &payload[..bytes])
        .map_err(|error| KernelInvokeError::Transport(format!("decode: {error}")))?;
    let BrokerResponse::EnvelopeInvoke(response) = response else {
        return Err(KernelInvokeError::Protocol(format!(
            "unexpected response kind: {response:?}"
        )));
    };
    if response.refusal.is_some() {
        return Err(KernelInvokeError::Refused {
            code: response.refusal.clone().unwrap_or_default(),
            detail: response.detail.clone(),
        });
    }
    Ok(KernelReply {
        response,
        fds: received,
    })
}