# ADR 0054: Single product Cargo workspace

- Status: Proposed
- Date: 2026-08-05
- Amends: [ADR 0052](0052-bazel-rust-build-and-test.md), replacing its
  workspace, dependency-hub, and lock inventory when this ADR is accepted.
- Scope: Cargo workspace membership and locks for product packages, the
  `crate_universe` hub boundary, targeted Cargo and Nix builds, configured
  first-party Bazel variants, and package-scoped supply-chain evidence.
- Non-scope: implementing the workspace merge, changing Rust package contents,
  rewriting Spec 003 in this PR, or changing static and ELF acceptance checks.

## Context

At v3 commit `9bd6e2ac`, `packages/Cargo.toml` is a resolver-v2 workspace that
explicitly excludes `d2b-priv-broker` and `d2b-guest-shell-runner`. Each
excluded package declares a nested `[workspace]` root and has a standalone
authoritative lock. The broker nevertheless has path dependencies on product
packages in the parent tree.

ADR 0052 planned a separate `crate_universe` hub for each of `main`, `broker`,
`guest`, and `walker`. A prior draft of this ADR added a generated synthetic
workspace because `crate_universe` could not splice the broker workspace and
its cross-workspace path dependencies directly.

That solves a boundary d2b does not need. The broker and guest runner were
separate workspaces to control binary dependency closure and size, not because
their locks form a required trust boundary. Cargo package selection controls
what is built. Cargo workspace membership controls shared resolution and
metadata; it does not create a dependency between sibling packages.

The no-bash AST walker is different. It is closed gate plumbing under
`tests/tools/`, outside the product package tree, and has no path dependency
into `packages/`. Keeping that tool separate preserves a real repository
boundary without making the product graph harder to represent.

## Evidence

### Cargo and rules_rust behavior

The upstream-supported behavior is sufficient:

- The Cargo [workspace reference](https://doc.rust-lang.org/cargo/reference/workspaces.html)
  states that members share one `Cargo.lock` and output directory. Its
  [package selection](https://doc.rust-lang.org/cargo/reference/workspaces.html#package-selection)
  section says `-p` selects packages while `--workspace` selects the workspace.
  The [`workspace.dependencies` section](https://doc.rust-lang.org/cargo/reference/workspaces.html#the-dependencies-table)
  requires each member to inherit a dependency explicitly. Membership alone
  does not link sibling code.
- [`cargo build` package selection](https://doc.rust-lang.org/cargo/commands/cargo-build.html#package-selection)
  says `-p` builds only the specified packages and `--workspace` builds all
  members. Therefore `cargo build -p d2b-priv-broker` selects the broker and
  its dependency closure, not the guest runner.
- Cargo's [feature-unification reference](https://doc.rust-lang.org/cargo/reference/features.html#feature-unification)
  and [resolver-v2 command-line rules](https://doc.rust-lang.org/cargo/reference/features.html#resolver-version-2-command-line-flags)
  apply features to the packages selected by `-p` or `--workspace`. Features
  are unified where the selected build graph uses the same dependency. A
  broad workspace build can therefore unify across all selected packages; a
  targeted package build does not select an unrelated sibling.
- The official `rules_rust` 0.73.0
  [Cargo workspace integration example](https://github.com/bazelbuild/rules_rust/tree/0.73.0/crate_universe/tests/integration/cargo_workspace)
  gives the root workspace manifest to `crate.from_cargo`, then declares
  first-party `rust_library`, `rust_binary`, and `rust_test` targets natively.
  Its `number_printer` target depends on first-party `//printer` directly and
  receives third-party dependencies from the generated crate repository.
  This is the product-hub shape selected here.

Cross-workspace splicing remains the less reliable path. Upstream issues
[#1525](https://github.com/bazelbuild/rules_rust/issues/1525) and
[#3571](https://github.com/bazelbuild/rules_rust/issues/3571) remained open on
2026-08-05. Issue
[#1773](https://github.com/bazelbuild/rules_rust/issues/1773) was closed after
upstream changes, but its follow-up still records absolute-path lock output
for packages outside the top-level workspace. These issues cover different
shapes; they do not establish that every cross-workspace case fails. They do
show that a synthetic projection or fork would depend on a shape-sensitive
surface that the supported single-workspace example avoids.

`rules_rust` 0.73.0, published 2026-07-31, was still the
[latest release](https://github.com/bazelbuild/rules_rust/releases/tag/0.73.0)
and the latest Bazel Central Registry version on 2026-08-05. There was no
newer release to measure for this decision.

### Local product-workspace spike

The spike used two detached checkouts of v3 commit `9bd6e2ac` in the
repository's pinned development environment. `<worktree>` is an operator-owned
scratch parent:

```text
git worktree add --detach <worktree>/standalone 9bd6e2ac
git worktree add --detach <worktree>/unified 9bd6e2ac
```

In `<worktree>/unified`, the spike added both packages to the root `members`,
removed them from `exclude`, removed only their nested `[workspace]` and
workspace-local profile tables, and removed their standalone lock files. It
then regenerated the root lock offline. Standalone baselines and unified
builds used distinct target directories:

```text
cd <worktree>/standalone
CARGO_TARGET_DIR=<worktree>/target/broker-standalone cargo build \
  --locked --release --manifest-path packages/d2b-priv-broker/Cargo.toml \
  --no-default-features
CARGO_TARGET_DIR=<worktree>/target/guest-standalone cargo build \
  --locked --release \
  --manifest-path packages/d2b-guest-shell-runner/Cargo.toml \
  --no-default-features --features real-libshpool
grep -c '^\[\[package\]\]' packages/Cargo.lock

cd <worktree>/unified
cargo generate-lockfile --offline --manifest-path packages/Cargo.toml
CARGO_TARGET_DIR=<worktree>/target/broker-unified cargo build \
  --locked --offline --release --manifest-path packages/Cargo.toml \
  -p d2b-priv-broker --no-default-features
CARGO_TARGET_DIR=<worktree>/target/guest-unified cargo build \
  --locked --offline --release --manifest-path packages/Cargo.toml \
  -p d2b-guest-shell-runner --no-default-features \
  --features real-libshpool
cargo tree --locked --offline --manifest-path packages/Cargo.toml \
  -p d2b-priv-broker
cargo tree --locked --offline --manifest-path packages/Cargo.toml \
  -p d2b-guest-shell-runner --features real-libshpool
grep -c '^\[\[package\]\]' packages/Cargo.lock

mkdir -p <worktree>/stripped
stat -c '%n %s' \
  <worktree>/target/broker-standalone/release/d2b-priv-broker \
  <worktree>/target/guest-standalone/release/d2b-guest-shell-runner \
  <worktree>/target/broker-unified/release/d2b-priv-broker \
  <worktree>/target/guest-unified/release/d2b-guest-shell-runner
strip -s -o <worktree>/stripped/broker-standalone \
  <worktree>/target/broker-standalone/release/d2b-priv-broker
strip -s -o <worktree>/stripped/guest-standalone \
  <worktree>/target/guest-standalone/release/d2b-guest-shell-runner
strip -s -o <worktree>/stripped/broker-unified \
  <worktree>/target/broker-unified/release/d2b-priv-broker
strip -s -o <worktree>/stripped/guest-unified \
  <worktree>/target/guest-unified/release/d2b-guest-shell-runner
stat -c '%n %s' <worktree>/stripped/*
```

Unstripped sizes were recorded with `stat -c %s`. Stripped comparison copies
were produced with the same `strip -s` tool. The results were:

| Binary | Standalone stripped | Unified stripped | Change |
| --- | ---: | ---: | ---: |
| `d2b-priv-broker` | 7,419,824 | 7,423,752 | +3,928 (+0.053%) |
| `d2b-guest-shell-runner` | 4,107,416 | 4,107,016 | -400 (-0.010%) |

The unstripped broker changed by +0.060% and the unstripped guest runner by
-0.030%. The root lock grew from 544 to 601 packages. Targeted `cargo tree`
output contained only each selected package's dependency closure. Version
shifts in those trees came from resolving one shared lock, not from linking
the unrelated sibling package.

The measurement does not claim byte identity. It demonstrates that workspace
membership does not defeat the binary-size purpose of the current targeted
builds.

## Decision

### 1. Use one Cargo workspace for product packages

`packages/d2b-priv-broker` and `packages/d2b-guest-shell-runner` become members
of the existing resolver-v2 workspace rooted at `packages/Cargo.toml`.
Implementation removes their nested `[workspace]` roots and their standalone
authoritative lock roles. `packages/Cargo.lock` is the sole authoritative
Cargo lock for product packages.

The broker and guest runner remain separate packages. Both retain
`default = []`, explicit package-local dependencies, and only the features
their own targets require. Workspace dependencies may centralize a version,
but each package must still opt into every dependency it uses.

### 2. Keep two crate_universe hubs

`crate_universe` has exactly two dependency hubs:

1. the product hub, derived from `packages/Cargo.toml` and
   `packages/Cargo.lock`; and
2. the walker tool hub, derived from
   `tests/tools/no-bash-ast-walker/Cargo.toml` and its lock.

The walker remains a separate tooling workspace because it is closed gate
plumbing outside `packages/` and has no cross-workspace path dependencies.

`packages/Cargo.guest.lock` remains the existing generated static-guest
closure input. It is neither a Cargo workspace authority nor a
`crate_universe` hub lock.

### 3. Keep package builds and test contexts separate

One workspace does not mean one build or test invocation.

Release and Nix builds select one package at a time with `--locked --release`,
`-p <package>`, and explicit feature flags. Broker production remains an empty
default-feature build. Guest runner production explicitly enables
`real-libshpool`.

The dedicated broker default, `layer1-bootstrap`, and `fake-backends` test
invocations remain separate. The guest runner's `real-libshpool` invocation
also remains separate. Broad main-workspace test and clippy lanes exclude the
broker and guest runner where necessary so those contexts are not compiled or
run twice. Formatting or another operation that is intentionally global need
not invent an exclusion.

### 4. Use native first-party Bazel targets

The product `crate_universe` hub supplies third-party dependencies. Product
packages, including broker and guest runner, are native first-party Bazel
targets. There is no generated splice workspace and no broker-specific
dependency hub.

First-party variants are keyed by configured context where their feature or
dependency graphs differ. Broker production, default test,
`layer1-bootstrap`, and `fake-backends` contexts remain independently checked.
Sharing the third-party hub does not permit one context's features, targets,
or test census to stand in for another's.

### 5. Build Nix packages from the workspace root

Nix broker and guest-runner derivations use the root `packages/` source and
root `packages/Cargo.lock`. Each derivation passes an explicit `--package`
selection and explicit feature flags. Existing static-link, ELF interpreter,
dynamic dependency, and release-artifact checks remain unchanged.

### 6. Preserve package-scoped supply-chain evidence

Cargo deny and audit run over the root product lock. This is at least as
enforcing for known packages and advisories because the root lock contains
the union resolved for all product members.

The shared lock does not by itself prove a privileged or static binary's
minimal selected closure. A repository-owned generator therefore derives and
commits two package-scoped closure inventories from locked metadata:

- broker production with default features disabled; and
- guest runner production with `real-libshpool`.

Each inventory binds the exact package closure, dependency edges, normalized
sources and checksums, and resolved features for its context. Policy checks
also reject the package-specific forbidden dependency classes, including the
guest runner's existing dynamic, PAM, and systemd exclusions. Check mode
regenerates from the authoritative root manifest and lock and fails on missing,
extra, or changed packages, sources, edges, features, or forbidden classes.
The generator output is evidence, not dependency authority.

Independent broker and guest lock-update cadence is lost. This is accepted.
The prior separation controlled binary size; it was not a required trust
boundary. The package-scoped inventories preserve independent drift review
without pretending the root lock is smaller than it is.

## Spec 003 consequence

The current Spec 003 four-hub plan conflicts with this decision. After this
ADR merges, Spec 003 must be amended to the two-hub model and re-panelled
before implementation resumes. This ADR PR makes no Spec 003 or code edit.

The broker uses the ordinary product-hub update path. No broker-specific repin
lifecycle ADR or pending broker-repin mechanism is required.

## Consequences

- Product packages have one shared resolution and update policy.
- The aggregate lock is larger: the spike grew it from 544 to 601 packages.
- Targeted builds, not nested workspaces, remain the binary-size control.
- Broker and guest closure changes remain separately reviewable through
  generator-derived inventories.
- Bazel follows the upstream workspace example and deletes the synthetic
  projection, broker hub, and their fidelity machinery from the plan.
- The walker retains an honestly separate tool boundary.

## Alternatives considered

### Keep the generated splice projection

Rejected. It duplicates Cargo workspace structure and requires extensive
fidelity checks only to preserve a workspace boundary with no required trust
property.

### Fork cargo-bazel

Rejected. A fork adds a trusted build-system maintenance surface while the
upstream-supported single-workspace path already represents the product.

### Use from_specs or a manually maintained third-party graph

Rejected. Either shape duplicates dependency declarations outside Cargo and
weakens the root manifest and lock as the single authority.

### Keep separate workspaces for binary size

Rejected. The measured targeted builds preserve the size property. A separate
workspace is not a substitute for selecting the package and features being
built.

### Merge the no-bash walker into the product workspace

Rejected for this decision. The walker has a real tooling boundary and no
cross-workspace path dependency. Merging it requires separate evidence and
justification.

## Invariants this decision creates

1. `packages/Cargo.lock` is the only authoritative product Cargo lock.
2. Broker and guest runner remain separate packages with empty default
   features and minimal explicit dependencies.
3. Release, Nix, broker test-context, and guest test-context invocations remain
   explicitly package and feature selected.
4. `crate_universe` has one product hub and one walker tool hub.
5. `packages/Cargo.guest.lock` is a generated static-guest input, not a hub.
6. First-party Bazel code is native, and broker's four configured contexts are
   checked independently.
7. Nix builds use root source and lock while existing static and ELF checks
   remain enforcing.
8. Root-lock deny and audit are paired with generated broker-production and
   guest-real-libshpool closure inventories.
9. No generated splice workspace, broker-specific hub, or broker-specific
   repin lifecycle may be reintroduced without a new measured decision.

## References

- [Cargo workspaces](https://doc.rust-lang.org/cargo/reference/workspaces.html)
- [`cargo build` package selection](https://doc.rust-lang.org/cargo/commands/cargo-build.html#package-selection)
- [Cargo feature unification](https://doc.rust-lang.org/cargo/reference/features.html#feature-unification)
- [`rules_rust` 0.73.0 Cargo workspace example](https://github.com/bazelbuild/rules_rust/tree/0.73.0/crate_universe/tests/integration/cargo_workspace)
- [`rules_rust` issues #1525](https://github.com/bazelbuild/rules_rust/issues/1525),
  [#1773](https://github.com/bazelbuild/rules_rust/issues/1773), and
  [#3571](https://github.com/bazelbuild/rules_rust/issues/3571)
- [ADR 0009](0009-rust-toolchain-msrv-and-supply-chain.md)
- [ADR 0052](0052-bazel-rust-build-and-test.md)
