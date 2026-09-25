# tail-3 - tail lane
Baseline: 6ebdd4cec22e6537d1376e83ff7a82b00a8492c0 | LOC audited: 1723 (excl. src/generated/**) | modules: d2b-provider-process-minijail, d2b-provider-quota, d2b-provider-resource-export, d2b-provider-resource-import, d2b-provider-role
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test, supply | Partitions: whole crates (tail lane)

## d2b-provider-process-minijail
### idiom
- clean: seeds ran 0/0/0; no index loops, no hand-written impls (the only impls are the required `ProcessProvider` trait impl and derives), no statement-style accumulation; let-chains used throughout (edition 2024).
### own
- clean: seeds ran 2/0/0/0; the two `.clone()` sites build the owned `ProcessStatusReport` in `report()` (`self.profile.provider().clone()`, `ticket.execution_ref().clone()`) and are explainable owned-report construction; no Rc/RefCell/Arc/Cow.
### type
- tail-3#1 sev=low blast=leaf effort=S verdict=actionable - provider-identity validation is duplicated: `MinijailProcessProvider::validate` re-checks selected provider name and provider ref that `launch::validate_launch_ticket` repeats whenever a platform gate is present, so the two can drift apart - fix: drop the two identity checks from `validate_launch_ticket` (keep the gate check; rename it `validate_platform_gate` to disambiguate from the sibling `d2b-provider-process-systemd/src/launch.rs:8` function of the same name with different semantics) and use `crate::PROVIDER_REF` at lib.rs:160 instead of the literal string - [packages/d2b-provider-process-minijail/src/lib.rs:151, packages/d2b-provider-process-minijail/src/lib.rs:160, packages/d2b-provider-process-minijail/src/launch.rs:50, packages/d2b-provider-process-minijail/src/launch.rs:62]
  evidence: seed `fn validate_\w+|fn check_\w+` = 1 hit (`validate_launch_ticket`, launch.rs:46); census: `validate_launch_ticket` over packages/nixos-modules/tests/docs/reference/labs = 3 hits (definition, the single call at lib.rs:193, and the sibling systemd definition)
### api
- clean: seeds ran 15/0/0; surface is the two `PROVIDER_NAME`/`PROVIDER_REF` consts, `MinijailProcessProvider<P: ProcessLaunchEffectPort>` over the injected effect port (the house provider-controller seam), and the `launch`/`adoption` modules; no Arc/Rc/Box/RefCell in signatures; no `pub use` arms.
### err
- clean: seeds ran 2/0/0/0; the two `expect` sites (lib.rs:94, 106) assert frozen compile-time constants ("the frozen provider name is a valid token", "the frozen system-minijail profile is well formed") - the recorded literally-built-value false-positive class; no panics, no swallowed Results, no crate-local error enum (shared `ProcessConformanceError`).
### serde
- N/A: seeds 0/0/0/0 all zero; crate crosses no wire (no serde derives, no serde_json).
### obs
- clean: seeds ran 0/0/0/1; all telemetry is `tracing::warn!`/`debug!` with named fields (`provider`, `resource`, `error`, `identity`), message-only events carry no interpolated data, no println, no secrets in fields (identity digests only).
### docs
- tail-3#2 sev=low blast=leaf effort=S verdict=actionable - public Result-returning items carry no `# Errors` section stating which conditions produce which `ProcessConformanceError` variant - fix: add `# Errors` to `PlatformGate::validate`, `validate_launch_ticket`, `launch_with_inherited_fds`, `adopt`, `stop`, and `stop_stale` (the shared catalog is `d2b_process_conformance::ProcessConformanceError`) - [packages/d2b-provider-process-minijail/src/launch.rs:33, packages/d2b-provider-process-minijail/src/launch.rs:46, packages/d2b-provider-process-minijail/src/lib.rs:313, packages/d2b-provider-process-minijail/src/lib.rs:371, packages/d2b-provider-process-minijail/src/lib.rs:453, packages/d2b-provider-process-minijail/src/lib.rs:475]
  evidence: docs seeds: pub items 13, `/// # (Examples|Errors|Panics|Safety)` 0, `-> Result<` 9 (2 in launch.rs, 7 in lib.rs; 6 of the 9 are public)
### perf
- clean: seeds ran 0/1/0; the single `Vec::new()` is the cold default in `launch()` delegating to `launch_with_inherited_fds` (lib.rs:306); no format! or to_string() in src; static (unmeasured).
### conc
- N/A: seeds 0/0/0/0 all zero; no threads, locks, atomics, or manual Send/Sync.
### async
- clean: seeds ran 19/0/0/0; `launch`/`adopt`/`stop`/`stop_stale` hold no locks across `.await`, spawn nothing, and do no blocking work; tests use the sanctioned plain `#[test]` + `d2b_process_conformance::testing::block_on` harness.
### unsafe
- N/A: seeds 0/0/0/0; manifest forbids `unsafe_code`.
### ffi
- N/A: seeds 0/0/0/0 all zero.
### macro
- N/A: seeds 0/0/0/0 all zero.
### test
- clean: seeds ran 22/52/0/0; conformance.rs, execution_parents.rs, and platform_gate.rs assert behavior through `ScriptedEffectPort` call sequences (`PortCall::Observe`/`OpenPidfd`/`Stop`) and outcome variants, not implementation; deterministic, no network, no ignored tests; the shared `suite::` assertions plus minijail-specific pidfd/wait-ownership cells cover the fail-closed paths.
### supply
- clean: deps d2b-contracts-resource, d2b-process-conformance, tracing all appear in src/; no unused deps (X1 owns workspace-level supply).

## Coverage
- idiom: clean (seeds ran: 0/0/0)
- own: clean (seeds ran: 2/0/0/0)
- type: 1 finding(s)
- api: clean (seeds ran: 15/0/0)
- err: clean (seeds ran: 2/0/0/0)
- serde: N/A (seeds: 0/0/0/0 all zero; no wire crossing)
- obs: clean (seeds ran: 0/0/0/1)
- docs: 1 finding(s)
- perf: clean (seeds ran: 0/1/0)
- conc: N/A (seeds: 0/0/0/0 all zero)
- async: clean (seeds ran: 19/0/0/0)
- unsafe: N/A (seeds: 0/0/0/0 all zero)
- ffi: N/A (seeds: 0/0/0/0 all zero)
- macro: N/A (seeds: 0/0/0/0 all zero)
- test: clean (seeds ran: 22/52/0/0)
- supply: clean (per-crate dep check; X1 owns workspace level)

## d2b-provider-quota
### idiom
- clean: seeds ran 0/0/0; no index loops, hand-written impls, or statement-style accumulation; derives carry the value type.
### own
- clean: seeds ran 0/0/0/0; no clones, no shared ownership; applicable because the accessors take `&self`.
### type
- clean: seeds ran 0/0/0; `QuotaStatusResource` is a read-only status shape with private fields and const accessors; no boolean-flag or stringly-typed state; applicable because the crate declares a struct.
### api
- tail-3#3 sev=low blast=family effort=S verdict=actionable - the `test-support` feature is declared empty and gates nothing: the `quota` module is unconditionally `pub`, and no manifest enables the feature - fix: delete the `[features] test-support = []` stanza (or gate the test-consumed exports behind it, matching the house pattern of feature-gated test-support) - [packages/d2b-provider-quota/Cargo.toml:17, packages/d2b-provider-quota/src/lib.rs:21]
  evidence: census: `test-support` over packages manifests = declared empty in 4 tail-lane crates (quota/role/resource-export/resource-import Cargo.toml:17), enabled by no manifest (d2bd enables it for 20+ provider crates, not these); prior audit deferred the class: docs/explanation/over-engineering-audit-record.md:916
### err
- clean: seeds ran 0/0/0/0; no unwrap/expect/panic, no swallowed Results; applicable because fns exist, and the panic policy is trivially clean.
### serde
- clean: seeds ran 1/1/0/0; `QuotaStatusResource` derives Serialize/Deserialize/JsonSchema with `rename_all = "camelCase"` + `deny_unknown_fields`; it is daemon output (status), so no try_from validation is owed; the shape is exercised by d2b-resource-api manager_backend tests (census: 2 hits there).
### obs
- N/A: seeds 0/0/0/0 all zero; no tracing/log dependency.
### docs
- clean: seeds ran 6/0/0; every pub item (struct, three accessors, `QuotaStatus` alias, `quota_descriptor`) documented under `#![deny(missing_docs)]`; module docs present in lib.rs, driver.rs, quota.rs.
### perf
- N/A: seeds 0/0/0 all zero.
### conc
- N/A: seeds 0/0/0/0 all zero.
### async
- N/A: seeds 0/0/0/0 all zero; the only async is the dev-dependency `#[tokio::test]` registration test, out of crate scope.
### unsafe
- N/A: seeds 0/0/0/0; manifest forbids `unsafe_code`.
### ffi
- N/A: seeds 0/0/0/0 all zero.
### macro
- N/A: seeds 0/0/0/0 all zero.
### test
- clean: seeds ran 1/0/0/0; the single test is the policy-required registration-test pattern calling the shared `assert_metadata_registration` (recorded refusal class; not flagged).
### supply
- tail-3#4 sev=low blast=leaf effort=S verdict=actionable - `serde_json` is a declared dependency but appears nowhere in the crate's src/ or tests/ - fix: drop `serde_json.workspace = true` from [dependencies] - [packages/d2b-provider-quota/Cargo.toml:24]
  evidence: census: `serde_json` over packages/d2b-provider-quota = 1 hit, the manifest line itself; zero hits in src/ and tests/ (the resource-api consumer test uses its own serde_json)

## Coverage
- idiom: clean (seeds ran: 0/0/0)
- own: clean (seeds ran: 0/0/0/0)
- type: clean (seeds ran: 0/0/0)
- api: 1 finding(s)
- err: clean (seeds ran: 0/0/0/0)
- serde: clean (seeds ran: 1/1/0/0)
- obs: N/A (seeds: 0/0/0/0 all zero; no tracing/log dependency)
- docs: clean (seeds ran: 6/0/0)
- perf: N/A (seeds: 0/0/0 all zero)
- conc: N/A (seeds: 0/0/0/0 all zero)
- async: N/A (seeds: 0/0/0/0 all zero)
- unsafe: N/A (seeds: 0/0/0/0 all zero)
- ffi: N/A (seeds: 0/0/0/0 all zero)
- macro: N/A (seeds: 0/0/0/0 all zero)
- test: clean (seeds ran: 1/0/0/0)
- supply: 1 finding(s)

## d2b-provider-resource-export
### idiom
- clean: seeds ran 0/0/0; single descriptor fn, no loops or accumulation.
### own
- N/A: seeds 0/0/0/0 all zero; no fn takes parameters.
### type
- N/A: seeds 0/0/0 all zero; no struct or enum declared.
### api
- tail-3#5 sev=low blast=family effort=S verdict=actionable - the `test-support` feature is declared empty and gates nothing: the crate exports only `resource_export_descriptor` unconditionally, and no manifest enables the feature - fix: delete the `[features] test-support = []` stanza - [packages/d2b-provider-resource-export/Cargo.toml:17, packages/d2b-provider-resource-export/src/lib.rs:18]
  evidence: census: `test-support` over packages manifests = declared empty in 4 tail-lane crates, enabled by no manifest; prior audit deferred the class: docs/explanation/over-engineering-audit-record.md:916
### err
- clean: seeds ran 0/0/0/0; no panic sites, no swallowed Results; applicable because `resource_export_descriptor` exists.
### serde
- N/A: seeds 0/0/0/0 all zero; crate crosses no wire.
### obs
- N/A: seeds 0/0/0/0 all zero; no tracing/log dependency.
### docs
- clean: seeds ran 1/0/0; `resource_export_descriptor` documented, module docs in lib.rs and driver.rs, `#![deny(missing_docs)]` active.
### perf
- N/A: seeds 0/0/0 all zero.
### conc
- N/A: seeds 0/0/0/0 all zero.
### async
- N/A: seeds 0/0/0/0 all zero; the only async is the dev-dependency `#[tokio::test]` registration test, out of crate scope.
### unsafe
- N/A: seeds 0/0/0/0; manifest forbids `unsafe_code`.
### ffi
- N/A: seeds 0/0/0/0 all zero.
### macro
- N/A: seeds 0/0/0/0 all zero.
### test
- clean: seeds ran 1/0/0/0; the single test is the policy-required registration-test pattern calling the shared `assert_metadata_registration` (recorded refusal class; not flagged).
### supply
- clean: the only dep, d2b-resource-types, appears in src/; no unused deps.

## Coverage
- idiom: clean (seeds ran: 0/0/0)
- own: N/A (seeds: 0/0/0/0 all zero; no fn takes parameters)
- type: N/A (seeds: 0/0/0 all zero; no struct or enum declared)
- api: 1 finding(s)
- err: clean (seeds ran: 0/0/0/0)
- serde: N/A (seeds: 0/0/0/0 all zero; no wire crossing)
- obs: N/A (seeds: 0/0/0/0 all zero; no tracing/log dependency)
- docs: clean (seeds ran: 1/0/0)
- perf: N/A (seeds: 0/0/0 all zero)
- conc: N/A (seeds: 0/0/0/0 all zero)
- async: N/A (seeds: 0/0/0/0 all zero)
- unsafe: N/A (seeds: 0/0/0/0 all zero)
- ffi: N/A (seeds: 0/0/0/0 all zero)
- macro: N/A (seeds: 0/0/0/0 all zero)
- test: clean (seeds ran: 1/0/0/0)
- supply: clean (per-crate dep check; X1 owns workspace level)

## d2b-provider-resource-import
### idiom
- clean: seeds ran 0/0/0; single descriptor fn, no loops or accumulation.
### own
- N/A: seeds 0/0/0/0 all zero; no fn takes parameters.
### type
- N/A: seeds 0/0/0 all zero; no struct or enum declared.
### api
- tail-3#6 sev=low blast=family effort=S verdict=actionable - the `test-support` feature is declared empty and gates nothing: the crate exports only `resource_import_descriptor` unconditionally, and no manifest enables the feature - fix: delete the `[features] test-support = []` stanza - [packages/d2b-provider-resource-import/Cargo.toml:17, packages/d2b-provider-resource-import/src/lib.rs:18]
  evidence: census: `test-support` over packages manifests = declared empty in 4 tail-lane crates, enabled by no manifest; prior audit deferred the class: docs/explanation/over-engineering-audit-record.md:916
### err
- clean: seeds ran 0/0/0/0; no panic sites, no swallowed Results; applicable because `resource_import_descriptor` exists.
### serde
- N/A: seeds 0/0/0/0 all zero; crate crosses no wire.
### obs
- N/A: seeds 0/0/0/0 all zero; no tracing/log dependency.
### docs
- clean: seeds ran 1/0/0; `resource_import_descriptor` documented, module docs in lib.rs and driver.rs, `#![deny(missing_docs)]` active.
### perf
- N/A: seeds 0/0/0 all zero.
### conc
- N/A: seeds 0/0/0/0 all zero.
### async
- N/A: seeds 0/0/0/0 all zero; the only async is the dev-dependency `#[tokio::test]` registration test, out of crate scope.
### unsafe
- N/A: seeds 0/0/0/0; manifest forbids `unsafe_code`.
### ffi
- N/A: seeds 0/0/0/0 all zero.
### macro
- N/A: seeds 0/0/0/0 all zero.
### test
- clean: seeds ran 1/0/0/0; the single test is the policy-required registration-test pattern calling the shared `assert_metadata_registration` (recorded refusal class; not flagged).
### supply
- clean: the only dep, d2b-resource-types, appears in src/; no unused deps.

## Coverage
- idiom: clean (seeds ran: 0/0/0)
- own: N/A (seeds: 0/0/0/0 all zero; no fn takes parameters)
- type: N/A (seeds: 0/0/0 all zero; no struct or enum declared)
- api: 1 finding(s)
- err: clean (seeds ran: 0/0/0/0)
- serde: N/A (seeds: 0/0/0/0 all zero; no wire crossing)
- obs: N/A (seeds: 0/0/0/0 all zero; no tracing/log dependency)
- docs: clean (seeds ran: 1/0/0)
- perf: N/A (seeds: 0/0/0 all zero)
- conc: N/A (seeds: 0/0/0/0 all zero)
- async: N/A (seeds: 0/0/0/0 all zero)
- unsafe: N/A (seeds: 0/0/0/0 all zero)
- ffi: N/A (seeds: 0/0/0/0 all zero)
- macro: N/A (seeds: 0/0/0/0 all zero)
- test: clean (seeds ran: 1/0/0/0)
- supply: clean (per-crate dep check; X1 owns workspace level)

## d2b-provider-role
### idiom
- clean: seeds ran 0/0/0; no index loops, no hand-written impls beyond the deliberate `Debug` for `PositiveDecisionCache` (lock-free diagnostics, documented and tested), no statement-style accumulation.
### own
- clean: seeds ran 0/0/0/0; no clones, no shared ownership; applicable because the cache fns take `&self`/`&key`.
### type
- clean: seeds ran 0/0/0; `PolicyRevisionSet`/`AuthorizationCacheKey`/`PositiveEntry`/`PositiveDecisionCache` model the revision-bound positive-only cache with no boolean-flag or stringly-typed state; applicable because the crate declares structs.
### api
- tail-3#8 sev=low blast=family effort=S verdict=actionable - the `test-support` feature is declared empty and gates nothing: `rbac` is unconditionally `pub`, and no manifest enables the feature - fix: delete the `[features] test-support = []` stanza - [packages/d2b-provider-role/Cargo.toml:17, packages/d2b-provider-role/src/lib.rs:16]
  evidence: census: `test-support` over packages manifests = declared empty in 4 tail-lane crates, enabled by no manifest; prior audit deferred the class: docs/explanation/over-engineering-audit-record.md:916
### err
- clean: seeds ran 5/0/0/0; all five unwrap hits sit inside `#[cfg(test)] mod tests` in rbac.rs (fixture refs and generations), the recorded test false-positive class; the production lock path handles poison explicitly (`unwrap_or_else` clearing the entries).
### serde
- N/A: seeds 0/0/0/0 all zero; crate crosses no wire.
### obs
- N/A: seeds 0/0/0/0 all zero; no tracing/log dependency.
### docs
- tail-3#7 sev=low blast=leaf effort=S verdict=actionable - role's lib.rs lacks the `#![deny(missing_docs)]` gate that its four sibling declaration crates in this lane all carry (quota lib.rs:16, resource-export lib.rs:14, resource-import lib.rs:14, minijail lib.rs:25), and `PolicyRevisionSet`'s four pub fields are undocumented - fix: add `#![deny(missing_docs)]` to lib.rs and document the `PolicyRevisionSet` fields - [packages/d2b-provider-role/src/lib.rs:1, packages/d2b-provider-role/src/rbac.rs:11]
  evidence: docs seeds: pub items 9, `/// # (Examples|Errors|Panics|Safety)` 0, `-> Result<` 0; grep `deny\(missing_docs\)` = 0 hits in role vs 1 each in the four sibling crates
### perf
- clean: seeds ran 3/1/0; the format! hits are test-only (redaction sentinel checks), the single `BTreeMap::new()` is the cache constructor; bounded cache with retain-on-access, no hot-path allocation; static (unmeasured).
### conc
- clean: seeds ran 0/1/0/0; the `std::sync::Mutex` behind `PositiveDecisionCache` carries the sanctioned `#[allow(clippy::disallowed_methods, reason = "synchronous path")]` (U1 (d)4 sanctioned reason), and the `Debug` impl deliberately uses `try_lock` so diagnostics never block; no atomics, no unsafe Send/Sync.
### async
- N/A: seeds 0/0/0/0 all zero; the only async is the dev-dependency `#[tokio::test]` registration test, out of crate scope.
### unsafe
- N/A: seeds 0/0/0/0; manifest forbids `unsafe_code`.
### ffi
- N/A: seeds 0/0/0/0 all zero.
### macro
- N/A: seeds 0/0/0/0 all zero; `redacted_debug!` is an invocation of the contracts-crate macro, not a definition.
### test
- clean: seeds ran 3/8/0/0; rbac unit tests assert the redaction contract (sentinel absence, exact Debug string) and expiry/revision-invalidation behavior with hand-written expectations; registration test is the policy-required pattern; deterministic, no ignored tests.
### supply
- clean: deps d2b-contracts-resource and d2b-resource-types both appear in src/; no unused deps.

## Coverage
- idiom: clean (seeds ran: 0/0/0)
- own: clean (seeds ran: 0/0/0/0)
- type: clean (seeds ran: 0/0/0)
- api: 1 finding(s)
- err: clean (seeds ran: 5/0/0/0)
- serde: N/A (seeds: 0/0/0/0 all zero; no wire crossing)
- obs: N/A (seeds: 0/0/0/0 all zero; no tracing/log dependency)
- docs: 1 finding(s)
- perf: clean (seeds ran: 3/1/0)
- conc: clean (seeds ran: 0/1/0/0)
- async: N/A (seeds: 0/0/0/0 all zero)
- unsafe: N/A (seeds: 0/0/0/0 all zero)
- ffi: N/A (seeds: 0/0/0/0 all zero)
- macro: N/A (seeds: 0/0/0/0 all zero)
- test: clean (seeds ran: 3/8/0/0)
- supply: clean (per-crate dep check; X1 owns workspace level)