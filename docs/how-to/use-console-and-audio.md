# How to use console and audio controls

**Diataxis category:** how-to.

> **Status:** The `d2b console` and `d2b audio` CLI verbs are
> daemon-native surfaces. This guide documents the operator workflow.

---

## Before you begin

- You must be a member of the `d2b` group on the host.
- For `d2b audio` commands, the target VM must have
  `d2b.vms.<vm>.audio.enable = true` in its NixOS configuration.
  Confirm with `d2b vm status <vm>` — the audio field appears in the
  capability summary for audio-enabled VMs.
- For `d2b console` on an ACA sandbox, the sandbox must be running a
  guestd-compatible in-sandbox agent. If it is absent, the daemon
  returns a typed `provider-misconfigured` error with remediation
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

2. Press `Ctrl-]` to detach from the console session and return to your
   shell. The VM continues running.

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
   enforcement:        host-and-guest
   ```

   The `enforcement` field reflects which side of the policy was
   applied:

   - `host-and-guest` — host and guest enforcement are active (Cloud
     Hypervisor NixOS VMs with guestd).
   - `host-only` — only host-side policy is available or applied.
   - `guest-only` — only guest/provider policy is available or applied.
   - `unsupported` — the provider does not support guest enforcement.
     This is normal and expected for qemu-media VMs, not an error.
   Provider failures such as `provider-misconfigured` are not successful
   enforcement postures; they appear in the separate per-target error
   entry with remediation text.

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

Follow the same status check used for `audio mic`.

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
| `provider-misconfigured` | ACA sandbox missing guestd agent. | Deploy the guestd-compatible in-sandbox agent. |
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
