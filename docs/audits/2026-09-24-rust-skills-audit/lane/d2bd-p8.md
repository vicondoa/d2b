# d2bd-p8 - d2bd - part 8/8
Baseline: 6ebdd4cec | LOC audited: 10828 (excl. src/generated/**) | modules: forward_rendezvous, shared_provider_effects, provider_registry, audio_dispatch
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: file-split part (forward_rendezvous.rs, shared_provider_effects.rs, provider_registry.rs, audio_dispatch.rs)

## idiom
- d2bd-p8#1 sev=low blast=leaf effort=S verdict=actionable - no-op `let _ =` suppression statements with dead bindings: `let _ = &mut chain;` after the chain re-root (no mutation follows), `let _ = kind;` masking the unused `kind` param of `network_content_fence`, and `let _ = error.code();` masking the unused `error` in the `Refused` arm - fix: delete the statements and bind the now-unused pattern args as `_` / drop the `kind` param - [packages/d2bd/src/forward_rendezvous.rs:672, packages/d2bd/src/shared_provider_effects.rs:1156, packages/d2bd/src/provider_registry.rs:531]
  evidence: seed `let _ = |\.ok\(\);` = 9 hits (3 are these no-ops; the rest are deliberate ignores of `OnceLock::set` results and test channel sends); production parts of all four files read in full
- d2bd-p8#2 sev=medium blast=leaf effort=S verdict=actionable - `reconcile_security_key` (SecurityKeyComponent::Service) acquires the Zone runtime with `let runtime = self.runtime()?;` that no branch uses; `let _ = runtime;` masks it, and `runtime()` (a try_lock spin, see d2bd-p8#17) returns `Unavailable` when the plane is absent, so a Service reconcile that never reads the plane fails spuriously - fix: delete the `let runtime = ...` and `let _ = runtime;` lines - [packages/d2bd/src/shared_provider_effects.rs:1903, packages/d2bd/src/shared_provider_effects.rs:1997]
  evidence: seed `let _ = |\.ok\(\);` = 9 hits (1997 is a no-op masking an unused value); read of the Service branch body confirms `runtime` is used in no path
- d2bd-p8#3 sev=low blast=leaf effort=S verdict=actionable - redundant let-else plus a provably-dead second match and `unreachable!` in `ProviderRuntime::dispatch_lifecycle`: the `else` of `let ProviderRuntimeState::Active(active) = ...` re-matches `&*state` and its `Active(_) => unreachable!)...)` arm can never fire - fix: collapse the else to `return Err(ProviderEffectError::RegistryUnavailable)` - [packages/d2bd/src/provider_registry.rs:527-536]
  evidence: seed `\bpanic!\(|\bunreachable!\(|\btodo!\(|\bunimplemented!\(` = 24 hits (this is the only production `unreachable!`; the rest are test fixtures)
- d2bd-p8#4 sev=low blast=leaf effort=M verdict=actionable - statement-style `Vec::new()` + push loops that are iterator shapes: `deploy_target_local_controllers` filters on component type / instance scope / target kind then pushes, and `dispatch_audio_status` partitions `Result<AudioVmState, AudioVmError>` into `entries` and `errors` - fix: `manifest.components().iter().filter)...).map)...).collect::<Result<Vec<_>, _>>()?` and `vm_names.iter().map)...).partition(Result::is_ok)` - [packages/d2bd/src/provider_registry.rs:359-390, packages/d2bd/src/audio_dispatch.rs:392-411]
  evidence: seed `for \w+ in 0\.\.` = 3, seed `let mut \w+ = (String|Vec)::new\(\)` = 2

## own
- d2bd-p8#5 sev=low blast=leaf effort=S verdict=actionable - avoidable `let zone = request.zone.clone();` in `ForwardRendezvous::invoke`: `zone` is used only as the `BTreeMap::get` key inside the `self.zones.lock().await` block, whose scope does not outlive the `request` borrow, so `zones.get(&request.zone)` compiles without the clone - fix: drop the clone and borrow `&request.zone` - [packages/d2bd/src/forward_rendezvous.rs:456, packages/d2bd/src/forward_rendezvous.rs:458-466]
  evidence: seed `\.clone\(\)` = 217 hits (sampled: 50 of 217; production parts read in full, this is the one avoidable production clone found); read confirms `zone` is unused after the lock block
- d2bd-p8#6 sev=low blast=leaf effort=S verdict=actionable - `validate_network_config_volume_spec` clones the whole JSON `spec` document (`let mut base = spec.clone();`) just to strip three fields and re-parse as `VolumeSpec`; on the `upsert_volume_content` path the document is cloned again at the caller, so the same wire document is cloned and re-parsed twice per reconcile/readiness check - fix: take the spec by ownership once at the boundary and parse to `VolumeSpec` directly (drop the clone by passing the already-owned `Value`) - [packages/d2bd/src/shared_provider_effects.rs:690, packages/d2bd/src/shared_provider_effects.rs:854-858, packages/d2bd/src/shared_provider_effects.rs:891-895]
  evidence: seed `\.clone\(\)` = 217 (shared_provider_effects.rs = 109 hits; production part read in full); callers of `validate_network_config_volume_spec` read at 850-897

## type
- d2bd-p8#7 sev=medium blast=leaf effort=S verdict=actionable - stringly-typed wire mode compared to string literals: `request.spec.pointer("/mode").and_then(Value::as_str) == Some("authority")` (USBIP service) and `mode == "projection"` (security-key service); an unknown or misspelled mode silently takes the non-authority / non-projection branch, flipping the admission posture without an error - fix: parse the mode once into a typed enum (`#[derive(Deserialize, PartialEq)]` with `rename_all = "kebab-case"`) at the effect boundary and refuse unknown values (fail closed) - [packages/d2bd/src/shared_provider_effects.rs:1299, packages/d2bd/src/shared_provider_effects.rs:1903-1908]
  evidence: seed `fn validate_\w+|fn check_\w+` = 2; the mode state is reached via `/mode` JSON pointers (the direct-field spelling `(mode|kind|state): String` = 0 in this lane); both comparison sites read in full

## api
- d2bd-p8#8 sev=low blast=leaf effort=S verdict=actionable - `pub use d2b_provider::{MAX_PROVIDER_REGISTRY_ENTRIES, ProviderRegistrySnapshot};` re-exports `ProviderRegistrySnapshot`, which nothing in d2bd uses; only `MAX_PROVIDER_REGISTRY_ENTRIES` is consumed (registry bound check) - fix: re-export `MAX_PROVIDER_REGISTRY_ENTRIES` only, removing the second path to `ProviderRegistrySnapshot` - [packages/d2bd/src/provider_registry.rs:48, packages/d2bd/src/provider_registry.rs:288]
  evidence: census `ProviderRegistrySnapshot` over `packages/`, `nixos-modules/`, `tests/`, `docs/reference/`, `labs/`, `BUILD.bazel` = 6 hits, all in `d2b-provider` and this re-export itself; no consumer of the `d2bd::provider_registry::ProviderRegistrySnapshot` path

## err
- d2bd-p8#9 sev=medium blast=wide effort=M verdict=needs-contract - user-input audio failures are flattened into `TypedError::InternalIo { context, detail }` strings on the mutation paths (VM absent, audio not enabled) in `dispatch_audio_set_volume` / `dispatch_audio_mute`, while the status path reports the same classes as structured `AudioVmError` + `AudioErrorKind::VmNotFound` / `AudioNotEnabled`; a caller of set-volume/mute cannot distinguish VM-not-found from an internal I/O failure except by string-matching the detail - fix: map the mutation paths onto the same structured kinds (extend `TypedError` with the audio kinds used by both paths); this changes the daemon-API wire error surface, so it is needs-contract - [packages/d2bd/src/audio_dispatch.rs:487-495, packages/d2bd/src/audio_dispatch.rs:594-602, packages/d2bd/src/audio_dispatch.rs:417-438]
  evidence: seed `\.unwrap\(\)|\.expect\(` = 269; read of both error paths; `TypedError` is the daemon-API wire error type (typed_error.rs:460+ `kind()`/`message()`)

## serde
- d2bd-p8#10 sev=low blast=leaf effort=S verdict=actionable - `declared_fd_kind` allocates a `serde_json::Value::String(kind.to_owned())` heap value just to deserialize the wire `FdKind` enum (the kebab-case `serde` mapping) - fix: use `serde_json::from_str::<FdKind>(kind)` (no intermediate `Value`) or a plain `match` over the kebab-case spellings - [packages/d2bd/src/forward_rendezvous.rs:979-981]
  evidence: seed `serde_json::from_|serde_json::to_` = 41 hits; site read in full

## obs
- d2bd-p8#11 sev=low blast=leaf effort=S verdict=actionable - message-only `tracing::warn!("forward rendezvous is at its in-flight cap; refusing the call")` carries no fields and sits in a loop with no enclosing span, so the cap refusal cannot be attributed to a caller or the cap value - fix: add a field (`peer_uid`, `max = posture.max_inflight`) - [packages/d2bd/src/forward_rendezvous.rs:1250]
  evidence: seed `(info|debug|warn|error|trace)!\("` = 1 hit (the only message-only event); `\bprintln!\(|\beprintln!\(` = 1 hit, a test `eprintln!` at forward_rendezvous.rs:4973 (out of scope)

## docs
- d2bd-p8#12 sev=medium blast=leaf effort=S verdict=actionable - `pub fn dispatch_audio` is the only `pub` item in the lane without a doc comment; it is the daemon's audio dispatch entry with three op arms and non-obvious error behavior - fix: add a doc comment covering the op arms, the capability resolution, and the `TypedError` error surface - [packages/d2bd/src/audio_dispatch.rs:372]
  evidence: seed `^\s*pub (fn|struct|enum|trait|const|type)` = 30 pub items over the four files; read of the pub-item list confirms every other one carries a doc contract
- d2bd-p8#13 sev=low blast=leaf effort=S verdict=actionable - doc-comment shape drift in forward_rendezvous.rs first sentences: double trailing periods and missing spacing (`The received descriptors,borrowed across the invocation..`, `attached:count equal`, `...kind the carrier vocabulary does not carry..`, `awaited for readiness..`) - fix: normalize punctuation/spacing in the affected comments - [packages/d2bd/src/forward_rendezvous.rs:1046-1049, packages/d2bd/src/forward_rendezvous.rs:1070, packages/d2bd/src/forward_rendezvous.rs:1093, packages/d2bd/src/forward_rendezvous.rs:1431-1432]
  evidence: read of the doc comments at forward_rendezvous.rs:1040-1095, 1421-1432; `-> Result<` seed = 136 hits (all items with Result return either carry `# Errors`-style prose or are `pub(crate)` with documented contracts)

## perf
- d2bd-p8#14 sev=low blast=leaf effort=M verdict=actionable - `AsyncSeqpacket::read_frame` allocates a fresh `vec![0u8; MAX_FRAME_SIZE + 5]` (1 MiB) per read, and `drain_pending` performs up to four such reads per refused call; the frame is length-prefixed, so the read buffer can be sized from the 4-byte prefix (or drained onto a reused buffer) instead of the full ceiling - fix: read the prefix, then allocate `declared + 5` - [packages/d2bd/src/forward_rendezvous.rs:1356, packages/d2bd/src/forward_rendezvous.rs:1476-1481]
  evidence: seed `Vec::new\(\)|VecDeque::new\(\)|HashMap::new\(\)|BTreeMap::new\(\)` = 57; `MAX_FRAME_SIZE = 1024 * 1024` (d2b-contracts/src/lib.rs:63); static (unmeasured)
- d2bd-p8#15 sev=low blast=leaf effort=S verdict=actionable - grow-by-push vectors with known upper bounds: `guest_uids = Vec::new()` (bound `spec.attachments().len()`) and `entries`/`errors = Vec::new()` (bound `vm_names.len()`) - fix: `Vec::with_capacity(<bound>)` - [packages/d2bd/src/shared_provider_effects.rs:1014-1016, packages/d2bd/src/audio_dispatch.rs:392-396]
  evidence: seed `Vec::new\(\)|VecDeque::new\(\)|HashMap::new\(\)|BTreeMap::new\(\)` = 57 hits; both sites read in full; static (unmeasured)

## conc
- d2bd-p8#16 sev=low blast=leaf effort=S verdict=actionable - `Ordering::SeqCst` on the standalone `broker_epoch` atomic (store and load). The epoch is a self-contained value; the zones map it gates is mutex-guarded, so there is no paired publication needing Acquire/Release - `Ordering::Relaxed` is the weakest correct ordering here - fix: use `Ordering::Relaxed` at both sites - [packages/d2bd/src/forward_rendezvous.rs:332, packages/d2bd/src/forward_rendezvous.rs:414]
  evidence: seed `Atomic\w+|Ordering::` = 13 hits; the two SeqCst sites and their ordering argument read in full (production atomics are otherwise Relaxed counters, and test atomics use Acquire/Release pairs for explicit handoff)

## async
- d2bd-p8#17 sev=medium blast=leaf effort=M verdict=actionable - `ProductionSharedProviderEffects::runtime()` and `NetworkRuntime::bundle()` busy-wait with `std::hint::spin_loop()` on `tokio::sync::Mutex::try_lock()`; `runtime()` is called from async reconcilers (reconcile_network, reconcile_usbip, reconcile_tpm, ...), so a contended lock spins an executor worker instead of awaiting. The `// async-gate-allow` markers in this file cover the `.lock()` sites (recorded in async-gate-inventory.json:1165-1198) but these `try_lock`+spin sites are not marked or recorded, and the gate scanner matches `.lock()`/`.read()`/`.write()` only, so they are invisible to it. The in-code comment cites plan U10 / the broker rate limiter as the choice - fix: use `.lock().await` where the caller is async (split a sync lock path for the sync trait callers), or record these sites in the async-gate inventory as a deliberate exception - [packages/d2bd/src/shared_provider_effects.rs:316-324, packages/d2bd/src/shared_provider_effects.rs:2633-2639]
  evidence: seed `async fn|async move|\.await` = 452, seed `tokio::sync::(Mutex|RwLock|Notify)` = 24; read of runtime()/bundle() and their callers; async-gate-inventory.json:1165-1198 covers the `.lock()` sites only

## unsafe
- N/A: seeds `\bunsafe \{|\bunsafe fn|\bunsafe impl|\bunsafe extern` = 0, `// SAFETY:` = 0, `transmute|from_raw|MaybeUninit|mem::zeroed` = 4 (all safe `io::Error::from_raw_os_error` constructors, not unsafe `from_raw` calls), `unsafe_code` = 0 - no unsafe blocks/fns/impls and no unsafe_code settings in the lane

## ffi
- N/A: seeds `extern "C"|no_mangle|unsafe\(link_section` = 0, `catch_unwind` = 0, `repr\(C\)|repr\(transparent\)` = 0, `CStr|CString|c_char` = 0 - no FFI boundary in the lane

## macro
- N/A: seeds `macro_rules!` = 0, `proc_macro|syn::|quote!` = 0, `\$crate` = 0, `to_compile_error|new_spanned` = 0 - no macro definitions or proc-macro machinery in the lane

## test
- clean: seeds `#\[test\]|#\[tokio::test\]` = 21, `assert_eq!\(|assert_ne!\(|assert!\(` = 180, `proptest!|insta::assert|rstest` = 0, `#\[ignore\]` = 0; checked the unit and `#[tokio::test(flavor = "multi_thread")]` suites in all four files - the rendezvous suite drives real seqpacket sockets across stalls, handler crashes, deadlines, in-flight caps, fd legs, attestation freshness/epoch invalidation, effect-service respawn and chain-recording, with multi_thread flavor on timing paths and generous bounds; the registry/audio suites assert behavior and error variants (never Display strings), and no test is unable to fail; no ignored or property tests exist (absence noted, not a finding)

## Coverage
- idiom: 4 finding(s)
- own: 2 finding(s)
- type: 1 finding(s)
- api: 1 finding(s)
- err: 1 finding(s)
- serde: 1 finding(s)
- obs: 1 finding(s)
- docs: 2 finding(s)
- perf: 2 finding(s)
- conc: 1 finding(s)
- async: 1 finding(s)
- unsafe: N/A (seeds: 0/0/4/0 - the 4 `from_raw` hits are safe `from_raw_os_error` constructors; no unsafe blocks/fns/impls or unsafe_code settings)
- ffi: N/A (seeds: 0/0/0/0 all zero; no FFI boundary)
- macro: N/A (seeds: 0/0/0/0 all zero; no macros)
- test: clean (seeds ran: 21/180/0/0; no findings)
