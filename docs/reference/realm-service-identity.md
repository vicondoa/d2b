# Realm service identity

**Diataxis category:** reference.

The `d2b.realm.v2` `RealmService` owns controller bootstrap, enrollment, route
resolution, direct-shortcut authorization and closure, inspection, and
cancellation. This page describes the runtime identity boundary. Declarative
identity records, rotation metadata, and revocation schemas remain documented
in [`realm-identity-lifecycle.md`](./realm-identity-lifecycle.md).

Every method runs inside an authenticated `ComponentSession`. The accepted
session identity, service package, realm scope, controller generation, request
lifetime, and idempotency metadata are authority; relay reachability, a
provider credential, or a caller-supplied realm field is not.

`Bootstrap` and `Enroll` must bind the parent trust anchor, child key pin, and
controller generation before publishing a route. Shortcut methods bind both
endpoint identities, controller generations, policy epoch, route digest,
operation, expiry, and close reports. Ambiguous or stale identity state returns
a typed denial and publishes no route.

The replacement service admits no bare-target alias, custom codec/version
handshake, HMAC display prologue, downgrade negotiation, or local-dispatch
fallback.

## Session authority

The service constructor accepts authority derived from the completed
ComponentSession handshake:

- Host-local controller sessions carry no realm credential material.
- Remote sessions are valid only when a gateway guest owns credential custody
  and the peer role is `RealmController` or `RemotePeer`.
- A relay-authenticated identity cannot create a host-local session or acquire
  local lifecycle authority.
- Every request realm and reconnect generation must exactly match the session
  authority. Request fields never expand that authority.

Bootstrap-purpose sessions can call only `Bootstrap`, `Enroll`, and `Inspect`.
Enrolled sessions can resolve routes and manage shortcuts after enrollment.

## State and routing

`Bootstrap` records a digest-bound controller generation. `Enroll` requires
that live bootstrap binding. `ResolveRoute` fails closed until enrollment and
returns only a digest-bound opaque route result.

`AuthorizeShortcut` requires an enrolled route, expiration within the request
lifetime, the current controller generation, and the current policy epoch.
Revocation and close reporting must match the original route digest. Closed
shortcuts cannot be reopened. Binding, shortcut, mutation-idempotency, audit,
dispatch-concurrency, request-size, and inspection-page state are all bounded.

`Cancel` addresses a request by its 16-byte ComponentSession request id and
reconnect generation. It distinguishes generation mismatch, unknown request,
already-terminal cancellation, and a newly signalled cancellation without
exposing operation contents.
