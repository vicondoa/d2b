# d2bd-p3 - d2bd - part 3/8
Baseline: 6ebdd4cec22e6537d1376e83ff7a82b00a8492c0 | LOC audited: 10801 (excl. src/generated/**) | modules: composition.rs (20143-30213), zone_enrollment.rs
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: src/composition.rs:20143-30213, src/zone_enrollment.rs

## idiom
- d2bd-p3#8 sev=low blast=leaf effort=S verdict=actionable - dispatch_live_guest_activation_resource builds the identical resource-List json and identical drive_sync dispatch twice (rollback branch and next-ordinal branch), differing only in post-processing - fix: hoist the `list` json and `runtime.dispatch_public_cli_request(&list)` call (with its map_err) above the `if mode == DaemonActivationMode::Rollback` split and branch only the filter/max computation - [packages/d2bd/src/composition.rs:20450-20461, packages/d2bd/src/composition.rs:20486-20498]
  evidence: static comparison of the two blocks (same "zoneRef"/"service"/"method"/"resourceType"/"executionRef"/"limit": 256 payload, same drive_sync + map_err shape); seed `for \w+ in 0\.\.` = 10 (all test accept loops), `let mut \w+ = (String|Vec)::new\(\)` = 2 (test helpers), `impl (Default|From|...) for` = 0
- d2bd-p3#9 sev=low blast=leaf effort=S verdict=actionable - qemu_media_registry_state takes `_registry_dir: &str` and never reads it (the probe reads global state), so every caller passes a value into a dead parameter - fix: drop the parameter and the `registry_dir` argument at the sole call site - [packages/d2bd/src/composition.rs:21306-21319, packages/d2bd/src/composition.rs:21291]
  evidence: `_registry_dir` appears only in the signature (fn body 21313 calls the zero-arg `qemu_media_probe_registry_records()` at composition.rs:10000); census: `qemu_media_registry_state` over packages/ = 1 call site
- clean: seeds 1-3 ran (10/0/2); the 10 index loops are accept-loop test harvesters and the two Vec::new accumulators are test broker helpers; no hand-written Default/From/Debug/Clone impls, no statement-style production accumulation found

## own
- d2bd-p3#4 sev=low blast=leaf effort=S verdict=actionable - `let zone = guard.zone().clone()` clones the ZoneId although `guard` can be borrowed for the whole body (it is only used again by its own Drop at scope end) - fix: bind `let zone = guard.zone();` and pass `&zone` to plane.zone and the json! formatters - [packages/d2bd/src/composition.rs:20404]
  evidence: seed `\.clone\(\)` = 82 lane hits; non-test hits (35) read in full, this is the only borrow-replaceable clone in non-test code; sampled: 50 of 394 test-mass hits (every 8th), all fixture-owned or required
- d2bd-p3#5 sev=low blast=leaf effort=S verdict=actionable - typed_error_from_resolution_error clones `workload_id` while destructuring an owned error; the binding can be moved into TypedError::WorkloadAliasConflict because `candidates` is only joined by reference - fix: bind `workload_id` (no `.clone()`) in the AliasConflict arm - [packages/d2bd/src/composition.rs:21091]
  evidence: seed `\.clone\(\)` = 82 lane hits; err is passed by value and neither field is used after construction of the TypedError
- d2bd-p3#6 sev=low blast=leaf effort=S verdict=actionable - `ResourceName::parse(readable.clone())` clones a just-built String although parse takes `impl Into<String>` and `&readable` converts without allocation - fix: `ResourceName::parse(&readable)` - [packages/d2bd/src/composition.rs:20638]
  evidence: `d2b_contracts_resource::v3::ResourceName::parse(value: impl Into<String>)` (packages/d2b-contracts-resource/src/v3/resource.rs:44); seed `\.clone\(\)` = 82 lane hits

## type
- d2bd-p3#2 sev=low blast=leaf effort=S verdict=needs-contract - HostActivationPendingMarker.mode is a stringly-typed activation mode on a persisted marker: it is deserialized, logged and rendered but never validated against the known label set, while the in-Rust mode already exists as DaemonActivationMode - fix: replace `mode: String` with a serde-mirrored enum (e.g. `DaemonActivationMode` behind kebab-case serde, or a marker-local enum) and validate on read; the marker file is written by out-of-tree activation machinery, so the serialized label set is a contract - [packages/d2bd/src/composition.rs:20146, packages/d2bd/src/composition.rs:20280, packages/d2bd/src/composition.rs:21135]
  evidence: seed `(mode|kind|state): String` = 1 hit (composition.rs:20146); marker read boundary at 20260-20277; census: `HostActivationPendingMarker` over packages/ + nixos-modules/ = 3 files (composition.rs, d2bd-runtime/metrics.rs via metric label, docs/reference/daemon-api.md), no Rust writer in-tree

## api
- N/A: seeds all zero in this partition (pub items 0, `pub .*\b(Arc|Rc|Box|RefCell)<` 0, `pub use ` 0); part 3 contains no exported surface - `pub(crate)` items in zone_enrollment.rs (ZONE_ENROLLMENT_PORT, GuestEnrollmentEndpoint)are crate-internal by design

## err
- d2bd-p3#1 sev=medium blast=leaf effort=S verdict=actionable - dispatch_audit maps any unrecognized severity string from the wire to `TypedError::InternalIo { context: "audit filter", detail: "severity-invalid" }`, surfacing caller input errors as internal I/O failures instead of a request-validation refusal - fix: return a wire-input refusal kind (e.g. a TypedError::Wire* invalid-request variant or the invalid_request_response frame used by mutating dispatch) for the `Some(_) =>` arm - [packages/d2bd/src/composition.rs:22793-22797, packages/d2b-contracts-control/src/public_wire.rs:2453]
  evidence: seed `\.unwrap\(\)|\.expect\(` = 537 lane hits (non-test sites read in full: 8, all documented invariants); the arm's kind is TypedError::InternalIo (packages/d2bd-runtime/src/typed_error.rs:490-493), an internal category for a user-typable field
- d2bd-p3#7 sev=medium blast=leaf effort=S verdict=actionable - ActivationLockGuard::drop silently swallows `finish_activation` failure (`let _ =`), so a coordinator refusal to close an activation is never even logged and the wedge is only discoverable via the deferred activation-pending marker - fix: log the error with tracing::warn! (boundary has no Result channel; the marker alone is not enough) - [packages/d2bd/src/composition.rs:20185-20190]
  evidence: seed `let _ = |\.ok\(\);` = 63 lane hits; this is the only non-test `let _ =` on a fallible call (20188) outside cfg(test) cleanup sites

## serde
- d2bd-p3#3 sev=low blast=leaf effort=S verdict=needs-contract - HostActivationPendingMarker.schema_version is deserialized but never validated, so a future marker version with a compatible field set would silently parse as current - fix: check `schema_version == 1` on read (refuse with a typed log/error otherwise) or drop the field from the read path if versioning is not enforced; the marker file is written by out-of-tree activation machinery, so its shape is a contract - [packages/d2bd/src/composition.rs:20144, packages/d2bd/src/composition.rs:20275, packages/d2bd/src/composition.rs:20298]
  evidence: census: `schema_version` over packages/d2bd/src = 7 hits, 1 for this type (20144) and no read site anywhere (the other hits are unrelated types: 3669/8497/16146/28121); seed `serde_json::from_|serde_json::to_` = 56 lane hits

## obs
- clean: seeds 1-4 ran (println 0, interpolated no-field events 0, instrument 0, tracing refs 18); every tracing event in the part uses named fields (vm = %marker.vm, endpoint = %path.display(), error = %error, activation_id, state = ?) and no event interpolates a message; no secret-bearing field spotted in the 18 sites (mode/activation_id are non-secret opaque identifiers); no subscriber installed (library/binary split respected)

## docs
- d2bd-p3#10 sev=low blast=leaf effort=S verdict=actionable - the activation generations List limit `"limit": 256` is duplicated as an undocumented magic literal in both branches of dispatch_live_guest_activation_resource - fix: hoist to a named constant (e.g. `const ACTIVATION_GENERATIONS_LIST_LIMIT: u64`) with a comment naming why 256 (bounded retained NixosGeneration scan) - [packages/d2bd/src/composition.rs:20451, packages/d2bd/src/composition.rs:20495]
  evidence: seed `^\s*pub (fn|struct|...)` = 0, `/// # (Examples|Errors|Panics|Safety)` = 0, `-> Result<` = 18 (all private binary-crate fns); the two literals are textually identical with no comment

## perf
- clean: seeds 1-3 ran (format! 55, collection news 54, to_string 28); every format!/to_string hit outside tests is cold (error detail strings, activation marker path names, one-shot generation names, per-poll response envelopes in a network-wait loop); the per-VM scoped workers in build_public_list/build_public_status are justified because provider probes can block up to PUBLIC_STATUS_PROVIDER_PROBE_TIMEOUT; no hot-path allocation pattern found; static (unmeasured) throughout

## conc
- clean: seeds 1-4 ran (thread 21, Mutex/RwLock 9, Atomic 8, thread_local 0); non-test concurrency is the documented shape: scoped-thread data parallelism for list/status builds, `Arc<tokio::sync::Mutex<ZoneCoordinator>>`/`Arc<Mutex<ZoneEnrollmentServer>>` shared state with multiple owners, atomics only in tests (NEXT_TEST_ID); no manual Send/Sync impls, no static mut; the deliberate serialization of one zone's enrollments through one mutex is documented at zone_enrollment.rs:220-225

## async
- clean: seeds 1-4 ran (async fn/.await 78, spawn/JoinSet/select 2, tokio::sync refs 94, tokio main/test 10); no guard held across .await except the tokio::sync::Mutex held across serve() in spawn_accept_loop, which is the documented per-link serialization (zone_enrollment.rs:297-308); blocking work in async contexts is absent (the 250 ms sleep poll in dispatch_live_guest_activation_resource runs on the sync worker-thread dispatch path, marked `#[allow(clippy::disallowed_methods, reason = "synchronous path")]` at 20385); 2 async-gate-allow markers at 26155/26552 are sanctioned cfg(test) sites; startup marker scans use tokio::fs with let-else skipping (no panics, no blocking)

## unsafe
- clean: seeds 1-4 ran (unsafe blocks/fns/impls 0, `// SAFETY:` 0, transmute/from_raw/MaybeUninit/zeroed 12, unsafe_code 0 in scope); all 12 from_raw hits are safe constructors (`io::Error::from_raw_os_error`, `nix::unistd::Pid::from_raw`) inside test helpers, not unsafe blocks; no unsafe code exists in this partition, so no SAFETY-comment obligations arise (workspace `unsafe_code = "forbid"` is inherited as recorded in U1 (d) 1)

## ffi
- N/A: seeds all zero (extern "C"/no_mangle 0, catch_unwind 0, repr(C)/repr(transparent) 0, CStr/CString 0); the part crosses no foreign-language boundary

## macro
- N/A: seeds all zero (macro_rules! 0, proc_macro/syn/quote 0, $crate 0, to_compile_error 0); no macros defined or consumed beyond std macros

## test
- clean: seeds 1-4 ran in partition scope (test attrs 139, asserts 457, proptest/insta/rstest 0, #[ignore] 2); the two `#[ignore]` tests are documented flakes ("flaky on shared hosts; Unix socket reuse races", composition.rs:25679-25680) which U1 (c) test lists as acceptable; sampled 50 of 596 hits (attrs every 3rd, asserts every 10th) and read test neighborhoods 21355-21463, 24537-24567, 26078-26167, zone_enrollment.rs:595-683: tests assert typed error kinds (error.kind()/assert_eq!(error, "bundle-intent-missing:store-view")), real wire round-trips through FramedVsockTransport with human-written expected values, fail-closed behavior and ordering, with per-case messages; no self-fulfilling expectation or assert-less test spotted; tests/ directory corpus is shared across d2bd parts and outside this partition's module scope

## Coverage
- idiom: 2 finding(s)
- own: 3 finding(s)
- type: 1 finding(s)
- api: N/A (seeds: 0/0/0 all zero; no pub items in this partition, pub(crate) only)
- err: 2 finding(s)
- serde: 1 finding(s)
- obs: clean (seeds ran: 0/0/0/18)
- docs: 1 finding(s)
- perf: clean (seeds ran: 55/54/28)
- conc: clean (seeds ran: 21/9/8/0)
- async: clean (seeds ran: 78/2/94/10)
- unsafe: clean (seeds ran: 0/0/12/0; all 12 from_raw hits are safe constructors)
- ffi: N/A (seeds: 0/0/0/0 all zero; no foreign boundary)
- macro: N/A (seeds: 0/0/0/0 all zero; no macro definitions)
- test: clean (seeds ran: 139/457/0/2)