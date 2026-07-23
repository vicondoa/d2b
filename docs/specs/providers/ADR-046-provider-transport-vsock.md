# ADR 0046 Provider dossier: transport-vsock

| Field | Value |
| --- | --- |
| Spec ID | `ADR-046-provider-transport-vsock` |
| Parent | ADR 0046 |
| Status | Proposed |
| Version | 2 |
| Baseline | `b5ddbed67867d9244bf33390868101bd9b053e49` |
| Normative | Yes |
| Owners | `d2b-provider-transport-vsock` crate; `d2b-session-unix` vsock adapter |
| Depends on | `ADR-046-terminology-and-identities`, `ADR-046-componentsession-and-bus`, `ADR-046-zone-routing`, `ADR-046-provider-model-and-packaging`, `ADR-046-resources-zone-control`, `ADR-046-resource-api-and-authorization`, `ADR-046-resources-host-guest-process-user`, `ADR-046-provider-state` |
| Supersedes | `packages/d2bd/src/guest_control_vsock.rs` transport probe (→ allocator-issued endpoint via VsockEffectPort); `packages/d2b-host/src/vsock_relay_argv.rs` socat relay (→ native FramedVsockTransport in Provider service); `NativeVsock`/`CloudHypervisorVsock` in `d2b-session-unix/src/vsock.rs` (→ FramedVsockTransport implementing OwnedTransport adapted as service-Provider transport bridge) |

## Source and reuse policy

The pre-ADR-0045 v3 baseline has no generic vsock transport Provider or
ZoneLink-aware vsock session. All existing vsock code is scoped to either
the guest-control ttrpc channel (port 14318 only, HMAC-bound) or the
observability socat relay (ports 14317/14319, not authenticated).

Main commit `a1cc0b2da4a08ca3240a770a972fe4da6f912bef` contains a `FramedVsockTransport`
and `NativeVsockListener`/`NativeVsockTransport` in
`packages/d2b-session-unix/src/vsock.rs`. These implement `OwnedTransport`
and are the primary reuse baseline for v3. They are **not** current v3
behavior and carry ADR 0045 endpoint-role assumptions that must be excluded.

Every reuse entry records the exact file/symbol/test, the selected
behavior, the v3 destination, and which ADR 0045 assumptions are excluded.

---

## Main commit reuse inventory

All sources in this section are from main commit
`a1cc0b2da4a08ca3240a770a972fe4da6f912bef`. They are **not** present in
the pre-ADR-0045 v3 baseline. Do not cite them as v3 baseline behavior.

### `d2b-session-unix/src/vsock.rs` — framed vsock transport

| Symbol | Selected behavior |
| --- | --- |
| `FramedVsockTransport` | Implements `OwnedTransport` over `AF_VSOCK`; 2-byte big-endian length-prefixed framing; async tokio-vsock send/receive; no SCM_RIGHTS (no attachment support); `TransportDescriptor::class=Vsock`, `locality=NonLocal`, `atomic_transfer=false`, `attachment_support=false` |
| `NativeVsockTransport` | Wraps a connected `tokio-vsock` stream as an `OwnedTransport`; per-frame bounded allocation; graceful vs. unclean EOF distinction |
| `NativeVsockListener` | Binds `AF_VSOCK VMADDR_PORT_ANY` then hands accepted streams to callers; per-accept CID verification against expected CID range |
| `VsockTransportError` (12 variants) | `BindFailed`, `ConnectFailed`, `CidMismatch`, `PortMismatch`, `FrameTooLarge`, `UnexpectedEof`, `WriteTimeout`, `ReadTimeout`, `ConnectionReset`, `ProtocolError`, `Backpressure`, `Shutdown` |
| `VsockEndpointPolicy` | `ExpectedCid { cid: u32 }` enforces that the connected peer CID equals the pre-negotiated value; connection from a mismatched CID is closed with `CidMismatch` without processing any bytes |

**V3 destination — split by ownership**:

- `FramedVsockTransport` framing utilities (2-byte length-prefix encode/decode,
  bounded frame allocation, EOF/reset classification) → `packages/d2b-provider-transport-vsock/src/framing.rs`.
  These contain no raw AF_VSOCK syscall calls; they operate on any `AsyncRead+AsyncWrite`.
- `NativeVsockTransport` / `NativeVsockListener` (raw `AF_VSOCK socket()`,
  `connect()`, `bind()`, `accept()`) → `packages/d2b-core-controller/` as the
  `LiveVsockEffectPort` implementation. These are NOT copied to the Provider crate;
  the Provider never calls AF_VSOCK syscalls.
- `VsockTransportError` variants → `packages/d2b-provider-transport-vsock/src/errors.rs`
  (framing/bridge errors only; raw-socket error variants stay in the core adapter).

**Tests** (`packages/d2b-session-unix/tests/unix_session.rs` — vsock subset):

| Test function | Covers | v3 destination |
| --- | --- | --- |
| `vsock_framing_handles_partial_and_coalesced_records` | 2-byte prefix reassembly, partial read, coalesced frames | `tests/framing.rs` in Provider crate |
| `vsock_cid_mismatch_closes_without_processing` | `VsockEndpointPolicy::ExpectedCid` enforcement | `effect_port_mock.rs` (OpaqueEndpointId mismatch); raw CID test stays in core adapter |
| `vsock_frame_too_large_rejects_before_allocating` | Bounded per-frame allocation | `tests/framing.rs` in Provider crate |
| `vsock_clean_eof_versus_reset_are_distinct` | EOF/reset distinction for reconnect decision | `tests/framing.rs` in Provider crate |

**Excluded ADR 0045 assumptions**:
- `NativeVsockListener::bind_any` uses a fixed well-known port derived from
  the ADR 0045 endpoint-purpose enum; v3 port is allocated by the Zone
  allocator. This is in the core effect adapter; the Provider never sees ports.
- `VsockEndpointPolicy::ExpectedCid { cid: u32 }` accepts a raw `u32`; v3
  uses `OpaqueEndpointId` in the core adapter. Provider never receives raw CID.
- `CloudHypervisorVsockTransport` (CONNECT-proxy through CH base UDS) is
  the ADR 0045 guest-control bootstrap path; excluded entirely.

---

### `d2b-session-unix/src/adapter.rs` — `OwnedTransport` contract

The `OwnedTransport` trait and `TransportPacket`/`TransportDescriptor` from
main are the binding interface that `FramedVsockTransport` implements. The
relevant contract is:

```text
trait OwnedTransport: Send + 'static {
    fn descriptor(&self) -> TransportDescriptor;
    async fn send(&mut self, packet: TransportPacket) -> Result<(), TransportError>;
    async fn receive(&mut self) -> Result<TransportPacket, TransportError>;
    async fn close(self: Box<Self>);
}
```

`TransportDescriptor` for vsock:
- `class = TransportClass::Vsock`
- `locality = Locality::NonLocal`
- `atomic_transfer = false`  (no attachment packet atomicity)
- `attachment_support = false`

Any `TransportPacket` carrying non-empty `attachments` over a vsock transport
is a protocol error. The `SessionEngine` checks `descriptor().attachment_support`
before dispatching and returns `attachment-not-permitted-over-vsock` without
contacting the remote end.

**V3 destination**: Imported from `packages/d2b-bus/src/session/` (ADR046-bus-001).
The `OwnedTransport` trait is not re-implemented in the transport Provider; it
is consumed from `d2b-session`.

---

## Baseline v3 current code

| Symbol | File | Evidence class | Notes |
| --- | --- | --- | --- |
| `connect_guest_control_vsock` | `packages/d2bd/src/guest_control_vsock.rs` | `implemented-and-reachable` | CONNECT-proxy path via CH base UDS; peer-credential/path validation; used at daemon startup for guest-control only; not a generic ZoneLink transport |
| `GuestControlConnectedStream` | `packages/d2bd/src/guest_control_vsock.rs` | `implemented-and-reachable` | Wraps post-CONNECT socket; handoff to ttrpc guest-control only |
| `generate_vsock_relay_argv` | `packages/d2b-host/src/vsock_relay_argv.rs` | `implemented-and-reachable` | socat relay for OTLP (ports 14317/14319); uses hard-coded CID 2 for guest egress; not a ComponentSession transport |
| `GUEST_CONTROL_CONNECT_PORT = 14318` | `packages/d2bd/src/guest_control_vsock.rs` | `implemented-and-reachable` | Reserved for guestd ttrpc; must not be reused by transport-vsock ZoneLink |
| `VsockRelayArgvInput` | `packages/d2b-host/src/vsock_relay_argv.rs` | `implemented-and-reachable` | OTLP relay shape; socat path; superseded by Provider OTLP/vsock in observability work |
| `SocatEndpoint::VsockConnect { cid, port }` | `packages/d2b-host/src/vsock_relay_argv.rs` | `implemented-and-reachable` | Raw CID in socat argv; pattern excluded from v3 ZoneLink `spec.provider.settings` |
| vsock addr `vsock://-1:14318` | `packages/d2b-host/tests/guest_vsock_ttrpc_compile.rs` | `test-only-or-preview` | Guest-side ttrpc compile proof; cfg-gated; not a production path |
| `vsock.rs` vsock transports | `packages/d2b-session-unix/src/vsock.rs` (v3 baseline) | `ADR-only` | Stub file; no implementation in v3 baseline; marked as Provider-specific |

---


## Provider identity and scope

**Provider crate**: `d2b-provider-transport-vsock`

**providerRef**: `Provider/transport-vsock`

**Role**: carriage-acquisition service Provider.

`Provider/transport-vsock` acquires AF_VSOCK byte channels on behalf of the
core Zone-link/delegation controller. It implements no ResourceType and owns
no reconcile loop, no status writes, no route publication, no finalizer, and
no session-generation management. Those responsibilities belong exclusively to
the core ZoneLink/delegation controller (`d2b-core-controller` crate). The
Provider runs one `service` Process and responds to three typed service method
invocations from core: `OpenTransport`, `CloseTransport`, and
`ObserveTransport`.

**Scope** (what this Provider does):
- Implements `d2b.transport.vsock.v3.VsockTransportService`.
- On `OpenTransport`: calls the injected `VsockEffectPort` to acquire a vsock
  connection using opaque endpoint and binding IDs; returns an
  `OwnedTransport` bridged as a named stream on the Provider's ComponentSession
  with d2b-bus.
- On `CloseTransport`: releases the named stream and any internally-held
  vsock socket.
- On `ObserveTransport`: streams `TransportEvent` records (acquired,
  bytes-transferred, error, released) to the core caller.
- Uses 2-byte big-endian length-prefixed framing for every vsock write/read.
- Never transfers file descriptors (structural: `attachment_support = false`
  on the vsock `TransportDescriptor`).

**Not in scope for this Provider** — owned by other components:
- ZoneLink reconcile loop, status condition writes, finalizers, route
  advertisement → core ZoneLink/delegation controller.
- Noise handshake (KK, IKpsk2), ComponentSession lifecycle, credit/flow
  control, stream multiplexing, session-generation management → d2b-bus /
  ComponentSession.
- Port allocation and the port registry → core Zone allocator state.
- Reconnect policy and generation increment → core ZoneLink/delegation
  controller; Provider only acquires the underlying vsock socket on demand.
- Guest-control ttrpc channel (port 14318) → ADR 0028 guest-control path.
- OTLP vsock relay (ports 14317/14319) → `observability-otel` Provider.
- Unix-socket ZoneLinks → `transport-unix` Provider.
- Azure Relay ZoneLinks → `transport-azure-relay` Provider.

### Currency and upgrade (D091)

The core ZoneLink controller, not the transport-vsock service, implements
`assess_update`, `plan_upgrade`, and `execute_upgrade`. A Provider generation or
signed artifact generation/digest change updates universal `status.update` with
`state: UpdateAvailable` or `state: UpgradeRequired`, `reasons` including
`ProviderGenerationChanged` or `ArtifactChanged`, observed/target generation or
digest IDs, `disruption: Reload` or `disruption: Restart`, `preserveState:
true`, bounded `owned`/`dependencies`, and `lastAssessedAt`. Disruptive changes
MUST return `UpgradeRequired` rather than applying in place; non-disruptive
changes reconcile normally. Upgrade recycles the transport service realization;
open byte-stream handles are re-established by core reconnect. ZoneLink session
state remains owned by the core ZoneLink controller. `status.update` MUST NOT
contain secrets.

### Expedited reconcile on mutation (D090)

For `Create`, `UpdateSpec`, and `Delete` with `waitForReconcile`, core MUST
perform no `OpenTransport`/`CloseTransport`, finalizer change, or status
mutation until it supplies
`CommittedRevisionProof {resourceUid,generation,revision,operationId}`. Abort
before that proof has no effect. After durable commit, the commit is never
rolled back if the reconcile pass times out. The response returns the committed
object, post-pass projected layered status, disposition
(`Converged|Progressing|Blocked|UpgradeRequired|Failed`), and
`statusPersistence: pending|committed`. Effect idempotency keys derive from
`(UID,generation,revision,operationId)` and use the same per-resource
single-flight priority lane.

**D089 desired-spec shape.** This transport Provider owns no ResourceType; core
reconciles the `ZoneLink` base `spec.*` fields, including `spec.providerRef`.
Vsock-specific desired input is carried only by the canonical
`ZoneLink.spec.provider = { schemaId, schemaVersion, settings }` envelope, whose
`settings` object mirrors `status.provider.details`, is registered/signed in the
Provider manifest, deny-unknown, bounded, versioned/digested, validated against
`spec.providerRef` at Nix build and API admission, and cannot shadow base
fields. Shared fields are promoted to the ResourceType base. The Provider
implements the exact base spec/status schema version/fingerprint, accepts the
canonical minimal base Spec, passes base conformance, and rejects an
unsupported optional base capability only through its signed capability matrix plus
provider-neutral `unsupported-capability`.
`spec.provider` aligns with `status.provider`. The `Provider` resource itself
keeps the D075 `spec.{artifactId, config}` exception.

---

## Crate/package boundary

```text
packages/d2b-provider-transport-vsock/
  src/
    lib.rs          — crate root; Provider identity constant
    service.rs      — VsockTransportService implementation; dispatches OpenTransport /
                      CloseTransport / ObserveTransport to effect port and bridge tasks
    effect_port.rs  — VsockEffectPort async trait + OpaqueEndpointId / OpaqueBindingId
    bridge.rs       — named-stream ↔ opaque AsyncRead+AsyncWrite byte pump task
    framing.rs      — 2-byte big-endian length-prefix encode / decode (no raw sockets)
    limits.rs       — per-connection and per-session constants
    errors.rs       — VsockEffectError / FramingError / ServiceError typed hierarchy
  tests/
    framing.rs           — 2-byte framing, partial reads, frame-size bounds
    effect_port_mock.rs  — FakeVsockEffectPort; opaque-ID mismatch / timeout injection
    open_close.rs        — OpenTransport / CloseTransport service round-trip (fake port)
    observe.rs           — ObserveTransport event stream (fake port)
    redaction.rs         — no CID / port / path in Debug / log / audit output
    schema.rs            — `spec.provider.settings` JSON Schema round-trip and rejection
    state_volume.rs      — state Volume spec shape, User/<name> layout principal, no ComponentPrincipal
  integration/
    host_guest.rs    — real vsock socketpair via injected effect; OpenTransport + byte round-trip
    no_fd_transfer.rs — structural attachment rejection over vsock transport
  README.md
```

`src/`, `tests/`, `integration/`, and `README.md` are required by workspace
policy. A workspace policy test rejects any Provider crate missing these paths.

The crate depends only on:
- `d2b-session` (`OwnedTransport` trait, `TransportDescriptor`, `TransportPacket`)
- `d2b-contracts` (v3 zone-session constants, `ServicePackage`)
- `d2b-provider` (`ProviderRegistry`, `AuthenticatedProviderRpc`)
- `tracing` (structured spans; no log macros that could emit raw IDs)
- `opentelemetry` (metric emission only; no OTEL SDK link)
- `serde` / `serde_json` (`spec.provider.settings` schema validation)
- `tokio` (async runtime; no `tokio-vsock` here)
- Test-only: `d2b-provider-toolkit` (conformance kit)

**`tokio-vsock` is NOT a dependency of this crate.** AF_VSOCK socket creation,
`connect`, `bind`, and `accept` calls live exclusively in the core effect
adapter (`LiveVsockEffectPort` in `d2b-core-controller`). The Provider crate
receives opaque `AsyncRead+AsyncWrite` streams from `VsockEffectPort::open`
and never calls AF_VSOCK syscalls directly.

No dependency on `d2bd`, `d2b-priv-broker`, `d2b-realm-*`, Nix emitter
internals, or any other Provider implementation crate is permitted.

---

## Components and binaries

### Component: service

| Field | Value |
| --- | --- |
| Component type | `service` |
| Binary | `d2b-transport-vsock` |
| ResourceTypes owned | none |
| Exported service | `d2b.transport.vsock.v3.VsockTransportService` |
| Cardinality | One per Zone runtime where `Provider/transport-vsock` is installed |
| Default domain | `system` |
| Host or Guest | Resolved from `Provider.spec.config.executionRef`; one service Process per installed Provider instance, not per ZoneLink |
| Placement template | `Process/transport-vsock-service` owned by `Provider/transport-vsock` |

There is one component. There are no additional binaries, listener daemons, or
background workers beyond this single service process.

### Provider root config schema

`Provider/transport-vsock` exports one required config field:

| Field | Type | Default | Semantics |
| --- | --- | --- | --- |
| `executionRef` | `ResourceRef` | required | `Host/<name>` or `Guest/<name>` in the same Zone; determines where the single service Process runs |

The Provider's controller creates exactly one `Process/transport-vsock-service`
resource using this `executionRef`. There is one service Process per installed
Provider instance; all ZoneLink `OpenTransport`/`CloseTransport`/`ObserveTransport`
calls for the Zone are handled by that single process.

### Process resource template

```yaml
apiVersion: resources.d2bus.org/v3
type: Process
metadata:
  name: transport-vsock-service
  ownerRef: Provider/transport-vsock
spec:
  providerRef:  Provider/system-minijail
  executionRef: <Provider.spec.config.executionRef>   # Host/<name> or Guest/<name>
  domain:       system
  processClass: service
  template:     transport-vsock-service
  sandbox:
    namespaceClasses:  []
    capabilityClasses: []
    seccompClass:      strict
    startRoot:         false
    noNewPrivileges:   true
    environmentClass:  minimal
    readOnlyRoot:      true
  budget:
    cpu:
      request: "100m"
      limit:   "500m"
    memory:
      request: "16Mi"
      limit:   "32Mi"
    pids:
      limit: 64
    fds:
      limit: 256
  mounts:
    - volumeRef:  Volume/transport-vsock--service--empty-state--<executionRef-short>
      view:       service
      mountPath:  /state
      access:     read-only
      required:   true
  readiness:
    class:             provider-defined
    initialDelay:      "0s"
    timeout:           "30s"
    failureThreshold:  3
    successThreshold:  1
  restartPolicy:
    class:             on-failure
    backoffBase:       "1s"
    backoffMax:        "60s"
    backoffMultiplier: 2.0
    maxRestarts:       10
    resetAfter:        "300s"
```

The stable vsock service binding is a standard `Endpoint` resource, not an
inline `ProcessSpec` field:

```yaml
apiVersion: resources.d2bus.org/v3
type: Endpoint
metadata:
  name: transport-vsock-service
  ownerRef: Process/transport-vsock-service
spec:
  providerRef: Provider/transport-vsock
  producerRef: Process/transport-vsock-service
  endpointClass: transport
  transport: vsock
  purpose: transport-vsock.d2bus.org/service
  serviceFingerprint: transport-vsock.d2bus.org/service.v1
  locality: cross-domain
  visibility: zone-private
  attachmentPolicy: none
  consumerPolicy: [core-controller.d2bus.org/zonelink]
  lifecyclePolicy: producer-owned
status:
  phase: Ready
  readiness: Ready
  observedProducerGeneration: 1
  observedResourceGeneration: 1
  endpointGeneration: 1
  connectionAvailability: Available
  leaseAvailability: Available
  conditions: []
```

### Endpoint resources (D092)

Stable allocator/listener bindings with visible lifecycle are standard
`Endpoint` resources. Consumers refer to `Endpoint/<name>` and receive no raw
CID, port, socket address, FD, or credential from `Endpoint.spec` or
`Endpoint.status`; resolution is through the authorized EffectPort/LaunchTicket
path, and unauthorized callers fail with `endpoint-resolve-denied`. A producer
Process restart bumps `endpointGeneration`, which consumers observe as
`dependency-changed`. ZoneLink session state remains owned by the core ZoneLink
controller.

### Retained opaque handles (D092)

`OpaqueEndpointId`, `OpaqueBindingId`, per-session `OpenTransport` named
streams, `OwnedTransport` byte-stream handles, transport connection handles,
pidfds, FD indexes, and `operationId` values remain controller-internal or
high-churn opaque handles. They are not stable resources and are not promoted to
`Endpoint`.

Prohibited fields (never present in any Process or EphemeralProcess spec):
`binary`, `allowedSyscalls`, `allowedSocketFamilies`, `resourceRefs`,
`metricsPrefix`. These are resolved internally from `seccompClass`,
`processClass`, and the template descriptor.

---

## Service API

The Provider implements the following service on its ComponentSession with
d2b-bus. All three methods are invoked exclusively by the core
Zone-link/delegation controller. No other caller is authorized.

### `OpenTransport`

```text
rpc OpenTransport(OpenTransportRequest) -> OpenTransportResponse
```

**Request fields**:

| Field | Type | Semantics |
| --- | --- | --- |
| `endpoint_id` | `OpaqueEndpointId` | Core-allocated internal endpoint-resolution token (encodes peer CID; opaque to Provider; not an `Endpoint` resource) |
| `binding_id` | `OpaqueBindingId` | Core-allocated binding identity (encodes allocated port; opaque to Provider) |
| `role` | `TransportRole` | `Initiator` (parent-to-child connect) or `Responder` (child-side accept) |
| `deadline_ms` | `u32` | Connect/accept deadline in ms from call arrival; range 1 000–60 000 |

**Response fields**:

| Field | Type | Semantics |
| --- | --- | --- |
| `transport_handle` | `TransportHandle` | Opaque handle identifying the open transport; used in `CloseTransport` |
| `stream_id` | `NamedStreamId` | Named stream on the Provider's ComponentSession that d2b-bus reads as its raw byte channel |
| `descriptor` | `TransportDescriptor` | `class=Vsock`, `locality=NonLocal`, `atomic_transfer=false`, `attachment_support=false` |

**Behavior**:
1. Validates that `endpoint_id` and `binding_id` are well-formed opaque IDs
   (`[a-z][a-z0-9-]{0,63}`). Rejects with `invalid-endpoint-id` or
   `invalid-binding-id` if malformed.
2. Calls `VsockEffectPort::open(endpoint_id, binding_id, role, deadline)`.
3. On success, opens a named stream on the inbound ComponentSession and
   spawns a bridge task pumping bytes between the vsock socket and the
   named stream.
4. Returns `transport_handle` and `stream_id` to the caller.
5. The Provider does not inspect the bytes flowing through the bridge. Noise
   handshake and ComponentSession framing are core/d2b-bus concerns.

**Error cases**: `deadline-exceeded`, `connect-refused`, `cid-unreachable`,
`port-conflict`, `invalid-endpoint-id`, `invalid-binding-id`,
`provider-overloaded` (max concurrent transports exceeded).

### `CloseTransport`

```text
rpc CloseTransport(CloseTransportRequest) -> CloseTransportResponse
```

**Request fields**: `transport_handle: TransportHandle`.

**Behavior**:
1. Looks up the bridge task for `transport_handle`. Returns
   `unknown-transport-handle` if not found.
2. Signals the bridge task to stop. Waits for graceful shutdown up to
   `CLOSE_GRACE_MS = 500` ms, then forces close.
3. Closes the named stream on the ComponentSession.
4. Releases the vsock socket via `VsockEffectPort::close`.

### `ObserveTransport`

```text
rpc ObserveTransport(ObserveTransportRequest) -> stream TransportEvent
```

**Request fields**: `transport_handle: TransportHandle`, `include_bytes: bool`.

**Event stream**:

| Event | Emitted when |
| --- | --- |
| `acquired` | vsock socket obtained (after `OpenTransport` succeeds) |
| `bytes_transferred { rx_bytes, tx_bytes }` | Emitted at most once per second if `include_bytes = true` |
| `error { kind, recoverable }` | vsock read/write error on bridge task |
| `released` | vsock socket closed (after `CloseTransport` or bridge task exit) |

The observer stream is best-effort. The Provider does not buffer events for
slow observers; overflowing observers receive `observer-buffer-full` and are
dropped.

---

## VsockEffectPort — injected interface

`VsockEffectPort` is an async Rust trait injected at Provider startup. It
abstracts all AF_VSOCK syscall access. The Provider never calls
`socket(AF_VSOCK, …)`, `connect`, or `bind` directly; it delegates every
vsock operation to this port.

```rust
/// Injected by the Zone runtime. Provider never calls AF_VSOCK syscalls directly.
#[async_trait]
pub trait VsockEffectPort: Send + Sync + 'static {
    /// Acquire a vsock connection for the given opaque IDs.
    ///
    /// For `role = Initiator`: performs a non-blocking connect with the given deadline.
    /// For `role = Responder`: accepts from a pre-bound listener with the given deadline.
    ///
    /// Returns a FramedVsockStream. The raw CID and port are never exposed to the caller.
    async fn open(
        &self,
        endpoint_id: &OpaqueEndpointId,
        binding_id: &OpaqueBindingId,
        role: TransportRole,
        deadline: Instant,
    ) -> Result<FramedVsockStream, VsockEffectError>;

    /// Release a previously opened vsock stream.
    async fn close(&self, stream: FramedVsockStream) -> Result<(), VsockEffectError>;
}
```

`OpaqueEndpointId` and `OpaqueBindingId` are newtype wrappers over bounded
strings with no public CID/port accessors. The Zone runtime provides a
`LiveVsockEffectPort` implementation that resolves opaque IDs against core
allocator state and opens the actual AF_VSOCK sockets. Tests inject a
`MockVsockEffectPort`.

The Provider never receives, stores, logs, or emits raw CID (`u32`) or port
(`u32`) values. Any path that would write a raw CID or port to a log,
structured event, audit record, metric label, or error message is a security
defect and must be caught by the `redaction.rs` test.

---

## `spec.provider.settings` schema

The `settings` object inside `ZoneLink.spec.provider` carries Provider-specific
configuration when `spec.providerRef = Provider/transport-vsock`. Core
resolves the opaque endpoint and binding IDs from these settings; the Provider
never receives the raw resolution.

**JSON Schema ID**: `docs/reference/schemas/v3/providers/transport-vsock.transport-binding.json`

| Field | Type | Default | Semantics |
| --- | --- | --- | --- |
| `guestRef` | `ResourceRef` | required | `Guest/<name>` in the same Zone; core resolves to vsock CID internally |
| `portClass` | `string` (enum) | `"d2b-link"` | Port class; core allocates a port from the class range; `"d2b-link"` → range `14420–14499` |
| `connectTimeoutSeconds` | `integer` [1, 60] | `30` | Passed as `deadline_ms` in `OpenTransport` |

**Forbidden fields** (rejected by schema `additionalProperties: false`):
`cid`, `port`, `socketPath`, `token`, `password`, and any field whose value
is or contains a raw socket address.

**Reserved ports** — never allocated by `portClass: "d2b-link"`:
- `14317`: OTLP gRPC relay (observability-otel)
- `14318`: guestd ttrpc (guest-control, ADR 0028)
- `14319`: OTLP HTTP relay (observability-otel)

These three ports are enforced as exclusions in the core allocator; the
Provider has no knowledge of them.

---

## ProviderStateSet

A **ProviderStateSet** is the optional, query-time grouping — the set of the
*declared* Volume resources in a Zone whose `metadata.ownerRef` resolves to
`Provider/<name>`. It is not a ResourceType and is empty for a Provider that
declares no state Volume.

`Provider/transport-vsock` declares **no** Provider state Volume; its
`ProviderStateSet` is empty. Its bounded non-secret operational state — service
readiness, transport-open/close reconcile stage, bounded connection/port
observation counters, and closed-enum error detail — lives in the owning
resource's `status` subresource and the core Operation ledger (D087). Port
allocation and the port registry are core allocator state; all ZoneLink session
state is owned by the core ZoneLink controller under the `ZoneLink` resource.
The service holds only opaque byte-stream handles in its own process memory
(`OwnedTransport`, per D081), which are never persisted.

Because this Provider's operational state is fully derivable from spec,
`status`, the core Operation ledger, and its live process memory, it fails the
storage-need test and declares no state namespace, no state Volume, no
state-view mount, and no dedicated state-layout `User/<name>` principal. There
is no empty identity-only Volume.

---

## RBAC and authorization

### RBAC grants for the core ZoneLink controller (caller)

The core ZoneLink/delegation controller is the only principal authorized to
invoke the Provider's service methods. No other principal (other Providers,
operators, end-users) may invoke `OpenTransport`, `CloseTransport`, or
`ObserveTransport`.

```yaml
RBACPolicy:
  subject: Principal/core-zone-link-controller
  zone:    <same Zone as Provider/transport-vsock>
  rules:
    - resource: d2b.transport.vsock.v3.VsockTransportService
      verbs: [invoke]
    - resource: ComponentSession/transport-vsock-service
      verbs: [attach]
```

### RBAC grants NOT required by this Provider

`Provider/transport-vsock` requires NO verbs on `ZoneLink`, `Guest`, `Zone`,
`Route`, `Credential`, `Certificate`, or any other ResourceType. All
ZoneLink resource access is performed exclusively by the core controller. The
Provider service holds no permissions to read, watch, update, or delete any
resource.

---

## Security invariants

| ID | Invariant | Enforcement |
| --- | --- | --- |
| INV-VSOCK-001 | Raw CID (`u32`) never appears in any resource spec, status field, audit event, metric label, log line, or error message surface | `VsockEffectPort` opacity; `OpaqueEndpointId` newtype has no public CID accessor; `redaction.rs` hermetic test |
| INV-VSOCK-002 | Raw port (`u32`) never appears in any resource spec, status field, audit event, metric label, log line, or error message surface | `OpaqueBindingId` newtype has no public port accessor; `redaction.rs` hermetic test |
| INV-VSOCK-003 | No file descriptor is transferred over the vsock byte channel | `TransportDescriptor.attachment_support = false`; d2b-bus serialization boundary checks `descriptor()` before dispatch; `no_fd_transfer.rs` integration test |
| INV-VSOCK-004 | Provider never calls `socket(AF_VSOCK, …)`, `connect`, or `bind` directly | `VsockEffectPort` is the only AF_VSOCK call surface; `cfg(target_os = "linux")` feature gate; all syscall paths in `LiveVsockEffectPort` only; seccomp strict profile on Provider process |
| INV-VSOCK-005 | Provider never accesses or modifies ZoneLink, Guest, Route, or any other ResourceType | RBAC grants contain no resource-type verbs; Provider holds no resource API client; conformance test asserts no resource calls |
| INV-VSOCK-006 | `spec.provider.settings` schema rejects any field carrying a raw socket address, port number, CID, path, or credential | JSON Schema `additionalProperties: false`; build-time emitter secret-key scanner |
| INV-VSOCK-007 | Provider process: no new privileges, strict seccomp, no network namespace join, read-only root filesystem | `sandbox.seccompClass: strict`, `sandbox.noNewPrivileges: true`, `sandbox.readOnlyRoot: true` in Process template; `Provider/system-minijail` profile |
| INV-VSOCK-008 | Port range `14420–14499` is reserved for `d2b-link` ZoneLink vsock sessions; ports 14317, 14318, 14319 are never allocated by `portClass: "d2b-link"` | Core allocator exclusion list; enforced in core, not in Provider |
| INV-VSOCK-009 | Service component receives only a dirfd into its local `service` view of its own state Volume; no raw filesystem path, no parent-directory access, no cross-component Volume mount | volume-local Provider validates `sensitivityClass: private` and `mountPath` scope before handing dirfd to process; domain isolation enforced at mount time |
| INV-VSOCK-010 | State Volume layout principal `User/d2b-transport-vsock` is a Nix-preprovisioned `User/<name>` ResourceRef; no `ComponentPrincipal` ResourceRef used | Volume `layout[].ownerRef`/`groupRef` fields hold only `User/<name>` refs; validated at Volume admission; Nix module declares the system user |

---

## Lifecycle

Per D088, `Provider/transport-vsock` does not write resource status directly:
core owns the universal `ResourceStatus` base and the ResourceType-common
`status.resource` projection for Provider and ZoneLink resources. Cross-provider
transport observations returned to core are promoted to `ZoneLink.status.resource`;
any bounded, non-secret vsock-only observation that core persists for this
Provider uses `ZoneLink.status.provider` with `providerRef:
Provider/transport-vsock`, qualified `schemaId:
transport-vsock.d2bus.org/ZoneLink/status`, `schemaVersion` (semver MAJOR.MINOR),
`observedProviderGeneration`, and a strict unknown-field-denied, ≤32 KiB,
redacted `details` schema registered and signed in the Provider manifest. Core
writes all present layers atomically and never copies shared
`status.resource` fields into `status.provider`.

### Provider installation

1. Nix module emits `Provider/transport-vsock` resource into the Zone resource
   store.
2. Core ProviderDeployment creates
   `Volume/transport-vsock--service--empty-state--<executionRef-short>` with
   `ownerRef: Provider/transport-vsock`; waits for Volume `Ready` (reconciled
   by `Provider/volume-local`). The transport-vsock Provider controller does
   not participate in Volume creation.
3. Provider controller creates `Process/transport-vsock-service` (with `mounts`
   referencing the pre-created state Volume); waits for Process `Ready`.
4. Service process connects to d2b-bus; receives a dirfd into its `/state` view
   from the volume-local Provider; registers
   `d2b.transport.vsock.v3.VsockTransportService` on the Zone service registry.
5. Service emits readiness; core ProviderDeployment observes it and sets
   `Provider/transport-vsock` status to `Ready`.

### Transport open (per ZoneLink session request from core)

1. Core ZoneLink/delegation controller calls `OpenTransport(endpoint_id,
   binding_id, role, deadline_ms)` on the Provider's ComponentSession.
2. Provider validates opaque IDs.
3. Provider calls `VsockEffectPort::open(...)`.
4. Zone runtime (`LiveVsockEffectPort`) resolves `endpoint_id` → CID and
   `binding_id` → port from core allocator state; opens or accepts the
   AF_VSOCK socket.
5. Provider opens a named stream on its ComponentSession and spawns a bridge
   task.
6. Provider returns `transport_handle` + `stream_id` to core.
7. Core hands `stream_id` to d2b-bus as the `OwnedTransport` for the ZoneLink.
8. d2b-bus runs Noise KK or IKpsk2 handshake on top of the raw bytes.

### Transport close

1. Core calls `CloseTransport(transport_handle)`.
2. Provider signals bridge task to stop; waits up to `CLOSE_GRACE_MS`.
3. Provider closes named stream and vsock socket.
4. Provider emits `transport.release` audit event.

### Provider removal

1. Core calls `CloseTransport` for every open transport handle.
2. Zone runtime stops `Process/transport-vsock-service`; Process finalizer
   completes. The transport-vsock Provider controller does not delete the
   Volume.
3. Core ProviderDeployment deletes
   `Volume/transport-vsock--service--empty-state--*` after the Process
   finalizer; waits for Volume `Deleted` (identity marker removed by broker).
4. Zone resource store marks `Provider/transport-vsock` `Deleted`.
   No other finalizer work is required: the ProviderStateSet contained exactly
   the one state Volume, which is now gone.

---

## Errors

Provider-emitted errors. Core-owned errors (hop-limit-exceeded,
reconnect-exhausted, route-not-found, bootstrap PSK invalid) are NOT
produced by this Provider.

| Error code | Trigger | Retryable |
| --- | --- | --- |
| `deadline-exceeded` | `VsockEffectPort::open` timed out | Yes (core decides retry) |
| `connect-refused` | Guest VM not ready or vsock port not listening | Yes |
| `cid-unreachable` | Guest VM not booted or vsock device absent | Yes |
| `port-conflict` | Binding ID resolves to a port already in use | No; core must allocate new port |
| `invalid-endpoint-id` | `endpoint_id` failed format validation | No |
| `invalid-binding-id` | `binding_id` failed format validation | No |
| `provider-overloaded` | Max concurrent open transports exceeded | Yes (backoff) |
| `unknown-transport-handle` | `CloseTransport` or `ObserveTransport` on unknown handle | No |
| `bridge-task-panicked` | Internal bridge task exited unexpectedly | No; core must reopen |
| `framing-error` | 2-byte length prefix violated protocol | No |

---

## Audit events

The Provider emits two audit event kinds. Session, handshake, route, and
reconnect audit events are NOT emitted by this Provider; they belong to
d2b-bus / core.

| Event kind | Fields | Emitted on |
| --- | --- | --- |
| `transport.acquire` | `provider=transport-vsock`, `handle=<opaque>`, `role`, `result` | Successful `OpenTransport` (after vsock acquired) |
| `transport.release` | `provider=transport-vsock`, `handle=<opaque>`, `reason` | `CloseTransport` or bridge task exit |

**Prohibited in all audit fields**: raw CID, raw port, socket path, named
stream bytes, Noise handshake material, session credentials.

---

## OTEL metrics

Provider-scoped metrics only. Session, handshake, reconnect, and route
metrics are NOT the Provider's responsibility.

| Metric name | Kind | Labels | Semantics |
| --- | --- | --- | --- |
| `d2b.transport.vsock.open.duration_ms` | Histogram | `role`, `result` | Time from `OpenTransport` receipt to vsock acquired or error |
| `d2b.transport.vsock.open.total` | Counter | `role`, `result` | `OpenTransport` calls by outcome |
| `d2b.transport.vsock.active` | Gauge | — | Currently open transport handles |
| `d2b.transport.vsock.bytes_rx` | Counter | — | Bytes received from vsock side of bridge |
| `d2b.transport.vsock.bytes_tx` | Counter | — | Bytes sent to vsock side of bridge |
| `d2b.transport.vsock.bridge_errors` | Counter | `error_kind` | Bridge task errors (framing, reset, timeout) |

**Prohibited labels**: raw CID, raw port, socket path, `endpoint_id` or
`binding_id` literal values (only opaque short codes), and any user or VM
identifier beyond the closed enum of label values.

---

## Performance gates

| Gate | Threshold | Measurement |
| --- | --- | --- |
| `OpenTransport` service overhead | ≤ 2 ms p99 (excluding vsock connect time) | Hermetic test with mock VsockEffectPort |
| Bridge throughput | ≥ 512 MiB/s on loopback vsock socketpair | `integration/host_guest.rs` benchmark |
| `d2b.transport.vsock.open.duration_ms` p99 (connect) | ≤ 100 ms on KVM host with running Guest | Integration test gate |
| Max concurrent open transports | 128 per service process | `limits.rs` constant; `provider-overloaded` enforced |
| Bridge task memory | ≤ 256 KiB working set per active transport | Measured in integration test |

---

## Nix authoring

```nix
# Provider resource (authored by the Zone operator):
d2b.zones.k0.resources.transport-vsock = {
  type = "Provider";
  spec = {
    artifactId = "transport-vsock";          # resolves to d2b.artifacts entry
    config = {
      executionRef = "Host/host-system";     # required; Host/<name> or Guest/<name>
    };
  };
};
# The Provider's controller creates Process/transport-vsock-service automatically
# using spec.config.executionRef. The operator does NOT author that Process resource.

# Example ZoneLink using this Provider (authored by the Zone operator):
d2b.zones.k0.resources.link-to-k1 = {
  type = "ZoneLink";
  spec = {
    childZoneRef          = "Zone/k1";
    providerRef  = "Provider/transport-vsock";
    provider = {
      schemaId      = "transport-vsock.d2bus.org/ZoneLink/spec";
      schemaVersion = "1.0";
      settings = {
        guestRef              = "Guest/k1-vm";
        portClass             = "d2b-link";
        connectTimeoutSeconds = 30;
      };
    };
  };
};
```

`spec.config.executionRef` is validated at eval time against declared Zone
resources; referential existence is verified at bundle activation time. The
Nix module also validates `spec.provider.settings` against the
`transport-vsock.transport-binding.json` schema and rejects any prohibited
field (`cid`, `port`, `socketPath`, etc.). Validation is performed by the
`d2b.zones.<name>.resources` schema checker, not by the Provider's own Nix
expression.

---

## Current-code fit

| Aspect | Anchor | Evidence class | Retained / Delta / Replacement |
| --- | --- | --- | --- |
| 2-byte framing utilities | `d2b-session-unix/src/vsock.rs` (main `a1cc0b2d`) | `implemented-but-unwired` | Retained: `FramedVsockTransport` framing (length-prefix encode/decode, bounded allocation, EOF/reset classification) → `framing.rs` in Provider crate. `NativeVsockTransport` / `NativeVsockListener` (raw AF_VSOCK socket calls) → core `LiveVsockEffectPort` in `d2b-core-controller`; NOT in Provider crate |
| `OwnedTransport` trait | `d2b-session-unix/src/adapter.rs` (main `a1cc0b2d`) | `implemented-but-unwired` | Retained verbatim; destination `d2b-session` crate (ADR046-bus-001) |
| vsock error variants (12) | `d2b-session-unix/src/vsock.rs` (main `a1cc0b2d`) | `implemented-but-unwired` | Retained verbatim in `errors.rs` |
| CONNECT-proxy guest-control | `d2bd/src/guest_control_vsock.rs` | `implemented-and-reachable` | Replacement: superseded by `LiveVsockEffectPort` allocator path; guest-control port 14318 excluded from ZoneLink allocation |
| socat OTLP relay | `d2b-host/src/vsock_relay_argv.rs` | `implemented-and-reachable` | Replacement: superseded by `observability-otel` Provider native vsock relay |
| Raw CID in relay argv | `SocatEndpoint::VsockConnect { cid, port }` | `implemented-and-reachable` | Replacement: pattern excluded; all CID surfaces become opaque allocator IDs in v3 |
| vsock CID 2 hard-code | `vsock_relay_argv.rs` line ~30 | `implemented-and-reachable` | Replacement: eliminated; ZoneLink allocator resolves `guestRef` → CID |
| `vsock.rs` v3 stub | `d2b-session-unix/src/vsock.rs` (v3 baseline) | `ADR-only` | Delta: stub becomes `FramedVsockTransport` implementation; entry point is `VsockEffectPort` not raw syscall |
| Framing tests | `d2b-session-unix/tests/unix_session.rs` vsock subset | `implemented-but-unwired` | Retained in `tests/framing.rs` (Provider crate): `vsock_framing_handles_partial_and_coalesced_records`, `vsock_frame_too_large_rejects_before_allocating`, `vsock_clean_eof_versus_reset_are_distinct` |
| CID mismatch enforcement | `vsock_cid_mismatch_closes_without_processing` | `implemented-but-unwired` | Adapted in `tests/effect_port_mock.rs` as `OpaqueEndpointId` mismatch case; raw CID variant moves to core adapter tests |
| ZoneLink resource | `ADR-046-zone-routing.md` | `generated-or-eval-contract` | Retained: ZoneLink base spec shape and `spec.provider.settings`; core controller owns all status/routes/finalizer |
| ComponentSession / Noise | `ADR-046-componentsession-and-bus.md` | `generated-or-eval-contract` | Retained: owned by d2b-bus; Provider is opaque carriage only |

---

## Work items

| ID | Title | Phase | Priority | Depends on | Owner crate | Proof type | Evidence class | Description |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| ADR046-vsock-001 | Implement `VsockEffectPort` trait and `OpaqueEndpointId`/`OpaqueBindingId` newtypes | Phase 1 | P0 | ADR046-bus-001 (OwnedTransport in d2b-session) | `d2b-provider-transport-vsock` | hermetic unit + redaction test | `test-only-or-preview` | Define `VsockEffectPort` async trait and opaque ID newtypes in `effect_port.rs`; implement `FakeVsockEffectPort` for tests; `redaction.rs` asserts no raw `u32` in any `Debug`/`Display` output of opaque types; no real vsock socket opened |
| ADR046-vsock-002 | Implement framing utilities and bridge task in Provider crate | Phase 1 | P0 | ADR046-vsock-001 | `d2b-provider-transport-vsock` | hermetic framing tests | `implemented-but-unwired` | Copy `FramedVsockTransport` framing-only code (length-prefix encode/decode, bounded allocation, EOF/reset) from main `a1cc0b2d` → `framing.rs`; implement bridge task pumping bytes between an opaque `AsyncRead+AsyncWrite` stream from `VsockEffectPort::open` and the named ComponentSession stream; hermetic tests using `FakeVsockEffectPort` (no real socket) |
| ADR046-vsock-003 | Implement `VsockTransportService` (OpenTransport/CloseTransport/ObserveTransport) | Phase 1 | P0 | ADR046-vsock-002, ADR046-bus-001 | `d2b-provider-transport-vsock` | service round-trip test (mock) | `test-only-or-preview` | Implement all three service methods in `service.rs`; `open_close.rs` and `observe.rs` test full service API against `FakeVsockEffectPort`; conformance kit passes |
| ADR046-vsock-004 | Implement `LiveVsockEffectPort` in Zone runtime | Phase 2 | P0 | ADR046-vsock-001, ADR046-alloc-001 (Zone allocator) | `d2b-core-controller` | integration test | `ADR-only` | Zone runtime provides `LiveVsockEffectPort` backed by core allocator state; resolves `OpaqueEndpointId` → CID and `OpaqueBindingId` → port; opens AF_VSOCK socket; injects into Provider service at startup; no raw CID/port exposed to Provider |
| ADR046-vsock-005 | Core ProviderDeployment creates/deletes service component state Volume | Phase 1 | P0 | ADR046-vol-001 (volume-local Provider) | `d2b-provider-transport-vsock` | unit + integration test | `test-only-or-preview` | Core ProviderDeployment creates `Volume/transport-vsock--service--empty-state--*` before the component Process and deletes it after the Process finalizer; transport-vsock Provider controller does not own Volume, does not add Volume to exported ResourceTypes, and does not create its prerequisite; Volume spec: empty schema, `kind: state`, `persistenceClass: persistent`, `migrationPolicy: none`, `User/d2b-transport-vsock` owner, minimal nonzero `quota.maxBytes`/`quota.maxInodes` with `enforcement: hard`, `private` sensitivity, `broker-maintained` identity marker; `state_volume.rs` test verifies Volume spec fields against canonical schema; integration test verifies marker written at install and removed at Provider deletion; no operator-authored Volume; component receives dirfd view only |
| ADR046-vsock-006 | Integration test: real vsock socketpair + full ZoneLink open/close | Phase 2 | P1 | ADR046-vsock-003, ADR046-vsock-004 | `d2b-provider-transport-vsock` | integration test | `test-only-or-preview` | `integration/host_guest.rs`: real vsock socketpair (Linux); `OpenTransport` + byte round-trip + `CloseTransport`; validates bridge throughput ≥ 512 MiB/s; `no_fd_transfer.rs`: structural rejection of attachment packets over vsock transport |
| ADR046-vsock-007 | Delete legacy socat OTLP relay and CONNECT-proxy guest-control vsock | Phase 3 | P2 | ADR046-obs-001 (observability-otel Provider), ADR046-guest-001 (Guest resource lifecycle) | `d2b-host`, `d2bd` | deletion + parity test | `implemented-and-reachable` | Remove `vsock_relay_argv.rs` socat path after `observability-otel` Provider native vsock relay passes parity; remove `guest_control_vsock.rs` CONNECT-proxy after Guest resource lifecycle + guestd vsock bootstrap reach parity; no raw CID or socat vsock path remains |

---

## Tests required

| Test | Location | Kind | Gate |
| --- | --- | --- | --- |
| `vsock_framing_handles_partial_and_coalesced_records` | `tests/framing.rs` | unit | `cargo test` |
| `vsock_frame_too_large_rejects_before_allocating` | `tests/framing.rs` | unit | `cargo test` |
| `vsock_clean_eof_versus_reset_are_distinct` | `tests/framing.rs` | unit | `cargo test` |
| `open_transport_returns_handle_and_stream_id` | `tests/open_close.rs` | unit | `cargo test` |
| `close_transport_unknown_handle_returns_error` | `tests/open_close.rs` | unit | `cargo test` |
| `observe_transport_emits_acquired_then_released` | `tests/observe.rs` | unit | `cargo test` |
| `no_raw_cid_in_debug_or_display` | `tests/redaction.rs` | unit | `cargo test` |
| `no_raw_port_in_debug_or_display` | `tests/redaction.rs` | unit | `cargo test` |
| `transport_settings_schema_rejects_cid_field` | `tests/schema.rs` | unit | `cargo test` |
| `transport_settings_schema_rejects_port_field` | `tests/schema.rs` | unit | `cargo test` |
| `state_volume_spec_matches_canonical_schema` | `tests/state_volume.rs` | unit | `cargo test` |
| `state_volume_layout_uses_nix_user_ref_not_component_principal` | `tests/state_volume.rs` | unit | `cargo test` |
| `conformance_provider_registers_service` | `tests/` (conformance kit) | unit | `cargo test` |
| `host_guest_vsock_byte_roundtrip` | `integration/host_guest.rs` | integration | `make test-integration` |
| `no_fd_transfer_over_vsock` | `integration/no_fd_transfer.rs` | integration | `make test-integration` |

### Fast hermetic execution and test placement (D094)

Per D094 and `ADR-046-validation-and-delivery` §10.16, this Provider's `src/`
unit tests and `tests/*.rs` hermetic suite are fast, in-process, deterministic,
and parallel-safe: an individual normal test has p95 ≤50 ms with no wall-clock
sleep, and `cargo test -p d2b-provider-transport-vsock --lib --tests` completes in ≤2 s warm-cache
execution time (compilation excluded). They use a deterministic fake clock/RNG
and the toolkit fakes/FakeEffectPort only — no process spawn, container,
network, DBus, systemd, broker daemon, Nix eval/build, KVM, USB/GPU/TPM
hardware, or live cloud, and no filesystem tree beyond tiny temp fixtures. Any
scenario needing those lives only in `integration/`, which keeps a lane
timeout/budget, parallel isolation, and fake external services by default; such
a need is re-placed into `integration/`, never given a sleep, larger timeout,
or `#[ignore]`. Bounded crypto/property tests are the only classified
exception, each named with a capped case count and a declared higher per-test
budget.

---

## Removal criteria

`Provider/transport-vsock` (and its crate) may not be removed while:
1. Any `ZoneLink` resource with `spec.providerRef: Provider/transport-vsock`
   exists in any Zone.
2. Any `d2b-link` port-class vsock session is active on any Zone runtime.
3. The state Volume (`Volume/transport-vsock--service--empty-state--*`) has not
   been deleted and its identity marker has not been cleared.
4. The legacy `vsock_relay_argv.rs` socat path has not been fully replaced by
   `observability-otel` Provider native vsock transport (ADR046-vsock-007).
5. The `guest_control_vsock.rs` CONNECT-proxy path has not been fully replaced
   by the Guest resource lifecycle bootstrap (ADR046-vsock-007).

When all four conditions are clear, the removal commit must delete
`packages/d2b-provider-transport-vsock/` and the `transport-vsock` entry in
the Provider catalog in `ADR-046-provider-model-and-packaging.md`.

Per D094, each replaced current-code test is retired with an explicit
keep/adapt/move/delete disposition and a removal gate: the minimum reusable
semantic assertions migrate into this crate's hermetic `tests/`, and the old
duplicate tests, shell gates, fixtures, static artifacts, CI jobs, and manifest
entries are deleted once successor coverage and the removal proof pass —
updating `tests/layer1-jobs.json`, the closed gate manifests, the
flake/matrix/Nix-unit pins, the generated ledgers, and the CI workflow shards.
Old and new suites never run in parallel indefinitely.

---

## README.md requirements

`packages/d2b-provider-transport-vsock/README.md` must document:

- Provider identity: `Provider/transport-vsock`; carriage-acquisition service Provider.
- Role: `service` component; no ResourceType ownership; core ZoneLink/delegation
  controller is the sole ZoneLink reconciler and the only caller of this service.
- ProviderStateSet: empty — `Provider/transport-vsock` declares no Provider
  state Volume; its bounded non-secret operational state lives in `status`/the
  core Operation ledger (D087). All ZoneLink session state is owned by the core
  ZoneLink controller; the service holds only opaque byte-stream handles in
  process memory.
- `Provider.spec.config.executionRef`: required `Host/<name>` or `Guest/<name>`;
  one service Process per Provider instance, not per ZoneLink.
- `spec.provider.settings`: `guestRef` / `portClass` fields; what is forbidden.
- Port range `14420–14499` reservation; ports 14317/14318/14319 excluded.
- `VsockEffectPort` injection: Provider never calls AF_VSOCK syscalls directly;
  `tokio-vsock` is NOT a Provider crate dependency.
- Components: `d2b-transport-vsock` binary; one service process per Zone.
- Dependencies: `d2b-session`, `d2b-contracts`, `d2b-provider`, `tokio` (no `tokio-vsock`).
- RBAC: only core ZoneLink controller may invoke service methods.
- Build: `cargo build -p d2b-provider-transport-vsock`.
- Tests: `cargo test -p d2b-provider-transport-vsock`.
- Integration: `make test-integration`.
- Link to this spec: `docs/specs/providers/ADR-046-provider-transport-vsock.md`.
