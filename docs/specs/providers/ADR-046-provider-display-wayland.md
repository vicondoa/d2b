# ADR 0046 Provider dossier: `display-wayland`

| Field | Value |
| --- | --- |
| Spec ID | `ADR-046-provider-display-wayland` |
| Crate | `packages/d2b-provider-display-wayland/` |
| Provider ResourceRef | `Provider/display-wayland` |
| Parent | ADR 0046 |
| Status | Proposed |
| Version | 2 |
| Baseline | `b5ddbed67867d9244bf33390868101bd9b053e49` |
| Normative | Yes |
| Owners | display-wayland crate owner, Wayland proxy binary, Nix integration |
| Depends on | `ADR-046-provider-model-and-packaging`, `ADR-046-components-processes-and-sandbox`, `ADR-046-componentsession-and-bus`, `ADR-046-resources-volume`, `ADR-046-resources-device`, `ADR-046-resources-host-guest-process-user`, `ADR-046-nix-configuration`, `ADR-046-telemetry-audit-and-support`, `ADR-046-resource-reconciliation`, `ADR-046-resource-api-and-authorization` |
| Supersedes | `ProcessRole::WaylandProxy` in `packages/d2b-core/src/processes.rs`; `LocalCrossDomainWaylandProvider` in `packages/d2b-host-providers/src/lib.rs`; `generate_wayland_proxy_argv` in `packages/d2b-host/src/wayland_proxy_argv.rs`; `nixos-modules/components/graphics.nix` `graphics.waylandProxy.*` options; `nixos-modules/ui-colors.nix` VM border color resolution; current `pkgs/wl-cross-domain-proxy` guest binary |

---

## 1. Purpose and scope

`Provider/display-wayland` is the d2b 3.0 Wayland display Provider. It owns
the complete Wayland cross-domain display pipeline for local VM Guests: the
jailed Host proxy process that mediates between the virtio-gpu/cross-domain
channel and the host compositor, and the in-guest frontend process that drives
the guest-side virtio-gpu Wayland cross-domain transport. It manages endpoint
streams, UI identity metadata (color/border/label), and filter policy over
the cross-domain channel.

This Provider does **not** own the GPU device allocation or the VMM process.
Those are owned by `Provider/device-gpu` and `Provider/runtime-cloud-hypervisor`
respectively. This Provider consumes `Device/display-wayland` endpoints exposed
by the GPU Provider and coordinates with the clipboard Provider
(`Provider/clipboard-wayland`) through the d2b-bus bridge socket.

The Provider does **not** own any audio path. Audio remains in
`Provider/audio-pipewire`.

The Provider does **not** implement a direct compositor fallback. When the
Host proxy component is not Ready, the cross-domain Wayland channel is
unavailable and the Guest application surface is not forwarded. There is no
transparent fallback to a direct compositor socket or a second IPC path.

---

## 2. Provider identity

```text
providerRef:   Provider/display-wayland
artifactId:    display-wayland
crate:         packages/d2b-provider-display-wayland/
```

Required crate layout:

```text
packages/d2b-provider-display-wayland/
  src/
  tests/
  integration/
  README.md
```

All four paths are mandatory per `ADR-046-provider-model-and-packaging`. The
workspace policy gate enforces this.

---

## 3. ResourceTypes

`Provider/display-wayland` does not implement any of the standard ResourceTypes
(Volume, Network, Device, Process, Host, Guest). It implements and exports the
following Provider-specific ResourceTypes:

| ResourceType | Short name | Responsibility |
| --- | --- | --- |
| `display-wayland.d2b.io.WaylandSession` | `WaylandSession` | One active Wayland cross-domain display session between a Guest and the host compositor, including endpoint references, filter policy, identity metadata, and lifecycle |
| `display-wayland.d2b.io.WaylandPolicy` | `WaylandPolicy` | Zone-scoped Wayland global filter policy template: classified global allowlist/denylist, version caps, dmabuf filters, identity rail settings |

Both ResourceTypes are scoped to the owning Zone and follow the standard
resource envelope contract from `ADR-046-resource-object-model`.

---

## 4. Components

### 4.1 Component table

| ID | Type | Binary | Domain | Placement | Cardinality |
| --- | --- | --- | --- | --- | --- |
| `host-proxy-controller` | controller | `d2b-display-wayland-host-proxy` | system | one per Host, on `Host/<name>` | 1 per Zone Host |
| `guest-frontend-controller` | controller | `d2b-display-wayland-guest-frontend` | system | one per Guest, on `Guest/<name>` where `providerSettings.displayWayland.enable = true` | 1 per enabled Guest |
| `policy-controller` | controller | `d2b-display-wayland-policy` | system | one per Zone, on the Zone self resource | 1 per Zone |

The `policy-controller` watches `WaylandPolicy` resources and updates the
filter policy digest served to `host-proxy-controller` instances. It does not
own any Process.

### 4.2 `host-proxy-controller`

Watches `WaylandSession` resources owned by each Host. For every Ready
`WaylandSession`:

1. Verifies that the referenced Guest `Device` (GPU cross-domain endpoint) is
   Ready and that the `device-gpu` Provider has exposed the cross-domain
   socket endpoint to the Zone.
2. Reads the `WaylandPolicy` compiled digest for this session.
3. Creates and manages a `Process` resource for the Host proxy worker (see
   §6.1).
4. Writes `WaylandSession.status` with endpoint revision, proxy process ref,
   and readiness.

On Guest stop or `WaylandSession` deletion the controller finalizes the proxy
Process. The proxy socket is an implementation detail of the Process Volume
mount; it is never exposed in status or audit.

### 4.3 `guest-frontend-controller`

Watches Guest resources where `spec.providerSettings.displayWayland.enable =
true`. For each such Guest:

1. Creates and manages a `Process` resource for the in-guest Wayland frontend
   worker (see §6.2). The process runs in the `system` domain under the Guest.
2. Creates the `WaylandSession` resource that `host-proxy-controller` will
   pick up.
3. Writes `WaylandSession.status.guestFrontendRef`.

### 4.4 `policy-controller`

Watches `WaylandPolicy` resources in the Zone. On create/update:

1. Validates the policy schema.
2. Compiles an ordered canonical policy digest into controller-local in-memory
   state.
3. Signals `host-proxy-controller` instances through the owner-trigger mechanism.

`WaylandPolicy` resources that reference unknown interface names are accepted
with a `UnknownInterface` condition. Policy violations produce `PolicyWarning`
audit records, not hard admission failures, because the list of advertised
interfaces is host-compositor-dependent.

---

## 5. WaylandSession ResourceSpec

```yaml
apiVersion: resources.d2b.io/v3
type: display-wayland.d2b.io.WaylandSession
metadata:
  name: corp-vm-display
  zone: dev
  uid: <store-generated>
  generation: 1
  ownerRef: Guest/corp-vm
  finalizers: [display-wayland.d2b.io/proxy-stopped]
spec:
  guestRef: Guest/corp-vm
  hostRef: Host/host-system
  policyRef: WaylandPolicy/default
  # UI identity metadata — compositor-agnostic
  identity:
    label: "corp-vm"              # max 64 chars; validated against Guest name
    activeColor: "#7fc8ff"        # #rrggbb hex; required
    inactiveColor: "#45475a"      # #rrggbb hex; defaults to activeColor
    urgentColor: "#f38ba8"        # #rrggbb hex; defaults to activeColor
    border:
      enable: true
      railWidth: 9                # logical pixels; 0 = no rail; max 64
    label:
      enable: true
      text: null                  # null = use identity.label; "" = suppress text
      position: top-left          # top-left | top-center
  crossDomainTrusted: true        # must be true; false is rejected at admission
  virglVideo: false               # opt-in experimental virglrenderer video path
  filter:
    debugLogging: false
    byteLogging: false
    denyGlobals: []
    allowGlobals: []
    maxVersions: {}
    dmabufAllow: []
    dmabufDeny: []
status: {}
```

### 5.1 WaylandSession spec field reference

| Field | Type | Required | Default | Bounds | Notes |
| --- | --- | --- | --- | --- | --- |
| `guestRef` | ResourceRef | yes | — | `Guest/<name>` | Target Guest |
| `hostRef` | ResourceRef | yes | — | `Host/<name>` | Host running the proxy Process |
| `policyRef` | ResourceRef | yes | — | `WaylandPolicy/<name>` | Resolved policy template |
| `identity.label` | string | yes | — | 1..64 chars; `^[a-z][a-z0-9-]*$` | Authenticated display label; matches Guest name by default |
| `identity.activeColor` | string | yes | — | `^#[0-9a-fA-F]{6}$` | Active/focused identity color |
| `identity.inactiveColor` | string | no | `activeColor` | `^#[0-9a-fA-F]{6}$` | Inactive/unfocused identity color |
| `identity.urgentColor` | string | no | `activeColor` | `^#[0-9a-fA-F]{6}$` | Urgent state identity color |
| `identity.border.enable` | bool | no | `true` | — | Proxy-drawn identity rail |
| `identity.border.railWidth` | u32 | no | `9` | 0..64 | Left rail width in logical pixels; 0 disables the rail |
| `identity.label.enable` | bool | no | `true` | — | Proxy-drawn identity label |
| `identity.label.text` | string? | no | `null` | max 64 chars | `null` uses `identity.label`; `""` suppresses the label text |
| `identity.label.position` | enum | no | `top-left` | `top-left \| top-center` | Label position |
| `crossDomainTrusted` | bool | yes | — | must be `true` | Explicit opt-in for cross-domain Wayland forwarding; `false` is rejected at spec admission |
| `virglVideo` | bool | no | `false` | — | Opt-in experimental virglrenderer video path; gated by `device-gpu` descriptor |
| `filter.debugLogging` | bool | no | `false` | — | Verbose Wayland protocol tracing; payload metadata may appear in logs; not for production |
| `filter.byteLogging` | bool | no | `false` | — | Raw transport hexdump logging; not for production |
| `filter.denyGlobals` | `[string]` | no | `[]` | max 128 items; each max 63 chars | Additional globals to deny beyond policy defaults |
| `filter.allowGlobals` | `[string]` | no | `[]` | max 128 items; each max 63 chars | Globals to allow; clipboard-boundary globals are ignored and produce audit advisory |
| `filter.maxVersions` | `map<string,u32>` | no | `{}` | max 128 entries | Per-interface version caps |
| `filter.dmabufAllow` | `[string]` | no | `[]` | max 64 items | dmabuf format/modifier allow rules: `FORMAT[:MODIFIER]` |
| `filter.dmabufDeny` | `[string]` | no | `[]` | max 64 items | dmabuf format/modifier deny rules: `FORMAT[:MODIFIER]` |

`crossDomainTrusted: false` is rejected at spec admission because the
cross-domain Wayland channel is the only transport this Provider serves.
When cross-domain is not desired the operator should not declare a
`WaylandSession` for that Guest.

### 5.2 WaylandSession status

```yaml
status:
  phase: Pending | Ready | Degraded | Failed | Unknown
  observedGeneration: 1
  conditions:
    - type: ProxyReady
      status: "True"
      reason: proxy-process-ready
    - type: GuestFrontendReady
      status: "True"
      reason: guest-process-ready
    - type: PolicyApplied
      status: "True"
      reason: policy-digest-applied
    - type: GpuEndpointAvailable
      status: "True"
      reason: device-gpu-endpoint-ready
  lastReconciledAt: null
  session:
    proxyProcessRef: Process/corp-vm-display-proxy
    guestFrontendProcessRef: Process/corp-vm-display-guest
    policyDigest: sha256:<hex>
    endpointRevision: <opaque>
```

`proxyProcessRef`, `guestFrontendProcessRef`, `policyDigest`, and
`endpointRevision` are opaque bounded strings. No socket path, compositor
socket name, user identity, window title, or raw argv appears in status.

### 5.3 Condition types

| Type | Meaning |
| --- | --- |
| `ProxyReady` | Host proxy Process is running and has emitted a readiness event |
| `GuestFrontendReady` | Guest frontend Process is running and connected |
| `PolicyApplied` | Compiled filter policy digest is current |
| `GpuEndpointAvailable` | `device-gpu` Provider advertises cross-domain capability and endpoint ID |
| `ClipboardBridgeReady` | Internal clipboard bridge to `clipboard-wayland` Provider is established; optional |
| `CrossDomainTrusted` | `spec.crossDomainTrusted = true` and GPU sidecar advertises cross-domain context type |

---

## 6. Process templates

### 6.1 Host proxy Process (`wayland-proxy-worker`)

The Host proxy is a long-lived `Process` resource owned by the `WaylandSession`
resource (via `metadata.ownerRef`).

```yaml
apiVersion: resources.d2b.io/v3
type: Process
metadata:
  name: corp-vm-display-proxy
  zone: dev
  ownerRef: display-wayland.d2b.io.WaylandSession/corp-vm-display
spec:
  providerRef: Provider/system-minijail
  executionRef: Host/host-system
  domain: system
  processClass: worker
  template: wayland-proxy-worker
  sandbox:
    namespaceClasses: [mount, pid]
    capabilityClasses: []
    seccompClass: w1-wayland-proxy
    noNewPrivileges: true
    startRoot: false
    environmentClass: minimal
    readOnlyRoot: true
    umask: "0022"
    oomScoreAdj: 0
  budget:
    memory:
      request: "32Mi"
      limit: "128Mi"
    pids:
      limit: 64
    fds:
      limit: 256
  mounts:
    - volumeRef: Volume/corp-vm-display-proxy-runtime
      view: proxy-runtime
      mountPath: /run/d2b-wlproxy
      access: read-write
  devices:
    - deviceRef: Device/corp-vm-gpu
      access: shared
      purpose: wayland-cross-domain-endpoint
  endpoints:
    - name: wayland-listen
      transport: unix
      purpose: wayland-cross-domain-listen
  telemetry:
    metricsEnabled: true
    tracingEnabled: true
    logLevel: info
    sensitiveLabels: false
```

**Process identity and naming:**

The process title is `d2b-<guest-name>-wlproxy` (derived from the authenticated
Guest resource name, max 63-char constraint). This title is the only
diagnostic-visible name for the process; it must not include compositor socket
names, user identities, or window content.

**Binary:** `d2b-display-wayland-host-proxy`

**Principal:** `d2b-<guest-name>-wlproxy` — a dedicated per-Guest system user.
No capabilities. No PipeWire/Pulse socket access.

**Sandbox:**
- Mount namespace: the per-session Volume (§7.1) provides the only writable
  surface at `/run/d2b-wlproxy`. The upstream compositor connection is
  established outside the jail by a same-UID authenticated user-session/display
  component that opens the connection and passes a pre-connected fd to the host
  proxy via ComponentSession attachment (§10). The host compositor socket path
  is never read from spec, status, or any ambient environment variable; no
  bind-mount of a compositor socket path occurs.
- The cross-domain GPU endpoint fd is received through the typed `device-gpu`
  ComponentSession service/descriptor handoff (§9). It is never a path or
  status field.
- `seccompClass: w1-wayland-proxy` — references the Provider-signed seccomp
  catalog entry, not a raw BPF program.
- `startRoot: false` — the process never holds root inside or outside the
  namespace.
- `userNamespace` is not required for this process class (the proxy holds no
  host capabilities requiring namespace fakery).

**Startup sequence (fail-closed):**
1. Resolve filter policy from the compiled `WaylandPolicy` digest. Exit
   non-zero on policy parse errors.
2. Receive the pre-connected upstream compositor fd delivered via
   ComponentSession attachment from the authenticated user-session/display
   component. Exit non-zero if no valid fd is received or if the connection
   is already broken. The per-session listen socket is **never** created
   before a live upstream fd is in hand.
3. Create the per-session listen socket inside the Volume mount.
4. Emit a `ProxyReadinessEvent` (bounded, path-free) over the internal
   readiness fd established by the supervisor.
5. Enter the dispatch loop.

The per-session listen socket path (the crosvm-facing side of the cross-domain
channel) is a private Volume implementation detail residing only in the Volume
mount and the LaunchTicket's sealed fd table. It is not public status, not
logged in its bound form, and not available to external callers. The
`device-gpu` Provider configures the VMM (crosvm) to connect to the proxy
using this path via its own process template and private bundle; the path
never surfaces in the resource API. The upstream compositor connection, by
contrast, is a pre-opened fd delivered via ComponentSession attachment with
no path exposed anywhere.

**ComponentSession transport:** the proxy binary communicates with the Zone
bus only through the `d2b-bus` ComponentSession over a local Unix transport
(inherited socketpair). It uses the NN profile (local purpose class, trusted
endpoint policy). The proxy never opens a second direct compositor connection,
an SSH path, or any out-of-band IPC fallback.

**No direct compositor fallback:** if the host compositor socket is
unavailable the proxy exits non-zero. The controller detects the failed Process
and sets `WaylandSession.status.phase = Failed`. There is no retry to a
different socket or a fallback display path.

### 6.2 Guest frontend Process (`wayland-frontend-worker`)

The guest frontend is a long-lived `Process` resource running in the `system`
domain under the Guest.

```yaml
apiVersion: resources.d2b.io/v3
type: Process
metadata:
  name: corp-vm-display-guest
  zone: dev
  ownerRef: display-wayland.d2b.io.WaylandSession/corp-vm-display
spec:
  providerRef: Provider/system-systemd
  executionRef: Guest/corp-vm
  domain: system
  processClass: worker
  template: wayland-frontend-worker
  sandbox:
    namespaceClasses: []
    capabilityClasses: []
    seccompClass: strict
    noNewPrivileges: true
    startRoot: false
    environmentClass: minimal
    readOnlyRoot: true
    umask: "0022"
    oomScoreAdj: 0
  budget:
    memory:
      request: "8Mi"
      limit: "32Mi"
    pids:
      limit: 16
    fds:
      limit: 64
  endpoints:
    - name: wayland-cross-domain
      transport: vsock
      purpose: wayland-cross-domain-guest
  telemetry:
    metricsEnabled: true
    tracingEnabled: true
    logLevel: info
    sensitiveLabels: false
```

**Binary:** `wl-cross-domain-proxy` (from `pkgs/wl-cross-domain-proxy`; see
§14 for the v3 migration path).

**Purpose:** drives the guest-side virtio-gpu Wayland cross-domain transport.
The binary connects to the in-guest virtio-gpu cross-domain context and
forwards Wayland protocol messages. Filtering, global hiding, and app-id
rewriting are performed by the Host proxy; the guest process performs no
filtering.

The binary does not use `--tag` or any filtering flag. It does not
connect to the compositor socket directly; it relies on the virtio-gpu
cross-domain context established by `Provider/device-gpu` and
`Provider/runtime-cloud-hypervisor`.

**Lifecycle gate:** this Process is not created until `crossDomainTrusted =
true` is confirmed in the `WaylandSession` spec. The `guest-frontend-controller`
checks this field at reconcile time and sets a `CrossDomainTrusted` condition
on the `WaylandSession` before creating the Process. A missing GPU cross-domain
context (because the VMM did not advertise the context type) causes the Process
to enter `Failed` phase; the controller sets `WaylandSession` to `Degraded`.

---

## 7. Volume resources

### 7.1 Host proxy runtime Volume

```yaml
apiVersion: resources.d2b.io/v3
type: Volume
metadata:
  name: corp-vm-display-proxy-runtime
  zone: dev
  ownerRef: display-wayland.d2b.io.WaylandSession/corp-vm-display
spec:
  providerRef: Provider/volume-local
  source:
    executionRef: Host/host-system
    settings:
      kind: tmpfs        # backend mount type; distinct from Volume.kind lifecycle
  kind: ephemeral        # lifecycle: removed on Host restart / session deletion
  layout:
    - path: ""
      type: directory
      ownerRef: User/d2b-corp-vm-wlproxy
      groupRef: User/d2b-corp-vm-wlproxy
      mode: "0700"
      sensitivity: private
      createPolicy: create-if-absent
      repairPolicy: exact-owner
      cleanupPolicy: owner-controlled
      noFollow: true
      accessAcl: []      # no additional access ACL entries beyond mode bits
      defaultAcl: []     # no default ACL entries propagated to new files
      invariants: [no-symlink]
  views:
    proxy-runtime:
      path: ""
      rights: [read, write, create, delete, traverse]
  quota:
    maxBytes: 4194304    # 4 MiB hard limit; sockets carry no payload bytes
    maxInodes: 64        # listen socket, clipboard bridge socket, lock files
    enforcement: hard
```

`source.settings.kind: tmpfs` selects the kernel tmpfs mount. `Volume.kind:
ephemeral` is the lifecycle classification: the volume is boot-scoped and
removed on Host restart or session deletion, which triggers `WaylandSession`
reconciliation. The tmpfs quota is enforced by the kernel mount options; the
charge is derived automatically from `source.executionRef` (`Host/host-system`)
without a separate `chargeTo` field.

The actual listen socket path inside the mount is a private implementation
detail derived from the Process template; it is never placed in the Volume spec,
resource status, logs, or audit. The `device-gpu` Provider receives the path
as a sealed bundle artifact, not through the resource API.

### 7.2 Policy controller state

The `policy-controller` holds compiled `WaylandPolicy` state in
controller-local in-memory state only. No durable Volume is required: the
policy is deterministically rebuilt from the `WaylandPolicy` ResourceSpec on
every controller restart. The `WaylandPolicy` ResourceSpec is itself durable
in the Zone store, making the compiled digest trivially recoverable.
There is no independent recovery need that would justify a separate persistent
Volume for this component.

---

## 8. WaylandPolicy ResourceSpec

```yaml
apiVersion: resources.d2b.io/v3
type: display-wayland.d2b.io.WaylandPolicy
metadata:
  name: default
  zone: dev
  uid: <store-generated>
  generation: 1
spec:
  denyGlobals: []
  allowGlobals: []
  maxVersions: {}
  dmabufAllow: []
  dmabufDeny: []
  defaults:
    acceleratedRendering: allow
    clipboardBoundary: virtualize
    highRisk: deny
    appDefaults: allow
    offDefaults: deny
    unclassified: deny
status: {}
```

### 8.1 WaylandPolicy spec fields

| Field | Type | Required | Default | Bounds | Notes |
| --- | --- | --- | --- | --- | --- |
| `denyGlobals` | `[string]` | no | `[]` | max 512 items; each max 63 chars | Zone-wide additional globals to deny |
| `allowGlobals` | `[string]` | no | `[]` | max 512 items; each max 63 chars | Zone-wide additional globals to allow; clipboard-boundary globals are ignored with advisory |
| `maxVersions` | `map<string,u32>` | no | `{}` | max 512 entries | Zone-wide per-interface version caps |
| `dmabufAllow` | `[string]` | no | `[]` | max 256 items | Zone-wide dmabuf allow rules |
| `dmabufDeny` | `[string]` | no | `[]` | max 256 items | Zone-wide dmabuf deny rules |
| `defaults.acceleratedRendering` | `allow \| deny` | no | `allow` | — | Default action for `AcceleratedRendering`-classified globals |
| `defaults.clipboardBoundary` | `virtualize \| deny` | no | `virtualize` | — | `virtualize`: synthesize clipboard locally; `deny`: hide all clipboard globals |
| `defaults.highRisk` | `allow \| deny` | no | `deny` | — | Default action for `HighRisk`-classified globals |
| `defaults.appDefaults` | `allow \| deny` | no | `allow` | — | Default action for `AppDefault`-classified globals |
| `defaults.offDefaults` | `allow \| deny` | no | `deny` | — | Default action for `OffDefault`-classified globals |
| `defaults.unclassified` | `allow \| deny` | no | `deny` | — | Default action for unclassified globals; overrides to `allow` produce audit advisory |

`WaylandSession.spec.filter.*` fields override the Zone-wide `WaylandPolicy`
for that session. Per-session overrides take precedence over Zone policy.
Neither per-session nor Zone policy can override the Required-baseline layer
(the fixed set of globals d2b always needs for graphics).

### 8.2 Classified global layers

The policy engine uses a four-layer classified allowlist (from lowest to
highest priority):

1. **Required-baseline** — core globals d2b needs for cross-domain Wayland
   graphics. These cannot be denied from config; a per-session `denyGlobals`
   entry for a Required-baseline global produces a `W-DENY-BASELINE` audit
   advisory and the entry is silently ignored.

2. **Secure feature defaults** — named classification bundles with safe
   defaults. Defaults are governed by `WaylandPolicy.spec.defaults.*`.

3. **Zone policy** — `WaylandPolicy` per-session overrides applied to all
   sessions referencing this policy.

4. **Session overrides** — `WaylandSession.spec.filter.*` per-session
   overrides with highest priority.

Unknown globals (not in any layer) are denied by default unless the operator
explicitly adds them to `allowGlobals` (which produces a `W-ALLOW-UNCLASSIFIED`
audit advisory).

### 8.3 Clipboard boundary

`wl_data_device_manager` and related clipboard/DnD globals are owned by the
virtual clipboard architecture (`Provider/clipboard-wayland`). This Provider
synthesizes a guest-visible `wl_data_device_manager` locally. Downstream
`wl_data_*` objects are **never** bound into the host compositor clipboard
namespace from within this Provider. Host and cross-realm clipboard
materialization routes through `d2b-clipd`.

Clipboard-boundary globals listed in `allowGlobals` are **ignored** (the
clipboard architecture does not permit forwarding) and produce a
`W-ALLOW-CLIPBOARD-BOUNDARY` audit advisory.

---

## 9. GPU dependency

`Provider/display-wayland` depends on `Provider/device-gpu` as a declared
optional dependency alias `gpu`. The GPU Provider must be installed and Ready
in the same Zone for `WaylandSession` resources to progress past `Pending`.

The dependency contract:

- `device-gpu` advertises an opaque capability/endpoint ID in the GPU Device
  status when the VMM advertises the `cross-domain` context type. No socket
  path or fd is exposed in the Device status or spec.
- `display-wayland` acquires the exact connected cross-domain fd through a
  typed `device-gpu` ComponentSession service/descriptor handoff, authenticated
  via an enrolled KK-profile ComponentSession between the two Provider
  controllers. The fd is passed as a ComponentSession attachment over the local
  Unix transport. No socket path for the GPU endpoint appears in any resource
  status, spec, or audit record.
- `virglVideo: true` in `WaylandSession.spec` is only valid when the GPU
  Device's `providerDiagnostic` reports video decode support. The controller
  produces a `VirglVideoUnsupported` condition if the GPU Device does not
  advertise this capability.
- The GPU cross-domain context type (`cross-domain`) must be enabled in the
  VMM configuration (`Provider/runtime-cloud-hypervisor`) for the session to
  become Ready. The `CloudHypervisorRunner` Process receives the cross-domain
  flag from the private bundle, not from a public resource field.
- When `Provider/device-gpu` is absent or the GPU Device is not Ready, the
  `GpuEndpointAvailable` condition is False, `WaylandSession.status.phase =
  Pending`, and the proxy Process is not created.

The `display-wayland` Provider must not query the GPU device node path, open
any DRM/render fd, or interact with the GPU hardware directly. It receives only
the cross-domain endpoint fd through the typed `device-gpu` ComponentSession
service/descriptor handoff; no socket path or device node is exposed to this
Provider.

---

## 10. ComponentSession transport and no direct compositor fallback

All intra-Zone communication from this Provider's processes uses `d2b-bus`
ComponentSession over local Unix transports (inherited socketpair or Unix
seqpacket). The Noise profile is `Noise_NN_25519_ChaChaPoly_SHA256` (local
purpose class; peer evidence from `SO_PEERCRED` + process identity).

The Host proxy process communicates:
- with the Zone runtime for status updates and lifecycle over d2b-bus;
- with the same-UID authenticated user-session/display component to receive
  the pre-opened upstream compositor fd as a ComponentSession attachment
  (NN-profile, local purpose class) before proxy startup;
- with `Provider/clipboard-wayland` over the internal clipboard bridge socket
  (a separate bounded per-session Unix socket, managed by the Volume mount,
  not the host compositor socket).

The clipboard bridge socket path is an implementation detail of the Volume
mount. It is not exposed in status, logs, or audit.

The proxy **does not** implement any fallback communication path to the host
compositor. If the upstream compositor fd received via ComponentSession
attachment is invalid or the connection breaks:
- The proxy emits a bounded `ProxyReadinessEvent` with
  `failure: upstream-unavailable` over the supervisor readiness fd.
- The proxy exits non-zero.
- The Process controller detects the exit and sets the Process phase to
  `Failed`.
- `host-proxy-controller` sets `WaylandSession.status.conditions.ProxyReady =
  False` and `WaylandSession.status.phase = Failed`.
- No second compositor fd or path is tried.
- No direct `WAYLAND_DISPLAY` environment variable is read or honored.
- No SSH or out-of-band channel is opened.

This fail-closed behavior is a hard requirement: the proxy jailed principal
(`d2b-<guest-name>-wlproxy`) holds no capabilities and no authority to open
compositor connections independently. The only upstream compositor connection
it holds is the pre-opened fd delivered via ComponentSession attachment.

---

## 11. UI color and border metadata

`Provider/display-wayland` is the authoritative consumer of the per-Guest
UI identity color and border metadata produced by the Zone resource bundle.

### 11.1 Color source of truth

In v3, the `WaylandSession.spec.identity.*` fields are the single source of
truth for the proxy identity colors. There is no separate `ui-colors.json`
artifact read by the proxy at runtime. The controller injects the resolved
colors into the Process template as sealed config (a `configRef` Volume
mounted read-only in the proxy's sandbox).

### 11.2 Color derivation

Colors are resolved at Nix eval time by the Zone resource bundle emitter:

1. If `d2b.zones.<zone>.resources.<session>.spec.identity.activeColor` is set
   explicitly, use that value.
2. Otherwise derive from the deterministic palette: `sha256("<seed>-<guest-name>")`
   selects a palette index from the 12-color set. This mirrors the current
   `colorForName "d2b-niri-border"` logic in `nixos-modules/ui-colors.nix`.

Colors must satisfy `^#[0-9a-fA-F]{6}$`. Values outside this pattern are
rejected at eval time.

### 11.3 Identity rail contract

The proxy-drawn identity rail (left vertical band, fixed `railWidth` logical
pixels) and optional label use only the resolved color triplet and a maximum
64-character label. The proxy binary:

- reads the sealed config Volume for color/rail/label settings;
- draws only wl_shm ARGB8888 surfaces using the memfd-backed in-process render
  path (current `packages/d2b-wayland-proxy/src/decoration.rs`);
- does not read app-ids, titles, or other compositor-managed metadata to
  determine the decoration color;
- always uses the authenticated Guest name as the fallback label when
  `identity.label.text` is null;
- logs neither the label text nor any guest surface title.

### 11.4 App-id prefix

The Host proxy prefixes every guest-originated xdg_toplevel app-id with
`d2b.<guest-name>.`. This prefix is injected using the `--app-id-prefix`
mechanism currently in `packages/d2b-host/src/wayland_proxy_argv.rs`. In v3
the prefix is derived from the authenticated `WaylandSession.spec.guestRef`
name and injected via sealed config; it is never a freeform string from an
untrusted source.

App-id values and window titles are not logged, not included in audit events,
and not exposed in resource status or metrics.

### 11.5 Title prefix

The Host proxy prepends a bounded title prefix (currently `[<vm>] `) to every
guest-originated window title forwarded to the host compositor. The title
prefix is derived from `identity.label` and formatted at runtime as
`[<label>] `. Window titles and their formatted form are never logged or
included in any status/audit/metrics surface.

---

## 12. RBAC and broker security

### 12.1 Permission claims

```yaml
permissionClaims:
  resourceVerbs:
    - resourceType: display-wayland.d2b.io.WaylandSession
      verbs: [get, list, watch, create, update-spec, update-status,
              update-finalizers, delete]
    - resourceType: display-wayland.d2b.io.WaylandPolicy
      verbs: [get, list, watch, create, update-spec, update-status]
    - resourceType: Process
      verbs: [get, list, watch, create, update-spec, update-status,
              update-finalizers, delete]
    - resourceType: Volume
      verbs: [get, list, watch, create, update-spec, update-status,
              update-finalizers, delete]
    - resourceType: Device
      verbs: [get, list, watch]
      resourceNames: []   # only Device resources in its owning Guest
    - resourceType: Guest
      verbs: [get, list, watch]
    - resourceType: Host
      verbs: [get, list, watch]
```

No permission claim grants access to Credential resources, Network resources,
or any `update-spec` verb on Host or Guest.

### 12.2 Broker operations

The Host proxy process runs under the `Provider/system-minijail` Process
Provider and requires exactly one privileged broker operation:

- **`SpawnRunner { role: WaylandProxy }`** (current baseline) →
  **`ProviderSupervisor.LaunchTicket`** (v3 target): the broker receives the
  compiled sandbox plan, the pre-opened upstream compositor fd (received via
  ComponentSession attachment from the authenticated user-session/display
  component), and the cross-domain GPU endpoint fd (received via typed
  `device-gpu` ComponentSession handoff). The broker establishes the mount
  namespace, installs both fds at their declared in-jail fd numbers from the
  LaunchTicket, and hands back a verified pidfd. No compositor socket path is
  passed in the LaunchTicket; only the pre-opened fd descriptor.

The upstream compositor fd is established before the LaunchTicket is issued.
It is never associated with a socket path in the LaunchTicket, spec, status,
audit, or logs. Any private listen socket path for the crosvm-facing side of
the cross-domain protocol (if required by the existing wire protocol) resides
only in the Volume mount and the LaunchTicket's sealed fd table. It **never
appears** in:
- the resource spec;
- the resource status;
- any audit record;
- any log line produced by the proxy;
- any OTEL span attribute or metric label.

The proxy binary receives the upstream compositor connection as a pre-opened
fd at a fixed in-jail fd number declared in the LaunchTicket. It receives no
socket path argument for the upstream connection.

### 12.3 Finalizer

```text
finalizer: display-wayland.d2b.io/proxy-stopped
```

The finalizer is installed on `WaylandSession` before any Process is created.
Finalizer handling:
1. Send a graceful stop signal to the proxy Process.
2. Wait for the Process to reach `Succeeded` or `Failed` phase (max 10 s).
3. Delete the proxy Process resource (final `Deleted`-phase revision committed,
   then row and index entries removed atomically).
4. Delete the Volume resource (same atomic deletion sequence).
5. Remove the finalizer from `WaylandSession`.
6. The Zone runtime commits the final `Deleted`-phase revision for
   `WaylandSession`, atomically removes its row and index entries, and commits
   the `display-wayland/session-finalized` audit record after the atomic removal.

If the Process does not stop within the deadline the controller removes the
Process resource unconditionally (the Process Provider enforces SIGKILL) and
proceeds to Volume deletion.

---

## 13. Lifecycle, phase transitions, and error handling

### 13.1 WaylandSession phase transitions

```
Pending → (GpuEndpointAvailable=True, PolicyApplied=True, CrossDomainTrusted=True)
        → (Process created) → (Process Ready) → Ready

Ready → (GPU Device removed or crossDomainTrusted revoked) → Degraded
Degraded → (condition resolved) → Ready

Ready/Degraded → (spec.crossDomainTrusted=false detected at admission) → rejected at UpdateSpec

Ready → (proxy Process Failed repeatedly) → Failed
Failed → (manual re-create or spec change) → Pending
```

### 13.2 Stable error codes

Stable error codes reported in `status.conditions[*].reason`:

| Code | Meaning |
| --- | --- |
| `gpu-endpoint-unavailable` | `Provider/device-gpu` Device is not Ready or does not expose a cross-domain endpoint |
| `cross-domain-not-trusted` | `WaylandSession.spec.crossDomainTrusted` is false; admitted to store but rejected by controller |
| `proxy-process-failed` | Host proxy Process exited with non-zero status |
| `guest-frontend-failed` | Guest frontend Process exited or never started |
| `policy-digest-missing` | `WaylandPolicy` referenced by `policyRef` is not Ready |
| `virgl-video-unsupported` | `virglVideo: true` but GPU Device does not advertise video decode |
| `upstream-unavailable` | Upstream compositor fd was invalid or connection already broken at proxy startup |
| `allow-clipboard-boundary-ignored` | One or more `allowGlobals` entries are clipboard-boundary globals; ignored with advisory |
| `unknown-interface-allowed` | One or more `allowGlobals` entries are unclassified globals |
| `proxy-readiness-timeout` | Proxy did not emit a readiness event within the readiness deadline |
| `no-principal-available` | All pre-provisioned dynamic session pool principals are occupied; no OS user created |

### 13.3 Readiness deadline

The Host proxy Process has a readiness deadline of `30 s` from process start
(emitted in the Process template descriptor). If the proxy does not emit a
`ProxyReadinessEvent` within this window the Process controller sets the
Process phase to `Failed` and sets the `proxy-readiness-timeout` condition on
the `WaylandSession`.

### 13.4 Restart and backoff

The Host proxy Process restart policy follows the standard Process controller
retry/backoff semantics from `ADR-046-resource-reconciliation`. The proxy is
classified as `class: worker` and uses the default binary-exponential backoff
with a cap of 60 s. After 5 consecutive failures within a 5-minute window the
controller sets `WaylandSession.status.phase = Failed` and stops retrying until
the spec or dependent resource changes.

---

## 14. Audit and OTEL telemetry

### 14.1 Audit records

The following security-relevant operations produce authoritative audit records:

| Operation | Audit kind | Fields |
| --- | --- | --- |
| `WaylandSession` created | `display-wayland/session-created` | `zone`, `guestRef`, `hostRef`, `policyRef`, `crossDomainTrusted`, `subject_digest`, `operation_id` |
| `WaylandSession` deleted / finalized | `display-wayland/session-finalized` | `zone`, `guestRef`, `subject_digest`, `operation_id`; record committed after atomic row/index removal |
| Proxy Process started | `display-wayland/proxy-started` | `zone`, `guestRef`, `subject_digest`, `process_revision_digest`, `operation_id` |
| Proxy Process exited | `display-wayland/proxy-exited` | `zone`, `guestRef`, `subject_digest`, `exit_class`, `operation_id` |
| Policy warning produced | `display-wayland/policy-advisory` | `zone`, `guestRef`, `warning_code`, `interface` (capped at 63 chars), `subject_digest`, `operation_id` |
| Clipboard-boundary override ignored | `display-wayland/clipboard-boundary-ignored` | `zone`, `guestRef`, `subject_digest`, `operation_id` |
| `WaylandPolicy` compiled | `display-wayland/policy-compiled` | `zone`, `policyRef`, `policyDigest`, `subject_digest`, `operation_id` |

Audit records must **not** contain:
- compositor socket paths;
- user or session identities beyond `subject_digest`;
- window titles or app-id values;
- clipboard payloads or DnD content;
- raw argv;
- process PIDs, pidfds, or unit names;
- Wayland protocol message bodies.

### 14.2 OTEL metrics

All metric labels use closed, pre-declared label sets. No label value carries
a guest name, Zone name, socket path, interface name, or window title.

| Metric name | Type | Labels | Description |
| --- | --- | --- | --- |
| `d2b_display_wayland_session_total` | counter | `zone_id`, `outcome` | Total WaylandSession create/delete events |
| `d2b_display_wayland_session_ready` | gauge | `zone_id` | Current Ready session count |
| `d2b_display_wayland_proxy_start_total` | counter | `zone_id`, `outcome` | Proxy process start events |
| `d2b_display_wayland_proxy_exit_total` | counter | `zone_id`, `exit_class` | Proxy process exit events |
| `d2b_display_wayland_policy_warning_total` | counter | `zone_id`, `warning_code` | Policy advisory events |
| `d2b_display_wayland_policy_compile_total` | counter | `zone_id`, `outcome` | Policy compile events |

`zone_id` is a stable opaque short ID (not the Zone name string) to avoid
metric cardinality explosion. `outcome` and `exit_class` are closed bounded
enums.

### 14.3 OTEL spans

OTEL spans are emitted for the following controller operations:

- `display_wayland.session.reconcile` — reconcile a single `WaylandSession`;
  attributes: `d2b.zone`, `d2b.provider`, `d2b.component`;
- `display_wayland.proxy.start` — proxy Process spawn ticket issue;
  attributes: `d2b.zone`, `d2b.provider`, `d2b.component`;
- `display_wayland.policy.compile` — policy compilation from `WaylandPolicy`;
  attributes: `d2b.zone`, `d2b.provider`, `d2b.component`.

No span carries socket paths, guest names (beyond the stable Zone/Provider/
component attributes), user identities, window titles, or clipboard content.

### 14.4 OTEL resource attributes

Provider processes emit the following OTEL resource attributes (advisory,
re-stamped at ingress per ADR-046-telemetry-audit-and-support):

```text
service.name = d2b-display-wayland-{host-proxy|guest-frontend|policy}
service.version = <CARGO_PKG_VERSION>
d2b.zone = <zone-name>
d2b.provider = display-wayland
d2b.component = <component-id>
```

Existing advisory attributes (`vm.name`, `vm.env`, `vm.role`) are **not** added
by this Provider's processes. They are stamped by the edge collector from the
Guest's advisory metadata, not by the proxy binary.

---

## 15. Async reconciliation

### 15.1 Controller watch plans

`host-proxy-controller`:
```yaml
watches:
  - resourceType: display-wayland.d2b.io.WaylandSession
    labelSelector: {}
  - resourceType: Process
    labelSelector: {ownerKind: display-wayland.d2b.io.WaylandSession}
  - resourceType: Volume
    labelSelector: {ownerKind: display-wayland.d2b.io.WaylandSession}
  - resourceType: Device
    labelSelector: {purpose: wayland-cross-domain-endpoint}
ownerTriggers:
  - parentType: display-wayland.d2b.io.WaylandSession
    childTypes: [Process, Volume]
```

`guest-frontend-controller`:
```yaml
watches:
  - resourceType: Guest
    labelSelector: {}
  - resourceType: display-wayland.d2b.io.WaylandSession
    labelSelector: {}
  - resourceType: Process
    labelSelector: {ownerKind: display-wayland.d2b.io.WaylandSession}
ownerTriggers:
  - parentType: display-wayland.d2b.io.WaylandSession
    childTypes: [Process]
```

`policy-controller`:
```yaml
watches:
  - resourceType: display-wayland.d2b.io.WaylandPolicy
    labelSelector: {}
```

### 15.2 Reconcile concurrency

Each controller uses a per-resource serialized handler with cross-resource
concurrency (independent resources reconcile in parallel under a semaphore
budget of 8). Independent Guest `WaylandSession` resources reconcile
concurrently. A slow proxy startup does not block sessions for other Guests.

### 15.3 Fast launch target

Per `ADR-046-components-processes-and-sandbox` fast-launch requirements:

- p95 handler start after durable Ready commit: ≤5 ms;
- p95 proxy Process launch attempt start: ≤20 ms;
- proxy Process readiness wait does not block the controller-wide queue.

---

## 16. Nix configuration

### 16.1 Zone resource configuration

```nix
# Artifact catalog entry (derivation-valued input, not inside spec)
d2b.artifacts.display-wayland-provider = {
  package = inputs.d2b-provider-display-wayland.packages.${system}.default;
  type    = "provider";
};

# Provider resource
d2b.zones.dev.resources.display-wayland = {
  type = "Provider";
  spec = {
    artifactId = "display-wayland-provider";
    config = {
      principalPoolSize = 4;  # pre-provisioned pool accounts for dynamic sessions; 1..32
    };
  };
};

# WaylandPolicy (Zone-scoped defaults)
d2b.zones.dev.resources.default-wayland-policy = {
  type = "display-wayland.d2b.io.WaylandPolicy";
  spec = {
    denyGlobals    = [];
    allowGlobals   = [];
    maxVersions    = {};
    dmabufAllow    = [];
    dmabufDeny     = [];
    defaults = {
      acceleratedRendering = "allow";
      clipboardBoundary    = "virtualize";
      highRisk             = "deny";
      appDefaults          = "allow";
      offDefaults          = "deny";
      unclassified         = "deny";
    };
  };
};

# WaylandSession for a VM Guest
d2b.zones.dev.resources.corp-vm-display = {
  type = "display-wayland.d2b.io.WaylandSession";
  spec = {
    guestRef  = "Guest/corp-vm";
    hostRef   = "Host/host-system";
    policyRef = "WaylandPolicy/default-wayland-policy";
    identity = {
      label         = "corp-vm";
      activeColor   = "#7fc8ff";
      inactiveColor = "#45475a";
      urgentColor   = "#f38ba8";
      border = {
        enable    = true;
        railWidth = 9;
      };
      label = {
        enable   = true;
        text     = null;
        position = "top-left";
      };
    };
    crossDomainTrusted = true;
    virglVideo = false;
    filter = {
      debugLogging = false;
      byteLogging  = false;
      denyGlobals  = [];
      allowGlobals = [];
      maxVersions  = {};
      dmabufAllow  = [];
      dmabufDeny   = [];
    };
  };
};
```

**Provider `spec.config` schema:**

| Field | Type | Default | Bounds | Description |
| --- | --- | --- | --- | --- |
| `principalPoolSize` | u32 | `4` | 1..32 | Number of pre-provisioned `d2b-wlproxy-pool-<N>` OS accounts available for dynamic (API-created) `WaylandSession` principals. Validated against the signed Provider config schema at bundle admission. |

Rendered canonical JSON for the `WaylandSession` resource:

```json
{
  "apiVersion": "resources.d2b.io/v3",
  "type": "display-wayland.d2b.io.WaylandSession",
  "metadata": {
    "name": "corp-vm-display",
    "zone": "dev"
  },
  "spec": {
    "guestRef": "Guest/corp-vm",
    "hostRef": "Host/host-system",
    "policyRef": "WaylandPolicy/default-wayland-policy",
    "identity": {
      "label": "corp-vm",
      "activeColor": "#7fc8ff",
      "inactiveColor": "#45475a",
      "urgentColor": "#f38ba8",
      "border": { "enable": true, "railWidth": 9 },
      "label": { "enable": true, "text": null, "position": "top-left" }
    },
    "crossDomainTrusted": true,
    "virglVideo": false,
    "filter": {
      "debugLogging": false,
      "byteLogging": false,
      "denyGlobals": [],
      "allowGlobals": [],
      "maxVersions": {},
      "dmabufAllow": [],
      "dmabufDeny": []
    }
  }
}
```

### 16.2 Color derivation in Nix

The Zone bundle emitter derives colors deterministically when not overridden:

```nix
colorPalette = [
  "#7fc8ff" "#90d090" "#ffb347" "#c8a0e0" "#ff8080"
  "#40e0d0" "#ffd700" "#ff69b4" "#a0c8a0" "#d4a0ff"
  "#ffa07a" "#87ceeb"
];

colorForName = seed: name:
  let
    hashHex = builtins.hashString "sha256" "${seed}-${name}";
    idx     = lib.mod
      (lib.fromHexString (builtins.substring 0 4 hashHex))
      (builtins.length colorPalette);
  in
  builtins.elemAt colorPalette idx;

resolvedActiveColor = name: session:
  if session.spec.identity.activeColor != null
  then session.spec.identity.activeColor
  else colorForName "d2b-niri-border" name;
```

This mirrors the deterministic derivation in `nixos-modules/ui-colors.nix`
(`colorForName "d2b-niri-border"`), preserving color continuity across the
v2 → v3 migration.

### 16.3 Artifact catalog rule

`artifactId = "display-wayland-provider"` must resolve to an artifact with
`type = "provider"` in `d2b.artifacts`. A missing ID or wrong type fails the
NixOS build with an actionable error. The store path of the derivation must
not appear in the rendered ResourceSpec JSON, in status, or in any audit event.

### 16.4 Eval-time validation

At eval time the Nix module enforces:

- `type` must be a known ResourceType in the Zone's locked schema;
- `spec.guestRef` is a valid `Guest/<name>` declared in the Zone;
- `spec.hostRef` is a valid `Host/<name>` declared in the Zone;
- `spec.policyRef` is a valid `WaylandPolicy/<name>` declared in the Zone;
- `spec.identity.activeColor` matches `^#[0-9a-fA-F]{6}$`;
- `spec.crossDomainTrusted = true` (false is a Nix eval error);
- `spec.filter.allowGlobals` does not contain clipboard-boundary globals
  (an eval-time warning is emitted; the entries are accepted but a
  `W-ALLOW-CLIPBOARD-BOUNDARY` advisory will be produced at runtime);
- resource name matches `^[a-z][a-z0-9-]*$`;
- the number of `WaylandSession` resources per Host must not exceed the total
  provisioned principal count for that Host (bundle-declared sessions +
  `spec.config.principalPoolSize`); excess sessions are a Nix eval error.

### 16.5 Principal provisioning

Each `WaylandSession` process runs under a dedicated per-Guest system principal
(`d2b-<guest-name>-wlproxy`). These OS accounts are provisioned by the NixOS
module, not created on demand at runtime.

#### Bundle-declared sessions (Nix-configured)

For every `WaylandSession` declared under `d2b.zones.<zone>.resources.*` in
the NixOS configuration, the Zone bundle emitter declares the corresponding
`d2b-<guest-name>-wlproxy` user and group in the NixOS system configuration
with `isSystemUser = true` and no explicit UID or GID:

```nix
# Auto-generated by the Zone bundle emitter (do not write by hand):
users.users."d2b-corp-vm-wlproxy" = {
  isSystemUser = true;
  group        = "d2b-corp-vm-wlproxy";
  description  = "d2b display-wayland proxy principal for corp-vm";
};
users.groups."d2b-corp-vm-wlproxy" = {};
```

The account is created at `nixos-rebuild switch` time, before any runtime
activity. Account names are derived solely from the authenticated Guest
resource name and are bounded to `^d2b-[a-z][a-z0-9-]*-wlproxy$` (max 63
chars). The `host-proxy-controller` and the broker verify account existence
via NSS at spawn time; neither creates OS accounts.

#### Dynamic API sessions (runtime-created)

For `WaylandSession` resources created at runtime via the Zone API (not
declared in the Nix bundle), the controller draws principals from a bounded
pool of pre-provisioned accounts. Pool accounts are named
`d2b-wlproxy-pool-<N>` (zero-padded index, N < `principalPoolSize`) and are
provisioned by the same Nix emitter. The pool size is set in
`Provider.spec.config` (§16.1):

```nix
d2b.zones.dev.resources.display-wayland.spec.config.principalPoolSize = 4;
# default: 4; bounds: 1..32; validated against signed Provider config schema
```

The Zone bundle emitter translates this into NixOS `users.users.*` declarations
with `isSystemUser = true` and no explicit UID or GID; the OS allocates UIDs
in the system range automatically:

```nix
# Auto-generated by the Zone bundle emitter (do not write by hand):
users.users."d2b-wlproxy-pool-0" = {
  isSystemUser = true;
  group        = "d2b-wlproxy-pool-0";
  description  = "d2b display-wayland dynamic session pool slot 0";
};
users.groups."d2b-wlproxy-pool-0" = {};
# ... repeated for each N in 0..(principalPoolSize - 1)
```

No raw UID or GID integer is set or required in the Nix declaration. The
broker resolves the principal by name through NSS at spawn time.

The controller maintains a lease table mapping pool slots to active dynamic
sessions. If all pool slots are occupied when a new dynamic session is
requested, the session is admitted to the store but immediately transitions to
`Failed` with condition `NoPrincipalAvailable`. There is no silent OS user
creation; no fallback to a shared or root account; no retry until a slot
becomes free.

#### Total concurrent session bound

The total number of concurrent display sessions on a given Host is bounded by:

```
(bundle-declared WaylandSession count) + (spec.config.principalPoolSize)
```

Nix eval enforces that the sum of declared `WaylandSession` resources per Host
does not exceed the provisioned principal count. Exceeding this limit is a Nix
eval error with an actionable message.

---

## 17. Current-code baseline mapping

### 17.1 Terminology

| Baseline name | v3 target |
| --- | --- |
| `ProcessRole::WaylandProxy` | `Process` resource template `wayland-proxy-worker` owned by `Provider/display-wayland` |
| `WaylandProxyArgvInput` / `generate_wayland_proxy_argv` | sealed Process template config; argv generation is internal to Provider template; never public API |
| `LocalCrossDomainWaylandProvider` | `display-wayland` controller component; current struct becomes controller reconcile logic |
| `RuntimeDisplayCapabilities.wayland_proxy: bool` | `WaylandSession` resource presence; `wayland_proxy: false` maps to no `WaylandSession` resource |
| `RuntimeDisplayCapabilities.graphics: bool` | `spec.providerSettings.displayWayland.enable` on the Guest resource |
| `graphics.crossDomainTrusted` (Nix) | `WaylandSession.spec.crossDomainTrusted = true` |
| `graphics.virglVideo` (Nix) | `WaylandSession.spec.virglVideo` |
| `graphics.waylandProxy.enable` (Nix) | presence of a `WaylandSession` resource for the Guest |
| `graphics.waylandProxy.border.*` (Nix) | `WaylandSession.spec.identity.border.*` |
| `graphics.waylandProxy.denyGlobals` (Nix) | `WaylandSession.spec.filter.denyGlobals` |
| `graphics.waylandProxy.allowGlobals` (Nix) | `WaylandSession.spec.filter.allowGlobals` |
| `graphics.waylandProxy.maxVersions` (Nix) | `WaylandSession.spec.filter.maxVersions` |
| `graphics.waylandProxy.dmabufAllow` / `dmabufDeny` (Nix) | `WaylandSession.spec.filter.dmabufAllow` / `dmabufDeny` |
| `graphics.niriBorderColor` (Nix, deprecated) | feeds `WaylandSession.spec.identity.activeColor` via bundle emitter color resolution |
| `WaylandProxyBorderConfig.active_color` / `inactive_color` / `urgent_color` | `WaylandSession.spec.identity.activeColor` / `inactiveColor` / `urgentColor` |
| `FilterPolicy` / `PolicyInput` / `GlobalAction` / `Classification` | internal to `d2b-provider-display-wayland/src/`; no longer a public type |
| `ProxyReadinessEvent` / `ProxyReadinessStage` | internal Process readiness fd protocol; scope unchanged |
| `BridgeConfig` / `BridgeReconnectPolicy` | internal clipboard bridge config; scoped to Provider crate |
| `wl-cross-domain-proxy` binary (`pkgs/wl-cross-domain-proxy`) | Process template `wayland-frontend-worker`; binary is a component in the Provider package at `packages/d2b-provider-display-wayland/src/bin/wl-cross-domain-proxy.rs`; included in the guest system NixOS closure via the Provider package |
| `d2b-wayland-proxy` binary (`packages/d2b-wayland-proxy/`) | `d2b-display-wayland-host-proxy` binary in new Provider crate |
| `nixos-modules/ui-colors.nix` color resolution | Zone bundle emitter color resolution for `WaylandSession.spec.identity.activeColor` (§16.2) |

### 17.2 Evidence class mapping

| Baseline symbol | Evidence class | Current location |
| --- | --- | --- |
| `ProcessRole::WaylandProxy` | `production-reachable` | `packages/d2b-core/src/processes.rs:247` |
| `generate_wayland_proxy_argv` | `production-reachable` | `packages/d2b-host/src/wayland_proxy_argv.rs` |
| `WaylandProxyArgvInput::for_vm` | `production-reachable` | `packages/d2b-host/src/wayland_proxy_argv.rs` |
| `LocalCrossDomainWaylandProvider` | `dead-reachable` | `packages/d2b-host-providers/src/lib.rs` |
| `d2b-wayland-proxy` main binary | `production-reachable` | `packages/d2b-wayland-proxy/src/main.rs` |
| `FilterPolicy` / `PolicyInput` | `production-reachable` | `packages/d2b-wayland-proxy/src/policy.rs` |
| `DecorationManager` / `BorderConfig` | `production-reachable` | `packages/d2b-wayland-proxy/src/decoration.rs` |
| `BridgeConfig` | `production-reachable` | `packages/d2b-wayland-proxy/src/bridge.rs` |
| `ProxyReadinessEvent` | `production-reachable` | `packages/d2b-wayland-proxy/src/readiness.rs` |
| `ProxyIdentity` | `production-reachable` | `packages/d2b-wayland-proxy/src/identity.rs` |
| `ClipboardGlobalDisposition` | `production-reachable` | `packages/d2b-wayland-proxy/src/clipboard.rs` |
| `graphics.nix` options | `nix-emitted` | `nixos-modules/components/graphics.nix` |
| `ui-colors.nix` color resolution | `nix-emitted` | `nixos-modules/ui-colors.nix` |
| `RuntimeDisplayCapabilities.wayland_proxy` | `production-reachable` | `packages/d2b-core/src/runtime.rs:278` |
| `RuntimeDisplayCapabilities.graphics` | `production-reachable` | `packages/d2b-core/src/runtime.rs:194` |
| conformance: `display_fails_closed_when_unsupported` | `test-only` | `packages/d2b-realm-provider/src/conformance.rs:24` |
| `MockWorkloadProvider` display fields | `test-only` | `packages/d2b-realm-provider/src/mock.rs` |

### 17.3 Guest frontend binary

`pkgs/wl-cross-domain-proxy` is the current guest-side binary. In v3 it
becomes the `wayland-frontend-worker` Process template binary. The binary is
a component in the single `d2b-provider-display-wayland` package at
`packages/d2b-provider-display-wayland/src/bin/wl-cross-domain-proxy.rs`. It
is built as part of the Provider package and included in the guest system
NixOS closure via the Provider artifact. The `pkgs/wl-cross-domain-proxy`
standalone derivation is superseded and removed after the Provider package
produces the guest binary (see §19 removal table).

---

## 18. Implementation work items

### ADR046-display-001

| Field | Value |
| --- | --- |
| Dependency/owner | `ADR046-provider-001`, `ADR046-process-001`; display Provider owner |
| Current source | `packages/d2b-wayland-proxy/`, `packages/d2b-host/src/wayland_proxy_argv.rs`, `packages/d2b-host-providers/src/lib.rs`, `packages/d2b-realm-provider/src/{conformance,mock}.rs` |
| Reuse action | extract and adapt |
| Destination | `packages/d2b-provider-display-wayland/src/` |
| Detailed design | Create Provider crate layout (`src/`, `tests/`, `integration/`, `README.md`); extract `FilterPolicy`, `PolicyInput`, `DecorationManager`, `BridgeConfig`, `ProxyReadinessEvent`, `ProxyIdentity`, `ClipboardGlobalDisposition` from `d2b-wayland-proxy`; implement `host-proxy-controller` and `policy-controller` using toolkit `ResourceClient`/`Reconciler`; implement pool-slot acquisition logic that fails closed with `NoPrincipalAvailable` when all pre-provisioned dynamic session principals are occupied; implement `wl-cross-domain-proxy` guest frontend binary at `src/bin/wl-cross-domain-proxy.rs` within the Provider package; implement provider-neutral `display_fails_closed_when_unsupported` conformance |
| Integration | Zone Provider resource/catalog → `WaylandSession` controller → Process resources → supervisor ticket |
| Data migration | Full reset; no v2 session compatibility |
| Validation | conformance vectors, fake-bus tests, filter policy golden tests (migrate from `packages/d2b-wayland-proxy/`), redaction/audit contract tests, no-fallback test |
| Removal proof | `packages/d2b-host-providers/src/lib.rs` `LocalCrossDomainWaylandProvider` removed only after `host-proxy-controller` passes conformance; `packages/d2b-host/src/wayland_proxy_argv.rs` removed only after Process template sealing verified |

### ADR046-display-002

| Field | Value |
| --- | --- |
| Dependency/owner | `ADR046-display-001`; Nix integrator |
| Current source | `nixos-modules/components/graphics.nix` `graphics.waylandProxy.*`; `nixos-modules/ui-colors.nix` VM border color resolution; `nixos-modules/options-vms.nix` `graphics.*` options |
| Reuse action | adapt |
| Destination | Zone bundle emitter for `WaylandSession` / `WaylandPolicy` ResourceSpecs under `d2b.zones.<zone>.resources.*`; `WaylandSession` color resolution in Nix bundle emitter |
| Detailed design | Emit `WaylandSession` and `WaylandPolicy` ResourceSpecs from Nix; derive colors from `d2b-niri-border` palette (§16.2); enforce `crossDomainTrusted = true` at eval time; emit v3 `display-wayland-provider` artifact catalog entry with `spec.config.principalPoolSize` (default 4, bounds 1..32); provision `d2b-<guest>-wlproxy` system user/group (`isSystemUser = true`, no raw UID/GID) for every bundle-declared `WaylandSession`; provision `d2b-wlproxy-pool-<N>` accounts (same convention) up to `spec.config.principalPoolSize` per Host; validate signed Provider config schema against `principalPoolSize`; enforce eval-time bound that bundle session count + pool size does not exceed provisioned principal count; nix-unit tests for color derivation, spec shape, JSON round-trip, and principal provisioning |
| Integration | Zone NixOS module system → bundle emitter → `/etc/d2b/zones/<zone>/bundle/generation-N.json` |
| Data migration | Legacy `graphics.waylandProxy.*` Nix options accepted with deprecation warning during migration window; removed after parity |
| Validation | nix-unit spec-shape tests, eval-time guard tests (crossDomainTrusted=false rejected), color derivation golden tests, principal provisioning count bound test |
| Removal proof | `nixos-modules/components/graphics.nix` `graphics.waylandProxy.*` and `nixos-modules/ui-colors.nix` VM color resolution removed only after Zone bundle emitter parity verified by a full nix-unit pass |

### ADR046-display-003

| Field | Value |
| --- | --- |
| Dependency/owner | `ADR046-display-001`; telemetry/audit owner |
| Current source | `packages/d2b-wayland-proxy/src/diag.rs` (rate-limited bounded diagnostics) |
| Reuse action | extract and adapt |
| Destination | `packages/d2b-provider-display-wayland/src/audit.rs`, `packages/d2b-provider-display-wayland/src/metrics.rs` |
| Detailed design | Implement audit record types for all events in §14.1; implement OTEL metric counters/gauges in §14.2; adapt `DiagRateLimiter` to use closed label sets; validate that no socket path, user identity, window title, or app-id appears in any log/audit/metric surface |
| Integration | Providers emit via Zone telemetry emitter; audit records committed before operation completion |
| Data migration | None |
| Validation | Redaction contract tests (`policy_observability.rs` pattern), audit record schema tests, label-cardinality tests |
| Removal proof | N/A (new code) |

### ADR046-display-004

| Field | Value |
| --- | --- |
| Dependency/owner | `ADR046-display-001`; integration test owner |
| Current source | `tests/integration/` test orchestration structure |
| Reuse action | new |
| Destination | `packages/d2b-provider-display-wayland/integration/` |
| Detailed design | Container/Host/Guest/cross-process integration fixtures for: (a) end-to-end WaylandSession create → proxy Process ready → guest frontend ready; (b) GPU endpoint unavailable → Pending; (c) proxy crash → Failed backoff; (d) policy policy warning production; (e) clipboard boundary denial; (f) crossDomainTrusted=false admission rejection. Follows `ADR-046-provider-model-and-packaging` integration/ convention. |
| Integration | Invoked by existing repository test orchestration (`make test-integration` / container lane) |
| Data migration | None |
| Validation | All six scenarios above pass; no socket paths in test output |
| Removal proof | N/A (new code) |

---

## 19. Removal proof requirements

Each removal listed below has a named prerequisite. Premature removal is a
blocking defect.

| Item to remove | Location | Prerequisite |
| --- | --- | --- |
| `ProcessRole::WaylandProxy` | `packages/d2b-core/src/processes.rs` | `ADR046-display-001` passes conformance and Process template verified |
| `generate_wayland_proxy_argv` / `WaylandProxyArgvInput` | `packages/d2b-host/src/wayland_proxy_argv.rs` | Process template sealing verified; no other callers remain |
| `LocalCrossDomainWaylandProvider` | `packages/d2b-host-providers/src/lib.rs` | `host-proxy-controller` passes conformance |
| `packages/d2b-wayland-proxy/` crate | whole directory | `ADR046-display-001` complete; all behavior migrated to Provider crate; all tests passing in new location |
| `nixos-modules/components/graphics.nix` `graphics.waylandProxy.*` options | `nixos-modules/components/graphics.nix` | `ADR046-display-002` Zone bundle emitter parity verified |
| `nixos-modules/ui-colors.nix` VM border color resolution | `nixos-modules/ui-colors.nix` | `ADR046-display-002` color derivation in Zone bundle emitter verified by nix-unit pass |
| `RuntimeDisplayCapabilities.wayland_proxy` | `packages/d2b-core/src/runtime.rs` | `WaylandSession` resource presence is the v3 equivalent; all callers migrated |
| `conformance.rs` `display_fails_closed_when_unsupported` | `packages/d2b-realm-provider/src/conformance.rs` | Equivalent conformance test added to Provider crate `tests/` |
| `pkgs/wl-cross-domain-proxy` | `pkgs/` | Guest frontend binary placement resolved per `ADR046-display-001` and guest system closure verified |

---

## 20. Tests

### 20.1 Unit tests (`packages/d2b-provider-display-wayland/tests/`)

| Test | Coverage |
| --- | --- |
| `filter_policy_default_shape` | Required-baseline globals always allowed; clipboard-boundary globals virtualized; high-risk globals denied |
| `filter_policy_layer_override_order` | Zone policy overrides defaults; session filter overrides Zone policy; required-baseline cannot be denied |
| `filter_policy_unknown_global_denied` | Unknown globals denied unless `allowGlobals` explicitly lists them |
| `filter_policy_clipboard_boundary_ignored` | `allowGlobals` entries for clipboard-boundary globals ignored with `W-ALLOW-CLIPBOARD-BOUNDARY` advisory |
| `filter_policy_allow_unclassified_produces_advisory` | `allowGlobals` for unclassified global produces `W-ALLOW-UNCLASSIFIED` advisory |
| `proxy_argv_no_socket_paths` | Process template argv/config serialization contains no socket path strings |
| `session_status_no_sensitive_fields` | `WaylandSession` status JSON contains no socket path, user identity, window title, or app-id |
| `decoration_color_round_trip` | `activeColor`/`inactiveColor`/`urgentColor` hex strings survive Nix eval → spec → sealed config → proxy config round-trip |
| `decoration_label_max_chars` | Labels exceeding 64 chars are truncated; `""` suppresses label text |
| `cross_domain_trusted_false_rejected` | `WaylandSession.spec.crossDomainTrusted = false` is rejected at admission |
| `virgl_video_unsupported_condition` | `virglVideo: true` with GPU Device not advertising video decode sets `VirglVideoUnsupported` condition |
| `conformance_display_fails_closed_when_unsupported` | Provider without `window-forwarding` capability returns `CapabilityDenied`; no fallback |
| `audit_record_no_paths` | All audit record types contain no socket paths, user identities, or window titles |
| `metric_labels_closed` | All metric label values are members of the closed pre-declared label sets |
| `readiness_event_bounded` | `ProxyReadinessEvent` serialization contains no socket paths |
| `principal_pool_exhausted_fails_closed` | All pool slots occupied → new dynamic session transitions to `Failed` with `NoPrincipalAvailable`; no OS user creation attempted |

### 20.2 Hermetic integration tests (`packages/d2b-provider-display-wayland/tests/`)

| Test | Coverage |
| --- | --- |
| `controller_session_create_to_ready` | Fake-bus: `WaylandSession` created → `host-proxy-controller` creates Process and Volume → fake Process transitions to Ready → session status transitions to Ready |
| `controller_gpu_endpoint_unavailable` | Fake-bus: GPU Device not Ready → session stays Pending → `GpuEndpointAvailable=False` condition |
| `controller_proxy_failed_backoff` | Fake-bus: proxy Process exits → retry policy applied → after 5 failures within window session transitions to Failed |
| `controller_finalize` | Fake-bus: `WaylandSession` deletionRequestedAt set → finalizer runs → Process deleted → Volume deleted → finalizer removed |
| `controller_policy_update_triggers_reconcile` | Policy controller compiles `WaylandPolicy` → owner trigger dispatched to `host-proxy-controller` → session reconciled with new policy digest |
| `controller_clipboard_bridge_disabled_without_clipboard_provider` | `clipboard-wayland` Provider absent → `ClipboardBridgeReady=False` condition; session proceeds without clipboard bridge |
| `controller_no_principal_available` | Fake-bus: all pool slots occupied → new dynamic `WaylandSession` reconcile → `NoPrincipalAvailable` condition set → session `Failed`; no spawn attempted |

### 20.3 Container/cross-process integration (`packages/d2b-provider-display-wayland/integration/`)

| Scenario | Coverage |
| --- | --- |
| `e2e_session_create_ready` | Real Zone runtime + pre-opened upstream compositor fd delivered via ComponentSession attachment: `WaylandSession` create → proxy starts → readiness event → session Ready |
| `e2e_gpu_endpoint_missing` | No GPU Device registered → session Pending for configured timeout |
| `e2e_proxy_crash_recovery` | Proxy crashes → backoff → re-launch → session recovers to Ready |
| `e2e_policy_warning_audit` | `allowGlobals` contains unclassified global → `W-ALLOW-UNCLASSIFIED` audit advisory emitted; no hard failure |
| `e2e_clipboard_boundary_denial` | `allowGlobals` contains `wl_data_device_manager` → ignored → `W-ALLOW-CLIPBOARD-BOUNDARY` advisory emitted; clipboard not forwarded |
| `e2e_cross_domain_trusted_false_rejected` | Nix eval with `crossDomainTrusted = false` → build fails with actionable error |

---

## 21. Required crate paths

Workspace policy rejects the Provider crate if any of the following paths
are absent:

```text
packages/d2b-provider-display-wayland/src/
packages/d2b-provider-display-wayland/tests/
packages/d2b-provider-display-wayland/integration/
packages/d2b-provider-display-wayland/README.md
```

---

## 22. `README.md` requirements

`packages/d2b-provider-display-wayland/README.md` must cover, at minimum:

- Provider identity / `providerRef` / `artifactId`;
- ResourceTypes implemented: `WaylandSession`, `WaylandPolicy`;
- Controllers: `host-proxy-controller`, `guest-frontend-controller`,
  `policy-controller`;
- Services: none (all communication via d2b-bus ComponentSession);
- Worker Processes: `wayland-proxy-worker`, `wayland-frontend-worker`;
- Binaries: `d2b-display-wayland-host-proxy`, `d2b-display-wayland-guest-frontend`,
  `d2b-display-wayland-policy`;
- Component placement: one `host-proxy-controller` per Zone Host; one
  `guest-frontend-controller` per enabled Guest; one `policy-controller` per Zone;
- Dependencies: `Provider/device-gpu` (optional, required for Ready sessions);
  `Provider/clipboard-wayland` (optional, clipboard bridge);
  `Provider/runtime-cloud-hypervisor` (mandatory for VMM cross-domain context);
- RBAC / permission claims (§12.1);
- Security posture: no direct compositor fallback, no capabilities, mandatory
  seccomp, mount namespace, per-session Volume, no socket paths in
  status/logs/audit;
- Principal provisioning: `d2b-<guest>-wlproxy` accounts provisioned by Nix
  at `nixos-rebuild switch` for bundle-declared sessions; dynamic sessions use
  the pre-provisioned `d2b-wlproxy-pool-<N>` account pool; no OS user creation
  at runtime; pool exhaustion fails closed with `NoPrincipalAvailable` (§16.5);
- State / Volume use: ephemeral tmpfs proxy runtime Volume per session (§7.1); no durable Volume for policy state (policy is deterministically rebuilt from WaylandPolicy spec on controller restart);
- Telemetry: closed metric labels, no socket paths, no user identities, no
  window titles in any observability surface;
- Build: `cargo build -p d2b-provider-display-wayland`;
- Test: `cargo test -p d2b-provider-display-wayland`;
- Integration: instructions for running `integration/` fixtures via the
  repository test orchestration;
- Standalone consumption notes (how to use this crate outside this repository).
