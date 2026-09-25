# d2b-broker-composition - d2b-broker-composition
Baseline: 6ebdd4cec22e6537d1376e83ff7a82b00a8492c0 | LOC audited: 1634 (excl. src/generated/**; no tests/ dir exists, test lens covers inline #[cfg(test)] modules) | modules: whole crate (lib.rs, main.rs, routing.rs, seam.rs, dependency_surface.rs)
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: none (single-part lane)

## idiom
- d2b-broker-composition#1 sev=low blast=leaf effort=S verdict=actionable - `workspace_root` walks up to four parent directories with a `for _ in 0..4` index loop and a mutable `current`, where the bounded walk is an iterator chain - fix: replace the loop with `std::iter::successors(Some(current), |c| c.parent()).take(4).find(|c| c.join("Cargo.toml").is_file() && c.join("packages").is_dir())` - [packages/d2b-broker-composition/src/dependency_surface.rs:136]
  evidence: seed `for \w+ in 0\.\.` = 1 hit (dependency_surface.rs:136); the other idiom seed hits are `let mut violations = Vec::new()` (line 165), a side-effect accumulation with early continues where an iterator would obscure the dedup/sort tail - not a finding
- d2b-broker-composition#2 sev=low blast=leaf effort=S verdict=actionable - `state_cell` silences its deliberately unused parameter with `let _ = invocation;` instead of naming it as unused - fix: rename the parameter to `_invocation` and delete the `let _ = invocation;` line (the doc comment's "the invocation's row" is prose, not the parameter name) - [packages/d2b-broker-composition/src/seam.rs:270, packages/d2b-broker-composition/src/seam.rs:274]
  evidence: seed `let mut \w+ = (String|Vec)::new\(\)` = 1, seed `for \w+ in 0\.\.` = 1; `let _ =` site read at seam.rs:274
- clean: seeds ran (1/0/1); no hand-written Default/From/PartialEq/Eq/Debug/Clone/Hash impls (all derives), no index loops over collections, naming discipline holds (`as_str` const fn, `is_clean`, no `get_`, free function `report_error` in main.rs)

## own
- d2b-broker-composition#3 sev=low blast=leaf effort=S verdict=actionable - `audit_crate` iterates `&added` and clones every dependency name into the report fields, though `added` is dead after the loop - fix: consume it with `for name in added { ... report.forbidden_dependencies.push(name); ... report.proc_macro_dependencies.push(name); }` (passing `&name` to `is_proc_macro`), removing both clones - [packages/d2b-broker-composition/src/dependency_surface.rs:251, packages/d2b-broker-composition/src/dependency_surface.rs:255]
  evidence: seed `\.clone\(\)` = 12 hits over src (10 dependency_surface.rs, 2 seam.rs test fixtures); sites read in full, `added` has no use after the loop
- d2b-broker-composition#4 sev=low blast=leaf effort=S verdict=actionable - the manifest scan checks `report.forbidden_dependencies.contains(&crate_name.to_string())`, allocating a fresh String per forbidden crate name (8 per audit run) for a membership test - fix: use `report.forbidden_dependencies.iter().any(|name| name == crate_name)` - [packages/d2b-broker-composition/src/dependency_surface.rs:272]
  evidence: seed `\.to_owned\(\)|\.to_vec\(\)|\.to_string\(\)` = 8 hits; the other hits (report construction at 244, error messages at 321/332, `(*crate_name).to_owned()` at 274, `owner_name.to_owned()` at 351) are explainable owned values
- d2b-broker-composition#5 sev=low blast=leaf effort=M verdict=actionable - `dependency_tree` clones every node id into `queue` and `seen` although all ids borrow from `metadata` for the whole traversal - fix: type the traversal as `Vec<&str>` / `BTreeSet<&str>` (`let mut queue = vec![root_id];`, `seen.insert(id)`), leaving the returned `Vec<String>` untouched - [packages/d2b-broker-composition/src/dependency_surface.rs:340, packages/d2b-broker-composition/src/dependency_surface.rs:343]
  evidence: seed `\.clone\(\)` = 12 hits; queue/seen sites read; `reachable` keeps `to_owned()` because it is the return value
- clean: no Rc/RefCell/Arc<Mutex>/Arc<RwLock>/Cow (seeds 3-4 = 0); the two seam.rs `invocation.payload.clone()` sites are test-fixture handlers echoing the payload and are explainable

## type
- clean: seeds ran (0/0/0); the crate declares structs and enums so the lens is applicable, but the refusal taxonomy is already enum-modeled (`RefusalClass`, `RoutingVerdict`, `RoutingRefusal`), `PureTransformClaim` is a private-field newtype with a constructor, and there are no boolean-flag or stringly-typed state fields to collapse

## api
- clean: seeds ran (24/0/0); all 24 pub items audited - the surface is deliberate and single-path (lib.rs `pub mod` arms with doc comments, the house pattern), no Arc/Rc/Box/RefCell in any public signature, `PureTransformClaim`/`HandlerDeclaration`/`SurfaceReport` are documented data types, and the d2b-broker types in `HandlerDeclaration.handler` are the crate's reason to exist (composition root, publish = false), not a leak

## err
- d2b-broker-composition#6 sev=low blast=leaf effort=S verdict=actionable - four public/private error returns are bare `Result<_, String>` (`verify_startup_routing`, `audit_crate`, `run_cargo_metadata`, `dependency_tree`), so a future caller that must distinguish failure classes (environment unavailable vs. cargo failure vs. invariant violation) can only string-match - fix: introduce a small typed error enum per module (the seam already owns `RoutingRefusal`; give `dependency_surface` an audit error enum with variants such as `WorkspaceUnavailable`/`CargoFailed`/`InvalidMetadata`) and return it from the cited functions - [packages/d2b-broker-composition/src/seam.rs:198, packages/d2b-broker-composition/src/dependency_surface.rs:226, packages/d2b-broker-composition/src/dependency_surface.rs:292, packages/d2b-broker-composition/src/dependency_surface.rs:317]
  evidence: seed `\.unwrap\(\)|\.expect\(` = 17 hits (13 inside #[cfg(test)] modules - repo false positive; 3 are `expect("static pattern compiles")` on literally-built regexes and 1 is the post-`route_row` invariant expect at seam.rs:238 - all justified); seed `\bpanic!\(|\bunreachable!\(|\btodo!\(|\bunimplemented!\(` = 0; seed `enum \w*Error` = 0; the String-error shapes were read at the cited sites
- clean: no swallowed Results (`let _ = invocation;` at seam.rs:274 is a deliberately ignored parameter, not a dropped Result); panic policy is sound - the only non-test expect names the invariant the mechanical rule just established

## serde
- d2b-broker-composition#7 sev=low blast=leaf effort=M verdict=actionable - `run_cargo_metadata` parses cargo's output into `serde_json::Value` and every consumer re-walks it with repeated `.get("packages")`/`and_then(as_array)`/`as_str` chains (`dependency_tree`, `package_name_of_id`, `is_proc_macro`), pushing the parse out of the boundary - fix: derive `Deserialize` on minimal `CargoMetadata`/`Package`/`ResolveNode` shapes and parse once in `run_cargo_metadata`, replacing the Value-walking chains with field access - [packages/d2b-broker-composition/src/dependency_surface.rs:308, packages/d2b-broker-composition/src/dependency_surface.rs:318, packages/d2b-broker-composition/src/dependency_surface.rs:365, packages/d2b-broker-composition/src/dependency_surface.rs:380]
  evidence: seed `serde_json::from_|serde_json::to_` = 1 hit (dependency_surface.rs:308); no serde derives or wire types exist in the crate (seeds 1-3 = 0), and cargo metadata is cargo's contract, not repo wire, so the change is actionable
- clean: no derive(Serialize/Deserialize), no serde attributes, no hand-written Deserialize impls; the single serde_json use is the metadata parse above

## obs
- clean: seeds ran (6/0/0/9); the 6 `eprintln!` hits are CLI product output (main.rs:34, 50, 54, 58 - operator-facing startup/exit diagnostics, the skill's own carve-out) and test skip notices (dependency_surface.rs:423, seam.rs:625); the 9 remaining hits are `log::` false-matching inside `d2b_broker::catalog::` paths; the lib emits no telemetry and the binary installs the subscriber exactly once at main.rs:24-32 (the sanctioned place); no interpolated message-only events, no secrets in fields

## docs
- d2b-broker-composition#8 sev=low blast=leaf effort=S verdict=actionable - the five Result-returning public items document their failure conditions in prose but carry no canonical `# Errors` section, so the failure contract is not machine-checkable at a glance - fix: add `# Errors` sections to `register_declared_handlers`, `register_production_handlers`, `verify_startup_routing`, `probe_crate_sources`, and `audit_crate` naming which conditions produce which refusal/error - [packages/d2b-broker-composition/src/seam.rs:157, packages/d2b-broker-composition/src/seam.rs:173, packages/d2b-broker-composition/src/seam.rs:198, packages/d2b-broker-composition/src/dependency_surface.rs:151, packages/d2b-broker-composition/src/dependency_surface.rs:226]
  evidence: seed `/// # (Examples|Errors|Panics|Safety)` = 0 hits; seed `-> Result<` = 9 hits (7 public items plus 2 test helpers); every pub item already has a one-line first sentence and every module has `//!` docs, so only the canonical-section shape is missing

## perf
- clean: seeds ran (5/4/1); all `format!` hits are cold error construction (seam.rs:205/210 invariant messages, dependency_surface.rs:228/267/269/309 audit diagnostics) and all `Vec::new()` hits are audit tooling with unknown sizes - repo false positives per the card; the single `to_string()` hit (dependency_surface.rs:272) is flagged under own; nothing here is on a hot path and no benchmark exists, so all observations are static (unmeasured)

## conc
- N/A: seeds (0/0/0/0) all zero; the crate spawns no threads, holds no Mutex/RwLock, uses no atomics or thread_local, and declares no manual Send/Sync

## async
- clean: seeds ran (17/0/0/4); every hit is inside `#[cfg(test)]` - four `#[tokio::test]` harnesses, two async fixture helpers, and their `.await` calls; the library declares no `async fn` (handlers are `fn` pointers returning d2b-broker's boxed `HandlerFuture`), and `dependency_surface`'s synchronous document/process reads carry the one sanctioned module-level blanket allow (`#![allow(clippy::disallowed_methods)]` at dependency_surface.rs:8, the U1 (d)4 exemption - cited, not re-flagged)

## unsafe
- N/A: seeds 1-3 (0/0/0) all zero; seed 4 alone hits - `#![deny(unsafe_code)]` at lib.rs:13 and `unsafe_code = "deny"` in the manifest lints table; the crate is on the U1 (d)8 deny list with zero unsafe blocks, consistent with the ledger

## ffi
- N/A: seeds (0/0/0/0) all zero; no extern "C", no no_mangle, no repr(C)/repr(transparent), no CStr/CString/c_char anywhere in the crate

## macro
- clean: seeds ran (0/8/0/0); all 8 hits are the `proc_macro` identifier in the dependency-surface audit vocabulary (field `proc_macro_dependencies`, fn `is_proc_macro`), not macro usage - no `macro_rules!`, `syn`/`quote`, `$crate`, or `to_compile_error`/`new_spanned` anywhere; the crate defines and uses no macros beyond std

## test
- d2b-broker-composition#9 sev=low blast=leaf effort=S verdict=actionable - `an_effectful_handler_offered_to_the_in_broker_table_is_refused_by_the_routing_rule` asserts `format!("{refusal}").contains("forward carrier")`, pinning the routing refusal's Display wording after the `matches!` variant check already pins the contract - fix: delete the Display-string assertion (the variant match is the contract; a wording change must not fail the suite) - [packages/d2b-broker-composition/src/seam.rs:523]
  evidence: seed `assert_eq!\(|assert_ne!\(|assert!\(` = 51 lines; the assertion was read in context - the preceding `matches!` on `RoutingRefusal::Forwarded { class: RefusalClass::Effectful, .. }` is the behavioral check
- d2b-broker-composition#10 sev=low blast=leaf effort=S verdict=actionable - `an_unregistered_admitted_operation_fails_the_startup_invariant` never exercises an admitted-without-handler operation (the committed catalog admits nothing this pass, and the fixture row is refused by `verify_startup_routing` as uncommitted), so the body only asserts the empty-registration happy path and registers a discarded fixture - fix: rename the test to what it asserts (e.g. `the_admitted_set_stays_empty_with_nothing_registered`) and drop the comment's claim that the admitted-without-handler leg is pinned by the fixture row, or restructure to feed a genuinely admitted row when one exists - [packages/d2b-broker-composition/src/seam.rs:731, packages/d2b-broker-composition/src/seam.rs:733]
  evidence: seed `#\[test\]|#\[tokio::test\]` = 31 hits (5 dependency_surface.rs, 11 routing.rs, 15 seam.rs); test body read in full - no assertion involves an admitted row
- clean: 31 tests, all in the right form (unit tests in `#[cfg(test)]` needing private access), expectations human-written literals, deterministic (no network/clock/randomness), no `#[ignore]`, no proptest/insta/rstest (not needed); the environment-dependent `skip_without` early returns are documented skips, and every remaining test can fail on a real regression

## Coverage
- idiom: 2 finding(s)
- own: 3 finding(s)
- type: clean (seeds ran: 0/0/0; structs and enums declared so the lens is applicable; refusal taxonomy already enum-modeled)
- api: clean (seeds ran: 24/0/0; deliberate single-path surface, no Arc/Rc/Box/RefCell in signatures)
- err: 1 finding(s)
- serde: 1 finding(s)
- obs: clean (seeds ran: 6/0/0/9; eprintln hits are CLI output and test skips, remaining hits are `log::`-in-`catalog::` false matches)
- docs: 1 finding(s)
- perf: clean (seeds ran: 5/4/1; all hits cold-path error/audit code, static (unmeasured))
- conc: N/A (seeds: 0/0/0/0 all zero; no threads, locks, or atomics)
- async: clean (seeds ran: 17/0/0/4; all hits are #[tokio::test] harnesses and helpers; dependency_surface sync reads are the sanctioned blanket allow, U1 (d)4)
- unsafe: N/A (seeds 1-3: 0/0/0; seed 4 only - #![deny(unsafe_code)] lib.rs:13, manifest deny; on the U1 (d)8 deny list, zero blocks)
- ffi: N/A (seeds: 0/0/0/0 all zero; no FFI surface)
- macro: clean (seeds ran: 0/8/0/0; all 8 hits are the audit's proc_macro identifier, no macro definitions)
- test: 2 finding(s)