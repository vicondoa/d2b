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
the base spec and base status only. A reference to the former Device
`spec.settings` denotes `spec.provider.settings`; no secret bytes are allowed
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
  endpoints:
    - name: ctrl-relay
      transport: unix
      purpose: device-security-key.relay-ctrl.v1
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
  endpoints:
    - name: ctaphid-relay
      transport: vsock
      purpose: d2b.security-key.v3
    - name: ctrl-relay
      transport: unix
      purpose: device-security-key.relay-ctrl.v1
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
  `Provider/transport-vsock` allocated endpoint (see §ComponentSession).
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
  `device-security-key.relay-ctrl.v1` using the bound endpoint FD from its
  LaunchTicket (`ctrl-relay` endpoint). Reports session lifecycle events and
  receives `CancelSession` signals.
- Has narrow ComponentSession service authority: responder of
  `d2b.security-key.v3` and client of `device-security-key.relay-ctrl.v1` only.
  No Zone resource API authority, no broker connection, no write access to Zone
  resource store.
- On crash, Core releases the DeviceGrant and OFD lease; the Device controller
  observes `owned-resource-changed` and sets `DeviceHealthy=False`. The relay is
  restarted after back-off.

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
  endpoints:
    - name: ctaphid-client
      transport: vsock
      purpose: d2b.security-key.v3
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
  ComponentSession over the `Provider/transport-vsock` allocated endpoint (the
  endpoint ID is compiled into the Process spec from `endpoints[ctaphid-client]`).
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

**Transport allocation:** `Provider/transport-vsock` allocates an opaque vsock
endpoint ID for each relay Process. The relay does not bind a raw vsock port. The
endpoint ID is:
- Compiled into the relay Process spec as the `endpoints[ctaphid-relay]` entry
  (transport: vsock, purpose: d2b.security-key.v3).
- Compiled into the frontend Process spec as the `endpoints[ctaphid-client]`
  connect target.
- Opaque to operators; never configurable as a port number. `vsockPort` does not
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
path. The bound endpoint FD is injected into the relay's LaunchTicket as the
`ctrl-relay` endpoint entry — the relay never resolves a path to find the
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
internal service endpoint FD is injected via LaunchTicket; no ambient path is
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

| Work item | Source (baseline evidence class) | v3 destination |
| --- | --- | --- |
| W-R01 | `packages/d2bd/src/security_key.rs` — `SecurityKeyState`, `LeaseState`, `LeaseId`, `CidTranslator`, `try_acquire_lease`, `release_lease`, `CEREMONY_TIMEOUT`, `QUEUE_WAIT_TIMEOUT` (implemented-and-reachable) | Move to `packages/d2b-provider-device-security-key/src/session.rs` and `cid.rs`; adapt to Provider Process model (remove daemon Mutex wrapping, add async relay protocol) |
| W-R02 | `packages/d2bd/src/security_key.rs` — CTAPHID relay loop, `SkAcceptHandle`, `relay_one_ceremony` (implemented-and-reachable) | Move to `packages/d2b-provider-device-security-key/src/relay.rs`; replace daemon-internal Unix socket proxy with vsock framing |
| W-R03 | `packages/d2b-sk-frontend/src/` — `main.rs`, `uhid.rs` (implemented-and-reachable); `framing.rs` and `vsock.rs` are obsolete under v3 (replaced by ComponentSession transport) | Adopt `main.rs` and `uhid.rs` as the v3 Process binary entry point; replace `framing.rs`/`vsock.rs` with ComponentSession client from `d2b-session-unix/src/vsock.rs`; wire as Process service in Provider crate |
| W-R04 | `packages/d2b-priv-broker/src/ops/security_key.rs` — `live_open_hidraw_security_key`, FIDO usage page revalidation, group validation, `ALLOWED_GROUPS` (implemented-and-reachable) | Preserve revalidation logic; update `SecurityKeyOpenDevice` to use bundle device table `device_token` as sole open target (no iterative sysfs scan); add zone-field handling; remove sysfs fallback. **Core's LaunchTicket calls this internally; the Provider does not call it.** |
| W-R05 | `packages/d2b-contracts/src/security_key.rs` — `SecurityKeySessionId`, `SecurityKeyDeviceLabel`, `SecurityKeySession`, `SecurityKeySessionResult`, `SecurityKeyStatusResponse`, `SecurityKeySessionsResponse`, `SecurityKeyOpenDeviceRequest`, `SecurityKeyEvent` (implemented-and-reachable) | Adapt to v3 Zone/ResourceRef identifiers; preserve serde shapes for zero downstream breakage where possible; remove `SecurityKeyApplyUdevRulesRequest` (W-X06) |
| W-R06 | `packages/d2b-contract-tests/tests/usb_sk_contract.rs` — DTO serde round-trips, unknown-field denial, broker capability set (implemented-and-reachable) | Move to `packages/d2b-provider-device-security-key/tests/`; update imports and v3 type names |
| W-R07 | `packages/d2b-contract-tests/tests/minijail_sk_frontend.rs` — minijail profile shape, `ProcessRole::SecurityKeyFrontend` (implemented-and-reachable) | Move to `packages/d2b-provider-device-security-key/tests/`; update for v3 Process resource minijail profile; retain zero-capabilities assertion |

### New items

| Work item | Description |
| --- | --- |
| W-N01 | New crate `packages/d2b-provider-device-security-key/` with `src/`, `tests/`, `integration/`, `README.md` (workspace policy requires all four) |
| W-N02 | Device controller: `controller.rs` implementing standard reconcile interface (`spec-generation-changed`, `deletion-requested`, `dependency-changed`, `scheduled-observe`, `owned-resource-changed`) |
| W-N03 | Relay Process entry point: `relay.rs` — async ComponentSession accept loop (`d2b.security-key.v3` responder over `Provider/transport-vsock` allocated endpoint), CID translation, hidraw fd received from LaunchTicket DeviceGrant (not broker), CTAPHID proxy over named CTAPHID stream; manifest-declared internal service `device-security-key.relay-ctrl.v1` connected via LaunchTicket endpoint FD for controller messaging |
| W-N04 | Session state machine: `session.rs` — `SessionStateMachine` (Idle/Active/Completed/TimedOut; no AwaitingLease), session ring, session ID allocation, ring eviction; DeviceGrant held for relay lifetime; `CancelSession(sessionId)` from controller terminates Active ceremony |
| W-N05 | CID translator: `cid.rs` — per-session u32→u64 host-CID allocation, bimap, eviction on session end |
| W-N06 | hidraw probe: `probe.rs` — calls `SecurityKeyEffectPort::observe_inventory(&device_id, &policy_id)` with opaque types injected by Core; interprets `InventoryObservation`; never reads `/sys/class/hidraw/` directly; bundle device table population at activation time (Provider activation resolves label → `device_token` via Core; stored in private bundle) |
| W-N07 | ComponentSession descriptor validation: `descriptor.rs` — manifest-declared relay↔controller service `device-security-key.relay-ctrl.v1` Noise NN handshake; socket FD injected via LaunchTicket endpoint (no ambient path); relay↔frontend `d2b.security-key.v3` Noise KK enrolled-key registration and session authority enforcement; relay uses canonical authenticated subject from ComponentSession, never raw vsock CID |
| W-N08 | Minijail profiles for relay and controller only; frontend uses `Provider/system-systemd` hardening directives compiled from `SandboxSpec` (no minijail profile for frontend). Add relay and controller entries to `nixos-modules/minijail-profiles.nix`; `capabilityClasses: []`; `seccompClass: sk-relay` and `seccompClass: sk-controller` |
| W-N09 | Process resource templates in Provider descriptor: controller template (`Host/host-system`, `system`, `controller`, `environmentClass: provider-defined`, `endpoints[ctrl-relay]`), relay template (`Host/host-system`, `system`, `service`, `environmentClass: provider-defined`, `endpoints[ctaphid-relay, ctrl-relay]`), and frontend template (`Guest/<vm>`, `user`, `service`, `environmentClass: provider-defined`, `endpoints[ctaphid-client]`, `userRef` required) |
| W-N10 | Provider descriptor JSON (signed): identity, config schema, exported ResourceType (Device/hidraw), controller/relay/frontend component descriptors, `d2b.security-key.v3` service declaration, D087 status-first state declaration with an empty ProviderStateSet, permission claims |
| W-N11 | v3 `SecurityKeyOpenDevice` broker op update: add `zone` field; implement bundle device table `device_token` lookup as sole open path; remove iterative sysfs scan from broker; add post-open revalidation steps (fstat, HIDIOCGRAWINFO, HIDIOCGRDESC). This is an internal Core operation called by LaunchTicket; the Provider controller does not call it. |
| W-N20 | Status-first Provider state contract: the signed Provider descriptor declares no Provider state Volume for controller, relay, or frontend; ProviderStateSet is empty. Device resources and the Core Operation ledger are the operational authority. Controller/relay/frontend Process templates have no `/state` mount and no dedicated state-layout principals. Nix pre-provisions only principals required for genuine Process placement and DeviceGrant access, not state Volume ownership. |
| W-N12 | Nix resource compilation: Device spec validation in `nixos-modules/`, eval-time mutual-exclusion assertion, label resolution against Provider config, prohibited-field rejection |
| W-N13 | Guest Nix module migration gate: `d2b.securityKey._legacySystemdUnit` option, defaulting to false when Provider is installed; remove `d2b-sk-frontend.service` unit |
| W-N14 | Audit record emission: bounded path-free `device-grant` records from Core at DeviceGrant resolution time; `device-session` lifecycle events from Device controller; neither block carries device path, guest name, session content, or CTAP bytes |
| W-N15 | OTEL metrics: `d2b_device_sk_session_total`, `d2b_device_sk_ceremony_duration_seconds`, `d2b_device_sk_relay_restarts_total` via bounded emitter ring |
| W-N16 | `README.md` for the crate: Provider identity, root config schema, Device spec, process model, RBAC, security invariants, state/telemetry, build/test/integration commands, standalone-repository consumption |
| W-N17 | `Provider/transport-vsock` integration: allocate opaque vsock endpoint ID for each relay Process at creation time; compile the endpoint ID into the relay Process `endpoints[ctaphid-relay]` spec and into the frontend Process `endpoints[ctaphid-client]` connect target; enroll Noise KK static keys for relay/frontend pair before first connection |
| W-N18 | `SecurityKeyEffectPort` trait and associated opaque types (`DeviceId`, `ObservationPolicyId`) defined in `d2b-contracts` (neutral contract crate); both types have custom `Debug` impls that redact content; `effect_port.rs` in the Provider crate re-exports from `d2b-contracts`; Core adapter implementation in `d2b-provider` or `d2b-provider-toolkit` crate; inject into Device controller at startup with concrete `DeviceId` and `ObservationPolicyId` per Device; relay does NOT use the port |
| W-N19 | Virtual frontend Device lifecycle: controller creates `Device/<device-name>-frontend` (`deviceClass: virtual`, `busClass: uhid`, `ownerRef: Device/<device-name>`, `settings.bindGuest`) on claim; updates `bindGuest` on claim transfer; emits event-only `Deleted` on Device deletion; Core pre-opens `/dev/uhid` inside the Guest at frontend launch time using the virtual Device's DeviceGrant |

### Removal items

| Removal item | Target to remove | Condition |
| --- | --- | --- |
| W-X01 | `packages/d2bd/src/security_key.rs` — `start_sk_accept_loop`, `SecurityKeyState`, `LeaseState`, `SkRegistry` | After v3 relay Process is live and stable; behind feature gate if needed during transition |
| W-X02 | `packages/d2bd/src/lib.rs` — `start_sk_accept_loop` call site and daemon-internal Unix socket proxy bind | After W-X01 |
| W-X03 | `nixos-modules/components/security-key-guest.nix` — `d2b-sk-frontend.service` systemd unit declaration | After W-N13 migration gate defaults to false |
| W-X04 | `packages/d2b-contract-tests/tests/minijail_sk_frontend.rs` and `packages/d2b-contract-tests/tests/usb_sk_contract.rs` | After W-R06/W-R07 tests are in Provider crate and cover all prior assertions |
| W-X05 | `ProcessRole::SecurityKeyFrontend` in `d2b-core/src/processes.rs` | After relay and frontend are v3 Process resources; no other code reference expected |
| W-X06 | Remove `SecurityKeyApplyUdevRules` broker op, `SecurityKeyApplyUdevRulesRequest` DTO in `packages/d2b-contracts/src/security_key.rs`, and all related broker code | After v3 guest Nix module with static udev rules is live and stable |

## Tests

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
| `descriptor_validation.rs` | Manifest-declared relay-ctrl service: unregistered service name rejected; LaunchTicket endpoint FD not at ambient path; wrong descriptor digest rejected; wrong SO_PEERCRED uid rejected; oversized record discarded. Noise KK relay ComponentSession: unenrolled static key rejected; wrong service name rejected. `DeviceId`/`ObservationPolicyId` Debug output redacted in test log capture. |
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
