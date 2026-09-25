# d2b-bus-p2 - d2b-bus - part 2/2
Baseline: 6ebdd4cec22e6537d1376e83ff7a82b00a8492c0 | LOC audited: 10327 (excl. src/generated/**) | modules: session/ (contract, enrollment, mod, noise_vectors, prologue, zone_link), session_seam_tests, streams, operations, wire, lib
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: part 2/2: src/session/**, src/session_seam_tests.rs, src/streams.rs, src/operations.rs, src/wire.rs, src/lib.rs

## idiom
- d2b-bus-p2#1 sev=low blast=leaf effort=S verdict=actionable - `PendingCancelDeliveries::abort_destination` collects into a `Vec` inside a `retain` closure (statement-style accumulation with a side effect in the predicate) instead of partitioning the entries - fix: `let (aborted, kept): (Vec<_>, Vec<_>) = entries.drain(..).partition(|entry| entry.destination == session); *entries = kept;` and abort the drained handles - [packages/d2b-bus/src/operations.rs:302-310]
  evidence: seed `let mut \w+ = (String|Vec)::new\(\)` = 3 hits; the other two hits are test task vectors (streams.rs:1221, 1283) where a collect would obscure the spawn loop, and the hand-written `impl Default for StreamLimits` (streams.rs:77) and `impl PartialEq/Eq for OperationAttempt` (operations.rs:77-83) are deliberate (nonzero defaults; identity semantics) and are not findings
- clean: seeds `for \w+ in 0\.\.` = 10 hits, `impl (Default|From|PartialEq|Eq|Debug|Clone|Hash) for` = 3 hits, `let mut \w+ = (String|Vec)::new\(\)` = 3 hits; the index loops are bounded retry loops over queues being mutated (streams.rs:368-402) or test spawn loops, so plain loops are the right shape; one finding above

## own
- d2b-bus-p2#2 sev=low blast=leaf effort=S verdict=actionable - `SubjectContextDigest::of_subject` builds six owned `String`s (four `to_owned()` on `&str`/`&'static str` fields plus two `to_canonical_string()` calls) only to hash length-prefixed bytes - fix: iterate `&[&str]` slices (the label helpers already return `&'static str`, and the subject/service/purpose accessors expose `&str`) and feed `len()` and `as_bytes()` directly, dropping all six allocations per digest - [packages/d2b-bus/src/session/prologue.rs:72-78]
  evidence: seed `\.to_owned\(\)|\.to_vec\(\)|\.to_string\(\)` = 26 hits; the four avoidable `to_owned()` calls are at prologue.rs:74-77; the remaining hits are test fixtures and wire-rendering boundaries
- d2b-bus-p2#3 sev=low blast=leaf effort=M verdict=actionable - `VerifiedRouteAdmission::revalidate` clones the whole admission body (including the session binding) on every call, and `ZoneLinkSession::admit`/`is_open` invoke it on every forwarded operation - fix: add a by-reference verification path (a `verify_body(&self, body: &RouteAdmissionBody)` helper or a `revalidate` that digests `&self.body` without rebuilding owned evidence) so the re-check allocates nothing - [packages/d2b-bus/src/session/contract.rs:1046-1056, packages/d2b-bus/src/session/zone_link.rs:147]
  evidence: seed `\.clone\(\)` = 132 hits; census: `\.revalidate\(\)` over packages/, nixos-modules/, tests/, docs/reference/, labs/ = 5 sites (contract.rs:1046 definition plus zone_link.rs:147, 218, 256, 358); the clone at contract.rs:1054 exists only to feed the consuming `verify` signature
- clean: seeds `Rc<|RefCell<|Arc<Mutex<|Arc<RwLock<` = 0, `Cow<` = 0; the remaining clone mass is map-key ownership (streams.rs:204-229, operations.rs:445), `Arc` sharing at handle boundaries (streams.rs:252, operations.rs:490), and test fixtures, all explainable

## type
- d2b-bus-p2#4 sev=medium blast=leaf effort=S verdict=actionable - `ZoneLinkSession` carries `admission: Option<VerifiedRouteAdmission>` and `liveness: Option<SessionLiveness>` that are always both `Some` or both `None` (the two constructors set them in lockstep), so a half-set lane is representable and would silently skip admission revalidation - fix: fold the pair into one `established: Option<EstablishedLane>` holding both values (the test lane stays the `None` case), making the impossible combination unconstructible - [packages/d2b-bus/src/session/zone_link.rs:111-117]
  evidence: type seeds (`fn validate_\w+|fn check_\w+`, `is_\w+: bool|\w+_flag: bool`, `(mode|kind|state): String`) = 0/0/0 hits; lens applicable because the part declares structs and enums; the lockstep invariant is read from the only two constructors at zone_link.rs:141-195
- clean: no boolean-flag soup or stringly-typed state found; the enrollment machine already models its five states as an enum with checked transitions, and the `revoked` marker with a persisted record is a documented crash-window state (enrollment.rs:392-396), not a flag finding

## api
- d2b-bus-p2#5 sev=medium blast=leaf effort=S verdict=actionable - two public types named `Cancellation` are reachable from the crate root: `d2b_bus::Cancellation` (operations) and `d2b_bus::session::Cancellation` (the re-exported `d2b_session::Cancellation`), so a caller importing both modules gets a name collision and can hand the wrong token to a handler - fix: drop `Cancellation` from the `d2b_session` re-export block in session/mod.rs (the bus's own token shadows the need) or rename one of the two - [packages/d2b-bus/src/lib.rs:34, packages/d2b-bus/src/session/mod.rs:89-97]
  evidence: seed `^\s*pub use ` = 15 hits; census: `d2b_bus::session::Cancellation` over packages/, nixos-modules/, tests/, docs/reference/, labs/ = 0 qualified uses today, but the in-crate distinction is already live at router.rs:2663-2664 where `Cancellation` and `d2b_session::Cancellation` sit in one struct
- clean: no `Arc`/`Rc`/`Box`/`RefCell` in a public signature beyond the deliberate `Arc<dyn BusObserver>`/`Arc<dyn BusTelemetry>` observer injection (streams.rs:149-150) and the `Arc`-shared `ZoneLinkSession` driver owner, both with private fields; the lib.rs re-export arms are the house single-surface pattern

## err
- d2b-bus-p2#6 sev=medium blast=leaf effort=S verdict=actionable - public `ZoneBoundPolicyIdentity::with_provider` returns `Result<Self, &'static str>`, a string a caller must string-match instead of matching on a variant - fix: return a closed error type (reuse `ZonePolicyError` with a new `NotProviderRef` variant, or a small `ZoneBoundPolicyIdentityError` enum) for the single failure condition - [packages/d2b-bus/src/wire.rs:49-56]
  evidence: seed `-> Result<` = 56 hits; census: `ZoneBoundPolicyIdentity::with_provider` over packages/, nixos-modules/, tests/, docs/reference/, labs/ = 2 hits, both in wire.rs tests (wire.rs:171, 179), so the fix carries no external call-site churn
- clean: seeds `\.unwrap\(\)|\.expect\(` = 827 hits (the overwhelming majority inside `#[cfg(test)]` modules and test fixtures, which the card exempts), `let _ = ` = 7 hits (the one in production is the deliberate fence compare_exchange at zone_link.rs:232, documented as keeping the stronger fence), `\bpanic!\(|\bunreachable!\(|\btodo!\(|\bunimplemented!\(` = 31 hits (test assertions and the `unimplemented` test-driver helper), `enum \w*Error` = 6 hits; production `expect`s are internal-invariant assertions with named reasons (streams.rs:372-408, operations.rs:487) and every std Mutex poison is recovered via `into_inner()`; one finding above

## serde
- N/A: seeds `derive\([^)]*(De)?[Ss]erialize`, `serde\((rename_all|deny_unknown_fields|try_from|untagged|flatten|default|skip_serializing_if)`, `impl .*Deserialize.*for`, `serde_json::from_|serde_json::to_` = 0/0/0/0 hits over the part; the crate's serde_json dependency is consumed in part 1 (router.rs), so this part crosses no serde boundary

## obs
- clean: seeds `\bprintln!\(|\beprintln!\(` = 0, `(info|debug|warn|error|trace)!\("` = 0, `\.instrument\(|#\[instrument` = 0, `tracing::|log::` = 2 (both false positives: `ApiCatalog::standard()` in session_seam_tests.rs:582 and 1289 matches `log::` inside the word "catalog"); the part emits no telemetry at all and the crate declares no tracing/log dependency, so there is nothing to judge beyond absence

## docs
- d2b-bus-p2#7 sev=medium blast=leaf effort=S verdict=actionable - `pub struct Cancellation` (re-exported at the crate root) has no doc comment while every sibling public item does, leaving the opaque token's contract (crate-private construction, one-attempt observation, `is_cancelled`) undocumented - fix: add a `///` doc comment stating the token is minted only by the bus and observes one operation attempt - [packages/d2b-bus/src/operations.rs:124-125]
  evidence: docs seeds: `^\s*pub (fn|struct|enum|trait|const|type)` = 137 hits, `/// # (Examples|Errors|Panics|Safety)` = 0; the struct at operations.rs:125 is the only root-re-exported item without a doc comment
- d2b-bus-p2#8 sev=low blast=leaf effort=M verdict=actionable - public `Result`-returning constructors and accessors (`StreamName::parse`, `OperationId::parse`, `ZoneBoundPolicyIdentity::digest`, `ZoneEndpointPolicy::lower`) carry no `# Errors` section naming which condition produces which failure, even though the failure conditions are closed and enumerated in the error enums - fix: add `# Errors` sections to the public parse/lower/digest items - [packages/d2b-bus/src/streams.rs:39-40, packages/d2b-bus/src/operations.rs:24-25, packages/d2b-bus/src/wire.rs:79-82, packages/d2b-bus/src/session/contract.rs:164-165]
  evidence: docs seeds: `-> Result<` = 56 hits, `/// # (Examples|Errors|Panics|Safety)` = 0; the repo style is one-line prose docs without canonical sections, so this is a consistency proposal rather than a coverage gap
- clean: module docs are present and substantive (session/mod.rs, prologue.rs, contract.rs, enrollment.rs, zone_link.rs), the redacted `Debug` impls are deliberate and tested, and the compile_fail doctests at contract.rs:312-315 and 760-763 run under `cargo test --doc`

## perf
- d2b-bus-p2#9 sev=low blast=leaf effort=M verdict=actionable - `OutgoingStream::send_wait` clones the whole payload on every backpressure wakeup because `StreamBridge::send` consumes the `Vec` and drops it on rejection, so a frame up to `max_frame_bytes` (64 KiB) is re-allocated per retry on the bounded-watch delivery path - fix: have `send` return the rejected payload (for example `Result<(), (StreamError, Vec<u8>)>`) or split an admit-check from the enqueue so the loop moves the buffer instead of cloning - [packages/d2b-bus/src/streams.rs:642-658]
  evidence: static (unmeasured); seed `\.clone\(\)` = 132 hits; census: `send_wait|send_and_wait_ack` over packages/, nixos-modules/, tests/, docs/reference/, labs/ = 4 hits, with the production caller at router.rs:4188
- clean: seeds `format!\(` = 32 hits (all in error paths, digest construction, and test fixtures, which the card exempts), `Vec::new\(\)|VecDeque::new\(\)|HashMap::new\(\)|BTreeMap::new\(\)` = 31 hits (mostly empty-case collection construction where the empty case is common, plus test fixtures), `\.to_string\(\)` = 1 hit (a test assertion); the BTreeMap choices are for deterministic iteration, and `direction_gauges` already uses saturating accumulation

## conc
- clean: seeds `std::thread::|thread::spawn|thread::scope` = 0, `\bMutex<|\bRwLock<` = 10, `Atomic\w+|Ordering::` = 23, `thread_local!|unsafe impl (Send|Sync) for` = 0; every Mutex and atomic use is a brief non-suspending critical section with a sanctioned `synchronous path` allow (contract.rs:1009-1054, operations.rs:295, streams.rs:562) or a cfg(test) helper, poison is recovered via `into_inner()` rather than `unwrap`, and the fence/attempt/cancellation atomics use correct Acquire/Release pairs with written ordering arguments

## async
- clean: seeds `async fn|async move|\.await` = 346, `tokio::spawn|spawn_blocking|JoinSet|select!\(|join!\(` = 29, `tokio::sync::(Mutex|RwLock|Notify)` = 2 (the two `use tokio::sync::Notify;` imports), `#\[tokio::(main|test)\]|Runtime::block_on` = 0; the Notify waiters are created before the condition check with the future pinned and `enable()`d (streams.rs:642-676, operations.rs:150-158), which is the correct tokio pattern, no guard is held across an `.await`, and all std-Mutex touches are brief critical sections with sanctioned allows

## unsafe
- N/A: seeds `\bunsafe \{|\bunsafe fn|\bunsafe impl|\bunsafe extern`, `// SAFETY:`, `transmute|from_raw|MaybeUninit|mem::zeroed` = 0/0/0 hits; seed 4 alone (`unsafe_code = "forbid"` in the manifest) does not make the lens applicable

## ffi
- N/A: seeds `extern "C"|no_mangle|unsafe\(link_section`, `catch_unwind`, `repr\(C\)|repr\(transparent\)`, `CStr|CString|c_char` = 0/0/0/0 hits; the part crosses no foreign-language boundary

## macro
- N/A: seeds `macro_rules!`, `proc_macro|syn::|quote!`, `\$crate`, `to_compile_error|new_spanned` = 0/0/0/0 hits over the part (the crate's single macro hit is in part 1); no macro definitions or proc-macro surface

## test
- d2b-bus-p2#10 sev=medium blast=leaf effort=M verdict=actionable - session_seam_tests.rs waits for service readiness with fixed-count yield and poll loops (`for _ in 0..16 { tokio::task::yield_now().await }` at 1623 and 1808, `for attempt in 0..32` at 1882, `for attempt in 0..8` plus an inner yield loop at 1941-1960), which is machine-dependent and can fail spuriously on a loaded runner - fix: replace with condition-driven waits (oneshot or Notify), the deterministic pattern the same file already uses elsewhere (advance_virtual, dispatched_wait) - [packages/d2b-bus/src/session_seam_tests.rs:1623-1625, packages/d2b-bus/src/session_seam_tests.rs:1808-1810, packages/d2b-bus/src/session_seam_tests.rs:1882-1884, packages/d2b-bus/src/session_seam_tests.rs:1941-1960]
  evidence: seed `for \w+ in 0\.\.` = 10 hits; the four readiness loops are the only machine-dependent waits in the part, and the file's own comment blocks document the deterministic counterpart pattern
- d2b-bus-p2#11 sev=low blast=leaf effort=S verdict=actionable - `cancel_retry_cannot_reach_a_same_id_replacement_while_tombstone_is_retained` pins the full `Display` sentence of `OperationError::RetainedOperationId`, so a wording change fails the test even though the contract is the variant and its `as_str()` label - fix: assert the variant (the surrounding code already matches on variants) and drop the `to_string()` equality - [packages/d2b-bus/src/operations.rs:1057-1060]
  evidence: seed `assert_eq!\(|assert_ne!\(|assert!\(` = 328 hits; this is the only assertion in the part that pins a `Display` string rather than a variant or label
- clean: seeds `#\[test\]|#\[tokio::test\]` = 113, `assert_eq!\(|assert_ne!\(|assert!\(` = 328, `proptest!|insta::assert|rstest` = 0, `#\[ignore\]` = 0; the suite is strong overall - frozen Noise golden vectors (noise_vectors.rs), deterministic concurrency tests with Barriers and start_paused virtual time (streams.rs:984-1062), error-variant assertions throughout, and the child-process peer test is a documented subprocess pattern; two findings above

## Coverage
- idiom: 1 finding (seeds 10/3/3)
- own: 2 findings (seeds 132/26/0/0)
- type: 1 finding (seeds 0/0/0; applicable via struct/enum presence)
- api: 1 finding (seeds 137/0/15)
- err: 1 finding (seeds 827/7/31/6)
- serde: N/A (seeds 0/0/0/0 all zero; no serde boundary in this part)
- obs: clean (seeds 0/0/0/2; both hits are `log::` false positives inside `ApiCatalog::`)
- docs: 2 findings (seeds 137/0/56)
- perf: 1 finding (seeds 32/31/1)
- conc: clean (seeds 0/10/23/0)
- async: clean (seeds 346/29/2/0)
- unsafe: N/A (seeds 0/0/0; manifest forbids unsafe_code)
- ffi: N/A (seeds 0/0/0/0 all zero)
- macro: N/A (seeds 0/0/0/0 all zero)
- test: 2 findings (seeds 113/328/0/0)