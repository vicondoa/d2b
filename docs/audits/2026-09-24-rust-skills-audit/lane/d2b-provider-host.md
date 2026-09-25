# d2b-provider-host - d2b-provider-host
Baseline: 6ebdd4cec | LOC audited: 2092 (src 1942 excl. src/generated/**, tests 150) | modules: whole crate (driver.rs, effects_service.rs, facets.rs, lib.rs, probe.rs, test_support.rs; tests/registration.rs)
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: whole crate

## idiom
- d2b-provider-host#1 sev=low blast=leaf effort=S verdict=actionable - stray misindented closing brace at test_support.rs:185 closes `impl RecordingMinijailGate` at 4-space indent (the fn body closes at :183, the impl at :185) - fix: reindent the stray `}` to column 0 (rustfmt would flag it) - [packages/d2b-provider-host/src/test_support.rs:185]
  evidence: idiom seeds = 1 hit (seed 3 `let mut \w+ = (String|Vec)::new()` at effects_service.rs:109, a push-loop the skill's plain-for carve-out covers: side-effecting `.await` probe calls plus an early `?` return); stray brace confirmed by awk line dump, not a seed hit
- clean: seeds ran: 0/0/1; no index loops, no hand-written derives (all impls are deliberate: Debug/Display redaction via label_identity macros, Display as wire kind names), no statement-style accumulation outside the one carve-out loop

## own
- d2b-provider-host#2 sev=low blast=leaf effort=S verdict=actionable - avoidable clones of the row key strings before parsing into identity newtypes: `ResourceTypeName::parse`/`ResourceName::parse` take `impl Into<String>`, so `&String` converts without cloning - fix: pass `&ctx.key().type_name` / `&ctx.key().name` at driver.rs:263/266 (or `.as_str()`) - [packages/d2b-provider-host/src/driver.rs:263, packages/d2b-provider-host/src/driver.rs:266]
  evidence: own seed 1 (`.clone()`) = 10 hits; the only production-code clones are these two (the rest are test fakes, test-support doubles, Arc refcount bumps, and error/status construction); parse signature at packages/d2b-contracts-resource/src/v3/identity.rs:79
- clean: seeds ran: 10/22/0/0; remaining clones are explainable: `error.detail.clone()` (driver.rs:334, trait passes `&HostDriverError`), `self.facets.clone()` (effects_service.rs:225, one Arc refcount bump per zone respawn), `Arc::clone` at spawn-free factory create, `to_owned()` at wire/error boundaries; no Rc/RefCell/Arc<Mutex>/Cow in production code

## type
- clean: seeds ran: 4/0/0; the 4 hits are `#[tokio::test] async fn validate_*` test names, not runtime validation helpers; no boolean flags, no stringly-typed state; `HostDriverErrorKind` is a closed enum splitting by caller action (refused/not-yet/retryable); the providerRef fence is the typed admission check at the decode boundary, not validate-at-every-callsite

## api
- d2b-provider-host#3 sev=low blast=leaf effort=S verdict=actionable - dead `pub` visibility on seven items in the private `mod driver` that are never re-exported: `HostDriver`, `HostDriverError`, `HostDriverStatus`, `HostDriverFactory`, `HostDriverEffects`, `host_spec_decoder`, `HOST_REOBSERVE` - fix: make them `pub(crate)` (the live surface is the lib.rs re-export set: host_descriptor, HOST_EFFECTS_SERVICE, HostEffectsServiceFactory, HostEffectFacets, MinijailPlatformGateSource, production_probe, the three probe constants, MinijailPlatformGate) - [packages/d2b-provider-host/src/driver.rs:84, packages/d2b-provider-host/src/driver.rs:111, packages/d2b-provider-host/src/driver.rs:147, packages/d2b-provider-host/src/driver.rs:174, packages/d2b-provider-host/src/driver.rs:189, packages/d2b-provider-host/src/driver.rs:206, packages/d2b-provider-host/src/driver.rs:239]
  evidence: api seed 1 (`\bpub (fn|struct|enum|trait|type|const|mod) `) = 35 hits; census: `HostDriverError|HostDriverStatus|HostDriverFactory|HostDriverEffects|host_spec_decoder|HOST_REOBSERVE` over packages/, nixos-modules/, tests/, docs/reference/, labs/ = 0 code hits outside the crate (2 README.md prose mentions only); `HostDriver::new` is already `pub(crate)` while its type is `pub`, marking the visibility as accidental
- clean: seeds ran: 35/7/5; the `Arc<dyn ...>` in public signatures (facets.rs:34, probe.rs:82-83, driver.rs:208/240) is genuine shared ownership - the same probe Arc is handed to the driver factory, the effects service, and the daemon composition root (d2bd/src/process_provider_runtime.rs:192-196, d2bd/src/shared_provider_effects.rs:3444); lib.rs `pub use` arms are the house single-surface pattern and every re-export has an external consumer (d2bd/src/resource_plane_v3.rs:64, provider_lifecycle.rs:43)

## err
- d2b-provider-host#4 sev=low blast=leaf effort=S verdict=actionable - the `HostDriverEffects::observe_host` seam returns `Result<HostObservationReport, String>`: the production impl flattens the probe error and the fallback reconcile error into one `format!("{probe_error}; {error}")` message, losing the source chain; an internal crate per the skill wants an enum (or thiserror with `#[source]`) - fix: introduce a small closed error enum (e.g. `ObserveError { Probe(SystemCoreError), Reconcile(String) }` with `#[source]`) on the trait and both impls - [packages/d2b-provider-host/src/driver.rs:197, packages/d2b-provider-host/src/effects_service.rs:180]
  evidence: err seed 1 (`.unwrap()|.expect()`) = 39 hits, all in `#[cfg(test)]` modules or test-support; seed 4 (`enum \w*Error`) = 1 hit; the String seam is the only untyped error in the crate (caller maps it to a FailureDetail note, no string-matching today, so low not medium)
- d2b-provider-host#5 sev=low blast=leaf effort=S verdict=actionable - `HostDriverError::Display` re-spells the three failure-kind codes ("system-core-spec-invalid", "system-core-host-observation-failed", "system-core-drain-pending") that `HostDriverErrorKind::failure_kind()` already maps to, so a registry-code rename drifts silently - fix: `formatter.write_str(self.kind.failure_kind().code())` using the public `FailureKind::code()` - [packages/d2b-provider-host/src/driver.rs:129, packages/d2b-provider-host/src/driver.rs:99]
  evidence: err seed 4 = 1 hit; `FailureKind::code()` is public and registry-backed (packages/d2b-resource-runtime/src/error.rs:980, docs/reference/resource-runtime-failure-kinds.md generated from it)
- clean: seeds ran: 39/0/0/1; production code has zero unwrap/expect/panic sites; the 39 unwrap/expect hits are all in `#[cfg(test)]` and test-support doubles with named invariants ("uncontended test mutex"); `HostDriverErrorKind` splits by caller action and maps onto the registered failure kinds

## serde
- clean: seeds ran: 0/0/0/4; the four `serde_json::from_`/`to_` hits are boundary decodes with error mapping: the spec decoder (driver.rs:175), the HostSpec admission decode of the canonical base (driver.rs:292), and the `inspect-host` payload built through the canonical JSON object path (effects_service.rs:75, from_value over json! - the documented escape-safe route); no hand-written Deserialize, no wire type defined in this crate

## obs
- N/A: seeds 0/0/0/0; the crate carries no tracing/log dependency (Cargo.toml [dependencies] has none) and emits no telemetry of its own - the effects service returns structured payloads instead

## docs
- d2b-provider-host#6 sev=low blast=leaf effort=S verdict=actionable - `HostDriverEffects::observe_host` is the one pub Result-returning item whose doc contract lacks an `# Errors` section: "or report why the observation could not be taken" does not enumerate the failure conditions (probe failure -> retryable HostObservation; spec decode -> SpecInvalid) - fix: add `# Errors` listing the two failure conditions and their classification - [packages/d2b-provider-host/src/driver.rs:189]
  evidence: docs seed 2 (`/// # (Examples|Errors|Panics|Safety)`) = 0 hits across 34 pub items; the crate is `#![deny(missing_docs)]` (lib.rs:25) and every pub item carries a first-sentence doc, so this is the remaining contract gap
- clean: seeds ran: 34/0/30; module docs present in all six modules; magic values documented with the why (HOST_REOBSERVE echoes the old 5s resync, the probe constants pin cross-family agreements); no `ignore`d doctests (no doctests at all)

## perf
- clean: seeds ran: 2/13/1; all hits are cold-path or test code: `format!("/sys/module/{}")` in the per-reconcile Usbip probe (probe.rs:174), the error-path `format!` (effects_service.rs:180), `Vec::new()` in test fakes and the fixed 11-class capability loop (one probe per reconcile); `to_string()` once in runtime_path building; no hot loop allocates; static (unmeasured)

## conc
- clean: seeds ran: 0/7/11/0; every Mutex/atomic hit lives in test-support doubles and unit-test fakes (tokio::sync::Mutex + try_lock with "uncontended test mutex" expects, SeqCst script flags) - appropriate for test doubles; production code holds no shared state, spawns no threads, and declares no manual Send/Sync

## async
- clean: seeds ran: 112/0/14/19; production async code uses the sanctioned vocabulary: `tokio::fs::read_dir` for /proc and /dev/dri enumeration (probe.rs:112/129); the two synchronous-path sites (`read_bounded` std::fs::File::open + read_to_end, `is_socket` std::fs::metadata) carry per-site `#[allow(clippy::disallowed_methods, reason = "synchronous path")]` - a sanctioned reason tracked by the blocking census (baseline packages/xtask/data/blocking-census-baseline.json lists the crate all-zero because the allows exempt the sites); no guard held across `.await`; no spawn/select!/join!; no cancellation-sensitive irreversible step (probes are read-only); no async-gate-allow markers in the crate

## unsafe
- N/A: seeds 0/0/0/0; manifest `unsafe_code = "forbid"` (Cargo.toml [lints.rust]) alone does not make the lens applicable

## ffi
- N/A: seeds 0/0/0/0; no extern "C", no repr(C), no CStr/CString anywhere in the crate

## macro
- N/A: seeds 0/0/0/0; no macro_rules!, no proc-macro/syn/quote usage in the crate

## test
- clean: seeds ran: 23/71/0/0; 23 test fns (19 `#[tokio::test]` in src, 3 `#[tokio::test]` + 1 `#[test]` in tests/registration.rs) assert observable behavior: report fields, failure classes (not Display strings), error variants, call orders, requeue counts, and the one-observation-per-generation invariant; the live-host probe test documents its non-degenerate guard (probe.rs:251-256); no `#[ignore]`, no flaky clock/network dependence; registration.rs is the policy-required registration boundary test (provider crate policy)

## Coverage
- idiom=1 | clean | N/A: -
- own=1 | clean | N/A: -
- type: clean (seeds ran: 4/0/0)
- api=1 | clean | N/A: -
- err=2 | clean | N/A: -
- serde: clean (seeds ran: 0/0/0/4)
- obs: N/A (seeds: 0/0/0/0; no tracing/log dependency in Cargo.toml)
- docs=1 | clean | N/A: -
- perf: clean (seeds ran: 2/13/1)
- conc: clean (seeds ran: 0/7/11/0)
- async: clean (seeds ran: 112/0/14/19)
- unsafe: N/A (seeds: 0/0/0/0; manifest `unsafe_code = "forbid"` alone does not make the lens applicable)
- ffi: N/A (seeds: 0/0/0/0)
- macro: N/A (seeds: 0/0/0/0)
- test: clean (seeds ran: 23/71/0/0)