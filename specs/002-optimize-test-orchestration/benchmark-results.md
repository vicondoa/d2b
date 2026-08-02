# Test-orchestration baseline results

This record covers only the pre-change baseline at
`d09cba95f7602b70f1f79b957f426ecfacf65bf1` on branch `test-speedup`.
It completes T001-T007. No implementation file was changed and no optimized
acceptance claim is made here.

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

The accepted Rust implementation and final measurements are at
`0e563f433ccd41f8a4a57c955679e10fc256cecc`. GNU Make owns nine bounded
`test-rust-leaf-*` nodes, while `tests/test-rust.sh` owns only explicit leaf
execution. The
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

Cold and CI runs do not use those duplicated warm targets. Cold execution
uses a four-lane bounded API/main/broker prebuild frontier, followed by a
full-budget fixture, schema and inventory chain on shared targets. CI runs
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
`.scratch/test-speedup-optimized/test-rust-cold.json` at 888 seconds, compared
with the 911.204650-second baseline cold median. Cold elapsed time is therefore
2.547% lower than baseline. The shared Nix store and `.scratch` compiler cache
were retained in both measurements.

## Rust CI result

The pre-change `v3` jobs completed in 5m24s for API, 7m30s for main and
10m20s for the combined remaining shard. The final PR run split the remaining
surface and reported:

| CI Make target | Duration |
| --- | ---: |
| `test-rust-api-surface` | 4m22s |
| `test-rust-main` | 6m11s |
| `test-rust-broker` | 4m16s |
| `test-rust-guest-shell-runner` | 1m56s |
| `test-rust-no-bash-ast` | 1m18s |
| `test-rust-schema` | 2m02s |
| `test-rust-inventory` | 7m35s |
| `test-rust-supply-chain` | 1m28s |
| `test-rust` rollup | 8s |
| `test-fixture-contracts` | 12m32s |

The complete PR workflow ran from 23:22:42Z through 23:36:18Z, a 13m36s
critical path. Every Rust leaf is below eight minutes and the adjacent
fixture lane remains below 15 minutes.

## Rust coverage evidence

The passing v1 manifest is
`.scratch/test-speedup-optimized/test-rust-executed.json`, SHA-256
`511d01418593d085a1216defd319f71b2253cf2ab256059aa0cefc0a90c86c56`.
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
