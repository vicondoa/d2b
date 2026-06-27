# AGENTS.md — the d2b test model (read before adding a test)

This file is the contract for **where a new test goes and how it runs**. It
exists to stop the failure mode that motivated the test rearchitecture: agents
reaching for a new ad-hoc `tests/*.sh` every time, which made the suite slow and
unmaintainable. If you are adding or changing test coverage, follow the decision
rule below. The human-facing structure + run instructions live in
[`README.md`](./README.md).

## The one rule

**New coverage MUST land as a Layer-1 test (types 1–6 below) unless it
*provably* requires a real container, a booted VM, a live host, or physical
hardware.** There is no "type 7/8" escape hatch: the drift gates and meta gates
are a **closed set** — do not add a new `tests/*.sh`. If you think you need a
shell gate, you almost certainly want a nix-unit case (type 1) or a Rust test
(types 2–5) instead.

When in doubt, push the test *down* the tiers (toward type 1), not up.

## Taxonomy — name, definition, home, how it runs

### Layer 1 — static gate (hermetic, fast, every PR + local via `make check`)

| # | Type | What it is | Lives in |
|---|------|------------|----------|
| 1 | **eval case** | declarative pure-Nix assertion (`{ expr; expected; }` / `{ expr; expectedError; }`) over module-config values + eval-rejection | `tests/unit/nix/cases/*.nix` (auto-discovered; pins in `tests/unit/nix/pinned/`) |
| 2 | **unit test** | `#[test]` over one crate's pure logic | `packages/<crate>/src/**` `#[cfg(test)]` |
| 3 | **integration test** | spawns the real binary (`CARGO_BIN_EXE_*`) over AF_UNIX/fd-passing; no host mutation | `packages/<crate>/tests/*.rs` |
| 4 | **contract test** | Rust assertion over a **rendered** Nix artifact (bundle / host-json / processes.json) — the Nix↔Rust + doc↔impl boundary | `packages/d2b-contract-tests/tests/*.rs` (`D2B_FIXTURES`) |
| 5 | **policy lint** | Rust scan of source/docs asserting a tree-wide invariant | `packages/d2b-contract-tests/tests/policy_*.rs` |
| 6 | **flake check** | realized example-config eval / supply-chain (`eval-*`, `rust-deny/audit`) | `flake.checks.<sys>.*`; smoke/check defs in `tests/unit/smoke/`, eval-case libs in `tests/unit/nix/eval-cases/` |

The remaining Layer-1 surface is a **closed set** you should not grow with new
files: **drift gates** (`tests/unit/gates/` — `xtask gen-* + git diff`) and
**meta gates** (`tests/unit/meta/` — guard the test infra itself).

### Layer 2 — integration tiers (only when Layer 1 genuinely can't cover it)

| # | Type | What it is | Lives in | Runs **where** |
|---|------|------------|----------|----------------|
| 9 | **container** | Nix-OCI image under rootless podman; proves a static binary runs on a foreign non-Nix userland | `tests/integration/containers/*.sh` + `containerImages.<sys>.*` | `make test-integration` — **local host/manual pre-PR; not the PR pipeline** |
| 10 | **VM (runNixOSTest)** | boots a real NixOS VM; asserts live daemon/broker/socket-activation/host-posture/kernel behaviour | `tests/host-integration/*.nix` + `vmChecks.<sys>.*` | `make test-host-integration` — **local NixOS host w/ KVM, manual pre-PR; not the PR pipeline** |
| 11 | **live-host** | runs against a **real deployed** d2b host; destructive/stateful | `tests/integration/live/*.sh` | `D2B_LIVE=1` / sudo — **manual, never CI** |
| 12 | **hardware** | real GPU / YubiKey / hardware-TPM passthrough | `tests/host-integration/hardware/*.sh` | **manual on a host with the devices** |

## How to add a test (decision rule)

1. **Asserting a Nix module value / option / eval-rejection?** → type 1, a
   nix-unit case in `tests/unit/nix/cases/`. Add a case file (it is
   auto-discovered; do not edit `default.nix`), then regenerate the pin list
   (`tests/tools/gen-nix-unit-pins.sh`). CI evaluates the corpus through
   sharded `nix-unit-<shard>` flake checks; add new cases to the existing
   topical file whose shard already owns that behavior.
2. **Asserting Rust logic?** → type 2, a `#[test]` in that crate's `src`.
3. **Asserting the real binary's wire/CLI behaviour?** → type 3, a test in
   `packages/<crate>/tests/*.rs` against `CARGO_BIN_EXE_*`. Spawn hermetically —
   point `D2B_PUBLIC_SOCKET` / `D2B_BROKER_SOCKET` / `D2B_*_PATH` at
   fixtures or missing paths so the test never touches the operator's live
   daemon.
4. **Asserting that a *rendered* Nix artifact matches a Rust DTO / doc?** →
   type 4, a contract test in `packages/d2b-contract-tests/` (driven by
   `D2B_FIXTURES`).
5. **Asserting a generated artifact is up to date (docs/schemas/CLI)?** → it is
   already covered by a **drift gate**; regenerate with the matching
   `cargo run -p xtask -- gen-*` and commit — do **not** add a new gate.
6. **Genuinely needs a foreign userland / real systemd boot / live host /
   device?** → the matching Layer-2 tier (9–12). Justify why Layer 1 cannot
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

Types 2–5 (unit/integration/contract/policy-lint) are Rust and live under
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
