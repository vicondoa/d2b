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
  contributor's diagnostic path.

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
- Open, write, sync, rename, and unlink sit behind a small injectable
  filesystem trait, so errno mapping, ownership state, and call ordering are
  hermetically testable rather than requiring a full disk or signal races on
  the shared host.

## Publication is enforcing

Publication is part of the enforcing test contract, not optional telemetry:

- a carrier whose tests pass but whose required structured result cannot be
  published **fails**, rather than returning a success-shaped result with
  missing evidence;
- when tests already failed, a publication failure preserves the test failure
  as the primary diagnosis and reports the publication failure as an additional
  bounded runner error.

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
