# ADR 0026: Guest control plane over virtio-vsock

- Status: Accepted
- Date: 2026-06-08
- Related: ADR 0010 (wire protocol and typed errors), ADR 0015
  (daemon-only clean break), ADR 0017 (no bash fallbacks), ADR 0018
  (microvm.nix removal), ADR 0024 (in-VM guest config sync)

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

nixling vm exec run <vm> [same flags] -- <argv...>
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
2. `ReadStdout` and `ReadStderr` read bounded chunks from server-owned
   append-only output logs at explicit byte offsets. In TTY mode, PTY
   output is merged into stdout and stderr reads return a typed
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

Every terminal transport candidate must pass the same matrix:

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
- no token, environment, stdout, or stderr leakage into logs, metrics,
  or JSON envelopes.
- retained stdout/stderr storage is guest-local, bounded by per-exec,
  per-user, and VM-global quotas, protected by ownership/mode and
  symlink-safe traversal, isolated between guest users, and cleaned by TTL
  or explicit removal.

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
- Cross-VM reuse is rejected by VM ID, socket path, CID, and HMAC VM
  binding.
- The port registry owns all guest-control ports. Reserve at least a
  guestd control port and, only if needed, a separate exec stream port:
  `14318` is the host-to-guest `nixling-guestd` ttRPC control port and
  `14319` is reserved for any future panel-approved guest-control
  stream side channel. The selected chunked-stdio design does not use
  `14319`. Existing guest-to-host observability port `14317` remains
  separate: it is owned by OTLP/Alloy relay traffic, uses the
  guest-to-host direction, and must not carry guest-control RPCs.
- Guest-control readiness requires CONNECT, Hello/auth, and Health.
  Socket existence alone is never readiness.
- Health returns a bounded state enum plus bounded reason/remediation
  enums, never guest-derived free-form text or transport/socket paths.
  The closed W0 enum set and allowed state/reason/remediation mappings
  live in
  [the chunked stdio reference](../reference/guest-control-exec-io-chunked-stdio.md#health-status-model).
  W0 reserves these states for the implementing schema: `healthy`,
  `degraded`, `unavailable-old-generation`, `listener-absent`,
  `transport-unreachable`, `auth-failed`, `protocol-mismatch`, and
  `stale-session`. `healthy` requires CONNECT + Hello/auth + Health to
  complete on the same post-CONNECT stream. `degraded` means guestd is
  authenticated and serving Health but one bounded subsystem check
  failed; callers may continue only operations whose capability bit is
  still healthy. The other states are unavailable and map to bounded
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
- The transcript includes host nonce, guest nonce, VM identity, CID and
  socket identity, protocol version, connection purpose, direction,
  guest boot ID, and capabilities hash. Replays are rejected, nonces
  are single-use per connection, and MAC verification is constant-time.
- Operator-supplied token files must pass runtime safety validation:
  regular file, no symlink, not under `/nix/store`, root-owned, not
  group/world readable, and safe parents.
- The token value is never written to the Nix store, public manifest,
  CLI JSON, logs, metrics, CH argv, or user-facing health text.
- Auth failure paths must not log or expose raw tokens, HMAC material,
  transcript bytes containing secrets, credential file paths, or
  derived MACs in logs, metrics, CLI JSON, health text, or typed error
  envelopes.

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
payloads and may appear only in the explicit `ReadStdout`, `ReadStderr`,
`ExecLogs`, attached CLI stdout/stderr, or an operator-requested output
file. They must not be duplicated into host daemon state, broker audit
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
