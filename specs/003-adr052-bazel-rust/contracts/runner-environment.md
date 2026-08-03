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
- The test binary is resolved through runfiles. Nothing is resolved by an
  absolute execution-root path, which is not a declared input and does not
  survive a different sandbox.
- Concurrency stays bounded by the same `D2B_RUST_BUDGET`-derived control the
  gate already uses. Per-target concurrency never multiplies `--local_test_jobs`
  into an unbounded process count.

## Binary provider resolution

The locator's Bazel arm and the runner's child-binary resolution both look up
a declared runfiles path, and both check the provider before use. Those two
operations sit behind two injected boundaries, not behind the standard library:

- Runfiles lookup goes through `RunfilesView` in
  `packages/d2b-bazel-support/src/runfiles.rs`. The supplied states are a
  declared entry present, a declared entry missing, and a runfiles environment
  that indicates no Bazel test at all. Mode is chosen once from that last
  state; a missing entry in Bazel mode is a hard failure that names the
  expected runfiles path and never falls back to the Cargo arm.
- Existence, executable mode, freshness, and identity go through `FileSystem`
  in `packages/d2b-bazel-support/src/fsops.rs`. Freshness compares the
  provider's modification time against the newest declared input's, both read
  through the boundary. Identity is the digest of the provider's bytes compared
  against the value the coverage map records for that provider. Identity is a
  read, never an execution, which is what allows every provider negative to be
  a supplied state.

**No provider test writes an executable to a live path.** The absent,
non-executable, out-of-date, and wrong-identity providers are states of the
`FileSystem` fake, and the removed runfiles entry is a state of the
`RunfilesView` fake. The stale-provider case in particular is proven by a fake
that reports an out-of-date, wrong-digest executable at the Cargo path while
the fake runfiles view reports no entry. Writing a real stale binary into
`packages/target/` would manufacture, on the shared host, precisely the hazard
the locator exists to refuse, and an interrupted run would leave it there for
the next suite to find. This is the same rule that keeps `ENOSPC` and `EINTR`
off the real disk, applied to the one directory the shadow stage keeps full of
real, out-of-date binaries.

## Per-case result document

- One JUnit document is written to the path Bazel supplies in
  `XML_OUTPUT_FILE`, with one case element per enumerated case.
- Outcomes are explicit `passed`, `failed`, and `ignored`. An ignored case is
  never reported as passed and never omitted.
- Permitted content is the stable case name, the outcome, a bounded duration,
  and bounded sanitized failure text.
- The canonical forbidden set is environment values, command-line arguments,
  absolute paths, Nix store paths, socket paths, runfiles or worktree
  locations, systemd unit names, process identifiers, user identifiers, opaque
  handles, terminal bytes, shell names, and raw child output. None of it enters
  a case element, `system-out`, or `system-err`.
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
- Open, write, sync, rename, unlink, directory enumeration, and the metadata
  reads the provider checks need sit behind one
  injectable filesystem trait in `packages/d2b-bazel-support/src/fsops.rs`, so
  errno mapping, ownership state, and call ordering are hermetically testable
  rather than requiring a full disk or signal races on the shared host.
- That trait is shared with cleanup, with the topology provider checks, and
  with the locator, all of which enforce the same anchored
  close-on-exec, link-refusal, escape-refusal, and own-what-you-unlink
  properties on the same syscalls. One implementation means one set of planted
  mutations proves every caller, and a later fix cannot land in one copy only.
  It lives in the neutral `packages/d2b-bazel-support/` crate, which declares
  no first-party dependency, so the locator reaches it without depending on the
  runner. See `recovery-deadline.md` for the cleanup side of the same boundary.
- No test in this contract may depend on live host filesystem state. `ENOSPC`,
  short writes, `EINTR` and `EAGAIN` retries, `EEXIST` collisions, replacement
  races, and every absent, non-executable, out-of-date, or wrong-identity
  provider are produced by the injected fake.

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
escape. Each property carries a planted mutation the test must reject.

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
