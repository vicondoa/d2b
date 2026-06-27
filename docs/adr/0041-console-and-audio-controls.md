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
- qemu-media must use a broker-owned fd-backed chardev or an equivalent
  broker-owned PTY/fd-store design. A qemu-created path socket is not the
  default because it weakens socket-permission posture and can race with
  stale socket cleanup.
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

### Audio control

Audio policy uses typed state and provider-specific enforcement:

- Local audio state is versioned and written atomically under an
  fd-lifetime lock in `/run/d2b/locks/`. Lock files are persistent
  coordination inodes and must never be unlinked during VM cleanup.
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
ACA guestd misconfiguration, provider guest-control routing, and the
host-only qemu audio subset.

The approach deliberately makes unsupported provider behavior explicit
instead of hiding it behind fallback shells or best-effort state edits.
That keeps the daemon/broker/provider trust boundaries intact while
allowing the same operator commands to work across local VMs and
provider-managed sandboxes.
