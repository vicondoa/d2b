# d2b-provider-provider - d2b-provider-provider
Baseline: 6ebdd4cec22e6537d1376e83ff7a82b00a8492c0 | LOC audited: 2280 (excl. src/generated/**) | modules: whole crate
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: whole crate

## idiom
- clean: seeds ran 0/1/1 - the hand-written `Default for ProviderDriverFactory` (driver.rs:229) is not derivable (fields carry no `Default`) and the `Vec::new()` accumulation loop (driver.rs:485) has early exits and `?` the skill's plain-for exception covers; no index loops, no ad-hoc converters (clone sites are judged under `own`)

## own
- d2b-provider-provider#1 sev=low blast=leaf effort=S verdict=actionable - `let zone = ctx.key().zone.clone()` clones a `String` the callee accepts as `impl Into<String>` in three spots - fix: pass `ctx.key().zone.as_str()` / `view.key.zone.as_str()` directly; drop the `zone` local in the fixed-provider branch - [src/driver.rs:355, src/driver.rs:478, src/driver.rs:522]
  evidence: own seed 1 `\.clone\(\)` = 15 lines; `ZoneId::parse` takes `impl Into<String>` (d2b-contracts-resource/src/v3/identity.rs:79) and `ResourceKey::new` takes `impl Into<String>` (d2b-resource-runtime/src/spec_store.rs:61), so `&str` compiles without the clone
- d2b-provider-provider#2 sev=low blast=leaf effort=S verdict=actionable - `ctx.status::<ProviderDriverStatus>().cloned()` deep-clones the whole in-memory status (incl. the `BTreeSet<String>` volume_refs) on every reconcile pass - fix: hold the `Option<&ProviderDriverStatus>` reference (`ctx.status()` returns `Option<&T>`, d2b-resource-runtime/src/context.rs:459); last read of `previous` precedes `ctx.set_status` - [src/driver.rs:360, src/driver.rs:435]
  evidence: own seed 1 `\.clone\(\)` = 15 lines; the test helper's clone (driver.rs:1066) is required, this site is not
- clean: seeds ran 15/9/0/0 - remaining clones are required by signatures (`classify_error` trait shape, `CoreResourceKey::new`/`with_owner_identity` owned args, `metadata.insert` owned keys, `spec_object` returning owned `Value` from a `&Value` decode) or are test fixtures; no `Rc`/`RefCell`/`Arc<Mutex>`/`Cow` in production code

## type
- clean: seeds ran 1/0/0 - the single hit (`fn validate_refuses_a_spec_that_is_not_an_object`, driver.rs:1074) is a test fn name, not a runtime validation fn; `ProviderObservation`'s eight booleans are independent observed facts feeding one projection (not flag soup), `ProviderIntent`/`ProviderPhase`/`ProviderChildAction` are enums, no stringly-typed state

## api
- d2b-provider-provider#3 sev=low blast=leaf effort=S verdict=actionable - `ProviderDriverFactory::new()` and its `Default` impl are zero-caller public surface (the doc names "unit fixtures", but the crate's own tests construct via `with_effects`) - fix: delete `new()` and `impl Default` (driver.rs:213-232), or drop them to `pub(crate)` if a fixture wants them - [src/driver.rs:215, src/driver.rs:229]
  evidence: census `ProviderDriverFactory` over packages/, nixos-modules/, tests/, docs/reference/, labs/, BUILD.bazel = 0 callers of `new()`/`default()`; d2bd reaches the factory only through `provider_descriptor` (d2bd/src/resource_plane_v3.rs:144)
- d2b-provider-provider#4 sev=low blast=leaf effort=S verdict=actionable - `ProviderHandler::plan_external` (and the `ProviderError`/`ProviderChildAction`/`Disable`/`Delete` planning surface it serves) is exported through `pub mod providers` with zero production callers - fix: reduce to `pub(crate)` or delete `plan_external` (providers.rs:121-171) and the `ProviderIntent::Disable`/`Delete` arms of `plan_observed` if the external-provider path is not coming back; keep the surface the driver consumes (`plan_observed` Enable/Update, `plan_system_core`, `provider_observation`, `fixed_system_core_handlers_ready`) - [src/providers.rs:121, src/lib.rs:19]
  evidence: census `ProviderHandler|plan_external` over packages/, nixos-modules/, tests/, docs/reference/, labs/, BUILD.bazel = 0 production callers; only the crate's own tests exercise it (providers.rs:688, 710, 728); the driver passes only Enable/Update intents (driver.rs:361-365)
- clean: seeds ran 35/3/1 - the `Arc<dyn ProviderDriverEffects>` in `ProviderDriverArgs`/factory/driver is genuine shared ownership (factory clones the Arc per created driver, driver.rs:243); `pub use` re-export arms in lib.rs:31 are the house single-surface pattern; `test_support` is feature-gated; `ProviderPlan` keeps private fields with accessors

## err
- clean: seeds ran 42/1/0/1 - every `unwrap`/`expect` sits in `#[cfg(test)]` or the `test-support`-gated `RecordingEffects`; the one production `expect` (driver.rs:481) is on a literal const in the same file; `let _ = ctx.requeue_after)...)` (driver.rs:449) is a deliberate fire-and-forget requeue; `ProviderError` is a closed taxonomy with a `code()` accessor and `Display` = code; `CoreReconcileError` is logged at its site before being mapped to `spec_invalid`

## serde
- clean: seeds ran 0/0/0/15 - no derives, no serde attributes, no hand-written `Deserialize`; all 15 hits are `serde_json::from_/to_` on untyped `Value` (the core types' deliberate spec-envelope shape), and the object fence runs at both validate and reconcile (`spec_object`, driver.rs:575-601)

## obs
- clean: seeds ran 0/0/0/5 - all five events (`tracing::debug!`/`tracing::warn!`, providers.rs:281/348/370/412/463) carry named fields (`resource = ...`, `reason = %error`) with a plain message, no interpolation, no secrets, no `println!`; levels match handled-vs-attention semantics

## docs
- d2b-provider-provider#5 sev=low blast=leaf effort=S verdict=actionable - the eight `pub` fields of `ProviderObservation` are undocumented while every other pub item in the crate carries a doc comment - fix: add one-line field docs (or a struct-level contract explaining each gate) at providers.rs:72-79 - [src/providers.rs:72, src/providers.rs:79]
  evidence: docs seed 1 `^\s*pub (fn|struct|enum|trait|const|type)` = 34 hits; ProviderObservation is the only pub struct whose pub fields lack `///`
- d2b-provider-provider#6 sev=low blast=leaf effort=S verdict=actionable - the three pub `Result`-returning functions (`plan_external`, `plan_observed`, `provider_observation`) have no `# Errors` section naming which condition produces which failure - fix: add `# Errors` sections enumerating the `ProviderError`/`CoreReconcileError` variants each fn returns - [src/providers.rs:121, src/providers.rs:179, src/providers.rs:406]
  evidence: docs seed 3 `-> Result<` = 10 hits; the three pub fns are the ones the skill's `# Errors` trigger names
- d2b-provider-provider#7 sev=low blast=leaf effort=S verdict=actionable - README "State and telemetry" claims "the driver keeps no in-memory status either", but `reconcile_provider` publishes `ProviderDriverStatus` via `ctx.set_status` every pass - fix: correct README.md:72-74 to say the status is in-memory only (R11, never persisted) - [README.md:72, src/driver.rs:435]
  evidence: static; README.md:72-74 vs driver.rs:435-441 (`ctx.set_status(ProviderDriverStatus { ... })`); README is a policy-required path (packages/xtask/src/provider_crate_policy.rs) so the text must be fixed, not the path removed
- clean: seeds ran 34/0/10 - module docs present in all four files; every other pub item has a one-line first sentence; no `ignore`d doctests, no magic values without the why

## perf
- clean: seeds ran 3/8/4 - `format!` at driver.rs:521/606 is per-reconcile on a cold path (static, unmeasured); `to_string()` hits are error-path notes; `Vec::new()` sites are small per-pass collections; no hot loop, no attacker-keyed hashing, no benchmark exists to claim anything stronger

## conc
- clean: seeds ran 0/5/6/0 - all `Mutex`/`AtomicBool`/`Ordering::SeqCst` hits are the `RecordingManager`/`RecordingEffects` test fakes (test-only synchronization); no threads, no `thread_local!`, no manual `Send`/`Sync`; the test-fake lock sites carry `async-gate-allow` markers (deliberate, cited not re-flagged)

## async
- clean: seeds ran 62/0/0/10 - production async (validate/recover/reconcile/finalize/delete/dependencies/drain_owned_children) has no `tokio::spawn`, no `spawn_blocking`, no blocking calls on the executor, no guard held across `.await`; `#[async_trait]` is the skill-sanctioned object-safe choice; the 10 `#[tokio::test]` harnesses are deterministic fakes (scripted manager, no sleeps)

## unsafe
- N/A (seeds: 0/0/0/0 all zero; manifest `unsafe_code = "forbid"`, no blocks/fns/impls, no `SAFETY:` sites)

## ffi
- N/A (seeds: 0/0/0/0 all zero; no extern surface, no repr, no CStr)

## macro
- N/A (seeds: 0/0/0/0 all zero; no macro_rules!, no proc-macro surface)

## test
- d2b-provider-provider#8 sev=medium blast=leaf effort=S verdict=actionable - the `Degraded` phase projection (`optional_components_degraded` -> `ProviderPhase::Degraded` in `plan_observed`) is production-reachable through the driver's Enable/Update intents and has no test - fix: add a `#[tokio::test]` (or `#[test]` on `plan_observed` directly) that sets `optional_components_degraded = true` with ready dependencies and asserts `phase == Degraded` and `publish_exports` stays true - [src/providers.rs:206, src/driver.rs:1147]
  evidence: test seeds = 74 hits (17 test fns, 57 assertions); no test sets `optional_components_degraded = true` - the existing observation assertions pin it `false` (driver.rs:1151) and the plan tests cover Ready/Pending/TrustOrCompatibilityDenied only
- clean: seeds ran 17/57/0/0 - tests assert behavior (phase transitions, call order, error variants via `matches!`, wire codes via `kind().code()`), not implementation; no `#[ignore]`, no network, no sleeps, no proptest/insta/rstest; the registration suite pins the declaration contract through the real `ProviderDirectory`

## Coverage
- idiom: clean (seeds: 0/1/1)
- own: 2 finding(s)
- type: clean (seeds: 1/0/0; single hit is a test fn name)
- api: 2 finding(s)
- err: clean (seeds: 42/1/0/1)
- serde: clean (seeds: 0/0/0/15)
- obs: clean (seeds: 0/0/0/5)
- docs: 3 finding(s)
- perf: clean (seeds: 3/8/4)
- conc: clean (seeds: 0/5/6/0)
- async: clean (seeds: 62/0/0/10)
- unsafe: N/A (seeds: 0/0/0/0 all zero; manifest `unsafe_code = "forbid"`)
- ffi: N/A (seeds: 0/0/0/0 all zero)
- macro: N/A (seeds: 0/0/0/0 all zero)
- test: 1 finding(s)