# d2bd-p2 - d2bd - part 2/8
Baseline: 6ebdd4cec | LOC audited: 10672 (excl. src/generated/**) | modules: composition.rs (10071-20142), audio_host_controller.rs
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: composition.rs:10071-20142 + audio_host_controller.rs (U1 section f)

## idiom
- d2bd-p2#1 sev=low blast=leaf effort=S verdict=actionable - open_resource_plane hardcodes the provider-identity seed window as `for attempt in 0..30` and `Duration::from_secs(2)` although the same-file consts PROVIDER_IDENTITY_SEED_ATTEMPTS (30) and PROVIDER_IDENTITY_SEED_INTERVAL (2s) at composition.rs:14193-14194 document exactly this "30 x 2s" window - fix: use `PROVIDER_IDENTITY_SEED_ATTEMPTS` and `PROVIDER_IDENTITY_SEED_INTERVAL` in the retry loop so the literals cannot drift from the documented window - [packages/d2bd/src/composition.rs:14699, packages/d2bd/src/composition.rs:14710, packages/d2bd/src/composition.rs:14193]
  evidence: seed `for \w+ in 0\.\.` = 2 hits (14227 parameterized retry is fine; 14699 is the literal); consts at 14193-14194 define the same window
- clean: seeds ran 2/0/4; the other index loop (14227) is a parameterized retry, the `let mut Vec::new()` sites (14967, 19761) are plain loops with early exits and side effects where the idiom skill itself prefers the loop, and no hand-written derive-class impls exist in the range

## own
- clean: seeds ran 189/286/0/0 (sampled: 50 of 475 hits, every 10th); every sampled clone is explainable - `caller_role.clone()` into spawned owner threads, `Arc<ServerState>` clones at spawn/effect boundaries, `.to_owned()` on wire error codes and JSON payload strings, test-fixture clones; no Rc/RefCell/Arc<Mutex>/Cow in the range

## type
- d2bd-p2#2 sev=low blast=leaf effort=S verdict=actionable - ShutdownDegradedMarker stores `outcome: String` and `severity: String` although the same file defines VmShutdownOutcome (composition.rs:16131) whose label()/degraded_severity() (16205-16239) are the only producers of those strings - fix: derive Serialize on VmShutdownOutcome with `#[serde(rename_all = "snake_case")]` and store the enum in the marker so the report shape cannot drift from the enum - [packages/d2bd/src/composition.rs:16152, packages/d2bd/src/composition.rs:16155, packages/d2bd/src/composition.rs:16131]
  evidence: type seeds 0/0/0 (model reading); the marker strings are produced from the enum at 16233-16239 and 16364
- d2bd-p2#3 sev=medium blast=leaf effort=S verdict=actionable - `force` on guest lifecycle requests is parsed from the wire (composition.rs:6794-6797), stored in DaemonGuestLifecycleEffect.force (6837-6848), and never consulted - apply() only reads it via `let _ = self.force;` (18659) - so `d2b guest ... --force` (sent by d2b/src/guest.rs:283) is silently ignored - fix: implement the force semantics in apply() (e.g. skip the graceful wait) or drop the field and the wire parse - [packages/d2bd/src/composition.rs:18659, packages/d2bd/src/composition.rs:18608]
  evidence: type seeds 0/0/0; census: `DaemonGuestLifecycleEffect|self.force` over packages/ = struct def 18603, construction 6837, single read 18659; CLI sends the flag (d2b/src/guest.rs:283)
- clean: seeds ran 0/0/0; the enums in the range (GuestComponentSessionCacheMode, VmRunnerLaunch, VmShutdownOutcome, HostActivationMarkerState) are well-formed, and no boolean-flag soup or stringly-typed state beyond the two findings exists

## api
- d2bd-p2#4 sev=low blast=leaf effort=S verdict=actionable - the pub surface of audio_host_controller.rs (trait HostAudioController 59, PipeWireHostController 92, from_audio_node 106, find_audio_node 124, QemuAudioController 214) is unreachable outside the crate because `mod audio_host_controller;` (composition.rs:395) is private - fix: reduce these to `pub(crate)` (FakeHostController is already cfg(test)) so the visibility says what the surface is - [packages/d2bd/src/audio_host_controller.rs:59, packages/d2bd/src/audio_host_controller.rs:92, packages/d2bd/src/composition.rs:395]
  evidence: api seeds 9/0/1; census: `HostAudioController|find_audio_node|enforce_grant|enforce_level` over packages/, nixos-modules/, tests/, docs/reference/, labs/ = 0 hits outside packages/d2bd
- d2bd-p2#5 sev=low blast=leaf effort=S verdict=actionable - `vm_name: &str` in HostAudioController::enforce_grant/enforce_level is dead trait surface: every implementation (PipeWire, Qemu, Fake) names it `_vm_name` and ignores it, and the only callers (audio_dispatch.rs:160,177) pass it pointlessly - fix: remove the parameter from both trait methods and the call sites - [packages/d2bd/src/audio_host_controller.rs:68, packages/d2bd/src/audio_host_controller.rs:78, packages/d2bd/src/audio_host_controller.rs:173]
  evidence: api seeds 9/0/1; census: `enforce_grant|enforce_level` over packages/ = call sites audio_dispatch.rs:160,177 only; all three impls ignore the parameter (173, 183, 219, 230, 287, 297)
- clean: no Arc/Rc/Box/RefCell in any public signature, the re-export `pub use crate::audio_dispatch::HostEnforcementResult` follows the house single-surface pattern, and the trait is dyn-safe as its docs claim

## err
- d2bd-p2#6 sev=low blast=family effort=M verdict=actionable - `detail: err.to_string()` collapses the source error into a String when building TypedError variants, losing the error chain for diagnostics - fix: carry the source in the variant (e.g. `InternalBrokerUnavailable { path, #[source] source: serde_json::Error }` with the detail rendered in Display) so the chain survives to the logging boundary - [packages/d2bd/src/composition.rs:13894, packages/d2bd/src/composition.rs:14127, packages/d2bd/src/composition.rs:15243, packages/d2bd/src/composition.rs:15247, packages/d2bd/src/composition.rs:16777, packages/d2bd/src/composition.rs:16785, packages/d2bd/src/composition.rs:16790, packages/d2bd/src/composition.rs:17015, packages/d2bd/src/composition.rs:17023]
  evidence: seed `let _ = |\.ok\(\);` = 53 hits; the 9 `detail: err.to_string()` sites are the chain-collapsing class, the rest are deliberate best-effort frame writes and shutdowns
- clean: seeds ran 41/53/1/0; all unwrap/expect hits are cfg(test) code, literal-built values (ShellName::new("primary"), own-constructed response objects), or startup invariants; the single unreachable!() (13622) is guarded by the Close/Cancel pre-check at 13510-13518 so it is not reachable from wire input; `let _ =` sites are deliberate best-effort writes/shutdowns

## serde
- clean: seeds ran 4/4/0/34; the derives (ShutdownDegradedReport/Marker camelCase, HostActivationMarkerState kebab-case) are consistent, no hand-written Deserialize impls, and all serde_json boundary calls map errors to wire codes or fall back explicitly

## obs
- d2bd-p2#7 sev=low blast=leaf effort=S verdict=actionable - `tracing::error!("Gateway Guest composition refused: root Zone generation unavailable")` is message-only although `topology.root` is in scope - fix: add `zone = %topology.root` (and the generation value if available) so the refusal is queryable per zone - [packages/d2bd/src/composition.rs:14781]
  evidence: seed `(info|debug|warn|error|trace)!\("` = 3 hits; this site has no fields and no enclosing span with the zone
- d2bd-p2#8 sev=low blast=leaf effort=S verdict=actionable - two identical message-only `tracing::warn!("resource plane still has live request owners during shutdown")` events in the two LiveRequestOwners branches of shutdown_resource_plane carry no fields, so the operator cannot tell which zones are stuck - fix: add `zones = ?zones` (or a count) to both events - [packages/d2bd/src/composition.rs:15195, packages/d2bd/src/composition.rs:15221]
  evidence: seed `(info|debug|warn|error|trace)!\("` = 3 hits; both warn sites are the duplicated message-only pair
- clean: seeds ran 0/3/0/158 (sampled: 40 of 158 tracing:: lines); the sampled events use named fields consistently (e.g. log_vm_start_report 17845-17874, log_host_prep_dag 17877-17893), no println/eprintln in the range, no secrets in fields, and the async-gate-allow marker at 10701 is a recorded deliberate exception

## docs
- clean: seeds ran 9/0/96 (sampled: 32 of 96 `-> Result<` lines); all 9 pub items in audio_host_controller.rs carry contract docs with one-line first sentences, the pub(crate) composition items in the range are documented, and no canonical-section or doctest gaps were found

## perf
- clean: seeds ran 90/18/18 (sampled: 30 of 90 format! lines); the format!/to_string sites are error paths, wire responses, and JSON payload building (cold by the card's own false-positive list), the Vec::new/BTreeMap::new sites are empty-case-common or plain-loop accumulations, and no hot-loop allocation was found

## conc
- clean: seeds ran 5/1/2/0; the AtomicU64 request-id counter uses Relaxed correctly (13804-13806), the std::thread spawns (12856) are dedicated daemon owner threads with names, the sleeps (16861, 17582) are on sanctioned synchronous paths, and the only Mutex is a cfg(test) journal buffer

## async
- clean: seeds ran 187/0/2/1 (sampled: 48 of 190 hits); the await chains are in async fns with proper error mapping, tokio::sync::Mutex guards (10884, 11064) are scoped and never held across await, the per-VM mutex map (10869, 10958) is lock striping, the single tokio::test is cfg(test), and the async-gate-allow marker at 10701 is a recorded deliberate exception

## unsafe
- clean: seeds ran 0/0/2/0; the two seed-3 hits are safe `io::Error::from_raw_os_error` constructors, not unsafe code - no `unsafe` blocks, fns, impls, or SAFETY comments exist in the scope

## ffi
- N/A (seeds: 0/0/0/0 all zero; no extern "C", no_mangle, catch_unwind, repr(C), or CStr/CString in the scope)

## macro
- N/A (seeds: 0/0/0/0 all zero; no macro_rules!, proc-macro, $crate, or to_compile_error in the scope)

## test
- clean: seeds ran 86/301/0/0 (sampled: 50 of 387 hits, every 8th); the in-range unit tests (guest_session_target_admission_tests, guest_target_session_tests, guest_component_session_cache_tests, audio_host_controller tests) and the tests/ suite assert behavior with messages (e.g. "the assignment survives the target loss"), no #[ignore] tests, no property/snapshot tooling, and no assertion that restates its implementation was found in the sample

## Coverage
- idiom: 1 finding(s)
- own: clean (seeds ran: 189/286/0/0; sampled 50 of 475)
- type: 2 finding(s)
- api: 2 finding(s)
- err: 1 finding(s)
- serde: clean (seeds ran: 4/4/0/34)
- obs: 2 finding(s)
- docs: clean (seeds ran: 9/0/96)
- perf: clean (seeds ran: 90/18/18)
- conc: clean (seeds ran: 5/1/2/0)
- async: clean (seeds ran: 187/0/2/1)
- unsafe: clean (seeds ran: 0/0/2/0; the two seed-3 hits are safe from_raw_os_error constructors, no unsafe code in scope)
- ffi: N/A (seeds: 0/0/0/0 all zero)
- macro: N/A (seeds: 0/0/0/0 all zero)
- test: clean (seeds ran: 86/301/0/0; sampled 50 of 387)