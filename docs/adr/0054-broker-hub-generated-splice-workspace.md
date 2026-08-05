# ADR 0054: A generated splice workspace for the privileged broker's Bazel dependency hub

- Status: Proposed
- Date: 2026-08-05
- Refines and corrects: [ADR 0052](0052-bazel-rust-build-and-test.md),
  which establishes the Bazel Rust migration and the scoped repin command.
  This record preserves its independent hubs and authoritative Cargo inputs,
  corrects its lock inventory, and decides how the `broker` hub obtains a
  spliceable workspace and publishes its Bazel-side lock.
- Related: [ADR 0009](0009-rust-toolchain-msrv-and-supply-chain.md), which
  establishes the aggregate Cargo supply-chain policy; ADR 0052 section 6,
  which applies that policy separately to the main, broker, and guest-shell
  locks; [ADR 0002](0002-non-root-daemon-and-privileged-broker.md) and
  [ADR 0015](0015-daemon-only-clean-break.md), which make the broker the
  privileged dependency closure that must remain separately auditable.
- Scope: the `broker` `crate_universe` hub, its generated splice workspace,
  ownership of generated Bazel inputs and `bazel/cargo/broker.lock`,
  first-party broker variants, and the checks required before Spec 003 W0 can
  close.
- Non-scope: runtime behavior, broker operations, Nix packaging, the other
  three hubs, and implementation. This record changes no code, Spec 003 plan,
  task, or contract.
- Implementation prerequisite: a Spec 003 amendment is pending. Its plan,
  tasks, ownership map, contracts, and validation commands must adopt this
  decision before implementation resumes.

## Context

ADR 0052 assigns one `crate_universe` hub to each Cargo workspace. The
authoritative hub/workspace lock set has four members:

1. `packages/Cargo.lock` for `main`;
2. `packages/d2b-priv-broker/Cargo.lock` for `broker`;
3. `packages/d2b-guest-shell-runner/Cargo.lock` for `guest-shell`;
4. `tests/tools/no-bash-ast-walker/Cargo.lock` for `walker`.

`packages/Cargo.guest.lock` is separate. It is a generated guest-workspace and
cache-key input, not a hub/workspace lock and not a fifth hub.

The broker is a standalone Cargo workspace. It path-depends on five packages
that are members of the separate main workspace: `d2b-contracts`, `d2b-core`,
`d2b-host`, `d2b-realm-core`, and `d2b-realm-provider`. Cargo resolves that
layout, but `crate_universe` 0.73.0 cannot splice it. Supplying only the broker
manifest relocates it without its path dependencies. Supplying the broker and
path manifests together is refused because they belong to different
workspaces.

Merging the workspaces would dissolve the broker's independently pinned and
audited closure. Binding broker targets to main-hub first-party targets would
compile the privileged binary against the wrong resolve. Neither is
acceptable.

A generated workspace containing the broker member and its realized
path-dependency closure does splice successfully. Its lock mirror satisfies
locked offline Cargo metadata when it includes the broker member's test
dependencies and excludes inactive optional and test-only dependencies of
non-member path packages. This is the smallest workaround that preserves the
standalone workspace and authoritative broker lock.

The generated tree is only useful if the bytes checked are the bytes Bazel
consumes. A check followed by in-place Bazel execution in the contributor
worktree leaves both the inputs and the permitted output pathname replaceable.
Repin therefore consumes an exact committed snapshot and publishes only the
validated lock back to the contributor worktree.

The existing Spec 003 plan, tasks, and
`contracts/workspace-and-tool-pinning.md` predate this decision. They do not
encode the generated workspace, the two-command ownership boundary, committed
snapshot repin, or the checks below. The required Spec 003 amendment is
pending; implementation remains parked.

## Decision

### 1. Preserve the standalone broker workspace and four-lock inventory

`packages/d2b-priv-broker/Cargo.toml` remains a standalone Cargo workspace,
and `packages/d2b-priv-broker/Cargo.lock` remains its independent
authoritative lock. Neither `cargo xtask gen-bazel` nor Bazel may edit either
file.

The `broker` hub uses:

- `packages/d2b-priv-broker/Cargo.lock` as `cargo_lockfile`;
- `bazel/cargo/broker.lock` as its Bazel-side lock;
- `skip_cargo_lockfile_overwrite = True`;
- the committed generated workspace under
  `bazel/cargo/broker-workspace/` as its manifest set.

The main, guest-shell, and walker hubs keep their own authoritative workspace
locks and Bazel-side locks. `packages/Cargo.guest.lock` stays outside that
inventory as a generated guest-workspace and cache-key input.

### 2. Generate an exact resolution witness

`bazel/cargo/broker-workspace/` is a tracked generated resolution witness. It
contains:

- a workspace root manifest;
- one generated package manifest and inert source target for the broker
  member and each package in its realized first-party path closure;
- a `Cargo.lock` byte-identical to
  `packages/d2b-priv-broker/Cargo.lock`;
- a `BUILD.bazel` exporting the exact manifest set and lock mirror.

The inert sources are resolution inputs, never first-party compilation
inputs.

The generator constructs two canonical projections.

The authoritative projection is derived from locked offline Cargo metadata
for the authoritative broker workspace, its consumed manifests, and its lock.
For every realized node and edge it records:

- package identity;
- normalized repository-relative path for a path package, or normalized
  source kind and source identity for a non-path package;
- registry checksum;
- for git sources, canonical URL, precise revision, and checksum when the
  lock records one;
- the exact resolved feature set;
- each applicable target's identity and kind;
- each realized dependency edge's source and destination identities,
  dependency kind, and normalized target condition.

The generated witness projection is derived independently by running locked
offline Cargo metadata over the generated workspace. It removes the synthetic
workspace root, maps generated package paths back through the generator's
closed path map, and removes only the declared inert stub targets. The two
projections must be symmetrically equal. Missing and extra nodes, fields,
targets, features, or edges fail.

Declarations intentionally not represented as realized fields are checked
separately rather than hidden by projection. The generator emits an exact
declaration ledger for inactive optional dependencies, excluded non-member
dev-dependencies, and synthesized inert targets. The check proves:

- every omitted optional dependency is declared optional and has no realized
  broker edge, and every such inactive declaration is listed exactly once;
- every excluded non-member dev-dependency is non-realized and listed exactly
  once;
- every synthesized target matches the closed inert-target template and the
  exact package census, and no inert target is a first-party compile input;
- representable feature and target declarations remain exactly equal after
  path normalization.

The generated lock mirror is checked separately for byte equality. The
generated root must also pass:

```text
cargo metadata --manifest-path bazel/cargo/broker-workspace/Cargo.toml --locked --offline
```

Independent planted negatives alter only one of feature selection, target
identity or kind, dependency edge kind or condition, source identity,
checksum, or offline metadata availability. Each must fail its named guard.
A names-and-versions comparison is insufficient.

### 3. Keep two commands and one writer per output

`cargo xtask gen-bazel` is the sole writer of
`bazel/cargo/broker-workspace/**` and all other generated Bazel inputs in its
ownership manifest. Per-hub Bazel-side locks are excluded from that manifest.
Hand edits and repin writes to generator-owned paths are forbidden.

`cargo xtask gen-bazel --check` is strictly read-only. It computes expected
bytes, the output census, the witness projections, and the declaration ledger
without creating a lock, scratch path, temporary file, or transaction state.
It fails on a missing, extra, byte-different, or semantically different
output. It never invokes the mutation form and never repairs drift.

Passing and failing `--check` tests snapshot the complete fixture tree,
including tracked bytes and census and seeded ignored, lock, scratch, and
temporary state. The before and after snapshots must be byte-identical. A
clean fixture separately proves that neither outcome creates any state.

The mutating generator and repin publication both take the same exclusive,
close-on-exec open file description lock under the owned ignored
`.scratch/bazel-mutations/` namespace. `gen-bazel --check` does not take or
create that lock. Make and workflows may invoke only `gen-bazel --check`.

### 4. Make the review and commit boundary explicit

The supported dependency-update workflow is:

1. edit the authoritative Cargo inputs;
2. run `cargo xtask gen-bazel`;
3. review and commit together the complete changed input/output set: every
   relevant Cargo manifest, every authoritative lock change, any changed
   `packages/Cargo.guest.lock`, and every changed path in the generator's
   exact output ownership manifest, including the complete generated broker
   workspace;
4. run `cargo xtask bazel-repin --hub broker`;
5. review and commit the resulting `bazel/cargo/broker.lock`.

The two mutation commands retain disjoint ownership. Repin never generates,
copies, repairs, or publishes the generated subtree.

Before any Bazel child is spawned, broker repin:

- captures one commit object id from `HEAD` and proves a second read is equal;
- requires the original index and all non-ignored worktree state to be clean;
- requires every authoritative input and every path in the exact generator
  output census to be a tracked regular non-symlink present in that `HEAD`;
- runs the broker slice of `gen-bazel --check` in its strictly read-only form;
- records the original index digest, broker-lock bytes and inode identity, and
  anchored `bazel/cargo` parent identity as the publication prestate.

A generated tree that is byte-current but changed only in the worktree or
index is still uncommitted and is refused before Bazel. Tests cover current
unstaged input/output changes, current staged input/output changes, an
incomplete mixed commit boundary, and the corresponding fully committed
success case.

Every stale, missing, extra, byte-different, untracked, staged, or unstaged
required input refuses with this remedy:

```text
run `cargo xtask gen-bazel`, review it, commit the authoritative Cargo inputs and all generated outputs together, then rerun `cargo xtask bazel-repin --hub broker`
```

The explicit closed-set `--hub` argument remains required. Repin is reachable
from neither Make nor a workflow, and no gate or build target sets repin
controls.

### 5. Run Bazel only in a bounded committed snapshot

Repin creates one detached Git worktree at the captured commit, never from
contributor worktree bytes. Its sole namespace is the user-owned mode-0700,
no-symlink `.scratch/bazel-mutations/broker-repin/` namespace. A fixed
lifecycle OFD lock admits one run and one worktree slot. The worktree and all
Bazel home, temporary, repository, output-base, and symlink-prefix state live
on one 16 GiB, 262144-inode bounded temporary filesystem mounted at that slot.
Those are hard limits, not cleanup targets or configurable soft warnings.

The Bazel launcher enters a nested mount and PID namespace. It exposes the
snapshot read-write at the fixed path `/work`, required tools and the Nix
store read-only, and private home, temporary, and output paths inside the
bounded filesystem. It does not mount the original worktree or common Git
directory, masks the snapshot's `.git` administrative file, mounts a private
`/proc`, and removes the original worktree path from arguments, environment,
response files, rc files, and inherited descriptors. If that isolation cannot
be established, repin refuses. No Bazel child can resolve or receive the
original worktree path.

Every Bazel invocation uses `--batch`; no server is permitted. The namespace
leader sets `PR_SET_PDEATHSIG`, verifies its parent after setting it, starts a
dedicated process group, and is held by pidfd. On a handled signal or failure,
the supervisor sends the group a bounded graceful termination, then kills and
reaps survivors. Death of the PID-namespace leader kills all remaining
processes in that namespace. No child or batch process may outlive an
ambiguous parent result.

Bazel's repin controls exist only in that child environment. Bazel may change
only `/work/bazel/cargo/broker.lock`; symlinks and all other output state are
directed outside `/work` but remain inside the bounded filesystem. After the
child exits, an exact tracked-path comparison against the captured commit
refuses any second changed path.

Handled exits reap children, remove the exact Git worktree registration,
unmount the bounded filesystem, and clear the fixed slot. `SIGKILL` can leave
only the fixed control record and Git administrative entry; the temporary
mount and its bytes disappear with the namespace. On the next run, while
holding the lifecycle lock, repin reclaims only a slot whose ownership record,
path, commit, Git worktree registration, uid, and mode all match. It first
proves that the recorded `(boot id, pid, start time, PID namespace)` process
identity and process group are not live. A live child, unknown entry, symlink,
identity mismatch, or second slot refuses without recursive deletion or
global `git worktree prune`.

Success, Bazel failure, validation failure, handled termination, supervisor
kill, stale-worktree recovery, size exhaustion, inode exhaustion, and a live
recorded child all have tests. The count remains one and the temporary
filesystem enforces the size and inode bounds in every case.

### 6. Validate the result in the snapshot before single-file publication

Still inside the isolated snapshot, repin validates:

- the changed-path set is exactly `bazel/cargo/broker.lock`;
- the lock parses and the broker hub resolves;
- locked offline metadata for the generated workspace still succeeds;
- real Bazel query and representative build checks resolve `@broker`;
- the actual `broker.lock` source projection and the materialized `@broker`
  repository source projection each equal the authoritative broker Cargo-lock
  projection symmetrically.

The authoritative source projection is the exact third-party package set. A
registry entry records package identity, normalized registry source, and
registry checksum. A git entry records package identity, canonical URL,
precise revision, and checksum when present. Both the Bazel-side lock and the
materialized repository must contain neither a missing nor an extra entry or
field. Independent tests mutate source and checksum in `broker.lock` and in
the realized repository while leaving the generated witness unchanged. Each
mutation must fail the corresponding comparison.

Only the validated broker-lock bytes may cross back. Before publication,
repin acquires the shared generator/repin OFD writer lock, then rechecks the
original `HEAD`, index digest, clean worktree, exact required committed set,
read-only generator check, anchored parent identity, and broker-lock
prestate. If any original state moved, the temporary result is discarded and
repin refuses.

Publication uses this closed protocol:

1. Open the original worktree once, then resolve `bazel/cargo` beneath that
   descriptor with no symlink or magic-link traversal. Re-resolve it and
   require the same directory identity recorded before Bazel.
2. Open `broker.lock` relative to that parent. Its directory entry and opened
   descriptor must identify the same regular, non-symlink, single-link file
   with the recorded uid, mode, device, inode, size, and byte digest.
3. Recover or refuse any fixed publication residue before proceeding. Create
   the fixed owned sibling candidate with `O_CREAT | O_EXCL | O_NOFOLLOW |
   O_CLOEXEC`; no random or caller-selected name is accepted.
4. Call `posix_fallocate` for the complete final length before writing. An
   `ENOSPC`, quota, or size failure removes only the owned candidate and
   refuses before exchange. Fill it with checked short-write and `EINTR`
   handling, set the live file's mode, `fsync` it, and read back its digest.
5. With no path re-resolution, require the live file's prestate once more and
   atomically exchange the live and candidate names with anchored
   `renameat2(RENAME_EXCHANGE)`. Unsupported exchange or cross-filesystem
   behavior is a refusal, not a replace fallback.
6. `fsync` the parent, recheck that `HEAD` and index are unchanged and that
   only `broker.lock` differs from the clean original, then unlink the old
   file now at the candidate name and `fsync` the parent again.

A failed post-exchange state recheck is not success. If the live and candidate
still form the exact recorded `(new, old)` pair, repin exchanges them back,
syncs the parent, discards the temporary result, and refuses. If either name
contains unmatched bytes or identity, repin preserves the unmatched live file,
quarantines only verified owned residue within the fixed bound, and refuses.

A fixed, fsynced receipt in the bounded control namespace records the captured
commit, parent identity, prestate digest, validated digest, and publication
phase. It is written before candidate creation and retired only after the
second parent sync. Recovery holds both OFD locks and compares actual
live/candidate bytes and identities with that receipt. It can delete an
unexchanged owned candidate, exchange an exact `(new, old)` pair back when
publication was not durable, or finish cleanup for a durably published new
lock. Any other combination preserves an unmatched live file, moves only
verified owned residue to one fixed quarantine slot, and refuses. An occupied
quarantine slot refuses; residue cannot grow without bound.

Tests place barriers before and after candidate creation, preallocation,
candidate sync, exchange, each parent sync, cleanup, and receipt retirement.
They also replace the live file or parent, introduce symlinks, exhaust space,
move `HEAD`, alter the index, run the generator writer concurrently, and kill
the publisher at each barrier. The only successful original-tree change is
the validated regular `bazel/cargo/broker.lock`.

### 7. Compile the broker closure as library-only variants

The five non-member path packages are compiled twice. Their ordinary variants
use the main hub; their `-broker` variants use the broker hub. Every broker
path-dependency variant is a library target only, with no `rust_test`,
doctest, or other test target.

Tests for those packages remain owned by their ordinary main-workspace
variants. Broker-hub tests exist only for members of the broker Cargo
workspace. Today that member is `d2b-priv-broker`.

### 8. Derive exact B and M sets before checking isolation

`F` is the complete generated first-party target set. Its expected form comes
from the generator's exact target inventory, and its actual form comes from a
real Bazel query.

`B_expected` is derived from locked broker metadata and the generator mapping.
It is exactly:

- the five path-dependency library variants
  `d2b-contracts-broker`, `d2b-core-broker`, `d2b-host-broker`,
  `d2b-realm-core-broker`, and `d2b-realm-provider-broker`; plus
- every generated target of every broker workspace member.

`M_expected` is exactly `F_expected - B_expected`; it is not a separately
curated list. Before any edge predicate runs, checks require symmetric
equality for `F`, B, and M, nonempty B and M, an empty B/M intersection, and
`B union M == F`. Independent missing, extra, and empty mutations for B and
for M fail before isolation predicates.

The primary isolation check covers first-party `deps` and
`proc_macro_deps`:

- the first-party portion reachable from B stays within B;
- the first-party portion reachable from M stays within M.

Each direction has an independent planted cross-edge.

A supplemental check validates direct third-party spokes. A B target may use
third-party crates only through `@broker//`. An M target's direct spoke must
match its independently derived hub owner and may not use `@broker//`.
Independent mutations bind a B target directly to `@main//` and an ordinary M
target directly to `@broker//`; each fixture first proves the first-party
guard still passes, then proves the spoke guard fails. Guest-shell and walker
targets retain their own hubs.

## Required validation

Before the amended Spec 003 W0 can close, its owners and enforcing carriers
must prove:

1. `gen-bazel --check` exact byte, census, projection, declaration-ledger, and
   no-state behavior on passing and failing runs.
2. Independent feature, target, dependency-edge, source, checksum, and locked
   offline-metadata planted negatives.
3. Refusal of byte-current unstaged, staged, and incomplete input/output sets,
   with the exact commit-together remedy and no Bazel child.
4. Success only from a stable clean original and a complete committed
   input/output set at the captured `HEAD`.
5. Exact detached-worktree commit identity and the one-slot, 16 GiB,
   262144-inode lifecycle bounds across success, failure, termination, kill,
   and stale recovery.
6. `--batch`, child-death, pidfd, process-group, namespace, path-hiding, reap,
   and no-survivor behavior.
7. A temporary changed-path set containing only `broker.lock`.
8. Exact symmetric source, revision, and checksum identity for actual
   `broker.lock` and `@broker`, with independent source and checksum mutations
   while the generated witness is unchanged.
9. Original-state movement and concurrent generator publication discard the
   temporary result and refuse.
10. Anchored regular-file, no-symlink, same-prestate, preallocation, exchange,
    sync, cleanup, crash-recovery, and bounded-quarantine publication.
11. Exact nonempty F, B, and M censuses, with missing, extra, and empty
    negatives before edge predicates.
12. Both first-party cross-edge directions and both direct cross-spoke
    directions, with the spoke negatives proving the first-party guard passes.
13. Real Bazel query reproduces the manifest, target, F, B, M, and repository
    censuses, and real representative builds use the committed witness.
14. ADR 0052's exact carrier checks remain total and unambiguous, with planted
    missing and extra carrier mutations.

Unit tests or generated maps alone do not close real Bazel query, repository,
build, process, publication, or carrier items.

## Consequences

- The privileged broker keeps a small independently pinned and audited Cargo
  closure.
- Dependency updates require two repository-owned mutation commands and two
  review points. That cost makes authority and publication explicit.
- Repin pays for an isolated detached checkout and bounded temporary
  filesystem. It never renders from contributor worktree bytes.
- The contributor worktree receives only one validated file through an
  anchored crash-recoverable exchange.
- The five shared libraries compile once for the main resolve and once for
  the broker resolve; their tests do not duplicate.
- Spec 003 remains blocked pending an amendment to its plan, tasks, ownership
  map, contracts, and validation commands.

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

Rejected. Manual declarations drift on features, sources, checksums, targets,
and edges.

### Make broker repin synchronize the generated subtree

Rejected. A self-synchronizing repin gives two commands ownership of generated
inputs, hides the required review-and-commit boundary, and widens a lock-only
operation into subtree publication.

### Let Bazel repin in the contributor worktree

Rejected. A preflight check does not freeze later reads, and a pathname
allowlist does not stop replacement of the permitted output. In-place Bazel
execution can consume uncommitted bytes, overwrite a replaced path, leave a
server alive, and modify more than the reviewed lock.

### Copy the current worktree into scratch

Rejected. A copy preserves the exact uncommitted and check/use race this
decision closes. The snapshot is checked out from the captured commit object.

### Publish with ordinary pathname replacement

Rejected. It follows mutable path components, provides no same-prestate proof,
and gives ENOSPC and crashes an ambiguous point between candidate and live
state. Anchored exchange keeps both inodes available until durable cleanup.

### Use unbounded temporary directories or global worktree pruning

Rejected. Interrupted Bazel runs would accumulate disk use, and global prune
can delete another contributor's worktree. One fixed bounded slot has a
targeted ownership proof.

## Invariants this decision creates

1. The broker remains a standalone Cargo workspace with its own authoritative
   lock.
2. There are four hub/workspace locks: main, broker, guest-shell, and walker.
   `packages/Cargo.guest.lock` is a separate generated and cache-key input.
3. The broker hub splices only the committed generated witness while reading
   the authoritative broker Cargo lock with overwrite disabled.
4. `gen-bazel` alone writes generated Bazel inputs; `gen-bazel --check` is
   strictly read-only and creates no state.
5. A dependency update commits all changed authoritative Cargo inputs and all
   changed generator outputs together before repin.
6. Broker repin admits only a stable clean original with the complete required
   set committed at one captured `HEAD`.
7. Bazel runs in `--batch` mode only in one isolated, bounded detached
   worktree at that exact commit and cannot resolve the original worktree.
8. Bazel may change only the temporary worktree's
   `bazel/cargo/broker.lock`.
9. The actual Bazel lock and materialized `@broker` repository equal the
   authoritative Cargo-lock source, revision, and checksum projection
   symmetrically.
10. Publication rechecks original state under the shared generator/repin OFD
    lock and publishes only one validated regular file through anchored
    no-symlink exchange.
11. Candidate preallocation precedes publication, and fixed crash residue,
    stale worktrees, quarantine, size, inode, and count are bounded.
12. Witness equality covers every representable realized field, while omitted
    declarations and inert synthesized targets have separate exact checks.
13. Broker path-dependency variants are library-only; their tests remain
    main-owned.
14. B is exactly derived, M is exactly the complete generated first-party set
    minus B, and symmetric nonempty censuses precede edge predicates.
15. First-party isolation is primary and direct-spoke isolation is
    supplemental; independent negatives prove both.
16. Neither mutating command is reachable from Make or workflows.
17. Real Bazel query, repository, build, process, publication, and exact
    carrier evidence are required before Spec 003 W0 closes.
18. Spec 003's amendment remains a prerequisite to implementation.

## References

- [ADR 0052](0052-bazel-rust-build-and-test.md), especially sections 2, 4, 5,
  and 6 and its scoped repin decision
- [ADR 0009](0009-rust-toolchain-msrv-and-supply-chain.md), the aggregate
  supply-chain policy applied per main, broker, and guest-shell lock by ADR
  0052 section 6
- `rules_rust` 0.73.0 `crate_universe` splicing and lock-digest behavior
- `packages/Cargo.toml` and the four hub/workspace locks listed above
- `packages/Cargo.guest.lock`, the separate generated and cache-key input
- `specs/003-adr052-bazel-rust/plan.md`
- `specs/003-adr052-bazel-rust/tasks.md`
- `specs/003-adr052-bazel-rust/contracts/workspace-and-tool-pinning.md`
