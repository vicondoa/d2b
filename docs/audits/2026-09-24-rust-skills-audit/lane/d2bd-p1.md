# d2bd-p1 - d2bd - part 1/8
Baseline: 6ebdd4cec22e6537d1376e83ff7a82b00a8492c0 | LOC audited: 13238 (excl. src/generated/**) | modules: src/resource_runtime.rs (whole file)
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: part 1/8: src/resource_runtime.rs (whole file, 13,238 lines)

## idiom
- d2bd-p1#3 sev=low blast=leaf effort=S verdict=actionable - `current_committed_resource` (9168) is a body-for-body duplicate of `committed_resource` (9153) plus an unused `_operation_id` parameter, and its only caller is `committed_wayland_session_for_vm` (4555) - fix: call `committed_resource` at 4555 and delete `current_committed_resource` - [packages/d2bd/src/resource_runtime.rs:9168, packages/d2bd/src/resource_runtime.rs:4555, packages/d2bd/src/resource_runtime.rs:9153]
  evidence: seeds `for \w+ in 0\.\.` = 1, `impl (Default|From|PartialEq|Eq|Debug|Clone|Hash) for` = 2, `let mut \w+ = (String|Vec)::new\(\)` = 17; duplicate bodies read at 9153-9210 vs 9168-9210; census: `current_committed_resource` over packages/, nixos-modules/, tests/, docs/reference/, labs/ = 1 file
- d2bd-p1#4 sev=low blast=leaf effort=S verdict=actionable - pointless `let setup = setup;` rebind in `reconcile_controller_sessions_locked` shadows the just-bound value to drop a mutability that was never declared - fix: bind once without `mut` and delete the rebind line - [packages/d2bd/src/resource_runtime.rs:6896]
  evidence: seed `let mut \w+ = (String|Vec)::new\(\)` = 17 hits; the rebind read at 6896 with the comment block 6893-6895 explaining the ordering that the rebind does not serve; remaining seed mass (1 index-loop hit at 6271, 2 hand-written Defaults at 839/867, 17 accumulators) is deliberate - the loop is a bounded retry, the Defaults preserve pinned wire defaults, and the accumulators have early exits or await-driven extends

## own
- d2bd-p1#5 sev=low blast=leaf effort=S verdict=actionable - avoidable `row.resource_ref.clone()` in `committed_controller_provider_identities`: the field is only borrowed by `committed_provider_spec` and then moved into the result map - fix: pass `&row.resource_ref` to `committed_provider_spec` and `identities.insert(row.resource_ref, (uid, generation))` after the call - [packages/d2bd/src/resource_runtime.rs:359]
  evidence: seed `\.clone\(\)` = 262 hits (sampled: 44 of 262 hits, every 6th); the sampled clone at 359 is the only one whose value survives only as a borrow target and can be moved instead
- d2bd-p1#6 sev=low blast=leaf effort=S verdict=actionable - nine call sites pass `.to_owned()` into parsers that take `impl Into<String>` (d2b-contracts-resource identity.rs:79,155,279,393), where `&str: Into<String>` makes the allocation unnecessary - fix: drop `.to_owned()` at 181, 4625, 9911, 9931, 10096, 10138, 10212, 11078, 11079, 11082 - [packages/d2bd/src/resource_runtime.rs:181, packages/d2bd/src/resource_runtime.rs:4625, packages/d2bd/src/resource_runtime.rs:9931, packages/d2bd/src/resource_runtime.rs:10096, packages/d2bd/src/resource_runtime.rs:11078]
  evidence: seed `\.to_owned\(\)|\.to_vec\(\)|\.to_string\(\)` = 102 hits; parser signatures read at packages/d2b-contracts-resource/src/v3/identity.rs:79 (`pub fn parse(value: impl Into<String>)`); all nine sites pass a `&str` that `Into<String>` accepts directly; the remaining sampled clone mass (44 of 262 hits, every 6th) is explainable - lock-guard escapes (1192), constructor inputs (2059, 6975), Arc clones at spawn/shared-state boundaries (5984, 11236), map-key/return copies (2220, 7231)

## type
- d2bd-p1#7 sev=low blast=leaf effort=S verdict=actionable - `ZoneResourceRuntime` carries three gate booleans `policy_installed`/`controller_endpoint_registered`/`watch_admitted` (3041-3043) with only two reachable states - (true, false, false) at open (3230-3232) and (true, true, true) after `activate_published_bundle` (3470-3472) - so six impossible combinations are representable - fix: replace the trio with one enum (e.g. `PlanePublicationStage { BootstrapOnly, Published }`) read by `readiness_error` (8104-8116) - [packages/d2bd/src/resource_runtime.rs:3041, packages/d2bd/src/resource_runtime.rs:3230, packages/d2bd/src/resource_runtime.rs:3470, packages/d2bd/src/resource_runtime.rs:8104]
  evidence: seeds `fn validate_\w+|fn check_\w+` = 3, `is_\w+: bool|\w+_flag: bool` = 0, `(mode|kind|state): String` = 0; the flag trio and its two write sites read at 3041-3043, 3230-3232, 3470-3472
- d2bd-p1#8 sev=low blast=leaf effort=S verdict=actionable - `ControllerSession` encodes ordered once-only teardown progress as three independent booleans `ingress_revoked`/`assignments_revoked`/`transport_closed` (1014-1016), so skipping or reordering a step (e.g. closing the transport before revoking assignments) is representable and silently leaks a lease or a revocation frame - fix: replace the three with a `TeardownStage` enum advanced monotonically in `remove_controller_session` (7614-7650) - [packages/d2bd/src/resource_runtime.rs:1014, packages/d2bd/src/resource_runtime.rs:7614]
  evidence: seeds `fn validate_\w+|fn check_\w+` = 3, `is_\w+: bool|\w+_flag: bool` = 0, `(mode|kind|state): String` = 0; flag fields read at 1013-1016 and their only writer `remove_controller_session` at 7614-7650; the three validate fns are wire-boundary checks on manager-served rows (the parse-once point), and the runtime's `Option` pairs are deliberately handled by `derive_interaction_state` (9271-9281)

## api
- clean: seeds `\bpub (fn|struct|enum|trait|type|const|mod) ` = 30, `pub .*\b(Arc|Rc|Box|RefCell)<` = 2, `^\s*pub use ` = 2; the two `Arc<Mutex<...>>`/`Arc<...>` returns (11161, 11200) are justified shared ownership - census: `network_admission_index` callers at shared_provider_effects.rs:1132, shared_provider_effects.rs:2436, composition.rs:15350, and `ResourcePlane::zone` hands out the plane's own Arc - and the `pub use` arms (115, 126) are the house re-export pattern; all `pub` items are deliberate (ZoneResourceRuntime, ResourcePlane, HostNetworkAdmissionIndex) with private fields.

## err
- d2bd-p1#1 sev=medium blast=family effort=M verdict=actionable - `credential_dependency_row` swallows a manager RPC failure into absence (`.ok().flatten()`), contradicting the module's own contract that "a manager RPC failure is an error - never reported as absence" (bridge_manager_row doc, 299-305); the caller `ProductionCredentialRuntime` facts closure (4511) then reports no dependency facts, so a transient manager failure silently degrades credential readiness and revocation decisions - fix: propagate the error (log it with the error field at minimum; change `credential_dependency_facts`/`CredentialRuntime::dependency_facts` to `Result<Option<_>>` so the driver can retry) - [packages/d2bd/src/resource_runtime.rs:381, packages/d2bd/src/resource_runtime.rs:4511]
  evidence: seed `let _ = |\.ok\(\);` = 35 hits; the `.ok().flatten()` read at 381-383 versus the never-absence contract documented at 299-305; the remaining seed mass is clean - unwrap/expect = 223 (sampled: 45 of 223 hits, every 5th) with every non-test hit an invariant expect naming its reason (6003, 6992, 4993/5399/5741), the two `unreachable!` sites (3402, 6288) statically guaranteed, and the `let _ =` sites deliberate best-effort cleanup or fenced operations whose error propagates via `?`

## serde
- clean: seeds `derive\([^)]*(De)?[Ss]erialize` = 7, `serde\((rename_all|deny_unknown_fields|try_from|untagged|flatten|default|skip_serializing_if)` = 27, `impl .*Deserialize.*for` = 0, `serde_json::from_|serde_json::to_` = 44; all seven wire types (802-916) use `rename_all = "camelCase"` + `deny_unknown_fields` with pinned `default = "..."` functions, the hand-written `Default` impls preserve those pinned defaults, and every parsed config is validated before use (`parse_committed_clipboard_configuration` 9718, `parse_committed_notification_configuration` 9780) - the serde boundary is the admission gate it should be.

## obs
- d2bd-p1#9 sev=low blast=leaf effort=M verdict=actionable - eighteen message-only `tracing::warn!` events carry no named fields and the file has no spans at all (`\.instrument\(|#\[instrument` = 0), so the events lose the underlying error: most are inside `map_err` closures that drop the error (2024, 2419, 4846, 5283), and 7169 discards the in-scope `context` (provider/process) when a controller assignment refresh retries - fix: capture the error and log it as a named field (`error = ?...`) in the `map_err` closures, and add `provider`/`process` fields at 7169 - [packages/d2bd/src/resource_runtime.rs:2024, packages/d2bd/src/resource_runtime.rs:2419, packages/d2bd/src/resource_runtime.rs:4846, packages/d2bd/src/resource_runtime.rs:5283, packages/d2bd/src/resource_runtime.rs:7169]
  evidence: seed `(info|debug|warn|error|trace)!\("` = 18 hits; seed `\.instrument\(|#\[instrument` = 0 (no enclosing span anywhere in the file); the field-less sites read at 2024-2071, 2419-2467, 4846-5364, 7169; the other seeds are clean - `\bprintln!\(|\beprintln!\(` = 0, `tracing::|log::` = 134 with the field-carrying events well-formed (7134, 7719), and no secret or identifier in any field

## docs
- d2bd-p1#10 sev=low blast=leaf effort=S verdict=actionable - the public-request get deadline literal `meta.deadline_ms = 30_000` appears four times (8294, 8390, 10242, 10280) with no comment naming the why (which peer or operation enforces it) - fix: extract `const PUBLIC_GET_DEADLINE_MS: u64 = 30_000;` with a why-comment and use it at all four sites - [packages/d2bd/src/resource_runtime.rs:8294, packages/d2bd/src/resource_runtime.rs:8390, packages/d2bd/src/resource_runtime.rs:10242, packages/d2bd/src/resource_runtime.rs:10280]
  evidence: seeds `^\s*pub (fn|struct|enum|trait|const|type)` = 30, `/// # (Examples|Errors|Panics|Safety)` = 0, `-> Result<` = 182; the four deadline literals read at 8294, 8390, 10242, 10280 with no surrounding comment; the rest of the surface is clean - all 30 public items carry prose doc contracts with one-line first sentences (3006, 3777, 10709, 11125), the module has a `//!` doc (1-9), and failure conditions are documented in prose (4725-4734, 3777-3788) - the absent `# Errors` sections are a style choice in a bin crate where `missing_docs` is not appropriate

## perf
- d2bd-p1#2 sev=medium blast=leaf effort=S verdict=actionable - three arms of `CloudHypervisorResourceSession::call` compute an operation id that is immediately discarded: `UpdateSpec` (2360-2364), `UpdateStatus` (2489-2505, `let _ = &operation_id`), and `DeleteChild` (2856-2859, `let _operation_id`) each build `operation_payload` and run a full SHA-256 `canonical_digest` plus a `format!` allocation that no caller reads - this runs on every provider status/spec update, i.e. every reconcile pass - fix: delete the dead digest/format computation and the `let _` bindings in all three arms - [packages/d2bd/src/resource_runtime.rs:2360, packages/d2bd/src/resource_runtime.rs:2489, packages/d2bd/src/resource_runtime.rs:2505, packages/d2bd/src/resource_runtime.rs:2856]
  evidence: static (unmeasured); seed `format!\(` = 21 hits; the dead bindings read at 2505 (`let _ = &operation_id;`) and 2856 (`let _operation_id = format!)...)`) with no later use of `operation_id` in the UpdateSpec arm (only `let _ = (&owner_ref, &payload, &operation_id)` at 2414); the remaining seed mass is clean - other `format!` sites are cold paths (960, error rendering), and the `Vec::new()` sites (72 hits) are page-capped list builders or empty-case-common accumulators

## conc
- clean: seeds `std::thread::|thread::spawn|thread::scope` = 0, `\bMutex<|\bRwLock<` = 42, `Atomic\w+|Ordering::` = 26, `thread_local!|unsafe impl (Send|Sync) for` = 0; all 42 mutex hits are `tokio::sync::Mutex` fields whose guards are held across `.await` (the correct choice, e.g. 1052-1117, 3010-3067), the atomics are `AtomicBool` flags with paired Acquire/Release (`system_core_rebind_pending` 14/8637, `finalizer_clear_requested` 11522, reconcile shutdown 6121-6131), and there are no manual `Send`/`Sync` claims or `thread_local!`.

## async
- clean: seeds `async fn|async move|\.await` = 537 (sampled: 49 of 537 hits, every 11th), `tokio::spawn|spawn_blocking|JoinSet|select!\(|join!\(` = 6, `tokio::sync::(Mutex|RwLock|Notify)` = 80, `#\[tokio::(main|test)\]|Runtime::block_on` = 1; no blocking work sits on an executor worker (the bundle reload is routed to the bounded loader worker at 5338-5341), the reconcile waker arms `Notify` before the check (6129-6160), guards are dropped before re-locking (`close_guest_session` 1942-1955) or are `tokio::sync` guards held deliberately under a documented lock order (3898-3900), cancellation paths abort and await their tasks (7644-7650), and the only `#[tokio::test]` (12714) is a current-thread harness.

## unsafe
- unsafe: N/A (seeds: 0/0/0/0 all zero; no `unsafe` blocks/fns/impls, no `// SAFETY:` comments, no transmute/raw-pointer/MaybeUninit sites, and no `unsafe_code` attribute in the file)

## ffi
- ffi: N/A (seeds: 0/0/0/0 all zero; no `extern "C"`/`no_mangle`, no `catch_unwind`, no `repr(C)`/`repr(transparent)`, no `CStr`/`CString`/`c_char` in the file)

## macro
- macro: N/A (seeds: 0/0/0/0 all zero; no `macro_rules!`, no proc-macro/syn/quote, no `$crate`, no `to_compile_error`/`new_spanned` in the file)

## test
- clean: seeds `#\[test\]|#\[tokio::test\]` = 38, `assert_eq!\(|assert_ne!\(|assert!\(` = 128, `proptest!|insta::assert|rstest` = 0, `#\[ignore\]` = 0; the tests are behavior-focused with regression documentation (reconnect fence 11313, endpoint gate 11887, policy digest 11931, manager merge 12206, owner fence 12269), table-driven loops carry failure messages (11936-11968), error assertions match variants not Display strings (11587-11666), and no test restates implementation or computes its expectation with the code under test.

## Coverage
- idiom: 2 finding(s)
- own: 2 finding(s)
- type: 2 finding(s)
- api: clean (seeds ran: 30/2/2)
- err: 1 finding(s)
- serde: clean (seeds ran: 7/27/0/44)
- obs: 1 finding(s)
- docs: 1 finding(s)
- perf: 1 finding(s)
- conc: clean (seeds ran: 0/42/26/0)
- async: clean (seeds ran: 537/6/80/1)
- unsafe: N/A (seeds: 0/0/0/0 all zero; no unsafe sites in the file)
- ffi: N/A (seeds: 0/0/0/0 all zero; no FFI surface in the file)
- macro: N/A (seeds: 0/0/0/0 all zero; no macros in the file)
- test: clean (seeds ran: 38/128/0/0)