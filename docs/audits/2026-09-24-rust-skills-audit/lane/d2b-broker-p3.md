# d2b-broker-p3 - d2b-broker - part 3/7
Baseline: 6ebdd4cec22e6537d1376e83ff7a82b00a8492c0 | LOC audited: 11019 (excl. src/generated/**) | modules: live_handlers, state_cells, ops::{store_sync, usbip_host, storage_contract, pidfd, sysctl, mod}
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: part 3/7 (whole files; no range splits)

## idiom
- clean: seeds `for \w+ in 0\.\.`=2 / `impl (Default|From|PartialEq|Eq|Debug|Clone|Hash) for`=3 / `let mut \w+ = (String|Vec)::new\(\)`=6; every hit is justified - the two index loops are test-only (state_cells.rs:1511, store_sync.rs:737), the hand-written impls are required (io::Error has no PartialEq; RetentionPolicy/FakePidfdSpawner Defaults carry magic values a derive would lose), and the Vec::new accumulation loops (live_handlers.rs:607,2156,2299; storage_contract.rs:229) are early-exit/error-return ancestor walks where iterator pipelines do not fit.

## own
- d2b-broker-p3#1 sev=low blast=leaf effort=S verdict=actionable - `RealPidfdSpawner::spawn` clones the whole payload argv into a never-read `_argv` binding on every spawn - fix: delete `let _argv = payload.argv.clone();` (keep the explanatory comment; the placeholder child needs no argv) - [packages/d2b-broker/src/ops/pidfd.rs:210]
  evidence: seed `\.clone\(\)` = 53 hits; site read shows the binding is never read after the clone.

## type
- d2b-broker-p3#2 sev=low blast=leaf effort=M verdict=needs-contract - `StorageContractError::Refused { reason: String }` carries a closed set of refusal slugs ("storage-path-parent-dir-refused", "storage-path-outside-owned-roots", "storage-path-escapes-owned-root", ...) that tests string-match (storage_contract.rs:380-403) and audit records emit - fix: introduce a `RefusalReason` enum (serde lowercase) so the slug set is exhaustive and a new refusal cannot typo; wire/audit-visible strings make this needs-contract - [packages/d2b-broker/src/ops/storage_contract.rs:24, packages/d2b-broker/src/ops/storage_contract.rs:380]
  evidence: type seeds `fn validate_\w+|fn check_\w+`=8 / `is_\w+: bool|\w+_flag: bool`=0 / `(mode|kind|state): String`=0; slug literals cross-referenced in the module's own tests.
- d2b-broker-p3#3 sev=low blast=leaf effort=M verdict=needs-contract - `reload_behavior` is a stringly-typed wire value re-validated at every call site (`validate_nm_reload_behavior` here and the remove path in ops/nm.rs, per the doc at live_handlers.rs:288-290) instead of parsed once at the bundle boundary - fix: parse `reloadBehavior` into an enum in the bundle resolver so both arms branch on the parsed type and a hand-declared typo fails at resolution; `reloadBehavior` is pinned in docs/reference/schemas/v1/host.json:409 and v2/host.json:543, so needs-contract - [packages/d2b-broker/src/live_handlers.rs:291, packages/d2b-broker/src/live_handlers.rs:362]
  evidence: type seed `fn validate_\w+` = 8 hits; site read of the validator and its two call sites.

## api
- d2b-broker-p3#4 sev=medium blast=leaf effort=S verdict=actionable - `ops::sysctl` exports a dead pub surface: `apply_sysctl_intents` (sysctl.rs:84), `ApplySysctlRequest` (16), `with_default_root` (23) and `intent_to_proc_path` (76) are referenced only inside sysctl.rs (its own tests at 258/283); the production entry point is `apply_with_readback` - fix: delete the dead trio (or demote to `pub(crate)` and keep only what tests need) - [packages/d2b-broker/src/ops/sysctl.rs:16, packages/d2b-broker/src/ops/sysctl.rs:84]
  evidence: census: `apply_sysctl_intents` over packages/, nixos-modules/, tests/, docs/reference/, labs/ = 1 file (the defining file only); `with_default_root` = 1 file; `intent_to_proc_path` = 1 file.

## err
- d2b-broker-p3#5 sev=low blast=leaf effort=S verdict=actionable - `CellStore` panic policy is inconsistent: `in_memory()` (state_cells.rs:348) and `with_retention()` (360) `.expect()` on `spawn_owner` failure while the sibling `open()` (353) propagates `CellStoreError::Io` from the same call - fix: make `with_retention` return `Result<Self, CellStoreError>` (its callers are tests), and have `in_memory` keep its infallible contract only with an `expect` that names the startup-precondition rationale - [packages/d2b-broker/src/state_cells.rs:348, packages/d2b-broker/src/state_cells.rs:360]
  evidence: seed `\.unwrap\(\)|\.expect\(` = 274 hits (5 in production zones; the rest are cfg(test)); site read of spawn_owner and its three callers.

## serde
- d2b-broker-p3#6 sev=low blast=leaf effort=S verdict=actionable - `DurableRecord.outcome` is a String on the durable wire format, re-parsed by hand in `load()` (state_cells.rs:924-931), while the in-process `CellOutcome` enum already exists - fix: derive Serialize/Deserialize on a wire enum (`#[serde(rename_all = "lowercase")]` over `CellOutcome` or a dedicated `DurableOutcome`) and delete the string match; the serialized shapes "unknown"/"completed" stay identical, so no DURABLE_VERSION bump is needed - [packages/d2b-broker/src/state_cells.rs:337, packages/d2b-broker/src/state_cells.rs:924]
  evidence: seeds `derive\([^)]*(De)?[Ss]erialize`=2 / `serde\((rename_all|deny_unknown_fields|try_from|untagged|flatten|default|skip_serializing_if)`=4 / `impl .*Deserialize.*for`=0 / `serde_json::from_|serde_json::to_`=9.

## obs
- d2b-broker-p3#7 sev=low blast=leaf effort=S verdict=actionable - `retry_acl_grant` interpolates its `label` into the message instead of a named field: `tracing::debug!(error = %err, "{label} ACL refresh not ready yet")` and `tracing::warn!("{label} ACL refresh timed out")`, with no enclosing span carrying it, so the refresh kind is not queryable - fix: emit `label = %label` as a field and keep the message interpolation-free - [packages/d2b-broker/src/live_handlers.rs:2532, packages/d2b-broker/src/live_handlers.rs:2539]
  evidence: seed `(info|debug|warn|error|trace)!\("` = 1 hit (plus the sibling debug! at 2532 read in context); obs1 `\bprintln!\(|\beprintln!\(` = 3 (all cfg(test) skip notices).

## docs
- d2b-broker-p3#8 sev=low blast=leaf effort=S verdict=actionable - pub types `UsbipHostInspectionError` (usbip_host.rs:17), `UsbipDriverBinding` (153) and `UsbipHostDeviceInspection` (160) carry no doc comment at all on a crate with a lib target - fix: add one-line docs (and field docs for the inspection struct) - [packages/d2b-broker/src/ops/usbip_host.rs:17, packages/d2b-broker/src/ops/usbip_host.rs:153, packages/d2b-broker/src/ops/usbip_host.rs:160]
  evidence: docs seeds `^\s*pub (fn|struct|enum|trait|const|type)`=59 / `/// # (Examples|Errors|Panics|Safety)`=0 / `-> Result<`=116; site read of the three items.
- d2b-broker-p3#9 sev=low blast=leaf effort=S verdict=actionable - pub types `ApplySysctlOutcome` (sysctl.rs:32), `ApplySysctlError` (40) and `ApplyWithReadbackError` (113) and the pub method `with_default_root` (23) are undocumented - fix: add one-line docs per item - [packages/d2b-broker/src/ops/sysctl.rs:32, packages/d2b-broker/src/ops/sysctl.rs:40, packages/d2b-broker/src/ops/sysctl.rs:113]
  evidence: docs seeds 59/0/116; site read of the items.
- d2b-broker-p3#10 sev=low blast=leaf effort=S verdict=actionable - pub enum `StorageContractError` (storage_contract.rs:21) has no doc comment - fix: one-line doc naming the refusal/invalid/Io contract - [packages/d2b-broker/src/ops/storage_contract.rs:21]
  evidence: docs seeds 59/0/116; site read.
- d2b-broker-p3#11 sev=low blast=leaf effort=S verdict=actionable - pub methods `PidfdMethod::as_str` (pidfd.rs:112), `StartTime::matches` (127), `RealPidfdSpawner::new` (191) and `AuditDecision::as_str` (ops/mod.rs:164) are undocumented - fix: one-line docs per method - [packages/d2b-broker/src/ops/pidfd.rs:112, packages/d2b-broker/src/ops/pidfd.rs:191, packages/d2b-broker/src/ops/mod.rs:164]
  evidence: docs seeds 59/0/116; site read of the items.

## perf
- d2b-broker-p3#12 sev=low blast=leaf effort=M verdict=actionable - `CellStore` partial-key lookups scan the whole record map: `contains` (state_cells.rs:470), `payload` (488), `remove` (506), `keys` (524) and `clear` (541) iterate `records: BTreeMap<CellKey, CellRecord>` filtering on (cell, invocation_id) because the principal is a key component, making every op O(n) where the durable file already uses the nested cell -> invocation layout - fix: mirror the durable layout in memory (cell -> invocation -> record, principal inside the record) so lookups become O(log n); static (unmeasured) - [packages/d2b-broker/src/state_cells.rs:470, packages/d2b-broker/src/state_cells.rs:488, packages/d2b-broker/src/state_cells.rs:524]
  evidence: static (unmeasured); perf seeds `format!\(`=185 (production hits are error paths) / `Vec::new\(\)|...`=31 / `\.to_string\(\)`=53.

## conc
- clean: seeds `std::thread::|thread::spawn|thread::scope`=2 / `\bMutex<|\bRwLock<`=3 / `Atomic\w+|Ordering::`=16 / `thread_local!|unsafe impl (Send|Sync) for`=0; all concurrency is the sanctioned R4 dedicated bounded worker (state_cells.rs:343,583,606,901 `#[allow)..., reason = "dedicated bounded worker per plan R4")]` sync_channel + owner thread with the documented poison latch), and the remaining hits are cfg(test) scopes/atomics (state_cells.rs:1509, usbip_host.rs:489-551, live_handlers.rs:5605).

## async
- d2b-broker-p3#13 sev=medium blast=leaf effort=M verdict=actionable - the initial ACL-grant attempt runs a blocking setfacl fork/exec on the executor worker: `refresh_spawn_runner_acls` (async, live_handlers.rs:1802) -> `refresh_obs_vsock_acl` -> `grant_obs_vsock_acl_once` (1614) -> `setfacl_fd_safe` -> `setfacl_fd_safe_op_classed` (1416) -> `sys::pidfd_sys::run_setfacl_op_on_fd`, while the retry paths (`spawn_obs_vsock_acl_retry` 1658, `retry_acl_grant` 2519) correctly defer the same shellout to `background.dispatches.run` on the bounded dispatch pool - fix: route the initial attempt through `dispatches.run` too (or make the refresh fns async and use the `setfacl_verified_device` async-shellout shape), matching the documented "kernel-path step on the bounded dispatch pool" design; bounded short shellout per spawn, so medium not high - [packages/d2b-broker/src/live_handlers.rs:1614, packages/d2b-broker/src/live_handlers.rs:1802]
  evidence: async seeds `async fn|async move|\.await`=367 / `tokio::spawn|spawn_blocking|JoinSet|select!\(|join!\(`=0 / `tokio::sync::(Mutex|RwLock|Notify)`=2 / `#\[tokio::(main|test)\]|Runtime::block_on`=60; call-chain read; the store_sync.rs:666 flock site is a sanctioned per-site allow ("synchronous path", U1 ledger 4) and the state_cells worker is the sanctioned R4 boundary, both left unflagged.

## unsafe
- clean: seeds `\bunsafe \{|\bunsafe fn|\bunsafe impl|\bunsafe extern`=0 / `// SAFETY:`=0 / `transmute|from_raw|MaybeUninit|mem::zeroed`=3 / `unsafe_code`=3; the three `from_raw` hits are safe nix constructors (`Uid::from_raw`/`Gid::from_raw` at storage_contract.rs:279,307, `Pid::from_raw` in a test at live_handlers.rs:5387) and the `unsafe_code` hits are doc prose (pidfd.rs:20,176,208); no unsafe block exists in this partition - all unsafe lives in sys.rs (d2b-broker-p5's scope).

## ffi
- clean: seeds `extern "C"|no_mangle|unsafe\(link_section`=0 / `catch_unwind`=1 / `repr\(C\)|repr\(transparent\)`=0 / `CStr|CString|c_char`=1; the `catch_unwind` at state_cells.rs:635 is the documented panic-isolation boundary of the cell-store poison latch, not a foreign-caller boundary, and the `CString` hit is doc prose (live_handlers.rs:2857); no FFI surface exists in this partition.

## macro
- N/A (seeds: `macro_rules!`=0 / `proc_macro|syn::|quote!`=0 / `\$crate`=0 / `to_compile_error|new_spanned`=0 all zero; no macro definitions or proc-macro machinery in the lane files).

## test
- d2b-broker-p3#14 sev=low blast=leaf effort=S verdict=actionable - `reconciliation_refuses_start_time_drift` (tests/pidfd_handoff_scm_rights.rs) asserts the Display string (`msg.contains("start-time drifted")`) instead of the error variant, while the sibling real-spawner test matches `PidfdOpError::ReconciliationStartTimeMismatch` - fix: match the variant like tests/pidfd_real_spawner.rs:66-70 - [packages/d2b-broker/tests/pidfd_handoff_scm_rights.rs:90]
  evidence: test seeds over lane src: `#\[test\]|#\[tokio::test\]`=114 / `assert_eq!\(|assert_ne!\(|assert!\(`=351 / `proptest!|insta::assert|rstest`=0 / `#\[ignore\]`=0; over tests/: 58/219/0/0; the in-module suites (state_cells, store_sync, usbip_host, storage_contract, sysctl) are invariant-focused and fail-closed, no other findings.

## Coverage
- idiom: clean (seeds ran: 2/3/6)
- own: 1 finding(s)
- type: 2 finding(s)
- api: 1 finding(s)
- err: 1 finding(s)
- serde: 1 finding(s)
- obs: 1 finding(s)
- docs: 4 finding(s)
- perf: 1 finding(s)
- conc: clean (seeds ran: 2/3/16/0)
- async: 1 finding(s)
- unsafe: clean (seeds ran: 0/0/3/3)
- ffi: clean (seeds ran: 0/1/0/1)
- macro: N/A (seeds: 0/0/0/0 all zero; no macro definitions in lane files)
- test: 1 finding(s)