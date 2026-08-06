# Runner Environment and Per-Case Evidence Contract

ADR 0054 changes where product crates resolve, not the runner isolation
contract established by ADR 0052.

## Context selection

- Main and guest carriers run one fresh process per exact libtest case.
- Broker default, layer1, and fake contexts run one process per test binary,
  bounded internal threads, carry exactly `tags = ["exclusive"]`, and never
  overlap each other or any other test.
- Broker and guest targets are native first-party targets with explicit
  configured dependencies and features.
- The external `@product` union cannot select a test topology.
- Walker execution comes from the separate `@walker` hub.

## Child environment

- Derive from the Bazel test environment and forward only declared values.
- Give every case its own directory beneath `TEST_TMPDIR`.
- Resolve each test binary from declared runfiles.
- Use `D2B_RUST_BUDGET` as the only concurrency control.
- Validate the budget once as a positive integer with value-redacted errors,
  propagate the effective value to Bazel jobs, local test jobs, runner
  process-per-case concurrency, and broker libtest threads, and prove the
  combined live process count never exceeds it. Scheduler-only, suite-only,
  invalid-value, and multiplicative-limit mutations are rejected.

## Provider contract

The locator selects Cargo or Bazel mode once. A Bazel miss never falls back to
Cargo.

The shared filesystem boundary:

1. validates a nonempty declared runfiles-relative provider key with no
   absolute or `..` component;
2. opens one provider descriptor with `O_RDONLY|O_CLOEXEC`;
3. resolves with `RESOLVE_NO_MAGICLINKS` only and deliberately without
   `RESOLVE_BENEATH` or `RESOLVE_NO_SYMLINKS`, because a Bazel runfiles leaf
   symlink may escape the anchor;
4. on the forced component-walk fallback, opens each intermediate component
   with `O_DIRECTORY|O_NOFOLLOW|O_CLOEXEC`, permits the declared leaf symlink,
   opens the leaf `O_RDONLY|O_CLOEXEC` without `O_NOFOLLOW`, and then applies
   every handle check;
5. refuses `ENOSYS` from `execveat` and names the kernel requirement;
6. checks regular-file kind, executable mode, freshness, and exact digest;
7. brackets the digest read with matching descriptor metadata;
8. returns an unforgeable verified handle; and
9. consumes the handle into the reviewed safe command-fd mapping, which gives
   the immutable static C supervisor a private descriptor sharing the verified
   descriptor's original open file description while preserving declared
   stdio; the supervisor forks once and the child executes it with `execveat`
   and `AT_EMPTY_PATH`.

No provider path is returned. No `Command` by path, `fexecve`, or
`/proc/self/fd` fallback is permitted.

`VerifiedExecutable` is an API seal, not only a runtime convention. Its fields
and minting trait remain private to the provider module. Its public inherent
API allowlist is empty: callers receive it from the provider and can only pass
it by value to the execution function. The defining crate exposes no
descriptor extraction or access, unchecked constructor, path conversion or
accessor, `Deref`, `Borrow<OwnedFd>`, `AsFd`, `AsRawFd`, `IntoRawFd`,
`Default`, `From`, `Into`, `AsRef`, `Clone`, `Copy`, `Debug`, `Display`,
`Serialize`, or `Deserialize`.

The compiler-derived API census under `packages/d2b-api-surface/` is the
authority. `VerifiedExecutable` is a capability root. Its public item snapshot
must contain only the opaque type and the by-value provider/execution
signatures, its explicit locally-authored trait-implementation allowlist is
empty, and its compiler-emitted auto/blanket implementation set is pinned
exactly for the selected toolchain. Any added public field, method, associated
item, re-export, explicit trait implementation, or changed auto/blanket set is
an API-surface failure. Focused rustdoc `compile_fail` examples prove the
downstream type-system properties that the census alone cannot: callers cannot
construct the type, access or extract its descriptor, coerce it through
`Deref`/`Borrow`/`AsFd`, clone it, serialize or format it, convert an
unverified path or descriptor into it, or implement the sealed minting trait.
There are no Cargo-shelling compile fixtures.

`VerifiedExecutable` and its only consuming public API are co-located in one
dependency-leaf crate. No other crate can name a consuming trait or method.
The safe Rust API consumes the handle by value and invokes
`d2b-bazel-exec-supervisor` only from the exact immutable Nix store path
supplied by the pinned toolchain artifact. It accepts no helper path parameter
and reads no environment override. A closed source census permits exactly this
one Rust invocation site.

The helper is not a Rust crate. One tiny reviewed C source under
`tests/tools/d2b-bazel-exec-supervisor/` is built by the dedicated
`pkgs/d2b-bazel-exec-supervisor` static Nix derivation. It is build/test
tooling and is absent from the product workspace and product Bazel hub. The
committed identity binds the exact C source digest, derivation dependency
closure hashes, output NAR hash, executable hash, protocol version, and
native-system identity. The exact store path is embedded from that toolchain
artifact; the record persists no complete store path.

### Patched-sandbox containment precondition

Live supervisor execution is valid only beneath the exact Nix-patched Bazel
Linux `sandboxed` strategy. Every governed action gets a fresh PID namespace.
The patched namespace PID 1 is the sandbox monitor, remains outside the action
command process tree, and is the crash-surviving owner of setup, the Rust
runner, the supervisor, the target, and every descendant. The monitor is the
only abnormal-teardown authority.

The normal path belongs to the supervisor: it sends TERM, waits the fixed
normal grace, sends unconditional KILL, and reaps the target. If action setup
or the action command exits before that protocol completes, including Rust
parent or supervisor crash at any protocol stage, the sandbox monitor starts
abnormal teardown. It sends namespace-local SIGKILL to every process other
than itself and makes nonblocking reap progress. Exactly one fixed 10,000 ms
monotonic ceiling bounds userspace TERM/KILL/monitor escalation and the
close-or-quarantine decision only. It never bounds kernel task exit,
namespace destruction, or reap.

If a consuming wait has not proved every member and PID 1 reaped at that
ceiling, including when an uninterruptible `D`-state task delays kernel
cleanup, outer `linux-sandbox` enters typed `pending-kernel-cleanup`, remains
the wait owner, and quarantines the sandbox and its outputs. The action can
never report success or reuse those resources. The owner continues
nonblocking observation until a consuming wait proves PID 1 reaped. Cleanup
then becomes `complete-after-quarantine`, but the action remains failed. A
kill/reap operation failure takes the same quarantine path. There is no
configurable second grace and no cgroup, PID-file, host PID, or process-group
fallback.

Rust never sends a signal to a numeric PID or PGID. On a post-spawn protocol
or wait failure it closes owned descriptors, preserves the first typed cause,
and returns the whole Bazel action nonzero immediately. That action exit is
the handoff to the sandbox monitor. Cargo tests use injected process,
transport, and containment mocks only. The live proof invokes the real
Nix-patched Bazel Linux sandbox and observes namespace teardown.

The startup and configured-strategy gates bind `CLONE_NEWPID`, the patched
PID-1 teardown code, the fixed userspace ceiling, the closed quarantine state,
and the absence of every strategy fallback. A runtime sandbox integration
plants helper crash before `READY`, after `READY`, after `EXECUTED`, during
supervisor TERM grace, and while direct and double-forked descendants remain
live. Ordinary plants must leave the monitor and outer sandbox observably
reaped and make the descendant's inherited liveness fd reach EOF. The
beyond-ceiling plant must instead prove owned `pending-kernel-cleanup`, no
reaped claim, no success, and no reuse before a controlled release permits
eventual consuming reap. Separate mutations remove the PID namespace,
teardown patch, quarantine state, no-reuse rule, or alter the ceiling, and
select each forbidden fallback strategy; each mutation fails without
signaling any host process.

The Rust parent validates declared stdin, stdout, and stderr and creates the
fixed supervisor status channel. The exact pinned reviewed safe
`command-fds` API maps the consumed verified open file description to its
fixed private fd outside 0, 1, and 2. `std::process::Command` preserves the
three declared stdio streams and spawns the exact helper path. Every new Rust
crate retains `unsafe_code = "forbid"`; first-party Rust defines no raw fork,
`pre_exec`, signal callback, or unsafe helper exception.

The helper starts and remains single-threaded. Its first signal operation
blocks the complete managed set `SIGHUP`, `SIGINT`, `SIGTERM`, and `SIGQUIT`;
it does not first expose an inherited or empty mask. While that set remains
blocked, it validates private-fd identity and declared stdio ownership,
restores default dispositions for every catchable signal, explicitly ignores
`SIGPIPE`, restores `SIGCHLD` to `SIG_DFL` with neither `SA_NOCLDWAIT` nor
`SA_NOCLDSTOP`, and installs the fixed synchronous signal consumer. Only
after every disposition and synchronous-consumption resource is ready does it
establish the final mask: the managed set stays blocked for synchronous
consumption and every other catchable signal is unblocked. A pending managed
signal is consumed, never discarded, before fork and at each pre-`READY`
transition.

The supervisor owns every managed termination signal from the first block. A
managed signal already pending at entry or arriving during normalization
records one `termination-requested` transition; the required fixtures plant
`SIGTERM` at both times. Before a child exists it refuses without forking.
After fork but before `READY` it performs the fixed child-group termination
and direct-child reap, closes every owned descriptor, emits no `READY`, and
exits with the closed helper failure status. The Rust parent therefore reports
`PARENT_READY`; neither the target nor the sandbox monitor inherits ambiguous
pre-`READY` termination ownership. `SIGPIPE` remains
ignored, so a closed status reader is typed `EPIPE`. Child status remains
waitable. The helper then creates exactly one
`O_CLOEXEC|O_NONBLOCK` child exec-error pipe before forking exactly once.
The child establishes the target process group, restores an empty signal mask
and default dispositions for every catchable signal including `SIGTERM` and
`SIGPIPE`, installs the declared stdin/stdout/stderr at 0/1/2, sets the
executable fd `FD_CLOEXEC`, closes every supervisor-only and non-surviving
descriptor, and calls
`execveat(private_fd, "", argv, envp, AT_EMPTY_PATH)`. There is no target path,
reopen, `fexecve`, `/proc/self/fd`, or fallback. A child setup or exec failure
writes exactly one fixed-size exec-error record with bounded
`EINTR`/`EAGAIN`/short-write handling under the absolute monotonic deadline,
then calls `_exit`.

The supervisor closes its exec-error writer after fork and emits exactly one
`READY` status frame to the Rust parent. It reads the exec-error pipe to
either empty close-on-exec EOF or one complete fixed failure record. Every
exec-error read and every supervisor-status read or write uses its original
absolute monotonic deadline: `EINTR` retries only after a budget check,
`EAGAIN` waits only for the remaining budget, and a short operation advances
an exact byte cursor without resetting time.

The exec-error pipe remains a single-record protocol. Its reader accepts only
empty EOF or exactly one fixed failure record followed by EOF, and uses one
additional byte only to distinguish overlong input. EOF after any failure
record byte is partial. A held writer, second record, unknown record, timeout,
or I/O failure is typed. Empty EOF is the only exec success.

The status pipe is a distinct framed stream and never uses the exec-error
pipe's one-byte overlong probe. Every frame has this fixed eight-byte header:
four ASCII bytes `D2BS`, version byte `1`, one type byte, and one unsigned
16-bit big-endian payload length. The closed types are `READY = 1` with
length zero, `EXECUTED = 2` with length zero, `EXITED = 3` with one unsigned
exit-code byte, and `SIGNALED = 4` with one signal byte in `1..64`. Any other
magic, version, type, length, or signal value is malformed.

The Rust parent owns one stateful decoder with a fixed 27-byte buffer, enough
for three maximum-size frames and therefore for encoded `READY` plus
`EXECUTED` plus one terminal frame. Reads may
fragment at every byte or coalesce all three frames. The decoder retains every
unconsumed complete or partial frame, parses as many complete frames as are
available, and advances only through
`Start -> Ready -> Executed -> Terminal -> EOF`. Buffer exhaustion, a frame
before its predecessor, a duplicate, bytes after terminal, EOF with a partial
frame, or EOF before terminal is typed failure. After terminal, the parent
drains to EOF before accepting the stream; the supervisor closes its writer
before mirroring status. This framing, rather than a byte beyond a record,
detects status overrun without consuming the first byte of a valid coalesced
frame.

A status writer completes each frame or returns typed timeout, `EPIPE`, or I/O
failure. Empty exec-error EOF causes exactly one `EXECUTED` frame. Partial,
overlong, duplicate, malformed, unknown, held-open-writer, timeout, or I/O
failure on either protocol is a typed helper failure at that protocol's
closed stage.

The supervisor remains alive after `EXECUTED`. It forwards only `SIGHUP`,
`SIGINT`, `SIGTERM`, and `SIGQUIT` to the target process group. Case-deadline
expiry starts the existing fixed TERM/full-grace/unconditional-KILL sequence.
An external `SIGTERM` starts the same sequence even when the case has no
deadline: TERM is forwarded once, the complete fixed grace elapses, and KILL
is unconditional if the group could still exist. The supervisor waits and
reaps the direct child, emits one fixed terminal status, and mirrors the exact
target result: the same normal exit code or the same terminating signal.
Inherited blocked or ignored termination state cannot reach either supervisor
or target because both masks and dispositions were normalized. A signal,
wait, reap, terminal-write, or exact-status-mirroring failure is a typed helper
failure.

Rust accepts no inferred exec result. Helper crash, helper exit, or status
channel EOF before `EXECUTED` is always a typed helper failure, regardless of
the helper process status. A target that execs and immediately exits with the
same status as a planted helper crash is accepted only when `READY`,
`EXECUTED`, the terminal record, and the mirrored status all agree.

No runfiles, worktree, copied, symlinked, caller-supplied, or fd-0 helper
transport is permitted. A closed invocation-site policy derives the complete
Rust/Bazel/Make/workflow spawn census and permits direct helper invocation only
at the typed consumer implementation site. The helper is unprivileged, but a
direct invocation elsewhere remains a policy failure.

Tests prove the API consumes the handle and exposes no descriptor; the mapped
private fd refers to the original verified open file description; a marker on
declared stdin reaches the target unchanged; stdout and stderr retain their
declared destinations; and provider, private executable, exec-error, status,
and auxiliary descriptors are absent from the target. The host conformance
test rebinds and mutates the provider path after verification and proves
execution still uses the verified open file description while no path appears
in argv, environment, evidence, or diagnostics. Mutations cover missing/wrong
helper output, copied or rebound helper, runfiles/worktree path, any second
Rust invocation site, fd-0 transport, absent/wrong private fd, reopen, `/proc`,
path fallback, non-CLOEXEC private and auxiliary descriptors, leaked provider
fd, replaced stdin, held-open exec-error writer, partial/overlong/malformed
exec-error and supervisor transport, helper EOF/crash before `EXECUTED`, fast
same-exit-status target versus helper crash, closed status reader, inherited
ignored or `SA_NOCLDWAIT` `SIGCHLD`, inherited blocked or ignored `SIGTERM`,
target-ignore-TERM escalation with no case deadline, disallowed or lost signal
forwarding, target-status mismatch, and any first-party Rust unsafe allowance.
Crash before `READY`, after `READY`, after `EXECUTED`, during grace, and with
long-lived descendants is covered by real patched-sandbox integration rather
than a Cargo mock.

### Rust parent stage, owner, and closure contract

| Stage | Owned resources and success transition | Injected failure cleanup |
| --- | --- | --- |
| `Verified` | Consumed handle exclusively owns the provider `OwnedFd`. | Identity or argument failure drops the provider once; no channel or child exists. |
| `HelperIdentity` | Exact immutable store path and every C source/derivation-dependency/output/protocol digest match. | Missing, wrong, copied, symlinked, runfiles, worktree, or rebound output drops the provider; no spawn. |
| `Prepared` | Rust owns provider fd, status reader/writer, mapping configuration, and declared stdio. | `PARENT_PREPARE`; close provider and both channel ends without changing stdio ownership. |
| `Spawned` | `std::process::Child` becomes the sole wait owner for the supervisor. Rust immediately closes its mapped provider and helper-side status copies and retains only the reader plus `Child`. | `PARENT_SPAWN` before a child exists or `PARENT_CLOSE` after spawn; close owned fds and return the action nonzero without signaling a PID or PGID. |
| `Ready` | The stateful decoder consumes one complete framed `READY`; supervisor remains wait-owned through `Child`. | `PARENT_READY` for EOF, exit, timeout, partial header/payload, buffer overflow, malformed header, duplicate, unknown, or out-of-order status; close owned fds and return the action nonzero so sandbox teardown owns survivors. |
| `Executed` | The retained decoder consumes one complete framed `EXECUTED` after `READY`; no target status is inferred from helper wait status. | `PARENT_EXECUTED` for EOF, exit, timeout, malformed input, or any frame before `EXECUTED`; close and return nonzero without a Rust signal operation. |
| `Terminal` | The decoder consumes one framed `EXITED` or `SIGNALED`, rejects retained or later trailing bytes, drains to EOF, then waits for the supervisor and verifies exact status equality. | `PARENT_TERMINAL`, `PARENT_WAIT`, or `PARENT_STATUS`; block publication, close owned fds, and return nonzero to the sandbox owner. |
| `Cleaned` | Status reader closes once and the supervisor is reaped by the successful wait path. | `PARENT_CLEANUP` attaches to the first failure; close is attempted once, raw OS text is never rendered, and Rust never performs numeric signal cleanup. |

### C supervisor stage, error, owner, and closure contract

| Stage | Owned resources and success transition | Typed failure and mandatory cleanup |
| --- | --- | --- |
| `Adopted` | Supervisor owns mapped executable fd, status writer, argv/environment, and declared stdio. | `HELPER_ADOPT` for absent/colliding/wrong descriptors; close all owned fds; no fork. |
| `Normalized` | Supervisor first blocks the managed set, then while blocked installs default dispositions, ignored `SIGPIPE`, waitable default `SIGCHLD`, and the synchronous consumer, and only then establishes the final mask. Pending or normalization-time `SIGTERM` is consumed into the owned pre-`READY` termination transition. | `HELPER_SIGNAL_NORMALIZE`; close all owned fds; no fork. |
| `ExecPipe` | Supervisor creates and owns exactly one `O_CLOEXEC|O_NONBLOCK` reader/writer pair. | `HELPER_EXEC_PIPE`; close both ends and prior resources; no fork. |
| `Forked` | Exactly one target child exists. Supervisor owns child pid, exec-error reader, and Rust status writer; child owns writer, executable fd, and stdio copies. | `HELPER_FORK` before child creation, or a later typed failure followed by target-group kill, direct-child reap, and closure of every supervisor fd. |
| `ChildSetup` | Child establishes target group, resets mask/dispositions, installs 0/1/2, marks executable fd CLOEXEC, and closes supervisor-only fds. | Fixed `CHILD_GROUP`, `CHILD_SIGNAL`, `CHILD_STDIO`, `CHILD_CLOEXEC`, or `CHILD_CLOSE` exec-error record; bounded exact write; `_exit`. |
| `Execveat` | Child calls `execveat` on the same open file description. Successful exec closes the error writer and executable fd. | Fixed `CHILD_EXECVEAT` record, including typed `ENOSYS`; bounded exact write; `_exit`; no fallback. |
| `ExecResult` | Supervisor emits framed `READY`, then accepts only empty EOF or one complete child failure record under one absolute deadline. Empty EOF emits framed `EXECUTED`; every exec-error cursor and retry remains under that deadline. | `HELPER_EXEC_TIMEOUT`, `HELPER_EXEC_PARTIAL`, `HELPER_EXEC_OVERLONG`, `HELPER_EXEC_UNKNOWN`, `HELPER_EXEC_EPIPE`, or `HELPER_EXEC_IO`; kill and reap target; close reader/status. |
| `Supervising` | Supervisor owns the live target group and status writer, forwards only the four allowed termination signals, and applies the fixed escalation on case expiry or external `SIGTERM`, including when no case deadline exists. | `HELPER_SIGNAL_FORWARD` or `HELPER_DEADLINE`; full grace and unconditional target-group kill when required, direct-child reap, typed failure. |
| `Reaped` | Supervisor has exact terminal wait status and no live child; it writes the framed `EXITED` or `SIGNALED` terminal, closes the status writer, and mirrors that status. | `HELPER_WAIT`, `HELPER_REAP`, `HELPER_TERMINAL_WRITE`, or `HELPER_STATUS_MIRROR`; retain the first cause and close every remaining fd. |
| `Closed` | No provider/private/pipe/status descriptor and no unreaped child remains. | `HELPER_CLEANUP` is attached to the first failure; cleanup is attempted on every reachable path. |

No successful stage owns an unclosed parent-only fd or an unreaped child.
No child path returns through C after fork: it either execs or calls `_exit`.

### Patched sandbox stage, error, owner, and closure contract

| Stage | Owned resources and success transition | Typed failure and mandatory cleanup |
| --- | --- | --- |
| `NamespaceCreated` | Outer `linux-sandbox` owns exactly one fresh `CLONE_NEWPID` child and the PID-1 synchronization pipes. | `SANDBOX_NAMESPACE`; no action exec and reap any created monitor. |
| `MonitorReady` | Namespace PID 1 owns the action command and is the adoption point for every orphan; outer sandbox wait-owns PID 1. | `SANDBOX_MONITOR`; close synchronization fds, force namespace-init exit, and outer-reap. |
| `ActionRunning` | PID 1 waits and reaps descendants while the action command runs; normal target escalation remains supervisor-owned. | Abnormal setup/action exit, including parent or supervisor crash, transitions once to `Aborting`. |
| `Aborting` | PID 1 sends namespace-local SIGKILL to all other namespace members and performs nonblocking reap progress. The fixed 10,000 ms ceiling bounds userspace TERM/KILL/monitor escalation and the decision to close or quarantine; it does not bound kernel task exit or reap. | `SANDBOX_KILL`, `SANDBOX_REAP`, or `SANDBOX_CEILING`; retain the first cause and never claim a namespace member or PID 1 reaped without a consuming wait result. |
| `PendingKernelCleanup` | At the userspace ceiling, any namespace member or PID 1 still not observably reaped enters closed result `pending-kernel-cleanup`. Outer `linux-sandbox` remains the wait owner, marks the sandbox and outputs quarantined, continues bounded nonblocking observation, and permits neither success nor sandbox/output reuse. | `SANDBOX_PENDING_KERNEL_CLEANUP`; keep quarantine owned until an exact consuming wait proves PID 1 reaped. The operator removes the worker from admission, corrects the kernel, filesystem, or device stall or reboots it, and retries only after quarantine release. |
| `Closed` | A consuming wait proved PID 1 reaped, no synchronization fd or outer waitable child remains, and cleanup is `complete` or `complete-after-quarantine`. Entry through quarantine keeps the action result nonzero permanently. | `SANDBOX_CLEANUP` attaches to the first sandbox failure; no host PID, PID file, or host process group is signaled. |

All absent, non-regular, non-executable, stale, wrong-digest, rebound-path,
short-read, metadata-change, and exec-stage cases are injected. Host-backed
conformance exercises the exact static supervisor against a declared
first-party probe. It covers closed-reader `EPIPE`; inherited ignored and
`SA_NOCLDWAIT` `SIGCHLD`; inherited blocked and ignored `SIGTERM`; a target
that ignores TERM with no case deadline; each exact `EINTR`, `EAGAIN`, short,
fragmented and coalesced status frames, partial single-record exec-error,
duplicate, malformed, and held-writer transport boundary;
a target that execs and exits immediately with the planted helper-crash
status; exact signal forwarding and target status; unchanged declared stdin;
separate stdout and stderr; absence of every provider, private, exec-error,
and status descriptor; and presence only of a planted descriptor explicitly
declared to survive.

The host-backed supervisor cases do not claim crash containment. A separate
real Bazel sandbox integration runs the action through the exact patched
`linux-sandbox`, plants helper crash before `READY`, after `READY`, after
`EXECUTED`, and during TERM grace, and leaves both direct and double-forked
long-lived descendants. The ordinary plants prove namespace kill, consuming
PID-1 and outer-monitor reap, and liveness-fd EOF. A separate deterministic
beyond-ceiling plant withholds the monitor's reap completion until after the
userspace ceiling, proves typed `pending-kernel-cleanup`, owned quarantine,
no success and no reuse, and absence of any reaped claim, then releases the
wait and proves `complete-after-quarantine` while the action remains failed.
Namespace removal, teardown-patch removal, changed-ceiling,
pending-state/remediation removal, and strategy-fallback builds must each make
that integration fail. Cargo unit tests exercise only mocks and cannot satisfy
this live done condition.

Every auxiliary descriptor is close-on-exec: the runfiles anchor, provider,
freshness-input handles, per-case directory, stdio setup copies not intended
for the child, and exec-error pipe. A behavioral child enumerates its own
descriptor table. One mutation clears `O_CLOEXEC` at each auxiliary-descriptor
position in turn and must make that test fail. Source-marker checks are not
accepted as proof of descriptor inheritance.

Provider tests separately force the `openat2` and component-walk routes. The
fallback cases prove intermediate symlink refusal, declared leaf-symlink
acceptance followed by full identity verification, and identical
same-open-file-description execution through the private descriptor. Mutations
that add `RESOLVE_BENEATH` or
`RESOLVE_NO_SYMLINKS`, place `O_NOFOLLOW` on the provider leaf, reopen for
digest or exec, use a path/`fexecve`/`/proc` fallback, or fall back after
`ENOSYS` must fail.

Every provider refusal is nonzero, redacted, and actionable:

| Reason | Stable input named | Exact recovery |
| --- | --- | --- |
| Runfiles entry missing in Bazel mode | Declared runfiles-relative provider key | `make test-bazel-rust-main`, `make test-bazel-rust-api`, `make test-bazel-rust-broker`, or `make test-bazel-rust-aux`, selected only by the closed coverage-map slice enum after declaring the key as `data`. |
| Provider is not a regular file | Declared runfiles-relative provider key | The same exact closed slice command after correcting the named target's `data` declaration. |
| Provider is not executable | Declared runfiles-relative provider key and mode | The same exact closed slice command after rebuilding the named target. |
| Provider is older than its newest declared input | Declared runfiles-relative provider key | The same exact closed slice command after rebuilding the named target. |
| Provider digest differs from the coverage map | Declared runfiles-relative provider key and coverage-map row | `(cd packages && cargo xtask gen-bazel --check)`, then the exact closed slice command after regenerating and reviewing the coverage map. |
| Handle metadata changed across digest read | Declared runfiles-relative provider key | The exact closed slice command; if it repeats, correct the writer named by the repository-relative coverage row and rerun that same command. |
| `execveat` returned `ENOSYS` | Stable kernel requirement | The exact closed slice command on a supported kernel providing `execveat`; no path fallback is available. |
| Other typed exec errno | Declared runfiles-relative provider key and errno class | The exact closed slice command after rebuilding the named target. |

The renderer accepts only the closed versioned slice-command enum; there is no
free-form command string. Before alias removal, command version 1 renders the
existing shadow slice target. Alias removal atomically updates every renderer
and byte-exact test to command version 2's corresponding enduring
`make test-rust-slice-{main,api,broker,aux}` target. No repository state may
name a target absent from that state. The declared runfiles-relative provider key is
repository content and is permitted in the refusal. The runfiles root,
resolved absolute location, descriptor number, argv, environment value, and
child output remain forbidden. Exact-message tests cover every reason in every
slice and reject an omitted key, omitted action, omitted rerun, borrowed
remedy, nonliteral command, or leaked local value.

Provider resolution is intentionally distinct from evidence and cleanup
resolution. Per-case directories, JUnit parents, execution-manifest parents,
and cleanup subtrees retain
`RESOLVE_BENEATH|RESOLVE_NO_SYMLINKS|RESOLVE_NO_MAGICLINKS`, with an
equivalent strict forced walk.

## Per-case result document

Write one JUnit document to `XML_OUTPUT_FILE` after all children are reaped.
Each enumerated case has one explicit `passed`, `failed`, or `ignored` entry.

Permitted content:

- stable case name;
- outcome;
- bounded duration;
- bounded sanitized failure text from a closed diagnostic-code table.

Forbidden content:

- environment values or argv;
- absolute, worktree, runfiles-root, store, or socket paths;
- process or user identifiers;
- opaque handles;
- unit names;
- shell names;
- terminal bytes;
- raw child output.

No sink receives raw child output. The runner sanitizes and bounds the stream
before writing JUnit, Bazel `test.log`, or emitted execution and qualification
evidence. `bazel/generated/evidence-sink-policy.json` is the committed
authority for each sink's maximum bytes, maximum records, closed permitted
fields, truncation code, and retention class. Retention classes are closed:

| Class | Sink | Maximum age | Maximum count and scope |
| --- | --- | ---: | --- |
| `junit-v1` | JUnit | 14 days | 128 files per slice output root |
| `test-log-v1` | `test.log` | 14 days | 128 files per slice output root |
| `evidence-v1` | unsealed execution and qualification evidence | 30 days | 32 files per workflow and head digest |
| `exporter-diagnostic-v1` | exporter diagnostics | 7 days | 64 records per workflow and head digest |

Sealed, schema-bounded source records under this specification are state
documents, not raw sink payloads; they remain one atomically replaced record
per declared path. Every other persisted sink must name exactly one class.
Before publication, descriptor-relative expiry removes owned entries older
than the class age and then retains only the newest permitted count. Failure
to classify, inspect, or expire refuses publication. CI upload configuration
uses the same literal age. Injected-clock tests cover just-inside, exact-bound,
and expired ages; count-minus-one, exact-count, and count-plus-one inventories;
newest retention; unowned/link refusal; and expiry failure with no
publication. Initial limits are generated
from measured sanitized fixtures and committed with the measurements; a limit
or permitted-field change requires the measured old and new values, an
explicit allowed delta, and review in the same change. Truncation emits only
the stable `D2B-BZLEVIDENCE-TRUNCATED` code and never a prefix or suffix of
forbidden bytes.

The planted fixture places distinct forbidden values in environment, argv,
failure text, stdout, and stderr. It first proves every value reached the
pre-sanitization stream, then proves every value is absent from JUnit,
`test.log`, emitted manifest evidence, emitted qualification evidence, and
all exporter diagnostics. Each sink is also proved at or below its committed
byte and record limit.

Test outcome and evidence publication are separate typed results.
`testVerdict` is the underlying `passed`, `failed`, `ignored`, or
`interrupted` result and is never rewritten by an exporter. `evidenceStatus`
is a closed tagged union inside one `EvidenceSinkResult`. The common record
carries `sinkKind` and `retentionClass` exactly once:

```text
{
  "sinkKind": "<closed>",
  "retentionClass": "<closed>",
  "testVerdict": "<closed>",
  "evidenceStatus": {
    "kind": "complete",
    "sinkPolicySha256": "<sha256>"
  }
}
```

or:

```text
{
  "sinkKind": "<closed>",
  "retentionClass": "<closed>",
  "testVerdict": "<closed>",
  "evidenceStatus": {
    "kind": "degraded",
    "code": "<closed-code>",
    "policyRowSha256": "<sha256>",
    "retryCommand": "<closed-command>"
  }
}
```

`sinkKind` determines exactly one retention class through the committed sink
policy; a mismatched pair refuses. Neither variant repeats either field. The
complete variant rejects degradation-only fields. The degraded variant
requires every field above and rejects complete-only fields, unknown fields,
unknown codes, and free-form commands. A sanitizer, bound, retention, write,
rename, exporter, or workflow-publication failure preserves `testVerdict` and
produces the structurally valid degraded variant. Surface completion and
qualification reject degraded evidence but report the evidence refusal
separately rather than claiming the underlying test failed. Execution-manifest
v1 remains byte- and schema-compatible: the tagged status is a sidecar
publication result and is never added to manifest v1.

The exact redacted remediation table is:

| Code | Stable input named | Exact recovery |
| --- | --- | --- |
| `D2B-BZLEVIDENCE-SANITIZE` | Repository-relative carrier definition and sink kind | Correct the sanitizer or closed permitted-field table, then run the exact closed slice command selected from the provider table. |
| `D2B-BZLEVIDENCE-LIMIT` | Repository-relative `bazel/generated/evidence-sink-policy.json` row and sink kind | Reduce the emitted diagnostic, or run `(cd packages && cargo xtask gen-bazel --check)` after reviewing measured policy changes, then run the exact closed slice command. |
| `D2B-BZLEVIDENCE-RETENTION` | Repository-relative sink-policy row and retention class | Correct the owned retention inventory, then run the exact closed slice command. |
| `D2B-BZLEVIDENCE-PUBLISH` | Stable carrier label and sink kind | Correct the publication backend, run the exact closed slice command, and require the complete tagged variant. |
| `D2B-BZLEVIDENCE-NO-VERDICT` | Stable workflow name and protected branch | `git fetch origin v3`, then `(cd packages && cargo xtask bazel-qualification-validate)`; if the fixed record remains incomplete, merge a new protected `v3` commit and rerun the same validator. |

Messages contain none of the forbidden planted values, no `$!`, run ID,
attempt ID, absolute path, Nix store path, cache key, token, opaque handle, or
raw exporter error. Artifact and validator failures render a fixed code,
repository-relative policy row, and SHA-256 only. Exact-message tests cover
every code/slice combination and reject an omitted stable input, omitted
command, borrowed remedy, leaked value, free-form command, or malformed or
success-shaped status variant.

Runner tests explicitly cover:

- prior manifest invalidation before dispatch;
- attribution when one surface has several carriers;
- sorted atomic partial manifest v1 publication after success, failure, and
  handled interruption;
- preservation of the original nonzero test or interruption status when
  publication also fails;
- exact ignored-case fidelity in listing, JUnit, and surface census;
- a planted failed result whose environment, argv, paths, identifiers,
  handles, shell name, terminal bytes, and raw output contain every forbidden
  redaction value. The fixture first proves every value is present, then proves
  all are absent from JUnit, `test.log`, emitted evidence, and exporter
  diagnostics.

## Filesystem semantics

`TEST_TMPDIR` and the output parent are anchored close-on-exec descriptors.
Strict paths refuse symlink, magic-link, and `..` traversal on both the
`openat2` and forced component-walk routes. Temporary creation, write, sync,
rename, and cleanup are descriptor-relative.

Creation collisions, short writes, `EINTR`, `EAGAIN`, `ENOSPC`, terminal
write failures, link parents, existing case directories, and cleanup ownership
are injected and mutation-tested.

## No-shell scope

Repository-owned wrapper, runner, cleanup, timeout, and process-control code
invokes no shell. The `rules_rust` stable-channel generated doctest runner
remains the recorded ADR 0052 difference. ADR 0017's governed source set is
unchanged.

An enforcing source and behavioral test inventories repository-owned spawn
sites and rejects `sh`, `bash`, `-c`, shell-script wrappers, and indirect shell
helpers. The upstream generated doctest runner is the only recorded exception
and is not repository-owned.

That inventory is generated, committed, and drift-checked at
`bazel/generated/no-shell-inventory.json`. Its governed-source and
declared-input sets are nonempty. Its spawn-site set is exact and may contain
zero entries for an individual governed source. It records:

1. `governedSources` - every repository-owned runner, cleanup, timeout, and
   process-control source subject to this rule, derived from the first-party
   configured-target census, not from a hand-maintained list;
2. `declaredInputs` - the exact declared inputs of the no-shell carrier; and
3. `scanResults` - exactly one successful record for every governed source,
   including a zero-site record when the source has no spawn construct; and
4. `spawnSites` - every discovered process-spawn construct, each naming its
   governed source, span, spawned program expression, and a typed
   `shellInvocation` verdict; any true verdict refuses.

`governedSources` and `declaredInputs` are equal in both directions. Every
`spawnSites[].source` belongs to that set, but the spawn-site source projection
is not required to contain a source with zero sites. `scanResults` has exactly
one successful entry for every governed source and no ungoverned entry. A
fresh scan derives the exact keyed spawn-site set from source path, span, and
spawned-program expression; that set and the committed `spawnSites` set are
equal in both directions. A walk, open, read, or parse failure produces a
failed scan result and refuses rather than shrinking either comparison.

Six plants are mandatory and each must fail at its own diagnostic:

```text
no-shell-inventory-empty
no-shell-inventory-missing-entry
no-shell-inventory-extra-entry
no-shell-inventory-unguarded-spawn
no-shell-inventory-missing-zero-site-record
no-shell-inventory-planted-shell
```

Both the raw `scanResults` record count and the unique scan-source count must
equal the governed-source count. A duplicate record is a refusal even when the
unique projection still matches.

The integrator commits the generated inventory with the rest of
`bazel/generated/`; slices produce `.scratch/` previews only. Its digest and
plant results enter the qualification evidence set.
