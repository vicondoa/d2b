# Implementation Plan: Optimize Test Orchestration

**Branch**: `test-speedup` | **Date**: 2026-08-01 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/002-optimize-test-orchestration/spec.md`

## Summary

Reduce the warm-cache wall time of `make test-rust`, `make test-nix-unit`, and
the local Layer-1 flake path by at least 50% without dropping coverage. Keep
the already-monolithic direct `make test-flake` path within a 20%
non-regression envelope. Local execution will stop mirroring CI's
process-per-shard topology:

- GNU Make will own the Rust dependency graph and bounded parallel lanes.
  Existing shell code will be reduced to leaf execution and environment setup,
  not scheduling.
- `test-nix-unit` will replace its Bash worker pool with one native Nix
  multi-installable invocation over the complete corpus.
- `test-flake` will use one native `nix flake check --no-build --keep-going`
  evaluation locally, followed only by the narrow checks that must be realized.
- The Nix unit corpus will be loaded once per evaluator and shared by the
  aggregate and shard checks instead of being reconstructed for each shard.

CI may retain its current Rust and Nix matrices. The public Make target names,
enforcing classifications, failure behavior, and coverage inventories remain
stable.

## Technical Context

**Language/Version**: Rust 1.97.0, Nix language evaluated by Lix 2.94.2 on the
representative host, GNU Make, and existing Bash leaf drivers

**Primary Dependencies**: Cargo, cargo-nextest, sccache, rustup, GNU Make,
Nix/Lix CLI, nixpkgs

**Storage**: Per-worktree Cargo target directories, shared local sccache,
Nix store, and Nix flake evaluation cache

**Testing**: Existing `make test-rust`, `make test-nix-unit`, `make
test-flake`, `make test-drift`, and targeted meta-policy tests

**Target Platform**: Local Linux development hosts; representative acceptance
host has 12 logical CPUs and 62 GiB RAM. CI remains Linux with its existing
x86_64 and aarch64 split.

**Project Type**: Repository test and build infrastructure

**Performance Goals**: `make test-rust`, `make test-nix-unit`, and the local
Layer-1 flake path each have a median warm-cache elapsed time no greater than
50% of the matching accepted pre-change median over three equivalent runs.
The direct `make test-flake` path may regress by no more than 20%. Cold-cache
elapsed time is measured and minimized but is not a completion gate.

**Constraints**: Preserve all enforcing coverage; keep current Make target
names; preserve explicit doctest and `harness = false` execution; keep broker
feature passes serial where process-global test state requires it; avoid
unrelated Nix builds; keep CI sharding valid; add no new Bash or custom-code
scheduler; do not weaken failure reporting or enforcement.

**Scale/Scope**: One main Rust workspace with more than 50 members, two
excluded standalone Rust workspaces, three broker feature passes, six Nix unit
shards plus their integrity check, and the native-system flake check inventory.

## Constitution Check

*GATE: Passed before research and passed again after design.*

- **Test-Layer Discipline**: The change modifies existing Layer-1 plumbing and
  does not add a new gate or top-level `tests/*.sh` entrypoint.
- **Existing code is canon**: Coverage is derived from the current committed
  scripts, `tests/layer1-jobs.json`, flake outputs, and nextest inventory rather
  than from prose alone.
- **Commit before validation**: Implementation tasks must commit new or moved
  files before any Nix evaluation used as acceptance evidence.
- **No new formatter, linter, or hook**: None is introduced.
- **No broad path-based flake source**: All new or retained Nix test
  invocations use `git+file://` through `d2b_flake_ref`.
- **No unsafe concurrency**: Broker feature passes remain serial. Parallel
  Rust lanes use a single bounded CPU budget and do not concurrently mutate the
  same target directory.
- **CI manifest authority**: Any local environment or CI wiring change updates
  `tests/layer1-jobs.json` and regenerates the workflow rather than editing the
  generated workflow directly.
- **Traceability**: A changelog fragment ships with the implementation, and no
  internal process markers enter shipped text.

No constitution violation or complexity exception is required.

## Project Structure

### Documentation (this feature)

```text
specs/002-optimize-test-orchestration/
├── plan.md
├── research.md
├── data-model.md
├── quickstart.md
├── contracts/
│   └── local-validation-targets.md
└── tasks.md
```

### Source Code (repository root)

```text
Makefile
flake.nix
changelog.d/
tests/
├── AGENTS.md
├── README.md
├── layer1-jobs.json
├── test-rust.sh
├── test-nix-unit.sh
├── test-flake.sh
├── static.sh
├── tools/
│   ├── layer1-jobs.py
│   └── flake-check-classes.sh
└── unit/
    ├── nix/
    │   ├── default.nix
    │   └── cases/
    ├── gates/
    │   └── ci-rust-cache-sync.sh
    └── meta/
        └── ci-runner-regression.py
packages/
├── .config/nextest.toml
└── xtask/tests/
    ├── policy_ci.rs
    └── policy_workspace.rs
```

**Structure Decision**: Keep the stable Make interface and existing test
drivers. Scheduling moves upward into GNU Make or downward into native
Cargo/Nix capabilities. Shell remains only where d2b-specific environment
setup, fixture materialization, secure diagnostics, or an unsupported test
surface makes it necessary.

## Design

### 1. Establish comparable inventories and baselines

Before changing execution, record for each public target:

- the exact enforcing test/check inventory;
- three warm-cache elapsed-time samples after one untimed priming run;
- three best-effort cold-cache samples using the documented cache reset;
- host CPU, memory, tool versions, commit, and cache condition;
- per-phase timing for Rust and the list of Nix installables evaluated or
  realized.

Coverage comparison is baseline-subset preservation, not exact equality. New
tests added for the orchestration change are listed separately and must not
mask a missing baseline item.

For flake validation, record two baselines:

- the legacy local Layer-1 path with `D2B_FLAKE_LOCAL_SHARDS=1`;
- the already-monolithic direct `make test-flake` path.

The hard 50% goal applies to the local Layer-1 path. The direct path is a
non-regression measurement with a maximum permitted slowdown of 20%.

Use `hyperfine` for timing rather than adding a repository benchmark runner.
Cargo's stable `--timings` report is diagnostic evidence for compilation and
linking bottlenecks, not a committed artifact.

### 2. Rust local execution becomes a bounded Make DAG

`make test-rust` becomes the only aggregate entrypoint. It invokes internal
Make targets with `--keep-going` and `--output-sync=target`, so independent
lanes overlap while each lane's output remains grouped and every failed lane is
reported.

The initial DAG is:

```text
test-rust
├── rust-api-surface
├── rust-main-workspace
├── rust-broker
├── rust-guest-shell-runner
├── rust-no-bash-ast
├── rust-schema-reproducibility
├── rust-supply-chain
└── rust-inventory-and-stub
```

Required ordering inside lanes remains:

- main workspace: format, clippy, nextest, doctests, discovered
  `harness = false` binaries, and optional fixture/CLI contract layer;
- broker: default, `layer1-bootstrap`, and `fake-backends` passes in sequence;
- schema: two generations followed by the reproducibility comparison;
- supply chain: the three workspace policies with existing retry semantics.

The aggregate CPU budget defaults to detected logical CPUs. Concurrent
CPU-heavy lanes divide that budget and pass it through Cargo's native
`--jobs`/`CARGO_BUILD_JOBS` and nextest's native test-thread setting. GNU Make
is invoked as a recursive Make command so its jobserver remains available to
Cargo. Lanes that share a target directory never overlap.

`tests/test-rust.sh` becomes a leaf dispatcher and environment provider. Its
serial `all` scheduler is removed. `tests/static.sh` and other callers use the
public Make target rather than bypassing the Make DAG.

The plan retains cargo-nextest, sccache, the aligned development/test profiles,
and the excluded-workspace boundaries. It does not add `just`, `cargo-make`,
nextest partitions, cargo-hakari, or a new Python/Rust scheduler.

Rust 1.97 should already use rust-lld by default on x86_64 GNU/Linux. The
implementation first verifies linker selection and collects Cargo timings. A
new linker dependency is not part of the initial design. If the Make DAG and
duplicate-work removal do not meet the hard target, a measured mold experiment
is the only approved linker contingency; it is adopted only if it materially
improves total target time on the representative host without changing
coverage. "Materially" means at least a 10% improvement in the three-run
whole-target warm median, with no supported-platform build regression.

### 3. Nix unit execution uses one native invocation

The full-corpus path discovers the same `nix-unit*` attr names but passes all
installables to one `nix build --no-link --keep-going` invocation. Nix owns
evaluation reuse, build scheduling, and failure aggregation. The Bash PID pool,
per-shard logs, and `D2B_NIX_UNIT_JOBS` local scheduler are removed.

The CI selector `D2B_NIX_UNIT_CHECK` remains supported and evaluates exactly
one discovered shard. CI may continue to dispatch one shard per runner.

The flake's Nix unit data graph is also normalized:

- import each case file once per evaluator;
- derive the complete corpus and each shard as selections from that shared
  case-file map;
- evaluate integrity pins and shard coverage from the same shared map;
- keep evaluation-time throws so `--no-build` validation remains fail-closed.

This avoids reconstructing the complete corpus for the integrity check and
again for every shard.

The upstream `nix-unit` CLI remains useful for focused author iteration but is
not the enforcing aggregate because the current corpus requires d2b's injected
flake/module context and separate pin/shard integrity guarantees. No new
`nix-unit` wrapper is introduced.

### 4. Flake validation uses one local evaluator

Local execution stops mirroring CI's one-process-per-check matrix.
`D2B_FLAKE_LOCAL_SHARDS` and its Bash scheduler are removed from the local
manifest path.

The public target performs:

1. one native-system `nix flake check --no-build --keep-going` against the
   `git+file://` flake reference;
2. one native multi-installable `nix build --no-link --keep-going` containing
   only checks classified as requiring realization.

CI-specific `D2B_FLAKE_CHECK` and `D2B_FLAKE_OUTPUTS` modes remain unchanged so
the hosted matrix can keep its memory isolation and required contexts.

`nix-eval-jobs` and `nix-fast-build` are not selected for the primary contract:
they parallelize derivation attrsets well, but they do not replace `nix flake
check` output-schema validation, and `nix-fast-build` would realize broader
outputs than this target permits. They remain diagnostic alternatives if a
single evaluator still exceeds a host's memory envelope after the shared Nix
graph is fixed.

### 5. Acceptance and rollback

After each target is changed, compare its inventory and benchmark record with
the baseline before proceeding. A target is accepted only when:

- the Rust, Nix unit, or local Layer-1 flake warm median is at most half the
  matching baseline, or the direct flake path remains within its 20%
  non-regression envelope;
- every baseline enforcing surface remains present and every added test is
  classified separately;
- failures from multiple independent lanes are visible in one invocation;
- no lane causes out-of-memory termination or sustained oversubscription;
- cold-cache behavior is recorded and any regression is explained, although
  cold-cache reduction is not a merge blocker.

If a consolidated Nix evaluator exceeds the representative memory envelope,
restore bounded external evaluation for that target using `nix-eval-jobs`
rather than reintroducing a Bash worker pool.

## Validation Strategy

- Rust DAG and contracts: targeted Make invocations, nextest inventory
  comparison, `packages/xtask` policy tests, and `ci-rust-cache-sync`.
- Nix unit graph: `make test-nix-unit`, pin regeneration check, and failure
  probes for missing, duplicate, and throwing cases.
- Flake graph: `make test-flake`, realized-check coverage, flake matrix drift,
  and the CI runner regression tests.
- Generated CI: `make layer1-workflow-check` after manifest changes.
- Fixture-dependent documentation policy: `make test-fixture-contracts`.
- Final acceptance: three warm samples for each target plus documented cold
  samples, followed by the smallest Layer-1 jobs covering all changed
  infrastructure.

## Panel Gates and Commit Tags

This plan uses strict phase ordering rather than pipelined dispatch.

- Run the ten-seat plan panel before implementation begins.
- After each integrated implementation phase, run a work panel and obtain
  unanimous sign-off before starting the next phase.
- Use `( spec002w1 )` for Rust commits, `( spec002w2 )` for Nix unit commits,
  `( spec002w3 )` for flake commits, `( spec002w4 )` for resource-tuning
  commits, and `( spec002w5 )` for evidence and documentation commits.
- Panel-fix commits append the applicable follow-up and finding suffix using
  the repository's canonical grammar.

## Deferred Findings Register

| Severity | Subject | Owning phase | Deferred round | Status |
|---|---|---|---|---|
| None | None | None | None | Open set is empty |

## Friction Log

| Category | Phase | Impact | Status |
|---|---|---|---|
| None | None | None | Open set is empty |

## Post-Design Constitution Check

The design still passes all pre-research gates. It reduces custom scheduling,
keeps the closed Layer-1 gate set intact, preserves all critical Rust companion
surfaces, preserves CI manifest authority, and keeps Nix evaluation on
`git+file://` references. No exception is introduced.

## Complexity Tracking

No constitution violations require justification.
