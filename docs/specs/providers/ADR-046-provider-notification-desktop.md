# ADR 0046 Provider: notification-desktop

| Field | Value |
| --- | --- |
| Spec ID | `ADR-046-provider-notification-desktop` |
| Parent | ADR 0046 |
| Status | Proposed |
| Version | 2 |
| Baseline | `b5ddbed67867d9244bf33390868101bd9b053e49` |
| Normative | Yes |
| Owners | `packages/d2b-provider-notification-desktop/` |
| Provider crate | `packages/d2b-provider-notification-desktop/` |
| providerRef | `Provider/notification-desktop` |
| Depends on | `ADR-046-provider-model-and-packaging`, `ADR-046-componentsession-and-bus`, `ADR-046-resource-reconciliation`, `ADR-046-telemetry-audit-and-support`, `ADR-046-nix-configuration`, `ADR-046-provider-state`, `ADR-046-primitive-resource-composition` |
| Supersedes | None |
| Runtime dependency | `Provider/display-wayland` (required for host desktop sink processes) |

---

## 1. Purpose

`Provider/notification-desktop` routes typed desktop notification events from
Guest sources (VM workloads, guest-control, container guests) to the Host
desktop notification sink over authenticated ComponentSession streams.  It
owns:

- the `d2b.notification.v3` service package and its guest-source and
  host-sink component processes;
- the bounded `NotificationRequest` and `NotificationResult` DTOs and
  named-stream record types in `d2b.notification.v3`;
- the bounded schema for notification fields, action capabilities, and icon
  references;
- the `DesktopNotificationSink` write-only named stream for the host process
  that calls `org.freedesktop.Notifications`;
- the `DesktopNotificationObserver` read-only named stream for authenticated
  desktop clients;
- action capability issuance, replay detection, and TTL enforcement;
- in-memory per-session host-sink projections and action nonces (no durable
  resource object; state persists only for the bounded session lifetime);
- audit of connection events and action invocations (never of notification
  content);
- OTEL metrics and traces using closed labels only (never notification
  contents, summary, body, icon paths, or action text);
- lifecycle and drain for host-sink and guest-source component processes;
- Nix configuration, placement templates, and removal proof requirements.

Notification delivery is fully transient: `NotificationRequest` and
`NotificationResult` are named-stream DTOs, not resource objects.  There is
no ResourceSpec, no revision, no `watch` verb, no finalizer, and no Volume
backing for any notification delivery event.  Only the bounded in-memory
action nonce store persists entries within a single host-sink session
lifetime.

Notification contents — summary, body, icon, action labels, correlation
bytes, VM names in notification bodies, and every byte of presented text —
are **never** written to logs, metrics, audit records, traces, OTEL spans, or
any persistent state.

---

## 2. Crate layout

```text
packages/d2b-provider-notification-desktop/
  src/
    lib.rs                    # crate root; forbid(unsafe_code)
    types.rs                  # NotificationRequest/NotificationResult DTOs,
                              #   stream record types, category/urgency enums
    controller.rs             # Process placement controller — watches guestSources
                              #   Guest refs; creates/manages guest-source Processes
    guest_source.rs           # guest-side source process — validates requests,
                              #   emits NotificationRequest records over stream
    host_sink.rs              # host-side sink process — consumes stream, calls D-Bus,
                              #   manages observer projection
    action_nonce.rs           # bounded single-use action capability store
    stream_admission.rs       # ComponentSession admission checks
    redact.rs                 # sanitize() — strip/cap notification text before use
    error.rs                  # typed stable error enum; no content in messages
  tests/
    stream_record.rs          # NotificationRequest/NotificationResult DTO schema,
                              #   field bounds, closed category set vectors
    stream_redaction.rs       # assert notification content never reaches metric/log
    action_nonce.rs           # single-use, TTL, capacity, replay vectors
    stream_admission.rs       # auth/contract/transport rejection vectors
    observer_projection.rs    # bounded read-only in-memory projection state machine
    fault_injection.rs        # disconnect, drain, backpressure, timeout
  integration/
    cross_zone_source.rs      # Guest→Host ComponentSession stream integration
    dbus_sink.rs              # host-sink → real D-Bus session notification round trip
    observer_client.rs        # authenticated desktop observer session
    action_invoke.rs          # action capability issuance and invocation end-to-end
  README.md
```

Workspace policy rejects a Provider crate missing any of `src/`, `tests/`,
`integration/`, or `README.md`.  The dossier itself is the per-Provider record
required by `ADR-046-provider-model-and-packaging` §"Provider dossier
requirement".

---

## 3. Provider identity and ResourceSpec

### 3.1 ResourceType

```text
Provider/notification-desktop
```

Installed as a `Provider` resource in the Zone where the host desktop sink
runs.

### 3.2 Exported ResourceTypes

Per D089, any notification-desktop-owned ResourceType uses its typed desired
spec as the ResourceType base spec (Layer 2): top-level `spec.*`, including
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

This Provider exports no ResourceTypes.  Notification delivery is fully
transient: `NotificationRequest` and `NotificationResult` are named-stream
DTOs with no durable desired state.  They have no `spec`, no `revision`, no
`watch` verb, no finalizer-worthy identity, and no Volume backing.  Only
in-memory host-sink projections and action nonces persist, bound to the
lifetime of a single host-sink process session.

### 3.3 Provider spec fields

The canonical Provider spec shape is `spec = { artifactId; config = { ... }; }`.
All operator-facing configuration is nested under `spec.config`.

| Field | Type | Required | Default | Notes |
| --- | --- | --- | --- | --- |
| `spec.artifactId` | bounded ID | yes | — | Nix artifact catalog entry |
| `spec.config.hostExecutionRef` | ResourceRef | yes | — | `Host/<name>` in same Zone; controller and host-sink are placed on this Host |
| `spec.config.hostUserRef` | ResourceRef | yes* | — | `User/<name>` in same Zone; user domain for host-sink; required when `dbusSinkEnabled = true`; must match the `display-wayland` session user |
| `spec.config.maxPendingNotifications` | u32 | no | 64 | Oldest unanswered entry dropped when exceeded; range `[8, 1024]` |
| `spec.config.actionNonceTtlSecs` | u32 | no | 120 | Single-use capability TTL; range `[30, 600]` |
| `spec.config.actionNonceStoreSize` | u32 | no | 256 | Maximum live capabilities per sink process; range `[64, 4096]` |
| `spec.config.acknowledgeTimeoutSecs` | u32 | no | 3600 | Auto-drop projection entry if no observer ack within this window |
| `spec.config.dbusSinkEnabled` | bool | no | true | Enable `host-sink` D-Bus process |
| `spec.config.observerEnabled` | bool | no | true | Enable authenticated observer named stream |
| `spec.config.displayWaylandRef` | ResourceRef | yes* | — | `Provider/display-wayland` in same Zone; required when `dbusSinkEnabled = true`; its session identity must match `hostUserRef` |
| `spec.config.guestSources` | `[GuestSourceSpec]` | no | `[]` | Guests for which a `guest-source` Process is created |

`GuestSourceSpec` fields:

| Field | Type | Required | Default | Notes |
| --- | --- | --- | --- | --- |
| `guestRef` | ResourceRef | yes | — | `Guest/<name>` in same Zone; controller watches this ref |
| `categories` | `[string]` | yes | — | Non-empty subset of the closed category set (§4.3); guest-source rejects out-of-list categories |

The controller creates one `guest-source` Process per entry in
`spec.config.guestSources` and watches the referenced Guest resource.
When a referenced Guest is deleted or becomes `Failed`, the controller
drains and removes the corresponding `guest-source` Process.

No field accepts raw paths, credentials, D-Bus addresses, UID values, or
freeform strings that could become control authority.

Compiled Provider resource YAML:

```yaml
apiVersion: resources.d2bus.org/v3
type: Provider
metadata:
  name: notification-desktop
  zone: dev
  ownerRef: null
spec:
  artifactId: provider-notification-desktop
  config:
    hostExecutionRef: Host/host-system
    hostUserRef: User/operator
    maxPendingNotifications: 64
    actionNonceTtlSecs: 120
    actionNonceStoreSize: 256
    acknowledgeTimeoutSecs: 3600
    dbusSinkEnabled: true
    observerEnabled: true
    displayWaylandRef: Provider/display-wayland
    guestSources:
      - guestRef: Guest/work-vm
        categories:
          - security.event
          - transfer.complete
          - transfer.error
          - device.added
          - device.removed
status:
  phase: Pending
```

---

## 4. Named-stream DTOs and bounded fields

Notifications are transient bus payload.  The canonical carrier types for
`d2b.notification.v3` are `NotificationRequest` (guest-source → host-sink) and
`NotificationResult` (host-sink → guest-source).  Both are named-stream records,
not ResourceType specs.

### 4.1 NotificationRequest fields

| Field | Type | Bound | Notes |
| --- | --- | --- | --- |
| `summary` | string | max 256 chars | Required. Sanitized before D-Bus call: control chars → REPLACEMENT CHARACTER U+FFFD, whitespace collapsed, truncated at bound. Never logged. |
| `body` | string | max 2048 chars | Optional. Same sanitization. Never logged. |
| `iconRef` | bounded ID | max 64 chars | Optional. Closed set of declared icon IDs from the Provider's signed icon catalog. No host paths. |
| `urgency` | enum | `low` \| `normal` \| `critical` | Optional; default `normal`. |
| `category` | enum | closed set | Required; see §4.3. |
| `expireTimeoutSecs` | u32 | 0–3600 | 0 = no timeout. |
| `actions` | `[]ActionSpec` | max 4 entries | Optional. See §4.2. |
| `correlationId` | opaque bytes | max 64 chars | Optional. Stable cross-request join key. Never logged or labeled. |
| `idempotencyKey` | opaque bytes | max 64 chars | Optional. Controls replay protection for duplicate delivery within a session. |

### 4.2 ActionSpec fields

| Field | Type | Bound | Notes |
| --- | --- | --- | --- |
| `id` | bounded ID | max 32 chars | Unique within this request. Pattern `^[a-z][a-z0-9-]*$`. |
| `label` | string | max 64 chars | Display text. Sanitized before D-Bus. Never logged. |

The `id` is a stable machine identifier.  It is never exposed outside the
action capability path.  The `label` is presentation-only; it does not select
any operation.  Both are sanitized by `redact::sanitize()` before the D-Bus
call.  Neither appears in metrics, audit, traces, or logs.

### 4.3 Category closed set

```text
device.added | device.removed | device.error
network.connected | network.disconnected | network.error
presence.online | presence.offline
security.event | security.error
transfer.complete | transfer.error | transfer.cancelled
update.available | update.downloading | update.ready | update.error
system.info | system.warning | system.error
```

Unknown category values are rejected at admission.  The category is emitted as
a metric label (`notification_category`) at a stable closed cardinality.

### 4.4 NotificationResult fields

| Field | Type | Notes |
| --- | --- | --- |
| `outcome` | enum | `accepted` \| `replaced` \| `timeout` \| `dbus-error` \| `sink-unavailable` \| `capacity-exceeded` |
| `actionNonces` | `map<action-id, nonce>` | Present when `outcome = accepted` and request had actions; nonces are single-use TTL-bound opaque strings. Map key is the action `id` from ActionSpec; never logged. |

`actionNonces` is included in the result returned to the `guest-source` so that
action invocations can be routed by the observer.  The guest-source passes
nonces through to the observer client over the `DesktopNotificationObserver`
projection; the guest-source itself does not invoke actions.

No NotificationResult field carries notification content, summary, body, icon
path, or action text.

---

## 5. Components and processes

### 5.1 Overview

| Component ID | Type | Binary | Domain | Cardinality | Description |
| --- | --- | --- | --- | --- | --- |
| `notification-desktop-controller` | controller | `d2b-notification-desktop-controller` | system | 1 per Zone Host | Watches `guestSources` Guest refs; manages guest-source and host-sink Process lifecycle; does not create Volumes (ProviderDeployment responsibility) |
| `notification-desktop-host-sink` | service | `d2b-notification-desktop-host-sink` | user | 1 per user Wayland session | Receives `DesktopNotificationSink` named stream; calls D-Bus |
| `notification-desktop-guest-source` | service | `d2b-notification-desktop-guest-source` | system | 1 per `guestSources` entry | Accepts authenticated ComponentSession `NotificationRequest` streams from guest workloads; forwards as stream records to host-sink |

No process uses `startRoot = true`.  No process receives a broker socket, a
cgroup path, a raw credential, or a host path via config.

### 5.2 controller Process template

```yaml
apiVersion: resources.d2bus.org/v3
type: Process
metadata:
  name: notification-desktop-controller
  zone: <zone>
  ownerRef: Provider/notification-desktop
spec:
  providerRef: Provider/system-minijail
  executionRef: <spec.config.hostExecutionRef>
  domain: system
  processClass: controller
  template: notification-desktop-controller
  sandbox:
    namespaceClasses: [mount, ipc]
    capabilityClasses: []
    seccompClass: notification-desktop-controller-v1
    noNewPrivileges: true
    startRoot: false
    readOnlyRoot: true
    environmentClass: minimal
  budget:
    memory:
      request: "16Mi"
      limit: "32Mi"
    pids:
      limit: 32
    fds:
      limit: 256
  readiness:
    initialDelay: "0s"
    timeout: "5s"
    failureThreshold: 3
    successThreshold: 1
    class: ready-condition
  restartPolicy:
    class: on-failure
    backoffBase: "1s"
    backoffMax: "60s"
    backoffMultiplier: 2.0
    maxRestarts: null
    resetAfter: "300s"
  mounts:
    - volumeRef: Volume/notification-desktop--controller--runtime--<host-short>
      view: main
      mountPath: /state
      access: read-write
      required: true
  drainTimeout: "30s"
```

The `controller` Process manages guest-source Process resources (one per
`spec.config.guestSources` entry) and manages the host-sink Process lifecycle
as a function of `Provider/display-wayland` readiness.  It holds no in-memory
notification state; all delivery state is in-session at the host-sink.

### 5.3 host-sink Process template

```yaml
apiVersion: resources.d2bus.org/v3
type: Process
metadata:
  name: notification-desktop-host-sink
  zone: <zone>
  ownerRef: Provider/notification-desktop
spec:
  providerRef: Provider/system-systemd
  executionRef: <spec.config.hostExecutionRef>
  domain: user
  userRef: <spec.config.hostUserRef>
  processClass: service
  template: notification-desktop-host-sink
  sandbox:
    namespaceClasses: [mount, ipc, pid]
    capabilityClasses: []
    seccompClass: notification-desktop-host-sink-v1
    noNewPrivileges: true
    startRoot: false
    readOnlyRoot: true
    environmentClass: provider-defined
  budget:
    memory:
      request: "8Mi"
      limit: "16Mi"
    pids:
      limit: 16
    fds:
      limit: 128
  readiness:
    initialDelay: "0s"
    timeout: "3s"
    failureThreshold: 3
    successThreshold: 1
    class: ready-condition
  restartPolicy:
    class: on-failure
    backoffBase: "1s"
    backoffMax: "30s"
    backoffMultiplier: 2.0
    maxRestarts: null
    resetAfter: "300s"
  mounts:
    - volumeRef: Volume/notification-desktop--host-sink--runtime--<host-short>
      view: main
      mountPath: /state
      access: read-write
      required: true
  drainTimeout: "10s"
```

The `host-sink` process runs as the same UID that owns the Wayland compositor
session.  It obtains an authenticated pre-opened D-Bus connection through a
ComponentSession to `Provider/display-wayland` under same-UID policy; the D-Bus
session socket path is **never** read from a status field, environment variable,
or ambient config.  See §15 for the full dependency protocol.

### 5.4 guest-source Process template

```yaml
apiVersion: resources.d2bus.org/v3
type: Process
metadata:
  name: notification-desktop-guest-source-<guest-name>
  zone: <zone>
  ownerRef: Provider/notification-desktop
spec:
  providerRef: Provider/system-minijail
  executionRef: Guest/<vm-name>
  domain: system
  processClass: service
  template: notification-desktop-guest-source
  sandbox:
    namespaceClasses: [mount, ipc]
    capabilityClasses: []
    seccompClass: notification-desktop-guest-source-v1
    noNewPrivileges: true
    startRoot: false
    readOnlyRoot: true
    environmentClass: minimal
  budget:
    memory:
      request: "4Mi"
      limit: "8Mi"
    pids:
      limit: 8
    fds:
      limit: 64
  readiness:
    initialDelay: "0s"
    timeout: "3s"
    failureThreshold: 3
    successThreshold: 1
    class: ready-condition
  restartPolicy:
    class: on-failure
    backoffBase: "1s"
    backoffMax: "30s"
    backoffMultiplier: 2.0
    maxRestarts: null
    resetAfter: "300s"
  mounts:
    - volumeRef: Volume/notification-desktop--guest-source--runtime--<guest-short>
      view: main
      mountPath: /state
      access: read-write
      required: true
  drainTimeout: "5s"
```

One `guest-source` Process is created per entry in `spec.config.guestSources`.
It runs inside the referenced Guest's process tree (via the Guest's Process
Provider), accepts authenticated `NotificationRequest` records from guest
workloads over an authenticated ComponentSession, validates and bounds-checks
fields against the configured category filter, and forwards them as
`NotificationRequest` records over the `DesktopNotificationSink` named stream
to the `host-sink`.

---

## Endpoint resources (D092)

`Provider/notification-desktop` declares standard `Endpoint` base-schema
conformance. Stable notification service identities are owned `Endpoint`
resources with `producerRef`; they are not inline `Process.spec` fields.
Consumers use `Endpoint/<name>` references. Endpoint spec/status/CLI/audit/
telemetry never include raw socket paths, D-Bus locators, notification content
bytes, action payloads, fds, or credentials. Resolution occurs only through an
authorized EffectPort/LaunchTicket; unauthorized resolution returns
`endpoint-resolve-denied`. Producer restart bumps
`Endpoint.status.endpointGeneration`, which triggers `dependency-changed` for
consumers.

Representative owned Endpoint resources:

```yaml
apiVersion: resources.d2bus.org/v3
type: Endpoint
metadata:
  name: notification-desktop-sink
  zone: dev
  ownerRef: Provider/notification-desktop
spec:
  providerRef: Provider/notification-desktop
  producerRef: Process/notification-desktop-host-sink
  endpointClass: service
  transport: unix
  purpose: notification-desktop.d2bus.org/host-sink
  serviceFingerprint: notification-desktop.d2bus.org/DesktopNotificationSink.v3
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
  name: notification-desktop-observer
  zone: dev
  ownerRef: Provider/notification-desktop
spec:
  providerRef: Provider/notification-desktop
  producerRef: Process/notification-desktop-host-sink
  endpointClass: service
  transport: unix
  purpose: notification-desktop.d2bus.org/observer
  serviceFingerprint: notification-desktop.d2bus.org/DesktopNotificationObserver.v3
  locality: host-local
  visibility: authorized-consumers
  attachmentPolicy: component-session
  consumerPolicy: observer-authorized
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
  name: notification-desktop-source-<guest-name>
  zone: dev
  ownerRef: Provider/notification-desktop
spec:
  providerRef: Provider/notification-desktop
  producerRef: Process/notification-desktop-guest-source-<guest-name>
  endpointClass: service
  transport: vsock
  purpose: notification-desktop.d2bus.org/guest-source
  serviceFingerprint: notification-desktop.d2bus.org/NotificationSource.v3
  locality: cross-domain
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

## Retained opaque handles

- pidfds: Process supervision handles, not stable service identities.
- Per-connection/session handles: action nonces, ComponentSession IDs, and
  notification delivery IDs are high-churn interaction handles.
- Named streams: `DesktopNotificationSink` and `DesktopNotificationObserver`
  carry records; they are payload channels behind Endpoint resolution, not
  Endpoint identities.
- `OwnedTransport`: authenticated session transport ownership is an in-memory
  capability.
- fd indexes: pre-opened D-Bus and stream descriptors are LaunchTicket-local
  numbers and remain opaque.

---

## 6. ComponentSession and d2b-bus integration

### 6.1 Service package

```text
d2b.notification.v3
```

Version, schema fingerprint, and transport/purpose/limit binding are in the
Noise prologue.  There is no v2 interoperability.

### 6.2 Named streams

| Stream kind | Direction | Admitted principals | Description |
| --- | --- | --- | --- |
| `DesktopNotificationSink` | guest-source → host-sink | `Process/notification-desktop-guest-source-<name>` | One `NotificationRequest` record per message; returns `NotificationResult`; bounded credits; no FD transfer |
| `DesktopNotificationObserver` | host-sink → observer client | Authorized desktop client (Role/RoleBinding) | Read-only projection of current/recent delivery events; bounded credits |

### 6.3 Stream record protocol

`DesktopNotificationSink` uses a request/result record protocol:

1. Guest-source sends a `NotificationRequest` record on the stream.
2. Host-sink validates fields (sanitize, category filter, bounds), submits to
   D-Bus `org.freedesktop.Notifications.Notify`, issues action nonces if
   actions present, then returns a `NotificationResult` record.
3. On `outcome = accepted`, result carries opaque `actionNonces` for any
   declared actions; guest-source passes these through to the observer
   projection so `InvokeAction` calls can be authorized.
4. Backpressure: stream credit is consumed on send; credit is restored when
   the `NotificationResult` is delivered.  If `maxPendingNotifications` is
   exceeded the oldest unanswered projection entry is evicted and
   `d2b_notification_drop_total{reason="capacity"}` increments.

`DesktopNotificationObserver` uses a unidirectional notification event stream
carrying redacted delivery events.  Observer clients call `InvokeAction` as
a separate method on the same session.

### 6.4 Noise profiles

- Guest→Host sink stream: `Noise_KK_25519_ChaChaPoly_SHA256` (enrolled;
  guest-source identity registered during Guest bootstrap).
- Desktop observer client: `Noise_NN_25519_ChaChaPoly_SHA256` (local Unix
  seqpacket; SO_PEERCRED maps to authorized user subject).
- Action `InvokeAction`: same session as observer; no new handshake.

NN is allowed for the observer because:

- purpose class is local;
- transport is Unix seqpacket with SO_PEERCRED;
- authenticated subject is resolved from SO_PEERCRED UID → `User/<name>`;
- endpoint policy is trusted/config-generated.

KK is required for the Guest→Host sink because the guest-source crosses a Zone
boundary (Host Zone ← Guest Zone vsock transport).

### 6.5 Admission checks — stream_admission.rs

`stream_admission.rs` checks on every session establishment:

1. Session is established and authenticated (`is_established() && is_authenticated()`).
2. Service package exactly equals `d2b.notification.v3`.
3. Endpoint purpose exactly equals one of the allowed purpose classes
   (`desktop-observer` or `notification-source`).
4. For `notification-source`: transport is vsock or enrolled KK.
5. For `desktop-observer`: transport is Unix seqpacket (`uses_pre_authorized_transport()`).
6. Schema fingerprint matches the installed Provider component descriptor.

Any mismatch returns a typed stable error and closes the session.  No
negotiation, downgrade, or fallback occurs.

### 6.6 Credit and backpressure

- `DesktopNotificationSink`: per-stream byte credit; `maxPendingNotifications`
  is the soft high-water per host-sink; oldest unanswered projection entry is
  dropped and `d2b_notification_drop_total{reason="capacity"}` increments.
- `DesktopNotificationObserver`: separate credit; observer backpressure does
  not block the sink stream.  A slow observer causes its own projection
  messages to be dropped; it does not block delivery to the D-Bus session.
- Session control channel has reserved capacity; a blocked sink or observer
  stream cannot starve keepalive, cancel, or health traffic.

---

## 7. Notification field sanitization

All user-controlled text fields pass through `redact::sanitize()` before use:

- Control characters (except space) are replaced with U+FFFD.
- Tab, CR, and LF are replaced with a single space.
- Consecutive whitespace is collapsed to one space.
- String is truncated to `max_chars` at the Unicode code-point boundary.

`summary` bound: 256 chars.  `body` bound: 2048 chars.  `actions[].label`
bound: 64 chars.  `iconRef` bound: 64 chars.

Sanitized values are used only for the D-Bus `org.freedesktop.Notifications`
call and the `DesktopNotificationObserver` projection.

**Notification contents never appear in:**

- `tracing` spans, events, or fields;
- OTEL metric labels or span attributes;
- authoritative audit records;
- `Debug` formatting of any public type;
- error messages or error `Display` output;
- the in-memory action nonce store (contains only opaque nonces and timestamps,
  never content);
- any log emitted by any component process.

This invariant is tested by `tests/stream_redaction.rs`, which injects
notifications with distinguishable payloads and asserts that no collected
tracing event, metric label, or audit record contains those bytes.

---

## 8. Action capability issuance and replay protection

### 8.1 ActionNonce

An `ActionNonce` is a 256-bit cryptographically random value (32 raw bytes,
hex-encoded to 64 lowercase hex chars).  It is issued by the host-sink process
when it presents an action to the D-Bus session.

Structure:

```text
ActionNonce = 64 lowercase-hex chars derived from getrandom(2)
```

The nonce is NOT the D-Bus notification ID.  The D-Bus notification ID is an
opaque u32 returned by `org.freedesktop.Notifications.Notify` and held only
in the host-sink's in-memory projection for the session lifetime.

### 8.2 ActionNonceStore

Held in the host-sink process heap for the lifetime of that process session
(in-memory only; no Volume, no disk backing).

Properties:

| Property | Value |
| --- | --- |
| `NONCE_TTL_SECS` | 120 |
| `MAX_STORE_SIZE` | 256 |
| `NONCE_BYTES` | 32 |

Rules:

- Issuance fails with `action-capability-capacity` when the store is full.
- Consumption fails with `action-capability-expired` when `now >= issued_at + TTL`.
- After a failed TTL check, the nonce is unconditionally removed (consumed).
- Consumption of a missing or already-consumed nonce fails with
  `action-capability-unavailable`.
- The nonce bytes are redacted in `Debug` output.
- The store is never logged or audited; capacity/issuance/expiry counters are
  OTEL metrics with no per-nonce labels.

The notification action key delivered to the D-Bus session is:

```text
d2b-action:<hex-nonce>
```

The prefix `d2b-action:` is a fixed constant.  The desktop notification server
includes this string in the `ActionInvoked` signal; the host-sink parses it,
validates the prefix, and routes to `InvokeAction` on the authorized
ComponentSession.

### 8.3 InvokeAction authorization

`InvokeAction` is admitted only on the `desktop-observer` session of the same
authenticated subject that holds the observer stream.  It is not accessible to
the guest-source session.  The action ID inside the request is matched
server-side; the caller never receives the mapping from nonce to action ID or
notification ResourceRef.

---

## 9. RBAC

### 9.1 Roles

Roles cover ComponentSession service verbs and Process management authority.
This Provider exports no ResourceTypes; there are no resource-level verbs for
`DesktopNotification`.

| Role name | Subjects | Verbs | Resource / Stream | Notes |
| --- | --- | --- | --- | --- |
| `notification-desktop-controller` | `Process/notification-desktop-controller` | `get,list,watch,update-spec,update-status` | `Process` (owned by `Provider/notification-desktop`) | Controller places and manages guest-source Processes |
| `notification-desktop-sink-service` | `Process/notification-desktop-host-sink` | `connect,open-stream` | `DesktopNotificationSink` stream | Host-sink accepts inbound notification records from guest-sources |
| `notification-desktop-source` | `Process/notification-desktop-guest-source-*` | `connect,open-stream` | `DesktopNotificationSink` stream | Guest-source stream write authority |
| `notification-desktop-observer` | Authorized user subjects | `connect,open-stream,invoke` | `DesktopNotificationObserver` stream; `InvokeAction` method | Read-only observer and action invocation |

Zone config binds each RoleBinding to exact subjects.  No role grants stream
authority outside the declared purpose.  No role grants `update-spec` on any
Provider resource to observer subjects.

### 9.2 Session authorization

ComponentSession authorization is revision-bound (see
`ADR-046-componentsession-and-bus` §"Native RBAC integration").  For the
observer session:

- verb = `open-stream`, stream = `DesktopNotificationObserver` is authorized
  to observer subjects;
- verb = `invoke`, method = `InvokeAction` is authorized to the observer
  subject that holds the open observer stream;
- verb = `open-stream`, stream = `DesktopNotificationSink` is denied to
  observer subjects;
- verb = `connect` on `notification-source` purpose is denied to observer
  subjects.

Authorization is cached under exact attributes with short expiry.  Relevant
Role/RoleBinding deletion invalidates the session lease immediately and closes
in-progress streams.

---

## 10. Security and privacy invariants

### 10.1 Notification content privacy

Notification contents (summary, body, action labels, icon identifiers, any
VM-sourced text) are private to the presentation layer.  They are:

- never stored outside the in-process heap (action nonces are in-memory only;
  not content; do not survive restart);
- never in any audit record;
- never in any OTEL span attribute or metric label;
- never in any `tracing` field;
- never in any stable error message;
- never in any `Debug` implementation.

### 10.2 Notification source isolation

Guest source processes are isolated from each other and from the host-sink.
One guest-source holds a ComponentSession to the host-sink.  The host-sink
processes inbound stream messages serially per guest-source session; it does not
share per-session state across guests.  Backpressure from one guest-source
cannot starve another.

### 10.3 D-Bus session boundary

The host-sink process calls `org.freedesktop.Notifications.Notify` on the
user D-Bus session.  It:

- does not accept the D-Bus session address from config, environment variable,
  or status field;
- obtains an authenticated pre-opened D-Bus connection FD through a
  ComponentSession to `Provider/display-wayland` under same-UID policy (see
  §15); the FD is passed at host-sink startup and is valid for the lifetime of
  the compositor session;
- does not open any other D-Bus service;
- does not forward D-Bus objects from the Guest;
- does not bridge or proxy arbitrary D-Bus methods;
- presents only the sanitized bounded fields: summary, body, urgency, timeout,
  and a closed action ID list.

The D-Bus `ActionInvoked` signal is the only inbound message from the desktop
notification server.  The host-sink parses only the notification ID (u32) and
the action key (string); it does not accept arbitrary D-Bus method calls.

### 10.4 Zone boundary crossing

The `guest-source → host-sink` path crosses the Guest Zone boundary.  The
crossing uses KK ComponentSession over vsock.  The Zone boundary does not
attenuate the content-privacy invariants: content is never logged or labeled on
either side.

### 10.5 Observer client trust

The observer client receives the `DesktopNotificationObserver` projection.
This projection may include sanitized summary/body text for rendering, but:

- the projection is never written to any on-disk state file that survives the
  session;
- the observer client must hold an authorized Role with `open-stream` on
  `DesktopNotificationObserver`;
- the observer client cannot invoke actions without a valid nonce, and nonces
  are single-use, TTL-bound, and issued only by the host-sink.

### 10.6 Icon reference safety

Icon references use a closed bounded ID that resolves to a path inside the
Provider's signed Nix store output.  No caller-supplied path, URI, URL, or
arbitrary filename is accepted.  The host-sink resolves the icon ID to a store
path using a sealed mapping derived at build time.  Unknown icon IDs are
rejected at admission.

---

## 11. Provider state

### 11.1 ProviderStateSet

A **ProviderStateSet** is the optional, query-time set of the *declared* Volume
resources in the Zone with `metadata.ownerRef = Provider/notification-desktop`.
It is not a ResourceType or stored artifact and is empty for a Provider that
declares no state Volume.

`Provider/notification-desktop` declares **no** Provider state Volume; its
`ProviderStateSet` is empty. Notification delivery state — the in-memory
projection and the action nonce store — remains exclusively in the host-sink
process heap and is never persisted. No notification summary, body, action
label, icon identifier, nonce, or content byte is ever written to durable
storage. Its bounded non-secret operational state — component readiness,
reconcile stage, and closed-enum error/health detail — lives in core-owned
Provider status and the core Operation ledger (D087).

Per D088, notification-desktop exports no semantic ResourceType and writes no
ResourceType status of its own. Core-owned Provider status uses universal
top-level `status.*` plus the Provider ResourceType-common `status.resource`;
optional `status.provider` is only for implementation observation
(`providerRef`, qualified immutable `schemaId`, semver `schemaVersion`, numeric
`observedProviderGeneration`, strict unknown-field-denied redacted `details`
≤32 KiB registered/signed in the Provider manifest) and never duplicates shared
fields. Core writes all present layers atomically in one status mutation.

Because this Provider's operational state is fully derivable from spec,
`status`, the core Operation ledger, and its live process memory, it fails the
storage-need test and declares no state namespace, no state Volume (neither
host, user-domain, nor guest-backed), no virtiofs Export for state, no
state-view mount, and no dedicated state-layout `User/<name>` principal. There
is no empty identity-only Volume. Its reconcile authority is over `Process`
resources only (see §16).

### 11.2 Status aggregation and restart semantics

`Provider/system-core` aggregates the overall Provider status from the health
reports emitted by component processes; the notification controller manages
child Process lifecycle and emits health reports but does not write
Provider-level status directly. D088 requires that any optional
`status.provider` extension name `providerRef`, a qualified immutable
`schemaId`, semver `schemaVersion`, numeric `observedProviderGeneration`, and
strict unknown-field-denied redacted `details` ≤32 KiB registered and signed in
the Provider manifest.

On host-sink restart the nonce store is empty; any action nonces issued before
restart are invalidated (`action-capability-unavailable` on the next invocation
attempt).  The in-memory projection is likewise empty; connected observer
clients receive a stream-close event and must reconnect.  The controller
re-derives component readiness from live `status` observation and reverifies
against the running processes, treating `status` as observation, never
authority (D087). No notification bytes, clipboard or terminal bytes, secrets,
paths, PIDs, unit names, or authority-conferring handles are ever persisted or
placed in any status layer.

D091 currency and upgrade: the notification-desktop controller implements
`assess_update`, `plan_upgrade`, and `execute_upgrade` for its qualified
ResourceTypes and semantic notification sessions. A `ProviderGenerationChanged`,
`ArtifactChanged`, `DependencyChanged`, or `SpecChanged` reason populates
universal `status.update` with
`UpdateAvailable` or `UpgradeRequired`; disruptive changes MUST return
`UpgradeRequired` rather than being applied in place, while non-disruptive
changes reconcile normally. These currency fields are universal/ResourceType
base fields, never `status.provider`. Upgrades recycle only the notification realization
(owned `Process` resources, endpoints, and sessions) with `disruption` set to
`Reload`, `Restart`, or `Recycle`; durable config is preserved, dependent
sessions and attachments are drained and restarted by the dependency-aware
planner, and owned ephemeral session state remains process memory. No
notification content bytes, clipboard bytes, terminal bytes, session bytes,
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

---

## 12. Lifecycle, status, and error handling

### 12.1 In-session notification lifecycle

Notification delivery is fully in-memory.  There is no ResourceType FSM, no
phase field, and no finalizer.  The host-sink maintains a bounded in-memory
projection keyed by opaque request handles:

```text
Request admitted
  → D-Bus Notify call dispatched
  → NotificationResult{outcome: accepted, actionNonces: {...}} returned to guest-source
  → Observer projection updated (sanitized fields)
  → [D-Bus NotificationClosed signal received]
  → Projection entry evicted; action nonces purged
```

If the D-Bus `Notify` call fails the host-sink returns
`NotificationResult{outcome: dbus-error}` immediately to the guest-source and
does not add an entry to the projection.  The guest-source returns the result
to its caller.

If `maxPendingNotifications` is exceeded before a new request is processed,
the oldest projection entry is evicted and its action nonces are invalidated
before the new request is processed.

### 12.2 Host-sink process lifecycle

The host-sink Process is created by the controller when `Provider/display-wayland`
transitions to `Ready`.  It is drained and stopped when `display-wayland`
becomes `NotReady` or `Failed`.  On drain, outstanding sink stream sessions
receive `NotificationResult{outcome: sink-unavailable}` for all pending
requests, then the stream is closed.  Observer clients receive a stream-close
event.

The action nonce store and in-memory projection are both in-process state;
neither survives restart.

### 12.3 Controller process lifecycle

The controller is the first Process created in the Zone when the Provider
transitions to `Pending → Ready`.  It performs an initial placement pass:

1. Reads `spec.config.guestSources` from the Provider spec.
2. For each `guestRef` entry, watches the referenced `Guest/<name>` resource.
3. Creates one `notification-desktop-guest-source-<guest-name>` Process per
   entry that references a Ready Guest.
4. Defers creation for Guests not yet Ready; re-evaluates on hint.
5. Watches `Provider/display-wayland`; creates or stops `host-sink` Process
   accordingly.
6. On `guestRef` deletion or permanent failure: drains and deletes the
   corresponding guest-source Process.

The controller never holds notification content or delivery state.

### 12.4 Drain

On `drain(deadline)` for the host-sink Process:

1. Stop accepting new `NotificationRequest` records on the sink stream.
2. Return `NotificationResult{outcome: drain-timeout}` for all pending requests
   within the deadline.
3. Clear all pending action nonces.
4. Close the observer stream.
5. Return `DrainResult::Complete` or `DrainResult::TimedOut`.

The guest-source process receives a drain signal before the host-sink drains.

### 12.5 Stable error codes

All error values use `snake_case` identifiers.  No error contains notification
content.

| Code | Stable | Notes |
| --- | --- | --- |
| `session-not-established` | yes | Admission: session not yet established |
| `session-unauthenticated` | yes | Admission: session not authenticated |
| `session-untrusted-transport` | yes | Admission: transport class not permitted |
| `session-contract-mismatch` | yes | Service package / purpose / schema mismatch |
| `notification-field-invalid` | yes | Bounded field validation failure (no content) |
| `notification-category-unknown` | yes | Category not in closed set |
| `notification-category-filtered` | yes | Category not in guest-source configured filter |
| `icon-ref-unknown` | yes | Icon ID not in sealed catalog |
| `action-id-invalid` | yes | Action ID fails pattern check |
| `action-label-too-long` | yes | Action label exceeds 64 chars |
| `capacity-exceeded` | yes | `maxPendingNotifications` projection full |
| `action-capability-invalid` | yes | Nonce format invalid |
| `action-capability-unavailable` | yes | Nonce missing, consumed, or store restarted |
| `action-capability-expired` | yes | Nonce past TTL |
| `action-capability-capacity` | yes | Nonce store full |
| `dbus-unavailable` | yes | D-Bus session not reachable |
| `dbus-method-error` | yes | D-Bus `Notify` returned an error |
| `sink-unavailable` | yes | host-sink process not Ready |
| `observer-unavailable` | yes | Observer stream closed |
| `drain-timeout` | yes | Drain exceeded deadline |

---

## 13. Telemetry, audit, and OTEL

### 13.1 Separation invariant

Telemetry (OTEL) and authoritative audit are distinct subsystems per
`ADR-046-telemetry-audit-and-support` §"Separation invariant".  No OTEL field
ever carries notification content, action text, or source identity beyond
opaque resource UID digests.

### 13.2 OTEL metrics (closed labels only)

| Metric | Type | Labels | Notes |
| --- | --- | --- | --- |
| `d2b_notification_created_total` | counter | `zone`, `category`, `urgency` | Count by category/urgency only |
| `d2b_notification_delivered_total` | counter | `zone`, `category`, `urgency`, `sink_result` | Terminal delivery outcome |
| `d2b_notification_action_invoked_total` | counter | `zone`, `category` | Action invoked; no action ID label |
| `d2b_notification_drop_total` | counter | `zone`, `reason` | `reason`: `capacity` \| `observer-backpressure` \| `sink-unavailable` |
| `d2b_notification_dbus_duration_seconds` | histogram | `zone`, `outcome` | D-Bus call latency |
| `d2b_notification_action_nonce_issued_total` | counter | `zone` | — |
| `d2b_notification_action_nonce_expired_total` | counter | `zone` | — |
| `d2b_notification_stream_sessions_active` | gauge | `zone`, `stream_kind` | Active named stream sessions |

**Labels that are explicitly excluded:**  summary, body, action label, action
ID, icon ref, source VM name, notification ResourceRef name, correlationId,
idempotency key, D-Bus notification ID, and any string derived from
notification content.

### 13.3 OTEL traces

Traces are created at notification record admission (root span on
`DesktopNotificationSink`) and `InvokeAction`.  Span attributes include:

| Attribute | Value |
| --- | --- |
| `d2b.zone` | Zone name |
| `d2b.provider` | `notification-desktop` |
| `d2b.component` | `host-sink` \| `guest-source` |
| `d2b.notification.category` | stable category token |
| `d2b.notification.urgency` | `low` \| `normal` \| `critical` |
| `d2b.notification.request_digest` | `sha256:<hex>` of the opaque request handle |

No span carries summary, body, action label, icon ref, or content-derived
values.  The `correlationId` from the request is carried as the W3C TraceContext
`tracestate` join key only, never as a span attribute containing content bytes.

### 13.4 Authoritative audit

Audit records are emitted for:

- `DesktopNotificationSessionEstablished` — on every admitted ComponentSession
  (source or observer).  Fields: zone, subject digest, stream kind, session
  generation digest, outcome.  No content.
- `DesktopNotificationActionInvoked` — on every consumed nonce.  Fields: zone,
  subject digest, request handle digest, outcome.  No content, no action ID.
- `DesktopNotificationDeliveryFailed` — when `dbus-error` or `dbus-unavailable`
  outcome is returned to a guest-source.  Fields: zone, outcome code only.
  No content.

Audit records are committed before the operation they describe completes.
Notification summary, body, action text, and icon ref never appear in any
audit field.

---

## 14. Nix configuration

### 14.1 Zone-level opt-in

```nix
d2b.zones.<zone>.resources.notification-desktop = {
  type = "Provider";
  spec = {
    artifactId = "provider-notification-desktop";
    config = {
      hostExecutionRef = "Host/host-system";
      hostUserRef = "User/operator";
      maxPendingNotifications = 64;
      actionNonceTtlSecs = 120;
      dbusSinkEnabled = true;
      displayWaylandRef = "Provider/display-wayland";
      guestSources = [];   # populated in §14.2
    };
  };
};
```

`notification-desktop` is default-disabled.  No notification component is
activated unless the Provider resource is explicitly declared.

### 14.2 Per-guest source opt-in

Guest notification sources are declared inside `spec.config.guestSources` of
the `Provider/notification-desktop` resource, not in the Guest resource spec.
The controller watches the referenced Guest resources and creates guest-source
Processes.

```nix
d2b.zones.<zone>.resources.notification-desktop = {
  type = "Provider";
  spec = {
    artifactId = "provider-notification-desktop";
    config = {
      hostExecutionRef = "Host/host-system";
      hostUserRef = "User/operator";
      maxPendingNotifications = 64;
      actionNonceTtlSecs = 120;
      dbusSinkEnabled = true;
      displayWaylandRef = "Provider/display-wayland";
      guestSources = [
        {
          guestRef = "Guest/work-vm";
          categories = [
            "security.event"
            "transfer.complete"
            "transfer.error"
            "device.added"
            "device.removed"
          ];
        }
      ];
    };
  };
};
```

A Guest that is not referenced in any `guestSources` entry receives no
guest-source Process.  The `categories` list constrains which category values
the guest-source process accepts; notifications with out-of-list categories are
rejected at the guest-source admission layer with
`notification-category-filtered`.

No field is added to `Guest.spec` for notification opt-in.

### 14.3 Artifact catalog entry

```nix
d2b.artifacts.provider-notification-desktop = {
  type = "provider";
};
```

The artifact catalog entry is separate from the Provider ResourceSpec.  The
`artifactId` is a plain bounded ID, not a ResourceRef.  The build resolves the
artifact from `d2b.artifacts` at build time; `packageDigest` is a build-
derived field and is not authored in the Provider spec.

### 14.4 Role and RoleBinding compilation

The Nix compiler emits the roles listed in §9 as `Role` and `RoleBinding`
resources with exact subject bindings derived from the Process resource names
generated by the controller placement templates.  No role uses a wildcard
subject.

### 14.5 Eval-time assertions

The Nix compiler enforces at eval time:

- `hostExecutionRef` resolves to a declared `Host/<name>` resource in the same Zone.
- `dbusSinkEnabled = true` requires `hostUserRef` to resolve to a declared
  `User/<name>` resource in the same Zone.
- `dbusSinkEnabled = true` requires `displayWaylandRef` to resolve to a
  declared `Provider/display-wayland` resource in the same Zone, and that
  Provider's configured session user must match `hostUserRef`.
- Each `guestSources[*].categories` is a non-empty subset of the closed
  category set (§4.3).
- Each `guestSources[*].guestRef` resolves to a declared `Guest/<name>`
  resource in the same Zone.
- `actionNonceTtlSecs` is in the range `[30, 600]`.
- `maxPendingNotifications` is in the range `[8, 1024]`.
- No duplicate `guestRef` values within `guestSources`.

---

## 15. `display-wayland` dependency

`Provider/notification-desktop` declares a required runtime dependency on
`Provider/display-wayland`:

```text
dependencies:
  display: Provider/display-wayland
```

The dependency alias `display` is resolved by Zone config to the exact
`Provider/display-wayland` resource fingerprint.

At runtime the controller watches `Provider/display-wayland` status.  When
`display-wayland` transitions to `Ready`, the controller creates the host-sink
Process.  When `display-wayland` becomes `NotReady` or `Failed`, the controller
drains and stops the host-sink process; all in-flight pending requests on the
sink stream receive `NotificationResult{outcome: sink-unavailable}`.

The dependency is synchronous: the controller's readiness condition
`NotificationSinkReady` remains `False` until `display-wayland` is `Ready`.
The `notification-desktop` Provider transitions to `Degraded` when the display
dependency is unavailable; it does not transition to `Failed`.

**D-Bus connection acquisition.**  The host-sink process does **not** read a
D-Bus session socket path from the `Provider/display-wayland` status field or
from any environment variable.  Instead, at host-sink startup the controller
opens a ComponentSession to `Provider/display-wayland` under same-UID policy —
using the UID of `spec.config.hostUserRef` — and receives an authenticated
pre-opened D-Bus connection FD.  This FD is passed to the host-sink process as
a sealed startup credential through the Process bootstrap protocol.  The FD is
valid for the lifetime of the compositor session; the host-sink does not
reconnect D-Bus independently.  The `displayWaylandRef` readiness is therefore
bound to the same authenticated user identity as `hostUserRef`; a mismatch
is a spec validation error (§14.5).

The `Provider/display-wayland` status resource never publishes a D-Bus address
or socket path.  No ambient environment lookup or `/run/user/<uid>/bus` default
path is used as a fallback.

---

## 16. Async reconcile loop

The controller implements the async interface from
`ADR-046-resource-reconciliation`:

```text
describe()    → ControllerDescriptor
validateSpec  → check bounded fields, closed categories per guestSources entry,
                guestRef resolution, displayWaylandRef resolution
plan          → compare desired vs observed guest-source Processes per guestSources entry;
                compare desired vs observed host-sink Process vs display-wayland readiness
reconcile     → create/update/delete guest-source Processes per guestSources entries;
                create host-sink Process when display-wayland Ready;
                stop/drain host-sink Process when display-wayland NotReady;
                delete guest-source Process when its guestRef Guest is deleted/Failed
observe       → periodic observe interval: 5m (detect stale Process states)
health        → check host-sink and guest-source process health
drain         → see §12.4
```

The controller's reconcile authority is over `Process` resources owned by
`Provider/notification-desktop`.  It does not own Volume resources, does not
add Volume to exported ResourceTypes, does not create or delete Volumes, and
does not hold notification delivery state.  Volume lifecycle is owned
exclusively by `Provider/volume-local` acting on the Volumes declared by
ProviderDeployment.

The reconcile loop is non-blocking.  No handler holds a blocking kernel,
systemd, or filesystem call across an `await`.  The D-Bus client in `host-sink`
uses an async D-Bus crate; the blocking `Notify` call is dispatched through an
explicit bounded adapter.

---

## 17. Current-code reuse and work items

### 17.1 Reuse from main

The following symbols from main commit `a1cc0b2da4a08ca3240a770a972fe4da6f912bef`
are candidates for copy/adapt:

| Main source | Reuse action | v3 destination |
| --- | --- | --- |
| `packages/d2b-notify/src/notifications.rs` — `sanitize()`, `Notification`, `Notifier`, `RecordingNotifier` | copy/adapt | `packages/d2b-provider-notification-desktop/src/redact.rs` |
| `packages/d2b-notify/src/nonce.rs` — `ActionNonce`, `ActionNonceStore`, `NONCE_BYTES`, `NONCE_TTL_SECS`, `MAX_STORE_SIZE`, `notification_action_key`, `parse_notification_action_key` | copy/adapt | `packages/d2b-provider-notification-desktop/src/action_nonce.rs` |
| `packages/d2b-notify/src/events.rs` — event enum, field bounds, `SecurityKeyEvent` | extract/adapt; generalize from security-key to generic category | `packages/d2b-provider-notification-desktop/src/types.rs` |
| `packages/d2b-notify/src/state.rs` — `CeremonySummary`, `SkNotifyState`, bound constants | adapt; generalize | `packages/d2b-provider-notification-desktop/src/types.rs` |
| `packages/d2b-notify/src/services/mod.rs` — `EstablishedDesktopSession`, `DesktopServices`, session evidence mapping, `DesktopStartupError` | copy/adapt | `packages/d2b-provider-notification-desktop/src/stream_admission.rs` |
| `packages/d2b-notify/src/services/actions.rs` — `ActionService`, `ActionSession`, `ActionOffer`, `InvokeActionRequest` | copy/adapt | `packages/d2b-provider-notification-desktop/src/action_nonce.rs` (client side) |
| `packages/d2b-notify/src/services/observer.rs` — `ObserverService`, `ObserverSession`, projection logic | adapt | `packages/d2b-provider-notification-desktop/src/host_sink.rs` (observer projection) |
| `packages/d2b-contracts/src/generated_v2_services/notify_ttrpc.rs` — `NotifyServiceClient`, `NotifyService` ttrpc shape | replace with v3 protobuf/ttrpc regenerated under `d2b.notification.v3` | `packages/d2b-provider-notification-desktop/src/` (generated) |
| v3 baseline `packages/d2b-clipd/src/notifications.rs` — `sanitize_notification_text` | absorb into `redact::sanitize()`; retire clipd copy | `packages/d2b-provider-notification-desktop/src/redact.rs` |
| v3 baseline `nixos-modules/notifications.nix` — `d2b.notifications.*` option set, `stateDir`, `statusHelper`, `securityKey` | superseded by v3 Zone resource authoring; Nix module retired after migration | see §17.3 |

The v3 baseline `d2b-notify` crate (`packages/d2b-notify/`) is the primary
existing source.  It is not yet wired to the ADR 0046 resource/session model.
The v2 `d2b.notify.v2.NotifyService` ttrpc contract is superseded by
`d2b.notification.v3`.

### 17.2 Work items

#### ADR046-notify-001

| Field | Value |
| --- | --- |
| Dependency/owner | W0 shared contract root; session/bus owner |
| Current source | `packages/d2b-notify/src/{events,state,notifications,nonce}.rs` |
| Reuse action | copy/adapt |
| Destination | `packages/d2b-provider-notification-desktop/src/{types,redact,action_nonce}.rs` |
| Detailed design | `NotificationRequest`/`NotificationResult` DTOs and stream record types; bounded fields; closed category set; icon catalog contract; `ActionNonce`/`ActionNonceStore` adapted from main; no ResourceType DTO |
| Integration | Zone bus service; host-sink stream consumer; guest-source stream producer |
| Data migration | No v2 compatibility; reset |
| Validation | `tests/stream_record.rs` — DTO schema vectors; `tests/action_nonce.rs` — single-use/TTL/capacity/replay |
| Removal proof | v2 `d2b.notify.v2` generated stubs removed after v3 service established |

#### ADR046-notify-002

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-session-001, ADR046-bus-001; session/bus wiring |
| Current source | `packages/d2b-notify/src/services/` |
| Reuse action | copy/adapt |
| Destination | `packages/d2b-provider-notification-desktop/src/stream_admission.rs` |
| Detailed design | Session admission checks, Noise profile enforcement, transport class validation |
| Integration | ComponentSession/d2b-bus |
| Data migration | None — full d2b 3.0 reset; no prior state to migrate |
| Validation | `tests/stream_admission.rs` — all rejection vectors |
| Removal proof | Old `DesktopServices` session admitted under v2 contract removed when v3 session established |

#### ADR046-notify-003

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-notify-001, ADR046-reconcile-001; controller owner |
| Current source | `packages/d2b-notify/src/services/observer.rs` |
| Reuse action | new |
| Destination | `packages/d2b-provider-notification-desktop/src/controller.rs` |
| Detailed design | Async Process placement controller; watches `guestSources` Guest refs; creates/drains/deletes guest-source Processes; creates/stops host-sink Process on display-wayland readiness change; declares no Provider state Volume and does not own/add/create/delete Volumes; bounded non-secret operational state lives in `status`/the core Operation ledger (D087); notification delivery state (in-memory projection, action nonce store) is host-sink process memory only; no ResourceType reconcile loop |
| Integration | Zone resource store (Process API); d2b-bus; display-wayland dependency watch |
| Data migration | None — full d2b 3.0 reset; no prior state to migrate |
| Validation | Unit tests for placement FSM in `tests/stream_record.rs`; Volume creation/deletion lifecycle in `tests/volume_lifecycle.rs`; see also `integration/cross_zone_source.rs` end-to-end |
| Removal proof | Not applicable (new controller) |

#### ADR046-notify-004

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-notify-002, ADR046-notify-003; host-sink owner |
| Current source | `packages/d2b-notify/src/services/actions.rs`, `packages/d2b-notify/src/bin/` |
| Reuse action | adapt |
| Destination | `packages/d2b-provider-notification-desktop/src/host_sink.rs` |
| Detailed design | D-Bus client; `DesktopNotificationSink` stream consumer; action nonce issuance; `DesktopNotificationObserver` projection (in-memory, not persisted); display-wayland ComponentSession bootstrap for pre-opened D-Bus FD |
| Integration | D-Bus session (pre-opened FD via ComponentSession bootstrap); ComponentSession named streams |
| Data migration | None — full d2b 3.0 reset; no prior state to migrate |
| Validation | `integration/dbus_sink.rs`, `integration/observer_client.rs`, `integration/action_invoke.rs` |
| Removal proof | `nixos-modules/notifications.nix` state-dir tmpfiles rule retired; all notification state is in-memory per-session with no Volume replacement |

#### ADR046-notify-005

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-notify-002; guest-source owner |
| Current source | v3 security-key guest vsock path (conceptual similarity only; not copied directly) |
| Reuse action | new |
| Destination | `packages/d2b-provider-notification-desktop/src/guest_source.rs` |
| Detailed design | Guest-side vsock ComponentSession; `NotificationRequest` record validation and field bounding; category filter; `DesktopNotificationSink` stream forwarding; `NotificationResult` handling; no host-side resource creation |
| Integration | Guest process vsock → host ComponentSession |
| Data migration | None — full d2b 3.0 reset; no prior state to migrate |
| Validation | `integration/cross_zone_source.rs` |
| Removal proof | v3 baseline security-key notification path in `d2b-notify` is superseded; clipd direct `notify_rust` call superseded |

#### ADR046-notify-006

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-notify-001; Nix/telemetry owner |
| Current source | `nixos-modules/notifications.nix`; `packages/d2bd/src/metrics.rs` |
| Reuse action | adapt |
| Destination | Nix: Zone resource authoring in `nixos-modules/`; metrics: `packages/d2b-provider-notification-desktop/src/` |
| Detailed design | Zone Provider resource and RoleBinding Nix compiler output; `spec.config.guestSources` authoring and eval-time assertions; OTEL metric emitters with closed labels; audit record emitters |
| Integration | Nix configuration compiler; OTEL emitter ring; authoritative audit |
| Data migration | `d2b.notifications.*` Nix options retired; `d2b.zones.<z>.resources.notification-desktop` with `spec.config.guestSources` replaces |
| Validation | Eval tests for category enforcement, displayWaylandRef assertion, guestRef resolution; `tests/stream_redaction.rs` for content-free telemetry |
| Removal proof | `nixos-modules/notifications.nix` removed after Zone resource equivalence confirmed by eval test |

### 17.3 Removal items

| Item | Removal condition |
| --- | --- |
| `nixos-modules/notifications.nix` and `d2b.notifications.*` option namespace | Zone resource authoring parity confirmed; eval assertion test added |
| `packages/d2b-notify/src/bin/waybar_helper.rs` and `d2b-sk-waybar-helper` binary | `display-wayland` Provider's Waybar/wlcontrol integration supersedes direct state-file polling |
| `packages/d2b-clipd/src/notifications.rs` direct `notify-rust` call | `notification-desktop` Provider guest-source on clipboard provider supersedes |
| v2 `d2b.notify.v2.NotifyService` generated stubs | v3 `d2b.notification.v3` service established and all callers migrated |
| State dir `/run/d2b/notify` tmpfiles rule | Retired; all notification state is in-memory per-session with no Volume replacement |

Per D094, each replaced current-code test is retired with an explicit
keep/adapt/move/delete disposition and a removal gate: the minimum reusable
semantic assertions migrate into this crate's hermetic `tests/`, and the old
duplicate tests, shell gates, fixtures, static artifacts, CI jobs, and manifest
entries are deleted once successor coverage and the removal proof pass —
updating `tests/layer1-jobs.json`, the closed gate manifests, the
flake/matrix/Nix-unit pins, the generated ledgers, and the CI workflow shards.
Old and new suites never run in parallel indefinitely.

---

## 18. Tests and integration requirements

Workspace policy requires every Provider crate to contain non-empty `tests/`
and `integration/` directories.

### Fast hermetic execution and test placement (D094)

Per D094 and `ADR-046-validation-and-delivery` §10.16, this Provider's `src/`
unit tests and `tests/*.rs` hermetic suite are fast, in-process, deterministic,
and parallel-safe: an individual normal test has p95 ≤50 ms with no wall-clock
sleep, and `cargo test -p d2b-provider-notification-desktop --lib --tests`
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

### 18.1 Required `tests/` coverage

| File | Coverage requirement |
| --- | --- |
| `tests/status_lifecycle.rs` | The Provider declares no state Volume; `ProviderStateSet(zone, "notification-desktop")` is empty; no component Process mounts a state Volume; bounded non-secret operational state (component readiness, reconcile stage, closed-enum error/health detail) is written to `status`/the core Operation ledger within the status bounds; no notification summary/body/action-label/icon/nonce byte is ever persisted; on restart the controller re-derives component readiness from live `status` observation and reverifies against running processes, treating status as observation, never authority |
| `tests/stream_record.rs` | `NotificationRequest`/`NotificationResult` DTO schema validation (all fields, closed category values, out-of-bound values, unknown fields rejected); category filter enforcement; `NotificationResult` `actionNonces` present only when actions declared; no content in error messages |
| `tests/stream_redaction.rs` | Inject requests with distinguishable content bytes; assert zero occurrences in collected tracing events, OTEL span attributes, metric label values, audit record fields, error messages, and Debug output |
| `tests/action_nonce.rs` | Single-use consumption; expired TTL consumed-and-cleared; capacity at `MAX_STORE_SIZE`; replay after consume; nonce opaque in Debug; notification action key round trip |
| `tests/stream_admission.rs` | All six rejection cases from §6.5; exact stable error codes |
| `tests/observer_projection.rs` | In-memory projection state machine: request admitted → delivered → evicted on close; TTL-based eviction; bounded projection size; no content in projection status codes |
| `tests/fault_injection.rs` | Disconnect mid-delivery; host-sink crash; drain under load; backpressure on observer; display-wayland unavailable |

### 18.2 Required `integration/` coverage

| File | Coverage requirement |
| --- | --- |
| `integration/cross_zone_source.rs` | Real KK ComponentSession from a fake guest process over vsock loopback; category filter enforcement; field bounds enforcement; sink stream delivery; `NotificationResult` returned |
| `integration/dbus_sink.rs` | host-sink process against a real D-Bus session (test session bus); `Notify` round trip; `ActionInvoked` signal routing; D-Bus FD obtained via mock ComponentSession to display-wayland |
| `integration/observer_client.rs` | Authenticated NN Unix observer session; projection updates on delivery; stream credit backpressure; session close drops projection |
| `integration/action_invoke.rs` | Full end-to-end: guest-source → host-sink → D-Bus `ActionInvoked` → observer `InvokeAction` → nonce consumed; replay rejected |

### 18.3 `README.md` requirement

`packages/d2b-provider-notification-desktop/README.md` must document:

- Provider identity (`Provider/notification-desktop`) and `providerRef`;
- `spec.config` schema fields, types, bounds, defaults; `guestSources` shape;
- named-stream DTOs: `NotificationRequest` and `NotificationResult` fields and
  bounds; no ResourceType exported;
- all component processes (controller, host-sink, guest-source) with domain and
  cardinality;
- placement (Host domain, user domain, Guest);
- dependencies (display-wayland required for dbusSinkEnabled; D-Bus FD via
  ComponentSession, not status field);
- RBAC: roles, subjects, stream verbs;
- security invariants: content privacy, D-Bus boundary, icon safety;
- state model: declares no Provider state Volume; `ProviderStateSet(zone,
  "notification-desktop")` is empty; bounded non-secret operational state lives
  in `status`/the core Operation ledger (D087); notification delivery state and
  action nonces are host-sink process memory only and are never persisted;
- build commands: `cargo build -p d2b-provider-notification-desktop`;
- test commands: `cargo test -p d2b-provider-notification-desktop`;
- integration commands: invoked by the repository test orchestration; see
  existing test infrastructure;
- standalone-repository consumption instructions.

---

## 19. Migration from pre-ADR 0046 (`d2b.notifications.*`)

| Old surface | v3 replacement | Migration action |
| --- | --- | --- |
| `d2b.notifications.enable` | `d2b.zones.<z>.resources.notification-desktop` Provider resource | Operator adds Provider resource |
| `d2b.notifications.securityKey.enable` | `spec.config.guestSources = [{ guestRef = "Guest/<name>"; categories = [...]; }]` in Provider resource | Operator adds guestSources entries (no change to Guest spec) |
| `d2b.notifications.securityKey.staleEntryTtlSecs` | `spec.config.acknowledgeTimeoutSecs` in Provider spec | Field rename with same semantics |
| `d2b.notifications.statusHelper.*` | display-wayland Provider's desktop status surface | statusHelper binary retired |
| `d2b.notifications.integrations.waybar.enable` | `display-wayland` Provider Waybar integration | Waybar integration moves to display-wayland |
| `d2b.notifications.runtime.stateDir` | no Volume replacement; all state is in-memory per-session | State dir tmpfiles rule retired |
| `notify_rust` direct call in `d2b-clipd` | `NotificationRequest` record on `DesktopNotificationSink` from clipboard guest-source | `d2b-clipd` sends v3 notification records |
| `d2b.notify.v2.NotifyService` ttrpc | `d2b.notification.v3` service | v2 service retired after migration |

There is no compatibility shim.  The migration requires a Zone configuration
update.  Consumers that do not set `d2b.zones.<z>.resources.notification-desktop`
receive no notification functionality.
