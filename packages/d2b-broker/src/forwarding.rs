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
//! a forwarding peer stays fail-closed.

use std::io;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::time::Duration;

use d2b_contracts_broker::broker_wire::{
    ForwardOperationOutcome, ForwardOperationRequest, ForwardOperationResponse,
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
    /// The validated canonical payload.
    pub payload: &'a CanonicalJsonObject,
}

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
    fn forward(&self, invocation: ForwardedOperation<'_>) -> Result<DispatchOutcome, DispatchFailure>;
}

/// A forwarder that answers for no operation.
///
/// The fail-closed default: a broker whose configuration names no
/// forwarding peer refuses every forwarded operation and names the missing
/// handler as the reason.
#[derive(Debug, Default)]
pub struct UnroutedForwarder;

impl OperationForwarder for UnroutedForwarder {
    fn forward(
        &self,
        invocation: ForwardedOperation<'_>,
    ) -> Result<DispatchOutcome, DispatchFailure> {
        Err(DispatchFailure::unregistered_handler(format!(
            "no forwarding peer serves {}",
            invocation.operation
        )))
    }
}

/// A forwarder that dials one Unix socket per invocation.
///
/// The carrier is the committed [`ForwardOperationRequest`] shape, so the
/// answering peer decides what a name means inside its own process; the
/// broker contributes the committed name, the Zone, the invocation
/// identifier, and the payload it validated, and nothing else.
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
    fn forward(
        &self,
        invocation: ForwardedOperation<'_>,
    ) -> Result<DispatchOutcome, DispatchFailure> {
        let payload = serde_json::to_value(invocation.payload).map_err(|error| {
            DispatchFailure::with_detail(ERRORED, format!("render forwarded payload: {error}"))
        })?;
        let request = ForwardOperationRequest {
            operation: invocation.operation.to_owned(),
            zone: invocation.zone.to_owned(),
            invocation_id: invocation.invocation_id.to_owned(),
            payload,
        };
        let response = dial_and_exchange(&self.socket_path, &request, self.timeout)
            .map_err(|error| DispatchFailure::unregistered_handler(error.to_string()))?;
        match response.outcome {
            ForwardOperationOutcome::Result { result } => {
                let result: CanonicalJsonObject =
                    serde_json::from_value(result).map_err(|error| {
                        DispatchFailure::with_detail(
                            ERRORED,
                            format!("decode forwarded result: {error}"),
                        )
                    })?;
                Ok(DispatchOutcome { result })
            }
            ForwardOperationOutcome::Refused { code } => Err(DispatchFailure::new(code)),
        }
    }
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

/// One frame exchange with the forwarding peer.
///
/// The request is bounded by the same ceiling every broker frame is, so a
/// payload the broker admitted cannot be refused for size on the way out.
fn dial_and_exchange(
    socket_path: &Path,
    request: &ForwardOperationRequest,
    timeout: Duration,
) -> io::Result<ForwardOperationResponse> {
    let payload = canonical_json_bytes(request)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "encode forward request"))?;
    if payload.len() > crate::protocol::MAX_FRAME_SIZE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "forward request exceeds the frame ceiling",
        ));
    }
    let fd = crate::protocol::connect_seqpacket(socket_path)?;
    apply_round_trip_timeout(&fd, timeout)?;
    crate::protocol::send_json_frame(fd.as_raw_fd(), request)?;
    let response = crate::protocol::recv_json_frame::<ForwardOperationResponse>(fd.as_raw_fd())?;
    response.ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "peer closed without a reply"))
}

/// Bound the whole dial-to-reply round trip with one absolute deadline.
///
/// A peer that accepts a forwarded call and then stalls must not hold a
/// broker worker past the budget it was given, so the read deadline is the
/// bound that survives a peer this process does not control. A sub-tick
/// budget floors to one microsecond rather than zero, because a zero
/// `SO_RCVTIMEO` means "wait forever" to the kernel.
fn apply_round_trip_timeout(fd: impl std::os::fd::AsFd, timeout: Duration) -> io::Result<()> {
    let seconds = i64::try_from(timeout.as_secs()).unwrap_or(i64::MAX);
    let mut micros = i64::try_from(timeout.subsec_micros()).unwrap_or(0);
    if seconds == 0 && micros == 0 {
        micros = 1;
    }
    let deadline = nix::sys::time::TimeVal::new(seconds, micros);
    nix::sys::socket::setsockopt(&fd, nix::sys::socket::sockopt::ReceiveTimeout, &deadline)
        .map_err(io_error_from_errno)?;
    nix::sys::socket::setsockopt(&fd, nix::sys::socket::sockopt::SendTimeout, &deadline)
        .map_err(io_error_from_errno)
}

fn io_error_from_errno(errno: nix::errno::Errno) -> io::Error {
    io::Error::from_raw_os_error(errno as i32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{bind_seqpacket, recv_json_frame, send_json_frame};
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
        fn spawn(answer: impl Fn(ForwardOperationRequest) -> ForwardOperationResponse + Send + 'static) -> Self {
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
    }

    fn accept(listener: &std::os::fd::OwnedFd) -> io::Result<std::os::fd::OwnedFd> {
        use nix::sys::socket::{SockFlag, accept4};
        accept4(listener.as_raw_fd(), SockFlag::empty())
            .map(crate::sys::owned_fd_from_raw)
            .map_err(io_error_from_errno)
    }

    fn echo_digest(request: ForwardOperationRequest) -> ForwardOperationResponse {
        let payload: CanonicalJsonObject =
            serde_json::from_value(request.payload).expect("canonical payload");
        ForwardOperationResponse {
            outcome: ForwardOperationOutcome::Result {
                result: serde_json::json!({
                    "operation": request.operation,
                    "invocation": request.invocation_id,
                    "fields": payload.len(),
                    "digest": d2b_contracts_resource::v3::resource_schema::canonical_digest(
                        "d2b.test.forward",
                        &payload.to_canonical_bytes(),
                    ),
                }),
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
        let outcome = peer
            .forwarder()
            .forward(ForwardedOperation {
                operation: "UsbipBind",
                zone: "work",
                invocation_id: "invocation-7",
                payload: &payload,
            })
            .expect("the peer answered");
        assert_eq!(peer.calls(), 1);
        let rendered = outcome.result.to_canonical_bytes();
        let rendered = String::from_utf8(rendered).expect("canonical json is utf-8");
        assert!(rendered.contains("\"operation\":\"UsbipBind\""), "{rendered}");
        assert!(rendered.contains("\"invocation\":\"invocation-7\""), "{rendered}");
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
        let outcome = peer
            .forwarder()
            .forward(ForwardedOperation {
                operation: "UsbipBind",
                zone: "work",
                invocation_id: "invocation-8",
                payload: &other,
            })
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
        let failure = peer
            .forwarder()
            .forward(ForwardedOperation {
                operation: "UsbipBind",
                zone: "work",
                invocation_id: "invocation-9",
                payload: &payload(),
            })
            .expect_err("the peer refused the call");
        assert_eq!(failure.code, "usbip-device-absent");
        assert_eq!(peer.calls(), 1);
    }

    #[test]
    fn an_unrouted_forwarder_refuses_every_operation() {
        let failure = UnroutedForwarder
            .forward(ForwardedOperation {
                operation: "UsbipBind",
                zone: "work",
                invocation_id: "invocation-10",
                payload: &payload(),
            })
            .expect_err("no peer is configured");
        assert_eq!(failure.code, crate::envelope::UNREGISTERED_HANDLER);
    }

    #[test]
    fn a_peer_that_is_not_there_refuses_rather_than_succeeding() {
        let dir = tempfile::tempdir().expect("dir");
        let forwarder = SocketForwarder::new(dir.path().join("absent.sock"));
        let failure = forwarder
            .forward(ForwardedOperation {
                operation: "UsbipBind",
                zone: "work",
                invocation_id: "invocation-11",
                payload: &payload(),
            })
            .expect_err("an absent peer serves nothing");
        assert_eq!(failure.code, crate::envelope::UNREGISTERED_HANDLER);
    }
}
