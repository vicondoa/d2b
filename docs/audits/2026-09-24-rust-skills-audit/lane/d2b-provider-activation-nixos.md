# d2b-provider-activation-nixos - d2b-provider-activation-nixos
Baseline: 6ebdd4cec22e6537d1376e83ff7a82b00a8492c0 | LOC audited: 4014 (src 3368 + tests 646, excl. src/generated/**, no generated dir present) | modules: whole crate (controller, driver, effects_service, facets, lib, test_support, vocabulary; tests/reconcile.rs, tests/registration.rs)
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: none (single-part lane)

## idiom
- clean: seeds 1/2/3 = 0/0/0 over src; no index loops, no hand-written derive-replaceable impls, no statement-style accumulation. The three hand-written `Debug` impls (controller.rs:217, 439, 508) are deliberate redaction of key/signature bytes (card false-positive class; a derive would leak).
- clean: checked expression shape, conversion impls, naming (`as_`/`to_`/`into_` discipline holds; no `get_` accessors), and newtype usage across all seven src files.

## own
- clean: seeds 1/2/3/4 = 41/2/0 (the card's `Rc<` alternative also matches inside every `Arc<`, inflating raw counts; real seed-3 hits are the two `Arc<parking_lot::Mutex<...>>` in test_support). Every clone read: Arc clones at the factory/facet boundaries are genuine shared ownership (call sites driver.rs:414, effects_service.rs:188 - the daemon composition root builds one dispatch source, the factory and each built service share it); spec-field clones build owned `RunnerRequest`/`HandoffIntent` values (controller.rs:354, 384, 789-790; driver.rs:630, 636, 806); `observed.clone()` at driver.rs:809 is required because the observation is consumed again by `execute_host_runner`/`apply_runner_result`; the rest are test doubles. No `&String`/`&Vec` parameters, no `Rc`/`RefCell`, no `Cow`, no mutable statics.
- clean: all 41 clone-family sites judged explainable in one sentence; no `mem::take` opportunity and no borrow-splitting conflict found.

## type
- clean: seeds 1/2/3 = 1/0/0; the single hit is the test fn name `validate_rejects_a_spec_outside_the_closed_generation_contract` (driver.rs:1229), not a validation fn. State is enum-typed throughout (`CallerRole`, `GenerationPhase`, `TrustStatus`, `ActivationMode`); `start_root`/`source_generation_preserved` are single semantic booleans, not flag pairs; `ActivationRunnerStep.label` is a deliberate wire label.
- clean: no boolean-flag soup, no `Option` pairs, no stringly-typed state, no validate-at-every-callsite pattern (the spec constructor is the single admission fence, driver.rs:220).

## api
- d2b-provider-activation-nixos#1 sev=low blast=leaf effort=S verdict=actionable - `ActivationDriver` is re-exported at lib.rs:38 but no external consumer names it: the factory returns `Box<dyn DynResourceDriver>` (driver.rs:410), so the concrete type never escapes the crate - fix: make `ActivationDriver` `pub(crate)` and drop it from the lib.rs re-export arm - [packages/d2b-provider-activation-nixos/src/driver.rs:426, packages/d2b-provider-activation-nixos/src/lib.rs:38]
  evidence: census: `ActivationDriver\b` over packages/, nixos-modules/, tests/, docs/reference/, labs/ = 8 hits, all inside this crate (driver.rs:411,426,436,715,964,1205; lib.rs:38); seed 1 (pub items) = 95 hits
- d2b-provider-activation-nixos#2 sev=low blast=leaf effort=S verdict=actionable - `ACTIVATION_RUNNER_RESOURCE_TYPE` (controller.rs:14) is `pub` inside the exported `controller` module but used only at controller.rs:296 in the same module, so it is reachable as `d2b_provider_activation_nixos::controller::ACTIVATION_RUNNER_RESOURCE_TYPE` with no consumer - fix: make the const private (or `pub(crate)`) - [packages/d2b-provider-activation-nixos/src/controller.rs:14, packages/d2b-provider-activation-nixos/src/controller.rs:296]
  evidence: census: `ACTIVATION_RUNNER_RESOURCE_TYPE` over packages/, nixos-modules/, tests/, docs/reference/, labs/ = 2 hits, both in controller.rs
- clean: seeds 1/2/3 = 95/5/6; the lib.rs re-export arms are the house single-surface pattern (card false positive); the `Arc<dyn ActivationBrokerDispatch>` pub field in `ActivationEffectFacets` (facets.rs:34) genuinely shares one daemon-supplied dispatch source across the factory and per-zone services (d2bd implements the trait at resource_plane_v3.rs:2316; clones at driver.rs:414, effects_service.rs:188); `ActivationDriverError` is the skill's struct-with-private-kind public error; every pub item carries a doc comment (`#![deny(missing_docs)]`, lib.rs:8).

## err
- d2b-provider-activation-nixos#3 sev=medium blast=leaf effort=S verdict=actionable - `GenerationObservation::terminal` (exported via lib.rs:33) panics with `assert!` on a caller-supplied name that is empty, contains '/', or exceeds 128 chars, instead of making the bound a type or a `Result` - fix: take `name: ResourceName` (already bounded: no '/', <=128 chars) and have `new` parse through the same path, or return `Result`; this deletes the runtime check the type makes impossible - [packages/d2b-provider-activation-nixos/src/controller.rs:119, packages/d2b-provider-activation-nixos/src/controller.rs:121]
  evidence: seed 1 (unwrap/expect) = 48 in src, of which 18 production expects are on literally-built static values in `activation_runner_spec`/name derivation (card false-positive class) and 30 sit in `#[cfg(test)]`; seed 3 (panic!/unreachable!/todo!/unimplemented!) = 0; the assert! at controller.rs:121 is the only library panic on caller input (assert! is not a card seed)
- clean: seeds 1/2/3/4 = 48/0/0/4; error taxonomy is closed and caller-action-split: `ActivationError` (5 Copy variants, controller.rs:395), `ActivationVerificationError` (9 variants, controller.rs:497), `ActivationDriverError` (struct with private `kind` + `op`, driver.rs:133); no swallowed Results, no `let _ =`, no panic macros; every driver failure maps through `map_err` to the typed error.

## serde
- clean: seeds 1/2/3/4 = 0/0/0/9; the 9 hits are `from_slice`/`to_value`/`from_value` at the decode hook (driver.rs:220, 549, 651, 664) and the effects payload (effects_service.rs:69). Deserialization lands directly in the closed `NixosGenerationSpec` contract type whose constructor is the validation fence; no derive/`rename_all`/`deny_unknown_fields`/`try_from` decisions live in this crate (the contract crates own them); the `providerRef` JSON insert in `ensure_runner` (driver.rs:656-659) is a deliberate wire-shape accommodation for the manager's child row, not a boundary parse.
- clean: no hand-written `Deserialize`, no enum-representation choices, no `flatten`; round-trip behavior is covered by the runner-spec assertions in tests/reconcile.rs:97-119.

## obs
- d2b-provider-activation-nixos#4 sev=low blast=leaf effort=S verdict=actionable - nine `tracing::warn!` refusal events in `ActivationTrust::verify` are message-only with no named fields and no enclosing span (no `#[instrument]` anywhere in the crate), so the failing fence is queryable only as message text - fix: add a named field carrying the error variant (e.g. `refusal = ?ActivationVerificationError::TrustEpochMismatch`), keeping fields identifier-free so the site stays under the ADR 0010/0028 redaction gate - [packages/d2b-provider-activation-nixos/src/controller.rs:569, packages/d2b-provider-activation-nixos/src/controller.rs:575, packages/d2b-provider-activation-nixos/src/controller.rs:581, packages/d2b-provider-activation-nixos/src/controller.rs:587, packages/d2b-provider-activation-nixos/src/controller.rs:593, packages/d2b-provider-activation-nixos/src/controller.rs:602, packages/d2b-provider-activation-nixos/src/controller.rs:609, packages/d2b-provider-activation-nixos/src/controller.rs:615, packages/d2b-provider-activation-nixos/src/controller.rs:623]
  evidence: seed 2 ((info|debug|warn|error|trace)!(") = 9 hits, all message-only without fields; seed 3 ).instrument|#[instrument) = 0; seed 1 (println!/eprintln!) = 0
- clean: the other 8 tracing sites (controller.rs:49, 57, 748, 756, 763, 815, 822, 833) carry named fields (`target`, `role`, `generation`, `prior`, `outcome`); no secret or identifier reaches a field; no library-installed subscriber.

## docs
- d2b-provider-activation-nixos#5 sev=medium blast=leaf effort=S verdict=actionable - the pub Result-returning policy API (`verify_application`, `reconcile`, `apply_runner_result`, `refuse_undeclared_runner_step`, `ActivationTrust::verify`) documents no `# Errors` section, leaving 5 `ActivationError` and 9 `ActivationVerificationError` failure conditions unstated in the contract - fix: add `# Errors` sections naming the variants each fn returns - [packages/d2b-provider-activation-nixos/src/controller.rs:711, packages/d2b-provider-activation-nixos/src/controller.rs:735, packages/d2b-provider-activation-nixos/src/controller.rs:808, packages/d2b-provider-activation-nixos/src/controller.rs:726, packages/d2b-provider-activation-nixos/src/controller.rs:562]
  evidence: seed 2 (/// # (Examples|Errors|Panics|Safety)) = 0 across the crate; seed 3 (-> Result<) = 111 hits total; seed 1 (pub items) = 95 hits
- d2b-provider-activation-nixos#6 sev=low blast=leaf effort=S verdict=actionable - `GenerationObservation::terminal` can panic (assert at controller.rs:121) but its doc comment carries no `# Panics` section, so the bound is unstated in the contract - fix: add `# Panics` naming the empty/'/'/length bound (or drop the section once finding #3's type change removes the panic) - [packages/d2b-provider-activation-nixos/src/controller.rs:118, packages/d2b-provider-activation-nixos/src/controller.rs:121]
  evidence: seed 2 (canonical sections) = 0 hits; terminal is the only panic-capable pub item (assert at controller.rs:121)
- clean: every pub item is documented (`#![deny(missing_docs)]`); first sentences are single-line contract statements (e.g. "Stable controller failures.", "One declared activation runner step."); all seven modules carry `//!` docs; no doctests and no `ignore`d examples exist (nothing to rot).

## perf
- clean: seeds 1/2/3 = 16/19/0; every `format!`/`Vec::new` site is cold: reconcile-time runner-name derivation (controller.rs:276-289), error paths, one-shot diagnostics, and test fixtures. The per-byte `format!("{byte:02x}")` digest loop (controller.rs:287) runs once per reconcile at most; no hot loop, no grow-by-push collection in a loop, no `to_string()` at a boundary.
- clean: static (unmeasured) - no benchmark exists for this crate; nothing here would move a perf budget.

## conc
- clean: seeds 1/2/3/4 = 0/6/4/0; production uses one `tokio::sync::Mutex<Option<ResourceKey>>` (driver.rs:433) whose guards are scoped to single statements (no guard across an await; `await_holding_lock` is denied at the workspace table), and the parking_lot `Mutex` + `AtomicU64` pairs live only in test-support doubles and the test `RecordingManager`, each with an `async-gate-allow` marker (sanctioned test-support reason; inventory `packages/xtask/data/async-gate-inventory.json`). No `std::thread`, no `thread_local!`, no manual `Send`/`Sync` claims.
- clean: the test-only `Ordering::SeqCst` uid counter (driver.rs:1033) is a fixture, not a synchronization argument worth weakening.

## async
- clean: seeds 1/2/3/4 = 45/0/2/19; no `tokio::spawn`/`spawn_blocking`/`JoinSet`/`select!`/`join!` anywhere in production. The sync `dispatch_handoff` facet call inside async `apply_host_generation_handoff` (effects_service.rs:137) is the daemon-supplied boundary (R2): the implementation lives in d2bd's composition root (resource_plane_v3.rs:2316), outside this crate, and no blocking evidence exists here. `watched_runner` lock is never held across an await; `ensure_runner` commits the child through the manager before the spawn notification (F1), so the irreversible step is the manager's single non-cancellable commit; the 19 `#[tokio::test]` hits are test harnesses.
- clean: no runtime started inside the library; no future-size or `Send`-bound hazards found; the `async-gate-allow` markers in test_support.rs:50-52 are deliberate recorded exceptions (cited, not re-flagged).

## unsafe
- N/A (seeds: 1/2/3 = 0/0/0 all zero; seed 4 = 1, `unsafe_code = "forbid"` in Cargo.toml [lints.rust] - a forbid setting alone does not make the lens applicable per the card)

## ffi
- N/A (seeds: 1/2/3/4 = 0/0/0/0 all zero; no extern "C", no repr(C)/repr(transparent), no CStr/CString, no catch_unwind)

## macro
- N/A (seeds: 1/2/3/4 = 0/0/0/0 all zero; no macro_rules! definitions, no proc-macro/syn/quote, no $crate, no to_compile_error/new_spanned)

## test
- d2b-provider-activation-nixos#7 sev=low blast=leaf effort=S verdict=actionable - the six-case verification-fence table in `activation_verification_requires_all_trust_and_digest_fences` asserts without a per-case message, so a failure in case 3 of 6 reports only a line number and no case identity - fix: add a per-case failure message (e.g. `"case {i}: expected {expected_error:?}"`) to the loop assert - [packages/d2b-provider-activation-nixos/tests/reconcile.rs:439, packages/d2b-provider-activation-nixos/tests/reconcile.rs:447]
  evidence: seed 2 (assert_eq!/assert_ne!/assert!) = 160 hits over src+tests; the loop at tests/reconcile.rs:439-450 is the only table without a failure message
- clean: seeds 1/2/3/4 = 38/160/0/0; 38 test fns (13 driver, 6 effects_service, 2 vocabulary, 14 reconcile, 3 registration), no `#[ignore]`, no proptest/insta/rstest. Assertions target error variants, not `Display` strings (`assert_eq!(result.unwrap_err(), ActivationError::OutcomeMismatch)`); the KTD13 argv-free fence (`assert_no_launch_argv`, driver.rs:1821) is a genuine can-fail recursive check; the F1 persist-before-spawn ordering and one-watch-per-child invariants are asserted on the recorded manager log; trust fixtures generate a fresh key per run, keeping assertions deterministic.

## Coverage
- idiom: clean (seeds ran: 0/0/0)
- own: clean (seeds ran: 41/2/0)
- type: clean (seeds ran: 1/0/0)
- api: 2 finding(s)
- err: 1 finding(s)
- serde: clean (seeds ran: 0/0/0/9)
- obs: 1 finding(s)
- docs: 2 finding(s)
- perf: clean (seeds ran: 16/19/0)
- conc: clean (seeds ran: 0/6/4/0)
- async: clean (seeds ran: 45/0/2/19)
- unsafe: N/A (seeds: 0/0/0 all zero; forbid-only, per card criteria)
- ffi: N/A (seeds: 0/0/0/0 all zero)
- macro: N/A (seeds: 0/0/0/0 all zero)
- test: 1 finding(s)