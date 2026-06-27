# How to use console and audio controls

**Diataxis category:** how-to.

> **Status:** The `d2b console` and `d2b audio` CLI verbs are
> currently `rust-native shim` surfaces. They parse and validate
> arguments natively, but return typed `not-yet-implemented` exit-78
> envelopes until the daemon-native backends ship. This guide documents
> the intended usage and the provider-specific behavior so operators can
> plan their configurations now.
>
> For the current deferred-verb remediation details, see
> [error-codes.md § Remediation rendering conventions](../reference/error-codes.md#remediation-rendering-conventions)
> and the [migration guide v0 → v1](migrate-d2b-v0-to-v1.md).

---

## Background

Console streaming and audio enforcement are
provider-capability-aware daemon surfaces. The daemon resolves the
target's runtime provider before touching local host state; unsupported
capabilities return typed refusals rather than silent fallbacks.

The three supported providers and their console/audio capabilities are
documented in the [provider capability matrix](../reference/provider-capability-matrix.md).
The architectural decision is [ADR 0041](../adr/0041-console-and-audio-controls.md).

---

## Connecting to a VM console

```text
d2b console <vm>
```

When the backend ships, this attaches a terminal to the serial console
of `<vm>`. Use `~.` to detach.

**Provider notes:**

- *Cloud Hypervisor NixOS VMs* — broker-owned serial backend; a
  persistent drainer maintains a ring buffer so the guest is never
  blocked when no operator is attached.
- *qemu-media VMs* — broker-owned fd-backed chardev (not a path
  socket); ring-buffer drainer contract is identical to Cloud Hypervisor.
- *ACA sandboxes* — console attaches over the guestd-compatible
  provider transport. Missing guestd is a provider-misconfiguration
  error, not a degraded-mode connection.

**Exit codes** (when the backend ships):

| Code | Meaning |
| --- | --- |
| `0` | Session ended normally. |
| `130` | Session interrupted with SIGINT. |
| `1` | Console launch failure. |
| `2` | Unknown VM, unsupported invocation, or graphics VM selected (graphics VMs use `d2b vm start <vm> --apply` instead). |

---

## Checking audio state

```text
d2b audio status [<vm>]
```

When the backend ships, this prints the audio state for one VM (or all
audio-enabled VMs when no argument is given). Multi-target output
returns per-target results; one misconfigured provider does not fail the
entire command.

**Example (Cloud Hypervisor NixOS VM):**

```text
$ d2b audio status corp-vm
audio:              enabled
mic:                off
speaker:            off
guestEnforcement:   applied
```

**Example (qemu-media VM):**

```text
$ d2b audio status media-vm
audio:              enabled
mic:                off
speaker:            off
guestEnforcement:   unsupported
```

The `guestEnforcement: unsupported` result on a qemu-media target is
normal, not an error; qemu-media VMs do not run guestd.

---

## Granting or revoking microphone access

```text
d2b audio mic on|off <vm>
```

Sets the microphone grant for `<vm>`. The VM must have
`d2b.vms.<vm>.audio.enable = true` in its NixOS configuration.

Volume and gain values are bounded to `0..=100` at the wire boundary.
Lock files at `/run/d2b/locks/` are persistent coordination inodes and
are never unlinked during VM cleanup.

**Provider behavior:**

- *Cloud Hypervisor NixOS* — host-side PipeWire enforcement plus
  guestd guest enforcement. Host-side `off` is fail-closed: the host
  boundary is sealed even if guestd is unresponsive; the response
  carries a degraded result for the guest side.
- *qemu-media* — host/qemu subset only; guest enforcement unsupported.
- *ACA sandbox* — remote guestd policy only; no local host mutations.

---

## Granting or revoking speaker access

```text
d2b audio speaker on|off <vm>
```

Sets the speaker grant for `<vm>`. Same provider behavior and lock
contract as `audio mic`.

---

## Turning off all audio for a VM

```text
d2b audio off <vm>
```

Shorthand for setting both mic and speaker to `off` in a single
operation.

---

## Desktop control surface (d2b-wlcontrol)

Audio controls in `d2b-wlcontrol` are grouped behind an explicit
expanded surface or audio view. Collapsed VM cards show only a subtle
badge for degraded, unsupported, host-only, or provider-misconfigured
audio state:

- `host-only` — qemu-media host subset supported; guest enforcement
  unavailable.
- `provider-misconfiguration` — ACA sandbox missing guestd; remedy by
  deploying the guestd-compatible in-sandbox agent.
- `degraded` — Cloud Hypervisor guest-side enforcement did not apply
  while host side succeeded.
- `unsupported` — provider does not advertise any audio capability.

---

## Related references

- [Provider capability matrix](../reference/provider-capability-matrix.md) —
  full per-provider console and audio capability boundaries.
- [ADR 0041](../adr/0041-console-and-audio-controls.md) — binding design
  decision.
- [Audio component reference](../reference/components-audio.md) — NixOS
  options, lifecycle, and hardening details for Cloud Hypervisor audio.
- [CLI contract — `console`](../reference/cli-contract.md#console) — full
  argument and exit-code contract.
- [CLI contract — `audio`](../reference/cli-contract.md#audio-status) — full
  audio subcommand contracts.
- [Error codes](../reference/error-codes.md) — typed error catalog.
