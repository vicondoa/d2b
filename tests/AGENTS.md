# AGENTS.md - the d2b test model (read before adding a test)

This file is the contract for **where a new test goes and how it runs**. It
exists to stop the failure mode that motivated the test rearchitecture: agents
reaching for a new ad-hoc `tests/*.sh` every time, which made the suite slow and
unmaintainable. If you are adding or changing test coverage, follow the decision
rule below. The human-facing structure + run instructions live in
[`README.md`](./README.md).

## The one rule

**New coverage MUST land as a Layer-1 test (types 1-6 below) unless it
*provably* requires a real container, a booted VM, a live host, or physical
hardware.** There is no "type 7/8" escape hatch: the drift gates and meta gates
are a **closed set** - do not add a new `tests/*.sh`. If you think you need a
shell gate, you almost certainly want a nix-unit case (type 1) or a Rust test
(types 2-5) instead.

That closed set covers *gates*. `tests/tools/` is the open home for the
plumbing a gate or a CI job calls - enumerators, partitioners, generators,
runners - and a new file may land there when it is genuinely plumbing and not a
test case. The distinction is what fails: a gate asserts an invariant and
belongs to the closed set; a tool produces or transforms data for something
else to assert on, and is itself covered by whichever gate consumes it. The
migration ledger inventories `tests/*.sh` only, so a tool needs no ledger row -
which is exactly why it must not smuggle in assertions that would then go
untracked.

When in doubt, push the test *down* the tiers (toward type 1), not up.

## Taxonomy - name, definition, home, how it runs

### Layer 1 - static gate (hermetic, fast, every PR + local via `make check`)

| # | Type | What it is | Lives in |
|---|------|------------|----------|
| 1 | **eval case** | declarative pure-Nix assertion (`{ expr; expected; }` / `{ expr; expectedError; }`) over module-config values + eval-rejection | `tests/unit/nix/cases/*.nix` (auto-discovered; pins in `tests/unit/nix/pinned/`) |
| 2 | **unit test** | `#[test]` over one crate's pure logic | `packages/<crate>/src/**` `#[cfg(test)]` |
| 3 | **integration test** | spawns the real binary (`CARGO_BIN_EXE_*`) over AF_UNIX/fd-passing; no host mutation | `packages/<crate>/tests/*.rs` |
| 4 | **contract test** | Rust assertion over a **rendered** Nix artifact (bundle / host-json / processes.json) - the Nix↔Rust + doc↔impl boundary | `packages/d2b-contract-tests/tests/*.rs` (`D2B_FIXTURES`) |
| 5 | **policy lint** | Rust scan of source/docs asserting a tree-wide invariant | `packages/d2b-contract-tests/tests/policy_*.rs` |
| 6 | **flake check** | realized example-config eval / supply-chain (`eval-*`, `rust-deny/audit`) | `flake.checks.<sys>.*`; smoke/check defs in `tests/unit/smoke/`, eval-case libs in `tests/unit/nix/eval-cases/` |

The remaining Layer-1 surface is a **closed set** you should not grow with new
files: **drift gates** (`tests/unit/gates/` - `xtask gen-* + git diff`) and
**meta gates** (`tests/unit/meta/` - guard the test infra itself).

Fixture-backed type 4 tests and fixture-dependent type 5 tests in
`d2b-contract-tests` are included once by the default `test-rust` aggregate
when Nix is available. The aggregate and `test-rust-main` honor
`D2B_SKIP_FIXTURE_BUILD=1`, which is set by the local and pull-request
Layer-1 orchestration so the separate enforcing
`D2B_ENABLE_FIXTURE_BUILD=1 make test-fixture-contracts` job does not duplicate
the contract and CLI surfaces. The fixture target materializes `D2B_FIXTURES`
from evaluated Nix artifact data and invoking it without the enable variable
fails rather than skipping. Selected hermetic policy files may also have
explicit enforcing entrypoints under `test-policy`; those binaries are excluded
from `test-fixture-contracts` so the Layer-1 graph runs their repository scans
once. Check the shared list in `tests/lib.sh` before citing one.

`test-policy` also runs `guest-workspace-drift`. It fails when the guest crates
copied by `mkGuestRustPackagesSrc` diverge from
`tests/fixtures/guest-rust-workspace/Cargo.toml`, its per-crate
`*.Cargo.toml` overrides, or `packages/Cargo.guest.lock`. If a shared crate
mirrored into the guest workspace gains or changes a dependency, update the
fixture and any affected override, refresh `packages/Cargo.guest.lock`, then
run `make test-policy`.

### Layer 2 - integration tiers (only when Layer 1 genuinely can't cover it)

| # | Type | What it is | Lives in | Runs **where** |
|---|------|------------|----------|----------------|
| 9 | **container** | Nix-OCI image under rootless podman; proves a static binary runs on a foreign non-Nix userland | `tests/integration/containers/*.sh` + `containerImages.<sys>.*` | `make test-integration` - **local host/manual pre-PR; not the PR pipeline** |
| 10 | **VM (runNixOSTest)** | boots a real NixOS VM; asserts live daemon/broker/socket-activation/host-posture/kernel behaviour | `tests/host-integration/*.nix` + `vmChecks.<sys>.*` | `make test-host-integration` - **local NixOS host w/ KVM, manual pre-PR; not the PR pipeline** |
| 11 | **live-host** | runs against a **real deployed** d2b host; destructive/stateful | `tests/integration/live/*.sh` | through the `cargo xtask heavy-gate` semaphore; `D2B_LIVE=1` / sudo - **manual, never CI** |
| 12 | **hardware** | real GPU / YubiKey / hardware-TPM passthrough | `tests/host-integration/hardware/*.sh` | through the `cargo xtask heavy-gate` semaphore - **manual on a host with the devices** |

Every Layer-2 tier (9-12) runs behind the `cargo xtask heavy-gate` sole-use
semaphore, never as a raw script. Use the gated public lane target
(`make test-integration`, `make test-host-integration`, `make test-hardware`;
`make pre-tag` / `make smoke-lite` for the live-VM smoke gate), or wrap an
ad-hoc live script as `cargo xtask heavy-gate -- env D2B_LIVE=1 bash
tests/integration/live/<name>.sh`.

Invoking a live script directly no longer bypasses the semaphore: each one
re-executes itself through the gate exactly once when `D2B_HEAVY_GATE` is
unset, so the shared Nix store, cargo target directory, and KVM device
cannot be oversubscribed. **Any new live, hardware, or performance
entrypoint must carry that same self-guard block**, or the fail-closed
inventory guard (`every_live_and_heavy_entrypoint_routes_through_the_gate`)
fails, since it walks the on-disk scripts and the Makefile.

## How to add a test (decision rule)

1. **Asserting a Nix module value / option / eval-rejection?** → type 1, a
   nix-unit case in `tests/unit/nix/cases/`. Add a case file (it is
   auto-discovered; do not edit `default.nix`), then regenerate the pin list
   (`tests/tools/gen-nix-unit-pins.sh`). CI evaluates the corpus through
   sharded `nix-unit-<shard>` flake checks; add new cases to the existing
   topical file whose shard already owns that behavior.
2. **Asserting Rust logic?** → type 2, a `#[test]` in that crate's `src`.
3. **Asserting the real binary's wire/CLI behaviour?** → type 3, a test in
   `packages/<crate>/tests/*.rs` against `CARGO_BIN_EXE_*`. Spawn hermetically -
   point `D2B_PUBLIC_SOCKET` / `D2B_BROKER_SOCKET` / `D2B_*_PATH` at
   fixtures or missing paths so the test never touches the operator's live
   daemon.
4. **Asserting that a *rendered* Nix artifact matches a Rust DTO / doc?** →
   type 4, a contract test in `packages/d2b-contract-tests/` (driven by
   `D2B_FIXTURES`).
5. **Asserting a generated artifact is up to date (docs/schemas/CLI)?** → it is
   already covered by a **drift gate**; regenerate with the matching
   `cargo run -p xtask -- gen-*` and commit - do **not** add a new gate. The
   compiler-derived capability API snapshots are regenerated explicitly with
   `make api-surface-pin`.
6. **Genuinely needs a foreign userland / real systemd boot / live host /
   device?** → the matching Layer-2 tier (9-12). Justify why Layer 1 cannot
   cover it; reach for the *lowest* tier that works (a native fd-passing test
   beats a container; a container beats a VM; a VM beats a live-host script).

## Retiring a test

Retirement is ledger-tracked. Create
`tests/migration-state.d/<name>.toml` (`status = "retired"`,
`successor_ids = [...]`), remove the script, sweep its references out of the
orchestrators (`tests/static*.sh`) and CI, keep its basename in the
`tests/tools/gen-migration-ledger.sh` inventory, then
`bash tests/tools/gen-migration-ledger.sh && bash tests/tools/gen-migration-ledger.sh --check`.
If the successor is a fail-closed native/contract test, pin its exact
`cargo nextest list` path in `tests/golden/pinned/<name>.txt` and confirm with
`bash tests/tools/assert-pinned-tests.sh`.

## Directory map (what lives where)

```
tests/
├── static.sh / runner.sh / test-*.sh                         orchestrators (entry points)
├── lib.sh / cli-rust-native-common.sh                              shared shell harness
├── README.md / AGENTS.md                                           docs (human guide + this file)
├── migration-ledger.toml / migration-state.d/                      retirement ledger + records
├── golden/ / fixtures/                                             shared test data + fixtures
├── tools/                                                          runners + codegen/asserter tools
├── unit/
│   ├── nix/      (cases/, pinned/, eval-cases/)                     type 1 eval cases
│   ├── smoke/                                                      type 6 smoke/check defs
│   ├── meta/                                                       meta gates (closed set)
│   └── gates/                                                      drift/perf gates (closed set)
├── integration/
│   ├── containers/                                                 type 9 podman (make test-integration; host/manual pre-PR)
│   ├── distro-matrix/                                              distro pins/fixtures
│   └── live/                                                        type 11 D2B_LIVE (manual)
└── host-integration/
    ├── *.nix                                                       type 10 runNixOSTest (make test-host-integration; host/manual pre-PR)
    └── hardware/                                                   type 12 device tests (manual)
```

Types 2-5 (unit/integration/contract/policy-lint) are Rust and live under
`packages/`, not here.

## Layer-1 orchestration manifest

`tests/layer1-jobs.json` is the source of truth for the Layer-1 PR/local gate
graph. Edit it when changing which `make test-*` targets belong to the
PR-equivalent gate, then run `make layer1-workflow` to regenerate
`.github/workflows/pr-l1-static-fast.yml`. `make test-drift` runs
`make layer1-workflow-check` via the manifest tool and fails if the committed
workflow was edited by hand or not regenerated.

The generated workflow intentionally exposes one stable final `check` job for
branch protection. Keep intermediate job/matrix names as generated
implementation details unless a required-context migration explicitly needs
them preserved.

### Running the Rust suites

Rust tests execute under `cargo-nextest`. Two surfaces are not nextest
surfaces, so each workspace runs them explicitly and you must not fold them
back into a single invocation:

- **Doctests.** nextest does not run them. Several here are `compile_fail`
  capability seals (`AdmittedMutation`, `OwnerIndexMutation`), so dropping
  them removes a trust boundary without failing anything.
- **Harness-free test and bench targets.** They expose no nextest execution
  surface, so nextest builds them but does not run their assertions.
  `d2b-core-smoke` is a test target and the routing and reaction benchmarks
  are bench targets; all carry real assertions. The set is derived from
  `nextest list` zero-case test suites plus Cargo metadata bench targets rather
  than pinned, and each target is run once with its matching `--test` or
  optimized `--release --bench` selector, so a new one cannot silently drop
  out of the gate or turn a performance contract into a debug-build timing.

The privileged broker workspace stays on `cargo test`. Its tests are not
process-per-test safe, and it runs 528 tests in about 1.4 s, so nextest has
nothing to win there.

`make test-rust` owns the bounded local GNU Make DAG. Its stable leaves cover
the API, main format/clippy/workspace, conditional fixture/CLI, broker,
guest-shell-runner, no-bash AST, schema, supply-chain, stub, and pinned-test
surfaces. Fixture and CLI leaves use an isolated stable target below
`.scratch/rust-test-cache`, so they can overlap the main workspace without
sharing mutable Cargo state; `D2B_SKIP_FIXTURE_BUILD=1` omits them for the
Layer-1 graph. The focused `make test-rust-main` retains the same conditional
fixture behavior. The public and private rustdoc censuses use separate stable
targets and overlap only when the API leaf has at least two admitted jobs,
with split Cargo quotas bounded by that leaf's budget. The snapshot checker
uses its release profile for the measured CPU-bound JSON pass. Budgets through
nine admit one job per active lane; surplus jobs above nine go to the measured
API long pole while the full nine-lane frontier remains within budget. Direct
`tests/test-rust.sh` calls require exactly one leaf mode and must not be used
as an aggregate scheduler. The broker passes remain
serial, and the main workspace, schema, and inventory leaves retain their
dependency edges.

Those dependency edges are warm-local-profile only. CI dispatches API, main,
broker, guest, no-bash, schema, inventory and supply-chain Make targets as
eight separate jobs, each with the full runner budget. When a local aggregate
starts without `packages/target`, its cold profile restores shared
workspace targets while retaining the warm-local split API census cache across
`make clean`. It overlaps a bounded API/main/broker prebuild frontier, then
runs fixture, inventory and schema as a full-budget chain so discovery reuses
all prior builds before schema generation. CI alone uses the shared API census
target.

The local Rust budget control is `D2B_RUST_BUDGET`, a positive requested upper
bound. Its automatic cap uses logical CPUs and cache-adjusted available memory,
reserves 2 GiB for the host, and budgets 3 GiB per heavy job. A visible but
unreadable cgroup v2 memory controller fails closed to budget 1. Cargo and
nextest quotas are derived so every active frontier stays within the effective
budget, including budget 1. Top-level Make `-j` is not the Rust budget knob.

### Nix-unit execution

`make test-nix-unit` uses `nix-eval-jobs --no-instantiate` against the locked
`nixUnitJobs.<system>` output. The output contains exactly one aggregate job
per current `*.nix` case file, named bijectively as
`case-<basename>`, plus the `nix-unit` shard/pin integrity job. The `fileJobs`
constructor reuses the same
`casesFor`/`resultsFor`/failure-report semantics as the seven existing flake
shard checks, so every file job reports all of its real
`FAIL <case>: <detail>` lines. No installable is submitted and no derivation
is realized.

The file-job count and the corpus case count are **derived, not contractual**,
so this document states the rule rather than a number. The bijection is the
contract; the cardinality is whatever the corpus currently holds, and the
pinned case list is what fails closed when it changes. Derive either count
from the tree:

```bash
ls tests/unit/nix/cases/*.nix | wc -l            # file jobs, one per case file
grep -hvc '^#' tests/unit/nix/pinned/common.txt  # cases present on every system
grep -hvc '^#' tests/unit/nix/pinned/$(nix eval --raw --impure \
  --expr builtins.currentSystem).txt             # extra cases on this system
```

The corpus total is **system-dependent**: it is the common pin count plus the
native-system pin count, so a single hardcoded total would be wrong on at least
one supported system. Hardcoding either number also puts this file one case
addition behind the tree, which is how the previous figures went stale.

The seven existing flake checks remain the stable manifest leaves:
`nix-unit`, `nix-unit-daemon`, `nix-unit-guest`, `nix-unit-misc`,
`nix-unit-network`, `nix-unit-runtime`, and `nix-unit-state`.
The single locked `nixUnitInventory.<system>` output contains sorted
`caseNames` and sorted `jobNames`, including integrity, and does not force case
expressions. The runner evaluates it once with a `git+file` flake reference,
compares result attrs by exact symmetric difference with `jobNames`, and
compares `caseNames` exactly with `tests/unit/nix/pinned/common.txt` plus the
native-system pin file. Missing or unexpected names fail closed and retain the
exact `run make nix-unit-pin` remedy. Do not replace this with a
repository-specific worker loop or a second scheduler.

The target enters `devShells.<system>.nix-unit` once when `nix-eval-jobs` or
`jq` is missing. That focused shell is a standard `mkShellNoCC` output backed
by the locked flake inputs; an existing toolchain or development shell runs
directly. `D2B_NIX_UNIT_JOBS` is retired and returns status 2 with a migration
message naming `D2B_NIX_UNIT_WORKERS`. Use that bounded operator-intent
control. Its effective count is capped by four workers, logical CPUs, any
finite cgroup CPU quota, and available memory after a 3 GiB host reserve at
the evaluator limit plus 2048 MiB of process and flake overhead per worker.
The full local runner defaults to four workers and a 4096 MiB evaluator limit.
`D2B_NIX_UNIT_MEMORY_MB` may set the limit from 512 through 4096 MiB.
Successful full runs suppress raw JSONL output. Every real `FAIL <case>:
<detail>` line from an aggregate error is parsed and printed as one concise,
path-sanitized stderr entry. Repository and home roots become fixed
placeholders; Nix store hashes are hidden without removing derivation names.
Source-code template lines such as `${result.name}` are excluded. If an
aggregate has no real FAIL line, one final fallback diagnostic remains
attributable to that result attribute.
Command progress uses the fixed path-free `d2b` flake label.

Nix-unit fixtures must not inherit environments, VMs, components, or rendered
artifact surfaces that the case does not assert. Keep a schema-only base for
option/assertion cases and add focused env/VM layers only where the contract
needs them. Retain at least one full positive and one full negative
`nixosSystem` integration path per affected module family. Identical scenario
configurations share one evaluated thunk while preserving one case per
contract. Cardinality boundaries use three tiers: pure limit/helper checks, a pure
production counter composed with the helper at boundary rows, and a small
real-wiring config pinning declarations -> index -> counter -> assertion
records. Do not force an internal `_index` value or materialize hundreds of
typed submodules solely to test a fixed count.

`D2B_NIX_UNIT_CHECK` remains the manual single-shard selector. When
set, it exits through the selected Nix check before eval-jobs bootstrap or
resource accounting. Hosted CI retains the pre-change discovery and
per-check matrix because the full eval-jobs path did not fit the hosted
runner envelope. When
`D2B_EXECUTION_MANIFEST` is set, enter the shared secure lifecycle before Nix
discovery or toolchain entry. A full pass records exactly these seven leaves:
`nix-unit`, `nix-unit-daemon`, `nix-unit-guest`, `nix-unit-misc`,
`nix-unit-network`, `nix-unit-runtime`, and `nix-unit-state`. A selected pass
records only its selected leaf. Reuse
`tests/tools/execution-manifest.pl`; do not weaken its locking, cleanup, or
interruption handling.

`D2B_EXECUTION_MANIFEST=<path>` opts the Rust aggregate into execution evidence.
The binding v1 schema and secure lifecycle live in
[`../docs/reference/test-execution-manifest.md`](../docs/reference/test-execution-manifest.md).
The prior record is invalidated before dispatch, fragments are same-filesystem
atomic evidence, and failed or handled-interruption runs publish partial
status. This evidence supplements source discovery and does not replace
`make test-policy` or
`D2B_ENABLE_FIXTURE_BUILD=1 make test-fixture-contracts`.

Tests that shell out to `cargo` cache their scratch trees between runs under
`.scratch/rust-test-cache/`, keyed on `rustc -vV`, because compiled artifacts
are not portable across compiler versions. CI restores that directory as one
cache surface. When adding such a test, key its subtree the same way and reuse
the compilation but not any output whose freshness the test asserts on.

### External compile-fail test policy

External compile-fail tests are a high-latency exception, not a general API
visibility test mechanism. Before adding one, first use the compiler-derived
rustdoc JSON census and snapshots under `tests/golden/api-surface/`. That
census owns unexpected exports, public members, hidden-public items, and
approved capability trait implementations without launching one Cargo build
per probe.

Add an external compile-fail fixture only when it proves a downstream
type-system or trust-boundary property that rustdoc JSON and doctests cannot
prove. The public capability types have explicit rustdoc `compile_fail`
examples for every prohibited `Clone`, `Default`, and `From<()>` case, and the
private `SessionAuthority` trait has an explicit doctest proving it is not
available downstream. The test comment and changelog entry must state which
semantic property requires a downstream crate. Do not add fixtures solely for
private fields, private modules, constructors, sealed public traits, missing
re-exports, or absent methods; those belong in the API census or a small
rustdoc `compile_fail` example.

The resource API external seal retains one forced-`cfg(test)` downstream probe
because rustdoc JSON does not render test-configuration exports. Its redundant
export/private-member probes were removed in favor of the compiler-derived API
census and rustdoc examples. Keep new cargo-shelling tests under
`rust-test-cache/` unless their tree is large enough to justify a different
cache trade, and document that trade.

The runtime ledger's per-test wall-clock ceiling is 60 seconds. A test sample
above that limit fails the CI gate; shorter advisory thresholds remain
diagnostic only. Do not add an exception for a slow test without either
removing the unnecessary work or documenting why the test cannot be split or
made cheaper.

When a failure reproduces only inside the gate's toolchain environment, use
`tests/tools/repro-rust-gate-env.sh <command>` instead of re-running the whole
gate.

### Standalone Rust workspaces

Most Rust crates are members of `packages/Cargo.toml`, but some crates are
intentionally excluded because they require a distinct safety or dependency
policy. The privileged broker lives at `packages/d2b-priv-broker/`; the
persistent-shell feasibility helper lives at
`packages/d2b-guest-shell-runner/`.

Tests for those excluded workspaces still follow the same taxonomy: Type 2 unit
tests live under `src/**`, Type 3 binary/integration tests live under
`packages/<crate>/tests/*.rs`, and Type 6 static/supply-chain assertions live in
existing `flake.checks.<system>.*` entries. Do not add a new top-level
`tests/*.sh`; extend the existing Rust/static orchestrators by manifest path.
