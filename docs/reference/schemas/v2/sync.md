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
per-consumer reinterpretation. The adapter opens/acquires the paired storage
row's real lock file (resolved through
`AnchoredResource::resolve_generated`, `packages/d2b-state/src/path.rs`) and
returns a `LockGuard` whose fields are *exactly* derived from the generated
contract, with no invented defaults:

| Generated field | Runtime derivation | Invention avoided |
| --- | --- | --- |
| `id`, `resourceId`, protected resource ids (from `allowedHolders`'s implied scope + the paired storage row) | Encoded via a deterministic, collision-checked charset bridge (`ContractId` → `d2b_contracts::v2_state::ResourceId`); any collision or unrepresentable byte fails closed (`InvalidSchema`). | No array-index-by-id hack; no silent lowercasing that could collide. |
| Total order | `SyncJson::global_order_rank(&lock.id)` — the unique `(scopeClass, anchoredRoot, normalizedPath, lockId)` sort key across every declared lock, converted to a 0-based rank. | No synthetic `global_order` field invented; no fabricated `acquire_after` edges — none exist in the generated contract, so none are processed. |
| `ownerProcess` / `releaseAuthority` | Rendered to `(ActorKind, name)` via `render_authority`, covering only `RealmController`/`RealmBroker` (the only authorities the current generator emits); every other `AuthorityRef` variant fails closed. Symmetry (`ownerProcess == releaseAuthority`) is required and verified, never assumed. | No default/first-holder guess when the two diverge. |
| `timeoutPolicy` | `FailFast`/`NoWait` require `timeoutMs == null` and produce no deadline; `BoundedWait` requires `timeoutMs` in `1..=300000` and produces that exact `Duration`. | No synthetic 1ms deadline; extraneous `timeoutMs` on a fail-fast lock fails closed instead of being silently ignored. |
| Cancellation | Honored unconditionally in the acquire loop (the generated contract carries no distinct cancellation-policy field). | Documented as the strictly-safer default, not a fabricated policy value. |
| `stalePolicy`, `adoptionPolicy`, `degradeScope` | Passed through byte-for-byte onto the returned `LockGuard` (`generated_stale_policy()`, `generated_adoption_policy()`, `generated_degrade_scope()` accessors) for the caller's own reconciliation, never reinterpreted inside the adapter. | No stale/adoption heuristic baked into `d2b-state`. |
| `fdPassingPolicy` | Mapped losslessly by name to `d2b_contracts::v2_state::FdTransferPolicy` (`None → Never`, `ScmRights → ScmRightsLeaseHandoff`, `ExplicitFdMapping → ExplicitFdMapping`); `inheritancePolicy` is cross-checked for the one valid pairing per mechanism. | No default transfer mechanism. |
| `cloexecRequired` | Verified `true` (fails closed otherwise) and independently re-checked via `fcntl(F_GETFD)` on the held fd after open. | Never assumed from the declared policy alone. |
| Storage row (`path`, `mode`, `mode`/owner metadata) | Resolved through the caller's trusted anchor + `AnchoredResource::resolve_generated` (`openat2` `BENEATH|NO_SYMLINKS|NO_MAGICLINKS`), never a separately-probed inode; cross-checked against `pathTemplate`/`scope`/`kind` on the lock spec itself. | No pairing by array position; no trust of a caller-supplied resource id that doesn't match the row the guard actually opened. |

The returned `LockGuard` exposes the held fd's *exact* `(dev, ino)` identity
via `fd_identity()` (a fresh `fstat` of the held fd on every call — immune to
a path being replaced after acquisition) and `verify_binding()`, which
rejects a resource whose id matches but whose containing directory's
`(dev, ino)` has since changed. A resolved `AnchoredResource` cannot be
constructed by pairing an arbitrary trusted root with an arbitrary resource
id: `resolve_generated` walks from the caller's anchor using the generated
absolute path only, and binds the resource to whichever guard subsequently
holds it.

This bridge is additive: existing callers of `LockSet::acquire`/
`acquire_with_clock` (the pre-existing scratch-`LockSpec` API) are
unaffected, and no field was added to `d2b_core::sync::LockSpec` or its
nested types — the generated schema (`sync.json`) is unchanged by this
bridge; only `d2b-state`'s runtime consumption of it is new.
