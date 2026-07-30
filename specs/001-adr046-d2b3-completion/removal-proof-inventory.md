# Removal-proof inventory (FR-023)

| Field | Value |
| --- | --- |
| Source of truth | `docs/specs/ADR-046-current-code-migration-map.md` |
| Cross-referenced | `docs/specs/ADR-046-work-items.json`, `docs/specs/ADR-046-implementation-graph.json` |
| Satisfies | FR-023 |
| Status | Inventory of missing removal proofs; each gap assigned to the wave that removes its path |

## 1. Why this exists

FR-023 requires that each superseded path scheduled for removal pass an
**explicit removal proof** before it is removed, and that the removal land in
its own change separate from the change that introduced the replacement. A
disposition of `DELETE` or `REPLACE` in the migration map therefore schedules a
removal, and each such row owes a proof.

This document inventories the rows that do not have one.

## 2. Counts found, versus the claim

The program's working note claimed that the migration map "supplies explicit
proofs for only 3 of its 16 DELETE rows". Counting the real file:

| Measure | Count |
| --- | --- |
| `DELETE` disposition rows | **16** |
| `REPLACE` disposition rows | **32** |
| `DELETE` + `REPLACE` rows, total | **48** |
| `DELETE` rows carrying an explicit removal proof | **4** |
| `REPLACE` rows carrying an explicit removal proof | **5** |
| Rows carrying an explicit removal proof, total | **9** |
| Rows scheduled for removal with **no** explicit proof | **39** |

**The claim is close on the denominator and understated on the numerator.** The
DELETE denominator of 16 is correct. The proofed count is **4, not 3**. The
discrepancy comes from how the per-realm unit table records its proof: three
DELETE rows there share a single trailing proof row rather than each carrying a
proof cell, so a naive per-row scan sees one proof where three rows are covered.

The claim also stops at DELETE and does not account for the 32 REPLACE rows,
which schedule removals just as DELETE rows do. Only 5 of those 32 carry a
proof, which is where the bulk of the outstanding gap actually is.

### The 9 rows that do carry a proof

| Line | Row | Disposition | Proof recorded |
| --- | --- | --- | --- |
| 507 | `public_wire.rs` - `WorkloadOp` / `WorkloadOpResponse` / `WorkloadPublicSummary` / `WorkloadListResult` | DELETE | handler-wire integration test passes via the `ResourceOp` path |
| 691 | `d2b-r-<realm>-broker.socket` | DELETE | shared: no `d2b-r-*` units in `systemctl list-units` on updated host |
| 692 | `d2b-r-<realm>-broker.service` | DELETE | shared, same as above |
| 693 | `d2b-r-<realm>-controller.service` | DELETE | shared, same as above |
| 490 | `session.rs` | REPLACE | remove after ComponentSession passes the `ADR046-session-001` integration test |
| 491 | `secure_session.rs` | REPLACE | same |
| 492 | `mux_session.rs` | REPLACE | same |
| 493 | `session_lifecycle.rs` | REPLACE | same |
| 494 | `router.rs` / `realm_router.rs` | REPLACE | Zone runtime replaces realm router; remove after `ADR046-core-001` |

## 3. Inventory of rows lacking a removal proof

Wave ownership is derived from the implementation graph's wave for the work item
the row names. Where a row names no work item, the owner is
`unassigned-needs-integrator`.

### 3.1 `DELETE` rows lacking a removal proof - 12 rows

| Line | Path / symbol | Work item | Owning wave |
| --- | --- | --- | --- |
| 427 | `workload.rs` - `WorkloadPlacement` / `WorkloadPlacementSummary` | `ADR046-primitives-002` | W2 |
| 465 | `DurableExecutionProvider` | `ADR046-provider-001` | W3 |
| 466 | `CredentialProvider` | `ADR046-provider-001` | W3 |
| 467 | `ObservabilitySinkProvider` | `ADR046-provider-001` | W3 |
| 468 | `InfrastructureProvider` | `ADR046-provider-001` | W3 |
| 469 | `NodeProvider` | `ADR046-core-001` | W4 |
| 470 | `RelayProvider` (`d2b-realm-provider` trait) | none - row records `-` | unassigned-needs-integrator |
| 621 | `options-realms-workloads.nix` - `vmsRef` link to `d2b.vms.<vm>` | `ADR046-identities-002` | W0 - see note below |
| 631 | `allocator-json.nix` | `ADR046-core-001` | W4 |
| 654 | `/etc/d2b/allocator.json` generated artifact | none - artifact table carries no work-item column | unassigned-needs-integrator; same subject as line 631, so W4 is the natural owner if the integrator binds it |
| 735 | `d2b userd *` CLI verb | `ADR046-primitives-003` | W2 |
| 750 | `/run/d2b/allocator.sock` | `ADR046-core-001` | W4 |

### 3.2 `REPLACE` rows lacking a removal proof - 27 rows

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
| 464 | `PersistentShellProvider` | `ADR046-primitives-003` | W2 |
| 476 | `d2b-provider-aca` - `AcaWorkloadProvider` + `GuestControlEndpointProvider` impl | `ADR046-session-001` | W1 - see note below |
| 478 | `d2b-provider-relay` - `AzureRelayTransportProvider` | `ADR046-provider-001` | W3 |
| 479 | `d2b-host-providers` | `ADR046-primitives-003` | W2 |
| 481 | `d2b-realm-codec-protobuf` | `ADR046-session-001` | W1 - see note below |
| 500 | `TransportListener` impl (`d2b-realm-transport`) | `ADR046-provider-001` | W3 |
| 501 | Other transport types (`d2b-realm-transport`) | `ADR046-session-001` | W1 - see note below |
| 525 | `d2b-guestd` - PAM login, workload user exec, `ExecOp` handler | `ADR046-session-001` | W1 - see note below |
| 527 | `d2b-userd` | `ADR046-primitives-003` | W2 |
| 544 | `d2b-guest-shell-runner` | `ADR046-primitives-003` | W2 |
| 570 | `GuestControlForwarder` | `ADR046-session-001` | W1 - see note below |

### 3.3 Rows whose owning wave is already delivered

Ten rows above name work items assigned to waves that are already recorded as
`Merged`: `ADR046-session-001` (W1) owns eleven REPLACE rows and
`ADR046-identities-002` (W0) owns one DELETE row. Their owning wave cannot
supply the missing proof retrospectively, because the wave is closed and, per
the W0 and W1 waiver, was delivered without sealed records in the first place.

These rows are therefore **not** discharged by their nominal wave. They need the
integrator to rebind them - most plausibly to the wave that actually performs
the physical removal, which for the superseded realm session and router crates
is the destructive cutover and superseded-control-plane removal later in the
program. Until rebound, treat their owner as
`unassigned-needs-integrator` in practice.

### 3.4 Summary by owning wave

| Owner | Rows lacking a proof |
| --- | --- |
| W2 (`ADR046-primitives-002`, `ADR046-primitives-003`) | 8 |
| W3 (`ADR046-provider-001`) | 10 |
| W4 (`ADR046-core-001`) | 5 |
| W1 (`ADR046-session-001`) - wave closed, needs rebinding | 11 |
| W0 (`ADR046-identities-002`) - wave closed, needs rebinding | 1 |
| unassigned-needs-integrator | 2 (lines 470 and 654) |
| **Total** | **39** |

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
not discharge FR-023 for the 39 rows above, which is exactly why those rows are
inventoried here.

By contrast, the manifest does contain items whose `removalProof` is
path-specific and executable - for example `ADR046-audio-001`'s
"`d2b-core/src/audio_policy.rs` deleted when no `d2bd` caller references it;
confirmed by `cargo check --no-default-features`" and `ADR046-activation-007`'s
enumeration of the exact functions deleted from `packages/d2b/src/lib.rs`. That
shape is the standard the 39 outstanding rows should be brought up to.

## 5. What each wave owes

For every row assigned to it above, a wave must, before removing the path:

1. name the specific check that proves the superseded path is unreferenced or
   its successor is at parity;
2. record that check's passing result against the wave's candidate snapshot; and
3. land the removal in its own change, separate from the change that introduced
   the replacement.

Rows marked `unassigned-needs-integrator`, and the closed-wave rows in section
3.3, must be bound to an owning wave before the path they name is removed.
