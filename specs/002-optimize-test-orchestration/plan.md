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
- `test-nix-unit` will run a measured candidate matrix covering a pure
  aggregate, `lix-unit`, `nix-eval-jobs`, consolidated flake evaluation, and
  the tuned current runner, then implement the fastest contract-preserving
  result.
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
│   └── rust-schema-reproducibility
│       └── rust-inventory-and-stub
├── rust-broker
├── rust-guest-shell-runner
├── rust-no-bash-ast
└── rust-supply-chain
```

Required ordering inside lanes remains:

- main workspace: format, clippy, nextest, doctests, discovered
  `harness = false` binaries, and optional fixture/CLI contract layer;
- broker: default, `layer1-bootstrap`, and `fake-backends` passes in sequence;
- schema: two generations followed by the reproducibility comparison;
- supply chain: the three workspace policies with existing retry semantics.

The public override is `D2B_RUST_BUDGET`. Its default is the smaller of
detected logical CPUs and an effective-memory cap. Effective available memory
is the smaller of Linux `MemAvailable` and the remaining finite cgroup v2
`memory.max` or `memory.high` allowance. Effective cgroup usage is
`memory.current` minus the reclaimable `inactive_file` value from
`memory.stat`, clamped at zero. Unlimited cgroup limits fall back to
`MemAvailable`. If `/proc/self/cgroup` reports v2 membership but the effective
controller files cannot be read, automatic sizing fails closed to budget `1`
with a warning. The calculation reserves 2 GiB for the host and budgets 3 GiB
per heavy Rust job.

`D2B_RUST_BUDGET` is a requested positive upper bound and cannot bypass CPU or
memory caps. Invalid values fail with exit status 2 and a static actionable
message requiring a positive integer; the untrusted environment value is not
echoed. If cgroup controller data is unreadable, the warning directs the
operator to fix controller visibility or run outside that constrained
environment. It does not offer an override that could bypass an unknown
boundary.

Top-level Make `-j` flags do not define Cargo concurrency because GNU Make does
not reliably propagate their numeric value through the jobserver environment.
The public target always logs its effective internal lane and Cargo budget and
names `D2B_RUST_BUDGET` as the target-specific control. The internal recursive
Make invocation owns the calculated active-lane cap.

GNU Make's jobserver schedules eligible workspace lanes. Each CPU-heavy lane
has a static relative weight, but its explicit Cargo `--jobs` and nextest
test-thread quota is computed at runtime from `D2B_RUST_BUDGET`. The maximum
number of simultaneous heavy lanes is also capped by the runtime budget, so a
budget of `1` linearizes heavy work. Quotas are distributed deterministically
across the active-lane cap, including remainder jobs, such that the largest
possible active set never sums above the budget. Contract tests exercise
budgets from `1` through the representative-host default. This preserves
per-workspace Cargo limits without making constrained hosts edit the DAG.
Same-target leaves are dependency-ordered as shown above.

Recursive Make recipe lines use `+$(MAKE)` so Make owns jobserver propagation.
Leaf-dispatch recipe lines contain neither `$(MAKE)` nor the `+` prefix and are
therefore ordinary non-submake recipes; GNU Make closes jobserver descriptors
before executing them. Stale descriptor numbers can remain in `MAKEFLAGS`, so
a Bash leaf does not parse or act on those numbers. At leaf entry, before any
setup subprocess is spawned, the dispatcher unsets `MAKEFLAGS`, `MFLAGS`, and
`MAKELEVEL`. Cargo and nextest therefore honor the explicit lane quota, no
jobserver descriptor reaches their children, and recursive Make calls retain
the jobserver.

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

### 3. Nix unit execution is selected by an experiment gate

The plan does not assume that fewer evaluator processes are faster. Nix
evaluation is single-threaded on the representative Lix 2.94.2 host, while
multiple evaluators repeat shared flake work and consume more memory. US2
therefore starts with isolated, committed experiments from one common base:

1. the tuned current process pool as control;
2. one pure-Nix whole-corpus aggregate using the existing non-throwing case
   result data;
3. a `lix-unit --flake` adapter over the fully injected corpus;
4. `nix-eval-jobs` over a dedicated Nix-unit attrset with bounded workers and
   memory;
5. one Lix `nix flake check --no-build --keep-going` that may consolidate the
   Nix-unit and flake evaluation paths;
6. `nix-fast-build` only if parallel realization or log rendering remains
   material after the evaluator experiment.

Candidates may be revised and benchmarked more than once. Each receives the
same source inventory, executed-surface evidence, one priming run, three warm
runs, a cold observation, peak CPU/RSS sampling, empty-discovery failure, and
simultaneous failures in separate shards. Tool acquisition/build time is
reported separately from steady warm execution.

The selected design must be an established external runner/evaluator or a
native Nix expression; no new Bash or custom-code scheduler is permitted. It
must meet the 50% warm target, report all observed failures in one invocation,
retain CI single-shard selection, preserve evaluation-time fail-closed checks,
and avoid unrelated output realization. If no candidate meets all conditions,
retain the current runner and continue the experiment loop rather than landing
a slower or weaker architecture.

The CI selector `D2B_NIX_UNIT_CHECK` remains supported and evaluates exactly
one discovered shard. CI may continue to dispatch one shard per runner.

Shared-corpus normalization is itself measured, not assumed. Candidate
implementations may:

- import each case file once per evaluator;
- derive the complete corpus and each shard as selections from that shared
  case-file map;
- expose result data separately from the final throwing check so the aggregate
  can collect every failing shard;
- evaluate integrity pins and shard coverage from the same shared map;
- keep evaluation-time throws so `--no-build` validation remains fail-closed.

The normalized graph is retained only if the winning candidate demonstrates a
measured benefit. `lix-unit`, unlike upstream `nix-unit`, matches the host's
evaluator and can consume a flake output adapter carrying d2b's injected
module, package, and helper context. Pin and shard integrity remain separate
enforcing checks unless the selected runner proves equivalent coverage.

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
- an execution manifest from the actual aggregate run proves every required
  leaf or Nix check class completed with schema `version = 1` and
  `run_status = "passed"`, so static discovery alone cannot mask a dropped
  lane. Failed and handled-interruption runs atomically publish partial
  diagnostic manifests after removing prior success evidence before dispatch;
- failures from multiple independent lanes are visible in one invocation;
- the target process tree consumes at least 80% of its effective CPU budget
  over the CPU-heavy interval for Rust; Nix measurements combine client
  evaluation and daemon-cgroup build counters, or use baseline-adjusted host
  counters when daemon counters are unreadable. Both Nix modes require an
  externally idle run and invalidate on unrelated daemon or host activity. The
  interval starts when the first nonzero-quota leaf is admitted and ends when
  the last such leaf completes. A lower result requires a measured non-CPU
  bottleneck after viable concurrency is exhausted;
- no active CPU-quota frontier exceeds the effective budget, worker counts
  remain within their declared bounds, peak memory remains within the
  calculated envelope, and no orchestration-attributable OOM occurs. Memory
  PSI `some total` may consume at most 10% and `full total` at most 1% of
  heavy-interval wall time. Swap thrashing is a baseline-adjusted total above
  both 64 MiB and 1 MiB per second. `memory.events` `max` or `high` deltas fail
  when paired with either sustained-stall threshold;
- cold-cache behavior is recorded and any regression is explained, although
  cold-cache reduction is not a merge blocker.

If a candidate exceeds the representative memory envelope, reduce its worker
count or reject it. The experiment continues until one candidate meets the
contract or the evidence shows that the hard target is not achievable without
weakening an invariant.

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
| Tooling | Plan panel round 1 | The research subagent returned no usable output, so the Nix tool survey was repeated directly against upstream documentation and repositories. | Resolved |
| Design | Plan panel round 2 | Cargo quota and Make jobserver ownership needed a second clarification pass to cover dynamic constrained-host budgets and cgroup memory limits. | Resolved |
| Kernel semantics | Plan panel round 3 | The automatic Rust budget needed cache-aware cgroup accounting and explicit jobserver descriptor closure. | Resolved |
| Interface semantics | Plan panel round 4 | GNU Make does not preserve a reliable numeric top-level `-jN` value for leaf parsing, so target-specific control was narrowed to the documented budget variable. | Resolved |
| Descriptor ownership | Plan panel round 5 | Numeric jobserver values in `MAKEFLAGS` can be stale after Make closes the descriptors, so leaves must unset metadata without closing ambiguous descriptor numbers. | Resolved |
| Migration UX | Plan panel round 5 | The changelog task needed to name `D2B_RUST_BUDGET` as the control when top-level Make `-j` does not cap inner Cargo concurrency. | Resolved |
| Measurement scope | Plan panel round 6 | Nix daemon work is outside the client process tree, so Nix resource evidence now uses daemon-cgroup or externally idle baseline-adjusted host counters. | Resolved |
| Metric precision | Plan panel round 6 | CPU, PSI, swap, worker, and memory-event acceptance needed exact counters, thresholds, and safely mocked rejection cases. | Resolved |
| Manifest atomicity | Plan panel round 6 | Concurrent leaves could corrupt or retain stale execution-manifest state, so each target now finalizes run-specific fragments through atomic replacement. | Resolved |
| Nix attribution | Plan panel round 7 | Nix evaluation and daemon builds require combined accounting, and the shared daemon requires contention invalidation even when its cgroup is readable. | Resolved |
| Counter semantics | Plan panel round 7 | CPU interval units and hierarchical task counts needed explicit microsecond and `pids.current` definitions. | Resolved |
| Failure evidence | Plan panel round 7 | A failed or interrupted run must remove the prior requested execution manifest before dispatch so stale success cannot survive. | Resolved |
| Partial evidence | Plan panel round 8 | Failure paths needed atomic partial manifests, pre-evaluation invalidation, zero-interval handling, and explicit PSI composite rejection tests. | Resolved |
| Signal lifecycle | Plan panel round 9 | Partial finalization needed bounded process-group shutdown, secure atomic fragments, temporary cleanup, schema versioning, and explicit interruption tests. | Resolved |
| Evidence path safety | Plan panel round 10 | Manifest ownership needed a persistent lockfile, anchored fd-relative cleanup, Make-owned descriptor closure, injectable shutdown tests, and a binding JSON schema. | Resolved |
| Lock and schema delivery | Plan panel round 11 | The persistent lock needed OFD and close-on-exec semantics, temp storage needed same-filesystem anchored resolution, and schema/docs/changelog had to ship with the first emitter commit. | Resolved |
| Descriptor and release closure | Plan panel round 12 | Every evidence fd needed close-on-exec, parent anchoring had to precede relative lock creation, lock telemetry needed a closed code, and each emitter phase had to reconcile schema/docs/changelog. | Resolved |
| Policy reconciliation | Plan panel round 13 | The lock remedy needed mandatory wording, and later Nix emitter commits needed to carry the schema/prose policy test with their reconciled artifacts. | Resolved |

## Post-Design Constitution Check

The design still passes all pre-research gates. It reduces custom scheduling,
keeps the closed Layer-1 gate set intact, preserves all critical Rust companion
surfaces, preserves CI manifest authority, and keeps Nix evaluation on
`git+file://` references. No exception is introduced.

## Complexity Tracking

No constitution violations require justification.
