# d2b-provider-guest - d2b-provider-guest
Baseline: 6ebdd4cec | LOC audited: 6,423 (src 6,192 + tests 231; excl. src/generated/**: none present) | modules: whole crate
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: whole crate

## idiom
- clean: seeds: idiom1=2 (both in test loops, effects_service.rs:1916,1926), idiom2=1 (shutdown.rs hand-written Default preserving ch_api::DEFAULT_TIMEOUT - the U1 false-positive class: a field-wise derive would not preserve the invariant), idiom3=3 (justified accumulators: recursive walk (driver.rs:1348), sequential-await filter (effects_service.rs:1000), conditional push+extend (effects_service.rs:1035);; checked expression shape across src/**: no production index loops, no hand-written replaceable derives, no statement-style accumulation the skill names a pipeline for.

## own
- d2b-provider-guest#4 sev=low blast=leaf effort=S verdict=actionable - Retiring obsolete children sorts by teardown rank plus row name by cloning every row's name String into the sort-key tuple - fix: sort with a comparator borrowing the name (`sort_by(|a,b| teardown_rank(&a.key.type_name).cmp(&teardown_rank(&b.key.type_name).then_with(|| a.key.name.cmp(&b.key.name)))`), dropping the per-row allocation - [packages/d2b-provider-guest/src/driver.rs:981]
  evidence: seed `\.clone\(\)` = 91 hits src; driver.rs:981 is the only clone whose purpose is the owned-key bound of sort_by_key; the name String is otherwise borrowed throughout
- d2b-provider-guest#5 sev=low blast=leaf effort=S verdict=actionable - The ACA framework controllers clone each stored candidate list before collect (`state.sandbox.clone().into_iter().collect()`), allocating an intermediate Vec per candidate read - fix: `state.sandbox.iter().cloned().collect()` (same element copies, one fewer allocation - [packages/d2b-provider-guest/src/effects_service.rs:243, packages/d2b-provider-guest/src/effects_service.rs:256]
  evidence: seed `\.clone\(\)` = 91 hits src; both sites clone an Option<Vec> held behind tokio::sync::Mutex only to iterate it into the candidate-set newtype
- clean: remaining clones (69 further hits incl. .to_owned/.to_vec/.to_string) are ownership copies into typed request structs and dyn ports (GuestEffectRequest field copies, Arc::clone at effects-service/factory boundaries), status.resource clones into the R11 status sink/projection;, and fixture seeds - each explainable in one sentence; no Rc</RefCell< in src

## type
- d2b-provider-guest#1 sev=medium blast=family effort=M verdict=actionable - GuestDriverArgs.zone is String while every other Guest surface speaks ZoneId (GuestEffectFacets.zone, GuestDriver.zone,,forcing a parse-at-construction expect at GuestDriver::new - fix: type GuestDriverArgs.zone as ZoneId and pass ZoneId directly into GuestDriver::new, dropping the expect and the daemon-side String round-trip - [packages/d2b-provider-guest/src/driver.rs:709, packages/d2b-provider-guest/src/driver.rs:788, packages/d2bd/src/resource_plane_v3.rs:2926, packages/d2b-provider-guest/tests/registration.rs:15]
  evidence: seed `(mode|kind|state): String` = 0 hits (the seed's field names miss zone; manual public-surface read found the String-typed zone); census: GuestDriverArgs over packages/+tests/docs/labs/nixos-modules =12 hits
- clean: seeds 6/0/0; the 4 production validate_*/check_* gates (effects_service.rs:832,842,883,953)are dynamic state/fence checks on wire-derived data (not parse-once replacements;; wire types (GuestSpec etc) are schema-mirrors pinned by the canonical-bytes test and docs/reference/manifest-schema.md;; no boolean-flag soup or Option-pair smells outside the one finding.

## api
- clean: seeds 114/9/7; public surface is the house pattern: pub mod + lib.rs pub use re-exports((lib.rs:41-49;; the deliberate wide exports of contract-family vocabulary stay; facets Arc<dyn> fields (facets.rs:59,65,and GuestTargetEffects Arc<dyn> map (target_service.rs:91)are declared shared-ownership values the daemon composition root supplies (documented, not caller-derived;; test-support feature-gated doubles are the recorded test-consumed surface;; the dependency types (ResourceRef, ResourceKey, GuestTargetError etc) in signatures are in-tree workspace contract crates (publish=false paths,,not published semver surfaces.

.

## err
- clean: seeds 128/3/1/3; production unwrap/expect sites are 3 invariant-naming expects (driver.rs:788 "driver zone was validated at construction", driver.rs:1304 "manager keys carry canonical resource references", driver.rs:1361 "child metadata renders"), all on compiler-invisible invariants, and the remaining 125 hits live in #[cfg(test)];; the let _ sites are a ?-propagating kind-classification call (driver.rs:1041, an deliberately unused request param (effects_service.rs:941,and a test-scope drop (effects_service.rs:1934 - none swallow a Result a caller must see;; the single panic!/unreachable! is a test-only match arm (effects_service.rs:1838;; error enums are closed, context-carrying variants with static code() labels used in logs only, no wire error-code surface touched.

.

## serde
- clean: seeds 2/13/0/40; GuestSpec's wire gate uses rename_all camelCase, deny_unknown_fields on the Wire admission struct, flatten+skip_serializing_if+default(fns)) for the three optionality meanings, and serde_json boundary sites map errors to closed kinds; the hand-written Deserialize at guest_spec.rs:81 is the recorded live admission-gate class (over-engineering-audit-refusal, not reflagged;; the canonical-bytes test pins the exact wire shape and a hand-written JsonSchema derive covers the schema surface.



## obs
- clean: seeds 0/0/0/13; all 13 tracing sites are structured events with named fields ((code=, source=, plane=?, detail=, guest=, dependency=, reason=, error=, field=, resource=; no println!/eprintln! in src (CLI product output lives elsewhere);; no secrets enter fields (tokens ride redacted types or as bounded labels;; no #[instrument] spans needed for these short per-pass contexts;; the status-sink lock sites carry recorded async-gate-allow marks (not reflagged).

## docs
- d2b-provider-guest#7 sev=low blast=leaf effort=M verdict=actionable - No public Result-returning item carries a canonical `# Errors` doc section (docs2 seed = 0 hits src), despite #![deny(missing_docs)]]and ~107 Result-returning pub items - fix: add `# Errors` headings naming the refusal conditions on the trait/fn contracts ((facets.rs:63, target_control.rs:95, driver.rs:403, target_service.rs:68 etc.) - [packages/d2b-provider-guest/src/facets.rs:63, packages/d2b-provider-guest/src/target_control.rs:95, packages/d2b-provider-guest/src/driver.rs:403, packages/d2b-provider-guest/src/target_service.rs:68]
  evidence: seed `/// # (Examples|Errors|Panics|Safety)` = 0 hits src; seed `-> Result<` = 107 hits src - every Result-returning pub item is undocumented for its error contract under the rust-docs canonical-section rule
- clean: #![deny(missing_docs)] ((lib.rs:17) makes every public item carry a doc comment, and first sentences are contract-shaped across the sampled surface;; no doctests area present to rot;; magic values ((TARGET_CONTROL_TIMEOUT, GUEST_RESYNC, DEFAULT_TIMEOUT(are documented with their why.

.

## perf
- d2b-provider-guest#6 sev=low blast=leaf effort=S verdict=actionable - Two hex-ID builders format a fresh String per byte ((driver.rs:863-868 map(|byte| format!("{byte:02x}")) into a String, effects_service.rs:983-988 push_str(&format!)...)) in a loop), allocating ~16 and ~8 Strings per reconcile pass - fix: write! to one with_capacity String per builder (or a crate-local hex helper reusing the buffer - [packages/d2b-provider-guest/src/driver.rs:863, packages/d2b-provider-guest/src/driver.rs:868, packages/d2b-provider-guest/src/effects_service.rs:983, packages/d2b-provider-guest/src/effects_service.rs:988]
  evidence: static (unmeasured); seed `format!\(` = 30 hits src; the remaining format! sites are error-path/one-shot spec-name Strings ((U1 false-positive class);
- clean: seeds 30/19/5; Vec::new() sites are empty-case-common field inits ((driver.rs:791,952)or push/filter accumulators with sequential awaits/no known capacity ((effects_service.rs:630,1000,1035;; to_string() sites are error-detail conversions into failure details ((effects_service.rs:664,676,908,1000;; no hot-path collections, attacker-controlled hashing, or unbounded scans identified in production code.



## conc
- d2b-provider-guest#2 sev=medium blast=family effort=M verdict=policy-confirmed - GuestStatusSink ((a pub type re-exported at lib.rs:42)is Arc<parking_lot::Mutex<Option<Value>>>, injecting the banned parking_lot lock type into this crate's and d2bd's public signatures; the production write sites carry recorded "synchronous path"/async-gate allows,,but every future sink caller inherits the banned type - fix: replace with Arc<tokio::sync::Mutex<Option<Value>>>and convert the write sites to .lock().await per the replacement vocabulary - [packages/d2b-provider-guest/src/driver.rs:502, packages/d2b-provider-guest/Cargo.toml:29, packages/d2b-provider-guest/src/effects_service.rs:1443]
  evidence: seed `\bMutex<|\bRwLock<` = 23 hits src (production tokio::sync::Mutex uses at driver.rs:449,effects_service.rs:181,400,618 are the sanctioned async vocabulary; policy: clippy.toml:40-43 bans parking_lot outright (KD3; U33 carve-out revoked)and clippy.toml:82-84 names tokio::sync::Mutex::lock as the replacement;; the per-site "synchronous path" allows at the daemon write sites are the U1 (d)4 recorded-exception list, not the public type
- d2b-provider-guest#3 sev=medium blast=leaf effort=M verdict=policy-confirmed - The test-support recorder doubles and the driver test harnesses hold recorder/queue state in parking_lot::Mutex fields, which the ban covers for tests too (KD4 uniform rule,,and no per-site clippy allow exists at these sites - fix: convert to tokio::sync::Mutex with async accessors (or the documented blocking-seat helpers for worker-thread-only callers),,keeping the recorded async-gate-allow marks until converted - [packages/d2b-provider-guest/src/test_support.rs:59, packages/d2b-provider-guest/src/test_support.rs:171, packages/d2b-provider-guest/src/driver.rs:1534, packages/d2b-provider-guest/src/driver.rs:1761]
  evidence: seed `\bMutex<|\bRwLock<` = 23 hits src (the four harness members above carry the banned type; policy: clippy.toml:40-43 (KD3, clippy.toml:82-84;; the 2026-09-16 async-purity plan KD4 includes tests in the ban;; U1 (d)2 names the R4 dedicated bounded-worker boundary as the only exception
- clean: seeds 0/23/11/0; production locks are all tokio::sync::Mutex, held briefly and never across an await ((driver.rs:449,effects_service.rs:181,400,618;; the test-only atomics order SeqCst on single-threaded doubles ((fine;,and no std::thread spawn/scope or unsafe Send/Sync claims exist in production code.

## async
- clean: seeds 252/0/10/28; no tokio::spawn/spawn_blocking/JoinSet/select!/join! in src;; production shared state is tokio::sync::Mutex held briefly, never across an await;; the sink lock calls at effects_service.rs:1443 (and the daemon-side equivalents)carry "async-gate-allow: synchronous lock acquisition" marks and "synchronous path" clippy allows - recorded exceptions, not reflagged;; test-support recorder locks carry "async-gate-allow: test-support recorder lock" marks - recorded;; the parking_lot policy class behind those locks is recorded under conc#2/#3; no std::thread::sleep, blocking I/O,,or CPU stretches without awaits inside async fns identified in production code.



## unsafe
- N/A: seeds 0/0/0; no unsafe blocks/fns/impls, no // SAFETY: or transmute/from_raw/MaybeUninit sites in src/**;; the crate manifest's [lints.rust] unsafe_code="forbid" (seed4 alone does not make the lens applicable.

## ffi
- N/A: seeds 0/0/0/0; no extern "C"/no_mangle/link_section, catch_unwind, repr(C)/repr(transparent), or CStr/CString/c_char sites;; the crate crosses no FFI boundary (all I/O rides tokio/ttrpc/d2b-session-unix wrappers.



## macro
- N/A: seeds 0/0/0/0; no macro_rules! definitions, proc_macro/syn::/quote!, $crate, or to_compile_error/new_spanned sites;; std macros ((format!, vec!, json!)) are not definitions; no DSL or impl-per-type macro need identified

## test
- clean: seeds 42/87/0/0; tests pin the descriptor declaration and registry behavior (tests/registration.rs), the canonical spec bytes and schema vector ((guest_spec.rs),,the qemu/aca/azure reconcile+finalize+adopt+delete+status-projection semantics ((driver.rs and effects_service.rs #[cfg(test)] modules),,and the Cloud Hypervisor fail-closed shutdown (shutdown.rs;; expected values are literal or pinned constants or hand-asserted enumerations, no test restates its own implementation or computes its expectation with the logic under test;; no #[ignore], no network dependence, no property/snapshot tooling needed for the current surface;; the test-harness parking_lot policy class is recorded under conc#3.

## Coverage
- idiom: clean (seeds ran: 2/1/3)
- own: 2 finding(s)
- type: 1 finding(s)
- api: clean (seeds ran:  114/9/7)
- err: clean (seeds ran: 128/3/1/3)
- serde: clean (seeds ran: 2/13/0/40)
- obs: clean (seeds ran:  0/0/0/13)
- docs: 1 finding(s)
- perf: 1 finding(s)
- conc:  2 finding(s)
- async: clean (seeds ran:  252/0/10/28)
- unsafe: N/A (seeds: 0/0/0; no unsafe blocks; manifest forbids (seed4 alone does not make it applicable))
- ffi: N/A (seeds:  0/0/0/0; no FFI surface)
- macro: N/A (seeds:  0/0/0/0; no macro definitions)
- test: clean (seeds ran:  42/87/0/0)