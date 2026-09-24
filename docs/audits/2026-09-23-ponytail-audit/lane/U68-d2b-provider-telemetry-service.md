# U68 d2b-provider-telemetry-service

Verified cuts, ranked biggest first. Search method on every zero-caller/dead claim: workspace-wide
Rust `grep` across `packages/*/src`, `packages/*/tests`, `packages/*/integration`, `packages/d2bd/**`,
`packages/d2bd-runtime/**`, `packages/d2bd/src/**`, `bazel/**`, `nixos-modules/**`, `docs/reference/**`,
plus BUILD.bazel + Cargo.lock consumer/edge scans; all mention searches run from repo root with
`gitignore` honored.

net: -24 lines, -0 deps

- shrink Replace `TELEMETRY_SERVICE_VERBS` with the merged family's shared `CONVERTED_TYPE_VERBS`. The verb block (doc comment driver.rs:391-404 + const driver.rs:405-415, 25 lines) is byte-identical to the 9-verb `get/list/watch/create/update-spec/update-status/update-metadata/update-finalizers/delete` surface `CONVERTED_TYPE_VERBS` already publishes at d2b-resource-types/src/descriptor.rs:19-28. This crate already imports the shared descriptor vocabulary (`DriverDescriptor`, `WellKnownType`) from `d2b_resource_types`, so the swap is `verbs: CONVERTED_TYPE_VERBS` + one import line, deleting the local const and its doc. This is the #S2/#S1 family dedup applied to six sibling crates; telemetry-binding's `TELEMETRY_BINDING_VERBS` was the half reported-refused as left outside the family, and this crate's copy is an unaddressed sibling of the same list. -24 (delete 25, add 1 import). [packages/d2b-provider-telemetry-service/src/driver.rs] leaf

## Consistency notes
- Not a types-layer crate (U2-U11); no consistency section required. The crate is a converted-type
  provider in the telemetry *pair* family, whose pair-collapse disposition is U1-refused (#PR10
  partial: pair collapse needs d2b-resource-runtime test support reachable across crates).

## Reopened refusals
- None for this crate. #PR10 [partial] stays as recorded: the remaining half (pair collapse into
  one driver, `DEPENDENCY_READINESS_PROVEN` readiness gate) requires cross-crate d2b-resource-runtime
  test support, unchanged since 515cbf610; no new evidence.

Family-crate note (not counted in net): every provider crate in the family (18 crates incl. this one)
declares an empty `test-support = []` feature and a byte-mirrored `d2b_provider_*_test_support` BUILD
target referenced by d2bd/BUILD.bazel:250-260. Zero `#[cfg(feature = "test-support")]` gates exist
workspace-wide (verified; the empty feature gates nothing). Cutting it is family-wide and cross-crate
(d2bd BUILD + 18 siblings), the same blast class the plan refused for the telemetry pair collapse, so
it stays refused without new evidence.

## Checked
Read full crate: Cargo.toml, BUILD.bazel (86), integration/README.md, resource-types.json,
src/lib.rs (37), src/driver.rs (906: both override deletes, ASPECT_END_DELETE/REPAIR_AFTER
conversions were the prior #PR10 applied work - verify no regressions), tests/registration.rs (116).
Verified the 9 service verbs match CONVERTED_TYPE_VERBS byte-for-byte (descriptor.rs:19-28); confirmed
`CONVERTED_TYPE_VERBS` is `pub` in d2b-resource-types (the audit's canonical converted-verb home);
confirmed this crate already depends on d2b-resource-types (Cargo.toml:23, BUILD.bazel deps). Caller
searches: TELEMETRY_SERVICE_VERBS referenced only in-driver + tests; descriptor/symbol carries:
verified d2bd references only `telemetry_service_descriptor` (resource_plane_v3.rs:72); siblings
telemetry-binding, volume, endpoint, zone publish independent verb consts (not byte-shared). No
policy-required scaffolds skipped (README.md, BUILD.bazel, resource-types.json, integration/README.md
are the xtask/provider_crate_policy.rs ratchet surface - kept). src/generated/ absent (out of scope).
Ledger items honored: #PR10 partial stays; no zero-caller claims made without workspace-wide search.
