# `d2b-provider-transport-unix`

This is the canonical crate root for `Provider/transport-unix`. It supplies
the authenticated local transport portal used by child ZoneLink controllers
and same-Zone ComponentSession callers.

See [Create a Provider](../../docs/how-to/create-provider.md) and the
[transport-unix dossier](../../docs/specs/providers/ADR-046-provider-transport-unix.md)
for the implementation contract.

## Provider identity

| Field | Value |
| --- | --- |
| Provider name | `transport-unix` |
| Provider reference | `Provider/transport-unix` |
| Package | `packages/d2b-provider-transport-unix/` |

## Config schema

The closed binding schema accepts an optional `socketKind` of `seqpacket` or
`stream`. It accepts no transport credentials, paths, raw descriptors, peer
identities, or broker role claims.

## Exported resource types

This Provider exports no ResourceType. It supplies only the authenticated local
transport portal consumed by ZoneLink and ComponentSession routing.

## Controllers / services / workers / binaries

`TransportService` owns one bounded `TransportPortal`. There are no standalone
workers or binaries: admission and broker access remain daemon-supervised.

## Transport admission

- `SO_TYPE` is checked against the declared seqpacket or stream route.
- Seqpacket routes enable `SO_PASSCRED`; stream routes never carry attachments.
- ZoneLink routes never carry attachments, even when their caller requests
  them.
- Every accepted descriptor and portal monitor duplicate is close-on-exec.
- The portal retains request-bound peer evidence and an owned monitor duplicate;
  callers receive the validated original descriptor exactly once.

### Graph-bound attach

`TransportService::open_under_binding` is the graph-bound path, and it is the
only path with no independent access list:

- The portal holds a bounded `TransportBindingRegistry` of admitted
  `EndpointBinding` relationships keyed by their graph binding key. A peer that
  is not under one of those keys has no attach path at all.
- `admit_attach` is the single ordered gate: a foreign Zone, a different store
  incarnation, a revoked or draining relationship, a stale source or consumer
  generation, an advanced desired revision or Zone desired sequence, and a
  reconnect ordinal below the relationship's floor each refuse at the shared
  admission stage and refusal reason.
- The kernel uid/gid a transport may be pinned to comes from the relationship
  itself. `open_under_binding` has no argument in which a caller could place a
  peer identity of its own; the caller-supplied `ExpectedPeer` path is the
  legacy one and is not the graph-bound decision.
- A descriptor-carrying open must have been admitted as an `Attach`
  relationship; a `Connect` or `Listen` relationship refuses with
  `attachment-kind-conflict`.
- Revocation is effective at the service, not at the caller's copy: the service
  attaches against the relationship its registry currently admits, so a
  reconnect that presents a value admitted before a revoke is refused.

### Data plane and control plane

`ControlPlaneRequest` is the closed set of privileged portal operations
(`Open`, `Close`, `Observe`). It can only be issued against a live admitted
route, because issuing one requires the opaque `ControlRouteToken` that only
`AdmittedTransportRoute::control_token` mints.
`ControlPlaneRequest::from_carriage` is the one function stream-carried bytes
could reach if the portal ever parsed its data plane for control: it scans the
bytes for an operation discriminant and always refuses, because carriage names
an operation but cannot carry a route token.

## Lifecycle

The local handle table is bounded to 256 opaque entries. `close` is idempotent
and service finalization retires only monitor descriptors owned by that portal.
The existing Unix session listener adoption path remains the restart-safe owner
for inherited local listeners.

## Session substrate

The portal deliberately has no dependency on the session transport
implementation. It validates only the accepted descriptor and leaves framing,
attachment credits, descriptor identity, and pidfd validation to the owning
session runtime. It does not resolve a peer into a subject: subject resolution
remains owned by the authenticated Zone runtime.

## Placement and dependencies

The portal is a daemon-supervised, same-Zone service component. It holds no
host path, credential, remote registry, or ambient broker mutation handle.

## RBAC requirements

Only the authenticated Zone controller and transport service may construct the
request binding passed to the portal. Broker authority and accepted peer
evidence remain bound to that one request. On the graph-bound path the request
binding the portal records is DERIVED from the admitted relationship - its Zone
and its consumer - so a caller cannot widen it.

## Security posture

The implementation performs only fd-relative socket operations. It never
accepts a socket path, raw identity claim, caller-supplied descriptor number,
or payload-derived subject. On the graph-bound path it also never accepts a
caller-supplied peer identity: the uid/gid the open enforces is the one the
admitted relationship carries, and a mismatch is reported as
`peer-policy-mismatch` rather than as an unreadable peer credential.

## State and telemetry

Audit and metric dimensions are closed enums. They contain no peer identity,
socket address, descriptor number, opaque handle, path, or payload.

## Build and test

```bash
bazel test //packages/d2b-provider-transport-unix:all-tests
```

The focused tests cover accepted-fd/peer/request binding, socket-kind and
attachment refusal, close-on-exec, and owned finalization, plus the graph-bound
path: foreign-Zone and stale-generation attachment refusal, revocation that a
reconnect cannot revive, a stream-carriage attempt to inject a privileged
control operation, the relationship-owned peer pin, and the bounded
relationship registry.
