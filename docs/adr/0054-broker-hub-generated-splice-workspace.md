# ADR 0054: A generated splice workspace for the privileged broker's Bazel dependency hub

- Status: Proposed
- Date: 2026-08-05
- Refines and corrects: [ADR 0052](0052-bazel-rust-build-and-test.md),
  especially section 2, "Cargo stays the authoritative dependency and
  toolchain input", section 3's scoped repin command, and section 6's
  intentional three-lock supply-chain scope.
- Authority: ADR 0052 section 2 remains the dependency-authority decision.
  Cargo manifests and authoritative Cargo locks decide dependency resolution
  and feature selection. The generated workspace and Bazel locks are derived
  witnesses, never declaration authorities.
- Related: [ADR 0009](0009-rust-toolchain-msrv-and-supply-chain.md), which
  establishes the aggregate Cargo supply-chain policy; ADR 0052 section 6,
  which carries it for the `main`, `broker`, and `guest` locks; and
  [ADR 0002](0002-non-root-daemon-and-privileged-broker.md) and
  [ADR 0015](0015-daemon-only-clean-break.md), which make the broker the
  privileged dependency closure that must remain separately auditable.
- Scope: the `broker` `crate_universe` hub, its generated splice workspace,
  the `gen-bazel` and broker-repin ownership boundary, exact dependency
  projections, broker first-party variants, and their validation.
- Non-scope: runtime behavior, broker operations, Nix packaging, the other
  three repin protocols, implementation, and Spec 003 edits.
- Implementation prerequisite: a Spec 003 amendment is pending. Its plan,
  tasks, ownership map, contracts, and validation commands must adopt this
  decision before implementation resumes. This ADR changes none of them.

## Context

ADR 0052 assigns one `crate_universe` hub to each Cargo workspace. The stable
hub tokens and authoritative hub/workspace locks are:

| Hub token | Authoritative Cargo lock | Bazel-side lock |
| --- | --- | --- |
| `main` | `packages/Cargo.lock` | `bazel/cargo/main.lock` |
| `broker` | `packages/d2b-priv-broker/Cargo.lock` | `bazel/cargo/broker.lock` |
| `guest` | `packages/d2b-guest-shell-runner/Cargo.lock` | `bazel/cargo/guest.lock` |
| `walker` | `tests/tools/no-bash-ast-walker/Cargo.lock` | `bazel/cargo/walker.lock` |

`guest` is the stable CLI token. `guest-shell` is not an accepted `--hub`
value. `packages/Cargo.guest.lock` is separate: it is a generated
guest-workspace and cache-key input, not a hub/workspace lock and not a fifth
hub.

The supply-chain scope is intentionally different from the hub inventory.
ADR 0052 section 6 applies `cargo-deny`, `cargo-audit`, and the yanked-state
carrier to exactly the `main`, `broker`, and `guest` locks. The `walker` lock
has no deny or audit surface today. This ADR neither adds one nor treats the
four-hub inventory as a four-lock supply-chain scope.

The broker is a standalone Cargo workspace. It path-depends on five packages
that are members of the separate main workspace: `d2b-contracts`, `d2b-core`,
`d2b-host`, `d2b-realm-core`, and `d2b-realm-provider`. Cargo resolves that
layout, but measured `crate_universe` 0.73.0 cannot splice it. Supplying only
the broker manifest relocates it without its path dependencies. Supplying the
broker and path manifests together is refused because they belong to different
workspaces.

A generated workspace containing the broker member and its realized
first-party path closure does splice successfully. Locked offline metadata
succeeds when the mirror includes the broker member's realized test
dependencies and excludes inactive optional and non-member test-only
dependencies. This is the smallest measured workaround that preserves the
standalone workspace and authoritative broker lock.

The current Spec 003 plan, tasks, and
`contracts/workspace-and-tool-pinning.md` predate this decision. They do not
encode the generated witness, the two-command ownership boundary, or the
broker-specific clean-worktree protocol below. Implementation remains parked
until that spec set is amended explicitly.

## Decision

### 1. Preserve the standalone broker workspace and Cargo authority

`packages/d2b-priv-broker/Cargo.toml` remains a standalone Cargo workspace,
and `packages/d2b-priv-broker/Cargo.lock` remains its authoritative lock.
Neither `cargo xtask gen-bazel` nor Bazel may edit either file.

The `broker` hub uses:

- `packages/d2b-priv-broker/Cargo.lock` as `cargo_lockfile`;
- `bazel/cargo/broker.lock` as its Bazel-side lock;
- `skip_cargo_lockfile_overwrite = True`; and
- the committed generated workspace under
  `bazel/cargo/broker-workspace/` as its manifest set.

The other three hubs retain their own authoritative and Bazel-side locks.
Every inventory, index, and exact-census check uses the four stable tokens
`main`, `broker`, `guest`, and `walker`.

ADR 0052's cache keys bind all four authoritative hub/workspace locks, all
four Bazel-side locks, and the separate `packages/Cargo.guest.lock`. They also
retain ADR 0052's other key inputs: `.bazelversion`, `MODULE.bazel`,
`MODULE.bazel.lock`, `.bazelrc`, both toolchain pins, all three `deny.toml`
files, the advisory and yanked-state pins, the `cargo-bazel` checksum,
`.bazelignore`, startup and symlink configuration, build-script and action
environment digests, and the generated BUILD-tree digest.

### 2. Generate one exact resolution witness

`bazel/cargo/broker-workspace/` is a tracked generated witness. It contains:

- one workspace-root manifest;
- one generated package manifest and inert source target for the broker member
  and each package in its realized first-party path closure;
- a `Cargo.lock` byte-identical to the authoritative broker lock; and
- a `BUILD.bazel` exporting the exact manifest set and lock mirror.

The inert sources are resolution inputs, never first-party compilation inputs.

The generator constructs four projections:

- **Authoritative `A`.** Locked offline Cargo metadata over the committed
  broker manifests and authoritative broker lock at `HEAD`.
- **Witness `W`.** Locked offline Cargo metadata over the generated splice
  workspace, with the synthetic root and only the declared inert targets
  removed and generated paths mapped through a closed path map.
- **Bazel lock `L`.** The actual parsed `bazel/cargo/broker.lock` after repin.
- **Repository `R`.** The actual materialized `@broker` repository, derived
  from real Bazel query and repository contents rather than the generator's
  expected map.

For every realized package, `A` and `W` record all representable fields:

- package identity;
- normalized path identity or source kind and normalized source identity;
- registry checksum;
- canonical git URL, precise revision, and checksum when present;
- exact resolved feature set; and
- every applicable target's identity and kind.

For every realized dependency edge they record:

- source and destination package identities;
- dependency kind;
- normalized target condition;
- the manifest dependency alias, including an explicit no-alias value;
- requested edge features;
- default-feature semantics; and
- the realized feature contribution of that edge.

`A` and `W` must be symmetrically equal. No comparison may collapse to names
and versions, aggregate package features, or an unordered destination set.
Aliases and per-edge features are part of identity.

`L` and `R` each have an explicit field-capability map. Each is compared
symmetrically with the projection of `A` onto every field that representation
can express. They are also compared symmetrically with each other over their
shared representable fields. In particular:

- source identity, checksum, and precise git revision are exact;
- resolved feature sets are exact;
- crate target identity and kind are exact wherever represented; and
- dependency destination, kind, condition, alias, requested features,
  default-feature semantics, and realized edge-feature contribution are exact
  wherever represented.

No field present in `L` or `R` may be discarded merely because the other
representation lacks it. A missing, extra, or empty package or edge set is a
failure, including when both sides are accidentally empty.

Declarations that Cargo metadata does not represent as realized fields use a
separate exact declaration ledger. It covers:

- inactive optional dependencies and their feature declarations;
- excluded non-member dev-dependencies;
- target declarations not otherwise represented; and
- synthesized inert targets.

Each omitted declaration appears exactly once with the reason it is omitted.
Each inert target matches the closed template and package census and is absent
from first-party compilation inputs. The lock mirror is checked separately for
byte equality. The generated root must pass:

```text
cargo metadata --manifest-path bazel/cargo/broker-workspace/Cargo.toml --locked --offline
```

Independent negatives cover missing, extra, and empty metadata, source,
checksum, git revision, package feature, target identity, target kind, alias,
edge kind, edge condition, requested edge feature, default-feature semantics,
realized edge-feature contribution, and offline metadata availability. A
negative changes one dimension and must fail its named guard.

### 3. Keep two single-owner commands and one worktree-local lock

`cargo xtask gen-bazel` alone writes
`bazel/cargo/broker-workspace/**` and every other path in its exact generated
output ownership manifest. Per-hub Bazel-side locks are excluded from that
manifest. Broker repin writes only `bazel/cargo/broker.lock`.

`cargo xtask gen-bazel --check` is strictly read-only. It computes expected
bytes, output census, `A`, `W`, and the declaration ledger without creating a
lock, scratch directory, temporary file, cache entry, or transaction state. It
fails on a missing, extra, byte-different, or semantically different output and
never repairs drift.

Passing and failing tests take before and after snapshots of the complete
tracked and ignored state. The snapshot includes path census, object kind,
mode, bytes, and symlink target. The snapshots must be identical. A clean
fixture separately proves that neither outcome creates a lock or any
`.scratch/` entry.

The mutating generator and broker repin share one exclusive worktree-local OFD
lock in the existing `.scratch/bazel/` policy root. There is no new scratch
namespace. The mutating process opens the lock with `O_CLOEXEC`; broker repin's
parent retains the open file description through child termination, reap, and
all postchecks. The Bazel child cannot inherit it. `gen-bazel --check` neither
takes nor creates the lock.

Cooperating repository writers must take this lock. Contention refuses and
tells the contributor to wait for the other repository mutation command to
finish, then rerun the same command.

### 4. Require a clean committed generation boundary

The supported dependency-update flow is:

1. edit authoritative Cargo inputs;
2. run `cargo xtask gen-bazel`;
3. review and commit together every changed authoritative Cargo input,
   authoritative lock, applicable `packages/Cargo.guest.lock`, and path in the
   exact generated-output ownership manifest;
4. run `cargo xtask bazel-repin --hub broker`; and
5. review and commit `bazel/cargo/broker.lock`.

Repin never invokes the mutating generator and never repairs, copies, stages,
or publishes generated inputs.

Before taking the OFD lock, broker repin records a full Git census. The census
includes `HEAD`, index entries and staged state, tracked worktree state,
untracked entries, and ignored entries. Only descendants of ADR 0052's exact
pre-existing bounded Bazel output-user-root, action-cache, and repository-cache
roots are excluded from path comparison. No other ignored path is excluded.

Admission requires:

- `HEAD` is stable across two reads;
- index and tracked worktree bytes equal `HEAD`;
- there is no staged, unstaged, untracked, or ignored entry outside the exact
  bounded Bazel roots;
- the complete authoritative input and generated-output change was committed
  together;
- every governed input and output is a tracked regular non-symlink at `HEAD`;
  and
- the generated ownership census is exact.

After taking the OFD lock, repin repeats the census and `HEAD` checks, records
the complete governed input set from `HEAD`, and runs the broker slice of
`gen-bazel --check`. The check is read-only even though repin already holds the
writer lock. Every governed input byte must equal its `HEAD` byte immediately
before the child starts.

A byte-current change in the index or worktree is still uncommitted and is
refused. Required-input or generated-output drift uses this exact remedy:

```text
run `cargo xtask gen-bazel`, review it, commit the authoritative Cargo inputs and all generated outputs together, then rerun `cargo xtask bazel-repin --hub broker`
```

An unrelated staged, unstaged, untracked, or ignored path uses this exact
remedy:

```text
commit or move every listed unrelated path out of this worktree so HEAD, index, and worktree are clean, then rerun `cargo xtask bazel-repin --hub broker`
```

The command lists repository-relative paths and spawns no Bazel child on
either refusal.

Before the child, `bazel/cargo/broker.lock` is opened without following a
symlink and must be a regular file with `st_nlink == 1`. Its bytes must equal
the file at `HEAD`. A missing file, symlink, hard link, non-regular object, or
byte difference refuses before Bazel.

### 5. Repin directly in the clean current worktree

Broker repin runs Bazel only in the contributor's current worktree. It creates
no detached worktree, snapshot, mount or PID namespace, fresh root, candidate
file, receipt, quarantine, or publication transaction.

The broker child alone uses `--batch`; no persistent Bazel server is started.
Its ordinary output user root, output base, action cache, repository cache, and
symlink prefix use ADR 0052's existing bounded `.scratch/` policy. Broker
repin adds no output root and no scratch namespace.

The parent constructs the child environment from an empty environment and
adds only the fixed tool, locale, worktree, bounded-output, and repin-control
values the invocation requires. Cache credentials, cloud credentials, proxy
credentials, agent sockets, user rc selection, and inherited repin controls
are absent. Non-stdio descriptors are closed. The repin controls exist only in
this child and select exactly `broker`.

The child enters a dedicated process group and sets Linux
`PR_SET_PDEATHSIG`, then verifies that its parent did not change while setting
it. On a handled signal, timeout, or failure, the parent sends TERM to the
group, waits a fixed bound, sends KILL to survivors, and reaps the child. The
parent retains the OFD lock until reap and postchecks complete.

This is process containment, not a host sandbox. There is no mount-namespace
or fresh-root security claim. Bazel actions are trusted same-user contributor
code and retain the ordinary host access current Cargo tooling has. A
concurrent adversarial process under the same uid can race files and is outside
the threat model. Cooperating repository writers are serialized by the OFD
lock; accidental movement by other tools is caught by the before and after
checks.

Bazel writes `bazel/cargo/broker.lock` directly. There is no rollback or
second publication step.

### 6. Validate the direct result and leave failure recoverable

After the Bazel child is terminated and reaped, while the parent still holds
the OFD lock, repin requires:

- `HEAD` still equals the captured commit;
- every governed input still equals its captured `HEAD` bytes;
- the full Git census, including tracked, staged, untracked, and ignored
  entries outside the exact bounded Bazel roots, differs from the prestate by
  either no path or exactly `bazel/cargo/broker.lock`;
- `broker.lock` is again a regular non-symlink with `st_nlink == 1`;
- the lock parses and the selected hub resolves;
- locked offline metadata for the generated workspace succeeds;
- real Bazel query and representative builds resolve the actual `@broker`;
  and
- `L` and `R` pass every exact comparison against `A` and each other.

An empty changed-path set is a successful already-current no-op. A changed set
containing only `broker.lock` is successful only after all semantic checks
pass. Any other path, index change, ignored entry, type change, link-count
change, parse failure, or projection mismatch is failure.

Independent spoke mutations alter source, checksum, git revision, feature,
target, alias, or edge semantics in actual `broker.lock` while `@broker` and
the generated witness remain unchanged, and separately in actual `@broker`
while `broker.lock` and the witness remain unchanged. Each side must fail its
own authoritative comparison and the symmetric actual-to-actual comparison
where the field is shared.

A Bazel error, validation failure, handled termination, or killed parent may
leave a dirty or partial `broker.lock`. Repin does not claim transactional
publication and does not attempt automatic recovery. The exact recovery is:

```text
git restore --source=HEAD --worktree --staged -- bazel/cargo/broker.lock
```

Then rerun `cargo xtask bazel-repin --hub broker`. This restoration is safe
because admission required the index and worktree, including `broker.lock`, to
equal clean `HEAD`. The command repairs only `broker.lock`; any separately
reported unrelated movement must also be resolved before rerunning.

### 7. Compile broker path dependencies as library-only variants

The five non-member path packages compile twice. Their ordinary variants use
the main hub; their `-broker` variants use the broker hub. Every broker
path-dependency variant is a library target only, with no `rust_test`, doctest,
or other test target.

Tests for those packages remain owned by their ordinary main-workspace
variants. Broker-hub tests exist only for members of the broker Cargo
workspace. Today that member is `d2b-priv-broker`.

### 8. Derive exact F, B, and M censuses before edge checks

`F_expected` is the generator's complete first-party target inventory,
including target identity, kind, source package, and hub ownership.
`F_actual` comes from real Bazel query.

`B_expected` contains exactly:

- the five library targets `d2b-contracts-broker`, `d2b-core-broker`,
  `d2b-host-broker`, `d2b-realm-core-broker`, and
  `d2b-realm-provider-broker`; and
- every generated target identity and kind for every broker workspace member,
  derived from authoritative locked metadata.

`B_actual` is the corresponding actual query set selected by broker hub
ownership. `M_expected` is exactly `F_expected - B_expected`.
`M_actual` is exactly `F_actual - B_actual`; M is never separately curated.

Before an edge predicate runs, checks require:

- symmetric equality of expected and actual F, B, and M;
- nonempty F, B, and M;
- empty `B intersection M`; and
- `B union M == F`.

Independent missing, extra, and empty mutations for B and M fail before edge
isolation. F has independent missing and extra mutations, and an empty F
cannot pass the nonempty partition checks.

For first-party `deps` and `proc_macro_deps`, the first-party closure reachable
from B stays in B and the closure reachable from M stays in M. Each direction
has an independent planted cross-edge.

For direct third-party spokes, B uses only `@broker//`. Every M target uses
its independently derived hub owner and never `@broker//` unless it is in B.
Independent mutations bind a B target to `@main//` and an ordinary M target to
`@broker//`; each fixture first proves the first-party guard still passes and
then proves the spoke guard fails. `guest` and `walker` retain their own hubs.

## State matrix

| Prestate or result | Outcome |
| --- | --- |
| Clean committed inputs and generated outputs; current broker lock | Success, no-op, empty changed set |
| Clean committed inputs and generated outputs; stale broker lock | Success only if `broker.lock` is the sole changed path and all projections pass |
| Current or stale required input/output staged or unstaged | Refuse before Bazel with the commit-together remedy |
| Unrelated tracked path staged or unstaged | Refuse before Bazel with the unrelated-path remedy |
| Untracked path outside the exact bounded Bazel roots | Refuse before Bazel with the unrelated-path remedy |
| Ignored path outside the exact bounded Bazel roots | Refuse before Bazel with the unrelated-path remedy |
| State only inside the exact bounded Bazel roots | Permitted subject to ADR 0052's ownership and bounds |
| `gen-bazel --check` fails | Refuse before Bazel; create no check state |
| `broker.lock` is absent, linked, non-regular, or differs from `HEAD` | Refuse before Bazel |
| Writer lock is contended | Refuse; wait for the repository mutation command and rerun |
| Child fails, is terminated, or a postcheck fails | Failure; broker lock may be partial and uses the exact restore command |
| Parent is killed | No success claim; broker lock may be partial and uses the exact restore command |
| `HEAD`, index, governed input, or another repository path moves | Failure; preserve evidence, restore only broker lock, resolve other movement, rerun |

## Required validation

Before amended Spec 003 W0 can close, enforcing carriers must prove:

1. Four hub tokens, four authoritative locks, four Bazel-side locks, the
   separate `packages/Cargo.guest.lock`, the three-lock supply-chain scope,
   and the complete cache-key input set are exact.
2. `gen-bazel --check` has exact byte, census, projection,
   declaration-ledger, and before/after tracked-plus-ignored identity on
   passing and failing runs, with no lock or scratch creation.
3. Independent missing, extra, empty, source, checksum, git revision, feature,
   target identity, target kind, alias, edge kind, edge condition, edge
   feature, and offline-metadata negatives fail their named guards.
4. Byte-current and stale required states in the index or worktree refuse with
   the exact commit-together remedy and spawn no child.
5. Unrelated staged, unstaged, untracked, and ignored states refuse with the
   exact unrelated-path remedy and spawn no child.
6. The worktree-local OFD lock serializes the mutating generator and repin,
   is `O_CLOEXEC`, is not inherited, and remains held through termination,
   reap, and postchecks; `gen-bazel --check` takes no lock.
7. The broker lock's prestate and poststate regular-file, no-symlink,
   single-link, and `HEAD`-byte requirements fail independently.
8. Broker Bazel uses `--batch`, a closed allowlisted credential-free
   environment, a dedicated process group, parent-death signal, bounded TERM
   then KILL, and reap, without a namespace or fresh-root claim.
9. HEAD and complete governed inputs remain stable, and the full tracked,
   staged, untracked, and ignored census outside exact bounded roots changes
   by only `broker.lock` or by nothing.
10. Actual `broker.lock` and actual `@broker` each equal the authoritative
    representable projection and each other symmetrically, with independent
    spoke mutations.
11. Clean already-current repin succeeds as a no-op; child failure, handled
    termination, killed parent, and every semantic postcheck failure leave no
    success claim and are recoverable by the exact restore command.
12. Exact nonempty F, B, and M censuses fail independent missing, extra, and
    empty mutations before both first-party cross-edge directions and both
    direct cross-spoke directions are checked.
13. Real Bazel query and representative builds reproduce the target,
    repository, F, B, and M censuses and consume the committed witness.
14. ADR 0052's carrier map remains total and unambiguous, with planted missing
    and extra carrier mutations.

Unit tests or generated expected maps alone do not close real Bazel query,
repository, build, process, census, or carrier items.

## Consequences

- The privileged broker keeps a small independently pinned and audited Cargo
  closure.
- Dependency updates require two repository-owned mutation commands and two
  review points.
- Broker repin is simpler: it runs in a required-clean current worktree and
  may leave one directly written lock dirty after failure.
- The clean `HEAD` precondition makes one exact Git restore a safe recovery.
- The same-user threat model matches current Cargo tooling instead of claiming
  a filesystem sandbox that is not provided.
- The five shared libraries compile once for the main resolve and once for the
  broker resolve; their tests do not duplicate.
- Spec 003 remains blocked pending an explicit amendment to its plan, tasks,
  ownership map, contracts, and validation commands.

## Alternatives considered

### Merge the broker into the main Cargo workspace

Rejected. It removes the broker's independent lock and expands the audited
privileged closure for build-tool convenience.

### Bind broker targets to main-hub first-party targets

Rejected. It silently mixes dependency resolves in the privileged binary.

### Upgrade or patch `rules_rust`

Rejected for this decision. Version 0.73.0 was the newest measured release and
did not accept the cross-workspace splice. A local splicer patch is a larger
maintenance surface than the generated witness.

### Hand-maintain splice manifests or BUILD targets

Rejected. Manual declarations drift on sources, checksums, features, targets,
aliases, and per-edge semantics.

### Generate in place as part of broker repin

Rejected. It gives two commands ownership of generated inputs and hides the
required review-and-commit boundary. `gen-bazel` and broker repin remain
single-owner commands.

### Repin in a detached or copied snapshot worktree

Rejected. A detached worktree adds lifecycle, cleanup, Git registration, and
result-transfer machinery. A copied worktree also preserves uncommitted input.
The clean committed current-worktree precondition and exact post-census are
smaller and match the contributor tool threat model.

### Publish through a candidate exchange transaction

Rejected. Candidate files, exchange, receipts, quarantine, rollback, and crash
recovery solve publication of a result produced elsewhere. Bazel now writes
the one permitted file directly, and clean `HEAD` provides an exact recovery.

### Put broker repin in a mount or PID namespace

Rejected. It would add platform and cleanup machinery without excluding
trusted same-user contributor code from ordinary host access. This decision
claims process containment only.

### Admit dirty unrelated worktree state

Rejected. A post-run changed-path allowlist cannot distinguish the
contributor's earlier dirt from the child's result. Clean prestate makes both
the result and recovery mechanically unambiguous.

### Harden against an adversarial same-uid race

Rejected for this contributor tool. Preventing a same-uid process from
mutating and restoring repository files would require a different security
boundary. The worktree-local writer lock serializes cooperating repository
writers, and exact before and after checks catch accidental movement.

## Invariants this decision creates

1. Cargo authority remains exactly where ADR 0052 section 2 places it.
2. Hub tokens are exactly `main`, `broker`, `guest`, and `walker`; supply-chain
   coverage intentionally remains the three `main`, `broker`, and `guest`
   locks.
3. `packages/Cargo.guest.lock` is a separate generated and cache-key input.
4. The broker hub consumes only the committed generated witness and the
   authoritative broker lock with overwrite disabled.
5. `gen-bazel` alone writes generated Bazel inputs; broker repin alone writes
   `broker.lock`; `gen-bazel --check` is read-only and state-free.
6. The two writers share one worktree-local OFD lock; the repin parent holds
   it through child reap and postchecks.
7. Broker repin admits only clean `HEAD`, index, worktree, untracked, and
   ignored state outside ADR 0052's exact bounded Bazel roots.
8. Broker Bazel runs directly in that current worktree with `--batch`, a
   closed credential-free environment, process-group containment, and no
   namespace or fresh-root security claim.
9. Success changes only regular single-link `broker.lock` or changes nothing.
10. Failure may leave `broker.lock` partial; clean `HEAD` makes the exact Git
    restore safe.
11. A, W, L, and R compare every representable realized field symmetrically;
    non-realized declarations use a separate exact ledger.
12. F, B, and M are exact nonempty symmetric censuses before edge checks.
13. Neither mutating command is reachable from Make or workflows.
14. Spec 003's explicit amendment remains a prerequisite to implementation.

## References

- [ADR 0052](0052-bazel-rust-build-and-test.md), section 2 authority,
  section 3 scoped repin, section 6 three-lock supply-chain scope, and
  sections 8 and 10 bounded Bazel state and cache keys
- [ADR 0009](0009-rust-toolchain-msrv-and-supply-chain.md), aggregate
  supply-chain policy
- `rules_rust` 0.73.0 `crate_universe` splicing and repin behavior
- `packages/Cargo.toml` and the four authoritative locks listed above
- `packages/Cargo.guest.lock`, the separate generated and cache-key input
- `specs/003-adr052-bazel-rust/plan.md`
- `specs/003-adr052-bazel-rust/tasks.md`
- `specs/003-adr052-bazel-rust/contracts/workspace-and-tool-pinning.md`
