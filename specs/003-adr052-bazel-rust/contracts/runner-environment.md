# Runner Environment and Per-Case Evidence Contract

One Bazel test action per carrier means the build event stream and the
per-target result document carry one result per target. Per-case attribution
therefore has to come from what the repository-owned runner publishes, not from
stdout. This contract is what makes "a single failing case names itself"
mechanically true.

## Child environment

- Each child environment derives from the Bazel test environment. Only the
  declared test environment is forwarded; the wrapper's incidental host
  environment is not.
- Each case receives its own directory beneath `TEST_TMPDIR`. That is what
  makes per-case process freshness equivalent to the current per-case
  isolation, which gives each case its own temporary handling.
- The test binary is resolved through runfiles and then opened once, and the
  descriptor that verification examined is the descriptor that runs. Nothing is
  resolved by an absolute execution-root path, which is not a declared input
  and does not survive a different sandbox, and nothing is re-resolved by name
  at spawn time. See "Binary provider resolution and execution" below.
- Concurrency stays bounded by the same `D2B_RUST_BUDGET`-derived control the
  gate already uses. Per-target concurrency never multiplies `--local_test_jobs`
  into an unbounded process count.

## Binary provider resolution and execution

The locator's two arms and the runner's child-binary resolution all reach a
provider the same way, and the way is a **single open**. `RunfilesView` decides
which provider; `FileSystem` opens it once, verifies the open descriptor, and
executes that same descriptor. No provider is named by path twice, and nothing
in this design stats a path and then hands the path to a spawner.

That ordering is the whole point. Checking a path and then executing the path
is two resolutions of one name, and between them the name can be rebound. This
is not a theoretical gap here: `packages/target/` holds real, executable,
out-of-date binaries for the entire shadow stage, and a concurrent Cargo build
replaces entries in it by rename while the gate is running. Measured directly:
after the provider path is replaced by a different executable, executing a
retained descriptor still runs the original verified bytes, while a freshly
path-opened descriptor runs the replacement. The check-then-spawn-by-path shape
therefore verifies one file and runs another, silently, and exits zero.

### The single-open rule

- `RunfilesView` in `packages/d2b-bazel-support/src/runfiles.rs` yields a
  runfiles-root anchor and one declared relative path. The supplied states are
  a declared entry present, a declared entry missing, and a runfiles
  environment that indicates no Bazel test at all. Mode is chosen once from
  that last state; a missing entry in Bazel mode is a hard failure that names
  the declared runfiles-relative path and never falls back to the Cargo arm.
  The Cargo
  arm supplies the same anchor-and-relative pair by splitting the
  `CARGO_BIN_EXE_<name>` value the call-site macro expands, so both arms enter
  the filesystem boundary through one signature.
- `FileSystem::open_provider(anchor, relative)` in
  `packages/d2b-bazel-support/src/fsops.rs` performs exactly one
  `openat2(anchor_fd, relative, O_RDONLY|O_CLOEXEC, RESOLVE_NO_MAGICLINKS)`
  and returns a `ProviderHandle`. `O_RDONLY` and not `O_PATH`, because identity
  is a digest of the provider's bytes and an `O_PATH` descriptor cannot be
  read. Measured: `pread` on an `O_PATH` descriptor returns `EBADF` while
  `execveat` on it succeeds, so choosing `O_PATH` would force a second open to
  compute the digest and reintroduce the gap this rule closes.
- The resolve policy is `RESOLVE_NO_MAGICLINKS`, deliberately not
  `RESOLVE_NO_SYMLINKS` and not `RESOLVE_BENEATH`. A Bazel runfiles tree is a
  symlink forest whose links point into the output base, outside the runfiles
  root. Measured against exactly that shape: `RESOLVE_BENEATH` fails `EXDEV`
  and `RESOLVE_NO_SYMLINKS` fails `ELOOP`, so either would refuse every real
  runfiles provider. Link refusal is not what protects a provider; handle
  identity is, because whatever the link resolved to at open time is the only
  thing ever measured or executed. Magic links stay refused so a
  `/proc/<pid>/fd/<n>` entry cannot be laundered into a provider; measured,
  `RESOLVE_NO_MAGICLINKS` refuses that path with `ELOOP` where a plain open
  succeeds.
- A declared relative path that is absolute, empty, or carries a `..` component
  is refused before the open. The anchored form is what keeps an absolute
  execution-root path out of the design.
- Where `openat2` is unavailable the boundary takes its forced component-walk
  route, which is the same route cleanup and the result writer already
  exercise, and the resolve policy still decides what that route accepts:
  `O_NOFOLLOW` on every intermediate component under both policies, and
  `O_NOFOLLOW` on the **final** component only under the strict policy. A
  provider open therefore reaches the same leaf on either route, and a strict
  caller gets the same leaf refusal on either route. "The resolve policy on
  both routes" below states that rule once, for every call site.

### Verification binds to the descriptor, never to the path

Every check runs against the open descriptor, through the same boundary:

- `fstat` on the handle. A provider that is not a regular file is refused.
- Executable mode from that same `fstat`. This is the early, well-named
  refusal, not the only one: measured, `execveat` on a mode `0644` regular file
  and on a directory descriptor both return `EACCES`, so the kernel remains the
  authoritative permission decision and its errno is mapped to the same reason.
- Freshness compares the handle's `st_mtim` against the newest declared input's
  `st_mtim`, each declared input opened once and `fstat`ed on its own handle
  through the same boundary.
- Identity is the digest of the provider's bytes, read with `pread` from offset
  zero to `st_size` on the same handle, compared against the value the coverage
  map records for that provider. A short or over-long read is a refusal.
- The handle is `fstat`ed again immediately after the digest read, and
  `st_dev`, `st_ino`, `st_size`, `st_mtim`, and `st_ctim` must equal the
  pre-read values. This closes the one mutation an open descriptor does not by
  itself exclude, an in-place rewrite of the same inode. Measured: writing
  eight bytes into an already-open regular file changes the bytes a later
  `pread` on that descriptor returns and moves `st_mtim` while `st_ino` is
  unchanged.
- Verification consumes the `ProviderHandle` and returns a
  `VerifiedExecutable`. That type has no public constructor, no conversion from
  a path, and no accessor that yields a path, so a caller cannot hold one
  without having passed every check on the descriptor it wraps, and cannot
  recover a path from one in order to spawn by name. A compile-level test
  asserts both.

### Execution is the same descriptor

`FileSystem::spawn_verified` is the only execution route for a first-party
provider in this design. It takes a `VerifiedExecutable` and executes it with
`execveat(fd, "", argv, envp, AT_EMPTY_PATH)`.

- No `std::process::Command`, no `fexecve`, and no `/proc/self/fd/<n>` path.
  glibc's `fexecve` falls back to `/proc/self/fd/<n>` when `execveat` is
  unavailable, and that fallback is a reopen by path, which is the exact route
  this rule removes. If `execveat` returns `ENOSYS` the runner refuses and
  names the kernel requirement rather than taking any fallback.
- The exec operation lives on the same trait as the open and the checks, not on
  a second injectable boundary. Two boundaries would let a composition satisfy
  both fakes while still executing by path; one boundary makes "hold a verified
  handle" and "reach an execution route" the same reachability question.
- Measured: `execveat` with `AT_EMPTY_PATH` on an `O_RDONLY|O_CLOEXEC`
  descriptor succeeds, and that descriptor is **absent** from the child's
  descriptor table, because the exec image is resolved before close-on-exec
  descriptors are flushed. The same descriptor executes repeatedly, which is
  what preserves process-per-case: one open and one digest per provider per
  carrier invocation, one fresh `fork` and `execveat` per case, and every case
  runs the bytes that were digested once.
- Measured control from the same run: a descriptor opened without `O_CLOEXEC`
  **is** present in the child. Close-on-exec is therefore asserted, not
  assumed.
- Measured limitation, recorded rather than worked around: a `#!` script
  executed from a close-on-exec descriptor fails `ENOENT`, because the
  interpreter reopens the descriptor by its `/proc` path after the flush.
  First-party providers are compiled binaries, so this never binds. A provider
  whose exec returns `ENOENT` from a valid descriptor is refused with that
  reason named, and the `rules_rust`-generated stable-channel doctest runner
  stays outside this path entirely because Bazel executes it, not this runner.

### Descriptor and child ownership

- Every descriptor this path opens is close-on-exec: the runfiles-root anchor,
  the provider handle, the per-case directory descriptor, and the errno pipe.
- The parent owns the provider handle for the whole carrier invocation and
  closes it through the boundary after the last child that used it is reaped,
  which is still before any output descriptor is opened.
- Between `fork` and `execveat` the child does only async-signal-safe work:
  `setpgid(0, 0)` into the dedicated group the deadline path already requires,
  `dup2` of the three stdio descriptors the parent prepared, `fchdir` into the
  per-case directory descriptor, and `execveat`. It opens no path, allocates
  nothing, and takes no lock; `argv` and `envp` are built in the parent as
  NUL-terminated arrays.
- On `execveat` failure the child writes the raw errno to a close-on-exec pipe
  and `_exit`s. The parent maps that errno to a named refusal rather than
  reporting a bare nonzero child exit.
- The child inherits the three stdio descriptors and nothing else.

### Provider refusal reasons

This rule binds **first-party providers**: the binaries this repository builds
and this gate executes. The pinned `bazel` client that the Make wrapper,
`cargo xtask bazel-repin`, and `cargo xtask bazel-module-refresh` spawn is a
dev-shell tool resolved by the wrapper, not a first-party provider; it keeps
its ordinary `Command` construction, which is also the one site permitted to
set the scoped repin child environment.

| Reason | Named remediation |
| --- | --- |
| Runfiles entry missing in Bazel mode | Declare the binary as `data` on the test target; the declared runfiles-relative path is named. |
| Provider is not a regular file | Correct the `data` declaration for the target. |
| Provider is not executable | Rebuild the target; the mode is reported and the path is not. |
| Provider older than its newest declared input | Rebuild the target. |
| Provider digest differs from the coverage map | Rebuild the target, then regenerate the coverage map. |
| Handle metadata changed across the digest read | Rerun; a writer modified the provider in place. |
| `execveat` returned `ENOSYS` | The gate requires a kernel providing `execveat`; no path fallback is taken. |
| `execveat` returned `EACCES`, `ENOEXEC`, `ENOENT`, or `ETXTBSY` | Rebuild the target; the errno is named and no path is printed. |

No reason string carries an absolute path, the runfiles root, a resolved
absolute runfiles location, an environment value, or a descriptor number, per
FR-029.

Two path-shaped strings are deliberately not the same thing, and the split
resolves what would otherwise read as a contradiction between the row above and
the rule beside it. The **declared runfiles-relative path** is repository
content: it is the string the target's own `data` declaration produces, it is
byte-identical on every machine and in every sandbox, and it is the subject of
the remedy, so a refusal that omits it tells a contributor to declare something
without saying what. The **runfiles root** and any **resolved absolute runfiles
location** are local values in the same sense a home directory is, and they are
forbidden here, in the per-case result document, and in a wave note alike. A
refusal that names `_main/packages/d2b-bazel-runner/d2b-exec-probe` is
actionable and carries nothing local; one that names the directory that path
was resolved beneath has published a worktree location into CI output, panel
comments, and PR bodies. Every reference in this feature's artifacts uses
"declared runfiles-relative path" for the first and "runfiles root" or
"resolved absolute runfiles location" for the second, and neither name is used
for the other.

### Hermetic provider tests

**No provider test writes an executable to a live path, and no provider check
executes a live path it did not first verify.** Every state below is a state of
the `FileSystem` fake in `packages/d2b-bazel-support/src/fsops.rs` or the
`RunfilesView` fake beside it. The fake models inodes rather than paths:
`open_provider` resolves a relative path to an in-memory inode record once and
returns a handle bound to that record, and `stat_handle`, `read_handle`, and
`spawn_verified` all read the record the handle names. A path rebound after the
open is therefore representable, and its effect is observable.

Supplied states: an absent entry; a non-regular provider; a non-executable
mode; a modification time older than the newest declared input; bytes whose
digest differs from the recorded identity; **a path rebound to a different
inode after `open_provider` returned**; metadata that changes between the two
`fstat` calls around the digest read; a short read; the forced component-walk
route under each of the two resolve policies; a leaf that is a symlink to a
regular file outside the anchor; an intermediate component that is a symlink;
and `spawn_verified` returning `ENOSYS`, `EACCES`, `ENOEXEC`, `ENOENT`, and
`ETXTBSY`.

Three cases bind the walk route to its policy parameter, because the route is
where an earlier draft of this contract silently exempted the leaf. Under the
provider policy the walk route opens the leaf symlink and yields the same inode
the `openat2` route yields, **and every identity check still runs on that
handle**: kind, mode, freshness, the digest compared against the coverage map,
and the bracketing `fstat`. Under the strict policy the same leaf symlink is
refused `ELOOP` and nothing is read. And an intermediate symlink is refused
under both policies, with `ENOTDIR` from the `O_DIRECTORY` open rather than
`ELOOP`.

Planted mutations each test must reject: re-resolving the declared path at exec
time instead of executing the handle; computing the digest from a second open;
falling back to `/proc/self/fd/<n>` or to `std::process::Command` when
`execveat` is unavailable; clearing `O_CLOEXEC` on the provider handle;
dropping the post-read `fstat` comparison; closing the handle and reopening it
before the last child is spawned; reporting a `spawn_verified` errno as a
generic nonzero child exit; **exempting the leaf from `O_NOFOLLOW` on the walk
route regardless of policy**, which the strict case must fail; **applying
`O_NOFOLLOW` to the leaf on the walk route regardless of policy**, which the
provider case must fail; and **skipping the digest or the bracketing `fstat`
when the leaf resolved through a symlink**, which is the shortcut that would
turn the provider policy's accepted leaf into an unverified one.

The stale-provider case in particular stays a supplied state: the fake reports
an out-of-date, wrong-digest executable at the Cargo path while the fake
runfiles view reports no entry. Writing a real stale binary into
`packages/target/` would manufacture, on the shared host, precisely the hazard
the locator exists to refuse, and an interrupted run would leave it there for
the next suite to find. This is the same rule that keeps `ENOSPC` and `EINTR`
off the real disk, applied to the one directory the shadow stage keeps full of
real, out-of-date binaries.

### The one deliberately non-hermetic test

Every claim above about `execveat` is a claim about the kernel, not about
repository logic, and a fake cannot prove a kernel. One test therefore drives
the host-backed implementation:
`packages/d2b-bazel-runner/tests/exec_handle.rs` opens a provider handle on
`packages/d2b-bazel-runner/src/bin/d2b-exec-probe.rs`, a first-party probe
binary that prints its own descriptor table with the device and inode each
descriptor names, and asserts that the exec succeeded, that the provider inode
appears in no child descriptor, that a second exec of the same handle succeeds,
and that a deliberately non-close-on-exec control descriptor does appear. It
arranges nothing: the probe is an ordinary declared input built by the same
graph as every other first-party binary, and the child inspects itself rather
than the parent inspecting the child. A design this size does not get to rest
on an unmeasured claim about a syscall.

## Per-case result document

- One JUnit document is written to the path Bazel supplies in
  `XML_OUTPUT_FILE`, with one case element per enumerated case.
- Outcomes are explicit `passed`, `failed`, and `ignored`. An ignored case is
  never reported as passed and never omitted.
- Permitted content is the stable case name, the outcome, a bounded duration,
  and bounded sanitized failure text.
- The canonical forbidden set is environment values, command-line arguments,
  absolute paths, Nix store paths, socket paths, the runfiles root and any
  resolved absolute runfiles or worktree location, systemd unit names, process
  identifiers, user identifiers, opaque handles, terminal bytes, shell names,
  and raw child output. None of it enters a case element, `system-out`, or
  `system-err`. The permitted content list above is closed, so no path-shaped
  string of any kind reaches this document; the declared runfiles-relative path
  a provider refusal may name is a runner refusal reason, not case content.
- Raw child stdout and stderr stay in Bazel's ordinary `test.log` artifact,
  reached through the failed target's test-log link or the Actions artifact, so
  removing raw output from the structured result does not remove the
  contributor's diagnostic path. This split is a contract, not an
  implementation detail: the structured document is bounded and redacted
  because machines consume it, and `test.log` is unbounded and unredacted
  because a human reads it after a failure. Every wave that ships or promotes
  this behavior states both halves in its release note, so a contributor
  reading the changelog learns where the raw output went rather than
  discovering it is missing.

## Filesystem semantics

- `TEST_TMPDIR` is opened once as an anchored directory with close-on-exec.
  Each per-case directory is created descriptor-relative without following
  symlinks or magic links, and an existing case directory is refused rather
  than reused.
- The parent of `XML_OUTPUT_FILE` is opened as a second anchored close-on-exec
  directory descriptor with the same link refusals.
- No output descriptor is opened until every child has been reaped.
- A close-on-exec same-directory temporary is written, synced, and installed
  with `renameat`. Creation and replacement are descriptor-relative.
- A bounded creation loop chooses another unpredictable name after `EEXIST`.
  Exhausting the bound fails **without unlinking any colliding path**, because
  no temporary was created and the runner owns nothing.
- After successful creation, a separate bounded write loop advances the buffer
  after a short write and retries `EINTR` and `EAGAIN`.
- `ENOSPC`, exhausted write retries, and every unhandled post-creation
  filesystem error unlink only the runner-created temporary with `unlinkat`
  before failing the carrier, so no partial evidence remains and no foreign
  path is removed.
- Open, write, sync, rename, unlink, directory enumeration, the anchored
  provider open, the metadata and byte reads the provider checks need, and the
  `execveat` of a verified handle sit behind one injectable filesystem trait in
  `packages/d2b-bazel-support/src/fsops.rs`, so errno mapping, ownership state,
  call ordering, and the open-to-exec binding are hermetically testable rather
  than requiring a full disk or signal races on the shared host.
- That trait is shared with cleanup, with the topology provider checks, with
  the locator, and with the wave-note policy lint, all of which enforce the
  same anchored close-on-exec, link-refusal, escape-refusal, and
  own-what-you-unlink properties on the same syscalls. One implementation means
  one set of planted mutations proves every caller, and a later fix cannot land
  in one copy only. It lives in the neutral `packages/d2b-bazel-support/`
  crate, which declares no first-party dependency, so the locator reaches it
  without depending on the runner and `packages/d2b-contract-tests` reaches it
  as a dev-dependency only. See `recovery-deadline.md` for the cleanup side of
  the same boundary and `workspace-and-tool-pinning.md` for the wave-note side.
- The link-refusal policy is per call site, not global, and each site states
  its policy. Directories the runner creates and files the repository commits
  are opened with `RESOLVE_BENEATH|RESOLVE_NO_SYMLINKS|RESOLVE_NO_MAGICLINKS`,
  because the runner owns those paths and no link belongs on them. Providers
  are opened with `RESOLVE_NO_MAGICLINKS` alone, because a runfiles tree is a
  symlink forest and the two stricter policies were measured to refuse every
  real provider. A site that silently used the other site's policy would either
  refuse every Bazel provider or accept a symlinked note, so the policy is a
  parameter the fake supplies and each caller's choice is asserted. That one
  parameter also decides the leaf on the forced component-walk route, so a
  route change cannot silently move a call site to the other policy.
- No provider negative and no note-corpus negative depends on live host
  filesystem state either. `ENOSPC`, short writes, `EINTR` and `EAGAIN`
  retries, `EEXIST` collisions, replacement races, every absent,
  non-executable, out-of-date, or wrong-identity provider, the post-open path
  rebind, and every `spawn_verified` errno are produced by the injected fake.

### The resolve policy on both routes

The resolve policy is one parameter with two routes, and it means the same
thing on each.

| Policy | Callers | `openat2` route | Forced component-walk route |
| --- | --- | --- | --- |
| Strict | Cleanup, the per-case directories, the `XML_OUTPUT_FILE` parent, the wave-note lint | `RESOLVE_BENEATH\|RESOLVE_NO_SYMLINKS\|RESOLVE_NO_MAGICLINKS` | `O_NOFOLLOW` on every component **including the leaf** |
| Provider | `open_provider` and each declared input its freshness check opens | `RESOLVE_NO_MAGICLINKS` | `O_NOFOLLOW` on every component **except the leaf** |

Intermediate components carry `O_NOFOLLOW` under both policies. The leaf is the
only component the policy moves, and it is the whole difference. Measured on
the reference host against a runfiles-shaped leaf symlink whose target lies
outside the anchor:

| Call | Result |
| --- | --- |
| `openat2` leaf, `RESOLVE_NO_MAGICLINKS` | opens; `st_ino` is the outside target |
| `openat2` leaf, `RESOLVE_NO_SYMLINKS` | `ELOOP` |
| `openat2` leaf, `RESOLVE_BENEATH` | `EXDEV` |
| `openat` leaf, no `O_NOFOLLOW` | opens; the **same** `st_ino` |
| `openat` leaf, `O_NOFOLLOW` | `ELOOP` |
| `openat` intermediate directory symlink, `O_DIRECTORY\|O_NOFOLLOW` | `ENOTDIR` |

The leaf flag is therefore the exact lever that reproduces the policy
difference on the walk route. Hardcoding the leaf exemption, which an earlier
draft of this contract did, hands every strict caller a weaker guarantee on one
of its two routes: a symlinked wave note, a symlinked per-case directory, or a
symlinked `XML_OUTPUT_FILE` parent would be followed out of the anchor instead
of refused, and the four-way errno distinguishability the wave-note lint
depends on would collapse into "it read something". The last row is recorded
because the errno differs by position: `O_DIRECTORY` reaches the refusal before
`O_NOFOLLOW` does, so an intermediate symlink is `ENOTDIR` while a leaf symlink
is `ELOOP`, and a test that asserts one errno for both asserts something the
kernel does not do.

**One property the walk route cannot reproduce, recorded rather than papered
over.** `RESOLVE_NO_MAGICLINKS` has no `openat` flag. Measured: a leaf symlink
whose body names `/proc/<pid>/fd/<n>` is refused `ELOOP` by `openat2` under
`RESOLVE_NO_MAGICLINKS`, opens successfully on the walk route's permissive
leaf, and yields a handle carrying the target's own `st_ino` and the target's
own `fstatfs` filesystem type, indistinguishable from a handle opened through a
leaf symlink that names the target directly. No descriptor-side test closes
that, so none is added: a partial check shaped like a magic-link refusal is
worse than a recorded difference, because it would be cited as one. Two things
bound it. Handle identity, which is what protects a provider in the first
place, is unchanged on this route: the laundered descriptor is still checked
for regular-file kind and executable mode, digested from offset zero to
`st_size`, compared against the value the coverage map records, and `fstat`ed
again after the read, so a descriptor that does not carry the recorded provider
bytes never reaches `spawn_verified`. And the kernel floor closes the
production case outright: ADR 0008 pins supported hosts at `6.6`, with the v1.1
uplift raising that to `6.9`, while `openat2` landed in `5.6`, so no supported
host takes the walk route at all. It exists so the walk's ordering and errno
mapping are provable through the fake, not to serve a kernel this project
supports.

The strict policy has no such gap. `O_NOFOLLOW` on every component including
the leaf refuses a symlink of any kind before it is traversed, which is exactly
what cleanup, the two output-path opens, and the wave-note lint require, and it
is why those callers are strict.

## Publication is enforcing

Publication is part of the enforcing test contract, not optional telemetry:

- a carrier whose tests pass but whose required structured result cannot be
  published **fails**, rather than returning a success-shaped result with
  missing evidence;
- when tests already failed, a publication failure preserves the test failure
  as the primary diagnosis and reports the publication failure as an additional
  bounded runner error.

This is deliberate and is not softened to a warning. One Bazel test action per
carrier means the event stream carries one verdict per target; every finer
attribution the eighteen-surface manifest consumes comes from this document.
A carrier that exits zero with no document has not produced a degraded signal,
it has produced no evidence, and `execution-manifest-binding.md` cannot mark
that surface complete from nothing. Warning instead would let a run report
`passed` for a surface nothing observed, which is precisely the empty-success
class this migration exists to remove, and it would fail in the one direction
reviewers do not check. The cost is bounded and named: the publication failure
is reported as a runner error distinct from a test failure, it never displaces
an existing test failure, and it carries a code-specific recovery message.

Two properties stop the rule from becoming a flake source. The document is
written only after every child is reaped, through the injected filesystem
boundary, into a same-directory close-on-exec temporary that is synced and
installed with `renameat`, so there is no window in which a contended
filesystem yields a partial document the carrier accepts. And every terminal
error on that path is a mapped errno with a planted mutation behind it, so
"publication failed" is a specific reproducible condition rather than an
unexplained nonzero exit.

Two injected outcome tests bind that ordering and must land with the runner
implementation: one starts from an all-passing case set and forces publication
failure, requiring a nonzero carrier result; the other starts from a planted
test failure and forces publication failure, requiring the original test
failure and exit classification to remain primary.

## Required behavioral tests

A committed planted failed-case fixture contains every member of the canonical
forbidden set in its environment, argv, output, and failure text. The test
first asserts every planted value is present in the unredacted fixture, then
requires every value absent from the JUnit bytes, the stable case name,
outcome, and duration present, and raw output recoverable only from the planted
`test.log` path. Asserting absence without first proving presence proves
nothing.

Separate injected cases prove each of: refusal of symlink and magic-link
parents; refusal of an existing case directory; buffer advancement after a
short write; bounded `EINTR` and `EAGAIN` retries; temporary-name collision
retries; failure on `ENOSPC`; no unlink when creation never succeeded;
temporary unlink on every terminal post-creation error; descriptor-relative
`renameat`; sync before rename; close-on-exec on every opened descriptor; no
output descriptor before every child is reaped; and refusal of an anchored `..`
escape. Each property carries a planted mutation the test must reject. The
provider states, mutations, and the one host-backed `execveat` conformance test
are listed under "Binary provider resolution and execution" above and carry the
same requirement.

Three further injected cases bind the resolve policy to the forced
component-walk route, and they belong to every strict caller, not only to the
provider path. On the walk route, a leaf symlink under the strict policy is
refused `ELOOP` for a per-case directory, for the `XML_OUTPUT_FILE` parent, and
for a wave note; the same leaf symlink under the provider policy opens and
every identity check still runs against the resulting handle; and an
intermediate symlink is refused under both policies. The planted mutation is
the hardcoded leaf exemption: a walk route that drops `O_NOFOLLOW` on the final
component regardless of policy must fail all three strict assertions.

## Scope of "no shell"

"No shell" binds repository-owned code and only that: no repository-owned Make
wrapper, case runner, cleanup helper, timeout wrapper, or process-control path
invokes a shell or is implemented as a shell script. On a stable channel
`rust_doc_test` declares a generated `.rustdoc_test.sh` as its test executable
and the compiled alternative is nightly-only, so that generated runner is a
recorded deliberate difference rather than a violation. ADR 0017's scan set is
unchanged and is not widened to output trees: a generated runner in a Bazel
output tree is untracked, is outside `packages/`, and is not a `.rs` file, so
it is excluded by construction and no new exclusion is added.
