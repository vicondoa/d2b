# d2b-provider-wayland-policy - d2b-provider-wayland-policy
Baseline: 6ebdd4cec22e6537d1376e83ff7a82b00a8492c0 | LOC audited: 3974 (src 3247 + tests 727; excl. src/generated/**, none present) | modules: whole crate
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: none (single-part lane)

## idiom
- clean: seeds ran: 0/0/0; no index loops over `0..`, no hand-written `impl (Default|From|PartialEq|Eq|Debug|Clone|Hash) for`, no `let mut ... = String/Vec::new()` accumulation. One hand-written `Debug` for `AudioResourceRuntime` (audio_registry.rs:142) was read and judged a deliberate state-summary (counts only, not the mediator handle/map bodies), acceptable for the internal state-holding type.



## own
- clean: seeds ran: 32/17/0/0; every one of the 32 `.clone()` lines is explainable: struct/field construction into owned values, `serde_json` Value edits from borrowed refs (`from_value` takes ownership), Arc clones at the genuine shared-ownership boundaries (factory `create` per driver, six drivers' shared effects port, per-zone audio registry handle),test fixtures; the 17 to_owned/to_vec/to_string lines are string/byte construction and test data; no `Rc`/`RefCell`/`Arc<Mutex<`/`Cow<` anywhere overviewed.



## type
- clean: seeds ran: 3/0/0; the three validate/check fns are fresh-evidence cross-checks, not parse-once candidates: `validate_audio_dependency_identity` (effects_service.rs:651) re-verifies a manager-rendered row's identity against the row it claims (defense against torn/foreign rows),`check_row` (interaction.rs:515) enforces the driver-zone/type/provider fences per pass (a relationship between two live values,a type cannot carry it),and `validate_relationships` is a cfg(test) helper (audio_registry.rs:628). The stringly-typed `InteractionDriverArgs.zone: String` (a parse-once candidate) is the same root as the panic filed under err#1 and is covered there.



## api
- clean: seeds ran: 70/6/5; all six public-signature `Arc<...>` sites are genuine shared ownership or sanctioned exports: `spec_decoder`/`wayland_policy_spec_decoder` return the manager-held `Arc<dyn SpecDecoder>` (call sites: wayland_policy.rs:86, tests/engine.rs:304, tests/registration.rs:62),`InteractionDriverArgs.effects` is the six drivers' shared effects port (effects_service.rs:85-87; factory clones it per create at interaction.rs:471),feature-gated `test_support::Log` is consumed by tests (repo false-positive class),and the `pub(crate) fn *() -> &Arc<...>` facet accessors are crate-internal. The `pub use` re-export arms (lib.rs:47-70) form the house single-surface pattern;`#![deny(missing_docs)]` (lib.rs:28) forces doc presence on every public item.



## err
- d2b-provider-wayland-policy#1 sev=high blast=family effort=M verdict=actionable - Panic reachable from caller input at the family engine's public boundary: `InteractionDriver::new` parses-and-expects `InteractionDriverArgs.zone: String` (pub field on pub struct with no validating constructor),and `key_ref` parses-and-expects a `ResourceKey` whose `new` accepts any strings; both invariants claimed in expect messages are not enforced by the types - fix: parse once at the args boundary (change `args.zone` to a parsed `ZoneId`, or make `InteractionDriver::new` return `Result<_, InteractionDriverError>`) and make `key_ref` return `Result<ResourceRef, InteractionDriverError>` (map to `SpecInvalid`) or enforce name canonicality at `ResourceKey::new` in d2b-resource-runtime - [packages/d2b-provider-wayland-policy/src/interaction.rs:428, packages/d2b-provider-wayland-policy/src/interaction.rs:488, packages/d2b-provider-wayland-policy/src/interaction.rs:836-838, packages/d2b-resource-runtime/src/spec_store.rs:60-63]
  evidence: seed `\.unwrap\(\)|\.expect\(` = 43 lines over src (most in `#[cfg(test)]`/`test-support`; the two production sites above are the panics); census: `ResourceKey::new` (spec_store.rs:60-63) builds the three String fields unvalidated; route: review-pass

- clean: seeds ran: 43/0/0/3 after removing the test-module noise; the other production expects are infallible (`expect("fixed digest width")` on a literal 8-byte slice, test-support fixed refs`, and enums are the error taxonomy (AudioResourceRuntimeError, InteractionEffectError, InteractionDriverError),closed and split by caller action (retryable vs terminal classes at interaction.rs:160-174`.



## serde
- d2b-provider-wayland-policy#2 sev=medium blast=family effort=M verdict=actionable - Every wire-parse failure collapses into a bare `InvalidResource` variant that discards the serde reason, so an operator cannot tell which row or which field is malformed (a third of the enum's refusals are spec-shape checks that reuse the same variant) - fix: add a reason-carrying variant to `InteractionEffectError` and `AudioResourceRuntimeError` (e.g. `InvalidResource { reason: String }` or `Decode(#[source] serde_json::Error)` via thiserror)and thread it through the ~15 `map_err(|_| ...InvalidResource)` sites (the enum Display codes are not pinne in `docs/reference/error-codes.md` - grep "interaction" = 0 hits - so not wire-contract) - [packages/d2b-provider-wayland-policy/src/interaction.rs:241-243, packages/d2b-provider-wayland-policy/src/effects_service.rs:205-206, packages/d2b-provider-wayland-policy/src/audio_registry.rs:517-534]
  evidence: seed `serde_json::from_|serde_json::to_` = 23 lines; every parse failure maps to a bare InvalidResource (or the empty `InteractionSpecDecodeError` at interaction.rs:269-270; serde derive/attribute/impl seeds are 0/0/0 - parse-only boundary, so the serde reason loss is the boundary flaw.



## obs
- obs: N/A (seeds: 0/0/0/0 all zero; no `tracing`/`log` dependency in Cargo.toml - the crate cross neither logging surface)



## docs
- d2b-provider-wayland-policy#3 sev=low blast=leaf effort=S verdict=actionable - Result-returning public items lack `# Errors` canonical sections, so callers must infer which conditions produce `InvalidResource` vs `Unavailable` (the terminal-vs-retryable mapping at interaction.rs:632-636 is non-obvious) - fix: add `# Errors` sections to `base_spec`, `spec_with_provider_ref`, `shell_pool_spec`, `shell_session_execution`, `shell_session_pool_ref`, `owned_child_ensure`, `binding_child_ensure`, and the two `InteractionDriverEffects` methods - [packages/d2b-provider-wayland-policy/src/interaction.rs:241, packages/d2b-provider-wayland-policy/src/vocabulary.rs:35, packages/d2b-provider-wayland-policy/src/interaction.rs:342]
  evidence: seeds: `^\s*pub (fn|struct|enum|trait|const|type)` = 66;`/// # (Examples|Errors|Panics|Safety)` = 0;`-> Result<` = 68 lines; the crate opted in to `#![deny(missing_docs)]` (lib.rs:28),so canonical sections are the next consistency step (the missing-docs lint is per-crate, contrary to the blanket "not enabled anywhere" note in U1)



## perf
- clean: seeds ran: 3/12/0; the three `format!` sites are test-support log pushes (test_support.rs:172,190)and the cold `key_ref` string build (interaction.rs:837);`Vec::new()`/`BTreeMap::new()` are empty-start constructors of long-lived registries and test data; no allocation in a reconcile/effect hot path (per-reconcile serde Value clones at interaction.rs:242,252 are cold, one per row pass); static (unmeasured)



## conc
- clean: seeds ran: 0/2/9/0; the two `Mutex<` lines are `tokio::sync::Mutex` guards for the per-zone audio registry (audio_registry.rs:413)and the test log (test_support.rs:127) - async-appropriate (guard spans a sync registry call, dropped at statement end; no std::thread/spawn/scope anywhere; the 9 atomic/Ordering lines are test-double flags (`AtomicBool`/`AtomicUsize`, SeqCst on plain scripted booleans - harmless and test-only)



## async
- clean: seeds ran: 94/0/4/0; all awaits are facet/plane/manager trait reads and the two tokio Mutex guards; no `tokio::spawn`/`spawn_blocking`/`JoinSet`/`select!`/`join!` hits (this engine's design: no spawn surface, documented at interaction.rs:20-23); no blocking std call in an async body; the reconcile/delete/watch loops mutate through idempotent manager verbs with await-per-step ; cancellation-safe (no lock held across `.await` beyond the statement); no `#[tokio::main(test]`/`Runtime::block_on` in src



## unsafe
- unsafe: N/A (seeds: 0/0/0/0; local `[lints.rust]` `unsafe_code = "forbid"` in Cargo.toml - the crate is fully safe)



## ffi
- ffi: N/A (seeds: 0/0/0/0; no extern/repr/CStr surface in the crate)



## macro
- macro: N/A (seeds: 0/0/0/0; no macros defined, no proc-macro/syn/quote usage)



## test
- d2b-provider-wayland-policy#4 sev=low blast=leaf effort=S verdict=actionable - Dead no-op line in `the_policy_envelope_is_the_whole_contract`: `let _ = ResourceRef::parse)...)` asserts nothing and cannot fail - fix: assert the parse succeeds (e.g. `.expect("the policy reference parses")`), or delete the line - [packages/d2b-provider-wayland-policy/tests/registration.rs:93]
  evidence: static read; `Result` is discarded with no assertion; the surrounding test already covers the decoder refusal paths at registration.rs:92

- clean: seeds ran: 25/75/0/0; the suite is behavior-focused: assertions carry messages and cite regressions (e.g. "Regression (P2)" at effects_service.rs:746-752),the recording-manager harness makes ordering assertions on log entries with context,no proptest/insta/rstest and no `#[ignore]` (deterministic fixtures, injected time, no network/clock); the `#[allow(clippy::disallowed_methods, reason = "cfg(test helper")]` sites use the sanctioned reason



## Coverage
- idiom: clean (0/0/0)
- own: clean (32/17/0/0)
- type: clean (3/0/0)
- api: clean (70/6/5)
- err: 1 finding(s)
- serde: 1 finding(s)
- obs: N/A (0/0/0/0; no tracing/log dep)
- docs: 1 finding(s)
- perf: clean (3/12/0)
- conc: clean (0/2/9/0)
- async: clean (94/0/4/0)
- unsafe: N/A (0/0/0/0; forbid)
- ffi: N/A (0/0/0/0)
- macro: N/A (0/0/0/0)
- test: 1 finding(s)