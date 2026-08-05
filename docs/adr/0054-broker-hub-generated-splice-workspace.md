# ADR 0054: A generated splice workspace for the privileged broker's Bazel dependency hub

- Status: Proposed
- Date: 2026-08-05
- Refines: [ADR 0052](0052-bazel-rust-build-and-test.md), only for the
  `broker` hub's generated splice input and graph-fidelity requirements.
- Authority: [ADR 0009](0009-rust-toolchain-msrv-and-supply-chain.md)
  decision items 3, 5, and 7 retain the pinned Cargo supply-chain baseline.
  ADR 0052 sections 2, 4, and 6 retain Cargo authority, generated Bazel
  ownership, and the existing supply-chain carrier set.
- Scope: the committed generated broker splice workspace, its exact source and
  graph projections, broker compilation contexts, and fidelity validation.
- Non-scope: broker repin lifecycle or commands, Spec 003 admission or
  qualification, and any parser, operator recovery mechanism, lock writer,
  publication protocol, or process-lifetime contract.

## Context

ADR 0052 assigns one `crate_universe` hub to each Cargo workspace. The exact
authority inventory has four independent hub/workspace locks:

| Hub | Authoritative Cargo lock | Bazel-side lock |
| --- | --- | --- |
| `main` | `packages/Cargo.lock` | `bazel/cargo/main.lock` |
| `broker` | `packages/d2b-priv-broker/Cargo.lock` | `bazel/cargo/broker.lock` |
| `guest` | `packages/d2b-guest-shell-runner/Cargo.lock` | `bazel/cargo/guest.lock` |
| `walker` | `tests/tools/no-bash-ast-walker/Cargo.lock` | `bazel/cargo/walker.lock` |

`packages/Cargo.guest.lock` is separate. It is a generated guest-workspace and
cache-key input, not an authoritative hub/workspace lock and not a fifth hub.
ADR 0052 section 6 intentionally applies deny, audit, and yanked-state
coverage to only `main`, `broker`, and `guest`. This decision changes neither
inventory.

The broker is a standalone Cargo workspace. Its realized first-party path
closure includes `d2b-contracts`, `d2b-core`, `d2b-host`, `d2b-realm-core`,
and `d2b-realm-provider` from the separate main workspace.

Measured `crate_universe` 0.73.0 cannot splice that shape. Supplying only the
broker manifest relocates it without its path dependencies. Supplying the
broker manifest and path manifests together is refused because they belong to
different workspaces. A generated workspace containing the broker and its
realized first-party path closure splices successfully.

## Decision

### 1. Preserve Cargo authority and independent locks

`packages/d2b-priv-broker/Cargo.toml` remains a standalone workspace and
`packages/d2b-priv-broker/Cargo.lock` remains its authoritative dependency
lock.

The `broker` hub uses:

- `packages/d2b-priv-broker/Cargo.lock` as `cargo_lockfile`;
- `bazel/cargo/broker.lock` as its independently committed Bazel-side lock;
- `skip_cargo_lockfile_overwrite = True`; and
- `bazel/cargo/broker-workspace/` as its generated manifest set.

The generated workspace contains a byte-identical mirror of the authoritative
broker lock for locked offline metadata. That mirror does not replace either
independent lock. The generator never writes `bazel/cargo/broker.lock`.

The other three hubs retain their ADR 0052 authority and lock pairs. Cache-key
inventory continues to bind all four authoritative locks, all four Bazel-side
locks, and the separate `packages/Cargo.guest.lock`.

### 2. Generate one exact splice witness

The committed `bazel/cargo/broker-workspace/` tree contains exactly:

- one workspace-root manifest;
- one generated package manifest and inert source target for the broker and
  every package in its realized first-party path closure;
- the byte-identical broker lock mirror; and
- one `BUILD.bazel` exporting the exact manifest and lock-mirror census.

Inert sources are resolution witnesses only. No generated inert source is a
first-party compilation input.

Validation derives four independently observed projections:

- **A - authority:** locked offline Cargo metadata for the standalone broker
  workspace and authoritative broker lock.
- **W - witness:** locked offline Cargo metadata for the generated workspace,
  after removing only declared synthetic roots and inert targets and mapping
  generated paths through a closed path map.
- **L - lock:** the actual parsed committed `bazel/cargo/broker.lock`.
- **R - repository:** the actual materialized `@broker` repository, observed
  through real Bazel query and repository contents.

For every realized package, A and W retain:

- package identity;
- normalized path identity, or exact source kind and normalized source;
- registry checksum;
- canonical git URL, precise revision, and checksum when present;
- exact resolved feature set; and
- each target's identity, kind, normalized source, testability, doctest
  setting, and required-feature set.

For every realized dependency edge, A and W retain:

- source and destination package identities;
- dependency kind and normalized target condition;
- manifest alias, including explicit no-alias;
- requested edge features and default-feature semantics; and
- the realized feature contribution of that edge.

A and W are symmetrically equal. Comparisons do not collapse packages to
name/version pairs, features to package aggregates, or edges to destination
sets.

L and R each declare an exact field-capability map. Each is symmetrically equal
to A projected onto every field it can represent, and L and R are
symmetrically equal over their shared fields. A field present in either is not
discarded because the other representation lacks it. Missing, extra, or empty
package and edge sets fail.

Target and source expectations come independently from authoritative Cargo
manifests and locked metadata. They never come from generated manifests,
generated BUILD files, or a generator-emitted expected map.

Declarations absent from realized metadata enter one exact ledger with the
closed classes `inactive-optional`, `excluded-nonmember-dev`,
`unrepresented-target`, and `synthesized-inert-target`. Every omitted
declaration appears exactly once with its reason, no realized declaration
appears there, and an empty class is accepted only when the authoritative
declaration census proves it empty.

### 3. Give the witness one writer and a read-only check

`cargo xtask gen-bazel` is the sole writer of
`bazel/cargo/broker-workspace/**`. No Bazel invocation, test, Make target,
workflow, or other generator may create, repair, or publish that tree.

The `gen-bazel --check` mode of an already-built xtask process is read-only. It
derives the expected bytes, output census, projections, and declaration ledger,
then refuses missing, extra, byte-different, or semantically different output.
It never repairs drift or creates repository, scratch, lock, temporary, cache,
or bookkeeping state. The public `cargo xtask gen-bazel --check` spelling is
the contributor interface; Cargo bootstrap before the built process starts is
outside the process-level read-only proof.

Passing and failing built-process checks preserve `HEAD`, the index, tracked
files, ordinary untracked files, ignored files, and the controlled Cargo,
Bazel, XDG, home, and temporary roots used by the check. Validation observes
mutation attempts as well as final state, so create-then-delete is not accepted
as read-only.

### 4. Split complete compilation contexts, not feature vectors

A compilation context consists of package and target, toolchain, compile mode,
resolved features, and every configured outgoing dependency edge recursively,
including the destination context. Direct feature equality is insufficient.

Locked Cargo unit graphs establish these shared-library variants:

| Package | Production variant | Test variant |
| --- | --- | --- |
| `d2b-core` | no features | `test-support` |
| `d2b-contracts` | no features, production `d2b-core` | no features, test `d2b-core` |
| `d2b-host` | `default`, production core/contracts | `default,fake-backends`, test core/contracts |
| `d2b-realm-core` | shared | shared |
| `d2b-realm-provider` | shared | shared |

The deterministic library labels are
`d2b-{core,contracts,host}-broker-{production,test}`,
`d2b-realm-core-broker-shared`, and
`d2b-realm-provider-broker-shared`. They are library-only. Shared-package
tests remain owned by ordinary main-workspace variants. A variant may be
shared only when its complete context is equal.

The broker has four complete contexts:

| Context | Broker features | Configured first-party targets | Test cases |
| --- | --- | ---: | ---: |
| `production` | `default` (empty) | 7 | not a test carrier |
| `default` | `default` (empty) | 23 | 557 |
| `layer1-bootstrap` | `default,layer1-bootstrap` | 23 | 492 |
| `fake-backends` | `default,fake-backends` | 23 | 559 |

Production owns `:broker-production-{lib,bin}`. Each test carrier owns its
carrier-local broker library and binary, library and binary unit harnesses,
library doctest, and these thirteen integration targets:

- `bridge-lifecycle`
- `broker-export-audit`
- `broker-protocol-compatibility`
- `broker-socket-acl`
- `bundle-tampered-broker`
- `kernel-surface`
- `persistent-tap-lifecycle`
- `pidfd-handoff-scm-rights`
- `pidfd-real-spawner`
- `security-key-broker`
- `socket-activation`
- `w12-fd-passing-response`
- `w15-install-migrate`

The exact Cargo `--list` case-name set is normative. A zero-case harness,
feature-disabled integration target, or doctest target remains in the target
census.

### 5. Require exact B, M, and spoke fidelity

`F_expected` is the full first-party target projection derived independently
from authoritative Cargo manifests, locked unit graphs, exact Cargo target
and case listings, and the closed hand-written-fragment registry.
`F_actual` comes from real Bazel query, configured cquery, and a provider
aspect over actual configured targets.

The independently derived broker projections are:

- `B_prod_expected`, with 7 configured first-party targets;
- `B_default_expected`, with 23;
- `B_layer1_expected`, with 23; and
- `B_fake_expected`, with 23.

The three test contexts share exactly the three test library variants and two
realm-shared variants. Production shares only the two realm variants. No other
cross-context overlap is allowed. Their unique union `B_expected` contains 64
configured first-party targets.

`B_actual` and its four context projections come from real query, configured
cquery, and the provider aspect. Plain query is insufficient because it cannot
observe configured features or dependency destinations.

`M_expected` is exactly `F_expected - B_expected`, and `M_actual` is exactly
`F_actual - B_actual`. M is never curated separately.

Before edge checks, expected and actual F, B, M, and each B context are
symmetrically equal and nonempty; the overlap ledger is exact; `B` and `M` are
disjoint; and `B union M == F`.

Production reaches only production or shared libraries. Each test carrier
reaches its own broker context and the test or shared libraries, never another
carrier's broker targets. First-party dependency closure from B stays in B and
closure from M stays in M. B's direct third-party spokes use the actual
`@broker//` repository. Each M target uses its independently derived hub and
may use `@broker//` only if it belongs to B.

The test must query and materialize the actual `@broker` identity. An expected
label map, generated repository model, or self-consistent fixture is not proof
of the repository Bazel used.

## Required validation

Enforcing validation must include independent planted mutations proving:

1. The four hub tokens, four authoritative locks, four Bazel-side locks,
   separate `packages/Cargo.guest.lock`, and three-lock supply-chain carrier
   set each reject missing or extra entries.
2. Passing and failing `gen-bazel --check` runs are read-only and reject
   missing, extra, byte-different, and semantic witness drift.
3. A, W, L, and R independently reject missing, extra, empty, source,
   checksum, revision, feature, target, alias, and dependency-edge drift.
4. The declaration ledger rejects a missing, extra, duplicate, wrong-reason,
   or falsely empty row, while an authoritative-empty fixture passes.
5. The lock mirror rejects byte drift independently from semantic equality.
6. Each broker context rejects target, feature, configured-edge, and exact
   case-set drift. Cross-context mutations reject production-to-test,
   test-to-production, and carrier-to-carrier leakage.
7. F, B, M, every B context, their overlap ledger, and their closures reject
   missing, extra, empty, misnamed, or wrong-hub targets.
8. Independent mutations of the actual `broker.lock` and actual `@broker`
   repository fail while the witness and the other observed representation
   remain unchanged.
9. Real query, cquery, provider-aspect output, and representative builds
   reproduce the committed target, repository, context, B, M, and spoke
   projections.
10. Separate guards reject removal of `skip_cargo_lockfile_overwrite = True`
    and any second writer for the generated witness.

Each mutation changes one dimension and reaches its named guard. Generated
expected maps alone do not prove actual lock, repository, query, build, or
spoke identity.

## Explicit non-decision

ADR 0054 does not authorize broker repin. It defines no broker repin parser,
command implementation, operator entry point, writer serialization, process
lifetime, scratch ownership, publication, recovery, diagnostics, cleanup, or
qualification lineage. None may be inferred from this record or from an
earlier draft.

Spec 003 remains blocked at broker lock regeneration. Resuming it requires,
in order, a separate accepted ADR for the broker repin lifecycle, a later
amendment of the Spec 003 artifacts, and renewed plan-panel approval of that
amendment. ADR 0054 alone is not admission evidence for Spec 003 work.

## Consequences

- The broker hub has a spliceable, reviewable witness without merging the
  privileged workspace into the main Cargo workspace.
- Actual Bazel targets and the materialized `@broker` repository, rather than
  generated expectations, prove B, M, and spoke fidelity.

## Alternatives considered

### Merge the broker into the main Cargo workspace

Rejected. It removes the broker's independent lock and expands the privileged
dependency closure for build-tool convenience.

### Bind broker targets to main-hub first-party targets

Rejected. It silently mixes dependency resolves in the privileged binary.

### Patch or upgrade `rules_rust`

Rejected for this decision. The measured 0.73.0 release does not accept the
cross-workspace splice, and a local splicer patch is a larger trusted surface
than the generated witness. A future compatible release can replace the
witness through a new measured decision.

### Put the decision only in Spec 003

Rejected. The broker hub input and authority boundary outlive one
implementation plan. Spec 003 must consume an accepted decision, not define
repository-wide architecture.

### Include broker repin lifecycle

Rejected. Writer serialization, process lifetime, publication, recovery,
diagnostics, and cleanup are a separate design whose unresolved details must
not block the settled graph decision.

## Invariants this decision creates

1. Cargo manifests and the four ADR 0052 hub/workspace locks remain
   authoritative; `packages/Cargo.guest.lock` remains separate.
2. The broker Cargo lock, broker Bazel-side lock, and generated lock mirror
   have distinct ownership; only the mirror is generator-owned.
3. `gen-bazel` alone writes the witness; `gen-bazel --check` is read-only.
4. A, W, L, and actual R retain every representable package, source, target,
   feature, alias, and edge field and compare symmetrically.
5. Complete contexts, not direct features, control library variant reuse.
6. Actual F, B, M, all four B contexts, their overlap ledger, `@broker`, case
   sets, and spokes match independent Cargo-derived expectations.
7. ADR 0054 authorizes no broker repin lifecycle or operator mechanism.
8. Spec 003 remains blocked pending a separate ADR, later amendment, and
   renewed plan-panel approval.

## References

- [ADR 0009](0009-rust-toolchain-msrv-and-supply-chain.md), items 3, 5, and 7
- [ADR 0052](0052-bazel-rust-build-and-test.md), sections 2, 4, and 6
- `rules_rust` 0.73.0 `crate_universe` splicing behavior
