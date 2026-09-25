# d2b-zone-routing - d2b-zone-routing
Baseline: 6ebdd4cec22e6537d1376e83ff7a82b00a8492c0 | LOC audited: 10120 (excl. src/generated/**; src 8214 + tests 1906) | modules: engine, enrollment, resolver, router, service, serving
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: whole crate

## idiom
- d2b-zone-routing#1 sev=low blast=leaf effort=S verdict=actionable - `SealedZoneTopology::longest_suffix_match` walks `for start in 0..labels.len()` with an inline `continue`, where the same logic is a `find_map` over the index range - fix: replace the loop with `(0..labels.len()).find_map(|start| { let Ok(suffix) = ZonePath::new(labels[start..].to_vec()) else { return None; }; self.zones.get(&suffix) })` - [packages/d2b-zone-routing/src/resolver.rs:144]
  evidence: seed `for \w+ in 0\.\.` = 2 hits; only this site is in production code (service.rs:1793 is a fixed-count test loop).
- d2b-zone-routing#2 sev=low blast=leaf effort=S verdict=actionable - `ZoneTopologyRequest` carries a hand-written `impl Default` that a field-wise derive reproduces exactly - fix: delete the manual impl and add `#[derive(Default)]` to the struct - [packages/d2b-zone-routing/src/service.rs:311]
  evidence: seed `impl (Default|From|PartialEq|Eq|Debug|Clone|Hash) for` = 5 hits; only this site's derive would be field-wise identical (the other four Default impls preserve non-zero frozen bounds and must stay manual).

## own
- d2b-zone-routing#3 sev=low blast=leaf effort=M verdict=actionable - In `ZoneRouteAdmission::consume`, the snapshot's zone pair is copied from `expected` with two `ZonePath` clones per consumed route admission, though `expected` is already owned by the match arm and only compared afterwards - fix: compare every non-zone field first, then move `expected.source_zone`/`expected.target_zone` intosnapshot (or split `validate_snapshot` into a zone-pair phase taking `expected` by value), removing the two clones - [packages/d2b-zone-routing/src/engine.rs:345, packages/d2b-zone-routing/src/engine.rs:346]
  evidence: seed `\.clone\(\)` = 141 hits; this site is the only production clone of a value already owned by the enclosing scope (all other clones buy a second owner or test fixture state).

## type
- clean: seeds ran: `fn validate_\w+|fn check_\w+` = 3 (`validate_snapshot`, `validate_session_binding`, `check_current`), `is_\w+: bool|\w+_flag: bool` = 0, `(mode|kind|state): String` = 0; all three hits are runtime comparisons against sealed admission evidence (target substitution, session binding, daemon time), not parse-once type candidates, and there is no flag soup or stringly-typed state.



## api
- clean: seeds ran: `\bpub (fn|struct|enum|trait|type|const|mod)` = 207, `pub .*\b(Arc|Rc|Box|RefCell)<` = 1, `^\s*pub use` = 0; every public item has exactly one path and a doc comment; the single `Arc<dyn Fn>` signature hit is the deliberate clock/placements seam (`pub type ZoneEnrollmentPlacements` and `ZoneEnrollmentAuthority::new`), no `Box`/`Rc`/`RefCell` appears in any signature, and `test-support`-gated `for_test` constructors are consumed cross-crate by the vector suites as intended.



## err
- clean: seeds ran: `\.unwrap\(\)|\.expect\(` = 207, `let _ = |\.ok\(\);` = 1, `\bpanic!\(|\bunreachable!\(|\btodo!\(|\bunimplemented!\(` = 15, `enum \w*Error` = 2; every unwrap/expect/panic hit sits in `#[cfg(test)]` or `#[cfg(any(test, feature = "test-support")))]` code or test helpers, the one `let _` is a stray test line (flagged under `test`),and the two error enums (`RouterError`, `ZoneEnrollmentServeError`) are closed with stable kebab labels and `std::error::Error` impls

## serde
- N/A (seeds: 0/0/0/0 all zero; no serde derives, serde attributes, hand-written `Deserialize` impls, or `serde_json` anywhere - this crate crosses no wire boundary; wire call types live in `d2b-contracts-zone-session`)

## obs
- N/A (seeds: 0/0/0/0 all zero; the crate has no `tracing`/`log` dependency in Cargo.toml - there is no telemetry surface to judge; zero `println!` in src)

## docs
- d2b-zone-routing#4 sev=low blast=leaf effort=M verdict=actionable - The crate's 47 `-> Result<` public signatures document failure modes in prose paragraphs (e.g., `SealedZoneTopology::seal`, `ZoneServiceLimits::new`, `ZoneEnrollmentAuthority::with_lifetime`) but zero canonical `# Errors`/`# Panics` sections exist anywhere, so rustdoc index and IDEs lose a scannable contract - fix: add a `# Errors` section to the public validators/constructors that enforce conditions (seal, the `Limits`/`Expectation`/`Authority` constructors, `with_runtime_admission`), keeping the prose as depth beneath it - [packages/d2b-zone-routing/src/resolver.rs:80, packages/d2b-zone-routing/src/service.rs:208, packages/d2b-zone-routing/src/enrollment.rs:424]
  evidence: seed `^\s*pub (fn|struct|enum|trait|const|type)` = 201, `/// # (Examples|Errors|Panics|Safety)` = 0, `-> Result<` = 47

## perf
- clean: seeds ran: `format!\(` = 15, `Vec::new\(\)|VecDeque::new\(\)|HashMap::new\(\)|BTreeMap::new\(\)` = 21, `\.to_string\(\)` = 0; all 15 `format!` hits are `cfg(test)` debug-render assertions and synthetic fingerprint builders (engine.rs:1959, resolver.rs:526, service.rs:1242), and the 21 collection initializers are bounded staging tables with ceilings (`MAX_ZONE_PARENT_ENTRIES`, `MAX_ZONE_ROUTE_ENTRIES`, `MAX_IDEMPOTENCY_ROWS`) - no measured hot path exists to name

## conc
- clean: seeds ran: `std::thread::|thread::spawn|thread::scope` = 0, `\bMutex<|\bRwLock<` = 4, `Atomic\w+|Ordering::` = 34, `thread_local!|unsafe impl (Send|Sync) for` = 0; the four `Mutex` sites are the single-use admission states and the router/exec tables, each `std::sync::Mutex` locked with per-site `#[allow(clippy::disallowed_methods, reason = "synchronous path")]` (no suspension point), atomics are `AtomicBool` revoke flags loaded Acquire/stored Release plus test clocks; no manual `Send`/`Sync` claims or `thread_local!` in production

## async
- d2b-zone-routing#5 sev=medium blast=leaf effort=M verdict=actionable - `ZoneEnrollmentServer::serve` commits the link FSM synchronously (PSK burn, enrollment record seal) inside `serve_bootstrap`/`serve_enroll` and then `.await`s the reply write `transport.send)...)`, so a `serve` future dropped between the mutation and the send leaves the link mid-transition and the peer never sees the reply - fix: make the FSM commit + encoded-reply write one non-cancellable unit (and document that dropping the task mid-send closes the connection as the peer's only signal), or make the operation resumable by deferring the transition until the reply write succeeds where the FSM allows - [packages/d2b-zone-routing/src/serving.rs:195, packages/d2b-zone-routing/src/serving.rs:207]
  evidence: seed `async fn|async move|\.await` = 6 hits (all in serving.rs:180-319); no tokio spawn/spawn_blocking/select!/join! or tokio sync types in src

## unsafe
- N/A (seeds: 0/0/0/0 all zero; no `unsafe` blocks/fns/impls, no `// SAFETY:` comments, and the manifest carries no `unsafe_code` setting - U1 ledger (d) 8 records it ABSENT here, so there is no unsafe surface to judge)

## ffi
- N/A (seeds: 0/0/0/0 all zero; no `extern "C"`, `no_mangle`, `catch_unwind`, `repr(C)`/`repr(transparent)`, or `CStr`/`CString` anywhere - no foreign-calling boundary exists)

## macro
- clean: seeds ran: `macro_rules!` = 1, `proc_macro|syn::|quote!` = 0, `\$crate` = 0, `to_compile_error|new_spanned` = 0; the single `macro_rules!` (`redacted_debug!`, service.rs:116) is impl-per-type `Debug` generation - a genuine macro answer - with the narrow `ident` fragment, a fully-qualified `::core::fmt` expansion, and no crate-path or error machinery to judge

## test
- d2b-zone-routing#6 sev=low blast=leaf effort=S verdict=actionable - `every_reason_the_engine_can_produce_is_covered_by_this_suite` checks only that the 16 named closed reasons have distinct `label()`s, not that the suite produces any of them - the name promises a coverage property the row never asserts - fix: rename it to `every_engine_reason_has_a_distinct_wire_label` and, if coverage is actually wanted, record the reasons each vector produced and assert the set at the end - [packages/d2b-zone-routing/src/engine.rs:3333]
  evidence: test seed `#\[test\]|#\[tokio::test\]` = 143; this test's body only sorts and dedups `label()` values, with no produced-reason tracking
- d2b-zone-routing#7 sev=low blast=leaf effort=S verdict=actionable - `durable_exec_table_is_bounded_to_ephemeral_processes` ends with a dead `let _ = ZoneId::parse("dev").unwrap();` line that exercises nothing and exists only to use an import - fix: delete the line and the now-unused `ZoneId` import from the test module - [packages/d2b-zone-routing/src/router.rs:517]
  evidence: err seed `let _ =` = 1 hit; the single hit is this stray test line

## Coverage
- idiom: 2 finding(s)
- own: 1 finding(s)
- type: clean (seeds ran: 3/0/0; all hits are runtime comparisons against sealed evidence, not type-model candidates)
- api: clean (seeds ran: 207/1/0; single-signature `Arc` is a deliberate seam; no internals or second paths exposed)
- err: clean (seeds ran: 207/1/15/2; no production panic or swallowed Result; error enums closed with stable labels)
- serde: N/A (seeds: 0/0/0/0 all zero; crate crosses no wire boundary)
- obs: N/A (seeds: 0/0/0/0 all zero; no tracing/log dependency - no telemetry surface)
- docs: 1 finding(s)
- perf: clean (seeds ran: 15/21/0; all format! hits are test paths; collection sites bounded, unmeasured)
- conc: clean (seeds ran: 0/4/34/0; Mutex sites are sanctioned synchronous surfaces; atomics are Acquire/Release pairs)
- async: 1 finding(s)
- unsafe: N/A (seeds: 0/0/0/0 all zero; no unsafe surface - manifest setting absent per U1 d8)
- ffi: N/A (seeds: 0/0/0/0 all zero; no foreign-calling boundary)
- macro: clean (seeds ran: 1/0/0/0; single macro is justified impl-per-type generation)
- test: 2 finding(s)