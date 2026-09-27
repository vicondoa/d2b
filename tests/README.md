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
  containers, KVM-booted VMs through the Bazel host lane, and live-host
  scripts. Used only
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
    └── lib.nix                                                  type 10: guest-node helpers for the Bazel host lane (make test-host-integration; local pre-PR)
```

Rust tests (types 2-5: unit, integration, contract, policy-lint) live under
`packages/<crate>/`, **not** here.

## Running tests

The source-hygiene gate fails closed when `D2B_SHELLCHECK_BIN` is unavailable.

| Command | Runs | Where |
|---------|------|-------|
| `make check` | complete PR-equivalent Bazel Layer-1 suite graph | local + CI |
| `make test-unit` | complete Bazel Layer-1 development suite graph | local + CI |
| `make check-tier0` | fast Bazel toolchain and source-policy suite | local + CI |
| `make test-lint` | fixed Bazel source-hygiene and required shell-lint suite | local + CI |
| `make test-changelog` | require release notes for code changes and validate every changelog fragment | local + CI |
| `make test-rust` | composed Bazel Rust unit, integration, and doctest suites | local + CI |
| `make test-rust-<leaf>` | focused Bazel suites for main, broker, guest shell runner, policy, schema, and supply-chain coverage | CI (local for a focused rerun) |
| `make test-fixture-contracts` | enforcing eval-rendered lane: materializes `D2B_FIXTURES` from evaluated Nix artifact data, then runs owner-local CLI contract cases; invoking it without the enforcing lane fails rather than skipping | local + CI |
| `make test-proofs` | standalone proofs/ crates | local + CI |
| `make test-flake` | Bazel Nix evaluation suite | local + CI |
| `make test-nix-unit` | Bazel Nix-unit surface suite | local + CI |
| `make test-drift` | native generated-artifact and parity checks | local + CI |
| `make generate` | regenerate committed schemas, docs, bindings, completions, and policy inputs with local Bazel | local only |
| `make test-policy` | composed Bazel source, workspace/lock, supply-chain, and changelog policy suites | local + CI |
| `make test-performance-budgets` | advisory performance canary; without `D2B_PERF_STABLE=1` it reports `SKIP` and enforces nothing | local + CI |
| `make test-integration` | type-9 podman container tests | conditional local host lane (podman; not the PR pipeline) |
| `make test-host-integration` | type-10 host-integration VM checks through one Bazel lane target; guests are graph outputs keyed on declared inputs and every assertion is Rust | local contributor pre-PR lane (x86_64-linux; needs `/dev/kvm`; no emulation fallback; not the PR pipeline) |
| `make check-fast` | compatibility alias for `make check` | local + CI |
| `make bazel-check` | Bazel aggregate suite used by `make check`. Developer Bazel and public Make aliases default to BuildBuddy remote through `.bazelrc`; CI sets `D2B_BAZEL_PROFILE=local` | local or remote |
| `make pre-tag` / `make smoke-lite`, or a live script directly with its own opt-ins (`D2B_LIVE=1`, sudo) | type-11 live-host tests | **manual, against a deployed d2b host** |

`make check`, `make test-unit`, and `make bazel-check` invoke the same nested
suite graph through one public facade label. Public Make aliases run
`bazel test` directly, passing `--config=$(D2B_BAZEL_PROFILE)` only when that
variable is set; otherwise `.bazelrc` supplies the BuildBuddy `remote` default.
Bazel owns Layer-1 scheduling; Make and CI are thin aliases over one suite
label per public target. Cargo manifests and `Cargo.lock` remain rules_rs
metadata authority, while standalone crate Cargo commands are not documented
or required gate evidence.

`make generate` invokes the single `//packages/xtask:generate` aggregate with
`bazel run --config=local`. It writes the committed schemas, documentation,
completions, protocol bindings, Nix outputs, and policy inputs in the checkout;
it does not alter the repository-default remote profile used by `make check`.

`make test-host-integration` runs the Bazel-owned host integration lane as one
`bazel test` invocation. Each of the eleven checks boots its own NixOS guest,
built as a graph output keyed on declared inputs, and the lane restores a
pooled guest per check rather than booting one guest per check. Every
assertion is Rust; the guest images rebuild when a guest module or a d2b host
binary changes, and repeat runs execute no Rust compilation actions.

The lane is a local, contributor-run pre-PR surface, not a CI gate, and it
declares virtualization as a precondition: it needs `/dev/kvm` and has no
silent emulation fallback, so on a host without KVM it stops with a message
rather than turning very slow. It is x86_64-linux only.

`D2B_VM_CHECK=<name>` runs one named check, and `bazel test --test_filter=<name>`
works too - the lane reads Bazel's own filter as well as that variable, and the
first of the two that names anything wins. A failing check reports under its own
name in the lane's test output, with the stage it was in, the rows it was
asserting on, and the guest's journal and zone dump.

There is no Attic preflight or closure upload here: the guest-image action
declares its own substituters and preflights them itself, so the cache handling
lives with the build that needs it rather than in a second place that can drift.

Run these aliases directly from a normal Nix-enabled checkout. Make enters
the pinned `.#bazel` shell automatically when the explicit d2b shell contract
is absent, and enters it only once for a multi-goal or parallel invocation.
Inside `nix develop`, the complete interactive shell already supplies the
pinned Bazel toolchain. Use the focused shell for one-shot labels:

```bash
nix develop --no-write-lock-file .#bazel -c bazel test //packages/<crate>:<owner-test>
```

The focused shell supplies Bazel, Make, jq, Git, Rustup, and the shell
utilities used by `tests/tools/bazel-check`; no ambient host Bazel or jq is
required. An unrelated Nix shell is not accepted as the d2b shell. Optional
direnv integration is supported for interactive use but is not required.
CI installs Nix and calls the same public Make aliases with
`D2B_BAZEL_PROFILE=local`, without a `tests/tools/bazel-check` wrapper.

`make test-policy` does not schedule
`tests/tools/guest-workspace-drift.py`. The retained
`//tests/unit/meta:w0_dep_direction` target owns workspace-and-lock policy, but
it does not assert copied Guest workspace parity. Do not cite that parity as
passing gate evidence. When a mirrored shared crate gains or changes a
dependency, update the guest workspace fixture and any affected override,
refresh `packages/Cargo.guest.lock`, and run the applicable owner-local targets
plus `make test-rust-supply-chain` and `make test-policy`. The supply-chain lane
realizes the copied Guest workspace for dependency metadata, license, source,
and audit validation; it does not compile Guest packages and is not a fifth
repository-wide policy class or copied-workspace parity result.

All Layer-2 lanes (types 9-11) run their own work directly. `make
test-integration`, `make test-host-integration`, `make perf`, `make pre-tag`,
and `make smoke-lite` are plain invocations: there is no repository
semaphore, no re-exec wrapper, and no internal `heavy-lane-*` targets in
between. The heavy-gate semaphore (the `xtask heavy-gate` facade, its
`D2B_HEAVY_GATE` re-exec guard, the `/run/d2b-heavy-gates` slot namespace,
`make heavy-gate-provision`, and the `heavy-test-*` aliases) was removed, and
nothing replaces it, so nothing prevents two heavy lanes from running at once
on one host except the caller. `make heavy-check` and
`make heavy-flake-check` survive as plain aliases that run their work
directly, not under a gate.

Live-host tests are run through `make pre-tag` / `make smoke-lite`, or
directly with the opt-in variables they require (`D2B_LIVE=1`, sudo). Those
scripts retain their own safety checks and cleanup behavior.

Current live-host scripts include `d2b-store.sh` for per-VM store
adoption and `usbip-lifecycle.sh` for USBIP attach/detach across a `d2bd`
restart. The USBIP script requires
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

The Layer-1 graph is composed by nested suites in `BUILD.bazel` and
`bazel/checks/`. Bazel owns selection, dependency ordering, parallelism, retry
classification, caching, and aggregation. Every public Layer-1 `make test-*`
alias invokes one matching facade suite, and every fixed CI job runs the same
graph with the local profile.
Individual labels remain available for focused reruns.

`bazel/checks/BUILD.bazel` is the public suite facade. Its fixed package-suite
list owns the package-wide main graph; the broker and
local Rust suites remain separate components. `make test-rust-main` excludes
`local` and `no-remote-exec` leaves by tag, while `make test-rust-local`
executes the audited local suite.

Cargo manifests and the root `Cargo.lock` remain authoritative for Rust
membership, dependencies, and features consumed by rules_rs. Do not add a
second Cargo lock, source inventory, generator, discovery job, or shell
scheduler.

`tests/tools/bazel-check` retains the BuildBuddy security boundary. It uses
Bazel's credential helper, withholds credentials from untrusted work, redacts
logs and BEP output, and retries the identical target set locally only for a
typed pre-dispatch infrastructure failure. Post-dispatch and test failures
fail closed. Provider measurements do not define a second acceptance gate.
The facade consumes `D2B_BAZEL_BIN` from the pinned shell and rejects an
incomplete shell contract; it does not search for a hard-coded Nix-store
Bazel path.

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

`make test-nix-unit` runs one Bazel action per named owner surface.
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
remain conditional or manual lanes that run their own work directly; they are
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
