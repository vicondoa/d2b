# ADR 0046 Provider dossier: device-security-key

| Field | Value |
| --- | --- |
| Spec ID | `ADR-046-provider-device-security-key` |
| Parent | ADR 0046 |
| Status | Accepted |
| Version | 6 |
| Baseline | `b5ddbed67867d9244bf33390868101bd9b053e49` |
| Normative | Yes |
| Owners | `packages/d2b-provider-device-security-key/`, `SecurityKeyService`/`SecurityKeyBinding` controllers, physical Device integration, Nix resource emitters |
| Depends on | `ADR-046-resources-device`, `ADR-046-resource-object-model`, `ADR-046-primitive-resource-composition`, `ADR-046-resource-api-and-authorization`, `ADR-046-resource-reconciliation`, `ADR-046-provider-model-and-packaging`, `ADR-046-components-processes-and-sandbox`, `ADR-046-componentsession-and-bus`, `ADR-046-telemetry-audit-and-support`, `ADR-046-nix-configuration`, `ADR-046-provider-state` |
| Supersedes | `ProcessRole::SecurityKeyFrontend` daemon-internal accept loop (`packages/d2bd/src/security_key.rs`), `nixos-modules/components/security-key-guest.nix` untracked guest `d2b-sk-frontend.service` unit |

## Purpose

This dossier exhaustively specifies the `device-security-key` Provider in d2b 3.0.
It covers the local physical hidraw `Device`; the provider-neutral
`security-key.d2bus.org.SecurityKeyService` authority/projection resource;
the provider-neutral `security-key.d2bus.org.SecurityKeyBinding` consumer-intent
resource, initially implemented by `Provider/device-security-key`; the
one-ceremony-at-a-time fair lease model; the unprivileged Host relay
Process and Guest frontend Process; the broker-only fd handoff path;
ComponentSession-protected service and stream surfaces with descriptor validation;
lifecycle, disconnect, and cancel semantics; RBAC and security invariants; root
configuration, the standard Device schema, provider-neutral Service/Binding
schemas, and their signed Provider extensions; status, conditions, and phase
transitions; error classes; audit and OTEL placement; Nix declaration and
eval-time constraints; async reconcile loop triggers; all reuse work items mapping
baseline code to v3 destinations; required tests; and the current-code removal
sequence.

No raw key data, physical hidraw node path, sysfs bus path, vendor/product
string concatenation, or device serial number ever appears in any public or
broker-wire surface. All external claims use opaque stable labels or
session-scoped digests only.

## Identity

```text
Provider/device-security-key
```

Crate: `packages/d2b-provider-device-security-key/`

Provider implements:

- standard `Device` for the **real local physical hidraw backing only**
  (`deviceClass: physical`, `busClass: hidraw`);
- provider-neutral
  `security-key.d2bus.org.SecurityKeyService` for the owner-Zone authority and
  the core-owned local import projection; and
- provider-neutral
  `security-key.d2bus.org.SecurityKeyBinding` for one consuming
  Guest/user attachment and its policy.

`Device` is never an import projection and never represents the UHID frontend.
`ResourceExport` targets `SecurityKeyService`; `ResourceImport` creates one local
projection `SecurityKeyService`; ordinary consumer intent is always a
Nix/operator-authored `SecurityKeyBinding` that references a same-Zone Service.
The base Service and Binding schemas carry only semantic security-key authority,
target, attachment, and lifecycle fields. `spec.provider` and `status.provider`
select and describe the initial `Provider/device-security-key` implementation;
physical Device, hidraw, CTAPHID, relay, UHID, queue, and ceremony details do not
enter the provider-neutral base. No provider-named ResourceType alias is
registered or accepted. A future Provider implements these same exact semantic
types through a different `providerRef` and its own signed extension; it does not
mint another Service or Binding type name.

## Resource catalog and cardinality

| ResourceType | Author | Cardinality and role |
| --- | --- | --- |
| `Device` | Nix/operator | One real local physical hidraw backing per selector. It owns discovery/presence status only and is referenced by an owner-Zone authority Service. It is not exported and is not projected. |
| `security-key.d2bus.org.SecurityKeyService` | Nix/operator for `mode: authority`; Core projection factory for `mode: projection` | Exactly one authority Service per opaque security key across the Host authority scope. Its base carries the semantic D097 `AuthorityDescriptor`; the authored owner extension references the same-Zone physical `Device` and local relay `Endpoint`. A projection has `ownerRef: ResourceImport/<name>`, `providerRef`, semantic base/import fields, and no `spec.provider`, physical Device, or hidraw authority. |
| `security-key.d2bus.org.SecurityKeyBinding` | Nix/operator only | One per consuming Guest/user attachment/policy. Its base references one same-Zone `SecurityKeyService` plus the Guest/User target. The initial Provider extension realizes its UHID frontend `Process` and private frontend `Endpoint`. |
| `Process`, `Endpoint` | Controllers/adapters | Owned realization children. They are not substitute service/binding resources and are never authored as ceremony records. |

Individual CTAP ceremonies, `LeaseId` values, CIDs, queue entries, and cancellation
handles are bounded high-churn session records. They are **not Resource objects**.

## Crate layout

```text
packages/d2b-provider-device-security-key/
  src/
  tests/
  integration/
  README.md
```

Workspace policy rejects the crate if any of `src/`, `tests/`, `integration/`,
or `README.md` is absent. Detailed module and test file layout is normative in the
respective sections below; the directory names are the fixed root contract.

## Root config schema

Root configuration appears in `Provider/device-security-key` `spec.config`,
validated against the signed JSON Schema in the Provider package descriptor.

| Field | Type | Default | Bounds | Notes |
| --- | --- | --- | --- | --- |
| `devices` | list | `[]` | 0-16 entries | Per-selector entries; may be empty when Zone configures no security-key Devices |
| `devices[].label` | string | - | `^[a-z][a-z0-9-]{0,62}$` | Stable operator-defined selector label; unique within the config |
| `devices[].vendorId` | uint16 | - | 0x0001-0xFFFE | USB vendor ID |
| `devices[].productId` | uint16 | - | 0x0001-0xFFFE | USB product ID |
| `devices[].serial` | string \| null | `null` | ≤ 128 UTF-8 chars, no NUL | Optional serial filter; null matches any serial |
| `sessionRingSize` | uint | `32` | 8-256 | Maximum recent ceremony-session records per SecurityKeyBinding; oldest entry evicted when full; records are not Resources |
| `leaseTimeoutSecs` | uint | `120` | 30-3600 | Provider ceiling/default for per-Binding ceremony timeout; preserves baseline `CEREMONY_TIMEOUT` |
| `queueWaitTimeoutSecs` | uint | `15` | 5-120 | Maximum wait for a busy lease before the relay returns `ERR_CHANNEL_BUSY`; maps to `QUEUE_WAIT_TIMEOUT` |

**Prohibited fields:** `devices[].hidrawPath`, `devices[].sysfsPath`, any field
containing a raw filesystem path. The Provider derives the physical node from
the trusted bundle device table at runtime; no path is accepted in config.

**Duplicate labels** are rejected at Provider spec admission. Labels that do not
match `^[a-z][a-z0-9-]{0,62}$` are rejected. An empty `devices` list is valid;
the controller remains installed but creates no Device sub-resources.

## Physical Device spec contract

Normative D089 spec layering: Device base fields are ResourceType base
`spec.*` fields, including `spec.providerRef`, `deviceClass`,
`inventory.selector`, attachments, and arbitration. This Provider's
desired-only extension is the canonical `spec.provider = { schemaId:
"device-security-key.d2bus.org/Device/spec", schemaVersion, settings }`
envelope; it is manifest-registered/signed, strict deny-unknown, bounded, versioned
and digested, validated against `spec.providerRef` at Nix build and API
admission, implementation-only, and may not shadow base fields. Shared fields
are promoted to the Device base. The Provider implements the exact base Device
spec/status version/fingerprint, accepts the canonical minimal valid base Spec,
and rejects unsupported optional base capabilities only through its signed
capability matrix and provider-neutral `unsupported-capability`.
`spec.provider` aligns with `status.provider`; generic CLI/controllers operate on
the base spec and base status only. No secret bytes are allowed
in any spec layer, and no credential material is allowed in
`spec.provider.settings`.

```yaml
spec:
  providerRef: Provider/device-security-key
  deviceClass: physical
  arbitration: exclusive
  maxConcurrentClaims: 1
  inventory:
    selector:
      busClass: hidraw
      label: yubikey-primary     # must resolve to a label in Provider root config
      vendorId: "1050"            # 4 lower-cased hex digits
      productId: "0407"           # 4 lower-cased hex digits
      serial: null                # optional; null matches any serial
  provider:
    schemaId: device-security-key.d2bus.org/Device/spec
    schemaVersion: "1.0.0"
    settings: {}
```

The `inventory.selector.label` must match exactly one entry in `Provider/device-security-key`
`spec.config.devices[].label`. At admission, the controller verifies the label is
present in the Provider's installed config. An unresolvable label fails Device
spec admission with `device-not-found` condition.

`arbitration` and `maxConcurrentClaims` for this Provider are always `exclusive`
and `1`. Any other value is rejected at admission.

`busClass` must be `hidraw`. Any other `busClass` is rejected at admission.

The physical `Device` does not identify a consuming Guest, own a relay/frontend,
or carry cross-Zone import metadata. Its only consumers are same-Zone authority
`SecurityKeyService` resources. An imported security key therefore never creates a
`Device`, and no projection has a `deviceClass`, `inventory.selector`, or
DeviceGrant.

## `SecurityKeyService` spec and status contract

The provider-neutral ResourceType has a strict discriminated `spec.mode` union.
Its base declares only semantic security-key authority. The initial
`Provider/device-security-key` extension names the physical backing and relay:

```yaml
apiVersion: resources.d2bus.org/v3
type: security-key.d2bus.org.SecurityKeyService
metadata:
  name: yubikey-primary
  zone: devices
spec:
  providerRef: Provider/device-security-key
  mode: authority
  authority:
    authorityScope: physical-device
    authorityKeyClass: semantic-security-key
    cardinality: zero-or-one
    arbitration: exclusive
    authorityRef: security-key.d2bus.org.SecurityKeyService/yubikey-primary
    duplicateConflict: security-key-authority-conflict
    updateStrategy: drain-recycle
    exportability: explicit-export
  provider:
    schemaId: device-security-key.d2bus.org/SecurityKeyService/spec
    schemaVersion: "1.0.0"
    settings:
      deviceRef: Device/yubikey-primary
      relayEndpointRef: Endpoint/yubikey-primary-ctaphid-relay
      authorityDerivation: physical-fido-selector
      ownerProof: service-and-relay-process-identity
      fairnessPolicy: bounded-fifo
status:
  phase: Pending | Ready | Degraded | Failed | Unknown
  observedGeneration: 1
  conditions:
    - type: BackingAuthorityReady
      status: "True"
      reason: backing-authority-claimed
  resource:
    authority:
      available: true
      holderCount: 0
      arbitration: exclusive
      opaqueOwnerDigest: sha256:<redacted-owner-digest>
  provider:
    providerRef: Provider/device-security-key
    schemaId: device-security-key.d2bus.org/SecurityKeyService/status
    schemaVersion: "1.0.0"
    observedProviderGeneration: 1
    details:
      physicalBackingClaim: ready
      relayProcessRef: Process/device-<uid-short>-sk-relay
      relayEndpointRef: Endpoint/yubikey-primary-ctaphid-relay
      relayReady: true
      queueDepth: 0
      observedDeviceGeneration: 1
      observedEndpointGeneration: 1
      lastCeremonyOutcome: success | timeout | cancelled | busy | error | null
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

The base `authorityKeyClass: semantic-security-key` identifies the
service-specific security-key authority. The Provider extension's
`authorityDerivation: physical-fido-selector` selects its initial derivation,
but the spec never carries the trusted bundle `device_token`, a raw digest
chosen by the caller, a path, serial, address, or fd. In addition, after trusted
physical-USB identity resolution, Core mandatorily derives the
`physical-usb-backing/v1` digest and claims
`(Host, physical-usb-backing, opaqueKeyDigest)`. Every USB and security-key
Provider backed by the same token submits that identical tuple; neither this
service-specific authority nor a Provider-private class can replace it. Core
admits both required claims before it permits the relay DeviceGrant/open. The
Provider setting `relayEndpointRef` is a reserved desired-child ref: the Service
controller creates that same-name owned Endpoint and the Service remains
`Pending` until it is Ready.

The projection branch is Core-only:

```yaml
apiVersion: resources.d2bus.org/v3
type: security-key.d2bus.org.SecurityKeyService
metadata:
  name: yubikey-primary
  zone: work
  ownerRef: ResourceImport/yubikey-primary
spec:
  providerRef: Provider/device-security-key
  mode: projection
status:
  phase: Pending | Ready | Degraded | Failed | Unknown
  observedGeneration: 1
  resource:
    import:
      leaseState: pending | bound | degraded | revoked
      remoteGeneration: 7
      remoteFingerprint: sha256:<semantic-service-fingerprint>
  provider:
    providerRef: Provider/device-security-key
    schemaId: device-security-key.d2bus.org/SecurityKeyService/status
    schemaVersion: "1.0.0"
    observedProviderGeneration: 1
    details:
      relayEndpointRef: Endpoint/yubikey-primary-import-ctaphid
      observedEndpointGeneration: 3
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

A projection permits only `providerRef`, semantic base/import fields, and
ResourceImport ownership. It rejects `spec.provider`, `deviceRef`, `authority`,
physical selectors, and DeviceGrant fields. Core creates and owns it by
invoking the signed projection factory, always with `metadata.ownerRef:
ResourceImport/<name>`. Routing derives from that signed local Provider
descriptor, `providerRef`, and the ResourceImport record; the Provider may
report local import-route Endpoint/lease observations only in
`status.provider`. Nix and ordinary API callers cannot create
`mode: projection`.

The signed Provider descriptor's D096 factory publishes
`serviceType: security-key.d2bus.org.SecurityKeyService`, the SHA-256
`projectionSchemaFingerprint` of the canonical committed projection schema, and
a `factoryFingerprint` over the semantic factory fields plus projection-protocol
version. Provider and export/import adapter identity are authenticated
separately by the signed descriptor and never affect the semantic factory
fingerprint. The authority relay Endpoint and projection import-route Endpoint
remain Service-owned implementation children; neither is an Export or Import
field.

## `SecurityKeyBinding` spec and status contract

One Binding expresses one consuming Guest/user attachment and policy. It is authored
by Nix or an authorized operator after either an authority Service or projection
Service exists in the **same Zone**:

```yaml
apiVersion: resources.d2bus.org/v3
type: security-key.d2bus.org.SecurityKeyBinding
metadata:
  name: corp-vm-yubikey
  zone: work
  ownerRef: Guest/corp-vm
spec:
  providerRef: Provider/device-security-key
  serviceRef: security-key.d2bus.org.SecurityKeyService/yubikey-primary
  target:
    guestRef: Guest/corp-vm
    userRef: User/alice
  policy:
    enabled: true
  provider:
    schemaId: device-security-key.d2bus.org/SecurityKeyBinding/spec
    schemaVersion: "1.0.0"
    settings:
      ceremonyTimeoutSecs: 120
      queueWaitTimeoutSecs: 15
status:
  phase: Pending | Ready | Degraded | Failed | Unknown
  observedGeneration: 1
  resource:
    attachment: detached | connecting | ready | degraded
  provider:
    providerRef: Provider/device-security-key
    schemaId: device-security-key.d2bus.org/SecurityKeyBinding/status
    schemaVersion: "1.0.0"
    observedProviderGeneration: 1
    details:
      frontendProcessRef: Process/binding-<uid-short>-sk-frontend
      frontendEndpointRef: Endpoint/binding-<uid-short>-sk-frontend
      frontendReady: true
      observedServiceGeneration: 1
      activeSession: false
      queued: false
      lastCeremonyOutcome: success | timeout | cancelled | busy | error | null
      providerDiagnostic: null
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

`serviceRef`, `target.guestRef`, and `target.userRef` are local ResourceRefs. The
User must belong to the target Guest. Exactly one Binding may exist for the tuple
`(serviceRef, guestRef, userRef, attachment-policy-key)`; duplicate intent is
rejected deterministically. Base Binding status carries only semantic attachment
state. The bounded, non-secret implementation observations live in
`status.provider.details`; neither layer may carry CTAP bytes, credential IDs,
PINs, signatures, raw CIDs, a `LeaseId`, or an unbounded ceremony history.
Individual ceremony records remain in the bounded relay/controller session table
and Core Operation/audit surfaces; no `SecurityKeySession` ResourceType exists.
`SecurityKeyState` is neither a ResourceType nor a compatibility alias. Consumer
state exists only in `SecurityKeyBinding.status`.

## hidraw discovery and identity

### Discovery contract

Physical FIDO/hidraw device discovery is split between the Core/broker adapter
(which owns sysfs matching, device-token resolution, and any related audit) and the
Provider controller (which receives only opaque `InventoryObservation` results through
the injected `SecurityKeyEffectPort`). The Provider process never reads `/sys`,
opens hidraw device nodes, manages OFD lock paths, calls broker operations, or
receives raw device paths or UIDs.

### Bundle device table (trusted source - Core/adapter)

During NixOS activation, the Provider's activation component records
`(zone, device_label)` → `device_token` mappings in the private bundle device table.
The `device_token` is an opaque reference used exclusively by the Core/adapter; it
is never surfaced to the Provider crate or any public interface.

### Core LaunchTicket DeviceGrant (relay Process startup)

When Core launches the relay Process, it resolves the `deviceUsage` entry
`{deviceRef: Device/<device-name>, access: exclusive}` into a DeviceGrant. Core:

1. Looks up `device_token` from the bundle device table for the Zone + label.
2. Opens the referenced node with `O_RDWR | O_NONBLOCK | O_NOFOLLOW` (no iterative
   sysfs scan; targeted open using `device_token`).
3. Revalidates the opened fd: `fstat` → `S_IFCHR`; `ioctl(HIDIOCGRDESC)` → FIDO
   usage page `0xF1D0`; `ioctl(HIDIOCGRAWINFO)` → vendor/product match.
4. Passes the pre-opened fd to the relay as part of its LaunchTicket (inherited fd;
   no path in the relay's filesystem namespace).
5. Holds the corresponding OFD lease for the relay's process lifetime. Lease releases
   automatically when the relay exits for any reason.

No path or UID crosses any wire. No Provider code participates in steps 1-5.

### Identity and FIDO usage page

A hidraw node is FIDO-class if its report descriptor contains usage page `0xF1D0`
(`[0x06, 0xD0, 0xF1]` - usage-page item type 0x04, 2-byte little-endian `[0xD0,
0xF1]`). This check is performed by Core during revalidation. It is the sole
hardware-identity signal; Core does not verify attestation, AAGUID, or authenticator
model at open time.

### Probe semantics

The Provider controller schedules `scheduled-observe` for physical Devices at the configured interval
(default 30 s, max 60 s). On each trigger the controller calls:

```rust
effect_port.observe_inventory(&device_id, &policy_id).await
```

where `device_id` and `policy_id` are the opaque values injected by Core at
controller startup. The injected Core adapter performs any sysfs/udev observation
required; the controller receives only `InventoryObservation { present, fido_confirmed }`.
Neither `DeviceId` nor `ObservationPolicyId` is logged anywhere. No zone string,
label string, sysfs access, device open, or broker call occurs inside the
Provider process.

Probe outcome handling:

1. On success (`present=true, fido_confirmed=true`): reset consecutive-failure
   counter; `DevicePresent=True`; if phase was `Unknown`/`Degraded` due to probe
   failures, transition to `Ready` (if all other conditions clear).
2. On first failure: increment counter; transition to `Unknown` if currently `Ready`.
3. On second failure: remain `Unknown`.
4. On third consecutive failure: `DevicePresent=False`; transition to `Degraded`.
   Dependent Guests receive `dependency-changed` through the normal watch path.
5. When device returns: reset counter; `DevicePresent=True`; retransition to `Ready`.

## SecurityKeyEffectPort

`SecurityKeyEffectPort` is an injected async trait through which the Provider
controller performs all inventory observations that cross outside its own process
boundary. The trait and its associated opaque types live in a **neutral contract
crate** (`d2b-contracts`) so both the Provider crate and the Core adapter crate
depend on it without a circular edge. The Provider crate itself never reads sysfs,
opens device nodes, manages OFD lock paths, calls broker operations, or receives
raw device UIDs, paths, or node names.

```rust
// In d2b-contracts (neutral; no Provider-crate or Core-crate dependency)

use std::fmt;

/// Opaque stable device identity token issued by Core at controller startup.
/// Custom Debug impl redacts content so this type never appears in logs.
#[derive(Clone, PartialEq, Eq)]
pub struct DeviceId(/* opaque bytes; not a path, label, or UID string */);

impl fmt::Debug for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DeviceId([redacted])")
    }
}

/// Opaque observation policy identity issued by Core at controller startup.
/// Encodes zone, interval, and selector context - Core resolves these; the
/// Provider never sees zone or label strings. Custom Debug redacts content.
#[derive(Clone, PartialEq, Eq)]
pub struct ObservationPolicyId(/* opaque bytes */);

impl fmt::Debug for ObservationPolicyId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ObservationPolicyId([redacted])")
    }
}

/// Opaque result of a single inventory observation. No path, UID, or node name.
pub struct InventoryObservation {
    pub present:        bool,  // true iff a matching FIDO hidraw node exists
    pub fido_confirmed: bool,  // true iff FIDO usage page 0xF1D0 confirmed
}

#[async_trait]
pub trait SecurityKeyEffectPort: Send + Sync {
    /// Observe whether a FIDO hidraw node matching this device identity is
    /// currently present. Core resolves inventory; the Provider only sees the
    /// opaque InventoryObservation result. Neither DeviceId nor
    /// ObservationPolicyId is logged; both are redacted by their Debug impls.
    async fn observe_inventory(
        &self,
        device_id: &DeviceId,
        policy_id: &ObservationPolicyId,
    ) -> Result<InventoryObservation, EffectError>;
}
```

The Core/broker adapter is the sole concrete implementation. It is the only
location where sysfs matching, `device_id`-to-sysfs resolution, and any related
audit occur. The Provider controller is injected with a `Box<dyn SecurityKeyEffectPort>`
plus a `DeviceId` and `ObservationPolicyId` per Device at startup; Core resolves
the Zone + label context to these opaque values before injecting. The Provider
passes them back to the port without inspecting their contents.

The Provider crate's `src/effect_port.rs` re-exports `SecurityKeyEffectPort`,
`DeviceId`, `ObservationPolicyId`, and `InventoryObservation` from `d2b-contracts`.
No additional trait methods are defined in the re-export.

**Relay receives no port.** The relay receives the pre-opened hidraw fd from its
LaunchTicket DeviceGrant at process start and never calls any port or broker
method.

## Process model

The Provider owns one static controller, one Host relay Process per authority
`SecurityKeyService`, and one Guest frontend Process per `SecurityKeyBinding`.
Projection Services never own a hidraw relay. The controller and authority relay
are host-side system processes; each Binding-owned frontend is a user-domain process
inside its target Guest.

### Controller Process (created by Core ProviderDeployment)

```yaml
type: Process
metadata:
  name: device-security-key-controller
  zone: <zone>
  ownerRef: Provider/device-security-key
spec:
  providerRef: Provider/system-minijail
  executionRef: Host/host-system
  domain: system
  processClass: controller
  template: sk-controller
  sandbox:
    namespaceClasses: [mount, ipc, pid]
    capabilityClasses: []
    seccompClass: sk-controller
    environmentClass: provider-defined
    startRoot: false
    noNewPrivileges: true
    readOnlyRoot: true
  budget:
    pids:
      limit: 16
    fds:
      limit: 64
    memory:
      limit: "64Mi"
  restartPolicy:
    class: always
    backoffBase: "1s"
    backoffMax: "30s"
    backoffMultiplier: 2.0
  readiness:
    initialDelay: "0s"
    timeout: "10s"
    failureThreshold: 3
    successThreshold: 1
    class: provider-defined
```

The controller exists for the lifetime of the Provider installation.
Core creates it once via ProviderDeployment and restarts it on failure. It holds
the narrow Zone resource API authorities listed in §RBAC for physical `Device`,
`SecurityKeyService`, `SecurityKeyBinding`, and their owned `Process`/`Endpoint`
children. It never creates a physical Device or a projection Service. It realizes
an authority Service as one relay Process plus relay Endpoint and realizes a Binding
as one frontend Process plus private frontend Endpoint. It receives relay session
lifecycle events via `device-security-key.relay-ctrl.v1`.

### Host relay Process

```yaml
type: Process
metadata:
  name: device-<uid-short>-sk-relay
  zone: <zone>
  ownerRef: security-key.d2bus.org.SecurityKeyService/<service-name>
spec:
  providerRef: Provider/system-minijail
  executionRef: Host/host-system
  domain: system
  processClass: service
  template: sk-relay
  deviceUsage:
    - deviceRef: Device/<service.spec.provider.settings.deviceRef-name>
      access: exclusive
      purpose: hidraw-fido
  sandbox:
    namespaceClasses: [mount, ipc, pid]
    capabilityClasses: []
    seccompClass: sk-relay
    environmentClass: provider-defined
    startRoot: false
    noNewPrivileges: true
    readOnlyRoot: true
  budget:
    pids:
      limit: 32
    fds:
      limit: 64
    memory:
      limit: "32Mi"
  restartPolicy:
    class: on-failure
    backoffBase: "1s"
    backoffMax: "30s"
    backoffMultiplier: 2.0
    maxRestarts: 5
  readiness:
    initialDelay: "0s"
    timeout: "10s"
    failureThreshold: 3
    successThreshold: 1
    class: provider-defined
```

`uid-short` is the first 12 hex characters of the owner Service resource UID. The
Process name never contains a Guest or User name. The Service UID is stable across
daemon restarts.

The relay Process:

- Receives the hidraw fd from its LaunchTicket DeviceGrant at process start. Core
  opens, revalidates, and passes the pre-opened fd. The relay **never calls any
  broker operation** and never sees a device path or UID.
- The `deviceUsage: access: exclusive` DeviceGrant IS the exclusive lock. Core
  holds the corresponding OFD lease for the relay's process lifetime; it releases
  automatically when the relay exits for any reason (clean or crash).
- Serves the `d2b.security-key.v3` ComponentSession over the
  `Endpoint/<service-name>-ctaphid-relay` resource resolved through
  Provider/transport-vsock (see §ComponentSession).
- Accepts a bounded set of authenticated Binding frontend connections. Exactly one
  CTAP ceremony across all connected local/imported Bindings may hold the Service
  lease. Other ceremony requests enter a bounded FIFO fair queue for at most
  `Binding.spec.provider.settings.queueWaitTimeoutSecs`, then receive
  `ERR_CHANNEL_BUSY`.
- Proxies 64-byte CTAPHID reports bidirectionally between the Guest frontend
  (over the ComponentSession named CTAPHID stream) and the physical token (over
  the hidraw fd from the LaunchTicket).
- Translates CIDs: guest-provided channel IDs are replaced with host-assigned
  monotonically-incrementing CIDs before forwarding to the token; responses are
  translated back to the guest CID (see §CID translation). The relay never
  inspects raw vsock CIDs; transport-vsock/ComponentSession authenticates the
  expected Guest endpoint and passes only a canonical authenticated subject to the
  relay. The relay uses that canonical subject, not any raw transport address.
- Connects to the Provider controller's manifest-declared internal service
  `device-security-key.relay-ctrl.v1` using the bound internal channel FD from its
  LaunchTicket. Reports session lifecycle events and
  receives `CancelSession` signals.
- Has narrow ComponentSession service authority: responder of
  `d2b.security-key.v3` and client of `device-security-key.relay-ctrl.v1` only.
  No Zone resource API authority, no broker connection, no write access to Zone
  resource store.
- On crash, Core releases the DeviceGrant and OFD lease; the Provider controller
  observes `owned-resource-changed`, marks the owner Service unavailable, and
  degrades dependent Bindings. The relay is restarted after back-off.

The Host relay produces the stable CTAPHID relay service as an owned `Endpoint`
resource:

```yaml
apiVersion: resources.d2bus.org/v3
type: Endpoint
metadata:
  name: <service-name>-ctaphid-relay
  zone: <zone>
  ownerRef: security-key.d2bus.org.SecurityKeyService/<service-name>
spec:
  providerRef: Provider/device-security-key
  producerRef: Process/device-<uid-short>-sk-relay
  endpointClass: device
  transport: vsock
  purpose: device-security-key.d2bus.org/ctaphid-relay
  serviceFingerprint: device-security-key.d2bus.org/SecurityKeyCtapRelay.v3
  locality: cross-domain
  visibility: zone
  attachmentPolicy: component-session
  consumerPolicy:
    allowedProviderComponents: [device-security-key.d2bus.org/frontend]
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

## Endpoint resources (D092)

`Provider/device-security-key` conforms to the standard `Endpoint` base schema.
An authority Service owns its stable relay Endpoint. A projection Service owns a
local import Endpoint created through the signed import adapter. Each
`SecurityKeyBinding` owns a private frontend Endpoint. The Binding controller resolves
only the same-Zone Service Endpoint; cross-Zone routing is behind the projection
Endpoint and never appears as a remote Ref. Service Endpoints remain
implementation details and never appear in `ResourceExport` or `ResourceImport`.
Endpoint spec/status never carries raw vsock CIDs, ports, hidraw/UHID paths, fd
numbers, CTAP/session bytes, PINs, CBOR payloads, signatures, or credentials.
Resolution occurs only through an authorized EffectPort/LaunchTicket;
unauthorized resolution returns `endpoint-resolve-denied`. Producer restart
bumps `Endpoint.status.endpointGeneration`, causing frontend consumers to observe
`dependency-changed` and reconnect through a fresh authorized ticket.

## Retained opaque handles (D092 promotion test)

- pidfds for relay/frontend supervision stay process-local identity handles.
- LaunchTicket fd indexes for hidraw, UHID, and internal controller channels stay
  opaque per-launch attachment slots.
- CTAPHID CIDs, relay `sessionId` values, and per-connection ComponentSession
  handles are high-churn ceremony/session records and are never promoted to
  Resources.
- The manifest-declared relay-controller channel handle is controller-internal;
  it is not independently consumed outside this Provider.
- `OwnedTransport` and named CTAPHID streams are in-memory transport
  capabilities behind Endpoint resolution, not Endpoint identities.
- `operationId` values remain opaque audit/idempotency correlation handles.

**Current implementation note (implemented-and-reachable):** In the baseline the
relay is NOT a separate process - it is a daemon-internal loop in
`packages/d2bd/src/security_key.rs`. `ProcessRole::SecurityKeyFrontend` is a
readiness-only tracking node, not a spawned process. The v3 Provider extracts
this logic into `d2b-provider-device-security-key` as a proper unprivileged
`system`-domain Process receiving its hidraw fd from the LaunchTicket.

### Binding-owned frontend realization

There is no virtual or projected `Device` for UHID. The standard `Device` remains
the owner-Zone physical hidraw backing only. The frontend template requests the
Guest-scoped `uhid` authority subresource from `Provider/system-core`; Core
resolves that request as a LaunchTicket DeviceGrant, opens `/dev/uhid` inside the
target Guest execution context, revalidates it, and passes only the fd. This is
the same authority-subresource pattern used for shared kernel devices and does not
create a new ResourceType or a Device row.

The `SecurityKeyBinding` owns both frontend realization children. Its private
Endpoint is:

```yaml
apiVersion: resources.d2bus.org/v3
type: Endpoint
metadata:
  name: binding-<uid-short>-sk-frontend
  zone: <zone>
  ownerRef: security-key.d2bus.org.SecurityKeyBinding/<binding-name>
spec:
  providerRef: Provider/device-security-key
  producerRef: Process/binding-<uid-short>-sk-frontend
  endpointClass: device
  transport: component-session
  purpose: device-security-key.d2bus.org/ctaphid-frontend
  serviceFingerprint: device-security-key.d2bus.org/SecurityKeyCtapRelay.v3
  locality: zone-local
  visibility: provider
  attachmentPolicy: launch-ticket-only
  consumerPolicy:
    allowedProviderComponents: [device-security-key.d2bus.org/service]
    allowedOperations: [resolve]
  lifecyclePolicy: recycle-with-producer
```

### Guest frontend Process

```yaml
type: Process
metadata:
  name: binding-<uid-short>-sk-frontend
  zone: <zone>
  ownerRef: security-key.d2bus.org.SecurityKeyBinding/<binding-name>
spec:
  providerRef: Provider/system-systemd
  executionRef: Guest/<vm>
  domain: user
  processClass: service
  template: sk-frontend
  userRef: User/<workload-user>          # required; Guest executionRef must have defaultUserRef if absent
  sandbox:
    namespaceClasses: [mount, ipc, pid]
    capabilityClasses: []
    seccompClass: sk-frontend
    environmentClass: provider-defined
    startRoot: false
    noNewPrivileges: true
    readOnlyRoot: true
  budget:
    pids:
      limit: 16
    fds:
      limit: 32
    memory:
      limit: "16Mi"
  restartPolicy:
    class: on-failure
    backoffBase: "2s"
    backoffMax: "60s"
    backoffMultiplier: 2.0
    maxRestarts: 5
  readiness:
    initialDelay: "2s"
    timeout: "15s"
    failureThreshold: 3
    successThreshold: 1
    class: provider-defined
```

`executionRef` and `userRef` are copied from the Binding target. The Provider
controller creates this Process only after the same-Zone Service is Ready and the
Guest/User relationship is verified.

The frontend `userRef` is required. If absent, the `executionRef` Guest must
declare a `defaultUserRef`; the controller fails Process admission if neither is
present (user-domain Process requires a resolved user identity).

`providerRef: Provider/system-systemd` manages the frontend as a transient
user-scope systemd unit. The `sandbox` spec fields compile into systemd service
hardening directives (no minijail profile for the frontend).

Core resolves the template's Guest-scoped UHID authority-subresource request as a
LaunchTicket DeviceGrant. No standard Device resource is created. No udev rule,
wildcard device permission, or ambient `/dev` access is needed: the guest `/dev`
is masked in the frontend sandbox and only the pre-opened UHID fd is available.

The frontend Process:

- Runs as a user-domain process inside the Guest under `Provider/system-systemd`.
- Receives the pre-opened UHID fd from its `Provider/system-core` Guest-substrate
  LaunchTicket DeviceGrant.
- Creates a virtual FIDO2 CTAPHID HID device (UHID_CREATE event) on the received
  UHID fd with the FIDO usage descriptor. The virtual device is visible to libfido2,
  browsers, and `pamu2fcfg` inside the Guest.
- Resolves `Binding.spec.serviceRef` in its own Zone and connects as initiator of
  `d2b.security-key.v3` through that Service's local Endpoint. For a projection
  Service, the local Endpoint carries the bounded encrypted named stream to the
  owner authority. It reconnects on ComponentSession/import generation change.
- Reads UHID_OUTPUT events from the Guest kernel and sends them to the relay over
  the named CTAPHID stream. Reads relay responses and injects them via UHID_INPUT2.
- Has narrow ComponentSession client authority: initiator of `d2b.security-key.v3`
  only. No resource API authority, no Zone bus access.
- Binary: `packages/d2b-sk-frontend/` (static binary; implemented-and-reachable).

**Current implementation note:** Baseline `d2b-sk-frontend.service` is an
untracked Guest systemd unit. The v3 target removes that unit when the Process
resource is live (see ADR046-security-key-020).

## ComponentSession: relay server endpoint

The Host relay Process has two channel types: an external ComponentSession serving
authenticated Binding frontends, and a manifest-declared typed internal
ComponentSession service to the Provider controller.

### CTAPHID relay ComponentSession (relay ↔ Guest frontend)

The relay Process serves the `d2b.security-key.v3` ComponentSession over
`Provider/transport-vsock`. This is the sole CTAPHID transport in v3; no parallel
raw AF_VSOCK framing exists.

**Transport allocation:** the Binding resolves only its same-Zone
`SecurityKeyService` and the Service's local Endpoint. For an authority Service,
`Provider/transport-vsock` resolves that owned relay Endpoint into opaque
attachments for relay/frontend LaunchTickets. For a projection Service, the
import adapter resolves its local Endpoint into a per-import bounded encrypted
named stream over ZoneLink. The relay does not bind a raw vsock port. Handles are
opaque, never configurable as port numbers, and never appear in resource
spec/status. `vsockPort` does not exist in v3; no FD or raw transport locator
crosses a Zone.

**Noise profile:** enrolled KK (`Noise_KK_25519_ChaChaPoly_SHA256`). Relay and
Binding frontend static keys are enrolled at Process provisioning before the first
connection. The relay acts as responder; the frontend acts as initiator. The
cross-Zone named stream retains end-to-end record encryption so intermediate
controllers see ciphertext only.

**Named CTAPHID stream:** one bounded bidirectional named stream `ctaphid` within
the session:
- Maximum message size: 64 bytes (one CTAPHID report); any message exceeding 64
  bytes is a protocol error - both ends close the session on receipt.
- Relay → frontend: host-CID-translated CTAPHID reports from the physical token.
- Frontend → relay: guest CTAPHID reports (relay translates CID before forwarding;
  see §CID translation).
- The relay never logs raw packet bytes, CTAP payload content, PINs, CBOR
  payloads, assertion bytes, or signature material.

**Session authority:**
- Relay: responder of `d2b.security-key.v3` service only; no resource API
  authority; no Zone bus method access beyond this service.
- Frontend: initiator of `d2b.security-key.v3` service only; no resource API
  authority.

The baseline `vsock.sock_14320` (port 14320) and `packages/d2b-sk-frontend/src/framing.rs`
raw-frame protocol are obsolete under v3. The frontend
`packages/d2b-sk-frontend/src/vsock.rs` is replaced by the ComponentSession
vsock client from `d2b-session-unix/src/vsock.rs` (see ADR046-security-key-003).

### Manifest-declared relay ↔ controller service

The relay connects to the Provider controller's typed internal ComponentSession
service `device-security-key.relay-ctrl.v1`. This service is declared in the
Provider's package descriptor (manifest); there is no ambient filesystem socket
path. The bound internal channel FD is injected into the relay's LaunchTicket as a
controller-internal attachment - the relay never resolves a path to find the
controller.

- **Service name:** `device-security-key.relay-ctrl.v1` (internal; not
  addressable by external Providers or the CLI).
- **Role:** relay is client (initiator); controller is server (responder).
- **Noise profile:** NN (`Noise_NN_25519_ChaChaPoly_SHA256`); both ends verify
  the Provider descriptor digest before handshake.
- The relay sends bounded typed messages (`SessionStarted`, `SessionCompleted`,
  `SessionTimeout`) and receives `CancelSession { session_id }` from the
  controller.
- **Descriptor validation:** relay verifies the controller component's descriptor
  digest against the signed Provider package before the Noise handshake. Mismatch
  causes the relay to refuse the connection and enter `Degraded` state.
- **SO_PEERCRED check:** the relay verifies the connected peer's uid maps to
  the `d2b-provider-device-security-key` controller principal.
- **Record bounds:** messages ≤ 512 bytes. Oversized messages are discarded and
  the connection is closed.
- No CTAP session bytes, device paths, guest VM identity strings, or secret
  material appear in this channel.

## Lease, CID, and session lifecycle

### Session state machine

Each authority relay holds the hidraw fd from its LaunchTicket DeviceGrant for its
entire lifetime; the DeviceGrant is the physical-open lock. It may retain bounded
authenticated frontend connections, but its fair queue grants exactly one
ceremony lease at a time. The state machine is per ceremony:

```
Idle  (relay alive; hidraw fd held via DeviceGrant)
  │  (Binding frontend submits a ceremony request)
  ▼
Queued (holderRef: SecurityKeyBinding/<name>; bounded FIFO; deadline attached)
  │  (head of queue; relay allocates monotonic LeaseId)
  ▼
Active (holderRef: SecurityKeyBinding/<name>, opaque sessionId + LeaseId)
  │  (one CTAPHID ceremony may be in-flight; at most one)
  ├──────────────────────────────────────────────────────┐
  │  CTAPHID op complete or CTAPHID_CANCEL received      │ CEREMONY_TIMEOUT elapsed
  ▼                                                      ▼
Completed                                        TimedOut
  │                                                      │
  └──────────────────────▼──────────────────────────────┘
                       Idle
(DeviceGrant/OFD lease released only when relay process exits)
```

Transitions:

| Transition | Trigger | Effect |
| --- | --- | --- |
| `Idle → Queued` | Binding frontend submits a CTAP ceremony | Relay authenticates the Service/Binding pairing, allocates an opaque session ID, and enqueues by bounded FIFO order |
| `Queued → Active` | Request reaches queue head before `QUEUE_WAIT_TIMEOUT` | Relay allocates a monotonic `LeaseId`, reports `SessionStarted` with the Binding UID digest, and marks only bounded Binding/Service observations |
| `Queued → Completed` | Queue deadline expires | Relay returns `ERR_CHANNEL_BUSY`; records a bounded `busy` outcome; no physical CTAP bytes are written |
| `Active → Completed` | CTAP op complete or CTAPHID_CANCEL received | Relay reports `SessionCompleted`; updates status; ready for next connection |
| `Active → TimedOut` | `CEREMONY_TIMEOUT` (120 s) elapsed | Same as Completed; audit reason `session-timeout` |
| `Active → Completed` via controller | Controller `CancelSession { session_id, lease_id }` | Relay verifies the current `LeaseId`, cancels all active translated CIDs, transitions to Completed (reason=operator-cancel), and reports `SessionCompleted` |
| `Completed/TimedOut → Idle` | (internal transition) | Relay ready for next frontend connection |

**One active ceremony per Service:** while `Active`, requests from other local or
imported Bindings wait in bounded FIFO order for up to
`Binding.spec.provider.settings.queueWaitTimeoutSecs` (15 s)
and then receive `ERR_CHANNEL_BUSY`. A monotonic `LeaseId` prevents a stale
release/cancel from affecting a later ceremony. Exactly one relay authority per
physical Device is enforced by the Service `AuthorityDescriptor`, authority index,
and exclusive DeviceGrant before any open.

**Session ID format:** `sk-<uid-short>-<monotonic-counter>` where `uid-short` is
the first 12 hex chars of the authority Service UID and `counter` is a
per-relay-process monotonic u64. Session IDs and `LeaseId` values are opaque. Each
Binding has at most `sessionRingSize` bounded non-secret ceremony records for
session queries and cancellation bookkeeping. Those rows are high-churn session
records, not Resource objects and not child resources of the Binding.

### CID translation

The relay maintains a `CidTranslator` (v3 counterpart to the type of the same
name in `packages/d2bd/src/security_key.rs`). On each incoming guest CTAPHID
packet:

- Bytes 0-3 are the guest-assigned CID (u32, big-endian per CTAPHID spec).
- The relay looks up the guest CID in the translation table. If absent, it
  allocates a new host-assigned CID from a monotonic counter and records the
  mapping.
- The outgoing packet to the token uses the host-assigned CID.
- Incoming token responses use the host-assigned CID; the relay reverses the
  lookup and replaces with the guest CID before forwarding to the frontend.

CID translation is per active ceremony lease, not per Zone or long-lived
frontend connection. The table is discarded when the ceremony ends. Different
Bindings and Services never share a CID namespace. Disconnect/cancel invokes
`build_cancel_packet` for **all** active translated CIDs before dropping the map.

### Disconnect and cancel semantics

**Guest frontend disconnect (clean close):**

1. The relay detects graceful ComponentSession shutdown from the frontend.
2. If a ceremony is in-flight (`Active` state), the relay sends `CTAPHID_CANCEL`
   to the physical token (via the hidraw fd from its LaunchTicket).
3. The relay waits up to 500 ms for the token's cancel acknowledgment.
4. The relay transitions to `Completed` (reason: `client-disconnect`).
5. The relay reports `SessionCompleted` to the Provider controller with only the
   opaque Binding/session/lease correlation.
6. DeviceGrant/OFD lease remains held (relay is still running and Idle).

**Guest frontend disconnect (crash/abrupt):**

1. The relay detects broken ComponentSession (transport error or POLLHUP).
2. Same cancel-and-complete sequence as clean close.

**Operator cancel (Binding cancel action; the compatibility device-cancel command
resolves the active Binding):**

1. The CLI emits an admin-only cancel action for the active
   `SecurityKeyBinding`.
2. The Provider controller sends `CancelSession { session_id, lease_id }` to the relay via
   the manifest-declared internal service `device-security-key.relay-ctrl.v1`.
3. The relay sends `CTAPHID_CANCEL` to the token, transitions from `Active` to
   `Completed` (reason: `operator-cancel`), and reports `SessionCompleted`.
4. The controller updates bounded Service/Binding status and emits an audit record.
5. `CancelSession` is a protocol-level operation on the typed internal service;
   it never implies finalizer removal or DeviceGrant release (the grant persists
   while the relay process lives).

**Relay crash:**

1. Core detects relay process exit; DeviceGrant and OFD lease release automatically.
2. The Provider controller observes `owned-resource-changed` → relay Process `Failed`.
3. The controller marks the authority Service unavailable and dependent Bindings
   `Degraded`; the physical Device presence observation is unchanged.
4. Controller restarts relay Process after back-off (initial 1 s, ×2, max 30 s,
   max 5 attempts before `Failed`). New relay Process gets a new LaunchTicket with
   a fresh DeviceGrant.

## Security invariants

All invariants below are normative and must be preserved in every implementation
change. Violation of any invariant is a security defect regardless of test pass
status.

### I-1: Core is the only hidraw opener

Core opens the physical hidraw node as part of the relay Process LaunchTicket
DeviceGrant resolution. Neither the Provider controller nor the relay process
ever opens any `/dev/hidraw*` path, reads sysfs, manages OFD lock paths, or
receives raw device UIDs. The Provider controller calls
`SecurityKeyEffectPort::observe_inventory()` for probe observations; the injected
Core adapter is the only concrete site of sysfs access. The relay receives a
pre-opened hidraw fd from its LaunchTicket and has no path to any device node.

### I-2: No path on any public surface

No raw hidraw path, sysfs path, vendor/product string, or device serial number
appears in:
- Device, SecurityKeyService, SecurityKeyBinding, Endpoint, or Process spec/status.
- Audit records (path-free; device_label digest only).
- OTEL spans, metrics, or log lines.
- Public wire DTOs (`SecurityKeyStatusResponse`, `SecurityKeySessionsResponse`).
- Broker request or response body.
- Any ComponentSession record or error message.

Session IDs and authority/owner digests are opaque. They are never the hidraw
node name, sysfs path, serial, or a concatenation of vendor/product IDs.

### I-3: Exclusive DeviceGrant per relay Process

At most one authority Service relay can hold the exclusive DeviceGrant for a
given physical Device. Before Core opens hidraw, the Host-global D097 authority
index admits both the Service's semantic security-key authority and the shared
Core-derived `(Host, physical-usb-backing, opaqueKeyDigest)` claim, rejecting a
duplicate with no effect. Core then enforces that no two concurrent exclusive
grants are issued for the Device. The relay spec uses the initial Provider extension:
`deviceUsage: [{deviceRef: Service.spec.provider.settings.deviceRef, access:
exclusive}]`. On relay
exit, Core releases the DeviceGrant/OFD lease automatically; there is no
in-relay OFD lock management. Frontend UHID access is a separate Guest-substrate
authority-subresource DeviceGrant from `Provider/system-core`, not a physical,
virtual, or projected Device.

### I-4: Security-key proxy and USBIP are mutually exclusive

A Service under `Provider/device-security-key` and a Service under any USB
Provider that resolve to the same physical USB device cannot hold authority
anywhere on the same Host.

This is enforced:

**At Nix eval time:** `assertions.nix` emits a hard error if the compiled Host
bundle contains a security-key authority Service/device pair and a USB
Service/device pair whose trusted selector inputs are known to resolve to the
same physical USB device, even when authored in different Zones. This preflight
blocks NixOS activation but is not runtime authority.

**At authority admission:** after trusted USB identity resolution, Core derives
one `physical-usb-backing/v1` opaque digest. Security-key and every USB Provider
must claim the exact Host-global
`(Host, physical-usb-backing, opaqueKeyDigest)` tuple before effects. The second
Service transitions to `Failed` with `BackingAuthorityReady=False`,
`AuthorityConflict=True`, and `physical-usb-backing-conflict`. Provider-private
labels, key classes, or digests cannot bypass the collision. No hidraw open,
USBIP withhold/bind, module, relay, or attachment effect occurs.

### I-5: One ceremony lease, many explicit consumer Bindings

Multiple local or imported `SecurityKeyBinding` resources may reference one
Service. Each represents explicit Guest/User intent, but exactly one ceremony
holds the physical token lease at a time. Other requests wait in the bounded FIFO
fair queue and time out with `ERR_CHANNEL_BUSY`; they do not cause a second open
or terminate the active ceremony. Duplicate Binding intent for the same
Service/Guest/User/policy tuple is rejected before frontend creation.

### I-6: Peer authentication

The relay accepts ComponentSession connections only from a frontend static key
enrolled for a Ready Binding bound to the same Service. Peer authentication is
performed by the Noise KK handshake on the `d2b.security-key.v3` session;
the relay rejects any initiator whose static key is not enrolled for the
expected frontend Process. Transport-vsock/ComponentSession authenticates the
expected Binding/Guest endpoint and passes a canonical authenticated subject to the relay;
the relay uses only this canonical subject for identity decisions. The relay never
inspects or verifies raw vsock CIDs - those are a transport-internal detail owned
entirely by the transport layer. A connection from an unenrolled key is refused;
the relay does not accept in-band identity claims from the peer.

### I-7: No credential material in log/audit/OTEL

Raw CTAP payloads, PINs, CBOR assertions, credential IDs, WebAuthn responses,
and signature bytes are never logged, audited, or included in OTEL spans or
metrics. Authorized bounded audit records may carry the target,
Service/Binding, and session digests specified below. OTEL spans and metrics
carry only fixed semantic operation, phase, outcome, and error-class values;
they carry no target or resource identity, including opaque digests.

### I-8: Relay has narrow ComponentSession service authority

The relay Process holds narrow ComponentSession service authority: responder of
`d2b.security-key.v3` and client of the manifest-declared internal service
`device-security-key.relay-ctrl.v1` only. It does not hold a Zone resource API
client, a broker connection, or write access to the Zone resource store. The
internal service channel FD is injected via LaunchTicket; no ambient path is
used.

### I-9: No guest udev rules required

Core pre-opens `/dev/uhid` before the Binding-owned frontend starts, using the
`Provider/system-core` Guest-substrate authority and passing the fd via a
LaunchTicket DeviceGrant. No Device Resource is created for UHID. The frontend
sandbox masks `/dev`, so it accesses only the pre-opened fd. No udev rules,
`plugdev` membership, or wildcard device permissions are required. The
`SecurityKeyApplyUdevRules` operation is removed.

### I-10: ProviderStateSet is empty under status-first state

Per D087, `device-security-key` declares **no Provider state Volume** for the
controller, relay, or frontend components. `ProviderStateSet` is the optional
query-time grouping of declared Provider state Volumes; for
`Provider/device-security-key` the set is empty.

Bounded non-secret operational state belongs to the Resource that owns it:
physical presence in `Device.status`, authority/relay/queue aggregates in
`SecurityKeyService.status`, and consumer/frontend/attachment aggregates in
`SecurityKeyBinding.status`, plus the Core Operation ledger. Individual ceremony
rows, CTAP bytes, stream content, CIDs, `LeaseId` values, UHID/hidraw fds, session
keys, and cancellation handles are never Resource objects and must not be
persisted or exposed through status.

Storage-need test rationale: v1 security-key controller, relay, and frontend
state has no durable secret recovery payload, no large or binary file content, no
private data that belongs outside authorized status readers, and no
bounded-but-revision-unsuitable recovery payload. Transient CTAP and relay bytes
fail the persistence side of the storage-need test and remain in memory only.

## RBAC


Physical Device resources use the standard Device RBAC verbs from
`ADR-046-resources-device`. Qualified resource and child bindings are:

| Role | Verbs | Scope | Subjects |
| --- | --- | --- | --- |
| `security-key-device-observer` | `get`, `list`, `watch`, `update-status` | physical `Device` rows for this Provider | Provider controller only; no create/delete/finalizer authority |
| `security-key-service-controller` | `get`, `list`, `watch`, `create`, `update-spec`, `update-status`, `update-finalizers`, `delete` | authority `SecurityKeyService` with `providerRef=Provider/device-security-key` and its owned Process/Endpoint children | Provider controller only |
| `security-key-projection-controller` | `create`, `get`, `watch`, `update-spec`, `update-status`, `update-finalizers`, `delete` | projection `SecurityKeyService` with `providerRef=Provider/device-security-key`, `ownerRef: ResourceImport/<name>`, and its Endpoint | Core export/import controller + signed import adapter only |
| `security-key-binding-author` | `get`, `list`, `watch`, `create`, `update-spec`, `delete` | `SecurityKeyBinding` with an admitted providerRef | authorized Nix/operator subjects; never relay/frontend |
| `security-key-binding-controller` | `get`, `list`, `watch`, `update-status`, `update-finalizers`; child `create`, `update-spec`, `delete` | Binding with `providerRef=Provider/device-security-key` and its owned Process/Endpoint children | Provider controller only |
| `security-key-reader` | `get`, `list`, `watch` | Service/Binding in the caller's Zone | authorized CLI/runtime readers |
| `security-key-cancel` | action `cancel` | `SecurityKeyBinding/<name>` | Admin-only (`d2b` group + SO_PEERCRED admission) |

No Role grants wildcard `*`. Ordinary operators cannot create projection
Services, controllers cannot author consumer Binding intent, and import controllers
cannot create physical Devices or authority Services. A `ResourceExport` may
target an authority Service only.

The relay Process has no RoleBinding to the Zone bus. It cannot invoke any
resource API method. Its typed internal ComponentSession service connection to the
Provider controller is manifest-declared and socket-FD-injected via LaunchTicket;
no ambient path is used and the relay cannot open arbitrary Zone bus connections.

## Provider state

Per D087, `device-security-key` declares **no Provider state Volume**. The
controller, relay, and frontend component descriptors contain no state namespace,
no Provider state Volume template, and no `/state` mount. The ProviderStateSet is
empty because no declared Volume passes the storage-need test.

### Controller operational state

Device, SecurityKeyService, SecurityKeyBinding, their owned Process/Endpoint
children, and the Core Operation ledger are the authority for controller
decisions. The controller writes only each Resource's bounded observations. On
restart it re-lists those resources, adopts an exact Service authority by D097
owner proof without speculative reopen, revalidates external reality, and writes
material changes; it never recovers from a private controller state directory.

### Relay operational state

Each relay Process keeps CTAPHID session state, CID translation maps, cancellation
state, and hidraw fd ownership in process memory and inherited LaunchTicket
DeviceGrant fds. These values are transient and authority-conferring; they are not
Volume payloads. Bounded queue/holder counts and outcomes may appear in Service or
Binding status, while CTAP bytes, CIDs, LeaseIds, session keys, and raw identities
never do. Individual ceremonies remain bounded session records, not Resources.

### Frontend operational state

Each frontend Process keeps UHID/ComponentSession stream state in process memory
and inherited DeviceGrant fds. No frontend Provider state Volume or host-side
attachment Volume exists in v3. Bounded readiness and error summaries are written
to the owning Binding status and Operation ledger.

### Lifecycle

Core ProviderDeployment has no Provider state Volumes to create, mount, migrate,
or delete for `device-security-key`. Process lifecycle still uses LaunchTicket
DeviceGrants for the authority Service's physical Device and the Binding frontend's
Guest-substrate UHID authority subresource; neither is Provider state. Service
finalizers drain/cancel ceremonies, stop the relay, release the physical
DeviceGrant/OFD lease, revoke exports, and delete the relay Endpoint. Binding
finalizers cancel its ceremony, stop the frontend, release UHID, and delete its
private Endpoint. projection teardown is Core-owned and child-first. There is no frontend Device
or Volume cleanup step.

The storage-need test is not met by controller, relay, or frontend operational
state: durable status/operation records cover bounded non-secret observations,
while CTAP bytes, fd handles, session keys, and relay/frontend stream state are
transient data that must never be persisted.

The Provider controller consumes
 **no** broker operations directly. Core resolves
the relay Process's `deviceUsage: exclusive` DeviceGrant internally when launching
the relay, using the trusted bundle `device_token` to open the hidraw node. The
operations below are internal to Core's LaunchTicket machinery and are not
called by the Provider at runtime.

| Internal operation | Effect | Caller | Audit |
| --- | --- | --- | --- |
| `SecurityKeyOpenDevice` | Resolve FIDO hidraw node from trusted bundle `device_token`; open `O_RDWR\|O_NONBLOCK\|O_NOFOLLOW`; fstat/HIDIOCGRDESC/HIDIOCGRAWINFO revalidation; pass fd to relay LaunchTicket | Core LaunchTicket (DeviceGrant resolution) | Yes - path-free; device_label digest, zone, outcome |

The Provider controller never calls `SecurityKeyOpenDevice`. Core emits a
path-free `device-grant` audit record when the hidraw fd is opened; the Provider
controller does not emit this record.

`SecurityKeyApplyUdevRules` is removed from the architecture. Guest Nix loads the
`uhid` kernel module, but supplies no security-key udev rule; no runtime broker op
writes or applies udev rules.

**SecurityKeyOpenDevice internal request (from `packages/d2b-contracts/src/security_key.rs`):**

```rust
pub struct SecurityKeyOpenDeviceRequest {
    pub device_label: SecurityKeyDeviceLabel, // opaque stable label; no path
    pub session_id:   SecurityKeySessionId,   // for audit correlation; not a path
    pub zone:         String,                 // Zone name
}
```

No path, sysfs node, or device string is accepted. Unknown fields are rejected.
The broker ignores any other field.

## Resource status, ownership, and finalizers

Per D088, every Resource writes its common `status.resource` base and its own
strict signed provider extension atomically. The provider-neutral Service and
Binding base schemas are
`security-key.d2bus.org/{SecurityKeyService,SecurityKeyBinding}/status`. The
initial implementation extensions are
`device-security-key.d2bus.org/{Device,SecurityKeyService,SecurityKeyBinding}/status`,
bounded to 32 KiB, deny unknown fields, and never duplicate common or semantic
base fields.

### Physical Device status

The standard Device carries physical inventory only:

```yaml
status:
  phase: Ready | Pending | Degraded | Failed | Unknown
  conditions:
    - type: DevicePresent
      status: "True" | "False" | "Unknown"
      reason: device-probed-present | device-probe-failed
              | device-consecutive-probe-failures-exceeded
  resource:
    present: true | false | null
    health: healthy | degraded | failed | unknown
    holderRefs:
      - security-key.d2bus.org.SecurityKeyService/<authority>
    lastProbedAt: "2026-07-22T00:05:00.000Z"
  provider:
    providerRef: Provider/device-security-key
    schemaId: device-security-key.d2bus.org/Device/status
    schemaVersion: "1.0.0"
    observedProviderGeneration: 1
    details:
      fidoUsagePageConfirmed: true
```

It has no frontend, import, Guest/User, session, relay, or ceremony fields. The
holder is the local authority Service, never a remote consumer.

### Service status

Authority Service base status carries the D097 provider-neutral authority fields
(`available`, holder count, arbitration, opaque owner digest, update currency).
`status.provider.details` carries queue depth, relay Process/Endpoint refs,
observed Device/Endpoint generations, the initial implementation's shared
physical-backing claim state, relay readiness, and the last non-secret ceremony
outcome. All semantic authority fields are nested under `status.resource`.
Projection `status.resource` carries import lease state and remote
generation/fingerprint; its local import-route Endpoint/ref and readiness remain
in `status.provider.details`. It has no Device or DeviceGrant observation.
Binding attachment state is likewise nested under `status.resource`; no
semantic authority/import/attachment field appears directly under `status`.

Closed semantic Service conditions are `AuthorityReady`, `ImportBound`,
`AuthorityConflict`, `BackingAuthorityReady`, and `ConsumersDrained`.
Initial-Provider relay readiness and reasons remain in
`status.provider.details`. A duplicate security-key authority sets
`AuthorityConflict=True`; a shared physical USB collision additionally sets
`BackingAuthorityReady=False` with `physical-usb-backing-conflict` and `Failed`
before any effect. Link loss/export revocation sets a projection
`ImportBound=False` and `Degraded`/`Failed` according to disconnect policy;
dependent Bindings observe it.

An authority Service owns the relay Process and relay Endpoint and installs
`device-security-key.d2bus.org/authority-drained`. It clears that finalizer only
after exports stop admitting, queued/active ceremonies are cancelled or drained,
the relay child is deleted, and Core confirms DeviceGrant/OFD release. A
projection Service is owned by its `ResourceImport`; Core's import finalizer
stops dependent consumers, releases the remote lease, deletes the projection
Endpoint, then deletes the projection Service.

### Binding status

Binding `status.resource` carries only semantic attachment state; common
phase/update/conditions remain universal top-level fields. Closed semantic conditions are `ServiceReady`,
`TargetReady`, and `AttachmentReady`. `status.provider.details` carries Service
generation, frontend Process/Endpoint refs, frontend readiness, whether this
Binding is queued or active, and the last closed ceremony outcome. Neither layer
embeds CTAP data, CIDs, LeaseIds, credentials, paths, or an unbounded session
list.

A Binding owns exactly one frontend Process and private Endpoint and installs
`device-security-key.d2bus.org/frontend-released`. Deletion cancels only that
Binding's queued/active ceremony, waits up to five seconds for a lease-identity
matched completion, deletes the frontend Process and Endpoint, confirms the UHID
DeviceGrant closed, then clears the finalizer. Deleting a Binding never deletes its
Service, physical Device, ResourceImport, or ResourceExport.

### Currency and update (D091)

The controller implements `assess_update`, `plan_upgrade`, and `execute_upgrade`
for Device observation and Service/Binding realization and populates only universal
`status.update`. A disruptive authority Service update returns
`UpgradeRequired`, stops new queue admission, drains/cancels active consumers,
then recycles the relay while preserving the Service/Device identities. A
projection update revalidates import generation/fingerprint before rebinding.
A Binding update cancels only its ceremony and recycles only its frontend.
Currency propagates Device → authority Service → export/import → projection
Service → Binding → frontend. Non-disruptive policy/status changes reconcile in
place. No update field carries session or credential material.

## Async reconcile loop

The Provider controller implements the standard async reconcile interface for the
physical Device and both provider-neutral semantic ResourceTypes when
`spec.providerRef=Provider/device-security-key`. For `Create`, `UpdateSpec`, or
`Delete` with `waitForReconcile` (D090), it performs no external effect, finalizer,
or status mutation until Core supplies
`CommittedRevisionProof {resourceUid,generation,revision,operationId}`. `Abort`
means no effect. Idempotency keys derive from the proof in one bounded
per-resource single-flight lane.

### Watch plan and indexes

The controller uses bounded indexed watches, never an unfiltered cross-Zone list:

| Watched type | Filter/index | Reconcile target |
| --- | --- | --- |
| `Device` | same Zone + `providerRef=Provider/device-security-key` | that Device and authority Services whose `spec.provider.settings.deviceRef` matches |
| `SecurityKeyService` | same Zone + providerRef; authority/projection branch | that Service and Bindings indexed by `spec.serviceRef` |
| `SecurityKeyBinding` | same Zone + providerRef | that Binding |
| `Process`, `Endpoint` | `ownerRef` index for Service or Binding | owning Service/Binding |
| `Guest`, `User` | `dependencyRefIn` from Binding target | dependent Bindings only |
| `ResourceExport`, `ResourceImport`, `ZoneLink` | local Service/export/import-owner indexes | authority/projection Service and its Bindings |

Core performs projection creation and import lease writes under its own RBAC; the
Provider watch observes those committed revisions but cannot synthesize a
projection or mutate ResourceImport.

### `spec-generation-changed`

- **Physical Device:** validate the root-config label, `busClass=hidraw`, physical
  class, exclusive arbitration, and USBIP mutual exclusion; schedule observation.
  Do not create relay, frontend, Endpoint, Service, Binding, or import resources.
- **Authority Service:** require same-Zone physical
  `spec.provider.settings.deviceRef`, require its reserved same-Zone
  `spec.provider.settings.relayEndpointRef`, verify the semantic D097 base and
  signed initial-Provider extension, and admit the Host-scoped opaque authority
  before any effect. Install
  `authority-drained`; create/repair the owned relay Process with exclusive
  `deviceUsage` for that exact Device and the owned relay Endpoint. A duplicate
  authority or USBIP owner fails without opening hidraw.
- **Projection Service:** accept creation only from Core/import adapter with
  `ownerRef: ResourceImport/<name>` after the adapter matches
  `expectedServiceType`, `expectedProjectionSchemaFingerprint`, and
  `expectedFactoryFingerprint` to the signed factory; reject base or
  Provider-extension `deviceRef`, authority, or physical selectors;
  create/repair only the local projection-Service-owned Endpoint and import
  binding.
  The Provider controller never opens hidraw for this branch.
- **Binding:** require a same-Zone Ready Service and a valid Guest/User target,
  reject duplicate attachment intent, install `frontend-released`, and
  create/repair the Binding-owned frontend Process plus private Endpoint. Resolve
  UHID through the Guest-substrate DeviceGrant; never create a Device.

### `deletion-requested`

- **Binding:** cancel its exact `{session_id, lease_id}` if queued/active, delete the
  frontend Process then private Endpoint, confirm UHID grant closure, clear
  `frontend-released`, and let Core delete the Binding.
- **Authority Service:** stop export admission, mark dependent local Bindings
  draining, cancel/drain the fair queue, delete relay Process then relay Endpoint,
  confirm physical DeviceGrant/OFD release, clear `authority-drained`, and let
  Core delete the Service. The physical Device remains independently authored.
- **Projection Service:** Core's ResourceImport finalizer first stops dependent
  Bindings, releases the remote lease, deletes the local import-route Endpoint, then deletes
  the Service. Provider code cannot bypass or reorder this child-first sequence.
- **Physical Device:** deletion is blocked while an authority Service references
  it. Once unreferenced, no Provider-owned child remains to delete.

### `dependency-changed` / `execution-status-changed`

- Device absence/degradation makes the authority Service unavailable and
  propagates through export/import/projection to every dependent Binding.
- Export revocation, ZoneLink loss, or generation/fingerprint mismatch revokes the
  import lease, degrades the projection Service, cancels its Binding ceremonies,
  and requires revalidation before reconnect.
- Guest/User stop or policy denial cancels only the affected Binding, deletes its
  frontend children, and marks that Binding `Degraded`/`Unknown`; it does not stop
  the authority relay or other consumers.

### `scheduled-observe`

For a physical Device, call
`SecurityKeyEffectPort::observe_inventory(&device_id, &policy_id)` and record
`lastProbedAt`. Success resets failures and sets `DevicePresent=True`; one or two
failures produce `Unknown`; three produce `DevicePresent=False` and `Degraded`.
Recovery resets the counter. The resulting Device change propagates to its
authority Service and downstream Bindings. The Provider never reads sysfs.

### `owned-resource-changed`

A relay failure marks its owner authority Service unavailable, degrades all
dependent Bindings, and restarts the relay after bounded backoff. A frontend
Process/Endpoint failure marks only its owner Binding degraded and restarts that
frontend after bounded backoff. A local import-route Endpoint failure degrades the
projection Service and triggers import revalidation. Logs use opaque Resource
names/digests only.

## Errors

Stable error classes for this Provider (subset of Device common errors plus
security-key-specific additions):

| Error slug | Condition type | Meaning |
| --- | --- | --- |
| `device-not-found` | `DevicePresent=False` | Physical hidraw node absent or label not in bundle table |
| `security-key-authority-conflict` | `AuthorityConflict=True` | A second semantic security-key authority resolved to the same service-specific key; no open occurred |
| `physical-usb-backing-conflict` | `BackingAuthorityReady=False`, `AuthorityConflict=True` | Another USB or security-key Service owns the Core-derived Host-global physical USB tuple; no backing effect occurred |
| `security-key-binding-conflict` | `AttachmentReady=False` | Duplicate Binding intent exists for the same Service/Guest/User/policy tuple |
| `device-grant-denied` | `BackingReady=False` | Core physical DeviceGrant open denied (revalidation failed or device absent) |
| `device-session-timeout` | - | CTAP ceremony exceeded `leaseTimeoutSecs` |
| `device-session-cancelled` | - | Binding ceremony was cancelled with a matching LeaseId |
| `device-session-busy` | `status.provider.details.queued=false` | Fair-queue wait exceeded the Provider extension's `queueWaitTimeoutSecs`; `ERR_CHANNEL_BUSY` returned |
| `device-worker-failed` | Provider detail `relayReady=false` or `frontendReady=false` | Owned relay/frontend Process failed after retry exhaustion |
| `device-cid-collision` | - | Internal CID allocation overflow (monotonic counter wraparound; effectively unreachable at u64) |
| `device-selector-label-unresolvable` | `DevicePresent=False` | `inventory.selector.label` does not match any entry in Provider root config |
| `security-key-import-invalid` | `ImportBound=False` | Projection owner, generation, fingerprint, or capability ceiling failed validation |
| `security-key-cross-zone-ref` | - | A Service/Binding/export/import attempted to carry a remote ResourceRef |

All error messages are bounded (≤ 256 UTF-8 chars), must not contain device
paths, hidraw node names, sysfs bus IDs, raw CTAP bytes, vendor/product strings,
or any credential material.

## Audit and OTEL

### Audit records

Audit records from this Provider and the Core device-grant operations it triggers:

**Core `device-grant` audit** (emitted by Core's LaunchTicket machinery, not by
the Provider controller; path-free):

```json
{
  "kind": "device-grant",
  "op": "SecurityKeyOpenDevice",
  "zone": "<zone>",
  "resource_type": "Device",
  "resource_name_digest": "sha256:<hex of device label; not label text>",
  "subject_digest": "sha256:<hex of relay Process principal identity>",
  "session_id_digest": "sha256:<hex of session_id string; not session_id text>",
  "outcome": "success | failure | denied",
  "error_class": "<closed-set slug or null>",
  "correlation_id": "<opaque trace/operation id>",
  "timestamp": "<RFC 3339 UTC>"
}
```

`resource_name_digest` is permitted only in this bounded Core audit record,
after the caller has already been authorized for the DeviceGrant operation. It
is not a telemetry field and must never be copied into a metric label, span
attribute, OTEL log, collector diagnostic, or support summary.

Excluded: hidraw node path, sysfs bus ID, vendor/product string, serial, device
file descriptor number, CTAP payload, guest VM name. The Provider controller does
not emit this record; Core emits it at DeviceGrant resolution time.

**Service/Binding ceremony-lifecycle controller audit** (emitted by the Provider
controller, not Core; uses the Zone runtime audit stream):

```json
{
  "kind": "device-lease",
  "event": "acquired | released | timeout | cancelled | conflict",
  "zone": "<zone>",
  "resource_type": "security-key.d2bus.org.SecurityKeyBinding",
  "service_digest": "sha256:<hex of Service UID; not name text>",
  "binding_digest": "sha256:<hex of Binding UID; not name text>",
  "holder_digest": "sha256:<hex of Guest/User target; not name text>",
  "session_id_digest": "sha256:<hex of session_id>",
  "correlation_id": "<opaque id>",
  "timestamp": "<RFC 3339 UTC>"
}
```

### OTEL telemetry

OTEL span attributes and metric labels follow `ADR-046-telemetry-audit-and-support`.
Constraints specific to this Provider:

- OTEL resource attributes include `d2b.provider="device-security-key"` and
  `d2b.zone=<Zone name>`; neither is copied into metric labels.
- **`phase`** metric label: `"Ready"` | `"Pending"` | `"Degraded"` | `"Failed"` | `"Unknown"`.
- Metric `d2b_device_sk_session_total{outcome}`: counter; `outcome` ∈ `{success, timeout, cancelled, busy, conflict, error}`.
- Metric `d2b_device_sk_ceremony_duration_seconds`: histogram; bucketed 0-120 s.
- Metric `d2b_device_sk_relay_restarts_total`: counter.
- No metric label or span attribute carries a device/resource/Service/Binding
  name, UID, ref, digest (including `resource_name_digest`), session ID, guest
  name, hidraw path, serial, or derived identity token. Spans use only fixed
  semantic operation, phase, outcome, and closed error-class attributes.
- `resource_name_digest` remains audit-only under the authorization rule above.
- OTEL emitter: lightweight bounded ring (no OTEL SDK in the Provider process;
  tracing crate only). The `observability-otel` Provider drains and forwards.

## Security-key authority and cross-Zone sharing (D096/D097)

**One authority Service (D097).** The owner-Zone
`security-key.d2bus.org.SecurityKeyService` in `mode: authority` is the
stable semantic authority Resource. Its provider-neutral D097
`AuthorityDescriptor` uses `authorityScope: physical-device`, opaque
`authorityKeyClass: semantic-security-key`, `cardinality: zero-or-one`,
`arbitration: exclusive`, `authorityRef` to itself, and `exportability:
explicit-export`. The initial `Provider/device-security-key` extension references
one same-Zone physical `Device` and its owned local relay `Endpoint`; that relay
is the sole holder of the physical hidraw FD. Its
`authorityDerivation: physical-fido-selector`, `ownerProof:
service-and-relay-process-identity`, and `fairnessPolicy: bounded-fifo` remain
inside `spec.provider.settings`. Core derives the non-authorizing key digest from
the trusted bundle `device_token`/FIDO selector. Separately and mandatorily, Core
resolves trusted physical USB identity and derives the
`physical-usb-backing/v1` digest used in the exact Host-global tuple
`(Host, physical-usb-backing, opaqueKeyDigest)`. Every physical-USB-backed
security-key and USB Provider claims that same tuple before effects; a
service-specific authority remains additive and cannot replace it. Restart
adopts the exact Service + relay Process DeviceGrant and shared backing claim by
the Provider owner proof; ambiguity quarantines. The physical Device is backing
inventory, not the service authority.

**Preserved reusable semantics** (grounded in
`packages/d2bd/src/security_key.rs` and
`packages/d2b-priv-broker/src/ops/security_key.rs`): sole hidraw opener (Core via
`SecurityKeyOpenDevice`/`live_open_hidraw_security_key`); post-open double
`fstat` + FIDO usage-page (0xF1D0) + HID raw-info revalidation on the pre-opened
`O_RDWR|O_NONBLOCK|O_NOFOLLOW` fd; async cancellation-safe fd relay I/O;
per-session `CidTranslator` (`alloc_host_cid`/`guest_to_host`/`host_to_guest`/
`release_guest_cid`); `LeaseId` stale-release guard; cancel of **all** active
CIDs (`build_cancel_packet`) on disconnect; exactly one CTAP ceremony at a time
(`CEREMONY_TIMEOUT` 120 s); a bounded fair wait for a busy lease
(`QUEUE_WAIT_TIMEOUT` 15 s, then `ERR_CHANNEL_BUSY`); and the guest-side UHID
frontend (`packages/d2b-sk-frontend/`: `/dev/uhid` virtual FIDO2 CTAPHID device,
64-byte report relay).

**Service export/import (D096).** The owner Zone declares a `ResourceExport`
whose local `resourceRef` is the authority `SecurityKeyService`, whose
`serviceType` is `security-key.d2bus.org.SecurityKeyService`, and whose
`projectionSchemaFingerprint` and `factoryFingerprint` match the signed
semantic projection factory. The Service's relay Endpoint remains its local
implementation child and is not copied into the Export. A physical `Device` is
never an export target. A consumer Zone declares a `ResourceImport` naming only
its local ZoneLink plus export key and supplies the corresponding
`expectedServiceType`, `expectedProjectionSchemaFingerprint`, and
`expectedFactoryFingerprint`. Core and the signed import adapter create exactly
one local projection
`SecurityKeyService` with `ownerRef: ResourceImport/<name>`. The projection has
no physical Device ref, hidraw FD, DeviceGrant, selector, or open path.

Nix/an authorized operator then creates one or more local `SecurityKeyBinding`
resources. Each Binding references the projection Service and its own Guest/User
target and owns the UHID frontend Process/private Endpoint. `SecurityKeyBinding` is
consumer intent and is never auto-created by ResourceImport. Ordinary consumers
never use ResourceImport or a projected Device as the typed security-key handle.

CTAPHID reports flow over a per-import bounded **encrypted named stream** with
credit backpressure, per-import session generation, deadline, idempotency, and
cancel; only the trusted authority relay and exact Binding frontend see plaintext.
Intermediate controllers see ciphertext. The authority Service serializes all
local and cross-Zone ceremonies through the same `LeaseId`-guarded fair queue,
per-session `CidTranslator`, timeout, and cancel-all-CIDs semantics. No FD, USBIP,
raw hidraw access, remote Ref, or token path crosses a Zone.

Core owns export/import routing, base lifecycle, and projection ownership; the
Provider adapters own semantic admission and observation. Export removal or
ZoneLink loss revokes the import lease, degrades the projection Service, and
cancels/degrades its Bindings child-first. Reconnect revalidates generation and
fingerprint before frontend replay. D091 drains all Bindings before recycling the
single authority Service.

**No legacy shortcuts; legacy removed after successor.** The relay adds **no**
new `ProcessRole`, no direct broker path, and no per-VM state file: the hidraw fd
arrives via the D077 EffectPort/LaunchTicket DeviceGrant, observed state is D087
status-first, the relay endpoint is a D092 `Endpoint`, and cross-Zone sharing is
D096. The legacy daemon-internal accept loop (`ProcessRole::SecurityKeyFrontend`
in `packages/d2bd/src/security_key.rs`), the raw CTAPHID framing over a fixed
vsock port (`SK_VSOCK_PORT`), and the broker sysfs `/sys/class/hidraw/` scan
fallback are removed only after the successor relay/`Endpoint`/named-stream path
and the `device_token`-only broker open are green (see the work items and removal
schedule).

## Nix configuration

### Nix authoring shape

Security-key resources are declared under `d2b.zones.<zone>.resources`. The
Provider must be installed in every Zone that owns an authority Service or a
consumer Binding. The physical Device and authority Service are authored only in
the owner Zone. A consuming Zone authors ResourceImport plus Binding; Core creates
the projection Service.

```nix
# Install the Provider
d2b.zones.devices.resources."sk-provider" = {
  type = "Provider";
  spec = {
    artifactId = "device-security-key";     # selects the signed package
    config = {
      devices = [
        {
          label     = "yubikey-primary";
          vendorId  = 4176;                 # 0x1050 - Yubico
          productId = 1031;                 # 0x0407 - YubiKey 5
          serial    = null;
        }
      ];
      sessionRingSize  = 32;
      leaseTimeoutSecs = 120;
      queueWaitTimeoutSecs = 15;
    };
  };
};

d2b.zones.work.resources."sk-provider" = {
  type = "Provider";
  spec = {
    artifactId = "device-security-key";
    config = {
      devices = [ ];
      sessionRingSize = 32;
      leaseTimeoutSecs = 120;
      queueWaitTimeoutSecs = 15;
    };
  };
};

# Owner Zone: real physical backing only.
d2b.zones.devices.resources."yubikey-primary-device" = {
  type = "Device";
  spec = {
    providerRef = "Provider/device-security-key";
    deviceClass = "physical";
    arbitration = "exclusive";
    inventory.selector = {
      busClass  = "hidraw";
      label     = "yubikey-primary";
      vendorId  = "1050";                   # 4 lower-cased hex digits
      productId = "0407";
      serial    = null;
    };
    provider = {
      schemaId = "device-security-key.d2bus.org/Device/spec";
      schemaVersion = "1.0.0";
      settings = { };
    };
  };
};

# Owner Zone: exactly one D097 authority Service for that physical key.
d2b.zones.devices.resources."yubikey-primary" = {
  type = "security-key.d2bus.org.SecurityKeyService";
  spec = {
    providerRef = "Provider/device-security-key";
    mode = "authority";
    authority = {
      authorityScope = "physical-device";
      authorityKeyClass = "semantic-security-key";
      cardinality = "zero-or-one";
      arbitration = "exclusive";
      authorityRef =
        "security-key.d2bus.org.SecurityKeyService/yubikey-primary";
      duplicateConflict = "security-key-authority-conflict";
      updateStrategy = "drain-recycle";
      exportability = "explicit-export";
    };
    provider = {
      schemaId = "device-security-key.d2bus.org/SecurityKeyService/spec";
      schemaVersion = "1.0.0";
      settings = {
        deviceRef = "Device/yubikey-primary-device";
        relayEndpointRef = "Endpoint/yubikey-primary-ctaphid-relay";
        authorityDerivation = "physical-fido-selector";
        ownerProof = "service-and-relay-process-identity";
        fairnessPolicy = "bounded-fifo";
      };
    };
  };
};

d2b.zones.devices.resources."yubikey-primary-export" = {
  type = "ResourceExport";
  spec = {
    providerRef = "Provider/device-security-key";
    resourceRef =
      "security-key.d2bus.org.SecurityKeyService/yubikey-primary";
    serviceType = "security-key.d2bus.org.SecurityKeyService";
    projectionSchemaFingerprint =
      "sha256:<security-key-service-projection-schema>";
    factoryFingerprint = "sha256:<security-key-projection-factory>";
    arbitration = "exclusive";
    operations = [ "security-key-ceremony" ];
    consumerZonePolicy = {
      zones = [ "Zone/work" ];
      capabilityCeiling = [ "security-key-ceremony" ];
    };
    visibility = "named-zones";
  };
};

# Consumer Zone: import creates the projection Service; do not author a Device
# or an Endpoint.
d2b.zones.work.resources."yubikey-primary-import" = {
  type = "ResourceImport";
  spec = {
    providerRef = "Provider/device-security-key";
    zoneLinkRef = "ZoneLink/work-uplink";
    exportKey = "devices/yubikey-primary-export";
    expectedServiceType = "security-key.d2bus.org.SecurityKeyService";
    expectedProjectionSchemaFingerprint =
      "sha256:<security-key-service-projection-schema>";
    expectedFactoryFingerprint = "sha256:<security-key-projection-factory>";
    projectionName = "yubikey-primary";
    requestedCapabilities = [ "security-key-ceremony" ];
    disconnectPolicy = { mode = "degrade"; };
  };
};

# Consumer intent: one Binding per Guest/User attachment and policy.
d2b.zones.work.resources."corp-vm-yubikey" = {
  type = "security-key.d2bus.org.SecurityKeyBinding";
  metadata.ownerRef = "Guest/corp-vm";
  spec = {
    providerRef = "Provider/device-security-key";
    serviceRef =
      "security-key.d2bus.org.SecurityKeyService/yubikey-primary";
    target = {
      guestRef = "Guest/corp-vm";
      userRef = "User/alice";
    };
    policy = {
      enabled = true;
    };
    provider = {
      schemaId = "device-security-key.d2bus.org/SecurityKeyBinding/spec";
      schemaVersion = "1.0.0";
      settings = {
        ceremonyTimeoutSecs = 120;
        queueWaitTimeoutSecs = 15;
      };
    };
  };
};
```

**Eval-time invariants:**

1. A physical Device label resolves exactly once in the same owner Zone's
   Provider config; `deviceClass=physical`, `busClass=hidraw`, and exclusive
   arbitration are mandatory.
2. An authority Service's semantic D097 descriptor is complete and its
   `authorityRef` names that Service. The initial Provider extension references
   one same-Zone physical Device and one same-Zone reserved relay Endpoint.
3. Two Services/USBIP resources that resolve to the same Host-scoped opaque
   physical key fail activation, including conflicts authored in different Zones.
4. A `ResourceExport` names only an authority `SecurityKeyService` through
   `resourceRef`, `serviceType`, `projectionSchemaFingerprint`, and
   `factoryFingerprint`. Its Service-owned Endpoint is not an Export field.
   Exporting a Device or Binding is rejected, as are the obsolete Export
   `endpointRef`, `exportedType`, `baseSchemaFingerprint`, and `exportKey`
   fields.
5. A ResourceImport's `expectedServiceType`,
   `expectedProjectionSchemaFingerprint`, and `expectedFactoryFingerprint`
   match the Export and signed local factory. The obsolete `expectedType`,
   `expectedBaseSchemaFingerprint`, and `projectionType` fields are rejected.
   Nix cannot author `mode=projection`; only Core creates it with `ownerRef:
   ResourceImport/<name>`, and it has no Device or hidraw fields.
6. Every Binding base references a same-Zone Service and same-Guest User target;
   duplicate `(serviceRef, guestRef, userRef, policy-key)` intent is rejected.
7. No cross-Zone ResourceRef, `hidrawPath`, sysfs path, serial-derived authority
   key, fd, USBIP field, or unknown field may appear in any
   Service/Binding/export/import base or Provider extension. The initial Provider
   extension admits only its strict same-Zone Device/Endpoint refs and bounded
   implementation settings.

**Eval-time derivations:**

| Field | Derived from |
| --- | --- |
| `metadata.name` | Resource attribute key |
| `metadata.zone` | Zone attribute key |
| `apiVersion` | Constant `"resources.d2bus.org/v3"` |
| `metadata.uid`, `generation`, `revision` | Core-assigned on creation |
| `metadata.finalizers` | Written by Provider controller |
| `status` | Entirely read-only |

### Guest Nix module (preserved wiring)

The guest NixOS module `nixos-modules/components/security-key-guest.nix`
continues to wire the following under v3. These module declarations remain in
the Guest Provider's `runtime-cloud-hypervisor` Nix module and are not replaced
by the Device resource:

- `boot.kernelModules = ["uhid"]` - required for the UHID kernel interface to
  exist at all; must be loaded before Core opens `/dev/uhid` for the frontend's
  LaunchTicket DeviceGrant.
- The static `d2b-sk-frontend` binary in the Guest store closure.

**Removed from v3:** `services.udev.extraRules` and
`users.users.<workload-user>.extraGroups = ["plugdev"]` are no longer needed.
Core pre-opens `/dev/uhid` with masked `/dev` and passes only the fd; the
frontend has no ambient device path access, so no udev rule or group is required.

**v3 change:** The `d2b-sk-frontend.service` systemd unit declared in
`security-key-guest.nix` is removed when the Binding-owned Process
`binding-<uid-short>-sk-frontend` is live. The Process controller manages the
frontend lifecycle. The Nix module removes the unit declaration behind a
`d2b.securityKey._legacySystemdUnit = false` option gate, defaulting to false
once the Provider is installed.

## Work items

All items are New (not yet implemented) unless marked with the baseline evidence
class.

### Reuse from baseline

### ADR046-security-key-001

| Field | Value |
| --- | --- |
| Dependency/owner | ADR-046 provider-device-security-key session/relay owner; depends on ADR046-security-key-008 and the ComponentSession/Process contracts. |
| Current source | `packages/d2bd/src/security_key.rs` - baseline internal `SecurityKeyState` (renamed `RelaySessionTable` in v3 so state terminology remains reserved for Resource status), `LeaseState`, `LeaseId`, `CidTranslator`, `try_acquire_lease`, `release_lease`, `CEREMONY_TIMEOUT`, `QUEUE_WAIT_TIMEOUT` (implemented-and-reachable) |
| Reuse action | adapt |
| Destination | Move to `packages/d2b-provider-device-security-key/src/session.rs` and `cid.rs`; adapt to Provider Process model (remove daemon Mutex wrapping, add async relay protocol) |
| Detailed design | Extract the baseline lease/session constants and CID mapping into provider-local modules. Preserve `LeaseId` stale-release protection, cancel-all-active-CIDs, `CEREMONY_TIMEOUT`, `QUEUE_WAIT_TIMEOUT`, and bounded fair queue semantics; remove daemon-global `Mutex` ownership; keep the authority relay's DeviceGrant/OFD lease for its lifetime. Ceremony rows remain high-churn session records, never Resources. Primary reuse disposition: `adapt`. Preserved source-plan detail: extract and adapt. |
| Integration | ADR046-security-key-010 owns the relay loop; ADR046-security-key-011/ADR046-security-key-012 consume this extracted foundation; ADR046-security-key-009 consumes lifecycle events and writes bounded Service/Binding observations; ComponentSession/encrypted named streams carry CTAPHID bytes. |
| Data migration | Full d2b 3.0 reset; no v2 state/config import |
| Validation | `session_state_machine.rs`, `session_ring.rs`, `cancel_propagation.rs`, `session_timeout.rs`, `fair_queue.rs`, and `cid_isolation.rs` verify queue/active/completed/timeout transitions, ring eviction, LeaseId stale-release denial, cancel-all-CIDs, fair timeout, and per-session CID isolation with no daemon-global lease state or ceremony Resource. |
| Removal proof | ADR046-security-key-030 deletes the superseded daemon-internal `packages/d2bd/src/security_key.rs` `SecurityKeyState`, `LeaseState`, `SkRegistry`, and accept-loop ownership after the provider relay/session tests pass. |

### ADR046-security-key-002

| Field | Value |
| --- | --- |
| Dependency/owner | ADR-046 provider-device-security-key relay extraction owner; depends on ADR046-security-key-008, ADR046-security-key-001, and the frozen ComponentSession/Endpoint contracts. |
| Current source | `packages/d2bd/src/security_key.rs` - CTAPHID relay loop, `SkAcceptHandle`, `relay_one_ceremony` (implemented-and-reachable) |
| Reuse action | adapt |
| Destination | Move to `packages/d2b-provider-device-security-key/src/relay.rs`; replace daemon-internal Unix socket proxy with ComponentSession over the owned Service Endpoint |
| Detailed design | Extract the CTAPHID ceremony relay behavior into the provider relay binary. Preserve one-ceremony-at-a-time proxy semantics and CTAPHID cancel handling, but replace daemon-internal Unix socket proxying with the `d2b.security-key.v3` ComponentSession over the owned CTAPHID Endpoint and named `ctaphid` stream. Primary reuse disposition: `adapt`. Preserved source-plan detail: extract and adapt. |
| Integration | Core launches the relay Process with a LaunchTicket DeviceGrant and Endpoint attachment; transport-vsock resolves `Endpoint/<service-name>-ctaphid-relay`; frontend Process connects as ComponentSession initiator; controller receives session events over the manifest-declared internal channel. |
| Data migration | Full d2b 3.0 reset; no v2 state/config import |
| Validation | `host_relay_guest_frontend/` integration fixture, `device_grant_no_path.rs`, `descriptor_validation.rs`, and `cancel_propagation.rs` prove relay fd injection, ComponentSession transport, cancel propagation, and absence of daemon-internal socket proxying. |
| Removal proof | ADR046-security-key-030 and ADR046-security-key-031 remove `start_sk_accept_loop`, `SkAcceptHandle`, `relay_one_ceremony`, and the daemon-internal Unix socket proxy bind from `packages/d2bd/src/security_key.rs` and `packages/d2bd/src/lib.rs`. |

### ADR046-security-key-003

| Field | Value |
| --- | --- |
| Dependency/owner | ADR-046 provider-device-security-key frontend extraction owner; depends on ADR046-security-key-008 and frozen Process/ComponentSession contracts. |
| Current source | `packages/d2b-sk-frontend/src/` - `main.rs`, `uhid.rs` (implemented-and-reachable); `framing.rs` and `vsock.rs` are obsolete under v3 (replaced by ComponentSession transport) |
| Reuse action | adapt |
| Destination | Adopt `main.rs` and `uhid.rs` as the v3 Process binary entry point; replace `framing.rs`/`vsock.rs` with ComponentSession client from `d2b-session-unix/src/vsock.rs`; wire as Process service in Provider crate |
| Detailed design | Retain UHID creation and frontend entry behavior, but run it as a Binding-owned v3 user-domain Process receiving a pre-opened `/dev/uhid` fd from the `Provider/system-core` Guest-substrate DeviceGrant. Delete raw frame/vsock protocol and use the ComponentSession client/named `ctaphid` stream. No virtual/projected Device exists. Primary reuse disposition: `adapt`. Preserved source-plan detail: extract and adapt. |
| Integration | ADR046-security-key-026 defines Service/Binding ownership; ADR046-security-key-020/N17 wire the Binding-owned frontend Process/private Endpoint and same-Zone Service resolution; Core injects UHID from the Guest substrate. |
| Data migration | Full d2b 3.0 reset; no frontend session state import |
| Validation | `host_relay_guest_frontend/`, `device_grant_no_path.rs`, `descriptor_validation.rs`, and guest Nix migration tests prove UHID fd injection, no `/dev/uhid` path, ComponentSession client use, and no raw `framing.rs`/`vsock.rs` protocol. |
| Removal proof | ADR046-security-key-032 removes the legacy `d2b-sk-frontend.service` unit declaration, and the v3 frontend excludes the obsolete `packages/d2b-sk-frontend/src/framing.rs` and `vsock.rs` raw transport behavior. |

### ADR046-security-key-004

| Field | Value |
| --- | --- |
| Dependency/owner | Core LaunchTicket/privileged broker reuse owner; depends on ADR-046-resources-device and ADR046-security-key-013 probe/device-token population. |
| Current source | `packages/d2b-priv-broker/src/ops/security_key.rs` - `live_open_hidraw_security_key`, FIDO usage page revalidation, group validation, `ALLOWED_GROUPS` (implemented-and-reachable) |
| Reuse action | adapt |
| Destination | Preserve revalidation logic; update `SecurityKeyOpenDevice` to use bundle device table `device_token` as sole open target (no iterative sysfs scan); add zone-field handling; remove sysfs fallback. **Core's LaunchTicket calls this internally; the Provider does not call it.** |
| Detailed design | Keep the FIDO usage-page and post-open revalidation logic, but make the trusted private bundle `device_token` the only open target. Add Zone-aware request handling, reject path/sysfs fallback inputs, and keep the operation internal to Core LaunchTicket DeviceGrant resolution rather than callable by the Provider controller. |
| Integration | Provider activation records label-to-`device_token` mappings; Core LaunchTicket resolves `deviceUsage` for the relay Process; broker opens and revalidates the hidraw fd; relay receives only an inherited fd; Core emits path-free `device-grant` audit. |
| Data migration | Full d2b 3.0 reset; no v2 device state import |
| Validation | `packages/d2b-priv-broker/tests/security_key_broker.rs` updates for bundle table lookup and zone-field round trip; `device_grant_no_path.rs` proves Provider code does not call the broker and sees no device path; audit tests prove path-free grant records. |
| Removal proof | The superseded iterative sysfs scan/fallback behavior in `packages/d2b-priv-broker/src/ops/security_key.rs` is removed once bundle-token lookup and revalidation tests pass. |

### ADR046-security-key-005

| Field | Value |
| --- | --- |
| Dependency/owner | `d2b-contracts` security-key ceremony/effect DTO owner; depends on ADR046-provider-004, ADR-046-resource-object-model, ADR-046-resources-device, and ADR046-security-key-008. |
| Current source | `packages/d2b-contracts/src/security_key.rs` - `SecurityKeySessionId`, `SecurityKeyDeviceLabel`, `SecurityKeySession`, `SecurityKeySessionResult`, `SecurityKeyStatusResponse`, `SecurityKeySessionsResponse`, `SecurityKeyOpenDeviceRequest`, `SecurityKeyEvent` (implemented-and-reachable) |
| Reuse action | adapt |
| Destination | Adapt to v3 Zone/ResourceRef identifiers; preserve serde shapes for zero downstream breakage where possible; remove `SecurityKeyApplyUdevRulesRequest` (ADR046-security-key-035) |
| Detailed design | Rebase wire DTOs onto v3 Zone/ResourceRef identifiers; consume the shared ADR046-provider-004 `security-key.d2bus.org` Service/Binding bases and define only strict `device-security-key.d2bus.org` Provider-extension DTOs; reject `spec.provider` on Core projections; place authority/import/attachment semantic observations only under `status.resource` and implementation observations only under `status.provider`; preserve opaque bounded ceremony records as non-Resource DTOs; add `zone` to `SecurityKeyOpenDeviceRequest`; drop the udev-rules request because UHID comes from the Guest-substrate DeviceGrant. No provider-named ResourceType alias is admitted. |
| Integration | Core LaunchTicket, broker open op, Provider controller Service/Binding status/audit, CLI session readers, and provider tests consume the v3 DTOs. |
| Data migration | Full d2b 3.0 reset; no v2 DTO compatibility migration beyond serde-shape preservation where possible |
| Validation | DTO serde round trips, exact provider-neutral ResourceType identity, provider-named alias rejection, canonical minimal base acceptance, Core projection `spec.provider` rejection, D088 `status.resource`/`status.provider` layering, base/Provider-extension field separation, unknown-field denial, zone-field round trip, path-redaction tests, and updated `usb_sk_contract.rs` assertions in the provider crate. |
| Removal proof | ADR046-security-key-035 removes `SecurityKeyApplyUdevRulesRequest`, the `SecurityKeyApplyUdevRules` broker op, and related broker code after UHID DeviceGrant coverage is live. |

### ADR046-security-key-006

| Field | Value |
| --- | --- |
| Dependency/owner | Provider crate test owner; depends on ADR046-security-key-005 v3 DTOs and ADR046-security-key-008 provider crate layout. |
| Current source | `packages/d2b-contract-tests/tests/usb_sk_contract.rs` - DTO serde round-trips, unknown-field denial, broker capability set (implemented-and-reachable) |
| Reuse action | adapt |
| Destination | Move to `packages/d2b-provider-device-security-key/tests/`; update imports and v3 type names |
| Detailed design | Move the reusable semantic assertions for security-key DTO serde, unknown-field denial, and broker capability shape into the provider crate's hermetic `tests/` suite, updating imports and names to the v3 contract modules without weakening assertions. Primary reuse disposition: `adapt`. Preserved source-plan detail: move and adapt. |
| Integration | `cargo test -p d2b-provider-device-security-key --lib --tests` runs the moved contract tests with the provider's DTO/controller test matrix; old contract-test manifests point to the successor coverage before deletion. |
| Data migration | None - test-only move; no runtime state |
| Validation | Moved tests pass under the provider crate; contract assertions are retained; D094 disposition records moved/adapted coverage before old duplicate tests are deleted. |
| Removal proof | ADR046-security-key-033 deletes `packages/d2b-contract-tests/tests/usb_sk_contract.rs` only after the provider-crate successor test covers all prior assertions. |

### ADR046-security-key-007

| Field | Value |
| --- | --- |
| Dependency/owner | Provider crate test/minijail adaptation owner; depends on ADR046-security-key-008 and the frozen Process sandbox contract. |
| Current source | `packages/d2b-contract-tests/tests/minijail_sk_frontend.rs` - minijail profile shape, `ProcessRole::SecurityKeyFrontend` (implemented-and-reachable) |
| Reuse action | adapt |
| Destination | Move to `packages/d2b-provider-device-security-key/tests/`; update for v3 Process resource minijail profile; retain zero-capabilities assertion |
| Detailed design | Move the reusable minijail/sandbox assertions into the provider crate and retarget them from `ProcessRole::SecurityKeyFrontend` to the v3 Process resource templates and relay/controller minijail profiles. Preserve zero-capabilities and seccomp-class assertions while recognizing the frontend uses `Provider/system-systemd` hardening rather than a minijail profile. Primary reuse disposition: `adapt`. Preserved source-plan detail: move and adapt. |
| Integration | Provider tests validate Nix minijail profile entries, Process resource sandbox templates, and system-minijail/system-systemd conformance expectations before old contract tests are retired. |
| Data migration | None - test-only move; no runtime state |
| Validation | Provider-crate tests retain zero-capability and seccomp assertions for relay/controller and assert no minijail profile is used for the frontend Process. |
| Removal proof | ADR046-security-key-033 deletes `packages/d2b-contract-tests/tests/minijail_sk_frontend.rs` only after the provider-crate successor test covers all prior assertions. |

### New items

### ADR046-security-key-008

| Field | Value |
| --- | --- |
| Dependency/owner | ADR-046 provider-device-security-key crate owner; depends on provider-model/package workspace policy. |
| Current source | None - net-new v3 work; no pre-ADR45 baseline equivalent |
| Reuse action | create |
| Destination | New crate `packages/d2b-provider-device-security-key/` with `src/`, `tests/`, `integration/`, `README.md` (workspace policy requires all four) |
| Detailed design | New crate `packages/d2b-provider-device-security-key/` with `src/`, `tests/`, `integration/`, `README.md` (workspace policy requires all four) |
| Integration | Workspace/package descriptor expose the crate to Core; ADR046-security-key-009 through ADR046-security-key-029 add controllers, resource contracts, relay/frontend, adapters, tests, and docs. |
| Data migration | Full d2b 3.0 reset; no v2 state/config import |
| Validation | Workspace package-policy check rejects missing `src/`, `tests/`, `integration/`, or `README.md`; `cargo test -p d2b-provider-device-security-key --lib --tests` discovers the hermetic suite; README acceptance criteria from the provider crate standard layout are satisfied. |
| Removal proof | None - net-new; no prior owner to remove |

### ADR046-security-key-009

| Field | Value |
| --- | --- |
| Dependency/owner | Provider controller owner; depends on ADR046-security-key-013 probe, ADR046-security-key-016 templates, ADR046-security-key-025 effect port, ADR046-security-key-026 Service/Binding contracts, ADR046-security-key-027 status contract, and ADR-046-resource-reconciliation. |
| Current source | None - net-new v3 work; no pre-ADR45 baseline equivalent |
| Reuse action | create |
| Destination | `packages/d2b-provider-device-security-key/src/controller.rs` |
| Detailed design | One controller implements standard reconcile for local physical Devices, authority/projection SecurityKeyServices, and SecurityKeyBindings. It observes Devices; realizes an authority Service as relay Process/Service-owned Endpoint; accepts projection Services only from Core/import after signed-factory admission; realizes each Binding as frontend Process/private Endpoint; enforces child-first finalizers and never creates an import or Device projection. Export/Import routing never treats an Endpoint as exported identity. |
| Integration | Watches Device and both provider-neutral semantic types filtered by `providerRef=Provider/device-security-key`, plus Process, Endpoint, Guest/User, ResourceExport/Import, and Service/export/import-owner dependency indexes; writes semantic base plus signed Provider-extension status/finalizers; drives relay-control messages. |
| Data migration | Full d2b 3.0 reset; no v2 state/config import |
| Validation | `controller_reconcile.rs`, `service_binding_projection.rs`, `mutual_exclusion.rs`, `status_binding.rs`, and deletion/finalizer tests cover authority/projection/Binding branches, signed-factory admission before projection reconcile, Service-owned Endpoint isolation from Export/Import identity, no Device projection, and no Volume API calls. |
| Removal proof | None - net-new; no prior owner to remove |

### ADR046-security-key-010

| Field | Value |
| --- | --- |
| Dependency/owner | Authority Service relay owner; depends on ADR046-security-key-001, ADR046-security-key-002, ADR046-security-key-011, ADR046-security-key-012, ADR046-security-key-014, ADR046-security-key-016, and ADR046-security-key-018. |
| Current source | None - net-new v3 work; no pre-ADR45 baseline equivalent |
| Reuse action | create |
| Destination | `packages/d2b-provider-device-security-key/src/relay.rs` |
| Detailed design | Authority Service-owned relay entry point: bounded authenticated Binding connections, one LeaseId-guarded fair ceremony queue, CID translation/cancel-all-CIDs, hidraw fd from Core DeviceGrant, CTAPHID named stream, and internal relay-control channel. |
| Integration | Core injects the Service's physical DeviceGrant, relay Endpoint, and controller channel; Bindings that reference authority or projection Services connect through same-Zone Service Endpoints; Core releases grant on relay exit. |
| Data migration | Full d2b 3.0 reset; no relay session state import |
| Validation | `host_relay_guest_frontend/`, `fair_queue.rs`, `device_grant_no_path.rs`, `descriptor_validation.rs`, `cancel_propagation.rs`, and `cid_isolation.rs` prove one authority open, multi-Binding fair serialization, fd-only access, LeaseId cancel, and CID isolation. |
| Removal proof | Supersedes daemon-internal relay behavior removed by ADR046-security-key-030/ADR046-security-key-031 after relay Process tests pass. |

### ADR046-security-key-011

| Field | Value |
| --- | --- |
| Dependency/owner | Relay ceremony-session foundation owner; depends on ADR046-security-key-001 and ADR046-security-key-008. |
| Current source | None - net-new v3 work; no pre-ADR45 baseline equivalent |
| Reuse action | create |
| Destination | `packages/d2b-provider-device-security-key/src/session.rs` |
| Detailed design | `SessionStateMachine` with Idle/Queued/Active/Completed/TimedOut, bounded FIFO queue, monotonic LeaseId stale-release guard, per-Binding session ring, timeout/cancel, and ring eviction. Ceremony rows are non-Resource records; DeviceGrant remains held for relay lifetime. |
| Integration | ADR046-security-key-010 consumes it; controller receives lifecycle messages; Service/Binding status receives aggregates only; session query/audit consumes bounded non-secret rows. |
| Data migration | Full d2b 3.0 reset; no session ring import |
| Validation | `session_state_machine.rs`, `session_ring.rs`, `fair_queue.rs`, `session_timeout.rs`, and `cancel_propagation.rs` cover queue fairness, eviction, stale LeaseId rejection, timeout, and cancel. |
| Removal proof | None - net-new; no prior owner to remove |

### ADR046-security-key-012

| Field | Value |
| --- | --- |
| Dependency/owner | Relay CID-translation foundation owner; depends on ADR046-security-key-001 and ADR046-security-key-008. |
| Current source | None - net-new v3 work; no pre-ADR45 baseline equivalent |
| Reuse action | create |
| Destination | `packages/d2b-provider-device-security-key/src/cid.rs` |
| Detailed design | CID translator: per-active-ceremony u32→u64 host-CID allocation, bimap, cancel-all-active-CIDs, and eviction on ceremony end |
| Integration | Relay rewrites frontend CTAPHID CIDs before sending to hidraw fd and reverses responses before writing the ComponentSession named stream; session teardown drops the map. |
| Data migration | Full d2b 3.0 reset; CID maps are transient and not imported |
| Validation | `cid_isolation.rs` verifies per-session allocation, round trip, no sharing across relays, and eviction on session end. |
| Removal proof | None - net-new; no prior owner to remove |

### ADR046-security-key-013

| Field | Value |
| --- | --- |
| Dependency/owner | Probe/effect-port and activation owner; depends on ADR046-security-key-025 effect port and Core private bundle device table support. |
| Current source | None - net-new v3 work; no pre-ADR45 baseline equivalent |
| Reuse action | create |
| Destination | `packages/d2b-provider-device-security-key/src/probe.rs`; Provider activation/Core private bundle device table population for label → `device_token` |
| Detailed design | hidraw probe: `probe.rs` - calls `SecurityKeyEffectPort::observe_inventory(&device_id, &policy_id)` with opaque types injected by Core; interprets `InventoryObservation`; never reads `/sys/class/hidraw/` directly; bundle device table population at activation time (Provider activation resolves label → `device_token` via Core; stored in private bundle) |
| Integration | Controller scheduled-observe invokes `probe.rs`; Core adapter implements `SecurityKeyEffectPort`; Nix activation emits private label-to-token bundle entries; Device status receives `DevicePresent` and phase updates. |
| Data migration | Full d2b 3.0 reset; no v2 probe state import |
| Validation | `controller_reconcile.rs` scheduled-observe tests, `descriptor_validation.rs` Debug-redaction capture, and path-safety tests prove Provider never reads sysfs and receives only opaque observations. |
| Removal proof | Supersedes provider-side or broker fallback sysfs scanning; ADR046-security-key-004/ADR046-security-key-018 removal proof verifies only bundle `device_token` lookup remains. |

### ADR046-security-key-014

| Field | Value |
| --- | --- |
| Dependency/owner | ComponentSession/security descriptor contract owner; depends on ADR046-security-key-008, ADR046-security-key-005, and ADR-046-componentsession-and-bus. |
| Current source | None - net-new v3 work; no pre-ADR45 baseline equivalent |
| Reuse action | create |
| Destination | `packages/d2b-provider-device-security-key/src/descriptor.rs` |
| Detailed design | Declare relay↔controller service and relay↔Binding-frontend `d2b.security-key.v3` fingerprints, Noise profiles, canonical Service/Binding subject pairing, bounded encrypted-stream records, and descriptor validation; no ambient path or raw vsock CID. |
| Integration | Provider descriptor declares services and fingerprints; LaunchTicket injects internal channel and Endpoint transport; relay/controller/frontend validate descriptors and peer authority before exchanging messages. |
| Data migration | Full d2b 3.0 reset; no v2 transport/session state import |
| Validation | `descriptor_validation.rs` covers wrong service, wrong descriptor digest, wrong SO_PEERCRED uid, unenrolled key, oversized records, no ambient path, and redacted opaque IDs. |
| Removal proof | None - net-new; no prior owner to remove |

### ADR046-security-key-015

| Field | Value |
| --- | --- |
| Dependency/owner | Sandbox/minijail foundation owner; depends on ADR046-security-key-008, ADR046-security-key-007, and ADR-046-components-processes-and-sandbox. |
| Current source | None - net-new v3 work; no pre-ADR45 baseline equivalent |
| Reuse action | create |
| Destination | `nixos-modules/minijail-profiles.nix` entries for relay and controller; provider descriptor sandbox templates for relay/controller/frontend |
| Detailed design | Minijail profiles for relay and controller only; frontend uses `Provider/system-systemd` hardening directives compiled from `SandboxSpec` (no minijail profile for frontend). Add relay and controller entries to `nixos-modules/minijail-profiles.nix`; `capabilityClasses: []`; `seccompClass: sk-relay` and `seccompClass: sk-controller` |
| Integration | Nix minijail profiles feed system-minijail Process launches for controller/relay; frontend Process template feeds system-systemd hardening; provider tests assert the split. |
| Data migration | Full d2b 3.0 reset; no sandbox state import |
| Validation | `minijail_sk_frontend` successor tests, sandbox template tests, and zero-capability/seccomp assertions cover relay/controller minijail profiles and no frontend minijail profile. |
| Removal proof | Supersedes `ProcessRole::SecurityKeyFrontend`-centric minijail test ownership removed by ADR046-security-key-033/ADR046-security-key-034 after Process-resource coverage passes. |

### ADR046-security-key-016

| Field | Value |
| --- | --- |
| Dependency/owner | Provider process/Endpoint-template owner; depends on ADR046-security-key-008, ADR046-security-key-015, and ADR046-security-key-026 Service/Binding ownership contracts. |
| Current source | None - net-new v3 work; no pre-ADR45 baseline equivalent |
| Reuse action | create |
| Destination | Provider descriptor Process templates and owned CTAPHID `Endpoint` template for `Provider/device-security-key` |
| Detailed design | Templates: Provider controller; authority Service-owned relay Process/relay Endpoint; Binding-owned frontend Process/private Endpoint; projection Service local Endpoint. Frontend requires Guest/User and the system-core UHID DeviceGrant; no virtual Device template exists. |
| Integration | Core creates controller; Provider controller realizes authority Services and Bindings plus each projection Service's ordinary local import-route Endpoint; Process Providers launch children and preserve ownerRef boundaries. |
| Data migration | Full d2b 3.0 reset; no v2 processes.json import |
| Validation | `controller_reconcile.rs`, Process template golden tests, Endpoint resource tests, and frontend `userRef` admission tests prove templates and Endpoint shape. |
| Removal proof | Supersedes the legacy readiness-only `ProcessRole::SecurityKeyFrontend` tracking node removed by ADR046-security-key-034 after v3 Process resources are live. |

### ADR046-security-key-017

| Field | Value |
| --- | --- |
| Dependency/owner | Provider package descriptor owner; depends on ADR046-security-key-008, ADR046-security-key-005, ADR046-security-key-014, ADR046-security-key-016, ADR046-security-key-026, ADR046-security-key-027, and ADR-046-provider-model-and-packaging. |
| Current source | None - net-new v3 work; no pre-ADR45 baseline equivalent |
| Reuse action | create |
| Destination | Signed Provider descriptor JSON for `Provider/device-security-key` in the provider package |
| Detailed design | Signed descriptor: config; physical Device integration; implementation claim for the provider-neutral `security-key.d2bus.org` Service/Binding base schemas/fingerprints; strict `device-security-key.d2bus.org` spec/status extensions; authority/projection union and D097 descriptor; a D096 projection factory with exact `serviceType`, `projectionSchemaFingerprint`, and semantic `factoryFingerprint`; controller/relay/frontend/Endpoint templates; export/import adapter capability; ComponentSession services; empty ProviderStateSet; permission claims. Provider/adapter identity is signed separately and Service-owned Endpoints are not factory or Export fields. |
| Integration | Core ProviderDeployment verifies the signed descriptor, installs ResourceApiBinding and component descriptors, exposes service fingerprints to ComponentSession validation, and supplies permission claims/RBAC bindings. |
| Data migration | Full d2b 3.0 reset; no provider descriptor import |
| Validation | Descriptor schema validation, semantic-base versus Provider-extension fingerprints, exact projection-schema/factory fingerprint derivation and stability under Provider/adapter identity changes, exact type/no-alias tests, service inventory tests, permission claim tests, empty ProviderStateSet tests, and README/provider package conformance checks. |
| Removal proof | None - net-new; no prior owner to remove |

### ADR046-security-key-018

| Field | Value |
| --- | --- |
| Dependency/owner | Core LaunchTicket/broker owner; depends on ADR046-security-key-004, ADR046-security-key-005, ADR046-security-key-013, and ADR-046-resources-device. |
| Current source | None - net-new v3 work; no pre-ADR45 baseline equivalent |
| Reuse action | create |
| Destination | v3 `SecurityKeyOpenDevice` broker op and Core LaunchTicket DeviceGrant resolution path |
| Detailed design | v3 `SecurityKeyOpenDevice` broker op update: add `zone` field; implement bundle device table `device_token` lookup as sole open path; remove iterative sysfs scan from broker; add post-open revalidation steps (fstat, HIDIOCGRAWINFO, HIDIOCGRDESC). This is an internal Core operation called by LaunchTicket; the Provider controller does not call it. |
| Integration | Authority Service controller derives relay `deviceUsage` from `spec.provider.settings.deviceRef`; Core admits authority then resolves DeviceGrant through the private bundle table; broker returns an fd to Core; projection Services never enter this path. |
| Data migration | Full d2b 3.0 reset; no v2 broker state import |
| Validation | Broker unit tests for zone field and token lookup, path-rejection tests, post-open revalidation tests, and provider tests proving no Provider broker call or sysfs path. |
| Removal proof | Superseded broker iterative sysfs scan behavior is removed; tests prove only bundle `device_token` lookup is accepted for `SecurityKeyOpenDevice`. |

### ADR046-security-key-027

| Field | Value |
| --- | --- |
| Dependency/owner | Provider state/status contract owner; depends on ADR046-security-key-008, ADR046-security-key-026, and ADR-046-provider-state. |
| Current source | None - net-new v3 work; no pre-ADR45 baseline equivalent |
| Reuse action | create |
| Destination | Provider descriptor state declaration, controller/status logic, Process templates, and Nix principal provisioning for `Provider/device-security-key` |
| Detailed design | Empty ProviderStateSet and strict bounded status schemas: physical presence in Device `status.resource`; semantic authority/import aggregates in Service `status.resource`; attachment aggregates in Binding `status.resource`; initial physical-backing claim, relay, Endpoint, queue, and ceremony observations only in `status.provider`. No semantic field appears directly under `status`, and Core projections contain no `spec.provider`. Ceremony rows remain high-churn non-Resource session records; CTAP/fd/LeaseId/CID data stays transient. No Process has `/state`. |
| Integration | ADR046-security-key-017 signs schemas; ADR046-security-key-009 writes resource-local status; Core Operation/session/audit surfaces own bounded records; Volume controllers see no request. |
| Data migration | Full d2b 3.0 reset; no v2 state/config import |
| Validation | `status_binding.rs` proves empty ProviderStateSet, no `/state` mounts, no Volume API calls, authority/import/attachment fields only under `status.resource`, implementation fields only under `status.provider`, no projection `spec.provider`, and no CTAP/fd/session secrets in status/log/audit/metrics. |
| Removal proof | None - net-new; no prior owner to remove |

### ADR046-security-key-019

| Field | Value |
| --- | --- |
| Dependency/owner | Nix resource compiler owner; depends on ADR046-security-key-017, ADR046-security-key-026, ADR046-zone-control-024, and ADR-046-nix-configuration. |
| Current source | None - net-new v3 work; no pre-ADR45 baseline equivalent |
| Reuse action | create |
| Destination | `nixos-modules/` resource compiler/eval assertions for physical Device, authority Service, ResourceExport/Import, and consumer Binding |
| Detailed design | Compile the owner Device→Service→export and consumer import→projection-Service→Binding shape. Emit Export `resourceRef`, `serviceType`, `projectionSchemaFingerprint`, and `factoryFingerprint`, and matching Import `expectedServiceType`, `expectedProjectionSchemaFingerprint`, and `expectedFactoryFingerprint`; the Import `exportKey` identifies the ResourceExport. Reject Export Endpoint/custom-key fields, authored projections, projection `spec.provider`, Device export/projection, cross-Zone refs, duplicate authorities/Bindings, paths, and any security-key/USB configuration that does not collide through the exact Core-derived `(Host, physical-usb-backing, opaqueKeyDigest)` tuple after trusted identity resolution. |
| Integration | Nix emits Device/authority Service/export/import/Binding with canonical D096 fields; Core alone creates projection Service; the Service controller alone owns relay/import-route Endpoints; bundle feeds Provider and authority-index admission. |
| Data migration | Full d2b 3.0 reset; current Nix options migrate to v3 Zone resources without state import |
| Validation | Nix eval tests for label resolution, `busClass=hidraw`, exclusive arbitration, exact canonical Export/Import field emission and fingerprint matching, rejection of obsolete Export `endpointRef`/`exportedType`/`baseSchemaFingerprint`/`exportKey` and Import `expectedType`/`expectedBaseSchemaFingerprint`/`projectionType`, Core-only projection without `spec.provider`, byte-identical USB/security-key physical backing tuple collision, Provider-private-class bypass rejection, prohibited fields, and providerRef resolution. |
| Removal proof | Supersedes current option shape only after v3 Zone resource option parity; legacy security-key/USBIP mutual-exclusion assertion is replaced by v3 resource assertion coverage. |

### ADR046-security-key-020

| Field | Value |
| --- | --- |
| Dependency/owner | Guest Nix migration owner; depends on ADR046-security-key-003, ADR046-security-key-016, and ADR046-security-key-026 Binding-owned frontend contract. |
| Current source | None - net-new v3 work; no pre-ADR45 baseline equivalent |
| Reuse action | create |
| Destination | `nixos-modules/components/security-key-guest.nix` migration gate `d2b.securityKey._legacySystemdUnit` |
| Detailed design | Guest Nix module migration gate: `d2b.securityKey._legacySystemdUnit` option, defaulting to false when Provider is installed; remove `d2b-sk-frontend.service` unit |
| Integration | Guest Nix keeps `uhid` and the static frontend binary; Binding controller owns frontend lifecycle and system-core supplies the UHID DeviceGrant; no Device row or udev rule is emitted. |
| Data migration | Full d2b 3.0 reset; no legacy frontend unit state import |
| Validation | Nix eval tests show the legacy unit is absent by default with Provider installed, can be gated only during transition if required, and `uhid` module/binary wiring remains present. |
| Removal proof | ADR046-security-key-032 deletes the superseded `nixos-modules/components/security-key-guest.nix` `d2b-sk-frontend.service` declaration after the gate defaults to false. |

### ADR046-security-key-021

| Field | Value |
| --- | --- |
| Dependency/owner | Audit owner for Core device-grant and Service/Binding lifecycle; depends on ADR046-security-key-009, ADR046-security-key-018, and ADR-046-telemetry-audit-and-support. |
| Current source | None - net-new v3 work; no pre-ADR45 baseline equivalent |
| Reuse action | create |
| Destination | Core `device-grant` audit and Provider controller Service/Binding ceremony lifecycle audit |
| Detailed design | Path-free authority-grant records from Core and bounded Service/Binding/session digests/outcomes from controller; no path, raw target identity, LeaseId, session content, or CTAP bytes. `resource_name_digest` is admitted only in the Core authority-grant audit after DeviceGrant authorization and is never copied to OTEL. |
| Integration | Core emits grant audit; controller emits Service/Binding lifecycle audit; Zone stream stores bounded records; CLI/support consumes digests/outcomes. |
| Data migration | Full d2b 3.0 reset; no v2 audit import |
| Validation | Audit tests assert path-free fields, bounded digests, no guest name/session content/CTAP bytes, grant emitted by Core not Provider controller, and lifecycle emitted by controller. |
| Removal proof | None - net-new; no prior owner to remove |

### ADR046-security-key-022

| Field | Value |
| --- | --- |
| Dependency/owner | Observability owner; depends on ADR046-security-key-010 relay, ADR046-security-key-009 controller, and ADR-046-telemetry-audit-and-support. |
| Current source | None - net-new v3 work; no pre-ADR45 baseline equivalent |
| Reuse action | create |
| Destination | Provider/controller bounded telemetry emitter and observability-otel handoff for security-key metrics |
| Detailed design | OTEL metrics: `d2b_device_sk_session_total`, `d2b_device_sk_ceremony_duration_seconds`, `d2b_device_sk_relay_restarts_total` via bounded emitter ring; descriptors use only closed semantic labels and never Zone/resource-name-derived identity. Provider spans use only fixed operation/phase/outcome/error-class attributes. Neither metrics nor spans admit a resource name, UID, ref, digest (including `resource_name_digest`), session ID, or derived identity token, while `d2b.zone` and `d2b.provider` remain OTEL Resource attributes. |
| Integration | Relay/controller write metric events to the bounded ring; observability-otel Provider drains and exports; dashboards/CLI consume closed labels and bounded histograms. |
| Data migration | Full d2b 3.0 reset; no v2 telemetry import |
| Validation | `telemetry_identity_canaries.rs` and metric inventory tests structurally assert closed label/span-attribute sets; exact absence of `vm`, `zone`, `zone_id`, `zone_uid`, resource-name-derived keys, and `resource_name_digest`; Device/Service/Binding/Guest/Zone name, UID, ref, and digest canary absence from metrics and spans; retained `d2b.zone` Resource attributes; bounded ring behavior; and correct session/ceremony/restart counters. |
| Removal proof | None - net-new; no prior owner to remove |

### ADR046-security-key-023

| Field | Value |
| --- | --- |
| Dependency/owner | Provider documentation owner; depends on ADR046-security-key-008 through ADR046-security-key-022 and ADR046-security-key-024 through ADR046-security-key-029 for complete behavior. |
| Current source | None - net-new v3 work; no pre-ADR45 baseline equivalent |
| Reuse action | create |
| Destination | `packages/d2b-provider-device-security-key/README.md` |
| Detailed design | README: initial Provider identity, provider-neutral Service/Binding catalog, strict Provider-extension fields, physical Device, owner/export/import/projection/Binding chain, process ownership, RBAC, invariants, status/telemetry, no-alias rule, and commands |
| Integration | Workspace/package policy and provider crate acceptance use the README as the human entry point; docs link to it for provider-local build/test/integration commands. |
| Data migration | None - docs/tooling only; no runtime state |
| Validation | README presence check from provider crate standard layout; documentation review verifies every listed section and command is present and matches the crate/package behavior. |
| Removal proof | None - net-new; no prior owner to remove |

### ADR046-security-key-024

| Field | Value |
| --- | --- |
| Dependency/owner | Endpoint/ComponentSession integration owner; depends on ADR046-security-key-003, ADR046-security-key-010, ADR046-security-key-014, ADR046-security-key-016, ADR046-security-key-026, and ADR-046-componentsession-and-bus. |
| Current source | None - net-new v3 work; no pre-ADR45 baseline equivalent |
| Reuse action | create |
| Destination | Authority/projection Service Endpoint and Binding private Endpoint resolution, including transport-vsock and ZoneLink encrypted streams |
| Detailed design | Resolve each Binding only through its same-Zone Service Endpoint; enroll Noise KK for Service/Binding frontend; authority uses transport-vsock locally, projection uses per-import bounded encrypted stream with credits/backpressure/generation/deadline/cancel. |
| Integration | Service/Binding-owned Endpoints produce opaque LaunchTicket attachments; the import adapter binds the projection Service's ordinary local import-route Endpoint; no remote Ref, FD, or raw locator is exposed. |
| Data migration | Full d2b 3.0 reset; no v2 transport state import |
| Validation | `host_relay_guest_frontend/` and `descriptor_validation.rs` verify Endpoint resolution, Noise KK enrollment, attachment opacity, and no raw vsock CID/port in status/spec. |
| Removal proof | Supersedes baseline `vsock.sock_14320` raw port usage; tests prove no `vsockPort` or raw AF_VSOCK framing remains for security-key transport. |

### ADR046-security-key-025

| Field | Value |
| --- | --- |
| Dependency/owner | `d2b-contracts` neutral effect-port foundation owner; depends on ADR046-security-key-008 and ADR-046-resources-device. |
| Current source | None - net-new v3 work; no pre-ADR45 baseline equivalent |
| Reuse action | create |
| Destination | `d2b-contracts` neutral `SecurityKeyEffectPort` trait/types; `packages/d2b-provider-device-security-key/src/effect_port.rs` re-export; Core adapter implementation in `d2b-provider` or `d2b-provider-toolkit` |
| Detailed design | Define/re-export the opaque redacting `SecurityKeyEffectPort` types in the neutral contract crate and implement the Core adapter; inject per physical Device into the Provider controller; relay and projection Service do not receive the port. |
| Integration | Core resolves Zone/label to opaque IDs and injects the port into the controller; controller scheduled-observe calls the trait; Provider crate depends only on the neutral contract/re-export; relay path is unaffected. |
| Data migration | Full d2b 3.0 reset; no v2 effect-port state import |
| Validation | Unit tests assert Debug redaction, controller calls `observe_inventory` with injected IDs, relay has no port dependency, and fake Core adapter returns bounded `InventoryObservation`. |
| Removal proof | None - net-new; no prior owner to remove |

### ADR046-security-key-026

| Field | Value |
| --- | --- |
| Dependency/owner | Device-security-key Service/Binding implementation owner; depends on ADR046-provider-004, ADR046-security-key-008, ADR046-security-key-005, resource object/Device/D096/D097 contracts. |
| Current source | None - net-new v3 work; no pre-ADR45 baseline equivalent |
| Reuse action | create |
| Destination | `packages/d2b-provider-device-security-key/src/{resource_type,provider_extension,admission}.rs`; controller contracts; system-core Guest UHID authority-subresource DeviceGrant (common base lives under ADR046-provider-004) |
| Detailed design | Bind the shared semantic authority/projection Service and Binding base versions/fingerprints from ADR046-provider-004, then define only the initial strict Provider extension and admission. The owner/Binding extension references the local physical Device/relay Endpoint and owns CTAPHID/fairness/frontend settings and observations. Projection is Core-owned by ResourceImport with `providerRef` plus semantic base/import fields, no `spec.provider`, and no Device/open; routing derives from the signed local descriptor, `providerRef`, and import record. Export admission binds the authority Service's `resourceRef` and `serviceType` to the signed projection-schema and factory fingerprints, never to its Endpoint. Binding is operator intent and the initial extension realizes its frontend Process/private Endpoint. Standard Device remains physical only; provider-named ResourceType aliases are rejected. |
| Integration | ResourceExport targets Service with canonical type/fingerprint fields; ResourceImport supplies matching expected fields and creates projection Service; Binding references same-Zone Service; Service controllers retain Endpoint ownership; Core injects Guest UHID without a Device row. |
| Data migration | Full d2b 3.0 reset; no legacy Device/claim projection import |
| Validation | Fast schema/lifecycle conformance consumes the ADR046-provider-004 fixtures, accepts canonical minimal base without `spec.provider`, includes a fake alternate security-key Provider, and proves Device→provider-neutral Service→export→import→projection Service→provider-neutral Binding→frontend, exact canonical Export/Import fields, no Endpoint export, projection `spec.provider` rejection, D088 status layering, strict base/Provider-extension separation, exact types with no aliases, strict ownership/finalizers, no Device projection, and no local hidraw open in consumer Zone. |
| Removal proof | Supersedes legacy frontend/import Device modeling; ADR046-security-key-035 removes udev mutation and ADR046-security-key-032 removes the legacy unit once Binding-owned realization is live. |

### Removal items

### ADR046-security-key-030

| Field | Value |
| --- | --- |
| Dependency/owner | Provider-device-security-key removal owner; depends on ADR046-security-key-001, ADR046-security-key-002, ADR046-security-key-010, ADR046-security-key-011, and ADR046-security-key-012 successor relay/session coverage. |
| Current source | `packages/d2bd/src/security_key.rs` - `start_sk_accept_loop`, `SecurityKeyState`, `LeaseState`, `SkRegistry` |
| Reuse action | delete-after-cutover |
| Destination | Removed from daemon; successor behavior lives in `packages/d2b-provider-device-security-key/src/relay.rs`, `session.rs`, and `cid.rs` |
| Detailed design | Remove target `packages/d2bd/src/security_key.rs` - `start_sk_accept_loop`, `SecurityKeyState`, `LeaseState`, `SkRegistry` after v3 relay Process is live and stable; keep behind feature gate only if needed during transition. Primary reuse disposition: `delete-after-cutover`. Preserved source-plan detail: delete. |
| Integration | d2bd no longer owns security-key accept/session state; Provider controller, authority Service relay, and Binding frontends own lifecycle; Core LaunchTicket owns hidraw/UHID grants. |
| Data migration | Full d2b 3.0 reset; no daemon session state migration |
| Validation | Provider relay/session tests pass; daemon build has no references to removed symbols; no legacy security-key accept loop starts under d2bd. |
| Removal proof | Concrete removed path/behavior: `packages/d2bd/src/security_key.rs` `start_sk_accept_loop`, `SecurityKeyState`, `LeaseState`, and `SkRegistry` daemon-internal accept/session ownership are absent. |

### ADR046-security-key-031

| Field | Value |
| --- | --- |
| Dependency/owner | d2bd integration removal owner; depends on ADR046-security-key-030. |
| Current source | `packages/d2bd/src/lib.rs` - `start_sk_accept_loop` call site and daemon-internal Unix socket proxy bind |
| Reuse action | delete-after-cutover |
| Destination | Removed from daemon startup; successor launch path is ProviderDeployment/controller-created relay Process plus Endpoint/ComponentSession transport |
| Detailed design | Remove target `packages/d2bd/src/lib.rs` - `start_sk_accept_loop` call site and daemon-internal Unix socket proxy bind after ADR046-security-key-030. Primary reuse disposition: `delete-after-cutover`. Preserved source-plan detail: delete. |
| Integration | d2bd startup no longer binds a security-key Unix socket proxy; Core/ProviderDeployment starts provider controller and relay Process resources; transport-vsock Endpoint supplies frontend connectivity. |
| Data migration | Full d2b 3.0 reset; no daemon socket state migration |
| Validation | d2bd startup tests/build prove no `start_sk_accept_loop` call or security-key proxy bind remains; provider integration test proves CTAPHID flow through Endpoint/ComponentSession. |
| Removal proof | Concrete removed path/behavior: `packages/d2bd/src/lib.rs` no longer calls `start_sk_accept_loop` and no longer binds the daemon-internal security-key Unix socket proxy. |

### ADR046-security-key-032

| Field | Value |
| --- | --- |
| Dependency/owner | Guest Nix module removal owner; depends on ADR046-security-key-020, ADR046-security-key-003, and ADR046-security-key-026 Binding frontend/UHID contract. |
| Current source | `nixos-modules/components/security-key-guest.nix` - `d2b-sk-frontend.service` systemd unit declaration |
| Reuse action | delete-after-cutover |
| Destination | Removed from guest Nix module; successor is Binding-owned `Process/binding-<uid-short>-sk-frontend` |
| Detailed design | Remove target `nixos-modules/components/security-key-guest.nix` - `d2b-sk-frontend.service` systemd unit declaration after ADR046-security-key-020 migration gate defaults to false. Primary reuse disposition: `delete-after-cutover`. Preserved source-plan detail: delete. |
| Integration | Guest Nix keeps `uhid` and frontend binary only; Provider controller creates the Binding-owned frontend; system-systemd manages it. |
| Data migration | Full d2b 3.0 reset; no legacy unit state migration |
| Validation | Nix eval tests prove no static `d2b-sk-frontend.service` is emitted with Provider installed; frontend Process integration proves replacement lifecycle. |
| Removal proof | Concrete removed path/behavior: `nixos-modules/components/security-key-guest.nix` no longer declares the static `d2b-sk-frontend.service` unit. |

### ADR046-security-key-033

| Field | Value |
| --- | --- |
| Dependency/owner | Test-suite migration/removal owner; depends on ADR046-security-key-006 and ADR046-security-key-007 provider-crate successor tests. |
| Current source | `packages/d2b-contract-tests/tests/minijail_sk_frontend.rs` and `packages/d2b-contract-tests/tests/usb_sk_contract.rs` |
| Reuse action | delete-after-cutover |
| Destination | Removed from `packages/d2b-contract-tests/tests/`; successor tests live in `packages/d2b-provider-device-security-key/tests/` |
| Detailed design | Remove target `packages/d2b-contract-tests/tests/minijail_sk_frontend.rs` and `packages/d2b-contract-tests/tests/usb_sk_contract.rs` after ADR046-security-key-006/ADR046-security-key-007 tests are in Provider crate and cover all prior assertions. Primary reuse disposition: `delete-after-cutover`. Preserved source-plan detail: delete after move/adapt. |
| Integration | D094 disposition updates closed gate manifests, layer1 jobs, pins, ledgers, and CI shards so only the provider-crate successor suite remains. |
| Data migration | None - test-only move/delete; no runtime state |
| Validation | Provider-crate tests pass with retained assertions; old contract-test paths are absent from manifests/CI; no duplicate old/new suite runs indefinitely. |
| Removal proof | Concrete removed paths: `packages/d2b-contract-tests/tests/minijail_sk_frontend.rs` and `packages/d2b-contract-tests/tests/usb_sk_contract.rs` are deleted after provider-crate successor coverage passes. |

### ADR046-security-key-034

| Field | Value |
| --- | --- |
| Dependency/owner | Core ProcessRole removal owner; depends on ADR046-security-key-016 Process resources, ADR046-security-key-015 sandbox templates, and system-minijail/system-systemd conformance. |
| Current source | `ProcessRole::SecurityKeyFrontend` in `d2b-core/src/processes.rs` |
| Reuse action | delete-after-cutover |
| Destination | Removed from `d2b-core/src/processes.rs`; successor frontend is a v3 Process resource owned by `Provider/device-security-key` |
| Detailed design | Remove target `ProcessRole::SecurityKeyFrontend` in `d2b-core/src/processes.rs` after relay and frontend are v3 Process resources; no other code reference expected. Primary reuse disposition: `delete-after-cutover`. Preserved source-plan detail: delete. |
| Integration | ProcessRole disposition table confirms all security-key frontend lifecycle, sandbox, readiness, and DeviceGrant semantics are represented by Resource Process templates and Process Providers before enum removal. |
| Data migration | Full d2b 3.0 reset; no processes.json role migration |
| Validation | Workspace build proves no `ProcessRole::SecurityKeyFrontend` references; provider Process template tests prove the v3 replacement; process conformance passes. |
| Removal proof | Concrete removed path/behavior: `d2b-core/src/processes.rs` no longer contains `ProcessRole::SecurityKeyFrontend` or a security-key frontend role in the legacy ProcessRole/VmProcessDag model. |

### ADR046-security-key-035

| Field | Value |
| --- | --- |
| Dependency/owner | Broker/contracts/Nix removal owner; depends on ADR046-security-key-005, ADR046-security-key-018, ADR046-security-key-020, and ADR046-security-key-026 Guest-substrate UHID replacement. |
| Current source | `SecurityKeyApplyUdevRules` broker op, `SecurityKeyApplyUdevRulesRequest` DTO in `packages/d2b-contracts/src/security_key.rs`, and all related broker code |
| Reuse action | delete-after-cutover |
| Destination | Removed from contracts and broker; successor access is static guest Nix `uhid` module plus Core pre-opened `/dev/uhid` DeviceGrant for the frontend Process |
| Detailed design | Remove `SecurityKeyApplyUdevRules`, its DTO, and related broker code after the Binding-owned frontend and system-core UHID DeviceGrant are live. Guest Nix loads `uhid` but emits no security-key udev rule. Primary reuse disposition: `delete-after-cutover`. Preserved source-plan detail: delete. |
| Integration | Guest Nix/Process DeviceGrant path provides UHID access; contracts no longer expose the op/request; broker capability set drops the udev mutation; provider/contract tests assert absence. |
| Data migration | Full d2b 3.0 reset; no udev rule state migration |
| Validation | DTO unknown-field/capability tests prove `SecurityKeyApplyUdevRulesRequest` and op are absent; `device_grant_no_path.rs` proves frontend has UHID fd without udev/plugdev; broker build has no related code. |
| Removal proof | Concrete removed path/behavior: `SecurityKeyApplyUdevRules` broker operation, `SecurityKeyApplyUdevRulesRequest` in `packages/d2b-contracts/src/security_key.rs`, and related broker code are absent. |

### ADR046-security-key-028

| Field | Value |
| --- | --- |
| Dependency/owner | Cross-Zone adapter owner; depends on ADR046-security-key-024, ADR046-security-key-026, ADR046-security-key-029, ADR046-zone-control-019, and ADR046-zone-control-020. |
| Current source | None - net-new ADR 0046 cross-Zone sharing (D096) |
| Reuse action | adapt |
| Destination | `packages/d2b-provider-device-security-key/src/share_adapter.rs` |
| Detailed design | Signed adapters admit ResourceExport only when `resourceRef` names an authority SecurityKeyService, `serviceType` is `security-key.d2bus.org.SecurityKeyService`, and `projectionSchemaFingerprint` plus `factoryFingerprint` match the signed semantic factory. ResourceImport must supply the corresponding `expectedServiceType`, `expectedProjectionSchemaFingerprint`, and `expectedFactoryFingerprint`; its `exportKey` identifies the ResourceExport. The Service's relay Endpoint stays a Service-owned implementation child and is never an Export field. Core invokes the factory to create one projection SecurityKeyService with `ownerRef: ResourceImport/<name>`, `providerRef`, semantic base/import fields, and no `spec.provider`; route selection comes from the signed local descriptor and ResourceImport record. The semantic factory fingerprint binds factory metadata plus projection-protocol version only, while adapter identity is authenticated separately by the signed Provider descriptor. They never project Device or auto-create Binding. Route Binding ceremonies over bounded encrypted named streams to the single authority fair queue; no FD/USBIP/hidraw/ref crosses Zones. Primary reuse disposition: `adapt`. Preserved source-plan detail: net-new (implement the signed security-key export/import adapter). |
| Integration | Core export/import routing/projection lifecycle; ADR046-security-key-029 authority; ADR046-security-key-024 Endpoint streams; Nix/operator-authored Binding consumes the same-Zone projection. |
| Data migration | Full d2b 3.0 reset; no cross-Zone sharing state |
| Validation | Fast fake-stream conformance proves owner Service→export→import→projection Service→Binding→frontend; exact canonical Export/Import type and fingerprint fields; rejection of Export `endpointRef`, `exportedType`, `baseSchemaFingerprint`, and `exportKey` plus Import `expectedType`, `expectedBaseSchemaFingerprint`, and `projectionType`; rejection of projection `spec.provider`; semantic factory-fingerprint stability when signed adapter identity changes; separate signed-descriptor identity authentication; one fair LeaseId-guarded ceremony; ciphertext to intermediaries; no Device projection/local hidraw/FD/USBIP; revocation degradation; and audit metadata only. |
| Removal proof | Not applicable (new surface) |

### ADR046-security-key-029

| Field | Value |
| --- | --- |
| Dependency/owner | D097 authority foundation owner; depends on ADR046-security-key-001, ADR046-security-key-002, ADR046-security-key-003, ADR046-security-key-004, ADR046-security-key-018, ADR046-security-key-026, ADR046-zone-control-024, and the D097 authority contract. |
| Current source | `packages/d2bd/src/security_key.rs` (`CidTranslator`, `SecurityKeyState`, `LeaseId`/`LeaseState`, `CEREMONY_TIMEOUT` 120 s, `QUEUE_WAIT_TIMEOUT` 15 s, `parse_ctaphid_report`/`build_cancel_packet`); `packages/d2b-priv-broker/src/ops/security_key.rs` (`live_open_hidraw_security_key`, double `fstat` + FIDO usage-page 0xF1D0 + HID raw-info revalidation, `O_RDWR\|O_NONBLOCK\|O_NOFOLLOW`); `packages/d2b-sk-frontend/src/{main,uhid,vsock,framing}.rs` (UHID FIDO2 CTAPHID frontend, 64-byte report relay) |
| Reuse source | Same baseline daemon/broker/frontend symbols |
| Reuse action | adapt |
| Destination | `packages/d2b-provider-device-security-key/src/{authority,relay,streams}.rs`; D097 `AuthorityDescriptor` on authority SecurityKeyService |
| Detailed design | The provider-neutral authority Service, not Device/Endpoint/Process, is the stable D097 owner and carries the semantic opaque Host-scoped zero-or-one descriptor. The initial Provider extension references the local physical Device and relay Endpoint and supplies service-specific physical-key derivation, Service+relay ownerProof, and bounded-fairness details. After trusted USB identity resolution, Core additionally derives `physical-usb-backing/v1` and atomically claims the exact `(Host, physical-usb-backing, opaqueKeyDigest)` tuple used by every USB Provider before any open, withhold, bind, module, relay, or attachment effect; Provider-private claims cannot replace it. Preserve sole Core open with double-fstat/FIDO/HID validation, async fd I/O, per-session CidTranslator, LeaseId stale-release guard, cancel-all-CIDs, one ceremony, bounded FIFO wait, and Binding-owned UHID frontend. Ceremony rows are not Resources. Primary reuse disposition: `adapt`. Preserved source-plan detail: `adapt` - relay becomes the D097 hidraw authority; transport moves to Endpoint/named-stream. |
| Integration | Authority Service owns relay/Endpoint; Core index admits it and LaunchTicket supplies physical DeviceGrant; ADR046-security-key-028 exports/imports Service; Binding owns frontend/private Endpoint; USBIP conflict remains Host-wide. |
| Data migration | Full d2b 3.0 reset; no per-session/lease state persisted |
| Validation | Fast hermetic tests adapt the existing `CidTranslator`/lease/cancel/UHID/broker-revalidation suites: CID alloc/translate/release, `LeaseId` stale-release, cancel-all-CIDs on disconnect, one-ceremony + 120 s timeout, 15 s fair-wait `ERR_CHANNEL_BUSY`, UHID frame round-trip, broker double-`fstat`+FIDO+HID revalidation, byte-identical USB/security-key backing tuple derivation for one fake token, and `physical-usb-backing-conflict` before effects under alternate labels/private authority classes - all with fakes/`FakeEffectPort`, no real hidraw. Integration proves cross-Zone CTAP ceremony **serialization** over the encrypted named stream and the shared physical USB collision. |
| Removal proof | The legacy daemon accept loop, raw CTAPHID framing, fixed `SK_VSOCK_PORT`, and broker sysfs `/sys/class/hidraw/` scan fallback are deleted only after the relay `Endpoint`/named-stream successor and the `device_token`-only broker open are green (coordinated with ADR046-security-key-034 `ProcessRole` removal and ADR046-security-key-004 broker-op revalidation). |

Per D094, each replaced current-code test is retired with an explicit
keep/adapt/move/delete disposition and a removal gate: the minimum reusable
semantic assertions migrate into this crate's hermetic `tests/`, and the old
duplicate tests, shell gates, fixtures, static artifacts, CI jobs, and manifest
entries are deleted once successor coverage and the removal proof pass -
updating `tests/layer1-jobs.json`, the closed gate manifests, the
flake/matrix/Nix-unit pins, the generated ledgers, and the CI workflow shards.
Old and new suites never run in parallel indefinitely.

## Tests

### Fast hermetic execution and test placement (D094)

Per D094 and `ADR-046-validation-and-delivery` §10.16, this Provider's `src/`
unit tests and `tests/*.rs` hermetic suite are fast, in-process, deterministic,
and parallel-safe: an individual normal test has p95 ≤50 ms with no wall-clock
sleep, and `cargo test -p d2b-provider-device-security-key --lib --tests`
completes in ≤2 s warm-cache execution time (compilation excluded). They use a
deterministic fake clock/RNG and the toolkit fakes/FakeEffectPort only - no
process spawn, container, network, DBus, systemd, broker daemon, Nix eval/build,
KVM, USB/GPU/TPM hardware, or live cloud, and no filesystem tree beyond tiny
temp fixtures. Any scenario needing those lives only in `integration/`, which
keeps a lane timeout/budget, parallel isolation, and fake external services by
default; such a need is re-placed into `integration/`, never given a sleep,
larger timeout, or `#[ignore]`. Bounded crypto/property tests are the only
classified exception, each named with a capped case count and a declared higher
per-test budget.

### Hermetic (in `tests/`)

| Test file | What is tested |
| --- | --- |
| `resource_chain_conformance.rs` | Fast in-process owner chain: physical Device → authority `security-key.d2bus.org.SecurityKeyService`/Provider relay Endpoint → ResourceExport → ResourceImport → Core-owned projection Service (`ownerRef: ResourceImport/<name>`, `providerRef`, semantic base/import fields, no `spec.provider`, Device, or hidraw) → Nix/operator `security-key.d2bus.org.SecurityKeyBinding` → Provider-realized Binding frontend Process/private Endpoint. Verifies canonical Export `resourceRef`/`serviceType`/`projectionSchemaFingerprint`/`factoryFingerprint`, matching Import `expectedServiceType`/`expectedProjectionSchemaFingerprint`/`expectedFactoryFingerprint`, rejection of obsolete Export and Import fields, Service-owned Endpoint isolation, local refs, ownership, finalizer order, semantic factory fingerprint independence from Provider/adapter identity, separately authenticated signed descriptor identity, and no Device projection. |
| `provider_neutral_type_identity.rs` | Admits only the two exact `security-key.d2bus.org` ResourceTypes; rejects provider-named aliases and unknown variants; verifies `providerRef=Provider/device-security-key`, canonical minimal bases, strict authored `spec.provider`/`status.provider` schemas, projection `spec.provider` rejection, D088 status layering, and that physical Device, CTAPHID, hidraw, relay, UHID, queue, and ceremony fields cannot enter either semantic base schema. |
| `ceremony_record_separation.rs` | Resource catalog/store contains Device, Service, Binding, Process, Endpoint, Export, and Import only; ceremony/session/LeaseId/CID/queue/cancel rows remain bounded high-churn records and cannot be admitted as Resource objects or ownerRef children. |
| `controller_reconcile.rs` | Device observation creates no children; authority Service creates relay Process/Endpoint and physical DeviceGrant; projection Service is Core/import-only and creates no hidraw grant; Binding creates frontend Process/private Endpoint and system-core UHID grant; child-first deletion and no Volume calls. |
| `session_ring.rs` | Per-Binding bounded ring evicts oldest at `sessionRingSize`; rows are non-secret non-Resource records |
| `session_state_machine.rs` | Idle → Queued → Active → Completed/TimedOut; FIFO handoff; matching LeaseId cancel; stale release denied; physical DeviceGrant persists until relay exit |
| `fair_queue.rs` | Multiple local/imported Bindings serialize one ceremony; FIFO order, bounded queue, 15 s fake-clock deadline → `ERR_CHANNEL_BUSY`; active Binding unaffected |
| `duplicate_binding_conflict.rs` | Duplicate Service/Guest/User/policy Binding rejected; distinct explicit Bindings admitted without a second hidraw open |
| `device_grant_no_path.rs` | Authority relay receives the sole Core-opened physical DeviceGrant and no hidraw path. Projection Service receives no physical grant/open. Binding frontend receives system-core Guest UHID grant and no `/dev/uhid` path; no virtual/projected Device, udev rule, or plugdev group. |
| `mutual_exclusion.rs` | USB and security-key Providers resolving one fake token through same or different labels submit a byte-identical Core-derived `(Host, physical-usb-backing, opaqueKeyDigest)` tuple; the second fails before effects with `BackingAuthorityReady=False`, `AuthorityConflict=True`, and `physical-usb-backing-conflict`; Provider-private authority classes/digests cannot bypass it; unrelated tokens pass |
| `cancel_propagation.rs` | Matching `{sessionId, LeaseId}` cancel invokes cancel-all-active-CIDs and completes only that Binding; stale LeaseId cannot cancel a later ceremony; DeviceGrant persists until authority relay exit |
| `session_timeout.rs` | CEREMONY_TIMEOUT elapsed → Active → TimedOut → Idle; audit `device-session-timeout`; relay restartable after timeout |
| `cid_isolation.rs` | Different Bindings/ceremonies do not share CID maps; round trip and cancel-all-CIDs; canonical Service/Binding subject only |
| `descriptor_validation.rs` | Relay-control and Service/Binding Noise identities, fingerprints, encrypted stream bounds, SO_PEERCRED, no ambient path/raw CID, and opaque-ID Debug redaction |
| `status_binding.rs` | Empty ProviderStateSet; authority/import/attachment observations occur only in `status.resource`, while shared-backing implementation state and relay/frontend/queue/ceremony observations remain in `status.provider`; no semantic field appears directly under `status`; ceremony rows are not resources/status history; CTAP, fd, LeaseId, CID, and session keys absent |
| `telemetry_identity_canaries.rs` | Exact semantic metric/span allowlists; `resource_name_digest` and Device/Service/Binding/Guest/Zone name, UID, ref, and digest canaries never enter metric labels or span attributes; allow-listed OTEL Resource identity remains |

### Integration (in `integration/`)

| Fixture | What is tested |
| --- | --- |
| `host_relay_guest_frontend/` | Authority Service relay receives fake hidraw DeviceGrant; Binding frontend receives system-core fake UHID grant; owned Endpoints establish Noise KK; 64-byte CTAPHID exchange translates/reverses CID with no Device projection |
| `cross_zone_service_binding/` | Real processes exercise canonical type/fingerprint Export/Import fields, projection Service, and Binding frontend over an encrypted named stream; the relay/import-route Endpoints remain Service-owned and absent from Export/Import; no FD/USBIP/hidraw open occurs in the consumer Zone; revocation degrades Binding |
| `fair_queue/` | Multiple Binding frontends serialize one authority ceremony; bounded wait returns `ERR_CHANNEL_BUSY` without disturbing active holder |
| `usbip_mutual_exclusion/` | Host-wide trusted identity resolution produces the same `physical-usb-backing` tuple for USBIP and security-key; the second claim fails before either duplicate effect |

### Existing contract tests (reuse/update)

| Existing test | Action |
| --- | --- |
| `packages/d2b-contract-tests/tests/usb_sk_contract.rs` | Move to `packages/d2b-provider-device-security-key/tests/` as part of ADR046-security-key-006; update v3 type imports; retain all existing assertions |
| `packages/d2b-contract-tests/tests/minijail_sk_frontend.rs` | Move to `packages/d2b-provider-device-security-key/tests/` as part of ADR046-security-key-007; update for the v3 Process resource sandbox; retain zero-`capabilityClasses` and `seccompClass` assertions |
| `packages/d2b-priv-broker/tests/security_key_broker.rs` | Retain in broker crate; update for v3 bundle table lookup path; add zone-field round-trip test |

## Nix option migration

| Current option | v3 successor |
| --- | --- |
| `d2b.host.usb.securityKey.enable = true` | Install `Provider/device-security-key` in Zone; configure `spec.config.devices` |
| `d2b.host.usb.securityKey.devices[].label` | `spec.config.devices[].label` in Provider resource |
| `d2b.host.usb.securityKey.devices[].vendorId` | `spec.config.devices[].vendorId` in Provider resource |
| `d2b.host.usb.securityKey.devices[].productId` | `spec.config.devices[].productId` in Provider resource |
| `d2b.vms.<vm>.securityKey.enable = true` | Declare a `SecurityKeyBinding` for the Guest/User referencing a same-Zone authority or projection `SecurityKeyService` |
| (none - was not configurable) | `sessionRingSize`, `leaseTimeoutSecs`, `queueWaitTimeoutSecs` in Provider root config |

The current USBIP/security-key mutual-exclusion assertion is preserved as a
Host-wide authority-index/eval assertion. Nix preflights known-equivalent
selectors across all Zones, while runtime Core resolves trusted physical USB
identity and requires both implementations to claim the byte-identical
`(Host, physical-usb-backing, opaqueKeyDigest)` tuple. The second claim receives
`physical-usb-backing-conflict` before either effect; a Provider-private
authority cannot bypass the shared tuple.

## References

- `packages/d2bd/src/security_key.rs` - baseline relay implementation (implemented-and-reachable)
- `packages/d2b-sk-frontend/src/` - baseline guest frontend binary (implemented-and-reachable)
- `packages/d2b-priv-broker/src/ops/security_key.rs` - broker hidraw open op (implemented-and-reachable)
- `packages/d2b-contracts/src/security_key.rs` - public and broker wire DTOs (implemented-and-reachable)
- `packages/d2b-contract-tests/tests/usb_sk_contract.rs` - existing contract tests (implemented-and-reachable)
- `packages/d2b-contract-tests/tests/minijail_sk_frontend.rs` - existing minijail contract tests (implemented-and-reachable)
- `packages/d2b-priv-broker/tests/security_key_broker.rs` - existing broker tests (implemented-and-reachable)
- `docs/specs/ADR-046-resources-device.md` §Provider: device-security-key - Device ResourceType contract and key invariants
- `docs/specs/ADR-046-componentsession-and-bus.md` - ComponentSession Noise profiles, descriptor validation, attachments
- `docs/specs/ADR-046-provider-model-and-packaging.md` - Provider crate boundary, component descriptors
- `docs/specs/ADR-046-components-processes-and-sandbox.md` - Process model, ProviderSupervisor, minijail
- `docs/specs/ADR-046-resource-reconciliation.md` - standard async reconcile interface
- `docs/specs/ADR-046-telemetry-audit-and-support.md` - OTEL label constraints, audit stream
- `docs/specs/ADR-046-nix-configuration.md` - Nix resource compilation, prohibited fields, eval invariants
- `nixos-modules/components/security-key-guest.nix` - current guest Nix module (eval-contract)
- `nixos-modules/assertions.nix` - current mutual-exclusion assertion (eval-contract)
