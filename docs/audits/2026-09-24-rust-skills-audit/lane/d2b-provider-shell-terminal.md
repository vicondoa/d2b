# d2b-provider-shell-terminal - d2b-provider-shell-terminal
Baseline: 6ebdd4cec22e6537d1376e83ff7a82b00a8492c0 | LOC audited: 4176 (excl. src/generated/**; none present) | modules: whole crate (src: lib, authz, guest_rules, host_rules, migration, observability, resources/{mod,pool,session}, service/{mod,controller,supervisor}, session/{mod,ring,adopt}; tests: 10 files)
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: one (whole crate)

## idiom
- clean: seeds `for \w+ in 0\.\.` 0, `impl (Default|From|PartialEq|Eq|Debug|Clone|Hash) for` 0, `let mut \w+ = (String|Vec)::new\(\)` 0 (all zero; no index loops, no hand-written derive-replaceable impls (the redacted Debug impls are deliberate and not matcheable by the seed), no statement-style accumulation).

## own
- d2b-provider-shell-terminal#1 sev=low blast=leaf effort=S verdict=actionable - `advance_session` clones the whole `Option<SupervisorIdentity>` only to end the first `session_mut` borrow before the retired-identity check; the check can compare the live field inside a scoped block instead. - fix: in `ShellAuthorityLedger::advance_session`, wrap the first `session_mut` borrow in `{ ... }` and compare `entry.supervisor_identity.as_ref() != retired_identity` inside it, dropping `let current_identity` and `.clone()`; keep the second borrow for minting and mutation. - [src/service/supervisor.rs:599, src/service/supervisor.rs:601]
  evidence: `\.clone\(\)` ~20 hits checked (fingerprint snapshots, capability/attachment accessor hand-offs, Arc clones at the genuinely-shared authority port, resource-map key copies, pool-entry inserts - all own required state except this one)
- clean: seeds `\.clone\(\)` ~20 hits, `\.to_owned\(\)|\.to_vec\(\)|\.to_string\(\)` ~16 hits, `Rc<|RefCell<|Arc<Mutex<|Arc<RwLock<` 0, `Cow<` 0; every other clone/to_owned is an owned field, map key, or shared-Arc copy that no single borrow can serve.

## type
- clean: seeds `fn validate_\w+|fn check_\w+` 7, `is_\w+: bool|\w+_flag: bool` 0, `(mode|kind|state): String` 0; validation runs once at the pool/session/request constructors (`validate_name`, `PoolSpec::new`, `ShellSession::from_pool`, `OpenSessionRequest::new`) and the `validate_session` family are state-dependent ledger checks against stored state - not parse-time validation a parsed type could replace.



## api
- clean: seeds pub surface ~160 hits, `pub .*\b(Arc|Rc|Box|RefCell)<` 1, `^\s*pub use ` 14; the one `Arc<dyn ShellAuthorityPort>` signature (`ShellTerminalController::new`, src/service/controller.rs:124, is genuinely shared ownership: the controller stores it, hands clones to every `OpenSessionResult`/`SessionSupervisor` (src/service/controller.rs:99, src/service/controller.rs:426, src/service/supervisor.rs:1104);`pub use` arms in `lib.rs:20-37` are the house single-surface pattern under private module trees;`InMemoryShellAuthority` is kept per the refusal ledger (docs/explanation/over-engineering-audit-record.md:354) - real behavior with live coverage, not re-flagged.

## err
- clean: seeds `\.unwrap\(\)|\.expect\(` 4, `let _ = |\.ok\(\);` 0, `\bpanic!\(|\bunreachable!\(|\btodo!\(|\bunimplemented!\(` 0, `enum \w*Error` 1; the 4 expects are on validated/static values naming the invariant (`ResourceRef::parse` on a `validate_name`-checked session name, src/service/supervisor.rs:228;`BoundedToken::parse("shell-supervisor-main")` literal, :234; re-checked `ExecutionSpec::new`,:244; ring capacity re-checked against the same bounds `PoolSpec::new` enforces, :1101);`ShellTerminalError` is a closed 14-variant enum split by caller action with wire-style Display codes; no panics, no swallowed Results in src.



## serde
- N/A (seeds: `derive\([^)]*(De)?[Ss]erialize` 0, `serde\((rename_all|deny_unknown_fields|try_from|untagged|flatten|default|skip_serializing_if)` 0, `impl .*Deserialize.*for` 0,, `serde_json::from_|serde_json::to_` 0; no serde dependency in Cargo.toml and no wire format crosses this crate).

## obs
- clean: seeds `\bprintln!\(|\beprintln!\(` 0, `(info|debug|warn|error|trace)!\("` 0,, `\.instrument\(|#\[instrument` 0,, `tracing::|log::` 2; every `warn!`/`debug!` event (in src/service/controller.rs and src/service/supervisor.rs) carries named fields (`provider`, `pool`, `session`, `error`, `decision`, `expected`, `actual`) with template messages and no secrets in fields (the redacting `Debug` impls keep session/canary values out of renders); no `println!` in this library.



## docs
- d2b-provider-shell-terminal#2 sev=medium blast=leaf effort=M verdict=actionable - the ~30 `pub fn` items returning `Result<_, ShellTerminalError>` (e.g. `Authorizer::authorize_request`, `OpenSessionRequest::new`, `PoolSpec::new`, `ShellSession::from_pool`, `restore_pool`, `reconcile_pool_attachments`, `restore_session`, `restart_supervisor`, `open_session`, `finalize_session`, `AttachRequest::new`, `OutputRing::new`, `SupervisorIdentity::new`, `ShellAuthorityLedger::validate_session`) lack an `# Errors` section naming which variants they emit. - fix: add an `# Errors` section to each Result-returning pub item enumerating the `ShellTerminalError` variants that item can return (e.g. `restore_pool`: `# Errors` `CapacityExceeded` when pool name already projected or the authority rejects the restore). - [src/authz.rs:76, src/service/controller.rs:134, src/service/supervisor.rs:88, src/session/ring.rs:16]
  evidence: docs seeds: `^\s*pub (fn|struct|enum|trait|const|type)` ~100 hits (every item carries a contract-shaped first sentence; `#![deny(missing_docs)]` at src/lib.rs:9), `/// # (Examples|Errors|Panics|Safety)` 0, `-> Result<` ~30 hits
- clean: seeds pub items ~100 (fully documented first sentences; all 8 modules carry `//!` docs;, first sentences are contract-shaped ), not implementation narration), no `ignore`d doctests exist to rot;; magic values (`SHELL_REPAIR_INTERVAL_SECS`, capacity bounds) carry meaning-comments; the only systematic gap is the missing `# Errors` class above.

## perf
- clean: seeds `format!\(` 5, `Vec::new\(\)|VecDeque::new\(\)|HashMap::new\(\)|BTreeMap::new\(\)` 5, `\.to_string\(\)` 0; the format sites are one-shot resource-ref/name builders in cold construction paths (`src/resources/session.rs:89-93`, `src/service/controller.rs:380`, `src/service/supervisor.rs:227`), the empty `Vec::new`/`BTreeMap::new` are fresh collection initializers where the empty case is common; no hot-loop allocation sites, no benchmark exists - static (unmeasured) reading only (`OutputRing::append` per-byte push is O(1) amortized and bounded by the 1 MiB ring).



## conc
- clean: seeds `std::thread::|thread::spawn|thread::scope` 0,, `\bMutex<|\bRwLock<` 2, `Atomic\w+|Ordering::` 1 (the word "Atomically" in a doc comment - false positive), `thread_local!|unsafe impl (Send|Sync) for` 0; both `tokio::sync::Mutex` holders (`ShellAuthorityLedger.state` src/service/supervisor.rs:401, `InMemoryShellAuthority.supervisor_processes`, :866) are synchronous surfaces used with `try_lock` fail-closed per a written design comment (src/service/supervisor.rs:410-413); no threads, no atomics, no manual `Send`/`Sync` claims in this crate.



## async
- clean: seeds `async fn|async move|\.await` 0,, `tokio::spawn|spawn_blocking|JoinSet|select!\(|join!\(` 0,, `tokio::sync::(Mutex|RwLock|Notify)` 2 (the two synchronous try_lock Mutexes noted under conc;, `#\[tokio::(main|test)\]|Runtime::block_on` 0; no async fn exists anywhere in src - no `.await`, no spawns, no guards across await points (tokio dep is sync-feature-only).



## unsafe
- N/A (seeds: `\bunsafe \{|\bunsafe fn|\bunsafe impl|\bunsafe extern` 0,, `// SAFETY:` 0,, `transmute|from_raw|MaybeUninit|mem::zeroed` 0; manifest `[lints.rust]` sets `unsafe_code = "forbid"` - no blocks, no exception sites).

## ffi
- N/A (seeds: `extern "C"|no_mangle|unsafe\(link_section` 0,, `catch_unwind` 0,, `repr\(C\)|repr\(transparent\)` 0,, `CStr|CString|c_char` 0; no foreign boundary in this crate).



## macro
- N/A (seeds: `macro_rules!` 0,, `proc_macro|syn::|quote!` 0,, `\$crate` 0,, `to_compile_error|new_spanned` 0; no macro definitions, no proc-macro machinery).

## test
- d2b-provider-shell-terminal#3 sev=low blast=leaf effort=S verdict=actionable - `tests/supervisor_runtime.rs` repeats the full 14-line `ShellPool::new(PoolSpec::new)...))` fixture in 7 of its tests, while sibling `tests/controller_reconcile.rs:8` already defines a `pool()` helper. - fix: extract a parameterized `fn pool(max_sessions: u32, max_attached: u32) -> ShellPool` helper at the top of `tests/supervisor_runtime.rs` (or a shared `tests/common/mod.rs` used by both files), replacing the 7 inline constructions. - [tests/supervisor_runtime.rs:17, tests/supervisor_runtime.rs:81, tests/supervisor_runtime.rs:142, tests/supervisor_runtime.rs:193, tests/supervisor_runtime.rs:255, tests/supervisor_runtime.rs:320, tests/supervisor_runtime.rs:367]
  evidence: test seeds over tests/: `#\[test\]|#\[tokio::test\]` 34, `assert_eq!\(|assert_ne!\(|assert!\(` ~108 hitting lines (matrix cell 142 = 34 + ~108);`proptest!|insta::assert|rstest` 0,, `#\[ignore\]` 0; over src/: 0/0/0/0 (no unit tests in src); the inline fixture repeats 7 times.
- clean: seeds 34 integration tests + ~108 assertions; all deterministic (no network, no clock, no ignored stress tests), error variants asserted via `matches!`/`assert_eq!` against enum variants and never Display strings, redaction tests assert distinct canaries are absent from every `Debug` render, capacity, capability-reuse, recovery-adoption,, attachment-slot edges are covered by named behavior tests; the two large files use local fixture helpers except for the copy-paste class above.



## Coverage
- idiom: clean (seeds 0/0/0
- own: 1 finding(s)
- type: clean (seeds 7/0/0
- api:	clean (seeds ~160/1/14; Arc-in-signature shared-ownership judged explainable
- err:	clean (seeds 4/0/0/1; all 4 expects are invariant-naming on validated values
- serde:	N/A (seeds 0/0/0/0; no serde dep
- obs:	clean (seeds 0/0/0/2; all events carry named fields
- docs:	1 finding(s)
- perf:	clean (seeds 5/5/0; cold paths only
- conc:	clean (seeds 0/2/1/0;2 Mutexes deliberate try_lock, 1 doc-word false positive
- async:	clean (seeds 0/0/2/0; sync-only Mutex use
- unsafe:	N/A (seeds 0/0/0; manifest forbid
- ffi:	N/A (seeds 0/0/0/0
- macro:	N/A (seeds 0/0/0/0
- test:	1 finding(s)