# d2b-resource-api-p2 - d2b-resource-api - part 2/2
Baseline: 6ebdd4cec22e6537d1376e83ff7a82b00a8492c0 | LOC audited: 6960 (excl. src/generated/**) | modules: authz, admission, error, identity, manager_backend (tests), lib
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: src/authz.rs, src/manager_backend/**, src/admission.rs, src/error.rs, src/identity.rs, src/lib.rs

## idiom
- d2b-resource-api-p2#1 sev=low blast=leaf effort=M verdict=actionable - A6 not-applied: 17 hand-written redaction Debug impls in authz.rs (15) and admission.rs (2) where the exported `redacted_debug!` macro exists - fix: fold byte-compatible impls to `redacted_debug!` or extend the macro with a count-preserving form, updating the Debug-shape pinning tests in the same change - [packages/d2b-resource-api/src/authz.rs:106, packages/d2b-resource-api/src/authz.rs:300, packages/d2b-resource-api/src/authz.rs:356, packages/d2b-resource-api/src/authz.rs:377, packages/d2b-resource-api/src/authz.rs:530, packages/d2b-resource-api/src/authz.rs:546, packages/d2b-resource-api/src/authz.rs:560, packages/d2b-resource-api/src/authz.rs:575, packages/d2b-resource-api/src/authz.rs:594, packages/d2b-resource-api/src/authz.rs:604, packages/d2b-resource-api/src/authz.rs:770, packages/d2b-resource-api/src/authz.rs:800, packages/d2b-resource-api/src/authz.rs:1090, packages/d2b-resource-api/src/authz.rs:1120, packages/d2b-resource-api/src/authz.rs:1270, packages/d2b-resource-api/src/admission.rs:60, packages/d2b-resource-api/src/admission.rs:300, packages/d2b-resource-api/src/authz.rs:3328, packages/d2b-resource-api/src/admission.rs:695]
  evidence: idiom seeds = 1/1/4 hits; A6 row at docs/explanation/over-engineering-audit-record.md:459 (not applied, no refusal reason; sites still match at 6ebdd4cec); the macro prints only `Type(<redacted>)` (packages/d2b-contracts-resource/src/v3/execution_policy.rs:22-28), so the count/presence fields these impls keep are not byte-compatible without a macro extension, and the shapes are pinned by the two Debug tests
- d2b-resource-api-p2#2 sev=low blast=leaf effort=S verdict=actionable - unformatted `use` lines inside a fn body break `cargo fmt --check` - fix: reindent to 4 spaces and drop the inner-brace spacing - [packages/d2b-resource-api/src/manager_backend/tests.rs:1045, packages/d2b-resource-api/src/manager_backend/tests.rs:1046]
  evidence: idiom seeds = 1/1/4 hits; the two `use` lines sit at mixed columns inside `converted_type_status_layers_round_trip_through_their_typed_decoders` (no rustfmt.toml in the repo, default rules apply)

## own
- d2b-resource-api-p2#3 sev=low blast=leaf effort=S verdict=actionable - redundant `.cloned()` in `StoreAdmissionBinding::verify`: `mutations` is already owned after the destructure, so the iterator clones every mutation before `prepare_mutation` consumes it - fix: `mutations.into_iter().map(prepare_mutation)` - [packages/d2b-resource-api/src/admission.rs:338, packages/d2b-resource-api/src/admission.rs:344, packages/d2b-resource-api/src/admission.rs:345]
  evidence: seed `\.clone\(\)` = 133 hits in scope (74 authz.rs, 53 tests.rs, 6 admission.rs); `prepare_mutation` takes `StoreMutation` by value (admission.rs:417), so `into_iter()` compiles without the clone; all other clones in this part are explainable (owned outputs, Arc clones, test fixtures)

## type
- clean: seeds `fn validate_\w+|fn check_\w+` = 2 (admission.rs:456,483), `is_\w+: bool|\w+_flag: bool` = 0, `(mode|kind|state): String` = 0; the two validate hits are boundary admission gates on untrusted wire bytes (the card's parse-once-at-boundary shape), and no boolean-flag or stringly-typed state fields exist in this part

## api
- clean: seeds `pub (fn|struct|enum|trait|type|const|mod)` = 81, `pub .*Arc|Rc|Box|RefCell<` = 0, `pub use` = 13; the surface is deliberate: private fields plus compile_fail doctests on AuthorizationLease/AdmittedMutation/AuthenticatedSubjectContext, the lib.rs re-export arms are the house single-surface pattern, and no internals or dependency types appear in public signatures

## err
- clean: seeds `\.unwrap\(\)|\.expect\(` = 379, `let _ = |\.ok\(\);` = 7, `\bpanic!\(|\bunreachable!\(|\btodo!\(|\bunimplemented!\(` = 17, `enum \w*Error` = 3; every production hit falls in the card's false-positive classes: one expect on a literal catalog (authz.rs:88), two invariant-named expects on bounded batch ordinals (admission.rs:206,208), fail-closed `unwrap_or_else` fallbacks (error.rs:74-78); panics and unwraps are otherwise confined to tests, and the three error enums (StoreSealHandoffError, AdmissionError, AuthorizationPolicyError) all carry Display plus Error

## serde
- clean: seeds `derive(...Serialize` = 0, `serde(...)` = 0, `impl .*Deserialize.*for` = 0, `serde_json::from_|to_` = 30; the two production `from_slice` parses (authz.rs:409,420) are the typed admission boundary where canonical JSON becomes RoleSpec/RoleBindingSpec with error collapse to RoleSchema/BindingShape, and the remaining hits are test payloads

## obs
- clean: seeds `\bprintln!\(|\beprintln!\(` = 0, `(info|debug|warn|error|trace)!\("` = 0, `\.instrument\(|#\[instrument` = 0, `tracing::|log::` = 0 real hits (the seed only matched `catalog::` substrings; tracing is used only in part 1's manager_backend.rs:194)

## docs
- d2b-resource-api-p2#4 sev=medium blast=leaf effort=S verdict=actionable - nine pub methods on the evaluator surface are undocumented, including `NativeAuthorizer::authorize` (the security decision entry returning nine AuthorizationDenial variants) and `take_store_seal` (which hands off an ownership-bearing seal acceptor) - fix: add doc comments with `# Errors` sections enumerating the denial variants on authorize, and one-line contracts on the remaining eight - [packages/d2b-resource-api/src/authz.rs:630, packages/d2b-resource-api/src/authz.rs:864, packages/d2b-resource-api/src/authz.rs:912, packages/d2b-resource-api/src/authz.rs:1093, packages/d2b-resource-api/src/authz.rs:1479, packages/d2b-resource-api/src/authz.rs:1505, packages/d2b-resource-api/src/authz.rs:1522, packages/d2b-resource-api/src/authz.rs:1550, packages/d2b-resource-api/src/authz.rs:1615]
  evidence: docs seed 1 (`^\s*pub (fn|struct|enum|trait|const|type)`) = 81 hits, seed 2 (`/// # ...`) = 0, seed 3 (`-> Result<`) = 48; the `///` scan over authz.rs shows no doc comment above these nine lines, and `missing_docs` is not enabled anywhere (proposal only)

## perf
- d2b-resource-api-p2#5 sev=low blast=leaf effort=S verdict=actionable - `compile_authorization_facts` grows its roles and bindings Vecs by push although the row count is known upfront - fix: `Vec::with_capacity(rows.len())` for both - [packages/d2b-resource-api/src/authz.rs:457, packages/d2b-resource-api/src/authz.rs:458]
  evidence: static (unmeasured); seed `Vec::new\(\)|VecDeque::new\(\)|HashMap::new\(\)|BTreeMap::new\(\)` = 22 hits in scope, and the loop over `rows` at authz.rs:461 bounds both collections

## conc
- clean: seeds `std::thread::|thread::spawn|thread::scope` = 2, `\bMutex<|\bRwLock<` = 5, `Atomic\w+|Ordering::` = 0, `thread_local!|unsafe impl (Send|Sync) for` = 0; every Mutex/RwLock site (authz.rs:1401,1404,1407, admission.rs:52,393) carries the sanctioned `#[allow(clippy::disallowed_methods, reason = "synchronous path")]` allow, and the two thread::spawn hits are the sanctioned cfg(test) linearization test (authz.rs:2686,2707)

## async
- clean: seeds `async fn|async move|\.await` = 77, `tokio::spawn|spawn_blocking|JoinSet|select!\(|join!\(` = 0, `tokio::sync::(Mutex|RwLock|Notify)` = 0, `#\[tokio::(main|test)\]|Runtime::block_on` = 13; every hit is the test harness in manager_backend/tests.rs under the sanctioned `#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]` allow, and the production surface in this part is deliberately synchronous (admission.rs:356-359)

## unsafe
- N/A: seeds `\bunsafe \{|\bunsafe fn|\bunsafe impl|\bunsafe extern` = 0, `// SAFETY:` = 0, `transmute|from_raw|MaybeUninit|mem::zeroed` = 0 all zero; the manifest sets `unsafe_code = "forbid"` (Cargo.toml [lints.rust])

## ffi
- N/A: seeds `extern "C"|no_mangle|unsafe\(link_section` = 0, `catch_unwind` = 0, `repr\(C\)|repr\(transparent\)` = 0, `CStr|CString|c_char` = 0 all zero

## macro
- clean: seeds `macro_rules!` = 1, `proc_macro|syn::|quote!` = 0, `\$crate` = 0, `to_compile_error|new_spanned` = 0; the single hit is the test-only `impl_has_error!` helper (tests.rs:463), the card-listed test-helper false positive

## test
- d2b-resource-api-p2#6 sev=medium blast=leaf effort=S verdict=actionable - `list_returns_snapshot_revision_and_watch_refuses_until_wired` compares the wire snapshot's epoch-seconds half against `SystemTime::now()` taken after the list round-trip, so a second boundary crossing between the two instants flakes the test - fix: assert the mapping with a one-second tolerance or inject the clock - [packages/d2b-resource-api/src/manager_backend/tests.rs:1459, packages/d2b-resource-api/src/manager_backend/tests.rs:1460, packages/d2b-resource-api/src/manager_backend/tests.rs:1461]
  evidence: seed `#\[test\]|#\[tokio::test\]` = 57 hits, `assert_eq!\(|assert_ne!\(|assert!\(` = 242, `proptest!|insta::assert|rstest` = 0, `#\[ignore\]` = 0; the compared values are read at different instants (the manager computes `snapshot_revision` before the awaits that precede the assertion), violating the determinism rule

## Coverage
- idiom: 2 finding(s)
- own: 1 finding(s)
- type: clean (seeds ran: 2/0/0; the two validate_* hits are boundary admission gates on untrusted wire bytes, admission.rs:456,483)
- api: clean (seeds ran: 81/0/13; deliberate private-field surface with compile_fail doctests, lib.rs re-export arms are the house pattern, no Arc/Rc/Box/RefCell in pub signatures)
- err: clean (seeds ran: 379/7/17/3; production hits are all card-listed false-positive classes, panics confined to tests, error enums carry Display plus Error)
- serde: clean (seeds ran: 0/0/0/30; the two production from_slice parses are the typed admission boundary, the rest is test payloads)
- obs: clean (seeds ran: 0/0/0/0; the log:: seed only matched catalog:: substrings, tracing lives in part 1's manager_backend.rs:194)
- docs: 1 finding(s)
- perf: 1 finding(s)
- conc: clean (seeds ran: 2/5/0/0; all Mutex/RwLock sites carry the sanctioned synchronous-path allow, thread::spawn confined to the sanctioned cfg(test) linearization test)
- async: clean (seeds ran: 77/0/0/13; every hit is the sanctioned test harness in manager_backend/tests.rs, production here is deliberately synchronous)
- unsafe: N/A (seeds: 0/0/0 all zero; manifest sets unsafe_code = "forbid")
- ffi: N/A (seeds: 0/0/0/0 all zero)
- macro: clean (seeds ran: 1/0/0/0; the single hit is the test-only impl_has_error! helper, a card-listed false positive)
- test: 1 finding(s)