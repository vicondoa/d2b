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

Each declaration has one class-specific reason from the closed set
`inactive-optional`, `excluded-nonmember-dev`, `unrepresented-target`, and
`synthesized-inert-target`. Each omitted declaration appears exactly once,
with the reason for its class; no realized declaration may also appear in the
ledger. Each inert target matches the closed template and package census and
is absent from first-party compilation inputs. The lock mirror is checked
separately for byte equality. The generated root must pass:

```text
cargo metadata --manifest-path bazel/cargo/broker-workspace/Cargo.toml --locked --offline
```

For each of `A`, `W`, `L`, and `R`, independent one-axis negatives cover a
missing, extra, and empty package projection and a missing, extra, and empty
edge projection. Further one-axis negatives cover package identity, source,
checksum, precise git revision, resolved feature, target identity, target
kind, edge kind, condition, alias, requested feature, default-feature
semantics, realized edge-feature contribution, locked-metadata failure, the
actual `broker.lock` identity, and the actual `@broker` identity. Every
declaration-ledger class independently covers missing, extra, duplicate, and
wrong-reason rows, and an empty ledger is accepted only when the authoritative
declaration census proves that class is empty. A negative changes one
dimension and must fail exactly once at its named guard, not at a shared
parser or an earlier unrelated guard.

### 3. Keep two single-owner commands and one stable bookkeeping lock

`cargo xtask gen-bazel` alone writes
`bazel/cargo/broker-workspace/**` and every other path in its exact generated
output ownership manifest. Per-hub Bazel-side locks are excluded from that
manifest. Broker repin writes only `bazel/cargo/broker.lock`.

`cargo xtask gen-bazel --check` is strictly read-only. It computes expected
bytes, output census, `A`, `W`, and the declaration ledger without creating a
lock, scratch directory, temporary file, cache entry, or transaction state. It
fails on a missing, extra, byte-different, or semantically different output and
never repairs drift. Passing and failing tests take identical before and after
snapshots of tracked worktree objects, staged objects, ordinary untracked
objects, and ignored objects, including each object's type, mode, bytes, and
symlink target. A clean fixture proves neither outcome creates bookkeeping or
`.scratch/` state.

The mutating generator and the broker-repin monitor share one exclusive Linux
OFD lock outside the reclaimable worktree:

```text
${D2B_BOOKKEEPING_DIR:-${TMPDIR:-/tmp}/d2b-bookkeeping}/bazel-repin/<worktree-id>.lock
```

`worktree-id` is the lowercase SHA-256 of a domain-separated tuple containing
the canonical absolute Git common directory and canonical absolute worktree
root. It distinguishes linked worktrees and is never printed. The configured
bookkeeping root must be absolute. Resolution starts from an anchored
descriptor, never follows a symlink or magic link, and verifies that the
bookkeeping root and `bazel-repin` directory are owned by the effective user
and mode `0700`; absent final directories are created at `0700`. The lock is
created once at mode `0600`, must remain an effective-user-owned regular
single-link file, and is never unlinked by repository tooling.

Every open uses `O_NOFOLLOW|O_CLOEXEC`. After acquiring the OFD lock, the
holder reopens the directory entry and requires its device and inode to equal
the locked descriptor before every mutation and before release. Replacement,
symlink, ownership, mode, type, or link-count drift refuses. The lock file
persists across successful commands so all later writers reuse one inode.

The mutating generator, the broker monitor, `make clean`, and every
repository-owned Bazel shutdown or cleanup path acquire this same lock before
mutating generated output, `packages/target/`, or `.scratch/bazel/`. Cleanup
never removes the bookkeeping file. Contention refuses without mutation and
tells the contributor to wait for the repository mutation command, then rerun.
`gen-bazel --check` takes no lock and creates no bookkeeping state. Tests cover
first creation, persistent inode reuse, all writer pairings, contention,
cleanup refusal, symlink and replacement attempts, and interruption.

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

Before the generic census, broker repin preflights
`bazel/cargo/broker.lock`. Its `HEAD`, index, and worktree forms must name one
tracked regular non-symlink, the worktree file must have `st_nlink == 1`, and
index and worktree bytes must equal `HEAD`. Every missing, staged, dirty,
symlinked, hard-linked, replaced, or non-regular state emits exactly:

```text
D2B-BAZEL-REPIN-BROKER-LOCK: broker lock is not the committed regular single-link file.
git restore --source=HEAD --worktree --staged -- bazel/cargo/broker.lock
cargo xtask bazel-repin --hub broker
```

The diagnostic sentence is fixed and path-free; the first command is the exact
recovery and the second is the exact rerun. No generic dirty-tree advice may
precede or replace it.

Broker repin then records a full Git object census. It includes stable `HEAD`,
tracked worktree objects, staged objects, ordinary untracked objects, and
ignored objects, recording census membership, object type, mode, bytes, and
symlink target. The only command-owned baselines are the exact roots that
already exist when admission is sampled: `packages/target/` and ADR 0052's
bounded Bazel output user root, disk/action cache, and repository/download
cache under `.scratch/bazel/`. Their descendants may change. No other ignored
root is created, excluded, or admitted.

Admission requires:

- `HEAD` is stable across two reads;
- index and tracked worktree bytes equal `HEAD`;
- there is no staged, unstaged, ordinary untracked, or ignored entry outside
  the exact command-owned roots;
- the complete authoritative input and generated-output change was committed
  together;
- every governed input and output is a tracked regular non-symlink at `HEAD`;
  and
- the generated ownership census is exact.

The CLI passes a close-on-exec lifetime pipe to the repository-owned monitor.
After the monitor takes the OFD lock, it repeats the broker-lock preflight,
census, and `HEAD` checks, records the complete governed input set from
`HEAD`, and runs the broker slice of `gen-bazel --check`. The check is
read-only even though the monitor holds the writer lock. Every governed input
byte must equal its `HEAD` byte immediately before the first Bazel invocation.

A byte-current change in the index or worktree is still uncommitted and is
refused. Required-input or generated-output drift uses this exact remedy:

```text
run `cargo xtask gen-bazel`, review it, commit the authoritative Cargo inputs and all generated outputs together, then rerun `cargo xtask bazel-repin --hub broker`
```

An unrelated staged, unstaged, ordinary untracked, or ignored object emits:

```text
D2B-BAZEL-REPIN-DIRTY: unrelated repository state blocks broker repin (<count> objects).
git status --short --untracked-files=all --ignored
```

`<count>` is decimal through `99` and `100+` thereafter. The fixed diagnostic
prints no object name or raw caller-controlled byte. The exact Git command is
the operator-controlled local inspection surface; after inspecting, the
operator commits or moves unrelated state and reruns the repin command.
Adversarial filenames containing control bytes, terminal escapes, newlines,
absolute-looking text, or non-UTF-8 bytes, and censuses on both sides of the
cap, prove default output contains only the fixed message and capped count.
Neither refusal spawns the monitor or Bazel.

### 5. Give one monitor the complete direct batch session

Broker repin runs Bazel only in the contributor's current worktree. It creates
no detached worktree, snapshot, mount or PID namespace, fresh root, candidate
file, receipt, quarantine, or publication transaction.

An internal repository-owned xtask monitor mode owns the whole broker-repin
session. The CLI supplies a control pipe whose write end stays open for its
lifetime and is never inherited by a worker. EOF, explicit cancellation, or
CLI death starts teardown. The monitor, not the CLI or an individual Bazel
leader, owns the OFD lock, Git snapshots, subprocesses, final census, and
result.

The monitor becomes a Linux child subreaper and creates a dedicated worker
process group outside its own group. A retained group anchor prevents group-id
reuse. Every subprocess joins that group; the repin, query, representative
build, and identity postcheck Bazel invocations run sequentially, and each
must reach descendant-empty completion before the next starts. Recursive
descendant discovery plus pidfds and `waitid` cover a leader that exits or is
killed while a descendant survives; leader exit alone is never an empty-group
proof.

Every Bazel invocation begins with these startup options:

```text
--batch --nosystem_rc --nohome_rc --noworkspace_rc --bazelrc=<worktree>/.bazelrc
```

Only the tracked regular `.bazelrc` whose bytes equal captured `HEAD` is
loaded. The output user root, output base, action cache, repository cache, and
symlink prefix retain ADR 0052's existing bounded `.scratch/` policy. The
monitor constructs every child environment from empty and adds only fixed
tool, locale, worktree, bounded-output, bookkeeping, and repin-control values.
Cache, cloud, proxy, and agent credentials and inherited repin controls remain
absent. The repin controls exist only for the repin invocation and select
exactly `broker`.

On control-pipe EOF, handled signal, timeout, or failure, the monitor sends
TERM to the worker group, waits the committed fixed grace, sends KILL, and
reaps every descendant before releasing the lock. A survivor after the KILL
poll bound leaves the monitor alive and the lock held; it never turns a
nonempty session into release. On normal completion the monitor closes and
reaps the anchor, proves both the worker group and its subreaper child set
empty, and only then performs the final Git census and releases the lock. No
repository-owned child or descendant may outlive the monitor or the lock.
Tests kill the CLI and, separately, a Bazel leader with a surviving
descendant, prove the lock remains contended until that descendant is dead and
reaped, and prove the next run then proceeds.

Independent integrations plant rejecting directives in system, home, and
ambient workspace rc sources and an accepting sentinel only in the explicitly
named committed `.bazelrc`. The poison is never observed. Argument-shape tests
also require all four rc options on every repin, query, build, and identity
invocation, not only the first.

This is process containment, not a host sandbox. There is no mount-namespace
or fresh-root security claim. Bazel actions are trusted same-user contributor
code and retain the ordinary host access current Cargo tooling has. A
concurrent adversarial process under the same uid can race files and is outside
the threat model. Repository-owned writers and cleanup are serialized by the
OFD lock; accidental movement by other tools is caught by the before and after
checks.
An adversarial same-uid process that does not use repository commands and
mutates outside this protocol remains a non-goal.

Bazel writes `bazel/cargo/broker.lock` directly. There is no rollback or
second publication step.

### 6. Validate the direct result and leave failure recoverable

After every worker is terminated and reaped, while the monitor still holds the
OFD lock, repin requires:

- `HEAD` still equals the captured commit;
- every governed input still equals its captured `HEAD` bytes;
- the full Git census, including tracked, staged, ordinary untracked, and
  ignored objects, has no change outside the exact command-owned roots and
  `bazel/cargo/broker.lock`;
- `broker.lock` is again a regular non-symlink with `st_nlink == 1`;
- the lock parses and the selected hub resolves;
- locked offline metadata for the generated workspace succeeds;
- real Bazel query and representative builds resolve the actual `@broker`;
  and
- `L` and `R` pass every exact comparison against `A` and each other.

After subtracting changes beneath the exact command-owned roots, an empty
changed-object set is a successful already-current no-op. A singleton
containing only `broker.lock` is successful only after all semantic checks
pass. No other singleton or set is accepted. Any other tracked, staged,
ordinary untracked, or ignored creation, removal, byte change, type change,
mode change, symlink-target change, index change, link-count change, parse
failure, or projection mismatch is failure. Independent post-admission
injections cover every object class and movement kind.

Independent spoke mutations alter package identity, source, checksum, git
revision, feature, target identity or kind, alias, edge kind, condition, or
edge feature in actual `broker.lock` while `@broker` and the generated witness
remain unchanged, and separately in actual `@broker` while `broker.lock` and
the witness remain unchanged. Each side must fail its own authoritative
comparison and the symmetric actual-to-actual comparison where the field is
shared.

A Bazel error, validation failure, handled termination, or killed CLI may
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
| Untracked path outside the exact command-owned roots | Refuse before Bazel with the unrelated-path remedy |
| Ignored path outside the exact command-owned roots | Refuse before Bazel with the unrelated-path remedy |
| State only inside the exact pre-existing command-owned roots | Permitted subject to ADR 0052's ownership and bounds |
| `gen-bazel --check` fails | Refuse before Bazel; create no check state |
| `broker.lock` is absent, staged, dirty, linked, replaced, non-regular, or differs from `HEAD` | Emit the fixed broker-lock code, exact restore, and exact rerun before the generic census |
| Writer lock is contended | Refuse; wait for the repository mutation command and rerun |
| CLI or Bazel leader dies while a descendant lives | Monitor kills and reaps the session while retaining the lock |
| Child fails, is terminated, or a postcheck fails | Failure; broker lock may be partial and the operator uses the exact restore command |
| `HEAD`, index, governed input, or another repository path moves | Failure; preserve evidence, restore only broker lock, resolve other movement, rerun |

## Required validation

Before amended Spec 003 W0 can close, enforcing carriers must prove:

1. Four hub tokens, four authoritative locks, four Bazel-side locks, the
   separate `packages/Cargo.guest.lock`, the three-lock supply-chain scope,
   and the complete cache-key input set are exact.
2. `gen-bazel --check` has exact byte, census, projection,
   declaration-ledger, and before/after tracked, staged, ordinary-untracked,
   and ignored identity on passing and failing runs, with no bookkeeping or
   scratch creation.
3. Each A, W, L, and R projection has independent missing, extra, and empty
   package and edge negatives. Independent package identity, source, checksum,
   git revision, feature, target identity and kind, alias, edge kind,
   condition, feature, locked-metadata, actual-lock identity, actual-repository
   identity, and direct cross-spoke negatives fail exactly once at their named
   guards.
4. Every declaration-ledger class has independent missing, extra, duplicate,
   wrong-reason, and authoritative-empty coverage with exact-once enforcement.
5. Byte-current and stale required states in the index or worktree refuse with
   the exact commit-together remedy and spawn no child.
6. Unrelated staged, unstaged, ordinary untracked, and ignored admission and
   post-admission states refuse without a Bazel result. Empty state and state
   confined to exact pre-existing command-owned roots succeed.
7. Broker-lock preflight runs first and independently rejects every tracked,
   type, symlink, link-count, replacement, index, worktree, and HEAD-byte
   fault with the fixed code, exact restore, and exact rerun.
8. The external OFD lock passes first-create, stable-reuse, writer-contention,
   cleanup-refusal, symlink, replacement, and interruption tests; cleanup and
   all repository writers share it, while check mode creates none.
9. CLI-death and leader-death tests retain the lock through bounded TERM,
   KILL, descendant death, and full reap; the next run proceeds only
   afterwards. Every Bazel invocation is sequential under `--batch` in the
   monitor-owned group.
10. System, home, and workspace rc poison is ignored; only captured committed
    `.bazelrc` loads. The closed environment contains no cache, cloud, proxy,
    agent, or inherited repin credential.
11. HEAD and complete governed inputs remain stable, and the full tracked,
    staged, ordinary-untracked, and ignored census changes outside exact
    command-owned roots by only `broker.lock` or by nothing.
12. Actual `broker.lock` and actual `@broker` each equal the authoritative
    representable projection and each other symmetrically, with independent
    spoke mutations.
13. Clean already-current repin succeeds as a no-op; child failure, handled
    termination, killed CLI, and every semantic postcheck failure leave no
    success claim and are recoverable by the exact restore command.
14. Refusals print only a stable code, fixed path-free message, capped count,
    and exact local inspection command; adversarial names and counts cannot
    enter default output.
15. Exact nonempty F, B, and M censuses fail independent missing, extra, and
    empty mutations before both first-party cross-edge directions and both
    direct cross-spoke directions are checked.
16. Real Bazel query and representative builds reproduce the target,
    repository, F, B, and M censuses and consume the committed witness.
17. ADR 0052's carrier map remains total and unambiguous, with planted missing
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
- Stable user-owned bookkeeping survives worktree scratch reclamation; cleanup
  now contends with generation and broker repin instead of replacing its lock.
- A monitor process, rather than the invoking CLI or one Bazel leader, retains
  exclusion until every repository-owned descendant is gone.
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
boundary. The stable bookkeeping lock serializes repository-owned writers and cleanup,
and exact before and after checks catch accidental movement.

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
6. Repository writers and cleanup share one stable external OFD lock; the
   broker monitor holds it through complete descendant reap and postchecks.
7. Broker repin admits only clean `HEAD`, index, worktree, untracked, and
   ignored state outside the exact pre-existing command-owned roots.
8. Every broker Bazel invocation runs directly and sequentially in that current
   worktree with `--batch`, ambient rc discovery disabled, only committed
   `.bazelrc`, a closed credential-free environment, and monitor-owned
   process-group containment.
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
