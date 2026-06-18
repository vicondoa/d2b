# Per-VM state directory ownership matrix

Reference for the typed ownership matrix that the nixling daemon
enforces against every per-VM state subdirectory under
`/var/lib/nixling/vms/<vm>/`.

The matrix is declared in
[`nixos-modules/options-ownership-matrix.nix`](../../nixos-modules/options-ownership-matrix.nix)
as `nixling.daemon.perVmStateOwnershipMatrix` and enforced by
[`nixling_host::ownership_matrix`](../../packages/nixling-host/src/ownership_matrix.rs)
via a VM-start preflight wired into nixlingd
(`ownership_preflight::preflight` →
`TypedError::OwnershipMatrixDrift`).

This page documents the current ownership matrix the daemon enforces.

## CRITICAL: hardlink-farm carve-out

> Critical detail: the per-VM hardlink farm shares inodes with
> `/nix/store`; `setfacl -R` on the per-VM store propagates ACLs into
> `/nix/store` and breaks ssh `safe_path()` checks. Never run
> recursive ACL changes across the per-VM store.

The canonical per-VM `/nix/store` hardlink pool lives at
`store-view/live` (with the legacy `store` sibling retained only during
migration). Every package file in it is a hardlink whose inode is
shared with the corresponding file in `/nix/store`. Running a recursive
ownership / mode / ACL operation (`chown -R`, `chmod -R`,
`setfacl -R`) across this subtree mutates those shared inodes, which
propagates the change INTO `/nix/store`. The canonical regression
(personal-dev): a `setfacl -R` against
`/var/lib/nixling/vms/<vm>/store/` propagated a default ACL onto the
ssh host key paths inside `/nix/store`, which then failed openssh's
`safe_path()` check on next VM boot.

The daemon-side enforcer encodes this as a two-layer carve-out:

1. The Nix option declares `recursive = false` for the hardlink-pool
   roots (`store`, `store-meta`, `store-view`, `store-view/live`), with
   descriptions that name the hazard.
2. The Rust enforcer
   (`nixling_host::ownership_matrix::should_recurse`) hard-rejects
   recursion into any path in `HARDLINK_FARM_CARVE_OUTS`
   (`store`, `store-view/live`) regardless of the `recursive` field.
   Even if a future operator flips the option to `true`, the carve-out
   holds.

The live readiness marker `store-view/live/.nixling-marker-<vm>` is a
`file`-kind entry: it is checked with a single no-follow stat of that
named inode and is explicitly exempt from the carve-out (a direct stat
of one named file is safe; only a recursive *walk* into `live/` is
forbidden).

Both layers carry a dedicated unit test:
[`hardlink_farm_carve_out_holds_for_store_view_live` /
`hardlink_farm_carve_out_holds_for_legacy_store`](../../packages/nixling-host/src/ownership_matrix.rs).

## Canonical matrix

| Path (relative to `/var/lib/nixling/vms/<vm>/`) | Owner | Group | Mode | Kind | Required | Recursive | Rationale |
|---|---|---|---|---|---|---|---|
| `.` | `nixlingd` | `users` | `3770` | dir | yes | false | Per-VM state root. `setgid` so role users (runner / gpu / swtpm) inherit the group on files they create; `sticky` (`+t`) so a role UID (which holds rwx via POSIX ACL) cannot rename/unlink entries it does not own — notably the principal-owned `swtpm` NVRAM dir (issue #64). |
| `state` | `nixlingd` | `nixling` | `0750` | dir | yes | false | Daemon-owned per-VM state (`audio-state.json`, etc.). |
| `swtpm` | `nixling-<vm>-swtpm` | `nixling-<vm>-swtpm` | `0700` | dir | yes | false | **CRITICAL SUBSYSTEM** (AGENTS.md): per-VM TPM 2.0 NVRAM. Wiping or rechowning this directory looks like device tampering to any IdP (Entra ID / Intune / BitLocker-class policies) and forces re-enrollment. Owned by the per-VM swtpm runner principal. |
| `sshd-host-keys` | `nixlingd` | `nixling` | `0750` | dir | yes | false | Container for per-VM sshd host keys. The daemon refuses to start the VM if any leaf has drifted (see [ssh-host-key-preflight.md](./ssh-host-key-preflight.md)). |
| `host-keys` | `nixlingd` | `nixling` | `0750` | dir | yes | false | Known-hosts pin store for per-VM ssh host key fingerprints. |
| `store` | `nixlingd` | `users` | `0755` | dir | **no (legacy)** | **false (carve-out)** | Legacy pre-store-view per-VM `/nix/store` hardlink farm retained only during migration. `required = false`: posture-if-present on native VMs. |
| `store-meta` | `nixlingd` | `users` | `0755` | dir | **no (legacy)** | false | Legacy metadata sibling retained only during migration. |
| `store-view` | `nixlingd` | `users` | `0755` | dir | yes | false | Canonical store-view root; holds the served `live/` pool, the guest-readable `meta/` subtree, and the host-only `state/`, `gcroots/`, `sync.lock`. |
| `store-view/live` | `nixlingd` | `users` | `0755` | dir | yes | **false (carve-out)** | Canonical per-VM `/nix/store` hardlink pool. Served read-only to the guest as `/nix/.ro-store`. The enforcer NEVER recurses into this path. |
| `store-view/meta` | `nixlingd` | `users` | `0755` | dir | yes | false | Guest read-only metadata share root. Served read-only as `/run/nixling-store-meta`. Runner/virtiofsd-readable. |
| `store-view/meta/generations` | `nixlingd` | `users` | `0755` | dir | yes | false | Guest-readable per-generation metadata directory under `store-view/meta`. |
| `store-view/state` | `nixlingd` | `nixling` | `0750` | dir | yes | false | **HOST-ONLY** broker StoreSync state. `nixling:nixling 0750` — the runner/virtiofsd identity has no access. Must NOT reuse the runner-readable `users 0755` posture. |
| `store-view/state/generations` | `nixlingd` | `nixling` | `0750` | dir | yes | false | **HOST-ONLY** per-generation broker state directory. Per-generation leaves (`marker.json`, `meta.json`, `integrity.json`) are `nixling:nixling 0640`, repaired out of band. |
| `store-view/gcroots` | `nixlingd` | `nixling` | `0750` | dir | yes | false | **HOST-ONLY** StoreSync GC roots: host-absolute symlinks into `/nix/store` protecting retained closures from host GC. Never guest/runner-readable. |
| `store-view/sync.lock` | `nixlingd` | `nixling` | `0600` | **file** | yes | n/a | **BROKER-PRIVATE** StoreSync serialization lock. File-kind: enforcer reasserts mode/uid/gid on the file inode with no-follow semantics. |
| `store-view/state/integrity-unknown.json` | `nixlingd` | `nixling` | `0640` | **file** | **no** | n/a | **HOST-ONLY** VM-level integrity fallback record. File-kind, created lazily; absence before first use must not fail preflight. |
| `store-view/live/.nixling-marker-<vm>` | `nixlingd` | `users` | `0644` | **file** | **no** | n/a | Guest-readable live readiness marker. Zero-length, file-kind single-inode check; exempt from the `live/` no-recursion carve-out. Absent before first sync. |

The `<vm>` token in `owner` / `group` (and in the marker `path`) is
substituted with the VM name at enforcement time. This keeps the matrix
VM-agnostic — every VM shares the same shape.

`kind` is `dir` (default) or `file`. `file`-kind entries assert the
inode is a regular file (no-follow) and reassert mode/uid/gid on it;
`dir`-kind entries assert a directory. `required` (default `true`)
controls only the ENOENT case: a `required = false` entry that is
absent is silently skipped, while a `required = true` entry that is
absent downgrades to a preflight warning during the migration window
(see Enforcement posture). A `kind` mismatch (file where a directory is
expected, or vice versa) is always fail-closed.

## Cross-reference: `minijail-profiles.nix` `writablePaths`

The matrix above describes WHO OWNS each subdirectory; the per-role
minijail profiles in
[`nixos-modules/minijail-profiles.nix`](../../nixos-modules/minijail-profiles.nix)
describe WHAT each runner role may write inside the per-VM tree:

| Role | `writablePaths` under `/var/lib/nixling/vms/<vm>/` | Matrix entry covering it |
|---|---|---|
| `host-reconcile` | the per-VM state dir | `.` |
| `store-virtiofs-preflight` | `.` / `store-view` / `store-view/live` (read-only) | `.`, `store-view`, `store-view/live` |
| `cloud-hypervisor-runner` | the per-VM state dir | `.` |
| `swtpm-pre-start-flush` | `swtpm` | `swtpm` |
| `swtpm` (long-lived) | `swtpm` | `swtpm` |
| `virtiofsd` (per-share) | the per-VM state dir, `/run/nixling/...` | `.` |

If a future role declares a new `writablePaths` row that isn't covered
by an existing matrix entry, add the entry to both
`nixos-modules/options-ownership-matrix.nix` AND
`packages/nixlingd/src/ownership_preflight.rs::CANONICAL_MATRIX`. The
shapes are mirrored so the Nix declaration is authoritative for
operators while the Rust constant carries the runtime resolution
(uid/gid lookup via NSS).

## Enforcement posture

The preflight is invoked unconditionally from
`dispatch_broker_vm_start`. The posture is:

- **Missing per-VM state directory** → warn-only (state is
  materialized lazily; a fresh host or a test fixture will hit this
  on the first start).
- **Unresolvable principal** (e.g. `nixling-<vm>-swtpm` user not yet
  provisioned because `tpm.enable = false`) → warn-only; the entry
  is skipped.
- **Optional entry absent (ENOENT)** — a `required = false` entry
  whose path does not exist (legacy `store` / `store-meta`,
  `integrity-unknown.json`, the live marker before first sync) → no
  drift emitted (silently skipped).
- **Required entry absent (ENOENT)** — a `required = true` entry whose
  path does not exist → `StatFailed { not_found: true }`, downgraded to
  a preflight **warning** and skipped. This is the migration-window
  posture: broker StoreSync prep (which creates `store-view/state`,
  `gcroots`, `sync.lock`, `meta`) is not yet wired, so a fresh host
  must not fail-closed on their absence.
- **Any other stat error** (EACCES, ELOOP, etc.) → `StatFailed
  { not_found: false }` → fail closed.
- **Kind mismatch** — a file where a directory is expected (or vice
  versa), checked no-follow so a symlink at a leaf never satisfies the
  entry → fail closed.
- **Present and owner/group/mode matches** → silent OK.
- **Present and owner/group/mode differs** → fail closed. The daemon
  returns `TypedError::OwnershipMatrixDrift { vm, path, drift_reason }`,
  exit code `61`, with the operator-facing message listing every
  drifted axis (`owner 100→0, group 200→0, mode 750→755`).

Operator recovery:

1. `nixos-rebuild switch` — re-runs the host-activation posture chain
   for the allowed top-level roots (`store-view`, `store-view/live`,
   `store-view/meta`, plus legacy `store` / `store-meta` if present).
   Host activation does NOT posture the host-only leaves
   (`store-view/state`, `gcroots`, `sync.lock`, integrity records);
   those are broker-owned and repaired out of band.
2. If activation cannot fix it (manual operator change, broken
   migration), `chown` / `chmod` the listed entries by hand. **Never**
   run a recursive ownership/ACL op across `store/` or
   `store-view/live/`.

## Tests

- Unit (Rust):
  [`packages/nixling-host/src/ownership_matrix.rs`](../../packages/nixling-host/src/ownership_matrix.rs)
  `tests::*` — happy path, per-axis drift, missing path (optional vs
  required ENOENT), `kind` mismatch (file↔dir, no-follow symlink),
  file-kind mode reassertion, and the
  `hardlink_farm_carve_out_holds_for_{store_view_live,legacy_store}`
  regressions.
- Preflight (Rust):
  [`packages/nixlingd/src/ownership_preflight.rs`](../../packages/nixlingd/src/ownership_preflight.rs)
  `tests::*` — missing state dir is clean, unresolvable principals are
  clean, and drift-message rendering (per-axis, kind mismatch, and the
  non-ENOENT stat-failure variant).
- Integration (shell):
  [`tests/per-vm-state-ownership-eval.sh`](../../tests/per-vm-state-ownership-eval.sh)
  — confirms `nixling.daemon.perVmStateOwnershipMatrix` is non-empty,
  every entry carries every required typed field (including `kind` and
  `required`), the hardlink-pool entries have `recursive = false`, the
  `swtpm` entry uses `<vm>` in its principal templates, the signed
  store-view layout is present (guest `store-view/meta`, host-only
  `store-view/state` + `store-view/gcroots` at `nixling:nixling 0750`,
  file-kind `sync.lock` and live marker), the retired
  `store-view/generations` path is gone, and the `nixling-store-sync`
  directory ownership fix-up matches the matrix (directory-only
  `chown nixlingd:users` / `chmod 0755`, no `root:kvm`, no `2775`).
