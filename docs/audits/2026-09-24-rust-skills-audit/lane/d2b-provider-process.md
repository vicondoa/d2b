# d2b-provider-process - d2b-provider-process
Baseline: 6ebdd4cec | LOC audited: 10721 (src 10469, tests 252, excl. src/generated/**) | modules: backend, driver, effects, effects_service, execution, facets, identity, launch_identity, operations, test_support (cfg-gated), worker_launch
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: whole crate

## idiom
- clean: seeds ran (1/1/0): one `for _ in 0..16` yield helper in a test (driver.rs:2657, a fixed-count yield loop where an iterator pipeline has no meaning) and one hand-written `Default for FakeFacetsConfig` (test_support.rs:88) whose scripted values (`VecDeque::from([ProviderAdoption::Absent])`, `Ok(ProcessIdentityDigest::from_bytes([0x51; 32]))`) a derive cannot express. No index loops, no statement-style accumulation, no replaceable hand-written impls.

## own
- d2b-provider-process#1 sev=low blast=leaf effort=S verdict=actionable - `envelope.provider_ref.clone().expect("checked")` clones the `Option<ResourceRef>` at three call sites where `as_ref().expect("checked")` borrows without copying - fix: replace `.clone().expect("checked")` with `.as_ref().expect("checked")` in the `identity()` call at recover, reconcile, and delete - [packages/d2b-provider-process/src/driver.rs:1988, packages/d2b-provider-process/src/driver.rs:2056, packages/d2b-provider-process/src/driver.rs:2105]
  evidence: `\.clone\(\)` seed, 3 of 72 driver.rs clone hits; `check_provider` ran immediately before each site so the invariant is already named by the expect, and the clone buys nothing.
- d2b-provider-process#2 sev=low blast=leaf effort=S verdict=actionable - `bind_cloud_hypervisor_guest_uid` takes `argv: &[String]` and clones the whole argv at both return paths (`Ok(argv.to_vec())` and `let mut bound = argv.to_vec()`), while its sole caller never uses `launch_argv` afterwards - fix: take `argv: Vec<String>` by value and return it (caller passes `launch_argv` directly), removing both copies - [packages/d2b-provider-process/src/operations.rs:1354, packages/d2b-provider-process/src/operations.rs:1362, packages/d2b-provider-process/src/operations.rs:2649]
  evidence: `\.to_vec\(\)` seed, 2 of 86 operations.rs to_owned/to_vec hits; census: `bind_cloud_hypervisor_guest_uid` over packages/ = 1 call site (operations.rs:2649), which reads `launch_argv` only through this call.

## type
- clean: seeds ran (5/0/0): the five `validate_*`/`check_*` functions (driver.rs:919 `check_provider`, operations.rs:576 `validate_request_fds`, 756 `validate_typed_process_metadata`, 987 `validate_sandbox_launch_plan`, 1208 `validate_spawn_runner_request_matches_intent`) are boundary fences on wire input, which is where validation belongs; the `typed: bool` parameter is an input-mode flag for one fence, not state. No boolean-flag fields, no stringly-typed state, no validate-at-every-callsite repetition.

## api
- clean: seeds ran (135/3/15): the pub surface is deliberate and single-path (lib.rs re-export arms are the house pattern); `Arc` in public signatures is genuine shared ownership with visible call sites (`process_spec_decoder() -> Arc<dyn SpecDecoder>` is Arc-cloned into every descriptor, driver.rs:291/619; `ProcessEffectFacets` Arc fields are shared between the driver and the effects service, facets.rs:409-413); `ProcessEffectError` is a closed `#[non_exhaustive]` enum with stable codes (backend.rs:190). No dependency types leak into signatures beyond the deliberate `d2b_process_conformance` re-export, which is the family-home contract.

## err
- d2b-provider-process#3 sev=low blast=leaf effort=S verdict=actionable - `.ok().and_then(...)` swallows the parse of a stored owning-row spec in the launch-identity path: a corrupt `VolumeBinding` or `Volume` row silently degrades to an unbound launch instead of refusing with `SpecInvalid` - fix: map the two `serde_json::from_slice` failures to `ProcessDriverErrorKind::SpecInvalid` (or return `None` only for genuinely absent rows, not for parse failures) in `identity()` and `serving_worker_launch()` - [packages/d2b-provider-process/src/driver.rs:1018-1030, packages/d2b-provider-process/src/driver.rs:1124-1130]
  evidence: `\.ok\(\);`/`.ok()` seed, 2 of 10 driver.rs `.ok()` sites; the downstream ticket fence (`provider-ticket:template-not-found`) still refuses closed, so severity stays low.
- clean: seeds ran (185/14/8/2): all `unwrap`/`expect` outside tests sit on literally-built constants (operations.rs:159-213, 278-280, 1549, 2174, 2205) or after a check the compiler cannot see (`expect("checked")`, driver.rs:1988/2056/2105); every `panic!`/`unreachable!` hit is in `#[cfg(test)]`; `let _ =` sites are deliberate best-effort sends and requeues (RequeueId, not Result, driver.rs:1391-1722) and a payload-shape validation (`let _request`, operations.rs:2093); both error enums (ProcessEffectError, ProcessDriverErrorKind) are closed with stable codes and no caller string-matching.

## serde
- clean: seeds ran (18 total): the crate derives no Serialize/Deserialize types of its own; all serde use is boundary deserialization of wire specs (`ResourceSpec`, `ProcessSpec`, `EphemeralProcessSpec`, `VolumeBindingSpec`, `VolumeSpec`) with errors mapped to typed failures (driver.rs:884-911, operations.rs:425-459), plus best-effort metadata reads. No hand-written `Deserialize` impls, no `rename_all`/`deny_unknown_fields`/`flatten` decisions to judge on crate-owned types.

## obs
- clean: seeds ran (8/0/0/8): all eight `tracing::warn!` events carry named fields (`resource`, `provider`, `operation`, `error`, `restart_count`) with static messages (driver.rs:1046, 1183, 1248, 1552, 1759, 1809, 1871, 1894); zero `println!`/`eprintln!`; no interpolated-message-only events; error chains logged once at the handling boundary (`map_provider_error`, driver.rs:1893-1898).

## docs
- d2b-provider-process#4 sev=low blast=leaf effort=L verdict=actionable - no `# Errors` or `# Panics` canonical sections exist on any of the crate's 142 Result-returning items, so the failure contract of the public surface is prose-only - fix: add `# Errors` sections naming the closed codes to the public Result-returning items, starting with `ProcessEffectBackend::launch` (which closed codes each operation can raise) and `resolve_launch_identity` (which `LaunchIdentityError` variants are possible) - [packages/d2b-provider-process/src/backend.rs:256, packages/d2b-provider-process/src/launch_identity.rs:64]
  evidence: seed 2 `/// # (Examples|Errors|Panics|Safety)` = 0 hits; seed 3 `-> Result<` = 142 hits; `#![deny(missing_docs)]` (lib.rs) means every item is documented, the gap is section shape only.
- clean: seeds ran (119/0/142): module docs present in all 12 modules, first sentences carry the load, magic values documented with the why (e.g. `PROCESS_RESYNC` 5s cadence, driver.rs:99-102); no doctests exist and none are marked `ignore`.

## perf
- clean: seeds ran (78/24/27): every `format!` hit is an error-detail string, a one-shot diagnostic, or a per-operation path construction (cold; the recorded false-positive class); every `Vec::new()` is an empty-case struct field, an empty fd vector, or a test fixture; `to_string()` sits at wire-rendering and Display boundaries. No `format!` in any loop, no grow-by-push collection, no attacker-keyed hashing. static (unmeasured).

## conc
- d2b-provider-process#5 sev=medium blast=leaf effort=S verdict=policy-confirmed - production `parking_lot::Mutex` fields in `EphemeralRuntime` (`started_at`, `completed`) are a live use of a banned primitive with no per-site allow, and the lock calls run on the actor's executor thread - fix: switch the two fields to `tokio::sync::Mutex` (the already-named replacement) or `std::sync::Mutex` with the same short critical sections; requires the parking_lot ban carve-out to be re-opened otherwise - [packages/d2b-provider-process/src/driver.rs:680, packages/d2b-provider-process/src/driver.rs:682]
  evidence: seed 2 `\bMutex<` = 2 production hits (of 36 conc hits; the rest are test doubles and test-support); policy: clippy.toml:40-43 ("parking_lot is banned outright (plan KD3); the U33 short-lock carve-out is revoked") and clippy.toml:82 (replacement `tokio::sync::Mutex::lock`); no `#[allow(clippy::disallowed_methods)]` at the site, and the sanctioned-reason list (U1 d.4) does not cover it.
- d2b-provider-process#6 sev=low blast=leaf effort=S verdict=actionable - `RestartBudget`, `EphemeralRuntime.started`, and `DurableRuntime.watching` use `Ordering::SeqCst` for plain counters and flags that publish no other data, so the strongest ordering buys nothing over `Relaxed` - fix: switch the 16 `Ordering::SeqCst` sites in driver.rs to `Ordering::Relaxed` (no paired acquire/release handoff exists; the actor and the spawned launch task only gate on these flags) - [packages/d2b-provider-process/src/driver.rs:438, packages/d2b-provider-process/src/driver.rs:451, packages/d2b-provider-process/src/driver.rs:458, packages/d2b-provider-process/src/driver.rs:462, packages/d2b-provider-process/src/driver.rs:466, packages/d2b-provider-process/src/driver.rs:470, packages/d2b-provider-process/src/driver.rs:695, packages/d2b-provider-process/src/driver.rs:703, packages/d2b-provider-process/src/driver.rs:732, packages/d2b-provider-process/src/driver.rs:761, packages/d2b-provider-process/src/driver.rs:765, packages/d2b-provider-process/src/driver.rs:772]
  evidence: seed 3 `Atomic\w+|Ordering::` = 16 SeqCst sites in driver.rs (of 36 conc hits); no `unsafe impl Send/Sync`, no `thread_local!`, no `std::thread` usage in the crate.

## async
- clean: seeds ran (444/3/0/25): the three `tokio::spawn` sites (driver.rs:1232, 1795, 2507) pre-capture everything (`spec.clone()`, `identity.clone()`, `Arc::clone`) before the `'static` move and complete through a oneshot whose failed send is harmless when the actor is gone; no blocking call sits in an async context (the async-gate markers on test-support recorder locks are deliberate exceptions, driver.rs:2426-2437, test_support.rs:227-355); no guard is held across an `.await`; tests use `start_paused = true` for deterministic time. No `spawn_blocking`, `JoinSet`, `select!`, or `tokio::sync::Mutex` in the crate.

## unsafe
- N/A (seeds: 0/0/0/0 - the single `from_raw` match is rustix's safe `Pid::from_raw` constructor, a seed false positive; no `unsafe` blocks, fns, impls, or `// SAFETY:` comments; the manifest forbids `unsafe_code`)

## ffi
- N/A (seeds: 0/0/0/0 - no `extern "C"`, `no_mangle`, `catch_unwind`, `repr(C)`/`repr(transparent)`, or `CStr`/`CString` anywhere in the crate)

## macro
- N/A (seeds: 0/0/0/0 - no `macro_rules!`, proc-macro, `$crate`, or `to_compile_error` usage)

## test
- clean: seeds ran (42/283/0/0): 42 tests (25 driver, 9 effects_service, 3 backend, 2 launch_identity, 3 integration) assert behavior with `start_paused` determinism, table-driven cases with per-case failure messages (launch_identity.rs:213-395), error-code assertions that pin the stable wire codes rather than incidental Display text (driver.rs:2936, 3051-3061), and integration tests through the public registry surface (tests/process_family.rs). No `#[ignore]`, no property/snapshot tooling (no rule-shaped surface needs it), no network or clock reads, no test that cannot fail.

## Coverage
- idiom: clean (seeds ran: 1/1/0)
- own: 2 findings (seeds: 123/150/1/0; all 273 hits inspected)
- type: clean (seeds ran: 5/0/0)
- api: clean (seeds ran: 135/3/15)
- err: 1 finding (seeds: 185/14/8/2)
- serde: clean (seeds ran: 18 total)
- obs: clean (seeds ran: 8/0/0/8)
- docs: 1 finding (seeds: 119/0/142)
- perf: clean (seeds ran: 78/24/27)
- conc: 2 findings (seeds: 0/4/16/0)
- async: clean (seeds ran: 444/3/0/25)
- unsafe: N/A (seeds: 0/0/0/0 all zero; no unsafe blocks or unsafe_code allow)
- ffi: N/A (seeds: 0/0/0/0 all zero)
- macro: N/A (seeds: 0/0/0/0 all zero)
- test: clean (seeds ran: 42/283/0/0)