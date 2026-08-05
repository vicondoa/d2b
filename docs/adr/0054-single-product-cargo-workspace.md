# ADR 0054: Single product Cargo workspace

- Status: Proposed
- Date: 2026-08-05
- Amends: [ADR 0052](0052-bazel-rust-build-and-test.md), replacing its
  product-workspace, dependency-hub, and lock inventory if this ADR is
  accepted.
- Scope: Cargo workspace membership and locks for product packages, Cargo and
  Nix package selection, Bazel dependency hubs and configured first-party
  targets, and package-scoped supply-chain enforcement.
- Non-scope: implementing the merge, changing Rust behavior, amending Spec 003
  in this PR, moving the no-bash walker, or weakening static and ELF checks.

## Context

At v3 commit `9bd6e2ac`, `packages/Cargo.toml` excludes
`d2b-priv-broker` and `d2b-guest-shell-runner`. Each excluded package is a
nested workspace with its own authoritative lock. ADR 0052 consequently
planned separate `main`, `broker`, `guest`, and `walker` `crate_universe`
hubs.

The workspace split was partly intended to prevent an accidental dependency
from privileged broker code or static guest code into unrelated product code.
It also kept binary closure and size review local to each package. A shared
lock resolves the union of workspace dependencies and therefore no longer
provides that visual or update boundary.

This decision accepts that tradeoff. A lock entry is not a build dependency.
The security boundaries are package-selected Cargo and Nix builds, explicit
native Bazel target dependencies, and enforcing policy over each selected
production closure. An unrelated package appearing only in the shared lock is
not a security defect. A new edge that connects that package to the broker or
guest selected closure is.

The no-bash AST walker is different. It is closed gate plumbing under
`tests/tools/`, outside the product package tree, and has no path dependency
into `packages/`. It remains a separate workspace and dependency hub.

## Evidence

### Full Cargo and Nix integration spike

The completed spike at commit `98ba0f9f` added the broker and guest runner to
the root workspace, removed both nested locks and workspace tables, and
regenerated the root lock offline. The standalone baseline had 54 members and
544 lock packages. The integrated workspace had 56 members and 608 lock
packages. Locked, offline `cargo tree` selection from the root showed no guest
runner in the broker closure and no broker in the guest closure.

Standalone and unified `cargo test -- --list` censuses were identical:

| Context | Root selector after merge | Cases |
| --- | --- | ---: |
| Broker default | `-p d2b-priv-broker --no-default-features` | 557 |
| Broker layer 1 | `-p d2b-priv-broker --no-default-features --features layer1-bootstrap` | 492 |
| Broker fake | `-p d2b-priv-broker --no-default-features --features fake-backends` | 559 |
| Guest production | `-p d2b-guest-shell-runner --no-default-features --features real-libshpool` | 11 |

The final dedicated broker gate passed all three serial, isolated Cargo test
streams. The guest gate passed 11 of 11. The generic main test lane passed
5,114 of 5,114 with 6 skipped after excluding the two package-specific crates
and `d2b-contract-tests`.

Inspection of the current `tests/test-rust.sh` baseline found exactly one
generic test exclusion, `--exclude d2b-contract-tests`. Its generic clippy
package-selection arguments are:

```text
/usr/bin/env -u BASH_ENV -u ENV ./tests/tools/scrub-shell-environment -c 'exec nix develop --command env -C packages cargo clippy --locked --workspace --all-targets -- -D warnings'
```

There is no clippy exclusion; because `d2b-contract-tests` is a root member,
clippy compiles it. The spike also excluded that crate from clippy, so that
spike result is not adopted. The workspace merge preserves current clippy,
enforcing `make test-policy` compilation, and fixture coverage. The unified
`D2B_ENABLE_FIXTURE_BUILD=1 make test-fixture-contracts` run passed exactly
359 of 359 cases in 105 seconds. That lane proves contract and CLI tests; it
does not replace clippy or the hermetic policy binaries.

Release artifacts from isolated target directories were:

| Binary | Standalone raw | Unified raw | Standalone stripped | Unified stripped |
| --- | ---: | ---: | ---: | ---: |
| Broker | 10,262,496 | 10,240,368 | 7,420,096 | 7,393,864 |
| Guest runner | 5,869,600 | 5,869,744 | 4,107,416 | 4,110,152 |

The actual Nix broker derivation and actual Nix static guest derivation both
built. The Nix broker was 8,635,480 bytes and remained the expected dynamically
linked host binary. The Nix guest was 5,313,928 bytes, static PIE, had no ELF
interpreter, and had no `NEEDED` entries. The existing guest selected-dependency
check, static ELF check, root deny and audit checks, flake no-build evaluation,
and `make test-policy` passed.

The Nix fixes were mechanical consequences of the workspace root: use the
root source and lock, select the package and features, and run the static
dependency query from the root. No static or ELF acceptance criterion changed.

### Package-closure policy spike

A disposable generator started from locked, offline root metadata, selected
normal and build edges for one production context, and emitted cargo-deny
metadata plus a dependency-pruned audit lock.

- Broker production contained 108 packages; its dev-inclusive deny metadata
  contained 153. The existing broker deny configuration and its audit with no
  ignore passed.
- Guest `real-libshpool` production contained 171 packages; its dev-inclusive
  deny metadata contained 181. Its audit passed with exactly
  `--ignore RUSTSEC-2024-0384`.
- The guest's current deny configuration did not pass. It reported six
  pre-existing license findings: BSD-3-Clause for `bindgen` and `instant`, ISC
  for `inotify`, `inotify-sys`, and `libloading`, and CC0-1.0 for `notify`.
  The same findings occur against the standalone guest lock.

A disconnected GPL canary left the broker verdict at exit 0. Connecting the
same canary to the broker closure made cargo-deny reject it at exit 4. The full
root graph also contains the main-only `wl-proxy` git source, which the broker
source policy rejects if the whole union is presented as the broker closure;
the actual broker closure excludes it and passes. These probes establish the
required property: unrelated main-only dependencies do not affect a
package-closure verdict, while a newly connected dependency does.

The six guest license findings are an implementation blocker, not evidence
against the workspace shape. The workspace-merge change must resolve them by a
reviewed update to the guest policy. It must not hide them by dropping the
selected dependency, weakening enforcement, or claiming that all policy
checks already pass.

### Unified Bazel spike

The Spec 003 W0 spike used Bazel 8.6.0, `rules_rust` 0.73.0,
`cargo-bazel` 0.18.0, and Cargo/rustc 1.97.0. Offline root lock generation
reported 553 packages on that branch. The full integration spike's 608 count
comes from a later branch with different product dependency contents, so the
two counts are branch observations rather than a resolution mismatch.

`MODULE.bazel` declared exactly `product` and `walker` hubs. The product hub
used `packages/Cargo.toml` and `packages/Cargo.lock`; the walker remained
independent. The pinned root command
`/usr/bin/env -u BASH_ENV -u ENV ./tests/tools/scrub-shell-environment -c 'exec nix develop --command env -C packages cargo xtask gen-bazel --check'`
and module-lock error mode passed. A full
`bazel query //... --output=label` returned 321 labels.

The following native targets and representative tests passed:

```text
bazel build //packages/d2b-priv-broker:d2b-priv-broker
bazel build //packages/d2b-guest-shell-runner:d2b-guest-shell-runner-real-libshpool
bazel test //packages/d2b-priv-broker:bridge_lifecycle_default
bazel test //packages/d2b-priv-broker:bridge_lifecycle_layer1_bootstrap
bazel test //packages/d2b-priv-broker:bridge_lifecycle_fake_backends
bazel test //packages/d2b-guest-shell-runner:tests
```

The final guest test label was named `cli` in the follow-up probe and also
passed. The spike does not claim the full broker suite: one test still needs
declared fixtures and another invokes `sh`, which the hermetic sandbox omits.

`cquery --output=build` showed empty, `layer1-bootstrap`, and `fake-backends`
feature contexts on the three broker libraries. `aquery` showed the matching
`--cfg feature="layer1-bootstrap"` and `--cfg feature="fake-backends"` rustc
arguments. The measured graph counts were:

| Measurement | Count |
| --- | ---: |
| All first-party labels | 321 |
| Broker context targets | 46 |
| Broker production dependency labels | 7,721 |
| Guest real-libshpool dependency labels | 10,332 |
| Product lock crate records | 596 |
| Product external labels | 297 |
| Walker external labels | 9 |
| Cargo identities: broker default/layer1/fake | 95 / 95 / 95 |
| Cargo identities: guest real-libshpool | 135 |
| Broker dependencies on guest runner | 0 |
| Broker unexpected first-party sibling packages | 0 |

Every selected Cargo third-party identity for all four contexts was present in
the product lock; every missing-identity count was zero. The product external
repository is a union by design: the 596-record hub is not expected to equal
any selected 95- or 135-identity Cargo context.

The initial spike needed `crate.spec` only because `libshpool` was optional in
the guest manifest. A follow-up changed it to a normal dependency while
leaving `real-libshpool` as the code feature, removed `crate.spec`, and passed
offline root lock generation, locked checks, `gen-bazel`, production broker
and guest builds, all four representative tests, and context queries. This
standard pinned command then passed:

```text
/usr/bin/env -u BASH_ENV -u ENV ./tests/tools/scrub-shell-environment -c 'exec nix develop --command env -C packages cargo xtask bazel-repin --hub product'
```

It generated only `bazel/cargo/product.lock`. Production already always
enables `real-libshpool`; default code remains feature-gated. Compiling the
normal dependency in a non-production guest context is an accepted cost of
manifest-driven hub resolution.

## Decision

### Root command hardening

Every root-runnable Cargo command starts with absolute
`/usr/bin/env -u BASH_ENV -u ENV`, then enters existing
`tests/tools/scrub-shell-environment` before any shell. This Make `SHELL`
removes all `BASH_FUNC_*` entries before `/bin/sh`, which execs `nix develop`;
`env -C packages` enters packages without another shell. No function shadows.

Policy generation/check and hub-repin probes seed hostile `BASH_ENV` and `BASH_FUNC_cargo%%`,
require absent sentinels, prove pinned Cargo/`xtask` executes, and refuse failed removal.

### 1. Use one authoritative product workspace and lock

Add `d2b-priv-broker` and `d2b-guest-shell-runner` to
`packages/Cargo.toml` members and remove them from `exclude`. Remove each
package's nested `[workspace]`, workspace-local `[profile.*]` tables, and
`Cargo.lock`. Generate and verify the sole authoritative product lock with:

```text
/usr/bin/env -u BASH_ENV -u ENV ./tests/tools/scrub-shell-environment -c 'exec nix develop --command env -C packages cargo generate-lockfile --offline'
/usr/bin/env -u BASH_ENV -u ENV ./tests/tools/scrub-shell-environment -c 'exec nix develop --command env -C packages cargo metadata --locked --offline --format-version 1'
```

The packages keep `default = []` and their explicit dependencies. The guest
manifest has this shape:

```toml
[features]
default = []
real-libshpool = []

[dependencies]
libshpool = "0.11.0"
```

The existing `real-libshpool` code gates remain.

### 2. Select every Cargo and Nix context explicitly

The root-runnable package selectors are:

```text
/usr/bin/env -u BASH_ENV -u ENV ./tests/tools/scrub-shell-environment -c 'exec nix develop --command env -C packages cargo test --locked -p d2b-priv-broker --no-default-features -- --test-threads 1'
/usr/bin/env -u BASH_ENV -u ENV ./tests/tools/scrub-shell-environment -c 'exec nix develop --command env -C packages cargo test --locked -p d2b-priv-broker --no-default-features --features layer1-bootstrap -- --test-threads 1'
/usr/bin/env -u BASH_ENV -u ENV ./tests/tools/scrub-shell-environment -c 'exec nix develop --command env -C packages cargo test --locked -p d2b-priv-broker --no-default-features --features fake-backends -- --test-threads 1'
/usr/bin/env -u BASH_ENV -u ENV ./tests/tools/scrub-shell-environment -c 'exec nix develop --command env -C packages cargo fmt -p d2b-guest-shell-runner --check'
/usr/bin/env -u BASH_ENV -u ENV ./tests/tools/scrub-shell-environment -c 'exec nix develop --command env -C packages cargo clippy --locked -p d2b-guest-shell-runner --no-default-features --features real-libshpool --all-targets -- -D warnings'
/usr/bin/env -u BASH_ENV -u ENV ./tests/tools/scrub-shell-environment -c 'exec nix develop --command env -C packages cargo nextest run --locked -p d2b-guest-shell-runner --no-default-features --features real-libshpool'
```

Broker lanes remain three serial `cargo test` processes in isolated target
directories because they mutate process-global signal and reap state. Guest
doctest and harness-free companions reuse the same root manifest, package,
default-feature, and `real-libshpool` selectors.

The exact generic-main split is:

```text
/usr/bin/env -u BASH_ENV -u ENV ./tests/tools/scrub-shell-environment -c 'exec nix develop --command env -C packages cargo clippy --locked --workspace --all-targets --exclude d2b-priv-broker --exclude d2b-guest-shell-runner -- -D warnings'
/usr/bin/env -u BASH_ENV -u ENV ./tests/tools/scrub-shell-environment -c 'exec nix develop --command env -C packages cargo nextest run --locked --workspace --exclude d2b-contract-tests --exclude d2b-priv-broker --exclude d2b-guest-shell-runner'
```

The generic doctest and harness-free companion discovery uses the nextest
exclusion list. `d2b-contract-tests` therefore keeps its pre-existing test
exclusion and enforcing fixture lane, remains in generic clippy, and keeps its
selected hermetic policy binaries under enforcing `make test-policy`. A later
dedicated contract-crate clippy lane is an optional improvement, not a
prerequisite for this merge. It becomes a prerequisite only if a future change
proposes excluding the crate from main clippy. Global formatting may remain
global.

Delete `packages/d2b-priv-broker/Cargo.lock` and
`packages/d2b-guest-shell-runner/Cargo.lock`; do not leave forwarding locks.
Ordinary root and release builds now write and copy binaries under
`packages/target/{debug,release}`. The gate-owned isolated outputs remain
`packages/d2b-priv-broker/target`,
`packages/d2b-priv-broker/target-layer1`,
`packages/d2b-priv-broker/target-fakebackends`, and
`packages/d2b-guest-shell-runner/target` through explicit
`CARGO_TARGET_DIR`. CI cache workspace declarations collapse to
`packages -> target`; the four explicit gate directories remain cache
directories, not workspace roots. Their pre-merge contents are untracked
build caches; deleting them is optional local cleanup, not a migration step.

The broker Nix derivation keeps root `src = packagesSrc`, removes
`sourceRoot = "source/d2b-priv-broker"`, and selects root
`lockFile = ../packages/Cargo.lock`. The static guest changes `src` from
`./packages/d2b-guest-shell-runner` to root `rustPackagesSrc`, sets
`sourceRoot = "d2b-rust-src/packages"`, and selects root
`lockFile = ./packages/Cargo.lock`. Both retain the pinned git output hash and
use these exact package, binary, and feature selectors:

```text
--package d2b-priv-broker --bin d2b-priv-broker --no-default-features
--package d2b-guest-shell-runner --bin d2b-guest-shell-runner \
  --no-default-features --features real-libshpool
```

The existing broker build, guest static selected-dependency policy, static PIE,
ELF interpreter, `NEEDED`, deny, audit, and release checks remain enforcing.

### 3. Use one product Bazel hub and one walker hub

`crate_universe` has exactly `product` from the root product manifest and lock,
and `walker` from the no-bash walker's manifest and lock.
`packages/Cargo.guest.lock` remains a generated static-guest and cache input,
not a Cargo authority or hub.

The authoritative hub input and output paths after migration are:

| Hub | Manifest | Cargo lock | Bazel-side lock |
| --- | --- | --- | --- |
| product | `packages/Cargo.toml` | `packages/Cargo.lock` | `bazel/cargo/product.lock` |
| walker | `tests/tools/no-bash-ast-walker/Cargo.toml` | `tests/tools/no-bash-ast-walker/Cargo.lock` | `bazel/cargo/walker.lock` |

The workspace merge regenerates only the product Bazel-side lock. The walker
lock stays byte-identical. These are the only root-runnable repin commands:

```text
/usr/bin/env -u BASH_ENV -u ENV ./tests/tools/scrub-shell-environment -c 'exec nix develop --command env -C packages cargo xtask bazel-repin --hub product'
/usr/bin/env -u BASH_ENV -u ENV ./tests/tools/scrub-shell-environment -c 'exec nix develop --command env -C packages cargo xtask bazel-repin --hub walker'
```

`main`, `broker`, and `guest` are retired, not aliases. They fail before Bazel
starts, while `product` and `walker` remain accepted. Tests bind this mapping
and these exact diagnostics:

| Refused hub | Exact diagnostic |
| --- | --- |
| `main` | `Hub 'main' is retired; run /usr/bin/env -u BASH_ENV -u ENV ./tests/tools/scrub-shell-environment -c 'exec nix develop --command env -C packages cargo xtask bazel-repin --hub product'.` |
| `broker` | `Hub 'broker' is retired; run /usr/bin/env -u BASH_ENV -u ENV ./tests/tools/scrub-shell-environment -c 'exec nix develop --command env -C packages cargo xtask bazel-repin --hub product'.` |
| `guest` | `Hub 'guest' is retired; run /usr/bin/env -u BASH_ENV -u ENV ./tests/tools/scrub-shell-environment -c 'exec nix develop --command env -C packages cargo xtask bazel-repin --hub product'.` |

Changing the walker lock remains a separately reviewed change. Entering
`packages/` is load-bearing: rustup discovers `rust-toolchain.toml` there and
Cargo discovers `.cargo/config.toml` and its `xtask` alias there. No command in
this decision relies on that alias from the repository root.

Product code is represented by native first-party targets. Each broker and
guest configured context declares its own direct first-party and `@product`
dependencies and feature flags. The repository owner accepts `@product` as an
external package and feature superset. Exact third-party feature parity with
each Cargo context is not an invariant. The selected Cargo closure and its
package policy are the security authority; native configured targets remain
authoritative for actual first-party Bazel edges and features.

Every Bazel selected-root or containment checker for broker default, broker
layer1, broker fake, guest real-libshpool, product hub, and walker hub runs
these steps in order:

1. Assert the selected root exists exactly once.
2. Materialize the complete context closure and assert it is nonempty.
3. Assert its exact generated census before evaluating any predicate. The
   census includes the root, configured first-party targets, expected direct
   first-party dependencies, cfg and feature values, and all reached external
   identities.
4. Then prove matching hub and lock containment, expected configured
   first-party dependencies, cfgs and features, zero cross-context edges, and
   zero unrelated first-party siblings through `cquery` and `aquery`.

Package-closure checkers apply the same root, nonempty-census, completeness,
containment, minimality, deny, audit, and leakage order. An empty census cannot
satisfy an absence predicate. The `@product` external union is accepted; a
wrong native edge or an inexact selected Cargo closure is not.

### 4. Enforce package-scoped selected-closure policy

The repository-owned generator has these exact root-runnable entry points:

```text
/usr/bin/env -u BASH_ENV -u ENV ./tests/tools/scrub-shell-environment -c 'exec nix develop --command env -C packages cargo xtask gen-package-policy-inputs'
/usr/bin/env -u BASH_ENV -u ENV ./tests/tools/scrub-shell-environment -c 'exec nix develop --command env -C packages cargo xtask gen-package-policy-inputs --check'
```

The target set is exactly the root flake's `systems`, with distinct host and
static guest targets:

| Nix system | Broker package target | Guest static `pkgsStatic` target |
| --- | --- | --- |
| `x86_64-linux` | `x86_64-unknown-linux-gnu` | `x86_64-unknown-linux-musl` |
| `aarch64-linux` | `aarch64-unknown-linux-gnu` | `aarch64-unknown-linux-musl` |

ADR 0008 supports broker host runtime only on x86_64 Linux. Aarch64 artifacts
cover flake evaluation/build, not broker runtime or graphics/audio support.

For each system, the generator deterministically derives tracked inputs for
broker production with default features disabled and guest production with
`real-libshpool`:

1. `production/closure.json` plus `production/Cargo.lock`: the selected normal
   and build closure, resolved features, and pruned lock used for binary and
   static-dependency minimality.
2. `policy/metadata.json` plus `policy/Cargo.lock`: the production graph plus
   every root dev edge and the complete transitive normal/build closure of each
   reached dev package, used for package deny and audit.

The resulting paths are:

```text
packages/policy-inputs/<system>/<gnu-target>/broker-production/{production,policy}/
packages/policy-inputs/<system>/<musl-target>/guest-real-libshpool/{production,policy}/
```

Both sets derive from locked, offline root metadata and the root lock and bind
the selected root, Nix system, Cargo target, package identity, version, source,
checksum, edge kind, cfg, and resolved features. Production includes all
target-specific normal and build dependencies for that system. Policy adds the
root dev closure described above. Every drift diagnostic lists all stale paths
repository-relative and ends with this remediation, in this order:

```text
/usr/bin/env -u BASH_ENV -u ENV ./tests/tools/scrub-shell-environment -c 'exec nix develop --command env -C packages cargo xtask gen-package-policy-inputs'
Review and commit the generated changes under packages/policy-inputs/.
/usr/bin/env -u BASH_ENV -u ENV ./tests/tools/scrub-shell-environment -c 'exec nix develop --command env -C packages cargo xtask gen-package-policy-inputs --check'
```

For each policy set, the implementation reuses ADR 0052's pinned offline source
materialization rather than defining a second vendor path. Nix uses the
root-lock-derived filtered lock through `rustPlatform.importCargoLock`; the
Bazel carrier re-declares the same registry URL and lock checksum or the pinned
git rev and archive checksum. Before cargo-deny starts, the gate proves the
exact nonempty source set, count, readability, source identities, checksums,
and equality between metadata and filtered-lock identities.

The existing package configs run over the dev-inclusive metadata with no
`--exclude-dev`; they check bans, licenses, and sources. Package audits use the
generated policy locks and pinned RustSec database with `--no-fetch`. Broker
has no ignore; guest has exactly `--ignore RUSTSEC-2024-0384`. Existing
aggregate checks remain unchanged, including the root audit's two ignores and
the no-ignore `Cargo.guest.lock` audit. Aggregate and package checks may each
block the change.

The package checks under `checks.<system>` are
`broker-production-dependency-policy`,
`guest-shell-runner-static-dependency-policy`,
`broker-production-package-policy`, and `guest-real-libshpool-package-policy`.
All four exist for both systems; none falls back to x86_64. Each reads only its
exact system-and-target path. Before root or graph work it checks embedded
system, exact GNU or musl target, then every edge's authoritative kind. Each
mismatch emits its own early diagnostic, never later policy or leakage output.

### Seeded refusal matrix

Each row is a separate fixture mutation over a passing baseline and must fail
with that predicate's diagnostic. Reuse, a later predicate, or exit zero is a
harness failure. Instantiate the matrix for every applicable checker.

| Case | Isolated mutation |
| --- | --- |
| `missing-root` | Remove the selected root. |
| `duplicate-root` | Emit the selected root twice. |
| `empty-closure` | Retain the root declaration but emit no closure. |
| `wrong-system` | Change only the embedded Nix system. |
| `wrong-target` | Change only the embedded GNU or musl target. |
| `wrong-edge-kind` | Change one edge to another valid kind; retain all other fields. |
| `omitted-normal-edge` | Remove one reached normal edge. |
| `omitted-build-edge` | Remove one reached build edge. |
| `omitted-root-dev-edge` | Remove one root dev edge from policy metadata. |
| `omitted-dev-normal-edge` | Remove a normal edge transitively reached from a root dev edge. |
| `omitted-dev-build-edge` | Remove a build edge transitively reached from a root dev edge. |
| `wrong-cfg` | Change one configured target cfg value. |
| `wrong-feature` | Change one resolved feature set. |
| `cross-context-edge` | Add an edge valid only in another configured context. |
| `unrelated-first-party-sibling` | Connect one unrelated product sibling. |
| `product-hub-containment` | Add an external identity absent from the product hub. |
| `walker-hub-containment` | Add an external identity absent from the walker hub. |
| `product-lock-containment` | Add a product-hub identity absent from the product lock. |
| `walker-lock-containment` | Add a walker-hub identity absent from the walker lock. |
| `broker-x86_64-target-edge` | Omit a synthetic x86_64-only broker dependency. |
| `guest-x86_64-target-edge` | Omit a synthetic x86_64-only guest dependency. |
| `broker-aarch64-target-edge` | Omit a synthetic aarch64-only broker dependency. |
| `guest-aarch64-target-edge` | Omit a synthetic aarch64-only guest dependency. |
| `stale-bazel-output` | Change generated Bazel output; the pinned Bazel generation check refuses. |
| `source-missing` | Remove one selected source in a clean source-cache environment. |
| `source-extra` | Materialize one source absent from metadata. |
| `source-unreadable` | Make one selected source unreadable. |
| `checksum-mismatch` | Change selected source bytes without changing its checksum. |
| `source-identity-mismatch` | Change a registry URL or git rev on one side only. |
| `metadata-lock-mismatch` | Make metadata and the filtered lock name/version/source sets differ. |
| `forbidden-production-class` | Connect a forbidden static dependency class. |
| `forbidden-license` | Connect a dev-only package denied by license policy. |
| `forbidden-source` | Connect a dev-only package denied by source policy. |
| `forbidden-ban` | Connect a dev-only package denied by bans policy. |
| `advisory` | Connect a dev-only package with a non-ignored pinned advisory. |
| `stale-policy-output` | Change a tracked policy artifact; the pinned policy generation check refuses. |

The guest real-libshpool policy currently has six pre-existing license
denials: BSD-3-Clause for `bindgen` and `instant`, ISC for `inotify`,
`inotify-sys`, and `libloading`, and CC0-1.0 for `notify`. The workspace merge
remains blocked unless the same change narrowly updates
`packages/d2b-guest-shell-runner/deny.toml` for precisely those six selected
package/license pairs. A blanket license expansion is not the remedy. The
exact root-runnable remedy and recheck sequence is:

```text
/usr/bin/env -u BASH_ENV -u ENV ./tests/tools/scrub-shell-environment -c 'exec nix develop --command env -C packages cargo xtask gen-package-policy-inputs'
Review and commit packages/d2b-guest-shell-runner/deny.toml and the generated packages/policy-inputs/ changes.
/usr/bin/env -u BASH_ENV -u ENV ./tests/tools/scrub-shell-environment -c 'exec nix develop --command env -C packages cargo xtask gen-package-policy-inputs --check'
make flake-matrix-pin
make test-drift
nix build --no-link \
  .#checks.x86_64-linux.broker-production-dependency-policy \
  .#checks.aarch64-linux.broker-production-dependency-policy \
  .#checks.x86_64-linux.guest-shell-runner-static-dependency-policy \
  .#checks.aarch64-linux.guest-shell-runner-static-dependency-policy \
  .#checks.x86_64-linux.broker-production-package-policy \
  .#checks.aarch64-linux.broker-production-package-policy \
  .#checks.x86_64-linux.guest-real-libshpool-package-policy \
  .#checks.aarch64-linux.guest-real-libshpool-package-policy
make test-rust-supply-chain
make test-policy
nix build --no-link .#checks.x86_64-linux.guest-static-elf
nix build --no-link \
  .#checks.x86_64-linux.rust-deny \
  .#checks.aarch64-linux.rust-deny \
  .#checks.x86_64-linux.rust-audit \
  .#checks.aarch64-linux.rust-audit
```

Guest static checks read only exact system-and-musl-target
`production/closure.json` and `production/Cargo.lock`, never standalone or root
locks. Review `make flake-matrix-pin`; then drift and builds above must pass.

### 5. Amend Spec 003 after merge

After this ADR merges, amend Spec 003's four-hub, three-product-workspace model
to this model and re-panel before implementation resumes. This ADR PR makes no
Spec or code edit; the walker stays separate.

## Consequences

- Product packages share one dependency resolution and update event.
- Broker and guest lock-update cadence and visual isolation are lost; accepted.
- Selected Cargo closure policy governs privileged/static minimality; native
  target edges enforce first-party configuration.
- Normal `libshpool` may compile outside production while its code is gated.
- The synthetic splice workspace, broker hub, guest hub, and broker-specific
  repin exception are unnecessary.

## Alternatives considered

### Keep separate product workspaces and generated splices

Rejected. They duplicate workspace and lock lifecycle. Measured package build,
native-target, and closure controls work without treating lock union as reach.

### Require each Bazel external repository to equal one Cargo closure

Rejected. It recreates per-context hubs and makes the external repository an
authority it is not. Selected native dependency and cfg checks catch leakage.

### Keep libshpool optional and add crate.spec

Rejected. Product repin failed on that shape and passed when the production
dependency was manifest-visible. Extra compilation beats duplicate declaration.

### Merge the no-bash walker

Rejected. It has a real tooling boundary and no product path dependency.

## Invariants this decision creates

1. `packages/Cargo.lock` is the only authoritative product Cargo lock.
2. Broker and guest production are always package and feature selected.
3. Generic main clippy and tests exclude broker and guest; contract tests leave
   main tests, not clippy, policy, or fixture compilation.
4. Broker default, layer 1, and fake lanes stay serial and target-isolated.
5. Nix uses root source and lock without weakening dependency or ELF checks;
   package checks bind exact system and GNU or musl target artifacts.
6. Bazel has product and walker hubs; `main`, `broker`, and `guest` refuse.
7. Every selected context proves one root and a nonempty exact census before
   predicates; native first-party contexts, not the product external union,
   define actual Bazel dependencies and features.
8. Broker and guest inputs cover production and root-dev closure, drift, exact
   sources, and hostile-shell refusal and are enforcing.
9. The guest license blocker is resolved by reviewed policy in the merge
   change, not waived or misreported.
10. Spec 003 is amended and re-panelled after this ADR merges and before
    implementation resumes.
