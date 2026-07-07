# Realm core model reference

**Diataxis category:** reference.

This page documents the committed `d2b-realm-core` DTO and parser
contract. It is a contributor-facing reference for ADR 0043 realm target
names, identifiers, capability checks, redacted audit metadata, typed
errors, realm-controller metadata, route/enrollment DTOs, host-resource
allocator DTOs, and semantic frame schema roots. See
[Local-root allocator contract](./local-root-allocator.md) for the
allocator-specific invariants and current implementation boundary.

For the current Nix option surface, see
[Realm option schema](./realm-options.md). The `d2b.realms.<realm>`
namespace validates realm declaration shape only in the current release;
it does not yet instantiate the per-realm runtime described by the DTO
model below.

The crate is intentionally codec-neutral. Protocol codecs map bytes to
and from `ConstellationFrame`; routing, authorization, audit, and
provider code reason over the Rust model below rather than over a
specific wire encoding.

## Schema-root status

The Rust DTOs derive or implement `serde` and `schemars::JsonSchema`.
They are emitted by `cargo xtask gen-schemas` through the
`D2bRealmCoreSchema` wrapper into the generated JSON companion
[`schemas/v2/d2b-realm-core.json`](./schemas/v2/d2b-realm-core.json).
Regenerate that file; do not edit generated JSON by hand.

| Root | Source | Contract |
| --- | --- | --- |
| `D2bRealmCoreSchema` | `packages/xtask/src/main.rs` | Generated schema wrapper whose top-level `anyOf` enumerates the committed core roots. |
| `RealmTarget` | `src/target.rs` | ADR 0043 workload-in-realm target string, serialized in canonical `<workload>.<realm>[.<ancestor>...].d2b` form. |
| `ConstellationFrame` | `src/frame.rs` | Top-level semantic frame enum: handshake proposal/accept/reject, operation request/response, stream open/data/flow/close, typed error, and admission audit. |
| `OperationRequest` / `OperationResponse` | `src/frame.rs` | Operation envelope with target realm/node/workload, authenticated principal, bounded body, trace context, and required idempotency for mutating kinds. |
| `Handshake` / `HandshakeAccepted` / `HandshakeRejected` / `OperationKind` | `src/frame.rs` | Negotiation outcome roots and closed operation taxonomy roots. |
| `AuditEnvelope` | `src/audit.rs` | Redacted post-auth audit metadata for mutating operations and stream opens. |
| `AdmissionAuditRecord` | `src/audit.rs` | Redacted pre-auth/session-admission denial metadata; principal may be absent only in this shape. |
| `AuditChainRecord` / `AuditChainLink` / `AuditHash` | `src/audit.rs` | Tamper-evident audit-chain metadata for gateway, remote-node, and daemon audit streams. |
| `AuditSinkHealth` / `AuditRetentionFloorStatus` | `src/audit.rs` | Redacted audit-sink health and retention-floor status for degraded/fail-closed reporting. |
| `ConstellationError` | `src/error.rs` | Typed error frame with stable `ErrorKind`, bounded message, and structured missing capability for capability denials. |
| `RealmControllerPlacement`, `RealmAccessBinding`, `RealmTransportBinding`, `AccessBindingRef`, `UnixSocketPath` | `src/realm.rs`, `src/access.rs` | ADR 0043 controller placement and access-binding DTOs for future realm access discovery. |
| `ProviderRegistryEntry`, `WorkloadPlacement`, `WorkloadPlacementSummary`, `RealmTreeEdge`, `DescendantRoute`, `RouteAdvertisement` | `src/registry.rs`, `src/routing.rs` | Provider/workload placement and tree-route metadata. |
| `EnrollmentRecord`, `RevocationRecord`, `KeyPin`, `SignatureRef`, migration DTOs | `src/enrollment.rs`, `src/routing.rs`, `src/migration.rs` | Realm identity lifecycle, signed-route metadata, and typed migration-error envelopes. |
| `LeaseAllocationRequest` / `LeaseAllocationResponse`, `AllocatorLease`, `ReconciliationReport`, allocator event DTOs | `src/allocator.rs` | Contract-only local-root host-resource leases, total acquisition order, reconciliation/quarantine/reclaim decisions, and bounded allocator observability metadata. |
| `NodeSummary` / `WorkloadSelector` / `WorkloadSummary` / `ExecutionSummary` / `ShellSummary` | `src/node.rs`, `src/workload.rs`, `src/execution.rs`, `src/shell.rs` | Bounded status summaries and selectors for nodes, workloads, durable executions, and persistent shells. |
| `ExecStartRequest` / `ExecAttachRequest` / `ExecLogsRequest` / `ExecCancelRequest` | `src/execution.rs` | Bounded durable-execution metadata for start, reconnect, retained logs, and retry-safe cancel. |
| `ShellListRequest` / `ShellAttachRequest` / `ShellDetachRequest` / `ShellKillRequest` / `ShellListResponse` / `ShellAttachSummary` | `src/shell.rs` | Bounded persistent-shell metadata for list, attach, detach, kill, list responses, and shell-authorized PTY attachment. |
| `RealmPath`, identifier newtypes, `CapabilitySet`, `CapabilityNegotiation`, `TraceContext`, `OpaquePayload`, `ProtocolToken` | `src/realm.rs`, `src/ids.rs`, `src/capability.rs`, `src/trace_context.rs`, `src/payload.rs`, `src/token.rs` | Reusable bounded primitives that every higher-level root depends on. |

## Target addresses

A realm target address names a workload inside a realm. It is **not** a
network address and does not imply DNS, SSH, IP
reachability, socket reachability, or an overlay route.

Canonical serialized form:

```text
<workload>.<realm>[.<ancestor>...].d2b
```

Examples:

```text
builder.dev.d2b
browser.work.d2b
api.payments.work.d2b
```

The first label is the workload. The remaining labels before `.d2b`
are the realm path, written most-specific first. `RealmTarget::parse`
requires at least one realm label and rejects bare workload names;
callers that intentionally support bare aliases must use
`RealmTargetParser` with an explicit default realm or alias table.

Parsing is fail-closed. Fully qualified public targets require the
reserved `.d2b` suffix. `all`, `*`, and non-suffix `d2b` labels are
rejected as target labels. Old ADR 0032 node-qualified targets are
accepted only by the legacy diagnostic parser so migration tooling can
produce a typed error and suggested ADR 0043 target; new routing code
must not treat node labels as part of the public target grammar.

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

Negotiated capability sets carry `CapabilityNegotiation` metadata: a schema
version, the selected positive assertions both peers understand and
support, and a deterministic bounded fingerprint for audit correlation.
Operations and streams that require absent
capabilities are rejected before execution with typed missing-capability
errors.

Capability codes:

- lifecycle, exec, pty, logs, file-copy, port-forward;
- `persistent-shell` for named persistent shell operations and
  shell-authorized PTY streams;
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

## Persistent shell routing

ADR 0039 persistent shell is part of the generated realm-core contract.
It is a semantic operation family, not durable exec and not a
provider-native shell channel.

| Operation | Mutates state | Required capability | Notes |
| --- | --- | --- | --- |
| `ShellList` | No | `persistent-shell` | Returns bounded shell summaries for the target workload. |
| `ShellAttach` | Yes | `persistent-shell` | Creates/adopts the named shell and authorizes one `shell-pty` terminal stream. |
| `ShellDetach` | Yes | `persistent-shell` | Detaches a live or stale attach handle without killing the named shell. |
| `ShellKill` | Yes | `persistent-shell` | Terminates the named shell session. |

`ShellAttach`, `ShellDetach`, and `ShellKill` require an idempotency key just
like other mutating operation kinds. `ShellList` is read-only and does not.

Shell terminal streams use `StreamKind::ShellPty`, whose required capability is
`persistent-shell`. Generic `Pty` remains available for non-shell terminal uses,
but it does not authorize persistent shell attach. `StreamOpen` decode rejects a
`shell-pty` descriptor paired with any capability other than
`persistent-shell`.

The shell DTOs carry only bounded metadata: validated 64-byte shell names,
generation tokens, state/cause enums, opaque attach/session ids, stream ids, and
bounded summaries. They do not contain terminal bytes, argv, environment, cwd,
provider endpoints, provider resource ids, credentials, raw helper output, or
paths. Unknown fields are rejected on shell request/summary decode.

## Durable execution

Durable execution metadata is keyed by `ExecutionId` and carries an
`ExecutionGeneration` binding. Reconnect/attach requests must match the
generation that created the execution; stale boot or workload-generation
tokens fail closed before a stream is opened. `ExecLogsRequest` carries a
non-zero byte bound and an optional retained cursor. `ExecCancelRequest` is
idempotent so lost-reply and reconnect retries can safely repeat cancel
without turning an already-terminal execution into a protocol error.

`ExecutionSummary` carries only bounded metadata: execution id, workload,
state, exit code, TTY flag, generation, attach mode, and retained cursors.
Argv, env, cwd, stdio, and log bytes remain operation/stream payloads and
are not schema fields.

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

- [ADR 0032 — d2b v2 constellation control plane](../adr/0032-d2b-v2-constellation-control-plane.md)
- [ADR 0039 - constellation persistent shell routing](../adr/0039-constellation-persistent-shell-routing.md)
- [Constellation peer protocol reference](./constellation-protocol.md)
- [Daemon API reference](./daemon-api.md)
- [Naming conventions](./naming-conventions.md)
- [Realm option schema](./realm-options.md)
- [Manifest bundle reference](./manifest-bundle.md)
