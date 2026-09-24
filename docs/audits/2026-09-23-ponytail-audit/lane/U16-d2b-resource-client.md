# U16 d2b-resource-client

net: -2 lines, -0 deps

## Findings (all caller-verified workspace-wide; see #Verified per row)

- #B9 [not applied] `ZoneServiceClient` type alias = `ZoneClient` (zone_client.rs:598).
  Zero external callers: workspace-wide reference search over `packages/**/*.rs`
  finds the alias only at its definition and at its re-export arm in lib.rs:73;
  no BUILD.bazel, nixos-modules/, test, or docs/policy reference names it.
  The alias adds no type-level distinction - `ZoneServiceClient<R, C, W> =
  ZoneClient<R, C, W>` with no re-bound service, verb, or carriage - so it is
  declared-one-purpose public surface in the #B9 family. Deleting the alias
  (1 line) and its re-export arm (1 of the line at lib.rs:73) nets -2 in-crate,
  leaves `ZoneClient` canonical. [packages/d2b-resource-client/src/zone_client.rs:598]
  (leaf)

- #B9 [not applied] limits consts in call.rs - `TRACE_ID_BYTES` (0 external
  callers: grep `TRACE_ID_BYTES` workspace-wide → only def call.rs:30 + 8
  in-crate readers), `MAX_CORRELATION_ID_BYTES` (0 external), plus the
  cross-crate family at `REQUEST_ID_BYTES`/`MAX_IDEMPOTENCY_KEY_BYTES`/
  `MAX_REQUEST_LIFETIME_MS` (external callers exist: d2b/src/context.rs,
  d2bd composition). These are the ADR45-carried protocol ceilings the crate
  doc names as a reconciliation obligation against the v3 session contract
  module once it lands; the four with in-crate readers stay for the wire
  bound, and the cross-crate-limit family already rides the ledger's refusal
  class for protocol ceilings (nix-pinned wire fields / catalog-pinned
  admission). No further cut.

- #C1 [not applied] `GuestControlEndpoint` declared twice, byte-identically:
  this crate's copy at zone_client.rs:129 and the peer copy at
  d2b-provider-guest-cloud-hypervisor/src/guest_local.rs:49 remain. Both
  copies have live workspace callers (d2bd/src/composition.rs,
  d2b-resource-client tests, guest-cloud-hypervisor tests), so dedup is a
  cross-crate refactor (one copy's callers must re-point at the other), which
  the consistency feed owns; both copies stay per the cross-crate-ownership
  refusal class. No new evidence the blocker changed.

## Consistency notes

- `GuestControlEndpoint`: the two byte-identical declarations are a
  cross-crate duplicate-type finding for U97. Canonical home: none selected
  yet - both copies carry live callers; dedup is a cross-crate move (either
  crate becomes the import site and the other becomes a re-export).
- `ZoneServiceClient`/`ZoneClient`: naming-drift pair - `ZoneServiceClient` is
  a one-purpose alias for the same type, exported alongside the canonical
  `ZoneClient`; report to U97 as an alias-vs-canonical naming drift row.

## Verified

Read all eight source files (call.rs 501, client.rs 201, dispatch.rs 655,
error.rs 152, lib.rs 74, process_attachment.rs 1381, target.rs 803,
zone_client.rs 1075). Caller tables per symbol via workspace-wide
`grep -rn "\b<sym>\b" packages --include=*.rs` plus non-`.rs` surfaces
(BUILD.bazel dep lists, nixos-modules/, docs/reference/policy/) checked for
`ZoneServiceClient`, `GuestControlEndpoint`, and every `*_BYTES`/`*_MS` limits
const. Both ledger rows (#B9, #C1) are [not applied] - still present, re-flagged
with fresh caller tables. No new findings beyond the two carried rows.

## U3 outcome (2026-09-24)
- applied: ZoneServiceClient alias (zone_client.rs) + its re-export arm (lib.rs). R4 at HEAD: zero callers (definition + export arm only).
