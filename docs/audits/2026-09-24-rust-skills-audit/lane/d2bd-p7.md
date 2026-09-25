# d2bd-p7 - d2bd - part 7/8
Baseline: 6ebdd4cec | LOC audited: 10662 (excl. src/generated/**) | modules: process_provider_runtime, provider_effects, effect_service_actors, main, guest_target_session, lib
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: src/process_provider_runtime.rs, src/provider_effects.rs, src/effect_service_actors.rs, src/main.rs, src/guest_target_session.rs, src/lib.rs

## idiom
- d2bd-p7#1 sev=low blast=leaf effort=S verdict=actionable - hand-written `impl Default` on both unit-struct actors (`EffectServiceActor`, `EffectServiceSupervisor`) delegates to `Self::new()` with zero call sites anywhere; a derive emits the same impl and cannot drift - fix: replace both with `#[derive(Default)]` (or delete both; no workspace caller) - [packages/d2bd/src/effect_service_actors.rs:268, packages/d2bd/src/effect_service_actors.rs:411]
  evidence: seed `impl (Default|From|PartialEq|Eq|Debug|Clone|Hash) for` = 2 hits (both Defaults); census: `EffectServiceActor::default|EffectServiceSupervisor::default` over packages/, nixos-modules/, tests/, docs/reference/, labs/, BUILD.bazel = 0 hits
- clean: seeds `for \w+ in 0\.\.` = 1 (test poll loop, esa:768), hand-written impls = 2, `let mut \w+ = (String|Vec)::new\(\)` = 1 (ppr:333, ordered required/optional field list with helper closures; pipeline not applicable); hand-written Debug impls at ppr:244/485/852 are deliberate redactions (ManagedResource/ControllerBootstrapContext/ProductionProcessProviders hide identities) - checked.

## own
- d2bd-p7#2 sev=low blast=leaf effort=S verdict=actionable - `match context.owner_uid.clone()` at ticket assembly clones the whole `Option<ResourceUid>` (a String-backed uid) on every launch, including the `None` arm and the guard-false `Some` arm where the value is never consumed - fix: match on `&context.owner_uid` and clone inside the arm (`Some(owner_uid) if ticket.owner_uid().is_none() => ticket.with_owner_uid(owner_uid.clone())`), so the `_ => ticket` path copies nothing - [packages/d2bd/src/process_provider_runtime.rs:4057]
  evidence: sampled: 50 of 276 `.clone()` hits (seeds 2-4: 178/1/0); remaining clones are struct construction from borrowed contexts, Arc clones at spawn boundaries, error-payload clones, and two bounded rollback snapshots (provider_effects.rs:973,1016, map capped by MAX_TRACKED_LIFECYCLE_MUTATIONS = 256)
- clean: `\.to_owned\(\)|\.to_vec\(\)|\.to_string\(\)` = 178 hits, `Rc<|RefCell<|Arc<Mutex<|Arc<RwLock<` = 1 (test recorder), `Cow<` = 0; to_owned sites are error-code strings and wire-rendering boundaries - all explainable.

## type
- clean: seeds `fn validate_\w+|fn check_\w+` = 6, `is_\w+: bool|\w+_flag: bool` = 0, `(mode|kind|state): String` = 0; the six validate/check functions (ppr:1012, ppr:3344, provider_effects:91, provider_effects:130, provider_effects:1291, main.rs:279) are policy/admission checks with caller-actionable failures, not shape validation a parsed type could absorb; stop rule applied, no flag-soup or stringly state in the part.

## api
- d2bd-p7#3 sev=medium blast=leaf effort=M verdict=actionable - `pub mod process_provider_runtime` and `pub mod provider_effects` are root-public, exposing the daemon's internal composition (83 pub items in the two modules, including `ProductionProcessProviders`, `FixedEffectAdapter`, `ProviderLifecycleDispatch`, and the pub `FixedEffectError`/`ProviderEffectError` enums) whose only external consumer is the crate's own `test-support`-gated integration test; the daemon binary reaches neither module - fix: declare both `pub(crate) mod` in composition.rs and keep a `#[cfg(feature = "test-support")]` re-export seam for `tests/resource_operator_activation.rs` (house pattern for test-support surface) - [packages/d2bd/src/composition.rs:398, packages/d2bd/src/composition.rs:400, packages/d2bd/src/provider_effects.rs:34, packages/d2bd/src/process_provider_runtime.rs:836]
  evidence: seed `\bpub (fn|struct|enum|trait|type|const|mod) ` = 83 hits in lane; census: `d2bd::(process_provider_runtime|provider_effects)` over packages/, nixos-modules/, tests/, docs/reference/, labs/ = 2 hits (1 file, test-support-gated); seed 2 = 1 hit (esa:100 `Arc<dyn EffectServiceFactory>` - shared factory across respawns, genuinely shared, kept)
- clean: `^\s*pub use ` = 0 hits; re-exports live in composition.rs (outside this partition); the effect-service surface (`EffectServiceRow`, `EffectServiceBinding`, `EffectServiceSupervisorMsg`) sits in a `pub(crate)` module; `ProviderLifecycleEffectPort` has a small required surface (1 required + 1 defaulted method) - checked.

## err
- d2bd-p7#4 sev=medium blast=leaf effort=S verdict=actionable - a durable service row that fails to (re)spawn is silently dropped: `let _ = state.spawn_service_actor)...)` in both the supervisor's restart-recovery loop and `supervise_exit` leaves a declared effect service unhosted with no trace, contradicting the module's own respawn promise (a crashed or killed service actor is respawned from its durable row; never leaves a service unhosted) - fix: log `tracing::warn!` with service/zone/error on spawn failure at both sites, keeping the non-fatal recovery semantics - [packages/d2bd/src/effect_service_actors.rs:551, packages/d2bd/src/effect_service_actors.rs:610]
  evidence: seed `let _ = |\.ok\(\);` = 57 hits; sites 551/610 judged per the per-site rule (the esa:564-570 oneshot `reply.send)...).ok()` sites are deliberate requester-gone ignores, kept); sibling pattern at ppr:804-809 shows the house rule is to warn on best-effort failures that matter
- d2bd-p7#5 sev=low blast=leaf effort=S verdict=actionable - the 0700 enforcement on a serving worker's socket parent is silently swallowed with `let _ =`; the sibling `create_dir_all` failure just above is a hard error, so a failed `set_permissions` leaves the launched socket dir at default umask perms with no diagnostic - fix: replace `let _ = tokio::fs::set_permissions)...)` with a `tracing::warn!` on Err, mirroring the pidfd snapshot warn at ppr:804-809 - [packages/d2bd/src/process_provider_runtime.rs:3436]
  evidence: seed `let _ = |\.ok\(\);` = 57 hits; site 3436 judged; house best-effort-warn pattern at ppr:804-809
- clean: seed `\.unwrap\(\)|\.expect\(` = 435 hits, of which 433 are inside cfg(test) or test-support constructors (exempt); the two production sites (ppr:4310, ppr:4326) are invariant expects the compiler cannot see (`[u8; 32]` hash prefix slicing and `ResourceUid::from_bytes`, which forces version/variant bits before parsing - parse cannot fail), acceptable per the panel policy; `panic!`/`unreachable!`/`todo!`/`unimplemented!` = 4, all in test modules; no error-taxonomy defect in the part's three error enums - checked.

## serde
- clean: seeds `derive\([^)]*(De)?[Ss]erialize` = 2, `serde\((rename_all|deny_unknown_fields|try_from|untagged|flatten|default|skip_serializing_if)` = 14, `impl .*Deserialize.*for` = 0, `serde_json::from_|serde_json::to_` = 49; the only serde types are `LifecycleMutationStatus` (lowercase) and `PersistedLifecycleMutation` (camelCase + `deny_unknown_fields` + `#[serde(default)]`/`alias` for rollback-compatible migration of legacy rows, provider_effects.rs:651-690) - the persisted schema choices are deliberate and documented; spec serialization is stable field-order `to_vec` for ticket digests - checked.

## obs
- clean: seeds `\bprintln!\(|\beprintln!\(` = 6 (all in main.rs: banner, error reporting, and the principal-allocation CLI diagnostic - product output per the carve-out), interpolated-message events `(info|debug|warn|error|trace)!\("` = 0, `\.instrument\(|#\[instrument` = 0, `tracing::` = 10; all tracing events carry named fields (vm, role, zone, error, mismatches, resource, identity); the binary installs the subscriber exactly once (main.rs:146-152) with EnvFilter; no secret material reaches any field (`identity` fields are hex digests; `ResourceUid` prints redacted by its own Display) - checked.

## docs
- d2bd-p7#6 sev=low blast=leaf effort=S verdict=actionable - the crate root carries no `//!` module doc: lib.rs opens with the lint attribute only, and the included composition.rs begins with a plain `//` comment, so the crate's large public surface (pub mods, dozens of pub use re-exports) renders without any module-level description - fix: add a `//!` crate doc in lib.rs naming the daemon composition facets and pointing at the daemon contract references - [packages/d2bd/src/lib.rs:1, packages/d2bd/src/composition.rs:1]
  evidence: static (no `//!` line in lib.rs:1-19 or composition.rs:1-40)
- d2bd-p7#7 sev=low blast=leaf effort=M verdict=actionable - no canonical `# Errors`/`# Panics` section exists anywhere in the part (0 hits) while `-> Result<` appears 121 times, including on the pub surface (`FixedEffectAdapter::validate_instance`, `dispatch`, `ProviderLifecycleDispatch::new_persistent`, `admit`, `EffectServiceBinding::call`/`call_expected`, `DaemonGuestTargetSession::request`); prose paragraphs describe the happy path but failure conditions are not structurally stated - fix: add `# Errors` sections naming refusal conditions to the pub Result-returning items of the two pub mods, keeping the existing prose - [packages/d2bd/src/provider_effects.rs:91, packages/d2bd/src/provider_effects.rs:711, packages/d2bd/src/provider_effects.rs:804, packages/d2bd/src/effect_service_actors.rs:201, packages/d2bd/src/guest_target_session.rs:37]
  evidence: seed `/// # (Examples|Errors|Panics|Safety)` = 0 hits; seed `-> Result<` = 121 hits
- clean: seed `^\s*pub (fn|struct|enum|trait|const|type)` = 83 hits; every top-level pub item in the part carries a doc comment with a first-sentence contract (spot-checked the full public surface, including the flagged-method set at ppr:1024-1064); `FIXED_PROCESS_PROVIDER_NAMES`, `MAX_TRACKED_LIFECYCLE_MUTATIONS`, `EffectServiceRow`, and both actor types are documented with their why - checked.

## perf
- d2bd-p7#8 sev=low blast=leaf effort=S verdict=actionable - `resource_identity_fields` builds a `Vec<IdentityField>` with exactly 12 statically-known pushes on every launch/adoption pass but grows from an empty `Vec::new()` - fix: `let mut fields = Vec::with_capacity(12);` (6 required + 6 optional entries) - [packages/d2bd/src/process_provider_runtime.rs:333]
  evidence: static (unmeasured); seed `Vec::new\(\)` = 56 hits, of which this is the one grow-by-push candidate with a fixed bound
- clean: seeds `format!\(` = 87, `Vec::new\(\)|VecDeque::new\(\)|HashMap::new\(\)|BTreeMap::new\(\)` = 56, `\.to_string\(\)` = 16; remaining format! sites are error strings, ticket-digest contexts, and launch-argv assembly (cold paths); no format! inside a loop, no attacker-keyed hashing, no grow-in-loop collections besides the finding - checked.

## conc
- d2bd-p7#9 sev=low blast=leaf effort=S verdict=actionable - two standalone monotonic counters use stronger orderings than the weakest correct one: the effect-service binding revision does `load(Ordering::SeqCst)` (esa:174) and `fetch_add(1, Ordering::SeqCst)` (esa:509), and `next_desired_generation` uses `fetch_update(Ordering::AcqRel, Ordering::Acquire, ...)` (provider_effects:1064); the revision is a version tag used only in equality staleness checks and the generation is a unique-value mint, so `Ordering::Relaxed` is correct for both - fix: switch the revision load/fetch_add and the generation fetch_update to `Ordering::Relaxed` - [packages/d2bd/src/effect_service_actors.rs:174, packages/d2bd/src/effect_service_actors.rs:509, packages/d2bd/src/provider_effects.rs:1064]
  evidence: seed `Atomic\w+|Ordering::` = 120 matching lines in lane (9 production sites examined; remaining mass is test-mod recorders); no unsafe Send/Sync impls, no thread_local, no std threads in the part (seed 4 = 0, seed 1 = 0)
- clean: seeds `std::thread::|thread::spawn|thread::scope` = 0, `\bMutex<|\bRwLock<` = 6 (all tokio::sync::Mutex state tables; `await_holding_lock`/`await_holding_refcell_ref` denied workspace-wide), `thread_local!|unsafe impl (Send|Sync)` = 0; the only shared state is the tokio Mutex tables and atomics above - checked.

## async
- clean: seeds `async fn|async move|\.await` = 231, `tokio::spawn|spawn_blocking|JoinSet|select!\(|join!\(` = 4 (actor spawns), `tokio::sync::(Mutex|RwLock|Notify)` = 5, `#\[tokio::(main|test)\]|Runtime::block_on` = 7; production state tables are `tokio::sync::Mutex` (ppr:18,937-940); the controller-bootstrap wait uses the sanctioned AsyncFd + `tokio::time::timeout` shape (ppr:95-100); the sync `LivenessProbe::probe` seat drives its future via `crate::drive_sync` (`block_in_place` + `handle.block_on`, inline `#[allow(clippy::disallowed_methods, reason = "synchronous path")]`, composition.rs:210-215) and is documented as the U13/R11 sync caller - no guard held across await, no blocking call on an executor worker, no cancellation-loss site found in the part; `EffectServiceBinding::send` awaits the reply oneshot without a deadline, but the production caller (forward_rendezvous) wraps dispatch in `tokio::time::timeout(handler_deadline, ...)` so a hung service surfaces as `forward-timeout`, and in-flight actor death closes the oneshot - checked.

## unsafe
- N/A (seeds: `\bunsafe \{|\bunsafe fn|\bunsafe impl|\bunsafe extern` = 0, `// SAFETY:` = 0, `transmute|from_raw|MaybeUninit|mem::zeroed` = 0, `unsafe_code` = 0; no unsafe blocks/fns/impls and no unsafe_code attribute in the six files; d2bd inherits workspace `unsafe_code = "forbid"`)

## ffi
- N/A (seeds: `extern "C"|no_mangle|unsafe\(link_section` = 0, `catch_unwind` = 0, `repr\(C\)|repr\(transparent\)` = 0, `CStr|CString|c_char` = 0; the part declares no foreign boundary)

## macro
- N/A (seeds: `macro_rules!` = 0, `proc_macro|syn::|quote!` = 0, `\$crate` = 0, `to_compile_error|new_spanned` = 0; no macro definitions or proc-macro surface in the part)

## test
- clean: seeds `#\[test\]|#\[tokio::test\]` = 66, `assert_eq!\(|assert_ne!\(|assert!\(` = 255, `proptest!|insta::assert|rstest` = 0, `#\[ignore\]` = 0 (scope: in-file tests in the six assigned files; `tests/**` integration targets are shared crate-wide and outside this partition); the in-file suites assert behavior - dedup counting (provider_effects:1783-1791), TTL and persist-failure release, restart/migration determinism with hand-worked expectations, supervision respawn with revision bumps, GPU/TPM argv pinning - with no same-logic expected values or Display-string error asserts found in the sampled assertions, and the only poll helper is bounded (200 iterations, esa:768) - checked.

## Coverage
- idiom: 1 finding(s)
- own: 1 finding(s)
- type: clean (seeds ran: 6/0/0; admission/policy checks with caller-actionable failures, no parse-once candidate)
- api: 1 finding(s)
- err: 2 finding(s)
- serde: clean (seeds ran: 2/14/0/49; deliberate migration-aware persisted schema)
- obs: clean (seeds ran: 6/0/0/10; named-field events, CLI output carve-out)
- docs: 2 finding(s)
- perf: 1 finding(s)
- conc: 1 finding(s)
- async: clean (seeds ran: 231/4/5/7; sanctioned sync seats, tokio state tables, deadline at the rendezvous caller)
- unsafe: N/A (seeds: 0/0/0/0 all zero; no unsafe constructs in the part)
- ffi: N/A (seeds: 0/0/0/0 all zero; no foreign boundary)
- macro: N/A (seeds: 0/0/0/0 all zero; no macros)
- test: clean (seeds ran: 66/255/0/0; behavior-focused in-file suites, no non-failable assertions found)