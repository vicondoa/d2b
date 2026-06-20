# Constellation core model reference

**Diataxis category:** reference.

This page documents the committed ADR 0032 constellation core model in
`packages/nixling-constellation-core`. It is a contributor-facing
contract for target names, identifiers, capability checks, redacted audit
metadata, typed errors, and semantic frame schema roots. It does not
describe live validation evidence, provider credentials, or relay
secrets.

The crate is intentionally codec-neutral. Protocol codecs map bytes to
and from `ConstellationFrame`; routing, authorization, audit, and
provider code reason over the Rust model below rather than over a
specific wire encoding.

## Schema-root status

The Rust DTOs derive or implement `serde` and `schemars::JsonSchema`.
They are emitted by `cargo xtask gen-schemas` through the
`ConstellationCoreSchema` wrapper into the generated JSON companion
[`schemas/v2/constellation-core.json`](./schemas/v2/constellation-core.json).
Regenerate that file; do not edit generated JSON by hand.

| Root | Source | Contract |
| --- | --- | --- |
| `ConstellationCoreSchema` | `packages/xtask/src/main.rs` | Generated schema wrapper whose top-level `anyOf` enumerates the committed core roots. |
| `ConstellationFrame` | `src/frame.rs` | Top-level semantic frame enum: handshake proposal/accept/reject, operation request/response, stream open/data/flow/close, typed error, and admission audit. |
| `OperationRequest` / `OperationResponse` | `src/frame.rs` | Operation envelope with target realm/node/workload, authenticated principal, bounded body, trace context, and required idempotency for mutating kinds. |
| `Handshake` / `HandshakeAccepted` / `HandshakeRejected` / `OperationKind` | `src/frame.rs` | Negotiation outcome roots and closed operation taxonomy roots. |
| `AuditEnvelope` | `src/audit.rs` | Redacted post-auth audit metadata for mutating operations and stream opens. |
| `AdmissionAuditRecord` | `src/audit.rs` | Redacted pre-auth/session-admission denial metadata; principal may be absent only in this shape. |
| `AuditChainRecord` / `AuditChainLink` / `AuditHash` | `src/audit.rs` | Tamper-evident audit-chain metadata for gateway, remote-node, and daemon audit streams. |
| `AuditSinkHealth` / `AuditRetentionFloorStatus` | `src/audit.rs` | Redacted audit-sink health and retention-floor status for degraded/fail-closed reporting. |
| `ConstellationError` | `src/error.rs` | Typed error frame with stable `ErrorKind`, bounded message, and structured missing capability for capability denials. |
| `NodeSummary` / `WorkloadSelector` / `WorkloadSummary` / `ExecutionSummary` | `src/node.rs`, `src/workload.rs`, `src/execution.rs` | Bounded status summaries and selectors for nodes, workloads, and durable executions. |
| `RealmPath`, identifier newtypes, `CapabilitySet`, `TraceContext`, `OpaquePayload`, `ProtocolToken` | `src/realm.rs`, `src/ids.rs`, `src/capability.rs`, `src/trace_context.rs`, `src/payload.rs`, `src/token.rs` | Reusable bounded primitives that every higher-level root depends on. |

## Target addresses

A constellation target address names a workload on a node inside a
realm. It is **not** a network address and does not imply DNS, SSH, IP
reachability, socket reachability, or an overlay route.

Canonical persisted form:

```text
nl://<workload>.<node>.<realm-path>.nixling
```

Accepted human forms:

| Form | Meaning |
| --- | --- |
| `<workload>` | v1-compatible local workload on the current node in the `local` realm. |
| `<workload>.nixling` | Explicit local workload on the current node in the `local` realm. |
| `<workload>.<node>.nixling` | Workload on a named local-realm node. |
| `<workload>.<node>.<realm>.nixling` | Workload on a node in a named realm. |
| `<workload>.<node>.<child>.<parent>.nixling` | Workload in a nested realm, written most-specific first. |

Parsing is fail-closed. Multi-label human forms require the reserved
`.nixling` suffix, `nl://` forms must be fully qualified, and `all`,
`*`, and non-suffix `nixling` labels are rejected as target labels.

## Identifier families

Every identifier constructor validates input and every serde decode path
routes through the same validator.

| Family | Types | Shape |
| --- | --- | --- |
| Label-shaped ids | `RealmId`, `NodeId`, `WorkloadId`, `ProviderId` | `^[a-z][a-z0-9-]*$`, 1-128 bytes. |
| Opaque ids | `GatewayId`, `ExecutionId`, `StreamId`, `StreamCursor`, `PrincipalId`, `OperationId`, `IdempotencyKey` | URL/filename-safe printable ASCII, 1-128 bytes; path-like and credential-shaped tokens are rejected by the Rust validators. |
| Protocol tokens | `ProtocolToken` | printable ASCII without spaces, 1-64 bytes. |
| Trace fields | `TraceContext.trace_id`, `TraceContext.span_id` | printable ASCII without spaces, 1-64 bytes each. |

`RealmPath` is a non-empty list of `RealmId` labels, written
most-specific first for target names and bounded to 16 labels / 255
rendered bytes. `RealmPath::storage_form()` renders the parent-first
storage key.

## Capabilities

Capabilities are positive assertions. A node, provider, workload, or
stream advertises exactly what it supports; absence means typed refusal,
not a silent fallback.

Capability codes:

- lifecycle, exec, pty, logs, file-copy, port-forward;
- vsock, virtiofs;
- window-forwarding, display-streaming, clipboard;
- audio-playback, audio-capture;
- hid, usb;
- gpu-accel, snapshots, hotplug, ephemeral-sessions,
  provider-managed-isolation.

Display, clipboard, audio, HID, and USB are deliberately independent so
one capability cannot smuggle another. Local GPU acceleration is not
automatically relay-exportable.

## Peer protocol handshake

`Handshake` is the codec-neutral peer-session proposal/selection shape.
It carries the protocol version, codec id, codec schema fingerprint, and
an optional non-secret peer-binding context. Plain peer sessions exchange
an explicit `HandshakeAccepted` or `HandshakeRejected`; secure sessions
bind version, codec id, schema fingerprint, authenticated identity, and
both nonces into the HMAC transcript before encrypted frames are accepted.

The protobuf codec advertises a bounded schema fingerprint via
`ProtocolCodec::schema_fingerprint()`. A version, codec, schema, identity,
or channel-binding mismatch fails closed before operation or stream frames
can be routed.

## Operation authorization and idempotency

`OperationKind` is a closed, typed enum. The required capability is
derived in trusted code from the operation kind; peers never provide the
required capability as a wire field.

Mutating operation kinds require an `IdempotencyKey` at decode time so
the gateway/router can deduplicate at-least-once delivery before any side
effect. The dedup fingerprint includes the request-identifying fields
(`kind`, `realm`, `node`, `workload`, `principal`, and body) and excludes
per-attempt correlation (`operation_id`), the idempotency key itself, and
trace metadata.

## Stream authorization and flow control

Every stream has a `StreamDescriptor` and `StreamAuthz`. `StreamAuthz`
is built from the authenticated principal, realm path, and a capability
derived from `StreamKind`. `StreamOpen` rejects mismatched
kind/capability pairs at decode.

The pure `StreamMux` state machine enforces:

- stream-open before data;
- authz consistency on open;
- an open-stream cap;
- strictly increasing per-stream sequence numbers;
- credit-based backpressure through non-zero `StreamFlow` grants;
- deterministic sendable-stream selection for fair draining;
- `Stdout`/`Stderr` channels only on `Stdio` streams;
- resume cursors only on `Logs` streams;
- `StreamResume` only on resumable stream kinds;
- idempotent cancellation retries for already-cancelled streams;
- no data after close and no double close for non-cancel terminal states.

## Audit and error redaction

`AuditEnvelope` is the post-auth audit shape and carries bounded metadata
only:

- operation id;
- realm;
- principal;
- node;
- optional workload, stream, execution, and trace context;
- authorization scope;
- allow/deny decision.

It never carries argv, stdio, log bytes, Wayland buffers, secrets, store
paths, or provider credential material. Decode rejects any audit envelope
without a principal. Pre-auth/session-admission failures use
`AdmissionAuditRecord`, whose principal is optional and whose decision is
always `deny`.

`AuditChainRecord` describes the tamper-evident metadata attached to
gateway, remote-node, and daemon audit streams. The core crate validates
hash shape (`sha256:<64 lowercase hex chars>`) and verifies a link against
trusted recomputed previous, payload, and record hashes; hash computation
is owned by the concrete daemon/gateway/provider crate so the core model
stays codec- and host-neutral. `AuditSinkHealth` and
`AuditRetentionFloorStatus` report bounded degraded/unavailable states and
never carry paths, credential material, argv, stdio, or raw provider
errors.

`ConstellationError` carries a stable `ErrorKind`, a bounded
operator-safe message, and a structured missing capability when
`kind == capability-denied`. Decode rejects a capability denial without
the structured capability.

## Related references

- [ADR 0032 — nixling v2 constellation control plane](../adr/0032-nixling-v2-constellation-control-plane.md)
- [Constellation peer protocol reference](./constellation-protocol.md)
- [Daemon API reference](./daemon-api.md)
- [Naming conventions](./naming-conventions.md)
- [Manifest bundle reference](./manifest-bundle.md)
