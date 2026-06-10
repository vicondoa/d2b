# Guest-control exec I/O credit-window protocol

This document records the bounded credit-window ttRPC stream candidate
for nixling guest-control exec I/O. The W0 decision selected
[chunked stdio RPCs](./guest-control-exec-io-chunked-stdio.md) instead
because raw ttRPC stream buffering was the observed failure mode and the
unary chunked model is simpler to bound and test. This credit-window
overlay remains a fallback candidate if the selected chunked-stdio path
fails later implementation evidence.

Under this candidate, unary guest-control APIs continue to use ordinary
ttRPC request/response messages. Attached and detached exec I/O uses a
ttRPC async duplex stream carrying nixling `TerminalFrame` protobuf
messages with application-level byte credit. Raw ttRPC stream
backpressure is not relied on for correctness.

The design goal is Docker-like exec behavior while preserving bounded
memory, byte-exact stdout/stderr/stdin delivery, explicit half-close
semantics, and typed failure when either endpoint stops consuming.

## Stream model

`ExecCreate`, `ExecInspect`, `ExecLogs`, `ExecSignal`, and `ExecKill`
are unary control RPCs. `ExecAttach` is the only duplex streaming RPC.
A host may attach when creating an exec, or attach later to a detached
exec that is still running or has retained logs.

```protobuf
syntax = "proto3";

package nixling.guestcontrol.v1;

service GuestControl {
  rpc ExecAttach(stream TerminalFrame) returns (stream TerminalFrame);
}

message ExecSessionId {
  string vm = 1;              // bounded VM name, not a secret
  bytes exec_id = 2;          // 128-bit random ID, encoded as 16 bytes
  uint64 generation = 3;      // rejects stale sessions after guestd restart
}

enum TerminalChannel {
  TERMINAL_CHANNEL_UNSPECIFIED = 0;
  TERMINAL_CHANNEL_STDIN = 1;     // host -> guest data
  TERMINAL_CHANNEL_STDOUT = 2;    // guest -> host data, non-TTY only
  TERMINAL_CHANNEL_STDERR = 3;    // guest -> host data, non-TTY only
  TERMINAL_CHANNEL_TTY = 4;       // merged PTY data in TTY mode
  TERMINAL_CHANNEL_CONTROL = 5;   // resize, signal, exit, errors, credit
}

enum CloseReason {
  CLOSE_REASON_UNSPECIFIED = 0;
  CLOSE_REASON_EOF = 1;           // clean half-close for a data channel
  CLOSE_REASON_CANCELLED = 2;     // local cancellation or host disconnect
  CLOSE_REASON_SLOW_CONSUMER = 3; // credit starvation timeout
  CLOSE_REASON_PROTOCOL_ERROR = 4;
  CLOSE_REASON_PROCESS_EXITED = 5;
}

enum ErrorCode {
  ERROR_CODE_UNSPECIFIED = 0;
  ERROR_CODE_PROTOCOL = 1;
  ERROR_CODE_OVERSIZE_FRAME = 2;
  ERROR_CODE_OUT_OF_ORDER = 3;
  ERROR_CODE_UNKNOWN_SESSION = 4;
  ERROR_CODE_UNSUPPORTED_CHANNEL = 5;
  ERROR_CODE_WINDOW_EXHAUSTED = 6;
  ERROR_CODE_SLOW_CONSUMER = 7;
  ERROR_CODE_INTERNAL = 8;
}

message TerminalFrame {
  ExecSessionId session = 1;
  TerminalChannel channel = 2;

  // Monotonic per sender and per session across all frame kinds. The peer
  // rejects gaps, duplicates, and regressions with ERROR_CODE_OUT_OF_ORDER.
  uint64 sequence = 3;

  // Monotonic byte offset for DATA frames on the named channel. Non-DATA
  // frames set this to zero. The receiver uses it to prove byte-exact order.
  uint64 data_offset = 4;

  oneof payload {
    DataFrame data = 10;
    CreditGrant credit = 11;
    CloseFrame close = 12;
    ResizeFrame resize = 13;
    SignalFrame signal = 14;
    ExitFrame exit = 15;
    ErrorFrame error = 16;
    AttachHello hello = 17;
  }
}

message AttachHello {
  uint32 protocol_version = 1;
  bool tty = 2;
  uint32 rows = 3;
  uint32 cols = 4;
  uint32 max_frame_bytes = 5;
  repeated ChannelWindow initial_windows = 6;
}

message ChannelWindow {
  TerminalChannel channel = 1;
  uint64 bytes = 2;
}

message DataFrame {
  bytes bytes = 1; // <= negotiated max_frame_bytes
}

message CreditGrant {
  TerminalChannel channel = 1;
  uint64 bytes = 2;
  uint64 consumed_offset = 3; // byte offset durably written to downstream sink
  uint64 ack_sequence = 4;    // highest peer sequence observed before grant
}

message CloseFrame {
  TerminalChannel channel = 1;
  CloseReason reason = 2;
  string detail = 3;          // bounded, no command/output/env content
  uint64 final_offset = 4;    // expected total bytes for data channels
}

message ResizeFrame {
  uint32 rows = 1;
  uint32 cols = 2;
}

message SignalFrame {
  string signal = 1;          // canonical name such as INT or TERM
  int32 number = 2;           // Linux signal number for diagnostics
}

message ExitFrame {
  int32 exit_code = 1;        // set when exited normally
  string signal = 2;          // set when terminated by signal
  int32 signal_number = 3;
}

message ErrorFrame {
  ErrorCode code = 1;
  string message = 2;         // bounded and redacted
  CloseReason close_reason = 3;
}
```

Only `DataFrame` consumes byte credit. Control frames do not consume data
credit, but each endpoint maintains a small bounded control queue so that
errors, close, resize, signal, and exit cannot be hidden behind bulk data.

## Negotiated limits

The first frame from each endpoint is `AttachHello`. The effective limit
for the stream is the minimum of both sides' advertised values and the
bundle policy.

Recommended initial constants for implementation proofs:

| Limit | Initial value | Rationale |
| --- | ---: | --- |
| `max_frame_bytes` | 32 KiB | Small enough for fairness; far below ttRPC's message ceiling. |
| stdout/stderr/TTY initial credit | 256 KiB per channel | Covers interactive bursts without hiding slow consumers. |
| stdin initial credit | 64 KiB | Avoids buffering large pasted input into a slow process. |
| credit-update threshold | half the channel window | Avoids one grant per small write. |
| credit-update timer | 10 ms | Keeps interactive echo latency below the target under small writes. |
| per-channel queued DATA | one negotiated window | Keeps memory proportional to advertised credit. |
| per-session queued DATA | sum of channel windows, capped at 1 MiB | Prevents one exec from monopolizing memory. |
| control queue | 64 frames per session | Bounded but large enough for resize bursts and teardown. |
| slow-consumer grace | 30 s without credit progress | Matches the slow-consumer stress duration. |

The exact constants are policy knobs, but any implementation must expose
them through generated schema or documented bundle metadata before they
become configurable.

## Flow-control algorithm

Each endpoint tracks these counters per session and data channel:

- `send_credit`: bytes the peer has granted but this endpoint has not
  yet sent;
- `sent_offset`: next byte offset to send;
- `delivered_offset`: bytes received from the peer and written to the
  local downstream sink;
- `grantable_bytes`: bytes eligible for a future `CreditGrant` after
  downstream completion;
- `last_credit_progress`: monotonic timestamp of the last credit grant
  or downstream write completion.

### Sending DATA

1. Split data into chunks no larger than `max_frame_bytes`.
2. Send a chunk only when `send_credit >= chunk_len` and all queue caps
   remain below their limits.
3. Decrement `send_credit` exactly by `chunk_len` when the frame is
   enqueued to ttRPC.
4. If credit is exhausted, pause the upstream reader instead of buffering:
   - guest stdout/stderr/PTY readers stop reading from the child fd;
   - host stdin readers stop reading from the terminal or pipe;
   - detached log replay pauses reading from the retained log source.
5. Resume the reader when new `CreditGrant` bytes arrive.

This makes the application window, not ttRPC internals, the memory owner.
A process that writes faster than the peer consumes eventually blocks in
its PTY/pipe, or receives typed cancellation if the stall exceeds policy.

### Receiving DATA and granting credit

Credit is returned only after bytes are accepted by the downstream sink:

- CLI stdout/stderr: after the async write to the host fd completes;
- guest stdin: after the write to the child stdin pipe or PTY completes;
- detached logs: after the bounded log sink records the bytes or rejects
  them with a typed error.

The receiver increments `delivered_offset` and accumulates
`grantable_bytes`. It sends `CreditGrant` when either:

- `grantable_bytes >= window / 2`; or
- `grantable_bytes > 0` and the 10 ms update timer fires; or
- a channel is being closed and the final delivered offset must be
  acknowledged.

A grant carries the cumulative `consumed_offset` and the highest peer
`sequence` observed. The sender treats duplicate or regressive grants as
protocol errors. Grants may be coalesced, but they must never grant bytes
that have not reached the downstream sink.

### Fairness

Within one session, control frames have priority over data frames. Data
channels are scheduled round-robin with a per-turn cap of one
`max_frame_bytes` chunk so stdout cannot starve stderr or TTY input.
Across sessions on the same ttRPC connection, the daemon uses weighted
round-robin:

1. one control frame per ready session;
2. one data chunk per ready session;
3. repeat while the transport accepts work.

Unary health and lifecycle RPCs run on their normal ttRPC paths and must
not wait behind stream data queues. The conformance gate must run health
traffic while slow-output and blocked-stdin streams are active.

### Slow-consumer cancellation

A channel becomes slow when all of the following are true for longer than
the configured grace period:

- the sender has data ready or an upstream fd is readable;
- `send_credit == 0` or local DATA queues are at their cap;
- the peer has not advanced `consumed_offset` for the channel.

The sender then sends an `ErrorFrame` with
`ERROR_CODE_SLOW_CONSUMER`, followed by `CloseFrame` with
`CLOSE_REASON_SLOW_CONSUMER` for the affected data channel. For attached
execs, stdout/stderr/TTY slow-consumer cancellation terminates the exec
because the command can no longer be represented faithfully to the user.
For stdin slow-consumer cancellation, the host closes stdin and lets the
process continue unless the CLI requested whole-session cancellation.

If control frames cannot be delivered because the stream itself is
blocked or disconnected, both sides clean up from local timeout state:
the host reports a typed stream error and the guest terminates or detaches
the exec according to the `ExecCreate` policy.

### Close and EOF sequencing

- Host stdin EOF is `CloseFrame{channel: STDIN, reason: EOF,
  final_offset}` after all prior stdin DATA frames are sent.
- The guest acknowledges by granting through `final_offset` and then
  closing the child stdin fd or sending PTY EOF as appropriate.
- In non-TTY mode the guest separately closes stdout and stderr with
  final offsets after the process closes those fds.
- In TTY mode the guest closes the TTY channel once the PTY reaches EOF.
- `ExitFrame` is sent after all process-output DATA and output close
  frames have been enqueued. The receiver may display the exit status
  only after it has delivered all bytes through the advertised final
  offsets.
- `ErrorFrame` may precede `CloseFrame` when a protocol or runtime error
  aborts a channel. After a session-level fatal error, no further DATA is
  valid.

A peer rejects DATA after a channel close, close offsets that do not match
the observed byte count, unsupported channels for the negotiated TTY mode,
and exit before output final offsets are known.

## CLI behavior

`nixling exec <vm> -- <argv...>` creates an attached, non-TTY exec with
stdin closed unless `--interactive` is set. The CLI grants stdout and
stderr credit only as writes to its local fds complete. If the user pipes
output to a slow command, guest output naturally blocks at the credit
window and never grows unbounded in daemon memory.

`--tty` switches output to `TERMINAL_CHANNEL_TTY`, puts the local terminal
in raw mode, sends the initial terminal geometry in `AttachHello`, and
sends `ResizeFrame` on local resize. Raw mode is restored on normal exit,
stream error, cancellation, and panic-safe cleanup paths.

`--interactive` grants stdin credit from the guest. The CLI reads local
stdin only while the guest has credit. Ctrl-C in TTY mode is transmitted as
terminal input to the foreground process group; non-TTY signal forwarding
uses `SignalFrame` and `ExecSignal` according to the CLI command mode.

`--detach` creates the exec without an attached stream. Later
`exec attach` uses the same credit-window protocol. `exec logs` may use a
server-streaming log RPC or `ExecAttach` in replay mode, but retained log
bytes are still released only according to client credit.

Human output mirrors Docker-style behavior: command bytes go only to the
requested stdout/stderr/TTY destinations; protocol errors print concise
redacted diagnostics. JSON mode reports bounded envelope fields such as
`exec_id`, `vm`, `outcome`, `exit_code`, `signal`, `bytes_stdout`,
`bytes_stderr`, and `error_code`; it never embeds stdout, stderr,
environment, command arguments, HMAC material, or raw session tokens.

## Conformance matrix

| Requirement | Protocol mechanism | Required evidence |
| --- | --- | --- |
| stdin open/EOF | `STDIN` DATA plus `CloseFrame EOF` final offset | Byte-exact stdin and correct child EOF behavior. |
| stdout/stderr split | Separate `STDOUT` and `STDERR` channels | 64 MiB each, byte-exact demux. |
| TTY merge | `TTY` channel only; stdout/stderr invalid | PTY transcript matches expected merged stream. |
| resize ordering | sender `sequence` plus `ResizeFrame` | Reordered resize is rejected or visibly fails test. |
| signal delivery | `SignalFrame` and unary `ExecSignal` | Ctrl-C/SIGTERM reach foreground process group. |
| exit propagation | `ExitFrame` after output close | CLI exit code/signal matches child. |
| bounded memory | channel/session queue caps and reader pausing | Slow consumer stays under budget for 30 s. |
| backpressure | credit returned only after downstream write ack | Writers block or receive slow-consumer cancellation. |
| concurrent sessions | weighted round-robin and bounded queues | No starvation with slow-output, blocked-stdin, TTY, health. |
| cancellation | `ErrorFrame` plus `CloseFrame` and local cleanup | Host disconnect and user cancel reap resources. |
| half-close | per-channel close state machine | stdin close does not imply stdout/stderr close. |
| stale sessions | `generation` in `ExecSessionId` | Old session rejected after guestd restart/reboot. |
| raw-mode cleanup | CLI cleanup around stream lifecycle | Terminal restored on success, error, and cancel. |
| max frame size | ttRPC receive cap before protobuf decode plus bounded post-decode frame validation | max passes; receive-cap + 1 fails before handler entry; effective max + 1 fails with typed error and no session-buffer copy. |
| malformed messages | sequence/channel/window validation | Redacted `ERROR_CODE_PROTOCOL`; no panic. |
| log hygiene | bounded aggregate telemetry only | No command/output/env/token leaks in logs or metrics. |

## Required tests

Implementation must add protocol-level tests before production enablement:

1. fake-transport byte-exact stdout/stderr/stdin tests with payload sizes
   at least as large as the feasibility dossier;
2. 30 s slow stdout, stderr, TTY, stdin, and log-replay consumer tests
   that assert memory high-water marks and typed cancellation behavior;
3. downstream-write-ack tests using a writer that deliberately stalls
   before completing writes, proving credit is not granted early;
4. sequence, offset, duplicate grant, regressive grant, DATA-after-close,
   close-offset-mismatch, unsupported-channel, and max+1 frame tests;
5. four-concurrent-session fairness test with a health RPC loop;
6. resize and signal ordering tests against a real PTY;
7. guestd restart, VM reboot, attach-to-detached, host-disconnect, and
   daemon-restart cleanup tests;
8. CLI raw-mode restoration tests for success, process signal, protocol
   error, local cancellation, and remote disconnect;
9. logging/metrics snapshot tests proving only bounded aggregate fields
   are emitted;
10. generated protobuf/ttRPC code checks that preserve the repository's
    first-party unsafe policy.

## Risks and open decisions

- ttRPC implementations may still buffer or decode whole protobuf messages
  before the application sees them. Keeping `max_frame_bytes` small is
  mandatory, but the enforceable invariant is a receive cap before
  protobuf decode plus bounded post-decode allocation under explicit
  per-connection byte budgets.
- Credit logic is more complex than chunked request/response stdio. The
  benefit is one full-duplex attach stream with explicit half-close and
  interactive latency; the cost is a larger state machine.
- PTY EOF and Ctrl-D behavior differs by program and terminal mode. Tests
  must cover shell-like PTY sessions and simple pipe-backed commands.
- Detached log retention needs a bounded sink policy. This protocol bounds
  replay to the client, but it does not by itself define retention size or
  persistence format.
- Multi-attach semantics are intentionally not selected here. The initial
  implementation should allow one interactive attach and reject competing
  writers with a typed error; read-only log replay can be added later.
- Window constants may need tuning after real vsock measurements. Changing
  defaults is compatible if peers continue to negotiate them in
  `AttachHello`.

## Implementation complexity

Expected complexity is medium-high. The production change needs a shared
protocol crate, generated protobuf/ttRPC bindings, a tested per-session
state machine, fd reader/writer tasks that can pause and resume cleanly,
PTY integration, CLI terminal cleanup, and conformance tests. It does not
require a custom binary transport, a host proxy daemon, or per-VM systemd
units.

A safe incremental plan is:

1. implement the state machine against an in-memory fake transport;
2. add PTY-backed tests for resize, signal, EOF, and exit ordering;
3. connect the state machine to ttRPC async streams over the existing
   Cloud Hypervisor vsock transport;
4. wire CLI attach/run/logs behavior;
5. enable the feature only after the full conformance matrix passes.
