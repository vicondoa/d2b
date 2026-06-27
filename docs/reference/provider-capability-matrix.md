# Provider capability matrix — console and audio

**Diataxis category:** reference.

This page is the canonical reference for the console and audio
capability boundaries across the three d2b runtime providers. The
architectural decision that grounds this matrix is
[ADR 0041](../adr/0041-console-and-audio-controls.md).

For display and virtual I/O capabilities beyond console and audio, see
[display and virtual I/O capabilities](./display-io-capabilities.md).

---

## Providers in scope

| Provider | Identity | Guest-control channel |
| --- | --- | --- |
| Cloud Hypervisor NixOS | Local VM managed by `d2bd` + `d2b-priv-broker` | `guestd` over authenticated vsock guest-control |
| qemu-media | Dedicated media/console workload; no `guestd` | None |
| ACA sandbox | Provider-managed workload (Azure Container Apps) | guestd-compatible in-sandbox agent over ADR 0032 relay/peer transport |

---

## Console capability matrix

| Provider | Console surface | Console transport | Persistent drainer | Notes |
| --- | --- | --- | --- | --- |
| Cloud Hypervisor NixOS | ✓ | Broker-owned `--serial` backend; attach-safe and non-blocking. | Daemon-side ring-buffer drainer; broker or broker-spawned component owns the fd. | See [Console transport — Cloud Hypervisor](#console-transport--cloud-hypervisor). |
| qemu-media | ✓ | Broker-owned fd-backed chardev (PTY/fd-store design, not a qemu-created path socket). | Same daemon ring-buffer contract; broker-owned fd. | Qemu path sockets weaken permission posture; the fd-backed design is the posture baseline. See [Console transport — qemu-media](#console-transport--qemu-media). |
| ACA sandbox | ✓ via guestd | Provider guestd terminal/console over ADR 0032 relay/peer transport. | Not applicable (provider-managed draining). | Missing guestd is provider misconfiguration; see [ACA console — provider misconfiguration](#aca-console--provider-misconfiguration). |

### Console transport — Cloud Hypervisor

Cloud Hypervisor VMs may use a `--serial socket=...` backend only
when the implementation demonstrates the socket is non-blocking and
attach-safe. A persistent drainer continuously reads console output
into a bounded ring buffer so the guest is never blocked by the
absence of an attached operator. The ring-buffer cursor contract
lets clients fast-forward cleanly when output has been dropped:

- `ReadOutput` responses include the current cursor position and
  byte count so callers can detect gaps.
- Consumers must handle dropped-output notifications explicitly;
  there is no implicit replay of dropped bytes.

If the fd is held by a persistent broker component, that component
owns the drainer. The broker must remain the sole reader of the
console fd during a `d2bd` restart so draining is not interrupted.

### Console transport — qemu-media

qemu-media VMs do not run `guestd`. The daemon accesses the console
through a broker-owned fd-backed chardev. A qemu-created UNIX path
socket is not the default because it weakens socket-permission posture
and can race with stale socket cleanup on restart.

The broker holds the console fd across VM lifecycle transitions. On
daemon restart, the broker fd owner survives restart (it is not the
daemon main process) so console draining is not paused. The drainer
contract is identical to the Cloud Hypervisor case.

### ACA console — provider misconfiguration

ACA sandboxes are expected to run a guestd-compatible in-sandbox
agent. If the agent is absent, the daemon returns a typed
`provider-misconfiguration` error with a remediation that points
to the sandbox configuration. The daemon does **not** fall back to
`executeShellCommand` as a console substitute; that would violate the
no-raw-shell-channel constraint in ADR 0032 and ADR 0041.

---

## Audio capability matrix

| Provider | Host audio enforcement | Guest audio enforcement | Offline audio policy | Notes |
| --- | --- | --- | --- | --- |
| Cloud Hypervisor NixOS | ✓ PipeWire/vhost-user-sound controller | ✓ via `guestd` over authenticated guest-control | N/A (live state) | Host-side `off` is fail-closed; see [Audio enforcement — Cloud Hypervisor](#audio-enforcement--cloud-hypervisor). |
| qemu-media | ✓ host/qemu audio subset when declared | `unsupported` — `guestEnforcement = "unsupported"` reported | ✓ Persisted offline policy | See [Audio enforcement — qemu-media](#audio-enforcement--qemu-media). |
| ACA sandbox | None (no local host PipeWire nodes or broker mutations) | ✓ remote guestd policy only | None | No local audio state files or broker host mutations for ACA sandboxes; see [ACA audio](#aca-audio). |

### Audio enforcement — Cloud Hypervisor

Cloud Hypervisor NixOS VMs support both host-side PipeWire enforcement
and guest-side enforcement via `guestd`:

- Volume and gain are bounded `0..=100` domain values validated at the
  public-wire boundary before reaching the daemon.
- Local audio state is versioned and written atomically under an
  fd-lifetime lock at `/run/d2b/locks/`.
- Lock files are persistent coordination inodes and must never be
  unlinked during VM cleanup.
- Host-side `off` requests are fail-closed: the host boundary is sealed
  even when `guestd` is unresponsive; the response carries a degraded
  result for the guest-side enforcement step so the operator knows the
  guest-side did not apply.
- Multi-target `audio status` returns per-target errors and remediations
  so one misconfigured provider does not fail the entire status command.

### Audio enforcement — qemu-media

qemu-media VMs do not run `guestd`. The daemon:

- applies the declared host/qemu audio subset when it is advertised in
  the qemu-media capability declaration;
- persists offline audio policy to the qemu-media state directory;
- reports `guestEnforcement = "unsupported"` in `audio status` output.

This is a normal operating mode, not a degraded result. Operators
seeing `guestEnforcement = "unsupported"` on a qemu-media target
should not treat it as an error.

### ACA audio

ACA sandboxes use remote guestd audio policy only. The host does not
create local audio state files, PipeWire nodes, vhost-user-sound
connections, or broker host mutations for ACA targets. Missing guestd
on an ACA sandbox is provider misconfiguration and surfaces as a typed
error with remediation, not a silent no-op.

---

## Desktop control surface (d2b-wlcontrol)

`d2b-wlcontrol` is the official compositor-side client for the d2b
daemon's public socket. The constraints below are binding regardless of
provider:

- `d2b-wlcontrol` must not talk to the broker, use `sudo`, or read or
  write root-owned d2b state.
- Audio controls are grouped behind an explicit expanded surface or audio
  view. Collapsed VM cards show only a subtle badge for degraded,
  unsupported, host-only, or provider-misconfigured audio state.
- qemu-media host-side audio controls are enabled when the host subset
  is supported; the UI shows a `host-only` annotation alongside the
  controls.
- ACA missing-guestd states surface as provider-misconfiguration with
  remediation text, not as disabled UI controls.
- Volume and gain sliders send final or debounced mutations only; they
  do not dismiss layer-shell popups during drag.
- Keyboard increments are supported; slider values are preserved across
  mute toggles.

---

## Related references

- [ADR 0041](../adr/0041-console-and-audio-controls.md) — binding decision
  for the provider-capability-aware console and audio design.
- [Display and virtual I/O capabilities](./display-io-capabilities.md) —
  display, clipboard, USB, HID, GPU, and video sidecar capability boundaries.
- [Provider-managed sandboxes](./provider-managed-sandboxes.md) — Azure
  Container Apps adapter capability matrix.
- [Runtime provider selection](./runtime-provider-selection.md) — local
  runtime provider boundaries and capability gating.
- [Audio component reference](./components-audio.md) — Cloud Hypervisor
  audio component options, lifecycle, and hardening details.
- [qemu-media reference](./qemu-media.md) — qemu-media runtime details.
- [CLI contract — `console`](./cli-contract.md#console) — `d2b console`
  argument and exit-code contract.
- [CLI contract — `audio`](./cli-contract.md#audio-status) — `d2b audio`
  subcommands and exit-code contract.
- [Daemon API](./daemon-api.md) — planned `ConsoleOp`/`AudioOp` public
  wire types; see the [console and audio wire types note](./daemon-api.md#console-and-audio-wire-types).
