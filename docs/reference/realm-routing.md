# Realm tree routing contract

**Diataxis category:** reference.

This page documents the committed discovery and tree routing schema contract.
It is a schema and validation contract only. The current runtime
still uses the global `d2bd` public socket and existing `d2b.envs` VM substrate;
it does not start live relay routing, VPN or overlay networking, SSH fallback,
raw tunnels, provider adapters, or per-realm lifecycle routing.

The route roots are generated into the schema reference directory and summarized
by [`realm core`](./realm-core.md). Route metadata composes with
[realm access resolution](./realm-access-resolver.md),
[realm identity lifecycle](./realm-identity-lifecycle.md), and
[realm policy](./realm-policy.md); none of those documents should be read as
claiming live relay/runtime routing before that runtime exists.

## Tree model

Realm paths are validated `RealmPath` values, written most-specific first for
public targets and parent-first for storage. Routing is strictly a parent/child
tree:

- `RealmTreeEdge` is valid only when `child` is exactly one label below
  `parent`.
- `TreeRouteHop` is valid only when `from`, `to`, and `direction` match one
  declared parent/child edge.
- `TreeRoutePath` is bounded to 32 hops, must be contiguous, starts at the
  source realm, ends at the target realm, and records the nearest common
  ancestor.
- Sibling-to-sibling routing is represented as child-to-parent hop(s), then
  parent-to-child hop(s). There is no sibling shortcut in the tree path itself.
- Loops, multiple parents, parent/sibling advertisements, and namespace
  violations are fail-closed route decisions.

A route decision carries `TreeRouteDecisionOutcome::Allowed { path }` or
`Denied { reason }`. It carries no socket, transport endpoint, relay address,
provider credential, SSH target, VPN route, overlay address, or raw tunnel
handle.

## Discovery and pre-auth admission

Discovery metadata is bounded before peer identity is trusted:

| Contract | Bound / behavior |
| --- | --- |
| `DiscoveryQueuePolicy.maxDepth` | 1..4096 unauthenticated discovery items. |
| `maxUnverifiedPeers` | 1..1024 redacted unverified peer handles. |
| Per-relay-class and per-peer pre-auth rate limits | 1..60000 events per minute. |
| Overflow behavior | `drop-new` only; new unauthenticated input is dropped when full. |
| `UnverifiedPeerAdmissionAttemptMetadata.queueDepth` | Must remain within the queue bound. |

`DiscoveryIngressClass` is deliberately low-cardinality: local root, parent
relay, child relay, provider relay, static config, or unknown. It is not a raw
endpoint label. `UnverifiedPeerRef` is a redacted queue-local handle, not an
authenticated realm identity and not a metric dimension.

## Replay and session admission bounds

`ReplayWindowMetadata` describes replay protection without exposing the concrete
storage or transport implementation:

| Field | Bound |
| --- | --- |
| `maxEntries` | 1..1,000,000 |
| `currentEntries` | `0..maxEntries` |
| `ttlSeconds` | 1..86,400 |
| `observedReplayCount` | 0..1,000,000 |

Post-auth `SessionAdmissionAttemptMetadata` records the local realm, remote
realm, operation kind, optional required capability, replay-window metadata, and
admit/deny outcome. Admission denial reasons are stable route fail-closed labels
such as `replay`, `rate-limited`, `missing-capability`, and `policy-denial`.

## Route advertisements and namespace validation

A `RouteAdvertisement` is signed, expiring, descendant-only metadata:

- `treeEdge.child` must equal the advertising realm.
- `routes[]` must be non-empty and contain at most 64 descendant routes.
- `expiresAtUnixSeconds` must be greater than `issuedAtUnixSeconds`.
- Every `DescendantRoute.descendant` must be below the advertising realm.
- `nextHopChild` must match the first child label below the advertising realm.
- `RouteSignature` contains algorithm, key role, signing-key fingerprint, and a
  detached `SignatureRef`; it never contains key bytes or credential material.

`RouteNamespaceAllocation` is the parent-to-direct-child delegation contract.
Allowed prefixes must be the child realm itself or descendants below that child;
parents, siblings, and arbitrary external prefixes are rejected. Allocations are
bounded to 16 prefixes and at most 64 advertised routes, and carry a capability
ceiling that future verifiers must apply before accepting an advertisement.

## Direct shortcut constraints

`DirectShortcutAuthorizationMetadata` is only an authorization record for a
shorter transport path after the tree route has been authorized. It must preserve
the same source realm, target realm, nearest common ancestor, operation kind,
optional required capability, policy rule id, and positive expiry interval as the
authorized `TreeRoutePath`.

A shortcut record still does not encode underlay details. It does not authorize a
VPN, overlay network, SSH fallback, raw TCP proxy, raw relay tunnel, file
descriptor tunnel, or provider-specific opaque bypass. The authorized tree path
remains the accountability path for policy, audit, teardown, and revocation.

Shortcut states are bounded (`authorized`, `established`, `teardown-requested`,
`torn-down`, `denied`), and teardown reasons are stable labels (`completed`,
`expired`, `policy-revoked`, `route-revoked`, `transport-unavailable`,
`peer-disconnected`).

## Correlation and audit chain

Routing metadata objects carry `correlationId` and optional `TraceContext` so discovery,
admission, advertisement, decision, shortcut, teardown, and later identity/audit
records can be correlated without copying payloads or credentials into labels.
Route audit labels are low-cardinality and include event kind, realm classes,
placement class, operation kind, optional fail-closed reason, and optional policy
rule id only.

For tamper-evident audit stream chaining, use the `AuditChainRecord` /
`AuditChainLink` contract in [realm core](./realm-core.md). Route metadata should
link to the same correlation id and redacted audit chain, but should not embed
payload bytes, argv, stdio, relay endpoints, provider headers, host paths,
identity strings, or secrets.

## Telemetry labels

`RouteTelemetryBatch` is bounded to 64 samples. Metric labels intentionally avoid
realm paths, targets, transport endpoints, and peer identifiers. Use only the
stable route counter kinds and low-cardinality realm/placement/fail-closed
classes defined by the realm routing schema.

## Current implementation boundary

The committed implementation provides data models, strict validation, tests, and
generated schema coverage. It does not yet:

- discover peers on a live relay;
- route `d2b` commands through per-realm daemons;
- enforce route advertisements in a running gateway/router;
- allocate realm networks or install VPN/overlay routes;
- open SSH, raw TCP, raw relay, vsock, or file-descriptor tunnels;
- evaluate live identity, revocation, teardown, or provider policy;
- migrate current `d2b.envs` networking or VM placement.

Future runtime implementations must fail closed on malformed advertisements,
unknown parents, namespace violations, sibling/parent advertisements, loops,
multiple parents, expiry, replay, rate limiting, missing capabilities, and policy
denials.

## Related references

- [Realm core model reference](./realm-core.md)
- [Realm access resolver contract](./realm-access-resolver.md)
- [Realm identity lifecycle contract](./realm-identity-lifecycle.md)
- [Realm policy](./realm-policy.md)
- [Realm controller configuration](./realm-controller-config.md)
- [Transport support policy](./transport-support-policy.md)
