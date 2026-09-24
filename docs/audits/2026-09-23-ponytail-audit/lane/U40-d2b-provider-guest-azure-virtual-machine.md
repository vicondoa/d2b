# U40 d2b-provider-guest-azure-virtual-machine

Measured 2,839 LOC (src 1,882 + tests 957; `wc -l`, not estimated). All ledger rows verified at HEAD `3f2664794`.

## Findings

- **#G78 [refused, stays] - BootstrapPskDelivery one-variant enum carried as config.** The cited caller MOVED: `packages/d2bd/src/guest_effects.rs:1697` no longer exists (file gone), but the live construction now sits at `packages/d2b-provider-guest/src/effects_service.rs:1892` (`bootstrap_psk_delivery: azure_vm_runtime::BootstrapPskDelivery::VmExtension`) - same caller class, relocated, not deleted. Refusal stands: deleting the enum is a config wire change on a live daemon path. No new evidence.
- **#G83 [refused, stays] - AzureVmConfig tenant_id/client_id never read.** Still present at `src/config.rs:90,92` as `Option<OpaqueAzureRef>` with `deny_unknown_fields` wire shape; deleting is a wire change. Refusal stands.
- **#G77 [partial, verified] - PskExtensionPayload::{len,is_empty} kept.** Present at `src/effect/mod.rs:167,172`; they are the inner `Zeroizing<Vec<u8>>` field's only readers (`from_secret` reads pre-wrap bytes; the payload is passed by value into `put_vm_extension`). Removing them trips dead_code on the field per the ledger note. `as_str`/`retryable` remain deleted.
- **#G76 [applied, verified]** - `verify_owned_vm` helper live with 3 production call sites (`controller/mod.rs:378,433,529`).
- **#G79 [applied, verified]** - `tag_digest`/`vm_delete_confirmed` state fields gone; only `expected_tag_digest` (TagDigest, read at `controller/mod.rs:950,1017`) remains.
- **#G80 [applied, verified]** - `bootstrap_svc.rs` gone; bootstrap folded into `src/bootstrap.rs`.
- **#G81 [applied, verified]** - `validate_credential_scope` zero call sites; gone.
- **#G82 [applied, verified]** - `idempotency.rs` gone; fn moved next to call sites.
- **#G84 [applied, verified]** - `operation_digest` gone.
- **#G85 [applied, verified]** - `serde_json` in `[dev-dependencies]` only (`Cargo.toml:30`).
- **#G98 [applied, verified]** - `telemetry.rs`/`audit.rs` absent.
- **#G99 [applied, verified]** - no `*RunnerContract` in src.
- **#G100 [partial, verified]** - azure-vm half applied: `controller/mod.rs:5` imports `d2b_provider_toolkit::plane::{Clock, SystemClock}`; `with_clock` seam at `controller/mod.rs:250`.

## New surface scan (no findings)

- `AzureVmPhase` 12/12 variants constructed (`controller/mod.rs`); `AzureVmState` 6/6, `LroStatus` 3/3 constructed.
- `AzureVmUpdate` 4/4 variants matched in production (`controller/mod.rs:645-660,981-995`).
- `InteractionEffectsServiceFactory`/`InteractionEffectsService::new` both live (d2bd `resource_plane_v3.rs:181,2286`, `shared_provider_effects.rs:3457`).
- `AZURE_VM_REPAIR_INTERVAL_SECS`/`AZURE_VM_GUEST_FINALIZER` consts - finalizer const is the live Guest finalizer contract (per #G99 note, FINALIZER consts stay).

## Checked

Read `src/{config,bootstrap,error,lib,effect/mod,controller/mod}.rs`, `Cargo.toml`, `tests/{lifecycle_hermetic,bootstrap_hermetic,error_redaction}.rs`; grep caller searches across `packages/**/*.rs` (non-generated) for every ledger symbol: `BootstrapPskDelivery`, `verify_owned_vm`, `PskExtensionPayload`, `tenant_id`, `client_id`, `operation_digest`, `validate_credential_scope`, `tag_digest`, `vm_delete_confirmed`, `Clock`, `serde_json`, `RunnerContract`, `telemetry.rs`/`audit.rs` presence. All 13 ledger rows honored; no new evidence for any `[refused]` row (the #G78 caller relocated but remains live - cited in the row).

Lean already. Ship - every applied row verified landed, both refused rows still blocked by live daemon callers, no new dead surface found.