# Reference: per-realm secrets lifecycle

> Component: W8 `secrets-lifecycle`. Owns
> `packages/d2b-priv-broker/src/ops/secrets_lifecycle.rs`,
> `packages/d2b-priv-broker/src/ops/secrets_rotation_audit.rs`, and
> `packages/d2b-sk-frontend/src/secrets_channel.rs`. This is the
> storage-agnostic transaction/rotation-state layer only — see
> "Non-goals" below for what it deliberately does not do.
>
> **Status: not wired into any live broker dispatch path, audit sink,
> or guest transport.** Every function this page documents is a
> tested, standalone Rust API reachable only from this component's own
> `#[cfg(test)]` modules today. See
> [Integration wiring points](#integration-wiring-points-not-performed-by-this-component)
> for the exact remaining steps and
> [How to rotate secrets](../how-to/rotate-secrets.md) for what a
> reviewer can verify in the meantime.

This page is the contract for the provision/rotate/rollback/retire
engine that tracks per-realm secrets material: TPM-bound credentials,
guest signing keys, and security-key channel state (the closed
[`SecretKind`] set). It follows the broker's existing identity-bound,
fail-closed, redaction-safe conventions (the same family as
`swtpm_dir.rs`'s hardening step and `store_sync_audit.rs`'s typed audit
schema), applied generically across all three secret kinds through one
shared engine rather than three separate ones.

## W8fu6: a pure transaction core behind an injected port

This document describes the **third** iteration of this engine.

- Rounds 1-5 (superseded) built a **filesystem-anchored** engine: raw
  fd-relative paths (`crate::sys::path_safe`), a private `F_OFD_SETLK`
  cross-process lock, a durable on-disk JSON marker + transaction log,
  fsync-heavy crash recovery, and an anchored enumerate-then-delete
  retirement walk.
- **This round (W8fu6, current) replaces all of that** with a **pure
  transaction core** that has zero side effects of its own: no
  filesystem access, no locking, no process spawning, no JSON
  serialization of its own state. Every stateful effect goes through
  one injected trait, [`SecretsAuthorityPort`], and every
  crash/fault/concurrency/tamper test in `secrets_lifecycle.rs` runs
  entirely against an in-memory `FakeAuthorityPort` test double defined
  in that file's own `#[cfg(test)] mod fake_port` — there is no test in
  this component that spins up a real directory tree, a real lock, or
  touches a real filesystem.

The redesign's driving property, unchanged from earlier rounds and
still the standard every change below is held to, is: **a crash, a
concurrent writer, or a tampered read must never leave this module
having silently activated something unverified, and must never force
a caller to see an error after unrecoverable state has already been
durably activated.**

### Why a compare-and-swap fencing token instead of a lock + txlog

The prior design's lock (mutual exclusion) and transaction log (crash
recovery) both existed to make a *sequence* of raw filesystem writes
durable and atomic despite crashes and concurrent callers. A storage
substrate that already offers atomic compare-and-swap (etcd's
`mod_revision`, ZooKeeper's `version`, a database row's optimistic-lock
column, or — for a filesystem adapter of the integrator's own design —
a `rename(2)`-based scheme) makes both unnecessary *from this module's
point of view*: every action below reads a [`DurableState`] plus an
opaque [`OwnershipEpoch`] fencing token via
[`SecretsAuthorityPort::read_state`], computes the next state, and
calls [`SecretsAuthorityPort::cas_commit`] with the token it read.
Either the CAS succeeds (this call was the only writer the whole time)
or it is fenced (some other writer's transition is now current; this
call fails cleanly with [`FailReason::OwnershipFenced`] and has mutated
nothing). This module never acquires or holds a lock, and never writes
a transaction log of its own — whatever recovery a concrete adapter
needs for its own commit primitive is the adapter's problem, entirely
hidden behind `cas_commit` returning `Ok`/`Err` atomically.

### What "forward recovery" means without a txlog

There is no separate "recovery mode": a caller that observes
`FailReason::OwnershipFenced` simply re-reads the current state and
retries (or gives up) — there is no crashed, half-applied transaction
to detect or unwind, because `cas_commit` is defined to be atomic
(all-or-nothing) from this module's point of view. "Forward recovery"
instead applies to *pruning*: a transition that supersedes a generation
(`rotate` superseding the old `previous`, `retire` superseding both
`active` and `previous`) commits the superseding [`DurableState`]
**first** and only then attempts to synchronously prune the superseded
material via [`SecretsAuthorityPort::prune_material`]. If that
synchronous prune does not fully succeed, the still-owed epochs are
recorded in the *already-durably-committed* state's
[`DurableState::pending_prune`] list (bounded at [`MAX_PENDING_PRUNE`])
rather than being lost or blocking the caller's success — every
subsequent action for that `(workload, kind)` resolves any outstanding
debt (via the shared `read_and_verify` helper) before doing its own
work, so the debt is self-healing and monotonically shrinks. This is
the module's concrete answer to "never return an error after silently
activating unrecoverable state": the CAS commit that activates a new
generation and the best-effort prune of the old one are two separate
steps, and a failure in the second step is recorded as durable,
retriable debt — never swallowed, and never allowed to turn a
successful activation into a reported failure.

### Why deterministic, high-water-keyed epoch allocation

A new generation's epoch number is always `state.high_water_epoch + 1`
— never "current epoch + 1" — so a `rotate` issued after a `rollback`
can never collide with (or silently resurrect) a still-materialized,
newer-numbered epoch the rollback moved away from. Because the next
epoch number is a pure function of the last *durably committed*
high-water mark, staging is naturally idempotent by epoch: two calls
that stage the same not-yet-committed epoch number simply race on
whose bytes are staged last (closed by the post-commit
re-verification described next), and there is never a need for the
collision-resistant random staging-name scheme rounds 1-5 required.

### Closing the "last stage wins" race

`stage_material` is defined to run *before* `cas_commit`, so two
concurrent callers that both read the same pre-commit state and
compute the same next epoch number can each call `stage_material` for
that epoch before either of them calls `cas_commit`. Only one of the
two `cas_commit` calls can win the race, but the *order* of the two
`stage_material` calls relative to the winning `cas_commit` is not
itself CAS-serialized — the loser's `stage_material` call could still
run (and overwrite the winner's staged bytes) *after* the winner's
`cas_commit` succeeds, silently corrupting the now-durably-active
generation's material without ever going through `cas_commit` again.
[`provision`] and [`rotate`] close this window by **immediately
re-reading the live digest of the epoch they just committed** and
comparing it against the digest they themselves staged: a mismatch
means some other writer's `stage_material` call landed after this
call's `cas_commit`, so this call quarantines the authority and fails
closed with `FailReason::ChecksumMismatch` rather than certifying
success over corrupted, no-longer-trusted material. `rollback` and
`retire` never stage material, so neither is exposed to this race; both
still independently re-verify every generation identity they read
before mutating (via the shared `read_and_verify` helper and, for
`rollback`, an additional call-site re-check immediately before its own
mutation).

### Canonical typed identity, no legacy VM-name string

Every function below takes a
[`d2b_contracts::v2_identity::WorkloadId`] — the same canonical v2
identity type already used elsewhere in this crate
(`guest_session_material.rs`, `child_realm_guest_material.rs`) —
rather than a bare `vm_id: &str` or the legacy
`d2b_contracts::types::VmId` human-label newtype rounds 1-5 used.
`WorkloadId`'s own `parse`/`FromStr` already enforces the canonical
bounded-opaque-string shape, so this module has no `valid_vm_id`-style
runtime string check of its own to maintain — an invalid identity
simply cannot exist as a `WorkloadId` value in the first place. Every
[`SecretsAuthorityPort`] method is scoped by `(&WorkloadId,
SecretKind)`; how (or whether) an adapter maps that pair onto any
underlying storage location — a path, a database key, a KV-store
prefix — is entirely the adapter's concern and never observable here.
The audit surface (`SecretsLifecycleAuditFields::workload_id`) carries
the same typed, opaque `WorkloadId` — never a human-readable label or a
filesystem path.

## Scope and invariants

- **Generic across kinds.** [`SecretKind::TpmBoundCredential`],
  [`SecretKind::GuestSigningKey`], and
  [`SecretKind::SecurityKeyChannelState`] all flow through the same
  `provision`/`rotate`/`rollback`/`retire` functions, parameterized by
  `(workload: &WorkloadId, kind: SecretKind)`. Generating the raw
  material itself — running `ssh-keygen`, deriving a TPM attestation
  blob, or producing channel-binding wire material — stays the
  caller's concern; this engine only owns durable-state transitions,
  CAS fencing, digest re-verification, and forward-recovery pruning.
- **Zero side effects of its own.** No filesystem access, no locking,
  no process spawning. Every mutating effect is a call through the
  injected [`SecretsAuthorityPort`] trait; this module has no opinion
  on, and no dependency on, whatever real storage/locking/CAS
  substrate a concrete adapter chooses.
- **Never silently activates unverified state.** See "W8fu6" above:
  CAS fencing, post-commit digest re-verification, and
  self-consistency validation on every read together mean a caller
  either sees a durably committed, independently re-verified success,
  or a loud, fail-closed error — never a quiet partial mutation.

### Durable state (`DurableState`)

[`DurableState`] is the *entire* payload [`SecretsAuthorityPort::cas_commit`]
ever writes for one `(workload, kind)` pair — there is no separate
marker, transaction log, or lock-state object:

- `high_water_epoch: u64` — monotonic high-water mark: the highest
  epoch number the current, unbroken lineage (since the last
  `provision`) has committed as active. Never decreases within one
  such lineage. `provision` deliberately resets this to `1` for a
  fresh lineage: the shared read helper unconditionally requires any
  `pending_prune` debt from a prior `retire` to be fully drained
  (every previously-live epoch confirmed pruned by the authority)
  before `provision` may run at all, so reusing epoch `1`'s key can
  never resurrect a prior lineage's leftover bytes.
- `active: Option<GenerationRecord>` — the currently active
  generation (`epoch` + SHA-256 hex `digest_hex`), when one exists.
  `None` for a never-provisioned or freshly retired pair.
- `previous: Option<GenerationRecord>` — the most recently superseded
  generation still retained for a possible `rollback`, when one
  exists. Always `None` when `active` is `None`.
- `retired: bool` — `true` exactly for a pair that has been retired
  and not since re-provisioned. A retired pair always has
  `active: None, previous: None`.
- `pending_prune: Vec<Epoch>` — epochs a prior committed transition
  determined are superseded and safe to prune, but whose synchronous
  best-effort prune attempt did not fully succeed. Bounded at
  [`MAX_PENDING_PRUNE`] (`2`); never contains the current `active` or
  `previous` epoch; always strictly ascending and duplicate-free.

`DurableState::validate_self_consistent` checks every one of the above
cross-field invariants (bound, ascending/duplicate-free order,
retired-implies-empty, digest-hex shape, epoch-vs-high-water bounds,
no overlap between `pending_prune` and the live generations) before
this module trusts a value read from the authority. This is
deliberately independent of live digest re-verification (which the
shared read helper performs separately by calling
`SecretsAuthorityPort::material_digest`) — `validate_self_consistent`
only checks that the state's *own* fields are mutually consistent.

### The authority port (`SecretsAuthorityPort`)

The single seam this pure transaction core depends on. An
integrator-owned adapter implements this against whatever real
storage/locking/CAS substrate lands (a filesystem tree with its own
`rename(2)`-based scheme, a KV store with native CAS, a database row
with an optimistic-lock column, etc). Every method is "guarded":
scoped to exactly one `(workload, kind)` pair, taking and returning
only this module's own typed values — never a raw path, file
descriptor, lock handle, or adapter-internal error detail.

| Method | Contract |
| --- | --- |
| `read_state(workload, kind) -> Result<(DurableState, OwnershipEpoch), PortError>` | A pair never committed to returns `DurableState::never_provisioned()` paired with `OwnershipEpoch::NEVER_COMMITTED`. |
| `stage_material(workload, kind, epoch, material) -> Result<String, PortError>` | Durably stores `material`'s bytes as the candidate content for `epoch`, returning the digest the adapter actually stored. May be called more than once for an epoch not yet referenced as `active`/`previous` by the currently committed state; MUST refuse (`PortError::EpochAlreadyCommitted`) a stage call for an epoch already so referenced. |
| `material_digest(workload, kind, epoch) -> Result<String, PortError>` | Re-derives the digest of whatever is currently stored for `epoch`, without ever handing the raw bytes back to this module. |
| `cas_commit(workload, kind, expected, next) -> Result<OwnershipEpoch, PortError>` | Atomically replaces the committed state with `next` iff the adapter's currently stored fencing token equals `expected` — all or nothing. On success returns a new, different token. On a lost race returns `PortError::OwnershipFenced` and mutates nothing. |
| `prune_material(workload, kind, epoch) -> Result<(), PortError>` | Durably discards the material for `epoch`. Idempotent: already absent is `Ok(())`. Never called for an epoch still referenced as `active`/`previous` by the last-known committed state — the superseding transition is always committed first, then pruned. |
| `quarantine(workload, kind, reason) -> Result<(), PortError>` | Durably marks the pair quarantined: every subsequent call of any method above for this pair must fail closed with `PortError::Quarantined` until an out-of-band, integrator-owned clearing operation (this module exposes none) resets it. Idempotent. |

Implementations MUST provide the exact atomicity/idempotency
guarantees documented on each method; this module's correctness (in
particular "never return an error after silently activating
unrecoverable state") depends on `cas_commit` being genuinely atomic
and `prune_material`/`quarantine` being genuinely idempotent.

`PortError` is a closed, four-variant enum (`OwnershipFenced`,
`Quarantined`, `EpochAlreadyCommitted`, `Unavailable`) — this module
never inspects or forwards an adapter's own internal error detail;
every variant is meaningful to *this* module's own algorithm, and
`map_port_error` collapses each onto the audit-facing [`FailReason`]
set (`Unavailable` maps to the single opaque `FailReason::PortUnavailable`
bucket, never an adapter-specific string).

### Secret material (`SecretMaterial`)

Caller-supplied secret bytes. Deliberately **not** `Copy` or `Clone`:
every holder of an owned `SecretMaterial` is a distinct, independently
zeroized buffer. The bytes are wrapped in `zeroize::Zeroizing` *before*
validation, so a rejected (empty or oversized) buffer is still zeroized
on drop rather than discarded as a plain `Vec<u8>`. Capped at
`SecretMaterial::MAX_LEN` (1 MiB) — generous for TPM-bound credential
blobs, guest signing keys, and security-key channel state, while
preventing an unbounded allocation/hash from a misbehaving caller. Its
`Debug` impl reports only a byte length (`finish_non_exhaustive`),
never the material itself.

### Quarantine

Quarantine is a one-way, fail-closed door from this module's point of
view: there is no "un-quarantine" call. [`QuarantineReason`] is a
closed three-variant enum (`ActiveChecksumMismatch`,
`PreviousChecksumMismatch`, `StateSelfInconsistent`) recording exactly
why this module chose to quarantine — never an arbitrary string. See
"Integration wiring points" § 3 for the operator-facing clearing path
this component deliberately does not implement.

### Redaction

No public function, error type, or `Debug` impl in this module ever
exposes secret bytes or a raw path:

- [`SecretsLifecycleError`] carries only a [`FailReason`] and an
  already-redacted [`SecretsLifecycleAuditFields`] value.
- [`SecretMaterial`]'s `Debug` impl reports only a byte length.
- [`SecretsAuthorityPort`] itself never receives or returns a path, fd,
  or lock handle — only `WorkloadId`, `SecretKind`, `Epoch`,
  `DurableState`, digests (hex strings), and this module's own closed
  error/reason enums.

## Guest-side channel state (`secrets_channel.rs`, protocol v2)

The guest-side counterpart (`d2b-sk-frontend::secrets_channel`) is a
standalone, zero-internal-dependency module (only `std`) mirroring the
same lifecycle shape in memory, already aligned with the W8fu6
broker-side model (no functional change was needed this round beyond a
doc-comment update to stop referencing the removed on-disk
`MarkerData::active` field). It splits three independent concepts:

- [`LineageEpoch`]: mirrors the broker-side
  `DurableState::active`'s `GenerationRecord::epoch`. **Rollbackable**
  — a legitimate `Rollback` action moves it backwards. This module
  enforces no monotonicity on it; the broker/authenticator is the
  authority for whether a given epoch transition is legitimate.
- [`DeliveryCounter`]: a strictly-monotonic anti-replay sequence
  number, checked first (before any other validation) on every
  [`ChannelState::apply`] call, **independent of** and **separate
  from** `LineageEpoch`. Critically, its high-water mark is a property
  of the in-memory [`ChannelState`] object's lifetime, not of the
  rollbackable storage generation: it is **never reset by `Retire` or
  a subsequent `Provision`**, unlike `LineageEpoch` (which legitimately
  restarts at a fresh baseline after a retire, mirroring the broker's
  own `DurableState::high_water_epoch` restart). A stale or repeated
  `DeliveryCounter` is rejected (`ChannelStateError::StaleDeliveryCounter`)
  regardless of which [`ChannelAction`] it is attached to, including
  across a retire-then-reprovision boundary.
- [`ChannelAction`]: an explicit four-way discriminator
  (`Provision`/`Rotate`/`Rollback`/`Retire`) carried alongside the
  epoch and counter, rather than inferred from whether the epoch went
  up or down — the same four actions [`LifecycleAction`] models on the
  broker side.

[`ChannelUpdate`] is the only way to apply a transition; its fields are
private and only constructible through named constructors
(`provision`/`rotate`/`rollback`/`retire`) that guarantee the
action/epoch/material combination is always internally consistent (for
example, `retire()` can never accidentally carry material). It is not
`Clone`/`Copy`: an update owns its [`ChannelBinding`] (when present)
and consumes it exactly once via `apply`; on rejection, the update
(and any embedded material) is simply dropped at the end of that call,
which zeroizes it through `ChannelBinding`'s `Drop` impl — there is no
separate "zeroize on rejection" code path to keep in sync.
[`ChannelBinding`] itself derives neither `Copy` nor `Clone`, wraps its
candidate bytes **before** validating them (so the rejection path also
zeroizes on drop), and exposes its raw bytes only through an explicit,
one-shot `expose_bytes()` call for final external handoff (e.g. to
`SessionConfig::new`) rather than through any implicit copy/clone.
Zeroization here is implemented without the `zeroize` crate —
`d2b-sk-frontend` has no such dependency, and adding one would require
a `Cargo.toml` edit this component does not own — using a
`#![forbid(unsafe_code)]`-compatible `std::hint::black_box`-guarded
best-effort overwrite instead of the `zeroize` crate's own
volatile-write internals.

**This module deliberately does not define or parse a wire byte
format.** A prior draft proposed a concrete 40-byte layout
(`from_wire_bytes`); that was exactly the kind of ad-hoc format
finalized without closed validation against the broker's actual
(nonexistent, today) `SecretKind::SecurityKeyChannelState` dispatch
serialization that this redesign was asked to remove. The integrator
must define the real wire schema (likely in `d2b-contracts`, out of
scope here), authenticate incoming messages against it, and translate
the result into a [`ChannelUpdate`] — never the other way around. This
module also does not authenticate messages itself: by the time a
`ChannelUpdate` reaches `apply`, it is assumed to already have been
authenticated (signature/MAC verified, origin checked) by the caller.

## Non-goals

- This component does not add a new broker op enum family, edit
  `runtime.rs`/`lib.rs`, or add a new wire request/response DTO. Those
  are explicit integration follow-ups (see below).
- This component never touches `swtpm_dir.rs`'s physical TPM NVRAM
  state, `security_key.rs`'s CTAPHID transport, or any
  `guest_material_*` file directly. `SecretKind::TpmBoundCredential`
  tracks rotation/retirement bookkeeping layered *atop* swtpm's own
  identity; it is not a replacement for `swtpm_dir.rs`.
- This component does not decide *how* material is generated for any
  kind (ssh-keygen invocation, TPM attestation derivation, or the
  exact security-key channel wire encoding). It only tracks durable
  state transitions and CAS commits for whatever [`SecretMaterial`]
  bytes a caller supplies.
- This component does not implement message authentication or a
  concrete wire byte format for the guest channel. See "Guest-side
  channel state" above.
- This component does not implement a `SecretsAuthorityPort` adapter
  over any real storage substrate. It is a pure library with an
  in-memory fault-injecting test double used only by its own
  `#[cfg(test)]` suite; a real adapter is entirely integrator-owned
  (see "Integration wiring points" § 1).

## Audit record shape

[`SecretsLifecycleAuditFields`] (schema version 3) is a path-free,
material-free JSON-serializable record: `workload_id` (canonical typed
`WorkloadId`, never a human-readable label or path), `kind`, `action`
(`provision`/`rotate`/`rollback`/`retire`), `result`
(`created`/`rotated`/`rolled_back`/`retired`/`verified_clean`/`denied`/
`failed_closed`), `marker_result`
(`created`/`verified`/`tombstoned`/`unchanged`/`failed_closed`), an
optional `lineage_epoch` (the active generation's epoch; never present
for `retired`), an optional `high_water_epoch`, a bounded
`retained_generations` list (nonzero whenever present, at most
`MAX_AUDITED_RETAINED_GENERATIONS`, and always excluding the active
epoch), an optional `material_digest_hex` (present only for
`provision`/`rotate`, a lowercase 64-hex-digit SHA-256 string), a
`prune_deferred` flag (only reachable when `result` is `Rotated` or
`Retired` — set exactly when this action's own successful commit left
at least one superseded generation not yet synchronously pruned, i.e.
recorded in `DurableState::pending_prune` for a future action to
resolve), and an optional `fail_reason` (present exactly for
`denied`/`failed_closed` results, drawn only from the closed
[`FailReason`] enum — never an arbitrary string or path).
`SecretsLifecycleAuditFields::validate` enforces a **complete**
`action` x `result` x `marker_result` compatibility matrix (not just
per-field presence checks) — every combination not actually reachable
from `secrets_lifecycle.rs`'s own call sites is rejected too. Every
constructor (`provisioned`/`rotated`/`rolled_back`/`retired`/
`verified_clean`/`denied`/`failed`) returns an already-valid record;
`failed` in particular hardcodes `MarkerResult::FailedClosed` rather
than accepting a caller-supplied `MarkerResult`, so it can never be
called into producing a schema-invalid combination.

This is a substantially smaller, storage-agnostic set than the rounds
1-5 filesystem-anchored engine's 27-variant `FailReason` enum: every
variant naming a raw path/lock/fsync/txlog/ACL/inode/link-count concept
was a property of that engine's *own* filesystem adapter, never
observable by the W8fu6 pure transaction core, which only ever calls
the six guarded, typed `SecretsAuthorityPort` methods. All of that
fine-grained tamper/I/O detail is now the adapter's own internal
concern, hidden behind `PortError` and reported to this audit surface,
when it must be, only via the single opaque `FailReason::PortUnavailable`
bucket.

## Integration wiring points (not performed by this component)

This module is a pure library with **zero** side effects of its own —
no filesystem access, no locking, no process spawning. A future
integrator must, in follow-up commits **outside this component's
ownership**:

1. Implement a concrete [`SecretsAuthorityPort`] adapter over whatever
   real durable storage/CAS substrate lands for the broker (e.g. the
   ADR 0034 storage/lock contract once it exists, or a dedicated KV
   store). This is now the **single dominant** wiring blocker — rounds
   1-5's many fine-grained filesystem wiring points (lock file
   placement, `dir_mode`/`file_mode`, owner uid/gid, `state_root`) are
   superseded by this one seam.
2. Add exactly one new
   `OperationFields::SecretsLifecycle(SecretsLifecycleAuditFields)`
   variant to `crate::ops::audit_op::OperationFields` (and a matching
   `from_operation_value` arm), and route each
   `Ok`/`Err(SecretsLifecycleError)` returned by [`provision`],
   [`rotate`], [`rollback`], [`retire`] into `crate::audit::AuditLog`
   via that new variant.
3. Add a broker dispatch path (an existing operation-request enum's
   new variant, or a new one, per the integrator's chosen RPC shape)
   that resolves a caller's `(realm, workload label)` into a
   [`d2b_contracts::v2_identity::WorkloadId`] (e.g. via
   `WorkloadId::derive`) and a [`SecretMaterial`] payload, then calls
   the four public functions below against the concrete
   `SecretsAuthorityPort` adapter from (1).
4. Decide and implement whatever real quarantine-clearing operation
   exists for [`QuarantineReason`] — this module deliberately exposes
   no "un-quarantine" call (quarantine is a one-way, fail-closed door
   from this module's own point of view), so an operator-facing clear
   path is an adapter/broker-level concern.
5. For `SecretKind::SecurityKeyChannelState`, wire the four public
   functions to whatever calls into `d2b-sk-frontend`'s
   `secrets_channel.rs` need this lifecycle — see that module's own
   "Integration wiring points" note for its side of the seam (session
   config wiring, wire schema/authentication, broker dispatch mapping,
   delivery-counter persistence).
6. Decide whether `SecretKind::GuestSigningKey` material comes from
   `exec_reconcile::run_ssh_keygen` output fed into `rotate`'s
   `material` parameter, or stays separate.
7. Decide the exact coupling between `SecretKind::TpmBoundCredential`
   rotation here and `swtpm_dir.rs`'s physical NVRAM (e.g. whether a
   rotate here should also trigger a swtpm reseal) — a product/security
   decision beyond this component's scope.
8. Add `pub mod secrets_channel;` to
   `packages/d2b-sk-frontend/src/lib.rs`, and wire
   `services/security_key/mod.rs`'s `SessionConfig` to source its
   `channel_binding`/`reconnect_generation` from
   `ChannelState::with_current` instead of the static
   `D2B_SK_CHANNEL_BINDING_HEX`/`D2B_SK_RECONNECT_GENERATION`
   environment variables it reads today.
9. Define the real wire schema for the message that carries
   `SecretKind::SecurityKeyChannelState` material to the guest (likely
   in `d2b-contracts`), including how it is authenticated, and
   translate it into a [`ChannelUpdate`] — this module has no wire
   format or authentication logic of its own by design (see "Guest-side
   channel state" above).
10. If replay protection must also survive a guest process restart
    (not just a broker-side retire/reprovision), persist
    [`ChannelState::delivery_high_water`] to guest-local storage and
    restore it before the first `apply` call after startup — this
    in-memory module does not do so itself.
11. Run `gen-nix-unit-pins.sh` after this component lands so
    `tests/unit/nix/pinned/*.txt` picks up the new
    `w8-secrets-lifecycle-eval.nix` case names (not run here since the
    pinned files are not owned by this component).
12. Add `pub mod secrets_lifecycle;` and `pub mod secrets_rotation_audit;`
    to `packages/d2b-priv-broker/src/ops/mod.rs` once (1)-(3) above are
    ready to consume them — this component deliberately never adds
    that declaration itself, since doing so without a real adapter or
    dispatch path would compile a component with no reachable caller.

## See also

- [How to rotate secrets](../how-to/rotate-secrets.md)
- [`docs/reference/cgroup-delegation.md`](./cgroup-delegation.md) —
  sibling reference doc style this page follows.
- `packages/d2b-priv-broker/src/ops/swtpm_dir.rs` — the physical TPM
  NVRAM state this component's `TpmBoundCredential` kind layers atop
  without editing.
- `packages/d2b-priv-broker/src/ops/store_sync_audit.rs` — the
  existing standalone-audit-schema precedent this component's
  `secrets_rotation_audit.rs` mirrors.
