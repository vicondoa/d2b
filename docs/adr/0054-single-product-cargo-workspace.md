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
cargo clippy --locked --manifest-path packages/Cargo.toml --workspace --all-targets -- -D warnings
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
independent. `cargo xtask gen-bazel --check` and module-lock error mode passed.
A full `bazel query //... --output=label` returned 321 labels.

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
nix develop --command cargo xtask bazel-repin --hub product
```

It generated only `bazel/cargo/product.lock`. Production already always
enables `real-libshpool`; default code remains feature-gated. Compiling the
normal dependency in a non-production guest context is an accepted cost of
manifest-driven hub resolution.

## Decision

### 1. Use one authoritative product workspace and lock

Add `d2b-priv-broker` and `d2b-guest-shell-runner` to
`packages/Cargo.toml` members and remove them from `exclude`. Remove each
package's nested `[workspace]`, workspace-local `[profile.*]` tables, and
`Cargo.lock`. Generate and verify the sole authoritative product lock with:

```text
cargo generate-lockfile --offline --manifest-path packages/Cargo.toml
cargo metadata --locked --offline --manifest-path packages/Cargo.toml \
  --format-version 1
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
cargo test --locked --manifest-path packages/Cargo.toml \
  -p d2b-priv-broker --no-default-features \
  -- --test-threads 1
cargo test --locked --manifest-path packages/Cargo.toml \
  -p d2b-priv-broker --no-default-features \
  --features layer1-bootstrap -- --test-threads 1
cargo test --locked --manifest-path packages/Cargo.toml \
  -p d2b-priv-broker --no-default-features \
  --features fake-backends -- --test-threads 1
cargo fmt --manifest-path packages/Cargo.toml \
  -p d2b-guest-shell-runner --check
cargo clippy --locked --manifest-path packages/Cargo.toml \
  -p d2b-guest-shell-runner --no-default-features \
  --features real-libshpool --all-targets -- -D warnings
cargo nextest run --locked --manifest-path packages/Cargo.toml \
  -p d2b-guest-shell-runner --no-default-features \
  --features real-libshpool
```

Broker lanes remain three serial `cargo test` processes in isolated target
directories because they mutate process-global signal and reap state. Guest
doctest and harness-free companions reuse the same root manifest, package,
default-feature, and `real-libshpool` selectors.

The exact generic-main split is:

```text
cargo clippy --locked --manifest-path packages/Cargo.toml \
  --workspace --all-targets \
  --exclude d2b-priv-broker \
  --exclude d2b-guest-shell-runner -- -D warnings
cargo nextest run --locked --manifest-path packages/Cargo.toml \
  --workspace \
  --exclude d2b-contract-tests \
  --exclude d2b-priv-broker \
  --exclude d2b-guest-shell-runner
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

The workspace merge regenerates only the product Bazel-side lock with
`nix develop --command cargo xtask bazel-repin --hub product`. The walker lock
stays byte-identical. Its corresponding command is
`nix develop --command cargo xtask bazel-repin --hub walker`, used only when
the walker Cargo inputs change in a separately reviewed change.

Product code is represented by native first-party targets. Each broker and
guest configured context declares its own direct first-party and `@product`
dependencies and feature flags. The repository owner accepts `@product` as an
external package and feature superset. Exact third-party feature parity with
each Cargo context is not an invariant. The selected Cargo closure and its
package policy are the security authority; native configured targets remain
authoritative for actual first-party Bazel edges and features.

Every Bazel selected-root or containment checker, for broker default, broker
layer1, broker fake, and guest real-libshpool, runs these steps in order:

1. Assert the selected root exists exactly once.
2. Materialize the complete context closure and assert it is nonempty.
3. Assert its exact generated census before evaluating any predicate. The
   census includes the root, configured first-party targets, expected direct
   first-party dependencies, cfg and feature values, and all reached external
   identities.
4. Then prove product-hub and product-lock containment, expected configured
   first-party dependencies and features, zero unrelated first-party siblings,
   and zero guest-runner dependencies in broker production through `cquery`
   and `aquery`.

Each package-closure checker independently asserts that its selected Cargo root
exists exactly once, its closure is nonempty, and its exact generated census of
normal and build edges is complete before applying a containment, minimality,
deny, audit, or leakage predicate. Every checker has seeded negatives that
must fail for a missing selected root, an empty or truncated closure, an
omitted normal edge, an omitted build edge, and connected leakage from one
unrelated first-party sibling. An empty census cannot satisfy an absence
predicate. The `@product` external union is accepted; a wrong native edge or
an inexact selected Cargo closure is not.

### 4. Enforce package-scoped selected-closure policy

The repository-owned generator has these exact root-runnable entry points:

```text
cargo run --manifest-path packages/Cargo.toml -p xtask -- gen-package-policy-inputs
cargo run --manifest-path packages/Cargo.toml -p xtask -- gen-package-policy-inputs --check
```

It deterministically derives two tracked input sets for broker production with
default features disabled and guest production with `real-libshpool`:

1. `production/closure.json` plus `production/Cargo.lock`: the selected normal
   and build closure, resolved features, and pruned lock used for binary and
   static-dependency minimality.
2. `policy/metadata.json` plus `policy/Cargo.lock`: the dev-inclusive package
   graph and filtered lock used to preserve the existing package deny and
   audit semantics.

The resulting paths are:

```text
packages/policy-inputs/broker-production/{production,policy}/
packages/policy-inputs/guest-real-libshpool/{production,policy}/
```

Both sets derive from locked, offline root metadata and the root lock and bind
the selected root, target, package identity, version, source, checksum, edge
kind, and resolved features. The policy set includes the selected package's
dev edges and every normal/build transitive edge they reach. Every drift
diagnostic lists every stale generated path repository-relative and ends with
this remediation, in this order:

```text
cargo run --manifest-path packages/Cargo.toml -p xtask -- gen-package-policy-inputs
Review and commit the generated changes under packages/policy-inputs/.
cargo run --manifest-path packages/Cargo.toml -p xtask -- gen-package-policy-inputs --check
```

For each policy set, the implementation reuses ADR 0052's pinned offline source
materialization rather than defining a second vendor path. Nix uses the
root-lock-derived filtered lock through `rustPlatform.importCargoLock`; the
Bazel carrier re-declares the same registry URL and lock checksum or the pinned
git rev and archive checksum. Before cargo-deny starts, the gate asserts the
exact nonempty selected external source set, count, and checksums and exact
equality with the metadata and filtered-lock identities. Any selected external
source that is missing, extra, unreadable, or checksum-mismatched refuses
before policy execution. A clean-environment negative, with no ambient Cargo
source cache, removes one selected source and must fail before cargo-deny
starts.

The existing package configs run over the dev-inclusive metadata with no
`--exclude-dev`:

```text
cargo deny check \
  --metadata-path packages/policy-inputs/broker-production/policy/metadata.json \
  --config packages/d2b-priv-broker/deny.toml \
  bans licenses sources
cargo deny check \
  --metadata-path packages/policy-inputs/guest-real-libshpool/policy/metadata.json \
  --config packages/d2b-guest-shell-runner/deny.toml \
  bans licenses sources
```

Package audits run the generated dev-inclusive `policy/Cargo.lock` files
against the repository-pinned RustSec database with `--no-fetch`. Broker has
no ignore; guest has exactly `--ignore RUSTSEC-2024-0384`. The existing
aggregate root deny and audits remain. The root-lock audit retains exactly
`--ignore RUSTSEC-2026-0194` and `--ignore RUSTSEC-2026-0195`; the
`packages/Cargo.guest.lock` audit retains no ignore. Aggregate and package
checks may each block the change.

```text
cargo-audit audit \
  --file packages/policy-inputs/broker-production/policy/Cargo.lock \
  --db ${advisoryDbGit} --no-fetch
cargo-audit audit \
  --file packages/policy-inputs/guest-real-libshpool/policy/Cargo.lock \
  --db ${advisoryDbGit} --no-fetch --ignore RUSTSEC-2024-0384
```

Source, license, ban, and advisory failures are enforcing. A main-only
dependency must not alter a broker or guest closure verdict. Adding an edge
that makes it reachable must alter the generated input and subject it to that
package's policy. Seeded dev-only forbidden-license, forbidden-source, and
advisory cases must each fail, proving dev edges were not filtered away.

The guest real-libshpool policy currently has six pre-existing license
denials: BSD-3-Clause for `bindgen` and `instant`, ISC for `inotify`,
`inotify-sys`, and `libloading`, and CC0-1.0 for `notify`. The workspace merge
remains blocked unless the same change narrowly updates
`packages/d2b-guest-shell-runner/deny.toml` for precisely those six selected
package/license pairs. A blanket license expansion is not the remedy. The
exact root-runnable remedy and recheck sequence is:

```text
cargo run --manifest-path packages/Cargo.toml -p xtask -- gen-package-policy-inputs
Review and commit packages/d2b-guest-shell-runner/deny.toml and the generated packages/policy-inputs/ changes.
cargo run --manifest-path packages/Cargo.toml -p xtask -- gen-package-policy-inputs --check
make test-rust-supply-chain
make test-policy
nix build --no-link \
  .#checks.x86_64-linux.guest-shell-runner-static-dependency-policy
nix build --no-link .#checks.x86_64-linux.guest-static-elf
nix build --no-link .#checks.x86_64-linux.rust-deny
nix build --no-link .#checks.x86_64-linux.rust-audit
```

The exact Nix check
`checks.x86_64-linux.guest-shell-runner-static-dependency-policy` reads only
the generated guest real-libshpool `production/closure.json` and
`production/Cargo.lock`. It never reads either deleted standalone lock or the
full root lock.

### 5. Amend Spec 003 after merge

Spec 003 currently requires four hubs and three product workspace authorities.
After this ADR merges, Spec 003 must be amended to this two-hub model and
re-panelled before implementation resumes. This ADR PR makes no Spec 003 or
code edit. The no-bash walker remains separate.

## Consequences

- Product packages share one dependency resolution and update event.
- Independent broker and guest lock-update cadence and lock-level visual
  isolation are lost. This is the accepted tradeoff.
- Selected Cargo closure policy is the security authority for privileged and
  static dependency minimality; native target edges enforce first-party
  configuration.
- A normal `libshpool` dependency may compile in non-production guest contexts
  even while its code is feature-gated.
- The synthetic splice workspace, broker hub, guest hub, and broker-specific
  repin exception are unnecessary.

## Alternatives considered

### Keep separate product workspaces and generated splices

Rejected. They duplicate workspace structure and lock lifecycle. The measured
package-selected build, native target, and closure-policy controls enforce the
property that matters without treating lock union as code reachability.

### Require each Bazel external repository to equal one Cargo closure

Rejected. It recreates per-context hubs and makes the external repository an
authority it is not. Selected native dependency and cfg checks catch leakage.

### Keep libshpool optional and add crate.spec

Rejected. The standard product repin failed on that exceptional shape and
succeeded when the production dependency was manifest-visible. The small
extra compilation cost is preferred to a second dependency declaration.

### Merge the no-bash walker

Rejected. It has a real tooling boundary and no product path dependency.

## Invariants this decision creates

1. `packages/Cargo.lock` is the only authoritative product Cargo lock.
2. Broker and guest production are always package and feature selected.
3. Generic main clippy and tests exclude broker and guest; contract tests are
   excluded from main tests, not from clippy, policy, or fixture compilation.
4. Broker default, layer 1, and fake lanes stay serial and target-isolated.
5. Nix uses root source and lock without weakening selected-dependency or ELF
   checks; guest static dependency policy reads only generated guest production
   closure inputs.
6. Bazel has one product hub and one separate walker hub.
7. Every selected context proves one root and a nonempty exact census before
   predicates; native first-party contexts, not the product external union,
   define actual Bazel dependencies and features.
8. Broker and guest package-closure deny and audit inputs are generated,
   checked for drift and exact pinned source completeness, and enforcing.
9. The guest license blocker is resolved by reviewed policy in the merge
   change, not waived or misreported.
10. Spec 003 is amended and re-panelled after this ADR merges and before
    implementation resumes.
