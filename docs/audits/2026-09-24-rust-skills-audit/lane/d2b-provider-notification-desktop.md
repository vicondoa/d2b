# d2b-provider-notification-desktop - d2b-provider-notification-desktop
Baseline: 6ebdd4cec | LOC audited: 5712 (excl. src/generated/**; none present) | modules: whole crate (17 src files, 5 tests files)
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: none (single-part lane)

## idiom
- d2b-provider-notification-desktop#1 sev=low blast=leaf effort=S verdict=actionable - `expected_acknowledgements` accumulates its two source acknowledgement arms with `Vec::new()` + `extend(iterator)` where the chain could collect the Vec directly - fix: `let mut acknowledgements: Vec<_> = plan.start_endpoints.iter().map)...).chain(plan.stop_endpoints.iter().map)...)).collect();` then keep the two conditional `HostSink` pushes - [packages/d2b-provider-notification-desktop/src/controller.rs:660-675]
  evidence: seed 3 (`let mut \w+ = (String|Vec)::new\(\)`) = 4 hits; this site is the actionable one (lifecycle.rs:403-405 accumulations track side-effecting plan application and are not collect-able)
- d2b-provider-notification-desktop#2 sev=low blast=leaf effort=S verdict=actionable - `NotificationProviderDescriptor::service_package()` hardcodes the wire literal `"d2b.notification.v3"` duplicating the exported `SERVICE_PACKAGE` const - fix: return `crate::SERVICE_PACKAGE` so the literal has one home - [packages/d2b-provider-notification-desktop/src/descriptor.rs:43-44, packages/d2b-provider-notification-desktop/src/lib.rs:70]
  evidence: seed  2 (`impl (Default|From|PartialEq|Eq|Debug|Clone|Hash) for`) = 1 hit (descriptor.rs:32 manual `Default` preserving the `schema_version=1` invariant - false positive); static read: literal duplicated from the exported const

## own
- d2b-provider-notification-desktop#3 sev=low blast=leaf effort=S verdict=actionable - `commit_reconciliation` takes `SourceReconcileResult` by value but only reads its fields, forcing `.clone()` at both call sites - fix: take `result: &SourceReconcileResult` and drop the two `.clone()` calls - [packages/d2b-provider-notification-desktop/src/controller.rs:1033, packages/d2b-provider-notification-desktop/src/controller.rs:1058, packages/d2b-provider-notification-desktop/src/controller.rs:1297-1318]
  evidence: seed  1 (`\.clone\(\)`) = 91 hits; these two are avoidable because `commit_reconciliation` reads only `result.stop/start_endpoints/start_host_sink/stop_host_sink/host_sink_fingerprint`
- d2b-provider-notification-desktop#4 sev=low blast=leaf effort=S verdict=actionable - `NotificationLifecycleSupervisor` wraps its owned backend in `Arc<B>`, counting one reference that nothing else shares - fix: store `backend: B` directly (drop `Arc`) while keeping the `Send + Sync` bounds - [packages/d2b-provider-notification-desktop/src/lifecycle.rs:338, packages/d2b-provider-notification-desktop/src/lifecycle.rs:346]
  evidence: seed  3-4 (`Rc<|RefCell<|Arc<Mutex<|Arc<RwLock<|Cow<`) = 0; static read: `new(backend: B)` wraps a single owner and the supervisor derives no `Clone`; census `NotificationLifecycleSupervisor` over packages/; nixos-modules/; tests/; docs/reference/; labs/ = 8 hits, all construct-by-value with no shared ownership

## type
- d2b-provider-notification-desktop#5 sev=low blast=leaf effort=S verdict=policy-confirmed - `NotificationRunnerContract` carries two boolean flags (`watched_configuration_is_dependency`, `component_session_only`) where a cutover enum could make the states explicit - fix: after a policy/ADR change, fold them into a `CutoverState` enum; currently kept per the refusal ledger - [packages/d2b-provider-notification-desktop/src/controller.rs:22-27, docs/explanation/over-engineering-audit-record.md:361]
  evidence: seed  2 (`is_\w+: bool|\w+_flag: bool`) = 1 hit (controller.rs:25); policy: `NotificationRunnerContract` refused in the over-engineering record row 361 (daemon composition test calls it)

## api
- d2b-provider-notification-desktop#6 sev=medium blast=leaf effort=S verdict=actionable - The non-effects controller surface has no production callers: `reconcile_authenticated_display` is uncalled at all, `reconcile_sources` and `drain_sources` serve only in-crate tests - fix: delete `reconcile_authenticated_display` or route its tests through the `_with_effects` twin; make `reconcile_sources`/`drain_sources` `pub(crate)` unless an external caller is planned - [packages/d2b-provider-notification-desktop/src/controller.rs:1019, packages/d2b-provider-notification-desktop/src/controller.rs:1329, packages/d2b-provider-notification-desktop/src/controller.rs:1411]
  evidence: census `reconcile_sources\(` over packages/; nixos-modules/; tests/; docs/reference/; labs/ = 11 hits (1 def + 10 in-crate test calls); `reconcile_authenticated_display\b` = 1 hit (def only); `drain_sources\(` = 4 hits (1 def + 3 crate test calls)
- d2b-provider-notification-desktop#7 sev=low blast=leaf effort=S verdict=actionable - `#[allow(dead_code)]` sits on `from_route`, a function reachable from production via `from_authenticated_route` - fix: delete the stale allow - [packages/d2b-provider-notification-desktop/src/controller.rs:110, packages/d2b-provider-notification-desktop/src/controller.rs:159-160]
  evidence: static read: `from_route` called at controller.rs:110; that chain reaches production through `reconcile_authenticated_display_with_effects` at controller.rs:1368 (used by `NotificationRuntime::drain`/`finalize`)
- d2b-provider-notification-desktop#8 sev=low blast=leaf effort=S verdict=actionable - `stream_admission.rs` is a private three-line re-export shim: `lib.rs` could re-export admission items directly - fix: delete `stream_admission.rs` and change `lib.rs:59` to `pub use admission::{AdmissionError, AdmissionPurpose, SessionEvidence, TransportClass};` - [packages/d2b-provider-notification-desktop/src/stream_admission.rs:1-3, packages/d2b-provider-notification-desktop/src/lib.rs:29, packages/d2b-provider-notification-desktop/src/lib.rs:59]
  evidence: seed  3 (`^\s*pub use `) = 13 hits (12 lib.rs re-export arms + 1 shim arm); the shim module is private (lib.rs:29), so its "canonical source path" doc names a path no external caller can import

## err
- d2b-provider-notification-desktop#9 sev=medium blast=family effort=L verdict=actionable - Fifty `Result<_, &'static str>` sites (controller, lifecycle, guest_source, runtime) form a stringly error family forcing callers to string-match, while sibling enums (AdmissionError, NotificationError, SinkError)_ are typed - fix: introduce one crate error enum (suggest `NotificationLifecycleError`) for the lifecycle/controller/config family and replace the str returns on pub fns and both effect-port traits; update d2bd's `InteractionNotificationLifecycleBackend` impl - [packages/d2b-provider-notification-desktop/src/guest_source.rs:18-21, packages/d2b-provider-notification-desktop/src/lifecycle.rs:291-297, packages/d2b-provider-notification-desktop/src/controller.rs:369, packages/d2b-provider-notification-desktop/src/controller.rs:930]
  evidence: seed  4 (`enum \w*Error`) = 7; grep `Result<[^>]*, &'static str>` over src/*.rs = 50 hits; tests match on the strings (guest_source.rs:100-115), so callers string-match
- d2b-provider-notification-desktop#10 sev=medium blast=leaf effort=S verdict=actionable - Delivery rejection paths collapse every admission/session/category failure into `NotificationError::InvalidOpaqueKey`, misreporting "notification-opaque-key-invalid" for unauthenticated, cross-zone,and category-denied cases - fix: add an `NotificationError::Denied` (or `SessionDenied`) variant and map the five admission/zone/category rejection sites to it; keep `InvalidOpaqueKey` for key-bound violations - [packages/d2b-provider-notification-desktop/src/host_sink.rs:185, packages/d2b-provider-notification-desktop/src/host_sink.rs:195, packages/d2b-provider-notification-desktop/src/host_sink.rs:202, packages/d2b-provider-notification-desktop/src/host_sink.rs:308, packages/d2b-provider-notification-desktop/src/runtime.rs:103]
  evidence: seed  2 (`let _ = |\.ok\(\);`) = 0; static read: `InvalidOpaqueKey` used as catch-all at 9 sites (host_sink.rs:185-308, runtime.rs:103-129); slug not pinned by docs/reference (grep "notification-" = 0 hits)

## serde
- clean: seeds ran:  6/13/2/0 - derive(Serialize/Deserialize)=6; serde attrs=13; hand-written Deserialize=2 (live admission gates for the wire twins at types.rs:232/307, sanctioned per record rows 143/151); serde_json=0 in src. Wire shapes land through deny_unknown_fields gates + TryFrom validation - clean

## obs
- clean: seeds ran:  0/24/0/4 - println!/eprintln!=0; tracing event macros=24, all with named fields (provider, zone, reason, action) + static messages; interpolated-message-with-no-fields events=0; instrument/spans=0 (sync provider path, no async context to carry); `use tracing` lines=4. All events carry the provider/zone/reason context as fields - clean

## docs
- d2b-provider-notification-desktop#11 sev=medium blast=leaf effort=M verdict=actionable - No `# Errors` section exists on any Result-returning pub item (112 `-> Result<` sites) even though the crate pins wire-leaning error enums - fix: add `# Errors` sections naming the exact variants (or stable slugs) on pub fns like `ActionNonceStore::register`, `NotificationRuntime::new`, `NotificationSink::deliver_from_guest_source` - [packages/d2b-provider-notification-desktop/src/action_nonce.rs:84-89, packages/d2b-provider-notification-desktop/src/runtime.rs:64-67, packages/d2b-provider-notification-desktop/src/host_sink.rs:297-305]
  evidence: seed  2 (`/// # (Examples|Errors|Panics|Safety)`) = 0 across 229 pub items (seed 1 = 229, seed 3 `-> Result<` = 112)
- d2b-provider-notification-desktop#12 sev=low blast=leaf effort=S verdict=actionable - Doc first sentences are broken fragments: "/// the daemon." opens `deliver_evidence`,"/// completes every effect immediately." opens `RecordingEffects`,"/// the current authenticated reconnect generation." runs into the `from_config_at_generation` doc - fix: rewrite each as a standalone 15-word summary before the trailing paragraph - [packages/d2b-provider-notification-desktop/src/runtime.rs:88-89, packages/d2b-provider-notification-desktop/src/test_support.rs:14, packages/d2b-provider-notification-desktop/src/guest_source.rs:17]
  evidence: static read: first-sentence shape at the three anchors ("the daemon."/"completes every effect immediately."/"the current authenticated...")

## perf
- d2b-provider-notification-desktop#13 sev=low blast=leaf effort=S verdict=actionable - `NotificationSink::deliver` formats "notification-{id}" once (request_id) but re-formats the same string three more times into projection map keys; `close` re-formats from u32 while callers already hold the request_id string - fix: reuse `request_id` (clone it into map keys where needed)and add an internal `close_by_request_id(&str)` to kill the u32-to-String-to-u32 round-trip in `close_session`/`gc_projections` - [packages/d2b-provider-notification-desktop/src/host_sink.rs:265, packages/d2b-provider-notification-desktop/src/host_sink.rs:276-291, packages/d2b-provider-notification-desktop/src/host_sink.rs:376, packages/d2b-provider-notification-desktop/src/host_sink.rs:484-493]
  evidence: seed  1 (`format!\(`) = 16 hits; four-plus of them re-format a string the caller already owns (host_sink.rs:265 vs 276/278/288/291; close at 376 vs callers holding request_id); static (unmeasured)

## conc
- clean: seeds ran:  0/1/0/0 - std::thread/spawn/scope=0; Mutex/RwLock=1 (lifecycle.rs:339 `state: Mutex<LifecycleState>` on sanitary synchronous path, guarded by tracked allows at lifecycle.rs:355/394/538 with reason "synchronous path"); Atomics/Ordering=0; thread_local!/unsafe impl Send/Sync=0 - clean

## async
- N/A (seeds:  0/0/0/0 all zero; the crate declares no async fn, await, spawn, or tokio runtime usage)

## unsafe
- N/A (seeds:  0/0/0/1; seeds 1-3 all zero; the lone seed 4 hit is `#![forbid(unsafe_code)]` at lib.rs:4, which per the lens card does not make the lens applicable)

## ffi
- N/A (seeds:  0/0/0/0 all zero; no extern "C", no_mangle, repr(C/transparent), CStr/CString, or catch_unwind in src)

## macro
- N/A (seeds:  0/0/0/0 all zero; no macro_rules!, proc-macro, or syn/quote usage)

## test
- clean: seeds ran:  41/131/0/0 - #[test]=41 (25 src + 16 tests); asserts=131 (86 src + 45 tests); proptest!/insta/rstest=0; #[ignore]=0; tests are behavioral (wire defaults, redaction canary, receipt matching, partial-effect rollback, nonce single-use/bounds, closed telemetry labels)and deterministic (injected now_secs, no network or clock reads)- clean

## Coverage
- idiom:  2 finding(s)
- own:  2 finding(s)
- type:  1 finding(s)
- api:  3 finding(s)
- err:  2 finding(s)
- serde: clean (seeds ran:  6/13/2/0)
- obs: clean (seeds ran:  0/24/0/4)
- docs:  2 finding(s)
- perf:  1 finding(s)
- conc: clean (seeds ran:  0/1/0/0)
- async: N/A (seeds:  0/0/0/0 all zero; no async fn/await/spawn/runtime)
- unsafe: N/A (seeds:  0/0/0/1; seeds 1-3 zero; seed 4 alone is the forbid attribute)
- ffi: N/A (seeds:  0/0/0/0 all zero)
- macro: N/A (seeds:  0/0/0/0 all zero)
- test: clean (seeds ran:  41/131/0/0)