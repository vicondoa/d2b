//! The forwarding rendezvous: the endpoint the broker dials.
//!
//! The broker is a seqpacket listener and links no provider crate, so a
//! committed operation's handler cannot run there. The dispatch step of its
//! envelope forwards instead: one validated, authorized invocation crosses to
//! the process that declares the handler, which answers with the handler's
//! canonical result or its own refusal code. This module is the receiving
//! end - a seqpacket listener the daemon owns, beside the broker socket - and
//! the routing it performs: the call names its Zone and its operation, and
//! the Zone's started providers resolve it to the provider that declared the
//! operation, whose toolkit envelope runs the handler.
//!
//! Two properties are structural. The listener binds only when the
//! environment names a path, so a daemon with no forwarding peer is
//! fail-closed exactly as the broker is, and the broker's own no-default
//! stance keeps the two ends in agreement. And resolution runs over the
//! *started* providers' own descriptor tables - the handler table the plane
//! registered - so an operation no started provider declares is refused by
//! name before any handler runs.
//!
//! The forwarded hop was authorized at the broker against the committed rows;
//! the carrier deliberately carries no caller identity, because a second
//! identity on this side would be a second authority to keep in sync. The
//! one identity this endpoint does check is the transport peer's: nothing in
//! the frame binds the call to the authorization the broker performed, so
//! `SO_PEERCRED` must name the broker before a single frame is read (see
//! [`ServingPosture`]). The provider-side envelope then runs each declared
//! operation under the declaring provider's own reference, which is the one
//! caller fact this process owns: a provider may run the handlers it
//! declared, and the envelope refuses every caller it holds no grant for.
//!
//! The endpoint serves on the daemon's runtime rather than on a thread per
//! call: the listener and every accepted connection are registered with the
//! reactor (`tokio::io::unix::AsyncFd`, the pattern the session crate drives
//! its own seqpacket endpoints with), the request frame and the reply frame
//! are awaited rather than blocked, and each call runs as a task under an
//! async admission bound. The in-flight cap therefore bounds live calls
//! rather than pinned threads, and a handler that never finishes is refused
//! by name at its deadline instead of holding a slot forever.

use std::collections::BTreeMap;
use std::io;
use std::os::fd::{AsRawFd, RawFd};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use d2b_contracts_broker::FORWARD_SOCKET_ENV;
use d2b_contracts_broker::broker_wire::{
    FD_LEG, FdKind, MAX_FRAME_FDS, ForwardOperationOutcome, ForwardOperationRequest,
    ForwardOperationResponse,
};
use d2b_contracts_resource::v3::CanonicalJsonObject;
use d2b_provider_toolkit::operations::{UNCOMMITTED_OPERATION, UNGRANTED_CALLER};
use d2bd_runtime::concurrency::DEFAULT_MAX_INFLIGHT_CONNECTIONS;
use d2bd_runtime::runtime_process::{RuntimeIdentity, bind_public_socket};
use d2bd_runtime::typed_error::TypedError;
use d2bd_runtime::unix_transport::{close_received_fds, read_frame_with_fds, write_frame_with_fds};
use d2bd_runtime::wire::MAX_FRAME_SIZE;
use nix::sys::socket::{MsgFlags, getsockopt, recv, send, sockopt};
use socket2::Socket;
use tokio::io::Interest;
use tokio::io::unix::AsyncFd;
use tokio::sync::Semaphore;

use crate::provider_lifecycle::ProviderRuntime;

/// The refusal code for a forwarded payload this endpoint cannot read as the
/// canonical object the broker validated.
pub(crate) const INVALID_PAYLOAD: &str = "invalid-payload";

/// The refusal code for a forwarded handler that did not finish within its
/// deadline.
///
/// The name is this endpoint's own: the broker's round-trip budget is the
/// outer bound on the call, and this is the refusal a caller sees when the
/// handler inside it stalls past the daemon's own.
pub(crate) const FORWARD_TIMEOUT: &str = "forward-timeout";

/// The read deadline for one forwarded request frame: a connected peer that
/// sends nothing is closed rather than holding an in-flight slot.
const FORWARD_REQUEST_DEADLINE: Duration = Duration::from_secs(30);

/// The handler deadline for one forwarded invocation.
///
/// It sits below the broker's default forward round trip (30 s), so a stalled
/// handler is refused by name while the caller is still listening rather than
/// reported as a peer that never answered.
const FORWARD_HANDLER_DEADLINE: Duration = Duration::from_secs(25);

/// The write deadline for one reply frame: a peer that will not read must not
/// hold an in-flight slot open either.
const FORWARD_REPLY_DEADLINE: Duration = Duration::from_secs(5);

/// The backoff after a failed accept, so a listener that keeps refusing does
/// not spin the loop.
const ACCEPT_ERROR_BACKOFF: Duration = Duration::from_millis(10);

/// The drain deadline for a refusal written before the peer's own frame was
/// read. Closing a seqpacket socket with input still pending makes the kernel
/// send a reset, which the peer sees instead of the refusal, so the pending
/// frames are consumed first - exactly as the public socket's refusal path
/// drains for the same reason.
const FORWARD_REFUSAL_DRAIN_DEADLINE: Duration = Duration::from_millis(10);

/// The privileged broker's uid: both the host broker and a realm broker run
/// as root, because their unit does host-mutating work (`nixos-modules/
/// host-broker.nix` fixes `User = "root"`).
const BROKER_UID: u32 = 0;

/// The rendezvous socket the environment names, when it names one.
///
/// The variable is declared with the carrier in `d2b-contracts-broker`, so
/// the broker that dials and the daemon that binds cannot disagree on it.
pub(crate) fn configured_socket() -> Option<PathBuf> {
    std::env::var_os(FORWARD_SOCKET_ENV)
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
}

/// The started providers of every Zone, keyed by Zone label.
#[derive(Default)]
pub(crate) struct ForwardRendezvous {
    zones: Mutex<BTreeMap<String, Arc<ProviderRuntime>>>,
}

impl ForwardRendezvous {
    /// Assemble an empty rendezvous: no Zone has started providers yet.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Publish the providers one Zone started.
    ///
    /// A Zone whose plane re-opens republishes its new provider set; the last
    /// published set is the one that answers.
    pub(crate) fn publish(&self, zone: &str, providers: Arc<ProviderRuntime>) {
        self.zones
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(zone.to_owned(), providers);
    }

    /// Answer one forwarded invocation.
    ///
    /// A Zone with no started providers and an operation no started provider
    /// declares are the same refusal: nothing in this process serves the
    /// call, and the code the broker's own envelope uses for that state names
    /// it.
    pub(crate) async fn invoke(
        &self,
        request: &ForwardOperationRequest,
        fds: &[RawFd],
    ) -> ForwardOperationResponse {
        let providers = self
            .zones
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&request.zone)
            .map(Arc::clone);
        let Some(providers) = providers else {
            return refused(UNCOMMITTED_OPERATION);
        };
        let Some(provider) = providers.declaring_provider(&request.operation) else {
            return refused(UNCOMMITTED_OPERATION);
        };
        let Ok(bytes) = serde_json::to_vec(&request.payload) else {
            return refused(INVALID_PAYLOAD);
        };
        let Ok(payload) = CanonicalJsonObject::parse(&bytes) else {
            return refused(INVALID_PAYLOAD);
        };
        match provider
            .invoke(&request.operation, &request.invocation_id, payload, fds)
            .await
        {
            Ok(result) => ForwardOperationResponse {
                outcome: ForwardOperationOutcome::Result {
                    result: serde_json::to_value(result.object())
                        .expect("canonical JSON objects always serialize"),
                    fd_indexes: vec![],
                    fd_kinds: vec![],
                },
            },
            Err(failure) => refused(failure.code()),
        }
    }

    /// Answer one admitted connection: read one request frame, invoke the
    /// declared handler under the handler deadline, write one reply frame.
    ///
    /// Driven on the runtime: every wait here is awaited, so a call holds its
    /// admission permit and no thread of its own.
    async fn serve_connection(
        &self,
        connection: &AsyncSeqpacket,
        handler_deadline: Duration,
    ) -> Result<(), TypedError> {
        let (frame, request_fds) = connection
            .read_frame_with_fds(FORWARD_REQUEST_DEADLINE)
            .await?;
        // The frame's descriptors belong to this call:they are closed
        // whether the call is served or refused, after the reply frame has
        // gone (or when this connection errors out).
        let fds = ScmFds::new(request_fds);
        let request: ForwardOperationRequest =
            serde_json::from_slice(&frame).map_err(|error| TypedError::WireInvalidFrame {
                detail: format!(
                    "forwarded request frame is not a ForwardOperationRequest: {error}"
                ),
            })?;
        if !request_fds_admitted(&request, fds.as_slice()) {
            // The declared leg and the attached leg disagree - not in the
            // count, not in index order, not in kernel kind - so the call is
            // refused with the carrier's own fd-leg code rather than letting
            // an anonymous truncation pass as the invocation.
            let response = refused(FD_LEG);
            return connection
                .write_frame_with_fds(&encode_reply(&response)?, &[], FORWARD_REPLY_DEADLINE)
                .await;
        }
        let response = match tokio::time::timeout(
            handler_deadline,
            self.invoke(&request, fds.as_slice()),
        )
        .await
        {
            Ok(response) => response,
            Err(_) => {
                // A handler that never finished is the daemon's answer to
                // give: the call is refused by name while the caller is still
                // listening, and the slot it held is free the moment this
                // call returns.
                tracing::warn!(
                    operation = %request.operation,
                    "forwarded handler exceeded its deadline; refusing the call"
                );
                refused(FORWARD_TIMEOUT)
            }
        };
        // The response leg is JSON-only until a provider can mint descriptors
        // (a later unit's work), so the reply frame carries no attachments.
        connection
            .write_frame_with_fds(
                &encode_reply(&response)?,
                &[],
                FORWARD_REPLY_DEADLINE,
            )
            .await
    }
}

struct ScmFds(Vec<RawFd>);

impl ScmFds {

    fn new(fds: Vec<RawFd>) -> Self {
        Self(fds)
    }

    /// The received descriptors,borrowed across the invocation..
    fn as_slice(&self) -> &[RawFd] {
        &self.0
    }
}

impl Drop for ScmFds {

    fn drop(&mut self) {
        close_received_fds(&self.0);
    }
}

/// Whether one request's declared fd leg is admitted by the descriptors the
/// frame actually attached:count equal (never truncated), indexes in frame
/// order, kinds against the kernel stat of each received descriptor,and the
/// whole leg within the carrier's frame ceiling.

fn request_fds_admitted(request: &ForwardOperationRequest, fds: &[RawFd]) -> bool {
    if request.fd_indexes.len() != request.fd_kinds.len() {
        return false;
    }
    if request.fd_indexes.len() > MAX_FRAME_FDS {
        return false;
    }
    if request.fd_indexes
        .iter()
        .enumerate()
        .any(|(position, declared)| *declared != position as u32)
    {
        return false;
    }
    if fds.len() != request.fd_indexes.len() {
        return false;
    }
    fds
        .iter()
        .zip(&request.fd_kinds)
        .all(|(fd, declared)| fd_kind_of(*fd) == Some(*declared))
}

/// The kernel kind one descriptor presents,or None when its fstat reports
/// a kind the carrier vocabulary does not carry..
fn fd_kind_of(fd: RawFd) -> Option<FdKind> {
    let stat = nix::sys::stat::fstat(fd).ok()?;
    match stat.st_mode & nix::libc::S_IFMT {
        nix::libc::S_IFIFO => Some(FdKind::Fifo),
        nix::libc::S_IFSOCK => Some(FdKind::Socket),
        nix::libc::S_IFCHR => Some(FdKind::CharDevice),
        nix::libc::S_IFBLK => Some(FdKind::BlockDevice),
        nix::libc::S_IFREG => Some(FdKind::Regular),
        nix::libc::S_IFDIR => Some(FdKind::Directory),
        _ => None,
    }
}

/// One refused forwarded invocation, named.
fn refused(code: &str) -> ForwardOperationResponse {
    ForwardOperationResponse {
        outcome: ForwardOperationOutcome::Refused {
            code: code.to_owned(),
        },
    }
}

/// Bind the rendezvous listener at `path`.
///
/// The socket's DAC posture is the public socket's - a seqpacket listener
/// owned by the daemon, mode 0660, chgrp'd to the socket group - but DAC is
/// not this endpoint's admission: that group carries every launcher and
/// admin, and a member that reached this socket would drive forwarded
/// operations the broker never authorized. The accepted peer is decided
/// per connection by `SO_PEERCRED` instead ([`ServingPosture`]).
pub(crate) fn bind(path: &Path, identity: &RuntimeIdentity) -> Result<Socket, TypedError> {
    bind_public_socket(path, identity)
}

/// The serving posture of one rendezvous loop: which peer identity the
/// endpoint accepts, how many calls may be in flight, and how long a handler
/// may run.
///
/// The accepted identity is the whole authorization of this hop. The call was
/// authorized at the broker against the committed rows, and the carrier
/// carries no caller identity, so a peer that is not the broker is a peer
/// this endpoint has nothing to check the call against. Accepted: the
/// privileged broker (uid 0 - both the host broker and a realm broker run as
/// root) and the daemon's own effective uid, which is the broker's identity
/// when both run under one unprivileged user, the test and
/// unprivileged-development shape. Nothing else is. In particular membership
/// of the public socket group - the group the socket is chgrp'd to, which
/// carries every launcher and admin - is not admission: a member that reached
/// this socket would drive provider operations with no broker authorization
/// and no broker audit record.
struct ServingPosture {
    /// The effective uid a peer must present to be admitted, beside the
    /// privileged broker's. `SO_PEERCRED` reports the peer's effective
    /// credentials, and the daemon's real and effective uids coincide (it
    /// either starts as its own user or drops to it with `setuid(2)`).
    accepted_uid: u32,
    /// The in-flight ceiling.
    max_inflight: usize,
    /// The handler deadline.
    handler_deadline: Duration,
}

impl ServingPosture {
    /// The daemon's production posture.
    fn production() -> Self {
        Self {
            accepted_uid: nix::unistd::geteuid().as_raw(),
            max_inflight: DEFAULT_MAX_INFLIGHT_CONNECTIONS,
            handler_deadline: FORWARD_HANDLER_DEADLINE,
        }
    }

    /// Whether one peer identity is the peer this endpoint serves.
    fn admits(&self, peer_uid: u32) -> bool {
        peer_uid == BROKER_UID || peer_uid == self.accepted_uid
    }
}

/// Serve accepted forwarded connections until the daemon exits.
///
/// The accept loop and every call it admits run as tasks on `runtime`: a call
/// holds one semaphore permit and no thread of its own, so the in-flight cap
/// bounds live calls rather than pinned threads.
pub(crate) fn spawn_server(
    rendezvous: Arc<ForwardRendezvous>,
    listener: Socket,
    runtime: tokio::runtime::Handle,
) -> Result<(), TypedError> {
    let listener = AsyncSeqpacket::register(listener)?;
    runtime.spawn(serve_accepted(
        rendezvous,
        listener,
        ServingPosture::production(),
    ));
    Ok(())
}

/// The accept loop: one task per admitted call, all of them on the runtime.
async fn serve_accepted(
    rendezvous: Arc<ForwardRendezvous>,
    listener: AsyncSeqpacket,
    posture: ServingPosture,
) {
    let admissions = Arc::new(Semaphore::new(posture.max_inflight));
    loop {
        let connection = match listener.accept().await {
            Ok(connection) => connection,
            Err(error) => {
                tracing::warn!(error = %error, "forward rendezvous accept failed; continuing");
                tokio::time::sleep(ACCEPT_ERROR_BACKOFF).await;
                continue;
            }
        };
        let connection = match AsyncSeqpacket::register(connection) {
            Ok(connection) => connection,
            Err(error) => {
                tracing::warn!(
                    reason = %error.message(),
                    "forward rendezvous call refused"
                );
                continue;
            }
        };
        // Authz-first: the peer is bound to its kernel identity before a
        // single frame is read, so a peer that is not the broker can neither
        // occupy a slot nor drive a forwarded operation.
        let peer_uid = match connection.peer_uid() {
            Ok(peer_uid) => peer_uid,
            Err(error) => {
                tracing::warn!(
                    reason = %error.message(),
                    "forward rendezvous could not read its peer's credentials; refusing the call"
                );
                refuse(&connection, UNGRANTED_CALLER).await;
                continue;
            }
        };
        if !posture.admits(peer_uid) {
            tracing::warn!(
                peer_uid,
                "forward rendezvous refused a peer that is not the broker"
            );
            refuse(&connection, UNGRANTED_CALLER).await;
            continue;
        }
        let Ok(permit) = Arc::clone(&admissions).try_acquire_owned() else {
            // The cap is the admission gate, and the carrier carries an
            // answer: the call is refused under the daemon's own capacity
            // code, so the broker reports that code rather than a handler it
            // never reached.
            tracing::warn!("forward rendezvous is at its in-flight cap; refusing the call");
            refuse(&connection, TypedError::DaemonBusy.kind()).await;
            continue;
        };
        let rendezvous = Arc::clone(&rendezvous);
        tokio::spawn(async move {
            // The permit lives exactly as long as the call does, so the slot
            // is released by the call finishing - never by the accept loop.
            let _permit = permit;
            if let Err(error) = rendezvous
                .serve_connection(&connection, posture.handler_deadline)
                .await
            {
                tracing::warn!(
                    reason = %error.message(),
                    "forward rendezvous call refused"
                );
            }
        });
    }
}

/// Refuse one call this endpoint will not serve, by name.
///
/// The refusal is written before the peer's own frame is read - the peer may
/// not have sent one - and the pending input is drained, so the close that
/// follows is graceful and the refusal is what the caller receives.
async fn refuse(connection: &AsyncSeqpacket, code: &str) {
    match encode_reply(&refused(code)) {
        Ok(frame) => {
            if let Err(error) = connection.write_frame(&frame, FORWARD_REPLY_DEADLINE).await {
                tracing::warn!(
                    reason = %error.message(),
                    "forward rendezvous refusal not delivered"
                );
            }
        }
        Err(error) => {
            tracing::warn!(
                reason = %error.message(),
                "forward rendezvous refusal not encodable"
            );
        }
    }
    connection
        .drain_pending(FORWARD_REFUSAL_DRAIN_DEADLINE)
        .await;
}

/// One seqpacket endpoint registered with the reactor.
///
/// The listener is one of these, and so is every accepted connection: frame
/// reads and writes wait on readiness instead of on a blocking syscall, so
/// nothing about a call owns a thread. The session crate drives its own
/// seqpacket endpoints the same way, so the daemon has one pattern for kernel
/// I/O on an async path rather than a second one here.
struct AsyncSeqpacket {
    io: AsyncFd<Socket>,
}

impl AsyncSeqpacket {
    /// Register one socket with the reactor.
    ///
    /// The socket is switched to nonblocking mode first: the reactor owns
    /// readiness, and a blocking descriptor would stall the worker that
    /// awaited it.
    fn register(socket: Socket) -> Result<Self, TypedError> {
        socket
            .set_nonblocking(true)
            .map_err(|error| TypedError::InternalIo {
                context: "set forward rendezvous socket nonblocking".to_owned(),
                detail: error.to_string(),
            })?;
        let io = AsyncFd::new(socket).map_err(|error| TypedError::InternalIo {
            context: "register forward rendezvous socket".to_owned(),
            detail: error.to_string(),
        })?;
        Ok(Self { io })
    }

    /// The uid the peer presented when it connected.
    ///
    /// `SO_PEERCRED` is the kernel's answer - the peer is whatever the kernel
    /// says connected, not what a frame claims - which is why this endpoint
    /// reads it before the first frame.
    fn peer_uid(&self) -> Result<u32, TypedError> {
        getsockopt(self.io.get_ref(), sockopt::PeerCredentials)
            .map(|credentials| credentials.uid())
            .map_err(|error| TypedError::InternalIo {
                context: "read forward rendezvous peer credentials".to_owned(),
                detail: error.to_string(),
            })
    }

    /// Accept the next connection, waiting on the listener instead of polling
    /// it.
    async fn accept(&self) -> io::Result<Socket> {
        self.io
            .async_io(Interest::READABLE, |listener| {
                listener.accept().map(|(connection, _)| connection)
            })
            .await
    }

    /// Read one frame, waiting at most `deadline` for it to arrive.
    async fn read_frame(&self, deadline: Duration) -> Result<Vec<u8>, TypedError> {
        let mut datagram = vec![0u8; MAX_FRAME_SIZE + 5];
        let read = match tokio::time::timeout(deadline, self.recv_datagram(&mut datagram)).await {
            Ok(Ok(read)) => read,
            Ok(Err(error)) => return Err(recv_failure(error.to_string())),
            Err(_) => return Err(recv_failure(format!("no frame within {deadline:?}"))),
        };
        decode_frame(&datagram[..read])
    }

    /// Write one frame, waiting at most `deadline` for the peer to take it.
    async fn write_frame(&self, body: &[u8], deadline: Duration) -> Result<(), TypedError> {
        let frame = encode_frame(body)?;
        let written = match tokio::time::timeout(deadline, self.send_datagram(&frame)).await {
            Ok(Ok(written)) => written,
            Ok(Err(error)) => return Err(send_failure(error.to_string())),
            Err(_) => return Err(send_failure(format!("no write within {deadline:?}"))),
        };
        if written != frame.len() {
            return Err(send_failure(format!(
                "short write: {written} of {}",
                frame.len()
            )));
        }
        Ok(())
    }

    /// Read one frame and the descriptors its SCM_RIGHTS attachments carried,
    /// waiting at most `deadline` for it to arrive.
    ///
    /// A frame and its attachments arrive together or not at all,so the
    /// received descriptor count is exactly what the sender put on the
    /// carrier;an oversized cmsg set is capped by the kernel at the receive
    /// buffer's ceiling,which is why the caller-side declaration check
    /// refuses a count over that ceiling rather than let a truncation pass..
    async fn read_frame_with_fds(&self, deadline: Duration) -> Result<(Vec<u8>, Vec<RawFd>), TypedError> {
        // The blocking transport read the prefixed frame and stripped the
        // length prefix itself, so the returned body is already the frame
        // payload,length-checked and cmsg-truncation-checked.
        match tokio::time::timeout(deadline, self.recv_frame_with_fds()).await {
            Ok(Ok(pair)) => Ok(pair),
            Ok(Err(error)) => Err(recv_failure(error.to_string())),
            Err(_) => Err(recv_failure(format!("no frame within {deadline:?}"))),
        }
    }

    /// Write one frame,attaching `fds` to it,waiting at most `deadline`
    /// for the peer to take it.
    async fn write_frame_with_fds(
        &self,
        body: &[u8],
        fds: &[RawFd],
        deadline: Duration,
    ) -> Result<(), TypedError> {
        // The transport writes the length prefix itself,so the body crosses
        // as-is;the receiving transport strips the same prefix back off..
        match tokio::time::timeout(deadline, self.send_datagram_with_fds(body, fds)).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => Err(send_failure(error.to_string())),
            Err(_) => Err(send_failure(format!("no write within {deadline:?}"))),
        }
    }

    /// One datagram read with its attachments,awaited for readiness.

    /// The blocking transport's `recvmsg` owns the control-message buffer for
    /// this read,and MSG_CMSG_CLOEXEC is set there,so the received descriptors
    /// arrive close-on-exec exactly as they do on the broker leg.

    async fn recv_frame_with_fds(&self) -> io::Result<(Vec<u8>, Vec<RawFd>)> {
        self.io
            .async_io(Interest::READABLE, |socket| {
                read_frame_with_fds(socket)
                    .map_err(|error| io::Error::new(io::ErrorKind::Other, format!("{error:?}")))
            })
            .await
    }

    /// One datagram write with its attachments,awaited for readiness..
    async fn send_datagram_with_fds(&self, frame: &[u8], fds: &[RawFd]) -> io::Result<()> {
        self.io
            .async_io(Interest::WRITABLE, |socket| {
                write_frame_with_fds(socket, frame, fds)
                    .map(|()| ())
                    .map_err(|error| io::Error::new(io::ErrorKind::Other, format!("{error:?}")))
            })
            .await
    }

    /// One datagram read, awaited for readiness.
    async fn recv_datagram(&self, datagram: &mut [u8]) -> io::Result<usize> {
        self.io
            .async_io(Interest::READABLE, |socket| {
                recv(socket.as_raw_fd(), &mut datagram[..], MsgFlags::empty())
                    .map_err(|errno| io::Error::from_raw_os_error(errno as i32))
            })
            .await
    }

    /// One datagram write, awaited for readiness.
    async fn send_datagram(&self, frame: &[u8]) -> io::Result<usize> {
        self.io
            .async_io(Interest::WRITABLE, |socket| {
                send(socket.as_raw_fd(), frame, MsgFlags::empty())
                    .map_err(|errno| io::Error::from_raw_os_error(errno as i32))
            })
            .await
    }

    /// Consume what a refused peer already sent, so the close that follows is
    /// graceful and the refusal arrives. Bounded: the loop stops at the first
    /// error, which includes the drain deadline.
    async fn drain_pending(&self, deadline: Duration) {
        for _ in 0..4 {
            if self.read_frame(deadline).await.is_err() {
                return;
            }
        }
    }
}

/// The failure of a frame read, in the vocabulary the blocking transport uses
/// for the same syscall.
fn recv_failure(detail: String) -> TypedError {
    TypedError::InternalIo {
        context: "recv seqpacket frame".to_owned(),
        detail,
    }
}

/// The failure of a frame write, in the vocabulary the blocking transport
/// uses for the same syscall.
fn send_failure(detail: String) -> TypedError {
    TypedError::InternalIo {
        context: "send seqpacket frame".to_owned(),
        detail,
    }
}

/// Encode one reply frame body: the same serde spelling the blocking
/// transport wrote for this endpoint.
fn encode_reply(response: &ForwardOperationResponse) -> Result<Vec<u8>, TypedError> {
    serde_json::to_vec(response).map_err(|error| TypedError::InternalIo {
        context: "serialize JSON frame".to_owned(),
        detail: error.to_string(),
    })
}

/// Encode one frame: the four-byte little-endian length prefix the broker's
/// forwarder writes, unchanged.
fn encode_frame(body: &[u8]) -> Result<Vec<u8>, TypedError> {
    if body.len() > MAX_FRAME_SIZE {
        return Err(TypedError::WireFrameTooLarge {
            declared: body.len(),
        });
    }
    let mut frame = Vec::with_capacity(body.len() + 4);
    frame.extend_from_slice(&(body.len() as u32).to_le_bytes());
    frame.extend_from_slice(body);
    Ok(frame)
}

/// Decode one received frame, refusing exactly what the blocking transport
/// refuses.
fn decode_frame(datagram: &[u8]) -> Result<Vec<u8>, TypedError> {
    if datagram.is_empty() {
        return Err(recv_failure("peer closed the socket".to_owned()));
    }
    if datagram.len() < 4 {
        return Err(TypedError::WireInvalidFrame {
            detail: format!("frame too short: {} bytes", datagram.len()),
        });
    }
    let declared = u32::from_le_bytes(datagram[..4].try_into().expect("prefix slice")) as usize;
    if declared > MAX_FRAME_SIZE {
        return Err(TypedError::WireFrameTooLarge { declared });
    }
    if datagram.len() - 4 != declared {
        return Err(TypedError::WireInvalidFrame {
            detail: format!(
                "declared {declared} bytes but received {}",
                datagram.len() - 4
            ),
        });
    }
    Ok(datagram[4..].to_vec())
}

#[cfg(test)]
mod tests {
    use std::sync::LazyLock;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    use d2b_contracts_resource::v3::canonical_json_bytes;
    use d2b_contracts_resource::v3::process::{EphemeralProcessSpec, ProcessSpec};
    use d2b_contracts_resource::v3::{ControllerGeneration, ResourceRef, ResourceUid, ZoneId};
    use d2b_process_conformance::{AdoptionCandidate, ProcessIdentityDigest};
    use d2b_provider_process::{
        ExecutionMode, INVALID_PROCESS_TYPE, ProcessDriverArgs, ProcessDriverEffects,
        ProcessFamilySpec, ProcessResourceIdentity, ProviderAdoption, ProviderLiveness,
        process_family_descriptors,
    };
    use d2b_resource_types::{
        DriverDescriptor, OperationCtx, OperationDef, OperationFailure, OperationHandler,
        OperationResult, ValidatedPayload,
    };
    use d2bd_runtime::unix_transport::{connect_seqpacket, read_frame};
    use tokio::sync::Semaphore;

    use super::*;
    use crate::provider_lifecycle::{ProviderSet, family_declaration};

    /// A port that refuses every effect: the pilot operation answers from the
    /// family's declaration alone, so an effect call would fail this test
    /// loudly instead of passing unnoticed.
    struct RefusingEffects;

    #[async_trait::async_trait]
    impl ProcessDriverEffects for RefusingEffects {
        async fn launch(
            &self,
            _identity: &ProcessResourceIdentity,
            _spec: &ProcessSpec,
            _timeout: Duration,
        ) -> Result<ProcessIdentityDigest, String> {
            Err("refused".to_owned())
        }

        async fn launch_ephemeral(
            &self,
            _identity: &ProcessResourceIdentity,
            _spec: &EphemeralProcessSpec,
            _timeout: Duration,
        ) -> Result<ProcessIdentityDigest, String> {
            Err("refused".to_owned())
        }

        async fn adopt(
            &self,
            _identity: &ProcessResourceIdentity,
            _spec: &ProcessSpec,
        ) -> Result<ProviderAdoption, String> {
            Err("refused".to_owned())
        }

        async fn probe(
            &self,
            _identity: &ProcessResourceIdentity,
            _spec: &ProcessSpec,
        ) -> Result<ProviderLiveness, String> {
            Err("refused".to_owned())
        }

        async fn adopt_ephemeral(
            &self,
            _identity: &ProcessResourceIdentity,
            _spec: &EphemeralProcessSpec,
        ) -> Result<ProviderAdoption, String> {
            Err("refused".to_owned())
        }

        async fn probe_ephemeral(
            &self,
            _identity: &ProcessResourceIdentity,
            _spec: &EphemeralProcessSpec,
        ) -> Result<ProviderLiveness, String> {
            Err("refused".to_owned())
        }

        async fn stop(
            &self,
            _identity: &ProcessResourceIdentity,
            _spec: &ProcessSpec,
            _term_timeout: Duration,
            _kill_timeout: Duration,
        ) -> Result<bool, String> {
            Err("refused".to_owned())
        }

        async fn stop_ephemeral(
            &self,
            _identity: &ProcessResourceIdentity,
            _spec: &EphemeralProcessSpec,
            _term_timeout: Duration,
            _kill_timeout: Duration,
        ) -> Result<bool, String> {
            Err("refused".to_owned())
        }

        async fn stop_stale(
            &self,
            _provider_ref: &ResourceRef,
            _candidate: &AdoptionCandidate,
        ) -> Result<(), String> {
            Err("refused".to_owned())
        }

        async fn device_worker_launch(
            &self,
            _ctx: &mut d2b_resource_runtime::context::ResourceContext,
            _identity: &ProcessResourceIdentity,
            _spec: &ProcessFamilySpec,
        ) -> Result<Option<d2b_provider_process::DeviceWorkerLaunch>, &'static str> {
            Ok(None)
        }

        async fn finalize(&self, _identity: &ProcessResourceIdentity) -> Result<(), String> {
            Err("refused".to_owned())
        }

        fn has_active(
            &self,
            _zone: &ZoneId,
            _zone_uid: Option<&ResourceUid>,
            _resource_ref: &ResourceRef,
        ) -> bool {
            false
        }
    }

    /// The socket identity the test binds under: the caller's own uid/gid,
    /// with the production root-owned-parent check off, exactly as the
    /// daemon's unprivileged test mode resolves it.
    fn test_identity() -> RuntimeIdentity {
        RuntimeIdentity {
            daemon_uid: nix::unistd::getuid(),
            daemon_gid: nix::unistd::getgid(),
            public_socket_gid: nix::unistd::getgid(),
            unsafe_local_helper_socket_gid: None,
            expect_root_owned_parent: false,
        }
    }

    /// The stall operations the tests below forward to, beside the process
    /// family's own pilot operation. Each gated stall has its own latch, so
    /// two tests that run at once never share one.
    const STALL_THREADS: &str = "stall-threads";
    const STALL_CAPACITY: &str = "stall-capacity";
    const STALL_FOREVER: &str = "stall-forever";

    /// The latch one gated stall waits on: the handler counts itself in and
    /// then waits until the test releases it, so a call can be held inside
    /// the daemon while the test looks at the daemon.
    struct StallGate {
        entered: AtomicUsize,
        release: Semaphore,
    }

    impl StallGate {
        const fn new() -> Self {
            Self {
                entered: AtomicUsize::new(0),
                release: Semaphore::const_new(0),
            }
        }

        /// Hold the calling handler until the test releases the stall.
        async fn hold(&self) {
            self.entered.fetch_add(1, Ordering::AcqRel);
            let permit = self
                .release
                .acquire()
                .await
                .expect("the release latch is never closed");
            permit.forget();
        }

        /// Wait until `count` calls are held inside the handler.
        async fn wait_for(&self, count: usize) {
            let deadline = Instant::now() + Duration::from_secs(10);
            while self.entered.load(Ordering::Acquire) < count {
                assert!(
                    Instant::now() < deadline,
                    "only {} of {count} calls reached the handler",
                    self.entered.load(Ordering::Acquire)
                );
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }

        /// Release `count` held calls.
        fn release(&self, count: usize) {
            self.release.add_permits(count);
        }
    }

    static THREADS_GATE: StallGate = StallGate::new();
    static CAPACITY_GATE: StallGate = StallGate::new();

    /// A handler that holds its call on the gate until the test releases it.
    struct GatedHandler {
        gate: &'static StallGate,
    }

    #[async_trait::async_trait]
    impl OperationHandler for GatedHandler {
        async fn execute(
            &self,
            _ctx: OperationCtx<'_>,
            _payload: ValidatedPayload,
        ) -> Result<OperationResult, OperationFailure> {
            self.gate.hold().await;
            Ok(stall_result())
        }
    }

    /// A handler that never finishes: only the rendezvous's own handler
    /// deadline can answer a call it is handed.
    struct StalledHandler;

    #[async_trait::async_trait]
    impl OperationHandler for StalledHandler {
        async fn execute(
            &self,
            _ctx: OperationCtx<'_>,
            _payload: ValidatedPayload,
        ) -> Result<OperationResult, OperationFailure> {
            std::future::pending::<Result<OperationResult, OperationFailure>>().await
        }
    }

    static THREADS_HANDLER: GatedHandler = GatedHandler {
        gate: &THREADS_GATE,
    };
    static CAPACITY_HANDLER: GatedHandler = GatedHandler {
        gate: &CAPACITY_GATE,
    };
    static STALLED_HANDLER: StalledHandler = StalledHandler;

    /// A handler that reads the descriptor the carrier attached to its call.

    /// The forwarded request leg carries the caller's descriptor over
    /// SCM_RIGHTS;the rendezvous validates it against the wire declarations
    /// and hands it to the declared handler,so this handler reading it back
    /// proves the round trip through the real socket and the provider envelope.to
    struct FdEchoHandler;

    #[async_trait::async_trait]
    impl OperationHandler for FdEchoHandler {
        async fn execute(
            &self,
            ctx: OperationCtx<'_>,
            _payload: ValidatedPayload,
        ) -> Result<OperationResult, OperationFailure> {
            use std::os::fd::AsRawFd;
            use nix::unistd::read;
            let fd = ctx.fds.first().ok_or_else(|| OperationFailure::new(FD_LEG))?;
            let mut buf = [0_u8; 4];
            let n = read(fd.as_raw_fd(), &mut buf)
                .map_err(|error| OperationFailure::with_detail(FD_LEG, error.to_string()))?;
            let bytes = buf[..n].to_vec();
            let result = CanonicalJsonObject::parse(
                &canonical_json_bytes(&serde_json::json!({ "read": String::from_utf8_lossy(&bytes).to_string() }))
                    .expect("the read-back result is canonical JSON"),
            )
            .expect("the read-back result is a JSON object");
            Ok(OperationResult::new(result))
        }
    }

    static FD_ECHO_HANDLER: FdEchoHandler = FdEchoHandler;

    /// The stall operations plus the fd-echo operation the fixture's
    /// EphemeralProcess driver declares.


    static STALL_OPERATIONS: LazyLock<[OperationDef; 4]> = LazyLock::new(|| {
        [
            OperationDef {
                operation_ref: operation_ref(STALL_THREADS),
                handler: &THREADS_HANDLER,
            },
            OperationDef {
                operation_ref: operation_ref(STALL_CAPACITY),
                handler: &CAPACITY_HANDLER,
            },
            OperationDef {
                operation_ref: operation_ref(STALL_FOREVER),
                handler: &STALLED_HANDLER,
            },
            OperationDef {
                operation_ref: operation_ref("fd-echo"),
                handler: &FD_ECHO_HANDLER,
            },
        ]
    });

    /// One canonical operation reference, stated the way the declared tables
    /// state them.
    fn operation_ref(name: &str) -> ResourceRef {
        ResourceRef::parse(&format!("Operation/{name}")).expect("a canonical operation reference")
    }

    /// The result a released stall answers with.
    fn stall_result() -> OperationResult {
        let bytes = canonical_json_bytes(&serde_json::json!({ "stalled": true }))
            .expect("the stall result is canonical JSON");
        OperationResult::new(
            CanonicalJsonObject::parse(&bytes).expect("the stall result is a JSON object"),
        )
    }

    /// One started Zone whose `Process` family declares the pilot operation
    /// and whose EphemeralProcess driver carries the stall operations, served
    /// by a rendezvous on a real socket.
    struct ServingRendezvous {
        socket_path: PathBuf,
        _scratch: tempfile::TempDir,
        _providers: Arc<ProviderRuntime>,
    }

    impl ServingRendezvous {
        /// The rendezvous the daemon starts: the production entry point, with
        /// the production cap, handler deadline, and accepted peer identity.
        async fn start() -> Self {
            Self::served_by(|rendezvous, listener| {
                spawn_server(rendezvous, listener, tokio::runtime::Handle::current())
            })
            .await
        }

        /// The same rendezvous with the posture a test needs, so a saturated
        /// cap, a stalled handler, and a peer identity that is not the
        /// dialing process's are reachable without the production numbers.
        async fn start_with(posture: ServingPosture) -> Self {
            Self::served_by(move |rendezvous, listener| {
                let listener = AsyncSeqpacket::register(listener)?;
                tokio::spawn(serve_accepted(rendezvous, listener, posture));
                Ok(())
            })
            .await
        }

        async fn served_by<F>(serve: F) -> Self
        where
            F: FnOnce(Arc<ForwardRendezvous>, Socket) -> Result<(), TypedError>,
        {
            let zone = ZoneId::parse("test").expect("the test zone label is canonical");
            let scratch = tempfile::tempdir().expect("test scratch");
            let [process, ephemeral] = process_family_descriptors(ProcessDriverArgs {
                zone: zone.clone(),
                effects: Arc::new(RefusingEffects),
                zone_uid: None,
                policy_revision: None,
                provider_assignment_generation: None,
                controller_generation: ControllerGeneration::new(1)
                    .expect("the test generation is canonical"),
                guest_execution: None,
                mode: ExecutionMode::Host,
            });
            let providers = ProviderSet::new(zone.clone(), scratch.path().to_path_buf())
                .with(
                    family_declaration("process"),
                    vec![
                        process,
                        DriverDescriptor {
                            operations: &STALL_OPERATIONS[..],
                            ..ephemeral
                        },
                    ],
                )
                .start()
                .await
                .expect("the process family starts through the base");
            let providers = Arc::new(providers);
            let rendezvous = Arc::new(ForwardRendezvous::new());
            rendezvous.publish(zone.as_str(), Arc::clone(&providers));
            let socket_path = scratch.path().join("d2bd-forward.sock");
            let listener = bind(&socket_path, &test_identity()).expect("bind the rendezvous");
            serve(Arc::clone(&rendezvous), listener).expect("start the rendezvous server");
            Self {
                socket_path,
                _scratch: scratch,
                _providers: providers,
            }
        }
    }

    /// Forward one invocation the way the broker's forwarder does, driven on
    /// the runtime rather than on a thread the test owns: one connection, one
    /// canonical request frame, one awaited reply frame.
    async fn forward_async(
        socket_path: PathBuf,
        operation: &str,
        zone: &str,
        payload: serde_json::Value,
    ) -> ForwardOperationResponse {
        let request = ForwardOperationRequest {
            operation: operation.to_owned(),
            zone: zone.to_owned(),
            invocation_id: "invocation-7".to_owned(),
            payload,
            fd_indexes: vec![],
            fd_kinds: vec![],
        };
        let encoded =
            canonical_json_bytes(&request).expect("the request encodes as canonical JSON");
        let socket = connect_seqpacket(&socket_path).expect("dial the rendezvous");
        let connection =
            AsyncSeqpacket::register(Socket::from(socket)).expect("register the forwarded call");
        let deadline = Duration::from_secs(10);
        connection
            .write_frame(&encoded, deadline)
            .await
            .expect("write the request frame");
        let frame = connection
            .read_frame(deadline)
            .await
            .expect("read the reply frame");
        serde_json::from_slice(&frame).expect("the reply is a ForwardOperationResponse")
    }

    /// The threads this process is running, one per task entry.
    fn thread_count() -> usize {
        std::fs::read_dir("/proc/self/task")
            .expect("/proc/self/task is readable")
            .count()
    }

    /// The production posture with a test's own in-flight cap and handler
    /// deadline.
    fn posture(max_inflight: usize, handler_deadline: Duration) -> ServingPosture {
        ServingPosture {
            max_inflight,
            handler_deadline,
            ..ServingPosture::production()
        }
    }

    /// Forward one invocation over a real socket the way the broker's
    /// forwarder does: one connection, one canonically encoded request frame,
    /// one reply frame.
    fn forward(
        socket_path: &Path,
        operation: &str,
        zone: &str,
        payload: serde_json::Value,
    ) -> ForwardOperationResponse {
        let socket = connect_seqpacket(socket_path).expect("dial the rendezvous");
        let request = ForwardOperationRequest {
            operation: operation.to_owned(),
            zone: zone.to_owned(),
            invocation_id: "invocation-7".to_owned(),
            payload,
            fd_indexes: vec![],
            fd_kinds: vec![],
        };
        // The broker encodes the request with the canonical profile, so the
        // endpoint is exercised against the exact bytes the broker sends.
        let encoded =
            canonical_json_bytes(&request).expect("the request encodes as canonical JSON");
        d2bd_runtime::unix_transport::write_frame(&socket, &encoded)
            .expect("write the request frame");
        let frame = read_frame(&socket).expect("read the reply frame");
serde_json::from_slice(&frame).expect("the reply is a ForwardOperationResponse")
    }

    use std::os::fd::{AsRawFd, RawFd};
    use d2b_contracts_broker::broker_wire::{FD_LEG, FdKind, MAX_FRAME_FDS};
    use d2bd_runtime::unix_transport::write_frame_with_fds;
    /// Forward one invocation with SCM_RIGHTS attachments on the request
    /// frame, the way the broker's forwarder does once the request leg
    /// carries fds.to
    fn forward_with_fds(
        socket_path: &Path,
        operation: &str,
        zone: &str,
        payload: serde_json::Value,
        fds: &[RawFd],
    ) -> ForwardOperationResponse {
        let request = ForwardOperationRequest {
            operation: operation.to_owned(),
            zone: zone.to_owned(),
            invocation_id: "invocation-7".to_owned(),
            payload,
            fd_indexes: (0..fds.len() as u32).collect(),
            fd_kinds: vec![FdKind::Fifo; fds.len()],
        };
        let encoded =
            canonical_json_bytes(&request).expect("the request encodes as canonical JSON");
        let socket = connect_seqpacket(socket_path).expect("dial the rendezvous");
        write_frame_with_fds(&socket, &encoded, fds).expect("write the request frame with fds");
        let frame = read_frame(&socket).expect("read the reply frame");
        serde_json::from_slice(&frame).expect("the reply is a ForwardOperationResponse")
    }

    /// Forward one invocation with explicit fd declarations, so a test can
    /// drive a request whose declarations disagree with its frame.

    fn forward_raw_declared(
        socket_path: &Path,
        fd_indexes: Vec<u32>,
        fd_kinds: Vec<FdKind>,
        fds: &[RawFd],
    ) -> ForwardOperationResponse {
        let request = ForwardOperationRequest {
            operation: "fd-echo".to_owned(),
            zone: "test".to_owned(),
            invocation_id: "invocation-7".to_owned(),
            payload: serde_json::json!({}),
            fd_indexes,
            fd_kinds,
        };
        let encoded =
            canonical_json_bytes(&request).expect("the request encodes as canonical JSON");
        let socket = connect_seqpacket(socket_path).expect("dial the rendezvous");
        write_frame_with_fds(&socket, &encoded, fds).expect("write the request frame");
        let frame = read_frame(&socket).expect("read the reply frame");
        serde_json::from_slice(&frame).expect("the reply is a ForwardOperationResponse")
    }

    /// A forwarded call crosses a real socket and the declared handler
    /// answers it:the result carries the family's own declaration, the Zone,
    /// and the invocation identifier the caller forwarded. A carrier that
    /// never reached the handler could not produce these values.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_forwarded_call_crosses_the_socket_and_the_declared_handler_answers() {
        let serving = ServingRendezvous::start().await;
        let response = forward(
            &serving.socket_path,
            "inspect-process-family",
            "test",
            serde_json::json!({ "resourceType": "Process" }),
        );
        let ForwardOperationOutcome::Result { result, .. } = response.outcome else {
            panic!("the declared operation must answer, got a refusal");
        };
        assert_eq!(result["family"], "process");
        assert_eq!(result["resourceType"], "Process");
        assert_eq!(
            result["memberTypes"],
            serde_json::json!(["Process", "EphemeralProcess"])
        );
        assert_eq!(result["zone"], "test");
        assert_eq!(
            result["operations"],
            serde_json::json!(["inspect-process-family"])
        );
        assert_eq!(result["verbs"][0], "get");
        assert_eq!(result["execution"], serde_json::json!(["host", "guest"]));
        // The handler's context carries the identifier the broker minted
        // before it forwarded, so both records name one invocation.
        assert_eq!(result["invocation"], "invocation-7");
    }

    /// An operation no started provider declares is refused by name, and so is
    /// a call naming a Zone this process has no providers for.
    #[tokio::test(flavor = "multi_thread")]
    async fn an_undeclared_operation_is_refused_by_name() {
        let serving = ServingRendezvous::start().await;
        for (operation, zone) in [
            ("NoSuchOperation", "test"),
            ("inspect-process-family", "no-such-zone"),
        ] {
            let response = forward(
                &serving.socket_path,
                operation,
                zone,
                serde_json::json!({ "resourceType": "Process" }),
            );
            assert_eq!(
                response.outcome,
                ForwardOperationOutcome::Refused {
                    code: UNCOMMITTED_OPERATION.to_owned(),
                },
                "{operation} in {zone} is not served by this process"
            );
        }
    }

    /// A refusal the handler itself decided crosses the socket under its own
    /// code, so the peer's record and the passed-through detail keep the
    /// family's vocabulary rather than a carrier-level one.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_handler_refusal_crosses_back_under_its_own_code() {
        let serving = ServingRendezvous::start().await;
        let response = forward(
            &serving.socket_path,
            "inspect-process-family",
            "test",
            serde_json::json!({ "resourceType": "Quota" }),
        );
        assert_eq!(
            response.outcome,
            ForwardOperationOutcome::Refused {
                code: INVALID_PROCESS_TYPE.to_owned(),
            }
        );
    }

    /// Calls are held inside their handlers at the same time, on the runtime:
    /// they do not queue behind one another, and calls in flight do not add a
    /// thread each - before this the serving path owned one thread per call,
    /// so four stalled calls added four.
    #[tokio::test(flavor = "multi_thread")]
    async fn concurrent_calls_are_held_on_the_runtime_without_a_thread_each() {
        const CALLS: usize = 4;
        let serving = ServingRendezvous::start().await;
        // One warm call, so the runtime's workers exist before the baseline.
        let warm = forward_async(
            serving.socket_path.clone(),
            "inspect-process-family",
            "test",
            serde_json::json!({ "resourceType": "Process" }),
        )
        .await;
        assert!(matches!(
            warm.outcome,
            ForwardOperationOutcome::Result { .. }
        ));

        let before = thread_count();
        let calls: Vec<_> = (0..CALLS)
            .map(|_| {
                tokio::spawn(forward_async(
                    serving.socket_path.clone(),
                    STALL_THREADS,
                    "test",
                    serde_json::json!({}),
                ))
            })
            .collect();
        // Every call reached the handler while none of them was released: the
        // second call does not wait for the first.
        THREADS_GATE.wait_for(CALLS).await;
        let held = thread_count();
        assert!(
            held <= before + 2,
            "{CALLS} calls in flight must not each own a thread: {before} -> {held}"
        );

        THREADS_GATE.release(CALLS);
        for call in calls {
            let answered = call.await.expect("the call task joins");
            assert!(
                matches!(answered.outcome, ForwardOperationOutcome::Result { .. }),
                "a released call answers"
            );
        }
    }

    /// A handler that never finishes is refused by the rendezvous's own
    /// deadline - by name, while the caller is still listening - and the slot
    /// it held is free again, so the call that follows is served rather than
    /// refused at the cap.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_stalled_handler_is_refused_by_name_and_frees_its_slot() {
        const HANDLER_DEADLINE: Duration = Duration::from_millis(200);
        let serving = ServingRendezvous::start_with(posture(1, HANDLER_DEADLINE)).await;

        let started = Instant::now();
        let refused = forward_async(
            serving.socket_path.clone(),
            STALL_FOREVER,
            "test",
            serde_json::json!({}),
        )
        .await;
        assert_eq!(
            refused.outcome,
            ForwardOperationOutcome::Refused {
                code: FORWARD_TIMEOUT.to_owned(),
            },
            "a handler that never finishes is refused by the deadline of the call"
        );
        assert!(
            started.elapsed() >= HANDLER_DEADLINE,
            "the refusal is the deadline's, not an immediate one: {:?}",
            started.elapsed()
        );

        // The slot the refused call held is free: with a cap of one, the call
        // that follows is served rather than refused for capacity.
        let after = Instant::now();
        loop {
            let served = forward_async(
                serving.socket_path.clone(),
                "inspect-process-family",
                "test",
                serde_json::json!({ "resourceType": "Process" }),
            )
            .await;
            if matches!(served.outcome, ForwardOperationOutcome::Result { .. }) {
                break;
            }
            assert!(
                after.elapsed() < Duration::from_secs(5),
                "the slot a refused call held was never released: {:?}",
                served.outcome
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// A call the rendezvous is at its cap for is answered with the daemon's
    /// own capacity code - a named refusal, not a silent close - and the
    /// calls already in flight are undisturbed.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_call_over_the_in_flight_cap_is_refused_by_name() {
        let serving = ServingRendezvous::start_with(posture(1, Duration::from_secs(10))).await;
        let held = tokio::spawn(forward_async(
            serving.socket_path.clone(),
            STALL_CAPACITY,
            "test",
            serde_json::json!({}),
        ));
        CAPACITY_GATE.wait_for(1).await;

        let over_cap = forward_async(
            serving.socket_path.clone(),
            "inspect-process-family",
            "test",
            serde_json::json!({ "resourceType": "Process" }),
        )
        .await;
        assert_eq!(
            over_cap.outcome,
            ForwardOperationOutcome::Refused {
                code: TypedError::DaemonBusy.kind().to_owned(),
            },
            "the cap refuses under the code the daemon's own socket refuses with"
        );

        CAPACITY_GATE.release(1);
        let answered = held.await.expect("the held call joins");
        assert!(
            matches!(answered.outcome, ForwardOperationOutcome::Result { .. }),
            "the call admitted before the cap was reached still answers"
        );
    }

    /// A peer this endpoint does not accept is refused by name before a
    /// frame is served: the group DAC that lets the dialer connect is not the
    /// admission, `SO_PEERCRED` is.
    ///
    /// The accepted identity is moved off the dialing process's, which is the
    /// only way to exercise the refusal from inside one process. The root arm
    /// cannot be moved - root is the privileged broker and is always
    /// accepted - so a run as root has nothing to refuse here.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_peer_that_is_not_the_broker_is_refused_by_name() {
        if nix::unistd::geteuid().is_root() {
            return;
        }
        let not_the_dialer = ServingPosture {
            accepted_uid: u32::MAX,
            ..ServingPosture::production()
        };
        let serving = ServingRendezvous::start_with(not_the_dialer).await;
        let response = forward_async(
            serving.socket_path.clone(),
            "inspect-process-family",
            "test",
            serde_json::json!({ "resourceType": "Process" }),
        )
        .await;
        assert_eq!(
            response.outcome,
            ForwardOperationOutcome::Refused {
                code: UNGRANTED_CALLER.to_owned(),
            },
            "the dialing process is not the accepted peer"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_fd_carrying_request_crosses_the_socket_and_the_declared_handler_reads_it_back() {
        use nix::unistd::{pipe, write};
        let serving = ServingRendezvous::start().await;
        let (read_end, write_end) = pipe().expect("pipe");
        write(&write_end, b"ok").expect("write the payload bytes");
        drop(write_end);
        let response = forward_with_fds(
            &serving.socket_path,
            "fd-echo",
            "test",
            serde_json::json!({}),
            &[read_end.as_raw_fd()],
        );
        let ForwardOperationOutcome::Result { result, .. } = response.outcome else {
            panic!("the fd-echo operation must answer, got a refusal");
        };
        assert_eq!(
            result["read"],
            "ok",
            "the handler must have read the caller's bytes through the received descriptor"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_request_whose_fd_count_mismatches_is_refused_with_the_fd_leg_code() {
        use nix::unistd::{pipe, write};
        let serving = ServingRendezvous::start().await;
        let (read_end, write_end) = pipe().expect("pipe");
        write(&write_end, b"x").expect("write payload bytes");
        let response = forward_raw_declared(
            &serving.socket_path,
            vec![0, 1],
            vec![FdKind::Fifo, FdKind::Fifo],
            &[read_end.as_raw_fd()],
        );
        assert_eq!(
            response.outcome,
            ForwardOperationOutcome::Refused {
                code: FD_LEG.to_owned(),
            },
            "a declared count that disagrees with the frame is refused with the fd-leg code"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_request_whose_fd_kind_mismatches_is_refused_with_the_fd_leg_code() {
        use nix::unistd::pipe;
        let serving = ServingRendezvous::start().await;
        let (read_end, _write_end) = pipe().expect("pipe");
        let response = forward_raw_declared(
            &serving.socket_path,
            vec![0],
            vec![FdKind::Socket],
            &[read_end.as_raw_fd()],
        );
        assert_eq!(
            response.outcome,
            ForwardOperationOutcome::Refused {
                code: FD_LEG.to_owned(),
            },
            "a descriptor whose kernel kind mismatches the declaration is refused"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_request_whose_fd_declarations_exceed_the_frame_ceiling_is_refused_with_the_fd_leg_code() {
        let serving = ServingRendezvous::start().await;
        let response = forward_raw_declared(
            &serving.socket_path,
            (0..=MAX_FRAME_FDS as u32).collect(),
            vec![FdKind::Fifo; MAX_FRAME_FDS + 1],
            &[],
        );
        assert_eq!(
            response.outcome,
            ForwardOperationOutcome::Refused {
                code: FD_LEG.to_owned(),
            },
            "declarations over the frame ceiling are refused with the fd-leg code, never truncated"
        );
    }

    }
