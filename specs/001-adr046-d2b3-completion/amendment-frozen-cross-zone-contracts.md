# Frozen cross-Zone contract gaps

This artifact records two technical gaps in the cross-Zone contract as
implementation rationale.

| Field | Value |
| --- | --- |
| Scope | The D101 frozen domain-tag list and the closed `ZoneRouteFailClosedReason` set |
| Affected code | `packages/d2b-bus/src/zone_route.rs`; `packages/d2b-contracts/src/v3/zone_routing.rs` |
| Status | The current fail-closed behavior remains in force until the contract is completed |

## 1. Principal digest domain

The cross-Zone idempotency key needs a subject digest. D101 currently freezes
the following domain tags:

> `d2b:v3:resource-envelope`, `d2b:v3:resource-spec`,
> `d2b:v3:resource-status`, `d2b:v3:schema`, `d2b:v3:change-payload`,
> `d2b:v3:operation-request`, `d2b:v3:artifact-catalog`, and
> `d2b:v3:resource-bundle`

None is a principal or subject tag. The digest computed at
`packages/d2b-bus/src/zone_route.rs` is consequently undomained. Before a
cross-Zone hop is enabled over a production transport, the contract must
either freeze a principal/subject domain tag or explicitly define which
existing tag domains that digest.

## 2. Multi-Zone batch refusal

`ZoneRouteFailClosedReason` in
`packages/d2b-contracts/src/v3/zone_routing.rs` is a closed enum. Its current
variants include `MalformedAdvert`, `UnknownParent`, `NamespaceViolation`,
`SiblingOrParentRouteAdvert`, `Loop`, `MultiParent`, `Expired`, `Replay`,
`RateLimited`, `QueueFullDropNew`, `MissingCapability`, `PolicyDenial`,
`ZoneLinkDisconnected`, `HopLimitExceeded`, `RelayDenied`, and
`AttachmentNotPermittedOverZoneLink`. No variant describes a batch spanning
Zones.

The shipped refusal site returns a structural error rather than misusing an
unrelated routing reason. That is fail-closed, but callers cannot discriminate
this case as a routing reason. Before such a batch can cross a Zone boundary,
the contract must either add a dedicated multi-Zone-batch reason or explicitly
define the structural-error behavior as permanent.

## 3. Validation boundary

Until a real cross-Zone transport is enabled, the bus remains unwired from
production listeners and no principal digest crosses a Zone boundary on the
wire. Focused contract tests must cover the selected domain-tag and refusal
semantics before that transport surface is enabled.
