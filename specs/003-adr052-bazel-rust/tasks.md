# Tasks: ADR 0052 Under ADR 0054

**Track**: A

**Authority**: ADR 0052 as amended by ADR 0054, this amended Spec 003
artifact set, and committed passing code.

**Base rule**: Start from merged `v3`. Parked historical branches
`spec003-w0-*`, `spec003-w0`, and `adr0054-unified-bazel-spike` are evidence
only; never merge or cherry-pick them.

**Task syntax**:

```text
TNNN [owner: <one scope>] [files: <exact files or none>] [depends: <IDs or none>]
```

The validator compares parsed records with this independently maintained exact
task-ID census:

<!-- D2B-SPEC003-PLAN-TASK-CENSUS:BEGIN -->
T001
T002
T003
T004
T005
T006
T007
T008
T009
T010
T011
T012
T013
T014
T015
T016
T017
T018
T019
T020
T021
T022
T023
T024
T025
T026
T027
T028
T029
T030
T031
T032
T033
T034
T035
T036
T037
T038
T039
T040
T041
T042
T043
T044
T045
T046
T047
T048
T049
T050
T051
T052
T053
T054
T055
T056
T057
T058
T059
T060
T061
T062
T063
T064
T065
T066
T067
T068
T069
T070
T071
T072
T073
T074
T075
T076
T077
T078
T079
T080
T081
T082
T083
T084
T085
T086
T087
T088
T089
T090
T091
T092
T093
T094
T095
T096
T097
T098
T099
T100
T101
T102
T103
T104
T105
T106
T107
T108
T109
T110
T111
T112
T113
T114
T115
T116
T117
T118
T119
T120
<!-- D2B-SPEC003-PLAN-TASK-CENSUS:END -->

The `depends` clause is the dependency graph authority. A task with several
dependencies starts only after all are complete. No task is marked parallel:
parallelism comes only from tasks whose dependency sets are satisfied and
whose plan-owned files are disjoint. Tasks that add tests and implementation
to the same file are explicitly sequential.

## Base and amended plan gate

- [ ] T001 [owner: integrator] [files: none] [depends: none] Verify the base
  contains ADR 0054 commit `a7093601`; record the actual base SHA only under
  `.scratch/spec003w0-base/`; require the two nested product locks to exist;
  require `.bazelversion`, `.bazelrc`, `.bazelignore`, `MODULE.bazel`,
  `MODULE.bazel.lock`, `BUILD.bazel`, and `bazel/` all to be absent.
- [ ] T002 [owner: integrator] [files: none] [depends: T001] Record only tip
  and changed-path inventories for the parked historical branches under
  `.scratch/spec003w0-parked-evidence/`; prove none is an ancestor of the base.
- [ ] T003 [owner: integrator] [files: none] [depends: T002] Run the amended
  read-only plan-structure validator self-tests and positive plan check, then
  run the Track A plan panel over every artifact in this directory using every
  trigger and the applicable floor from the versioned selection table. Dispatch
  exactly the roster and per-seat profiles recorded by the lifecycle selection
  and require unanimous signoff with empty recommendations from every selected
  seat.

## spec003w0 product workspace and foundation

- [ ] T004 [owner: integrator] [files: none] [depends: T003] Run the
  spec003w0 pre-implementation panel against the exact ownership map and refuse
  dispatch until every scope is file-disjoint.
- [ ] T005 [owner: spec003w0-prep] [files:
  packages/d2b-bazel-exec/tests/provider_handle.rs,
  packages/d2b-bazel-exec/tests/verified_executable_api.rs,
  packages/d2b-bazel-exec/tests/execute.rs,
  packages/d2b-bazel-exec/tests/supervisor_protocol.rs,
  packages/d2b-bazel-support/tests/startup.rs,
  packages/xtask/tests/policy_workspace.rs] [depends: T004] Add failing prep
  tests for the unified workspace and for the complete provider, runfiles,
  startup, fake-filesystem, same-descriptor, provider
  `RESOLVE_NO_MAGICLINKS`-only open, escaping runfiles leaf, intermediate
  `O_NOFOLLOW`, permissive provider leaf, `ENOSYS`, and close-on-exec
  contracts. Make `VerifiedExecutable` a compiler-derived capability root with
  an empty inherent API and empty locally-authored explicit-trait allowlist;
  pin its public/hidden and compiler auto/blanket surface and add focused
  rustdoc compile-fail examples for construction, descriptor extraction/access,
  Deref/Borrow/fd traits, Debug/Display, serialization, conversion,
  duplication/default, and minting. Add no Cargo-shelling compile fixture.
  Co-locate `VerifiedExecutable` and the only public consuming API in one
  dependency-leaf crate. Add tests that the safe API consumes by value,
  resolves `d2b-bazel-exec-supervisor` only from the exact immutable Nix
  toolchain output, and uses the pinned reviewed safe `command-fds` dependency
  to map the verified descriptor to a private fd while preserving
  stdin/stdout/stderr. Under an injected process-wide serialization guard,
  test the spawning thread's reviewed safe `nix::sys::signal::SigSet`
  capture, full-managed-set block before helper spawn, and exact prior-mask
  restoration after successful and failed spawn before unlock. Inject capture
  failure, block failure, poisoned-guard refusal, and restoration failure
  after each spawn outcome. Add a deterministic overlapping-launch case and
  mutations proving both launches share one process-wide guard, the second
  cannot capture/block/spawn while the first is inside the handoff, and the
  first attempts restoration before unlock. Add complete
  Rust-parent and C-supervisor stage-error and owner/closure tables. Inject
  missing/wrong/rebound helper identity,
  private-fd identity, descriptor absence, CLOEXEC, stdin, held-open child
  exec-error writer, exact bounded single-record exec-error
  `EINTR`/`EAGAIN`/short/partial/overlong behavior under one absolute
  deadline, and the distinct status-stream decoder with fixed `D2BS`
  header/version/type/length, a 27-byte retained buffer, fragmented reads at
  every boundary, coalesced `READY` plus `EXECUTED` plus terminal frames, and
  malformed-header/version/type/length, duplicate, out-of-order, trailing,
  partial-EOF, and buffer-overflow negatives. The status decoder uses no
  one-byte overlong probe. Inject closed status reader and typed `EPIPE`,
  helper crash/EOF before `EXECUTED`, fast target exit with the same status as
  the crash, ignored or `SA_NOCLDWAIT` inherited `SIGCHLD`,
  a managed `SIG_IGN` refusal before fork, pending-at-entry,
  Rust-to-helper-handoff-window, normalization-time, and blocked `SIGTERM`,
  parent-first/child-first setpgid and initial-trace-stop races; typed
  `ESRCH`/`EPERM`/other-error/group-mismatch/early-child-exit cleanup; exact
  descriptor setup, four-argument
  `ptrace(PTRACE_TRACEME, 0, (void *)0, (void *)0)`, final child signal
  restoration, initial `SIGSTOP`,
  `ptrace(PTRACE_SETOPTIONS, child, (void *)0, (void *)(uintptr_t)PTRACE_O_TRACEEXEC)`,
  `READY` before `ptrace(PTRACE_CONT, child, (void *)0, (void *)0)`, exact kernel
  `PTRACE_EVENT_EXEC`, and
  `ptrace(PTRACE_DETACH, child, (void *)0, (void *)0)` before `EXECUTED`;
  bind request and pid values plus pointer positions/types and add omission,
  integer-in-pointer-position, exchange, wrong-pid, nonchild,
  options-in-address, and nonzero-continue/detach-signal mutations; a
  pending managed signal before group/trace confirmation; pre-`READY`
  termination ownership; and a deterministic initial-stop hold after `READY`
  for every managed signal. Require one coalesced pre-exec termination request,
  no forwarding or grace, immediate helper-owned confirmed-group kill/reap,
  typed `HELPER_PRE_EXEC_TERMINATION`, no `EXECUTED`, no target terminal frame,
  and no target-executed audit event. Add direct pre-exec
  `SIGKILL`, `SIGSYS`, fault, normal-exit, and OOM-like-kill cases; empty EOF
  without an exec event; missing/wrong/plain ptrace events; detach failure; and
  a target that exits on its first instruction after event and detach. Add
  mutations for pre-exec forwarding/escalation, accepting EOF or any/wrong
  stop as exec, accepting detach failure, and false execution/audit publication,
  plus distinct pre-helper Nix/toolchain/sandbox codes and owners for
  unsupported system, minimum kernel, Yama refusal, startup-probe failure, and
  ptrace seccomp-policy drift; distinct post-spawn helper initial-stop,
  options, continuation, event, and detach codes; exact fixed inputs, repairs,
  phase-valid reruns, wrong-remedy tests, the exact
  `TRACEME`/`SETOPTIONS`/`CONT`/`DETACH` seccomp request allowance with only
  enforceable constant pid/address/data constraints, no static future-child
  pid claim, supervisor-owned dynamic child identity, wrong-pid/nonchild
  host-conformance refusal, forbidden ptrace requests, and unchanged action
  no-network,
  target-ignore-TERM escalation with no case deadline,
  signal-forwarding/status mismatch, spawn, close, cleanup, wait, and reap
  failures. The fixed protocol requires `READY`, then `EXECUTED`, then terminal
  target status; no process status is inferred before `EXECUTED`. Rust closes
  owned descriptors and returns the Bazel action nonzero on post-spawn failure;
  it never signals a numeric PID or PGID. Use `std::process`, `command-fds`,
  and `nix::sys::signal::SigSet` only through their safe APIs for Rust
  supervisor spawn and signal handoff. Add no Rust raw fork, `pre_exec`, signal
  handler, disposition mutation, or unsafe exception. Add an
  enforcing closed invocation-site test that permits exactly one Rust typed
  consumer and rejects helper invocation through runfiles, worktree paths,
  other Rust sources, Bazel rules, Make, or workflows. Reject fd-0 executable
  transport, first-party Rust unsafe, and broad lint overrides. These prep
  tests use injected identity/spawn/fd/protocol/containment backends and do not
  execute a Cargo, runfiles, worktree, or sandbox helper. Cargo mocks are not
  crash-containment evidence. The sequential toolchain scope owns the static C
  source, real patched-sandbox crash plants, and host-backed real-output
  conformance. Add a
  mutation that rejects reintroducing
  `RESOLVE_BENEATH` on
  provider opens and retain all three strict resolve flags in result/cleanup
  tests. Do not add future coverage, topology, deadline, cleanup, or recovery
  tests.
- [ ] T006 [owner: spec003w0-prep] [files: packages/Cargo.toml,
  packages/Cargo.lock, packages/d2b-priv-broker/Cargo.toml,
  packages/d2b-priv-broker/Cargo.lock,
  packages/d2b-guest-shell-runner/Cargo.toml,
  packages/d2b-guest-shell-runner/Cargo.lock,
  packages/d2b-contract-tests/Cargo.toml,
  packages/d2b-bazel-support/Cargo.toml,
  packages/d2b-bazel-support/src/lib.rs,
  packages/d2b-bazel-support/src/fsops.rs,
  packages/d2b-bazel-support/src/runfiles.rs,
  packages/d2b-bazel-support/src/startup.rs,
  packages/d2b-bazel-exec/Cargo.toml,
  packages/d2b-bazel-exec/src/lib.rs,
  packages/d2b-bazel-exec/src/provider.rs,
  packages/d2b-bazel-exec/src/execute.rs,
  packages/d2b-bazel-runner/Cargo.toml,
  packages/d2b-bazel-runner/src/lib.rs,
  packages/d2b-test-locator/Cargo.toml,
  packages/d2b-test-locator/src/lib.rs,
  packages/xtask/Cargo.toml,
  packages/xtask/src/main.rs] [depends: T005] Merge broker and
  guest into the resolver-v2 root workspace, delete both nested locks, make
  libshpool normal with an empty code feature, regenerate the root lock
  offline, create and register green runner and locator skeleton crates with
  complete future dependencies and stable spec003w0 crate roots before their first
  tests, declare the complete spec003w0 xtask dependency set, retain a green xtask
  root without declaring not-yet-present modules, implement the
  dependency-leaf owner, safe command-fd mapping, typed supervisor protocol,
  one process-wide serialized safe signal-mask handoff with restore-before-unlock,
  the Rust-side initial-stop/exec-event/detach state model, pre-exec signal
  queuing, helper-owned setup termination, and cleanup behavior required for
  every T005 test to pass. Add the already reviewed pinned `nix` 0.29 `signal`
  feature to that leaf for safe `SigSet` mask operations; add no new signal
  FFI dependency. No Rust helper crate, runner `sys.rs`, raw-fork
  implementation, `pre_exec`, signal-disposition mutation, or first-party Rust
  unsafe allowance exists.
- [ ] T007 [owner: spec003w0-prep] [files: none] [depends: T006] Commit and
  validate prep with locked offline metadata and all T005 tests; open only the
  toolchain scope from this exact green tip.
- [ ] T120 [owner: spec003w0-toolchain] [files: flake.nix,
  pkgs/bazel-8.6.0-seccomp/default.nix,
  pkgs/bazel-8.6.0-seccomp/linux-sandbox-seccomp.patch,
  pkgs/bazel-8.6.0-seccomp/seccomp-policy.json,
  tests/tools/d2b-bazel-exec-supervisor/supervisor.c,
  tests/tools/d2b-bazel-exec-supervisor/sandbox-crash-plant.c,
  pkgs/d2b-bazel-exec-supervisor/default.nix,
  tests/golden/bazel-toolchain.json,
  tests/golden/bazel-exec-supervisor.json,
  tests/unit/nix/cases/bazel-toolchain.nix,
  tests/unit/nix/pinned/common.txt,
  tests/unit/nix/pinned/x86_64-linux.txt,
  tests/unit/nix/pinned/aarch64-linux.txt,
  docs/contributing/critical-subsystems.md,
  packages/d2b-contract-tests/tests/policy_bazel_toolchain.rs] [depends: T007]
  In one sequential toolchain scope, add and observe the exact Nix/package tests
  fail, then package Bazel 8.6.0 with the reviewed Linux sandbox patch and
  fixed policy, make that package the only Bazel in the dev shell, and patch
  its fresh PID-namespace monitor to own abnormal action teardown: namespace
  PID 1 kills every other member and makes nonblocking reap progress. Use one
  fixed 10,000 ms ceiling only for userspace TERM/KILL/monitor escalation and
  the close-or-quarantine decision. If a consuming wait has not proved PID 1
  reaped, outer `linux-sandbox` remains the wait owner, emits
  `pending-kernel-cleanup`, quarantines the sandbox and outputs, and permits
  neither success nor reuse until eventual consuming reap by that original
  live monitor; it remains the sole wait owner and publishes the only valid
  release. Kernel task exit, namespace destruction, and reap have no false
  ten-second bound. Add the exact governed contributing section
  `docs/contributing/critical-subsystems.md#bazel-pending-kernel-cleanup-quarantine`
  with ordered steps to keep the original job/monitor live, inspect the typed
  pending diagnostic, drain the CI worker/provider from new admission without
  termination, wait for and confirm that same monitor's byte-exact
  consuming-reap release, and only then rerun the diagnostic's exact closed
  slice command. State that a GitHub-hosted job-exclusive allocation is
  drained by leaving the original job running with no retry; a shared provider
  without drain-without-terminate stays blocked. Prohibit reboot,
  retry-before-release, replacement wait ownership, and manual release. Build
  the tiny single-threaded C `d2b-bazel-exec-supervisor` as a dedicated static
  immutable build/test-tooling derivation outside the product Rust workspace.
  Commit exact
  Bazel source/patch/policy/capability hashes plus per-native-system output-NAR
  and executable hashes, and exact C source/Nix expression/protocol/static
  compiler/libc/header/dependency-closure hashes plus per-native-system
  derivation/output-NAR/executable/static-ELF hashes, protocol version, Linux
  minimum, supported systems, Yama assumption, and exact static ptrace request
  plus enforceable constant-argument set, never full store paths. Bind both
  native startup-probe and host-conformance results, dynamic child identity,
  wrong-pid/nonchild refusal, every exact ptrace value/position/type test and
  mutation, every
  exec-event negative/mutation, unchanged no-network result, all pre-helper
  Nix/toolchain/sandbox diagnostic bytes and wrong-remedy results, and all new
  helper/child recovery-code bytes into execution evidence and the
  qualification input schema.
  Test exact framed `READY`/`EXECUTED`/`EXITED`/`SIGNALED` status with fixed
  header/version/type/length, retained fragmented and coalesced frames, every
  malformed/duplicate/order negative, and no status-stream overlong probe;
  separately test the single-record close-on-exec exec-error pipe and its
  one-byte overlong check, sole fork, `SIGPIPE` to typed `EPIPE`, waitable
  default `SIGCHLD`, inherited full managed mask, first-operation observation
  of every managed disposition, typed refusal of any managed `SIG_IGN` before
  fork without reset-and-continue, disposition and synchronous-consumer
  installation only after verification, and final-mask establishment. Create
  no confirmation pipe. Make child and supervisor both call `setpgid`; have
  the child finish stdio/CLOEXEC/descriptor closure setup, call
  `ptrace(PTRACE_TRACEME, 0, (void *)0, (void *)0)`, restore its final signal
  mask and dispositions, and only then raise the initial `SIGSTOP` immediately
  before its sole `execveat`. Have the supervisor consume exactly
  that stop, confirm the exact group, install options with
  `ptrace(PTRACE_SETOPTIONS, child, (void *)0, (void *)(uintptr_t)PTRACE_O_TRACEEXEC)`,
  complete `READY`, and release with
  `ptrace(PTRACE_CONT, child, (void *)0, (void *)0)`. Under the original
  absolute deadline, accept
  execution only from exact `WIFSTOPPED`/`SIGTRAP`/`PTRACE_EVENT_EXEC`, then
  detach exactly once with
  `ptrace(PTRACE_DETACH, child, (void *)0, (void *)0)` and emit
  `EXECUTED` only after detach succeeds. Bind request and pid values plus
  pointer positions/types and fail omission, integer-in-pointer-position,
  exchange, wrong-pid, nonchild, options-in-address, and nonzero-signal
  mutations. Empty exec-pipe EOF is closure only. Test pending,
  handoff-window,
  normalization-time, and pre-trace-confirmation `SIGTERM`, pre-`READY`
  termination ownership, every managed signal at the deterministic
  post-`READY` initial stop, one queued pre-exec request, helper-owned group
  kill/reap, typed setup failure, and absence of `EXECUTED`, target terminal,
  grace, and target-executed audit publication. Separately inject pre-exec
  `SIGKILL`, `SIGSYS`, fault, normal exit, OOM-like kill, empty EOF without an
  event, plain/missing/wrong ptrace event, and detach failure; none may
  false-confirm. Prove a target exiting on its first instruction still yields
  event, successful detach, `EXECUTED`, then terminal. Add parent-first/
  child-first confirmation races and typed
  `ESRCH`/`EPERM`/other-error/group-mismatch/early-child-exit cleanup; Linux
  >= 3.19 and native x86_64/aarch64 gates; Yama 0/1 parent-child acceptance and
  2/3 refusal; distinct Nix evaluation, toolchain startup, and patched-sandbox
  owners/codes for unsupported system, old kernel, Yama, failed real startup
  probe, and ptrace policy drift before helper spawn; no `CAP_SYS_PTRACE`; an
  static action seccomp allowance for exactly
  `PTRACE_TRACEME`, `PTRACE_SETOPTIONS`, `PTRACE_CONT`, and `PTRACE_DETACH`
  plus only enforceable constant arguments; no static match of the future
  child pid for `SETOPTIONS`, `CONT`, or `DETACH`; supervisor enforcement from
  its owned fork result, confirmed process group/direct-parent relation,
  traced initial stop, wait ownership, and exact event; native wrong-pid and
  nonchild refusal tests; denial of every other ptrace request; unchanged
  socket/socketcall/io_uring/
  `pidfd_getfd` denial and every existing no-network plant; signal forwarding,
  external-SIGTERM escalation with no case deadline, target wait/reap and exact
  status mirroring, held-open writer, exact single-record exec-error
  `EINTR`/`EAGAIN`/short/partial/overlong transport, fast-same-status target
  versus helper crash, pending/handoff-window/normalization-time/blocked
  SIGTERM and ignored-disposition refusal,
  target-ignore-TERM,
  descriptor absence/private identity/CLOEXEC/stdin, and cleanup on every path.
  Make Nix evaluation own `NIX_PTRACE_SYSTEM`, toolchain startup own the
  kernel/Yama/probe rows, and the patched sandbox own and render every
  `SANDBOX_*` recovery row; keep runner recovery free of all five pre-helper
  rows. Assert each refuses before helper spawn. Byte-test every fixed causing
  input, exact correction, phase-valid rerun, and wrong-remedy mutation across
  all four slices and both diagnostic command versions. For
  `SANDBOX_PTRACE_POLICY`, the only accepted ordered remedy is: restore the
  pinned patch and policy so seccomp admits only the four request values with
  the enforceable constant arguments and retains every no-network denial; run
  `make test-flake`; then run the phase-valid closed slice command selected
  verbatim from the command-version table. Resolve the patch
  path, the full repository-relative runner-contract anchor, and the exact
  contributing runbook file/anchor from the repository root. Byte-test the
  pending diagnostic, runbook link, and consuming-reap release record; reject
  an absent runbook, reboot remedy, retry-before-release, replacement waiter,
  manual release, or non-owner consuming wait. Run real patched-Bazel Linux-sandbox plants for helper crash before `READY`,
  after `READY`, after `EXECUTED`, during grace, and with direct and
  double-forked long-lived descendants. Require consuming namespace and
  outer-monitor reap and liveness-fd EOF on ordinary completion. Add a
  deterministic beyond-ceiling plant whose kernel stall self-resolves after a
  test barrier. It first proves typed
  `pending-kernel-cleanup`, owned quarantine, no false PID-1-reaped claim, no
  success, and no sandbox/output reuse, then proves the original monitor alone
  publishes `complete-after-quarantine` while the action stays failed. No
  release API exists. Namespace
  removal, teardown-patch removal, changed-ceiling, pending-state removal,
  false-reaped, reboot-remedy, retry-before-release, manual-release,
  replacement-waiter, success-after-quarantine, reuse-while-quarantined, and
  every forbidden strategy-fallback mutation must fail without signaling a
  host process.
  The Bazel startup probe, wrong-system, patch-removal, wrong-output,
  policy/filter-load, strategy-fallback, and supervisor
  missing/wrong/rebound/dynamic-output tests must pass. Prove no Rust helper
  crate or Rust unsafe exception exists. Regenerate and commit all three
  Nix-unit presence pins, prove a second `make nix-unit-pin` is a clean no-op,
  and run `make test-nix-unit`. Commit this stable toolchain and pin base before
  T008 opens.

### spec003w0 Bazel generator scope

- [ ] T008 [owner: spec003w0-bazel-generator] [files:
  packages/xtask/tests/bazel_foundation.rs,
  packages/xtask/tests/bazel_module_refresh.rs,
  packages/xtask/tests/package_policy_refusals.rs,
  packages/xtask/tests/bazel_action_network.rs,
  packages/xtask/src/bazel.rs,
  packages/xtask/src/package_policy.rs,
  packages/xtask/src/bazel_yanked.rs] [depends: T120] Add tests before
  implementation for two hubs, exact lock attributes, retired-hub refusals,
  non-mutating repin executor, fresh-tree repin with command-local
  `--lockfile_mode=off`, refusal of that mode after module-lock creation,
  package-policy contexts, exact source census,
  the three-way-join selected-context oracle over target-filtered locked
  offline metadata (identities, sources, candidate edges, `cfg`),
  `packages/Cargo.lock` plus the committed git archive pin (checksums), and
  package-selected `cargo tree` traversals pinned to
  `--locked --offline -p <package> --target <target> --no-default-features`
  with explicit `--features`, `--charset ascii`, `--prefix depth`,
  `--no-dedupe`, and the repository-pinned delimited `--format`, with
  separate production and dev-inclusive traversals, per-row identity
  cross-checks against metadata and the lock, and refusals for unpinned format,
  metadata-sourced checksums, and post-filtered dev edges; the shared-dependency
  feature union refusal, generic/dedicated context selection,
  product-only yanked authority, separate networked refresh and offline check,
  startup capability and configured-target/`aquery`/strategy coverage for
  every stable/nightly Rust compile, build, setup, and test action; exact
  Nix-patched Bazel identity; sandboxed-only execution; process/local/
  standalone/worker/remote refusal; patch-removal, wrong-output, and
  filter-load plants; inherited socket and ordinary/SQPOLL/
  registered-fixed-socket ring refusal; setup-before-payload and all eight
  IPv4/IPv6/netlink/packet/pathname-Unix/abstract-Unix/socketpair/io_uring
  pre-action plants; descendant, external/live-index refusals; exact Cargo
  compatibility census, the
  complete ADR-0054 drift/refusal message table, and no-argument module refresh with
  lock-only mutation, startup-option identity, idempotence, exact remediation,
  and no Make/workflow reachability.
- [ ] T009 [owner: spec003w0-bazel-generator] [files: .bazelversion, .bazelrc,
  MODULE.bazel, BUILD.bazel, bazel/BUILD.bazel,
  bazel/defs.bzl, bazel/toolchains.bzl, bazel/rules/sandboxed_action.bzl,
  bazel/cargo/README.md,
  bazel/cargo/BUILD.bazel, bazel/cargo/cargo_bazel.bzl,
  packages/xtask/src/bazel.rs, packages/xtask/src/package_policy.rs,
  packages/xtask/src/bazel_yanked.rs, packages/xtask/src/schema.rs,
  packages/xtask/src/hermeticity.rs] [depends: T008] Implement the tested
  generator, repin, module-refresh, policy-input, yanked, schema, and
  hermeticity behavior. Require the exact patched-Bazel capability, route every
  governed action through the Linux sandbox, emit complete configured-target,
  `aquery`, and strategy inventories, and reject any non-sandboxed strategy or
  fallback. Keep all repository fetches outside governed actions, offline, and
  pinned. Emit previews only under `.scratch/`; do not create
  the module lock, either hub lock, any BUILD output, pin, or golden.
- [ ] T010 [owner: integrator] [files: none] [depends: T009] Merge the
  generator scope without editing its owned files.
- [ ] T011 [owner: integrator] [files: packages/xtask/src/main.rs,
  bazel/cargo/product.lock, bazel/cargo/walker.lock,
  MODULE.bazel.lock] [depends: T010] Wire the integrated generator modules
  into xtask, then follow the initial-setup refresh order exactly: generate and
  commit the product hub lock, generate and commit the independent walker hub
  lock, and generate and commit `MODULE.bazel.lock` last.
  On this fresh tree only, both initial repins use the tested command-local
  `--lockfile_mode=off` bootstrap because the module lock is absent; neither
  bootstrap may create that lock, and `off` must refuse after module refresh.
  Prove each command changes only its selected output and every second run is
  a clean no-op, then open the remaining independent spec003w0 scopes from
  this exact green generator-checkpoint tip.

### spec003w0 Cargo gate scope

- [ ] T012 [owner: spec003w0-cargo-gates] [files: tests/test-rust.sh,
  tests/tools/assert-pinned-tests.sh]
  [depends: T011] Add failing same-file selector tests for package-only guest
  fmt, locked broker and guest resolving commands, exact generic-main
  exclusions, gate-owned target directories, and serial broker contexts. In the
  same test-first step, add failing assertions that the package supply-chain
  surfaces read the four native selected policy inputs
  (`packages/policy-inputs/<system>/<gnu-target>/broker-production/policy/{metadata.json,Cargo.lock}`
  and
  `packages/policy-inputs/<system>/<musl-target>/guest-real-libshpool/policy/{metadata.json,Cargo.lock}`)
  with an exact source census, deny, and pinned `--no-fetch` audit and no
  retry wrapper; that the aggregate root-lock and `Cargo.guest.lock` checks
  stay independent; that no deleted nested lock path is referenced; and that
  the pinned inventory lists with `--locked` root-lock package selection,
  creates no backup path, registers no restoring `EXIT` trap, and leaves the
  candidate clean under tracked, staged, and untracked assertions.
- [ ] T013 [owner: spec003w0-cargo-gates] [files: tests/test-rust.sh,
  tests/tools/assert-pinned-tests.sh,
  tests/golden/pinned/kernel-canaries.txt,
  tests/golden/pinned/usbip-firewall-skeleton.txt,
  tests/golden/pinned/host-prepare-network.txt,
  tests/golden/pinned/broker-socket-acl.txt,
  tests/golden/pinned/broker-export-audit.txt]
  [depends: T012] Implement the root-workspace command shapes and make every
  T012 assertion pass without changing topology or fixture behavior. Move the
  package deny, audit, and census onto the selected policy inputs; replace the
  nested broker-lock snapshot, restore function, scratch path, and `EXIT`
  trap with the two root-lock listings
  `cargo nextest list --locked --workspace --message-format oneline` and
  `cargo nextest list --locked -p d2b-priv-broker --no-default-features
  --features layer1-bootstrap,fake-backends --message-format oneline`; and
  update only the stale nested-workspace comment headers in the five listed
  `tests/golden/pinned/*.txt` files. Change no pinned entry.

### spec003w0 runner and locator foundation scopes

- [ ] T014 [owner: spec003w0-runner-foundation] [files:
  packages/d2b-bazel-runner/tests/exec_handle.rs] [depends: T011] Add the
  injected adapter test for transfer of the original verified open file
  description after path rebind/mutation, preservation of declared stdin and
  separate stdout/stderr destinations, and exact typed-consumer delegation.
  Require the runner adapter to call only the dependency-leaf typed consumer,
  never the immutable helper directly. Do not execute a Cargo, runfiles, or
  worktree helper; the Nix-policy scope owns host-backed real-output
  conformance. Load the not-yet-wired implementation file through a test-local
  path module.
- [ ] T015 [owner: spec003w0-runner-foundation] [files:
  packages/d2b-bazel-runner/src/exec_handle.rs,
  packages/d2b-bazel-runner/src/bin/d2b-exec-probe.rs] [depends: T014]
  Implement only the green adapter and probe needed by T014 using the
  prep-owned typed consumer. Defer coverage, topology, manifest, result,
  deadline, process, cleanup, and recovery tests and behavior to their owning
  later scopes.
- [ ] T016 [owner: spec003w0-locator-foundation] [files:
  packages/d2b-test-locator/tests/mode_selection.rs] [depends: T011] Add
  failing tests for one-time mode selection, Bazel miss refusal, Cargo
  call-site expansion, and no arm chaining. Load the not-yet-wired
  implementation file through a test-local path module.
- [ ] T017 [owner: spec003w0-locator-foundation] [files:
  packages/d2b-test-locator/src/mode.rs] [depends: T016] Implement the green
  locator foundation required by T016; defer call-site migration to
  spec003w1.

### spec003w0 Nix, policy, CI, and binding documentation scopes

- [ ] T018 [owner: spec003w0-nix-policy] [files:
  tests/unit/nix/cases/bazel-package-policy.nix,
  packages/d2b-contract-tests/tests/policy_bazel_nix.rs,
  packages/d2b-contract-tests/tests/policy_bazel_supply_chain.rs] [depends: T013]
  Add failing tests for root source/lock selection, exact package and
  feature flags, dedicated derivations, both exact `wl-proxy-0.1.2` hash
  values, four system/target contexts, six narrow license exceptions, pinned
  no-fetch audit, generic Nix build/test and Clippy broker/guest exclusions,
  dedicated-context exact selectors, expected native `e_machine`, `ET_DYN`,
  no `PT_INTERP`, no `DT_NEEDED`, and all missing/wrong/one-sided pin,
  non-PIE, and wrong-machine mutations. Add failing tests for realization of
  both dedicated derivations, exact broker interpreter and sorted `DT_NEEDED`
  SONAME set, transient exact recursive broker/guest Nix closures with only
  persisted counts/digests, selected-policy digests, and exactly four measured
  executable baseline rows. Add unchanged-size and exact-approved-growth
  positives where the positive delta exists only in
  `sizeGrowthAuthorization`, plus missing/denied/stale/replayed/
  wrong-system-or-artifact/arithmetic/absolute-rationale/wrong-prior-baseline/
  wrong-realized-new-bytes/duplicate-allowance-source/size-plus-one
  authorization negatives. Add
  changed-SONAME/interpreter, static-broker, dynamic-guest,
  closure-add/remove, cross-artifact, and unrelated-sibling mutations. Assert
  that no diagnostic emits a store path and neither deleted nested
  lock path remains an input to the aggregate flake audit or to the
  guest-shell-runner static dependency policy, and that the aggregate
  `packages/Cargo.lock` and `packages/Cargo.guest.lock` deny and audit
  checks remain independent and enforcing. Bind the prep-owned Bazel and
  static-supervisor identity records into the integrated Nix/package-policy
  tests without changing them. Run the host-backed real supervisor output to
  prove provider-path rebind resistance, same-open-file-description execution,
  private-fd identity, declared stdin and split stdout/stderr, target
  descriptor absence, exact framed `READY`/`EXECUTED`/terminal transport,
  fragmented/coalesced and malformed/duplicate/order cases, held-open writer
  refusal, closed-reader typed `EPIPE`, exact single-record exec-error
  `EINTR`/`EAGAIN`/short/partial/overlong transport, fast-same-status
  target/helper discrimination, ignored/`SA_NOCLDWAIT` SIGCHLD normalization,
  serialized safe spawning-thread mask capture/block/restore after successful
  and failed spawn, capture/block/poison/restoration failures and overlapping
  restore-before-unlock mutation coverage, inherited-mask verification, managed-`SIG_IGN` refusal
  before fork, pending/handoff-window/normalization-time/blocked SIGTERM,
  parent-first/child-first setpgid and initial-stop races, typed
  `ESRCH`/`EPERM`/other-error/group-mismatch/early-child-exit cleanup, exact
  options/continue/event/detach order, pending signal before group/trace
  confirmation, pre-`READY` ownership, deterministic post-`READY` pre-exec
  managed-signal setup termination, direct pre-exec
  `SIGKILL`/`SIGSYS`/fault/exit/OOM-like kill, empty EOF without event,
  missing/wrong event, detach failure, fast first-instruction exit, no false
  `EXECUTED` or audit publication, native/minimum-kernel/Yama and exact ptrace
  seccomp request gates with unchanged action no-network, no-deadline
  external-TERM escalation,
  target-ignore-TERM, allowed-signal forwarding, exact target status, and
  terminal child reap. Use reviewed C test tooling for disposition planting;
  add no Rust unsafe.
- [ ] T019 [owner: spec003w0-nix-policy] [files:
  nixos-modules/host-broker.nix, flake.nix,
  packages/d2b-guest-shell-runner/deny.toml] [depends: T018] Implement the
  dedicated root-lock derivations and package checks, retain the exact output
  hash in both derivations, preserve exact generic/dedicated contexts, drop
  both deleted nested lock inputs from the aggregate audit and repoint the
  guest-shell-runner static dependency policy at
  `guest-real-libshpool/production/{closure.json,Cargo.lock}` while reserving
  `policy/{metadata.json,Cargo.lock}` for deny and audit, keep the aggregate
  root and `Cargo.guest.lock` checks independent, preserve the prep-owned
  patched Bazel and immutable static supervisor outputs as the only governed
  toolchain, and make the T018 tests pass without changing their identity pins.
  Add `broker-host-artifact-contract`; extend `guest-static-elf` to realize the
  actual guest derivation and enforce the exact artifact-baseline row and
  closed size-growth authorization. Do not persist a store path or invent a
  byte ceiling or row-level allowance field.
- [ ] T020 [owner: integrator] [files:
  tests/unit/nix/pinned/common.txt,
  tests/unit/nix/pinned/x86_64-linux.txt,
  tests/unit/nix/pinned/aarch64-linux.txt] [depends: T019] Run
  `make nix-unit-pin` after the later T018/T019 cases, commit only any resulting
  changes to the same three presence pins first generated by T120, rerun it
  under clean-diff assertions, and run `make test-nix-unit`. A no-change result
  is valid; this task does not weaken T120's pre-T008 pin and test requirement.
- [ ] T021 [owner: spec003w0-policy-ci] [files:
  tests/unit/meta/ci-runner-regression.py,
  tests/unit/gates/flake-check-matrix-sync.sh,
  tests/unit/gates/ci-rust-cache-sync.sh] [depends: T019] Add failing
  regressions proving each of the three new fixture-independent policy binaries
  is absent from the shared list, plus extra and duplicate membership
  fixtures, and proving the native arm renderer lacks the six realizations,
  `make test-rust-supply-chain`, and stable-head binding. Update the two
  existing fail-closed gates with failing predicates for the unified release
  root manifest, collapsed cache workspace, explicit gate directories, and
  native arm six-realization shape; add a failing advisory-classification
  mutation for `test-flake-aarch64`; do not delete or retire either gate.
- [ ] T022 [owner: spec003w0-policy-ci] [files: tests/lib.sh,
  packages/xtask/tests/policy_ci.rs,
  packages/d2b-contract-tests/tests/policy_docs.rs,
  tests/unit/meta/w0-dep-direction.sh] [depends: T021] Add exactly the three
  new fixture-independent `policy_bazel_toolchain`, `policy_bazel_nix`, and
  `policy_bazel_supply_chain` binaries once each to the fail-closed
  `test-policy` inventory in `tests/lib.sh`; satisfy fixture exclusion,
  contributor-mutation reachability, exact dependency-direction,
  repository-command, and no-process-marker policy tests; and make isolated
  missing, extra, and duplicate inventory fixtures fail.
- [ ] T023 [owner: spec003w0-policy-ci] [files:
  tests/layer1-jobs.json, tests/tools/layer1-jobs.py,
  tests/ci/layer1-workflow.template.yml,
  tests/tools/flake-check-classes.sh,
  tests/tools/gen-flake-check-matrix-pin.sh,
  .github/workflows/release-host-binaries.yml] [depends: T021,T022] Implement
  native `test-flake-aarch64` with six realizations plus
  `make test-rust-supply-chain`, 60-minute bound, renderer coverage, stable-head
  binding, explicit non-advisory enforcement, and
  wrong-runner/foreign-system/remote-builder refusals. Update the
  release workflow to the root manifest, `--locked`, explicit package/bin/
  default-feature selectors, `packages/target/release`, one `packages ->
  target` workspace cache mapping, and explicit gate target directories; make
  both retained T021 gates pass.
- [ ] T024 [owner: spec003w0-binding-docs] [files: AGENTS.md,
  tests/AGENTS.md, CONTRIBUTING.md, docs/contributing/gates-and-lints.md,
  docs/contributing/workflow.md,
  docs/contributing/critical-subsystems.md,
  docs/adr/0052-bazel-rust-build-and-test.md, docs/adr/README.md,
  changelog.d/adr0054-broker-hub.md,
  packages/d2b-contract-tests/tests/policy_modules.rs] [depends: T009,T013,T015,T017,T020,T022,T023]
  Update unified-workspace, lock, target,
  release, and native-arm gate language in the same implementation wave; use
  semantic wording and no process markers. Change ADR 0052's amendment label,
  the ADR index summary, and the ADR 0054 changelog fragment from proposed to
  accepted and replace their retired four-hub summary. State that ADR 0054
  governs the newer workspace shape and do not edit dated ADR 0038. Preserve
  the earlier T120-owned
  `docs/contributing/critical-subsystems.md#bazel-pending-kernel-cleanup-quarantine`
  section byte-for-byte and keep its repository-link existence check green.

### spec003w0 integration, validation, and merge

- [ ] T025 [owner: integrator] [files:
  packages/d2b-bazel-runner/src/lib.rs,
  packages/d2b-test-locator/src/lib.rs] [depends:
  T009,T013,T015,T017,T020,T022,T023,T024] Merge every scope, wire only the
  integrated runner and locator implementation modules into their prep-owned
  crate roots, and require the committed root, product-hub, walker-hub, and
  module locks to remain current and the walker Cargo and Bazel-side locks to
  remain byte-identical.
- [ ] T026 [owner: integrator] [files:
  packages/policy-inputs/x86_64-linux/x86_64-unknown-linux-gnu/broker-production/production/closure.json,
  packages/policy-inputs/x86_64-linux/x86_64-unknown-linux-gnu/broker-production/production/Cargo.lock,
  packages/policy-inputs/x86_64-linux/x86_64-unknown-linux-gnu/broker-production/policy/metadata.json,
  packages/policy-inputs/x86_64-linux/x86_64-unknown-linux-gnu/broker-production/policy/Cargo.lock,
  packages/policy-inputs/x86_64-linux/x86_64-unknown-linux-musl/guest-real-libshpool/production/closure.json,
  packages/policy-inputs/x86_64-linux/x86_64-unknown-linux-musl/guest-real-libshpool/production/Cargo.lock,
  packages/policy-inputs/x86_64-linux/x86_64-unknown-linux-musl/guest-real-libshpool/policy/metadata.json,
  packages/policy-inputs/x86_64-linux/x86_64-unknown-linux-musl/guest-real-libshpool/policy/Cargo.lock,
  packages/policy-inputs/aarch64-linux/aarch64-unknown-linux-gnu/broker-production/production/closure.json,
  packages/policy-inputs/aarch64-linux/aarch64-unknown-linux-gnu/broker-production/production/Cargo.lock,
  packages/policy-inputs/aarch64-linux/aarch64-unknown-linux-gnu/broker-production/policy/metadata.json,
  packages/policy-inputs/aarch64-linux/aarch64-unknown-linux-gnu/broker-production/policy/Cargo.lock,
  packages/policy-inputs/aarch64-linux/aarch64-unknown-linux-musl/guest-real-libshpool/production/closure.json,
  packages/policy-inputs/aarch64-linux/aarch64-unknown-linux-musl/guest-real-libshpool/production/Cargo.lock,
  packages/policy-inputs/aarch64-linux/aarch64-unknown-linux-musl/guest-real-libshpool/policy/metadata.json,
  packages/policy-inputs/aarch64-linux/aarch64-unknown-linux-musl/guest-real-libshpool/policy/Cargo.lock,
  tests/golden/bazel-rust-artifact-baselines.json]
  [depends: T025] Generate and review the exact four production/policy context
  trees; realize both dedicated derivations on both native systems; generate
  exactly four linkage, closure-count/digest, selected-policy, and measured
  binary-size baseline rows with null initial size authorization and no store
  paths; run both size-authorization positives and every negative; rerun
  `--check` and require a clean diff.
- [ ] T027 [owner: integrator] [files: .bazelignore,
  bazel/generated/BUILD.bazel,
  bazel/generated/action-network-policy.json,
  bazel/generated/configured-targets.json,
  bazel/generated/evidence-sink-policy.json,
  bazel/generated/no-shell-inventory.json,
  bazel/generated/output-manifest.json,
  bazel/generated/package-policy-targets.bzl,
  bazel/generated/product-targets.bzl,
  bazel/generated/source-census.json,
  tests/golden/api-surface/roots.json,
  tests/golden/api-surface/capability-api.txt,
  tests/golden/api-surface/capability-trait-impls.txt,
  tests/golden/api-surface/hidden-public-api.txt,
  tests/golden/api-surface/public-api.txt,
  tests/golden/bazel-rust-coverage.json,
  tests/golden/bazel-rust-query.json] [depends: T025] Generate first-party
  target definitions and coverage/query seeds once as the closed exact
  integrator-owned output set; run `make api-surface-pin` so
  `VerifiedExecutable` becomes a capability root with the reviewed exact
  snapshots, rerun `gen-bazel --check` and the API checker, and require a clean
  diff.
- [ ] T028 [owner: integrator] [files:
  tests/golden/flake-check-matrix/x86_64-linux.txt,
  tests/golden/flake-check-matrix/aarch64-linux.txt,
  .github/workflows/pr-l1-static-fast.yml] [depends: T023,T025] Regenerate
  both flake inventories and the workflow, then prove every generator is a
  clean no-op.
- [ ] T029 [owner: integrator] [files: Makefile,
  changelog.d/adr052-bazel-foundation.md] [depends: T026,T027,T028] Wire only
  approved existing gates, add a semantic fragment, and commit the exact
  candidate.
- [ ] T030 [owner: integrator] [files: none] [depends: T029] Run the complete
  spec003w0 validation from `plan.md` in its exact order, including clean-diff
  assertions around product-lock generation, product repin, walker repin,
  module refresh last, Nix-unit pin regeneration, and every generator. Run the
  full `make test-rust` aggregate in addition to focused checks and the
  explicit fixture-contract lane.
- [ ] T031 [owner: integrator] [files: none] [depends: T030] Obtain native x86
  and arm results on one stable PR head; require arm six-check realization and
  `make test-rust-supply-chain` plus renderer coverage.
- [ ] T032 [owner: integrator] [files: none] [depends: T031] Run the
  selected-roster integrated-diff panel using exactly the roster and per-seat
  profiles recorded by the lifecycle selection. Across fix verification, rerun
  selection over the full candidate and every fix delta and only widen the
  lifecycle roster. Any content fix invalidates affected validation and panel
  records.
- [ ] T033 [owner: integrator] [files: none] [depends: T032] Seal
  `spec003w0`, merge to protected `v3`, record the merged SHA, collect garbage,
  and remove finished worktrees.

## spec003w1 complete Bazel carriers

- [ ] T034 [owner: integrator] [files: none] [depends: T033] Run the
  spec003w1 plan panel and require unanimous empty recommendations.
- [ ] T035 [owner: integrator] [files:
  packages/d2b-bazel-runner/Cargo.toml,
  packages/d2b-bazel-runner/src/lib.rs,
  packages/d2b-bazel-runner/src/contracts.rs,
  packages/d2b-test-locator/Cargo.toml,
  packages/d2b-test-locator/src/lib.rs,
  packages/d2b-test-locator/src/contracts.rs,
  packages/xtask/Cargo.toml,
  packages/xtask/src/main.rs,
  packages/Cargo.lock,
  bazel/cargo/product.lock,
  MODULE.bazel.lock,
  bazel/generated/locator-migration-files.json] [depends: T034] Land complete
  green shared interfaces, complete future dependencies, crate-root and xtask
  contract seams that do not declare not-yet-present implementation modules,
  and the exact sorted locator file inventory before scopes add modules. If a
  product manifest changes, regenerate `packages/Cargo.lock`, product repin,
  and module refresh in that order, commit each generated result, prove all
  three are clean no-ops, and prove the walker Cargo lock and
  `bazel/cargo/walker.lock` byte-identical. If a walker manifest or lock
  changes, regenerate the walker Cargo lock, walker repin, and module refresh
  in that order and prove `packages/Cargo.lock` and
  `bazel/cargo/product.lock` byte-identical. `MODULE.bazel.lock` is always
  committed last. No parallel scope may edit these files.
- [ ] T036 [owner: spec003w1-main] [files:
  packages/d2b-bazel-runner/tests/main_topology.rs] [depends: T035] Add failing
  main carrier, process-per-case, doctest, harness-free, census, and result
  assertions.
- [ ] T037 [owner: spec003w1-main] [files: bazel/carriers/main.bzl]
  [depends: T036] Implement the main carriers and satisfy T036.
- [ ] T038 [owner: spec003w1-api] [files:
  bazel/rules/tests/channel_transition.rs,
  bazel/rules/tests/rustdoc_json.rs] [depends: T035] Add failing per-target
  nightly, emitted-version, rustdoc JSON, and global-channel refusal tests.
- [ ] T039 [owner: spec003w1-api] [files:
  bazel/rules/channel_transition.bzl, bazel/rules/rustdoc_json.bzl] [depends: T038]
  Implement the API rules and satisfy T038.
- [ ] T040 [owner: spec003w1-broker] [files:
  packages/d2b-bazel-runner/tests/broker_topology.rs,
  packages/d2b-bazel-runner/tests/broker_exclusive.rs] [depends: T035] Add
  failing exact-census, per-binary, bounded-thread, literal
  `tags = ["exclusive"]`, no-overlap, and tag-removal mutation tests for all
  three contexts.
- [ ] T041 [owner: spec003w1-broker] [files:
  bazel/carriers/broker.bzl] [depends: T040] Implement all three broker
  carriers and satisfy T040.
- [ ] T042 [owner: spec003w1-guest] [files:
  packages/d2b-bazel-runner/tests/guest_topology.rs] [depends: T035] Add
  failing guest configured-target, process-per-case, companion, and census
  tests.
- [ ] T043 [owner: spec003w1-guest] [files: bazel/carriers/guest.bzl]
  [depends: T042] Implement the guest carriers and satisfy T042.
- [ ] T044 [owner: spec003w1-supply-chain] [files:
  packages/xtask/tests/bazel_yanked.rs,
  packages/xtask/tests/bazel_action_network.rs] [depends: T035] Add failing
  product-only snapshot, full-main, exact broker/guest projection, reviewed
  refresh, offline check, exact patched-Bazel startup capability plus
  configured-target, `aquery`, and strategy coverage for stable/nightly Rustc,
  metadata, Clippy, rustdoc, doctest compile/run, rustfmt, unpretty,
  build-script, repository, setup, and test actions; patch-removal,
  wrong-output, filter-load, process/local/standalone/worker/remote, and every
  strategy-fallback plant; inherited socket, ordinary-ring, SQPOLL-ring, and
  registered-fixed-socket-ring refusal; setup-before-payload and the eight
  IPv4/IPv6/netlink/packet/pathname-Unix/abstract-Unix/socketpair/io_uring
  pre-action plants; descendant, live-index, and forbidden external-egress;
  exact mandatory
  socket-test Cargo
  compatibility census, same-commit non-advisory verdict, missing/skipped/
  advisory/wrong-head/misattributed compatibility negatives, pinned-fetch
  inventory, source-census, deny,
  audit, Cargo/decomposed-Bazel raw-status and normalized-finding equivalence,
  finding-class, missing-union-leg, projection-swap, extra-finding, and
  status-difference tests.
- [ ] T045 [owner: spec003w1-supply-chain] [files:
  bazel/vendor/repositories.bzl, bazel/supply_chain/BUILD.bazel,
  bazel/supply_chain/defs.bzl,
  packages/xtask/src/bazel_yanked.rs] [depends: T044] Implement the offline
  supply-chain carriers, equivalence comparison, action-network evidence, and
  exact Cargo compatibility carriers. Require the exact patched-Bazel package
  and sandboxed-action factory already landed in spec003w0; keep repository
  fetches outside governed actions and offline/pinned; do not assign a socket
  plant to the stub carrier or claim namespaces deny socket creation. Emit the
  yanked snapshot only as a scratch preview.
- [ ] T046 [owner: spec003w1-runner] [files:
  packages/d2b-bazel-runner/tests/coverage.rs,
  packages/d2b-bazel-runner/tests/result_publication.rs,
  packages/d2b-bazel-runner/tests/provider_execution.rs] [depends: T035] Add
  failing exact coverage, topology, environment, prior-evidence invalidation,
  multi-carrier attribution, success/failure/handled-interruption sorted atomic
  manifest v1 publication, original-status preservation, ignored-case
  fidelity, every-forbidden-value redaction fixture across JUnit, bounded
  `test.log`, emitted manifest/qualification evidence, and exporter
  diagnostics,   typed `testVerdict` plus one common `sinkKind`/`retentionClass` pair and
  structurally closed tagged complete/degraded `evidenceStatus`, unchanged
  manifest-v1 schema, sink
  byte/record limits, `junit-v1`, `test-log-v1`, `evidence-v1`, and
  `exporter-diagnostic-v1` age/count retention and expiry failures, exact
  publication-remediation rows, enforcing
  complete evidence,
  no-shell, `D2B_RUST_BUDGET` validation/propagation/combined-limit,
  same-open-file-description safe-by-value typed-consumer execution through
  the exact immutable static C supervisor and reviewed command-fd mapping,
  `RESOLVE_NO_MAGICLINKS`-only provider,
  provider-`RESOLVE_BENEATH` rejection, permissive fallback leaf,
  declared-stdin preservation, auxiliary-CLOEXEC, no-provider-fd leak, no
  second Rust helper invocation, no runfiles/worktree helper, no target
  path/fd-0 transport, exact framed
  `READY`/`EXECUTED`/`EXITED`/`SIGNALED` protocol with retained fragmented and
  coalesced reads and malformed/duplicate/order negatives, single-record
  exec-error EOF/overlong behavior, held-open/closed-reader/exact partial
  transport, helper crash versus fast same-status target, waitable SIGCHLD,
  serialized safe mask handoff, capture/block/poison/restoration failures,
  overlapping-launch restore-before-unlock mutation, inherited managed
  `SIG_IGN` refusal, handoff-window and normalization-time SIGTERM,
  parent/child setpgid and initial-trace-stop races, typed
  `ESRCH`/`EPERM`/early-child-exit cleanup, exact options/continue/event/detach
  order, pending signal before group/trace confirmation, pre-`READY`
  termination ownership, deterministic post-`READY` pre-exec signal queuing,
  pre-exec death/fault/OOM-like kill, empty EOF without event, missing/wrong
  event, detach failure, fast first-instruction exit, native/kernel/Yama gates,
  exact static ptrace request/constant-argument allowance, supervisor-owned
  dynamic child identity, wrong-pid/nonchild refusal, unchanged no-network,
  helper kill/reap,
  no false
  `EXECUTED`/target terminal/audit event, full post-`EXECUTED` signal forwarding,
  no-deadline external-TERM escalation,
  target-ignore-TERM, target-status mirroring, no-numeric-Rust-signal, and
  no-fallback mutations.
  Each test loads its scope-owned not-yet-wired implementation modules through
  test-local paths.
- [ ] T047 [owner: spec003w1-runner] [files:
  packages/d2b-bazel-runner/src/coverage.rs,
  packages/d2b-bazel-runner/src/topology.rs,
  packages/d2b-bazel-runner/src/runner_env.rs,
  packages/d2b-bazel-runner/src/junit.rs,
  packages/d2b-bazel-runner/src/manifest.rs] [depends: T046] Implement the
  runner behavior, pre-sink streaming sanitizer, tagged evidence union,
  descriptor-relative retention expiry, and bounded sink policy and satisfy
  T046 without changing manifest v1.
- [ ] T048 [owner: spec003w1-locator] [files:
  packages/d2b-test-locator/tests/locator.rs] [depends: T035] Add failing
  tests for every prep-frozen migration disposition, Bazel miss, stale Cargo
  provider, rebound path, and no fallback.
  The test loads `src/locator.rs` through a test-local path until integration
  wires it into the crate root.
- [ ] T049 [owner: spec003w1-locator] [files:
  packages/d2b-test-locator/src/locator.rs] [depends: T048] Implement the
  locator. Verify the prep-frozen inventory marks existing Cargo-only call
  sites retained and names no additional changed path; do not edit the
  inventory.
- [ ] T050 [owner: spec003w1-no-bash] [files:
  tests/tools/no-bash-ast-walker/src/main.rs] [depends: T035] Add inline
  failing tests for walk, open, read, and parse errors and for equality among
  governed manifest, declared inputs, and parsed-file census.
- [ ] T051 [owner: spec003w1-no-bash] [files:
  tests/tools/no-bash-ast-walker/src/main.rs,
  bazel/carriers/no_bash.bzl] [depends: T050] Implement the fail-closed walker
  and its separate carrier and satisfy T050.
- [ ] T052 [owner: spec003w1-census-generator] [files:
  packages/xtask/tests/bazel_generation.rs,
  packages/d2b-bazel-runner/tests/schema_inventory.rs] [depends: T035] Add
  failing native-target, source, companion, fragment, query,
  stale-generation, two-independent-nonempty-schema-generation, schema
  mismatch/empty, stub missing-executable/wrong-identity/runtime-state, and
  inventory empty/missing/extra tests. Add failing
  tests for the generated no-shell inventory: governed and declared sets are
  nonempty and equal in both directions; every spawn source is governed; every
  governed source has exactly one successful scan record, including planted
  zero-site sources; raw scan-record count and unique scan-source count each
  equal governed-source count; a fresh scan's
  exact keyed spawn-site set equals the committed `spawnSites` set in both
  directions; every spawn site has a false `shellInvocation` verdict; every
  governed source records a scan result; and the `no-shell-inventory-empty`,
  `no-shell-inventory-missing-entry`, `no-shell-inventory-extra-entry`,
  `no-shell-inventory-unguarded-spawn`,
  `no-shell-inventory-missing-zero-site-record`, and
  `no-shell-inventory-planted-shell` plants each fail at their own
  diagnostic. Assert that all socket-denial plants belong only to the
  hermeticity/action-network carrier and none belongs to `stub.bzl`.
- [ ] T053 [owner: spec003w1-census-generator] [files:
  packages/xtask/src/bazel.rs, packages/xtask/src/schema.rs,
  bazel/carriers/schema.bzl,
  bazel/carriers/stub.bzl, bazel/carriers/inventory.bzl] [depends: T052]
  Implement spec003w1 generation and the three file-disjoint carriers without writing
  shared generated outputs from the slice. Generation includes
  `bazel/generated/no-shell-inventory.json` with its governed-source,
  declared-input, per-source scan-result, and spawn-site sets; also generate
  the measured `bazel/generated/evidence-sink-policy.json`. The slice emits
  both only as a
  `.scratch/` preview.
- [ ] T054 [owner: spec003w1-coverage] [files:
  packages/d2b-bazel-runner/tests/coverage_map.rs,
  packages/xtask/tests/policy_ci.rs] [depends: T035] Add failing coverage
  guards for exactly eighteen surfaces, total carriers, exact censuses,
  broker tags, narrowed action-network fields, separate schema/stub/inventory/
  no-bash files, fragments, queries, the no-bash census, and the nonempty
  governed/declared-equal, spawn-subset, zero-site-record, raw-count, and
  unique-count no-shell inventory. Reject any socket plant assigned to the
  stub carrier.
  In `policy_ci.rs`, add all six shadow
  targets (`test-bazel-rust`, `test-bazel-rust-main`,
  `test-bazel-rust-api`, `test-bazel-rust-broker`, `test-bazel-rust-aux`,
  and `bazel-shutdown`) to `APPROVED_MAKE_TARGETS`, with a positive test
  over a supplied Makefile fixture proving each approved shadow name resolves
  to a rule and a workflow step calling it is accepted, and negative fixtures
  proving an unapproved `test-bazel-rust-<name>` call and an approved shadow
  name with no Makefile rule are both rejected. T057 applies the same
  consistency assertion to the integrated repository Makefile after T056 adds
  the real entry points. No other spec003w1 scope edits `policy_ci.rs`.
- [ ] T055 [owner: spec003w1-coverage] [files:
  bazel/carriers/coverage.bzl] [depends: T054] Implement the coverage guard and
  make every T054 case pass without committing a golden.
- [ ] T056 [owner: integrator] [files: Makefile, ci/rust/BUILD.bazel,
  packages/d2b-bazel-runner/src/lib.rs,
  packages/d2b-test-locator/src/lib.rs,
  packages/xtask/src/main.rs]
  [depends: T037,T039,T041,T043,T045,T047,T049,T051,T053,T055] Integrate the
  six shadow Make entry points (`test-bazel-rust`, `test-bazel-rust-main`,
  `test-bazel-rust-api`, `test-bazel-rust-broker`, `test-bazel-rust-aux`,
  and `bazel-shutdown`) matching the names T054 approved, wire the completed
  runner, locator, generator, schema, and manifest modules into their
  prep-owned roots, and keep Cargo authoritative.
- [ ] T057 [owner: integrator] [files: .bazelignore,
  bazel/generated/BUILD.bazel,
  bazel/generated/action-network-policy.json,
  bazel/generated/configured-targets.json,
  bazel/generated/evidence-sink-policy.json,
  bazel/generated/no-shell-inventory.json,
  bazel/generated/output-manifest.json,
  bazel/generated/package-policy-targets.bzl,
  bazel/generated/product-targets.bzl,
  bazel/generated/source-census.json,
  tests/golden/bazel-rust-coverage.json,
  tests/golden/bazel-rust-query.json,
  bazel/supply_chain/yanked-snapshot.json,
  changelog.d/adr052-bazel-carriers.md] [depends: T056] Regenerate
  integrator-owned shared outputs once, including
  the exact closed nine-file generated set, add measured bounds and the four
  retention classes, add the semantic spec003w1 fragment, commit the candidate,
  rerun checks, and require a clean diff.
- [ ] T058 [owner: integrator] [files: none] [depends: T057] Run all
  spec003w1 validation, every carrier mutation, all eight socket/io_uring
  plants plus external-egress/live-index, the provider/toolchain/strategy
  inventory and fallback mutations,
  exact Cargo compatibility carriers, exact
  Cargo/Bazel census and
  supply-chain-equivalence comparisons, and fixture contracts; then run the
  integrated-diff panel, seal `spec003w1`, merge, collect garbage, and remove
  finished worktrees.

## spec003w2 operational safety

- [ ] T059 [owner: integrator] [files: none] [depends: T058] Run the
  spec003w2 plan panel over safety, recovery, and cache contracts.
- [ ] T060 [owner: integrator] [files:
  packages/d2b-bazel-runner/Cargo.toml,
  packages/d2b-bazel-runner/src/lib.rs,
  packages/d2b-bazel-runner/src/clock.rs,
  packages/d2b-bazel-runner/src/process_backend.rs,
  packages/d2b-bazel-runner/tests/process_backend_contract.rs,
  packages/xtask/Cargo.toml,
  packages/xtask/src/main.rs,
  packages/d2b-bazel-support/src/startup.rs,
  packages/d2b-bazel-support/tests/startup_contract.rs,
  packages/Cargo.lock, bazel/cargo/product.lock,
  MODULE.bazel.lock] [depends: T059] Land complete green clock,
  process-backend, startup, crate-root, and xtask contract seams without
  declaring not-yet-present spec003w2 implementation modules. If a product
  manifest changes, regenerate `packages/Cargo.lock`, product repin, and
  module refresh in that order, commit each generated result, prove clean no-op
  reruns, and prove both walker inputs byte-identical. If a walker manifest or
  lock changes, regenerate the walker Cargo lock, walker repin, and module
  refresh in that order and prove both product inputs byte-identical.
  `MODULE.bazel.lock` is always committed last. No slice edits these files.
- [ ] T061 [owner: spec003w2-process] [files:
  packages/d2b-bazel-runner/tests/deadline.rs,
  packages/d2b-bazel-runner/tests/process.rs] [depends: T060] Add failing
  parser, rounding, repeated `EXITED|NOWAIT|NOHANG`, full-grace,
  informational-observation, unconditional-kill, final-reap, blocking-wait,
  early-reap, shortened-grace, and conditional-kill tests.
  Add missing-`process_group(0)`, wrapper-group, group-zero, group-minus-one,
  and PID-file-decoy mutations; require the sibling process and decoy process
  to remain alive while only the dedicated child group is signalled.
  Load the not-yet-wired implementation modules through test-local paths.
- [ ] T062 [owner: spec003w2-process] [files:
  packages/d2b-bazel-runner/src/deadline.rs,
  packages/d2b-bazel-runner/src/process.rs] [depends: T061] Implement deadline
  and escalation behavior and satisfy T061.
- [ ] T063 [owner: spec003w2-cleanup] [files:
  packages/d2b-bazel-runner/tests/cleanup.rs,
  packages/d2b-contract-tests/tests/policy_docs.rs] [depends: T060] Add failing
  strict-route, leaf, race, tracked, live, descriptor-inheritance, and
  path-recursive-removal tests.
  Load the not-yet-wired cleanup module through a test-local path.
- [ ] T064 [owner: spec003w2-cleanup] [files:
  packages/d2b-bazel-runner/src/cleanup.rs] [depends: T063] Implement
  descriptor-relative cleanup and satisfy T063.
- [ ] T065 [owner: spec003w2-local-wrapper] [files:
  packages/d2b-bazel-support/tests/startup.rs] [depends: T060] Add failing
  startup-option identity, scratch limit, synchronous trim, and high-water
  tests.
- [ ] T066 [owner: spec003w2-local-wrapper] [files: Makefile, .bazelrc]
  [depends: T065] Implement the one startup construction, bounded local state,
  synchronous trim, and Make integration.
- [ ] T067 [owner: spec003w2-recovery] [files:
  packages/d2b-bazel-runner/tests/recovery.rs] [depends: T060] Add table-driven
  exact ADR 0052 command, redaction, missing-remedy, borrowed-remedy,
  external-target, replacement-directory, recursive-remove, manual-signal,
  and ceiling-remedy mutations. Add exact nonzero ADR-0054 rows for stale
  product and walker Cargo locks with their full Cargo-lock, matching-hub, and
  final-module refresh sequences; stale product/walker hub locks, module lock,
  generator output, package-policy output, yanked snapshot, ambient repin
  controls, and unexpected tracked mutation, including the two-step shell
  context and exact
  command/review/rerun sequence plus exact-message, wrong-remedy, and redaction
  plants.
  Add exact redacted provider, sanitizer, sink-limit, exporter, publication,
  retention, and no-verdict qualification rows, each naming its stable
  repository-relative input, corrective action, and exact literal rerun
  command. Cover every provider reason in every closed slice, every seccomp
  binding/preflight/`no_new_privs`/filter/exec stage in every slice, every
  qualification query/refusal/publication row, and every release
  query/refusal row. Reject `$!`, descriptor numbers, absolute, runfiles,
  socket, and Nix store paths, errno/OS text, raw child/tool/API output, raw
  cursors, opaque handles, argv/environment values, dynamic identifiers,
  generic owning-slice placeholders, query-as-absence mappings, and free-form
  commands. Add the complete runner-owned Rust-parent, C-helper, and
  child-setup code tables from `recovery-deadline.md`, crossed with all four
  slices and both closed command versions. Assert fixed repository-relative
  input, exact correction, phase-valid literal rerun, nonzero status, empty
  stdout, and byte-exact stderr for every row. Resolve every governed fixed
  artifact locator from the repository root: require a regular governed file,
  and for the full
  `specs/003-adr052-bazel-rust/contracts/runner-environment.md#...` locators
  require exactly one normalized heading anchor. Add isolated missing-input,
  missing-correction, missing-rerun, wrong-version, absent-in-phase command,
  wrong-slice, wrong-code, borrowed parent/helper/child remedy, unresolved
  path, and missing/duplicate-anchor plants. Reject numeric PID/PGID and raw
  protocol bytes in addition to the existing redaction set. Assert no
  `NIX_PTRACE_*`, `TOOLCHAIN_PTRACE_*`, or `SANDBOX_*` row or renderer exists
  in runner recovery; sequential T120 owns those pre-helper mappings and live
  exact tests. Include exact rows for
  `PARENT_SIGNAL_HANDOFF`, `HELPER_SIGNAL_INHERITED_IGNORED`,
  `HELPER_SIGNAL_HANDOFF`, `HELPER_GROUP_ESRCH`, `HELPER_GROUP_EPERM`,
  `HELPER_GROUP_ERROR`, `HELPER_GROUP_EARLY_EXIT`,
  `HELPER_PTRACE_STOP`, `HELPER_PTRACE_OPTIONS`,
  `HELPER_PTRACE_CONT`, `HELPER_PRE_EXEC_TERMINATION`,
  `HELPER_PRE_EXEC_DEATH`, `HELPER_PTRACE_EVENT`,
  `HELPER_PTRACE_DETACH`, `CHILD_PTRACE`, and `CHILD_STOP`; their remedies
  preserve safe Rust mask handoff, fail-before-fork ignored-disposition
  refusal, group plus tracing confirmation before `READY`, exact exec-event
  proof and zero-signal detach before `EXECUTED`, and no capability, broad
  ptrace, unsafe Rust, numeric signaling, or no-network weakening.
  Load the not-yet-wired recovery module through a test-local path.
- [ ] T068 [owner: spec003w2-recovery] [files:
  packages/d2b-bazel-runner/src/recovery.rs] [depends: T067] Implement the
  closed parent/helper/child recovery mapping, safe-input enum, correction
  enum, and versioned phase-valid slice-command enum, and satisfy T067. It
  rejects every `NIX_PTRACE_*`, `TOOLCHAIN_PTRACE_*`, and `SANDBOX_*` mapping
  because Nix evaluation, toolchain startup, and the patched sandbox own those
  pre-helper lifetimes. No free-form remedy or command and no numeric signal
  instruction is accepted. The ignored-disposition code never offers
  reset-and-continue, and the group codes retain distinct `ESRCH`, `EPERM`,
  other-error, and early-exit corrections. Helper ptrace codes retain distinct
  initial-stop, options, continue, pre-exec-death, wrong-event, and detach
  corrections.
- [ ] T069 [owner: spec003w2-evidence] [files:
  packages/xtask/tests/bazel_evidence.rs] [depends: T060] Add failing
  cold-local preparation and evidence validation tests, loading the
  not-yet-wired implementation through a test-local path.
- [ ] T070 [owner: spec003w2-evidence] [files:
  packages/xtask/src/bazel_evidence.rs] [depends: T069] Implement only the
  temporary cold-local evidence helper.
- [ ] T071 [owner: integrator] [files:
  packages/d2b-bazel-runner/src/lib.rs,
  packages/xtask/src/main.rs,
  bazel/generated/BUILD.bazel,
  bazel/generated/output-manifest.json,
  bazel/generated/package-policy-targets.bzl,
  bazel/generated/product-targets.bzl,
  changelog.d/adr052-bazel-safety.md]
  [depends: T062,T064,T066,T068,T070] Merge scope results, wire the completed
  spec003w2 runner and evidence modules into their prep-owned roots, regenerate
  integrator-owned BUILD output once, add the semantic fragment, and commit
  the candidate.
- [ ] T072 [owner: integrator] [files: none] [depends: T071] Run complete
  spec003w2 validation and every required cleanup, provider, process, recovery,
  startup, trim, and no-shell mutation.
- [ ] T073 [owner: integrator] [files: none] [depends: T072] Run the
  integrated-diff panel, seal `spec003w2`, merge, collect garbage, and remove
  finished worktrees.

## spec003w3 cache-free shadow

- [ ] T074 [owner: integrator] [files: none] [depends: T073] Run the
  spec003w3 plan panel over workflow, cache, record, and cold-evidence
  contracts.
- [ ] T075 [owner: spec003w3-shadow-workflow] [files:
  packages/xtask/tests/bazel_qualification.rs] [depends: T074] Add failing
  record tests proving pull-request runs emit no qualification record and
  execute zero cache actions, while protected-`v3` push records carry
  event/branch provenance, same-head pairing, explicit zero
  `bazelRestoreCount`, `bazelSaveCount`, and `bazelPublicationCount`, four
  `sliceDurationsSeconds` entries in every cold record, fixture verdict, and
  streak reset rules; a protected push with either or both workflows missing a
  verdict emits the structurally valid degraded tagged variant preserving
  available `testVerdict` values and resets the streak; complete/degraded
  opposite fields, repeated/mismatched sink-kind or retention fields, and
  unknown fields/codes/commands refuse without changing manifest v1; a missing
  count or duration is a refusal, never an implied zero. Add fixed-code
  repository-relative digest-only diagnostic and exact command tests for every
  qualification query, reference, consistency, candidate, derived-threshold,
  degraded-evidence, and publication failure. Bind each threshold class to its
  one exact correction and reject runtime paths, descriptors, OS
  text, raw output, and dynamic identifiers. Add
  failing tests for the typed validator:
  every threshold is
  derived from complete paginated Cargo, Bazel, and fixture run inventories
  plus immutable content references. Run references bind run ID, positive
  attempt, and head SHA; the validator refuses page gaps, missing attempts,
  omitted intervening protected-`v3` pushes, and duplicate/conflicting run
  identities, normalizes each run ID to its highest terminal attempt, derives
  Cargo/Bazel/fixture pairing and resets by head SHA, and selects the five
  newest qualifying cold records from the complete stream. Content references
  bind commit SHA or path plus digest;
  omitted, forged or ill-formed, duplicate, inconsistent, and wrong-candidate
  references each refuse at their own diagnostic; a record whose boolean or
  summary mirror disagrees with the derived verdict refuses; and no record
  qualifies through a boolean field. Add the closed seven-stage
  PID-namespace containment result census with bounded supervisor recovery,
  userspace escalation, cleanup, and quarantine enums; exact sandbox patch,
  canonical monitor identity, pending-observation, and result SHA-256 values;
  and no raw PID, process-group ID, descriptor, path, process output, kernel
  text, command line, environment, handle, or opaque identity. Add and require
  passing omitted/duplicate/unknown-stage, wrong-recovery-class,
  malformed-digest, patch/monitor-mismatch, illegal-cleanup/quarantine,
  false-reaped, success-after-quarantine, quarantined-reuse, and
  forbidden-field validator mutations. Add a closed exec-event qualification
  input binding both native startup and host-conformance results, exact
  source/protocol/seccomp identities, Linux minimum, supported systems, Yama
  assumption, static four-request plus enforceable constant-argument ptrace
  allowance, supervisor-owned dynamic child identity, all four exact libc
  argument values and pointer types, unchanged no-network result, exact
  event/detach positive, fast-exit positive, every call-position/type,
  wrong-pid/nonchild host refusal, and death/fault/EOF/wrong-event/detach
  negative and mutation, every distinct
  pre-helper Nix/toolchain/sandbox code plus wrong-remedy result, and every
  parent/helper/child recovery-code byte result. Missing, duplicate,
  wrong-system, wrong-policy, swapped/omitted/nonzero ptrace argument,
  borrowed remedy, incomplete-matrix, or no-network-regression inputs refuse.
- [ ] T076 [owner: spec003w3-shadow-workflow] [files:
  .github/workflows/pr-bazel-rust.yml,
  packages/xtask/src/bazel_qualification.rs] [depends: T075] Implement the
  credentialless four-slice shadow workflow, qualification capture, and the
  typed qualification validator behind
  `cargo xtask bazel-qualification-validate`, which takes no arguments, reads
  the fixed repository-relative record path, plus the no-argument atomic
  correction command `cargo xtask bazel-evidence refresh-qualification`.
  Both stay unreachable from Make and every workflow; a query failure leaves
  the prior record untouched and is never interpreted as an empty inventory.
  Qualification requires all seven containment results, the complete
  exec-event qualification input, and every named validator mutation result;
  no summary count or trusted boolean substitutes.
- [ ] T077 [owner: spec003w3-workflow-policy] [files:
  packages/xtask/tests/policy_ci.rs,
  packages/xtask/tests/fixtures/ci/cache-save-pr.yml,
  packages/xtask/tests/fixtures/ci/cache-post-step-pr.yml,
  packages/xtask/tests/fixtures/ci/unknown-cache-writer-pr.yml,
  packages/xtask/tests/fixtures/ci/actions-write-job-pr.yml,
  packages/xtask/tests/fixtures/ci/actions-write-workflow-pr.yml,
  packages/xtask/tests/fixtures/ci/shadow-valid.yml,
  packages/xtask/tests/fixtures/ci/qualification-wrong-event.yml,
  packages/xtask/tests/fixtures/ci/qualification-missing-count.yml] [depends: T074]
  Add the failing and passing workflow fixtures.
- [ ] T078 [owner: spec003w3-workflow-policy] [files:
  packages/xtask/tests/policy_ci.rs] [depends: T077] Extend the fail-closed
  reachability, permission, cache-count, and event policy until every fixture
  has its required verdict.
- [ ] T079 [owner: integrator] [files: packages/xtask/src/main.rs,
  changelog.d/adr052-bazel-shadow.md] [depends: T076,T078] Integrate only the
  qualification routing and workflow allowlist seam, add a semantic fragment,
  commit the exact candidate, and run local spec003w3 validation.
- [ ] T080 [owner: integrator] [files: none] [depends: T079] Inspect one
  stable-head draft pull-request run for four slices, one rollup, read-only
  permissions, zero cache actions, and no qualification record.
- [ ] T081 [owner: integrator] [files: none] [depends: T080] Record one
  protected-`v3` feasibility result under `.scratch/spec003w3-cold-ci/` with
  all four complete durations and explicit zero restore, save, and publication
  counts; run the injected `bazel_qualification` fixture suite only, not the
  no-argument fixed-path validator whose record does not exist until
  spec003w4; select only a larger runner or further disjoint split if the
  ceiling fails.
- [ ] T082 [owner: integrator] [files: none] [depends: T081] Run the
  integrated-diff panel, seal `spec003w3`, merge, collect garbage, and remove
  finished worktrees.

## spec003w4 immutable qualification

- [ ] T083 [owner: integrator] [files: none] [depends: T082] Run the
  spec003w4 plan panel over the complete evidence contract.
- [ ] T084 [owner: spec003w4-curator] [files:
  specs/003-adr052-bazel-rust/evidence/qualification.json] [depends: T083]
  Create the one curator worktree and initialize only the qualification
  record.
- [ ] T085 [owner: spec003w4-curator] [files:
  specs/003-adr052-bazel-rust/evidence/qualification.json] [depends: T084]
  Add ten consecutive protected-`v3` records with explicit
  `bazelRestoreCount`, `bazelSaveCount`, and `bazelPublicationCount` and
  reset arithmetic.
- [ ] T086 [owner: spec003w4-curator] [files:
  specs/003-adr052-bazel-rust/evidence/qualification.json] [depends: T085]
  Add the eighteen isolated failure results, exact censuses, locator proof,
  prior-evidence invalidation, multi-carrier attribution, sorted atomic
  manifest v1 success/failure/interruption evidence, original-status
  preservation, ignored-case fidelity, complete forbidden-value absence from
  JUnit, bounded `test.log`, emitted evidence, and exporter diagnostics, typed
  degraded-evidence rejection, no-shell proof, combined-budget mutations,
  per-case publication, all containment-validator mutation results,
  and five topology proofs. The no-shell proof references the committed
  `bazel/generated/no-shell-inventory.json` path and digest, its nonempty
  governed/declared equality result, governed-spawn result, complete
  per-source scan records including zero-site sources, the fresh-scan/
  committed spawn-site-key equality result, raw and unique scan-record counts
  each equal to governed-source count, and the
  `no-shell-inventory-empty`,
  `no-shell-inventory-missing-entry`, `no-shell-inventory-extra-entry`,
  `no-shell-inventory-unguarded-spawn`,
  `no-shell-inventory-missing-zero-site-record`, and
  `no-shell-inventory-planted-shell` plant results.
- [ ] T087 [owner: spec003w4-curator] [files:
  specs/003-adr052-bazel-rust/evidence/qualification.json] [depends: T086]
  Add twenty consecutive executions for each broker context with exclusive
  tags, no overlap, and the tag-removal mutation.
- [ ] T088 [owner: spec003w4-curator] [files:
  specs/003-adr052-bazel-rust/evidence/qualification.json] [depends: T087]
  Add three valid warm, three valid cold-local, and the five newest cold
  records, each cold record carrying `bazelRestoreCount` of zero and four
  `sliceDurationsSeconds` entries.
- [ ] T089 [owner: spec003w4-curator] [files:
  specs/003-adr052-bazel-rust/evidence/qualification.json] [depends: T088]
  Add four package-policy results, one product-only yanked snapshot with exact
  main/broker/guest semantics, Cargo/decomposed-Bazel status and normalized
  finding equality for all three contexts, both Nix hash proofs,
  module-refresh evidence, exactly four artifact-baseline row digests, all four
  dedicated artifact realizations, exact broker linkage, transient closure
  validation with persisted counts/digests only, all four sizes, every
  size-authorization positive/negative, all artifact mutations, and every
  policy refusal.
- [ ] T090 [owner: spec003w4-curator] [files:
  specs/003-adr052-bazel-rust/evidence/qualification.json] [depends: T089]
  Add native x86 and arm six-realization evidence, including arm supply-chain
  plus renderer on the same stable head, expected native `e_machine`, `ET_DYN`,
  exact broker interpreter/SONAMEs, no guest interpreter or `DT_NEEDED`,
  non-PIE/wrong-machine plants, exact patched-Bazel source/patch/policy/output/
  executable/capability hashes, startup probe, configured-target plus `aquery`
  stable/nightly action-kind inventory, sandbox strategy inventory,
  patch-removal/wrong-output/filter-load/inherited-capability/stage/fallback
  gaps, setup-before-payload and all eight pre-action socket/io_uring plants
  plus descendant/external-egress/live-index,
  exactly one bounded containment result for crash before `READY`, after
  `READY`, after `EXECUTED`, during grace, direct and double-forked
  long-lived descendants, and beyond-ceiling pending cleanup. Bind each closed
  supervisor recovery class, userspace escalation, cleanup, and quarantine
  result; matching sandbox patch and canonical monitor identity digests; the
  pending-observation and per-result digests; consuming reap evidence;
  no-success/no-reuse behavior; and absence of raw process identifiers,
  descriptors, paths, output, or opaque identities,
  exact
  same-commit non-advisory Cargo compatibility-carrier results, and the
  fetch inventory. Add non-advisory manifest evidence and advisory mutations
  for `test-flake-aarch64`, all four Rust slices, and `test-rust`.
- [ ] T091 [owner: spec003w4-curator] [files:
  specs/003-adr052-bazel-rust/evidence/qualification.json] [depends: T090]
  Run `cargo xtask bazel-qualification-validate` so every threshold is
  derived from the record's immutable evidence references, require its success
  with no pending item and no boolean mirror disagreeing with the derived
  verdict, require all seven containment results and every named validator
  mutation result, require all four sink retention classes and unchanged
  manifest-v1 compatibility, commit the immutable record, and validate both Rust
  aggregates, policy, drift, and fixture companions.
- [ ] T092 [owner: integrator] [files: none] [depends: T091] Run the
  integrated-diff panel, seal `spec003w4`, merge, verify the merged digest,
  collect garbage, and remove measurement worktrees.

## spec003w5 promotion and promotion record

- [ ] T093 [owner: integrator] [files: none] [depends: T092] Run the
  spec003w5 plan panel against the immutable qualification digest and cache
  contract.
- [ ] T094 [owner: integrator] [files:
  packages/xtask/src/bazel_cache_contract.rs,
  packages/xtask/src/promotion_contract.rs,
  packages/xtask/Cargo.toml, packages/xtask/src/main.rs,
  packages/Cargo.lock, bazel/cargo/product.lock,
  MODULE.bazel.lock] [depends: T093] Land complete green cache, promotion, and
  typed run-unit interfaces and a green xtask contract seam that does not
  declare not-yet-present cache or run-unit implementation modules. If a
  product manifest changes, regenerate `packages/Cargo.lock`, product repin,
  and module refresh in order, commit each output, prove clean no-op reruns,
  and prove both walker inputs byte-identical. If a walker manifest or lock
  changes, regenerate the walker Cargo lock, walker repin, and module refresh
  in order and prove both product inputs byte-identical.
  `MODULE.bazel.lock` is always committed last. No parallel scope edits these
  files.
- [ ] T095 [owner: spec003w5-interface-tests] [files:
  packages/d2b-bazel-runner/tests/make_interface.rs] [depends: T094] Add
  failing authoritative `test-rust-slice-{main,api,broker,aux}`, exact
  eight-public-leaf subset, `test-rust-main` conditional fixture, exact alias
  stderr, alias-status, fixture-mode, and promoted-deadline tests.
- [ ] T096 [owner: spec003w5-promotion-make] [files: Makefile,
  tests/test-rust.sh] [depends: T095] Switch the eighteen surfaces to Bazel,
  introduce the four authoritative slice targets, preserve fixture mode and
  all eight public names with exact subsets, and add exact-message
  status-preserving aggregate/slice aliases.
- [ ] T097 [owner: spec003w5-promotion-manifest] [files:
  tests/unit/meta/ci-runner-regression.py] [depends: T094] Add failing renderer
  tests proving the four generated jobs call only
  `test-rust-slice-{main,api,broker,aux}`, keep stable
  `ciJobId: test-rust`, preserve public leaf names, and set the promoted
  deadline. Require each slice and the `test-rust` rollup to be non-advisory
  and add an advisory-classification mutation for each class.
- [ ] T098 [owner: spec003w5-cache] [files:
  packages/xtask/tests/bazel_cache.rs,
  packages/xtask/tests/post_promotion_observations.rs,
  packages/xtask/tests/promotion_record.rs,
  packages/xtask/tests/release_containment.rs,
  packages/xtask/tests/policy_ci.rs,
  packages/xtask/tests/fixtures/ci/promoted-cache-valid.yml,
  packages/xtask/tests/fixtures/ci/promoted-cache-prefix-run-id.yml,
  packages/xtask/tests/fixtures/ci/promoted-cache-prefix-sha.yml,
  packages/xtask/tests/fixtures/ci/promoted-cache-delete-newest.yml] [depends: T094]
  Add failing pagination, authorization, run-unique key,
  run/SHA-free prefix, newest-generation retention, trim, headroom, one-writer,
  credential, cross-architecture, every-bound-key-input applicability, and
  action/repository namespace-separation fixtures. Each applicable input
  mutation must change both its primary key and its run/SHA-free restore
  prefix. Define deletion authority as the closed typed committed prefix enum;
  add three-page mixed authorized/unauthorized fixtures, preserve unknown
  entries, and require zero delete calls on page-gap, caller-prefix, unknown,
  or ambiguous-prefix refusal. Add run-unit fixtures for
  complete `1..max` attempt history, a repeated-attempt unit that must
  contribute exactly one streak position, an old-rerun-after-failure unit that
  must stay before the newer failure in `(createdAt, runId)` order so the
  failure still resets the streak, a
  missing-attempt rejection, and a conflicting head/provenance rejection.
  Add persisted-evidence fixtures proving the full protected stream is fetched
  transiently while only closed complete state, page/stream counts, a fixed
  digest, and final ten normalized units with attempt-history digests are
  atomically persisted within record and byte bounds. Reject persisted raw
  cursors. Add promotion-record fixtures for actual sealed merge, old SHA,
  candidate SHA, wrong seal, unsealed merge, and wrong PR merge SHA. Add
  fixed-code exact-remedy release-containment fixtures for promotion-record,
  local-tag, origin-tag, and release-metadata query failures plus no semantic
  tag, unpushed tag, divergent tag, absent release, draft, and prerelease.
  Prove query failures are typed degradation rather than absence and no
  diagnostic or persisted result contains a candidate/tag/object identifier or
  raw `git`/`gh` output.
  Load the not-yet-wired cache and run-unit modules through test-local
  paths.
- [ ] T099 [owner: spec003w5-cache] [files:
  packages/xtask/src/bazel_cache.rs,
  packages/xtask/src/post_promotion_observations.rs,
  packages/xtask/src/promotion_record.rs,
  packages/xtask/src/release_containment.rs,
  packages/xtask/tests/policy_ci.rs] [depends: T098] Implement ordered
  protected-`v3` maintenance, separate action/repository cache publication,
  closed-prefix authorization, typed sealed-merge promotion validation,
  no-argument closed-diagnostic release-containment validation, and
  typed complete transient post-promotion run-unit validation and derivation,
  with bounded fixed-shape persistence, where a
  unit is one distinct push-created (run ID, head SHA) pair with complete
  `1..max` attempt history ordered by `(createdAt, runId)`, and the
  workflow policy that makes every T098 fixture enforce. Release query
  failures are closed degraded outcomes and semantic ineligibility is a closed
  refusal; neither suppresses a failed backend.
- [ ] T100 [owner: spec003w5-promotion-manifest] [files:
  tests/layer1-jobs.json, tests/tools/layer1-jobs.py,
  tests/ci/layer1-workflow.template.yml] [depends: T097] Replace eight Rust
  jobs with the four authoritative slice targets while retaining
  `ciJobId: test-rust`, require all four slices and the rollup to remain
  non-advisory, and make every renderer test pass.
- [ ] T119 [owner: spec003w5-hybrid-policy] [files:
  packages/d2b-contract-tests/tests/policy_bazel_hybrid_docs.rs,
  tests/lib.sh] [depends: T100] Add an enforcing fixture-independent type-5
  policy lint under `make test-policy`. Derive the exact sorted nonempty
  `cargoCompatibilityCarriers` census from
  `tests/golden/bazel-rust-coverage.json`, retaining each entry's surface ID,
  Cargo selector, test identity, and socket class even when several entries
  share a surface; compare the complete canonical entries bidirectionally with
  the semantic retained-compatibility block in every fixed governed hybrid doc
  and every present promotion/alias-removal/Cargo-retirement semantic
  fragment. Add isolated fail-closed fixtures for empty source census, missing,
  extra, malformed block, duplicate block, malformed identity, duplicate
  identity, stale attribution, and governed-document mismatch; every
  comparison remains over the complete four-field identity. Land these
  fixtures before repository disclosure is updated, wire the binary into the shared
  fixture-independent list, and prove `test-policy` runs it while fixture
  contracts exclude it.
- [ ] T101 [owner: spec003w5-binding-docs] [files: AGENTS.md,
  tests/AGENTS.md, docs/contributing/gates-and-lints.md, tests/README.md,
  docs/reference/test-execution-manifest.md] [depends: T096,T099,T100,T119]
  Update binding docs from eight Cargo leaves to four Bazel
  slices in the same promotion change, including `tests/README.md` and
  `docs/reference/test-execution-manifest.md` because both describe the eight
  CI jobs, document every exact alias replacement and retained public leaf,
  list every exact permanently hybrid surface and socket-using Cargo case,
  state that separate authorization is required before their retirement, and
  use no process markers. Make each semantic retained-compatibility block
  exactly equal the type-5 policy's nonempty source census.
- [ ] T102 [owner: integrator] [files:
  packages/xtask/src/main.rs,
  .github/workflows/pr-l1-static-fast.yml,
  .github/workflows/pr-bazel-rust.yml,
  changelog.d/adr052-bazel-promotion.md] [depends: T096,T099,T100,T101]
  Record the spec003w5 parent, integrate or squash every spec003w5 scope result into one
  atomic promotion candidate relative to that parent, wire the completed cache
  and run-unit modules into xtask, generate the required workflow, delete
  the shadow workflow, remove only the temporary cold-local helper routing,
  add every exact replacement plus the exact permanently hybrid surface and
  retained Cargo socket-case inventory and separate authorization requirement
  to the semantic fragment, make that fragment pass the same type-5 exact
  census comparison, assert the candidate's complete changed-path diff, and
  commit its one SHA.
- [ ] T103 [owner: integrator] [files: none] [depends: T102] Run
  `cargo xtask bazel-qualification-validate` against the sealed spec003w4
  record at this candidate and require success, then run
  `make layer1-workflow`, `make test-drift`, `make check`, fixture contracts,
  `make test-policy`, qualification digest, all three supply-chain equivalence
  results, aliases,
  deadline, cache policy, every bound-key mutation, and exact clean-diff
  validation. Resolve the rehearsal candidate from the verified current atomic
  candidate HEAD and the recorded spec003w5 parent, not from
  `promotion-record.json`, which does not exist before merge; verify the
  candidate parent and complete path diff, revert the exact atomic candidate
  without committing in a disposable worktree, and prove Cargo-authoritative
  Rust plus fixture contracts pass.
- [ ] T104 [owner: integrator] [files: none] [depends: T103] Run the
  integrated-diff panel, seal `spec003w5`, merge, and observe ordered
  maintenance, publication, and the first promoted verdict.
- [ ] T105 [owner: integrator] [files:
  specs/003-adr052-bazel-rust/evidence/promotion-record.json,
  specs/003-adr052-bazel-rust/evidence/post-promotion.json] [depends: T104]
  Record immutable promotion and initialize only the typed paginated
  post-promotion checkpoint and independent clocks. Run
  `cargo xtask bazel-promotion-record-validate` and require the recorded
  promotion SHA to equal the actual protected-`v3` PR merge and the exact
  sealed `spec003w5` identities. Derive from the complete transient run stream,
  but persist only complete pagination state, page/stream counts, the fixed
  checkpoint digest, and final ten normalized units with attempt-history
  digests within schema byte/record bounds; do not write a raw cursor, trust
  eligible/count/run-ID summaries, or append complete attempt history.
  This is the first task that may read `promotion-record.json`. Run the
  follow-up panel, seal
  `spec003w5fu1`, merge, and collect garbage.

## Independent post-promotion children

### spec003w6 compatibility alias removal

- [ ] T106 [owner: spec003w6-alias-removal] [files: none] [depends: T105]
  First require `cargo xtask bazel-promotion-record-validate` to bind the
  promotion commit to the actual sealed merge. Then run the no-argument
  `cargo xtask bazel-release-containment-validate` to prove a published semantic
  release tag contains that commit by transiently filtering local tag
  references through `^v[0-9]+\.[0-9]+\.[0-9]+$`, proving ancestry, comparing
  peeled origin objects, and requiring present non-draft/non-prerelease release
  metadata. Local enumeration, origin resolution, and metadata query errors
  produce distinct typed degraded codes and never become absence; candidate,
  tag, and object identifiers remain transient and unprinted. A two-component
  tag such as `v1.0`, an unpushed tag, a divergent same-named local and remote
  tag, a draft release, and a prerelease each fail entry. Then run the
  spec003w6 plan panel without consulting the green-run count.
- [ ] T107 [owner: spec003w6-alias-removal] [files:
  packages/d2b-bazel-runner/tests/make_interface.rs,
  packages/d2b-bazel-runner/tests/diagnostic.rs,
  packages/xtask/tests/bazel_action_network.rs,
  packages/xtask/tests/policy_ci.rs] [depends: T106] Update the interface test
  before implementation and observe it fail, then add
  published-semantic-release-containment, removed-alias, workflow-use, and
  public-name assertions, including two-component-tag, unpushed-tag,
  divergent-local/remote-tag, draft-release, prerelease, and wrong-promotion-
  SHA negatives. Bind each release refusal to its fixed code and exact rendered
  remedy; add independent promotion-record/local/origin/metadata query-failure
  fixtures and prove none maps to absence. Reject `$!`, descriptors,
  candidate/tag/object/commit/run identifiers, absolute/store/socket paths, OS
  text, raw cursors/handles/output, free-form commands, and borrowed remedies.
- [ ] T108 [owner: spec003w6-alias-removal] [files: Makefile,
  packages/d2b-bazel-exec/src/provider.rs,
  packages/d2b-bazel-exec/src/execute.rs,
  packages/d2b-bazel-exec/tests/execute.rs,
  packages/d2b-bazel-runner/src/lib.rs,
  packages/d2b-bazel-runner/src/coverage.rs,
  packages/d2b-bazel-runner/src/diagnostic.rs,
  packages/d2b-bazel-runner/src/junit.rs,
  packages/d2b-bazel-runner/src/manifest.rs,
  packages/d2b-bazel-runner/src/recovery.rs,
  packages/d2b-bazel-runner/tests/diagnostic.rs,
  packages/d2b-bazel-runner/tests/provider_execution.rs,
  packages/d2b-bazel-runner/tests/recovery.rs,
  packages/d2b-bazel-runner/tests/result_publication.rs,
  packages/xtask/src/main.rs,
  packages/xtask/src/bazel_evidence.rs,
  packages/xtask/src/bazel_qualification.rs,
  packages/xtask/src/hermeticity.rs,
  packages/xtask/tests/bazel_action_network.rs,
  packages/xtask/tests/bazel_evidence.rs,
  packages/xtask/tests/bazel_qualification.rs,
  packages/xtask/tests/policy_ci.rs,
  AGENTS.md,
  tests/AGENTS.md,
  tests/README.md,
  docs/contributing/gates-and-lints.md,
  docs/reference/test-execution-manifest.md,
  changelog.d/adr052-bazel-alias-removal.md,
  specs/003-adr052-bazel-rust/evidence/post-promotion.json] [depends: T107]
  Remove only Bazel-specific aliases and their approved-target entries. In the
  same atomic change, replace diagnostic command version 1's shadow aggregate/
  slice targets with version 2's enduring `test-rust` and
  `test-rust-slice-{main,api,broker,aux}` targets in every production provider,
  sandbox-policy, qualification threshold/table, evidence/publication,
  cleanup, and recovery renderer; both module-wiring roots; every byte-exact
  unit/integration/policy test; the qualification evidence fields; all five
  governed semantic docs; and the semantic alias-removal fragment. Repeat the
  exact permanently hybrid surface and retained Cargo socket-case inventory
  and separate authorization requirement. Add a closed-census policy assertion
  that no diagnostic, qualification threshold, evidence variant, doc, fragment,
  task-state label, or renderer names a removed or nonexistent target. The
  pre-change test fixture may select version 1 only while all shadow rules
  exist; the changed candidate selects version 2 only after all five enduring
  aggregate/slice rules exist. Change no refusal class or recovery operation.
- [ ] T109 [owner: spec003w6-alias-removal] [files: none] [depends: T108]
  Audit the atomic T108 census: require every production renderer, both module
  roots, threshold row, evidence/publication path, exact-message test,
  governed doc, semantic fragment, and alias evidence record to select version
  2; require version 1 to remain only in its pre-change fixture; and run the
  enforcing type-5 exact nonempty hybrid disclosure comparison. Refuse any
  missing, extra, duplicate, mixed-version, removed-target, or nonexistent-
  target member.
- [ ] T110 [owner: spec003w6-alias-removal] [files: none] [depends: T109]
  Validate every public Rust leaf, policy, tier0, drift, and fixture target;
  run the integrated-diff panel; seal `spec003w6`; then merge and collect
  garbage.

### spec003w7 Cargo implementation retirement

- [ ] T111 [owner: spec003w7-cargo-retirement] [files: none] [depends: T105]
  First require `cargo xtask bazel-promotion-record-validate`. Then paginate
  and validate every promoted protected-`v3` Rust run unit transiently, where a
  unit is one distinct push-created (run ID, head SHA) pair whose attempts
  `1..max` are complete nested history and whose conclusion normalizes to the
  highest terminal attempt, order units by `(createdAt, runId)` and never by
  rerun start time, derive reset positions and the current streak counting each
  unit exactly once, prove the final ten distinct ordered units are successes
  with no intervening failure or cancellation, and run the
  spec003w7 plan panel without consulting release containment or self-asserted
  eligible/count/ID fields. Require the persisted checkpoint and final-ten
  suffix to remain within schema byte/record bounds. This task may complete
  before T106 through T110.
- [ ] T112 [owner: spec003w7-cargo-retirement] [files:
  packages/xtask/tests/policy_workspace.rs,
  packages/xtask/tests/post_promotion_observations.rs] [depends: T111] Add
  failing pagination-gap, missing-attempt, missing/duplicate unit identity,
  conflicting attempt provenance, repeated-attempt (one unit with several
  successful attempts contributes exactly one streak position),
  old-rerun-after-failure (a unit created before a later failing unit and rerun
  successfully afterwards still orders before it and leaves the reset in
  place), non-v3, non-push, pre-promotion, nonterminal, failure/cancellation
  reset, self-asserted-summary, fixture deletion, non-migrated deletion,
  public-name deletion, and unreachable-source inventory tests.
- [ ] T113 [owner: spec003w7-cargo-retirement] [files:
  tests/test-rust.sh,
  packages/xtask/src/post_promotion_observations.rs] [depends: T112] Make any
  run-unit validator corrections exposed by T112, then remove only Cargo
  implementations for the eighteen surfaces and unreachable Cargo-only
  plumbing.
- [ ] T114 [owner: spec003w7-cargo-retirement] [files: AGENTS.md,
  tests/AGENTS.md, docs/contributing/gates-and-lints.md,
  changelog.d/adr052-cargo-retirement.md,
  specs/003-adr052-bazel-rust/evidence/post-promotion.json] [depends: T110,T113]
  Update semantic docs, fragment, and typed run-unit evidence while
  preserving every public Make name, fixture mode, and mandatory socket-test
  Cargo compatibility carrier. List the exact permanently hybrid surfaces and
  separate authorization requirement; persist only complete pagination state,
  page/stream counts, the bounded digest checkpoint and final-ten suffix, add
  no raw cursor, and do not add trusted eligible/count/run-ID fields. Require
  the enforcing type-5 policy's exact nonempty census comparison to pass for
  every governed doc and the Cargo-retirement fragment.
- [ ] T115 [owner: spec003w7-cargo-retirement] [files: none] [depends: T114]
  Run `make check`, Rust, four slices, policy, drift, fixture contracts, and
  the retirement inventory; run the integrated-diff panel; seal `spec003w7`;
  then merge and collect garbage. Rebase onto merged spec003w6 before
  validation and obtain a fresh panel verdict.

## Final analysis

- [ ] T116 [owner: integrator] [files: none] [depends: T110,T115] Map every
  FR-001 through FR-090 and SC-001 through SC-043 to a completed task and
  immutable mechanical evidence.
- [ ] T117 [owner: integrator] [files: none] [depends: T116] Scan all Spec 003
  and implementation artifacts for stale workspace, hub, nested-lock,
  yanked-authority, provider-`RESOLVE_BENEATH`, provider fallback,
  namespace-as-socket-enforcement, action-wrapper setup claim, missing or
  wrong patched-Bazel identity/capability, inherited ring/SQPOLL/fixed-socket
  omission, process/local/standalone/worker/remote or stage fallback, socket
  plant in the stub carrier, runfiles/worktree exec helper, direct helper
  invocation outside the typed consumer, fd-0 executable transport,
  first-party unsafe exception, missing typed resource/transport stage, open
  VerifiedExecutable API/trait
  surface, old
  slice target, generated-slice ownership,
  self-asserted post-promotion eligibility, attempt-counted streak,
  rerun-start-time ordering, trusted qualification boolean, two-view context
  oracle, single lock-refresh order, snake_case cache-count, nested-lock gate
  input, stale native-check/artifact-row cardinality, incomplete no-shell plant
  list, raw/unique scan-count omission, persisted store path/raw cursor,
  duplicate size allowance, repeated sink classification, contradictory
  evidence variants, unclosed evidence status, missing retention class,
  generic refusal remedy, release query suppressed as absence, undisclosed or
  policy-unchecked hybrid Cargo case, stale shadow-target diagnostic after
  alias removal, or unqualified wave assumptions; require
  every process reference to
  use exactly `spec003w0` through `spec003w7` plus `spec003w5fu1` and allow
  historical branch literals only where explicitly labelled parked evidence.
- [ ] T118 [owner: integrator] [files: none] [depends: T117] Verify every wave
  and follow-up is merged and sealed, every shared-file second child rebased,
  revalidated, and re-paneled, evidence contains no raw logs or credentials,
  shipped docs contain no process markers, the read-only plan-structure
  validator passes, and garbage collection completed.

## Dependency graph

The following adjacency list is exactly the `depends` clauses above and is
machine-checked before panel:

```text
T001 <- none
T002 <- T001
T003 <- T002
T004 <- T003
T005 <- T004
T006 <- T005
T007 <- T006
T120 <- T007
T008 <- T120
T009 <- T008
T010 <- T009
T011 <- T010
T012 <- T011
T013 <- T012
T014 <- T011
T015 <- T014
T016 <- T011
T017 <- T016
T018 <- T013
T019 <- T018
T020 <- T019
T021 <- T019
T022 <- T021
T023 <- T021,T022
T024 <- T009,T013,T015,T017,T020,T022,T023
T025 <- T009,T013,T015,T017,T020,T022,T023,T024
T026 <- T025
T027 <- T025
T028 <- T023,T025
T029 <- T026,T027,T028
T030 <- T029
T031 <- T030
T032 <- T031
T033 <- T032
T034 <- T033
T035 <- T034
T036 <- T035
T037 <- T036
T038 <- T035
T039 <- T038
T040 <- T035
T041 <- T040
T042 <- T035
T043 <- T042
T044 <- T035
T045 <- T044
T046 <- T035
T047 <- T046
T048 <- T035
T049 <- T048
T050 <- T035
T051 <- T050
T052 <- T035
T053 <- T052
T054 <- T035
T055 <- T054
T056 <- T037,T039,T041,T043,T045,T047,T049,T051,T053,T055
T057 <- T056
T058 <- T057
T059 <- T058
T060 <- T059
T061 <- T060
T062 <- T061
T063 <- T060
T064 <- T063
T065 <- T060
T066 <- T065
T067 <- T060
T068 <- T067
T069 <- T060
T070 <- T069
T071 <- T062,T064,T066,T068,T070
T072 <- T071
T073 <- T072
T074 <- T073
T075 <- T074
T076 <- T075
T077 <- T074
T078 <- T077
T079 <- T076,T078
T080 <- T079
T081 <- T080
T082 <- T081
T083 <- T082
T084 <- T083
T085 <- T084
T086 <- T085
T087 <- T086
T088 <- T087
T089 <- T088
T090 <- T089
T091 <- T090
T092 <- T091
T093 <- T092
T094 <- T093
T095 <- T094
T096 <- T095
T097 <- T094
T098 <- T094
T099 <- T098
T100 <- T097
T119 <- T100
T101 <- T096,T099,T100,T119
T102 <- T096,T099,T100,T101
T103 <- T102
T104 <- T103
T105 <- T104
T106 <- T105
T107 <- T106
T108 <- T107
T109 <- T108
T110 <- T109
T111 <- T105
T112 <- T111
T113 <- T112
T114 <- T110,T113
T115 <- T114
T116 <- T110,T115
T117 <- T116
T118 <- T117
```
