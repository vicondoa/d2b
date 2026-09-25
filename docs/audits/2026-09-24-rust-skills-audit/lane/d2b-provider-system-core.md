# d2b-provider-system-core - d2b-provider-system-core
Baseline: 6ebdd4cec22e6537d1376e83ff7a82b00a8492c0 | LOC audited: 1810 (excl. src/generated/**, src 1229 + tests 581) | modules: whole crate (error, host, lib, ownership, testing, user)
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: whole crate

## idiom
- d2b-provider-system-core#1 sev=low blast=leaf effort=S verdict=actionable - `UserIdentityDigest::to_hex` pushes hex nibbles with `char::from_digit)...).unwrap_or('0')`, a fallback that can never fire (from_digit is total for 0-15 at radix 16) - fix: const `HEX: [char; 16]` table lookup, or `write!(out, "{byte:02x}")` via `std::fmt::Write` which pushes without allocating - [src/user.rs:75, src/user.rs:76]
  evidence: seeds `for \w+ in 0\.\.` = 0, `impl (Default|From|PartialEq|Eq|Debug|Clone|Hash) for` = 0, `let mut \w+ = (String|Vec)::new\(\)` = 0; dead fallback read in to_hex body
- d2b-provider-system-core#2 sev=low blast=leaf effort=S verdict=actionable - `UserReconciler::required_bindings` is an associated fn that never uses `Self`, the shape the naming rule calls a free function - fix: free `required_bindings(spec: &UserSpec)` in user.rs, updating the two test call sites (tests/user_discovery.rs:45, tests/user_discovery.rs:73) - [src/user.rs:232]
  evidence: seeds 0/0/0; impl block user.rs:214-298 read, no Self use

## own
- d2b-provider-system-core#3 sev=low blast=leaf effort=S verdict=actionable - `reconcile_observed` copies `kernel_release`/`os_name` out of a by-value `HostProbeSnapshot` with `to_owned()` where destructuring the owned snapshot moves the Strings - fix: `let HostProbeSnapshot { capabilities, kernel_release, os_name, user_manager_available, minijail_gate, active_process_count } = snapshot;` at the method top and move fields into the report - [src/host.rs:466, src/host.rs:467]
  evidence: seed `\.to_owned\(\)|\.to_vec\(\)|\.to_string\(\)` = 3 (host.rs:378 to_vec is status-owned, fine; 466/467 are the copies)
- clean: seeds `\.clone\(\)` = 4, `\.to_owned\(\)|\.to_vec\(\)|\.to_string\(\)` = 3, `Rc<|RefCell<|Arc<Mutex<|Arc<RwLock<` = 0, `Cow<` = 0; every clone is explainable (status ownership at host.rs:372/user.rs:308, local mutable required-set at host.rs:429, scripted-port replay at testing.rs:95)

## type
- clean: seeds `fn validate_\w+|fn check_\w+` = 0, `is_\w+: bool|\w+_flag: bool` = 0, `(mode|kind|state): String` = 0; no flag soup, Option pairs, or stringly state - the two bool observations (cgroup_kill_available, user_manager_available) are single probe facts, not state flags

## api
- d2b-provider-system-core#4 sev=medium blast=leaf effort=M verdict=actionable - `pub mod testing` (lib.rs:41) ships the hand-rolled `block_on` driver, `ScriptedDiscoveryPort`, and fixture set unconditionally in the library surface although only this crate's own `tests/` consumes them - fix: gate behind a `test-support = []` feature (house pattern: d2b-provider-host/Cargo.toml:28, d2b-provider-user/Cargo.toml:30) with `#[cfg(feature = "test-support")]` and `required-features` on the three [[test]] targets - [src/lib.rs:41, src/testing.rs:23]
  evidence: census: `d2b_provider_system_core::testing` over packages/, nixos-modules/, tests/, docs/reference/, labs/ = 3 hits, all this crate's own tests/
- d2b-provider-system-core#5 sev=low blast=leaf effort=S verdict=actionable - `pub mod ownership` (lib.rs:40) makes `OWNED_RESOURCE_TYPES`/`DISOWNED_RESOURCE_TYPES` reachable at two paths (module plus root re-export), the refused two-path shape; its fns `owns`/`require_owned`/`require_resource_type` have no external callers - fix: `mod ownership;` private, keep the root re-export (lib.rs:49) - [src/lib.rs:40, src/ownership.rs:42]
  evidence: census: `system_core::ownership|system_core::owns|system_core::require_owned|system_core::require_resource_type` over packages/, nixos-modules/, tests/, labs/ = 0 hits outside the crate
- d2b-provider-system-core#6 sev=low blast=leaf effort=S verdict=actionable - `HostReconciler::reconcile_observed` (host.rs:419) and `HostProbeSnapshot` (host.rs:134, root re-export lib.rs:46) are pub with zero external callers; the doc frames reconcile_observed as a conformance/fault-injection seam no consumer uses yet - fix: `pub(crate)` both until a consumer exists, or keep as the documented seam - [src/host.rs:419, src/host.rs:134]
  evidence: census: `reconcile_observed|reconcile_with_probe|HostReconciler` over packages/, nixos-modules/, tests/, labs/ = external callers only for `reconcile` (d2b-provider-host/effects_service.rs:179) and `reconcile_with_probe` (d2b-provider-host/effects_service.rs:163); reconcile_observed is called only internally (host.rs:531)
- d2b-provider-system-core#7 sev=low blast=leaf effort=S verdict=actionable - `PROVIDER_UID` (lib.rs:70) is a zero-caller pub const whose doc says "the bus keeps its own copy"; d2b-bus consumes the generated `BOOTSTRAP_PROVIDER_UID` (d2b-contracts-zone-session/src/generated/service_provider_catalog.rs:15) with the identical value - fix: have d2b-bus import `d2b_provider_system_core::PROVIDER_UID` (or land the daemon re-home the doc promises) or drop the const until wired - [src/lib.rs:70]
  evidence: census: `PROVIDER_UID` over packages/, nixos-modules/, tests/, docs/reference/, labs/ = 1 hit, the definition itself
- d2b-provider-system-core#8 sev=medium blast=wide effort=L verdict=needs-contract - `HostReconciler::reject_operator_status_fields` (host.rs:389), documented as the enforcement half of the ADR-046 no-suppression obligation ("both are met here", host.rs:13), has zero callers outside this crate's tests - the "operators can neither suppress nor override" posture rule is test-only today - fix: wire the check into the daemon's status admission path (d2b-resource-api `update_status`, service.rs:735, or the daemon's Host status publication) or document the structural exclusion - [src/host.rs:389, src/host.rs:13]
  evidence: census: `reject_operator_status_fields` over packages/, nixos-modules/, tests/, labs/ = 4 hits, all this crate's tests/; no daemon/broker/admission caller (status admission surface is docs/reference/daemon-api.md territory)

## err
- clean: seeds `\.unwrap\(\)|\.expect\(` = 10 (all `expect` in src/testing.rs fixture constructors on literal values - the recorded false-positive class), `let _ = |\.ok\(\);` = 0, `\bpanic!\(|\bunreachable!\(|\btodo!\(|\bunimplemented!\(` = 0, `enum \w*Error` = 1; SystemCoreError is a closed value-free taxonomy, tests assert error variants never Display strings

## serde
- clean: seeds 11 hits (5 Serialize derives + 6 rename_all attrs); the one hand-written impl (`Serialize for UserIdentityDigest`, user.rs:88) deliberately renders hex; no Deserialize admission gates, no try_from gaps, rename_all consistent (kebab-case enums, camelCase status)

## obs
- clean: seeds `\bprintln!\(|\beprintln!\(` = 0, `(info|debug|warn|error|trace)!\("` = 0, `\.instrument\(|#\[instrument` = 0, `tracing::|log::` = 2; every event uses named fields with lazy `%`/`?` values and a static message - no interpolation, no secrets (value-free error enum; resource refs are public status per the redaction tests)

## docs
- d2b-provider-system-core#9 sev=medium blast=leaf effort=S verdict=actionable - public Result-returning items carry no `# Errors` sections, and some fail non-obviously (`HostProbeSnapshot::new` rejects control characters and >64/128-byte strings with `HostProbeFailed`; `MinijailPlatformGate::validate` maps kernel/cgroup failures to two distinct variants) - fix: add `# Errors` sections to validate, HostProbeSnapshot::new, reject_operator_status_fields, reconcile/reconcile_observed/reconcile_with_probe, UserReconciler::reconcile, and the two effect ports' methods - [src/host.rs:121, src/host.rs:161, src/user.rs:241]
  evidence: seed `/// # (Examples|Errors|Panics|Safety)` = 0 across 74 pub items; seed `-> Result<` = 9

## perf
- clean: seeds `format!\(` = 0, `Vec::new\(\)|VecDeque::new\(\)|HashMap::new\(\)|BTreeMap::new\(\)` = 0, `\.to_string\(\)` = 0; the only allocation site (to_hex, 64-byte String) is a cold serialization path

## conc
- d2b-provider-system-core#10 sev=low blast=leaf effort=S verdict=actionable - `ScriptedDiscoveryPort.calls: Mutex<u32>` (testing.rs:43) uses a tokio::sync::Mutex for a counter - the crate's only tokio use - and `call_count` (testing.rs:79) silently reports 0 on contention via try_lock - fix: `AtomicU32` with `fetch_add`/`load` (Relaxed) and drop `tokio = { workspace = true, features = ["sync"] }` from Cargo.toml - [src/testing.rs:43, src/testing.rs:79]
  evidence: seed `\bMutex<|\bRwLock<` = 1 (testing.rs:43); `tokio::` appears only at testing.rs:9 in src/

## async
- d2b-provider-system-core#11 sev=low blast=leaf effort=S verdict=actionable - `block_on` (testing.rs:23) busy-spins (`std::hint::spin_loop()`, testing.rs:30) on `Poll::Pending`, so any future that genuinely yields - a contended tokio Mutex, a future test with real I/O - hangs the test process at 100% CPU instead of failing; the doc comment asserts hermiticity but nothing enforces it - fix: `debug_assert!` the never-pending invariant or drive these tests with a real runtime - [src/testing.rs:30, src/testing.rs:19]
  evidence: seed `async fn|async move|\.await` = 15 hits; block_on body read (Waker::noop + spin_loop)

## unsafe
- N/A: seeds `\bunsafe \{|\bunsafe fn|\bunsafe impl|\bunsafe extern` = 0, `// SAFETY:` = 0, `transmute|from_raw|MaybeUninit|mem::zeroed` = 0; manifest carries `unsafe_code = "forbid"` (Cargo.toml [lints.rust])

## ffi
- N/A: seeds `extern "C"|no_mangle|unsafe\(link_section` = 0, `catch_unwind` = 0, `repr\(C\)|repr\(transparent\)` = 0, `CStr|CString|c_char` = 0

## macro
- N/A: seeds `macro_rules!` = 0, `proc_macro|syn::|quote!` = 0, `\$crate` = 0, `to_compile_error|new_spanned` = 0

## test
- d2b-provider-system-core#12 sev=low blast=leaf effort=S verdict=actionable - tests/host_reconciliation.rs repeats the `Probe` struct literal plus a 5-field `HostProbeMetadata` block five times (lines 204, 230, 254, 280, 306), one field differing per case - fix: a `Probe::new(capabilities, user_manager_available, gate, kernel_release)` constructor or default-and-mutate helper - [tests/host_reconciliation.rs:204, tests/host_reconciliation.rs:230]
  evidence: seed `#\[test\]|#\[tokio::test\]` = 23 (22 integration + 1 unit), `assert_eq!\(|assert_ne!\(|assert!\(` = 49, `proptest!|insta::assert|rstest` = 0, `#\[ignore\]` = 0; five Probe constructions read

## Coverage
- idiom: 2 finding(s)
- own: 1 finding(s)
- type: clean (seeds ran: 0/0/0)
- api: 5 finding(s)
- err: clean (seeds ran: 10/0/0/1)
- serde: clean (seeds ran: 11)
- obs: clean (seeds ran: 0/0/0/2)
- docs: 1 finding(s)
- perf: clean (seeds ran: 0/0/0)
- conc: 1 finding(s)
- async: 1 finding(s)
- unsafe: N/A (seeds: 0/0/0 all zero; manifest `unsafe_code = "forbid"`)
- ffi: N/A (seeds: 0/0/0/0 all zero)
- macro: N/A (seeds: 0/0/0/0 all zero)
- test: 1 finding(s)