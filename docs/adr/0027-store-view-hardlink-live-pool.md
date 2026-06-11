# ADR 0027: Hardlink-backed store-view live pool

- Status: Draft — plan/design panel signed off
- Date: 2026-06-09
- Related: ADR 0015 (daemon-only clean break), ADR 0017 (no bash fallbacks), ADR 0018 (microvm.nix removal), ADR 0021 (broker user namespace for virtiofsd)

The nixling default panel has signed off on this ADR and the accompanying
session plan. Existing worktree diffs before signoff are WIP evidence only.

## Context

nixling currently has a legacy flat per-VM hardlink farm at
`<stateDir>/<vm>/store` that virtiofsd serves to the guest, plus a
partially-wired daemon-native `store-view/generations/<N>` path that
materializes full hardlink trees per generation.

The legacy flat farm is live-switch-friendly because virtiofsd serves
one stable directory and new top-level paths can be added in place. The
full generation tree repeats directory-entry work for large closures.

## Decision

Use hard links only. Rust broker `StoreSync` is the sole writer to the
canonical `store-view` tree. No bash store-view writer, env knob, backend
toggle, or custom filesystem/device is introduced.

Deep live-pool verification/repair is explicit operator surface, not VM start
fast-path behavior: `nixling store verify <vm> [--repair] [--json]`.

Target layout:

```text
<stateDir>/<vm>/store-view/
  live/
    <hash>-pkg/
    .nixling-marker-<vm>
  meta/
    current -> generations/<generation-id>
    generations/
      <generation-id>/
        store-paths
        db.dump
        meta.json
  state/
    current -> generations/<generation-id>
    integrity-unknown.json
    generations/
      <generation-id>/
        system -> /nix/store/<hash>-nixos-system-<vm>-...
        marker.json
        meta.json
        integrity.json
    generation-log.json
  gcroots/
    generation-<generation-id> -> /nix/store/<hash>-nixos-system-<vm>-...
  sync.lock
```

`live/` is served read-only by virtiofsd as `/nix/.ro-store`.
`meta/` is served read-only as `/run/nixling-store-meta`.
`state/`, `gcroots/`, and `sync.lock` are host-only.

The on-disk `generation-id` is collision-free (for example full closure
hash). Any u32 generation token is display/wire metadata only and is not
used as the directory key.

## Trust and security model

Actors:

- trusted writer: `nixling-priv-broker` StoreSync;
- untrusted: guest;
- potentially compromised: runner/virtiofsd process.

Invariants:

- the guest never obtains a writable fd to hardlinked lower-store inodes;
- runner/virtiofsd has read-only host access to `live/` and `meta/`, and
  no write access to marker files, current symlinks, or `sync.lock`;
- runner/virtiofsd has no host access to `state/`, `gcroots/`,
  broker-authoritative marker files, or `sync.lock`;
- caller identity comes from kernel peer credentials on the trusted broker IPC
  channel, not from caller-supplied data;
- StoreSync derives closure paths and `db.dump` from the trusted bundle;
- the `db.dump` is an exact closure-scoped DB export:
  `registered set == store-paths`, never a host DB superset;
- recursive `chmod`, `chown`, and `setfacl` never touch hardlinked
  regular files;
- broad inherited access and default ACLs are cleared on `store-view`, `live`,
  `meta`, `state`, `gcroots`, generation dirs, and stage parents before use;
- StoreSync authorizes caller before lock acquisition or filesystem side
  effects and audits caller identity, authz outcome, target VM, and
  `error_stage = authz` on denial.

Guest-visible metadata is limited to `meta/current`, `store-paths`, `db.dump`,
and guest-safe `meta.json`. Guest `meta.json` is produced by an independent
serializer whose key set must equal this positive allow-list:

- `schema_version`
- `generation_id`
- `generation_token`
- `sync_status`
- `closure_count`

The guest serializer never receives the full host audit struct. Broker markers,
host absolute `system` symlinks, gcroots, caller/authz fields, retained
generations, swept counts, timings, cleanup fields, error details, and host-only
paths are not guest-visible.

`live/.nixling-marker-<vm>` is guest-readable because it lives under `live/`.
It is a zero-length readiness marker and carries no host paths, caller
principal, generation metadata, timings, counts, audit fields, or other payload.

## StoreSync audit schema

Every StoreSync attempt emits exactly one terminal structured audit record.
`in_progress` may exist as transient state but does not increment terminal
attempt metrics. Host-side `state/generations/<id>/meta.json` may carry the same
full record. Guest `meta/generations/<id>/meta.json` carries only the
allow-listed guest-safe subset.

Fields:

- `schema_version`
- `vm` / `vm_id`
- `env` (host-audit field; never in guest metadata; not a broker-emitted
  StoreSync metric label)
- `generation_id` (string, audit/guest-meta field, never metric label)
- `generation_token` (u32, audit/guest-meta field, never metric label)
- `sync_status`: `ok`, `failed`, `in_progress`
- `error_stage`: `none`, `authz`, `lock`, `probe`, `verify`, `stage`,
  `rename`, `metadata`, `integrity`, `current_swap`, `marker`
- `caller_principal` (audit only, never metric label)
- `authz_outcome`: `allow`, `deny`
- `closure_count`
- `linked_count`
- `skipped_count`
- `retained_generations` (audit only, never metric label)
- `swept_count`
- `fast_path`
- `cleanup_status`: `not_attempted`, `completed`, `deferred_online`,
  `deferred_ambiguous`, `deferred_metadata`, `skipped_fast_path`,
  `failed`
- `cleanup_reason`: `none`, `vm_running`,
  `running_generation_ambiguous`, `missing_retained_metadata`,
  `io_error`, `fast_path`
- `total_ms`, `lock_wait_ms`, `lock_hold_ms`, `probe_ms`, `verify_ms`,
  `stage_ms`, `metadata_ms`, `sweep_ms`, `cleanup_ms`

Invariants:

- `sync_status = ok` implies `error_stage = none`;
- cleanup failures after activation use `sync_status = ok`,
  `error_stage = none`, and `cleanup_status = failed` with
  `cleanup_reason = io_error`;
- failures before the cleanup/sweep phase use `sync_status = failed`,
  `cleanup_status = not_attempted`, and `cleanup_reason = none`;

Valid cleanup pairs:

- `completed` + `none`
- `not_attempted` + `none`
- `deferred_online` + `vm_running`
- `deferred_ambiguous` + `running_generation_ambiguous`
- `deferred_metadata` + `missing_retained_metadata`
- `skipped_fast_path` + `fast_path`
- `failed` + `io_error`

Telemetry rules:

- for `sync_status = ok`, `linked_count + skipped_count == closure_count`;
  for failed records before accounting is complete, `closure_count` is the
  enumerated closure size if known (else 0), and linked/skipped counts report
  completed work without being forced to sum to `closure_count`;
- `fast_path = true` always implies `linked_count = 0`;
- pure fast path has `linked_count = 0`, `skipped_count = closure_count`,
  `swept_count = 0`, `cleanup_status = skipped_fast_path`;
- fast path that reconciles deferred cleanup may have `swept_count > 0`
  and `cleanup_status = completed`;
- metric labels are allow-listed to `sync_status`, `error_stage`,
  `cleanup_status`, `cleanup_reason`, `fast_path`, and `authz_outcome`;
- terminal attempt metric values for `sync_status` are limited to `ok` and
  `failed`; `in_progress` is transient only and never emitted as a terminal
  metric value;
- store-path basenames, generation IDs/tokens, retained-generation lists, and
  caller principals are never broker-emitted StoreSync metric labels;
- VM/env attribution is carried in the structured audit record; it is not added
  as a Loki stream/index label on the single broker audit stream;
- `db.dump`, `marker.json`, and `store-paths` payloads are never logged.

StoreSync-only observability export uses an independent positive-allow-list
serializer that never receives the full host audit struct. Exported fields:

- `schema_version`
- `target_vm` / `vm_id`
- `target_env`
- `generation_id`
- `generation_token`
- `sync_status`
- `error_stage`
- `cleanup_status`
- `cleanup_reason`
- `authz_outcome`
- `closure_count`
- `linked_count`
- `skipped_count`
- `swept_count`
- `fast_path`
- `total_ms`, `lock_wait_ms`, `lock_hold_ms`, `probe_ms`, `verify_ms`,
  `stage_ms`, `metadata_ms`, `sweep_ms`, `cleanup_ms`

It explicitly excludes `caller_principal`, `retained_generations`, host paths,
store paths/basenames, `db.dump`, marker payloads, and any future host-only
audit fields by default.

## Sync algorithm

StoreSync assumes a prepared and postured store-view root already exists.

1. Authorize caller for target VM. Denial has no filesystem side effects.
2. Acquire an in-process per-VM async mutex, then `flock` `sync.lock`
   on a blocking thread with bounded wait/backoff and an explicit request
   deadline. Timeout/cancellation is caller-facing only: it does not release
   the lock while workers may still be writing. The broker cancels
   cooperatively, waits for worker quiescence, records the failed stage, cleans
   once, and only then unlocks. Same-vfsmount in-process workers check
   cancellation between top-level path operations; lock release may exceed the
   caller deadline while quiescence completes and is reported separately.
   Helper-backed work uses a dedicated single-process helper: no shell, no
   child processes, no background work, and no subprocess copy tools. The
   helper performs namespace setup, detach, link/copy, and fsync work itself.
   The broker starts it in a dedicated leaf cgroup and restricts it with a
   helper-specific confinement profile: only the mount capability needed for
   namespace/detach work, a seccomp profile for the required filesystem/mount
   syscalls, and an RLIMIT_NPROC/single-process policy. Cancellation closes the
   helper IPC boundary, escalates SIGTERM then cgroup.kill/SIGKILL after a
   bounded grace, waits for `cgroup.events populated=0`, and reaps the direct
   helper with pidfd/waitpid before cleanup/unlock.
3. Remove orphan `live.stage.*` siblings from crashed syncs.
4. Reconcile stale `state/current.tmp` and `meta/current.tmp`. Also reconcile
   committed current divergence before any fast-path decision: `state/current`
   is the host-authoritative pointer. If `state/current` and `meta/current`
   disagree and the `state/current` generation has valid host + guest metadata
   and complete live basenames, repoint `meta/current` to `state/current` and
   fsync `meta/`; otherwise fail with `error_stage=current_swap` and do not
   fast-path.
5. Probe topology using actual source paths and the stage parent: differing
   `st_dev` is fatal; same-`st_dev` cross-vfsmount `EXDEV` uses a separately
   exec'd private mount namespace helper. Record pre-helper `st_dev`/`st_ino`
   for every source path. The helper must recursively set mount propagation
   private/slave before any lazy `/nix/store` detach so helper success/error/
   SIGKILL cannot affect host `/nix/store`. After helper-side `/nix/store`
   detach, source and stage parent must share one vfsmount and every source path
   must re-stat to the exact pre-helper `st_dev`/`st_ino`; if a stage/live
   ancestor remains a separate vfsmount or a source identity changes/disappears,
   fail deterministically with `error_stage=probe` before partial live changes.
6. Fast-path guard verifies both `state/current` and `meta/current` equal the
   requested `generation_id`, then verifies current marker, live marker, and
   every closure top-level basename in `live/`. Pure fast path is O(top-level
   closure paths): it trusts the host-authoritative manifest and readiness
   marker produced by a prior successful non-fast-path materialization and does
   not recursively walk package subtrees. A `suspect` or `unknown` integrity
   state disables pure fast path for that generation. VM-start StoreSync may
   still materialize missing top-level paths, but it must leave integrity
   unchanged unless the record is `suspect` with a concrete `drift_signature`
   and StoreSync re-materializes/replaces and verifies every top-level path
   implicated by that signature.
7. Stage missing top-level paths into a `live.stage.*` sibling outside
   the served `live/` root.
8. Same-vfsmount hardlink workers live in shared Rust `build_farm`.
   Cross-vfsmount `EXDEV` delegates the actual link/copy work to the
   helper process, which returns counts and errors over an explicit boundary.
   On worker/helper error, panic, or request-deadline cancellation: convert to
   a per-VM error, cancel siblings, wait for quiescence, then clean the stage
   tree once.
9. Before any top-level rename, fsync staged package subtrees bottom-up in both
   the same-vfsmount and helper paths: fsync every created directory containing
   new hardlink/symlink/copy dirents, then fsync the stage parent. EMLINK copy
   fallback additionally fsyncs copied files and produces a byte-for-byte
   content-equal and mode/executable-bit-equal independent copy. It is not used
   for cross-filesystem EXDEV.
10. Rename staged basenames into `live/` and fsync live parent.
11. Write guest-safe metadata under `meta/generations/<id>/` and
    host-only metadata under `state/generations/<id>/`; fsync files and
    directories.
12. Before either `state/current` or `meta/current` is swapped, every newly
    materialized or replaced path must pass internal completeness verification
    against authoritative source metadata. Failure in this verification emits
    `error_stage=verify`, `sync_status=failed`,
    `cleanup_status=not_attempted`, and `cleanup_reason=none`. Existing live paths that are
    referenced by a valid current marker/manifest are trusted on the pure fast
    path and may be deeply verified only by an explicit verification/repair
    operation outside the VM-start critical path.
13. After step 12 succeeds and before current swaps, write generation-scoped
    `integrity.json` with `state=ok`, no `drift_signature`, and no
    `unknown_reason` only when permitted by the integrity-clearing rules: new
    generation, or all paths implicated by an existing `suspect`
    `drift_signature` were re-materialized/replaced and verified, or explicit
    full verify/repair completed cleanly. If an existing suspect/unknown record
    is not covered by those conditions, preserve it unchanged. Integrity-record
    write/fsync failure is fatal before current swaps and emits
    `error_stage=integrity`, `sync_status=failed`, `cleanup_status=not_attempted`,
    and `cleanup_reason=none`.
14. Commit current symlinks in fixed crash-recoverable order: atomically swap
    `state/current` first and fsync `state/`, then atomically swap
    `meta/current` and fsync `meta/`. The pair is not atomic as a unit; step 4
    is the recovery path for a crash between the two swaps.
15. Plant/refresh `live/.nixling-marker-<vm>` via tmp+rename only after
    `live/` contains every required basename for `meta/current`.
    The marker is the cold-start readiness signal; `meta/current` is not.
16. Compute retained generations and sweep only when safe.
17. Update host-only GC roots after safe cleanup decisions. During StoreSync,
    the trusted bundle toplevel and runtime closure remain protected by their
    existing bundle/profile roots until the new host-only `gcroots/` entries are
    planted, so concurrent host GC cannot collect a source path that has already
    been linked into `live/`.

Cleanup failures after step 15 do not fail activation; they return
StoreSync success with cleanup failed/deferred status and retained
overexposure.

`live-index.json`, if added later, is not authoritative for destructive
cleanup. Sweeps derive desired basenames from retained generations'
authoritative `store-paths`.

## Retention and cleanup

Generation ordering uses an activation journal/index and current/rollback
markers, not numeric generation-token ordering.

Retain current, running, rollback, pending, and most-recent previous
generation as appropriate. Sweep only from the union of retained
generations' authoritative `store-paths`; missing retained metadata defers
cleanup. If running-generation detection is ambiguous, defer cleanup.

Offline cleanup requires virtiofsd for that VM not serving `live/`, not merely
cloud-hypervisor exit. Any live virtiofsd process/cgroup for the VM, open fd
under `live/`, or uncertainty defers cleanup.

Every live/metadata mutator — StoreSync, `nixling gc`, and VM-stop
deferred cleanup — acquires `sync.lock` and revalidates that virtiofsd
is not serving `live/` before destructive shrinkage.

## Kernel assumptions

`store-view/live` and realized `/nix/store` must share `st_dev`.
Cross-filesystem is fatal and unsupported. Copy fallback applies only to
EMLINK link-count saturation.

The design relies on live-additive behavior from the legacy flat farm:
virtiofsd serves a stable directory, new top-level lower entries can be
added while a VM runs, and guest overlayfs performs on-demand lower
lookups. Validation must cover `cache=auto` negative-dentry behavior.

## Operator recovery

Migrated-vs-native provenance is host-only state recorded at migration/VM
creation time. `migrated` VMs had legacy `store`/`store-meta` artifacts at
cutover and those artifacts are protected rollback inputs. `native` VMs were
created after cutover, never had legacy artifacts, are reported as
`native -- no legacy rollback path`, and are excluded from the legacy-artifact
readiness requirement.

Recovery requires a legacy-serving host generation or pinned flake input
plus every migrated VM's legacy artifact.

If a VM fails after cutover:

1. verify host-level rollback readiness for migrated VMs;
2. revert nixling flake input or host generation to a legacy-serving version;
3. rebuild/switch host;
4. restart `nixlingd`;
5. restart affected VMs;
6. select/rollback guest boot entry to the frozen migration-time
   generation if newer default entries require store-view-only paths.

Plain `nixling gc` never removes legacy artifacts. Removal requires a
destructive opt-in flag, typed VM-name or explicit acknowledgement, and a
warning before commit that removing any one migrated VM's artifact can make the
whole host rollback-not-ready for migrated VMs. The warning/status enumerates whether the
action transitions the host from rollback-ready to rollback-not-ready for
migrated VMs and which artifact/generation facts drive that decision. Deleting
any migrated VM's artifact degrades host-level rollback.

`nixling status <vm>` reports store mechanism, current generation, last
StoreSync outcome/timestamp/audit reference, legacy artifact existence/frozen
generation/staleness/disk footprint, cleanup status/remediation, and
rollback-readiness. Remediation strings are defined for each cleanup
status/reason pair and for rollback-not-ready/stale-artifact states.

`nixling status` without a VM argument reports host-level rollback readiness:
migrated VMs' legacy-artifact presence/frozen generation plus the retained
legacy-serving host generation or pinned flake input. Native VMs are listed as
non-degrading `native -- no legacy rollback path`. It names the specific
missing migrated-VM artifact, host generation, or pinned input that makes the
host rollback-not-ready.

Remediation taxonomy:

| State | Operator action |
| --- | --- |
| `completed` / `none` | no action |
| `skipped_fast_path` / `fast_path` | no action; reconsider on next non-fast-path sync or global `nixling gc` |
| `deferred_online` / `vm_running` | wait for VM stop or stop it and run global `nixling gc` |
| `deferred_ambiguous` / `running_generation_ambiguous` | stop/restart VM or clear stale serving/open-fd state, then run global `nixling gc` |
| `deferred_metadata` / `missing_retained_metadata` | rerun StoreSync; if persistent, use audit ref plus legacy artifact/rollback readiness before repair |
| `failed` / `io_error` | inspect audit/journal, fix disk/permission/topology error, rerun StoreSync or global `nixling gc` |
| `not_attempted` / `none` on failed attempts | no cleanup ran; fix the reported `error_stage` and rerun StoreSync |
| rollback-not-ready for migrated VM | restore missing legacy artifact or retain/pin a legacy-serving host generation |
| native VM | no legacy artifact exists by design; stop/recreate after host rollback if needed |
| live-pool internal-integrity ok | no action |
| live-pool internal-integrity suspect | run `nixling store verify <vm>`; rerun with `--repair` if verification reports repairable drift and no prior repair attempt is recorded; if repair was already attempted and drift remains, inspect the audit reference and broker logs |
| live-pool internal-integrity unknown | status could not determine integrity because marker/manifest state is missing, unreadable, from an older host generation, or generation identity is unavailable; run `nixling store verify <vm>` |
| stale legacy artifact | preserve by default; remove only through destructive host-wide warning flow |

## Process graph

`store-virtiofs-preflight` is unit-less. The daemon role handler
dispatches broker StoreSync synchronously before polling marker
readiness. `share.source` for ro-store remains `/nix/store` as the
sentinel; only virtiofsd `--shared-dir` becomes `store-view/live`, and
`--readonly` remains asserted.

`nl-meta` source becomes `store-view/meta` and must be read-only at the
virtiofsd/device layer, either by `host.nix` setting `readOnly = true` on the
share or by `processes-json.nix` forcing `--readonly` for the `nl-meta` tag.

`host-activation.nix` may create missing top-level directories and reassert
posture on allowed directory inodes only. It must not recurse into populated
`live/`, posture broker-owned metadata leaves, build closure content, sweep,
or activate generations.

The ownership matrix gains `kind = "dir" | "file"` (default `dir`) and
`required = true | false` (default `true`). Entries with `required = false`
are posture-if-present: only `ENOENT`/not-found is skipped; every other stat
error remains drift/error. File-kind entries use no-follow `symlink_metadata`,
never `mkdir`, and never recurse; if present they must be regular files. Broker
prep creates required `sync.lock`; StoreSync creates optional
`live/.nixling-marker-<vm>` and broker integrity code creates optional
`state/integrity-unknown.json`. Legacy `store`/`store-meta` entries are also
`required = false` so native-born post-cutover VMs without
legacy artifacts pass preflight while migrated VMs' artifacts are still checked
when present.

Host-only entries must not reuse the runner/virtiofsd-readable `users 0755`
store-view posture: `state/`, `state/generations/`, and `gcroots/` are
`nixlingd:nixling 0750`; per-generation host-only metadata leaves
(`state/generations/<id>/{marker.json,meta.json,integrity.json}`) and
`state/integrity-unknown.json` are `nixlingd:nixling 0640`; `sync.lock` is
broker-private `nixlingd:nixling 0600`. File-kind matrix entries re-assert
mode/uid/gid on the file inode with no-follow semantics; they are not just
regular-file kind checks.

The live readiness marker is `nixlingd:users 0644`: guest/runner may read it
through the read-only `live/` share, but only the broker may write it. It is a
single-file check inside `live/` and does not allow recursion into package
trees.

Observability replaces, rather than silently removes, the per-VM store-sync
unit signal. StoreSync terminal broker audit records include `vm` and host-audit
`env`, but the unified broker audit log remains host-confidential and is not
readable by Alloy. Instead, the broker writes a StoreSync-only observability
JSONL export under `/var/lib/nixling/observability/store-sync/` containing only
the StoreSync observability export allow-list defined above. `host.nix` adds a
`loki.source.file` (or Alloy equivalent) on
`/var/lib/nixling/observability/store-sync/store-sync-*.jsonl`, follows daily
rotation, and grants `alloy` focused read/traverse access only to that
StoreSync export directory via access/default ACLs. Do not add `alloy` to the
`nixlingd` group, do not grant it access to `/run/nixling/priv.sock`, and do not
grant it read access to `/var/lib/nixling/audit/broker-*.jsonl`. The stream is
labelled as a host singleton under the Loki contract, for example `vm="host"`,
`env=<host-env>`, `role="host"`, `source="store-sync-audit"`. Target VM/env stay
in JSON content as `target_vm`/`target_env`, not Loki stream/index labels. The source is wired to the
existing Alloy -> OTLP/vsock logs path. The obs-VM stack derives the bounded
alerting metric from the received StoreSync log stream (stage.metrics or
equivalent) and exports it through the obs-VM's existing Alloy self-scrape /
Prometheus ruleFiles path. `05-per-vm-store` uses query-time JSON extraction of
`target_vm`/`target_env`. `NixlingStoreSyncFailure` stays on the existing Prometheus
ruleFiles evaluation/routing path with `vm`/`env` labels only on that
replacement alerting metric. This introduces no new host- or VM-exposed port, no
host Alloy self-scrape, no Loki ruler, no Alertmanager listener, and no
dependency on the deferred broker `/metrics` endpoint.

`nixling store verify <vm> [--repair] [--json]` is the operator-facing deep
verification path for suspected live-pool internal-integrity drift. It is a
typed broker operation (`StoreVerify`) with the same peer-credential
authorization model as StoreSync; the thin CLI never reads `state/` or `live/`
directly. Read-only verify acquires the same per-VM mutex + `sync.lock` as
StoreSync, `nixling gc`, and stop cleanup to take a consistent snapshot and
block concurrent mutation while it walks `live/`. `--repair` delegates lock
acquisition to StoreSync and must not acquire the same lock before calling
StoreSync, avoiding re-entrant mutex/flock deadlock. Without `--repair`, it
verifies every live top-level path for the VM against the trusted bundle closure
and host-authoritative manifest without mutating `live/`. With `--repair`, it
routes through broker StoreSync as a non-fast-path operation, stages repaired
top-level paths, and uses `renameat2(RENAME_EXCHANGE)` for existing served
basenames. Exit codes: `0` clean or repaired successfully; `4` drift found or
integrity still unknown after verification, including `--repair` attempted but
drift remains; `1` daemon unreachable (`#daemon-down`); `2` CLI/usage error;
`70` named VM is not declared in the active manifest or caller is not authorized
to learn whether it exists; `78` broker/system error.
`--json` emits `{ "vm", "status": "ok|drift|unknown|repaired|failed|not_found",
"checked", "drifted", "repaired", "unknown_reason", "audit_ref",
"remediation" }`. Remediation values:
`ok=null`, `drift` without `--repair` = "rerun with --repair to repair
live-pool drift", `drift` without `--repair` when a prior repair attempt is
recorded for the same `(generation_id, drift_signature)` = "repair already
attempted; inspect audit_ref and broker logs", `drift` with `--repair` =
"repair incomplete; inspect audit_ref and broker logs", `unknown` =
reason-specific remediation (`marker_or_manifest_missing`: "run with --repair
or activate a new generation to recreate marker/manifest state",
`marker_or_manifest_unreadable`: "fix permissions or storage errors, then rerun
verify", `older_host_generation`: "activate a current store-view-capable
generation, then rerun verify", `generation_identity_unavailable`: "restore
state/current or activate a new generation, then rerun verify"),
`repaired=null`, `not_found="check the VM name, declaration, and
authorization"`,
`failed="inspect audit_ref and broker logs, then retry"`.
`docs/reference/error-codes.md` gains anchors for `#drift` (exit 4) and
`#not-found` (exit 70), and `docs/reference/cli-contract.md` points all "named
VM is not declared" exit-70 rows, including StoreVerify, at `#not-found` rather
than `#not-yet-implemented`.
`nixling status <vm>` reports live-pool integrity as `ok`, `suspect`, or
`unknown`. `ok` means the stored integrity record is `ok`, the current
marker/manifest pair is valid, and no drift was last observed; a stored `ok`
record never overrides an absent/unreadable readiness marker. `suspect` is set
only when the host can identify a concrete top-level path set: a
present-but-mismatched marker/manifest, a top-level
basename missing from a generation already marked complete, StoreSync
verification failure with implicated paths, repairable drift found by
`nixling store verify`, or incomplete `--repair` with drift remaining. Routine
first-sync/new-generation/incremental materialization of missing top-level paths
before a completion marker is planted is not suspect. The integrity state
records the latest verify/repair audit reference and whether repair has already
been attempted, so status can avoid recommending a repair loop. `unknown` means
the host cannot determine integrity because marker/manifest state is missing,
unreadable, from an older host generation, generation identity itself is
unavailable, or the live readiness marker is absent/unreadable despite a stored
`ok` record. `suspect` and `unknown` remediation points to `nixling store verify
<vm>`; `suspect` recommends `--repair` only if drift is repairable and no prior
repair attempt is recorded.

Integrity state is host-only. Generation-scoped records live under
`state/generations/<generation-id>/integrity.json`. If generation identity is
indeterminate because `state/current` or equivalent manifest state is missing or
unreadable, the broker writes/updates VM-level `state/integrity-unknown.json`
instead. Integrity writes use tmp+rename and fsync the file and parent dir;
status treats parse failure or partial/torn data as `unknown`.

Integrity records store `generation_id` (nullable only for VM-level unknown),
`state`, `drift_signature`, `unknown_reason`, latest verify/repair `audit_ref`,
and `repair_attempted`. `generation_id` is non-null for `ok`, `suspect`, and
generation-scoped `unknown`; it is null only for VM-level `unknown`.
`unknown_reason` is required iff `state=unknown` and is one of
`marker_or_manifest_missing`, `marker_or_manifest_unreadable`,
`older_host_generation`, or `generation_identity_unavailable`. It is forbidden
on `ok` and `suspect`. `generation_identity_unavailable` is valid only for
VM-level unknown (`generation_id=null`); `older_host_generation` is valid only
for generation-scoped unknown (`generation_id != null`).
`marker_or_manifest_missing` and `marker_or_manifest_unreadable` are
generation-scoped when `state/current` resolves to a generation, otherwise the
VM-level reason is `generation_identity_unavailable`. `drift_signature` is
required and non-empty iff `state=suspect`; it is forbidden on `ok` and
`unknown`. They are never written under `meta/`, never exported to the guest,
and never become metric labels.
`suspect` records carry a concrete `drift_signature` naming the implicated
top-level paths. `unknown` records have no concrete drift signature because the
host could not trust marker/manifest state; they are never treated as an empty
signature.

For `suspect`, the record is scoped to `(generation_id, drift_signature)`: it is
cleared when integrity returns to `ok` after successful explicit verify/repair,
or after a non-fast-path StoreSync that re-materializes/replaces and verifies
every top-level path implicated by the recorded `drift_signature`. Pure
fast-path StoreSync never clears `suspect` or `unknown` because it does not
recursively verify package subtrees. A VM-start non-fast-path StoreSync that
only stages missing top-level paths also must not clear an existing `suspect`
record unless those staged paths cover the recorded `drift_signature`.

For `unknown`, only an explicit `nixling store verify` full walk returning
`ok`, a successful `--repair`, or activation of a new StoreSync generation may
clear the state. Explicit verify must resolve a trusted generation identity
before it can return `ok`; otherwise it leaves/sets `unknown`, returns
`status=unknown`/exit `4`, and reports the appropriate `unknown_reason`. Partial
VM-start StoreSync, including a run that stages zero or unrelated missing paths,
must leave `unknown` intact. If `unknown` was stored in VM-level
`state/integrity-unknown.json`, a determinate generation-scoped record takes
read precedence whenever `state/current` resolves successfully; the VM-level
file is authoritative only while generation identity is indeterminate.
Status/broker reconciliation deletes stale VM-level unknown state under the
per-VM mutex + `sync.lock` after revalidating that `state/current` still resolves
to an authoritative generation-scoped record. Resolving a new generation clears
the VM-level file and starts a fresh generation-scoped integrity record.
A new StoreSync generation starts with no prior repair attempt. For the same
unresolved drift on the same generation, the repair-attempt flag remains sticky
so status does not recommend an endless `--repair` loop.

## Validation requirements

- crash/fault injection across pre-activation stage, rename, metadata, integrity, current
  swap, marker, and namespace-helper failure boundaries with recovery
  assertions; for every injected pre-cleanup failure boundary (`lock`, `probe`,
  `verify`, `stage`, `rename`, `metadata`, `integrity`, `current_swap`, `marker`), the
  terminal record has `sync_status=failed`, the matching `error_stage`,
  `cleanup_status=not_attempted`, and `cleanup_reason=none`;
- namespace-helper SIGKILL/error before detach, during detach, and after detach
  proves host `/nix/store` remains mounted, read-only, same `st_dev`,
  content-intact, and with unchanged propagation/peer-group;
- running, cloud-hypervisor-exited-but-virtiofs/open-fd-still-serving,
  ambiguous-running, rollback, and pending retention cases;
- readiness ordering, including stale marker plus failed StoreSync;
- single-writer lock serialization across StoreSync, gc, and stop cleanup;
- deadline-timeout mid-stage holds `sync.lock` until worker quiescence, blocks
  second entrants, records the interrupted stage, and cleans once before unlock;
- helper-backed cancellation closes IPC, escalates SIGTERM -> SIGKILL after
  bounded grace, confirms the helper cgroup reaches `populated=0`, confirms the
  direct helper is reaped via pidfd/waitpid, then cleans once and unlocks;
  static/behavioral validation asserts the helper does not shell out, fork,
  spawn children, or use subprocess copy tools; helper confinement validation
  asserts the capability bounding set, seccomp profile, RLIMIT_NPROC/
  single-process policy, and dedicated cgroup placement are applied;
- authz denial rejects unauthorized or spoofed callers before lock acquisition
  or filesystem side effects, emits `sync_status=failed`,
  `error_stage=authz`, `authz_outcome=deny`, caller principal, and target VM,
  `cleanup_status=not_attempted`, and `cleanup_reason=none`, and contributes
  to terminal authz metrics without path/dump payloads;
- host activation posture touches only allowed directory inodes and never
  recurses into populated `live/`;
- inode mode/uid/gid/ACL preservation after posture repair and
  copy-fallback, including byte-for-byte content equality, mode equality, and
  crash durability after fsync+rename; staged hardlink subtrees are fsynced
  bottom-up and remain internally complete across a crash;
- ACL clearing: seed broad inherited access and default ACLs on store-view and
  subdirs; prep/posture clears both access and default ACL entries on
  `store-view`, `live`, `meta`, `state`, `gcroots`, generation dirs, and stage
  parents, and subsequently created files/markers/gcroot symlinks do not expose
  broad ACL-derived access. ACL operations are directory-inode scoped and
  no-follow; they never follow `gcroots/*` or `state/generations/*/system`
  symlinks into `/nix/store`;
- symlink-no-follow cleanup and metadata symlink no host traversal;
- metadata share read-only at virtiofsd/device layer, guest `meta.json` exact
  allow-list equality, no live/no state/no gcroots/no marker/no caller/authz
  exposure, and zero-length live marker;
- scratch Nix DB load, exact registered set, and corrupt dump failure;
- independently compute the trusted bundle runtime closure and assert the
  enumerated `store-paths` set exactly equals that closure;
- internal completeness: non-fast-path materialization verifies each newly
  staged/replaced top-level path's internal tree against authoritative source
  metadata (file type, mode/executable bit, symlink target, and hardlink inode
  identity or copy content hash as applicable) before current swaps. Pure fast
  path remains O(top-level) and trusts the prior successful marker/manifest;
  deep verification/repair is an explicit non-fast-path operation;
- fast-path complete/incomplete/deferred-cleanup/identity-mismatch cases. If
  explicit verification finds an internally incomplete existing live path, the
  repair is a non-fast-path operation: `fast_path=false`, repaired top-level
  paths count in `linked_count`, untouched paths in `skipped_count`, and the
  replace path uses `renameat2(RENAME_EXCHANGE)` on the same filesystem so the
  served basename is never absent while virtiofsd may be serving it;
- orphan stage and stale current.tmp cleanup;
- current-swap crash recovery: fault between `state/current` and `meta/current`
  swaps; next StoreSync detects divergence, does not fast-path while currents
  disagree, reconciles `meta/current` from valid `state/current`, and converges
  both pointers;
- same-filesystem fatal different-filesystem behavior, same-`st_dev`
  stage-side vfsmount rejection and post-detach source `st_dev`/`st_ino`
  mismatch rejection before partial live changes, and EMLINK fallback induction
  via fault injection or saturated-link fixture;
- guest writable-fd denial and runner write-denial to markers/current/lock;
- runner read-denial to host-only `state/`, `gcroots`, broker markers, and
  `sync.lock`;
- virtiofs negative lookup then addition visibility;
- broker-emitted StoreSync metric label allow-list enforcement with no
  VM/env/generation/caller/store path labels and terminal `sync_status` metric
  values limited to `ok`/`failed`; bounded label values for `cleanup_status`,
  `cleanup_reason`, `error_stage`, and `authz_outcome` stay within the
  documented enums, including `not_attempted` + `none` on
  failed-before-cleanup records; every terminal record's
  `(cleanup_status, cleanup_reason)` pair is a member of the valid pairs table;
  every successful full, incremental, and fast-path sync satisfies
  `linked_count + skipped_count == closure_count`; the separate Alloy-derived
  replacement alerting metric may carry bounded `vm`/`env` labels sourced from
  parsed audit fields and no generation/caller/store-path labels;
- audit/log redaction: no `db.dump` bytes, marker payloads, or store-path
  payloads/basename lists in emitted records;
- observability continuity: StoreSync-only observability file source can be
  read by the `alloy` identity, follows rotated `store-sync-*.jsonl`, and still
  reads a newly created post-rotation file from the real broker export path. The
  unified `/var/lib/nixling/audit/broker-*.jsonl` remains unreadable to `alloy`,
  and no non-StoreSync privileged-operation audit records are ingested or
  egressed. It ingests ok + failed StoreSync terminal records as a host
  singleton Loki stream with `target_vm`/`target_env` in JSON content; the obs-VM stack
  derives the replacement alerting metric from the received log stream using
  `target_vm`/`target_env` and the
  Prometheus rule remains on the existing ruleFiles path, with no new exposed
  listener/route, no host Alloy self-scrape, and no dependency on deferred
  broker `/metrics`; dashboard attribution uses   query-time JSON extraction;
- StoreSync export redaction: `store-sync-*.jsonl` keys equal the StoreSync
  observability export allow-list; `caller_principal`, `retained_generations`,
  host paths, store paths/basenames, `db.dump`, marker payloads, and newly added
  host-only audit fields do not appear;
- Alloy privilege: `alloy` is not a member of `nixlingd`, cannot connect to
  `/run/nixling/priv.sock`, cannot read `/var/lib/nixling/audit/broker-*.jsonl`,
  but can read a newly rotated StoreSync-only export file;
- explicit verify/repair CLI: `nixling store verify <vm> [--repair] [--json]`
  has documented exit codes, JSON envelope, broker routing, and non-fast-path
  repair semantics; read-only verify obtains a consistent broker snapshot,
  detects drift with exit 4/status `drift`, leaves `live/` byte-for-byte and
  inode unchanged while virtiofsd may be serving it, maps daemon/broker failure
  to exit 1 `#daemon-down` or exit 78/status `failed`, maps unknown or
  unauthorized VM to exit 70/status `not_found`, maps incomplete repair to exit
  4/status `drift` with non-looping remediation, and status recommends
  it when live-pool integrity is `suspect` or `unknown`;
- recovery validation: cutover, advance generation, fail, revert to
  legacy-serving version, select frozen guest generation if needed, boot;
- host-level readiness validation: `nixling status` with no VM argument reports
  aggregate rollback readiness for migrated VMs, names missing migrated-VM
  artifacts, missing legacy-serving host generation, or missing pinned input,
  and reports native VMs as non-degrading `native -- no legacy rollback path`;
- fresh native VM validation: absent legacy `store`/`store-meta` artifacts and
  absent first-sync marker do not fail ownership preflight; migrated VMs still
  check legacy artifacts when present;
- integrity-state lifecycle validation: a successful verify/repair or new
  StoreSync generation clears stale repair-attempt state and can recommend
  `--repair` for a distinct new drift; repeated status reads or VM starts for
  the same unresolved `(generation_id, drift_signature)` after an attempted
  repair do not re-recommend `--repair` and instead point to the stored
  `audit_ref` and broker logs. A `suspect`/`unknown` integrity state disables
  pure fast path; a same-generation fast-path StoreSync must not clear it.
  `suspect` may be cleared by explicit verify/repair or non-fast-path StoreSync
  that verifies every concrete `drift_signature`-implicated top-level path.
  `unknown` may be cleared only by explicit full `nixling store verify`,
  successful `--repair`, or a new StoreSync generation. VM-start non-fast-path
  StoreSync that stages zero, unrelated, or subset paths must leave
  suspect/unknown intact;
- integrity-state SET validation: missing, unreadable, or older-generation
  marker/manifest writes or derives `unknown` with no `drift_signature` and does
  not report `ok` or `suspect`; if generation identity is unavailable, the
  VM-level `state/integrity-unknown.json` path is used. Present-but-mismatched
  marker/manifest or missing top-level basenames from a completed generation set
  `suspect` with a non-empty concrete `drift_signature`. Benign first-sync,
  new-generation, or incremental missing-path materialization before completion
  does not set `suspect`. Schema validation rejects `suspect` with missing/empty
  `drift_signature`, rejects `unknown` with any signature, requires valid
  `unknown_reason` iff `state=unknown`, rejects `ok` with `drift_signature` or
  `unknown_reason`, requires non-null `generation_id` for `ok`/`suspect`,
  permits null `generation_id` only for VM-level unknown, rejects
  `generation_identity_unavailable` on generation-scoped records, and permits
  VM-level unknown only with `unknown_reason=generation_identity_unavailable`;
  VM-level records with `marker_or_manifest_missing`,
  `marker_or_manifest_unreadable`, or `older_host_generation` are invalid. A
  VM-level unknown record is
  removed or ignored when generation identity becomes determinate and a
  generation-scoped record is authoritative; deletion rechecks under lock and a
  new StoreSync generation removes VM-level unknown and starts fresh
  generation-scoped integrity state. Missing generation-scoped `integrity.json`
  for a resolved generation is treated as `unknown`, not `ok`;
  a stored generation-scoped `state=ok` record with an absent/unreadable live
  readiness marker is reported as `unknown`, not `ok`;
- successful non-fast-path StoreSync validation: after benign first-sync,
  new-generation, or incremental materialization completes, the generation has a
  generation-scoped `integrity.json` with `state=ok`, non-null `generation_id`,
  no `drift_signature`, and no `unknown_reason` only when there was no prior
  suspect/unknown record or the run satisfied the integrity-clearing rules;
  status reports `ok` and pure fast path is eligible on the next
  same-generation start. If a same-generation incremental run stages only
  unrelated missing paths while suspect/unknown is active, it preserves the
  existing integrity record and status remains suspect/unknown;
- integrity persistence validation: no systemd-tmpfiles or activation cleanup
  rule age-deletes `state/integrity-unknown.json` or
  `state/generations/*/integrity.json`, and no recursive `z`/`Z` tmpfiles rule
  or recursive chmod/chown/ACL activation action targets `store-view/`, `live/`,
  host-only `state/`/`gcroots/`, or any ancestor in a way that descends into
  those paths; stale non-current generation records are removed only when that
  generation is swept;
- source-GC protection validation: during StoreSync, run host GC between
  materialization and final `gcroots/` update and assert source closure paths
  remain protected by existing bundle/profile roots until host-only StoreSync GC
  roots are planted;
- legacy artifact protection: plain `nixling gc` preserves artifacts,
  destructive removal is explicit/acknowledged and warns about host-wide
  rollback loss, and status reports degraded rollback readiness;
- post-activation cleanup failure: inject I/O error during sweep/gcroots after
  current swaps and marker are committed; assert `sync_status=ok`,
  `error_stage=none`, `cleanup_status=failed`, `cleanup_reason=io_error`,
  activation/current/marker preserved, VM start not failed, and over-retained
  paths tolerated;
- status remediation: every cleanup status/reason, rollback-not-ready, and
  stale-artifact state maps to a defined operator action;
- performance gates: at least five warm runs after one cold throwaway run;
  fixed synthetic fixture plus one real heavy VM closure as a hard work-review
  gate; legacy flat-farm and old full generation-tree p95 baselines are captured
  on a named pre-cutover commit/tag or benchmark-only harness before removal and
  recorded for both fixtures; p95 first sync parity-or-better versus legacy flat
  farm; p95 incremental/repeat sync at least 2x faster than both legacy flat
  farm incremental and old full generation-tree build with `linked_count` equal
  to the exact delta;
  same-generation fast path has `linked_count=0`, `swept_count=0`, and p95 <=
  max(500ms, 0.05ms * closure_count); caller-facing VM-start critical path
  times out deterministically at the configured IPC/worker deadline, while lock
  release after quiescence is reported separately.

## Consequences

Positive: avoids full per-generation hardlink trees, preserves stable
served directory live-switch behavior, makes Rust StoreSync authoritative,
and provides auditable performance/cleanup data.

Negative: cleanup may defer and over-retain paths; recovery is host-scoped
and depends on frozen legacy artifacts plus a retained legacy-serving host
generation; validation burden is high.
