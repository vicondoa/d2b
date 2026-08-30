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

Run public gates as `make <target>` from a normal Nix-enabled host. The
Makefile is the environment dispatcher: it detects the explicit
`D2B_PROJECT_SHELL=d2b` and executable `D2B_BAZEL_BIN` contract, enters
`nix develop --no-write-lock-file .#bazel` once when needed, and preserves the
original goals, variables, profile, trust settings, and parallelism. It does
not trust an unrelated `IN_NIX_SHELL` value. A missing Nix installation fails
clearly; enter `nix develop` or install Nix before retrying.

`nix develop` is the complete interactive contributor shell with the pinned
Bazel and Rust toolchains. `nix develop --no-write-lock-file .#bazel` is the
focused shell used for Make re-entry and one-shot direct Bazel labels:

```bash
nix develop --no-write-lock-file .#bazel -c bazel test //packages/<crate>:<owner-test>
```

Optional direnv integration may enter the interactive shell automatically, but
is not required. Normal profiles retain panic line tables but omit dependency
DWARF; use the explicit Bazel debugging profile when a full debugger build is
required.

Bazel is the only supported contributor build and test interface. The public
suite facade in `bazel/checks/BUILD.bazel` composes package-level Rust suites
with the privileged broker, guest shell runner, doctests, and
`harness = false` binaries through owner-local Bazel targets.
Cargo manifests and lockfiles remain rules_rs metadata authority and are not
invoked by tests or gate helpers.

When a failure reproduces only inside the Bazel test environment, rerun the
owning Bazel label directly with the same profile and test environment rather
than adding a compatibility helper.

```bash
# Focused Layer-1 jobs over fixed Bazel labels.
make check-tier0
make test-lint
make test-changelog
make test-rust
make test-proofs
make test-flake
make test-nix-unit
make test-policy
make test-drift
make test-performance-budgets
make test-fixture-contracts

# Layer-1 development umbrella.
make test-unit

# PR-equivalent Layer-1 gate.
make check

# Conditional container integration. Run it only when the changed surface
# requires a foreign userland.
make test-integration
```

### Bazel and BuildBuddy execution

Bazel is the sole Layer-1 scheduler. The nested suite graph under
`BUILD.bazel` and `bazel/checks/` owns target selection, dependency ordering,
parallelism, cache behavior, retry classification, and aggregation. Make and
CI expose compatibility aliases over one public suite label per target; they
must not add discovery,
sharding, fan-out, or rollup logic.

The facade's package-level `all-tests` suites provide the fixed main package
authority. Broker and guest-shell-runner workspaces use dedicated component
suites, while local Rust leaves stay in the audited tag-driven local suite.

```bash
make check-tier0
make test-lint
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

Local aliases use the BuildBuddy `remote` profile when credentials and trust
permit it. CI invokes the same public Make aliases after installing Nix and
sets `D2B_BAZEL_PROFILE=local`; public Make aliases run `bazel test
--config=$(D2B_BAZEL_PROFILE)` directly with no `tests/tools/bazel-check`
wrapper. `tests/tools/bazel-check` remains the BuildBuddy credential helper
only. Post-dispatch, analysis, policy, build, and test failures fail closed.

The committed fixed workflow exposes one stable required `check` result. A
guarded performance skip is advisory and is not validation evidence.

For the final U20 acceptance lane, both public integration targets,
`make test-host-integration` and `make test-integration`, are mandatory and
may run alongside the `/etc/nixos` real-host switch/startup/Cloud Hypervisor
Guest boot sequence. U19 leaves their declarations and current inputs
converged but does not run host acceptance.

The fixture-contract lane remains enforcing and local-only. It materializes
`D2B_FIXTURES` through the existing Bazel fixture target and fails when
`D2B_ENABLE_FIXTURE_BUILD=1` is absent. Nix actions remain local and remote
cache/execution disabled.

See [Bazel and BuildBuddy](../reference/bazel-buildbuddy.md) for profile,
credential, redaction, and focused-rerun details.

### Rust and Nix compatibility surfaces

Cargo manifests and `Cargo.lock` remain metadata inputs over the root workspace;
they are consumed by `rules_rs`, not exposed as contributor gates. The Bazel
graph exposes doctest, feature, harness-free, fixture, and policy coverage as
explicit targets. No second Cargo lock, source inventory, generator, or shell
scheduler is authoritative.

Nix-unit and flake checks use Bazel targets with declared inputs. Each
named Nix surface declares its expression and exact module/helper/fixture
closure directly in `bazel/checks/nix/BUILD.bazel`; the graph has no corpus
discovery, case-presence pins, secondary evidence, test census, or provider
qualification gate. Surface actions copy that closure into an isolated source
root and evaluate the expression with the shared Bazel-provided nixpkgs pin,
not the repository flake outputs, per-test Git input fetching, or ambient
`D2B_REPO_ROOT`.

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

`make test-host-integration` builds the fixed set of nine host tools with
local Bazel, stages them as one bundle, and injects that bundle into the
selected NixOS `vmChecks`. After every selected check succeeds, the lane
uploads the built dependency closures to the configured Attic cache in one
operation. It excludes the `vmCheck` result paths so a capability `SKIP` or
`BLOCKED` result cannot be substituted as a passing test on another host.

Attic is optional for this lane. When the Attic client or its configuration is
unavailable, the lane reports an explicit skip and continues with the Bazel
and VM work. A present configuration that is invalid, ambiguous, inaccessible,
or otherwise unusable fails closed before the expensive work; an upload failure
also fails the lane. Use `D2B_VM_CHECK=<name>` to select one named VM check.

For cold and unchanged warm evidence, run the same command twice:

```bash
make test-host-integration
make test-host-integration
```

The unchanged repeat should reuse the Bazel and Nix outputs without Rust
compilation actions. The optional `d2b.site.hostSccache.enable` module remains
available for other Nix source builds; it is not required by this
Bazel-backed host-integration lane.

Hardware and live-host tests remain explicit manual tiers and require the
matching devices or deployed d2b state.

## Heavy lanes

Every Layer-2, host-integration, hardware, live, and perf-heavy command
runs through **one** semaphore, invoked from the repository root through the
Bazel-built `bazel-bin/packages/xtask/xtask heavy-gate` facade. It grants
two slots per uid via open file description locks so concurrent heavy lanes
cannot oversubscribe the shared Nix store, Bazel output tree, or KVM
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
  `make test-host-integration`, `make perf`) acquire
  a slot and then delegate to a guarded internal `heavy-lane-*` target.
  Run these.
- **Internal `heavy-lane-*` targets** hold the raw work and fail closed
  through `heavy-lane-guard` if invoked outside the gate (the gate exports
  `D2B_HEAVY_GATE` across its re-exec). Do not run them directly.
- **Convenience wrappers** `make heavy-check`, `make heavy-flake-check`, and
  the `heavy-test-*` aliases run a Layer-1
  gate, the Rust suite, the building flake check, or a public lane under
  the same semaphore.

Run a heavy lane through its public target (or, for an arbitrary command,
`make heavy-gate-build && bazel-bin/packages/xtask/xtask heavy-gate --
<command>`) whenever another heavy lane might be running; do not invoke the
internal targets directly. Live-host
tests obey the same rule: use the gated live-VM smoke entrypoints (`make
pre-tag` for the full gate, `make smoke-lite` for the lite gate) or wrap a
raw live script with the Bazel-built xtask artifact.

The repository-root `Cargo.toml` and `Cargo.lock` are rules_rs metadata
authority. The Bazel-built xtask label is the only supported gate entrypoint;
do not add a direct Cargo compatibility wrapper.

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
