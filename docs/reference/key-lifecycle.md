# SSH key lifecycle

Reference for nixling-managed SSH identities, guest authorized-key staging,
and host-key trust state.

## Two different key surfaces

nixling manages two related but distinct things:

1. **The operator SSH identity** used by `nixling` for guest SSH.
2. **The guest staging share** under `/var/lib/nixling/vms/<vm>/host-keys/`,
   which contains only `host.pub` and `user-authorized-keys` for guest boot-time
   consumption.

The private key never appears in the public manifest and is never copied into
that guest staging share.

## Managed key path resolution

`nixling` resolves the SSH identity path like this:

- if `nixling.vms.<vm>.ssh.keyPath` is set, the CLI uses that path for
  SSH-driven lifecycle operations;
- otherwise it derives the path from `nixling.site.keysDir` as
  `<keysDir>/<vm>_ed25519`.

The framework-managed key generation in `host-keys.nix` still writes the
managed key under `nixling.site.keysDir` on every activation. In other words:

- `keysDir` is the storage location for the framework-owned keypair;
- `ssh.keyPath` is an operator override for which identity file the CLI should
  present over SSH.

That distinction is intentional: consumer-supplied keys are operator-owned and
must not be rotated or replaced by `nixling keys rotate`.

### Current read-only CLI note

The rust-native `keys list` / `keys show` surfaces intentionally return
`managedKeyPath: null` today. The public manifest does not expose raw
private-key paths, so the CLI cannot safely reconstruct the resolved path from
public data alone. Use host configuration as the source of truth.

## Ownership and permissions

Framework-managed keys follow these permissions:

| Path | Owner / mode |
| --- | --- |
| `${nixling.site.keysDir}` | `root:nixling-launcher` `0710` |
| `${nixling.site.keysDir}/<vm>_ed25519` | `root:nixling-launcher` `0640` |
| `${nixling.site.keysDir}/<vm>_ed25519.pub` | `root:root` `0644` |
| `${nixling.site.keysDir}/.lock` | `root:root` `0600` |

The CLI copies the private key to a caller-owned `0600` tempfile before passing
it to `ssh`, because OpenSSH refuses group-readable identity files directly.

## Rotation flow

`nixling keys rotate <vm>` is the managed-key rollover for the framework-owned
`${keysDir}/<vm>_ed25519` keypair.

1. Resolve the managed key path from `nixling.site.keysDir`.
2. Verify the old key still reaches the guest over SSH.
3. Record the old fingerprint.
4. Move the old private/public pair under `old/<timestamp>/` beside the managed
   keys directory.
5. Generate a fresh Ed25519 pair as `.new` staging files.
6. Push the new public key into the guest's `authorized_keys` using the old key.
7. Verify the new key works.
8. Remove the old key from the guest.
9. Retain only the three newest archived rotations under `old/`.
10. Re-run `nixos-rebuild switch` so future guest boots refresh the staged
    `host.pub` share with the new public key.

If you use a per-VM `ssh.keyPath` override, rotate that external key with its
own owner/tooling. `nixling keys rotate` is for the framework-managed key only.

## Trust operations

nixling tracks guest SSH host keys separately from operator private keys.

### `nixling trust <vm>`

- requires a VM `staticIp`;
- scans the guest with `ssh-keyscan -t ed25519`;
- rewrites `/var/lib/nixling/known_hosts.nixling` under a lock;
- replaces any existing entry for that VM/IP with the newly scanned line.

Use this on first boot or after an intentional host-key reset.

### `nixling rotate-known-host <vm>`

- removes the recorded entry for the VM from `known_hosts.nixling`;
- does **not** generate a new host key by itself;
- is the right pre-step when a guest will come back with a different SSH host
  key and you want the next `trust` to be explicit.

## Audit logging

When a key-management verb goes through the daemon -> broker path, the broker
emits a daily JSONL audit record under
`/var/lib/nixling/audit/broker-<utc-date>.jsonl` for:

- `RunKeysRotate`
- `RunHostKeyTrust`
- `RunRotateKnownHost`

Use `nixling audit` / `nixling audit --json` to inspect those records. If the
CLI had to fall back to the legacy bash path, rely on shell history, sudo/journal
logs, and your config history instead — only broker-handled requests land in the
broker audit log.

## Upgrading from bash nixling

Managed keys, `known_hosts`, and trust-state all carry forward into
the Rust/daemon path. The control-plane owner changes; the files do
not.

### What stays in place

- `${nixling.site.keysDir}/<vm>_ed25519` and `.pub`
- `/var/lib/nixling/known_hosts.nixling`
- any existing broker audit history under `/var/lib/nixling/audit/`

### Transition steps

1. Rebuild the host so `nixling keys *`, `trust`, and
   `rotate-known-host` land from the Rust CLI.
2. Start with read-only checks: `nixling keys list` and
   `nixling keys show <vm>`.
3. Use `--dry-run` first on `keys rotate`, `trust`, or
   `rotate-known-host`; add `NIXLING_NATIVE_ONLY=1` only when you
   want to validate the daemon path without bash fallback.
4. Existing guest host keys and authorized-keys entries remain valid
   until you intentionally rotate them.

### Rollback

- The `NIXLING_LEGACY_BASH_OPT_IN=1` escape hatch was retired in
  P6 (per ADR 0015). Roll back a half-completed rotation by
  rebuilding to the prior host generation and restoring from your
  backup of `keysDir` / `known_hosts` before rerunning the verb
  through `nixlingd` → broker `RunKeysRotate`.
- If a rotation half-completes, restore from your backup of
  `keysDir` / `known_hosts` before rerunning.
- Never wipe `/var/lib/nixling/vms/<vm>/swtpm/` as a shortcut; TPM
  identity loss is separate from SSH trust rotation and forces
  external re-enrollment.

## See also

- [`components-tpm.md`](./components-tpm.md)
- [`daemon-api.md`](./daemon-api.md)
- [`privileges.md`](./privileges.md)
- [`../how-to/uninstall-nixling.md`](../how-to/uninstall-nixling.md)
