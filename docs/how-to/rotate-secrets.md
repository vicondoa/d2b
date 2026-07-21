# Rotate secrets

**Diataxis category:** how-to.

> Status: the W8 `secrets-lifecycle` component
> (`packages/d2b-priv-broker/src/ops/secrets_lifecycle.rs`) implements
> the provision/rotate/rollback/retire engine, but it is **not yet
> wired** into a broker op, wire DTO, or `d2b` CLI verb. There is no
> `d2b secrets rotate`-style command today. This page describes the
> operator-facing procedure once that wiring lands, and what an
> integrator or reviewer can check in the meantime. See
> [the reference doc's "Integration wiring points"](../reference/secrets-lifecycle.md#integration-wiring-points-not-performed-by-this-component)
> for the exact remaining steps.

Per-realm secrets material — TPM-bound credentials, guest signing
keys, and security-key channel state — is provisioned once and
rotated periodically or on demand. Every rotation retains exactly one
prior generation so a bad rotation can be rolled back, and every
action is fail-closed against a tampered or missing identity marker.

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

   This creates a fresh generation, atomically swaps `current` to it,
   retains the immediately-prior generation, and prunes anything
   older. A `stale-marker`/`marker-tampered-or-missing-material`
   failure means the on-disk identity no longer matches the tracked
   marker — stop and investigate before retrying; do not delete state
   by hand.

3. If the new material is bad, roll back to the retained prior
   generation:

   ```bash
   d2b vm secrets rollback <vm> --kind guest-signing-key
   ```

   This fails with `no-rollback-target` if no prior generation is
   tracked (nothing to roll back to).

4. To retire a kind entirely (e.g. the VM is being decommissioned):

   ```bash
   d2b vm secrets retire <vm> --kind guest-signing-key
   ```

   Retiring is idempotent: retiring an already-retired or
   never-provisioned kind reports `verified_clean`, not an error.

Repeat with `--kind tpm-bound-credential` or
`--kind security-key-channel-state` for the other two closed-set
kinds.

## Today: verifying the engine directly

Until the CLI/broker wiring lands, the engine can only be exercised
through its Rust API and inline test suite:

```bash
cd packages
cargo test -p d2b-priv-broker secrets_lifecycle::
cargo test -p d2b-sk-frontend secrets_channel::
```

Every provision/rotate/rollback/retire transition, every fail-closed
tamper/drift case, and the guest-side channel-state generation-replay
guard are covered by inline `#[cfg(test)]` cases in
`secrets_lifecycle.rs` and `secrets_channel.rs`. Reviewers should not
expect these tests to run standalone from a fresh checkout without a
temporary local `pub mod` wiring edit, since neither module is
referenced from `ops/mod.rs` or `lib.rs` yet — see the reference doc's
wiring-points list.
