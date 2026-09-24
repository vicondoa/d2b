# U54 d2b-provider-credential-entra
net: -0 lines, -0 deps - Lean already. Ship.

## Checked (per refusal classes 1-2 + caller census)
- Hand-rolled uid/UUID renderers (#P7/#S1 continuation probe): zero non-test sites workspace-wide (grep -rn "ResourceUid|uid_to_text|lower_hex" packages/d2b-provider-credential-entra/src returned only test-file expectations); the crate's sha256 use is zero - it forwards entirely to d2b-contracts HMAC surface (credential.rs in d2b-provider-toolkit), so no in-crate renderer/VSM copy exists to migrate. Caller census: crate has no zero-caller module; three src modules (audit/controller/service + lib + tests) are all live (audit is the registration gate, controller is the wire-session controller with live_handlers callers confirmed at d2bd/src/composition.rs:5920, service is the README-scaffold entry).
- Dead module/surface: none found (audit.rs, telemetry.rs surfaces are small + used by live handlers/tests).

## Consistency notes
- dtype: none (types-layer finding not applicable - this is a provider crate).
## Checked
Sources read: packages/d2b-provider-credential-entra/src/*.rs (2667 LOC). Searches: workspace-wide grep for the #P7/#S1 uid-render pattern + dead-module census (zero candidates). Refusal-ledger rows honored (no prior findings row → "no prior findings" honored; enta appears only as part of the cross-cutting #P7/#S1 ledger rows which are refused-crate continuation - no new evidence, stays refused).

## U5 execution (2026-09-24)

- U53 family findings applied to this crate: `ABSOLUTE_UNIX_MILLIS_THRESHOLD` const +
  `now_unix_ms` + `is_absolute_unix_ms` re-pointed to the toolkit credential module
  (byte-identical bodies; `time_bound_instant_at` now reads the toolkit threshold
  const); `reject_process_environment_credential_chain` env-scan re-pointed to the
  toolkit wrapper (public error surface unchanged).
- R4 divergence note: entra's `operation_deadline` was NOT byte-identical to the
  family trio at HEAD (delegates to `time_bound_instant`/`time_bound_instant_at` with
  `checked_sub` and no zero-duration rejection); kept in-crate to preserve behavior -
  the U53 lane's byte-identical claim is stale for this symbol.
