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
  meta gates).
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
| `make test-unit` | **L1 umbrella** from `tests/layer1-jobs.json`: lint + rust + proofs + flake + nix-unit + drift + policy + runtime-ledger | local + CI (parallel jobs) |
| `make test` | `test-unit` + `test-integration` | local host; still run `make test-host-integration` before opening an agent-owned PR |
| `make test-lint` | preflight + nix-parse + shellcheck | local + CI |
| `make test-rust` | comprehensive Rust gate (fmt, clippy, cargo test, contract, broker ×3, deny/audit) | local + CI |
| `make test-proofs` | standalone proofs/ crates | local + CI |
| `make test-flake` | `nix flake check --no-build` (native system); `D2B_FLAKE_CHECK=<name>` instantiates one check, `D2B_FLAKE_OUTPUTS=1` sweeps non-`checks` outputs, `D2B_FLAKE_LOCAL_SHARDS=1` runs the local bounded shard fan-out | local + CI (x86 sharded per-check matrix; aarch64 PR job runs a lightweight smoke eval) |
| `make test-flake-list` | emit native-system flake check names as JSON (CI matrix plumbing) | CI (dynamic matrix) |
| `make test-nix-unit` | sharded nix-unit corpus checks, retained as explicit evidence in the manifest-driven local and CI graph | local + CI |
| `make test-drift` | drift-check + vms-json-parity + flake-check-matrix-sync | local + CI |
| `make test-policy` | meta gates (ci-coverage, adr-index, deliverable inventory, etc.) | local + CI |
| `make test-runtime-ledger` | hermetic execution-budget gate: after a warm build, times the pinned closed crate census (`tests/runtime-ledger-census.json`) and enforces absolute per-test and per-crate p95 budgets (holds no baseline; makes no historical-regression claim) | local + CI |
| `make test-integration` | type-9 podman container tests | **local host/manual pre-PR** (podman; not the PR pipeline) |
| `make test-host-integration` | type-10 runNixOSTest VM checks | **local NixOS host w/ KVM**, manual pre-PR (not the PR pipeline; TCG fallback) |
| `make check-tier0` | sub-60s syntax + shellcheck gate | local + CI |
| `make check-fast` | alias for `test-unit` (backward compat) | local + CI |
| `make check` | PR-equivalent Layer-1 gate from `tests/layer1-jobs.json` with bounded local parallelism | local |
| `make check-static` | legacy/full-static monolithic gate (`tests/static.sh`) | local |
| `make layer1-workflow` | regenerate `.github/workflows/pr-l1-static-fast.yml` from `tests/layer1-jobs.json` + template | local |
| `make layer1-workflow-check` | verify the generated workflow is up to date | local + CI via `make test-drift` |
| `make flake-matrix-pin` | regenerate the CI flake-check-matrix drift pin after adding/removing a flake check | local |
| `make nix-unit-pin` | regenerate the nix-unit case-presence pins | local |
| `cargo run --manifest-path packages/Cargo.toml -p xtask -- heavy-gate -- env D2B_LIVE=1 bash tests/integration/live/<x>.sh` | type-11 live-host tests, through the heavy-gate semaphore | **manual, against a deployed d2b host** |

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
each live/hardware/perf entrypoint re-executes itself through the gate
exactly once when `D2B_HEAVY_GATE` is unset, so the shared Nix store,
cargo target directory, and KVM device cannot be oversubscribed. The gated
targets remain the documented path. The `cargo run --manifest-path
packages/Cargo.toml` spelling is required because there is no root cargo
workspace, so the bare `cargo xtask` alias resolves only when run from
`packages/`; see AGENTS.md for the `sccache` tradeoff and the `cd packages
&& cargo xtask <command>` alternative.

The semaphore uses a protected, system-provisioned namespace under
`/run/d2b-heavy-gates`; it never falls back to a user-writable runtime or
temporary directory. The NixOS module provisions the fixed root at boot and
creates two private slots for every configured `d2b.site.launcherUsers` member
after numeric UIDs are available. On a development machine that does not use
the module, run `make heavy-gate-provision` once after each boot. That target
uses `sudo` only to create the root-owned namespace and the current user's two
mode-`0600` slot files. A missing or malformed namespace fails closed with
stable code `heavy-gate-provisioning-required` and names that Make target as
the remediation; do not work around it by moving the gate into `/tmp` or
another user-owned location.

Current live-host scripts include `d2b-store.sh` for per-VM store
adoption and `usbip-guestd-lifecycle.sh` for USBIP guestd attach/detach across
a `d2bd` restart. The USBIP script requires
`D2B_USBIP_VM=<vm>` and `D2B_USBIP_BUSID=<busid>` and uses only `d2b usb`
verbs for USB state changes.

`tests/layer1-jobs.json` is the central Layer-1 job graph. `make check` and
`make test-unit` consume it directly; `.github/workflows/pr-l1-static-fast.yml`
is generated from it by `make layer1-workflow` and checked by
`make layer1-workflow-check` during `make test-drift`. CI runs the individual
Layer-1 sub-targets (`test-lint`, `test-rust`, etc.) in parallel and exposes a
stable final `check` rollup job intended for branch protection.

The `test-runtime-ledger` job is part of that graph. It is an absolute
per-test and per-crate execution-budget gate: it warm-builds the pinned census
crate, records execution-only p95s, and fails any p95 over its frozen budget or
a census that does not reproduce the pin exactly. It holds no baseline and makes
no historical-regression claim - a slower run that still fits its budget passes.
Growing the census to a real multi-crate shard inventory (with a per-shard
budget) and adding a cross-machine reference baseline for a true
historical-regression gate is the deferred follow-up
`runtime-ledger-full-census-and-real-shards`. If the description above diverges
from the current `Makefile` target or `tests/layer1-jobs.json`, treat those as
authoritative and flag the drift for the integrator rather than hand-editing the
census pin.

The x86 `test-flake` leg is sharded one job per flake check (the matrix is
enumerated at CI time by `make test-flake-list`; the `test-flake-x86` job is a
stable aggregator over the shards + the non-`checks` outputs job). The aarch64
leg runs only the lightweight `smoke-eval-aarch64.nix` expression. A fail-closed
drift gate keeps the matrix and smoke wiring in sync with the flake (`make
flake-matrix-pin` to update its pin). Locally, manifest-driven `make check`
sets `D2B_FLAKE_LOCAL_SHARDS=1` for `make test-flake` and
`D2B_SKIP_FIXTURE_BUILD=1` for `make test-rust`, matching the PR Rust job because
the fixture checks run in the flake shard set; tune `D2B_CHECK_JOBS` and
`D2B_FLAKE_JOBS` for host capacity. Agent-owned PRs also run
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
