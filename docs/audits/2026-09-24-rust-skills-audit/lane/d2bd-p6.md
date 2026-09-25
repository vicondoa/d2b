# d2bd-p6 - d2bd - part 6/8
Baseline: 6ebdd4cec22e6537d1376e83ff7a82b00a8492c0 | LOC audited: 10772 (excl. src/generated/**) | modules: resource_plane_v3, provider_lifecycle, credential_resource_runtime, resource_runtime/plane_controller_bridge
Lenses: idiom own type api err serde obs docs perf conc async unsafe ffi macro test | Partitions: src/resource_plane_v3.rs, src/provider_lifecycle.rs, src/resource_runtime/**, src/credential_resource_runtime.rs

## idiom
- d2bd-p6#1 sev=low blast=leaf effort=S verdict=actionable - `registered_service_decl` (provider_lifecycle) and `registered_service_factories` (resource_plane_v3) are 15-arm `if ... else if` chains over `&'static str` equality where a `match` reads as a table, gets exhaustiveness-free fallthrough by construction, and does not re-test the winner's earlier arms - fix: convert both chains to `match service { PROCESS_EFFECTS_SERVICE.id => ..., ... , _ => None/continue }`, keeping the `as Arc<dyn EffectServiceFactory>` coercions on the factory arms - [packages/d2bd/src/provider_lifecycle.rs:78, packages/d2bd/src/resource_plane_v3.rs:2226]
  evidence: idiom seeds `for \w+ in 0\.\.`/`impl (Default|From|PartialEq|Eq|Debug|Clone|Hash) for`/`let mut \w+ = (String|Vec)::new\(\)` = 6/1/8 hits (the three index-loop hits and all eight `Vec::new` accumulators are bounded wait loops and topped-up listings with early exits that an iterator pipeline would obscure; the only hand-impl hit is a test `Default`); the two if-else chains were read whole at the cited lines, not seed-caught
- clean: none of the flagged classes otherwise - the `for _ in 0..N` hits are bounded poll waits (tests), the `Vec::new` hits are partition/plan listings with early returns.

## own
- clean: seeds `\.clone\(\)`/`\.to_owned\(\)|\.to_vec\(\)|\.to_string\(\)`/`Rc<|RefCell<|Arc<Mutex<|Arc<RwLock<`/`Cow<` = 220/171/1/0 hits; sampled: 49 of 392 hits (every 8th, lens order) - every sampled clone is explainable (Arc clones at construction and `tokio::spawn` boundaries, `key`/`zone` clones moved into async calls, `to_owned` into map keys and wire metadata), and the single `Arc<Mutex<..>>` (ProviderAgentAuditLog, provider_lifecycle.rs:439) is a genuinely shared sync-only sink fed through the envelope.

## type
- clean: seeds `fn validate_\w+|fn check_\w+`/`is_\w+: bool|\w+_flag: bool`/`(mode|kind|state): String` = 2/0/0 hits - both validation fns are boundary gates (`validate_request_scope` on a test-only wire reader, `check_registry_catalog` as a startup invariant), no flag-soup fields or stringly-typed state in the four files.

## api
- d2bd-p6#2 sev=low blast=leaf effort=S verdict=actionable - `ResourcePlaneV3::targets`/`hub`/`store`/`registry` return `&Arc<T>`, exposing refcount plumbing in the accessor surface and forcing the two callers that need the shared handle to clone through the reference - fix: return `&TargetDirectory`/`&SpecStore`/`&PlaneResourceRegistry` from the borrow-only accessors and `Arc<WatchHub>`/`Arc<TargetDirectory>` by value from `hub()`/`targets()`, then update `Arc::clone(plane.hub())` at resource_runtime.rs:4423 and `Arc::clone(plane.targets())` at composition.rs:11250 to `plane.hub()`/`plane.targets()` - [packages/d2bd/src/resource_plane_v3.rs:3227, packages/d2bd/src/resource_plane_v3.rs:3295, packages/d2bd/src/resource_plane_v3.rs:3301, packages/d2bd/src/resource_plane_v3.rs:3308]
  evidence: api seed `pub .*\b(Arc|Rc|Box|RefCell)<` = 8 hits; census: `\.hub\(\)` over packages/ (excl. generated) = 1 hit (resource_runtime.rs:4423), `\.targets\(\)` = 4 hits (composition.rs:11121,11143,11158,11250), `\.store\(\)` in-lane = 3 test hits - the two `Arc::clone)..)` call sites cited are the load-bearing ones
- clean: no internals leak beyond the `&Arc` accessors - `client()` returns `&ResourceManagerClient`, `pub use` seed = 0, module surface is `pub(crate)` per lib.rs:16-17, and the `Arc<dyn ...>` fields of `ConstructionInputs` are genuine shared facet ownership with documented callers.

## err
- d2bd-p6#3 sev=medium blast=leaf effort=M verdict=actionable - `PlaneError` carries five `String` variants (`FoundationSeed`, `ManagerSpawn`, `Authority`, `Target`, `Bundle`) that wrap the inner error with `error.to_string()`/`format!` at every production site, dropping the source chain the enum's `#[from]` variants already preserve for `SpecStore`/`ProviderRegistration`/`ManagerRpc` - callers of `ResourcePlaneV3::prepare` cannot distinguish a refused spec-store open from a create failure without string-matching - fix: give each String variant a typed payload or `#[source]` (e.g. `PlaneError::Authority(#[from] d2b_core::loader_worker::Error)` where `SpecStore::open` already yields `SpecStoreError` through `#[from]`, and keep the stage word in the `Display` message, not the variant), deleting the `to_string()` wraps at the cited sites - [packages/d2bd/src/resource_plane_v3.rs:2646, packages/d2bd/src/resource_plane_v3.rs:3016, packages/d2bd/src/resource_plane_v3.rs:3025, packages/d2bd/src/resource_plane_v3.rs:3346, packages/d2bd/src/resource_plane_v3.rs:3081]
  evidence: err seed `enum \w*Error` = 2 hits (PlaneError, ProviderStartupError); err seed `\.unwrap\(\)|\.expect\(` = 390 hits, sampled: 49 of 390 (every 8th) - zero production hits below the test-mod boundaries; the String wraps were read at the cited lines (map_err to_string cluster: 2568, 3081, 3109, 3131, 3244-3275, 3346)
- d2bd-p6#4 sev=low blast=leaf effort=S verdict=actionable - `ConstructionInputs::production` swallows the `attach_process_providers` Result at the compose-once fallback, so a `StateUnavailable` collision (or a future attach failure) silently leaves whichever instance won in the shared slot, and the plane keeps composing with its own instance either way - fix: `state.provider_runtime.attach_process_providers(Arc::clone(&providers)).map_err(|error| PlaneError::Authority(error.to_string()))?` (or a dedicated variant), matching the site's other rejections - [packages/d2bd/src/resource_plane_v3.rs:1956]
  evidence: err seed `let _ = |\.ok\(\);` = 3 hits (339 is the documented idempotent store attach; 5324 is test code); the swallowed call is the only production `let _ =` on a fallible Result - read against attach_process_providers' sole `StateUnavailable` error at provider_registry.rs:471-484
- clean: panic policy is sound in production - `\bpanic!\(|...` = 26 hits, all inside the `#[cfg(test)]` modules (canonical-builder helpers and bounded wait loops); err4 both enums are typed with documented variants, and `ProviderStartupError::code()` keeps stable refusal names.

## serde
- clean: seeds `derive\([^)]*(De)?[Ss]erialize`/`serde\((rename_all|...)\)`/`impl .*Deserialize.*for`/`serde_json::from_|serde_json::to_` = 0/0/0/63 hits - the 63 decode/encode sites are canonical-JSON boundary reads and test fixtures: production sites decode store-derived or bundle-verified bytes with `.ok()?`/`map_err` guards (no untrusted-input deserialization in this scope), and no hand-written `Deserialize` impls live here; the ~40 `from_value(json!({...}))` hits are literal test payloads.

## obs
- d2bd-p6#5 sev=low blast=leaf effort=S verdict=actionable - `publish_trusted_context`'s failure arm interpolates the error into the message (`"trusted-context publication refused: {error}"`) while the sibling `Ok(_)` arm and every other event in the file carry named fields, so the failure reason is not queryable as a column - fix: move the error into a field (`error = %error, "trusted-context publication refused"`), matching the adjacent arms and the plane's other warn sites - [packages/d2bd/src/provider_lifecycle.rs:1085]
  evidence: obs seed `(info|debug|warn|error|trace)!\("[^"]*\{` = 0 hits (the macro call opens on the next line, so the seed misses it); obs seed `tracing::|log::` = 25 hits, all 25 read - this is the only message-interpolation site in the scope
- clean: zero `println!`/`eprintln!` hits; no `#[instrument]` spans but every event carries named fields (`zone`, `revision`, `operation`, `error = %error`), and errors are logged only at the boundary that resolves them.

## docs
- d2bd-p6#6 sev=low blast=leaf effort=S verdict=actionable - four public items in the plane's most-documented file are undocumented while their siblings carry `///` contracts: `PlaneResourceRegistry::new()`, `ZoneAuthorityInputs::controller_generation`, and the three fields of `BundleIngestReport` - fix: add the one-line contract each (constructor convenience, the zone authority's controller generation source, and per-field "the rows this ingestion applied/removed/protected") - [packages/d2bd/src/resource_plane_v3.rs:292, packages/d2bd/src/resource_plane_v3.rs:1770, packages/d2bd/src/resource_plane_v3.rs:3432]
  evidence: docs seed `^\s*pub (fn|struct|enum|trait|const|type)` = 22 hits, all read - 18 carry doc comments; the four gaps are the cited lines; `/// # (Examples|Errors|Panics|Safety)` = 0 and `-> Result<` = 0 (line-broken signatures), and canonical sections are not required for this `pub(crate)` module surface per the card's internal-crate carve-out
- clean: no other public item in the scope lacks a first-sentence contract; module docs exist in all four files.

## perf
- clean: seeds `format!\(`/`Vec::new\(\)|...`/`\.to_string\(\)` = 40/89/21 hits, all read - the `format!` hits are error paths, refusal-reason rendering, and test literals; the `Vec::new`/`BTreeMap::new` hits are startup planning listings, bounded drain windows, and test fixtures; the `to_string` hits are `map_err` conversions (the err finding #3's subject) - nothing sits on a hot path, and every loop here is bounded (drain windows, budget polls); static (unmeasured).

## conc
- clean: seeds `std::thread::|...`/`\bMutex<|\bRwLock<`/`Atomic\w+|Ordering::`/`thread_local!|unsafe impl (Send|Sync) for` = 0/17/45/0 hits - the 17 `Mutex` sites are `tokio::sync::Mutex` over shared maps and gates with brief guards (never held across a fallible await), the 45 atomic hits are `Relaxed` counters/flag in `AnchorSubscriptionState` (the weakest correct ordering for test-observable counters) plus test counters, and `SeqCst` appears only in test assertions; no `unsafe impl Send/Sync`, no threads spawned in this scope; the `drain_order` `try_lock` fail-closed view is documented at provider_lifecycle.rs:1042-1044.

## async
- clean: seeds `async fn|async move|\.await`/`tokio::spawn|...`/`tokio::sync::(Mutex|RwLock|Notify)`/`#\[tokio::(main|test)\]|Runtime::block_on` = 564/4/22/3 hits; sampled: 47 of 564 (every 12th) plus full reads of the 4 spawn sites, 22 tokio-sync sites and 3 test attributes - no blocking work inside async context (SQLite open and store migration run on the sanctioned `d2b_core::loader_worker` bounded seat per the KTD2 comment at `prepare`), no std-sync guard spans an await (the single-flight `tokio::sync::Mutex` gate in `ComponentCredentialSession` is the correct shape), `tokio::spawn` is limited to the one long-lived subscription task, and waits are bounded (`timeout_at` windows, budget polls).

## unsafe
- clean: seeds `\bunsafe \{|...`/`// SAFETY:`/`transmute|from_raw|MaybeUninit|mem::zeroed`/`unsafe_code` = 0/0/4/0 hits - all four `from_raw` hits are safe functions (`Mode::from_raw_mode`, `std::io::Error::from_raw_os_error`), so the scope contains no `unsafe` block, no `unsafe fn`, and no unsafe-code lint exception; the ledger's enumeration (U1 section d 8) needs no new entry from this lane.

## ffi
- N/A (seeds: 0/0/0/0 all zero; no `extern "C"`, `no_mangle`, `catch_unwind`, `repr(C)`/`repr(transparent)` or `CStr`/`CString`/`c_char` anywhere in the scope - the anchored-fd API is exercised through `rustix` safe facades only).

## macro
- N/A (seeds: 0/0/0/0 all zero; no `macro_rules!` definitions, no proc-macro or `$crate` usage - the only macros are `include!`d generated registrations and std macros).

## test
- clean: seeds `#\[test\]|#\[tokio::test\]`/`assert_eq!\(|assert_ne!\(|assert!\(`/`proptest!|insta::assert|rstest`/`#\[ignore\]` = 13/202/0/0 hits; sampled: 41 of 202 (every 5th) plus all 13 test attributes - assertions target behavior with messages (refusal codes, applied/removed counts, revisions, projection state), use `matches!` on variants before any Display check, and wait on deterministic poll loops with bounded budgets; no ignored tests, no property/snapshot tooling, no network or wall-clock dependence beyond bounded sleeps.

## Coverage
- idiom: 1 finding
- own: clean (seeds ran: 220/171/1/0)
- type: clean (seeds ran: 2/0/0)
- api: 1 finding
- err: 2 findings
- serde: clean (seeds ran: 0/0/0/63)
- obs: 1 finding
- docs: 1 finding
- perf: clean (seeds ran: 40/89/21)
- conc: clean (seeds ran: 0/17/45/0)
- async: clean (seeds ran: 564/4/22/3)
- unsafe: clean (seeds ran: 0/0/4/0; all four hits are safe `from_raw*` functions, no unsafe code in scope)
- ffi: N/A (seeds: 0/0/0/0 all zero; no FFI surface in the assigned files)
- macro: N/A (seeds: 0/0/0/0 all zero; no macro definitions or proc-macro usage)
- test: clean (seeds ran: 13/202/0/0)