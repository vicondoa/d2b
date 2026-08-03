# Removal-proof inventory (FR-023)

| Field | Value |
| --- | --- |
| Source of truth | `docs/specs/ADR-046-current-code-migration-map.md` |
| Cross-referenced | `docs/specs/ADR-046-work-items.json`, `docs/specs/ADR-046-implementation-graph.json` |
| Satisfies | FR-023 |
| Status | Inventory of missing removal proofs; each gap assigned to the wave that removes its path |
| Proofs supplied so far | W5 only, in [`removal-proof-w5.md`](./removal-proof-w5.md) |

**This file is a census, not an evidence store.** It counts rows that lack a
proof. When a wave supplies one, the proof itself lives in that wave's own
record and this file records only the resulting change to the count. Section
3.5 carries the W5 deltas.

## 1. Why this exists

FR-023 requires that each superseded path scheduled for removal pass an
**explicit removal proof** before it is removed, and that the removal land in
its own change separate from the change that introduced the replacement. A
disposition of `DELETE` or `REPLACE` in the migration map therefore schedules a
removal, and each such row owes a proof.

This document inventories the rows that do not have one.

**Removal proofs live in two places in the migration map, and both must be read.**
The first is the per-row disposition cell (a `Removal Proof` column, or a proof
stated inline in a row's target or notes cell). The second is the map's own
section 8.2, "Tests Required for Removal Proof", which is a separate table of
five removal targets naming an explicit executable proof for each. A scan that
reads only the per-row cells undercounts the proofed rows and can mark a row
`unassigned-needs-integrator` when section 8.2 already supplies its proof. An
earlier revision of this inventory made exactly that mistake with
`RelayProvider`. Read both places.

## 2. Counts found, versus the claim

The program's working note claimed that the migration map "supplies explicit
proofs for only 3 of its 16 DELETE rows". Counting the real file, and counting
section 8.2 as well as the per-row cells:

| Measure | Count |
| --- | --- |
| `DELETE` disposition rows | **16** |
| `REPLACE` disposition rows | **32** |
| `DELETE` + `REPLACE` rows, total | **48** |
| `DELETE` rows carrying an explicit removal proof | **5** |
| `REPLACE` rows carrying an explicit removal proof | **7** |
| Rows carrying an explicit removal proof, total | **12** |
| Rows scheduled for removal with **no** explicit proof | **36** |

**The claim is correct on the denominator and understated on the numerator.**
The DELETE denominator of 16 is correct. The proofed DELETE count is **5, not
3**. Two separate effects account for the difference. First, the per-realm unit
table records its proof once for three DELETE rows rather than per row, so a
naive per-row scan sees one proof where three rows are covered. Second, section
8.2 supplies the proof for `RelayProvider`, which carries no proof in its own
row.

The claim also stops at DELETE and does not account for the 32 REPLACE rows,
which schedule removals just as DELETE rows do. Only 7 of those 32 carry a
proof, which is where the bulk of the outstanding gap actually is.

### The 12 rows that do carry a proof

Column `Source` records where the proof was found: `row` for a per-row
disposition cell, `§8.2` for the migration map's removal-proof test table.

| Line | Row | Disposition | Source | Proof recorded |
| --- | --- | --- | --- | --- |
| 507 | `public_wire.rs` - `WorkloadOp` / `WorkloadOpResponse` / `WorkloadPublicSummary` / `WorkloadListResult` | DELETE | row, §8.2 | handler-wire integration test passes via the `ResourceOp` path; `grep -r WorkloadOp packages/ --include='*.rs'` returns zero results |
| 691 | `d2b-r-<realm>-broker.socket` | DELETE | row, §8.2 | shared: `systemctl list-units 'd2b-r-*'` returns empty on updated host |
| 692 | `d2b-r-<realm>-broker.service` | DELETE | row, §8.2 | shared, same as above |
| 693 | `d2b-r-<realm>-controller.service` | DELETE | row, §8.2 | shared, same as above |
| 470 | `RelayProvider` (`d2b-realm-provider` trait) | DELETE | §8.2 | `grep -r RelayProvider packages/ --include='*.rs'` returns zero results |
| 490 | `session.rs` | REPLACE | row, §8.2 | remove after ComponentSession passes the `ADR046-session-001` integration test; §8.2 restates it as an end-to-end ComponentSession handshake test |
| 491 | `secure_session.rs` | REPLACE | row, §8.2 | same |
| 492 | `mux_session.rs` | REPLACE | row, §8.2 | same |
| 493 | `session_lifecycle.rs` | REPLACE | row, §8.2 | same |
| 494 | `router.rs` / `realm_router.rs` | REPLACE | row, §8.2 | Zone runtime replaces realm router; remove after `ADR046-core-001` |
| 464 | `PersistentShellProvider` | REPLACE | §8.2 | covered by §8.2 "`unsafe-local` as separate Provider": user-only `Host` resource created with `defaultDomain=user`, `allowedDomains=[user]`, `defaultUserRef=User/<name>`; no-isolation posture visible in host status, shell-session CLI warnings, and audit events; child process sessions are normal `Process` resources |
| 544 | `d2b-guest-shell-runner` | REPLACE | §8.2 | same §8.2 row as line 464 |

Lines 464 and 544 are matched to section 8.2 by subject rather than by an
explicit line citation: section 8.2 names the removal target
"`unsafe-local` as separate Provider", and these two are the only `DELETE` or
`REPLACE` rows in the map whose subject is that Provider family. Every other
`unsafe-local` row in the map is `ADAPT`, which schedules no removal and owes no
proof. An integrator who disagrees with that mapping should move these two rows
back into section 3.2 under `ADR046-primitives-003` (W2); the totals below then
become 38 proofed-out-of-48 minus those two, and the W2 owner count returns to
8.

## 3. Inventory of rows lacking a removal proof

Wave ownership is derived from the implementation graph's wave for the work item
the row names. Where a row names no work item, the owner is
`unassigned-needs-integrator`.

### 3.1 `DELETE` rows lacking a removal proof - 10 rows

Originally 11. Line 735 was retired as naming no path; see section 3.5.

| Line | Path / symbol | Work item | Owning wave |
| --- | --- | --- | --- |
| 427 | `workload.rs` - `WorkloadPlacement` / `WorkloadPlacementSummary` | `ADR046-primitives-002` | W2 |
| 465 | `DurableExecutionProvider` | `ADR046-provider-001` | W3 |
| 466 | `CredentialProvider` | `ADR046-provider-001` | W3 |
| 467 | `ObservabilitySinkProvider` | `ADR046-provider-001` | W3 |
| 468 | `InfrastructureProvider` | `ADR046-provider-001` | W3 |
| 469 | `NodeProvider` | `ADR046-core-001` | W4 |
| 621 | `options-realms-workloads.nix` - `vmsRef` link to `d2b.vms.<vm>` | `ADR046-identities-002` | W0 - see note below |
| 631 | `allocator-json.nix` | `ADR046-core-001` | W4 |
| 654 | `/etc/d2b/allocator.json` generated artifact | none - artifact table carries no work-item column | unassigned-needs-integrator; same subject as line 631, so W4 is the natural owner if the integrator binds it |
| 735 | `d2b userd *` CLI verb | `ADR046-primitives-003` | **retired - names no path; see 3.5** |
| 750 | `/run/d2b/allocator.sock` | `ADR046-core-001` | W4 |

### 3.2 `REPLACE` rows lacking a removal proof - 23 rows

Originally 25. Lines 479 and 527 were proved by W5; see section 3.5.

| Line | Path / symbol | Work item | Owning wave |
| --- | --- | --- | --- |
| 398 | `realm_stubs.rs` | `ADR046-session-001` | W1 - see note below |
| 433 | `route_engine.rs` - `RouteEngine` | `ADR046-core-001` | W4 |
| 434 | `routing.rs` | `ADR046-core-001` | W4 |
| 436 | `shell.rs` | `ADR046-primitives-003` | W2 |
| 438 | `frame.rs` / `payload.rs` / `mux.rs` / `stream.rs` | `ADR046-session-001` | W1 - see note below |
| 439 | `access.rs` | `ADR046-session-001` | W1 - see note below |
| 441 | `token.rs` | `ADR046-session-001` | W1 - see note below |
| 454 | `WorkloadProvider` | `ADR046-provider-001` | W3 |
| 455 | `GuestControlEndpointProvider` | `ADR046-session-001` | W1 - see note below |
| 456 | `HostSubstrateProvider` | `ADR046-primitives-003` | W2 |
| 457 | `RuntimeProvider` | `ADR046-provider-001` | W3 |
| 458 | `DisplayProvider` | `ADR046-provider-001` | W3 |
| 459 | `TransportProvider` | `ADR046-provider-001` | W3 |
| 460 | `TransportListener` | `ADR046-provider-001` | W3 |
| 461 | `ProtocolCodec` | `ADR046-session-001` | W1 - see note below |
| 462 | `StreamMux` | `ADR046-session-001` | W1 - see note below |
| 476 | `d2b-provider-aca` - `AcaWorkloadProvider` + `GuestControlEndpointProvider` impl | `ADR046-session-001` | W1 - see note below |
| 478 | `d2b-provider-relay` - `AzureRelayTransportProvider` | `ADR046-provider-001` | W3 |
| 479 | `d2b-host-providers` | `ADR046-primitives-003` | **proved by W5; see 3.5** |
| 481 | `d2b-realm-codec-protobuf` | `ADR046-session-001` | W1 - see note below |
| 500 | `TransportListener` impl (`d2b-realm-transport`) | `ADR046-provider-001` | W3 |
| 501 | Other transport types (`d2b-realm-transport`) | `ADR046-session-001` | W1 - see note below |
| 525 | `d2b-guestd` - PAM login, workload user exec, `ExecOp` handler | `ADR046-session-001` | W1 - see note below |
| 527 | `d2b-userd` | `ADR046-primitives-003` | **proved by W5; see 3.5** |
| 570 | `GuestControlForwarder` | `ADR046-session-001` | W1 - see note below |

### 3.3 Rows whose owning wave is already delivered

Thirteen rows above name work items assigned to waves that are already recorded
as `Merged`: `ADR046-session-001` (W1) owns twelve REPLACE rows and
`ADR046-identities-002` (W0) owns one DELETE row. Their owning wave cannot
supply the missing proof retrospectively, because the wave is closed and, per
the W0 and W1 waiver, was delivered without sealed records in the first place.

**Resolved by FR-060.** These rows carry no outstanding obligation against
their closed wave. FR-060 binds the removal proof to the wave that performs the
removal rather than the wave the map records as owner, so a sealed wave is never
asked to produce evidence it cannot produce and whose snapshot is immutable.

The practical consequence is unchanged in substance and clearer in ownership:
these paths acquire a proof obligation at the moment a later wave removes them,
which for the superseded realm session and router crates is the destructive
cutover and superseded-control-plane removal. A path that no wave ever removes
is not removed at all, so nothing is owed and nothing is silently dropped.

This is a scoping rule and not a second waiver. It moves *which* wave owes the
proof; it never removes the requirement that a removal has one. The same is true
of the single row at line 654 that maps to no work item: it owes a proof if and
when it is removed, and the wave performing that removal owns it.

### 3.4 Summary by owning wave

| Owner | Rows lacking a proof |
| --- | --- |
| W2 (`ADR046-primitives-002`, `ADR046-primitives-003`) | 3 |
| W3 (`ADR046-provider-001`) | 11 |
| W4 (`ADR046-core-001`) | 5 |
| W1 (`ADR046-session-001`) - wave closed, needs rebinding | 12 |
| W0 (`ADR046-identities-002`) - wave closed, needs rebinding | 1 |
| unassigned-needs-integrator | 1 (line 654) |
| **Total** | **33** |

This table reconciles with section 3: 10 unproofed DELETE rows plus 23
unproofed REPLACE rows is 33, and 33 unproofed plus 12 previously proofed plus
the 2 proofed by W5 plus the 1 retired row is the 48 rows scheduled for
removal. An earlier revision of this table recorded W3 as 10 and W1 as 11,
which summed to 37 rather than the 39 it claimed at the time; both counts were
corrected. The W2 count fell from 6 to 3 and the total from 36 to 33 in the W5
pass; section 3.5 shows the three rows that moved.

### 3.5 Deltas from the W5 removals

W5 removed three Rust crates. The evidence is in
[`removal-proof-w5.md`](./removal-proof-w5.md); this section records only what
that evidence does to the census.

| Line | Row | Was | Now | Basis |
| --- | --- | --- | --- | --- |
| 479 | `d2b-host-providers` | REPLACE, no proof, owner W2 | proof recorded, performed by W5 | FR-060 binds the proof to the removing wave, not the wave the map names |
| 527 | `d2b-userd` | REPLACE, no proof, owner W2 | proof recorded, performed by W5 | Same |
| 735 | `d2b userd *` CLI verb | DELETE, no proof, owner W2 | **owes no proof** | The verb does not exist at the map's own baseline `b5ddbed6`, in the `d2b` CLI crate or in `docs/reference/cli-contract.md`, and no commit on this lineage ever added or removed it from `packages/d2b/src` |
| 463, 480 | `d2b-daemon-access` | ADAPT - outside this census entirely | removed in W5, proof recorded | An ADAPT row schedules no removal, so the crate never appeared here. It was removed anyway. Disposition drift, raised not corrected |

Three points that a later reader should not have to reconstruct.

**Line 735 is retired, not waived.** FR-023 binds removals. A row scheduling
the deletion of a surface that was never present schedules no removal, so there
is nothing to prove. The row is a migration-map defect - a
`production-reachable` classification against a verb absent from the CLI crate
at the map's own baseline - and it is recorded rather than edited because the
map is a member specification and editing it re-triggers Gate 0 under FR-056.

**The `d2b-daemon-access` disposition is stale for the source path only.** Its
ADAPT half completed: the adaptation landed under `ADR046-api-001`, which is
`Merged`. The source crate was then an orphan and was deleted. The map cell
should say the source is retired after adaptation; it does not. Same handling,
same reason: raised under FR-046, not corrected inside a wave.

**Two proved rows are not two closed migrations.** Both crates were unreachable
when they were deleted, so their removal withdrew no operator-facing
capability. The successors the map names for them - `Provider/system-core`,
`Provider/runtime-cloud-hypervisor`, `Provider/display-wayland`, and the fixed
user supervisor `Process` under `Provider/system-systemd` - do not exist. The
trait-level rows at lines 456, 457 and 458 remain open against their own
owners and are still counted above.


## 4. Item-level removal conditions are not per-path proofs

The work-item manifest does record a `removalProof` field for the items that own
these rows:

| Work item | Manifest `removalProof` |
| --- | --- |
| `ADR046-primitives-002` | Role branches removed only after successor Provider tests |
| `ADR046-primitives-003` | `storage.json` rows removed only after Volume successor parity |
| `ADR046-provider-001` | Old trait crate retired only after all Provider dossiers migrate |
| `ADR046-core-001` | Current daemon branches removed after handler/Provider parity |
| `ADR046-identities-002` | Realm-facing declarations removed only in the reset/purge wave |
| `ADR046-session-001` | v3 old Realm PeerSession removed only after all v3 peer routes move |
| `ADR046-api-001` | Old command/resource-equivalent paths removed only per integration wave |

These are **coarse item-level preconditions**, not per-path removal proofs. Each
states a condition in prose, but none names a specific executable check bound to
a specific superseded path, and several govern dozens of rows at once. They do
not discharge FR-023 for the 36 rows above, which is exactly why those rows are
inventoried here.

By contrast, the manifest does contain items whose `removalProof` is
path-specific and executable - for example `ADR046-audio-001`'s
"`d2b-core/src/audio_policy.rs` deleted when no `d2bd` caller references it;
confirmed by `cargo check --no-default-features`" and `ADR046-activation-007`'s
enumeration of the exact functions deleted from `packages/d2b/src/lib.rs`. That
shape is the standard the 33 outstanding rows should be brought up to, and it
is the shape [`removal-proof-w5.md`](./removal-proof-w5.md) uses for the three
paths W5 removed.

Note also that `ADR046-api-001`'s `removalProof` cell above does not cover the
`d2b-daemon-access` deletion. It reads "Old command/resource-equivalent paths
removed only per integration wave", which is a sequencing precondition for the
*command* paths that item replaces; it names no crate and no check. The crate's
proof is in the W5 record, not in this field.

## 5. What each wave owes

For every row assigned to it above, a wave must, before removing the path:

1. name the specific check that proves the superseded path is unreferenced or
   its successor is at parity;
2. record that check's passing result against the wave's candidate snapshot; and
3. land the removal in its own change, separate from the change that introduced
   the replacement.

Rows marked `unassigned-needs-integrator`, and the closed-wave rows in section
3.3, must be bound to an owning wave before the path they name is removed.

W5 is the first wave to discharge this. Its three proofs are the worked example
of clauses 1 through 3, including the two failure modes a later wave should
expect: a reverse-dependency scan run at a commit where the crate had already
left the workspace proves nothing, and a Cargo-only scan misses Nix, lock,
fixture, golden and policy surfaces. See
[`removal-proof-w5.md`](./removal-proof-w5.md) sections 2 and 5.
