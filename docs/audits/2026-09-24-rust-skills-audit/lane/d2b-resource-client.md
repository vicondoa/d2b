# d2b-resource-client - d2b-resource-client
Baseline: 6ebdd4cec | LOC audited: 4839 (excl. src/generated/**, no tests/ dir, no build.rs) | modules: whole crate (call, client, dispatch, error, lib, process_attach, target, zone_client)
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: none (whole crate)

## idiom
- d2b-resource-client#1 sev=low blast=leaf effort=S verdict=actionable - two byte-identical private async helpers each exist twice in this crate: `await_with_cancellation` (zone_client vs process_attach) and `classify_session_error`/`classify_attach_error` - fix: hoist both into one shared pub(crate) module (e.g., call.rs) and have zone_client.rs and process_attach.rs call the single copies - [packages/d2b-resource-client/src/zone_client.rs:914, packages/d2b-resource-client/src/process_attach.rs:764, packages/d2b-resource-client/src/zone_client.rs:936, packages/d2b-resource-client/src/process_attach.rs:785]
  evidence: census: `async fn await_with_cancellation` over src = 2 hits; `fn classify_\w+_error` over src = 2 hits
- d2b-resource-client#2 sev=low blast=leaf effort=S verdict=actionable - `GuestControlEndpoint::endpoint_uid` is an exact duplicate of `uid()` (same field, same doc sentence; a test pins the equivalence at zone_client.rs:1067) - fix: keep one accessor (e.g., `uid()`) and drop or deprecate the other - [packages/d2b-resource-client/src/zone_client.rs:194, packages/d2b-resource-client/src/zone_client.rs:199, packages/d2b-resource-client/src/zone_client.rs:1067]
  evidence: static read: both return `&self.uid`; census: `endpoint_uid` over packages/, nixos-modules/, tests/, docs/reference/, labs/ = 22 hits (live, so removal touches callers)

## own
- d2b-resource-client#3 sev=low blast=family effort=S verdict=actionable - by-value `resource_ref()` accessors clone a `ResourceRef` (target.rs:155, 279, 407) and `ResolvedTarget::matches_assignment` clones just to compare (`self.resource_ref().as_ref() == Some(reference)`, target.rs:424) - fix: give the in-crate comparison a borrow-returning variant (`Option<&ResourceRef>`) and consider tightening the pub accessors later, migrating about 15 caller files - [packages/d2b-resource-client/src/target.rs:155, packages/d2b-resource-client/src/target.rs:279, packages/d2b-resource-client/src/target.rs:407, packages/d2b-resource-client/src/target.rs:424]
  evidence: census: `\.resource_ref\(\)` over packages/, nixos-modules/, tests/, docs/reference/, labs/ = 21 hits across 15 files; seed `\.clone\(\)` = 74 (remaining clones explainable: waker registry, per-attempt payload, by-value trait seams)

## type
- d2b-resource-client#4 sev=low blast=leaf effort=S verdict=actionable - `MetadataInput::validate_lifetime` (call.rs:168) is a re-validation of the invariant `MetadataInput::new` already enforces at construction (private fields); `CallDriver::new` re-checks it (dispatch.rs:189) where it cannot fail - fix: drop the `validate_lifetime()?` re-check at CallDriver::new (or convert to a debug_assert) - [packages/d2b-resource-client/src/call.rs:168, packages/d2b-resource-client/src/dispatch.rs:189]
  evidence: seed `fn validate_\w+|fn check_\w+` = 2 (validate_lifetime, validate_for); census: `validate_lifetime` over packages = 3 hits (definition plus the two calls: call.rs:103, dispatch.rs:189)

## api
- d2b-resource-client#5 sev=medium blast=leaf effort=M verdict=actionable - eight zero-caller pub items form dead surface: `ZonePeerIdentity::from_enrolled_peer` (zone_client.rs:83), `ZoneSocketConnector::local_daemon_endpoint_identity` (361), `ZoneClient::scoped_query` (635), `scoped_child_query` (646), `call_resource` (703), `ProcessAttachTarget::from_target` (process_attach.rs:125), `configured_launcher_from_target` (132), `ProcessAttachClient::attach_local` (721) - fix: remove or demote to `pub(crate)` (and, if kept, merge the two from-target constructors into one) - [packages/d2b-resource-client/src/zone_client.rs:83, packages/d2b-resource-client/src/zone_client.rs:361, packages/d2b-resource-client/src/zone_client.rs:635, packages/d2b-resource-client/src/zone_client.rs:646, packages/d2b-resource-client/src/zone_client.rs:703, packages/d2b-resource-client/src/process_attach.rs:125, packages/d2b-resource-client/src/process_attach.rs:132, packages/d2b-resource-client/src/process_attach.rs:721]
  evidence: census: each name over packages/, nixos-modules/, tests/, docs/reference/, labs/ = 1 hit (its own definition); the only seam consumer of the call path is `d2b/src/context.rs` via `call_connected`, not `call_resource`

## err
- d2b-resource-client#6 sev=low blast=leaf effort=S verdict=actionable - three reflexive `Mutex::lock().unwrap()` sites in the cancellation waker registry (CancellationFuture::poll, CancellationFuture::drop, CancellationToken::cancel) handle poisoning by panic instead of an explicit choice - fix: use `expect("waker registry lock is not poisoned: no user code runs under it")` or `into_inner()` with the same written reason - [packages/d2b-resource-client/src/call.rs:281, packages/d2b-resource-client/src/call.rs:309, packages/d2b-resource-client/src/call.rs:335]
  evidence: seed `\.unwrap\(\)|\.expect\(` = 92 (89 in #[cfg(test)]; non-test hits are exactly call.rs:281, 309, 335, all carrying the sanctioned synchronous-path allows); `let _ = |\.ok\(\);` = 6 (all deliberate best-effort cancel/close forwards on the cancellation path)

## serde
- clean: seeds ran: 0/0/0/4 (`serde_json::from_|serde_json::to_` hits are the frame codecs at process_attach.rs:489, 497 plus 2 test sites); checked: frames are contract-owned (`d2b-contracts-control`) and codec errors map to `ClientError::ContractViolation`; no serde attributes live in this crate

## obs
- N/A: (seeds: 0/0/0/0 all zero; Cargo.toml declares no `tracing`/`log` dependency, so the lens's applicability condition fails)

## docs
- d2b-resource-client#7 sev=low blast=leaf effort=L verdict=actionable - 50 Result-returning pub items carry no `# Errors` section (zero `# Examples|Errors|Panics|Safety` sections anywhere in the crate), so callers must infer failure conditions from prose - fix: add `# Errors` to the public Result-returning entry points (MetadataInput::new, RetryPolicy::new, CallDriver::new, ZoneClient::connect, ZoneClient::call_connected, ZoneClient::scoped_commit_batch, ProcessAttachClient::attach) - [packages/d2b-resource-client/src/call.rs:87, packages/d2b-resource-client/src/dispatch.rs:131, packages/d2b-resource-client/src/zone_client.rs:711, packages/d2b-resource-client/src/process_attach.rs:648]
  evidence: seeds: `^\s*pub (fn|struct|enum|trait|const|type) ` = 194; `/// # (Examples|Errors|Panics|Safety)` = 0; `-> Result<` = 50

## perf
- clean: seeds ran: 19/12/0 (`format!(`, `Vec::new(|VecDeque::new(|HashMap::new(|BTreeMap::new(`, `\.to_string(`); all 19 format! sites and all 12 Vec::new sites are in #[cfg(test)] fixtures or diagnostic asserts; no allocation sits in a non-test loop (`payload.clone()` per bounded retry attempt is an explainable by-value-trait cost); static (unmeasured)

## conc
- d2b-resource-client#8 sev=low blast=leaf effort=S verdict=actionable - `ResourceWatch` models the open/closing/closed stream state with two `Arc<AtomicBool>` fields (state, closing; zone_client.rs:510-513) where the sibling `ProcessAttachStream` already uses the single `AtomicU8` three-state machine (STREAM_OPEN/CLOSING/CLOSED, process_attach.rs:409-412) - fix: align ResourceWatch onto the same single-atomic state enum - [packages/d2b-resource-client/src/zone_client.rs:510, packages/d2b-resource-client/src/zone_client.rs:512, packages/d2b-resource-client/src/zone_client.rs:513, packages/d2b-resource-client/src/process_attach.rs:409, packages/d2b-resource-client/src/process_attach.rs:412]
  evidence: seeds: `Atomic\w+|Ordering::` = 47; `\bMutex<|\bRwLock<` = 6 (1 non-test waker registry lock, 5 test fakes); ordering pairs are correct (Relaxed counter, Acquire/Release/AcqRel flags), no ordering misfit found

## async
- clean: seeds ran: 80/1/20/0 (`async fn|\.await` = 80; `tokio::spawn` family = 1, a test at process_attach.rs:1118; `tokio::sync::(Mutex|RwLock|Notify)` = 20, all #[cfg(test)] fakes; `Runtime::block_on` = 0); checked: `retry_backoff` refuses without a caller runtime rather than panicking, and cancel-forward-on-cancel is best-effort with no swallowed failures beyond intended cleanup

## unsafe
- N/A: (seeds: 0/0/0 all zero; manifest `unsafe_code = "forbid"`, so seed 4 alone does not make the lens applicable)

## ffi
- N/A: (seeds: 0/0/0/0 all zero)

## macro
- N/A: (seeds: 0/0/0/0 all zero)

## test
- d2b-resource-client#9 sev=medium blast=leaf effort=M verdict=actionable - the core resource-call execution path has no test: no test drives `call_connected`/`call_resource`/`scoped_commit_batch`, so the `execute_resource_call` retry loop, its retry-after-delay backoff branch, scoped-commit admission, and cancel-forward are only exercised indirectly by attach tests - fix: add a fake `ConnectedZoneSession` test covering `call_with_timeout` success, retry-after-delay, cancel-forward, and a scoped commit path - [packages/d2b-resource-client/src/zone_client.rs:703, packages/d2b-resource-client/src/zone_client.rs:711, packages/d2b-resource-client/src/zone_client.rs:755, packages/d2b-resource-client/src/zone_client.rs:858]
  evidence: seeds: `#\[test\]|#\[tokio::test\]` = 34; `assert_eq!\(|assert_ne!\(|assert!\(` = 144; census: `call_connected` over tests/ = 0 hits (the only external caller is d2b/src/context.rs:1622, not a test)
- d2b-resource-client#10 sev=low blast=leaf effort=S verdict=actionable - the close/cancel error-rollback paths are untested: `ProcessAttachStream::close`/`cancel` and `ResourceWatch::close` restore the open state when the transport close errors, but no test injects that failure - fix: add failure-injection tests asserting the state rolls back to open and a second close retries - [packages/d2b-resource-client/src/process_attach.rs:515, packages/d2b-resource-client/src/process_attach.rs:541, packages/d2b-resource-client/src/zone_client.rs:567]
  evidence: static read: close()/cancel() error arms at process_attach.rs:534 and zone_client.rs:580 restore state; seed `#\[test\]` = 34 sites, none names a close/cancel failure injection

## Coverage
- idiom: 2 findings
- own: 1 finding
- type: 1 finding
- api: 1 finding
- err: 1 finding
- serde: clean (seeds ran: 0/0/0/4)
- obs: N/A (seeds: 0/0/0/0 all zero; no tracing/log dependency)
- docs: 1 finding
- perf: clean (seeds ran: 19/12/0)
- conc: 1 finding
- async: clean (seeds ran: 80/1/20/0)
- unsafe: N/A (seeds: 0/0/0 all zero; forbid manifest)
- ffi: N/A (seeds: 0/0/0/0 all zero)
- macro: N/A (seeds: 0/0/0/0 all zero)
- test: 2 findings