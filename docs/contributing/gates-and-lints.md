# Gates and lints

Reference for the contributor validation lanes and policy lints whose
exemptions are easy to get wrong. The binding summary and enforcing/advisory rule live under
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
are exempt only below the canonical version directory that each committed
`.agents/skills/<skill>` link resolves into - one vendored tree per upstream
source (`third_party/agent-skills/compound-engineering`,
`third_party/agent-skills/caveman`, `third_party/agent-skills/ponytail`,
`third_party/agent-skills/rewrite-rs`; currently v3.28.2, v2.7.0, v4.9.0,
and v0.1.0-alpha.1) - and only for the skill directories
those links admit. The matching root `LICENSE` and `UPSTREAM.json` provenance
manifest under each of those version directories is also exempt so upstream
bytes, legal notice, and provenance stay exact. No other notice file, source,
version, or sibling path is admitted. The gate carries no hard-coded version
or child-name list: `make update-agent-skills` refreshes the vendored trees,
both adapter directories, and the exemption set together, because admission
derives from the committed links.

The `.agents/skills/<skill>` and `.claude/skills/<skill>` adapter entries are
committed relative symlinks into the canonical skill directories, and the
exemption admits the canonical tree each link points into. The links
themselves are never scanned as content: the scan reads regular files only, so
an entry that resolves to a directory is skipped. The gate does not validate
adapter targets - `make update-agent-skills` is what keeps both adapter
directories pointing at the vendored trees, and tier0 consumes the committed
targets as its admission set. Regular files placed in adapter trees, lookalike
names, and other versions remain in the scanned set. Product documentation,
plans, changelog entries, configuration, and ordinary source files have no
exemption.

Enumeration happens before filtering and must still be non-empty and
successful. The gate then removes only these validated paths before invoking
`grep`; if every enumerated path is exempt, it reports success without invoking
`grep`. Any non-exempt `grep` error remains a failure.

## Async-gate and the method-call lock hatch

`make check-async-gate` runs the xtask async-gate scanner over the broker, the
daemon, and every provider crate (Layer-1 policy). It flags denied blocking
calls inside async contexts, and since U6 (issue #590) it also flags the
conservative method-call lock shape: a `lock()`, `read()`, or `write()` method
call inside an `async fn` (or `async` block) that is not followed by `.await`.
The shape is deliberately conservative - the scanner does not resolve receiver
types, so any `.lock()`/`.read()`/`.write()` method call in an async context
matches, including `tokio::fs::OpenOptions::read(true)`-style builder flags
and `AsyncReadExt::read` calls awaited through a timeout. An awaited
`tokio::sync::Mutex::lock()` site passes via the `.await` exclusion, and a
site that must stay is exempted by the source-level marker
`// async-gate-allow: <reason>` on the call's own line, at or after the call.

The marker is not an allowlist: every marked site must be recorded in
`packages/xtask/data/async-gate-inventory.json` (the same file records the
marker format), and every inventory entry must correspond to a marked
method-call site in the scanned roots. A marker without an inventory entry
fails the gate, and an inventory entry without a marked site fails it too -
as does an entry whose file no longer exists in the tree (a deleted marked
file is stale in every scan mode; a file that exists but is outside the
current scan set is tolerated, so subset scans stay valid). The
qualified-path form (`std::sync::Mutex::lock(...)`) has no hatch and
always fails.

The inventory keys sites by `(file, line)`, so a line-shifting edit above a
marked call turns the gate red until the entry is re-recorded. Regenerate the
inventory from the run's marker sites instead of hand-editing line numbers:
`cargo xtask check-async-gate --write-inventory` from the repo root (the
default scan roots only - a subset scan would drop entries for unscanned
files). The regeneration output is byte-stable, so a no-op regeneration
produces no diff.

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
authority. The broker workspace uses a dedicated component
suite, while local Rust leaves stay in the audited tag-driven local suite.

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
make generate
make test-unit
make check
```

Use `make generate` before committing changes to generated schemas, docs,
completions, protocol bindings, Nix resource outputs, or policy inputs. The
target runs `//packages/xtask:generate` with the explicit local Bazel profile;
ordinary `make check` remains on the repository-default remote profile.

Bare developer Bazel commands and public Make aliases use the BuildBuddy
`remote` profile by default through `.bazelrc`; Make passes an explicit
`--config=$(D2B_BAZEL_PROFILE)` only when that variable is set. CI invokes the
same public Make aliases after installing Nix and sets
`D2B_BAZEL_PROFILE=local`. A contributor or agent never sets that variable and
never passes a profile flag of its own, so the Makefile passing through a value
you exported is your override rather than the repository's selection; see the
profile rule in [`AGENTS.md`](../../AGENTS.md). Public Make aliases run
`bazel test` directly with no `tests/tools/bazel-check` wrapper.
`tests/tools/bazel-check` remains the
BuildBuddy credential helper only. Post-dispatch, analysis, policy, build, and
test failures fail closed.

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

`make test-host-integration` runs the Bazel-owned host integration lane as one
`bazel test` invocation of `//bazel/checks/vm:host_integration_lane_run`. Each of
the eleven checks boots its own NixOS guest, built as a graph output keyed on
declared inputs - the flake and its lock, the guest module sources, and the d2b
host binaries all arrive as label inputs - and the lane restores a pooled guest
per check rather than booting one guest per check. Every assertion is Rust; the
fixtures' `runNixOSTest` `testScript` surface is gone. Nix realizes the guest
closure and does not rebuild the injected d2b binaries; the implementation is in
[`bazel/checks/vm/defs.bzl`](../../bazel/checks/vm/defs.bzl),
[`nix/test-support/guest-image.nix`](../../nix/test-support/guest-image.nix), and
[`nix/test-support/bazel-host-tools.nix`](../../nix/test-support/bazel-host-tools.nix).

The guest-image action declares its own substituters and preflights them itself,
so the Attic preflight and closure upload that the old nix recipe carried are
gone with it. Cache handling now lives with the build that needs it rather than
in a second place that can drift out of step.

The lane is a local, contributor-run pre-PR surface, not a CI gate, and it
declares virtualization as a precondition: it needs `/dev/kvm` and has no silent
emulation fallback, so on a host without KVM it stops with a message rather than
turning very slow. It is x86_64-linux only.

`D2B_VM_CHECK=<name>` runs one named check, and `bazel test --test_filter=<name>`
works too - the lane reads Bazel's own filter as well as that variable, and the
first of the two that names anything wins. Each check reports under its own name
in the lane's output with the stage it was in, the rows it asserted on, and the
guest's journal and zone dump.

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

## Dead-code lane

`make check-dead-code` runs the workspace dead-code gate through the
`packages/xtask` `deadcode-check` entrypoint. It is not part of `make check`
or the fixed Layer-1 job set; run it as a local gate when restructuring
visibility, deleting code, or editing dependency lists.

The scan is three independent passes over the repository-root workspace:

- `cargo hawk check` - dead and overbroad public API (`pub` that can become
  `pub(crate)`) across the production targets.
- `cargo shear` - unused `Cargo.toml` dependencies (edition-2024 aware).
- `cargo check --workspace --all-targets` with
  `RUSTFLAGS="-A unused -D dead_code"` - the rustc `dead_code` lint isolated
  as a hard error.

Both external scanners are required, not optional: a missing binary fails the
gate with an install hint, and that fail-closed behavior is deliberate.

The lane is classified as a local Make goal, so `make check-dead-code` runs
inside the d2b development shell that the dispatcher re-enters for classified
goals (`nix develop .#bazel`), where `cargo-shear` is provisioned as a pinned
shell package rather than fetched per invocation.
`cargo-hawk` remains unprovisioned by the shell: it is built against rustc
internals (`rustc_private`) and only runs with the exact toolchain it was
built against, which the pinned stable toolchain cannot satisfy - install it
against a matching nightly to run that pass.

**Baseline.** The lane is red on an untouched tree and is meant to be read as a
delta, not a boolean: `cargo-shear` reports 81 pre-existing findings repo-wide
(identical on the untouched `v3` baseline) and the `cargo-hawk` pass does not
run at all. Re-run the lane before and after a change and compare the finding
set; a new finding is a regression, the standing corpus is not. The rustc
`dead_code` pass is the one that is expected to be clean.

**Cargo lanes and the test-support feature.** Three integration test binaries -
`d2b-provider-wayland-policy`'s `tests/engine.rs` and `tests/registration.rs`,
and `d2b-provider-guest`'s `tests/registration.rs` - declare
`required-features = ["test-support"]`, so a plain `cargo test -p <crate>` skips
them silently; run those crates with `--features test-support` (or
`--all-features`) when working through cargo instead of the Bazel layer.

## Layer-2 and manual lanes

Layer-2 container, VM, live-host, hardware, and performance surfaces remain
conditional or manual and are not part of the Bazel Layer-1 scheduler. Run the
public interfaces directly:

- `make test-integration` runs the container rollup
  `bash tests/test-integration.sh`.
- `make test-host-integration` runs the type-10 host-integration lane as one
  Bazel test invocation of `//bazel/checks/vm:host_integration_lane_run`,
  described above under "Build and validate, in detail". It is a local pre-PR
  x86_64-linux lane and declares `/dev/kvm` as a precondition with no
  emulation fallback.
- `make perf` invokes the advisory Bazel facade suite
  `//bazel/checks:test-performance-budgets`.
- `make pre-tag` and `make smoke-lite` run the full and lite live-VM smoke
  scripts.
- For an individual live-host script, set its required opt-in variables such
  as `D2B_LIVE=1` and invoke the script explicitly. These scripts retain their
  own safety checks and cleanup behavior.

The heavy-gate semaphore that used to serialize these lanes - with its
`D2B_HEAVY_GATE` re-exec guard, `/run/d2b-heavy-gates` slot namespace, and
`make heavy-gate-provision` step - was removed, and nothing replaces it:
concurrent heavy lanes on one host are the caller's responsibility.

The repository-root `Cargo.toml` and `Cargo.lock` remain rules_rs metadata
authority. Bazel remains the sole Layer-1 scheduler; do not add local
fan-out, scheduling, or wrapper machinery to Layer-2/manual lanes.

For where tests live, when to add or retire each kind of test, and
which pins/ledgers to update, read [`tests/AGENTS.md`](../../tests/AGENTS.md).
[`tests/README.md`](../../tests/README.md) is the human quick-start for the
same test model.
