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
fixed-code command table in `make-target-compatibility.md`. They emit no
planted forbidden value and do not rewrite the underlying `testVerdict`; they
produce a structurally valid typed degraded status that completion and
qualification reject.

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
