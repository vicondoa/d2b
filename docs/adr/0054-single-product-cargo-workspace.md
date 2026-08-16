# ADR 0054: Single product Cargo workspace

- Status: Accepted
- Date: 2026-08-05
- Scope: Cargo workspace membership and locks for product packages, Cargo and
  Nix package selection, selected production closures, and package-scoped
  supply-chain enforcement.
- Non-scope: changing Rust behavior, moving the no-bash walker, or weakening
  static, ELF, or policy checks.
- Threat-model non-goal: contributor mutation commands run in a trusted local
  operator shell. They are not a credential or sandbox boundary and are
  unreachable from workflows and Make targets.

## Context

The product packages share one repository-root Cargo workspace and one
authoritative `Cargo.lock`. The privileged broker and static guest runner are
members of that workspace, while the no-bash AST walker remains a separate
tooling workspace with its own manifest and lock.

A shared lock resolves the union of workspace dependencies. That union is not
itself a build dependency or an approval to ship a package. The security
boundaries are package-selected Cargo and Nix builds and enforcing policy over
each selected production closure. An unrelated package appearing only in the
shared lock is harmless; a new edge that connects it to a privileged or static
selected closure is not.

The no-bash walker has a different boundary. It is closed gate plumbing under
`tests/tools/`, outside the product package tree, and has no path dependency
into the product workspace. It therefore remains separate.

## Evidence

### Cargo and Nix integration

The integration work added the broker and guest runner to the root workspace,
removed their nested workspace tables and locks, and regenerated the root lock
offline. Locked, offline selection showed no guest runner in the broker
production closure and no broker in the guest production closure. The
standalone and unified test censuses remained identical.

The required selected contexts are:

| Context | Selector |
| --- | --- |
| Main product | workspace selection excluding broker, guest runner, and contract-test execution surfaces where the gate requires it |
| Broker default | `-p d2b-priv-broker --no-default-features` |
| Broker Layer 1 | `-p d2b-priv-broker --no-default-features --features layer1-bootstrap` |
| Broker fake | `-p d2b-priv-broker --no-default-features --features fake-backends` |
| Guest runner | `-p d2b-guest-shell-runner --no-default-features --features real-libshpool` |

The broker streams remain serial because their tests mutate process-global
signal and reap state. Guest doctest and harness-free companions reuse the
root manifest and the same package and feature selectors. Fixture-dependent
contract tests remain in their enforcing fixture lane. Main clippy keeps the
contract crate's build targets unless a future decision explicitly changes
that coverage.

The broker and guest release artifacts retain their existing acceptance
criteria. The broker is a dynamically linked host binary. The guest runner is
a static PIE with no ELF interpreter or dynamic `NEEDED` entry. Selected
dependency, static ELF, deny, audit, flake evaluation, and policy checks remain
enforcing.

### Selected-closure policy

Locked, offline root metadata produces selected production closure metadata
and filtered audit locks. Production closure output covers the selected normal
and build dependencies. Policy output adds the root development closure needed
for package deny and audit. A disconnected package canary does not affect a
context until a selected edge connects it; once connected, the package policy
must reject any forbidden license, source, ban, or advisory.

The package generator derives the following contexts for both supported Linux
systems:

| System | Target | Contexts |
| --- | --- | --- |
| `x86_64-linux` | `x86_64-unknown-linux-gnu` | `main-product`, `broker-production`, `broker-default-tests`, `broker-layer1-bootstrap-tests`, `broker-fake-backends-tests` |
| `x86_64-linux` | `x86_64-unknown-linux-musl` | `guest-shell-runner-static`, `guestd-static` |
| `aarch64-linux` | `aarch64-unknown-linux-gnu` | `main-product`, `broker-production`, `broker-default-tests`, `broker-layer1-bootstrap-tests`, `broker-fake-backends-tests` |
| `aarch64-linux` | `aarch64-unknown-linux-musl` | `guest-shell-runner-static`, `guestd-static` |

Each context is written below:

```text
packages/policy-inputs/<system>/<target>/<context>/
  production/{closure,metadata}.json
  production/Cargo.lock
  policy/{closure,metadata}.json
  policy/Cargo.lock
```

The generated records bind the selected root packages, system, target,
package identity, version, source, checksum, edge kind, cfg, and resolved
features. The production lock is the filtered audit input, not a Cargo
resolution input. The policy lock is likewise used only by the policy tools.

The generator's check is:

```text
cargo xtask gen-package-policy-inputs --check
```

It must reject stale outputs, missing contexts, extra files, mismatched
metadata, wrong edge kinds, missing normal or build edges, missing root dev
edges, forbidden production classes, stale approvals, forbidden licenses,
sources or bans, and unignored pinned advisories. Approval metadata is
context-scoped and must carry an owner, rationale, approval marker, and
expiry where the policy requires it. Global or cross-context ignores are not
allowed.

## Decision

### Contributor mutation workflow

Lock and policy-input regeneration are contributor-only mutations. They run
from a trusted local operator shell, not from a workflow or Make target. The
shell is not a credential or sandbox boundary. Its `HOME`, startup
configuration, functions, and other operator-controlled state are outside this
decision's security model.

From the repository root, enter the pinned development environment and run:

```text
nix develop
cargo generate-lockfile --offline
cargo metadata --locked --offline --format-version 1
cargo xtask gen-package-policy-inputs --check
```

Continuous integration and gates call approved Make targets in controlled
environments. Package-policy checks remain hermetic through vendored sources
and the pinned RustSec database.

### One authoritative product workspace and lock

The repository-root `Cargo.toml` includes `d2b-priv-broker` and
`d2b-guest-shell-runner`. Their manifests keep `default = []` and their
explicit dependencies. The guest manifest keeps the normal `libshpool`
dependency while the `real-libshpool` feature gates its production bridge.

The root `Cargo.lock` is the only authoritative product lock. Nested product
locks and workspace tables are not retained, and no forwarding lock is
created. Ordinary root and release builds use the root `target/` directory.
Broker serial feature streams may use explicit execution-only target
directories, but those directories are cache surfaces rather than workspace
roots.

### Explicit Cargo and Nix selection

The main Cargo gate uses the root workspace and its documented exclusions:

```text
cargo clippy --locked --workspace --all-targets \
  --exclude d2b-priv-broker --exclude d2b-guest-shell-runner -- -D warnings
cargo nextest run --locked --workspace \
  --exclude d2b-contract-tests \
  --exclude d2b-priv-broker --exclude d2b-guest-shell-runner
```

The broker lanes use three serial `cargo test` processes with
`--no-default-features`, selecting `layer1-bootstrap` and `fake-backends` in
their dedicated streams. The guest lane selects `real-libshpool`. Doctests,
harness-free targets, benches, fixture contracts, deny, audit, and static ELF
checks use the same root manifest and explicit package selectors.

The broker Nix derivation selects the broker package and binary with default
features disabled. The static guest derivation selects the guest runner,
binary, `real-libshpool`, and the musl production closure for the native
system. Nix must not silently switch to a standalone or root-unfiltered lock
for a selected package.

### Package-scoped selected-closure authority

For each system and target, the generator produces production and policy
inputs under the context paths above. Nix reads the exact native system and
target output. The production closure is the approval authority for binary
and static-dependency minimality. The filtered locks are audit-only inputs.
Cargo locked metadata remains the reachability source of truth.

The selected-closure checks prove, in order:

1. the selected root exists exactly once;
2. the closure and census are nonempty and complete;
3. the root package, target, system, cfg, features, and edge kinds match;
4. all required normal, build, and policy development edges are present;
5. no forbidden package, license, source, advisory, or cross-context edge is
   connected to the selected closure; and
6. generated output and approval metadata are current.

An empty or incomplete census cannot satisfy an absence predicate. A package
appearing in the shared lock is not approval to reach it. Only the selected
closure and its checked policy inputs authorize the corresponding binary or
static artifact.

## Consequences

- Product packages share one dependency resolution and update event.
- Broker and guest lock-update cadence and visual isolation are lost; this is
  accepted because selected closure policy preserves the security boundary.
- The no-bash walker remains independently buildable and independently pinned.
- Broker default, Layer 1, and fake streams stay serial and target-isolated.
- Nix uses the exact native system, target, source, and selected closure without
  weakening static, ELF, deny, audit, or policy checks.
- The guest license findings remain narrow policy inputs that require reviewed
  resolution; they must not be hidden by a blanket policy expansion.

## Alternatives considered

### Keep separate product workspaces and locks

Rejected. They duplicate workspace and lock lifecycle. Measured package
selection and selected-closure controls preserve the needed isolation without
duplicating resolution authority.

### Merge the no-bash walker

Rejected. It has a real tooling boundary and no product path dependency.

### Make the shared lock the security boundary

Rejected. Lock membership is not reachability. Selected Cargo metadata,
production closures, and package policy must remain the approval boundary.

## Invariants this decision creates

1. `Cargo.lock` at repository root is the only authoritative product lock.
2. Broker and guest production are always package and feature selected.
3. The no-bash walker keeps its separate manifest, lock, and dependency policy.
4. Broker default, Layer 1, and fake lanes stay serial and target-isolated.
5. Nix reads the exact native system and target selected-closure outputs.
6. Production closure approval and filtered audit locks remain distinct inputs.
7. Every selected context proves one root and a nonempty exact census before
   absence or containment predicates.
8. Existing supply-chain, drift, flake, fixture, static ELF, and policy jobs
   remain enforcing for the contexts they own.
