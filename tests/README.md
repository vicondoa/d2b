# d2b tests

How the test suite is organized, where each kind of test lives, and how to run
and add them. For the **decision rule on where a new test goes** (and the rule
that you must *not* add new ad-hoc `tests/*.sh`), read [`AGENTS.md`](./AGENTS.md) -
that is the binding contract; this file is the human quick-start.

## Two layers

- **Layer 1 - static gate.** Hermetic, fast, deterministic; no live host, VM, or
  container. Available in CI and locally via `make check`; focused
  component tests are sufficient when a full aggregate is not needed. This is where the
  overwhelming majority of tests live (Nix eval cases, Rust unit/integration/
  contract/policy-lint tests, flake checks, and a small closed set of drift +
  meta gates). The manifest records which jobs are enforcing and which are
  advisory; an advisory success may be a guarded skip and is not validation
  evidence.
- **Layer 2 - integration tiers.** Real systemd / kernel / userland: podman
  containers, runNixOSTest VMs, and live-host scripts. Used only
  when Layer 1 *provably* cannot cover the behaviour. Physical-device
  validation is manual operator work, not a repository evidence script.

## Directory structure

```
tests/
├── lib.sh, cli-rust-native-common.sh                              shared shell harness
├── README.md, AGENTS.md                                           this guide + the test-model contract
├── golden/, fixtures/                                           shared golden data + fixtures
├── tools/                                                       runners + codegen/asserter tools
│                                                                (bazel-check, rust-workspace-checks, gen-*, …)
├── unit/                          ── Layer 1 ──
│   ├── nix/        surfaces/ + cases/ + eval-cases/             type 1: owner-local Nix eval cases
│   ├── smoke/      smoke-eval*.nix                              type 6: smoke / flake-check defs
│   ├── meta/                                                    meta gates (guard the test infra; closed set)
│   └── gates/                                                   drift + perf gates (closed set)
├── integration/                   ── Layer 2 ──
│   ├── containers/                                              type 9: podman (make test-integration; conditional)
│   ├── distro-matrix/                                           distro pins + fixtures
│   └── live/                                                    type 11: D2B_LIVE live-host (manual)
└── host-integration/
    └── *.nix                                                    type 10: runNixOSTest (make test-host-integration; conditional)
```

Rust tests (types 2-5: unit, integration, contract, policy-lint) live under
`packages/<crate>/`, **not** here.

## Running tests

| Command | Runs | Where |
|---------|------|-------|
| `make test-unit` | complete fixed Bazel Layer-1 development graph | local + CI |
| `make test` | `test-unit` + `test-integration` | local convenience aggregate; use wider lanes when the changed surface needs them |
| `make check-tier0` | fast Bazel toolchain and source-policy suite | local + CI |
| `make test-lint` | fixed Bazel source-hygiene and shell-lint suite | local + CI |
| `make test-changelog` | require release notes for code changes and validate every changelog fragment | local + CI |
| `make test-rust` | fixed owner-local Bazel Rust unit, integration, and doctest suite | local + CI |
| `make test-rust-<leaf>` | focused Bazel Rust labels for main, broker, guest shell runner, policy, schema, and supply-chain coverage | CI (local for a focused rerun) |
| `make test-fixture-contracts` | enforcing eval-rendered lane: materializes `D2B_FIXTURES` from evaluated Nix artifact data, then runs owner-local CLI contract cases; invoking it without the enforcing lane fails rather than skipping | local + CI |
| `make test-proofs` | standalone proofs/ crates | local + CI |
| `make test-flake` | fixed Bazel Nix evaluation target | local + CI |
| `make test-nix-unit` | fixed Bazel Nix-unit surface targets | local + CI |
| `make test-drift` | native generated-artifact and parity checks | local + CI |
| `make test-policy` | fixed Bazel source, workspace/lock, supply-chain, and changelog policy suites | local + CI |
| `make test-performance-budgets` | advisory performance canary; without `D2B_PERF_STABLE=1` it reports `SKIP` and enforces nothing | local + CI |
| `make test-integration` | type-9 podman container tests | conditional local host lane (podman; not the PR pipeline) |
| `make test-host-integration` | type-10 runNixOSTest VM checks; set `D2B_VM_CHECK=<name>` for one named check | conditional local NixOS host lane (KVM; TCG fallback; not the PR pipeline) |
| `make check-fast` | alias for `test-unit` (backward compat) | local + CI |
| `make check` | complete fixed Bazel graph with fixed CI enforcement | local |
| `make bazel-check` | Bazel aggregate used by `make check`. Defaults to BuildBuddy remotely; CI forces `D2B_BAZEL_PROFILE=local` | local or remote |
| `make heavy-gate-build && bazel-bin/packages/xtask/xtask heavy-gate -- env D2B_LIVE=1 bash tests/integration/live/<x>.sh` | type-11 live-host tests, through the heavy-gate semaphore | **manual, against a deployed d2b host** |

`make check`, `make test-unit`, and `make bazel-check` invoke the same fixed
target-pattern and owner-suite set through one Bazel call.
`tests/tools/bazel-check --profile local` uses the same facade for focused
reruns. Bazel owns Layer-1 scheduling; Make and CI are thin aliases over fixed
labels. Cargo manifests and `Cargo.lock` remain rules_rs metadata authority,
while standalone crate Cargo commands are not documented or required gate
evidence.

`make test-policy` does not schedule
`tests/tools/guest-workspace-drift.py`. The retained
`//tests/unit/meta:w0_dep_direction` target owns workspace-and-lock policy, but
it does not assert copied Guest workspace parity. Do not cite that parity as
passing gate evidence. When a mirrored shared crate gains or changes a
dependency, update the guest workspace fixture and any affected override,
refresh `packages/Cargo.guest.lock`, and run the applicable owner-local targets
plus `make test-policy`.

All Layer-2 lanes (types 9-11) run behind one sole-use semaphore (two slots
per uid via open file description locks), so concurrent heavy lanes cannot
oversubscribe the shared Nix store, Bazel output tree, or KVM device. The
public lane targets above (`make test-integration`,
`make test-host-integration`, `make perf`) acquire a slot and then delegate
to a guarded internal `heavy-lane-*` target that fails closed if run outside
the gate; run the public targets, not the internal ones. `make heavy-check`,
`make heavy-flake-check`, and the `heavy-test-*` aliases run a Layer-1 gate,
the building flake check, or a public lane under the same semaphore.
Live-host scripts obey the same rule: use the gated `make pre-tag` /
`make smoke-lite` live-VM smoke entrypoints, or wrap a raw live script as
`make heavy-gate-build && bazel-bin/packages/xtask/xtask heavy-gate -- env
D2B_LIVE=1 bash tests/integration/live/<x>.sh`. Invoking `D2B_LIVE=1 bash
tests/integration/live/<x>.sh` directly no longer bypasses the semaphore:
each live entrypoint, plus the enforcing path of each performance
entrypoint, verifies its inherited slot and re-executes itself through the gate
exactly once when no genuine slot is held. The advisory performance skip exits
before acquiring a slot because it does no heavy work. A bare `D2B_HEAVY_GATE`
value is not trusted, so the shared Nix store, Bazel output tree, and KVM
device cannot be oversubscribed. The gated targets remain the documented path.

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

The fixed CI workflow at `.github/workflows/pr-l1-static-fast.yml` runs the
public Make aliases over the Bazel graph and exposes one stable required
`check` result. The graph keeps `test-performance-budgets` advisory when the
stable runner is unavailable; a guarded skip is not validation evidence.

The fixture lane is enforcing and local-only. It fails when
`D2B_ENABLE_FIXTURE_BUILD=1` is absent, materializes its declared fixture
outputs, and runs the fixture-dependent contract and CLI targets exactly once.

### Bazel graph execution

The Layer-1 graph is fixed in `BUILD.bazel` and `bazel/checks/`. Bazel owns
selection, dependency ordering, parallelism, retry classification, caching,
and aggregation. Every public Layer-1 `make test-*` alias is a thin direct
Bazel invocation, and every fixed CI job runs the same graph with the local
profile.
Individual labels remain available for focused reruns.

Cargo manifests and the root `Cargo.lock` remain authoritative for Rust
membership, dependencies, and features consumed by rules_rs. Do not add a
second Cargo lock, source inventory, generator, discovery job, or shell
scheduler.

`tests/tools/bazel-check` retains the BuildBuddy security boundary. It uses
Bazel's credential helper, withholds credentials from untrusted work, redacts
logs and BEP output, and retries the identical target set locally only for a
typed pre-dispatch infrastructure failure. Post-dispatch and test failures
fail closed. Provider measurements do not define a second acceptance gate.

The complete local and CI surfaces are:

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

### Nix-unit surfaces

`make test-nix-unit` runs one fixed Bazel action per named owner surface.
Each action declares its expression, modules, helpers, fixtures, and pinned
external inputs directly in `bazel/checks/nix/BUILD.bazel`; there is no corpus
discovery or case-presence pin generator. The action copies those runfiles into
an isolated source root and evaluates the surface directly through a minimal
runner flake, without the repository flake outputs or ambient
`D2B_REPO_ROOT`. No secondary test census or successor pin is maintained.

No secondary execution record, migration ledger update, successor pin, or
evidence script is required.

### CI and manual lanes

The fixed workflow is committed at `.github/workflows/pr-l1-static-fast.yml`
and exposes one stable required `check` result. Intermediate job names are
implementation details. Layer-2 container, VM, live-host, and performance scripts
remain conditional or manual lanes behind the heavy-gate semaphore; they are
not folded into the Layer-1 Bazel scheduler.

## Adding a test

See [`AGENTS.md`](./AGENTS.md) for the full decision rule. In short, default to
Layer 1:

- Nix module value / option / eval-rejection → an owner-local expression in
  `tests/unit/nix/surfaces/*.nix` with an explicit Bazel input closure.
- Rust logic → a `#[test]` in the crate's `src`.
- Real-binary behaviour → `packages/<crate>/tests/*.rs` against
  `CARGO_BIN_EXE_*`. **Spawn hermetically**: point `D2B_PUBLIC_SOCKET`,
  `D2B_BROKER_SOCKET`, and the `D2B_*_PATH` fixture env vars at fixtures
  or missing paths so the test never touches the operator's live daemon.
- Rendered-artifact ↔ DTO/doc contract → a contract test in the owning
  `packages/<crate>/tests/` directory.
- Generated docs/schemas/CLI freshness → already a drift gate; regenerate with
  `bazel run //packages/xtask:xtask -- gen-*`. Do **not** add a new shell gate.

Only reach for Layer 2 (containers / VMs / live-host) when a foreign
userland, a real systemd boot, or a live host is genuinely
required - and pick the lowest tier that works. Physical-device
validation is manual operator work, not a repository evidence script.

## Conventions

- **Commit before building.** `nix flake check` and the eval gates resolve the
  flake via `git+file://`, which only sees git-tracked files - an untracked new
  module/test is invisible until committed.
- **Retire tests directly.** Delete superseded coverage and all references.
  Preserve only current owner-local behavior checks or structural enforcement;
  do not add migration records, successor pins, or evidence scripts.
