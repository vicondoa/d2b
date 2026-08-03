# d2b tests

How the test suite is organized, where each kind of test lives, and how to run
and add them. For the **decision rule on where a new test goes** (and the rule
that you must *not* add new ad-hoc `tests/*.sh`), read [`AGENTS.md`](./AGENTS.md) -
that is the binding contract; this file is the human quick-start.

## Two layers

- **Layer 1 - static gate.** Hermetic, fast, deterministic; no live host, VM, or
  container. Runs on every PR and locally via `make check`. This is where the
  overwhelming majority of tests live (Nix eval cases, Rust unit/integration/
  contract/policy-lint tests, flake checks, and a small closed set of drift +
  meta gates). The manifest records which jobs are enforcing and which are
  advisory; an advisory success may be a guarded skip and is not validation
  evidence.
- **Layer 2 - integration tiers.** Real systemd / kernel / userland: podman
  containers, runNixOSTest VMs, live-host scripts, and hardware tests. Used only
  when Layer 1 *provably* cannot cover the behaviour.

## Directory structure

```
tests/
├── static.sh, runner.sh, test-*.sh                                orchestrators (entry points)
├── lib.sh, cli-rust-native-common.sh                              shared shell harness
├── README.md, AGENTS.md                                           this guide + the test-model contract
├── migration-ledger.toml, migration-state.d/                    retirement ledger + per-test records
├── golden/, fixtures/                                           shared golden data + fixtures
├── tools/                                                       runners + codegen/asserter tools
│                                                                (rust-workspace-checks, gen-*, assert-pinned-tests, …)
├── unit/                          ── Layer 1 ──
│   ├── nix/        cases/ + pinned/ + eval-cases/               type 1: nix-unit eval cases
│   ├── smoke/      smoke-eval*.nix                              type 6: smoke / flake-check defs
│   ├── meta/                                                    meta gates (guard the test infra; closed set)
│   └── gates/                                                   drift + perf gates (closed set)
├── integration/                   ── Layer 2 ──
│   ├── containers/                                              type 9: podman (make test-integration; host/manual pre-PR)
│   ├── distro-matrix/                                           distro pins + fixtures
│   └── live/                                                    type 11: D2B_LIVE live-host (manual)
└── host-integration/
    ├── *.nix                                                    type 10: runNixOSTest (make test-host-integration; host/manual pre-PR)
    └── hardware/                                                type 12: real-device tests (manual)
```

Rust tests (types 2-5: unit, integration, contract, policy-lint) live under
`packages/<crate>/`, **not** here.

## Running tests

| Command | Runs | Where |
|---------|------|-------|
| `make test-unit` | **post-preflight L1 umbrella** from `tests/layer1-jobs.json`; `make check` also runs the manifest's preflight jobs | local + CI (parallel jobs) |
| `make test` | `test-unit` + `test-integration` | local host; still run `make test-host-integration` before opening an agent-owned PR |
| `make check-tier0` | sub-60s syntax + shellcheck gate | local + CI |
| `make check-inventory` | fail-closed migration-ledger drift check | local + CI |
| `make test-lint` | preflight + nix-parse + shellcheck | local + CI |
| `make test-changelog` | require release notes for code changes and validate every changelog fragment | local + CI |
| `make test-rust` | bounded Make DAG for the Rust leaves (API, fmt, clippy, workspace tests, conditional fixture/CLI, broker x3, deny/audit, schema, inventory, and no-bash); fixture/CLI are included once when Nix is available | local + CI |
| `make test-rust-<leaf>` | eight CI leaf targets (API, main, broker, guest shell runner, no-bash AST, schema, inventory, supply chain) behind the stable `test-rust` rollup; each receives the full runner budget | CI (local for a focused rerun) |
| `make test-fixture-contracts` | enforcing eval-rendered lane: materializes `D2B_FIXTURES` from evaluated Nix artifact data, then runs `d2b-contract-tests` and the CLI-contract cases; both lanes set `D2B_ENABLE_FIXTURE_BUILD=1`, and invoking it without that variable fails rather than skipping | local + CI |
| `make test-proofs` | standalone proofs/ crates | local + CI |
| `make test-flake` | `nix flake check --no-build` (native system); `D2B_FLAKE_CHECK=<name>` instantiates one check, `D2B_FLAKE_OUTPUTS=1` sweeps non-`checks` outputs, `D2B_FLAKE_LOCAL_SHARDS=1` runs the local bounded shard fan-out | local + CI (x86 sharded per-check matrix; aarch64 PR job runs a lightweight smoke eval) |
| `make test-flake-list` | emit native-system flake check names as JSON; the partition tool below reads it | source helper |
| `make test-flake-partition` | emit those names split into the eval, realized, and nix-unit dispatch classes (CI matrix plumbing) | CI (dynamic matrix) |
| `make test-nix-unit` | sharded nix-unit corpus checks, retained as explicit evidence in the manifest-driven local and CI graph | local + CI |
| `make test-drift` | drift-check + vms-json-parity + flake-check-matrix-sync | local + CI |
| `make test-policy` | meta gates (ci-coverage, adr-index, deliverable inventory, etc.) | local + CI |
| `make test-runtime-ledger` | hermetic execution-budget gate: after a warm build, enforces aggregate per-crate process-CPU p95 budgets, fails any individual census test sample over 60 seconds, and reports shorter per-test wall-clock p95s as advisory diagnostics (holds no baseline; makes no historical-regression claim) | local + CI |
| `make test-performance-budgets` | advisory performance canary; without `D2B_PERF_STABLE=1` it reports `SKIP` and enforces nothing | local + CI |
| `make test-integration` | type-9 podman container tests | **local host/manual pre-PR** (podman; not the PR pipeline) |
| `make test-host-integration` | type-10 runNixOSTest VM checks | **local NixOS host w/ KVM**, manual pre-PR (not the PR pipeline; TCG fallback) |
| `make check-fast` | alias for `test-unit` (backward compat) | local + CI |
| `make check` | PR-equivalent manifest target set with bounded local parallelism; enforcement classifications come from `tests/layer1-jobs.json` | local |
| `make check-static` | legacy/full-static monolithic gate (`tests/static.sh`) | local |
| `make layer1-workflow` | regenerate `.github/workflows/pr-l1-static-fast.yml` from `tests/layer1-jobs.json` + template | local |
| `make layer1-workflow-check` | verify the generated workflow is up to date | local + CI via `make test-drift` |
| `make flake-matrix-pin` | regenerate the CI flake-check-matrix drift pin after adding/removing a flake check | local |
| `make nix-unit-pin` | regenerate the nix-unit case-presence pins | local |
| `make runtime-ledger-pin` | regenerate the runtime-ledger census pin after adding, removing or renaming a timed test | local |
| `cargo run --manifest-path packages/Cargo.toml -p xtask -- heavy-gate -- env D2B_LIVE=1 bash tests/integration/live/<x>.sh` | type-11 live-host tests, through the heavy-gate semaphore | **manual, against a deployed d2b host** |

`make test-policy` includes the fail-closed `guest-workspace-drift` guard. The
guard checks that the crates copied by `mkGuestRustPackagesSrc`, the members and
workspace dependencies in
`tests/fixtures/guest-rust-workspace/Cargo.toml`, any
`tests/fixtures/guest-rust-workspace/*.Cargo.toml` overrides, and
`packages/Cargo.guest.lock` remain one resolvable locked workspace. When a
mirrored shared crate gains or changes a dependency, update the guest workspace
fixture and any affected override, refresh `packages/Cargo.guest.lock`, and run
`make test-policy`.

All Layer-2 lanes (types 9-12) run behind one sole-use semaphore, invoked
from the repository root as `cargo run --manifest-path packages/Cargo.toml
-p xtask -- heavy-gate` (two slots per uid via open file description locks), so
concurrent heavy lanes cannot oversubscribe the shared Nix store, cargo
target directory, or KVM device. The public lane targets above
(`make test-integration`, `make test-host-integration`, `make test-hardware`,
`make perf`) acquire a slot and then delegate to a guarded internal
`heavy-lane-*` target that fails closed if run outside the gate; run the
public targets, not the internal ones. `make heavy-check`,
`make heavy-cargo-test`, `make heavy-flake-check`, and the `heavy-test-*`
aliases run a Layer-1 gate, the Rust suite, the building flake check, or a
public lane under the same semaphore. Live-host and hardware scripts obey the
same rule: use the gated `make pre-tag` / `make smoke-lite` live-VM smoke
entrypoints, or wrap a raw live script as `cargo run --manifest-path
packages/Cargo.toml -p xtask -- heavy-gate -- env
D2B_LIVE=1 bash tests/integration/live/<x>.sh`. Invoking `D2B_LIVE=1 bash
tests/integration/live/<x>.sh` directly no longer bypasses the semaphore:
each live and hardware entrypoint, plus the enforcing path of each performance
entrypoint, verifies its inherited slot and re-executes itself through the gate
exactly once when no genuine slot is held. The advisory performance skip exits
before acquiring a slot because it does no heavy work. A bare `D2B_HEAVY_GATE`
value is not trusted, so the shared Nix store, cargo target directory, and KVM
device cannot be oversubscribed. The gated targets remain the documented path.
The `cargo run --manifest-path
packages/Cargo.toml` spelling is required because there is no root cargo
workspace, so the bare `cargo xtask` alias resolves only when run from
`packages/`; see AGENTS.md for the `sccache` tradeoff and the `cd packages
&& cargo xtask <command>` alternative.

The semaphore uses a protected, system-provisioned namespace under
`/run/d2b-heavy-gates`; it never falls back to a user-writable runtime or
temporary directory. The NixOS module provisions the fixed root at boot and
creates two private slots for each configured `d2b.site.launcherUsers` member
that NSS can resolve during activation. An unavailable network-backed user is
deferred rather than failing activation. After that user logs in, or on a
development machine that does not use the module, run
`make heavy-gate-provision` once per boot when the gate requests it. The target
uses the caller's numeric UID without an NSS user-name lookup and uses `sudo`
only to create the root-owned namespace and the current user's two mode-`0600`
slot files. This per-boot step is necessary because `/run` is a tmpfs. Until it
is complete, a missing or malformed namespace fails closed with stable code
`heavy-gate-provisioning-required` and names that Make target as the
remediation; do not work around it by moving the gate into `/tmp` or another
user-owned location.

Current live-host scripts include `d2b-store.sh` for per-VM store
adoption and `usbip-guestd-lifecycle.sh` for USBIP guestd attach/detach across
a `d2bd` restart. The USBIP script requires
`D2B_USBIP_VM=<vm>` and `D2B_USBIP_BUSID=<busid>` and uses only `d2b usb`
verbs for USB state changes.

`tests/layer1-jobs.json` is the central Layer-1 job graph. In its local phase
order, the enforcing jobs are `check-tier0`, `check-inventory`, `test-lint`,
`test-changelog`, `test-rust`, `test-proofs`, `test-flake`, `test-nix-unit`,
`test-policy`, `test-drift`, `test-runtime-ledger`, and
`test-fixture-contracts`. The only advisory job is
`test-performance-budgets`. `make test-unit`
consumes the same manifest but skips its preflight phase. Re-read the manifest
rather than assuming this split is fixed.

Jobs are enforcing by default. The manifest's optional `"enforcement":
"advisory"` field classifies a job whose successful exit might not represent
executed checks, and the accompanying `advisoryReason` records why. An advisory
command still runs, and a nonzero exit still fails the graph, but a guarded
skip is not a failure. The runner labels such a result `advisory:` rather than
`ok:` and reports enforcing and advisory counts separately. Do not cite an
advisory job result as validation evidence for a change.

The performance advisory exists because latency budgets require a pinned
self-hosted runner. `test-performance-budgets` exits successfully with `SKIP`
unless `D2B_PERF_STABLE=1`; no current project runner provides that stable
environment. Promotion to enforcing requires provisioning a pinned self-hosted
runner, setting `D2B_PERF_STABLE=1` there, and removing the `enforcement` and
`advisoryReason` fields from the job after that wiring lands.

The fixture lane is enforcing. It fails when `D2B_ENABLE_FIXTURE_BUILD=1` is
absent, evaluates both Nix configurations, materializes their artifact data, and
runs the fixture-dependent contract and CLI tests. The default `test-rust` and
focused `test-rust-main` include those surfaces once when Nix is available;
`D2B_SKIP_FIXTURE_BUILD=1` omits them for the Layer-1 graph so this separate
lane does not duplicate work. Selected hermetic policy files may have separate
enforcing entrypoints under `test-policy`; inspect that target before citing
one.

### Rust DAG budget and execution evidence

`make test-rust` is the only aggregate Rust entrypoint. GNU Make schedules its
explicit leaves with `--keep-going` and `--output-sync=target`, so independent
leaves continue after a failure and each target's output remains grouped. The
broker feature passes stay serial. Fixture/CLI work and the API snapshot
checker use isolated stable targets below `.scratch/rust-test-cache`, so they
can overlap the main workspace without sharing mutable Cargo state. The public
and private rustdoc censuses overlap only when the API leaf receives at least
two jobs, and their split Cargo quotas stay within that leaf's budget. The
snapshot checker uses its release profile for the measured CPU-bound JSON
pass. Budgets through nine use one job per active lane; surplus jobs above nine are assigned
to the measured API long pole while the complete frontier stays bounded.
Direct calls to `tests/test-rust.sh` require one explicit leaf mode; callers
that need the complete gate must use `make test-rust`. The focused
`make test-rust-main` also retains conditional fixture/CLI coverage.

CI invokes one Make target per Rust leaf, so local-only dependency edges do
not repeat schema or inventory work in the main and broker jobs. A cold local
aggregate (detected when `packages/target` is absent) restores the shared
workspace target layout and retains the warm-local split API census cache
across `make clean`. A bounded API/main/broker prebuild frontier runs first;
fixture, inventory and schema then run as a full-budget chain so discovery
reuses all prior builds before schema generation. Warm local runs retain the
parallel isolated-target profile, while CI alone uses the shared API census
target.

Use `D2B_RUST_BUDGET=<positive-integer>` to request a Rust budget. It is an
upper bound, not a host-capacity bypass. The default is the smaller of logical
CPUs and a memory cap derived from `MemAvailable` plus cache-adjusted finite
cgroup v2 `memory.max` or `memory.high` allowance. The calculation reserves
2 GiB for the host and budgets 3 GiB per heavy job. If visible cgroup v2
controller state cannot be read, the gate warns and fails closed to budget 1.
Cargo jobs and nextest test threads are assigned quotas whose every active
frontier remains within the effective budget, including budget 1. Top-level
Make `-j` does not replace this control.

Set `D2B_EXECUTION_MANIFEST` to opt into deterministic execution evidence:

```bash
D2B_EXECUTION_MANIFEST=.scratch/test-rust-executed.json make test-rust
```

The v1 schema and lifecycle are documented in
[`../docs/reference/test-execution-manifest.md`](../docs/reference/test-execution-manifest.md).
The previous manifest is removed before dispatch, complete fragments are
published atomically, and handled failures or interruptions publish partial
evidence without stale success. A manifest is diagnostic evidence and does not
replace source inventory, `make test-policy`, or
`D2B_ENABLE_FIXTURE_BUILD=1 make test-fixture-contracts`.

### Nix-unit runner

`make test-nix-unit` uses the established `nix-eval-jobs` runner with
`--no-instantiate` against the locked `nixUnitJobs.<system>` flake output.
That output contains one aggregate attr per current `*.nix` case file (45
file jobs), plus the `nix-unit` shard/pin integrity attr. File jobs use stable
names of the form `case-<basename>`. Each file job
reuses the same `casesFor`/`resultsFor`/failure-report constructor as the
seven topical `checks.<system>` leaves, so it reports every
`FAIL <case>: <detail>` from its file without submitting installables to the
daemon or realizing derivations.
The seven topical checks remain the stable manifest leaves:
`nix-unit`, `nix-unit-daemon`, `nix-unit-guest`, `nix-unit-misc`,
`nix-unit-network`, `nix-unit-runtime`, and `nix-unit-state`.
`nixUnitInventory.<system>` is one locked object containing sorted `caseNames`
and sorted `jobNames`, including the integrity attr. The runner evaluates that
inventory once, compares `jobNames` exactly with the result attrs, and compares
`caseNames` exactly with the
common and native-system pin files. Discovery is empty-set failure, and
missing or unexpected names still direct operators to `run make nix-unit-pin`.

If `nix-eval-jobs` or `jq` is absent, the target makes one guarded re-entry
through `devShells.<system>.nix-unit`, a focused `mkShellNoCC` output containing
the locked versions from this flake's `flake.lock`. An existing toolchain or
development shell with both commands runs directly. `D2B_NIX_UNIT_JOBS` is
retired and exits with status 2; use
`D2B_NIX_UNIT_WORKERS=<1..4>` instead. The requested worker count is
bounded by the CPU cap `min(4, logical CPUs, finite cgroup CPU quota)` and the
memory cap
`max(1, floor((effective available MiB - 3072) / (limit + 2048)))`.
The evaluator limit defaults to 4096 MiB locally and 2048 MiB on GitHub
Actions, plus 2048 MiB of per-worker process and flake overhead. GitHub
Actions requests two workers by default; local development requests four.
This keeps the hosted envelope near 11 GiB while preserving useful
parallelism and local speed.
Effective available memory is the smaller of `MemAvailable` and the finite
cgroup allowance after reclaimable file cache. A visible but unreadable cgroup
controller fails closed to one worker. On the reference 12-CPU, 62-GiB host
the effective cap preserves four workers.
`D2B_NIX_UNIT_MEMORY_MB` may set the evaluator limit from 512 through 4096 MiB;
the 2048 MiB overhead remains reserved. Successful full runs
suppress raw JSONL output. Every real `FAIL <case>: <detail>` line from an
aggregate error is printed as one concise, path-sanitized stderr entry.
Repository and home roots become fixed placeholders; Nix store hashes are
redacted while derivation names remain visible. Source-code template lines are
ignored, and an aggregate with no real FAIL line receives one attributable
fallback diagnostic. Result attributes are
also compared exactly with the locked file-job names. Command progress uses the
fixed path-free `d2b` flake label.

`D2B_NIX_UNIT_CHECK=<name>` remains the manual single-shard selector. It
requires one of the seven discovered aggregate checks and evaluates only that
check. With execution evidence enabled, a full pass publishes exactly the
seven leaves `nix-unit`, `nix-unit-daemon`, `nix-unit-guest`, `nix-unit-misc`,
`nix-unit-network`, `nix-unit-runtime`, and `nix-unit-state`; a selected pass
publishes only its selected leaf.

`.github/workflows/pr-l1-static-fast.yml` is generated from the manifest by
`make layer1-workflow` and checked by `make layer1-workflow-check` during
`make test-drift`. CI runs the individual Layer-1 jobs in parallel and exposes
a stable final `check` rollup job intended for branch protection. Locally,
`make check` runs each manifest job's `makeTarget` and every
`extraMakeTargets` entry, so the manifest-declared target set matches the
pull-request graph. The performance job remains advisory in both places.

The `test-runtime-ledger` job is part of that graph. It warm-builds the pinned
census, records per-test wall-clock p95s as advisory diagnostics, and enforces
each crate's aggregate process-CPU p95 budget. Process CPU excludes time the
test process is descheduled behind unrelated machine load. The closed census
presently pins one crate and exactly 190 tests, so a vanished or extra test,
an incomplete or under-repeated run, or an aggregate crate CPU p95 over budget
fails; a per-test diagnostic-threshold breach does not.

The gate holds no baseline and makes no historical-regression claim. The
`test-runtime-ledger check` output is authoritative for exact advisory-report
formatting and selection. Growing the census to a real multi-crate shard
inventory (with a per-shard budget) and adding a cross-machine reference
baseline for a true historical-regression gate is the deferred follow-up
`runtime-ledger-full-census-and-real-shards`. If this description diverges from
the current `Makefile` target or `tests/layer1-jobs.json`, treat those as
authoritative and flag the drift for the integrator rather than hand-editing
the census pin.

The x86 `test-flake` leg is sharded one job per flake check (the classes are
enumerated at CI time by `make test-flake-partition`; the `test-flake-x86` job
is a stable aggregator over both shard lanes + the non-`checks` outputs job).
That partition splits the checks three ways. Checks the driver must *build*
rather than instantiate run in their own lane, because a build takes minutes
where an instantiate takes seconds, and queueing one behind a bounded matrix of
the latter sets the whole run's critical path. The nix-unit corpus checks are
dropped, because the dedicated Nix-unit lane already builds exactly those and
instantiating them again is redundant; that lane reads the same partition, so
the names dropped here are the names it runs. Everything else instantiates in
the bounded matrix. The tool fails closed if the partition is not total, if the
enumeration is empty, or if a check named as realized is not in the flake. The
aarch64
leg runs only the lightweight `smoke-eval-aarch64.nix` expression. A fail-closed
drift gate keeps the matrix and smoke wiring in sync with the flake (`make
flake-matrix-pin` to update its pin). Locally, manifest-driven `make check`
sets `D2B_FLAKE_LOCAL_SHARDS=1` for `make test-flake` and
`D2B_SKIP_FIXTURE_BUILD=1` for `make test-rust`, matching the PR Rust job. The
flake shards do not execute `d2b-contract-tests`; the separate enforcing
fixture-contract lane runs them with evaluated fixtures and fails rather than skipping. Tune
`D2B_CHECK_JOBS` and `D2B_FLAKE_JOBS` for host capacity. Agent-owned PRs also run
`make test-integration` and `make test-host-integration` on the host before the
PR is opened; those manual integration tiers are not replaced by PR pipeline
jobs.

Useful knobs:
- `D2B_NO_SCCACHE=1` - disable sccache in the rust gate.
- `D2B_CI_SCCACHE=1` - opt the rust gate back into sccache under CI (off by
  default there; `pr-l1-static-fast` sets it and backs `SCCACHE_DIR` with
  `actions/cache`, using sccache's local-disk backend - never the native GHA
  backend, which would export `ACTIONS_RUNTIME_TOKEN` into the build env).
- `D2B_NO_PARALLEL_BROKER=1` - run the broker feature passes serially.
- The rust gate uses **sccache** (a shared per-crate compilation cache) and
  runs the broker's three feature passes (default / layer1-bootstrap /
  fake-backends) concurrently with the main workspace, on deterministic target
  dirs so the sccache cache key stays stable.

## Adding a test

See [`AGENTS.md`](./AGENTS.md) for the full decision rule. In short, default to
Layer 1:

- Nix module value / option / eval-rejection → a nix-unit case in
  `tests/unit/nix/cases/*.nix` (auto-discovered; regenerate pins with
  `tests/tools/gen-nix-unit-pins.sh`).
- Rust logic → a `#[test]` in the crate's `src`.
- Real-binary behaviour → `packages/<crate>/tests/*.rs` against
  `CARGO_BIN_EXE_*`. **Spawn hermetically**: point `D2B_PUBLIC_SOCKET`,
  `D2B_BROKER_SOCKET`, and the `D2B_*_PATH` fixture env vars at fixtures
  or missing paths so the test never touches the operator's live daemon.
- Rendered-artifact ↔ DTO/doc contract → a contract test in
  `packages/d2b-contract-tests/`.
- Generated docs/schemas/CLI freshness → already a drift gate; regenerate with
  `cargo run -p xtask -- gen-*`. Do **not** add a new shell gate.

Only reach for Layer 2 (containers / VMs / live-host / hardware) when a foreign
userland, a real systemd boot, a live host, or a physical device is genuinely
required - and pick the lowest tier that works.

## Conventions

- **Commit before building.** `nix flake check` and the eval gates resolve the
  flake via `git+file://`, which only sees git-tracked files - an untracked new
  module/test is invisible until committed.
- **Retiring a test is ledger-tracked** (`tests/migration-state.d/<name>.toml` +
  `tests/tools/gen-migration-ledger.sh --check`); fail-closed native successors
  are pinned in `tests/golden/pinned/` and checked by
  `tests/tools/assert-pinned-tests.sh`.
