# ADR 0046 Provider dossier: observability-otel

| Field | Value |
| --- | --- |
| Spec ID | `ADR-046-provider-observability-otel` |
| Parent | ADR 0046 |
| Status | Proposed |
| Version | 1 |
| Baseline | `b5ddbed67867d9244bf33390868101bd9b053e49` |
| Normative | Yes |
| Owners | `d2b-provider-observability-otel`, telemetry/observability integrator |
| Depends on | `ADR-046-terminology-and-identities`, `ADR-046-resource-object-model`, `ADR-046-resource-api-and-authorization`, `ADR-046-resource-reconciliation`, `ADR-046-provider-model-and-packaging`, `ADR-046-primitive-resource-composition`, `ADR-046-components-processes-and-sandbox`, `ADR-046-componentsession-and-bus`, `ADR-046-resources-volume`, `ADR-046-resources-credential`, `ADR-046-telemetry-audit-and-support`, `ADR-046-nix-configuration` |
| Supersedes | `ProcessRole::OtelHostBridge` / `RunnerRole::OtelHostBridge`; socat-based vsock forwarder in `packages/d2b-host/src/otel_host_bridge_argv.rs`; `packages/d2bd/src/otel_host_bridge_readiness.rs`; hand-rolled per-VM `nixos-modules/components/observability/` pipeline (adapted to per-Zone) |

## Purpose

This dossier exhaustively specifies `Provider/observability-otel`: the d2b 3.0
observability telemetry Provider. It is the only place in the d2b process graph
that links the full OpenTelemetry SDK with an OTLP/gRPC exporter. Every other
Zone and core process uses a lightweight `BoundedEmitter` (from `d2b-telemetry`)
that has no `opentelemetry_sdk` or `opentelemetry-otlp` dependency.

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
| `ProcessRole::OtelHostBridge` (`d2b-core/src/processes.rs`) | `Process` resource with `ownerRef: Provider/observability-otel`; template `otel-collector` | implemented-and-reachable |
| `RunnerRole::OtelHostBridge` (`d2b-contracts/src/broker_wire.rs`) | ProviderSupervisor launch ticket issued by observability-otel controller; broker `SpawnRunner` retired | implemented-and-reachable |
| `OtelHostBridgeArgvInputs` (`packages/d2b-host/src/otel_host_bridge_argv.rs`) | native OTLP/gRPC-over-vsock forwarder owned by the Provider; socat argv retired | implemented-and-reachable |
| `OtelHostBridgeReadiness::{Ready,Pending,Failed}` (`packages/d2bd/src/otel_host_bridge_readiness.rs`) | Provider phase `Ready`/`Pending`/`Degraded` in `Provider/observability-otel` resource status | implemented-and-reachable |
| `nixos-modules/components/observability/host.nix` `otelRuntimeDir = "/run/d2b/otel"` | `$ZONE_STATE/telemetry/` Volume owned by observability-otel controller | implemented-and-reachable |
| `nixos-modules/components/observability/host.nix` `hostEgressSocket` | `$ZONE_STATE/telemetry/emitter.sock` datagram socket, owned by the collector process UID | implemented-and-reachable |
| `nixos-modules/components/observability/stack.nix` `ingressSources` per `vmName` | `ingressSources` per Zone name (from `d2b.zones.<zone>` attr key) | generated-or-eval-contract |
| `nixos-modules/components/observability/guest.nix` per-VM guest collector | no per-Guest separate OTel Collector process; vsock-forwarder long-lived `Process` per Guest routes to the Zone collector | generated-or-eval-contract |
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

---

## Crate layout

```
packages/d2b-provider-observability-otel/
  src/
    lib.rs                  provider identity + version constants
    config.rs               config schema DTOs and validation
    controller.rs           lifecycle controller (owns collector Process + telemetry Volume)
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
    config_schema.rs
    redaction.rs
  integration/
    scenario_full_pipeline.rs
    scenario_obs_zone_forwarding.rs
    scenario_provider_removal.rs
  README.md
```

Workspace policy rejects a Provider crate missing any of `src/`, `tests/`,
`integration/`, or `README.md` (per `ADR-046-provider-model-and-packaging`
§Crate/package boundary).

---

## Process components

### collector (controller-managed, long-lived Process)

The primary component. Runs the full OTEL SDK.

| Field | Value |
| --- | --- |
| Binary | `d2b-provider-observability-otel-collector` (built from `src/collector_bin.rs`) |
| Process template name | `otel-collector` |
| Component type | `service` (no ResourceType ownership; all reconciliation is in the controller) |
| cardinality | exactly one per `Provider/observability-otel` instance |
| executionRef | `Host/<host>` — same Host as the owning Zone runtime |
| domain | `system` |
| ownerRef | `Provider/observability-otel` |
| `managedBy` | `controller` |
| Restart policy | restart-on-failure with bounded exponential backoff; max 5 restarts in 10 min before `Failed` |
| SDK linkage | `opentelemetry_sdk`, `opentelemetry-otlp`, `opentelemetry_sdk::export::trace`, `opentelemetry_sdk::metrics` |

**Process responsibilities:**
1. Owns `$ZONE_STATE/telemetry/emitter.sock` (Unix datagram socket) — drains
   compact telemetry frames from core `BoundedEmitter` instances.
2. Owns `$ZONE_STATE/telemetry/otlp.sock` (Unix socket) — accepts OTLP/gRPC
   from Provider processes that embed the full SDK.
3. Decodes frames from `emitter.sock`, reconstructs OTEL metrics/traces/logs.
4. Accepts standard OTLP/gRPC protocol on `otlp.sock`.
5. Batches and exports via OTLP/gRPC to the configured backend through the
   declared transport Provider (`spec.config.export.exportTransportProviderRef`).
6. Runs the optional journald receiver when `spec.config.journald.enable = true`.
7. Registers the `d2b.observability.v1.SelfMetrics` ComponentSession service on
   d2b-bus (`direction: local`) for `d2b zone doctor` and `d2b zone support-bundle`.

### vsock-forwarder (long-lived Process, one per active Guest)

A long-lived Process, active while the corresponding Guest is running, launched
by the observability-otel controller for Guests that need to forward telemetry to
the host-side Zone collector.

| Field | Value |
| --- | --- |
| Binary | `d2b-provider-observability-otel-forwarder` (built from `src/forwarder_bin.rs`) |
| Process template name | `otel-vsock-forwarder` |
| Component type | `worker` (long-lived Process) |
| cardinality | at most one per Guest attachment |
| executionRef | `Host/<host>` |
| domain | `system` |
| ownerRef | `Provider/observability-otel` |
| `managedBy` | `controller` |
| Restart policy | restart-on-failure with bounded backoff; desired lifecycle set to `Stopped` on Guest deletion |
| SDK linkage | none; this binary is a thin vsock ↔ Unix stream relay only |

**Process responsibilities:**
1. Listens on a vsock port allocated by the Zone.
2. Accepts OTLP/gRPC from the Guest-side collector (or Guest-resident SDK).
3. Forwards frames to the `otlp.sock` path inside the telemetry Volume (resolved
   via LaunchTicket at launch time; never exposed in spec/status/audit).
4. Enforces per-frame bounded size (max 4 MiB); timeout on stalled connections.
5. Runs until the Guest stops; controller sets desired lifecycle to `Stopped` on
   Guest deletion or Provider deletion.
6. No OTEL SDK; purely packet relay with size-bounded records.
7. Replaces the socat-based `OtelHostBridgeArgvInputs` relay
   (`packages/d2b-host/src/otel_host_bridge_argv.rs`).

---

## Root config schema

Every leaf has explicit bounds. No inline token, password, key, endpoint URI,
CID, port, or TLS material may appear anywhere in `spec.config`. Network/TLS
endpoint addressing belongs to the transport Provider; Credential operations
belong to the transport Provider/export alias. The configured transport
Provider/export alias owns all auth, TLS, and endpoint details.

```yaml
spec:
  artifactId: provider-observability-otel   # selects catalog entry
  config:
    # Required. Host where the collector Process runs.
    # Must be a Host in the same Zone. Defaults to Zone's primary Host.
    executionRef: "Host/host-system"
    export:
      # Required. Same-Zone ResourceRef to the transport Provider that carries
      # OTLP data to the backend. Must match pattern Provider/transport-*.
      # The transport Provider owns endpoint addressing, TLS, network retry,
      # and any Credential operations for auth.
      exportTransportProviderRef: "Provider/transport-vsock"
      # Required. Bounded alias string identifying the export target within the
      # transport Provider's private config. Pattern [a-z][a-z0-9-]*, 1..64 chars.
      exportTargetAlias: "signoz-otlp"
      # Maximum consecutive export failures before component health reports
      # telemetry-export-unavailable. Range 1..100. Default 5.
      failureThreshold: 5
    emitter:
      # Capacity of the in-process ring for metrics frames.
      # Range: 524288 (512 KiB) .. 67108864 (64 MiB). Default 4194304 (4 MiB).
      ringCapacityBytesMetrics: 4194304
      # Capacity of the in-process ring for trace frames.
      # Range: 524288 .. 67108864. Default 4194304.
      ringCapacityBytesTraces: 4194304
      # Capacity of the in-process ring for log frames.
      # Range: 262144 (256 KiB) .. 33554432 (32 MiB). Default 2097152 (2 MiB).
      ringCapacityBytesLogs: 2097152
    otlpExporter:
      # Maximum number of spans/metric points per batch. Range 1..2048.
      batchMaxExportSize: 512
      # Batch assembly delay before forced flush. Range 100..30000 ms.
      batchScheduleDelayMs: 5000
      # Batch export timeout (OTEL SDK level). Range 1000..120000 ms.
      batchExportTimeoutMs: 30000
      # Maximum queue depth (batches). Range 8..4096.
      maxQueueSize: 2048
      # Enable gzip compression on OTLP export. Default true.
      compressionEnabled: true
    journald:
      # Enable journald receiver for per-Zone cgroup log scraping.
      # Default false. Requires explicit operator consent.
      enable: false
      # Cgroup filter prefix derived from Zone ID; not user-configurable.
    selfMetrics:
      # Expose self-metrics as a ComponentSession service on d2b-bus. Default true.
      enable: true
```

### Forbidden config values (enforced at eval and build time)

- Any bare endpoint URI, host address, CID, port number, TLS material, or
  Credential ref in any config leaf; the transport Provider owns all of these.
- `exportTransportProviderRef` that does not resolve to an installed Provider in
  the same Zone fails eval with `unresolved-transport-provider`.
- `exportTransportProviderRef` values that do not match `^Provider/transport-[a-z][a-z0-9-]*$`
  fail eval with `invalid-transport-provider-ref`.
- `exportTargetAlias` values that do not match `^[a-z][a-z0-9-]{0,63}$` fail
  eval with `invalid-export-target-alias`.
- Any leaf value matching a secret heuristic pattern (token, password, key,
  secret, credential bytes) in any config field.
- Numeric values outside their stated ranges fail NixOS eval with a
  schema-range violation.
- Unknown `spec.config.*` fields fail NixOS eval (schema-generated options).

---

## ResourceTypes

### Implemented (owned by this Provider)

This Provider declares no new public ResourceTypes and does **not** export or
implement `Volume` as a ResourceType. `Process` children (forwarder workers) are
the sole reconciliation effect produced by the lifecycle controller.
`Provider/volume-local` is the sole Volume reconciler; the observability-otel
controller never acts as a Volume reconciler.

### Consumed (controller reads or watches)

| ResourceType | Usage |
| --- | --- |
| `Provider` (self) | controller watches own resource for config changes |
| `Provider` (transport) | controller resolves `spec.config.export.exportTransportProviderRef` |
| `Process` | controller creates and owns vsock-forwarder `Process` children per Guest |
| `Volume` | controller reads (watch-only) telemetry socket directory Volume status to derive forwarder readiness; controller does NOT create, update, or delete any Volume |
| `Host` | controller reads `spec.config.executionRef` for forwarder placement |
| `Guest` | controller watches for Guests that need vsock-forwarder Process children |

Core (not the controller) creates the runtime telemetry socket directory Volume
(`kind: tmp`) and the collector `Process` from the signed `ProviderDeployment`
descriptor before component Processes start. It does **not** create any Provider
state Volume — the collector declares none, and its bounded non-secret
operational state lives in `status`/the core Operation ledger (D087). Core
deletes the sockets Volume after component Process finalizers complete when the
Provider is removed. Core also derives and writes
`Provider/observability-otel` status by aggregating component health.

**D089 desired-spec shape.** `Provider/observability-otel` writes mostly
ResourceType base `spec.*` fields for `Process` children and the
ProviderDeployment-created `Volume`; it carries little or no `spec.provider`
payload on those resources today. Any future implementation-only desired
configuration must use the canonical `spec.provider = { schemaId,
schemaVersion, settings }` envelope, registered/signed in the Provider
manifest, deny-unknown, bounded, versioned/digested, validated against
`spec.providerRef` at Nix build and API admission, and forbidden to shadow base
fields. Shared fields are promoted to the ResourceType base. Each
`ResourceApiBinding` implements the exact base spec/status schema
version/fingerprint, accepts the canonical minimal base Spec, passes base
conformance, and rejects an
unsupported optional base capability only through
its signed capability matrix plus provider-neutral `unsupported-capability`.
`spec.provider` aligns with `status.provider`. The `Provider` resource itself
keeps the D075 `spec.{artifactId, config}` exception.

The semantic controller does not create, update, delete, or watch-for-mutation
any Volume. Volume is not added to the controller's exported resource permissions.
`Provider/volume-local` is the sole Volume reconciler and owns all layout, ACL,
identity-marker, quota, and lifecycle operations on the backing filesystem.

---

## Controller

### Lifecycle controller

The observability-otel lifecycle controller is one controller component inside
the `d2b-provider-observability-otel-controller` binary. It runs as a normal
async reconcile loop over d2b-bus/ComponentSession, using the standard toolkit
`ResourceClient` / `Reconciler` API.

**Scope.** Core creates the static telemetry socket directory `Volume` and the
collector `Process` from the signed `ProviderDeployment` descriptor at install
time. The controller does **not** bootstrap Volumes or core Processes, and does
**not** write `Provider/observability-otel` status (core derives it from
component health). The controller's exclusive runtime responsibility is watching
`Guest/*` and managing per-Guest vsock-forwarder `Process` children.

**Controller watch plan:**

```text
watch Provider/observability-otel          # own resource; detect config changes
watch Process/<forwarder-*>               # owned vsock-forwarder Process children
watch Guest/*                              # for vsock-forwarder dispatch
```

**Reconcile flow on `Provider/observability-otel` Ready:**

1. Verify `spec.config.export.exportTransportProviderRef` resolves to a Ready
   transport Provider in the same Zone; set `transport-unresolved` component
   health condition if not.
2. On new `Guest/*` resources: create vsock-forwarder long-lived `Process` child
   with `ownerRef: Provider/observability-otel`, `managedBy: controller`,
   `providerRef: Provider/system-minijail`, and config projection scoped to the
   forwarder template (see §Canonical Process templates).
3. On `Guest/*` deletion: set matching vsock-forwarder `Process` desired lifecycle
   to `Stopped`; clear after finalizer.

**Reconcile flow on `deletion-requested`:**

Follows `ADR-046-telemetry-audit-and-support` §observability-otel Provider
cleanup sequence exactly:
1. Receive `deletion-requested` hint; add `deleting-children` condition.
2. Set desired lifecycle to `Stopped` on all vsock-forwarder `Process` children;
   wait for their finalizers to complete.
3. Core stops the collector `Process` (drain emitter ring before SIGTERM, up to
   10 s; flush OTLP exporter batches, up to 30 s; SIGTERM; SIGKILL after
   drainTimeout); core deletes the telemetry Volume after collector exits.
4. Clear controller finalizer; core commits a Deleted-phase revision event and
   removes the `Provider/observability-otel` row and all index entries in one
   store transaction; the `ResourceMutation{event="deleted", trigger="config-cleanup"}`
   audit record is written to the audit sink post-commit (durably after the store
   transaction commits).

**Conformance requirement:** the controller must pass the standard reconciliation
conformance suite from `d2b-provider-toolkit` (optimistic `ResourceMutationBatch`
writes, expected-revision preconditions, bounded requeue backoff, idempotent
reconcile).

---

## Volume: telemetry socket directory

Core creates one `Volume` for the telemetry socket directory from the static
`ProviderDeployment` descriptor in the signed artifact, when the Provider is
installed. The Volume naming convention per `ADR-046-provider-state` §Volume
creation and ownership is `<provider>--<component>--<namespace>--<exec-short>`.

### Exact Volume ResourceSpec

```yaml
apiVersion: resources.d2bus.org/v3
type: Volume
metadata:
  name: observability-otel--collector--sockets--host-system
  zone: work                       # Zone name resolved at runtime
  ownerRef: Provider/observability-otel
  finalizers: []
spec:
  providerRef: Provider/volume-local
  source:
    executionRef: Host/host-system  # resolved from spec.config.executionRef
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
`ProviderStateSet` is empty. The collector carries no durable data across
restarts: telemetry is exported to its configured backend, and OTEL owns
metrics/traces by construction. Its bounded non-secret operational state —
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
- The observability-otel controller does not add `Volume` to its exported
  ResourceTypes.
- The controller re-derives collector readiness from live `status` observation
  after restart and reverifies against the running process, treating `status`
  as observation, never authority (D087).
---

## Canonical Process templates

Core creates the collector Process from the static `ProviderDeployment`
descriptor. The controller creates per-Guest forwarder Processes. Both use
`providerRef: Provider/system-minijail`. The mount uses the named view declared
in the Volume spec. No executable path, UID/GID, host path, or raw capability
appears in these specs; the sandbox is compiled semantically by system-minijail.

### otel-collector Process (static; created by core)

```yaml
apiVersion: resources.d2bus.org/v3
type: Process
metadata:
  name: observability-otel--collector
  zone: work
  ownerRef: Provider/observability-otel
  finalizers: []
spec:
  providerRef: Provider/system-minijail
  executionRef: Host/host-system         # resolved from spec.config.executionRef
  domain: system
  userRef: User/observability-otel-system
  processClass: service
  template: otel-collector               # signed template from Provider package
  mounts:
    - volumeRef: Volume/observability-otel--collector--sockets--host-system
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

### otel-vsock-forwarder Process (dynamic; created by controller per Guest)

One instance per active Guest. `<guest-uid-short>` is a stable opaque short ID
derived from the Guest UID; it is not the human-readable Guest name.

```yaml
apiVersion: resources.d2bus.org/v3
type: Process
metadata:
  name: observability-otel--fwd-<guest-uid-short>
  zone: work
  ownerRef: Provider/observability-otel
  finalizers: []
spec:
  providerRef: Provider/system-minijail
  executionRef: Host/host-system         # same Host as collector
  domain: system
  userRef: User/observability-otel-system
  processClass: worker
  template: otel-vsock-forwarder         # signed template from Provider package
  mounts:
    - volumeRef: Volume/observability-otel--collector--sockets--host-system
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
conformance. Stable collector ingest services and per-Guest forwarder ingest
services are owned `Endpoint` resources with `producerRef`; they are not inline
`Process.spec` fields. Consumers use `Endpoint/<name>` references. Endpoint
spec/status/CLI/audit/telemetry never include raw socket paths, CIDs, ports,
IP addresses, fd numbers, OTLP payload bytes, span/log bodies, or credentials.
Resolution occurs only through an authorized EffectPort/LaunchTicket;
unauthorized resolution returns `endpoint-resolve-denied`. Producer restart
bumps `Endpoint.status.endpointGeneration`, which triggers `dependency-changed`
for consumers.

The runtime sockets tmpfs `Volume` remains the backing store for collector
transport files; it is not the stable endpoint identity.

Representative owned Endpoint resources:

```yaml
apiVersion: resources.d2bus.org/v3
type: Endpoint
metadata:
  name: observability-otel-bounded-emitter-ingest
  zone: work
  ownerRef: Provider/observability-otel
spec:
  providerRef: Provider/observability-otel
  producerRef: Process/observability-otel--collector--host-system
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
  name: observability-otel-otlp-ingest
  zone: work
  ownerRef: Provider/observability-otel
spec:
  providerRef: Provider/observability-otel
  producerRef: Process/observability-otel--collector--host-system
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
  ownerRef: Provider/observability-otel
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
  identify the stable ingest service.
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

When `Provider/observability-otel` is **installed but collector not yet `Ready`**
(startup transient):

1. Same emitter drop behavior as absent (socket does not yet exist).
2. Core derives Provider phase `Pending` from collector Process health; core
   propagates `telemetry-export-unavailable` (reason: `CollectorStarting`) as a
   Zone-visible condition per standard Provider phase rules.
3. Doctor reports `telemetry: { "phase": "buffering" }`.
4. When collector `Process` becomes `Ready` and socket is created, core clears
   the condition and Provider phase returns to `Ready`.

When `Provider/observability-otel` is **installed and collector fails or exporter
has an outage**:

1. Core derives Provider phase `Degraded` from collector Process health.
2. See §Exporter outage for detailed behavior.

---

## Provider status

`Provider/observability-otel` uses the **common resource status**. Core derives
and writes the status by aggregating component (Process and Volume) health.
The observability-otel controller does **not** write Provider status.

Per D088, core writes the Provider universal `ResourceStatus` base at top-level
`status.*` and any Provider ResourceType-common aggregate under
`Provider.status.resource` (core-derived per D085). The observability-otel
controller writes status only for collector/forwarder Process resources it owns:
Process-common observation lives in `Process.status.resource`, while bounded
non-secret OTEL-specific export, ingestion, or backpressure detail lives in
`Process.status.provider` with `providerRef: Provider/observability-otel`,
qualified schema IDs such as `observability-otel.d2bus.org/Process/status`,
`schemaVersion` (semver MAJOR.MINOR), `observedProviderGeneration`, and a strict
unknown-field-denied, ≤32 KiB, redacted `details` object registered and signed
in the Provider manifest. The controller writes all present layers atomically in
one status mutation; shared fields are promoted to `status.resource` and never
duplicated into `status.provider`.

D091 currency and upgrade: the observability-otel controller implements
`assess_update`, `plan_upgrade`, and `execute_upgrade`. A
`ProviderGenerationChanged`, collector `ArtifactChanged`, `DependencyChanged`,
or `SpecChanged` reason populates universal `status.update` with
`UpdateAvailable` or `UpgradeRequired`; disruptive changes MUST return
`UpgradeRequired` rather than being applied in place, while
non-disruptive changes reconcile normally. These currency fields are
universal/ResourceType base fields, never `status.provider`. Upgrades recycle
the collector `Process` and any owned forwarder `Process` resources with
`disruption` set to `Reload`, `Restart`, or `Recycle`; telemetry is best-effort
and lossy, so drain does not guarantee zero data loss beyond bounded graceful
stop. No telemetry payload, secret, path, credential, or handle may appear in
`status.update`.

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

**Core-derived phase rules:**

| Phase | Core derivation rule |
| --- | --- |
| `Pending` | Core has created the telemetry Volume and collector Process; collector Process phase is not yet `Ready`. |
| `Ready` | Collector Process phase is `Ready`; telemetry Volume phase is `Ready`. |
| `Degraded` | Collector Process phase is `Degraded` (restart loop or export failures); or telemetry Volume phase is `Degraded`. Forwarder Process `Degraded` is reported as a per-component condition but does not alone transition Provider to `Degraded`. |
| `Failed` | Collector Process phase is `Failed` (max restarts exhausted); or telemetry Volume phase is `Failed`. |
| `Unknown` | Core cannot compute component status. |

**Core-derived conditions** (surfaced from component health events):

| Condition type | Source | Reason values |
| --- | --- | --- |
| `collector-ready` | Collector Process phase | `CollectorRunning`, `CollectorStarting`, `CollectorFailed` |
| `telemetry-export-unavailable` | Collector component health report | `ExporterOutage`, `BackpressureLimit`, `TransportUnresolved` |
| `exporter-backpressure` | Collector component health report | `QueueFull`, `BatchTimeout` |
| `journald-ingestion-active` | Collector component health report | `Enabled`, `Disabled`, `FilterError` |
| `volume-ready` | Telemetry Volume phase | `VolumeReady`, `VolumeNotReady`, `VolumeFailed` |

**Component health reports.** The collector process emits structured component
health events through the standard telemetry/process health path (not Provider
status writes). Core observes these events and reflects them as Provider
conditions. The exporter health gauge (`d2b_otel_exporter_failures_consecutive`
reaching `failureThreshold`) triggers a `telemetry-export-unavailable` component
health event.

**No Provider-owned Zone health writes.** `Provider/observability-otel` does not
write Zone health or set Zone-level conditions directly. Core derives Zone health
from Provider phase per standard rules.

**No `no_isolation` field in Provider status.** `no_isolation` is exclusively an
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

The collector re-stamps the same attributes from incoming frames before export,
preserving existing `vm.name`, `vm.env`, `vm.role`, `host.name`,
`deployment.environment`, and `service.namespace` fields from legacy emitters.
The closed allowlist from `ADR-046-telemetry-audit-and-support`
§OTEL resource attributes governs what may appear.

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
beyond those defined in `ADR-046-telemetry-audit-and-support`. Its process
lifecycle (launch, stop, adoption, quarantine) generates standard `ProcessEffect`
records via the Zone audit sink. Credential resolution generates `RouteAdmission`
records via the bus admission path. Provider resource mutations generate
`ResourceMutation` records via the store write path.

The observability-otel Provider **never** reads from the authoritative audit
sink. OTEL telemetry and audit are strictly separated subsystems; audit data
never enters the OTEL pipeline.

---

## Exporter outage and backpressure behavior

### Exporter outage

When the OTLP export call to the backend fails at the OTEL SDK level (transport
has already delivered or raised an error to the SDK):

1. The OTEL SDK export call returns an error.
2. `d2b_otel_export_batch_total{outcome="error"}` increments.
3. `d2b_otel_exporter_failures_consecutive` gauge increments.
4. The SDK applies its internal backoff and retry (governed by `otlpExporter`
   batch settings). Network/TLS retry at the transport layer is the transport
   Provider's responsibility and is not configurable here.
5. If the SDK exhausts retries, the batch is dropped:
   `d2b_otel_export_batch_total{outcome="dropped"}` increments;
   `d2b_telemetry_drop_total{reason="export_error"}` increments.
6. If `d2b_otel_exporter_failures_consecutive` ≥ `failureThreshold`:
   - Collector emits a `telemetry-export-unavailable` component health event
     (reason: `ExporterOutage`); core reflects this in Provider conditions.
   - Core derives Provider phase `Degraded`.
   - `d2b zone doctor` reports `telemetry: { "phase": "unavailable" }`.
7. On next successful export: `d2b_otel_exporter_failures_consecutive` resets
   to 0; collector clears the component health event; core clears the condition
   and derives Provider phase `Ready`.

Zone mutations, reconciliation, process launch, and authoritative audit are
unaffected throughout.

### Backpressure

When the OTLP SDK export queue reaches `maxQueueSize`:

1. `d2b_otel_backpressure_active` gauge is set to 1.
2. New incoming frames from `emitter.sock` that cannot be enqueued are dropped
   from the emitter ring in FIFO order.
3. `d2b_telemetry_drop_total{reason="buffer_full"}` increments per dropped frame.
4. Collector emits `exporter-backpressure` component health event (reason:
   `QueueFull`); core reflects this in Provider conditions.
5. Core derives Provider phase `Degraded` (not `Failed`).
6. When queue depth drops below `maxQueueSize * 0.75` (75% watermark):
   - `d2b_otel_backpressure_active` is reset to 0.
   - Collector clears the component health event; core clears the condition.
   - Core derives Provider phase `Ready` (if no other Degraded conditions).

The emitter socket drain loop continues accepting frames during backpressure; it
simply discards frames that cannot be enqueued rather than blocking.

---

## Lifecycle, drain, and restart

### Startup sequence

1. Core reads the signed `ProviderDeployment` descriptor from the installed
   `provider-observability-otel` artifact when `Provider/observability-otel`
   reaches `Ready` spec in the Zone store.
2. Core creates the telemetry socket directory `Volume`
   (`observability-otel--collector--sockets--host-system`) from the static
   deployment graph.
3. Core creates the collector `Process`
   (`observability-otel--collector`) from the static deployment graph.
4. ProviderSupervisor (via system-minijail) launches the collector binary.
5. Collector binary opens `emitter.sock` (SOCK_DGRAM) and `otlp.sock`
   (SOCK_STREAM) in the Volume mount at `/run/d2b-otel`.
6. Collector initializes the OTEL SDK (metrics, traces, logs pipelines) and
   resolves the transport alias connection handle.
7. Collector begins drain loop; reports readiness through the standard Process
   readiness path.
8. Core observes collector `Process` phase `Ready` → derives Provider phase
   `Ready`; aggregates to Provider status.

In parallel, the observability-otel controller loop starts:
- Watches `Guest/*` in the Zone.
- For each Guest, creates a vsock-forwarder `Process` child (see
  §otel-vsock-forwarder Process template).

During steps 1–7, Zone/core processes emit to `emitter.sock`; socket does not
yet exist; `BoundedEmitter` drops with `d2b_telemetry_drop_total` increments.
Zone startup and audit are unaffected.

### Drain and graceful stop

On ordered stop (SIGTERM received by the collector binary):

1. Collector enters draining phase; emits `collectorPhase=draining` component health event.
2. Accept no new writes on `emitter.sock` (close the socket; new writes from
   core processes will fail and increment `d2b_telemetry_drop_total`).
3. Drain the remaining emitter ring contents; encode and enqueue for OTLP export.
4. Flush all pending SDK export batches; timeout = `spec.config.otlpExporter.batchExportTimeoutMs`.
5. After flush completes or timeout expires, exit cleanly.
6. The owning Process controller receives exit; sets Process `Succeeded` or
   `Failed` based on exit code.

Drain timeout is enforced by the ProviderSupervisor via a SIGKILL if the
collector does not exit within `drainTimeout` (default 60 s; bounded 10..300 s;
configurable via `spec.config.otlpExporter.drainTimeoutMs`).

### Restart

On unexpected exit:

1. Process controller detects exit via pidfd wait notification.
2. `ProcessEffect{event: "stop", exit_class: "exited|signaled|killed"}` audit
   record is emitted.
3. `d2b_process_restart_total{provider="observability-otel"}` increments.
4. Process controller applies bounded exponential backoff restart; max 5 restarts
   in 10 min (per restart spec in the Process template).
5. Emitter ring continues accumulating frames during restart; core derives
   Provider phase `Degraded` from collector Process `Degraded` phase.
6. After max restarts exhausted, Process phase → `Failed`; core derives Provider
   phase `Failed`; Zone health derives from Provider phase per standard rules.

### Zone stop

On Zone-level shutdown (all Processes stopped):

1. Provider lifecycle controller receives `deletion-requested` hint (as part of
   Zone teardown).
2. Follows the §Cleanup sequence from `ADR-046-telemetry-audit-and-support`:
   vsock-forwarders stopped first; collector stopped with drain; Volume deleted.

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
| `transport` | Provider resolved from `spec.config.export.exportTransportProviderRef` | OTLP network/TLS plumbing and all auth/Credential operations |
| `volume` | `Volume/observability-otel--collector--sockets--host-system` | Telemetry socket directory Volume |

The `credential` alias is absent: the transport Provider/export alias owns all
auth, TLS, and endpoint details, including any Credential operation. The
observability-otel Provider never acquires or routes Credential bytes.

### Required RBAC verbs

The observability-otel component descriptor declares these minimum permission
claims:

| ResourceType | Verb | Purpose |
| --- | --- | --- |
| `Provider` (self) | `get`, `watch` | controller reads own spec for config changes |
| `Provider` (transport) | `get`, `watch` | resolve and monitor `export.exportTransportProviderRef` |
| `Process` | `get`, `watch` | read own forwarder Process status |
| `Process` | `create`, `update`, `delete`, `finalize` | controller manages per-Guest forwarder Process children |
| `Volume` | `get`, `watch` | controller reads telemetry socket directory Volume status |
| `Guest` | `get`, `list`, `watch` | controller watches for Guests needing vsock-forwarder |

Core creates the telemetry `Volume` and collector `Process` and does not require
these verbs from the controller. No `Zone`, `Credential`, `Role`, `RoleBinding`,
`Host`, `User`, `Network`, or `Device` verbs are claimed. No cross-Zone verbs
are claimed. No `Provider (self) status` write is claimed; core writes status.

### Security properties

- The collector process runs as a dedicated UID (`d2b-<zone>-otel`), distinct
  from Zone runtime, core-controller, and all other Provider UIDs.
- No host network access; vsock is the only permitted network-layer transport
  to the obs Zone.
- No read access to `/nix/store`, host paths, other VM/Zone state directories,
  or broker sockets.
- Sandbox: `system-minijail` or `system-systemd` enforcement; `noNewPrivileges: true`;
  `capabilityClasses: []` (zero capabilities). Neither the collector nor the
  forwarder requires `CAP_NET_BIND_SERVICE`; they use Unix/vsock sockets bound
  only to Volume-private paths and Zone-allocated vsock ports.
- Credential bytes are never held, routed, or processed by this Provider; all
  auth material stays inside the transport Provider's scope.
- Redaction filter (`src/redaction.rs`) is applied to all forwarded data before
  export; it runs before the OTLP SDK batching step.

---

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
# Provider resource: installs observability-otel in this Zone.
# Zone.spec = {} -- ring sizing and telemetry config live in Provider spec.config only.
d2b.zones.work.resources.observability-otel = {
  type = "Provider";
  spec = {
    artifactId = "provider-observability-otel";
    config = {
      # Host where the collector Process runs (defaults to Zone primary Host).
      executionRef = "Host/host-system";
      export = {
        # Transport Provider owns endpoint addressing, TLS, network retry, and auth.
        # Any Credential operations for auth are the transport Provider's concern.
        exportTransportProviderRef = "Provider/transport-vsock";
        exportTargetAlias          = "signoz-otlp";
        failureThreshold           = 5;
      };
      emitter = {
        ringCapacityBytesMetrics = 4194304;
        ringCapacityBytesTraces  = 4194304;
        ringCapacityBytesLogs    = 2097152;
      };
      otlpExporter = {
        batchMaxExportSize    = 512;
        batchScheduleDelayMs  = 5000;
        batchExportTimeoutMs  = 30000;
        maxQueueSize          = 2048;
        compressionEnabled    = true;
      };
      journald.enable = false;
      selfMetrics.enable = true;
    };
  };
};
```

### Canonical ResourceSpec JSON shapes (Nix bundle output)

**Provider/observability-otel bundle input** (keys sorted alphabetically):

```json
{
  "apiVersion": "resources.d2bus.org/v3",
  "metadata": {
    "name": "observability-otel",
    "zone": "work"
  },
  "spec": {
    "artifactId": "provider-observability-otel",
    "config": {
      "emitter": {
        "ringCapacityBytesLogs": 2097152,
        "ringCapacityBytesMetrics": 4194304,
        "ringCapacityBytesTraces": 4194304
      },
      "executionRef": "Host/host-system",
      "export": {
        "exportTargetAlias": "signoz-otlp",
        "exportTransportProviderRef": "Provider/transport-vsock",
        "failureThreshold": 5
      },
      "journald": { "enable": false },
      "otlpExporter": {
        "batchExportTimeoutMs": 30000,
        "batchMaxExportSize": 512,
        "batchScheduleDelayMs": 5000,
        "compressionEnabled": true,
        "maxQueueSize": 2048
      },
      "selfMetrics": { "enable": true }
    }
  },
  "type": "Provider"
}
```

Note: no `secretValue`, `token`, `password`, `key`, `endpoint`, CID, port, or
Credential bytes appear. Network addressing and Credential operations belong to
the transport Provider's private config.

**Persisted record after activation** (core adds `uid`, `revision`, `generation`,
`finalizers`, `managedBy`, `configurationGeneration`):

```json
{
  "apiVersion": "resources.d2bus.org/v3",
  "metadata": {
    "configurationGeneration": 1,
    "finalizers": ["observability-otel/lifecycle"],
    "generation": 1,
    "managedBy": "configuration",
    "name": "observability-otel",
    "ownerRef": null,
    "revision": 1,
    "uid": "01960000-0001-7000-8000-000000000001",
    "zone": "work"
  },
  "spec": { "...": "see bundle input above" },
  "type": "Provider"
}
```

### NixOS eval and build validation

**Eval-time assertions** (via generated option types from Provider schema):

1. `spec.config.executionRef` must match `^Host/[a-z][a-z0-9-]*$` and must
   resolve to a declared Host in the same Zone; missing Host fails eval with
   `unresolved-execution-ref`.
2. `spec.config.export.exportTransportProviderRef` must match
   `^Provider/transport-[a-z][a-z0-9-]*$`; otherwise fail with
   `invalid-transport-provider-ref`.
3. `spec.config.export.exportTransportProviderRef` must resolve to an entry in
   `d2b.zones.<zone>.resources` with `type = "Provider"`; missing Provider fails
   eval with `unresolved-transport-provider`.
4. `spec.config.export.exportTargetAlias` must match `^[a-z][a-z0-9-]{0,63}$`;
   otherwise fail with `invalid-export-target-alias`.
5. `spec.config.export` must not contain any `credentialRef`, `token`, `apiKey`,
   `password`, `endpoint`, `host`, `port`, `cid`, or TLS field; these belong in
   the transport Provider's private config. Any such field fails eval with
   `forbidden-export-field`.
6. All numeric config values validated against their stated ranges (compile-time
   generated option bounds).
7. Unknown `spec.config.*` fields rejected by generated option type (no `freeformType`).
8. Any `spec.config` leaf value matching a secret-heuristic pattern
   (contains `password`, `token`, `apiKey`, `secret`, `-----BEGIN`)
   fails eval with a forbidden-inline-secret assertion.
9. Duplicate `resources.<name>` entries with the same `type = "Provider"` and
   `<name> = "observability-otel"` fail with a duplicate-name assertion.
10. `artifactId` must resolve to an entry in `d2b.artifacts.*` with
    `type = "provider"`; missing artifact fails eval.

**Build-time validation** (`nixos-modules/resources-bundle.nix`):

1. Provider `spec.config` validated against the signed `resourceTypeSchema`
   embedded in the `provider-observability-otel` package.
2. SHA-256 digest computed for the `spec` object (canonical sorted JSON bytes).
3. Generation digest computed as SHA-256 of sorted per-resource digest list.
4. Bundle emitted as `zone-resources-<zone>.json`; store path is the integrity pin.
5. No raw secret bytes, host paths, argv tokens, or UID strings in any
   `spec` leaf (pattern-checked against the forbidden-field set from
   `packages/d2b-contract-tests/tests/policy_observability.rs`).

**Runtime activation** (core-controller configuration publication handler):

1. Re-validates Provider package identity against installed package.
2. Resolves `spec.config.executionRef` against live Host store; fails closed if
   unresolved.
3. Resolves transport Provider alias from `export.exportTransportProviderRef`;
   fails closed if unresolved or not Ready.
4. Checks `observability-otel` cardinality (at most one per Zone).
5. Validates Provider schema matches installed package schema.
6. On any failure: rejects generation with `generation-rejected` audit record.

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

### Unit and hermetic Cargo tests (`tests/`)

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
- Assert Zone/controller health fixture transitions to `Degraded` (not `Failed`).
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
- Assert OTEL SDK-level batch retry and backpressure are applied (transport-level
  retry is the transport Provider's responsibility and is not tested here).
- Assert `d2b_otel_export_batch_total{outcome="dropped"}` increments when SDK
  retries exhausted.
- Assert `d2b_otel_exporter_failures_consecutive` gauge increments monotonically.
- Assert Provider status condition `telemetry-export-unavailable` is set
  (`status: "True"`, `reason: "ExporterOutage"`) after `failureThreshold`
  consecutive failures.
- Assert Provider phase transitions to `Degraded` (not `Failed`).
- Assert `d2b_telemetry_drop_total{reason="export_error"}` increments for
  each dropped batch.
- Restore mock OTLP server to healthy; assert `d2b_otel_exporter_failures_consecutive`
  resets to 0; assert `telemetry-export-unavailable` condition cleared; assert
  Provider phase returns to `Ready`.
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
- Assert Provider condition `exporter-backpressure` is set
  (`status: "True"`, `reason: "QueueFull"`).
- Assert Provider phase is `Degraded` (not `Failed`) during backpressure.
- Unblock the mock OTLP server; allow all queued batches to drain.
- Assert `d2b_otel_backpressure_active` resets to 0 when queue depth drops
  below the 75% watermark.
- Assert `exporter-backpressure` condition cleared.
- Assert Provider phase returns to `Ready`.
- Assert drain loop continues accepting frames from the socket throughout the
  backpressure period (no socket closure).
- Assert no audit records or Zone mutations are affected.

#### `tests/bundle_contract.rs`

**Source:** specified in `ADR-046-telemetry-audit-and-support`
§Nix configuration and resource bundle tests.

- `bundle_is_sorted_canonically`: render a two-resource bundle (Zone + Provider);
  assert JSON keys at every level are in ascending alphabetical order; assert
  resources are sorted by `(type, name)`.
- `bundle_digest_is_deterministic`: render the same config twice; assert the
  generation digest round-trips identically.
- `bundle_contains_no_secret_values`: set an `executionRef` and transport config;
  assert the rendered JSON contains no key named `secretValue`, `password`,
  `token`, `key`, or any value matching a secret pattern.
- `bundle_schema_validates_against_provider_schema`: assert the rendered
  `Provider/observability-otel` spec validates against the `resourceTypeSchema`
  output from the declared Provider package.

#### `tests/controller_conformance.rs`

**Source:** new; required by standard `d2b-provider-toolkit` conformance suite.

- Drive the lifecycle controller with a fake Zone store and fake ProviderSupervisor.
  Core (not the controller) creates the runtime sockets Volume (`kind: tmp`) and
  the collector Process; inject them as pre-existing resources. No Provider state
  Volume exists.
- Assert: controller does NOT emit Create, Update, or Delete operations for any
  Volume; the sockets Volume is core's static deployment responsibility.
- Assert: `Volume` does not appear in the controller's exported ResourceType
  permissions; the controller holds watch-status authority only.
- Assert: the collector declares no Provider state Volume; the
  ProviderStateSet query (`ownerRef == Provider/observability-otel`) returns
  zero Volumes.
- Assert: no cross-component Volume sharing; the runtime sockets Volume view is
  not reachable from any forwarder Process mount outside its declared view.
- Assert: controller does NOT write Provider status; that is core's aggregation.
- Assert: controller creates vsock-forwarder `Process` (with `providerRef:
  Provider/system-minijail`, canonical mount to `view: forwarder-write`, and
  `processClass: worker`) on new `Guest/*` resource.
- Assert: forwarder Process spec has `capabilityClasses: []` and
  `startRoot: false`.
- Assert: forwarder has one owned
  `Endpoint/observability-otel-vsock-ingest-<guest-uid-short>` with
  `producerRef` pointing at the forwarder Process and no d2b-bus endpoint.
- Assert: controller handles `deletion-requested` hint: sets desired lifecycle to
  `Stopped` on all vsock-forwarder `Process` children and waits for finalizers;
  final deletion uses event-only Deleted-phase revision + post-commit audit.
- Assert: all mutations use `ResourceMutationBatch` with expected-revision
  preconditions.
- Assert: stale-revision conflicts trigger bounded requeue with backoff.
- Assert: idempotent reconcile: second reconcile with unchanged state emits no
  additional mutations.
- Assert: desired lifecycle set to `Stopped` on `Guest/*` deletion.

#### `tests/provider_state.rs`

**Source:** new; required by `ADR-046-provider-state` status-first invariants.

- `no_provider_state_volume`: drive a fake Zone store with the Provider
  installed; assert `ProviderStateSet(zone, "observability-otel")` is empty (the
  collector declares no state Volume) and no
  `observability-otel--collector--runtime-state--*` Volume exists.
- `operational_state_in_status`: assert the collector's bounded non-secret
  operational state (readiness, export/backpressure reconcile stage, bounded
  drop/queue counters, closed-enum error detail) is written to the owning
  resource's `status` subresource within the frozen status bounds and carries no
  secret/path/argv/PID/unit content.
- `sockets_volume_is_core_created`: assert the runtime sockets Volume
  (`kind: tmp`) exists with core as creator and is not a Provider state Volume.
- `no_cross_component_volume_sharing`: assert no forwarder Process mount points
  to any state Volume; the sockets Volume view `forwarder-write` covers only the
  socket path.
- `restart_re_derives_status`: assert that on controller restart the collector
  readiness is re-derived from live `status` observation and reverified against
  the running process, treating status as observation, never authority.
- `volume_not_in_controller_exported_types`: assert the controller's exported
  ResourceType permission set does not include `Volume` as an owned or
  writable type; only watch-status is present.

#### `tests/config_schema.rs`

**Source:** new; adapts `policy_observability.rs` pattern.

- Assert: `executionRef` with non-Host format is rejected at config validation.
- Assert: `exportTransportProviderRef` with invalid format is rejected with
  `invalid-transport-provider-ref` error code.
- Assert: `exportTargetAlias` with invalid format (e.g., uppercase) is rejected.
- Assert: `export` block with any `credentialRef`, `endpoint`, `host`, `port`,
  `cid`, or TLS field is rejected with `forbidden-export-field` error code.
- Assert: numeric out-of-range values are rejected (e.g., `maxQueueSize = 0`).
- Assert: unknown config field is rejected.
- Assert: inline secret value in any config leaf is rejected.

#### `tests/redaction.rs`

**Source:** new; extends `policy_telemetry_redaction.rs` pattern from
`ADR-046-telemetry-audit-and-support`.

- `redaction_drops_no_isolation_attribute`: inject a span with attribute
  `no_isolation = true`; assert it is removed by the redaction filter before
  forwarding; assert `d2b_otel_frames_decoded_total{outcome="error"}` increments.
- `redaction_drops_forbidden_resource_attribute`: inject a frame with resource
  attribute key outside the allowlist; assert frame is dropped.
- `redaction_drops_path_span_attribute`: inject a span with attribute `path =
  "/run/d2b/..."``; assert it is removed.
- `redaction_drops_realm_field_in_log`: inject a structured log record with field
  `realm = "dev"`; assert the `realm` key is absent from the forwarded record.
- `redaction_passes_allowed_resource_attributes`: inject a frame with resource
  attributes from the v3 allowlist only; assert all attributes are forwarded
  unchanged.

### Integration tests (`integration/`)

#### `integration/scenario_full_pipeline.rs`

End-to-end: fake Zone store → observability-otel controller → collector process
(real binary) → mock OTLP server.

- Bootstrap: start fake Zone store with `Zone/dev` + `Provider/observability-otel` resources.
- Start controller; assert Provider reaches `Ready`.
- Send 200 mixed frames (metrics, traces, logs) via the emitter socket.
- Assert mock OTLP server receives all 200 frames, partitioned by signal type.
- Assert `d2b.zone = "dev"` resource attribute on all received OTLP records.
- Assert `d2b.provider = "observability-otel"` resource attribute.
- Assert `no_vm_label_in_metrics`: no received batch carries a metric label named
  `vm` with a resource-name value.
- Assert self-metrics ComponentSession service returns correct `d2b_otel_frames_received_total`
  counts.
- Drive `d2b zone doctor` fixture; assert `telemetry.phase = "ok"`.

#### `integration/scenario_obs_zone_forwarding.rs`

vsock-forwarder path: Guest → vsock → host forwarder → `otlp.sock` → collector
→ mock OTLP backend.

- Bootstrap: `Zone/dev` + `Provider/observability-otel` + `Guest/dev-vm` resources.
- Assert controller creates vsock-forwarder long-lived `Process` for `Guest/dev-vm`.
- Start vsock-forwarder binary; open connection to simulated vsock endpoint.
- Send 50 OTLP/gRPC frames from the simulated Guest-side collector.
- Assert mock OTLP server receives all 50 frames.
- Assert `d2b_otel_vsock_forwarder_active` gauge is 1 during test; 0 after Guest
  deletion.
- Remove `Guest/dev-vm` resource; assert controller stops the vsock-forwarder
  `Process` and its resource transitions to `Succeeded`.

#### `integration/scenario_provider_removal.rs`

Config-owned cleanup: remove Provider from Nix config; assert ordered child
deletion and Zone status.

- Bootstrap: activate generation 1 with `Provider/observability-otel` in Zone.
- Activate generation 2 without `Provider/observability-otel`.
- Assert `Provider/observability-otel` receives `metadata.deletionRequestedAt`.
- Assert `deletion-pending` condition is set on Provider.
- Assert Zone `pending-cleanup` condition is set; Zone phase is `Degraded`.
- Assert collector `Process` is stopped and deleted by core (not by the
  observability-otel controller), and that the controller does not emit a Delete
  on the core-owned `Process`.
- Assert the telemetry socket directory `Volume` is deleted by core after the
  collector Process exits; it is not deleted by the observability-otel
  controller. No Provider state Volume exists to delete.
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
| Behavior retained | Socat-forwarding vsock pattern (adapted to native binary); `OtelHostBridgeReadiness::{Ready,Pending,Failed}` state machine (adapted to Provider phase); `ingressSources` per-source model (adapted to per-Zone name); ACL/socket directory pattern from `host.nix`; `scrapeJournal` option (preserved, now per-Zone); SigNoz stack shape from `stack.nix`; `loki_native_otel_resource_attributes` allowlist (extended); `startup_tracing_avoids_host_path_fields` redaction policy; `tempo_stack_signoz_backend_and_collector` backend-only assertion |
| Required delta | One `d2b-provider-observability-otel` crate with all required paths; full OTEL SDK linkage in collector binary only; native vsock OTLP/gRPC forwarder replacing socat; per-Zone `emitter.sock` + `otlp.sock` datagram sockets; lifecycle controller; Volume child for socket directory; transport Provider dependency alias (`exportTransportProviderRef`); Provider phase/condition model; exporter outage/backpressure handling; self-metrics ComponentSession service on d2b-bus; journald cgroup filter; Nix per-Zone `ingressSources`; updated `policy_observability.rs` allowlist |
| Reuse path | Adapt `OtelHostBridgeArgvInputs` vsock forwarder logic into `src/forwarder_bin.rs` (extract, replace socat exec with native Rust OTLP/gRPC relay); adapt `OtelHostBridgeReadiness` state machine into Provider phase controller in `src/controller.rs`; copy `otelRuntimeDir` ACL/socket pattern from `host.nix` into `src/controller.rs` Volume spec; adapt `ingressSources` per-VM configuration in `stack.nix` to per-Zone in Provider Nix config; adapt `scrapeJournal` journald receiver from `host.nix` into `src/nix/journald.nix`; adapt `policy_observability.rs` tests (retain all existing assertions; add v3 allowlist extensions and `no_isolation` gate) |
| Replacement/deletion | `otel_host_bridge_argv.rs` socat runner retired after `Provider/observability-otel` delivers native OTLP/vsock and passes conformance. `otel_host_bridge_readiness.rs` readiness gate retired after Provider phase lifecycle is live. `ProcessRole::OtelHostBridge` and `RunnerRole::OtelHostBridge` retired from `d2b-core/src/processes.rs` and `d2b-contracts/src/broker_wire.rs` after Provider migration. `nixos-modules/components/observability/guest.nix` per-VM guest collector retired after vsock-forwarder long-lived Process parity. |
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
| Reuse action | adapt (`OtelHostBridgeArgvInputs` vsock logic → native Rust OTLP relay); adapt (`OtelHostBridgeReadiness` state machine → Provider phase model); delete-after-cutover (`RunnerRole::OtelHostBridge`, `ProcessRole::OtelHostBridge`) |
| Destination | `packages/d2b-provider-observability-otel/src/forwarder_bin.rs`; `packages/d2b-provider-observability-otel/src/controller.rs` (readiness/phase logic) |
| Detailed design | vsock-forwarder binary: open vsock server socket at Zone-allocated port; accept OTLP/gRPC connections from Guest-side collector; relay OTLP proto frames to `otlp.sock` in the Volume mount via `forwarder-write` view; no OTEL SDK; bounded record size (max 4 MiB per frame); session timeout. Controller: adapts `OtelHostBridgeReadiness` three-state model to per-Guest forwarder `Process` lifecycle; collector readiness is reported by collector binary through the standard Process readiness path (not a path check in the spec). |
| Integration | Controller creates vsock-forwarder long-lived `Process` → ProviderSupervisor → system-minijail/systemd launch → vsock socket bind → Guest side connects |
| Data migration | Full reset; existing socat bridge retired after cutover |
| Validation | `integration/scenario_obs_zone_forwarding.rs`; adapted `minijail_relay_otel.rs` shape test for Provider-managed runner; assert `RunnerRole::OtelHostBridge` is absent from `d2b-contracts` after removal |
| Removal proof | `otel_host_bridge_argv.rs`, `otel_host_bridge_readiness.rs`, and `RunnerRole::OtelHostBridge` / `ProcessRole::OtelHostBridge` removed only after `integration/scenario_obs_zone_forwarding.rs` passes and Provider phase lifecycle is tested |

### ADR046-otel-002

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-otel-002` |
| Dependency/owner | ADR046-otel-001 + ADR046-telem-001 + ADR046-provider-001 (Provider toolkit) + a transport Provider (`ADR-046-provider-transport-unix`); W2; observability owner |
| Current source | `nixos-modules/components/observability/host.nix` (`otelRuntimeDir`, `hostEgressSocket`, `setfacl` ACL pattern, `scrapeJournal` option, `identityName`); `nixos-modules/components/observability/stack.nix` (`ingressSources`, `vmName`, `receiverGrpcPort`, loopback binding, `signoz.listenPort`) |
| Reuse action | adapt Nix pipeline shape (replace per-VM `vmName` with per-Zone name; replace socat runner with vsock-forwarder long-lived Process; adapt `ingressSources` per-Zone entry) |
| Destination | `packages/d2b-provider-observability-otel/src/collector_bin.rs`; `packages/d2b-provider-observability-otel/src/emitter_socket.rs`; `packages/d2b-provider-observability-otel/src/exporter.rs`; `packages/d2b-provider-observability-otel/src/controller.rs` (forwarder management only); updated `nixos-modules/components/observability/{host,stack}.nix` |
| Detailed design | Collector binary: full OTEL SDK initialization (metrics, traces, logs pipelines); OTLP/gRPC exporter connected via transport alias; Unix datagram receiver at `emitter.sock` (drain loop); OTLP/gRPC Unix stream receiver at `otlp.sock`; `d2b.observability.v1.SelfMetrics` ComponentSession service on d2b-bus (local); journald receiver (disabled by default). Exporter: OTEL SDK-level batching and backpressure; `d2b_otel_backpressure_active` gauge; drain-before-shutdown; component health events on outage/backpressure (not Provider status writes). Core (not controller) creates the telemetry sockets Volume and the collector Process from the static ProviderDeployment descriptor; the collector declares no Provider state Volume (bounded non-secret operational state in status/core ledger, D087); controller only manages per-Guest forwarder Processes and does not own or export Volume as a ResourceType. Nix: Zone.spec is empty for observability (`spec = {}`); ring sizing lives in Provider spec.config only; adapt `identityName` to Zone name; adapt `ingressSources` to per-Zone entry; preserve `stack.nix` SigNoz shape. |
| Integration | core `BoundedEmitter` → `emitter.sock` → collector drain loop → OTEL SDK → OTLP/gRPC → vsock → obs Zone SigNoz |
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
| Detailed design | Per `ADR-046-telemetry-audit-and-support` §journald stdout/stderr ingestion: cgroup filter `z-<zone-id>/*` and `s-<execution-id>/*`; redaction: drop `MESSAGE` bodies matching credential/secret/path patterns, `_CMDLINE`, `_EXE`, `INVOCATION_ID`; retain `_SYSTEMD_CGROUP`, `PRIORITY`, `SYSLOG_IDENTIFIER`, and structured `KEY=VALUE` from declared allow-set. `d2b.zones.<name>.observability.journald.enable = false` Nix option (default disabled). |
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

---

## README.md requirement

`packages/d2b-provider-observability-otel/README.md` must document:

1. **Provider identity**: `Provider/observability-otel`, crate name, API major version.
2. **Purpose**: the only place linking `opentelemetry_sdk`; drains Zone `BoundedEmitter` frames and exports via OTLP/gRPC.
3. **Config schema**: all fields, bounds, `executionRef`, `exportTransportProviderRef`, `exportTargetAlias`, forbidden inline secrets; no Credential refs (transport Provider owns auth).
4. **ResourceTypes**: consumed types (Process, Volume, Host, Guest, Provider/transport); Zone.spec is {} — ring sizing lives in Provider config only.
5. **ProviderDeployment**: core creates the runtime sockets Volume (`kind: tmp`) and the collector Process before component Processes start; core deletes the sockets Volume after component finalizers when the Provider is removed. The collector declares no Provider state Volume — its bounded non-secret operational state lives in `status`/the core Operation ledger (D087). Controller manages per-Guest forwarder Processes only and does not create, update, delete, or export any Volume. `Provider/volume-local` is the sole Volume reconciler.
6. **Components**: collector (long-lived Process, `service` class, `providerRef: Provider/system-minijail`), vsock-forwarder (long-lived `Process`, `worker` class, per Guest).
7. **Placement**: both run under the Zone Host as `domain=system` via system-minijail.
8. **Volumes**: the only Volume is the runtime telemetry socket directory (`kind: tmp`, `source.settings.kind: tmpfs`; sockets only), core-created from the `ProviderDeployment` descriptor. The collector declares **no** Provider state Volume; its bounded non-secret operational state lives in `status`/the core Operation ledger (D087). Socket paths are Volume-private. ProviderStateSet is an optional query-time logical grouping (`ownerRef == Provider/observability-otel`), not a ResourceType, and is empty.
9. **Dependencies**: `d2b-telemetry` (BoundedEmitter); `d2b-provider-toolkit` (conformance suite); `opentelemetry_sdk`; `opentelemetry-otlp`; transport Provider alias.
10. **RBAC**: minimum permission claims (this spec §Bus/RBAC section); no Credential or Zone verbs; no Provider status write.
11. **Security**: dedicated UID (User/observability-otel-system); zero capabilities; no host network; no Credential bytes held by this Provider.
12. **Telemetry**: OTEL resource attributes; self-metrics instruments; cardinality rules; component health events (not Provider status writes).
13. **Startup invariant**: Zone startup does not wait for this Provider; absent (not installed) has no Zone health impact; installed-but-unready → core derives Provider `Degraded`.
13. **Exporter outage/backpressure**: behavior, metrics, conditions, recovery path.
14. **Lifecycle/drain/restart**: drain timeout; restart policy; Zone stop sequence.
15. **Build and test commands**: `cargo build -p d2b-provider-observability-otel`; `cargo test -p d2b-provider-observability-otel`; `make test-integration` for integration scenarios.
16. **Standalone consumption**: how an external repository consumes this Provider package, including Nix flake input and artifact catalog registration pattern.
