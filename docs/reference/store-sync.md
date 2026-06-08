# `StoreSync` broker op

This page documents the typed broker op that hardlink-farms a VM's
resolved closure into `/var/lib/nixling/vms/<vm>/store/` and
atomically swaps the per-VM `current` symlink to the freshly built
generation directory.

It replaces the per-VM `nixling-<vm>-store-sync.service` systemd
oneshot (a bash script invoking `ln(1)` + `mv(1)` directly) with a
broker-side typed handler so the daemon-only end-state can delete the
generated systemd unit.

## Why a broker op (instead of the daemon doing it)

The per-VM hardlink farm shares **inodes** with `/nix/store`. The
`hardlink_farm` primitive enforces:

1. The farm root and `/nix/store` live on the same filesystem
   (otherwise the `link(2)` call would fail at runtime; we check
   eagerly so we can refuse with a typed error).
2. Each generation is built into a side directory
   (`generations/N/`) and only flipped live by `renameat(2)` from
   `current.tmp` → `current`. A crash mid-build leaves the prior
   `current` intact.
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
`/var/lib/nixling/vms/<vm>/store/`. Any recursive mode/owner
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
| `hardlink_farm_path`   | `String` | Per-VM farm root (`/var/lib/nixling/vms/<vm>/store`). |
| `closure_count`        | `u32`    | Number of top-level closure paths linked in.         |

## Audit fields

The `StoreSync` operation emits an `OperationFields::StoreSync`
record carrying:

- `vm_id`
- `bundle_closure_ref`
- `generation`
- `closure_count`
- `hardlink_farm_path`

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

## Implementation file map

- `packages/nixling-priv-broker/src/ops/store_sync.rs` — pure
  handler (`run_store_sync`) + typed `StoreSyncError`.
- `packages/nixling-host/src/hardlink_farm.rs` — underlying
  same-filesystem-checked atomic-swap primitive (unchanged).
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
