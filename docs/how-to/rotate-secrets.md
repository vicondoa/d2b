# Rotate secrets

**Diataxis category:** how-to.

> Status: the W8 `secrets-lifecycle` component
> (`packages/d2b-priv-broker/src/ops/secrets_lifecycle.rs`,
> `packages/d2b-priv-broker/src/ops/secrets_rotation_audit.rs`,
> `packages/d2b-sk-frontend/src/secrets_channel.rs`) implements a
> **pure transaction core** for provision/rotate/rollback/retire (the
> W8fu6 ports-and-adapters rewrite) plus a replay-safe guest-side
> channel-state holder, but **none of it is wired into a broker op,
> wire DTO, audit sink, storage adapter, or `d2b` CLI verb yet.** There
> is no `d2b secrets rotate`-style command today, and nothing in these
> modules is reachable from a running broker or guest transport — the
> engine has zero side effects of its own and only ever calls the
> caller-injected `SecretsAuthorityPort` trait. This page describes the
> operator-facing procedure once that wiring (and, above all, a real
> `SecretsAuthorityPort` adapter) lands, and what a reviewer can check
> in the meantime. See
> [the reference doc's "Integration wiring points"](../reference/secrets-lifecycle.md#integration-wiring-points-not-performed-by-this-component)
> for the exact remaining steps.

Per-realm secrets material — TPM-bound credentials, guest signing
keys, and security-key channel state — is provisioned once and rotated
periodically or on demand. Every rotation retains exactly one prior
generation so a bad rotation can be rolled back; every action is
fail-closed against a mismatched or corrupted digest; and every action
is either fully applied and durably committed, or has mutated nothing
at all — the engine never leaves a caller unsure whether a rotation
"partly happened" (see the reference doc's "W8fu6: a pure transaction
core behind an injected port" section for how compare-and-swap fencing
replaces the earlier filesystem-anchored lock + crash-recovery design).

## Once wired: rotating a workload's secret material

1. Confirm the target kind is currently provisioned (an unprovisioned
   kind must be provisioned first, not rotated):

   ```bash
   d2b vm secrets status <vm> --kind guest-signing-key
   ```

2. Rotate it:

   ```bash
   d2b vm secrets rotate <vm> --kind guest-signing-key
   ```

   This stages a fresh generation's material through the storage
   adapter, then atomically compare-and-swaps the durable state to make
   it active and retain the immediately-prior generation, fencing off
   any concurrent writer that read the same starting state. The new
   generation's epoch number is always strictly greater than any epoch
   this lineage has ever used (including ones a prior `rollback` moved
   away from), so a rotate can never collide with or resurrect a
   still-materialized generation. Immediately after the commit succeeds,
   the engine re-reads the live digest of exactly the epoch it just
   activated and compares it against what it staged — if some other
   writer's late write landed in between, this is detected and the
   pair is quarantined rather than reporting a false success. A
   `checksum-mismatch` or `quarantined` failure means live storage no
   longer matches the tracked identity — stop and investigate before
   retrying; do not attempt to repair state by hand outside the engine.

3. If the new material is bad, roll back to the retained prior
   generation:

   ```bash
   d2b vm secrets rollback <vm> --kind guest-signing-key
   ```

   This fails with `no-rollback-target` if no prior generation is
   tracked (nothing to roll back to). A rollback legitimately moves the
   active generation's epoch *backwards*; it independently re-verifies
   the rollback target's live digest against the tracked identity
   before committing, just like a rotate does for the generation it
   activates.

4. To retire a kind entirely (e.g. the workload is being
   decommissioned):

   ```bash
   d2b vm secrets retire <vm> --kind guest-signing-key
   ```

   Retiring commits a tombstoned durable state (no active, no
   previous, `retired: true`) first, then attempts to prune the
   now-superseded generations. Retiring is idempotent: retiring an
   already-retired or never-provisioned kind reports `verified_clean`,
   not an error.

Repeat with `--kind tpm-bound-credential` or
`--kind security-key-channel-state` for the other two closed-set
kinds.

## If a prune could not complete synchronously

Committing the tombstoned or rotated state and pruning the superseded
generation's material are two separate steps against the storage
adapter. If the prune does not fully succeed at commit time, the
still-owed epochs are recorded as durable `pending_prune` debt on the
already-committed state — the rotation/retirement itself is **not**
rolled back or reported as failed, and the caller sees the normal
success result with an additional `prune_deferred: true` flag in the
audit record. There is no separate manual "resume pruning" command:
the very next `provision`/`rotate`/`rollback`/`retire` call for that
exact `(workload, kind)` pair automatically re-attempts every
outstanding prune before doing its own work, and will refuse to
proceed with `prune-debt-unresolved` if any of it still cannot be
resolved. This debt is self-healing and monotonically shrinks; it is
never lost and never silently abandoned.

## If a compare-and-swap is fenced

`ownership-fenced` means another writer's transition committed first;
the failed call mutated nothing at all (compare-and-swap is
all-or-nothing). There is no partial state to recover — simply retry
the same action, which will re-read the now-current state and compute
a fresh transition against it.

## Today: verifying the engine directly

Until a real `SecretsAuthorityPort` adapter and the CLI/broker/audit-
sink wiring land, the engine can only be exercised through its Rust
API and inline test suite, entirely against the in-memory
fault-injecting `FakeAuthorityPort` test double defined in
`secrets_lifecycle.rs`'s own `#[cfg(test)] mod fake_port`:

```bash
cd packages
cargo test -p d2b-priv-broker --lib secrets_lifecycle::
cargo test -p d2b-priv-broker --lib secrets_rotation_audit::
cargo test -p d2b-sk-frontend secrets_channel::
```

Every provision/rotate/rollback/retire transition, every fail-closed
denial/quarantine/fencing case, the "last stage wins" race and its
closure via post-commit digest re-verification, forward-recovery
prune-debt draining under repeated and then resolved prune faults, and
the guest-side channel-state replay/tamper guard (including across a
retire-then-reprovision boundary) are covered by inline `#[cfg(test)]`
cases in `secrets_lifecycle.rs` and `secrets_channel.rs`. Reviewers
should not expect these tests to run standalone from a fresh checkout
without a temporary local `pub mod` wiring edit, since neither
`secrets_lifecycle` nor `secrets_rotation_audit` is referenced from
`ops/mod.rs` yet, and `secrets_channel` is not referenced from
`d2b-sk-frontend`'s `lib.rs` yet — see the reference doc's
wiring-points list.
