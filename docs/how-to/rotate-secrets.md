# Rotate secrets

**Diataxis category:** how-to.

> Status: the W8 `secrets-lifecycle` component
> (`packages/d2b-priv-broker/src/ops/secrets_lifecycle.rs`,
> `packages/d2b-priv-broker/src/ops/secrets_rotation_audit.rs`,
> `packages/d2b-sk-frontend/src/secrets_channel.rs`) implements a
> crash-safe provision/rotate/rollback/retire engine plus a replay-safe
> guest-side channel-state holder, but **none of it is wired into a
> broker op, wire DTO, audit sink, or `d2b` CLI verb yet.** There is no
> `d2b secrets rotate`-style command today, and nothing in these
> modules is reachable from a running broker or guest transport. This
> page describes the operator-facing procedure once that wiring lands,
> and what a reviewer can check in the meantime. See
> [the reference doc's "Integration wiring points"](../reference/secrets-lifecycle.md#integration-wiring-points-not-performed-by-this-component)
> for the exact remaining steps.

Per-realm secrets material — TPM-bound credentials, guest signing
keys, and security-key channel state — is provisioned once and
rotated periodically or on demand. Every rotation retains exactly one
prior generation so a bad rotation can be rolled back, every action is
fail-closed against a tampered or missing identity marker, and every
action is safe to resume after a broker crash mid-transaction (see the
reference doc's "Cross-process locking and the durable transaction
log" section).

## Once wired: rotating a VM's secret material

1. Confirm the target kind is currently provisioned (an unprovisioned
   kind must be provisioned first, not rotated):

   ```bash
   d2b vm secrets status <vm> --kind guest-signing-key
   ```

2. Rotate it:

   ```bash
   d2b vm secrets rotate <vm> --kind guest-signing-key
   ```

   This stages and fsyncs a fresh generation, atomically swaps
   `current` to it, durably commits the marker, retains the
   immediately-prior generation, and prunes anything older only after
   the new marker is committed. The new generation's epoch number is
   always strictly greater than any epoch this lineage has ever used
   (including ones a prior `rollback` moved away from), so a rotate
   can never collide with or resurrect a still-materialised generation.
   A `marker-tampered-or-missing-material`-shaped failure (any of the
   `identity-*-mismatch` reasons) means the on-disk identity no longer
   matches the tracked marker — stop and investigate before retrying;
   do not delete state by hand.

3. If the new material is bad, roll back to the retained prior
   generation:

   ```bash
   d2b vm secrets rollback <vm> --kind guest-signing-key
   ```

   This fails with `no-rollback-target` if no prior generation is
   tracked (nothing to roll back to). A rollback legitimately moves the
   active generation's epoch *backwards*; it is still subject to the
   same identity-tamper verification as a rotate.

4. To retire a kind entirely (e.g. the VM is being decommissioned):

   ```bash
   d2b vm secrets retire <vm> --kind guest-signing-key
   ```

   Retiring always enumerates and validates the entire on-disk
   generation tree first, regardless of what the marker says — a
   missing marker is never treated as proof the tree is already clean.
   Retiring is idempotent: retiring an already-retired or
   never-provisioned kind reports `verified_clean`, not an error. Any
   entry retirement cannot account for (unexpected name, unexpected
   type, or a hard-linked material file) aborts the whole retirement
   with zero deletions (`retirement-tree-anomaly`) rather than deleting
   what it can and leaving the rest.

Repeat with `--kind tpm-bound-credential` or
`--kind security-key-channel-state` for the other two closed-set
kinds.

## If the broker crashed mid-rotation

Once wired, the integrator is expected to call
`recover_in_flight_transaction` for every known `(vm, kind)` pair at
controller/broker startup (see the reference doc's wiring point §7),
so a leftover transaction from a crash mid-rotate/rollback/retire is
drained automatically before any new request is dispatched. If that
startup sweep is not yet wired, the same recovery runs automatically
as the first step of the *next* `rotate`/`rollback`/`retire`/`provision`
call for that exact `(vm, kind)` pair — there is no separate manual
"resume" command. A recovery that finds the leftover transaction
already past its commit point only ever completes it forward (finishes
the marker write, pruning, or tombstoning); it never reverts an
already-activated `current` swap or resurrects an already-deleted
generation. A recovery that cannot verify the leftover state against
what was logged before the crash fails closed
(`recovery-content-mismatch`/`recovery-ambiguous`) without touching
anything further — this needs operator/integrator investigation, not a
retry.

## Today: verifying the engine directly

Until the CLI/broker/audit-sink wiring lands, the engine can only be
exercised through its Rust API and inline test suite:

```bash
cd packages
cargo test -p d2b-priv-broker --lib secrets_lifecycle::
cargo test -p d2b-priv-broker --lib secrets_rotation_audit::
cargo test -p d2b-sk-frontend secrets_channel::
```

Every provision/rotate/rollback/retire transition, every fail-closed
tamper/drift/anomaly case, crash recovery at each phase of both the
promote and retire state machines, cross-process lock contention, and
the guest-side channel-state replay/tamper guard (including across a
retire-then-reprovision boundary) are covered by inline `#[cfg(test)]`
cases in `secrets_lifecycle.rs` and `secrets_channel.rs`. Reviewers
should not expect these tests to run standalone from a fresh checkout
without a temporary local `pub mod` wiring edit, since neither module
is referenced from `ops/mod.rs` or `lib.rs` yet — see the reference
doc's wiring-points list.
