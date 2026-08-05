# ADR 0054: A generated splice workspace for the privileged broker's Bazel dependency hub

- Status: Proposed
- Date: 2026-08-04
- Refines and corrects: [ADR 0052](0052-bazel-rust-build-and-test.md),
  which establishes the Bazel Rust migration and the scoped repin command.
  This record preserves its independent hubs and authoritative Cargo inputs,
  corrects its lock count, and decides how the `broker` hub obtains a
  spliceable workspace.
- Related: [ADR 0009](0009-rust-toolchain-msrv-and-supply-chain.md), whose
  per-lock supply-chain policy is why the broker lock remains independent;
  [ADR 0002](0002-non-root-daemon-and-privileged-broker.md) and
  [ADR 0015](0015-daemon-only-clean-break.md), which make the broker the
  privileged dependency closure that must remain separately auditable.
- Scope: the `broker` `crate_universe` hub, the generated workspace it
  splices, ownership of generated Bazel inputs and the broker Bazel-side lock,
  first-party broker variants, and the checks required before Spec 003 W0 can
  close.
- Non-scope: runtime behavior, broker operations, Nix packaging, the other
  three hubs, and implementation. This record changes no code, Spec 003 plan,
  task, or contract.
- Implementation prerequisite: Spec 003's plan, tasks, and contracts require a
  pending amendment for this decision. Implementation remains parked until
  that amendment is reviewed and merged.

## Context

ADR 0052 assigns one `crate_universe` hub to each Cargo workspace. The
authoritative set is four, not three:

1. `packages/Cargo.lock` for `main`;
2. `packages/d2b-priv-broker/Cargo.lock` for `broker`;
3. `packages/d2b-guest-shell-runner/Cargo.lock` for `guest`;
4. `tests/tools/no-bash-ast-walker/Cargo.lock` for `walker`.

`packages/Cargo.guest.lock` remains a generated input and cache-key input. It
is not a Cargo workspace lock and does not have a hub.

The broker is a standalone Cargo workspace. It path-depends on five packages
that are members of the separate main workspace: `d2b-contracts`, `d2b-core`,
`d2b-host`, `d2b-realm-core`, and `d2b-realm-provider`. Cargo resolves that
layout, but `crate_universe` 0.73.0 cannot splice it. Supplying only the broker
manifest relocates it without its path dependencies. Supplying the broker and
path manifests together is refused because they belong to different
workspaces.

Merging the workspaces would make the build tool happy by dissolving the
broker's independently pinned and independently audited dependency closure.
Cross-binding the broker to main-hub first-party targets would instead compile
the privileged binary against the wrong resolve. Neither is acceptable.

A measured generated workspace containing the broker member and the realized
path-dependency closure does splice successfully. Its lock mirror satisfies
`cargo metadata --locked --offline` when it describes the realized closure,
including the broker member's test dependencies and excluding test-only or
unselected optional dependencies of non-member path packages. This is the
smallest workaround that preserves the standalone workspace and authoritative
broker lock.

The prior draft made repin synchronize this generated tree itself. That gave
two commands ownership of one output and expanded a lock-only operation into
subtree publication and recovery. This record rejects that expansion. A
tracked generated artifact and its ordinary drift check are enough.

The existing Spec 003 plan, tasks, and
`contracts/workspace-and-tool-pinning.md` predate this decision. They do not
encode the generated broker workspace, the two-command ownership boundary, or
the checks below. Their current contract is not preserved by assertion.
Spec 003 must be amended before implementation resumes.

## Decision

### 1. Preserve the standalone broker workspace and independent lock

`packages/d2b-priv-broker/Cargo.toml` remains a standalone Cargo workspace,
and `packages/d2b-priv-broker/Cargo.lock` remains its independent
authoritative lock. Neither `cargo xtask gen-bazel` nor any Bazel command may
edit either file.

The `broker` hub continues to use:

- `packages/d2b-priv-broker/Cargo.lock` as `cargo_lockfile`;
- `bazel/cargo/broker.lock` as its Bazel-side lock;
- `skip_cargo_lockfile_overwrite = True`.

Only its `manifests` input changes. It names a committed generated workspace
under `bazel/cargo/broker-workspace/`, including the generated root manifest
and every generated package manifest needed to put their content inside the
hub digest.

The other three hubs and their locks are unchanged by this decision.

### 2. Generate an exact resolution witness

`bazel/cargo/broker-workspace/` is a tracked generated resolution witness. It
contains:

- a workspace root manifest;
- one generated package manifest and inert source target for the broker
  member and every package in its realized first-party path closure;
- a `Cargo.lock` byte-identical to
  `packages/d2b-priv-broker/Cargo.lock`;
- a `BUILD.bazel` file exporting the exact manifest set and lock mirror.

The inert sources are never first-party compilation inputs. They exist only
so Cargo and `crate_universe` can resolve the same package graph from one
workspace root.

Fidelity is exact, not name-and-version similarity. The generator derives a
canonical record from locked Cargo metadata and the authoritative lock, then
requires equality for all of the following:

- path-package identity, including package identity and repository-relative
  path;
- registry package identity, registry source identity, and checksum;
- git package identity, canonical URL, precise revision, and checksum where
  the source records one;
- the feature map and realized feature selection;
- target identity and target kind;
- dependency edges, including dependency kind and target condition.

The generated witness carries only dependency edges realized by the broker
workspace. The broker workspace member retains the dependencies required by
its own targets and tests. A non-member path package does not gain its
dev-dependencies or an unselected optional dependency merely because its
manifest declares them.

Checks compare those exact canonical records. Independent planted mutations
to a source identity and to a checksum must each fail. A comparison over only
package names and versions is insufficient.

The lock mirror is separately checked for byte equality. The generated root
must also pass:

```text
cargo metadata --manifest-path bazel/cargo/broker-workspace/Cargo.toml --locked --offline
```

No network fallback is permitted.

### 3. Give each output one writer

`cargo xtask gen-bazel` is the sole writer of
`bazel/cargo/broker-workspace/**` and every other generated Bazel input owned
by the repository generator. Hand edits and repin writes to those paths are
forbidden. Per-hub Bazel-side lock files are excluded from the generator's
output manifest and remain owned by scoped repin.

The mutation form uses the repository's ordinary generated-artifact writer
and ownership manifest. It does not add a special publication protocol for
the broker subtree.

`cargo xtask gen-bazel --check` is strictly read-only. It computes the exact
expected output bytes and output census, compares both with the tracked
outputs, and exits nonzero on any missing, extra, or byte-different output. It
does not invoke the mutation form and does not repair drift.

Tests for `--check` snapshot tracked output bytes and census plus seeded
ignored state before and after both a passing run and a failing run. The two
snapshots must be identical. The seeded state includes lock, scratch, and
transaction-directory sentinels so the test also proves that `--check`
creates, removes, or changes none of them. A separate absence assertion proves
that a clean fixture gains no lock file, scratch path, or transaction
directory.

Make and workflows may invoke `cargo xtask gen-bazel --check` through the
ordinary drift gate. They may not invoke the mutating
`cargo xtask gen-bazel`.

### 4. Keep broker repin lock-only

`cargo xtask bazel-repin --hub broker` is the sole writer of
`bazel/cargo/broker.lock`. Its admitted tracked output set is exactly that one
path. It does not generate, copy, publish, repair, or otherwise modify
`bazel/cargo/broker-workspace/**` or any other generated Bazel input.

Before spawning Bazel, broker repin runs the broker slice of
`cargo xtask gen-bazel --check` and requires the generated inputs to be
tracked and committed. A stale, missing, extra, byte-different, or uncommitted
generated input refuses before Bazel is spawned and before the broker lock is
written. The refusal carries this exact two-command remedy:

```text
run `cargo xtask gen-bazel`, review and commit the generated workspace, then rerun `cargo xtask bazel-repin --hub broker`
```

There is no automatic generation in repin. This ordered workflow is
deliberate:

1. run `cargo xtask gen-bazel`, review the generated diff, and commit it;
2. run `cargo xtask bazel-repin --hub broker`, review the lock diff, and
   commit it.

That is the contributor cost of preserving single-writer ownership while
keeping repin's output set lock-only. Bazel and `rules_rust` contributors must
not be promised a one-command dependency update. No existing command is
claimed to provide one.

Repin retains ADR 0052's required explicit closed-set `--hub` argument,
single-hub child environment, and changed-path guard. It is an explicit
contributor mutation command and is reachable from neither Make nor a
workflow. No gate or build target sets repin controls.

This decision adds no subtree lock, scratch lifecycle, transaction state,
quarantine mode, rename or anchored-open protocol, child recovery, or static
transaction error taxonomy. Once Bazel starts, ordinary Bazel failure
semantics apply. A failed repin may leave its one tracked output modified; the
contributor reviews or discards that lock diff and reruns the command. The
repository does not build a second recovery system around a regenerable,
version-controlled lock file.

### 5. Compile the broker closure as library-only variants

The five non-member path packages are compiled twice: their ordinary variants
use the main hub, and their `-broker` variants use the broker hub. Every
`-broker` path-dependency variant is a library target only. It has no
`rust_test`, doctest, or other test target.

Tests for those five packages remain owned by their ordinary main-workspace
variants. Broker-hub tests exist only for members of the broker Cargo
workspace. Today that member is `d2b-priv-broker`, whose member targets and
tests continue to use the broker lock.

This preserves Cargo's current test ownership. Adding broker variants for
path-package tests would require dev-dependencies that the authoritative
broker lock does not resolve.

### 6. Derive exact B and M sets before checking isolation

The generator and the analysed graph expose the same exact first-party target
census.

`B_expected` is derived from locked broker metadata and is exactly:

- the five path-dependency library variants
  `d2b-contracts-broker`, `d2b-core-broker`, `d2b-host-broker`,
  `d2b-realm-core-broker`, and `d2b-realm-provider-broker`; plus
- every generated target of every broker workspace member.

The complete expected first-party target set `F_expected` is derived
independently from the authoritative workspace metadata and generated target
inventory, not by adding labels to B. `M_expected` is exactly
`F_expected - B_expected`.

Before any edge predicate runs, the checks require:

- `B_actual == B_expected`;
- `M_actual == M_expected`;
- both actual sets are nonempty;
- their intersection is empty;
- their union is `F_expected`.

Equality is symmetric: both missing and extra labels fail. Planted missing,
extra, and empty mutations for B and for M must fail before the isolation
predicates execute. The same census is checked against the generator's edge
map and a real Bazel query so a generator/query disagreement cannot pass.

### 7. Check first-party edges directly, then third-party spokes

The primary isolation check is over first-party compile edges, including
`deps` and `proc_macro_deps`:

- the first-party portion reachable from B stays within B;
- the first-party portion reachable from M stays within M.

Each direction has its own planted mutation. One mutation rebinds a broker
variant to an ordinary first-party variant; another rebinds an ordinary
target to a broker variant. Each must fail and name the offending edge.

A supplemental check validates direct third-party spokes. A B target may
reach third-party crates only through `@broker//`. An M target's direct
third-party spoke must match that target's independently derived hub ownership
and may not use `@broker//`; main targets normally use `@main//`, while guest
and walker targets retain their own hubs. This direct-spoke check has its own
planted wrong-hub mutations in both directions.

The spoke check does not replace the first-party check. A first-party target
can have no third-party dependencies, so spoke-only isolation can pass while a
wrong first-party compile edge remains.

## Required validation

Before Spec 003 W0 closes, the amended plan must assign owners and enforcing
carriers for all of the following:

1. `cargo xtask gen-bazel --check` passes on the committed tree and fails for
   missing, extra, and byte-different generated outputs.
2. Passing and failing `--check` tests prove byte-identical tracked and
   ignored fixture state before and after and prove that no lock, scratch, or
   transaction directory is created.
3. Exact package/source fidelity passes, and independent source-identity and
   checksum mutations fail.
4. The generated lock mirror is byte-identical to the authoritative broker
   lock, and offline locked metadata succeeds on the generated workspace.
5. B and M pass symmetric exact-census and nonempty checks; missing, extra,
   and empty mutations fail before predicates.
6. Both directions of direct first-party compile-edge isolation and both
   directions of supplemental direct third-party spoke isolation fail under
   independent planted mutations.
7. A stale broker generated workspace makes broker repin print the required
   two-command remedy, spawn no Bazel child, and change no path.
8. A successful broker repin changes no tracked path other than
   `bazel/cargo/broker.lock`; the authoritative Cargo lock and generated
   workspace remain byte-identical.
9. Real Bazel query checks reproduce the exact manifest, first-party target,
   B, M, and broker repository censuses from the generated checks.
10. Real Bazel builds of the broker member targets and representative broker
    repository crates succeed against the committed generated workspace and
    lock.
11. ADR 0052's exact carrier checks remain total and unambiguous. Every
    required surface has its exact carrier set, no extra carrier is accepted,
    and planted missing and extra carrier mutations fail.

Unit tests or generated-map checks alone do not close items 9 through 11.
Real Bazel query and build evidence and the exact carrier checks are required
before W0 can close.

## Consequences

- The privileged broker keeps a small, independently pinned and independently
  audited Cargo closure.
- The generated workspace is reviewable and drift-checked like other
  generated artifacts. Its exact source and edge identity is checked rather
  than inferred from a successful lock parse.
- Dependency updates take two repository commands and an intentional review
  boundary between them. This is slower than self-synchronizing repin and
  makes ownership substantially simpler.
- Repin remains explicit, scoped, lock-only, and absent from Make and
  workflows.
- Five first-party libraries compile once for the main resolve and once for
  the broker resolve. Their tests do not duplicate.
- A failed repin can leave a reviewable broker-lock diff. Version control and
  rerunning the deterministic command are the recovery; there is no bespoke
  recovery subsystem.
- Spec 003 cannot resume from its current plan. Its plan, tasks, ownership
  map, contracts, and validation commands must first be amended to this
  decision.

## Alternatives considered

### Merge the broker into the main Cargo workspace

Rejected. It removes the broker's independent lock and expands the audited
privileged closure to the main workspace resolve for build-tool convenience.

### Bind broker targets to main-hub first-party targets

Rejected. A first-party library compiled against the main resolve is not the
same artifact as that library compiled against the broker resolve. Linking it
into the broker silently mixes dependency sets.

### Upgrade or patch `rules_rust`

Rejected for this decision. Version 0.73.0 was the newest release measured,
and no released upgrade accepted the cross-workspace splice. Carrying a local
splicer patch would create a larger permanent maintenance surface than a
small generated witness. A future upstream feature can replace the witness
through a new decision and migration.

### Hand-maintain splice manifests or BUILD targets

Rejected. Manual specifications drift on optional features, source identity,
checksums, target kinds, and edges. The repository already has a generated
artifact ownership and drift-check pattern.

### Make broker repin synchronize generated inputs

Rejected. Self-synchronizing repin creates two writers for the generated tree,
widens repin beyond its selected lock, hides a review boundary, and requires
special partial-failure behavior. The explicit two-command workflow is the
smaller design.

## Invariants this decision creates

1. The broker remains a standalone Cargo workspace with its own authoritative
   lock.
2. There are four hub/workspace Cargo locks. `packages/Cargo.guest.lock` is a
   separate generated and cache-key input, not a hub lock.
3. The broker hub splices only the committed generated broker workspace while
   rendering from the authoritative broker Cargo lock with overwrite disabled.
4. `cargo xtask gen-bazel` is the sole writer of generated Bazel inputs, and
   its `--check` form is strictly read-only.
5. `cargo xtask bazel-repin --hub broker` writes only
   `bazel/cargo/broker.lock` and never generates its inputs.
6. Stale or uncommitted broker generated inputs refuse repin before Bazel with
   the required two-command remedy.
7. The generated lock mirror is byte-identical to the authoritative broker
   lock, and locked offline metadata succeeds on the generated workspace.
8. Generated workspace fidelity includes exact package and source identity,
   checksums where applicable, features, target kinds, and dependency edges.
9. The five broker path-dependency variants are library-only. Their tests
   remain main-owned; broker member tests remain broker-owned.
10. B is exactly the five path-dependency library variants plus broker member
    targets. M is independently derived as the exact complete first-party
    target set minus B. Exact, nonempty censuses precede edge predicates.
11. Direct first-party compile-edge isolation is primary. Direct third-party
    spoke isolation is supplemental. Both have independent planted failures.
12. Neither mutating command is reachable from Make or workflows. Gates may
    invoke only `cargo xtask gen-bazel --check`.
13. Real Bazel query and build evidence and exact carrier checks are required
    before Spec 003 W0 closes.
14. Spec 003's plan, tasks, and contracts must be amended before implementation
    resumes.

## References

- [ADR 0052](0052-bazel-rust-build-and-test.md), especially its hub, repin,
  generated BUILD, coverage, and carrier decisions
- [ADR 0009](0009-rust-toolchain-msrv-and-supply-chain.md), the per-lock
  supply-chain policy preserved here
- `rules_rust` 0.73.0 `crate_universe` splicing and lock-digest behavior
- `packages/Cargo.toml` and the four hub/workspace Cargo locks listed above
- `packages/Cargo.guest.lock`, the separate generated and cache-key input
- `specs/003-adr052-bazel-rust/plan.md`
- `specs/003-adr052-bazel-rust/tasks.md`
- `specs/003-adr052-bazel-rust/contracts/workspace-and-tool-pinning.md`
