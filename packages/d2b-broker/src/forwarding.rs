//! Broker-side forwarding of validated operations to declaring processes.
//!
//! The broker holds the committed rows and links no provider crate, so the
//! dispatch step of the operation envelope cannot run a family row's
//! `OperationDef.handler` locally. It forwards instead: one validated,
//! authorized invocation crosses to the process that declared the handler,
//! which answers with the handler's canonical result or its refusal code.
//!
//! Two properties are structural. A forwarder answers for exactly the
//! operations a peer has registered; every other operation is refused with
//! [`UNREGISTERED_HANDLER`](crate::envelope::UNREGISTERED_HANDLER), so a
//! transport a peer has not wired is a closed refusal and never a silent
//! success. And the payload that crosses is the canonical object the
//! envelope validated - [`ForwardedOperation`] borrows it, so a forwarder
//! cannot substitute, widen, or re-render the payload it was handed.
//!
//! The transport is a Unix `SOCK_SEQPACKET` dial: one connection, one
//! [`ForwardOperationRequest`] frame, one [`ForwardOperationResponse`]
//! frame. The peer that answers owns the socket path; the broker refuses
//! every forwarded operation while it has none, so a broker started without
//! a forwarding peer stays fail-closed. The dial and the exchange wait in
//! async time under one round-trip deadline, so a peer that accepts and then
//! stalls costs the caller a waiting task instead of a blocked thread.

use std::io;
use std::os::fd::{AsRawFd, OwnedFd, RawFd};
use std::path::{Path, PathBuf};
use std::time::Duration;

use d2b_contracts_broker::broker_wire::{
    FD_LEG, FdKind, MAX_FRAME_FDS, ForwardOperationOutcome, ForwardOperationRequest,
    ForwardOperationResponse,
};
use d2b_contracts_resource::v3::{CanonicalJsonObject, canonical_json_bytes};

use crate::envelope::{DispatchFailure, DispatchOutcome, ERRORED};

/// The environment variable that names the forwarding peer's socket.
///
/// The deployment fact is declared with the carrier it configures, so the
/// broker and the peer that binds the socket cannot drift apart.
pub use d2b_contracts_broker::FORWARD_SOCKET_ENV;

/// The environment variable that bounds one forward round trip, in
/// milliseconds.
pub const FORWARD_TIMEOUT_ENV: &str = "D2B_BROKER_FORWARD_TIMEOUT_MS";

/// The round-trip budget a forwarder dials with when the environment names
/// none.
pub const DEFAULT_FORWARD_TIMEOUT: Duration = Duration::from_secs(30);

/// One validated, authorized invocation as a forwarder sees it.
///
/// The payload is the canonical object the envelope validated against the
/// committed row, borrowed rather than copied: a forwarder that changed it
/// would be serving an operation the broker did not admit.
#[derive(Debug)]
pub struct ForwardedOperation<'a> {
    /// The committed operation name the caller invoked.
    pub operation: &'a str,
    /// The Zone the invocation runs in.
    pub zone: &'a str,
    /// The invocation identifier the broker's audit record carries.
    pub invocation_id: &'a str,
    /// The evidence chain this leg runs under (KTD6): the root invocation
    /// id and the ordered identities, root first. A nested call's chain
    /// travels with the forwarded invocation so the peer's leg records its
    /// correlation key the same way the broker's leg would.
    pub chain: &'a d2b_audit::evidence_chain::EvidenceChain,
    /// Whether this leg is a nested call under an existing invocation
    /// (U10, KTD6).
    ///
    /// The flag is the wire's chain-bearing signal: a nested call presents
    /// a chain even when the chain carries a single identity, so the peer
    /// records the leg as a correlation record rather than a second root
    /// record for the invocation id.
    pub nested: bool,
    /// The validated canonical payload.
    pub payload: &'a CanonicalJsonObject,
    /// The descriptors the caller attached to this invocation,when any.
    /// The forwarder borrows them for the frame it sends,never owns them.
    pub fds: &'a [OwnedFd],

    /// The kernel kind the row's fd facet declares for those descriptors,
    /// when it declares one;the envelope validated every attached descriptor
    /// against it before dispatch, so the forwarder can declare it back to
    /// the peer verbatim.

    pub fd_kind: Option<FdKind>,
    /// The broker-attested context block the envelope minted for this
    /// invocation, when the broker holds a context store. The forwarder
    /// transports it verbatim; it never mints, edits, or drops a block the
    /// envelope handed it.
    pub context: Option<&'a d2b_contracts_broker::broker_wire::ForwardContext>,
}

/// The round-trip budget the environment names, when it names one.
///
/// A zero or unparsable budget is not a budget: the default stands rather
/// than a peer reading an unbounded or instantaneous deadline.
pub fn default_forward_timeout() -> Duration {
    std::env::var(FORWARD_TIMEOUT_ENV)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|millis| *millis > 0)
        .map(Duration::from_millis)
        .unwrap_or(DEFAULT_FORWARD_TIMEOUT)
}

/// The future one forwarded call completes on.
///
/// The dispatcher holds its forwarder as a trait object, so the future is
/// boxed rather than an associated type: `dyn OperationForwarder` is the
/// seam, and a boxed future is the price of keeping it.
pub type ForwardFuture<'a> = std::pin::Pin<
    Box<dyn std::future::Future<Output = Result<DispatchOutcome, DispatchFailure>> + Send + 'a>,
>;

/// The peer-mediated half of one dispatch: the process that declares an
/// operation's handler.
pub trait OperationForwarder: Send + Sync + 'static {
    /// Forward one validated, authorized invocation.
    ///
    /// A forwarder answers for exactly the operations its peer registered.
    /// An operation the peer does not serve - and a peer the forwarder
    /// cannot reach - is [`DispatchFailure::unregistered_handler`] rather
    /// than a success or a synthesized result, because a dispatch that did
    /// not reach a handler did not happen.
    ///
    /// The call is async because the peer leg is: the dial, the request
    /// frame, and the reply frame all wait in async time under one deadline,
    /// so a peer that accepts and then stalls costs a waiting task rather
    /// than a broker thread.
    fn forward<'a>(&'a self, invocation: ForwardedOperation<'a>) -> ForwardFuture<'a>;
}

/// A forwarder that answers for no operation.
///
/// The fail-closed default: a broker whose configuration names no
/// forwarding peer refuses every forwarded operation and names the missing
/// handler as the reason.
#[derive(Debug, Default)]
pub struct UnroutedForwarder;

impl OperationForwarder for UnroutedForwarder {
    fn forward<'a>(&'a self, invocation: ForwardedOperation<'a>) -> ForwardFuture<'a> {
        Box::pin(async move {
            Err(DispatchFailure::unregistered_handler(format!(
                "no forwarding peer serves {}",
                invocation.operation
            )))
        })
    }
}

/// A forwarder that dials one Unix socket per invocation.
///
/// The carrier is the committed [`ForwardOperationRequest`] shape, so the
/// answering peer decides what a name means inside its own process; the
/// broker contributes the committed name, the Zone, the invocation
/// identifier, and the payload it validated, and nothing else.
///
/// The evidence chain is a seam object, not a socket field: the committed
/// request shape carries the root invocation id and the minted context's
/// initiating identity, so the wire preserves the root anchor and the
/// invoking identity of a nested call while the full chain crosses once
/// the carrier's request shape grows (a later carrier unit's wire change).
/// Until then the daemon-side leg re-roots a chain from the request's id,
/// context, and the attestation it validates.
///
/// One connection carries one invocation. A dial that fails, a peer that
/// closes without answering, and a malformed or oversized frame are all
/// [`DispatchFailure::unregistered_handler`]: from the envelope's side the
/// operation reached no handler, which is exactly what happened.
#[derive(Debug, Clone)]
pub struct SocketForwarder {
    socket_path: PathBuf,
    timeout: Duration,
}

impl SocketForwarder {
    /// Dial one peer's forwarding socket.
    pub fn new(socket_path: impl Into<PathBuf>) -> Self {
        Self {
            socket_path: socket_path.into(),
            timeout: default_forward_timeout(),
        }
    }

    /// Dial one peer's forwarding socket within one round-trip budget.
    pub fn with_timeout(socket_path: impl Into<PathBuf>, timeout: Duration) -> Self {
        Self {
            socket_path: socket_path.into(),
            timeout,
        }
    }

    /// Borrow the socket this forwarder dials.
    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }
}

impl OperationForwarder for SocketForwarder {
    fn forward<'a>(&'a self, invocation: ForwardedOperation<'a>) -> ForwardFuture<'a> {
        Box::pin(self.forward_to_peer(invocation))
    }
}

impl SocketForwarder {
    /// Forward one invocation and decode the peer's answer.
    async fn forward_to_peer(
        &self,
        invocation: ForwardedOperation<'_>,
    ) -> Result<DispatchOutcome, DispatchFailure> {
        let payload = serde_json::to_value(invocation.payload).map_err(|error| {
            DispatchFailure::with_detail(ERRORED, format!("render forwarded payload: {error}"))
        })?;
        let fd_indexes: Vec<u32> = (0..invocation.fds.len() as u32).collect();
        // The request-leg kinds were validated against the committed row before
        // dispatch, so the forwarder only transports the indexes; the peer
        // maps each declared index to the kernel attachment the frame order
        // assigns it. An `Any` row's descriptors keep their own fstat kinds
        // so the receiving leg can still validate them per index.
        let fd_kinds: Vec<FdKind> = match invocation.fd_kind {
            Some(FdKind::Any) => invocation
                .fds
                .iter()
                .map(|fd| Self::fd_kind_of(fd).unwrap_or(FdKind::Any))
                .collect(),
            Some(kind) => vec![kind; invocation.fds.len()],
            None => Vec::new(),
        };
        let request = ForwardOperationRequest {
            operation: invocation.operation.to_owned(),
            zone: invocation.zone.to_owned(),
            invocation_id: invocation.invocation_id.to_owned(),
            payload,
            // The broker-minted context crosses verbatim: the forwarder
            // transports the block the envelope attested, never a copy it
            // re-derived from the request.
            context: invocation.context.cloned(),
            // The evidence chain's identities cross with a nested leg
            // (U10, KTD6): the peer re-roots the chain from the root
            // invocation id plus these identities, records the leg as a
            // correlation record rather than a second root record, and
            // hands the chain to the declaring handler's context.
            chain_identities: invocation.nested.then(|| {
                invocation
                    .chain
                    .identities()
                    .to_vec()
            }),
            fd_indexes,
            fd_kinds,
        };
        let raw: Vec<RawFd> = invocation.fds.iter().map(AsRawFd::as_raw_fd).collect();
        let (response, response_fds) = self
            .exchange(&request, &raw)
            .await
            .map_err(|error| DispatchFailure::unregistered_handler(error.to_string()))?;
        match response.outcome {
            ForwardOperationOutcome::Result { result, fd_indexes, fd_kinds } => {
                if !Self::response_fds_admitted(&fd_indexes, &fd_kinds, &response_fds, invocation.fds) {
                    // The peer declared a leg that did not arrive with it - or
                    // returned one of the caller's own descriptors - so the
                    // answer leg is invalid closed,count-validated rather than
                    // truncated:the refused code is the fd-leg code,the
                    // carrier's own..
                    drop(response_fds);
                    return Err(DispatchFailure::new(FD_LEG));
                }
                let result: CanonicalJsonObject =
                    serde_json::from_value(result).map_err(|error| {
                        DispatchFailure::with_detail(
                            ERRORED,
                            format!("decode forwarded result: {error}"),
                        )
                    })?;
                Ok(DispatchOutcome { result, fds: response_fds })
            }
            ForwardOperationOutcome::Refused { code } => {
                drop(response_fds);
                Err(DispatchFailure::new(code))
            }
        }
    }

    /// Whether one reply leg the peer returned is admitted:count against
    /// the declared indexes,indexes in frame order,kinds against the kernel
    /// stat of each received descriptor,and no descriptor the caller itself
    /// attached (a peer that returned the caller's own descriptor did not
    /// mint it).
    fn response_fds_admitted(
        declared_indexes: &[u32],
        declared_kinds: &[FdKind],
        received: &[OwnedFd],
        request_fds: &[OwnedFd],
    ) -> bool {
        if received.len() != declared_indexes.len() || declared_indexes.len() != declared_kinds.len() {
            return false;
        }
        if received.len() > MAX_FRAME_FDS {
            return false;
        }
        if declared_indexes
            .iter()
            .enumerate()
            .any(|(position, declared)| *declared != position as u32)
        {
            return false;
        }
        if !declared_kinds
            .iter()
            .zip(received)
            .all(|(declared, fd)| {
                // An `Any` declaration admits every descriptor regardless
                // of fstat kind (the mixed or anon-inode legs, U10).
                *declared == FdKind::Any || Self::fd_kind_of(fd) == Some(*declared)
            })
        {
            return false;
        }
        let ours: Vec<(u64, u64, u64)> = request_fds
            .iter()
            .filter_map(Self::fstat_triple)
            .collect();
        if received
            .iter()
            .filter_map(Self::fstat_triple)
            .any(|triple| ours.contains(&triple))
        {
            return false;
        }
        true
    }

    /// The kernel kind one descriptor presents,or None when its fstat
    /// reports a kind the carrier vocabulary does not carry..
    fn fd_kind_of(fd: &OwnedFd) -> Option<FdKind> {
        use nix::libc;
        let stat = nix::sys::stat::fstat(fd.as_raw_fd()).ok()?;
        match stat.st_mode & libc::S_IFMT {
            libc::S_IFIFO => Some(FdKind::Fifo),
            libc::S_IFSOCK => Some(FdKind::Socket),
            libc::S_IFCHR => Some(FdKind::CharDevice),
            libc::S_IFBLK => Some(FdKind::BlockDevice),
            libc::S_IFREG => Some(FdKind::Regular),
            libc::S_IFDIR => Some(FdKind::Directory),
            _ => None,
        }
    }

    /// The identity triple (device,inode,mode) a caller's own descriptor
    /// presents,so a received descriptor can be recognized as one of them.
    fn fstat_triple(fd: &OwnedFd) -> Option<(u64, u64, u64)> {
        let stat = nix::sys::stat::fstat(fd.as_raw_fd()).ok()?;
        Some((stat.st_dev, stat.st_ino, stat.st_mode as u64))
    }

    /// One frame exchange with the forwarding peer.
    ///
    /// The request is bounded by the same ceiling every broker frame is, so a
    /// payload the broker admitted cannot be refused for size on the way out.
    /// The whole round trip - dial, request frame, reply frame - sits under
    /// one async deadline: the connection is nonblocking, so a peer that
    /// accepts and then stalls is waited out by this deadline rather than by
    /// the kernel, and no socket timeout has to be armed for it.
async fn exchange(
        &self,
        request: &ForwardOperationRequest,
        request_fds: &[RawFd],
    ) -> io::Result<(ForwardOperationResponse, Vec<OwnedFd>)> {
        let payload = canonical_json_bytes(request)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "encode forward request"))?;
        if payload.len() > crate::protocol::MAX_FRAME_SIZE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "forward request exceeds the frame ceiling",
            ));
        }
        let path = self.socket_path.as_path();
        let timeout = self.timeout;
        tokio::time::timeout(timeout, async move {
            let connection = crate::protocol::connect_seqpacket_bounded(path, timeout).await?;
            connection.send_json_frame_with_fds(request, request_fds).await?;
            let response = connection
                .recv_json_frame_with_fds::<ForwardOperationResponse>()
                .await?;
            response.ok_or_else(|| {
                io::Error::new(io::ErrorKind::UnexpectedEof, "peer closed without a reply")
            })
        })
        .await
        .unwrap_or_else(|_| {
            Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!(
                    "forwarded call to {} exceeded its {} ms round-trip budget",
                    path.display(),
                    timeout.as_millis()
                ),
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{bind_seqpacket, recv_json_frame, send_json_frame, send_json_frame_with_fds};
    use std::os::fd::AsRawFd;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// One peer the broker dials: a seqpacket listener that answers every
    /// forwarded call with the canonical digest of the payload it received.
    ///
    /// The double is honest by construction - it answers only from bytes it
    /// read off the accepted connection, so a forwarder that never crossed
    /// the socket, or crossed with a different payload, cannot produce the
    /// digest the assertions expect.
    struct Peer {
        path: PathBuf,
        calls: Arc<AtomicUsize>,
        _dir: tempfile::TempDir,
    }

    impl Peer {
        fn spawn(
            answer: impl Fn(ForwardOperationRequest) -> ForwardOperationResponse + Send + 'static,
        ) -> Self {
            let dir = tempfile::tempdir().expect("peer socket dir");
            let path = dir.path().join("forward.sock");
            let listener = bind_seqpacket(&path).expect("bind peer socket");
            let calls = Arc::new(AtomicUsize::new(0));
            let observed = Arc::clone(&calls);
            std::thread::spawn(move || {
                while let Ok(fd) = accept(&listener) {
                    let Some(request) = recv_json_frame::<ForwardOperationRequest>(fd.as_raw_fd())
                        .expect("read forwarded call")
                    else {
                        continue;
                    };
                    observed.fetch_add(1, Ordering::AcqRel);
                    let response = answer(request);
                    send_json_frame(fd.as_raw_fd(), &response).expect("write forward reply");
                }
            });
            Self {
                path,
                calls,
                _dir: dir,
            }
        }

        fn forwarder(&self) -> SocketForwarder {
            SocketForwarder::new(self.path.clone())
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::Acquire)
        }

        /// A raw peer that answers with descriptor attachments, so a test
        /// can drive a response whose declarations disagree with its frame.
        fn spawn_raw(
            answer: impl FnMut(ForwardOperationRequest) -> (ForwardOperationResponse, std::os::fd::OwnedFd) + Send + 'static,
        ) -> Self {
            let dir = tempfile::tempdir().expect("peer socket dir");
            let path = dir.path().join("forward.sock");
            let listener = bind_seqpacket(&path).expect("bind peer socket");
            let calls = Arc::new(AtomicUsize::new(0));
            let observed = Arc::clone(&calls);
            std::thread::spawn(move || {
                use std::os::fd::AsRawFd;
                let mut answer = answer;
                while let Ok(fd) = accept(&listener) {
                    let Some(request) = recv_json_frame::<ForwardOperationRequest>(fd.as_raw_fd())
                        .expect("read forwarded call")
                    else {
                        continue;
                    };
                    observed.fetch_add(1, Ordering::AcqRel);
                    let (response, response_fd) = answer(request);
                    send_json_frame_with_fds(
                        fd.as_raw_fd(),
                        &response,
                        &[response_fd.as_raw_fd()],
                    )
                    .expect("write forward reply with fd attachment");
                }
            });
            Self {
                path,
                calls,
                _dir: dir,
            }
        }
    }

    fn accept(listener: &std::os::fd::OwnedFd) -> io::Result<std::os::fd::OwnedFd> {
        use nix::sys::socket::{SockFlag, accept4};
        accept4(listener.as_raw_fd(), SockFlag::empty())
            .map(crate::sys::owned_fd_from_raw)
            .map_err(|err| io::Error::from_raw_os_error(err as i32))
    }

    /// One runtime for these tests: the forwarder is async because the peer
    /// leg is, so a test drives it exactly the way the broker does.
    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Runtime::new().expect("forwarding test runtime")
    }

    /// One forwarded call through a fresh runtime, as the broker makes it.
    fn forward(
        forwarder: &SocketForwarder,
        operation: &str,
        invocation_id: &str,
        payload: &CanonicalJsonObject,
    ) -> Result<DispatchOutcome, DispatchFailure> {
        let chain = d2b_audit::evidence_chain::EvidenceChain::root(invocation_id, "daemon");
        runtime().block_on(forwarder.forward(ForwardedOperation {
            operation,
            zone: "work",
            invocation_id,
            chain: &chain,
            nested: false,
            payload,
            fds: &[],
            fd_kind: None,
            context: None,
        }))
    }

    fn echo_digest(request: ForwardOperationRequest) -> ForwardOperationResponse {
        let payload: CanonicalJsonObject =
            serde_json::from_value(request.payload).expect("canonical payload");
        ForwardOperationResponse {
            outcome: ForwardOperationOutcome::Result {
                result: serde_json::json!({
                    "operation": request.operation,
                    "invocation": request.invocation_id,
                    "zone": request.zone,
                    "fields": payload.len(),
                    "digest": d2b_contracts_resource::v3::resource_schema::canonical_digest(
                        "d2b.test.forward",
                        &payload.to_canonical_bytes(),
                    ),
                }),
                fd_indexes: vec![],
                fd_kinds: vec![],
            },
        }
    }

    fn payload() -> CanonicalJsonObject {
        CanonicalJsonObject::parse(br#"{"busId":"1-2","persistent":true}"#)
            .expect("canonical payload")
    }

    #[test]
    fn a_forwarded_call_reaches_the_peer_and_returns_its_result() {
        let peer = Peer::spawn(echo_digest);
        let payload = payload();
        let outcome = forward(&peer.forwarder(), "UsbipBind", "invocation-7", &payload)
            .expect("the peer answered");
        assert_eq!(peer.calls(), 1);
        let rendered = outcome.result.to_canonical_bytes();
        let rendered = String::from_utf8(rendered).expect("canonical json is utf-8");
        assert!(
            rendered.contains("\"operation\":\"UsbipBind\""),
            "{rendered}"
        );
        assert!(
            rendered.contains("\"invocation\":\"invocation-7\""),
            "{rendered}"
        );
        assert!(rendered.contains("\"fields\":2"), "{rendered}");
        assert!(
            rendered.contains(
                &d2b_contracts_resource::v3::resource_schema::canonical_digest(
                    "d2b.test.forward",
                    &payload.to_canonical_bytes(),
                )
            ),
            "the peer must have received the exact canonical payload: {rendered}"
        );
    }

    #[test]
    fn a_payload_the_peer_never_saw_cannot_produce_the_result() {
        let peer = Peer::spawn(echo_digest);
        let other = CanonicalJsonObject::parse(br#"{"busId":"9-9"}"#).expect("canonical payload");
        let outcome = forward(&peer.forwarder(), "UsbipBind", "invocation-8", &other)
            .expect("the peer answered");
        let rendered = String::from_utf8(outcome.result.to_canonical_bytes()).expect("utf-8");
        assert!(
            !rendered.contains(
                &d2b_contracts_resource::v3::resource_schema::canonical_digest(
                    "d2b.test.forward",
                    &payload().to_canonical_bytes(),
                )
            ),
            "a different payload must not carry the other payload's digest: {rendered}"
        );
    }

    #[test]
    fn a_peer_that_refuses_carries_its_own_code() {
        let peer = Peer::spawn(|_request| ForwardOperationResponse {
            outcome: ForwardOperationOutcome::Refused {
                code: "usbip-device-absent".to_owned(),
            },
        });
        let failure = forward(&peer.forwarder(), "UsbipBind", "invocation-9", &payload())
            .expect_err("the peer refused the call");
        assert_eq!(failure.code, "usbip-device-absent");
        assert_eq!(peer.calls(), 1);
    }

    #[test]
    fn an_unrouted_forwarder_refuses_every_operation() {
        let chain = d2b_audit::evidence_chain::EvidenceChain::root("invocation-10", "daemon");
        let failure = runtime()
            .block_on(UnroutedForwarder.forward(ForwardedOperation {
                operation: "UsbipBind",
                zone: "work",
                invocation_id: "invocation-10",
                chain: &chain,
                nested: false,
                payload: &payload(),
                fds: &[],
                fd_kind: None,
                context: None,
            }))
            .expect_err("no peer is configured");
        assert_eq!(failure.code, crate::envelope::UNREGISTERED_HANDLER);
    }

    #[test]
    fn a_peer_that_is_not_there_refuses_rather_than_succeeding() {
        let dir = tempfile::tempdir().expect("dir");
        let forwarder = SocketForwarder::new(dir.path().join("absent.sock"));
        let failure = forward(&forwarder, "UsbipBind", "invocation-11", &payload())
            .expect_err("an absent peer serves nothing");
        assert_eq!(failure.code, crate::envelope::UNREGISTERED_HANDLER);
    }

    #[test]
    fn a_peer_that_accepts_and_never_answers_is_bounded_by_the_round_trip_budget() {
        // The peer accepts the dial and then says nothing. A blocking
        // exchange with no deadline would hold the caller forever; the
        // round trip is bounded in async time instead, so the call returns
        // inside its budget with the fail-closed refusal.
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("silent.sock");
        let listener = bind_seqpacket(&path).expect("bind silent peer socket");
        let held = std::thread::spawn(move || {
            let accepted = accept(&listener).expect("accept the dial");
            // Hold the connection open past the caller's budget.
            std::thread::sleep(Duration::from_millis(750));
            drop(accepted);
        });
        let forwarder = SocketForwarder::with_timeout(path, Duration::from_millis(150));
        let started = std::time::Instant::now();
        let failure = forward(&forwarder, "UsbipBind", "invocation-12", &payload())
            .expect_err("a peer that never answers serves nothing");
        let elapsed = started.elapsed();
        assert_eq!(failure.code, crate::envelope::UNREGISTERED_HANDLER);
        assert!(
            elapsed < Duration::from_millis(700),
            "the round trip must end on its own budget, not on the peer: {elapsed:?}"
        );
        held.join().expect("silent peer thread");
    }

    use crate::envelope::FD_LEG;
    use d2b_contracts_broker::broker_wire::{FdKind, MAX_FRAME_FDS};
    use nix::unistd::pipe;

    #[test]
    fn a_response_whose_fd_count_mismatches_its_declared_indexes_is_refused_not_truncated() {

        // The peer declares two descriptors but attaches only one:the
        // carrier must refuse with the fd-leg code rather than succeed with a
        // truncated answer.where
        let (read_end, _write_end) = pipe().expect("pipe");
        let mut read_end = Some(read_end);
        let peer = Peer::spawn_raw(move |_request| {
            let response = ForwardOperationResponse {
                outcome: ForwardOperationOutcome::Result {
                    result: serde_json::json!({ "ok": true }),
                    fd_indexes: vec![0, 1],
                    fd_kinds: vec![FdKind::Fifo, FdKind::Fifo],
                },
            };
            (response, read_end.take().expect("the peer answers once"))
        });
        let failure = forward(&peer.forwarder(), "UsbipBind", "invocation-13", &payload())
            .expect_err("a count mismatch is refused, never truncated");
        assert_eq!(failure.code, FD_LEG);
        assert_eq!(peer.calls(), 1);
    }

    #[test]
    fn a_response_whose_fd_kind_mismatches_its_declared_kind_is_refused() {
        let (read_end, _write_end) = pipe().expect("pipe");
        let mut read_end = Some(read_end);
        let peer = Peer::spawn_raw(move |_request| {
            let response = ForwardOperationResponse {
                outcome: ForwardOperationOutcome::Result {
                    result: serde_json::json!({ "ok": true }),
                    fd_indexes: vec![0],
                    fd_kinds: vec![FdKind::Socket],
                },
            };
            (response, read_end.take().expect("the peer answers once"))
        });
        let failure = forward(&peer.forwarder(), "UsbipBind", "invocation-14", &payload())
            .expect_err("a pipe where the response declares a socket is refused");
        assert_eq!(failure.code, FD_LEG);
        assert_eq!(peer.calls(), 1);
    }

    #[test]
    fn a_response_whose_fd_declarations_exceed_the_frame_ceiling_is_refused() {
        // Nine declared descriptors cannot ride an eight-descriptor frame;
        // the refusal must name the fd leg, never surface as a transport-side
        // control-truncation error.where
        let (read_end, _write_end) = pipe().expect("pipe");
        let mut read_end = Some(read_end);
        let peer = Peer::spawn_raw(move |_request| {
            let response = ForwardOperationResponse {
                outcome: ForwardOperationOutcome::Result {
                    result: serde_json::json!({ "ok": true }),
                    fd_indexes: (0..=MAX_FRAME_FDS as u32).collect(),
                    fd_kinds: vec![FdKind::Fifo; MAX_FRAME_FDS + 1],
                },
            };
            (response, read_end.take().expect("the peer answers once"))
        });
        let failure = forward(&peer.forwarder(), "UsbipBind", "invocation-15", &payload())
            .expect_err("declarations over the frame ceiling are refused");
        assert_eq!(failure.code, FD_LEG);
        assert_eq!(peer.calls(), 1);
    }

    #[test]
    fn a_minted_context_crosses_the_wire_verbatim() {
        use d2b_contracts_broker::broker_wire::{ForwardContext, STALE_CONTEXT};
        // The peer echoes the context block it received: a forwarder that
        // dropped, edited, or re-derived the block could not produce the
        // minted value, so the attestation provably reaches the declaring
        // process unchanged.
        let peer = Peer::spawn(|request| ForwardOperationResponse {
            outcome: ForwardOperationOutcome::Result {
                result: serde_json::json!({
                    "epoch": request.context.as_ref().map(|c| c.broker_epoch).unwrap_or(0),
                    "zone": request.context.as_ref().map(|c| c.zone.as_str()).unwrap_or(""),
                    "revision": request.context.as_ref().map(|c| c.provider_set_revision).unwrap_or(0),
                    "controller": request.context.as_ref().map(|c| c.controller_generation).unwrap_or(0),
                    "guest": request.context.as_ref().map(|c| c.guest_generation).unwrap_or(0),
                    "identity": request.context.as_ref().map(|c| c.initiating_identity.as_str()).unwrap_or(""),
                    "deadline": request.context.as_ref().map(|c| c.deadline_ms).unwrap_or(0),
                }),
                fd_indexes: vec![],
                fd_kinds: vec![],
            },
        });
        let context = ForwardContext {
            broker_epoch: 1,
            zone: "work".to_owned(),
            provider_set_revision: 2,
            controller_generation: 3,
            guest_generation: 4,
            initiating_identity: "daemon".to_owned(),
            deadline_ms: 25_000,
        };
        let chain = d2b_audit::evidence_chain::EvidenceChain::root("invocation-16", "daemon");
        let outcome = runtime().block_on(peer.forwarder().forward(ForwardedOperation {
            operation: "UsbipBind",
            zone: "work",
            invocation_id: "invocation-16",
            chain: &chain,
            nested: false,
            payload: &payload(),
            fds: &[],
            fd_kind: None,
            context: Some(&context),
        }))
        .expect("the peer answered");
        let rendered = String::from_utf8(outcome.result.to_canonical_bytes()).expect("utf-8");
        assert!(rendered.contains("\"epoch\":1"), "{rendered}");
        assert!(rendered.contains("\"zone\":\"work\""), "{rendered}");
        assert!(rendered.contains("\"revision\":2"), "{rendered}");
        assert!(rendered.contains("\"controller\":3"), "{rendered}");
        assert!(rendered.contains("\"guest\":4"), "{rendered}");
        assert!(rendered.contains("\"identity\":\"daemon\""), "{rendered}");
        assert!(rendered.contains("\"deadline\":25000"), "{rendered}");
        assert_eq!(peer.calls(), 1);
        assert_eq!(STALE_CONTEXT, "stale-context");
    }
}
