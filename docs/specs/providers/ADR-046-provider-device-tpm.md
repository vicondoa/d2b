# Provider: device-tpm

| Field | Value |
| --- | --- |
| Spec ID | `ADR-046-provider-device-tpm` |
| Parent | ADR 0046 |
| Status | Proposed |
| Version | 3 |
| Baseline | `b5ddbed67867d9244bf33390868101bd9b053e49` |
| Normative | Yes |
| Owners | `d2b-provider-device-tpm` crate |
| Depends on | `ADR-046-resources-device`, `ADR-046-resources-volume`, `ADR-046-resources-host-guest-process-user`, `ADR-046-provider-model-and-packaging`, `ADR-046-provider-state`, `ADR-046-primitive-resource-composition`, `ADR-046-resource-object-model`, `ADR-046-resource-reconciliation` |
| Current code fit | Partial; v3 requires privilege-boundary inversion (controller → TpmEffectPort), controller Process resource, controller-created Device-owned Volume with exact canonical base fields, `userNamespace.mappingClass`, `tpmEndpointId` opaque handle, and crate split. |

---

## 1. Purpose

`Provider/device-tpm` manages one emulated per-Guest virtual TPM 2.0 device
backed by `swtpm`. It reconciles a `Device` resource whose claim holder
is a `Guest`. The Provider:

- discovers emulated devices (always present for `deviceClass: emulated`,
  verified against the Host's observed `tpm2` capability);
- creates and supervises the per-Device persistent TPM state `Volume`
  (controller-created, `managedBy: controller`, owned by the `Device`);
- creates and supervises the long-lived `swtpm socket` worker `Process`;
- creates and supervises a mandatory pre-start flush `EphemeralProcess`
  before each swtpm activation cycle to prevent stale session handles
  (`TPM_RC_SESSION_HANDLES`) — this flush is a **load-bearing invariant**
  and has no configurable skip path;
- publishes an opaque `tpmEndpointId` (`TpmEndpointHandle`) in Device status
  for the Guest runtime Provider to obtain the socket file descriptor via a
  sealed LaunchTicket;
- preserves TPM identity state unconditionally: a `repairPolicy: fail-closed`
  + `createPolicy: create-if-never-provisioned` Volume entry ensures a
  missing or replaced swtpm directory after first provision is a hard
  failure; the controller never silently re-provisions.

The controller communicates with privileged infrastructure **only through an
injected async `TpmEffectPort`** over opaque resource IDs. It never calls
broker operations directly, never receives socket paths, UIDs, GIDs, pidfds,
or broker wire types. `volume-local` and `system-minijail` Providers translate
resource API operations into the actual broker effects; the broker remains the
sole executor and audit owner of all privileged filesystem and process-spawn
operations.

---

## 2. Crate boundary

```text
packages/d2b-provider-device-tpm/
  src/
  tests/
  integration/
  README.md
```

Workspace policy rejects this crate if any of `src/`, `tests/`,
`integration/`, or `README.md` is absent.

Canonical internal layout (informative; workspace policy checks root-level
paths only):

```text
src/
  controller.rs        — Device reconcile loop; TpmEffectPort consumer
  effect_port.rs       — TpmEffectPort trait; TpmEndpointHandle opaque type
  effect_impl.rs       — ResourceClient-backed TpmEffectPort implementation
  status.rs            — Device status builder (tpmEndpointId, markerStatus)
  resources.rs         — Volume/Process/EphemeralProcess spec builders
  errors.rs            — TpmProviderError, TpmEffectError
  telemetry.rs         — typed OTEL span/metric helpers
  lib.rs
tests/
  controller_fsm.rs    — Device state-machine: all phase transitions
  effect_fake.rs       — FakeTpmEffectPort; no broker import
  volume_create.rs     — controller-created Volume canonical fields
  flush_mandatory.rs   — flush always issued; no skip path
  endpoint_handle.rs   — tpmEndpointId is opaque; no path string
  marker_fail_closed.rs — fail-closed marker → Device Failed
  finalizer.rs         — finalizer: Process deleted; Volume retained
  redaction.rs         — no path/UID/socket/pidfd in status/audit
  schema.rs            — Device spec admission round-trip
  nix_roundtrip.rs     — Nix form emits no Volume/Process/EphemeralProcess
integration/
  README.md
  basic_tpm_start.rs
  marker_tamper.rs
  guest_endpoint.rs
  lifecycle_restart.rs
```

---

## 3. Provider resource

### 3.1 Canonical Provider spec

```yaml
apiVersion: resources.d2b.io/v3
type: Provider
metadata:
  name: device-tpm
  zone: dev
spec:
  artifactId: d2b-provider-device-tpm
  config:
    controllerExecutionRef: "Host/host-system"   # required; Host for controller Process
    logLevel: 20                                 # swtpm --log level; 1–20; default 20
    # startupClear: REJECTED — flush always mandatory (load-bearing invariant)
    # stateDirPath: REJECTED — path is policy-derived by volume-local; never configurable
```

The Provider ResourceSpec is exactly `{ artifactId; config }` (D075). The
exported ResourceTypes and permission claims are resolved from the signed
manifest/catalog entry `artifactId` selects; they are read-only derived data,
never authored Provider spec fields. For `device-tpm` the manifest declares:

- exported ResourceType: `Device`;
- permission claims: `Device`, `Volume`, `Process`, `EphemeralProcess`
  (`get`, `list`, `watch`, `create`, `update-spec`, `update-status`, `delete`,
  `update-metadata`) and `Host`, `User` (`get`, `list`, `watch`).

### 3.2 config field reference

| Field | Type | Required | Default | Bound | Description |
| --- | --- | --- | --- | --- | --- |
| `controllerExecutionRef` | ResourceRef | yes | — | `Host/<name>` in same Zone | Host on which the controller Process is placed. Must resolve to a Ready Host with `system` in `allowedDomains`. |
| `logLevel` | uint | no | `20` | `1..20` | swtpm `--log level`; compiled into the `swtpm-socket` template by the signed component descriptor. |

`startupClear` is rejected at spec admission. The pre-start flush
(`swtpm_ioctl -i`) is compiled unconditionally into the `swtpm-init-flush`
worker template; there is no knob to disable it.

`stateDirPath` is rejected. volume-local derives the TPM state path from its
own policy, given the Volume's `source.executionRef` and `sourceId`.

Binary paths for swtpm and swtpm_ioctl are embedded in the signed component
descriptor inside the Provider's package closure; they are not configurable.

---

## 4. Controller Process

The Provider framework creates one controller Process from the Provider's
signed controller component descriptor and `config.controllerExecutionRef`.

### 4.1 Canonical controller Process spec

```yaml
apiVersion: resources.d2b.io/v3
type: Process
metadata:
  name: device-tpm-controller
  zone: dev
  ownerRef: Provider/device-tpm
spec:
  providerRef: Provider/system-minijail
  executionRef: Host/host-system        # from Provider config.controllerExecutionRef
  domain: system
  processClass: controller
  template: controller
  mounts: []                            # no Provider state Volume; operational state is in status/core ledger (D087)
  sandbox:
    namespaceClasses: [pid, mount]
    capabilityClasses: []
    seccompClass: controller-default
    noNewPrivileges: true
    startRoot: false
    readOnlyRoot: true
    environmentClass: minimal
    umask: "0022"
    oomScoreAdj: 0
  budget:
    cpu:
      request: "100m"
      limit: "500m"
    memory:
      request: "32Mi"
      limit: "128Mi"
    pids:
      limit: 64
    fds:
      limit: 128
  endpoints:
    - name: controller-api
      transport: unix
      purpose: device-tpm-controller-api
  telemetry:
    metricsEnabled: true
    tracingEnabled: true
    logLevel: info
    sensitiveLabels: false
  readiness:
    initialDelay: "0s"
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
  restartPolicy:
    class: on-failure
    backoffBase: "2s"
    backoffMax: "120s"
    backoffMultiplier: 2.0
    maxRestarts: null
    resetAfter: "3600s"
  adoptionPolicy: adopt-on-restart
  drainTimeout: "30s"
  desiredLifecycle: running
```

### 4.2 ProviderStateSet

The **ProviderStateSet** for `Provider/device-tpm` is the optional, query-time
set `{ v : Volume | v.metadata.zone == zone && v.metadata.ownerRef == "Provider/device-tpm" }`.
It is a logical grouping concept, not a ResourceType or stored artifact, and is
empty of controller state Volumes.

`Provider/device-tpm`'s controller declares **no** Provider state Volume. The
controller reconstructs all Device reconcile state from the Zone resource store
on restart, and its bounded non-secret operational state — reconcile stage,
per-Device attach/provision observations, marker-status observations, bounded
counters, and closed-enum error detail — lives in the owning resource's
`status` subresource and the core Operation ledger (D087). Because that
operational state is fully derivable from spec, `status`, the core Operation
ledger, and independent external observation, the controller-scratch payload
fails the storage-need test: there is no controller `scratch` state Volume, no
`controller-scratch` state namespace, no `scratch` mount, and no dedicated
`User/device-tpm-controller-system` state-layout principal. There is no empty
identity-only Volume.

The **TPM data Volume** described in §7 is a separate matter: it is the
per-Device swtpm NVRAM/EK-seed payload (a genuine large, secret, private
Device payload), created by the controller as a Device-owned Volume. It is
`ownerRef: Device/<name>` (not `Provider/device-tpm`), so it is not part of the
ProviderStateSet, and it is retained unchanged — it easily passes the
storage-need test as secret/large private Device state that must never enter
status.

---

## 5. TpmEffectPort — controller/privilege boundary

### 5.1 Rationale

The device-tpm controller creates Device child resources (Volume, Process,
EphemeralProcess) via a `TpmEffectPort` abstraction over the `ResourceClient`.
The downstream Providers (`volume-local`, `system-minijail`) drive their own
reconcile loops and call the broker for privileged effects. This means:

- No socket path, filesystem path, UID integer, GID integer, pidfd, or broker
  wire type ever crosses the controller/port boundary.
- `PrepareSwtpmDir` and `SpawnRunner` are invoked exclusively by `volume-local`
  and `system-minijail` respectively — never by the device-tpm controller.
- The broker remains the sole audited executor of all privileged effects.
- The controller can be tested against `FakeTpmEffectPort` without any store,
  broker, or host.

Any `use d2b_priv_broker::` in a non-test file in this crate is a workspace
policy violation.

### 5.2 TpmEffectPort trait

```rust
/// Injected async effect abstraction for the device-tpm Device controller.
/// All parameters and return values are opaque resource IDs or handles.
/// No path, UID, GID, pidfd, socket name, or broker wire type is accepted
/// or returned.
pub trait TpmEffectPort: Send + Sync + 'static {
    /// Create or verify the persistent TPM state Volume for this Device.
    /// Returns the opaque VolumeId once the Volume is Ready.
    async fn ensure_state_volume(
        &self,
        device_uid: &DeviceUid,
        execution_ref: &ResourceRef,
    ) -> Result<VolumeId, TpmEffectError>;

    /// Request creation of the swtpm worker Process.
    /// Returns the opaque ProcessId once the Process is Running (pre-Ready).
    async fn request_swtpm_process(
        &self,
        device_uid: &DeviceUid,
        volume_id: &VolumeId,
        execution_ref: &ResourceRef,
    ) -> Result<ProcessId, TpmEffectError>;

    /// Request the mandatory pre-start flush EphemeralProcess.
    /// The flush is always issued; there is no skip parameter.
    /// The ctrl socket fd is passed as a local LaunchTicket attachment;
    /// no socket path crosses this boundary.
    async fn request_flush_process(
        &self,
        device_uid: &DeviceUid,
        swtpm_process_id: &ProcessId,
        execution_ref: &ResourceRef,
    ) -> Result<EphemeralProcessId, TpmEffectError>;

    /// Stop the swtpm Process. Idempotent.
    async fn stop_swtpm_process(
        &self,
        process_id: &ProcessId,
    ) -> Result<(), TpmEffectError>;

    /// Delete the flush EphemeralProcess if present and non-terminal. Idempotent.
    async fn delete_flush_process(
        &self,
        process_id: &EphemeralProcessId,
    ) -> Result<(), TpmEffectError>;

    /// Watch the swtpm Process's tpm endpoint.
    /// Returns an opaque TpmEndpointHandle once the Process is Ready.
    /// Never returns a socket path or file descriptor number.
    async fn watch_tpm_endpoint(
        &self,
        process_id: &ProcessId,
    ) -> Result<TpmEndpointHandle, TpmEffectError>;
}

/// Opaque handle for a ready swtpm TPM socket endpoint.
/// Wraps a bounded opaque string identifier. Never a filesystem path or fd number.
/// Published as `tpmEndpointId` in Device status.
pub struct TpmEndpointHandle(pub(crate) BoundedString<128>);
```

All opaque types (`DeviceUid`, `VolumeId`, `ProcessId`, `EphemeralProcessId`,
`ResourceRef`, `TpmEndpointHandle`) are bounded string newtypes in
`d2b-contracts`. They carry no path, UID integer, or network address.

### 5.3 Port implementation

`TpmEffectPortImpl` wraps a `ResourceClient` and `WatchStream`. For each
port method it constructs the canonical resource spec (§7, §8, §9) using
only opaque IDs, issues a `Create` or `Get` call, and watches the resource
status until the relevant condition is True. The broker is never imported.

### 5.4 FakeTpmEffectPort

`FakeTpmEffectPort` records all calls in an in-memory log and returns
configurable success/failure. Controller state-machine tests run against
it without daemon, store, or broker.

---

## 6. Device resource

### 6.1 Canonical Device spec

```yaml
apiVersion: resources.d2b.io/v3
type: Device
metadata:
  name: corp-vm-tpm
  zone: dev
  ownerRef: Guest/corp-vm
spec:
  providerRef: Provider/device-tpm
  deviceClass: emulated
  arbitration: exclusive
  maxConcurrentClaims: 1
  inventory:
    selector: {}
  settings:
    logLevel: 20
    executionRef: "Host/host-system"
    # startupClear: REJECTED — flush is always mandatory
```

### 6.2 Device settings schema

| Field | Type | Required | Default | Bound | Description |
| --- | --- | --- | --- | --- | --- |
| `logLevel` | uint | no | Provider config.logLevel | `1..20` | Per-Device swtpm log level override. |
| `executionRef` | string | no | Provider config.controllerExecutionRef | `Host/<name>` | Which Host to run the swtpm Process on. Validated as a ResourceRef resolving to a Ready Host at admission. |

No binary path, UID, GID, socket path, or broker wire type is accepted in
Device spec settings.

---

## 7. TPM state Volume (controller-created)

The Device controller creates this Volume once per Device via
`TpmEffectPort.ensure_state_volume`. It is never declared in Nix.
It is separate from the ProviderStateSet (§4.2).

### 7.1 Canonical Volume spec

```yaml
apiVersion: resources.d2b.io/v3
type: Volume
metadata:
  name: device-<uid-short>-tpm-state    # uid-short = first 12 hex chars of Device UID
  zone: dev
  ownerRef: Device/corp-vm-tpm
  managedBy: controller
spec:
  providerRef: Provider/volume-local
  source:
    sourceId: "d2b/tpm-state"            # opaque policy class; volume-local resolves path
    executionRef: Host/host-system       # resolved from Device settings.executionRef
  kind: state
  layout:
    - path: ""
      type: directory
      ownerRef: User/device-<uid-short>-swtpm-system
      groupRef: User/device-<uid-short>-swtpm-system
      mode: "0700"
      sensitivity: secret-adjacent
      createPolicy: create-if-never-provisioned
      repairPolicy: fail-closed
      cleanupPolicy: never
      adoptionPolicy: quarantine-on-ambiguity
      restartPolicy: preserve-across-controller-restart
      leaseClass: none
      noFollow: true
      invariants:
        - no-symlink
        - broker-opaque-id-only
        - scope-authorization-required
  views:
    swtpm-process:
      path: ""
      rights: [read, write, create]
    controller:
      path: ""
      rights: [read, write, create, delete, traverse]
  attachments: []
  quota: null
```

### 7.2 Volume naming

`uid-short` = first 12 hexadecimal characters of the Device resource's
store-assigned UID. Stable across restarts and renames. Guest human names
never appear in the Volume name.

### 7.3 Identity marker and fail-closed detection

The identity marker is maintained by the broker (via volume-local's
`PrepareSwtpmDir` operation) outside the Volume tree — the broker-opaque-id-only
and scope-authorization-required invariants on the `""` entry enforce that
no caller below the broker can substitute or replace the swtpm directory.

The `createPolicy: create-if-never-provisioned` + `repairPolicy: fail-closed`
combination is the v3 canonical encoding of the current
`previously-provisioned-swtpm-state-missing` invariant: if the swtpm
directory is absent or its `st_ino` has changed after prior provision, the
volume-local controller sets the Volume to `Failed` with a typed condition
rather than silently re-provisioning.

| Condition | Volume phase | Device controller response |
| --- | --- | --- |
| `markerStatus: missing` after prior provision | `Failed` | Device → Failed; no auto-recovery |
| `markerStatus: replaced` (st_ino mismatch) | `Failed` | Device → Failed; no auto-recovery |
| Volume root absent, marker present | `Failed` | Device → Failed |

None of these cases auto-recover. The controller watches Volume status and
propagates `Failed` to the Device phase. The controller never issues a second
`ensure_state_volume` call after `markerStatus: replaced` or `missing` — it
transitions to Device `Failed` with condition `TpmStateCompromised` and
requires operator intervention followed by explicit Device deletion and
re-creation.

### 7.4 Key constraints

- `cleanupPolicy: never` — the entry is never removed by the Volume
  controller, even on Device or Guest deletion.
- `repairPolicy: fail-closed` — any owner/mode drift is treated as a fatal
  condition; no automatic chown is performed.
- `createPolicy: create-if-never-provisioned` — existing content is preserved
  on first bind; the broker never overwrites NVRAM.
- `adoptionPolicy: quarantine-on-ambiguity` — if the existing directory's
  identity proof is ambiguous on controller restart, the Volume is quarantined
  rather than adopted or destroyed.
- `sensitivity: secret-adjacent` — the swtpm state path must not appear in
  public status, audit records, or log output.
- `source.sourceId` is opaque; volume-local resolves the path from its
  internal policy. The device-tpm controller never provides a `hostPath`.
- No `identityMarker`, `persistenceClass`, `quotaBytes`, `stateSchema`,
  `snapshotPolicy`, or `retentionPolicy` top-level fields — those are
  ProviderStateSet extensions that do not apply to a Device-owned Volume.
- No automatic snapshots. Restoring a TPM state snapshot appears as device
  tampering to any Identity Provider and forces re-enrollment.

---

## 8. swtpm Process (controller-created)

Created via `TpmEffectPort.request_swtpm_process` once the TPM state
Volume reaches `Ready` phase.

### 8.1 Canonical Process spec

```yaml
apiVersion: resources.d2b.io/v3
type: Process
metadata:
  name: device-<uid-short>-swtpm
  zone: dev
  ownerRef: Device/corp-vm-tpm
  managedBy: controller
spec:
  providerRef: Provider/system-minijail
  executionRef: Host/host-system       # from Device settings.executionRef
  domain: system
  processClass: worker
  template: swtpm-socket
  mounts:
    - volumeRef: Volume/device-<uid-short>-tpm-state
      view: swtpm-process
      mountPath: /state
      access: read-write
      required: true
  sandbox:
    namespaceClasses: [pid, mount, user]
    capabilityClasses: []
    seccompClass: w1-swtpm
    noNewPrivileges: true
    startRoot: false
    readOnlyRoot: true
    userNamespace:
      mappingClass: process-principal-root
    environmentClass: minimal
    umask: "0022"
    oomScoreAdj: 500
  budget:
    cpu:
      request: "50m"
      limit: "500m"
    memory:
      request: "32Mi"
      limit: "128Mi"
    pids:
      limit: 32
    fds:
      limit: 64
  endpoints:
    - name: tpm
      transport: unix
      purpose: swtpm-tpm-socket
    - name: ctrl
      transport: unix
      purpose: swtpm-control-socket
  telemetry:
    metricsEnabled: true
    tracingEnabled: true
    logLevel: info
    sensitiveLabels: true
  readiness:
    initialDelay: "0s"
    timeout: "30s"
    failureThreshold: 3
    successThreshold: 1
    class: provider-defined
  healthCheck:
    enabled: false
  restartPolicy:
    class: on-failure
    backoffBase: "2s"
    backoffMax: "60s"
    backoffMultiplier: 2.0
    maxRestarts: 10
    resetAfter: "3600s"
  adoptionPolicy: adopt-on-restart
  drainTimeout: "10s"
  desiredLifecycle: running
```

### 8.2 `userNamespace.mappingClass: process-principal-root`

`mappingClass: process-principal-root` declares that the broker shall
pre-establish a single-entry user namespace mapping the process's principal
UID/GID (resolved by core from `User/device-<uid-short>-swtpm-system`) to
in-namespace UID/GID 0. Core resolves the numeric UID/GID privately; no
numeric value appears in the Process spec or status. The broker pre-establishes
the user namespace via `clone3(CLONE_NEWUSER)` + pipe sync + uid_map/gid_map
writes before swtpm's first instruction, per ADR 0021.

`namespaceClasses: [pid, mount, user]` is the matching class set; `user` is
required when `userNamespace` is set. The resulting swtpm process has zero
host capabilities (proven by `minijail_swtpm_video.rs`) and is confined to
read-only rootfs + the mounted Volume view at `/state`.

### 8.3 `readOnlyRoot: true` and Volume mount

The rootfs is mounted read-only. The only writable path is `/state`, provided
by the Volume mount (view `swtpm-process`, `path: ""`). swtpm writes NVRAM
content and its working files directly under `/state`.

### 8.4 Endpoints

Two private endpoints are declared. Neither endpoint name, socket path, nor
address appears in Device spec, Device status, or any audit payload.

| Name | Purpose | Consumer |
| --- | --- | --- |
| `tpm` | swtpm TPM socket | Guest runtime Provider; opaque `tpmEndpointId` in Device status |
| `ctrl` | swtpm control socket | Flush EphemeralProcess only; local fd attachment in LaunchTicket |

The Zone runtime assigns opaque endpoint IDs at Process creation time. The
Device controller obtains the `tpm` endpoint handle via
`TpmEffectPort.watch_tpm_endpoint` and publishes it as `tpmEndpointId` in
Device status (§10).

### 8.5 Template and argv

The exact swtpm argv is compiled by `system-minijail` from the `swtpm-socket`
template in the signed component descriptor. Template parameters are resolved
from the Volume mount path (`/state`), the endpoint socket names assigned by
the Zone runtime, the `logLevel` from Device settings, and `--flags startup-clear`
(always compiled in; not controlled by any config field). No path, UID, GID, or
binary path from spec or controller code reaches swtpm argv outside the
template compilation step.

---

## 9. Pre-start flush EphemeralProcess (mandatory; load-bearing invariant)

Before each swtpm activation cycle the controller issues a mandatory
`swtpm_ioctl -i` flush. This is a **load-bearing invariant**: without it,
an unclean prior shutdown leaves stale TPM session handles causing
`TPM_RC_SESSION_HANDLES`. There is no `startupClear` knob or any other
mechanism to disable the flush. A Device spec containing `settings.startupClear`
is rejected at admission.

### 9.1 Canonical EphemeralProcess spec

```yaml
apiVersion: resources.d2b.io/v3
type: EphemeralProcess
metadata:
  name: device-<uid-short>-flush
  zone: dev
  ownerRef: Device/corp-vm-tpm
  managedBy: controller
spec:
  providerRef: Provider/system-minijail
  executionRef: Host/host-system       # same Host as swtpm Process
  domain: system
  processClass: worker
  template: swtpm-init-flush           # always compiles --flags startup-clear
  mounts: []
  sandbox:
    namespaceClasses: [pid, mount]
    capabilityClasses: []
    seccompClass: w1-swtpm
    noNewPrivileges: true
    startRoot: false
    readOnlyRoot: true
    environmentClass: minimal
    umask: "0022"
    oomScoreAdj: 0
  budget:
    cpu:
      request: "10m"
      limit: "100m"
    memory:
      request: "8Mi"
      limit: "16Mi"
    pids:
      limit: 4
    fds:
      limit: 16
  endpoints: []
  telemetry:
    metricsEnabled: false
    tracingEnabled: true
    logLevel: info
    sensitiveLabels: true
  startDeadline: "30s"
  runtimeDeadline: "60s"
  successfulTtl: "1h"
  failedTtl: "24h"
  incidentHold: false
```

No `userNamespace` is set for the flush process; it runs as the system
principal directly. The contract test `minijail_swtpm_video.rs` proves
"user-NS long-lived only" — the user namespace applies exclusively to the
long-lived swtpm Process, not to the one-shot flush.

### 9.2 Control socket handoff (local fd attachment)

The flush EphemeralProcess receives the swtpm `ctrl` endpoint fd as an
**inherited fd attachment** in its LaunchTicket — not as a path string or
endpoint name. The Provider supervisor:

1. Reads the swtpm Process's `ctrl` endpoint opaque ID after the Process is
   Running (pre-Ready).
2. Opens the ctrl socket fd as a local LaunchTicket attachment (a validated,
   CLOEXEC-cleared fd in the inherited fd table).
3. The `swtpm-init-flush` template uses the inherited fd directly for the
   `swtpm_ioctl -i` call; no path traversal occurs.

No path string, socket name, or ctrl socket address appears in the
EphemeralProcess spec, its status, or any audit record.

### 9.3 Deadlines and TTL

| Field | Value |
| --- | --- |
| `startDeadline` | `"30s"` — max time from spec commit to process start |
| `runtimeDeadline` | `"60s"` — max wall-clock runtime for `swtpm_ioctl -i` |
| `successfulTtl` | `"1h"` — retention after Succeeded |
| `failedTtl` | `"24h"` — retention after Failed |

### 9.4 Flush ordering in the start cycle

```
Volume Ready (layoutPhase: Ready, markerStatus: verified)
  → create flush EphemeralProcess via TpmEffectPort.request_flush_process
  → wait flush.status.phase = Succeeded
  → set swtpm Process desiredLifecycle = running
  → wait swtpm Process.status.phase = Ready
  → TpmEffectPort.watch_tpm_endpoint → TpmEndpointHandle
  → Device status: tpmEndpointId = handle; phase = Ready
```

If the flush reaches `Failed`, Device transitions to `Failed` with condition
`TpmFlushFailed`. The swtpm Process is not started. The controller does not
automatically retry; it waits for the flush EphemeralProcess to be cleaned up
(after `failedTtl` expires) or for an operator-initiated Device spec update.

---

## 10. Device status

Per D088, ResourceType-common Device observation lives in
`status.resource`: the provider-neutral claim/arbitration/presence base that is
identical across Device implementations. TPM-specific observation lives only in
`status.provider` with `providerRef`, qualified `schemaId`
`device-tpm.d2b.io/Device/status`, `schemaVersion`,
`observedProviderGeneration`, and strict bounded redacted `details`
(≤32 KiB, unknown-field-denied). The controller writes all present layers
atomically in one status mutation; shared
fields are never duplicated into `status.provider`, and the extension schema is
registered and signed in the Provider manifest.

### 10.1 Common phase model

`Provider/device-tpm` uses the standard Device phase model without
Device-specific phases.

| Phase | Meaning |
| --- | --- |
| `Pending` | Controller initializing; Volume not Ready or flush not Succeeded |
| `Ready` | swtpm Process Ready; `tpmEndpointId` published; claim holder may proceed |
| `Degraded` | swtpm Process in restart backoff or health check failing |
| `Failed` | Volume marker fail-closed, flush Failed, or swtpm maxRestarts exceeded |
| `Deleted` | Terminal event only; core emits and removes the row |

### 10.2 Canonical Device status

```yaml
status:
  observedGeneration: 1
  phase: Ready
  conditions: []
  lastReconciledAt: "2026-07-22T00:00:10Z"
  startedAt: "2026-07-22T00:00:01Z"
  completedAt: null
  outcome: null
  resource:
    present: true
    health: healthy
    holderRefs:
      - Guest/corp-vm
    claims: []
    provisionedAt: "2026-07-22T00:00:05Z"
  provider:
    providerRef: Provider/device-tpm
    schemaId: "device-tpm.d2b.io/Device/status"
    schemaVersion: "1.0.0"
    observedProviderGeneration: 1
    details:
      tpm:
        stateVolumeRef: "Volume/device-7f3a9e12b4c6-tpm-state"   # ResourceRef; opaque to path
        swtpmProcessRef: "Process/device-7f3a9e12b4c6-swtpm"     # ResourceRef; for diagnostics
        markerStatus: "verified"         # verified|missing|replaced|unknown
        tpmEndpointId: "<opaque-handle>" # TpmEndpointHandle; NOT a ResourceRef; NOT a path
        lastFlushRef: "EphemeralProcess/device-7f3a9e12b4c6-flush"
        lastFlushPhase: "Succeeded"
        lastFlushAt: "2026-07-22T00:00:09Z"
```

### 10.3 Typed TPM provider details fields

The fields below are TPM-specific and therefore live under
`status.provider.details.tpm`; the Device claim/arbitration/presence base stays
promoted to `status.resource`.

| Field | Type | Description |
| --- | --- | --- |
| `stateVolumeRef` | ResourceRef string | `Volume/<name>` in same Zone. Opaque to filesystem path; never a path. |
| `swtpmProcessRef` | ResourceRef string | `Process/<name>` in same Zone. Stable for diagnostics; not a PID. |
| `markerStatus` | string | Volume marker status reflected from `Volume.status.markerStatus`: `verified`, `missing`, `replaced`, `unknown`. |
| `tpmEndpointId` | string | **Opaque endpoint handle** (`TpmEndpointHandle`). NOT a `*Ref` ResourceRef. The Guest runtime presents this to the Zone runtime to receive the TPM socket fd via a sealed LaunchTicket. No path, address, or fd number is exposed. |
| `lastFlushRef` | ResourceRef string | Most recent flush EphemeralProcess. |
| `lastFlushPhase` | string | Phase of the most recent flush at last observation. |
| `lastFlushAt` | string? | RFC 3339 UTC timestamp of last flush completion. |

`tpmEndpointId` is named `Id` (not `Ref`) because it is not a canonical
ResourceRef pointing to a typed resource. All `*Ref` fields in Device status
and spec are canonical `ResourceType/<name>` strings in the same Zone.

### 10.4 Guest runtime endpoint handoff

When the Guest runtime Provider needs the TPM socket for Cloud Hypervisor:

1. Reads `Device.status.provider.details.tpm.tpmEndpointId` (opaque bounded string).
2. Presents the opaque ID to the Zone runtime endpoint resolver.
3. Zone runtime returns an inherited fd attachment in the Guest's LaunchTicket.
4. Cloud Hypervisor receives the socket fd (not a path) via the inherited fd table.

The `--tpm socket=<path>` form is an internal detail of the Cloud Hypervisor
runtime; the path is resolved from the fd inside the LaunchTicket handling and
never appears in Device spec, Device status, or Guest spec.

---

## 11. Controller reconcile algorithm

### 11.1 Device reconcile loop

```
trigger: spec-generation-changed | dependency-changed | startup-relist | scheduled-observe

 1. Read fresh Device spec snapshot.
 2. Validate spec invariants:
    - providerRef = Provider/device-tpm; deviceClass = emulated
    - settings.executionRef resolves to Ready Host with system domain
    - settings.startupClear absent (reject if present)
 3. Resolve: executionRef = settings.executionRef ?? config.controllerExecutionRef
 4. Probe Host.status.capabilities for tpm2.
    - Absent → Device Degraded; condition TpmCapabilityAbsent; requeue 60s.
 5. TpmEffectPort.ensure_state_volume(device_uid, executionRef)
    - Volume not Ready → Device Pending; condition VolumeNotReady; return pending.
    - Volume.status.markerStatus ∈ {missing, replaced}
      → Device Failed; condition TpmStateCompromised; stop swtpm; return.
 6. Check flush EphemeralProcess:
    - Succeeded: proceed to step 8.
    - Failed: Device Failed; condition TpmFlushFailed; return.
    - Pending/Running: Device Pending; return pending (watch fires on completion).
 7. No prior flush (or prior flush cleaned up after TTL):
    a. Set swtpm Process desiredLifecycle = stopped (if running).
    b. Wait for swtpm stopped (bounded: drainTimeout + 30s).
    c. TpmEffectPort.request_flush_process(device_uid, swtpm_process_id_or_none, executionRef)
    d. Watch flush EphemeralProcess status.
    e. Succeeded → proceed to step 8.  Failed → Device Failed; condition TpmFlushFailed.
 8. TpmEffectPort.request_swtpm_process(device_uid, volume_id, executionRef)
    - Set desiredLifecycle = running.
    - Pending/Launching → Device Pending; return pending.
    - Failed → Device Failed; condition TpmProcessFailed.
 9. TpmEffectPort.watch_tpm_endpoint(swtpm_process_id) → TpmEndpointHandle
10. UpdateStatus (expected revision):
    - phase = Ready
    - status.resource.present = true; status.resource.health = healthy
    - status.provider.details.tpm.tpmEndpointId = handle (opaque string; no path)
    - status.provider.details.tpm.markerStatus from Volume.status
    - status.provider.details.tpm.stateVolumeRef / swtpmProcessRef / lastFlushRef (ResourceRefs)
11. Emit Claim for holderRef from Device ownerRef or claim request.
12. Return converged.
```

Non-blocking steps complete in ≤ 10 s. Long-running waits (Volume Ready,
flush completion, Process Ready) are async task-gated: the handler writes
a `Pending` condition and returns `pending`; the async watch task writes a
reconcile trigger when the watched condition fires.

### 11.2 Device finalizer

Registered as `device-tpm/cleanup`.

On `deletion-requested`:

1. Set swtpm Process `desiredLifecycle = stopped`.
2. Wait for swtpm Process terminal phase (bounded: `drainTimeout + 30s`).
3. Delete swtpm Process resource via resource API; wait deletion confirmed (60s).
4. Delete flush EphemeralProcess if non-terminal.
5. **Do not delete the TPM state Volume** (`cleanupPolicy: never`). Volume
   persists. Controller releases its references to the Volume.
6. Clear the `device-tpm/cleanup` finalizer.
7. Return `finalized`.

Core emits `phase=Deleted` event and removes the Device row after all
finalizers clear. The controller never manages the Deleted phase directly.

If any child resource is in ambiguous state, the finalizer returns `blocked`
with condition `device-tpm/cleanup: ambiguous-child-state`; max 5 retries
at 300 s before `failed-terminal` with explicit audit record.

---

## 12. Error classes and conditions

### 12.1 Device conditions

| Condition type | reason codes |
| --- | --- |
| `TpmCapabilityVerified` | `tpm-capability-absent`, `host-not-ready` |
| `StateVolumeReady` | `volume-not-ready`, `volume-failed`, `volume-pending` |
| `MarkerVerified` | `marker-missing`, `marker-replaced`, `marker-unknown` |
| `FlushSucceeded` | `flush-process-failed`, `flush-process-pending`, `flush-start-deadline-exceeded` |
| `SwtpmReady` | `swtpm-process-failed`, `swtpm-process-pending`, `swtpm-process-crashed` |
| `EndpointReady` | `endpoint-not-ready`, `endpoint-resolve-failed` |
| `ClaimAdmitted` | `device-claim-already-held`, `device-claim-invalid` |

### 12.2 Error codes

| Code | Retryable | Description |
| --- | --- | --- |
| `tpm-capability-absent` | yes | Host capability `tpm2` not observed. |
| `volume-marker-missing` | no | Marker absent after prior provision. |
| `volume-marker-replaced` | no | st_ino mismatch: swtpm directory replaced. |
| `flush-process-failed` | no | `swtpm_ioctl -i` exited non-zero or timed out. |
| `swtpm-process-crashed` | yes | swtpm exited with signal before Ready. |
| `swtpm-max-restarts-exceeded` | no | swtpm restart count > `maxRestarts`. |
| `endpoint-resolve-failed` | yes | TpmEndpointHandle could not be resolved. |
| `device-claim-already-held` | no | Exclusive claim already active. |
| `host-not-ready` | yes | executionRef Host not in Ready phase. |
| `effect-port-error` | yes | TpmEffectPort returned a transient error. |

Non-retryable errors transition Device to `Failed`. `volume-marker-missing`,
`volume-marker-replaced`, and `swtpm-max-restarts-exceeded` require operator
action; the controller does not auto-recover.

---

## 13. Audit events

All payloads carry `sensitivity: private`. No path, socket name, UID, GID,
PID, argv fragment, TPM NVRAM content, or raw binary data appears in any
audit payload.

| Event | Stable kind | Payload fields |
| --- | --- | --- |
| Device first provision | `device-tpm/state-volume-created` | `zone`, `device_uid` (opaque), `volume_uid` (opaque) |
| Marker verified on start | `device-tpm/marker-verified` | `zone`, `device_uid`, `volume_uid` |
| Marker fail-closed | `device-tpm/marker-fail-closed` | `zone`, `device_uid`, `volume_uid`, `marker_status` |
| Flush succeeded | `device-tpm/flush-succeeded` | `zone`, `device_uid`, `process_uid` (opaque) |
| Flush failed | `device-tpm/flush-failed` | `zone`, `device_uid`, `process_uid`, `exit_class` |
| swtpm started | `device-tpm/swtpm-started` | `zone`, `device_uid`, `process_uid`, `adoption_state` |
| swtpm crashed | `device-tpm/swtpm-crashed` | `zone`, `device_uid`, `process_uid`, `exit_class`, `restart_count` |
| swtpm max restarts | `device-tpm/swtpm-max-restarts-exceeded` | `zone`, `device_uid`, `process_uid` |
| Claim granted | `device-tpm/claim-granted` | `zone`, `device_uid`, `holder_ref` (opaque ResourceRef) |
| Claim released | `device-tpm/claim-released` | `zone`, `device_uid`, `holder_ref` |
| Finalizer cleared | `device-tpm/finalized` | `zone`, `device_uid`, `volume_retained: true` |
| Security violation | `device-tpm/security-violation` | `zone`, `device_uid`, `violation_class` |

`device_uid`, `volume_uid`, `process_uid`, `holder_ref` are opaque
store-assigned UIDs — not human names, filesystem paths, or socket addresses.

---

## 14. Observability (OTEL)

### 14.1 Metrics

| Metric | Type | Labels | Description |
| --- | --- | --- | --- |
| `d2b_device_tpm_phase` | gauge | `zone`, `phase` | Device count per phase |
| `d2b_device_tpm_flush_duration_seconds` | histogram | `zone`, `outcome` | Pre-start flush wall-clock duration |
| `d2b_device_tpm_swtpm_restart_count` | counter | `zone` | swtpm restart events |
| `d2b_device_tpm_marker_status` | gauge | `zone`, `status` | TPM state Volume marker status |

No label carries a device name, VM name, path, process name, UID, GID,
PID, or socket address.

### 14.2 Traces

| Span | Description |
| --- | --- |
| `device-tpm.reconcile` | Full Device reconcile cycle |
| `device-tpm.ensure-state-volume` | `ensure_state_volume` call |
| `device-tpm.flush` | EphemeralProcess flush lifecycle |
| `device-tpm.swtpm-start` | swtpm Process start or adopt |
| `device-tpm.endpoint-ready` | TpmEndpointHandle resolution |
| `device-tpm.finalize` | Finalizer execution |

Span attributes: `zone` (string), `device_uid` (opaque ≤ 64 chars),
`outcome` (bounded stable label). No path, argv, socket, UID, GID, or PID.

---

## 15. RBAC

### 15.1 Provider-declared Roles

#### `device-tpm-operator`

```json
{
  "rules": [
    {
      "resourceTypes": ["Device"],
      "verbs": ["get", "list", "watch", "create", "update-spec", "delete"],
      "providerConstraint": "device-tpm"
    },
    {
      "resourceTypes": ["Guest"],
      "verbs": ["get", "list"]
    }
  ]
}
```

#### `device-tpm-viewer`

```json
{
  "rules": [
    {
      "resourceTypes": ["Device"],
      "verbs": ["get", "list", "watch"],
      "providerConstraint": "device-tpm"
    }
  ]
}
```

### 15.2 Controller-internal permissions

Declared in the Provider descriptor; granted by the Provider framework:

| ResourceType | Verbs | Scope |
| --- | --- | --- |
| `Device` | `get`, `list`, `watch`, `update-status`, `update-metadata` | `providerConstraint: device-tpm` |
| `Volume` | `get`, `list`, `watch`, `create`, `update-status`, `update-metadata` | `ownerConstraint: Device/<any>` |
| `Process` | `get`, `list`, `watch`, `create`, `update-spec`, `delete` | `ownerConstraint: Device/<any>` |
| `EphemeralProcess` | `get`, `list`, `watch`, `create`, `delete` | `ownerConstraint: Device/<any>` |
| `Host` | `get`, `list`, `watch` | read-only; for capability check |
| `User` | `get`, `list`, `watch` | read-only; for principal resolution |

The controller holds **no** permissions to call broker operations. Broker
permissions belong exclusively to `Provider/volume-local` and
`Provider/system-minijail`.

---

## 16. Async reconcile patterns

### 16.1 Watch-gated async tasks

Long-running waits (Volume Ready, flush Succeeded, Process Ready) do not
block the controller's reconcile queue. The reconcile handler starts an
async watch task, writes a `Pending` condition, and returns `pending`. When
the watch fires, the task writes a reconcile trigger. The reconcile loop
re-enters and short-circuits the wait.

### 16.2 Bounded concurrency

At most 32 concurrent Device reconcile tasks (semaphore-guarded). Per-Device
TpmEffectPort calls are sequential; no concurrent calls for the same Device UID.

### 16.3 Restart adoption

After controller restart:
1. Re-list all Device resources.
2. For each Ready/Degraded Device: `ensure_state_volume` and
   `request_swtpm_process` use `Get` semantics (resources already exist).
3. If swtpm Process is already Ready: re-fetch the endpoint handle and
   publish `tpmEndpointId`; no new flush.
4. New flush issued only if swtpm Process is not running or has exited.

### 16.4 Idempotency

All controller effects are idempotent:
- `ensure_state_volume`: `Create` is a no-op if Volume exists with the same
  name and owner. Port returns existing `VolumeId`.
- `request_swtpm_process`: `Create` is a no-op if Process already exists.
- `request_flush_process`: controller deletes prior EphemeralProcess before
  issuing a new `Create`; name `device-<uid-short>-flush` is stable.
- `stop_swtpm_process`: no-op if already stopped or Process absent.

---

## 17. Nix authoring form

### 17.1 What operators declare

Operators declare **only** the Device resource. The TPM state Volume, swtpm
Process, and flush EphemeralProcess are **never** declared in Nix. They are
`managedBy: controller` resources; the Nix emitter does not emit them.

```nix
d2b.zones.dev.resources."corp-vm-tpm" = {
  type = "Device";
  metadata.ownerRef = "Guest/corp-vm";
  spec = {
    providerRef = "Provider/device-tpm";
    deviceClass = "emulated";
    arbitration = "exclusive";
    maxConcurrentClaims = 1;
    inventory.selector = {};
    settings = {
      logLevel = 20;
      # executionRef may be omitted to inherit Provider config.controllerExecutionRef
    };
  };
};
```

The Guest Provider declares the Device attachment:

```nix
d2b.zones.dev.resources."corp-vm" = {
  type = "Guest";
  spec = {
    providerRef = "Provider/runtime-cloud-hypervisor";
    deviceAttachments = [
      { deviceRef = "Device/corp-vm-tpm"; exclusive = true; }
    ];
  };
};
```

### 17.2 Provider config in Nix

```nix
d2b.zones.dev.resources."device-tpm" = {
  type = "Provider";
  spec = {
    artifactId = "d2b-provider-device-tpm";
    config = {
      controllerExecutionRef = "Host/host-system";
      logLevel = 20;
    };
  };
};
```

### 17.3 Migration from current form

Current: `d2b.vms.corp-vm.tpm.enable = true` (in `nixos-modules/components/tpm.nix`).

v3: the Nix Device declaration in §17.1 replaces this option. Migration steps:
- Remove `d2b.vms.<vm>.tpm.enable`.
- Declare Device resource per §17.1.
- The existing `/var/lib/d2b/vms/<vm>/swtpm/` directory is migrated to the
  controller-created Volume path via a one-time migration EphemeralProcess.
- The existing provisioning marker in `swtpm-markers/<vm>` must be preserved
  (re-keyed by volume-local from the old basename to the new `device_uid`-based
  name). A missing or dropped marker fails the Volume provision fail-closed —
  no silent re-creation.

---

## 18. Current-code fit

| Current artifact | Location | v3 disposition |
| --- | --- | --- |
| `SwtpmArgvInput`, `SwtpmIoctlFlushInput` | `packages/d2b-host/src/swtpm_argv.rs` | Extract into `d2b-provider-device-tpm/src/`; remove caller-supplied binary path fields |
| `PrepareSwtpmDir` | `packages/d2b-priv-broker/src/ops/swtpm_dir.rs` | Retained; invoked only by `volume-local`; device-tpm controller never calls it |
| `SpawnRunner { role: Swtpm }` | `packages/d2b-priv-broker/src/ops/spawn_runner.rs` | Retained; invoked only by `system-minijail` |
| `ProcessRole::Swtpm`, `::SwtpmPreStartFlush` | `packages/d2b-core/src/processes.rs` | Retire after Provider parity |
| `minijail_swtpm_video.rs` | `packages/d2b-contract-tests/tests/` | Preserved; proves zero caps, `w1-swtpm`, user-NS long-lived only |
| `policy_swtpm_readiness.rs` | `packages/d2b-contract-tests/tests/` | Preserved; socket-ready predicate |
| `nixos-modules/components/tpm.nix` | `nixos-modules/components/tpm.nix` | Replaced by Device Nix declaration (§17.1) |
| Direct broker/swtpm call sites in daemon | `packages/d2bd/src/*` | Remove; device-tpm controller uses TpmEffectPort |

---

## 19. Work items

### ADR046-device-tpm-001 — Crate scaffold

| Field | Value |
| --- | --- |
| Priority | P0 |
| Blocked by | — |
| Evidence class | implemented-and-reachable |

Create `packages/d2b-provider-device-tpm/` with `src/`, `tests/`,
`integration/README.md`, `README.md`. Add to Cargo workspace. Workspace
policy test must pass.

### ADR046-device-tpm-002 — TpmEffectPort and FakeTpmEffectPort

| Field | Value |
| --- | --- |
| Priority | P0 |
| Blocked by | ADR046-device-tpm-001 |
| Evidence class | implemented-and-reachable |

Implement `TpmEffectPort` trait, `TpmEndpointHandle` opaque type, and
`FakeTpmEffectPort`. Prove: no `use d2b_priv_broker::` in non-test files.

### ADR046-device-tpm-003 — Controller reconcile state machine

| Field | Value |
| --- | --- |
| Priority | P0 |
| Blocked by | ADR046-device-tpm-002 |
| Evidence class | implemented-and-reachable |

Implement Device reconcile algorithm (§11.1) against `FakeTpmEffectPort`.
Tests cover: happy path, Volume not-ready, marker fail-closed, flush failed,
swtpm maxRestarts, finalizer (Process deleted; Volume retained).

### ADR046-device-tpm-004 — Controller-created Volume spec

| Field | Value |
| --- | --- |
| Priority | P0 |
| Blocked by | ADR046-device-tpm-001 |
| Evidence class | implemented-and-reachable |

Implement `build_tpm_state_volume_spec` in `resources.rs`. Tests prove:
- `cleanupPolicy: never`; `repairPolicy: fail-closed`;
  `adoptionPolicy: quarantine-on-ambiguity`; `sensitivity: secret-adjacent`.
- `invariants` includes `no-symlink`, `broker-opaque-id-only`,
  `scope-authorization-required`.
- `source.sourceId` present; no `hostPath` field.
- No `identityMarker`/`persistenceClass`/`quotaBytes`/`stateSchema` top-level fields.
- `ownerRef: Device/<name>`; `managedBy: controller`.
- `attachments: []`; `quota: null`.

### ADR046-device-tpm-005 — Canonical swtpm Process spec

| Field | Value |
| --- | --- |
| Priority | P0 |
| Blocked by | ADR046-device-tpm-004 |
| Evidence class | implemented-and-reachable |

Implement `build_swtpm_process_spec` in `resources.rs`. Tests prove:
- `readOnlyRoot: true`.
- `userNamespace: {mappingClass: process-principal-root}` (no numeric UID/GID fields).
- `namespaceClasses: [pid, mount, user]`; `capabilityClasses: []`;
  `seccompClass: w1-swtpm`.
- Two `EndpointSpec` entries: `tpm` and `ctrl`; no path.
- `mounts[0].required: true` (canonical MountSpec field).
- No socket path, binary path, UID integer, or GID integer in any spec field.

### ADR046-device-tpm-006 — Mandatory flush EphemeralProcess spec

| Field | Value |
| --- | --- |
| Priority | P0 |
| Blocked by | ADR046-device-tpm-003 |
| Evidence class | implemented-and-reachable |

Implement `build_flush_ephemeral_process_spec` in `resources.rs`. Tests prove:
- No `startupClear` field anywhere in spec or controller code.
- Flush always created before swtpm Process start; no code path skips it.
- `successfulTtl: "1h"`; `failedTtl: "24h"` exactly.
- No `userNamespace` on flush Process (user-NS long-lived only, per contract test).
- `startDeadline: "30s"`; `runtimeDeadline: "60s"`.

### ADR046-device-tpm-007 — Device status builder

| Field | Value |
| --- | --- |
| Priority | P1 |
| Blocked by | ADR046-device-tpm-003 |
| Evidence class | implemented-and-reachable |

Implement `build_device_status` in `status.rs`. Tests prove:
- `tpmEndpointId` is an opaque bounded string — NOT a `*Ref` ResourceRef field;
  never a filesystem path.
- `stateVolumeRef` and `swtpmProcessRef` are canonical `ResourceType/<name>` strings.
- No path, socket name, UID, GID, PID, or pidfd in any status field.
- `markerStatus` carries only: `verified`, `missing`, `replaced`, `unknown`.

### ADR046-device-tpm-008 — Endpoint opaque handoff

| Field | Value |
| --- | --- |
| Priority | P1 |
| Blocked by | ADR046-device-tpm-007 |
| Evidence class | implemented-and-reachable |

Hermetic test: `tpmEndpointId` is opaque; no path. Integration test:
Guest runtime Provider reads `tpmEndpointId` and obtains socket fd from Zone
runtime endpoint resolver; no path string in Guest spec or LaunchTicket API
surface.

### ADR046-device-tpm-009 — Marker fail-closed test

| Field | Value |
| --- | --- |
| Priority | P0 |
| Blocked by | ADR046-device-tpm-004 |
| Evidence class | implemented-and-reachable |

Hermetic: FakeTpmEffectPort returns `markerStatus: replaced` → Device Failed;
no second `ensure_state_volume` call; swtpm Process not created.

Integration: physically replace swtpm/ dir → volume-local sets Volume Failed →
Device Failed; no auto-recovery.

### ADR046-device-tpm-010 — Controller Process (status-first; no Provider state Volume)

| Field | Value |
| --- | --- |
| Priority | P1 |
| Blocked by | ADR046-device-tpm-001 |
| Evidence class | implemented-and-reachable |

Implement controller Process spec (§4.1). Tests prove:
- `processClass: controller`; `readOnlyRoot: true`.
- `mounts` is empty: the controller declares no Provider state Volume; its
  bounded non-secret operational state lives in the owning resource's `status`
  subresource and the core Operation ledger (D087). No `controller-scratch`
  namespace, no `scratch` mount, and no `User/device-tpm-controller-system`
  state-layout principal exist.
- The device-tpm controller holds no `create` permission for Volumes with
  `ownerRef: Provider/<any>`; it creates only the per-Device TPM data Volume
  with `ownerRef: Device/<any>` (see §7 and §15.2), which is not part of the
  ProviderStateSet.
- On restart the controller re-derives Device reconcile state from the Zone
  resource store and reverifies against external reality (marker checks, running
  swtpm processes), treating `status` as observation, never authority.

### ADR046-device-tpm-011 — Nix roundtrip test

| Field | Value |
| --- | --- |
| Priority | P1 |
| Blocked by | ADR046-device-tpm-001 |
| Evidence class | implemented-and-reachable |

Device Nix spec (§17.1) round-trips through the Nix emitter to expected
resource JSON. Emitted bundle contains no Volume, Process, or EphemeralProcess
resources (`managedBy: controller` resources are not in the Nix bundle).

### ADR046-device-tpm-012 — Finalizer: Volume retained on Device deletion

| Field | Value |
| --- | --- |
| Priority | P0 |
| Blocked by | ADR046-device-tpm-003 |
| Evidence class | implemented-and-reachable |

Hermetic: Device deletion finalizer → swtpm Process deleted → TPM state
Volume NOT deleted (`cleanupPolicy: never`) → Volume persists. Core emits
`phase=Deleted` for Device after finalizer clears. No path/UID in audit event.

### ADR046-device-tpm-013 — Remove direct broker references

| Field | Value |
| --- | --- |
| Priority | P0 |
| Blocked by | ADR046-device-tpm-002 |
| Evidence class | implemented-and-reachable |

Remove direct broker call sites for swtpm from pre-ADR-0046 daemon path.
Retire `ProcessRole::Swtpm` and `::SwtpmPreStartFlush` from `d2b-core`.
Move argv builders from `d2b-host/src/swtpm_argv.rs` to
`d2b-provider-device-tpm/src/` with binary path fields removed.
`d2b-priv-broker/src/ops/swtpm_dir.rs` and `spawn_runner.rs` retained
(used by `volume-local` and `system-minijail` respectively).

---

## 20. Tests

### 20.1 Hermetic (Cargo) — `tests/`

| Test file | What it proves |
| --- | --- |
| `controller_fsm.rs` | All Device reconcile state transitions; FakeTpmEffectPort |
| `effect_fake.rs` | FakeTpmEffectPort records all calls; no broker import in crate |
| `volume_create.rs` | Volume canonical fields: `cleanupPolicy: never`, `repairPolicy: fail-closed`, `sensitivity: secret-adjacent`, correct invariants, no ProviderStateSet extensions |
| `flush_mandatory.rs` | Flush always issued; no skip; no `startupClear`; correct TTLs; no userNamespace |
| `endpoint_handle.rs` | `tpmEndpointId` is opaque bounded string; NOT a ResourceRef; no path |
| `marker_fail_closed.rs` | Marker replaced/missing → Device Failed; no auto-recovery; no second `ensure_state_volume` |
| `finalizer.rs` | Process deleted; Volume retained; Deleted emitted by core |
| `redaction.rs` | No path/UID/socket/pidfd in status, audit span attrs, or log records |
| `schema.rs` | Device spec admission round-trip through JSON schema |
| `nix_roundtrip.rs` | Nix form emits no Volume/Process/EphemeralProcess resources |

### 20.2 Integration — `integration/`

| Test file | What it proves |
| --- | --- |
| `basic_tpm_start.rs` | Host fixture: Device → Volume Ready → flush Succeeded → swtpm Ready |
| `marker_tamper.rs` | Replace swtpm/ dir → Volume Failed → Device Failed; no auto-recovery |
| `guest_endpoint.rs` | Guest reads opaque `tpmEndpointId`; receives socket fd; no path in LaunchTicket |
| `lifecycle_restart.rs` | Controller restart: adopts swtpm Process; Volume retained; no double-flush |

### 20.3 Existing contract tests (preserved)

| Test file | Location | Invariants proved |
| --- | --- | --- |
| `minijail_swtpm_video.rs` | `d2b-contract-tests/tests/` | Zero host caps; `w1-swtpm` seccomp; user-NS long-lived only (flush has no userNamespace) |
| `policy_swtpm_readiness.rs` | `d2b-contract-tests/tests/` | Unix socket readiness predicate |
| `swtpm_dir.rs` unit tests | `d2b-priv-broker/src/ops/` | Tamper marker; fresh/existing dir; symlink/mismatch fail-closed |

---

## 21. Removal plan

When this Provider reaches `Evidence class: implemented-and-reachable`:

1. Remove `d2b.vms.<vm>.tpm.enable` from `nixos-modules/components/tpm.nix`.
2. Retire `ProcessRole::Swtpm` and `::SwtpmPreStartFlush` from `d2b-core`.
3. Move `SwtpmArgvInput`, `SwtpmIoctlFlushInput` from `d2b-host/src/swtpm_argv.rs`
   to `d2b-provider-device-tpm/src/`; remove binary path fields.
4. Remove direct swtpm call sites from `packages/d2bd/src/`.
5. Update `CHANGELOG.md` under `## [Unreleased]`.
6. Emit migration guide entry in `docs/how-to/migrate-d2b-v2-to-v3.md`.
7. Update `docs/specs/ADR-046-resources-device.md` device-tpm section to
   reference this dossier as canonical source.
