# Implementation Plan: ADR 0052 Bazel Rust Gate

**Branch**: `adr052-bazel-rust-spec` | **Date**: 2026-08-02 |
**Spec**: [spec.md](./spec.md)

**Input**: Accepted ADR 0052 and the feature specification in this directory.

## Summary

After the prerequisite ADR 0052 branch-authority amendment, add Bazel 8.6.0
beside the authoritative Cargo Rust gate, preserve the exact eighteen-surface
execution and isolation contract, collect shadow evidence, and promote only
after every mechanical gate passes. Cargo manifests, three Cargo locks, policy
files, and stable/nightly toolchain pins remain authoritative. Promotion
preserves the `test-rust` context and Make interface. Alias removal and Cargo
implementation retirement are later independent changes.

## Technical Context

**Language/Version**: Rust 1.97.0 for the gate and
`nightly-2026-02-16` for the API census.

**Primary Dependencies**: Bazel 8.6.0 from pinned nixpkgs; Bzlmod;
`rules_rust` 0.73.0 as the initial explicit pin, including `crate_universe`,
subject only to the ADR-required Bazel 8.6.0 compatibility measurement;
repository-owned `cargo xtask gen-bazel`, Rust topology runner, deadline
wrapper, cleanup plumbing, and existing Cargo, nextest, cargo-deny,
cargo-audit, RustSec, Make, and workflow generators.

**Storage**: Committed module locks, generated `BUILD.bazel` files,
governed-source and coverage-map artifacts, and migration evidence summaries.
Local output, action cache, and repository cache live only below
`.scratch/bazel/`. Promoted action and download caches are separate; the
output base is never cached.

**Testing**: Existing Layer-1 Rust tests, `test-policy`, `test-drift`,
`test-fixture-contracts`, workflow policy tests, and Cargo Rust gate. New
behavior uses the lowest existing surface. No new shell gate or Layer-1 job.

**Target Platform**: Linux `x86_64-linux`. Local reference: at least 12
physical cores, 32 GiB available RAM, and SSD/NVMe. CI reference:
`ubuntu-latest`, 4 vCPU and 16 GiB.

**Project Type**: Internal monorepo build, test, policy, and delivery
orchestration. No runtime daemon, broker, VM, package, image, fixture, or
release contract changes.

**Promotion Lineage**: Protected `v3`. ADR 0052 must be amended before W0 to
replace repository-default-branch assumptions with `v3`-only promotion,
maintenance, and publication, and to source cold CI evidence from qualifying
merged-PR shadow runs targeting `v3`.

**Performance Goals**: Three warm local runs: median at most 10 minutes and
maximum at most 12. Three cold local runs: median at most 15 and maximum at
most 18. Five most recent qualifying cold shadow runs for PRs merged into
`v3`: median at most 15 and none above 18. A promoted job has a 15-minute total
ceiling, 2-minute checkout allowance, 13-minute post-checkout in-band window,
and 17-minute outer backstop.

**Constraints**: Cargo/toolchain inputs stay authoritative; no Bazel network
action, remote cache/execution, fixture migration, Nix/package/release
migration, new linter/formatter/hook, new Layer-1 job, or new required
context. `D2B_RUST_BUDGET` remains the resource control. Local action cache:
8 GiB/14 days; local repository cache: 2 GiB; output-root soft/hard marks:
20/40 GiB. Promoted action/download snapshots: 4/1 GiB, with repository use
plus planned snapshot at most 8 GiB.

**Scale/Scope**: Exactly eighteen execution-manifest IDs, four CI slices
(`main`, `api`, `broker`, `aux`), three Cargo workspaces and locks, 56 main
workspace members, 205 integration-test files, 912 tracked Rust files, 641
governed no-bash inputs at the ADR measurement, six harness-free targets, and
twenty schemas. Two fixture-backed surfaces remain on Cargo/Nix.

## Constitution Check

### Pre-research gate

| Principle | Result | Basis |
| --- | --- | --- |
| I. Daemon-Only Control Plane | PASS | No service, unit, per-VM work, or runtime path is added. |
| II. Broker-Mediated Privilege | PASS | Validation is unprivileged. Cleanup acts only as the invoking user below `.scratch/`. |
| III. Reasonable Isolation | PASS | Broker suites retain exclusive per-binary topology; main and guest retain per-case processes. |
| IV. Contract-Driven Compatibility | PASS | Execution-manifest v1 is reused unchanged. The ADR branch-authority amendment is a hard pre-W0 gate; generated and Make/context contracts are guarded. |
| V. Test-Layer Discipline | PASS | Existing Rust, policy, drift, and workflow-policy surfaces carry coverage. |
| VI. Panel-Gated Work | PASS | Every wave has plan and diff gates with all ten roles. Green tests waive nothing. This plan declines pipelining. |
| VII. Traceable Artifacts | PASS | Markers stay in planning/commits/PRs. Code waves carry fragments and ASCII hyphens. |

Broker topology, execution evidence, cleanup safety, cache permissions, and
shell-free execution are load-bearing and receive positive and planted
negative guards before shadow use.

### Post-design gate

All seven principles still pass. Phase 1 adds only internal migration
contracts, defers to execution-manifest v1, uses existing Layer-1 carriers,
separates evidence collection from promotion, and blocks implementation until
the ADR branch-authority amendment lands. There is no violation.

## Project Structure

### Planning artifacts

```text
specs/003-adr052-bazel-rust/
|-- plan.md
|-- research.md
|-- data-model.md
|-- quickstart.md
|-- contracts/
|   |-- README.md
|   |-- make-target-compatibility.md
|   |-- coverage-map.md
|   |-- execution-manifest-binding.md
|   |-- shadow-promotion-evidence.md
|   |-- cache-workflow-boundaries.md
|   `-- recovery-deadline.md
|-- evidence/                       # Added from W4; summaries only
|   |-- qualification.json          # W4 immutable qualification record
|   |-- promotion-record.json       # W5 promotion outcome
|   `-- post-promotion.json         # Independent release/run clocks
`-- tasks.md                        # Phase 2; not changed here
```

### Expected implementation locations

```text
.bazelversion
.bazelrc
MODULE.bazel
MODULE.bazel.lock
Makefile
flake.nix                            # Dev-shell tools only
bazel/                               # Repository-owned Bazel rules/helpers
ci/rust/BUILD.bazel                  # ADR-fixed carriers and aggregate
packages/**/BUILD.bazel              # Generated first-party targets
packages/xtask/src/                  # gen-bazel and schema output support
packages/xtask/tests/policy_ci.rs
packages/xtask/tests/fixtures/ci/
packages/d2b-contract-tests/tests/policy_docs.rs
packages/                            # Runner crate path fixed by W0 prep
tests/golden/bazel-rust-coverage.json
tests/test-rust.sh                    # Cargo authority; fixture mode survives
tests/layer1-jobs.json                # Promotion only
tests/ci/layer1-workflow.template.yml # If generator input requires it
.github/workflows/pr-bazel-rust.yml   # Added W3, deleted W5
.github/workflows/pr-l1-static-fast.yml
changelog.d/                         # One unique fragment per code scope
```

`gen-bazel` owns all generated BUILD files and the governed-source manifest.
ADR 0052 fixes labels and ownership, not every helper filename. W0 prep
selects the runner crate and exact helper paths before parallel worktrees open.

## Spec Corrections

| Drift | Canon retained | Treatment |
| --- | --- | --- |
| ADR 0009 names `tests/static.sh`, old flake checks, and Rust 1.94.1. | Current Make DAG, `tests/test-rust.sh`, current checks, Rust 1.97.0. | No code is realigned to ADR 0009. |
| Current schema leaf snapshots `packages/xtask/out`, while generation writes under `docs/reference/schemas/v2`; empty snapshots compare equal. | Record current behavior, not valid reproducibility evidence. | W0 adds `--out-dir`, emitted census, and exact nonempty checks. |
| Workflow prose predates constitution pipelining. | Constitution 2.1.0 controls. | Use stricter serialization. |
| GitHub default branch is `main`, while protected `v3` never merges to `main`; ADR 0052 says default-branch-only for evidence and cache writes. | Repository branch policy and the user-selected `v3` promotion lineage. | A standalone ADR 0052 amendment must land before W0. It defines `v3`-only maintenance/publication and the merged-PR cold-CI sample source. |

There is no eighteen-surface drift: committed code publishes eighteen with
`D2B_SKIP_FIXTURE_BUILD=1` and two fixture surfaces when enabled.

## Wave Graph and Delivery Rules

```text
ADR 0052 amendment -> W0 foundation -> W1 coverage -> W2 safety -> W3 shadow CI
  -> W4 evidence -> W5 promotion
W5 -> W6 alias removal              # release-containment gate only
W5 -> W7 Cargo retirement           # ten-green-run gate only
```

The ADR amendment is a hard pre-W0 gate and runs through the standalone
`d2b-adr` workflow. No implementation branch, generator change, or Bazel
workspace file starts before that amendment is reviewed, indexed, and merged.

Every scope uses its own worktree/branch, commits before validation, and owns
a unique changelog fragment when code changes. The integrator merges scope
commits into one wave branch and opens one wave PR. Shared contracts land in
an integrator prep commit before parallel scopes. Each boundary has a plan
panel and integrated-diff panel with `software`, `test`, `nixos`,
`networking`, `security`, `rust`, `product`, `docs`, `observability`, and
`kernel`. All ten must sign off with no recommendations. Reviewers inspect
supplied evidence and do not rerun validation. Content changes invalidate
prior signoffs.

### W0 - Reversible foundation

**Deliverable**: Pinned Bazel/Bzlmod, Cargo-derived generator, schema output
prerequisite, generated first-party graph, coverage-map structure, and frozen
runner-owner decision. Cargo remains authoritative.

**Ownership**:

- `foundation-tools`: root Bazel files, `flake.nix`, and root Bazel support.
- `generator`: `packages/xtask/src/**`, generator tests, generated outputs.
- `schema`: schema generator/tests and current schema leaf only.
- Integrator prep: `Makefile`, Cargo workspace membership, coverage-map
  format, runner crate selection, and shared changelog folding.

**Validation**: `make check-tier0`, `make test-lint`,
`make test-rust-schema`, `make test-rust-inventory`, `make test-drift`,
`make test-policy`.

**Done when**: Bazel is 8.6.0; lock mode is `error`; toolchain equality and
repin mutations fail; `gen-bazel --check` is clean; both generations report
twenty nonempty valid schemas; Cargo remains authoritative and green; the
ten-role panel and wave PR are sealed and merged.

### W1 - Coverage carriers

**Deliverable**: Eighteen attributed carriers, four slices, shell-free
topology runner, exact censuses, coverage guard, manifest adapter, and the six
ADR Make targets.

**Ownership after prep**:

- `main`: main format/clippy/tests/doctests/harness-free/no-bash/schema/stub/
  pinned labels.
- `api`: API census, nightly binding, snapshots.
- `broker`: three feature carriers and exclusivity.
- `aux`: guest runner and six offline supply-chain carriers.
- `runner`: only the frozen runner crate and tests.
- `coverage`: `ci/rust/BUILD.bazel`, coverage JSON, and guard.
- Integrator prep: `Makefile`, approved targets, manifest adapter boundary.

Generated BUILD outputs remain generator-owned and are regenerated once after
scope merges.

**Validation**: `make test-bazel-rust`, all four slice targets, an aggregate
with `D2B_EXECUTION_MANIFEST=.scratch/adr052-w1-manifest.json`,
`make test-rust`, `make test-policy`,
`D2B_ENABLE_FIXTURE_BUILD=1 make test-fixture-contracts`, `make test-drift`.

**Done when**: every baseline ID has one carrier; no Rust test or fragment is
unmapped; censuses and ignored counts are exact; binary identities pass;
failed/interrupted runs publish partial evidence; runner execution uses no
shell; Cargo is still authoritative; panel and PR are sealed and merged.

### W2 - Operational safety

**Deliverable**: Budget propagation, bounded scratch state, safe cleanup,
deadline/process-group wrapper, per-code recovery, and all guards. An
evidence-only `cargo xtask bazel-evidence prepare-cold-local` helper prepares
the ADR cold-local state without adding a Make target or persistent
contributor control. W5 removes that helper after W4 qualification is
complete.

**Ownership**:

- `process-control`: deadline, process group, wait ordering, shutdown tests.
- `cleanup`: cleanup modules/tests and its `policy_docs.rs` marker block.
- `local-wrapper`: `Makefile`, `.bazelrc`, scratch budgets.
- `recovery`: recovery table/tests only.

Prep first splits stable interfaces if process-control and cleanup would share
a file.

**Validation**: `make test-rust-main`, `make test-policy`,
`make check-tier0`, `make test-bazel-rust`,
`D2B_CLEAN_DRY_RUN=1 make clean`, and planted mutations through their existing
Rust/policy carriers.

**Done when**: cleanup, descriptor inheritance/race, signal order, deadline,
redaction, and wrong-remedy mutations fail; no descendant survives; unrelated
processes survive; 20/40 GiB marks work; panel and PR are sealed and merged.

### W3 - Shadow CI

**Deliverable**: Non-required four-slice workflow with no restore/save,
workflow/cache permission guards and fixtures, and evidence capture. Required
Cargo CI is unchanged.

**Ownership**:

- `shadow-workflow`: `.github/workflows/pr-bazel-rust.yml`.
- `workflow-policy`: `policy_ci.rs` and CI fixtures.
- `target-policy`: approved-target list if not landed in W1.
- Integrator: triggers, paths, and workflow allowlist reconciliation.

**Validation**: `make test-rust-main`, `make test-policy`, `make test-lint`,
`make check-tier0`, four Bazel slices, and one `workflow_dispatch` inspected
for permissions, zero writes, slice verdicts, and rollup attribution.

**Done when**: workflow is non-required and outside `V3_PR_GATE_WORKFLOWS`;
jobs call approved Make targets; PR reachability has only `contents: read`, no
writer or `actions: write`; shadow publishes nothing; policy fixtures pass;
pull-request runs targeting `v3` retain the mergeable commit/run references
needed to identify later merged PRs; panel and PR are sealed and merged.

### W4 - Evidence qualification

**Deliverable**: Reviewed promotion evidence summary, with no executor flip or
cache publication.

**Ownership**: One curator owns the immutable
`specs/003-adr052-bazel-rust/evidence/qualification.json`. No implementation
source.

**Validation**: Audit ten consecutive matching `v3` shadow verdicts;
eighteen isolated seeded failures; exact census/topology/ignored counts;
twenty exclusive broker repetitions; three warm, three cold-local, and last
five qualifying cold-CI shadow runs from PRs merged into `v3`; supply-chain
equivalence; zero shadow cache publication. Run both Rust aggregates, policy,
and fixture contracts on the evidence commit.

**Done when**: the Qualification Evidence Record validates every threshold and
reference, no item is pending, and the load-bearing documentation wave gets
unanimous panel signoff and merges. The committed record is immutable.
Qualification evidence and promotion never combine.

### W5 - Promotion

**Entry**: W4 merged; maintenance code/fixtures green; a pre-merge cache API
audit has complete pagination, no ambiguous prefix, no retired writer run
after the audit, and enough projected headroom. The amended ADR authorizes
protected-`v3` maintenance and publication.

**Deliverable**: Keep `test-rust`, switch eighteen surfaces to Bazel, retain
Cargo fixture mode, replace eight leaves with four slices, delete shadow,
stop old writes, delete authorized retired caches, verify at most 8 GiB, and
publish through one protected-`v3` writer. Remove the W2 evidence-only
cold-local preparation helper after the qualified W4 measurements have been
used.

**Ownership**:

- `promotion-make`: `Makefile`, `tests/test-rust.sh`.
- `promotion-manifest`: `tests/layer1-jobs.json` and generator inputs.
- `cache`: maintenance implementation and fixtures.
- Integrator: regenerate PR workflow, delete shadow, order shared jobs.

**Validation**: `make layer1-workflow`, `make test-drift`, `make check`,
fixture contracts, alias status tests, deadline policy, maintenance dry-run,
and first ordered protected-`v3` maintenance/save run.

**Done when**: context remains `test-rust`; eighteen surfaces use Bazel; two
fixture surfaces use Cargo/Nix; shadow is absent; old and Bazel names forward
with equal status; workflows call no deprecated alias; retired keys are
absent; both headroom checks pass; exactly one writer publishes; panel and PR
are sealed and merged. After the first ordered protected-`v3` run, a W5
follow-up records `promotion-record.json` with the promotion commit, cache
maintenance result, and first promoted verdict. Rollback is reverting W5.

### W6 - Compatibility alias removal

**Deliverable**: Remove only Bazel-specific aliases after release. Keep
authoritative Rust leaf names and Cargo fallback implementation.

**Ownership**: Make/approved-target policy, related contributor docs, unique
fragment. No Cargo leaf deletion.

**Entry**: `promotion-record.json` exists and `git tag --contains` confirms at
least one release contains the promotion commit. The ten-green-run clock and
Cargo retirement state are irrelevant to this entry.

**Validation**: Recheck release containment, then run `make test-rust`, all
authoritative leaf targets,
`make test-rust-main`, `make test-policy`, `make check-tier0`, and workflow
absence checks for removed names.

**Done when**: release containment is rechecked; aliases are absent;
authoritative names retain status; release notes, panel, and PR are complete.

### W7 - Cargo implementation retirement

**Deliverable**: Remove Cargo implementations only for the eighteen surfaces
after ten promoted green runs. Preserve fixture mode and both fixture surfaces.

**Ownership**: `tests/test-rust.sh`, obsolete Make leaf internals, unreachable
Cargo-only plumbing, related docs, unique fragment. Fixture files are
read-only.

**Entry**: `promotion-record.json` exists and ten consecutive promoted `v3`
run IDs are recorded in `post-promotion.json`. Release
containment and compatibility-alias removal are irrelevant to this entry.

**Validation**: `make check`, `make test-rust`, four slices,
`D2B_ENABLE_FIXTURE_BUILD=1 make test-fixture-contracts`,
`make test-policy`, `make test-drift`, and inventory proving only eighteen
Cargo paths disappeared.

**Done when**: ten runs are rechecked; no migrated Cargo implementation
remains; fixture mode passes; coverage stays exact; panel and PR merge.

W6 and W7 are independent children of W5. Either may land first once its own
entry condition holds; neither consumes or weakens the other's condition.

Host, VM, live-host, hardware, and manual deployment tiers do not cover this
internal build feature and are not claimed.

## Risks and Rollback

| Failure | Guard | Rollback |
| --- | --- | --- |
| Sandboxed scan/generator passes empty. | Exact census, manifest, drift check, planted violation. | Revert carrier; Cargo remains authority. |
| Plain `rust_test` weakens isolation. | Topology map, runner census, ignored counts, broker repetition. | Revert W1. |
| Cleanup follows replacement or leaks descriptors. | Descriptor-relative, forced fallback, exec-leak, decoy race tests. | Revert W2; never use raw recursive removal. |
| Timeout kills caller or leaves descendants. | Dedicated-group order and real sibling/descendant tests. | Revert W5 to Cargo. |
| Shadow cache evicts required cache. | Zero cache actions and policy fixtures. | Disable/delete shadow. |
| Promotion deadlocks on cache space. | Stop writes, authorized deletion, pagination, two checks, one writer. | Revert W5; maintenance stays outside Rust verdict. |
| Ceiling is missed. | Fixed samples and in-band deadline. | Stay shadow; only larger runner or disjoint split is authorized. |
| Third-party build script differs. | Cold shadow and Cargo comparison. | Fix declared inputs or stop migration. |

After each merge run `nix-collect-garbage`, prune old system generations per
operator policy, and remove finished worktree targets. Never share
`packages/target` or `.scratch/bazel`. The wrapper reports sizes, enforces
age/size, and refuses unsafe cleanup.

## Delivery Memory

### Deferred findings

| Severity | Subject | Wave | Round | Tracking item |
| --- | --- | --- | --- | --- |
| None | None | None | None | None |

Only LOW/MEDIUM findings may be deferred from round nine. CRITICAL/HIGH block.

### Friction log

| Wave | Category | Impact | Follow-up |
| --- | --- | --- | --- |
| None | None | None | None |

These tables store classification metadata only, never transcripts,
validation output, credentials, or attestations. A category recurring across
three waves becomes a separately filed task.
