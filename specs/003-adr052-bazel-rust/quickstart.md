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

## Amended ADR verification - before W0

The ADR 0052 amendment is a merged prerequisite, not work this feature
performs. Before any W0 branch is created, prove the amended record is present
in the base by content, not by a remembered commit hash:

```bash
fail() { printf '%s\n' "$*" >&2; exit 1; }
grep -q '^- Status: Accepted$' docs/adr/0052-bazel-rust-build-and-test.md \
  || fail 'ADR 0052 is not accepted'
grep -q '^- Amended: 2026-08-03\.' \
  docs/adr/0052-bazel-rust-build-and-test.md \
  || fail 'ADR 0052 amendment is missing'
grep -q 'yanked' docs/adr/0052-bazel-rust-build-and-test.md \
  || fail 'ADR 0052 yanked carrier is missing'
grep -q 'bazel-repin' docs/adr/0052-bazel-rust-build-and-test.md \
  || fail 'ADR 0052 repin contract is missing'
grep -q '0052-bazel-rust-build-and-test.md' docs/adr/README.md \
  || fail 'ADR 0052 index row is missing'
```

The record must be `Status: Accepted`, must carry the 2026-08-03 amendment
line, must already name protected `v3` as the promotion,
cache-maintenance, cache-publication, streak, and post-promotion lineage, with
the cold measurement set drawn from qualifying cold qualification records, must
state that the section 6 yanked carrier lands unconditionally, and must name
`cargo xtask bazel-repin` as the one supported hub-lock regeneration path.

The ADR names no command for the module lock; it requires only that
`--lockfile_mode=error` fail "with a named remediation". This feature supplies
that name as `cargo xtask bazel-module-refresh`, which is an addition inside
the ADR's own terms rather than a change to them, so no further amendment is a
prerequisite here.

Resolve the amendment commit from history rather than pasting a hash, so the
check keeps working after any rebase or backport:

```bash
git rev-list --reverse HEAD -- docs/adr/0052-bazel-rust-build-and-test.md \
  | while read -r sha; do
      if git show "$sha:docs/adr/0052-bazel-rust-build-and-test.md" \
           | grep -q '^- Amended: 2026-08-03\.'; then
        printf '%s\n' "$sha"
        break
      fi
    done
```

Record that resolved SHA as evidence and require it to be an ancestor of the
W0 base. If any of `.bazelversion`, `.bazelrc`, `.bazelignore`, `MODULE.bazel`,
`MODULE.bazel.lock`, `bazel/`, or generated `packages/**/BUILD.bazel` exists in
a commit that predates it, stop.

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

The last one is not optional and is easy to skip, because a W0 diff looks like
build configuration. `tests/test-rust.sh` excludes `d2b-contract-tests` from
every workspace leaf, and that crate reads `Makefile`, `flake.nix`,
`packages/Cargo.toml`, `packages/xtask/src/main.rs`, and every `.rs` file under
`packages/`. Without this target, W0 changes those files with no coverage at
all. The same applies to every later code-changing wave.

Review the schema result for two independent generations, each containing the
exact generated nonempty valid JSON census before comparison. The census is the
manifest the generator returns, not a number written by hand; a review that
checks a literal is checking the wrong thing.

Check the pinning and boundary invariants that are easy to get wrong quietly:

```bash
fail() { printf '%s\n' "$*" >&2; exit 1; }

# .bazelrc carries no startup line and no channel flag.
if grep -qE '^[[:space:]]*startup[[:space:]]' .bazelrc; then
  fail '.bazelrc contains a startup option'
fi
if grep -q 'rust/toolchain/channel' .bazelrc; then
  fail '.bazelrc sets the global Rust channel'
fi

# Both module-graph checks fail closed rather than warn.
grep -q '^common --lockfile_mode=error$' .bazelrc \
  || fail 'lockfile mode is not fail-closed'
grep -q '^common --check_direct_dependencies=error$' .bazelrc \
  || fail 'direct dependency checks are not fail-closed'

# All four hubs declare a Bazel-side lock, a Cargo lock, and the overwrite opt-out.
test "$(grep -c 'lockfile' MODULE.bazel)" -ge 4 \
  || fail 'fewer than four hub locks are declared'
test "$(grep -c 'cargo_lockfile' MODULE.bazel)" -eq 4 \
  || fail 'Cargo lock declarations do not match four hubs'
test "$(grep -c 'skip_cargo_lockfile_overwrite = True' MODULE.bazel)" -eq 4 \
  || fail 'Cargo overwrite opt-out does not match four hubs'

# No repin escape hatch anywhere on the gate path.
if grep -rqE 'CARGO_BAZEL_REPIN|CARGO_BAZEL_REPIN_ONLY|(^|[^A-Z_])REPIN=' \
  Makefile .github/workflows/; then
  fail 'repin control is reachable from Make or CI'
fi

# None of the five contributor-only commands is reachable from Make or CI.
contributor_only='bazel-repin|bazel-module-refresh|bazel-yanked-refresh'
contributor_only="$contributor_only|bazel-yanked-check|bazel-evidence"
if grep -rqE "xtask ($contributor_only)" Makefile .github/workflows/; then
  fail 'contributor-only xtask command is reachable from Make or CI'
fi

# The only site that may assign a repin control to a process environment.
assignments="$(
  grep -rnE '\.env\("(CARGO_BAZEL_REPIN|CARGO_BAZEL_REPIN_ONLY|REPIN)"' \
    packages/ | cut -d: -f1 | sort -u
)"
test "$assignments" = 'packages/xtask/src/bazel.rs' \
  || fail 'repin assignment exists outside packages/xtask/src/bazel.rs'

# The only site that may set one process-globally is nowhere.
if grep -rqE 'set_var\("(CARGO_BAZEL_REPIN|CARGO_BAZEL_REPIN_ONLY|REPIN)"' \
  packages/; then
  fail 'repin control uses process-global mutation'
fi

# The workspace boundary covers scratch and every Cargo output directory.
grep -q '^\.scratch/$' .bazelignore \
  || fail '.bazelignore omits .scratch/'
```

The third `grep -c` must report four, not zero: at `rules_rust` 0.73.0
`skip_cargo_lockfile_overwrite` defaults to false, so a repin would otherwise
rewrite the authoritative `Cargo.lock`. The assignment grep must print exactly
`packages/xtask/src/bazel.rs`. Grepping for the bare variable names instead
would also match `packages/xtask/tests/policy_ci.rs`, which necessarily
contains all three literals because it is the guard that refuses them, so the
check is on the assignment form, which is what the rule is actually about.

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
grep -rq 'cargo xtask bazel-module-refresh' packages/xtask/src/bazel.rs
```

Confirm neither lock command is reachable from a build entry point:

```bash
if grep -rqE 'xtask (bazel-repin|bazel-module-refresh)' \
  Makefile .github/workflows/; then
  printf '%s\n' 'lock regeneration is reachable from Make or CI' >&2
  exit 1
fi
```

Each of the two commands refuses an exported repin control, and each refusal
ends on the command that was refused rather than on a shared template. Check
both, because a single templated remedy would have to name a `--hub` that
`bazel-module-refresh` never takes:

```bash
# Plant a value nothing else could have produced. The digit 1 is not a
# sentinel: it occurs in version strings, counts, and paths, so asserting
# its absence asserts nothing.
ambient_value='D2B-AMBIENT-SENTINEL-a41f7c'

fail() { printf 'FAIL: %s\n' "$1" >&2; exit 1; }

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

# Each refusal ends on the command that was refused.
printf '%s\n' "$repin_out" \
  | grep -qF 'then run `cargo xtask bazel-repin --hub main`.' \
  || fail 'the bazel-repin refusal lost its own remedy'
printf '%s\n' "$refresh_out" \
  | grep -qF 'then run `cargo xtask bazel-module-refresh`.' \
  || fail 'the bazel-module-refresh refusal lost its own remedy'

# The module-lock refusal must not name the hub command at all. The match
# is literal and case-sensitive, so the `CARGO_BAZEL_REPIN` that refusal
# does name is not a false positive.
if printf '%s\n' "$refresh_out" | grep -qF 'bazel-repin'; then
  fail 'the bazel-module-refresh refusal names bazel-repin'
fi

# Neither refusal may echo back the value it refused.
if printf '%s\n' "$repin_out" | grep -qF "$ambient_value"; then
  fail 'the bazel-repin refusal echoed the ambient value'
fi
if printf '%s\n' "$refresh_out" | grep -qF "$ambient_value"; then
  fail 'the bazel-module-refresh refusal echoed the ambient value'
fi
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

Every check in that block is an explicit guard rather than a bare `!` line.
Measured on bash 5.3.9: a command whose value is inverted with `!` is exempt
from `set -e`, so under `set -e` a bare `! grep -q ...` neither stops the
script nor prints anything, and a leak would pass unnoticed.
`grep ... && exit 1` is worse: when the grep correctly finds nothing, the
`&&` list itself reports non-zero, so the block ends in apparent failure
exactly when it passed.

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
baseline. Validate fixture coverage separately:

```bash
D2B_ENABLE_FIXTURE_BUILD=1 make test-fixture-contracts
```

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
- broker one-process-per-binary topology with exclusivity;
- exact governed-source, runfiles, and parsed-file equality;
- two independent schema generations against the generated census;
- the nightly API census, including the toolchain version the action actually
  used compared against the committed pin;
- binary existence, executability, and identity through the dual-mode locator.

Run the carriers:

```bash
make test-bazel-rust-main
make test-bazel-rust-api
make test-bazel-rust-broker
make test-bazel-rust-aux
```

Inspect the per-case evidence rather than trusting a green target. For a
carrier under the local output tree, confirm the result document exists, names
individual cases, keeps ignored cases distinct, and carries none of the
forbidden values, while raw output remains reachable in `test.log`:

```bash
find .scratch/bazel -name test.xml -path '*ci/rust*' | head
find .scratch/bazel -name test.log -path '*ci/rust*' | head
```

Two negative controls matter more than the positive ones:

- the planted stale-binary fixture, which puts an out-of-date executable at the
  Cargo path, removes the runfiles entry, runs under Bazel, and must fail
  naming the expected runfiles path rather than passing against the stale
  binary;
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
# The trait and its single networked implementation live together.
grep -q 'trait YankedIndex' packages/xtask/src/bazel_yanked.rs
grep -q 'struct IndexClient' packages/xtask/src/bazel_yanked.rs

# Only the refresh and its routing seam name the networked implementation.
grep -rn 'IndexClient' packages/xtask/src/ | cut -d: -f1 | sort -u

# Every refresh case runs against an injected fake, so this passes offline.
cargo test -p xtask bazel_yanked
```

The `grep -rn` must print exactly `packages/xtask/src/bazel_yanked.rs` and
`packages/xtask/src/main.rs`, and nothing on the validator's path may name
`YankedIndex` or `IndexClient`. The `cargo test` line is the real check: every
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
  .locator_migration.stale_binary_fixture_failed_under_bazel
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
filesystem cases: link and anchored-escape refusal, creation ownership,
short-write and collision handling, sync before rename, close-on-exec, and
child-reap ordering.

Every one of those states is supplied through an injected boundary, not
arranged on the host. Cleanup and the result writer share the `FileSystem`
trait in `packages/d2b-bazel-runner/src/fsops.rs`, and the deadline path takes
`Clock` and `UptimeSource` from `packages/d2b-bazel-runner/src/clock.rs`. When
reviewing W2, check that no test fills a disk, requires a privileged mount,
sleeps to reach an expiry, or reads the host clock; a test that does is not
reproducible and will be disabled later:

```bash
if grep -rnE 'thread::sleep|SystemTime::now|/proc/uptime' \
  packages/d2b-bazel-runner/tests/; then
  printf '%s\n' 'runner tests depend on ambient time or uptime' >&2
  exit 1
fi
grep -rn 'fsops::' packages/d2b-bazel-runner/src/cleanup.rs | head
grep -rn 'clock::' packages/d2b-bazel-runner/src/deadline.rs | head
```

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
      .source_event == "push" and .branch == "v3" and
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

Evidence comes only from qualification records: push events on
`refs/heads/v3` produced by merged pull requests, where the Bazel run and the
required Cargo run carry the same head commit. A pull-request run is
diagnostic and produces no record, because its merge ref is recomputed against
a moving base and its path filter excludes exactly the changes a divergence
would appear in.

Before the cold ceiling binds, record the W3 feasibility measurement with all
four slice durations. If it does not clear the ceiling, the only authorized
answers are a larger runner class or a further disjoint slice split.

Audit the streak and shadow cache behavior:

```bash
jq -e '
  (.qualification_records | length) >= 10 and
  ([.qualification_records[-10:][] |
    (.source_event == "push") and
    (.branch == "v3") and
    (.cargo_run_head_sha == .bazel_run_head_sha) and
    (.cargo_verdict == .bazel_verdict) and
    (.fixture_verdict == "passed") and
    (.cache_writes == 0)] | all)
' specs/003-adr052-bazel-rust/evidence/qualification.json
```

## Promotion evidence audit - W4

```bash
jq -e '
  .status == "qualified" and
  .coverage.exact_surface_count == 18 and
  .coverage.unmapped_count == 0 and
  .coverage.carriers_claimed_more_than_once == 0 and
  .coverage.analysis_time_label_check_passed and
  .coverage.out_of_test_completeness_check_passed and
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

W6 owns `Makefile`, `AGENTS.md`, and `docs/contributing/`, all of which the
`d2b-contract-tests` crate reads, so the fixture-contract target is part of W6
validation and not an optional extra.

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
