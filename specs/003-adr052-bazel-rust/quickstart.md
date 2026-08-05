# Quickstart: Validate the ADR 0052 Migration

This guide is for implementation and review waves. Commands marked with an
earliest wave do not exist before that wave lands. Run validation only from a
scope-owned worktree on a committed tree.

## Prerequisites

```bash
export D2B_WORKTREE=/absolute/path/to/your/adr052-worktree
cd "$D2B_WORKTREE"
git status --short --branch
nix develop
rustc --version
cat packages/d2b-api-surface/rust-toolchain.toml
```

Expected stable Rust is 1.97.0 and the API pin is `nightly-2026-02-16`. From
W0:

```bash
bazel --version
cat .bazelversion
buildifier --version
make test-drift
```

Both Bazel versions must be 8.6.0, and `bazel_8` plus `bazel-buildtools` must
come from the pinned dev shell. Drift must fail rather than rewrite a lock. Do
not use Bazelisk, direct Bazel workflow commands, a remote cache, or a shared
worktree output tree.

## Mechanical execution admission - before W0

The current tree is intentionally parked. Admission reads exactly three marked
blocks: `plan.md`, `tasks.md`, and
`contracts/workspace-and-tool-pinning.md`. Run this read-only check before T021
or any task, generator, or repin command that can spawn or write:

```bash
set -euo pipefail

if ! command -v python3 >/dev/null 2>&1; then
  printf '%s\n' \
    'broker-repin-architecture-pending' \
    'broker repin is unavailable; no local recovery command exists; prerequisite is an accepted repin-lifecycle ADR plus amended/re-panelled Spec 003.' \
    >&2
  exit 10
fi

python3 - <<'PY'
import os
import stat
import sys

PENDING = (
    b"broker-repin-architecture-pending\n"
    b"broker repin is unavailable; no local recovery command exists; "
    b"prerequisite is an accepted repin-lifecycle ADR plus "
    b"amended/re-panelled Spec 003.\n"
)
BEGIN = b"<!-- BEGIN SPEC003-EXECUTION-STATUS -->"
END = b"<!-- END SPEC003-EXECUTION-STATUS -->"
READY = b"status: READY"
PARKED = b"status: PARKED"
IMMUTABLE_FIELDS = (
    b"parked_at: broker-lock-regeneration",
    b"next_refused_task: T021",
    b"runtime_state_source: task-checkpoints",
    b"resume_requires: "
    b"accepted-repin-lifecycle-adr+amended-spec003+renewed-plan-panel",
)
STATUS_PATHS = (
    ("specs", "003-adr052-bazel-rust", "plan.md"),
    ("specs", "003-adr052-bazel-rust", "tasks.md"),
    (
        "specs",
        "003-adr052-bazel-rust",
        "contracts",
        "workspace-and-tool-pinning.md",
    ),
)
DIR_FLAGS = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC
# O_NONBLOCK makes a planted FIFO reject after fstat instead of blocking.
FILE_FLAGS = os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC | os.O_NONBLOCK


def read_fixed_file(root_fd, components):
    opened_dirs = []
    directory_fd = root_fd
    file_fd = None
    try:
        for component in components[:-1]:
            directory_fd = os.open(
                component, DIR_FLAGS, dir_fd=directory_fd
            )
            opened_dirs.append(directory_fd)
        file_fd = os.open(
            components[-1], FILE_FLAGS, dir_fd=directory_fd
        )
        if not stat.S_ISREG(os.fstat(file_fd).st_mode):
            raise ValueError("execution-status input is not regular")
        chunks = []
        while True:
            chunk = os.read(file_fd, 65536)
            if not chunk:
                return b"".join(chunks)
            chunks.append(chunk)
    finally:
        if file_fd is not None:
            os.close(file_fd)
        for opened_fd in reversed(opened_dirs):
            os.close(opened_fd)


def parse_block(data):
    if b"\0" in data:
        raise ValueError("NUL in execution-status input")
    data.decode("utf-8", errors="strict")
    lines = data.split(b"\n")
    if lines.count(BEGIN) != 1 or lines.count(END) != 1:
        raise ValueError("execution-status marker count")
    begin_index = lines.index(BEGIN)
    end_index = lines.index(END)
    if begin_index >= end_index:
        raise ValueError("execution-status marker order")
    payload = tuple(lines[begin_index + 1 : end_index])
    if len(payload) != 5:
        raise ValueError("execution-status field count")
    if payload[0] not in (READY, PARKED):
        raise ValueError("execution-status value")
    if payload[1:] != IMMUTABLE_FIELDS:
        raise ValueError("execution-status immutable fields")
    return payload


root_fd = None
admitted = False
try:
    # The wrapper is run from the repository root. Open that root once, then
    # resolve every fixed component relative to already-open descriptors.
    root_fd = os.open(".", DIR_FLAGS)
    blocks = tuple(
        parse_block(read_fixed_file(root_fd, path))
        for path in STATUS_PATHS
    )
    admitted = (
        blocks[0] == blocks[1] == blocks[2]
        and all(block[0] == READY for block in blocks)
    )
except (OSError, UnicodeError, ValueError):
    admitted = False
finally:
    if root_fd is not None:
        try:
            os.close(root_fd)
        except OSError:
            admitted = False

if not admitted:
    sys.stderr.buffer.write(PENDING)
    raise SystemExit(10)
PY
```

The wrapper retains `set -euo pipefail`; the executable Python 3 body performs
no task callback, subprocess spawn, or write. It opens the repository root
once, walks every fixed directory component descriptor-relative with
`O_DIRECTORY|O_NOFOLLOW|O_CLOEXEC`, opens each leaf with
`O_RDONLY|O_NOFOLLOW|O_CLOEXEC` (plus `O_NONBLOCK` so a nonregular FIFO cannot
stall refusal), verifies that leaf is regular with `fstat`, and reads bytes
from that same descriptor. It rejects NUL before parsing and validates UTF-8
strictly without using decoded text for the byte comparisons. A directory or
leaf replacement after open cannot redirect a later check or read to a second
path lookup.

The committed development shell does not currently provide Python 3. Provision
the pinned interpreter before entering this wrapper, for example with
`nix shell --quiet --inputs-from "$D2B_WORKTREE" nixpkgs#python3`, then run the
wrapper from the repository root inside that shell. Interpreter acquisition is
bootstrap outside the admission proof. The wrapper itself never invokes Nix;
if `python3` is absent it fails closed with the same exit 10 and fixed two-line
pending result.

On the current tree, exit 10 with exactly the two fixed
architecture-pending lines is required. Exit 0 is valid only when all three
blocks are atomically changed to byte-identical records whose first line is
exactly `status: READY`. The four non-status lines are immutable literals;
only the status may change, and only under a future accepted ADR, amended
Spec 003, and renewed unanimous plan panel. A prefix, suffix, substring, or
`READY` token anywhere else in an extracted block is not a status value.

The persistent admission suite plants every row below independently. Every
rejecting row must return 10, emit only the fixed two-line result and remedy,
and leave the task, child, and write activity record empty.

| Planted case | Required result |
| --- | --- |
| Three byte-identical blocks with exactly `status: READY` | Admit. |
| Any exact `status: PARKED`, including the current three-block state | Reject. |
| `xstatus: READY`, `status: XREADY`, or `status: READY-extra` | Reject prefix or substring. |
| `READY`, `note: READY`, or a second status token elsewhere in the block | Reject misplaced token. |
| Missing, duplicated, empty, misnamed, or unknown status key/value | Reject. |
| Missing source or source extraction error | Reject. |
| A symlink at any directory component or final file | Reject without following it. |
| A directory, FIFO, socket, device, or other nonregular final object | Reject without reading it as status input. |
| A component or leaf replaced between the fake filesystem seam's open and read/check callbacks | Read and validate only the already-open descriptor; the decoy cannot admit. |
| Missing, duplicated, reversed, or nested BEGIN/END marker | Reject. |
| A BEGIN or END marker in any order other than one BEGIN followed by one END | Reject. |
| A NUL inserted into a marker, status field, or non-status field | Reject before parsing. |
| Any invalid UTF-8 byte sequence anywhere in a source | Reject before parsing. |
| Any missing, duplicated, reordered, unknown, misplaced, empty, or changed non-status schema line | Reject. |
| Any byte disagreement among the three extracted blocks | Reject. |

The component-replacement/check-open race is deterministic coverage, not a
timing test: the persistent Layer-1 Rust admission test uses an injected
descriptor-filesystem fake that swaps the name to a decoy after the component
or leaf open callback and before the check/read callback. Independent fixtures
cover the directory-component and leaf cases. The same suite plants the
symlink, nonregular, NUL-marker, NUL-field, invalid-UTF-8, reversed, nested,
missing, duplicate, unknown, misplaced, and three-source disagreement cases.

## Assertion helpers

Every block in this guide that asserts an invariant starts with the same two
lines: `set -e`, then a source of one shared helper file. Write the file once
per validation worktree:

```bash
set -e
mkdir -p .scratch
cat > .scratch/adr052-assert.sh <<'HELPERS'
fail() { printf 'FAIL: %s\n' "$*" >&2; exit 1; }

# Every path an absence claim inspects must be readable, and a directory that
# holds no files makes the claim vacuous rather than true.
require_input() {
  local path
  for path in "$@"; do
    test -r "$path" || fail "cannot read $path"
    if test -d "$path"; then
      test -n "$(find "$path" -type f -print -quit)" \
        || fail "$path holds no files"
    fi
  done
}

# One absence claim. Measured on GNU grep: exit 0 means the pattern is
# present, exit 1 means it is absent, and exit 2 or higher means grep could
# not inspect its input, which is an inspection failure and never a pass.
# Only exit 1 passes. Keeping the grep inside `if` also keeps its non-zero
# exit from tripping `set -e`.
refute() {
  local desc="$1" rc
  shift
  if grep "$@"; then
    fail "$desc"
  else
    rc=$?
    test "$rc" -eq 1 || fail "grep exited $rc while checking: $desc"
  fi
}

# One presence claim, the mirror of refute and subject to the same exit-code
# rule: exit 0 passes, exit 1 is the missing thing, and exit 2 or higher is an
# inspection failure that is never a pass. Never end a presence claim in a
# pipeline; a pipeline reports the last command's status, so `| head` would
# turn a deleted guard into a pass.
require_match() {
  local desc="$1" rc
  shift
  if grep "$@"; then
    :
  else
    rc=$?
    test "$rc" -eq 1 || fail "grep exited $rc while checking: $desc"
    fail "$desc"
  fi
}
HELPERS
. .scratch/adr052-assert.sh
```

`.scratch/` is already ignored by Git, so writing the helper leaves
`git status --short` empty and does not disturb any check below that requires
a clean tree. The helper file sets no shell option of its own, so sourcing it
is safe in an interactive shell; each checking block arms `set -e` itself.
Sourcing rather than executing it is what lets the `exit` inside `fail` stop
the calling block on the line that failed. Run each block as a script; the
checks do not depend on `set -e` because every one of them ends in an explicit
`fail`, but the surrounding `make` and `cargo` lines do.

The exit-code split is the whole point of the helper. A bare
`if grep -q PATTERN FILE; then fail; fi` treats a deleted, renamed, or
unreadable `FILE` exactly like a clean tree: grep exits 2, the `if` takes the
false branch, and the block reports that the forbidden pattern is absent from
a file it never read. The same hazard is worse for `grep -r` over a directory,
because a whole missing tree, for instance `.github/workflows/` after a
rename, is the most likely way that failure arrives. Every absence claim below
therefore goes through `refute`, and every path those claims inspect goes
through `require_input` first.

## Amended ADR verification - before W0

The ADR 0052 amendment is a merged prerequisite, not work this feature
performs. Before any W0 branch is created, prove the amended record is present
in the base by content, not by a remembered commit hash:

```bash
set -e
. .scratch/adr052-assert.sh
require_input docs/adr/0052-bazel-rust-build-and-test.md \
  docs/adr/0054-broker-hub-generated-splice-workspace.md docs/adr/README.md
grep -q '^- Status: Accepted$' docs/adr/0052-bazel-rust-build-and-test.md \
  || fail 'ADR 0052 is not accepted'
grep -q '^- Amended: 2026-08-03\.' \
  docs/adr/0052-bazel-rust-build-and-test.md \
  || fail 'ADR 0052 substrate amendment is missing'
grep -q '^- Amended: 2026-08-05\.' \
  docs/adr/0052-bazel-rust-build-and-test.md \
  || fail 'ADR 0052 qualification amendment is missing'
grep -q 'yanked' docs/adr/0052-bazel-rust-build-and-test.md \
  || fail 'ADR 0052 yanked carrier is missing'
grep -q 'bazel-repin' docs/adr/0052-bazel-rust-build-and-test.md \
  || fail 'ADR 0052 repin contract is missing'
grep -q '0052-bazel-rust-build-and-test.md' docs/adr/README.md \
  || fail 'ADR 0052 index row is missing'
grep -q '^- Status: Proposed$' \
  docs/adr/0054-broker-hub-generated-splice-workspace.md \
  || fail 'ADR 0054 must remain Proposed while broker repin is unresolved'
grep -q '0054-broker-hub-generated-splice-workspace.md' docs/adr/README.md \
  || fail 'ADR 0054 index row is missing'
```

The record must be `Status: Accepted`, must carry both the 2026-08-03 substrate
and 2026-08-05 qualification amendment lines, must already name protected
`v3` as the promotion,
cache-maintenance, cache-publication, streak, and post-promotion lineage, with
the cold measurement set drawn from canonical `C` rows in the complete
authoritative eligible lineage and both same-commit policy and fixture jobs
gating `Q`, must state that the section 6 yanked carrier lands
unconditionally, and must define
`HubInventory = {main, broker, guest, walker}` separately from
`RepinnableHub = {main, guest, walker}`. ADR 0054 must remain Proposed and
indexed; it does not authorize broker lock regeneration.

The ADR names no command for the module lock; it requires only that
`--lockfile_mode=error` fail "with a named remediation". This feature supplies
that name as `cargo xtask bazel-module-refresh`, which is an addition inside
the ADR's own terms rather than a change to them, so no further amendment is a
prerequisite here.

Resolve the amendment commit from history rather than pasting a hash, so the
check keeps working after any rebase or backport. Capture and validate the
history before iterating over it; the pipeline form of this loop cannot fail:

```bash
set -e
. .scratch/adr052-assert.sh
require_input docs/adr/0052-bazel-rust-build-and-test.md

shas=$(git rev-list --reverse HEAD \
  -- docs/adr/0052-bazel-rust-build-and-test.md) \
  || fail 'git rev-list failed on the ADR 0052 path'
test -n "$shas" || fail 'ADR 0052 has no history reachable from HEAD'

substrate_amendment=
qualification_amendment=
while read -r sha; do
  test -n "$sha" || continue
  body=$(git show "$sha:docs/adr/0052-bazel-rust-build-and-test.md") \
    || fail "cannot read ADR 0052 at $sha"
  if printf '%s\n' "$body" | grep -q '^- Amended: 2026-08-03\.'; then
    test -n "$substrate_amendment" || substrate_amendment=$sha
  else
    rc=$?
    test "$rc" -eq 1 || fail "grep exited $rc reading ADR 0052 at $sha"
  fi
  if printf '%s\n' "$body" | grep -q '^- Amended: 2026-08-05\.'; then
    test -n "$qualification_amendment" || qualification_amendment=$sha
  else
    rc=$?
    test "$rc" -eq 1 || fail "grep exited $rc reading ADR 0052 at $sha"
  fi
done <<EOF
$shas
EOF
test -n "$substrate_amendment" \
  || fail 'no commit in that history introduces the 2026-08-03 amendment'
test -n "$qualification_amendment" \
  || fail 'no commit in that history introduces the 2026-08-05 amendment'
printf '%s\n' "$substrate_amendment" "$qualification_amendment"
```

The shape matters more than it looks. `git rev-list ... | while read` reports
only the exit status of the `while`, so a `rev-list` that failed, a path that
no longer exists in this checkout, and a history that genuinely carries no
amendment are all one silent empty result, and the block that consumes it
records "no SHA" as if that were an answer. Capturing first splits those three
into a transport failure, an empty history, and an unresolved amendment, each
with its own `fail`. The loop also reads from a here-document rather than from
a pipe, because a piped `while` runs in a subshell where `amendment` is
assigned and then discarded, which would turn a successful resolution into the
same empty result. Both were measured rather than assumed: the piped form
returns an empty variable where the here-document form returns the value, and
`false | while read -r x; do :; done` exits zero. The `git show` refusal inside
the loop is a backstop and is unreachable on any history that carries the
amendment, since `--reverse` reaches the amendment commit before any later
commit that could have removed the file; it fires only if the object store
cannot produce a blob the commit list just named, which is a condition to stop
on rather than to skip past.

Record both resolved SHAs as evidence and require both to be ancestors of the
W0 base. If any of `.bazelversion`, `.bazelrc`, `.bazelignore`, `MODULE.bazel`,
`MODULE.bazel.lock`, `bazel/`, or generated `packages/**/BUILD.bazel` exists in
a commit that predates the 2026-08-03 substrate amendment, stop.

## Foundation validation - W0

```bash
make check-tier0
make test-lint
make test-rust-schema
make test-rust-inventory
make test-drift
make test-policy
D2B_ENABLE_FIXTURE_BUILD=1 make test-fixture-contracts
```

Both policy and fixture commands are intentional. `make test-policy` executes
the fixture-independent policy binaries, including `policy_docs`, and fails on
a zero-test binary. The fixture command excludes those binaries and covers the
actual fixture-dependent contract and CLI surfaces. Cite each only for the
surface it executed.

Review the schema result for two independent generations, each containing the
exact generated nonempty valid JSON census before comparison. The census is the
manifest the generator returns, not a number written by hand; a review that
checks a literal is checking the wrong thing.

Check the pinning and boundary invariants that are easy to get wrong quietly:

```bash
set -e
. .scratch/adr052-assert.sh
require_input .bazelrc MODULE.bazel .bazelignore Makefile \
  .github/workflows/ packages/

# .bazelrc carries no startup line and no channel flag.
refute '.bazelrc contains a startup option' \
  -qE '^[[:space:]]*startup[[:space:]]' .bazelrc
refute '.bazelrc sets the global Rust channel' \
  -q 'rust/toolchain/channel' .bazelrc

# Both module-graph checks fail closed rather than warn.
grep -q '^common --lockfile_mode=error$' .bazelrc \
  || fail 'lockfile mode is not fail-closed'
grep -q '^common --check_direct_dependencies=error$' .bazelrc \
  || fail 'direct dependency checks are not fail-closed'

# All four hubs declare a Bazel-side lock, a Cargo lock, and the overwrite opt-out.
test "$(grep -cE '^[[:space:]]*lockfile[[:space:]]*=' MODULE.bazel)" -eq 4 \
  || fail 'Bazel-side lock declarations do not match four hubs'
test "$(grep -cE '^[[:space:]]*cargo_lockfile[[:space:]]*=' MODULE.bazel)" -eq 4 \
  || fail 'Cargo lock declarations do not match four hubs'
test "$(
  grep -cE '^[[:space:]]*skip_cargo_lockfile_overwrite[[:space:]]*=[[:space:]]*True' \
    MODULE.bazel
)" -eq 4 \
  || fail 'Cargo overwrite opt-out does not match four hubs'

# Inventory is not execution authority.
grep -qF 'HubInventory = {main, broker, guest, walker}' \
  specs/003-adr052-bazel-rust/contracts/workspace-and-tool-pinning.md \
  || fail 'HubInventory drifted'
grep -qF 'RepinnableHub = {main, guest, walker}' \
  specs/003-adr052-bazel-rust/contracts/workspace-and-tool-pinning.md \
  || fail 'RepinnableHub drifted or accidentally gained broker'

# No repin escape hatch anywhere on the gate path.
refute 'repin control is reachable from Make or CI' \
  -rqE 'CARGO_BAZEL_REPIN|CARGO_BAZEL_REPIN_ONLY|(^|[^A-Z_])REPIN=' \
  Makefile .github/workflows/

# None of the five contributor-only commands is reachable from Make or CI.
contributor_only='bazel-repin|bazel-module-refresh|bazel-yanked-refresh'
contributor_only="$contributor_only|bazel-yanked-check|bazel-evidence"
refute 'contributor-only xtask command is reachable from Make or CI' \
  -rqE "xtask ($contributor_only)" Makefile .github/workflows/

# The only site that may assign a repin control to a process environment.
# The scan must exit 0: exit 1 means the one allowlisted site vanished, and
# exit 2 or higher means the tree was not read, so neither is a pass.
assign_pattern='\.env\("(CARGO_BAZEL_REPIN|CARGO_BAZEL_REPIN_ONLY|REPIN)"'
if scan="$(grep -rnE "$assign_pattern" packages/)"; then
  rc=0
else
  rc=$?
fi
test "$rc" -eq 0 \
  || fail "the repin assignment scan exited $rc rather than matching"
assignments="$(printf '%s\n' "$scan" | cut -d: -f1 | sort -u)"
test "$assignments" = 'packages/xtask/src/bazel.rs' \
  || fail 'repin assignment exists outside packages/xtask/src/bazel.rs'

# The only site that may set one process-globally is nowhere.
setvar_pattern='set_var\("(CARGO_BAZEL_REPIN|CARGO_BAZEL_REPIN_ONLY|REPIN)"'
refute 'repin control uses process-global mutation' \
  -rqE "$setvar_pattern" packages/

# The workspace boundary covers scratch and every Cargo output directory.
grep -q '^\.scratch/$' .bazelignore \
  || fail '.bazelignore omits .scratch/'
```

The third `grep -c` must report four, not zero: at `rules_rust` 0.73.0
`skip_cargo_lockfile_overwrite` defaults to false, so a repin would otherwise
rewrite the authoritative `Cargo.lock`. The assignment scan must resolve to
exactly `packages/xtask/src/bazel.rs`, and it is written so that a grep that
could not read part of `packages/` fails instead of narrowing the result to
the one path that happened to be readable. Grepping for the bare variable
names instead would also match `packages/xtask/tests/policy_ci.rs`, which
necessarily contains all three literals because it is the guard that refuses
them, so the check is on the assignment form, which is what the rule is
actually about.

Regenerating a hub lock is a single reviewed command, never an exported
variable. It is deliberately not a Make target:

```bash
cargo xtask bazel-repin --hub main
git status --short
```

The command must refuse an unknown `--hub` and must refuse to run when the
ambient environment already carries any repin control. It must never leave a
changed `Cargo.lock`, `MODULE.bazel.lock`, `.bazelignore`, generated
`BUILD.bazel`, or `bazel/generated/**`; those are defects, not results, and the
command fails on them rather than reporting success.

Whether the hub lock itself changes depends on whether its inputs moved. On a
committed tree whose lock is already current, the expected outcome is exit zero
with an empty `git status --short`, which is why this is safe to run during
review. The run that must report a changed lock is the one immediately after a
Cargo workspace membership or dependency change; a command wired to the wrong
Bazel invocation shows up there, as a repin that changed nothing when something
had to change.

Broker is not a fourth generic path. Build xtask before beginning observation,
then run the enforcing test through `make test-rust-main`. That test executes
the already-built binary directly as `bazel-repin --hub broker` and requires:

```text
stdout: empty
stderr line 1: broker-repin-architecture-pending
stderr line 2: broker repin is unavailable; no local recovery command exists; prerequisite is an accepted repin-lifecycle ADR plus amended/re-panelled Spec 003.
result: nonzero, no child callback, no write callback
```

Do not substitute `cargo xtask bazel-repin --hub broker` for that evidence.
Cargo may print bootstrap diagnostics and create cache or target state before
the built process runs, so the public launcher is not promised exact two-line
stderr or aggregate no-write behavior.

The same W0 carrier owns the `gen-bazel --check` no-state proof. It runs the
already-built process for both pass and every missing, extra, byte-drift, and
semantic-drift failure after redirecting `HOME`, `TMPDIR`, `TMP`, `TEMP`,
`XDG_CACHE_HOME`, `CARGO_HOME`, `RUSTUP_HOME`, `CARGO_TARGET_DIR`,
`BAZELISK_HOME`, and `TEST_TMPDIR` to empty observed roots. Evidence is valid
only when those roots and the full repository HEAD/index/tracked/untracked/
ignored/kind/mode/bytes/symlink snapshot are unchanged and the injected
observer records no mutation elsewhere. A bare `git status` comparison is not
evidence for this property.

The module lock has its own command, and it is the only supported way to move
`MODULE.bazel.lock`:

```bash
cargo xtask bazel-module-refresh
git status --short
cargo xtask bazel-module-refresh
git status --short
```

Run it twice on purpose. The second run must print nothing, because the command
is required to be idempotent; a second run that changes the lock again means
the invocation is not deterministic and the pin is not a pin. On a tree whose
module lock is already current, both runs are empty. The command takes no
arguments, refuses to start when any repin control is exported, and fails
rather than succeeds if it changed any tracked file other than
`MODULE.bazel.lock`.

Do not copy the refresh line out of a Bazel error. Bazel's own module-lock
diagnostic names `bazel mod deps --lockfile_mode=update`, which is correct
about the mode and silent about every startup option this repository requires;
running it as printed starts a second server and writes a second output base
under your home directory instead of `.scratch/`. Confirm the repository
remediation is what actually ships:

```bash
set -e
. .scratch/adr052-assert.sh
require_input packages/xtask/src/bazel.rs
grep -q 'cargo xtask bazel-module-refresh' packages/xtask/src/bazel.rs \
  || fail 'the module-lock remediation string does not ship'
```

Confirm neither lock command is reachable from a build entry point:

```bash
set -e
. .scratch/adr052-assert.sh
require_input Makefile .github/workflows/
refute 'lock regeneration is reachable from Make or CI' \
  -rqE 'xtask (bazel-repin|bazel-module-refresh)' Makefile .github/workflows/
```

W0 also fixes the dependency direction among the three internal crates this
migration adds. `packages/d2b-bazel-support/` is neutral and holds what more
than one consumer needs: the `FileSystem` boundary, the `RunfilesView`
boundary, and, from W2, the one startup-option construction. The runner, the
locator, and `xtask` read it as a non-dev dependency,
`packages/d2b-contract-tests` reads it as a dev-dependency for the wave-note
lint, and nothing reads the runner. Check the
authority first and the three manifests second:

```bash
set -e
. .scratch/adr052-assert.sh
require_input tests/unit/meta/w0-dep-direction.sh \
  packages/d2b-bazel-support/Cargo.toml \
  packages/d2b-contract-tests/Cargo.toml \
  packages/xtask/Cargo.toml
tests/unit/meta/w0-dep-direction.sh
refute 'the support crate declares a first-party dependency' \
  -nE '^[[:space:]]*d2b[a-z0-9_-]*[[:space:]]*=' \
  packages/d2b-bazel-support/Cargo.toml
refute 'xtask declares the runner or the locator' \
  -nE '^[[:space:]]*d2b-(bazel-runner|test-locator)[[:space:]]*=' \
  packages/xtask/Cargo.toml
require_match 'the contract-tests crate lost its dev edge to the boundary' \
  -nE '^[[:space:]]*d2b-bazel-support[[:space:]]*=' \
  packages/d2b-contract-tests/Cargo.toml
```

The gate is the authority and the two `refute` calls are only a fast local
read: the gate resolves names through `cargo metadata --no-deps`, so it sees a
`package =` rename, a workspace-inherited dependency, and a target-specific
entry that a manifest grep would miss, and it already fails closed when the
resolver is unavailable. Running it directly here is deliberate; `make
test-policy` above ran it too, and a contributor who is about to add a helper
to the runner wants the answer before the wave, not after.

W0 also fixes how a first-party binary is located and run, and that is one
operation rather than two. The boundary opens the provider exactly once, every
check runs against that open descriptor, and the same descriptor is executed
with `execveat` and `AT_EMPTY_PATH`. Nothing on this path stats a name and then
spawns the name. The reason is measurable rather than theoretical: replace a
provider path after the descriptor is open and the retained descriptor still
runs the original verified bytes, while a freshly path-opened descriptor runs
the replacement, so a check-then-spawn locator verifies one file and runs
another and exits zero. `packages/target/` holds real, out-of-date binaries for
the whole shadow stage, and a concurrent Cargo build replaces entries in it by
rename, so the window is not hypothetical here.

```bash
set -e
. .scratch/adr052-assert.sh
require_input packages/d2b-bazel-support/src/fsops.rs \
  packages/d2b-bazel-support/tests/provider_handle.rs \
  packages/d2b-bazel-runner/tests/exec_handle.rs \
  packages/d2b-bazel-runner/src/bin/d2b-exec-probe.rs
require_match 'the boundary does not execute a verified descriptor' \
  -nE 'execveat' packages/d2b-bazel-support/src/fsops.rs
require_match 'the exec is not AT_EMPTY_PATH on the open descriptor' \
  -nE 'AT_EMPTY_PATH' packages/d2b-bazel-support/src/fsops.rs
require_match 'the provider open no longer refuses magic links' \
  -nE 'RESOLVE_NO_MAGICLINKS' packages/d2b-bazel-support/src/fsops.rs
require_match 'the walk route no longer applies O_NOFOLLOW at all' \
  -nE 'O_NOFOLLOW' packages/d2b-bazel-support/src/fsops.rs
require_match 'the strict-leaf refusal on the walk route is gone' \
  -n 'walk_route_strict_policy_refuses_a_leaf_symlink' \
  packages/d2b-bazel-support/tests/provider_handle.rs
require_match 'the provider-leaf acceptance on the walk route is gone' \
  -n 'walk_route_provider_policy_accepts_a_leaf_symlink_and_still_verifies_identity' \
  packages/d2b-bazel-support/tests/provider_handle.rs
require_match 'the intermediate-symlink refusal is gone' \
  -n 'walk_route_refuses_an_intermediate_symlink_under_both_policies' \
  packages/d2b-bazel-support/tests/provider_handle.rs
require_match 'the post-open path-rebind negative is gone' \
  -nE 'rebound|rebind' packages/d2b-bazel-support/tests/provider_handle.rs
refute 'a first-party provider is spawned by path' \
  -rnE 'Command::new|fexecve|/proc/self/fd' \
  packages/d2b-bazel-support/src/fsops.rs \
  packages/d2b-test-locator/src \
  packages/d2b-bazel-runner/src/topology.rs
```

Then run the two provider suites and read what they claim, because a green
target says nothing about which cases ran:

```bash
set -e
cargo test -p d2b-bazel-support --test provider_handle -- --nocapture
cargo test -p d2b-bazel-runner --test exec_handle -- --nocapture
```

`provider_handle.rs` is hermetic: every provider negative is a state of the
`FileSystem` fake, which models inodes rather than paths, so "the path now
names a different file" is representable. Read its three walk-route cases
before trusting the boundary, because the walk route is where an earlier draft
of the contract exempted the final component from `O_NOFOLLOW` for every
caller. Under the strict policy a leaf symlink must be refused `ELOOP` with
nothing read; under the provider policy the same leaf symlink must open, yield
the inode the `openat2` route yields, and still pass kind, mode, freshness, the
digest compared against the coverage map, and the bracketing `fstat`; and an
intermediate symlink must be refused under both policies with `ENOTDIR`, not
`ELOOP`, because `O_DIRECTORY` reaches the refusal first. Both hardcoded-leaf
mutations, exempting the leaf always and applying `O_NOFOLLOW` always, must
fail. `exec_handle.rs` is the one
deliberately non-hermetic test in the design, and it is deliberate because
every claim above about `execveat`, close-on-exec inheritance, and executing
one descriptor repeatedly is a claim about the kernel, which a fake cannot
prove. It opens a handle on the first-party probe binary, requires the exec to
succeed, requires the provider inode to be absent from the child's descriptor
table, requires a second exec of the same handle to succeed so process-per-case
survives, and requires a deliberately non-close-on-exec control descriptor to
be present, so the absence assertion is proven able to fail. It arranges
nothing on the host and writes no executable anywhere.

Each of the two commands refuses an exported repin control, and each refusal
ends on the command that was refused rather than on a shared template. Check
both, because a single templated remedy would have to name a `--hub` that
`bazel-module-refresh` never takes:

```bash
set -e
. .scratch/adr052-assert.sh

# Plant a value nothing else could have produced. The digit 1 is not a
# sentinel: it occurs in version strings, counts, and paths, so asserting
# its absence asserts nothing.
ambient_value='D2B-AMBIENT-SENTINEL-a41f7c'

# Both commands must refuse, so exit zero is itself the defect. Capturing
# the expected failure inside `if` keeps the non-zero exit from tripping
# `set -e`.
if repin_out="$(CARGO_BAZEL_REPIN="$ambient_value" \
    cargo xtask bazel-repin --hub main 2>&1)"; then
  fail 'bazel-repin ran under an ambient control instead of refusing'
fi
if refresh_out="$(CARGO_BAZEL_REPIN="$ambient_value" \
    cargo xtask bazel-module-refresh 2>&1)"; then
  fail 'bazel-module-refresh ran instead of refusing'
fi

# Each refusal ends on the command that was refused. The captured output is
# an in-memory string, so grep reads standard input rather than a file, but
# the exit-code rule is the same: only 1 is an absence and only 0 is a
# presence, and any higher status means grep did not read what it was given.
printf '%s\n' "$repin_out" \
  | grep -qF 'then run `cargo xtask bazel-repin --hub main`.' \
  || fail 'the bazel-repin refusal lost its own remedy'
printf '%s\n' "$refresh_out" \
  | grep -qF 'then run `cargo xtask bazel-module-refresh`.' \
  || fail 'the bazel-module-refresh refusal lost its own remedy'

# The module-lock refusal must not name the hub command at all. The match
# is literal and case-sensitive, so the `CARGO_BAZEL_REPIN` that refusal
# does name is not a false positive.
refute 'the bazel-module-refresh refusal names bazel-repin' \
  -qF 'bazel-repin' <<<"$refresh_out"

# Neither refusal may echo back the value it refused.
refute 'the bazel-repin refusal echoed the ambient value' \
  -qF "$ambient_value" <<<"$repin_out"
refute 'the bazel-module-refresh refusal echoed the ambient value' \
  -qF "$ambient_value" <<<"$refresh_out"
```

The two remedy checks are what a shared template would break: one row would
have to name a `--hub` that the module-lock command never takes. The check
after them is written as a refusal, because the module-lock command's output
must not mention `bazel-repin` at all; a shared row would have put it there,
sending a contributor who was updating the module lock off to repin a hub they
never named. The last two checks assert that the planted value is absent from
both refusal outputs, which is a claim worth making only because the value is
unique: the ambient controls are named in the remedy, but what they were set
to is never printed.

The three absence checks go through `refute` and are fed by a here-string
rather than a pipe. A pipe would run `refute` in a subshell, where the `exit`
inside `fail` leaves only that subshell; the here-string keeps the helper in
the current shell, so a detected leak stops the block on the line that found
it. Neither shape is a bare `!` line. Measured on bash 5.3.9: a command whose
value is inverted with `!` is exempt from `set -e`, so under `set -e` a bare
`! grep -q ...` neither stops the script nor prints anything, and a leak would
pass unnoticed. `grep ... && exit 1` is worse: when the grep correctly finds
nothing, the `&&` list itself reports non-zero, so the block ends in apparent
failure exactly when it passed.

Confirm `make test-rust` still invokes Cargo and remains authoritative:

```bash
D2B_SKIP_FIXTURE_BUILD=1 \
  D2B_EXECUTION_MANIFEST=.scratch/adr052-cargo-foundation.json \
  make test-rust
nix shell --quiet --inputs-from . nixpkgs#check-jsonschema --command \
  check-jsonschema \
  --schemafile docs/reference/schemas/test-execution-manifest-v1.json \
  .scratch/adr052-cargo-foundation.json
jq -e '
  .version == 1 and
  .target == "test-rust" and
  .run_status == "passed" and
  .completed_leaves == [
    "rust-api-surface",
    "rust-assert-pinned",
    "rust-audit-broker",
    "rust-audit-guest",
    "rust-audit-main",
    "rust-broker-default",
    "rust-broker-fakebackends",
    "rust-broker-layer1",
    "rust-deny-broker",
    "rust-deny-guest",
    "rust-deny-main",
    "rust-guest-shell-runner",
    "rust-main-clippy",
    "rust-main-format",
    "rust-main-workspace-tests",
    "rust-no-bash-ast",
    "rust-schema-reproducibility",
    "rust-stub-no-socket"
  ] and
  (.failed_surfaces | length) == 0
' .scratch/adr052-cargo-foundation.json
```

Use `D2B_SKIP_FIXTURE_BUILD=1` only when comparing the eighteen-surface CI
baseline. Validate policy documentation and fixture coverage under their
actual separate carriers:

```bash
make test-policy
D2B_ENABLE_FIXTURE_BUILD=1 make test-fixture-contracts
```

`make test-policy` is the wave-note verdict from W0 onward. Every measured
invocation a wave records under `specs/003-adr052-bazel-rust/wave-notes/` is
written as a command shape with `<worktree>` placeholders, and the rule is
enforced by a policy lint in
`packages/d2b-contract-tests/tests/policy_docs.rs` that `make test-policy`
already
executed. Do not hand-roll a shell scan over the notes and do not plant a note
file, tracked or untracked, to see the scan fail: the lint refuses an empty
corpus and refuses any entry in that directory that is not a readable
`w<digits>.md` note, so a stray file is caught either way, and the lint's own
planted cases live in the injected fake and in test literals.
Committed `tests/lib.sh` includes `policy_docs` in the fixture-independent
policy list, `tests/test-policy.sh` executes that closed list and fails on a
zero-test binary, and the fixture lane excludes it. The fixture command above
therefore proves only its fixture-dependent contract and CLI surfaces. Confirm
the lint is present, reaches the notes through the shared
filesystem boundary rather than through the standard library, carries both
violation shapes, and carries the two cases that police its own refusals:

```bash
set -e
. .scratch/adr052-assert.sh
require_input packages/d2b-contract-tests/tests/policy_docs.rs \
  specs/003-adr052-bazel-rust/wave-notes/
require_match 'the wave-note lint no longer names the notes directory' \
  -n 'specs/003-adr052-bazel-rust/wave-notes' \
  packages/d2b-contract-tests/tests/policy_docs.rs
require_match 'the wave-note scanner was removed from the policy carrier' \
  -n 'wave_note_violations' \
  packages/d2b-contract-tests/tests/policy_docs.rs
require_match 'the corpus is no longer read through the shared boundary' \
  -n 'wave_note_entries' \
  packages/d2b-contract-tests/tests/policy_docs.rs
require_match 'the two violation shapes collapsed back into one' \
  -nE 'PathLeak' packages/d2b-contract-tests/tests/policy_docs.rs
require_match 'the read-failure shape was removed' \
  -nE 'ReadError' packages/d2b-contract-tests/tests/policy_docs.rs
require_match 'the scanner no longer refuses its own rendered refusals' \
  -n 'wave_note_refusals_carry_no_path_token' \
  packages/d2b-contract-tests/tests/policy_docs.rs
require_match 'the per-shape remedy case is gone' \
  -n 'wave_note_refusals_carry_only_their_own_remedy' \
  packages/d2b-contract-tests/tests/policy_docs.rs
require_match 'the stable-enumeration-order case is gone' \
  -n 'wave_note_entries_are_sorted_before_any_label_is_assigned' \
  packages/d2b-contract-tests/tests/policy_docs.rs
require_match 'the fixed corpus-directory case is gone' \
  -n 'wave_note_corpus_errors_name_the_repository_relative_directory' \
  packages/d2b-contract-tests/tests/policy_docs.rs
require_match 'the raw-byte ordering case is gone' \
  -n 'wave_note_entries_sort_raw_names_by_unsigned_byte_order' \
  packages/d2b-contract-tests/tests/policy_docs.rs
require_match 'the non-UTF-8 entry-name case is gone' \
  -n 'wave_note_refusal_label_falls_back_to_position_for_a_non_utf8_entry_name' \
  packages/d2b-contract-tests/tests/policy_docs.rs
refute 'the lint reads the corpus with the standard library' \
  -nE 'std::fs::read_dir|std::fs::read_to_string|read_dir\(' \
  packages/d2b-contract-tests/tests/policy_docs.rs
require_match 'the entry name is no longer held as raw bytes' \
  -n 'OsString' packages/d2b-contract-tests/tests/policy_docs.rs
require_match 'the enumeration no longer sorts on the raw name bytes' \
  -n 'as_bytes' packages/d2b-contract-tests/tests/policy_docs.rs
```

Read one refusal of each shape before trusting the lint. A `PathLeak` must name
the note label and the one-based line and give the rewrite-or-drop remedy; a
`ReadError` must name the note label and its errno and give the file-level
remedy; an `Unreadable` and an `Empty` must each name the fixed
repository-relative `specs/003-adr052-bazel-rust/wave-notes/` and no resolved
path, because a corpus error names no entry and the directory is the whole
subject of its remedy;
neither may print the offending token, and neither may carry the other's
remedy. A refusal is republished into the target's output, into panel comments,
and into PR bodies, so a message that echoes the leaked path has spread it
further than the note did, and a message that carries the wrong remedy sends a
contributor to fix a line that is not the problem. The six named-case
`require_match`
calls above are the mechanical half of those rules: one names the case that
runs every rendered refusal and every rendered corpus error back through the
scanner's own path-token and
worktree-substring tests, the second names the case that asserts each shape
renders its own remedy and none of the other three, the third names the case
that supplies one corpus in two enumeration orders and requires identical entry
order, violation order, and position labels, the fourth names the case that
pins the corpus directory literal, the fifth names the case that requires the
sort to run over the raw name bytes, and the sixth names the case that requires
a position-labelled refusal for an entry whose raw name has no UTF-8 rendering.
The two trailing presence claims are regression tripwires rather than proofs:
they catch a change that reverts the name field to `String` or the sort key to
a rendered form, which is the shape of the defect this section exists to
prevent.

Why that literal is fixed rather than rendered is the same self-application
argument: the enumerator resolves the corpus beneath `repo_root()`, so a
message that printed what it opened would carry the contributor's worktree,
which is an absolute path FR-029 forbids in a refusal and which
`wave_note_refusals_carry_no_path_token` would then catch. That is left to the
test rather than to a grep, because a `repo_root().join(...)` call can wrap
across lines and a regex that misses the wrap reports a clean file it never
understood.

Sorting matters for the same reason. Directory order is not an order: the same
seven note names enumerate as `w2 w0 w1 w11 w3 w10 w9` on ext4 and as
`w3 w11 w1 w0 w2 w10 w9` on tmpfs, so a position label taken from raw
enumeration names a different entry in CI than it does locally, and a refusal
nobody can reproduce is a refusal that gets ignored.

The name type is part of the same argument. A directory entry on Linux is any
NUL-free, `/`-free byte string, so the entry name is a `std::ffi::OsString`
holding those bytes and the sort key is `OsStr::as_bytes()`. A `String` field
would have to drop such an entry, which makes the `w<digits>.md` refusal blind
to the one entry nobody creates by accident, or convert it lossily, which was
measured to give the distinct names `w\xff9.md` and `w\xfe9.md` one shared
label and one shared sort key, and to sort raw `w\x80.md` after the valid UTF-8
name `w\xc3\xa9.md` when the bytes order it before. UTF-8 is required only at
the renderer, where `NoteLabel::Name` is built solely from a successful
`OsStr::to_str()` whose `&str` then passes the lint's own rules.

Two halves are not greppable and are deliberately left to the compiler and the
tests. The first is the shape of the enumeration API: it returns a corpus error
for an unreadable or empty directory and a `std::io::Result` per entry holding
exactly what the boundary read returned, so a committed subdirectory, a
symlink, a permission denial, and non-UTF-8 content stay four distinguishable
errnos. The second is that `PathLeak` has no error member and `ReadError` has
no line member, which is what makes a cross-rendered remedy unrepresentable
rather than merely tested. A grep over a Rust signature that may wrap across
lines is exactly the fragile check the rest of this guide argues against, so do
not add one.

Every check above is a presence or absence claim and none ends in `| head`: a
pipeline
reports the last command's status, so a `head` on the end would turn a deleted
lint into a pass. In the W1 and W2 sections the same target covers `w1.md` and
`w2.md`; nothing further is needed there, because the committed lint scans the
whole directory and a wave that adds a note has already changed an input this
crate reads.

## Local Bazel aggregate and slices - W1

```bash
make test-bazel-rust
make test-bazel-rust-main
make test-bazel-rust-api
make test-bazel-rust-broker
make test-bazel-rust-aux
```

Capture and verify execution evidence:

```bash
D2B_SKIP_FIXTURE_BUILD=1 \
  D2B_EXECUTION_MANIFEST=.scratch/adr052-bazel-pass.json \
  make test-bazel-rust

nix shell --quiet --inputs-from . nixpkgs#check-jsonschema --command \
  check-jsonschema \
  --schemafile docs/reference/schemas/test-execution-manifest-v1.json \
  .scratch/adr052-bazel-pass.json
jq -e '
  .version == 1 and
  .target == "test-rust" and
  .run_status == "passed" and
  (.completed_leaves | length) == 18 and
  (.failed_surfaces | length) == 0
' .scratch/adr052-bazel-pass.json
```

Then keep the adjacent enforcing surfaces green:

```bash
make test-bazel-rust
make test-rust
make test-policy
D2B_ENABLE_FIXTURE_BUILD=1 make test-fixture-contracts
make test-drift
```

A failed or handled-interrupted probe must leave a schema-valid partial
manifest with nonempty `failed_surfaces` and must not retain an earlier passing
manifest. Perform interruption only in a disposable validation worktree.

## Census, topology, evidence, and planted failures - W1 and W4

The W1 aggregate checks:

- exact per-binary test and ignored-case census, generator-derived;
- derived nonempty doctest and executed harness-free companions, with every
  out-of-census manifest entry recorded with its reason;
- main and guest one-process-per-case topology with a per-case directory
  beneath the executor temporary root;
- broker production plus independent default, layer1-bootstrap, and
  fake-backends configured graphs. The measured first-party target censuses are
  7/23/23/23 and the three test-carrier case censuses are 557/492/559; exact
  target and case-name sets, not these descriptive counts, are authoritative;
- broker one-process-per-binary topology with exclusivity for all three test
  carriers;
- exact governed-source, runfiles, and parsed-file equality;
- two independent schema generations against the generated census;
- the nightly API census, including the toolchain version the action actually
  used compared against the committed pin;
- binary existence, kind, executability, freshness, and identity through the
  dual-mode locator, every one of them read from the one descriptor the
  provider was opened on rather than by touching a live path, and the same
  descriptor executed with `execveat` and `AT_EMPTY_PATH`.

Run the carriers:

```bash
make test-bazel-rust-main
make test-bazel-rust-api
make test-bazel-rust-broker
make test-bazel-rust-aux
```

Before accepting broker coverage, inspect the independent missing, extra,
empty, and misnamed mutations for F, B, M, B-prod, B-default, B-layer1, and
B-fake. Also require target omission/addition, inert-source substitution,
per-carrier feature/edge/target/case mutations, contracts and host
configured-destination swaps, production-to-test edges, each test carrier
reaching production or another carrier, B reaching `@main//`, and M reaching
`@broker//` to fail at distinct guards. Expected source and target identities
must come from authoritative Cargo manifests and locked metadata; actual
configured features and edges must come from query/cquery plus an aspect, not
from a generator-emitted expected map.

Inspect the per-case evidence rather than trusting a green target. For a
carrier under the local output tree, confirm the result document exists, names
individual cases, keeps ignored cases distinct, and carries none of the
forbidden values, while raw output remains reachable in `test.log`:

```bash
find .scratch/bazel -name test.xml -path '*ci/rust*' | head
find .scratch/bazel -name test.log -path '*ci/rust*' | head
```

Two negative controls matter more than the positive ones:

- the planted stale-provider case, in which the injected `FileSystem` fake
  reports an out-of-date, wrong-digest executable at the Cargo path and the
  injected `RunfilesView` fake reports no entry, so the Bazel arm must fail
  naming the declared runfiles-relative path rather than passing against the
  stale provider. That path is repository content, the string the target's own
  `data` declaration produces, so naming it leaks nothing; the runfiles root it
  would resolve beneath is a local value and stays out of the message. Nothing
  is written to `packages/target/` to produce it: that
  directory is the hazard, not the fixture. Beside it, the planted
  path-rebind case rebinds the provider path to a different inode after the
  open and requires the verified descriptor's bytes to be the ones that ran,
  which is the failure a check-then-spawn-by-path locator has and cannot see;
- the planted redaction fixture, which must first prove every forbidden value
  is present in the unredacted fixture and then prove every one absent from the
  result document.

Confirm the supply-chain carriers include the yanked-state check, which lands
whether or not the recorded comparison found a difference:

```bash
ls bazel/supply_chain/
jq -e 'has("entries") and (.entries | length) > 0' \
  bazel/supply_chain/yanked-snapshot.json
cargo xtask bazel-yanked-check
make test-drift
```

An all-clear snapshot with no yanked entry is the expected result and is still
committed. What must fail is a snapshot whose `(name, version)` key set differs
from the three committed locks in either direction.

The snapshot has two commands and they are not interchangeable.
`cargo xtask bazel-yanked-refresh` is the reviewed networked update that
rewrites the snapshot; `cargo xtask bazel-yanked-check` is the offline
validator that proves the committed snapshot still matches the three locks. The
check is what the three Bazel carriers run as a declared-input action, so the
command above prints the same bytes the gate would, without contacting the
index. When the check refuses, the recovery is exactly:

```bash
cargo xtask bazel-yanked-refresh
git diff -- bazel/supply_chain/yanked-snapshot.json
git add bazel/supply_chain/yanked-snapshot.json
cargo xtask bazel-yanked-check
```

Ending on the check rather than on the refresh is the point. A refreshed
snapshot nobody validated is how a mismatched key set reaches continuous
integration instead of your shell.

The refresh is the only command here that opens a socket, and it does so
through one boundary so its failure paths are testable without a network.
Confirm the boundary is where it is claimed to be:

```bash
set -e
. .scratch/adr052-assert.sh
require_input packages/xtask/src/

# The trait and its single networked implementation live together.
grep -q 'trait YankedIndex' packages/xtask/src/bazel_yanked.rs \
  || fail 'the YankedIndex boundary is missing'
grep -q 'struct IndexClient' packages/xtask/src/bazel_yanked.rs \
  || fail 'the networked IndexClient is missing'

# Only the refresh and its routing seam name the networked implementation.
# The scan must exit 0: exit 1 means the implementation vanished, and exit 2
# or higher means part of the tree was never read, so neither is a pass.
if client_scan="$(grep -rn 'IndexClient' packages/xtask/src/)"; then
  rc=0
else
  rc=$?
fi
test "$rc" -eq 0 \
  || fail "the IndexClient scan exited $rc rather than matching"
client_files="$(printf '%s\n' "$client_scan" | cut -d: -f1 | sort -u)"
expected_client_files="$(printf '%s\n' \
  packages/xtask/src/bazel_yanked.rs packages/xtask/src/main.rs | sort -u)"
test "$client_files" = "$expected_client_files" \
  || fail 'IndexClient is named outside the refresh and its routing seam'

# Every refresh case runs against an injected fake, so this passes offline.
cargo test -p xtask bazel_yanked
```

The scan resolves to exactly `packages/xtask/src/bazel_yanked.rs` and
`packages/xtask/src/main.rs`, compared against a generated list rather than
read by eye, and nothing on the validator's path may name `YankedIndex` or
`IndexClient`. The `cargo test` line is the real check: every
refresh case in that module supplies its index answer through an injected fake,
so it must pass with no route to the index at all. Run it that way. What no
fake can prove is that `IndexClient` speaks to the real index correctly; that
is measured once, by the reviewed refresh above, whose command shape and
observed index revision the wave that committed the snapshot recorded.

Seeded failures are made on eighteen disposable, committed evidence branches,
one protected condition per branch. Each branch runs only its owning approved
slice and the aggregate manifest adapter. W4 records immutable commits and
results in:

```text
specs/003-adr052-bazel-rust/evidence/qualification.json
```

Audit the completed matrix:

```bash
jq -e '
  .status == "qualified" and
  (.seeded_failures | length) == 18 and
  ([.seeded_failures[].surface_id] | unique | length) == 18 and
  all(.seeded_failures[];
    (.observed_failed_surfaces == [.surface_id]) and
    (.unrelated_failures | length) == 0)
' specs/003-adr052-bazel-rust/evidence/qualification.json
```

Audit topology, per-case evidence, locator migration, and broker repetition:

```bash
jq -e '
  (.topology_proofs | length) == 5 and
  all(.topology_proofs[];
    .census_matches and .ignored_census_matches and
    .per_case_results_published and .shell_free) and
  (.broker_repetitions | length) == 3 and
  all(.broker_repetitions[];
    .exclusive and .consecutive_passes == 20) and
  .locator_migration.unresolved_files == 0 and
  .locator_migration.injected_stale_provider_refused_in_bazel_mode and
  .locator_migration.rebound_provider_path_did_not_change_executed_bytes and
  .locator_migration.exec_handle_conformance_passed and
  (.locator_migration.live_paths_written == 0)
' specs/003-adr052-bazel-rust/evidence/qualification.json
```

## Operational safety - W2

Run the existing carriers that own the new behavioral and policy tests:

```bash
make test-rust-main
make test-policy
make check-tier0
make test-bazel-rust
D2B_CLEAN_DRY_RUN=1 make clean
D2B_ENABLE_FIXTURE_BUILD=1 make test-fixture-contracts
```

The targeted tests must exercise both cleanup syscall routes, descriptor
inheritance, replacement race, dedicated process group, fixed grace, final
SIGKILL, sibling survival, deadline grammar and rounding, stuck shutdown,
message redaction, every wrong-remedy mutation, and the result-document
filesystem cases: link and anchored-escape refusal on both resolution routes,
creation ownership,
short-write and collision handling, sync before rename, close-on-exec, and
child-reap ordering. Cleanup and the result writer are strict callers, so the
forced component-walk route must carry `O_NOFOLLOW` on the leaf as well as on
every intermediate component for both of them; a symlink planted at the final
name of a cleanup target or of the `XML_OUTPUT_FILE` parent has to be refused
`ELOOP` on that route and not only on the `openat2` one.

Every one of those states is supplied through an injected boundary, not
arranged on the host. Cleanup, the result writer, the topology provider checks,
the locator, and the wave-note policy lint share the `FileSystem`
trait in `packages/d2b-bazel-support/src/fsops.rs`, the locator and the runner
share `RunfilesView` in `packages/d2b-bazel-support/src/runfiles.rs`, and the
deadline path takes
`Clock` and `UptimeSource` from `packages/d2b-bazel-runner/src/clock.rs`. The
one exception is `packages/d2b-bazel-runner/tests/exec_handle.rs`, which drives
the host-backed implementation on purpose because its subject is kernel
behavior a fake cannot establish. When
reviewing W2, check that no test fills a disk, requires a privileged mount,
sleeps to reach an expiry, or reads the host clock; a test that does is not
reproducible and will be disabled later:

```bash
set -e
. .scratch/adr052-assert.sh
require_input packages/d2b-bazel-runner/tests/ \
  packages/d2b-bazel-runner/src/cleanup.rs \
  packages/d2b-bazel-runner/src/deadline.rs \
  packages/d2b-bazel-support/src/fsops.rs
refute 'runner tests depend on ambient time or uptime' \
  -rnE 'thread::sleep|SystemTime::now|/proc/uptime' \
  packages/d2b-bazel-runner/tests/
if grep -rn 'fsops::' packages/d2b-bazel-runner/src/cleanup.rs; then
  :
else
  rc=$?
  test "$rc" -eq 1 || fail "grep exited $rc while inspecting cleanup.rs"
  fail 'cleanup.rs never calls through the fsops boundary'
fi
if grep -rn 'clock::' packages/d2b-bazel-runner/src/deadline.rs; then
  :
else
  rc=$?
  test "$rc" -eq 1 || fail "grep exited $rc while inspecting deadline.rs"
  fail 'deadline.rs never calls through the clock boundary'
fi
```

The `refute` line is the one that could go quiet. Written as a bare
`if grep -rn ... tests/; then ... fi`, a renamed or not-yet-created
`packages/d2b-bazel-runner/tests/` makes grep exit 2, the `if` take the false
branch, and the block report that no test reads the host clock when it read no
test at all. `require_input` refuses a missing or empty directory first, and
`refute` refuses every exit above 1 after that.

The two `fsops::` and `clock::` checks are the mirror image, and they carry the
same hazard in the opposite direction. They are presence claims, not absence
claims, so `refute` is the wrong helper; but written as
`grep -rn 'fsops::' ... | head` they were worse than either, because the
pipeline reports `head`'s status and never grep's. A file that lost its
boundary call, was renamed, or was never created all produced the same silent
success. Written as above, exit 1 fails as a missing call and exit 2 or higher
fails as an inspection error, each with its own message, and only exit 0
passes.

Prove startup options are byte-identical across every command the wrapper
issues, because a mismatch starts a second server:

```bash
make bazel-shutdown
```

On `D2B-BZLSERVER-STUCK`, close other clients and rerun that command. Do not
delete `.scratch/bazel/` or signal a PID by hand.

Trimming is an explicit synchronous step, not an idle-time side effect.
Confirm the wrapper runs the on-demand collector and observes its completion
before it measures or publishes anything; a measurement taken before the
collector finished is invalid even if the number looks fine.

## Warm and cold local measurements - W2 and W4

Use three dedicated measurement worktrees based on the same W4 candidate.
Ensure no heavy lane is active and that no measurement uses
`--test_output=streamed`, which silently serializes every test. In each
worktree, independently perform the complete warm sequence: prime
successfully, append exactly one comment line to
`packages/d2b-core/src/lib.rs`, and immediately measure without shutdown,
cleanup, or another command between the prime and the edit:

```bash
make test-bazel-rust
printf '\n// ADR 0052 warm measurement\n' \
  >> packages/d2b-core/src/lib.rs
time make test-bazel-rust
```

Discard each measurement worktree after recording its result. Record
output-root size before and after. The median must be at most 600 seconds and
every sample at most 720.

For each cold-local sample, retain a populated repository cache while creating
a fresh output user root and empty action cache:

```bash
cargo xtask bazel-evidence prepare-cold-local
time make test-bazel-rust
```

The evidence-only preparation helper exists from W2 through W4 and is removed
in W5. Run three samples. The median must be at most 900 seconds and every
sample at most 1080. A cleanup, hard-limit refusal, server restart during a
warm sample, wrong cache state, streamed test output, or heavy-lane overlap
invalidates and replaces the sample.

Expect the broker suites to run last and alone in the local aggregate: exclusive
tests execute one at a time after the whole parallel phase, so that shape is
correct rather than a scheduling bug.

Audit all performance sets:

```bash
jq -e '
  (.performance_sets | length) == 3 and
  all(.performance_sets[]; .valid == true) and
  all(.performance_sets[]; .invocation_flags | index("--test_output=streamed") | not) and
  (.performance_sets[] | select(.profile == "warm-local")
    | .median_seconds <= 600 and .maximum_seconds <= 720) and
  all(.performance_sets[] | select(.profile != "warm-local");
    .median_seconds <= 900 and .maximum_seconds <= 1080) and
  (.performance_sets[] | select(.profile == "cold-ci") |
    (.sample_refs | length) == 5 and
    (.feasibility_ref | length) > 0 and
    all(.sample_refs[];
      .qualifies and
      .source_event == "push" and .branch == "refs/heads/v3" and
      .policy_job.conclusion == "success" and
      .fixture_job.conclusion == "success" and
      (.slice_jobs | length) == 4 and
      all(.slice_jobs[]; .conclusion == "success") and
      .cache_restored == 0))
' specs/003-adr052-bazel-rust/evidence/qualification.json
```

## Shadow workflow and qualification records - W3 and W4

Run all local slices first:

```bash
make test-bazel-rust-main
make test-bazel-rust-api
make test-bazel-rust-broker
make test-bazel-rust-aux
make test-rust-main
make test-policy
make test-lint
D2B_ENABLE_FIXTURE_BUILD=1 make test-fixture-contracts
```

Review the shadow run rather than treating green as sufficient. It must be
non-required, use four slices and a rollup, restore and save nothing, call only
Make targets, use credentialless checkout, and grant pull-request jobs only
`contents: read`.

Evidence comes only from ADR 0052 section 9's canonical `Q` predicate over the
complete chronological eligible protected-`v3` lineage. The authoritative
resolver, not authored JSON, binds immutable workflow run/job IDs, attempt,
head SHA, event, branch, merged-PR eligibility, and conclusion for Cargo,
Bazel, same-commit `make test-policy`, same-commit fixture contracts, and all
four slices.

A genuine eligible push with a missing, failed, timed-out, skipped, or
cancelled carrier remains a false-`Q` reset row, including full cancellation.
Wrong-commit, stale-reused, forged, unresolved, ineligible, omitted, duplicate,
reordered, or omitted-reset evidence disqualifies the set. Cold qualification
is exactly `C(row) = Q(row) && cache_restored == 0 && four complete durations
from the same resolved slice job IDs`.

From W3, validate the closed normative-site and executable-predicate inventory
before auditing evidence. This list is explicit rather than globbed so a new
second predicate is a review-visible contract change:

```bash
set -euo pipefail
perl -Mstrict -Mwarnings -e '
  my $root = "specs/003-adr052-bazel-rust";
  my @sites = (
    ["docs/adr/0052-bazel-rust-build-and-test.md",
     "The canonical qualification predicate is `Q`"],
    ["docs/adr/0052-bazel-rust-build-and-test.md",
     "The streak is the length of the maximal suffix"],
    ["docs/adr/0052-bazel-rust-build-and-test.md",
     "Cold-sample qualification is exactly"],
    ["docs/adr/0052-bazel-rust-build-and-test.md",
     "The complete authoritative lineage through the"],
    ["docs/adr/0052-bazel-rust-build-and-test.md",
     "qualification now retains the complete eligible"],
    ["docs/adr/0052-bazel-rust-build-and-test.md",
     "A laundered equivalence streak."],
    ["docs/adr/0052-bazel-rust-build-and-test.md",
     "Section 9'\''s canonical `Q` is the only qualification predicate."],
    ["docs/adr/0052-bazel-rust-build-and-test.md",
     "packages/xtask/tests/bazel_qualification.rs"],
    ["$root/contracts/shadow-promotion-evidence.md",
     "ADR 0052 section 9'\''s `Q(row)` table is the single canonical"],
    ["$root/contracts/shadow-promotion-evidence.md",
     "C(row) = Q(row) && cache_restored == 0"],
    ["$root/contracts/shadow-promotion-evidence.md",
     "maximal suffix contains at least ten true-`Q` rows"],
    ["$root/contracts/shadow-promotion-evidence.md",
     "missing policy, missing fixture, failed, cancelled, wrong commit"],
    ["$root/contracts/cache-workflow-boundaries.md",
     "ADR 0052 section 9'\''s canonical"],
    ["$root/contracts/execution-manifest-binding.md",
     "ADR 0052 section 9'\''s"],
    ["$root/contracts/make-target-compatibility.md",
     "bazel-evidence qualification [--check]"],
    ["$root/spec.md", "one canonical `Q` predicate. No requirement"],
    ["$root/spec.md", "**FR-045**: Promotion MUST be blocked"],
    ["$root/spec.md",
     "**Qualification Record**: One push event on protected `v3`"],
    ["$root/spec.md",
     "**SC-002**: The authoritative complete eligible lineage"],
    ["$root/spec.md",
     "**SC-008**: A recorded feasibility measurement"],
    ["$root/spec.md",
     "Fixture-contract and `make test-policy` verdicts both pass"],
    ["$root/research.md",
     "canonical `Q` predicate over a complete authoritative lineage"],
    ["$root/research.md",
     "the continuous-integration set is the five most recent canonical"],
    ["$root/research.md",
     "a complete resolver-confirmed eligible lineage ending in at least ten"],
    ["$root/research.md",
     "The resolver retains every eligible merged-PR push"],
    ["$root/data-model.md",
     "Derived only by ADR 0052 section 9'\''s canonical `Q(row)`"],
    ["$root/data-model.md",
     "A real eligible push with any missing, failed, timed-out"],
    ["$root/data-model.md",
     "Exactly `C(row) = Q(row) && cache_restored == 0"],
    ["$root/data-model.md",
     "Complete eligible lineage from the merged W3 anchor"],
    ["$root/plan.md", "canonical `Q` is the only predicate"],
    ["$root/plan.md", "Five most recent canonical `C` rows"],
    ["$root/plan.md",
     "resolver records immutable run/job IDs and authoritative evidence"],
    ["$root/plan.md",
     "Resolve the complete chronological eligible lineage"],
    ["$root/plan.md",
     "the complete-lineage digest, every immutable run/job"],
    ["$root/plan.md",
     "The equivalence streak is laundered by pairing another commit'\''s job"],
    ["$root/tasks.md", "canonical `Q` is the only qualification predicate"],
    ["$root/tasks.md",
     "fixtures/qualification/` for missing policy, missing fixture"],
    ["$root/tasks.md",
     "Resolve and retain under `.scratch/adr052-w4/records/`"],
    ["$root/tasks.md",
     "Select the five most recent canonical `C(row)`"],
    ["$root/tasks.md",
     "compute every threshold only through `qualification_decision`"],
    ["$root/tasks.md",
     "all ten named negative fixture classes"],
    ["$root/quickstart.md",
     "canonical `Q` predicate over the complete chronological", 2],
    ["$root/quickstart.md",
     "Cold qualification is exactly `C(row) = Q(row)", 2],
    ["$root/quickstart.md",
     ".qualification_records[-1].derived_streak >= 10", 2],
    ["$root/quickstart.md",
     "all(.qualification_records[-10:][]; .qualifies and", 3],
  );
  my @executables = (
    ["packages/xtask/src/bazel_qualification.rs",
     "pub fn qualification_decision"],
    ["packages/xtask/tests/bazel_qualification.rs",
     "qualification_negative_omitted_reset"],
    ["packages/d2b-contract-tests/tests/policy_docs.rs",
     "qualification_normative_sites_are_closed"],
    ["packages/xtask/tests/policy_ci.rs",
     "pr_bazel_rust_workflow_is_observation_only"],
  );

  for my $site (@sites) {
    my ($artifact, $needle, $expected) = @$site;
    $expected //= 1;
    open my $handle, "<:encoding(UTF-8)", $artifact
      or die "$artifact: cannot read normative site: $!\n";
    local $/;
    my $text = <$handle>;
    close $handle or die "$artifact: cannot close: $!\n";
    $text =~ s/\s+/ /g;
    my $count = () = $text =~ /\Q$needle\E/g;
    die "$artifact: qualification site missing or duplicate: $needle\n"
      unless $count == $expected;
  }

  for my $site (@executables) {
    my ($artifact, $needle) = @$site;
    open my $handle, "<:encoding(UTF-8)", $artifact
      or die "$artifact: cannot read executable site: $!\n";
    local $/;
    my $text = <$handle>;
    close $handle or die "$artifact: cannot close: $!\n";
    my $count = () = $text =~ /\Q$needle\E/g;
    die "$artifact: executable predicate site missing or duplicate\n"
      unless $count == 1;
  }

  print "qualification normative/executable consistency: PASS\n";
'

make test-rust-main
make test-policy
```

`make test-rust-main` must report the passing complete-lineage fixture and
independent missing-policy, missing-fixture, failed, cancelled, wrong-commit,
stale-reused-job, forged-ID, ineligible-event, omitted-eligible-push, and
omitted-reset negatives from
`packages/xtask/tests/fixtures/qualification/`. `make test-policy` must report
the closed normative-site inventory. The failed fixture is parameterized over
Cargo, Bazel, policy, fixture, and all four slices; the cancelled fixture
includes a fully cancelled eligible push. Each fixture is schema-valid and
reaches its named canonical decision rather than failing common parsing.

Before the cold ceiling binds, record the W3 feasibility measurement with all
four authoritative slice job IDs and durations, require that Cargo, Bazel,
policy, fixture, and all slices passed at the same head, and derive that
record's duration as their maximum. A single feasibility run does not claim a
median. If its critical-path duration shows the ceiling is not plausible, the
only authorized answers are a larger runner class or a further disjoint slice
split.

Audit the streak and shadow cache behavior:

```bash
cargo xtask bazel-evidence qualification --check \
  specs/003-adr052-bazel-rust/evidence/qualification.json

jq -e '
  .lineage_complete and
  .lineage_order == "first-parent-chronological" and
  .lineage_resolver_digest != "" and
  (.qualification_records | length) >= 10 and
  (.qualification_records[-1].derived_streak >= 10) and
  all(.qualification_records[];
    . as $record |
    .source_event == "push" and
    .branch == "refs/heads/v3" and
    .merged_pr.merged and
    .merged_pr.base_ref == "v3" and
    .merged_pr.merge_commit_sha == .head_sha and
    (.resolver_evidence.digest | length) > 0 and
    (.slice_jobs | length) == 4 and
    all([
      .cargo_job, .bazel_job, .policy_job, .fixture_job,
      .slice_jobs[0], .slice_jobs[1], .slice_jobs[2], .slice_jobs[3]
    ][];
      .head_sha == $record.head_sha and
      (.workflow_run_id | type) == "number" and
      (.job_id | type) == "number" and
      (.run_attempt | type) == "number")) and
  all(.qualification_records[-10:][];
    .qualifies and
    .cargo_job.conclusion == "success" and
    .bazel_job.conclusion == "success" and
    .policy_job.conclusion == "success" and
    .fixture_job.conclusion == "success" and
    all(.slice_jobs[]; .conclusion == "success") and
    .cache_restored == 0 and .cache_writes == 0)
' specs/003-adr052-bazel-rust/evidence/qualification.json
```

## Promotion evidence audit - W4

```bash
cargo xtask bazel-evidence qualification --check \
  specs/003-adr052-bazel-rust/evidence/qualification.json

jq -e '
  .status == "qualified" and
  .coverage.exact_surface_count == 18 and
  .coverage.unmapped_count == 0 and
  .coverage.carriers_claimed_more_than_once == 0 and
  .coverage.analysis_time_label_check_passed and
  .coverage.out_of_test_completeness_check_passed and
  .lineage_complete and
  .lineage_resolver_digest != "" and
  (.qualification_records | length) >= 10 and
  all(.qualification_records[-10:][];
    .qualifies and
    .policy_job.conclusion == "success" and
    .fixture_job.conclusion == "success") and
  .supply_chain.differing_enforcing_outcomes == 0 and
  .supply_chain.yanked_carrier_landed and
  .supply_chain.yanked_snapshot_key_set_matches_all_three_locks and
  .shadow_cache.publications == 0 and
  .workflow_policy.all_positive_and_negative_controls_pass
' specs/003-adr052-bazel-rust/evidence/qualification.json

make test-bazel-rust
make test-rust
make test-policy
D2B_ENABLE_FIXTURE_BUILD=1 make test-fixture-contracts
```

Do not begin W5 if any evidence field is absent, false, incomparable, or tied
to superseded candidate content.

## Promotion validation and rollback rehearsal - W5

Before merge:

```bash
make layer1-workflow
make test-drift
make check
D2B_ENABLE_FIXTURE_BUILD=1 make test-fixture-contracts
```

After the ordered maintenance and save run, confirm the required context is
still `test-rust`, the synchronous trim completed before both headroom checks,
exactly one protected-`v3` writer published, the output base was not cached,
retired prefixes are absent, and both headroom checks were at most 8 GiB.

Rehearse rollback in a separate worktree after the promotion commit is created
and before Cargo retirement:

```bash
promotion_sha=$(jq -r '.promotion_commit' \
  specs/003-adr052-bazel-rust/evidence/promotion-record.json)
git revert --no-commit "$promotion_sha"
make test-rust
D2B_ENABLE_FIXTURE_BUILD=1 make test-fixture-contracts
```

The rehearsal must restore Cargo authority without reconstructing deleted
behavior. Discard the entire rehearsal worktree afterward; do not merge it.

## Independent post-promotion checks - W6 and W7

W6 removes aliases using only release containment:

```bash
promotion_sha=$(jq -r '.promotion_commit' \
  specs/003-adr052-bazel-rust/evidence/promotion-record.json)
test -n "$(git tag --contains "$promotion_sha")"

jq --arg sha "$promotion_sha" -e '
  .promotion_commit == $sha and
  .alias_removal_eligible and
  (.release_tags | length) >= 1
' specs/003-adr052-bazel-rust/evidence/post-promotion.json
```

Then remove only Bazel-specific aliases:

```bash
make test-rust
make test-rust-main
make test-rust-api-surface
make test-rust-broker
make test-rust-guest-shell-runner
make test-rust-no-bash-ast
make test-rust-schema
make test-rust-inventory
make test-rust-supply-chain
make test-policy
make check-tier0
D2B_ENABLE_FIXTURE_BUILD=1 make test-fixture-contracts
```

W6 owns `Makefile`, `AGENTS.md`, and `docs/contributing/`. The affected
fixture-independent policy binaries run under `make test-policy`; the separate
fixture command remains only for the fixture-dependent contract and CLI
surfaces. Neither verdict substitutes for the other.

W7 retires the eighteen Cargo implementations using only the independent
green-run clock:

```bash
promotion_sha=$(jq -r '.promotion_commit' \
  specs/003-adr052-bazel-rust/evidence/promotion-record.json)
jq --arg sha "$promotion_sha" -e '
  .promotion_commit == $sha and
  .cargo_retirement_eligible and
  .consecutive_green_count >= 10 and
  (.green_run_ids | length) >= 10
' specs/003-adr052-bazel-rust/evidence/post-promotion.json
```

Then run the retirement validation. Every public name below must still exist
and must invoke a Bazel carrier; retirement removes implementations, never
names:

```bash
make check
make test-rust
make test-rust-main
make test-rust-api-surface
make test-rust-broker
make test-rust-guest-shell-runner
make test-rust-no-bash-ast
make test-rust-schema
make test-rust-inventory
make test-rust-supply-chain
D2B_ENABLE_FIXTURE_BUILD=1 make test-fixture-contracts
make test-policy
make test-drift
```

Do not cite container, VM, live-host, hardware, or deployed-host tiers for this
feature. They do not cover this internal build scheduler.
