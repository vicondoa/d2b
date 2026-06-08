# Security runbook

**Diataxis category:** reference.

Day-2 security guidance for hosts running nixling's daemon / broker control
plane.

## Secret storage and access policy

nixling does **not** yet ship a general-purpose secret backend. The reserved
broker operations (`ReadSecretById`, `InjectSecretById`, `RotateSecretById`)
exist in the privileges matrix, but they are not the operator-facing secret
store for today's releases.

Treat these paths as the sensitive baseline instead:

| Surface | Why it matters | Access policy |
| --- | --- | --- |
| `nixling.site.keysDir` (default `/var/lib/nixling/keys`) | Framework-managed SSH private keys. | Keep root-owned; launcher-group readability exists only so the CLI can copy the key to a caller-owned `0600` tempfile before `ssh`. |
| `/var/lib/nixling/vms/<vm>/host-keys/` | Guest boot-time staging (`host.pub`, `user-authorized-keys`). | Public keys + authorized-keys text only; never put private keys here. |
| `/var/lib/nixling/vms/<vm>/swtpm/` | TPM NVRAM + EK seed. | Treat as identity-bearing state. Do not wipe casually; wiping forces guest re-enrollment with any external IdP. |
| `/etc/nixling/*.json` and `/var/lib/nixling/current-bundle/` | Trusted bundle + daemon config. Usually integrity-sensitive rather than secret. | Root-owned, not world-readable, changed only by nixling's own emitters/install flow. |
| `/var/lib/nixling/audit/broker-<utc-date>.jsonl` | Security evidence for privileged operations. | Root-owned append-only writer; read through `nixling audit` or direct root access. |

Operational rules:

- keep real secrets out of `vms.json`, examples, README snippets, and public
  issue reports;
- keep `launcherUsers` small — that boundary is broad enough to matter;
- prefer framework-managed keys unless you have a documented reason to own an
  external `ssh.keyPath` yourself.

## Broker authz matrix overview

The closed-world privileges matrix lives in `privileges.json`, but the operator
view is usually this simpler table:

| Actor | Socket / entry point | What they can do |
| --- | --- | --- |
| `launcherUsers` | `/run/nixling/public.sock` via the CLI | Day-to-day public CLI traffic: read-only verbs plus mutating verbs that dispatch through nixlingd → broker (v1.0 daemon-only per ADR 0015). Treat membership as privileged host access. |
| `adminUsers` | `/run/nixling/public.sock` via the CLI | Everything launchers can do, plus the admin-only `audit` export surface. |
| `nixlingd` | `/run/nixling/priv.sock` | The **only** direct broker client. The broker re-resolves paths from the trusted bundle and emits allow/deny audit rows for privileged operations. |
| `root` | host OS | Break-glass access outside the nixling control plane. |

Two consequences matter operationally:

1. the real privilege split is **public socket caller -> nixlingd -> broker**,
   not "human user talks to broker directly";
2. destructive broker ops are audited and tagged as destructive in the private
   matrix, but the current public-socket human boundary is still the configured
   launcher/admin user set. Restrict those groups accordingly.

## Audit log inspection

Start with the supported surface:

```bash
nixling audit --json
```

For direct on-host inspection, read the daily JSONL files:

```bash
sudo tail -n 50 /var/lib/nixling/audit/broker-$(date -u +%F).jsonl
```

Fields worth pivoting on during an incident:

- `operation`
- `public_operation_id`
- `authz_result`
- `decision`
- `error_kind`
- `subject_id`
- `scope_id`
- `bundle_hash`
- `tracing_span_id`

Use `journalctl` for surrounding service context:

```bash
sudo journalctl -u nixlingd.service --since "-1h"
```

The broker defaults to 14-day retention. The `nixling.site.audit.retentionDays`
option is already declared, but on today's NixOS path its value may still lag
behind the runtime broker invocation wiring. If you need longer retention now,
ship the JSONL files off-host before prune-on-rotate does it for you.

## USBIP emergency response

Use this when a YubiKey or other allowlisted USBIP device is
attached to the wrong VM/env, appears stuck after a crash, or needs
immediate containment.

### 1. Detach or unbind immediately

Prefer the public CLI first:

```bash
sudo nixling usb detach work-entra 1-3 --apply
```

If the guest still holds the device and you need the low-level
tools, use `usbip port` / `usbip detach --port <N>` on the guest
side and `sudo usbip unbind -b 1-3` on the host.

### 2. Stop the per-env USBIP runners

In v1.0 (per ADR 0015) the per-env usbipd backend + proxy run as
broker-spawned runners on the per-env DAG under
`nixling.slice/sys-<env>/usbipd-*`. Stop them via the broker
`SignalRunner` op dispatched through the daemon — e.g. detach
the device through `nixling usb detach <vm> --apply` which the
broker translates into a SIGTERM on the per-env usbipd runner.
(The pre-P6 `nixling-sys-<env>-usbipd-{backend,proxy}.{service,socket}`
systemd units were retired in P6; the equivalent operator action
no longer goes through `systemctl stop`.)

### 3. Clear stale locks only after detach is confirmed

```bash
sudo rm -f /run/nixling/locks/usbip/1-3
```

Do this only after the device is detached/unbound and the per-env
units above are stopped; otherwise the next attach may race the old
owner.

### 4. Validate the recovered state

```bash
nixling usb probe --json
nixling audit --json
sudo journalctl -u nixlingd.service --since "-15m"
```

Confirm that the busid no longer shows an active owner, the lock
file is gone, and the audit trail contains the detach/unbind you
expected before re-enabling the proxy/backend units.

## Compromise recovery

### 1. Freeze the control plane

Pick the narrowest effective move first:

- remove the affected user from `launcherUsers` / `adminUsers` and rebuild;
- or stop `nixlingd` while you investigate;
- or, on NixOS, temporarily revert daemon-managed VMs back to
  `supervisor = "systemd"` and/or set `daemonExperimental.enable = false`.

### 2. Preserve evidence before cleanup

Copy or snapshot at least:

- `/var/lib/nixling/audit/`
- the trusted bundle (`/etc/nixling/*.json` or `/var/lib/nixling/current-bundle/`)
- `journalctl -u nixlingd.service`
- the relevant VM state directories under `/var/lib/nixling/vms/<vm>/`

Do **not** delete `swtpm/` unless you have already decided to rotate the guest's
TPM identity and re-enroll it everywhere.

### 3. Rotate the affected trust material

- **Framework-managed SSH identity compromised:** run `nixling keys rotate <vm>`.
- **Consumer-supplied `ssh.keyPath` compromised:** rotate that key out-of-band;
  nixling deliberately will not overwrite it.
- **Guest SSH host key changed or suspected compromised:** run
  `nixling rotate-known-host <vm>` and then `nixling trust <vm>` once the guest
  is back with its replacement host key.
- **Bundle/config tampering:** re-render and re-land the trusted bundle, then
  restart the daemon.

### 4. Rebuild from known-good state

- reinstall host artifacts if needed with `nixling host install --apply`;
- restart or switch affected VMs from a known-good generation;
- if you are fully unwinding the rollout, follow
  [`../how-to/uninstall-nixling.md`](../how-to/uninstall-nixling.md).

### 5. Validate the recovered host

Use at least:

```bash
nixling auth status --json
nixling host check --strict
nixling audit --json
```

Then confirm the recovered VM trust state (`keys show`, `trust`, `status`) for
any VM touched by the incident.

## See also

- [`./key-lifecycle.md`](./key-lifecycle.md)
- [`./privileges.md`](./privileges.md)
- [`./error-codes.md`](./error-codes.md)
- [`../how-to/uninstall-nixling.md`](../how-to/uninstall-nixling.md)
