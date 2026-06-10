# Guest control feasibility dossier

This dossier records the W0 evidence required by
[ADR 0026](../adr/0026-guest-control-plane-over-vsock.md). It is the
panel-review input for locking the guest-control IPC direction.

## Summary recommendation

Use **ttRPC/protobuf for guest-control unary APIs**:

- `Hello`
- `Health`
- `Capabilities`
- exec lifecycle metadata (`ExecCreate`, `ExecInspect`, `ExecLogs`,
  `ExecSignal`, `ExecCancel`)
- typed framework guest operations

Do **not** use raw `ttrpc-rust` async streams alone for Docker-like exec
I/O. W0 backpressure evidence shows that raw async streams can buffer
too much and lose byte-exact delivery under a stalled receiver.

Select **Kata-style chunked stdio RPCs** for exec I/O:

- `ExecCreate`, `ExecInspect`, `ExecWait`, `ExecLogs`, `ExecSignal`,
  resize, and cancellation remain unary ttRPC/protobuf calls.
- `WriteStdin`, `ReadStdout`, `ReadStderr`, and `CloseStdin` move
  bounded byte chunks with explicit offsets, deadlines, and typed
  backpressure/slow-consumer errors.
- Attached CLI exec polls stdout/stderr with short long-poll reads;
  detached exec and logs use the same cursor model over bounded retained
  output.

The credit-window stream overlay remains a viable future design if
chunked stdio fails implementation evidence, but it is not the selected
W0 protocol.

## Proof branches

| Proof | Branch | Commit | Result |
| --- | --- | --- | --- |
| ADR gate | `guest-control-ttRPC` | `c3bd66888722bc03c19f678b3e7da9b23954977e` | ADR 0026 added, then accepted after feasibility evidence and panel review. |
| CH CONNECT transport | integrated W0 decision branch | `a4867b19488a8ebf8e238469287ef4e134815b8a` | PASS: CH post-OK stream can be wrapped in `ttrpc-rust` async `Socket` and `Client` without a host proxy; `OK <local-port>` is validated as an opaque u32 ACK, not used as a buffer limit, malformed/refused ACK failures surface only bounded error categories, handshake timeout is bounded, and post-OK half-close preserves guest output drain. |
| Static guest build | `guest-control-w0-static` | `a085e68be5bfa9ed19fcb3441b4f914c7120ac69` | PASS with implementation constraints: representative ttRPC guest dependency probe builds as static musl for x86_64 and aarch64; real `nixling-guestd`/`nixling-userd` artifacts remain a follow-up implementation gate. |
| ttRPC stream semantics | `guest-control-w0-stream` | `eeaaf881a0aa4b7344b2005290248533a1576605` | CONDITIONAL: duplex streams are semantically expressive, but raw stream queues still need bounded flow control. |
| HMAC auth | `guest-control-w0-auth` | `7a97d09cbd15290d6f738c6bcaaea482bf804324` | PASS: transcript-bound proof-of-possession prototype with redaction and replay tests. |
| Safe PTY | `guest-control-w0-pty` | `72ddbe351bc5c3a70eedfb479f1225017c35790f` | PASS: `portable-pty` plus safe `nix` APIs can cover PTY open/resize/I/O and foreground process-group signaling without first-party unsafe. |
| Strengthened PTY/job-control | `guest-control-w0-pty2` | `eb4fedb2f5128d8a53cadfa9eefc79a23bf32d54` | PASS: session leadership, controlling-terminal foreground ownership, SIGINT after shell job handoff, TIOCSWINSZ/SIGWINCH, PTY EIO/POLLHUP drain, and TTY protocol-side CloseStdin semantics. |
| Generated-code unsafe | `guest-control-w0-codegen` | `06298c0ce0dd48aa5bce5ca6111acb186ebb460d` | PASS: proof build postprocesses ttRPC generated code to remove `#![allow(unsafe_code)]` and verifies no generated unsafe tokens remain. |
| Backpressure | `guest-control-w0-pressure` | `9a849c918c4e97065d78e40e55c99b512de90de6` | FAIL for raw ttRPC streams: 30s slow consumer exceeded memory budget and output was not byte-exact. |
| Guest AF_VSOCK ttRPC server | `guest-control-w0-vsock` + integrated W0 decision branch | `35a25ba2be7e9a88bf367f5554bbb4103fb241ed` + `19dd688fe9f7e0ac905678ba7307a53be5ca416c` | PASS as static compile proof: safe guest `ttrpc-rust` async server/listener shape over `vsock://-1:14318`, plus post-CH-CONNECT Unix stream wrapping with `Socket::new(stream)`; runtime AF_VSOCK tests are cfg-gated for hosts with virtio-vsock. |
| Chunked stdio conformance | integrated W0 decision branch | `9dd7fb36784b81695d92af3ff39361869b6f6ac0` | PASS: executable proof covers 64 MiB stdout + 64 MiB stderr offset reads, TTY stdout-only/stderr-unavailable behavior, zero-length read and append-after-EOF rejection, 16 MiB slow stdin idempotency, deterministic slow-consumer bounds, four-attached-session byte-skew fairness, mixed three-exec plus unary-Health scheduler fairness with capacity saturation, stale restart, EOF vs Ctrl-D, resize/signal/cancel ordering, control/idempotency replay, close-after semantics, terminal-status cursor accounting, and signal exit mapping. |

## Evidence details

### Cloud Hypervisor CONNECT transport

Result: **pass**.

The proof crate validates the exact host-side shape:

1. connect to the CH base UDS;
2. send `CONNECT <port>\n`;
3. read the complete `OK <local-port>\n` acknowledgement without
   consuming payload;
4. hand the post-OK stream to `ttrpc::r#async::transport::Socket::new`;
5. construct the client with `Client::new`.

The `OK` number is Cloud Hypervisor's host-side allocated local port (or
an opaque numeric acknowledgement for nixling's purposes), not a buffer
size. Guest-control must not derive flow-control or ttRPC message limits
from it.

Tests cover success, wrong port refusal, malformed OK, EOF before OK,
timeout, and host-write EOF after OK while guest output continues to
drain. The remaining design cases are locked as follows and must be in
the implementation harness before guest-control ships:

- **guest-side half-close:** after a successful `OK`, guest-side EOF wakes
  pending host reads without implying the host request stream was already
  closed. Host-write EOF preserving guest-output drain is covered by the
  `4c5ededc82fdcfb222e375dafcb528a7d771331d` proof.
- **stale socket after VM restart:** socket existence is not readiness.
  The host must run `CONNECT`, Hello/auth, and Health on every use. A
  stale base UDS or old listener that no longer matches the VM boot ID,
  CID, socket identity, and HMAC transcript returns a typed
  `stale-guest-control-socket`/`stale-session` error and remediation to
  restart or refresh the VM state.
- **guest listener absent / transport unavailable:** CH refusal, EOF,
  malformed/overlong ACK, transport I/O error, or timeout during
  `CONNECT 14318` maps to the bounded Health state
  `transport-unreachable` with the matching bounded reason enum, not
  fallback to SSH for generic exec and not a successful readiness result.

No host proxy daemon or per-VM host unit is required.

### Guest AF_VSOCK ttRPC server

Result: **pass as static compile proof; runtime test cfg-gated**.

Routine developer and CI hosts generally do not expose a guest
AF_VSOCK device, so W0 cannot require a live `bind(AF_VSOCK)` in the
default Layer-1 gate. The committed compile proof
`packages/nixling-host/tests/guest_vsock_ttrpc_compile.rs` type-checks
the safe guest-side shape instead:

1. build a guest-side ttRPC async listener with
   `Listener::bind("vsock://-1:14318")`;
2. attach that listener to `ttrpc::r#async::Server::new()`;
3. type-check wrapping an already-connected post-CH-CONNECT Unix stream
   with `Socket::new(stream)`;
4. keep the proof crate under `#![forbid(unsafe_code)]`.

The host-side transport proof is the CH base-UDS `CONNECT <port>` proof
above, not direct host AF_VSOCK client construction.

`ttrpc-rust`'s Linux vsock transport is implemented with
`tokio-vsock`; Nixling does not call `from_raw_vsock_listener_fd` or any
other unsafe listener constructor. A live runtime test should be added
behind an explicit cfg or integration-test knob on a host/microVM that
provides virtio-vsock. That runtime test must prove:

- guest bind on `VMADDR_CID_ANY:14318`;
- host `CONNECT 14318` through CH and ttRPC Hello/Health over the same
  post-OK stream;
- half-close behavior in both directions;
- typed failure when the guest listener is absent;
- typed stale-socket rejection after VM restart or guest boot-ID change.

### Static guest build

Result: **pass with implementation constraints**.

The representative ttRPC dependency probe builds through a Nix
static-musl derivation for:

- `x86_64-unknown-linux-musl`
- `aarch64-unknown-linux-musl`

`readelf` evidence shows no ELF interpreter and no `DT_NEEDED`
entries. `cargo-deny` and `cargo-audit` passed for the proof dependency
set. The actual `nixling-guestd`, `nixling-userd`, and
`nixling-exec-runner` binaries do not exist in W0; their static package
outputs remain a required implementation-wave gate.

Reproducible proof source:

- branch: `guest-control-w0-static`
- commit: `a085e68be5bfa9ed19fcb3441b4f914c7120ac69`
- derivation: `.w0-static-proof/ttrpc-static-proof.nix`
- probe crate: `.w0-static-proof/probe`

Static derivation invocations:

```console
$ nix-build .w0-static-proof/ttrpc-static-proof.nix \
    --argstr target x86_64-unknown-linux-musl --no-out-link
/nix/store/3n3v6yy65y13gnpk8ja5ash913ns4iyw-nixling-ttrpc-static-proof-x86_64-unknown-linux-musl-0.0.0

$ nix-build .w0-static-proof/ttrpc-static-proof.nix \
    --argstr target aarch64-unknown-linux-musl --no-out-link
/nix/store/j94w60hai3bjm79pxsshmxj7qlh3qqs3-nixling-ttrpc-static-proof-aarch64-unknown-linux-musl-0.0.0
```

The derivation sets `CARGO_BUILD_TARGET` to the requested target
triple and `RUSTFLAGS="-C target-feature=+crt-static"`, then runs:

```sh
readelf -lW "$out/bin/nixling-ttrpc-static-proof" \
  > "$out/readelf-program-headers.txt"
readelf -dW "$out/bin/nixling-ttrpc-static-proof" \
  > "$out/readelf-dynamic.txt" || true
grep -q 'Requesting program interpreter' "$out/readelf-program-headers.txt" \
  && exit 1
grep -q '(NEEDED)' "$out/readelf-dynamic.txt" \
  && exit 1
```

Rerun evidence:

```console
$ file /nix/store/3n3v6yy65y13gnpk8ja5ash913ns4iyw-nixling-ttrpc-static-proof-x86_64-unknown-linux-musl-0.0.0/bin/nixling-ttrpc-static-proof
ELF 64-bit LSB pie executable, x86-64, version 1 (SYSV), static-pie linked, not stripped
$ grep -n 'Requesting program interpreter' /nix/store/3n3v6yy65y13gnpk8ja5ash913ns4iyw-nixling-ttrpc-static-proof-x86_64-unknown-linux-musl-0.0.0/readelf-program-headers.txt || echo 'no Requesting program interpreter'
no Requesting program interpreter
$ grep -n '(NEEDED)' /nix/store/3n3v6yy65y13gnpk8ja5ash913ns4iyw-nixling-ttrpc-static-proof-x86_64-unknown-linux-musl-0.0.0/readelf-dynamic.txt || echo 'no DT_NEEDED entries'
no DT_NEEDED entries

$ file /nix/store/j94w60hai3bjm79pxsshmxj7qlh3qqs3-nixling-ttrpc-static-proof-aarch64-unknown-linux-musl-0.0.0/bin/nixling-ttrpc-static-proof
ELF 64-bit LSB executable, ARM aarch64, version 1 (SYSV), statically linked, not stripped
$ grep -n 'Requesting program interpreter' /nix/store/j94w60hai3bjm79pxsshmxj7qlh3qqs3-nixling-ttrpc-static-proof-aarch64-unknown-linux-musl-0.0.0/readelf-program-headers.txt || echo 'no Requesting program interpreter'
no Requesting program interpreter
$ grep -n '(NEEDED)' /nix/store/j94w60hai3bjm79pxsshmxj7qlh3qqs3-nixling-ttrpc-static-proof-aarch64-unknown-linux-musl-0.0.0/readelf-dynamic.txt || echo 'no DT_NEEDED entries'
no DT_NEEDED entries
```

Supply-chain proof commands:

```console
$ cargo-deny --manifest-path .w0-static-proof/probe/Cargo.toml \
    check --config packages/deny.toml bans licenses sources
bans ok, licenses ok, sources ok

$ nix shell nixpkgs#cargo-audit -c cargo-audit audit \
    --file .w0-static-proof/probe/Cargo.lock \
    --db /nix/store/c6lmvfkhycqqzry2y47245lc5l9xmnph-rustsec-advisory-db-git \
    --no-fetch --format json
{"database":{"advisory-count":1098,"last-commit":null,"last-updated":null},"lockfile":{"dependency-count":138},"settings":{"target_arch":[],"target_os":[],"severity":null,"ignore":[],"informational_warnings":["unmaintained","unsound","notice"]},"vulnerabilities":{"found":false,"count":0,"list":[]},"warnings":{}}
```

The audit run used the same pinned RustSec snapshot as the repository
flake check (`rev 831c50f4a4304068f125e603add6a8839f08b3eb`). It
returned exit code 0. Because the offline run had no crates.io index,
`cargo-audit` also printed non-fatal yanked-metadata lookup errors to
stderr; the JSON report still records zero vulnerabilities and zero
warnings.

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

Reproducible proof source:

- branch: `guest-control-w0-codegen`
- commit: `06298c0ce0dd48aa5bce5ca6111acb186ebb460d`
- crate: `packages/ttrpc-unsafe-proof`

The proof crate was added as a temporary workspace member and built
with the workspace Rust lint `unsafe_code = "forbid"`. Its build
script:

1. invokes `ttrpc_codegen::Codegen`;
2. requires at least one generated protocol source to contain
   `#![allow(unsafe_code)]`;
3. removes that exact crate-level allowance;
4. scans `proof.rs` and `proof_ttrpc.rs` token-by-token for `unsafe`;
5. writes a module shim and lets the crate compile under
   `#![forbid(unsafe_code)]`.

Rerun command and result:

```console
$ cargo test -p ttrpc-unsafe-proof --locked
running 1 test
test tests::generated_server_bindings_compile_under_forbid_unsafe ... ok

test result: ok. 1 passed; 0 failed
```

Generated-code postprocess proof:

```console
$ grep -n 'unsafe' proof.rs || echo 'no unsafe token'
no unsafe token
$ grep -n 'unsafe' proof_ttrpc.rs || echo 'no unsafe token'
no unsafe token
$ grep -n '#!\[allow(unsafe_code)\]' proof*.rs || echo 'no generated allow(unsafe_code)'
no generated allow(unsafe_code)
```

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
- `#![forbid(unsafe_code)]`.

Tests prove a spawned shell can receive input, report the resized
terminal geometry, exit cleanly, and expose the kernel job-control
state needed by the implementation:

- the spawned interactive shell is the session leader and process-group
  leader for its controlling terminal;
- the PTY master reports a foreground process group via `tcgetpgrp`
  once the shell hands the terminal to a foreground child group;
- a terminal `^C` byte is delivered by the kernel as SIGINT to that
  foreground child group, after which the shell regains the terminal and
  reports the child status as `130`;
- a single `TIOCSWINSZ` resize delivers one SIGWINCH with the new size,
  and unchanged sizes are deduplicated before issuing another resize;
- slave close is treated as output EOF only after buffered PTY output has
  been drained, including Linux `EIO`-on-master-read behavior after the
  slave closes;
- TTY `CloseStdin` is protocol-side input closure only: the proof keeps
  the PTY master/writer open and verifies output produced after the
  protocol close marker is still delivered.

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

Panel follow-up corrected the message-size invariant for protobuf
`bytes`: generated decoders may allocate the field before a
`WriteStdin` handler can compare it with `max_chunk_bytes`. The selected
chunked-stdio design therefore treats `ttrpc-rust` 0.9.x's fixed 4 MiB
frame limit as the selected pre-handler allocation bound. Messages above
4 MiB are rejected by ttRPC framing; messages at or below 4 MiB may
allocate one transport/protobuf payload before application admission.
The bounded post-decode design then limits fan-in to four concurrent
`WriteStdin` handlers per connection, a 16 MiB per-connection
decoded-byte semaphore, and one per-exec stdin in-flight permit. An
effective `max_chunk_bytes + 1` request may allocate one bounded decoded
protobuf value, but it is rejected before session-buffer copy or stdin
queueing. Lowering the pre-handler bound requires a pre-ttRPC frame
limiter or patched/configurable ttRPC transport and a new proof.

## Port registry

| Port | Direction | Owner | Status |
| ---: | --- | --- | --- |
| `14317` | guest-to-host | Observability OTLP/Alloy relay | Existing; not available for guest control. |
| `14318` | host-to-guest | `nixling-guestd` ttRPC control | Reserved for Hello, Health, capabilities, lifecycle, exec chunked stdio, and framework guest operations. |
| `14319` | host-to-guest | Future guest-control stream side channel | Reserved but unused by W0; requires a new panel-approved decision before use. |

The control reservation starts at `14318` specifically to avoid
colliding with the existing observability port `14317`. The two surfaces
also differ by protocol and direction: `14317` carries OTLP relay traffic
and may be guest-to-host, while `14318` carries authenticated ttRPC
guest-control traffic after the CH `CONNECT` handshake and HMAC
Hello/auth. No guest-control RPC may reuse `14317`.

## Long-poll and overload caps

Default per-VM caps:

| Budget | Default | Overload behavior |
| --- | ---: | --- |
| Concurrent exec sessions | 32 per VM | `exec-capacity-exceeded`; no process is spawned. |
| Attached exec sessions | 8 per VM | `exec-attach-capacity-exceeded`; detached execs continue. |
| Pending `ReadStdout` waits | 1 per exec per connection, 64 per VM | New duplicate wait is `superseded-read-wait` or `read-wait-capacity-exceeded`. |
| Pending `ReadStderr` waits | 1 per exec per connection, 64 per VM | Same as stdout; TTY mode returns `tty-stderr-unavailable`. |
| Pending `ExecWait` calls | 1 per exec per connection, 64 per VM | Duplicate wait is superseded or rejected with `wait-capacity-exceeded`. |
| Long-poll timeout | 100 ms default, 1 s hard max | Server clamps higher requests. |
| ttRPC request rate | 200 RPC/s per connection, 1000 RPC/s per VM burst | Excess returns `rate-limited` with bounded retry-after. |
| Retained output | 16 MiB stdout + 16 MiB stderr per detached exec, 512 MiB per VM | Older detached bytes are evicted with explicit dropped-byte accounting; attached readers get `offset-expired`. |

All overload errors are typed protocol errors with bounded fields only:
limit name, effective limit, current count where safe, and retry/remedy
enum. They must not include argv, environment, stdout/stderr payloads,
socket paths, tokens, MACs, or guest-derived free-form strings.

### Chunked stdio conformance

Result: **pass for the selected protocol model**.

The executable proof crate at
`proofs/chunked-stdio-conformance` models the selected Kata-style
chunked stdio protocol with bounded unary RPC semantics and safe Rust
only. Run the proof with:

```bash
cargo test --manifest-path proofs/chunked-stdio-conformance/Cargo.toml
```

It validates:

- 64 MiB stdout and 64 MiB stderr byte-exact delivery through
  independent `ReadStdout`/`ReadStderr` offset cursors;
- 16 MiB slow stdin through `WriteStdin`, including exact-offset
  appends, same-request duplicate replay acceptance, different-request
  stale replay rejection, offset-gap rejection, stale-data rejection, and
  drainable bounded stdin retention/backpressure;
- simulated pipe and PTY partial child writes draining from the bounded
  stdin queue without duplicate or lost bytes at the RPC offset boundary;
- atomic `WriteStdin.close_after` success, backpressure failure, duplicate
  replay, mismatched-duplicate rejection, and endpoint-specific close
  behavior for pipe and PTY stdin;
- per-connection decoded-byte budget and per-exec stdin permit tests
  bounding malicious concurrent `WriteStdin` fan-in;
- a deterministic active slow-consumer stress that keeps retained output
  under the configured cap while producers continue attempting
  stdout/stderr writes and receive typed `SlowConsumer` pressure rather
  than growing unbounded buffers;
- four concurrent attached sessions with bounded byte-skew fairness;
- a mixed deterministic scheduler with slow-output exec, blocked-stdin
  exec, interactive echo exec, and unary Health RPC capacity saturation,
  with bounded service-turn gaps for health and interactive work;
- stale-generation rejection after restart;
- EOF (`CloseStdin` at the next offset) distinct from TTY Ctrl-D
  (`0x04` data through `WriteStdin`);
- resize, signal, and cancel events ordered by one client control
  sequence with `request_id` replay for identical retained requests and
  typed rejection for mismatched duplicate IDs;
- process exit status recorded separately from client controls, visible
  only after preceding output is retained, delivered/acknowledged, or
  explicitly dropped with cursor accounting, with unaccounted output loss
  mapped to a visible protocol-error terminal state and signal exits mapped
  to shell-style `128 + signal` status codes.

SSH compatibility remains design-level rather than a broad executable
prototype:

| VM state | CLI behavior | Compatibility result |
| --- | --- | --- |
| Old running VM without `guest-control` capability and existing SSH-backed command (`config sync`, `vm konsole`) | Keep using that command's current SSH path with `transport: "ssh-compat"` and remediation. | Compatible; no forced restart. |
| Old running VM without `guest-control` capability and new generic exec (`nixling exec`, `nixling vm exec run`) | Return typed `guest-control-unavailable-old-generation`; do not use SSH. | Fail closed; no new generic SSH exec surface. |
| New or restarted VM advertising `guest-control` capability | Use chunked stdio RPCs for exec I/O. | New protocol active. |
| VM restarts while a client holds an old generation token | Reject the next RPC as stale. | Fail closed; client must reconnect/rediscover. |
| Guest-control unavailable but SSH still configured | Fall back only through the documented SSH compatibility path. | Operator-visible old-generation behavior. |
| Future removal gate reached | Remove SSH fallback only after the documented migration gate. | No silent behavior change before the gate. |

## Protocol lock recommendation

Lock the implementation design to:

1. ttRPC/protobuf for guest-control unary APIs;
2. Kata-style chunked stdio RPCs for exec I/O, using explicit byte
   offsets, bounded chunks, short long-poll reads, bounded retained logs,
   typed backpressure/slow-consumer cancellation, and explicit
   post-decode byte-budget semaphores for protobuf `bytes` fields;
3. no raw ttRPC stream forwarding for exec I/O;
4. no custom binary stream unless the selected chunked-stdio path fails a
   later implementation gate and a new panel review approves a fallback.

Do not implement raw ttRPC stream forwarding for exec I/O.

### Bounded exec I/O decision

The bounded candidates both satisfy the ADR terminal matrix on paper:

- the credit-window overlay carries byte-exact `TerminalFrame` messages
  over a ttRPC duplex stream and offers the most Docker-like attached
  full-duplex UX with low interactive latency;
- chunked stdio uses unary ttRPC calls with explicit stdin/stdout/stderr
  offsets, bounded server-owned logs, and short long-poll reads.

Select **chunked stdio** for the W0 implementation path. It better fits
the W0 proof outcomes because raw ttRPC stream buffering was the observed
failure mode, while unary request/response boundaries make allocation,
backpressure, and retry behavior explicit and independently testable.
It also gives detached logs the same byte-cursor contract as attached
exec, avoids a second stream state machine, isolates conformance failures
to individual RPCs, and follows Kata prior art.

The tradeoff is attached UX latency: chunked stdio must keep reads
concurrent and use short long-poll timeouts. That is acceptable because
the W0 proof now locks deterministic fairness/service-gap invariants, and
runtime p95/max latency gates remain part of the implementation test plan.
The credit-window overlay is not selected because its lower-latency duplex
UX comes with higher protocol complexity, more subtle half-close/fairness
interactions, and more implementation risk around ttRPC stream buffering.

## Required follow-up gates

- Keep the executable chunked stdio conformance proof green as the
  production guest-control implementation replaces the model with real
  ttRPC handlers.
- Design and review any remaining production-specific details around
  long-poll deadlines and cancellation.
- Keep ADR 0026 aligned with the selected chunked-stdio outcome.
- Add generated-code postprocessing to the implementation plan.
- Keep guest binaries static and first-party unsafe-free.
- Carry the retained stdout/stderr storage security contract into the
  implementation plan: guest-local runtime/state roots, restrictive
  ownership and modes, symlink-safe traversal, per-user isolation,
  per-exec/per-user/VM quotas, TTL cleanup, and no host-visible retained
  bytes outside explicit logs responses.
- Add canary-based redaction tests for logs, metrics, spans, health, and
  CLI JSON across argv, command lines, cwd, credential paths, HMAC/MAC and
  transcript material, session/exec/request IDs in telemetry,
  guest-derived free-form errors, stdout/stderr payloads, tokens,
  environment values, socket paths, and debug-formatted failures.
- Preserve old-running-VM SSH compatibility until the documented removal
  gate.
