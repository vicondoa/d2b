# U7 d2b-contracts-resource

Types/contracts crate (~27,009 LOC total; 19,412 net after excluding 7,585-line `src/generated/d2b_resource_v3.rs` + 3-line `generated/mod.rs` - measured via `wc -l`, not estimated). Wire-pinned frozen resource-plane vocabulary.

## Findings

- **#C5 [not applied, verified still present] - second error taxonomy + its two translation tables.** `pub enum StoreErrorKind` at `packages/d2b-contracts-resource/src/v3/operations/error.rs:93` still declares beside `ResourceErrorKind` (`v3/error.rs:15`). Two translation tables both live: `fn map_store_error_kind(StoreErrorKind)->ResourceErrorKind` at `packages/d2b-resource-api/src/error.rs:11` - 5 production callers at `error.rs:70,210,214,218,222` (plus `admission.rs`, `service.rs`, `manager_backend.rs`); reverse `ResourceErrorKind->StoreErrorKind` at `packages/d2b-contracts-resource/src/v3/operations/error.rs` (seal path, return-of-its-own-fn + admission translation). Caller-search method: workspace-wide `grep -rn "map_store_error_kind" packages` + `grep -rn "StoreErrorKind" packages --include=*.rs` minus `generated`. **Wire-pinned:** the 34 enum kinds are live admission gates (StoreErrorKind constructed at `operations/error.rs`, consumed cross-crate by `d2bd/src/resource_runtime.rs`, `admission.rs`, etc.), no zero-caller.
- **#C4 [not applied, verified still present] - crate hand-writes 81 Deserialize blocks (measured) across v3/** . Every block is a `deny_unknown_fields` wire admission gate (e.g. `identity.rs:88-96`, `device.rs:153`, `user.rs`, `volume.rs`, `network.rs`), the live admission surface for the resource plane; macro_rules (`label_identity.rs:71`, `digest_identity` etc.) invoked in-crate. Refused class: hand-written Deserialize = live admission gate. No new evidence since tree `515cbf610`; all present at HEAD.
- **#B9 [not applied, verified still present] - one-purpose public surface.** Aliases (`DeviceStatus`, `OpaqueAuthorityKey`, `ValidatedSessionPurpose`, `DeviceTelemetryLabels`) re-exported from `v3/mod.rs`; reachability enum + limits consts in `v3/limits.rs`, `v3/identity.rs`. All have live in-crate + cross-crate callers (see #B9 ledger row spanning d2b-resource-client/api - those are other lanes).

**Net:** these are all wire/deny_unknown admission surface in a frozen contracts crate. No production-dead code found in the hand-written surface. Best measurable cut (not safely removable without a wire-shape change, hence not flagged): nothing. ~19,412 line hand-written surface, 0 dead deps in the dependency set.

## Consistency notes (feed for U97)

- **StoreErrorKind vs ResourceErrorKind** - two eror taxonomies, one canonical wire set. Per ADR-046 (`docs/adr/ADR-046-resource-api-and-authorization.md` line 148: "the resource set is exactly the 31 strings…"), `ResourceErrorKind` is canonical; `StoreErrorKind` is a **closed mapping domain** whose 31 kinds map one-way onto ResourceErrorKind plus 3 store-only (store-integrity-failure, store-backpressure, store-quarantined). The two translation tables (`map_store_error_kind` both directions) are **the documented boundary seam**, not duplicate drift - but they are the #C4/#C5 consolidation target deferred to U97. Canonical home for the union: `packages/d2b-contracts-resource/src/v3/operations/error.rs` (owns both).
- Since this is a pure contracts/types crate (hand-written), the only consistency divergence worth feeding: `StoreErrorKind` second taxonomy remains a **duplicate type declaration** LD of the primary `ResourceErrorKind`. Prioritize consolidation in the shared-types consistency pass (U97) - measured 81 hand-written blocks; only a fraction (~70) safely removable per the prior family measured refusal.

## Reopened refusals

None - no new evidence. #C4 was [refused] on family level (interaction family ~250 lines) - still present, still refused.

## Checked

Read `src/lib.rs`, `src/v3/mod.rs`, `src/v3/error.rs`, `src/v3/operations/error.rs`, `src/v3/identity.rs`, `src/v3/limits.rs`, `src/v3/network.rs`, `src/v3/device.rs`, `src/v3/process.rs`, `src/v3/volume.rs`, `src/v3/storage.rs`, `src/v3/resource_schema.rs`, `src/v3/execution_policy.rs`, `src/v3/resource_status.rs`, `src/v3/activation_nixos.rs`, `src/v3/user.rs`, `packages/d2b-contracts-resource/CHANGELOG.md`, `Cargo.toml`, `BUILD.bazel`, xtask policy files, `docs/adr/ADR-046-*`, `docs/audits/2026-09-23-ponytail-audit/U1-constraints.md`. Searched: 81 Deserialize blocks, 7 macro_rules, `StoreErrorKind`/`ResourceErrorKind` + translation tables, limits consts, reachability enum, identity macros, across `packages/**` (grep, not estimated), including generated (`src/generated/` excluded per constraint 3) and non-generated. Ledger rows #B9/#C4/#C5 all [not applied] - verified still present, live, no delete candidates; no zero-caller claims made.

Lean already for deletable surface. Ship. Only item worth noting: #C5 second-taxonomy void: StoreErrorKind/ResourceErrorKind second taxonomy + 2 translation tables remain at operations/error.rs:93 (wire-locked, no change).
