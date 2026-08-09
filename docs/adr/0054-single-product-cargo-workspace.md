# ADR 0054: Single product Cargo workspace

- Status: Accepted
- Date: 2026-08-05
- Amends: [ADR 0052](0052-bazel-rust-build-and-test.md), replacing its
  product-workspace, dependency-hub, and lock inventory.
- Scope: Cargo workspace membership and locks for product packages, Cargo and
  Nix package selection, Bazel dependency hubs and configured first-party
  targets, and package-scoped supply-chain enforcement.
- Non-scope: implementing the merge, changing Rust behavior, amending Spec 003
  in this PR, moving the no-bash walker, or weakening static and ELF checks.
- Threat-model non-goal: contributor mutation commands run in a trusted local
  operator shell. They are not a credential or sandbox boundary and are
  unreachable from workflows and Make targets.

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

Commit `98ba0f9f` added broker and guest to the root workspace, removed their
nested locks and workspace tables, and regenerated the root lock offline. The
workspace moved from 54 members and 544 lock packages to 56 and 608. Locked,
offline selection showed no guest runner in the broker closure and no broker
in the guest closure. Standalone and unified test censuses were identical:

| Context | Root selector after merge | Cases |
| --- | --- | ---: |
| Broker default | `-p d2b-priv-broker --no-default-features` | 557 |
| Broker layer 1 | `-p d2b-priv-broker --no-default-features --features layer1-bootstrap` | 492 |
| Broker fake | `-p d2b-priv-broker --no-default-features --features fake-backends` | 559 |
| Guest production | `-p d2b-guest-shell-runner --no-default-features --features real-libshpool` | 11 |

All three isolated broker streams passed; guest passed 11 of 11; generic main
passed 5,114 of 5,114 with 6 skipped. The committed baseline excludes
`d2b-contract-tests` from generic tests but not from
`cargo clippy --locked --workspace --all-targets -- -D warnings`. The spike's
clippy exclusion is therefore rejected. Unified fixture contracts passed 359
of 359 in 105 seconds. Existing clippy, `make test-policy`, and fixture
compilation remain required.

Release artifacts from isolated target directories were:

| Binary | Standalone raw | Unified raw | Standalone stripped | Unified stripped |
| --- | ---: | ---: | ---: | ---: |
| Broker | 10,262,496 | 10,240,368 | 7,420,096 | 7,393,864 |
| Guest runner | 5,869,600 | 5,869,744 | 4,107,416 | 4,110,152 |

Both actual Nix derivations built. Broker remained a dynamically linked host
binary at 8,635,480 bytes. Guest was a 5,313,928-byte static PIE with no ELF
interpreter or `NEEDED` entry. Selected-dependency, static ELF, root deny and
audit, flake evaluation, and `make test-policy` checks passed. Moving Nix to
the root source and lock changed no static or ELF acceptance criterion.

### Package-closure policy spike

Locked, offline root metadata produced selected cargo-deny metadata and pruned
audit locks. Broker had 108 production and 153 dev-inclusive packages; deny
and no-ignore audit passed. Guest had 171 and 181; audit passed with exactly
`--ignore RUSTSEC-2024-0384`. Guest deny retained six pre-existing license
findings: BSD-3-Clause for `bindgen` and `instant`, ISC for `inotify`,
`inotify-sys`, and `libloading`, and CC0-1.0 for `notify`.

A disconnected GPL canary did not affect broker; connecting it made
cargo-deny exit 4. The main-only `wl-proxy` git source fails broker policy
when the whole union is supplied but is absent from the passing selected
broker closure. Thus unrelated lock members are harmless and newly connected
members are enforced. The six guest findings remain a narrow implementation
blocker; dropping dependencies or weakening policy is not a remedy.

### Unified Bazel spike

The Spec 003 W0 spike used Bazel 8.6.0, `rules_rust` 0.73.0,
`cargo-bazel` 0.18.0, and Cargo/rustc 1.97.0. Its 553-package lock and the
later integration spike's 608-package lock describe different branch content.
`MODULE.bazel` declared only `product` and `walker`; package-local generation,
module-lock error mode, and a 321-label full query passed.

The following native targets and representative tests passed:

```text
bazel build //packages/d2b-priv-broker:d2b-priv-broker
bazel build //packages/d2b-guest-shell-runner:d2b-guest-shell-runner-real-libshpool
bazel test //packages/d2b-priv-broker:bridge_lifecycle_default
bazel test //packages/d2b-priv-broker:bridge_lifecycle_layer1_bootstrap
bazel test //packages/d2b-priv-broker:bridge_lifecycle_fake_backends
bazel test //packages/d2b-guest-shell-runner:tests
```

The follow-up guest `cli` label also passed. The spike does not claim two
broker tests that still need declared fixtures or an unavailable `sh`.
Queries proved empty, `layer1-bootstrap`, and `fake-backends` contexts and
matching rustc cfgs. There were 46 broker context targets, 7,721 broker
production dependency labels, and 10,332 guest labels. Product held 596 crate
records and 297 external labels; walker held 9. Cargo selected 95 identities
per broker context and 135 for guest, all contained in product. Broker reached
no guest runner or unrelated first-party sibling.

Making `libshpool` a normal dependency while retaining the
`real-libshpool` code feature removed the only `crate.spec`. Generation,
builds, tests, queries, and `cargo xtask bazel-repin --hub product` then
passed, changing only `bazel/cargo/product.lock`. Non-production compilation
of the dependency is the accepted cost of manifest-driven resolution.

## Decision

### Contributor mutation workflow and threat model

Lock, hub, and policy-input regeneration are contributor-only mutations. They
follow existing repository practice and run from a trusted local operator
shell, not from a workflow or Make target. The shell is not a credential or
sandbox boundary. Its `HOME`, startup configuration, functions, and other
operator-controlled state are explicitly outside this decision's security
model.

The exact workflow has two steps:

1. From the repository root, enter the pinned environment with `nix develop`.
2. Inside that environment, run `cd packages`, then the exact Cargo command
   named below.

All package-local command blocks in this decision assume those steps. Entering
`packages/` is load-bearing: rustup finds `rust-toolchain.toml` there and Cargo
finds `.cargo/config.toml` and the `xtask` alias there. Continuous integration
and gates instead call approved Make targets in controlled environments.
Package-policy checks remain hermetic through vendored sources and the pinned
RustSec database.

### 1. Use one authoritative product workspace and lock

Add `d2b-priv-broker` and `d2b-guest-shell-runner` to
`packages/Cargo.toml` members and remove them from `exclude`. Remove each
package's nested `[workspace]`, workspace-local `[profile.*]` tables, and
`Cargo.lock`. Generate and verify the sole authoritative product lock with:

```text
cargo generate-lockfile --offline
cargo metadata --locked --offline --format-version 1
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

After the two entry steps, the package selectors are:

```text
cargo test --locked -p d2b-priv-broker --no-default-features -- --test-threads 1
cargo test --locked -p d2b-priv-broker --no-default-features --features layer1-bootstrap -- --test-threads 1
cargo test --locked -p d2b-priv-broker --no-default-features --features fake-backends -- --test-threads 1
cargo fmt -p d2b-guest-shell-runner --check
cargo clippy --locked -p d2b-guest-shell-runner --no-default-features --features real-libshpool --all-targets -- -D warnings
cargo nextest run --locked -p d2b-guest-shell-runner --no-default-features --features real-libshpool
```

Broker lanes remain three serial `cargo test` processes in isolated target
directories because they mutate process-global signal and reap state. Guest
doctest and harness-free companions reuse the same root manifest, package,
default-feature, and `real-libshpool` selectors.

The exact generic-main split is:

```text
cargo clippy --locked --workspace --all-targets --exclude d2b-priv-broker --exclude d2b-guest-shell-runner -- -D warnings
cargo nextest run --locked --workspace --exclude d2b-contract-tests --exclude d2b-priv-broker --exclude d2b-guest-shell-runner
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
lock stays byte-identical. After the two entry steps, these are the only repin
commands:

```text
cargo xtask bazel-repin --hub product
cargo xtask bazel-repin --hub walker
```

`main`, `broker`, and `guest` are retired, not aliases. They fail before Bazel
starts, while `product` and `walker` remain accepted. They print fixed,
actionable diagnostics:

| Refused hub | Diagnostic |
| --- | --- |
| `main` | `Hub 'main' is retired; after entering nix develop, run from packages/: cargo xtask bazel-repin --hub product` |
| `broker` | `Hub 'broker' is retired; after entering nix develop, run from packages/: cargo xtask bazel-repin --hub product` |
| `guest` | `Hub 'guest' is retired; after entering nix develop, run from packages/: cargo xtask bazel-repin --hub product` |

Tests bind each refused-hub mapping and exact diagnostic line. They pass the
remediation through an injected non-mutating executor and require the exact
`cargo xtask bazel-repin --hub product` argv with cwd fixed to `packages/`;
the genuine repin command is never run by a test, workflow, or Make target.
A `cd packages` or `packages/` path prefix must fail as a duplicated
`packages/packages` path. No final newline contract is created.
Changing the walker lock remains separately reviewed.

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

After the two entry steps, the repository-owned generator has these exact
entry points:

```text
cargo xtask gen-package-policy-inputs
cargo xtask gen-package-policy-inputs --check
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
repository-relative and says to enter `nix develop` from the repository root,
then prints these package-local steps:

```text
cd packages
cargo xtask gen-package-policy-inputs
Review and commit the generated changes under packages/policy-inputs/.
cargo xtask gen-package-policy-inputs --check
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

Recurring enforcement stays in existing Layer-1 jobs:

- `make test-rust-supply-chain` runs generated source census, deny, and pinned
  offline audit logic for broker GNU and guest musl on each native runner.
- `make test-drift` enforces generation `--check`, the eight-check inventory
  and exact mapping, with missing-check, cross-system, wrong-runner,
  wrong-target, and separate per-architecture foreign-system and remote-builder
  plants.
- `make test-flake` owns realization. `D2B_FLAKE_REALIZED_CHECKS` adds
  `broker-production-dependency-policy`, `guest-shell-runner-static-dependency-policy`,
  `broker-production-package-policy`, `guest-real-libshpool-package-policy`, and
  `broker-host-artifact-contract`, and `guest-static-elf`, retaining
  `video-binary-contract`. X86 shards build those six checks on a native
  x86_64 runner. Existing job `test-flake-aarch64` retains its id and context,
  moves to public `ubuntu-24.04-arm`, and runs the same target for the six
  aarch64 checks; no top-level job is added. Neither lane passes a foreign
  `--system` or configures a remote builder.

Implementation pins it as `flake-aarch64-realized` at 60 minutes in `tests/layer1-jobs.json`, regenerates the workflow and realized class, and makes
`make flake-matrix-pin` regenerate both system inventories. Either stale pin fails drift, making aarch64 wiring and execution recurrent evidence.

### Amendment: six native checks

This ADR adopts the six-check native inventory already required by the
amendment to ADR 0052. For each native system, the enforcing realization set
is exactly:

```text
broker-production-dependency-policy
guest-shell-runner-static-dependency-policy
broker-production-package-policy
guest-real-libshpool-package-policy
broker-host-artifact-contract
guest-static-elf
```

The two policy-input contexts for each architecture, the two native artifact
contracts, and the six-check inventory are one committed manifest. The flake,
shell gates, CI generation, xtask, and Bazel inventory either consume that
manifest or fail closed when their inventory differs. `video-binary-contract`
remains a separate realized check and is not part of the six native
architecture-specific checks.

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
| `wrong-runner` | Put a native-system realization on the other architecture's runner. |
| `wrong-target` | Change only the embedded GNU or musl target. |
| `x86_64-foreign-system` | Add foreign `--system` to the x86_64 native mapping; expect `x86_64-linux native realization must not set a foreign system`. |
| `x86_64-remote-builder` | Add `--builders` to the x86_64 native mapping; expect `x86_64-linux native realization must not configure a remote builder`. |
| `aarch64-foreign-system` | Add foreign `--system` to the aarch64 native mapping; expect `aarch64-linux native realization must not set a foreign system`. |
| `aarch64-remote-builder` | Add `--builders` to the aarch64 native mapping; expect `aarch64-linux native realization must not configure a remote builder`. |
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
operator enters `nix develop` at the repository root, then runs:

```text
cd packages
cargo xtask gen-package-policy-inputs
Review and commit packages/d2b-guest-shell-runner/deny.toml and the generated packages/policy-inputs/ changes.
cargo xtask gen-package-policy-inputs --check
cd ..
make flake-matrix-pin
make test-drift
```

Then run this block on a native x86_64-linux runner:

```text
nix build --no-link \
  .#checks.x86_64-linux.broker-production-dependency-policy \
  .#checks.x86_64-linux.guest-shell-runner-static-dependency-policy \
  .#checks.x86_64-linux.broker-production-package-policy \
  .#checks.x86_64-linux.guest-real-libshpool-package-policy \
  .#checks.x86_64-linux.broker-host-artifact-contract \
  .#checks.x86_64-linux.guest-static-elf \
  .#checks.x86_64-linux.rust-deny \
  .#checks.x86_64-linux.rust-audit
make test-rust-supply-chain
make test-policy
```

The recurring `flake-eval-x86-realized` lane must pass on that same native
runner. Separately, run this block on a native aarch64-linux runner:

```text
nix build --no-link \
  .#checks.aarch64-linux.broker-production-dependency-policy \
  .#checks.aarch64-linux.guest-shell-runner-static-dependency-policy \
  .#checks.aarch64-linux.broker-production-package-policy \
  .#checks.aarch64-linux.guest-real-libshpool-package-policy \
  .#checks.aarch64-linux.broker-host-artifact-contract \
  .#checks.aarch64-linux.guest-static-elf \
  .#checks.aarch64-linux.rust-deny \
  .#checks.aarch64-linux.rust-audit
make test-rust-supply-chain
```

The recurring `test-flake-aarch64` lane must pass on that same native runner.
Neither block may set a foreign system, use `--builders`, or rely on a remote
builder. No single invocation builds both systems. Guest static checks read
only the exact native system-and-musl-target `production/closure.json` and
`production/Cargo.lock`, never standalone or root locks. The common pin and
drift step and both native blocks must pass.

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
5. Nix uses root source and lock without weakening checks; package policy and static ELF realization recur on both systems.
6. Bazel has product and walker hubs; `main`, `broker`, and `guest` refuse.
7. Every selected context proves one root and a nonempty exact census before
   predicates; native first-party contexts, not the product external union,
   define actual Bazel dependencies and features.
8. Existing Layer-1 supply-chain, drift, and flake jobs enforce the four target
   contexts, eight wrappers, wrong-runner/system and architecture-specific
   foreign-system/remote-builder refusals, and dual-system pins.
9. The guest license blocker is resolved by reviewed policy in the merge
   change, not waived or misreported.
10. Spec 003 is amended and re-panelled after this ADR merges and before
    implementation resumes.
