# Removal proofs: the three W5 crate removals (FR-023)

| Field | Value |
| --- | --- |
| Satisfies | FR-023, scoped by FR-060 |
| Wave performing the removal | W5 |
| Snapshot proofs were run against | `a7f4a6a4` on `adr046-w5-audit-docs` |
| Paths proved | `packages/d2b-daemon-access/`, `packages/d2b-host-providers/` (with `packages/d2b-host/src/runtime_provider.rs`), `packages/d2b-userd/` |
| Companion record | [`removal-proof-inventory.md`](./removal-proof-inventory.md) |
| Source of truth for dispositions | `docs/specs/ADR-046-current-code-migration-map.md` |

## 1. Why this file exists, and what it is not

[`removal-proof-inventory.md`](./removal-proof-inventory.md) counts the rows
that **lack** a proof. It is a census. It deliberately contains no proofs,
because a census that also carried evidence would let a reader mistake the
absence of a row for the presence of a proof.

This file is the other half for W5 only: the three superseded paths this wave
actually deleted, each with the evidence FR-023 demands. It exists because
`ADR046-W5` deleted three crates and the program had, until this record, only
*rationale* for those deletions in
[`plan.md`](./plan.md)'s Recorded-drift section. Rationale is not a removal
proof. FR-023 requires three things per path, and a paragraph explaining why a
crate looked unused satisfies none of them:

1. the replacement is integrated and covered by tests;
2. an explicit removal proof passes; and
3. the removal lands in its own change, separate from the change that
   introduced the replacement.

Clause 2 is the one that was missing, and it is the one this file supplies.

**What a proof must contain here.** A passing check that names the specific
superseded path, run against a named commit, whose output is recorded rather
than asserted. Two directions are required and neither alone is sufficient:

- **Pre-removal reachability.** Run *before* the deletion, at the removal
  commit's parent, to establish that nothing reached the path. Without this a
  reader cannot tell a safe removal from a removal that broke a caller and was
  papered over by deleting the caller in the same commit.
- **Post-removal zero residual.** Run at the current snapshot, to establish
  that no reference survives anywhere the build, the packaging, or the tests
  can see it. Without this a reader cannot tell a completed removal from a
  half-removal whose dangling reference happens not to be compiled today.

## 2. The command set, and why each command is there

Every command below is verbatim and runnable from the repository root. The
crate-name and module-name spellings differ (`d2b-daemon-access` in Cargo and
Nix, `d2b_daemon_access` in Rust), so both are scanned; a scan of one spelling
only is the standard way this class of check fails open.

```
# (A) pre-removal reverse dependency, Cargo surface, at the removal parent
git grep -l -e '<crate>' <parent> -- 'packages/*/Cargo.toml'

# (B) pre-removal reverse dependency, Rust surface, at the removal parent
git grep -l -e '<crate_underscored>' <parent> -- 'packages/*/src' 'packages/*/tests'

# (C) post-removal: the path itself is gone from the index
git ls-files 'packages/<crate>/*'

# (D) post-removal: no Rust reference anywhere in the workspace
git grep -l -e '<crate_underscored>' -- 'packages/*/src' 'packages/*/tests' 'packages/*/benches'

# (E) post-removal: no reference on any build, packaging, or test surface
git grep -l -e '<crate>' -- packages nixos-modules pkgs nix tests flake.nix Makefile

# (F) post-removal: the workspace still resolves without the crate
cd packages && cargo metadata --no-deps --format-version 1 --offline
```

(A) and (B) are the pre-removal half. (C) through (E) are the zero-residual
half. (F) is the parity floor: a removal that leaves the workspace unresolvable
is not a removal, it is a break, and `cargo metadata` catches a dangling
`members` entry or a dangling `path` dependency without compiling anything.

**On (E)'s path list.** It names `nixos-modules`, `pkgs`, `nix`, `flake.nix`
and `Makefile` explicitly rather than scanning the whole tree, because the whole
tree includes `docs/specs/`, `docs/adr/` and this directory, where the removed
names appear legitimately and permanently as baseline citations. A scan that
included them would never return zero, so it would be quietly downgraded to a
human eyeball pass. See section 6 for why those citations are correct and must
not be edited.

## 3. Proof 1: `packages/d2b-daemon-access/`

| Field | Value |
| --- | --- |
| Removal commit | `0e58a79a` `build: remove daemon-access bootstrap crate` |
| Removal parent | `8fd98153` |
| Migration map rows | line 463 (`DaemonAccessTransport` / `DaemonAccessApi`, ADAPT) and line 480 (the crate, ADAPT), both owned by `ADR046-api-001` |
| Named successor | `packages/d2b-resource-api/src/{service,client,error,adapter}.rs`, `packages/d2b-resource-client/` |
| Successor state | `ADR046-api-001` is `Merged` |

### Pre-removal reachability, at `8fd98153`

```
$ git grep -l -e 'd2b-daemon-access' 8fd98153 -- 'packages/*/Cargo.toml'
8fd98153:packages/d2b-daemon-access/Cargo.toml

$ git grep -l -e 'd2b_daemon_access' 8fd98153 -- 'packages/*/src' 'packages/*/tests'
(no match)
```

The only Cargo mention was the crate's own manifest. No crate in the workspace
declared a dependency on it and no Rust file outside it named it.

The same scan one commit earlier, at `dc85f350` - **before** `8fd98153` removed
the three crates from the `members` array - returns the identical result:

```
$ git grep -l -e 'd2b-daemon-access' dc85f350 -- 'packages/*/Cargo.toml'
dc85f350:packages/d2b-daemon-access/Cargo.toml

$ git grep -l -e 'd2b_daemon_access' dc85f350 -- 'packages/*/src' 'packages/*/tests'
(no match)
```

That second run is the one that matters, and it is why the check is recorded at
two commits rather than one. Checking only at `8fd98153` would prove nothing:
the prep commit had already dropped the crate from the workspace, so a
reverse-dependency scan there could not have found a consumer even if one
existed. Running it at `dc85f350` establishes that the crate was already
unreferenced **while it was still a workspace member**, which is the claim the
removal actually rests on.

### Post-removal zero residual, at `a7f4a6a4`

```
$ git ls-files 'packages/d2b-daemon-access/*' | wc -l
0

$ git grep -l -e 'd2b_daemon_access' -- 'packages/*/src' 'packages/*/tests' 'packages/*/benches' | wc -l
0

$ git grep -l -e 'd2b-daemon-access' -- packages nixos-modules pkgs nix tests flake.nix Makefile | wc -l
0

$ cd packages && cargo metadata --no-deps --format-version 1 --offline
exit 0; 58 workspace packages; no package named d2b-daemon-access
```

### FR-023 clause 3: separate change

`0e58a79a` deletes the crate and touches nothing else except its changelog
fragment and one line of `policy_provider_crates.rs`. The replacement it defers
to, `ADR046-api-001`, was introduced in W0 and merged long before. The removal
is therefore in its own change by construction, not by discipline.

### Verdict

**Proof passes.** The path was unreferenced while it was still a member, it is
gone, nothing names it, and the workspace resolves without it.

## 4. Proof 2: `packages/d2b-host-providers/` and `packages/d2b-host/src/runtime_provider.rs`

| Field | Value |
| --- | --- |
| Removal commit | `15076c77` `host: remove obsolete provider adapters` |
| Removal parent | `0e58a79a` |
| Migration map row | line 479 (the crate, REPLACE, owner `ADR046-primitives-003`) |
| Named successors | `Provider/system-core` substrate, `Provider/runtime-cloud-hypervisor` VM execution lifecycle, `Provider/display-wayland` cross-domain |
| Successor state | **not at parity** - see section 6.1 |

### Pre-removal reachability, at `0e58a79a`

```
$ git grep -l -e 'd2b-host-providers' 0e58a79a -- 'packages/*/Cargo.toml'
0e58a79a:packages/d2b-host-providers/Cargo.toml

$ git grep -l -e 'd2b_host_providers' 0e58a79a -- 'packages/*/src' 'packages/*/tests'
(no match)
```

`15076c77` also deleted `packages/d2b-host/src/runtime_provider.rs`, a module of
a **live** crate, so that module needs its own reachability check rather than
inheriting the crate's:

```
$ git grep -n -e 'd2b_host::runtime_provider' -e 'host::runtime_provider' 0e58a79a -- packages tests
(no match)
```

A bare `runtime_provider` scan at the same commit returns eight files and is
**not** evidence of reachability. Seven of them match the unrelated
`runtime_providers` DTO field, for example
`packages/d2b-core/src/host.rs:122: pub runtime_providers: Vec<RuntimeMetadata>`,
and the eighth is the module's own file. This is recorded because the loose
scan is the one a reviewer reaches for first, and it reads as eight live
consumers when there are none. The module-path scan above is the one that
carries the claim.

### Post-removal zero residual, at `a7f4a6a4`

```
$ git ls-files 'packages/d2b-host-providers/*' 'packages/d2b-host/src/runtime_provider.rs' | wc -l
0

$ git grep -l -e 'd2b_host_providers' -- 'packages/*/src' 'packages/*/tests' 'packages/*/benches' | wc -l
0

$ git grep -l -e 'd2b-host-providers' -- packages nixos-modules pkgs nix tests flake.nix Makefile | wc -l
0

$ git grep -l -e 'd2b_host::runtime_provider' -e 'runtime_provider::' -- packages | wc -l
0

$ cd packages && cargo metadata --no-deps --format-version 1 --offline
exit 0; 58 workspace packages; no package named d2b-host-providers
```

### FR-023 clause 3: separate change

`15076c77` deletes the adapter crate, the `d2b-host` module, and the two
`d2b-host` dependency lines that existed only to serve it
(`d2b-realm-core`, `d2b-realm-provider`). It introduces no replacement.

### Verdict

**Proof passes for the removal. The row does not close.** The path is provably
gone and provably unreferenced, and clause 3 is satisfied. Clause 1 is
satisfied only in the narrow sense that applies here: the crate was an
unreachable adapter, so its deletion removed no operator-facing capability and
there was no behaviour for a replacement to be at parity with. The three
Provider successors the migration map names are **not** at parity, and section
6.1 records that the trait-level rows this crate implemented remain open
against their own owners.

## 5. Proof 3: `packages/d2b-userd/`

| Field | Value |
| --- | --- |
| Removal commit | `442172a5` `guest: remove unused userd stub` |
| Removal parent | `2e92622f` |
| Migration map rows | line 527 (the crate, REPLACE, owner `ADR046-primitives-003`) and line 735 (`d2b userd *` CLI verb, DELETE, same owner) |
| Named successor | fixed user supervisor `Process` under `Provider/system-systemd` user domain |
| Successor state | **not at parity** - see section 6.1 |

### Pre-removal reachability, at `2e92622f`

```
$ git grep -l -e 'd2b-userd' 2e92622f -- 'packages/*/Cargo.toml'
2e92622f:packages/d2b-userd/Cargo.toml

$ git grep -l -e 'd2b_userd' 2e92622f -- 'packages/*/src' 'packages/*/tests'
(no match)
```

`d2b-userd` is the only one of the three with surfaces outside Cargo, so the
Cargo scan alone would have been insufficient. The full pre-removal surface
scan at the same commit:

```
$ git grep -l -e 'd2b-userd' -e 'd2b_userd' -e 'userd' 2e92622f \
    -- flake.nix nix pkgs nixos-modules tests packages/Cargo.guest.lock packages/d2b-contract-tests
2e92622f:flake.nix
2e92622f:nixos-modules/net.nix
2e92622f:packages/Cargo.guest.lock
2e92622f:packages/d2b-contract-tests/tests/policy_contracts.rs
2e92622f:tests/fixtures/guest-rust-workspace/Cargo.toml
2e92622f:tests/golden/api-surface/workspace-metadata.json
2e92622f:tests/migration-ledger.toml
2e92622f:tests/migration-state.d/guest-exec-policy-eval.toml
2e92622f:tests/migration-state.d/guest-exec-runtime-static.toml
2e92622f:tests/unit/nix/eval-cases/guest-exec-policy-eval.nix
```

Ten surfaces, not zero. This is the case the Cargo-only check would have
missed, and it is why (E) exists as a separate command rather than as a
formality after (D). One of the ten is a false positive:
`nixos-modules/net.nix:476` matches inside the comment "useradd/userdel at
runtime" and is not a `d2b-userd` reference at all. The other nine were genuine
packaging, lock, fixture, golden and policy surfaces, and `442172a5` with its
predecessor `2e92622f` clears every one that named the crate.

### The removal left standing negative controls, deliberately

Three bare-`userd` references survive at `a7f4a6a4`, and they are the reason
command (E) scans for the hyphenated crate name rather than the bare token.
None of them is residue:

```
$ git grep -n 'userd' -- packages/d2b-contract-tests tests/unit/nix
packages/d2b-contract-tests/tests/policy_contracts.rs:326:        r"userd",
tests/unit/nix/eval-cases/guest-exec-policy-eval.nix:166:    lib.filter (name: lib.hasInfix "userd" name)
```

The first is the search pattern of an `assert_files_have_no_line` call whose
message reads "exec runtime must not reference legacy user-session code". The
second filters the guest's `systemd.services` attribute names under the comment
"No per-user legacy user-session services exist anywhere anymore". Both are
assertions that the name is **absent**, and both fail closed if it returns.

This is worth recording rather than tidying away. The three removals deleted
code; these two guards are what keep the code from coming back, and a later
reader running a bare `grep userd` will find them and needs to know they are
the seal rather than the leak. `tests/migration-ledger.toml` and the two
`tests/migration-state.d/*.toml` rows likewise carry the token inside prose
describing those same guards.

### Post-removal zero residual, at `a7f4a6a4`

```
$ git ls-files 'packages/d2b-userd/*' | wc -l
0

$ git grep -l -e 'd2b_userd' -- 'packages/*/src' 'packages/*/tests' 'packages/*/benches' | wc -l
0

$ git grep -l -e 'd2b-userd' -- packages nixos-modules pkgs nix tests flake.nix Makefile | wc -l
0

$ grep -c 'userd' packages/Cargo.guest.lock
0

$ cd packages && cargo metadata --no-deps --format-version 1 --offline
exit 0; 58 workspace packages; no package named d2b-userd

$ cargo metadata --manifest-path packages/d2b-priv-broker/Cargo.toml \
    --no-deps --format-version 1 --offline
exit 0
```

The guest workspace is checked separately because `d2b-userd` was a member of
it. `tests/fixtures/guest-rust-workspace/Cargo.toml` now lists six members and
`d2b-userd` is not among them.

### The CLI verb at migration-map line 735 owes nothing

Row 735 schedules `DELETE` of the "`d2b userd *`" CLI verb and classifies it
`production-reachable`. Measured at the migration map's own declared baseline,
`b5ddbed6`, that verb does not exist:

```
$ git grep -ln 'userd' b5ddbed6 -- packages/d2b/
(no match; exit 1)

$ git grep -n 'userd' b5ddbed6 -- docs/reference/cli-contract.md
(no match)

$ git grep -ln 'userd' b5ddbed6 -- packages
b5ddbed6:packages/Cargo.guest.lock
b5ddbed6:packages/Cargo.lock
b5ddbed6:packages/Cargo.toml
b5ddbed6:packages/d2b-contract-tests/tests/policy_contracts.rs
b5ddbed6:packages/d2b-userd/Cargo.toml
b5ddbed6:packages/d2b-userd/src/lib.rs
b5ddbed6:packages/d2b-userd/src/main.rs
b5ddbed6:packages/d2b-userd/tests/fail_closed.rs
```

At the baseline the name appears only in the crate itself, the workspace
manifests and locks, and one policy test. It appears in neither the CLI crate
nor the published CLI contract, and no commit on this lineage has ever added or
removed it from `packages/d2b/src`:

```
$ git log --oneline -S'userd' -- packages/d2b/src
(no commits)
```

**Ruling: row 735 names a path that does not exist and owes no removal proof.**
This is not a waiver. FR-023 binds removals; a row that schedules the deletion
of a surface which was never present schedules no removal. The row is a
migration-map defect - a `production-reachable` classification against a
nonexistent verb - and is recorded as drift in section 6.2 rather than
corrected, because the map is a member specification.

### Verdict

**Proof passes for the removal. Row 527 does not close.** The crate is gone
across every surface it occupied, including the guest workspace and its lock,
and both workspaces resolve. As with `d2b-host-providers`, clause 1 holds only
because the removed code was a stub - `main.rs` exited 78 with "service mode is
not implemented" - so no operator-facing capability was withdrawn. The user
supervisor `Process` successor does not exist; see section 6.1.

## 6. What these three proofs do not discharge

This section exists because the most likely misreading of this file is that
three passing proofs closed three migration-map rows and the successor work
with them. They did not.

### 6.1 Removing a crate is not discharging the rows its symbols owned

`d2b-host-providers` implemented three traits that the migration map schedules
separately, at lines 456, 457 and 458:

| Line | Symbol | Disposition | Owner | State after W5 |
| --- | --- | --- | --- | --- |
| 456 | `HostSubstrateProvider` | REPLACE | `ADR046-primitives-003` | open |
| 457 | `RuntimeProvider` | REPLACE | `ADR046-provider-001` | open |
| 458 | `DisplayProvider` | REPLACE | `ADR046-provider-001` | open |

Deleting the crate that implemented them removed an implementation, not the
obligation to provide a successor at parity. Each row still owes its own proof
when its owner performs its own removal, and `ADR046-primitives-003` and
`ADR046-provider-001` both remain `Planned`. The same holds for `d2b-userd`
line 527's successor: no fixed user supervisor `Process` exists under
`Provider/system-systemd`, and none of the three proofs above claims otherwise.

The honest statement of what W5 achieved is narrow and worth stating plainly:
it deleted three unreachable code artifacts and proved they were unreachable.
It delivered no successor and closed no capability migration.

### 6.2 The `d2b-daemon-access` disposition is drift, and is not corrected here

The migration map gives `d2b-daemon-access` the disposition **ADAPT** at both
line 463 and line 480. An `ADAPT` row schedules no removal, which is why the
crate never appeared in
[`removal-proof-inventory.md`](./removal-proof-inventory.md)'s census of rows
owing a proof. W5 nevertheless deleted it.

That is not a contradiction of FR-023, and the resolution is that both halves
of `ADAPT` happened in order:

- the **adaptation** landed under `ADR046-api-001`, which is `Merged` with
  destination evidence naming
  `packages/d2b-resource-api/src/{service,client,error,adapter}.rs`; and
- the **source crate** was then an orphan carrying no capability, and was
  deleted with the proof in section 3.

What is genuinely wrong is the map's disposition cell, which should record that
the source is retired after adaptation rather than implying it survives. That
cell is in `docs/specs/ADR-046-current-code-migration-map.md`, a member of the
55-spec set. Editing it re-opens that spec's validation and panel evidence and
re-triggers Gate 0 across the whole manifest under FR-056, for one table cell.
Per FR-046 the drift is therefore **recorded and raised, not corrected in
place**. The standing instruction until a dedicated amendment lands: the crate
is removed, this file is its proof, and the ADAPT cell is stale for the source
path only.

### 6.3 The `currentSource` citations naming these crates are correct

`ADR046-api-001` is `Merged` and its `currentSource` reads:

```
`packages/d2b-contracts/src/public_wire.rs`, `broker_wire.rs`;
`d2b-daemon-access/src/lib.rs`; `d2b-realm-router/src/lib.rs`
```

This names a crate that no longer exists, and it is **not** stale. Ten work
items in total cite one of the three removed crates in `currentSource` - two
`Merged` (`ADR046-api-001`, `ADR046-api-002`) and eight `Planned` - and none of
them is stale either:

```
$ jq -r '.items[] | select((.currentSource // "")
    | test("d2b-daemon-access|d2b-host-providers|d2b-userd"))
    | .workItemId + " " + .implementationState' docs/specs/ADR-046-work-items.json
ADR046-api-001 Merged
ADR046-api-002 Merged
ADR046-display-001 Planned
ADR046-exec-004 Planned
ADR046-exec-011 Planned
ADR046-zone-control-004 Planned
ADR046-zone-control-005 Planned
ADR046-zone-control-006 Planned
ADR046-zone-control-010 Planned
ADR046-zone-control-018 Planned
```

`docs/specs/README.md` settles it in two places. Each member spec carries a
`Baseline` field defined as "Exact v3 commit analyzed", and the evidence rule
reads: "Current behavior is cited by exact v3 file, symbol, and baseline
commit." `currentSource` is therefore a **baseline-pinned historical citation**,
not a live-tree assertion. The baseline for
`ADR-046-resource-api-and-authorization` is `b5ddbed6`, and all three removed
crates are present in that tree:

```
$ git ls-tree -d b5ddbed6 packages/d2b-daemon-access packages/d2b-host-providers packages/d2b-userd
040000 tree 80334991e269fdf5def40fa8a1f1db1fd0ffae3f	packages/d2b-daemon-access
040000 tree 4078eb534eeca50927bb1ef1cddfd5502ad233c8	packages/d2b-host-providers
040000 tree c9941c5854451dfa1003f31580064ad82ba112a4	packages/d2b-userd
```

**Ruling: no reconciliation is owed, and none is performed.** Rewriting a
baseline citation to match the current tree would destroy the property that
makes it useful - that a reader can check the claim against the exact commit
the analysis was performed on - and would additionally re-trigger Gate 0 for a
field that is behaving correctly. A future reader who finds a `currentSource`
naming a deleted path should resolve it against that spec's `Baseline`, not
against `HEAD`.

The correct place for the removal to become visible is this file and the
inventory, both of which are program-local and carry no Gate 0 cost.

### 6.4 Shipped prose already reflects the removal

`docs/reference/` mentions `d2b-userd` four times, and every one is a
statement of **absence** that the removal made more true rather than less:

```
$ git grep -n 'd2b-userd' -- docs/reference
docs/reference/guest-control-exec-interactive-tty.md:30:`d2b-guestd`; there is no per-user `d2b-userd` involvement.
docs/reference/guest-control-exec-io-chunked-stdio.md:564:  workload user or `d2b-userd`.
docs/reference/guest-control-exec-io-chunked-stdio.md:575:   trusted runner; there is no `d2b-userd` involvement and no per-user
docs/reference/guest-control-exec-io-chunked-stdio.md:579:   no separate user-session daemon (`d2b-userd` was removed; see
```

Line 579 continues "[ADR 0030](../adr/0030-guest-exec-as-workload-user.md))".

No shipped-doc edit is owed. This was checked rather than assumed, because a
removal that leaves a reference doc describing a live component is exactly the
kind of residue command (E) is scoped to catch and, being prose, would not have
failed any gate.

## 7. Effect on the inventory census

[`removal-proof-inventory.md`](./removal-proof-inventory.md) section 3 is
amended by this record as follows. The counts there were correct when written;
these are the deltas W5 produces.

| Inventory row | Was | Now | Basis |
| --- | --- | --- | --- |
| 3.2 line 479 `d2b-host-providers` | REPLACE, no proof, owner W2 | REPLACE, **proof recorded here**, performed by W5 | FR-060 binds the proof to the removing wave; section 4 |
| 3.2 line 527 `d2b-userd` | REPLACE, no proof, owner W2 | REPLACE, **proof recorded here**, performed by W5 | FR-060; section 5 |
| 3.1 line 735 `d2b userd *` CLI verb | DELETE, no proof, owner W2 | **owes no proof**; path absent at baseline | Section 5, measured at `b5ddbed6` |
| lines 463 / 480 `d2b-daemon-access` | ADAPT, outside the census | **removed in W5, proof recorded here**; disposition drift raised | Section 3, section 6.2 |
| 3.1 lines 456-458 trait rows | REPLACE, no proof | unchanged, still open | Section 6.1 |

Net effect on the outstanding count: 36 rows lacking a proof becomes **33** -
two proved here, one (line 735) retired as naming no path. The two proved rows
move out of the W2 owner column entirely, so the W2 total falls from 6 to 4.
No row is closed by assertion and no successor obligation is discharged.

## 8. The stopping condition, stated so a machine can evaluate it

These proofs are re-runnable, and FR-023 read together with T563 requires them
to hold on the wave candidate snapshot, not merely on the day they were
written. The condition is:

```
test 0 -eq "$(git ls-files \
    'packages/d2b-daemon-access/*' \
    'packages/d2b-host-providers/*' \
    'packages/d2b-userd/*' \
    'packages/d2b-host/src/runtime_provider.rs' | wc -l)" \
&& test 0 -eq "$(git grep -l \
    -e 'd2b_daemon_access' -e 'd2b_host_providers' -e 'd2b_userd' \
    -- 'packages/*/src' 'packages/*/tests' 'packages/*/benches' | wc -l)" \
&& test 0 -eq "$(git grep -l \
    -e 'd2b-daemon-access' -e 'd2b-host-providers' -e 'd2b-userd' \
    -- packages nixos-modules pkgs nix tests flake.nix Makefile | wc -l)" \
&& test 0 -eq "$(git grep -l \
    -e 'd2b_host::runtime_provider' -e 'runtime_provider::' -- packages | wc -l)" \
&& (cd packages && cargo metadata --no-deps --format-version 1 --offline >/dev/null)
```

All five conjuncts return true at `a7f4a6a4`. The W5 gate at T219 and the W8
re-verification at T563 evaluate this expression; a nonzero count or a nonzero
`cargo metadata` exit is a failed removal proof, not a warning.

The expression is deliberately not wired into a `tests/` gate. The drift and
meta gate set is a closed set, the paths it names are gone permanently rather
than being an invariant that could regress from ordinary work, and a
single-purpose gate for three deleted crates would outlive its subject. If a
later wave wants standing enforcement, the place for it is a row in the
existing policy crate, not a new shell gate.
