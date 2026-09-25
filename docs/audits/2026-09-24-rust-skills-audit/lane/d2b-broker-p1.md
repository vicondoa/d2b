# d2b-broker-p1 - d2b-broker - part 1/7
Baseline: 6ebdd4cec22e6537d1376e83ff7a82b00a8492c0 | LOC audited: 11016 (excl. src/generated/**) | modules: runtime.rs:10301-20603 (item-range split; probe/parse helpers, BrokerError audit+response, SIGCHLD reaper, targeted reap, spawn-rollback cleanup, mod tests 11840-20603), ops/usbip_lock.rs, seccomp_compile_tests.rs
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: runtime.rs:10301-20603, ops/usbip_lock.rs, seccomp_compile_tests.rs

## idiom
- d2b-broker-p1#4 sev=low blast=leaf effort=S verdict=actionable - three near-identical hand-rolled flag parsers `parse_probe_flags`/`parse_stub_flags`/`parse_export_flags` duplicate the same index-loop skeleton and the `--socket-path`/`--test-uid` arms (with `expect_arg` bound-checking) three times - fix: extract one table-driven flag parser (flag spec -> value) that the three wrappers compose, or a shared `parse_common_flags` helper returning `(socket_path, test_uid)` - [packages/d2b-broker/src/runtime.rs:10392, packages/d2b-broker/src/runtime.rs:10418, packages/d2b-broker/src/runtime.rs:10453, packages/d2b-broker/src/runtime.rs:10495]
  evidence: seeds `for \w+ in 0\.\.` = 2, `impl (Default|From|PartialEq|Eq|Debug|Clone|Hash) for` = 0, `let mut \w+ = (String|Vec)::new\(\)` = 4; the two index loops (20235, 20457) and the accumulations (13341, 17867, 20234, usbip_lock.rs:302) are side-effectful test loops / required read buffers, not iterator candidates
- clean: seeds ran 2/0/4; checked every hit - index loops and Vec/String accumulation sites are test fixtures with side-effectful bodies or required read buffers, no derive-able hand-written impls in scope

## own
- clean: seeds ran 137/282/0/0 (clone, to_owned/to_vec/to_string, Rc/RefCell/Arc<Mutex>/Arc<RwLock>, Cow) - sampled: 47 of 419 hits (every 9th); every sampled clone/to_owned is a test-fixture literal, an audit-record snapshot (`audit_context.request_fields.clone()` at 10934), a wire-boundary owned string (11452), or an error-context path capture in usbip_lock.rs; no Rc/RefCell/Arc<Mutex>/Cow anywhere in scope

## type
- clean: seeds ran 1/0/0; the single `fn validate_` hit is `validate_socket_parent` (runtime.rs:10321), a one-shot CLI startup preflight rather than a parse-once candidate; no boolean-flag soup, no stringly-typed state, no Option-pair smells in scope (TargetedReapOutcome and UsbipLockError are well-shaped enums)

## api
- d2b-broker-p1#3 sev=low blast=leaf effort=S verdict=actionable - pub fn `acquire_lock` takes `_daemon_uid: u32` (underscore-prefixed) that the body never uses - the record owner is always `Uid::current()`, so every caller (live_handlers.rs:467 plus 8 test sites) supplies a value the function ignores - fix: drop the parameter and update the call sites - [packages/d2b-broker/src/ops/usbip_lock.rs:93, packages/d2b-broker/src/live_handlers.rs:467]
  evidence: census `acquire_lock(` over packages/, nixos-modules/, tests/, docs/reference/, labs/, BUILD.bazel/*.bzl = 24 hits; seed `\bpub (fn|struct|enum|trait|type|const|mod) ` = 5 (all usbip_lock.rs), `pub .*\b(Arc|Rc|Box|RefCell)<` = 0, `^\s*pub use ` = 0
- clean: seeds ran 5/0/0; the 5 public items are confined to ops/usbip_lock.rs (UsbipLockError, ensure_lock_root, acquire_lock, release_lock, peek_owner) - no Arc/Rc/Box/RefCell or dependency types in public signatures, no re-export arms, runtime.rs range exposes only pub(crate) items

## err
- d2b-broker-p1#2 sev=medium blast=leaf effort=S verdict=actionable - `UsbipLockError::Io { path: PathBuf, detail: String }` flattens the underlying `io::Error` into its Display string at 12 conversion sites, losing the source chain (os error number and context) a `#[source]` field would keep for diagnosis - fix: change the variant to `Io { path: PathBuf, #[source] source: std::io::Error }` and drop the `detail: e.to_string()` maps - [packages/d2b-broker/src/ops/usbip_lock.rs:47, packages/d2b-broker/src/ops/usbip_lock.rs:84, packages/d2b-broker/src/ops/usbip_lock.rs:98, packages/d2b-broker/src/ops/usbip_lock.rs:110, packages/d2b-broker/src/ops/usbip_lock.rs:122, packages/d2b-broker/src/ops/usbip_lock.rs:128, packages/d2b-broker/src/ops/usbip_lock.rs:134, packages/d2b-broker/src/ops/usbip_lock.rs:138, packages/d2b-broker/src/ops/usbip_lock.rs:142, packages/d2b-broker/src/ops/usbip_lock.rs:162, packages/d2b-broker/src/ops/usbip_lock.rs:174, packages/d2b-broker/src/ops/usbip_lock.rs:179, packages/d2b-broker/src/ops/usbip_lock.rs:187]
  evidence: seed `enum \w*Error` = 1 (UsbipLockError); `\.unwrap\(\)|\.expect\(` = 518 - sampled: 48 of 518 hits (every 11th), all test-code expects with meaningful messages; production band 10301-11840 has 0 unwrap/expect; `let _ = |\.ok\(\);` = 62 (3 production sites at 10540/11491/11587 are deliberate: cfg-gated param, OnceLock set, best-effort cleanup); `\bpanic!\(|\bunreachable!\(|\btodo!\(|\bunimplemented!\(` = 50 (all tests plus the justified unix-only `Component::Prefix` unreachable at usbip_lock.rs:284)
- clean: seeds ran 518/62/50/1 - panic policy is sound in production code (zero unwrap/expect/panic in 10301-11840); the only taxonomy issue is the Io-variant source-chain loss recorded above

## serde
- clean: seeds ran 0/0/0/13; no serde derives or hand-written Deserialize impls in scope - the 13 serde_json hits are test round-trips and redaction assertions (serialObserved present, raw serial/busid/key_hex absent) plus the probe CLI's stdout rendering at 10377; no wire type crosses a boundary in this part

## obs
- clean: seeds ran 5/0/0/35; the two println!/eprintln! hits in runtime.rs are the layer1-bootstrap probe CLI's product output (10375) and a test skip message (15893), the three seccomp_compile_tests eprintln! sites are test skip notices; all 12 production tracing events (10822-11818) carry named fields (operation, error_kind, detail, runner_id, error, pid, exit_status) with no interpolated-message events and no secrets

## docs
- d2b-broker-p1#5 sev=low blast=leaf effort=S verdict=actionable - the four Result-returning public fns in ops/usbip_lock.rs (`ensure_lock_root`, `acquire_lock`, `release_lock`, `peek_owner`) document failure conditions only in prose and carry no `# Errors` canonical section, which the docs contract wants on every Result-returning item - fix: add `# Errors` sections naming the returned variants (LockAlreadyHeld, OwnerMismatch, Io) - [packages/d2b-broker/src/ops/usbip_lock.rs:81, packages/d2b-broker/src/ops/usbip_lock.rs:90, packages/d2b-broker/src/ops/usbip_lock.rs:171, packages/d2b-broker/src/ops/usbip_lock.rs:316]
  evidence: seed `-> Result<` = 17 (14 runtime + 3 usbip), `/// # (Examples|Errors|Panics|Safety)` = 0, `^\s*pub (fn|struct|enum|trait|const|type)` = 5; all 5 public items have one-line doc summaries; runtime.rs range items are pub(crate) with adequate docs
- clean: seeds ran 5/0/17 - every public item in scope has a one-line doc comment; the only gap is the missing `# Errors` sections recorded above

## perf
- clean: seeds ran 89/57/29 - all format!/Vec::new/to_string hits are cold error-message construction (RunError/BrokerError/UsbipLockError detail strings), test fixtures, or wire-boundary owned strings; production band 10301-11840 has zero Vec::new() and zero hot-loop allocation; static (unmeasured)

## conc
- clean: seeds ran 10/4/0/0; all 14 hits are test-only - std::thread::spawn in harnesses, test Mutexes (FakeDispatchBackend, RegistryTestGuard's LazyLock with `poisoned.into_inner()`), and std::thread::sleep polling loops with documented convergence; no atomics, no unsafe Send/Sync impls, no shared-state design issues in production code

## async
- d2b-broker-p1#1 sev=high blast=wide effort=S verdict=actionable - `cleanup_spawned_runner_after_failure` performs a blocking `waitid(Id::PIDFd(pidfd), WaitPidFlag::WEXITED)` (no WNOHANG) at runtime.rs:11815, and is called directly from async `spawn_process` (kernel_ops.rs:919, 955) on the broker's tokio executor; a child stuck in uninterruptible sleep blocks that worker indefinitely, and every sibling reap path in this file is explicitly WNOHANG - fix: bounded WNOHANG poll loop (or spawn_blocking) that preserves the no-live-process-left-behind guarantee; route review-pass - [packages/d2b-broker/src/runtime.rs:11815, packages/d2b-broker/src/kernel_ops.rs:919, packages/d2b-broker/src/kernel_ops.rs:955]
  evidence: seeds `async fn|async move|\.await` = 60, `tokio::spawn|spawn_blocking|JoinSet|select!\(|join!\(` = 1, `tokio::sync::(Mutex|RwLock|Notify)` = 5, `#\[tokio::(main|test)\]|Runtime::block_on` = 4; the WNOHANG discipline is documented at runtime.rs:11486-11529 and applied at 11549/11656; no async-gate-allow marker covers this site; kernel_ops.rs:741 `async fn spawn_process` is the enclosing caller
- clean: seeds ran 60/1/5/4 - reaper loop, targeted reap, and notification paths hold no guard across .await and use non-blocking probes; the single blocking-waitid rollback path is the finding above; the block_on at 11498 is startup signal registration (sanctioned)

## unsafe
- clean: seeds ran 5/5/16 - all 5 unsafe blocks live in seccomp_compile_tests.rs (can_set_no_new_privs, two forks, two child closures) and each carries a `// SAFETY:` comment stating the invariant; the 3 `#[allow(unsafe_code)]` sites are on the recorded exception list (U1 d8); the 16 transmute/from_raw/MaybeUninit/zeroed hits are safe nix `Pid::from_raw` constructors, not unsafe operations

## ffi
- N/A (seeds: 0/0/0/0 all zero; no extern "C"/no_mangle/catch_unwind/repr(C)/CStr surface in this part)

## macro
- N/A (seeds: 0/0/0/0 all zero; no macro_rules!/proc-macro/$crate/to_compile_error definitions in this part)

## test
- clean: seeds ran 123/560/0/0 (#[test]/#[tokio::test], assert_eq!/assert_ne!/assert!, proptest!/insta::assert/rstest, #[ignore]) - sampled: 56 of 123 test fns (every 3rd); the suite asserts observable behavior (wire codes, audit records, redaction, rollback semantics, restart-replay refusal, rate-limiter fail-closed caps) with human-written expectations, table-driven cases with failure messages, injected time (`check_at(now)`), and progress-based waits with documented convergence instead of fixed sleeps; no test that cannot fail, no network, no #[ignore]

## Coverage
- idiom: 1 finding(s)
- own: clean (seeds ran: 137/282/0/0; sampled 47 of 419)
- type: clean (seeds ran: 1/0/0)
- api: 1 finding(s)
- err: 1 finding(s)
- serde: clean (seeds ran: 0/0/0/13)
- obs: clean (seeds ran: 5/0/0/35)
- docs: 1 finding(s)
- perf: clean (seeds ran: 89/57/29)
- conc: clean (seeds ran: 10/4/0/0)
- async: 1 finding(s)
- unsafe: clean (seeds ran: 5/5/16)
- ffi: N/A (seeds: 0/0/0/0 all zero; no FFI boundary surface in this part)
- macro: N/A (seeds: 0/0/0/0 all zero; no macro definitions in this part)
- test: clean (seeds ran: 123/560/0/0; sampled 56 of 123 test fns)