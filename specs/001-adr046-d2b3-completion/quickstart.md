# Quickstart: Validating the d2b 3.0 control plane

**Feature**: `001-adr046-d2b3-completion`

This run guide covers focused implementation validation and the conditional wider lanes needed
by the changed surface. Implementation details remain in `tasks.md` and the contracts in this
directory.

## Prerequisites

- Use the pinned Rust toolchain from `packages/rust-toolchain.toml`.
- Use NixOS and `/dev/kvm` only for host or VM checks that the changed surface requires.
- Keep heavy lanes within the repository semaphore and invoke public `make` targets only.
- Do not put generated or diagnostic state in the source tree.

## Focused implementation loop

1. Read the owning product requirement and contract before changing code.
2. Run the smallest enforcing test for the changed component. Rust changes normally use:

   ```bash
   make test-rust
   make test-fixture-contracts
   ```

   `make test-rust` does not include the fixture-dependent contract crate.
3. For policy, generated schema, Nix, or security changes, run the corresponding focused
   `make test-policy`, schema, Nix, or security target and its negative fixtures.
4. Review the diff for generated-artifact lockstep, changelog impact, and accidental raw
   identity or credential output.
5. `make check` remains available as an optional broader Layer-1 check; it is not a required
   pre-PR or pre-review step. An advisory result is not evidence for an enforcing requirement.

## Conditional wider validation

Run these only when the changed surface needs them:

```bash
make test-integration          # container or Layer-2 behavior
make test-host-integration     # NixOS/KVM host behavior
```

Use the applicable public live or hardware target for provider, host, device, or cutover
changes. Do not substitute a skipped or advisory job for an enforcing check. Heavy lanes use
the two-slot-per-UID semaphore; never invoke internal `heavy-lane-*` targets directly.

## Product acceptance paths

### Resource and provider plane

Validate declaration, compilation, routing, watch/replay, controller registration, effect
idempotency, audit durability, redacted status, and per-Zone readiness for the affected
resource family. A missing or unhealthy system-core handler degrades only that Zone; startup
and close must still visit every Zone.

For Network/Host integration, prove the four combinations of
`Network.spec.isolation.allowEastWest` and `d2b.site.allowUnsafeEastWest`. Both default false,
and effective east-west access is true only when both are true.

### Recovery and cutover

Before any irreversible cutover, accept one external version-1 recovery-point record covering
boot/system state, the active generation, the preview inventory, and preserved identity state.
Bind candidate, commit, tree, preview, host, operator, and restore instructions. Require
`previewed <= captured <= verified <= attested <= verifier-now < expires`, checked bounded
arithmetic, valid digest-bound import, and fail-closed handling for malformed, stale, expired,
wrong-host, or mismatched records. The external backup mechanism is outside this feature.

The broker owns the durable handoff coordinator before the first mutation and transfers it
exactly once to the target broker through the existing broker service. Privileged apply uses
only the separately pinned installed object; it performs no Nix evaluation or target
resolution. Public-socket peer credentials and current `d2b` group classification are the
operator admission input. Daemon identity, Hello, and euid 0 never authorize independently.
Every mutation has immutable audit evidence, and export-pending state never reports ordinary
success or implies rollback.

### Release and compatibility

Verify companion contracts and release notes for any changed public operator surface. Run live,
host, hardware, or compatibility checks when the changed component requires them. Preserve
capability parity unless the requirement explicitly records a retirement, successor, rationale,
and release-note entry.

## Evidence record

Report the exact focused commands and results, plus any conditional lanes intentionally not run
because their components were unchanged. Include changed paths, generated-file status, and
whether a changelog entry or documentation update was required. Do not claim a broad gate from
an unrelated test.
