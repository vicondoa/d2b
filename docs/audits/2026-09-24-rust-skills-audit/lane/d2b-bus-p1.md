# d2b-bus-p1 - d2b-bus - part 1/2
Baseline: 6ebdd4cec22e6537d1376e83ff7a82b00a8492c0 | LOC audited: 10547 (excl. src/generated/**) | modules: router, authorization, registry, metrics
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: src/router.rs, src/authorization.rs, src/registry.rs, src/metrics.rs

## idiom
- d2b-bus-p1#1 sev=low blast=leaf effort=S verdict=actionable - AuthoritativeUnixSubjectResolver::resolve_for_service collects matching subject indices into a Vec and indexes [0], allocating and double-scanning where a take-two iterator would do - fix: replace the collect-then-index with subjects.iter().enumerate().filter_map(...) checked via next() then next().is_some() - [packages/d2b-bus/src/router.rs:1773-1781]
  evidence: seeds `for \w+ in 0\.\.` = 4 (3 test loops, 1 fixed-round Feistel loop at router.rs:2641), `impl (Default|From|PartialEq|Eq|Debug|Clone|Hash) for` = 3 (invariant-preserving Default at router.rs:148, cfg-macro test impls at 2164-2171), `let mut \w+ = (String|Vec)::new\(\)` = 0; the collect-then-index shape is the only production accumulation

## own
- d2b-bus-p1#2 sev=low blast=leaf effort=S verdict=actionable - ResourceCall::authorization_request clones the whole AssignmentIdentity and mutation Vec just to learn whether ScopedCommitTransport::new rejects them, and invoke clones the same pair again to build the real transport - fix: add a reference-taking ScopedCommitTransport::validate(&AssignmentIdentity, &[ScopedResourceMutation]) in d2b-core-controller and call it from authorization_request so the validation clone disappears - [packages/d2b-bus/src/router.rs:480, packages/d2b-bus/src/router.rs:2928]
  evidence: seeds `\.clone\(\)` = 316 (260 router + 41 authorization + 15 registry; production sites are owned-value constructions for RouteKey/SessionAuthorizationRequest/Arc handles), `\.to_owned\(\)|\.to_vec\(\)|\.to_string\(\)` = 44, `Rc<|RefCell<|Arc<Mutex<|Arc<RwLock<` = 4 (authorization.rs:32-63 external ControllerAssignmentRegistry Arc, deliberate), `Cow<` = 0; only the 480/2928 pair clones for validation

## type
- d2b-bus-p1#3 sev=low blast=leaf effort=M verdict=actionable - ResourceQuery carries assignment: Option<AssignmentIdentity> and scope: Option<ScopedResourceScope> that are always both Some or both None, with a runtime validate_scoped re-check at every use site to keep the pair in sync - fix: fold the pair into one Option<(AssignmentIdentity, ScopedResourceScope)> or a ScopedQuery struct so the one-Some state is unrepresentable and the re-validation disappears - [packages/d2b-bus/src/router.rs:218-219, packages/d2b-bus/src/router.rs:298-337]
  evidence: seeds `fn validate_\w+|fn check_\w+` = 7 (boundary validators), `is_\w+: bool|\w+_flag: bool` = 0, `(mode|kind|state): String` = 0; the Option pair is the named skill smell, enforced only by construction plus runtime checks
- d2b-bus-p1#4 sev=low blast=leaf effort=M verdict=actionable - UnixSubjectRecord holds expected_peer: Option<PeerCredentials> and expected_peer_uid: Option<u32> where exactly one is always Some, and bind() ORs the two options at match time as if the state were open - fix: replace the pair with an enum (Exact(PeerCredentials) | Uid(u32)) so the exactly-one invariant is structural and the runtime OR branch disappears - [packages/d2b-bus/src/router.rs:1445-1446, packages/d2b-bus/src/router.rs:1667-1674]
  evidence: seeds `fn validate_\w+|fn check_\w+` = 7, `is_\w+: bool|\w+_flag: bool` = 0, `(mode|kind|state): String` = 0; the pair is enforced by the two constructor families (new vs guest_for_uid/provider_for_uid) and branched on at bind and resolve_for_service

## api
- d2b-bus-p1#5 sev=medium blast=leaf effort=S verdict=actionable - ZoneBus exposes eight pub constructors but only new, with_interaction_subject_issuer, and with_clock_observer_and_metrics_and_interaction_subject_issuer have production callers; with_observer, with_observer_and_metrics, with_clock, with_clock_and_observer, and with_clock_observer_and_metrics are internal delegation rungs or test-only - fix: keep the three live constructors pub, move with_clock/with_clock_observer_and_metrics under #[cfg(test)] or pub(crate), and delete or fold with_observer/with_observer_and_metrics/with_clock_and_observer - [packages/d2b-bus/src/router.rs:1208, packages/d2b-bus/src/router.rs:1224, packages/d2b-bus/src/router.rs:1258, packages/d2b-bus/src/router.rs:1267, packages/d2b-bus/src/router.rs:1287]
  evidence: census: `ZoneBus::` over packages/ = 4 files; external ctor calls = new (d2bd/src/resource_runtime.rs:3426, d2bd-runtime/src/resource_runtime_support.rs:3432), with_interaction_subject_issuer (d2bd/src/interaction_composition.rs:4879), with_clock_observer_and_metrics_and_interaction_subject_issuer (d2bd/src/interaction_composition.rs:6976); with_clock/with_clock_observer_and_metrics = test-only (router.rs:4946, 5422, session_seam_tests.rs:578); the remaining three = 0 callers anywhere; prior relay-island surface cut U12 applied (docs/explanation/over-engineering-audit-record.md:830) and its cross-crate ownership refusal (docs/audits/2026-09-23-ponytail-audit/README.md:153) consulted, not re-proposed
- d2b-bus-p1#6 sev=medium blast=leaf effort=S verdict=actionable - native_authorizer() returns Arc<NativeAuthorizer> in a pub signature on both BusAuthorizer and ZoneBus and has zero callers anywhere in the workspace, so the shared-authority accessor is dead surface that also leaks the Arc type - fix: remove both accessors or reduce them to pub(crate) until a daemon consumer exists - [packages/d2b-bus/src/authorization.rs:75, packages/d2b-bus/src/router.rs:1415-1416]
  evidence: seed `pub .*\b(Arc|Rc|Box|RefCell)<` = 2 hits, both this accessor pair; census: `native_authorizer` over packages/, nixos-modules/, tests/, docs/reference/, labs/ = 2 hits, both inside d2b-bus (definition plus the ZoneBus delegation)

## err
- clean: seeds `\.unwrap\(\)|\.expect\(` = 459 (318 router + 133 authorization + 2 registry + 6 metrics; production sites are only router.rs:90 and 4158 named-invariant expects plus the fixed-ref expects at 2219-2222 and 3119-3230, all card false positives), `let _ = |\.ok\(\);` = 31 (oneshot best-effort sends and deliberate metric-emit swallows), `\bpanic!\(|\bunreachable!\(|\btodo!\(|\bunimplemented!\(` = 8 (7 test panics, 1 unreachable! on the impossible AssignmentVerb::CommitBatch variant at router.rs:712), `enum \w*Error` = 5 (closed taxonomies with class accessors, no string-matching callers)

## serde
- clean: seeds `derive\([^)]*(De)?[Ss]erialize` = 0, `serde\((rename_all|deny_unknown_fields|try_from|untagged|flatten|default|skip_serializing_if)` = 0, `impl .*Deserialize.*for` = 0, `serde_json::from_|serde_json::to_` = 2 (metrics.rs:427 production frame construction following d2b-telemetry's emit_metric pattern, authorization.rs:933 test helper); no derives, no deserialization, no untrusted input crossing in this partition

## obs
- N/A: seeds `\bprintln!\(|\beprintln!\(` = 0, `(info|debug|warn|error|trace)!\(\"` = 0, `\.instrument\(|#\[instrument` = 0, `tracing::|log::` = 0 real (the 13 raw matches are ApiCatalog:: false positives); the crate declares no tracing/log dependency in packages/d2b-bus/Cargo.toml

## docs
- d2b-bus-p1#7 sev=medium blast=leaf effort=S verdict=actionable - The exported observer contract BusEvent, BusFailureReason, BusObserver, and NoopBusObserver carry no doc comments, so the semantics of the 17 failure reasons and when record fires are undocumented for the d2bd consumer - fix: add module-level or item docs stating when each event is recorded and what each BusFailureReason variant means - [packages/d2b-bus/src/router.rs:1126, packages/d2b-bus/src/router.rs:1135, packages/d2b-bus/src/router.rs:1187, packages/d2b-bus/src/router.rs:1192]
  evidence: seed `^\s*pub (fn|struct|enum|trait|const|type)` = 158; these four items have no preceding /// line while every neighboring item does; NoopBusObserver is consumed by d2bd/src/interaction_composition.rs:6981
- d2b-bus-p1#8 sev=low blast=leaf effort=S verdict=actionable - DEFAULT_MAX_ROUTES_PER_SESSION and DEFAULT_MAX_TOTAL_ROUTES are pub consts without docs while their sibling DEFAULT_MAX_PAYLOAD_BYTES has one - fix: add one-line docs naming the bound each constant sets - [packages/d2b-bus/src/router.rs:67-68]
  evidence: seed `^\s*pub (fn|struct|enum|trait|const|type)` = 158; lines 67-68 have no preceding /// comment
- d2b-bus-p1#9 sev=low blast=leaf effort=S verdict=actionable - CommittedInteractionSubjectInstallBody (consumed by d2bd), AuthorizationErrorClass, EndpointSessionFailure::class/code/remediation, and the metrics label accessors are pub items without doc comments - fix: add one-line docs to each, at least on the struct and the class enum - [packages/d2b-bus/src/router.rs:1895, packages/d2b-bus/src/authorization.rs:401, packages/d2b-bus/src/registry.rs:259-267, packages/d2b-bus/src/metrics.rs:110]
  evidence: seed `^\s*pub (fn|struct|enum|trait|const|type)` = 158; these items have no preceding /// line; CommittedInteractionSubjectInstallBody is constructed by d2bd/src/resource_runtime.rs:591
- d2b-bus-p1#10 sev=low blast=leaf effort=S verdict=actionable - No pub Result-returning item carries a canonical # Errors section anywhere in the partition (94 Result-returning pub items, zero sections), so the failure conditions of non-obvious APIs such as BusIngress::invoke and ZoneRegistrar::register_component_session are undocumented - fix: add # Errors sections to the non-obvious Result-returning pub items, starting with the bus entry points - [packages/d2b-bus/src/router.rs:3709, packages/d2b-bus/src/router.rs:3261, packages/d2b-bus/src/registry.rs:366]
  evidence: seeds `^\s*pub (fn|struct|enum|trait|const|type)` = 158, `/// # (Examples|Errors|Panics|Safety)` = 0, `-> Result<` = 94

## perf
- d2b-bus-p1#11 sev=low blast=leaf effort=S verdict=actionable - The WatchSink delivery path copies every watch frame payload with frame.payload().to_vec() before send_and_wait_ack, allocating a fresh Vec per frame on the watch-delivery path (the recorded kept-half credit path, B1, docs/explanation/over-engineering-audit-record.md:463) - fix: pass the payload slice through OutgoingStream::send_and_wait_ack (streams.rs:661) or clone once at the bridge so per-frame allocation is avoided - [packages/d2b-bus/src/router.rs:4228-4235]
  evidence: `format!\(` = 13 (12 doc/test, 1 cold bootstrap path at router.rs:2221), `Vec::new\(\)|VecDeque::new\(\)|HashMap::new\(\)|BTreeMap::new\(\)` = 87 (mostly tests), `\.to_string\(\)` = 5 (test helpers); static (unmeasured), no benchmark exists

## conc
- d2b-bus-p1#12 sev=low blast=leaf effort=S verdict=actionable - RouteLeaseState wraps a single bool in Mutex<bool>, paying a lock for one flag that an atomic would serve - fix: replace revoked: Mutex<bool> with AtomicBool and use Acquire/Release in with_active and remove - [packages/d2b-bus/src/registry.rs:522-523, packages/d2b-bus/src/registry.rs:573-582]
  evidence: seeds `std::thread::|thread::spawn|thread::scope` = 1 (test-only thread::scope at router.rs:4791), `\bMutex<|\bRwLock<` = 25 (all std Mutex with into_inner poisoning recovery, brief critical sections, none across await), `Atomic\w+|Ordering::` = 49 (ManualClock AcqRel/Acquire pair, ComponentActivity Release/Acquire valid flags, test counters), `thread_local!|unsafe impl (Send|Sync) for` = 0

## async
- clean: seeds `async fn|async move|\.await` = 203 (198 router + 1 authorization + 4 registry), `tokio::spawn|spawn_blocking|JoinSet|select!\(|join!\(` = 11 (1 production tokio::spawn at router.rs:2426 for the response dispatcher, 10 test join! sites), `tokio::sync::(Mutex|RwLock|Notify)` = 2 (Notify in the cfg(test) hook struct at 837-838 plus one test import), `#\[tokio::(main|test)\]|Runtime::block_on` = 24; checked: no std lock held across await (all std Mutex critical sections are brief with sanctioned disallowed-method allows and comments), AsyncMutex only for session and inbound receivers, select! biased with cancellation first, lease Drop aborts on cancellation and deadline paths

## unsafe
- N/A: seeds `\bunsafe \{|\bunsafe fn|\bunsafe impl|\bunsafe extern` = 0, `// SAFETY:` = 0, `transmute|from_raw|MaybeUninit|mem::zeroed` = 0; crate lints table forbids unsafe_code (packages/d2b-bus/Cargo.toml:9), so the lens is not applicable

## ffi
- N/A: seeds `extern "C"|no_mangle|unsafe\(link_section` = 0, `catch_unwind` = 0, `repr\(C\)|repr\(transparent\)` = 0, `CStr|CString|c_char` = 0; the partition crosses no foreign boundary

## macro
- clean: seeds `macro_rules!` = 1 (router.rs:2162 mutate_component_session_admission_trait!, a cfg-gated compile-assertion harness with narrow ident fragment specifiers and unreachable! bodies, deliberate), `proc_macro|syn::|quote!` = 0, `\$crate` = 0, `to_compile_error|new_spanned` = 0

## test
- d2b-bus-p1#13 sev=high blast=leaf effort=S verdict=actionable - emitter_records_only_closed_bus_labels exercises every BusTelemetry method but asserts nothing, and every emit outcome is swallowed by `let _ = self.emit(...)` inside BusMetrics, so a label drifting out of the closed set passes silently - fix: make the test assert something observable, for example return EmitOutcome from a test-visible emit path or expose a read-back of the BoundedEmitter queue in d2b-telemetry, and assert Ok per call (route review-pass) - [packages/d2b-bus/src/metrics.rs:612-633, packages/d2b-bus/src/metrics.rs:451-534]
  evidence: seeds `#\[test\]|#\[tokio::test\]` = 62 (40 router + 19 authorization + 3 metrics), `assert_eq!\(|assert_ne!\(|assert!\(` = heavy across the suite, `proptest!|insta::assert|rstest` = 0, `#\[ignore\]` = 0; the named test contains zero assertion calls and no panic path, so it cannot fail on the property it names; the remaining 61 tests assert observable behavior (delivery counts, error variants, revocation races with Notify hooks, start_paused timeouts, redaction of Debug output); the tests/ui compile-fail fixtures (4 files) belong to the session_seam_tests macro surface in part 2

## Coverage
- idiom: 1 finding
- own: 1 finding
- type: 2 findings
- api: 2 findings
- err: clean (seeds ran: 459/31/8/5; production unwrap/expect sites are fixed-ref expects and named-invariant expects per card false positives)
- serde: clean (seeds ran: 0/0/0/2; the 2 s4 hits are frame construction following the d2b-telemetry pattern and a test helper)
- obs: N/A (seeds: 0/0/0/0 real; no tracing/log dependency in the crate manifest)
- docs: 4 findings
- perf: 1 finding
- conc: 1 finding
- async: clean (seeds ran: 203/11/2/24; no lock held across await, biased select with cancellation first, lease Drop aborts)
- unsafe: N/A (seeds: 0/0/0; unsafe_code = "forbid" in the crate lints table)
- ffi: N/A (seeds: 0/0/0/0; no foreign boundary in the partition)
- macro: clean (seeds ran: 1/0/0/0; single cfg-gated compile-assertion harness macro)
- test: 1 finding