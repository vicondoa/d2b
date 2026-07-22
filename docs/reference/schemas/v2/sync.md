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
- Every rendered `LockSpec.resourceId` MUST be non-null and MUST reference a
  real generated `storage.json` row — the *protected state* resource this lock
  guards (its schema-intended semantic), never the lock file's own row. The
  lock file's own row is a distinct concept, discovered internally (never by a
  second id field) as the unique regular-file `storage.json` row whose
  `pathTemplate` exactly equals this lock's own `pathTemplate` (e.g. the
  per-realm `keys.lock`/`state.lock`/`audit.lock` rows). A lock with no
  `resourceId` or no `pathTemplate` has no runtime-acquirable identity and
  cannot be driven by the generated-row runtime bridge below.

## Runtime bridge: `d2b_state::LockSet::acquire_from_generated`

`d2b_core::sync::LockSpec` (this schema's Rust type) is consumed directly by
`d2b-state`'s `LockSet::acquire_from_generated` / `acquire_from_generated_with_clock`
(`packages/d2b-state/src/lock.rs`) — a single canonical adapter, not a
per-consumer reinterpretation. The adapter never invents a row: it takes the
*whole* trusted `sync.json` (`&SyncJson`) and `storage.json` (`&StorageJson`)
documents plus one opaque `ContractId` (`lock_id`), looks up the exactly-one
matching lock row itself (`find_unique_lock` — a missing *or* duplicate id
fails closed with `InvalidSchema`), and derives both the protected resource and
the lock file's own row exclusively from that lock's own fields — never from
any caller-supplied resource selection. There is no detached `LockSpec`/
`StoragePathSpec` row parameter for a caller to substitute a same-id-but-
different row into, no caller-supplied protected-resource parameter of any
kind, and no caller-supplied `AnchoredResource` for a caller to pair with the
wrong row:

| Generated field | Runtime derivation | Invention avoided |
| --- | --- | --- |
| `id` | Encoded via a deterministic, collision-checked charset bridge (`ContractId` → `d2b_contracts::v2_state::ResourceId`); any collision or unrepresentable byte fails closed (`InvalidSchema`). | No array-index-by-id hack; no silent lowercasing that could collide. |
| `resourceId` (the *protected state* resource this lock guards) | Required non-null (`InvalidSchema` if absent) and resolved to exactly one row in the trusted `storage.json` via `find_unique_storage_row` (a missing or duplicate id fails closed); the row must share the lock's own `scope`. Stored on the guard as `protected_resources()` (always exactly the one, schema-declared element — never a caller-selected set). | No caller-supplied protected-resource parameter of any kind; no inferred parent/path protection; no silent reuse of the lock file's own row as "the" protected resource. |
| Lock file's own storage row (a distinct concept from `resourceId` — see above) | Resolved internally via `find_unique_lock_file_row`: the unique row in `storage.json` whose `pathTemplate` exactly equals this lock's own (mandatory) `pathTemplate`. That row must be a `RegularFile` sharing the lock's `scope`, must declare an owner (`owner.value`) matching `ownerProcess.value`, and must declare both the `no-symlink` and `no-magic-link` invariants. Stored on the guard as `lock_file_resource_id()`. | No second schema field for the lock-file row; no caller-suppliable row of any kind; a path-template collision (more than one match) or a scope/owner/invariant mismatch fails closed rather than guessing. |
| Total order | `SyncJson::global_order_rank(&lock.id)` — the unique `(scopeClass, anchoredRoot, normalizedPath, lockId)` sort key across every declared lock, converted to a 0-based rank. | No synthetic `global_order` field invented; no fabricated `acquire_after` edges — none exist in the generated contract, so none are processed. |
| `ownerProcess` / `releaseAuthority` | Rendered to `(ActorKind, name)` via `render_authority`, covering only `RealmController`/`RealmBroker` (the only authorities the current generator emits); every other `AuthorityRef` variant fails closed. Symmetry (`ownerProcess == releaseAuthority`) is required and verified, never assumed. | No default/first-holder guess when the two diverge. |
| `timeoutPolicy` | `FailFast`/`NoWait` require `timeoutMs == null`, perform no sleep, and produce no deadline. `BoundedWait` requires `timeoutMs` in `1..=300000`; the acquire loop computes one absolute monotonic deadline up front (`Clock::now() + timeoutMs`) and, on each contended poll, sleeps `min(remaining, MAX_LOCK_POLL_BACKOFF)` via `poll_backoff_or_deadline` — never an invented fixed backoff, and never a sleep that would overshoot the deadline. Before every retry attempt after the first — including immediately after any sleep — the loop re-checks the same absolute deadline (`deadline_elapsed`) and fails closed on a late wakeup rather than issuing one more `set_ofd_lock` attempt past the deadline. | No synthetic 1ms deadline; extraneous `timeoutMs` on a fail-fast lock fails closed instead of being silently ignored; no unconditional max-backoff sleep once less than the cap remains; no acquisition past the deadline due to OS scheduling delay after a sleep returns. |
| Cancellation | Honored unconditionally in the acquire loop (the generated contract carries no distinct cancellation-policy field). | Documented as the strictly-safer default, not a fabricated policy value. |
| `stalePolicy`, `adoptionPolicy`, `degradeScope` | Passed through byte-for-byte onto the returned `LockGuard` (`generated_stale_policy()`, `generated_adoption_policy()`, `generated_degrade_scope()` accessors) for the caller's own reconciliation, never reinterpreted inside the adapter. | No stale/adoption heuristic baked into `d2b-state`. |
| `fdPassingPolicy` | Mapped losslessly by name to `d2b_contracts::v2_state::FdTransferPolicy` (`None → Never`, `ScmRights → ScmRightsLeaseHandoff`, `ExplicitFdMapping → ExplicitFdMapping`); `inheritancePolicy` is cross-checked for the one valid pairing per mechanism. | No default transfer mechanism. |
| `cloexecRequired` | Verified `true` (fails closed otherwise) and independently re-checked via `fcntl(F_GETFD)` on the held fd after open. | Never assumed from the declared policy alone. |
| Lock-file row (`path`, `mode`, owner metadata) | Resolved through the caller's trusted anchor + the crate-private `GeneratedResource::resolve` (`openat2` `BENEATH|NO_SYMLINKS|NO_MAGICLINKS`), never a separately-probed inode; cross-checked against the row's own declared `mode`/owner metadata. The lock file itself is only ever **opened** (`O_RDWR`), never created by the holder: a missing lock file fails closed and the caller must route through broker reconciliation to (re-)create it. Broker-side reconciliation (`d2b-priv-broker`'s `reconcile_storage_scope`) creates a missing generated regular-file lock row anchored/`O_NOFOLLOW`, with `O_CREAT\|O_EXCL\|O_CLOEXEC`, the row's exact declared owner/group/mode, `fsync`s the new file then its parent directory, and on `EEXIST` validates the existing file's type/owner/mode/link-count and `fsync`s the parent before reporting success — never a holder-side create and never an arbitrary-path regular-file creation. | No pairing by array position; no `O_CREAT`/`O_EXCL` in the generated-row acquisition path; no trust of a caller-supplied resource id that doesn't match the row the guard actually opened. |

The returned `LockGuard` exposes the held lock file's *exact* `(dev, ino)`
identity via `fd_identity()` (a fresh `fstat` of the held fd on every call —
immune to a path being replaced after acquisition), its `lock_file_resource_id()`,
and its `protected_resources()` (always exactly one element: the schema's own
`resourceId`, encoded). `validate_state_binding(lock_id, resource_id, owner,
ownership_epoch)` authorizes the *protected* resource — it checks `resource_id`
against `protected_resources` (by construction, always the single
schema-declared protected resource, never the lock file's own id) — so
`AtomicWrite` (`atomic.rs`, unchanged by this bridge) can keep calling it with
the id of the state it is writing, not the id of the lock file guarding that
state.

To actually use the protected resource under the guard's authority, callers
call `LockGuard::protected_resource(resource_id)` — a no-argument (besides the
opaque id it is asserting, purely for a fail-closed identity check), no-storage,
no-anchor, no-metadata borrow of a capability the guard already resolved and
retained *once*, at generated-lock acquisition time (`acquire_from_generated_with_clock`),
from the same trusted `storage.json` + anchor that call was given. There is no
way to hand the guard a new inventory, a new anchor, or a "same id, different
row" substitute after acquisition: the `GeneratedResource` it resolved is
retained on the guard for its entire lifetime and is never re-resolved. Passing
any `resource_id` other than the guard's own single protected resource (for
example, the lock file's own `lock_file_resource_id()`) is rejected with
`LockMismatch`, as is any call after `release`/`release_in_place`
(`LockReleased`).

`protected_resource` returns a `GuardedResource<'guard>` — a non-cloneable,
non-`Debug` capability borrowed from (and lifetime-tied to) `&LockGuard`. It
cannot be constructed, mutated, or cloned by public code; it exposes only
`resource_id()` (the resolved resource's own id, for a fail-closed identity
check — never a caller-supplied override) and `verify_live_identity()` (a
fresh `fstat` of the bound directory, re-checked against the identity captured
once at resolution time, so a resolved binding can never silently authorize
access through a since-replaced directory). The underlying crate-private
`GeneratedResource` capability itself remains fully opaque outside
`d2b-state`: its resource id, directory descriptor, leaf name, and captured
directory identity are private fields with no public constructor, field
mutation, or clone that would let a resolved resource outlive the guard that
produced it.

`d2b-state`'s `AtomicWrite<RealAtomicFilesystem>` exposes two generated/guarded
entry points built on this capability — `write_guarded(guard, resource_id,
payload, policy)` and `read_guarded::<T>(guard, resource_id, policy)` — which
resolve the `GuardedResource`, re-verify its live identity, and perform the
durable write/read, returning only the plain `AtomicWriteReceipt`/
`DurableState<T>` result: the filesystem handle and the `GuardedResource`
itself never escape these calls. The pre-existing `AtomicWrite::write`/`read`
APIs (`atomic.rs`, otherwise unchanged by this bridge) remain available for
callers still driving a plain `AnchoredResource`; the guarded entry points are
the non-forgeable authority seam for callers consuming generated locks.

This bridge is additive: existing callers of `LockSet::acquire`/
`acquire_with_clock` (the pre-existing scratch-`LockSpec` API) are
unaffected, and no field was added to `d2b_core::sync::LockSpec` or its
nested types — the generated schema (`sync.json`) is unchanged by this
bridge; only `d2b-state`'s runtime consumption of it is new. In particular,
`resourceId`'s runtime meaning (the protected state resource, not the lock
file's own row) is this bridge's semantic reading of an existing field, not a
schema change.
