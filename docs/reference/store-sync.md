# `StoreSync` broker op

This page documents the typed broker op that hardlink-farms a VM's
resolved closure into `/var/lib/nixling/vms/<vm>/store-view/live/`
and atomically swaps the per-VM `store-view/current` symlink to the
freshly built metadata generation.

It is the canonical writer for `store-view`; host activation may publish
next-generation pointers and enforce directory posture, but it must not
build/sweep/activate store-view closures.

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
   so a partially built generation can be reconciled by
   `reconcile_stale_swap_tmp` on the next run.

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
| `generation`           | `u32`               | The generation the daemon expects to activate.     |
| `tracing_span_id`      | `Option<TracingSpanId>` | Audit correlation only.                        |

The daemon never names raw closure paths or generation
directories on the wire — the broker re-derives them from the
trusted bundle via
`BundleResolver::find_store_view_intent(vm_name)`.

Response (`BrokerResponse::StoreSync(StoreSyncResponse)`):

| Field                  | Type     | Notes                                                |
| ---------------------- | -------- | ---------------------------------------------------- |
| `vm`                   | `String` | Echoed VM name from the resolved intent.             |
| `generation`           | `u32`    | Activated generation (matches `current` symlink).    |
| `hardlink_farm_path`   | `String` | Per-VM store-view root (`/var/lib/nixling/vms/<vm>/store-view`). |
| `closure_count`        | `u32`    | Number of top-level closure paths linked in.         |
| `retained_generations` | `Vec<u32>` | Generations retained for cleanup safety.           |
| `swept_count`          | `u32`    | Top-level live entries removed by cleanup.           |
| `cleanup_deferred`     | `bool`   | Whether cleanup was deferred after activation.       |

## Audit fields

The `StoreSync` operation emits an `OperationFields::StoreSync`
record carrying:

- `vm_id`
- `bundle_closure_ref`
- `generation`
- `closure_count`
- `hardlink_farm_path`
- `retained_generations`
- `swept_count`
- `cleanup_deferred`

The `decision` field follows the broker default
(`allowed` / `denied-refused` / `errored`).

## Refusal modes

The handler is fail-closed and maps each refusal to a typed
`StoreSyncError`:

- `BundleIntentMissing { kind: "store-sync-closure" }` — the wire
  `bundle_closure_ref` does not match the bundle-resolved intent
  (or no store-view intent exists for the VM).
- `GenerationMismatch` — the wire `generation` does not match the
  bundle-resolved generation. Generations are monotonic; a stale
  daemon must not race the activator.
- `GenerationOverflow` — bundle resolver carries `u64` generations;
  refuse if the wire's `u32` cannot represent the resolved value
  (would otherwise silently truncate).
- `VmMismatch` — bundle resolver returned an intent keyed at a
  different VM than the wire `vm_id`.
- `HardlinkFarm` — the underlying primitive returned a
  `HardlinkFarmError` (cross-filesystem, marker missing/unparseable,
  I/O failure).

All map onto `BrokerError::LiveHandler(_)` (or
`BrokerError::BundleIntentMissing`) for the wire-level error
envelope.

## Guest-visible generation metadata

Each generation directory carries a guest-safe `meta.json` written by
an **independent allow-list serializer**
([ADR 0027](../adr/0027-store-view-hardlink-live-pool.md)). The
serializer (`GuestGenerationMeta` in `hardlink_farm.rs`) is constructed
from primitives and never receives the full host audit record, so a
field added to the host audit struct cannot leak to the guest. Its key
set is exactly:

- `schema_version`
- `generation_id` (collision-free closure identity; the canonical key)
- `generation_token` (u32 display/wire token; never the on-disk key)
- `sync_status` (only `ok` reaches the guest — `meta.json` is written
  after the generation materialised)
- `closure_count`

It exposes no `live/`, host-only paths, host-absolute symlinks, marker
payloads, caller/authz fields, retained generations, swept counts,
timings, cleanup fields, or error details.

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
  handler (`run_store_sync`) + typed `StoreSyncError`.
- `packages/nixling-host/src/hardlink_farm.rs` — underlying
  same-filesystem-checked atomic-swap primitive; authors the
  zero-length live marker and the guest-safe `meta.json`.
- `packages/nixling-priv-broker/src/runtime.rs` — wire dispatch
  arm (`RealBrokerRequest::StoreSync(req) => …`).
- `packages/nixling-ipc/src/broker_wire.rs` — typed request/
  response structs + enum variants.
- `packages/nixling-ipc/src/types.rs` — `BundleClosureRef`
  opaque newtype.
- `packages/nixling-priv-broker/src/ops/audit_op.rs` —
  `OperationFields::StoreSync` audit shape.

## Migration: deleting the per-VM systemd oneshot

The generated `nixling-<vm>-store-sync.service` unit is the
caller of the bash hardlink-farm script today. Deletion of the
generator is owned by the daemon-only cleanup — `StoreSync` is the
typed replacement op that the per-VM start path will call instead.
