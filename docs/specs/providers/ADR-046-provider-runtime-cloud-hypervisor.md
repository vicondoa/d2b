# ADR 0046 Provider dossier: `runtime-cloud-hypervisor`

| Field | Value |
| --- | --- |
| Spec ID | `ADR-046-provider-runtime-cloud-hypervisor` |
| Parent | ADR 0046 |
| Status | Proposed |
| Version | 1 |
| Baseline | `b5ddbed67867d9244bf33390868101bd9b053e49` |
| Main reuse | Permitted; exact commit and selected behavior named per work item |
| Normative | Yes |
| Owners | `packages/d2b-provider-runtime-cloud-hypervisor/` (future crate) |
| Depends on | `ADR-046-resource-object-model`, `ADR-046-resource-reconciliation`, `ADR-046-components-processes-and-sandbox`, `ADR-046-provider-model-and-packaging`, `ADR-046-provider-state`, `ADR-046-resources-host-guest-process-user`, `ADR-046-resources-network`, `ADR-046-resources-volume`, `ADR-046-resources-device`, `ADR-046-componentsession-and-bus`, `ADR-046-nix-configuration`, `ADR-046-telemetry-audit-and-support`, `ADR-046-primitive-resource-composition` |
| Supersedes | `packages/d2b-host/src/runtime_provider.rs` `CloudHypervisorRuntimeProvider`; `packages/d2bd/src/` VM lifecycle paths; `d2b-host-providers` adapter; `ProcessRole::CloudHypervisor`, `ProcessRole::Swtpm`, `ProcessRole::NetVm`; systemd unit `d2b-<vm>-vm.service`; `SwtpmDir` broker op |

---

## 1 Purpose and scope

`Provider/runtime-cloud-hypervisor` reconciles `Guest` resources whose
`spec.providerRef` is `Provider/runtime-cloud-hypervisor`. For each such Guest
it:

- asserts an owned VMM `Process` resource and observes Device, Network, and
  Volume dependency readiness through `ResourceClient` before launching it;
- supervises the Cloud Hypervisor VMM process as a long-lived `Process`;
- presents the running guest to the Zone resource plane with typed health status
  and conditions;
- tears down the VMM Process in finalizer-safe order on deletion.

This Provider does **not**:

- reconcile `Network`, `Volume`, `Device`, or `Credential` resources;
- open broker sockets or issue broker operations directly — all privileged
  effects are mediated by the selected `Process` Provider
  (`Provider/system-minijail`);
- expose or manage virtiofsd processes — those belong to
  `Provider/volume-virtiofs` per the Volume spec;
- manage swtpm state Volumes — the `device-tpm` Provider owns the swtpm
  `Process` and its persistent `Volume`;
- embed GPU/video/audio/display/transport child resources — those are reconciled
  by their respective Providers which expose `Device` and other ResourceTypes to
  this Guest's bootstrap graph as dependencies.

---

## 2 Crate and package boundary

```text
packages/d2b-provider-runtime-cloud-hypervisor/
  src/
  tests/
  integration/
  README.md

The workspace policy gate rejects the crate if any of these four top-level paths
is absent. A nested `integration/README.md` is recommended but optional and is
not enforced by the policy gate.
```

- `src/`: controller binary, guest-bootstrap actor, VMM process template
  builder, reconcile/observe/finalize handlers, config schema, internal
  modules, and colocated unit tests.
- `tests/`: hermetic Cargo integration tests — ResourceType conformance,
  fault/retry/restart scenarios, redaction, schema golden vectors, fake-port
  bus tests, and pidfd adoption property tests.
- `integration/`: heavier container/Host/Guest/cross-process fixtures invoked
  by existing repository test orchestration (`make test-integration`,
  `make test-host-integration`).
- `README.md`: Provider identity/config, ResourceTypes, controllers/services/
  workers/binaries, placement, dependencies/RBAC, security/state/telemetry,
  build/test/integration commands, and standalone-repository consumption.

The workspace policy gate rejects the crate if any of these four top-level paths
is absent. The crate declares exactly one Provider identity
(`Provider/runtime-cloud-hypervisor`) and may not import d2bd, broker,
Zone-store, or another Provider's implementation internals.

---

## 3 Provider resource and installation

### 3.1 Installation

`Provider/runtime-cloud-hypervisor` is installed as a Zone-local resource:

```yaml
apiVersion: resources.d2bus.org/v3
type: Provider
metadata:
  name: runtime-cloud-hypervisor
  zone: dev
spec:
  artifactId: provider-runtime-cloud-hypervisor
  config: {}
```

`spec.artifactId` references a `d2b.artifacts.<id>` catalog entry with
`type = "provider"`. The build validates this at derivation time; a missing or
wrong-type ID is a hard build error. The artifact catalog stores the store path
privately at `root:d2bd` 0640; it never appears in the ResourceSpec,
status, or any audit field.

Package presence alone is not installation. `providerRef:
Provider/runtime-cloud-hypervisor` in a Guest spec resolves only a Ready
`Provider/runtime-cloud-hypervisor` resource in the same Zone. Installation
fails closed if the artifact digest is untrusted, the signature is invalid, or
the conformance attestation is absent.

### 3.2 Provider root config schema

```yaml
config:
  controllerExecutionRef: "Host/my-host"  # required; explicit Host ref; never resolved implicitly
  # All remaining fields are optional; defaults shown.
  defaultVcpus: 2             # int [1, 1024]; inherited by Guests without explicit vcpus
  defaultMemoryMb: 512        # int [128, 524288]
  defaultMachineType: q35     # q35 | microvm
  watchdog: true              # bool; CH software watchdog emitted to all VMMs
  adoptionWindow: "30s"       # duration; maximum pidfd adoption wait after controller restart
  healthCheckInterval: "30s"  # duration [5s, 300s]; guest-control health polling period
  healthCheckTimeout: "5s"    # duration [1s, 60s]
  healthCheckFailureThreshold: 3
  startupDeadline: "120s"     # duration; maximum time from Process Ready to bootstrapReady
```

`controllerExecutionRef` is required and must name an explicit `Host` resource
in the same Zone. The controller uses it as `executionRef` for every VMM
`Process` it creates. It is never inferred from a Zone default or ambient Host
discovery; a missing or non-existent `Host` reference fails Provider installation
with reason `controller-execution-ref-invalid`.

All other fields are optional and bounded. Secrets are never root config values.

### 3.3 Provider status

The aggregate Provider status — `phase`, conditions (`ControllerRunning`,
`ArtifactTrusted`, `ConformancePassed`), and component readiness — is managed
entirely by the **ProviderDeployment** framework component, not by the
controller process. The controller process writes only `Guest.status` fields
and creates/updates/deletes child `Process` resources; it does not write to
`Provider.status`.

The ProviderDeployment derives Provider phase and component health by watching
the controller component's `Process` liveness and the signed descriptor
conformance record. An illustrative (framework-written) status:

```yaml
status:
  phase: Ready
  conditions:
    - type: ControllerRunning
      status: "True"
      reason: controller-process-ready
    - type: ArtifactTrusted
      status: "True"
      reason: digest-verified
    - type: ConformancePassed
      status: "True"
      reason: conformance-attestation-valid
  components:
    - id: controller
      phase: Ready
      processRef: Process/runtime-ch-controller
  exportedResourceTypes:
    - Guest
  providerGeneration: 1
```

---

## 4 Guest ResourceSpec: `runtime-cloud-hypervisor` extension

`Guest.spec.provider.settings` is validated against the runtime-cloud-hypervisor
exported Guest spec schema. Unknown fields inside `spec.provider.settings` are
rejected.

**D089 spec extension contract:** this Provider's implementation-only desired
configuration is carried in `spec.provider.settings` under
`runtime-cloud-hypervisor.d2bus.org/Guest/spec`; the schema is registered/signed in
the manifest, deny-unknown, bounded, versioned, and validated against
`spec.providerRef` at Nix build and API admission. Base fields stay at `spec.*`;
shared semantics are promoted to the Guest base and never placed in
`spec.provider`. The Provider implements the exact base spec/status schema
version/fingerprint, accepts the canonical minimal valid base Spec, and rejects
an unsupported optional base capability only through its signed capability matrix
plus provider-neutral `unsupported-capability`. `spec.provider` aligns with
`status.provider` for `Provider/runtime-cloud-hypervisor`.

### 4.1 `spec.provider.settings` schema

```yaml
provider:
  schemaId: runtime-cloud-hypervisor.d2bus.org/Guest/spec
  schemaVersion: 1.0.0
  settings:
    vcpus: 2                  # int [1, 1024]; overrides Provider root default
    memoryMb: 512             # int [128, 524288]; overrides Provider root default
    machineType: q35          # q35 | microvm; overrides root default
    consoleType: virtio       # virtio | serial; default virtio
    serialPort: false         # bool; emit --serial null (default) or --serial tty
    pvpanic: false            # bool; emit --pvpanic device
    watchdogOverride: null    # bool | null; null = inherit Provider root watchdog setting
    memoryShared: true        # bool; must remain true for virtiofs; hard-fail if false and
                              # any Volume attachment uses virtiofs transport
```

**Required fields**: no provider settings are required for every
`runtime-cloud-hypervisor` Guest. Guest-control addressing is expressed by an
owned `Endpoint` resource, not by a raw CID in `spec.provider.settings`. All
settings inherit Provider root defaults unless set.

**Invariants enforced at `validateSpec` time**:

1. `vcpus` must be ≥ 1 and ≤ 1024.
2. `memoryMb` must be ≥ 128 and ≤ 524288.
3. `memoryShared: false` combined with any virtiofs Volume attachment is a hard
   spec error at admission time.
4. Any Guest-control transport identity must be represented as an owned
   `Endpoint` resource; raw CIDs, socket paths, or ports are rejected from
   Guest spec and status.

No raw argv string, store path, host path, socket path, credential bytes,
broker operation name, or free-form kernel cmdline fragment is accepted in
`spec.provider.settings`. Sandbox parameters (seccomp filter, capability set,
namespace classes) are fixed by the signed Process template and cannot be
overridden by Guest settings.

### 4.2 Required top-level `systemArtifactId`

`Guest.spec.systemArtifactId` is a **top-level `spec` field** (not inside
`spec.provider.settings`). It is a plain bounded ID string referencing a
`d2b.artifacts.<id>` catalog entry with `type = "nixos-system"`.

```yaml
spec:
  providerRef: Provider/runtime-cloud-hypervisor
  systemArtifactId: dev-vm-system   # required for this Provider; NOT in spec.provider.settings
  provider:
    schemaId: runtime-cloud-hypervisor.d2bus.org/Guest/spec
    schemaVersion: 1.0.0
    settings:
      vcpus: 4
      memoryMb: 4096
```

For `runtime-cloud-hypervisor`, `systemArtifactId` is **required**. A Guest
spec with `systemArtifactId: null` and `providerRef:
Provider/runtime-cloud-hypervisor` fails admission with reason
`system-artifact-required`.

At build time the Nix compiler resolves the artifact catalog entry:
- derives the kernel, initrd, and rootfs paths privately into the artifact
  catalog under `root:d2bd` 0640;
- stores only the opaque ID string in the ResourceSpec JSON;
- exposes no store path in spec, status, audit, metric label, or span attribute.

### 4.3 Generic vs per-Guest runtime config

| Setting | Location | Scope | Override |
| --- | --- | --- | --- |
| `controllerExecutionRef` | Provider root config | Zone-wide; required | No per-Guest override; all VMM Processes use this Host |
| `defaultVcpus`, `defaultMemoryMb`, `defaultMachineType`, `watchdog` | Provider root config | Zone-wide default for all Guests | Per-Guest `spec.provider.settings.vcpus`, `memoryMb`, `machineType`, `watchdogOverride` |
| Guest-control transport identity | Owned `Endpoint` resource | Guest-unique stable endpoint | Controller-created; no raw CID in Guest spec/status |
| `systemArtifactId` | Per-Guest top-level spec | Guest-unique system closure | No default; required |
| `consoleType`, `serialPort`, `pvpanic` | Per-Guest `spec.provider.settings` | Individual VMM tuning | No inheritance |
| `healthCheckInterval`, `healthCheckTimeout`, `healthCheckFailureThreshold`, `adoptionWindow`, `startupDeadline` | Provider root config | Zone-wide | Not per-Guest overridable in v3 initial catalog |

---

## 5 Owned bootstrap graph

The controller owns the VMM `Process` child resource. On every reconcile it
reads the current `Process` snapshot via `owner_index`, diffs the desired spec
against the observed state, creates the `Process` when all dependencies are
ready, repairs spec drift with expected-revision writes, and requests deletion
when the Guest is being finalized.

### 5.1 Bootstrap graph topology

```
Guest/<name>
  └── Process/<name>-vmm    (Provider/system-minijail)
```

The net-VM Guest (auto-declared by `Provider/network-local` for each Network)
has the same single-Process shape. `ProcessRole::NetVm` in the current baseline
maps to a `Provider/runtime-cloud-hypervisor` VMM process under a
controller-created `Guest/<network-name>-net-vm`.

#### Process: `<name>-vmm`

- **Template**: `cloud-hypervisor-runner` — declared in the Process spec as
  `spec.template`; the system-minijail supervisor resolves the executable from
  the template's artifact catalog entry. No `artifactId` field appears in the
  Process spec.
- **Domain**: `system`; `executionRef`: taken from Provider root config
  `controllerExecutionRef` field.
- **Sandbox**: minijail, `clone3(CLONE_PIDFD)`,
  `namespaceClasses: [pid, mount, ipc]`, `readOnlyRoot: true`,
  `noNewPrivileges: true`, `startRoot: false`, `capabilityClasses: []`
  (empty — zero capabilities beyond baseline), `seccompClass: vmm-default`
  (required; fixed by the signed template; cannot be overridden by Guest
  settings), `environmentClass: minimal`.
- **pidfd**: mandatory; d2b owns wait/reap for this template.
- **networkUsage**: single object `{networkRef: null, ports: [], allowEgress:
  false}`. All Network attachment controllers supply ready typed tap Fds in
  the LaunchTicket; the VMM itself has no ambient egress.
- **deviceUsage**: one entry per `deviceAttachments` entry plus a required
  `Device/kvm` entry (purpose `kvm-fd`, access `shared` — KVM is safely
  shareable across multiple VMs). TPM and other exclusive devices use access
  `exclusive` exactly as declared in the Guest `deviceAttachments`. The Device
  Provider owns each passthrough socket/fd; only `deviceRef`, `access`, and
  `purpose` appear in the spec.
- **mounts**: empty (virtiofs sockets are implementation details of
  `Provider/volume-virtiofs`; the VMM receives them through the supervisor
  ticket).
- **Endpoint resources**: the controller creates owned `Endpoint` resources for
  the CH API control socket and Guest-control vsock service. The VMM Process spec
  has no inline endpoint list.
- **cgroup**: placed directly in the delegated leaf:
  `z-<zone-id>/e-<guest-uid>/system/providers/p-<provider-id>/components/c-controller/process/`.
- **Restart policy**: `on-failure`, `backoffBase: "1s"`, `backoffMax: "60s"`,
  `maxRestarts: null` (unlimited).
- **Readiness**: `class: ready-condition` with `initialDelay: "0s"`,
  `timeout: "30s"`, `failureThreshold: 3`.
- **Desired lifecycle**: `running` while Guest's `spec.desiredState` is
  `running`; `stopped` when Guest is stopping.
- **Adoption**: `adopt-on-restart`. After controller restart, the controller
  attempts to reopen a pidfd for the running process within `adoptionWindow`.
  Adoption verifies pid/start-time/cgroup/executable/template/generation before
  `pidfd_open`; ambiguity sets the VMM Process to `Unknown`/`Degraded`, never
  causes a broad kill.

### 5.2 Pre-start dependency ordering

The controller enforces this ordering by watching resource statuses through
`ResourceClient` and gating VMM Process creation in the reconcile loop:

1. All `Device` resources in `Guest.spec.deviceAttachments` (including
   `Device/kvm`) must be `Ready` before the VMM Process is created.
2. All `Network` resources in `Guest.spec.networkAttachments` must be `Ready`
   (bridges and tap dispatch in place) before the VMM Process is created.
3. All virtiofs-exported Volumes referenced by the Guest must be `Ready` per
   `Provider/volume-virtiofs` before the VMM Process is created.

When all conditions hold in the same reconcile turn, the controller creates the
VMM Process immediately — no intermediate EphemeralProcess steps. These
dependency checks are declared as explicit `dependency` watch selectors in the
controller descriptor; the reconcile loop receives `dependency-ready` triggers
and re-evaluates in constant time.

---

## 6 Network: TAP and macvtap

### 6.1 TAP attachment

Each `NetworkAttachmentSpec` entry in `Guest.spec.networkAttachments` resolves
one TAP interface on the host. The TAP name and MAC are derived from the Network
resource's `attachments` table, which names the Guest's `executionRef`. The
controller does not compute or store TAP names in the Guest spec or status; it
reads the provider-neutral `Network.status.resource.attachments[*]` readiness
and opaque handoff record once the Network is Ready.

TAP creation is a privileged broker effect. The VMM Process supervisor ticket
mediates TAP fd passing to the CH binary using the appropriate net-handoff mode
(`TapFd` or `PersistentTap`) discovered at host-check time. No raw TAP name,
fd number, or broker socket path appears in the Process spec.

### 6.2 macvtap (external attachment)

The Network resource declares an optional `ExternalAttachmentSpec` with
`mode: macvtap`. When this is present, the network-local Provider creates the
macvtap interface and reports the provider-neutral readiness base in
`Network.status.resource.externalAttachment`. The `runtime-cloud-hypervisor`
controller receives the private interface handoff from the dependency resolver
and passes it to the VMM supervisor ticket.
No macvtap interface name, fd, or host interface path appears in the Guest
spec or VMM Process spec.

---

## 7 Volume attachments: virtiofs

Volume attachments for `runtime-cloud-hypervisor` Guests use the `virtiofs`
transport managed by `Provider/volume-virtiofs`. The chain:

1. **Volume resource** (`Provider/volume-local`): owns the host-side storage,
   layout, ACLs, and named views.
2. **virtiofs attachment**: declared in `Volume.spec.attachments[*]` with
   `transport: virtiofs`, `executionRef: Guest/<name>`, and a named view.
3. **virtiofsd Process** (`Provider/volume-virtiofs`): one long-lived
   `Process` per attachment, owned by the Volume resource (not by the Guest).
   The virtiofsd Process spec requires `startRoot: false`,
   `namespaceClasses: [user, pid, mount]`, and `userNamespace` mapping
   (per ADR 0021 / virtiofsd sandbox spec).
4. **`/nix/store` Volume**: the per-Guest read-only store Volume is identified
   by an opaque `sourceId` assigned by `Provider/volume-local`; the backing
   hardlink farm path is resolved by the Volume Provider from its policy record
   and never appears in any spec, status, or audit field. The Volume carries
   `VolumeKind: state` with a `readPolicy` that restricts writes from the Guest.
5. **Guest-control token share** (`d2b-gctl`): a separate read-only virtiofs
   share that carries the guest bootstrap credential. The `d2b-<vm>-gctlfs`
   principal (narrower than `d2b-<vm>-runner`) owns this virtiofsd Process.
   The runtime-cloud-hypervisor controller depends on this share being Ready
   before the VMM Process starts.

The virtiofsd export socket path is a generated private implementation detail
of `Provider/volume-virtiofs` and is never a field in any spec, status, or
audit record (per DRVOL-010).

---

## 8 Device dependencies

### 8.1 TPM (`Provider/device-tpm`)

A Guest that requires a TPM declares:

```yaml
spec:
  deviceAttachments:
    - deviceRef: Device/<name>-tpm
      exclusive: true
```

The `Provider/device-tpm` controller:
- creates the persistent swtpm state Volume (`VolumeKind: state`);
- starts the swtpm `Process` (owned by the Device resource, **not** by the
  Guest);
- sets the Device to `Ready` when swtpm is healthy.

The `runtime-cloud-hypervisor` controller waits for the Device to be `Ready`
(`DevicePresent=True`, `DeviceClaimed=True`) before creating the VMM Process. The VMM Process spec includes:

```yaml
deviceUsage:
  - deviceRef: Device/<name>-tpm
    access: exclusive
    purpose: tpm-socket
```

The `purpose: tpm-socket` field tells the supervisor to pass the swtpm socket
fd through the LaunchTicket. No swtpm socket path appears in spec or status.

### 8.2 GPU (`Provider/device-gpu`)

A Guest with `deviceAttachments` referencing a GPU Device receives:

```yaml
deviceUsage:
  - deviceRef: Device/<name>-gpu
    access: exclusive
    purpose: gpu-virtio         # or gpu-render-node for render-node-only mode
```

The GPU Device Provider owns the vhost-user GPU worker `Process`
(`ProcessRole::Gpu` → `Process` under `Provider/device-gpu`). The
`runtime-cloud-hypervisor` controller depends on the Device being Ready and
adds a `--gpu` or `--vhost-user-gpu` argument to the VMM supervisor ticket.

For the `video` sidecar (`ProcessRole::VhostUserVideo`): the device-gpu Provider
owns this `Process` resource when `spec.provider.settings.videoSidecar: true` is
declared on the Guest (via the GPU Device spec extension). The controller waits
for the Video Process to be Ready before the VMM starts.

### 8.3 Security key (`Provider/device-security-key`)

```yaml
deviceUsage:
  - deviceRef: Device/<name>-yubikey
    access: exclusive
    purpose: virtiofs-hidraw
```

The security-key Provider owns the host relay `Process` and guest frontend
`Process`. The `runtime-cloud-hypervisor` controller depends on the security-key
Device being Ready and adds the hidraw virtiofs share to the VMM ticket.

### 8.4 KVM (`Provider/device-kvm`)

KVM acceleration is **not** an implicit Host capability. Every Guest that
requires KVM must include an explicit `Device/kvm` resource in its
`deviceAttachments` closure. Because `/dev/kvm` is safely shareable across
multiple VMs simultaneously, the KVM Device does **not** require
`exclusive: true`; the Device contract marks it as `shared`:

```yaml
spec:
  deviceAttachments:
    - deviceRef: Device/<name>-kvm   # no exclusive; Device contract marks shared
```

The `Provider/device-kvm` controller owns the `/dev/kvm` entry and reports it
as Ready. The VMM Process spec includes:

```yaml
deviceUsage:
  - deviceRef: Device/<name>-kvm
    access: shared                   # KVM is safely shareable
    purpose: kvm-fd
```

The `purpose: kvm-fd` field tells the supervisor to open and pass the KVM fd
through the LaunchTicket. A Guest without a `Device/kvm` attachment runs the
VMM in TCG (software emulation) mode; this is intentional and not an error, but
it must be explicit in the spec, not inferred from Host capabilities.

---

## 9 Reconciliation

### 9.1 Async loop

The controller implements the standard reconciliation contract from
`ADR-046-resource-reconciliation`:

```text
async describe() -> ControllerDescriptor
async validateSpec(context, resource) -> ValidationResult
async plan(context, resource, dependencies) -> ReconcilePlan
async reconcile(context, resource, dependencies) -> ReconcileResult
async observe(context, resource) -> ObservationResult
async finalize(context, deletingResource) -> FinalizeResult
async health() -> ControllerHealth
async drain(deadline) -> DrainResult
```

Independent Guests run in parallel under the controller-wide semaphore.
Long-running effects (VMM start, adoption wait, guest-control handshake) use
async tasks; status writes are asynchronous expected-revision commits.

### 9.2 Fast path

After a Guest `spec` durable commit:

- post-commit dispatcher pushes a hint immediately;
- p95 controller handler start: ≤5 ms;
- controller reads dependency statuses synchronously;
- if all Device, Network, and Volume dependencies are already `Ready`: VMM
  Process creation commit p95 ≤20 ms from hint receipt (immediate async launch);
- if any dependency is not yet `Ready`: controller writes `Guest.status` phase
  `Pending`, returns, and will be re-triggered by `dependency-ready` events.
  When the final dependency becomes Ready the controller creates the VMM Process;
  p95 ≤20 ms from that trigger receipt.

### 9.3 Reconcile steps

1. Receive trigger (spec-generation-changed, owned-resource-changed,
   dependency-ready, dependency-changed, deletion-requested, retry-due, etc.).
2. Read fresh Guest spec snapshot plus owner-index VMM Process snapshot.
3. Call `validateSpec`: check spec.provider.settings bounds, Endpoint resource shape,
   systemArtifactId catalog type, memoryShared+virtiofs invariant,
   controllerExecutionRef validity.
4. Read dependency snapshots (Device/kvm and all declared Devices, all Networks,
   all virtiofs Volume statuses) through the capability-limited `ResourceClient`.
5. If any dependency is not Ready: write Guest status `Pending`/conditions;
   return `pending`. Controller will be re-triggered by `dependency-ready`.
6. Diff desired VMM Process spec against observed child. If absent, create it;
   if drifted, repair with expected-revision `update-spec`. Batch with
   expected-revision preconditions.
7. Stale conflict on any batch → discard result; toolkit re-reads and the
   handler retries under policy.
8. Write Guest status (`status.resource.bootstrapReady`, Guest readiness,
   conditions, and Cloud Hypervisor `status.provider.details`) atomically via
   `update-status` with expected revision.
9. Return `converged`, `pending`, `failed-retryable`, or `failed-terminal`.

### 9.4 Adoption after controller restart

The controller restart trigger is `startup-relist`:

1. The controller lists all `runtime-cloud-hypervisor` Guests in the Zone.
2. For each Guest in `Ready` phase, it attempts to adopt its VMM Process by
   verifying the existing pidfd (pid/start-time/cgroup/executable/template/
   generation) within `adoptionWindow`.
3. If adoption succeeds: the controller reconciles current state without
   disrupting the running VMM.
4. If adoption fails (process gone, ambiguous identity): the VMM Process
   transitions to `Unknown`; the Guest transitions to `Degraded`; the
   controller requests a restart through a new `desiredLifecycle: running`
   expected-revision write.
5. Ambiguous identity sets condition `AdoptionAmbiguous=True` and never issues
   a broad kill.

### 9.5 Observe interval

The controller declares `observeInterval: "30s"` (configurable via root config
`healthCheckInterval`). Core schedules a `scheduled-observe` trigger at this
interval. The controller calls the guest-control health endpoint and updates
the `GuestReachable` condition.

---

## 10 Readiness, restart, and pidfd

### 10.1 pidfd contract

The VMM Process uses `Provider/system-minijail` which acquires a pidfd via
`clone3(CLONE_PIDFD)` and owns wait/reap:

- pidfd is obtained at spawn time by the broker through `clone3`;
- pidfd is not persisted across daemon/controller restart;
- pidfd is not public status and never crosses d2b-bus;
- after controller restart, pidfd is reopened via `pidfd_open` after identity
  re-verification (adoption path above);
- the controller holds the pidfd locally; it is closed on clean restart
  re-verification or final process exit;
- the ProviderSupervisor returns the stable process identity and pidfd evidence
  to the Process controller, which retains it locally.

### 10.2 Process readiness

The VMM Process `readiness.class: ready-condition` means the Process controller
writes `Ready` when the VMM process is live (pidfd valid) and the first
readiness check passes (no process crash within `readiness.initialDelay`).

Guest `bootstrapReady` transitions to `true` only after:
- the VMM Process is `Ready`;
- the guest-control health check passes at least once
  (`GuestReachable=True`).

No EphemeralProcess resource is involved.

### 10.3 Restart policy for VMM

```yaml
restartPolicy:
  class: on-failure
  backoffBase: "1s"
  backoffMax: "60s"
  backoffMultiplier: 2.0
  maxRestarts: null
  resetAfter: "300s"
```

On unexpected VMM exit:
1. Process transitions to `Degraded` with `exitCode` in outcome.
2. Guest transitions to `Degraded` with condition `GuestReachable=False`,
   `BootstrapReady=False`, reason `vmm-process-exited`.
3. Controller applies backoff and requests VMM restart via
   `desiredLifecycle: running` expected-revision write.
4. On restart, the controller re-checks all Device/Network/Volume dependencies
   before allowing the new VMM Process to start.

### 10.4 Guest-control readiness vs SSH readiness

`ProcessRole::GuestSshReadiness` is retired at v3 cutover. Guest readiness is
established exclusively through the authenticated guest-control ComponentSession
(vsock, enrolled KK Noise) called directly by the controller's `observe`
handler. There is no SSH fallback path and no EphemeralProcess health-check
resource.

---

## 11 Config schema and signed component descriptor

### 11.1 Controller component descriptor

```yaml
id: controller
type: controller
binary: d2b-provider-runtime-ch-controller
resourceTypes:
  - Guest
supportedHostProviders:
  - system-core
supportedGuestProviders: []    # this Provider IS the guest; it runs on Hosts
processDomainsSupported:
  - system
specVerbs: [create, update-spec, delete]
statusVerbs: [update-status]
finalizerVerbs: [clear-finalizer]
watchSelectors:
  - resourceType: Guest
    providerRef: Provider/runtime-cloud-hypervisor
  - resourceType: Process
    ownerRefType: Guest
  - resourceType: Device
    phase: Ready
  - resourceType: Network
    phase: Ready
  - resourceType: Volume
    phase: Ready
dependencySelectors:
  - resourceType: Device
    reason: device-dependency-ready
  - resourceType: Network
    reason: network-dependency-ready
  - resourceType: Volume
    reason: volume-dependency-ready
ownerChildTriggers:
  - ownerType: Guest
    childTypes: [Process]
reconcileConcurrency: 8
maxPendingResources: 256
finalizersOwned:
  - runtime.runtime-cloud-hypervisor.d2bus.org/guest
observeIntervalSeconds: 30
resyncPolicy: dependency-change-only
deadlines:
  reconcile: "60s"
  finalize: "300s"
  observe: "10s"
retryClasses:
  - code: transient
    backoffBase: "1s"
    backoffMax: "60s"
  - code: dependency-not-ready
    backoffBase: "5s"
    backoffMax: "120s"
  - code: terminal
    maxAttempts: 1
serviceFingerprints: []
schemaFingerprints:
  - resourceType: Guest
    version: "1.0"
    digest: sha256:<pinned-at-build>
stateNamespaces: []                    # no Provider state Volume; operational state is in status/core ledger (D087)
```

The controller descriptor is signed into the Provider package; runtime
registration must match the installed Provider descriptor and the authenticated
Process/Host identity. Descriptor mismatch fails registration closed.

The controller declares an empty `stateNamespaces` list: it holds no durable
payload that passes the storage-need test, so the ProviderDeployment creates no
Provider state Volume for it. Bounded non-secret operational state lives in the
owning resource's `status` subresource and the core Operation ledger (§16.1,
D087); the controller mounts no `/state` Volume.

### 11.2 Signed Guest spec extension

The `spec.provider.settings` schema for `runtime-cloud-hypervisor` is exported
as a JSON Schema artifact signed with the Provider package:

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "runtime-cloud-hypervisor.d2bus.org/Guest/spec",
  "type": "object",
  "additionalProperties": false,
  "properties": {
    "vcpus":             { "type": "integer", "minimum": 1, "maximum": 1024 },
    "memoryMb":          { "type": "integer", "minimum": 128, "maximum": 524288 },
    "machineType":       { "type": "string", "enum": ["q35", "microvm"] },
    "consoleType":       { "type": "string", "enum": ["virtio", "serial"] },
    "serialPort":        { "type": "boolean" },
    "pvpanic":           { "type": "boolean" },
    "watchdogOverride":  { "type": ["boolean", "null"] },
    "memoryShared":      { "type": "boolean" }
  }
}
```

This schema is registered in the Provider `ResourceApiExport` and used for:
- build-time `spec.provider.settings` validation during Nix resource bundle
  compilation;
- runtime `validateSpec` in the controller;
- Nix option type generation via `xtask gen-resource-nix-options`.

---

## 12 Nix authoring

### 12.1 Artifact catalog

```nix
d2b.artifacts = {
  provider-runtime-cloud-hypervisor = {
    package = inputs.d2b-provider-runtime-cloud-hypervisor.packages.${system}.default;
    type    = "provider";
  };
  dev-vm-system = {
    package = inputs.nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [ ./guests/dev-vm.nix ];
    }.config.system.build.toplevel;
    type = "nixos-system";
  };
};
```

Derivation-valued inputs (`package =`) are **only** permitted inside
`d2b.artifacts.<id>`. They must not appear inside any `spec.*` field. The build
derives content digests, validates type/ID constraints, and emits a private
integrity-pinned catalog at `root:d2bd` 0640.

### 12.2 Provider installation resource

```nix
d2b.zones.dev.resources.runtime-cloud-hypervisor = {
  type = "Provider";
  spec = {
    artifactId = "provider-runtime-cloud-hypervisor";
    config = {
      controllerExecutionRef = "Host/dev-host";  # required; explicit Host ref
      defaultVcpus    = 2;
      defaultMemoryMb = 512;
      watchdog        = true;
    };
  };
};
```

### 12.3 Guest resource

```nix
d2b.zones.dev.resources.dev-vm = {
  type = "Guest";
  spec = {
    providerRef    = "Provider/runtime-cloud-hypervisor";
    systemArtifactId = "dev-vm-system";   # top-level spec field; NOT inside spec.provider.settings
    budget = {
      cpu    = { request = "500m"; limit = "4000m"; };
      memory = { request = "512Mi"; limit = "4096Mi"; };
    };
    networkAttachments = [
      { networkRef = "Network/dev-net"; default = true; }
    ];
    deviceAttachments = [
      { deviceRef = "Device/dev-vm-kvm"; }                      # shared; no exclusive needed
      { deviceRef = "Device/dev-vm-tpm"; exclusive = true; }
    ];
    provider = {
      schemaId = "runtime-cloud-hypervisor.d2bus.org/Guest/spec";
      schemaVersion = "1.0.0";
      settings = {
        vcpus       = 4;
        memoryMb    = 4096;
        machineType = "q35";
        consoleType = "virtio";
      };
    };
  };
};
```

### 12.4 Rendered ResourceSpec JSON

The build compiles the above to (sorted canonical form; `status` absent from
bundle; `metadata.managedBy` set at activation time, not in bundle):

```json
{
  "apiVersion": "resources.d2bus.org/v3",
  "type": "Guest",
  "metadata": {
    "name": "dev-vm",
    "zone": "dev"
  },
  "spec": {
    "allowedDomains": ["system"],
    "budget": {
      "cpu": { "limit": "4000m", "request": "500m" },
      "memory": { "limit": "4096Mi", "request": "512Mi" }
    },
    "defaultDomain": "system",
    "defaultUserRef": null,
    "deviceAttachments": [
      { "deviceRef": "Device/dev-vm-kvm" },
      { "deviceRef": "Device/dev-vm-tpm", "exclusive": true }
    ],
    "networkAttachments": [
      { "default": true, "networkRef": "Network/dev-net" }
    ],
    "providerRef": "Provider/runtime-cloud-hypervisor",
    "provider": {
      "schemaId": "runtime-cloud-hypervisor.d2bus.org/Guest/spec",
      "schemaVersion": "1.0.0",
      "settings": {
        "consoleType": "virtio",
        "machineType": "q35",
        "memoryMb": 4096,
        "vcpus": 4
      }
    },
    "systemArtifactId": "dev-vm-system",
    "volumeAttachmentDefaults": []
  }
}
```

Note:
- `systemArtifactId` is a **top-level `spec` field**, not inside
  `spec.provider.settings`. The rendered JSON confirms this placement.
- No store path, kernel path, initrd path, or any derivation value appears
  anywhere in the JSON envelope.
- `spec.provider.settings` contains only the closed bounded fields — `vcpus`,
  `memoryMb`, `machineType`, `consoleType` — from the signed Guest
  schema extension. No `cmdlineExtra`, no `seccompOverride`, no free-form fields.
- `Device/dev-vm-kvm` appears in `deviceAttachments` without `exclusive`; KVM
  is a shared device.

### 12.5 Eval-time validation rules

Rules 1–17 from `ADR-046-nix-configuration` apply to every Guest resource. The
following are additional rules enforced specifically for `runtime-cloud-hypervisor`
Guests:

| Rule | Code | Check |
| --- | --- | --- |
| 17 | `system-artifact-required` | `spec.systemArtifactId` must be set and non-null for this Provider |
| 17 | `system-artifact-type-mismatch` | The artifact catalog entry must have `type = "nixos-system"` |
| CH-1 | `guest-control-endpoint-required` | Each running Guest must have an owned `Endpoint` with `producerRef: Guest/<name>`, `endpointClass: control`, and `transport: vsock` before guest-control health can pass |
| CH-2 | `raw-endpoint-locator-denied` | Raw CIDs, ports, socket paths, and fd numbers are rejected from Guest spec/status and CLI output |
| CH-3 | `memory-virtiofs-conflict` | `memoryShared: false` is rejected when any Volume attachment under this Guest uses `transport: virtiofs` |
| CH-4 | `provider-settings-unknown-field` | Any `spec.provider.settings` field not in the signed Guest schema extension is rejected (rejects `cmdlineExtra`, `seccompOverride`, raw endpoint locators, and any other unlisted field) |
| CH-5 | `controller-execution-ref-invalid` | `Provider.spec.config.controllerExecutionRef` must reference an existing `Host` resource in the Zone; missing or wrong type fails Provider installation |

---

## 13 Process templates

This Provider has **two** Process resources:

| Process | Created by | Template | Provider | `processClass` | Purpose |
| --- | --- | --- | --- | --- | --- |
| `runtime-ch-controller` | ProviderDeployment (static) | `runtime-ch-controller` | system-minijail | `controller` | Runs the controller binary; one per Provider installation |
| `<name>-vmm` | controller (dynamic, one per Guest) | `cloud-hypervisor-runner` | system-minijail | `worker` | Long-lived VMM process |

Workers do not receive d2b-bus authority and do not create EphemeralProcess
resources. Guest-control health is a typed `ComponentSession` call executed
directly by the controller's `observe` handler, not by any worker process.

### 13.1 `runtime-ch-controller` canonical ResourceSpec (static, ProviderDeployment-created)

The **ProviderDeployment** creates this Process before invoking the controller
binary. The spec is synthesized from the signed component descriptor and the
Provider root config `controllerExecutionRef`; the controller binary cannot
observe or modify it. It is written once at Provider installation and updated
only when the Provider is upgraded or its `controllerExecutionRef` changes.

```yaml
apiVersion: resources.d2bus.org/v3
type: Process
metadata:
  name: runtime-ch-controller
  zone: dev
  ownerRef: Provider/runtime-cloud-hypervisor
spec:
  providerRef: Provider/system-minijail
  executionRef: Host/dev-host        # from Provider.spec.config.controllerExecutionRef; required
  template: runtime-ch-controller    # system-minijail resolves executable from artifact catalog
  processClass: controller           # grants d2b-bus ComponentSession; receives Zone watch authority
  domain: system
  budget:
    cpu:    { request: "100m",  limit: "500m"  }
    memory: { request: "64Mi",  limit: "256Mi" }
  sandbox:
    namespaceClasses: [pid, mount, ipc]
    readOnlyRoot: true
    noNewPrivileges: true
    startRoot: false
    capabilityClasses: []            # zero capability classes beyond baseline
    seccompClass: controller-default # fixed by signed template; not caller-settable
    environmentClass: minimal
  networkUsage:                      # single object; controllers have no network attachment
    networkRef: null
    ports: []
    allowEgress: false
  deviceUsage: []                    # no device access; d2b-bus provides all resource authority
  mounts: []                         # no Provider state Volume; operational state is in status/core ledger (D087)
  readiness:
    class: ready-condition
    initialDelay: "0s"
    successThreshold: 1
    timeout: "10s"
    failureThreshold: 3
  restartPolicy:
    class: on-failure
    backoffBase: "2s"
    backoffMax: "120s"
    backoffMultiplier: 2.0
    maxRestarts: null                # unlimited; ProviderDeployment manages Provider phase
    resetAfter: "600s"
  desiredLifecycle: running
```

The controller Process is managed entirely by the ProviderDeployment. The
ProviderDeployment sets `desiredLifecycle: stopped` during Provider teardown
and awaits the Process reaching `Stopped` before clearing the Provider
finalizer. The controller binary does **not** write to this Process resource
and cannot alter its own `executionRef`, `sandbox`, or `restartPolicy`.

The `processClass: controller` grants the binary a `ComponentSession` with
Zone-watch authority bounded to the selectors declared in the component
descriptor (§11.1). No ambient egress, no ambient device access, and no
host-path mounts are granted to the controller process.

### 13.2 `cloud-hypervisor-runner` canonical ResourceSpec

The controller builds the VMM `Process` spec by composing the signed template
with resource-derived fields at reconcile time. The following is the canonical
ResourceSpec written to the Zone store (no argv, paths, or socket names):

```yaml
apiVersion: resources.d2bus.org/v3
type: Process
metadata:
  name: dev-vm-vmm
  zone: dev
  ownerRef: Guest/dev-vm
spec:
  providerRef: Provider/system-minijail
  executionRef: Host/dev-host          # from Provider root config controllerExecutionRef
  template: cloud-hypervisor-runner    # system-minijail resolves executable from this template
  processClass: worker
  domain: system
  budget:
    cpu:    { request: "500m",   limit: "4000m" }
    memory: { request: "512Mi",  limit: "4096Mi" }
  sandbox:
    namespaceClasses: [pid, mount, ipc]
    readOnlyRoot: true
    noNewPrivileges: true
    startRoot: false
    capabilityClasses: []              # empty — zero capability classes beyond baseline
    seccompClass: vmm-default          # required; fixed by signed template; not caller-settable
    environmentClass: minimal
  networkUsage:                        # single object; not a list
    networkRef: null                   # tap Fds supplied by LaunchTicket from Network controllers
    ports: []
    allowEgress: false                 # VMM has no ambient egress
  deviceUsage:
    - deviceRef: Device/dev-vm-kvm
      access: shared                   # KVM is safely shareable; Device contract marks shared
      purpose: kvm-fd
    - deviceRef: Device/dev-vm-tpm     # present only when Guest declares this Device
      access: exclusive                # TPM must be exclusive per Device contract
      purpose: tpm-socket
  mounts: []                           # virtiofs sockets supplied through LaunchTicket, not mounts
  readiness:
    class: ready-condition
    initialDelay: "0s"
    successThreshold: 1
    timeout: "30s"
    failureThreshold: 3
  restartPolicy:
    class: on-failure
    backoffBase: "1s"
    backoffMax: "60s"
    backoffMultiplier: 2.0
    maxRestarts: null
    resetAfter: "300s"
  desiredLifecycle: running
```

The controller does **not** store computed argv, kernel paths, initrd paths,
TAP fd numbers, guest-control transport locators, or socket paths in the Process
spec or status. These
are implementation details resolved at `ProviderSupervisor` dispatch time from
the signed template's artifact catalog entry and the current resource/dependency
state. `Process.spec.artifactId` is not a field in the Process ResourceType;
the executable is entirely owned by the template.

The load-bearing stable endpoints are modeled as resources rather than inline
Process fields:

```yaml
apiVersion: resources.d2bus.org/v3
type: Endpoint
metadata:
  name: dev-vm-ch-api
  zone: dev
  ownerRef: Guest/dev-vm
spec:
  providerRef: Provider/runtime-cloud-hypervisor
  producerRef: Process/dev-vm-vmm
  endpointClass: control
  transport: unix
  purpose: ch-api-socket
  serviceFingerprint: runtime-cloud-hypervisor.d2bus.org/ch-api/v1
  locality: host-local
  visibility: provider-internal
  attachmentPolicy: launch-ticket-only
  consumerPolicy: [Provider/runtime-cloud-hypervisor]
  lifecyclePolicy: recycle-with-producer
---
apiVersion: resources.d2bus.org/v3
type: Endpoint
metadata:
  name: dev-vm-guest-control
  zone: dev
  ownerRef: Guest/dev-vm
spec:
  providerRef: Provider/runtime-cloud-hypervisor
  producerRef: Guest/dev-vm
  endpointClass: control
  transport: vsock
  purpose: guest-control
  serviceFingerprint: d2b.guest-control.d2bus.org/kk/v1
  locality: cross-domain
  visibility: provider-internal
  attachmentPolicy: launch-ticket-only
  consumerPolicy: [Provider/runtime-cloud-hypervisor]
  lifecyclePolicy: recycle-with-producer
```

Consumers refer to `Endpoint/dev-vm-ch-api` and
`Endpoint/dev-vm-guest-control`; Core/ProviderSupervisor resolves the private
Unix or vsock locator only through EffectPort/LaunchTicket authorization.

### Endpoint resources (D092)

`Provider/runtime-cloud-hypervisor` declares conformance to the standard
`Endpoint` base schema. Stable CH API, Guest-control, and vhost-user data
surfaces are owned `Endpoint` resources with `ownerRef`, `producerRef`, closed
`endpointClass`, closed `transport`, and no raw path, CID, port, fd, or
credential in spec, status, audit, metrics, or CLI output. Consumers use
`Endpoint/<name>` ResourceRefs; Core/ProviderSupervisor resolves private
transports or fds only through authorized EffectPort/LaunchTicket flows, and an
unauthorized request fails `endpoint-resolve-denied`. A producer restart bumps
`endpointGeneration`, causing dependent consumers to receive the standard
`dependency-changed` trigger.

### Retained opaque handles

Permitted opaque values are limited to controller-internal or high-churn data:
`pidfd`/process generation observations, LaunchTicket fd indexes, per-session
ComponentSession/OwnedTransport handles, operation IDs, and QMP/CH connection
handles. They have no independent resource lifecycle, are not stable managed
endpoint identities, and remain absent from public spec/status/CLI fields except
as bounded non-authorizing diagnostics where already allowed.

---

## 14 d2b-bus methods and streams

The controller communicates exclusively through `d2b-bus →
ComponentSession → d2b.resource.v3`. No direct broker socket, no direct
store handle, no ambient route table.

### 14.1 ResourceClient verbs used

| Verb | ResourceType | Purpose |
| --- | --- | --- |
| `list` | Guest | Initial relist on startup |
| `watch` | Guest, Process, Device, Network, Volume | Continuous watch stream |
| `get` | All of the above | Fresh snapshot per reconcile |
| `create` | Process | VMM Process creation when all deps ready |
| `update-spec` | Process | Drift repair |
| `update-status` | Guest | Status write |
| `delete` | Process | Finalizer-safe teardown |
| `clear-finalizer` | Guest | After finalize completes |

### 14.2 Named streams

Controller watch subscriptions are named streams on the `d2b.resource.v3`
service. Credit-based flow control; one blocked watch stream cannot starve
other named streams or control/cancel traffic.

### 14.3 Guest bootstrap ComponentSession

After the VMM Process is Ready, the controller opens an enrolled KK
ComponentSession to `Endpoint/<guest>-guest-control`. The private vsock locator
is resolved only through the authorized EffectPort/LaunchTicket path. The session uses
`Noise_KK_25519_ChaChaPoly_SHA256` with the guest bootstrap credential
(delivered through the `d2b-gctl` virtiofs share). The controller uses this
session for:
- authenticated health checks (`GuestReachable` condition);
- bootstrap readiness verification;
- any future guest-control operations.

The session carries an authorization lease revision. When the Zone's
Role/RoleBinding policy changes, the lease invalidates and the controller
re-establishes the session before the next health check.

---

## 15 RBAC and broker operations

### 15.1 Role requirements

The controller process must hold a Role binding granting:

```yaml
rules:
  - resourceTypes: [Guest]
    verbs: [get, list, watch, update-status, clear-finalizer]
  - resourceTypes: [Process]
    verbs: [get, list, watch, create, update-spec, delete]
  - resourceTypes: [Device, Network, Volume]
    verbs: [get, list, watch]
```

The `runtime-cloud-hypervisor` controller does **not** require `update-spec` on
Guest (spec is caller-controlled), `delete` on Guest (deletion is finalizer-
mediated), or any verb on Credential, Zone, Provider, Host, or EphemeralProcess
resources. No EphemeralProcess resources are created by this controller.

### 15.2 Broker operations

The controller does not call broker operations directly. Privileged effects are
mediated through the `Provider/system-minijail` supervisor ticket. The
supervisor ticket covers:

- `clone3(CLONE_PIDFD)` spawn with compiled minijail/sandbox arguments;
- TAP fd allocation and passing (via existing `TapBridgeAllocate` / net-handoff
  broker chain, translated to supervisor ticket parameters);
- cgroup placement in the delegated Zone subtree;
- pidfd return.

The `SwtpmDir` broker op (current: `d2b-priv-broker/src/ops/swtpm_dir.rs`) is
owned by `Provider/device-tpm` in v3, not by this Provider.
The `store_view_farm` broker op (current: `d2b-priv-broker`) is owned by
`Provider/volume-local` in v3.
The `SpawnRunner{role: CloudHypervisor}` broker op (current:
`d2b-priv-broker/src/ops/spawn_runner.rs`) becomes the
`cloud-hypervisor-runner` template dispatch in the system-minijail supervisor.

### 15.3 Guest finalizer

Finalizer ID: `runtime.runtime-cloud-hypervisor.d2bus.org/guest`

Algorithm on `deletion-requested`:

1. Set Guest status atomically: `status.provider.details.providerPhase: draining`,
   condition `GuestDraining=True`.
2. Set `desiredLifecycle: stopped` on the VMM Process via expected-revision
   `update-spec`.
3. Wait for the owned VMM Process to be deleted (owner-child cascade with its
   own finalizer).
4. Verify VMM process exit through the local pidfd.
5. Clear the `runtime.runtime-cloud-hypervisor.d2bus.org/guest` finalizer.
6. Return `finalized`.

If any child finalizer is blocked beyond `finalize` deadline (300 s), the
finalizer returns `blocked` with condition `FinalizationBlocked` and a bounded
reason message. The operator must resolve the block; the controller retries at
`retryClasses.transient` interval.

---

## 16 State

### 16.1 Provider state (controller process)

**ProviderStateSet is an optional, query-time concept, not a ResourceType.**
The `ProviderStateSet` for `Provider/runtime-cloud-hypervisor` is the set of the
*declared* Volume resources in the Zone whose `metadata.ownerRef` resolves to
`Provider/runtime-cloud-hypervisor`, and is empty for this Provider:

```
ProviderStateSet(zone, "runtime-cloud-hypervisor") =
  { v : Volume | v.metadata.zone == zone
              && v.metadata.ownerRef == "Provider/runtime-cloud-hypervisor" }
```

`Provider/runtime-cloud-hypervisor`'s controller declares **no** Provider state
Volume; its `ProviderStateSet` is empty. All application-level recovery data —
resource generations, watch cursors, adoption tokens — is derivable from the
Zone resource store, the core Operation ledger, and independent external
observation (running VMM/virtiofsd processes re-adopted from declared cgroup
leaves and fresh pidfds) at restart time. Its bounded non-secret operational
state — reconcile stage, per-Guest launch/adoption observations, bounded
counters, and closed-enum error detail — lives in the owning resource's
`status` subresource and the core Operation ledger (D087). Because that state is
fully derivable and duplicating it would create a split-brain risk on restart,
the controller payload fails the storage-need test: there is no controller
state namespace, no controller state Volume, no `/state` mount, and no dedicated
`User/d2b-runtime-ch-controller` state-layout principal. There is no empty
identity-only Volume.

No Guest spec, status, argv, kernel path, or ephemeral runtime state is stored
in any Provider state Volume.

### 16.2 Guest runtime state (retained)

The Guest itself has no Provider-owned Volume. All durable per-Guest state
(swtpm NVRAM, persistent storage, the `/nix/store` farm) belongs to the
Volume resources declared in the Zone configuration, owned by their respective
Providers (`Provider/device-tpm`, `Provider/volume-local`) — these are genuine
durable payloads (large/secret/private) that pass the storage-need test and are
retained unchanged. The controller reads their status but does not write to
them.

### 16.3 State migration

Because the controller declares no state Volume, no migration EphemeralProcess
is created for this Provider at version `1.0`. If a future schema version ever
introduces a durable controller payload that passes the storage-need test
(secret/large/private/revision-unsuitable data that cannot live in `status`),
the component descriptor would declare a `stateNamespaces` entry with
`migrationPolicy: pre-launch-required` (breaking) or `online-optional`
(additive), and the ProviderDeployment would orchestrate migration via a signed
EphemeralProcess template before relaunching the controller Process. That is a
future concern; version `1.0` requires no state Volume and no migration
infrastructure.

---

## 17 Security

### 17.1 Isolation posture

Every `runtime-cloud-hypervisor` Guest has `IsolationPosture: VirtualMachine`.
This is a hardware-virtualization isolation boundary (KVM, IOMMU, virtio).
The isolation posture is reported in `Host.status.isolationPosture` (the Host
that runs the VMM) and in the authoritative audit record for Guest creation.
It must **not** appear in OTEL metrics, span attributes, or log fields.

### 17.2 No secret in spec or status

The following are forbidden in any Guest spec, spec.provider.settings, or status field:
- store paths, kernel paths, initrd paths;
- socket paths (CH API socket, virtiofsd socket, swtpm socket);
- TAP interface names or fd numbers;
- guest-control transport locators in spec or status diagnostic fields;
- raw broker operation names;
- credential bytes or token material;
- argv fragments or environment variable values.

Bounded `status.provider.details.guestIdentityDigest` and `providerPhase` are the
only provider-observable runtime identity fields in status.

### 17.3 Audit records

The controller emits authoritative audit records (not OTEL) for:

| Event | Durability | Fields |
| --- | --- | --- |
| `GuestProvisionStarted` | durable | `zone`, `resource: Guest/<name>`, `generation`, `correlation_id` |
| `GuestProvisionSucceeded` | durable | `zone`, `resource`, `generation`, `guestIdentityDigest` |
| `GuestProvisionFailed` | durable | `zone`, `resource`, `generation`, `reason_code` (stable enum) |
| `GuestDeletionStarted` | durable | `zone`, `resource`, `generation`, `correlation_id` |
| `GuestDeletionSucceeded` | durable | `zone`, `resource`, `generation` |
| `VmmProcessExited` | durable | `zone`, `resource`, `exitCode` (bounded int) |
| `AdoptionAttempted` | durable | `zone`, `resource`, `outcome: adopted|failed|ambiguous` |

No argv, paths, socket names, kernel cmdline, OEM strings, PID, pidfd, TAP
name, guest-control locator, or credential material appears in any audit field. Bounded
`reason_code` values use stable lower-kebab-case machine identifiers.

### 17.4 virtiofsd sandbox

virtiofsd Process resources created by `Provider/volume-virtiofs` for Guests
managed by this Provider must declare:
- `capabilityClasses: []` (empty — zero capability classes beyond baseline);
- `seccompClass: virtiofsd-default` (required);
- `startRoot: false`;
- `userNamespace` block mapping in-NS UID/GID 0 to the per-share principal
  (`d2b-<vm>-runner` or `d2b-<vm>-gctlfs`);
- `--sandbox=chroot --inode-file-handles=never` (added by the virtiofs Provider
  supervisor; not in spec).

This invariant is validated by `tests/minijail-validator-virtiofsd.sh` and
`tests/virtiofsd-argv-shape.sh` (current tests; adapted per migration map).

---

## 18 Status, errors, OTEL, and audit

### 18.1 Guest status

D088 status layering is normative: the controller populates the Guest
ResourceType-common `status.resource` with runtime readiness, capabilities,
observed lifecycle phase, bootstrap readiness, and active process count in the
same shape as sibling Guest runtime providers. Cloud Hypervisor-specific VMM
lifecycle/adoption observations, including `providerPhase` and the bounded
non-authorizing guest identity digest, live only in `status.provider.details`
with `providerRef: Provider/runtime-cloud-hypervisor`, qualified `schemaId`
(`runtime-cloud-hypervisor.d2bus.org/Guest/status`), `schemaVersion`, and
`observedProviderGeneration`. Controller status writes include all present
layers atomically in one status mutation; shared fields are never duplicated
into `status.provider`, and the strict, ≤32 KiB, redacted extension schema is
registered and signed in the Provider manifest.

#### Currency and expedited reconcile (D091/D090)

D091 currency is universal status, not Cloud Hypervisor provider detail. The
controller implements `assess_update`, `plan_upgrade`, and `execute_upgrade`,
populates universal `status.update`, and keeps shared currency fields out of
`status.provider`; backend-specific observations may appear only under
`status.provider.details`. A new `systemArtifactId`/NixOS system-image
generation, provider package generation, or disruptive runtime spec change MUST
set `status.update.state = UpgradeRequired`, with `reasons =
[ImageOrSystemGenerationChanged]`, `[ProviderGenerationChanged]`, or
`[SpecChanged]`, `disruption = Recycle|Restart`, and `preserveState = true`
rather than applying in place. Non-disruptive spec changes reconcile normally.
`execute_upgrade` recycles the VMM/runner `Process` and endpoints while
preserving the Guest UID/spec identity, durable/data Volumes, and TPM identity
supplied by `Provider/device-tpm`; the dependency-aware planner restarts the
Guest after the recycle.

D090 expedited `waitForReconcile` on `Create`/`UpdateSpec`/`Delete` performs no
external effect, finalizer change, or status mutation until Core supplies
`CommittedRevisionProof {resourceUid,generation,revision,operationId}`. The
one-pass response returns the committed object, projected layered status,
disposition `Converged|Progressing|Blocked|UpgradeRequired|Failed`, and
`statusPersistence = pending|committed`; the durable commit is never rolled back
after a reconcile timeout. Effect idempotency keys derive from
`(UID,generation,revision,operationId)`, and the expedited pass uses the bounded
priority lane inside the same per-resource single-flight.

```yaml
status:
  observedGeneration: 1
  phase: Ready
  conditions:
    - type: GuestProvisioned
      status: "True"
      reason: vmm-process-ready
      observedGeneration: 1
      lastTransitionAt: 2026-07-22T00:00:01Z
    - type: BootstrapReady
      status: "True"
      reason: all-bootstrap-resources-ready
      observedGeneration: 1
      lastTransitionAt: 2026-07-22T00:00:02Z
    - type: GuestReachable
      status: "True"
      reason: guest-control-health-passed
      observedGeneration: 1
      lastTransitionAt: 2026-07-22T00:00:03Z
    - type: CapabilitiesVerified
      status: "True"
      reason: all-attachments-ready
      observedGeneration: 1
      lastTransitionAt: 2026-07-22T00:00:01Z
  lastReconciledAt: 2026-07-22T00:00:03Z
  resource:
    observedLifecyclePhase: running
    runtimeReady: true
    bootstrapReady: true
    capabilitiesVerified: true
    activeProcessCount: 1
  provider:
    providerRef: Provider/runtime-cloud-hypervisor
    schemaId: runtime-cloud-hypervisor.d2bus.org/Guest/status
    schemaVersion: 1.0.0
    observedProviderGeneration: 1
    details:
      providerPhase: running
      guestIdentityDigest: sha256:<bounded-hex>
      vmmProcess: ready
```

### 18.2 Stable error codes

| Code | Phase | Condition | Meaning |
| --- | --- | --- | --- |
| `system-artifact-required` | Failed | — | `systemArtifactId` is null for a CH Guest |
| `system-artifact-type-mismatch` | Failed | — | Referenced artifact is not `nixos-system` |
| `guest-control-endpoint-conflict` | Failed | PolicyValid=False | Guest-control Endpoint identity conflicts with an existing resource in this Zone |
| `vmm-process-exited` | Degraded | BootstrapReady=False, GuestReachable=False | Unexpected VMM exit |
| `guest-control-health-failed` | Degraded | GuestReachable=False | Authenticated health check failed |
| `guest-control-health-timeout` | Degraded | GuestReachable=False | Health check timed out |
| `dependency-device-not-ready` | Pending | CapabilitiesVerified=False | A required Device (including Device/kvm) is not Ready |
| `dependency-network-not-ready` | Pending | CapabilitiesVerified=False | A required Network is not Ready |
| `dependency-volume-not-ready` | Pending | BootstrapReady=False | A required virtiofs Volume share is not Ready |
| `adoption-ambiguous` | Degraded | AdoptionAmbiguous=True | VMM process identity ambiguous after restart |
| `quota-exceeded` | Failed | BudgetAdmitted=False | Budget overcommit at spec admission |
| `finalization-blocked` | Degraded | FinalizationBlocked=True | VMM Process finalizer blocked beyond deadline |

All `message` fields in conditions and outcomes are bounded (≤1024 chars),
UTF-8 validated, and must not contain paths, socket names, argv, environment
values, credential material, or PIDs.

### 18.3 OTEL metrics

The controller emits the following metrics through the Zone lightweight bounded
emitter (no OTEL SDK; frames forwarded by `Provider/observability-otel`):

| Metric name | Type | Labels | Description |
| --- | --- | --- | --- |
| `d2b_runtime_ch_guest_total` | Counter | `zone`, `outcome: provisioned|failed|deleted` | Guest lifecycle events |
| `d2b_runtime_ch_guest_phase` | Gauge | `zone`, `phase` | Current Guests by phase |
| `d2b_runtime_ch_vmm_restarts_total` | Counter | `zone` | VMM Process restart count |
| `d2b_runtime_ch_reconcile_duration_seconds` | Histogram | `zone`, `result: converged|pending|failed` | Per-reconcile duration |
| `d2b_runtime_ch_adoption_total` | Counter | `zone`, `outcome: adopted|failed|ambiguous` | Controller restart adoption events |
| `d2b_runtime_ch_health_check_duration_seconds` | Histogram | `zone`, `result: passed|failed|timeout` | Guest health check duration |

Cardinality rules:
- `zone` is allowed in metric labels (bounded by Zone count per host).
- Guest name (`vm.name`), guest-specific IDs, and endpoint locators are **not** metric
  label values; they may appear only in OTEL resource attributes (advisory).
- No path, socket name, CID, PID, or runtime detail appears in any metric label.

### 18.4 OTEL resource attributes

Processes in this Provider stamp:

```
d2b.zone = <zone-name>
d2b.provider = runtime-cloud-hypervisor
d2b.component = controller
service.name = d2b-provider-runtime-ch-controller
service.version = <CARGO_PKG_VERSION>
vm.name = <guest-name>   # advisory; re-stamped at ingress boundary
vm.env = <zone-name>     # advisory; preserved from current baseline
vm.role = vmm-controller # advisory
```

The closed allowlist from `policy_observability.rs::loki_native_otel_resource_attributes`
(current test) is extended to include `d2b.zone`, `d2b.provider`,
`d2b.component` per the telemetry spec.

---

## 19 Nix artifact catalog

The following named artifacts are declared in `d2b.artifacts` by consumers of
this Provider:

| Artifact ID pattern | Catalog `type` | Content | Validated by |
| --- | --- | --- | --- |
| `provider-runtime-cloud-hypervisor` | `provider` | Provider crate binary + descriptor + schema | rule 17 (`Provider.spec.artifactId`) |
| `<name>-system` | `nixos-system` | Guest NixOS system closure (kernel + initrd + rootfs) | rule 17 (`Guest.spec.systemArtifactId`) |

Rules:

1. Every `runtime-cloud-hypervisor` Guest must have `spec.systemArtifactId` set
   to a catalog ID of type `nixos-system`.
2. The catalog entry's `package` attribute must be a NixOS system build
   derivation (e.g., `nixosSystem { ... }.config.system.build.toplevel`).
3. No store path, derivation output path, or Nix expression appears in the
   ResourceSpec JSON or artifact catalog's public surface.
4. The build resolves `systemArtifactId` at derivation time. A missing key or
   type mismatch fails the NixOS build with a stable rule-17 error code.

---

## 20 Async fast-reconcile path

The fast path from `ADR-046-resource-reconciliation §Process fast path` applies
directly to the single VMM Process:

1. Guest spec commit → post-commit dispatcher pushes hint immediately.
2. Controller receives hint: p95 ≤5 ms.
3. Controller reads all dependency statuses (Device/kvm, all declared Devices,
   Networks, virtiofs Volumes) through `ResourceClient`.
4. **If all dependencies are Ready**: creates `<name>-vmm` Process; p95 ≤20 ms
   from hint receipt. This is the immediate async launch path.
5. **If any dependency is not Ready**: controller writes `Guest.status` phase
   `Pending`, returns `pending`. Watch loop continues; on each
   `dependency-ready` trigger the controller re-evaluates and creates the
   Process when the final dependency becomes Ready (p95 ≤20 ms from trigger).
6. Watch loop dispatches independent Guests concurrently under
   `reconcileConcurrency: 8` budget.
7. VMM Process readiness and guest-control health check (observe handler)
   complete asynchronously; Guest status is written with expected-revision
   commits.

No EphemeralProcess steps intervene. There is no artificial serialisation
between dependency readiness and VMM Process creation.

---

## 21 Lifecycle and upgrades

### 21.1 Controller version upgrade

1. New Provider resource generation (new `artifactId` in Nix → new
   `configurationGeneration`).
2. Controller drains existing reconcile queue (`drain` handler, deadline 60 s).
3. New controller binary starts; adoption re-verifies all running VMM pidfds.
4. Guests remain running across controller upgrade (KillMode=process equivalent
   in cgroup/pidfd model).
5. Controller descriptor changes that add new ResourceType verbs require a new
   `registrationGeneration`.

### 21.2 Guest migration (d2b 3.0 reset)

Migration from v2 (`d2b.vms.<vm>`) to v3 (`d2b.zones.<zone>.resources.<name>`):

1. v2 runtime is stopped.
2. Operator configures v3 Guest resource with matching `systemArtifactId`,
   Guest-control Endpoint resources, and `spec.provider.settings`.
3. Persistent Volume resources for swtpm state and workload storage are
   retained; v3 resource cleanup contract prevents their deletion.
4. `d2b 3.0 reset` activates v3; the controller creates the bootstrap graph
   fresh.

There is no in-place upgrade path. d2b 3.0 is a clean reset.

### 21.3 Net VM lifecycle

The `Provider/network-local` controller creates `Guest/<network-name>-net-vm`
resources automatically for each Network. These Guests use
`providerRef: Provider/runtime-cloud-hypervisor` and carry:
- `spec.systemArtifactId`: set from `Network.spec.netVmSystemArtifactId`;
- Guest-control `Endpoint` resource: created by the Network controller without
  exposing the private transport locator;
- `spec.deviceAttachments`: includes the net-VM's `Device/kvm` and any declared
  network device refs; no implicit Host capabilities;
- standard single VMM Process bootstrap.

Net VM Guests are `managedBy: controller` resources; they are not in the Nix
bundle and are never swept by configuration generation cleanup.

---

## 22 Current-code fit

### 22.1 Summary

| Item | Treatment |
| --- | --- |
| Current anchor | `packages/d2b-host/src/runtime_provider.rs` (`CloudHypervisorRuntimeProvider`, `CloudHypervisorRuntimeControl`); `packages/d2b-host/src/ch_argv.rs` (`ChArgvInput`, `ChArgvGenerator`); `packages/d2bd/src/provider_shutdown.rs` (`CloudHypervisorShutdown`); `packages/d2b-core/src/processes.rs` (`ProcessRole::CloudHypervisor`, `ProcessRole::Swtpm`, `ProcessRole::NetVm`); `packages/d2b-host-providers/src/lib.rs` (`RuntimeProvider` adapter); `nixos-modules/components/tpm.nix`; `nixos-modules/network.nix`; `nixos-modules/processes-json.nix` (VMM/swtpm/net-VM process node emitters); `nixos-modules/store.nix` |
| Evidence class | `production-reachable` for all items above; see migration map §2 |
| Behavior retained | Typed argv generation (pure data, no syscalls); pidfd identity/adoption; direct cgroup placement; fail-closed adoption ambiguity; redacted Debug for paths/argv; process-scoped TAP net-handoff; broker privilege mediation; minijail sandbox with user-NS for virtiofsd; swtpm pre-start flush; watchdog emission; OEM strings for observability |
| Required delta | Controller as async ResourceReconciler; Guest ResourceSpec validation; VMM Process as single owned child resource; direct dependency-readiness gate via ResourceClient (no EphemeralProcess); ComponentSession guest-control health in observe handler; typed provider descriptor; framework-provisioned ProviderStateSet; bus-only resource access; ResourceMutationBatch status writes; explicit Device/kvm in Guest deviceAttachments; required controllerExecutionRef in Provider config |
| Reuse path | See §22.2 |
| Replacement/deletion | Current `d2b-<vm>-vm.service` systemd unit, `SpawnRunner{role: CloudHypervisor}` broker op, `RuntimeProvider` trait calls, and `CloudHypervisorRuntimeProvider` adapter remain until runtime-cloud-hypervisor integration passes full test parity |
| Feasibility proof | `ADR046-ch-001` spike |

### 22.2 Detailed reuse plan

| Current symbol / path | Evidence class | Current callers | Reuse action | v3 destination |
| --- | --- | --- | --- | --- |
| `d2b-host/src/ch_argv.rs::ChArgvInput`, `generate_ch_argv` | production-reachable | `d2b-host/src/runtime_provider.rs` | EXTRACT and ADAPT | `packages/d2b-provider-runtime-cloud-hypervisor/src/vmm_argv.rs`; `ChArgvInput` fields are renamed to align with `spec.provider.settings` schema; store paths move to private artifact-catalog resolution; no `spec.*` exposure |
| `d2b-host/src/runtime_provider.rs::CloudHypervisorRuntimeProvider` | production-reachable | `d2b-host-providers/src/lib.rs`; `d2bd/src/lib.rs` | REPLACE | `packages/d2b-provider-runtime-cloud-hypervisor/src/controller.rs`; new controller owns reconcile loop, not a `RuntimeProvider` trait implementation |
| `d2b-host/src/runtime_provider.rs::CloudHypervisorRuntimeControl` trait | production-reachable | `d2bd`; supervisor test seams | REPLACE | Supervisor ticket passed through `Provider/system-minijail` LaunchTicket; no ambient trait |
| `d2bd/src/provider_shutdown.rs::CloudHypervisorShutdown` | production-reachable | `d2bd` graceful shutdown | ADAPT | `packages/d2b-provider-runtime-cloud-hypervisor/src/shutdown.rs`; integrates with Process finalizer drain handler |
| `d2b-core/src/processes.rs::ProcessRole::CloudHypervisor` | production-reachable | Broker `SpawnRunner`; process DAG | REPLACE | `cloud-hypervisor-runner` Process template; old ProcessRole variant deleted after integration |
| `d2b-core/src/processes.rs::ProcessRole::Swtpm` | production-reachable | Broker `SpawnRunner{role: Swtpm}`; swtpm_dir provisioning | MOVE | Owned by `Provider/device-tpm`; work item `ADR046-device-tpm-001` |
| `d2b-core/src/processes.rs::ProcessRole::NetVm` | production-reachable | Auto net VM bootstrap | REPLACE | `Guest/<network-name>-net-vm` resource created by network-local controller; VMM process is a `cloud-hypervisor-runner` Process |
| `d2b-host/src/swtpm_argv.rs` | production-reachable | `d2bd` swtpm start | MOVE | `packages/d2b-provider-device-tpm/src/swtpm_argv.rs`; no work item for this Provider dossier |
| `d2b-host/src/virtiofsd_argv.rs` | production-reachable | `d2bd` virtiofsd start | MOVE | `packages/d2b-provider-volume-virtiofs/src/virtiofsd_argv.rs` |
| `nixos-modules/processes-json.nix` (CloudHypervisor/Swtpm/NetVm node emitters) | nix-emitted | Bundle artifact consumer | REPLACE | `packages/d2b-provider-runtime-cloud-hypervisor/` Nix builder per `ADR-046-nix-configuration`; current emitters deleted after integration |
| `nixos-modules/store.nix` | nix-emitted | Per-VM hardlink farm setup | REPLACE | `Provider/volume-local` owns Volume with `VolumeKind: state` for store farm; the controller watches Volume readiness via ResourceClient before creating the VMM Process — no EphemeralProcess preflight |
| `tests/golden/runner-shape/cloud-hypervisor-argv-*.txt` | test-only | `tests/virtiofsd-argv-shape.sh`, `tests/video-contract-eval.sh` | COPY/ADAPT | `packages/d2b-provider-runtime-cloud-hypervisor/tests/vmm_argv_golden_test.rs`; new golden vectors for v3 spec-driven argv; old shell golden tests adapted to `integration/` |
| `tests/video-sidecar-hardening-eval.sh` | test-only | `make test-policy` | ADAPT | `packages/d2b-provider-runtime-cloud-hypervisor/integration/video_sidecar_integration_test.rs`; device-gpu Provider must also have a corresponding test |
| `packages/d2bd/src/metrics.rs` (`d2b_daemon_vm_*` with `vm=` label) | production-reachable | Current Prometheus hand-roll | REPLACE | `d2b_runtime_ch_*` metrics from §18.3; `vm=` label removed from metric labels; VM identity stays in OTEL resource attributes only |

---

### Fast hermetic execution and test placement (D094)

Per D094 and `ADR-046-validation-and-delivery` §10.16, this Provider's `src/`
unit tests and `tests/*.rs` hermetic suite are fast, in-process, deterministic,
and parallel-safe: an individual normal test has p95 ≤50 ms with no wall-clock
sleep, and `cargo test -p d2b-provider-runtime-cloud-hypervisor --lib --tests`
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

## 23 Implementation work items

### ADR046-ch-001 (feasibility spike)

| Field | Value |
| --- | --- |
| Dependency/owner | Provider toolkit / system-minijail; W1 spike owner |
| Current source | `d2b-host/src/runtime_provider.rs`; `d2b-host/src/ch_argv.rs`; `d2bd/src/supervisor/dag.rs` |
| Reuse action | Extract and adapt |
| Destination | `packages/d2b-provider-runtime-cloud-hypervisor/src/controller.rs` |
| Detailed design | End-to-end: single Guest reconcile → synchronous dependency-readiness check via ResourceClient → VMM Process creation → guest-control health check in observe handler → status write. Uses fake bus/store/supervisor stubs from toolkit. Proves fast-path latency gates (≤5 ms hint, ≤20 ms VMM Process creation when all deps ready). No EphemeralProcess resources at any step. |
| Integration | Zone ResourceClient + system-minijail Process Provider + fake broker effect |
| Data migration | None (spike) |
| Validation | Unit: reconcile state machine, fast-path latency, adoption/ambiguity, finalize ordering. Integration: end-to-end VMM boot with real KVM and guest-control session (requires `make test-host-integration`) |
| Removal proof | Not applicable (new crate) |

### ADR046-ch-002 (Guest bootstrap graph)

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-ch-001; Volume and Device foundation |
| Current source | `d2b-core/src/processes.rs`; `nixos-modules/processes-json.nix`; `d2b-priv-broker/src/ops/swtpm_dir.rs`; `d2b-host/src/swtpm_argv.rs` |
| Reuse action | EXTRACT and REPLACE |
| Destination | `packages/d2b-provider-runtime-cloud-hypervisor/src/bootstrap_graph.rs` |
| Detailed design | Single owned VMM Process resource; synchronous ResourceClient dependency check (Device/kvm + all declared Devices, Networks, virtiofs Volumes); immediate Process creation when all deps ready; no EphemeralProcess resources; conditional net-VM Guest creation; per-dependency readiness tracking in reconcile loop |
| Integration | Depends on `Provider/volume-virtiofs`, `Provider/device-tpm`, `Provider/device-kvm`, `Provider/network-local` ResourceType readiness |
| Data migration | v3 reset; no v2 process graph migration |
| Validation | Golden VMM Process spec vectors; dependency-ordering tests; parallel Guest tests (8 concurrent); net-VM creation tests; Device/kvm explicit-ref enforcement |
| Removal proof | `ProcessRole::CloudHypervisor`, `ProcessRole::Swtpm`, `ProcessRole::NetVm` variant callers deleted; `nixos-modules/processes-json.nix` VMM emitter deleted after parity |

### ADR046-ch-003 (VMM argv builder v3)

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-ch-002; artifact catalog foundation |
| Current source | `d2b-host/src/ch_argv.rs::ChArgvInput`, `generate_ch_argv`; `tests/golden/runner-shape/cloud-hypervisor-argv-*.txt` |
| Reuse action | COPY/ADAPT |
| Destination | `packages/d2b-provider-runtime-cloud-hypervisor/src/vmm_argv.rs`; `tests/vmm_argv_golden_test.rs` |
| Detailed design | `VmmArgvInput` derived from validated `GuestSpec.spec.provider.settings`; kernel/initrd/rootfs paths resolved privately from artifact catalog at dispatch time; no path in spec/status; golden tests for headless/q35/microvm/gpu/video/macvtap variants |
| Integration | ProviderSupervisor LaunchTicket resolution |
| Data migration | None |
| Validation | Golden argv vectors matching `cloud-hypervisor-argv-*.txt` shapes with v3 adaptations; redaction test (no store path in Debug output) |
| Removal proof | `d2b-host/src/ch_argv.rs::generate_ch_argv` callers removed; old golden test files adapted |

### ADR046-ch-004 (Nix resource compiler)

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-ch-002; nix-configuration foundation (`ADR046-identities-002`) |
| Current source | `nixos-modules/options-realms-workloads.nix`; `nixos-modules/options-vms.nix`; `nixos-modules/processes-json.nix`; `nixos-modules/store.nix` |
| Reuse action | ADAPT and REPLACE |
| Destination | `packages/d2b-provider-runtime-cloud-hypervisor/nix/` (Nix emitter); `nixos-modules/` option extension for `runtime-cloud-hypervisor` Guest schema |
| Detailed design | `d2b.zones.<z>.resources.<n>` with `type = "Guest"` and `spec.provider.settings` validated against signed Provider schema; `spec.systemArtifactId` top-level field; artifact catalog `type = "nixos-system"` enforced by rule 17; Guest-control `Endpoint` resource emitted without raw locator; `make test-drift` gate for schema/Nix drift |
| Integration | Zone resource bundle emission; private artifact catalog; `xtask gen-resource-nix-options` for auto-generated Nix option types |
| Data migration | `d2b.vms.<vm>` → `d2b.zones.<z>.resources.<n>` documented in migration guide |
| Validation | nix-unit eval tests: rule CH-1 through CH-4 + rules 1–17; golden resource bundle JSON (no store path); type-mismatch eval errors; raw locator rejection; `spec.systemArtifactId` at top-level in JSON (not in `spec.provider.settings`) |
| Removal proof | `options-vms.nix`; `options-realms-workloads.nix` (LocalVm path); `nixos-modules/processes-json.nix` (VMM emitter); `nixos-modules/store.nix` removed after integration parity |

### ADR046-ch-005 (guest-control health and adoption)

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-ch-001; ComponentSession/d2b-bus (`ADR046-componentsession-001`) |
| Current source | `packages/d2bd/src/provider_shutdown.rs::GracefulVmShutdown`; `packages/d2b-host/src/runtime_provider.rs::RuntimeProvider::plan_guest_update` |
| Reuse action | ADAPT |
| Destination | `packages/d2b-provider-runtime-cloud-hypervisor/src/health.rs`; `src/adoption.rs` |
| Detailed design | Authenticated KK ComponentSession health check over vsock; adoption verification (pid/cgroup/executable/generation) within `adoptionWindow`; ambiguity → Unknown/Degraded, never broad kill; graceful shutdown via guest-control session before SIGTERM |
| Integration | ComponentSession enrolled KK; guest bootstrap credential from `d2b-gctl` virtiofs share; `GuestReachable` condition write |
| Data migration | None |
| Validation | Fake guest-control server test; health check timeout/failure/retry; adoption property test (ambiguity, gone, stale pid); graceful shutdown ordering |
| Removal proof | `ProcessRole::GuestControlHealth` observation path; `ProcessRole::GuestSshReadiness` deleted at cutover |

### ADR046-ch-006 (metrics and audit)

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-ch-001; telemetry foundation (`ADR046-telemetry-001`) |
| Current source | `packages/d2bd/src/metrics.rs` (`d2b_daemon_vm_*`); `packages/d2b-contract-tests/tests/policy_observability.rs` |
| Reuse action | REPLACE |
| Destination | `packages/d2b-provider-runtime-cloud-hypervisor/src/metrics.rs`; `src/audit.rs` |
| Detailed design | `d2b_runtime_ch_*` metrics from §18.3; bounded durable audit records from §17.3; no `vm=` metric label; no path/argv/socket in any field; closed OTEL attribute allowlist extended per §18.4 |
| Integration | Zone lightweight bounded emitter; `Provider/observability-otel` forwarding |
| Data migration | `d2b_daemon_vm_*` metrics retired; consumers must update dashboards |
| Validation | `policy_observability.rs` updated with v3 allowlist; cardinality tests; bounded message/field tests; audit record schema golden vectors |
| Removal proof | Hand-rolled Prometheus registry (`d2bd/src/metrics.rs` `d2b_daemon_vm_*` section) deleted after migration |

### ADR046-ch-007 (controller status-first operational state)

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-ch-001; `ADR046-pstate-001` (common status types) |
| Current source | `packages/d2b-core/src/storage.rs` (`StoragePathSpec`, `SensitivityClass`) — to be retired |
| Reuse action | REPLACE (storage.rs) |
| Destination | `packages/d2b-provider-runtime-cloud-hypervisor/src/state.rs`; `packages/d2b-provider-runtime-cloud-hypervisor/tests/state_status_test.rs` |
| Detailed design | `state.rs` owns the controller's bounded non-secret operational-state projection into the owning resource's `status` subresource (reconcile stage, per-Guest launch/adoption observations, bounded counters, closed-enum error detail) — the controller declares no Provider state Volume and mounts no `/state`; on restart it re-derives observed state from the Zone resource store, the core Operation ledger, and external observation (running VMM/virtiofsd re-adopted from cgroup leaves + fresh pidfds), treating `status` as observation, never authority (D087); status writes occur only on material change and stay within the status bounds |
| Integration | The controller reads Volume/Device/Network dependency status through its ComponentSession/ResourceClient and writes its own bounded `status`; no Provider state Volume is provisioned or mounted |
| Data migration | v3 reset; no v2 state storage migration |
| Validation | `state_status_test.rs` (hermetic): status projection round-trip and bound enforcement; restart re-derivation from store/ledger/external observation without a state Volume; no secret/path/argv/PID in status |
| Removal proof | `d2b-core/src/storage.rs` `StoragePathSpec` / `SensitivityClass` retired only after all Provider state consumers migrate to v3 status/optional-Volume helpers |
| Integration | Volume is provisioned and mounted by ProviderDeployment before controller Process launch (`required: true` mount); controller observes Volume status only through its ComponentSession, never through direct ResourceClient Volume verbs |
| Data migration | v3 reset; no v2 state storage migration |
| Validation | `state_volume_test.rs` (hermetic): StateEnvelope round-trip, view helper correctness, startup-check behavior when Volume phase is not current |
| Removal proof | `d2b-core/src/storage.rs` `StoragePathSpec` / `SensitivityClass` retired only after all Provider state consumers migrate to v3 Volume state helpers |

Per `ADR-046-provider-model-and-packaging` and `ADR-046-nix-configuration`, the
workspace policy gate rejects the crate unless all four paths exist:

```text
packages/d2b-provider-runtime-cloud-hypervisor/
  src/
    lib.rs                       # crate root; re-exports controller, guest_spec, vmm_argv
    controller.rs                # async ResourceReconciler, describe/validate/plan/reconcile/finalize/observe
    bootstrap_graph.rs           # VMM Process spec builder and dependency-readiness check
    vmm_argv.rs                  # VmmArgvInput, vmm_argv_build (pure; no store paths in output)
    guest_spec.rs                # GuestProviderSpecSettings, spec.provider.settings schema, validateSpec
    health.rs                    # ComponentSession KK health check, GuestReachable condition (observe)
    adoption.rs                  # pidfd adoption, ambiguity detection, quarantine
    shutdown.rs                  # graceful shutdown via guest-control session
    metrics.rs                   # d2b_runtime_ch_* metric definitions
    audit.rs                     # bounded durable audit record types and emit helpers
    state.rs                     # controller status-first operational-state projection helpers (no state Volume)
  tests/
    vmm_argv_golden_test.rs      # golden argv vectors (headless, q35, gpu, video, macvtap)
    guest_spec_validation_test.rs # validateSpec: Endpoint required, memoryShared, systemArtifactId,
                                 # controllerExecutionRef; rejects cmdlineExtra/seccompOverride
    bootstrap_graph_test.rs      # VMM Process spec construction, dependency ordering,
                                 # immediate-launch when all deps ready, drift repair
    reconcile_state_machine_test.rs # full reconcile handler state machine
    adoption_property_test.rs    # pidfd adoption: gone/ambiguous/stale-pid property tests
    health_check_test.rs         # fake guest-control server; timeout/failure/retry (observe handler)
    finalize_ordering_test.rs    # finalizer algorithm, single VMM Process teardown, ambiguity
    metrics_cardinality_test.rs  # no vm= label; bounded audit fields; no path/argv in output
    schema_golden_test.rs        # spec.provider.settings JSON Schema golden vector (no cmdlineExtra/seccompOverride)
    redaction_test.rs            # no store path in Debug, status, or audit output
    state_status_test.rs         # status projection round-trip; bound enforcement;
                                 # restart re-derivation without a state Volume
  integration/
    README.md                    # (optional) how to run integration fixtures; prerequisites
    vmm_boot_test.rs             # single Guest boot + guest-control health on real KVM
    vmm_adoption_test.rs         # controller restart + pidfd adoption with running VMM
    vmm_restart_test.rs          # unexpected VMM exit + backoff + restart + re-health
    network_attachment_test.rs   # TAP allocation, macvtap external attachment
    device_kvm_test.rs           # explicit Device/kvm dependency; TCG fallback without it
    device_tpm_test.rs           # swtpm Device dependency + VMM boot with TPM socket
    device_gpu_test.rs           # GPU Device dependency + vhost-user-gpu handoff
    net_vm_test.rs               # auto-declared net-VM Guest via network-local Provider
    parallel_guests_test.rs      # 8 concurrent Guests; fast-path p95 latency gate
    dependency_gate_test.rs      # VMM not created until all Device/Network/Volume deps Ready;
                                 # immediate launch when final dep fires
  README.md                      # Provider identity, config (incl. controllerExecutionRef),
                                 # ResourceTypes, placement, RBAC, security, build/test/integration
                                 # commands, standalone-repository consumption instructions
```

### 24.1 Integration fixture host/guest fixtures

Each file under `integration/` uses the existing repository test orchestration
(`make test-host-integration` → `tests/host-integration/*.nix`
`runNixOSTest` VM checks). The integration fixtures require:

- `d2b.zones.test-zone.resources.runtime-cloud-hypervisor`: installed Provider
  with `config.controllerExecutionRef = "Host/test-host"`;
- `d2b.zones.test-zone.resources.host-system`: Host resource;
- `d2b.zones.test-zone.resources.<guest-name>`: Guest resource with
  `type = "nixos-system"` artifact and explicit `Device/<name>-kvm` in
  `deviceAttachments`;
- a NixOS host with KVM (`/dev/kvm`); falls back to slow TCG if absent (log
  warning; do not fail closed for TCG-only CI);
- `device_kvm_test.rs` explicitly validates both KVM and TCG paths;
- the `d2b-controller-toolkit` fake-bus adapters for unit tests;
- `tests/integration/containers/` for container-level (non-KVM) integration
  (e.g., `vmm_argv_golden_test` can run without KVM).

Real Host/Guest fixtures are declared as `runNixOSTest` modules in
`tests/host-integration/runtime-cloud-hypervisor.nix`. They import the
`integration/` Rust test binaries as NixOS test scripts and report pass/fail
back to the `make test-host-integration` gate.

---

## 25 Removal proof requirements

A work item is **not complete** until:

1. The v3 controller integration test passes for the replaced behavior
   (hermetic `tests/` + host/KVM integration/).
2. All callers of the replaced current symbol compile against the new interface.
3. The removed symbol/unit/emitter is absent from the codebase (confirmed by
   `grep` in CI).
4. Removed Nix modules no longer appear in any module import chain.
5. `make test-drift` passes with the updated schema/Nix option alignment.
6. The `d2b-host-providers` adapter crate is empty of `runtime_provider`
   content and scheduled for deletion once all Provider dossiers migrate (per
   `ADR046-provider-002`).

The following specific removals are gated on ADR046-ch integration parity:

| Current artifact | Removal gate |
| --- | --- |
| `d2b-host/src/runtime_provider.rs::CloudHypervisorRuntimeProvider` | ADR046-ch-002 parity |
| `d2b-core/src/processes.rs::ProcessRole::{CloudHypervisor, NetVm}` | ADR046-ch-002 parity |
| `nixos-modules/processes-json.nix` (VMM/NetVm process node emitters) | ADR046-ch-004 parity |
| `nixos-modules/options-vms.nix` | ADR046-ch-004 parity + all VM consumers migrated |
| `d2b-<vm>-vm.service` systemd unit | ADR046-ch-002 parity |
| `d2bd/src/metrics.rs` `d2b_daemon_vm_*` section | ADR046-ch-006 parity |
| `nixos-modules/store.nix` (hardlink farm emitter) | ADR046-ch-004 parity + volume-local parity |
| `ProcessRole::GuestSshReadiness` | v3 cutover (no compat window) |
| `ProcessRole::GuestControlHealth` (as standalone role) | ADR046-ch-005 parity |

Per D094, each replaced current-code test is retired with an explicit
keep/adapt/move/delete disposition and a removal gate: the minimum reusable
semantic assertions migrate into this crate's hermetic `tests/`, and the old
duplicate tests, shell gates, fixtures, static artifacts, CI jobs, and manifest
entries are deleted once successor coverage and the removal proof pass —
updating `tests/layer1-jobs.json`, the closed gate manifests, the
flake/matrix/Nix-unit pins, the generated ledgers, and the CI workflow shards.
Old and new suites never run in parallel indefinitely.
