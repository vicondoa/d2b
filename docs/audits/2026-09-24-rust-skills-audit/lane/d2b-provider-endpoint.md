# d2b-provider-endpoint - d2b-provider-endpoint
Baseline: 6ebdd4cec | LOC audited: 2534 (excl. src/generated/**; incl. tests/**) | modules: whole crate (`src/lib.rs`, `src/driver.rs`, `src/endpoint.rs`, `src/effects_service.rs`, `src/facets.rs`, `src/test_support.rs`, `tests/registration.rs`)
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: none

## idiom
- d2b-provider-endpoint#1 sev=low blast=leaf effort=S verdict=actionable - hand-written `impl Default for EndpointConsumerPolicy` returns `Self::unrestricted()`, which the field-wise derive would produce identically (empty Vecs) - fix: add `Default` to the derive list on `EndpointConsumerPolicy` and drop the manual impl - [packages/d2b-provider-endpoint/src/endpoint.rs:395-398]
  evidence: idiom seed `impl (Default|From|PartialEq|Eq|Debug|Clone|Hash) for` = 1 hit (endpoint.rs:395)
- d2b-provider-endpoint#2 sev=medium blast=leaf effort=S verdict=actionable - the `inspect-endpoint` payload table hardcodes the four committed purposes and their producer/locality/class strings, duplicating the same-module derivations `guest_control_producer`/`device_worker_endpoint_class` built from the provider constants, so a provider role/purpose rename drifts the report silently - fix: build the table from those fns, or constrain it with a unit test pinning the payload rows to the derivations - [packages/d2b-provider-endpoint/src/effects_service.rs:50-60,73-84,128-143]
  evidence: static cross-read: payload rows at effects_service.rs:132-137 repeat the values the fns at 50-60,73-84 produce,and no test exercises `inspect_endpoint_response()`
- clean: idiom seeds ran: two/one/zero hits; the two index-loop hits are deliberate `yield_now()` nudge loops in tests,and no statement-style accumulation exists

## own
- clean: own seeds ran: fourteen/eleven/zero/zero hits; every clone/`to_owned` is explainable: `Arc` clones at factory/spawn fences (driver.rs:338,273,57-58), `ResourceRef`/`String` copies riding into `tokio::spawn` (driver.rs:463-464), `error.detail.clone()` inside the borrowed `classify_error`, test-fixture rows, wire-rendering `to_owned` strings in the service payload

## type
- clean: type seeds ran: one/zero/zero hits; the one `check_shape` hit classifies a closed realization set that depends on a dynamic vocabulary trait,not an invariant a parsed type can carry; no boolean-flag soup or stringly-typed state exists

## api
- clean: api seeds ran: sixty-eight/seven/three hits over three seeds; the public surface is the deliberate contract vocabulary (`pub mod endpoint` + `pub use` arms are loaded by d2bd,display-wayland,volume-binding - census: `provider_endpoint::endpoint::` over packages = more than ten hits),andthe only `Arc<...>` in public positions are the composition-supplied facet seams whose ownership is genuinely shared (factory clones `EndpointEffectFacets` per driver at driver.rs:338; daemon builds once at d2bd/resource_plane_v3.rs:2125)

## err
- clean: err seeds ran: about forty hits over four seeds; every `unwrap`/`expect`/`panic!` sits in `#[cfg(test)]` fixtures over literal-built values; the one production `let _ =` (driver.rs:484) drops the effect-completion `send` as best-effort at resource teardown (the receiver dies with the context,so the report has no consumer); the two `String`-returning effect seams are report-only,re-wrapped into `FailureDetail` notes

## serde
- d2b-provider-endpoint#3 sev=medium blast=leaf effort=S verdict=actionable - `EndpointAttachmentPolicy`'s derived `Deserialize` admits illegal state (`supported=false, max_attachments>0`,andthe converse), while its sibling `EndpointConsumerPolicy` routes `Deserialize` through `Self::new` as an admission gate,so a standalone deserializer yields a shape the constructor refuses - fix: route `EndpointAttachmentPolicy::deserialize` through `Self::new` (try_from or the sibling hand-written pattern at endpoint.rs:197-216) - [packages/d2b-provider-endpoint/src/endpoint.rs:112-120,125-131,197-216,377-381]
  evidence: serde seeds one-two=about forty-four hits; the derive at endpoint.rs:112-113 andthe sibling hand-written gate at endpoint.rs:197-216

## obs
- N/A: obs seeds ran: zero/zero/zero/zero all zero; the crate has no `tracing`/`log` dependency and no stdout telemetry

## docs
- d2b-provider-endpoint#4 sev=low blast=leaf effort=S verdict=actionable -the pub Result-returning constructors lack the canonical `# Errors` section naming which condition produces which `EndpointSpecError` variant - fix: add `# Errors` blocks to `EndpointAttachmentPolicy::new`, `EndpointConsumerPolicy::new`,and `EndpointSpec::new`,each enumerated briefly - [packages/d2b-provider-endpoint/src/endpoint.rs:124-125,152-153,254-255]
  evidence: docs seed two (`/// # (Examples|Errors|Panics|Safety))`) = zero hits while seed three (`-> Result<`) about six hits in pub items

## perf
- clean: perf seeds ran: about fourteen hits over three seeds; all `format!` sites are one-shot error reports (driver.rs:385,effects_service.rs:219) or test fixtures;`Vec::new()` sites are test rows andthe `unrestricted()` policy;`to_string()` sites are test fixtures - no hot-path allocation identified (static, unmeasured)

## conc
- clean: conc seeds ran: about thirteen hits over four seeds; the only locks/atomics are the `cfg(test)`/test-support recording doubles (`parking_lot::Mutex` + `AtomicBool`),with `SeqCst` on scripted flags - repo recorded false positive (test-only synchronization; atomics as counters),and every lock site carries the recorded `async-gate-allow` marker

## async
- clean: async seeds ran: about one hundred two hits over four seeds; no guard held across an `.await`, no blocking work inside async contexts (the two `parking_lot` locks in test-support are marker-allowed synchronous acquisitions with no await while held),the spawned long-effect task captures `Send` values and reports through an unbounded mpsc,andthe evidence-wait loop uses async `sleep` inside a `tokio::time::Instant` deadline - cancellation-safe (the wait is resumable on retry)

## unsafe
- N/A: unsafe seeds ran: zero/zero/zero/zero all zero; no `unsafe` block/fn/impl and no `unsafe_code` manifest allowance (local lints table: `unsafe_code = "forbid"`)

## ffi
- N/A: ffi seeds ran: zero/zero/zero/zero all zero; no extern/C/repr/CStr surface exists

## macro
- N/A: macro seeds ran: zero/zero/zero/zero all zero; no `macro_rules!`,proc-macro,or `$crate` use occurs

## test
- d2b-provider-endpoint#5 sev=low blast=leaf effort=S verdict=actionable -the two long-effect tests wait a fixed sixteen-`yield_now()` budget for the spawned task to report through the double,instead of polling the observable recorded call - fix: replace each fixed loop with a bounded poll over `fake.call_order().contains("ensure-socket")` (async yield or small sleep until it appears,then assert) - [packages/d2b-provider-endpoint/src/driver.rs:824-827,1248-1251]
  evidence: test seed two (`assert_eq!|assert_ne!|assert!`) about sixty of the eighty-one test-seed hits; rows at driver.rs:825,1249 are the fixed-budget waits

## Coverage
- idiom: two finding(s)
- own: clean (seeds ran: fourteen/eleven/zero/zero; clones/to_owned all explainable: Arcs at fences,spawn copies,test rows,wire-rendering strings)
- type: clean(seeds ran: one/zero/zero; check_shape classifies a dynamic closed set,not a typestate-invariant)
- api: clean(seeds ran: sixty-eight/seven/three; public surface is deliberate contract vocabulary with loaded `endpoint::` consumers; Arc seams share ownership at the composition root; no leaking internals found)
- err: clean(seeds ran: about forty hits over four seeds; panics are test-only; the one swallowed send is best-effort at teardown; String effect errors are report-only notes)
- serde: one finding(s)
- obs: N/A (seeds: zero/zero/zero/zero; no tracing/log dep in Cargo.toml)
- docs: one finding(s)
- perf: clean(seeds ran: about fourteen hits over three seeds; all allocation sites are error paths,fixtures,andthe unrestricted policy; static inspection only)
- conc: clean(seeds ran: about thirteen hits over four seeds; only test-support/cfg(test) locks+atomics with async-gate-allow markers - recorded repo false positives)
- async: clean(seeds ran:about one hundred two hits over four seeds; no guard-across-await,blocking work,or unbounded growth; spawned effect path is Send and marker-allowed)
- unsafe: N/A (seeds: zero/zero/zero/zero; no unsafe blocks or allow manifests; local lints: unsafe_code=forbid)
- ffi: N/A (seeds: zero/zero/zero/zero; no extern/C/repr/CStr surface)
- macro: N/A (seeds: zero/zero/zero/zero; no macro_rules!,proc-macro,or $crate use)
- test: one finding(s)
