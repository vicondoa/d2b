# U83 d2b-provider-quota

Declared metadata crate (527 LOC). The shared declaration-only metadata
driver is the whole convergence (`src/driver.rs` = 22 lines →
`metadata_descriptor(WellKnownType::QUOTA)`); registration + status admission
are the shared vocabulary (#P1/#P2 applied in the U83 ledger rows). Re-audit
found **no new findings** beyond the typed-spec surface below.

## Findings

- `stdlib:` The crate's typed **spec** surface has zero production readers.
  `src/quota.rs` declares a hand-rolled typed `QuotaSpec`/`QuotaCeilings`/
  `QuotaTypeCeiling` plus `QuotaEnforcementPolicy`/`QuotaScope`/
  `QuotaContractError`/`QuotaConditionType` and three hand-written
  `Deserialize` impls (quota.rs:165,260,337). The admission gate converges
  the stored spec **opaque**: `d2bd/src/foundation_seed.rs:1135` admits
  quota rows by name only, and the plane's metadata driver decodes the spec
  object as opaque `serde_json::Value` — nothing anywhere constructs,
  decodes, or validates a typed `QuotaSpec`, `QuotaCeilings`, or
  `QuotaTypeCeiling` in production, and the `MAX_QUOTA_*` per-type bounds
  exist only to bound that dead typed spec. Workspace-wide search (all
  `packages/**/*.rs`, `BUILD.bazel` dep lists, `nixos-modules/`,
  `docs/reference/policy/`) found zero in-tree readers of the typed spec:
  the only consumers are the crate's own `#[cfg(test)]` block and the
  resource-api converted-status wire test, which reads **status**, not spec.
  Delete the typed spec types, the three hand-written `Deserialize` impls
  that decode only that dead spec, and the spec-only `MAX_*` consts; keep
  the served typed status (`QuotaStatusResource`/`QuotaStatus`, pinned by
  the resource-api converted-status round-trip admission at
  `d2b-resource-api/src/manager_backend/tests.rs:1046,1106` — same family
  wire-pin class the other metadata crates keep). [packages/d2b-provider-quota/src/quota.rs] (leaf) (guard)

net: -360 lines (typed spec surface in quota.rs, measured 11-380),
-0 deps

## Consistency notes

- Quota's **typed spec** is the lone dead surface here: its siblings in the
  metadata family keep typed specs where a live typed construction exists
  (e.g. operation spec is constructed at `d2bd/src/foundation_seed.rs:409`),
  but quota's typed spec is never constructed, decoded, or validated in
  production — admission is name-only opaque. The crate-level divergence is
  the served **status** layer staying typed (wire-pinned family class), which
  keeps parity with role-binding/operation/volume-local.

## Reopened refusals

- none — no ledger row for this crate is refused-and-still-present without
  new evidence; #P1/#P2 stay applied (shared driver + shared registration).

## Checked

Read `src/lib.rs`, `src/driver.rs`, `src/quota.rs` (481 lines), the shared
metadata driver, and `tests/registration.rs`. Ran workspace-wide reader
searches (`grep -rn` over all `packages/**/*.rs` + `BUILD.bazel` dep lists +
`docs/reference/policy/` + `nixos-modules/`) for every typed spec item —
`QuotaSpec`, `QuotaCeilings`, `QuotaTypeCeiling`, `QuotaEnforcementPolicy`,
`QuotaScope`, `QuotaContractError`, `QuotaConditionType`, and the
`MAX_QUOTA_*` consts. Only the crate's own tests and the resource-api
converted-status wire admission (which reads the **status** layer) consume
the crate's surface; the typed spec vocabulary has zero production readers.
LOC claims measured, not estimated.
