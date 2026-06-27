# migrate-d2b-v0.1.0.sh — historical pre-v0.1.0 migration record

> **This document is a historical record** of the original
> consumer-side migration from a vendored `/etc/nixos/modules/d2b/`
> in-tree layout to consuming `github:vicondoa/d2b` `v0.1.0` as a
> flake input. It was written against one specific deployment
> (specific VM names, specific consumer flake checkout path) and is
> preserved here for reference — and as a worked example of a
> TPM-state-preserving migration — but it is **NOT** the general
> migration guide.
>
> If you are migrating to d2b from raw `microvm.nix`, see
> [`docs/how-to/migrating-from-microvm.md`](../docs/how-to/migrating-from-microvm.md).
>
> If you are starting fresh, see
> [`templates/default`](../templates/default/) or
> [`examples/minimal`](../examples/minimal/).
>
> The script itself (`scripts/migrate-d2b-v0.1.0.sh`) ships with
> the historical hostnames baked in; if you genuinely want to reuse
> the script's TPM-snapshot+verify-then-rename pattern for your own
> tree, fork it and adjust the VM-name arrays at the top.

# migrate-d2b-v0.1.0.sh

One-shot host migration script for moving `/var/lib/d2b/` state from
the old in-tree `/etc/nixos/modules/d2b/` layout to the new
[`vicondoa/d2b`](https://github.com/vicondoa/d2b) `v0.1.0`
external-flake layout.

**Purpose:** rename state dirs + disable obsolete systemd units **without
regenerating any swtpm/TPM state.** Preserving the persistent TPM
enrollment is the whole point: if it gets corrupted, Entra-ID will treat
the work VM as a tampered device and force re-enrollment, which Intune
flags as a security incident.

## Where it lives

This script is committed to the public flake at
`scripts/migrate-d2b-v0.1.0.sh` in the
[`vicondoa/d2b`](https://github.com/vicondoa/d2b) repo. Pull
it down with any of:

```
git clone https://github.com/vicondoa/d2b
nix flake archive github:vicondoa/d2b   # then `jq .path`
```

…and copy `scripts/migrate-d2b-v0.1.0.sh` somewhere on your host
(typically `/etc/nixos/scripts/`) before running it. The script is
single-file with no flake-eval dependencies, so you can also `curl`
the raw URL if that's faster.

## What it does, in order

1. **Acquire lock** on `/var/lib/d2b/.migration.lock` (flock,
   mode 0600 root:root) so two concurrent runs can't race.
2. **Pre-flight checks** (any failure → exit 1, except in `--dry-run` where
   they warn and continue):
   - Running as root.
   - **Full tool inventory** is on PATH: `sha256sum`, `mv`, `cp`, `rm`,
     `ln`, `install`, `date`, `du`, `df`, `stat`, `readlink`, `rmdir`,
     `mktemp`, `tac`, `wc`, `dirname`, `basename`, `printf`, `chmod`,
     `find`, `xargs`, `diff`, `awk`, `sed`, `grep`, `flock`,
     `systemctl`, `git`, `bash`. `tpm2_getcap`, `swtpm_setup`, `ssh`
     are optional (only needed when a TPM VM is running at snapshot
     time). Each missing required tool prints an explicit install hint.
   - `/etc/nixos` is a git repo with a clean working tree.
   - Workload VMs (`work-aad`, `personal-dev`, `d2b-test`) are
     `inactive`.
   - Net VMs (`work-router`, `personal-router`) are `inactive` — or the
     `--stop-net-vms` flag was passed, in which case they're stopped.
   - At least 2× the size of `/var/lib/d2b/` (or 2 GiB, whichever is
     larger) is free on the same filesystem.
3. **Pre-rename stop phase** — stops every sidecar that could mutate
   state during the snapshot window (this **must** happen before the
   snapshot — see "Why stop before snapshot" below):
   - **Critical sidecars** (`swtpm@<vm>`, `microvm-virtiofsd@<vm>`,
     `d2b-gpu@<vm>`) — stop failure is **fatal**. The script aborts
     with prescriptive recovery instructions; no state has been modified
     yet so `--rollback` is not needed (just fix the sidecar and re-run).
   - **Other sidecars** (`d2b-snd@<vm>`, `d2b-store-sync@<vm>`,
     `d2b-known-hosts-refresh@<vm>`) — stop failure is a warning.
   - **USBIPD units** — both old naming (`usbipd-d2b-*`) and the new
     W2 naming (`d2b-sys-<env>-usbipd-{backend,proxy}.{service,socket}`),
     discovered dynamically via `systemctl list-units`.
   - Runs `sync` to flush all dirty pages before the snapshot.
4. **Snapshot phase** (writes to `/var/lib/d2b-migration-backup/<ts>/`):
   - SHA256-hashes every file in `/var/lib/d2b/swtpm/<vm>/` and
     `/var/lib/private/d2b/swtpm/<vm>/` for each TPM-enabled VM
     (currently just `work-aad`).
   - Captures `tpm2_getcap properties-fixed` from the running VM if
     it's up, otherwise records `swtpm_setup --print-capabilities` and a
     note that in-guest verification has to wait for post-rebuild.
   - Writes a `.migration-in-progress` pointer file (atomically) so
     re-runs after a partial failure can find the snapshot.
   - When resuming a partial run that already recorded renames, the
     existing snapshot hashes are preserved as the verification anchor
     instead of being overwritten with current (post-rename) state.
5. **Rename phase** — `mv` for same-filesystem (the typical case),
   `cp -a --reflink=auto` + verify-checksums + `rm -rf` if the source
   and destination land on different filesystems. **Any temp/hash/diff
   failure in the cross-filesystem path is fatal BEFORE the source is
   removed**:
   - `/var/lib/d2b/<vm>/` → `/var/lib/d2b/vms/<vm>/` for each
     workload VM.
   - `/var/lib/d2b/work-router/` → `/var/lib/d2b/vms/sys-work-net/`
   - `/var/lib/d2b/personal-router/` → `/var/lib/d2b/vms/sys-personal-net/`
   - `/var/lib/d2b/swtpm/<vm>/` → `/var/lib/d2b/vms/<vm>/swtpm/`
     (TPM public state).
   - `/var/lib/private/d2b/swtpm/<vm>/` →
     `/var/lib/private/d2b/vms/<vm>/swtpm/` (TPM `DynamicUser` private
     state for `swtpm@.service`).
   - Runs `sync` to flush dirty pages.
6. **Verification phase** — re-hashes every TPM state file (both public
   AND private) at its new location and compares with the snapshot.
   **Any mismatch → ABORT immediately with instructions to `--rollback`.**
   This is the non-negotiable gate that protects the TPM enrollment.
7. **Unit-disable phase** — `systemctl disable --now` on the old-named
   units that the new flake will not recreate:
   - `swtpm@<vm>.service` (becomes `d2b-<vm>-swtpm.service`)
   - `d2b-snd@<vm>.service` (becomes `d2b-<vm>-snd.service`)
   - `d2b-gpu@<vm>.service` (becomes `d2b-<vm>-gpu.service`)
   - `d2b-store-sync@<vm>.service` (becomes `d2b-<vm>-store-sync.service`)
   - `microvm@work-router.service` (becomes `microvm@sys-work-net.service`)
   - `microvm@personal-router.service` (becomes `microvm@sys-personal-net.service`)
   - `usbipd-d2b.service`, `usbipd-d2b-<env>.{service,socket}`,
     `usbipd-d2b-<env>-backend.service` (replaced by
     `d2b-sys-*-usbipd-*`).
   - **Any `d2b-sys-*-usbipd-*` units already present** (from a
     partially-applied Phase 9 flake) are also disabled.
   - Records each disabled unit in the snapshot dir for rollback.
   - **Critical sidecar disable failures (swtpm/virtiofsd/gpu) are fatal**;
     other disable failures are warnings.
   - "Unit not found" is treated as an acceptable no-op (the script's
     worst-case list includes names that don't exist on every host).
   - Note: `microvm@work-aad`, `microvm@personal-dev`,
     `microvm@d2b-test` are **not** disabled — the attribute key
     stays unchanged in the new design, those units still exist.
8. **Back-compat symlink cleanup** — removes
   `/var/lib/microvms → /var/lib/d2b` and
   `/var/lib/swtpm → /var/lib/d2b/swtpm` (legacy shims from an
   earlier migration). Records them for rollback.
9. **Marker write (atomic)** — writes `/var/lib/d2b/.migration-state.json`
   via `mktemp` + `mv -T` so a crash mid-write can't leave a corrupt
   marker. Contains `migrationVersion: 1`, `appliedAt`, `fromVersion:
   "pre-v0.1.0"`, `toVersion: "v0.1.0"`, and a pointer to the snapshot
   dir. Subsequent runs check this and exit 0 if the version is already
   current. **A corrupt marker is fatal**, not silently re-migrated.
10. Prints the post-migration verification command, including the
    `d2b list` / `d2b status <vm>` smoke-test.

### Why stop before snapshot

The script's own forward-anomaly log (look for "anomaly #1") records
that on the maintainer's host, `swtpm@work-aad.service` was observed
**active** while `microvm@work-aad.service` was **inactive** — i.e.
`BindsTo` is not reliably propagating stops in the pre-v0.1.0 tree.
If swtpm is still running when we hash, an in-flight TPM write between
snapshot and rename would make the snapshot stale and silently break
the post-rename verification. Stopping all sidecars (and `sync`-ing)
first eliminates the race.

## Prerequisites checklist (before running)

Run through this before invoking the script:

- [ ] **/etc/nixos is clean**: `git -C /etc/nixos status --porcelain` is
      empty. Commit or stash any pending work first — the `nixos-usb-backup`
      service auto-commits dirty trees under a generic message, which
      would swallow your migration intent.
- [ ] **The new flake builds**: in a checkout of the post-Phase-9
      `/etc/nixos` (or a worktree), run
      `nixos-rebuild build --flake .#desktop` and confirm it succeeds.
      Do this BEFORE running the migration so you don't end up in a
      half-migrated state with no working closure to switch to.
- [ ] **A snapshot of `/var/lib/d2b` exists**: even though the
      script creates its own snapshot, an out-of-band backup (e.g.
      `tar` to an external disk) gives you a clean recovery path if
      `--rollback` itself fails.
- [ ] **The TPM-enabled VM is healthy** *before* you start: SSH in,
      run `tpm2_getcap properties-fixed`, confirm Entra-ID login still
      works, save the output for post-migration comparison.
- [ ] **You can run as root**: this host uses `sudo -A`, which pops a
      Plasma password dialog. Be at the console; don't try to do this
      over SSH unless `SUDO_ASKPASS` is wired up.

## Invocation

**Always dry-run first** to inspect the planned actions:

```bash
sudo -A bash /etc/nixos/scripts/migrate-d2b-v0.1.0.sh --dry-run
```

The dry-run prints `[DRY]` for every action it would take and exits 0.
It does not modify anything, even if pre-flight checks fail (they warn
instead).

Once you're happy with the dry-run output, run for real:

```bash
sudo -A bash /etc/nixos/scripts/migrate-d2b-v0.1.0.sh
```

If the net VMs (`work-router`, `personal-router`) are still up and you
want the script to stop them for you:

```bash
sudo -A bash /etc/nixos/scripts/migrate-d2b-v0.1.0.sh --stop-net-vms
```

The script is idempotent — re-running after success exits cleanly, and
re-running after a partial failure resumes from the existing snapshot.

## If something goes wrong → rollback

If the verification phase aborts, **do not** proceed to `nixos-rebuild`.
Run:

```bash
sudo -A bash /etc/nixos/scripts/migrate-d2b-v0.1.0.sh --rollback
```

Rollback:
- Finds the snapshot via `.migration-in-progress` or `.migration-state.json`.
- Reverses every recorded `mv` (in reverse order so nested moves undo
  correctly).
- Re-enables every recorded `systemctl disable`d unit.
- Recreates the back-compat symlinks.
- **Re-hashes BOTH public AND private swtpm state at the original paths**
  and verifies they match the snapshot — this is symmetric with the
  forward verification, so a corrupt private (`DynamicUser`) TPM dir
  cannot let rollback report false success.
- Removes the migration marker so the forward run can be retried.

After rollback, if you'd already started Phase 9 of the d2b refactor
on `/etc/nixos` (i.e. updated `flake.nix` inputs, added `d2b-site.nix`,
deleted `modules/d2b/`), revert those commits before running
`nixos-rebuild switch` so the system reactivates against the old layout.

## Common aborts and fixes

The script aims to abort early with prescriptive messages. Every fatal
exit prints: what failed → current state → whether `--rollback` is
needed → exact next commands.

| Symptom (the script prints…) | What it means | Recovery |
|------------------------------|---------------|----------|
| `Must run as root` | You forgot `sudo -A`. | Re-run with `sudo -A bash …`. No state changed. |
| `Pre-flight would FAIL: Workload VMs are still running` | `microvm@work-aad` or another is up. | `d2b down work-aad` etc., re-run. |
| `Pre-flight would FAIL: Net VMs are still running` | `microvm@work-router` is up. | Re-run with `--stop-net-vms`, or stop manually. |
| `/etc/nixos has uncommitted changes` | Working tree dirty. | `git -C /etc/nixos status`, commit or stash, re-run. |
| `Insufficient disk space on …` | < 2× state size free under `/var/lib`. | `nix-collect-garbage`, free up space, re-run. No state changed. |
| `Missing required tools: …` | `find`, `mktemp`, `diff` etc. not on PATH. | Each missing entry comes with an install hint; add to `environment.systemPackages`, rebuild, re-run. |
| `FATAL: critical sidecar would not stop: swtpm@work-aad.service` | `systemctl stop swtpm@<vm>` returned non-zero, or returned 0 but the unit is still active. | **No state has been moved.** Investigate the unit (`systemctl status`, `journalctl -xeu`), force-stop with `systemctl kill --signal=SIGKILL`, `reset-failed`, re-run. `--rollback` is **not** needed. |
| `Marker file is present but unparseable` | A previous run crashed mid-marker-write, or the file got hand-edited. | Inspect `/var/lib/d2b/.migration-state.json`; recover per the script's printed instructions. Do **not** silently `rm` it without checking the state layout. |
| `Marker file reports migrationVersion=<X>` (older than current) | Shouldn't happen with v0.1.0 (no prior migration version exists). | Inspect the file; either it was hand-edited or you're running a downgraded script. |
| `TPM HASH MISMATCH` (forward verify) | State was moved but byte content differs from snapshot. | **Run `--rollback` IMMEDIATELY.** Do NOT `nixos-rebuild`. Do NOT start `work-aad`. Booting in this state will look like device tampering to Entra-ID. |
| `Refusing to merge: both <src> and <dst> exist` | Either a partial previous run or a manual fix-up. | The script prints exact ls/rm/`--rollback` instructions; follow them. |
| `Cross-FS copy hash mismatch` | `cp -a` across filesystems produced different bytes (rare). | `rm -rf` the bad copy, investigate filesystem, `--rollback` if other renames were already done in this run. |
| `Failed to create temp file in <snapshot dir>` | Snapshot filesystem ran out of space mid-cross-FS-copy. | Free space, remove the partial destination, re-run. Source is intact (F7). |
| `Rollback hash mismatch` (public OR private) | State was reversed but bytes differ from snapshot. | Snapshot is intact; compare hashes manually to identify divergent files. Don't reboot. Restore from off-host backup if available. |
| Reboot mid-migration | Lock file releases when the holder exits. Marker writes are atomic, so either the marker is fully written or absent. | Re-run; the script detects the in-progress marker and resumes from the existing snapshot. |
| Ctrl+C mid-migration | flock auto-releases when bash exits. | Re-run; idempotent. If `--rollback` is needed, the in-progress marker still points at the snapshot. |

If anything not on this list happens, **don't improvise**. The script's
fatal-exit text is the authoritative recovery guide; it tells you what
state you're in and which command(s) to run.

## Manual follow-up after a successful run

The script **does not** run `nixos-rebuild`. That's the user's next step:

1. Commit the `/etc/nixos` changes that consume the new flake (Phase 9
   steps 1, 4–8 of the plan):
   - `flake.nix`: add `inputs.d2b.url = "github:vicondoa/d2b/v0.1.0";`
   - `flake.nix`: replace `./modules/d2b` import with
     `inputs.d2b.nixosModules.default`
   - Add `modules/d2b-site.nix` with `d2b.site.*` options
   - Add `modules/entrablau-site.nix` for the work VM
   - Move sbctl backup activation out of `host.nix`
   - Delete `/etc/nixos/modules/d2b/`
2. ```bash
   sudo -A nixos-rebuild switch --flake /etc/nixos#desktop
   ```
3. **Smoke-test the new unit names with `d2b list` / `d2b status`**
   (F11 verification):
   ```bash
   d2b list
   # Expect entries like:
   #   d2b@work-aad           (workload, status: stopped or running)
   #   d2b@personal-dev       (workload)
   #   microvm@sys-work-net       (system)
   #   microvm@sys-personal-net   (system)
   # Old names (microvm@work-router, etc.) must NOT appear.

   d2b status work-aad
   # Expect a healthy status report. 'unknown unit' or 'unit not found'
   # means the rebuild didn't pick up the new flake's unit definitions —
   # check that flake inputs/imports are correct before bringing VMs up.
   ```
4. Bring VMs back up:
   ```bash
   d2b up work-aad
   d2b up personal-dev
   # net VMs autostart under their new names

   d2b status work-aad       # expect: running, healthy
   d2b status personal-dev   # expect: running, healthy
   ```
5. **Verify TPM enrollment survived**:
   ```bash
   systemctl status d2b-work-aad-swtpm
   ssh work-aad.local tpm2_getcap properties-fixed
   ```
   Compare with the snapshot at
   `/var/lib/d2b-migration-backup/<ts>/tpm2_getcap/work-aad.txt`.
   If `tpm2_getcap` shows a freshly-initialised TPM (default vendor
   strings, no platform hierarchy), stop and run `--rollback`.

Keep the snapshot dir around for at least a week. Once you're confident
the new system is good (Entra-ID logins still work, no Intune
device-tampering alert), you can clear it:

```bash
sudo rm -rf /var/lib/d2b-migration-backup/<ts>
```

## Files

- `migrate-d2b-v0.1.0.sh` — the migration script.
- `README.md` — this file.

## Files written by the script (at runtime)

- `/var/lib/d2b/.migration.lock` — flock guard (empty file).
- `/var/lib/d2b/.migration-in-progress` — points at the snapshot
  while the script is running. Removed on clean finish or rollback.
- `/var/lib/d2b/.migration-state.json` — written on success.
  Subsequent runs treat `migrationVersion >= 1` as "already done".
- `/var/lib/d2b-migration-backup/<ts>/` — snapshot dir:
  - `hashes/<vm>__public.sha256` — SHA256s of `/var/lib/d2b/swtpm/<vm>/`.
  - `hashes/<vm>__private.sha256` — SHA256s of `/var/lib/private/d2b/swtpm/<vm>/`.
  - `tpm2_getcap/<vm>.txt` — captured TPM properties (or
    `swtpm_setup --print-capabilities` if the VM was stopped).
  - `renames.tsv` — `<src>\t<dst>` per rename, in execution order.
  - `disabled-units.txt` — one unit name per line.
  - `removed-symlinks.tsv` — `<link>\t<target>` per removed symlink.
- `/var/log/d2b-migration.log` — append-only run log.

## Why not just rely on the existing `host.nix` activation migrations?

The current `/etc/nixos/modules/d2b/host.nix` already has activation
blocks that migrated `/var/lib/microvms/` → `/var/lib/d2b/` and
`/var/lib/swtpm/` → `/var/lib/d2b/swtpm/`. Those blocks ran when
the in-tree tree last shipped them, and the back-compat symlinks they
left behind (`/var/lib/microvms`, `/var/lib/swtpm`) are still present.

This script is the next migration in that chain: `/var/lib/d2b/<vm>/`
→ `/var/lib/d2b/vms/<vm>/`. It is deliberately **not** an activation
block in the new `vicondoa/d2b` flake — that flake is a clean
external module aimed at multiple consumers, and bundling a one-time
host-specific data-migration into a reusable module is a bad fit. The
migration is a one-shot, ships out-of-band, and the new flake assumes
the host has already been migrated.

After this runs and the rebuild lands, `/etc/nixos/scripts/` retains
the script as a historical artifact so future-you knows what was done.
