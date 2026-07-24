# ADR 0046 Provider dossier: observability-otel

| Field | Value |
| --- | --- |
| Spec ID | `ADR-046-provider-observability-otel` |
| Parent | ADR 0046 |
| Status | Proposed |
| Version | 3 |
| Baseline | `b5ddbed67867d9244bf33390868101bd9b053e49` |
| Normative | Yes |
| Owners | `d2b-provider-observability-otel`, `TelemetryService`/`TelemetryBinding` controllers, telemetry/observability integrator |
| Depends on | `ADR-046-terminology-and-identities`, `ADR-046-resource-object-model`, `ADR-046-resource-api-and-authorization`, `ADR-046-resource-reconciliation`, `ADR-046-provider-model-and-packaging`, `ADR-046-primitive-resource-composition`, `ADR-046-components-processes-and-sandbox`, `ADR-046-componentsession-and-bus`, `ADR-046-resources-volume`, `ADR-046-resources-credential`, `ADR-046-telemetry-audit-and-support`, `ADR-046-nix-configuration` |
| Supersedes | `ProcessRole::OtelHostBridge` / `RunnerRole::OtelHostBridge`; socat-based vsock forwarder in `packages/d2b-host/src/otel_host_bridge_argv.rs`; `packages/d2bd/src/otel_host_bridge_readiness.rs`; hand-rolled per-VM `nixos-modules/components/observability/` pipeline (adapted to per-Zone) |

## Purpose

This dossier exhaustively specifies `Provider/observability-otel`: the d2b 3.0
observability telemetry Provider and its universal cross-Zone service/binding
pair:

- `telemetry.d2bus.org.TelemetryService` is the provider-neutral semantic ingest
  capability. The owner-Zone authority instance represents one telemetry-ingest
  service and is the `ResourceExport` target. Each producer Zone receives a
  core-owned local projection of the same type. This initial Provider selects
  SigNoz and OTLP only through the Service's strict `spec.provider`.
- `telemetry.d2bus.org.TelemetryBinding` is provider-neutral per-Zone or per-Guest
  producer intent and bounded, non-secret operational status. It references a
  same-Zone `TelemetryService` and owns the edge realization.

`Endpoint` resources remain private implementation transport. An Endpoint is
never the semantic imported capability or local projection.

Within Service and Binding resource objects, OTEL, OTLP, SigNoz, collector,
forwarder, and backend choices are forbidden in the provider-neutral type name,
base spec, and `status.resource`. They appear only in this implementation's
strict `spec.provider`, bounded `status.provider`, or installation-wide
`Provider.spec.config`. This dossier keeps backend/protocol choices
per-resource in `spec.provider`; its Provider config remains limited to
installation-wide self-metrics. No provider-qualified ResourceType alias
exists.

The Provider is the only place in the d2b process graph that links the full
OpenTelemetry SDK with an OTLP/gRPC exporter. Every core process uses a
lightweight `BoundedEmitter` (from `d2b-telemetry`) that has no
`opentelemetry_sdk` or `opentelemetry-otlp` dependency.

The Provider is an **ordinary optional non-bootstrap Process**. Zone runtime,
core-controller, and all mandatory Provider processes start before and without
it. Zone startup never waits for it. A Provider that is not installed imposes no
health condition on the Zone. Only an installed Provider that becomes unready or
enters outage transitions Zone health to `Degraded`, never `Failed`, and does
not affect authoritative audit.

---

## Terminology mapping (current baseline → v3 target)

| Baseline name / symbol | v3 target | Evidence class |
| --- | --- | --- |
| `ProcessRole::OtelHostBridge` (`d2b-core/src/processes.rs`) | `Process` resource with `ownerRef: telemetry.d2bus.org.TelemetryBinding/<name>`; template `otel-collector` | implemented-and-reachable |
| `RunnerRole::OtelHostBridge` (`d2b-contracts/src/broker_wire.rs`) | ProviderSupervisor launch ticket issued by observability-otel controller; broker `SpawnRunner` retired | implemented-and-reachable |
| `OtelHostBridgeArgvInputs` (`packages/d2b-host/src/otel_host_bridge_argv.rs`) | native OTLP/gRPC-over-vsock forwarder owned by the Provider; socat argv retired | implemented-and-reachable |
| `OtelHostBridgeReadiness::{Ready,Pending,Failed}` (`packages/d2bd/src/otel_host_bridge_readiness.rs`) | `TelemetryBinding` ResourceType-common phase/conditions and bounded Provider status extension | implemented-and-reachable |
| `nixos-modules/components/observability/host.nix` `otelRuntimeDir = "/run/d2b/otel"` | `$ZONE_STATE/telemetry/` Volume owned by observability-otel controller | implemented-and-reachable |
| `nixos-modules/components/observability/host.nix` `hostEgressSocket` | `$ZONE_STATE/telemetry/emitter.sock` datagram socket, owned by the collector process UID | implemented-and-reachable |
| `nixos-modules/components/observability/stack.nix` `ingressSources` per `vmName` | authority `TelemetryService` plus one producer `TelemetryBinding` per Zone/Guest; source identity is stamped from `producerRef` | generated-or-eval-contract |
| `nixos-modules/components/observability/guest.nix` per-VM guest collector | `TelemetryBinding`-owned edge collector/forwarder children route through the same-Zone `TelemetryService` | generated-or-eval-contract |
| `d2b.observability.vmName` / `identityName` Nix options | `d2b.zones.<name>` attr key populates Zone name; `d2b.observability.host.identityName` preserved unchanged | generated-or-eval-contract |

---

## Provider identity

| Field | Value |
| --- | --- |
| `providerRef` | `Provider/observability-otel` |
| Crate name | `d2b-provider-observability-otel` |
| Package | signed Nix package from `d2b.artifacts.provider-observability-otel` |
| API major | `1` |
| Cardinality | at most one per Zone (enforced at Provider resource admission) |
| Bootstrap role | none; ordinary optional Process only |
| Core aggregate budget | not counted; optional Provider has its own separate budget |
| SDK linkage | this crate is the **only** place that depends on `opentelemetry_sdk` and `opentelemetry-otlp` |
| Qualified ResourceTypes | `telemetry.d2bus.org.TelemetryService`, `telemetry.d2bus.org.TelemetryBinding` (initial implementation) |

---

## Crate layout

```
packages/d2b-provider-observability-otel/
  src/
    lib.rs                  provider identity + version constants
    config.rs               config schema DTOs and validation
    controller.rs           TelemetryService/TelemetryBinding reconciliation
    authority.rs            D097 owner proof and uniqueness
    service.rs              authority and imported service-projection semantics
    binding.rs              producer intent, child ownership, and status
    projection.rs           core-owned import projection adapter
    share_adapter.rs        D096 ExportAdapter/ImportAdapter
    collector_bin.rs        collector binary entry point (full OTEL SDK)
    forwarder_bin.rs        vsock-forwarder long-lived Process entry point
    emitter_socket.rs       datagram socket drain loop
    exporter.rs             OTLP/gRPC exporter setup, retry, backpressure
    metrics.rs              self-metrics (d2b_otel_*) definitions
    journald.rs             optional journald receiver (disabled by default)
    redaction.rs            redaction filter applied before forwarding
    nix/
      journald.nix          per-Zone journald cgroup filter config fragment
  tests/
    emitter_socket_receive.rs
    emitter_ring_drains_on_socket_available.rs
    emitter_ring_drop_on_overflow.rs
    no_vm_label_in_metrics.rs
    zone_startup_proceeds_without_provider.rs
    exporter_outage.rs
    exporter_backpressure.rs
    bundle_contract.rs
    controller_conformance.rs
    provider_state.rs
    config_schema.rs
    resource_service_binding.rs
    projection_chain.rs
    redaction.rs
  integration/
    scenario_full_pipeline.rs
    scenario_obs_zone_forwarding.rs
    scenario_signoz_authority.rs
    scenario_real_projection_stream.rs
    scenario_provider_removal.rs
  README.md
```

Workspace policy rejects a Provider crate missing any of `src/`, `tests/`,
`integration/`, or `README.md` (per `ADR-046-provider-model-and-packaging`
§Crate/package boundary).

---

## Process components

### edge collector (controller-managed, long-lived Process)

One edge collector realizes each admitted `TelemetryBinding`. It runs the full
OTEL SDK and sends only through the Binding's same-Zone `serviceRef`.

| Field | Value |
| --- | --- |
| Binary | `d2b-provider-observability-otel-collector` (built from `src/collector_bin.rs`) |
| Process template name | `otel-collector` |
| Component type | `service` |
| cardinality | exactly one per admitted `TelemetryBinding` |
| executionRef | derived from the same-Zone `producerRef` and Provider extension |
| domain | `system` |
| ownerRef | `telemetry.d2bus.org.TelemetryBinding/<name>` |
| `managedBy` | `controller` |
| Restart policy | restart-on-failure with bounded exponential backoff; max 5 restarts in 10 min before `Failed` |
| SDK linkage | `opentelemetry_sdk`, `opentelemetry-otlp`, `opentelemetry_sdk::export::trace`, `opentelemetry_sdk::metrics` |

**Process responsibilities:**
1. Owns the Binding-private `emitter.sock` Unix datagram Endpoint and drains
   compact telemetry frames from authorized `BoundedEmitter` instances.
2. Owns the Binding-private `otlp.sock` Endpoint for Provider processes that
   embed the full SDK.
3. Decodes frames and reconstructs the selected OTEL metrics/traces/logs.
4. Resolves the same-Zone `TelemetryService` named by
   `TelemetryBinding.spec.serviceRef` and exports only through its authorized
   route. It never resolves a remote ResourceRef.
5. Enforces the Binding's quotas, redaction/cardinality policy, and
   drop/backpressure behavior before sending.
6. Upserts source identity from the trusted `producerRef` observation;
   incoming self-asserted Zone/Guest identity never overrides that stamp.
7. Runs optional journald ingestion only when enabled by the strict
   `TelemetryBinding.spec.provider.settings` extension.
8. Registers the `d2b.observability.v1.SelfMetrics` ComponentSession service on
   d2b-bus (`direction: local`) for `d2b zone doctor` and support bundles.

### vsock-forwarder (long-lived Process, one per Guest Binding)

A long-lived Process active for a Guest-scoped `TelemetryBinding`, launched by
the observability-otel controller to forward telemetry to that Binding's edge
collector.

| Field | Value |
| --- | --- |
| Binary | `d2b-provider-observability-otel-forwarder` (built from `src/forwarder_bin.rs`) |
| Process template name | `otel-vsock-forwarder` |
| Component type | `worker` (long-lived Process) |
| cardinality | at most one per Guest-scoped `TelemetryBinding` |
| executionRef | Host backing the same-Zone `producerRef` |
| domain | `system` |
| ownerRef | `telemetry.d2bus.org.TelemetryBinding/<name>` |
| `managedBy` | `controller` |
| Restart policy | restart-on-failure with bounded backoff; desired lifecycle set to `Stopped` on producer/Binding deletion |
| SDK linkage | none; this binary is a thin vsock ↔ Unix stream relay only |

**Process responsibilities:**
1. Listens on a privately allocated vsock transport Endpoint.
2. Accepts OTLP/gRPC from the exact Guest named by `producerRef`.
3. Forwards frames to the Binding-private collector Endpoint through a
   LaunchTicket; no locator appears in spec/status/audit.
4. Enforces per-frame bounded size (max 4 MiB), Binding quota/credit, and timeout.
5. Stops before the owning Binding finalizer clears.
6. Uses no OTEL SDK.
7. Replaces the socat-based `OtelHostBridgeArgvInputs` relay.

---

## Root config schema

Provider root config installs the controller and sets only bounded
installation-wide implementation defaults. Semantic producer intent belongs to
`TelemetryBinding`; ingest authority and routing belong to `TelemetryService`.
No `serviceRef`, `producerRef`, signal selection, quota, policy, Endpoint,
transport route, token, password, key, URI, CID, port, TLS material, or
Credential ref may appear in `Provider.spec.config`.

```yaml
spec:
  artifactId: provider-observability-otel
  config:
    selfMetrics:
      enable: true
```

### Forbidden config values (enforced at eval and build time)

- Any ResourceRef, signal, quota, routing, Endpoint, transport, address, CID,
  port, TLS, Credential, batching, ring, journald, or redaction field. These
  belong to a qualified ResourceType base or strict Provider extension.
- Any leaf value matching a secret heuristic pattern.
- A non-boolean `selfMetrics.enable`.
- Any unknown `spec.config.*` field.

---

## ResourceTypes

### Implemented initially by this Provider

The signed Provider manifest registers the initial implementation of these
provider-neutral public qualified ResourceTypes:

| ResourceType | Scope | Semantic role | Exportability |
| --- | --- | --- | --- |
| `telemetry.d2bus.org.TelemetryService` | one authority in the owner Zone; one core-owned projection in each importing producer Zone | stable telemetry ingest service capability | authority instance is the `ResourceExport` target; projections are never re-exported |
| `telemetry.d2bus.org.TelemetryBinding` | one per Zone or Guest producer | producer intent and bounded operational status | forbidden; it is never a `ResourceExport` target |

The manifest binds the canonical provider-neutral base schema IDs
`telemetry.d2bus.org/TelemetryService/spec`,
`telemetry.d2bus.org/TelemetryService/status`,
`telemetry.d2bus.org/TelemetryBinding/spec`, and
`telemetry.d2bus.org/TelemetryBinding/status` to exact versions and
fingerprints, then registers this implementation's separate strict Provider
extensions. The canonical API type IDs use
`telemetry.d2bus.org.TelemetryService` and
`telemetry.d2bus.org.TelemetryBinding`; ResourceRefs use
`telemetry.d2bus.org.TelemetryService/<name>` and
`telemetry.d2bus.org.TelemetryBinding/<name>`. The
`observability-otel.d2bus.org` namespace is reserved for this implementation's
strict `spec.provider` and `status.provider` schemas; it is not a ResourceType
alias.

#### TelemetryService base spec (D089)

| Field | Type | Required | Bounds/semantics |
| --- | --- | --- | --- |
| `providerRef` | ResourceRef | yes | Provider registered for this provider-neutral type; initially `Provider/observability-otel` |
| `serviceRole` | enum | yes | `authority` or `projection` |
| `ingestEndpointRefs` | `[ResourceRef]` | authority only | 1..8 same-Zone local telemetry-ingest `Endpoint` refs; absent on a projection |
| `signals` | enum set | yes | non-empty subset of `metrics`, `traces`, `logs` |
| `quota` | object | yes | bounded `maxProducers`, `bytesPerSecond`, `burstBytes`, `maxInFlightBytes`, `maxStreamsPerProducer` |
| `policy` | object | yes | provider-neutral `backpressure` (`drop-oldest`), `redactionProfile`, `cardinalityProfile`, source-stamping requirement, and disconnect behavior |
| `authorityDescriptor` | D097 object | authority only | `authorityScope: external-service`, opaque key class, `cardinality: exactly-one`, `arbitration: multiplexed`, `exportability: explicit-export` |
| `updatePolicy` | object | no | D091 manual-disruptive default |

An authority Service represents one semantic telemetry-ingest service, not an
individual socket, backend product, or wire protocol. It references local
ingest Endpoints and carries the D097 descriptor. Its
`ResourceExport.spec.resourceRef` names the
`TelemetryService`; the required `ResourceExport.spec.endpointRef` names one
local ingest Endpoint only as the private arbitration transport front door.
`exportedType` and `projectionType` are
`telemetry.d2bus.org.TelemetryService`.

This initial implementation places every OTEL, OTLP, and SigNoz choice in the
strict Provider extension:

```yaml
provider:
  schemaId: observability-otel.d2bus.org/TelemetryService/spec
  schemaVersion: "1.0"
  settings:
    backend: signoz
    backendEndpointRefs:
      - Endpoint/signoz-query-backend
    ingestProtocol: otlp-grpc
```

The extension is authority-only, signed, versioned/digested, deny-unknown, and
bounded. A projection has no Service `spec.provider`. The extension cannot
shadow the generic ingest Endpoint refs, signals, quota, policy, authority
descriptor, or update policy and contains no locator, credential, or secret.

A projection Service is created and owned by core with
`metadata.ownerRef: ResourceImport/<name>`. Core and the signed import adapter
derive its signal/quota/policy ceiling from the admitted export. It has no
`authorityDescriptor`, Provider backend ownership, Provider backend Endpoint,
local backend Process, or durable storage. It routes to the authority over the
import's bounded encrypted stream. No remote Ref or route locator appears in
its spec.

`TelemetryService.status.resource` contains `serviceRole`,
`serviceReadiness`, effective signals/quota/policy digests, bounded local
ingest Endpoint readiness summaries for an authority, or import lease/route readiness
for a projection, plus admitted-producer counts, bounded queue/drop counters,
and D091 currency. It never contains an address, CID, port, socket path,
credential, payload, raw `exportKey`, or stream handle.

Closed provider-neutral Service conditions are `ServiceReady`, `IngestReady`,
`AuthorityUnique`, `ExportReady`, `ImportBound`, `RouteReady`,
`QuotaSaturated`, `BackpressureActive`, `Revoking`, and `UpgradeRequired`.
Authority conditions that are inapplicable to projections are absent, not
false. Projection loss/revocation sets `ImportBound=False`,
`RouteReady=False`, and phase `Degraded`.

`TelemetryService.status.provider.details` for this implementation may contain
only bounded non-secret `backend: signoz`, `ingestProtocol: otlp-grpc`, backend
readiness, collector stage, and closed error code on an authority. A projection
has no backend detail and may expose only a bounded import-adapter stage/error
from this schema. None is duplicated into `status.resource`. The strict status
extension schema ID is `observability-otel.d2bus.org/TelemetryService/status`.

#### TelemetryBinding base spec (D089)

| Field | Type | Required | Bounds/semantics |
| --- | --- | --- | --- |
| `providerRef` | ResourceRef | yes | Provider registered for this provider-neutral type; initially `Provider/observability-otel` |
| `serviceRef` | ResourceRef | yes | same-Zone `telemetry.d2bus.org.TelemetryService/<name>`; authority or projection |
| `producerRef` | ResourceRef | yes | same-Zone `Zone/<name>` or `Guest/<name>` |
| `signals` | enum set | yes | non-empty subset of `metrics`, `traces`, `logs` and of the Service signal set |
| `quota` | object | yes | bounded producer rate, burst, queue bytes, in-flight bytes, stream count, and drop budget; cannot exceed Service/import quota |
| `policy` | object | yes | provider-neutral backpressure, disconnect, redaction/cardinality profile, and mandatory trusted source-identity stamping |
| `updatePolicy` | object | no | D091 manual-disruptive default |

These fields are ResourceType-common because every implementation must expose
the same producer/service contract. Implementation-only settings use exactly
one strict extension:

```yaml
provider:
  schemaId: observability-otel.d2bus.org/TelemetryBinding/spec
  schemaVersion: "1.0"
  settings:
    executionRef: Host/host-system
    emitterRingBytes:
      logs: 2097152
      metrics: 4194304
      traces: 4194304
    otlpExporter:
      batchExportTimeoutMs: 30000
      batchMaxExportSize: 512
      batchScheduleDelayMs: 5000
      compressionEnabled: true
      failureThreshold: 5
      maxQueueSize: 2048
    journald:
      enable: false
```

The extension is signed, versioned/digested, deny-unknown, bounded, and cannot
shadow `serviceRef`, `producerRef`, signals, quota, policy, update policy, or
source identity. It contains no locator or secret.

`TelemetryBinding.status.resource` contains observed Service and producer
generations, effective signals/quota/policy digests, queue occupancy/drop
counters, last successful ingest class/time, and D091 currency. Source identity
is reported only as `stamped: true|false` plus a stable producer-kind enum; raw
payload fields are never echoed. `status.provider.details` for this
implementation may carry only bounded owned Process/Endpoint/Volume refs and
readiness summaries, collector/forwarder stage, OTLP retry class, and closed
error code. Its strict status extension schema ID is
`observability-otel.d2bus.org/TelemetryBinding/status`.

Closed provider-neutral Binding conditions are `ProducerReady`, `ServiceResolved`,
`SourceIdentityStamped`, `QuotaEnforced`, `BackpressureActive`,
`IngestAvailable`, `Draining`, and `UpgradeRequired`. Ingest outage or
backpressure degrades the Binding but never fails Zone startup or authoritative
audit.

#### Ownership, finalizers, and Endpoint boundary

- An authority Service is explicitly configuration/API-owned in the authority
  Zone; the controller never auto-creates a second authority. A projection
  Service is always core-owned with `ownerRef: ResourceImport/<name>`.
- A Binding may be configuration-owned or owned by its same-Zone producer. Every
  edge collector/forwarder Process, runtime sockets Volume, and private ingest
  Endpoint has `ownerRef: telemetry.d2bus.org.TelemetryBinding/<name>`.
- Service deletion stops dependent Bindings, revokes exports/import leases, and
  then releases local Endpoint references. Projection deletion is owned by the
  `ResourceImport` finalizer.
- Binding deletion stops/drains Processes, revokes private Endpoint leases,
  deletes Endpoints, deletes the runtime Volume after producer exit, and only
  then clears `observability-otel.d2bus.org/telemetry-binding`.
- `Endpoint` is implementation transport. A Binding references the Service, never
  an Endpoint, for semantic ingest. No local imported Endpoint can substitute
  for a projected `TelemetryService`.

### Consumed (controller reads or watches)

| ResourceType | Usage |
| --- | --- |
| `Provider` (self) | controller watches own resource for config changes |
| `TelemetryService` | reconcile authority instances; observe core-owned projections without taking import ownership |
| `TelemetryBinding` | reconcile producer intent and write layered status |
| `ResourceExport` / `ResourceImport` | observe export/import lifecycle, projection ownership, revocation, and D091 propagation |
| `Endpoint` | create/delete Binding-private Endpoints; read authority backend/ingest Endpoint readiness |
| `Process` | create/update/delete Binding-owned edge collector and forwarder children |
| `Volume` | create/delete desired runtime socket Volume resources and observe status; `Provider/volume-local` remains the sole filesystem reconciler |
| `Zone` / `Guest` | resolve and watch `producerRef`; derive trusted source identity |
| `Host` | resolve extension `executionRef` and producer placement |

Each `ResourceApiBinding` implements the exact base schema fingerprint, accepts
the canonical minimal valid base, passes common lifecycle/status/finalizer
conformance, and rejects unsupported optional base capability only through the
signed capability matrix and provider-neutral `unsupported-capability`.
`spec.provider` aligns with `status.provider`; the Provider resource keeps the
D075 `{artifactId, config}` exception. Operational state remains status-first
(D087): no Provider state Volume is introduced, and core never links the OTEL
SDK.

---

## Controller

### Lifecycle controller

The observability-otel lifecycle controller is one controller component inside
the `d2b-provider-observability-otel-controller` binary. It runs as a normal
async reconcile loop over d2b-bus/ComponentSession, using the standard toolkit
`ResourceClient` / `Reconciler` API.

**Scope.** Core creates only the controller Process from the signed
`ProviderDeployment` descriptor. The controller reconciles authority
`TelemetryService` resources and producer `TelemetryBinding` resources. Core owns
`ResourceImport` and projected-Service base lifecycle; the Provider import
adapter supplies semantic admission/observation without taking ownership from
core. The controller writes layered Service/Binding status, never Provider status.
The Process and Volume Providers remain the only reconcilers of their effects.

**Controller watch plan:**

```text
watch Provider/observability-otel          # own resource; detect config changes
watch telemetry.d2bus.org.TelemetryService/*
watch telemetry.d2bus.org.TelemetryBinding/*
watch ResourceExport/*, ResourceImport/*   # projection/revocation/currency
watch Endpoint/*                           # authority and Binding-private transport
watch Process/*, Volume/*                  # Binding-owned realization status
watch Zone/*, Guest/*, Host/*              # producer identity and placement
```

**Reconcile flow for an authority `TelemetryService`:**

1. Validate same-Zone generic ingest Endpoint refs, signal/policy bounds, D097
   descriptor, and the strict observability-otel `spec.provider`; require
   exactly one authority index owner.
2. Observe generic ingest readiness and Provider backend readiness separately.
   The Service owns neither a projection route nor a duplicate backend; its
   Provider extension maps the semantic service to the existing SigNoz stack.
3. Admit `ResourceExport` only when `resourceRef` names this Service,
   `endpointRef` names one of its local ingest Endpoints, and
   `exportedType` is `TelemetryService`.
4. Write generic Service observations to `status.resource` and implementation
   observations to `status.provider`. It never copies a locator or telemetry
   payload into status.

**Reconcile flow for a projected `TelemetryService`:**

1. Require `ownerRef: ResourceImport/<name>`, `serviceRole: projection`, matching
   fingerprint, and an active import lease.
2. Reject backend/ingest Endpoint ownership and any authority descriptor.
3. Bind semantic routing through the core-owned import route and signed adapter;
   named streams remain internal transport.
4. On revocation/ZoneLink loss, revoke credits and set the projection Degraded.
   Reconnect revalidates generation/fingerprint before `Ready`.

**Reconcile flow for `TelemetryBinding`:**

1. Resolve `producerRef` and `serviceRef` in the same Zone; validate signal
   subset and clamp requested quota/policy to the Service/import ceiling.
2. Derive and pin the trusted source identity from the observed producer.
3. Create the runtime sockets `Volume`, edge collector `Process`, and private
   collector Endpoints with
   `ownerRef: telemetry.d2bus.org.TelemetryBinding/<name>`. A Guest producer
   additionally causes one forwarder Process and private vsock Endpoint.
4. Observe child readiness and atomically write universal,
   ResourceType-common, and Provider status layers. No live effect precedes the
   D090 `CommittedRevisionProof`.
5. On Service disruption, preserve Binding identity, bound queue occupancy, apply
   credit/backpressure/drop policy, and report Degraded without touching audit.

**Reconcile flow on `deletion-requested`:**

1. Binding: reject new producer sessions; drain bounded queues; stop forwarder then
   collector; wait for Process finalizers; revoke/delete private Endpoints;
   delete the runtime Volume and wait for volume-local; clear Binding finalizer.
2. Authority Service: stop dependent local Bindings; quiesce new imports; let the
   `ResourceExport` finalizer revoke remote leases; release Endpoint refs; clear
   Service finalizer. It never deletes a separately owned backend.
3. Projection Service: the `ResourceImport` finalizer stops local Binding
   consumers, releases the remote lease, deletes the projection, then clears.
4. Provider: require all owned Services/Bindings gone before controller finalizer
   clears. Core commits the Deleted-phase revision and post-commit
   `ResourceMutation` audit record.

**Conformance requirement:** the controller must pass the standard reconciliation
conformance suite from `d2b-provider-toolkit` (optimistic `ResourceMutationBatch`
writes, expected-revision preconditions, bounded requeue backoff, idempotent
reconcile), both qualified ResourceType base suites, import/export adapter
conformance, finalizer ordering, and D090 commit-before-effect.

---

## Volume: telemetry socket directory

The controller creates one desired runtime `Volume` per `TelemetryBinding`;
`Provider/volume-local` alone realizes its layout, ACL, quota, and cleanup. The
Volume is a Binding-owned ephemeral child, not Provider durable state.

### Exact Volume ResourceSpec

```yaml
apiVersion: resources.d2bus.org/v3
type: Volume
metadata:
  name: observability-otel--work--sockets--host-system
  zone: work                       # Zone name resolved at runtime
  ownerRef: telemetry.d2bus.org.TelemetryBinding/work
  finalizers: []
spec:
  providerRef: Provider/volume-local
  source:
    executionRef: Host/host-system  # resolved from Binding producer/extension
    settings:
      kind: tmpfs                   # memory-backed; kernel-enforces quota
      sourcePolicyId: observability-otel-sockets-tmpfs
  kind: tmp                         # process-scoped; cleaned on collector exit
  layout:
    - path: ""                      # Volume root
      type: directory
      ownerRef: User/observability-otel-system
      groupRef: User/observability-otel-system
      mode: "0750"
      accessAcl: []
      defaultAcl:
        - principal:
            ref: User/observability-otel-writers
          permissions: rwx
      foreignChildPolicy: preserve
      noFollow: true
      sensitivity: private
      createPolicy: always-recreate
      repairPolicy: exact-owner
      cleanupPolicy: process-exit-with-proof
      adoptionPolicy: not-adoptable
      restartPolicy: cleanup-after-owner-death
      leaseClass: process-pidfd
      invariants: [no-symlink]
    - path: "emitter.sock"
      type: unix-socket              # collector creates; Volume observes lifecycle
      ownerRef: User/observability-otel-system
      groupRef: User/observability-otel-writers
      mode: "0660"
      sensitivity: private
      createPolicy: observe-only     # collector creates socket at startup
      repairPolicy: none
      cleanupPolicy: process-exit-with-proof
      leaseClass: process-pidfd
    - path: "otlp.sock"
      type: unix-socket
      ownerRef: User/observability-otel-system
      groupRef: User/observability-otel-writers
      mode: "0660"
      sensitivity: private
      createPolicy: observe-only
      repairPolicy: none
      cleanupPolicy: process-exit-with-proof
      leaseClass: process-pidfd
  views:
    collector:
      path: ""
      rights: [read, write, create, delete, traverse]
    forwarder-write:
      path: ""
      rights: [write, traverse]
  attachments: []                    # host-local sockets; no Host/Guest attachment
  quota:
    maxBytes: 10485760              # 10 MiB; kernel-enforced by tmpfs size= option
    maxInodes: 4096
    enforcement: hard               # tmpfs always enforces; required by DRVOL-005
```

`User/observability-otel-system` is the dedicated Zone-local User resource for
the collector process UID. `User/observability-otel-writers` is the Zone-local
group User whose members are all Zone core processes and controllers that emit
telemetry to `emitter.sock`. Both User resources are provisioned by system-core
before the collector is launched.

The mount path `/run/d2b-otel` inside the collector sandbox is a broker FD
delivered in the LaunchTicket, resolved by the volume-local Provider. The
host-absolute path is never exposed in resource `spec`/`status`, audit records,
OTEL spans, or any observable surface. Relative socket names (`emitter.sock`,
`otlp.sock`) are Volume-private. The collector declares no Provider state
Volume; the sockets Volume is the only Volume for this component (see the next
section).
---


## Provider state (ProviderStateSet)

### Concept: optional logical grouping, not a ResourceType

Per `ADR-046-provider-state` §No ProviderState ResourceType, there is no
`ProviderState` ResourceType. A **ProviderStateSet** is the optional,
query-time grouping of the *declared* Volume resources owned by a Provider, and
is empty for a Provider that declares no state Volume:

```text
ProviderStateSet(zone, "observability-otel") =
  { v : Volume | v.metadata.zone == zone
              && v.metadata.ownerRef == "Provider/observability-otel" }
```

`Provider/observability-otel` declares **no** Provider state Volume; its
`ProviderStateSet` is empty. `TelemetryBinding` is a public producer-intent
ResourceType, not a `ProviderState` or Volume alias. Edge collectors carry no
durable data across restarts: telemetry is exported through the same-Zone
Service, and the authority backend owns metrics/traces by construction. Its
bounded non-secret operational state —
collector readiness, export/backpressure reconcile stage, bounded drop/queue
counters, and closed-enum error detail — lives in the owning resource's
`status` subresource and the core Operation ledger (D087).

Because the collector's operational state is fully derivable from spec,
`status`, the core Operation ledger, and its live process memory, it fails the
storage-need test and declares no state namespace, no state Volume, no
state-view mount, and no dedicated state-layout `User/<name>` principal. There
is no empty identity-only Volume.

### Runtime sockets Volume (retained)

The collector's runtime `sockets` Volume (`kind: tmp`) is a genuine runtime
operational Volume that carries the collector's live socket endpoints; it is
retained and is not a Provider state Volume. It holds no durable payload,
carries no `stateSchema` extension, and is not part of the (empty)
ProviderStateSet.

### Invariants

- The observability-otel `ProviderStateSet` is empty; no state Volume exists,
  and none is shared with the runtime `sockets` Volume (`kind: tmp`) or with a
  forwarder worker Process.
- `Volume` is not a Provider-defined ResourceType. The controller may create a
  desired Binding-owned runtime Volume, but never performs Volume effects.
- The controller re-derives collector readiness from live child observation
  into `status.provider` after restart, then derives provider-neutral ingest
  readiness in `status.resource`; it reverifies against the running process and
  treats status as observation, never authority (D087).
---

## Canonical Process templates

The `TelemetryBinding` controller creates edge collector Processes and, for
Guest-scoped Bindings, forwarder Processes. Both use `providerRef:
Provider/system-minijail`. The mount uses the named view declared in the
Binding-owned Volume spec. No executable path, UID/GID, host path, or raw
capability appears in these specs; system-minijail compiles the sandbox.

### otel-collector Process (dynamic; created per TelemetryBinding)

```yaml
apiVersion: resources.d2bus.org/v3
type: Process
metadata:
  name: observability-otel--work--collector
  zone: work
  ownerRef: telemetry.d2bus.org.TelemetryBinding/work
  finalizers: []
spec:
  providerRef: Provider/system-minijail
  executionRef: Host/host-system         # resolved from Binding producer/extension
  domain: system
  userRef: User/observability-otel-system
  processClass: service
  template: otel-collector               # signed template from Provider package
  mounts:
    - volumeRef: Volume/observability-otel--work--sockets--host-system
      view: collector
      mountPath: /run/d2b-otel
      access: read-write
      required: true
  sandbox:
    namespaceClasses: [mount, pid, ipc, uts]
    capabilityClasses: []                # zero capabilities
    seccompClass: strict
    noNewPrivileges: true
    startRoot: false
    readOnlyRoot: true
    environmentClass: minimal
  budget:
    cpu:
      request: "512m"
    memory:
      request: "128Mi"
      limit: "256Mi"
    pids:
      limit: 64
    fds:
      limit: 256
  networkUsage: null
  readiness:
    class: provider-defined              # collector binary reports readiness when
                                         # emitter.sock drain loop is active
  restartPolicy:
    class: on-failure
    backoffBase: "1s"
    backoffMax: "60s"
    backoffMultiplier: 2.0
    maxRestarts: 5
    resetAfter: "600s"
```

### otel-vsock-forwarder Process (dynamic; created per Guest TelemetryBinding)

One instance per admitted Guest-scoped Binding. `<guest-uid-short>` is a stable
opaque short ID derived from the Guest UID; it is not the Guest name.

```yaml
apiVersion: resources.d2bus.org/v3
type: Process
metadata:
  name: observability-otel--fwd-<guest-uid-short>
  zone: work
  ownerRef: telemetry.d2bus.org.TelemetryBinding/dev-vm
  finalizers: []
spec:
  providerRef: Provider/system-minijail
  executionRef: Host/host-system         # same Host as collector
  domain: system
  userRef: User/observability-otel-system
  processClass: worker
  template: otel-vsock-forwarder         # signed template from Provider package
  mounts:
    - volumeRef: Volume/observability-otel--dev-vm--sockets--host-system
      view: forwarder-write
      mountPath: /run/d2b-otel
      access: read-write
      required: true
  sandbox:
    namespaceClasses: [mount, pid, ipc, uts]
    capabilityClasses: []                # zero capabilities; vsock and Unix sockets
                                         # do not require CAP_NET_BIND_SERVICE
    seccompClass: strict
    noNewPrivileges: true
    startRoot: false
    readOnlyRoot: true
    environmentClass: minimal
  budget:
    cpu:
      request: "128m"
    memory:
      request: "32Mi"
      limit: "64Mi"
    pids:
      limit: 16
    fds:
      limit: 64
  networkUsage: null
  readiness:
    class: provider-defined              # forwarder reports readiness on vsock bind
  restartPolicy:
    class: on-failure
    backoffBase: "1s"
    backoffMax: "30s"
    backoffMultiplier: 2.0
    maxRestarts: 10
    resetAfter: "300s"
```

The forwarder worker has **no bus service, no dependency alias authority, and no
controller/CLI authority**. It is a pure worker: receive OTLP frames over vsock
from the Guest, relay to `otlp.sock` in the Volume mount, enforce size bounds
(max 4 MiB per frame), and apply session timeout.

## Endpoint resources (D092)

`Provider/observability-otel` declares standard `Endpoint` base-schema
conformance. Authority generic ingest transport, Provider-extension backend
transport, and Binding-private collector or forwarder transport are owned
`Endpoint` resources with `producerRef`; they are not inline `Process.spec`
fields. An ordinary telemetry producer references
`TelemetryBinding.spec.serviceRef`, never an Endpoint, as its semantic capability.
Only the Service/Binding controller, its exact child Processes, and
`ResourceExport.endpointRef` resolve Endpoints for implementation transport.

Endpoint spec/status/CLI/audit/telemetry never include raw socket paths, CIDs,
ports, IP addresses, fd numbers, OTLP payload bytes, span/log bodies, or
credentials. Resolution occurs only through an authorized
EffectPort/LaunchTicket; unauthorized resolution returns
`endpoint-resolve-denied`. Producer restart bumps
`Endpoint.status.endpointGeneration`, but the stable Service identity remains.

The runtime sockets tmpfs `Volume` remains the backing store for collector
transport files; it is not the stable endpoint identity.

Representative owned Endpoint resources:

```yaml
apiVersion: resources.d2bus.org/v3
type: Endpoint
metadata:
  name: observability-otel-work-bounded-emitter-ingest
  zone: work
  ownerRef: telemetry.d2bus.org.TelemetryBinding/work
spec:
  providerRef: Provider/observability-otel
  producerRef: Process/observability-otel--work--collector
  endpointClass: service
  transport: unix
  purpose: observability-otel.d2bus.org/bounded-emitter-drain
  serviceFingerprint: observability-otel.d2bus.org/bounded-emitter.v3
  locality: host-local
  visibility: authorized-consumers
  attachmentPolicy: component-session
  consumerPolicy: same-zone-authorized
  lifecyclePolicy: recycle-with-producer
status:
  readiness: Ready
  observedProducerGeneration: 1
  observedResourceGeneration: 1
  endpointGeneration: 1
  connectionAvailability: available
  leaseAvailability: lease-required
```

```yaml
apiVersion: resources.d2bus.org/v3
type: Endpoint
metadata:
  name: observability-otel-work-otlp-ingest
  zone: work
  ownerRef: telemetry.d2bus.org.TelemetryBinding/work
spec:
  providerRef: Provider/observability-otel
  producerRef: Process/observability-otel--work--collector
  endpointClass: service
  transport: unix
  purpose: observability-otel.d2bus.org/otlp-grpc-ingest
  serviceFingerprint: observability-otel.d2bus.org/otlp-grpc.v3
  locality: host-local
  visibility: authorized-consumers
  attachmentPolicy: component-session
  consumerPolicy: telemetry-producer-authorized
  lifecyclePolicy: recycle-with-producer
status:
  readiness: Ready
  observedProducerGeneration: 1
  observedResourceGeneration: 1
  endpointGeneration: 1
  connectionAvailability: available
  leaseAvailability: lease-required
```

```yaml
apiVersion: resources.d2bus.org/v3
type: Endpoint
metadata:
  name: observability-otel-vsock-ingest-<guest-uid-short>
  zone: work
  ownerRef: telemetry.d2bus.org.TelemetryBinding/dev-vm
spec:
  providerRef: Provider/observability-otel
  producerRef: Process/observability-otel--fwd-<guest-uid-short>
  endpointClass: service
  transport: vsock
  purpose: observability-otel.d2bus.org/private-guest-ingest
  serviceFingerprint: observability-otel.d2bus.org/guest-otlp-ingest.v3
  locality: cross-domain
  visibility: authorized-consumers
  attachmentPolicy: launch-ticket
  consumerPolicy: same-zone-authorized
  lifecyclePolicy: recycle-with-producer
status:
  readiness: Ready
  observedProducerGeneration: 1
  observedResourceGeneration: 1
  endpointGeneration: 1
  connectionAvailability: available
  leaseAvailability: lease-required
```

## Retained opaque handles

- pidfds: Process supervision handles; not durable endpoint identities.
- Per-connection/session handles: OTLP stream and export attempt handles are
  high-churn flow-control state.
- Named streams: bounded emitter streams carry telemetry records and do not
  identify the stable ingest service. Cross-Zone named streams are private
  implementation transport behind the projected `TelemetryService`.
- `OwnedTransport`: ComponentSession transport ownership remains an in-memory
  authenticated capability.
- fd indexes: collector and forwarder descriptors are LaunchTicket-local slots
  and stay opaque.

---

## Zone startup and bootstrap invariant

`Provider/observability-otel` is an **ordinary optional non-bootstrap Process**.
The bootstrap boundary (per `ADR-046-components-processes-and-sandbox`
§Bootstrap boundary) is closed: Zone runtime, Zone privileged broker, fixed
core-controller / Provider/system-core, Provider/system-minijail controller,
and required transport resources.

`Provider/observability-otel` is never in that boundary. Specifically:

- Zone runtime startup does not wait for `Provider/observability-otel`.
- core-controller startup does not wait for `Provider/observability-otel`.
- Mandatory Providers (`system-systemd`, `system-minijail`) do not wait for it.
- `d2b zone ready` (readiness gate per ADR-046-core-controllers) excludes
  optional Providers from the mandatory readiness set; `observability-otel`
  is optional.
- The ≤64 MiB core aggregate budget (ADR 0046 D008,
  `ADR-046-resource-store-redb`) is unchanged by this Provider.

When `Provider/observability-otel` is **absent (not installed)**:

1. Each core process `BoundedEmitter` writes frames to the `emitter.sock` path.
2. Because the socket file does not exist, the emitter drops write attempts;
   `d2b_telemetry_drop_total` increments.
3. Emitter ring fills to capacity; oldest frames are evicted in FIFO order.
4. Zone health is **not affected**; `telemetry-export-unavailable` condition is
   **not** set by an absent Provider.
5. Authoritative audit is completely unaffected.
6. Doctor reports `telemetry: { "phase": "unavailable" }` (informational only).
7. When the Provider is later installed, the emitter resumes draining in FIFO order.

When the Provider is installed but an authored `TelemetryBinding` edge collector
is not yet `Ready`:

1. Same emitter drop behavior as absent (socket does not yet exist).
2. The controller sets that Binding `Pending` with
   `IngestAvailable=False`; `status.provider.details.collectorReady` is false.
   Core may aggregate the installed Provider `Degraded`, but the Zone readiness
   gate remains unblocked because telemetry is optional.
3. Doctor reports `telemetry: { "phase": "buffering" }`.
4. When the collector and same-Zone Service become `Ready`, the Binding returns
   to `Ready`.

When a collector, Service route, or exporter fails:

1. The affected `TelemetryBinding` and any unavailable projected
   `TelemetryService` become `Degraded`; core may aggregate Provider phase
   `Degraded`.
2. See §Exporter outage for detailed behavior.

---

## Resource status, conditions, and currency

`Provider/observability-otel` uses the **common resource status**. Core derives
and writes the status by aggregating component (Process and Volume) health.
The observability-otel controller does **not** write Provider status.

Per D088, core writes the Provider universal `ResourceStatus` and aggregate
health. The observability controller writes all present status layers
atomically for `TelemetryService` and `TelemetryBinding`. Universal phase,
conditions, timestamps, outcome, and update currency remain top-level;
provider-neutral Service/Binding observations live in `status.resource`; only
implementation-specific bounded non-secret details live in
`status.provider.details`. The strict Provider status schemas are ≤32 KiB,
deny unknown fields, redacted, signed, and versioned. Shared fields are promoted
to `status.resource`, never duplicated into the extension.

The Process Provider writes Process-common observation. The Volume Provider
writes Volume status. Observability reads those statuses and projects bounded
summaries into its owning Binding; it does not impersonate either controller.

**Phase derivation:**

| Resource | `Ready` | `Pending` | `Degraded` / `Failed` |
| --- | --- | --- | --- |
| authority `TelemetryService` | unique authority and generic ingest Endpoints Ready; Provider realization Ready | authority admitted but dependencies not yet Ready | duplicate/ambiguous authority fails closed; ingest or Provider realization outage degrades |
| projection `TelemetryService` | import bound, fingerprint/generation current, route credits available | waiting for import | ZoneLink loss/revocation/stale fingerprint degrades; forbidden Provider realization ownership fails |
| `TelemetryBinding` | producer and Service Ready; Provider realization Ready; quota/source stamp enforced | waiting for Service or Provider realization | ingest outage, queue saturation, or route loss degrades; invalid ownership/schema or exhausted implementation restart may fail |

The closed conditions are those declared in §ResourceTypes. Condition reasons
are provider-neutral closed enums (`Starting`, `Ready`,
`DependencyUnavailable`, `SchemaMismatch`, `ImportRevoked`, `QueueFull`,
`IngestUnavailable`, `SourceStampFailed`, `DuplicateAuthority`, `Draining`),
with bounded non-secret outcome detail. Implementation-only `BatchTimeout` and
`ExporterOutage` codes occur only in `status.provider.details`.

D091 currency and upgrade: the observability-otel controller implements
`assess_update`, `plan_upgrade`, and `execute_upgrade`. A
`ProviderGenerationChanged`, collector `ArtifactChanged`, service/import
`DependencyChanged`, or `SpecChanged` reason populates universal `status.update` with
`UpdateAvailable` or `UpgradeRequired`; disruptive changes MUST return
`UpgradeRequired` rather than being applied in place, while
non-disruptive changes reconcile normally. These currency fields are
universal/ResourceType base fields, never `status.provider`. Authority Service
upgrade drains admitted producer Bindings before recycling its backend/ingest
realization. D091 propagates authority → export → import → projected Service →
TelemetryBinding → owned children. A Binding upgrade preserves its UID/spec and
recycles only its collector/forwarder/private Endpoints/ephemeral Volume.
Endpoint generations may change without replacing Service identity. Telemetry
is best-effort and lossy; no payload, secret, path, credential, locator, or
handle appears in `status.update`.

D090 expedited reconcile: Create, UpdateSpec, and Delete requests that set
`waitForReconcile` perform no external effect, finalizer mutation, or status
mutation until Core supplies a typed `CommittedRevisionProof`
`{resourceUid,generation,revision,operationId}`. Abort produces no effect; a
durable commit is never rolled back on later reconcile timeout. The response is
the committed object plus one-pass projected layered status, a disposition
(`Converged`, `Progressing`, `Blocked`, `UpgradeRequired`, or `Failed`), and
`statusPersistence` (`pending` or `committed`); effect idempotency keys derive
from `(UID,generation,revision,operationId)` in the same per-resource
single-flight priority lane.

**Status-first state.** Collector readiness, route/import stage, bounded
queue/drop counters, source-stamping result, and closed error classes live in
Service/Binding status and the core Operation ledger. No parallel state file or
durable Provider state Volume is permitted. Status is observation, never repair
authority; restart re-observes Process, Endpoint, Volume, import generation, and
authority owner proof before reporting Ready.

**No Provider-owned Zone health writes.** `Provider/observability-otel` does not
write Zone health or set Zone-level conditions directly. Core derives Zone health
from Provider phase per standard rules.

**No `no_isolation` field in any status.** `no_isolation` is exclusively an
audit record field for user-only Host `ProcessEffect` records. It does not appear
in Provider status, metrics, or OTEL spans (per
`ADR-046-telemetry-audit-and-support` §Host resource status).
---

## Metrics, traces, and redaction invariants

### OTEL resource attributes stamped by the collector process

The collector stamps these resource attributes on all outgoing OTLP data:

| Attribute | Value | Source |
| --- | --- | --- |
| `service.name` | `d2b-provider-observability-otel` | compile-time constant |
| `service.version` | `CARGO_PKG_VERSION` | compile-time constant |
| `d2b.zone` | Zone name string | Zone self-resource `metadata.name` |
| `d2b.provider` | `observability-otel` | closed Provider name catalog |
| `d2b.component` | `collector` or `vsock-forwarder` | signed component descriptor |

Before export, the collector derives source identity from the Binding's observed
same-Zone `producerRef` and upserts the protected `d2b.zone`, `vm.name`,
`vm.env`, `vm.role`, `host.name`, and `source` fields. Self-asserted incoming
values for those keys are ignored; other allowed
`deployment.environment`/`service.namespace` values are preserved. Projection
and authority routing cannot restamp one producer as another. The closed
allowlist from `ADR-046-telemetry-audit-and-support` governs what may appear.

### Attribute allowlist enforcement

The collector rejects any incoming frame whose OTEL resource attributes contain
keys not in the v3 closed allowlist:

```text
deployment.environment, host.name, service.name, service.namespace,
source, vm.env, vm.name, vm.role,
d2b.zone, d2b.provider, d2b.component, service.version
```

Frames with extra keys are **dropped** (not forwarded); `d2b_otel_frames_decoded_total{outcome="error"}` increments.

### Forbidden metric label values

All of the following are unconditionally forbidden as metric label values in
any data processed or forwarded by this Provider (per
`ADR-046-telemetry-audit-and-support` §Cardinality rules):

- VM names, Zone names, Provider names, resource `metadata.name` values
- Zone/Provider/Process UIDs
- Host/Guest/User/Volume/Network/Device names
- Filesystem paths, socket paths, executable paths
- argv or environment values
- Status detail messages or outcome text beyond stable error codes
- Subject names or principal identifiers
- PID, pidfd, or cgroup path values
- Operation IDs or correlation IDs
- Endpoint addresses, port numbers, or IP addresses

### no_isolation invariant

`no_isolation` is exclusively an audit record field for user-only Host
`ProcessEffect` records. It must not appear as:
- A metric label key or value
- A span attribute key
- A log field name
- Any OTEL data emitted or forwarded by this Provider

This invariant applies to data originating from this Provider's own processes
and to ingested data from the emitter socket and OTLP socket. The redaction
filter (`src/redaction.rs`) drops any span attribute, log body field, or metric
exemplar field named `no_isolation` from forwarded data.

---

## Traces

The collector and forwarder emit the following spans using `d2b-telemetry`
`BoundedEmitter` (not the OTEL SDK directly, for consistency with other d2b
processes):

| Span name | Kind | Attributes | Notes |
| --- | --- | --- | --- |
| `d2b.otel.collector.drain` | Internal | `signal`, `frame_count`, `outcome` | Per drain cycle |
| `d2b.otel.collector.export` | Client | `signal`, `batch_size`, `outcome` | Per OTLP export call |
| `d2b.otel.collector.startup` | Internal | `outcome`, `socket_ready` | Provider startup |
| `d2b.otel.forwarder.session` | Internal | `outcome`, `guest_uid_digest` | Per vsock session; no guest name |
| `d2b.otel.journald.cycle` | Internal | `records_total`, `redacted`, `dropped`, `outcome` | Journald batch |

Forbidden span attributes: path, socket path, endpoint address, argv, pid,
resource name, guest name, user name, zone name. `d2b.zone` is allowed in
resource attributes only.

---

## Audit

The observability-otel Provider does not emit any new audit record classes
beyond those defined in `ADR-046-telemetry-audit-and-support`. Process lifecycle
generates standard `ProcessEffect`; Service/Binding/Export/Import mutations
generate `ResourceMutation`; import admission and route lease changes generate
the standard `RouteAdmission` records. Audit includes bounded ResourceRefs,
operation/outcome, and closed reason codes, never telemetry payloads, source
attributes, quotas consumed, locators, credentials, or stream handles.

The observability-otel Provider **never** reads from the authoritative audit
sink. OTEL telemetry and audit are strictly separated subsystems; audit data
never enters the OTEL pipeline. A `TelemetryService` export transfers ingest
capability only and cannot transfer audit authority. Exporting allow-listed
audit copies is a separate explicit export, and the producer Zone remains the
system of record.

---

## Exporter outage and backpressure behavior

### Exporter outage

When the OTLP export call to the backend fails at the OTEL SDK level (transport
has already delivered or raised an error to the SDK):

1. The OTEL SDK export call returns an error.
2. `d2b_otel_export_batch_total{outcome="error"}` increments.
3. `d2b_otel_exporter_failures_consecutive` gauge increments.
4. The SDK applies bounded retry from the Binding Provider extension while the
   Binding and Service quotas remain authoritative.
5. If the SDK exhausts retries, the batch is dropped:
   `d2b_otel_export_batch_total{outcome="dropped"}` increments;
   `d2b_telemetry_drop_total{reason="export_error"}` increments.
6. If consecutive failures reach the Binding extension's `failureThreshold`:
   - Controller writes provider-neutral `IngestAvailable=False`
     (`IngestUnavailable`) on that Binding and
     `status.provider.details.errorCode: ExporterOutage`.
   - The Binding becomes `Degraded`; Service remains independently observed.
   - `d2b zone doctor` reports `telemetry: { "phase": "unavailable" }`.
7. On next successful export: `d2b_otel_exporter_failures_consecutive` resets
   to 0; controller clears the Binding condition and returns it to `Ready` when
   all other conditions are satisfied.

Zone mutations, reconciliation, process launch, and authoritative audit are
unaffected throughout.

### Backpressure

When the OTLP SDK queue reaches the lesser of the Binding's common quota and the
extension `maxQueueSize`:

1. `d2b_otel_backpressure_active` gauge is set to 1.
2. New incoming frames from `emitter.sock` that cannot be enqueued are dropped
   from the emitter ring in FIFO order.
3. `d2b_telemetry_drop_total{reason="buffer_full"}` increments per dropped frame.
4. Controller sets `BackpressureActive=True` (`QueueFull`) on the Binding; import
   credit exhaustion also sets `QuotaSaturated=True` on the projected Service.
5. The affected Binding becomes `Degraded` (not `Failed`).
6. When queue depth drops below `maxQueueSize * 0.75` (75% watermark):
   - `d2b_otel_backpressure_active` is reset to 0.
   - Controller clears the Binding/Service backpressure conditions.
   - The Binding returns to `Ready` if no other degraded condition remains.

The emitter socket drain loop continues accepting frames during backpressure; it
simply discards frames that cannot be enqueued rather than blocking.

---

## Lifecycle, drain, and restart

### Startup sequence

1. Core installs the Provider and launches only its controller Process from the
   signed `ProviderDeployment`.
2. The controller observes an authority or projected `TelemetryService`.
3. For each committed `TelemetryBinding`, it validates the same-Zone
   `producerRef`/`serviceRef`, signal subset, quotas, policy, and source stamp.
4. The controller creates the Binding-owned runtime Volume, private Endpoints,
   edge collector Process, and Guest forwarder when required.
5. ProviderSupervisor launches the children. The collector opens its private
   sockets, initializes the OTEL SDK, and resolves the Service route.
6. For an imported Service, core/import adapter opens the per-import bounded
   encrypted named stream only after fingerprint/generation/lease validation.
7. Collector begins the drain loop and reports readiness; controller writes
   Binding status from observed child and Service status.

Before step 7, `BoundedEmitter` writes may drop into their bounded rings.
Zone startup and authoritative audit are unaffected.

### Drain and graceful stop

On ordered stop (SIGTERM received by the collector binary):

1. Collector enters draining phase; emits `collectorPhase=draining` component health event.
2. Accept no new writes on `emitter.sock` (close the socket; new writes from
   core processes will fail and increment `d2b_telemetry_drop_total`).
3. Drain the remaining emitter ring contents; encode and enqueue for OTLP export.
4. Flush pending SDK export batches up to the strict Binding extension timeout.
5. After flush completes or timeout expires, exit cleanly.
6. The owning Process controller receives exit; sets Process `Succeeded` or
   `Failed` based on exit code.

Drain timeout is enforced by the ProviderSupervisor via a SIGKILL if the
collector does not exit within the bounded Binding extension drain timeout.

### Restart

On unexpected exit:

1. Process controller detects exit via pidfd wait notification.
2. `ProcessEffect{event: "stop", exit_class: "exited|signaled|killed"}` audit
   record is emitted.
3. `d2b_process_restart_total{provider="observability-otel"}` increments.
4. Process controller applies bounded exponential backoff restart; max 5 restarts
   in 10 min (per restart spec in the Process template).
5. Emitter ring continues accumulating frames during restart; controller marks
   the owning `TelemetryBinding` `Degraded`.
6. After max restarts, Process phase becomes `Failed` and the Binding becomes
   `Failed`; optional telemetry still does not block Zone bootstrap.

### Zone stop

On Zone-level shutdown (all Processes stopped):

1. Core requests deletion of producer Bindings before Services/Provider.
2. Each Binding stops forwarder first, drains collector, revokes/deletes private
   Endpoints, and deletes the runtime Volume.
3. Imports release routes and delete projected Services; authority export
   revocation completes before authority Service teardown.

---

## d2b-bus and RBAC

### Bus service registration

The observability-otel controller registers no public bus service. It consumes
the standard resource API services for its reconciliation loop (get/list/watch/
mutate via `ResourceClient`).

The collector binary registers one internal bus service:

| Service | Method | Bus direction | Purpose |
| --- | --- | --- | --- |
| `d2b.observability.v1.SelfMetrics` | `GetMetrics` | local | Read self-metrics; consumed by `d2b zone doctor` via authenticated d2b-bus call |

The SelfMetrics service is `local` direction only; it is not routable through
ZoneLink or cross-Zone paths.

### Dependency aliases

Per `ADR-046-provider-model-and-packaging` §Dependency aliases, the
observability-otel component descriptor declares:

| Alias | Bound to | Purpose |
| --- | --- | --- |
| `telemetry-service` | Binding's same-Zone `TelemetryService` | semantic ingest capability; authority or imported projection |
| `process` | `Provider/system-minijail` | Binding-owned edge collector/forwarder realization |
| `volume` | `Provider/volume-local` | Binding-owned ephemeral socket Volume realization |

These are internal component dependency handles, not ResourceType aliases or
compatibility names. The two telemetry ResourceTypes have no aliases.

The `credential` alias is absent. Cross-Zone transport and authentication belong
to core ZoneLink/ResourceImport routing; authority backend transport/auth belong
behind the Service's local Endpoint implementation. This Provider never
acquires or routes Credential bytes.

### Required RBAC verbs

The observability-otel component descriptor declares these minimum permission
claims:

| ResourceType | Verb | Purpose |
| --- | --- | --- |
| `Provider` (self) | `get`, `watch` | controller reads own spec for config changes |
| `telemetry.d2bus.org.TelemetryService` | `get`, `list`, `watch`, `update-status`, `finalize` | reconcile authority; observe/write semantic status for core-owned projections |
| `telemetry.d2bus.org.TelemetryBinding` | `get`, `list`, `watch`, `update-status`, `finalize` | reconcile producer intent and child-first deletion |
| `ResourceExport`, `ResourceImport` | `get`, `list`, `watch` | observe export/import binding, revocation, projection, and update currency |
| `Process` | `get`, `list`, `watch`, `create`, `update`, `delete` | manage Binding-owned collector/forwarder desired resources |
| `Endpoint` | `get`, `list`, `watch`, `create`, `update`, `delete` | observe Service endpoints and manage Binding-private endpoints |
| `Volume` | `get`, `list`, `watch`, `create`, `delete` | manage Binding-owned desired runtime Volume; no Volume effects |
| `Zone`, `Guest`, `Host` | `get`, `list`, `watch` | resolve producer identity and placement |

The controller cannot create/update/delete `ResourceExport`, `ResourceImport`,
or core-owned projected Services. It has no cross-Zone ResourceRef authority and
no `Credential`, `Role`, `RoleBinding`, `User`, `Network`, or `Device` verbs.
No Provider-self status write is claimed; core writes Provider status.

### Security properties

- The collector process runs as a dedicated UID (`d2b-<zone>-otel`), distinct
  from Zone runtime, core-controller, and all other Provider UIDs.
- No ambient host network access; local Endpoints and the authorized
  ResourceImport bounded encrypted named stream are the only transports.
- No read access to `/nix/store`, host paths, other VM/Zone state directories,
  or broker sockets.
- Sandbox: `system-minijail` or `system-systemd` enforcement; `noNewPrivileges: true`;
  `capabilityClasses: []` (zero capabilities). Neither the collector nor the
  forwarder requires `CAP_NET_BIND_SERVICE`; they use Unix/vsock sockets bound
  only to Volume-private paths and Zone-allocated vsock ports.
- Credential bytes are never held, routed, or processed by this Provider; all
  auth material stays behind the authority Endpoint or core ZoneLink session.
- Redaction filter (`src/redaction.rs`) is applied to all forwarded data before
  export; it runs before the OTLP SDK batching step.

---

## Observability authority and cross-Zone sharing (D096/D097)

**Initial SigNoz ingest authority (D097).** The observability Zone (`sys-obs`) owns
the native SigNoz + ClickHouse + ClickHouse Keeper + SigNoz OTel Collector
stack (`nixos-modules/components/observability/stack.nix`; no container
runtime), with storage/query bound to loopback. One authority
`telemetry.d2bus.org.TelemetryService/telemetry` represents that whole
semantic telemetry-ingest service and references its same-Zone ingest Endpoint.
Its `spec.provider` selects SigNoz, the OTLP protocol, and same-Zone backend
Endpoints. The
`TelemetryService`, not an Endpoint, carries the D097 `AuthorityDescriptor`:
`authorityScope: external-service`, opaque authority-key class (never an
address/port), `cardinality: exactly-one`, `arbitration: multiplexed`, and
`exportability: explicit-export`. Core rejects a second authority with
`duplicateConflict`; restart adopts by owner proof; ambiguity quarantines.

**Preserved reusable behavior** (grounded in
`nixos-modules/components/observability/{stack,host,guest}.nix`,
`nixos-modules/{index,observability-vm}.nix`,
`packages/d2b-host/src/{otel_host_bridge_argv,vsock_relay_argv}.rs`,
`packages/d2bd/src/otel_host_bridge_readiness.rs`): distinct **per-source** vsock
ingress (the bridge uses pre-opened vsock fds only and the broker rejects any
bundle intent whose source VM ≠ obs VM); trusted **source-identity upsert** of
`vm.name`/`vm.env`/`vm.role`/`source` OTEL resource attributes; the loopback
backend; edge collector **retry/queue** (`sending_queue.enabled`,
`retry_on_failure.enabled`); bounded metric/cardinality caps and the closed
attribute **allow-list** redaction; and audit separation by **positive
allow-list projection only** (e.g. `source: store-sync-audit`).

**Cross-Zone sharing (D096).** The owner Zone's `ResourceExport` has
`resourceRef: telemetry.d2bus.org.TelemetryService/telemetry`,
`endpointRef` equal to one referenced
local ingest Endpoint, and
`exportedType: telemetry.d2bus.org.TelemetryService`. The Endpoint is only the
export adapter's transport front door. Every producer Zone declares a
`ResourceImport` expecting/projecting
`telemetry.d2bus.org.TelemetryService`; core creates
`telemetry.d2bus.org.TelemetryService/<projection>` with
`ownerRef: ResourceImport/<name>`.
`TelemetryBinding.spec.serviceRef` names that same-Zone projection.

The projection has no `spec.provider`, backend/ingest Endpoint ownership,
authority descriptor, or durable state. Core/import adapter routes it to the
authority over per-import bounded encrypted named streams with exact identity,
quota, credits, backpressure, session generation, deadline, and cancel.
Intermediaries see ciphertext; no FD/socket/resource grant crosses a Zone.
Authority and Binding controllers enforce the signal schema, trusted
source-identity stamp, quota, redaction, and cardinality caps. Export removal or
ZoneLink loss revokes credits/lease and degrades the projected Service and its
Bindings. Reconnect revalidates generation/fingerprint. A D091 upgrade propagates
authority → export → import → projection → Binding and drains producers before
recycling authority realization.

**Service/Binding separation.** `TelemetryService` answers “where is the stable
ingest capability?”; `TelemetryBinding` answers “what does this producer emit and
what bounded state is observed in status?”. A Binding is never exported. An Endpoint or
named stream answers only “how is this realization transported?” and can never
be used as the imported semantic projection.

**No audit-authority transfer.** Authoritative **audit** stays Zone-local — the
local Zone remains the system of record. Exporting audit *copies* requires a
separate, explicit `ResourceExport` and transfers **no** authority; the positive
allow-list projection is the only path and no import can promote a copy to
authority.

**No legacy shortcuts.** The bridge/forwarder use D077 EffectPort/LaunchTicket
pre-opened fds (no ambient socket creation, no direct broker path), observed
state is D087 status-first, transport endpoints are D092 `Endpoint`s, and
cross-Zone semantic routing is the D096 projected `TelemetryService`. Core has
no OTEL SDK dependency.

**Explicit gaps (conservative; refined by evidence before final commit).** These
are recorded as gaps in the current baseline and are NOT claimed as implemented:

- The `TelemetryService`/`TelemetryBinding` resources and signed export/import
  adapter are net-new work grounded in the current bridge/stack.
- **Retention/sampling options are not actually wired** in the current baseline;
  the dossier's retention/sampling knobs are target design, gated behind the
  sink Provider work item, not existing behavior.
- **No formal runner `sd_notify` readiness** exists; readiness today is
  `packages/d2bd/src/otel_host_bridge_readiness.rs` probing, and a formal
  notify-ready runner contract is target work.

## Nix configuration

### Provider artifact catalog registration

```nix
d2b.artifacts.provider-observability-otel = {
  package = inputs.d2b.packages.${system}.provider-observability-otel;
  type = "provider";
};
```

### Zone resource authoring

```nix
# Install the semantic Provider in authority and producer Zones.
d2b.zones.sys-obs.resources.observability-otel = {
  type = "Provider";
  spec = {
    artifactId = "provider-observability-otel";
    config.selfMetrics.enable = true;
  };
};
d2b.zones.work.resources.observability-otel = {
  type = "Provider";
  spec = {
    artifactId = "provider-observability-otel";
    config.selfMetrics.enable = true;
  };
};

# Owner Zone: the provider-neutral semantic ingest service.
d2b.zones.sys-obs.resources.telemetry = {
  type = "telemetry.d2bus.org.TelemetryService";
  spec = {
    providerRef = "Provider/observability-otel";
    serviceRole = "authority";
    ingestEndpointRefs = [ "Endpoint/telemetry-ingest" ];
    signals = [ "metrics" "traces" "logs" ];
    quota = {
      maxProducers = 64;
      bytesPerSecond = 16777216;
      burstBytes = 33554432;
      maxInFlightBytes = 67108864;
      maxStreamsPerProducer = 4;
    };
    policy = {
      backpressure = "drop-oldest";
      disconnect = "bounded-buffer-then-drop";
      redactionProfile = "d2b-closed-v3";
      cardinalityProfile = "d2b-bounded-v3";
      sourceIdentity = "trusted-producer-ref";
    };
    authorityDescriptor = {
      authorityScope = "external-service";
      authorityClass = "telemetry-ingest";
      authorityKey = "configured-telemetry-authority";
      cardinality = "exactly-one";
      arbitration = "multiplexed";
      exportability = "explicit-export";
    };
    updatePolicy.mode = "manual-disruptive";
    provider = {
      schemaId = "observability-otel.d2bus.org/TelemetryService/spec";
      schemaVersion = "1.0";
      settings = {
        backend = "signoz";
        backendEndpointRefs = [ "Endpoint/signoz-query-backend" ];
        ingestProtocol = "otlp-grpc";
      };
    };
  };
};

d2b.zones.sys-obs.resources.telemetry-export = {
  type = "ResourceExport";
  spec = {
    providerRef = "Provider/observability-otel";
    resourceRef = "telemetry.d2bus.org.TelemetryService/telemetry";
    endpointRef = "Endpoint/telemetry-ingest";
    exportedType = "telemetry.d2bus.org.TelemetryService";
    baseSchemaFingerprint = "sha256:<telemetry-service-base>";
    operations = [ "ingest-metrics" "ingest-traces" "ingest-logs" ];
    arbitration = "multiplexed";
    quota = { maxConsumers = 64; fairness = "weighted"; leaseDeadlineMs = 30000; };
    consumerZonePolicy = {
      zones = [ "Zone/work" ];
      capabilityCeiling = [ "ingest-metrics" "ingest-traces" "ingest-logs" ];
    };
    visibility = "named-zones";
    updatePolicy.mode = "manual-disruptive";
    revocationPolicy.graceMs = 5000;
  };
};

# Producer Zone: import the Service. Core creates the qualified local Service
# with ownerRef ResourceImport/telemetry; Nix does not author an Endpoint projection.
d2b.zones.work.resources.telemetry = {
  type = "ResourceImport";
  spec = {
    providerRef = "Provider/observability-otel";
    zoneLinkRef = "ZoneLink/work-uplink";
    exportKey = "sys-obs/telemetry-export";
    expectedType = "telemetry.d2bus.org.TelemetryService";
    expectedBaseSchemaFingerprint = "sha256:<telemetry-service-base>";
    projectionName = "telemetry";
    projectionType = "telemetry.d2bus.org.TelemetryService";
    requestedCapabilities = [ "ingest-metrics" "ingest-traces" "ingest-logs" ];
    requestedQuota = { bytesPerSecond = 4194304; maxInFlightBytes = 16777216; };
    updatePolicy.mode = "manual-disruptive";
    disconnectPolicy.mode = "degrade";
  };
};

# Producer intent references the same-Zone projected Service.
d2b.zones.work.resources.zone-telemetry = {
  type = "telemetry.d2bus.org.TelemetryBinding";
  spec = {
    providerRef = "Provider/observability-otel";
    serviceRef = "telemetry.d2bus.org.TelemetryService/telemetry";
    producerRef = "Zone/work";
    signals = [ "metrics" "traces" "logs" ];
    quota = {
      bytesPerSecond = 4194304;
      burstBytes = 8388608;
      maxQueueBytes = 4194304;
      maxInFlightBytes = 16777216;
      maxStreams = 3;
      dropBudgetPerMinute = 10000;
    };
    policy = {
      backpressure = "drop-oldest";
      disconnect = "bounded-buffer-then-drop";
      redactionProfile = "d2b-closed-v3";
      cardinalityProfile = "d2b-bounded-v3";
      sourceIdentity = "trusted-producer-ref";
    };
    updatePolicy.mode = "manual-disruptive";
    provider = {
      schemaId = "observability-otel.d2bus.org/TelemetryBinding/spec";
      schemaVersion = "1.0";
      settings = {
        executionRef = "Host/host-system";
        emitterRingBytes = {
          logs = 2097152;
          metrics = 4194304;
          traces = 4194304;
        };
        otlpExporter = {
          batchExportTimeoutMs = 30000;
          batchMaxExportSize = 512;
          batchScheduleDelayMs = 5000;
          compressionEnabled = true;
          failureThreshold = 5;
          maxQueueSize = 2048;
        };
        journald.enable = false;
      };
    };
  };
};
```

### Canonical ResourceSpec JSON shapes (Nix bundle output)

The Nix bundle contains the authored authority Service, Export, Import, and
Binding. The core-created projection is not configuration-owned. Representative
canonical shapes:

```json
{
  "apiVersion": "resources.d2bus.org/v3",
  "metadata": { "name": "telemetry", "zone": "sys-obs" },
  "spec": {
    "authorityDescriptor": {
      "arbitration": "multiplexed",
      "authorityClass": "telemetry-ingest",
      "authorityKey": "configured-telemetry-authority",
      "authorityScope": "external-service",
      "cardinality": "exactly-one",
      "exportability": "explicit-export"
    },
    "ingestEndpointRefs": ["Endpoint/telemetry-ingest"],
    "provider": {
      "schemaId": "observability-otel.d2bus.org/TelemetryService/spec",
      "schemaVersion": "1.0",
      "settings": {
        "backend": "signoz",
        "backendEndpointRefs": ["Endpoint/signoz-query-backend"],
        "ingestProtocol": "otlp-grpc"
      }
    },
    "providerRef": "Provider/observability-otel",
    "serviceRole": "authority",
    "signals": ["logs", "metrics", "traces"]
  },
  "type": "telemetry.d2bus.org.TelemetryService"
}
```

```json
{
  "apiVersion": "resources.d2bus.org/v3",
  "metadata": {
    "managedBy": "core",
    "name": "telemetry",
    "ownerRef": "ResourceImport/telemetry",
    "zone": "work"
  },
  "spec": {
    "providerRef": "Provider/observability-otel",
    "serviceRole": "projection",
    "signals": ["logs", "metrics", "traces"]
  },
  "type": "telemetry.d2bus.org.TelemetryService"
}
```

The projection has no `spec.provider` or backend ownership. The Binding
separately carries `serviceRef`, `producerRef`, signals, quotas/policy, and its
strict Provider extension. No shape contains a raw address, socket, CID, port,
stream handle, credential, or secret.

### NixOS eval and build validation

**Eval-time assertions** (via generated option types from Provider schema):

1. Both provider-neutral qualified ResourceTypes resolve to canonical signed
   base schemas and this installed Provider's strict extension schemas; unknown
   base or extension fields fail.
2. Authority Service ingest Endpoint refs resolve locally, its D097 descriptor
   is exact, and duplicate external-service authority keys fail eval. SigNoz,
   OTLP, and OTEL fields outside its strict `spec.provider` fail.
3. Projection Services cannot be authored directly, carry ingest refs,
   `spec.provider`, or an authority descriptor; only core may create one
   under a `ResourceImport`.
4. `ResourceExport.resourceRef`, `exportedType`, and `endpointRef` form the
   Service → local ingest Endpoint chain. Exporting a Binding or Endpoint as the
   semantic telemetry type fails.
5. `ResourceImport.expectedType` and `projectionType` are both
   exactly `telemetry.d2bus.org.TelemetryService`; an Endpoint projection fails.
6. Binding `serviceRef` and `producerRef` resolve in the same Zone; signals are a
   non-empty Service subset; quotas/policy satisfy all bounds.
7. The strict Service Provider extension validates backend and protocol
   settings; the strict Binding Provider extension validates `executionRef` and
   numeric implementation settings. Neither can shadow common fields.
8. Any raw address, socket path, CID, port, FD, stream handle, Credential ref,
   or secret-like value fails eval.
9. Provider root config accepts only `selfMetrics.enable`; per-resource backend,
   OTLP, ring, batching, and journald fields fail there as misplaced.
10. `artifactId` resolves to a Provider artifact; duplicate resource names fail.

**Build-time validation** (`nixos-modules/resources-bundle.nix`):

1. Provider config, both provider-neutral base spec/status layers, and all
   observability-otel `spec.provider`/`status.provider` layers validate against
   signed schemas and fingerprints.
2. SHA-256 digest computed for the `spec` object (canonical sorted JSON bytes).
3. Generation digest computed as SHA-256 of sorted per-resource digest list.
4. Bundle emitted as `zone-resources-<zone>.json`; store path is the integrity pin.
5. No raw secret bytes, host paths, argv tokens, or UID strings in any
   `spec` leaf (pattern-checked against the forbidden-field set from
   `packages/d2b-contract-tests/tests/policy_observability.rs`).

**Runtime activation** (core-controller configuration publication handler):

1. Re-validates Provider package identity against installed package.
2. Validates exactly one authority Service owner proof and local Endpoint refs.
3. Validates Export → authority Service → ingest Endpoint and Import →
   projected Service fingerprints/capabilities.
4. Resolves each Binding's same-Zone Service/producer and clamps quota to the
   admitted Service/import ceiling.
5. Validates Provider package/schema generation and extension digest.
6. On failure, rejects the generation before effects with a bounded
   `generation-rejected` audit record.

---

## Tests

All test files must be present. Workspace policy rejects a missing `tests/` or
`integration/` directory.

### Fast hermetic execution and test placement (D094)

Per D094 and `ADR-046-validation-and-delivery` §10.16, this Provider's `src/`
unit tests and `tests/*.rs` hermetic suite are fast, in-process, deterministic,
and parallel-safe: an individual normal test has p95 ≤50 ms with no wall-clock
sleep, and `cargo test -p d2b-provider-observability-otel --lib --tests`
completes in ≤2 s warm-cache execution time (compilation excluded). They use a
deterministic fake clock/RNG and the toolkit fakes/FakeEffectPort only — no
process spawn, container, network, DBus, systemd, broker daemon, Nix eval/build,
KVM, USB/GPU/TPM hardware, or live cloud, and no filesystem tree beyond tiny
temp fixtures. Any scenario needing those lives only in `integration/`, which
keeps a lane timeout/budget, parallel isolation, and fake external services by
default; such a need is re-placed into `integration/`, never given a sleep,
larger timeout, or `#[ignore]`. Bounded crypto/property tests are the only
classified exception, each named with a capped case count and a declared higher
per-test budget.

Service/Binding schema, ownership, and projection-chain tests use only in-memory
resource fixtures and fake adapters, so they remain fast hermetic tests. Any
test that launches a real SigNoz/ClickHouse/Keeper process, opens a real vsock
or ZoneLink/ComponentSession stream, or performs stream encryption/rendezvous is
integration-only.

### Unit and hermetic Cargo tests (`tests/`)

#### `tests/resource_service_binding.rs`

- Accept one authority `telemetry.d2bus.org.TelemetryService` with local generic
  ingest Endpoints, the exact D097 external-service descriptor, and a strict
  observability-otel Service Provider extension selecting SigNoz/OTLP.
- Reject a second authority, an authority without local Endpoint refs, and a
  projection carrying `spec.provider`, backend ownership, or an authority
  descriptor.
- Accept `telemetry.d2bus.org.TelemetryBinding` with same-Zone
  `serviceRef`/`producerRef`, signal subset, common quota/policy, and strict
  Provider extension.
- Reject exporting a Binding, using an Endpoint as `serviceRef`, moving common
  fields into the extension, putting OTLP/SigNoz/OTEL fields in either base
  spec, unknown extension fields, or quota overflow.
- Reject the old provider-qualified ResourceType names and every alias.
- Assert Binding-owned collector/forwarder/Volume/private Endpoints have
  `ownerRef: telemetry.d2bus.org.TelemetryBinding/<name>` and finalizers
  delete child-first.

#### `tests/projection_chain.rs`

- In memory, build authority Service → `ResourceExport.resourceRef`, local
  ingest Endpoint → `ResourceExport.endpointRef`, `ResourceImport` → core-owned
  projected Service → producer Binding `serviceRef`.
- Assert Export/Import expected/projected type is exactly
  `telemetry.d2bus.org.TelemetryService`, never `Endpoint`, and projection owner
  is exactly `ResourceImport/<name>`.
- Assert the projection has no `spec.provider`, backend/ingest Endpoint
  ownership, or authority descriptor, while a fake import route reaches the
  authority.
- Revoke the export and lose the fake ZoneLink; assert projection and Binding
  become Degraded, credits are revoked, and reconnect requires matching
  generation/fingerprint.
- Assert source identity is upserted from `producerRef`, quotas/backpressure are
  applied at Binding/import/authority, intermediaries receive only fake
  ciphertext, and audit authority remains local.

#### `tests/emitter_socket_receive.rs`

**Source:** new; adapted from emitter socket ACL pattern in
`nixos-modules/components/observability/host.nix`.

- Drive the emitter drain loop with a mock datagram socket.
- Write 100 frames covering metric, trace, and log signal types.
- Assert all frames arrive at the mock OTLP sink.
- Assert every forwarded span carries `d2b.zone` resource attribute.
- Assert no forwarded span attribute or metric label carries a resource name
  (`vm`, `zone`, `provider` as label key).
- Assert `d2b_otel_frames_received_total` increments correctly per signal type.

#### `tests/emitter_ring_drains_on_socket_available.rs`

**Source:** specified in `ADR-046-telemetry-audit-and-support` §OTEL endpoint tests.

- Start emitter; write 50 frames before socket exists (socket path absent).
- Assert `d2b_telemetry_drop_total` increments for each dropped write.
- Create the socket; start drain loop.
- Assert the 50 buffered frames arrive at the mock OTLP sink in FIFO order.
- Assert `d2b_otel_frames_received_total` reaches 50.

#### `tests/emitter_ring_drop_on_overflow.rs`

**Source:** specified in `ADR-046-telemetry-audit-and-support` §OTEL endpoint tests.

- Configure ring capacity to 10 frames (test override).
- Write 20 frames; assert `d2b_telemetry_drop_total` reaches exactly 10.
- Assert the owning Binding transitions to `Degraded` (not `Failed`).
- Assert Zone startup fixture is not blocked.
- Assert oldest 10 frames arrive at mock OTLP sink (FIFO eviction).

#### `tests/no_vm_label_in_metrics.rs`

**Source:** specified in `ADR-046-telemetry-audit-and-support` §OTEL endpoint tests.

- Collect all metric descriptors from the Provider's `METRIC_INVENTORY`.
- Assert no descriptor carries a label named `vm`.
- Assert old `d2b_daemon_vm_state` metric shape is absent from Provider's
  metric inventory.
- Assert `d2b_otel_*` label keys are all from the closed set enumerated in this spec.
- Assert `no_isolation` does not appear as a label key in any descriptor.

#### `tests/zone_startup_proceeds_without_provider.rs`

**Source:** specified in `ADR-046-telemetry-audit-and-support` §OTEL endpoint tests.

- Drive Zone bootstrap fixture with `observability-otel` Provider absent.
- Assert Zone reaches `Ready` phase.
- Assert `d2b_telemetry_drop_total` > 0 (emitter dropping frames).
- Assert no audit records are affected (audit write count unchanged).
- Assert Zone status does NOT have `telemetry-export-unavailable` condition
  (absent Provider does not set Zone conditions).

#### `tests/exporter_outage.rs`

**Source:** new; required by this dossier.

- Configure a mock OTLP server that returns a gRPC error on all requests.
- Run the collector drain loop with 50 frames.
- Assert `d2b_otel_export_batch_total{outcome="error"}` increments on each attempt.
- Assert OTEL SDK-level batch retry and Binding quota/backpressure are applied;
  ZoneLink/import transport retry is outside this unit.
- Assert `d2b_otel_export_batch_total{outcome="dropped"}` increments when SDK
  retries exhausted.
- Assert `d2b_otel_exporter_failures_consecutive` gauge increments monotonically.
- Assert provider-neutral `TelemetryBinding` condition
  `IngestAvailable=False` (`reason: "IngestUnavailable"`) and Provider detail
  `errorCode: ExporterOutage` after `failureThreshold` failures.
- Assert Binding phase transitions to `Degraded` (not `Failed`).
- Assert `d2b_telemetry_drop_total{reason="export_error"}` increments for
  each dropped batch.
- Restore mock OTLP server to healthy; assert `d2b_otel_exporter_failures_consecutive`
  resets to 0; assert Binding `IngestAvailable=True`; assert Binding returns `Ready`.
- Assert Zone mutations and audit writes continue unaffected throughout the
  outage period.

#### `tests/exporter_backpressure.rs`

**Source:** new; required by this dossier.

- Configure the mock OTLP server to stall (never respond) — simulating a slow
  backend.
- Set `maxQueueSize = 8` (test override).
- Write frames faster than the stalled exporter can drain them.
- Assert `d2b_otel_backpressure_active` gauge transitions to 1 when queue
  reaches `maxQueueSize`.
- Assert subsequent frames are dropped from the emitter ring in FIFO order.
- Assert `d2b_telemetry_drop_total{reason="buffer_full"}` increments for each
  dropped frame beyond `maxQueueSize`.
- Assert Binding `BackpressureActive=True` (`reason: "QueueFull"`) and projected
  Service `QuotaSaturated=True` when import credits are exhausted.
- Assert Binding phase is `Degraded` (not `Failed`) during backpressure.
- Unblock the mock OTLP server; allow all queued batches to drain.
- Assert `d2b_otel_backpressure_active` resets to 0 when queue depth drops
  below the 75% watermark.
- Assert Binding/Service backpressure conditions clear.
- Assert Binding returns to `Ready`.
- Assert drain loop continues accepting frames from the socket throughout the
  backpressure period (no socket closure).
- Assert no audit records or Zone mutations are affected.

#### `tests/bundle_contract.rs`

**Source:** specified in `ADR-046-telemetry-audit-and-support`
§Nix configuration and resource bundle tests.

- `bundle_is_sorted_canonically`: render Provider, authority Service, Export,
  Import, and Binding resources;
  assert JSON keys at every level are in ascending alphabetical order; assert
  resources are sorted by `(type, name)`.
- `bundle_digest_is_deterministic`: render the same config twice; assert the
  generation digest round-trips identically.
- `bundle_contains_no_secret_values`: set a Binding extension and Service policy;
  assert the rendered JSON contains no key named `secretValue`, `password`,
  `token`, `key`, or any value matching a secret pattern.
- `bundle_schema_validates_against_provider_schema`: assert Provider plus both
  qualified ResourceType specs validate against signed schemas/fingerprints.

#### `tests/controller_conformance.rs`

**Source:** new; required by standard `d2b-provider-toolkit` conformance suite.

- Drive authority Service, projected Service, and Zone/Guest Binding reconciliation
  with a fake store, fake ProviderSupervisor, and fake import adapter.
- Assert the Binding controller creates desired runtime Volume, collector,
  private Endpoints, and Guest forwarder with Binding ownership; only
  `Provider/volume-local` performs Volume effects.
- Assert: the collector declares no Provider state Volume; the
  ProviderStateSet query (`ownerRef == Provider/observability-otel`) returns
  zero Volumes.
- Assert: no cross-component Volume sharing; the runtime sockets Volume view is
  not reachable from any forwarder Process mount outside its declared view.
- Assert: controller writes Service/Binding layered status but not Provider status.
- Assert: controller creates collector and vsock-forwarder `Process` (with `providerRef:
  Provider/system-minijail`, canonical mount to `view: forwarder-write`, and
  `processClass: worker`) for a committed Guest `TelemetryBinding`.
- Assert: forwarder Process spec has `capabilityClasses: []` and
  `startRoot: false`.
- Assert: forwarder has one owned
  `Endpoint/observability-otel-vsock-ingest-<guest-uid-short>` with
  `producerRef` pointing at the forwarder Process and ownerRef pointing at the
  Binding.
- Assert: controller handles `deletion-requested` hint: sets desired lifecycle to
  `Stopped` on Binding-owned Processes, revokes/deletes private Endpoints, deletes
  the runtime Volume, and waits for finalizers;
  final deletion uses event-only Deleted-phase revision + post-commit audit.
- Assert: all mutations use `ResourceMutationBatch` with expected-revision
  preconditions.
- Assert: stale-revision conflicts trigger bounded requeue with backoff.
- Assert: idempotent reconcile: second reconcile with unchanged state emits no
  additional mutations.
- Assert Service/Binding finalizer ordering and D090 commit-before-effect.

#### `tests/provider_state.rs`

**Source:** new; required by `ADR-046-provider-state` status-first invariants.

- `no_provider_state_volume`: drive a fake Zone store with the Provider
  installed; assert `ProviderStateSet(zone, "observability-otel")` is empty (the
  collector declares no state Volume) and no
  `observability-otel--collector--runtime-state--*` Volume exists.
- `operational_state_in_status`: assert the Binding's bounded non-secret
  operational state (readiness, ingest/backpressure reconcile stage, bounded
  drop/queue counters, closed-enum error detail) is written to the owning
  resource's `status` subresource within the frozen status bounds and carries no
  secret/path/argv/PID/unit content.
- `provider_details_are_layered`: assert provider-neutral readiness, ingest,
  quota, and source-stamp observations occur only in `status.resource`, while
  SigNoz, OTLP, OTEL collector/forwarder, backend, and retry details occur only
  in the strict observability-otel `status.provider`.
- `sockets_volume_is_state_owned`: assert the runtime sockets Volume
  (`kind: tmp`) exists with
  `ownerRef: telemetry.d2bus.org.TelemetryBinding/<name>` and is not a
  Provider state Volume.
- `no_cross_component_volume_sharing`: assert no forwarder Process mount points
  to any state Volume; the sockets Volume view `forwarder-write` covers only the
  socket path.
- `restart_re_derives_status`: assert that on controller restart the collector
  readiness is re-derived from live `status` observation and reverified against
  the running process, treating status as observation, never authority.
- `volume_effects_remain_external`: assert controller writes only desired Volume
  resources and never performs filesystem/layout/ACL/quota effects.

#### `tests/config_schema.rs`

**Source:** new; adapts `policy_observability.rs` pattern.

- Assert Provider root config rejects misplaced Service/Binding/export settings.
- Assert canonical provider-neutral `telemetry.d2bus.org` base schema IDs,
  authority/projection Service conditional schemas, and exact D097 descriptor;
  reject the old provider-qualified type names and all aliases.
- Assert same-Zone Binding `serviceRef`/`producerRef`, signal subset, and common
  quota/policy bounds.
- Assert strict observability-otel Service and Binding extension schema IDs,
  backend/protocol placement, `executionRef` format, bounded implementation
  settings, deny-unknown behavior, and no common-field shadowing.
- Assert any inline secret, locator, Credential, CID, port, or stream handle is
  rejected.

#### `tests/redaction.rs`

**Source:** new; extends `policy_telemetry_redaction.rs` pattern from
`ADR-046-telemetry-audit-and-support`.

- `redaction_drops_no_isolation_attribute`: inject a span with attribute
  `no_isolation = true`; assert it is removed by the redaction filter before
  forwarding; assert `d2b_otel_frames_decoded_total{outcome="error"}` increments.
- `redaction_drops_forbidden_resource_attribute`: inject a frame with resource
  attribute key outside the allowlist; assert frame is dropped.
- `redaction_drops_path_span_attribute`: inject a span with attribute
  `path = "/run/d2b/..."`; assert it is removed.
- `redaction_drops_realm_field_in_log`: inject a structured log record with field
  `realm = "dev"`; assert the `realm` key is absent from the forwarded record.
- `redaction_passes_allowed_resource_attributes`: inject a frame with resource
  attributes from the v3 allowlist only; assert all attributes are forwarded
  unchanged.

### Integration tests (`integration/`)

#### `integration/scenario_full_pipeline.rs`

End-to-end: fake Zone store → observability-otel controller → collector process
(real binary) → mock OTLP server.

- Bootstrap a local authority `TelemetryService` and Zone `TelemetryBinding`.
- Start controller; assert Service and Binding reach `Ready`.
- Send 200 mixed frames (metrics, traces, logs) via the emitter socket.
- Assert mock OTLP server receives all 200 frames, partitioned by signal type.
- Assert `d2b.zone = "dev"` resource attribute on all received OTLP records.
- Assert `d2b.provider = "observability-otel"` resource attribute.
- Inject spoofed source identity and assert trusted `producerRef` stamping
  overwrites it.
- Assert `no_vm_label_in_metrics`: no received batch carries a metric label named
  `vm` with a resource-name value.
- Assert self-metrics ComponentSession service returns correct `d2b_otel_frames_received_total`
  counts.
- Drive `d2b zone doctor` fixture; assert `telemetry.phase = "ok"`.

#### `integration/scenario_obs_zone_forwarding.rs`

vsock-forwarder path: Guest → vsock → host forwarder → `otlp.sock` → collector
→ mock OTLP backend.

- Bootstrap a Guest-scoped `TelemetryBinding` referencing a same-Zone Service.
- Assert controller creates the Binding-owned forwarder and private Endpoint.
- Start vsock-forwarder binary; open connection to simulated vsock endpoint.
- Send 50 OTLP/gRPC frames from the simulated Guest-side collector.
- Assert mock OTLP server receives all 50 frames.
- Assert `d2b_otel_vsock_forwarder_active` gauge is 1 during test; 0 after Guest
  deletion.
- Remove `Guest/dev-vm` resource; assert controller stops the vsock-forwarder
  `Process` and its resource transitions to `Succeeded`.

#### `integration/scenario_signoz_authority.rs`

- Launch the real native SigNoz/ClickHouse/Keeper stack and real OTLP ingest
  Process; this is integration-only and never part of the hermetic suite.
- Reconcile one authority Service and reject a duplicate D097 authority.
- Export the Service (not its Endpoint), ingest all selected signals, and verify
  backend/ingest readiness, source stamping, redaction, cardinality, quota, and
  bounded backpressure.

#### `integration/scenario_real_projection_stream.rs`

- Launch two isolated Zones, a real enrolled ZoneLink/ComponentSession stream,
  one Service Export/Import, core-owned projected Service, and producer Binding.
- Verify bytes traverse the real bounded encrypted named stream while
  intermediate controllers see ciphertext and no FD/socket crosses the Zone.
- Revoke/reconnect and verify lease/credit teardown, Binding/projection Degraded,
  then generation/fingerprint revalidation before readiness.
- Verify authoritative audit remains in the producer Zone.

#### `integration/scenario_provider_removal.rs`

Config-owned cleanup: remove Provider from Nix config; assert ordered child
deletion and Zone status.

- Bootstrap: activate generation 1 with `Provider/observability-otel` in Zone.
- Activate generation 2 without `Provider/observability-otel`.
- Assert `Provider/observability-otel` receives `metadata.deletionRequestedAt`.
- Assert `deletion-pending` condition is set on Provider.
- Assert Zone `pending-cleanup` condition is set; Zone phase is `Degraded`.
- Assert Bindings are deleted first; controller drains/deletes their owned
  Processes/private Endpoints and desired runtime Volumes.
- Assert Imports revoke and core deletes projected Services before authority
  Export/Service deletion. No Provider state Volume exists.
- Assert emitter socket is not removed while collector is still running.
- Assert all scheduled Deletes complete; `pending-cleanup` condition cleared.
- Assert Zone returns to `Ready` phase.
- Assert `d2b_telemetry_drop_total` increments after collector stops (emitter
  ring filling).
- Assert audit segment files in `$ZONE_STATE/audit/` are unchanged (cleanup
  does not touch audit data).
- Assert final deletion: `Provider/observability-otel` row is removed from the
  store via an event-only Deleted-phase revision; the
  `ResourceMutation{event="deleted"}` audit record is durably written to the
  audit sink only after the store transaction commits (post-commit audit).

Per D094, each replaced current-code test is retired with an explicit
keep/adapt/move/delete disposition and a removal gate: the minimum reusable
semantic assertions migrate into this crate's hermetic `tests/`, and the old
duplicate tests, shell gates, fixtures, static artifacts, CI jobs, and manifest
entries are deleted once successor coverage and the removal proof pass —
updating `tests/layer1-jobs.json`, the closed gate manifests, the
flake/matrix/Nix-unit pins, the generated ledgers, and the CI workflow shards.
Old and new suites never run in parallel indefinitely.

---

## Current-code fit

| Item | Treatment |
| --- | --- |
| Current anchor | `packages/d2b-host/src/otel_host_bridge_argv.rs` (`OtelHostBridgeArgvInputs`, `otel_host_bridge_argv`, vsock forwarding via socat); `packages/d2bd/src/otel_host_bridge_readiness.rs` (`OtelHostBridgeReadiness::{Ready,Pending,Failed}`); `packages/d2b-core/src/processes.rs::ProcessRole::OtelHostBridge`; `packages/d2b-contracts/src/broker_wire.rs::RunnerRole::OtelHostBridge`; `nixos-modules/components/observability/host.nix` (ACL/socket pattern, `scrapeJournal`, `identityName`); `nixos-modules/components/observability/stack.nix` (SigNoz stack, `ingressSources`); `nixos-modules/components/observability/guest.nix` (per-VM guest collector); `packages/d2b-contract-tests/tests/{policy_observability.rs,minijail_relay_otel.rs}` |
| Evidence class | `otel_host_bridge_argv.rs`, readiness module, `ProcessRole::OtelHostBridge`, `RunnerRole::OtelHostBridge`: implemented-and-reachable. Nix `observability/{host,stack,guest}.nix`: implemented-and-reachable for the v1 daemon pipeline. `d2b-provider-observability-otel` crate: ADR-only. |
| Behavior retained | Native adaptation of the vsock pattern; readiness state machine mapped to Service/Binding status; per-source ingress mapped to trusted `producerRef` stamping; ACL/socket pattern; edge retry/queue; SigNoz loopback stack; bounded redaction/cardinality allowlist; audit separation |
| Required delta | Two provider-neutral `telemetry.d2bus.org` public ResourceTypes, canonical base schemas, and separate strict observability-otel Provider extensions; one D097 authority `TelemetryService`; D096 Service Export/Import/projection chain; per-producer `TelemetryBinding`; Binding-owned collector/forwarder/private Endpoints/runtime Volume; source stamping; quota/backpressure status; self-metrics; Nix authoring and conformance tests; full OTEL SDK only in the Provider |
| Reuse path | Adapt `OtelHostBridgeArgvInputs` into Binding-owned native forwarding; adapt readiness into Service/Binding conditions; adapt `otelRuntimeDir` into a Binding-owned desired Volume; adapt `ingressSources` into trusted producer identity and one authority Service; preserve `scrapeJournal`, SigNoz stack, policy tests, and redaction gates |
| Replacement/deletion | Retire socat runner, old readiness gate, `ProcessRole::OtelHostBridge`, and `RunnerRole::OtelHostBridge` after Service/Binding and projection-chain parity. Retire per-VM guest collector after Binding-owned edge children pass. Endpoint remains private transport; no Endpoint projection compatibility alias. |
| Feasibility proof | `OtelHostBridgeArgvInputs` vsock forwarding proven in the v3 baseline. SigNoz OTLP collector proven operational in `stack.nix`. Unix datagram socket ACL/emitter pattern proven in `host.nix`. `OtelHostBridgeReadiness` state machine proven in `otel_host_bridge_readiness.rs`. OTLP/gRPC client proven by `minijail_relay_otel.rs`. |
| Future owner | Work items below |

---

## Implementation work items

### ADR046-otel-001

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-otel-001` |
| Dependency/owner | ADR046-telem-001 (BoundedEmitter crate); W0/W2; telemetry/observability owner |
| Current source | `packages/d2b-host/src/otel_host_bridge_argv.rs` (`OtelHostBridgeArgvInputs`, `otel_host_bridge_argv`, `OtelHostBridgeArgvError`); `packages/d2bd/src/otel_host_bridge_readiness.rs` (`OtelHostBridgeReadiness`, `otel_host_bridge_read`); `packages/d2b-contracts/src/broker_wire.rs::RunnerRole::OtelHostBridge`; `packages/d2b-core/src/processes.rs::ProcessRole::OtelHostBridge` |
| Reuse action | adapt (`OtelHostBridgeArgvInputs` vsock logic → native Rust OTLP relay); adapt (`OtelHostBridgeReadiness` → `TelemetryBinding` conditions); delete-after-cutover (`RunnerRole::OtelHostBridge`, `ProcessRole::OtelHostBridge`) |
| Destination | `packages/d2b-provider-observability-otel/src/{forwarder_bin,controller,binding}.rs` |
| Detailed design | Binding-owned forwarder: accept OTLP only from the exact Guest producer, relay through the Binding-private Endpoint/Volume to its edge collector, enforce bounded frames/quota/session timeout, and use no OTEL SDK. Map forwarder readiness to `status.provider`, then derive provider-neutral Binding ingest readiness; Process Provider owns launch/pidfd. |
| Integration | Controller creates vsock-forwarder long-lived `Process` → ProviderSupervisor → system-minijail/systemd launch → vsock socket bind → Guest side connects |
| Data migration | Full reset; existing socat bridge retired after cutover |
| Validation | `integration/scenario_obs_zone_forwarding.rs`; adapted `minijail_relay_otel.rs` shape test for Provider-managed runner; assert `RunnerRole::OtelHostBridge` is absent from `d2b-contracts` after removal |
| Removal proof | Legacy symbols removed only after Binding ownership/readiness and forwarding integration pass |

### ADR046-otel-002

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-otel-002` |
| Dependency/owner | ADR046-otel-001 + ADR046-telem-001 + ADR046-provider-001 (Provider toolkit) + resource/Endpoint/Volume contracts; W2; observability owner |
| Current source | `nixos-modules/components/observability/host.nix` (`otelRuntimeDir`, `hostEgressSocket`, `setfacl` ACL pattern, `scrapeJournal` option, `identityName`); `nixos-modules/components/observability/stack.nix` (`ingressSources`, `vmName`, `receiverGrpcPort`, loopback binding, `signoz.listenPort`) |
| Reuse action | adapt Nix pipeline shape (replace per-VM `vmName` with per-Zone name; replace socat runner with vsock-forwarder long-lived Process; adapt `ingressSources` per-Zone entry) |
| Destination | `packages/d2b-provider-observability-otel/src/{collector_bin,emitter_socket,exporter,controller,service,binding}.rs`; updated Nix observability modules |
| Detailed design | Register the initial implementation of both provider-neutral qualified ResourceTypes, their canonical base schemas, and separate strict observability-otel Service/Binding spec and status extensions. Reconcile each Binding into an edge collector, private Endpoints, runtime Volume, and optional forwarder. Collector links the full OTEL SDK, resolves `serviceRef`, stamps trusted producer identity, and enforces common signals/quota/policy plus strict batching extension. Write generic Service/Binding observations to `status.resource` and SigNoz/OTLP/OTEL observations only to `status.provider`; no state file or Provider state Volume. Provider root config remains installation-only. |
| Integration | `BoundedEmitter` → Binding-private Endpoint → edge collector/OTEL SDK → same-Zone authority or projected Service → SigNoz |
| Data migration | Existing SigNoz data not migrated; v3 starts fresh per Zone |
| Validation | `tests/emitter_socket_receive.rs`; `tests/exporter_outage.rs`; `tests/exporter_backpressure.rs`; `integration/scenario_full_pipeline.rs`; adapted `policy_observability.rs` (retain all existing assertions; add new `d2b.zone`, `d2b.provider` allowlist entries) |
| Removal proof | `guest.nix` per-VM guest collector retired after `integration/scenario_obs_zone_forwarding.rs` passes |

### ADR046-otel-003

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-otel-003` |
| Dependency/owner | ADR046-otel-002 + ADR046-telem-001 + ADR046-volume-001; Nix/observability owner |
| Current source | `nixos-modules/components/observability/host.nix::journaldStorageDir`, `scrapeJournal` option; journald cgroup-path filtering pattern |
| Reuse action | adapt journald receiver config for per-Zone cgroup filter |
| Destination | `packages/d2b-provider-observability-otel/src/nix/journald.nix`; `packages/d2b-provider-observability-otel/src/journald.rs` |
| Detailed design | Per `ADR-046-telemetry-audit-and-support` §journald stdout/stderr ingestion: cgroup filter derived from trusted `producerRef`; redaction drops credential/secret/path fields, `_CMDLINE`, `_EXE`, and `INVOCATION_ID`; the strict `TelemetryBinding.spec.provider.settings.journald.enable` defaults false. |
| Integration | Collector binary journald receiver config path → cgroup filter expression → OTel Collector journald receiver → redaction filter → OTLP export |
| Data migration | None — full d2b 3.0 reset; no prior state to migrate |
| Validation | Nix eval test: filter expression set when enabled; assert `_CMDLINE` and `INVOCATION_ID` in drop list; `tests/redaction.rs` for journald field redaction |
| Removal proof | Not applicable |

### ADR046-otel-004

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-otel-004` |
| Dependency/owner | ADR046-otel-002; policy/contract-tests owner |
| Current source | `packages/d2b-contract-tests/tests/policy_observability.rs` (`loki_native_otel_resource_attributes` allowlist; `tempo_stack_signoz_backend_and_collector`; `startup_tracing_avoids_host_path_fields`); `packages/d2b-contract-tests/tests/policy_metrics.rs` (`EXPECTED_METRICS` table); `packages/d2b-contract-tests/tests/minijail_relay_otel.rs` |
| Reuse action | adapt and extend existing tests; keep existing test assertions |
| Destination | `packages/d2b-contract-tests/tests/policy_observability.rs` (updated); `packages/d2b-contract-tests/tests/policy_telemetry_redaction.rs` (new, per ADR046-telem-008); `packages/d2b-provider-observability-otel/tests/no_vm_label_in_metrics.rs` |
| Detailed design | (1) Extend `loki_native_otel_resource_attributes` allowlist with `d2b.zone`, `d2b.provider`, `d2b.component`, `service.version`. (2) Add gate: `no_isolation` must not appear in any Provider `MetricDescriptor` label or span attribute catalog. (3) Adapt `minijail_relay_otel.rs` shape test for Provider-managed runner (no broker `RunnerRole::OtelHostBridge`). (4) Add metric inventory gates for `d2b_otel_*` instruments from this spec. (5) Retain: `startup_tracing_avoids_host_path_fields`; SigNoz-only backend assertion; `tempo_guest_collector_shape`; `config_source = "realm-controllers"` absence gate. |
| Integration | Contract-tests run in workspace `make test-drift` and `make test-lint` |
| Data migration | None — full d2b 3.0 reset; no prior state to migrate |
| Validation | All contract-tests pass after update; existing allowlist test does not regress |
| Removal proof | Not applicable |

### ADR046-otel-005: Cross-Zone telemetry-ingest export/import adapter (D096)

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-otel-005` |
| Dependency/owner | ADR046-zone-control-019, ADR046-zone-control-020; observability Provider owner |
| Current source | None — net-new ADR 0046 cross-Zone sharing (D096) |
| Reuse source | SigNoz ingest authority (this dossier); `packages/d2b-provider/src/share_adapter.rs` `ExportAdapter`/`ImportAdapter` traits |
| Reuse action | net-new (implement the signed observability export/import adapter) |
| Destination | `packages/d2b-provider-observability-otel/src/share_adapter.rs` |
| Detailed design | Implement the signed adapter: `sys-obs` exports the authority `TelemetryService`, with one referenced local ingest Endpoint as transport; every producer imports a core-owned local `TelemetryService` projection. Binding `serviceRef` targets that projection. Enforce many-to-one quota/credit/backpressure/schema/source-stamp/redaction/cardinality over bounded encrypted streams; no FD/socket crosses a Zone and audit authority stays local. |
| Integration | Core export/import controller (ADR046-zone-control-019); local projection lifecycle (ADR046-zone-control-020); ComponentSession bounded encrypted named streams |
| Data migration | None — full d2b 3.0 reset |
| Validation | Fast `projection_chain.rs` proves Service projection semantics with a fake stream; integration alone runs real encrypted streams and SigNoz; revocation/reconnect, quotas, source stamp, redaction/cardinality, no FD crossing, and audit locality |
| Removal proof | Not applicable |

### ADR046-otel-006: TelemetryService authority and TelemetryBinding realization (D096/D097)

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-otel-006` |
| Dependency/owner | ADR046-otel-005, ADR046-zone-control-019, ADR046-zone-control-020; observability Provider owner |
| Current source | `nixos-modules/components/observability/{stack,host,guest}.nix` (SigNoz/ClickHouse/Keeper loopback authority, per-source vsock ingress, source-identity `upsert`, `sending_queue`/`retry_on_failure`, allow-list projection); `nixos-modules/{index,observability-vm}.nix`; `packages/d2b-host/src/{otel_host_bridge_argv,vsock_relay_argv}.rs` (pre-opened vsock fds; broker rejects source VM ≠ obs VM); `packages/d2bd/src/otel_host_bridge_readiness.rs` |
| Reuse source | Same baseline stack/bridge/readiness symbols; `packages/d2b-provider/src/share_adapter.rs` traits |
| Reuse action | net-new universal provider-neutral `telemetry.d2bus.org.TelemetryService`/`telemetry.d2bus.org.TelemetryBinding` pair; adapt existing authority/edge behavior as the initial observability-otel implementation |
| Destination | `packages/d2b-provider-observability-otel/src/{authority,service,binding,projection}.rs`; `AuthorityDescriptor` on the `sys-obs` `TelemetryService` |
| Detailed design | Implement one provider-neutral D097 authority Service with generic telemetry-ingest Endpoint refs and common service/signals/quota/policy fields. Its strict observability-otel `spec.provider` alone selects the loopback SigNoz stack, backend Endpoints, and OTLP. Core rejects duplicates and adopts by owner proof. Implement core-owned imported Service projections with no `spec.provider` or backend ownership. Implement per-producer Bindings with common service/producer/signals/quota/policy fields and strict implementation extension; Bindings own/cause edge collector/forwarder/private Endpoints/runtime Volume. Keep generic observations in `status.resource` and SigNoz/OTLP/OTEL observations in `status.provider`. Preserve trusted source upsert, retry/queue, bounded cardinality/redaction, audit non-transfer, status-first state, and no OTEL SDK in core. Endpoint is transport only. |
| Integration | Ingest authority + export (ADR046-otel-005); core export/import controller and projection lifecycle (ADR046-zone-control-019/020); ComponentSession per-import encrypted streams; d2b-telemetry closed-label metrics |
| Data migration | None — full d2b 3.0 reset |
| Validation | Fast `resource_service_binding.rs` and `projection_chain.rs` plus reused nix-unit/policy tests prove provider-neutral names, base/Provider field separation, status layering, ownership, schemas, stamping, quotas, redaction, and projection chain. Real SigNoz and real stream scenarios are integration-only. |
| Removal proof | Legacy fixed per-source vsock ingress and old gates are removed only after the Service/Binding/ComponentSession successor passes; neither old provider-qualified ResourceType name nor any Endpoint-projection or ResourceType alias remains, and no duplicate suite remains. |

---

## README.md requirement

`packages/d2b-provider-observability-otel/README.md` must document:

1. **Provider identity**: `Provider/observability-otel`, crate name, API major version.
2. **Purpose**: the only place linking `opentelemetry_sdk`; core uses only `BoundedEmitter`.
3. **ResourceTypes**: provider-neutral
   `telemetry.d2bus.org.TelemetryService` and
   `telemetry.d2bus.org.TelemetryBinding`, initially implemented by
   `Provider/observability-otel`, including canonical base
   schemas/status/conditions, strict Provider extensions, and no aliases.
4. **Cross-Zone chain**: authority Service → Export plus local ingest Endpoint → Import → core-owned projected Service → Binding `serviceRef`; Endpoint is never the semantic projection.
5. **D097**: descriptor on the generic authority Service, with SigNoz/OTLP
   selection only in `spec.provider`, plus duplicate
   rejection/adoption/quarantine.
6. **Binding intent**: same-Zone Service/producer refs, signals, common quotas/policy, strict Provider extension, and no Binding export.
7. **Ownership**: Binding-owned collector/forwarder/private Endpoints/runtime Volume; projection owned by ResourceImport; child-first finalizers.
8. **Status/update**: status-first bounded non-secret state, provider-neutral
   observations in `status.resource`, SigNoz/OTLP/OTEL details only in
   `status.provider`, and D091 authority-to-child propagation; no Provider state
   Volume.
9. **Components/placement**: Binding-owned edge collector and optional Guest forwarder under system-minijail.
10. **RBAC/security**: minimum claims, zero capabilities, no ambient network/credentials/remote Refs, no Provider-self status write.
11. **Telemetry policy**: trusted source stamping, signal/quotas/backpressure, redaction/cardinality, and audit non-transfer.
12. **Startup/lifecycle**: optional Provider never blocks Zone bootstrap; drain/restart/revoke/delete ordering.
13. **Tests**: fast hermetic Service/Binding separation and projection-chain tests; real SigNoz and real stream tests integration-only.
14. **Standalone consumption**: Nix artifact registration and authority/producer authoring examples.
