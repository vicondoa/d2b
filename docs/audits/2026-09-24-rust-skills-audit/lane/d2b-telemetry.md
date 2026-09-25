# d2b-telemetry - d2b-telemetry
Baseline: 6ebdd4cec22e6537d1376e83ff7a82b00a8492c0 | LOC audited: 1,983 (excl. src/generated/**) | modules: whole crate (audit_hash, emitter, meter_registry, metric_label_policy, redaction_guard, session_metrics_sink, trace_context)
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: whole crate (single-part lane)

## idiom
- clean: seeds `for \w+ in 0\.\.`=0, `impl (Default|From|PartialEq|Eq|Debug|Clone|Hash) for`=0, `let mut \w+ = (String|Vec)::new\(\)`=0; crate declares fns so the lens applies, and no index loops, no hand-written derive-replaceable impls, no statement-style accumulation were found (the hand-written Debug impls on AuditHash, TraceContext, and BoundedEmitter are the deliberate redaction class, and the hand-written Serialize/Deserialize impls are deliberate admission gates).

## own
- clean: seeds `\.clone\(\)`=4, `\.to_owned\(\)|\.to_vec\(\)|\.to_string\(\)`=18, `Rc<|RefCell<|Arc<Mutex<|Arc<RwLock<`=1, `Cow<`=0; every clone is explainable - `Arc<Mutex<State>>` plus `Arc<DropCounters>` are the shared-ownership handles behind `BoundedEmitter`'s `Clone` (emitter.rs:167-168), the key/value clones build owned `BTreeMap`s at the admission boundary (emitter.rs:541, meter_registry.rs:107), the `child_span` trace_id clone is required by the Self-owned fields (trace_context.rs:63), and the rest are test fixtures; no `Rc`/`RefCell`/`Cow` anywhere.

## type
- clean: seeds `fn validate_\w+|fn check_\w+`=3, `is_\w+: bool|\w+_flag: bool`=0, `(mode|kind|state): String`=0; the three validators (`validate_metric_frame` emitter.rs:502, `validate_resource_attributes` metric_label_policy.rs:14, `validate_span_field` redaction_guard.rs:95) are parse-once admission checks on untrusted wire input at the boundary, not validate-at-every-callsite smells, and no boolean-flag soup, stringly-typed state, or `Option`-pair states exist.

## api
- clean: seeds `\bpub (fn|struct|enum|trait|type|const|mod) `=94, `pub .*\b(Arc|Rc|Box|RefCell)<`=0, `^\s*pub use `=9; the surface is deliberate - every pub item is documented, the `Arc` fields on `BoundedEmitter` are private (`socket_path()` returns `&Path`), the `pub mod` + `pub use` shape in lib.rs is the house single-surface pattern (d2b-audit/src/hash_chain.rs:3-6 and d2b-bus/src/metrics.rs:10-13 consume both paths; census: `d2b_telemetry::` over packages/nixos-modules/tests/docs/reference/labs/BUILD.bazel = 16 hits), and the `meter_registry::label` re-export is a live cross-crate API consumed by d2b-bus/src/metrics.rs:12.

## err
- d2b-telemetry#1 sev=medium blast=leaf effort=S verdict=actionable - `BoundedEmitter::new_with_limits` reports invalid constructor arguments as `EmitterError::StatePoisoned`, conflating a permanent programming error with transient lock poisoning - fix: add a dedicated variant (e.g. `InvalidLimits`) and return it from the zero-capacity / zero-frame / zero-age / zero-retry guard clauses, keeping `StatePoisoned` for the `lock().map_err` sites - [packages/d2b-telemetry/src/emitter.rs:201-206, packages/d2b-telemetry/src/emitter.rs:93]
  evidence: seed `enum \w*Error` = 5; the guard clauses at emitter.rs:201-206 return `StatePoisoned`, whose doc comment says "The emitter lock was poisoned" (emitter.rs:92-93); the Display strings are unpinned - census: `telemetry-emitter-state-poisoned` over packages/nixos-modules/tests/docs/labs/BUILD.bazel = 1 hit (its own Display arm) and no telemetry code appears in docs/reference/error-codes.md
- d2b-telemetry#2 sev=low blast=leaf effort=S verdict=actionable - `SessionMetricsError::Encode` is never constructed; the `io::Error` from `encode_frame` is folded into `EmitterError::MetricPolicy(DescriptorMalformed)` inside `emit_metric`, so the variant and its Display arm are dead surface - fix: delete the `Encode(std::io::Error)` variant and the `"session-metric-encode-failed"` Display arm - [packages/d2b-telemetry/src/session_metrics_sink.rs:73, packages/d2b-telemetry/src/session_metrics_sink.rs:82]
  evidence: census: `SessionMetricsError::Encode|Encode\(` over packages/nixos-modules/tests/docs/reference/labs/BUILD.bazel = 2 hits, both self-referential (definition + Display arm); the only encode failure path maps to `EmitterError::MetricPolicy` at emitter.rs:328, so no construction path exists

## serde
- clean: seeds `derive\([^)]*(De)?[Ss]erialize`=3, `serde\((rename_all|deny_unknown_fields|try_from|untagged|flatten|default|skip_serializing_if)`=2, `impl .*Deserialize.*for`=2, `serde_json::from_|serde_json::to_`=5; the hand-written `Deserialize` impls on `AuditHash` (audit_hash.rs:48-56) and `TraceContext` (trace_context.rs:73-87) are live admission gates over private fields (the skill's parse-once pattern), `AuditChainLink` carries `rename_all = "camelCase"` + `deny_unknown_fields`, and `TraceContext`'s hand-written `Serialize` deliberately emits digested identities; no optionality or representation mistakes found.

## obs
- N/A: seeds `\bprintln!\(|\beprintln!\(`=0, `(info|debug|warn|error|trace)!\("`=0, `\.instrument\(|#\[instrument`=0, `tracing::|log::`=0, and Cargo.toml declares no tracing/log dependency; the crate is a synchronous library with no telemetry-emitting surface of its own.

## docs
- d2b-telemetry#3 sev=low blast=leaf effort=M verdict=actionable - Result-returning public items carry no `# Errors` section stating which conditions produce which failure - fix: add `# Errors` sections to `AuditHash::parse` (audit_hash.rs:23), `AuditChainLink::verify`/`verify_at` (audit_hash.rs:103,126), `BoundedEmitter::new`/`new_with_limits`/`with_default_capacity`/`emit`/`emit_metric`/`drain`/`buffered_frames`/`buffered_bytes` (emitter.rs:183,194,228,236,316,340,385,395), `MetricFamily::new`/`record`, `MeterRegistry::register`/`record`, `RedactionGuard::new`/`validate_span_field`/`span_attributes`, `validate_resource_attributes`, and `SessionMetricsSink::record` - [packages/d2b-telemetry/src/emitter.rs:236, packages/d2b-telemetry/src/audit_hash.rs:23]
  evidence: seed `/// # (Examples|Errors|Panics|Safety)` = 0 hits against `-> Result<` = 20 hits; repo-wide `/// # Errors` over packages/ = 0 hits, so this is a proposal, not a house deviation; all pub items themselves are documented and every module carries a `//!` doc

## perf
- clean: seeds `format!\(`=5, `Vec::new\(\)|VecDeque::new\(\)|HashMap::new\(\)|BTreeMap::new\(\)`=4, `\.to_string\(\)`=0; the two production `format!` sites render hash strings in `AuditHash::from_bytes`/`record_hash` (cold construction paths), `hex_lower` pre-sizes with `with_capacity`, and the collection constructors are one-shot constructor-time allocations where the empty case is common; all static (unmeasured), no hot-loop allocation found.

## conc
- clean: seeds `std::thread::|thread::spawn|thread::scope`=1, `\bMutex<|\bRwLock<`=1, `Atomic\w+|Ordering::`=6, `thread_local!|unsafe impl (Send|Sync) for`=1; `Arc<Mutex<State>>` is a `std::sync::Mutex` on a genuinely synchronous path (the sanctioned class), the drop counters use `Relaxed` orderings (the weakest correct ordering for counters nobody synchronizes on), the `thread_local!` and `thread::sleep` are cfg(test) only, and no unsafe `Send`/`Sync` claims or `static mut` exist.

## async
- N/A: seeds `async fn|async move|\.await`=0, `tokio::spawn|spawn_blocking|JoinSet|select!\(|join!\(`=0, `tokio::sync::(Mutex|RwLock|Notify)`=0, `#\[tokio::(main|test)\]|Runtime::block_on`=0; the crate is a synchronous library surface (its own Cargo.toml comment records the frozen no-async-form posture).

## unsafe
- N/A: seeds `\bunsafe \{|\bunsafe fn|\bunsafe impl|\bunsafe extern`=0, `// SAFETY:`=0, `transmute|from_raw|MaybeUninit|mem::zeroed`=0; seed 4 (`unsafe_code`) = 1 hit, the `#![forbid(unsafe_code)]` attribute at lib.rs:6, which per the lens card does not make the lens applicable; the crate manifest carries no `unsafe_code` setting (U1 (d) 8 lists it as ABSENT with no sites).

## ffi
- N/A: seeds `extern "C"|no_mangle|unsafe\(link_section`=0, `catch_unwind`=0, `repr\(C\)|repr\(transparent\)`=0, `CStr|CString|c_char`=0; no foreign boundary exists in the crate.

## macro
- N/A: seeds `macro_rules!`=0, `proc_macro|syn::|quote!`=0, `\$crate`=0, `to_compile_error|new_spanned`=0; no macros are defined or expanded beyond std/derive macros.

## test
- d2b-telemetry#4 sev=medium blast=leaf effort=S verdict=actionable - contract rejection paths have no tests: `BoundedEmitter::new`/`new_with_limits` argument rejections (zero capacity -> StatePoisoned, relative path -> SocketPathInvalid), `AuditChainLink::verify` mismatch variants, `MeterRegistry::register` duplicate-name rejection, `MetricFamily::record` kind/value mismatch rejection, and `RedactionGuard::new` duplicate-key rejection are all untested, so a change that breaks any admission check passes the suite - fix: add unit tests asserting the exact variants, e.g. `assert_eq!(BoundedEmitter::new("relative", 128).unwrap_err(), EmitterError::SocketPathInvalid)`, `link.verify(&other, &payload, &record) == Err(ChainVerificationError::PreviousHashMismatch)`, and `registry.register(duplicate).unwrap_err() == MetricPolicyError::DescriptorMalformed` - [packages/d2b-telemetry/src/emitter.rs:201-210, packages/d2b-telemetry/src/audit_hash.rs:103-116, packages/d2b-telemetry/src/meter_registry.rs:88-96, packages/d2b-telemetry/src/meter_registry.rs:127-133, packages/d2b-telemetry/src/redaction_guard.rs:63-64]
  evidence: seed `#\[test\]|#\[tokio::test\]` = 31 tests, all read; none exercise the listed rejection paths - the suite covers redaction, FIFO order, ring bounds, label policy, and success paths only
- d2b-telemetry#5 sev=low blast=leaf effort=S verdict=actionable - `target_buckets_are_present` pins three bucket constants by asserting one member value each, a tautology that fails on refactor and passes on behavior change - fix: delete the test, or replace it with a behavior test (e.g. a `MetricFamily::new` built with `CONTROLLER_HINT_BUCKETS_SECONDS` accepts an in-range histogram value and rejects an out-of-range one) - [packages/d2b-telemetry/src/meter_registry.rs:176-180]
  evidence: seed `assert_eq!\(|assert_ne!\(|assert!\(` = 77 hits; the test's expected values are the constants themselves (`CONTROLLER_HINT_BUCKETS_SECONDS.contains(&0.005)`), not derived from an independent source

## Coverage
- idiom: clean (seeds ran: 0/0/0)
- own: clean (seeds ran: 4/18/1/0)
- type: clean (seeds ran: 3/0/0)
- api: clean (seeds ran: 94/0/9)
- err: 2 finding(s)
- serde: clean (seeds ran: 3/2/2/5)
- obs: N/A (seeds: 0/0/0/0 all zero; no tracing/log dependency in Cargo.toml)
- docs: 1 finding(s)
- perf: clean (seeds ran: 5/4/0)
- conc: clean (seeds ran: 1/1/6/1)
- async: N/A (seeds: 0/0/0/0 all zero; synchronous library surface)
- unsafe: N/A (seeds: 0/0/0 all zero for the block/fn/impl seeds; seed 4 = 1 hit, the `#![forbid(unsafe_code)]` attribute at lib.rs:6, which does not make the lens applicable)
- ffi: N/A (seeds: 0/0/0/0 all zero; no foreign boundary)
- macro: N/A (seeds: 0/0/0/0 all zero; no macros defined)
- test: 2 finding(s)