# d2bd-runtime-p3 - d2bd-runtime - part 3/4
Baseline: 6ebdd4cec22e6537d1376e83ff7a82b00a8492c0 | LOC audited: 10718 (excl. src/generated/**) | modules: exec_session, unsafe_local_helper, metrics, authority_persistence, readiness, otel_host_bridge_readiness, ownership_preflight, unix_transport, exec_session_real, terminal_session, pidfs_probe, lib, runtime_util
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: part 3/4 of d2bd-runtime (whole-file lane; no range splits)

## idiom
- d2bd-runtime-p3#1 sev=low blast=leaf effort=S verdict=actionable - two fd-extraction loops grow a Vec via `extend` in a `for` over `cmsgs()`, where a filter_map collect would read as one expression - fix: collect `message.cmsgs().map_err)...)?.filter_map(|c| ...).flatten().collect()` into the result Vec in `receive_frame` and `read_frame_with_fds` - [packages/d2bd-runtime/src/unsafe_local_helper.rs:789, packages/d2bd-runtime/src/unix_transport.rs:294]
  evidence: seed `let mut \w+ = (String|Vec)::new\(\)` = 4 hits; the two allocate-then-extend fd loops are the replaceable pair (the other two hits build String/axes buffers, deliberate accumulation)
- clean: additional seeds for this lens: `for \w+ in 0\.\.` = 6, `impl (Default|From|PartialEq|Eq|Debug|Clone|Hash) for` =  4; index loops are all `for _ in 0..N` bounded retry/drain loops (no element indexing), hand-written `Default` impls set non-zero invariants (`ExecOpDeadlines`, `ExecSessionCaps`, `ReadinessWaitConfig`) or wrap non-Default fields (`Registry::new` -> `Self::new`), so none is derive-replaceable

## own
- d2bd-runtime-p3#2 sev=low blast=leaf effort=S verdict=actionable - three `operation_id.to_string()` copies of an already-owned `String` are produced only to be borrowed or passed along (`complete_pending` takes `String` just for one comparison), so each completed/rejected helper op pays a heap alloc - fix: change `complete_pending` to take `operation_id: &str` and pass `&result.operation_id` / `&rejected.operation_id` at the three call sites (the second local `let operation_id = result.operation_id.to_string()` becomes `&result.operation_id` directly) - [packages/d2bd-runtime/src/unsafe_local_helper.rs:625, packages/d2bd-runtime/src/unsafe_local_helper.rs:629, packages/d2bd-runtime/src/unsafe_local_helper.rs:644]
  evidence: seed `\.to_owned\(\)|\.to_vec\(\)|\.to_string\(\)` = 157 hits; the three sites are the only String-to-String copies made solely for borrowing
- clean: all own seeds (58 `.clone()`, 157 `.to_owned`/`.to_vec`/`.to_string`,  1 `Arc<Mutex<`, 0 `Cow<`) hits read; the `Arc<Mutex<...>>` hit is `authority_persistence.rs:77` type alias `Rows`, part of a documented multi-thread shared ledger; every other clone/copy is a map-correlated stored value, a test fixture, an error-context `to_owned`, or an Arc clone at a spawn/thread boundary (required for `'static`)

## type
- clean: seeds ran:  1/0/0; the single hit is a `#[test]` fn name (`validate_rejects_wrong_kind` in metrics.rs:1139), not a runtime validation predicate; pub struct/enum surfaces were surveyed via the api-runs for flag soup, Option-pair smells, stringly-typed state (none;`ReadinessProbe`'s two booleans fold into the terminal `OtelHostBridgeReadiness` verdict enum, `NegotiatedCaps`'s booleans are independently real capability gates)

## api
- d2bd-runtime-p3#3 sev=low blast=leaf effort=M verdict=actionable - `pub fn spawn_session_worker` (with `pub struct WorkerSpawn`, `SessionTable`, `ExecOpDeadlines`, `ExecStartSpec`, etc.) has no production caller in the workspace - only its own crate's tests - so the whole exec-session worker surface is either pending wiring from d2bd composition or dead public API - fix: wire `spawn_session_worker`/`SessionTable` into d2bd's exec composition (or gate the module test-support-only pending that wiring) - [packages/d2bd-runtime/src/exec_session.rs:900]
  evidence: census: `spawn_session_worker` over packages/, nixos-modules/, tests/, docs/reference/, labs/, BUILD.bazel =  4 hits (pub fn def at :900, doc cross-ref at :880, two test call sites at :2486 and :3624); api seed counts:   214/9/0
- clean: the 9 `pub .*(Arc|Rc|Box|RefCell)<` hits are all genuine shared-ownership tokens (`Established.client`, `WorkerSpawn.connector/clock/owner_reaper`, `HelperRegistry::accept_loop(Arc<Self>)`, `ZoneAuthorityLedger::install_*`) whose shared ownership is exercised at spawn/thread boundaries;`pub use` absent, every item reachable via exactly one path (lib.rs `pub mod` arms are the house single-surface pattern)

## err
- d2bd-runtime-p3#4 sev=low blast=leaf effort=S verdict=actionable - `spawn_session_worker` panics at `std::thread::Builder::spawn)...).expect("spawn exec session worker thread")` in library code on an environmental failure (thread exhaustion/ENOMEM) with a caller-visible alternative - fix: return `std::io::Result<JoinHandle<()>>` (or map to `TypedError`) and have the two test call sites adjust - [packages/d2bd-runtime/src/exec_session.rs:939]
  evidence: seed `\.unwrap\(\)|\.expect\(` = 254 hits; the only non-test hit besides :939 is metrics.rs:352 `descriptor)...).expect("validated above")`, a post-check invariant; census: spawn_session_worker =  4 hits (no prod caller, so the panic is test-reachable only today)
- clean: remaining err seeds: `let _ = |\.ok\(\);` =  41 hits (every site is deliberate best-effort teardown/oneshot/`write!` onto a `String`, or test cleanup), `\bpanic!\(|\bunreachable!\(|\btodo!\(|\bunimplemented!\(` =  24 (all test code or internal-invariant `unreachable!`s in non-test code at exec_session.rs:1203,:1256), `enum \w*Error` =  7 (closed enums with `slug()` accessors, fine taxonomy shapes)

## serde
- clean: seeds ran:  1/0/0/13; the lone derive hit is `OtelHostBridgeReadiness` (internal-tag `status` + `kebab-case` enum, pinned by envelope-shape tests at otel_host_bridge_readiness.rs:601-623); all `serde_json` boundary calls map failures into typed errors (`ExecOpError::Protocol`, `HelperRegistryError::InvalidFrame`, `AuthorityPersistenceError::RowInvalid`, `TypedError::InternalIo`) rather than unwrapping

## obs
- d2bd-runtime-p3#5 sev=low blast=leaf effort=S verdict=actionable - the one-shot-exit unparseable-stat warning is message-only with no named fields (`tracing::warn!("wait_for_one_shot_exit: /proc/<pid>/stat unparseable; ...")`), though `pid` and a `path` string are in scope and sibling warnings carry `%err`/field-style context - fix: emit fields (`pid = %pid`, `path = %path`) with a short message (or wrap the poll loop in a span carrying `pid`) - [packages/d2bd-runtime/src/readiness.rs:327]
  evidence: seed `(info|debug|warn|error|trace)!\("` = 23 hits; all other hit sites carry named fields (or `%err` field); no instrument span envelopes this helper (seed `\.instrument\(|#\[instrument` = 0)
- d2bd-runtime-p3#6 sev=low blast=leaf effort=S verdict=actionable - pidfs probe warns/errors interpolate a prebuilt `{msg}` string with embedded `st_dev`/`detail` values instead of named fields, while the sibling `PidfsAvailable` arm already emits `pidfs_st_dev`/`pidfs_st_ino` fields - fix: give `PidfsNotPresent` and `UnexpectedError` arms named `pidfs_st_dev = %st_dev` / `detail = %detail` fields (and keep the long operator-facing sentence as the message template) - [packages/d2bd-runtime/src/pidfs_probe.rs:129, packages/d2bd-runtime/src/pidfs_probe.rs:132, packages/d2bd-runtime/src/pidfs_probe.rs:144, packages/d2bd-runtime/src/pidfs_probe.rs:147, packages/d2bd-runtime/src/pidfs_probe.rs:158]
  evidence: seed `(info|debug|warn|error|trace)!\("` = 23 hits; the cited five are the only `{msg}`-interpolated events in this lane (readiness.rs:327 is finding d2bd-runtime-p3#5)
- clean: `\bprintln!\(|\beprintln!\(` =  0;`tracing::|log::` =  23 (all `tracing`, structured, prefixed `provider`/`event_kind`/`result` scheme in unsafe_local_helper, `vm`/`path`/`reason` fields elsewhere); no secret material in any field (Debug impls redact terminal bytes, argv/env, and unguessable handles)

## docs
- d2bd-runtime-p3#7 sev=medium blast=leaf effort=M verdict=actionable - unsafe_local_helper.rs has no `//!` module doc and its public surface (consts `HELPER_HEARTBEAT_INTERVAL`/`HELPER_STALE_AFTER`/`HELPER_OPERATION_TIMEOUT`, enums `HelperRegistryError`/`HelperAvailability`/`HelperReply`, struct `HelperRegistry` + its seven pub methods) carries no doc comments, unlike every sibling module in this crate - fix: add a `//!` header (lifecycle, wire protocol, thread model, redaction rules) and one-line `///` docs per pub item - [packages/d2bd-runtime/src/unsafe_local_helper.rs:1, packages/d2bd-runtime/src/unsafe_local_helper.rs:33, packages/d2bd-runtime/src/unsafe_local_helper.rs:42, packages/d2bd-runtime/src/unsafe_local_helper.rs:65, packages/d2bd-runtime/src/unsafe_local_helper.rs:71, packages/d2bd-runtime/src/unsafe_local_helper.rs:190]
  evidence: seed `^\s*pub (fn|struct|enum|trait|const|type)` = 164 hits in lane; the cited items have zero `///` lines (verified by contiguous read)
- d2bd-runtime-p3#8 sev=medium blast=leaf effort=S verdict=actionable - public exec-session DTO fields lack doc comments on a cross-crate contract surface (`ExecStartSpec.vm/argv/tty/detached/env/cwd/term_size`, `ExecSessionInfo.tty/stdout_offset/stderr_offset`, `Established.client/info/control_seq/caps`, `WorkerSpawn.connector/spec/deadlines/establish_tx/control_rx`), while sibling fields (`request_id`, `NegotiatedCaps.*`, `TerminalReaper`/`SessionSlot` fields) are documented - fix: add `///` per field (semantics plus any redaction/derivation promise), especially what `control_seq`/`establish_tx` carry - [packages/d2bd-runtime/src/exec_session.rs:181, packages/d2bd-runtime/src/exec_session.rs:211, packages/d2bd-runtime/src/exec_session.rs:249, packages/d2bd-runtime/src/exec_session.rs:882]
  evidence: seed `^\s*pub (fn|struct|enum|trait|const|type)` = 164 hits; the cited field blocks were read contiguously and have no per-field `///`
- d2bd-runtime-p3#9 sev=medium blast=leaf effort=S verdict=actionable - readiness.rs exposes seven undocumented pub predicates/functions (`readiness_predicate_ready`, `unix_socket_exists`, `unix_socket_listening`, `tcp_port_ready`, `wait_for_tcp_port`, `command_ready`, `readiness_predicate_ready_async`) whose contracts are non-obvious (e.g. `unix_socket_listening` parses `/proc/net/unix` flags;`command_ready` strips `NOTIFY_SOCKET`), while `api_socket_info_ready`/`wait_for_readiness_async` do carry `///` - fix: add one-line `///` first sentences + `# Errors` notes on the `Result<_, String>` shapes - [packages/d2bd-runtime/src/readiness.rs:15, packages/d2bd-runtime/src/readiness.rs:78, packages/d2bd-runtime/src/readiness.rs:85, packages/d2bd-runtime/src/readiness.rs:103, packages/d2bd-runtime/src/readiness.rs:112, packages/d2bd-runtime/src/readiness.rs:125, packages/d2bd-runtime/src/readiness.rs:145]
  evidence: seed `^\s*pub (fn|struct|enum|trait|const|type)` = 164 hits; the seven cited fns were read contiguously and have no `///`; seed `-> Result<` =  113 (the String-typed error returns are message-shaped wire slugs, not enum-typed, so `# Errors` sections can name their slugs)
- clean: seed `/// # (Examples|Errors|Panics|Safety)` = 0 in lane; no pub item needs a doctest/canonical section beyond the `# Errors`-naming noted above (this crate's contract docs live in module/dossier prose, consistent with repo convention)

## perf
- clean: seeds ran:  59/52/27; the `format!` hits are error-path diagnostics (cold), the metrics renderer's `push_str(&format!)...))`-built exposition, and test fixtures (per U1 the wire-artifact/String-building class is excluded);`Vec::new()`/`BTreeMap::new()` hits are empty-value constructors or test fakes (empty case common);`to_string()` hits duplicate the own-lens borrow finding (d2bd-runtime-p3#2) or are error-context copies; no hot path with an unbounded allocation was identified (static (unmeasured) assessment only)

## conc
- d2bd-runtime-p3#10 sev=medium blast=leaf effort=M verdict=policy-confirmed - unsafe_local_helper.rs uses `parking_lot::Mutex` for its registry/connection/ledger state (`use parking_lot::Mutex` + 4 `Mutex<...>` field types + 39 `.lock()` call sites), which the repo bans outright outside the R4 dedicated bounded-worker boundary - fix: replace with `tokio::sync::Mutex` reached through the documented blocking-seat patterns this crate already uses (`metrics::Registry::blocking_lock` for worker-thread-only seats, `authority_persistence::lock_sync` try_lock spin where an ambient runtime may exist) - [packages/d2bd-runtime/src/unsafe_local_helper.rs:17, packages/d2bd-runtime/Cargo.toml:34]
  evidence: seed `\bMutex<|\bRwLock<` =  27 hits; parking_lot import at unsafe_local_helper.rs:17 and dep at Cargo.toml:34; policy: clippy.toml:40-43 bans parking_lot outright (KD3; single R4 exception, not this site)and clippy.toml:82-84 names `tokio::sync::Mutex::lock` as the replacement
- clean: remaining conc seeds: `std::thread::` = 13 (dedicated daemon/helper handler threads with documented ownership, plus test threads), `Atomic\w+|Ordering::` = 97 (paired Acquire/Release, AcqRel idempotency guards, Relaxed counters - weakest-correct orderings), `thread_local!|unsafe impl (Send|Sync) for` = 0

## async
- clean: seeds ran:  260/10/27/14; the async code is well-disciplined:dedicated current-thread runtime per session worker (documented concurrency contract), long-polls spawned onto it so fast control ops never head-of-line block, `tokio::sync::Mutex` guards are scoped or lazily-dropped before awaiting (`prove_claim` clones the Arc out of the guard first), blocking seats are the sanctioned `blocking_lock`/`try_lock`-spin patterns renamed in code comments (plan U17), and the sync-only readiness fns carry `#[allow)..., reason = "synchronous path")]` and are pinned for d2bd's worker-thread callers

## unsafe
- clean: seeds ran:  0/0/2/0; the two `from_raw` hits are `rustix::process::Pid::from_raw`, a safe constructor, not a UB hazard; no `unsafe` block/fn/impl, no `// SAFETY:` comment, and no raw-pointer `from_raw`/`transmute`/`MaybeUninit` exists in this lane's scope - nothing to justify or doc-inspect (the crate inherits the workspace `unsafe_code = "forbid"` posture)

## ffi
- N/A (seeds: 0/0/0/0 all zero; no `extern "C"`, no `catch_unwind`, no `repr(C)`/`repr(transparent)`, no C string/char types - this part crosses no foreign-caller boundary;`nip`/`rustix` syscall wrappers only)

## macro
- N/A (seeds: 0 all zero; no `macro_rules!` definitions or proc-macro/syn/quote usage in the lane scope; std invariants like `cmsg_space!` are external-crate macros, not this crate's)

## test
- clean: seeds ran:  125/293/0/0; the suite is hermetic and behavior-focused:all fakes/fixtures injected (fake clock, fake driver, fake connector, fake source), no network/no real timeouts beyond bounded local sleeps, error behavior asserted via `matches!)...Error::Variant)` not Display strings; `runtime_boundary.rs` pins the crate's dependency discipline, `#[should_panic]` on an internal-invariant check, and no `#[ignore]`/flaky gates exist

## Coverage
- idiom: 1 finding(s)
- own:  1 finding(s)
- type: clean (seeds ran:  1/0/0; single hit is a test fn name, not a validation predicate; no flag-soup/Option-pair/string-state invariant class)
- api:  1 finding(s)
- err:  1 finding(s)
- serde: clean (seeds ran:  1/0/0/13; single serde type is an internal-tag Status enum pinned by envelope tests; all serde_json boundary errors mapped to typed errors)
- obs:   2 finding(s)
- docs:	 3 finding(s)
- perf: clean (seeds ran:	 59/52/27; format!/collection/to_string hits are error paths, bounded renderers, wire artifact builders, or test fixtures - no hot-path allocation)
- conc:	 1 finding(s)
- async: clean (seeds ran:	 260/10/27/14; spawn/await/lock discipline matches the documented concurrency contract; blocking seats sanctioned with `"synchronous path"` allows and plan U17 comments)
- unsafe: clean (seeds ran:	 0/0/2/0; the 2 hits are safe `rustix::process::Pid::from_raw` constructors - no unsafe code in scope)
- ffi:	N/A (seeds: 0/0/0/0 all zero; no FFI boundary in this part)
- macro:	N/A (seeds: 0 all zero; no macro definitions in this part)
- test:	clean (seeds ran:	 125/293/0/0; hermetic, injected, variant-asserting suite with no ignored tests)