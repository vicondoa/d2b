# d2b-provider-user - d2b-provider-user
Baseline: 6ebdd4cec22e6537d1376e83ff7a82b00a8492c0 | LOC audited: 2079 (excl. src/generated/**; src 1934 + tests 145) | modules: whole crate (driver.rs, effects_service.rs, facets.rs, probe.rs, test_support.rs, lib.rs; tests/registration.rs)
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: n/a (single-part lane)

## idiom
- clean: seeds 0/0/0 - no index loops, no hand-written derive-able impls (all derives are `#[derive]`), no statement-style accumulation; the crate reads as expressions (payload-value iterator in `InspectUserRequest::parse`, let-else, `is_some_and`).

## own
- d2b-provider-user#1 sev=low blast=leaf effort=S verdict=actionable - `serve_inspect_user` clones `request.groups` into the spec even though the owned `InspectUserRequest` could yield it by move; the username clone at the same site is required (reused in `inspect_user_response`) - fix: destructure `let InspectUserRequest { user_ref, username, groups } = request;` and pass `username`/`groups` by value into `UserSpec::new`, borrowing `user_ref` and `username` afterwards - [packages/d2b-provider-user/src/effects_service.rs:179, packages/d2b-provider-user/src/effects_service.rs:164-187]
  evidence: `\.clone\(\)` = 16 hits; the 15 other clones are test doubles (driver.rs:497,544,579; test_support.rs:60,81,130,151), Arc refcount bumps (effects_service.rs:258,302,605; driver.rs:217), required ownership transfers (driver.rs:246,249,305; effects_service.rs:177,386), or test fixtures - only this pair has an owned local as source
- clean: seeds 16/21/0/0 - every clone/to_owned inspected; no Rc/RefCell/Arc<Mutex>/Cow anywhere; argument positions take `&str`/`&ResourceRef`/`&UserSpec` throughout.

## type
- clean: seeds 2/0/0 - the two `fn validate_` hits are test names (`validate_accepts_the_bootstrap_user_row`, `validate_rejects_a_malformed_user_spec`), not runtime validation; domain state is parsed once into closed contract types (`InspectUserRequest`, `OsUsername`, `OsGroupName`, `ResourceRef`, `UserSpec`), no boolean-flag soup or stringly-typed state in production code.

## api
- clean: seeds 30/6/3 - the 3 `pub use` arms in lib.rs:42-44 are the house single-path pattern; `UserEffectFacets.probe: Arc<dyn UserDiscoveryEffectPort>` (facets.rs:33) is genuine shared ownership across the driver factory (driver.rs:202-205) and the service factory (effects_service.rs:258), with external consumers in d2bd (resource_plane_v3.rs, provider_lifecycle.rs, shared_provider_effects.rs; census over packages/ = 8 files); `UserDriverError`/`UserDriverStatus`/`UserDriverEffects` are pub-in-private-module and unreachable outside the crate; the test_support surface is feature-gated and consumed by d2bd plane tests.

## err
- d2b-provider-user#2 sev=low blast=leaf effort=S verdict=actionable - `UserDriverError`'s `Display` hand-copies the three `system-core-*` failure codes that `d2b-contracts` already registers as `FailureKind` constants, so the strings can silently drift from the registry and its rendered reference - fix: write `self.kind.failure_kind().code()` in the `Display` impl (driver.rs:118-124) instead of the per-arm string match, keeping the registry the single source - [packages/d2b-provider-user/src/driver.rs:118-124, packages/d2b-contracts/src/failure_kinds.rs:342-363]
  evidence: `enum \w*Error` = 1; census `SYSTEM_CORE_SPEC_INVALID|SYSTEM_CORE_USER_DISCOVERY_FAILED|SYSTEM_CORE_DRAIN_PENDING` over packages/ = 3 sites (registry, provider-host, provider-user); the codes are rendered to docs/reference/resource-runtime-failure-kinds.md
- clean: seeds 46/0/2/1 - 45 of 46 unwrap/expect hits are in `#[cfg(test)]` code; the one production expect (effects_service.rs:178) is a literally-built value (`BoundedText::parse(String::new())`) whose invariant the message names (per-card false-positive class); the 2 panics are test-helper failure messages with context; no swallowed Results (`let _ =`/`.ok();` = 0); the private `UserDriverErrorKind` enum is a closed, correctly-split taxonomy (refused/not-yet/retryable mapped in `classify_error`).

## serde
- clean: seeds 0/0/0/5 - no serde derives or attributes in this crate; the 5 serde_json sites are the wire boundaries (spec decode hooks driver.rs:164,263; the inspect-user payload round-trip effects_service.rs:83,90; one test helper); `InspectUserRequest::parse` is the hand-written admission gate over the canonical payload into closed contract types (per-card false-positive class), validating before any probe runs.

## obs
- clean: seeds 0/1/0/1 - no println/eprintln/dbg; the single tracing event (probe.rs:76) is well-formed (`tracing::debug!` with named fields `user`/`group` and a literal message, not interpolation); no spans needed on the short async paths; no secret-shaped fields anywhere.

## docs
- clean: seeds 29/0/24 - all 29 public items carry doc comments; the crate-level `#![deny(missing_docs)]` (lib.rs:5) enforces it, so the U1 card's "missing_docs not enabled anywhere" gate note does not hold for this crate; first sentences are one-line and non-narrative; the Result-returning surface (seed 3 = 24) is trait impls and private helpers whose contracts are documented at the seam (`UserDriverEffects::observe_user`, `UserDiscoveryEffectPort::discover`); no canonical-section gaps on user-facing items.

## perf
- clean: seeds 1/8/2 - the 1 `format!` (effects_service.rs:483) is test-data construction; the 8 `Vec::new` sites are empty-by-construction fixtures and recorders (empty case is the common case); the 2 `to_string` are one-shot error stringification at the seam (effects_service.rs:225) and a test assertion; no hot-path allocation, no attacker-controlled hashing, no grow-by-push loops in production code.

## conc
- d2b-provider-user#3 sev=medium blast=leaf effort=M verdict=policy-confirmed - the test-support recorder doubles and the driver's test fakes use `parking_lot::Mutex` (banned outright, KD3) at 20 `lock()` call sites with no `#[allow(clippy::disallowed_methods)]` on the enclosing items, while sibling `d2b-provider-host` uses `tokio::sync::Mutex` for the same recorder shape - fix: swap `parking_lot::Mutex` to `tokio::sync::Mutex` in `RecordingEffects`/`ScriptedProbe`/`RecordingManager`/`RecordingRequeue` (or add the sanctioned inline allow with reason "cfg(test) helper" at each site) so the deny-level flip needs no special case - [packages/d2b-provider-user/src/test_support.rs:41-42,108, packages/d2b-provider-user/src/driver.rs:469-470,563]
  evidence: `\bMutex<|\bRwLock<` = 6 hits (all parking_lot, all test-support/test-fake); lock call sites: test_support.rs:60,65,76,83,130,151 and driver.rs:483,497,508,516,524,529,530,543,544,552,557,574,579,585; policy: clippy.toml:40 (KD3 ban), clippy.toml:82 (replacement tokio::sync::Mutex), U1 (d)4 sanctioned reason "cfg(test) helper"; the 9 async-gate-allow markers record the sites as deliberate async exceptions (cite, not re-flagged), but the clippy allow is absent
- d2b-provider-user#4 sev=low blast=leaf effort=S verdict=actionable - the scripted-double flags (`fail`, `absent`, `failing`) use `Ordering::SeqCst` though they are set and read within one test task on a single-threaded `#[tokio::test]` runtime, so the strongest ordering buys nothing - fix: use `Ordering::Relaxed` for the loads/stores in test_support.rs and driver.rs:854, per the weakest-correct-ordering rule - [packages/d2b-provider-user/src/test_support.rs:77,135,140,152,155, packages/d2b-provider-user/src/driver.rs:854]
  evidence: `Atomic\w+|Ordering::` = 13 hits; 6 ordering uses, all `SeqCst`, all in test doubles; no cross-thread publish exists (flags are set and read in the same test task)
- clean: seeds 0/6/13/0 - no threads, no thread_local, no unsafe Send/Sync; production code holds no locks and shares no mutable state; the only shared state in the crate is the test-support recorders judged above.

## async
- d2b-provider-user#5 sev=high blast=leaf effort=L verdict=policy-confirmed - the bounded probe's `discover_local_user` runs blocking NSS lookups (`nix::unistd::User::from_name`, `Group::from_gid`, `Group::from_name`) inside async fns on the plane's executor worker (driver reconcile via effects_service.rs:224, and the hosted `inspect-user`), so a slow or hung NSS backend (LDAP/NIS) stalls a runtime worker per call; `spawn_blocking` is itself banned (KD2) - fix: move the NSS reads onto a dedicated bounded worker in the `d2b-core` `loader_worker` shape (one thread, bounded sync_channel, oneshot replies) and have the probe await it - [packages/d2b-provider-user/src/probe.rs:48,63,72, packages/d2b-provider-user/src/probe.rs:40-79]
  evidence: `async fn|async move|\.await` = 91 hits; reachability path is static and complete: driver.rs:342 -> effects_service.rs:224 -> probe.rs:28 -> probe.rs:48,63,72; `tokio::spawn|spawn_blocking|JoinSet|select!|join!` = 0; policy: clippy.toml:112 (spawn_blocking banned, bounded-worker shape named as the replacement); the blocking-census baseline lists no NSS class for this crate, so the site is above any tracked set
- clean: seeds 91/0/0/21 - no spawn/JoinSet/select!/tokio::sync types; no guard held across an await (the recorder locks drop before every suspension point, per the async-gate-allow markers, which are deliberate exceptions cited in finding #3); the 21 `#[tokio::test]` harnesses are single-threaded and deterministic; cancellation-safety surface is minimal (no irreversible step between awaits in `reconcile`/`serve_inspect_user`).

## unsafe
- N/A: seeds 0/0/0 all zero; no unsafe blocks/fns/impls and no `unsafe_code = "allow"` manifest (packages/d2b-provider-user/Cargo.toml sets `unsafe_code = "forbid"` under `[lints.rust]`).

## ffi
- N/A: seeds 0/0/0/0 all zero; no extern boundary, no repr(C)/transparent, no CStr/CString - the crate crosses no foreign caller.

## macro
- N/A: seeds 0/0/0/0 all zero; no macro_rules!/proc-macro definitions, no `$crate` uses (the crate only invokes std macros).

## test
- d2b-provider-user#6 sev=medium blast=leaf effort=S verdict=actionable - the "cached unrealized phase re-discovers" contract is tested only for `Pending` (`reconcile_rediscovers_a_cached_unrealized_phase`), so a regression that widened the `observed_ready` short-circuit predicate (driver.rs:283) to accept `Degraded` or `Unknown` would pass every test - fix: extend the phase loop in `reconcile_publishes_the_user_discovery_projection` (driver.rs:789-813) to run a second reconcile per phase and assert the second `observe-user` call for all three unrealized phases - [packages/d2b-provider-user/src/driver.rs:789-848, packages/d2b-provider-user/src/driver.rs:281-284]
  evidence: `#\[test\]|#\[tokio::test\]` = 25 (src+tests), `assert_eq!\(|assert_ne!\(|assert!\(` = 88; the `phase == ResourcePhase::Ready` predicate is pinned by no test for the non-Pending unrealized phases (the existing cached-phase test covers Pending only, and the phase-loop test stops after the first reconcile)
- clean: seeds 25/88/0/0 - 25 tests (12 driver, 9 effects-service, 4 registration) assert observable behavior (recorded call orders, status fields, failure classes, registry outcomes, wire payload fields) with human-written expectations; table-driven loops carry per-case failure messages; no `#[ignore]`, no property/snapshot tooling, no network or clock dependence; the registration boundary suite pins the descriptor contract (allowed sources, verbs, services, decoder, duplicate/late registration refusals).

## Coverage
- idiom: clean (seeds ran: 0/0/0)
- own: 1 finding (seeds ran: 16/21/0/0)
- type: clean (seeds ran: 2/0/0)
- api: clean (seeds ran: 30/6/3)
- err: 1 finding (seeds ran: 46/0/2/1)
- serde: clean (seeds ran: 0/0/0/5)
- obs: clean (seeds ran: 0/1/0/1)
- docs: clean (seeds ran: 29/0/24)
- perf: clean (seeds ran: 1/8/2)
- conc: 2 findings (seeds ran: 0/6/13/0)
- async: 1 finding (seeds ran: 91/0/0/21)
- unsafe: N/A (seeds: 0/0/0 all zero; no unsafe blocks and no unsafe_code = "allow" manifest - Cargo.toml sets forbid)
- ffi: N/A (seeds: 0/0/0/0 all zero; no extern boundary)
- macro: N/A (seeds: 0/0/0/0 all zero; no macro definitions)
- test: 1 finding (seeds ran: 25/88/0/0)