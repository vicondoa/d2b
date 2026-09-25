# d2b-provider-display-wayland-p2 - d2b-provider-display-wayland - part 2/2
Baseline: 6ebdd4cec | LOC audited: 6464 (excl. src/generated/**) | modules: controller, runtime, process, bin (d2b-wayland-proxy), spec, policy, session_children, principal, lib
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: part 2/2 per U1 (f): src/controller.rs, src/runtime.rs, src/process.rs, src/bin/**, src/spec.rs, src/policy.rs, src/session_children.rs, src/principal.rs, src/lib.rs

## idiom
- d2b-provider-display-wayland-p2#1 sev=low blast=leaf effort=S verdict=actionable - three stale `#[allow(dead_code)]` markers sit on constructors that production code calls: `FinalizationInput::from_supervisor` (controller.rs:464), `LaunchGrants::from_supervisor_for_session_with_frontend_and_controller` (process.rs:402), `ProcessObservation::from_supervisor` (process.rs:676) - fix: delete the three allows (keep process.rs:380, whose constructor is test/test-support-only) so a future real dead-code warning is not masked - [src/controller.rs:464, src/process.rs:402, src/process.rs:676]
  evidence: idiom seeds: 0 index loops / 3 hand-written impls / 2 statement accumulations; allows judged stale by reading call sites: runtime.rs:247,748,811,858 and process.rs:349 all reach production paths
- d2b-provider-display-wayland-p2#2 sev=medium blast=leaf effort=S verdict=actionable - the session binding digest is derived twice with byte-identical bodies: free fn `session_digest` (controller.rs:1442) duplicates `WaylandSessionSpec::session_digest` (spec.rs:385), so the two can silently diverge - fix: make the controller free fn delegate to `spec.session_digest(controller_generation)` and keep spec.rs:385 as the canonical home (census: d2bd already consumes the method at interaction_composition.rs:4080,4267) - [src/controller.rs:1442, src/spec.rs:385]
  evidence: session_digest census over packages/ = 2 definitions with identical bodies (controller.rs:1442, spec.rs:385); both hash guest/host/user refs, reconnect generation, controller generation with [0] separators
- clean: index-loop seed 0 hits; the 3 hand-written Default impls (controller.rs:221, process.rs:154, process.rs:641) are invariant-preserving or test-support, not derive candidates; the 2 `Vec::new()` accumulations (process.rs:219, policy.rs:272) are conditional-push loops where an iterator would obscure

## own
- clean: seeds ran 66 clone / 37 to_owned-family / 0 Rc-RefCell-Arc-Mutex / 0 Cow; every clone inspected is explainable (multi-pass reconcile clones at runtime.rs:397,451,599,654; lease principal copies at controller.rs:1085,1112; set-ownership clones in policy.rs:281-354; durable-name clones in session_children.rs:65-143; CLI startup clones in bin); no Rc/RefCell in library code, only in the single-threaded bin accept loop where shared mutable handler state genuinely has multiple owners

## type
- clean: seeds ran 4 validate/check fns / 1 bool flag / 0 stringly state; the validate fns are parse-once boundary checks inside constructors (validate_label/validate_color in DisplayIdentity::new, validate_bounds in FilterInput::new, validate_layer in WaylandPolicy::compile) exactly per the skill; the one bool (DisplayRunnerContract.watched_configuration_is_dependency, controller.rs:30) is a contract flag; wire-mirroring booleans (cross_domain_trusted, virgl_video, debug_logging, border_enabled) are schema-pinned and not flagged

## api
- d2b-provider-display-wayland-p2#3 sev=medium blast=leaf effort=S verdict=actionable - `PrincipalReleaseReceipt` (controller.rs:702, re-exported at lib.rs:20) is unconstructible: private `session_key` field and no constructor, so `DisplayController::release_session_principal` (controller.rs:1379) can never be called by the daemon; the principal-release path is dead exported surface - fix: add a constructor and wire the daemon cleanup path to call release_session_principal, or make both pub(crate) until the path is wired - [src/controller.rs:702, src/controller.rs:1379, src/lib.rs:20]
  evidence: census: PrincipalReleaseReceipt|release_session_principal over packages/; nixos-modules/; tests/; docs/reference/; labs = 4 hits, all in-crate (controller.rs:702,706,1381; lib.rs:20); release_session_principal has zero callers
- d2b-provider-display-wayland-p2#4 sev=low blast=leaf effort=S verdict=actionable - `WaylandPolicySnapshot::from_authenticated_session` (controller.rs:580) has no callers anywhere; the daemon resolves snapshots via `from_authenticated_route` - fix: delete the wrapper or mark it deliberate with a comment naming the route-based entry as canonical - [src/controller.rs:580]
  evidence: census: from_authenticated_session over packages/ = 1 hit (the definition); daemon uses from_authenticated_route (d2bd interaction_composition.rs:2279)
- clean: api seed 2 (Arc/Rc/Box/RefCell in pub signatures) = 0 hits; seed 3 re-export arms in lib.rs are the house single-surface pattern; pub surface is otherwise deliberate (opaque grant/lease types with redacted Debug, pub(crate) fields on ProcessObservation and WorkerRestartEvidence, test-support-gated constructors)

## err
- d2b-provider-display-wayland-p2#5 sev=medium blast=leaf effort=S verdict=actionable - `DisplayController::new(pool_size)` panics via `PrincipalPool::new(pool_size).expect(...)` (controller.rs:740-741) on any caller-supplied pool size outside 1..=32; the pub library API should not panic on input-derived values - fix: return `Result<Self, PrincipalPoolError>` from `DisplayController::new` (or document `# Panics` naming the bound) and update the two daemon call sites - [src/controller.rs:740, src/controller.rs:741]
  evidence: err seed 1 = 75 hits, 3 outside tests (controller.rs:741,746,1048); the 1048 expect is justified (grants checked non-None two branches earlier); current DisplayController::new callers pass constants (d2bd interaction_composition.rs:2294,7164), so no live trigger
- d2b-provider-display-wayland-p2#6 sev=low blast=leaf effort=S verdict=actionable - `WaylandSpecError::NoPrincipalAvailable` (spec.rs:30) is never constructed: pool exhaustion is mapped to a Failed status with `SessionCondition::NoPrincipalAvailable` instead of the error variant - fix: either construct the variant in the exhaustion path (controller.rs:1089) or delete it and its Display arm - [src/spec.rs:30, src/spec.rs:43]
  evidence: census: WaylandSpecError::NoPrincipalAvailable over packages/ = 2 hits (spec.rs:30,43); controller.rs:1089-1101 returns a Failed status rather than the error
- d2b-provider-display-wayland-p2#7 sev=medium blast=leaf effort=M verdict=actionable - grant and ticket constructors return `Result<_, &'static str>` error codes (`issue_for_supervisor_with_controller_generation` process.rs:335-346, `new_for_role_with_controller_generation` process.rs:825-887), so callers cannot match the failure and the codes are untyped strings - fix: introduce a closed `LaunchError` enum (thiserror) with `SessionInvalid` and `TicketInvalid` variants and return it from both constructors; the daemon caller maps to WorkerEffectError today (d2bd interaction_composition.rs:4283) - [src/process.rs:335, src/process.rs:346, src/process.rs:825, src/process.rs:886]
  evidence: err seed 4 = 6 error enums in lane scope, all closed and well-split; the two &'static str returns are the only untyped error paths
- clean: err seeds 75 unwrap/expect (72 in tests) / 8 let _ (7 best-effort readiness reports before exit in bin, 1 test artifact) / 0 panic-unreachable-todo / 6 error enums; error taxonomy is otherwise exemplary (WorkerEffectError split by caller action, DisplayRuntimeError forwards effect codes, PolicyCompileError carries the offending interface)

## serde
- d2b-provider-display-wayland-p2#8 sev=low blast=leaf effort=S verdict=actionable - `#[serde(try_from = "WaylandSessionSpecWire")]` (spec.rs:233) is inert: WaylandSessionSpec derives only Serialize, and Deserialize is hand-written (spec.rs:263-270) to do exactly what the derive plus try_from would generate - fix: delete the inert attribute (keep rename_all for the Serialize side), or derive Deserialize with try_from and delete the manual impl; pick one mechanism - [src/spec.rs:230, src/spec.rs:233, src/spec.rs:263]
  evidence: serde seeds 10 derives / 12 serde attrs / 3 hand-written Deserialize impls (spec.rs:105,263; policy.rs:194); the other two manual impls are live admission gates (recorded refusal, not re-flagged); deny_unknown_fields is enforced through the Wire structs so the attribute block adds nothing
- clean: wire admission gates (DisplayIdentityWire, WaylandSessionSpecWire, FilterInputWire) all deny_unknown_fields and validate through TryFrom; CompiledWaylandPolicy round-trips with private fields; DisplayProcessRole and DisplayLabelPosition use kebab-case per house convention

## obs
- clean: seeds ran 17 println/eprintln (all in bin, CLI product output carved out by the card) / 51 interpolated log macros (all in bin, same carve-out) / 0 instrument / 44 tracing uses; library tracing is exemplary: named fields (zone, guest, session, error) on every event, lazy format! closures in the bin's DiagRateLimiter, no secret material in any field, error chains logged once at the boundary that handles them

## docs
- d2b-provider-display-wayland-p2#9 sev=low blast=leaf effort=S verdict=actionable - `FilterInput::allow_globals` and `FilterInput::deny_globals` doc comments say "Add an allowed global to this layer" / "Add a denied global to this layer" but the methods are getters returning `&[String]` - fix: reword to "Borrow the allowed globals of this layer" / "Borrow the denied globals of this layer" - [src/policy.rs:114, src/policy.rs:119]
  evidence: docs seed 1 = 226 pub items, all read in full; policy.rs:114-122 doc text contradicts the getter shape (copy-paste from the builder intent)
- d2b-provider-display-wayland-p2#10 sev=low blast=leaf effort=M verdict=actionable - Result-returning pub API has no `# Errors` canonical sections despite `#![deny(missing_docs)]`: constructors and reconcilers document their failure modes only through the error-enum variant docs - fix: add `# Errors` sections naming the variants on the pub Result items, e.g. `DisplayIdentity::new`, `WaylandSessionSpec::new`, `WaylandPolicy::compile`, `DisplayController::reconcile_authenticated_session` - [src/spec.rs:117, src/spec.rs:286, src/policy.rs:247, src/controller.rs:759]
  evidence: docs seed 2 (canonical sections) = 0 hits; seed 3 = 81 `-> Result<` items; first sentences are otherwise strong and every pub item is documented
- clean: docs seed 1 = 226 pub items (all read), seed 2 = 0, seed 3 = 81; doc quality is high (one-line first sentences, module docs on every module, redacted Debug documented as deliberate); the two findings above are the only shape gaps

## perf
- d2b-provider-display-wayland-p2#11 sev=low blast=leaf effort=S verdict=actionable - `durable_display_suffix` (session_children.rs:316-318) builds a 40-char hex suffix with one `format!` allocation per byte (20 allocations) instead of writing into the already-preallocated `String::with_capacity(40)` - fix: push two hex digits per byte via a lookup table or a single hex write into `suffix`, keeping the preallocation - [src/session_children.rs:316, src/session_children.rs:317]
  evidence: perf seed 1 = 16 format! sites; this is the only format! in a loop; cold durable-naming path, static (unmeasured)
- clean: perf seeds 16 format! (rest are cold error/diagnostic paths or single-shot renders) / 20 collection news (empty-case-common or bounded) / 37 to_string (wire rendering and CLI); no hot-path allocation or collection-choice issue found

## conc
- N/A: seeds 0/0/0/0 all zero; no threads, locks, atomics, or thread_local in part 2 (Rc/RefCell in the bin is single-threaded shared state, not a concurrency model)

## async
- N/A: seeds 0/0/0/0 all zero; no async fn, spawn, tokio sync, or runtime entry in part 2; the crate is a synchronous reconciler plus a poll-loop binary

## unsafe
- N/A: seeds 1-3 zero real hits (the 4 `from_raw` matches are the safe std `io::Error::from_raw_os_error` in bin tests); lib.rs:4 `#![forbid(unsafe_code)]` alone does not make the lens applicable

## ffi
- N/A: seeds 0/0/0/0 all zero; no extern, no_mangle, repr(C), or CStr in part 2

## macro
- N/A: seeds 0/0/0/0 all zero; no macro_rules!, proc macro, or $crate in part 2

## test
- d2b-provider-display-wayland-p2#12 sev=medium blast=leaf effort=S verdict=actionable - the principal-release contract has no test and the test named `core_policy_snapshot_and_principal_receipt_are_consumed_by_controller` (controller.rs:1481) never touches `PrincipalReleaseReceipt` or `release_session_principal`; the name overclaims and the release path (acquire, release, re-acquire, UnknownLease on foreign receipt) is unverified - fix: rename the test to what it asserts (snapshot consumption) and add a release-path test exercising `release_session_principal` with a constructed receipt, asserting pool re-acquisition and UnknownLease for a foreign receipt - [src/controller.rs:1481, src/controller.rs:1379]
  evidence: test seed 1 = 48 #[test] in lane scope (6 controller, 3 runtime, 6 process, 16 bin, 1 src/policy.rs, 1 session_children, 13 provider_behavior, 1 tests/policy.rs, 1 lifecycle); controller.rs:1481-1505 body reconciles and asserts Phase::Ready only
- clean: test seeds 48 #[test] / ~120 asserts / 0 proptest-insta-rstest / 0 #[ignore]; tests assert behavior (phase transitions, error variants via matches!, cleanup order, wire validation reuse, digest fencing), expectations are human-written, and the bin tests cover accept-error classification and poll-timeout bounds; no test that cannot fail found

## Coverage
- idiom: 2 finding(s)
- own: clean (seeds ran: 66/37/0/0)
- type: clean (seeds ran: 4/1/0)
- api: 2 finding(s)
- err: 3 finding(s)
- serde: 1 finding(s)
- obs: clean (seeds ran: 17/51/0/44)
- docs: 2 finding(s)
- perf: 1 finding(s)
- conc: N/A (seeds: 0/0/0/0 all zero; no threads/locks/atomics in part 2)
- async: N/A (seeds: 0/0/0/0 all zero; no async code in part 2)
- unsafe: N/A (seeds: 0/0/0 real; only lib.rs:4 forbid(unsafe_code); from_raw_os_error is a safe std fn)
- ffi: N/A (seeds: 0/0/0/0 all zero)
- macro: N/A (seeds: 0/0/0/0 all zero)
- test: 1 finding(s)