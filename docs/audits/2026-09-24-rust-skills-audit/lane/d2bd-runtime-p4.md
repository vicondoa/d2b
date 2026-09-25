# d2bd-runtime-p4 - d2bd-runtime - part 4/4
Baseline: 6ebdd4cec22e6537d1376e83ff7a82b00a8492c0 | LOC audited: 10729 (excl. src/generated/**) | modules: target_runtime, daemon_audit, guest_mode, kernel_module_check, console_session, guest_component_session, ch_stats, concurrency, daemon_version, ch_api, typed_shell_targets, wire_response_helpers, exec_support
Lenses: idiom own type api err serde obs docs perf conc async unsafe ffi macro test | Partitions: part 4/4 - the 13 whole-file units listed in U1 section (f); no item-range splits

## idiom
- d2bd-runtime-p4#1 sev=low blast=leaf effort=S verdict=actionable - `monotonic_tick()` is duplicated verbatim in guest_mode.rs and guest_component_session.rs (identical `OnceLock<Instant>` elapsed-millis helper, two copies of the same code) - fix: move one `monotonic_tick()` into `crate::runtime_util` and have both modules call it - [packages/d2bd-runtime/src/guest_mode.rs:849, packages/d2bd-runtime/src/guest_component_session.rs:587]
  evidence: census: pattern `fn monotonic_tick` over packages/d2bd-runtime/src = 2 hits (both definitions, same body)
- d2bd-runtime-p4#2 sev=low blast=leaf effort=S verdict=actionable - `impl Default for ConsoleSessionTable` hand-writes what `#[derive(Default)]` produces field-wise (all three HashMap fields are Default) - fix: replace the impl with `#[derive(Default)]` on `ConsoleSessionTable` and delete the manual `default()` - [packages/d2bd-runtime/src/console_session.rs:162]
  evidence: idiom seed 2 (`impl (Default|From|PartialEq|Eq|Debug|Clone|Hash) for`) = 3 hits; the other two Defaults (ConsoleRing:82, ConsoleClientHandle:132) are invariant-preserving/panicking and correctly hand-written
- clean: idiom seeds 1/2/3 = 4/3/12 hits; seed 1 loops are side-effecting test fills, seed 3 accumulations all have early returns or byte-buffer shapes where a collect would obscure control flow
  sampled: 50 of 19 hits (all 19 read in full context)

## own
- d2bd-runtime-p4#3 sev=low blast=leaf effort=S verdict=actionable - `ConsoleSessionTable` lookups allocate a `String` on every call (`ConsoleClientHandle(session_handle.to_owned())` in five methods) because the map key newtype does not implement `Borrow<str>` - fix: implement `Borrow<str>` for `ConsoleClientHandle` (or key the two maps by `String`) so `self.clients.get(session_handle)` resolves without allocation - [packages/d2bd-runtime/src/console_session.rs:250, packages/d2bd-runtime/src/console_session.rs:268, packages/d2bd-runtime/src/console_session.rs:293, packages/d2bd-runtime/src/console_session.rs:304, packages/d2bd-runtime/src/console_session.rs:318]
  evidence: own seed 2 (`\.to_owned\(\)|\.to_vec\(\)|\.to_string\(\)`) = 188 hits; the 5 named sites are the only per-call handle-construction allocations outside tests
- d2bd-runtime-p4#4 sev=low blast=leaf effort=S verdict=actionable - every audit write clones the whole `DaemonEvent` (strings included) in `enqueue` because the write API takes `&DaemonEvent`, while every caller (d2bd composition.rs:19356, tests) constructs the event solely to write it - fix: change `write_event`/`write_event_with_authority`/`write_event_async`/`write_event_with_authority_async` to take `DaemonEvent` by value and drop the `event: event.clone()` in `enqueue` - [packages/d2bd-runtime/src/daemon_audit.rs:976, packages/d2bd-runtime/src/daemon_audit.rs:1003]
  evidence: own seed 1 (`\.clone\(\)`) = 124 hits; site-specific (the only per-write event clone; all other clones inspected are required by map keys, closure `'static` bounds, or shared ownership)
- clean: own seeds 1/2/3/4 = 124/188/17/0 hits; Arc clones sit at spawn/closure boundaries, key clones are required by BTreeMap/HashMap ownership, `Arc<Mutex>` state is genuinely shared (ServerState, ring, op locks)
  sampled: 50 of 329 hits (deterministic every-7th; all files read in full or in hit neighborhoods)

## type
- d2bd-runtime-p4#5 sev=medium blast=family effort=S verdict=actionable - `DaemonEvent::ApiReadyTimeout.mode: String` models a closed two-value state (`"strict"` | `"no-wait-api"`, documented at daemon_audit.rs:193) as an open string, so an invalid mode is representable and would land in the preserved audit record - fix: introduce a two-variant `SplitReadinessMode`-style enum with `#[serde(rename_all = "kebab-case")]` and use it for the field; serialized bytes stay `"strict"`/`"no-wait-api"` so the daemon-events JSONL shape is unchanged - [packages/d2bd-runtime/src/daemon_audit.rs:194, packages/d2bd/src/composition.rs:19361]
  evidence: type seed 3 (`(mode|kind|state): String`) = 14 hits; the other hits are wire-mirroring fields (daemon_audit.rs:734-744 bounded error-kind tokens, ch_stats.rs:58 CH-API state) that are deliberate
- d2bd-runtime-p4#6 sev=medium blast=family effort=S verdict=actionable - `retarget_mutating_response` and `response_outcome` re-derive the wire outcome as strings (`Some("applied")`, `Some("broker-error")`, `Some("api-ready-timeout")`) although the same file already builds responses from the `MutatingVerbOutcome` enum, forcing every caller into string matching (8 sites in d2bd composition.rs) - fix: add a typed accessor that parses `outcome` into `MutatingVerbOutcome` (serde) and match on the enum variants in `retarget_mutating_response`, keeping the `_` pass-through for unknown broker outcomes - [packages/d2bd-runtime/src/wire_response_helpers.rs:99, packages/d2bd-runtime/src/wire_response_helpers.rs:118]
  evidence: type seed 3 = 14 hits; census: pattern `response_outcome` over packages = 8 hits (5 string-comparison call sites in packages/d2bd/src/composition.rs:6879,6918,19883,19891,19918)
- d2bd-runtime-p4#7 sev=low blast=leaf effort=S verdict=actionable - the typed-shell target key `(u32, String)` (uid + shell name) is a bare tuple repeated across three collections and every public method signature, so uid/name swap is a type error waiting to happen - fix: extract `TypedShellTargetKey { uid: u32, name: String }` (derive Ord) and use it in `entries`/`recency`/`create_reservations` and the `remember`/`cached`/`forget`/`reserve` signatures - [packages/d2bd-runtime/src/typed_shell_targets.rs:13, packages/d2bd-runtime/src/typed_shell_targets.rs:82]
  evidence: census: pattern `(u32, String)` over packages/d2bd-runtime/src/typed_shell_targets.rs = 21 hits (field types, signatures, test literals)

## api
- d2bd-runtime-p4#8 sev=low blast=leaf effort=S verdict=actionable - `ConsoleClientHandle(pub String)` exposes the inner token of a type documented as "Opaque per-client session token", so any caller can fabricate handles and the opacity claim is unenforced - fix: make the field private, add `FromStr`/`as_str`, and route the table's own lookups through them - [packages/d2bd-runtime/src/console_session.rs:118]
  evidence: api seed 1 (`\bpub (fn|struct|enum|trait|type|const|mod) `) = 333 hits (surface enumerated via structural summaries); site-specific
- d2bd-runtime-p4#9 sev=low blast=leaf effort=S verdict=actionable - `spawn_ch_serial_drainer(_vm: String, ...)` takes an unused `_vm` parameter, and its only caller allocates a hardcoded `"ch-console".to_owned()` per session to satisfy it - fix: drop the parameter and the call-site allocation in `create_ch_session` - [packages/d2bd-runtime/src/console_session.rs:341, packages/d2bd-runtime/src/console_session.rs:422]
  evidence: census: pattern `spawn_ch_serial_drainer` over packages = 2 hits (definition + the single call at console_session.rs:422)
- d2bd-runtime-p4#10 sev=low blast=leaf effort=S verdict=actionable - `DrainerSource` is a dead public enum: never constructed anywhere, with a `#[allow(dead_code)]` `Connected` variant carrying a tokio stream - fix: delete the enum (and the allow) - [packages/d2bd-runtime/src/console_session.rs:54]
  evidence: census: pattern `DrainerSource` over packages = 1 hit (the definition itself; zero constructions or matches)
- d2bd-runtime-p4#11 sev=low blast=leaf effort=M verdict=actionable - `ConsoleRing` and `ConsoleSession` expose all fields `pub` (`ring: RingBuffer`, `notify`, `drainer`, `stdin_tx`), so the documented invariant "notify fires whenever bytes are pushed or EOF is set" (console_session.rs:68) is convention-only: an external caller can push bytes without notifying and waiters hang - fix: make the fields private and expose `push_bytes`/`set_eof`/`read_at` on `ConsoleRing` and accessors on `ConsoleSession` that notify internally - [packages/d2bd-runtime/src/console_session.rs:66, packages/d2bd-runtime/src/console_session.rs:90]
  evidence: api seed 1 = 333 hits; site-specific (the two structs' field lists read in full)
- clean: api seeds 1/2/3 = 333/0/0 hits; the wide `pub mod` surface is the crate's internal-daemon contract (d2bd and d2b-provider-guest are the only consumers); no `Arc`/`Rc`/`Box`/`RefCell` in public signatures beyond the genuine shared-ownership `driver()` handle
  sampled: 50 of 333 hits (surface enumerated via lib.rs re-export list + per-file structural summaries)

## err
- d2bd-runtime-p4#12 sev=low blast=leaf effort=S verdict=actionable - `impl Default for ConsoleClientHandle` panics via `expect("console handle entropy unavailable")` when entropy fails, and nothing in the workspace calls `ConsoleClientHandle::default()` - fix: delete the Default impl (the type already has a fallible `new()` used at attach) - [packages/d2bd-runtime/src/console_session.rs:132]
  evidence: err seed 1 (`\.unwrap\(\)|\.expect\(`) = 295 hits; census: pattern `ConsoleClientHandle::default` over packages = 0 hits; every other non-test unwrap/expect site in the lane is on a literally-built constant, a validated fingerprint, or a startup runtime build (card false-positive classes)
- d2bd-runtime-p4#13 sev=low blast=leaf effort=S verdict=actionable - `FilesystemReader` reports failures as `Result<..., String>`, so `compute_restart_status` cannot distinguish "file missing" from "file unreadable" without string inspection and the detail is only embeddable in a banner - fix: introduce a small `VersionFileReadError` enum (e.g. `Missing` vs `Unreadable(String)`) returned by both trait methods - [packages/d2bd-runtime/src/daemon_version.rs:77, packages/d2bd-runtime/src/daemon_version.rs:85]
  evidence: err seed 4 (`enum \w*Error`) = 14 hits (all other error enums in the lane are closed, Display-bearing, wire-label taxonomies)
- clean: err seeds 1/2/3/4 = 295/14/0/14 hits; panic policy is sound outside tests - every remaining expect is a justified invariant (validated fingerprints, literal constants, map keys collected from the same map, runtime startup); swallowed results are deliberate best-effort cleanup or oneshot replies whose failure is the refusal signal
  sampled: 50 of 295 hits (all non-test unwrap/expect sites read in context)

## serde
- d2bd-runtime-p4#14 sev=low blast=leaf effort=S verdict=actionable - `parse_vm_info` hand-walks `serde_json::Value` with `and_then` chains to extract `state`/`boot_vcpus`/`memory.size` from the Cloud Hypervisor vm.info payload, re-implementing what a derived raw shape does at the boundary - fix: derive `Deserialize` on a raw `ChVmInfoRaw` with `#[serde(default)]` on every field (nested `config.cpus.boot_vcpus` / `config.memory.size`) and convert to `ChVmInfo` - [packages/d2bd-runtime/src/ch_api.rs:79]
  evidence: serde seeds 1/2/3/4 = 22/13/0/46 hits; site-specific (the only Value-walking parse in the lane)
- clean: serde seeds 1/2/3/4 = 22/13/0/46 hits; `DaemonVersionFile`/`DaemonRestartStatus`/`GuestComponentSessionDescriptor` use `deny_unknown_fields` + `rename_all` + internal tagging correctly; the hand-written `Serialize for DaemonEvent` is a deliberate redaction admission gate (sanitize_daemon_event), not a derive candidate

## obs
- d2bd-runtime-p4#15 sev=low blast=leaf effort=S verdict=actionable - `tracing::warn!("qemu console: failed to convert fd to tokio stream: {e}")` interpolates the error into the message instead of a named field, so the event is not queryable by error - fix: `tracing::warn!(error = %e, "qemu console: failed to convert fd to tokio stream")` - [packages/d2bd-runtime/src/console_session.rs:450]
  evidence: obs seed 2 (`(info|debug|warn|error|trace)!\("`) = 7 hits; the other 6 events use named fields (daemon_audit.rs:892, kernel_module_check.rs:417, console_session.rs:402, guest_component_session.rs:322,342) or are doc prose
- clean: obs seeds 1/2/3/4 = 0/7/0/5 hits; zero println/eprintln, zero `instrument` spans (context comes from enclosing daemon spans), no secret-bearing fields found in any event

## docs
- d2bd-runtime-p4#16 sev=low blast=leaf effort=S verdict=actionable - `wire_response_helpers.rs` ships 11 undocumented `pub fn`s (the module doc is the only prose), including `retarget_mutating_response` whose pass-through-on-unknown-outcome behavior is load-bearing for broker-forwarded responses - fix: add one-line contract docs per fn, naming the pass-through semantics and the wire fields projected - [packages/d2bd-runtime/src/wire_response_helpers.rs:7, packages/d2bd-runtime/src/wire_response_helpers.rs:118]
  evidence: docs seed 1 (`^\s*pub (fn|struct|enum|trait|const|type)`) = 393 hits; the file's pub surface enumerated in full (11 fns, zero doc comments)
- d2bd-runtime-p4#17 sev=low blast=leaf effort=S verdict=actionable - `ch_api.rs` leaves its public constants, error enum, info struct, and async entry points undocumented: `DEFAULT_TIMEOUT`/`MAX_RESPONSE_BYTES` are magic values without the why (contrast `CH_HTTP_TIMEOUT` at ch_stats.rs:120 which cites the legacy exporter), and `ChApiError` variants/`ChVmInfo` fields/`get_vm_info`/`shutdown_vm` have no docs - fix: document the consts with their provenance and add one-line docs to the enum, struct, and fns - [packages/d2bd-runtime/src/ch_api.rs:11, packages/d2bd-runtime/src/ch_api.rs:15, packages/d2bd-runtime/src/ch_api.rs:37, packages/d2bd-runtime/src/ch_api.rs:43]
  evidence: docs seed 1 = 393 hits; site enumerated in full (ch_api.rs read whole)
- d2bd-runtime-p4#18 sev=low blast=leaf effort=S verdict=actionable - `target_runtime.rs` documents its domain types thoroughly but leaves a cluster of pub accessors undocumented: `AdmissionBudget::new/limits/active`, `AdmissionPermit::kind/release`, `ProviderDeployment::mode/target_kind/admission` - fix: add one-line docs (at minimum to `AdmissionPermit::release`, whose idempotence is a caller-relevant contract) - [packages/d2bd-runtime/src/target_runtime.rs:256, packages/d2bd-runtime/src/target_runtime.rs:311, packages/d2bd-runtime/src/target_runtime.rs:354, packages/d2bd-runtime/src/target_runtime.rs:1108]
  evidence: docs seed 1 = 393 hits; sites verified by direct read (no `///` on the named methods)
- clean: docs seeds 1/2/3 = 393/0/393 hits; the lane's domain types (DaemonEvent, GuestIdentity, ModuleCheckReport, DaemonVersionFile, OpLockManager, leases) carry contract-grade docs with first-sentence shape; `# Errors`/`# Examples` sections are absent crate-wide (consistent prose style, not a per-item gap)
  sampled: 50 of 393 hits (pub surface enumerated via structural summaries of all 13 files)

## perf
- clean: perf seeds 1/2/3 = 61/27/2 hits; every `format!` site is cold (error diagnostics, audit rendering, one-shot startup) or the artifact IS text (ch_stats Prometheus block, daemon_version banner); `Vec::new()` sites are empty-case-common or bounded buffers; `read_async_capped`/`read_blocking_capped` already use `with_capacity`; no hot-path allocation or bounds-check class found

## conc
- d2bd-runtime-p4#19 sev=medium blast=leaf effort=M verdict=policy-confirmed - `OpLockManager::acquire` busy-spins (`try_lock` + `std::hint::spin_loop()`) while the per-VM/global lock is held across a whole lifecycle op (composition.rs:5612 holds the guard across `dispatch_request_locked`, i.e. seconds for a VM start), so a concurrent same-VM or global request burns a full core for the op duration; the doc's "critical sections are single map ops" justification covers only the map-entry lock, not the held op lock - fix: replace the spin with the repo's sanctioned wait-on-condition shape (`tokio::sync::Notify` armed before the check + `tokio::time::timeout`, clippy.toml:37-39) or park/wake on the dedicated dispatch threads; requires a policy/ADR decision first - [packages/d2bd-runtime/src/concurrency.rs:163, packages/d2bd-runtime/src/concurrency.rs:189, packages/d2bd/src/composition.rs:5612]
  evidence: conc seeds 1/2/3/4 = 26/28/45/0 hits; site read in full; static (unmeasured) - no benchmark exists for contended op throughput
- clean: conc seeds 1/2/3/4 = 26/28/45/0 hits; `ConnSemaphore` CAS uses the weakest correct orderings (Acquire/AcqRel), `HEALTHCHECK_COUNTER` is a Relaxed counter, the audit appender and connect-probe worker are dedicated bounded threads (R4 shape), no `unsafe impl Send/Sync`, no `thread_local!`/`static mut`

## async
- d2bd-runtime-p4#20 sev=low blast=family effort=M verdict=actionable - `CONSOLE_DRAINER_RUNTIME` is a `static OnceLock<tokio::runtime::Runtime>` started inside library code (console_session.rs:33-44), giving the daemon a second multi-thread runtime per process that is never shut down, while the binary already owns a `#[tokio::main(flavor = "multi_thread")]` runtime (d2bd/src/main.rs:142) - fix: own the runtime at the binary top and pass a `tokio::runtime::Handle` into `create_ch_session`/`create_qemu_session` (or spawn drainers on the daemon runtime) instead of a crate-static `OnceLock` - [packages/d2bd-runtime/src/console_session.rs:33, packages/d2bd-runtime/src/console_session.rs:35]
  evidence: async seed 4 (`#\[tokio::(main|test)\]|Runtime::block_on`) = 4 hits; census: pattern `tokio::main` over packages/d2bd/src/main.rs = 1 hit (line 142); no async-gate-allow marker covers the drainer runtime (grep over packages/xtask/data/async-gate-inventory.json = 0 hits)
- clean: async seeds 1/2/3/4 = 47/8/22/4 hits; no guard is held across an await (ring guards are dropped before `notify_waiters()`), the async audit seat awaits a oneshot reply, `run_connect_probe` never parks the caller's executor, and the `#[tokio::test]` sites are plain harnesses

## unsafe
- clean: unsafe seeds 1/2/3/4 = 0/0/1/0 hits; the single hit is doc prose (`from_raw_fd` mentioned in the `create_qemu_session` doc at console_session.rs:435, same false-positive class recorded in U1 (d)8 for typed_error.rs); zero unsafe blocks/fns/impls, zero SAFETY comments, zero transmute/MaybeUninit/zeroed in the lane

## ffi
- N/A: ffi seeds 1/2/3/4 = 0/0/0/0 all zero; the lane crosses no foreign-language boundary (no extern, no repr(C), no CStr/CString, no catch_unwind)

## macro
- N/A: macro seeds 1/2/3/4 = 0/0/0/0 all zero; no macro_rules!, no proc-macro/syn/quote, no $crate, no trybuild machinery in the lane

## test
- d2bd-runtime-p4#21 sev=low blast=leaf effort=S verdict=actionable - `no_op_does_not_write_file` cannot fail on the behavior it names: the temp dir is never connected to the log (`DaemonAuditLog::no_op()` has no state dir; the comment at daemon_audit.rs:2368 admits the limitation), so `count == 0` is vacuously true and only the write-does-not-error `expect` is exercised - fix: make the state dir injectable (or test via a log constructed with a read-only/blocked state dir) so the no-file-created claim is actually asserted, or rename the test to what it verifies - [packages/d2bd-runtime/src/daemon_audit.rs:2367]
  evidence: test seeds 1/2/3/4 = 75/317/0/0 hits (src + tests/runtime_boundary.rs); site read in full
- clean: test seeds 1/2/3/4 = 75/317/0/0 hits; the suite is behavior-asserting and deterministic - leak-safety sentinels with closed key-set assertions (daemon_audit), timing-free concurrency tests via barriers/channels (concurrency), table-driven parse cases (ch_api, kernel_module_check), zero `#[ignore]`, zero proptest/insta/rstest (plain unit + integration tests fit the assertions)
  sampled: 50 of 392 hits (test bodies read via full-file reads of the smaller modules and hit neighborhoods of the two large files)

## Coverage
- idiom: 2 finding(s)
- own: 2 finding(s)
- type: 3 finding(s)
- api: 4 finding(s)
- err: 2 finding(s)
- serde: 1 finding(s)
- obs: 1 finding(s)
- docs: 3 finding(s)
- perf: clean (seeds ran: 61/27/2)
- conc: 1 finding(s)
- async: 1 finding(s)
- unsafe: clean (seeds ran: 0/0/1/0)
- ffi: N/A (seeds: 0/0/0/0 all zero; no foreign boundary in the lane)
- macro: N/A (seeds: 0/0/0/0 all zero; no macro definitions or proc-macro machinery)
- test: 1 finding(s)