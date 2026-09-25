# d2bd-p4 - d2bd - part 4/8
Baseline: 6ebdd4cec22e6537d1376e83ff7a82b00a8492c0 | LOC audited: 10850 (excl. src/generated/**) | modules: composition.rs (lines 1-10070), plane_port.rs
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: composition.rs:1-10070, plane_port.rs (whole file)

## idiom
- clean: seeds ran: `for \w+ in 0\.\.` = 2, `impl (Default|From|PartialEq|Eq|Debug|Clone|Hash) for` = 0, `let mut \w+ = (String|Vec)::new\(\)` = 4; both index loops are test loops (8889, 8910) and all four `Vec::new()` accumulations are conditional-push loops in complex functions where an iterator pipeline would obscure early exits; no hand-written derive-replaceable impls and no `get_` field accessors.

## own
- clean: seeds ran: `\.clone\(\)` = 196, `\.to_owned\(\)|\.to_vec\(\)|\.to_string\(\)` = 313, `Rc<|RefCell<|Arc<Mutex<|Arc<RwLock<` = 0, `Cow<` = 0; sampled: 46 of 509 hits (every 11th); every sampled site is an Arc clone at a tokio::spawn/thread boundary, error-detail string construction, request/operation-id building, or a test fixture; the `Arc<tokio::sync::Mutex<...>>` shared state in `ServerState` and `ZoneLinkGatewayComposition` is the U10-sanctioned concurrent-state pattern (composition.rs:19, composition.rs:522).

## type
- clean: seeds ran: `fn validate_\w+|fn check_\w+` = 4, `is_\w+: bool|\w+_flag: bool` = 1, `(mode|kind|state): String` = 2; the four validate/check fns are one-shot boundary checks on wire/config input (correct per the skill), the bool is a parameter not a field flag, and the two `source_kind: String` fields are daemon-written registry records whose string values come from a closed enum match, not user state.

## api
- clean: seeds ran: `\bpub (fn|struct|enum|trait|type|const|mod) ` = 16, `pub .*\b(Arc|Rc|Box|RefCell)<` = 0, `^\s*pub use ` = 11; the pub surface (composition.rs:397-403, 414, 419, 431, 4156, 4184) is consumed by main.rs and d2bd's own integration tests (tests/mode_separation.rs, tests/core_composition.rs, tests/resource_operator_activation.rs), the `pub use` arms (composition.rs:120-230) are the house single-surface pattern, and no public signature carries Arc/Rc/Box/RefCell.

## err
- d2bd-p4#1 sev=medium blast=leaf effort=S verdict=actionable - `record_workload_availability_metrics` panics via `.expect("bounded workload availability tuple")` when the metric key set drifts from the label fns: `WORKLOAD_AVAILABILITY_STATES` (composition.rs:8097) + `WORKLOAD_PROVIDERS` (composition.rs:8090) live here, while `workload_availability_label`/`workload_provider_label` live in d2bd-runtime's workload_dispatch.rs, so adding a `WorkloadAvailability` variant compiles cleanly (the exhaustive match only forces the label fn update) but makes the daemon panic on the next workload List/Status - fix: seed the `counts` map from a single source of truth exported next to the label fns (e.g. `workload_availability_states()`/`workload_provider_labels()`), or replace the expect with a graceful `entry()`/skip so an unknown label degrades to a missing gauge instead of a panic - [packages/d2bd/src/composition.rs:8140, packages/d2bd-runtime/src/workload_dispatch.rs:104, packages/d2bd/src/composition.rs:8097]
  evidence: `\.unwrap\(\)|\.expect\(` seed = 154 hits across the lane; only four production expect sites exist (7574, 8140, 9214, 9256) and the other three name compiler-invisible invariants that the guards literally enforce (mutating_verb_preflight at 10071; ShellName literal at 7574); this one's invariant is maintained across a crate boundary.

## serde
- d2bd-p4#2 sev=medium blast=leaf effort=S verdict=actionable - `GatewayGuestConfigFile` and `GatewayGuestRelayConfigFile` deserialize user-written guest gateway config with `rename_all = "camelCase"` but no `deny_unknown_fields`, so a typo'd key is silently ignored and surfaces later as "Guest Relay namespace is unavailable" instead of a parse error - fix: add `#[serde(deny_unknown_fields)]` to both types (the `QemuMediaProbeRegistry*` records are daemon-written and may stay permissive); add a config-typo test to `load_gateway_guest_zone_link_options` - [packages/d2bd/src/composition.rs:4196, packages/d2bd/src/composition.rs:4205]
  evidence: `serde\((rename_all|deny_unknown_fields|try_from|untagged|flatten|default|skip_serializing_if)` seed = 4 hits (both pairs of types); no `deny_unknown_fields` anywhere in the lane; the repo's manifest-schema types use it as the admission-gate pattern.


## obs
- d2bd-p4#3 sev=medium blast=leaf effort=S verdict=actionable - the daemon's accept loop reports runtime errors with `eprintln!` (authorization refusal at 4099/4132, connection-handler failure at 4081/4132-ish, spawn failure at 4138) while the rest of the crate uses tracing and main.rs:146-152 installs a `tracing_subscriber`, so these error events bypass level filtering, structured fields, and the redaction gates; the daemon's stderr goes to the journal as unstructured prose - fix: replace the four `eprintln!` calls with `tracing::error!` events carrying named fields (`error = %error.message()`, `peer_uid`) - [packages/d2bd/src/composition.rs:4081, packages/d2bd/src/composition.rs:4099, packages/d2bd/src/composition.rs:4132, packages/d2bd/src/composition.rs:4138]
  evidence: `\bprintln!\(|\beprintln!\(` seed = 4 hits, all in serve()'s sync connection paths;`tracing::|log::` seed = 92 hits in the same lane, so eprintln is the exception not the norm.
- d2bd-p4#4 sev=low blast=leaf effort=S verdict=actionable - three lifecycle `tracing::info!` events are message-only with no named fields ("Guest-local ZoneLink transport Provider composed", "Guest target-control service composed", "autostart: nothing to do (empty plan)") and no enclosing span exists (0 `#[instrument]` hits in the lane), so the events cannot be filtered by zone/vm - fix: add named fields (`zone`, `guest_ref`, or `vm`) to the three events, or wrap them in instrumented callers - [packages/d2bd/src/composition.rs:4487, packages/d2bd/src/composition.rs:4538, packages/d2bd/src/composition.rs:5300]
  evidence: `(info|debug|warn|error|trace)!\("` seed = 3 hits (all three are the message-only events);`\.instrument\(|#\[instrument` = 0, so no span context carries those fields.


## docs
- d2bd-p4#5 sev=medium blast=leaf effort=S verdict=actionable - `pub async fn serve`, the daemon's primary entry point (composition.rs is `include!`d into lib.rs:183),has no doc comment at all, and `pub async fn lock_only` has none either; both return `Result` and carry no `# Errors` contract, so callers cannot learn from the docs what each loads/binds/runs and how it fails - fix: add a doc comment to `serve` (loads config, applies overrides, binds the operator socket, runs the accept loop; `# Errors` for config/IO/authz failures) and to `lock_only` - [packages/d2bd/src/composition.rs:3455, packages/d2bd/src/composition.rs:4828]
  evidence: `^\s*pub (fn|struct|enum|trait|const|type)` seed =  10 pub items;`/// # (Examples|Errors|Panics|Safety)` = 0 hits in the lane;`-> Result<` = 128 hits; ly the two undocumented pub entry points return Result without an Errors section.
- d2bd-p4#6 sev=low blast=leaf effort=S verdict=actionable - `StaticProviderComposition::new` is a pub constructor returning `Result<Self, AdmissionError>` with no doc comment and no `# Errors` section, so the mode_separation.rs callers must read the body to learn it fails on `AdmissionError` - fix: one-line doc plus a `# Errors` section naming `AdmissionError` - [packages/d2bd/src/composition.rs:438]
  evidence: static (unmeasured); pub-item seed = 10 hits and `-> Result<` = 128 hits; `new` is the only pub constructor without docs among the crate's public surface in this lane.


## perf
- clean: seeds ran: `format!\(` = 83, `Vec::new\(\)|VecDeque::new\(\)|HashMap::new\(\)|BTreeMap::new\(\)` = 19, `\.to_string\(\)` = 33; all format!/to_string sites are error-detail strings, operation-id/ref construction, one-shot probe/registry reads, or test fixtures - none sits in a loop over a hot request path; static (unmeasured), no benchmark exists in the crate.


## conc
- clean: seeds ran: `std::thread::|thread::spawn|thread::scope` = 4, `\bMutex<|\bRwLock<` = 23, `Atomic\w+|Ordering::` = 4, `thread_local!|unsafe impl (Send|Sync) for` = 0; the named threads are deliberate per-connection/dedicated worker threads (composition.rs:3650, 4124, 5546),the AtomicU64 stream-id counter uses Relaxed correctly (3437-3440),the Arc<AtomicBool> stop flag is clear shared state (3805),and all 23 Mutex/RwLock sites are `tokio::sync::Mutex` in the U10-sanctioned production state tables (composition.rs:19) or test fakes.



## async
- clean: seeds ran: `async fn|async move|\.await` = 155, `tokio::spawn|spawn_blocking|JoinSet|select!\(|join!\(` = 3, `tokio::sync::(Mutex|RwLock|Notify)` = 50, `#\[tokio::(main|test)\]|Runtime::block_on` = 0 (the bare seed misses the attribute-carrying `#[tokio::test(flavor = "multi_thread")]` forms, which appear 10 times in plane_port.rs tests);`drive_sync` (composition.rs:210) has the sanctioned "synchronous path" inline allow and `block_in_place` is a documented no-op on dededicated threads (composition.rs:200-213),the select! cancellation in serve_guest (4559-4580) aborts serving then awaits it - the correct shutdown shape,and no guard is held across an .await beyond the async-gate scanner's covered set.


## unsafe
- N/A (seeds: `\bunsafe \{|\bunsafe fn|\bunsafe impl|\bunsafe extern` = 0, `// SAFETY:` = 0, `transmute|from_raw|MaybeUninit|mem::zeroed` = 0, `unsafe_code` = 0 - all zero; the d2bd crate's unsafe sites (22 crate-wide per U1 (e))) live in other parts of composition.rs and sibling files, not in this part).


## ffi
- N/A (seeds: `extern "C"|no_mangle|unsafe\(link_section` = 0, `catch_unwind` = 0, `repr\(C\)|repr\(transparent\)` = 0, `CStr|CString|c_char` = 0 - all zero; no FFI-boundary code exists in this part).


## macro
- N/A (seeds: `macro_rules!` = 0, `proc_macro|syn::|quote!` = 0, `\$crate` = 0, `to_compile_error|new_spanned` = 0 - all zero; no macro definitions or proc-macro usage in this part).


## test
- clean: seeds ran: `#\[test\]|#\[tokio::test\]` = 21 (plus 10 `#[tokio::test(flavor = "multi_thread")]` in plane_port.rs that the bare seed does not match), `assert_eq!\(|assert_ne!\(|assert!\(` = 87, `proptest!|insta::assert|rstest` = 0, `#\[ignore\]` = 0; the 21 tests assert behavior (topology resolution, gateway session establishment/refusal, cursor adoption, fencing, metric availability counts, workload dispatch denial, plane claim refusal/ordering/release) with hand-written expected values, deterministic (no network, no sleeps), and the async tests use multi_thread flavor per the async skill; no test restates implementation or cannot fail.


## Coverage
- idiom: clean (seeds ran: 2/0/4; both index loops are test loops and all four Vec::new accumulations are conditional-push loops with early exits; no derive-replaceable hand-written impls)
 
- own: clean (seeds ran: 196/313/0/0; sampled: 46 of 509 hits; every sampled clone/to_owned is an Arc clone at a spawn/thread boundary, error-detail construction, request building, or test fixture; Rc/RefCell/Cow absent; Arc<tokio::sync::Mutex> shared state is the U10-sanctioned pattern)
- type: clean (seeds ran: 4/1/2; all four validate/check fns are one-shot boundary checks, the bool is a parameter not a flag, the String fields are daemon-written registry records)
- api: clean (seeds ran: 16/0/11; pub surface consumed by main.rs and d2bd tests, pub use arms are the house single-surface pattern, no internals leak into signatures)
- err: 1 finding(s)
- serde: 1 finding(s)
- obs: 2 finding(s)
- docs: 2 finding(s)
- perf: clean (seeds ran: 83/19/33; no format!/allocation sits in a hot request loop; static (unmeasured))
- conc: clean(seeds ran: 4/23/4/0; dedicated handler threads, Relaxed counter, clear stop flag, U10-sanctioned tokio mutexes)
 
- async: clean (seeds ran: 155/3/50/0; drive_sync sanctioned inline allow, select! shutdown shape correct, no guards across .await beyond gate coverage; the 0 for seed 4 is a seed-regex artifact (attribute-carrying tokio::test forms missed))
- unsafe: N/A (seeds: 0/0/0/0 all zero; no unsafe constructs in this part - crate-level sites live elsewhere)
- ffi: N/A (seeds: 0/0/0/0 all zero; no FFI-boundary code in this part)
- macro: N/A (seeds: 0/0/0/0 all zero; no macros in this part)
- test: clean(seeds ran: 21/87/0/0; behavior-focused, deterministic, multi_thread-flavored async tests; no property/snapshot tooling needed for a daemon composition surface)