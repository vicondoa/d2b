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

The `store` subdirectory under `/var/lib/nixling/vms/<vm>/` is a
hardlink farm: every file in it is a hardlink whose inode is shared
with the corresponding file in `/nix/store`. Running a recursive
ownership / mode / ACL operation (`chown -R`, `chmod -R`,
`setfacl -R`) across this subtree mutates those shared inodes, which
propagates the change INTO `/nix/store`. The canonical regression
(personal-dev, today): a `setfacl -R` against
`/var/lib/nixling/vms/<vm>/store/` propagated a default ACL onto the
ssh host key paths inside `/nix/store`, which then failed openssh's
`safe_path()` check on next VM boot.

The daemon-side enforcer encodes this as a two-layer carve-out:

1. The Nix option declares `recursive = false` for `store` (and
   `store-meta`), with a description that names the hazard.
2. The Rust enforcer
   (`nixling_host::ownership_matrix::should_recurse`) hard-rejects
   recursion into `path == "store"` regardless of the `recursive`
   field. Even if a future operator flips the option to `true`, the
   carve-out holds.

Both layers carry a dedicated unit test:
[`hardlink_farm_carve_out_holds`](../../packages/nixling-host/src/ownership_matrix.rs).

## Canonical matrix

| Path (relative to `/var/lib/nixling/vms/<vm>/`) | Owner | Group | Mode | Recursive | Rationale |
|---|---|---|---|---|---|
| `.` | `nixlingd` | `users` | `2770` | false | Per-VM state root. `setgid` so role users (runner / gpu / swtpm) inherit the group on files they create. |
| `state` | `nixlingd` | `nixling` | `0750` | false | Daemon-owned per-VM state (`audio-state.json`, etc.). |
| `swtpm` | `nixling-<vm>-swtpm` | `nixling-<vm>-swtpm` | `0700` | false | **CRITICAL SUBSYSTEM** (AGENTS.md): per-VM TPM 2.0 NVRAM. Wiping or rechowning this directory looks like device tampering to any IdP (Entra ID / Intune / BitLocker-class policies) and forces re-enrollment. Owned by the per-VM swtpm runner principal. |
| `sshd-host-keys` | `nixlingd` | `nixling` | `0750` | false | Container for per-VM sshd host keys. The daemon refuses to start the VM if any leaf has drifted (see [ssh-host-key-preflight.md](./ssh-host-key-preflight.md)). |
| `host-keys` | `nixlingd` | `nixling` | `0750` | false | Known-hosts pin store for per-VM ssh host key fingerprints. |
| `store` | `nixlingd` | `users` | `2775` | **false (carve-out)** | Per-VM `/nix/store` hardlink farm. See the CRITICAL section above. The enforcer NEVER recurses into this path; the matrix verifies only the top-level dir's owner/group/mode. |
| `store-meta` | `nixlingd` | `users` | `2775` | false | StoreSync metadata sibling: `current` symlink, per-generation marker, gcroots. Although not hardlinked into `/nix/store`, `recursive` is kept false so the "no recursive ownership ops on per-VM store state" rule applies uniformly. |

The `<vm>` token in `owner` / `group` is substituted with the VM name
at enforcement time. This keeps the matrix VM-agnostic — every VM
shares the same shape.

## Cross-reference: `minijail-profiles.nix` `writablePaths`

The matrix above describes WHO OWNS each subdirectory; the per-role
minijail profiles in
[`nixos-modules/minijail-profiles.nix`](../../nixos-modules/minijail-profiles.nix)
describe WHAT each runner role may write inside the per-VM tree:

| Role | `writablePaths` under `/var/lib/nixling/vms/<vm>/` | Matrix entry covering it |
|---|---|---|
| `host-reconcile` | the per-VM state dir | `.` |
| `store-virtiofs-preflight` | `.` / `store` / `store-meta` (read-only) | `.`, `store`, `store-meta` |
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
- **Subdirectory present and owner/group/mode matches** → silent OK.
- **Subdirectory present and owner/group/mode differs** → fail
  closed. The daemon returns
  `TypedError::OwnershipMatrixDrift { vm, path, drift_reason }`,
  exit code `61`, with the operator-facing message listing every
  drifted axis (`owner 100→0, group 200→0, mode 750→755`).

Operator recovery:

1. `nixos-rebuild switch` — re-runs the host-activation chown chain.
2. If activation cannot fix it (manual operator change, broken
   migration), `chown` / `chmod` the listed entries by hand. **Never**
   run a recursive ownership/ACL op across `store/`.

## Tests

- Unit (Rust):
  [`packages/nixling-host/src/ownership_matrix.rs`](../../packages/nixling-host/src/ownership_matrix.rs)
  `tests::*` — happy path, per-axis drift, missing path, and the
  `hardlink_farm_carve_out_holds` regression.
- Preflight (Rust):
  [`packages/nixling-host/../../nixlingd/src/ownership_preflight.rs`](../../packages/nixlingd/src/ownership_preflight.rs)
  `tests::*` — missing state dir is clean, drift message rendering.
- Integration (shell):
  [`tests/per-vm-state-ownership-eval.sh`](../../tests/per-vm-state-ownership-eval.sh)
  — confirms `nixling.daemon.perVmStateOwnershipMatrix` is non-empty,
  every entry carries every required typed field, the `store` entry
  has `recursive = false`, and the `swtpm` entry uses `<vm>` in its
  principal templates.
