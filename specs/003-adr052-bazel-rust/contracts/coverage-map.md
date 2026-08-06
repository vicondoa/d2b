# Coverage Map Contract

`tests/golden/bazel-rust-coverage.json` binds the existing eighteen
execution-manifest IDs to Bazel carriers. It does not replace manifest v1.

## Exact ID set

```text
rust-api-surface
rust-main-format
rust-main-clippy
rust-main-workspace-tests
rust-no-bash-ast
rust-schema-reproducibility
rust-stub-no-socket
rust-assert-pinned
rust-broker-default
rust-broker-layer1
rust-broker-fakebackends
rust-guest-shell-runner
rust-deny-main
rust-deny-broker
rust-deny-guest
rust-audit-main
rust-audit-broker
rust-audit-guest
```

Fixture-backed IDs do not appear.

## Required row

Each row contains:

- one `surfaceId`;
- nonempty carriers with exactly one verdict owner;
- one of `main`, `api`, `broker`, `aux`;
- the current Cargo baseline using root product package selectors;
- exact generated census and out-of-census reasons;
- per-carrier topology;
- all carried Rust tests;
- every hand-written fragment;
- configured first-party target labels and direct dependency, cfg, and feature
  census;
- binary providers and declared runfiles-relative paths;
- locator migration dispositions;
- deliberate ADR 0052 differences;
- generated BUILD digest.
- `actionNetwork = "none"` for every Bazel action, plus the declared-input
  source for every tool, advisory database, yanked record, and vendored crate;
- `sandboxPolicy`, naming the exact Nix-patched Bazel output, capability ABI,
  fixed policy digest, patched Linux sandbox strategy, and load-before-action-
  exec result for every stable/nightly compile, build, setup, and test action;
- `cargoCompatibilityCarriers`, an exact sorted census of mandatory
  socket-using Rust tests that cannot run as Bazel actions under ADR 0052.
  Each entry binds its existing surface ID, Cargo selector, test identity,
  socket class, and same-commit verdict owner. A row with no such carrier uses
  an empty array;
- for each broker row, the literal target tag set `["exclusive"]`.

Rows and arrays are sorted. Required collections cannot be empty.

## Hub and native-target invariants

- Third-party product dependencies come only from `@product`.
- Walker dependencies come only from `@walker`.
- Every first-party product crate is a native Bazel target.
- Broker default, layer1, and fake contexts and guest real-libshpool each have
  an exact configured native target census.
- The product external package and feature union may exceed a configured
  context.
- Actual first-party dependencies and features are defined by configured
  native targets, not by the product hub union.
- No configured broker context reaches guest or an unrelated first-party
  sibling.
- No guest context reaches broker or an unrelated first-party sibling.

## Broker scheduling isolation

`rust-broker-default`, `rust-broker-layer1`, and
`rust-broker-fakebackends` each map to a Bazel suite carrying exactly
`tags = ["exclusive"]`. Bazel must schedule each after all nonexclusive tests,
so none may overlap another broker suite or any other test. A custom local
resource is not an equivalent mechanism.

The coverage guard rejects a missing or renamed tag. Its mutation removes
`exclusive` from one suite and must observe overlap with a planted ordinary
test. Qualification runs each broker context twenty consecutive times with
`--runs_per_test=20`, one context at a time, while an ordinary overlap probe is
present; every run must show the broker suite alone.

## Action network inventory

ADR 0052's action no-network rule remains absolute. Linux network namespaces
remain defense in depth for external reachability, but are not socket-creation
enforcement and are never cited as such.

The repository packages Bazel 8.6.0 only through
`pkgs/bazel-8.6.0-seccomp/default.nix`. The derivation applies exactly
`pkgs/bazel-8.6.0-seccomp/linux-sandbox-seccomp.patch` and installs the fixed
policy in the same immutable output. `tests/golden/bazel-toolchain.json`
records the exact upstream source, patch, policy, output NAR, Bazel executable,
and capability-ABI hashes. The gate invokes that exact output, not Bazelisk or
an ambient Bazel. Before starting the server, a startup probe verifies the ABI,
loads the filter in a sandbox child, and observes the fixed denial. Missing
capability, missing/wrong output, changed policy, patch removal, or failed
filter load refuses before analysis or execution.

The patch changes the Linux sandbox runner and child plumbing. The runner
passes only the fixed installed policy and compiled expected digest. After
namespace, mount, and other sandbox construction but before exec of the action
command, the sandbox child completes inherited-capability preflight, sets
`no_new_privs`, verifies and loads the fixed filter, and then execs the action
argv. This covers the compiler/build command and Bazel's `test-setup.sh` or
equivalent first test action command, the test, and every descendant. No
action wrapper or `--run_under` claim is used.

Generated configured-target queries, `aquery` snapshots, and a strategy
inventory cover stable/nightly `Rustc`, `RustcMetadata`, Clippy, rustdoc,
rustdoc-test compile/run, rustfmt, unpretty, `CargoBuildScript`, repository,
setup, and test actions. Every governed action must select the patched Linux
`sandboxed` strategy. `process`, `local`, `standalone`, `worker`, `remote`,
`no-sandbox`, a network-enabling tag, and every ordered or implicit fallback
are refusals. A process-wrapper sandbox is not equivalent.

Before filter load, the sandbox child performs a complete inherited descriptor
preflight. It rejects every socket descriptor and every io_uring ring
descriptor. Rejecting the ring itself covers ordinary rings, SQPOLL rings, and
rings carrying registered files or fixed sockets; no inherited ring state is
grandfathered. The filter returns the fixed `EACCES` sentinel for `socket`,
`socketpair`, `connect`, `bind`, `listen`, `accept`, `accept4`, `sendto`,
`sendmsg`, `sendmmsg`, `recvfrom`, `recvmsg`, `recvmmsg`, `shutdown`,
`getsockname`, `getpeername`, `setsockopt`, `getsockopt`, `pidfd_getfd`,
`io_uring_setup`, `io_uring_enter`, and `io_uring_register`, plus `socketcall`
where the native architecture exposes it. There is no identity, policy-open,
digest, preflight, `no_new_privs`, filter-load, or action-exec fallback.
For the immutable execution supervisor, the same static filter permits only
four ptrace request values with the constant arguments it can enforce:
`PTRACE_TRACEME` requires pid zero, address `(void *)0`, and data
`(void *)0`; `PTRACE_SETOPTIONS` requires address `(void *)0` and data
`(void *)(uintptr_t)PTRACE_O_TRACEEXEC`; `PTRACE_CONT` and `PTRACE_DETACH`
each require address and data `(void *)0`. The filter does not compare the
future child pid for `SETOPTIONS`, `CONT`, or `DETACH`; classic seccomp cannot
derive a fork result or direct-parent relation. Dynamic identity is enforced
by the supervisor-owned fork result, confirmed `getpgid(child) == child` and
direct-parent relation, traced initial stop, sole wait ownership, and exact
`PTRACE_EVENT_EXEC` for that child. Attach, seize, memory/register access,
syscall tracing, every other request, wrong constant arguments, options in the
address position, or nonzero `CONT`/`DETACH` data remain denied. This changes
none of the socket, io_uring, `pidfd_getfd`, or action no-network denials.
Startup and native host-conformance evidence bind exact request and pid values,
pointer positions and types, the dynamic child relation, and wrong-pid and
nonchild refusal; mutations reject missing, integer-in-pointer-position,
exchanged, or additional arguments without claiming static child-pid matching.

The same pinned Linux sandbox patch also binds crash containment. Every
governed action has a fresh `CLONE_NEWPID` namespace. Namespace PID 1 remains
outside the action command tree and owns abnormal teardown: namespace-local
SIGKILL and nonblocking adopted-child reap progress. One fixed 10,000 ms
monotonic ceiling bounds userspace TERM/KILL/monitor escalation and the
close-or-quarantine decision only, never kernel cleanup. A PID 1 not proved
reaped by a consuming wait enters outer-owned `pending-kernel-cleanup`;
sandbox and outputs cannot succeed or be reused until the original live
monitor's eventual consuming reap. That monitor remains sole wait owner and
publishes the only release; reboot, retry-before-release, replacement wait
ownership, and manual release are forbidden. The fixed pending diagnostic
links to
`docs/contributing/critical-subsystems.md#bazel-pending-kernel-cleanup-quarantine`.
The supervisor retains normal TERM/grace/KILL/reap. Rust never signals a
numeric PID or PGID.

A real patched-sandbox integration plants supervisor crash before `READY`,
after `READY`, after `EXECUTED`, during grace, and with direct and
double-forked long-lived descendants. Ordinary cases require liveness-fd EOF
plus consuming outer-sandbox reap; Cargo mocks are not evidence. A
deterministic beyond-ceiling plant proves pending cleanup, owned quarantine,
no reaped claim, no success/reuse, and eventual consuming reap by that same
monitor while the action stays failed. Separate mutations remove
`CLONE_NEWPID`, the teardown patch, fixed ceiling, pending state,
no-success/no-reuse rule, runbook/link/release records, or select reboot,
retry-before-release, replacement-waiter, manual-release, and every forbidden
strategy fallback. The patched sandbox owns the
execution-containment codes, corrections, renderer, and live byte-exact cases; T067/T068
own no `SANDBOX_*` row. The closed table is in `recovery-deadline.md`.

Nix/toolchain admission and patched-sandbox runtime failures use this closed
stage table. Every row names the fixed causing input from
`recovery-deadline.md` and renders no observed value:

| Stage | Code | Exact correction before rerun |
| --- | --- | --- |
| Unsupported native system before helper start | `D2B-BZLEXEC-NIX-PTRACE-SYSTEM` | Move evaluation and execution to native `x86_64-linux` or `aarch64-linux`; run `make test-flake`; run the exact closed slice retry command. |
| Kernel below the ptrace minimum before helper start | `D2B-BZLEXEC-TOOLCHAIN-PTRACE-KERNEL` | Migrate to a native supported runner with Linux 3.19 or newer; run `make test-flake`; run the exact closed slice retry command. |
| Yama parent-child refusal before helper start | `D2B-BZLEXEC-TOOLCHAIN-PTRACE-YAMA` | Migrate to a native supported runner whose boot policy fixes `kernel.yama.ptrace_scope=1`; grant no `CAP_SYS_PTRACE`; run `make test-flake`; run the exact closed slice retry command. |
| Real ptrace startup-probe failure before helper start | `D2B-BZLEXEC-TOOLCHAIN-PTRACE-PROBE` | Restore and rebuild the pinned static supervisor and probe; if exact identities pass but the probe refuses, migrate to a runner satisfying the kernel and Yama rows; run `make test-flake`; run the exact closed slice retry command. |
| Patched-sandbox ptrace seccomp-policy drift before helper start | `D2B-BZLEXEC-SANDBOX-PTRACE-POLICY` | Restore the pinned patch and policy so seccomp admits only the four request values with the enforceable constant arguments and retains every no-network denial; run `make test-flake`; then run the phase-valid closed slice command selected verbatim from the command-version table. |
| Bazel output identity | `D2B-BZLNET-BAZEL-IDENTITY` | Re-enter the repository Nix environment and restore the pinned Bazel output; run `make test-flake`; run the exact closed slice retry command. |
| Patched capability probe | `D2B-BZLNET-CAPABILITY` | Restore the repository-pinned Bazel patch and capability ABI; run `make test-flake`; run the exact closed slice retry command. |
| Strategy inventory | `D2B-BZLNET-STRATEGY` | Remove every non-sandboxed strategy or fallback; run `(cd packages && cargo xtask gen-bazel --check)`; run the exact closed slice retry command. |
| Fixed policy identity | `D2B-BZLNET-POLICY` | Restore `bazel/generated/action-network-policy.json` and the pinned Bazel package; run `(cd packages && cargo xtask gen-bazel --check)`; run `make test-flake`; run the exact closed slice retry command. |
| Inherited descriptor census could not complete | `D2B-BZLNET-PREFLIGHT` | Correct the runner descriptor-inspection capability; run `make test-flake`; run the exact closed slice retry command. |
| Inherited socket | `D2B-BZLNET-INHERITED-SOCKET` | Remove the inherited socket from the governed action or test fixture; run the exact closed slice retry command. |
| Inherited io_uring ring, including SQPOLL or registered/fixed-socket state | `D2B-BZLNET-INHERITED-RING` | Remove the inherited ring and every registered file from the governed action or test fixture; run the exact closed slice retry command. |
| `no_new_privs` | `D2B-BZLNET-NO-NEW-PRIVS` | Run the exact closed slice retry command on a supported Linux runner whose sandbox permits `no_new_privs`. |
| Filter load | `D2B-BZLNET-FILTER-LOAD` | Run `make test-flake`; run the exact closed slice retry command on a supported Linux runner with seccomp filter loading enabled. |
| Action-command exec | `D2B-BZLNET-EXEC` | Correct the configured action command and executable mode; run `(cd packages && cargo xtask gen-bazel --check)`; run the exact closed slice retry command. |

The retry command is the versioned typed enum from
`make-target-compatibility.md`. Before alias removal it renders exactly one
existing shadow slice target. Alias removal atomically changes the renderer and
all byte-exact tests to the corresponding enduring
`make test-rust-slice-{main,api,broker,aux}` target. No placeholder,
nonexistent target, or free-form string reaches a message. Every diagnostic contains only its
fixed code, the repository-relative
`bazel/generated/action-network-policy.json` row, that row's SHA-256, the
exact correction text, and the literal retry command. Exact-message tests for
every stage and slice reject descriptor numbers, absolute, runfiles, socket,
and Nix store paths, errno or other OS text, raw tool or child output, argv,
environment values, and process, user, run, attempt, candidate, or tag
identifiers.

The action inventory rejects a missing or wrong patched Bazel output,
capability mismatch, missing patch, policy mismatch, a stable/nightly
toolchain gap, an uncovered build-script, setup, or doctest action, an
action-level URL, live-index input, downloader, network-enabling tag,
process/local/standalone/worker/remote/no-sandbox strategy, fallback, or
missing declared offline input. These eight real pre-action plants must each
observe the fixed seccomp errno and fail if the patch is removed:

```text
action-network-ipv4
action-network-ipv6
action-network-netlink
action-network-packet
action-network-unix-pathname
action-network-unix-abstract
action-network-socketpair
action-network-io-uring
```

One test plant performs a forbidden socket operation from Bazel test setup
before the real test payload. Separate compile/build, test, and descendant
plants prove the same inherited filter placement. The external-egress and
live-index plants remain additional failures.
Preflight plants pass inherited IPv4 and Unix sockets, an ordinary io_uring
ring, an SQPOLL ring, and a ring with a registered fixed socket to the sandbox child
and require the matching inherited-capability code before filter load. An
injected descriptor backend covers SQPOLL on kernels that do not permit its
creation; a supported-kernel conformance leg covers a real inherited ring.
All socket-denial and inherited-capability plants belong only to the
hermeticity/action-network carrier. The stub carrier tests executable identity
and runtime state and owns no such plant. The only fetch rows are outside governed actions, offline during gates, and
pinned by a Cargo checksum or the `wl-proxy` revision plus archive sha256.

Committed mandatory tests that use sockets remain on the existing
non-Bazel Cargo compatibility path until a separately authorized design
changes the invariant. Their exact case census is generated from the Cargo
listing and committed in the coverage map. The same protected commit must
produce both the Bazel carrier verdict and every compatibility-carrier verdict
for their shared surface. Missing, skipped, advisory, stale-head, or
misattributed compatibility evidence fails surface completion and promotion.
Promotion reports these surfaces as hybrid and Cargo retirement retains the
compatibility executor and its public target. No endpoint declaration or
network namespace is cited as enforcement.

## Enforcing hybrid-disclosure policy

`packages/d2b-contract-tests/tests/policy_bazel_hybrid_docs.rs` is an
enforcing type-5 policy lint wired into the existing `make test-policy`
surface through `tests/lib.sh`; it is excluded from the fixture-dependent
lane so it runs exactly once. It derives the exact sorted nonempty
`cargoCompatibilityCarriers` census from the committed coverage map. Each
canonical disclosure entry carries the surface ID, Cargo selector, test
identity, and socket class; entries are not projected to surface ID, because
several retained cases may share one surface. The policy compares that complete
entry set in both directions with the semantic "Retained Cargo compatibility
cases" block in every governed document.

The fixed governed document set is `AGENTS.md`, `tests/AGENTS.md`,
`docs/contributing/gates-and-lints.md`, `tests/README.md`, and
`docs/reference/test-execution-manifest.md`. When present on a candidate,
`changelog.d/adr052-bazel-promotion.md`,
`changelog.d/adr052-bazel-alias-removal.md`, and
`changelog.d/adr052-cargo-retirement.md` are governed too. No other document
or fragment can opt itself into or out of the set. The parser accepts one
semantic block per governed file and rejects duplicates, an empty source
census, a missing block, a missing or extra case, a duplicated canonical
entry, a malformed surface/selector/test/socket-class field, and stale
coverage-map attribution. Multiple distinct cases under one surface are valid
and must all be disclosed. Isolated fixtures cover empty census, missing,
extra, malformed block, duplicate block, malformed full identity, duplicate
full identity, stale coverage-map attribution, and one governed document
disagreeing while the others remain valid. Repository positives prove every
governed file equals the exact source census. This test lands with promotion
disclosure, before Cargo retirement becomes possible.

## Test-first non-main carriers

The generated carrier files are deliberately disjoint:

| Carrier file | Surface |
| --- | --- |
| `bazel/carriers/schema.bzl` | One action runs two sequential generations into distinct directories, proving two independent nonempty exact censuses before comparison; mismatch and empty-output plants. |
| `bazel/carriers/stub.bzl` | Stub-no-socket executable identity and runtime-state checks; missing executable, wrong identity, and state-creation plants. It owns no socket-denial plant. |
| `bazel/carriers/inventory.bzl` | Pinned test inventory; empty, missing, and extra inventory plants. |
| `bazel/carriers/no_bash.bzl` | No-bash walker input and parsed-census wiring, separate from main. |

`bazel/carriers/main.bzl` is not a shared writer for these surfaces.

## Promoted public target mapping

Promotion introduces exactly four authoritative CI slice targets:

```text
test-rust-slice-main
test-rust-slice-api
test-rust-slice-broker
test-rust-slice-aux
```

Generated CI calls those names only. The eight existing public leaves retain
their current surface semantics and forward to these exact carrier subsets:

| Public leaf | Bazel subset after promotion |
| --- | --- |
| `test-rust-api-surface` | `//ci/rust:api_census`. |
| `test-rust-main` | `//ci/rust:fmt`, `//ci/rust:clippy`, `//ci/rust:main_tests`, `//ci/rust:main_doctests`, and `//ci/rust:main_harness_free`, plus the unchanged conditional Cargo/Nix fixture and CLI path. |
| `test-rust-broker` | `//ci/rust:broker_default`, `//ci/rust:broker_layer1`, and `//ci/rust:broker_fakebackends`. |
| `test-rust-guest-shell-runner` | `//ci/rust:guest_shell_runner`. |
| `test-rust-no-bash-ast` | `//ci/rust:no_bash_ast`. |
| `test-rust-schema` | `//ci/rust:schema_reproducibility`. |
| `test-rust-inventory` | `//ci/rust:stub_no_socket` and `//ci/rust:pinned_test_inventory`. |
| `test-rust-supply-chain` | `//ci/rust:deny_main`, `//ci/rust:deny_broker`, `//ci/rust:deny_guest`, `//ci/rust:audit_main`, `//ci/rust:audit_broker`, and `//ci/rust:audit_guest`; each deny carrier includes its yanked projection. |

## Guard placement

| Invariant | Enforcement |
| --- | --- |
| Mapped carrier label exists | Analysis-time `deps` or `data` edge |
| Carrier belongs to exactly one ID | Coverage test |
| No Rust test target is unclaimed | Make wrapper and `test-drift` over committed query result |
| Query result is current | `test-drift` |
| Exact census, topology, native target, cfg, feature, and fragment list | Coverage test |
| Hub and lock containment | Selected-context query checks |
| Generated BUILD and policy output current | `test-drift` |
| Broker suite keeps `tags = ["exclusive"]` and cannot overlap any test | Coverage test plus scheduling mutation |
| Every governed Bazel Rust action is no-network; mandatory socket tests remain exact same-commit Cargo compatibility carriers; every fetch is outside governed actions and pinned/offline | Exact Nix Bazel source/patch/policy/output identity and startup probe, configured-target plus `aquery` action-kind and strategy inventories, patch-removal/filter-load/setup-before-payload plants, inherited socket/ring/SQPOLL/fixed-socket plants, all eight syscall plants, external-egress/live-index plants, compatibility census, and `test-policy` |
| Every governed hybrid disclosure is exact | Enforcing type-5 `policy_bazel_hybrid_docs.rs` derives the nonempty full carrier identities from the coverage map and compares surface, selector, test identity, and socket class bidirectionally with every fixed hybrid document and present semantic migration fragment; isolated empty, missing, extra, malformed/duplicate block, malformed/duplicate identity, stale-attribution, and governed-document mismatch fixtures run under `test-policy`. |
| No-bash parsed-file census equals governed manifest and declared inputs | Walker unit tests plus coverage test |
| Generated `bazel/generated/no-shell-inventory.json` has equal nonempty governed and declared sets, one scan record per governed source including zero-site records, only governed spawn sites, and exact fresh-scan/committed spawn-site keys | Census-generator tests, coverage test, and `test-drift` |

The raw `scanResults` length and its unique-source length must each equal the
governed-source count. No Bazel test invokes `bazel query` or starts a nested
server.

## Required hand-written fragments

Exactly once:

- per-target nightly transition;
- `rustdoc_json` rule;
- pinned vendor repository rule;
- package-policy carriers and selected-source census checker;
- product and walker hub containment checker;
- aggregate, slice, carrier, and coverage guards.

There is no synthetic splice fragment and no `crate.spec` fragment.

## Fail-closed cases

The guard refuses missing, duplicate, or added IDs; empty carriers; multiply
claimed carriers; absent labels; unclaimed Rust tests; missing topology or
census; stale query or BUILD output; missing fragment; empty scan or companion
sets; mismatched configured native target dependencies, cfgs, or features;
wrong product or walker containment; cross-context edges; unrelated
first-party siblings; any first-party target represented as an external
generated crate; a broker tag removal or overlap; a missing/wrong patched
Bazel output, source/patch/policy/output/capability mismatch, failed startup
probe, patch removal, stable/nightly/build-script/setup/doctest action-kind
gap, process/local/standalone/worker/remote or other strategy fallback,
preflight or filter fallback, inherited socket, inherited ordinary or SQPOLL
ring, registered fixed-socket ring state, setup-before-payload or any of the
eight socket/io_uring plants succeeding or returning a non-policy errno,
forbidden external egress,
a live-index input, missing, stale, advisory, or wrong-head Cargo
compatibility evidence, or a governed hybrid document or semantic fragment
with an empty census, missing/extra case, malformed/duplicate block,
malformed/duplicate full identity, stale attribution, or governed-document
mismatch; a no-bash walk, read, or parse
failure or mismatch among
the governed manifest, declared inputs, and parsed-file census; and an empty,
missing-entry, extra-entry, planted-shell, governed/declared mismatch,
unguarded-spawn-site, missing-zero-site-scan-record, or
fresh-scan/committed-spawn-mismatch no-shell inventory, including duplicate
raw scan records whose unique projection would otherwise hide the duplicate.
The six named plant records are exactly `no-shell-inventory-empty`,
`no-shell-inventory-missing-entry`, `no-shell-inventory-extra-entry`,
`no-shell-inventory-unguarded-spawn`,
`no-shell-inventory-missing-zero-site-record`, and
`no-shell-inventory-planted-shell`.
