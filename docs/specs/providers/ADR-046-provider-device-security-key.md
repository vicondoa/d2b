# ADR 0046 Provider dossier: device-security-key

| Field | Value |
| --- | --- |
| Spec ID | `ADR-046-provider-device-security-key` |
| Parent | ADR 0046 |
| Status | Proposed |
| Version | 5 |
| Baseline | `b5ddbed67867d9244bf33390868101bd9b053e49` |
| Normative | Yes |
| Owners | `packages/d2b-provider-device-security-key/`, Device controller contracts, Nix device emitters |
| Depends on | `ADR-046-resources-device`, `ADR-046-resource-object-model`, `ADR-046-primitive-resource-composition`, `ADR-046-resource-api-and-authorization`, `ADR-046-resource-reconciliation`, `ADR-046-provider-model-and-packaging`, `ADR-046-components-processes-and-sandbox`, `ADR-046-componentsession-and-bus`, `ADR-046-telemetry-audit-and-support`, `ADR-046-nix-configuration`, `ADR-046-provider-state` |
| Supersedes | `ProcessRole::SecurityKeyFrontend` daemon-internal accept loop (`packages/d2bd/src/security_key.rs`), `nixos-modules/components/security-key-guest.nix` untracked guest `d2b-sk-frontend.service` unit |

## Purpose

This dossier exhaustively specifies the `device-security-key` Provider in d2b 3.0.
It covers the complete physical hidraw device discovery, identity, probe, and claim
contract; the one-session-per-device exclusive lease model; the unprivileged Host
relay Process and Guest frontend Process that the Provider owns; the broker-only
fd handoff path; ComponentSession-protected service and stream surfaces with
descriptor validation; lifecycle, disconnect, and cancel semantics; RBAC and
security invariants; root configuration and settings schema; device status,
conditions, and phase transitions; error classes; audit and OTEL placement; Nix
declaration and eval-time constraints; async reconcile loop triggers; all reuse
work items mapping baseline code to v3 destinations; required tests; and the
current-code removal sequence.

No raw key data, physical hidraw node path, sysfs bus path, vendor/product
string concatenation, or device serial number ever appears in any public or
broker-wire surface. All external claims use opaque stable labels or
session-scoped digests only.

## Identity

```text
Provider/device-security-key
```

Crate: `packages/d2b-provider-device-security-key/`

Provider implements: Device ResourceType (physical, `busClass: hidraw`, exclusive
per-session lease, `arbitration: exclusive`, `maxConcurrentClaims: 1`).

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
| `devices` | list | `[]` | 0–16 entries | Per-selector entries; may be empty when Zone configures no security-key Devices |
| `devices[].label` | string | — | `^[a-z][a-z0-9-]{0,62}$` | Stable operator-defined selector label; unique within the config |
| `devices[].vendorId` | uint16 | — | 0x0001–0xFFFE | USB vendor ID |
| `devices[].productId` | uint16 | — | 0x0001–0xFFFE | USB product ID |
| `devices[].serial` | string \| null | `null` | ≤ 128 UTF-8 chars, no NUL | Optional serial filter; null matches any serial |
| `sessionRingSize` | uint | `32` | 8–256 | Maximum recent-session ring entries per Device; oldest entry evicted when full |
| `leaseTimeoutSecs` | uint | `300` | 30–3600 | Per-session ceremony lease timeout in seconds; maps to `CEREMONY_TIMEOUT` |
| `queueWaitTimeoutSecs` | uint | `15` | 5–120 | Maximum wait for a busy lease before the relay returns `ERR_CHANNEL_BUSY`; maps to `QUEUE_WAIT_TIMEOUT` |

**Prohibited fields:** `devices[].hidrawPath`, `devices[].sysfsPath`, any field
containing a raw filesystem path. The Provider derives the physical node from
the trusted bundle device table at runtime; no path is accepted in config.

**Duplicate labels** are rejected at Provider spec admission. Labels that do not
match `^[a-z][a-z0-9-]{0,62}$` are rejected. An empty `devices` list is valid;
the controller remains installed but creates no Device sub-resources.

## Device spec contract

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
```

The `inventory.selector.label` must match exactly one entry in `Provider/device-security-key`
`spec.config.devices[].label`. At admission, the controller verifies the label is
present in the Provider's installed config. An unresolvable label fails Device
spec admission with `device-not-found` condition.

`arbitration` and `maxConcurrentClaims` for this Provider are always `exclusive`
and `1`. Any other value is rejected at admission.

`busClass` must be `hidraw`. Any other `busClass` is rejected at admission.

## hidraw discovery and identity

### Discovery contract

Physical FIDO/hidraw device discovery is split between the Core/broker adapter
(which owns sysfs matching, device-token resolution, and any related audit) and the
Provider controller (which receives only opaque `InventoryObservation` results through
the injected `SecurityKeyEffectPort`). The Provider process never reads `/sys`,
opens hidraw device nodes, manages OFD lock paths, calls broker operations, or
receives raw device paths or UIDs.

### Bundle device table (trusted source — Core/adapter)

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

No path or UID crosses any wire. No Provider code participates in steps 1–5.

### Identity and FIDO usage page

A hidraw node is FIDO-class if its report descriptor contains usage page `0xF1D0`
(`[0x06, 0xD0, 0xF1]` — usage-page item type 0x04, 2-byte little-endian `[0xD0,
0xF1]`). This check is performed by Core during revalidation. It is the sole
hardware-identity signal; Core does not verify attestation, AAGUID, or authenticator
model at open time.

### Probe semantics

The Device controller schedules `scheduled-observe` at the configured interval
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
/// Encodes zone, interval, and selector context — Core resolves these; the
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

The Provider owns three process classes per Device resource: a controller (static,
created by Core's ProviderDeployment), a Host relay Process, and a Guest frontend
Process. The controller and relay are host-side system processes; the frontend is
a user-domain process inside the Guest VM.

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
    class: provider-defined for the lifetime of the Provider installation;
Core creates it once via ProviderDeployment and restarts it on failure. It holds
Zone resource API authority (Device `get`, `list`, `watch`, `create`,
`update-spec`, `update-status`, `update-finalizers`, `delete`) for Device
resources whose `providerRef` resolves to this Provider. The controller creates,
updates, and finalizes relay and frontend Process resources and the virtual
frontend Device. It receives relay session lifecycle events via the
`device-security-key.relay-ctrl.v1` service bound at the `ctrl-relay` endpoint.

### Host relay Process

```yaml
type: Process
metadata:
  name: device-<uid-short>-sk-relay
  zone: <zone>
  ownerRef: Device/<device-name>
spec:
  providerRef: Provider/system-minijail
  executionRef: Host/host-system
  domain: system
  processClass: service
  template: sk-relay
  deviceUsage:
    - deviceRef: Device/<device-name>
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

`uid-short` is the first 12 hex characters of the owner Device resource UID. The
Process name never contains a Guest human name. The Device UID is stable across
daemon restarts.

The relay Process:

- Receives the hidraw fd from its LaunchTicket DeviceGrant at process start. Core
  opens, revalidates, and passes the pre-opened fd. The relay **never calls any
  broker operation** and never sees a device path or UID.
- The `deviceUsage: access: exclusive` DeviceGrant IS the exclusive lock. Core
  holds the corresponding OFD lease for the relay's process lifetime; it releases
  automatically when the relay exits for any reason (clean or crash).
- Serves the `d2b.security-key.v3` ComponentSession over the
  `Endpoint/<device-uid>-sk-ctaphid-relay` resource resolved through Provider/transport-vsock (see §ComponentSession).
- Accepts at most one concurrent Guest frontend connection. A second concurrent
  connect attempt is held for up to `queueWaitTimeoutSecs` before receiving
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
- Connects to the Device controller's manifest-declared internal service
  `device-security-key.relay-ctrl.v1` using the bound internal channel FD from its
  LaunchTicket. Reports session lifecycle events and
  receives `CancelSession` signals.
- Has narrow ComponentSession service authority: responder of
  `d2b.security-key.v3` and client of `device-security-key.relay-ctrl.v1` only.
  No Zone resource API authority, no broker connection, no write access to Zone
  resource store.
- On crash, Core releases the DeviceGrant and OFD lease; the Device controller
  observes `owned-resource-changed` and sets `DeviceHealthy=False`. The relay is
  restarted after back-off.

The Host relay produces the stable CTAPHID relay service as an owned `Endpoint`
resource:

```yaml
apiVersion: resources.d2bus.org/v3
type: Endpoint
metadata:
  name: device-<uid-short>-sk-ctaphid-relay
  zone: <zone>
  ownerRef: Device/<device-name>
spec:
  providerRef: Provider/device-security-key
  producerRef: Process/device-<uid-short>-sk-relay
  endpointClass: device
  transport: vsock
  purpose: device-security-key.d2bus.org/ctaphid-relay
  serviceFingerprint: device-security-key.d2bus.org/SecurityKeyCtapRelay.v3
  locality: cross-domain
  visibility: authorized-consumers
  attachmentPolicy: component-session
  consumerPolicy: device-security-key.d2bus.org/frontend-only
  lifecyclePolicy: recycle-with-producer
status:
  readiness: Ready
  observedProducerGeneration: 1
  observedResourceGeneration: 1
  endpointGeneration: 1
  connectionAvailability: available
  leaseAvailability: lease-required
```

## Endpoint resources (D092)

`Provider/device-security-key` conforms to the standard `Endpoint` base schema.
The stable CTAPHID relay/frontend service identity is an owned `Endpoint`
resource with `producerRef`; the Guest frontend consumes it as `Endpoint/<name>`.
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
  handles are high-churn ceremony/session state and are never promoted.
- The manifest-declared relay-controller channel handle is controller-internal;
  it is not independently consumed outside this Provider.
- `OwnedTransport` and named CTAPHID streams are in-memory transport
  capabilities behind Endpoint resolution, not Endpoint identities.
- `operationId` values remain opaque audit/idempotency correlation handles.

**Current implementation note (implemented-and-reachable):** In the baseline the
relay is NOT a separate process — it is a daemon-internal loop in
`packages/d2bd/src/security_key.rs`. `ProcessRole::SecurityKeyFrontend` is a
readiness-only tracking node, not a spawned process. The v3 Provider extracts
this logic into `d2b-provider-device-security-key` as a proper unprivileged
`system`-domain Process receiving its hidraw fd from the LaunchTicket.

### Virtual frontend Device

For each exclusive Device claim, the controller creates a child virtual Device
that represents `/dev/uhid` access in the claiming Guest. This model avoids two
exclusive uses of the same physical Device resource and gives the frontend its own
DeviceGrant, which Core uses to pre-open `/dev/uhid` and pass the fd.

```yaml
type: Device
metadata:
  name: <device-name>-frontend
  zone: <zone>
  ownerRef: Device/<device-name>    # child of the physical Device
spec:
  providerRef: Provider/device-security-key
  deviceClass: virtual
  arbitration: exclusive
  inventory:
    selector:
      busClass: uhid
  provider:
    schemaId: "device-security-key.d2bus.org/Device/spec"
    schemaVersion: "1.0.0"
    settings:
      bindGuest: Guest/<vm>           # exact Guest that holds the claim
```

The controller creates this virtual Device as part of `spec-generation-changed`
and updates its `spec.provider.settings.bindGuest` whenever the claiming Guest
changes. Core
resolves the virtual Device's DeviceGrant by opening `/dev/uhid` inside the Guest
at relay/frontend launch time and passing the pre-opened fd. Because `/dev` is
masked in the frontend's sandbox, the frontend process has no path to any device
node; it receives only the pre-opened UHID fd through the LaunchTicket.

### Guest frontend Process

```yaml
type: Process
metadata:
  name: device-<uid-short>-sk-frontend
  zone: <zone>
  ownerRef: Device/<device-name>
spec:
  providerRef: Provider/system-systemd
  executionRef: Guest/<vm>
  domain: user
  processClass: service
  template: sk-frontend
  userRef: User/<workload-user>          # required; Guest executionRef must have defaultUserRef if absent
  deviceUsage:
    - deviceRef: Device/<device-name>-frontend
      access: exclusive
      purpose: uhid-virtual-fido
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

`executionRef` is set to the Guest that holds the exclusive Device claim. The
Device controller creates this Process resource after the claim is confirmed.

The frontend `userRef` is required. If absent, the `executionRef` Guest must
declare a `defaultUserRef`; the controller fails Process admission if neither is
present (user-domain Process requires a resolved user identity).

`providerRef: Provider/system-systemd` manages the frontend as a transient
user-scope systemd unit. The `sandbox` spec fields compile into systemd service
hardening directives (no minijail profile for the frontend).

The `deviceUsage` entry references the controller-created virtual
`Device/<device-name>-frontend`. Core opens `/dev/uhid` in the Guest's execution
context and passes the pre-opened fd via the frontend's LaunchTicket. No udev
rule, no wildcard device permission, and no ambient `/dev` access is needed —
the guest `/dev` is masked in the frontend's sandbox and only the pre-opened UHID
fd is available.

The frontend Process:

- Runs as a user-domain process inside the Guest under `Provider/system-systemd`.
- Receives the pre-opened UHID fd from its LaunchTicket DeviceGrant for
  `Device/<device-name>-frontend`.
- Creates a virtual FIDO2 CTAPHID HID device (UHID_CREATE event) on the received
  UHID fd with the FIDO usage descriptor. The virtual device is visible to libfido2,
  browsers, and `pamu2fcfg` inside the Guest.
- Connects to the host relay as initiator of the `d2b.security-key.v3`
  ComponentSession over the `Provider/transport-vsock` resolved `Endpoint/<device-uid>-sk-ctaphid-relay` resource.
  Reconnects automatically on ComponentSession drop (tolerates relay restarts).
- Reads UHID_OUTPUT events from the Guest kernel and sends them to the relay over
  the named CTAPHID stream. Reads relay responses and injects them via UHID_INPUT2.
- Has narrow ComponentSession client authority: initiator of `d2b.security-key.v3`
  only. No resource API authority, no Zone bus access.
- Binary: `packages/d2b-sk-frontend/` (static binary; implemented-and-reachable).

**Current implementation note:** Baseline `d2b-sk-frontend.service` is an
untracked Guest systemd unit. The v3 target removes that unit when the Process
resource is live (see W-N13).

## ComponentSession: relay server endpoint

The Host relay Process has two channel types: an external ComponentSession serving
the Guest frontend, and a manifest-declared typed internal ComponentSession service
to the Device controller.

### CTAPHID relay ComponentSession (relay ↔ Guest frontend)

The relay Process serves the `d2b.security-key.v3` ComponentSession over
`Provider/transport-vsock`. This is the sole CTAPHID transport in v3; no parallel
raw AF_VSOCK framing exists.

**Transport allocation:** `Provider/transport-vsock` resolves the owned
`Endpoint/<device-uid>-sk-ctaphid-relay` resource into an opaque vsock attachment
for the relay and frontend LaunchTickets. The relay does not bind a raw vsock
port. The resolved transport handle is opaque to operators, never configurable as
a port number, and never appears in Endpoint spec/status. `vsockPort` does not
exist in v3.

**Noise profile:** enrolled KK (`Noise_KK_25519_ChaChaPoly_SHA256`). Both relay
and frontend static keys are enrolled at Process provisioning time before the first
connection. The relay acts as responder; the frontend acts as initiator.

**Named CTAPHID stream:** one bounded bidirectional named stream `ctaphid` within
the session:
- Maximum message size: 64 bytes (one CTAPHID report); any message exceeding 64
  bytes is a protocol error — both ends close the session on receipt.
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
vsock client from `d2b-session-unix/src/vsock.rs` (see W-R03).

### Manifest-declared relay ↔ controller service

The relay connects to the Device controller's typed internal ComponentSession
service `device-security-key.relay-ctrl.v1`. This service is declared in the
Provider's package descriptor (manifest); there is no ambient filesystem socket
path. The bound internal channel FD is injected into the relay's LaunchTicket as a
controller-internal attachment — the relay never resolves a path to find the
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

Each relay Process has one session at a time. The relay holds the hidraw fd from
its LaunchTicket DeviceGrant for its entire lifetime; the DeviceGrant IS the
exclusive lock. The session state machine tracks per-ceremony state:

```
Idle  (relay alive; hidraw fd held via DeviceGrant; ready to accept connection)
  │  (Guest frontend ComponentSession connects)
  ▼
Active (holderRef: Guest/<vm>, sessionId: sk-<uid-short>-<counter>)
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
| `Idle → Active` | Guest frontend ComponentSession connects | Relay allocates session ID; reports `SessionStarted` to controller; updates Device status claim |
| `Active → Completed` | CTAP op complete or CTAPHID_CANCEL received | Relay reports `SessionCompleted`; updates status; ready for next connection |
| `Active → TimedOut` | `CEREMONY_TIMEOUT` (120 s) elapsed | Same as Completed; audit reason `session-timeout` |
| `Active → Completed` via controller | Controller `CancelSession { session_id }` | Relay sends CTAPHID_CANCEL to token; transitions to Completed (reason=operator-cancel); reports `SessionCompleted` |
| `Completed/TimedOut → Idle` | (internal transition) | Relay ready for next frontend connection |

**One active session per relay:** while `Active`, subsequent frontend connect
attempts wait up to `queueWaitTimeoutSecs` (15 s) then receive `ERR_CHANNEL_BUSY`.
One relay per Device is enforced by the exclusive DeviceGrant (Core prevents two
concurrent exclusive grants for the same Device).

**Session ID format:** `sk-<uid-short>-<monotonic-counter>` where `uid-short` is
the first 12 hex chars of the Device UID and `counter` is a per-relay-process
monotonic u64. Session IDs are opaque and stored in the bounded session ring
(max `sessionRingSize` entries) for status queries and cancelled-session tracking.

### CID translation

The relay maintains a `CidTranslator` (v3 counterpart to the type of the same
name in `packages/d2bd/src/security_key.rs`). On each incoming guest CTAPHID
packet:

- Bytes 0–3 are the guest-assigned CID (u32, big-endian per CTAPHID spec).
- The relay looks up the guest CID in the translation table. If absent, it
  allocates a new host-assigned CID from a monotonic counter and records the
  mapping.
- The outgoing packet to the token uses the host-assigned CID.
- Incoming token responses use the host-assigned CID; the relay reverses the
  lookup and replaces with the guest CID before forwarding to the frontend.

CID translation is per-relay-session (per vsock connection). The table is
discarded when the session ends. Two simultaneous guest frontends connecting to
different relays on different Devices do not share a CID namespace.

### Disconnect and cancel semantics

**Guest frontend disconnect (clean close):**

1. The relay detects graceful ComponentSession shutdown from the frontend.
2. If a ceremony is in-flight (`Active` state), the relay sends `CTAPHID_CANCEL`
   to the physical token (via the hidraw fd from its LaunchTicket).
3. The relay waits up to 500 ms for the token's cancel acknowledgment.
4. The relay transitions to `Completed` (reason: `client-disconnect`).
5. The relay reports `SessionCompleted` to the Device controller.
6. DeviceGrant/OFD lease remains held (relay is still running and Idle).

**Guest frontend disconnect (crash/abrupt):**

1. The relay detects broken ComponentSession (transport error or POLLHUP).
2. Same cancel-and-complete sequence as clean close.

**Operator cancel (`d2b device cancel <zone> <device-name>`):**

1. The CLI emits a `DeviceCancel` request to the Zone bus.
2. The Device controller sends `CancelSession { session_id }` to the relay via
   the manifest-declared internal service `device-security-key.relay-ctrl.v1`.
3. The relay sends `CTAPHID_CANCEL` to the token, transitions from `Active` to
   `Completed` (reason: `operator-cancel`), and reports `SessionCompleted`.
4. The controller updates Device status and emits an audit record.
5. `CancelSession` is a protocol-level operation on the typed internal service;
   it never implies finalizer removal or DeviceGrant release (the grant persists
   while the relay process lives).

**Relay crash:**

1. Core detects relay process exit; DeviceGrant and OFD lease release automatically.
2. The Device controller observes `owned-resource-changed` → relay Process `Failed`.
3. Controller sets `DeviceHealthy=False`, transitions to `Degraded`, clears claim.
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
- Device spec, status, conditions, or any DeviceClaim field.
- Audit records (path-free; device_label digest only).
- OTEL spans, metrics, or log lines.
- Public wire DTOs (`SecurityKeyStatusResponse`, `SecurityKeySessionsResponse`).
- Broker request or response body.
- Any ComponentSession record or error message.

Device status `device.claims[].sessionId` is an opaque string. It is never the
hidraw node name, the sysfs path, or a concatenation of vendor/product IDs.

### I-3: Exclusive DeviceGrant per relay Process

At most one relay Process can hold the exclusive DeviceGrant for a given Device at
any time. The DeviceGrant IS the exclusive lock; Core enforces that no two
concurrent exclusive grants are issued for the same Device resource. The relay
Process spec declares `deviceUsage: [{deviceRef: Device/<name>, access: exclusive}]`.
Within a relay's lifetime, the relay's session state machine enforces one-at-a-time
frontend connections. On relay exit, Core releases the DeviceGrant and OFD lease
automatically. There is no separate in-relay OFD lock management. A separate
virtual `Device/<name>-frontend` with its own exclusive DeviceGrant is used by
the frontend, avoiding two exclusive uses of the physical Device.

### I-4: Security-key proxy and USBIP are mutually exclusive

A Device resource under `Provider/device-security-key` and a Device resource
under `Provider/device-usbip` that share the same physical USB device (matching
`vendorId` and `productId` and, if configured, `serial`) cannot coexist in the
same Zone.

This is enforced:

**At Nix eval time:** `assertions.nix` emits a hard error if
`d2b.zones.<zone>.resources` contains both a `device-security-key` Device and a
`device-usbip` Device whose selectors match the same physical USB device. The
eval check compares vendorId, productId, and serial. A failing assertion blocks
NixOS activation.

**At Device controller startup:** on `spec-generation-changed`, the controller
queries existing Device resources in the Zone. If any `device-usbip` Device
resource is `Ready` or `Pending` with a selector matching the same physical
device, the `device-security-key` Device transitions to `Failed` with condition
`ClaimConflict` and error `device-mutual-exclusion-violation`.

### I-5: Exclusive claim per Guest

At most one Guest may hold the exclusive hidraw lease at a time. A second Guest
attachment on the same Device is rejected at claim time with `ClaimConflict`
condition. The Device controller writes the conflict condition and sets phase
`Degraded` for the second claimant without terminating the first.

### I-6: Peer authentication

The relay accepts ComponentSession connections only from the enrolled frontend
static key matching the current Device claim's Guest. Peer authentication is
performed by the Noise KK handshake on the `d2b.security-key.v3` session;
the relay rejects any initiator whose static key is not enrolled for the
expected frontend Process. Transport-vsock/ComponentSession authenticates the
expected Guest endpoint and passes a canonical authenticated subject to the relay;
the relay uses only this canonical subject for identity decisions. The relay never
inspects or verifies raw vsock CIDs — those are a transport-internal detail owned
entirely by the transport layer. A connection from an unenrolled key is refused;
the relay does not accept in-band identity claims from the peer.

### I-7: No credential material in log/audit/OTEL

Raw CTAP payloads, PINs, CBOR assertions, credential IDs, WebAuthn responses,
and signature bytes are never logged, audited, or included in OTEL spans or
metrics. Only vm identity (opaque digest), device_label (opaque digest), high-level
op type (acquire/release/timeout/cancel), and lease lifecycle events are emitted.

### I-8: Relay has narrow ComponentSession service authority

The relay Process holds narrow ComponentSession service authority: responder of
`d2b.security-key.v3` and client of the manifest-declared internal service
`device-security-key.relay-ctrl.v1` only. It does not hold a Zone resource API
client, a broker connection, or write access to the Zone resource store. The
internal service channel FD is injected via LaunchTicket; no ambient path is
used.

### I-9: No guest udev rules required

Core pre-opens `/dev/uhid` before the frontend process starts, passing the fd via
the frontend's LaunchTicket DeviceGrant for `Device/<device-name>-frontend`. The
frontend's sandbox masks `/dev`, so the frontend process has no path to any device
node and accesses only the pre-opened UHID fd. No udev rules, no `plugdev` group
membership, and no wildcard device permissions are required in the Guest. The
`SecurityKeyApplyUdevRules` operation is removed from the architecture and no
static udev rule entry is needed in the guest Nix module for UHID access.

### I-10: ProviderStateSet is empty under status-first state

Per D087, `device-security-key` declares **no Provider state Volume** for the
controller, relay, or frontend components. `ProviderStateSet` is the optional
query-time grouping of declared Provider state Volumes; for
`Provider/device-security-key` the set is empty.

Bounded non-secret operational state belongs in the owning `Device` status and
the core Operation ledger. Per D088, the common Device claim/arbitration/presence
base lives in `status.resource`; security-key-specific virtual frontend,
relay/frontend Process, observation timestamp, session lifecycle, and finalizer
progress observations live in `status.provider.details`. CTAP bytes, relay stream
content, UHID/hidraw fds, session keys, and cancellation handles are transient
in-process or fd-scoped data and must never be persisted or exposed through
status.

Storage-need test rationale: v1 security-key controller, relay, and frontend
state has no durable secret recovery payload, no large or binary file content, no
private data that belongs outside authorized status readers, and no
bounded-but-revision-unsuitable recovery payload. Transient CTAP and relay bytes
fail the persistence side of the storage-need test and remain in memory only.

## RBAC


Device resources use the standard Device RBAC verbs defined in `ADR-046-resources-device`.
Additional security-key-specific role bindings:

| Role | Verbs | Scope | Subjects |
| --- | --- | --- | --- |
| `device-manager` | `get`, `list`, `watch`, `create`, `update-spec`, `delete` | Zone | `Provider/device-security-key` controller component only |
| `device-status-owner` | `update-status` | Zone | `Provider/device-security-key` controller component only |
| `device-finalizer-owner` | `update-finalizers` | Zone | `Provider/device-security-key` controller component only |
| `device-reader` | `get`, `list`, `watch` | Zone | Guest/Host runtime Providers, `d2b` CLI (read-only) |
| `device-claimant` | `get`, `watch` | Zone | Guest/Host runtime Providers holding Device claims |
| `device-cancel` | (action verb) | Device/<name> | Admin-only (`d2b` group + SO_PEERCRED admission) |

No Role grants wildcard `*` over Device resources. RoleBindings for
`Provider/device-security-key` cover only Device resources whose `providerRef`
resolves to this Provider.

The relay Process has no RoleBinding to the Zone bus. It cannot invoke any
resource API method. Its typed internal ComponentSession service connection to the
Device controller is manifest-declared and socket-FD-injected via LaunchTicket;
no ambient path is used and the relay cannot open arbitrary Zone bus connections.

## Provider state

Per D087, `device-security-key` declares **no Provider state Volume**. The
controller, relay, and frontend component descriptors contain no state namespace,
no Provider state Volume template, and no `/state` mount. The ProviderStateSet is
empty because no declared Volume passes the storage-need test.

### Controller operational state

Device resources and the core Operation ledger are the authority for controller
decisions. The controller writes bounded phase, conditions, claim observations,
relay/frontend references, observation timestamps, and finalizer progress to the
owning `Device.status`. On restart it re-lists Device, Process, and virtual
frontend Device resources, revalidates external reality, and writes materially
changed status; it never recovers from a private controller state directory.

### Relay operational state

Each relay Process keeps CTAPHID session state, CID translation maps, cancellation
state, and hidraw fd ownership in process memory and inherited LaunchTicket
DeviceGrant fds. These values are transient and authority-conferring; they are not
Volume payloads and must not appear in status, logs, audit, metrics, or persisted
files. Relay lifecycle summaries suitable for readers are written through Device
status and Operation rows only.

### Frontend operational state

Each frontend Process keeps UHID/ComponentSession stream state in process memory
and inherited DeviceGrant fds. No frontend Provider state Volume or host-side
attachment Volume exists in v1. Bounded readiness and error summaries are written
to the owning Device status and Operation ledger.

### Lifecycle

Core ProviderDeployment has no Provider state Volumes to create, mount, migrate,
or delete for `device-security-key`. Process lifecycle still uses genuine Device
attachments and LaunchTicket-injected hidraw/UHID fds; those are Device grants,
not Provider state Volumes. Deletion finalizers stop relay/frontend Processes,
release DeviceGrant/OFD leases, delete the virtual frontend Device, and clear
status/finalizer state without any Volume cleanup step.

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
| `SecurityKeyOpenDevice` | Resolve FIDO hidraw node from trusted bundle `device_token`; open `O_RDWR\|O_NONBLOCK\|O_NOFOLLOW`; fstat/HIDIOCGRDESC/HIDIOCGRAWINFO revalidation; pass fd to relay LaunchTicket | Core LaunchTicket (DeviceGrant resolution) | Yes — path-free; device_label digest, zone, outcome |

The Provider controller never calls `SecurityKeyOpenDevice`. Core emits a
path-free `device-grant` audit record when the hidraw fd is opened; the Provider
controller does not emit this record.

`SecurityKeyApplyUdevRules` is removed from the architecture. Guest Nix supplies
static udev rules at activation time; no runtime broker op writes or applies udev
rules.

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

## Device status

Per D088, ResourceType-common Device observation lives in `status.resource`: the
provider-neutral claim/arbitration/presence base that is identical across Device
implementations. Security-key relay/session observations live only in
`status.provider` with `providerRef`, qualified `schemaId`
`device-security-key.d2bus.org/Device/status`, `schemaVersion`,
`observedProviderGeneration`, and strict bounded redacted `details`
(≤32 KiB, unknown-field-denied). The controller writes all present layers
atomically in one status mutation; shared
fields are never duplicated into `status.provider`, and the extension schema is
registered and signed in the Provider manifest.

**Currency and upgrade (D091).** The controller implements `assess_update`,
`plan_upgrade`, and `execute_upgrade` for security-key relay/frontend
realization and populates only the universal `status.update`, never
`status.provider`, with
`state: Current|UpdateAvailable|UpgradeRequired|Upgrading|Blocked|Unknown`,
`reasons` from `CoreGenerationChanged`, `ProviderGenerationChanged`,
`ArtifactChanged`, `ImageOrSystemGenerationChanged`, `SpecChanged`,
`DependencyChanged`, or `SecurityPolicyChanged`, observed/target
generation/digest IDs, `disruption: None|Reload|Restart|Recycle|Replace`,
`preserveState`, optional `operationId`, `lastAssessedAt`, and
`owned`/`dependencies` refs. It honors base `spec.updatePolicy` (manual
disruptive default; auto non-disruptive), while the Core Operation ledger owns
upgrade operation, idempotency, and progress. Disruptive relay/frontend changes
return `UpgradeRequired` rather than applying in place; the planner recycles the
relay/frontend realization, accepts that transient session state is in memory,
and keeps secrets out of `status.update`. Non-disruptive changes reconcile
normally.

```yaml
status:
  phase: Ready | Pending | Degraded | Failed | Unknown
  conditions:
    - type: DevicePresent
      status: "True" | "False" | "Unknown"
      reason: device-probed-present | device-probe-failed
              | device-consecutive-probe-failures-exceeded
    - type: DeviceClaimed
      status: "True" | "False"
      reason: exclusive-claim-held | no-active-claim
    - type: DeviceHealthy
      status: "True" | "False"
      reason: relay-running | relay-failed | relay-restarting
    - type: ClaimConflict
      status: "True"
      reason: usbip-mutual-exclusion | second-exclusive-claim
  resource:
    present: true | false | null
    health: healthy | degraded | failed | unknown
    holderRefs: ["Guest/<vm>"]          # at most 1 for exclusive
    claims:
      - holderRef: Guest/<vm>
        claim: exclusive
        passthrough: hidraw-relay
        claimedAt: "2026-07-22T00:05:00Z"
        health: healthy | degraded | failed | unknown
    provisionedAt: null                  # physical device; always null
    lastProbedAt: "2026-07-22T00:05:00Z"
  provider:
    providerRef: Provider/device-security-key
    schemaId: "device-security-key.d2bus.org/Device/status"
    schemaVersion: "1.0.0"
    observedProviderGeneration: 1
    details:
      securityKey:
        virtualFrontendRef: Device/<virtual-name>
        relayProcessRef: Process/<relay-name>
        frontendProcessRef: Process/<frontend-name>
        sessionId: "sk-abc123def456-42"  # opaque; NOT hidraw path or descriptor
        sessionState: active | idle | degraded | failed | unknown
        providerDiagnostic: null         # bounded ≤128 UTF-8; never paths/secrets
```

**Phase transitions:**

| Phase | Entry condition |
| --- | --- |
| `Pending` | Device spec committed; controller not yet completed first probe |
| `Ready` | `DevicePresent=True`, relay Process running or no active claim |
| `Degraded` | Probe uncertainty (1–2 consecutive failures), relay `Degraded`/restarting, or `ClaimConflict` for second exclusive claimant |
| `Failed` | Three consecutive probe failures; relay `Failed` after retry exhaustion; `device-mutual-exclusion-violation`; Core DeviceGrant open denied |
| `Unknown` | Controller cannot determine device state (one probe failure; relay status unknown) |

**Condition `ClaimConflict`:** set when a second Guest attempts to claim this
exclusive Device, or when the USBIP mutual-exclusion invariant is violated. The
condition is cleared when the conflicting claim is released or the conflicting
Device resource is deleted.

## Async reconcile loop

The Device controller for `device-security-key` implements the standard async
reconcile interface from `ADR-046-resource-reconciliation`. Trigger handlers:

For `Create`, `UpdateSpec`, or `Delete` with `waitForReconcile` (D090), the
controller performs no external effect, finalizer mutation, or status mutation
until Core supplies
`CommittedRevisionProof {resourceUid,generation,revision,operationId}`. `Abort`
means no effect; a durable commit is never rolled back after a later reconcile
timeout. The response contains the committed object, one-pass projected layered
status, `disposition: Converged|Progressing|Blocked|UpgradeRequired|Failed`,
and `statusPersistence: pending|committed`; effect idempotency keys derive from
`(UID,generation,revision,operationId)` in the same per-resource single-flight
using a bounded priority lane.

### `spec-generation-changed`

1. Validate `inventory.selector.label` resolves against Provider root config.
2. Validate `busClass=hidraw`, `arbitration=exclusive`, `maxConcurrentClaims=1`.
3. Check mutual-exclusion invariant against `device-usbip` Devices in the Zone.
   If violated: set `ClaimConflict` condition, phase `Failed`. Stop.
4. If first-time: install finalizer `device-security-key.d2bus.org/lease-released`.
5. Ensure relay Process resource exists with correct spec, including
   `deviceUsage: [{deviceRef: Device/<device-name>, access: exclusive, purpose: hidraw-fido}]`
   referencing the exact Device and exclusive access. If not: create it.
6. Ensure the virtual frontend Device resource `Device/<device-name>-frontend`
   exists with `ownerRef: Device/<device-name>`, `deviceClass: virtual`,
   `busClass: uhid`. If not: create it. Update `settings.bindGuest` to the
   current claiming Guest if the claim has changed.
7. Ensure frontend Process resource exists with correct spec and `userRef` for
   the claiming Guest user. If not: create it.
8. Trigger `scheduled-observe` to probe device presence.

### `deletion-requested`

1. Send `CancelSession { session_id }` to the relay via the manifest-declared
   internal service `device-security-key.relay-ctrl.v1` if any session is
   `Active`. Wait for `SessionCompleted` or a 5 s timeout.
2. Emit event-only `Deleted` revision for the relay Process resource; Core removes
   the row and index entries atomically; DeviceGrant and OFD lease release
   automatically on relay process stop; audit records appended after commit.
3. Emit event-only `Deleted` revision for the frontend Process resource (if any);
   same atomic row/index removal; audit appended after commit.
4. Emit event-only `Deleted` revision for the virtual frontend Device resource
   (`Device/<device-name>-frontend`); Core removes atomically; audit appended
   after commit.
5. Clear finalizer `device-security-key.d2bus.org/lease-released`.
6. Core removes the physical Device resource after finalizer clears.

### `dependency-changed` / `execution-status-changed`

If the owning Guest stops (Guest phase transitions to `Succeeded`, `Failed`, or
`Unknown`):

1. Send `CancelSession { session_id }` to the relay via the manifest-declared
   internal service if `Active`. DeviceGrant releases automatically on relay stop.
2. Clear the active claim entry from `device.claims`.
3. Transition Device to `Degraded` or `Unknown` per Guest phase.
4. Emit event-only `Deleted` revision for the frontend Process resource; Core
   removes row and index atomically; audit appended after commit.

### `scheduled-observe`

1. Call `SecurityKeyEffectPort::observe_inventory(&device_id, &policy_id)` to
   obtain an `InventoryObservation { present, fido_confirmed }`. Record
   `lastProbedAt`. The Provider never reads `/sys/class/hidraw/` directly; it
   passes only the opaque `DeviceId` and `ObservationPolicyId` injected at startup.
2. On success (`present=true`, `fido_confirmed=true`): reset consecutive-failure
   counter; set `DevicePresent=True`; if phase was `Unknown`/`Degraded` due to
   probe failures, transition to `Ready` (if all other conditions clear).
3. On first failure: increment counter; transition to `Unknown` if currently
   `Ready`.
4. On second failure: remain `Unknown`.
5. On third consecutive failure: set `DevicePresent=False`, transition to
   `Degraded`. Dependent Guests receive `dependency-changed` through normal watch path.
6. When device returns (probe success after failures): reset counter; set
   `DevicePresent=True`; retransition to `Ready`.

### `owned-resource-changed`

If relay Process transitions to `Failed` or `Unknown`:

1. Set `DeviceHealthy=False`.
2. Transition Device to `Degraded`.
3. Restart relay Process after back-off (initial 1 s, ×2, max 30 s, max 5
   attempts before `Failed`).

If frontend Process transitions to `Failed` or `Unknown`:

1. Log `tracing::warn!` with opaque Process resource name (not guest name).
2. Set `DeviceHealthy=False`.
3. Restart frontend Process after back-off.

## Errors

Stable error classes for this Provider (subset of Device common errors plus
security-key-specific additions):

| Error slug | Condition type | Meaning |
| --- | --- | --- |
| `device-not-found` | `DevicePresent=False` | Physical hidraw node absent or label not in bundle table |
| `device-claim-conflict` | `ClaimConflict=True` | A second exclusive claim was attempted |
| `device-grant-denied` | `DeviceHealthy=False` | Core DeviceGrant open denied (revalidation failed or device absent) |
| `device-session-timeout` | — | CTAP ceremony exceeded `leaseTimeoutSecs` |
| `device-session-cancelled` | — | Operator cancel via `d2b device cancel` |
| `device-mutual-exclusion-violation` | `ClaimConflict=True` | USBIP Device for same physical device is active |
| `device-worker-failed` | `DeviceHealthy=False` | Relay Process in `Failed` phase after retry exhaustion |
| `device-cid-collision` | — | Internal CID allocation overflow (monotonic counter wraparound; effectively unreachable at u64) |
| `device-selector-label-unresolvable` | `DevicePresent=False` | `inventory.selector.label` does not match any entry in Provider root config |

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

Excluded: hidraw node path, sysfs bus ID, vendor/product string, serial, device
file descriptor number, CTAP payload, guest VM name. The Provider controller does
not emit this record; Core emits it at DeviceGrant resolution time.

**Device session-lifecycle controller audit** (emitted by Device controller, not
Core; uses Zone runtime audit stream):

```json
{
  "kind": "device-lease",
  "event": "acquired | released | timeout | cancelled | conflict",
  "zone": "<zone>",
  "resource_type": "Device",
  "resource_name_digest": "sha256:<hex of device resource name; not name text>",
  "holder_digest": "sha256:<hex of Guest resource name; not name text>",
  "session_id_digest": "sha256:<hex of session_id>",
  "correlation_id": "<opaque id>",
  "timestamp": "<RFC 3339 UTC>"
}
```

### OTEL telemetry

OTEL span attributes and metric labels follow `ADR-046-telemetry-audit-and-support`.
Constraints specific to this Provider:

- **`d2b.device.provider`** label value: `"device-security-key"` (closed literal).
- **`d2b.device.zone`** label: Zone name. Cardinality ≤ number of Zones.
- **`d2b.device.phase`** label: `"Ready"` | `"Pending"` | `"Degraded"` | `"Failed"` | `"Unknown"`.
- Metric `d2b_device_sk_session_total{zone, outcome}`: counter; `outcome` ∈ `{success, timeout, cancelled, conflict, error}`.
- Metric `d2b_device_sk_ceremony_duration_seconds{zone}`: histogram; bucketed 0–120 s.
- Metric `d2b_device_sk_relay_restarts_total{zone}`: counter.
- No metric or span attribute carries device name (only `resource_name_digest`),
  session ID, guest name, hidraw path, or serial.
- OTEL emitter: lightweight bounded ring (no OTEL SDK in the Provider process;
  tracing crate only). The `observability-otel` Provider drains and forwards.

## Security-key authority and cross-Zone sharing (D096/D097)

**One hidraw authority relay (D097).** The owner-Zone `device-security-key`
**relay** is the sole holder of the physical hidraw FD (invariant I-1); no other
Zone opens the device and no USBIP or direct hidraw access crosses a Zone. The
relay declares a D097 `AuthorityDescriptor` with `authorityScope:
physical-device`, an **opaque** `authorityKey` class (a digest of the trusted
bundle `device_token`/FIDO selector — never a raw hidraw path, sysfs path, or
serial), `cardinality: zero-or-one` per physical token, and `arbitration:
exclusive`. Core's authority index rejects a second relay authority for the same
`(Zone, physical-device, opaqueKeyDigest)` with `duplicateConflict` before any
open; restart adopts the exact authority by `ownerProof` (the relay Process
identity holding the DeviceGrant), and ambiguity quarantines.

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

**Transport change (D096).** Cross-Zone, the fixed vsock accept path is replaced
by the D096/D092 mechanism: the owner Zone declares a `ResourceExport`
referencing only the local relay `Endpoint` and the exported `Device`
(security-key) type (`exportability: explicit-export`, `arbitration: exclusive`);
a child Zone declares a `ResourceImport` binding its local `ZoneLink` +
`exportKey` to a local `Device` projection/frontend (ordinary consumers use that
local `Device` Ref). CTAPHID reports flow over a **per-import bounded encrypted
named stream** with credit backpressure, per-import session generation, deadline,
and cancel; they are visible only to the trusted authority relay and the exact
child frontend — intermediate controllers see ciphertext only. Only bounded audit
metadata (Zone, lease state, ceremony outcome) is recorded, never CTAP payload
bytes. The export **serializes CTAP ceremonies** across Zones (one exclusive
per-device lease, fair queue, deadline/cancel). The Provider's signed
export/import adapter enforces the lease/fair-queue/deadline/cancel and builds the
local `Device` projection; **core** owns `ResourceExport`/`ResourceImport`
routing and base lifecycle. Export removal or ZoneLink loss revokes the lease and
degrades the local projection; reconnect revalidates generation/fingerprint; a
D091 upgrade drains the consumer before recycling the relay authority.

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

Security-key Device resources in a Zone are declared under
`d2b.zones.<zone>.resources`. The Provider must be installed in the Zone before
any Device using it can be admitted.

```nix
# Install the Provider
d2b.zones.dev.resources."sk-provider" = {
  type = "Provider";
  spec = {
    artifactId = "device-security-key";     # selects the signed package
    config = {
      devices = [
        {
          label     = "yubikey-primary";
          vendorId  = 4176;                 # 0x1050 — Yubico
          productId = 1031;                 # 0x0407 — YubiKey 5
          serial    = null;
        }
      ];
      sessionRingSize  = 32;
      leaseTimeoutSecs = 300;
    };
  };
};

# Declare the Device
d2b.zones.dev.resources."corp-vm-sk" = {
  type = "Device";
  metadata.ownerRef = "Guest/corp-vm";
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
  };
};
```

**Eval-time invariants:**

1. `inventory.selector.label` must match exactly one `config.devices[].label` in the
   same Zone's installed `Provider/device-security-key`.
2. `busClass` must be `"hidraw"` — any other value is an assertion error.
3. `arbitration` must be `"exclusive"` and `maxConcurrentClaims` must be 1 (or absent).
4. If a `device-usbip` Device resource in the same Zone has an `inventory.selector`
   with matching `vendorId` and `productId` (and matching serial if non-null),
   activation fails with a hard assertion error.
5. `providerRef` must resolve to an installed `Provider/device-security-key`
   resource in the same Zone.
6. No prohibited field (`hidrawPath`, raw sysfs path) may appear anywhere in the
   Device spec. Unknown fields are rejected.

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

- `boot.kernelModules = ["uhid"]` — required for the UHID kernel interface to
  exist at all; must be loaded before Core opens `/dev/uhid` for the frontend's
  LaunchTicket DeviceGrant.
- The static `d2b-sk-frontend` binary in the Guest store closure.

**Removed from v3:** `services.udev.extraRules` and
`users.users.<workload-user>.extraGroups = ["plugdev"]` are no longer needed.
Core pre-opens `/dev/uhid` with masked `/dev` and passes only the fd; the
frontend has no ambient device path access, so no udev rule or group is required.

**v3 change:** The `d2b-sk-frontend.service` systemd unit declared in
`security-key-guest.nix` is removed when the Device controller's Process
resource (`device-<uid-short>-sk-frontend`) is live. The Process controller
manages the frontend lifecycle. The Nix module removes the unit declaration
behind a `d2b.securityKey._legacySystemdUnit = false` option gate, defaulting
to false once the Provider is installed.

## Work items

All items are New (not yet implemented) unless marked with the baseline evidence
class.

### Reuse from baseline

### W-R01

| Field | Value |
| --- | --- |
| Dependency/owner | ADR-046 provider-device-security-key session/relay owner; depends on ADR-046-components-processes-and-sandbox, ADR-046-componentsession-and-bus, and W-N03/W-N04/W-N05 relay/session/CID implementation. |
| Current source | `packages/d2bd/src/security_key.rs` — `SecurityKeyState`, `LeaseState`, `LeaseId`, `CidTranslator`, `try_acquire_lease`, `release_lease`, `CEREMONY_TIMEOUT`, `QUEUE_WAIT_TIMEOUT` (implemented-and-reachable) |
| Reuse action | extract and adapt |
| Destination | Move to `packages/d2b-provider-device-security-key/src/session.rs` and `cid.rs`; adapt to Provider Process model (remove daemon Mutex wrapping, add async relay protocol) |
| Detailed design | Extract the baseline lease/session constants and CID mapping into provider-local modules. Preserve `CEREMONY_TIMEOUT` and `QUEUE_WAIT_TIMEOUT`, remove daemon-global `Mutex` ownership, model the relay as an async Provider-owned Process, keep DeviceGrant/OFD lease ownership for the relay lifetime, and expose session transitions to the controller through the typed relay-control protocol. |
| Integration | Host relay Process (W-N03) owns `session.rs` and `cid.rs`; Device controller (W-N02) consumes relay lifecycle events over `device-security-key.relay-ctrl.v1`; Device status and audit rows consume bounded session-ring summaries; ComponentSession named stream carries CTAPHID bytes. |
| Data migration | Full d2b 3.0 reset; no v2 state/config import |
| Validation | `session_state_machine.rs`, `session_ring.rs`, `cancel_propagation.rs`, `session_timeout.rs`, and `cid_isolation.rs` verify Idle/Active/Completed/TimedOut transitions, ring eviction, cancel/timeout behavior, and per-session CID isolation with no daemon-global lease state. |
| Removal proof | W-X01 deletes the superseded daemon-internal `packages/d2bd/src/security_key.rs` `SecurityKeyState`, `LeaseState`, `SkRegistry`, and accept-loop ownership after the provider relay/session tests pass. |

### W-R02

| Field | Value |
| --- | --- |
| Dependency/owner | ADR-046 provider-device-security-key relay owner; depends on W-N03 relay Process, W-N07 descriptor validation, and W-N17 transport-vsock integration. |
| Current source | `packages/d2bd/src/security_key.rs` — CTAPHID relay loop, `SkAcceptHandle`, `relay_one_ceremony` (implemented-and-reachable) |
| Reuse action | extract and adapt |
| Destination | Move to `packages/d2b-provider-device-security-key/src/relay.rs`; replace daemon-internal Unix socket proxy with vsock framing |
| Detailed design | Extract the CTAPHID ceremony relay behavior into the provider relay binary. Preserve one-ceremony-at-a-time proxy semantics and CTAPHID cancel handling, but replace daemon-internal Unix socket proxying with the `d2b.security-key.v3` ComponentSession over the owned CTAPHID Endpoint and named `ctaphid` stream. |
| Integration | Core launches the relay Process with a LaunchTicket DeviceGrant and Endpoint attachment; transport-vsock resolves `Endpoint/<device-uid>-sk-ctaphid-relay`; frontend Process connects as ComponentSession initiator; controller receives session events over the manifest-declared internal channel. |
| Data migration | Full d2b 3.0 reset; no v2 state/config import |
| Validation | `host_relay_guest_frontend/` integration fixture, `device_grant_no_path.rs`, `descriptor_validation.rs`, and `cancel_propagation.rs` prove relay fd injection, ComponentSession transport, cancel propagation, and absence of daemon-internal socket proxying. |
| Removal proof | W-X01 and W-X02 remove `start_sk_accept_loop`, `SkAcceptHandle`, `relay_one_ceremony`, and the daemon-internal Unix socket proxy bind from `packages/d2bd/src/security_key.rs` and `packages/d2bd/src/lib.rs`. |

### W-R03

| Field | Value |
| --- | --- |
| Dependency/owner | ADR-046 provider-device-security-key frontend owner; depends on W-N03 relay service, W-N07 ComponentSession validation, W-N13 guest Nix migration gate, W-N17 transport-vsock, and W-N19 virtual frontend Device lifecycle. |
| Current source | `packages/d2b-sk-frontend/src/` — `main.rs`, `uhid.rs` (implemented-and-reachable); `framing.rs` and `vsock.rs` are obsolete under v3 (replaced by ComponentSession transport) |
| Reuse action | extract and adapt |
| Destination | Adopt `main.rs` and `uhid.rs` as the v3 Process binary entry point; replace `framing.rs`/`vsock.rs` with ComponentSession client from `d2b-session-unix/src/vsock.rs`; wire as Process service in Provider crate |
| Detailed design | Retain the UHID creation and frontend binary entry behavior from `main.rs` and `uhid.rs`, but run it as a v3 user-domain Process that receives a pre-opened `/dev/uhid` fd from the virtual Device LaunchTicket. Delete the raw frame/vsock protocol and use the ComponentSession client and named `ctaphid` stream for relay communication. |
| Integration | Device controller creates the frontend Process and virtual `Device/<device-name>-frontend`; Core pre-opens UHID for the frontend LaunchTicket; frontend consumes the CTAPHID Endpoint via transport-vsock and reports readiness through Process status. |
| Data migration | Full d2b 3.0 reset; no frontend session state import |
| Validation | `host_relay_guest_frontend/`, `device_grant_no_path.rs`, `descriptor_validation.rs`, and guest Nix migration tests prove UHID fd injection, no `/dev/uhid` path, ComponentSession client use, and no raw `framing.rs`/`vsock.rs` protocol. |
| Removal proof | W-X03 removes the legacy `d2b-sk-frontend.service` unit declaration, and the v3 frontend excludes the obsolete `packages/d2b-sk-frontend/src/framing.rs` and `vsock.rs` raw transport behavior. |

### W-R04

| Field | Value |
| --- | --- |
| Dependency/owner | Core LaunchTicket/privileged broker device-grant owner; depends on ADR-046-resources-device, W-N06 probe/device-token population, and W-N11 broker op update. |
| Current source | `packages/d2b-priv-broker/src/ops/security_key.rs` — `live_open_hidraw_security_key`, FIDO usage page revalidation, group validation, `ALLOWED_GROUPS` (implemented-and-reachable) |
| Reuse action | adapt |
| Destination | Preserve revalidation logic; update `SecurityKeyOpenDevice` to use bundle device table `device_token` as sole open target (no iterative sysfs scan); add zone-field handling; remove sysfs fallback. **Core's LaunchTicket calls this internally; the Provider does not call it.** |
| Detailed design | Keep the FIDO usage-page and post-open revalidation logic, but make the trusted private bundle `device_token` the only open target. Add Zone-aware request handling, reject path/sysfs fallback inputs, and keep the operation internal to Core LaunchTicket DeviceGrant resolution rather than callable by the Provider controller. |
| Integration | Provider activation records label-to-`device_token` mappings; Core LaunchTicket resolves `deviceUsage` for the relay Process; broker opens and revalidates the hidraw fd; relay receives only an inherited fd; Core emits path-free `device-grant` audit. |
| Data migration | Full d2b 3.0 reset; no v2 device state import |
| Validation | `packages/d2b-priv-broker/tests/security_key_broker.rs` updates for bundle table lookup and zone-field round trip; `device_grant_no_path.rs` proves Provider code does not call the broker and sees no device path; audit tests prove path-free grant records. |
| Removal proof | The superseded iterative sysfs scan/fallback behavior in `packages/d2b-priv-broker/src/ops/security_key.rs` is removed once bundle-token lookup and revalidation tests pass. |

### W-R05

| Field | Value |
| --- | --- |
| Dependency/owner | `d2b-contracts` security-key DTO owner; depends on ADR-046-resource-object-model, ADR-046-resources-device, W-N10 provider descriptor, and W-X06 udev-op removal. |
| Current source | `packages/d2b-contracts/src/security_key.rs` — `SecurityKeySessionId`, `SecurityKeyDeviceLabel`, `SecurityKeySession`, `SecurityKeySessionResult`, `SecurityKeyStatusResponse`, `SecurityKeySessionsResponse`, `SecurityKeyOpenDeviceRequest`, `SecurityKeyEvent` (implemented-and-reachable) |
| Reuse action | adapt |
| Destination | Adapt to v3 Zone/ResourceRef identifiers; preserve serde shapes for zero downstream breakage where possible; remove `SecurityKeyApplyUdevRulesRequest` (W-X06) |
| Detailed design | Rebase the security-key wire DTOs onto v3 Zone and ResourceRef identifiers while preserving compatible serde shape where the semantics remain unchanged. Keep opaque session/device labels and bounded events, add the `zone` field to `SecurityKeyOpenDeviceRequest`, and drop the udev-rules request because UHID is pre-opened through DeviceGrant. |
| Integration | Core LaunchTicket, broker security-key open op, Device controller status/audit, CLI status/session readers, and provider tests consume the v3 DTOs from `d2b-contracts`. |
| Data migration | Full d2b 3.0 reset; no v2 DTO compatibility migration beyond serde-shape preservation where possible |
| Validation | DTO serde round trips, unknown-field denial, zone-field round trip, path-redaction tests, and updated `usb_sk_contract.rs` assertions in the provider crate. |
| Removal proof | W-X06 removes `SecurityKeyApplyUdevRulesRequest`, the `SecurityKeyApplyUdevRules` broker op, and related broker code after UHID DeviceGrant coverage is live. |

### W-R06

| Field | Value |
| --- | --- |
| Dependency/owner | Provider crate test owner; depends on W-R05 v3 DTOs and W-N01 provider crate layout. |
| Current source | `packages/d2b-contract-tests/tests/usb_sk_contract.rs` — DTO serde round-trips, unknown-field denial, broker capability set (implemented-and-reachable) |
| Reuse action | move and adapt |
| Destination | Move to `packages/d2b-provider-device-security-key/tests/`; update imports and v3 type names |
| Detailed design | Move the reusable semantic assertions for security-key DTO serde, unknown-field denial, and broker capability shape into the provider crate's hermetic `tests/` suite, updating imports and names to the v3 contract modules without weakening assertions. |
| Integration | `cargo test -p d2b-provider-device-security-key --lib --tests` runs the moved contract tests with the provider's DTO/controller test matrix; old contract-test manifests point to the successor coverage before deletion. |
| Data migration | None — test-only move; no runtime state |
| Validation | Moved tests pass under the provider crate; contract assertions are retained; D094 disposition records moved/adapted coverage before old duplicate tests are deleted. |
| Removal proof | W-X04 deletes `packages/d2b-contract-tests/tests/usb_sk_contract.rs` only after the provider-crate successor test covers all prior assertions. |

### W-R07

| Field | Value |
| --- | --- |
| Dependency/owner | Provider crate test/minijail owner; depends on W-N08 minijail profiles, W-N09 Process templates, and W-N01 provider crate layout. |
| Current source | `packages/d2b-contract-tests/tests/minijail_sk_frontend.rs` — minijail profile shape, `ProcessRole::SecurityKeyFrontend` (implemented-and-reachable) |
| Reuse action | move and adapt |
| Destination | Move to `packages/d2b-provider-device-security-key/tests/`; update for v3 Process resource minijail profile; retain zero-capabilities assertion |
| Detailed design | Move the reusable minijail/sandbox assertions into the provider crate and retarget them from `ProcessRole::SecurityKeyFrontend` to the v3 Process resource templates and relay/controller minijail profiles. Preserve zero-capabilities and seccomp-class assertions while recognizing the frontend uses `Provider/system-systemd` hardening rather than a minijail profile. |
| Integration | Provider tests validate Nix minijail profile entries, Process resource sandbox templates, and system-minijail/system-systemd conformance expectations before old contract tests are retired. |
| Data migration | None — test-only move; no runtime state |
| Validation | Provider-crate tests retain zero-capability and seccomp assertions for relay/controller and assert no minijail profile is used for the frontend Process. |
| Removal proof | W-X04 deletes `packages/d2b-contract-tests/tests/minijail_sk_frontend.rs` only after the provider-crate successor test covers all prior assertions. |

### New items

### W-N01

| Field | Value |
| --- | --- |
| Dependency/owner | ADR-046 provider-device-security-key crate owner; depends on provider-model/package workspace policy. |
| Current source | None — net-new v3 work; no pre-ADR45 baseline equivalent |
| Reuse action | net-new |
| Destination | New crate `packages/d2b-provider-device-security-key/` with `src/`, `tests/`, `integration/`, `README.md` (workspace policy requires all four) |
| Detailed design | New crate `packages/d2b-provider-device-security-key/` with `src/`, `tests/`, `integration/`, `README.md` (workspace policy requires all four) |
| Integration | Workspace membership and provider package descriptor expose the crate to Core ProviderDeployment; W-N02 through W-N20 add controller, relay, frontend, descriptor, tests, and README content under this crate. |
| Data migration | Full d2b 3.0 reset; no v2 state/config import |
| Validation | Workspace package-policy check rejects missing `src/`, `tests/`, `integration/`, or `README.md`; `cargo test -p d2b-provider-device-security-key --lib --tests` discovers the hermetic suite; README acceptance criteria from the provider crate standard layout are satisfied. |
| Removal proof | None — net-new; no prior owner to remove |

### W-N02

| Field | Value |
| --- | --- |
| Dependency/owner | Device controller owner for `Provider/device-security-key`; depends on W-N01 crate layout, W-N06 probe port, W-N09 templates, W-N18 effect port, and ADR-046-resource-reconciliation. |
| Current source | None — net-new v3 work; no pre-ADR45 baseline equivalent |
| Reuse action | net-new |
| Destination | `packages/d2b-provider-device-security-key/src/controller.rs` |
| Detailed design | Device controller: `controller.rs` implementing standard reconcile interface (`spec-generation-changed`, `deletion-requested`, `dependency-changed`, `scheduled-observe`, `owned-resource-changed`) |
| Integration | Zone ResourceClient watches Device resources for this Provider, creates relay/frontend Process and virtual frontend Device resources, writes Device status/finalizers, and drives relay-control messages through `device-security-key.relay-ctrl.v1`. |
| Data migration | Full d2b 3.0 reset; no v2 state/config import |
| Validation | `controller_reconcile.rs`, `mutual_exclusion.rs`, `status_state.rs`, and deletion/finalizer tests cover all reconcile triggers, status writes, virtual Device lifecycle, and absence of Volume API calls. |
| Removal proof | None — net-new; no prior owner to remove |

### W-N03

| Field | Value |
| --- | --- |
| Dependency/owner | Relay Process owner for `Provider/device-security-key`; depends on W-R01/W-R02/W-N04/W-N05/W-N07/W-N17. |
| Current source | None — net-new v3 work; no pre-ADR45 baseline equivalent |
| Reuse action | net-new |
| Destination | `packages/d2b-provider-device-security-key/src/relay.rs` |
| Detailed design | Relay Process entry point: `relay.rs` — async ComponentSession accept loop (`d2b.security-key.v3` responder over the owned `Endpoint/<device-uid>-sk-ctaphid-relay` resource), CID translation, hidraw fd received from LaunchTicket DeviceGrant (not broker), CTAPHID proxy over named CTAPHID stream; manifest-declared internal service `device-security-key.relay-ctrl.v1` connected via LaunchTicket internal channel FD for controller messaging |
| Integration | Core LaunchTicket injects hidraw fd, CTAPHID Endpoint attachment, and internal controller channel; frontend connects over ComponentSession; controller consumes session events and sends `CancelSession`; Core releases DeviceGrant on relay exit. |
| Data migration | Full d2b 3.0 reset; no relay session state import |
| Validation | `host_relay_guest_frontend/`, `claim_conflict/`, `device_grant_no_path.rs`, `descriptor_validation.rs`, `cancel_propagation.rs`, and `cid_isolation.rs` prove relay launch, one-session policy, fd-only device access, internal service validation, cancel, and CID translation. |
| Removal proof | Supersedes daemon-internal relay behavior removed by W-X01/W-X02 after relay Process tests pass. |

### W-N04

| Field | Value |
| --- | --- |
| Dependency/owner | Relay session-state owner; depends on W-R01 and W-N03. |
| Current source | None — net-new v3 work; no pre-ADR45 baseline equivalent |
| Reuse action | net-new |
| Destination | `packages/d2b-provider-device-security-key/src/session.rs` |
| Detailed design | Session state machine: `session.rs` — `SessionStateMachine` (Idle/Active/Completed/TimedOut; no AwaitingLease), session ring, session ID allocation, ring eviction; DeviceGrant held for relay lifetime; `CancelSession(sessionId)` from controller terminates Active ceremony |
| Integration | Relay uses the state machine for ComponentSession connections; controller receives lifecycle messages; Device status consumes bounded session ring observations; audit consumes timeout/cancel/release outcomes. |
| Data migration | Full d2b 3.0 reset; no session ring import |
| Validation | `session_state_machine.rs`, `session_ring.rs`, `session_timeout.rs`, and `cancel_propagation.rs` cover transitions, eviction, timeout, cancel, and the absence of `AwaitingLease`. |
| Removal proof | None — net-new; no prior owner to remove |

### W-N05

| Field | Value |
| --- | --- |
| Dependency/owner | Relay CID-translation owner; depends on W-R01 and W-N03. |
| Current source | None — net-new v3 work; no pre-ADR45 baseline equivalent |
| Reuse action | net-new |
| Destination | `packages/d2b-provider-device-security-key/src/cid.rs` |
| Detailed design | CID translator: `cid.rs` — per-session u32→u64 host-CID allocation, bimap, eviction on session end |
| Integration | Relay rewrites frontend CTAPHID CIDs before sending to hidraw fd and reverses responses before writing the ComponentSession named stream; session teardown drops the map. |
| Data migration | Full d2b 3.0 reset; CID maps are transient and not imported |
| Validation | `cid_isolation.rs` verifies per-session allocation, round trip, no sharing across relays, and eviction on session end. |
| Removal proof | None — net-new; no prior owner to remove |

### W-N06

| Field | Value |
| --- | --- |
| Dependency/owner | Probe/effect-port and activation owner; depends on W-N18 effect port and Core private bundle device table support. |
| Current source | None — net-new v3 work; no pre-ADR45 baseline equivalent |
| Reuse action | net-new |
| Destination | `packages/d2b-provider-device-security-key/src/probe.rs`; Provider activation/Core private bundle device table population for label → `device_token` |
| Detailed design | hidraw probe: `probe.rs` — calls `SecurityKeyEffectPort::observe_inventory(&device_id, &policy_id)` with opaque types injected by Core; interprets `InventoryObservation`; never reads `/sys/class/hidraw/` directly; bundle device table population at activation time (Provider activation resolves label → `device_token` via Core; stored in private bundle) |
| Integration | Controller scheduled-observe invokes `probe.rs`; Core adapter implements `SecurityKeyEffectPort`; Nix activation emits private label-to-token bundle entries; Device status receives `DevicePresent` and phase updates. |
| Data migration | Full d2b 3.0 reset; no v2 probe state import |
| Validation | `controller_reconcile.rs` scheduled-observe tests, `descriptor_validation.rs` Debug-redaction capture, and path-safety tests prove Provider never reads sysfs and receives only opaque observations. |
| Removal proof | Supersedes provider-side or broker fallback sysfs scanning; W-R04/W-N11 removal proof verifies only bundle `device_token` lookup remains. |

### W-N07

| Field | Value |
| --- | --- |
| Dependency/owner | ComponentSession/security descriptor owner; depends on ADR-046-componentsession-and-bus, W-N03 relay, W-N17 transport-vsock, and W-N18 effect-port redaction types. |
| Current source | None — net-new v3 work; no pre-ADR45 baseline equivalent |
| Reuse action | net-new |
| Destination | `packages/d2b-provider-device-security-key/src/descriptor.rs` |
| Detailed design | ComponentSession descriptor validation: `descriptor.rs` — manifest-declared relay↔controller service `device-security-key.relay-ctrl.v1` Noise NN handshake; socket FD injected via LaunchTicket internal channel (no ambient path); relay↔frontend `d2b.security-key.v3` Noise KK enrolled-key registration and session authority enforcement; relay uses canonical authenticated subject from ComponentSession, never raw vsock CID |
| Integration | Provider descriptor declares services and fingerprints; LaunchTicket injects internal channel and Endpoint transport; relay/controller/frontend validate descriptors and peer authority before exchanging messages. |
| Data migration | Full d2b 3.0 reset; no v2 transport/session state import |
| Validation | `descriptor_validation.rs` covers wrong service, wrong descriptor digest, wrong SO_PEERCRED uid, unenrolled key, oversized records, no ambient path, and redacted opaque IDs. |
| Removal proof | None — net-new; no prior owner to remove |

### W-N08

| Field | Value |
| --- | --- |
| Dependency/owner | Sandbox/minijail owner; depends on W-N09 Process templates and ADR-046-components-processes-and-sandbox. |
| Current source | None — net-new v3 work; no pre-ADR45 baseline equivalent |
| Reuse action | net-new |
| Destination | `nixos-modules/minijail-profiles.nix` entries for relay and controller; provider descriptor sandbox templates for relay/controller/frontend |
| Detailed design | Minijail profiles for relay and controller only; frontend uses `Provider/system-systemd` hardening directives compiled from `SandboxSpec` (no minijail profile for frontend). Add relay and controller entries to `nixos-modules/minijail-profiles.nix`; `capabilityClasses: []`; `seccompClass: sk-relay` and `seccompClass: sk-controller` |
| Integration | Nix minijail profiles feed system-minijail Process launches for controller/relay; frontend Process template feeds system-systemd hardening; provider tests assert the split. |
| Data migration | Full d2b 3.0 reset; no sandbox state import |
| Validation | `minijail_sk_frontend` successor tests, sandbox template tests, and zero-capability/seccomp assertions cover relay/controller minijail profiles and no frontend minijail profile. |
| Removal proof | Supersedes `ProcessRole::SecurityKeyFrontend`-centric minijail test ownership removed by W-X04/W-X05 after Process-resource coverage passes. |

### W-N09

| Field | Value |
| --- | --- |
| Dependency/owner | Provider descriptor/process-template owner; depends on W-N01 crate, W-N08 sandbox profiles, W-N17 Endpoint transport, and W-N19 virtual frontend Device. |
| Current source | None — net-new v3 work; no pre-ADR45 baseline equivalent |
| Reuse action | net-new |
| Destination | Provider descriptor Process templates and owned CTAPHID `Endpoint` template for `Provider/device-security-key` |
| Detailed design | Process resource templates in Provider descriptor: controller template (`Host/host-system`, `system`, `controller`, `environmentClass: provider-defined`), relay template (`Host/host-system`, `system`, `service`, `environmentClass: provider-defined`), frontend template (`Guest/<vm>`, `user`, `service`, `environmentClass: provider-defined`, `userRef` required), and the owned CTAPHID relay Endpoint resource |
| Integration | Core ProviderDeployment creates controller; Device controller creates relay/frontend Process and Endpoint resources from templates; Process Providers launch them through system-minijail/system-systemd; frontend consumes Endpoint. |
| Data migration | Full d2b 3.0 reset; no v2 processes.json import |
| Validation | `controller_reconcile.rs`, Process template golden tests, Endpoint resource tests, and frontend `userRef` admission tests prove templates and Endpoint shape. |
| Removal proof | Supersedes the legacy readiness-only `ProcessRole::SecurityKeyFrontend` tracking node removed by W-X05 after v3 Process resources are live. |

### W-N10

| Field | Value |
| --- | --- |
| Dependency/owner | Provider package descriptor owner; depends on W-N01 crate, W-N09 templates, W-N20 state contract, and ADR-046-provider-model-and-packaging. |
| Current source | None — net-new v3 work; no pre-ADR45 baseline equivalent |
| Reuse action | net-new |
| Destination | Signed Provider descriptor JSON for `Provider/device-security-key` in the provider package |
| Detailed design | Provider descriptor JSON (signed): identity, config schema, exported ResourceType (Device/hidraw), controller/relay/frontend component descriptors, `d2b.security-key.v3` service declaration, D087 status-first state declaration with an empty ProviderStateSet, permission claims |
| Integration | Core ProviderDeployment verifies the signed descriptor, installs ResourceApiBinding and component descriptors, exposes service fingerprints to ComponentSession validation, and supplies permission claims/RBAC bindings. |
| Data migration | Full d2b 3.0 reset; no provider descriptor import |
| Validation | Descriptor schema validation, signature/fingerprint tests, service inventory tests, permission claim tests, empty ProviderStateSet tests, and README/provider package conformance checks. |
| Removal proof | None — net-new; no prior owner to remove |

### W-N11

| Field | Value |
| --- | --- |
| Dependency/owner | Core LaunchTicket/broker owner; depends on W-R04, W-R05, W-N06, and ADR-046-resources-device. |
| Current source | None — net-new v3 work; no pre-ADR45 baseline equivalent |
| Reuse action | net-new |
| Destination | v3 `SecurityKeyOpenDevice` broker op and Core LaunchTicket DeviceGrant resolution path |
| Detailed design | v3 `SecurityKeyOpenDevice` broker op update: add `zone` field; implement bundle device table `device_token` lookup as sole open path; remove iterative sysfs scan from broker; add post-open revalidation steps (fstat, HIDIOCGRAWINFO, HIDIOCGRDESC). This is an internal Core operation called by LaunchTicket; the Provider controller does not call it. |
| Integration | Device controller declares relay `deviceUsage`; Core LaunchTicket resolves DeviceGrant through the private bundle table; broker returns an fd to Core; relay receives the inherited fd; audit consumes the path-free grant outcome. |
| Data migration | Full d2b 3.0 reset; no v2 broker state import |
| Validation | Broker unit tests for zone field and token lookup, path-rejection tests, post-open revalidation tests, and provider tests proving no Provider broker call or sysfs path. |
| Removal proof | Superseded broker iterative sysfs scan behavior is removed; tests prove only bundle `device_token` lookup is accepted for `SecurityKeyOpenDevice`. |

### W-N20

| Field | Value |
| --- | --- |
| Dependency/owner | Provider state/status owner; depends on ADR-046-provider-state, W-N10 descriptor, and W-N09 Process templates. |
| Current source | None — net-new v3 work; no pre-ADR45 baseline equivalent |
| Reuse action | net-new |
| Destination | Provider descriptor state declaration, controller/status logic, Process templates, and Nix principal provisioning for `Provider/device-security-key` |
| Detailed design | Status-first Provider state contract: the signed Provider descriptor declares no Provider state Volume for controller, relay, or frontend; ProviderStateSet is empty. Device resources and the Core Operation ledger are the operational authority. Controller/relay/frontend Process templates have no `/state` mount and no dedicated state-layout principals. Nix pre-provisions only principals required for genuine Process placement and DeviceGrant access, not state Volume ownership. |
| Integration | Provider descriptor advertises an empty ProviderStateSet; controller writes bounded observations to Device status and Operation ledger; Nix principal provisioning feeds Process placement and DeviceGrant access only; Volume controllers see no provider state Volume requests. |
| Data migration | Full d2b 3.0 reset; no v2 state/config import |
| Validation | `status_state.rs` proves empty ProviderStateSet, no `/state` mounts, no Volume API calls, and no CTAP/fd/session secrets in status/log/audit/metrics. |
| Removal proof | None — net-new; no prior owner to remove |

### W-N12

| Field | Value |
| --- | --- |
| Dependency/owner | Nix resource compiler owner; depends on W-N10 provider descriptor/config schema and ADR-046-nix-configuration. |
| Current source | None — net-new v3 work; no pre-ADR45 baseline equivalent |
| Reuse action | net-new |
| Destination | `nixos-modules/` v3 resource compiler/eval assertions for `Provider/device-security-key` Device resources |
| Detailed design | Nix resource compilation: Device spec validation in `nixos-modules/`, eval-time mutual-exclusion assertion, label resolution against Provider config, prohibited-field rejection |
| Integration | Nix authoring under `d2b.zones.<zone>.resources` emits Device resources; eval assertions block invalid labels, bus classes, raw paths, and USBIP conflicts; zone bundle feeds Provider controller admission. |
| Data migration | Full d2b 3.0 reset; current Nix options migrate to v3 Zone resources without state import |
| Validation | Nix eval tests for label resolution, `busClass=hidraw`, exclusive arbitration, USBIP mutual exclusion, prohibited fields, and providerRef resolution. |
| Removal proof | Supersedes current option shape only after v3 Zone resource option parity; legacy security-key/USBIP mutual-exclusion assertion is replaced by v3 resource assertion coverage. |

### W-N13

| Field | Value |
| --- | --- |
| Dependency/owner | Guest Nix module migration owner; depends on W-N03 frontend Process, W-N09 Process templates, and W-N19 virtual frontend Device. |
| Current source | None — net-new v3 work; no pre-ADR45 baseline equivalent |
| Reuse action | net-new |
| Destination | `nixos-modules/components/security-key-guest.nix` migration gate `d2b.securityKey._legacySystemdUnit` |
| Detailed design | Guest Nix module migration gate: `d2b.securityKey._legacySystemdUnit` option, defaulting to false when Provider is installed; remove `d2b-sk-frontend.service` unit |
| Integration | Guest Nix keeps `uhid` kernel module and static frontend binary in the closure while Process controller owns frontend lifecycle; option gate disables the legacy unit when Provider/device-security-key is installed. |
| Data migration | Full d2b 3.0 reset; no legacy frontend unit state import |
| Validation | Nix eval tests show the legacy unit is absent by default with Provider installed, can be gated only during transition if required, and `uhid` module/binary wiring remains present. |
| Removal proof | W-X03 deletes the superseded `nixos-modules/components/security-key-guest.nix` `d2b-sk-frontend.service` declaration after the gate defaults to false. |

### W-N14

| Field | Value |
| --- | --- |
| Dependency/owner | Audit owner for Core device-grant and Device controller lifecycle; depends on W-N02 controller, W-N11 DeviceGrant open, and ADR-046-telemetry-audit-and-support. |
| Current source | None — net-new v3 work; no pre-ADR45 baseline equivalent |
| Reuse action | net-new |
| Destination | Core audit emission for `device-grant` and Device controller audit emission for `device-session`/lease lifecycle events |
| Detailed design | Audit record emission: bounded path-free `device-grant` records from Core at DeviceGrant resolution time; `device-session` lifecycle events from Device controller; neither block carries device path, guest name, session content, or CTAP bytes |
| Integration | Core LaunchTicket emits grant audit; Device controller emits lifecycle audit; Zone audit stream stores bounded records; CLI/support tooling consumes digests and stable outcomes. |
| Data migration | Full d2b 3.0 reset; no v2 audit import |
| Validation | Audit tests assert path-free fields, bounded digests, no guest name/session content/CTAP bytes, grant emitted by Core not Provider controller, and lifecycle emitted by controller. |
| Removal proof | None — net-new; no prior owner to remove |

### W-N15

| Field | Value |
| --- | --- |
| Dependency/owner | Observability owner; depends on W-N03 relay, W-N02 controller, and ADR-046-telemetry-audit-and-support. |
| Current source | None — net-new v3 work; no pre-ADR45 baseline equivalent |
| Reuse action | net-new |
| Destination | Provider/controller bounded telemetry emitter and observability-otel handoff for security-key metrics |
| Detailed design | OTEL metrics: `d2b_device_sk_session_total`, `d2b_device_sk_ceremony_duration_seconds`, `d2b_device_sk_relay_restarts_total` via bounded emitter ring |
| Integration | Relay/controller write metric events to the bounded ring; observability-otel Provider drains and exports; dashboards/CLI consume closed labels and bounded histograms. |
| Data migration | Full d2b 3.0 reset; no v2 telemetry import |
| Validation | Metrics tests assert closed label sets, no device/session/guest/path labels, bounded ring behavior, and correct session/ceremony/restart counters. |
| Removal proof | None — net-new; no prior owner to remove |

### W-N16

| Field | Value |
| --- | --- |
| Dependency/owner | Provider documentation owner; depends on W-N01 through W-N15 and W-N17 through W-N20 for complete crate behavior. |
| Current source | None — net-new v3 work; no pre-ADR45 baseline equivalent |
| Reuse action | net-new |
| Destination | `packages/d2b-provider-device-security-key/README.md` |
| Detailed design | `README.md` for the crate: Provider identity, root config schema, Device spec, process model, RBAC, security invariants, state/telemetry, build/test/integration commands, standalone-repository consumption |
| Integration | Workspace/package policy and provider crate acceptance use the README as the human entry point; docs link to it for provider-local build/test/integration commands. |
| Data migration | None — docs/tooling only; no runtime state |
| Validation | README presence check from provider crate standard layout; documentation review verifies every listed section and command is present and matches the crate/package behavior. |
| Removal proof | None — net-new; no prior owner to remove |

### W-N17

| Field | Value |
| --- | --- |
| Dependency/owner | transport-vsock/ComponentSession integration owner; depends on W-N03 relay, W-N07 descriptor validation, and ADR-046-componentsession-and-bus. |
| Current source | None — net-new v3 work; no pre-ADR45 baseline equivalent |
| Reuse action | net-new |
| Destination | `Provider/transport-vsock` Endpoint resolution and LaunchTicket attachment integration for security-key relay/frontend |
| Detailed design | `Provider/transport-vsock` integration: resolve the owned CTAPHID relay Endpoint into opaque LaunchTicket transport attachments for the relay and frontend; enroll Noise KK static keys for relay/frontend pair before first connection |
| Integration | Endpoint resource is produced by relay template/controller; transport-vsock resolves opaque attachments; Core injects attachments and enrolled keys into relay/frontend LaunchTickets; ComponentSession establishes `d2b.security-key.v3`. |
| Data migration | Full d2b 3.0 reset; no v2 transport state import |
| Validation | `host_relay_guest_frontend/` and `descriptor_validation.rs` verify Endpoint resolution, Noise KK enrollment, attachment opacity, and no raw vsock CID/port in status/spec. |
| Removal proof | Supersedes baseline `vsock.sock_14320` raw port usage; tests prove no `vsockPort` or raw AF_VSOCK framing remains for security-key transport. |

### W-N18

| Field | Value |
| --- | --- |
| Dependency/owner | `d2b-contracts` neutral effect-port owner and Core adapter owner; depends on ADR-046-resources-device and W-N06 probe behavior. |
| Current source | None — net-new v3 work; no pre-ADR45 baseline equivalent |
| Reuse action | net-new |
| Destination | `d2b-contracts` neutral `SecurityKeyEffectPort` trait/types; `packages/d2b-provider-device-security-key/src/effect_port.rs` re-export; Core adapter implementation in `d2b-provider` or `d2b-provider-toolkit` |
| Detailed design | `SecurityKeyEffectPort` trait and associated opaque types (`DeviceId`, `ObservationPolicyId`) defined in `d2b-contracts` (neutral contract crate); both types have custom `Debug` impls that redact content; `effect_port.rs` in the Provider crate re-exports from `d2b-contracts`; Core adapter implementation in `d2b-provider` or `d2b-provider-toolkit` crate; inject into Device controller at startup with concrete `DeviceId` and `ObservationPolicyId` per Device; relay does NOT use the port |
| Integration | Core resolves Zone/label to opaque IDs and injects the port into the controller; controller scheduled-observe calls the trait; Provider crate depends only on the neutral contract/re-export; relay path is unaffected. |
| Data migration | Full d2b 3.0 reset; no v2 effect-port state import |
| Validation | Unit tests assert Debug redaction, controller calls `observe_inventory` with injected IDs, relay has no port dependency, and fake Core adapter returns bounded `InventoryObservation`. |
| Removal proof | None — net-new; no prior owner to remove |

### W-N19

| Field | Value |
| --- | --- |
| Dependency/owner | Virtual frontend Device lifecycle owner; depends on W-N02 controller, W-N03 frontend/relay, W-N11 DeviceGrant, and ADR-046-resources-device. |
| Current source | None — net-new v3 work; no pre-ADR45 baseline equivalent |
| Reuse action | net-new |
| Destination | Device controller virtual `Device/<device-name>-frontend` lifecycle and Core frontend LaunchTicket UHID DeviceGrant resolution |
| Detailed design | Virtual frontend Device lifecycle: controller creates `Device/<device-name>-frontend` (`deviceClass: virtual`, `busClass: uhid`, `ownerRef: Device/<device-name>`, `settings.bindGuest`) on claim; updates `bindGuest` on claim transfer; emits event-only `Deleted` on Device deletion; Core pre-opens `/dev/uhid` inside the Guest at frontend launch time using the virtual Device's DeviceGrant |
| Integration | Device controller creates/updates/deletes the virtual Device; Core uses its DeviceGrant to inject the UHID fd into the frontend Process; frontend creates the virtual FIDO2 device without ambient `/dev` access. |
| Data migration | Full d2b 3.0 reset; no v2 frontend device state import |
| Validation | `controller_reconcile.rs`, `device_grant_no_path.rs`, and `host_relay_guest_frontend/` prove virtual Device creation/update/delete, UHID fd injection, no `/dev/uhid` path, and child-first deletion. |
| Removal proof | Supersedes guest udev/plugdev access for UHID; W-X06 removes runtime udev broker op and W-X03 removes legacy frontend unit once DeviceGrant path is live. |

### Removal items

### W-X01

| Field | Value |
| --- | --- |
| Dependency/owner | Provider-device-security-key removal owner; depends on W-R01, W-R02, W-N03, W-N04, and W-N05 successor relay/session coverage. |
| Current source | `packages/d2bd/src/security_key.rs` — `start_sk_accept_loop`, `SecurityKeyState`, `LeaseState`, `SkRegistry` |
| Reuse action | delete |
| Destination | Removed from daemon; successor behavior lives in `packages/d2b-provider-device-security-key/src/relay.rs`, `session.rs`, and `cid.rs` |
| Detailed design | Remove target `packages/d2bd/src/security_key.rs` — `start_sk_accept_loop`, `SecurityKeyState`, `LeaseState`, `SkRegistry` after v3 relay Process is live and stable; keep behind feature gate only if needed during transition. |
| Integration | d2bd no longer owns security-key accept/session state; Device controller and relay Process own lifecycle; Core LaunchTicket owns hidraw DeviceGrant; tests and call sites are redirected before deletion. |
| Data migration | Full d2b 3.0 reset; no daemon session state migration |
| Validation | Provider relay/session tests pass; daemon build has no references to removed symbols; no legacy security-key accept loop starts under d2bd. |
| Removal proof | Concrete removed path/behavior: `packages/d2bd/src/security_key.rs` `start_sk_accept_loop`, `SecurityKeyState`, `LeaseState`, and `SkRegistry` daemon-internal accept/session ownership are absent. |

### W-X02

| Field | Value |
| --- | --- |
| Dependency/owner | d2bd integration removal owner; depends on W-X01. |
| Current source | `packages/d2bd/src/lib.rs` — `start_sk_accept_loop` call site and daemon-internal Unix socket proxy bind |
| Reuse action | delete |
| Destination | Removed from daemon startup; successor launch path is ProviderDeployment/controller-created relay Process plus Endpoint/ComponentSession transport |
| Detailed design | Remove target `packages/d2bd/src/lib.rs` — `start_sk_accept_loop` call site and daemon-internal Unix socket proxy bind after W-X01. |
| Integration | d2bd startup no longer binds a security-key Unix socket proxy; Core/ProviderDeployment starts provider controller and relay Process resources; transport-vsock Endpoint supplies frontend connectivity. |
| Data migration | Full d2b 3.0 reset; no daemon socket state migration |
| Validation | d2bd startup tests/build prove no `start_sk_accept_loop` call or security-key proxy bind remains; provider integration test proves CTAPHID flow through Endpoint/ComponentSession. |
| Removal proof | Concrete removed path/behavior: `packages/d2bd/src/lib.rs` no longer calls `start_sk_accept_loop` and no longer binds the daemon-internal security-key Unix socket proxy. |

### W-X03

| Field | Value |
| --- | --- |
| Dependency/owner | Guest Nix module removal owner; depends on W-N13 migration gate and W-N03/W-N19 frontend Process/UHID DeviceGrant. |
| Current source | `nixos-modules/components/security-key-guest.nix` — `d2b-sk-frontend.service` systemd unit declaration |
| Reuse action | delete |
| Destination | Removed from guest Nix module; successor frontend lifecycle is the v3 Process resource `device-<uid-short>-sk-frontend` managed by Process Provider |
| Detailed design | Remove target `nixos-modules/components/security-key-guest.nix` — `d2b-sk-frontend.service` systemd unit declaration after W-N13 migration gate defaults to false. |
| Integration | Guest Nix keeps `uhid` module and frontend binary closure only; Device controller creates frontend Process; system-systemd manages the transient user-scope Process. |
| Data migration | Full d2b 3.0 reset; no legacy unit state migration |
| Validation | Nix eval tests prove no static `d2b-sk-frontend.service` is emitted with Provider installed; frontend Process integration proves replacement lifecycle. |
| Removal proof | Concrete removed path/behavior: `nixos-modules/components/security-key-guest.nix` no longer declares the static `d2b-sk-frontend.service` unit. |

### W-X04

| Field | Value |
| --- | --- |
| Dependency/owner | Test-suite migration/removal owner; depends on W-R06 and W-R07 provider-crate successor tests. |
| Current source | `packages/d2b-contract-tests/tests/minijail_sk_frontend.rs` and `packages/d2b-contract-tests/tests/usb_sk_contract.rs` |
| Reuse action | delete after move/adapt |
| Destination | Removed from `packages/d2b-contract-tests/tests/`; successor tests live in `packages/d2b-provider-device-security-key/tests/` |
| Detailed design | Remove target `packages/d2b-contract-tests/tests/minijail_sk_frontend.rs` and `packages/d2b-contract-tests/tests/usb_sk_contract.rs` after W-R06/W-R07 tests are in Provider crate and cover all prior assertions. |
| Integration | D094 disposition updates closed gate manifests, layer1 jobs, pins, ledgers, and CI shards so only the provider-crate successor suite remains. |
| Data migration | None — test-only move/delete; no runtime state |
| Validation | Provider-crate tests pass with retained assertions; old contract-test paths are absent from manifests/CI; no duplicate old/new suite runs indefinitely. |
| Removal proof | Concrete removed paths: `packages/d2b-contract-tests/tests/minijail_sk_frontend.rs` and `packages/d2b-contract-tests/tests/usb_sk_contract.rs` are deleted after provider-crate successor coverage passes. |

### W-X05

| Field | Value |
| --- | --- |
| Dependency/owner | Core ProcessRole removal owner; depends on W-N09 Process resources, W-N08 sandbox templates, and system-minijail/system-systemd conformance. |
| Current source | `ProcessRole::SecurityKeyFrontend` in `d2b-core/src/processes.rs` |
| Reuse action | delete |
| Destination | Removed from `d2b-core/src/processes.rs`; successor frontend is a v3 Process resource owned by `Provider/device-security-key` |
| Detailed design | Remove target `ProcessRole::SecurityKeyFrontend` in `d2b-core/src/processes.rs` after relay and frontend are v3 Process resources; no other code reference expected. |
| Integration | ProcessRole disposition table confirms all security-key frontend lifecycle, sandbox, readiness, and DeviceGrant semantics are represented by Resource Process templates and Process Providers before enum removal. |
| Data migration | Full d2b 3.0 reset; no processes.json role migration |
| Validation | Workspace build proves no `ProcessRole::SecurityKeyFrontend` references; provider Process template tests prove the v3 replacement; process conformance passes. |
| Removal proof | Concrete removed path/behavior: `d2b-core/src/processes.rs` no longer contains `ProcessRole::SecurityKeyFrontend` or a security-key frontend role in the legacy ProcessRole/VmProcessDag model. |

### W-X06

| Field | Value |
| --- | --- |
| Dependency/owner | Broker/contracts/Nix removal owner; depends on W-R05, W-N11, W-N13, and W-N19 UHID DeviceGrant replacement. |
| Current source | `SecurityKeyApplyUdevRules` broker op, `SecurityKeyApplyUdevRulesRequest` DTO in `packages/d2b-contracts/src/security_key.rs`, and all related broker code |
| Reuse action | delete |
| Destination | Removed from contracts and broker; successor access is static guest Nix `uhid` module plus Core pre-opened `/dev/uhid` DeviceGrant for the frontend Process |
| Detailed design | Remove `SecurityKeyApplyUdevRules` broker op, `SecurityKeyApplyUdevRulesRequest` DTO in `packages/d2b-contracts/src/security_key.rs`, and all related broker code after v3 guest Nix module with static udev rules is live and stable. |
| Integration | Guest Nix/Process DeviceGrant path provides UHID access; contracts no longer expose the op/request; broker capability set drops the udev mutation; provider/contract tests assert absence. |
| Data migration | Full d2b 3.0 reset; no udev rule state migration |
| Validation | DTO unknown-field/capability tests prove `SecurityKeyApplyUdevRulesRequest` and op are absent; `device_grant_no_path.rs` proves frontend has UHID fd without udev/plugdev; broker build has no related code. |
| Removal proof | Concrete removed path/behavior: `SecurityKeyApplyUdevRules` broker operation, `SecurityKeyApplyUdevRulesRequest` in `packages/d2b-contracts/src/security_key.rs`, and related broker code are absent. |

### W-N21

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-zone-control-019, ADR046-zone-control-020; security-key relay/session owner |
| Current source | None — net-new ADR 0046 cross-Zone sharing (D096) |
| Reuse action | net-new (implement the signed security-key export/import adapter) |
| Destination | `packages/d2b-provider-device-security-key/src/share_adapter.rs` |
| Detailed design | Implement the signed security-key `ExportAdapter`/`ImportAdapter`: the owner-Zone relay stays the sole hidraw FD holder; the `ResourceExport` serializes CTAP ceremonies with one exclusive per-device lease, a fair queue, and per-ceremony deadline/cancel; the import adapter builds a local `Device` projection/frontend. CTAPHID bytes flow over the bounded encrypted named stream, visible only to the trusted relay and the exact child frontend; intermediaries see ciphertext. No USBIP or direct hidraw access crosses a Zone; only bounded audit metadata is recorded. Core owns routing and base lifecycle. |
| Integration | Core export/import controller (ADR046-zone-control-019); local projection lifecycle (ADR046-zone-control-020); relay session/lease (`session.rs`); ComponentSession bounded encrypted named streams |
| Data migration | Full d2b 3.0 reset; no cross-Zone sharing state |
| Validation | CTAP ceremony serialization with one exclusive per-device lease/fair-queue/deadline/cancel; child `Device` projection reachable; CTAP bytes ciphertext to intermediaries; no hidraw FD/USBIP crosses a Zone; reconnect revalidation and revocation degrade the projection; audit metadata only (fake-stream hermetic + real-device integration) |
| Removal proof | Not applicable (new surface) |

### W-N22

| Field | Value |
| --- | --- |
| Dependency/owner | W-N21, ADR046-zone-control-019, ADR046-zone-control-020; security-key relay/session owner |
| Current source | `packages/d2bd/src/security_key.rs` (`CidTranslator`, `SecurityKeyState`, `LeaseId`/`LeaseState`, `CEREMONY_TIMEOUT` 120 s, `QUEUE_WAIT_TIMEOUT` 15 s, `parse_ctaphid_report`/`build_cancel_packet`); `packages/d2b-priv-broker/src/ops/security_key.rs` (`live_open_hidraw_security_key`, double `fstat` + FIDO usage-page 0xF1D0 + HID raw-info revalidation, `O_RDWR|O_NONBLOCK|O_NOFOLLOW`); `packages/d2b-sk-frontend/src/{main,uhid,vsock,framing}.rs` (UHID FIDO2 CTAPHID frontend, 64-byte report relay) |
| Reuse source | Same baseline daemon/broker/frontend symbols |
| Reuse action | `adapt` — relay becomes the D097 hidraw authority; transport moves to Endpoint/named-stream |
| Destination | `packages/d2b-provider-device-security-key/src/{authority,relay,streams}.rs`; `AuthorityDescriptor` on the relay `Endpoint`/`Device` |
| Detailed design | The relay is the single D097 hidraw **authority**: `AuthorityDescriptor` with `authorityScope: physical-device`, opaque `authorityKey` class (digest of the trusted bundle `device_token`/FIDO selector — never a raw path/serial), `cardinality: zero-or-one`, `arbitration: exclusive`; core's authority index rejects a duplicate relay with `duplicateConflict` before any open; restart adopts by `ownerProof`; ambiguity quarantines; D091 drains the consumer then recycles the relay. Preserves the exact reusable semantics: sole hidraw opener (Core `SecurityKeyOpenDevice`/`live_open_hidraw_security_key` with double-`fstat`+FIDO+HID revalidation), async cancellation-safe fd I/O, per-session `CidTranslator`, `LeaseId` stale-release guard, cancel of all active CIDs on disconnect, one ceremony (120 s), bounded fair wait (15 s → `ERR_CHANNEL_BUSY`), UHID frontend. Transport is D096/D092: per-import bounded **encrypted named stream** over the relay `Endpoint` (credit backpressure, per-import session generation, deadline, cancel) replaces the fixed `SK_VSOCK_PORT` accept loop and raw framing. No new `ProcessRole`, no direct broker path, no state file: hidraw/UHID fds via D077 EffectPort/LaunchTicket DeviceGrant, observed state D087 status-first. |
| Integration | Relay authority owns the hidraw fd and the `Endpoint/<device-uid>-sk-ctaphid-relay`; export/import adapter (W-N21) and controller call it; core authority index (ADR046-zone-control-019) admits exactly one relay authority; USBIP mutual-exclusion assertion (`!(usbip.yubikey && usb.securityKey.enable)`) remains. |
| Data migration | Full d2b 3.0 reset; no per-session/lease state persisted |
| Validation | Fast hermetic tests adapt the existing `CidTranslator`/lease/cancel/UHID/broker-revalidation suites: CID alloc/translate/release, `LeaseId` stale-release, cancel-all-CIDs on disconnect, one-ceremony + 120 s timeout, 15 s fair-wait `ERR_CHANNEL_BUSY`, UHID frame round-trip, and broker double-`fstat`+FIDO+HID revalidation — all with fakes/`FakeEffectPort`, no real hidraw. Integration proves cross-Zone CTAP ceremony **serialization** over the encrypted named stream; the USBIP-vs-security-key conflict assertion/test remains. |
| Removal proof | The legacy daemon accept loop, raw CTAPHID framing, fixed `SK_VSOCK_PORT`, and broker sysfs `/sys/class/hidraw/` scan fallback are deleted only after the relay `Endpoint`/named-stream successor and the `device_token`-only broker open are green (coordinated with W-X05 `ProcessRole` removal and the W-R broker-op revalidation item). |

Per D094, each replaced current-code test is retired with an explicit
keep/adapt/move/delete disposition and a removal gate: the minimum reusable
semantic assertions migrate into this crate's hermetic `tests/`, and the old
duplicate tests, shell gates, fixtures, static artifacts, CI jobs, and manifest
entries are deleted once successor coverage and the removal proof pass —
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
deterministic fake clock/RNG and the toolkit fakes/FakeEffectPort only — no
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
| `controller_reconcile.rs` | Device reconcile trigger handlers: spec-changed → relay Process created with correct `access: exclusive` deviceUsage + virtual frontend Device created + frontend Process created; deletion-requested → relay Process Deleted → DeviceGrant released → frontend Process Deleted → virtual Device Deleted → finalizer cleared; dependency-changed → session cancelled; scheduled-observe → `observe_inventory(&device_id, &policy_id)` called → phase transitions. Controller never calls Volume create/delete; no Volume API authority asserted. |
| `session_ring.rs` | Bounded overflow: ring at `sessionRingSize` capacity evicts oldest; newest session always present; ring size bounds (8–256) enforced |
| `session_state_machine.rs` | Idle → Active → Completed; Idle → Active → TimedOut; back-to-back sessions; `CancelSession(sessionId)` from controller in Active state → Completed → Idle; no AwaitingLease state; DeviceGrant persists through cancel until relay exit |
| `second_claim_conflict.rs` | Second Guest attach rejected with `ClaimConflict`; first session unaffected; condition cleared when first session ends |
| `device_grant_no_path.rs` | Relay LaunchTicket DeviceGrant: relay process namespace contains no `/dev/hidraw*` path; relay never calls `SecurityKeyOpenDevice` broker op; fd is pre-opened by Core and injected via LaunchTicket. Frontend LaunchTicket DeviceGrant for virtual Device: frontend process namespace contains no `/dev/uhid` path; UHID fd is pre-opened by Core; no udev rule or plugdev group needed. |
| `mutual_exclusion.rs` | Runtime: `device-usbip` active for same vendorId/productId → `device-security-key` Device → `Failed`/`ClaimConflict`; eval: Nix assertion catches both active in same Zone |
| `cancel_propagation.rs` | Operator cancel → controller sends `CancelSession(sessionId)` via manifest-declared internal service → relay sends CTAPHID_CANCEL to token → `SessionCompleted`(reason=operator-cancel); DeviceGrant NOT released by CancelSession (persists until relay exit); disconnect mid-ceremony → same CancelSession sequence; relay crash → Core releases DeviceGrant automatically |
| `session_timeout.rs` | CEREMONY_TIMEOUT elapsed → Active → TimedOut → Idle; audit `device-session-timeout`; relay restartable after timeout |
| `cid_isolation.rs` | Two concurrent Guest connections to different relay instances do not share CID namespace; CID allocated per-session; CID translation round-trip for CTAPHID_INIT response; relay uses canonical subject from ComponentSession, not raw vsock CID |
| `descriptor_validation.rs` | Manifest-declared relay-ctrl service: unregistered service name rejected; LaunchTicket internal channel FD not at ambient path; wrong descriptor digest rejected; wrong SO_PEERCRED uid rejected; oversized record discarded. Noise KK relay ComponentSession: unenrolled static key rejected; wrong service name rejected. `DeviceId`/`ObservationPolicyId` Debug output redacted in test log capture. |
| `status_state.rs` | Provider descriptor declares no Provider state Volume; ProviderStateSet query is empty; controller/relay/frontend Process templates have no `/state` mounts; controller never calls Volume create/delete; bounded operational observations are written to Device status and Operation rows; CTAP bytes, relay stream data, fd handles, and session keys are absent from status/log/audit/metrics and remain transient in process memory |

### Integration (in `integration/`)

| Fixture | What is tested |
| --- | --- |
| `host_relay_guest_frontend/` | Container/host fixture: relay binary receives a fake hidraw fd via LaunchTicket DeviceGrant; frontend binary receives a fake UHID fd via virtual Device LaunchTicket DeviceGrant (no `/dev/uhid` path in frontend namespace); frontend connects over `d2b.security-key.v3` ComponentSession (Noise KK, transport-vsock allocated endpoint); 64-byte CTAPHID INIT exchange completes over named CTAPHID stream; CID translated and reversed; session completed on frontend exit |
| `claim_conflict/` | Two simulated Guests race to connect to same relay; first succeeds; second receives ERR_CHANNEL_BUSY; first session completes normally |
| `usbip_mutual_exclusion/` | Eval check: Nix assertion fires; runtime check: controller sets Failed + ClaimConflict when USBIP Device present for same selector |

### Existing contract tests (reuse/update)

| Existing test | Action |
| --- | --- |
| `packages/d2b-contract-tests/tests/usb_sk_contract.rs` | Move to `packages/d2b-provider-device-security-key/tests/` as part of W-R06; update v3 type imports; retain all existing assertions |
| `packages/d2b-contract-tests/tests/minijail_sk_frontend.rs` | Move to `packages/d2b-provider-device-security-key/tests/` as part of W-R07; update for the v3 Process resource sandbox; retain zero-`capabilityClasses` and `seccompClass` assertions |
| `packages/d2b-priv-broker/tests/security_key_broker.rs` | Retain in broker crate; update for v3 bundle table lookup path; add zone-field round-trip test |

## Nix option migration

| Current option | v3 successor |
| --- | --- |
| `d2b.host.usb.securityKey.enable = true` | Install `Provider/device-security-key` in Zone; configure `spec.config.devices` |
| `d2b.host.usb.securityKey.devices[].label` | `spec.config.devices[].label` in Provider resource |
| `d2b.host.usb.securityKey.devices[].vendorId` | `spec.config.devices[].vendorId` in Provider resource |
| `d2b.host.usb.securityKey.devices[].productId` | `spec.config.devices[].productId` in Provider resource |
| `d2b.vms.<vm>.securityKey.enable = true` | Declare `Device/<name>` resource with `providerRef: Provider/device-security-key` and `metadata.ownerRef: Guest/<vm>` |
| (none — was not configurable) | `sessionRingSize`, `leaseTimeoutSecs`, `queueWaitTimeoutSecs` in Provider root config |

The current `d2b.vms.<vm>.usbip.yubikey` and `d2b.vms.<vm>.securityKey` mutual-exclusion
assertion in `nixos-modules/assertions.nix` is preserved in v3 as an eval-time assertion
on the v3 `d2b.zones.<zone>.resources` tree. The assertion shape changes to compare
`inventory.selector.vendorId`/`productId` across `device-usbip` and `device-security-key`
Device resources in the same Zone.

## References

- `packages/d2bd/src/security_key.rs` — baseline relay implementation (implemented-and-reachable)
- `packages/d2b-sk-frontend/src/` — baseline guest frontend binary (implemented-and-reachable)
- `packages/d2b-priv-broker/src/ops/security_key.rs` — broker hidraw open op (implemented-and-reachable)
- `packages/d2b-contracts/src/security_key.rs` — public and broker wire DTOs (implemented-and-reachable)
- `packages/d2b-contract-tests/tests/usb_sk_contract.rs` — existing contract tests (implemented-and-reachable)
- `packages/d2b-contract-tests/tests/minijail_sk_frontend.rs` — existing minijail contract tests (implemented-and-reachable)
- `packages/d2b-priv-broker/tests/security_key_broker.rs` — existing broker tests (implemented-and-reachable)
- `docs/specs/ADR-046-resources-device.md` §Provider: device-security-key — Device ResourceType contract and key invariants
- `docs/specs/ADR-046-componentsession-and-bus.md` — ComponentSession Noise profiles, descriptor validation, attachments
- `docs/specs/ADR-046-provider-model-and-packaging.md` — Provider crate boundary, component descriptors
- `docs/specs/ADR-046-components-processes-and-sandbox.md` — Process model, ProviderSupervisor, minijail
- `docs/specs/ADR-046-resource-reconciliation.md` — standard async reconcile interface
- `docs/specs/ADR-046-telemetry-audit-and-support.md` — OTEL label constraints, audit stream
- `docs/specs/ADR-046-nix-configuration.md` — Nix resource compilation, prohibited fields, eval invariants
- `nixos-modules/components/security-key-guest.nix` — current guest Nix module (eval-contract)
- `nixos-modules/assertions.nix` — current mutual-exclusion assertion (eval-contract)
