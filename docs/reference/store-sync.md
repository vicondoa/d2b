# `StoreSync` broker op

This page documents the typed broker op that hardlink-farms a VM's
resolved closure into the per-VM ADR 0027 **split** store view under
`/var/lib/nixling/vms/<vm>/store-view/` and atomically publishes the new
generation.

It is the **sole canonical writer** for `store-view`; host activation may
publish next-generation pointers and enforce directory posture, but it
must not build/sweep/activate store-view closures. There is no bash
store-view writer and no backend toggle — the Rust broker owns the layout.

## On-disk layout (ADR 0027 split)

`run_store_sync` writes the split layout, keyed by a collision-free
`generation_id` (a SHA-256 over the full ordered closure identity plus the
system store path, `g-<hex>`; **not** the truncated u32 token):

| Path                                              | Trust          | Contents                                                                 |
| ------------------------------------------------- | -------------- | ------------------------------------------------------------------------ |
| `store-view/live/`                                | guest (ro)     | flat hardlink pool of top-level closure basenames; served as `/nix/.ro-store` |
| `store-view/live/.nixling-marker-<vm>`            | guest (ro)     | zero-length cold-start readiness marker, planted **last**                |
| `store-view/meta/current`                         | guest (ro)     | `-> generations/<generation_id>`                                          |
| `store-view/meta/generations/<id>/`               | guest (ro)     | `store-paths`, guest-safe `meta.json`, `db.dump`                          |
| `store-view/state/current`                        | host-only      | `-> generations/<generation_id>`                                         |
| `store-view/state/generations/<id>/`              | host-only      | `system -> /nix/store/...`, `marker.json`, host `meta.json`              |
| `store-view/gcroots/generation-<id>`              | host-only      | symlink to the generation's system store path (GC pin)                   |
| `store-view/sync.lock`                            | host-only      | `flock(2)` exclusion for the op                                          |

The guest `nl-meta` share points at `store-view/meta/` only; `state/`,
`gcroots/`, and `sync.lock` are never exposed to the guest.

Publish ordering is fixed: materialise `live/` + `meta/`/`state/`
generations + the gcroot, copy `db.dump`, then swap `state/current`,
then `meta/current`, then plant the zero-length live marker **last**.
The marker's existence therefore implies a fully-published generation.

A **fast path** short-circuits relinking/republishing when a complete,
consistent same-generation layout already exists (`state/current` and
`meta/current` both resolve to `generation_id`, the host marker matches
the closure + VM, the live marker is present, and every top-level
basename is already in `live/`).

## Why a broker op (instead of the daemon doing it)

The per-VM hardlink farm shares **inodes** with `/nix/store`. The
`hardlink_farm` primitive enforces:

1. The farm root and `/nix/store` live on the same filesystem
   (otherwise the `link(2)` call would fail at runtime; we check
   eagerly so we can refuse with a typed error).
2. Each generation is built into a side directory
   (`live.stage.*`) and only appears in `live/` after an atomic
   rename of each top-level basename. A crash mid-build leaves the
   prior live pool intact.
3. A `marker.json` (typed `GenerationMarker`) is written **last**
   under `state/generations/<id>/` (and, on the legacy path,
   `generations/<u32>/`) so a partially built generation can be
   reconciled on the next run.

The daemon does not have the privileges to satisfy any of these
guarantees — the farm root lives under `/var/lib/nixling/vms/<vm>/`
which is broker-owned and chmod'd `0o700` to root — so the work
must happen in the broker.

## CRITICAL invariant (must read)

The per-VM store-view path **shares inodes** with `/nix/store`.

**NEVER `chown -R`, `chmod -R`, or `setfacl -R`** anywhere under
`/var/lib/nixling/vms/<vm>/store-view/live/`. Any recursive mode/owner
mutation propagates **into `/nix/store` via the shared hardlink
inodes** and immediately breaks ssh's `safe_path()` check on every
host on the network (every `~/.ssh/authorized_keys` lookup walks
the parent chain and refuses on non-root-owned or world-writable
ancestors).

The `StoreSync` handler holds the line by only ever calling:

- `link(2)` (via `std::fs::hard_link`)
- `symlinkat(2)` + `renameat(2)` (via the primitive's atomic
  swap helpers)
- `unlink(2)` for reconciling stale `current.tmp` left by a
  crashed prior run

It does **not** call `chown(2)`, `chmod(2)`, `fsetxattr(2)`, or
`setfacl(8)` anywhere — and the inline unit test
`farm_shares_inodes_with_source_no_recursive_chown` asserts the
source `/nix/store` file's mode/uid/gid is byte-identical before
and after the op.

## Wire shape

Request (`BrokerRequest::StoreSync(StoreSyncRequest)`):

| Field                  | Type                | Notes                                              |
| ---------------------- | ------------------- | -------------------------------------------------- |
| `vm_id`                | `VmId`              | Opaque per-VM scope id.                            |
| `bundle_closure_ref`   | `BundleClosureRef`  | Opaque ref at the `store-view` intent row.         |
| `generation_token`     | `u32`               | Display/wire equality token the daemon expects to activate. Never used as the on-disk key. |
| `tracing_span_id`      | `Option<TracingSpanId>` | Audit correlation only.                        |

The daemon never names raw closure paths or generation
directories on the wire — the broker re-derives them from the
trusted bundle via
`BundleResolver::find_store_view_intent(vm_name)`.

Response (`BrokerResponse::StoreSync(StoreSyncResponse)`):

| Field                  | Type     | Notes                                                |
| ---------------------- | -------- | ---------------------------------------------------- |
| `vm`                   | `String` | Echoed VM name from the resolved intent.             |
| `generation_id`        | `String` | Activated generation **id** — the collision-free on-disk layout key (SHA-256 over the full ordered closure identity, [ADR 0027](../adr/0027-store-view-hardlink-live-pool.md)). |
| `generation_token`     | `u32`    | Activated generation **token** — truncated u32 display/wire value carried for backcompat; never the on-disk key. |
| `hardlink_farm_path`   | `String` | Per-VM store-view root (`/var/lib/nixling/vms/<vm>/store-view`). |
| `closure_count`        | `u32`    | Number of top-level closure paths linked in.         |
| `retained_generations` | `Vec<u32>` | Generations retained for cleanup safety.           |
| `swept_count`          | `u32`    | Top-level live entries removed by cleanup.           |
| `cleanup_deferred`     | `bool`   | Whether cleanup was deferred after activation.       |

## Audit fields

Every StoreSync attempt emits exactly **one terminal structured
broker audit record** (`OperationFields::StoreSync`), serialized from
the allow-list `StoreSyncAuditFields` schema
(`store_sync_audit.rs`, `schema_version = 1`). The schema and its
invariants are signed off in
[ADR 0027](../adr/0027-store-view-hardlink-live-pool.md).

Fields:

| Field                  | Type            | Notes                                                                 |
| ---------------------- | --------------- | --------------------------------------------------------------------- |
| `schema_version`       | `u32`           | Audit schema version (`1`).                                           |
| `vm`                   | `String`        | Resolved VM name.                                                     |
| `vm_id`                | `String`        | Canonical VM id (`store-view:vm:<vm>`).                               |
| `env`                  | `Option<String>`| Host-audit env attribution. Never in guest metadata.                 |
| `generation_id`        | `String`        | Collision-free on-disk layout key. Audit/guest-meta only.            |
| `generation_token`     | `u32`           | Truncated display/wire token. Audit/guest-meta only.                 |
| `sync_status`          | enum            | `ok` \| `failed` \| `in_progress`.                                   |
| `error_stage`          | enum            | `none` \| `authz` \| `lock` \| `probe` \| `verify` \| `stage` \| `rename` \| `metadata` \| `integrity` \| `current_swap` \| `marker`. |
| `cleanup_status`       | enum            | `not_attempted` \| `completed` \| `deferred_online` \| `deferred_ambiguous` \| `deferred_metadata` \| `skipped_fast_path` \| `failed`. |
| `cleanup_reason`       | enum            | `none` \| `vm_running` \| `running_generation_ambiguous` \| `missing_retained_metadata` \| `io_error` \| `fast_path`. |
| `caller_principal`     | `Option<String>`| Audit only — **never** a metric label and never in guest metadata.  |
| `authz_outcome`        | enum            | `allow` \| `deny`.                                                   |
| `closure_count`        | `u32`           | Enumerated top-level closure size.                                   |
| `linked_count`         | `u32`           | Top-level paths newly hardlinked this attempt.                       |
| `skipped_count`        | `u32`           | Top-level paths already present (fast-path / reuse).                 |
| `retained_generations` | `Vec<u32>`      | Audit only — never a metric label and never in guest metadata.      |
| `swept_count`          | `u32`           | Top-level live entries removed by cleanup.                           |
| `fast_path`            | `bool`          | Whether the closure was already fully materialised.                  |
| `timings`              | object          | `total_ms`, `lock_wait_ms`, `lock_hold_ms`, `probe_ms`, `verify_ms`, `stage_ms`, `metadata_ms`, `sweep_ms`, `cleanup_ms`. |

Constructors (`ok_non_fast_path`, `ok_fast_path`,
`ok_cleanup_failed`, `failed`, `denied`) build records that satisfy the
schema invariants; `validate()` enforces them and is asserted in the
dispatch path. The invariants are:

- `sync_status = ok` implies `error_stage = none`.
- A cleanup failure after a successful activation is
  `sync_status = ok` + `error_stage = none` +
  `cleanup_status = failed` + `cleanup_reason = io_error`.
- A failure before cleanup is `sync_status = failed` +
  `cleanup_status = not_attempted` + `cleanup_reason = none` with a
  concrete `error_stage`.
- `error_stage = authz` ⟺ `authz_outcome = deny`.
- For `ok` records (except the post-activation cleanup-failure case),
  `linked_count + skipped_count == closure_count`.
- Pure fast path: `fast_path = true`, `linked_count = 0`,
  `skipped_count = closure_count`, `swept_count = 0`,
  `cleanup_status = skipped_fast_path`.
- Valid `cleanup_status` + `cleanup_reason` pairs are exactly:
  `completed`+`none`, `not_attempted`+`none`,
  `deferred_online`+`vm_running`,
  `deferred_ambiguous`+`running_generation_ambiguous`,
  `deferred_metadata`+`missing_retained_metadata`,
  `skipped_fast_path`+`fast_path`, `failed`+`io_error`.

The record never carries store paths, host paths, marker payloads,
`db.dump` contents, or basenames. `caller_principal` and
`retained_generations` are audit-only and are excluded from any guest
metadata and (per the signed plan) from future metric labels.

The `decision` field follows the broker default
(`allowed` / `denied-refused` / `errored`).

> **Current wiring (W4):** the success path (`ok_fast_path` /
> `ok_non_fast_path`) and the failure path (`failed`) both emit the signed
> terminal record. Every `run_store_sync` attempt that reaches the handler
> emits exactly one terminal `OperationFields::StoreSync` record — success
> with `decision = allowed`, failure with `decision = errored` and a
> classified `error_stage` — and the failure no longer falls back to the
> generic `BrokerError` audit record (`BrokerError::StoreSyncFailed`'s own
> `audit()` is a no-op so the record is never duplicated). The `denied`
> constructor is implemented and unit-tested but **not yet reachable from
> dispatch**: there is no per-VM/per-caller StoreSync authorization policy
> at this layer (the only kernel-trusted identity is the global peer-uid
> gate applied before dispatch), so a real authz-deny trigger awaits that
> policy. `ok_cleanup_failed` likewise awaits the post-activation
> sweep/cleanup wave. `env` attribution and per-phase timings beyond
> `total_ms` are follow-up enrichment.

## Observability export (W5)

The terminal audit record above is **host-confidential** (`0640
root:nixlingd` under `<stateDir>/audit/broker-*.jsonl`) and carries
context the observability plane must never see. Grafana Alloy is **not**
granted read access to that record or to the unified broker audit log.

Instead, every terminal StoreSync attempt *also* emits a narrow,
positive-allow-list projection to a dedicated export file:

```text
<stateDir>/observability/store-sync/store-sync-<utc-date>.jsonl
```

written `0640`, `O_APPEND`, one JSON object per line, daily-rotated by
UTC date. The projection is a dedicated
`StoreSyncObservabilityRecord` struct
(`packages/nixling-priv-broker/src/ops/store_sync_export.rs`) built by
`from_audit_fields()`, which reads **only** the allow-listed fields — so
no serializer ever receives the full audit struct and host-only fields
cannot leak by construction (`#[serde(deny_unknown_fields)]` + an
`EXPORTED_KEYS` key-set test pin the contract).

Exported keys (exactly):

```text
schema_version, target_vm, vm_id, target_env, generation_id,
generation_token, sync_status, error_stage, cleanup_status,
cleanup_reason, authz_outcome, closure_count, linked_count,
skipped_count, swept_count, fast_path,
total_ms, lock_wait_ms, lock_hold_ms, probe_ms, verify_ms,
stage_ms, metadata_ms, sweep_ms, cleanup_ms
```

Notes:

- The audit `vm`/`env` fields are renamed to **`target_vm`/`target_env`**
  so the observability plane treats them as JSON *content*, never as Loki
  stream labels. `target_env` is always serialized (`null` until env
  attribution is threaded) so the key-set stays stable.
- The nested `timings` object is **flattened** to top-level `*_ms` keys.
- Redacted by construction (never exported): `caller_principal`,
  `retained_generations`, `bundle_closure_ref`, `hardlink_farm_path`, the
  nested `timings` object, the raw `vm`/`env` keys, host/store paths and
  basenames, `db.dump` contents, and marker payloads.
- The export is **best-effort**: a failed export write logs a
  `tracing::warn!` but never fails the StoreSync attempt — the
  host-confidential audit record remains the source of truth.

The host Nix/Alloy wiring
(`nixos-modules/components/observability/host.nix`) tails only the
`store-sync-*.jsonl` glob (via `local.file_match` + `loki.source.file`,
following rotation and new files) and grants the `alloy` identity
focused read/traverse ACLs to the export directory **only** — never to
the broker audit log, the privileged daemon socket, or nixlingd state.
The Loki stream labels stay host singletons (`vm="host"`, `env="host"`,
`role="host"`, `source="store-sync-audit"`); `target_vm`/`target_env`
remain in JSON content. See
[`components-observability.md`](./components-observability.md) and
[`loki-label-contract.md`](./loki-label-contract.md).

> **Not in this wave (W5):** there is no log-derived alert *metric*. The
> current obs stack has no `loki.process`/`stage.metrics` log→metric
> path, and the signed scope forbids adding a Loki ruler, Alertmanager,
> host Alloy self-scrape, a broker `/metrics` endpoint, or a new exposed
> port — so the existing systemd-unit-failed `NixlingStoreSyncFailure`
> alert (Prometheus, `stack.nix`) is left unchanged.

## Refusal modes

The handler is fail-closed and maps each refusal to a typed
`StoreSyncError`:

- `BundleIntentMissing { kind: "store-sync-closure" }` — the wire
  `bundle_closure_ref` does not match the bundle-resolved intent
  (or no store-view intent exists for the VM).
- `GenerationMismatch` — the wire `generation_token` does not match the
  bundle-resolved generation. Generations are monotonic; a stale
  daemon must not race the activator.
- `GenerationOverflow` — bundle resolver carries `u64` generations;
  refuse if the wire's `u32` token cannot represent the resolved value
  (would otherwise silently truncate).
- `VmMismatch` — bundle resolver returned an intent keyed at a
  different VM than the wire `vm_id`.
- `HardlinkFarm { stage, source }` — the underlying primitive returned a
  `HardlinkFarmError` (cross-filesystem, marker missing/unparseable,
  I/O failure). `stage` carries the classified `error_stage` of the
  failing publish step (`probe` for topology/cross-filesystem,
  `metadata` for the `db.dump` write, `current_swap` for the
  state/meta `current` swaps, `marker` for the live marker, `lock` for
  the sync-lock, otherwise `stage` for materialisation).

`BundleIntentMissing` is raised in the dispatch arm **before** the
StoreSync handler runs (there is no resolved intent to attribute), so it
keeps its own `BrokerError::BundleIntentMissing` audit path and does not
emit a terminal StoreSync record. Every other variant is raised **inside**
`run_store_sync`: each emits exactly one signed `failed` terminal record
(carrying its classified `error_stage`) and then surfaces as
`BrokerError::StoreSyncFailed { error_stage, message }` for the wire-level
error envelope.

## Guest-visible generation metadata

Each guest generation directory (`store-view/meta/generations/<id>/`)
carries a guest-safe `meta.json` written by an **independent allow-list
serializer** ([ADR 0027](../adr/0027-store-view-hardlink-live-pool.md)).
The serializer (`GuestGenerationMeta` in `hardlink_farm.rs`) is
constructed from primitives and never receives the full host audit
record, so a field added to the host audit struct cannot leak to the
guest. Its key set is exactly:

- `schema_version`
- `generation_id` (collision-free closure identity; the canonical key)
- `generation_token` (u32 display/wire token; never the on-disk key)
- `sync_status` (only `ok` reaches the guest — `meta.json` is written
  after the generation materialised)
- `closure_count`

It exposes no `live/`, host-only paths, host-absolute symlinks, marker
payloads, caller/authz fields, retained generations, swept counts,
timings, cleanup fields, or error details. The host-only counterpart
(`HostGenerationMeta`, written to `state/generations/<id>/meta.json`)
carries `vm`, `linked_count`, and `skipped_count` and is never served to
the guest.

## Live readiness marker

`store-view/live/.nixling-marker-<vm>` is the cold-start readiness
signal, planted last via tmp+rename+fsync after `live/` contains every
required basename. Per ADR 0027 it is a **zero-length** file: its
existence alone is the signal (the readiness probe is a `test -e`), and
because it is served to the guest through the read-only `live/` share it
must carry no payload. The inline test `live_marker_is_zero_length`
asserts `len() == 0`.

## Implementation file map

- `packages/nixling-priv-broker/src/ops/store_sync.rs` — pure
  handler (`run_store_sync`) + typed `StoreSyncError`. Derives the
  `generation_id`, materialises via `build_store_view_cross_mount_safe`,
  and publishes (`state/current`, `meta/current`, live marker).
- `packages/nixling-priv-broker/src/ops/store_view_farm.rs` —
  cross-mount-safe wrappers (`build_store_view_cross_mount_safe`) that
  retry the build in a private mount namespace when `/nix/store` is a
  separate vfsmount.
- `packages/nixling-host/src/hardlink_farm.rs` — underlying
  same-filesystem-checked split-layout primitive (`build_store_view`,
  the `generation_id` derivation, the publish/read helpers); authors the
  zero-length live marker and the guest-safe + host-only `meta.json`.
- `packages/nixling-host/src/bin/nixling-activation-helper.rs` —
  the `build-store-view` verb run inside the private mount namespace.
- `packages/nixling-priv-broker/src/runtime.rs` — wire dispatch
  arm (`RealBrokerRequest::StoreSync(req) => …`).
- `packages/nixling-ipc/src/broker_wire.rs` — typed request/
  response structs + enum variants. The wire carries both the
  collision-free `generation_id` (response) and the u32
  `generation_token` (request + response); the token is display/wire
  only and is never the on-disk key.
- `packages/nixling-ipc/src/types.rs` — `BundleClosureRef`
  opaque newtype.
- `packages/nixling-priv-broker/src/ops/audit_op.rs` —
  `OperationFields::StoreSync` newtype over `StoreSyncAuditFields`.
- `packages/nixling-priv-broker/src/ops/store_sync_audit.rs` —
  the signed `StoreSyncAuditFields` terminal audit schema, its enums,
  invariant-enforcing constructors, and `validate()`.

## Migration: deleting the per-VM systemd oneshot

The generated `nixling-<vm>-store-sync.service` unit is the
caller of the bash hardlink-farm script today. Deletion of the
generator is owned by the daemon-only cleanup — `StoreSync` is the
typed replacement op that the per-VM start path will call instead.
