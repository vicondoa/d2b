# U88 d2b-provider-operation
net: -0 lines, -0 deps

Lean already. Ship.

Every in-scope surface verified live at HEAD (3f2664794):
- `operation_descriptor` - production callers at d2bd/src/foundation_seed.rs:1117 (`register_operation_controller`) and d2bd/src/resource_plane_v3.rs:142 (import) + the descriptor's `OperationSpec` shape is the DTO-resource-schema ratchet's committed reader at packages/xtask/src/zone_schema.rs:1243 (`dto_resource_schema::<d2b_provider_operation::OperationSpec>`), so the descriptor, its spec type, and the wire-shape contract are all pinned by resolver-authority parities
- `CREDENTIAL_TYPE_NAME`/session/effects - the #PR14 ledger surface is production-read on the credential side and remains policy-required on this crate's `integration/` (README-only ratchet excluded)
- integration/operation.rs + tests/registration.rs - policy-required per-crate scaffolds (U88 #P9 [refused] stands; xtask provider_crate_policy ratchet requires integration/*.rs for these crates)
- driver.rs (21 lines) - the #P1 [applied] ten-copy metadata-driver shared-driver migration is in place; this crate's driver.rs is the shared `operation_descriptor` adapter

## Reopened refusals
None. #P9 [refused] integration scaffolds - crate-layout policy requires an integration/*.rs for every crate not on the README-only ratchet (packages/xtask/src/provider_crate_policy.rs); not reopened, no new evidence.

## Checked
Read lib.rs, driver.rs, operation.rs (810 lines); workspace-wide caller search for `operation_descriptor` (+ BUILD.bazel refs) - the only live consumers are d2bd (foundation_seed.rs, resource_plane_v3.rs) and the committed zone-schema ratchet in xtask; passed integration/ read; verified BUILD.bazel dependency list matches Cargo.toml (no unused deps - the shared-driver migration removed the d2b-contracts-resource dependency per U88 #P10 [applied]).
