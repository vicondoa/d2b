# ADR 0028: Guest control plane over virtio-vsock

- Status: Accepted
- Date: 2026-06-08
- Related: ADR 0010 (wire protocol and typed errors), ADR 0015
  (daemon-only clean break), ADR 0017 (no bash fallbacks), ADR 0018
  (microvm.nix removal), ADR 0024 (in-VM guest config sync)

> **Update (W16) — current shipped reality.** The guest-control plane
> described here has landed: `nixling-guestd` serves `Hello`/`Health`/
> `Capabilities`, the bounded `ReadGuestFile` read, and the exec
> lifecycle RPCs over the authenticated vsock channel. `nixling config
> sync` reads the guest config working copy over `ReadGuestFile` (no
> `ssh cat`) and **fails closed** on a VM whose running generation does
> not declare the guest-control transport — the SSH compatibility path
> sketched below is **not yet wired** into the command. `nixling vm
> konsole` runs `nixling vm exec -it` over guest-control (no SSH), and
> the admin-only `nixling vm exec` verb shipped alongside it. The DAG
> readiness node is `guest-control-health`, not `guest-ssh-readiness`.
> Per-VM SSH keys are retained only for the remaining compatibility
> surfaces (notably `usb attach --apply`). There is no separate
> guest-control field on `nixling status` yet. The original decision
> text below is preserved as the historical record.

## Context

Nixling's host control plane is daemon-only: the CLI talks to
`nixlingd`, and privileged host mutation is quarantined in the host
broker. Some framework guest operations still depend on SSH:

- `nixling config sync` pulls the guest-editable config with `ssh cat`.
- `nixling vm konsole` opens a terminal running SSH.
- the process DAG still has a `guest-ssh-readiness` node.

That is the wrong long-term control boundary. SSH is useful as an
operator workload tool, but framework control should not depend on
guest network reachability, host firewall posture, known-host state, or
an SSH user account. The desired end-state is a guest control plane:
host `nixlingd` communicates with an in-guest `nixling-guestd` over
virtio-vsock, and `nixling-guestd` brokers per-user execution to
allowlisted `nixling-userd` instances.

The hard part is the wire protocol. We do not want to invest in a
bespoke protocol if an existing microVM agent practice meets the need.
The feasibility gate therefore targets ttRPC first and requires evidence
before locking the implementation path.

## Prior art

### ttRPC over vsock

Kata Containers' Rust agent is the closest architectural match: a
long-running Rust process inside a VM, controlled by a host runtime over
vsock. Kata documents that its runtime talks to the agent through a
ttRPC API defined by protobuf files. The agent configuration defaults
to a vsock server address.

`ttrpc-rust` describes itself as "GRPC for low-memory environments". It
supports Unix and vsock socket addresses on Linux and has async
client/server support. It also has async stream types and examples for
client-streaming, server-streaming, and bidirectional streaming.

For terminal/exec behavior, the closest public examples are:

- Kata-style chunked stdio RPCs: `ExecProcess`, `SignalProcess`,
  `WaitProcess`, `WriteStdin`, `ReadStdout`, `ReadStderr`,
  `CloseStdin`, and `TtyWinResize`.
- `ttrpc-rust` async bidirectional streams, such as
  `EchoStream(stream EchoPayload) returns (stream EchoPayload)`.

No all-in-one Rust package was found that provides a ready-made
terminal/PTY-over-ttRPC protocol. Nixling must still define its own exec
session semantics, but it should first try to express them through
ttRPC/protobuf rather than inventing a control protocol.

### gRPC over vsock

Tonic is a mature Rust gRPC implementation and can use custom
transports, but it brings HTTP/2, tower/hyper, protobuf/prost, and a
larger static dependency surface. It is appropriate for service APIs
that need broad gRPC interoperability. It is not the first target for a
small in-guest control daemon.

### Vsock proxies

Nitro Enclave deployments often bridge vsock to HTTP or gRPC through a
proxy. That pattern is useful prior art for application traffic, but it
is less direct for nixling's control plane because nixling already owns
both host and guest endpoints and should not add a host proxy daemon or
per-VM host unit for the control path.

### Docker and Kubernetes exec behavior

The user-facing `nixling exec` behavior should follow Docker-style
semantics:

- non-TTY mode keeps stdout and stderr separate;
- TTY mode presents one raw terminal stream with stdout and stderr
  merged by the PTY;
- stdin is closed by default and open only with `-i`;
- terminal geometry is sent initially and on resize;
- command exit status, signals, stream errors, and cancellation are
  surfaced explicitly.

Docker's internal stream format and Kubernetes remotecommand channels
are useful references, but nixling does not promise Docker or Kubernetes
wire compatibility.

The concrete user-facing exec surface to preserve through the feasibility
gate is:

```text
nixling exec <vm> [--interactive|-i] [--tty|-t] [--detach|-d]
  [--user <user>] [--workdir <path>] [--env KEY=VALUE]...
  [--env-file <path>] [--timeout <duration>]
  -- <argv...>

nixling vm exec run <vm> [same flags] [--json with --detach only] -- <argv...>
nixling vm exec inspect <vm> <exec-id> [--json]
nixling vm exec logs <vm> <exec-id> [--stdout] [--stderr] [--json]
nixling vm exec attach <vm> <exec-id>
nixling vm exec kill <vm> <exec-id> [--signal <name-or-number>]
```

Plain `exec` is attached, non-TTY, and has stdin closed unless
`--interactive` is set. `--tty` allocates a PTY and implies merged
stdout/stderr; `--interactive --tty` is the Docker-style shell case.
Arguments after `--` are an argv array, not an implicit shell string.
Attached `exec`/`vm exec run` never mixes metadata JSON with remote
stdout. `--json` is valid only for detached `run` (where no command output
is streamed and stdout contains exec metadata), and for `inspect`/`logs`.
Detached exec is in scope for the full design, but the feasibility gate
only has to prove whether the selected IPC can carry the lifecycle and
stream semantics.
Exit status propagation follows the remote command: normal exit returns
the command's exit code; signal termination returns signal metadata and
shell-style status `128 + signal`. Transport/protocol failures use typed
nixling errors.

## Decision

The feasibility gate targets ttRPC/protobuf for guest-control APIs. Unary
ttRPC calls carry
health, capabilities, lifecycle, inspection, wait, signal, resize, and
log metadata.

Raw `ttrpc-rust` async streams are not selected for Docker-like exec I/O:
the feasibility backpressure proof showed unbounded-enough buffering and
non-byte-exact output when an application receiver stalled.

The selected bounded exec I/O protocol is **Kata-style chunked stdio
over unary ttRPC calls**:

1. `WriteStdin` transfers bounded stdin chunks with explicit byte
   offsets, idempotency metadata, and deadlines.
2. `ReadOutput(stream=stdout|stderr)` reads bounded chunks from
   server-owned append-only output logs at explicit byte offsets. In TTY
   mode, PTY output is merged into stdout and stderr reads return a typed
   unavailable error.
3. `CloseStdin` half-closes stdin independently of output EOF and process
   exit.
4. `TtyWinResize`, `ExecSignal`, `ExecCancel`, `ExecInspect`, and
   `ExecWait` remain typed unary control calls.
5. Attached exec uses concurrent short long-poll reads to preserve
   interactive UX; detached exec and `exec logs` use the same retained-log
   cursor model.

The credit-window ttRPC stream overlay remains documented as a fallback
candidate if chunked stdio fails later implementation evidence. A custom
binary stream or custom JSON control remains a last resort and requires a
new panel-approved decision.

### Detached exec

Detached exec (`exec_create` with `detach=true`) runs a non-interactive
command that outlives the originating connection. It is served entirely by
the root-owned guest daemon (`nixlingd`'s guest counterpart `nixling-guestd`);
there is no per-user `nixling-userd` involvement and no per-user state
directory.

**Slot-based transient units.** Each detached exec occupies one of 32 fixed
slots. guestd launches the per-exec worker through `systemd-run` as a
transient unit named `nixling-exec-<NN>.service` (zero-padded slot index),
scoped to the guest-internal `nixling-exec.slice`. The unit name and its
`ExecStart` argv carry **only** the slot index — never the opaque exec id,
argv, environment, or cwd — so journald/systemd metadata cardinality is
bounded to ≤32 stable values and leaks no command detail. The worker is the
dependency-pure `nixling-exec-runner` binary invoked as
`nixling-exec-runner --serve-exec --slot <NN>`.

**Retained logs and quota.** Each exec retains stdout and stderr in
slot-keyed files under the root-owned, 0700, boot-scoped parent
`/run/nixling-exec/slot-<NN>/`. Each stream is capped at **4 MiB** with
drop-oldest truncation accounting (a truncation marker plus a dropped-byte
counter); there is no per-user quota. The VM-global retained-log quota is
**256 MiB** (32 slots × 2 streams × 4 MiB), enforced as an exact invariant.
Capacity exhaustion (active slots, retained slots, or log quota) is detected
before a unit starts.

**Indefinite runtime + optional ceiling.** A detached exec runs indefinitely
by default (`detachedMaxRuntimeSec = 0`): the runner installs no ceiling
timer and guestd emits no systemd `RuntimeMaxSec`. A long-running `Running`
record is never reaped by TTL/GC. When `detachedMaxRuntimeSec > 0`, the
runner enforces the ceiling (cancel + retain) and guestd emits a
strictly-larger `RuntimeMaxSec` backstop.

**Cancellation.** Cancellation is a two-phase, control-file mechanism with no
in-process signal handler: guestd writes a cancel sentinel and waits for the
runner to publish a terminal status before any `stop_unit`; `stop_unit` is
invoked only if the cancel deadline elapses with no status. The runner's
watcher thread polls the control file, then drives TERM → grace → KILL → reap
on the child's process group and writes the terminal status exactly once
(natural exit vs cancel precedence is resolved exactly-once).

**Re-adoption within one boot.** guestd persists each exec's lifecycle record
(including a `Dispatching` phase written and fsync'd *before* `systemd-run`,
with a persisted `dispatch_deadline`) under the slot dir. On restart guestd
reconciles the full state matrix against an all-states systemd unit query: a
`Dispatching` record with a live authentic unit is adopted as `Running`
(never killed); a `Dispatching` record with no unit is held in-flight within
its `dispatch_deadline` (slot reserved, non-listable) and adopts a
late-registering unit, but is deleted and released past the deadline on a
negative re-query; `infra-failed` rows are cleaned up and released. A vanished
unit with no terminal status is marked lost exactly like a natural
termination — slot, logs, and quota are retained as a terminal record until
TTL/GC. Re-adoption is bounded to a single boot: a boot-id mismatch is a
`StaleSession`.

**Discovery (`ExecList`).** A minimal read-only `ExecList` RPC enumerates the
caller's detached records for the same VM token + boot (bounded ≤32). Each
entry carries the exec id, slot, state, create time, an **argv SHA-256 hash**
(never raw argv), and the per-stream truncation/dropped-byte counters. A job
whose create-reply was lost (crash-after-dispatch, re-adopted) is discoverable
via `ExecList` and cancellable by the returned id. Attached execs are not
listed.

**Retention + eviction.** Terminal records are retained for **30 minutes**
(TTL applies to terminal records only; a `Running` detached job is
indefinite), then GC'd to a tombstone. Three distinct not-available
conditions are reported as distinct error kinds: `StaleSession` (boot
mismatch), `ExecExpired` (retention-evicted / tombstoned), and `ExecNotFound`
(never existed for this caller). An in-flight `ExecLogs` read is guarded
against GC: the read completes with stable bytes/offset, and only after the
guard drops do files and the registry entry drop, with future reads returning
`ExecExpired`.

### Interactive exec

Interactive exec (`exec_create` with `tty=true && detach=false`) routes to a
**PTY-backed, connection-owned, non-durable** attached exec. It is served by
`nixling-guestd` with no per-user `nixling-userd` involvement and no retained
log/registry record: the session lives only for the originating connection.
`tty=true && detach=true` is rejected with a typed `ProtocolError`
(unsupported mode); there is no durable interactive exec.

**Helper-exec, no first-party unsafe.** guestd never performs the
controlling-terminal handshake itself and never acquires a controlling tty.
It allocates the PTY master (`posix_openpt`/`grantpt`/`unlockpt`/`ptsname`)
and spawns a dedicated `--tty-exec` mode of the static `nixling-exec-runner`
helper, passing the slave on stdin and an `O_CLOEXEC` status pipe on stdout
via safe `Stdio::from(OwnedFd)` (no `pass_fds`, no `pre_exec`, no
`process_group`). The helper, in safe `rustix`, dups the status fd high,
`setsid()`s, acquires the slave as its controlling terminal (`TIOCSCTTY`),
`tcsetwinsize`s the initial geometry, `dup2`s the slave onto 0/1/2, and
`execve`s the target. On success the `O_CLOEXEC` status fd closes during
`execve` (EOF == success); on setup/exec failure the helper writes one typed
byte that guestd maps to a typed `ExecCreate` error. The whole guest stack
keeps `unsafe_code = "forbid"`.

**Merged output, stderr disabled.** A PTY has a single output side, so
stdout and stderr are merged onto the stdout stream. `ReadOutput(stream=stderr)`
on a TTY exec returns a typed stderr-unavailable error (the stderr ring is
pre-marked EOF at create).

**Initial geometry.** An absent `initial_terminal_size` defaults to 24 rows ×
80 cols. A present size must be in `1..=65535` for both dimensions; `0` or
out-of-range is rejected as a validation/protocol error.

**Stdin and close.** `WriteStdin` uses the same monotonic-offset, serialized,
bounded-backpressure machine as non-TTY exec. `CloseStdin` injects VEOF
(`0x04`) into the PTY and keeps the master open (it is **not** a master-close
half-close); it is idempotent, and a subsequent `WriteStdin` is rejected with
a typed stdin-closed error. `WriteStdin` with `close_after=true` writes the
data and then injects VEOF.

**Resize and signal.** `TtyWinResize` and `ExecSignal` are serialized through
the same per-exec strictly-increasing `control_seq` dispatcher (stale,
duplicate, and out-of-order sequences are rejected). `ExecSignal` is TTY-only
and rejects any `SignalTarget` other than `FOREGROUND_PROCESS_GROUP`; the
target is resolved via `tcgetpgrp(master)` **at delivery time**. The delivered
signal must be in the allowlist
`INT/TERM/HUP/QUIT/WINCH/USR1/USR2/KILL/TSTP/CONT`. An invalid target and an
invalid signal number are both rejected (as `protocol-error`) **before** the
control sequence is consumed, so the client may retry at the same seq.

**Indefinite runtime scoped to TTY.** A TTY exec runs indefinitely by default
(`interactiveMaxRuntimeSec = 0` ⇒ unlimited), or under an optional ceiling
when `interactiveMaxRuntimeSec > 0`. The 6-hour non-TTY attached ceiling is
unchanged; only the interactive path opts into unlimited runtime.

**Teardown and in-session no-orphan.** Teardown moves the session
`Running → Closing → Terminal`. Entering `Closing` atomically rejects new
stdin/control RPCs (typed no-op/error, no side effect), drops pending accepted
writes, stops issuing master clones, releases handles, and drops the last
master (delivering `SIGHUP`); after a bounded grace it `SIGKILL`s the whole
TTY **session** (sid enumeration via `/proc`, repeated until empty) and reaps.
The no-orphan guarantee is scoped to in-session processes; a `setsid`/
double-fork escapee is a documented trusted-root limitation. Teardown is
idempotent and runs on child exit, ceiling expiry, explicit cancel, and host
disconnect.

**Capabilities.** `ExecTty`, `TtyResize`, and `Signals` are advertised only
when the interactive path is usable (the PTY spawner is wired, which requires
the exec-runner helper to be present).

The full interactive contract — mode matrix, helper-exec handshake, merged
output, VEOF close, resize/signal ordering, and teardown — is specified in the
[interactive TTY exec reference](../reference/guest-control-exec-interactive-tty.md).

## Feasibility gate

The feasibility dossier must include:

- a representative static-musl ttRPC guest dependency probe for
  `x86_64-linux` and `aarch64-linux`;
- the representative probe must target `x86_64-unknown-linux-musl` and
  `aarch64-unknown-linux-musl`, or an equivalent Nix static-musl target
  that produces target-native static Linux binaries;
- cargo-deny/cargo-audit results for ttRPC, protobuf/codegen, and
  transitive dependencies;
- proof that generated protobuf/ttRPC code does not weaken
  `unsafe_code = "forbid"` in the new guest crates;
- proof that the representative static probe has no ELF interpreter or
  `DT_NEEDED` dynamic dependencies;
- exact commands or Nix derivations used for static evidence. At
  minimum the W0 check shape is:

  ```text
  nix build .#w0-static-proof-x86_64-unknown-linux-musl
  nix build .#w0-static-proof-aarch64-unknown-linux-musl
  readelf -lW <proof-binary>   # no INTERP
  readelf -dW <proof-binary>   # no NEEDED
  cargo deny check bans licenses sources
  cargo audit --no-fetch
  ```

Real `nixling-guestd`, `nixling-userd`, and `nixling-exec-runner`
static/no-unsafe artifacts are not produced by W0. They are a required
implementation gate before any guest-control release.

  The final implementation may use Nix static derivations instead of
  raw cargo commands, but the emitted evidence must prove the same
  properties;
- a host prototype that performs the Cloud Hypervisor
  `CONNECT <port>` handshake and wraps the post-connect
  `tokio::net::UnixStream` with `ttrpc::r#async::transport::Socket`
  and `Client::new`;
- a guest prototype serving ttRPC on AF_VSOCK through supported safe
  crates;
- a transcript-bound HMAC authentication proof of concept;
- typed error and version-negotiation mapping;
- generated API documentation/schema integration plan;
- terminal transport conformance results for each candidate.

If any must-pass item fails, the dossier records the concrete failure and the
next candidate is evaluated. A fallback cannot be selected by
preference alone.

## Terminal conformance matrix

Every terminal transport candidate must pass the W0 transport portion of
the same matrix:

- stdin open/close behavior, including EOF and TTY Ctrl-D distinction;
- stdout/stderr separation in non-TTY mode;
- stdout/stderr merge in TTY mode;
- initial terminal geometry and resize ordering, including SIGWINCH
  propagation to the guest PTY;
- PTY session leadership, controlling-terminal setup, and foreground
  process-group ownership;
- Ctrl-C/signal delivery to the foreground process group;
- command exit code/signal propagation;
- bounded memory under slow stdout/stderr/stdin consumers;
- backpressure under large output and blocked stdin;
- concurrent exec sessions over one ttRPC client connection, covering
  per-stream fairness, head-of-line blocking, bounded internal queues,
  and backpressure when one stream stalls;
- cancellation and host-disconnect cleanup;
- half-close behavior;
- guestd restart and daemon restart behavior;
- VM reboot and stale-session rejection;
- terminal raw-mode restoration in the CLI;
- max message/frame size enforcement before handler/protobuf admission
  at the selected transport's available cap, and bounded post-decode
  allocation with explicit semaphore budgets;
- typed protocol errors for malformed messages;
- implementation gate: no token, environment, stdout, or stderr leakage
  into logs, metrics, audit records, health/status, or JSON envelopes.
  W0 records the closed redaction contract and canary matrix; production
  guestd/userd implementations must provide the canary tests.
- implementation gate: retained stdout/stderr storage is guest-local,
  bounded by per-exec, per-user, and VM-global quotas, protected by
  ownership/mode and symlink-safe traversal, isolated between guest users,
  and cleaned by TTL or explicit removal. W0 records the storage contract;
  production retained-log backends must provide storage/ACL/quota tests.

The dossier must quantify the stress cases it runs. The exact numbers may be
adjusted during implementation, but the dossier must record payload
sizes, stall durations, frame/message limits, maximum resident memory
observed, and expected typed error behavior for oversized input,
distinguishing transport receive-cap rejection before protobuf decode
from bounded post-decode application-limit rejection.

Initial pass/fail thresholds are:

- non-TTY large-output test: at least 64 MiB stdout and 64 MiB stderr
  from one process, with byte-exact demultiplexed output;
- stdin pressure test: at least 16 MiB stdin into a process that reads
  slowly, with bounded buffering and correct close-stdin behavior;
- slow-consumer test: deterministically pause each host-side
  stdout/stderr consumer while the guest process continues writing;
  memory must remain bounded and the remote writer must block or receive
  a typed slow-consumer cancellation rather than unbounded buffering.
  Real implementation soak gates may add a separate 30-second runtime
  variant, but the default W0 proof must not depend on wall-clock timing;
- frame/message limit test: send one message at the configured maximum,
  one byte above the effective application chunk limit, and one byte
  above the ttRPC frame cap. The frame-cap violation is rejected before
  handler/protobuf admission; the effective-limit violation may allocate
  one bounded decoded protobuf `bytes` field but must be rejected before
  session-buffer copy while holding the documented byte-budget permits;
- fake-scheduler fairness tests: one test covers four concurrent attached
  exec sessions with bounded byte-skew; a separate mixed workload covers
  slow-output exec, blocked-stdin exec, interactive TTY exec, and a unary
  Health RPC loop sharing the scheduler, with bounded service-turn gaps
  for interactive and health work and no byte-skew starvation;
- memory high-water mark: raw-stream candidates must record idle RSS and
  test RSS because transport queues can grow outside the application
  model. The selected chunked-stdio W0 proof records retained-byte,
  decoded-byte, and queue budgets deterministically; implementation
  runtime gates must add RSS high-water evidence and fail if resident
  memory grows without bound or exceeds the recorded per-session budget.
  The initial per-session runtime budget is 64 MiB above idle.

Each candidate must also produce byte-exact transcripts for the matrix,
fail on reordered resize/signal/cancel control events, and keep terminal
process status hidden until output preceding terminal observation is
retained, delivered/acknowledged, or explicitly dropped with cursor
accounting. Real-transport implementation tests must record p50/p95/max
latency; deterministic W0 proofs record service-turn gaps. Concurrent
proof must run slow-output exec, blocked-stdin exec, interactive TTY exec,
and unary Health work together and show no head-of-line blocking, no
starvation, and bounded memory.

Nice-to-have properties such as lower latency, lower dependency
footprint, and simpler docs can break ties, but they cannot compensate
for failing a must-pass row.

## Transport invariants

- Each VM has one bundle-derived Cloud Hypervisor vsock base UDS path
  for guest control, under a daemon/runtime-owned per-VM directory.
  The parent directory is not world-searchable; the socket is mode
  `0660` or stricter and owned so only nixlingd/runner authority can
  open it.
- CLI users never receive the CH base UDS path.
- Cross-VM reuse is rejected by host-side CH socket selection and peer checks,
  plus guest-side HMAC binding over the shared guest-observable VM/CID/port
  transcript.
- The port registry owns all guest-control ports. Reserve at least a
  guestd control port and, only if needed, a separate exec stream port:
  `14318` is the host-to-guest `nixling-guestd` ttRPC control port and
  `14319` is reserved for any future panel-approved guest-control
  stream side channel. The selected chunked-stdio design does not use
  `14319`. Existing guest-to-host observability port `14317` remains
  separate: it is owned by OTLP/Alloy relay traffic, uses the
  guest-to-host direction, and must not carry guest-control RPCs.
- Guest-control readiness requires CONNECT, unauthenticated Hello challenge,
  Authenticate proof-of-possession, and authenticated Health.
  Socket existence alone is never readiness.
- Health returns a bounded state enum plus bounded reason/remediation
  enums, never guest-derived free-form text or transport/socket paths.
  The closed W0 enum set and allowed state/reason/remediation mappings
  live in
  [the chunked stdio reference](../reference/guest-control-exec-io-chunked-stdio.md#health-status-model).
  W0 reserves these states for the implementing schema: `healthy`,
  `degraded`, `unavailable-old-generation`, `listener-absent`,
  `transport-unreachable`, `auth-failed`, `protocol-mismatch`, and
  `stale-session`. `healthy` requires CONNECT + Hello challenge +
  Authenticate + Health to complete on the same post-CONNECT stream.
  `degraded` means guestd is authenticated and serving Health but one bounded
  subsystem check failed; callers may continue only operations whose capability
  bit is still healthy. The other states are unavailable and map to bounded
  remediation enums such as `restart-vm`, `retry`, `upgrade-guest`, or
  `check-auth-token`.
- No host proxy daemon or per-VM host systemd unit may be introduced
  for guest control.
- Host CONNECT setup is part of the transport contract: connect to the
  CH base UDS, send exactly `CONNECT <port>\n`, read and validate the
  full `OK <local-port>\n` acknowledgement without consuming payload
  bytes, and only then hand the raw post-OK byte stream to ttRPC. The
  numeric value is the host-side allocated local port/opaque
  acknowledgement from Cloud Hypervisor; nixling must not derive buffer
  sizes, flow-control windows, or ttRPC limits from it. The feasibility
  proof covers success, refusal, malformed reply, timeout, EOF before OK,
  and host-write EOF after OK while guest output continues to drain. The
  implementation harness before guest-control ships must also cover
  guest-side EOF, stale socket after VM restart, and guest listener
  absence. Half-close means EOF on one side is propagated without
  treating the opposite direction as already closed; stale sockets and
  absent listeners fail readiness with typed transport errors and never
  degrade to socket-existence success.
- The CH CONNECT harness must reject wrong ports and then wrap the same
  accepted stream as the ttRPC client/server transport so the test
  proves the real handoff shape, not just the textual prelude.
- Guest-side ttRPC serving uses `ttrpc-rust`'s safe async listener API
  on `vsock://-1:14318`, which is backed by `tokio-vsock` on Linux. The
  feasibility gate
  carries a compile-only proof for this shape because routine CI hosts
  do not expose a guest AF_VSOCK device. Runtime vsock tests remain
  cfg-gated to hosts or microVMs that provide virtio-vsock.

## Security invariants

- No first-party unsafe code is added for this work. New guest crates
  keep `unsafe_code = "forbid"` and must not add `unsafe` blocks or
  `#[allow(unsafe_code)]`.
- Low-level AF_VSOCK, PTY, raw terminal, process-group, and user
  switching work must use supported crates with safe APIs. If no safe
  supported crate satisfies a requirement, implementation pauses for a
  new design review.
- Guestd consumes its token through systemd `LoadCredential`.
- The guest-control token is never sent over vsock. Authentication is
  proof-of-possession only.
- Host-side proof generation goes through `nixling-priv-broker`'s structured
  guest-control signer. `nixlingd` sends typed transcript fields and receives
  only a fixed-size HMAC tag; the broker keeps token bytes confined to the
  privileged process and audits only bounded metadata.
- The guest-verified auth transcript includes only values both sides can
  obtain from trusted local context: host nonce, guest nonce, VM identity,
  guest-control port, observable peer/host CID when available, protocol
  version, connection purpose, direction, guest boot ID, and, for the guest
  proof, the authenticated capabilities hash. Host-local Cloud Hypervisor base
  socket identity is not in the guest HMAC; it remains a host connector
  precondition combined with the guest proof. Replays are rejected, nonces are
  single-use per connection, and MAC verification is constant-time.
- Operator-supplied token files must pass runtime safety validation:
  regular file, no symlink, not under `/nix/store`, root-owned, mode `0400` or
  the materialized `0440 root:nixling-<vm>-gctlfs` share-reader posture, and
  safe parents.
- The token value is never written to the Nix store, public manifest,
  CLI JSON, logs, metrics, CH argv, or user-facing health text.
- Auth failure paths must not log or expose raw tokens, HMAC material,
  transcript bytes containing secrets, credential file paths, or
  derived MACs in logs, metrics, CLI JSON, health text, or typed error
  envelopes.

Canonical transcript encoding is binary and versioned. Each field is encoded
as one tag byte, a four-byte big-endian length, then the exact byte value.
Malformed, duplicate, missing, overlong, or alternate encodings are rejected
before MAC verification.

| Tag | Field | Source |
| ---: | --- | --- |
| 1 | `guest-control-auth-v1` domain label | constant |
| 2 | proof role (`host-proof` or `guest-proof`) | local verifier/signer |
| 3 | direction (`host-to-guest`) | trusted connection context |
| 4 | purpose (`guest-control-auth-v1`) | trusted connection context |
| 5 | VM id | guest configuration / host manifest |
| 6 | protocol version | negotiated guest-control protocol |
| 7 | guest-control port (`14318`) | trusted listener/connector context |
| 8 | observable peer/host CID, when available | trusted listener context |
| 10 | host nonce, 32 raw bytes | `HelloRequest` validated length |
| 11 | guest nonce, 32 raw bytes | generated challenge state |
| 12 | guest boot id | guest boot-id source |
| 13 | capabilities hash | guest proof only, after Authenticate |

The server still tracks a private per-accepted-connection instance for
challenge lookup, replay rejection, and cleanup. That value is not part of the
HMAC transcript because the host cannot know it until the guest exposes a
separate public connection nonce; the authenticated guest nonce is the
connection-bound challenge shared over the protocol.

## Observability contract

Guest-control health, status, CLI JSON, and metrics must use bounded
fields. They must not include free-form guest text, command argv, cwd,
environment, token material, stdout/stderr payloads, CH socket paths, or
session transcripts.

Exec stream telemetry may report only aggregate counters/histograms such
as bytes, frames, backpressure events, cancellations, terminal errors,
and protocol errors. Labels and span attributes are limited to bounded
values such as VM, env, subsystem, outcome, error kind, protocol version,
and stream kind. They must not include session IDs, commands, user names
beyond existing bounded VM/user policy fields, environment values,
payloads, or guest-derived free-form error strings.

Structured logs for exec streams follow the same rule as metrics and
spans. They may contain only bounded aggregate fields such as VM, env,
stream kind, byte counts, frame counts, bounded outcome, bounded error
kind, and truncation/cancellation booleans. They must not log per-frame
payloads, session IDs, commands, user or environment values, CH socket
paths, guest-derived free-form errors, stdout/stderr bytes, transcript
bytes, tokens, MACs, or credential paths.

Retained stdout/stderr bytes are not observability data. They are command
payloads and may appear only in the explicit `ReadOutput`, `ExecLogs`,
attached CLI stdout/stderr, or an operator-requested output file. They
must not be duplicated into host daemon state, broker audit
records, health responses, traces, metrics, structured logs, bundle
manifests, or host sidecar directories. JSON responses that are not the
explicit logs API carry offsets, counters, booleans, bounded error kinds,
and remediation only.

The implementation test plan must include canary-based redaction coverage
for:

- argv and command lines;
- cwd and environment values;
- token values, env-file names, credential paths, HMAC MACs, and auth
  transcript material;
- Cloud Hypervisor/vsock/socket paths;
- session IDs, exec IDs, and request IDs in telemetry surfaces;
- guest-derived free-form errors;
- stdout/stderr payload bytes;
- debug/display formatting of transport and auth failures.

Each canary must be asserted absent from daemon/guestd/userd logs,
metrics, spans/events, health output, and CLI JSON error envelopes across
success, auth failure, protocol failure, stale-session, quota failure,
attached, detached, TTY, and non-TTY paths. The only allowed matches are
the explicit payload stream requested by the user and stable CLI JSON
fields whose contract intentionally returns values such as `execId`.

Retained-log storage has its own security gate. File-backed logs must live
under guest-local guestd runtime/state directories, never under
`/nix/store`, host-shared mounts, virtiofs exports, host bundle state, or
host audit/observability state. Directories and segments are created with
restrictive ownership and mode, path traversal is rooted at pre-opened
directory file descriptors, symlinks and unsafe parents are rejected, and
cleanup unlinks by directory file descriptor. Tests must prove per-user
isolation, symlink/hard-link rejection, quota enforcement, TTL/startup
cleanup, and absence of retained bytes from host-visible state except the
intentional logs API response.

## Backward compatibility

Mixed-generation support is mandatory. A host may update before a
running VM restarts into a guestd-capable generation.

Existing SSH-backed commands keep their current behavior for old
running VMs through the first release that ships guestd plus one
following minor release. The exact release numbers are set when the
feature lands, but the window must not be shorter than one minor
release after the first guestd-capable release:

- `nixling config sync`;
- `nixling vm konsole`;
- current SSH-key/known-host convenience paths.

Framework guest operations such as `config sync` prefer guestd when it
is healthy. If guestd is absent because the running VM is old and SSH
metadata exists, they use the existing SSH path, emit
`transport: "ssh-compat"` in JSON, and print human remediation to
restart/switch the VM. Human remediation names the VM and the exact
command, for example `nixling vm restart <vm>` or
`nixling switch <vm> --apply`, depending on the command context. JSON
uses a stable typed error/remediation shape with fields for `kind`,
`vm`, `transport`, and `remediation`.

`nixling vm konsole` is an explicit compatibility exception because it
is a user-facing SSH convenience rather than a framework guest
operation. It may keep the SSH path for guestd-capable VMs until a
documented conversion gate turns it into a guest-control wrapper. That
gate must update this ADR's follow-up docs and tests so implementers do
not silently change terminal behavior mid-window.

The new `nixling vm exec` / `nixling exec` command does not fall back
to SSH. On old running VMs it returns a typed
`guest-control-unavailable-old-generation` error with remediation.

Operators discover old VMs before the implementing release through the
compatibility warning/remediation emitted by SSH-backed commands. The
release that implements guest-control must add a versioned status JSON
field for guest-control state, such as `unavailable-old-generation`, and
update the status schema/docs in the same change; W0 does not add that
field to the current frozen status schema. During the compatibility
window, every SSH compatibility use emits a deprecation warning.
Removing the compatibility path requires a follow-up ADR or changelogged
release gate with tests proving no old-generation VMs remain in the
supported upgrade path.

The compatibility test matrix must cover:

- `config sync` with guestd healthy: uses guest control and reports that
  transport;
- `config sync` with an old running VM, SSH metadata present, and guestd
  absent: succeeds through SSH with `transport: "ssh-compat"` and
  remediation;
- `config sync` with an old running VM but missing SSH key/known-host
  metadata: fails with the existing SSH/key diagnostic plus
  guest-control remediation, not silent success;
- `vm konsole` with guestd healthy: either uses the new guest-control
  path once implemented or remains an explicitly documented SSH
  convenience until converted;
- `vm konsole` with an old running VM and SSH metadata present: keeps
  the existing SSH behavior and emits the compatibility warning;
- the implementing release updates `nixling status` and
  `nixling vm status <vm>` with a schema-versioned guest-control state
  field exposing `unavailable-old-generation` and the remediation
  command;
- `nixling exec` and `nixling vm exec run` never fall back to SSH and
  return typed `guest-control-unavailable-old-generation` on old VMs;
- human output includes the VM name and exact remediation command;
- JSON output includes stable `kind`, `vm`, `transport`, and
  `remediation` fields where applicable.

## Consequences

Positive:

- Nixling follows existing microVM-agent practice before inventing a
  custom control protocol.
- The protocol choice is evidence-driven and panel-gated.
- Existing SSH workflows keep working for old running VMs during the
  compatibility window.
- The no-new-unsafe and static guest-binary invariants stay explicit.

Negative:

- The feasibility gate must complete before most guest-control
  implementation can be
  parallelized safely.
- ttRPC/protobuf introduces a second generated-contract toolchain if
  selected.
- Exec I/O still needs a new chunked-stdio contract, offset state
  machine, and retained-log implementation.

## Alternatives considered

- **Keep SSH for framework operations:** rejected for the end-state
  because it couples framework control to guest networking, SSH
  accounts, known-host state, and firewall posture.
- **Use gRPC/tonic by default:** rejected as the first target because
  it is heavier than needed for a small VM agent and brings HTTP/2
  complexity.
- **Start with a bespoke JSON protocol:** rejected as premature unless
  feasibility evidence proves ttRPC cannot meet the requirements.
- **Use a host proxy daemon:** rejected because it would add another
  persistent host surface and complicate the daemon-only model.
- **Fallback to SSH for generic `nixling exec`:** rejected because it
  would introduce a new generic SSH exec surface. Compatibility SSH is
  limited to commands that already use SSH today.

## References

- [ADR 0010 — Wire protocol and typed errors](0010-wire-protocol-and-typed-errors.md)
- [ADR 0015 — Daemon-only clean break](0015-daemon-only-clean-break.md)
- [ADR 0017 — No bash fallbacks invariant](0017-no-bash-fallbacks-invariant.md)
- [ADR 0024 — In-VM guest config editing, sync, and containment](0024-in-vm-guest-config-sync.md)
- [Kata agent README: ttRPC/protobuf API](https://github.com/kata-containers/kata-containers/blob/6d2066b692ce69a908bb4daec2c6b71ccfad3829/src/agent/README.md#L61-L64)
- [Kata agent README: vsock server address](https://github.com/kata-containers/kata-containers/blob/6d2066b692ce69a908bb4daec2c6b71ccfad3829/src/agent/README.md#L142)
- [Kata agent protocol: stdio RPCs](https://github.com/kata-containers/kata-containers/blob/6d2066b692ce69a908bb4daec2c6b71ccfad3829/src/libs/protocols/protos/agent.proto#L33-L49)
- [ttrpc-rust README](https://github.com/containerd/ttrpc-rust/blob/master/README.md)
- [ttrpc-rust crate docs: socket addresses](https://docs.rs/ttrpc/0.9.0/ttrpc/#socket-address)
- [ttrpc-rust async transport source](https://docs.rs/ttrpc/0.9.0/src/ttrpc/asynchronous/transport/mod.rs.html#22-29)
- [ttrpc-rust async stream types](https://docs.rs/ttrpc/0.9.0/ttrpc/asynchronous/index.html)
- [ttrpc-rust streaming example proto](https://github.com/containerd/ttrpc-rust/blob/5e2b5068bb05a23619a0c9de0aa98ef715155552/example/protocols/protos/streaming.proto#L29-L36)
- [Docker exec CLI reference](https://docs.docker.com/reference/cli/docker/container/exec/)
- [Docker exec API](https://github.com/moby/moby/blob/22de4f231ce26e26f0bdf41096321e667bd54d84/api/docs/v1.44.yaml#L9786-L9948)
- [Docker stream format](https://github.com/moby/moby/blob/22de4f231ce26e26f0bdf41096321e667bd54d84/api/docs/v1.44.yaml#L8032-L8075)
- [Kubernetes remotecommand constants](https://github.com/kubernetes/apimachinery/blob/master/pkg/util/remotecommand/constants.go)
- [Kubernetes remotecommand stream options](https://github.com/kubernetes/client-go/blob/master/tools/remotecommand/remotecommand.go#L31-L37)
- [AWS Nitro Enclaves vsock proxy](https://github.com/aws/aws-nitro-enclaves-cli/blob/main/vsock_proxy/README.md#L1-L4)
- [AWS Nitro Enclaves vsock proxy usage](https://github.com/aws/aws-nitro-enclaves-cli/blob/main/vsock_proxy/README.md#L27-L65)
- [`nixos-modules/nixling-ch-vsock-connect.nix`](../../nixos-modules/nixling-ch-vsock-connect.nix)
- [`packages/nixling-host/src/ch_argv.rs`](../../packages/nixling-host/src/ch_argv.rs)
