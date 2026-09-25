# d2b-session-p1 - d2b-session - part 1/2
Baseline: 6ebdd4cec | LOC audited: 5296 (excl. src/generated/**) | modules: driver, server, handshake, operation, streams, scheduler, cancellation, fragmentation, attachment, metrics, typed_stream
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: part 1/2 per U1 (f): src/driver.rs, src/server.rs, src/handshake.rs, src/operation.rs, src/streams.rs, src/scheduler.rs, src/cancellation.rs, src/fragmentation.rs, src/attachment.rs, src/metrics.rs, src/typed_stream.rs

## idiom
- clean: seeds ran: 5/0/0. The five `for ... in 0..` hits are bounded rotate scans (`pump_named_stream` driver.rs:1626, `FairScheduler::dequeue` scheduler.rs:222) and test loops (driver.rs:2208, 2261, streams.rs:308); no hand-written `Default`/`From`/`PartialEq`/`Eq`/`Debug`/`Clone`/`Hash` impls, no statement-style `String`/`Vec` accumulation. Hand-written `Debug` impls all redact secrets (repo-sanctioned pattern).

## own
- clean: seeds ran: 30/14/1. Every clone is explainable: `Cancellation`/`Arc` clones at task and command boundaries (driver.rs:124, 347-350, 501-502, 1539, 1611, 2149; server.rs:45, 70, 254, 317, 527; cancellation.rs:71, 184), `RequestId` clones for registry-plus-command pairs (server.rs:245, 259), `policy.clone()` at the handshake encoding boundary (handshake.rs:112, 171), `offer_bytes.to_vec()` for owned canonical bytes (handshake.rs:134), fragment `bytes.to_vec()` for per-fragment ownership (fragmentation.rs:92); the single `Arc<Mutex<...>>` is a test fixture (driver.rs:1735). No `Rc`/`RefCell`/`Cow`.

## type
- d2b-session-p1#1 sev=low blast=leaf effort=S verdict=actionable - `OutboundFrame::channel()` silently falls back to `SESSION_CONTROL` for the constructor-prevented NamedStream-without-stream combination, so an invariant break would misroute a frame to the session control channel with no error - fix: encode the stream inside the class (enum variants `SessionControl`/`TtrpcControl`/`AttachmentControl`/`Named(StreamId, ...)`) or make `channel()` return `Result<ChannelId>` and drop the `unwrap_or(SESSION_CONTROL)` fallback - [scheduler.rs:18-21, scheduler.rs:66-68]
  evidence: type seeds: validate/check fns 4 hits (all boundary validators - `validate_frame` server.rs:360, `validate_credentials` handshake.rs:365, attachment descriptor validation - not findings); `(mode|kind|state): String` 0 hits; bool-flag 0 hits. The class/stream pairing is the one representable-but-guarded combination; constructors `control()`/`named()` (scheduler.rs:25, 36) already reject the bad pair, making the fallback a silent-wrong-value hazard rather than a live illegal state.

## api
- d2b-session-p1#2 sev=low blast=leaf effort=S verdict=actionable - `Fragment.header` is a public mutable field on an exported wire-facing struct while `bytes` is private behind `as_bytes()`, so external crates can corrupt the header/bytes pairing (reassembly validates at use, but the surface invites it) - fix: make `header` private and add `pub fn header(&self) -> &FragmentHeader`, keeping construction through `Fragmenter`/`from_parts` - [fragmentation.rs:10-13]
  evidence: api seeds: pub items 129 (surface re-exported single-path from lib.rs, house pattern); `pub ... Arc|Rc|Box|RefCell<` 0 hits in signatures except evaluated `Arc<dyn ComponentSessionDriver>` at server.rs:202-206 (call sites d2bd/src/composition.rs:7940, d2b-provider-toolkit/src/server/mod.rs:68 - shared ownership across server task and caller, justified); `pub use` arms only in lib.rs. Census: `Fragment` consumed by engine.rs and d2b-bus/src/session/mod.rs:106 - reads header fields only, so an accessor suffices.

## err
- clean: seeds ran: 88/40/6/2. All 88 unwrap/expect hits are inside `#[cfg(test)]` modules except `resource_operation`'s `expect("every ApiMethod has one unary ResourceService member")` (operation.rs:182) on the generated catalog invariant (compile-time-known closed `ApiMethod` set - acceptable per card). All 40 `let _ =` sites are deliberate best-effort `reply.send`/`writer.close()` on error paths where the receiver may be gone (driver.rs:567, 596, 862, 872, 992, 1264-1346; server.rs:269, 354; cancellation.rs:103). All 6 panics are test assertions. Error enums `SessionServerError` (server.rs:182) and `AttachmentValidationError` (attachment.rs:23) are small and caller-action-split.

## serde
- N/A: seeds: 0/0/0/0 all zero; crate crosses no serde wire boundary (canonical binary encodings and protobuf framing instead).

## obs
- clean: seeds ran: 0/0/0/7. Zero `println!`/`eprintln!`; zero interpolated-message events; all 7 `tracing::` sites use named fields (`error = %error`, `stream_id = ...`, `count = ...` - driver.rs:167, 526, 538; server.rs:270, 324, 331, 348). No secrets in fields; no subscriber installed (library).

## docs
- d2b-session-p1#3 sev=medium blast=leaf effort=M verdict=actionable - the security-critical handshake module has zero doc comments on its entire pub surface (23 pub items re-exported from lib.rs): wire functions with magic lengths and closed error codes (`x25519_public_key`, `encode_offer`, `negotiate_offer`, `accept_generation_discovery_request`, `decode_generation_discovery_response`, `NoiseHandshake`, `EstablishedHandshake`, `HandshakeCredentials`, `NegotiatedOffer`) carry no first-sentence contract, no `# Errors`, no `# Panics` - fix: add first-sentence docs plus `# Errors` sections naming the `SessionErrorCode` each function returns, and `# Panics` where a step mismatch panics - [handshake.rs:25, handshake.rs:111, handshake.rs:155, handshake.rs:239, handshake.rs:441]
  evidence: docs seeds: pub items 129, `/// # (Examples|Errors|Panics|Safety)` 0 hits, `-> Result<` 124 hits; per-file doc scan: handshake.rs 0 `///` on 23 pub items (worst in lane; driver.rs 20 on 4, typed_stream.rs 11 on 8 show the house pattern exists). Consumers: engine.rs, d2b-bus/src/session/mod.rs:102-156, d2bd, d2bd-runtime, d2b-provider-toolkit - all rely on these functions.
- d2b-session-p1#4 sev=medium blast=leaf effort=M verdict=actionable - the flow-critical state machines `NamedStreamMux`, `FairScheduler`, `Fragmenter`/`Reassembler`, `StreamId`/`StreamPhase`/`StreamEvent`, `OutboundFrame`, `QueueClass` have zero doc comments on 46 pub items (credit accounting, phase transitions, and error codes are non-obvious) - fix: first-sentence docs on each type and pub method, `# Errors` on the Result-returning mutators, and a module doc in each file - [streams.rs:10, streams.rs:77, streams.rs:165, scheduler.rs:18, scheduler.rs:95, fragmentation.rs:38, fragmentation.rs:99]
  evidence: docs seeds: pub items 129, `/// # (Examples|Errors|Panics|Safety)` 0 hits; per-file doc scan: streams.rs 0 `///` on 20 pub items, scheduler.rs 0 on 16, fragmentation.rs 0 on 10. These types are the multiplexing/flow-control core consumed by engine.rs and d2b-bus.
- d2b-session-p1#5 sev=low blast=leaf effort=S verdict=actionable - the magic bound `value.len() > 128` in `OperationMember::parse` is undocumented: the reader cannot tell why 128 is the canonical member spelling limit or what happens to longer wire strings - fix: extract `const MAX_MEMBER_SPELLING_LEN: usize = 128;` with a comment naming the bound's purpose (wire-visible admission bound) - [operation.rs:207]
  evidence: docs seeds: `/// # (Examples|Errors|Panics|Safety)` 0 hits; the 128 literal is the only undocumented magic value in the lane's validation paths (checked against `valid_identifier` at operation.rs:210).

## perf
- clean: seeds ran: 0/16/14. Zero `format!` sites; all 16 `Vec::new`/`VecDeque::new`/`BTreeMap::new` are struct constructors or test fixtures (cold); all 14 `to_string`/`to_vec` are error-path logging (driver.rs:524), wire-boundary ownership copies (handshake.rs:134, server.rs:295), or tests. Hot paths are already shaped: `Vec::with_capacity` in `Fragmenter::fragment` (fragmentation.rs:70), `Reassembler::accept` (fragmentation.rs:135), `ttrpc_request_id` (server.rs:381), `NegotiatedOffer::prologue` (handshake.rs:100); `NamedStreamEventQueue::receive_for`'s O(n) scan is bounded by DRIVER_EVENT_CAPACITY 128. static (unmeasured).

## conc
- clean: seeds ran: 0/0/57/0. No `std::thread` usage; no `unsafe impl Send/Sync`; the 57 atomics/Mutex/Ordering hits are the documented lock-free admission counter (cancellation.rs:18-30, plan U19 comment; Release/Acquire/AcqRel pairs with a written ordering argument), the generation counter `Arc<AtomicU64>` (driver.rs:109, 189), and test fixtures. `ActiveInboundCalls` (server.rs:30, 224) is a tokio Mutex held per-statement, never across an await.

## async
- clean: seeds ran: 21. Two dedicated worker tasks (`tokio::spawn(run_writer)` driver.rs:354, `tokio::spawn(run_driver)` driver.rs:361) with bounded channels and `Notify`-armed-before-check waits (cancellation.rs:74-80, matches the clippy.toml replacement vocabulary); no guard held across `.await` (server.rs lock scopes are per-statement); no blocking work inside async contexts; `cancel_and_wait` returns `impl Future + Send + 'static`; `abort_writer_and_wait` cannot hang (oneshot send failure skips the wait, driver.rs:1540-1548); the remaining hits are `#[tokio::test]` and test spawns.

## unsafe
- N/A: seeds: 0/0/0 all zero; crate declares `#![forbid(unsafe_code)]` (lib.rs:5), no unsafe blocks, fns, or impls in the lane.

## ffi
- N/A: seeds: 0 all zero; no extern "C", no_mangle, repr(C), CStr/CString, or catch_unwind in the lane.

## macro
- N/A: seeds: 0 all zero; no macro_rules!, proc-macro, syn/quote, or $crate usage in the lane.

## test
- clean: seeds ran: 23/53/0/0. Six of the eleven files carry `#[cfg(test)]` modules (driver.rs 11 tests, server.rs 4, operation.rs 3, streams.rs 1, cancellation.rs 1, metrics.rs 1); all are deterministic (Notify-based, `tokio::time::timeout` only for liveness bounds), assert observable behavior (error codes, ordering, capacity semantics, redaction), use table-driven cases with per-case messages (metrics.rs:97-123, operation.rs:339-347), and can fail (e.g. driver.rs:2166-2169 asserts the full-queue rejection path). No `#[ignore]`, no proptest/insta/rstest. tests/noise_vectors.rs and tests/component_session.rs (out of the published partition) differentially exercise handshake.rs against snow's own implementation.

## Coverage
- idiom: clean (seeds ran: 5/0/0; index loops are bounded rotate/drain patterns)
- own: clean (seeds ran: 30/14/1; every clone/to_owned explainable at a boundary)
- type: 1 finding(s)
- api: 1 finding(s)
- err: clean (seeds ran: 88/40/6/2; only non-test unwrap/expect is the generated-catalog invariant expect at operation.rs:182)
- serde: N/A (seeds: 0/0/0/0 all zero; crate crosses no serde wire boundary)
- obs: clean (seeds ran: 0/0/0/7; all tracing events use named fields)
- docs: 3 finding(s)
- perf: clean (seeds ran: 0/16/14; no format!, hot paths pre-sized, static (unmeasured))
- conc: clean (seeds ran: 0/0/57/0; atomics documented with a written ordering argument)
- async: clean (seeds ran: 21; two worker tasks, no guard across await, no blocking)
- unsafe: N/A (seeds: 0/0/0 all zero; #![forbid(unsafe_code)] at lib.rs:5)
- ffi: N/A (seeds: 0 all zero)
- macro: N/A (seeds: 0 all zero)
- test: clean (seeds ran: 23/53/0/0; deterministic, behavior-asserting, table-driven)