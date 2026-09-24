# U21 d2b-broker - retention-lane audit (lane U21, 2026-09-23 ponytail)

net: -437 lines, -0 deps across the two applied findings.

## Findings (ranked)

1. `delete:` `packages/d2b-broker/src/zone_identity.rs` (413 lines) + the `pub mod zone_identity;` declaration at `packages/d2b-broker/src/lib.rs:55` - workspace-wide zero-caller module: it re-derives zone resource-binding/index identity vocabulary that no workspace code (Rust sources, tests, BUILD.bazel, nixos-modules/, docs/reference/) references outside its own declaration and its own in-file tests. Verified: `grep -rn "zone_identity"` across `packages/` (+ BUILD.bazel, nixos-modules/, docs) returned only `lib.rs:55` and the module file itself with its tests; `#![allow(dead_code)]` at `lib.rs:17` + `pub mod` here, so the compiler never surfaces it)Skip. `ResourceUid`/`ResourceRef`/`ZoneId` all live in d2b-contracts(+resource) injected via `ResourceRef::parse` at the boundary; the broker does not need its own ZoneResourceBinding/ZoneResourceIndex. Delete the module file + the declaration line. [leaf]
   - **Caller-verified:** `grep -rn "zone_identity\|ZoneResourceBinding\|ZoneResourceIndex\|ZoneIdentityError" packages/d2b-broker` returned only lib.rs:55 + the module file (413 lines); `grep -rn "zone_identity" --include=*.rs --include=BUILD.bazel .` returned no other workspace site. Method: full-workspace grep on module+type names.
   - Replacement: none (nothing replaces it).
   - **Zero-caller: YES** - ref search method: `grep -rn "zone_identity"` whole-workspace incl. BUILD.bazel/nixos-modules/tests; + `ResourceUid`/`ZoneResourceIndex` name searches.

2. `shrink:` `packages/d2b-broker/src/ops/device_worker.rs:436-475` - hand-rolled UUIDv4 renderer tail in `deterministic_resource_uid` duplicates the shape forcer + dashed renderer already shared at `packages/d2b-contracts/src/identity.rs:580-621` (`ResourceUid::from_bytes`), which does byte-for-byte the same: forces bit shape, forces version/variant bits, forces dashed rendering, `ResourceUid::parse`. The broker's tail (lines 436-475, ~20 lines) can be replaced by one `ResourceUid::from_bytes(&bytes)` call plus `expect`. The sha2 digest front (440-448) stays.
   - **Caller-verified:** `deterministic_resource_uid` has in-crate callers (device_worker.rs:417-418, 467) - this is NOT a zero-caller claim; the migration is a shared-render consolidation, matching the cross-crate #P7/#S1 ledger (twelve remain; broker is in-scope). Replacement: `ResourceUid::from_bytes(&bytes).expect("row uids satisfy the UUIDv4 contract")` - removes the hand-rolled format! tail (~19 lines), keeps sha2 front.
   - **Zero-caller: NO** - 3 in-crate callers + tests.

## Consistency note
- zone_identity.rs re-mints `ResourceUid`-shaped types locally even though the crate already injects the shared `ResourceUid`/`ResourceRef` through the ops boundary (device_worker.rs:24 `ResourceRef`, `ResourceUid` imports from d2b_contracts_resource). Deleting the local re-mint is consistent with U4's #C4 row (cross-crate contract unification already in the ledger).

## Checked
- zone_uid → `ResourceUid::parse` renderer sites; sha256 uid renderer duplicates across broker src; the U21 prior-ledger rows (#P7 partial, #S1 applied) honored - broker's remaining uid renderer consolidated via shared `ResourceUid::from_bytes`; zone_identity dead module. Scout independently verified zero-caller workspace-wide (no other crates/tests/BUILD refs). Refusal-ledger rows honored: no prior refused row for this crate contains new evidence I can reopen.
