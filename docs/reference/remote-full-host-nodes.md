# Remote full-host nodes

**Diataxis category:** reference.

**Status: experimental / preview.** The remote full-host node adapter
proves gateway-managed registration and routing semantics with
mock and loopback peer clients only. Production transports (QUIC, SSH,
Azure Relay over a live WAN, remote host install, and remote host
prepare) are not yet supported. Do not depend on this interface for
production workloads.

This page documents the committed adapter model for gateway-managed
remote nixling hosts: registration, heartbeat/liveness, capability
gating, operation routing, remote-side deduplication and idempotency,
disconnect/reconnect, authentication/principal binding, audit shape, and
the non-tunneling boundary. For the constellation core model (frame
schema, operation kinds, idempotency, stream authz) see
[constellation core](./constellation-core.md). For the transport layer
(loopback, local TCP, Azure Relay) see the
[transport conformance matrix](./transport-conformance-matrix.md) and the
[transport support policy](./transport-support-policy.md). For host
substrate capabilities see [host substrate providers](./host-substrate-providers.md).
The architectural rationale is in
[ADR 0032](../adr/0032-nixling-v2-constellation-control-plane.md).
For provider-managed sandboxes — nodes whose lifecycle is owned by a
cloud provider API rather than by a locally managed `nixling-priv-broker`
— see [provider-managed sandboxes](./provider-managed-sandboxes.md).

---

## What a remote full-host node is

A remote full-host node is a host running its own `nixlingd`,
`nixling-priv-broker`, and guest-control stack that a gateway guest
can reach through a transport peer session. From the gateway's point
of view, the remote host appears as a named `NodeId` in a realm with
a declared capability set. All lifecycle, broker, and guest-control
work executes on the remote host itself; the gateway routes typed
operation requests and receives typed responses.

This model is distinct from a provider-managed sandbox (which has no
local broker or systemd) and from a gateway guest VM (which runs inside
the gateway host's hypervisor). Remote full-host nodes are
substrate-independent once the host's substrate provider has advertised
the required capabilities.

---

## Registration

A remote host registers with the gateway by opening an authenticated peer
session over a supported transport and presenting a bounded registration
record. The preview adapter keeps this record in router state rather than
adding a public registration frame shape. The registration payload carries:

| Field | Constraint |
| --- | --- |
| `node_id` | Stable, operator-assigned label (`^[a-z][a-z0-9-]*$`). Must be unique within the realm. |
| `realm_path` | The realm the node is joining. Must match the realm the gateway is managing. |
| `capabilities` | Positive-assertion capability set derived from the host's substrate provider report. No missing capabilities appear in this set. |
| `substrate_adapter` | Opaque identifier for the host substrate adapter (`nixos-host-substrate` or `generic-linux-host-substrate`). |
| `gateway_principal` | Authenticated gateway principal bound to the peer session. |
| `gateway_node` | Authenticated gateway node bound to the peer session, distinct from the registered full-host node. |
| `generation` | Bounded non-secret token for the remote node's current boot/generation. |

The gateway rejects registration if:

- the `node_id` conflicts with an existing active node in the realm;
- the `realm_path` does not match the gateway's managed realm;
- the node kind is not `NodeKind::FullHost`;
- the authenticated gateway principal is not authorized for the realm/node
  (see [Authentication and principal binding](#authentication-and-principal-binding));
- the transport session fails frame-cap or codec/version negotiation.

A rejected registration produces a `ConstellationError` with
the closest existing `ErrorKind` (`InvalidTarget`, `Unauthorized`,
`Backpressure`, or `CapabilityDenied`), a bounded human-readable message,
and a `missing_capability` field when the denial is capability-driven. No
registration state is persisted on rejection.

Successful registration is idempotent for the same (`node_id`,
`realm_path`, `generation`) tuple. A same-node registration with a new
generation supersedes the old generation and marks old-generation pending
operations stale before they can reach the new generation.

---

## Heartbeat and liveness

After registration, the remote node sends periodic heartbeat frames to
signal liveness. The gateway treats a node as degraded when no heartbeat
arrives within the configured liveness window and as disconnected when
the transport peer session closes or is lost.

| State | Gateway behavior |
| --- | --- |
| Registered, heartbeats arriving | Node is eligible for operation routing. |
| Degraded (missed heartbeats, transport still open) | New routing requests for this node receive `ErrorKind::GatewayUnavailable` with reason `remote-node-unavailable`. In-flight operations reconcile through idempotency before retry. |
| Disconnected (transport closed) | Node transitions to unavailable immediately. Pending mutating operations stay unknown-result until reconnect can query remote idempotency/durable state. |

Heartbeat metadata carries only the bounded generation token and local
monotonic time in router state. It does not carry credentials, paths, host
metrics, or topology data.

---

## Capability gating

Every operation routed to a remote node is validated against the node's
registered capability set before the gateway sends the request. The
gateway derives the required capability from the `OperationKind` in
trusted code; remote nodes never provide the required capability as a
wire field.

If a required capability is absent from the node's registered set, the
gateway returns `ErrorKind::CapabilityDenied` with a
`missing_capability` field to the requesting peer. The operation request
is never forwarded to the remote node.

Capabilities are positive assertions. A node that has not advertised
`lifecycle` cannot receive workload list/start/stop requests. A node that
has not advertised `exec` cannot receive exec start/attach/cancel requests.
A node that has not advertised `logs` cannot receive retained-log requests.

Capability sets are fixed at registration time for a given transport
session. A node that gains or loses a substrate capability must
disconnect and re-register to update its advertised set.

---

## Operation routing

The gateway routes `OperationRequest` frames to the remote node over the
established transport session. Routing proceeds as follows:

1. The gateway validates the authenticated gateway principal against the
   operation kind and realm policy (see
   [Authentication and principal binding](#authentication-and-principal-binding)).
2. The gateway checks the node's capability set (see
   [Capability gating](#capability-gating)).
3. The gateway routes through its shared operation router and preserves the
   idempotency key for mutating operations.
4. The gateway sends the semantic `OperationRequest` to the remote peer
   client. The adapter never exposes a raw byte/frame tunnel to callers.
5. The remote node's local `nixlingd` receives the operation request,
   enforces its own realm/capability policy, and invokes the local
   broker or guest-control path.
6. The remote node returns a semantic response to the gateway.
7. The gateway records the result in dedup state and returns the response to
   the requesting peer.

The gateway never forwards:

- raw broker operation frames or payloads;
- guest-control frames or vsock data;
- pidfds or file descriptors;
- host paths, endpoint strings, or socket addresses;
- relay, provider, or realm credentials;
- command arguments, environment, cwd, or stdio bytes as routing
  metadata (these remain opaque operation/stream payloads);
- authentication tokens or principal assertions beyond the constellation
  authz envelope.

The remote host re-originates all side effects through its own
`nixlingd`/broker/guest-control stack. The gateway is a routing
intermediary, not an execution proxy.

---

## Remote-side deduplication and idempotency

Mutating operation kinds carry a required `IdempotencyKey` (see
[constellation core — operation authorization and idempotency](./constellation-core.md#operation-authorization-and-idempotency)).
The gateway deduplicates at-least-once delivery before forwarding. The
remote node performs a second deduplication layer against its own
in-memory idempotency store:

- A same-key same-operation request within the dedup window returns the
  retained `OperationResponse` without re-executing the side effect.
- A same-key request with different operation fields (`kind`, `realm`,
  `node`, `workload`, `principal`, or body) fails closed with
  `ErrorKind::IdempotencyKeyConflict`.
- Dedup entries are retained for the duration of the transport session
  plus a bounded grace window. They are not persisted across remote node
  restarts.

Non-mutating operations (read, inspect, list) are not idempotency-keyed
and may be retried freely.

---

## Disconnect and reconnect

When a transport peer session is lost the remote node is marked
unavailable immediately; the heartbeat timeout is only a fallback for
silent peers. New routing requests for the node receive
`ErrorKind::GatewayUnavailable` with reason `remote-node-unavailable`.

For unacknowledged mutating operations, reconnect recovery first queries
the remote node's idempotency or durable-exec state with the same
idempotency key and generation. The gateway never blindly resends a
side-effecting operation that may have already started. Same-node
re-registration with a new generation supersedes the old generation; old
generation pending operations fail with reason `stale-node-generation`
before they can reach the new generation.

---

## Authentication and principal binding

The gateway authenticates the transport peer session using the
peer-session handshake mechanism described in
[constellation core — peer protocol handshake](./constellation-core.md#peer-protocol-handshake).
For the preview adapter, the authenticated principal is obtained from the
`PeerContext` established during handshake, or from an injected test
equivalent in hermetic test environments.

Key invariants:

- **Relay identity is reachability only.** A relay-authenticated peer is
  a transport endpoint. Relay credentials never map to a constellation
  principal and never authorize nixling lifecycle or broker operations.
  Relay transport grants only the ability to reach the gateway's relay
  rendezvous point; all operation authorization is based on the
  `OperationRequest`'s authenticated principal and realm policy.
- **Principal binding is per operation.** For the preview adapter,
  `OperationRequest.principal` is the authenticated gateway principal
  derived from the handshake context and must match the peer-session
  principal under the router invariant. End-user impersonation/delegation
  is out of scope until a typed delegated-principal field and policy model
  land. A long-lived transport session does not cache authorization
  decisions.
- **No local `Admin` promotion.** A relay-authenticated or
  constellation-authenticated principal is never mapped to the local
  `nixling` group or to the broker's `Admin` peer credential. Local
  lifecycle authorization remains `SO_PEERCRED` + `nixling` group
  membership on `public.sock`. Remote principals may hold constellation
  enrollment, lifecycle, exec, logs, or other realm-scoped authorization;
  they do not inherit host-local privileges.
- **Node agent service principals.** A remote node may register using a
  dedicated node-agent service principal rather than a user principal.
  The principal identity is carried in the constellation authz/audit
  envelope and does not replace the local peer identity used for
  `SO_PEERCRED` authorization on the remote host's `public.sock`.

In hermetic test environments the `PeerContext` is supplied by an
injected test authenticator that binds a fixed principal to the session
without opening a real relay or network connection.

---

## Audit shape

Every mutating operation routed through the gateway produces an
`AuditEnvelope` on the gateway and a corresponding `OpAuditRecord` on
the remote host. The two records share a `trace_id` for correlation.

The preview implementation exposes `RemoteRoute` / `RemoteNodeError`
metadata that is safe to feed into gateway and remote audit records:

| Field | Value |
| --- | --- |
| `node_id` | The registered node label. |
| `realm_path` | The target realm. |
| `principal` | The authenticated gateway principal (bounded, redacted). |
| `operation_kind` | The closed `OperationKind` enum variant. |
| `capability_fingerprint` | Bounded fingerprint of the registered capability set. |
| `outcome` | Low-cardinality result kind such as `accepted`, `remote-node-unavailable`, `capability-denied`, or `stale-node-generation`. |
| `trace_id` / `span_id` | From the bounded `TraceContext`, when present. |

The remote host continues to record its own broker and daemon audit records
for locally re-originated side effects. Gateway and remote records correlate
through the operation id and trace context; the gateway does not write remote
host audit files.

Audit records never include:

- relay or provider credentials;
- transport endpoint addresses or socket paths;
- operation payloads, command arguments, environment, or stdio;
- host paths, closures, or store hashes.

The `AuditChainRecord` / `AuditChainLink` tamper-evident chain covers
both the gateway and remote-node audit streams (see
[constellation core — audit and error redaction](./constellation-core.md#audit-and-error-redaction)).

---

## Error and remediation shapes

Errors from the remote full-host node adapter use the standard
`ConstellationError` shape:

| Adapter reason | `ErrorKind` | Meaning | Remediation hint |
| --- | --- | --- |
| `wrong-realm` / `wrong-node` | `InvalidTarget` | Request targets a node outside the registered realm/node. | Check the target realm and node enrollment. |
| `not-full-host` | `InvalidTarget` | Registration used a gateway or provider-managed node kind. | Register only full nixling hosts through this adapter. |
| `unauthorized-gateway` | `Unauthorized` or `InvalidTarget` | Peer/session principal is not authorized for the realm/node. | Enroll the gateway principal for this realm and node. |
| `duplicate-registration` | `InvalidTarget` | Same node/generation conflicts with existing metadata. | Re-register with the current generation or remove the conflicting node. |
| `stale-node-generation` | `InvalidTarget` | Operation or heartbeat targets an old generation. | Refresh remote node registration before retrying. |
| `registry-capacity-exceeded` / `dedup-capacity-exceeded` | `Backpressure` | Registry or dedup bounds would be exceeded. | Remove stale nodes, wait for in-flight operations, or raise the configured capacity. |
| `missing-idempotency-key` | `InvalidTarget` | Mutating operation arrived without an idempotency key. | Retry with an idempotency key. |
| `idempotency-key-conflict` | `IdempotencyKeyConflict` | Same key presented with different operation fields. | Use a fresh idempotency key or retry the original request. |
| `idempotency-key-expired` | `IdempotencyKeyExpired` | Key was reused after its retention window. | Start a new operation with a fresh idempotency key. |
| `missing-workload` | `InvalidTarget` | A workload or execution operation omitted the workload target. | Target a workload for workload or execution operations. |
| `remote-operation-unknown` | `GatewayUnavailable` | Reconnect reconciliation found no matching remote-side operation state. | Retry after the remote node reconciles operation state. |
| `remote-node-unavailable` | `GatewayUnavailable` | Node is disconnected or stale. | Check the remote daemon peer session and re-register the node. |
| `capability-denied` | `CapabilityDenied` | Required capability absent from the node's registered set. | Check the substrate provider report and re-register after resolving capability gaps. |
| `unsupported-operation` | `UnsupportedFeature` | Operation is outside the preview remote full-host adapter scope. | Use a supported operation or wait for later capability support. |

All `ConstellationError` messages are bounded and do not include host
paths, command output, credential details, or internal identifiers.

---

## Non-tunneling boundaries

The following items are explicitly outside the remote full-host node
adapter's scope. Requests for these operations fail closed; the adapter
does not implement fallbacks or workarounds.

| Surface | Boundary |
| --- | --- |
| Raw broker operation forwarding | The gateway never forwards raw `nixling-priv-broker` frames. All broker work stays on the remote host. |
| Guest-control frame tunneling | Guest-control (vsock) frames are not proxied through the gateway. The remote `nixlingd` opens its own guest-control sessions. |
| Pidfd / fd forwarding | File descriptors, pidfds, and socket handles are never sent across the transport session. |
| Host path and endpoint exposure | Host-local paths, socket addresses, runner argv, and endpoint strings are not visible in the operation envelope or in gateway audit records. |
| Provider/relay credential forwarding | Transport and realm credentials remain in the layer that owns them and are never placed in operation payloads. |
| Remote host install / host prepare | Registration assumes the remote host is already running a compatible `nixlingd`/`nixling-priv-broker` stack. Host installation and host preparation are out of scope for this adapter. |
| Production WAN transports | The preview is validated with mock and loopback peer clients only. Azure Relay over a live WAN, QUIC, and SSH are not yet connected. See [transport support policy](./transport-support-policy.md). |
| Network mutation | The adapter does not configure routing, firewall rules, or overlays on the remote host or on the gateway network. |
| SSH fallback | There is no implicit SSH fallback when the peer session transport is unavailable. Callers receive a typed `GatewayUnavailable` / transport-layer refusal. |

---

## Scope limitations of the preview

The preview adapter is validated against mock and loopback peer clients
only. The following items are deferred to later work:

- Production transport connectors (Azure Relay rendezvous, QUIC, SSH
  bootstrap).
- Live Internet reachability and WAN NAT traversal.
- Remote host installation and remote `nixling host prepare`.
- Remote display, audio, USB, and device streams to/from the remote host.
- Provider-provisioned remote hosts (see the
  [provider-managed sandbox](./provider-managed-sandboxes.md)
  model for provider-scoped work).
- Automatic capability refresh without re-registration.
- End-user principal delegation across a gateway; the preview binds the
  authenticated gateway principal only.
- Multi-gateway realm federation with remote full-host nodes.

These limitations are documented here and not gated by runtime checks; the
adapter does not advertise capabilities it has not implemented. Operators
evaluating production use cases should wait for transport connectors and
live-host validation to land before relying on remote full-host node
routing.
