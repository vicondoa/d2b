# Recovery and Deadline Contract

ADR 0054 does not weaken ADR 0052 cleanup or deadline behavior.

## Cleanup

Cleanup anchors `.scratch/` once, resolves only `.scratch/bazel/`, and removes
descriptor-relative. It refuses links, magic links, escapes, tracked files,
replacement races, foreign content, or a live matching server before deleting
anything. Both `openat2` and forced component-walk routes are tested through
the injected filesystem boundary.

| Code | Recovery |
| --- | --- |
| `D2B-BZLCLEAN-TRACKED` | Run `D2B_CLEAN_DRY_RUN=1 make clean`; remove or relocate the unexpected tracked entry from `.scratch/bazel/`; run `make clean`. |
| `D2B-BZLCLEAN-SYMLINK` | Run `D2B_CLEAN_DRY_RUN=1 make clean`; remove the offending symlink or magic link from under `.scratch/bazel/`; run `make clean`. State that the external target is outside managed cleanup, remains untouched, and may be reclaimed only after separate inspection and independent ownership verification. |
| `D2B-BZLCLEAN-ESCAPE` | Run `D2B_CLEAN_DRY_RUN=1 make clean`; remove the escaping layout from under `.scratch/bazel/`; run `make clean`. State that external content is outside managed cleanup, remains untouched, and may be reclaimed only after separate inspection and independent ownership verification. |
| `D2B-BZLCLEAN-LIVE` | Close other Bazel clients running against this worktree; run `make bazel-shutdown`; run `make clean`. Do not dry-run, inspect, or correct `.scratch/bazel/` first. |
| `D2B-BZLSERVER-STUCK` | Close other Bazel clients running against this worktree; run `make bazel-shutdown`. Do not delete `.scratch/bazel/` or signal any process identifier by hand. |

Messages contain no `$!`, absolute or Nix store path, user or process
identifier, raw deadline, raw pagination cursor, opaque handle,
recursive-removal command, or another code's remedy. When an artifact must be
identified, the message carries only its repository-relative contract row and
SHA-256.
They do not instruct the operator to replace a refused entry with a directory.
Table-driven tests assert the exact command tokens and ordered steps for each
code. Redaction, omitted-remedy, borrowed-remedy, external-target removal,
replacement-directory, recursive-removal, manual-signal, and
ceiling-remedy-substitution mutations each fail their own row.

The workspace drift and contributor-mutation refusals are the separate
ADR-0054-adjusted table in `workspace-and-tool-pinning.md`. It covers stale
product and walker hub locks, module lock drift, generator drift,
package-policy drift, yanked snapshot drift, ambient repin controls, and
unexpected tracked mutation. Each row has an exact nonzero message with the
ordered `nix develop`, `cd packages`, exact command, review/commit, and rerun
sequence. The same redaction and exact-message harness rejects a missing step,
wrong command, borrowed remedy, absolute path, secret, identifier, or echoed
ambient value. The retired-hub diagnostic bytes are unchanged and remain
outside that table.

Provider and evidence failures use the exact tables in
`runner-environment.md`. Provider rows name the stable declared
runfiles-relative key, corrective action, and owning slice rerun.
`D2B-BZLEVIDENCE-SANITIZE`, `D2B-BZLEVIDENCE-LIMIT`,
`D2B-BZLEVIDENCE-RETENTION`, `D2B-BZLEVIDENCE-PUBLISH`, and
`D2B-BZLEVIDENCE-NO-VERDICT` name their stable repository-relative policy row,
carrier, or workflow plus the exact literal correction and rerun command.
The provider matrix renders each reason for each of the four closed slice
commands; a generic `owning slice` placeholder is not accepted. Qualification
failures use the fixed-code command table in
`shadow-promotion-evidence.md`. Release-containment failures use the
fixed-code command table in `make-target-compatibility.md`. Patched-Bazel
identity/capability, strategy, inherited-capability, `no_new_privs`, filter,
and action-exec stages use the complete table in `coverage-map.md`. Query errors are typed degraded outcomes, never
absence. All tables emit no planted forbidden value and do not rewrite the
underlying `testVerdict`; evidence failures produce a structurally valid typed
degraded status that completion and qualification reject.

All slice reruns use the versioned closed diagnostic command enum in
`make-target-compatibility.md`. Before alias removal every message names an
existing shadow target. Alias removal atomically updates provider,
sandbox-policy, qualification threshold/table, evidence/publication, cleanup,
and recovery renderers, both module roots, every byte-exact expectation, all
governed docs, the evidence record, and the semantic fragment to the enduring
promoted aggregate/slice targets. A diagnostic, task-state label, or evidence
variant naming a target absent from that state is a policy failure. Version 1
is retained only in the closed pre-change fixture where every shadow rule
exists.

### Execution containment recovery

Every Rust-parent, C-helper, child-setup, and patched-sandbox cleanup failure
has one public code. Its diagnostic names only the code, closed stage, closed
slice, command version, and fixed repository-relative input below. The remedy
is the literal correction in the same row. The rerun is the literal cell
selected from the command-version table. There is no generic `owning slice`
text, free-form command, dynamic path, numeric PID/PGID, errno text, or raw
transport byte.

| Command version | Phase where valid | `main` | `api` | `broker` | `aux` |
| --- | --- | --- | --- | --- | --- |
| `bazel-diagnostic-v1` | Shadow targets exist and aliases have not been removed | `make test-bazel-rust-main` | `make test-bazel-rust-api` | `make test-bazel-rust-broker` | `make test-bazel-rust-aux` |
| `bazel-diagnostic-v2` | Alias removal has landed | `make test-rust-slice-main` | `make test-rust-slice-api` | `make test-rust-slice-broker` | `make test-rust-slice-aux` |

The renderer accepts the eight cells above as a closed enum. It refuses
`bazel-diagnostic-v1` after alias removal, `bazel-diagnostic-v2` before alias
removal, an unknown slice, or a command that is not present in that repository
state. The
alias-removal commit changes the selected version and byte-exact expectations
atomically; it does not change a code, input, or correction.

Every parent code names the fixed input
`specs/003-adr052-bazel-rust/contracts/runner-environment.md#rust-parent-stage-owner-and-closure-contract`.

| Internal stage | Public code | Exact correction |
| --- | --- | --- |
| `PARENT_PREPARE` | `D2B-BZLEXEC-PARENT-PREPARE` | Correct status-channel construction and private-fd mapping in `packages/d2b-bazel-exec/src/execute.rs`. |
| `PARENT_SIGNAL_HANDOFF` | `D2B-BZLEXEC-PARENT-SIGNAL-HANDOFF` | Under the one process-wide launch guard, use the reviewed safe `nix::sys::signal::SigSet` API to capture and block the full managed set before spawn, attempt exact spawning-thread mask restoration after every spawn result before unlock, and fail closed on capture, block, poisoned-guard, or restoration failure; add no Rust unsafe or disposition mutation. |
| `PARENT_SPAWN` | `D2B-BZLEXEC-PARENT-SPAWN` | Correct the exact immutable-helper spawn in `packages/d2b-bazel-exec/src/execute.rs`; do not add a path fallback. |
| `PARENT_CLOSE` | `D2B-BZLEXEC-PARENT-CLOSE` | Correct single-owner post-spawn descriptor closure in `packages/d2b-bazel-exec/src/execute.rs`. |
| `PARENT_READY` | `D2B-BZLEXEC-PARENT-READY` | Correct the bounded stateful framed `READY` decoder in `packages/d2b-bazel-exec/src/execute.rs`. |
| `PARENT_EXECUTED` | `D2B-BZLEXEC-PARENT-EXECUTED` | Correct retained framed `EXECUTED` decoding after `READY` without inferring exec from process status. |
| `PARENT_TERMINAL` | `D2B-BZLEXEC-PARENT-TERMINAL` | Correct framed terminal decoding, trailing-byte refusal, and EOF drain in `packages/d2b-bazel-exec/src/execute.rs`. |
| `PARENT_WAIT` | `D2B-BZLEXEC-PARENT-WAIT` | Correct supervisor wait ownership and return the Bazel action nonzero on failure; do not signal a numeric PID or PGID. |
| `PARENT_STATUS` | `D2B-BZLEXEC-PARENT-STATUS` | Correct terminal-record and supervisor-status equality checking. |
| `PARENT_CLEANUP` | `D2B-BZLEXEC-PARENT-CLEANUP` | Correct idempotent owned-fd closure and preserve the first parent failure. |

Every helper code names the fixed input
`tests/tools/d2b-bazel-exec-supervisor/supervisor.c` and the contract row
`specs/003-adr052-bazel-rust/contracts/runner-environment.md#c-supervisor-stage-error-owner-and-closure-contract`.

| Internal stage | Public code | Exact correction |
| --- | --- | --- |
| `HELPER_SIGNAL_INHERITED_IGNORED` | `D2B-BZLEXEC-HELPER-SIGNAL-INHERITED-IGNORED` | Restore a non-ignored disposition for every managed signal in the launching environment, then rerun; the helper must inspect first and fail before fork rather than reset and continue. |
| `HELPER_SIGNAL_HANDOFF` | `D2B-BZLEXEC-HELPER-SIGNAL-HANDOFF` | Correct the typed Rust launch handoff so the helper inherits the complete managed set blocked before its first setup operation. |
| `HELPER_ADOPT` | `D2B-BZLEXEC-HELPER-ADOPT` | Correct private-fd, status-fd, argv, environment, and stdio adoption before fork. |
| `HELPER_SIGNAL_NORMALIZE` | `D2B-BZLEXEC-HELPER-SIGNAL-NORMALIZE` | After refusing any inherited managed `SIG_IGN`, install dispositions and synchronous consumption while the inherited managed set remains blocked, establish the final mask, preserve pending termination, ignore `SIGPIPE`, and restore waitable default `SIGCHLD` without `SA_NOCLDWAIT`. |
| `HELPER_EXEC_PIPE` | `D2B-BZLEXEC-HELPER-EXEC-PIPE` | Correct creation and ownership of the single nonblocking close-on-exec exec-error pipe; the kernel ptrace stop is the only release barrier. |
| `HELPER_FORK` | `D2B-BZLEXEC-HELPER-FORK` | Correct the sole supervisor fork and leave no child on a reported fork failure. |
| `HELPER_GROUP_ESRCH` | `D2B-BZLEXEC-HELPER-GROUP-ESRCH` | Correct the parent-and-child `setpgid` handshake; an absent child or group must fail before `READY` with direct-child cleanup. |
| `HELPER_GROUP_EPERM` | `D2B-BZLEXEC-HELPER-GROUP-EPERM` | Correct the parent-and-child `setpgid` handshake without changing session or group authority; `EPERM` must fail before `READY` with direct-child cleanup. |
| `HELPER_GROUP_ERROR` | `D2B-BZLEXEC-HELPER-GROUP-ERROR` | Correct the parent-and-child `setpgid` handshake; any other setpgid error or confirmed-group mismatch must fail before `READY` with direct-child cleanup and no raw errno text. |
| `HELPER_GROUP_EARLY_EXIT` | `D2B-BZLEXEC-HELPER-GROUP-EARLY-EXIT` | Reject child exit before the expected initial trace stop and `READY`; consume-reap it and close every owned descriptor. |
| `HELPER_PTRACE_POLICY` | `D2B-BZLEXEC-HELPER-PTRACE-POLICY` | Run on Linux 3.19 or newer on a supported native system, require unprivileged Yama parent-child mode 0 or 1 when present, and retain only the four-request ptrace seccomp allowance without granting `CAP_SYS_PTRACE` or weakening action no-network. |
| `HELPER_PTRACE_STOP` | `D2B-BZLEXEC-HELPER-PTRACE-STOP` | Correct the child `PTRACE_TRACEME` plus initial `SIGSTOP` barrier and accept no other initial wait state before `READY`. |
| `HELPER_PTRACE_OPTIONS` | `D2B-BZLEXEC-HELPER-PTRACE-OPTIONS` | Install exactly `PTRACE_O_TRACEEXEC` on the stopped direct child before emitting `READY`; do not infer tracing state from the stop alone. |
| `HELPER_PTRACE_CONT` | `D2B-BZLEXEC-HELPER-PTRACE-CONT` | Release the confirmed initial stop exactly once with zero-signal `PTRACE_CONT` after the complete `READY` write. |
| `HELPER_PRE_EXEC_TERMINATION` | `D2B-BZLEXEC-HELPER-PRE-EXEC-TERMINATION` | Before the kernel exec event, coalesce any managed signal into one pre-exec setup termination, suppress `EXECUTED` and target terminal/audit publication even on empty exec-pipe EOF, immediately kill and consume-reap the confirmed child group without forwarding or grace, and let sandbox containment backstop incomplete cleanup. |
| `HELPER_PRE_EXEC_DEATH` | `D2B-BZLEXEC-HELPER-PRE-EXEC-DEATH` | Treat every normal exit, `SIGKILL`, OOM-like kill, or other child death before the kernel exec event as setup failure; consume-reap and never publish execution. |
| `HELPER_PTRACE_EVENT` | `D2B-BZLEXEC-HELPER-PTRACE-EVENT` | Accept only `WIFSTOPPED`, `SIGTRAP`, and exact `PTRACE_EVENT_EXEC` after options and release; empty EOF, `SIGSYS`, faults, plain `SIGTRAP`, missing/wrong events, and other stops must fail closed. |
| `HELPER_PTRACE_DETACH` | `D2B-BZLEXEC-HELPER-PTRACE-DETACH` | At the exact exec-event stop, detach exactly once with signal zero; on failure suppress `EXECUTED`, kill and consume-reap the group, and leave no live trace relationship. |
| `HELPER_EXEC_TIMEOUT` | `D2B-BZLEXEC-HELPER-EXEC-TIMEOUT` | Correct absolute-deadline accounting without resetting time after retry or short I/O. |
| `HELPER_EXEC_PARTIAL` | `D2B-BZLEXEC-HELPER-EXEC-PARTIAL` | Correct exact record cursors so EOF after any byte is partial. |
| `HELPER_EXEC_OVERLONG` | `D2B-BZLEXEC-HELPER-EXEC-OVERLONG` | Correct the one-record-plus-one-byte overlong check. |
| `HELPER_EXEC_UNKNOWN` | `D2B-BZLEXEC-HELPER-EXEC-UNKNOWN` | Correct the closed single-record exec-error decoder and distinct framed status decoder. |
| `HELPER_EXEC_EPIPE` | `D2B-BZLEXEC-HELPER-EXEC-EPIPE` | Keep `SIGPIPE` ignored and map a closed status reader to typed `EPIPE`. |
| `HELPER_EXEC_IO` | `D2B-BZLEXEC-HELPER-EXEC-IO` | Correct bounded `EINTR`, `EAGAIN`, short-read, and short-write handling without rendering OS text. |
| `HELPER_SIGNAL_FORWARD` | `D2B-BZLEXEC-HELPER-SIGNAL-FORWARD` | Correct the four-signal allowlist and target-group forwarding in the supervisor. |
| `HELPER_DEADLINE` | `D2B-BZLEXEC-HELPER-DEADLINE` | Correct TERM, full fixed grace, unconditional KILL, and direct-child reap for deadline or external `SIGTERM`, including no-deadline cases. |
| `HELPER_WAIT` | `D2B-BZLEXEC-HELPER-WAIT` | Correct wait ownership without `SA_NOCLDWAIT` and retain exact child status. |
| `HELPER_REAP` | `D2B-BZLEXEC-HELPER-REAP` | Correct direct-child reap after escalation and before terminal publication. |
| `HELPER_TERMINAL_WRITE` | `D2B-BZLEXEC-HELPER-TERMINAL-WRITE` | Correct the bounded exact terminal write and typed closed-reader handling. |
| `HELPER_STATUS_MIRROR` | `D2B-BZLEXEC-HELPER-STATUS-MIRROR` | Correct exact normal-exit or terminating-signal mirroring. |
| `HELPER_CLEANUP` | `D2B-BZLEXEC-HELPER-CLEANUP` | Correct idempotent target-group, child, and descriptor cleanup while preserving the first helper failure. |

Every child code names the same fixed supervisor source and the contract row
`specs/003-adr052-bazel-rust/contracts/runner-environment.md#c-supervisor-stage-error-owner-and-closure-contract`.

| Internal stage | Public code | Exact correction |
| --- | --- | --- |
| `CHILD_GROUP` | `D2B-BZLEXEC-CHILD-GROUP` | Correct the child's `setpgid(0, 0)` half of the target-group handshake before the initial trace stop. |
| `CHILD_SIGNAL` | `D2B-BZLEXEC-CHILD-SIGNAL` | Restore the empty child mask and default disposition for every catchable signal before raising the initial trace stop. |
| `CHILD_STDIO` | `D2B-BZLEXEC-CHILD-STDIO` | Correct exact installation of declared stdin, stdout, and stderr at fds 0, 1, and 2. |
| `CHILD_CLOEXEC` | `D2B-BZLEXEC-CHILD-CLOEXEC` | Set close-on-exec on the private executable fd and every non-surviving descriptor. |
| `CHILD_CLOSE` | `D2B-BZLEXEC-CHILD-CLOSE` | Correct closure of every supervisor-only and non-surviving child descriptor. |
| `CHILD_PTRACE` | `D2B-BZLEXEC-CHILD-PTRACE` | Correct unprivileged parent-child `PTRACE_TRACEME`; do not add attach, capability, or sibling tracing. |
| `CHILD_STOP` | `D2B-BZLEXEC-CHILD-STOP` | Raise exactly one initial `SIGSTOP` after all fallible child setup and before the sole `execveat`. |
| `CHILD_EXECVEAT` | `D2B-BZLEXEC-CHILD-EXECVEAT` | Correct same-open-file-description `execveat(AT_EMPTY_PATH)`; do not add a reopen or path fallback. |

Every sandbox code names the fixed inputs
`pkgs/bazel-8.6.0-seccomp/linux-sandbox-seccomp.patch` and
`specs/003-adr052-bazel-rust/contracts/runner-environment.md#patched-sandbox-stage-error-owner-and-closure-contract`.

| Internal stage | Public code | Exact correction |
| --- | --- | --- |
| `SANDBOX_NAMESPACE` | `D2B-BZLEXEC-SANDBOX-NAMESPACE` | Restore one fresh `CLONE_NEWPID` namespace for every governed action and refuse every fallback strategy. |
| `SANDBOX_MONITOR` | `D2B-BZLEXEC-SANDBOX-MONITOR` | Restore namespace PID 1 as the action's adoption, abnormal-teardown, and reap owner. |
| `SANDBOX_KILL` | `D2B-BZLEXEC-SANDBOX-KILL` | Correct namespace-local kill of every member other than PID 1; do not signal a host PID or PGID. |
| `SANDBOX_REAP` | `D2B-BZLEXEC-SANDBOX-REAP` | Correct nonblocking adopted-child reap progress and require a consuming wait before recording any PID-1 reap. |
| `SANDBOX_CEILING` | `D2B-BZLEXEC-SANDBOX-CEILING` | Restore the single fixed 10,000 ms userspace TERM/KILL/monitor escalation and close-or-quarantine ceiling; do not use it as a kernel cleanup bound. |
| `SANDBOX_PENDING_KERNEL_CLEANUP` | `D2B-BZLEXEC-SANDBOX-PENDING-KERNEL-CLEANUP` | Keep the action failed and quarantined under the original live monitor as sole wait owner; drain new admission without terminating it; do not reboot, retry, or release manually; follow `docs/contributing/critical-subsystems.md#bazel-pending-kernel-cleanup-quarantine` and rerun only after that monitor publishes and the operator confirms consuming-reap release. |
| `SANDBOX_CLEANUP` | `D2B-BZLEXEC-SANDBOX-CLEANUP` | Correct consuming PID-1 reap and owned quarantine release while preserving the first sandbox failure; never convert a quarantined action to success. |

Successful release from quarantine is not a recovery error code. Only the
original live monitor may publish the fixed
`D2B-BZLEXEC-SANDBOX-CONSUMING-REAP-RELEASE` record with
`cleanup=complete-after-quarantine` and
`quarantine=entered-and-released-after-consuming-reap`.

T067 owns table-driven byte-exact tests for every parent, helper, and child
public code crossed with all four slices and both closed command versions.
T068 implements only that runner-owned closed mapping and makes T067 pass.
Each case asserts nonzero status, empty stdout, fixed safe input, exact
correction, exact phase-valid rerun command, and absence of forbidden values.
T067 also resolves every governed fixed artifact locator from the repository
root: a path must name a regular governed file, and a Markdown locator must
name that file plus a heading whose normalized anchor exists exactly once.
Isolated plants cover omitted input, omitted correction, omitted rerun, wrong
command version, command absent in phase, wrong slice, borrowed
parent/helper/child remedy, wrong code, unresolved path, missing/duplicate
anchor, numeric PID/PGID, descriptor, absolute/runfiles/store path, errno/OS
text, raw protocol bytes, argv/environment, and dynamic identifier.

The patched `linux-sandbox` owns `SANDBOX_*` mapping and rendering because it
exists before action setup and remains the wait owner through normal cleanup
or `pending-kernel-cleanup`. Sequential T120 owns the byte-exact sandbox
cross-product tests in its exact toolchain files. Those tests apply the same
repository-root path and unique-anchor resolver to both fixed sandbox inputs,
and to
`docs/contributing/critical-subsystems.md#bazel-pending-kernel-cleanup-quarantine`,
cover every sandbox code across all slices and both command versions, and
plant omitted/wrong/borrowed remedies, unresolved locators, missing runbook or
anchor, wrong runbook link, reboot, retry-before-release, manual release,
replacement waiter, pending-state removal, false-reaped reporting,
success-after-quarantine, and reuse while quarantined. Pending and
consuming-reap release diagnostics are byte-exact. No runner recovery file
renders a sandbox code.

Together the T067 runner harness and T120 live sandbox harness cover every
execution parent/helper/child/sandbox code, sandbox-policy stage and slice,
qualification
query/refusal/publication class, and release query/refusal class. It rejects
missing or wrong remedies and command versions, descriptor numbers, absolute,
runfiles, socket, and Nix store paths, OS/errno text, raw
child/tool/API/protocol output, argv, environment values, numeric PID/PGID,
and process, user, run, attempt, candidate, object, or tag identifiers.
Repository-relative policy rows and SHA-256 digests are the only artifact
locators permitted.

## Deadline

Promoted checkout has a two-minute bound. The first post-checkout action reads
uptime through the shared checked parser and exports:

```text
deadline_ms = anchor_ms + 780000
```

Capture truncates milliseconds, read rounds up, and child timeout rounds down.
Missing is the unbounded local default and is forbidden in promoted jobs.
Malformed or overflowing input refuses without echo. Zero or underflow is an
ordinary expired budget.

Time is injected. Tests do not sleep or manipulate the host clock.

## Process control

On expiry:

1. spawn the client in a dedicated process group without a shell;
2. signal SIGTERM to that group;
3. independently time the full fixed grace;
4. throughout that grace, repeatedly observe the direct child with
   `waitid(EXITED|NOWAIT|NOHANG)`;
5. treat every observation as informational only: no observation consumes
   status, blocks the grace clock, shortens the grace, authorizes an early
   return, or triggers a reap;
6. when the grace expires, send unconditional group SIGKILL even if the leader
   has exited;
7. only after the group SIGKILL, reap the direct child;
8. request server shutdown only through bounded `bazel shutdown` with matching
   startup options.

The wrapper never signals its own group, group zero, group -1, an unrelated
process, or a server PID read from a file.

The process backend records the exact order and clock progress. Tests require
more than one non-consuming poll when the grace spans more than one poll
interval, a poll that reports no state while the leader runs, a repeated poll
that reports the same exited status, full grace after an immediate leader exit,
unconditional SIGKILL, and final direct-child reap. A blocking-wait mutation
that removes `NOHANG` and an early-reap mutation that consumes or reaps the
leader before SIGKILL must fail the order test. Separate mutations that
shorten grace on observed exit or make SIGKILL conditional must also fail.
The spawn path has a planted missing-`process_group(0)` variant. Separate
variants target the wrapper's group, group zero, group -1, and a PID-file
decoy. Every variant fails while a sibling process and the decoy process
remain alive, proving the wrapper signals only the dedicated child group and
never trusts a PID read from a file.

A ceiling miss authorizes only a larger runner class or a further disjoint
slice split.
