# How to use console and audio controls

> **Status:** The `d2b console` and `d2b audio` CLI verbs are
> currently staged surfaces that parse and validate arguments natively
> but return typed `not-yet-implemented` exit-78 envelopes until the
> daemon-native backends ship. This guide documents the intended
> operator workflow so you can plan your configuration now.
>
> For current deferred-verb remediation details, see
> [error-codes.md § Remediation rendering conventions](../reference/error-codes.md#remediation-rendering-conventions)
> and the [migration guide v0 → v1](migrate-d2b-v0-to-v1.md).

---

## Before you begin

- You must be a member of the `d2b` group on the host.
- For `d2b audio` commands, the target VM must have
  `d2b.vms.<vm>.audio.enable = true` in its NixOS configuration.
  Confirm with `d2b vm status <vm>` — the audio field appears in the
  capability summary once the backend ships.
- For `d2b console` on an ACA sandbox, the sandbox must be running a
  guestd-compatible in-sandbox agent. If it is absent, the daemon
  returns a typed `provider-misconfiguration` error with remediation
  text; there is no console fallback.
- Graphics VMs do not have a serial console surface. Use
  `d2b vm start <vm> --apply` to launch graphics VMs.

---

## Connect to a VM's serial console

1. Run:

   ```text
   d2b console <vm>
   ```

   The daemon resolves the target's runtime provider, then attaches
   your terminal to the serial console of `<vm>`.

2. Type `~.` (tilde followed by a period) to detach from the console
   session and return to your shell. The VM continues running.

3. If the command exits with code `2`, verify that `<vm>` is a
   declared VM name and that it is not a graphics VM. Run
   `d2b vm list` to confirm the VM name and provider.

**Expected exit codes:**

| Code | Meaning |
| --- | --- |
| `0` | Session ended normally. |
| `130` | Session interrupted with SIGINT. |
| `1` | Console launch failure (see error envelope for details). |
| `2` | Unknown VM, unsupported invocation, or graphics VM selected. |

**Provider behavior:**

- *Cloud Hypervisor NixOS VMs* — the daemon connects through the
  broker-owned serial backend. A persistent drainer maintains a bounded
  ring buffer so the guest is never blocked when no operator is
  attached. You may see buffered output from before you attached.
- *qemu-media VMs* — the daemon uses a broker-owned fd-backed chardev.
  The ring-buffer drainer contract is identical to Cloud Hypervisor.
- *ACA sandboxes* — the console attaches over the guestd-compatible
  provider transport. Missing guestd is a provider-misconfiguration
  error, not a degraded-mode connection.

---

## Check audio state for a VM

1. To check one VM:

   ```text
   d2b audio status <vm>
   ```

   To check all audio-enabled VMs at once:

   ```text
   d2b audio status
   ```

2. Read the output fields:

   ```text
   audio:              enabled
   mic:                off
   speaker:            off
   guestEnforcement:   applied
   ```

   The `guestEnforcement` field reflects whether the guest-side
   enforcement step succeeded:

   - `applied` — guest-side enforcement is active (Cloud Hypervisor
     NixOS VMs via guestd).
   - `unsupported` — the provider does not support guest enforcement.
     This is normal and expected for qemu-media VMs, not an error.
   - `degraded` — host-side enforcement succeeded but guestd was
     unresponsive; the guest boundary may not be sealed.
   - `provider-misconfiguration` — ACA sandbox missing its guestd
     agent; use the remediation text in the error envelope.

3. If one provider in a multi-VM status run fails, its entry carries an
   inline error and remediation. The remaining entries are unaffected.

---

## Grant or revoke microphone access

1. To grant microphone access:

   ```text
   d2b audio mic on <vm>
   ```

2. To revoke it:

   ```text
   d2b audio mic off <vm>
   ```

3. Confirm with `d2b audio status <vm>` and verify `mic: on` or
   `mic: off` as expected.

**Provider behavior:**

- *Cloud Hypervisor NixOS* — host-side PipeWire enforcement plus
  guestd guest enforcement. `off` is fail-closed: the host boundary is
  sealed even if guestd is unresponsive; the response carries a
  degraded result for the guest side.
- *qemu-media* — host/qemu subset only; `guestEnforcement: unsupported`
  is always reported.
- *ACA sandbox* — remote guestd policy only; no local host mutations.

Volume and microphone gain values are bounded to `0..=100` at the wire
boundary; values outside this range are rejected before reaching the
daemon.

---

## Grant or revoke speaker access

1. To grant speaker access:

   ```text
   d2b audio speaker on <vm>
   ```

2. To revoke it:

   ```text
   d2b audio speaker off <vm>
   ```

3. Confirm with `d2b audio status <vm>`.

Provider behavior and fail-closed semantics are the same as for
`audio mic`.

---

## Silence all audio for a VM

To set both microphone and speaker to `off` in a single operation:

```text
d2b audio off <vm>
```

Confirm with `d2b audio status <vm>` that both `mic` and `speaker`
show `off`.

---

## Manage audio from the desktop (d2b-wlcontrol)

Audio controls in `d2b-wlcontrol` are available in the expanded VM
card view or the dedicated audio panel. To reach them:

1. Click or activate a VM card to expand it, or open the audio view.
2. Use the microphone and speaker toggles or the volume/gain sliders.
   Sliders send mutations only on release or keyboard-increment
   confirmation; they do not dismiss the layer-shell popup during drag,
   and slider values are preserved across mute toggles.

Collapsed cards show a subtle badge for states that need attention:

| Badge | Meaning | Action |
| --- | --- | --- |
| `host-only` | qemu-media host subset only; guest enforcement unavailable. | Expected; no action required. |
| `provider-misconfiguration` | ACA sandbox missing guestd agent. | Deploy the guestd-compatible in-sandbox agent. |
| `degraded` | Cloud Hypervisor guest-side did not apply while host succeeded. | Check guestd status inside the VM. |
| `unsupported` | Provider advertises no audio capability. | Verify the VM's audio configuration. |

`d2b-wlcontrol` communicates only through the daemon's public socket.
It does not invoke `sudo`, talk to the broker directly, or read
root-owned d2b state.

---

## Related references

- [Provider capability matrix](../reference/provider-capability-matrix.md) —
  full per-provider console and audio capability boundaries, including
  stream isolation and lock semantics.
- [ADR 0041](../adr/0041-console-and-audio-controls.md) — binding design
  decision.
- [Audio component reference](../reference/components-audio.md) — NixOS
  options, lifecycle, and hardening details for Cloud Hypervisor audio.
- [CLI contract — `console`](../reference/cli-contract.md#console) — full
  argument and exit-code contract.
- [CLI contract — `audio`](../reference/cli-contract.md#audio-status) — full
  audio subcommand contracts.
- [Error codes](../reference/error-codes.md) — typed error catalog.
