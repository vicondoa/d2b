# Guest control W0 feasibility dossier

This dossier records the W0 evidence required by
[ADR 0026](../adr/0026-guest-control-plane-over-vsock.md). It is the
panel-review input for locking the guest-control IPC direction.

## Summary recommendation

Use **ttRPC/protobuf for guest-control unary APIs**:

- `Hello`
- `Health`
- `Capabilities`
- exec lifecycle metadata (`ExecCreate`, `ExecInspect`, `ExecLogs`,
  `ExecSignal`, `ExecKill`)
- typed framework guest operations

Do **not** use raw `ttrpc-rust` async streams alone for Docker-like exec
I/O. W0 backpressure evidence shows that raw async streams can buffer
too much and lose byte-exact delivery under a stalled receiver. Exec I/O
needs either:

1. a nixling-defined credit/window protocol carried as protobuf messages
   over a ttRPC duplex stream; or
2. a Kata-style chunked stdio RPC model with explicit byte budgets and
   slow-consumer cancellation.

The next design step should compare those two bounded-I/O shapes. The
custom binary stream remains a fallback only if the bounded ttRPC shapes
fail.

## Proof branches

| Proof | Branch | Commit | Result |
| --- | --- | --- | --- |
| ADR gate | `guest-control-ttRPC` | `c3bd668` | ADR 0026 added and panel-signed after R3 fixes. |
| CH CONNECT transport | `guest-control-w0-ch` | `36619d1` | PASS: CH post-OK stream can be wrapped in `ttrpc-rust` async `Socket` and `Client` without a host proxy. |
| Static guest build | `guest-control-w0-static` | `a085e68` | PASS with implementation constraints: Nix static-musl derivation works for x86_64 and aarch64; ELF has no interpreter/NEEDED; generated-code unsafe allowance must be handled. |
| ttRPC stream semantics | `guest-control-w0-stream` | `eeaaf88` | CONDITIONAL: duplex streams are semantically expressive, but raw stream queues still need bounded flow control. |
| HMAC auth | `guest-control-w0-auth` | `7a97d09` | PASS: transcript-bound proof-of-possession prototype with redaction and replay tests. |
| Safe PTY | `guest-control-w0-pty` | `72ddbe3` | PASS: `portable-pty` plus safe `nix` APIs can cover PTY open/resize/I/O and foreground process-group signaling without first-party unsafe. |
| Generated-code unsafe | `guest-control-w0-codegen` | `06298c0` | PASS: proof build postprocesses ttRPC generated code to remove `#![allow(unsafe_code)]` and verifies no generated unsafe tokens remain. |
| Backpressure | `guest-control-w0-pressure` | `9a849c9` | FAIL for raw ttRPC streams: 30s slow consumer exceeded memory budget and output was not byte-exact. |

## Evidence details

### Cloud Hypervisor CONNECT transport

Result: **pass**.

The proof crate validates the exact host-side shape:

1. connect to the CH base UDS;
2. send `CONNECT <port>\n`;
3. read the complete `OK <buffer-size>\n` line without consuming payload;
4. hand the post-OK stream to `ttrpc::r#async::transport::Socket::new`;
5. construct the client with `Client::new`.

Tests cover success, wrong port refusal, malformed OK, EOF before OK, and
timeout. No host proxy daemon or per-VM host unit is required.

### Static guest build

Result: **pass with implementation constraints**.

The proof crate builds through a Nix static-musl derivation for:

- `x86_64-unknown-linux-musl`
- `aarch64-unknown-linux-musl`

`readelf` evidence shows no ELF interpreter and no `DT_NEEDED`
entries. `cargo-deny` and `cargo-audit` passed for the proof
dependency set.

Raw cargo musl builds failed on the local host because the installed
Rust toolchain lacks musl std targets. The implementation should use Nix
static derivations for guest artifacts, and optionally document raw cargo
as unsupported unless the developer installs the target.

### Generated ttRPC code and unsafe policy

Result: **pass**.

`ttrpc-rust` codegen emits `#![allow(unsafe_code)]`. The proof crate
uses a build step that:

1. generates ttRPC Rust code;
2. asserts the unwanted allowance was present;
3. removes it;
4. scans the generated files for remaining `unsafe` tokens;
5. compiles under `#![forbid(unsafe_code)]`.

Recommended implementation: use an xtask generator or build step that
postprocesses generated ttRPC files and fails the build if generated
code reintroduces unsafe allowance or unsafe tokens.

### Transcript-bound HMAC auth

Result: **pass**.

The prototype binds the HMAC transcript to:

- protocol version;
- purpose;
- direction;
- VM ID;
- CID;
- CH socket identity;
- host nonce;
- guest nonce;
- guest boot ID;
- capabilities hash.

Tests prove cross-VM/CID/socket/direction changes fail verification,
nonce pairs are single-use after successful verification, configured
previous token generations can be accepted during rotation, and errors
and debug output redact token/MAC/path material.

Recommended crates:

- `hmac`
- `sha2`
- `subtle`

These are RustCrypto/pure-Rust dependencies and fit the no-first-party
unsafe rule.

### Safe PTY/job control

Result: **pass for W0**.

The proof crate uses:

- `portable-pty` for PTY open/spawn/resize/I/O;
- safe `nix::sys::signal::killpg` for foreground process-group
  signaling;
- `#![forbid(unsafe_code)]`.

Tests prove a spawned shell can receive input, report the resized
terminal geometry, exit cleanly, and receive SIGINT via the foreground
process group.

Remaining implementation work: user switching and supplementary group
initialization need a safe supported API or a design that delegates
those operations to systemd. Do not add first-party unsafe code.

### ttRPC async stream semantics

Result: **expressive but insufficient alone**.

The stream proof defines a typed `TerminalFrame` protobuf and shows that
ttRPC async duplex streams can express:

- stdin data and close;
- stdout/stderr split;
- TTY merged output;
- resize ordering;
- signal-to-exit mapping;
- exit frames;
- oversized-frame rejection;
- concurrent sessions over one ttRPC client connection.

The fake-transport run achieved interactive p95 latency under 1 ms while
also running slow-output, blocked-stdin, and health streams.

However, this proof did not include a real PTY, real vsock, real HMAC,
or the 30s slow-consumer stress case. Those were covered by separate W0
proofs.

### Raw ttRPC stream backpressure

Result: **fail for raw streams**.

The backpressure proof ran:

- 30s slow consumer;
- 64 MiB stdout;
- 64 MiB stderr;
- 16 MiB stdin;
- four concurrent streams.

Observed:

- idle RSS: 3656 KiB;
- high-water RSS: 152580 KiB;
- stdout/stderr were not byte-exact after the stalled run;
- stdin was not byte-exact;
- oversized messages above ttRPC's 4 MiB max were rejected.

The proof observed that `ttrpc-rust` async connection handling uses
bounded mpsc queues, but message delivery tasks can still accumulate
payloads when an application receiver stalls. Raw ttRPC stream queues
therefore do not satisfy ADR 0026's backpressure and byte-exact
requirements by themselves.

## Protocol lock recommendation

Lock the next design step to:

1. ttRPC/protobuf for guest-control unary APIs;
2. ttRPC duplex stream carrying nixling `TerminalFrame` messages **only
   if** a credit/window layer is added and proves byte-exact bounded I/O;
3. Kata-style chunked stdio as the main fallback for exec I/O if the
   credit/window layer is too complex or fails;
4. custom binary stream only as a last-resort fallback after another
   panel review.

Do not implement raw ttRPC stream forwarding for exec I/O.

## Required follow-up gates

- Design and review the bounded exec I/O shape:
  - ttRPC `TerminalFrame` with credit/window messages, or
  - Kata-style chunked stdio with budgets and cancellation.
- Update ADR 0026 once that bounded exec I/O shape is selected.
- Add generated-code postprocessing to the implementation plan.
- Keep guest binaries static and first-party unsafe-free.
- Preserve old-running-VM SSH compatibility until the documented removal
  gate.
