# ADR 0046 Provider dossier: clipboard-wayland

| Field | Value |
| --- | --- |
| Spec ID | `ADR-046-provider-clipboard-wayland` |
| Version | 2 |
| Parent | ADR 0046 |
| Status | Accepted |
| Baseline | `b5ddbed67867d9244bf33390868101bd9b053e49` |
| Normative | Yes |
| Owners | `Provider/clipboard-wayland` controller |
| Depends on | `ADR-046-provider-model-and-packaging`, `ADR-046-componentsession-and-bus`, `ADR-046-resources-host-guest-process-user`, `ADR-046-provider-state`, `ADR-046-nix-configuration`, `ADR-046-telemetry-audit-and-support`, `ADR-046-resource-reconciliation`, `ADR-046-core-controllers`, [ADR 0042](../../adr/0042-d2b-clipboard-authority-and-picker-split.md) |
| Supersedes | `nixos-modules/clipboard.nix` (v3 migration), ADR-046-provider-clipboard-wayland v1 |

---

## Purpose

`Provider/clipboard-wayland` delivers the split-trust clipboard bridge for
NixOS desktop microVM hosts running Wayland-only compositors. It exposes
controlled, policy-audited clipboard transfer between the host compositor
and one or more Guest VMs while upholding the invariants established in
ADR 0042:

- clipboard bytes never appear in resources, status, audit, or logs;
- picker UI runs as an isolated EphemeralProcess without clipboard FDs;
- FD transfer uses SCM_RIGHTS over ComponentSession-authenticated transport only;
- all compositor integration flows through the mandatory `Provider/display-wayland`
  typed dependency;
- no direct compositor socket (WAYLAND_DISPLAY, NIRI_SOCKET) connections from
  clipboard-wayland components;
- no shared filesystem bridge, no per-Guest Unix socket groups or ACLs.

This dossier is normative for the clipboard-wayland crate, Nix module,
ResourceType schema, ComponentSession services, RBAC, lifecycle, audit
format, and test requirements.

---

## Scope

- Host component: `clipd-host` user-domain service Process
- Host component: `clipboard-controller` system-domain controller Process
- Guest component: none (Guest clipboard access is mediated through
  `Provider/display-wayland`'s wayland-proxy, which connects to clipd-host
  via the `d2b.clipboard.bridge.v3` service)
- Typed dependency: `Provider/display-wayland` (alias: `display`)

Out of scope:

- DND drag-and-drop (no DataDeviceManager protocol)
- primary X11 selection (no primary selection protocol)
- cross-Zone clipboard (default-deny; requires explicit future ADR)
- clipboard history persistence (in-memory only)
- Wayland protocol client implementation (delegated to display-wayland)

---

## Architecture overview

```
                   ┌─────────────────────────────────────────────┐
                   │         Provider/clipboard-wayland           │
                   │                                             │
                   │  Process/clipboard-controller               │
                   │  (system-minijail, system domain)           │
                   │  - owns ResourceType/clipboard-wayland-*    │
                   │  - serves d2b.clipboard.picker-coord.v3     │
                   │  - creates EphemeralProcess/picker-<id>     │
                   │              │  (bus: picker coord)         │
                   │              ▼                              │
                   │  Process/clipd-host                         │
                   │  (system-systemd, user domain)              │
                   │  - serves d2b.clipboard.bridge.v3           │
                   │  - serves d2b.clipboard.v3 (management)     │
                   │  - consumes d2b.display.host-clipboard.v3   │
                   └────────────┬──────────────────────┬────────┘
                                │ enrolled KK          │ enrolled KK
                   d2b.clipboard.bridge.v3   d2b.display.host-clipboard.v3
                                │                      │
                   ┌────────────▼──────┐   ┌───────────▼────────┐
                   │ Provider/          │   │ Provider/           │
                   │ display-wayland    │   │ display-wayland     │
                   │ (wayland-proxy)    │   │ (compositor svc)    │
                   └───────────────────┘   └────────────────────┘
```

Key structural rules:

1. **Core creates both Processes.** The `Provider lifecycle` handler in
   `Provider/system-core` creates `Process/clipboard-controller` and
   `Process/clipd-host` from the signed component templates in the
   clipboard-wayland descriptor. The controller does not create itself or
   the service Process.

2. **Controller creates only operation-scoped EphemeralProcesses.**
   `Process/clipboard-controller` creates `EphemeralProcess/picker-<id>` per
   paste request via the resource API. It owns no other child resources.

3. **No shared filesystem IPC.** All inter-component communication uses
   ComponentSession over private local transport with enrolled KK Noise
   authentication. There are no bridge directories, per-Guest socket paths,
   or SO_PEERCRED peer config in sealed configuration.

4. **Core derives Provider status.** The core controller aggregates exact
   child Process/EphemeralProcess statuses into `Provider/clipboard-wayland`
   status. The clipboard-wayland controller does not write Provider status.
   Per D088, that core-owned status uses universal top-level `status.*` plus
   the Provider ResourceType-common `status.resource`; optional
   `status.provider` is only for implementation observation (`providerRef`,
   qualified immutable `schemaId`, semver `schemaVersion`, numeric
   `observedProviderGeneration`, strict unknown-field-denied redacted `details`
   ≤32 KiB registered/signed in the Provider manifest) and never duplicates
   shared fields. Core writes all present layers atomically in one status
   mutation.

5. **Core delivers Guest lifecycle messages.** The orchestrator sends
   authenticated `GuestStopped`, `GuestLocked`, `GuestDestroyed` messages
   directly to the clipboard controller via ComponentSession. The controller
   does not watch `Guest/*` resources broadly.

---

## Provider resource spec

Per D089, provider-owned clipboard ResourceTypes use their typed desired spec
as the ResourceType base spec (Layer 2): top-level `spec.*`, including
`spec.providerRef` where applicable. Any implementation-variant desired
settings use only the canonical Layer 3 `spec.provider = { schemaId,
schemaVersion, settings }` envelope, whose `settings` are
manifest-registered/signed, deny-unknown, bounded, versioned/digested,
validated against `spec.providerRef`, and forbidden to shadow base fields;
shared fields are promoted into the base spec. The owning Provider implements
the exact base spec/status schema version/fingerprint, accepts the canonical
minimal base Spec, and rejects an unsupported optional base capability only
through its signed capability matrix plus typed provider-neutral
`unsupported-capability`. `spec.provider` aligns with `status.provider`. The
`Provider` resource itself remains the D075 `{ artifactId, config }`
exception.

### Operator-authored Nix form

```nix
d2b.zones.dev.resources.clipboard-wayland = {
  type = "Provider";
  spec = {
    artifactId = "clipboard-wayland";   # registered in d2b.artifacts catalog
    config = {
      # Required: execution placement
      hostExecutionRef  = "Host/host-system";
      hostUserRef       = "User/alice";

      # Optional: typed dependency on display-wayland.
      # null = host-only mode (no VM clipboard bridge; dependencyStatus.display = Absent)
      displayWaylandRef = "Provider/display-wayland";

      # Optional: signed picker EphemeralProcess template override.
      # null = use the default picker bundled with clipboard-wayland.
      pickerArtifactId  = null;

      # Policy flags
      policy = {
        allowHostCapture      = true;   # host compositor → clipboard history
        allowGuestCapture     = true;   # VM selection → clipboard history
        requirePickerForPaste = true;   # interactive picker before guest paste
        suppressEcho          = true;   # loop suppression (default true)
        crossZone.enable      = false;  # deny cross-Zone clipboard (default)
      };

      # Capability bounds
      caps = {
        maxHistoryEntries   = 20;       # LRU bound; [1, 200]
        maxItemBytes        = 8388608;  # 8 MiB; [4096, 67108864]
        maxTotalBytes       = 67108864; # 64 MiB total; ≥ maxItemBytes
        maxConcurrentFds    = 32;       # active SCM_RIGHTS FDs in flight; [1, 256]
        maxGuestRatePerMin  = 60;       # materialization requests per Guest per minute
      };

      # Timing
      ttl = {
        pickerTimeoutSeconds  = 20;     # [5, 120]
        fdWriteTimeoutSeconds = 30;     # [5, 120]
        hostEntrySeconds      = 3600;   # clipboard history TTL for host entries
        guestEntrySeconds     = 3600;   # clipboard history TTL for guest entries
      };
    };
  };
};
```

### YAML canonical form

```yaml
apiVersion: resources.d2bus.org/v3
type: Provider
metadata:
  name: clipboard-wayland
  zone: dev
  ownerRef: null      # root config-owned; core configuration publication handler owns it
spec:
  artifactId: clipboard-wayland
  config:
    hostExecutionRef: Host/host-system
    hostUserRef: User/alice
    displayWaylandRef: Provider/display-wayland   # null for host-only mode
    pickerArtifactId: null
    policy:
      allowHostCapture: true
      allowGuestCapture: true
      requirePickerForPaste: true
      suppressEcho: true
      crossZone:
        enable: false
    caps:
      maxHistoryEntries: 20
      maxItemBytes: 8388608
      maxTotalBytes: 67108864
      maxConcurrentFds: 32
      maxGuestRatePerMin: 60
    ttl:
      pickerTimeoutSeconds: 20
      fdWriteTimeoutSeconds: 30
      hostEntrySeconds: 3600
      guestEntrySeconds: 3600
```

### Nix notes

- `spec.config` mirrors the signed Provider component JSON Schema projection.
  There are no `spec.settings`, `spec.componentPlacements`, or `spec.status`
  fields in the Nix authoring form.
- `d2b.artifacts.clipboard-wayland` must be registered in the Zone artifact
  catalog before this resource is declared. The `artifactId` is a plain
  bounded ID, not a ResourceRef.
- `hostExecutionRef` and `hostUserRef` carry placement intent; framework
  derives Process `executionRef`/`userRef` from these fields during
  ProviderDeployment.
- `displayWaylandRef` must be a Ready `Provider/display-wayland` instance in
  the same Zone when non-null. Framework resolves the typed `display`
  dependency alias from this value.
- Ref validation rejects forward references to non-existent resources at
  config-publication time.

---

## Provider typed dependency

The clipboard-wayland manifest declares one dependency alias:

```text
alias: display
type: Provider
serviceContract: d2b.display.host-clipboard.v3
optional: true
```

Zone config binds `display` to the `displayWaylandRef` value from
`spec.config`. When `displayWaylandRef` is null the dependency is absent; the
framework does not fail the Provider but sets `dependencyStatus.display =
Absent` in derived Provider status.

When display-wayland is present and Ready, the framework sets up an enrolled
KK ComponentSession transport from `Process/clipd-host` (consumer) to the
display-wayland `Endpoint/<host-clipboard-service>`. The transport
uses Noise_KK_25519_ChaChaPoly_SHA256 with both static public keys registered
in the Zone identity registry before handshake. Neither component receives a
global route table.

---

## Process resources

### Process/clipboard-controller

Core ProviderDeployment creates this Process from the signed
`clipboard-controller` component template.

```yaml
apiVersion: resources.d2bus.org/v3
type: Process
metadata:
  name: clipboard-controller
  zone: dev
  ownerRef: Provider/clipboard-wayland
spec:
  providerRef: Provider/system-minijail
  executionRef: <spec.config.hostExecutionRef>
  domain: system
  userRef: null
  processClass: controller
  template: clipboard-controller
  configRef: null
  credentialRefs: []
  mounts: []
  sandbox:
    namespaceClasses: [mount, ipc, uts, network]
    capabilityClasses: []
    seccompClass: strict
    noNewPrivileges: true
    startRoot: false
    readOnlyRoot: true
    environmentClass: minimal
  budget:
    cpu:
      request: "50m"
      limit: "500m"
    memory:
      request: "32Mi"
      limit: "64Mi"
    pids:
      limit: 64
    fds:
      limit: 256
  networkUsage: null
  deviceUsage: []
  telemetry:
    metricsEnabled: true
    tracingEnabled: true
    logLevel: info
    sensitiveLabels: false
  desiredLifecycle: running
  restartPolicy:
    class: on-failure
    backoffBase: "1s"
    backoffMax: "60s"
    backoffMultiplierMilli: 2000
    maxRestarts: null
    resetAfter: "300s"
  readiness:
    initialDelay: "0s"
    timeout: "30s"
    failureThreshold: 3
    successThreshold: 1
    class: provider-defined
  healthCheck:
    enabled: true
    interval: "60s"
    timeout: "5s"
    failureThreshold: 3
    class: provider-defined
  adoptionPolicy: adopt-on-restart
  drainTimeout: "30s"
```

The clipboard-controller:

- serves the internal `d2b.clipboard.picker-coord.v3` service on
  `Endpoint/clipboard-picker-coord`;
- creates `EphemeralProcess/picker-<id>` resources via the resource API per paste
  request that requires interactive confirmation;
- receives `GuestStopped`, `GuestLocked`, `GuestDestroyed`, `GuestSuspended`
  lifecycle messages from core orchestrator via ComponentSession and forwards
  corresponding `PurgeZone`/`SuspendZone` instructions to `clipd-host` over
  the `d2b.clipboard.picker-coord.v3` service;
- writes only bounded, redacted operational observations through the optimistic
  status writer for `Provider/clipboard-wayland` and related Operations;
- does not own, export, reconcile, or mount any Provider state Volume. Under
  D087 its ProviderStateSet is empty because clipboard operational state is
  derivable from status, the core Operation ledger, and external observation.

### Process/clipd-host

Core ProviderDeployment creates this Process from the signed
`clipd-host` component template.

```yaml
apiVersion: resources.d2bus.org/v3
type: Process
metadata:
  name: clipd-host
  zone: dev
  ownerRef: Provider/clipboard-wayland
spec:
  providerRef: Provider/system-systemd
  executionRef: <spec.config.hostExecutionRef>
  domain: user
  userRef: <spec.config.hostUserRef>
  processClass: service
  template: clipd-host
  configRef: null
  credentialRefs: []
  mounts: []
  sandbox:
    namespaceClasses: []                  # user-domain; inherits user namespace
    capabilityClasses: []
    seccompClass: strict
    noNewPrivileges: true
    startRoot: false
    readOnlyRoot: true
    environmentClass: provider-defined    # system-systemd user scope provides XDG_RUNTIME_DIR
  budget:
    cpu:
      request: "100m"
      limit: "1000m"
    memory:
      request: "64Mi"
      limit: "256Mi"
    pids:
      limit: 128
    fds:
      limit: 1024
  networkUsage: null
  deviceUsage: []
  telemetry:
    metricsEnabled: true
    tracingEnabled: true
    logLevel: info
    sensitiveLabels: false
  desiredLifecycle: running
  restartPolicy:
    class: on-failure
    backoffBase: "2s"
    backoffMax: "120s"
    backoffMultiplierMilli: 2000
    maxRestarts: null
    resetAfter: "300s"
  readiness:
    initialDelay: "1s"
    timeout: "30s"
    failureThreshold: 3
    successThreshold: 1
    class: provider-defined
  healthCheck:
    enabled: true
    interval: "30s"
    timeout: "5s"
    failureThreshold: 3
    class: provider-defined
  adoptionPolicy: adopt-on-restart
  drainTimeout: "30s"
```

`clipd-host`:

- serves `d2b.clipboard.bridge.v3` on `Endpoint/clipboard-bridge` (consumed by
  display-wayland's wayland-proxy);
- serves `d2b.clipboard.v3` management API on `Endpoint/clipboard-management`
  (consumed by authorized CLI/operator sessions);
- opens one enrolled KK ComponentSession to display-wayland's
  `d2b.display.host-clipboard.v3` service to receive host selection events,
  focus attribution data, and to publish selections back to the host compositor;
- opens one enrolled KK ComponentSession to the clipboard-controller's
  `Endpoint/clipboard-picker-coord` for picker session dispatch and result
  delivery;
- does not connect to WAYLAND_DISPLAY or NIRI_SOCKET directly;
- does not own ResourceTypes;
- holds clipboard history in bounded process memory only (never in any Volume).

---

## EphemeralProcess/picker-session

`Process/clipboard-controller` creates one `EphemeralProcess/picker-<uuid>`
per paste request that requires interactive user confirmation.

```yaml
apiVersion: resources.d2bus.org/v3
type: EphemeralProcess
metadata:
  name: picker-3f7a9c12
  zone: dev
  ownerRef: Process/clipboard-controller   # controller-owned, not Provider-owned
spec:
  providerRef: Provider/system-systemd
  executionRef: <spec.config.hostExecutionRef>
  domain: user
  userRef: <spec.config.hostUserRef>
  processClass: worker
  template: picker-session
  configRef: null                # metadata arrives via named stream from controller
  credentialRefs: []
  mounts: []
  sandbox:
    namespaceClasses: []
    capabilityClasses: []
    seccompClass: strict
    noNewPrivileges: true
    startRoot: false
    readOnlyRoot: true
    environmentClass: minimal    # WAYLAND_SOCKET FD pre-opened by ProviderSupervisor (FD number only)
  budget:
    cpu:
      request: "50m"
      limit: "500m"
    memory:
      request: "16Mi"
      limit: "64Mi"
    pids:
      limit: 32
    fds:
      limit: 64
  networkUsage: null
  deviceUsage: []
  telemetry:
    metricsEnabled: false        # worker; no per-picker metrics stream
    tracingEnabled: true
    logLevel: warn
    sensitiveLabels: false
  startDeadline: "10s"
  runtimeDeadline: "120s"        # picker must complete within 2 min
  successfulTtl: "1h"
  failedTtl: "24h"
  incidentHold: false
```

### Picker invariants

- `processClass` is always `worker`; a controller or service EphemeralProcess
  is rejected at spec admission.
- Picker receives **no clipboard FDs**, no clipboard bytes, no compositor
  credentials, and no NIRI_SOCKET path in sealed config or as SCM_RIGHTS
  attachments.
- Picker metadata (source attribution, MIME list hint, operation ID) arrives
  via a bounded named stream from `Process/clipboard-controller` over an
  inherited-socketpair ComponentSession established at spawn time.
- Picker Wayland access: `d2b.display.host-clipboard.v3` on display-wayland
  exposes a fixed presentation-only compositor portal. At picker spawn,
  ProviderSupervisor pre-opens a restricted Wayland connection FD backed by
  this portal and passes it to the picker with `WAYLAND_SOCKET` set to the
  FD number (not a socket path). GTK4 connects via this FD. The portal
  exposes only the presentation subset of the compositor protocol;
  `zwlr_data_control_manager_v1` and all clipboard-manager globals are absent.
  No compositor credential, socket path, or WAYLAND_DISPLAY path reaches the
  picker process.
- GTK4 and all its runtime dependencies are packaged in the `picker-session`
  artifact's Nix closure. There is no ambient host GTK4 dependency.
- Picker sends exactly one `Select(item_digest)` or `Cancel` message back to
  the controller via the same stream. No clipboard content transits this stream.
- `successfulTtl: "1h"` - completed/cancelled picker retained 1 hour for
  debug correlation.
- `failedTtl: "24h"` - failed picker retained 24 hours for incident hold.
- Controller writes status to the EphemeralProcess resource; never to
  `Provider/clipboard-wayland` directly.

---

## Endpoint resources (D092)

`Provider/clipboard-wayland` declares standard `Endpoint` base-schema
conformance. Stable clipboard service identities are owned `Endpoint` resources
with `producerRef`; they are not inline `Process.spec` fields. Consumers use
`Endpoint/<name>` references. Endpoint spec/status/CLI/audit/telemetry never
include raw socket paths, compositor locators, clipboard bytes, MIME payloads,
fds, or credentials. Resolution occurs only through an authorized
EffectPort/LaunchTicket; unauthorized resolution returns
`endpoint-resolve-denied`. Producer restart bumps
`Endpoint.status.endpointGeneration`, which triggers `dependency-changed` for
consumers.

Representative owned Endpoint resources:

```yaml
apiVersion: resources.d2bus.org/v3
type: Endpoint
metadata:
  name: clipboard-picker-coord
  zone: dev
  ownerRef: Provider/clipboard-wayland
spec:
  providerRef: Provider/clipboard-wayland
  producerRef: Process/clipboard-controller
  endpointClass: service
  transport: unix
  purpose: clipboard-wayland.d2bus.org/picker-coordination
  serviceFingerprint: clipboard-wayland.d2bus.org/picker-coord.v3
  locality: host-local
  visibility: zone
  attachmentPolicy: component-session
  consumerPolicy:
    allowedOperations: [resolve]
  lifecyclePolicy: recycle-with-producer
status:
  resource:
    readiness: Ready
    observedProducerGeneration: 1
    observedResourceGeneration: 1
    endpointGeneration: 1
    connectionAvailability: available
    leaseAvailability: lease-required
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

```yaml
apiVersion: resources.d2bus.org/v3
type: Endpoint
metadata:
  name: clipboard-bridge
  zone: dev
  ownerRef: Provider/clipboard-wayland
spec:
  providerRef: Provider/clipboard-wayland
  producerRef: Process/clipd-host
  endpointClass: service
  transport: unix
  purpose: clipboard-wayland.d2bus.org/bridge
  serviceFingerprint: clipboard-wayland.d2bus.org/bridge.v3
  locality: host-local
  visibility: zone
  attachmentPolicy: component-session
  consumerPolicy:
    allowedOperations: [resolve]
  lifecyclePolicy: recycle-with-producer
status:
  resource:
    readiness: Ready
    observedProducerGeneration: 1
    observedResourceGeneration: 1
    endpointGeneration: 1
    connectionAvailability: available
    leaseAvailability: lease-required
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

```yaml
apiVersion: resources.d2bus.org/v3
type: Endpoint
metadata:
  name: clipboard-management
  zone: dev
  ownerRef: Provider/clipboard-wayland
spec:
  providerRef: Provider/clipboard-wayland
  producerRef: Process/clipd-host
  endpointClass: service
  transport: unix
  purpose: clipboard-wayland.d2bus.org/management
  serviceFingerprint: clipboard-wayland.d2bus.org/management.v3
  locality: host-local
  visibility: zone
  attachmentPolicy: component-session
  consumerPolicy:
    allowedSubjects: [User/alice]
    allowedOperations: [resolve]
  lifecyclePolicy: recycle-with-producer
status:
  resource:
    readiness: Ready
    observedProducerGeneration: 1
    observedResourceGeneration: 1
    endpointGeneration: 1
    connectionAvailability: available
    leaseAvailability: lease-required
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

## Retained opaque handles

- pidfds: kernel supervision handles for Process resources, not stable service
  identities.
- Per-connection/session handles: selection tokens, picker IDs, and
  ComponentSession IDs are high-churn and scoped to one interaction.
- Named streams: clipboard data/control streams carry bounded records or FDs and
  never represent a stable managed endpoint.
- `OwnedTransport`: authenticated session transport ownership remains an
  in-memory capability.
- fd indexes: `WAYLAND_SOCKET` and transfer-FD slots are LaunchTicket-local
  descriptor numbers and stay opaque.

---

## ComponentSession services

### d2b.display.host-clipboard.v3

**Served by:** `Provider/display-wayland` (compositor service component)
**Consumed by:** `Process/clipd-host`
**Profile:** Enrolled KK (`Noise_KK_25519_ChaChaPoly_SHA256`)
**Transport:** enrolled Zone transport established by framework on display-wayland
dependency readiness

Methods and named streams:

| Name | Direction | Class | Description |
| --- | --- | --- | --- |
| `SubscribeSelectionChanges` | clipd-host → display-wayland → clipd-host | named stream | Stream of `HostSelectionChangedEvent` (MIME type list, byte hint, source attribution; no payload bytes). |
| `SubscribeFocusEvents` | clipd-host → display-wayland → clipd-host | named stream | Stream of `HostFocusEvent` (app_id, title, output, workspace; no PII or raw window content). |
| `MaterializeData` | clipd-host → display-wayland | request+attachment | Request data for a given MIME type from current host selection. Response carries attachment class `host-selection-transfer-fd` (one `O_RDONLY` FD, read-once, validated by fstat/fstatfs). |
| `AnnounceSelection` | clipd-host → display-wayland | request | Declare clipd-host as host selection owner for given MIME type list. Returns opaque `SelectionToken`. |
| `ServeDataRequest` | display-wayland → clipd-host | named stream | Reverse stream: display-wayland sends `DataRequest(token, mime_type)` when host compositor needs data from clipd's announced selection. clipd sends `DataResponse` with attachment class `host-selection-supply-fd`. |
| `RevokeSelection` | clipd-host → display-wayland | request | Relinquish the announced selection (SelectionToken required). |

Clipboard bytes appear only as SCM_RIGHTS attachment FDs. They never appear
in method arguments, named stream frames, status fields, audit payloads, or
traces.

### d2b.clipboard.bridge.v3

**Served by:** `Process/clipd-host`
**Consumed by:** `Provider/display-wayland` (wayland-proxy component)
**Profile:** Enrolled KK
**Transport:** inherited private transport via framework enrollment on dependency

This service carries the Guest ↔ Host clipboard bridge. wayland-proxy calls
this service when a Guest selection becomes available or when a paste into a
Guest is authorized.

Methods:

| Name | Direction | Class | Description |
| --- | --- | --- | --- |
| `NotifyGuestSelection` | wayland-proxy → clipd-host | request | Notify clipd-host that a Guest VM has a new selection. Carries: `zone_id`, `guest_name`, MIME type list, byte hint, source attribution. No clipboard payload. Returns `EntryToken`. |
| `FetchGuestData` | clipd-host → wayland-proxy | request+attachment | clipd-host requests clipboard data for a given `EntryToken` and `mime_type`. Response carries attachment class `clipboard-transfer-fd` (one `O_RDONLY` FD). Validated: fstat, fstatfs, MSG_CMSG_CLOEXEC, MSG_CTRUNC = fail-closed. |
| `AuthorizeGuestPaste` | clipd-host → wayland-proxy | request+attachment | clipd-host authorizes a paste to a given Guest. Carries `entry_token`, `mime_type`, attachment class `clipboard-transfer-fd` (one `O_WRONLY` FD, write-once, size-limited). |
| `CancelEntry` | clipd-host → wayland-proxy | request | Cancel a pending `EntryToken`; wayland-proxy releases associated selection offer. |

### d2b.clipboard.picker-coord.v3

**Served by:** `Process/clipboard-controller`
**Consumed by:** `Process/clipd-host`
**Profile:** Local NN (same Host, inherited socketpair)
**Transport:** inherited socketpair at clipd-host startup

Methods:

| Name | Direction | Class | Description |
| --- | --- | --- | --- |
| `RequestPickerSession` | clipd-host → controller | request | Request picker confirmation for a pending paste. Carries: `source_zone_id`, `dest_guest_name`, MIME type list (no payload, no FDs). Returns `picker_session_id`. |
| `NotifyPickerResult` | controller → clipd-host | callback | Controller calls back when picker EphemeralProcess completes. Carries: `picker_session_id`, outcome (`Selected(item_digest)` or `Cancelled` or `TimedOut`). |
| `CancelPickerSession` | clipd-host → controller | request | Cancel a pending picker (Guest disconnected, entry expired, etc.). |
| `PurgeZoneClipboard` | controller → clipd-host | request | Instructs clipd-host to purge all history entries for a given zone/guest. Sent when core delivers `GuestStopped`/`GuestDestroyed`. |
| `SuspendZoneClipboard` | controller → clipd-host | request | Suspend paste authorization for a given zone/guest. Sent on `GuestLocked`/`GuestSuspended`. |

### d2b.clipboard.v3

**Served by:** `Process/clipd-host`
**Consumed by:** authorized CLI sessions and operator tooling
**Profile:** enrolled KK or local NN (operator tool subject)
**Authorization:** `Role/clipboard-admin` (arm/disarm/delete), `Role/clipboard-viewer`
(list/status/events)

Methods:

| Name | Description |
| --- | --- |
| `ArmBridge` | Enable clipboard capture from host and/or guests. |
| `DisarmBridge` | Disable capture; pending FDs are closed. |
| `ListHistory` | Return bounded list of clipboard history metadata entries. No payload bytes. |
| `DeleteEntry` | Delete a specific history entry by opaque ID. |
| `GetStatus` | Return current operational status (Ready/Degraded/ArmedHost/ArmedGuest). |
| `SubscribeEvents` | Named stream of clipboard lifecycle events (no payload bytes). |

---

## MIME and FD safety model

### MIME allowlist

Only the following MIME types are accepted into clipboard history. All other
MIME types produce audit event `reason: mime-rejected`.

```text
text/plain;charset=utf-8
text/plain
text/html
image/png
```

The allowlist is a closed enum defined in the clipboard-wayland descriptor's
signed policy projection. It cannot be extended at runtime.

### Secret-hint MIME detection

The following MIME types trigger automatic suppression (entry not added to
history; audit reason: `secret-hint-mime`):

```text
x-kde-passwordManagerHint
application/x-password
x-secret-content
```

Detection is on the source MIME type list before any FD transfer. No clipboard
payload inspection is performed.

### FD safety validation

Every SCM_RIGHTS-received FD is validated before use:

1. `fstat(2)` - confirm `st_nlink == 1`, `st_size ≤ maxItemBytes`, expected
   `st_mode` (regular file or pipe as declared by attachment class).
2. `fstatfs(2)` - confirm filesystem type is not a network filesystem.
3. `MSG_CMSG_CLOEXEC` - set at `recvmsg` call; validated before use.
4. `MSG_CTRUNC` - if set, the ancillary message was truncated; fail closed,
   do not proceed with partial FD set.
5. Size guard - read at most `maxItemBytes` bytes before treating as oversized
   and closing the FD without adding an entry.

FDs are never duplicated after validation, never sent downstream, and are
closed after exactly one read/write operation.

### Backpressure and cancellation

Named streams use credit-based flow control from ComponentSession. clipd-host
issues one credit per `maxConcurrentFds` slots. When all FD slots are in use,
clipd-host withholds credits on the `d2b.clipboard.bridge.v3` bridge stream;
wayland-proxy cannot enqueue further offers until credits are restored. This
prevents unbounded FD accumulation.

Every pending FD transfer has a `fdWriteTimeoutSeconds` deadline enforced via
ComponentSession cancellation token. Expiry closes the FD and emits audit
event `reason: fd-write-timeout`.

---

## Clipboard policy model

### Direction and allowlist

| Direction | Controlled by | Default |
| --- | --- | --- |
| Host compositor → clipboard history | `policy.allowHostCapture` | `true` |
| Guest VM → clipboard history | `policy.allowGuestCapture` | `true` |
| Clipboard history → Guest VM paste | `policy.requirePickerForPaste` | `true` |
| Clipboard history → Host compositor | automatic on selection announcement | always |
| Cross-Zone clipboard | `policy.crossZone.enable` | `false` (deny) |

`requirePickerForPaste` is enforced by the clipboard-controller via the picker
session dispatch protocol. If `false`, paste is authorized directly by
clipd-host without picker confirmation (for automated/headless environments).

### Loop suppression

`policy.suppressEcho` (default `true`) activates per-entry source tracking.
When clipd-host publishes a selection to the host compositor and that same
selection arrives back as a new host selection event (same content digest and
source attribution within a 2-second window), it is silently dropped without
creating a new history entry. Audit event `reason: suppressed-echo`.

### Same-Guest route

When the source and destination of a paste are the same Guest, the same-MIME
path is preserved: rich MIME types (text/html, image/png) are preserved
without downgrade. Cross-Guest paste always uses the MIME allowlist
intersection.

### Rate limiting

clipd-host enforces a per-Guest materialization rate limit of
`caps.maxGuestRatePerMin` requests per minute using a per-Guest sliding
window. Requests that exceed the rate are rejected with audit reason
`rate-limit-exceeded` and d2b-bus error `resource-exhausted`. This prevents
a compromised Guest from exhausting FD slots.

---

## ProviderStateSet

A `ProviderStateSet` is the set of all Volume resources in the Zone whose
`metadata.ownerRef` resolves to `Provider/clipboard-wayland`. It is a
query-time logical grouping, not a ResourceType or stored artifact.

```text
ProviderStateSet(zone, clipboard-wayland) =
  { v : Volume | v.metadata.zone == zone
              && v.metadata.ownerRef == "Provider/clipboard-wayland" }
```

Under D087, `Provider/clipboard-wayland` declares **no Provider state Volume**.
Its ProviderStateSet is therefore empty:

```text
ProviderStateSet(zone, clipboard-wayland) = {}
```

The two formerly-declared component state entries fail the storage-need test:
their operational state is bounded, non-secret, and derivable from
`Provider/clipboard-wayland.status`, the core Operation ledger, component
readiness, and external compositor/guest observation after restart. Provider
configuration is delivered exclusively via the sealed LaunchTicket config FD at
Process start - not through any Volume.

The clipboard-wayland controller (`Process/clipboard-controller`) does not
create, own, watch, update, delete, or mount Provider state Volumes, and does
not add `Volume` to any exported or reconciled ResourceType list. No dedicated
state-layout `User/<name>` principals, identity markers, migration workers,
or reset/destroy hooks are declared for component state.

Clipboard history remains bounded in-memory state in `clipd-host`'s process
heap and is never written to a Volume or status. Clipboard bytes, entry data,
terminal or notification bytes, secrets, paths, socket paths, FDs, PIDs, unit
names, and authority-conferring handles are excluded from every status layer,
audit, metrics, and Operations. Status is revisioned, optimistic status-writer
controlled, RBAC-readable, redacted, written only on material change, and
re-verified against external reality after restart; oversize status is rejected
with `status-oversize`.

There is no bootstrap state-Volume mechanism; the previous bootstrap exception
(D086, superseded by D087) does not apply. This dossier declares no runtime
socket/config/tmpfs Volume either: Wayland access is carried by display-wayland
and ProviderSupervisor as pre-opened FDs, not filesystem mounts.
---

## RBAC

The clipboard-controller creates and manages these RBAC resources as part of
its reconcile loop. Core does not pre-create them. The controller manages only
service-RBAC (ComponentSession roles and bindings); it does not create or manage
any Volume RBAC, does not hold a Volume reconciler role, and does not interact
with Volume resources in any way.

### Roles

| Role name | Verbs | Resource targets |
| --- | --- | --- |
| `clipboard-admin` | `connect`, `invoke`, `stream` | `d2b.clipboard.v3` (ArmBridge, DisarmBridge, DeleteEntry) |
| `clipboard-viewer` | `connect`, `invoke`, `stream` | `d2b.clipboard.v3` (ListHistory, GetStatus, SubscribeEvents) |
| `clipboard-bridge-peer` | `connect`, `stream` | `d2b.clipboard.bridge.v3` |
| `clipboard-picker-worker` | `connect`, `stream` | `d2b.clipboard.picker-coord.v3` (picker result stream only) |

### RoleBindings

| Binding name | Subject | Role |
| --- | --- | --- |
| `display-wayland-bridge` | `Provider/display-wayland` | `clipboard-bridge-peer` |
| `host-admin-clipboard` | `User/alice` | `clipboard-admin` |
| `picker-session-worker` | `Process/picker-*` (by template label) | `clipboard-picker-worker` |

All RoleBindings are Zone-local and scoped to the clipboard-wayland Provider
instance. The `Process/picker-*` binding uses a label selector matching the
`clipboard-picker-worker` processClass from the `picker-session` template.

---

## Lifecycle phases and conditions

Provider lifecycle is derived by core from child Process statuses. The
clipboard-wayland controller does not write `Provider/clipboard-wayland.status`.

### Provider phases (core-derived)

| Phase | Condition |
| --- | --- |
| `Pending` | Either child Process is not yet Ready |
| `Ready` | Both `clipboard-controller` and `clipd-host` Processes are Ready; `display-wayland` dependency is Ready or Absent |
| `Degraded` | One Process is Degraded or display-wayland is down in non-host-only mode |
| `Failed` | Any required Process has failed beyond maxRestarts |
| `Disabled` | `desiredLifecycle: stopped` set on both Processes |

### Controller reconcile states

```
Initial → AwaitingDependency → Ready → Degraded (transient) → Ready
                                   ↘ Failed (terminal)
```

- `AwaitingDependency`: display-wayland not yet Ready (if non-null); waiting
  for `Endpoint/clipboard-bridge` to report Ready.
- `Ready`: enrolled KK sessions established; `Endpoint/clipboard-bridge`,
  `Endpoint/clipboard-management`, and `Endpoint/clipboard-picker-coord` are
  accepting authorized connections.
- `Degraded`: display-wayland transiently unavailable; clipd-host is restarting;
  new paste requests held in bounded queue (`maxRestarts` not yet exceeded).
- `Failed`: Process exhausted maxRestarts; EphemeralProcess cleanup handler
  detects unrecoverable picker failures.

D091 currency and upgrade: the clipboard-wayland controller implements
`assess_update`, `plan_upgrade`, and `execute_upgrade` for its qualified
ResourceTypes and semantic clipboard sessions. A `ProviderGenerationChanged`,
`ArtifactChanged`, `DependencyChanged`, or `SpecChanged` reason populates
universal `status.update` with
`UpdateAvailable` or `UpgradeRequired`; disruptive changes MUST return
`UpgradeRequired` rather than being applied in place, while non-disruptive
changes reconcile normally. These currency fields are universal/ResourceType
base fields, never `status.provider`. Upgrades recycle only the clipboard realization
(owned `Process` resources, endpoints, and sessions) with `disruption` set to
`Reload`, `Restart`, or `Recycle`; durable config is preserved, dependent
sessions and attachments are drained and restarted by the dependency-aware
planner, and owned ephemeral session state remains process memory. No clipboard
content bytes, terminal bytes, notification content bytes, session bytes,
secrets, paths, or handles may appear in `status.update`.

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

### clipd-host startup sequence

1. Open inherited-socketpair ComponentSession to `clipboard-controller`
   (`d2b.clipboard.picker-coord.v3`); receive and validate sealed config.
2. Open enrolled KK ComponentSession to display-wayland
   (`d2b.display.host-clipboard.v3`); subscribe `HostSelectionChangedEvent`
   and `HostFocusEvent` streams.
3. Resolve the backing locator for `Endpoint/clipboard-bridge` through the
   LaunchTicket; signal readiness via provider-defined mechanism to
   system-systemd.
4. Resolve the backing locator for `Endpoint/clipboard-management`.
5. Enter main event loop: bridge requests, host selection events, picker coord
   callbacks, management API calls.

---

## Guest lifecycle handling

Core delivers the following authenticated messages to `Process/clipboard-controller`
via ComponentSession when Guest lifecycle events occur:

| Core message | Controller action |
| --- | --- |
| `GuestStarted(guest_name, zone_id)` | Arm clipboard tracking for that guest; notify clipd-host via `PurgeZoneClipboard` (idempotent) to initialize per-guest state. |
| `GuestStopped(guest_name, zone_id)` | Cancel all in-flight picker sessions for that guest; call `PurgeZoneClipboard` on clipd-host; release associated `EntryToken`s. |
| `GuestLocked(guest_name, zone_id)` | Call `SuspendZoneClipboard` on clipd-host; paste authorization denied while suspended. |
| `GuestUnlocked(guest_name, zone_id)` | Resume paste authorization for the guest. |
| `GuestDestroyed(guest_name, zone_id)` | `PurgeZoneClipboard` + revoke all associated history entries; release any pending picker sessions. |
| `GuestSuspended(guest_name, zone_id)` | Same as `GuestLocked`. |
| `GuestResumed(guest_name, zone_id)` | Same as `GuestUnlocked`. |

The controller does not subscribe to `Guest/*` resource watch events. It
receives only the authenticated lifecycle messages forwarded by core.

---

## Picker session full lifecycle

```
1. Guest initiates paste (wayland-proxy detects DataOffer)
2. wayland-proxy calls NotifyGuestSelection on d2b.clipboard.bridge.v3
   → clipd-host receives EntryToken
3. policy.requirePickerForPaste == true:
   clipd-host calls RequestPickerSession on d2b.clipboard.picker-coord.v3
   → controller creates EphemeralProcess/picker-<id>
     spec.startDeadline = "10s", runtimeDeadline = "120s"
4. picker binary starts; receives metadata stream from controller:
   { operation_id, source_zone, dest_guest, mime_list_hint }
   (no FDs, no payload, no compositor credentials)
5. picker renders GTK4 UI via ProviderSupervisor pre-opened WAYLAND_SOCKET FD
   (presentation-only portal; no clipboard-manager globals)
6. user selects item or cancels (or runtimeDeadline expires → TimedOut)
7. picker sends Select(item_digest)|Cancel back to controller stream
   picker exits 0 (Select/Cancel) or non-zero (error)
8. controller calls NotifyPickerResult(session_id, outcome) on clipd-host
9. on Selected:
   clipd-host calls AuthorizeGuestPaste(entry_token, mime_type, fd)
   on d2b.clipboard.bridge.v3
   → wayland-proxy writes clipboard data to Guest selection
10. on Cancelled/TimedOut:
   clipd-host calls CancelEntry(entry_token) on d2b.clipboard.bridge.v3
   → wayland-proxy releases selection offer
11. EphemeralProcess/picker-<id> enters Succeeded|Failed phase;
   cleanupEligibleAt set per successfulTtl/failedTtl
```

---

## Error taxonomy

| Error code | Description | Retryable |
| --- | --- | --- |
| `mime-rejected` | MIME type not in MIME allowlist | No |
| `secret-hint-mime` | Clipboard content matches secret-hint MIME pattern | No |
| `item-too-large` | Clipboard item exceeds `maxItemBytes` | No |
| `total-quota-exceeded` | History total exceeds `maxTotalBytes`; LRU eviction ran but insufficient | No |
| `fd-count-exceeded` | `maxConcurrentFds` in-flight FDs reached | Yes (backoff) |
| `fd-write-timeout` | FD write not completed within `fdWriteTimeoutSeconds` | Yes (limited) |
| `msg-ctrunc` | `MSG_CTRUNC` detected on recvmsg; partial ancillary data | No |
| `fd-safety-violation` | fstat/fstatfs validation failed | No |
| `rate-limit-exceeded` | Per-Guest materialization rate exceeded | Yes (backoff) |
| `picker-timed-out` | Picker runtimeDeadline expired without user action | No |
| `picker-cancelled` | User cancelled picker | No |
| `picker-start-failed` | EphemeralProcess/picker failed startDeadline | Yes (limited) |
| `echo-suppressed` | Selection suppressed as host echo | No |
| `dependency-absent` | display-wayland dependency is Absent; bridge operations rejected | No |
| `dependency-degraded` | display-wayland transiently unavailable | Yes |
| `zone-suspended` | Paste rejected because zone is in Suspended state | No |
| `unauthorized` | RBAC check denied the operation | No |
| `cross-zone-denied` | `crossZone.enable == false` | No |

---

## Audit format

Audit events are emitted by `clipd-host` to the Zone audit sink via the
`d2b.audit.v3` service using the `fail-closed` per-Zone queue model from
ADR 0042. If the queue is full, new clipboard operations are rejected rather
than proceeding without an audit record.

### AuditEvent schema

```rust
pub struct ClipboardAuditEvent {
    pub operation_id: Uuid,             // unique per operation
    pub event_type: ClipboardEventType,
    pub source_zone_id: Option<BoundedId>,  // max 63 chars; no path/payload
    pub dest_zone_id: Option<BoundedId>,
    pub mime_type: Option<AllowedMime>,     // from MIME allowlist only; null if rejected before check
    pub byte_hint: Option<SizeBucket>,      // discretized: <1K, 1-64K, 64K-1M, >1M; never exact size
    pub reason: ReasonCode,
    pub attribution_quality: AttributionQuality,
    pub occurred_at: OffsetDateTime,
    pub operation_duration_ms: u32,
}

pub enum ClipboardEventType {
    HostCapture,
    GuestCapture,
    PasteAuthorized,
    PasteRejected,
    EchoSuppressed,
    EntryExpired,
    EntryPurged,
    BridgeArmed,
    BridgeDisarmed,
    PickerSessionStarted,
    PickerSessionCompleted,
    PickerSessionFailed,
}

// closed enum - no fallback/unknown variant that accepts arbitrary strings
pub enum ReasonCode {
    Ok,
    MimeRejected,
    SecretHintMime,
    ItemTooLarge,
    TotalQuotaExceeded,
    FdCountExceeded,
    FdWriteTimeout,
    MsgCtrunc,
    FdSafetyViolation,
    RateLimitExceeded,
    PickerTimedOut,
    PickerCancelled,
    PickerStartFailed,
    EchoSuppressed,
    DependencyAbsent,
    DependencyDegraded,
    ZoneSuspended,
    Unauthorized,
    CrossZoneDenied,
}

pub enum AttributionQuality {
    Verified,     // focus window attribution confirmed via display-wayland FocusEvent
    Approximate,  // focus changed between selection and capture
    Unknown,      // no attribution data available
}

pub enum SizeBucket {
    Lt1K,
    K1To64K,
    K64ToM1,
    GtM1,
}
```

**Redaction rules:**

- Clipboard bytes, MIME content, raw FD data: never in any field.
- Zone IDs are bounded opaque IDs; no host-visible user data.
- `byte_hint` is a discrete bucket, never an exact byte count.
- Guest names, window titles, app_id, and user-visible text: excluded.
- PID, cgroup path, socket path, pidfd FD numbers: never in audit.
- Attribution quality is a coarse enum; no raw window manager metadata.

---

## Telemetry and OTEL

All metrics use stable closed label sets. No clipboard bytes, content hashes,
window titles, user identifiers, or zone credentials appear in spans or metrics.

### Metrics

| Metric name | Type | Labels | Description |
| --- | --- | --- | --- |
| `clipboard_operations_total` | Counter | `operation`, `reason`, `direction` | Clipboard operations by type and outcome. |
| `clipboard_active_fds` | Gauge | `direction` | Currently in-flight SCM_RIGHTS FDs. |
| `clipboard_history_entries` | Gauge | `source` (`host`/`guest`) | Current history entry count. |
| `clipboard_history_bytes_total` | Gauge | - | Total in-memory history bytes. |
| `clipboard_picker_sessions_total` | Counter | `outcome` (`selected`/`cancelled`/`timed_out`/`failed`) | Picker session outcomes. |
| `clipboard_mime_rejections_total` | Counter | `reason` | MIME rejections by reason code. |

`operation` labels: `host-capture`, `guest-capture`, `paste-authorized`,
`paste-rejected`, `purge`, `suspend`, `arm`, `disarm`.

`direction` labels: `host-to-guest`, `guest-to-host`.

No per-Zone, per-Guest, or per-user dimension labels on metrics (cardinality
control).

### Spans

- One span per clipboard operation (correlation with `operation_id`).
- Span attributes: `operation_type`, `reason_code`, `mime_type` (from
  allowlist or `rejected`), `attribution_quality`, `direction`.
- Excluded span attributes: byte sizes, content hashes, zone IDs, guest names,
  user names, process IDs, socket paths, FD numbers.

---

## Nix catalog registration

```nix
# In the host NixOS configuration (or d2b Zone config):
d2b.artifacts.clipboard-wayland = {
  package  = inputs.d2b.packages.${system}.clipboard-wayland;
  type     = "provider";
  # artifactId is derived from package metadata; no manual ID needed.
};
```

The `artifactId = "clipboard-wayland"` in Provider spec resolves through the
offline catalog compiled from this artifact declaration. The Nix store path is
never exposed in resource spec, status, or audit.

### Migration from nixos-modules/clipboard.nix

| Old option | New mechanism |
| --- | --- |
| `d2b.clipboard.enable` | Declare `d2b.zones.<zone>.resources.clipboard-wayland` with `type = "Provider"` |
| `d2b.clipboard.bridgeSocketDir` | Removed: no bridge socket directory; FDs flow via ComponentSession |
| `d2b.clipboard.allowedMimeTypes` | Closed MIME allowlist in Provider descriptor; not operator-configurable |
| `d2b.clipboard.pickerCommand` | Replaced by `spec.config.pickerArtifactId` (optional; null = bundled picker) |
| `d2b.clipboard.niriSocket` | Removed: compositor access via `d2b.display.host-clipboard.v3` only |
| `d2b.clipboard.historySize` | `spec.config.caps.maxHistoryEntries` |
| `d2b.clipboard.maxItemSize` | `spec.config.caps.maxItemBytes` |
| `d2b.clipboard.requirePicker` | `spec.config.policy.requirePickerForPaste` |
| `d2b.clipboard.guestGroups` | Removed: no per-Guest Unix groups; access via enrolled ComponentSession only |

---

## Invariants (normative)

The following invariants are checked by contract tests and must not be
violated by any implementation:

1. **No bytes in resources.** No clipboard payload byte sequence may appear
   in any resource spec, status, condition message, or audit payload field.

2. **Closed MIME allowlist.** Only `text/plain;charset=utf-8`, `text/plain`,
   `text/html`, `image/png` are accepted. No runtime extension.

3. **FD safety.** Every SCM_RIGHTS-received FD is validated with fstat,
   fstatfs, MSG_CMSG_CLOEXEC, and MSG_CTRUNC detection. MSG_CTRUNC = fail closed.

4. **Picker authority.** Picker EphemeralProcess receives no clipboard FDs,
   clipboard bytes, compositor credentials, or NIRI_SOCKET path.

5. **No DND.** wl_data_device_manager drag-and-drop is never implemented.

6. **No primary selection.** zwp_primary_selection_device_manager_v1 is
   never implemented.

7. **Fail-closed audit.** If the Zone audit queue is full, new clipboard
   operations are rejected; they do not proceed unaudited.

8. **ReasonCode closed enum.** No arbitrary string reason code is accepted.
   Unknown proto fields fail closed.

9. **No filesystem IPC.** No bridge directory, per-Guest Unix socket, or
   shared path is used. All IPC is ComponentSession over private transport.

10. **Loop suppression default-on.** `policy.suppressEcho` defaults to `true`.
    Implementations may not change this default.

11. **No cross-Zone default.** `policy.crossZone.enable` defaults to `false`.
    Cross-Zone is denied unless explicitly enabled.

12. **Controller does not create itself.** Core ProviderDeployment creates both
    `Process/clipboard-controller` and `Process/clipd-host`. The controller
    does not create or adopt itself.

13. **Core derives Provider status.** `Provider/clipboard-wayland.status` is
    written only by core. The clipboard-wayland controller does not write it.
    D088 layers that status as universal `status.*` plus core-owned
    `status.resource`; any optional `status.provider` extension is strict,
    unknown-field-denied, manifest-registered/signed, bounded, redacted, and
    shared-field-free.

---

## Work items

### ADR046-clipboard-001 - Crate skeleton
| Field | Value |
| --- | --- |
| Dependency/owner | ADR-046-provider-model-and-packaging; Provider/clipboard-wayland crate owner |
| Current source | None - net-new v3 work; no pre-ADR45 baseline equivalent for the provider crate skeleton |
| Reuse action | create |
| Destination | packages/d2b-provider-clipboard-wayland/ with src, tests, integration, README.md, and binaries clipboard-controller, clipd-host, picker-session |
| Detailed design | Create the provider crate skeleton, required source layout, three binaries, and README covering purpose, component map, local build instructions, test commands, and display-wayland fake dependency for integration tests. |
| Integration | Workspace package manifest and Provider artifact catalog consume the crate; provider packaging registers component templates for core ProviderDeployment. |
| Data migration | None - docs/tooling only; no runtime state |
| Validation | Workspace provider layout policy plus README content review and follow-on make test-rust -p d2b-provider-clipboard-wayland once implementation exists. |
| Removal proof | None - net-new; no prior owner to remove |

**Type:** implementation  
**Inputs:** This dossier, `ADR-046-provider-model-and-packaging`

Create `packages/d2b-provider-clipboard-wayland/` with root directories
`src/`, `tests/`, `integration/`, and `README.md` as required by the
source layout section of this dossier. Binaries: `clipboard-controller`,
`clipd-host`, `picker-session`.

`README.md` must include: purpose, component map, local build instructions,
test commands, dependency on display-wayland fake in integration tests.

### ADR046-clipboard-002 - Service process (clipd-host)
| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-clipboard-001; clipd-host service owner |
| Current source | packages/d2b-clipd/ algorithms and types listed in the Reuse from baseline section |
| Reuse action | adapt |
| Destination | packages/d2b-provider-clipboard-wayland/src/clipd_host/ service binary modules such as service, display_client, bridge, picker_coord, policy, fd, audit, history |
| Detailed design | Adapt d2b-clipd into clipd-host: call RequestPickerSession over d2b.clipboard.picker-coord.v3, consume d2b.display.host-clipboard.v3 instead of WAYLAND_DISPLAY or NIRI_SOCKET, serve d2b.clipboard.bridge.v3 on Endpoint/clipboard-bridge, remove filesystem bridge and peer group ACL logic, and preserve MIME allowlist, FD safety, fail-closed audit, loop suppression, and LRU history algorithms. Primary reuse disposition: `adapt`. Preserved source-plan detail: port and adapt algorithms; replace direct compositor, picker subprocess, Unix bridge, SO_PEERCRED, bridge directory, and group ACL paths. |
| Integration | Core creates Process/clipd-host; display-wayland wayland-proxy consumes Endpoint/clipboard-bridge; clipd-host consumes display-wayland host-clipboard service and controller picker coordination service. |
| Data migration | Full d2b 3.0 reset; clipboard history is bounded process memory and no v2 clipboard runtime state is imported |
| Validation | make test-rust -p d2b-provider-clipboard-wayland plus unit coverage for MIME policy, FD safety, audit fail-closed queue, history bounds, lifecycle purge and suspension, no filesystem bridge, and no bytes in status or audit. |
| Removal proof | Baseline picker subprocess, direct compositor/Niri clients, Unix bridge socket server, bridge directories, SO_PEERCRED peer config, and per-Guest groups are absent from the provider crate and covered by invariant tests. |

**Type:** implementation  
**Inputs:** ADR046-clipboard-001; ported from `packages/d2b-clipd/`

Adapt clipboard algorithms from `d2b-clipd` into the `clipd-host` service
binary. Key changes from baseline:

- Replace `picker.rs` subprocess fork/exec with `RequestPickerSession` call
  to controller via `d2b.clipboard.picker-coord.v3`.
- Replace direct WAYLAND_DISPLAY connection with `d2b.display.host-clipboard.v3`
  client session (display_client.rs).
- Replace NIRI_SOCKET NiriJsonClient with focus events from the display client.
- Replace Unix bridge socket server with `d2b.clipboard.bridge.v3` ComponentSession
  service on `Endpoint/clipboard-bridge`.
- Remove all `SO_PEERCRED` peer config, bridge directories, and group ACL logic.
- Port MIME allowlist, FD safety, audit, loop suppression, LRU history from
  d2b-clipd verbatim (algorithm preservation invariant).
- `environmentClass: provider-defined` (system-systemd user scope provides
  XDG_RUNTIME_DIR; clipd-host does not use WAYLAND_DISPLAY itself).

Conformance gates: `make test-rust -p d2b-provider-clipboard-wayland`.

### ADR046-clipboard-003 - Controller process (clipboard-controller)
| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-clipboard-001; clipboard-controller owner |
| Current source | None - net-new v3 controller; no pre-ADR45 baseline equivalent |
| Reuse action | create |
| Destination | packages/d2b-provider-clipboard-wayland/src/controller/ and clipboard-controller binary |
| Detailed design | Implement Process/clipboard-controller as a system-domain system-minijail Process serving d2b.clipboard.picker-coord.v3, creating picker EphemeralProcesses from the signed picker-session template, observing picker status, relaying Guest lifecycle messages to clipd-host, creating clipboard RBAC Roles and RoleBindings, writing only bounded redacted operational observations through the optimistic status writer, and never owning or mounting Provider state Volumes. Primary reuse disposition: `create`. Preserved source-plan detail: net-new controller using existing resource API and ComponentSession contracts. |
| Integration | Core ProviderDeployment creates the controller Process; controller uses Zone resource API for EphemeralProcess and RBAC resources and ComponentSession to clipd-host for picker and purge or suspend coordination. |
| Data migration | Full d2b 3.0 reset; no controller durable state import because ProviderStateSet is empty |
| Validation | Controller unit tests for picker request validation, EphemeralProcess spec shape, terminal status callback, GuestStopped and GuestLocked handling, RBAC resources, status bounds, and empty ProviderStateSet. |
| Removal proof | None - net-new; no prior controller owner to remove |

**Type:** implementation  
**Inputs:** ADR046-clipboard-001

Implement `Process/clipboard-controller` as a system-domain system-minijail
Process. Responsibilities:

- Serve `d2b.clipboard.picker-coord.v3` on
  `Endpoint/clipboard-picker-coord`.
- On `RequestPickerSession`: validate, create `EphemeralProcess/picker-<uuid>`
  via resource API with signed template `picker-session`, `processClass: worker`,
  `successfulTtl: "1h"`, `failedTtl: "24h"`, `startDeadline: "10s"`,
  `runtimeDeadline: "120s"`. Picker config is null; metadata arrives via
  inherited-socketpair ComponentSession stream at spawn time.
- Watch `EphemeralProcess/picker-*` status for terminal transitions; call
  `NotifyPickerResult` back to clipd-host.
- Receive `GuestStopped`/`GuestLocked`/etc. from core orchestrator;
  call `PurgeZoneClipboard`/`SuspendZoneClipboard` on clipd-host.
- Create and manage RBAC Role/RoleBinding resources listed in this dossier.
- Writes only bounded, redacted operational observations to
  `Provider/clipboard-wayland.status` through the optimistic status writer.
- Does not own, export, reconcile, or mount any Provider state Volume; the
  ProviderStateSet is empty under D087.

### ADR046-clipboard-004 - EphemeralProcess picker binary
| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-clipboard-001 and ADR046-clipboard-003; picker worker owner |
| Current source | packages/d2b-clipd/src/picker.rs subprocess flow is replacement context only; picker-session worker is net-new |
| Reuse action | adapt |
| Destination | packages/d2b-provider-clipboard-wayland/src/picker_session/ and picker-session binary |
| Detailed design | Implement picker-session as a user-domain worker EphemeralProcess with minimal environment, metadata over inherited ComponentSession named stream, restricted WAYLAND_SOCKET FD from display-wayland presentation portal, GTK4 closure-contained runtime, one Select or Cancel response, no clipboard FDs, no clipboard bytes, no compositor credentials, no socket paths, and typed PickerStartFailed on install or start failure instead of bypass. Primary reuse disposition: `adapt`. Preserved source-plan detail: rewrite as EphemeralProcess worker. |
| Integration | clipboard-controller creates picker EphemeralProcess per paste request; ProviderSupervisor pre-opens restricted Wayland FD; picker returns result to controller which notifies clipd-host. |
| Data migration | Full d2b 3.0 reset; picker state is per-operation EphemeralProcess status only |
| Validation | Contract test that picker cannot bind zwlr_data_control_manager_v1 plus unit tests for processClass worker, no FDs or payload in picker config, TTL defaults, response framing, and requirePickerForPaste false bypass semantics in clipd-host. |
| Removal proof | Old d2b-clipd subprocess picker path is absent once RequestPickerSession and picker EphemeralProcess tests pass. |

**Type:** implementation  
**Inputs:** ADR046-clipboard-001, ADR046-clipboard-003

Implement `picker-session` worker binary. Invariants:

- `processClass: worker`; `domain: user`; system-systemd; `environmentClass: minimal`.
- Receives metadata from clipboard-controller via inherited-socketpair
  ComponentSession named stream (operation_id, source_zone, dest_guest,
  mime_list_hint).
- Wayland access: ProviderSupervisor pre-opens a restricted compositor
  connection FD backed by display-wayland's presentation-only portal and
  passes it with `WAYLAND_SOCKET=<fd_number>` (FD number only, no path).
  GTK4 connects via this FD. `zwlr_data_control_manager_v1` and all
  clipboard-manager globals are absent from the portal; seccomp policy
  prevents any attempt to open a compositor socket path.
- GTK4 and all runtime dependencies are in the picker artifact's Nix closure.
  No ambient host GTK4 dependency.
- Sends exactly one `Select(item_digest)` or `Cancel` frame back to the
  controller via the named stream. No clipboard content transits this stream.
- Receives no clipboard FDs, no compositor credentials, no socket path.
- Exits 0 on Select/Cancel; non-zero on startup failure.
- If `spec.config.policy.requirePickerForPaste` is `false`, this binary is
  not invoked; clipd-host authorizes pastes directly without a picker session.
  Install or start failure of the picker EphemeralProcess is a typed error
  (`PickerStartFailed`), not a silent bypass.

Contract test: picker binary must fail to bind `zwlr_data_control_manager_v1`
(absent from seccomp allowlist and restricted compositor portal).

### ADR046-clipboard-005 - ComponentSession service definitions
| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-clipboard-001; ADR-046-componentsession-and-bus; clipboard service contract owner |
| Current source | None - net-new clipboard-wayland ComponentSession service definitions; display-wayland client stubs are consumed from the display-wayland contract |
| Reuse action | create |
| Destination | packages/d2b-provider-clipboard-wayland service descriptors and generated Rust async ttrpc bindings, plus any shared contracts crate selected by ADR-046-componentsession-and-bus |
| Detailed design | Generate stubs for d2b.clipboard.bridge.v3, d2b.clipboard.picker-coord.v3, and d2b.clipboard.v3, consume display-wayland d2b.display.host-clipboard.v3 client stubs, reject service-name collisions, and declare attachment classes clipboard-transfer-fd, host-selection-transfer-fd, and host-selection-supply-fd in the signed descriptor for ComponentSession handshake validation. Primary reuse disposition: `create`. Preserved source-plan detail: net-new generation of service stubs and named-stream types. |
| Integration | Service registry, Zone ComponentSession enrollment, clipd-host, clipboard-controller, display-wayland wayland-proxy, and CLI/operator clients all consume the generated bindings. |
| Data migration | None - docs/tooling only; no runtime state |
| Validation | Contract tests for wire format, service-name collision rejection, attachment class matching, and descriptor handshake validation. |
| Removal proof | Shared filesystem bridge path and SO_PEERCRED contract tests are removed in ADR046-clipboard-011 after ComponentSession contracts pass. |

**Type:** implementation  
**Inputs:** ADR046-clipboard-001; `ADR-046-componentsession-and-bus`

Generate Rust async ttrpc stubs and named-stream types for:

- `d2b.clipboard.bridge.v3` (server: clipd-host; client: wayland-proxy)
- `d2b.clipboard.picker-coord.v3` (server: controller; client: clipd-host)
- `d2b.clipboard.v3` (server: clipd-host; client: CLI/operator)

Consume display-wayland's `d2b.display.host-clipboard.v3` generated client
stubs (imported from the display-wayland crate or a shared contracts crate).
Service names `d2b.display.host-clipboard.v3` and `d2b.clipboard.picker-coord.v3`
are normative; generation compilation rejects service-name collisions in the
Zone service registry.

All attachment class definitions (`clipboard-transfer-fd`,
`host-selection-transfer-fd`, `host-selection-supply-fd`) must be declared in
the signed service descriptor and validated at ComponentSession handshake.

### ADR046-clipboard-006 - Provider Nix configuration
| Field | Value |
| --- | --- |
| Dependency/owner | ADR-046-nix-configuration; Provider/clipboard-wayland Nix owner |
| Current source | nixos-modules/clipboard.nix and current d2b.clipboard.* options |
| Reuse action | replace |
| Destination | nixos-modules/providers/clipboard-wayland.nix and d2b.artifacts.clipboard-wayland catalog entry |
| Detailed design | Implement Nix module emitting d2b.zones.<zone>.resources.<name> Provider resources with spec.artifactId and spec.config, validate hostExecutionRef, hostUserRef, displayWaylandRef, and pickerArtifactId, forbid spec.componentPlacements, spec.settings, and spec.status, and remove nixos-modules/clipboard.nix in the same landing sequence as the new module. Primary reuse disposition: `replace`. Preserved source-plan detail: replace option surface with Provider resource Nix module. |
| Integration | Nix resource compiler emits Provider resource and artifact catalog data consumed by core configuration publication and ProviderDeployment. |
| Data migration | Full d2b 3.0 reset; operators translate old d2b.clipboard.* options using the dossier mapping table |
| Validation | Nix eval tests for resource shape, reference validation, null displayWaylandRef host-only mode, artifact catalog lookup, and absence of deprecated spec fields. |
| Removal proof | nixos-modules/clipboard.nix import is removed and examples/static checks no longer reference old d2b.clipboard.* options. |

**Type:** implementation  
**Inputs:** `ADR-046-nix-configuration`; this dossier Nix authoring section

Implement the d2b Nix module for `Provider/clipboard-wayland`:

- `d2b.zones.<zone>.resources.<name>` with `type = "Provider"` and
  `spec.{artifactId, config}` shape as defined in this dossier.
- Validate: `hostExecutionRef`/`hostUserRef` resolve to declared resources;
  `displayWaylandRef` resolves to a `Provider/display-wayland` if non-null;
  `pickerArtifactId` null or registered in artifact catalog.
- No `spec.componentPlacements`, `spec.settings`, or `spec.status` in Nix.
- Attribute path: `d2b.artifacts.clipboard-wayland` for the package.
- Remove `nixos-modules/clipboard.nix` in the same commit as the new module
  lands (see ADR046-clipboard-012).

### ADR046-clipboard-007 - RBAC resources
| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-clipboard-003; clipboard RBAC owner |
| Current source | None - net-new Zone RBAC resources for clipboard-wayland; no pre-ADR45 baseline equivalent |
| Reuse action | create |
| Destination | packages/d2b-provider-clipboard-wayland/src/controller/rbac.rs or equivalent controller reconcile module |
| Detailed design | Controller creates Role/clipboard-admin, Role/clipboard-viewer, Role/clipboard-bridge-peer, Role/clipboard-picker-worker and RoleBindings display-wayland-bridge, host-admin-clipboard, picker-session-worker, all Zone-scoped, owned by Process/clipboard-controller, selector-bound for Process/picker-*, and cleaned up when Provider is deleted. |
| Integration | Resource API stores RBAC resources; ComponentSession authorization checks consume Roles and RoleBindings for management, bridge, and picker worker services. |
| Data migration | Full d2b 3.0 reset; no v2 RBAC state import |
| Validation | Controller RBAC reconcile tests for create, idempotent update, provider deletion cleanup, selector scoping, bridge peer authorization, and denied unauthorized management calls. |
| Removal proof | None - net-new; no prior owner to remove |

**Type:** implementation  
**Inputs:** ADR046-clipboard-003; RBAC section of this dossier

The clipboard-controller reconcile loop creates:

- `Role/clipboard-admin`, `Role/clipboard-viewer`,
  `Role/clipboard-bridge-peer`, `Role/clipboard-picker-worker`
- `RoleBinding/display-wayland-bridge`, `RoleBinding/host-admin-clipboard`,
  `RoleBinding/picker-session-worker`

All Roles and RoleBindings are Zone-scoped, owned by
`Process/clipboard-controller`, and are cleaned up when the Provider is
deleted.

### ADR046-clipboard-008 - Audit and telemetry
| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-clipboard-002; ADR-046-telemetry-audit-and-support; clipboard observability owner |
| Current source | packages/d2b-clipd/src/audit.rs and policy types from packages/d2b-clipd/src/policy.rs |
| Reuse action | adapt |
| Destination | packages/d2b-provider-clipboard-wayland/src/service/audit.rs and packages/d2b-provider-clipboard-wayland/src/service/metrics.rs |
| Detailed design | Implement ClipboardAuditEvent and fail-closed Zone audit queue by porting baseline audit code, renaming realm fields to source_zone_id and dest_zone_id, making ReasonCode a closed enum with unknown protobuf fields rejected, replacing exact byte counts with SizeBucket, emitting to d2b.audit.v3, and adding closed-semantic-label OTEL metrics and spans from the dossier tables. Metric descriptors carry no Zone/resource-name-derived identity; `d2b.zone` remains a resource attribute. Primary reuse disposition: `adapt`. Preserved source-plan detail: port and adapt audit plus resource-name-free metrics and redaction changes. |
| Integration | clipd-host emits audit events to the Zone audit sink and OTEL metrics/spans to the observability Provider pipeline during clipboard operations. |
| Data migration | Full d2b 3.0 reset; audit stream is v3 Zone-local and no v2 audit records are imported |
| Validation | Audit tests for no bytes in events, closed ReasonCode deserialization, fail-closed queue rejection, SizeBucket discretization, excluded span attributes, and structural metric descriptor assertions for exact absence of `vm`, `zone`, `zone_id`, `zone_uid`, and resource-name-derived keys plus clipboard/Zone-name canary absence while preserving `d2b.zone` resource attributes. |
| Removal proof | Old audit shape with realm field names and exact byte counts is absent after ported tests assert the v3 ClipboardAuditEvent schema. |

**Type:** implementation  
**Inputs:** ADR046-clipboard-002; `ADR-046-telemetry-audit-and-support`

Implement `ClipboardAuditEvent` and fail-closed queue in `service/audit.rs`:

- Port `audit.rs` from `packages/d2b-clipd/src/audit.rs`.
- Rename `source_realm`/`destination_realm` → `source_zone_id`/`dest_zone_id`.
- `ReasonCode` must be a closed Rust enum (`#[non_exhaustive]` is forbidden).
  Unknown protobuf fields fail the deserialization.
- `SizeBucket` discretization replaces exact byte counts.
- Emit `ClipboardAuditEvent` to Zone audit sink via `d2b.audit.v3`.
- Implement OTEL metrics with label sets from the Metrics table above.
- Implement OTEL spans with allowed/excluded attribute lists from the Spans
  section above.

### ADR046-clipboard-009 - Hermetic unit tests
| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-clipboard-001 through ADR046-clipboard-008; clipboard test owner |
| Current source | packages/d2b-clipd/ behavior and this dossier invariants; no single baseline test path is declared for every assertion |
| Reuse action | extract |
| Destination | packages/d2b-provider-clipboard-wayland/tests/ |
| Detailed design | Create hermetic unit and Cargo integration tests covering closed MIME policy, secret-hint suppression, FD validation and bounds, LRU and TTL history, fail-closed audit, lifecycle purge and suspend, picker EphemeralProcess invariants, no filesystem bridge, core-created Processes, empty ProviderStateSet, no state mounts or state-layout principals, status-first observation, and no clipboard bytes in status, audit, metrics, Operations, or Volumes. Primary reuse disposition: `extract`. Preserved source-plan detail: extract semantic assertions into hermetic provider tests. |
| Integration | cargo test -p d2b-provider-clipboard-wayland --lib --tests consumes the provider crate and fake clocks/effect ports without live Wayland, systemd, broker, or Nix eval. |
| Data migration | None - docs/tooling only; no runtime state |
| Validation | All tests listed in the Required test coverage table must pass under make test-rust -p d2b-provider-clipboard-wayland. |
| Removal proof | Replaced current-code tests receive explicit keep/adapt/move/delete dispositions and old duplicate tests are deleted once successor coverage passes. |

**Type:** test  
**Inputs:** ADR046-clipboard-001 through ADR046-clipboard-008

Required test coverage (in `packages/d2b-provider-clipboard-wayland/tests/`):

| Test | What it verifies |
| --- | --- |
| `policy::test_mime_allowlist_closed` | All listed MIME types accepted; all others rejected |
| `policy::test_secret_hint_detection` | All secret-hint MIME types trigger suppression |
| `fd::test_msg_ctrunc_fail_closed` | MSG_CTRUNC on recvmsg → operation rejected, no partial FD |
| `fd::test_fstat_nlink_guard` | FD with st_nlink > 1 → rejected |
| `fd::test_item_too_large` | FD content exceeds maxItemBytes → closed without entry |
| `history::test_lru_eviction` | History bounded at maxHistoryEntries; LRU entry evicted |
| `history::test_total_quota` | Total bytes quota enforced across entries |
| `history::test_ttl_expiry` | Entries expire after hostEntrySeconds/guestEntrySeconds |
| `audit::test_no_bytes_in_event` | ClipboardAuditEvent contains no clipboard bytes |
| `audit::test_reason_code_closed_enum` | Unknown reason code → deserialization error |
| `audit::test_fail_closed_queue` | Full queue → operation rejected, not bypassed |
| `lifecycle::test_guest_stopped_purge` | GuestStopped → all entries for that guest purged |
| `lifecycle::test_guest_locked_suspend` | GuestLocked → paste requests rejected with zone-suspended |
| `picker::test_ephemeral_no_fds` | Picker spec contains no SCM_RIGHTS attachments |
| `picker::test_ephemeral_process_class` | processClass = worker; controller/service rejected |
| `picker::test_ephemeral_ttl_defaults` | successfulTtl=1h, failedTtl=24h |
| `invariants::test_no_filesystem_bridge` | No socket path or dir appears in Process spec config |
| `invariants::test_core_creates_processes` | Controller does not create Process/clipd-host |
| `state::test_provider_state_set_empty` | Provider declares no Provider state Volume; ProviderStateSet query returns empty for `Provider/clipboard-wayland` |
| `state::test_no_state_mounts` | Component Process specs contain no `/state` mount and no Provider state Volume reference |
| `state::test_no_state_layout_principals` | No dedicated state-layout `User/<name>` or ComponentPrincipal reference is emitted for component state |
| `state::test_status_first_operational_state` | Bounded non-secret operational observations live in revisioned status/core Operation ledger and are re-verified after restart |
| `state::test_no_clipboard_bytes_in_status` | No clipboard bytes, entry data, FD content, socket path, or authority handle appears in status, audit, metrics, Operations, or any Volume |

### ADR046-clipboard-010 - Integration tests
| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-clipboard-009; display-wayland fake and clipboard integration owner |
| Current source | None - net-new v3 provider integration scenarios; no pre-ADR45 baseline equivalent |
| Reuse action | create |
| Destination | packages/d2b-provider-clipboard-wayland/integration/ |
| Detailed design | Implement e2e paste, host capture, bridge backpressure, rate limiting, echo suppression, dependency absent host-only mode, GuestDestroyed purge, audit fail-closed, picker start timeout, and cross-zone denied scenarios using fake d2b.display.host-clipboard.v3 server and fake wayland-proxy bridge client without requiring a live compositor. Primary reuse disposition: `create`. Preserved source-plan detail: net-new integration suite with fake display-wayland and fake wayland-proxy. |
| Integration | Provider integration lane exercises clipd-host, clipboard-controller, generated ComponentSession services, fake display-wayland service, and fake bridge client end-to-end. |
| Data migration | None - docs/tooling only; no runtime state |
| Validation | All integration scenarios in the dossier table pass and assert no live Wayland compositor dependency. |
| Removal proof | None - net-new; no prior owner to remove |

**Type:** test  
**Inputs:** ADR046-clipboard-009; display-wayland fake

Required integration test scenarios (in `packages/d2b-provider-clipboard-wayland/integration/`):

| Test | Scenario |
| --- | --- |
| `e2e_paste.rs` | Full paste flow: fake wayland-proxy calls NotifyGuestSelection → RequestPickerSession → fake picker selects → AuthorizeGuestPaste → verify FD arrives |
| `e2e_host_capture.rs` | Host selection event from fake display client → entry stored → ListHistory returns metadata (no bytes) |
| `bridge_backpressure.rs` | maxConcurrentFds reached → NotifyGuestSelection returns backpressure error; after FD closed → proceeds |
| `rate_limit.rs` | maxGuestRatePerMin exceeded → rate-limit-exceeded audit event and rejection |
| `echo_suppression.rs` | Host selection echoed back from clipd publish → suppressed, not re-added |
| `dependency_absent.rs` | displayWaylandRef = null → bridge methods return dependency-absent; management API works |
| `guest_destroy_purge.rs` | GuestDestroyed → all history entries for zone purged; picker sessions cancelled |
| `audit_fail_closed.rs` | Audit queue filled → clipboard operation rejected; no unaudited operation proceeds |
| `picker_start_timeout.rs` | Picker EphemeralProcess startDeadline exceeded → PickerStartFailed audit event; operation cancelled |
| `cross_zone_denied.rs` | crossZone.enable = false → cross-Zone paste rejected |

Integration tests use a fake `d2b.display.host-clipboard.v3` server and a
fake wayland-proxy bridge client. They do not require a live Wayland compositor.

### ADR046-clipboard-011 - Contract tests
| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-clipboard-005; packages/d2b-contract-tests owner |
| Current source | packages/d2b-contract-tests/tests/policy_clipboard.rs |
| Reuse action | adapt |
| Destination | packages/d2b-contract-tests/tests/policy_clipboard.rs |
| Detailed design | Add contract tests for d2b.clipboard.bridge.v3 and d2b.clipboard.picker-coord.v3 wire formats, ReasonCode numeric stability, and attachment class descriptor names while removing tests that assume shared filesystem bridge paths or SO_PEERCRED config. Primary reuse disposition: `adapt`. Preserved source-plan detail: adapt contract tests and delete obsolete filesystem bridge assumptions. |
| Integration | Contract test suite consumes generated service descriptors and guards downstream ComponentSession consumers. |
| Data migration | None - docs/tooling only; no runtime state |
| Validation | packages/d2b-contract-tests policy_clipboard.rs passes with v3 wire format and attachment descriptor assertions. |
| Removal proof | Tests for shared filesystem bridge paths and SO_PEERCRED config are removed from policy_clipboard.rs after v3 ComponentSession contract coverage lands. |

**Type:** test  
**Inputs:** ADR046-clipboard-005; `packages/d2b-contract-tests/`

Update `packages/d2b-contract-tests/tests/policy_clipboard.rs`:

- Add contract tests for `d2b.clipboard.bridge.v3` wire format.
- Add contract tests for `d2b.clipboard.picker-coord.v3`.
- Add tests verifying ReasonCode proto numbers are stable across schema
  versions (closed enum numeric stability).
- Verify attachment class names match descriptor declarations.
- Remove tests that assume shared filesystem bridge paths or SO_PEERCRED config.

### ADR046-clipboard-012 - Remove nixos-modules/clipboard.nix
| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-clipboard-006; Nix/module removal owner |
| Current source | nixos-modules/clipboard.nix and its import in nixos-modules/default.nix |
| Reuse action | delete-after-cutover |
| Destination | nixos-modules/default.nix, docs/how-to/ migration guide, tests/static.sh example iteration expectations, CHANGELOG.md |
| Detailed design | After the new Provider Nix module validates, delete nixos-modules/clipboard.nix, remove its default.nix import, update migration guide with the option mapping table, ensure tests/static.sh examples no longer rely on removed paths, and add an Unreleased changelog entry. Primary reuse disposition: `delete-after-cutover`. Preserved source-plan detail: delete superseded module and update migration docs. |
| Integration | Nix module aggregator, docs migration flow, static example iteration, and changelog all reflect Provider/clipboard-wayland as the only clipboard surface. |
| Data migration | Full d2b 3.0 reset; no v2 clipboard runtime state or option import |
| Validation | New module eval tests pass and tests/static.sh example iteration has no references to removed option paths. |
| Removal proof | nixos-modules/clipboard.nix is deleted, default.nix import removed, and grep/static checks show no old d2b.clipboard option references in examples. |

**Type:** removal  
**Inputs:** ADR046-clipboard-006

After `ADR046-clipboard-006` lands and is validated:

- Delete `nixos-modules/clipboard.nix`.
- Remove the `clipboard.nix` import from `nixos-modules/default.nix`.
- Update `docs/how-to/` migration guide with option mapping table from this
  dossier's Nix migration section.
- Ensure `tests/static.sh` example iterations do not fail on removed option
  paths.
- Add a CHANGELOG entry under `## [Unreleased]` noting the removal.

---

## Reuse from baseline

The following algorithms and types are ported verbatim from the
`packages/d2b-clipd/` baseline (algorithm preservation invariant from
ADR 0042):

| Source | Destination | Notes |
| --- | --- | --- |
| `src/policy.rs` - `ALLOWED_MIME_TYPES`, `SECRET_HINT_MIME_TYPES`, `ReasonCode`, `AttributionQuality` | `service/policy.rs` | Exact port; rename `source_realm`/`dest_realm` → `_zone_id` |
| `src/fd.rs` - `FdCapModel`, `FdSafetyError`, MSG_CTRUNC validation | `service/fd.rs` | Exact port |
| `src/audit.rs` - `AuditEvent`, fail-closed queue | `service/audit.rs` | Port; adapt zone field names; add `SizeBucket` |
| `src/framing.rs` - `PICKER_TO_DAEMON_MAX_FRAME_BYTES`, encode/decode | `service/picker_coord.rs` | Adapt to ComponentSession named-stream framing |
| `packages/d2b-wayland-proxy/src/clipboard.rs` - `ClipboardMimePolicy`, `ClipboardRoute`, `ClipboardObjectForwarding` | wayland-proxy crate (display-wayland Provider) | Not ported here; clipboard-wayland consumes via bridge service |

The following are **not** ported and must be rewritten:

| Baseline | Replacement |
| --- | --- |
| `src/picker.rs` subprocess spawn | `EphemeralProcess/picker-<id>` via resource API |
| `src/niri.rs` NiriJsonClient | `d2b.display.host-clipboard.v3` focus event stream |
| Unix bridge socket server | `d2b.clipboard.bridge.v3` ComponentSession service |
| Bridge directory / `clipboard-bridge-root` Volume | None (no filesystem bridge) |
| NIRI_SOCKET / WAYLAND_DISPLAY injection | `d2b.display.host-clipboard.v3` + ProviderSupervisor pre-opened WAYLAND_SOCKET FD |
| `SO_PEERCRED` bridge peer validation | Enrolled KK ComponentSession authentication |
| Per-Guest Unix groups and ACLs | Zone RBAC RoleBinding |

---

## Required source and test layout

The following root directories must exist before ADR046-clipboard-001 closes:

```text
packages/d2b-provider-clipboard-wayland/
  Cargo.toml
  src/
  tests/
  integration/
  README.md
```

`README.md` must cover: purpose, component map, local build instructions, test
commands, and the display-wayland fake used by integration tests.

`packages/d2b-contract-tests/tests/policy_clipboard.rs` must be updated
as part of ADR046-clipboard-011. `nixos-modules/clipboard.nix` is removed in
ADR046-clipboard-012.

### Fast hermetic execution and test placement (D094)

Per D094 and `ADR-046-validation-and-delivery` §10.16, this Provider's `src/`
unit tests and `tests/*.rs` hermetic suite are fast, in-process, deterministic,
and parallel-safe: an individual normal test has an advisory wall-clock p95
diagnostic threshold of <=50 ms; gate enforcement is aggregate per-crate
process CPU only. There is no wall-clock
sleep, and `cargo test -p d2b-provider-clipboard-wayland --lib --tests`
completes in ≤2 s warm-cache execution time (compilation excluded). They use a
deterministic fake clock/RNG and the toolkit fakes/FakeEffectPort only - no
process spawn, container, network, DBus, systemd, broker daemon, Nix eval/build,
KVM, USB/GPU/TPM hardware, or live cloud, and no filesystem tree beyond tiny
temp fixtures. Any scenario needing those lives only in `integration/`, which
keeps a lane timeout/budget, parallel isolation, and fake external services by
default; such a need is re-placed into `integration/`, never given a sleep,
larger timeout, or `#[ignore]`. Bounded crypto/property tests are the only
classified exception, each named with a capped case count and a declared higher
per-test advisory threshold.

Per D094, each replaced current-code test is retired with an explicit
keep/adapt/move/delete disposition and a removal gate: the minimum reusable
semantic assertions migrate into this crate's hermetic `tests/`, and the old
duplicate tests, shell gates, fixtures, static artifacts, CI jobs, and manifest
entries are deleted once successor coverage and the removal proof pass -
updating `tests/layer1-jobs.json`, the closed gate manifests, the
flake/matrix/Nix-unit pins, the generated ledgers, and the CI workflow shards.
Old and new suites never run in parallel indefinitely.
