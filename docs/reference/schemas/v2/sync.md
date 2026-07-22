# `sync.json` schema (`v2`)

Schema: [`sync.json`](./sync.json)

`sync.json` is the private synchronization and lock contract selected by
[ADR 0034](../../adr/0034-storage-lifecycle-restart-and-synchronization.md).
It declares framework locks, holders, fd inheritance policy, fd transfer
mechanism, acquisition order, stale-owner policy, adoption behavior, and
degraded-state handling.

## Top-level fields

- `schemaVersion` — schema version for this artifact.
- `locks` — lock specs keyed by stable lock id.

## Contract notes

- New advisory file locks use Linux OFD locks (`F_OFD_SETLK`).
- Lock fds are opened with `O_CLOEXEC` and are not inherited by runner
  payloads unless the sync spec explicitly allows `SCM_RIGHTS` or explicit
  fd mapping plus a lease transfer record.
- Multi-lock operations acquire locks through the declared total order.
- Ambiguous owner state degrades/quarantines the protected scope rather than
  force-unlocking behind a possible live owner.
- Every host-mutable lock surface has one repair owner. New locks must be
  represented by a generated sync row and reconciled through that owner instead
  of through side-channel lock files or cleanup scripts.
- Every rendered `LockSpec.resourceId` MUST reference a real generated
  `storage.json` row (a regular-file lock resource, e.g. the per-realm
  `keys.lock`/`state.lock`/`audit.lock` rows). A lock with no `resourceId` has
  no runtime-acquirable identity and cannot be driven by the generated-row
  runtime bridge below.

## Runtime bridge: `d2b_state::LockSet::acquire_from_generated`

`d2b_core::sync::LockSpec` (this schema's Rust type) is consumed directly by
`d2b-state`'s `LockSet::acquire_from_generated` / `acquire_from_generated_with_clock`
(`packages/d2b-state/src/lock.rs`) — a single canonical adapter, not a
per-consumer reinterpretation. The adapter never invents a row: it takes the
*whole* trusted `sync.json` (`&SyncJson`) and `storage.json` (`&StorageJson`)
documents plus opaque `ContractId`s (`lock_id`, and the caller's requested
`protected_resource_ids`), looks up the exactly-one matching lock/storage rows
itself (`find_unique_lock`/`find_unique_storage_row` — a missing *or*
duplicate id fails closed with `InvalidSchema`), and only then resolves the
generated resource. There is no detached `LockSpec`/`StoragePathSpec` row
parameter for a caller to substitute a same-id-but-different row into, and no
caller-supplied `AnchoredResource` for a caller to pair with the wrong row:

| Generated field | Runtime derivation | Invention avoided |
| --- | --- | --- |
| `id`, `resourceId` (the lock file's own paired storage row, looked up internally — never caller-supplied) | Encoded via a deterministic, collision-checked charset bridge (`ContractId` → `d2b_contracts::v2_state::ResourceId`); any collision or unrepresentable byte fails closed (`InvalidSchema`). | No array-index-by-id hack; no silent lowercasing that could collide; no caller-substitutable row. |
| Protected resource ids | Caller-supplied `&[ContractId]`, deduplicated and each resolved to exactly one `RegularFile` row in the trusted `storage.json`; distinct from the lock file's own `resourceId` (a lock protects state, it is not itself the protected state). Stored on the guard as `protected_resources()`. | No inferred parent/path protection; no silent reuse of the lock file's own id as "the" protected resource. |
| Total order | `SyncJson::global_order_rank(&lock.id)` — the unique `(scopeClass, anchoredRoot, normalizedPath, lockId)` sort key across every declared lock, converted to a 0-based rank. | No synthetic `global_order` field invented; no fabricated `acquire_after` edges — none exist in the generated contract, so none are processed. |
| `ownerProcess` / `releaseAuthority` | Rendered to `(ActorKind, name)` via `render_authority`, covering only `RealmController`/`RealmBroker` (the only authorities the current generator emits); every other `AuthorityRef` variant fails closed. Symmetry (`ownerProcess == releaseAuthority`) is required and verified, never assumed. | No default/first-holder guess when the two diverge. |
| `timeoutPolicy` | `FailFast`/`NoWait` require `timeoutMs == null`, perform no sleep, and produce no deadline. `BoundedWait` requires `timeoutMs` in `1..=300000`; the acquire loop computes one absolute monotonic deadline up front (`Clock::now() + timeoutMs`) and, on each contended poll, sleeps `min(remaining, MAX_LOCK_POLL_BACKOFF)` via `poll_backoff_or_deadline` — never an invented fixed backoff, and never a sleep that would overshoot the deadline. | No synthetic 1ms deadline; extraneous `timeoutMs` on a fail-fast lock fails closed instead of being silently ignored; no unconditional max-backoff sleep once less than the cap remains. |
| Cancellation | Honored unconditionally in the acquire loop (the generated contract carries no distinct cancellation-policy field). | Documented as the strictly-safer default, not a fabricated policy value. |
| `stalePolicy`, `adoptionPolicy`, `degradeScope` | Passed through byte-for-byte onto the returned `LockGuard` (`generated_stale_policy()`, `generated_adoption_policy()`, `generated_degrade_scope()` accessors) for the caller's own reconciliation, never reinterpreted inside the adapter. | No stale/adoption heuristic baked into `d2b-state`. |
| `fdPassingPolicy` | Mapped losslessly by name to `d2b_contracts::v2_state::FdTransferPolicy` (`None → Never`, `ScmRights → ScmRightsLeaseHandoff`, `ExplicitFdMapping → ExplicitFdMapping`); `inheritancePolicy` is cross-checked for the one valid pairing per mechanism. | No default transfer mechanism. |
| `cloexecRequired` | Verified `true` (fails closed otherwise) and independently re-checked via `fcntl(F_GETFD)` on the held fd after open. | Never assumed from the declared policy alone. |
| Storage row (`path`, `mode`, owner metadata) | Resolved through the caller's trusted anchor + the crate-private `GeneratedResource::resolve` (`openat2` `BENEATH|NO_SYMLINKS|NO_MAGICLINKS`), never a separately-probed inode; cross-checked against the row's own declared `mode`/owner metadata. The lock file itself is only ever **opened** (`O_RDWR`), never created: a missing lock file fails closed and the caller must route through broker reconciliation to (re-)create it, instead of the generated adapter silently materializing broker-owned state. | No pairing by array position; no `O_CREAT`/`O_EXCL` in the generated-row acquisition path; no trust of a caller-supplied resource id that doesn't match the row the guard actually opened. |

The returned `LockGuard` exposes the held lock file's *exact* `(dev, ino)`
identity via `fd_identity()` (a fresh `fstat` of the held fd on every call —
immune to a path being replaced after acquisition), its `lock_file_resource_id()`,
and its `protected_resources()`. `validate_state_binding(lock_id, resource_id,
owner, ownership_epoch)` authorizes a *protected* resource — it checks
`resource_id` for **membership** in `protected_resources`, not equality
against the lock file's own id — so `AtomicWrite` (`atomic.rs`, unchanged by
this bridge) can keep calling it with the id of the state it is writing, not
the id of the lock file guarding that state.

To actually open the protected resource under the guard's authority, callers
use `LockGuard::bind_protected_resource(storage, resource_id, anchor,
anchor_path, metadata)`: it re-validates the row against the trusted
`storage.json`, requires `resource_id` to be a member of this exact guard's
`protected_resources`, and only then resolves a fresh, non-forgeable
`GeneratedResource` the same way the lock file itself was resolved — a
resolved resource is never a clone of anything the caller supplied, and a
guard that has been released (`release`/`release_in_place`) rejects every
subsequent bind with `LockReleased`. The crate-private `GeneratedResource`
capability itself is fully opaque outside `d2b-state`: its resource id,
directory descriptor, leaf name, and re-derived directory identity are
private fields with no public constructor, field mutation, or clone that
would let a resolved resource outlive the guard/anchor that produced it;
public code only ever receives the legacy `AnchoredResource` shape via
`into_anchored_resource()`.

This bridge is additive: existing callers of `LockSet::acquire`/
`acquire_with_clock` (the pre-existing scratch-`LockSpec` API) are
unaffected, and no field was added to `d2b_core::sync::LockSpec` or its
nested types — the generated schema (`sync.json`) is unchanged by this
bridge; only `d2b-state`'s runtime consumption of it is new.
