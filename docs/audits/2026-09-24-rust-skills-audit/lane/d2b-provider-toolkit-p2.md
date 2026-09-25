# d2b-provider-toolkit-p2 - d2b-provider-toolkit - part 2/2
Baseline: 6ebdd4cec22e6537d1376e83ff7a82b00a8492c0 | LOC audited: 10495 (excl. src/generated/**; src 6937 + tests 3558) | modules: testing/**, server/**, shared_provider.rs, credential.rs, service.rs, lib.rs
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: part 2/2 per U1 (f): src/testing/**, src/server/**, src/shared_provider.rs, src/credential.rs, src/service.rs, src/lib.rs

## idiom
- clean: seeds ran: 0/1/1 (`for \w+ in 0\.\.` = 0; `impl (Default|From|PartialEq|Eq|Debug|Clone|Hash) for` = 1; `let mut \w+ = (String|Vec)::new\(\)` = 1). The one hand-written `impl Default for DispatchLimiter` (server/adapter.rs:448) is justified: `Arc<AtomicUsize>` is not derivable and the manual impl preserves the frozen-ceiling invariant; the one `Vec::new()` accumulation (testing/mod.rs:900) is a recursive reference collector where an iterator pipeline would obscure the recursion.

## own
- d2b-provider-toolkit-p2#1 sev=low blast=family effort=S verdict=actionable - serve_component_session clones all four fields of the owned decoded request per frame (`request.zone().clone(), request.provider_ref().clone(), request.method().clone(), request.payload().clone()`) instead of moving them out - fix: destructure `let ProviderRequest { request_id, zone, provider_ref, method, payload } = request;`, pass the owned values to `dispatch_for_route`, and call `codec.encode_response(&request_id, &response)` - [packages/d2b-provider-toolkit/src/server/adapter.rs:331-334]
  evidence: seed `\.clone\(\)` = 76 hits in scope; the four clones at adapter.rs:331-334 are the only ones on an owned request that could be moves (the request is decoded into an owned value at adapter.rs:325 and used only through the loop).
- d2b-provider-toolkit-p2#2 sev=low blast=family effort=S verdict=actionable - the session loop clones the bound route out of the async mutex twice per frame (`self.authenticated_route.lock().await.clone()` at loop entry and per iteration) to compare identities - fix: compare inside the lock scope, e.g. `if self.authenticated_route.lock().await.as_ref() != Some(&route)`, avoiding the per-frame `AuthenticatedSessionRouteBinding` clone - [packages/d2b-provider-toolkit/src/server/adapter.rs:305, packages/d2b-provider-toolkit/src/server/adapter.rs:314-316]
  evidence: seed `\.clone\(\)` = 76 hits in scope; the route binding carries context and provider identity, so the two per-frame clones are the largest per-frame copies in the hot loop.
- d2b-provider-toolkit-p2#3 sev=low blast=leaf effort=S verdict=actionable - `retire_obsolete_children` sorts obsolete rows with `sort_by_key` over `(teardown_rank, row.key.name.clone())`, allocating a String per owned row per pass - fix: use `sort_by` with a comparator `teardown_rank(&a.key.type_name).cmp(&teardown_rank(&b.key.type_name)).then_with(|| a.key.name.cmp(&b.key.name))` - [packages/d2b-provider-toolkit/src/shared_provider.rs:771]
  evidence: seed `\.clone\(\)` = 76 hits in scope; this is the only sort-key clone (all other clone sites are Arc clones, snapshot reads, or owned-argument passes required by the async APIs).

## type
- clean: seeds ran: 7 (`fn validate_\w+|fn check_\w+` = 7; `is_\w+: bool|\w+_flag: bool` = 0; `(mode|kind|state): String` = 0). Every hit is a boundary validation over already-parsed types (`check_closed_code_set`, `check_descriptor_conformance`, `check_provider_conformance`, `validate_attachment_indexes`, `validate_bound_request`, `validate_authenticated_provider_request`, `validate_provider_route`); no boolean-flag soup, no stringly-typed state, no Option-pair invariants in the scope.

## api
- d2b-provider-toolkit-p2#4 sev=low blast=leaf effort=S verdict=actionable - two dead public methods on `GeneratedProviderServiceServer`: `response_request_id` is a pure identity function (`request_id` in, same reference out) and `generated_service` has no callers anywhere - fix: delete both methods (and the `response_request_id` doc), keeping `generated_services()` which the registration-boundary doc justifies - [packages/d2b-provider-toolkit/src/server/service.rs:297-299, packages/d2b-provider-toolkit/src/server/service.rs:174-176]
  evidence: census: `response_request_id` over packages/, nixos-modules/, tests/, docs/reference/, labs/ = 1 hit (the definition); `generated_service\(\)` = 0 hits; `generated_services\(\)` = 1 hit (toolkit's own test).
- d2b-provider-toolkit-p2#5 sev=low blast=family effort=S verdict=actionable - `SharedProviderEffectRequest::envelope()` (the old-shape owner-envelope document) has zero callers and clones the full spec and metadata Values on every call - fix: delete the method; the driver and families read `spec`/`metadata` directly - [packages/d2b-provider-toolkit/src/shared_provider.rs:525-531]
  evidence: census: `request\.envelope\(\)` over packages/, nixos-modules/, tests/, docs/reference/, labs/ = 0 hits; the method is not in the lib.rs re-export list but is pub on a pub struct.
- d2b-provider-toolkit-p2#6 sev=low blast=family effort=S verdict=actionable - `TestHarness::clock()` returns `&Arc<DeterministicClock>`, exposing the Arc in the public signature when callers only need the clock - fix: return `&DeterministicClock` (callers at testing/mod.rs:686 and tests/harness.rs:857-909 all deref) - [packages/d2b-provider-toolkit/src/testing/mod.rs:530-532]
  evidence: seed `pub .*\b(Arc|Rc|Box|RefCell)<` = 4 hits in scope; the other three (spec decoder return, `family` field, `created_children`) genuinely share ownership, this one does not.

## err
- d2b-provider-toolkit-p2#7 sev=medium blast=family effort=M verdict=actionable - `SharedProviderDriver::new` panics via `ZoneId::parse(args.zone).expect("driver zone was validated at construction")`, but `SharedProviderDriverArgs.zone` is a plain pub `String` with no validation anywhere at the factory boundary, so a family passing an invalid zone crashes the provider process at driver construction - fix: hold `ZoneId` in `SharedProviderDriverArgs` (parse once in `SharedProviderDriverFactory::new` and return a `Result`), or make `create` fallible - [packages/d2b-provider-toolkit/src/shared_provider.rs:615, packages/d2b-provider-toolkit/src/shared_provider.rs:539-546]
  evidence: seed `\.unwrap\(\)|\.expect\(` = 90 hits in scope (about 70 in `#[cfg(test)]`/tests); census: `SharedProviderDriverArgs` is constructed at d2b-provider-device-security-key/src/driver.rs:327, d2b-provider-device-usbip/src/driver.rs:217, d2b-provider-device/src/driver.rs:296, d2b-provider-network-local/src/driver.rs:270, all passing a caller-supplied zone String; the expect's claimed invariant is not enforced by the type.
- d2b-provider-toolkit-p2#8 sev=low blast=family effort=S verdict=actionable - `key_ref` panics via `expect("manager keys carry canonical resource references")` on a `ResourceKey` whose fields are pub and unvalidated (`ResourceKey::new` accepts any strings), so the pub helper can panic on a non-canonical key a caller constructs - fix: return `Result<ResourceRef, SharedProviderEffectError>` (map to `InvalidResource`) like the sibling `owner_ref`/`resource_uid` helpers - [packages/d2b-provider-toolkit/src/shared_provider.rs:405-408, packages/d2b-resource-runtime/src/spec_store.rs:60-63]
  evidence: seed `\.unwrap\(\)|\.expect\(` = 90 hits in scope; census: `key_ref\(` = 20 call sites across packages (manager-derived keys today, so the invariant holds in practice, but the type does not enforce it).
- d2b-provider-toolkit-p2#9 sev=medium blast=family effort=S verdict=actionable - `Fixture::method` maps `SpecifiedProviderMethod` with a `_ => unreachable!("specified Provider method is closed")` arm, but the enum is `#[non_exhaustive]` (d2b-contracts-provider/src/v3/provider.rs:2759), so any future contract variant becomes a runtime panic in every fixture-based test suite - fix: return `Result<ProviderMethodName, ProviderToolkitError>` and map unknown methods to `WireInvalid`, updating the two call sites (fixture.rs:190 and the `ProviderAgentService` impl) - [packages/d2b-provider-toolkit/src/testing/fixture.rs:181-192]
  evidence: seed `\bpanic!\(|\bunreachable!\(|\btodo!\(|\bunimplemented!\(` = 1 hit in scope (this site); the enum it matches is declared `#[non_exhaustive]`, which forces the `_` arm and makes the panic reachable from a contract extension.
- d2b-provider-toolkit-p2#10 sev=medium blast=family effort=S verdict=actionable - every fake port swallows the recorder-capacity error with `let _ = self.recorder.record(...)`, so `FakePortError::RecorderFull` is never constructed (dead variant in the public closed set `ALL`) and a test exceeding `MAX_RECORDED_CALLS` silently truncates its record, contradicting the variant's own doc "the call is refused rather than dropped silently" - fix: map `ProviderToolkitError::CapacityOutOfRange` to `FakePortError::RecorderFull` and return it from the fake methods (or delete the variant and its `ALL` slot) - [packages/d2b-provider-toolkit/src/testing/fakes.rs:275-277, packages/d2b-provider-toolkit/src/testing/fakes.rs:289-291, packages/d2b-provider-toolkit/src/testing/fakes.rs:337-339, packages/d2b-provider-toolkit/src/testing/fakes.rs:391-393, packages/d2b-provider-toolkit/src/testing/fakes.rs:430, packages/d2b-provider-toolkit/src/testing/fakes.rs:464]
  evidence: seed `let _ = |\.ok\(\);` = 10 hits in scope; six are these swallowed `record` results (`FakePortError::RecorderFull` appears only in the enum, its `code()` match, and `ALL`); the recorder's own error path (fakes.rs:194-198) is unreachable from any fake port.

## serde
- clean: seeds ran: 0/0/0/4 (`derive(...Serialize/Deserialize)` = 0; `serde(...)` attributes = 0; hand-written Deserialize = 0; `serde_json::from_|to_` = 4). The four sites are Value-level decodes with explicit object validation at the boundary (shared_provider.rs:369-372, 385); no typed wire types are defined in this scope, so there is no deserialization-into-domain-type surface to judge.

## obs
- d2b-provider-toolkit-p2#11 sev=low blast=family effort=S verdict=actionable - three message-only `warn!` events in the authenticated session loop carry no named fields even though zone/provider/method are in scope at each site, so the events are not queryable per provider - fix: add fields, e.g. `warn!(zone = ?session.route_binding().zone(), "component session receive failed; closing provider session")` and the analogous provider/method fields at the readiness and loop-failure sites - [packages/d2b-provider-toolkit/src/server/session.rs:135, packages/d2b-provider-toolkit/src/server/session.rs:255, packages/d2b-provider-toolkit/src/server/session.rs:262]
  evidence: seeds: `\bprintln!\(|\beprintln!\(` = 0; `(info|debug|warn|error|trace)!\(` = 36 hits, of which 33 already carry named fields and 3 are message-only with no enclosing span to inherit fields from (`\.instrument\(|#\[instrument` = 0 in scope).

## docs
- d2b-provider-toolkit-p2#12 sev=low blast=leaf effort=S verdict=actionable - no public item in the scope carries the canonical `# Errors` section even though many return `Result` with closed, non-obvious failure sets (`check_descriptor_conformance` has ten `ConformanceError` variants; `operation_deadline` fails on exhausted deadlines; `validate_attachment_indexes` fails on non-monotone indexes) - fix: add `# Errors` sections naming the variant per condition to the Result-returning pub items, starting with conformance.rs:267, conformance.rs:291, credential.rs:111, credential.rs:131, adapter.rs:31 - [packages/d2b-provider-toolkit/src/testing/conformance.rs:267, packages/d2b-provider-toolkit/src/testing/conformance.rs:291, packages/d2b-provider-toolkit/src/credential.rs:111, packages/d2b-provider-toolkit/src/credential.rs:131, packages/d2b-provider-toolkit/src/server/adapter.rs:31]
  evidence: seeds: `^\s*pub (fn|struct|enum|trait|const|type)` = 150 hits (all documented; `#![deny(missing_docs)]` is on); `/// # (Examples|Errors|Panics|Safety)` = 0 hits; `-> Result<` = 30 hits.

## perf
- d2b-provider-toolkit-p2#13 sev=medium blast=family effort=M verdict=actionable - every reconcile and delete pass clones the row's full spec document (`spec: envelope.value().clone()` at shared_provider.rs:944 and 1024) into the request even though the envelope outlives the effect call and the request is only read by the family - fix: change `SharedProviderEffectRequest.spec` from `Value` to `&'a Value` (the struct is constructed only in this file; family call sites read via method calls and auto-deref), removing one full-spec allocation per pass - [packages/d2b-provider-toolkit/src/shared_provider.rs:944, packages/d2b-provider-toolkit/src/shared_provider.rs:1024, packages/d2b-provider-toolkit/src/shared_provider.rs:479-481]
  evidence: static (unmeasured); seed `format!\(` = 20 hits and `Vec::new\(\)` = 15 hits in scope, but the spec clone is the only per-pass allocation proportional to spec size (the pass runs on every resync cadence and every change); the `operation_id` format! per pass is small and not flagged.

## conc
- clean: seeds ran: 40 (`std::thread::|thread::spawn|thread::scope` = 0; `\bMutex<|\bRwLock<` = 4; `Atomic\w+|Ordering::` = 36; `thread_local!|unsafe impl (Send|Sync) for` = 1). Atomics use correct orderings (Acquire/Release pairs on the limiter and server state, Relaxed on the delivery-sequence counter, AcqRel in `DeterministicClock::advance`); `shutdown` arms the `Notify` before checking in-flight (the clippy.toml-sanctioned pattern); the `thread_local!` runtime in credential.rs is the sanctioned synchronous-path allow.

## async
- clean: seeds ran: many (`async fn|async move|\.await` throughout; `tokio::spawn` = 1 at server/mod.rs:68; `tokio::sync::(Mutex|RwLock|Notify)` = 3; `#\[tokio::(main|test)\]|Runtime::block_on` = 0). No blocking work inside async contexts (the one `block_on` is the documented synchronous dispatch half with the sanctioned `reason = "synchronous path"` allow); `ContextChildSurface` holds a `tokio::sync::Mutex` guard across the context's own awaits (async-aware, never across another effect call); both session loops are cancellation-aware; the spawn/yield/is_finished immediate-failure check in `serve_authenticated_route` aborts cleanly on error paths.

## unsafe
- N/A: seeds: `\bunsafe \{|\bunsafe fn|\bunsafe impl|\bunsafe extern` = 0; `// SAFETY:` = 0; `transmute|from_raw|MaybeUninit|mem::zeroed` = 0; `unsafe_code` = 0 (manifest carries `unsafe_code = "forbid"` under `[lints.rust]`).

## ffi
- N/A: seeds: `extern "C"|no_mangle|unsafe\(link_section` = 0; `catch_unwind` = 0; `repr\(C\)|repr\(transparent\)` = 0; `CStr|CString|c_char` = 0. No foreign boundary in this scope.

## macro
- N/A: seeds: `macro_rules!` = 0; `proc_macro|syn::|quote!` = 0; `\$crate` = 0; `to_compile_error|new_spanned` = 0. No macro definitions in this scope.

## test
- clean: seeds ran: 55 in src + 90 in tests (`#\[test\]|#\[tokio::test\]` = 20 src + 14 tests; `assert_eq!\(|assert_ne!\(|assert!\(` = 35 src + 76 tests; `proptest!|insta::assert|rstest` = 0; `#\[ignore\]` = 0). Tests assert behavior and error variants rather than Display strings, expectations are human-written or independent (`br#"..."#` literals, closed-code sets), the shared statics (`SEEN_INVOCATIONS`, `TEMP_FILE_SEQUENCE`) are cleared or unique per test so runs stay deterministic, and the `#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]` sites use the sanctioned reason.

## Coverage
- idiom: clean (seeds ran: 0/1/1; hand-written Default justified, no index loops)
- own: 3 finding(s)
- type: clean (seeds ran: 7/0/0; all hits are boundary validations over parsed types)
- api: 3 finding(s)
- err: 4 finding(s)
- serde: clean (seeds ran: 0/0/0/4; Value-level decode with explicit object validation)
- obs: 1 finding(s)
- docs: 1 finding(s)
- perf: 1 finding(s)
- conc: clean (seeds ran: 0/4/36/1; correct atomic orderings, sanctioned Notify pattern)
- async: clean (seeds ran: many/2/3/0; no blocking in async contexts, cancellation-aware loops)
- unsafe: N/A (seeds: 0/0/0/0; manifest forbids unsafe_code)
- ffi: N/A (seeds: 0/0/0/0 all zero)
- macro: N/A (seeds: 0/0/0/0 all zero)
- test: clean (seeds ran: 34 tests + 111 assertions; behavior and error-variant assertions, deterministic)