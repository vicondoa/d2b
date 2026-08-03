# Recovery and Deadline Contract

## Cleanup boundary

Cleanup anchors worktree `.scratch/` once with a close-on-exec directory
descriptor, resolves only `.scratch/bazel/` beneath it without symlink,
magic-link, or escape traversal, and removes entries descriptor-relative.
Both `openat2` and forced component-walk fallback are tested. Every opened
descriptor is close-on-exec. A tracked file, unsafe layout, replacement race,
or live matching server causes refusal before deletion.

| Code | Required recovery |
| --- | --- |
| `D2B-BZLCLEAN-TRACKED` | Run `D2B_CLEAN_DRY_RUN=1 make clean`; remove or relocate the unexpected tracked entry under `.scratch/bazel/`; rerun `make clean`. |
| `D2B-BZLCLEAN-SYMLINK` | Dry-run; remove the offending link under `.scratch/bazel/`; rerun clean. External target remains untouched and needs separate ownership verification. |
| `D2B-BZLCLEAN-ESCAPE` | Same safe steps as symlink/unsafe layout; no external target operation. |
| `D2B-BZLCLEAN-LIVE` | Close other clients; run `make bazel-shutdown`; rerun `make clean`. No dry-run or tree correction first. |
| `D2B-BZLSERVER-STUCK` | Close other clients; run `make bazel-shutdown`; do not delete scratch or signal a PID manually. |

Refusal deletes nothing. Messages never include absolute paths, output hashes,
user/PID values, raw deadlines, opaque handles, recursive-removal commands, or
a remedy belonging to another code.

## Deadline

Promoted CI checkout has `timeout-minutes: 2`. The first action after checkout
reads `/proc/uptime`; the shared checked parser accepts ASCII digits with an
optional nonempty fractional part. Capture truncates to milliseconds and
exports an absolute boot-relative deadline:

```text
deadline_ms = anchor_ms + 780000
```

The Make target reads current uptime with the same parser, rounds it up, and
uses checked subtraction. Missing deadline is the unbounded local default and
is forbidden by promoted workflow policy. Malformed, signed, nonnumeric,
trailing, or overflowing input fails without echo. `None` or zero remaining
means an ordinary expired budget, not malformed input. Child duration rounds
down.

## Process control

Repository-owned Rust code spawns the Bazel client into a new process group
without a shell. On expiry:

1. send SIGTERM to only the child group;
2. wait the fixed grace in full;
3. observe leader state only with `EXITED|NOWAIT|NOHANG`;
4. send unconditional SIGKILL to the group;
5. reap the direct child;
6. request server termination only through a bounded `bazel shutdown` carrying
   byte-identical absolute startup options.

It never signals its own group, group zero, group -1, or a server PID read from
a file. Tests prove call order, full grace after leader exit, surviving
descendant termination, unrelated sibling survival, and stuck shutdown.

A ceiling miss reports measured duration and target and authorizes only a
larger runner or further disjoint slice split. It never suggests weaker
coverage, weaker enforcement, surface removal, or a relaxed ceiling. The cold
continuous-integration ceiling does not become binding until the W3 feasibility
measurement records it as attainable on the real runner class; a feasibility
shortfall takes one of the same two remedies.
