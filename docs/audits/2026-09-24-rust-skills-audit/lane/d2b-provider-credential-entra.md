# d2b-provider-credential-entra - d2b-provider-credential-entra
Baseline: 6ebdd4cec22e6537d1376e83ff7a82b00a8492c0 | LOC audited: 5,246 (src 2,655 + tests 2,591, excl. src/generated/**, none present) | modules: whole crate (lib.rs, controller.rs [audit.rs, telemetry.rs via #[path]], service.rs, main.rs; tests: common, canary, conformance, controller, delivery, entrypoint, faults, lifecycle, placement)
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: single-part lane

## idiom
- d2b-provider-credential-entra#1 sev=medium blast=family effort=M verdict=actionable - in-crate deadline trio (`operation_deadline`/`time_bound_instant`/`time_bounds_not_after`/`time_bound_instant_at`/`is_expired_unix_ms`) duplicates the toolkit's `credential::operation_deadline` with identical absolute-or-relative semantics; the family finding that folded this trio onto the toolkit was applied to secret-service but not here - fix: fold the trio onto `d2b_provider_toolkit::credential::{operation_deadline, deadline_remaining, now_unix_ms, is_absolute_unix_ms}` (keep the injectable-clock `time_bound_instant_at` only if the tests need it), deleting lib.rs:1164-1220 - [packages/d2b-provider-credential-entra/src/lib.rs:1164, packages/d2b-provider-credential-entra/src/lib.rs:1172, packages/d2b-provider-credential-entra/src/lib.rs:1187, packages/d2b-provider-toolkit/src/credential.rs:111, docs/explanation/over-engineering-audit-record.md:872]
  evidence: seeds 0/0/0; direct duplicate read at lib.rs:1164-1220 vs toolkit credential.rs:111-130; not-applied row U54 (over-engineering-audit-record.md:872) - site confirmed present at baseline
- clean: seeds `for \w+ in 0\.\.` = 0, `impl (Default|From|PartialEq|Eq|Debug|Clone|Hash) for` = 0, `let mut \w+ = (String|Vec)::new\(\)` = 0; the six hand-written `Debug` impls are secret-redacting (`<redacted>`) and deliberate, the hand-written `Display`/`Error` impls carry wire codes, and no index loops or statement-style accumulation exist

## own
- clean: seeds `\.clone\(\)` = 31, `\.to_owned\(\)|\.to_vec\(\)|\.to_string\(\)` = 11, `Rc<|RefCell<|Arc<Mutex<|Arc<RwLock<` = 0, `Cow<` = 0; every clone/to_owned inspected is owned construction from borrowed data at a boundary (EntraLeaseRef/LeaseRecord/DeliveryResponse construction, single-flight map keys at controller.rs:264-294, map keys at lib.rs:1074) and none silences a borrow-checker fight

## type
- clean: seeds `fn validate_\w+|fn check_\w+` = 2 (validate_zone lib.rs:666, validate_endpoint_generation lib.rs:965 - runtime-state checks against placement, not parse-once candidates), `is_\w+: bool|\w+_flag: bool` = 0, `(mode|kind|state): String` = 0; the `visibility: &str` parameter of `EntraEndpointPolicy::new` mirrors the Endpoint schema's closed wire values `owner|provider|zone` (ADR-046-provider-credential-entra.md:254-260,535-539), the `zone_ref`/`login_endpoint_ref` Option pair is constrained by three private-field constructors, and config is parse-once (`OpaqueAzureRef::parse` in `EntraConfig::new`)

## api
- d2b-provider-credential-entra#2 sev=medium blast=leaf effort=S verdict=actionable - `pub fn controller_binary_entrypoint()` is an exact duplicate of `run_from_fd10()` with zero callers anywhere in the workspace; the sibling credential crate deleted its twin as production-dead - fix: delete controller_binary_entrypoint (lib.rs:102-105) and its doc comment, keeping `run_from_fd10` as the single entrypoint - [packages/d2b-provider-credential-entra/src/lib.rs:102, packages/d2b-provider-credential-entra/src/lib.rs:103]
  evidence: seed 1 `\bpub (fn|struct|enum|trait|type|const|mod) ` = 73 hits; census: `controller_binary_entrypoint` over packages/ nixos-modules/ tests/ docs/reference/ labs/ = 1 hit (its own definition); precedent row 261 deleted the secret-service twin (docs/explanation/over-engineering-audit-record.md:261)
- d2b-provider-credential-entra#3 sev=low blast=leaf effort=S verdict=actionable - `EntraCredentialOwner` enum and the `owner()` accessor have no caller inside or outside the crate - fix: delete the enum (lib.rs:445-449) and `owner()` (lib.rs:939-942), or wire them into the toolkit's dispatch/controller surface if the ownership policy is meant to be observable - [packages/d2b-provider-credential-entra/src/lib.rs:446, packages/d2b-provider-credential-entra/src/lib.rs:940]
  evidence: census: `EntraCredentialOwner` over packages/ nixos-modules/ tests/ docs/reference/ labs/ = 2 hits (definition + owner() body), zero external callers
- clean: seed 2 `pub .*\b(Arc|Rc|Box|RefCell)<` = 2 (Arc<dyn EntraCredentialClient> in Factory::new - genuine shared ownership; Pin<Box<dyn Future>> in the EntraFuture alias - required for the object-safe dyn client trait), seed 3 `pub use` = 1 (lib.rs:33, the house single-surface arm); all other pub items are consumed by main.rs, tests, or the toolkit runtime

## err
- clean: seeds `\.unwrap\(\)|\.expect\(` = 16 (every hit inside a `#[cfg(test)]` module - audit.rs:51, controller.rs:320-343, lib.rs:1363-1386, service.rs:786, telemetry.rs:50), `let _ = |\.ok\(\);` = 0, `\bpanic!\(|\bunreachable!\(|\btodo!\(|\bunimplemented!\(` = 0, `enum \w*Error` = 2 (EntraClientError, EntraProviderError - closed variants, Display strings are stable wire codes, mapped to CredentialServiceErrorCode in map_client_error); no panic site is reachable from caller input in src

## serde
- N/A: seeds `derive\([^)]*(De)?[Ss]erialize` = 0, `serde\((rename_all|deny_unknown_fields|try_from|untagged|flatten|default|skip_serializing_if)` = 0, `impl .*Deserialize.*for` = 0, `serde_json::from_|serde_json::to_` = 0; the crate crosses the wire only through `serde_json::json!` payload construction in GuestEntraClient (lib.rs:200-360) with no derives or deserializers of its own

## obs
- clean: seeds `\bprintln!\(|\beprintln!\(` = 0, `(info|debug|warn|error|trace)!\("` = 0, `\.instrument\(|#\[instrument` = 0, `tracing::|log::` = 35; every event uses named fields (`provider =`, `resource =`, `%error`, `operation =`) with lazy field expressions, errors are logged once at the mapping boundary (map_client_error lib.rs:1224), and no secret or identifier reaches a field (canary tests pin this)

## docs
- d2b-provider-credential-entra#4 sev=low blast=leaf effort=S verdict=actionable - no public item carries a canonical `# Errors` section although ~30 pub items return `Result` (EntraEndpointPolicy::new, EntraConfig::new, EntraPlacement::new/new_in_zone/new_runtime_in_zone, EntraCredentialProviderFactory::new, revoke_owned_handles, reject_*); `#![deny(missing_docs)]` guarantees presence, not the failure contract - fix: add `# Errors` sections naming the returned error variant (e.g. InvalidConfig, InvalidPlacement, InvalidEndpoint, InvalidConsumer, DeadlineExceeded) to the Result-returning pub constructors - [packages/d2b-provider-credential-entra/src/controller.rs:47, packages/d2b-provider-credential-entra/src/lib.rs:502, packages/d2b-provider-credential-entra/src/lib.rs:565, packages/d2b-provider-credential-entra/src/lib.rs:1049]
  evidence: seed 1 `^\s*pub (fn|struct|enum|trait|const|type)` = 73 hits, seed 2 `/// # (Examples|Errors|Panics|Safety)` = 0 hits, seed 3 `-> Result<` = 30 hits; missing_docs is denied at lib.rs:7 so every pub item is documented, but zero canonical sections exist
- clean: module docs present on lib.rs, controller.rs, service.rs, audit.rs, telemetry.rs; pub-item docs are one-line contract sentences (no implementation narration, no design journals)

## perf
- clean: seeds `format!\(` = 2 (both in `#[cfg(test)]` canary tests), `Vec::new\(\)|VecDeque::new\(\)|HashMap::new\(\)|BTreeMap::new\(\)` = 3 (BTreeMap::new in construct lib.rs:875-877 - empty-case-common, bounded by MAX_LOCAL_LEASES), `\.to_string\(\)` = 0; no allocation site sits on a hot path; `to_canonical_string()` key computation is per-operation and bounded (static, unmeasured)

## conc
- clean: seeds `std::thread::|thread::spawn|thread::scope` = 0, `\bMutex<|\bRwLock<` = 5 (four tokio::sync::Mutex maps + one std::sync::Mutex<()> mutation_gate), `Atomic\w+|Ordering::` = 0, `thread_local!|unsafe impl (Send|Sync) for` = 0; the std Mutex is touched only via `try_lock` on the synchronous dispatch path (mutation_guard lib.rs:1152, no await), the tokio mutexes are async-aware, and the mutation-gate serialization is deliberate and test-pinned (concurrent_acquires_issue_once)

## async
- clean: seeds `async fn|async move|\.await` = 101, `tokio::spawn|spawn_blocking|JoinSet|select!\(|join!\(` = 0, `tokio::sync::(Mutex|RwLock|Notify)` = 4, `#\[tokio::(main|test)\]|Runtime::block_on` = 1 (tests/entrypoint.rs, with sanctioned allow); every client await is bounded by `tokio::time::timeout` via await_client (service.rs:742) or ensure_client_ready_async (service.rs:648), no guard is held across an await (statement-scoped locks), no blocking call sits in an async context, and cancellation ambiguity (uncommitted grants, Draining lifecycle) is deliberately designed and covered by faults.rs/lifecycle.rs

## unsafe
- N/A: seeds `\bunsafe \{|\bunsafe fn|\bunsafe impl|\bunsafe extern` = 0, `// SAFETY:` = 0, `transmute|from_raw|MaybeUninit|mem::zeroed` = 0; seed 4 `unsafe_code` = 1 is the `#![forbid(unsafe_code)]` attribute (lib.rs:8) which per the card does not make the lens applicable

## ffi
- N/A: seeds `extern "C"|no_mangle|unsafe\(link_section` = 0, `catch_unwind` = 0, `repr\(C\)|repr\(transparent\)` = 0, `CStr|CString|c_char` = 0; no FFI surface exists in this crate

## macro
- N/A: seeds `macro_rules!` = 0, `proc_macro|syn::|quote!` = 0, `\$crate` = 0, `to_compile_error|new_spanned` = 0; no macros defined or used beyond std macros

## test
- d2b-provider-credential-entra#5 sev=low blast=leaf effort=S verdict=actionable - `exact_consumer_guard_is_independent_of_request_fields` never exercises the guard its name claims: it only asserts that two `ResourceRef::parse` results differ, which can fail only if parsing collapses distinct inputs - fix: replace the body with an assertion on the actual guard (e.g. `provider.authorizes_consumer(&ResourceRef::parse("Provider/runtime-azure-container-apps").unwrap())` true and a different Provider ref false), or delete the test - [packages/d2b-provider-credential-entra/src/lib.rs:1361, packages/d2b-provider-credential-entra/src/lib.rs:1362]
  evidence: seed 1 `#\[test\]|#\[tokio::test\]` = 50 hits, seed 2 `assert_eq!\(|assert_ne!\(|assert!\(` = 176 hits, seed 3 `proptest!|insta::assert|rstest` = 0, seed 4 `#\[ignore\]` = 0; body read at lib.rs:1361-1366
- clean: the remaining suite asserts behavior and error codes (never Display strings), is deterministic (FakeEntraClient with injected state, no network, real-thread concurrency test with wakers, deadline test with a never-completing client and recv_timeout), and includes canary tests pinning that secrets never reach any rendered surface

## Coverage
- idiom: 1 finding(s)
- own: clean (seeds ran: 31/11/0/0)
- type: clean (seeds ran: 2/0/0)
- api: 2 finding(s)
- err: clean (seeds ran: 16 all cfg(test)/0/0/2)
- serde: N/A (seeds: 0/0/0/0 all zero; no serde derives or deserializers, json! payload construction only)
- obs: clean (seeds ran: 0/0/0/35)
- docs: 1 finding(s)
- perf: clean (seeds ran: 2/3/0)
- conc: clean (seeds ran: 0/5/0/0)
- async: clean (seeds ran: 101/0/4/1)
- unsafe: N/A (seeds: 0/0/0 all zero; only the forbid(unsafe_code) attribute, which does not make the lens applicable)
- ffi: N/A (seeds: 0/0/0/0 all zero; no FFI surface)
- macro: N/A (seeds: 0/0/0/0 all zero; no macros)
- test: 1 finding(s)