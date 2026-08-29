# AGENTS.md - the d2b test model (read before adding a test)

This file defines **where new tests go and how they run**. It prevents the
failure mode behind the test rearchitecture: agents adding ad-hoc
`tests/*.sh`, making the suite slow and unmaintainable. For coverage changes,
follow the decision rule below. human-facing structure and run instructions
live in
[`README.md`](./README.md).

## The one rule

**New coverage MUST land as a Layer-1 test (types 1-6 below) unless it
*provably* requires a real container, booted VM, live host, or hardware.** No
"type 7/8" escape hatch: drift and meta gates are a **closed set** - do not add
a new `tests/*.sh`. A needed shell gate almost certainly belongs as a nix-unit
case (type 1) or Rust test (types 2-5).

That closed set covers *gates*. `tests/tools/` is the open home for gate and CI
plumbing - generators and runners - when it is not a test case. A gate asserts
an invariant and belongs to the closed set; a tool produces data for another
gate to assert. Do not add evidence, migration, wave, or ADR shell scripts.

When in doubt, push the test *down* toward type 1, not up.

## Taxonomy - name, definition, home, how it runs

### Layer 1 - static gate (hermetic, fast, available in CI and locally via
`make check`; focused component tests are sufficient for review)

| # | Type | What it is | Lives in |
|---|------|------------|----------|
| 1 | **eval case** | declarative pure-Nix assertion (`{ expr; expected; }` / `{ expr; expectedError; }`) over module-config values + eval-rejection | `tests/unit/nix/surfaces/*.nix`, with explicit case/module inputs in Bazel |
| 2 | **unit test** | `#[test]` over one crate's pure logic | `packages/<crate>/src/**` `#[cfg(test)]` |
| 3 | **integration test** | spawns the real binary (`CARGO_BIN_EXE_*`) over AF_UNIX/fd-passing; no host mutation | `packages/<crate>/tests/*.rs` |
| 4 | **contract test** | Rust assertion over a **rendered** Nix artifact (bundle / host-json / processes.json) - the Nix↔Rust + doc↔impl boundary | `packages/<crate>/tests/*.rs` (`D2B_FIXTURES`) |
| 5 | **policy lint** | One of the four retained repository-wide policy classes | `packages/xtask/tests/*.rs` or the fixed source-hygiene gate |
| 6 | **flake check** | realized example-config eval / supply-chain (`eval-*`, `rust-deny/audit`) | `flake.checks.<sys>.*`; smoke/check defs in `tests/unit/smoke/`, eval-case libs in `tests/unit/nix/eval-cases/` |

The remaining Layer-1 surface is a **closed set** you should not grow with new
files: owner-local generated-artifact actions under `packages/xtask/` and
**meta gates** (`tests/unit/meta/` - guard the test infra itself).

Fixture-backed type 4 tests are owned by their product crate and included by
the fixed fixture aggregate when Nix is available. The fixture target
materializes `D2B_FIXTURES` from evaluated Nix artifact data and invoking it
without the enforcing lane's declared environment fails rather than skipping.
Repository-wide policy is limited to source hygiene, workspace and lock
integrity, supply chain, and changelog policy.

`test-policy` does not run the unscheduled
`tests/tools/guest-workspace-drift.py` helper.
`//tests/unit/meta:w0_dep_direction` owns the retained workspace-and-lock
policy class, but it does not assert copied Guest workspace parity. Do not cite
that parity as passing gate evidence. When a shared crate mirrored into the
Guest workspace gains or changes a dependency, update the fixture and any
affected override, refresh `packages/Cargo.guest.lock`, and run the applicable
owner-local targets plus `make test-rust-supply-chain` and `make test-policy`.
The supply-chain lane realizes the copied Guest workspace for dependency
metadata, license, source, and audit validation; it does not compile Guest
packages and is not a fifth repository-wide policy class or copied-workspace
parity result.

### Layer 2 - integration tiers (only when Layer 1 genuinely can't cover it)

| # | Type | What it is | Lives in | Runs **where** |
|---|------|------------|----------|----------------|
| 9 | **container** | Nix-OCI image under rootless podman; proves a static binary runs on a foreign non-Nix userland | `tests/integration/containers/*.sh` + `containerImages.<sys>.*` | `make test-integration` - conditional local host lane when the changed surface needs a foreign userland |
| 10 | **VM (runNixOSTest)** | boots a real NixOS VM; asserts live daemon/broker/socket-activation/host-posture/kernel behaviour | `tests/host-integration/*.nix` + `vmChecks.<sys>.*` | `make test-host-integration` - conditional NixOS/KVM lane when the changed surface needs host behavior |
| 11 | **live-host** | runs against a **real deployed** d2b host; destructive/stateful | `tests/integration/live/*.sh` | through the Bazel-built xtask heavy-gate semaphore; `D2B_LIVE=1` / sudo - **manual, never CI** |

Every retained Layer-2 tier (9-11) runs behind the Bazel-built xtask heavy-gate sole-use
semaphore, never as a raw script. Use the gated public lane target
(`make test-integration`, `make test-host-integration`;
`make pre-tag` / `make smoke-lite` for the live-VM smoke gate), or wrap an
ad-hoc live script as
`make heavy-gate-build && bazel-bin/packages/xtask/xtask heavy-gate -- env
D2B_LIVE=1 bash tests/integration/live/<name>.sh`.

Invoking a live script directly no longer bypasses the semaphore: it re-executes
through the gate exactly once when `D2B_HEAVY_GATE` is unset, so shared Nix
store, Bazel output tree, and KVM are not oversubscribed. **Any new live or
performance entrypoint must carry that same self-guard block**, or the
fail-closed inventory guard (`every_live_and_heavy_entrypoint_routes_through_the_gate`)
fails while walking on-disk scripts and the Makefile.

## How to add a test (decision rule)

1. **Asserting a Nix module value / option / eval-rejection?** → type 1, add
   the smallest owner-local expression to its named file under
   `tests/unit/nix/surfaces/` and declare its exact case, module, helper, and
   fixture inputs in `bazel/checks/nix/BUILD.bazel`. Do not add a corpus
   discovery rule, pin file, or aggregate inventory.
2. **Asserting Rust logic?** → type 2, a `#[test]` in that crate's `src`.
3. **Asserting the real binary's wire/CLI behaviour?** → type 3, a test in
   `packages/<crate>/tests/*.rs` against `CARGO_BIN_EXE_*`. Spawn hermetically -
   point `D2B_PUBLIC_SOCKET` / `D2B_BROKER_SOCKET` / `D2B_*_PATH` at
   fixtures or missing paths so the test never touches the operator's live
   daemon.
4. **Asserting that a *rendered* Nix artifact matches a Rust DTO / doc?** →
   type 4, a contract test in the owning crate's `packages/<crate>/tests/`
   (driven by `D2B_FIXTURES`).
5. **Asserting a generated artifact is up to date (docs/schemas/CLI)?** → it is
   already covered by a **drift gate**; regenerate with the matching
   `bazel run //packages/xtask:xtask -- gen-*` and commit - do **not** add a
   new gate.
6. **Genuinely needs a foreign userland / real systemd boot / live host?** →
   the matching Layer-2 tier (9-11). Justify why Layer 1 cannot cover it and
   reach for the lowest tier that works. Physical-device validation is manual
   operator work, not a repository evidence script.

## Retiring a test

Delete the test and sweep its Bazel, Make, CI, and documentation references.
Preserve behavior only when a current owner-local test or structural boundary
is still needed. Do not add retirement ledgers, successor pins, evidence
scripts, or replacement inventory machinery.

## Directory map (what lives where)

```
tests/
├── lib.sh / cli-rust-native-common.sh                              shared shell harness
├── README.md / AGENTS.md                                           docs (human guide + this file)
├── golden/ / fixtures/                                             shared test data + fixtures
├── tools/                                                          Bazel facade, runners, codegen, and asserter tools
├── unit/
│   ├── nix/      (surfaces/, cases/, eval-cases/)                    type 1 eval cases
│   ├── smoke/                                                      type 6 smoke/check defs
│   ├── meta/                                                       meta gates (closed set)
│   └── gates/                                                      drift/perf gates (closed set)
├── integration/
│   ├── containers/                                                 type 9 podman (make test-integration; conditional)
│   ├── distro-matrix/                                              distro pins/fixtures
│   └── live/                                                        type 11 D2B_LIVE (manual)
└── host-integration/
    └── *.nix                                                       type 10 runNixOSTest (make test-host-integration; conditional)
```

Types 2-5 (unit/integration/contract/policy-lint) are Rust and live under
`packages/`, not here.

## Layer-1 orchestration and Bazel authority

Bazel is the sole Layer-1 scheduler. The nested suite graph in `BUILD.bazel`
and `bazel/checks/`, including the top-level facade and package-level suites,
owns target selection, dependency ordering, parallelism, retry classification,
caching, and aggregation. Make targets and fixed CI jobs are thin aliases over
one facade suite per public target and must not grow local fan-out, discovery,
sharding, or rollup logic.

Public Make aliases run `bazel test --config=$(D2B_BAZEL_PROFILE)` directly.
Do not wrap `make check` or `make test-*` through `tests/tools/bazel-check`.
That script remains the BuildBuddy credential helper only. Default profile is
`remote`; PR gates set `D2B_BAZEL_PROFILE=local`.

Bazel is the only supported contributor build and test interface. Cargo
manifests and the root `Cargo.lock` own Rust package and dependency facts;
`rules_rs` supplies the Bazel-side Cargo integration. Do not add a second Cargo
lock, source inventory, generator, or repository-owned scheduler.

The fixed CI workflow is committed at
`.github/workflows/pr-l1-static-fast.yml` and exposes one stable required
`check` result. Keep its jobs aligned with the public Make aliases. Do not add
discovery jobs or new inventory files.

### Running the Layer-1 graph

Use the public aliases for focused or complete runs:

```bash
make check-tier0
make test-lint
make test-changelog
make test-rust
make test-proofs
make test-flake
make test-nix-unit
make test-policy
make test-drift
make test-fixture-contracts
make test-unit
make check
```

Each Layer-1 alias performs one direct Bazel invocation over its matching
facade suite. The individual Bazel labels remain directly runnable for focused
reruns.
The performance target is advisory and is not validation evidence when it
reports a guarded skip.

Nix-unit surfaces are fixed Bazel labels with explicit source closures.
Each action copies only its declared runfiles into an isolated source root and
evaluates the surface directly with the Bazel-provided nixpkgs source. The
repository flake outputs, per-test flake input fetching, and ambient
`D2B_REPO_ROOT` do not participate.
The shared evaluator fails closed when a surface evaluates zero cases. Do not
add a test census, successor pin, secondary inventory, or validator.

### Retained Layer-2 and manual scripts

Layer-2 container, VM, live-host, and performance scripts remain
manual or conditional surfaces. They run through the documented heavy-gate
semaphore and are not part of the Bazel Layer-1 scheduler. A shell script may
remain under `tests/tools/` or `tests/unit/` when it is the subject of a
native Bazel test, a fixture materializer, a generator, or a Layer-2 lane; it
must not schedule sibling Layer-1 work.

### Standalone Rust workspaces

Product crates use the repository-root `Cargo.toml` and `Cargo.lock` as
rules_rs metadata. The privileged broker and guest shell runner retain their
explicit feature contexts, while the no-bash walker and compile-fail UI crates
retain their separate tooling boundaries. Doctests, harness-free binaries,
feature variants, fixtures, and policy checks must remain visible as direct
Bazel targets. Standalone Cargo remains technically usable for focused local
debugging, but it is not a contributor or CI authority.
