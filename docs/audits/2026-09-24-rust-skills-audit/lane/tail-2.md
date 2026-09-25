# tail-2 - tail lane
Baseline: 6ebdd4cec22e6537d1376e83ff7a82b00a8492c0 | LOC audited: 2457 (excl. src/generated/**) | modules: d2b-provider-audio-service, d2b-provider-command, d2b-provider-device, d2b-provider-emergency-policy, d2b-provider-operation
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: whole crates

## d2b-provider-audio-service

### idiom
- clean: seeds s1 index loop = 0, s2 `impl (Default|From|...) for` = 0, s3 `let mut ... = String/Vec::new()` = 0; one declaration struct + thin descriptor fns, no hand-written derives, no statement-style accumulation.

### own
- clean: seeds 1-4 all zero (clone/to_owned/to_vec/to_string, Rc/RefCell/Arc<Mutex/Arc<RwLock<, Cow) over src; no clones of any kind; `Arc<dyn SpecDecoder>` in `audio_service_spec_decoder` is the required trait-object return.

### type
- clean: seeds 1-3 all zero (`fn validate_/check_`, `is_x: bool`/`x_flag: bool`, stringly `mode/kind/state`); unit struct `AudioService` carries no state, validation is delegated to the typed spec decode in the family engine.

### api
- clean: seed 1 pub surface = 8 hits (AUDIO_SERVICE_PROVIDER_REF, AUDIO_SERVICE_RESYNC, AudioService, AudioServiceFactory, audio_service_spec_decoder, audio_service_descriptor, lib.rs re-export arms) - single re-export path, every item documented, no Arc/Rc/Box/RefCell/dependency types in signatures beyond the shared `InteractionDriverArgs<AudioService>` family-engine args (house interaction pattern, `d2b-provider-wayland-policy`).

### err
- clean: seeds 1-4 all zero outside tests (no unwrap/expect, no panic!/unreachable!/todo!/unimplemented!, no error enum); `validate` returns `Result` and never panics on row input.

### serde
- N/A: seeds 1-4 all zero (no serde derives, no serde attributes, no hand-written Deserialize, no serde_json calls); the crate crosses no wire of its own - `AudioServiceSpec` decode lives in `d2b-provider-wayland-policy@interaction::spec_decoder`.

### obs
- N/A: seeds 1-4 all zero (no println!/eprintln!, no no-field message events, no instrument spans, no tracing/log) and Cargo.toml carries no tracing/log dependency.

### docs
- clean: seed 1 pub items (six) + seed 3 `-> Result<` (three trait methods) = 9 hits, all public items documented under `#![deny(missing_docs)]` in lib.rs; module header explains the family split.

### perf
- clean: seeds 1-3 = 2 hits (`Ok(Vec::new())` in `dependencies`/`desired_children`) - both are the deliberate empty desired-child set, empty case is the only case.

### conc
- N/A: seeds 1-4 all zero (no threads, no Mutex/RwLock, no atomics, no thread_local/unsafe Send-Sync).

### async
- N/A: seeds 1-4 all zero (no async fn, no spawn/select/join, no tokio sync types, no tokio main/test) over src; the registration test is plain `#[test]`.

### unsafe
- N/A: seeds 1-4 all zero (no unsafe blocks/fns, no SAFETY comments, no transmute/from_raw/MaybeUninit/zeroed) and manifest declares `unsafe_code = "forbid"`.

### ffi
- N/A: seeds 1-4 all zero (no extern "C"/no_mangle, no catch_unwind, no repr(C)/repr(transparent), no CStr/CString/c_char).

### macro
- N/A: seeds 1-4 all zero (no macro_rules!, no proc_macro/syn/quote, no `$crate`, no to_compile_error/new_spanned).

### test
- clean: seeds over src+tests = 17 hits (4 `#[test]`, 13 `assert*`); registration tests pin the declaration, the required Provider selector, duplicate-type refusal, spec decode, and foreign-row refusal - each fails on a concrete regression.

### Coverage
- idiom: clean (0/0/0)
- own: clean (0/0/0/0)
- type: clean (0/0/0)
- api: clean (8 hits)
- err: clean (0/0/0/0)
- serde: N/A (seeds: 0/0/0/0 all zero; crate crosses no wire)
- obs: N/A (seeds: 0/0/0/0 all zero; no tracing/log dep)
- docs: clean (9 hits)
- perf: clean (2 hits; deliberate empty returns)
- conc: N/A (seeds: 0/0/0/0 all zero)
- async: N/A (seeds: 0/0/0/0 all zero)
- unsafe: N/A (seeds: 0/0/0/0 all zero)
- ffi: N/A (seeds: 0/0/0/0 all zero)
- macro: N/A (seeds: 0/0/0/0 all zero)
- test: clean (17 hits: 4 #[test], 13 asserts)

## d2b-provider-command

### idiom
- clean: seeds 1-3 all zero; constructor-style parsing with `if let`/`else` and `strip_prefix`/`strip_suffix` chains reads idiomatic; `format!("{{{name}}}")` is the canonical-brace normalization.

### own
- clean: seeds 1-4 = 6 hits (3 `.to_owned()` in `JsonSchema::schema_name`/`pattern` at the schemars trait boundary which demands owned values, 2 `.clone()` in `#[cfg(test)]` fixtures, 1 `.to_owned()` in JsonSchema metadata) - every hit is a required ownership transfer at a trait boundary or test fixture, none avoids a borrow.

### type
- clean: seeds 1-3 all zero; `CommandExec`/`CommandArgvSlot` are parse-once private-field newtypes and `CommandSpec::new` validates cross-field invariants once at construction - the pattern this lens names as the goal.

### api
- clean: seed 1 pub surface = 27 hits (constants, four newtypes, CommandIntent, CommandSpec, CommandContractError, command_descriptor, `pub use` arms); single re-export path (`pub mod command` + `pub use command::*` is the house spec-vocabulary pattern), fields private with accessors, no Arc/Rc/Box/RefCell or dependency types in signatures.

### err
- clean: seeds 1-3 = 0 outside tests (all unwrap/expect live in `#[cfg(test)]`), seed 4 error enum = 1 (`CommandContractError`); the four variants split by caller action (exec vs argv vs role vs placeholder), each documented, deserialize failures surface as serde errors, never panics.

### serde
- tail-2#1 sev=medium blast=leaf effort=S verdict=needs-contract - the emitted JsonSchema for `CommandExec` (`pattern: "^/[^\\u0000]*$"`) and `CommandArgvSlot` (no pattern) is weaker than the parse admission (rejects control chars/empty/malformed braces/invalid placeholder names), so a value satisfying the published schema can be refused at serde deserialization - fix: tighten the `CommandExec` pattern to exclude `char::is_control` code points and add a `CommandArgvSlot` pattern (or an explicit `format`/`pattern` encoding the whole-slot brace rule), then regenerate the committed schema - [packages/d2b-provider-command/src/command.rs:64-80, packages/d2b-provider-command/src/command.rs:133-152, docs/reference/schemas/v3/core.d2bus.org_Command.schema.json:21]
  evidence: serde seed 2 `serde\((rename_all|...|pattern...)` = 0 but seed 3 `impl .*Deserialize.*for` = 2 plus seed 1 derive = 2; divergence verified against the emitted, committed schema `docs/reference/schemas/v3/core.d2bus.org_Command.schema.json` definitions.CommandExec (`pattern "^/[^\\u0000]*$"`) and definitions.CommandArgvSlot (no pattern), generated from these impls; generated-schema text is a wire contract surface.
- clean: other than tail-2#1 - hand-written `Deserialize` for CommandExec/CommandArgvSlot/CommandSpec are live admission gates on a wire shape (recorded refusal class (d) 6), `deny_unknown_fields` on the `Wire` shape and `CommandIntent`, `#[serde(transparent)]` round-trips; seed counts: seed1 = 2, seed2 = 5, seed3 = 3, seed4 = 0.

### obs
- N/A: seeds 1-4 all zero and Cargo.toml carries no tracing/log dependency.

### docs
- tail-2#2 sev=low blast=leaf effort=S verdict=actionable - public `Result`-returning constructors `CommandExec::parse`, `CommandArgvSlot::parse`, and `CommandSpec::new` return `Result<..., CommandContractError>` without an `# Errors` section naming which variants each can produce, though callers match on them (tests assert exact variants) - fix: add a one-line `# Errors` per constructor naming its `CommandContractError` variants - [packages/d2b-provider-command/src/command.rs:38, packages/d2b-provider-command/src/command.rs:89, packages/d2b-provider-command/src/command.rs:192]
  evidence: docs seed 3 `-> Result<` = 6 hits over the three public constructors plus validation methods; seed 1 public items = 27 hits, all documented (missing_docs denied) except the canonical-section gap.
- clean: otherwise all public items carry one-line first sentences and module header explains the launch-shape contract.

### perf
- clean: seeds 1-3 = 1 hit (`format!("{{{name}}}")` in `CommandArgvSlot::parse`) - argument-slot canonicalization runs at declaration admission once, not in a loop or hot path; no other allocation sites.

### conc
- N/A: seeds 1-4 all zero (no threads, no Mutex/RwLock, no atomics, no thread_local/unsafe Send-Sync) over src.

### async
- N/A: seeds 1-4 all zero over src (no async fn, no tokio spawn/select/join, no tokio sync, no tokio main/test); the only async surface is the `#[tokio::test]` registration scaffold in tests, which owns no state.

### unsafe
- N/A: seeds 1-4 all zero and manifest declares `unsafe_code = "forbid"`.

### ffi
- N/A: seeds 1-4 all zero.

### macro
- N/A: seeds 1-4 all zero; the `redacted_debug!` uses are imported macros from `d2b-contracts-resource` (deliberate exported-API macros, not definitions here).

### test
- clean: seeds over src+tests = 14 hits (4 `#[test]`, 1 `#[tokio::test]` registration, 9 `assert*`); unit tests pin placeholder resolution, undeclared-placeholder refusal, brace/exec malformation refusal, role-type refusal, and a canonical round trip with unknown-field refusal - each fails on a real regression; the registration test is the documented 13-17-line shared-assertion pattern (do not flag).

### Coverage
- idiom: clean (0/0/0)
- own: clean (6 hits; all trait-boundary/fixture)
- type: clean (0/0/0)
- api: clean (27 hits)
- err: clean (0/0/0 outside tests; 1 error enum)
- serde: 1 finding (10 hits)
- obs: N/A (seeds: 0/0/0/0 all zero; no tracing/log dep)
- docs: 1 finding (30 hits)
- perf: clean (1 hit; cold admission path)
- conc: N/A (seeds: 0/0/0/0 all zero)
- async: N/A (seeds: 0/0/0/0 all zero over src)
- unsafe: N/A (seeds: 0/0/0/0 all zero)
- ffi: N/A (seeds: 0/0/0/0 all zero)
- macro: N/A (seeds: 0/0/0/0 all zero)
- test: clean (14 hits: 4 #[test], 1 #[tokio::test], 9 asserts)

## d2b-provider-device

### idiom
- clean: seeds 1-3 all zero; match-based `declared_dependency_refs`, const-fn descriptor building, and async-trait delegation read idiomatic; no index loops, hand-written derives, or statement accumulation.

### own
- clean: seeds 1-4 = 3 hits (2 `.to_owned()` in the `inspect-device-response-invalid` error path, 1 `Arc<Mutex<...>>` in `DeviceResourceState`); the to_owned builds a cold error message, and the Arc-guarded caches are genuine shared ownership between the driver family and the daemon runtime - the four-question test passes; the `parking_lot::Mutex` choice is contract-justified (see api tail-2#3).

### type
- clean: seeds 1-3 all zero; `DeviceComponent` is a closed four-variant vocab for the declared family, `DeviceResourceState` is a plain state bag, no boolean flags or stringly state.

### api
- tail-2#3 sev=medium blast=family effort=L verdict=actionable - `DeviceResourceState` exposes raw `Arc<Mutex<BTreeMap<...>>>` and `Arc<parking_lot::Mutex<BTreeMap<[u8; 16], ...>>>` as pub fields, leaking wrapper types and the `parking_lot` dependency into the crate's public API (parking_lot is banned outright by (d) 2 outside the R4 worker boundary; this site is comment-justified only, driver.rs:137-138, matching the GPU crate's cache at d2b-provider-device-gpu/src/effects_service.rs:79) - fix: make the three caches private and expose narrow typed accessor methods on `DeviceResourceState` (or an effects-owned registry handle), keeping the GPU authority-lease construction contract behind the crate, and migrate the nine daemon read sites - [packages/d2b-provider-device/src/driver.rs:128-143, packages/d2bd/src/shared_provider_effects.rs:1584, packages/d2bd/src/shared_provider_effects.rs:2092, packages/d2bd/src/shared_provider_effects.rs:2122]
  evidence: api seed 2 `pub .*\b(Arc|Rc|Box|RefCell)<` = 3 hits (tpm_controllers, gpu_controllers, gpu_authority_leases); census: `tpm_controllers|gpu_controllers|gpu_authority_leases` over packages = 9 read sites in packages/d2bd/src/shared_provider_effects.rs (1584, 1632, 1657, 2092, 2122, 2479, 2512, 2568, 2586) - the field access is genuinely needed across the crate boundary; parking_lot (d) 2 ban context cited.
- clean: otherwise seed 1 pub surface = 31 hits, single re-export path (lib.rs `pub use driver::{...}` + `pub use effects_service::DEVICE_EFFECTS_SERVICE`), documented under `#![deny(missing_docs)]`, `DeviceComponents`/trait seams carry no other wrapper or dependency types.

### err
- clean: seeds 1-4 all zero outside tests (no unwrap/expect, no panic!/unreachable!/todo!, no error enum); `inspect_device_response` maps the trusted-static-json refusal to a named `EffectServiceError::Declined` reason instead of swallowing, and the error `map_err` names its own code.

### serde
- clean: seeds 1-4 = 1 hit (`serde_json::from_value` in `inspect_device_response`) - a static literal payload rendered through the canonical JSON path, not an untrusted-input admission gate; no wire types deserialize in this crate.

### obs
- N/A: seeds 1-4 all zero and Cargo.toml carries no tracing/log dependency; all daemon-side telemetry lives behind the facet trait in d2bd.

### docs
- clean: seed 1 pub items + seed 3 = 35 hits; every pub item, pub field, and trait method documented under `#![deny(missing_docs)]`; the dead-code `facets` carry in `DeviceEffectsServiceFactory` is documented as the unwired R5 respawn contract (refused scaffolding class (d) 6 - cited, not re-flagged).

### perf
- clean: seeds 1-3 = 1 hit (`Vec::new()` in `declared_dependency_refs` for the Usbip/SecurityKey arms) - the deliberate empty dependency set; no format!/string copies in cold or hot paths.

### conc
- clean: seeds 1-3 = 3 hits (two `Arc<Mutex<BTreeMap>>`, one `parking_lot::Mutex` scope) - the shared-state model is the documented daemon/driver seam over per-resource caches; guards are std/sync, held briefly, never across an `.await` (`await_holding_lock` is denied at the workspace); the `async-gate-allow: test-support recorder lock` markers in test_support.rs:37,49 are recorded exceptions (d) 3 - cited, not re-flagged; the parking_lot dependency leak is carried by api tail-2#3.

### async
- clean: seeds 1-4 = 16 hits over src (async fn trait methods + `.await` delegation) - the effects delegate to the `DeviceRuntime` facet trait, no blocking work runs in src async bodies, no tokio sync guard spans an await, and `#[async_trait]` bounds the Send/Sync claims on `DeviceDriverEffects`/`DeviceRuntime`; test recorder locks carry their (d) 3 markers.

### unsafe
- N/A: seeds 1-4 all zero and manifest declares `unsafe_code = "forbid"`.

### ffi
- N/A: seeds 1-4 all zero.

### macro
- N/A: seeds 1-4 all zero.

### test
- clean: seeds over src+tests = 9 hits (1 `#[test]`, 1 `#[tokio::test]`, 7 `assert*`; the `integration/device_family.rs` scaffold is policy-required (d) 5 and out of the test lane's src+tests scope); tests pin the four realizer provider identities, per-row resync, component-dispatch dispatch, and foreign-provider terminal failure - each fails on a real regression; `test-support` feature gating is the house pattern.

### Coverage
- idiom: clean (0/0/0)
- own: clean (3 hits; shared-state seam + cold error paths)
- type: clean (0/0/0)
- api: 1 finding (31 hits)
- err: clean (0/0/0/0)
- serde: clean (1 hit; static payload only)
- obs: N/A (seeds: 0/0/0/0 all zero; no tracing/log dep)
- docs: clean (35 hits)
- perf: clean (1 hit; deliberate empty set)
- conc: clean (3 hits; documented seam, no guard across await)
- async: clean (16 hits)
- unsafe: N/A (seeds: 0/0/0/0 all zero)
- ffi: N/A (seeds: 0/0/0/0 all zero)
- macro: N/A (seeds: 0/0/0/0 all zero)
- test: clean (9 hits: 1 #[test], 1 #[tokio::test], 7 asserts)

## d2b-provider-emergency-policy

### idiom
- clean: seeds 1-3 all zero; the single descriptor fn is a one-liner delegating to `metadata_descriptor`.

### own
- N/A: seeds 1-4 all zero and no fn takes parameters (`emergency_policy_descriptor()`).

### type
- N/A: seeds 1-3 all zero and the crate declares no struct or enum (driver fn only).

### api
- clean: seed 1 pub surface = 2 hits (`emergency_policy_descriptor` + lib.rs re-export arm) - one documented pub fn, single re-export path, no wrappers or dependency types.

### err
- clean: seeds 1-4 all zero; the only fn builds a descriptor and cannot panic on input.

### serde
- N/A: seeds 1-4 all zero (crate crosses no wire; the metadata conversion owns no spec shape).

### obs
- N/A: seeds 1-4 all zero and Cargo.toml carries no tracing/log dependency.

### docs
- clean: seed 1 = 1 hit (`pub fn emergency_policy_descriptor`, documented under `#![deny(missing_docs)]`); module header explains the metadata-only conversion.

### perf
- N/A: seeds 1-3 all zero.

### conc
- N/A: seeds 1-4 all zero.

### async
- N/A: seeds 1-4 all zero.

### unsafe
- N/A: seeds 1-4 all zero and manifest declares `unsafe_code = "forbid"`.

### ffi
- N/A: seeds 1-4 all zero.

### macro
- N/A: seeds 1-4 all zero.

### test
- clean: seeds over src+tests = 1 hit (`#[tokio::test]` registration calling the shared `assert_metadata_registration` - the documented 13-17-line pattern, do not flag as trivial).

### Coverage
- idiom: clean (0/0/0)
- own: N/A (seeds: 0/0/0/0 all zero; no fn parameters)
- type: N/A (seeds: 0/0/0 all zero; declares no struct/enum)
- api: clean (2 hits)
- err: clean (0/0/0/0)
- serde: N/A (seeds: 0/0/0/0 all zero)
- obs: N/A (seeds: 0/0/0/0 all zero; no tracing/log dep)
- docs: clean (1 hit)
- perf: N/A (seeds: 0/0/0 all zero)
- conc: N/A (seeds: 0/0/0/0 all zero)
- async: N/A (seeds: 0/0/0/0 all zero)
- unsafe: N/A (seeds: 0/0/0/0 all zero)
- ffi: N/A (seeds: 0/0/0/0 all zero)
- macro: N/A (seeds: 0/0/0/0 all zero)
- test: clean (1 hit: 1 #[tokio::test] registration)

## d2b-provider-operation

### idiom
- clean: seeds 1-3 = 1 hit (`impl Default for OperationBounds`) - the hand-written Default is the sanctioned invariant-preserving case (a field-wise derive would produce 0/0/0, which `OperationBounds::new` rejects), and it shares its value source with the serde `default =` fns; no index loops or statement accumulation.

### own
- clean: seeds 1-4 = 1 hit (`.to_owned()` in a `#[cfg(test)]` assertion's expected string) - cold test-only, no production clones or refcounts.

### type
- clean: seeds 1-3 all zero; every facet is a private-field struct with a validating constructor, `owner_ref`/`wire_tag` mutual exclusion (a two-Option pair the lens warns about) is enforced in `OperationSpec::new` and the type is a wire-pinned manifest shape - restructuring it is needs-contract with no new bug class covered, so left as validated state.

### api
- clean: seed 1 pub surface = 68 hits (six facet structs, seven closed enums, OperationSpec, OperationContractError, eight bounds consts) - the deliberate wide wire vocabulary that IS the contract (api card false positive), single re-export path (`pub mod operation` + `pub use operation::*` house pattern), fields private with accessors, `Copy` where the enum has no payload; `OperationSpec::new`'s 11 arguments are `#[allow(clippy::too_many_arguments)]`-recorded and constructed once from admission, a builder would be over-engineering.

### err
- clean: seeds 1-3 = 0 outside tests (all unwrap/expect in `#[cfg(test)]`), seed 4 = 1 (`OperationContractError`, eight variants); each variant is a distinct caller-action rejection of a declaration, all documented, constructors return `Result` and never panic on input, `unwrap_or_default` on `.split(':').next()` is infallible by construction.

### serde
- clean: seeds 1-4 = 49 hits; the hand-written `Deserialize for OperationSpec` over a `deny_unknown_fields` `Wire` shape calling `Self::new` is the recorded live admission-gate pattern ((d) 6, recorded refusal class - cited, not re-flagged); kebab-case external enums are consistent, `deny_unknown_fields` on every facet, `skip_serializing_if = "Option::is_none"` round-trips (test asserts `wireTag` is absent), `serde(default = ...)` backs `OperationBounds`.

### obs
- N/A: seeds 1-4 all zero and Cargo.toml carries no tracing/log dependency.

### docs
- tail-2#4 sev=low blast=leaf effort=S verdict=actionable - public `Result`-returning constructors `OperationAudit::new`, `AuditJoin::new`, `OperationFds::new`, `OperationBounds::new`, and `OperationSpec::new` return `Result<..., OperationContractError>` without `# Errors` sections naming which variants each can produce, though callers match exact variants (tests assert them) - fix: add a one-line `# Errors` per constructor naming its `OperationContractError` variants - [packages/d2b-provider-operation/src/operation.rs:138, packages/d2b-provider-operation/src/operation.rs:194, packages/d2b-provider-operation/src/operation.rs:347, packages/d2b-provider-operation/src/operation.rs:405, packages/d2b-provider-operation/src/operation.rs:475]
  evidence: docs seed 3 `-> Result<` = 8 hits across the five public constructors plus accessor returns; seed 1 public items = 72 hits, all documented (missing_docs denied) except the canonical-section gap.
- clean: otherwise every pub item, const, and enum variant carries a one-line doc; module header explains the materialized-vs-inherited split and the facet set.

### perf
- N/A: seeds 1-3 all zero (no format!, no grow-by-push collections, no to_string copies in src).

### conc
- N/A: seeds 1-4 all zero.

### async
- N/A: seeds 1-4 all zero over src (no async fn, no tokio surface); the only async is the `#[tokio::test]` registration scaffold.

### unsafe
- N/A: seeds 1-4 all zero and manifest declares `unsafe_code = "forbid"`.

### ffi
- N/A: seeds 1-4 all zero.

### macro
- N/A: seeds 1-4 all zero.

### test
- clean: seeds over src+tests = 19 hits (5 `#[test]`, 1 `#[tokio::test]` registration, 13 `assert*`); tests pin materialized-command owner, wire-tag exclusivity, secret-access ceiling, a table-driven audit-join rejection loop with per-case messages, bounds ceiling rejections, and a closed round trip - each fails on a real regression.

### Coverage
- idiom: clean (1 hit; sanctioned invariant-preserving Default)
- own: clean (1 hit; test-only)
- type: clean (0/0/0)
- api: clean (68 hits)
- err: clean (0/0/0 outside tests; 1 error enum)
- serde: clean (49 hits)
- obs: N/A (seeds: 0/0/0/0 all zero; no tracing/log dep)
- docs: 1 finding (72 hits)
- perf: N/A (seeds: 0/0/0 all zero)
- conc: N/A (seeds: 0/0/0/0 all zero)
- async: N/A (seeds: 0/0/0/0 all zero)
- unsafe: N/A (seeds: 0/0/0/0 all zero)
- ffi: N/A (seeds: 0/0/0/0 all zero)
- macro: N/A (seeds: 0/0/0/0 all zero)
- test: clean (19 hits: 5 #[test], 1 #[tokio::test], 13 asserts)
