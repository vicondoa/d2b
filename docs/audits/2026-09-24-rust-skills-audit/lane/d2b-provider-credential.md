# d2b-provider-credential - d2b-provider-credential
Baseline: 6ebdd4cec22e6537d1376e83ff7a82b00a8492c0 | LOC audited: 3322 (excl. src/generated/**, none present) | modules: whole crate (driver.rs, session.rs, effects_service.rs, facets.rs, test_support.rs, lib.rs; tests/registration.rs for the test lens)
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: whole crate

## idiom
- clean: seeds 1-3 (`for \w+ in 0\.\.`, `impl (Default|From|PartialEq|Eq|Debug|Clone|Hash) for`, `let mut \w+ = (String|Vec)::new\(\)`) all 0 hits over src/; no index loops, no hand-written derives (the only manual impls are Debug redaction impls on CredentialRevocationRequest/Evidence and Display/Error impls, all deliberate), no statement-style accumulation.

## own
- d2b-provider-credential#1 sev=low blast=leaf effort=S verdict=actionable - dead derives: `#[derive(Clone)]` on `CredentialDriver` and `#[derive(Default)]` on `RecordingRuntime` are never used by any call site - fix: drop `Clone` from `CredentialDriver` (driver.rs:342) and `Default` from `RecordingRuntime` (test_support.rs:178), keeping `RecordingRuntime::new` as the only constructor - [packages/d2b-provider-credential/src/driver.rs:342, packages/d2b-provider-credential/src/test_support.rs:178]
  evidence: seed `\.clone\(\)` = 61 hits, all explainable (owned returns, Arc clones at factory/effects boundaries, test-support recorders); census: `CredentialDriver` with `.clone()` over packages/ = 0 hits (d2bd uses only `CredentialDriverArgs`/`credential_descriptor`, resource_plane_v3.rs:2968); census: `RecordingRuntime::default` over packages/ = 0 hits (d2bd calls `RecordingRuntime::new`, resource_plane_v3.rs:3812, shared_provider_effects.rs:3361)
- clean: seeds 1-4 (`.clone\(\)` 61, `.to_owned\(\)|\.to_vec\(\)|\.to_string\(\)` 31, `Rc<|RefCell<|Arc<Mutex<|Arc<RwLock<` 10, `Cow<` 0) read in full; every production clone/to_owned buys a real owned value (decoded-spec envelope at driver.rs:376 is required because `ctx.spec` returns `&T`, context.rs:439; factory `create` clones args per row at driver.rs:330-332; `Arc<dyn CredentialSession>`/`Arc<dyn CredentialRuntime>` share genuine ownership across the composition root and factory); remaining Arc<Mutex> sites are test-support/test fixtures only.

## type
- d2b-provider-credential#2 sev=low blast=leaf effort=M verdict=actionable - `CredentialDriverArgs.zone: String` (and the mirrored `CredentialDriver.zone: String`) carries a bare string where the daemon already holds a validated `ZoneId`, so the driver re-derives and re-parses the zone at use sites instead of receiving the invariant - fix: change `zone` to `d2b_contracts_resource::v3::ZoneId` in `CredentialDriverArgs` (driver.rs:295) and `CredentialDriver` (driver.rs:344); the daemon construction site drops `inputs.zone.as_str().to_owned()` and passes `inputs.zone` (resource_plane_v3.rs:2969, where `ConstructionInputs.zone: ZoneId` at resource_plane_v3.rs:1784); `agent_child` then builds `format!("Zone/{}", self.zone.as_str())` without the fallible `ResourceRef::parse` failure path (driver.rs:457-461); test fixtures switch to `ZoneId::parse("dev").unwrap()` - [packages/d2b-provider-credential/src/driver.rs:295, packages/d2b-provider-credential/src/driver.rs:457, packages/d2bd/src/resource_plane_v3.rs:2969]
  evidence: seed `(mode|kind|state): String` = 0 hits (seed 2 `fn validate_\w+` matched only two test fn names, driver.rs:1314,1334); the zone string is validated nowhere until `agent_child`'s `ResourceRef::parse` (driver.rs:461), and never for the non-managed-identity providers whose rows skip that path; sibling family args structs (BindingDriverArgs/EndpointDriverArgs/VolumeDriverArgs, resource_plane_v3.rs:2956-2975) share the same String pattern (X3 candidate)
- clean: seeds 1-3 ran (2 hits total, both test fn names); no boolean flag soup, no Option-pair states, no stringly-typed state; `CredentialDriverStatus`/`CredentialRevocationOutcome`/`CredentialDriverErrorKind` are proper enums and `CredentialRevocationRequest` keeps its derived identity fields private behind accessors (session.rs:84-181).

## api
- clean: seeds 1-3 (`\bpub (fn|struct|enum|trait|type|const|mod) ` 85, `pub .*\b(Arc|Rc|Box|RefCell)<` 5, `^\s*pub use ` 2) read against the full public surface; lib.rs re-exports are the single house surface (lib.rs:36-53), modules stay private, `CredentialDriverError` is a struct with a private kind (driver.rs:141), `Arc<dyn CredentialRuntime>` in `CredentialEffectFacets.runtime` (facets.rs:54) and `Arc<dyn SpecDecoder>` from `credential_spec_decoder` (driver.rs:215) are deliberate shared-ownership/registry patterns with cited call sites (composition root resource_plane_v3.rs:3812, factory effects_service.rs:164, test doubles), and the test-support exports are feature-gated (lib.rs:33-34).

## err
- clean: seeds 1-4 ran (`.unwrap\(\)|\.expect\(` 33, `let _ = |\.ok\(\);` 0, `\bpanic!\(|\bunreachable!\(|\btodo!\(|\bunimplemented!\(` 4, `enum \w*Error` 1); every unwrap/expect/unreachable sits in `#[cfg(test)]` modules or the `test-support`-gated recorder (test_support.rs:140), no production panic site; `CredentialDriverError` is the struct-with-private-kind shape (driver.rs:141-170) with a `class()` split by caller action (Terminal vs Retryable, driver.rs:112-120), `CredentialResourceRuntimeError` has two variants callers match on (session.rs:29-47), and the Display strings are the documented old durable codes, not asserted as the contract anywhere outside deliberate tests.

## serde
- clean: seeds 1-4 ran (derive 0, serde-attr 0, hand-written Deserialize 0, `serde_json::from_|serde_json::to_` 14); the crate derives no wire types (CredentialSpec/ResourceSpec come from the contracts crates), the spec decoder parses through the canonical `typed_spec_decoder` path (driver.rs:215-223), the inspect-credential payload is built through the canonical JSON object path (effects_service.rs:66-80), and `canonical_bytes` round-trips through `CanonicalJsonValue` (driver.rs:718-723).

## obs
- clean: seeds 1-4 ran (`\bprintln!\(|\beprintln!\(` 0, interpolated `(info|debug|warn|error|trace)!\("` 0, `\.instrument\(|#\[instrument` 0, `tracing::` 2); the two production events (driver.rs:682, 692) are structured with named fields (`credential`, `outcome`, `session_generation`), carry no interpolated secrets (the redacted `operation_id` is deliberately not logged; Debug impls redact at session.rs:108-126, 253-265), and the error path logs once at the handling boundary.

## docs
- d2b-provider-credential#3 sev=low blast=leaf effort=S verdict=actionable - the two Result-returning public items lack the canonical `# Errors` section: `CredentialRevocationRequest::new` states its failure condition only in prose (session.rs:128-131) and `CredentialSession::revoke_credential` documents no failure conditions at all (session.rs:273-274) - fix: add `# Errors` sections naming `CredentialResourceRuntimeError::InvalidResource` (zero/unknown session generation, zero rotation generation, foreign Provider) and the `Revocation` variant respectively - [packages/d2b-provider-credential/src/session.rs:132, packages/d2b-provider-credential/src/session.rs:275]
  evidence: seed `/// # (Examples|Errors|Panics|Safety)` = 0 hits across src/ while `-> Result<` = 9 hits; `#![deny(missing_docs)]` (lib.rs:29) keeps every pub item documented, so this is the canonical-section shape only
- clean: seeds 1-3 ran (pub items 102, canonical sections 0, `-> Result<` 9); module docs present in all six files, first sentences carry the contract, no `ignore`d doctests exist, and the crate is `#![deny(missing_docs)]` (lib.rs:29).

## perf
- clean: seeds 1-3 ran (`format!\(` 12, `Vec::new\(\)|VecDeque::new\(\)|HashMap::new\(\)|BTreeMap::new\(\)` 8, `\.to_string\(\)` 31); all format!/Vec::new sites are cold paths (per-pass agent/credential ref derivation driver.rs:420,431,457, the durable operation-id preimage session.rs:188-200, error-path and test-recorder strings) with no loop bodies, and no collection is grown in a hot path; static (unmeasured).

## conc
- d2b-provider-credential#4 sev=medium blast=leaf effort=S verdict=policy-confirmed - `parking_lot::Mutex` is used throughout the test-support recorder and test fixtures (test_support.rs:18,30,41-46,97-113,159,233-249; driver.rs:1053,1074-1075,1109-1172; session.rs:315,429) despite the recorded outright ban whose only exception is the R4 bounded-worker boundary, and the impl methods holding most `.lock()` calls carry no per-site `#[allow(clippy::disallowed_methods)]` even though the test fns do (`reason = "cfg(test) helper"`, the sanctioned form) - fix: switch the recorder locks to `tokio::sync::Mutex` per the clippy.toml replacement column, or record a test-support exception in the policy and add the sanctioned per-site allows to the impl methods; the `// async-gate-allow: test-support recorder lock` markers (30 sites, async-gate-inventory.json) are recorded exceptions and are not re-flagged - [packages/d2b-provider-credential/src/test_support.rs:18, packages/d2b-provider-credential/src/test_support.rs:97, packages/d2b-provider-credential/src/driver.rs:1109, clippy.toml:40, clippy.toml:82]
  evidence: seed `\bMutex<|\bRwLock<` = 12 hits, all in test-support/test context (production code holds no lock); census: blocking-census-baseline.json counts `parking_lot::Mutex::lock` = 0 for this crate (test context is excluded from the production ratchet); the same family-wide test-support pattern appears in d2b-provider-device, -endpoint, -guest, -network-local, -process, -usbip, -activation-nixos test_support modules (X3 candidate)
- clean: seeds 1-4 ran (`std::thread::|thread::spawn|thread::scope` 0, `\bMutex<|\bRwLock<` 12, `Atomic\w+|Ordering::` 2, `thread_local!|unsafe impl (Send|Sync) for` 0); production code uses no threads, locks, or atomics; the only atomics are a Relaxed test counter (session.rs:430,444) and all synchronization is test-only (card false positive), with the parking_lot choice flagged above.

## async
- clean: seeds 1-4 ran (`async fn|async move|\.await` 137, `tokio::spawn|spawn_blocking|JoinSet|select!\(|join!\(` 0, `tokio::sync::(Mutex|RwLock|Notify)` 0, `#\[tokio::(main|test)\]|Runtime::block_on` 21); no spawn/select/join in the crate, no guard held across an `.await` (the driver's revoke/reconcile awaits at driver.rs:647,669,860,887 hold no lock), no blocking call inside an async context, and every test-support recorder lock carries its recorded `// async-gate-allow` marker (30 sites in async-gate-inventory.json) - cited, not re-flagged.

## unsafe
- N/A (seeds: `\bunsafe \{|\bunsafe fn|\bunsafe impl|\bunsafe extern` 0, `// SAFETY:` 0, `transmute|from_raw|MaybeUninit|mem::zeroed` 0, `unsafe_code` 0; manifest `unsafe_code = "forbid"` with no exception sites - the (d)8 exception set does not include this crate).

## ffi
- N/A (seeds: `extern "C"|no_mangle|unsafe\(link_section` 0, `catch_unwind` 0, `repr\(C\)|repr\(transparent\)` 0, `CStr|CString|c_char` 0; the crate crosses no foreign boundary).

## macro
- N/A (seeds: `macro_rules!` 0, `proc_macro|syn::|quote!` 0, `\$crate` 0, `to_compile_error|new_spanned` 0; no macros defined).

## test
- clean: seeds 1-4 ran over src/ + tests/ (`#\[test\]|#\[tokio::test\]` 29, `assert_eq!\(|assert_ne!\(|assert!\(` 128, `proptest!|insta::assert|rstest` 0, `#\[ignore\]` 0); the suite is behavior-focused (revocation-before-child-deletion ordering, fail-closed session binding, durable operation-id dedup, drift deletion, registration boundary), asserts error classes and status variants rather than implementation, is deterministic with no network/time dependence, and every test can fail (no tautologies, no `#[ignore]`, no golden pinning).

## Coverage
- idiom: clean (seeds ran: 0/0/0)
- own: 1 finding(s)
- type: 1 finding(s)
- api: clean (seeds ran: 85/5/2)
- err: clean (seeds ran: 33/0/4/1)
- serde: clean (seeds ran: 0/0/0/14)
- obs: clean (seeds ran: 0/0/0/2)
- docs: 1 finding(s)
- perf: clean (seeds ran: 12/8/31)
- conc: 1 finding(s)
- async: clean (seeds ran: 137/0/0/21)
- unsafe: N/A (seeds: 0/0/0/0 all zero; no unsafe blocks, manifest forbids)
- ffi: N/A (seeds: 0/0/0/0 all zero; no foreign boundary)
- macro: N/A (seeds: 0/0/0/0 all zero; no macros)
- test: clean (seeds ran: 29/128/0/0)