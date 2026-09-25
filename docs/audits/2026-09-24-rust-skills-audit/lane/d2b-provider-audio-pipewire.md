# d2b-provider-audio-pipewire - d2b-provider-audio-pipewire
Baseline: 6ebdd4cec22e6537d1376e83ff7a82b00a8492c0 | LOC audited: 2709 (excl. src/generated/**, none present) | modules: whole crate (src: authority, controller, lib, mediator, resource_type, state; tests: audio_policy, authority, controller, mediator, resource_type, state)
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test (+ supply per-crate note) | Partitions: whole crate (single-part lane; on the README-only integration ratchet, provider_crate_policy.rs:331-332)

## idiom
- clean: seeds `for \w+ in 0\.\.` = 0, `impl (Default|From|PartialEq|Eq|Debug|Clone|Hash) for` = 1, `let mut \w+ = (String|Vec)::new\(\)` = 0; the single hit is the hand-written `Default for AudioGrants` (resource_type.rs:161), which preserves the grants/policy-state invariant by seeding from `AudioPolicyState::default_v2()` - a listed repo false-positive class; no index loops, no statement-style accumulation, derives already present everywhere else

## own
- clean: seeds `.clone()` = 6 (src 3: controller.rs:250-252, tests 3), `.to_owned()|.to_vec()|.to_string()` = 7 (src 5, tests 2), `Rc<|RefCell<|Arc<Mutex<|Arc<RwLock<` = 1, `Cow<` = 0; every hit explainable - the controller.rs:250-252 clones feed `explicit_binding_children`, whose callee signature takes owned `ResourceRef` (boundary clone, cite d2b-contracts-provider `child_resources`), the `Arc<tokio::sync::Mutex>` arbiter is the documented multi-controller shared authority (authority.rs:44), `to_owned()` copies a `&'static str` const, test clones share one arbiter across two controllers; no borrow-fight clones, no `mem::take` candidates

## type
- d2b-provider-audio-pipewire#1 sev=low blast=leaf effort=S verdict=actionable - constructors re-validate invariants the admission gate already enforces: `AudioServiceSpec::owner` (endpoint type) and `AudioBindingSpec::new` (service/target ref types) return the same error variants `validate_audio_service`/`validate_audio_binding` produce, so the same invariant is checked at two layers and the constructors' `Result` promises a rejection path that is dead in practice - fix: drop the checks from `owner()`/`new()` (make them infallible) and keep `validate_audio_*` as the single parse-once admission gate, or delete the gate checks and keep the constructor checks - [src/resource_type.rs:64-70, src/resource_type.rs:117-125, src/resource_type.rs:206-231, src/resource_type.rs:233-246]
  evidence: seed `fn validate_\w+|fn check_\w+` = 3 hits (resource_type.rs:206,233,249); constructor checks at resource_type.rs:68-70 and 122-124 duplicate gate invariants with identical error variants (EndpointType, ReferenceType)
- d2b-provider-audio-pipewire#2 sev=low blast=leaf effort=S verdict=actionable - the shared-vs-owned controller mode is a private bool `activate_promoted` (controller.rs:210 vs 218-224) and `finalize`/`finalize_shared` are byte-identical delegations to `finalize_inner`, so the two public methods' behavioral difference is invisible in their signatures and a caller can invoke `finalize_shared` on an owned controller and get promotion activation anyway - fix: encode the mode in the type (typestate or a `MicrophoneHandoff::{Enable,Defer}` field set by construction) so the method contract holds by construction, or collapse the two methods into one documented by the constructor - [src/controller.rs:589-598, src/controller.rs:602-608, src/controller.rs:199]
  evidence: static read (seed `is_\w+: bool|\w+_flag: bool` = 0, manual catch); `finalize` and `finalize_shared` both body `self.finalize_inner(lease)`; `activate_promoted` true from `new()`, false from `with_shared_microphone`

## api
- d2b-provider-audio-pipewire#3 sev=low blast=leaf effort=S verdict=actionable - `SpeakerMixer::set_grant(lease, on: bool)` takes a boolean command and returns a bool whose meaning flips with the argument (true: was-empty, false: was-last), and the only caller ignores the revoke return (it calls `is_last_grant` first) - fix: split into `grant(lease) -> Result<bool, _>` (was-empty) and `revoke(lease) -> Result<bool, _>` (was-last), or return a named enum, so the return contract stops being argument-dependent - [src/authority.rs:157-171, src/controller.rs:395-403]
  evidence: static read; seed `pub .*\b(Arc|Rc|Box|RefCell)<` = 1 (authority.rs:44, unrelated shared-ownership alias); controller.rs:395-403 discards the `set_grant(lease, false)` return after `is_last_grant`
- d2b-provider-audio-pipewire#4 sev=low blast=leaf effort=S verdict=actionable - `register_service` is exported from lib.rs but has zero callers anywhere (the daemon's audio paths and the wayland-policy audio_registry validate specs directly), leaving a dead registration gate on the surface - fix: consume `register_service` in the daemon's audio Service registration path or drop the export (crate is 0.0.0-bootstrap, no semver gate) - [src/controller.rs:746-748, src/lib.rs:32]
  evidence: census `register_service` over packages/, nixos-modules/, tests/, docs/reference/, labs/ = 2 hits (definition + lib.rs re-export), 0 callers
- d2b-provider-audio-pipewire#5 sev=low blast=family effort=S verdict=needs-contract - `AudioLastSetApplied::OfflineOnly` is named as if it meant "applied offline only" while its doc says "No setting was applied in the current reconcile"; the variant is rendered to a wire-visible status string "OfflineOnly" by the wayland-policy projection and pinned in daemon tests - fix: rename the variant (e.g. `NotApplied`) and update the projection string and pinned expectations together - [src/controller.rs:124-133, packages/d2b-provider-wayland-policy/src/audio_registry.rs:117-122, packages/d2bd/src/resource_plane_v3.rs:4626]
  evidence: census `AudioLastSetApplied|OfflineOnly` over packages/ = 9 hits; wire rendering at audio_registry.rs:121 and pinned at resource_plane_v3.rs:4626 and audio_registry.rs:704,745

## err
- d2b-provider-audio-pipewire#6 sev=medium blast=leaf effort=S verdict=actionable - `MicrophoneArbiter::new(0)` and `SpeakerMixer::new(0)` panic via `assert!` on caller input to a pub constructor; the skill's panic policy says input validation is always a `Result`, and the type-level answer (`NonZeroUsize`) exists - fix: take `NonZeroUsize` (or return `Result<Self, _>`) in both constructors; no current caller passes 0 (daemon uses 64), so the change is mechanical - [src/authority.rs:53-54, src/authority.rs:144-145]
  evidence: seed `\bpanic!\(|\bunreachable!\(|\btodo!\(|\bunimplemented!\(` = 0; `assert!(max_queue > 0)` / `assert!(max_consumers > 0)` at authority.rs:54,145 (manual catch; pub constructor input)
- d2b-provider-audio-pipewire#7 sev=low blast=leaf effort=S verdict=actionable - the crate's error enums never chain sources: `AudioStateIoError`'s seven `io::Error` payloads and `AudioControllerError::Mediator(AudioMediatorError)` leave `Error::source()` returning `None`, flattening the chain into the Display message - fix: implement `std::error::Error::source()` for the payload variants (or move the crate to `thiserror` `#[source]`, which also removes the hand-written Display impls) - [src/state.rs:114-123, src/controller.rs:155-160]
  evidence: seed `enum \w*Error` = 5 (all wire-code Display impls, no source()); AudioStateIoError variants hold io::Error without #[source]-equivalent, AudioControllerError::Mediator wraps AudioMediatorError without chaining

## serde
- clean: seeds `derive)...Serialize` = 4, `serde)...)` = 6, `impl .*Deserialize.*for` = 0, `serde_json::from_|to_` = 0 in src (12 in tests); wire shapes are camelCase + deny_unknown_fields with `#[serde(skip)]` on `zone` (metadata, not spec) and `provider_extension` (signed-envelope only), all pinned by tests/resource_type.rs and tests/audio_policy.rs round-trips; no hand-written deserializers, no try_from needed (post-parse validate gates are the deliberate admission pattern)

## obs
- clean: seeds `println!|eprintln!` = 0, `(info|debug|warn|error|trace)!\("` = 1 (the `use tracing::{debug, warn};` import at controller.rs:16, a seed false positive), `\.instrument\(|#\[instrument` = 0, `tracing::|log::` = 1; every event in controller.rs uses named fields (`zone`, `lease`, `channel`, `error`) with static messages, no interpolated messages, no secrets in fields, errors logged once at the mediation boundary

## docs
- d2b-provider-audio-pipewire#8 sev=medium blast=leaf effort=S verdict=actionable - `AudioStateLock` is a pub struct with no doc comment at all (its module carries `#[allow(missing_docs)]`, lib.rs:9-10), and it is non-obvious: a caller must know holding the value keeps the OFD lock and dropping it releases it - fix: document the guard semantics (or remove the module-level allow and document the item) - [src/state.rs:81-84, src/lib.rs:9-10]
  evidence: seed `^\s*pub (fn|struct|enum|trait|const|type)` = 85 hits; AudioStateLock (state.rs:81) is the only pub item with no doc comment
- d2b-provider-audio-pipewire#9 sev=medium blast=leaf effort=M verdict=actionable - none of the ~23 Result-returning public functions carries a `# Errors` section, and the failure conditions are non-obvious (LockOpen vs TempWrite vs AtomicRename on the state-I/O path; Admission vs Mediator on reconcile) - fix: add `# Errors` sections to `acquire/read/write_audio_state_*`, `child_resources`, `reconcile*`, `SpeakerMixer::set_grant/set_level`, `validate_audio_*` naming each failure variant - [src/state.rs:91, src/state.rs:148, src/state.rs:181, src/controller.rs:236, src/controller.rs:303, src/authority.rs:157]
  evidence: seed `/// # (Examples|Errors|Panics|Safety)` = 0 of 85 pub items; `-> Result<` = 23 hits, none documented with an Errors section
- d2b-provider-audio-pipewire#10 sev=low blast=leaf effort=S verdict=actionable - magic values lack the why: `AUDIO_REPAIR_INTERVAL_SECS = 300` says it is the repair interval but not why 300s, and the arbiter/mixer bound `64` in `AudioBindingController::new` is undocumented (and hardcoded again in tests/controller.rs:302-308) - fix: document the cadence rationale and hoist the 64 into a named const (e.g. `AUDIO_QUEUE_BOUND`) used by both the controller and the bound test - [src/controller.rs:21, src/controller.rs:209, src/controller.rs:212]
  evidence: static read; no doc text explains either constant's derivation

## perf
- clean: seeds `format!\(` = 1 src (state.rs:17 lock-path build, cold), `Vec::new\(\)|VecDeque::new\(\)|BTreeMap::new\(\)` = 3 src (empty-case constructors), `\.to_string\(\)` = 0 src; all hits are cold one-shot paths (state I/O, constructors), no allocation in any loop or reconcile hot path; static (unmeasured), no benchmark exists

## conc
- clean: seeds `std::thread::|thread::spawn|thread::scope` = 0, `\bMutex<|\bRwLock<` = 1, `Atomic\w+|Ordering::` = 3 (all `AtomicRename` error-variant false positives), `thread_local!|unsafe impl (Send|Sync) for` = 0; the single shared-state site (Arc<tokio::sync::Mutex> arbiter, authority.rs:44) is the justified multi-owner Service authority, reached only through non-blocking `try_lock` (U4 fail-closed), never held across awaits; no threads, no atomics, no manual Send/Sync claims

## async
- clean: seeds `async fn|async move|\.await` = 0, `tokio::spawn|spawn_blocking|JoinSet|select!\(|join!\(` = 0, `tokio::sync::(Mutex|RwLock|Notify)` = 2 (the arbiter alias, authority.rs:44,48), `#\[tokio::(main|test)\]|Runtime::block_on` = 0; the crate is fully synchronous; the tokio Mutex is deliberate (documented: no executor worker ever parks on it, authority.rs:40-44) and used only via try_lock; no guards across awaits, no spawned tasks, no cancellation surface

## unsafe
- N/A (seeds: 0/0/2/0; the 2 seed-3 hits are `io::Error::from_raw_os_error` at state.rs:49,67 - a safe std function, not an unsafe block; no `unsafe` blocks/fns/impls, no SAFETY comments needed, manifest `unsafe_code = "forbid"` at Cargo.toml:14)

## ffi
- N/A (seeds: `extern "C"|no_mangle|unsafe\(link_section` = 0, `catch_unwind` = 0, `repr\(C\)|repr\(transparent\)` = 0, `CStr|CString|c_char` = 0; no FFI surface - libc/nix calls in state.rs are safe syscall wrappers, not a foreign boundary)

## macro
- N/A (seeds: `macro_rules!` = 0, `proc_macro|syn::|quote!` = 0, `\$crate` = 0, `to_compile_error|new_spanned` = 0; no macros defined anywhere in the crate)

## test
- d2b-provider-audio-pipewire#11 sev=low blast=leaf effort=S verdict=actionable - `SpeakerMixer::mix_level`'s saturation cap (sum capped at 100, authority.rs:236-241) has no boundary test: tests/authority.rs:26-31 asserts only 80+20=100, so a regression that removed the `min(100)` would pass - fix: add saturation rows (e.g. 80+80, 60+60+60) asserting the capped result - [tests/authority.rs:26-31, src/authority.rs:236-241]
  evidence: seed `#\[test\]|#\[tokio::test\]` = 40 tests, assert mass = 174 seed hits; mix_level boundary rows absent from the only mixer test
- d2b-provider-audio-pipewire#12 sev=low blast=leaf effort=S verdict=actionable - tests/mediator.rs:13-24 is named `projection_cannot_open_pipewire_and_failed_set_preserves_state` but asserts only the Err and readiness; the state-preservation half of the claim is unasserted, so a regression that mutated grant/level on failure would pass - fix: assert `mediator.grant()`/`mediator.level()` unchanged after the failed set, or rename the test - [tests/mediator.rs:13-24]
  evidence: test body asserts two `assert_eq!)... Err)...))` and one readiness check; no grant/level state assertions

## supply
- d2b-provider-audio-pipewire#13 sev=low blast=leaf effort=S verdict=actionable - Cargo.toml declares `schemars` as a runtime dependency with zero uses in src or tests, and `serde_json` (tests-only, 12 hits) sits in `[dependencies]` instead of `[dev-dependencies]` - fix: drop the `schemars` entry and move `serde_json` to `[dev-dependencies]` (Cargo.toml:24-25) - [Cargo.toml:24, Cargo.toml:25]
  evidence: census `schemars` over src/ + tests/ = 0 hits (manifest only); `serde_json` over src/ = 0, tests/ = 12 hits; per-crate supply note, lens owned by lane X1

## Coverage
- idiom: clean (seeds: 0/1/0; single deliberate Default)
- own: clean (seeds: 9/7/1/0 over src+tests; all clones explainable)
- type: 2 finding(s)
- api: 3 finding(s)
- err: 2 finding(s)
- serde: clean (seeds: 4/6/0/0; wire shapes pinned)
- obs: clean (seeds: 0/1/0/1; named-field events only)
- docs: 3 finding(s)
- perf: clean (seeds: 1/3/0; cold paths only)
- conc: clean (seeds: 0/1/3/0; 3 false positives, 1 justified shared arbiter)
- async: clean (seeds: 0/0/2/0; synchronous crate, try_lock-only)
- unsafe: N/A (seeds: 0/0/2/0; both hits are from_raw_os_error safe calls; manifest forbid)
- ffi: N/A (seeds: 0/0/0/0)
- macro: N/A (seeds: 0/0/0/0)
- test: 2 finding(s)
- supply: 1 finding(s) (directly evidenced manifest note; lens owned by X1)