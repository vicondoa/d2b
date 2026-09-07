---
title: Rust Test Value Cleanup - Plan
type: refactor
date: 2026-09-07
topic: rust-test-value-cleanup
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
product_contract_source: ce-brainstorm
execution: code
---

# Rust Test Value Cleanup - Plan

## Goal Capsule

- **Objective:** Land one test-only PR that removes low-value Rust tests and folds duplicated test setup, then babysit it through review and gates to a squash-merge.
- **Product authority:** This plan governs test code only. Runtime behavior, security invariants, and the volume, cloud-hypervisor, and Guest efforts are not active scope.
- **Execution profile:** Delete dead code and dead targets first, then per-slice deletions, then fixture shrinks. Validate each crate with its focused suite before the aggregate gates.
- **Stop conditions:** Stop when review disputes a subsumption claim without a named covering test, when a deletion touches production behavior, or when a gate failure traces to removed coverage rather than removed tautology.
- **Tail ownership:** The PR lands through reviewed squash with an expected-head guard and babysitting through checks, feedback, and base currency. Product Contract preservation: unchanged in enrichment except the two deferred planning questions, whose answers folded into KTD1, KTD2, and the units. R-IDs stable.

---

## Product Contract

### Summary

Delete tautological, duplicated, and source-pinning Rust tests across the workspace. Fold duplicated test fixtures into shared helpers. Ship the result as a single PR with a changelog fragment and merge it squash-only after independent review and gates pass.

### Problem Frame

The workspace accumulates tests that pin implementation text, re-assert derive-generated behavior, and duplicate coverage between embedded and integration files. These tests cost review attention and rot on unrelated changes without catching regressions. A workspace-wide audit ranked roughly 979 lines of such tests for removal. The deletions are safe only as a set with their subsuming coverage named, so the work lands as one PR rather than scattered edits.

### Key Decisions

- **One PR over stacked slices** (session-settled: user-directed — chosen over per-slice PRs: faster wall-clock for a mechanical, test-only change).
- **Deletes plus shrinks over deletes-only** (session-settled: user-directed — chosen over deletes-only: lands the full audit value at once).
- **Disputed-subsumption rule.** If review challenges any deletion's covering test, that test stays. Governs R1, R2, R3, R4, R5.
- **Replacement tests are out.** Every deletion names kept coverage; no new tests are added. Governs R1, R2, R3, R4.

### Requirements

**Control-plane source pins and dead helpers**

- R1. The source-text-pinning test groups are deleted: the `include_str!` self-assertion sets in the daemon composition suite, the broker spawn/sys suite, and the runtime readiness and kernel-module suites.
- R2. The dead test code is deleted: the uncalled `wait_timeout` builder in the daemon integration fixtures, the self-admitted-dead `set_mode` helper in the bundle-tamper test, the never-read admission parameter with its call-site arguments, and the test-only role branch collapsed to real role data.
- R3. The tautology tests are deleted: the guest-signal mapping stability test and the per-path structured-log formatting test.

**Contract, session, and resource tautologies**

- R4. The duplicated foundation contract tests are deleted in favor of the embedded token, identifier, error-envelope, and workload-identity tests that already cover the same behavior.
- R5. The constant and serializer pins are deleted: service-string, schema-version, queue-capacity, audit-capacity, clock-sanity, and JSON round-trip tests that assert derive-generated or literal behavior.
- R6. The duplicate race-harness wrapper is collapsed to one test with a name describing the race.

**Provider duplicate coverage**

- R7. The weaker embedded duplicates are deleted where an integration or snapshot test asserts the same behavior exactly: ownership allowlists, wire-constant snapshots, interface-name derivation, and firewall projection tests.
- R8. The no-op GPU lifecycle test target is removed together with its test-target registration, sweeping build references per the retirement rule.

**Test-helper shrinks**

- R9. Duplicated fixture constructors are hoisted once: the six controller test files share one common test module, and the session cancellation and admission setups share one harness each.
- R10. Same-shape test rows become table-driven: audio policy parsing, security-key event options, GPU argument rejections, resource-compiler failure setups, and name-conflict ownership variants.
- R11. The wrapper-layer collapse is dropped: the sole forwarding-wrapper candidate proved production-owned by other files and stays untouched per the no-production-changes boundary.
- R12. The hand-rolled temporary-directory helper is replaced with the temporary-directory helper sibling crates already use.

**PR hygiene and shipping tail**

- R13. The PR ships a changelog fragment under `changelog.d/` for the branch, since the change touches shipped test surfaces.
- R14. The PR body groups changes per audit slice and names the subsuming coverage for each deletion group.
- R15. The PR lands only after fresh independent review with no actionable findings, required gates green, and an expected-head guarded squash merge with babysitting through the PR tail.

### Acceptance Examples

- AE1. **Covers R1.** Given a source-text-pinning test is deleted, when the suite runs, then the surrounding functional tests for that behavior still pass.
- AE2. **Covers R8.** Given the no-op test target and its registration are removed, when the aggregate gate runs, then no target references the deleted file.
- AE3. **Covers the disputed-subsumption rule.** Given review disputes a deletion's covering test, when the PR is updated, then the disputed test is restored and the PR proceeds without it.

### Success Criteria

- Required gates are green on the merge head and review evidence binds the repository, base, head, and verdict with no actionable findings remaining.
- The merged diff touches test code and test-only seams only, plus the changelog fragment.

### Scope Boundaries

- Volume, cloud-hypervisor, and Guest test files are excluded; parallel efforts own them.
- The `CallRecorder::is_empty` helper stays; reference checks show three live callers.
- No production behavior changes. The two production-touching items are test-only seams: the dead admission parameter and the test-only role branch.
- Layer-2 container, VM, and live-host lanes do not run for this change; it is hermetic Layer-1 test code.

### Dependencies / Assumptions

- The test-model retirement rule in `tests/AGENTS.md` authorizes deletion plus reference sweeps without replacement ledgers.
- Commit and changelog grammar follows `docs/contributing/changelog-and-commits.md`: area-prefixed imperative subjects, no tool attribution.
- The branch `ponytail-cleanup` worktree carries the work; landing targets the protected branch through a reviewed PR only.

### Outstanding Questions

- Resolve Before Planning: none.
- Deferred to Planning: none. Both deferred items from the requirements pass answered during planning and folded into KTD1, KTD2, and the units.

### Sources / Research

- Workspace test-value audit with per-slice findings and line estimates, verified by reference checks against the cited files.
- `tests/AGENTS.md` retiring-a-test rule and Layer-1 lane definitions.
- `docs/contributing/changelog-and-commits.md` fragment and commit conventions.
- `docs/plans/2026-08-19-002-refactor-build-test-ownership-cleanup-plan.md` as adjacent prior art on test ownership boundaries.

---

## Planning Contract

### Key Technical Decisions

- KTD1. Shared fixtures hoist into each crate's existing `tests/common/mod.rs`, consumed by plain `mod common;` with `#![allow(dead_code)]` at the top. This extends the daemon, d2b, and xtask precedent instead of the new support-module layout the audit suggested. Governs R9.
- KTD2. The dead-target sweep covers the Cargo manifest entry, the Bazel `rust_test` rule, the workspace-sources filegroup glob check, and retirement of the target's integration README. The generated provider catalog needs no change since it already points at the kept suite. The same-named container-lane tests in the security-key crate stay untouched; only its integration relay test is trimmed per U6. Governs R8.
- KTD3. Verification runs per-crate focused suites with the local Bazel profile before the aggregate aliases. This instantiates the gate rule per R15 without restating it.
- KTD4. Table-driven folds mirror the named-vector shape used by the zone-routing vector tests: one test per behavioral family, rows carrying human-readable names surfaced in assertion messages. Governs R10.
- KTD5. Units land dead code first, then deletions per slice, then shrinks. Early units keep the tree green while later units reshape files the deletions already settled.

### Sequencing

- U1 first: it removes the dead target registration that later units must not reference.
- U2, U3, U4 next in any order: they touch disjoint crate sets.
- U5, U6 last: they reshape helpers and test bodies inside files the deletion units already settled.
- The changelog fragment ships with U1 and stays current as units land.

---

## Implementation Units

### U1. Dead code, dead target, and changelog fragment

- **Goal:** Remove unreferenced test code and the no-op test target, and open the changelog fragment.
- **Requirements:** R2, R8, R13.
- **Dependencies:** None.
- **Files:** `packages/d2bd/tests/common/mod.rs`, `packages/d2bd/tests/bundle_tampered_envelope.rs`, `packages/d2b-bus/src/session_seam_tests.rs` and its admission call sites, `packages/d2b/src/doctor.rs`, `packages/d2b-provider-device-gpu/integration/provider_lifecycle.rs`, `packages/d2b-provider-device-gpu/Cargo.toml`, `packages/d2b-provider-device-gpu/BUILD.bazel`, `packages/d2b-provider-device-gpu/integration/README.md`, `changelog.d/ponytail-cleanup.md`.
- **Approach:** Delete the uncalled builder, the self-admitted-dead helper, and the never-read admission parameter with its per-call-site arguments. Collapse the test-only role branch to real role data. Remove the no-op target file, its manifest entry, its build rule, and its README per KTD2. Open the fragment with `Removed` and `Changed` bullets in operator-facing language.
- **Patterns to follow:** The retirement rule in `tests/AGENTS.md` for the sweep shape.
- **Test scenarios:**
  - Each touched crate suite passes with the dead code absent.
  - The aggregate gate references no removed target or file.
  - The changelog gate accepts the new fragment.
- **Verification:** Focused per-crate suites for every touched crate, then the changelog gate. Per the Verification Contract.

### U2. Control-plane pin and tautology deletions

- **Goal:** Delete source-text pins and tautologies in the daemon, broker, runtime, CLI, and activation-helper crates.
- **Requirements:** R1, R3.
- **Dependencies:** U1.
- **Files:** `packages/d2bd/src/composition.rs`, `packages/d2b-broker/src/sys.rs`, `packages/d2b-broker/src/live_handlers.rs`, `packages/d2bd-runtime/src/readiness.rs`, `packages/d2bd-runtime/src/kernel_module_check.rs`, `packages/d2b/src/exec_client.rs`, `packages/d2b/src/complete.rs`, `packages/d2b-host-activation-helper/src/main.rs`.
- **Approach:** Delete the `include_str!` self-assertion test groups per R1. Delete the mapping-stability and log-formatting tautologies per R3. Demote the const-assert wrapper test to a module-level const assertion.
- **Patterns to follow:** The disputed-subsumption rule: each deletion group keeps its named surrounding functional coverage.
- **Test scenarios:**
  - Surrounding functional tests for each deleted pin still pass and still exercise the pinned behavior.
  - No remaining test in these files reads its own source via `include_str!`.
  - Touched crate suites are green.
- **Verification:** Focused per-crate suites for every touched crate. Per the Verification Contract.

### U3. Contract, session, and resource deletions

- **Goal:** Delete duplicated and tautological tests in the contract, session, zone-routing, resource, and provider-core crates.
- **Requirements:** R4, R5, R6.
- **Dependencies:** U1.
- **Files:** `packages/d2b-contracts/tests/foundation.rs`, `packages/d2b-contracts-zone-session/tests/contracts.rs`, `packages/d2b-contracts/src/security_key.rs`, `packages/d2b-contracts/src/error.rs`, `packages/d2b-contracts/src/lib.rs`, `packages/d2b-bus/src/metrics.rs`, `packages/d2b-core/src/host_w3.rs`, `packages/d2b-core-controller/src/authority.rs`, `packages/d2b-core-controller/tests/configuration_name_conflict.rs`, `packages/d2b-core/tests/manifest_v04_roundtrip.rs`, `packages/d2b-session/src/driver.rs`, `packages/d2b-session/src/server.rs`, `packages/d2b-zone-routing/src/service.rs`, `packages/d2b-resource-api/tests/protocol.rs`, `packages/d2b-resource-api/src/watch.rs`, `packages/d2b-resource-api/src/store.rs`, `packages/d2b-resource-client/src/call.rs`, `packages/d2b-resource-store-redb/src/tests.rs`, `packages/d2b-provider/src/registry.rs`, `packages/d2b-provider/tests/runtime.rs`, `packages/d2b-provider-toolkit/src/audit.rs`.
- **Approach:** Delete the foundation duplicates in favor of the named embedded tests per R4. Delete constant, capacity, clock, and round-trip pins per R5. Collapse the identical race wrappers to one race-named test per R6. Trim the manifest round-trip file to its structural equality assertion.
- **Patterns to follow:** The disputed-subsumption rule: the nonce-helper self-test goes only because its three real call sites stay.
- **Test scenarios:**
  - Embedded token, identifier, error-envelope, and workload-identity tests pass and cover the deleted foundation cases.
  - The single kept race test fails if the notification helper regresses.
  - Touched crate suites are green.
- **Verification:** Focused per-crate suites for every touched crate. Per the Verification Contract.

### U4. Provider duplicate deletions

- **Goal:** Delete weaker embedded duplicates and tautologies across the device, network, audio, clipboard, and transport provider crates.
- **Requirements:** R7.
- **Dependencies:** U1.
- **Files:** `packages/d2b-provider-device-gpu/src/video_argv.rs`, `packages/d2b-provider-device-gpu/src/gpu_argv.rs`, `packages/d2b-provider-device-tpm/src/swtpm_argv.rs`, `packages/d2b-provider-transport-vsock/src/relay_argv.rs`, `packages/d2b-provider-system-core/src/ownership.rs`, `packages/d2b-provider-network-local/tests/network_primitives.rs`, `packages/d2b-provider-network-local/src/nftables.rs`, `packages/d2b-provider-audio-pipewire/src/audio_argv.rs`, `packages/d2b-provider-audio-pipewire/tests/audio_policy.rs`, `packages/d2b-provider-clipboard-wayland/src/clipd_host/audit.rs`, `packages/d2b-provider-clipboard-wayland/tests/clipd_pipe.rs`.
- **Approach:** Delete serde round-trip tautologies across all five copies per R7. Delete embedded copies where the exact snapshot or integration test subsumes them. Delete the upstream-flag test and the third queue-probe row.
- **Patterns to follow:** Keep the exact-snapshot and end-to-end reconciler tests as the surviving canonical coverage.
- **Test scenarios:**
  - The exact wire and snapshot tests pass and pin the behavior the deleted copies asserted.
  - The end-to-end ownership reconciler test refuses every disowned type.
  - Touched crate suites are green.
- **Verification:** Focused per-crate suites for every touched crate. Per the Verification Contract.

### U5. Fixture-hoist shrinks

- **Goal:** Hoist duplicated test fixtures into shared per-crate helpers.
- **Requirements:** R9, R12.
- **Dependencies:** U2, U3, U4.
- **Files:** `packages/d2b-core-controller/tests/`, `packages/d2b-core-controller/BUILD.bazel`, `packages/d2b-session/src/driver.rs`, `packages/d2b-session/src/server.rs`, `packages/d2b-session/tests/admission.rs`, `packages/d2b-host-activation-helper/src/main.rs`, `packages/d2b-host-activation-helper/Cargo.toml`, `packages/d2b-host-activation-helper/BUILD.bazel`, `packages/d2b/tests/stub_no_socket.rs`, `packages/d2bd/tests/stub_no_socket.rs`.
- **Approach:** Create the shared common module per KTD1 and migrate the six controller files to it. Extract the cancellation harness and fold driver construction into the existing policy fixture. Replace the hand-rolled temporary directory with the shared helper, adding the dev-dependency and its build-rule wiring. Document the intentional twin snapshot helpers with a comment instead of cross-crate sharing.
- **Patterns to follow:** The per-crate common-module precedent with dead-code allowance at the top.
- **Test scenarios:**
  - Every migrated test file passes through the shared helper with identical assertions.
  - No migrated file retains a local copy of a hoisted constructor.
  - Touched crate suites are green.
- **Verification:** Focused per-crate suites for every touched crate. Per the Verification Contract.

### U6. Table-driven folds and remaining shrinks

- **Goal:** Convert same-shape test rows to named tables and finish the remaining shrinks.
- **Requirements:** R10.
- **Dependencies:** U2, U3, U4.
- **Files:** `packages/d2b-provider-notification-desktop/src/security_key/events.rs`, `packages/d2b-provider-device-usbip/src/state_machine.rs`, `packages/d2b-provider-device-gpu/tests/combined_reconcile.rs`, `packages/d2b-provider-device-gpu/src/gpu_argv.rs`, `packages/d2b-provider-device-gpu/src/video_argv.rs`, `packages/d2b-provider-audio-pipewire/tests/audio_policy.rs`, `packages/d2b-provider-network-local/src/nftables.rs`, `packages/d2b-provider-network-local/tests/network_primitives.rs`, `packages/d2b-provider-device-security-key/integration/provider_lifecycle.rs`, `packages/d2b-resource-compiler/tests/phase2.rs`, `packages/d2b-core-controller/tests/configuration_name_conflict.rs`.
- **Approach:** Fold option-variant, plan-variant, preamble, rejection-shape, parse-variant, and failure-setup rows into named tables per KTD4. Merge the overlapping projection test into the integration copy. Trim the relay integration test to its integration-specific assertions.
- **Patterns to follow:** The named-vector shape with row names surfaced in assertion messages.
- **Test scenarios:**
  - Each folded table passes and fails with the offending row named.
  - The explicit-versus-declared plan difference is asserted exactly once.
  - Touched crate suites are green.
- **Verification:** Focused per-crate suites for every touched crate. Per the Verification Contract.

---

## Verification Contract

- Focused runs use direct Bazel labels with the local profile, one per touched crate: `bazel test //packages/<crate>:all-tests --config=local`.
- Aggregates run after all units: `make test-rust`, then `make test-unit`, then `make check`, plus `make test-changelog` for the fragment.
- Commit before validation so tracked inputs are visible to evaluation.
- Layer-2 container, VM, and live-host lanes do not run; the change is hermetic Layer-1 test code.
- A gate failure traces to either removed tautology or removed coverage; the disputed-subsumption rule decides whether the test stays or the failure is accepted as cleanup.

---

## Definition of Done

- All six units land with their test scenarios satisfied and no other caller referencing removed helpers or targets.
- Focused suites pass per touched crate, and the aggregate gates plus the changelog gate are green.
- Independent review returns no actionable findings on the merge head.
- The PR squash-merges with an expected-head guard after babysitting through checks, feedback, and base currency.
- The diff holds test code and test-only seams plus the changelog fragment; abandoned-attempt code and throwaway files are removed, not left in the diff.
- The PR body groups changes per audit slice and names the subsuming coverage for each deletion group (R14).
- A review-disputed deletion is restored without blocking the remainder of the PR (AE3).
