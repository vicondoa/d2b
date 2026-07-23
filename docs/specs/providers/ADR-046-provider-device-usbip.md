# ADR 0046 Provider dossier: device-usbip

| Field | Value |
| --- | --- |
| Spec ID | `ADR-046-provider-device-usbip` |
| Parent | ADR 0046 |
| Status | Proposed |
| Version | 3 |
| Baseline | `b5ddbed67867d9244bf33390868101bd9b053e49` |
| Normative | Yes |
| Owners | `d2b-provider-device-usbip` crate, Device controller contracts, Nix device emitter |
| Depends on | `ADR-046-resources-device`, `ADR-046-resources-network`, `ADR-046-components-processes-and-sandbox`, `ADR-046-provider-model-and-packaging`, `ADR-046-resource-reconciliation`, `ADR-046-telemetry-audit-and-support`, `ADR-046-componentsession-and-bus`, `ADR-046-nix-configuration`, `ADR-046-resource-api-and-authorization`, `ADR-046-provider-state` |
| Supersedes | `nixos-modules/components/usbip.nix` (host-side), per-env usbipd systemd units in `nixos-modules/network.nix`, `ProcessRole::Usbip` / `RunnerRole::Usbip` in current v3 baseline |

---

## Purpose

This dossier specifies `Provider/device-usbip`, the d2b 3.0 Provider that owns
USB/IP (USBIP) device inventory, arbitration, busid probe/claim, host-side
kernel bind, per-Device backend and proxy Process lifecycle, firewall
carve-out, guest-side import coordination, and operator CLI for the
`d2b device usb` surface.

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
| `d2b-contracts` | `validate_bus_id`, `SYSFS_BUS_ID_MAX`, `UsbipClaimSource`, `sanitize_usb_hex_id`, `UsbipEffectPort` trait |
| `d2b-provider-toolkit` | `ReconcileContext`, `ResourceClient`, `ResourceMutationBatch`, `phase_*` helpers, generic conformance |

The Provider crate **must not** import `d2b-priv-broker`, `d2bd`, `d2b-host`,
`d2b-realm-core`, Zone-store internals, or another Provider's implementation.
No raw broker op DTOs, lock file paths, nftables rule bodies, or busid strings
appear in the Provider's public types or internal reconcile logic.

---

## Implements

| ResourceType | Arbitration | Max concurrent claims |
| --- | --- | --- |
| `Device` | `exclusive` | 1 |

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
`usbipHostKernelModule`, `vhciHcdKernelModule`, `backendPort`, and the USBIP TCP
proxy port are signed manifest/effect policy constants embedded in the Provider
package descriptor. They are never operator-configurable. Executable paths for
usbip, usbipd, and the TCP proxy binary are resolved exclusively from the signed
component descriptor inside the Provider package closure.

---

## Device spec

```yaml
apiVersion: resources.d2b.io/v3
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
  settings:
    networkRef: Network/work-net  # required; used as zone-scoping dependency
    claimMode:  declared          # declared|explicit
```

### Settings fields

| Field | Type | Default | Bounds | Notes |
| --- | --- | --- | --- | --- |
| `networkRef` | ResourceRef | — | required; `Network/<name>` in same Zone | Dependency for uplink IP, bridge interface, and proxy listen address. Read-only status watch; Network controller does not own USBIP firewall. |
| `claimMode` | `declared\|explicit` | `declared` | — | `declared` = the controller brings up the full daemon+proxy process stack automatically upon Device ready; `explicit` = operator calls `d2b device usb attach` to trigger bind and proxy after the daemon is running. |

Bus-class `usb` is the only accepted value for `busClass` in a `device-usbip`
Device. Any other value fails spec admission with `unsupported-bus-class`.

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
apiVersion: resources.d2b.io/v3
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
  endpoints:     []
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

### `device-<uid-short>-daemon`

Long-lived per-Device usbipd backend. Owned by the Device resource.

```yaml
apiVersion: resources.d2b.io/v3
type: Process
metadata:
  name: device-b3a7f1d2c591-daemon
  zone: work
  ownerRef: Device/corp-vm-usb
spec:
  providerRef:  Provider/system-minijail
  executionRef: Host/host-system
  domain:       system
  processClass: worker
  template:     usbip-daemon              # resolves to usbipd binary from Provider closure
  sandbox:
    namespaceClasses: [mount, network]
    capabilityClasses: []
    seccompClass: usbip-daemon
    noNewPrivileges: true
    startRoot: false
    environmentClass: minimal
    readOnlyRoot: true
  budget:
    cpu:
      request: "50m"
      limit: "500m"
    memory:
      request: "16Mi"
      limit: "64Mi"
    pids:
      limit: 64
    fds:
      limit: 128
  networkUsage:
    networkRef: Network/work-net
    ports:
      - port: 3241        # backend port policy constant from signed manifest
        protocol: tcp
        purpose: usbip-backend
    allowEgress: false
  endpoints:
    - name:      backend
      transport: tcp
      purpose:   usbip-backend-listener   # adapter resolves bind addr from signed policy
  readiness:
    class: provider-defined
    initialDelay: "0s"
    timeout: "5s"
    failureThreshold: 3
  restartPolicy:
    class:           on-failure
    backoffBase:     "1s"
    backoffMax:      "30s"
    backoffMultiplier: 2.0
    maxRestarts:     5
    resetAfter:      "300s"
  adoptionPolicy: adopt-on-restart
  drainTimeout: "5s"
```

### `device-<uid-short>-proxy`

Long-lived TCP proxy that forwards guest connections from the Network uplink
IP (TCP 3240) to the loopback usbipd backend. Owned by the Device resource.

```yaml
apiVersion: resources.d2b.io/v3
type: Process
metadata:
  name: device-b3a7f1d2c591-proxy
  zone: work
  ownerRef: Device/corp-vm-usb
spec:
  providerRef:  Provider/system-minijail
  executionRef: Host/host-system
  domain:       system
  processClass: worker
  template:     usbip-proxy               # resolves to d2b TCP proxy from Provider closure
  sandbox:
    namespaceClasses: [mount, network]
    capabilityClasses: []
    seccompClass: usbip-proxy
    noNewPrivileges: true
    startRoot: false
    environmentClass: minimal
    readOnlyRoot: true
  budget:
    cpu:
      request: "25m"
      limit: "250m"
    memory:
      request: "8Mi"
      limit: "32Mi"
    pids:
      limit: 32
    fds:
      limit: 64
  networkUsage:
    networkRef: Network/work-net
    ports:
      - port: 3240        # standard USBIP TCP port; policy constant from signed manifest
        protocol: tcp
        purpose: usbip-proxy-listener
    allowEgress: false
  endpoints:
    - name:      proxy
      transport: tcp
      purpose:   usbip-proxy-listener    # adapter resolves bind addr from Network status
  readiness:
    class: provider-defined
    initialDelay: "0s"
    timeout: "5s"
    failureThreshold: 3
  restartPolicy:
    class:           on-failure
    backoffBase:     "1s"
    backoffMax:      "30s"
    backoffMultiplier: 2.0
    maxRestarts:     5
    resetAfter:      "300s"
  adoptionPolicy: adopt-on-restart
  drainTimeout:   "5s"
```

### Guest-side effects (no Process resource)

Guest-local USBIP effects (`vhci_hcd` module load, `usbip attach`, `usbip
detach`) go through a **guest-side `UsbipGuestEffectPort`** adapter injected
into the controller when the claiming Guest's supervisor is addressed. The
Guest supervisor (e.g., `Provider/runtime-cloud-hypervisor`) owns this
adapter and exposes it to the controller through the reconcile framework's
per-Guest effect channel.

The controller calls `guest_effect.attach(device_uid, claim_uid)` or
`guest_effect.detach(device_uid, claim_uid)` semantically. The Guest supervisor
adapter privately:

1. Locates the Guest-side `usbip` binary from the signed bundle.
2. Issues the attach/detach command via its privileged guest-control channel.
3. Returns a typed `UsbipGuestEffectError` to the controller.

A one-shot guest-side command is neither a long-lived worker nor an
EphemeralProcess resource. No Process resource is created in the Guest for
attach or detach operations. The guest-side `vhci_hcd` kernel module is wired
at Guest build time via `nixos-modules/components/usbip.nix`.

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
/// Zeroized on drop; Debug is redacted.
pub struct LeaseToken([u8; 32]);
pub struct FirewallToken([u8; 16]);

impl fmt::Debug for DeviceUid    { fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { write!(f, "DeviceUid(<redacted>)") } }
impl fmt::Debug for NetworkUid   { fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { write!(f, "NetworkUid(<redacted>)") } }
impl fmt::Debug for LeaseToken   { fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { write!(f, "LeaseToken(<redacted>)") } }
impl fmt::Debug for FirewallToken{ fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { write!(f, "FirewallToken(<redacted>)") } }

// Clone impls are manual (not derived) to avoid clippy::expl_impl_clone_on_copy
// on the non-Copy structs; also prevents accidental derive(Debug) via derive macro chains.

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

    /// Install the nftables carve-out for this Device+Network pair.
    /// The adapter validates that device_uid and network_uid are in the same Zone
    /// before dispatching UsbipBindFirewallRule through the privileged broker.
    /// Wrong-Zone returns WrongZone immediately without any host mutation.
    async fn apply_firewall(
        &self,
        device_uid:  &DeviceUid,
        network_uid: &NetworkUid,
        lease_token: &LeaseToken,
    ) -> Result<FirewallToken, UsbipEffectError>;

    /// Remove the nftables carve-out.
    async fn release_firewall(
        &self,
        device_uid:     &DeviceUid,
        firewall_token: FirewallToken,
    ) -> Result<(), UsbipEffectError>;

    /// Bind the physical device to usbip-host (kernel-level).
    /// Requires firewall already applied and daemon Process Ready.
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

/// Guest-side effect port injected by the Guest supervisor when a claim
/// is associated with a specific Guest. The controller calls semantic
/// methods; the adapter issues commands through the Guest supervisor's
/// privileged guest-control channel.
#[async_trait]
pub trait UsbipGuestEffectPort: Send + Sync {
    /// Trigger `usbip attach` inside the claiming Guest.
    async fn attach(&self, device_uid: &DeviceUid, claim_uid: &[u8; 16]) -> Result<(), UsbipEffectError>;
    /// Trigger `usbip detach` inside the claiming Guest.
    async fn detach(&self, device_uid: &DeviceUid, claim_uid: &[u8; 16]) -> Result<(), UsbipEffectError>;
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
   privileged executor).
5. Records the broker/core audit record **post-effect** (after the host mutation
   completes); audit is not atomic with resource store commits.
6. Returns typed `UsbipEffectError` variants to the controller; no broker wire
   types, lock paths, audit structs, or nftables details leak.

The audit record emitted by the adapter contains: zone, device name-digest,
network name-digest, operation tag, outcome, error_class, and correlation_id.
No raw busid, lock path, nftables body, or vendor/product ID appears in audit.

---

## Bring-up sequence

The canonical step order (preserved from `packages/d2bd/src/usbip_state_machine.rs`):

```
modprobe → lock → withhold → firewall → backend → bind → proxy
```

Each step maps to an EffectPort call or a Process resource operation:

| Step | EffectPort call / Process action | Notes |
| --- | --- | --- |
| `modprobe` | `ensure_kernel_module(uid, KernelModuleClass::UsbipHost)` | Idempotent; ok if already loaded |
| `lock` | `acquire_lease(uid)` → `LeaseToken` | Adapter holds OFD lock; controller holds opaque token |
| `withhold` | `withhold_device(uid, &token)` | Adapter writes sysfs authorized=0 via broker |
| `firewall` | `apply_firewall(uid, net_uid, &token)` → `FirewallToken` | Adapter validates same-Zone; dispatches `UsbipBindFirewallRule` |
| `backend` | Create + wait for `device-<uid-short>-daemon` Process → Ready | Long-lived Process; readiness class provider-defined; adapter verifies backend listener |
| `bind` | `bind_busid(uid, &token)` | Adapter runs usbip bind internally; requires daemon Ready |
| `proxy` | Create + wait for `device-<uid-short>-proxy` Process → Ready | Long-lived Process; readiness class provider-defined; adapter verifies listener |

Teardown is the reverse:

| Step | EffectPort call / Process action |
| --- | --- |
| Stop proxy | Delete / drain `device-<uid-short>-proxy` Process |
| `unbind` | `unbind_busid(uid, &token)` |
| Stop backend | Delete / drain `device-<uid-short>-daemon` Process |
| `release_firewall` | `release_firewall(uid, firewall_token)` |
| `release_withhold` | `release_device_withhold(uid, &token)` |
| `release_lock` | `release_lease(uid, token)` |

Each step is idempotent. The controller reads `providerStatus.usbip.completedSteps`
from Device status on every reconcile entry and skips already-completed steps.

---

## Typed Device status

Step detail lives in the typed provider status extension, not in the common
Device phase:

```yaml
status:
  phase: Pending | Ready | Degraded | Failed | Unknown   # common phase ONLY
  conditions: []
  device:
    providerStatus:
      usbip:
        currentStep:    firewall   # last attempted step
        completedSteps:
          - modprobe
          - lock
          - withhold
        lastStepOutcome:  success | transient-failure | terminal-failure
        lastStepError:    ""       # closed-set slug; empty on success
        leaseHeld:        true     # opaque; signals adapter state to readers
        firewallApplied:  true
        daemonProcessRef: Process/device-b3a7f1d2c591-daemon
        proxyProcessRef:  Process/device-b3a7f1d2c591-proxy
        claimerRef:       Guest/corp-vm   # current claimant; null if unclaimed
        attachedAt:       "2024-01-15T10:23:44Z"
        labelDigest:      "a3f7..."       # stable hash of logical label; never raw busid
```

### Common Device phase semantics

| Phase | Meaning |
| --- | --- |
| `Pending` | Controller is reconciling; steps in progress |
| `Ready` | All bring-up steps complete; proxy Process is Ready; claimant attached |
| `Degraded` | Device previously Ready; now one or more steps failed transiently |
| `Failed` | Terminal failure; manual intervention required |
| `Unknown` | Controller unreachable or crashed; last known status stale |

The `providerStatus.usbip.currentStep` / `completedSteps` fields carry the
step-level detail. The common `phase` field is driven only by the overall
bring-up / teardown result, not by individual step names.

---

## Declared vs explicit claim mode

### `claimMode: declared`

The controller automatically drives the full bring-up sequence when a Device
claim is approved and the Network dependency is Ready. The operator authors
only the Device resource and the Zone claim.

Claim source: `UsbipClaimSource::Declared { device_uid, network_uid }` —
tracked in `providerStatus.usbip.claimSource`; never exposed to the requesting
Guest.

### `claimMode: explicit`

Steps `modprobe` through `backend` run automatically on Device Ready (daemon
process starts). The controller pauses at `bind` and waits for an operator
invocation of `d2b device usb attach <device-name>`.

The `bind` and `proxy` steps are triggered by the `AttachDevice` bus method
(see § d2b-bus methods). After `bind` and `proxy` succeed, the controller
calls `guest_effect.attach(device_uid, claim_uid)` through the injected
`UsbipGuestEffectPort` to complete the Guest-side import. No Process resource
is created in the Guest.

Claim source: `UsbipClaimSource::Explicit` — tracked in providerStatus.

---

## Exclusivity and cardinality

- `arbitration: exclusive` — at most one approved Device claim at any time.
- `maxConcurrentClaims: 1` — enforced by the Device arbiter in core.
- A pending second claim waits in `Pending` phase until the first is released.
- `d2b.zones.<zone>.resources.<device-name>` and
  `d2b.zones.<zone>.resources.<security-key-name>` sharing the same
  `selector.label` are mutually exclusive at eval time.
  `nixos-modules/assertions.nix` rejects the configuration:
  ```
  assertion: !(d2b-usbip-devices ∩ security-key-devices by label is non-empty)
  message: "USBIP device and security-key device share selector.label '<label>'
            in zone '<zone>'; the label must be unique per zone."
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
│  1. Read fresh Device + dependency snapshot      │
│  2. Compute desired step delta                   │
│  3. Execute EffectPort steps (await each)        │
│  4. Create/delete child Process resources        │
│  5. Commit ResourceMutationBatch + status        │
└─────────────────────────────────────────────────┘
```

**The watch receiver loop MUST continue reading and dispatching during steps 3
and 4.** Independent Device resources run concurrently under a semaphore budget
(one slot per Device). The receiver never blocks waiting for an effect step.

On restart, the controller:
1. Lists all Device resources it owns.
2. Reads `providerStatus.usbip.completedSteps` from each Device status.
3. Skips completed steps idempotently.
4. Attempts adoption of child Process resources (`adoptionPolicy: adopt-on-restart`).
5. Resumes from the earliest non-completed step.

---

## Network dependency handling

The controller uses a `ResourceClient` dependency watch (read-only) on the
Network resource named in `settings.networkRef`. It never contacts the Network
controller directly; it observes Network status changes via the watch.

The `ResourceClient` dependency watch is the **only** channel for Network
information. No broker connection for Network data; no route table access;
no direct NetworkManager/nftables query.

Fields read from Network status by the controller (read-only, via ResourceClient watch):
- `status.phase` — gating: proxy Process not created until Network is `Ready`

The adapter privately reads additional Network status fields (such as the host
uplink IP) through its own trusted channel when constructing the proxy Process
or executing firewall effects. These fields are adapter-internal; the controller
never reads or holds a raw IP address from Network status.

The Network controller does **not** own the USBIP firewall carve-out.
`D-NETWORK-002` (`ADR-046-resources-network.md` lines 817–832) confirms:
`device-usbip` owns the firewall semantic authority. The `apply_firewall`
EffectPort step is the only path that creates or removes the nftables rule;
the Network controller must not create, remove, or reference USBIP firewall
rules.

---

## Finalizer lifecycle

The controller adds its finalizer `device-usbip.d2b.io/finalizer` to the
Device resource after the first successful step in the bring-up sequence.
It does NOT add the finalizer before any host mutation has occurred.

Teardown on deletion request:
1. Controller detects `deletionTimestamp` set.
2. Executes teardown sequence (proxy stop → unbind → daemon stop → firewall
   release → withhold release → lease release).
3. Marks teardown progress in `providerStatus.usbip.teardownSteps` (same
   pattern as `completedSteps`; idempotent on restart).
4. Clears the finalizer only after all teardown steps succeed.
5. Core commits the finalizer removal; the resource is garbage-collected.

If a teardown step fails terminally, the controller sets `phase: Degraded` and
`providerStatus.usbip.teardownBlocked: true`, emits a structured event, and
requeuess under exponential backoff.

---

## ProviderStateSet

A **ProviderStateSet** is the logical/query-time set of all Volume resources in
a Zone whose `metadata.ownerRef` resolves to `Provider/device-usbip`. It is a
query-time grouping, not a ResourceType or stored artifact:

```text
ProviderStateSet(zone, "device-usbip") =
  { v : Volume | v.metadata.zone == zone
              && v.metadata.ownerRef == "Provider/device-usbip" }
```

Core **ProviderDeployment** creates every declared component state Volume before
the component Process is started, and deletes them after all component Process
and child finalizers complete. The semantic `device-usbip` controller does not
create, delete, or own Volumes. `Provider/volume-local` is the sole Volume
reconciler.

### Controller component Volume

Even payload-empty component state Volumes are **durable** (`kind: state`,
`persistenceClass: persistent`). They survive component and Provider restart,
participate in Provider upgrade (schema migration path, even when empty),
Provider destroy (Volume deleted only after all child finalizers complete), and
Provider reset (Volume wiped and re-provisioned). A payload-empty Volume is
never `kind: ephemeral` and never carries `quota: null` or zero byte/inode
limits; the identity marker and directory entry require nonzero allocation.

For `device-usbip`, the controller has no durable payload state beyond Device
resource status and child Process resources, so the Volume carries an empty
`stateSchema`. The Volume still exists as the durable lifecycle and identity
anchor for the controller component across all restart and upgrade events.

```yaml
apiVersion: resources.d2b.io/v3
type: Volume
metadata:
  name: device-usbip--controller--state--host-system
  zone: work
  ownerRef: Provider/device-usbip
spec:
  providerRef:      Provider/volume-local
  kind:             state           # durable; fail-closed on missing-after-provision
  persistenceClass: persistent      # survives daemon restart; not NixOS-activation-managed
  sensitivityClass: private
  stateSchema:
    migrationPolicy: none           # empty payload; no schema, no migration worker
  quotaBytes: 65536                 # provider-state extension; must equal quota.maxBytes
  quota:
    maxBytes:    65536              # 64 KiB — identity marker + directory entry
    maxInodes:   32                 # marker inode, state directory, and small head-room
    enforcement: none               # advisory; hard enforcement not required for identity-only content
  sealingCredentialRef: null
  source:
    executionRef: Host/host-system
    settings:     {}
  layout:
    - path:           state
      type:           directory
      ownerRef:       User/d2b-device-usbip-ctrl   # Nix-preprovisioned system user
      groupRef:       User/d2b-device-usbip-ctrl   # same principal; no other group
      mode:           "0700"
      sensitivity:    private
      createPolicy:   create-if-never-provisioned
      repairPolicy:   exact-owner
      cleanupPolicy:  owner-controlled
      noFollow:       true
  views:
    state:
      path:   state
      rights: [read, write, create, delete, traverse]
  identityMarker:
    class:      broker-maintained
    markerRoot: provider-state-markers
  snapshotPolicy: null
  retentionPolicy: null
```

### Invariants

- **Layout principals are Nix-preprovisioned `User/<name>` resources** (or a
  bounded principal pool declared in the Nix module). The `ownerRef` and
  `groupRef` in every layout entry bind to `User/<name>` — never to a
  ComponentPrincipal ResourceRef or a raw OS username string.
- **No cross-component shared Volume.** `sensitivityClass: private` means only
  the controller component Process may mount the `state` view. The daemon and
  proxy worker processes have no Volume; stateless workers receive no dirfd.
- **Component gets only its local view dirfd.** The volume-local Provider
  delivers the named view dirfd to the mounting Process; no raw host filesystem
  path is exposed outside the view boundary.
- The Volume's stable identity (inode pair recorded in the broker-maintained
  identity marker) persists across controller restarts and is checked on every
  daemon restart and Process launch.
- **Lifecycle participation.** `kind: state` Volumes participate in Provider
  upgrade (migration path invoked even for empty `stateSchema`), Provider
  destroy (deleted only after all child finalizers clear), and Provider reset
  (wiped and re-provisioned by the controller). The Volume is never
  auto-expired, never ephemeral, and quota is never zero.

### Nix activation

The Nix module for `Provider/device-usbip` provisions the system user
`d2b-device-usbip-ctrl` and the corresponding `User/d2b-device-usbip-ctrl`
resource at NixOS activation time. The user must exist before the controller
Volume is provisioned.

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
| `resource_type` | `Device` |
| `resource_name_digest` | Stable hash of device label; never raw busid or path |
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
| `device-usbip.reconcile` | `zone`, `device.name_digest`, `device.phase`, `trigger_reason` |
| `device-usbip.effect.ensure_kernel_module` | `outcome`, `error_class` |
| `device-usbip.effect.acquire_lease` | `outcome`, `error_class` |
| `device-usbip.effect.withhold_device` | `outcome`, `error_class` |
| `device-usbip.effect.apply_firewall` | `outcome`, `error_class`, `zone_match: bool` |
| `device-usbip.effect.release_firewall` | `outcome`, `error_class` |
| `device-usbip.effect.bind_busid` | `outcome`, `error_class` |
| `device-usbip.effect.unbind_busid` | `outcome`, `error_class` |
| `device-usbip.process.daemon_start` | `outcome`, `error_class` |
| `device-usbip.process.proxy_start` | `outcome`, `error_class` |

Attributes must never carry raw busids, lock paths, nftables text, binary
paths, or operator/user identifiers. Cardinality is bounded: `error_class` is
a closed enum; `device.name_digest` is a fixed-length hash; `zone` is a Zone
name (bounded by Zone cardinality).

---

## d2b-bus methods

The controller registers these typed d2b-bus methods on the Device ResourceType:

| Method | Authority | Description |
| --- | --- | --- |
| `AttachDevice` | Admin | Trigger `bind` and `proxy` steps in `explicit` claim mode. Returns immediately; progress reflected in Device status. |
| `DetachDevice` | Admin | Initiate teardown from `proxy` step backward. Returns immediately. |
| `ProbeDevice` | Admin | Re-run physical probe; update `providerStatus.usbip.lastProbeResult`. Returns synchronously. |
| `GetDeviceStatus` | Admin, StatusReader | Return current `providerStatus.usbip.*` snapshot. |
| `ListBusIds` | Admin | Return the Zone's USBIP device catalog (name-digests and logical labels only; no raw busids). |

No bus method returns or accepts a raw busid, lock path, or broker wire type.
The `ListBusIds` response contains only the logical label and `providerStatus`
snapshot; the internal busid is never surfaced.

---

## RBAC

| Role | ResourceType | Verbs | Scope | Granted to |
| --- | --- | --- | --- | --- |
| `device-manager` | Device | `get`, `list`, `watch`, `update/status`, `patch/status` | Zone-scoped (own Devices) | device-usbip controller Process identity |
| `device-finalizer-owner` | Device | `patch/finalizers` | Zone-scoped (own Devices) | device-usbip controller Process identity |
| `device-status-owner` | Device | `update/status`, `patch/status` | Zone-scoped (own Devices) | device-usbip controller Process identity |
| `process-owner` | Process | `get`, `list`, `watch`, `create`, `update`, `delete` | Zone-scoped (names matching `device-<uid-short>-*`) | device-usbip controller Process identity |
| `network-reader` | Network | `get`, `watch` | Zone-scoped (networks referenced by managed Devices) | device-usbip controller Process identity |

There is no direct broker channel in the RBAC table. The controller communicates
with the broker exclusively through the injected `UsbipEffectPort`; the
framework adapter holds the broker connection. The controller Process identity
does not have a direct broker credential.

No wildcard `*` over all Device resources. No cross-Zone Device access.
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
token is issued, and the controller transitions the Device to `Degraded` with
`error_class: wrong-zone`.

This invariant ensures a Device bound in Zone A cannot be accessed from Zone B:
- The proxy in Zone A binds only to the Zone A Network uplink IP.
- The firewall carve-out allows only the Zone A bridge.
- A wrong-Zone firewall request is rejected before any effect.

The `integration/wrong_zone_exposure.rs` test MUST be present and passing as a
required integration gate.

### Anti-spoofing

The adapter's `probe_device` implementation validates the physical device's
vendor/product/serial against the signed bundle's expected values. An
anti-spoofing mismatch returns `UsbipEffectError::AntiSpoofFailed` and the
Device transitions to `Failed` with `providerStatus.usbip.lastProbeResult.antiSpoofFailed: true`.

The controller never uses the spec-level `vendorId`/`productId` fields for
anti-spoofing decisions; those are configuration-level filters only. The
authoritative values come from the signed bundle.

### Firewall ownership

`Provider/device-usbip` is the **semantic owner** of the USBIP nftables
carve-out. Only the `apply_firewall` and `release_firewall` EffectPort calls
may create or remove a USBIP firewall rule. The ownership marker used in the
nftables comment (`comment "d2b managed: device-usbip/<zone>/<device-uid>"`)
is constructed and verified by the adapter, not the controller.

The adapter enforces that:
- a second `apply_firewall` for the same Device UID is idempotent (returns the
  existing `FirewallToken`);
- a `release_firewall` with a token that does not match the installed rule
  returns `UsbipEffectError::FirewallDenied`;
- a foreign rule with the same ownership marker prefix causes the adapter to
  return `UsbipEffectError::FirewallDenied` and emit a `foreign-rule-conflict`
  audit event.

### Proxy bind address

The proxy binds to the per-Zone Network uplink IP only (not `0.0.0.0`). The
adapter derives this address from Network status through its own trusted
channel; the controller never reads or passes an IP address. The proxy
Process's `endpoints` entry declares only `{name, transport, purpose}`; the
adapter resolves the actual bind address privately when the Process starts.

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
    settings = {
      networkRef = "Network/work-net";
      claimMode  = "declared";    # or "explicit"
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

### Guest claim declaration

```nix
d2b.zones.work.resources.corp-vm-usb.claims = [
  {
    claimerRef = "Guest/corp-vm";
    priority   = 100;
  }
];
```

### Guest-side module wiring (unchanged from v3 baseline)

`nixos-modules/components/usbip.nix` remains under the guest's
`runtime-cloud-hypervisor` Nix module. It wires:
- `vhci_hcd` kernel module
- `usbip` CLI tools (from the Provider package closure via the guest bundle)
- `d2b.guestControl.usbipPath` for guest-side `usbip attach`

The old `d2b.vms.<vm>.usbip.yubikey = true` option is removed at the v3 reset
boundary. The new expression is the Zone resource + claim declaration above.

### Eval-time assertions

```nix
# Mutual exclusion: same label used for USBIP and security-key in same zone
assert !(usbipLabels ∩ securityKeyLabels != {});
message = "USBIP device label '<label>' also declared as security-key in zone '<zone>'";

# controllerExecutionRef must resolve to a Host in the same zone
assert isValidHostRef cfg.providers.device-usbip.config.controllerExecutionRef;

# networkRef must reference a Network in the same zone
assert settings.networkRef zone == device zone;
```

---

## Errors

| Error class | Phase | Retryable | Notes |
| --- | --- | --- | --- |
| `unsupported-bus-class` | `Failed` | no | Only `usb` accepted for `busClass` |
| `invalid-vendor-id` | `Failed` | no | `vendorId` not exactly 4 ASCII hex digits |
| `invalid-product-id` | `Failed` | no | `productId` not exactly 4 ASCII hex digits |
| `invalid-selector-label` | `Failed` | no | Label violates ResourceName grammar |
| `network-ref-not-found` | `Degraded` | yes | `settings.networkRef` does not resolve |
| `network-not-ready` | `Pending` | yes | Network dependency not yet Ready |
| `wrong-zone` | `Degraded` | no | Device and Network in different Zones |
| `kernel-module-load-failed` | `Degraded` | yes | `ensure_kernel_module` returned error |
| `device-not-present` | `Degraded` | yes | Physical device absent from sysfs |
| `anti-spoof-failed` | `Failed` | no | Vendor/product/serial mismatch |
| `lease-denied` | `Degraded` | yes | `acquire_lease` failed; adapter contention |
| `withhold-failed` | `Degraded` | yes | sysfs write failed |
| `firewall-denied` | `Degraded` | yes | Adapter rejected `apply_firewall` |
| `firewall-foreign-conflict` | `Failed` | no | Foreign ownership marker at expected position |
| `daemon-start-failed` | `Degraded` | yes | daemon Process failed to become Ready |
| `bind-failed` | `Degraded` | yes | `bind_busid` returned error |
| `proxy-start-failed` | `Degraded` | yes | proxy Process failed to become Ready |
| `teardown-blocked` | `Degraded` | yes | One or more teardown steps failed |
| `mutual-exclusion-conflict` | `Failed` | no | Same label used for security-key in same Zone |
| `claim-arbitration-conflict` | `Pending` | yes | Second claim waiting for first to release |

---

## Current-code baseline reuse

| v3 baseline source | Reuse disposition | Notes |
| --- | --- | --- |
| `packages/d2b-contracts/src/usbip.rs` — `validate_bus_id`, `SYSFS_BUS_ID_MAX`, `UsbipClaimSource`, `sanitize_usb_hex_id` | Copy unchanged into `d2b-contracts`; reference from Provider crate | These are contracts, not broker internals; safe to reference |
| `packages/d2bd/src/usbip_state_machine.rs` — `CANONICAL_STEPS`, `UsbipBusidStep`, step ordering | Adapt step ordering into `src/reconcile.rs` EffectPort model; remove all broker-call sites | Step semantics and idempotency invariants preserved |
| `packages/d2bd/src/usbip_reconcile_state.rs` — desired/carrier/bind/proxy state enums | Map to `providerStatus.usbip.*` typed status fields | Restart-safe reconcile model preserved; state now in Device status |
| `packages/d2b-host/src/usbip_argv.rs` — argv generators | Remain in `d2b-host`; called by the core adapter only | Provider crate has no compile dependency on `d2b-host` |
| `packages/d2b-priv-broker/src/ops/usbip_firewall.rs` — `bind_firewall_rule`, audit structs | Adapter-internal only; Provider crate never imports this | Audit structs are broker-internal; `UsbipBindFirewallRuleAudit` never visible to Provider |
| `packages/d2b-priv-broker/src/ops/usbip_host.rs` — `withhold_device` impl | Adapter-internal | Same as above |
| `packages/d2b-priv-broker/src/ops/usbip_lock.rs` — OFD lock | Adapter-internal | Lock fd never leaves adapter |
| `packages/d2b-contract-tests/tests/usbip_policy_network_scoping.rs` | Adapt into integration tests at `d2b-provider-device-usbip/integration/wrong_zone_exposure.rs` | Ownership moves to Provider integration test |
| `nixos-modules/components/usbip.nix` — guest vhci_hcd + tools | Unchanged; guest runtime module stays; host-side bits removed at v3 reset | Remains under runtime-cloud-hypervisor Guest module |
| `packages/d2bd/src/usbipd_perenv_autostart.rs` — per-env autostart | Delete; replaced by controller's daemon Process lifecycle | No per-env systemd unit; Process resource owned by controller |

---

## Work items

### ADR046-usbip-001: `UsbipEffectPort` trait definition

| Field | Value |
| --- | --- |
| Title | Define `UsbipEffectPort` trait in `d2b-contracts` |
| Destination | `packages/d2b-contracts/src/usbip_effect_port.rs` |
| Depends on | `d2b-contracts` crate shape stabilised (shared root contract) |
| Source | New |
| Evidence class | ADR-only → design + implement |

Define the `UsbipEffectPort` async trait with the method set in § UsbipEffectPort.
Define `DeviceUid`, `NetworkUid`, `LeaseToken`, `FirewallToken`, `KernelModuleClass`,
`DeviceProbeResult`, `UsbipEffectError`. Export from `d2b-contracts`. No implementation
in `d2b-contracts`; trait only. Add conformance tests in `d2b-contracts/tests/usbip_effect_port.rs`.

---

### ADR046-usbip-002: Core adapter implementation

| Field | Value |
| --- | --- |
| Title | Implement `UsbipEffectPort` in framework core adapter |
| Destination | `packages/d2b-core/src/device_usbip_adapter.rs` |
| Depends on | ADR046-usbip-001; `UsbipBindFirewallRule` broker op; `usbip_argv.rs` in `d2b-host` |
| Source | Adapt: `packages/d2bd/src/usbip_state_machine.rs`, `usbip_reconcile_state.rs`, `packages/d2b-host/src/usbip_argv.rs`, `packages/d2b-priv-broker/src/ops/usbip_firewall.rs`, `usbip_host.rs`, `usbip_lock.rs` |
| Evidence class | implemented-but-unwired → adapt and wire |

Implement the adapter: busid lookup from signed bundle, same-Zone check, OFD lock
management, broker dispatch for `UsbipBindFirewallRule`, sysfs withhold, post-effect
audit emission. The adapter MUST NOT expose any raw busid, lock path, or broker wire
type to the trait caller. Add unit tests for same-Zone gate and anti-spoof logic in
`packages/d2b-core/tests/device_usbip_adapter.rs`.

---

### ADR046-usbip-003: Provider crate skeleton

| Field | Value |
| --- | --- |
| Title | Create `d2b-provider-device-usbip` crate with required layout |
| Destination | `packages/d2b-provider-device-usbip/` |
| Depends on | ADR046-usbip-001; Provider model crate structure |
| Source | New |
| Evidence class | ADR-only → implement |

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

### ADR046-usbip-004: Controller and reconcile loop

| Field | Value |
| --- | --- |
| Title | Implement Device controller and async reconcile loop |
| Destination | `packages/d2b-provider-device-usbip/src/controller.rs`, `reconcile.rs` |
| Depends on | ADR046-usbip-001, ADR046-usbip-003 |
| Source | Adapt: `packages/d2bd/src/usbip_state_machine.rs`, `usbip_reconcile_state.rs` |
| Evidence class | implemented-but-unwired → adapt |

Implement the full bring-up/teardown step machine consuming `UsbipEffectPort`.
Map `usbip_reconcile_state.rs` desired/carrier/bind/proxy states to
`providerStatus.usbip.completedSteps`. Implement finalizer add/clear. Implement
skip-already-completed-step idempotency on restart. Implement async watch receiver
that continues reading while per-resource effect tasks run (reconciliation spec
steps 10–11). Implement declared vs explicit claim mode branching.

Tests required:
- `tests/controller_state_machine.rs`: full bring-up / teardown with a `FakeUsbipEffectPort`
- `tests/async_loop.rs`: receiver dispatches Device B while Device A effect is running
- `tests/finalizer.rs`: finalizer add/clear through partial progress
- `tests/wrong_zone.rs`: WrongZone error → Degraded phase + correct error class

---

### ADR046-usbip-005: Process resource management

| Field | Value |
| --- | --- |
| Title | Implement daemon and proxy Process resource lifecycle |
| Destination | `packages/d2b-provider-device-usbip/src/reconcile.rs` |
| Depends on | ADR046-usbip-003; Process ResourceType schema |
| Source | New; templates derived from Provider package descriptor |
| Evidence class | ADR-only → implement |

Implement creation/deletion of `device-<uid-short>-daemon` and
`device-<uid-short>-proxy` Process resources using `ResourceMutationBatch`.
Populate full canonical Process specs (providerRef system-minijail, executionRef,
domain system, template ID, sandbox semantic classes, nested BudgetSpec,
networkUsage with ports array, endpoints with name/transport/purpose only,
readiness: provider-defined, restart policy) as defined in § Worker Process
resources. Confirm no `spec.command`, argv, binary path, or raw bind address
appears in any Process spec field. Guest-side attach/detach are EffectPort
calls; no Process resource is created in the Guest.

---

### ADR046-usbip-006: Typed providerStatus

| Field | Value |
| --- | --- |
| Title | Define and implement typed `providerStatus.usbip` status extension |
| Destination | `packages/d2b-provider-device-usbip/src/status.rs` |
| Depends on | ADR046-usbip-003; Device status extension schema |
| Source | Adapt: `packages/d2bd/src/usbip_reconcile_state.rs` state fields |
| Evidence class | implemented-but-unwired → adapt |

Define the `UsbipProviderStatus` struct with `currentStep`, `completedSteps`,
`lastStepOutcome`, `lastStepError`, `leaseHeld`, `firewallApplied`,
`daemonProcessRef`, `proxyProcessRef`, `claimerRef`, `attachedAt`, `labelDigest`.
No raw busid, lock path, or broker wire type in any status field.
`labelDigest` is a stable bounded hash of the logical label; not a busid.
Tests: `tests/status_serde.rs`.

---

### ADR046-usbip-007: Integration tests

| Field | Value |
| --- | --- |
| Title | Container and host integration test suite |
| Destination | `packages/d2b-provider-device-usbip/integration/` |
| Depends on | ADR046-usbip-004, ADR046-usbip-005 |
| Source | Adapt: `packages/d2b-contract-tests/tests/usbip_policy_network_scoping.rs`; new |
| Evidence class | partially ADR-only → new integration scenarios |

Required tests:

| Test file | Scenario | Gate |
| --- | --- | --- |
| `wrong_zone_exposure.rs` | Device bound in Zone A; confirm Zone B has no firewall carve-out, proxy not reachable on Zone B Network | **Required**; blocks merge |
| `declared_vs_explicit.rs` | `claimMode: declared` drives full bring-up without operator action; `explicit` pauses at bind step | Required |
| `backend_ready_probe.rs` | bind step does not start until daemon reaches Ready | Required |
| `proxy_listener.rs` | Proxy endpoint resolves to Zone A Network only; adapter does not bind on Zone B Network | Required |
| `guest_side_effects.rs` | `explicit` + `AttachDevice` calls through `UsbipGuestEffectPort`; no Process created in Guest | Required |

`integration/README.md` must document:
- how to run each scenario locally (`cargo test --test integration`);
- what container/Host/KVM privileges are required;
- how to add a new scenario;
- the wrong-zone scenario's required assertions.

---

### ADR046-usbip-008: Nix and eval assertions

| Field | Value |
| --- | --- |
| Title | Nix module updates and eval assertions |
| Destination | `nixos-modules/components/usbip.nix`, `nixos-modules/options-zones.nix`, `nixos-modules/assertions.nix` |
| Depends on | ADR046-usbip-003; ADR 0046 Nix config spec |
| Source | Adapt: `nixos-modules/components/usbip.nix` (guest keeps, host-side removed); new Zone resource declarations |
| Evidence class | partially implemented → adapt + extend |

- Add `d2b.zones.<zone>.providers.device-usbip.config.controllerExecutionRef` option.
- Remove `d2b.vms.<vm>.usbip.yubikey` at v3 reset; add deprecation warning until removal.
- Add `d2b.zones.<zone>.resources.<name>` shape for Device/device-usbip (type, spec, claims).
- Add eval-time assertions: label mutual exclusion (USBIP ∩ security-key), controllerExecutionRef Host resolution, networkRef Zone match.
- Retain guest-side `nixos-modules/components/usbip.nix` (vhci_hcd + tools) unchanged under runtime-cloud-hypervisor.
- Add or update `tests/unit/nix/cases/usbip-*.nix` for each new assertion path.

---

### ADR046-usbip-009: Removal of v3 daemon-coupled USBIP

| Field | Value |
| --- | --- |
| Title | Remove daemon-coupled USBIP from v3 d2bd and network.nix |
| Destination | `packages/d2bd/src/`, `nixos-modules/network.nix` |
| Depends on | ADR046-usbip-004, ADR046-usbip-008; Provider fully wired and validated |
| Source | `packages/d2bd/src/usbipd_perenv_autostart.rs` (delete); `nixos-modules/network.nix` lines 444–461 (remove USBIP firewall block) |
| Evidence class | implemented-and-reachable → delete after Provider replaces |

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

### Required unit tests (`tests/`)

| File | Coverage |
| --- | --- |
| `controller_state_machine.rs` | Bring-up step sequence with `FakeUsbipEffectPort` and `FakeUsbipGuestEffectPort`; teardown; step skip on restart with partial `completedSteps`; daemon/proxy Process create/delete; explicit mode calls `UsbipGuestEffectPort.attach`, no Process in Guest |
| `effect_port_contract.rs` | `UsbipEffectPort` and `UsbipGuestEffectPort` trait object safety; all method signatures callable from Provider crate; no import of broker types or `d2b-priv-broker`; `TransientDetail` Debug output is `<redacted>` |
| `conformance.rs` | Device ResourceTypeSchema serde round-trip; deny_unknown_fields; `providerStatus.usbip` JSON fidelity |
| `state_volume.rs` | Controller Volume schema conformance: `stateSchema: {}`, layout `ownerRef: User/<name>` (not ComponentPrincipal), `sensitivityClass: private`, single `state` view; no cross-component Volume; dirfd delivery to controller only |
| `status_serde.rs` | Typed `UsbipProviderStatus` JSON serialization; no raw busid in output; `labelDigest` is non-empty hash |
| `validation_corpus.rs` | Bus-id max length (31 chars); metachar rejection; leading-zero segment rejection; vendor/product id exactly 4 hex digits; `busClass != usb` → `unsupported-bus-class` |
| `mutual_exclusion.rs` | USBIP + security-key same label → `mutual-exclusion-conflict`; two USBIP claims → second in `Pending` |
| `wrong_zone.rs` | `apply_firewall` with mismatched zones → `WrongZone` error; Device phase → `Degraded`; error class `wrong-zone` in status |
| `finalizer.rs` | Finalizer added after first EffectPort step succeeds; finalizer cleared only after all teardown steps succeed; partial teardown on restart resumes at correct step |
| `async_loop.rs` | Receiver dispatches second Device reconcile while first Device's EffectPort step is awaiting; both converge concurrently |

### Required integration tests (`integration/`)

| File | Scenario | Gate |
| --- | --- | --- |
| `wrong_zone_exposure.rs` | Wrong-Zone denial end-to-end | **Required** |
| `declared_vs_explicit.rs` | declared / explicit claim mode difference | Required |
| `backend_ready_probe.rs` | bind step gated on daemon TCP readiness | Required |
| `proxy_listener.rs` | Proxy binds to correct Network uplink IP only | Required |
| `guest_side_effects.rs` | explicit + AttachDevice calls through `UsbipGuestEffectPort`; no Process created in Guest | Required |

`integration/README.md` must be present and document each scenario, required
privileges, and wrong-zone assertion requirements.

---

## Removal sequence

When `Provider/device-usbip` is fully deployed and all Device resources in all
Zones have been migrated from the v3 daemon-coupled model:

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
