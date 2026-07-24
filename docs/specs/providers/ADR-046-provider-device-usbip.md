# ADR 0046 Provider dossier: device-usbip

| Field | Value |
| --- | --- |
| Spec ID | `ADR-046-provider-device-usbip` |
| Parent | ADR 0046 |
| Status | Proposed |
| Version | 7 |
| Baseline | `b5ddbed67867d9244bf33390868101bd9b053e49` |
| Normative | Yes |
| Owners | `d2b-provider-device-usbip` crate, USB Service/Binding controller contracts, Nix USB resource emitter |
| Depends on | `ADR-046-resources-device`, `ADR-046-resources-network`, `ADR-046-resources-zone-control`, `ADR-046-zone-routing`, `ADR-046-components-processes-and-sandbox`, `ADR-046-provider-model-and-packaging`, `ADR-046-resource-reconciliation`, `ADR-046-telemetry-audit-and-support`, `ADR-046-componentsession-and-bus`, `ADR-046-nix-configuration`, `ADR-046-resource-api-and-authorization`, `ADR-046-provider-state` |
| Supersedes | `nixos-modules/components/usbip.nix` (host-side), per-env usbipd systemd units in `nixos-modules/network.nix`, `ProcessRole::Usbip` / `RunnerRole::Usbip` in current v3 baseline |

---

## Purpose

This dossier specifies `Provider/device-usbip`, the initial implementation of
the generic D098 USB Service/Binding family. This Provider owns USB/IP (USBIP)
device inventory, arbitration, busid probe/claim, host-side kernel bind,
singleton backend and per-Network relay lifecycle, firewall carve-out,
per-Guest attachment state, policy-gated cross-Zone service propagation, and
operator CLI for the `d2b device usb` surface. Future Providers may implement
the same `usb.d2bus.org.UsbService` and `usb.d2bus.org.UsbBinding` base
contracts without USBIP.

`Provider/device-usbip` is one of the four frozen Device Providers in the
`ADR-046-resources-device` catalog. It replaces:

- the broker-spawned `RunnerRole::Usbip` backend and proxy runners managed by
  `packages/d2bd/src/usbipd_perenv_autostart.rs`;
- the typed step machine in `packages/d2bd/src/usbip_state_machine.rs`;
- the reconcile state model in `packages/d2bd/src/usbip_reconcile_state.rs`;
- the per-env firewall carve-out in `nixos-modules/network.nix` lines 444–461;
- the guest-side module wiring in `nixos-modules/components/usbip.nix`.

---

## Provider identity

```text
Provider/device-usbip
```

Crate: `packages/d2b-provider-device-usbip/`

The Provider is installed in a Zone via the standard artifact catalog mechanism.
The Provider resource `spec.artifactId` references the catalog entry named
`provider-device-usbip`. The Zone controller creates the controller Process
resource at install time using the `controllerExecutionRef` from
`Provider.spec.config`.

---

## Crate layout and dependency rules

Required root layout (per `ADR-046-provider-model-and-packaging`):

```
d2b-provider-device-usbip/
  src/
  tests/
  integration/
  README.md
```

Workspace policy rejects the crate if any of these four paths is absent.
Internal module organisation within `src/` and `tests/` is implementation
detail; notable files are called out in work items and test coverage sections
below.

### Dependency rules

The `d2b-provider-device-usbip` crate depends **only** on:

| Crate | Purpose |
| --- | --- |
| `d2b-contracts` | busid validation/redaction, EffectPort types, signed D096 export/import adapter contracts, D097 authority descriptors |
| `d2b-provider-toolkit` | `ReconcileContext`, `ResourceClient`, `ResourceMutationBatch`, Service/Binding schema helpers, `phase_*` helpers, generic conformance |

The Provider crate **must not** import `d2b-priv-broker`, `d2bd`, `d2b-host`,
`d2b-realm-core`, Zone-store internals, or another Provider's implementation.
No raw broker op DTOs, lock file paths, nftables rule bodies, or busid strings
appear in the Provider's public types or internal reconcile logic.

---

## Implements

| ResourceType | Role | Exportability | Arbitration |
| --- | --- | --- | --- |
| `Device` | Owner-Zone physical USB inventory and busid identity | forbidden | `exclusive` physical claim |
| `usb.d2bus.org.UsbService` | Provider-neutral whole-USB-device authority or imported local projection | `explicit-export` for authority mode only | shared Host-global backing authority is `exclusive` |
| `usb.d2bus.org.UsbBinding` | Provider-neutral per-Guest access and attachment intent; observation lives in resource `status` | forbidden | one active Binding per Service |

---

## Provider.spec.config

```yaml
spec:
  artifactId: provider-device-usbip
  config:
    controllerExecutionRef: Host/host-system   # required; may reference any Zone Host
```

| Field | Type | Default | Bounds | Notes |
| --- | --- | --- | --- | --- |
| `controllerExecutionRef` | ResourceRef | — | required; `Host/<name>` only | Host on which the framework creates the controller Process. |

`controllerExecutionRef` is the only operator-configurable field.
`usbipHostKernelModule`, `vhciHcdKernelModule`, the private backend transport,
and standard USBIP TCP port 3240 are signed manifest/effect policy constants
embedded in the Provider package descriptor. They are never
operator-configurable. Executable paths for usbip, usbipd, and relay/proxy
binaries are resolved exclusively from the signed component descriptor inside
the Provider package closure.

---

## Device spec

Normative D089 spec layering: Device base fields are ResourceType base
`spec.*` fields, including `spec.providerRef`, `deviceClass`,
`inventory.selector`, attachments, and arbitration. This Provider's
desired-only extension is the canonical `spec.provider = { schemaId:
"device-usbip.d2bus.org/Device/spec", schemaVersion, settings }` envelope; it
is manifest-registered/signed, strict deny-unknown, bounded, versioned and
digested,
validated against `spec.providerRef` at Nix build and API admission,
implementation-only, and may not shadow base fields. Shared fields are promoted
to the Device base. The Provider implements the exact base Device spec/status
version/fingerprint, accepts the canonical minimal valid base Spec, and rejects
unsupported optional base capabilities only through its signed capability matrix
and provider-neutral `unsupported-capability`. `spec.provider` aligns with
`status.provider`; generic CLI/controllers operate on the base spec and base
status only. No secret bytes are allowed in any spec layer, and no
credential material is allowed in `spec.provider.settings`.

```yaml
apiVersion: resources.d2bus.org/v3
type: Device
metadata:
  name: corp-vm-usb
  zone: work
  ownerRef: null          # operator-declared; no automated owner
spec:
  providerRef: Provider/device-usbip
  deviceClass: physical
  arbitration: exclusive
  maxConcurrentClaims: 1
  inventory:
    selector:
      busClass:   usb
      label:      yubikey-work   # logical label; never a raw busid
      vendorId:   "1050"         # exactly 4 ASCII hex digits, lowercased
      productId:  "0407"
      serial:     null           # optional; null = match any serial for this vid/pid
  provider:
    schemaId: "device-usbip.d2bus.org/Device/spec"
    schemaVersion: "1.0.0"
    settings: {}
```

### Provider settings fields

The provider settings object is empty. The physical `Device` owns only
inventory, presence, and anti-spoof state; its common claim observation reflects
the Service authority holder. It carries no Guest, Network, relay, export, or
attachment policy. The provider-neutral Service/Binding pair owns only generic
whole-device backing, access, and attachment semantics; USBIP Network and relay
metadata lives in those resources' strict Provider extensions.

Bus-class `usb` is the only accepted value for `busClass` in a `device-usbip`
Device. Any other value fails spec admission with `unsupported-bus-class`.

---

## USB Service/Binding resource split (D096/D097)

`usb.d2bus.org.UsbService` and `usb.d2bus.org.UsbBinding` are
provider-neutral whole-USB-device contracts. `Provider/device-usbip` is one
implementation Provider; it is not the ResourceType namespace. A future
direct-local Provider or another USB transport Provider may implement the same
base types without exposing USBIP concepts in their base spec or status. No
USBIP-qualified ResourceType or compatibility alias exists.

The Service is the stable backing authority and the only USB resource that may
be exported. The Binding is one consuming Guest's access and attachment intent;
every observation belongs in resource `status`. A physical `Device`, an
`Endpoint`, or a `UsbBinding` is never an export target. The frozen D096 catalog
classifies this USB capability as policy-gated `explicit-export`; exclusive
backing authority, typed transport, and encrypted stream boundaries make the
USBIP implementation exportable.

### Owner authority `UsbService`

The owner Zone declares one authority-mode `usb.d2bus.org.UsbService` for each
whole physical USB device it may offer:

```yaml
apiVersion: resources.d2bus.org/v3
type: usb.d2bus.org.UsbService
metadata:
  name: work-token
  zone: work
  ownerRef: null
spec:
  providerRef: Provider/device-usbip
  mode: authority
  backingDeviceRef: Device/work-token
  accessPolicy:
    mode: exclusive
    maxActiveBindings: 1
    queue:
      discipline: fair
      maxDepth: 16
      acquireDeadline: "15s"
  backingAuthority:
    authorityScope: physical-device
    authorityClass: physical-usb-backing
    keyDerivation: core-trusted-usb-v1
    cardinality: zero-or-one
    arbitration: exclusive
    authorityRef: usb.d2bus.org.UsbService/work-token
    duplicateConflict: physical-usb-backing-conflict
    exportability: explicit-export
  provider:
    schemaId: "device-usbip.d2bus.org/UsbService/spec"
    schemaVersion: "1.0.0"
    settings:
      relayEndpointRef: Endpoint/work-net-usbip-relay
```

The base spec contains only generic whole-device semantics:
`backingDeviceRef`, generic access policy, and the shared Host-global physical
backing authority. `backingDeviceRef` resolves in the Service's Zone to a
physical standard `Device` implemented by `Provider/device-usbip`; it is the
sole source of inventory and anti-spoof identity. Core derives the backing
authority's opaque Host-global key from the trusted selector. USB services and
security-key resources contend on this same shared backing-authority index, so
neither Provider can bypass mutual exclusion by using a private authority
namespace. After trusted physical-USB identity resolution, Core derives the
`physical-usb-backing/v1` opaque digest and indexes the exact tuple
`(Host, physical-usb-backing, opaqueKeyDigest)`. Every USB or security-key
implementation backed by the same token submits that byte-identical tuple
before any open, withhold, bind, module, relay, or attachment effect. The digest
is never caller supplied or exposed in public spec/status.

The strict `spec.provider` extension is selected by `providerRef`. For the
USBIP implementation it may carry only desired USBIP implementation metadata,
including the local relay `Endpoint` reference. Network selection, TCP 3240
topology, host module/backend policy, proxy/server/client behavior, firewall
policy, busid handling, and transport tuning are Provider extension, Provider
configuration, or `status.provider` concerns; they never enter the base
`UsbService` or `UsbBinding` contract. No raw busid, sysfs path, interface,
address, port, fd, or credential appears in either base spec.

The USBIP implementation registers additional signed D097 authorities:

| Authority class | Owning Resource | Scope/cardinality | Arbitration | Conflict |
| --- | --- | --- | --- | --- |
| physical USB backing | authority `UsbService` referencing local `Device` | Host-global, `zero-or-one` per Core-derived trusted physical-USB digest under exact class `physical-usb-backing` | `exclusive`, shared with every security-key Provider | `physical-usb-backing-conflict` |
| `usbip-host` module/backend | Provider-owned Host authority derived from backing placement | `host`, `exactly-one` | shared by that Host's USBIP Services | `usbip-host-module-authority-conflict` |
| TCP 3240 relay/firewall | Provider-owned relay `Endpoint` | Host-global `host`, `exactly-one` per Core-derived Network UID/signed-policy-port opaque digest | `multiplexed` | `usbip-network-relay-authority-conflict` |

Only the first row is part of provider-neutral USB backing semantics. The Host
module/backend and relay rows are USBIP Provider metadata. Core indexes all
three before module load, busid bind, listener open, or firewall effect.
Restart adopts exact owner proofs; ambiguity quarantines instead of opening a
second backing.

There is exactly one `usbip-host` module/backend authority per Host and one
typed relay `Endpoint` per Network. The Provider's relay binds that Network on
TCP 3240 and multiplexes every admitted USBIP Service. A new Service registers
another exclusively claimed busid with the existing multiplexer; it does not
create another listener or module authority.

### Imported `UsbService` projection

The signed import adapter preserves the same provider-neutral ResourceType:

```yaml
apiVersion: resources.d2bus.org/v3
type: usb.d2bus.org.UsbService
metadata:
  name: work-token
  zone: dev
  ownerRef: ResourceImport/work-token
spec:
  providerRef: Provider/device-usbip
  mode: projection
  accessPolicy:
    mode: exclusive
    maxActiveBindings: 1
  sourceSchemaFingerprint: sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
```

Projection mode forbids `backingDeviceRef`, backing-authority ownership, and
any local physical effect. It permits only `providerRef`, semantic base/import
fields, and ResourceImport ownership; `spec.provider` is forbidden. Core
creates only this same-type Service projection, with `ownerRef` exactly equal
to the creating `ResourceImport`; it never projects a Device, Endpoint, or
Binding. Routing derives from the signed local Provider descriptor,
`providerRef`, and ResourceImport record; USBIP route observations may appear
only in `status.provider`.
The projection never discovers, opens, binds, withholds, or loads a module for
local physical USB. Import revocation degrades every dependent `UsbBinding`.

### Signed D096 projection factory

The Provider descriptor carries exactly one signed factory:

| Field | USBIP value |
| --- | --- |
| `serviceType` | `usb.d2bus.org.UsbService` |
| `bindingType` | `usb.d2bus.org.UsbBinding` |
| `allowedBackingRefTypes` | `Device` |
| `allowedBindingTargetRefTypes` | `Guest` |
| `projectionSchema` | strict same-type projection schema with `providerRef` and semantic base/import fields; no `spec.provider`, backing ref, or raw locator/path/credential/fd/bytes |
| `projectionSchemaFingerprint` | SHA-256 of canonical committed projection schema |
| `factoryFingerprint` | SHA-256 binding the semantic factory fields plus the projection-protocol version; never Provider or ExportAdapter/ImportAdapter identity/version |

Provider install, Nix admission, API admission, export, and import fail closed
if the factory/signature/type/schema/fingerprint differs. Provider adapters
are authenticated separately by the signed Provider descriptor and therefore
cannot affect the semantic fingerprint; they validate implementation
extensions. Core owns provider-neutral base lifecycle,
Export/Import routing, projection ownership, and layered status.

### Per-Guest `UsbBinding`

Each consuming Guest has one `usb.d2bus.org.UsbBinding`:

```yaml
apiVersion: resources.d2bus.org/v3
type: usb.d2bus.org.UsbBinding
metadata:
  name: corp-vm-work-token
  zone: dev
  ownerRef: Guest/corp-vm
spec:
  providerRef: Provider/device-usbip
  serviceRef: usb.d2bus.org.UsbService/work-token
  guestRef: Guest/corp-vm
  accessPolicy:
    mode: exclusive
    priority: 100
  attachmentPolicy:
    desired: attached
    activation: declared
  provider:
    schemaId: "device-usbip.d2bus.org/UsbBinding/spec"
    schemaVersion: "1.0.0"
    settings:
      networkRef: Network/dev-net
```

The base Binding contains only `serviceRef`, `guestRef`, and generic access and
attachment policy. `serviceRef` and `guestRef` are same-Zone; `ownerRef` must
equal `guestRef`. The Service may be an owner authority or imported projection.
The USBIP-only `networkRef` is validated as a same-Zone reference inside the
strict Provider extension. `attachmentPolicy.desired` is
`attached|detached`; `activation` is `declared|explicit`; priority never
overrides fair exclusive arbitration.

The USBIP Provider realization owns the Guest proxy Process, its private
exact-Guest Endpoint, `vhci_hcd` attach/detach progress, and finalizer. Those
are Provider implementation children, not base Binding fields. The proxy uses
Guest-private loopback TCP 3240 and resolves the relay through a LaunchTicket
or encrypted import stream; it never opens the physical Device. Attach/detach
commands remain internal EffectPort operations.

Only one Binding may hold the Service's shared Host-global backing claim. A
second Binding waits fairly and never causes a second busid bind, module load,
listener, or physical open. Binding deletion drains Provider-owned children,
detaches the Guest, releases the Service lease, and clears the finalizer.

### ResourceExport/ResourceImport contract

The owner Zone exports the provider-neutral Service:

```yaml
type: ResourceExport
spec:
  providerRef: Provider/device-usbip
  resourceRef: usb.d2bus.org.UsbService/work-token
  serviceType: usb.d2bus.org.UsbService
  projectionSchemaFingerprint: sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
  factoryFingerprint: sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb
  operations: [usbip-control, usbip-data]
  arbitration: exclusive
  quota:
    maxConsumers: 1
    fairness: fifo
    leaseDeadlineMs: 15000
  consumerZonePolicy:
    relation: child-zones
    capabilityCeiling: [usbip-control, usbip-data]
  visibility: child-zones
```

`resourceRef` MUST target an authority-mode `UsbService`. Import materializes a
same-qualified-type `usb.d2bus.org.UsbService` projection; it never substitutes
a Provider-qualified type. Admission rejects Device, Endpoint, `UsbBinding`,
projection Service, and internal session/transfer handles as export targets.
The Provider capability manifest advertises only `UsbService` as
`explicit-export`.

Across a ZoneLink, USBIP control and data use bounded, credit-controlled,
cancellable ComponentSession named streams under enrolled Noise_KK. Transport
Endpoint, server/client, proxy, port, session, and frame details remain
Provider-internal. No fd, DeviceGrant, busid/sysfs path, address, socket path,
or credential crosses a Zone. Session, lease, transfer, and stream handles are
never ResourceTypes. Revocation cancels the stream, detaches the Guest, and
degrades the same-type local Service projection and Binding.

---

## Controller Process resource

The framework creates the controller state Volume **before** starting the
controller Process. This is done by core **ProviderDeployment**, not by the
semantic `device-usbip` controller. The `device-usbip` controller does not own
Volumes, does not add Volume to its exported ResourceTypes, and does not create
its own prerequisite. The controller Process spec carries a mandatory `mounts`
entry that references that Volume; the runtime rejects the Process launch if
the Volume is not Ready.

```yaml
apiVersion: resources.d2bus.org/v3
type: Process
metadata:
  name: device-usbip--controller
  zone: work
  ownerRef: Provider/device-usbip
spec:
  providerRef:  Provider/system-minijail
  executionRef: Host/host-system          # from Provider.spec.config.controllerExecutionRef
  domain:       system
  processClass: controller
  template:     controller-main           # plain ID from signed Provider component descriptor
  sandbox:
    namespaceClasses: [mount]
    capabilityClasses: []
    seccompClass: strict
    noNewPrivileges: true
    startRoot: false
    environmentClass: minimal
    readOnlyRoot: true
  budget:
    cpu:
      request: "100m"
      limit: "1000m"
    memory:
      request: "32Mi"
      limit: "128Mi"
    pids:
      limit: 128
    fds:
      limit: 256
  networkUsage:  null
  readiness:
    class: ready-condition
    initialDelay: "0s"
    timeout: "10s"
    failureThreshold: 3
  restartPolicy:
    class: on-failure
    backoffBase: "2s"
    backoffMax: "60s"
    backoffMultiplier: 2.0
    maxRestarts: null
    resetAfter: "300s"
  adoptionPolicy: adopt-on-restart
  drainTimeout: "10s"
  mounts:
    - volumeRef: Volume/device-usbip--controller--state--host-system
      view:      state
      mountPath: /state
      access:    read-write
      required:  true
```

The `template: controller-main` field binds to the signed component descriptor
inside the Provider package closure. No caller-controlled executable path,
argv, UID/GID, host path, capability, or broker op escapes this schema.

---

## Worker Process resources

Only **real long-lived processes** become Process resources. Ephemeral
operations (kernel module load, device withhold, busid bind/unbind, lock
acquisition) are **semantic EffectPort steps** executed through the injected
`UsbipEffectPort`; they are NOT EphemeralProcess resources.

The controller reconciles three closed worker classes:

| Worker | Cardinality/owner | Endpoint | Network behavior |
| --- | --- | --- | --- |
| `usbip-host-<host-uid-short>-backend` | exactly one per Host; Provider-owned Host authority | one provider-internal host-local backend Endpoint | no externally reachable listener; all busids register with this backend |
| `usbip-net-<network-uid-short>-relay` | exactly one per Network; Provider-owned relay authority | `Endpoint/<network>-usbip-relay` | binds only that Network's uplink on TCP 3240 and multiplexes all admitted Services |
| `usbip-binding-<binding-uid-short>-proxy` | one per attached `UsbBinding`; Binding-owned | `Endpoint/<binding>-usbip-private` | runs for the exact Guest and exposes only Guest-private loopback TCP 3240 |

Every Process uses `Provider/system-minijail`, a signed template
(`usbip-daemon`, `usbip-relay`, or `usbip-guest-proxy`), semantic sandbox
classes, bounded CPU/memory/pid/fd budgets, provider-defined readiness,
`adopt-on-restart`, and bounded drain. No Process spec contains argv, an
executable or socket path, a busid, an IP address, an interface name, or an fd.
The Host backend and per-Network relay are shared authorities, not per-Device
workers. The Binding proxy is the only per-consumer long-lived worker.

The Network relay Endpoint has:

```yaml
apiVersion: resources.d2bus.org/v3
type: Endpoint
metadata:
  name: work-net-usbip-relay
  zone: work
  ownerRef: Provider/device-usbip
spec:
  providerRef: Provider/device-usbip
  producerRef: Process/usbip-net-a09c3f218ee1-relay
  endpointClass: service
  transport: tcp
  purpose: device-usbip.d2bus.org/relay
  serviceFingerprint: device-usbip.d2bus.org/UsbipRelay.v4
  locality: cross-domain
  visibility: authorized-consumers
  attachmentPolicy: launch-ticket-only
  consumerPolicy: device-usbip.d2bus.org/leased-service-and-binding-proxy
  lifecyclePolicy: recycle-with-producer
  authority:
    authorityScope: host
    authorityKey: usbip-network-relay/v1
    cardinality: exactly-one
    arbitration: multiplexed
    authorityRef: Endpoint/work-net-usbip-relay
    duplicateConflict: usbip-network-relay-authority-conflict
    ownerProof: endpoint-and-relay-process-identity
    updateStrategy: drain-recycle
    exportability: forbidden
    quota:
      maxServices: 16
      fairness: fifo
```

Core derives the relay key from the Network UID and signed standard-port policy;
neither the Provider nor an operator supplies the digest or port. This
`Endpoint` is the single D097 relay/listener/firewall authority for that Network.
Every admitted Service is a holder of this multiplexer, never a second listener
or firewall owner.

The per-Binding private Endpoint has `ownerRef:
usb.d2bus.org.UsbBinding/<name>`, names the Binding-owned Guest proxy as
`producerRef`, uses purpose `device-usbip.d2bus.org/guest-attachment`, is
`guest-local`, and permits only the exact `guestRef`. Neither Endpoint is an
export target. Producer restart increments `endpointGeneration`; dependent
Services or Bindings reconnect through fresh authorized LaunchTickets.

## Endpoint resources (D092)

`Provider/device-usbip` conforms to the standard `Endpoint` base schema. Stable
USBIP attach/relay identities that can be independently consumed are owned
`Endpoint` resources with `producerRef`; consumers use `Endpoint/<name>`.
Endpoint spec/status never carries raw bind addresses, bus IDs, CIDs, ports,
paths, fds, or credentials. Resolution occurs only through an authorized
EffectPort/LaunchTicket; unauthorized resolution returns `endpoint-resolve-denied`.
Producer restart bumps `Endpoint.status.endpointGeneration`, causing consumers to
observe `dependency-changed` and reconnect through a fresh authorized ticket.
The relay Endpoint may be resolved only by the device-usbip Core adapter and an
exact Binding-owned proxy with an active Service lease. Network controllers,
generic Network readers, unrelated Services, and arbitrary same-Zone Processes
have no resolve permission. Binding proxies receive an already-authorized
connected stream through their LaunchTicket, so network-local needs no generic
TCP/3240 host or net-VM allow.

## Retained opaque handles (D092 promotion test)

- pidfds for backend/relay/Binding-proxy supervision stay process-local because they are
  restart-time identity handles, not stable managed endpoint identities.
- LaunchTicket fd indexes and inherited listener fds stay opaque; they are
  per-launch attachment slots resolved under authorization.
- Per-busid attach connection handles, `LeaseToken`, `FirewallToken`, and OFD
  lock fds stay opaque because they are high-churn effect-port capabilities tied
  to one operation or lease.
- `operationId` and committed-revision proofs stay opaque correlation/idempotency
  handles in the core Operation ledger.
- `OwnedTransport`/ComponentSession transport handles stay in-memory transport
  capabilities behind Endpoint resolution, not addressable resources.

### Guest-side attachment effects

Guest-local USBIP effects (`vhci_hcd` module load, `usbip attach`, `usbip
detach`) go through a **guest-side `UsbipGuestEffectPort`** adapter injected
into the controller when the Binding's `guestRef` supervisor is addressed. The
Guest supervisor (e.g., `Provider/runtime-cloud-hypervisor`) owns this
adapter and exposes it to the controller through the reconcile framework's
per-Guest effect channel.

The controller calls `guest_effect.attach(binding_uid)` or
`guest_effect.detach(binding_uid)` semantically. The Guest supervisor
adapter privately:

1. Locates the Guest-side `usbip` binary from the signed bundle.
2. Resolves the Binding-owned private Endpoint and issues the attach/detach
   command via its privileged guest-control channel.
3. Returns a typed `UsbipGuestEffectError` to the controller.

A one-shot guest-side command is neither a long-lived worker nor an
EphemeralProcess resource. The Binding-owned Guest proxy is a normal long-lived
Process; the attach/detach command itself is not a second Process resource.
The guest-side `vhci_hcd` kernel module is wired at Guest build time via
`nixos-modules/components/usbip.nix`.

### What is NOT a Process resource

The following operations are **not** EphemeralProcesses and not long-lived
Process resources. They are semantic steps executed through injected EffectPorts:

- `modprobe usbip-host` — `EnsureKernelModule` host EffectPort step
- Device withhold (sysfs `authorized = 0`) — `WithholdDevice` host EffectPort step
- `usbip bind --busid <id>` — `BindBusid` host EffectPort step
- `usbip unbind --busid <id>` — `UnbindBusid` host EffectPort step
- Per-busid OFD lock acquisition / release — `AcquireLease` / `ReleaseLease` host EffectPort steps
- nftables firewall carve-out — `ApplyFirewall` / `ReleaseFirewall` host EffectPort steps
- Guest-side `usbip attach` / `usbip detach` — `UsbipGuestEffectPort` steps

The host adapter and guest supervisor adapter execute these internally.
No EphemeralProcess resource is created for any of these operations.

---

## UsbipEffectPort — injected semantic port

The `UsbipEffectPort` trait is defined in `d2b-contracts` (or
`d2b-provider-toolkit`). The Provider controller receives an
`Arc<dyn UsbipEffectPort>` through its reconcile context at construction time.
The controller calls semantic methods with validated opaque IDs only.

```rust
use std::fmt;

/// Opaque validated identifiers that cross the EffectPort boundary.
/// Custom Debug impls redact bytes; `derive(Debug)` is intentionally absent
/// to prevent byte arrays from leaking into logs or error output.
pub struct DeviceUid([u8; 32]);
pub struct NetworkUid([u8; 32]);
pub struct UsbBindingUid([u8; 32]);
/// Zeroized on drop; Debug is redacted.
pub struct LeaseToken([u8; 32]);
pub struct FirewallToken([u8; 16]);
pub struct FirewallObservation {
    pub matches_expected: bool,
    pub digest: [u8; 32],        // opaque ownership-scoped digest
    pub observed_generation: u64,
}

impl fmt::Debug for DeviceUid    { fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { write!(f, "DeviceUid(<redacted>)") } }
impl fmt::Debug for NetworkUid   { fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { write!(f, "NetworkUid(<redacted>)") } }
impl fmt::Debug for UsbBindingUid{ fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { write!(f, "UsbBindingUid(<redacted>)") } }
impl fmt::Debug for LeaseToken   { fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { write!(f, "LeaseToken(<redacted>)") } }
impl fmt::Debug for FirewallToken{ fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { write!(f, "FirewallToken(<redacted>)") } }

// Opaque UID/token Clone impls are manual (not derived) to avoid
// clippy::expl_impl_clone_on_copy on the non-Copy structs; this also prevents
// accidental derive(Debug) via derive macro chains.

/// Kernel module class (non-exhaustive; additions are non-breaking).
#[non_exhaustive]
#[derive(Debug, Clone)]
pub enum KernelModuleClass {
    UsbipHost,
}

/// Device probe result. Fields are opaque: `label` is a logical label digest,
/// not a raw busid; `present`/`anti_spoof` are boolean outcomes only.
#[derive(Debug, Clone)]
pub struct DeviceProbeResult {
    pub present:    bool,
    pub anti_spoof: bool,
    pub label:      String,   // opaque logical label; never raw busid or path
}

/// Closed-set error enum. No String payload that could carry raw paths, busids,
/// or broker wire details. Retriability is inferred from the variant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UsbipEffectError {
    KernelModuleLoadFailed,
    DeviceNotPresent,
    AntiSpoofFailed,
    LeaseDenied,
    LeaseNotHeld,
    FirewallDenied,
    FirewallForeignConflict,
    WrongZone,
    BackendNotReady,
    BindFailed,
    UnbindFailed,
    WithholdFailed,
    ReleaseWithholdFailed,
    GuestEffectUnavailable,
    /// Transient adapter-internal error. The bounded detail string is
    /// redacted in Display/Debug to prevent log leakage; use the
    /// structured span attribute `error_class` for observability.
    Transient(TransientDetail),
}

/// Bounded redacted detail for transient errors. Stored in the adapter's
/// internal log only; redacted in all Debug/Display impls so it cannot leak
/// into spans, error messages, or audit fields.
#[derive(Clone, PartialEq, Eq)]
pub struct TransientDetail(Box<str>);  // bounded; no raw paths
impl fmt::Debug   for TransientDetail { fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { write!(f, "<redacted>") } }
impl fmt::Display for TransientDetail { fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { write!(f, "<redacted>") } }

#[async_trait]
pub trait UsbipEffectPort: Send + Sync {
    /// Ensure `usbip-host` kernel module is loaded.
    async fn ensure_kernel_module(
        &self,
        device_uid: &DeviceUid,
        module_class: KernelModuleClass,
    ) -> Result<(), UsbipEffectError>;

    /// Probe physical device presence and return presence/label/anti-spoof status.
    /// The adapter reads sysfs using its own bundle-validated busid; the controller
    /// never receives the raw busid.
    async fn probe_device(
        &self,
        device_uid: &DeviceUid,
    ) -> Result<DeviceProbeResult, UsbipEffectError>;

    /// Acquire an exclusive per-device lease. Returns an opaque token.
    /// The adapter acquires the OFD lock internally; the lock fd or path
    /// never leaves the adapter.
    async fn acquire_lease(
        &self,
        device_uid: &DeviceUid,
    ) -> Result<LeaseToken, UsbipEffectError>;

    /// Release a previously acquired lease.
    async fn release_lease(
        &self,
        device_uid: &DeviceUid,
        token: LeaseToken,
    ) -> Result<(), UsbipEffectError>;

    /// Withhold the physical device from OS auto-claim (sysfs authorized=0).
    /// Requires a valid lease token.
    async fn withhold_device(
        &self,
        device_uid:  &DeviceUid,
        lease_token: &LeaseToken,
    ) -> Result<(), UsbipEffectError>;

    /// Re-allow OS auto-claim (sysfs authorized=1).
    async fn release_device_withhold(
        &self,
        device_uid:  &DeviceUid,
        lease_token: &LeaseToken,
    ) -> Result<(), UsbipEffectError>;

    /// Acquire a reference to the Network relay's nftables carve-out.
    /// The adapter validates that device_uid and network_uid are in the same Zone
    /// before dispatching UsbipBindFirewallRule through the privileged broker.
    /// Wrong-Zone returns WrongZone immediately without any host mutation.
    async fn apply_firewall(
        &self,
        device_uid:  &DeviceUid,
        network_uid: &NetworkUid,
        lease_token: &LeaseToken,
    ) -> Result<FirewallToken, UsbipEffectError>;

    /// Observe the exact per-Network/per-busid USBIP ownership projection.
    /// Network-local rules are excluded. The digest is opaque and may be
    /// written only to the owning Service's strict provider status.
    async fn observe_firewall(
        &self,
        device_uid:    &DeviceUid,
        network_uid:   &NetworkUid,
        firewall_token: &FirewallToken,
    ) -> Result<FirewallObservation, UsbipEffectError>;

    /// Remove the nftables carve-out.
    async fn release_firewall(
        &self,
        device_uid:     &DeviceUid,
        firewall_token: FirewallToken,
    ) -> Result<(), UsbipEffectError>;

    /// Bind the physical device to usbip-host (kernel-level).
    /// Requires the shared Network firewall/relay and Host backend Ready.
    async fn bind_busid(
        &self,
        device_uid:  &DeviceUid,
        lease_token: &LeaseToken,
    ) -> Result<(), UsbipEffectError>;

    /// Unbind the physical device from usbip-host.
    async fn unbind_busid(
        &self,
        device_uid:  &DeviceUid,
        lease_token: &LeaseToken,
    ) -> Result<(), UsbipEffectError>;
}

/// Guest-side effect port injected by the Guest supervisor when a UsbBinding
/// is associated with a specific Guest. The controller calls semantic
/// methods; the adapter issues commands through the Guest supervisor's
/// privileged guest-control channel.
#[async_trait]
pub trait UsbipGuestEffectPort: Send + Sync {
    /// Resolve the private Endpoint and trigger `usbip attach` in the Binding's Guest.
    async fn attach(&self, binding_uid: &UsbBindingUid) -> Result<(), UsbipEffectError>;
    /// Trigger `usbip detach` inside the Binding's Guest.
    async fn detach(&self, binding_uid: &UsbBindingUid) -> Result<(), UsbipEffectError>;
}
```

### Adapter responsibilities (framework-internal, NOT in Provider crate)

The core adapter that implements `UsbipEffectPort` is owned by the framework
(not the Provider crate). It:

1. Looks up the raw busid from the Device UID through the trusted signed bundle
   (never from the Device spec payload; the spec contains only logical labels).
2. Validates that the Device and Network UIDs belong to the same Zone
   (`device_uid.zone() == network_uid.zone()`); returns `WrongZone` immediately
   if they differ.
3. Acquires / releases the per-busid OFD lock at
   `/run/d2b/locks/usbip/<busid>` — adapter-internal; no controller visibility.
4. Dispatches `UsbipBindFirewallRule` through the privileged broker (the sole
   privileged executor) for the complete per-Network/per-busid TCP/3240
   exposure. No generic network-local allow is a prerequisite.
5. Records the broker/core audit record **post-effect** (after the host mutation
   completes); audit is not atomic with resource store commits.
6. Reads the same ownership projection for drift through `observe_firewall`;
   Network-local ownership is excluded.
7. Returns typed `UsbipEffectError` variants to the controller; no broker wire
   types, lock paths, audit structs, or nftables details leak.

The audit record emitted by the adapter contains: zone, device name-digest,
network name-digest, operation tag, outcome, error_class, and correlation_id.
No raw busid, lock path, nftables body, or vendor/product ID appears in audit.

---

## Bring-up sequence

The authority Service preserves the safety ordering from
`packages/d2bd/src/usbip_state_machine.rs` while sharing Host and Network
singletons:

```
host-module/backend → physical-lock → withhold → network-firewall/relay
                    → bind-busid → service-ready
```

| Step | EffectPort call / resource action | Authority rule |
| --- | --- | --- |
| Host module/backend | `ensure_kernel_module` and adopt/create the Host backend Process | exactly one per Host; idempotently shared |
| Physical lock | `acquire_lease(device_uid)` | exclusive OFD claim for the Service's local Device |
| Withhold | `withhold_device(device_uid, token)` | requires the exclusive token |
| Network firewall/relay | reference-counted `apply_firewall`, `observe_firewall`, and adopt/create the Network relay Process/Endpoint authority | exactly one TCP 3240 multiplexer and firewall authority per Network; exact per-busid rules |
| Bind busid | `bind_busid(device_uid, token)` | requires backend and relay Ready |
| Service Ready | commit Service authority status | no Guest is attached yet |

An attached Binding then reconciles:

| Step | Action |
| --- | --- | --- |
| Acquire Service slot | fair exclusive admission; imported Services also acquire a D096 lease |
| Create proxy/Endpoint | Binding-owned Guest proxy Process and private Endpoint become Ready |
| Attach Guest | `UsbipGuestEffectPort::attach(binding_uid)` targets the private Endpoint |
| Commit status | Binding becomes Ready with the exact observed Service/Endpoint generations |

Binding teardown is Guest detach → private Endpoint/proxy deletion → Service slot
release. Service deletion first drains all Bindings and exports/imports, then
unbinds the busid, releases firewall/relay references, releases withhold and
physical lock, and finally releases Host backend/module references. Shared Host
or Network authorities remain while another Service references them.

Each observe cycle calls `observe_firewall` for the Service's exact
Network/busid claim. A mismatch changes only the USB Service's strict provider
firewall state, degrades that Service, and queues device-usbip reconcile.
Network `FirewallReady` and its digest do not change.

Every step is idempotent. Completed authority steps live in Service status;
completed attachment steps live in Binding status. Physical Device status never
stores a Guest attachment state.

---

## Typed Device, Service, and Binding status

Per D088, ResourceType-common Device observation lives in `status.resource`.
`UsbService` base status contains only generic whole-device availability and
access observations; `UsbBinding` base status contains only generic attachment
phase, queue position, and observed Service generation. A direct-local Provider
would populate the same base fields. USBIP module, backend, Network, Endpoint,
proxy, stream, server/client, port, firewall, and busid observations exist only
in strict `status.provider`. Every Provider extension is signed, bounded to
32 KiB, unknown-field-denied, and may not duplicate common status fields.

```yaml
status:
  phase: Ready
  observedGeneration: 1
  conditions:
    - type: BackingAuthorityReady
      status: "True"
      reason: physical-usb-backing-claimed
  resource:
    access:
      mode: exclusive
      available: true
      activeBindings: 0
    backingAuthority:
      authorityScope: physical-device
      authorityClass: physical-usb-backing
      claimed: true
  provider:
    providerRef: Provider/device-usbip
    schemaId: "device-usbip.d2bus.org/UsbService/status"
    schemaVersion: "1.0.0"
    observedProviderGeneration: 1
    details:
      mode: authority
      authorityAvailable: true
      completedSteps: [host-backend, physical-lock, withhold, network-relay, bind]
      backendProcessRef: Process/usbip-host-b3a7f1d2c591-backend
      relayEndpointRef: Endpoint/work-net-usbip-relay
      hostModuleAuthority: ready
      networkRelayAuthority: ready
      firewallState: ready                  # ready|drifted|applying|releasing
      firewallDigest: "<opaque-hex-sha256>" # exact USBIP ownership projection
      observedFirewallGeneration: 1
      labelDigest: "a3f7..."
```

Projection Service `status.resource` uses the same availability/access shape
plus common import lease state and remote generation/fingerprint, but omits the
authority-only backing claim. `status.provider` reports only internal route
readiness while omitting physical and owner-Endpoint fields. Binding
`status.resource`
reports `attachmentPhase`
(`detached|waiting|attaching|attached|detaching|failed|unknown`), observed
Service generation, bounded queue position, last closed error, and
attach/detach timestamps. USBIP-only proxy subphases, relay generation, and
private Process/Endpoint refs are confined to Binding `status.provider`. No
status layer carries a busid, host path, fd, remote address, stream handle,
session identifier, or transfer bytes.

`firewallState`, `firewallDigest`, and `observedFirewallGeneration` are owned
and written only by `Provider/device-usbip`. They never appear in Network status
or provider-neutral USB status. Drift sets `firewallState: drifted`, degrades
the affected authority Service with closed error class `firewall-drift`, and
requeues `apply_firewall`; no raw rule or identity enters status.

A shared backing collision sets `BackingAuthorityReady=False`, reason
`physical-usb-backing-conflict`, and phase `Failed` before any effect. That
condition and the claim state are provider-neutral USB Service observations;
USBIP module/relay diagnostics remain only in `status.provider`.

### Phase semantics

| Phase | Meaning |
| --- | --- |
| `Pending` | Service authority/projection or Binding attachment is reconciling or fairly queued |
| `Ready` | Service is available, or Binding proxy/Endpoint and Guest attachment are Ready |
| `Degraded` | Previously Ready dependency, authority, import, stream, proxy, or attachment is unavailable |
| `Failed` | Terminal failure; manual intervention required |
| `Unknown` | Controller unreachable or crashed; last known status stale |

**Currency and upgrade (D091).** The controller implements `assess_update`,
`plan_upgrade`, and `execute_upgrade` for Service authority/projection and
Binding attachment realizations and populates only universal `status.update` with
`state: Current|UpdateAvailable|UpgradeRequired|Upgrading|Blocked|Unknown`,
`reasons` from `CoreGenerationChanged`, `ProviderGenerationChanged`,
`ArtifactChanged`, `ImageOrSystemGenerationChanged`, `SpecChanged`,
`DependencyChanged`, or `SecurityPolicyChanged`, observed/target
generation/digest IDs, `disruption: None|Reload|Restart|Recycle|Replace`,
`preserveState`, optional `operationId`, `lastAssessedAt`, and
`owned`/`dependencies` refs. It honors base `spec.updatePolicy` (manual
disruptive default; auto non-disruptive), while the Core Operation ledger owns
upgrade operation, idempotency, and progress. Disruptive attach changes return
`UpgradeRequired` rather than applying in place; the planner recycles the attach
realization, drains/restarts dependent Processes and Guests, and preserves
Device and Service identity. A Service upgrade drains dependent Bindings before
recycling the authority; an imported source update propagates import →
projection Service → Binding. Non-disruptive changes reconcile normally.

---

## Declared vs explicit attachment activation

### `attachmentPolicy.activation: declared`

The controller automatically acquires the Service slot, creates the
Binding-owned Guest proxy/private Endpoint, and invokes
`UsbipGuestEffectPort::attach` when `attachmentPolicy.desired: attached` and the
Service, Guest, and local Network are Ready.

Claim source: `UsbipClaimSource::Declared { device_uid, network_uid }` —
tracked in the Binding's bounded status; never exposed to the Guest.

### `attachmentPolicy.activation: explicit`

The authority Service may become Ready, but the Binding pauses before Service
slot acquisition until an operator invokes `d2b device usb attach
<binding-name>`. `AttachBinding` then creates the Binding-owned proxy/private
Endpoint and calls `guest_effect.attach(binding_uid)`. The attach command is an
internal EffectPort operation; the proxy is the Binding-owned long-lived Process.

Claim source: `UsbipClaimSource::Explicit` — tracked in
the Binding's bounded status.

---

## Exclusivity and cardinality

- `accessPolicy.mode: exclusive` and `maxActiveBindings: 1` are enforced by the
  Service authority, including imported consumers.
- A pending second Binding waits in a bounded fair queue until the first releases
  its lease; it never creates a second physical bind.
- The shared Host-global backing-authority index enforces one physical USB
  owner across all USB transport and security-key Providers before an external
  effect. The USBIP adapter separately enforces one busid owner.
- Nix preflights duplicate trusted selectors, but the shared Host-global
  backing-authority index is authoritative across all Zones and API-created
  resources. USBIP, direct-local USB, and security-key Providers must submit
  the exact `(Host, physical-usb-backing, opaqueKeyDigest)` tuple derived by
  Core under the `physical-usb-backing/v1` domain after trusted identity
  resolution; no Provider-private label, class, or digest can satisfy or bypass
  this invariant. `nixos-modules/assertions.nix` rejects a known
  duplicate before activation:
  ```
  assertion: uniqueHostGlobalUsbBackingAuthority allUsbAndSecurityKeyResources
  message: "physical USB backing '<digest>' has multiple Host-global owners"
  ```

---

## Async reconcile loop design

The controller's async reconcile loop follows
`ADR-046-resource-reconciliation` steps 10–11:

```
┌─────────────────────────────────────────────────┐
│  Dedicated watch receiver (async task)           │
│  continuously reads hints and dispatches:        │
│    hint → resource idle? → spawn per-resource   │
│             task (non-blocking)                  │
└───────────────┬─────────────────────────────────┘
                │  per-resource tasks run concurrently
                ▼
┌─────────────────────────────────────────────────┐
│  Per-resource reconcile task                     │
│  1. Read fresh Service/Binding + dependencies     │
│  2. Compute desired step delta                   │
│  3. Execute EffectPort steps (await each)        │
│  4. Create/delete child Process resources        │
│  5. Commit ResourceMutationBatch + status        │
└─────────────────────────────────────────────────┘
```

**The watch receiver loop MUST continue reading and dispatching during steps 3
and 4.** Independent Services and Bindings run concurrently under bounded
semaphores; per-Service single-flight serializes authority/attachment
arbitration. The receiver never blocks waiting for an effect step.

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

On restart, the controller:
1. Lists all authority/projection Services and Bindings it owns and reads their
   referenced Device, Host, Network, Guest, import, and Endpoint dependencies.
2. Reads completed authority steps from Service status and attachment steps
   from Binding status.
3. Skips completed steps idempotently.
4. Adopts exact Host backend, Network relay, and Binding proxy Processes using
   their authority owner proofs (`adoptionPolicy: adopt-on-restart`).
5. Quarantines ambiguous authority, otherwise resumes from the earliest
   non-completed step.

---

## Network dependency handling

The controller uses `ResourceClient` dependency watches (read-only) on the
Network identified by each authority Service's USBIP provider
`relayEndpointRef` and on each Binding's USBIP provider `networkRef`. Neither
reference is a base `UsbService`/`UsbBinding` field. The controller never
contacts the Network controller directly; it observes Network status changes
via the watch.

The `ResourceClient` dependency watch is the **only** channel for Network
information. No broker connection for Network data; no route table access;
no direct NetworkManager/nftables query.

Fields read from Network status by the controller (read-only, via ResourceClient watch):
- `status.phase` — gates Network relay and Binding proxy creation until Ready

The adapter supplies the Network UID to Core's private fabric resolver when
constructing the shared relay or executing firewall effects. Core derives the
host uplink attachment and address from private Network realization state; no
raw field is read from public Network status, and the controller never reads or
holds a raw address or IfName.

The Network controller does **not** own the USBIP firewall carve-out.
`D-NETWORK-002` in `ADR-046-resources-network` confirms:
`device-usbip` owns the firewall semantic authority. The `apply_firewall`
EffectPort step is the only path that creates or removes the nftables rule;
the Network controller must not create, remove, or reference USBIP firewall
rules. `observe_firewall` is the only USBIP drift reader, and strict USBIP
provider status is the only status authority. Network-local emits no generic
TCP/3240 host or net-VM allow and excludes device-usbip markers from its digest.

---

## Finalizer lifecycle

The controller adds `device-usbip.d2bus.org/service-finalizer` to an authority
Service after its first host effect and
`device-usbip.d2bus.org/binding-finalizer` to a Binding after acquiring its Service
slot. A projection Service's lifecycle is owned by its `ResourceImport`. No
finalizer is added before an effect or lease exists.

Teardown on deletion request:
1. Controller detects `deletionTimestamp` set.
2. For a Binding, detaches the Guest, drains/deletes its private Endpoint/proxy,
   and releases the Service/import lease.
3. For an authority Service, drains dependent Bindings and exports, unbinds its
   busid, releases its relay/firewall/backend references, withhold, and physical
   lease; shared authorities remain until their last reference is gone.
4. Marks teardown progress in the deleting Service or Binding status; restart is
   idempotent.
5. Clears the finalizer only after all teardown steps succeed.
6. Core commits the finalizer removal; the resource is garbage-collected.

If a teardown step fails terminally, the controller sets `phase: Degraded` and
the typed Service/Binding `teardownBlocked: true`, emits a structured event, and
requeues under exponential backoff.

---

## ProviderStateSet

A **ProviderStateSet** is the optional, query-time set of the *declared* Volume
resources in a Zone whose `metadata.ownerRef` resolves to `Provider/device-usbip`.
It is a query-time grouping, not a ResourceType or stored artifact, and is empty
for a Provider that declares no state Volume:

```text
ProviderStateSet(zone, "device-usbip") =
  { v : Volume | v.metadata.zone == zone
              && v.metadata.ownerRef == "Provider/device-usbip" }
```

`Provider/device-usbip` declares **no** Provider state Volume; its
`ProviderStateSet` is empty. The controller has no durable payload state beyond
Device/Service/Binding resource status, owned Process/Endpoint resources, and the
core Operation ledger. Bounded non-secret authority state lives in Service
status; per-Guest attach/detach state lives in `UsbBinding` status; common
Device presence/probe state stays in Device `status.resource` (D087/D088).
Sessions, transfers, leases, and stream handles remain internal and are not
persisted as resources.

Because this Provider's operational state is fully derivable from spec,
`status`, the core Operation ledger, and independent external observation
(running attach Processes re-derived from cgroup leaves, fresh pidfds), it fails
the storage-need test and declares no state namespace, no state Volume, no
state-view mount, and no dedicated state-layout `User/<name>` principal. There
is no empty identity-only Volume.

The backend, relay, and Binding proxy workers likewise hold no state Volume;
stateless workers receive no dirfd. The stable identity of an adopted Process is
re-derived after restart from its declared cgroup leaf and a fresh pidfd, not
from any persisted snapshot.

---

## Audit separation and OTEL telemetry

### Audit (broker/core)

The privileged broker emits an audit record **after** each `UsbipBindFirewallRule`
effect completes. The core resource store emits its own audit record after each
`ResourceMutationBatch` commit. These two audit records are independent events;
the Provider controller does not own, submit, or claim ownership of either.

**The Provider controller must never assert that its own status update is
atomic with a broker audit record.** They are separate commits on separate
subsystems.

Each broker-emitted audit record contains:

| Field | Value |
| --- | --- |
| `subject` | Provider Process identity digest |
| `zone` | Zone name |
| `op` | `UsbipBindFirewallRule` (or release variant) |
| `resource_type` | `usb.d2bus.org.UsbService` |
| `resource_name_digest` | Stable hash of Service name; never raw busid or path |
| `outcome` | `success \| failure \| denied` |
| `error_class` | closed-set slug |
| `correlation_id` | operation/trace ID from reconcile context |
| `timestamp` | RFC 3339 UTC |

No raw busid, lock path, nftables rule body, vendor/product ID, or network
interface name appears in any audit record field.

### OTEL telemetry (Provider controller)

The controller emits OTEL spans for its own reconcile operations:

| Span name | Attributes |
| --- | --- |
| `device-usbip.service.reconcile` | `zone`, `service.name_digest`, `mode`, `phase`, `trigger_reason` |
| `device-usbip.binding.reconcile` | `zone`, `binding.name_digest`, `phase`, `trigger_reason` |
| `device-usbip.effect.ensure_kernel_module` | `outcome`, `error_class` |
| `device-usbip.effect.acquire_lease` | `outcome`, `error_class` |
| `device-usbip.effect.withhold_device` | `outcome`, `error_class` |
| `device-usbip.effect.apply_firewall` | `outcome`, `error_class`, `zone_match: bool` |
| `device-usbip.effect.release_firewall` | `outcome`, `error_class` |
| `device-usbip.effect.bind_busid` | `outcome`, `error_class` |
| `device-usbip.effect.unbind_busid` | `outcome`, `error_class` |
| `device-usbip.process.backend_start` | `outcome`, `error_class` |
| `device-usbip.process.relay_start` | `outcome`, `error_class` |
| `device-usbip.process.binding_proxy_start` | `outcome`, `error_class` |

Attributes must never carry raw busids, lock paths, nftables text, binary
paths, Endpoint addresses, import/export keys, session/transfer identifiers, or
operator/user identifiers. Cardinality is bounded: `error_class` is a closed
enum; resource names are fixed-length digests; `zone` is bounded by Zone
cardinality.

---

## d2b-bus methods

The controller registers these typed d2b-bus methods:

| Method | Authority | Description |
| --- | --- | --- |
| `AttachBinding` | Admin | Trigger Service-slot acquisition, proxy creation, and Guest attach for an explicit `UsbBinding`; progress is reflected in Binding status. |
| `DetachBinding` | Admin | Detach the Guest and release only that Binding realization; returns immediately. |
| `ProbeDevice` | Admin | Re-run physical probe; update `status.provider.details.usbip.lastProbeResult`. Returns synchronously. |
| `GetServiceStatus` | Admin, StatusReader | Return the bounded authority/projection status. |
| `GetBindingStatus` | Admin, StatusReader | Return bounded attachment status. |
| `ListServices` | Admin, StatusReader | Return local Service refs and logical name digests only. |

No bus method returns or accepts a raw busid, lock path, or broker wire type.
Sessions, transfers, stream handles, and import leases remain internal and have
no resource or general bus API.

---

## RBAC

| Role | ResourceType | Verbs | Scope | Granted to |
| --- | --- | --- | --- | --- |
| `device-reader` | Device | `get`, `list`, `watch`, `update/status` | Zone-scoped referenced physical Devices | device-usbip controller Process identity |
| `usbip-service-manager` | `usb.d2bus.org.UsbService` implemented by `Provider/device-usbip` | `get`, `list`, `watch`, `update/status`, `patch/finalizers` | Zone-scoped owned Services/projections | controller identity |
| `usbip-binding-manager` | `usb.d2bus.org.UsbBinding` implemented by `Provider/device-usbip` | all lifecycle/status/finalizer verbs | Zone-scoped owned Bindings | controller identity |
| `process-endpoint-owner` | Process, Endpoint | lifecycle verbs | exact Provider/Service/Binding-owned component identities | controller identity |
| `dependency-reader` | Host, Guest, Network, ResourceExport, ResourceImport | `get`, `list`, `watch` | exact same-Zone referenced resources | controller identity |

There is no direct broker channel in the RBAC table. The controller communicates
with the broker exclusively through the injected `UsbipEffectPort`; the
framework adapter holds the broker connection. The controller Process identity
does not have a direct broker credential.

No wildcard `*` over all Device resources. No cross-Zone Device access or
ResourceRef exists; the export/import adapter receives only signed D096
capabilities.
No role grants the controller access to raw nftables operations, lock files,
or other Device providers' resources.

---

## Security model

### Provider boundary

The `d2b-provider-device-usbip` crate has no compile-time or runtime access to:

- the privileged broker socket or any broker op DTOs;
- the OFD lock file at `/run/d2b/locks/usbip/<busid>`;
- raw busid strings (the adapter derives them from the signed bundle);
- nftables rule bodies or ownership-marker strings;
- the Network bridge interface name or host-side route table.

These are all adapter-internal. The Provider's security boundary is the
`UsbipEffectPort` interface; the adapter is the trust boundary between the
unprivileged Provider crate and the privileged broker.

### Same-Zone gating (wrong-Zone denial)

The adapter's `apply_firewall` implementation checks:

```
assert device_uid.zone() == network_uid.zone()
```

If they differ, `apply_firewall` returns `UsbipEffectError::WrongZone`
immediately with no host mutation. No nftables rule is written, no firewall
token is issued, and the controller transitions the authority Service to `Degraded` with
`error_class: wrong-zone`.

This invariant ensures a Device bound in Zone A cannot be reached by a
different local Zone without D096 export/import:
- The shared relay in Zone A binds only to the Zone A Network uplink IP.
- The firewall carve-out allows only the Zone A bridge.
- A wrong-Zone firewall request is rejected before any effect.
- Cross-Zone consumers terminate only the encrypted import stream at their
  local projection/Binding proxy; they receive no direct firewall opening.

The wrong-Zone case in `tests/host-integration/usbip-service.nix` MUST pass in
the required host-integration gate.

### Anti-spoofing

The adapter's `probe_device` implementation validates the physical device's
vendor/product/serial against the signed bundle's expected values. An
anti-spoofing mismatch returns `UsbipEffectError::AntiSpoofFailed` and the
Device transitions to `Failed` with `status.provider.details.usbip.lastProbeResult.antiSpoofFailed: true`.

The controller never uses the spec-level `vendorId`/`productId` fields for
anti-spoofing decisions; those are configuration-level filters only. The
authoritative values come from the signed bundle.

### Firewall ownership

`Provider/device-usbip` is the **semantic owner** of the USBIP nftables
carve-out. Only the `apply_firewall` and `release_firewall` EffectPort calls
may create or remove USBIP TCP/3240 exposure; `observe_firewall` is its only
drift reader. Network-local emits no host or net-VM USBIP allow and owns no
USBIP status. The ownership marker used in the nftables comment is constructed
and verified by the adapter from opaque Network and busid identity, not the
controller; no raw identity is embedded in the marker or surfaced in status.

The adapter enforces that:
- `apply_firewall` is reference-counted and idempotent for the same Network
  relay authority; multiple Services reuse one TCP 3240 listener and its
  Network-scoped base hook while retaining separate per-busid authorization
  entries;
- each Service still receives exact per-busid authorization under that
  per-Network relay; releasing one busid cannot retain or remove another's
  exposure;
- the closed `UsbipBindFirewallRule` operation installs every required
  host-ingress/coexistence hook atomically, so no generic uplink allow can
  bypass the per-busid decision;
- `observe_firewall` hashes only the exact device-usbip ownership projection
  and writes drift solely to the owning Service's strict provider status;
- a `release_firewall` with a token that does not match the installed rule
  returns `UsbipEffectError::FirewallDenied`;
- a foreign rule with the same ownership marker prefix causes the adapter to
  return `UsbipEffectError::FirewallForeignConflict` and emit a
  `foreign-rule-conflict` audit event.

### Relay and private proxy bind addresses

The shared relay binds to the per-Zone Network uplink IP only (not `0.0.0.0`).
The Binding-owned Guest proxy binds only Guest loopback. The
adapter derives this address through Core's private Network fabric resolver;
the controller never reads or passes an IP address. Endpoint resources declare
only stable semantic fields; the adapter resolves actual bind addresses
privately when Processes start. An exact Binding-owned proxy with an active
Service lease receives the connected relay stream through a LaunchTicket;
generic Network consumers cannot resolve the Endpoint.

---

## Nix configuration

### Operator declaration (Zone resource)

```nix
d2b.zones.work.resources.corp-vm-usb = {
  type = "Device";
  spec = {
    providerRef  = "Provider/device-usbip";
    deviceClass  = "physical";
    arbitration  = "exclusive";
    maxConcurrentClaims = 1;
    inventory.selector = {
      busClass  = "usb";
      label     = "yubikey-work";
      vendorId  = "1050";
      productId = "0407";
    };
    provider = {
      schemaId = "device-usbip.d2bus.org/Device/spec";
      schemaVersion = "1.0.0";
      settings = {};
    };
  };
};

d2b.zones.work.resources.work-token = {
  type = "usb.d2bus.org.UsbService";
  spec = {
    providerRef = "Provider/device-usbip";
    mode = "authority";
    backingDeviceRef = "Device/corp-vm-usb";
    accessPolicy = {
      mode = "exclusive";
      maxActiveBindings = 1;
      queue = {
        discipline = "fair";
        maxDepth = 16;
        acquireDeadline = "15s";
      };
    };
    backingAuthority = {
      authorityScope = "physical-device";
      authorityClass = "physical-usb-backing";
      keyDerivation = "core-trusted-usb-v1";
      cardinality = "zero-or-one";
      arbitration = "exclusive";
      authorityRef = "usb.d2bus.org.UsbService/work-token";
      duplicateConflict = "physical-usb-backing-conflict";
      exportability = "explicit-export";
    };
    provider = {
      schemaId = "device-usbip.d2bus.org/UsbService/spec";
      schemaVersion = "1.0.0";
      settings.relayEndpointRef = "Endpoint/work-net-usbip-relay";
    };
  };
};

d2b.zones.work.resources.corp-vm-work-token = {
  type = "usb.d2bus.org.UsbBinding";
  metadata.ownerRef = "Guest/corp-vm";
  spec = {
    providerRef = "Provider/device-usbip";
    serviceRef = "usb.d2bus.org.UsbService/work-token";
    guestRef = "Guest/corp-vm";
    accessPolicy = {
      mode = "exclusive";
      priority = 100;
    };
    attachmentPolicy = {
      desired = "attached";
      activation = "declared";
    };
    provider = {
      schemaId = "device-usbip.d2bus.org/UsbBinding/spec";
      schemaVersion = "1.0.0";
      settings.networkRef = "Network/work-net";
    };
  };
};
```

### Provider installation

```nix
d2b.zones.work.providers.device-usbip = {
  artifactId = "provider-device-usbip";
  config = {
    controllerExecutionRef = "Host/host-system";
  };
};
```

### Cross-Zone authoring

```nix
# Owner Zone. resourceRef is always UsbService, never Device/Endpoint/Binding.
d2b.zones.work.resources.work-token-export = {
  type = "ResourceExport";
  spec = {
    providerRef = "Provider/device-usbip";
    resourceRef = "usb.d2bus.org.UsbService/work-token";
    serviceType = "usb.d2bus.org.UsbService";
    projectionSchemaFingerprint = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    factoryFingerprint = "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    operations = [ "usbip-control" "usbip-data" ];
    arbitration = "exclusive";
    quota = {
      maxConsumers = 1;
      fairness = "fifo";
      leaseDeadlineMs = 15000;
    };
    consumerZonePolicy = {
      relation = "child-zones";
      capabilityCeiling = [ "usbip-control" "usbip-data" ];
    };
    visibility = "child-zones";
  };
};

d2b.zones.dev.resources.work-token-import = {
  type = "ResourceImport";
  spec = {
    providerRef = "Provider/device-usbip";
    zoneLinkRef = "ZoneLink/work";
    exportKey = "work-token";
    expectedServiceType = "usb.d2bus.org.UsbService";
    expectedProjectionSchemaFingerprint = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    expectedFactoryFingerprint = "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    projectionName = "work-token";
    requestedCapabilities = [ "usbip-control" "usbip-data" ];
    requestedQuota = { maxConsumers = 1; };
    disconnectPolicy = { behavior = "degrade"; };
  };
};

# Core creates the same-type UsbService/work-token with
# ownerRef=ResourceImport/work-token-import. The operator authors a dev-Zone
# UsbBinding with base Guest/service policy and USBIP network settings; the
# import controller never creates that Binding.
```

### Guest-side module wiring (unchanged from v3 baseline)

`nixos-modules/components/usbip.nix` remains under the guest's
`runtime-cloud-hypervisor` Nix module. It wires:
- `vhci_hcd` kernel module
- `usbip` CLI tools (from the Provider package closure via the guest bundle)
- `d2b.guestControl.usbipPath` for guest-side `usbip attach`

The old `d2b.vms.<vm>.usbip.yubikey = true` option is removed at the v3 reset
boundary. The new expression is Device + authority/projection Service +
per-Guest Binding, with explicit ResourceExport/ResourceImport only when crossing
a Zone.

### Eval-time assertions

```nix
# Mutual exclusion: every USB transport and security-key Provider uses the
# same Host-global backing-authority key derived from the physical selector.
assert uniqueHostGlobalUsbBackingAuthority allUsbAndSecurityKeyResources;

# controllerExecutionRef must resolve to a Host in the same zone
assert isValidHostRef cfg.providers.device-usbip.config.controllerExecutionRef;

# Base Service Device and Binding Service/Guest refs resolve in-Zone.
assert sameZoneRefs usbService;
assert sameZoneRefs usbBinding;
# USBIP Endpoint/Network refs are validated only in strict provider settings.
assert sameZoneUsbipProviderRefs usbService usbBinding;

# Only an authority UsbService is exportable.
assert export.resourceRef.type == "usb.d2bus.org.UsbService";
assert resolve(export.resourceRef).spec.mode == "authority";
```

---

## Errors

| Error class | Phase | Retryable | Notes |
| --- | --- | --- | --- |
| `unsupported-bus-class` | `Failed` | no | Only `usb` accepted for `busClass` |
| `invalid-vendor-id` | `Failed` | no | `vendorId` not exactly 4 ASCII hex digits |
| `invalid-product-id` | `Failed` | no | `productId` not exactly 4 ASCII hex digits |
| `invalid-selector-label` | `Failed` | no | Label violates ResourceName grammar |
| `network-ref-not-found` | `Degraded` | yes | USBIP provider `networkRef` or relay Endpoint Network does not resolve |
| `network-not-ready` | `Pending` | yes | Network dependency not yet Ready |
| `wrong-zone` | `Degraded` | no | A base or USBIP provider-extension reference resolves outside its Zone |
| `kernel-module-load-failed` | `Degraded` | yes | `ensure_kernel_module` returned error |
| `device-not-present` | `Degraded` | yes | Physical device absent from sysfs |
| `anti-spoof-failed` | `Failed` | no | Vendor/product/serial mismatch |
| `lease-denied` | `Degraded` | yes | `acquire_lease` failed; adapter contention |
| `withhold-failed` | `Degraded` | yes | sysfs write failed |
| `firewall-denied` | `Degraded` | yes | Adapter rejected `apply_firewall` |
| `firewall-foreign-conflict` | `Failed` | no | Foreign ownership marker at expected position |
| `firewall-drift` | `Degraded` | yes | Device-usbip ownership projection differs from expected Network/busid rules |
| `backend-start-failed` | `Degraded` | yes | Host backend authority failed to become Ready |
| `bind-failed` | `Degraded` | yes | `bind_busid` returned error |
| `relay-start-failed` | `Degraded` | yes | per-Network TCP 3240 relay failed to become Ready |
| `binding-proxy-start-failed` | `Degraded` | yes | Binding-owned Guest proxy/private Endpoint failed |
| `physical-usb-backing-conflict` | `Failed` | no | Another USB or security-key Service owns the Core-derived Host-global physical USB tuple; `BackingAuthorityReady=False` and no backing effect occurred |
| `usbip-host-module-authority-conflict` | `Failed` | no | A second Host module/backend authority was proposed |
| `usbip-network-relay-authority-conflict` | `Failed` | no | A second TCP 3240 listener was proposed for one Network |
| `invalid-export-target` | `Failed` | no | ResourceExport target is not an authority-mode `UsbService` |
| `import-revoked` | `Degraded` | yes | ResourceImport/ZoneLink lease was revoked |
| `stream-generation-mismatch` | `Degraded` | yes | Encrypted import stream generation/fingerprint is stale |
| `teardown-blocked` | `Degraded` | yes | One or more teardown steps failed |
| `claim-arbitration-conflict` | `Pending` | yes | Second Binding waits fairly for the exclusive Service slot |

---

## Current-code baseline reuse

| v3 baseline source | Reuse disposition | Notes |
| --- | --- | --- |
| `packages/d2b-contracts/src/usbip.rs` — `validate_bus_id`, `SYSFS_BUS_ID_MAX`, `UsbipClaimSource`, `sanitize_usb_hex_id` | Copy unchanged into `d2b-contracts`; reference from Provider crate | These are contracts, not broker internals; safe to reference |
| `packages/d2bd/src/usbip_state_machine.rs` — `CANONICAL_STEPS`, `UsbipBusidStep`, step ordering | Adapt step ordering into `src/reconcile.rs` EffectPort model; remove all broker-call sites | Step semantics and idempotency invariants preserved |
| `packages/d2bd/src/usbip_reconcile_state.rs` — desired/carrier/bind/proxy state enums | Split into typed Service authority and per-Guest Binding status | Restart-safe reconcile model preserved without putting attachment state on Device |
| `packages/d2b-host/src/usbip_argv.rs` — argv generators | Remain in `d2b-host`; called by the core adapter only | Provider crate has no compile dependency on `d2b-host` |
| `packages/d2b-priv-broker/src/ops/usbip_firewall.rs` — `bind_firewall_rule`, audit structs | Adapter-internal only; Provider crate never imports this | Audit structs are broker-internal; `UsbipBindFirewallRuleAudit` never visible to Provider |
| `packages/d2b-priv-broker/src/ops/usbip_host.rs` — `withhold_device` impl | Adapter-internal | Same as above |
| `packages/d2b-priv-broker/src/ops/usbip_lock.rs` — OFD lock | Adapter-internal | Lock fd never leaves adapter |
| `packages/d2b-contract-tests/tests/usbip_policy_network_scoping.rs` | Split into fast Provider `tests/wrong_zone.rs` admission coverage and real `tests/host-integration/usbip-service.nix` firewall coverage | Old duplicate retires only after both successors pass |
| `nixos-modules/components/usbip.nix` — guest vhci_hcd + tools | Unchanged; guest runtime module stays; host-side bits removed at v3 reset | Remains under runtime-cloud-hypervisor Guest module |
| `packages/d2bd/src/usbipd_perenv_autostart.rs` — per-env autostart | Delete; replace with one Host backend and one typed TCP 3240 relay Endpoint per Network | No per-env systemd unit and no per-Device port collision |

---

## Work items

### ADR046-usbip-001: `UsbipEffectPort` trait definition

| Field | Value |
| --- | --- |
| Dependency/owner | d2b-contracts crate shape stabilised by shared root contract; d2b-contracts owner |
| Current source | None — net-new v3 work; no pre-ADR45 baseline equivalent |
| Reuse action | create |
| Destination | packages/d2b-contracts/src/usbip_effect_port.rs |
| Detailed design | Define UsbipEffectPort and UsbipGuestEffectPort in d2b-contracts with DeviceUid, NetworkUid, UsbBindingUid, LeaseToken, FirewallToken, FirewallObservation, KernelModuleClass, DeviceProbeResult, and UsbipEffectError; export traits/types only with no implementation; keep firewall apply/observe/release Network/busid-scoped, attach/detach Binding-addressed, and all fd/path/busid values private. `TransientDetail` derives `Clone, PartialEq, Eq` while retaining manual redacted Debug/Display so `UsbipEffectError`'s derives compile without disclosure. Primary reuse disposition: `create`. Preserved source-plan detail: net-new trait definition. |
| Integration | Provider/device-usbip controller depends on this trait for injected semantic effects; the framework core adapter implements it in ADR046-usbip-002. |
| Data migration | None — docs/tooling only; no runtime state |
| Validation | d2b-contracts tests for trait object safety, firewall apply/observe/release signatures, `UsbipEffectError: Clone + PartialEq + Eq`, `TransientDetail` clone/equality, redacted Debug/Display behavior, and no implementation leakage. |
| Removal proof | None — net-new; no prior owner to remove |

Define the `UsbipEffectPort` async trait with the method set in § UsbipEffectPort.
Define `DeviceUid`, `NetworkUid`, `UsbBindingUid`, `LeaseToken`,
`FirewallToken`, `FirewallObservation`, `KernelModuleClass`, `DeviceProbeResult`, and
`UsbipEffectError`. Export from `d2b-contracts`. No implementation
in `d2b-contracts`; trait only. Add conformance tests in `d2b-contracts/tests/usbip_effect_port.rs`.

---

### ADR046-usbip-002: Core adapter implementation

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-usbip-001, ADR046-zone-control-024; UsbipBindFirewallRule broker op; d2b-host usbip argv support; framework core adapter owner |
| Current source | packages/d2bd/src/usbip_state_machine.rs, packages/d2bd/src/usbip_reconcile_state.rs, packages/d2b-host/src/usbip_argv.rs, packages/d2b-priv-broker/src/ops/usbip_firewall.rs, usbip_host.rs, and usbip_lock.rs |
| Reuse action | adapt |
| Destination | packages/d2b-core/src/device_usbip_adapter.rs |
| Detailed design | Implement UsbipEffectPort in the core adapter: signed-bundle busid lookup, same-Zone validation, trusted physical-USB identity resolution, mandatory Core-derived `physical-usb-backing/v1` digest and exact `(Host, physical-usb-backing, opaqueKeyDigest)` claim shared with every security-key/USB Provider, exclusive OFD claim, sole ownership of all USBIP TCP/3240 and exact per-Network/per-busid `UsbipBindFirewallRule` effects plus ownership-scoped observe/release, anti-spoof probe, one shared Host module/backend authority, one Core-derived D097 relay Endpoint/firewall authority per Network, D097 authority-index preflight/adoption, and post-effect audit; complete the shared claim before any open, withhold, bind, module, relay, firewall, or attachment effect; never expose raw busid, identity digest, path, fd, bind address, nftables body, audit structs, or broker wire types. Primary reuse disposition: `adapt`. Preserved source-plan detail: extract and adapt into framework-internal adapter. |
| Integration | Reconcile framework injects the adapter into Provider/device-usbip; D097 authority index gates effects; adapter calls privileged broker and d2b-host argv helpers behind the semantic trait. |
| Data migration | Full d2b 3.0 reset; adapter resumes from Service/Binding status and authority owner proofs rather than daemon-coupled snapshots |
| Validation | Fast packages/d2b-core/tests/device_usbip_adapter.rs covers same-Zone gate, exact shared physical backing tuple derivation, byte-identical keys for USB/security-key views of one fake token, Provider-private-class/digest bypass rejection, `physical-usb-backing-conflict` before effects, separate USBIP module/relay authorities, one-module/one-relay reuse, exact Network/busid firewall scoping, foreign-marker failure, ownership-scoped drift/status, independent per-busid release, no network-local dependency, anti-spoof, redaction, broker mapping, and no digest/busid/path/fd exposure. |
| Removal proof | Old daemon-coupled adapter call sites are removed by ADR046-usbip-009 after Provider wiring and adapter tests pass. |

Implement the adapter: busid lookup from signed bundle, same-Zone check, OFD lock
management, broker dispatch for `UsbipBindFirewallRule`, sysfs withhold, post-effect
audit emission. The adapter MUST NOT expose any raw busid, lock path, or broker wire
type to the trait caller. Add unit tests for same-Zone gate and anti-spoof logic in
`packages/d2b-core/tests/device_usbip_adapter.rs`.

---

### ADR046-usbip-003: Provider crate skeleton

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-usbip-001, ADR046-provider-004; Provider model crate structure; device-usbip provider owner |
| Current source | None — net-new Provider crate; no pre-ADR45 baseline equivalent |
| Reuse action | create |
| Destination | packages/d2b-provider-device-usbip/ |
| Detailed design | Create the required crate layout; bind the shared D098 `UsbService`/`UsbBinding` base versions/fingerprints from ADR046-provider-004 and implement only strict USBIP Provider extensions; sign/register extension schemas and advertise explicit export only for authority `UsbService` resources implemented by this Provider; implement validation.rs and compile-checked EffectPort injection. Declare the controller user/User resource in Nix activation. Primary reuse disposition: `create`. Preserved source-plan detail: net-new crate skeleton with contract reuse. |
| Integration | Workspace manifests, Provider artifact catalog, Nix module, and ProviderDeployment consume the crate and component descriptor. |
| Data migration | None — docs/tooling only; no runtime state |
| Validation | make test-policy passes; Cargo.toml has no d2b-priv-broker dependency; fast schema/manifest tests consume the common fixtures, accept canonical minimal base without `spec.provider`, prove a fake direct-local Provider can implement the same base, and cover Service-only exportability, Binding non-exportability, Core projection ownerRef/base fields with explicit `spec.provider` rejection, D088 status layering, semantic factory-fingerprint stability across Provider/adapter identity changes, strict refs, and trait injection. |
| Removal proof | None — net-new; no prior owner to remove |

Create the crate with the layout in § Crate layout. Implement `lib.rs`, `validation.rs`
(bus-id corpus from `d2b-contracts::usbip`), and stub controller with compile-checked
`UsbipEffectPort` dependency injection. Validate workspace policy passes
(`make test-policy`). Confirm `d2b-priv-broker` does NOT appear in `Cargo.toml`.

The Nix module declares the `d2b-device-usbip-ctrl` system user and its
`User/d2b-device-usbip-ctrl` resource at NixOS activation time. The
controller component state Volume (`device-usbip--controller--state--host-system`)
is created by core ProviderDeployment — not by the device-usbip controller —
before the controller Process is started. Volume is not an exported ResourceType
of this Provider.

---

### ADR046-usbip-004: Service/Binding controller and D096 adapters

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-usbip-001, ADR046-usbip-002, ADR046-usbip-003, and ADR046-zone-control-024; device-usbip controller owner |
| Current source | packages/d2bd/src/usbip_state_machine.rs and packages/d2bd/src/usbip_reconcile_state.rs |
| Reuse action | adapt |
| Destination | packages/d2b-provider-device-usbip/src/{controller,reconcile,export_import}.rs |
| Detailed design | Reconcile provider-neutral authority/projection `UsbService` and per-Guest `UsbBinding` resources through strict USBIP Provider extensions, consuming UsbipEffectPort, the exact shared Host-global physical USB tuple, USBIP-private D097 authorities, and signed D096 ExportAdapter/ImportAdapter. Enforce same-Zone base and provider refs; ResourceExport authority-Service-only target; same-type ResourceImport-owned projection with `providerRef`, semantic base/import fields, no `spec.provider`, and no physical fields/effects; route selection from the signed local descriptor/import record; semantic factory fingerprint independent of separately authenticated adapter identity; exclusive fair Binding admission; encrypted bounded named-stream control/data; D088 layered status; Service/Binding finalizers; restart adoption; declared/explicit modes; no session/transfer resources. Primary reuse disposition: `adapt`. Preserved source-plan detail: extract and adapt step machine into Provider reconcile loop. |
| Integration | Controller watches Device/Host/Network/Guest/Endpoint/Export/Import dependencies, calls injected EffectPorts, commits ResourceMutationBatch updates, coordinates children from ADR046-usbip-005, and delegates only semantic export/import admission to the Provider adapter while core owns D096 routing/lifecycle. |
| Data migration | Full d2b 3.0 reset; no direct import of d2bd usbip_reconcile_state snapshots |
| Validation | Fast tests/controller_state_machine.rs, service_binding_schema.rs, export_import.rs, authority_conflict.rs, async_loop.rs, finalizer.rs, and wrong_zone.rs cover authority/projection/Binding lifecycle, Service-only export, projection `spec.provider` rejection, semantic fingerprint stability under adapter identity changes plus separate descriptor authentication, D088 `status.resource`/`status.provider` placement, shared physical tuple collision before effects, encrypted fake streams, no physical projection effect, exclusivity, restart, and WrongZone degradation. |
| Removal proof | packages/d2bd/src/usbip_state_machine.rs and usbip_reconcile_state.rs are deleted by ADR046-usbip-009 once Provider parity tests pass. |

Implement the Service authority/projection and Binding attachment step machines.
Map legacy desired/carrier/bind/proxy observations to Service/Binding status,
never Device attachment status. Implement finalizers, restart step skipping,
non-blocking watch dispatch, fair exclusive Binding admission, declared/explicit
mode, and D096 export/import stream revocation.

Tests required:
- `tests/controller_state_machine.rs`: full bring-up / teardown with a `FakeUsbipEffectPort`
- `tests/async_loop.rs`: receiver dispatches Service/Binding B while A awaits an effect
- `tests/finalizer.rs`: finalizer add/clear through partial progress
- `tests/wrong_zone.rs`: WrongZone error → Degraded phase + correct error class

---

### ADR046-usbip-005: Process resource management

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-usbip-003; Process ResourceType schema; device-usbip process lifecycle owner |
| Current source | None — net-new Process resources; templates derive from the Provider package descriptor |
| Reuse action | adapt |
| Destination | packages/d2b-provider-device-usbip/src/reconcile.rs |
| Detailed design | Create/adopt exactly one Host backend Process authority, exactly one D097 Network relay Process/Endpoint/firewall authority bound to TCP 3240 with a Core-derived Network/policy key, and one Binding-owned Guest proxy/private Endpoint per attached Binding. Restrict relay resolution to the Core adapter and exact active-lease Binding proxies; deliver their connected streams by LaunchTicket. Use canonical system-minijail specs, signed templates, bounded budgets/readiness/restart, no argv/path/address/fd fields; attach/detach remains a one-shot EffectPort operation, not a second Process. Primary reuse disposition: `adapt`. Preserved source-plan detail: adapt into singleton Host backend, per-Network relay, and per-Binding Guest proxy management. |
| Integration | Service controller registers physical busids with shared backend/relay; Binding controller creates its Guest proxy/private Endpoint; Process controller launches workers; UsbipGuestEffectPort attaches to the private Endpoint. |
| Data migration | Full d2b 3.0 reset; old per-env runners become Host/Network authorities and per-Device port-3240 workers are forbidden |
| Validation | Fast Process/Endpoint shape tests prove one backend per Host, one Core-derived multiplexed TCP 3240 Endpoint/firewall authority per Network, deterministic duplicate conflict, adapter/exact-Binding-only resolution, LaunchTicket-connected stream, no generic Network reader access, Binding ownership/private Guest policy, no per-Device listener, no raw address/argv/path/fd, and readiness before bind/attach. |
| Removal proof | Old per-env usbipd autostart and ProcessRole::Usbip paths are removed by ADR046-usbip-009 after Process resource lifecycle tests pass. |

Implement the three worker classes in § Worker Process resources using
`ResourceMutationBatch`. Confirm that two Services on one Host/Network reuse
the same backend and relay Endpoint, while two Guest Bindings receive distinct
private proxies/Endpoints. Attach/detach remains an EffectPort call.

---

### ADR046-usbip-006: Typed status.provider.details

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-usbip-003; Device provider details plus `UsbService` and `UsbBinding` base/provider status schema owner |
| Current source | packages/d2bd/src/usbip_reconcile_state.rs state fields |
| Reuse action | adapt |
| Destination | packages/d2b-provider-device-usbip/src/status.rs |
| Detailed design | Define provider-neutral `UsbService` `status.resource` with whole-device availability/access counts, authority-only `physical-usb-backing` claim state, and projection-only common import state; define `UsbBinding.status.resource` with generic attachment phase/queue/generation/timestamps. Define separate strict `status.provider` USBIP details: Device probe, owner backend/relay, USBIP-owned `firewallState`/opaque digest/observed generation, imported-route observations, and Binding proxy/private Endpoint/subphase. No common access/import/attachment field appears directly under `status`; no USBIP module, Network, Endpoint, proxy, server/client, port, firewall, or busid field enters base status; no raw backing digest, busid, path, fd, address, session/transfer ID, remote identity, or payload appears anywhere. Primary reuse disposition: `adapt`. Preserved source-plan detail: adapt state fields to typed status.provider.details. |
| Integration | Controller writes each extension atomically with its resource's common status; dependency/update propagation is Device/Export → Service/projection → Binding. |
| Data migration | Full d2b 3.0 reset; current d2bd reconcile state is not imported |
| Validation | Fast tests/status_serde.rs covers generic base status plus three strict USBIP detail schemas, exact `status.resource` placement for access/import/attachment/backing claim, exact `status.provider` placement for relay/firewall implementation observations, drift transition and ownership, rejection of semantic fields directly under `status`, mode-dependent omissions, bounded counts/refs, unknown-field denial, and a deny corpus proving USBIP-only fields are rejected from base and Network status. |
| Removal proof | Old d2bd USBIP reconcile-state structs are removed by ADR046-usbip-009 after status extension coverage passes. |

Define generic `UsbServiceStatus`/`UsbBindingStatus` base projections and strict
`UsbipDeviceDetails`, `UsbipServiceDetails::{Authority,Projection}`, and
`UsbipBindingDetails`; keep the mode union strict and path-free. Tests:
`tests/status_serde.rs`. The `Usbip*Details` names are Provider-extension DTOs,
not ResourceTypes or compatibility aliases.

---

### ADR046-usbip-007: Hermetic and real-system validation

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-usbip-004 and ADR046-usbip-005; device-usbip integration owner |
| Current source | packages/d2b-contract-tests/tests/usbip_policy_network_scoping.rs plus new integration scenarios |
| Reuse action | adapt |
| Destination | packages/d2b-provider-device-usbip/{src,tests,integration/README.md}; tests/host-integration/usbip-service.nix; tests/host-integration/hardware/usbip-service.sh |
| Detailed design | Put provider-neutral Service/Binding base-schema separation, strict USBIP extensions, projection `spec.provider` rejection, D088 layered status, semantic factory-fingerprint independence from Provider/adapter identity, exact shared physical backing tuple/conflict, one Core-derived per-Network relay Endpoint/firewall authority, exact per-busid firewall ownership/drift/release, arbitration, same-type export/import, encrypted fake-stream, and least-privilege process/Endpoint shape coverage in fast Layer-1 Rust tests. Include a fake direct-local Provider proving the same base contract has no USBIP dependency. Reserve runNixOSTest for real Linux usbip_host/vhci_hcd, usbipd, namespaces/nftables, TCP 3240, zero network-local carve-out, and Guest checks; reserve the hardware script for an approved physical device. Use existing Make gates only. Primary reuse disposition: `adapt`. Preserved source-plan detail: adapt existing network-scoping assertion and add new scenarios. |
| Integration | Layer-2 lanes exercise actual kernel/backend/relay/Guest/device paths and do not duplicate pure controller/schema cases. Cross-Zone protocol logic remains hermetic with fake peers; the runNixOSTest only proves its real-system integration. |
| Data migration | None — docs/tooling only; no runtime state |
| Validation | `make test-host-integration` runs the non-hardware real-kernel case on a capable host; `make test-hardware` runs the explicit manual device case. No Layer-1 test opens a device, loads a module, creates a namespace, or listens on a socket. |
| Removal proof | Old usbip_policy_network_scoping coverage is retired only after the fast wrong-Zone admission test and `tests/host-integration/usbip-service.nix` successor both pass and the migration ledger is updated. |

Required tests:

| Path | Scenario | Gate |
| --- | --- | --- |
| `tests/host-integration/usbip-service.nix` | Real modules, one Host backend, one Core-derived multiplexed Network TCP 3240 Endpoint/firewall authority, exact per-busid rules, ownership-scoped drift/status/release, zero network-local carve-out, wrong-Zone denial, and least-privilege Guest Binding proxy/attach/revocation with fake USB backend | `make test-host-integration` |
| `tests/host-integration/hardware/usbip-service.sh` | Approved physical USB device, exclusive busid, second-Binding fairness, shared Host-global USB/security-key authority conflict, data/detach, no fd/path crossing | `make test-hardware`; manual only |

`packages/d2b-provider-device-usbip/integration/README.md` must document:
- how to run each scenario locally through its existing integration/hardware gate;
- what container/Host/KVM privileges are required;
- which exact cases require a real approved USB device and are manual-only;
- how to add a new scenario;
- the wrong-zone scenario's required assertions.

---

### ADR046-usbip-008: Nix and eval assertions

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-usbip-003, ADR046-zone-control-024; ADR-046-nix-configuration; Nix integrator |
| Current source | nixos-modules/components/usbip.nix guest wiring and new Zone resource declarations |
| Reuse action | adapt |
| Destination | nixos-modules/components/usbip.nix, nixos-modules/options-zones.nix, nixos-modules/assertions.nix |
| Detailed design | Add Provider config; remove the old per-VM option; emit provider-neutral authority `UsbService`, per-Guest `UsbBinding`, strict USBIP Provider envelopes, and optional ResourceExport/ResourceImport authoring shapes; imported same-type projection Services remain Core-created with no `spec.provider`. Assert same-Zone base/provider refs, projection ownerRef/forbidden implementation and physical fields, Service-only export target, one Host backend/Network relay, exact Core-derived Host-global `(Host, physical-usb-backing, opaqueKeyDigest)` exclusion shared by USB/security-key, Provider-private-class bypass rejection, and retain guest vhci_hcd/tools. Primary reuse disposition: `adapt`. Preserved source-plan detail: adapt guest module, remove host-side option surface, and extend eval assertions. |
| Integration | Nix compiler emits Device plus provider-neutral `UsbService`/`UsbBinding` and explicit D096 resources consumed by core/Provider; guest runtime supplies USBIP proxy/attach tools; generated base and provider-extension schemas/fingerprints remain canonical. |
| Data migration | Full d2b 3.0 reset; operators reauthor old per-VM options as Device + authority/projection Service + per-Guest Binding |
| Validation | Fast tests/unit/nix/cases/usbip-*.nix cover schema shape, Core projection `spec.provider` rejection, D088 layered status fixtures including USBIP-owned firewall state, all reference/owner/export assertions, byte-identical USB/security-key tuple collision and private-class bypass rejection, one Core-derived 3240 Endpoint/firewall authority per Network, least-privilege consumer policy, absence of network-local 3240 rules, old-option removal, and guest module retention. |
| Removal proof | d2b.vms.<vm>.usbip.yubikey and host-side USBIP module paths are removed at reset once Zone resource emitter coverage passes. |

- Add `d2b.zones.<zone>.providers.device-usbip.config.controllerExecutionRef` option.
- Remove `d2b.vms.<vm>.usbip.yubikey` at v3 reset; add deprecation warning until removal.
- Add Zone resource shapes for Device, authority `UsbService`, `UsbBinding`, and
  optional ResourceExport/ResourceImport; projection Services are Core-owned
  and never contain `spec.provider`.
- Add eval assertions for same-Zone base/provider refs, same-type Service-only
  export, projection restrictions, one Host/Network USBIP authority, and shared
  Host-global USB/security-key backing exclusion.
- Retain guest-side `nixos-modules/components/usbip.nix` (vhci_hcd + tools) unchanged under runtime-cloud-hypervisor.
- Add or update `tests/unit/nix/cases/usbip-*.nix` for each new assertion path.

---

### ADR046-usbip-009: Removal of v3 daemon-coupled USBIP

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-usbip-004 and ADR046-usbip-008; Provider fully wired and validated; daemon cleanup owner |
| Current source | packages/d2bd/src/usbipd_perenv_autostart.rs, packages/d2bd/src/usbip_state_machine.rs, packages/d2bd/src/usbip_reconcile_state.rs, nixos-modules/network.nix USBIP firewall block, and ProcessRole::Usbip in packages/d2b-core/src/processes.rs |
| Reuse action | delete-after-cutover |
| Destination | packages/d2bd/src/, nixos-modules/network.nix, packages/d2b-core/src/processes.rs |
| Detailed design | Remove daemon-coupled USBIP after Provider tests and integration tests pass: delete per-env autostart, state machine, and reconcile state modules after migration; remove USBIP firewall block from network.nix; remove ProcessRole::Usbip; run Layer-1 gates and confirm no d2bd or network.nix references remain outside the adapter and contracts. Primary reuse disposition: `delete-after-cutover`. Preserved source-plan detail: delete after Provider replacement reaches parity. |
| Integration | Provider/device-usbip, core D096/D097 adapters, Nix Device plus provider-neutral USB resource emitter, USBIP authority workers, and Binding-owned children are the sole USBIP lifecycle path after deletion. |
| Data migration | Full d2b 3.0 reset; no daemon-coupled USBIP runtime state import |
| Validation | make test-unit and make test-flake plus grep or contract checks for removed symbols and no residual d2bd/network.nix USBIP lifecycle references. |
| Removal proof | usbipd_perenv_autostart.rs, usbip_state_machine.rs, usbip_reconcile_state.rs, network.nix USBIP firewall block, and ProcessRole::Usbip are deleted after parity. |

Deletion sequence:
1. Confirm Provider tests and integration tests pass.
2. Delete `packages/d2bd/src/usbipd_perenv_autostart.rs`.
3. Remove `packages/d2bd/src/usbip_state_machine.rs` and `usbip_reconcile_state.rs`
   after verifying all logic has been migrated.
4. Remove USBIP firewall block from `nixos-modules/network.nix`.
5. Remove `ProcessRole::Usbip` from `packages/d2b-core/src/processes.rs`.
6. Run `make test-unit` and `make test-flake`; confirm no USBIP references remain in
   d2bd or network.nix outside the adapter and contracts.

---

## Tests

### Fast hermetic execution and test placement (D094)

Per D094 and `ADR-046-validation-and-delivery` §10.16, this Provider's `src/`
unit tests and `tests/*.rs` hermetic suite are fast, in-process, deterministic,
and parallel-safe: an individual normal test has p95 ≤50 ms with no wall-clock
sleep, and `cargo test -p d2b-provider-device-usbip --lib --tests` completes in
≤2 s warm-cache execution time (compilation excluded). They use a deterministic
fake clock/RNG and the toolkit fakes/FakeEffectPort only — no process spawn,
container, network, DBus, systemd, broker daemon, Nix eval/build, KVM,
USB/GPU/TPM hardware, or live cloud, and no filesystem tree beyond tiny temp
fixtures. Any scenario genuinely needing a booted kernel/Guest lives only in
`tests/host-integration`; physical-device coverage lives only in
`tests/host-integration/hardware`. Such a need is moved to the matching existing
Layer-2 gate, never given a sleep, larger unit-test timeout, or `#[ignore]`.
Bounded crypto/property tests are the only classified exception, each named
with a capped case count and declared higher per-test budget.

### Required unit tests (`tests/`)

| File | Coverage |
| --- | --- |
| `service_binding_schema.rs` | Provider-neutral authority/projection `UsbService` and `UsbBinding` base schemas plus strict USBIP Provider envelopes; canonical minimal base; same-Zone refs; imported projection ResourceImport ownerRef, `spec.provider` rejection, and physical-field denial; same-type Service-only explicit export; Device/Endpoint/Binding export denial |
| `provider_neutral_base.rs` | Base specs/status reject USBIP port, Network, Endpoint, proxy, server/client, firewall, and busid fields; a fake direct-local Provider implements the same base types without USBIP schemas or aliases |
| `controller_state_machine.rs` | Authority and Binding sequences with fake ports; shared Host backend/Network relay; Binding-owned proxy/private Endpoint; declared/explicit attach; restart and reverse teardown |
| `authority_conflict.rs` | USBIP, fake direct-local USB, and security-key Providers resolve one fake token to a byte-identical Core-derived `(Host, physical-usb-backing, opaqueKeyDigest)` tuple; private-class/digest bypass fails; the second claim returns `physical-usb-backing-conflict` before effects; one Host USBIP module/backend; one Core-derived TCP 3240 relay Endpoint/firewall authority per Network; multiplex reuse and deterministic conflicts |
| `export_import.rs` | ResourceExport targets authority `usb.d2bus.org.UsbService` only; ResourceImport preserves that exact type and creates an ownerRef projection with no `spec.provider`; semantic factory fingerprint is stable under Provider/adapter identity changes while signed descriptor authentication remains exact; encrypted bounded fake control/data streams; generation/fingerprint/revocation; no fd/path/busid; sessions/transfers remain internal |
| `effect_port_contract.rs` | `UsbipEffectPort` and `UsbipGuestEffectPort` trait object safety; firewall apply/observe/release signatures callable from Provider crate; no import of broker types or `d2b-priv-broker`; compile-time `UsbipEffectError: Clone + PartialEq + Eq`; `TransientDetail` clone/equality and manual Debug/Display both produce `<redacted>` |
| `firewall_ownership.rs` | Exact per-Network/per-busid intent; one relay listener/base hook with independent busid entries; foreign marker rejection; ownership-scoped drift and strict Service provider status; release one busid preserves another; Network digest/status unaffected; no network-local generic rule dependency |
| `relay_endpoint_authority.rs` | Exactly one D097 relay Endpoint authority per Network; Core-derived key; multiplexed Service holders; duplicate conflict before listen/firewall effect; adapter/exact-active-Binding-only resolution; LaunchTicket connected stream; generic Network readers denied |
| `conformance.rs` | Device/`UsbService`/`UsbBinding` ResourceTypeSchema round-trip, signed USBIP extension fingerprints, deny_unknown_fields, and Provider capability advertisement |
| `state_volume.rs` | Controller Volume schema conformance: `stateSchema: {}`, layout `ownerRef: User/<name>` (not ComponentPrincipal), `sensitivityClass: private`, single `state` view; no cross-component Volume; dirfd delivery to controller only |
| `status_serde.rs` | Provider-neutral whole-device Service/Binding `status.resource` plus strict USBIP `status.provider` details including firewall state/digest/generation; reject access/import/attachment/backing fields directly under `status`, reject module/Network/Endpoint/proxy/server/client/port/firewall/busid fields from base and Network status, and deny raw identity/path/fd/address/session/transfer/payload fields everywhere |
| `validation_corpus.rs` | Bus-id max length (31 chars); metachar rejection; leading-zero segment rejection; vendor/product id exactly 4 hex digits; `busClass != usb` → `unsupported-bus-class` |
| `mutual_exclusion.rs` | USBIP, fake direct-local USB, and security-key claims for one Host-global backing key conflict; second Binding queues fairly and causes no second bind/open |
| `wrong_zone.rs` | Every base and USBIP provider ref is same-Zone; cross-Zone use requires D096 import; wrong-Zone firewall causes Service degradation and no effect |
| `finalizer.rs` | Service/Binding finalizers start only after effect/lease and clear only after child/lease teardown; restart resumes partial teardown |
| `async_loop.rs` | Independent Service/Binding dispatch remains concurrent while per-Service single-flight preserves arbitration |

All rows above are Layer-1 Rust tests in the crate's `src/` or `tests/`; they
use fake EffectPorts/streams/authority indexes and perform no module load,
network bind, namespace operation, process spawn, or device open.

### Integration-only real kernel/device checks

| Path | Scenario | Gate |
| --- | --- | --- |
| `tests/host-integration/usbip-service.nix` | Real NixOS boot, `usbip_host`/`vhci_hcd`, one Network TCP 3240 relay Endpoint authority multiplexing two fake USB backends, exact per-busid rules, drift/status/reapply, independent release, zero network-local TCP/3240 rule, wrong-Zone denial, least-privilege Binding proxy/attach/revocation | `make test-host-integration`; capable host |
| `tests/host-integration/hardware/usbip-service.sh` | Approved physical USB device: shared Host-global backing and exclusive busid claims, attach/data/detach, second Binding fairness, USBIP/security-key conflict, and no fd/path crossing | `make test-hardware`; manual hardware only |

`packages/d2b-provider-device-usbip/integration/README.md` indexes these Layer-2
scenarios, their existing Make targets, required privileges/KVM/modules, and
the approved-device/manual safety preconditions. Real kernel/device behavior
must not be moved into unit tests, hidden behind `#[ignore]`, or simulated as a
claim of hardware coverage.

---

## Removal sequence

When `Provider/device-usbip` is fully deployed and every USBIP Device has an
authority Service plus per-Guest Bindings/projections as needed:

1. Remove `d2b.vms.<vm>.usbip.yubikey` Nix option (deprecated at reset; removed
   now).
2. Delete `packages/d2bd/src/usbipd_perenv_autostart.rs`.
3. Delete `packages/d2bd/src/usbip_state_machine.rs` and
   `usbip_reconcile_state.rs` (logic migrated to Provider crate and adapter).
4. Remove `ProcessRole::Usbip` from `packages/d2b-core/src/processes.rs`.
5. Remove USBIP firewall block from `nixos-modules/network.nix`.
6. Run `make check` (Layer-1 gate); confirm zero remaining references to removed
   symbols from outside `d2b-provider-device-usbip` and `d2b-core/src/device_usbip_adapter.rs`.
7. Update this dossier's `Supersedes` field to mark all removed targets as `removed`.

Per D094, each replaced current-code test is retired with an explicit
keep/adapt/move/delete disposition and a removal gate: the minimum reusable
semantic assertions migrate into this crate's hermetic `tests/`, and the old
duplicate tests, shell gates, fixtures, static artifacts, CI jobs, and manifest
entries are deleted once successor coverage and the removal proof pass —
updating `tests/layer1-jobs.json`, the closed gate manifests, the
flake/matrix/Nix-unit pins, the generated ledgers, and the CI workflow shards.
Old and new suites never run in parallel indefinitely.
