# U39 d2b-provider-guest-azure-container-apps

net: -402 lines, -0 deps

- delete `AcaController::adopt()` — zero callers anywhere (src, tests, daemon); the crate's `ambiguous_adoption_fails_closed` test exercises `reconcile()`, not `adopt()`. [packages/d2b-provider-guest-azure-container-apps/src/controller.rs:436-521](leaf)
- delete recovery surface: `recovery_state()`, `restore_recovery_state()`, `AcaRecoveryState`, `AcaFinalizationStage::{as_str,parse}` — read only by the crate's own tests (`finalization_stage_survives_controller_restart`, `restored_delete_stage_rechecks_a_stopping_sandbox`); no production caller. [packages/d2b-provider-guest-azure-container-apps/src/controller.rs:324-357,182-195,235-256](leaf)
- delete `AcaStatus` + `status()` + Debug impl + accessors + lib.rs re-export arm — read only by `d2bd/tests/cloud_composition.rs:490` (Debug-redaction assertion); delete both; the daemon publishes status through its own sink. [packages/d2b-provider-guest-azure-container-apps/src/controller.rs:108-145,370-383](family)
- delete `AcaSandboxProfile::{cpu,memory,auto_suspend_secs,sandbox_identity_binding_id}` accessors — zero callers (wire-pinned fields stay). [packages/d2b-provider-guest-azure-container-apps/src/effects.rs:219-233](leaf)
- delete `AcaCpuMillis::get` / `AcaMemoryMib::get` — zero callers. [packages/d2b-provider-guest-azure-container-apps/src/effects.rs:114-116,140-142](leaf)
- delete `AcaControlContext::{operation_id,deadline_remaining_ms}` accessors — zero callers. [packages/d2b-provider-guest-azure-container-apps/src/effects.rs:791-796](leaf)
- delete `AcaCredentialLeaseRequest::{operation_id,purpose}` accessors — zero callers. [packages/d2b-provider-guest-azure-container-apps/src/effects.rs:753-760](leaf)
- delete `AcaCredentialLease::metadata()` accessor — zero callers. [packages/d2b-provider-guest-azure-container-apps/src/effects.rs:714-716](leaf)
- delete `AcaCredentialPurpose::as_str()` — zero callers. [packages/d2b-provider-guest-azure-container-apps/src/effects.rs:687-694](leaf)
- shrink `lib.rs` re-export block: `CompletedOperationLedger` + `SystemAcaClock` re-export arms have no external consumers (only controller.rs + lib.rs); types stay internal. [packages/d2b-provider-guest-azure-container-apps/src/lib.rs](leaf)
- delete `AcaProviderConfig::validate_gateway_execution()` + its own test — test-only (gateway_execution_validation_has_no_host_fallback). [packages/d2b-provider-guest-azure-container-apps/src/effects.rs:473-484](leaf)

## Consistency notes

n/a — this crate is not a types-layer contract crate; it is a declared guest provider (U1 U39 ledger: G71-G75 applied).

## Checked

Read `src/controller.rs` (controller + lib + Debug/Debug-redacted projections), `src/effects.rs` (effect contracts + raw twins), `src/lib.rs`, `Cargo.toml`, `BUILD.bazel`, `README.md`, `integration/README.md`, `nix/default.nix`, `nix/tests/default.nix`, `tests/provider_lifecycle.rs`. Workspace-wide caller verification (Rust sources + BUILD.bazel + nixos-modules + d2bd + d2bd-swift + tests): `adopt()` zero callers; recovery surface read only by its own tests; `AcaStatus`/`status()` read only by the `d2bd/tests/cloud_composition.rs:490` Debug-redaction assertion + the daemon's own sink; the four `AcaSandboxProfile` accessors, `AcaCpuMillis::get`/`AcaMemoryMib::get`, `AcaControlContext` accessors, `AcaCredentialLeaseRequest` accessors, `AcaCredentialLease::metadata()`, `AcaCredentialPurpose::as_str()`, `validate_gateway_execution` all have zero callers workspace-wide. `CompletedOperationLedger`/`SystemAcaClock` re-export arms not consumed externally. Ledger items G71-G75 [applied] honored; code they deleted is gone. Refused classes preserved: wire-pinned `AcaSandboxProfile` fields, `FINALIZER`/`ACA_GUEST_FINALIZER`/`REPAIR_INTERVAL_SECS` constants (runner-contract family), `deny_unknown_fields` on raw wire shapes.## U1 execution (2026-09-24)

All 10 executable findings applied:

- `src/controller.rs` — deleted `AcaStatus` (struct + impl + Debug), `AcaRecoveryState` struct, `impl AcaFinalizationStage` (`as_str`/`parse`), `recovery_state()`, `restore_recovery_state()`, `status()`, `adopt()`; dropped now-unused `sha2` import. `AcaFinalizationStage` enum itself stays (field type in `AcaController` + used by `finalize()` match); `CompletedOperationLedger`/`SystemAcaClock` stay internal.

- `src/effects.rs` — deleted `AcaCpuMillis::get`, `AcaMemoryMib::get`, `AcaSandboxProfile::{cpu,memory,auto_suspend_secs,sandbox_identity_binding_id}`, `AcaCredentialPurpose::as_str`, `AcaCredentialLease::metadata`, `AcaCredentialLeaseRequest::{operation_id,purpose}`, `AcaControlContext::{operation_id,deadline_remaining_ms}`, `AcaProviderConfig::validate_gateway_execution()`; deleted its test `gateway_execution_validation_has_no_host_fallback`; shrunk the `mod tests` import block (kept `AcaRuntimeConfig`)..


- `src/lib.rs` — re-export arm shrunk to `AcaClock, AcaController, AcaControllerError, AcaPhase, AcaReconcileOutcome, AzureContainerAppsRuntimeProvider, ACA_GUEST_FINALIZER, ACA_REPAIR_INTERVAL_SECS` (`AcaRecoveryState`, `AcaStatus`, `CompletedOperationLedger`, `SystemAcaClock` dropped)..


- `tests/provider_lifecycle.rs` — deleted `finalization_stage_survives_controller_restart` + `restored_delete_stage_rechecks_a_stopping_sandbox`; removed `status()` redaction assert from `running_sandbox_reaches_ready_without_exposing_identity`; removed `AcaRecoveryState` import..
- `packages/d2bd/tests/cloud_composition.rs` — removed the `aca.status()` Debug-redaction assert block (phase assert kept). Nothing else touched therein..

TESTS: `cargo check -p d2b-provider-guest-azure-container-apps --all-targets` PASS;; `cargo test -p d2b-provider-guest-azure-container-apps` PASS (1 unit + 13 provider_lifecycle integration +and 0 doc); `cargo test -p d2bd --test cloud_composition` — dispatched in background at yield time. R4: all claims re-verified at HEAD before cuts.