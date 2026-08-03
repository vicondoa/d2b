# Test-orchestration baseline results

This record covers only the pre-change baseline at
`d09cba95f7602b70f1f79b957f426ecfacf65bf1` on branch `test-speedup`.
It completes T001-T007. No implementation file was changed and no optimized
acceptance claim is made here.

The optimized Nix-unit measurements later in this record describe the
pre-CI-cold-refinement runner. The refinement recorded at the end changes the
evaluation surface and has not been benchmarked or observed on a hosted
GitHub Actions runner.

## Measurement identity and environment

The complete capture is
`.scratch/test-speedup-baseline/environment.txt`. The relevant values are:

| Item | Baseline |
| --- | --- |
| Commit | `d09cba95f7602b70f1f79b957f426ecfacf65bf1` |
| System | `x86_64-linux`, NixOS 26.11.0 |
| CPU | 12 logical CPUs, Intel Core i9-10920X |
| Memory / swap | 62 GiB RAM / 68 GiB swap |
| Rust / Cargo | `rustc 1.97.0`, `cargo 1.97.0` |
| cargo-nextest | `0.9.136` |
| Nix | Lix `2.94.2` |
| GNU Make | `4.4.1` |
| hyperfine | `1.20.0`, acquired externally with `nix shell` |
| sccache | `0.15.0` |

The initial tracked worktree status was clean. The target drivers acquired
the pinned Rust toolchain and cargo-nextest through their existing bootstrap
paths; hyperfine was not added to the repository. The shared Nix store was
not cleared.

## Inventory and trace artifacts

The inventory commands and their complete captured output are retained in
`.scratch/test-speedup-baseline/test-rust-inventory-commands.log` and
`.scratch/test-speedup-baseline/nix-inventory-commands.log`. The pinned-test
assertion is in `assert-pinned-tests.trace.log`.

| Surface | Inventory | Count | SHA-256 |
| --- | --- | ---: | --- |
| Rust source census | `test-rust-inventory.txt` | 6,085 sorted lines | `0ec385cd722fcd10c05eaf42435f961a75f8b2dbe07dacb2d0e411c14b5edcbb` |
| Rust harness-free companions | `test-rust-harness-free.txt` | 1 (`d2b-core::d2b-core-smoke`) | `7a9695aaddbce24da39e698ede463d53536c899cefe2f2367f4defb918d3934a` |
| Pinned Rust tests | `assert-pinned-tests.trace.log` | 504 tests, 43 files | n/a |
| Nix-unit source checks | `test-nix-unit-inventory.json` | 7 | `c625d1ac0accef72f3f637fbaceeb605a6e6199e0ab7341717c579a3291442a9` |
| Nix-unit sorted names | `test-nix-unit-inventory.txt` | 7 | `5b7c84c88491b11b350d801c6ae1fae212f31ea8286dcda5d5d7853d3ad64843` |
| Flake source checks | `test-flake-inventory.json` | 32 | `4be2a8fc938bc64fbf50e4dc6b893156f3359a42c3dd20469b9442aee45c7809` |
| Flake sorted names | `test-flake-inventory.txt` | 32 | `47e4cf42f8c6e829b248530a976f62884d61e9cb284b9450c86c61de3a853282` |

Every public baseline run was captured with `script` so the target's complete
diagnostic stream is retained. The priming and warm traces are:

```text
.scratch/test-speedup-baseline/traces/test-rust-prime.trace.log
.scratch/test-speedup-baseline/traces/test-rust-warm.trace.log
.scratch/test-speedup-baseline/traces/test-nix-unit-prime.trace.log
.scratch/test-speedup-baseline/traces/test-nix-unit-warm.trace.log
.scratch/test-speedup-baseline/traces/test-flake-direct-prime.trace.log
.scratch/test-speedup-baseline/traces/test-flake-direct-warm.trace.log
.scratch/test-speedup-baseline/traces/test-flake-layer1-prime.trace.log
.scratch/test-speedup-baseline/traces/test-flake-layer1-warm.trace.log
```

The three cold observations for each target have matching
`test-*-cold-{1,2,3}.trace.log` files. All traces ended with exit status 0.

## Cache definitions and invalidation rules

* **Warm:** one untimed priming invocation at the same committed revision,
  followed by three sequential hyperfine samples. Cargo target directories,
  the shared sccache, the Nix store, and the normal evaluator cache were
  retained. The four warm JSON files are the direct hyperfine exports
  `test-rust.json`, `test-nix-unit.json`, `test-flake-direct.json`, and
  `test-flake-layer1.json`.
* **Cold Rust:** before each observation,
  `D2B_CLEAN_SKIP_GC=1 D2B_CLEAN_KEEP_SCRATCH=1 make clean` removed this
  worktree's Cargo targets but retained sccache, followed by the public target.
  `D2B_CLEAN_KEEP_SCRATCH=1` is necessary because the committed clean driver
  otherwise removes the evidence directory itself.
* **Cold Nix:** each observation used a newly created
  `XDG_CACHE_HOME` below
  `.scratch/test-speedup-baseline/cold-cache/`, while retaining the shared
  Nix store. The per-observation cache directories are retained as provenance.
* **Flake comparison:** direct samples used `make test-flake`; legacy local
  Layer-1 samples used
  `D2B_FLAKE_LOCAL_SHARDS=1 make test-flake`.
* A sample is invalid only for a nonzero target status, a revision/cache
  mismatch, or recorded unrelated host/daemon activity. Invalid samples must
  remain recorded with their reason and be repeated; they are never silently
  substituted. No sample in this baseline set was invalidated. The Nix-unit
  SQLite `busy` messages were caused by its own two-worker evaluator pool and
  were retained as an orchestration observation, not hidden or treated as a
  failed target.

## Baseline samples

The priming records are `test-*-prime.json`; the cold aggregates are
`test-*-cold.json`. Warm values are in seconds and preserve hyperfine's sample
order.

| Target | Priming | Warm samples | Warm median | Cold samples | Cold median |
| --- | ---: | --- | ---: | --- | ---: |
| `make test-rust` | 1,173.000 | 413.241605, 324.763168, 315.438021 | **324.763168** | 938.403843, 910.859268, 911.204650 | 911.204650 |
| `make test-nix-unit` | 612.000 | 557.898274, 538.889473, 337.752675 | **538.889473** | 660.019541, 659.634732, 659.700177 | 659.700177 |
| direct `make test-flake` | 604.000 | 560.220703, 565.495060, 575.048672 | **565.495060** | 550.142324, 576.399980, 562.365417 | 562.365417 |
| legacy Layer-1 flake | 451.000 | 489.007330, 472.991949, 471.752824 | **472.991949** | 464.927313, 462.230731, 482.690424 | 464.927313 |

All warm and cold sample exit-code arrays are all zero. The first Rust warm
sample and the third Nix-unit warm sample are retained despite their variance;
the median was not selected by discarding an inconvenient observation.

## Trace-derived execution manifests

The baseline driver at this commit does not implement
`D2B_EXECUTION_MANIFEST`. The following manifests are therefore manually
derived from the completed public-target traces, with
`emitter_support: false`:

| Target | Manifest | Completed leaves | Trace completion |
| --- | --- | ---: | --- |
| Rust | `test-rust-executed.json` | 20 | `traces/test-rust-prime.trace.log:46247` |
| Nix unit | `test-nix-unit-executed.json` | 7 | `traces/test-nix-unit-prime.trace.log:58` |
| direct flake | `test-flake-executed.json` | 1 aggregate leaf | `traces/test-flake-direct-prime.trace.log:32` |
| legacy Layer-1 flake | `test-flake-layer1-executed.json` | 33 (32 checks + packages) | `traces/test-flake-layer1-prime.trace.log:38` |

Each `completed_leaves` entry has a `trace_citations` entry pointing to the
physical trace line that proves completion. Manifest digests are:

```text
test-rust-executed.json       4bea46707e64ca03c46f9151b16877d465e1d3f58a0741ecd2ec169e1d1f52e9
test-nix-unit-executed.json   8a74b80803900461a2cc6341612cf088a89c9567342738e8e1d9ca548650f9b1
test-flake-executed.json      c05cd02707608b821631a23a72151a3b1aab58348a5365f6241e432646946a3e
test-flake-layer1-executed.json
                              5d543cab0ffeb7a952a5bcd8a20f156be3283b8e78401105dbed21141a4b6cc9
```

The planned binding v1 fields are:

| Field | Baseline meaning |
| --- | --- |
| `version` | Integer schema version, `1` |
| `target` | Owning public target |
| `commit` | Exact measured Git revision |
| `run_status` | `passed`, `failed`, or `interrupted` |
| `completed_leaves` | Sorted stable leaves completed successfully |
| `failed_surfaces` | Sorted observed failure identifiers |
| `installables` | Nix attrs actually submitted |
| `realized_checks` | Flake checks actually realized |
| `source_inventory_digest` | Digest of the matching source census |
| `external_contention` | Closed reason code, not free-form process data |

The planned schema path is
`docs/reference/schemas/test-execution-manifest-v1.json`; the planned prose
path is `docs/reference/test-execution-manifest.md`. Neither path exists at
the baseline commit, so the trace-derived records also carry diagnostic
`run_trace`, `trace_citations`, `cache_condition`, `sample`, `command`,
`emitter_support`, and `derivation_note` fields. A future emitter must publish
partial failed/interrupted evidence atomically after removing stale success
evidence; a static source inventory alone is not acceptance evidence.

## Rust leaf ownership and conflicts

The baseline serial order is API surface, main workspace, and remaining suite.
The manually named leaves and their committed ownership are:

| Leaf owner | Coverage | Target directory / state | Conflict or ordering rule |
| --- | --- | --- | --- |
| `rust-api-surface` | Compiler-derived API census | `.scratch/rust-test-cache/api-surface-<pin>/census` | Separate rustdoc target; does not share the main Cargo target |
| `rust-main-format` / `rust-main-clippy` | Format and clippy | `packages/target` | Ordered before workspace tests |
| `rust-main-workspace-tests` | nextest plus doctests and `d2b-core-smoke` companion | `packages/target` | Companion follows nextest; same target directory |
| `rust-contract-tests` / `rust-cli-contract-tests` | Fixture-rendered contract and CLI layers | `packages/target` plus fixture outputs | Reuse the main target; cannot overlap it |
| `rust-no-bash-ast` | AST bash-exec scan | `tests/tools/no-bash-ast-walker/target` | Separate target |
| `rust-broker-default` → `rust-broker-layer1` → `rust-broker-fakebackends` | Three broker feature passes | `packages/d2b-priv-broker/target`, `target-layer1`, `target-fakebackends` | Serial process-global SIGCHLD/reap state despite separate target dirs |
| `rust-guest-shell-runner` | Standalone guest workspace | `packages/d2b-guest-shell-runner/target` | Separate workspace and target |
| `rust-schema-reproducibility` | Two `cargo xtask gen-schemas` runs and comparison | `packages/xtask/out` | The two generations are serial |
| `rust-deny-*` / `rust-audit-*` | Three workspace policy scans | Lockfiles and tool caches | Independent of test execution, serial in baseline |
| `rust-stub-no-socket` / `rust-assert-pinned` | Stub smoke and inventory guard | Workspace/broker targets and pinned files | Runs after the other remaining leaves |

The planned optimized DAG can admit API, main, broker, guest, no-bash, and
supply-chain owners concurrently subject to the target-directory edges. The
broker feature chain must remain serial, and all leaves using
`packages/target` must remain ordered. The baseline does not yet implement
that DAG; the phase and duplicate-work details are in
`.scratch/test-speedup-baseline/rust-analysis.md`.

## Nix installables and realized flake set

The baseline Nix-unit installable set is exactly:

```text
.#checks.x86_64-linux.nix-unit
.#checks.x86_64-linux.nix-unit-daemon
.#checks.x86_64-linux.nix-unit-guest
.#checks.x86_64-linux.nix-unit-misc
.#checks.x86_64-linux.nix-unit-network
.#checks.x86_64-linux.nix-unit-runtime
.#checks.x86_64-linux.nix-unit-state
```

The committed baseline Nix-unit driver submits these through bare
`.#checks...` expressions and realizes each with `nix build --no-link`.
The source inventory itself used the safer `git+file://` reference specified
by the quickstart.

The direct flake target submits one native `git+file://` flake reference to
`nix flake check --no-build`; it has no realized checks. The legacy local
Layer-1 target submits all 32 native check attrs plus the native package
output sweep. Its realized-check set is exactly:

```text
video-binary-contract
```

The complete 32-check source set is retained in
`test-flake-inventory.txt` and is:

```text
eval-fixture-contracts
eval-graphics
eval-minimal
eval-multi-env
eval-multi-env-daemon
eval-template
eval-with-observability
fixture-smoke
fixture-smoke-full
guest-control-vsock
guest-exec-policy
guest-rust-deny
guest-shell-runner-static-dependency-policy
guest-static-consumption
guest-static-dependency-policy
guest-static-elf
harness-ubuntu-skeleton
module-helper-wiring
nix-unit
nix-unit-daemon
nix-unit-guest
nix-unit-misc
nix-unit-network
nix-unit-runtime
nix-unit-state
provider-catalog-determinism
rust-audit
rust-build
rust-clippy
rust-deny
rust-tests
video-binary-contract
```

## Cargo timings and duplicate-work evidence

T004 diagnostics are under `.scratch/test-speedup-baseline/cargo-timings/`.
The isolated command

```text
CARGO_TARGET_DIR=.scratch/test-speedup-baseline/cargo-timings/d2b-target \
  cargo build --manifest-path packages/Cargo.toml -p d2b --bin d2b --timings -vv
```

returned 0 and produced a Cargo report with 109 dirty units, 109 total units,
and 50.2 seconds total diagnostic-build time. An `execve` trace proves the
Rust 1.97 bundled self-contained `rust-lld` path (21 linker executions);
`ld.lld` wrapper executions are also retained. No mold dependency was used.
The full per-phase table, linker line citations, and sccache snapshot are in
`rust-analysis.md`.

The baseline repeats work in ways relevant to the optimization:

* The three broker feature passes compile against separate target directories
  to avoid lock contention, but still repeat feature-specific links and
  remain a serial chain.
* The main target runs nextest and then required doctest/harness-free
  companions; those are separate required surfaces, not removable duplicate
  tests.
* Fixture preparation, the CLI daemon build, schema generation twice, and
  policy/audit scans are all inside the serial baseline aggregate.
* Nix-unit launches one evaluator/build per shard and produced repeated
  ignored SQLite eval-cache-busy messages with its two workers.
* The legacy Layer-1 flake path repeats source evaluation in each child
  process, whereas direct flake validation evaluates one native flake. Only
  `video-binary-contract` is realized in the Layer-1 check class.

## Drift reconciliation and deliberate omissions

Committed code proved the following documentation drift, so T006 updated both
`quickstart.md` and `contracts/local-validation-targets.md` with explicit
baseline notes:

* baseline Rust, Nix-unit, and flake drivers have no execution-manifest
  emitter;
* baseline Rust is serial rather than a bounded Make DAG;
* baseline Nix-unit accepts `D2B_NIX_UNIT_JOBS` and uses bare `.#` attrs;
* baseline direct flake omits an explicit `--keep-going`, and the legacy
  `D2B_FLAKE_LOCAL_SHARDS` scheduler remains available;
* baseline cleanup removes `.scratch` unless
  `D2B_CLEAN_KEEP_SCRATCH=1` is supplied.

No resource-stability counters, failure-injection experiment, optimized
measurement, or 50% acceptance comparison was performed: those belong to
later tasks. No shared Nix store cleanup was performed.

## Exact validation and results

The enforcing public commands used for the baseline were:

```text
make test-rust                                      # priming: 0
make test-nix-unit                                 # priming: 0
make test-flake                                    # priming: 0
D2B_FLAKE_LOCAL_SHARDS=1 make test-flake            # priming: 0
```

Each was followed by the three-run external hyperfine command recorded in its
`test-*.json`, and by three one-run cold commands recorded in the matching
`test-*-cold-{1,2,3}.json`. `bash tests/tools/assert-pinned-tests.sh` returned
0 with all 504 pinned tests present. The execution-manifest `jq` checks and
all source/trace digest checks returned 0. The baseline target set therefore
has no enforcing blocker; it is intentionally a measured pre-optimization
reference, not a passing implementation of the future manifest contract.

## Rust optimized result

The accepted three-run warm benchmark was captured at
`0e563f433ccd41f8a4a57c955679e10fc256cecc`. Final behavior tip
`0775159a46427364f943e6b7a49fd3079cd79c7f` changes only cold/cache policy and
passed the complete warm aggregate with exact evidence in 141 coarse seconds.
GNU Make owns nine bounded `test-rust-leaf-*` nodes, while
`tests/test-rust.sh` owns only explicit leaf execution. The
representative host calculated a 12-job budget and admitted at most nine
lanes. Budgets through nine use one job per lane; the three surplus jobs on
this host are assigned to the measured API long pole, so its two rustdoc
passes receive two Cargo jobs each while the complete runnable frontier stays
within 12.

Two measured target-state changes removed the remaining warm critical path:

* warm-local fixture/CLI work uses
  `.scratch/rust-test-cache/fixture-contracts`, independent of
  `packages/target`, so its Nix evaluation and Cargo work overlap the main
  workspace without mutable target-directory sharing;
* the public and private rustdoc JSON passes use separate stable census
  targets, and the CPU-bound snapshot checker runs from Cargo's release
  profile in its own stable target.

CI does not use those duplicated warm targets. Cold execution retains them
across `make clean` and
uses a four-lane bounded API/main/broker prebuild frontier, followed by a
full-budget fixture, inventory and schema chain on shared targets. CI runs
API, main, broker, guest shell runner, no-bash AST, schema, inventory and
supply chain as eight independent full-budget Make jobs.

The final hyperfine record is
`.scratch/test-speedup-optimized/test-rust.json`:

| Result | Seconds |
| --- | ---: |
| Baseline warm median | 324.763168 |
| Optimized warm samples | 139.945960, 140.117662, 139.419719 |
| Optimized warm median | **139.945960** |
| Warm reduction | **56.908%** |
| Slowest / median | 1.0012 |

The optimized median is 43.092% of baseline and therefore passes the
50%-of-baseline ceiling of 162.381584 seconds. The slowest sample is less
than 1% above the median, within the 20% stability limit. Because the Make
DAG met the hard target, the conditional mold experiment in T017 was not
entered and no linker dependency was added.

The final cold observation is
`.scratch/test-speedup-optimized/test-rust-cold.json` at 835 seconds, compared
with the 911.204650-second baseline cold median. Cold elapsed time is therefore
8.362% lower than baseline. The shared Nix store and `.scratch` compiler cache
were retained in both measurements.

## Rust CI result

The pre-change `v3` jobs completed in 5m24s for API, 7m30s for main and
10m20s for the combined remaining shard. The final PR run split the remaining
surface and reported:

| CI Make target | Duration |
| --- | ---: |
| `test-rust-api-surface` | 4m09s |
| `test-rust-main` | 6m50s |
| `test-rust-broker` | 4m15s |
| `test-rust-guest-shell-runner` | 2m10s |
| `test-rust-no-bash-ast` | 1m30s |
| `test-rust-schema` | 2m19s |
| `test-rust-inventory` | 7m33s |
| `test-rust-supply-chain` | 1m30s |
| `test-rust` rollup | 7s |
| `test-fixture-contracts` | 12m12s |

The final PR workflow at `39b09ca4` ran from 00:54:35Z through 01:08:22Z, a
13m47s
critical path. Every Rust leaf is below eight minutes and the adjacent
fixture lane remains below 15 minutes.

## Rust coverage evidence

The passing v1 manifest is
`.scratch/test-speedup-optimized/test-rust-executed.json`, SHA-256
`bbc9c72c498e437682251f3790baaa8935d12051e7c7ed40cdcad71c9dc39c8d`.
It records `run_status = "passed"` and all 20 baseline leaf identifiers.
Direct sorted comparison with the trace-derived baseline manifest has no
missing or added leaf.

The optimized source census is
`.scratch/test-speedup-optimized/test-rust-inventory.txt`. Cargo doctest
listing prints elapsed-time summary lines, so both source inventories were
also normalized by removing empty lines and lines beginning
`all doctests ran in `.
The normalized optimized digest is
`aa1ccf12e9c31d178dcf5fbbaf12e6c2489394086aff2310c7fa2d9aff03a336`.
The normalized comparison has no missing baseline test and 16 added tests:
the execution-manifest documentation policy plus 15 Rust DAG, companion,
profile, quota, mutation, and excluded-workspace policy tests.

## Rust validation evidence

The final implementation passed:

```text
nix shell --inputs-from . nixpkgs#python3 --command \
  python3 tests/unit/meta/ci-runner-regression.py
bash tests/unit/gates/ci-rust-cache-sync.sh
cd packages && cargo test -p xtask --test policy_workspace
cd packages && cargo test -p xtask --test policy_ci
cargo test --manifest-path packages/d2b-contract-tests/Cargo.toml \
  --test policy_docs
make layer1-workflow-check
make test-policy
D2B_ENABLE_FIXTURE_BUILD=1 make test-fixture-contracts
D2B_EXECUTION_MANIFEST=.scratch/test-speedup-optimized/test-rust-executed.json \
  make test-rust
```

The executable manifest regressions cover success, failure, lock contention,
failed publication, handled TERM, shutdown-only subreaper activation,
descendant draining, parent/path injection, cleanup chaining, Nix re-entry,
status preservation, and descriptor closure. Targeted runs also exercised the
warm/cold profile switch, all eight CI Make targets, schema/inventory
dependency chains, isolated and shared API/fixture targets, PTY behavior under
the manifest helper, invalid budget exit 2, and all harness-free smoke checks
without libtest arguments.

## Nix-unit candidate comparison

Every candidate started from committed base `28aa3b57`. Timing commands ran
sequentially with no other candidate active. Samples with an external Nix
client were retained as invalid and excluded. Candidate artifacts are under
`.scratch/test-speedup-nix-candidates/`.

| Candidate | Result | Evidence and disposition |
| --- | --- | --- |
| N0 - tuned Bash pool | Rejected | Four-worker warm observation: 337.13s. It exceeds the 269.445s ceiling and retains the repository-specific scheduler prohibited by the runner contract. |
| N1 - pure aggregate | Rejected | The single evaluator remained inside whole-corpus evaluation after 610s and was terminated. No result derivation had been realized. |
| N2 - lix-unit | Rejected | Tool acquisition took 452.48s, the fork retains the `nix-unit` binary name, and corpus evaluation repeatedly overflowed its stack. A 64 MiB stack retry still exceeded 600s. The dependency was not selected. |
| N3 - nix-eval-jobs | Selected after refinement | Four workers with instantiation took 311.93s. Per-case `--no-instantiate` reached a 202.20s median but exhausted hosted memory. Seven shard aggregates took 543s. The selected file partition plus integrity reached a 231s local median and a 591s one-worker hosted-shape observation. |
| N4 - consolidated flake check | Rejected | The command is the already measured direct `nix flake check --no-build --keep-going` path. Its 565.495s baseline warm median exceeds the target and broadens focused iteration to the full flake. |
| N5 - nix-fast-build | Not entered | N3 removed realization from the critical path. Parallel realization and grouped build logs were therefore not material. |

The selected runner exposes one aggregate attribute per case file and invokes
`nix-eval-jobs --no-instantiate`. It preserves the existing case evaluator and
all pin and shard checks while avoiding output realization and the memory cost
of 893 per-case result attributes. The runner owns parallel evaluation; the
shell validates the separate case/job inventory and caps the requested worker
count by logical CPUs, finite cgroup CPU quota, and available memory.

The focused locked dev shell supplies `nix-eval-jobs` and `jq`. Plain
`make test-nix-unit` enters it once when necessary. The retired
`D2B_NIX_UNIT_JOBS` variable fails with status 2 and names
`D2B_NIX_UNIT_WORKERS` as its replacement. The operator-intent
`D2B_NIX_UNIT_MEMORY_MB` control lowers, but cannot raise, the retained
per-worker evaluator limit.

Successful full runs suppress the raw JSONL result dump. Failed attributes
remain individually attributable as concise stderr entries with the
repository root replaced by `<repo>`, while evaluated-vs-pinned case-name
drift names every missing or unexpected case and directs operators to
`run make nix-unit-pin`. Command progress uses the fixed path-free `d2b` flake
label.

## Nix-unit optimized result

The representative warm samples on the integrated runner were:

| Result | Seconds |
| --- | ---: |
| Baseline warm median | 538.889473 |
| Optimized warm samples | 222, 233, 231 |
| Optimized warm median | **231** |
| Warm reduction | **57.13%** |
| Slowest / median | 1.0087 |

The optimized median is 42.86% of baseline and passes the 50%-of-baseline
ceiling of 269.444737 seconds. The slowest valid sample is less than 1% above
the median. Every run evaluated all 45 file attributes plus shard/pin
integrity and compared all 893 pinned x86 case names through the separate
inventory.

The final fresh-cache observation used plain `make test-nix-unit`, a newly
created `XDG_CACHE_HOME`, and locked tool self-provisioning. It completed in
196s, compared with the 659.700177s baseline cold median, a 70.29%
reduction. The shared Nix store was retained. No candidate or final accepted
sample overlapped another Nix client.

The final runner reserves 2048 MiB of process and flake overhead per worker
plus 3072 MiB for the host when calculating the memory cap. GitHub Actions
uses a 3072 MiB evaluator restart limit, while local development retains
4096 MiB. A local one-worker hosted-shape run completed the full file
partition plus integrity in 591s. Hosted success remains a merge condition.

## Nix-unit failure and coverage evidence

The passing manifest is
`.scratch/test-speedup-optimized/test-nix-unit-file-integrity.json` at
integrated tip `455b1648`. It records
`run_status = "passed"`, the seven baseline leaves, no failed surface, no
installable, and no realized check. The completed leaves are:

```text
nix-unit
nix-unit-daemon
nix-unit-guest
nix-unit-misc
nix-unit-network
nix-unit-runtime
nix-unit-state
```

An isolated committed probe changed one daemon case and one state case and
substituted one pin name. One invocation reported both exact case names under
their file-job attributes, named the missing and unexpected inventory cases
with the `make nix-unit-pin` remedy, and published
`.scratch/test-speedup-optimized/test-nix-unit-file-failed.json` with
`run_status = "failed"` and no stale completed leaf.

The retained selector probe
`D2B_NIX_UNIT_CHECK=nix-unit-misc make test-nix-unit` passed and published a
manifest containing only `nix-unit-misc`. CI no longer uses the selector: the
generated workflow contains one enforcing `test-nix-unit` job whose test
command is exactly `make test-nix-unit`.

## CI-cold refinement result

The earlier per-case runner reached a 202.20s local four-worker warm median but
repeatedly caused a GitHub Actions 16 GiB runner to shut down. The seven
topical-check aggregate candidate took 543s locally and was rejected for
latency.

The selected refinement changes only the Nix-unit evaluation surface.
`nixUnitJobs.<system>` contains exactly one aggregate
attr per current case file (45 file jobs), plus the `nix-unit` shard/pin
integrity attr, using the shared constructor also used by the seven topical
flake checks. `nixUnitInventory.<system>` is one
locked, sorted inventory object containing both full `caseNames` and file-job
`jobNames`, including integrity, derived without forcing case expressions. The
runner evaluates that
inventory once through `git+file`, compares result attrs exactly with
`jobNames`, and compares `caseNames` exactly with the common and native-system
pins. Aggregate errors retain every real `FAIL <case>: <detail>` line,
excluding source templates, and emit one attributable fallback when a file
aggregate provides no real FAIL line.

The local defaults remain four requested workers and a 4096 MiB evaluator
limit. GitHub Actions requests one worker with a 3072 MiB limit on a 16 GiB
runner. The local hosted-shape run completed in 591s, below the 20-minute job
timeout and the 12m39s fixture critical path. Actual hosted success and
duration are not claimed until the PR job passes.
