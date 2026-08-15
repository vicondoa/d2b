# Removal-proof inventory (FR-023)

| Field | Value |
| --- | --- |
| Source record | `docs/specs/ADR-046-current-code-migration-map.md` |
| Satisfies | FR-023 |
| Status | Historical census of superseded paths and their evidence |
| Detailed evidence | [`removal-proof-w5.md`](./removal-proof-w5.md) |

This file preserves the removal-proof census. The source migration map records
which paths are superseded; this artifact records the technical evidence needed
before a removal is accepted.

## 1. Removal-proof rule

Every `DELETE` or `REPLACE` path must have an explicit, executable check before
removal. The check must prove that the superseded path is unreferenced or that
its replacement has the required parity. A directory listing or a manifest
status alone is insufficient.

The proof must cover source, Cargo, Nix, lockfile, fixture, golden, policy, and
reference surfaces as applicable. It must also prove that the replacement does
not silently withdraw an operator-facing capability. Generic item-level prose
does not substitute for a path-specific check.

## 2. Census

The recorded migration map contains:

| Measure | Count |
| --- | --- |
| `DELETE` disposition rows | **16** |
| `REPLACE` disposition rows | **32** |
| `DELETE` plus `REPLACE` rows | **48** |
| `DELETE` rows with explicit proof | **5** |
| `REPLACE` rows with explicit proof | **9** |
| Rows with explicit proof | **14** |
| Rows with no explicit proof | **33** |

The earlier claim of three proofed `DELETE` rows undercounted the per-realm
unit table and omitted the dedicated `RelayProvider` check. The six
non-RSS measurement rows remain historical evidence in their owning result
artifacts and do not affect this source-removal census.

## 3. Recorded proof examples

The map records executable evidence for these representative paths:

| Superseded path or symbol | Required evidence |
| --- | --- |
| `WorkloadOp`, `WorkloadOpResponse`, `WorkloadPublicSummary`, `WorkloadListResult` | Handler-wire integration through the `ResourceOp` path and a zero-reference source scan |
| `RelayProvider` | Zero-reference source scan |
| Realm session files and router | End-to-end ComponentSession handshake and Zone-runtime route tests |
| `PersistentShellProvider`, `d2b-guest-shell-runner` | `unsafe-local` Provider parity, host-status visibility, CLI warning, audit-event, and normal `Process` checks |
| `d2b-host-providers`, `d2b-userd` | The removal evidence in [`removal-proof-w5.md`](./removal-proof-w5.md) |
| `d2b-daemon-access` | Adaptation parity plus zero-reference source, package, and policy checks |

Rows without proof include old workload and Provider traits, realm routing and
transport types, legacy allocator paths, and the old realm-session surfaces.
Their path-specific checks remain outstanding until the corresponding
replacement contract and focused tests exist. A path absent from the baseline
does not create a removal obligation.

## 4. Completed removal evidence

The evidence record documents removal of three Rust crates and the
`d2b-daemon-access` source after adaptation. It also records that the
`d2b userd` verb was absent from the baseline, so no deletion proof is owed for
that nonexistent path. The successors named by the migration map must still
exist and meet their own Provider, Process, and CLI contracts before a
replacement is considered complete.

## 5. Evidence requirements for future removals

Before removing a superseded path, the implementation change must:

1. name the focused check that proves the path is unreferenced or its successor
   has parity;
2. run the check against the changed source and all relevant generated,
   packaging, fixture, and reference surfaces; and
3. record the result with the removal evidence for the path.

These are product-safety evidence requirements. The owning contract and focused
checks determine whether a removal is safe.
