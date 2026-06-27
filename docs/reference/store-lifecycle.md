# Store lifecycle

Reference for d2b's per-VM `/nix/store` lifecycle.

This contract is implemented in two layers today:

- the shipping NixOS/module path in [`nixos-modules/store.nix`](../../nixos-modules/store.nix)
- the Rust primitive layer in [`packages/d2b-host/src/hardlink_farm.rs`](../../packages/d2b-host/src/hardlink_farm.rs)

The file names differ slightly between those layers, but the operator-visible
invariants are the same.

## On-disk layout

Per VM, d2b keeps the canonical store-view tree under
`/var/lib/d2b/vms/<vm>/store-view/`.

Rust `StoreSync` writes the ADR 0027 **split** layout, keyed by a
collision-free `generation_id` (SHA-256 over the full ordered closure
identity + system path, `g-<hex>`; the u32 token survives only as
display/wire `generation_token`):

| Path | Trust | Purpose |
| --- | --- | --- |
| `store-view/live/` | guest (ro) | The flat hardlink pool virtiofsd exposes to the guest. Every entry is a hardlink to a host `/nix/store` path, never a copy. |
| `store-view/live/.d2b-marker-<vm>` | guest (ro) | Zero-length cold-start readiness marker, planted **last**. |
| `store-view/meta/current -> generations/<id>` | guest (ro) | Active guest-served generation pointer. |
| `store-view/meta/generations/<id>/store-paths` | guest (ro) | Newline-delimited closure list. |
| `store-view/meta/generations/<id>/meta.json` | guest (ro) | Guest-safe allow-list metadata. |
| `store-view/meta/generations/<id>/db.dump` | guest (ro) | `nix-store --dump-db` payload for the retained closure. |
| `store-view/state/current -> generations/<id>` | host-only | Active host-side generation pointer (swapped **before** `meta/current`). |
| `store-view/state/generations/<id>/system` | host-only | Symlink to the host's system toplevel for that generation. |
| `store-view/state/generations/<id>/marker.json` | host-only | Typed generation marker used by the hardlink-farm primitive. |
| `store-view/state/generations/<id>/meta.json` | host-only | Host-only metadata (`vm`, link/skip counts). |
| `store-view/gcroots/generation-<id>` | host-only | Host-side GC root pinning the retained generation's system path. |
| `store-view/sync.lock` | host-only | `flock(2)` exclusion for the `StoreSync` op. |

The guest `d2b-meta` share is pointed at `store-view/meta/` only;
`state/`, `gcroots/`, and `sync.lock` are host-only and never exposed.

> **Transitional note:** the shipping `nixos-modules/store.nix`
> activation path and the legacy `build_farm`/rollback flows still use
> the older single-root layout — `store-view/current -> generations/<N>`
> with `generations/<N>/{system,store-paths,db.dump,marker.json}` and
> `gcroots/generation-<N>` — keyed by the u32 token. Consolidating those
> non-`StoreSync` callers onto the split layout is a follow-up wave.

The Rust primitive layer models crash-safety with a per-generation
`marker.json` and staged `current.tmp` symlinks before each final atomic
rename.

## Hardlink-farm invariants

d2b relies on these invariants for every per-VM store:

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

Hardlinks cannot cross filesystems. d2b therefore fails closed before it
tries to materialize a farm:

- the shell helper compares the filesystem ID of `/nix/store` and the per-VM
  state root and aborts if they differ;
- the Rust layer's `assert_same_filesystem` checks `st_dev` for the same reason.

If `/var/lib/d2b` and `/nix/store` are on different filesystems,
`switch`/`boot`/`test`/`rollback` refuse rather than silently copying data or
building an unusable tree.

## Marker checks

d2b uses two marker styles:

- **Farm marker**: `store-view/live/.d2b-marker-<vm>` is planted by the
  broker `StoreSync` writer as the cold-start readiness signal. Per
  [ADR 0027](../adr/0027-store-view-hardlink-live-pool.md) it is a
  **zero-length** file — existence alone is the signal (the readiness probe is
  a `test -e`), and it carries no host paths, generation metadata, or counts
  because it lives under the guest-served `live/` pool. The virtiofsd preflight
  checks both the real tree and the bind-mounted view visible to the service.
  A hand-made directory without the marker is refused.
- **Generation marker**: the Rust primitive layer writes the typed
  `marker.json` (under `state/generations/<id>/` for the split-layout
  `StoreSync` path, or `generations/<N>/` for the legacy path) and
  refuses to activate a generation if the marker is missing, malformed,
  or mismatched. Alongside it the primitive writes a guest-safe
  `meta.json` under `meta/generations/<id>/` (an independent allow-list
  serializer: `schema_version`, `generation_id`, `generation_token`,
  `sync_status`, `closure_count`).

Together, those checks stop both stale bind-mount views and partially created
activations from being mistaken for a valid store generation.

## Crash-safe symlink updates

The active generation pointer is always updated with a staged symlink and a
single rename:

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
`destructive: true` and audits every decision. Only `d2bd` may call those
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

- unkept `store-view/generations/<N>/` directories;
- their matching `store-view/gcroots/generation-<N>` links;
- host per-user GC roots for those generations;
- hardlink-farm entries not referenced by the union of retained `store-paths`
  files.

> **Transitional note:** the split-layout `StoreSync` path keys
> generations by `generation_id` (under `state/`/`meta/generations/<id>/`
> and `gcroots/generation-<id>`) and currently reports
> `cleanup_deferred: true` with `swept_count: 0` — retention/sweep for
> the split layout is a follow-up wave. The rule above still governs the
> legacy `generations/<N>` callers.

This is **per-VM** retention. It does not prune host NixOS system generations;
operators still need host-level garbage collection such as:

```bash
sudo nix-collect-garbage --delete-older-than 7d
```

## Upgrading from bash d2b

The Rust/daemon migration deliberately reuses the bash-era store
state on disk. There is no one-shot data migration as long as the
existing per-VM store tree is healthy.

### Bash-era state that stays in place

- `/var/lib/d2b/vms/<vm>/current`
- `/var/lib/d2b/vms/<vm>/booted`
- `/var/lib/d2b/vms/<vm>/store/` (legacy fallback only)
- `/var/lib/d2b/vms/<vm>/store-meta/generations/` (legacy fallback only)
- `/var/lib/d2b/vms/<vm>/store-meta/gcroots/` (legacy fallback only)
- `/var/lib/d2b/vms/<vm>/store-view/`

### Safety checks before the first native `--apply`

- confirm `/var/lib/d2b` and `/nix/store` are on the same
  filesystem;
- confirm `current` / `booted` still resolve and the latest retained
  generation still has its marker file;
- run `d2b generations <vm>` and a dry run (`d2b switch <vm>
  --dry-run` or `d2b gc --dry-run`) before the first apply.

### Transition steps

1. Rebuild the host so the Rust CLI / daemon bits are on `$PATH`.
2. Leave existing legacy generations in place as fallback artifacts; Rust
   `StoreSync` is the canonical writer for `store-view/`.
3. Run the native verb with `--dry-run` first, then `--apply`. The
   v1.0 daemon-only contract (ADR 0015) is the only path; the
   historical `D2B_NATIVE_ONLY=1` env var is a no-op (its
   behaviour is the default).
4. If the daemon is unavailable, the CLI surfaces a typed
   `daemon-down` envelope (exit-1); the store layout stays intact
   because no mutation runs without the daemon.

### Rollback

- Roll back by reverting the host generation and rebuilding (the
  `D2B_LEGACY_BASH_OPT_IN=1` escape hatch was retired in v1.0
  along with the bash CLI; see ADR 0015 for the full removal list).
- Do **not** manually delete `store-view` or `store-meta` generations just because
  you changed control-plane owners; both paths still expect the
  retained rollback generation to exist.

## See also

- [`store-virtiofs.md`](./store-virtiofs.md)
- [`cli-contract.md`](./cli-contract.md)
- [`privileges.md`](./privileges.md)
