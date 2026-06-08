# Store lifecycle

Reference for nixling's per-VM `/nix/store` lifecycle.

This contract is implemented in two layers today:

- the shipping NixOS/module path in [`nixos-modules/store.nix`](../../nixos-modules/store.nix)
- the Rust primitive layer in [`packages/nixling-host/src/hardlink_farm.rs`](../../packages/nixling-host/src/hardlink_farm.rs)

The file names differ slightly between those layers, but the operator-visible
invariants are the same.

## On-disk layout

Per VM, nixling keeps two closely related trees under
`/var/lib/nixling/vms/<vm>/`:

| Path | Purpose |
| --- | --- |
| `store/` | The hardlink farm that virtiofsd exposes to the guest. Every entry is a hardlink to a host `/nix/store` path, never a byte-for-byte copy. |
| `store-meta/current -> generations/<N>` | The active generation pointer. Updated atomically on activation. |
| `store-meta/generations/<N>/system` | Symlink to the host's system toplevel for that generation. |
| `store-meta/generations/<N>/store-paths` | Newline-delimited closure list for retention + GC. |
| `store-meta/generations/<N>/db.dump` | `nix-store --dump-db` payload for the retained closure. |
| `store-meta/generations/<N>/meta.json` | Generation metadata (`generation`, `timestamp`, `runner`). |
| `store-meta/gcroots/generation-<N>` | Host-side GC root pinning the retained generation. |

The Rust primitive layer models the same lifecycle with a per-generation
`marker.json` and a staged `current.tmp` symlink before the final atomic rename.
That is the crash-safety contract the daemon-era activation path follows.

## Hardlink-farm invariants

nixling relies on these invariants for every per-VM store:

1. **Only the VM's declared closure is exposed.** The guest does not see the
   host's whole `/nix/store`.
2. **Bytes are shared, not copied.** The farm is built with hardlinks into the
   host store; extra disk cost is directory entries and metadata only.
3. **virtiofsd serves a bind-mounted view of the farm, not the real host
   store.** The helper binds the farm onto `/nix/store` inside the service's
   namespace before virtiofsd starts.
4. **The tree must be framework-owned.** Marker files are checked before the
   share is exported.

The legacy helper runs in a private mount namespace because NixOS bind-mounts
`/nix/store` on top of itself; without the private namespace, `link(2)` would
fail with `EXDEV` even on the same block device.

## Same-filesystem fatal checks

Hardlinks cannot cross filesystems. nixling therefore fails closed before it
tries to materialize a farm:

- the shell helper compares the filesystem ID of `/nix/store` and the per-VM
  state root and aborts if they differ;
- the Rust layer's `assert_same_filesystem` checks `st_dev` for the same reason.

If `/var/lib/nixling` and `/nix/store` are on different filesystems,
`switch`/`boot`/`test`/`rollback` refuse rather than silently copying data or
building an unusable tree.

## Marker checks

nixling uses two marker styles:

- **Farm marker**: `store/.nixling-marker-<vm>` is planted by the sync helper.
  The virtiofsd preflight checks both the real tree and the bind-mounted view
  visible to the service. A hand-made directory without the marker is refused.
- **Generation marker**: the Rust primitive layer writes `generations/<N>/marker.json`
  and refuses to activate a generation if the marker is missing, malformed, or
  mismatched.

Together, those checks stop both stale bind-mount views and partially created
activations from being mistaken for a valid store generation.

## Crash-safe symlink updates

The active generation pointer is always updated with a staged symlink and a
single rename:

- the shell path writes `store-meta/current.new` and renames it over `current`;
- the Rust path writes `current.tmp`, renames it over `current`, and removes any
  stale `current.tmp` on the next activation via `reconcile_stale_swap_tmp`.

That keeps the public pointer either on the old generation or on the new one,
never on a half-written target.

## Destructive and downgrade operations

The store lifecycle has two destructive surfaces:

- `rollback` activates an older retained generation;
- `gc` deletes unkept generation metadata, GC roots, and farm entries that are
  no longer referenced.

On the private wire, the broker marks `RunActivation` and `RunGc` as
`destructive: true` and audits every decision. Only `nixlingd` may call those
broker operations directly. On the public socket, the current outer boundary is
still the configured launcher/admin user set, so treat `rollback` and `gc` as
admin-owned operational procedures whenever you need a narrower human trust
boundary than "can launch VMs".

## Generation retention policy

The legacy sync helper's retention rule is explicit:

- always keep the **next** generation being written;
- if the VM is running, also keep the generation backing the running VM;
- if the VM is down, keep the most recent prior generation as the rollback
  target.

Everything else is pruned:

- unkept `store-meta/generations/<N>/` directories;
- their matching `store-meta/gcroots/generation-<N>` links;
- host per-user GC roots for those generations;
- hardlink-farm entries not referenced by the union of retained `store-paths`
  files.

This is **per-VM** retention. It does not prune host NixOS system generations;
operators still need host-level garbage collection such as:

```bash
sudo nix-collect-garbage --delete-older-than 7d
```

## Upgrading from bash nixling

The Rust/daemon migration deliberately reuses the bash-era store
state on disk. There is no one-shot data migration as long as the
existing per-VM store tree is healthy.

### Bash-era state that stays in place

- `/var/lib/nixling/vms/<vm>/current`
- `/var/lib/nixling/vms/<vm>/booted`
- `/var/lib/nixling/vms/<vm>/store/`
- `/var/lib/nixling/vms/<vm>/store-meta/generations/`
- `/var/lib/nixling/vms/<vm>/store-meta/gcroots/`

### Safety checks before the first native `--apply`

- confirm `/var/lib/nixling` and `/nix/store` are on the same
  filesystem;
- confirm `current` / `booted` still resolve and the latest retained
  generation still has its marker file;
- run `nixling generations <vm>` and a dry run (`nixling switch <vm>
  --dry-run` or `nixling gc --dry-run`) before the first apply.

### Transition steps

1. Rebuild the host so the Rust CLI / daemon bits are on `$PATH`.
2. Leave existing generations in place; the native path reads the
   same `store-meta/` tree the bash path wrote.
3. Run the native verb with `--dry-run` first, then `--apply`. The
   v1.0 daemon-only contract (ADR 0015) is the only path; the
   historical `NIXLING_NATIVE_ONLY=1` env var is a no-op (its
   behaviour is the default).
4. If the daemon is unavailable, the CLI surfaces a typed
   `daemon-down` envelope (exit-1); the store layout stays intact
   because no mutation runs without the daemon.

### Rollback

- Roll back by reverting the host generation and rebuilding (the
  `NIXLING_LEGACY_BASH_OPT_IN=1` escape hatch was retired in v1.0
  along with the bash CLI; see ADR 0015 for the full removal list).
- Do **not** manually delete `store-meta` generations just because
  you changed control-plane owners; both paths still expect the
  retained rollback generation to exist.

## See also

- [`store-virtiofs.md`](./store-virtiofs.md)
- [`cli-contract.md`](./cli-contract.md)
- [`privileges.md`](./privileges.md)
