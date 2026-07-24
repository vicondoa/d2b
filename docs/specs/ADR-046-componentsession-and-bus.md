# ADR 0046 ComponentSession and d2b-bus

| Field | Value |
| --- | --- |
| Spec ID | `ADR-046-componentsession-and-bus` |
| Parent | ADR 0046 |
| Status | Proposed |
| Version | 1 |
| Baseline | `b5ddbed67867d9244bf33390868101bd9b053e49` |
| Normative | Yes |
| Owners | `d2b-session`, transport adapters, `d2b-bus` |
| Depends on | `ADR-046-terminology-and-identities`, `ADR-046-resource-api-and-authorization` |
| Supersedes | Current v3 Realm PeerSession and ad hoc guest/user/public IPC |

## Source and reuse policy

The pre-ADR45 v3 baseline has no ComponentSession. This spec deliberately
copies/adapts the implementation from main commit:

```text
a1cc0b2da4a08ca3240a770a972fe4da6f912bef
ADR-0045 W9: coordinate toolkit and sibling cutover (#314)
```

Selected main sources:

| Main source | Selected behavior |
| --- | --- |
| `packages/d2b-session/src/handshake.rs` | Strict canonical offer/preface, Noise prologue binding, NN/KK/IKpsk2 profiles, generation discovery, credentials, transcript |
| `packages/d2b-session/src/bootstrap.rs` | Zeroizing single-use operation/nonce/expiry-bound bootstrap PSK |
| `packages/d2b-session/src/record.rs` | Directional encrypted records, sequence/reconnect generation, replay detection, bounds |
| `packages/d2b-session/src/engine.rs` | Async establish/reconnect, timeout, cancellation, ttrpc, named streams, attachments |
| `packages/d2b-session/src/scheduler.rs` | Bounded priority/fair scheduling and named-stream round robin |
| `packages/d2b-session/src/streams.rs` | Credit-based named stream state machine |
| `packages/d2b-session/src/lifecycle.rs` | Keepalive/reconnect/close state |
| `packages/d2b-session/src/transport.rs` | Async owned transport abstraction |
| `packages/d2b-session-unix/src/adapter.rs` | Unix peer identity, atomic attachments, descriptor validation/credits |
| `packages/d2b-session-unix/src/vsock.rs` | Async framed vsock transport, expected CID, no attachments |
| `packages/d2b-session/tests/noise_vectors.rs` | Exact snow 0.10 NN/KK/IKpsk2 vectors and rejection mutations |
| `packages/d2b-session/tests/component_session.rs` | Integrated strict negotiation/record/lifecycle/limits |
| `packages/d2b-session-unix/tests/unix_session.rs` | SO_PEERCRED/SCM_RIGHTS/pidfd/object identity/credit tests |

Unrelated ADR 0045 Provider types, endpoint roles, service inventory, realm
process model, generated v2 DTO names, and delivery assumptions are not copied
as v3 architecture.

## Layering

```text
generated async ttrpc services / named streams
  -> d2b-bus exact addressed route/admission
  -> ComponentSession
       Noise authentication + record protection
       schema/purpose/transport/limits/reconnect binding
       cancellation/deadlines/keepalive
       fair control/stream scheduling
       local typed attachment validation
  -> owned d2b transport
       inherited socketpair / Unix seqpacket / Unix stream / vsock /
       enrolled Zone/provider transport
```

d2b-transport provides carriage/evidence only. ComponentSession authenticates
and protects a session. d2b-bus resolves an exact service/Zone/Provider/
controller route. Native Role/RoleBinding authorizes connect/invoke/stream/
resource verbs. No layer may widen the authority established below it.

## Noise profiles

### Local NN

`Noise_NN_25519_ChaChaPoly_SHA256` is permitted only when:

- purpose class is local;
- transport is inherited socketpair or Unix stream/seqpacket;
- transport provides directional peer identity evidence;
- endpoint policy is trusted/config-generated;
- authenticated Unix peer evidence maps to an exact canonical subject.

NN supplies ephemeral encrypted record protection; peer authentication comes
from the bound Unix evidence, not a peer-supplied subject.

### Enrolled KK

`Noise_KK_25519_ChaChaPoly_SHA256` is used for enrolled peers:

- both static public keys are known before handshake;
- local private keys are sealed/zeroizing;
- static key registry maps the authenticated remote key to one canonical
  `Zone/*`, `Provider/*`, `Process/*`, `Host/*`, or `Guest/*` subject;
- route/purpose/service/schema/limits/channel binding are in the Noise prologue.

### Bootstrap IKpsk2

`Noise_IKpsk2_25519_ChaChaPoly_SHA256` is used only for one-time bootstrap:

- responder static key is known;
- single-use PSK is bound to operation ID, replay nonce, expected subject/
  Host/Guest/Provider/controller purpose, and expiry;
- PSK admission is consumed exactly once;
- successful enrollment replaces bootstrap with an enrolled identity/session;
- replay, expiry, wrong operation/subject/purpose fails closed.

## Handshake policy

The trusted endpoint policy binds:

- endpoint purpose and purpose class;
- service package and exact schema fingerprint;
- initiator/responder evidence requirements;
- expected/authenticated subject constraints;
- selected Noise profile;
- transport class/locality/channel binding;
- reconnect generation;
- exact limit profile;
- attachment policy;
- authorization service/verb requirements.

The canonical preface+offer is the Noise prologue. Any mismatch changes the
transcript and fails. There is no negotiation down to another service, schema,
profile, transport, purpose, role/evidence class, limit, or attachment policy.

Generation discovery remains local-only, exact-policy-bound, and unauthenticated
only to the extent required to learn the nonzero reconnect generation; it
cannot admit a request or authorize a service.

## Authenticated subject

ComponentSession never trusts a subjectRef/role from the peer payload.

Evidence mapping:

| Evidence | Subject source |
| --- | --- |
| Unix pathname/socketpair | trusted endpoint config + verified SO_PEERCRED/process identity maps uid/pid to exact Zone-local subject |
| Enrolled KK key | Zone identity registry maps static public key to exact subject |
| Bootstrap IKpsk2 | one-time admission record contains expected subject and operation |
| Native vsock | expected CID/Guest bootstrap identity plus IKpsk2/enrolled key maps to exact subject |

The established session carries the shared
`AuthenticatedSubjectContext` from the terminology/identity spec, plus an
authorization lease revision/expiry. This spec owns the evidence-to-context
mapping, not a second identity schema.

It is redacted in Debug/logs and cannot be changed without a new handshake.

## Native RBAC integration

After authentication and before service dispatch, the same native
Role/RoleBinding evaluator used by the resource API checks:

```text
subject
Zone
session purpose/service
verb = connect | invoke | open-stream | attach | cancel | observe
method/stream kind
target ResourceRef/Provider/Host/Guest
Provider/controller/session generations
```

Resource API methods add their resource verb attributes.

Rules may authorize ComponentSession connect/service/stream attributes in the
same Role resource as resource verbs. There is no endpoint-local parallel RBAC
language.

Authorization:

- is required for the session itself and each method/stream;
- is revision-bound to Role/RoleBinding/Provider/API/Zone policy;
- is cached only under exact attributes/short expiry;
- invalidates immediately after relevant durable policy commit;
- grants long-lived streams a short lease;
- closes/refuses new work when lease revalidation fails;
- preserves already admitted bounded work only to its original deadline.

Noise authentication proves identity/channel; it does not itself grant a role.

## Records

The copied RecordProtector behavior is retained:

- directional Noise transport keys;
- bounded u16 length-prefixed encrypted record;
- authenticated header containing record kind, channel, sequence, reconnect
  generation, payload length;
- strict send/receive sequence;
- replay digest rejection;
- reconnect-generation rejection;
- no plaintext/status/credential Debug.

The v3 contract versions all record and canonical offer encodings separately
from v2. Protobuf numbers and binary tags are regenerated/frozen; v2 and v3 do
not interoperate.

## Channels and fairness

Closed channel classes:

- session control;
- ttrpc control;
- attachment control;
- named stream.

Session/control/cancel/keepalive/status health traffic has reserved bounded
capacity. Named streams use per-stream and aggregate byte credit plus round-
robin fairness. One blocked terminal/watch stream cannot starve resource status,
cancel, or health.

Controller resource watches and reconcile hints are named streams. Their
delivery/ack cursor remains resource revision, not session sequence.

## Async behavior

All transport/session/service methods are async.

- one driver owns session transport/protector/scheduler;
- service handlers run as independent bounded tasks;
- named streams expose async read/write/credit/cancel;
- blocking fd/kernel/filesystem operations use explicit adapters;
- no nested runtime/block_on;
- cancellation/deadline propagates from bus to handler/effect;
- Process reconcile fast path cannot wait for unrelated stream traffic.

## Attachments

Attachments are local Unix only. They require:

- packet-atomic seqpacket policy;
- encrypted descriptor bound to service/method/request/operation/generation;
- exact object/access/purpose;
- CLOEXEC;
- SO_PEERCRED/pidfd/object identity validation;
- duplicate kernel-object rejection unless explicitly permitted;
- atomic multi-scope credits and cleanup on every failure.

Unix stream/vsock/remote Zone transports carry no SCM_RIGHTS. Destination
controllers re-origin local resources/effects.

### ResourceExport and ResourceImport streams (D096)

Cross-Zone `ResourceExport`/`ResourceImport` payload bytes use the existing
ComponentSession named-stream machinery only: bounded encrypted streams with
per-stream and aggregate credits/backpressure, cancel, deadline, and idempotency,
bound to a per-import session generation. No FD, SCM_RIGHTS attachment, or
resource grant crosses a Zone. Intermediate Zone controllers route only the
encrypted stream records and see ciphertext, never plaintext payload bytes,
device handles, paths, or tokens.

### Fixed listener/endpoint authority (D097)

A fixed listener binding (a stable Unix/vsock/TCP socket or a stable
`Endpoint`/port) is a D097 authority: exactly one authority service owns the
bind. The owning Provider/Resource declares an `AuthorityDescriptor`
(`authorityScope` matching the listener's reach, `cardinality: zero-or-one` per
`(scope, opaque bind-key digest)`, `arbitration: exclusive`), and core's
authority index rejects a second binder of the same listener with the typed
`duplicateConflict` before the bind. The `authorityKey` digests the bind
selector and is never a raw address/path/CID/port in status or audit. The d2b-bus
itself is an `exactly-one` per-Zone singleton authority (see the Zone-control
core authority table). Per-session named-stream and `OwnedTransport` handles
stay internal/high-churn (D092) and are not authorities.

## d2b-bus

d2b-bus is an exact addressed router, not pub/sub.

Route key:

```text
(Zone, service package, method/stream, target ResourceRef or Provider,
 schema fingerprint, Provider/controller/session generation)
```

It:

- resolves local/Host/Guest/parent-child Zone transport;
- binds authenticated subject and operation/deadline;
- checks session connect/service/stream RBAC;
- invokes exact generated ttrpc client/server;
- bridges named streams with bounded credit;
- preserves pinned reverse route/cancellation;
- exposes no global subscriber socket/topic namespace/route table to Providers.

ResourceClient always uses d2b-bus, even beside the Zone runtime. There is no
direct-store shortcut.

## Sensitive credential delivery

Credential resources/status/store/revision/audit/OTEL remain free of token
bytes. A Credential Provider may deliver a raw token only through a dedicated
end-to-end sensitive ComponentSession:

- initiator/responder are fully enrolled Provider/component identities;
- Noise profile is KK; NN and IKpsk2 are forbidden;
- consumerRef may name Provider/<name>; its signed component descriptor and
  Role/RoleBinding resolve the exact receiving component/Process;
- the offer/prologue binds Credential ref/UID/generation, Credential Provider
  and consumer Provider/component generations, audience/operation class,
  route, schema, limits, expiry/deadline, and authorization revisions;
- d2b-bus/Zone/relay intermediaries authorize route establishment but forward
  opaque protected records and cannot terminate/decrypt the inner session;
- token payload has a strict small bound, zeroizing buffers, redacted Debug,
  replay-safe sequence, no logging/audit/metrics, and immediate close/zeroize;
- ambiguous delivery never becomes success and is not automatically replayed
  outside the credential method's explicit idempotency contract.

This is the only initial cross-process secret-byte channel. It grants no generic
raw HTTP, signing, endpoint, or credential forwarding authority.

## Lifecycle

Retain/adapt main's:

- Established/Disconnected/Reconnecting/Closing/Closed;
- monotonic keepalive;
- bounded reconnect window/attempts;
- reconnect generation increment;
- cancel all old-generation requests;
- new Noise handshake/protector after reconnect;
- explicit close reason/remediation.

Resource/controller streams relist/resume using resource revisions after
reconnect; session generation alone is not durable state.

## Errors and telemetry

Stable errors include malformed/unsupported handshake, authentication,
transcript/schema/purpose/transport/evidence/subject mismatch, policy denial,
generation/replay/sequence, limit/backpressure, invalid attachment, timeout,
cancel, disconnect, and internal invariant.

Metrics use closed labels:

- purpose/service/transport/profile;
- operation class;
- outcome/error code;
- stream/attachment class.

Never label subject/resource names, endpoint paths, keys, payload, terminal
bytes, credentials, PIDs, or Provider diagnostics.

Audit records authenticated subject digest/ref where policy permits, endpoint/
service/method, authorization revision/decision, operation/correlation,
transcript/session generation digest, route, and fixed outcome.

## Current-code fit

| Item | Treatment |
| --- | --- |
| v3 current anchor | `d2b-realm-router` PeerSession/SecurePeerSession/MuxSession and `d2b-realm-transport`; guest ttrpc/vsock/HMAC |
| v3 evidence class | Mostly implemented-but-unwired for Realm peer; guest control reachable; ComponentSession absent |
| Main reuse source | main `a1cc0b2d`, `d2b-session`, `d2b-session-unix`, v2 ComponentSession contracts/tests |
| Behavior retained | Strict Noise profiles/transcript, encrypted replay-safe records, async owned transports, fair streams, cancellation/reconnect, exact attachments |
| Required delta | v3 contract names/versions, shared AuthenticatedSubjectContext, Role/RoleBinding authorization, d2b-bus routing, Zone services |
| Excluded main assumptions | v2 EndpointRole/Realm/service inventory, Provider registry/process model, delivery/Nix ownership |
| Feasibility proof | Copy main tests, add subject/RBAC/revocation/resource-watch/latency integration on v3 |
| Future owner | Work items below |

## Implementation work items

### ADR046-session-001

| Field | Value |
| --- | --- |
| Dependency/owner | W0 shared contract root |
| Current source | v3 `d2b-realm-router/src/{session,secure_session,mux_session,lifecycle}.rs`, guest auth/transport |
| Reuse source | main `a1cc0b2d`: `d2b-contracts/src/v2_component_session.rs`, `d2b-session/src/{handshake,bootstrap,record,engine,scheduler,streams,lifecycle,transport}.rs`, Noise/component tests/vectors |
| Reuse action | adapt |
| Destination | `packages/d2b-contracts/src/v3/component_session.rs`, `packages/d2b-session/` |
| Detailed design | Reversion canonical offer/records; retain NN/KK/IKpsk2; add canonical subject/authorization context hooks Primary reuse disposition: `adapt`. Preserved source-plan detail: copy and adapt. |
| Integration | d2b-bus, resource/controller/Provider services |
| Data migration | No v2 session compatibility; reconnect on v3 |
| Validation | Copied exact vectors/rejections plus subject/RBAC/revocation tests |
| Removal proof | v3 old Realm PeerSession removed only after all v3 peer routes move |

### ADR046-session-002

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-session-001; transport owner |
| Current source | v3 guest vsock/local seqpacket implementations and d2b-realm-transport traits |
| Reuse source | main `a1cc0b2d`: `d2b-session-unix/src/{adapter,socket,descriptor,pidfd,vsock,systemd,credit}.rs`, `tests/unix_session.rs` |
| Reuse action | adapt |
| Destination | `packages/d2b-session-unix/`, future enrolled transport adapter crates |
| Detailed design | Unix/socketpair/vsock owned transports, peer evidence, fd/pidfd/object validation, credits Primary reuse disposition: `adapt`. Preserved source-plan detail: copy and adapt. |
| Integration | ProviderSupervisor/Host/Guest/Zone listeners hand owned transports to session |
| Data migration | None — full d2b 3.0 reset; no prior state to migrate |
| Validation | Copied fd/peer/credit tests plus Host/Guest subject mapping |
| Removal proof | Ad hoc guest/public/helper transport removed only per service cutover |

### ADR046-bus-001

| Field | Value |
| --- | --- |
| Dependency/owner | Sessions + resource API; bus owner |
| Current source | v3 `d2b-realm-router`, target resolver, CLI routing, operation router |
| Reuse source | Any useful main d2b-client/provider/session routing symbols named by implementation sub-items |
| Reuse action | adapt |
| Destination | `packages/d2b-bus/src/{router,registry,authorization,streams,operations}.rs` |
| Detailed design | Exact service/resource routes, RBAC, pinned reverse route, cancellation, named stream bridge, no wildcard pub/sub Primary reuse disposition: `adapt`. Preserved source-plan detail: extract/adapt. |
| Integration | Every ResourceClient/controller/Provider/CLI service |
| Data migration | None — full d2b 3.0 reset; no prior state to migrate |
| Validation | Message isolation, route/auth revocation, fairness, reconnect, no direct-store path |
| Removal proof | Old direct dispatch branches removed only after route parity |
