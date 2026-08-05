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

Two closed types prevent that inventory from becoming execution authority:

```text
HubInventory = {main, broker, guest, walker}
RepinnableHub = {main, guest, walker}
```

`broker` is always the exact pending-refusal branch. It is never accepted by
generic repin dispatch.

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

The `gen-bazel --check` operation of an already-built xtask process is strictly
read-only. It computes the expected bytes, output census, semantic projections,
and declaration ledger, then refuses any missing, extra, byte-different, or
semantically different generated output. It never repairs drift. The public
`cargo xtask gen-bazel --check` spelling is the contributor interface, but any
Cargo bootstrap state created before the built process starts is outside the
process-level read-only proof.

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

### 5. Separate complete compilation contexts

A compilation context is not a direct feature vector. Its identity is the
package and target, toolchain, compile mode, resolved features, and every
configured outgoing dependency edge recursively, including the destination
context. Two variants are equal only when that complete graph is equal.

Locked Cargo unit graphs measured on 2026-08-05 establish these shared-library
contexts:

| Package | Production context | Test-carrier context |
| --- | --- | --- |
| `d2b-core` | no features | `test-support` |
| `d2b-contracts` | no features; edge to production `d2b-core` | no features; edge to test `d2b-core` |
| `d2b-host` | `default`; edges to production `d2b-core` and `d2b-contracts` | `default,fake-backends`; edges to test `d2b-core` and `d2b-contracts` |
| `d2b-realm-core` | shared | shared |
| `d2b-realm-provider` | shared | shared |

The deterministic library labels are
`d2b-{core,contracts,host}-broker-{production,test}`,
`d2b-realm-core-broker-shared`, and
`d2b-realm-provider-broker-shared`. All eight are library-only. Shared-package
tests remain solely on ordinary main-workspace variants. A shared label may be
reused only where its complete context is equal; direct feature equality alone
is insufficient.

Broker member contexts are independent:

| Context | Broker-local Cargo features | First-party configured targets | Enumerated cases |
| --- | --- | ---: | ---: |
| `production` | `default` (empty) | 7 | not a test carrier |
| `default` | `default` (empty) | 23 | 557 |
| `layer1-bootstrap` | `default,layer1-bootstrap` | 23 | 492 |
| `fake-backends` | `default,fake-backends` | 23 | 559 |

Each test carrier has five shared libraries, carrier-local broker library and
binary build targets, broker library and binary unit-test harnesses, thirteen
integration targets, and one library doctest target. Zero cases in the binary
unit harness, a feature-disabled integration target, or the doctest target are
retained in the target census rather than erased. The case census is the exact
Cargo `--list` name set, not only the measured count.

Under `//packages/d2b-priv-broker`, production labels are
`:broker-production-{lib,bin}`. For each test carrier `<carrier>` in
`default`, `layer1-bootstrap`, and `fake-backends`, labels are
`:broker-<carrier>-{lib,bin}`,
`:broker-<carrier>-unit-{lib,bin}`,
`:broker-<carrier>-doctest-lib`, and
`:broker-<carrier>-test-<cargo-target>`, with Cargo `_` normalized to Bazel
`-`. The thirteen integration suffixes are `bridge-lifecycle`,
`broker-export-audit`, `broker-protocol-compatibility`, `broker-socket-acl`,
`bundle-tampered-broker`, `kernel-surface`, `persistent-tap-lifecycle`,
`pidfd-handoff-scm-rights`, `pidfd-real-spawner`, `security-key-broker`,
`socket-activation`, `w12-fd-passing-response`, and `w15-install-migrate`.

### 6. Require exact first-party and spoke graph fidelity

`F_expected` is independently derived from authoritative Cargo manifests and
locked metadata plus the closed hand-written-fragment registry. It includes
target identity, kind, normalized source, source package, compilation context,
feature vector, and hub owner. It does not read generator output.
`F_actual` comes from real Bazel query, configured cquery, and the same
provider aspect used for B's actual graph.

`B_prod_expected`, `B_default_expected`, `B_layer1_expected`, and
`B_fake_expected` are independently derived reachable configured-target sets.
Their measured censuses are 7, 23, 23, and 23. The three test sets reuse the
same three shared test variants and two realm-shared variants; production
reuses only the two realm-shared variants. No other cross-context overlap is
allowed. Their unique union `B_expected` currently contains 64 configured
first-party targets. Counts are observations; authoritative manifests, locked
unit graphs, and exact Cargo target and case listings are normative.

`B_actual` and its four context projections come from real Bazel `query`,
configured `cquery`, and an aspect over actual providers. Plain query alone is
not graph evidence because it cannot observe configured features or dependency
destinations.

`M_expected` is exactly `F_expected - B_expected`, and `M_actual` is exactly
`F_actual - B_actual`. M is never separately curated.

Before checking edges:

- expected and actual F, B, and M are symmetrically equal and nonempty;
- each expected and actual B context is independently symmetric and nonempty;
- the exact permitted context-overlap ledger above is symmetric;
- `B intersection M` is empty; and
- `B union M == F`.

Production reaches only production/shared libraries and no test-only feature.
Each test carrier reaches its exact broker-local context and the shared
test/shared libraries, never another carrier's broker member target. For
first-party `deps` and `proc_macro_deps`, the closure reachable from B stays in
B and the closure reachable from M stays in M. For direct third-party spokes,
B uses only the actual `@broker//` repository. Every M target uses its
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
8. F, B, M, and each of B-prod, B-default, B-layer1, and B-fake have
   independent missing, extra, empty, and misnamed mutations that fail before
   edge isolation. Each carrier independently mutates its broker-local feature,
   configured edge, target, and case census. Cross-context mutations cover
   production reaching each test variant; each test carrier reaching a
   production variant or another carrier's broker member; production/test
   `d2b-contracts` reaching the wrong `d2b-core`; production/test `d2b-host`
   reaching the wrong core or contracts context; B bound to `@main//`; and an
   ordinary M target bound to `@broker//`.
9. Real Bazel query, cquery, provider-aspect output, and representative builds
   reproduce the target, repository, F, B, M, and spoke censuses from the
   committed witness.
10. Three separate Layer-1 carriers reject: removing
    `skip_cargo_lockfile_overwrite = True`; granting any second writer the
    broker witness; and letting the pending built-xtask broker arm spawn any
    child, vary its exact result, or attempt any write. These carriers do not
    share an expected map or one mutation dispatcher.

Each mutation changes one dimension and fails exactly once at its named guard,
not at a shared parser or an earlier unrelated guard. Generated expected maps
alone do not prove actual lock, repository, query, build, or spoke identity.

## Explicit non-decision and implementation block

ADR 0054 does not authorize, define, refine, or implement broker repin. Until a
separate accepted ADR defines its writer serialization, process lifetime,
output publication, recovery, diagnostics, and cleanup contract, an
already-built `xtask` process invoked as `bazel-repin --hub broker` must return
nonzero with empty stdout and exactly these two LF-terminated stderr lines:

```text
broker-repin-architecture-pending
broker repin is unavailable; no local recovery command exists; prerequisite is an accepted repin-lifecycle ADR plus amended/re-panelled Spec 003.
```

The built process emits the result before generic repin dispatch, spawns no
child, and creates, removes, or changes no path. Tests execute that built
binary directly with a sentinel child and a write-refusing filesystem. The
public `cargo xtask bazel-repin --hub broker` spelling may produce Cargo
bootstrap output and Cargo cache or target state before the built process
starts; its aggregate stderr and filesystem effects are therefore not the
exact-result contract. The generic implementation accepts only
`RepinnableHub`, whose behavior remains exactly as ADR 0052 defines it.

This record selects no lock, monitor, process hierarchy, worktree admission
rule, bookkeeping location, scratch layout, rc policy, publication mechanism,
recovery command, diagnostic envelope, or cleanup behavior for broker repin.
None may be inferred from an earlier ADR 0054 draft.

Spec 003 W0 remains parked at broker lock regeneration. Admission reads exactly
one marked execution-status block from each of `plan.md`, `tasks.md`, and the
workspace contract. Only exactly three `READY` blocks with the closed key set
and byte-identical values admit T021. The four non-status literals are
immutable; only status may change under a future accepted ADR, amended Spec
003, and renewed unanimous plan panel. Admission opens the repository root
once, resolves each fixed path component descriptor-relative without following
links, opens and verifies each regular final file once, and reads bytes from
that same descriptor. A missing, duplicate, reversed, nested, unknown,
misplaced, NUL-bearing, invalid-UTF-8, symlinked, nonregular,
component-replaced, or disagreeing input refuses before T021, any child, or any
write. A fake filesystem fixture replaces a component and final file between
open and check/read to prove there is no check-open or check-read path race.
After ADR 0054 merges, the required order is:

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
- Equal complete contexts compile once. `d2b-core`, `d2b-contracts`, and
  `d2b-host` split between production and test carriers; realm contexts stay
  shared. Broker-local default, layer1-bootstrap, and fake-backends contexts
  remain independently enumerable. Shared package tests remain single-owned.
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
6. Library-only shared variants and actual F, B-prod, B-default, B-layer1,
   B-fake, B, M, `@broker`, targets, cases, and spokes match their
   authoritative derivation.
7. ADR 0054 authorizes no broker repin execution or lifecycle contract.
8. Broker repin remains a built-process no-child, no-write exact refusal with
   `broker-repin-architecture-pending` until a separate accepted ADR replaces
   that block; Cargo-launcher bootstrap output and state are outside the exact
   result.
9. Main, guest, and walker repin behavior remains exactly ADR 0052 behavior.
10. Spec 003 W0 remains parked, and only three exact agreeing `READY` blocks
    can admit T021. The four non-status literals are immutable, and descriptor-
    relative, no-follow, same-fd parsing rejects link, replacement, nonregular,
    NUL, invalid-UTF-8, marker, field, and disagreement mutations before
    activity, pending that ADR, amendment, and a new panel.

## References

- [ADR 0009](0009-rust-toolchain-msrv-and-supply-chain.md), items 3, 5, and 7
- [ADR 0052](0052-bazel-rust-build-and-test.md), sections 2, 3, 4, 6, and 10
- `rules_rust` 0.73.0 `crate_universe` splicing behavior
