# Amendment request: two frozen cross-Zone contract gaps, batched

| Field | Value |
| --- | --- |
| Scope | The D101 frozen domain-tag list, and the closed `ZoneRouteFailClosedReason` set |
| Raised under | FR-046 / FR-047 |
| Affected member specs | `ADR-046-decision-register` (D101); `ADR-046-zone-routing` |
| Affected code | `packages/d2b-bus/src/zone_route.rs`; `packages/d2b-contracts/src/v3/zone_routing.rs` |
| Owning items, both sealed | `ADR046-object-001` (W0); `ADR046-routing-001` and `ADR046-routing-005` (W2) |
| Status | Raised to the integrator; awaiting a separate specification amendment |
| Must land before | W6, the first wave in which a cross-Zone hop is reachable over a real transport |

## 1. Why these two are one amendment and not wave work

Both entries below were recorded in
[`implementation-debt.md`](./implementation-debt.md) at Wave 2 close as needing
an integrator ruling, because each names a **frozen contract whose only owning
work item sits in an already-sealed wave**, and no later wave's destination set
names either file. There is therefore no wave that can take them as scheduled
work without first re-opening a sealed wave's evidence.

That is the definition of an amendment rather than a schedule slot. Under
FR-046 a frozen contract is not corrected inside an implementation wave; it is
carried by a dedicated amendment with its own validation and panel round. The
same reasoning is set out at length in section 3 of
[`amendment-w2-destination-drift.md`](./amendment-w2-destination-drift.md) and
is not repeated here.

They are batched into **one** amendment rather than two because they share a
single affected surface (the cross-Zone hop), a single trigger, and a single
validation story. Two amendments would re-open overlapping evidence twice for
one behaviour.

## 2. Gap one: the principal digest has no frozen domain tag

The cross-Zone idempotency key needs a subject digest. D101 freezes the
complete domain-tag list, and it is a closed set:

> Frozen domain tags: `d2b:v3:resource-envelope`, `d2b:v3:resource-spec`,
> `d2b:v3:resource-status`, `d2b:v3:schema`, `d2b:v3:change-payload`,
> `d2b:v3:operation-request`, `d2b:v3:artifact-catalog`, and
> `d2b:v3:resource-bundle`

None of the eight is a principal or subject tag. The digest computed at
`packages/d2b-bus/src/zone_route.rs` is consequently **undomained**, and if a
tag is later frozen the computation changes.

The amendment must decide one of: freeze a new principal/subject domain tag in
D101, or record that the principal digest is deliberately domained by an
existing tag and say which.

## 3. Gap two: there is no closed reason for a multi-Zone batch

`ZoneRouteFailClosedReason` in `packages/d2b-contracts/src/v3/zone_routing.rs`
is a closed enum. Verified against the current tree, its variants are
`MalformedAdvert`, `UnknownParent`, `NamespaceViolation`,
`SiblingOrParentRouteAdvert`, `Loop`, `MultiParent`, `Expired`, `Replay`,
`RateLimited`, `QueueFullDropNew`, `MissingCapability`, `PolicyDenial`,
`ZoneLinkDisconnected`, `HopLimitExceeded`, `RelayDenied` and
`AttachmentNotPermittedOverZoneLink`. **No variant describes a batch that spans
Zones.**

The shipped refusal site therefore returns a structural error rather than
misusing an unrelated routing reason, which is the correct fail-closed
behaviour but is not a routing reason a caller can discriminate on.

The amendment must decide whether to append a multi-Zone-batch variant to the
frozen set, or to affirm that a cross-Zone batch is permanently a structural
error and not a routing outcome.

## 4. The trigger, and how it was determined

Neither gap is observable until a cross-Zone hop is carried over a real
transport. Until then the bus stays deliberately unwired from production
listeners, no principal digest crosses a Zone boundary on the wire, and no
caller can receive a routing reason for a batch it could not have sent.

The wave that first makes such a hop reachable is the wave owning the transport
Provider work. Determined from `docs/specs/ADR-046-implementation-graph.json`
rather than assumed: every `ADR046-transport-unix-001` through
`ADR046-transport-unix-011`, every `ADR046-transport-relay-001` through
`ADR046-transport-relay-007`, and the `ADR-046-provider-transport-unix` and
`ADR-046-provider-transport-vsock` spec nodes all carry `"wave": "W6"`. No
transport work item is assigned to any earlier wave.

**The amendment must land before W6.** It does not block W3, W4 or W5, none of
which reaches a real cross-Zone transport hop.

## 5. Record and disposition

- Both gaps are **recorded here** and **raised to the integrator**.
- They are to be resolved by a **single specification amendment** against
  `ADR-046-decision-register` (D101) and `ADR-046-zone-routing`, scheduled
  outside any implementation wave, which will re-run those specs' validation
  and panel evidence.
- Until that amendment lands, the shipped behaviour stands: the principal
  digest is undomained and a cross-Zone batch is a structural error.
- This amendment does **not** absorb the section 3.2 destination drifts. Those
  affect a different member specification and belong with
  [`amendment-w2-destination-drift.md`](./amendment-w2-destination-drift.md);
  the reasoning is recorded in section 6 of `implementation-debt.md`.
