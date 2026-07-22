# Reference: per-realm secrets lifecycle

> Component: W8 `secrets-lifecycle`. Owns
> `packages/d2b-priv-broker/src/ops/secrets_lifecycle.rs`,
> `packages/d2b-priv-broker/src/ops/secrets_rotation_audit.rs`, and
> `packages/d2b-sk-frontend/src/secrets_channel.rs`. This is the
> storage/atomicity/rotation-state layer only — see "Non-goals" below
> for what it deliberately does not do.
>
> **Status: not wired into any live broker dispatch path or guest
> transport.** Every function this page documents is a tested,
> standalone Rust API reachable only from this component's own
> `#[cfg(test)]` modules today. See
> [Integration wiring points](#integration-wiring-points-not-performed-by-this-component)
> for the exact remaining steps and
> [How to rotate secrets](../how-to/rotate-secrets.md) for what a
> reviewer can verify in the meantime.

This page is the contract for the provision/rotate/rollback/retire
engine that tracks per-realm secrets material: TPM-bound credentials,
guest signing keys, and security-key channel state (the closed
[`SecretKind`] set). It follows the broker's existing fd-relative,
anchored, atomic, identity-bound, fail-closed conventions (the same
family as `swtpm_dir.rs`'s hardening step and `store_sync_audit.rs`'s
typed audit schema), applied generically across all three secret
kinds through one shared engine rather than three separate ones.

This document describes the **second, redesigned** iteration of this
engine (superseding an earlier draft that an external security review
found was not crash-safe). The redesign's driving property, repeated
throughout this page, is: **a process crash must never leave storage
in a state where recovery is forced to either silently activate
something unverified, or return an error while unrecoverable state has
already been activated.**

## Scope and invariants

- **Generic across kinds.** [`SecretKind::TpmBoundCredential`],
  [`SecretKind::GuestSigningKey`], and
  [`SecretKind::SecurityKeyChannelState`] all flow through the same
  `provision`/`rotate`/`rollback`/`retire` functions, parameterized by
  [`SecretsLifecyclePaths`] (derived per `(vm_id, kind)` via
  `derive_paths`). Generating the raw material itself — running
  `ssh-keygen`, deriving a TPM attestation blob, or producing
  channel-binding wire material — stays the caller's concern; this
  engine only owns storage, atomicity, rotation-state tracking, and
  crash recovery.
- **Fd-relative and anchored.** Every filesystem mutation goes through
  `crate::sys::path_safe`'s existing primitives (`ensure_dir`,
  `open_at`, `mkdir_at_exclusive`, `create_file_at_safe`,
  `atomic_replace_fd_with_owner`, `remove_path_safe`, `fstat_fd`,
  `fstatat_nofollow`, `fd_extended_acl_present`) — no bare `std::fs`
  path-string call ever touches a role-writable directory. `state_root`
  itself is treated as an externally-established precondition (opened
  read-only via `open_dir_path_safe`, never `ensure_dir`'d): only
  `state_root/<vm_id>` and `state_root/<vm_id>/<kind-slug>` are created
  by this module, since a bare `ensure_dir` on `state_root` itself would
  fail closed under `path_safe::refuse_world_writable_parent` for any
  `state_root` whose own parent is world-writable.

### Cross-process locking and the durable transaction log

Every mutating action first opens (or creates)
`<kind_root>/lock` and takes an exclusive, blocking-with-timeout
`flock(2)` on it (`acquire_lock`, bounded by
`SecretsLifecycleConfig::lock_max_wait`/`lock_poll_interval`, failing
closed with `FailReason::LockUnavailable` on timeout — this is the
cross-process, per-`(vm, kind)` exclusive lock finding (1) asked for).
Before trusting the lock, `acquire_lock` verifies: the anchored parent
(`kind_root`) is a trusted-broker-owned directory of the expected mode
(`verify_broker_owned`); the lock file itself is a trusted-broker-owned
regular file of the expected mode with exactly one hard link (never a
hard-link plant); and, immediately after `flock` succeeds, that the
`lock` directory entry still resolves — by `(dev, ino)` — to the exact
file this call opened and locked, closing the classic
unlink-and-recreate-between-open-and-flock race. Any mismatch is
`FailReason::BrokerOwnershipViolation`. "Broker-owned" here means
"owned by this process's own effective uid/gid" (`broker_identity`):
metadata directories (`kind_root`, `generations/`) and the lock file
are never `cfg.owner_uid`/`cfg.owner_gid`-owned — that consumer-facing
identity applies only to the `material` file leaf a generation
directory contains.

While holding that lock, every promote-shaped action
(`provision`/`rotate`/`rollback`) and `retire` runs through one of two
shared phase-machine engines:

- **Promote** ([`PromotePhase`]): `Planned` (material staged into a
  fresh `.stage-*` directory, or a rollback's already-existing target
  generation tamper-verified) → `EpochReady` (the target generation is
  materialised at its final `generations/<epoch>` name, fsynced) →
  `CurrentPromoted` (`current` atomically repointed at the new
  generation, fsynced) → `MarkerCommitted` (marker durably rewritten,
  fsynced; superseded generation pruned best-effort; any leftover
  `.stage-*` directory enumerated/validated/deleted with every failure
  propagated) → transaction log removed.
- **Retire** ([`RetirePhase`]): `Enumerated` (the physical
  `generations/` tree anchored-enumerated and validated) →
  `CurrentRemoved` (`current` unlinked, then every recorded generation
  *and* every recorded `.stage-*` directory deleted, each re-validated
  immediately before its own deletion) → `EpochsRemoved` (a fresh
  re-enumeration must observe the tree fully empty) → `ProvenEmpty`
  (marker tombstoned) → transaction log removed.

Before each phase transition, the *current phase* is written to
`<kind_root>/txlog` (JSON, fsynced) — this is the durable
transaction/recovery state machine finding (1) asked for. Every fsync
in this module (`fsync_fd`) returns `Result<(), FailReason::FsyncFailed>`;
a failed sync at any phase transition — material write, staging
directory, epoch rename, `current` swap, marker write, txlog write,
generation/stage deletion — propagates immediately and blocks that
phase from ever being recorded as advanced. No phase transition is
ever considered committed on an unsynced write, so a crash immediately
after a failed sync is always safely re-driveable by a later recovery
pass; nothing is ever left activated with only a *possible* failure to
persist it durably.

Immediately on read, before any recovery attempt acts on it, a leftover
`txlog` undergoes full semantic validation
(`validate_txlog_semantics`/`validate_promote_intent_semantics`/
`validate_retire_intent_semantics`) bound to the exact `(vm_id, kind)`
directory it was found in: the recorded `vm`/`kind` must match the
directory; the action/`create_epoch`/`stage_name`/`expected_identity`
combination must be one this module can legally have written;
epoch/high-water/prune-epoch/carry-previous relationships must be
internally consistent; `RetireIntent.epochs`/`stage_names` must be
sorted, deduplicated, in-bounds, and correctly prefixed. Any violation
is `FailReason::IntentCorrupt` — a hand-edited or otherwise corrupted
txlog is never handed to the recovery engine to interpret.
[`recover_if_needed`] is called at the start of every action (and by
the standalone [`recover_in_flight_transaction`] entry point) and, once
the txlog passes semantic validation:

- if the phase recorded is at or before the point where an external
  observer could see any change (`Planned` for promote; `Enumerated`
  for retire), the leftover state is safely discarded or completed —
  nothing was ever activated, so a fresh re-drive is always safe;
- `Planned`-phase promote recovery never abandons or silently
  re-stages a target epoch that a prior crashed attempt already
  renamed into place: if `generations/<to_epoch>` already exists with
  content whose digest matches the intent's `expected_digest_hex`, the
  engine treats it as already-materialised, **retries the
  `generations_fd` durability barrier** (the prior attempt's own
  trailing fsync may be exactly what crashed it) before advancing, and
  continues forward without needing the original material again; if
  content already occupies that epoch number with a **different**
  digest, recovery fails closed (`RecoveryContentMismatch`) rather than
  ever adopting or overwriting a stale, foreign, or previously-aborted
  generation;
- a `Planned`-phase abort with no leftover material to complete the
  transaction (`PromoteOutcome::AbortedNoMaterial`) never leaves an
  orphan wedge: `discard_partial_stage_if_present` validates and
  deletes any partial `.stage-*` directory the crashed attempt left
  behind (propagating any deletion error) before the txlog is cleared;
  if that partial stage cannot be safely validated (an unrecognised
  on-disk shape), the abort fails closed and **retains** the txlog
  instead, so a future recovery attempt still has the record needed to
  reason about it — the transaction is never silently dropped over
  unrecognised on-disk state;
- if the phase recorded is at or after the commit point
  (`CurrentPromoted`/`MarkerCommitted` for promote,
  `CurrentRemoved`/`EpochsRemoved`/`ProvenEmpty` for retire), recovery
  **only ever moves forward** — it re-verifies the already-activated
  content against what the log recorded and completes the remaining
  steps; it never reverts an already-swapped `current` or resurrects an
  already-deleted generation. A verification mismatch at this stage
  fails closed (`RecoveryContentMismatch`/`RecoveryAmbiguous`) without
  touching anything further — it does not silently re-diverge storage,
  and it does not clear the txlog (so a subsequent recovery attempt,
  e.g. after fixing the anomaly by hand, gets another chance).

`provision`, `rotate`, and `rollback` share the same
[`execute_promote`] codepath (parameterized by a [`PromoteIntent`]) for
both a fresh call and for completing a leftover recovery — there is no
separate "recovery-only" branch that could drift out of sync with the
normal path. `retire` and its own recovery both run through
[`execute_retire`] the same way. Every generation deletion inside
`execute_retire` is preceded by `revalidate_generation_before_delete`:
an immediate, anchored, nofollow re-stat/re-validate of that exact
generation directory right before it is removed, so a tamper injected
between the original enumeration and the delete (not merely between
process restarts) is still caught rather than trusted from a stale
snapshot. `revalidate_generation_before_delete` accepts exactly two
directory shapes: the fully-materialized generation
`enumerate_and_validate_generation_tree` originally validated, or the
one legitimate mid-deletion crash checkpoint `remove_generation` can
leave behind — `material` already unlinked, the epoch directory itself
not yet removed. Recognizing that checkpoint requires every one of: the
entry is still a directory (not a symlink, file, or anything else
swapped into the name); it is still trusted-broker-owned at the
expected `cfg.dir_mode` (a foreign owner or any other mode means the
directory was replaced, not left behind by this module's own deletion
sequence); and it contains **zero** entries other than `.`/`..` (an
unrecognized leftover entry, or a `material` entry of the wrong type or
link-count, still fails closed rather than being swept away). This
acceptance is reachable **only** from `execute_retire`'s
`CurrentRemoved` phase deletion loop, always acting on a `RetireIntent`
that has already passed `validate_retire_intent_semantics` — never from
`enumerate_and_validate_generation_tree`, which fresh pre-retirement
planning (`retire`'s initial enumeration) and the final `EpochsRemoved`
prove-empty check both still call, and which remains exactly as strict
as before: an empty generation directory encountered there is still
`FailReason::RetirementTreeAnomaly`, not a silently-accepted "nothing
here yet" state. A missing marker, or a marker that still claims the
epoch active over that same physically-empty directory, is likewise
never treated as clean storage by the fresh path — it fails closed on
the more specific `PreviouslyProvisionedMaterialMissing` reason raised
while re-verifying the marker's claimed active generation identity,
before enumeration is even reached. Both `execute_promote`'s post-commit
stage cleanup and
`execute_retire`'s `CurrentRemoved` phase likewise call
`revalidate_stage_before_delete` on each `.stage-*` directory
immediately before its own deletion. `CurrentPromoted` and
`MarkerCommitted` recovery/completion each re-verify, before any
further mutation or txlog removal, that `current` still resolves by
exact literal symlink text to the intended target epoch
(`current_resolves_exactly_to`) — and `MarkerCommitted` additionally
re-reads and checks the marker's `active` fields against the intent —
so a phase is never advanced, and the txlog never cleared, over a
`current`/marker state that does not match what that phase's own
identity contract requires. A missing generation or stage directory
encountered while `retire` is retried still retries the
`generations_fd` durability barrier before the retirement phase
advances, so an unsynced prior deletion can never reappear (e.g. via a
crash-induced write-back) after the marker has already been
tombstoned.

Staging directory names use `random_stage_name`: a collision-resistant
name mixing the process id, a per-process atomic monotonic counter,
wall-clock nanoseconds, and a `RandomState`-seeded hash folded through
SHA-256, always exactly `STAGE_PREFIX` (`.stage-`) followed by 32
lowercase-hex characters. `is_well_formed_stage_name` closes the
syntax to exactly that shape — no path separator, no `..`, no
alternate length or alphabet — and every stage name recorded in a
txlog intent is checked against it (`validate_promote_intent_semantics`/
`validate_retire_intent_semantics`) before that intent is ever written
or acted on, in addition to `enumerate_and_validate_generation_tree`'s
own defense-in-depth check over whatever names are physically present
on disk. Staged entries are **real secret-tree contents**, never a
separately-swept, best-effort concern: `enumerate_and_validate_generation_tree`
collects `.stage-*` directories exactly like generation epochs
(`validate_stage_dir_contents` refuses anything but at most one
regular, single-hard-link entry inside), and every deletion path
(`remove_stage_dir`, used by promote's post-commit leftover sweep and
by retire's `CurrentRemoved` phase) fully enumerates, deletes,
positively proves empty, and fsyncs — propagating every failure rather
than ignoring it, including a `fsync` that must run even when the
directory to remove already appears absent (`remove_generation`'s and
`remove_stage_dir`'s own `NotFound` fast paths still retry the
containing directory's durability barrier rather than skipping it, so
a deletion whose fsync had not yet landed cannot silently resurface
after the transaction is considered complete). A stage directory is
included in the final provably-empty proof retirement requires before
tombstoning the marker. The transient `current`-swap staging entry
(`CURRENT_SWAP_STAGE_NAME`) is never removed via the generic
symlink-refusing `remove_path_safe` helper — `atomic_swap_current`
verifies by anchored `nofollow` stat that it is exactly a symlink
before an anchored `unlinkat`, and fails closed
(`FailReason::CurrentSwapFailed`) without deleting anything if it ever
finds something else occupying that name.

### Marker identity (v2)

A dedicated marker file per `(vm, kind)` at `<kind_root>/marker.json`
(root-owned, `0600`, JSON, `MARKER_SCHEMA_VERSION = 2`) records:

- `high_water_epoch`: the monotonic generation high-water mark (see
  below);
- `active`: `None` if retired/never-provisioned, or `Some(`[`MaterialIdentity`]`)` binding the active generation's:
  - `epoch` (the lineage epoch number);
  - `dev`/`ino` of the material file, captured via a `nofollow`-safe fd
    open (never a path re-resolution that could be raced);
  - owner `uid`/`gid`, permission `mode` bits, hard-link count, and
    whether an extended POSIX ACL is present;
  - a SHA-256 content digest.
- `previous`: the immediately-prior retained generation's full identity,
  if any (used by `rollback`). `previous.epoch` need not be numerically
  less than `active.epoch` — a `rollback` swaps the pair, so `previous`
  can legitimately be the *larger* epoch just rolled back away from;
  what always holds is that it is a distinct, real, never-exceeding-
  high-water epoch.

Every public action (`provision`/`rotate`/`rollback`/`retire`) routes
through the single shared `open_and_recover` entry point, which — right
after loading the marker and whenever it records an active generation —
runs one central pre-mutation check, `verify_marker_against_live_state`,
rather than four separately-maintained call sites. This is the one
reachable trigger for `FailReason::IdentityCurrentTargetMismatch`: it
confirms that `current` resolves, by **exact literal symlink text**, to
`generations/<active.epoch>` — a path that merely *ends* in the right
epoch number (an absolute path, a foreign-rooted relative path, or any
text other than that precise relative form) is never accepted as a
match, even when a naive `stat()`-based comparison would otherwise
agree. It then re-derives the active (and, if present, previous)
generation's identity tuple from the **live** filesystem state and
compares it field-by-field against the marker. Each mismatch axis has
its own closed [`FailReason`] variant so a hard-link plant
(`IdentityLinkCountMismatch`, or `IdentityInodeMismatch` when the
digest happens to match but the physical inode does not — the case
byte-content comparison alone cannot see), a digest-preserving
directory swap (`IdentityInodeMismatch`), an ownership drift
(`IdentityOwnerMismatch`), a mode drift (`IdentityModeMismatch`), an
ACL drift (`IdentityAclMismatch`), or a content drift
(`IdentityDigestMismatch`) are independently distinguishable on the
audit surface rather than collapsed into one generic "tampered"
reason.

Beyond that live-state check, every freshly constructed intent is also
run through `validate_promote_intent_semantics`/
`validate_retire_intent_semantics` immediately before it is durably
logged (`write_txlog`) in `provision`/`rotate`/`rollback`/`retire` —
not only when a leftover txlog is re-read during recovery. In
particular, a `rollback`'s intent must satisfy
`expected_digest_hex == expected_identity.digest_hex` before it is
ever acted on: the rollback target's claimed content digest and its
claimed full on-disk identity must agree with each other at
construction time, not merely be independently plausible.

### Fail-closed on drift, not just on error

Mirroring the `swtpm_dir.rs` philosophy:

- `provision` refuses (`AlreadyProvisioned`) if active material
  already exists;
- `provision` fails closed (`PreviouslyProvisionedMaterialMissing`) if
  the marker says active but the generation vanished;
- `provision` fails closed (`GenerationConflict`) if material exists on
  disk with **no** matching active marker — never silently adopted;
- `rotate`/`rollback`/`retire` all fail closed on any
  [`MaterialIdentity`] mismatch axis (above), on any
  `IdentityCurrentTargetMismatch`, and on any internally-inconsistent
  marker (mutually exclusive `retired`/`active`, out-of-bounds epoch)
  via the shared central validation — rather than proceeding against a
  live generation that does not match what the marker recorded.

### Retirement never trusts the marker's word alone

`retire()` always **enumerates and strictly validates the entire
physical `generations/` tree** ([`enumerate_and_validate_generation_tree`])
before looking at what the marker says, regardless of marker state:

- an *active* marker over a tree that anchored-enumeration finds
  already empty is a hard-fail anomaly
  (`PreviouslyProvisionedMaterialMissing`) — a missing marker, or an
  empty tree, never by itself implies "already cleanly retired";
- a tree with any unrecognised entry name, unexpected file type, a
  `material` file whose link count is not exactly `1`, or a `.stage-*`
  directory containing anything other than at most one regular,
  single-hard-link entry aborts the entire retirement with **zero
  deletions** (`FailReason::RetirementTreeAnomaly`);
- otherwise every validated generation *and* every validated stage
  directory is deleted, each re-validated immediately before its own
  deletion (`revalidate_generation_before_delete`) so a tamper injected
  after enumeration is still caught; each containing directory is
  fsynced after its deletions (with any sync failure blocking further
  progress, never silently ignored); the tree is then positively
  re-verified empty (`FailReason::RetirementNotProvablyEmpty` if not)
  before the marker is tombstoned;
- retiring an already-retired or never-provisioned kind (empty tree,
  no active marker) is idempotent and reports `verified_clean`, not an
  error.

### Monotonic generation numbering, survivable across rollback

`MarkerData::high_water_epoch` is a monotonic high-water mark, not
`current_epoch + 1`: `rotate` always allocates
`high_water_epoch + 1`. This means a `rotate` issued **after** a
`rollback` can never collide with (or silently resurrect) a
still-materialised newer epoch that the rollback moved away from —
the newer epoch's generation directory is left physically in place
(pruned only after the new marker is durably committed) but its epoch
number is never reissued. `rollback` itself never grows
`high_water_epoch` and prunes nothing (it just re-points `current` at
the retained `previous`, subject to the same identity-tamper
verification as any other promote). `provision` always allocates epoch
`1` and resets `high_water_epoch` to `1` for a fresh lineage — this is
sound (not a numbering collision risk) specifically because `retire()`
physically empties and proves-empty the entire `generations/` tree
before the marker is tombstoned; the monotonic high-water invariant
only needs to hold *within* one non-retired lineage.

### Redaction

No public function in this module ever returns or logs a raw path, a
raw secret byte, or a raw `io::Error` message. Errors are a closed-set
[`FailReason`] enum plus a fully-formed
[`SecretsLifecycleAuditFields`] record. That record itself never
carries a path (only an FNV1a-64 `base_dir_hash`, parity with
`crate::ops::hosts::stable_hash_str`) or raw material (only a SHA-256
`material_digest_hex` fingerprint, present only for
`provision`/`rotate`). [`SecretMaterial`]:

- derives neither `Copy` nor `Clone`;
- wraps caller-supplied bytes in `zeroize::Zeroizing<Vec<u8>>`
  **before** the length/emptiness validation check runs, so a
  rejected (empty or oversized) buffer is zeroized on drop just like an
  accepted one — there is no code path where a rejected buffer is
  dropped without zeroization;
- has a `Debug` impl that never prints its bytes.

## Guest-side channel state (`secrets_channel.rs`, protocol v2)

The guest-side counterpart (`d2b-sk-frontend::secrets_channel`) is a
standalone, zero-internal-dependency module (only `std`) mirroring the
same lifecycle shape in memory. Its own redesign (superseding the
first draft's single conflated `ChannelGeneration` counter) splits
three previously-conflated concepts:

- [`LineageEpoch`]: the broker-side active generation identity.
  **Rollbackable** — a legitimate `Rollback` action moves it backwards.
  This module enforces no monotonicity on it; the broker/authenticator
  is the authority for whether a given epoch transition is legitimate.
- [`DeliveryCounter`]: a strictly-monotonic anti-replay sequence
  number, checked first (before any other validation) on every
  [`ChannelState::apply`] call, **independent of** and **separate
  from** `LineageEpoch`. Critically, its high-water mark is a property
  of the in-memory [`ChannelState`] object's lifetime, not of the
  rollbackable storage generation: it is **never reset by `Retire` or
  a subsequent `Provision`**, unlike `LineageEpoch` (which legitimately
  restarts at a fresh baseline after a retire, mirroring the broker's
  own storage-generation restart). A stale or repeated
  `DeliveryCounter` is rejected
  (`ChannelStateError::StaleDeliveryCounter`) regardless of which
  [`ChannelAction`] it is attached to, including across a
  retire-then-reprovision boundary.
- [`ChannelAction`]: an explicit four-way discriminator
  (`Provision`/`Rotate`/`Rollback`/`Retire`) carried alongside the
  epoch and counter, rather than inferred from whether the epoch went
  up or down.

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
`SessionConfig::new`) rather than through any implicit
copy/clone. Zeroization here is implemented without the `zeroize`
crate — `d2b-sk-frontend` has no such dependency, and adding one would
require a `Cargo.toml` edit this component does not own — using a
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
  exact security-key channel wire encoding). It only stores, activates,
  and tracks generations of whatever [`SecretMaterial`] bytes a caller
  supplies.
- This component does not implement message authentication or a
  concrete wire byte format for the guest channel. See "Guest-side
  channel state" above.

## Audit record shape

[`SecretsLifecycleAuditFields`] (schema version 2) is a path-free,
material-free JSON-serializable record: `vm_id`, `kind`, `action`
(`provision`/`rotate`/`rollback`/`retire`), `base_dir_hash`, `result`
(`created`/`rotated`/`rolled_back`/`retired`/`verified_clean`/`denied`/
`failed_closed`), `marker_result`
(`created`/`verified`/`tombstoned`/`unchanged`/`failed_closed`), an
optional `lineage_epoch` (the active generation's epoch; never present
for `retired`), an optional `high_water_epoch`, a bounded
`retained_generations` list (nonzero whenever present, at most
`MAX_AUDITED_RETAINED_GENERATIONS`, and always excluding the active
epoch), an optional `material_digest_hex` (present only for
`provision`/`rotate`, a lowercase 64-hex-digit SHA-256 string), a
`recovered_prior_transaction` flag (set whenever the action first had
to drain a leftover crash-interrupted transaction), and an optional
`fail_reason` (present exactly for `denied`/`failed_closed` results,
drawn only from the closed [`FailReason`] enum — never an arbitrary
string or path). `SecretsLifecycleAuditFields::validate` enforces every
cross-field invariant listed above; every constructor
(`provisioned`/`rotated`/`rolled_back`/`retired`/`verified_clean`/
`denied`/`failed`) returns an already-valid record.

## Marker and path layout

```
<state_root>/<vm_id>/<kind-slug>/
  generations/
    <epoch>/material        # 0600, expected_uid:expected_gid
  current -> generations/<epoch>
  marker.json                # 0600 JSON, schema v2
  lock                        # 0600, flock(2) target, never contains data
  txlog                       # 0600 JSON, present only mid-transaction
```

`<kind-slug>` is one of `tpm-bound-credential`, `guest-signing-key`,
`security-key-channel-state` ([`SecretKind::as_slug`]).

## Integration wiring points (not performed by this component)

This component's public functions are ready to call but are not wired
into any shared sink. An integrator still needs to:

1. Add `pub mod secrets_lifecycle;` and
   `pub mod secrets_rotation_audit;` to
   `packages/d2b-priv-broker/src/ops/mod.rs`.
2. Add an
   `OperationFields::SecretsLifecycle(SecretsLifecycleAuditFields)`
   variant (and matching `from_operation_value` arm) to
   `packages/d2b-priv-broker/src/ops/audit_op.rs`, and wire the
   returned record into `crate::audit::AuditLog`.
3. Add new wire request/response DTOs (in `d2b-contracts`) and a
   `runtime.rs` dispatch path calling
   `provision`/`rotate`/`rollback`/`retire`. The plan text for this
   component explicitly forbids adding a new broker op enum family
   from within it, so this step is deliberately deferred.
4. Decide the real `SecretsLifecycleConfig::state_root` source
   (candidate: a subdirectory of the per-realm state root established
   by the ADR 0034 storage-lifecycle contract) — **this path must
   already exist** with a non-world-writable mode before this module
   is called; this module only creates `state_root/<vm_id>/<kind-slug>`
   beneath it, never `state_root` itself — and the real owner
   uid/gid for each kind.
5. Decide whether `SecretKind::GuestSigningKey` material comes from
   `exec_reconcile::run_ssh_keygen` output fed into `rotate`'s
   `material` parameter, or stays separate.
6. Decide the exact coupling between `SecretKind::TpmBoundCredential`
   rotation here and `swtpm_dir.rs`'s physical NVRAM (e.g. whether a
   rotate here should also trigger a swtpm reseal) — a product/security
   decision beyond this component's scope.
7. Call [`recover_in_flight_transaction`] once at controller/broker
   startup for every known `(vm, kind)` pair before dispatching any
   live request, so a leftover transaction from a prior crash is
   drained proactively rather than only on the next incoming request
   for that exact pair.
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
