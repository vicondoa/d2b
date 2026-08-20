//! Provider-neutral exec owner connection I/O.
//!
//! The daemon composition root owns session establishment and provider-backed
//! guest execution; this module owns the bounded owner reader/writer protocol.

use std::os::fd::{AsRawFd, RawFd};
use std::sync::Arc;
use std::time::{Duration, Instant};

use d2b_contracts_control::public_wire;
use socket2::Socket;

use crate::exec_session;
use crate::exec_support::{
    EXEC_SUBSYSTEM, exec_error_kind_label, exec_metric_into, exec_op_session, map_exec_op_error,
};
use crate::typed_error::TypedError;
use crate::unix_transport::{read_frame, write_json_frame};

enum ExecOwnerFrame {
    Response(Box<public_wire::ExecOpResponse>),
    Error {
        error: Box<TypedError>,
        metric_kind: &'static str,
    },
}

/// One item handed from the owner reader to the owner writer. `Pending` carries
/// the worker reply receiver (awaited concurrently so multiple ops, including a
/// long-poll plus an urgent control op, are in flight at once and matched by
/// `op_id`). `Immediate` is a reader-resolved error (parse / session-binding /
/// non-exec frame) that needs no worker round trip. Both variants carry the
/// owned [`InflightPermit`] for the op so the writer releases it only after the
/// reply frame is written (or on teardown), enforcing the real in-flight cap.
pub enum ExecWriterItem {
    Pending {
        op_id: u64,
        reply_rx: tokio::sync::oneshot::Receiver<
            Result<public_wire::ExecOpResponse, exec_session::ExecOpError>,
        >,
        permit: InflightPermit,
    },
    Immediate {
        op_id: u64,
        error: Box<TypedError>,
        metric_kind: &'static str,
        permit: InflightPermit,
    },
}

/// Bound on owner-connection ops concurrently in flight. This is a HARD
/// per-connection limit on the number of ops dispatched-but-not-yet-replied -
/// including long-polls (`ReadOutput`/`Wait`) that each pin a guest RPC. A
/// backpressure-aware owner (the real CLI is strictly sequential - one op,
/// await its reply, then the next) stays at 1-2 in flight and never approaches
/// this cap; a flooding/pipelining owner that exceeds it has its session closed
/// promptly (the reader never blocks acquiring a permit).
pub const EXEC_OWNER_INFLIGHT_CAP: usize = 64;

/// Bounded grace for the owner writer to flush its last resolved replies (e.g. a
/// final exit-status `Wait`) during teardown before the owner socket is
/// force-shut-down. A healthy writer exits in microseconds; this only bounds the
/// wait for a writer wedged on a blocking `send` to an owner that stopped
/// reading, after which the socket is shut down so the send fails and the writer
/// can exit (otherwise `join()` would hang and strand the owner thread + slot).
pub const EXEC_OWNER_WRITER_DRAIN_GRACE: Duration = Duration::from_millis(250);
const EXEC_OWNER_WRITER_DRAIN_POLL: Duration = Duration::from_millis(5);

/// A non-blocking counting semaphore bounding the owner connection's actual
/// concurrent in-flight ops. The earlier design only bounded the
/// reader→writer channel, but the worker immediately spawns each long-poll and
/// the writer immediately spawns each awaiter, so both channels drained as fast
/// as the reader filled them - the reader was never bounded and a pipelining
/// owner could open unbounded concurrent long-polls/guest RPCs. Here a permit
/// is taken just before an op is dispatched and HELD until its reply frame is
/// written (or the op is torn down), so the cap hard-bounds the number of ops
/// genuinely in flight. A permit is taken with the NON-BLOCKING
/// [`InflightSemaphore::try_acquire`]: a well-behaved (backpressure-aware) owner
/// stays far below the cap, and an owner that exceeds it has its session closed
/// promptly rather than the reader BLOCKING on a permit (which would delay
/// observing owner EOF/POLLHUP under saturation). The reader runs on a plain OS
/// thread (no tokio runtime), so a plain `Mutex<usize>` is used rather than
/// `tokio::sync::Semaphore`.
pub struct InflightSemaphore {
    available: std::sync::Mutex<usize>,
}

impl InflightSemaphore {
    fn new(permits: usize) -> Arc<Self> {
        Arc::new(Self {
            available: std::sync::Mutex::new(permits),
        })
    }

    /// Take a permit WITHOUT blocking. Returns `Some(permit)` iff one is free,
    /// else `None` (the cap is saturated). The returned guard releases the
    /// permit on drop (reply written, immediate error, or teardown). Never
    /// blocks: the reader must always be free to return to `read_frame` and
    /// observe owner EOF/POLLHUP. Mutex poison is recovered rather than
    /// propagated as a panic - the critical section only increments/decrements
    /// a counter and cannot leave broken invariants.
    fn try_acquire(self: &Arc<Self>) -> Option<InflightPermit> {
        let mut available = self
            .available
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if *available == 0 {
            return None;
        }
        *available -= 1;
        Some(InflightPermit {
            semaphore: Arc::clone(self),
        })
    }
}

/// RAII permit for [`InflightSemaphore`]. Releasing on drop covers every
/// teardown path (reply written, immediate error frame written, owner
/// disconnect, worker teardown dropping the reply oneshot).
pub struct InflightPermit {
    semaphore: Arc<InflightSemaphore>,
}

impl Drop for InflightPermit {
    fn drop(&mut self) {
        let mut available = self
            .semaphore
            .available
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        *available += 1;
    }
}

/// Parts produced by [`spawn_exec_owner_writer`]: the writer join handle, the
/// reader→writer item channel sender, and the shared in-flight semaphore.
pub type ExecOwnerWriterParts = (
    std::thread::JoinHandle<()>,
    tokio::sync::mpsc::Sender<ExecWriterItem>,
    Arc<InflightSemaphore>,
);

/// Spawn the owner-connection writer thread and return the channel + semaphore
/// the reader drives it with. Returns the OS-thread-spawn error to the caller
/// (instead of panicking) so a resource-exhausted host fails the session with a
/// typed internal error rather than crashing the handler.
pub fn spawn_exec_owner_writer(
    stream: &Arc<Socket>,
    metrics: &Arc<crate::metrics::Registry>,
) -> std::io::Result<ExecOwnerWriterParts> {
    let (item_tx, item_rx) = tokio::sync::mpsc::channel::<ExecWriterItem>(EXEC_OWNER_INFLIGHT_CAP);
    let inflight = InflightSemaphore::new(EXEC_OWNER_INFLIGHT_CAP);
    let writer_stream = Arc::clone(stream);
    let writer_metrics = Arc::clone(metrics);
    let writer = std::thread::Builder::new()
        .name("d2b-exec-writer".to_owned())
        .spawn(move || exec_owner_writer(writer_stream, writer_metrics, item_rx))?;
    Ok((writer, item_tx, inflight))
}

/// Emit the closed-allowlist observability signal for an owner connection that
/// exceeded the in-flight op cap and is being closed. A leak-safe metric
/// (closed `outcome`/`error_kind` labels - no vm/handle/uid/argv) plus a
/// rate-bounded structured log carrying only the constant cap. No wire frame is
/// written from the reader thread (the writer thread is the sole socket
/// writer).
fn signal_owner_inflight_cap_exceeded(metrics: &crate::metrics::Registry) {
    exec_metric_into(metrics, "op-error", "inflight-cap-exceeded");
    tracing::warn!(
        kind = "critical",
        subsystem = EXEC_SUBSYSTEM,
        error_kind = "inflight-cap-exceeded",
        cap = EXEC_OWNER_INFLIGHT_CAP,
        "guest-control-exec: owner connection exceeded the in-flight op cap; closing the session",
    );
}

/// Drive the owner connection's reader loop (this function runs on the owner
/// thread). Each frame is parsed into `(op_id, op)`, bound to `handle`, and
/// dispatched to the worker over `control_tx` WITHOUT waiting for the reply;
/// the reply receiver is forwarded to the writer thread, which matches replies
/// to ops by `op_id` and writes them out of order. A permit from `inflight` is
/// taken (NON-BLOCKING) before EACH op is queued and travels with it to the
/// writer. The reader NEVER blocks on a permit: a well-behaved owner stays far
/// below the cap, and an owner that exceeds `EXEC_OWNER_INFLIGHT_CAP` ops in
/// flight has its session closed through the single teardown path below
/// (after emitting an observability signal). Because the reader never blocks
/// acquiring a permit, owner EOF/POLLHUP is always observed promptly - even
/// when the cap is fully saturated by parked long-polls.
/// On reader EOF/POLLHUP (owner disconnect) or over-cap close the loop returns,
/// `control_tx` is dropped (tearing the worker down and cancelling any
/// in-flight long-poll), then the writer is joined.
pub fn run_exec_owner_io(
    stream: &Arc<Socket>,
    control_tx: tokio::sync::mpsc::Sender<exec_session::WorkerCommand>,
    item_tx: tokio::sync::mpsc::Sender<ExecWriterItem>,
    inflight: Arc<InflightSemaphore>,
    writer: std::thread::JoinHandle<()>,
    metrics: &Arc<crate::metrics::Registry>,
    handle: &str,
) {
    // EOF / POLLHUP / shutdown / any read error closes the connection and ends
    // the loop, triggering the teardown below.
    while let Ok(frame) = read_frame(stream.as_ref()) {
        let op_id = crate::wire::exec_op_id(&frame);
        let op = match crate::wire::parse_exec_op(&frame) {
            Ok((_, op)) => op,
            Err(error) => {
                // Take a permit even for an immediate error so a flood of
                // malformed frames is bounded by the same in-flight cap; it is
                // released once the error frame is written. Over-cap closes the
                // session (the reader never blocks).
                let Some(permit) = inflight.try_acquire() else {
                    signal_owner_inflight_cap_exceeded(metrics);
                    break;
                };
                if item_tx
                    .blocking_send(ExecWriterItem::Immediate {
                        op_id,
                        error: Box::new(error),
                        metric_kind: "protocol",
                        permit,
                    })
                    .is_err()
                {
                    break;
                }
                continue;
            }
        };
        // Bind every op to THIS session handle (peer-uid binding is implicit:
        // the handle was minted for this connection's admin peer).
        if exec_op_session(&op) != Some(handle) {
            let error = TypedError::GuestControlExecFailed {
                kind: crate::typed_error::GuestControlExecErrorKind::Protocol,
            };
            let Some(permit) = inflight.try_acquire() else {
                signal_owner_inflight_cap_exceeded(metrics);
                break;
            };
            if item_tx
                .blocking_send(ExecWriterItem::Immediate {
                    op_id,
                    error: Box::new(error),
                    metric_kind: "protocol",
                    permit,
                })
                .is_err()
            {
                break;
            }
            continue;
        }

        // Take the in-flight permit BEFORE handing the op to the worker. When
        // the cap is reached this does NOT block: it closes the session (a
        // backpressure-aware owner never reaches the cap), so the reader is
        // always free to observe owner EOF/POLLHUP promptly.
        let Some(permit) = inflight.try_acquire() else {
            signal_owner_inflight_cap_exceeded(metrics);
            break;
        };
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        let command = exec_session::WorkerCommand {
            op,
            reply: reply_tx,
        };
        if control_tx.blocking_send(command).is_err() {
            // Worker gone (terminal cleanup / teardown): close the connection.
            // `permit` drops here, releasing it.
            break;
        }
        if item_tx
            .blocking_send(ExecWriterItem::Pending {
                op_id,
                reply_rx,
                permit,
            })
            .is_err()
        {
            break;
        }
    }

    // Reader done. Drop `control_tx` FIRST so the worker returns and resolves
    // every pending reply oneshot (cancelling in-flight long-polls without
    // waiting for their deadline), THEN drop `item_tx` so the writer drains the
    // resolved replies and exits. Joining the writer guarantees no further
    // socket writes after this returns.
    drop(control_tx);
    drop(item_tx);
    // Give the writer a brief, bounded grace to flush its last resolved replies
    // and exit on its own, then force teardown: a misbehaving owner that stopped
    // reading and filled its socket receive buffer would wedge the writer's
    // blocking `send`, so `join()` would hang forever and strand the owner
    // thread + session slot. Shutting the owner socket down makes the wedged
    // send fail promptly so the writer can exit. A healthy writer finishes well
    // within the grace and never reaches the shutdown.
    let drain_deadline = Instant::now() + EXEC_OWNER_WRITER_DRAIN_GRACE;
    while !writer.is_finished() && Instant::now() < drain_deadline {
        std::thread::sleep(EXEC_OWNER_WRITER_DRAIN_POLL);
    }
    if !writer.is_finished() {
        let _ = nix::sys::socket::shutdown(
            stream.as_ref().as_raw_fd(),
            nix::sys::socket::Shutdown::Both,
        );
    }
    let _ = writer.join();
}

/// The owner-connection writer: a current-thread tokio runtime that awaits each
/// op's worker reply concurrently and writes op-id-tagged frames back to the
/// socket from a single drain task (so the socket has exactly one writer).
fn exec_owner_writer(
    stream: Arc<Socket>,
    metrics: Arc<crate::metrics::Registry>,
    mut item_rx: tokio::sync::mpsc::Receiver<ExecWriterItem>,
) {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(_) => return,
    };
    runtime.block_on(async move {
        // The frame channel carries the op's in-flight permit alongside the
        // frame so the permit is released only AFTER the reply is written to the
        // socket (in the drain task), which is what makes the cap bound ACTUAL
        // in-flight ops rather than just the reader→writer channel depth.
        let (frame_tx, mut frame_rx) =
            tokio::sync::mpsc::unbounded_channel::<(u64, ExecOwnerFrame, InflightPermit)>();
        let drain_stream = Arc::clone(&stream);
        let drain_metrics = Arc::clone(&metrics);
        let drain = tokio::spawn(async move {
            while let Some((op_id, frame, permit)) = frame_rx.recv().await {
                let value = match &frame {
                    ExecOwnerFrame::Response(response) => {
                        crate::wire::exec_response_with_id(op_id, response)
                    }
                    ExecOwnerFrame::Error { error, .. } => {
                        crate::wire::error_frame_with_id(op_id, error)
                    }
                };
                let write_result = write_json_frame(drain_stream.as_ref(), &value);
                if let ExecOwnerFrame::Error { metric_kind, .. } = &frame
                    && write_result.is_ok()
                {
                    exec_metric_into(&drain_metrics, "op-error", metric_kind);
                }
                // Release the in-flight permit only now that this op's reply has
                // left the writer (or failed to). Explicit for clarity; `permit`
                // would drop at the end of the iteration regardless.
                drop(permit);
                if write_result.is_err() {
                    break;
                }
            }
        });

        while let Some(item) = item_rx.recv().await {
            match item {
                ExecWriterItem::Pending {
                    op_id,
                    reply_rx,
                    permit,
                } => {
                    let frame_tx = frame_tx.clone();
                    tokio::spawn(async move {
                        let frame = match reply_rx.await {
                            Ok(Ok(response)) => ExecOwnerFrame::Response(Box::new(response)),
                            Ok(Err(op_error)) => {
                                let error = map_exec_op_error(op_error);
                                let metric_kind = exec_error_kind_label(&error);
                                ExecOwnerFrame::Error {
                                    error: Box::new(error),
                                    metric_kind,
                                }
                            }
                            // Worker dropped the reply (teardown). The owner is
                            // going away; emit nothing for this op. `permit`
                            // drops here, releasing it.
                            Err(_) => return,
                        };
                        let _ = frame_tx.send((op_id, frame, permit));
                    });
                }
                ExecWriterItem::Immediate {
                    op_id,
                    error,
                    metric_kind,
                    permit,
                } => {
                    let _ = frame_tx.send((
                        op_id,
                        ExecOwnerFrame::Error { error, metric_kind },
                        permit,
                    ));
                }
            }
        }
        // Reader closed the item channel. Drop this task's frame sender so the
        // drain finishes once the still-pending awaiters resolve (they resolve
        // promptly: worker teardown drops their reply oneshots).
        drop(frame_tx);
        let _ = drain.await;
    });
}

/// Owner-socket teardown seam for the terminal-cleanup reaper. Shutting
/// down the socket unblocks the owner reader (`read_frame` returns), which
/// releases the session slot. Idempotent: a second shutdown is a harmless
/// `ENOTCONN`.
pub struct SocketShutdownReaper {
    fd: RawFd,
}

impl SocketShutdownReaper {
    pub fn new(fd: RawFd) -> Self {
        Self { fd }
    }
}

impl exec_session::OwnerReaper for SocketShutdownReaper {
    fn reap(&self) {
        let _ = nix::sys::socket::shutdown(self.fd, nix::sys::socket::Shutdown::Both);
    }
}

#[cfg(test)]
#[cfg(test)]
mod exec_owner_io_tests {
    //! Hermetic coverage for the owner reader/writer: the owner
    //! connection dispatches frames to the worker WITHOUT blocking on each
    //! reply, so (a) an urgent control op is serviced while a long-poll is in
    //! flight (no head-of-line), and (b) owner disconnect tears the session
    //! down promptly (the in-flight long-poll is cancelled, not awaited).

    use super::{
        EXEC_OWNER_INFLIGHT_CAP, EXEC_OWNER_WRITER_DRAIN_GRACE, Socket, exec_session, read_frame,
        run_exec_owner_io, spawn_exec_owner_writer,
    };
    use crate::exec_support::EXEC_METRIC;
    use crate::unix_transport::write_frame;
    use d2b_contracts_control::public_wire::{
        ExecCloseArgs, ExecCloseResult, ExecControlResult, ExecOp, ExecOpResponse,
        ExecReadOutputResult, ExecSignalArgs, ExecWaitArgs,
    };
    use serde_json::{Value, json};
    use std::sync::Arc;
    use std::time::{Duration, Instant};
    use tokio::sync::oneshot;

    const HANDLE: &str = "h-test-owner";

    fn seqpacket_pair() -> (Socket, Socket) {
        use nix::sys::socket::{AddressFamily, SockFlag, SockType, socketpair};
        let (a, b) = socketpair(
            AddressFamily::Unix,
            SockType::SeqPacket,
            None,
            SockFlag::empty(),
        )
        .expect("seqpacket socketpair");
        (Socket::from(a), Socket::from(b))
    }

    fn exec_frame(op_id: u64, op: &ExecOp) -> Vec<u8> {
        let mut value = serde_json::to_value(op).expect("encode exec op");
        let object = value.as_object_mut().expect("exec op object");
        object.insert("type".to_owned(), json!("exec"));
        object.insert("opId".to_owned(), json!(op_id));
        serde_json::to_vec(&value).expect("serialize exec frame")
    }

    fn send_op(socket: &Socket, op_id: u64, op: &ExecOp) {
        write_frame(socket, &exec_frame(op_id, op)).expect("client sends exec frame");
    }

    fn recv_reply(socket: &Socket) -> Value {
        let bytes = read_frame(socket).expect("client reads reply");
        serde_json::from_slice(&bytes).expect("reply is JSON")
    }

    /// A fake worker that replies to fast control ops immediately but STASHES a
    /// long-poll (`Wait`/`ReadOutput`) reply sender so the poll stays in flight.
    /// On channel close (owner teardown) every stashed reply sender is dropped,
    /// modelling the production worker dropping its in-flight oneshots.
    fn spawn_fake_worker(
        mut control_rx: tokio::sync::mpsc::Receiver<exec_session::WorkerCommand>,
        longpoll_seen: std::sync::mpsc::Sender<()>,
    ) -> std::thread::JoinHandle<()> {
        std::thread::spawn(move || {
            let mut stashed: Vec<
                oneshot::Sender<Result<ExecOpResponse, exec_session::ExecOpError>>,
            > = Vec::new();
            while let Some(exec_session::WorkerCommand { op, reply }) = control_rx.blocking_recv() {
                match op {
                    ExecOp::Wait(_) | ExecOp::ReadOutput(_) => {
                        stashed.push(reply);
                        let _ = longpoll_seen.send(());
                    }
                    ExecOp::Signal(_) => {
                        let _ = reply.send(Ok(ExecOpResponse::Signal(ExecControlResult {
                            delivered: true,
                        })));
                    }
                    ExecOp::Close(_) => {
                        let _ = reply.send(Ok(ExecOpResponse::Close(ExecCloseResult {
                            stdin_closed: true,
                        })));
                    }
                    _ => {
                        let _ = reply.send(Err(exec_session::ExecOpError::Protocol));
                    }
                }
            }
            // Owner teardown: drop the stashed long-poll reply senders so the
            // writer's awaiters resolve `Err` (the poll is cancelled, never
            // awaited to its deadline).
            drop(stashed);
        })
    }

    fn wait_op() -> ExecOp {
        ExecOp::Wait(ExecWaitArgs {
            session: HANDLE.to_owned(),
            timeout_ms: 60_000,
        })
    }

    fn signal_op() -> ExecOp {
        ExecOp::Signal(ExecSignalArgs {
            session: HANDLE.to_owned(),
            signo: 2,
            op_id: 0,
        })
    }

    fn close_op() -> ExecOp {
        ExecOp::Close(ExecCloseArgs {
            session: HANDLE.to_owned(),
        })
    }

    #[test]
    fn control_op_is_serviced_while_a_long_poll_is_in_flight() {
        let (daemon, client) = seqpacket_pair();
        let daemon = Arc::new(daemon);
        let metrics = Arc::new(crate::metrics::Registry::new());
        let (control_tx, control_rx) = tokio::sync::mpsc::channel(16);
        let (seen_tx, seen_rx) = std::sync::mpsc::channel();
        let worker = spawn_fake_worker(control_rx, seen_tx);

        let io_daemon = Arc::clone(&daemon);
        let io_metrics = Arc::clone(&metrics);
        let io = std::thread::spawn(move || {
            let (writer, item_tx, inflight) =
                spawn_exec_owner_writer(&io_daemon, &io_metrics).expect("writer thread spawns");
            run_exec_owner_io(
                &io_daemon,
                control_tx,
                item_tx,
                inflight,
                writer,
                &io_metrics,
                HANDLE,
            );
        });

        // A normal op completes (owner-open + unrelated request proceeds).
        send_op(&client, 1, &close_op());
        let close_reply = recv_reply(&client);
        assert_eq!(close_reply["type"], "execResponse");
        assert_eq!(close_reply["opId"], 1);
        assert_eq!(close_reply["op"], "close");

        // Park a long-poll, then send an urgent control op. The control reply
        // must come back (out of order, by op-id) BEFORE the parked poll - proof
        // the owner socket read is not serialized behind the long-poll reply.
        send_op(&client, 10, &wait_op());
        seen_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("worker observed the parked long-poll");
        send_op(&client, 11, &signal_op());
        let signal_reply = recv_reply(&client);
        assert_eq!(
            signal_reply["opId"], 11,
            "control reply must be serviced first"
        );
        assert_eq!(signal_reply["op"], "signal");

        // Teardown: closing the client unblocks the reader; the parked poll is
        // cancelled (never replied), and the io thread returns promptly.
        drop(client);
        let start = Instant::now();
        io.join().expect("owner io thread joins");
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "owner io did not tear down promptly after disconnect"
        );
        worker.join().expect("fake worker joins");
    }

    #[test]
    fn disconnect_during_long_poll_tears_down_without_awaiting_the_deadline() {
        let (daemon, client) = seqpacket_pair();
        let daemon = Arc::new(daemon);
        let metrics = Arc::new(crate::metrics::Registry::new());
        let (control_tx, control_rx) = tokio::sync::mpsc::channel(16);
        let (seen_tx, seen_rx) = std::sync::mpsc::channel();
        let worker = spawn_fake_worker(control_rx, seen_tx);

        let io_daemon = Arc::clone(&daemon);
        let io_metrics = Arc::clone(&metrics);
        let io = std::thread::spawn(move || {
            let (writer, item_tx, inflight) =
                spawn_exec_owner_writer(&io_daemon, &io_metrics).expect("writer thread spawns");
            run_exec_owner_io(
                &io_daemon,
                control_tx,
                item_tx,
                inflight,
                writer,
                &io_metrics,
                HANDLE,
            );
        });

        // Park a 60s long-poll, then disconnect. Teardown must NOT wait for the
        // poll's deadline.
        send_op(&client, 10, &wait_op());
        seen_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("worker observed the parked long-poll");
        drop(client);

        let start = Instant::now();
        io.join().expect("owner io thread joins");
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "teardown blocked on the long-poll deadline"
        );
        worker.join().expect("fake worker joins");
    }

    /// The in-flight cap must bound the number of ops ACTUALLY
    /// dispatched-but-unanswered (each pins a guest RPC), not merely the depth
    /// of a channel the worker/writer drain as fast as the reader fills it. A
    /// pipelining owner that floods `cap + N` long-polls must see at most `cap`
    /// reach the worker concurrently; the `(cap + 1)`-th op finds NO free permit
    /// and - crucially - the reader does NOT block on it. Instead the session is
    /// closed PROMPTLY (the over-cap observability signal is emitted and the
    /// reader returns through the single teardown path). This proves both that
    /// the cap hard-bounds concurrent in-flight work AND that the reader never
    /// parks acquiring a permit, so owner EOF/POLLHUP is always observable.
    #[test]
    fn concurrent_inflight_ops_are_bounded_by_the_cap() {
        use std::sync::Mutex;

        let cap = EXEC_OWNER_INFLIGHT_CAP;
        let (daemon, client) = seqpacket_pair();
        let daemon = Arc::new(daemon);
        let metrics = Arc::new(crate::metrics::Registry::new());
        let (control_tx, mut control_rx) = tokio::sync::mpsc::channel(16);
        let (seen_tx, seen_rx) = std::sync::mpsc::channel::<()>();

        // A counting fake worker: every long-poll reply sender is parked in a
        // shared stash (never answered) so the op holds its in-flight permit;
        // each receipt signals `seen_tx`. The stash is also held by the test so
        // it can release the parked senders to let the writer awaiters resolve
        // for a clean teardown.
        type Stash =
            Arc<Mutex<Vec<oneshot::Sender<Result<ExecOpResponse, exec_session::ExecOpError>>>>>;
        let stash: Stash = Arc::new(Mutex::new(Vec::new()));
        let worker_stash = Arc::clone(&stash);
        let worker = std::thread::spawn(move || {
            while let Some(exec_session::WorkerCommand { op, reply }) = control_rx.blocking_recv() {
                match op {
                    ExecOp::Wait(_) | ExecOp::ReadOutput(_) => {
                        worker_stash.lock().expect("stash lock").push(reply);
                        let _ = seen_tx.send(());
                    }
                    _ => {
                        let _ = reply.send(Err(exec_session::ExecOpError::Protocol));
                    }
                }
            }
        });

        let io_daemon = Arc::clone(&daemon);
        let io_metrics = Arc::clone(&metrics);
        let io = std::thread::spawn(move || {
            let (writer, item_tx, inflight) =
                spawn_exec_owner_writer(&io_daemon, &io_metrics).expect("writer thread spawns");
            run_exec_owner_io(
                &io_daemon,
                control_tx,
                item_tx,
                inflight,
                writer,
                &io_metrics,
                HANDLE,
            );
        });

        // Pipeline well beyond the cap. The frames are tiny and fit in the
        // socket buffer, so these client writes do not block even though the
        // reader closes the session after the cap is exceeded.
        let total = cap + 8;
        for op_id in 0..total {
            send_op(&client, op_id as u64, &wait_op());
        }

        // Exactly `cap` long-polls reach the worker. The `(cap + 1)`-th op finds
        // no permit and the reader closes the session rather than dispatching it
        // - so no more than `cap` are ever seen.
        for _ in 0..cap {
            seen_rx
                .recv_timeout(Duration::from_secs(5))
                .expect("worker observes parked long-polls up to the cap");
        }
        assert!(
            seen_rx.recv_timeout(Duration::from_millis(750)).is_err(),
            "more than the cap of {cap} ops were dispatched concurrently to the worker",
        );

        // The over-cap close path emits the closed-allowlist observability
        // signal. Poll briefly: the reader breaks just after the cap-th
        // dispatch, so the metric appears within a short window.
        let mut saw_signal = false;
        for _ in 0..50 {
            if metrics.render().lines().any(|line| {
                line.starts_with(EXEC_METRIC)
                    && line.contains("outcome=\"op-error\"")
                    && line.contains("error_kind=\"inflight-cap-exceeded\"")
            }) {
                saw_signal = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(
            saw_signal,
            "over-cap close must emit the inflight-cap-exceeded exec metric",
        );

        // The reader did NOT block on the over-cap permit: the session is
        // already closing on its own. Release the parked replies so the held
        // permits free and the writer awaiters resolve, then the io thread joins
        // PROMPTLY - without the test ever dropping the client.
        stash.lock().expect("stash lock").clear();
        let start = Instant::now();
        io.join().expect("owner io thread joins");
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "owner io did not close the session promptly after exceeding the cap",
        );
        drop(client);
        worker.join().expect("fake worker joins");
    }

    /// Prompt teardown under saturation: when the in-flight cap is FULLY held
    /// (every permit taken by a parked long-poll), an owner disconnect must
    /// still tear the session down promptly. The reader is parked in
    /// `read_frame` (never in a permit acquisition), so owner EOF is observed at
    /// once: `control_tx` is dropped, the worker cancels its parked polls, and
    /// the io thread returns without waiting for any poll deadline.
    #[test]
    fn disconnect_while_inflight_cap_saturated_tears_down_promptly() {
        let cap = EXEC_OWNER_INFLIGHT_CAP;
        let (daemon, client) = seqpacket_pair();
        let daemon = Arc::new(daemon);
        let metrics = Arc::new(crate::metrics::Registry::new());
        let (control_tx, mut control_rx) = tokio::sync::mpsc::channel(16);
        let (seen_tx, seen_rx) = std::sync::mpsc::channel::<()>();

        // Parked long-poll senders live in a stash owned by the worker thread;
        // dropping them on worker-loop exit (owner teardown) models the
        // production worker dropping its in-flight oneshots so the polls are
        // cancelled rather than awaited.
        let worker = std::thread::spawn(move || {
            let mut stashed: Vec<
                oneshot::Sender<Result<ExecOpResponse, exec_session::ExecOpError>>,
            > = Vec::new();
            while let Some(exec_session::WorkerCommand { op, reply }) = control_rx.blocking_recv() {
                match op {
                    ExecOp::Wait(_) | ExecOp::ReadOutput(_) => {
                        stashed.push(reply);
                        let _ = seen_tx.send(());
                    }
                    _ => {
                        let _ = reply.send(Err(exec_session::ExecOpError::Protocol));
                    }
                }
            }
            drop(stashed);
        });

        let io_daemon = Arc::clone(&daemon);
        let io_metrics = Arc::clone(&metrics);
        let io = std::thread::spawn(move || {
            let (writer, item_tx, inflight) =
                spawn_exec_owner_writer(&io_daemon, &io_metrics).expect("writer thread spawns");
            run_exec_owner_io(
                &io_daemon,
                control_tx,
                item_tx,
                inflight,
                writer,
                &io_metrics,
                HANDLE,
            );
        });

        // Saturate EXACTLY to the cap (do not exceed it): all `cap` permits are
        // taken by parked long-polls, and the reader is now parked in
        // `read_frame` awaiting the next frame.
        for op_id in 0..cap {
            send_op(&client, op_id as u64, &wait_op());
        }
        for _ in 0..cap {
            seen_rx
                .recv_timeout(Duration::from_secs(5))
                .expect("worker observes parked long-polls up to the cap");
        }

        // Disconnect while fully saturated. The reader (parked in read_frame,
        // NOT in a permit acquisition) observes EOF immediately and tears down.
        drop(client);
        let start = Instant::now();
        io.join().expect("owner io thread joins");
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "saturated owner disconnect did not tear down promptly (reader blocked?)",
        );
        let _ = &metrics;
        worker.join().expect("fake worker joins");
    }

    #[test]
    fn over_cap_teardown_completes_when_owner_stops_reading() {
        // A misbehaving owner that pipelines past the cap while NEVER reading
        // replies fills the daemon's owner-socket send buffer, wedging the
        // writer's blocking `send`. Teardown must still complete: the bounded
        // drain grace elapses, the owner socket is shut down to unblock the
        // wedged send, and the io thread joins - instead of hanging forever and
        // stranding the owner thread + session slot.
        let cap = EXEC_OWNER_INFLIGHT_CAP;
        let (daemon, client) = seqpacket_pair();
        // Squeeze both buffers so a couple of unread ~1 KiB replies fill the pipe
        // and wedge the writer well before the cap is reached.
        let _ = daemon.set_send_buffer_size(1024);
        let _ = client.set_recv_buffer_size(1024);
        let daemon = Arc::new(daemon);
        let metrics = Arc::new(crate::metrics::Registry::new());
        let (control_tx, mut control_rx) = tokio::sync::mpsc::channel(16);

        // Resolve EVERY long-poll immediately with a ~1 KiB reply so the writer
        // actively sends; with the owner never reading, the send buffer backs up
        // and the writer's blocking `send` wedges.
        let worker = std::thread::spawn(move || {
            let payload = "x".repeat(1024);
            while let Some(exec_session::WorkerCommand { op, reply }) = control_rx.blocking_recv() {
                match op {
                    ExecOp::Wait(_) | ExecOp::ReadOutput(_) => {
                        let _ = reply.send(Ok(ExecOpResponse::ReadOutput(ExecReadOutputResult {
                            data_base64: payload.clone(),
                            next_offset: 0,
                            eof: false,
                            dropped_bytes: 0,
                            truncated: false,
                            timed_out: false,
                        })));
                    }
                    _ => {
                        let _ = reply.send(Err(exec_session::ExecOpError::Protocol));
                    }
                }
            }
        });

        let io_daemon = Arc::clone(&daemon);
        let io_metrics = Arc::clone(&metrics);
        let started = Instant::now();
        let io = std::thread::spawn(move || {
            let (writer, item_tx, inflight) =
                spawn_exec_owner_writer(&io_daemon, &io_metrics).expect("writer spawns");
            run_exec_owner_io(
                &io_daemon,
                control_tx,
                item_tx,
                inflight,
                writer,
                &io_metrics,
                HANDLE,
            );
        });

        // Pipeline far past the cap WITHOUT ever reading a reply, from a thread so
        // a backed-up client send (once the reader over-caps and stops reading)
        // cannot wedge the test itself. Hold the client socket OPEN so teardown is
        // an over-cap close, NOT an EOF (which would unblock the writer for free).
        let sender = std::thread::spawn(move || {
            for op_id in 0..(cap * 2) {
                if write_frame(&client, &exec_frame(op_id as u64, &wait_op())).is_err() {
                    break;
                }
            }
            client
        });

        // The io thread must JOIN - proving teardown did not hang on the wedged
        // send. Poll up to 10s (the bounded grace is sub-second).
        let mut joined = false;
        for _ in 0..400 {
            if io.is_finished() {
                joined = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        assert!(
            joined,
            "owner teardown hung: the wedged writer's send was never unblocked",
        );
        io.join().expect("io thread joins");

        // The teardown waited the full bounded grace, proving the writer was
        // genuinely wedged (a healthy writer exits in microseconds, far under the
        // grace) and the shutdown path - not a free EOF - released it.
        assert!(
            started.elapsed() >= EXEC_OWNER_WRITER_DRAIN_GRACE,
            "expected the bounded drain grace to elapse on a wedged writer; got {:?}",
            started.elapsed(),
        );

        let client = sender.join().expect("sender thread joins");
        drop(client);
        worker.join().expect("fake worker joins");
    }
}
