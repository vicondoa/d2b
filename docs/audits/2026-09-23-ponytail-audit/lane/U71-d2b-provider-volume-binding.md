# U71 d2b-provider-volume-binding

net: -0 lines, -0 deps

Provider-dossier row-reader lane. No production-cut verified this pass — the two
row_reader surfaces and the two effects-service facades are all live, with their
construction sites lodged outside this crate's write scope.

## Refusal-ledger section (U1 packet for U71)

Honored every row in my lane: #P7 [partial], #S1 [applied], #S2 [applied],
#S10 [applied], #S11 [partial]; and the volume-binding closed family rows
(#S1/#S8/#S11 wording) — all still accurate at HEAD, none reopened. No new
evidence since the prior pass.

## Findings (ranked, biggest first)

**#U71-1 `binding_readiness_current` + `parsed_binding_spec` are the crate's
only row_reader outputs and both are live.** `grep -rn "binding_readiness_current\|parsed_binding_spec"` workspace-wide (all `packages/*/src`, `packages/*/tests`, `packages/*/integration`, `docs/reference/policy/`, `nixos-modules/`) → the only cross-crate caller is `parsed_binding_spec` consumed at `d2bd/src/resource_runtime.rs:6017` and `binding_readiness_current` at `d2bd/src/resource_runtime.rs:2247`; in-crate `driver.rs` (row_readers consumption at `driver.rs` Binding readiness path) and `row_readers.rs` tests exercise both. Not dead; not a finding.
`[leaf — verified live; closed]`

**#U71-2 `BindingEffectsService` + `BindingEffectsServiceFactory` + `BindingEffectFacets.into_facets()` have workspace consumers.** `grep -rn "BindingEffectsService\|BindingEffectsServiceFactory\|BindingEffectFacets"` → `d2bd/src/resource_plane_v3.rs:2301/3969` constructs `BindingEffectsServiceFactory`, and `d2bd/src/shared_provider_effects.rs:3512-3513` constructs the `BindingEffectsService` via an effects-query; `facets.rs::into_facets` feeds `resource_plane_v3.rs:82` (`BindingDriverFacets`). All live production paths. Not a finding.
`[leaf — verified live; closed]`

**#U71-3 Effects-service entry `reason` vocabulary: all variants constructed in-crate and consumed at `d2bd/src/resource_runtime.rs:6017` (message attribution for the read-failure surface) — no dead variant. Checked; closed.**
`[leaf — verified; closed]`

## Consistency notes

`row_readers.rs` is a per-crate lane (U71 row_readers), not a types-layer crate — the
U2–U11 "Consistency notes" section does not apply here Ring. No wire-skew observed between
`row_readers.rs` and the dossier Row.ReadinessCurrent / ParsedBindingSpec spellings; both
read `StoredResource` and project the same closed field sets.

## Reopened refusals

none — #S1/#S8/#S11 not reopened (no new evidence; the slot-shape refusals remain with
their workspace callers).

## Checked

- read in full: `driver.rs` (2,035), `row_readers.rs` (276), `facets.rs` (65),
  `effects_service.rs` (200), `lib.rs` (42), `tests/registration.rs`, `test_support.rs`,
  `row_readers_test` inline suite, BUILD.bazel (98 surface lines), `tests/` glob,
  `proto/` wire inventory
- caller-search run and result (each finding): `grep -rn "<sym>" packages/*/src packages/*/tests packages/*/integration docs/reference/policy/ nixos-modules/`
  → named live callers above; no zero-caller public surface remains in row_readers.rs
  or effects_service.rs
