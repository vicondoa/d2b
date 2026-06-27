# ADR 0041: Console and audio controls across runtime providers

- Status: Accepted
- Date: 2026-06-27
- Related: ADR 0015 (daemon-only clean break), ADR 0028 (guest
  control plane over virtio-vsock), ADR 0032 (d2b v2 constellation
  control plane), ADR 0034 (storage lifecycle, restart adoption, and
  synchronization), ADR 0036 (qemu-media runtime), ADR 0037 (local
  hypervisor runtime seam), ADR 0039 (constellation persistent shell
  routing)

## Context

The Rust CLI already exposes native `d2b console <vm>` and
`d2b audio ...` command shapes, but both currently return typed
`not-yet-implemented` envelopes. The missing behavior spans more than
one runtime provider:

- Cloud Hypervisor NixOS VMs run the d2b guest stack and can use
  guestd for in-guest console-like terminal and audio policy work.
- qemu-media VMs intentionally do not run guestd. They still need a
  console stream and a host-side/offline audio-policy subset where a
  qemu audio backend is declared.
- ACA sandboxes are provider-managed workloads. They are expected to
  run a guestd-compatible in-sandbox agent and must use the ADR 0032
  provider/relay/peer transport rather than a provider-specific shell
  side channel.

The control plane must preserve d2b's daemon-only architecture: no
per-VM systemd units, no bash fallback, no direct CLI or wlcontrol
state-file mutation, and no host-held provider credentials or raw ACA
resource identifiers in public output.

## Decision

D2b will implement console and audio as provider-capability-aware
daemon surfaces.

### Provider capability matrix

The daemon will resolve a target's runtime/provider before touching
local host state. Provider capability DTOs will explicitly distinguish
console streaming, host audio enforcement, and guest audio enforcement.

| Provider | Console | Audio host enforcement | Audio guest enforcement |
| --- | --- | --- | --- |
| Cloud Hypervisor NixOS | Local hypervisor console backend plus daemon/broker drainer | Host PipeWire/vhost-user-sound controller | guestd over authenticated guest-control |
| qemu-media | QEMU serial chardev/fd backend | Host/qemu audio subset when declared | Unsupported; report `guestEnforcement = "unsupported"` |
| ACA sandbox | Provider guestd terminal/console over ADR 0032 relay/peer transport | None on the local host | guestd-compatible sandbox agent over provider guest-control |

Missing ACA guestd is provider misconfiguration, not an alternate
execution mode. The daemon returns a typed provider-misconfigured error
with remediation; it never falls back to ACA `executeShellCommand` as a
fake console, shell, or audio channel.

### Console transport

`d2b console <target>` will use a shared CLI terminal FSM over a typed
public `ConsoleOp` surface. Console bytes are never logged, audited, or
used as metric labels.

Local hypervisor providers must use a console backend that cannot block
the guest when no operator is attached:

- Cloud Hypervisor may use a reviewed `--serial socket=...` backend only
  if it is proven non-blocking and attach-safe.
- qemu-media must use a broker-owned fd-backed chardev backed by a
  socketpair or broker-opened PTY master, with the relevant fd passed to
  QEMU at launch time via the `chardev fd,fd=N` or `chardev socket,fd=N`
  mechanism. A qemu-created UNIX path socket (`chardev socket,path=...`)
  is not the default for the following reasons:

  1. **Ownership inversion**: With a path socket, QEMU binds and listens
     and the daemon connects. That inverts the fd-ownership relationship;
     the broker loses authority over the listening end.
  2. **Filesystem path exposure**: A path-based UNIX socket is addressable
     by any process that can traverse its parent directories. A broker-held
     fd passed via `SCM_RIGHTS` has no filesystem path after the descriptor
     is passed; there is no path-based access vector.
  3. **Stale socket race**: A previous QEMU crash may leave a stale socket
     file at the expected path. Cleanup requires an unlink-and-rebind
     sequence that races with a reconnect attempt. A broker-owned fd has
     no such leftover; the kernel reclaims resources when the fd closes.
  4. **QEMU socket-permission posture**: QEMU's path socket applies only
     filesystem permissions to restrict connections. A broker-held fd
     enforces the restriction at the kernel level via fd-transfer semantics;
     there is nothing to "connect to" from outside.

  A broker-held PTY master (`chardev pty` equivalent with the master fd
  pre-opened and passed to QEMU) avoids the path-socket problems but
  requires the broker to open the PTY. The socketpair/fd-store design is
  preferred because it does not place any device node in the filesystem
  namespace at all.

- A persistent drainer continuously reads console output into a bounded
  ring buffer. If the console fd is retained by a persistent component
  such as the broker or a broker-spawned helper, that same persistent
  component owns the drainer. A broker-held PTY master with a d2bd-only
  reader is forbidden because daemon restart would pause draining while
  the hypervisor still sees the fd as connected.
- `ReadOutput` responses include enough ring-buffer cursor metadata for
  clients to detect dropped output and fast-forward cleanly.

Provider-managed ACA console attaches over the guestd-compatible
provider transport. The public surface must redact ACA resource ids,
relay coordinates, command payloads, and credentials.

### Console stream isolation and QoS

Console I/O is a continuous streaming workload. If console data and
health/audio/control RPCs share the same transport connection or vsock
channel, a large burst of console output can fill send buffers and
stall time-sensitive RPC traffic.

The daemon must prevent console streams from starving other traffic:

- **Local vsock (Cloud Hypervisor and qemu-media)**: The daemon must use
  a separate vsock CID/port for console streaming, distinct from the
  vsock port used for guestd health checks, audio RPCs, and exec
  sessions. Multiplexing console data and control RPCs over the same
  vsock connection is forbidden. virtio-vsock per-connection flow control
  is credit-based; a stalled consumer on one connection does not affect
  independent connections.
- **ADR 0032 relay/peer transport (ACA sandboxes)**: Console bytes must
  travel on a dedicated logical stream or channel within the relay
  transport, separated from health-check pings, audio policy RPC
  messages, and other control traffic. The relay transport layer must
  not share backpressure state between the console stream and control
  message queues. If the relay protocol provides per-stream priority or
  weight, the console stream must be assigned lower priority than
  health/control messages.
- **Ring-buffer backpressure limit**: The daemon-side ring buffer is
  bounded. When the buffer is full, the drainer drops the oldest bytes
  and records the drop in cursor metadata so clients can detect the gap.
  The drainer must not apply backpressure to the guest console fd; the
  guest must never block on console output regardless of whether any
  operator is attached.
- **Attach/detach with no stall**: An operator attaching to or detaching
  from a console session must not pause draining or cause the guest to
  stall. The persistent drainer owns the console fd continuously; the
  attaching operator session is a secondary reader of the ring buffer,
  not a holder of the console fd.

### Audio control

Audio policy uses typed state and provider-specific enforcement:

- Local audio state is versioned and written atomically under an
  fd-lifetime OFD lock in `/run/d2b/locks/`. The lock is acquired with
  `fcntl(F_OFD_SETLKW)` (blocking exclusive write lock for mutations;
  shared read lock for readers). Lock file descriptors are opened with
  `O_CLOEXEC` so exec'd child processes do not inherit the lock. Lock
  files are persistent coordination inodes and must never be unlinked
  during VM cleanup (the kernel releases the OFD lock when all fds to
  the open file description close, but the inode stays on disk as a
  stable coordination point; unlinking it would silently create a new
  inode on next open, breaking coordination with any process that still
  holds the old fd). The per-VM lock inode footprint is bounded by the
  declared VM name set: exactly one lock file per named VM
  (`/run/d2b/locks/<vm>.lock`) is created at first audio mutation; no
  lock files are created for VMs that have never had an audio mutation.
  The total inodes consumed equals the number of VM names that have ever
  had audio state written in the current host activation. Declared VM
  names are validated at eval time (regex `^[a-z][a-z0-9-]*$`), so the
  inode count is strictly bounded by the operator's declared
  configuration.
- Cloud Hypervisor NixOS VMs apply both host-side PipeWire policy and
  guest-side guestd policy. Host-side `off` requests are fail-closed:
  the host boundary is sealed even if guestd is unresponsive, with a
  degraded result for the guest side.
- qemu-media VMs never call guestd. They may persist offline audio
  policy and apply the declared host/qemu subset while reporting guest
  enforcement as unsupported.
- ACA sandbox audio is remote guestd policy only. The host does not
  create local audio state files, host PipeWire nodes, or broker host
  mutations for ACA sandboxes.

Speaker volume and microphone gain are bounded `0..=100` domain values
validated at the public-wire boundary. Multi-target `audio status`
returns per-target errors/remediations so one misconfigured provider
does not fail the entire status command.

### Desktop control surface

`d2b-wlcontrol` remains a public-socket / official-CLI client. It must
not talk to the broker, use `sudo`, or read/write root-owned d2b state.

The UI should keep VM cards scannable: audio controls are grouped behind
an explicit expanded surface or audio view, while collapsed cards show a
subtle badge for degraded, unsupported, host-only, or provider
misconfigured audio. qemu-media host-side controls stay enabled when the
host subset is supported, with a host-only warning. ACA missing-guestd
states surface as provider misconfiguration with remediation.

Volume/gain sliders send final or debounced mutations only, do not
dismiss the layer-shell popup mid-drag, support keyboard increments, and
preserve slider values across mute toggles.

## Consequences

The implementation must add or update:

- public `ConsoleOp` and `AudioOp` wire DTOs and generated schemas;
- provider capability DTOs covering Cloud Hypervisor, qemu-media, and
  ACA sandbox targets;
- console storage/runtime contracts, including stale local socket cleanup
  and persistent drainer ownership;
- qemu-media argv/storage contracts for the fd-backed console backend;
- guestd audio RPCs and provider guestd routing;
- CLI output schemas, manpages, completions, reference docs, how-to
  guides, and provider capability matrix docs;
- d2b-wlcontrol model, planner, protocol, and UI updates.

Tests must cover provider capability mapping, ring-buffer cursor
behavior, slow/no-client console draining, daemon restart while a
persistent fd owner keeps draining, qemu console fd lifecycle, missing
ACA guestd misconfiguration, provider guest-control routing, the
host-only qemu audio subset, and console-stream starvation of control
RPC traffic (verify that a saturated ring-buffer drainer does not delay
health-check or audio RPC responses on the same transport peer).

The approach deliberately makes unsupported provider behavior explicit
instead of hiding it behind fallback shells or best-effort state edits.
That keeps the daemon/broker/provider trust boundaries intact while
allowing the same operator commands to work across local VMs and
provider-managed sandboxes.
