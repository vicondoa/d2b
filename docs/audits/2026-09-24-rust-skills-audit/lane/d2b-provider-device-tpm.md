# d2b-provider-device-tpm - d2b-provider-device-tpm
Baseline: 6ebdd4cec22e6537d1376e83ff7a82b00a8492c0 | LOC audited: 3333 (excl. src/generated/**, none present) | modules: whole crate
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: none (single-part lane)

## idiom
- d2b-provider-device-tpm#1 sev=low blast=leaf effort=S verdict=actionable - the swtpm log-level bound is spelled twice: lib.rs exports MIN_SWTPM_LOG_LEVEL/MAX_SWTPM_LOG_LEVEL (1/20) which runner.rs uses, while swtpm_argv.rs:160 hardcodes `1..=20` in generate_swtpm_argv, so a bound change in one place silently drifts from the other - fix: import crate::{MIN_SWTPM_LOG_LEVEL, MAX_SWTPM_LOG_LEVEL} in swtpm_argv.rs and replace the literal range - [swtpm_argv.rs:160, lib.rs:63, lib.rs:65]
  evidence: idiom seed 2 (`impl (Default|From|PartialEq|Eq|Debug|Clone|Hash) for`) = 1 hit (runner.rs:18, deliberate invariant-preserving Default, not a finding); the bound duplication was read directly at the two sites
- clean: idiom seeds ran: 0/1/0; no index loops, no statement-style accumulation; the one hand-written Default preserves the 20 default a derive cannot express; argv building uses Vec::with_capacity(20) push pipelines

## own
- d2b-provider-device-tpm#2 sev=low blast=leaf effort=S verdict=actionable - avoidable clones of Option<ResourceRef> and String where a reborrow suffices: resource_controller.rs:233-234 clones self.volume_ref only to borrow it, resource_controller.rs:247 clones self.process_ref the same way, effects_service.rs:280 clones self.device_ref to pass `&self.device_ref` to key(), and effects_service.rs:450 clones self.zone to call zone.as_str() on a live self - fix: use self.volume_ref.as_ref().ok_or(...)?, self.process_ref.as_ref(), self.key(&self.device_ref), and self.zone.as_str() - [resource_controller.rs:233, resource_controller.rs:247, effects_service.rs:280, effects_service.rs:450]
  evidence: own seed 1 (`.clone()`) = 26 hits, seed 2 (`.to_owned()|.to_vec()|.to_string()`) = 46 hits, all sites read; the remaining clones are required (dual ownership into DeclaredTpmRows + port at effects_service.rs:684-692, borrowed-input error paths in swtpm_argv.rs, test fixtures)
- clean: own seed 3 (`Rc<|RefCell<|Arc<Mutex<|Arc<RwLock<`) = 0, seed 4 (`Cow<`) = 0; no Arc<Mutex> shared state outside the tokio Mutex covered under async; the Arc<dyn TpmRuntime> facet clone is genuine shared ownership

## type
- clean: type seeds ran: 1/1/0; validate_absolute (swtpm_argv.rs:119) is a single-boundary validator on a wire-shaped input type (a newtype would rewrite the refused SwtpmArgvInput fields), and watched_configuration_is_dependency (resource_controller.rs:18) is a cutover-contract constant with one constructor - both judged deliberate, no illegal-state finding

## api
- d2b-provider-device-tpm#3 sev=medium blast=leaf effort=S verdict=actionable - the state.rs token module (StateDirectoryToken, TamperMarkerToken, StateOwnerToken, StateDirIntent, state.rs:6-107, re-exported lib.rs:36-38) has zero consumers repo-wide at HEAD, so the refusal ledger's stated reason for keeping it ("the daemon references them", over-engineering-audit-record.md:386, rows 71/72) is no longer evidenced - the daemon-side uses were deleted by the same applied finding - fix: delete state.rs and its re-export, or wire the daemon side that the record claims exists - [state.rs:6, state.rs:29, state.rs:51, state.rs:73, lib.rs:36]
  evidence: census: `StateDirIntent|StateDirectoryToken|StateOwnerToken|TamperMarkerToken` over packages/ nixos-modules/ tests/ docs/reference/ labs/ = 0 hits outside packages/d2b-provider-device-tpm (only state.rs + lib.rs:36-38); changed evidence vs the record row
- d2b-provider-device-tpm#4 sev=low blast=leaf effort=S verdict=actionable - the exported inspect-tpm effects service is unwired: TPM_EFFECTS_SERVICE (effects_service.rs:53), TpmEffectsService, and TpmEffectsServiceFactory (effects_service.rs:103-116, whose facets field carries #[allow(dead_code)] for an R5 respawn path "not wired yet") are never registered, while d2bd registers every sibling provider's factory in shared_provider_effects.rs:3434-3493 without the TPM one - fix: register the factory there, or delete the service surface until the respawn path is wired - [effects_service.rs:53, effects_service.rs:103, effects_service.rs:114]
  evidence: census: `TpmEffectsServiceFactory|TPM_EFFECTS_SERVICE` over packages/ nixos-modules/ tests/ docs/reference/ labs/ = 0 hits outside the crate
- d2b-provider-device-tpm#5 sev=low blast=leaf effort=S verdict=actionable - LiveTpmResourceEffectPort is pub (effects_service.rs:341) but is only constructed and consumed inside effects_service.rs (into_port at effects_service.rs:697), never appearing in a public signature or external caller - fix: make it pub(crate) - [effects_service.rs:341]
  evidence: census: `LiveTpmResourceEffectPort` over packages/ nixos-modules/ tests/ docs/reference/ labs/ = 0 hits outside the crate
- d2b-provider-device-tpm#6 sev=low blast=leaf effort=S verdict=actionable - LegacyMigrationOutcome (migration.rs:5, re-exported lib.rs:24) has zero callers repo-wide: the "closed outcome of the broker-owned one-time legacy state adoption" is consumed by no broker or daemon code at HEAD - fix: delete the enum and its re-export, or wire the broker consumer it documents - [migration.rs:5, lib.rs:24]
  evidence: census: `LegacyMigrationOutcome` over packages/ nixos-modules/ tests/ docs/reference/ labs/ = 0 hits
- clean: api seed 1 = 91 pub items, seed 2 = 1 hit (facets.rs:35 pub runtime: Arc<dyn TpmRuntime> - genuine shared ownership across facet clones, not a leak), seed 3 = 7 pub use arms (house single-surface pattern); the remaining exports (builders, controller, reconcile/finalize entry points, vocabulary) are consumed by d2bd or by this crate's integration tests

## err
- d2b-provider-device-tpm#7 sev=medium blast=leaf effort=S verdict=actionable - two same-named error enums for one domain: runner.rs:43 SwtpmArgvError (one variant, LogLevelOutOfRange with no payload, returned by SwtpmSettings::validate) and swtpm_argv.rs:104 SwtpmArgvError (six variants including LogLevelOutOfRange { level }), both reachable from the crate root (lib.rs:35 re-exports the runner one; pub mod swtpm_argv exposes the other), so callers must disambiguate by module path and the two same-named LogLevelOutOfRange variants differ in shape - fix: make SwtpmSettings::validate return swtpm_argv::SwtpmArgvError::LogLevelOutOfRange { level } and delete runner::SwtpmArgvError - [runner.rs:43, swtpm_argv.rs:104, lib.rs:35, tests/conformance.rs:11]
  evidence: err seed 4 (`enum \w*Error`) = 4 hits (resource_controller.rs:98, resource_effect.rs:10, runner.rs:43, swtpm_argv.rs:104); the two SwtpmArgvError definitions read in full
- clean: err seeds ran: 143/0/0/4; all 143 unwrap/expect hits are in #[cfg(test)] modules or on literally-built DurationMs::parse values in the production builders (resources.rs:215-270, fixed literals with fixed bounds - card false-positive class); no panic!/unreachable!/todo!/unimplemented!; no swallowed Results; TpmResourceControllerError wraps TpmResourceEffectError without leaking path/broker detail

## serde
- clean: serde seeds ran: 5/7/0/14; SwtpmSettings carries deny_unknown_fields + serde default + a validate() admission gate (the conformance test pins the unknown-field refusal); no hand-written Deserialize; serde_json use is boundary rendering (declared child documents) and tests; TpmResourcePhase is Serialize-only for status rendering

## obs
- clean: obs seeds ran: 0/0/0/11; all 11 tracing events use named fields (device=, error=, phase=, reason=) with %/? formatting; zero println/eprintln, zero interpolated messages, no instrument spans needed on these short paths

## docs
- d2b-provider-device-tpm#8 sev=low blast=leaf effort=M verdict=actionable - Result-returning public items never enumerate their error variants: 47 `-> Result<` items (build_tpm_state_volume_spec, build_swtpm_process_spec, build_swtpm_flush_spec, generate_swtpm_argv, generate_swtpm_ioctl_flush_argv, TpmResourceController::new/reconcile/finalize, SwtpmSettings::validate, reconcile_device_tpm_controller, finalize_device_tpm_controller) carry one-line docs but no `# Errors` section, while the crate's doc standard is otherwise high (#![deny(missing_docs)] plus module docs) - fix: add `# Errors` sections naming the TpmResourceEffectError/TpmResourceControllerError/SwtpmArgvError variants each item can return - [resources.rs:53, swtpm_argv.rs:130, resource_controller.rs:132, resource_controller.rs:190]
  evidence: docs seed 3 (`-> Result<`) = 47 hits, seed 2 (`/// # (Examples|Errors|Panics|Safety)`) = 0 hits
- d2b-provider-device-tpm#9 sev=low blast=leaf effort=S verdict=actionable - swtpm_argv.rs:39 `#![allow(missing_docs)]` is redundant: every pub item in the module is already documented and the crate root denies missing docs, so the opt-out lets a future undocumented pub item pass silently - fix: remove the module-level allow - [swtpm_argv.rs:39]
  evidence: docs seed 1 = 78 pub items, all with doc comments; swtpm_argv.rs read in full

## perf
- clean: perf seeds ran: 20/8/1; format! sites are swtpm argv rendering (the artifact text), one-shot inspect/JSON payloads, and error paths (cold); Vec::new() sites are empty-case metadata; the single to_string is a cold payload render; the argv builder pre-sizes with with_capacity(20) - no hot-loop allocation

## conc
- clean: conc seeds ran: 0/3/20/0; the only production synchronization is tokio::sync::Mutex<bool> (covered under async); the AtomicBool/AtomicUsize/Ordering hits are test-only SeqCst counters in tests/resource_controller.rs; no threads, no thread_local, no unsafe Send/Sync impls

## async
- d2b-provider-device-tpm#10 sev=medium blast=family effort=M verdict=actionable - prepare_state_dir (effects_service.rs:412) runs blocking work on the executor worker: a synchronous broker round-trip envelope_invoke_kernel (effects_service.rs:467, blocking connect/poll/recv up to kernel_io_timeout) plus NSS lookups nix::unistd::User::from_name/Group::from_name (effects_service.rs:518,525) in row_posture, none marked async-gate-allow (the crate's inventory lists only the 3 test lock sites) - fix: move the invoke and the NSS resolution off the worker (spawn_blocking or an async broker client); the same pattern exists in d2b-provider-supervisor/process/process-systemd/network-local and d2bd, so consolidation may treat it as one family class - [effects_service.rs:412, effects_service.rs:467, effects_service.rs:518, effects_service.rs:525]
  evidence: async seed 1 (`async fn|async move|\.await`) = 76 hits; static (unmeasured); async-gate inventory for this crate = 3 sites, all test locks (effects_service.rs:865,874,887)
- d2b-provider-device-tpm#11 sev=low blast=leaf effort=S verdict=actionable - lifecycle_lease_consumed: tokio::sync::Mutex<bool> (effects_service.rs:361,698) guards a flag owned by exactly one task (into_port builds a fresh port per reconcile/finalize call and uses it once), and consume_lifecycle_lease holds the guard across `.await` (effects_service.rs:384-394); the lock can never contend - fix: replace with AtomicBool (preserves &self + Sync) or restructure the once-gate - [effects_service.rs:361, effects_service.rs:384, effects_service.rs:698]
  evidence: async seed 3 (`tokio::sync::(Mutex|RwLock|Notify)`) = 2 hits (effects_service.rs:361,698); port construction read at effects_service.rs:697-700

## unsafe
- N/A: seeds 1-3 all zero (no unsafe blocks/fns/impls, no SAFETY comments, no transmute/from_raw/MaybeUninit/zeroed); seed 4 alone = `unsafe_code = "forbid"` in Cargo.toml:9 plus a doc-comment mention (swtpm_argv.rs:38) - a forbid attribute does not make the lens applicable; the crate contains no unsafe code

## ffi
- N/A: seeds 1-4 all zero (no extern "C"/no_mangle/link_section, no catch_unwind, no repr(C)/repr(transparent), no CStr/CString/c_char)

## macro
- N/A: seeds 1-4 all zero (no macro_rules!, no proc_macro/syn/quote!, no $crate, no to_compile_error/new_spanned)

## test
- clean: test seeds ran: 36/119/0/0; behavior-focused suite with no ignored tests: golden byte-parity against tests/golden/runner-shape/swtpm-argv-minimal.txt (swtpm_argv.rs:275), round-trips through the real v3 contract types (resources.rs:362), typed error variants via matches!/assert_eq, phase transitions, the owner fence, and the flush one-shot-outcome gate; deterministic (no network, no clock injection needed); the custom block_on harness (tests/resource_controller.rs:230-242) busy-polls with a noop waker but every scripted effect completes synchronously, so no test can hang today

## Coverage
- idiom: 1 finding(s)
- own: 1 finding(s)
- type: clean (seeds ran: 1/1/0)
- api: 4 finding(s)
- err: 1 finding(s)
- serde: clean (seeds ran: 5/7/0/14)
- obs: clean (seeds ran: 0/0/0/11)
- docs: 2 finding(s)
- perf: clean (seeds ran: 20/8/1)
- conc: clean (seeds ran: 0/3/20/0)
- async: 2 finding(s)
- unsafe: N/A (seeds: 0/0/0/1 - seeds 1-3 all zero; manifest `unsafe_code = "forbid"` at Cargo.toml:9)
- ffi: N/A (seeds: 0/0/0/0 all zero)
- macro: N/A (seeds: 0/0/0/0 all zero)
- test: clean (seeds ran: 36/119/0/0)