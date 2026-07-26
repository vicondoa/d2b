# ADR 0046 Provider dossier: transport-vsock

| Field | Value |
| --- | --- |
| Spec ID | `ADR-046-provider-transport-vsock` |
| Parent | ADR 0046 |
| Status | Accepted |
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

### `d2b-session-unix/src/vsock.rs` - framed vsock transport

| Symbol | Selected behavior |
| --- | --- |
| `FramedVsockTransport` | Implements `OwnedTransport` over `AF_VSOCK`; 2-byte big-endian length-prefixed framing; async tokio-vsock send/receive; no SCM_RIGHTS (no attachment support); `TransportDescriptor::class=Vsock`, `locality=NonLocal`, `atomic_transfer=false`, `attachment_support=false` |
| `NativeVsockTransport` | Wraps a connected `tokio-vsock` stream as an `OwnedTransport`; per-frame bounded allocation; graceful vs. unclean EOF distinction |
| `NativeVsockListener` | Binds `AF_VSOCK VMADDR_PORT_ANY` then hands accepted streams to callers; per-accept CID verification against expected CID range |
| `VsockTransportError` (12 variants) | `BindFailed`, `ConnectFailed`, `CidMismatch`, `PortMismatch`, `FrameTooLarge`, `UnexpectedEof`, `WriteTimeout`, `ReadTimeout`, `ConnectionReset`, `ProtocolError`, `Backpressure`, `Shutdown` |
| `VsockEndpointPolicy` | `ExpectedCid { cid: u32 }` enforces that the connected peer CID equals the pre-negotiated value; connection from a mismatched CID is closed with `CidMismatch` without processing any bytes |

**V3 destination - split by ownership**:

- `FramedVsockTransport` framing utilities (2-byte length-prefix encode/decode,
  bounded frame allocation, EOF/reset classification) → `packages/d2b-provider-transport-vsock/src/framing.rs`.
  These contain no raw AF_VSOCK syscall calls; they operate on any `AsyncRead+AsyncWrite`.
- `NativeVsockTransport` / `NativeVsockListener` (raw `AF_VSOCK socket()`,
  `connect()`, `bind()`, `accept()`) → `packages/d2b-core-controller/` as the
  `LiveVsockEffectPort` implementation. These are NOT copied to the Provider crate;
  the Provider never calls AF_VSOCK syscalls.
- `VsockTransportError` variants → `packages/d2b-provider-transport-vsock/src/errors.rs`
  (framing/bridge errors only; raw-socket error variants stay in the core adapter).

**Tests** (`packages/d2b-session-unix/tests/unix_session.rs` - vsock subset):

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

### `d2b-session-unix/src/adapter.rs` - `OwnedTransport` contract

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
| `SocatEndpoint::VsockConnect { cid, port }` | `packages/d2b-host/src/vsock_relay_argv.rs` | `implemented-and-reachable` | Raw CID in socat argv; pattern excluded from v3 ZoneLink `spec.transportSettings` |
| vsock addr `vsock://-1:14318` | `packages/d2b-host/tests/guest_vsock_ttrpc_compile.rs` | `test-only-or-preview` | Guest-side ttrpc compile proof; cfg-gated; not a production path |
| `vsock.rs` vsock transports | `packages/d2b-session-unix/src/vsock.rs` (v3 baseline) | `ADR-only` | Stub file; no implementation in v3 baseline; marked as Provider-specific |

---


## Provider identity and scope

**Provider crate**: `d2b-provider-transport-vsock`

**providerRef**: `Provider/transport-vsock`

**Role**: carriage-acquisition service Provider.

`Provider/transport-vsock` acquires AF_VSOCK byte channels on behalf of the
child Zone's core Zone-link/delegation controller. The selected Provider and
ZoneLink are installed in that same child Zone. The Provider implements no
ResourceType and owns no reconcile loop, no status writes, no route
publication, no finalizer, and no session-generation management. Those
responsibilities belong exclusively to the child Zone's core
ZoneLink/delegation controller (`d2b-core-controller` crate). The Provider runs
one `service` Process and responds to three typed service method invocations
from child core: `OpenTransport`, `CloseTransport`, and `ObserveTransport`.
Compiler-only `parentZone` selects the allocator; the parent keeps only sealed
allocator/route state and has no reciprocal ZoneLink, Provider, Process,
Endpoint, or status handler.

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

**Not in scope for this Provider** - owned by other components:
- ZoneLink reconcile loop, status condition writes, finalizers, and local
  route/session state → child Zone's core ZoneLink/delegation controller.
- Noise handshake profile selection and sequencing (the core-owned
  enrollment-and-session state machine `Unenrolled -> IKpsk2 ->
  EnrollmentCommitted -> KK -> Ready`: one-time IKpsk2 bootstrap when
  `Unenrolled`, then enrolled KK), ComponentSession lifecycle, credit/flow
  control, stream multiplexing, session-generation management → d2b-bus /
  ComponentSession.
- Port allocation and the port registry → selected parent allocator's sealed
  route state; child code receives only the sealed local binding.
- Reconnect policy and generation increment → child Zone's core
  ZoneLink/delegation controller; Provider only acquires the underlying vsock
  socket on demand.
- Guest-control ttrpc channel (port 14318) → ADR 0028 guest-control path.
- OTLP vsock relay (ports 14317/14319) → `observability-otel` Provider.
- Unix-socket ZoneLinks → `transport-unix` Provider.
- Azure Relay ZoneLinks → `transport-azure-relay` Provider.

### Currency and upgrade (D091)

The child Zone's core ZoneLink controller, not the transport-vsock service or
the parent allocator, implements
`assess_update`, `plan_upgrade`, and `execute_upgrade`. A Provider generation or
signed artifact generation/digest change updates universal `status.update` with
`state: UpdateAvailable` or `state: UpgradeRequired`, `reasons` including
`ProviderGenerationChanged` or `ArtifactChanged`, observed/target generation or
digest IDs, `disruption: Reload` or `disruption: Restart`, `preserveState:
true`, bounded `owned`/`dependencies`, and `lastAssessedAt`. Disruptive changes
MUST return `UpgradeRequired` rather than applying in place; non-disruptive
changes reconcile normally. Upgrade recycles the transport service realization;
open byte-stream handles are re-established by child-core reconnect. ZoneLink
session state remains owned in the child store by the child Zone's core
ZoneLink controller. `status.update` MUST NOT contain secrets.

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

**D089 desired-spec shape.** This transport Provider owns no ResourceType; child
core reconciles the exact canonical ZoneLink base fields:
`childZoneName`, `transportProviderRef`, `transportSettings`,
`transportCredentials`, `disabled`, and `limits`. Vsock-specific desired input
is carried only by `spec.transportSettings`, whose deny-unknown schema is
registered and signed by the Provider selected through
`spec.transportProviderRef`. Vsock uses an empty
`spec.transportCredentials` list. No desired-state provider envelope or schema
metadata appears in ZoneLink spec.
`status.provider` remains the D088 implementation-observation layer and does not
mirror desired spec. The `Provider` resource itself keeps the D075
`spec.{artifactId, config}` exception.

---

## Crate/package boundary

```text
packages/d2b-provider-transport-vsock/
  src/
    lib.rs          - crate root; Provider identity constant
    service.rs      - VsockTransportService implementation; dispatches OpenTransport /
                      CloseTransport / ObserveTransport to effect port and bridge tasks
    effect_port.rs  - VsockEffectPort async trait + OpaqueEndpointId / OpaqueBindingId
    bridge.rs       - named-stream ↔ opaque AsyncRead+AsyncWrite byte pump task
    framing.rs      - 2-byte big-endian length-prefix encode / decode (no raw sockets)
    limits.rs       - per-connection and per-session constants
    errors.rs       - VsockEffectError / FramingError / ServiceError typed hierarchy
  tests/
    framing.rs           - 2-byte framing, partial reads, frame-size bounds
    effect_port_mock.rs  - FakeVsockEffectPort; opaque-ID mismatch / timeout injection
    open_close.rs        - OpenTransport / CloseTransport service round-trip (fake port)
    observe.rs           - ObserveTransport event stream (fake port)
    redaction.rs         - no CID / port / path in Debug / log / audit output
    schema.rs            - `spec.transportSettings` JSON Schema round-trip and rejection
    state_volume.rs      - state Volume spec shape, User/<name> layout principal, no ComponentPrincipal
  integration/
    host_guest.rs    - real vsock socketpair via injected effect; OpenTransport + byte round-trip
    no_fd_transfer.rs - structural attachment rejection over vsock transport
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
- `serde` / `serde_json` (`spec.transportSettings` schema validation)
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

The child Zone's core ProviderDeployment creates exactly one
`Process/transport-vsock-service` resource using this `executionRef`. There is
one service Process per installed child-local Provider instance; all
`OpenTransport`/`CloseTransport`/`ObserveTransport` calls for that child Zone's
ZoneLink are handled by that single process.

### Process resource template

```yaml
apiVersion: resources.d2bus.org/v3
type: Process
metadata:
  name: transport-vsock-service
  zone: k1
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
  zone: k1
  ownerRef: Process/transport-vsock-service
spec:
  providerRef: Provider/transport-vsock
  producerRef: Process/transport-vsock-service
  endpointClass: transport
  transport: vsock
  purpose: transport-vsock.d2bus.org/service
  serviceFingerprint: transport-vsock.d2bus.org/service.v1
  locality: cross-domain
  visibility: zone
  attachmentPolicy: none
  consumerPolicy:
    allowedProviderComponents: [core-controller.d2bus.org/zonelink]
    allowedOperations: [resolve]
  lifecyclePolicy: producer-owned
status:
  phase: Ready
  resource:
    readiness: Ready
    observedProducerGeneration: 1
    observedResourceGeneration: 1
    endpointGeneration: 1
    connectionAvailability: Available
    leaseAvailability: Available
  conditions: []
  update:
    state: Current
    reasons: []
    observedGeneration: 1
    targetGeneration: 1
    disruption: None
    preserveState: true
    operationId: null
    lastAssessedAt: null
    owned: { count: 0, refs: [] }
    dependencies: { count: 0, refs: [] }
```

### Endpoint resources (D092)

The child-local service binding with visible lifecycle is the standard
`Endpoint` resource above. Consumers in that child Zone refer to
`Endpoint/<name>` and receive no raw CID, port, socket address, FD, or credential
from `Endpoint.spec` or `Endpoint.status`; resolution is through the authorized
EffectPort/LaunchTicket path, and unauthorized callers fail with
`endpoint-resolve-denied`. The selected parent allocator's listener/route
binding remains sealed allocator state and is **not** an Endpoint or any other
parent-store resource. A producer Process restart bumps the child Endpoint
`endpointGeneration`, which child-local dependents observe as
`dependency-changed`. ZoneLink session state remains owned by the child Zone's
core ZoneLink controller.

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
d2b-bus. All three methods are invoked exclusively by the child Zone's core
Zone-link/delegation controller over a same-Zone service session. No other
caller is authorized, and the parent allocator never invokes this Provider.
Before `OpenTransport`, child core resolves `spec.transportProviderRef`,
validates `spec.transportSettings`, requires `spec.transportCredentials = []`,
checks `spec.disabled`, enforces `spec.limits`, and derives the opaque
endpoint/binding IDs and deadline. The Provider receives only those derived
values, never the ZoneLink spec or a legacy provider envelope.

### `OpenTransport`

```text
rpc OpenTransport(OpenTransportRequest) -> OpenTransportResponse
```

**Request fields**:

| Field | Type | Semantics |
| --- | --- | --- |
| `endpoint_id` | `OpaqueEndpointId` | Child-local opaque endpoint-resolution token derived from the selected allocator's sealed binding; opaque to Provider; not an `Endpoint` resource |
| `binding_id` | `OpaqueBindingId` | Child-local opaque binding identity derived from the selected allocator's sealed port allocation; opaque to Provider |
| `role` | `TransportRole` | `Initiator` (child endpoint connects to the selected parent route endpoint) or `Responder` (child endpoint accepts that parent-facing route); never a parent Provider call |
| `deadline_ms` | `u32` | Connect/accept deadline in ms from call arrival; range 1 000-60 000 |

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

## VsockEffectPort - injected interface

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
strings with no public CID/port accessors. The child Zone runtime provides a
`LiveVsockEffectPort` implementation that consumes its sealed local binding for
the allocator selected by compiler-only `parentZone` and opens the child
AF_VSOCK endpoint. The parent allocator retains only its peer binding and route
allocation as sealed state. Neither a ResourceRef nor an FD crosses the Zone
boundary. Tests inject a `MockVsockEffectPort`.

The Provider never receives, stores, logs, or emits raw CID (`u32`) or port
(`u32`) values. Any path that would write a raw CID or port to a log,
structured event, audit record, metric label, or error message is a security
defect and must be caught by the `redaction.rs` test.

---

## `spec.transportSettings` schema

The child-local `ZoneLink.spec.transportSettings` object carries
Provider-specific configuration when
`spec.transportProviderRef = Provider/transport-vsock`. The child Zone's core resolves
the opaque endpoint and binding IDs from these settings plus its sealed
selected-parent allocation; the Provider never receives the raw resolution.

**JSON Schema ID**: `docs/reference/schemas/v3/providers/transport-vsock.transport-binding.json`

| Field | Type | Default | Semantics |
| --- | --- | --- | --- |
| `guestRef` | `ResourceRef` | required | `Guest/<name>` in the same child Zone as the ZoneLink and selected Provider; child core resolves the local endpoint through the sealed allocation |
| `portClass` | `string` (enum) | `"d2b-link"` | Port class; core allocates a port from the class range; `"d2b-link"` → range `14420-14499` |
| `connectTimeoutSeconds` | `integer` [1, 60] | `30` | Passed as `deadline_ms` in `OpenTransport` |

**Forbidden fields** (rejected by schema `additionalProperties: false`):
`cid`, `port`, `socketPath`, `token`, `password`, and any field whose value
is or contains a raw socket address.

**Reserved ports** - never allocated by `portClass: "d2b-link"`:
- `14317`: OTLP gRPC relay (observability-otel)
- `14318`: guestd ttrpc (guest-control, ADR 0028)
- `14319`: OTLP HTTP relay (observability-otel)

These three ports are enforced as exclusions in the core allocator; the
Provider has no knowledge of them.

---

## ProviderStateSet

A **ProviderStateSet** is the optional, query-time grouping - the set of the
*declared* Volume resources in a Zone whose `metadata.ownerRef` resolves to
`Provider/<name>`. It is not a ResourceType and is empty for a Provider that
declares no state Volume.

`Provider/transport-vsock` declares **no** Provider state Volume; its
`ProviderStateSet` is empty. Its bounded non-secret operational state - service
readiness, transport-open/close reconcile stage, bounded connection/port
observation counters, and closed-enum error detail - lives in the owning
resource's `status` subresource and the child core Operation ledger (D087).
Port allocation and the port registry are selected-parent allocator state; the
parent retains only that sealed allocator/route state. All ZoneLink session
state is owned by the child Zone's core ZoneLink controller under the child-local
`ZoneLink` resource.
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

The child Zone's core ZoneLink/delegation controller is the only principal
authorized to invoke the Provider's service methods. No other principal (the
parent allocator, other Providers, operators, or end-users) may invoke
`OpenTransport`, `CloseTransport`, or `ObserveTransport`.

```yaml
type: Role
metadata:
  name: transport-vsock-caller
  zone: <child-zone>
spec:
  rules:
    - resourceTypes: [Provider]
      verbs: []
      sessionVerbs: [invoke, attach]
      subresources:
        - d2b.transport.vsock.v3.VsockTransportService/OpenTransport
        - d2b.transport.vsock.v3.VsockTransportService/CloseTransport
        - d2b.transport.vsock.v3.VsockTransportService/ObserveTransport
      resourceNames: [transport-vsock]
      zones: [<child-zone>]
      executionRefs: []
---
type: RoleBinding
metadata:
  name: transport-vsock-caller
  zone: <child-zone>
spec:
  roleRef: Role/transport-vsock-caller
  subjects: [Process/core-zone-link-controller]
  externalPrincipalSelector: null
  scopeNarrowing: null
```

### RBAC grants NOT required by this Provider

`Provider/transport-vsock` requires no resource verbs on `Provider`,
`ZoneLink`, `Guest`, `Zone`,
`Route`, `Credential`, `Certificate`, or any other ResourceType. All
ZoneLink resource access is performed exclusively by the child Zone's core
controller. The Provider service holds no permissions to read, watch, update,
or delete any resource. The parent allocator has no resource row or RBAC grant
for this child-local Provider/ZoneLink.

---

## Security invariants

| ID | Invariant | Enforcement |
| --- | --- | --- |
| INV-VSOCK-001 | Raw CID (`u32`) never appears in any resource spec, status field, audit event, metric label, log line, or error message surface | `VsockEffectPort` opacity; `OpaqueEndpointId` newtype has no public CID accessor; `redaction.rs` hermetic test |
| INV-VSOCK-002 | Raw port (`u32`) never appears in any resource spec, status field, audit event, metric label, log line, or error message surface | `OpaqueBindingId` newtype has no public port accessor; `redaction.rs` hermetic test |
| INV-VSOCK-003 | No file descriptor is transferred over the vsock byte channel | `TransportDescriptor.attachment_support = false`; d2b-bus serialization boundary checks `descriptor()` before dispatch; `no_fd_transfer.rs` integration test |
| INV-VSOCK-004 | Provider never calls `socket(AF_VSOCK, …)`, `connect`, or `bind` directly | `VsockEffectPort` is the only AF_VSOCK call surface; `cfg(target_os = "linux")` feature gate; all syscall paths in `LiveVsockEffectPort` only; seccomp strict profile on Provider process |
| INV-VSOCK-005 | Provider never accesses or modifies ZoneLink, Guest, Route, or any other ResourceType | RBAC grants contain no resource-type verbs; Provider holds no resource API client; conformance test asserts no resource calls |
| INV-VSOCK-006 | `spec.transportSettings` schema rejects any field carrying a raw socket address, port number, CID, path, or credential | JSON Schema `additionalProperties: false`; build-time emitter secret-key scanner |
| INV-VSOCK-007 | Provider process: no new privileges, strict seccomp, no network namespace join, read-only root filesystem | `sandbox.seccompClass: strict`, `sandbox.noNewPrivileges: true`, `sandbox.readOnlyRoot: true` in Process template; `Provider/system-minijail` profile |
| INV-VSOCK-008 | Port range `14420-14499` is reserved for `d2b-link` ZoneLink vsock sessions; ports 14317, 14318, 14319 are never allocated by `portClass: "d2b-link"` | Core allocator exclusion list; enforced in core, not in Provider |
| INV-VSOCK-009 | Service component receives only a dirfd into its local `service` view of its own state Volume; no raw filesystem path, no parent-directory access, no cross-component Volume mount | volume-local Provider validates `sensitivityClass: private` and `mountPath` scope before handing dirfd to process; domain isolation enforced at mount time |
| INV-VSOCK-010 | State Volume layout principal `User/d2b-transport-vsock` is a Nix-preprovisioned `User/<name>` ResourceRef; no `ComponentPrincipal` ResourceRef used | Volume `layout[].ownerRef`/`groupRef` fields hold only `User/<name>` refs; validated at Volume admission; Nix module declares the system user |

---

## Lifecycle

Per D088, `Provider/transport-vsock` does not write resource status directly:
child core owns the universal `ResourceStatus` base and the ResourceType-common
`status.resource` projection for Provider and ZoneLink resources. Cross-provider
transport observations returned to child core are promoted to the child-local
`ZoneLink.status.resource`; any bounded, non-secret vsock-only observation that
child core persists for this
Provider uses `ZoneLink.status.provider` with `providerRef:
Provider/transport-vsock`, qualified `schemaId:
transport-vsock.d2bus.org/ZoneLink/status`, `schemaVersion` (semver MAJOR.MINOR),
`observedProviderGeneration`, and a strict unknown-field-denied, ≤32 KiB,
redacted `details` schema registered and signed in the Provider manifest. Child
core writes all present layers atomically in the child store and never copies shared
`status.resource` fields into `status.provider`.

### Provider installation

1. Nix module emits `Provider/transport-vsock` into the same child Zone store as
   its ZoneLink. The parent store receives no Provider or ZoneLink row.
2. The child Zone's core ProviderDeployment creates
   `Volume/transport-vsock--service--empty-state--<executionRef-short>` with
   `ownerRef: Provider/transport-vsock`; waits for Volume `Ready` (reconciled
   by `Provider/volume-local`). The transport-vsock Provider controller does
   not participate in Volume creation.
3. The child Zone's core ProviderDeployment creates
   `Process/transport-vsock-service` (with `mounts` referencing the pre-created
   state Volume); waits for Process `Ready`.
4. Service process connects to d2b-bus; receives a dirfd into its `/state` view
   from the volume-local Provider; registers
   `d2b.transport.vsock.v3.VsockTransportService` on the Zone service registry.
5. Service emits readiness; child-local core ProviderDeployment observes it and
   sets `Provider/transport-vsock` status to `Ready`.

### Transport open (per ZoneLink session request from core)

1. The child Zone's core ZoneLink/delegation controller calls `OpenTransport(endpoint_id,
   binding_id, role, deadline_ms)` on the Provider's ComponentSession.
2. Provider validates opaque IDs.
3. Provider calls `VsockEffectPort::open(...)`.
4. Child Zone runtime (`LiveVsockEffectPort`) resolves the opaque IDs from its
   sealed selected-parent binding and opens or accepts the child AF_VSOCK
   endpoint. The parent endpoint remains sealed allocator/route state; no FD or
   ResourceRef crosses Zones.
5. Provider opens a named stream on its ComponentSession and spawns a bridge
   task.
6. Provider returns `transport_handle` + `stream_id` to child core.
7. Child core hands `stream_id` to d2b-bus as the `OwnedTransport` for its local
   ZoneLink.
8. d2b-bus runs the ZoneLink handshake selected by core from the core-owned
   enrollment-and-session state machine
   `Unenrolled -> IKpsk2 -> EnrollmentCommitted -> KK -> Ready` (canonical in
   ADR-046-zone-routing) on top of the raw bytes: the one-time IKpsk2 bootstrap
   consuming the allocator-issued single-use PSK only when the link is
   `Unenrolled` (and after revocation), otherwise the enrolled KK handshake.
   This Provider carries opaque bytes only and never selects, negotiates, or
   reorders the handshake profile.

### Transport close

1. Child core calls `CloseTransport(transport_handle)`.
2. Provider signals bridge task to stop; waits up to `CLOSE_GRACE_MS`.
3. Provider closes named stream and vsock socket.
4. Provider emits `transport.release` audit event.

### Provider removal

1. Child core calls `CloseTransport` for every open transport handle.
2. Zone runtime stops `Process/transport-vsock-service`; Process finalizer
   completes. The child Zone's core ProviderDeployment owns deletion ordering;
   the transport-vsock service does not delete the Volume.
3. Child-local core ProviderDeployment deletes
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
| `d2b.transport.vsock.active` | Gauge | - | Currently open transport handles |
| `d2b.transport.vsock.bytes_rx` | Counter | - | Bytes received from vsock side of bridge |
| `d2b.transport.vsock.bytes_tx` | Counter | - | Bytes sent to vsock side of bridge |
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
# local-root is the provisioning parent. It has no ZoneLink or transport
# Provider resource for this edge; only its allocator's sealed route state exists.
d2b.zones.local-root = {};

# K1 is the CHILD Zone. parentZone is compiler-only and selects local-root's
# allocator; it is not emitted into Zone/k1 or any other resource.
d2b.zones.k1.parentZone = "local-root";

# This focused fragment assumes Guest/k1-vm is declared in K1.

# Selected Provider resource, authored in the same CHILD Zone as the ZoneLink:
d2b.zones.k1.resources.transport-vsock = {
  type = "Provider";
  spec = {
    artifactId = "transport-vsock";          # resolves to d2b.artifacts entry
    config = {
      executionRef = "Guest/k1-vm";          # same-Zone Host/<name> or Guest/<name>
    };
  };
};
# K1's core ProviderDeployment creates Process/transport-vsock-service
# automatically. The operator does NOT author that Process resource.

# K1's sole child-local uplink. childZoneName self-matches; local-root gets no
# reciprocal ZoneLink or Provider resource.
d2b.zones.k1.resources.k1-uplink = {
  type = "ZoneLink";
  spec = {
    childZoneName        = "k1";
    transportProviderRef = "Provider/transport-vsock";
    transportSettings = {
      guestRef              = "Guest/k1-vm";
      portClass             = "d2b-link";
      connectTimeoutSeconds = 30;
    };
    transportCredentials = [];
    disabled = false;
    limits = {
      maxPendingIntents    = 256;
      maxActiveStreams     = 32;
      reconnectMaxAttempts = 10;
      reconnectWindowSecs  = 300;
    };
  };
};
```

The compiler derives `metadata.zone: k1` for both the Provider and ZoneLink.
The emitted ZoneLink identity is:

```yaml
apiVersion: resources.d2bus.org/v3
type: ZoneLink
metadata:
  name: k1-uplink
  zone: k1
spec:
  childZoneName: k1
  transportProviderRef: Provider/transport-vsock
  transportSettings:
    guestRef: Guest/k1-vm
    portClass: d2b-link
    connectTimeoutSeconds: 30
  transportCredentials: []
  disabled: false
  limits:
    maxPendingIntents: 256
    maxActiveStreams: 32
    reconnectMaxAttempts: 10
    reconnectWindowSecs: 300
```

local-root's resource bundle contains no reciprocal ZoneLink or selected Provider.

`spec.config.executionRef` is validated at eval time against declared Zone
resources in the same child Zone; referential existence is verified at bundle
activation time. The compiler separately seals
`k1.parentZone = "local-root"` into
allocator bootstrap state and emits no parent-store reciprocal resource. The Nix
module also resolves `spec.transportProviderRef` and validates
`spec.transportSettings` against the
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
| ZoneLink resource | `ADR-046-zone-routing.md` | `generated-or-eval-contract` | Retained: exact child-local ZoneLink base spec with `transportProviderRef`, `transportSettings`, empty `transportCredentials`, `disabled`, and `limits`; child core owns local status/routes/finalizer; selected parent retains only sealed allocator/route state with no reciprocal row |
| ComponentSession / Noise | `ADR-046-componentsession-and-bus.md` | `generated-or-eval-contract` | Retained: owned by d2b-bus; Provider is opaque carriage only |

---

## Work items

### ADR046-vsock-001
| Field | Value |
| --- | --- |
| Dependency/owner | Title: Implement `VsockEffectPort` trait and `OpaqueEndpointId`/`OpaqueBindingId` newtypes; Phase 1; Priority P0; Depends on ADR046-bus-001 (OwnedTransport in d2b-session); Owner crate `d2b-provider-transport-vsock`. |
| Current source | Evidence class `test-only-or-preview`; baseline has no generic vsock transport Provider or opaque endpoint/binding ID trait. |
| Reuse action | create |
| Destination | `packages/d2b-provider-transport-vsock/src/effect_port.rs`; test fake in `tests/effect_port_mock.rs`; redaction checks in `tests/redaction.rs`. |
| Detailed design | Implement `VsockEffectPort` trait and `OpaqueEndpointId`/`OpaqueBindingId` newtypes. Define `VsockEffectPort` async trait and opaque ID newtypes in `effect_port.rs`; implement `FakeVsockEffectPort` for tests; `redaction.rs` asserts no raw `u32` in any `Debug`/`Display` output of opaque types; no real vsock socket opened. Primary reuse disposition: `create`. Preserved source-plan detail: net-new trait/newtypes with redaction tests; no real vsock socket opened. |
| Integration | Child Zone's core ZoneLink/delegation controller calls its same-Zone Provider with opaque IDs; Provider calls injected `VsockEffectPort`; live AF_VSOCK child-endpoint resolution remains in child core runtime while the selected parent retains only sealed peer/route state. |
| Data migration | Full d2b 3.0 reset; no v2 state/config import. |
| Validation | Proof type: hermetic unit + redaction test; `tests/effect_port_mock.rs` and `tests/redaction.rs`. |
| Removal proof | None - net-new; no prior owner to remove. |

### ADR046-vsock-002
| Field | Value |
| --- | --- |
| Dependency/owner | Title: Implement framing utilities and bridge task in Provider crate; Phase 1; Priority P0; Depends on ADR046-vsock-001; Owner crate `d2b-provider-transport-vsock`. |
| Current source | Evidence class `implemented-but-unwired`; main commit `a1cc0b2da4a08ca3240a770a972fe4da6f912bef` `packages/d2b-session-unix/src/vsock.rs` contains `FramedVsockTransport` framing behavior, not current v3 baseline behavior. |
| Reuse source | Main `a1cc0b2d` `packages/d2b-session-unix/src/vsock.rs` framing-only code and `packages/d2b-session-unix/tests/unix_session.rs` vsock framing subset. |
| Reuse action | adapt |
| Destination | `packages/d2b-provider-transport-vsock/src/framing.rs` and `src/bridge.rs`; tests in `packages/d2b-provider-transport-vsock/tests/framing.rs`. |
| Detailed design | Implement framing utilities and bridge task in Provider crate. Copy `FramedVsockTransport` framing-only code (length-prefix encode/decode, bounded allocation, EOF/reset) from main `a1cc0b2d` → `framing.rs`; implement bridge task pumping bytes between an opaque `AsyncRead+AsyncWrite` stream from `VsockEffectPort::open` and the named ComponentSession stream; hermetic tests using `FakeVsockEffectPort` (no real socket). Primary reuse disposition: `adapt`. Preserved source-plan detail: copy/adapt framing-only code; exclude raw AF_VSOCK socket calls and ADR 0045 endpoint-role assumptions. |
| Integration | OpenTransport creates a framed opaque stream, bridge task pumps to a named ComponentSession stream, and d2b-bus consumes it as an `OwnedTransport` without FD attachment support. |
| Data migration | Full d2b 3.0 reset; no v2 state/config import. |
| Validation | Proof type: hermetic framing tests; `tests/framing.rs` covers partial/coalesced records, oversized frames, EOF/reset classification, and no real socket. |
| Removal proof | None for framing; raw socket portions from the source are deliberately not copied into the Provider crate. |

### ADR046-vsock-003
| Field | Value |
| --- | --- |
| Dependency/owner | Title: Implement `VsockTransportService` (OpenTransport/CloseTransport/ObserveTransport); Phase 1; Priority P0; Depends on ADR046-vsock-002 and ADR046-bus-001; Owner crate `d2b-provider-transport-vsock`. |
| Current source | Evidence class `test-only-or-preview`; no current v3 generic `VsockTransportService` implementation exists. |
| Reuse action | adapt |
| Destination | `packages/d2b-provider-transport-vsock/src/service.rs`; tests `tests/open_close.rs`, `tests/observe.rs`, and conformance kit. |
| Detailed design | Implement `VsockTransportService` (OpenTransport/CloseTransport/ObserveTransport). Child core resolves the exact six-field ZoneLink spec, validates `transportSettings` through the Provider selected by `transportProviderRef`, rejects non-empty `transportCredentials`, and passes only derived opaque IDs/deadline to `service.rs`; `open_close.rs`, `observe.rs`, and topology tests cover the service and reject legacy ZoneLink provider/fingerprint/capability fields; conformance kit passes. Primary reuse disposition: `adapt`. Preserved source-plan detail: net-new service implementation over ComponentSession and fake effect port tests. |
| Integration | Child Zone's core ZoneLink/delegation controller is the only authorized caller; service opens named stream handles for child d2b-bus, releases them on close, and streams transport events for observe; no parent-side Provider or handler exists. |
| Data migration | Full d2b 3.0 reset; no v2 state/config import. |
| Validation | Proof type: service round-trip plus exact-shape tests; `tests/open_close.rs`, `tests/observe.rs`, `tests/topology.rs::{canonical_zonelink_spec_fields_are_exact,legacy_zonelink_provider_fields_are_rejected,transport_credentials_must_be_empty}`, and provider conformance tests. |
| Removal proof | None - net-new; no prior owner to remove. |

### ADR046-vsock-004
| Field | Value |
| --- | --- |
| Dependency/owner | Title: Implement `LiveVsockEffectPort` in child Zone runtime; Phase 2; Priority P0; Depends on ADR046-vsock-001 and the Zone allocator (`ADR-046-resources-zone-control`); Owner crate `d2b-core-controller`. |
| Current source | Evidence class `ADR-only`; baseline has guest-control and relay vsock paths, but no allocator-backed `LiveVsockEffectPort` for ZoneLink transport. |
| Reuse action | adapt |
| Destination | `d2b-core-controller` child Zone runtime `LiveVsockEffectPort`; child-local Provider receives it by dependency injection at startup. |
| Detailed design | Implement `LiveVsockEffectPort` in the child Zone runtime. It consumes the child's sealed binding for the allocator selected by compiler-only `parentZone`, resolves opaque endpoint/binding IDs only inside the effect adapter, opens the child AF_VSOCK endpoint, and injects an opaque stream into the same-Zone Provider service; the selected parent keeps its peer binding and port registry only as sealed allocator/route state; no raw CID/port is exposed to the Provider and no FD or ResourceRef crosses Zones. Primary reuse disposition: `adapt`. Preserved source-plan detail: net-new core adapter; keep raw AF_VSOCK syscalls outside Provider crate. |
| Integration | Selected parent allocator issues a sealed endpoint/binding allocation without creating resources; child core resolves its local side, opens/accepts the AF_VSOCK endpoint, returns an opaque stream to its Provider service, and excludes reserved ports. |
| Data migration | Full d2b 3.0 reset; no v2 state/config import. |
| Validation | Proof type: integration test; `integration/host_guest.rs` exercises live open/close byte round-trip with the injected effect and proves the selected parent has only sealed allocator/route state, with no parent-store Provider/ZoneLink row. |
| Removal proof | None - net-new core adapter; no prior owner to remove. |

### ADR046-vsock-005
| Field | Value |
| --- | --- |
| Dependency/owner | Title: Child core ProviderDeployment creates/deletes service component state Volume; Phase 1; Priority P0; Depends on the volume-local Provider (`ADR-046-provider-volume-local`); Owner crate `d2b-provider-transport-vsock`. |
| Current source | Evidence class `test-only-or-preview`; no operator-authored v3 state Volume exists for transport-vsock in baseline. |
| Reuse action | create |
| Destination | ProviderDeployment Volume creation/deletion path plus `packages/d2b-provider-transport-vsock/tests/state_volume.rs`. |
| Detailed design | Child core ProviderDeployment creates/deletes the service component state Volume in the same child Zone. It creates `Volume/transport-vsock--service--empty-state--*` before the component Process and deletes it after the Process finalizer; the transport-vsock service does not own Volume, add Volume to exported ResourceTypes, or create its prerequisite; Volume spec: empty schema, `kind: state`, `persistenceClass: persistent`, `migrationPolicy: none`, `User/d2b-transport-vsock` owner, minimal nonzero `quota.maxBytes`/`quota.maxInodes` with `enforcement: hard`, `private` sensitivity, `broker-maintained` identity marker; `state_volume.rs` tests the canonical schema; installation/removal tests verify marker lifecycle; no operator-authored or parent-store Volume exists; component receives only its child-local dirfd view. Primary reuse disposition: `create`. Preserved source-plan detail: net-new ProviderDeployment/Volume integration and tests. |
| Integration | Child core ProviderDeployment creates the Volume before Process, volume-local reconciles it, the Provider process receives only a child-local dirfd view, and Provider deletion removes Process before Volume/identity marker; parent state remains allocator/route-only. |
| Data migration | Full d2b 3.0 reset; no v2 state/config import; state Volume is created fresh with `migrationPolicy: none`. |
| Validation | Proof type: unit + integration test; `tests/state_volume.rs` and Provider install/remove integration tests verify schema, user refs, marker lifecycle, and no ComponentPrincipal. |
| Removal proof | Remove the state Volume and its broker-maintained identity marker during Provider deletion; no operator-authored Volume remains. |

### ADR046-vsock-006
| Field | Value |
| --- | --- |
| Dependency/owner | Title: Integration test: real vsock socketpair + full ZoneLink open/close; Phase 2; Priority P1; Depends on ADR046-vsock-003 and ADR046-vsock-004; Owner crate `d2b-provider-transport-vsock`. |
| Current source | Evidence class `test-only-or-preview`; existing guest-control compile proof and socat relay tests are not full ZoneLink transport coverage. |
| Reuse action | create |
| Destination | `packages/d2b-provider-transport-vsock/integration/host_guest.rs` and `integration/no_fd_transfer.rs`. |
| Detailed design | Integration test: fixture declares compiler-only `k1.parentZone = "local-root"`, puts the selected Provider and an exact six-field ZoneLink (`transportProviderRef`, validated `transportSettings`, empty `transportCredentials`, `disabled`, and `limits`, plus self-matching `childZoneName`) only in K1, and gives local-root only sealed allocator/route state; `integration/host_guest.rs` opens a real Linux vsock path through K1's `LiveVsockEffectPort`, then exercises `OpenTransport` + byte round-trip + `CloseTransport` and validates bridge throughput ≥ 512 MiB/s; `no_fd_transfer.rs` structurally rejects attachment packets and asserts no FD/ResourceRef crosses the Zone boundary. Primary reuse disposition: `create`. Preserved source-plan detail: net-new integration coverage with no FD transfer over vsock. |
| Integration | Test drives K1-local Provider service, child `LiveVsockEffectPort`, d2b-bus `OwnedTransport`, byte bridge, close path, absent local-root reciprocal resource row, and attachment rejection across the integration lane. |
| Data migration | None - docs/tooling only; no runtime state. |
| Validation | Proof type: integration test; `make test-integration` runs `host_guest.rs` and `no_fd_transfer.rs`. |
| Removal proof | None - test coverage net-new; old duplicate vsock tests are retired only after successor assertions migrate. |

### ADR046-vsock-007
| Field | Value |
| --- | --- |
| Dependency/owner | Title: Delete legacy socat OTLP relay and CONNECT-proxy guest-control vsock; Phase 3; Priority P2; Depends on the observability-otel Provider (`ADR-046-provider-observability-otel`) and the Guest resource lifecycle (`ADR-046-resources-host-guest-process-user`); Owner crates `d2b-host`, `d2bd`. |
| Current source | Evidence class `implemented-and-reachable`; legacy sources are `packages/d2b-host/src/vsock_relay_argv.rs` socat OTLP relay and `packages/d2bd/src/guest_control_vsock.rs` CONNECT-proxy guest-control path. |
| Reuse action | delete-after-cutover |
| Destination | Remove legacy paths from `d2b-host` and `d2bd`; replacement lives in `observability-otel` Provider native vsock relay and Guest resource lifecycle/bootstrap. |
| Detailed design | Delete legacy socat OTLP relay and CONNECT-proxy guest-control vsock. Remove `vsock_relay_argv.rs` socat path after `observability-otel` Provider native vsock relay passes parity; remove `guest_control_vsock.rs` CONNECT-proxy after Guest resource lifecycle + guestd vsock bootstrap reach parity; no raw CID or socat vsock path remains. Primary reuse disposition: `delete-after-cutover`. Preserved source-plan detail: delete after replacement parity; preserve reserved guest-control/OTLP port exclusions until replacements own them. |
| Integration | Observability-otel owns OTLP vsock relay replacement; Guest lifecycle owns guest-control bootstrap replacement; transport-vsock ZoneLink allocator excludes ports 14317, 14318, and 14319. |
| Data migration | Full d2b 3.0 reset; no v2 relay or guest-control state/config import. |
| Validation | Proof type: deletion + parity test; parity tests for observability-otel and Guest lifecycle plus redaction checks that no raw CID/socat vsock path remains. |
| Removal proof | Delete `packages/d2b-host/src/vsock_relay_argv.rs` socat path after observability parity, delete `packages/d2bd/src/guest_control_vsock.rs` CONNECT-proxy after Guest lifecycle parity, and prove no raw CID or socat vsock path remains. |

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
| `canonical_zonelink_spec_fields_are_exact` | `tests/topology.rs` | unit | `cargo test` |
| `legacy_zonelink_provider_fields_are_rejected` | `tests/topology.rs` | unit | `cargo test` |
| `transport_credentials_must_be_empty` | `tests/topology.rs` | unit | `cargo test` |
| `provider_and_zonelink_are_child_local` | `tests/topology.rs` | unit | `cargo test` |
| `child_zone_name_self_matches_and_parent_is_compiler_only` | `tests/topology.rs` | unit | `cargo test` |
| `parent_store_has_no_reciprocal_resources` | `tests/topology.rs` | unit | `cargo test` |
| `state_volume_spec_matches_canonical_schema` | `tests/state_volume.rs` | unit | `cargo test` |
| `state_volume_layout_uses_nix_user_ref_not_component_principal` | `tests/state_volume.rs` | unit | `cargo test` |
| `conformance_provider_registers_service` | `tests/` (conformance kit) | unit | `cargo test` |
| `host_guest_vsock_byte_roundtrip` | `integration/host_guest.rs` | integration | `make test-integration` |
| `no_fd_transfer_over_vsock` | `integration/no_fd_transfer.rs` | integration | `make test-integration` |

### Fast hermetic execution and test placement (D094)

Per D094 and `ADR-046-validation-and-delivery` §10.16, this Provider's `src/`
unit tests and `tests/*.rs` hermetic suite are fast, in-process, deterministic,
and parallel-safe: an individual normal test has an advisory wall-clock p95
diagnostic threshold of <=50 ms; gate enforcement is aggregate per-crate
process CPU only. There is no wall-clock
sleep, and `cargo test -p d2b-provider-transport-vsock --lib --tests` completes in ≤2 s warm-cache
execution time (compilation excluded). They use a deterministic fake clock/RNG
and the toolkit fakes/FakeEffectPort only - no process spawn, container,
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
1. Any child-local `ZoneLink` resource with
   `spec.transportProviderRef: Provider/transport-vsock` exists in its owning
   child Zone.
2. Any `d2b-link` port-class vsock session is active on any Zone runtime.
3. The state Volume (`Volume/transport-vsock--service--empty-state--*`) has not
   been deleted and its identity marker has not been cleared.
4. The legacy `vsock_relay_argv.rs` socat path has not been fully replaced by
   `observability-otel` Provider native vsock transport (ADR046-vsock-007).
5. The `guest_control_vsock.rs` CONNECT-proxy path has not been fully replaced
   by the Guest resource lifecycle bootstrap (ADR046-vsock-007).

When all five conditions are clear, the removal commit must delete
`packages/d2b-provider-transport-vsock/` and the `transport-vsock` entry in
the Provider catalog in `ADR-046-provider-model-and-packaging.md`. Removal
proof must also show that topology fixtures contain no parent-store reciprocal
Provider/ZoneLink row and that no cross-Zone FD/ResourceRef path replaced the
transport.

Per D094, each replaced current-code test is retired with an explicit
keep/adapt/move/delete disposition and a removal gate: the minimum reusable
semantic assertions migrate into this crate's hermetic `tests/`, and the old
duplicate tests, shell gates, fixtures, static artifacts, CI jobs, and manifest
entries are deleted once successor coverage and the removal proof pass -
updating `tests/layer1-jobs.json`, the closed gate manifests, the
flake/matrix/Nix-unit pins, the generated ledgers, and the CI workflow shards.
Old and new suites never run in parallel indefinitely.

---

## README.md requirements

`packages/d2b-provider-transport-vsock/README.md` must document:

- Provider identity: `Provider/transport-vsock`; carriage-acquisition service Provider.
- Placement: Provider and ZoneLink are in the same child Zone;
  `spec.childZoneName` self-matches; compiler-only `parentZone` selects the
  allocator; the parent has only sealed allocator/route state and no reciprocal
  resource.
- Role: `service` component; no ResourceType ownership; the child Zone's core
  ZoneLink/delegation controller is the sole ZoneLink reconciler and the only
  caller of this service.
- ProviderStateSet: empty - `Provider/transport-vsock` declares no Provider
  state Volume; its bounded non-secret operational state lives in `status`/the
  child core Operation ledger (D087). All ZoneLink session state is owned by the
  child Zone's core ZoneLink controller; the service holds only opaque
  byte-stream handles in process memory.
- `Provider.spec.config.executionRef`: required `Host/<name>` or `Guest/<name>`;
  one service Process per Provider instance, not per ZoneLink.
- `spec.transportSettings`: `guestRef` / `portClass` fields and forbidden raw
  endpoint values; `spec.transportCredentials` must be empty.
- Port range `14420-14499` reservation; ports 14317/14318/14319 excluded.
- `VsockEffectPort` injection: Provider never calls AF_VSOCK syscalls directly;
  `tokio-vsock` is NOT a Provider crate dependency.
- Components: `d2b-transport-vsock` binary; one service process per Zone.
- Dependencies: `d2b-session`, `d2b-contracts`, `d2b-provider`, `tokio` (no `tokio-vsock`).
- RBAC: only the child Zone's core ZoneLink controller may invoke service
  methods.
- Build: `cargo build -p d2b-provider-transport-vsock`.
- Tests: `cargo test -p d2b-provider-transport-vsock`.
- Integration: `make test-integration`.
- Link to this spec: `docs/specs/providers/ADR-046-provider-transport-vsock.md`.
