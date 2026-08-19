# Gates and lints

Reference for the heavy-lane semaphore and policy lints whose exemptions are
easy to get wrong. The binding summary and enforcing/advisory rule live under
[worktree, validation, and landing rules](../../AGENTS.md#worktree-validation-and-landing-rules);
read that first. This file covers the parts needing more than a rule.

`.github/workflows/pr-l1-static-fast.yml` and the `Makefile` are authoritative
for the fixed Layer-1 job set and its enforcement classification. This file
documents their current behavior.

## Non-ASCII dash scan exemption

The tier0 dash gate keeps its repository-wide fail-closed behavior while
allowing punctuation that is part of approved upstream agent assets. The
exemption is a closed path set, owned by
`tests/tools/tier0-first-pass.sh`, and is not a general vendor or adapter
directory exemption.

The exact instruction files are `AGENTS.md`, `tests/AGENTS.md`,
`labs/venus-vulkan-video/AGENTS.md`, and `CLAUDE.md`. Canonical skill payloads
are exempt only below these exact pinned roots, and only for the approved skill
directories:

- `third_party/agent-skills/ponytail/v4.9.0/skills`
- `third_party/agent-skills/caveman/v2.0.0/skills`
- `third_party/agent-skills/compound-engineering/compound-engineering-v3.21.4/skills`

The matching root `LICENSE` file under each of those three pinned version
directories is also exempt so its upstream bytes and legal notice stay exact.
No other notice file, source, version, or sibling path is admitted.

The admitted child directory names are `ponytail`, `ponytail-audit`,
`ponytail-debt`, `ponytail-gain`, `ponytail-help`, `ponytail-review`,
`caveman`, `ce-babysit-pr`, `ce-brainstorm`, `ce-code-review`,
`ce-commit-push-pr`, `ce-debug`, `ce-doc-review`, `ce-plan`,
`ce-resolve-pr-feedback`, `ce-simplify-code`, `ce-work`, and `ce-worktree`.

The `.agents/skills/<skill>` and `.claude/skills/<skill>` adapter entries are
admitted only when they are relative symlinks to the matching canonical skill.
The static Claude fallback additionally admits only relative symlinks for
components that resolve to the matching canonical component. Regular files in
adapter trees, lookalike names, other versions, and links to other targets
remain in the scanned set. Product documentation, plans, changelog entries,
configuration, and ordinary source files have no exemption.

Enumeration happens before filtering and must still be non-empty and
successful. The gate then removes only these validated paths before invoking
`grep`; if every enumerated path is exempt, it reports success without invoking
`grep`. Any non-exempt `grep` error remains a failure.

## Build and validate, in detail

Use top-level `Makefile` targets. Shell scripts under `tests/` are
implementation details unless a target or `tests/AGENTS.md` says to run one.

`nix develop` provides the pinned Rust release plus sccache, cargo-nextest,
cargo-deny, cargo-audit, shellcheck, and jq. Gate scripts re-enter a nix shell
and bootstrap a private toolchain when missing, so a dev shell skips that setup.
Normal dev/test
profiles retain panic line tables but omit dependency DWARF; use
`cargo build --profile debugging` or `cargo test --profile debugging` for full
debugger symbols.

The main workspace, privileged broker, and guest shell runner tests use Bazel
inside `make check`. Cargo remains an explicit compatibility and local-tool
path. That compatibility path needs companion runs for **doctests** (several
`compile_fail` cases are capability seals) and **`harness = false` binaries**
(`d2b-core-smoke` carries fail-closed minijail assertions). The harness-free
set comes from `nextest list`, not a pin. The broker Cargo compatibility
contexts stay serial because those tests are not process-per-test safe.

`make test-runtime-ledger` also stays on `cargo test`, and that is load
bearing. It enforces an aggregate process-CPU budget, and nextest's
one-process-per-test model costs about 1.9x the CPU for the same census
(measured: 1.2 s against 2.3 s). Porting it would mean roughly doubling the
budget and losing that much sensitivity, for no speedup.

When a failure reproduces only inside the gate toolchain, use
`tests/tools/repro-rust-gate-env.sh <command>` instead of re-running
`make test-rust`.

```bash
# Focused Layer-1 jobs over fixed Bazel labels.
make check-tier0
make test-lint
make check-inventory
make test-changelog
make test-rust
make test-proofs
make test-flake
make test-nix-unit
make test-policy
make test-drift
make test-runtime-ledger
make test-performance-budgets
make test-fixture-contracts

# Layer-1 development umbrella.
make test-unit

# PR-equivalent Layer-1 gate.
make check

# Legacy/full-static monolithic gate retained for explicit use.
make check-static

# Local Layer 1 + container integration. Run wider lanes only when the changed
# surface requires them.
make test
```

### Bazel and BuildBuddy execution

Bazel is the sole Layer-1 scheduler. The fixed graph under `BUILD.bazel` and
`bazel/checks/` owns target selection, dependency ordering, parallelism,
cache behavior, retry classification, and aggregation. Make and CI expose
compatibility aliases over those fixed labels; they must not add discovery,
sharding, fan-out, or rollup logic.

```bash
make check-tier0
make check-inventory
make test-lint
make test-rust
make test-proofs
make test-flake
make test-nix-unit
make test-policy
make test-drift
make test-runtime-ledger
make test-fixture-contracts
make test-unit
make check
```

Local aliases use the BuildBuddy `remote` profile when credentials and trust
permit it. CI sets `D2B_BAZEL_PROFILE=local` and `D2B_BAZEL_UNTRUSTED=1`.
The credential helper, trust partition, redaction, and typed one-retry
pre-dispatch fallback live in `tests/tools/bazel-check`; do not duplicate
those behaviors in Make or workflow code. Post-dispatch, analysis, policy,
build, and test failures fail closed.

The committed fixed workflow exposes one stable required `check` result. A
guarded performance skip is advisory and is not validation evidence.

The fixture-contract lane remains enforcing and local-only. It materializes
`D2B_FIXTURES` through the existing Bazel fixture target and fails when
`D2B_ENABLE_FIXTURE_BUILD=1` is absent. Nix actions remain local and remote
cache/execution disabled.

See [Bazel and BuildBuddy](../reference/bazel-buildbuddy.md) for profile,
credential, redaction, and focused-rerun details.

### Rust and Nix compatibility surfaces

Cargo remains a direct development surface over the root `Cargo.toml` and
`Cargo.lock`; nextest does not replace `cargo test --doc` or harness-free
companion commands. `rules_rs` supplies the Bazel Cargo integration, and the Bazel graph exposes
doctest, feature, harness-free, fixture, and policy coverage as explicit
targets. No second Cargo lock, source inventory, generator, or shell scheduler
is authoritative.

Nix-unit and flake checks use fixed Bazel targets with declared inputs. Their
existing case pins remain under `tests/unit/nix/pinned/`; regenerate only
with `make nix-unit-pin` after a case change. Runtime-ledger changes use
`make runtime-ledger-pin`. The graph has no secondary evidence or provider
qualification gate.

### Realized Nix checks and runtime budget

`//bazel/checks/nix:flake-eval-x86-realized` is the fixed local-only target
for checks that must build their derivations rather than only instantiate
metadata. Its declared inputs and RSS ceiling live in
`bazel/checks/nix/BUILD.bazel`; do not add a workflow matrix or an outer cache
scheduler around it.

When a change needs container or NixOS host coverage, run the corresponding
conditional target on the development host:

```bash
make test-integration
make test-host-integration
```

Hardware and live-host tests remain explicit manual tiers and require the
matching devices or deployed d2b state.

`make test-runtime-ledger` is the hermetic execution-budget Layer-1 job. It
uses the existing `tests/runtime-ledger-census.json` and
`make runtime-ledger-pin` when a governed test is added, removed, or renamed.
The aggregate process-CPU budget is enforcing; shorter per-test timing
thresholds remain advisory diagnostics. This gate holds no historical
regression baseline.

## Heavy lanes

Every Layer-2, host-integration, hardware, live, and perf-heavy command
runs through **one** semaphore, invoked from the repository root as `cargo
run --manifest-path Cargo.toml -p xtask -- heavy-gate`. It grants
two slots per uid via open file description locks so concurrent heavy lanes
cannot oversubscribe the shared Nix store, cargo target directory, or KVM
device. Do not add a second lock file, sleep-and-retry loop, or per-crate
guard.

The slot namespace is fixed at `/run/d2b-heavy-gates/uid-<uid>/`. The root
and per-uid directory are root-owned and non-writable by unprivileged users;
the two `slot-*` files are pre-created for the target uid at mode `0600`.
No runtime-directory or temporary-directory fallback. The NixOS
module provisions the root with systemd-tmpfiles, then activation provisions
directories and slots for configured lifecycle users that NSS can resolve.
An unavailable network-backed user is deferred rather than failing
activation; after that user logs in, run `make heavy-gate-provision`. Use
the same target on a host that does not consume the module. Because `/run`
is a tmpfs, run it once per boot when the gate requests it. An absent or
malformed namespace is an environment error with that provisioning
remediation, never permission to create a weaker pool. In particular,
`/run/user/<uid>` is rejected because its owner can rename slot names or
their parent and create an independent pool.

The structure is public-lane-plus-guarded-internal:

- **Public lane targets** (`make test-integration`,
  `make test-host-integration`, `make test-hardware`, `make perf`) acquire
  a slot and then delegate to a guarded internal `heavy-lane-*` target.
  Run these.
- **Internal `heavy-lane-*` targets** hold the raw work and fail closed
  through `heavy-lane-guard` if invoked outside the gate (the gate exports
  `D2B_HEAVY_GATE` across its re-exec). Do not run them directly.
- **Convenience wrappers** `make heavy-check`, `make heavy-cargo-test`,
  `make heavy-flake-check`, and the `heavy-test-*` aliases run a Layer-1
  gate, the Rust suite, the building flake check, or a public lane under
  the same semaphore.

Run a heavy lane through its public target (or, for an arbitrary command,
`cargo run --manifest-path Cargo.toml -p xtask -- heavy-gate --
<command>`) whenever another heavy lane might be running; do not invoke the
internal targets directly. Live-host and hardware
tests obey the same rule: use the gated live-VM smoke entrypoints (`make
pre-tag` for the full gate, `make smoke-lite` for the lite gate) or wrap a
raw live script as `cargo run --manifest-path Cargo.toml -p xtask
-- heavy-gate -- env D2B_LIVE=1 bash tests/integration/live/<name>.sh`.

The repository-root `Cargo.toml` is the product workspace and the root
`.cargo/config.toml` is its Cargo configuration. The bare `cargo xtask`
alias therefore resolves from the repository root; use the explicit
`--manifest-path Cargo.toml` spelling when a command's authority should be
visible in the invocation.

Invoking a live script directly is safe but not the documented path: each
one verifies the inherited slot and re-executes itself through the semaphore
exactly once when no genuine slot is held. A bare `D2B_HEAVY_GATE` value is
not trusted, so it cannot bypass the sole-use invariant.
**A new live, hardware, or performance entrypoint must carry that same
self-guard block**, or the fail-closed inventory guard
(`every_live_and_heavy_entrypoint_routes_through_the_gate`) rejects it.

For where tests live, when to add or retire each kind of test, and
which pins/ledgers to update, read [`tests/AGENTS.md`](../../tests/AGENTS.md).
[`tests/README.md`](../../tests/README.md) is the human quick-start for the
same test model.
