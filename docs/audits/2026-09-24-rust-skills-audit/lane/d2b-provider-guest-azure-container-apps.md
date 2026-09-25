# d2b-provider-guest-azure-container-apps - d2b-provider-guest-azure-container-apps
Baseline: 6ebdd4cec22e6537d1376e83ff7a82b00a8492c0 | LOC audited: 2420 (excl. src/generated/**) | modules: whole crate (lib.rs, controller.rs, effects.rs; tests/provider_lifecycle.rs)
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test, supply | Partitions: none (single-part lane)

## idiom
- clean: seeds `for \w+ in 0\.\.`=0, `impl (Default|From|PartialEq|Eq|Debug|Clone|Hash) for`=0, `let mut \w+ = (String|Vec)::new\(\)`=0 over src; no index loops, no hand-written derives (the TryFrom impls delegate to validated constructors), no statement-style accumulation; the one `for index in 0..2` loop lives in tests and is a bounded retry driver, not an index idiom.

## own
- d2b-provider-guest-azure-container-apps#1 sev=low blast=leaf effort=S verdict=actionable - CompletedOperationLedger::record evicts the oldest entry by cloning the map key only to hand it to BTreeMap::remove, which accepts a borrowed key - fix: drop the `.map(|(operation_id, _)| operation_id.clone())` and call `self.completed.remove(&oldest)` directly on the `&AcaOperationId` that `iter().min_by_key)...)` yields - [src/controller.rs:156-157]
  evidence: seed `\.clone\(\)` = 44 hits over src; BTreeMap::remove takes `&Q where K: Borrow<Q>`, so the eviction path needs zero clones.
- d2b-provider-guest-azure-container-apps#2 sev=low blast=leaf effort=S verdict=actionable - reconcile_observed clones the whole owned `record` parameter into `self.observed` and then matches on it, although only the Copy `lifecycle` field is read after the store - fix: `let lifecycle = record.lifecycle; self.observed = Some(record); match lifecycle { ... }` - [src/controller.rs:500-501]
  evidence: seed `\.clone\(\)` = 44 hits over src; the match arms read only `record.lifecycle` (Copy), so moving the record and extracting the field first removes the clone.
- d2b-provider-guest-azure-container-apps#3 sev=low blast=leaf effort=S verdict=actionable - in the Suspended/Stopped arm of reconcile_observed the owned `record` parameter is dead after the resume closure is built, yet `record.id.clone()` copies the id instead of moving it out - fix: `let id = record.id;` (partial move) before the `move` closure - [src/controller.rs:527]
  evidence: seed `\.clone\(\)` = 44 hits over src; `record` is not referenced after line 527 in that arm (later reads go through `self.observed`/`resumed`), so the move compiles.
- d2b-provider-guest-azure-container-apps#4 sev=low blast=leaf effort=S verdict=actionable - the stop and delete stages clone the entire observed `AcaSandboxRecord` (`self.observed.clone().ok_or)...)?`) although the closures consume only `record.id` - fix: clone just the id (`self.observed.as_ref().ok_or)...)?.id.clone()`) and move that into the closure - [src/controller.rs:417-419, src/controller.rs:469-471]
  evidence: seed `\.clone\(\)` = 44 hits over src; both closures call `stop_sandbox`/`delete_sandbox` with `&record.id` only, so the record-level clone copies the id String plus Copy fields needlessly.
- d2b-provider-guest-azure-container-apps#5 sev=low blast=leaf effort=S verdict=actionable - one_candidate and one_disk_image clone the single match out of a slice pattern although they own the `candidates` parameter and return an owned record - fix: consume with `let mut it = candidates.into_iter(); match (it.next(), it.next()) { (Some(c), None) => Ok(Some(c)), (None, None) => Ok(None), _ => Err)...) }` - [src/controller.rs:892, src/controller.rs:903]
  evidence: seed `\.clone\(\)` = 44 hits over src; both helpers take `AcaSandboxCandidates`/`AcaDiskImageCandidates` by value and every caller passes a freshly returned value, so an into_iter consumption removes both clones.
- d2b-provider-guest-azure-container-apps#6 sev=low blast=leaf effort=S verdict=actionable - AcaProviderConfig::validate() re-clones all 11 fields to re-run the constructor checks, when every check is readable from `&self` - fix: extract a private `fn validate_refs(&self) -> Result<(), AcaTypeError>` holding the resource_type() comparisons and call it from both `new` (on the raw args) and `validate` (on self) - [src/effects.rs:450-464]
  evidence: seed `\.clone\(\)` = 44 hits over src; lines 451-462 clone gateway_execution_ref, tenant_id, client_id, subscription_id, control_credential_ref, pull_credential_ref, environment_id, resource_group_id, network_ref, sandbox_transport_alias, defaults solely to rebuild the struct the admission boundary already validated.

## type
- d2b-provider-guest-azure-container-apps#7 sev=medium blast=leaf effort=M verdict=actionable - AcaProviderConfig exposes all 11 fields `pub` while its sibling validated configs (AcaRuntimeConfig, AcaSandboxProfile, AcaReadinessPolicy) keep fields private behind constructors, so a literal construction bypasses the execution-boundary validation that `new()`/`validate()` enforce - fix: privatize the fields and add accessors (network_ref, sandbox_transport_alias, defaults are read in-crate at controller.rs:940-946; no external field reads exist) - [src/effects.rs:394-406]
  evidence: census: `AcaProviderConfig` over packages+tests+docs/reference+labs+nixos-modules = 20 hits, all constructions via `::new()` (packages/d2b-provider-guest/src/driver.rs:1942, packages/d2b-provider-guest/src/effects_service.rs:1807) or `serde_json::from_value` (effects_service.rs:1145); literal `AcaProviderConfig {` constructions = 0 real sites (the two regex matches are the struct definition and an accessor brace); field reads only in-crate.

## api
- d2b-provider-guest-azure-container-apps#8 sev=low blast=leaf effort=S verdict=actionable - lib.rs re-exports the whole effects module via `pub use effects::*;` (so every future pub item in effects silently becomes public API) and effects.rs re-exports four dependency types (`CredentialLeaseHandle`, `OpaqueAzureRef`, `ResourceRef`, `ResourceUid`) with zero consumers through this crate's path - fix: replace the glob with named arms listing the intended effect surface and drop the uncalled dependency-type re-exports - [src/lib.rs:14, src/effects.rs:8-9]
  evidence: census: `guest_azure_container_apps::(CredentialLeaseHandle|OpaqueAzureRef|ResourceRef|ResourceUid)` over packages+tests+labs+nixos-modules+docs/reference = 0 hits; callers import the contracts-crate paths directly (packages/d2b-provider-guest/src/driver.rs:1944), so the re-exports are surface without consumers.

## err
- clean: seeds `\.unwrap\(\)|\.expect\(`=1 (controller.rs:539 `expect("stored above")` asserts the invariant just stored on the previous line - acceptable per the skill's invariant-panic channel), `let _ = |\.ok\(\);`=0, `\bpanic!\(|\bunreachable!\(|\btodo!\(|\bunimplemented!\(`=0, `enum \w*Error`=3 (AcaControllerError, AcaTypeError, AcaControlErrorKind); the taxonomies split by caller action with stable `code()` strings, AcaControlError wraps a private kind per the struct pattern, and no wire-visible error enum is restructured.

## serde
- clean: seeds ran: 31 combined hits (derive Serialize/Deserialize, serde attributes, hand-written Deserialize, serde_json calls); every wire type validates through `try_from` (RawAca* shapes and numeric bounds via TryFrom delegating to the validated constructors), `deny_unknown_fields` is applied to all config raws, `rename_all = "camelCase"` everywhere, and the only hand-written Deserialize impls are the opaque_id admission gates (recorded refusal class per U1 (d) 6 - not re-flagged); a real-payload round-trip test exists at effects.rs:886.

## obs
- clean: seeds `\bprintln!\(|\beprintln!\(`=0, interpolated message-only events=0, `\.instrument\(|#\[instrument`=0, `tracing::`=15; all 15 warn!/debug! events carry named fields (resource, provider, code, purpose, lifecycle, attempt), messages are static literals, and the hand-written Debug impls (opaque_id, AcaProviderConfig, AcaSandboxRecord) redact identity material so no secret reaches a field.

## docs
- d2b-provider-guest-azure-container-apps#9 sev=low blast=leaf effort=M verdict=actionable - `#[allow(missing_docs)]` blanket-exempts the effects module whose pub surface (AcaControl and AcaCredentialLeaseClient trait methods, AcaProviderConfig fields, Aca*Error enums, MAX_ACA_* constants) is re-exported at the crate root, undercutting the crate's own `#![deny(missing_docs)]` - fix: document the effect trait methods and validated-config accessors and drop the module-level allow (README.md already carries the prose contract, so this is rustdoc-surface work, not a contract gap) - [src/lib.rs:7, src/effects.rs:813-882]
  evidence: docs seed 1 (`^\s*pub (fn|struct|enum|trait|const|type)`) = 87 hits over src, the large majority inside the allow-exempted effects module; seed 2 (`/// # (Examples|Errors|Panics|Safety)`) = 0.
- d2b-provider-guest-azure-container-apps#10 sev=low blast=leaf effort=M verdict=actionable - Result-returning pub items carry no `# Errors` section naming their failure conditions (AcaController::reconcile and finalize, AzureContainerAppsRuntimeProvider::new, the validated constructors AcaProviderConfig::new/validate, AcaCpuMillis::new, AcaMemoryMib::new, the opaque_id parse) - fix: add `# Errors` sections listing the AcaTypeError/AcaControllerError conditions each returns - [src/controller.rs:257, src/controller.rs:308, src/controller.rs:924, src/effects.rs:61, src/effects.rs:106, src/effects.rs:128, src/effects.rs:410, src/effects.rs:450]
  evidence: docs seed 3 (`-> Result<`) = 40 hits over src; seed 2 (`/// # (Examples|Errors|Panics|Safety)`) = 0 canonical sections anywhere in the crate.

## perf
- clean: seeds `format!\(`=0, `Vec::new\(\)|VecDeque::new\(\)|HashMap::new\(\)|BTreeMap::new\(\)`=0, `\.to_string\(\)`=0 over src; no allocation sites in the reconcile path beyond the required owned-effect payloads, and the clone-heavy spots are cold (per-interval reconcile, admission boundary) - static (unmeasured).

## conc
- N/A (seeds: `std::thread::|thread::spawn|thread::scope`=0, `\bMutex<|\bRwLock<`=0, `Atomic\w+|Ordering::`=0, `thread_local!|unsafe impl (Send|Sync) for`=0 all zero; no threads, locks, atomics, or manual Send/Sync in src - the crate is single-task async).

## async
- clean: seeds ran: 51 combined hits (async fn/async move/.await, async_trait effect ports); every provider call is wrapped in `timeout_at(deadline, ...)` with a deadline derived once from `deadline_remaining_ms` (controller.rs:676-677), no blocking work sits inside an async context, no guard is held across an await (no Mutex in src), no spawn/spawn_blocking exists, and shared state is limited to Arc effect ports with genuine multi-controller ownership (AzureContainerAppsRuntimeProvider::controller shares Arc<C>/Arc<L> across per-Guest controllers).

## unsafe
- N/A (seeds: `\bunsafe \{|\bunsafe fn|\bunsafe impl|\bunsafe extern`=0, `// SAFETY:`=0, `transmute|from_raw|MaybeUninit|mem::zeroed`=0; seed 4 alone - `#![forbid(unsafe_code)]` at src/lib.rs:4 plus the manifest's local `unsafe_code = "forbid"` - does not make the lens applicable per the card).

## ffi
- N/A (seeds: `extern "C"|no_mangle|unsafe\(link_section`=0, `catch_unwind`=0, `repr\(C\)|repr\(transparent\)`=0, `CStr|CString|c_char`=0 all zero; the crate crosses no foreign boundary).

## macro
- clean: seeds `macro_rules!`=1 (effects.rs:54), `proc_macro|syn::|quote!`=0, `\$crate`=0, `to_compile_error|new_spanned`=0; the single `opaque_id!` macro is the genuine impl-per-type case (8 ID newtypes with identical parse/as_str/Deserialize/Debug shapes), uses narrow fragment specifiers (ident, expr), needs no `$crate` (it references only std macros and its own parameters), and is by-example, not procedural.

## test
- d2b-provider-guest-azure-container-apps#11 sev=low blast=leaf effort=S verdict=actionable - stable_error_codes_are_bounded ends with a dead `let _ = ResourceRef::parse("Guest/gateway").unwrap();` that asserts nothing the test name claims and duplicates parse coverage exercised everywhere else - fix: delete the line (or fold the parse into an assertion the test actually promises) - [tests/provider_lifecycle.rs:536]
  evidence: test seeds: `#\[test\]|#\[tokio::test\]`=14, `assert_eq!\(|assert_ne!\(|assert!\(`=28, `proptest!|insta::assert|rstest`=0, `#\[ignore\]`=0; the stray line is the only statement in the suite whose result is discarded.
- d2b-provider-guest-azure-container-apps#12 sev=medium blast=leaf effort=S verdict=actionable - the completed-operation ledger replay path (reconcile with a previously recorded operation id returns Converged without re-running effects, controller.rs:264-266) is contract behavior with no test - every test calls reconcile with a fresh operation id - fix: add a test that reconciles twice with the same id against a Running sandbox and asserts the second pass performs no effect calls (calls list unchanged) - [src/controller.rs:264-266, tests/provider_lifecycle.rs:210-225]
  evidence: test seeds: 14 tests, 0 reuse an operation id across reconcile calls (all ids are unique per test, including the loop in readiness_attempts_are_bounded which formats fresh ids); the ledger replay branch is therefore never exercised.

## supply
- d2b-provider-guest-azure-container-apps#13 sev=low blast=leaf effort=S verdict=actionable - the `sha2` dependency is unused: the name appears nowhere in src/ or tests/ (only in Cargo.toml:21 and as the prose word "digests" in README.md:51) - fix: remove `sha2 = { workspace = true }` from the crate manifest (the workspace dep stays for its other consumers) - [Cargo.toml:21]
  evidence: census: `sha2|Sha2|Sha256|digest` over packages/d2b-provider-guest-azure-container-apps/src + tests = 0 hits; the only manifest mention is Cargo.toml:21.

## Coverage
- idiom: clean (seeds ran: 0/0/0)
- own: 6 finding(s)
- type: 1 finding(s)
- api: 1 finding(s)
- err: clean (seeds ran: 1/0/0/3)
- serde: clean (seeds ran: 31 combined)
- obs: clean (seeds ran: 0/0/0/15)
- docs: 2 finding(s)
- perf: clean (seeds ran: 0/0/0)
- conc: N/A (seeds: 0/0/0/0 all zero; no threads, locks, atomics, or manual Send/Sync in src)
- async: clean (seeds ran: 51 combined; all effect calls bounded by timeout_at deadlines, no blocking work, no guards across awaits, no spawn sites)
- unsafe: N/A (seeds: 0/0/0; seed 4 alone - `#![forbid(unsafe_code)]` at src/lib.rs:4 - does not make the lens applicable)
- ffi: N/A (seeds: 0/0/0/0 all zero; no foreign boundary)
- macro: clean (seeds ran: 1/0/0/0)
- test: 2 finding(s)
- supply: 1 finding(s)