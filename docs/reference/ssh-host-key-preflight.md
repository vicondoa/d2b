# Per-VM sshd host key preflight

Reference for the daemon-side VM-start preflight that refuses to start
a VM when its per-VM sshd host keys directory or any key leaf has
drifted from the canonical posture.

Related preflight: per-VM state-directory ownership matrix — see
[`per-vm-state-ownership.md`](./per-vm-state-ownership.md).

## What it checks

For every VM the daemon starts (`nixling vm start <vm>`) the daemon
runs the preflight against
`/var/lib/nixling/vms/<vm>/sshd-host-keys/` before dispatching any
broker op:

1. **Directory shape.** The path must exist as a real directory, not
   a symlink. Real directory ownership and mode are enforced
   separately by the
   [ownership matrix](./per-vm-state-ownership.md) preflight (which
   declares `nixlingd:nixling 0750` for this directory).
2. **Key file shape.** For each `ssh_host_*_key` private-key file
   directly under the directory (excluding `.pub` siblings and any
   unrelated files):
   - the file is a regular file (no symlinks — equivalent to opening
     with `O_NOFOLLOW`);
   - owner uid is `0` (root);
   - group gid is `0` (root);
   - mode is exactly `0o0400`.

The first drift finding causes the daemon to refuse VM start with
the typed `daemon.sshd-host-key-drift` envelope (exit code `62`).

## Why these specific posture rules

Per the threat model in [`design.md`](../explanation/design.md), the
per-VM sshd host key is the root of the host's trust in the guest's
SSH endpoint. Two failure modes drove this preflight:

- **Symlinked key file.** If a `ssh_host_*_key` becomes a symlink,
  the daemon would silently hand the runner a key whose contents and
  permissions live elsewhere on the filesystem. Refusing symlinks
  (the daemon-side equivalent of `O_NOFOLLOW`) closes a
  symlink-substitution TOCTOU between activation and runner exec.
- **ACL / mode propagation.** Per
  [`per-vm-state-ownership.md`](./per-vm-state-ownership.md) the
  per-VM hardlink farm under `…/store/` shares inodes with
  `/nix/store`. A recursive `setfacl` against the per-VM tree once
  propagated a default ACL onto ssh host key paths inside
  `/nix/store`; OpenSSH's `safe_path()` then refused to load them.
  This preflight catches the drift even when the offending operation
  ran outside nixling.

## Operator-facing failure envelope

```json
{
  "kind": "sshd-host-key-drift",
  "exitCode": 62,
  "message": "vm 'work' refused: sshd host key drift: ssh host key mode 644 != expected 400: /var/lib/nixling/vms/work/sshd-host-keys/ssh_host_ed25519_key",
  "remediation": "regenerate or chown/chmod the per-VM sshd host keys so each ssh_host_*_key under /var/lib/nixling/vms/<vm>/sshd-host-keys is a regular file owned root:root with mode 0400 (no symlinks); see docs/reference/ssh-host-key-preflight.md. Recovery: nixos-rebuild switch (re-runs the host-activation key sync), or remove the offending key and let nixling keys rotate <vm> reprovision it."
}
```

The drift reason text is one of:

| Drift class            | Example message fragment                                   |
| ---------------------- | ---------------------------------------------------------- |
| `DirMissing`           | `sshd-host-keys directory missing: <path>`                 |
| `DirIsSymlink`         | `sshd-host-keys directory is a symlink: <path>`            |
| `DirNotADirectory`     | `sshd-host-keys path is not a directory: <path>`           |
| `KeyIsSymlink`         | `ssh host key is a symlink: <path>`                        |
| `KeyNotARegularFile`   | `ssh host key is not a regular file: <path>`               |
| `KeyWrongOwner`        | `ssh host key owner uid <a> != expected <e>: <path>`       |
| `KeyWrongGroup`        | `ssh host key group gid <a> != expected <e>: <path>`       |
| `KeyWrongMode`         | `ssh host key mode <a> != expected <e>: <path>`            |
| `KeyStatFailed`        | `ssh host key stat failed: <path> (<errno>)`               |
| `DirReadFailed`        | `sshd-host-keys read_dir failed: <path> (<errno>)`         |
| `DirStatFailed`        | `sshd-host-keys stat failed: <path> (<errno>)`             |

The preflight stops at the **first** finding so the surfaced message
is short and actionable. Iteration is sorted by file name so the
same drift reproducibly surfaces the same offending entry across runs.

## Migration-window posture

The per-VM `sshd-host-keys` directory is materialized by the
nixling host-activation chain on first
`nixos-rebuild switch`. Until that has run, the directory may be
absent on a freshly provisioned host. To avoid a chicken-and-egg
on first boot the daemon **tolerates a missing keys directory** (a
warn log records the skip). Once the directory exists, any drift in
its contents is fail-closed.

The companion ownership-matrix preflight retains the same posture
for the directory itself: missing directory → warn-only, drift on
an existing path → fail-closed.

## Implementation

- Pure check:
  [`nixlingd::ssh_host_key_preflight::check_sshd_host_keys`](../../packages/nixlingd/src/ssh_host_key_preflight.rs)
  — takes `(vm, keys_dir)`, returns `Result<(), SshdHostKeyDrift>`.
- Typed error:
  [`TypedError::SshdHostKeyDrift`](../../packages/nixlingd/src/typed_error.rs)
  — exit code `62`, kind `sshd-host-key-drift`.
- Call sites:
  1. `dispatch_broker_vm_start` runs the preflight inline after
     `ownership_preflight::preflight`. Refusal short-circuits all
     broker dispatch for the VM.
  2. `execute_host_prep_dag` calls the same handler when its
     `HostPrepStepKind::SshHostKeyPreflight` step is reached (when
     the host-prep DAG executor is gated on via
     `NIXLING_HOST_PREP_DAG_EXECUTE=1`). The broker stub variant
     (`BrokerRequest::SshHostKeyPreflight`) is left in the wire enum
     as a typed placeholder; the live handler lives daemon-side
     because the check is a pure filesystem stat that the daemon
     already has `CAP_DAC_READ_SEARCH` for.

## Tests

- Unit:
  [`nixlingd::ssh_host_key_preflight::tests`](../../packages/nixlingd/src/ssh_host_key_preflight.rs)
  exhaustively covers each drift class against a tempdir-built
  fixture.
- Integration: [`tests/ssh-host-key-preflight-eval.sh`](../../tests/ssh-host-key-preflight-eval.sh)
  drives those unit tests under the static gate alongside the
  `TypedError::SshdHostKeyDrift` envelope-shape assertion.

## Spec correction

Earlier drafts referenced `/var/lib/nixling/keys/<vm>/sshd-host-keys`
and a `root:nixling-<vm>-runner 0750` directory posture with `0640`
key files. The canonical paths and modes shipped by the ownership
matrix (`/var/lib/nixling/vms/<vm>/sshd-host-keys`,
`nixlingd:nixling 0750` directory, `root:root 0400` key
files) take precedence per AGENTS.md "Existing code is canon".
