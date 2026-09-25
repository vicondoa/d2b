# d2b-provider-credential-secret-service - d2b-provider-credential-secret-service
Baseline: 6ebdd4cec | LOC audited: 6,699 (excl. src/generated/**, none present) | modules: lib.rs, service.rs, controller.rs, audit.rs, telemetry.rs, main.rs; tests: session.rs, lifecycle.rs, faults.rs, canary.rs, delivery.rs, conformance.rs, entrypoint.rs, placement.rs, common/mod.rs
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: whole crate

## idiom
- d2b-provider-credential-secret-service#1 sev=low blast=leaf effort=S verdict=actionable - exported enum variant is misspelled `Userd` for `User` in the only supported owner classification, so every consumer must copy the typo - fix: rename `SecretServiceOwner::Userd` to `SecretServiceOwner::User` (and `owner()` return at lib.rs:1233); in-tree census shows no consumers to update - [packages/d2b-provider-credential-secret-service/src/lib.rs:446, packages/d2b-provider-credential-secret-service/src/lib.rs:1233]
  evidence: census: `Userd` over packages/, nixos-modules/, tests/, docs/reference/, labs/, BUILD.bazel = 2 hits (definition + owner() return, both in-crate)
- clean: seeds ran: 0/0/0; no index loops (`for i in 0..`),no hand-written Default/From/PartialEq/Eq/Clone/Hash impls (the hand-written `Debug` impls deliberately redact via `<redacted>`/`write_str`, where a derive would leak),no statement-style `let mut String/Vec::new()` accumulation; the only naming defect is the typo above

## own
- clean: seeds ran: ~80/~4/0/0; every clone/is explainable one sentence: `Arc` clones feed `'static` port futures and shared port ownership (GuestCredentialBackend, `SessionAuthority` self-clone for capability ownership),owned field moves build injected port requests and Mutex/BTreeMap keys (`lease_key`, `user_ref`, `credential_ref`, idempotency),cfg(test) fixtures clone canary markers;`to_owned()` sites (`lib.rs:170,406,1618`) feed owned struct fields/opaque handles; no Rc/RefCell/Cow; the deadline-helper and env-scan triplication classes arerecorded refusals (refusal ledger U53: folded onto toolkit) and are not re-flagged
- evidence: seeds ran: ~80 (`.clone()` mostly cfg(test) module in lib.rs and test suites)/~4 (`.to_owned()/to_vec()/to_string()`)/0 (`Rc<|RefCell<|Arc<Mutex<|Arc<RwLock<`)/0 (`Cow<`); spot-checked via `packages/d2b-provider-credential-secret-service/src` + `tests`

## type
- clean: seeds ran: 2/0/0;`validate_binding` (lib.rs:950) and`validate_delivery` (service.rs:80) are boundary checks on already-typed references (SessionBinding fields, delivery/request/consumer refs) rather than validate-at-every-callsite; no boolean-flag soup, no dual-Option pairs, no stringly-typed state;`LockPolicy`/`SecretServiceState`/`CredentialLeaseState`-style closed enums carry the state
- evidence: seeds: `fn validate_\w+|fn check_\w+` = 2 hits, `is_\w+: bool|\w+_flag: bool` = 0, `(mode|kind|state): String` = 0

## api
- clean: seeds ran: ~62/0/1; public surface is single-path: `pub use` re-export arm at lib.rs:37 exposes controller module items; otherwise pub types are closed structs with private fields plus doc'd accessors/constructors (SecretServiceConfig, SecretServicePlacement, SecretServiceLeaseRequest/Ref, factory, provider, capability,error enums, Oo7SecretServicePort trait with defaulted recovery methods); the only dependency type in a pub signature is `Arc<dyn Oo7SecretServicePort>` on the factory, where the value genuinely shares ownership (provider keeps it, tests and runtime construct different ports; call sites: lib.rs:171-178 factory construction, tests/common/mod.rs:187-188)`; no Arc/Rc/Box/RefCell leaks beyond it
- evidence: seeds: `\bpub (fn|struct|enum|trait|type|const|mod) ` ~62 hits, `pub .*\b(Arc|Rc|Box|RefCell)<` = 0 (Arc<dyn port> appears in fn args not `pub .* <` form), `^\s*pub use ` = 1

## err
- clean: seeds ran: ~50/~2/0/4; all unwrap/expect sites are in `#[cfg(test)]` module/tests (incl. `expect("plain-test admission runtime")` at lib.rs:1296); the four `*Error` enums split by caller action (port errors closed and mapped to wire codes, provider construction errors, crate-private SessionAuthorityError, crate-private SecretServicePollError)and are not wire-visible (wire error codes come from `d2b_core` via toolkit); no non-test panics in src; the deadline/env-scan helper structure already folded onto `d2b-provider-toolkit` (refusal ledger U53) - this crate delegates via `operation_deadline`/`deadline_remaining` and`reject_process_environment_credential_chain`, not re-flagged
- evidence: seeds: `\.unwrap\(\)|\.expect\(` ~50 (all test code), `let _ = |\.ok\(\);` ~2, `\bpanic!\(|\bunreachable!\(|\btodo!\(|\bunimplemented!\(` = 0 in src, `enum \w*Error` = 4

## serde
- N/A (seeds: 0/0/0/0; no serde derives/attrs/hand-written Deserialize impls and no `serde_json::from_/to_` in src; this crate crosses no serde boundary itself - wire shapes live in d2b-contracts-provider/toolkit, and the port only shapes `serde_json::json!` request payloads for `GuestCredentialBackend`; serde_json appears only in tests (canary.rs:150))

## obs
- d2b-provider-credential-secret-service#2 sev=medium blast=leaf effort=S verdict=actionable - session-close failure warn is message-only with no named fields, so a fleet cannot filter which provider/session close left unresolved leases - fix: add `provider = crate::PROVIDER_REF` and the two booleans (`unresolved_leases`, `unresolved_operations`)as fields to the `tracing::warn!` - [packages/d2b-provider-credential-secret-service/src/service.rs:860]
  evidence: obs seed 2 (`(info|debug|warn|error|trace)!\(\("`): ~33 warn/error events, of which exactly 1 has no fields (service.rs:860; the other events carry provider/operation/user/resource/%error fields); seed1 `println!/eprintln!` = 0, seed3 instrument =  0, seed4 tracing:: =~33
- clean: every other event uses named fields (`provider`, `operation`, `user`, `resource`, `state`, `%error`), never a secret (Debug impls redact, telemetry frames pass `validate_collector_fields` canary gate, redaction already gated by ADR 0010/0028 scanner `packages/xtask/src/diagnostic_redaction.rs`)

## docs
- d2b-provider-credential-secret-service#3 sev=medium blast=leaf effort=M verdict=actionable - public Result-returning constructors/projections lack `# Errors` sections naming which condition yields which error (despite `#![deny(missing_docs)]` forcing presence, no canonical section exists anywhere in the crate) - fix: add `# Errors` to `SecretServiceConfig::new`, `SecretServicePlacement::new`, `SecretServiceCredentialProviderFactory::new`, `SecretServiceController::reconcile` (and the remaining pub Result items) naming `SecretServiceProviderError`/`CredentialServiceError` failures - [packages/d2b-provider-credential-secret-service/src/lib.rs:523, packages/d2b-provider-credential-secret-service/src/lib.rs:610, packages/d2b-provider-credential-secret-service/src/lib.rs:1140, packages/d2b-provider-credential-secret-service/src/controller.rs:73]
  evidence: docs seed 2 (`/// # (Examples|Errors|Panics|Safety)`) = 0 across src (no canonical sections),seed 3 (`-> Result<`) ~135 hits covering most pub methods/constructors returning Result
- d2b-provider-credential-secret-service#4 sev=low blast=leaf effort=S verdict=actionable - the one-second session-close revoke deadline is a bare magic `1_000` triplicated with no why (it bounds revoke of potentially many leases during disconnect/finalize/drain) - fix: extract `const SESSION_CLOSE_REVOKE_DEADLINE_MS: u64 = 1_000;` and document why (one-second cap so a stalled backend cannot hang session teardown foreve) - [packages/d2b-provider-credential-secret-service/src/service.rs:660, packages/d2b-provider-credential-secret-service/src/service.rs:674, packages/d2b-provider-credential-secret-service/src/service.rs:684]
  evidence: census: `operation_deadline(1_000)` over src =  3 hits (service.rs:660,674,684; all three sync close paths)
- clean: `#![deny(missing_docs)]` is enabled and pub items carry doc'd first sentences; module docs present on lib/controller/service/audit/telemetry; the capability carries a `compile_fail` doctest (lib.rs:1088-1094), no `ignore`d doctests; the gaps flagged above are the canonical-section and magic-value state

## perf
- clean: seeds ran: ~3/~9/~6;`format!` appears only in cfg(test) canary markers and test rendering (audit.rs:39, telemetry.rs:33, lib.rs:1999,2008),`Vec::new()` only in test double ports (lifecycle.rs:610,705) and cold one-time `BTreeMap::new()`/`BTreeSet::new()` in provider construction (lib.rs:1179-1187),`to_string()` at wire-rendering/test boundaries (canary.rs:146,148); no `format!` or grow-by-push allocation in any hot path; nothing further (static (unmeasured), no benchmark exists for these cold provider paths)
- evidence: seeds: `format!\(` ~3 non-test src (0), `Vec::new\(\)|VecDeque::new\(\)|HashMap::new\(\)|BTreeMap::new\(\)` ~9 (all cold/test), `\.to_string\(\)` ~6 (wire-rendering boundaries/test fixtures)

## conc
- clean: seeds ran: ~2/~8/~6/0; concurrency model is deliberate and workload-shaped: per-provider entry gates (`std::sync::Mutex` `mutation_gate` for sync surface, `tokio::sync::Mutex` `async_mutation_gate` for async dispatch),tokio::sync::Mutex-guarded BTreeMap/BTreeSet state for shared maps, atomics for counters (`next_counter` with Relaxed load + AcqRel CAS, `finalized` Acquire/Release pair;`NEXT_AUTHORITY_ID` process-unique static counter is a deliberate singleton - not flagged); no thread_local/static-mut/unsafe Send-Sync claims; sync-path `blocking_lock()` sites are documented sync-only (never executor workers), with inline reasons
- evidence: seeds: `std::thread::|thread::spawn|thread::scope` ~2 (src: thread::current/park_timeout poll loop, tests spawn), `\bMutex<|\bRwLock<` ~8 (src fields; RwLock 0), `Atomic\w+|Ordering::` ~6, `thread_local!|unsafe impl (Send|Sync) for` = 0

## async
- d2b-provider-credential-secret-service#5 sev=medium blast=leaf effort=S verdict=actionable - lock order between `sessions` and`user_sessions` is inverted across two branches of `authorize_session_for_user_locked` (first branch acquires `sessions` then awaits `user_sessions`; cached-key branch acquires `user_sessions` then awaits `sessions`), a latent tokio-Mutex deadlock that the outer `async_mutation_gate`/entry-timing currently masks - fix: acquire in one consistent order in both branches (`sessions` before `user_sessions`, e.g. in the cached-key branch scope the `user_sessions` guard chain and then lock `sessions`, or collapse the dual lookup into one map) - [packages/d2b-provider-credential-secret-service/src/lib.rs:1323, packages/d2b-provider-credential-secret-service/src/lib.rs:1341, packages/d2b-provider-credential-secret-service/src/lib.rs:1380]
  evidence: async seed 2 (`tokio::sync::(Mutex|RwLock|Notify)`) ~9 hits; the two conflicting order edges are lib.rs:1323-1327 (sessions held across user_sessions await) and lib.rs:1341-1351 (user_sessions held across sessions await)
- clean: production async paths use `tokio::time::timeout` + deadline checks (`ensure_unlocked_async`, `await_port`), await-aware locks, and the port futures are cancellation-bookkept (CompletionUnknown/Deadline arms remember ambiguous operations before returning);`blocking_lock()` sites are sync-only with written reasons (cfg(test) helper, sync public surface, never executor workers);`Runtime::block_on` appears only in cfg(test) helper (lib.rs:1290-1297) with inline allow; no async-gate-allow markers needed
- evidence: seeds: `async fn|async move|\.await` ~100 hits, `tokio::spawn|spawn_blocking|JoinSet|select!\(|join!\(` = 0 in src, `tokio::sync::(Mutex|RwLock|Notify)` ~9,, `#\[tokio::(main|test)\]|Runtime::block_on` ~3 (cfg-test/current-thread test harness only)

## unsafe
- N/A (seeds: 0/0/0/1; sole hit is the `#![forbid(unsafe_code)]` attribute at lib.rs:8; no unsafe blocks/fns/impls, no SAFETY comments, no transmute/from_raw/MaybeUninit/mem::zeroed; safelens non-applicable per U1 card rule)

## ffi
- N/A (seeds: 0/0/0/0; no extern "C"/no_mangle/link_section, catch_unwind, repr(C)/repr(transparent), or CStr/CString/c_char in src; no FFI boundary)

## macro
- N/A (seeds: 0/0/0/0; no macro_rules! definitions, proc-macro/syn/quote, $crate, or to_compile_error/new_spanned uses; all macros used are std/tracing/serde_json built-ins)

## test
- d2b-provider-credential-secret-service#6 sev=low blast=leaf effort=S verdict=actionable - table-driven loops assert without per-case failure messages (`locked_and_unavailable_map_to_provider_unavailable`, `only_user_agent_on_host_or_guest_is_accepted`, `collection_alias_accepts_spaces_and_rejects_unsafe_text`), so a failure reports only the line number and not which case failed - fix: add a `"case: {case:?}"`-style message to each loop assertion - [packages/d2b-provider-credential-secret-service/tests/faults.rs:25, packages/d2b-provider-credential-secret-service/tests/placement.rs:8, packages/d2b-provider-credential-secret-service/src/lib.rs:1966]
  evidence: test seeds: `#\[test\]|#\[tokio::test\]` ~45, `assert_eq!\(|assert_ne!\(|assert!\(` ~130, `proptest!|insta::assert|rstest` = 0, `#\[ignore\]` = 0;; three loops lack per-case messages
- clean: seeds ran: ~45/~130/0/0; suite covers lifecycle, idempotency, faults/mapping, deadlines (NeverPort, DelayedUnlockPort), concurrent admission/close fencing, redaction canaries (audit, telemetry, every rendered surface), entrypoint refusal, dynamic two-user scope, generation/binding/consumer refusal; no ignored tests, no network, seeded nonces via `std::process::id()` keep tests process-unique but deterministic; the per-case-message gap above is the only polish defect

## Coverage
- idiom: 1 finding(s)
- own: clean (seeds ran: ~80/~4/0/0; clones explained above; deadline/env-scan helpers are recorded refusals (U53), not re-flagged)
- type: clean (seeds ran: 2/0/0)
- api: clean(seeds ran: ~62/0/1)
- err: clean(seeds ran: ~50/~2/0/4; all panic sites test-only; error taxonomy split by caller action)
- serde: N/A(seeds: 0/0/0/0; no serde boundary in this crate itself)
- obs: 1 finding(s)
- docs:  2 finding(s)
- perf: clean(seeds ran: ~3/~9/~6; allocation sites cold/test-only)
- conc: clean(seeds ran: ~2/~8/~6/0)
- async: 1 finding(s)
- unsafe: N/A(seeds: 0/0/0/1; forbid attribute only)
- ffi: N/A(seeds:  0/0/0/0)
- macro: N/A(seeds:  0/0/0/0)
- test: 1 finding(s)