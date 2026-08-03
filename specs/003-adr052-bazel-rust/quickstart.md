# Quickstart: Validate the ADR 0052 Migration

This guide is for implementation and review waves. Commands marked with an
earliest wave do not exist before that wave lands. Run validation only from a
scope-owned worktree on a committed tree.

## Prerequisites

```bash
export D2B_WORKTREE=/absolute/path/to/d2b-adr0052-spec
cd "$D2B_WORKTREE"
git status --short --branch
nix develop
rustc --version
cat packages/d2b-api-surface/rust-toolchain.toml
```

Expected stable Rust is 1.97.0 and the API pin is
`nightly-2026-02-16`. From W0:

```bash
bazel --version
cat .bazelversion
make test-drift
```

Both Bazel versions must be 8.6.0. Drift must fail rather than rewrite a lock.
Do not use Bazelisk, direct Bazel workflow commands, a remote cache, or a
shared worktree output tree.

## Branch-authority gate - before W0

W0 must not start until a standalone ADR 0052 amendment has merged. The
amended decision must name protected `v3` as the promotion, maintenance,
publication, shadow-streak, and post-promotion lineage. It must replace the
weekly default-branch cold profile with the five most recent qualifying Bazel
shadow runs for PRs merged into `v3`.

## Foundation validation - W0

```bash
make check-tier0
make test-lint
make test-rust-schema
make test-rust-inventory
make test-drift
make test-policy
```

Review the schema result for two independent generations, each with exactly
twenty nonempty valid JSON files before comparison. Confirm `make test-rust`
still invokes Cargo and remains authoritative:

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
' .scratch/adr052-bazel-pass.json
```

Compare exact IDs with the committed coverage map through the aggregate guard,
then independently keep adjacent enforcing surfaces green:

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

## Census, topology, and planted failures - W1 and W4

The W1 aggregate checks:

- exact per-binary test and ignored-case census;
- derived nonempty doctest and harness-free companions;
- main and guest one-process-per-case topology;
- broker one-process-per-binary topology with exclusivity;
- exact governed-source/runfiles/parsed-file equality;
- exact two-by-twenty schema census;
- nightly API and pinned-test inventories;
- binary existence, executability, and identity.

Run the carriers:

```bash
make test-bazel-rust-main
make test-bazel-rust-api
make test-bazel-rust-broker
make test-bazel-rust-aux
```

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

Audit topology and broker repetition:

```bash
jq -e '
  (.topology_proofs | length) == 5 and
  all(.topology_proofs[];
    .census_matches and .ignored_census_matches and .shell_free) and
  (.broker_repetitions | length) == 3 and
  all(.broker_repetitions[];
    .exclusive and .consecutive_passes == 20)
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
```

The targeted tests must exercise both cleanup syscall routes, descriptor
inheritance, replacement race, dedicated process group, fixed grace, final
SIGKILL, sibling survival, deadline grammar/rounding, stuck shutdown, message
redaction, and every wrong-remedy mutation.

Shutdown uses only the approved entry point:

```bash
make bazel-shutdown
```

On `D2B-BZLSERVER-STUCK`, close other clients and rerun that command. Do not
delete `.scratch/bazel/` or signal a PID by hand.

## Warm and cold local measurements - W2 and W4

Use three dedicated measurement worktrees based on the same W4 candidate.
Ensure no heavy lane is active. In each worktree, independently perform the
complete warm sequence: prime successfully, append exactly one comment line
to `packages/d2b-core/src/lib.rs`, and immediately measure without shutdown,
cleanup, or another command between the prime and edit:

```bash
make test-bazel-rust
printf '\n// ADR 0052 warm measurement\n' \
  >> packages/d2b-core/src/lib.rs
time make test-bazel-rust
```

Discard each measurement worktree after recording its result. Record
output-root size before and after. Median must be at most 600 seconds and every
sample at most 720.

For each cold-local sample, retain a populated repository cache while creating
a fresh output user root and empty action cache:

```bash
cargo xtask bazel-evidence prepare-cold-local
time make test-bazel-rust
```

The evidence-only preparation helper exists from W2 through W4 and is removed
in W5. Run three samples. Median must be at most 900 seconds and every sample
at most 1080. A cleanup, hard-limit refusal, server restart during a warm
sample, wrong cache state, or heavy-lane overlap invalidates and replaces the
sample.

Audit all performance sets:

```bash
jq -e '
  (.performance_sets | length) == 3 and
  all(.performance_sets[]; .valid == true) and
  (.performance_sets[] | select(.profile == "warm-local")
    | .median_seconds <= 600 and .maximum_seconds <= 720) and
  all(.performance_sets[] | select(.profile != "warm-local");
    .median_seconds <= 900 and .maximum_seconds <= 1080) and
  (.performance_sets[] | select(.profile == "cold-ci") |
    (.sample_refs | length) == 5 and
    all(.sample_refs[]; .branch == "v3" and .merged == true))
' specs/003-adr052-bazel-rust/evidence/qualification.json
```

## Shadow workflow - W3 and W4

Run all local slices first:

```bash
make test-bazel-rust-main
make test-bazel-rust-api
make test-bazel-rust-broker
make test-bazel-rust-aux
make test-rust-main
make test-policy
make test-lint
```

Review the shadow run rather than treating green as sufficient. It must be
non-required, use four slices and a rollup, restore/save nothing, call only
Make, use credentialless checkout, and grant PR jobs only `contents: read`.
Cold-CI qualification uses the complete slowest-slice job duration from each
of the five most recent qualifying runs for PRs merged into `v3`.

Audit ten-run equivalence and shadow cache behavior:

```bash
jq -e '
  (.shadow_runs | length) >= 10 and
  ([.shadow_runs[-10:][] |
    (.branch == "v3") and
    (.merged == true) and
    (.cargo_verdict == .bazel_verdict) and
    (.cache_writes == 0)] | all)
' specs/003-adr052-bazel-rust/evidence/qualification.json
```

## Promotion evidence audit - W4

```bash
jq -e '
  .status == "qualified" and
  .coverage.exact_surface_count == 18 and
  .coverage.unmapped_count == 0 and
  .supply_chain.differing_enforcing_outcomes == 0 and
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

After the ordered maintenance/save run, confirm the required context is still
`test-rust`, exactly one protected-`v3` writer published, output base was not
cached, retired prefixes are absent, and both headroom checks were at most
8 GiB.

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
```

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

Then run the retirement validation:

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
