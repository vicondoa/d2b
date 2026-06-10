# Guest control exec I/O: chunked stdio ttRPC design

This document specifies the bounded exec I/O protocol selected for the
guest-control design: ttRPC unary calls for lifecycle and
Kata-style chunked stdio RPCs for stdin/stdout/stderr. It is the design
follow-up to [ADR 0026](../adr/0026-guest-control-plane-over-vsock.md)
and the [guest-control feasibility dossier](../adr/guest-control-feasibility-dossier.md).

## Decision summary

Use one authenticated ttRPC connection per host-to-guest control session.
Exec lifecycle, inspection, wait, signal, resize, and stdio movement are
all unary ttRPC calls. Stdio is not represented as raw ttRPC streams.
Instead, each caller transfers bounded chunks with explicit cursors:

- `WriteStdin` appends one bounded stdin chunk if the guest has capacity.
- `ReadOutput` returns bounded stdout or stderr chunks at requested byte
  offsets from server-owned append-only per-exec logs.
- `CloseStdin` half-closes stdin exactly once.
- `TtyWinResize` and `Signal` are ordered control events on the exec's
  control sequence.
- `Wait` long-polls for terminal process state, and `Inspect` returns the
  current state without consuming output.

This follows Kata Agent's prior-art shape (`WriteStdin`, `ReadStdout`,
`ReadStderr`, `CloseStdin`, `TtyWinResize`, `SignalProcess`, and
`WaitProcess`) while adding nixling-specific offset cursors, memory
budgets, long-poll behavior, detached-log retention, and typed
slow-consumer cancellation.

## Service surface

The committed protobuf service source is
[`packages/nixling-ipc/proto/guest_control.proto`](../../packages/nixling-ipc/proto/guest_control.proto).
Lifecycle requests carry `vm_id`, `request_id`, and negotiated
`protocol_version` in `RequestMetadata`; exec-specific requests also carry
`exec_id` and the Hello-returned `guest_boot_id` in `ExecRequestMetadata` so
VM reboot or guestd restart changes reject stale sessions instead of
reattaching to an unrelated reused exec id.

```protobuf
service GuestControl {
  rpc Hello(HelloRequest) returns (HelloResponse);
  rpc Capabilities(CapabilitiesRequest) returns (CapabilitiesResponse);
  rpc Health(HealthRequest) returns (HealthResponse);

  rpc ExecCreate(ExecCreateRequest) returns (ExecCreateResponse);
  rpc ExecInspect(ExecInspectRequest) returns (ExecInspectResponse);
  rpc ExecWait(ExecWaitRequest) returns (ExecWaitResponse);
  rpc ExecLogs(ExecLogsRequest) returns (ExecLogsResponse);

  rpc WriteStdin(WriteStdinRequest) returns (WriteStdinResponse);
  rpc ReadOutput(ReadOutputRequest) returns (ReadOutputResponse);
  rpc CloseStdin(CloseStdinRequest) returns (CloseStdinResponse);
  rpc TtyWinResize(TtyWinResizeRequest) returns (ControlAck);
  rpc ExecSignal(ExecSignalRequest) returns (ControlAck);
  rpc ExecCancel(ExecCancelRequest) returns (ControlAck);
}
```

`ExecLogs` is a convenience wrapper for old/detached execs. Attached
clients use `ReadOutput(stream=stdout|stderr)` directly so polling state,
cursors, and backpressure are identical for attached and detached sessions.

## Exec lifecycle messages

### `ExecCreate`

Request fields:

- `argv`: repeated string, already split by the CLI after `--`.
- `user`: optional validated guest user selector.
- `cwd`: optional absolute path.
- `env`: repeated key/value entries after host-side policy filtering.
- `tty`: bool. When true, stdout and stderr are PTY-merged into stdout;
  `ReadOutput(stream=stderr)` returns `tty-stderr-unavailable`.
- `stdin_open`: bool. Defaults false unless CLI used `--interactive`.
- `detached`: bool. Detached execs persist bounded logs after caller
  disconnect.
- `initial_terminal_size`: optional `{rows, cols}` required for `tty`.
- `output_policy`: `{max_chunk_bytes, max_stdout_log_bytes,
  max_stderr_log_bytes, slow_consumer_timeout_ms, wait_timeout_ms}`. The
  server clamps each value to the VM capability maximum.

Response fields:

- `exec_id`.
- `created_at_monotonic_ns` and `control_seq` initially `0`.
- `stdout_cursor` and `stderr_cursor` initially `0`.
- `effective_limits`: the clamped protocol limits.
- `state`: `created` or typed failure.

`ExecCreate` starts the guest process only after the session object,
stdio pipes or PTY, log buffers, and event queue have been allocated.
For TTY execs, the PTY is opened with the initial geometry before spawn,
so the child observes the correct size from its first instruction.

### `ExecInspect`

Returns non-consuming state:

- state enum: `created`, `running`, `exited`, `signaled`, `cancelled`,
  `slow-consumer-cancelled`, `protocol-error`, `lost-guestd`, `reaped`;
- `exit_code` or `signal` when terminal;
- `stdin_state`: `open`, `closing`, `closed`, `closed-by-process`,
  `rejected-not-interactive`;
- `stdout_start_offset`, `stdout_end_offset`, `stderr_start_offset`,
  `stderr_end_offset`;
- `stdout_dropped_bytes`, `stderr_dropped_bytes`,
  `stdout_truncated_for_retention`, and
  `stderr_truncated_for_retention`;
- `last_control_seq`, `state_generation`, and any bounded `error`.

The start/end offsets define the valid read window. If a caller asks for
an offset below `*_start_offset`, the server returns `offset-expired`
with the new start offset rather than silently serving newer bytes.

Terminal status has two forms:

- `recorded_terminal_status`: the child exit/signal/cancel result guestd
  has observed internally. This is not returned to host callers until it
  becomes visible.
- `visible_terminal_status`: populated only after every output byte guestd
  read before terminal observation is either retained in the stream/log
  cursor window, already delivered/acknowledged by the attached reader, or
  explicitly represented by detached `start_offset`/`dropped_bytes`
  accounting. Until then, `ExecInspect` and `ExecWait` report a
  non-terminal state with `visible_terminal_status = null` and the stream
  end offsets needed to discover the output first.

Detached execs may record terminal status before a client reads retained
logs, but `visible_terminal_status` still requires retained-log cursor or
explicit dropped-byte metadata that makes any preceding output discoverable
through `ExecLogs`. Attached CLI flows use `visible_terminal_status` for
process exit and local raw-mode cleanup once their read cursor has consumed
or can still read the preceding output.

If guestd loses pre-terminal output without delivering it to the attached
reader or representing it through detached `start_offset`/`dropped_bytes`
accounting, the exec transitions to terminal `protocol-error` with bounded
kind `output-lost`. This wakes waiters/readers and lets the CLI restore
local terminal state with a typed nixling error instead of polling
forever.

### `ExecWait`

Request fields:

- `timeout_ms`: clamped by `effective_limits.wait_timeout_ms`.
- `known_state_generation`: optional value from a previous inspect/wait.

Response fields:

- current terminal/non-terminal state;
- `visible_terminal_status`, populated only after preceding output is
  available through the returned stdout/stderr offset window;
- `state_generation`;
- current stdout/stderr offset window.

If the process is still running, or terminal status is recorded but not
yet visible because preceding output has not drained into the retained
cursor window, `ExecWait` holds the unary request until the timeout
expires or state/visibility changes. Timeout is not an error; it returns
the current state with `timed_out: true`. Client-side RPC cancellation
cancels only that wait call, not the exec.

## Stdio message shapes

### `WriteStdin`

Request fields:

- `request_id`: idempotency key for this stdin write.
- `offset`: caller's stdin byte offset for idempotency.
- `data`: bytes, length `1..max_chunk_bytes`.
- `close_after`: required bool for atomic final chunk plus half-close; callers
  send `false` when they only want to write bytes.
- `client_deadline_ms`: optional deadline for waiting on bounded guestd
  stdin-queue capacity.

Response fields:

- `accepted_offset` and `accepted_len`;
- `next_offset`;
- `stdin_state`;
- `blocked_ms`;
- `disposition`: `accepted`, `duplicate`, or `rejected`;
- `error`, populated for typed retryable failures such as
  `stdin-offset-mismatch`, `stdin-backpressure`, `request-id-conflict`, and
  `stdin-byte-budget-exhausted`.

Rules:

1. The ttRPC receiver enforces its fixed 4 MiB frame limit before
   protobuf decode, and `data.len()` must not exceed
   `effective_limits.max_chunk_bytes` after decode. Because
   `ttrpc-rust` 0.9.x allocates frames up to that fixed transport limit
   before handler admission, the decoded request is admitted only while
   holding the per-connection decoded-byte budget and the per-exec stdin
   write permit described in
   [Receive limits and concurrency proof](#receive-limits-and-concurrency-proof).
2. `offset` must equal the server's next expected stdin offset, unless it
   exactly repeats the previous accepted chunk and `request_id` matches a
   dedupe entry. Otherwise the server returns `stdin-offset-mismatch`
   with the expected offset.
3. `WriteStdin` is all-or-nothing at the RPC boundary. Guestd either
   copies the entire chunk into its bounded stdin queue and returns
   `accepted_len = data.len()`, or returns `stdin-backpressure` with
   `accepted_len = 0` and leaves `next_offset` unchanged.
4. Guestd's child-stdin writer drains that queue into the Linux pipe or
   PTY and tracks partial kernel writes internally. A partial pipe/PTY
   write is never exposed as a retryable offset-stable failure, so a
   retry cannot duplicate bytes already delivered to the child.
5. If the bounded queue cannot accept the whole chunk, the server waits
   only until `min(client_deadline_ms, stdin_write_deadline_ms)`. On
   expiry it returns `stdin-backpressure` before accepting any bytes.
6. If stdin is already closed, the call returns `stdin-closed` and the
   expected offset.
7. In non-interactive execs, all writes return `stdin-not-open`.
8. `close_after` commits atomically with the accepted bytes. The writer
   drains the accepted bytes first, then applies the same endpoint-specific
   close semantics as `CloseStdin`: pipe-backed execs close child stdin
   after the queue drains, while TTY execs mark protocol input closed
   without closing the PTY master/writer or stopping output reads. If the
   chunk cannot be accepted, stdin remains open and the caller retries or
   calls `CloseStdin`.

### `CloseStdin`

Request fields:

- `request_id`: idempotency key for the close operation.
- `offset`: the caller's final stdin offset.

Response fields:

- `stdin_state`, `final_offset`, and `disposition`;
- `error`, populated for typed failures such as `stdin-closed`,
  `request-id-conflict`, or `stdin-offset-mismatch`.

Rules:

- The close is accepted only when `offset` equals the next expected stdin
  offset.
- Close is idempotent only for the same retained `request_id` and final
  offset. Reusing the same `request_id` with a different offset returns
  `request-id-conflict`; a different `request_id` after stdin is already
  closed returns `stdin-closed`.
- For pipe-backed non-TTY execs, the server schedules EOF and closes the
  child's stdin fd only after all already accepted stdin bytes through the
  final offset have drained from the bounded queue.
- For TTY execs, close means host input is closed; it does not synthesize
  Ctrl-D. If the CLI wants Ctrl-D semantics it sends the terminal byte
  through `WriteStdin` before close.
- TTY close is protocol-side only: guestd marks future host input writes
  rejected, but it does not close the PTY master, drop the writer handle,
  send SIGHUP, or stop the output reader. Output already buffered or
  produced later by the foreground program remains readable until PTY
  output EOF.
- A child closing its read side transitions to `closed-by-process`; later
  writes get `stdin-closed-by-process`.

### `ReadOutput`

Request fields:

- `offset`: absolute byte offset in the stream log.
- `stream`: `stdout` or `stderr`; TTY execs reject `stderr` with
  `tty-stderr-unavailable`.
- `max_len`: requested maximum bytes, `1..max_chunk_bytes`; larger or
  zero-length requests fail with a typed protocol error.
- `wait`: bool.
- `timeout_ms`: long-poll duration if `wait` is true.

Response fields:

- `stream`: `stdout` or `stderr`;
- `offset`: the request offset served;
- `data`: bytes, length `0..max_len`;
- `next_offset`;
- `start_offset` and `end_offset` after the read;
- `eof`: true only when process output for that stream is complete and
  `next_offset == end_offset`;
- `truncated`: true if any older bytes were dropped by retention;
- `timed_out`: true for long-poll timeout with no bytes;
- `error`, populated for typed failures such as `offset-expired`,
  `offset-in-future`, `output-lost`, or `tty-stderr-unavailable`.

Rules:

1. `max_len` is validated before allocation and must be non-zero and at
   most `max_chunk_bytes` after clamping.
2. If `offset < start_offset`, the server returns `offset-expired` with
   `start_offset`; the client decides whether to fail the command or
   resume from the retained prefix boundary for log viewing.
3. If `offset > end_offset`, the server returns `offset-in-future`.
4. If data is available at `offset`, return immediately with up to
   `max_len` bytes.
5. If no data is available and EOF is not reached, `wait=true` holds the
   unary call until bytes arrive, EOF arrives, process state changes, or
   timeout. Timeout returns an empty successful response with
   `timed_out: true`.
6. EOF is sticky and idempotent. Reads at `end_offset` after EOF return
   empty data with `eof: true`.
7. For PTY output, Linux may surface slave closure as `EIO` on master
   reads and readiness APIs may report hangup. Guestd treats those as the
   PTY-output EOF edge only after the output reader has appended every
   byte returned before the `EIO`/hangup. `ReadOutput(stdout)` must therefore
   report `eof: true` only when `next_offset == end_offset` after that
   drain; it must not translate the first hangup into premature EOF or
   discard buffered tail bytes.

## Cursors, offsets, and idempotency

All stdin/stdout/stderr offsets are unsigned 64-bit absolute byte offsets
per exec and per stream. They never wrap during a retained exec; reaching
`u64::MAX` is a fatal `offset-exhausted` protocol error that cancels the
exec. The server maintains:

- `stdin_next_offset`: next expected `WriteStdin`/`CloseStdin` offset;
- `stdout_start_offset` / `stdout_end_offset`;
- `stderr_start_offset` / `stderr_end_offset`;
- a bounded `request_id` dedupe ring for recently accepted stdin writes,
  close, signal, resize, and cancel operations.

Read calls are naturally idempotent because offsets address immutable
log bytes. Write/control calls are idempotent only for the same
`request_id` and same payload hash while their dedupe entry is retained.
A duplicate with mismatched payload is `request-id-conflict`.

## Chunk and buffer limits

Default design limits:

| Limit | Default | Hard maximum | Notes |
| --- | ---: | ---: | --- |
| `max_chunk_bytes` | 64 KiB | 1 MiB | Application chunk limit; larger decoded chunks are rejected before session-buffer copy. |
| ttRPC frame cap | 4 MiB | 4 MiB | Fixed by `ttrpc-rust` 0.9.x before handler/protobuf admission; lowering it requires a wrapper or patch and a new proof. |
| chunked-stdio connections per VM | 4 | 4 | Bounds cross-connection decoded stdin pressure. |
| stdin in-flight/queue per exec | 1 chunk | 1 MiB | One decoded request may feed one bounded guestd stdin queue; pipe/PTY partial writes are tracked behind that queue. |
| decoded WriteStdin bytes per connection | 16 MiB | 16 MiB | Four concurrent ttRPC frames at the fixed 4 MiB cap; effective chunks above 1 MiB are rejected before session-buffer copy. |
| concurrent WriteStdin handlers per connection | 4 | 4 | Bounds protobuf `bytes` allocations even for malicious fan-in. |
| stdout live buffer per stream | 1 MiB | 8 MiB | Attached sessions; producer blocks before exceeding it. |
| stderr live buffer per stream | 1 MiB | 8 MiB | Not used for TTY execs. |
| detached stdout log | 16 MiB | 128 MiB | Ring-retained by offset with truncation accounting. |
| detached stderr log | 16 MiB | 128 MiB | Separate in non-TTY mode. |
| long-poll read timeout | 100 ms | 1 s | CLI can immediately re-poll for interactive use. |
| slow-consumer grace | 30 s | 5 min | Must satisfy the slow-consumer proof. |
| concurrent exec sessions per VM | 32 | 256 | New sessions fail before spawn with `exec-capacity-exceeded`. |
| attached exec sessions per VM | 8 | 64 | New attaches fail without affecting detached execs. |
| pending `ReadOutput(stdout)` waits | 1 per exec/connection, 64 per VM | 512 per VM | Duplicate waits are superseded or rejected. |
| pending `ReadOutput(stderr)` waits | 1 per exec/connection, 64 per VM | 512 per VM | TTY mode always returns `tty-stderr-unavailable`. |
| pending `ExecWait` calls | 1 per exec/connection, 64 per VM | 512 per VM | Duplicate waits are superseded or rejected. |
| RPC rate budget | 200/s per connection, 1000/s per VM burst | policy-defined | Excess calls return `rate-limited` with bounded retry-after. |

The hard maximums are capability values negotiated at `Hello` time and
may be lowered by VM policy. Raising them requires updating tests and
memory-budget documentation.

Proto3 presence is explicit where absence changes semantics:
`ExecCreateRequest.user`, `ExecCreateRequest.cwd`,
`ExecCreateResponse.exec_id`, `ExecWaitRequest.known_state_generation`,
`WriteStdinRequest.client_deadline_ms`, and
`GuestControlError.retry_after_ms` are `optional` in the protobuf source and
nullable in the generated JSON schema. Plain scalar zero/empty values are not
used as implicit absence sentinels for those fields.

## Receive limits and concurrency proof

The selected invariant is **bounded post-decode allocation**, with a
transport prefilter where ttRPC exposes one. A protobuf `bytes` field can
be allocated by the generated decoder before the `WriteStdin` handler
examines `data.len()`, so the design does not claim that every
`max_chunk_bytes + 1` request is rejected before all allocation. Instead,
the receiver enforces these gates, in order:

1. Use `ttrpc-rust` 0.9.x's fixed `MESSAGE_LENGTH_MAX = 4 MiB` as the
   selected transport pre-decode bound. Messages larger than that are
   rejected by ttRPC framing; messages at or below that bound may allocate
   a ttRPC payload buffer before handler/protobuf admission. Lowering this
   bound requires a pre-ttRPC frame limiter, patched/configurable ttRPC
   transport, and a new proof.
2. Limit chunked-stdio decode/handler concurrency per connection to four
   `WriteStdin` calls. With the transport cap above, a malicious client
   can force at most `4 * 4 MiB = 16 MiB` decoded request bytes per
   connection before application admission runs.
3. At handler entry, charge `data.len()` against a per-connection
   weighted semaphore with a 16 MiB budget. The permit is acquired before
   hashing for idempotency, copying into any session buffer, or waiting on
   bounded stdin-queue capacity, and is released when the response is sent.
   Exhaustion returns typed `stdin-byte-budget-exhausted` without
   retaining the payload.
4. Acquire the per-exec `stdin_inflight` semaphore, exactly one permit,
   before comparing or writing a non-empty chunk. A second concurrent
   `WriteStdin` for the same exec either waits until its
   `client_deadline_ms`/`stdin_write_deadline_ms` expires or returns
   `stdin-backpressure`. This serializes offset advancement and prevents
   same-exec fan-in from building more than the single bounded stdin queue.
5. After both permits are held, validate `data.len() <=
   effective_limits.max_chunk_bytes`. An effective-limit `max + 1`
   request that is still below the ttRPC frame cap is rejected with
   `max-chunk-exceeded`; it may have incurred one bounded decoded bytes
   allocation, but it is never copied into session storage or queued
   behind another stdin write.

Therefore, for `N` active connections, malicious concurrent
`WriteStdin` fan-in is bounded by:

```text
decoded request bytes <= N * 4 * 4 MiB
session-copied payload bytes <= N * 4 * 1 MiB
per-exec guestd-retained stdin <= 1 decoded in-flight chunk + 1 bounded queued chunk
```

The per-VM connection cap from the guest-control transport budget bounds
`N`; with the initial four-connection cap, worst-case decoded stdin
pressure is 64 MiB per VM before ordinary Rust allocator overhead, while
session-copied stdin is bounded to 16 MiB per VM before per-exec queue
limits. This budget is separate from output log retention and must be
included in RSS evidence.

Overload behavior is fail-closed and non-spawning: capacity failures
return typed errors before creating a process, allocating a retained log,
or accepting another long-poll waiter. Error payloads carry only bounded
limit metadata and never include argv, environment, output bytes, socket
paths, token material, MACs, or guest free-form strings.

## Server-side storage model

Each exec owns two output log objects, or one stdout log for TTY mode.
Each log is an append-only byte sequence with a bounded retained window.
The implementation may use a ring buffer, segmented files under guestd's
runtime directory, or a hybrid, but the visible semantics are identical:
absolute offsets, immutable retained bytes, explicit dropped-byte counts,
and EOF markers.

Attached execs keep live logs at least until the process reaches a
terminal state and all attached readers have consumed EOF. Detached execs
persist logs until the retention policy expires, the operator explicitly
removes the exec record, or the VM reboots. Guestd restart loses only
non-persisted runtime state; host callers observe `lost-guestd` or
`stale-session` and must not attach to an exec whose guest-side session
identity no longer matches.

### Retained-log storage security

Retained stdout/stderr bytes are **guest-local state**. The host may
receive them only as the explicit response body of
`ReadOutput`/`ExecLogs`, and the CLI may write them only to the
operator-requested terminal, stdout/stderr, or output file. They must
not be copied into host daemon state, broker audit records, host metrics,
host spans, host health JSON, bundle manifests, or any host-visible
sidecar directory.

### Guest-control audit contract

Guest-control audit records are allowlist-only. They may contain:

- bounded operation kind from the closed guest-control operation enum: `hello`,
  `capabilities`, `health`, `exec-create`, `exec-inspect`, `exec-wait`,
  `exec-logs`, `write-stdin`, `read-output`,
  `close-stdin`, `tty-win-resize`, `exec-signal`, `exec-cancel`, and
  `framework-guest-op`;
- VM/environment identifiers, caller role/uid where already authorized,
  target uid/user kind, and bounded capability names;
- bounded outcome/error/remediation enums;
- numeric counters and limits such as byte counts, offsets, chunk counts,
  durations, retry-after, and truncation booleans;
- timestamps and monotonic state-generation numbers.

No other audit operation kind is valid until a schema update and panel
review add it to the closed enum.

They must not contain argv, cwd, environment values, stdout/stderr/stdin
payload bytes, retained log paths, tokens, MACs, transcripts, CH/vsock
socket paths, guest-derived free-form errors, or unbounded request/session
IDs unless a later ADR explicitly approves a specific bounded identifier
shape. Tests must seed canaries for every denied class and fail if any
daemon, broker, guestd, userd, metric, span, health, or JSON error surface
contains them outside the explicit stdio/log payload APIs.

If retained logs are file-backed, they live below guestd-owned guest
paths, never below `/nix/store`, a host-shared mount, or a virtiofs export:

- live attached state: `/run/nixling/guest-control/exec/<uid>/<exec-id>/`;
- detached retained state:
  `/var/lib/nixling/guest-control/exec/<uid>/<exec-id>/`.

The path components above are design-level names; implementations may use
different leaves only if they preserve the same security properties:

1. The top-level runtime/state parents are created by guest activation as
   `root:nixling-guestd`, mode `0750` or stricter. Per-exec directories
   and log segments are `0700` directories and `0600` files, owned by the
   in-guest `nixling-guestd` service account (or an equivalently isolated
   service principal), not by the target workload user.
2. Per-user isolation is enforced before opening a log object. Guestd
   maps every exec to the authenticated target user and hands any
   append/read capability to the matching `nixling-userd` over already-open
   file descriptors; userd never resolves another user's retained-log path.
3. Every open is rooted at the pre-opened runtime/state directory and uses
   symlink-safe, beneath-root traversal. Symlinks, `..`, hard-link count
   surprises, non-regular log segments, world/group-writable parents, and
   cross-device escapes are fatal `retained-log-path-unsafe` errors before
   bytes are read or written.
4. File-backed segments are created with exclusive create semantics and
   restrictive mode before any child output is accepted. Cleanup must
   unlink by directory file descriptor, not by re-parsing a string path.
5. Quotas are enforced at three levels: per-stream/per-exec
   `max_*_log_bytes`, per-guest-user retained-log bytes, and a VM-global
   guest-control retained-log budget. `ExecCreate(detached=true)` fails
   with a bounded `retained-log-quota-exceeded` error if admitting the exec
   would exceed either aggregate quota.
6. Cleanup is deterministic. Attached non-detached logs are removed after
   all readers have observed EOF, or after a short terminal-state grace
   period if the client disappears. Detached logs expire at the earlier of
   explicit operator removal, VM reboot, or the configured TTL; the default
   TTL is 24 hours and implementations must support a lower site policy.
   Startup cleanup removes expired, orphaned, partially-created, and
   path-unsafe records before serving `ExecLogs`.

Guestd may keep small live rings in memory instead of files for attached
execs, but detached retained logs still count against the same per-user and
VM quotas. In-memory implementations must prove that reboot/restart loss
is surfaced as `lost-guestd`/`stale-session` rather than silently serving
partial output as durable retained logs.

Output reader tasks append bytes from the child fd into the log only
while the log has capacity. When the retained window is full:

- For attached, non-detached execs, the reader stops reading from the fd,
  causing pipe/PTY backpressure to reach the child. If the full condition
  persists past `slow_consumer_grace`, guestd cancels the exec with
  `slow-consumer-cancelled`.
- For detached execs, the log may evict the oldest bytes and increment
  `dropped_bytes` up to the configured retention limit. The child is not
  cancelled solely because no host is attached, but total disk/memory use
  remains bounded.
- For attached+detached execs, the detached retention policy wins for log
  storage, while attached clients still receive `offset-expired` if they
  fall behind the retained window.

## Polling and long-poll behavior

Long-polling is deliberately short. It hides idle polling latency without
allowing one stalled unary call to monopolize resources indefinitely.
Recommended CLI loop:

1. Issue parallel `ReadOutput(stream=stdout, wait=true)` and, for non-TTY,
   `ReadOutput(stream=stderr, wait=true)` from current cursors.
2. Issue `ExecWait(timeout_ms=long_poll_timeout)` concurrently or after a
   read timeout.
3. Write local terminal/stdout/stderr bytes immediately as chunks arrive.
4. Re-issue reads until each stream reports EOF and wait reports a
   terminal state.

The server enforces a per-exec cap on simultaneous pending read waits per
stream, normally one per connection, plus the per-VM caps above. A second
wait at the same cursor replaces or rejects the older one with
`superseded-read-wait`; per-VM exhaustion returns
`read-wait-capacity-exceeded`. This prevents abandoned clients from
accumulating waiters.

## Ordering of resize, signal, cancel, and exit

Every control mutation carries a `request_id` and a `control_seq`
supplied by the client. A new control request is accepted when
`control_seq` equals `last_control_seq + 1`; otherwise the server returns
`control-seq-mismatch` with the expected value. Accepted controls are
idempotent only for the same retained `request_id`, `control_seq`, and
payload. Reusing a retained `request_id` with a different sequence or
payload returns `request-id-conflict`. This sequence covers:

- `TtyWinResize`;
- `ExecSignal`;
- `ExecCancel`.

`WriteStdin` and `CloseStdin` are ordered by stdin byte offset and
`request_id`, not by `control_seq`.
Output chunks are ordered by stream offset. Terminal state is ordered
after all output bytes guestd read before observing process exit. If a
process exits while unread bytes remain in OS pipes, guestd drains those
pipes until EOF or the bounded buffer policy blocks; only then does it
return stream EOF. If draining cannot complete because a client is too
slow, the terminal state becomes `slow-consumer-cancelled` rather than
silently dropping bytes for attached sessions.

### Resize

`TtyWinResize` is valid only for TTY execs and includes `{rows, cols}`.
The first resize after create must have `control_seq = 1` only if the CLI
observes a size change after `ExecCreate`; otherwise the create-time
geometry is authoritative. Guestd applies the PTY resize with the PTY
backend's `TIOCSWINSZ` equivalent and relies on the kernel to deliver
SIGWINCH to the current foreground process group. Guestd must not also
call `killpg(SIGWINCH)` for the ordinary resize path, because that would
duplicate the kernel notification. If the requested rows/cols equal the
last applied size, guestd acknowledges the idempotent request without
issuing another `TIOCSWINSZ`; intentional duplicate SIGWINCH would
require a future explicit API flag.

### Signal

`ExecSignal` includes a numeric signal and a `target` enum:
`foreground_process_group` for TTY and `process_tree` for non-TTY. The
default CLI termination behavior maps to foreground process group in TTY
and process tree in non-TTY. A signal accepted after process exit is
idempotent no-op only for duplicate `request_id`; otherwise it returns
`exec-already-exited`.

For TTY execs, guestd resolves `foreground_process_group` from the PTY
foreground owner (`tcgetpgrp` semantics), not from the root shell PID.
This matters after an interactive shell transfers the controlling
terminal to a foreground child job: Ctrl-C and `ExecSignal(INT)` must
hit that child process group, then allow the shell to regain the terminal
and report the child's signal-derived status.

### Cancel

`ExecCancel` is the host-initiated cleanup primitive. It closes stdin,
terminates the process according to the guest policy escalation, marks
state `cancelled`, wakes all waiters/readers, and preserves retained logs
according to detached/attached retention. Transport disconnect does not
cancel detached execs. It cancels attached non-detached execs after a
small grace period unless another authorized attach takes ownership.

## EOF, close, and half-close semantics

- Stdin close and output EOF are independent.
- `CloseStdin` never implies process termination.
- Child exit never implies stdin close succeeded; inspect reports both.
- Output EOF is per stream. In non-TTY mode stdout may EOF before stderr.
- In TTY mode stderr is absent; all PTY output is stdout.
- PTY EOF is a drained state, not the first kernel hangup indication:
  bytes returned before Linux `EIO` or `POLLHUP` remain part of stdout
  and advance `stdout_end_offset` before EOF becomes visible.
- Ctrl-D is data in TTY mode, not a protocol close. The CLI sends byte
  `0x04` through `WriteStdin` when the local terminal produces it.
- EOF responses are replayable forever while the exec record exists.

## Bounded memory and backpressure proof

Raw ttRPC streams failed the feasibility gate because application receivers could stall
while payloads accumulated behind stream delivery tasks. This design
moves flow control into the application protocol:

1. The server never accepts more than one bounded stdin chunk per exec
   beyond the decoded in-flight request. A blocked child therefore fills
   that bounded queue and blocks or rejects the next `WriteStdin`; guestd
   does not build an unbounded stdin queue. Partial Linux pipe/PTY writes
   are completed from that queue before any later stdin offset is admitted.
2. Output is copied from child fds only into bounded logs. If an attached
   reader stops, output log capacity fills, guestd stops reading, and the
   kernel pipe/PTY blocks the child. If the condition lasts past the
   grace window, guestd returns a typed slow-consumer cancellation.
3. Detached output uses bounded ring retention. Dropping old detached log
   bytes is explicit through `start_offset`, `dropped_bytes`, and
   `truncated` fields, never silent for attached readers.
4. ttRPC rejects messages above its fixed 4 MiB frame cap before handler
   entry. Effective `max_chunk_bytes` violations that fit under that
   transport cap may allocate one decoded protobuf `bytes` field, but
   per-connection decoded-byte semaphores and per-exec stdin permits bound
   the allocation and reject before session-buffer copy.
5. Pending waits are bounded per exec and timeout quickly.
6. Per-connection and per-VM caps limit concurrent exec count, pending RPC
   count, total retained log bytes, and total live buffer bytes.

With default limits, a non-TTY attached exec consumes at most roughly:

```text
stdin_rpc_chunk        <= 64 KiB
decoded_stdin_budget   <= 16 MiB per connection, shared across execs
stdout_live_buffer     <= 1 MiB
stderr_live_buffer     <= 1 MiB
read_response_chunks   <= 2 * 64 KiB
metadata/dedupe/events <= implementation budget, target < 1 MiB
```

Detached execs add configured retained log storage but remain bounded by
`detached stdout log + detached stderr log`. The conformance budget of
64 MiB above idle for four concurrent sessions is therefore achievable
with the defaults and must be proven by tests before implementation
locks.

## Conformance matrix mapping

| ADR 0026 row | Chunked stdio behavior |
| --- | --- |
| stdin open/close, EOF, TTY Ctrl-D | `stdin_open`, `WriteStdin` offsets, `CloseStdin`, and TTY Ctrl-D-as-data define this explicitly. |
| stdout/stderr separation | Separate logs and read RPCs in non-TTY mode. |
| TTY merge | PTY output maps to stdout; stderr read is typed unavailable. |
| initial geometry and resize ordering | Geometry is part of create; later resizes use `control_seq`. |
| PTY leadership / foreground process group | Not solved by wire format; implementation must use the safe-PTY proof path: session leader, controlling terminal, `tcgetpgrp` foreground owner, and child job handoff are required. |
| Ctrl-C/signal delivery | `ExecSignal` targets the current `tcgetpgrp` foreground process group for TTY, including after a shell hands the terminal to a child job. |
| exit code/signal propagation | `ExecWait` and `Inspect` hide recorded terminal state until output preceding terminal observation is retained, delivered/acknowledged, or explicitly dropped with cursor accounting, then expose `visible_terminal_status`. |
| bounded memory / backpressure | Bounded logs, one stdin chunk, limited waiters, and slow-consumer cancellation. |
| concurrent sessions/fairness | Per-exec caps and short polling prevent one stalled exec from owning global queues. |
| cancellation/disconnect cleanup | `ExecCancel` plus attached/detached disconnect policy. |
| half-close behavior | `CloseStdin` is independent of output reads and wait. |
| guestd restart / VM reboot | Session identity, inspect state, and stale-session errors reject unsafe reattach. |
| raw-mode restoration | CLI owns local raw mode and restores on terminal state or protocol error. |
| max message size | ttRPC fixed 4 MiB frame cap plus bounded post-decode `max_chunk_bytes` check. |
| malformed messages | Typed `protocol-error` codes with bounded fields only. |
| no data leakage | Logs/metrics carry counters, offsets, outcomes, and booleans, not payloads. |

## Health status model

Guest-control health is bounded and schema-versioned. Guest-returned
`Health` payloads carry `origin = guest-reported`; host-synthesized readiness
failures that happen before a guest `Health` RPC can complete carry
`origin = host-synthesized`. Every health status then includes a state enum
plus bounded reason/remediation enums:

- `healthy`
- `degraded`
- `unavailable-old-generation`
- `listener-absent`
- `transport-unreachable`
- `auth-failed`
- `protocol-mismatch`
- `stale-session`

The bounded reason enum is closed for this protocol:

- `none`
- `old-generation`
- `listener-absent`
- `connect-refused`
- `connect-timeout`
- `eof-before-ack`
- `malformed-ack`
- `ack-too-long`
- `transport-io`
- `auth-token-rejected`
- `protocol-version-unsupported`
- `session-generation-mismatch`
- `exec-subsystem-unavailable`
- `log-storage-unavailable`
- `quota-exceeded`
- `rate-limited`
- `internal-health-check-failed`

The bounded remediation enum is also closed:

- `none`
- `retry`
- `restart-vm`
- `upgrade-guest`
- `check-auth-token`
- `check-guestd-service`
- `reduce-load`
- `inspect-guest-logs`

Allowed state mappings:

| State | Allowed reasons | Allowed remediations |
| --- | --- | --- |
| `healthy` | `none` | `none` |
| `degraded` | `exec-subsystem-unavailable`, `log-storage-unavailable`, `quota-exceeded`, `rate-limited`, `internal-health-check-failed` | `retry`, `reduce-load`, `inspect-guest-logs`, `restart-vm` |
| `unavailable-old-generation` | `old-generation` | `upgrade-guest`, `restart-vm` |
| `listener-absent` | `listener-absent` | `check-guestd-service`, `restart-vm` |
| `transport-unreachable` | `connect-refused`, `connect-timeout`, `eof-before-ack`, `malformed-ack`, `ack-too-long`, `transport-io` | `retry`, `restart-vm`, `check-guestd-service` |
| `auth-failed` | `auth-token-rejected` | `check-auth-token` |
| `protocol-mismatch` | `protocol-version-unsupported` | `upgrade-guest` |
| `stale-session` | `session-generation-mismatch` | `retry`, `restart-vm` |

The matrix is also origin-constrained: `healthy` and `degraded` are
guest-reported, while `unavailable-old-generation`, `listener-absent`,
`transport-unreachable`, `auth-failed`, `protocol-mismatch`, and
`stale-session` are host-synthesized readiness/status results.

`healthy` requires CH `CONNECT`, Hello/auth, and Health to succeed on the
same post-CONNECT stream. `degraded` means guestd is authenticated and
serving Health but one bounded subsystem check failed; callers may proceed
only with operations whose capability bit remains healthy. Every other
state is unavailable for new exec work. Health, status JSON, logs,
metrics, spans, and audit records must carry only the bounded state,
reason, remediation, protocol version, and capability names; they must not
include socket paths, token/MAC material, transcripts, argv/env/cwd,
payload bytes, or guest free-form error text.

## CLI behavior

### Attached exec

`nixling exec <vm> -- <argv...>` creates a non-TTY exec with stdin
closed. The CLI reads stdout/stderr through offsets until both streams
EOF and `ExecWait` returns terminal. The CLI exits with the remote exit
code for normal command exit. Remote signal termination is reported as the
command result with signal metadata and shell-style status `128 + signal`.
Typed nixling errors are reserved for transport, protocol, authorization,
and pre-exec failures.

`--interactive` opens stdin. The CLI forwards local stdin in
`max_chunk_bytes` chunks with increasing offsets and sends `CloseStdin`
on local EOF. If `WriteStdin` returns `stdin-backpressure`, the CLI
blocks local input reading or uses terminal flow control until retry is
accepted.

`--tty` allocates a PTY, sends initial geometry in `ExecCreate`, sends
resizes as sequenced `TtyWinResize` calls, merges output through stdout,
and restores local terminal raw mode on every return path.

### Detached exec

`--detach` returns the `exec_id`, initial state, and effective retention
limits. The process continues after host transport disconnect. Operators
use:

- `nixling vm exec inspect <vm> <exec-id>` for state and offset windows;
- `nixling vm exec logs <vm> <exec-id>` for retained logs;
- `nixling vm exec attach <vm> <exec-id>` to resume attached polling from
  a chosen cursor;
- `nixling vm exec kill <vm> <exec-id>` for `ExecSignal` or later policy
  escalation.

### Old-generation VMs

As required by ADR 0026, new exec commands do not fall back to SSH. If a
running VM lacks guest-control capabilities, the CLI returns
`guest-control-unavailable-old-generation` with remediation. Existing
SSH-backed compatibility commands outside generic exec keep their
separate compatibility window.

### Logs UX

`ExecLogs` uses the same offset model as `ReadOutput(stream=...)` but
packages stream records for CLI display or JSON. Human logs default to
available retained bytes and warn when `dropped_bytes > 0`. JSON includes
`startOffset`, `endOffset`, `nextOffset`, `droppedBytes`, `eof`, and
`truncated`, but never embeds unbounded metadata. Payload bytes are
emitted only as the requested command output/log stream, not in error
objects, daemon logs, metrics, or health JSON.

## Errors

Required typed error kinds:

- `exec-not-found`
- `stale-session`
- `exec-already-exited`
- `tty-required`
- `tty-stderr-unavailable`
- `stdin-not-open`
- `stdin-closed`
- `stdin-closed-by-process`
- `stdin-offset-mismatch`
- `stdin-backpressure`
- `stdin-byte-budget-exhausted`
- `offset-expired`
- `offset-in-future`
- `offset-exhausted`
- `output-lost`
- `max-chunk-exceeded`
- `control-seq-mismatch`
- `request-id-conflict`
- `superseded-read-wait`
- `read-wait-capacity-exceeded`
- `wait-capacity-exceeded`
- `exec-capacity-exceeded`
- `exec-attach-capacity-exceeded`
- `guest-exec-disabled`
- `guest-exec-root-denied`
- `guest-exec-user-denied`
- `cwd-invalid`
- `cwd-denied`
- `retained-log-path-unsafe`
- `retained-log-quota-exceeded`
- `rate-limited`
- `slow-consumer-cancelled`
- `guest-control-unavailable-old-generation`
- `auth-failed`
- `transport-unreachable`
- `protocol-error`

Each error response carries only bounded fields: expected offsets,
current windows, limits, state enum, and remediation enum. It must not
include argv, env, cwd, stdout/stderr bytes, token material, socket
paths, or guest-derived free-form strings.

## Observability

Metrics and logs may include:

- bytes read/written per stream;
- chunks read/written per stream;
- read wait timeouts;
- blocked stdin write duration histograms;
- output backpressure duration histograms;
- cancellations by bounded reason enum;
- offset-expired and max-chunk-exceeded counters;
- current exec count and retained-log byte gauges.

They must not include payload bytes, session IDs, command lines,
environment variables, cwd, credential paths, CH socket paths, HMAC
material, or guest free-form errors.

Health responses and CLI JSON use the same rule except for fields that are
the explicit user-facing API result. Attached exec forms reject `--json`
with usage; detached run JSON, for example
`nixling vm exec run <vm> --detach --json -- <argv...>`, may return the
new `execId`, and `ExecLogs --json` may return the requested log payload
when the user asked for logs, but daemon logs, metrics, spans, health JSON,
and error JSON must not duplicate those payloads or IDs.

## Required tests

Before implementation exits design hardening, add at least:

1. protobuf/schema drift tests for every message above;
2. receive-limit tests: ttRPC's fixed 4 MiB frame cap rejects cap+1
   messages before handler entry; effective `max_chunk_bytes + 1` below
   that transport cap fails with one bounded decoded allocation and no
   session-buffer copy;
3. stdin offset/idempotency tests, including duplicate request IDs and
   mismatched duplicate payloads;
4. close-stdin and `WriteStdin.close_after` tests for non-TTY pipe and
   TTY Ctrl-D-as-data behavior, plus TTY protocol-side close proving the
   PTY master/writer stays open and output after close is not lost;
   duplicate close/final-write is accepted only for the same retained
   `request_id` and payload/final offset, while mismatched or
   different-request duplicates fail typed;
5. stdout/stderr byte-exact 64 MiB + 64 MiB non-TTY test;
6. stdin 16 MiB slow-reader test with bounded RSS;
6a. malicious concurrent `WriteStdin` fan-in test: four hard-maximum-sized
    ttRPC-frame requests on one connection consume the 16 MiB decoded-byte
    budget, a fifth concurrent request receives
    `stdin-byte-budget-exhausted` or waits for a permit, and concurrent
    effective-limit `max + 1` writes to one exec preserve offset order and
    never queue more than one chunk behind the single per-exec stdin
    permit; include pipe and PTY partial child-write cases proving the
    bounded queue drains without duplicate or lost stdin bytes before
    `CloseStdin` is observed;
7. deterministic slow stdout/stderr consumer stress proving block or
   `slow-consumer-cancelled` with bounded retained bytes; a separate
   non-default runtime soak may cover 30-second wall-clock behavior;
8. detached retention tests proving dropped-byte accounting and
   `offset-expired` behavior;
9. long-poll timeout and waiter-cap tests;
10. resize ordering tests that fail on reordered `control_seq`, prove a
    single `TIOCSWINSZ` yields a single SIGWINCH, and prove unchanged
    sizes are deduplicated before `TIOCSWINSZ`; control tests must also
    cover `request_id` idempotent replay for resize, signal, and cancel,
    and reject mismatched duplicate control payloads;
11. signal ordering and foreground process-group delivery tests covering
    session leadership, controlling-terminal setup, `tcgetpgrp`
    ownership, and SIGINT after a shell hands the terminal to a
    foreground child group;
12. concurrent-session fake-scheduler test with slow-output,
    blocked-stdin, interactive TTY, and unary health loop, requiring
    bounded service-turn gaps and no byte-skew starvation;
13. guestd restart and VM reboot stale-session rejection tests;
14. CLI raw-mode restoration tests for success, signal, protocol error,
    and disconnect;
14a. Health state/reason/remediation matrix tests enumerating every
     Health state, reason, and remediation, rejecting invalid combinations
     such as `healthy` plus an error reason, and proving CONNECT,
     Hello/auth, and Health failures map to the documented bounded states
     without leaking socket paths, tokens, transcripts, guest text, or
     unbounded IDs;
15. retained-log storage security tests covering guest-local path roots,
    ownership/mode, symlink and hard-link rejection, per-user isolation,
    per-exec/per-user/VM quota enforcement, TTL/startup cleanup, and the
    absence of retained stdout/stderr bytes from host-visible state other
    than explicit `ExecLogs` responses;
16. observability and audit redaction tests proving stdout/stderr/env/
    argv/cwd/token/socket/transcript material never enters logs, broker
    or guest-control audit records, metrics, spans, health, or JSON
    errors.
17. PTY close/drain tests proving Linux `EIO` or `POLLHUP` after slave
    close becomes stdout EOF only after all buffered output has advanced
    the stream cursor.

### Redaction test matrix

The implementation test suite must seed every surface below with stable
canary values and assert that only the allowed API response contains them.
Use distinct canaries for argv, cwd, environment, credential paths, HMAC
MACs, socket paths, IDs, guest error text, stdout, stderr, and transcripts
so a failure names the leaking class.

| Canary class | Seed location | Must be absent from | Allowed location |
| --- | --- | --- | --- |
| argv / command line | `ExecCreate argv = ["true", "ARG_CANARY"]` | daemon logs, guestd logs except bounded kind, audit records, metrics labels/samples, spans, health, error JSON | never; CLI human dry-run is out of scope for guestd telemetry |
| cwd | `--workdir /home/alice/CWD_CANARY` | logs, audit records, metrics, spans, health, error JSON | never; use bounded `cwd-invalid`/`cwd-denied` kinds |
| env / tokens | `--env TOKEN_CANARY=...`, env-file path | logs, audit records, metrics, spans, health, error JSON, retained-log metadata | child stdout/stderr only if the command itself prints it and user requested logs |
| credential path | guestd `LoadCredential` path and token-file validation errors | logs, audit records, metrics, spans, health, CLI JSON errors | never; report bounded credential error kind |
| HMAC/MAC/transcript | failed auth proof with known MAC and transcript canaries | logs, audit records, metrics, spans, health, CLI JSON errors | never; report bounded auth failure kind |
| CH/vsock/socket paths | stale or refused CONNECT using a path canary | logs, audit records, metrics, spans, health, CLI JSON errors | never; report bounded transport kind |
| session / exec / request IDs in telemetry | successful and failed calls with ID canaries | logs, audit records, metrics labels, span attrs/events, health | CLI JSON fields whose contract explicitly returns `execId`/cursor state |
| guest-derived free-form errors | child exits after writing `ERR_CANARY` to stderr; guestd returns malformed free-form text in a fake transport | daemon logs, guestd structured logs, audit records, metrics, spans, health, CLI JSON error object | stderr stream only when requested as command output/logs |
| stdout/stderr payloads | child writes `STDOUT_CANARY` and `STDERR_CANARY` | logs, audit records, metrics, spans, health, CLI JSON errors, inspect/wait JSON | `ReadOutput`/`ExecLogs` payloads and attached CLI stdout/stderr |
| socket paths / transcripts in debug formatting | force `Debug`/`Display` on transport/auth errors | all structured logs, audit records, spans, health, CLI JSON errors | never |

Required assertions:

1. Capture daemon, guestd, and userd structured logs plus host broker and
   guest-control audit records during the test and grep for every canary;
   matches are allowed only in the explicit payload stream under test.
2. Scrape Prometheus/OpenTelemetry metric output and assert canaries are
   absent from metric names, labels, exemplars, and sample values except
   numeric byte counts.
3. Export spans/events and assert canaries are absent from span names,
   attributes, events, status descriptions, and exception fields.
4. Call `Health`, `ExecInspect`, `ExecWait`, failed `ExecCreate`, failed
   auth, and failed `ExecLogs --json`; assert JSON contains bounded enums,
   counters, offsets, booleans, and remediation only.
5. Run the same canaries through attached, detached, TTY, non-TTY,
   success, protocol-error, auth-error, quota-error, and stale-session
   paths so redaction is not limited to the happy path.

## UX latency tradeoffs

Chunked stdio trades stream immediacy for explicit bounds. Interactive
latency depends on read long-poll timeout, chunk size, scheduler fairness,
and RPC overhead. The defaults intentionally choose small 64 KiB chunks
and 100 ms read waits so shell echo and resize feedback stay responsive
under load. Larger chunks improve bulk throughput but can increase
head-of-line time for a single response; callers may request smaller
chunks for TTY sessions.

The CLI should issue stdout/stderr reads concurrently and immediately
re-poll after any response. It should not sleep between successful reads.
For bulk detached logs, the CLI may request larger chunks up to the
effective maximum.

## Implementation complexity

This design is more complex than raw ttRPC streams because guestd must
own per-exec logs, offsets, dedupe state, waiter accounting, and explicit
backpressure policy. It is still simpler than a custom binary stream
because:

- lifecycle and I/O remain protobuf/ttRPC contracts;
- every method is unary and fits existing ttRPC request handling;
- retries/idempotency are explicit;
- attached and detached logs share one cursor model;
- message-size enforcement lives at protobuf RPC boundaries;
- conformance failures can be isolated to individual RPCs.

The main implementation risks are fairness bugs in the guestd scheduler,
PTY drain behavior after process exit, retention-window edge cases, and
incorrect CLI retry behavior around `stdin-backpressure` or
`offset-expired`.

## Implementation gates

- PTY output backpressure is kernel- and workload-sensitive. The feasibility
  proof covers the wire/protocol model for TTY stdout-only reads and bounded
  output logs;
  the implementation must add runtime tests proving the selected PTY crate
  and guest kernel block writers instead of dropping data before
  guest-control ships.
- Detached logs can hide slow consumers by evicting old bytes; UX must
  make truncation obvious in human and JSON output.
- Short long-poll timeouts increase RPC rate. Capability negotiation may
  need per-VM rate limits if many clients attach concurrently.
- Server-side segmented files reduce RSS but introduce cleanup and disk
  quota risk. Pure memory rings are simpler but constrain detached log
  sizes. Implementation must choose one and test quota behavior.
- `control_seq` protects ordering for host-originated controls, but
  process exit can race with accepted signal/resize calls. The state
  machine must define the visible result and tests must lock it.
- Guestd restart cannot preserve live process handles unless a later
  design adds durable supervision. Current design intentionally rejects
  stale reattach rather than pretending continuity.

## References

- [ADR 0026: Guest control plane over virtio-vsock](../adr/0026-guest-control-plane-over-vsock.md)
- [Guest control feasibility dossier](../adr/guest-control-feasibility-dossier.md)
- [Kata Agent protocol stdio RPCs](https://github.com/kata-containers/kata-containers/blob/6d2066b692ce69a908bb4daec2c6b71ccfad3829/src/libs/protocols/protos/agent.proto#L33-L49)
- [Kata Agent stream message shapes](https://github.com/kata-containers/kata-containers/blob/6d2066b692ce69a908bb4daec2c6b71ccfad3829/src/libs/protocols/protos/agent.proto#L211-L227)
