# nixling tests

How the test suite is organized, where each kind of test lives, and how to run
and add them. For the **decision rule on where a new test goes** (and the rule
that you must *not* add new ad-hoc `tests/*.sh`), read [`AGENTS.md`](./AGENTS.md)
— that is the binding contract; this file is the human quick-start.

## Two layers

- **Layer 1 — static gate.** Hermetic, fast, deterministic; no live host, VM, or
  container. Runs on every PR and locally via `make check`. This is where the
  overwhelming majority of tests live (Nix eval cases, Rust unit/integration/
  contract/policy-lint tests, flake checks, and a small closed set of drift +
  meta gates).
- **Layer 2 — integration tiers.** Real systemd / kernel / userland: podman
  containers, runNixOSTest VMs, live-host scripts, and hardware tests. Used only
  when Layer 1 *provably* cannot cover the behaviour.

## Directory structure

```
tests/
├── static.sh, static-fast-tier0.sh, runner.sh, test-*.sh          orchestrators (entry points)
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
│   └── live/                                                    type 11: NL_LIVE live-host (manual)
└── host-integration/
    ├── *.nix                                                    type 10: runNixOSTest (make test-host-integration; host/manual pre-PR)
    └── hardware/                                                type 12: real-device tests (manual)
```

Rust tests (types 2–5: unit, integration, contract, policy-lint) live under
`packages/<crate>/`, **not** here.

## Running tests

| Command | Runs | Where |
|---------|------|-------|
| `make test-unit` | **L1 umbrella**: lint + rust + proofs + flake + drift + policy | local + CI (parallel jobs) |
| `make test` | `test-unit` + `test-integration` | local host; still run `make test-host-integration` before opening an agent-owned PR |
| `make test-lint` | preflight + nix-parse + shellcheck | local + CI |
| `make test-rust` | comprehensive Rust gate (fmt, clippy, cargo test, contract, broker ×3, deny/audit) | local + CI |
| `make test-proofs` | standalone proofs/ crates | local + CI |
| `make test-flake` | `nix flake check --no-build` (native system); `NL_FLAKE_CHECK=<name>` instantiates one check, `NL_FLAKE_OUTPUTS=1` sweeps non-`checks` outputs | local + CI (x86 sharded per-check matrix; aarch64 PR job runs a lightweight smoke eval) |
| `make test-flake-list` | emit native-system flake check names as JSON (CI matrix plumbing) | CI (dynamic matrix) |
| `make test-nix-unit` | sharded nix-unit corpus checks (already covered by test-flake; focused convenience target) | local |
| `make test-drift` | drift-check + vms-json-parity + flake-check-matrix-sync | local + CI |
| `make test-policy` | meta gates (ci-coverage, ci-uses-make, adr-index, etc.) | local + CI |
| `make test-integration` | type-9 podman container tests | **local host/manual pre-PR** (podman; not the PR pipeline) |
| `make test-host-integration` | type-10 runNixOSTest VM checks | **local NixOS host w/ KVM**, manual pre-PR (not the PR pipeline; TCG fallback) |
| `make check-tier0` | sub-60s syntax + shellcheck gate | local + CI |
| `make check-fast` | alias for `test-unit` (backward compat) | local + CI |
| `make check` | full Layer-1 gate (`tests/static.sh`) | local + CI |
| `make flake-matrix-pin` | regenerate the CI flake-check-matrix drift pin after adding/removing a flake check | local |
| `make nix-unit-pin` | regenerate the nix-unit case-presence pins | local |
| `NL_LIVE=1 bash tests/integration/live/<x>.sh` | type-11 live-host tests | **manual, against a deployed nixling host** |

Current live-host scripts include `nixling-store.sh` for per-VM store
adoption and `usbip-guestd-lifecycle.sh` for USBIP guestd attach/detach across
a `nixlingd` restart. The USBIP script requires
`NL_USBIP_VM=<vm>` and `NL_USBIP_BUSID=<busid>` and uses only `nixling usb`
verbs for USB state changes.

CI runs the individual Layer-1 sub-targets (`test-lint`, `test-rust`, etc.) in parallel.
The x86 `test-flake` leg is sharded one job per flake check (the matrix is
enumerated at CI time by `make test-flake-list`; the `test-flake-x86` job is a
stable aggregator over the shards + the non-`checks` outputs job). The aarch64
leg runs only the lightweight `smoke-eval-aarch64.nix` expression. A fail-closed
drift gate keeps the matrix and smoke wiring in sync with the flake (`make
flake-matrix-pin` to update its pin).
Locally, `make test-unit` runs the sub-targets serially and `make test-flake`
runs the full native check. Agent-owned PRs also run `make test-integration`
and `make test-host-integration` on the host before the PR is opened; those
manual integration tiers are not replaced by PR pipeline jobs.

Useful knobs:
- `NL_NO_SCCACHE=1` — disable sccache in the rust gate.
- `NL_CI_SCCACHE=1` — opt the rust gate back into sccache under CI (off by
  default there; `pr-l1-static-fast` sets it and backs `SCCACHE_DIR` with
  `actions/cache`, using sccache's local-disk backend — never the native GHA
  backend, which would export `ACTIONS_RUNTIME_TOKEN` into the build env).
- `NL_NO_PARALLEL_BROKER=1` — run the broker feature passes serially.
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
  `CARGO_BIN_EXE_*`. **Spawn hermetically**: point `NIXLING_PUBLIC_SOCKET`,
  `NIXLING_BROKER_SOCKET`, and the `NIXLING_*_PATH` fixture env vars at fixtures
  or missing paths so the test never touches the operator's live daemon.
- Rendered-artifact ↔ DTO/doc contract → a contract test in
  `packages/nixling-contract-tests/`.
- Generated docs/schemas/CLI freshness → already a drift gate; regenerate with
  `cargo run -p xtask -- gen-*`. Do **not** add a new shell gate.

Only reach for Layer 2 (containers / VMs / live-host / hardware) when a foreign
userland, a real systemd boot, a live host, or a physical device is genuinely
required — and pick the lowest tier that works.

## Conventions

- **Commit before building.** `nix flake check` and the eval gates resolve the
  flake via `git+file://`, which only sees git-tracked files — an untracked new
  module/test is invisible until committed.
- **Retiring a test is ledger-tracked** (`tests/migration-state.d/<name>.toml` +
  `tests/tools/gen-migration-ledger.sh --check`); fail-closed native successors
  are pinned in `tests/golden/pinned/` and checked by
  `tests/tools/assert-pinned-tests.sh`.
