# ADR 0054: A generated splice workspace for the privileged broker's Bazel dependency hub

- Status: Proposed
- Date: 2026-08-05
- Refines: [ADR 0052](0052-bazel-rust-build-and-test.md), only for the
  `broker` hub's generated splice input and graph-fidelity requirements.
- Authority: [ADR 0009](0009-rust-toolchain-msrv-and-supply-chain.md)
  decision items 3, 5, and 7 establish the pinned Cargo supply-chain
  baseline. ADR 0052 section 2 names the four authoritative hub/workspace
  locks and keeps Cargo authoritative. ADR 0052 section 6 intentionally
  applies deny, audit, and yanked-state coverage to only three of those locks.
- Scope: the committed generated broker splice workspace, its exact
  relationship to the standalone broker workspace, production and test
  library variants, and graph-fidelity validation.
- Non-scope: broker repin execution, writer serialization, locking,
  monitoring, process lifetime, bookkeeping, clean-worktree admission,
  scratch ownership, output publication, recovery, rc selection, diagnostics,
  cleanup, implementation, and Spec 003 execution beyond mechanically parking
  it at the unresolved broker lock.

## Context

ADR 0052 assigns one `crate_universe` hub to each Cargo workspace. The exact
hub and lock inventory is:

| Hub | Authoritative Cargo lock | Bazel-side lock |
| --- | --- | --- |
| `main` | `packages/Cargo.lock` | `bazel/cargo/main.lock` |
| `broker` | `packages/d2b-priv-broker/Cargo.lock` | `bazel/cargo/broker.lock` |
| `guest` | `packages/d2b-guest-shell-runner/Cargo.lock` | `bazel/cargo/guest.lock` |
| `walker` | `tests/tools/no-bash-ast-walker/Cargo.lock` | `bazel/cargo/walker.lock` |

`packages/Cargo.guest.lock` is separate. It is a generated guest-workspace
and cache-key input, not an authoritative hub/workspace lock and not a fifth
hub.

The four-lock authority inventory is not the supply-chain carrier inventory.
ADR 0009 decision items 3 and 5 require pinned deny and audit coverage, and
item 7 binds reproducible vendoring to the authoritative Cargo lock. ADR 0052
section 6 carries that policy for exactly `main`, `broker`, and `guest`.
`walker` has no deny, audit, or yanked-state carrier. This ADR changes neither
inventory.

The broker is a standalone Cargo workspace. It path-depends on five packages
that belong to the separate main workspace: `d2b-contracts`, `d2b-core`,
`d2b-host`, `d2b-realm-core`, and `d2b-realm-provider`.

Measured `crate_universe` 0.73.0 cannot splice that shape. Supplying only the
broker manifest relocates it without its path dependencies. Supplying the
broker manifest and the five path manifests together is refused because they
belong to different workspaces. A generated workspace containing the broker
member and its realized first-party path closure does splice successfully.

That result settles the hub input, not lock regeneration. Spec 003 W0 is
parked at broker lock regeneration. The previous drafts coupled the generated
workspace to an unaccepted repin lifecycle. Panel review showed that repin
serialization, lifetime, publication, failure, and cleanup form a separate
architectural contract.

## Decision

### 1. Preserve Cargo authority and the standalone broker workspace

`packages/d2b-priv-broker/Cargo.toml` remains a standalone workspace and
`packages/d2b-priv-broker/Cargo.lock` remains its authoritative lock. Bazel
does not become a dependency declaration surface.

The `broker` hub uses:

- `packages/d2b-priv-broker/Cargo.lock` as `cargo_lockfile`;
- `bazel/cargo/broker.lock` as its Bazel-side lock;
- `skip_cargo_lockfile_overwrite = True`; and
- the committed generated workspace under
  `bazel/cargo/broker-workspace/` as its manifest set.

The other hubs retain the exact ADR 0052 authority and lock inventory.
ADR 0052's cache-key inventory continues to bind all four authoritative
hub/workspace locks, all four Bazel-side locks, and the separate
`packages/Cargo.guest.lock`.

### 2. Give the generated workspace one writer and a read-only check

`cargo xtask gen-bazel` is the sole writer of the committed
`bazel/cargo/broker-workspace/**` tree. No repin command, Bazel invocation,
test, Make target, or workflow may generate, repair, or publish that tree.
`bazel/cargo/broker.lock` is not generator-owned.

`cargo xtask gen-bazel --check` is strictly read-only. It computes the
expected bytes, output census, semantic projections, and declaration ledger,
then refuses any missing, extra, byte-different, or semantically different
generated output. It never repairs drift.

Both passing and failing check runs leave the complete repository state
unchanged. The before and after identity covers `HEAD`, the index, tracked
worktree objects, ordinary untracked objects, and ignored objects, including
object type, mode, bytes, and symlink target. Check mode creates no lock,
temporary file, scratch path, cache entry, or bookkeeping state.

The real-command test redirects the closed environment-root set `HOME`,
`TMPDIR`, `TMP`, `TEMP`, `XDG_CACHE_HOME`, `CARGO_HOME`, `RUSTUP_HOME`,
`CARGO_TARGET_DIR`, `BAZELISK_HOME`, and `TEST_TMPDIR` to empty observed
directories. It snapshots those directories and the repository before and
after both a passing and a failing run. An injected filesystem and process
observer separately refuses any attempted mutation, including a path outside
those roots. A repository-only snapshot is not evidence of read-only behavior.

### 3. Generate an exact resolution witness

The generated workspace contains:

- one workspace-root manifest;
- one generated package manifest and inert source target for the broker member
  and every package in its realized first-party path closure;
- a `Cargo.lock` byte-identical to the authoritative broker lock; and
- a `BUILD.bazel` exporting the exact manifest and lock-mirror census.

The inert sources are resolution inputs only. They are never first-party
compilation inputs.

Validation derives four independently observed projections:

- **A - authoritative:** locked offline Cargo metadata over the committed
  standalone broker workspace and authoritative broker lock.
- **W - witness:** locked offline Cargo metadata over the generated workspace,
  after removing only the synthetic root and declared inert targets and
  mapping generated paths through a closed path map.
- **L - Bazel lock:** the actual parsed committed
  `bazel/cargo/broker.lock`.
- **R - repository:** the actual materialized `@broker` repository, obtained
  from real Bazel query and repository contents rather than an expected map.

For every realized package, A and W record:

- package identity;
- normalized path identity, or exact source kind and normalized source;
- registry checksum;
- canonical git URL, precise revision, and checksum when present;
- exact resolved feature set; and
- every applicable target identity, kind, normalized source, testability,
  doctest setting, and required-feature set.

For every realized dependency edge, A and W record:

- source and destination package identities;
- dependency kind and normalized target condition;
- manifest alias, including an explicit no-alias value;
- requested edge features and default-feature semantics; and
- the realized feature contribution of that edge.

A and W are symmetrically equal. Comparisons do not collapse packages to
name/version pairs, features to package aggregates, or edges to unordered
destination sets.

L and R each declare an exact field-capability map. Each is symmetrically equal
to A projected onto every field it can represent, and L and R are
symmetrically equal over their shared representable fields. A field present
in L or R is never discarded merely because the other representation lacks
it. Missing, extra, or empty package and edge sets fail, including when two
incorrect projections are both empty.

Locked offline metadata over the generated root must succeed. The lock mirror
is checked separately for byte equality so semantic equality cannot conceal
lock drift.

Target and source expectations are read independently from the authoritative
Cargo manifests and locked offline metadata. They never come from generated
manifests, generated `BUILD.bazel` files, or a generator-emitted expected map.
The check refuses a generated manifest that omits or adds a target and an inert
target whose source is substituted with another inert source, even when the
generated output is internally self-consistent.

### 4. Account exactly for declarations outside realized metadata

Declarations that Cargo metadata does not expose as realized fields enter one
exact ledger. Its closed classes are:

- `inactive-optional`;
- `excluded-nonmember-dev`;
- `unrepresented-target`; and
- `synthesized-inert-target`.

Every omitted declaration appears exactly once with its class-specific reason.
No realized declaration also appears in the ledger. A genuinely empty class
is accepted only when the authoritative declaration census proves it empty.
Every inert target matches the closed template and package census and is
absent from first-party compilation inputs.

### 5. Separate production features from broker-test features

Authoritative locked metadata and `cargo tree` over normal/build edges versus
normal/build/dev edges show two differing shared feature vectors:

| Package | Production | Broker test |
| --- | --- | --- |
| `d2b-core` | no features | `test-support` |
| `d2b-host` | `default` (empty) | `default,fake-backends` |

Those packages have exactly these distinct library targets:

- `d2b-core-broker-production`;
- `d2b-core-broker-test`;
- `d2b-host-broker-production`; and
- `d2b-host-broker-test`.

The other shared packages have equal production and test feature vectors and
remain single broker variants: `d2b-contracts-broker`,
`d2b-realm-core-broker`, and `d2b-realm-provider-broker`. No equal context may
be duplicated in anticipation of a future difference.

All seven are library-only. They expose no `rust_test`, doctest, binary,
example, benchmark, or other test target. Production broker member targets
reach only the production variants; broker member test and doctest targets
consume the two test variants. A production closure reaching `test-support` or
`fake-backends`, or a broker test bypassing the matching test variant, fails.
Tests for shared packages remain solely on their ordinary main variants.

### 6. Require exact first-party and spoke graph fidelity

`F_expected` is independently derived from authoritative Cargo manifests and
locked metadata plus the closed hand-written-fragment registry. It includes
target identity, kind, normalized source, source package, compilation context,
feature vector, and hub owner. It does not read generator output.
`F_actual` comes from real Bazel query.

`B_production_expected` is the five shared production/equal library variants
plus every non-test broker member target derived from authoritative metadata.
The measured current census is seven: five shared libraries, the broker
library, and the broker binary. `B_test_expected` is the two test-only shared
variants plus every broker member unit, integration, and doctest target derived
from the same authority. The measured current census is eighteen: two shared
test libraries, two member unit-test harnesses, thirteen integration tests,
and one doctest. Counts are observations; the derivation is normative.

Within `//packages/d2b-priv-broker`, the member labels are exactly
`:broker-production-lib`, `:broker-production-bin`, `:broker-test-lib`,
`:broker-test-bin`, `:broker-doctest-lib`, and
`:broker-test-<cargo-target>` for each authoritative integration target, with
Cargo `_` normalized to Bazel `-`. The current integration suffixes are
`bridge-lifecycle`, `broker-export-audit`, `broker-protocol-compatibility`,
`broker-socket-acl`, `bundle-tampered-broker`, `kernel-surface`,
`persistent-tap-lifecycle`, `pidfd-handoff-scm-rights`,
`pidfd-real-spawner`, `security-key-broker`, `socket-activation`,
`w12-fd-passing-response`, and `w15-install-migrate`.

`B_expected` is their disjoint union. `B_actual`,
`B_production_actual`, and `B_test_actual` are queried sets owned by the broker
hub and classified from actual features, edges, kinds, and sources.

`M_expected` is exactly `F_expected - B_expected`, and `M_actual` is exactly
`F_actual - B_actual`. M is never separately curated.

Before checking edges:

- expected and actual F, B, and M are symmetrically equal and nonempty;
- expected and actual B production and test partitions are independently
  symmetric, nonempty, and disjoint;
- `B intersection M` is empty; and
- `B union M == F`.

For first-party `deps` and `proc_macro_deps`, the closure reachable from B
stays in B and the closure reachable from M stays in M. For direct third-party
spokes, B uses only the actual `@broker//` repository. Every M target uses its
independently derived hub owner and never `@broker//` unless it belongs to B.
The actual `@broker` identity is queried and materialized; an expected label
map is not accepted as proof.

## Required validation

The implementation plan must assign enforcing carriers for these independent
planted mutations:

1. Four hub tokens, four authoritative hub/workspace locks, four Bazel-side
   locks, the separate `packages/Cargo.guest.lock`, and the three-lock
   supply-chain carrier inventory each fail on missing or extra entries.
2. Passing and failing `gen-bazel --check` runs preserve the full `HEAD`,
   index, tracked, ordinary-untracked, and ignored state described above.
   Missing, extra, byte-different, and semantic generated outputs each fail
   without state creation. Each controlled external root is also unchanged,
   and the injected observer sees no mutation outside it.
3. A, W, L, and R each have independent missing, extra, and empty package and
   edge mutations.
4. Independent package mutations cover identity, source kind, source identity,
   checksum, precise git revision, resolved features, target identity, target
   kind, target source, manifest target omission, manifest target addition,
   inert-source substitution, and locked-offline metadata failure.
5. Independent edge mutations cover destination, dependency kind, condition,
   alias, requested features, default-feature semantics, and realized
   edge-feature contribution.
6. Every declaration-ledger class independently covers missing, extra,
   duplicate, wrong-reason, and incorrectly empty rows. An authoritative-empty
   fixture proves the permitted empty case.
7. The lock mirror independently fails byte drift. Actual `broker.lock` and
   actual `@broker` each have an identity mutation and independent
   source/checksum/revision/feature/target/alias/edge mutations while the
   witness and the other actual representation remain unchanged.
8. F, B, B-production, B-test, and M have independent missing, extra, and
   empty mutations that fail before edge isolation. Independent mutations swap
   each production/test feature vector and target name. Independent planted
   edges cover both first-party cross-partition directions, each broker
   production/test direction, B bound to `@main//`, and an ordinary M target
   bound to `@broker//`.
9. Real Bazel query and representative builds reproduce the target,
   repository, F, B, M, and spoke censuses from the committed witness.
10. Three separate Layer-1 carriers reject: removing
    `skip_cargo_lockfile_overwrite = True`; granting any second writer the
    broker witness; and letting the pending broker-repin arm spawn its child,
    vary its exact output, or attempt any write. These carriers do not share an
    expected map or one mutation dispatcher.

Each mutation changes one dimension and fails exactly once at its named guard,
not at a shared parser or an earlier unrelated guard. Generated expected maps
alone do not prove actual lock, repository, query, build, or spoke identity.

## Explicit non-decision and implementation block

ADR 0054 does not authorize, define, refine, or implement
`cargo xtask bazel-repin --hub broker`. Until a separate accepted ADR defines
its writer serialization, process lifetime, output publication, recovery,
diagnostics, and cleanup contract, that command must perform no repin work and
must return nonzero with empty stdout and exactly these two LF-terminated
stderr lines:

```text
broker-repin-architecture-pending
broker repin is unavailable; no local recovery command exists; prerequisite is an accepted repin-lifecycle ADR plus amended/re-panelled Spec 003.
```

The refusal happens before generic repin dispatch. It spawns no Bazel child
and creates, removes, or changes no path. A sentinel child and a write-refusing
filesystem prove both properties. The generic repin implementation accepts
only `main`, `guest`, and `walker`, whose behavior remains exactly as ADR 0052
defines it.

This record selects no lock, monitor, process hierarchy, worktree admission
rule, bookkeeping location, scratch layout, rc policy, publication mechanism,
recovery command, diagnostic envelope, or cleanup behavior for broker repin.
None may be inferred from an earlier ADR 0054 draft.

Spec 003 W0 remains parked at broker lock regeneration. After ADR 0054 merges,
the required order is:

1. accept a separate broker-repin ADR;
2. amend the Spec 003 plan, tasks, contracts, ownership map, and validation;
3. re-panel the amended Spec 003 artifacts; and
4. only then resume implementation.

An initial proof may use a measured scratch or prototype broker lock. No
committed implementation or contributor command may cite that proof, or this
ADR alone, as completing W0.

## Consequences

- This ADR does not unblock Spec 003 W0.
- It removes the broker hub-input ambiguity and isolates broker repin as the
  one remaining architectural decision.
- Generated workspace drift and actual Bazel graph drift fail closed against
  Cargo authority.
- Equal shared contexts compile once for the broker resolve; only `d2b-core`
  and `d2b-host` split production from broker-test features. Shared package
  tests remain single-owned.
- Broker lock regeneration remains unavailable until its separate ADR is
  accepted and the Spec 003 artifacts are amended and re-panelled.

## Alternatives considered

### Merge the broker into the main Cargo workspace

Rejected. It removes the broker's independent lock and expands the privileged
dependency closure for build-tool convenience.

### Bind broker targets to main-hub first-party targets

Rejected. It silently mixes dependency resolves in the privileged binary.

### Patch or upgrade `rules_rust`

Rejected for this decision. The measured 0.73.0 release does not accept the
cross-workspace splice, and a local splicer patch is a larger trusted
maintenance surface than the generated witness. A future compatible upstream
release can replace the witness through a new measured decision.

### Put the decision only in Spec 003

Rejected. The generated hub input and authority boundary are repository-wide
architecture that outlives one implementation plan. Spec 003 must consume the
accepted decision rather than define it.

### Couple broker repin lifecycle into this ADR

Rejected. Panel review demonstrated that serialization, process lifetime,
publication, recovery, diagnostics, and cleanup are a separate contract.
Keeping them here would make the settled graph decision depend on an unsettled
execution protocol.

## Invariants this decision creates

1. Cargo manifests and the four ADR 0052 hub/workspace locks remain
   authoritative; `packages/Cargo.guest.lock` is separate.
2. ADR 0052 section 6's carrier set remains `main`, `broker`, and `guest`.
3. `gen-bazel` alone writes the witness; `gen-bazel --check` is read-only.
4. The witness models the standalone broker workspace's realized path closure
   and exact package, source, revision, checksum, feature, target, alias, and
   edge semantics.
5. The ledger is exact and the lock mirror is byte-identical.
6. Library-only production/test broker variants and actual F, B production,
   B test, M, `@broker`, and spokes match their authoritative derivation.
7. ADR 0054 authorizes no broker repin execution or lifecycle contract.
8. Broker repin remains a no-child, no-write exact refusal with
   `broker-repin-architecture-pending` until a separate accepted ADR replaces
   that block.
9. Main, guest, and walker repin behavior remains exactly ADR 0052 behavior.
10. Spec 003 W0 remains parked pending that ADR, amendment, and a new panel.

## References

- [ADR 0009](0009-rust-toolchain-msrv-and-supply-chain.md), items 3, 5, and 7
- [ADR 0052](0052-bazel-rust-build-and-test.md), sections 2, 3, 4, 6, and 10
- `rules_rust` 0.73.0 `crate_universe` splicing behavior
