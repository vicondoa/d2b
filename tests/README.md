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
│   ├── containers/                                              type 9: podman (make test-integration)
│   ├── distro-matrix/                                           distro pins + fixtures
│   └── live/                                                    type 11: NL_LIVE live-host (manual)
└── host-integration/
    ├── *.nix                                                    type 10: runNixOSTest (make test-host-integration)
    └── hardware/                                                type 12: real-device tests (manual)
```

Rust tests (types 2–5: unit, integration, contract, policy-lint) live under
`packages/<crate>/`, **not** here.

## Running tests

| Command | Runs | Where |
|---------|------|-------|
| `make test-unit` | **L1 umbrella**: lint + rust + proofs + flake + drift + policy | local + CI (parallel jobs) |
| `make test` | `test-unit` + `test-integration` | local |
| `make test-lint` | preflight + nix-parse + shellcheck | local + CI |
| `make test-rust` | comprehensive Rust gate (fmt, clippy, cargo test, contract, broker ×3, deny/audit) | local + CI |
| `make test-proofs` | standalone proofs/ crates | local + CI |
| `make test-flake` | `nix flake check --no-build` (native system) | local + CI (2-arch matrix) |
| `make test-nix-unit` | nix-unit corpus (already covered by test-flake; focused convenience target) | local |
| `make test-drift` | drift-check + vms-json-parity | local + CI |
| `make test-policy` | meta gates (ci-coverage, ci-uses-make, adr-index, etc.) | local + CI |
| `make test-integration` | type-9 podman container tests | **ubuntu CI + local** (podman) |
| `make test-host-integration` | type-10 runNixOSTest VM checks | **local NixOS host w/ KVM** (manual; TCG fallback) |
| `make check-tier0` | sub-60s syntax + shellcheck gate | local + CI |
| `make check-fast` | alias for `test-unit` (backward compat) | local + CI |
| `make check` | full Layer-1 gate (`tests/static.sh`) | local + CI |
| `NL_LIVE=1 bash tests/integration/live/<x>.sh` | type-11 live-host tests | **manual, against a deployed nixling host** |

CI runs the individual sub-targets (`test-lint`, `test-rust`, etc.) in parallel.
Locally, `make test-unit` runs them serially.

Useful knobs:
- `NL_NO_SCCACHE=1` — disable sccache in the rust gate (it is auto-disabled in CI).
- `NL_NO_PARALLEL_BROKER=1` — run the broker feature passes serially.
- The rust gate uses **sccache** locally (a shared per-crate compilation cache)
  and runs the broker's three feature passes (default / layer1-bootstrap /
  fake-backends) concurrently with the main workspace.

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
