# Recovery and Deadline Contract

## Cleanup boundary

Cleanup anchors worktree `.scratch/` once with a close-on-exec directory
descriptor, resolves only `.scratch/bazel/` beneath it without symlink,
magic-link, or escape traversal, and removes entries descriptor-relative.
Both `openat2` and forced component-walk fallback are tested. Every opened
descriptor is close-on-exec. A tracked file, unsafe layout, replacement race,
or live matching server causes refusal before deletion.

Cleanup performs every one of those operations through the same injectable
filesystem trait in `packages/d2b-bazel-support/src/fsops.rs` that the per-case
result writer, the topology provider path, the locator, and the wave-note
policy lint use. Those
subsystems enforce identical properties on
identical syscalls, so a single implementation and a single mutation set cover
them, and no cleanup test needs live host filesystem state: the tracked-file,
symlink, magic-link, escape, replacement-race, decoy-survival, and
descriptor-inheritance negatives are all produced by the injected fake. The
forced component-walk route is selected through the same boundary rather than
by finding a kernel that lacks `openat2`. Cleanup uses the strict resolve
policy, `RESOLVE_BENEATH` with `RESOLVE_NO_SYMLINKS` and
`RESOLVE_NO_MAGICLINKS`, because it operates only on paths the runner created;
the provider open uses `RESOLVE_NO_MAGICLINKS` alone for the reasons
`runner-environment.md` records, and each call site's choice is asserted so
neither inherits the other's. That choice binds the walk route too: on the
forced component-walk route the strict policy carries `O_NOFOLLOW` on the leaf
as well as on every intermediate component, so a symlink planted at the final
name of a cleanup target is refused `ELOOP` whichever route the boundary took.
A route that exempted the leaf would let cleanup follow a link out of
`.scratch/bazel/` on the one route its own negatives force, which is the
failure this whole subsystem exists to refuse. The trait lives in the neutral
support crate rather than in the runner so that the locator can reach it
without depending on the runner, so `packages/d2b-contract-tests` can reach it
as a dev-dependency, and so `xtask` can reach the startup-option
construction beside it without an `xtask -> d2b-bazel-runner` edge.

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

Both the raw uptime field and the current instant arrive through
`packages/d2b-bazel-runner/src/clock.rs`, which declares an `UptimeSource` and
a `Clock`. That boundary stays in the runner rather than moving to the shared
support crate, because the deadline and process paths are its only readers; the
locator resolves provider freshness from timestamps the filesystem boundary
returns from the provider's own descriptor and needs no clock at all. Nothing
in the deadline path opens
`/proc/uptime` or reads the host
clock directly. That is what makes the grammar and rounding table testable:
every accepted and rejected field, the truncate-on-capture and round-up-on-read
pair, the exactly-zero remaining budget, and the overflow case are supplied
values rather than states a test has to wait for or provoke. Expiry-path tests
drive the fake clock past the deadline so the SIGTERM, full-grace, SIGKILL, and
reap ordering below is asserted deterministically, with no sleep and no timing
race on a loaded host.

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
