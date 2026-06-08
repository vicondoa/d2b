# Reference: broker privileged operation matrix

> Diataxis: reference. Stable catalog of every closed-enum operation
> the privileged broker (`nixling-priv-broker`) is allowed to perform
> on behalf of the unprivileged `nixlingd` daemon. The wire-level
> source of truth is `nixling_ipc::BrokerRequest`; this page is the
> human-readable index keyed by operation name.

Every row carries three policy flags:

- **audit** — yes for every W3 operation; the broker writes one
  append-only JSON record per decision to
  `/var/lib/nixling/audit/broker-<utc-date>.jsonl`.
- **destructive** — `yes` for any operation whose audit decision can
  mutate persistent host state. Pure-`open` device handoffs are `no`
  because the broker only opens; the daemon owns the resulting fd.
- **secret** — `yes` for operations whose audit record may reference
  secret-material identifiers. W3 has no secret-bearing variants.

Unknown variants and unknown fields in security-sensitive artifacts
are denied (`defaultForUnknown: deny`).

## W3 operation catalog (PROTOCOL_VERSION = 2)

The W3 wave-delivered subset of `BrokerRequest`. Every row carries
`audit: yes` and `defaultForUnknown: deny`.

| Variant | Subject | Scope | Wave first delivered | Destructive | Secret access | Allowed groups | Audit | Default-for-unknown | Audit fields (in addition to common header) | Owner ADR |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `DelegateCgroupV2` | cgroup | global | W3 | no (chown only) | no | `nixling-admin` | yes | deny | `slice_path`, `controllers_enabled`, `owner_uid` | [0011](../adr/0011-cgroup-v2-delegation-and-pidfd-handoff.md) |
| `OpenCgroupDir` | cgroup | per VM / role | W3 | no | no | `nixling-launcher` + `nixling-admin` | yes | deny | `cgroup_id`, `path_class` | [0011](../adr/0011-cgroup-v2-delegation-and-pidfd-handoff.md) |
| `PrepareStateDir` | fs | global / per VM | W3 | yes (mkdir/chown/chmod) | no | `nixling-admin` | yes | deny | `base_dir_hash`, `vm_id_or_scope`, `created_paths_hash`, `mode`, `owner_uid`, `owner_gid`, `replace_or_create_result` | [0012](../adr/0012-w3-ipv6-off-sysctl-set-and-hash-ifname.md) |
| `PrepareRuntimeDir` | fs (`/run/nixling`) | global / per VM | W3 | yes (mkdir/chown/chmod) | no | `nixling-admin` | yes | deny | `base_dir_hash`, `vm_id_or_scope`, `created_paths_hash`, `mode`, `owner_uid`, `owner_gid`, `replace_or_create_result` | [0012](../adr/0012-w3-ipv6-off-sysctl-set-and-hash-ifname.md) |
| `OpenKvm` | device | per role | W3 | no | no | `nixling-launcher` | yes | deny | `device_class`, `role_id` | [0014](../adr/0014-w3-modules-devices-runner-shape.md) |
| `OpenVhostNet` | device | per role | W3 | no | no | `nixling-launcher` | yes | deny | `device_class`, `role_id` | [0014](../adr/0014-w3-modules-devices-runner-shape.md) |
| `OpenFuse` | device | per role | W3 | no | no | `nixling-launcher` | yes | deny | `device_class`, `role_id` | [0014](../adr/0014-w3-modules-devices-runner-shape.md) |
| `OpenDevice` | device | per role | W3 | no | no | `nixling-launcher` | yes | deny | `device_class`, `role_id` | [0014](../adr/0014-w3-modules-devices-runner-shape.md) |
| `CreateTapFd` | network | per env / VM / TAP | W3 | possible (link create/destroy) | no | `nixling-admin` | yes | deny | `ifname_derived`, `role`, `flags_after`, `flags_diff` | [0012](../adr/0012-w3-ipv6-off-sysctl-set-and-hash-ifname.md) |
| `CreatePersistentTap` | network | per env / VM / TAP | W3 | possible (link create/destroy) | no | `nixling-admin` | yes | deny | `ifname_derived`, `role`, `flags_after`, `flags_diff` | [0012](../adr/0012-w3-ipv6-off-sysctl-set-and-hash-ifname.md) |
| `SetBridgePortFlags` | network | per env / VM / TAP | W3 | possible (flag flip) | no | `nixling-admin` | yes | deny | `ifname_derived`, `role`, `flags_after`, `flags_diff` | [0012](../adr/0012-w3-ipv6-off-sysctl-set-and-hash-ifname.md) |
| `ApplyNftables` | network host | global / per env | W3 | yes | no | `nixling-admin` | yes | deny | `table_hash_before`, `table_hash_after`, `coexistence_policy`, `manager_detected` | [0013](../adr/0013-w3-firewall-coexistence-policy.md) |
| `ApplyRoute` | routing | global / per env | W3 | yes | no | `nixling-admin` | yes | deny | `route_key`, `route_diff` | [0012](../adr/0012-w3-ipv6-off-sysctl-set-and-hash-ifname.md) |
| `ApplySysctl` | sysctl | per link / global | W3 | yes | no | `nixling-admin` | yes | deny | `sysctl_key`, `value_before`, `value_after` | [0012](../adr/0012-w3-ipv6-off-sysctl-set-and-hash-ifname.md) |
| `ApplyNmUnmanaged` | name resolution / NM | per ifname | W3 | yes | no | `nixling-admin` | yes | deny | `nm_file_path_hash`, `ifname_set` | [0012](../adr/0012-w3-ipv6-off-sysctl-set-and-hash-ifname.md) |
| `UpdateHostsFile` | name resolution | global | W3 | yes | no | `nixling-admin` | yes | deny | `managed_block_hash_before`, `managed_block_hash_after` | [0012](../adr/0012-w3-ipv6-off-sysctl-set-and-hash-ifname.md) |
| `BindUnixSocket` | socket | per VM / role | W3 | partial (replace stale only) | no | `nixling-admin` | yes | deny | `socket_path_hash`, `mode`, `acl_diff` | [0011](../adr/0011-cgroup-v2-delegation-and-pidfd-handoff.md) |
| `SetSocketAcl` | socket | per VM / role | W3 | partial (replace stale only) | no | `nixling-admin` | yes | deny | `socket_path_hash`, `mode`, `acl_diff` | [0011](../adr/0011-cgroup-v2-delegation-and-pidfd-handoff.md) |
| `ModprobeIfAllowed` | kernel module | global / feature | W3 | yes | no | `nixling-admin` | yes | deny | `module_name`, `matrix_entry_id`, `modules_disabled_sysctl` | [0014](../adr/0014-w3-modules-devices-runner-shape.md) |
| `UsbipBindFirewallRule` | USBIP firewall | per busid | W3 (skeleton only) | no (rule add only) | no | `nixling-admin` | yes | deny | `busid`, `rule_hash` | [0013](../adr/0013-w3-firewall-coexistence-policy.md) |

## W2-delivered variants (still callable in W3)

| Variant | Subject | Scope | Wave | Destructive | Secret | Allowed groups | Audit | Default-for-unknown |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `ValidateBundle` | bundle | global | W2 | no | no | `nixling-launcher` + `nixling-admin` | yes | deny |
| `ExportBrokerAudit` | audit log | global | W2 | no (read-only export) | no | `nixling-admin` | yes | deny |
| `CreateOrReconcileUsersGroups` | user/group | global | W2 (partial — bootstrap only) | yes | no | `nixling-admin` | yes | deny |

## W7/W14-delivered lifecycle variants

| Variant | Subject | Scope | Wave first delivered | Destructive | Secret access | Allowed groups | Audit | Default-for-unknown | Audit fields (in addition to common header) |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `PrepareStoreView` | fs (store view) | per VM | W7 | yes | no | `nixling-launcher` + `nixling-admin` | yes | deny | `generation`, `hardlink_farm_path`, `target_view_path` |
| `StoreSync` | fs (hardlink farm) | per VM | P2 | yes (atomic `current` symlink swap) | no | `nixling-launcher` + `nixling-admin` | yes | deny | `bundle_closure_ref`, `generation`, `closure_count`, `hardlink_farm_path` |
| `SetupMountNamespace` | mount ns | per VM / role | W7 | partial (mount-root prep + bind target) | no | `nixling-launcher` + `nixling-admin` | yes | deny | `role_id`, `mount_root`, `mount_view_path`, `source_view_path` |

## Variants reserved on the wire but deferred (W3 returns `unknown-operation`)

These are present in the `BrokerRequest` enum so the wire protocol
stays stable across waves, but the W3 broker dispatches them to a
`unknown-operation` refusal with an audit record. Each variant lists
the wave that delivers a real handler.

| Variant | Subject | Deferred until | Destructive | Secret access | Why deferred |
| --- | --- | --- | --- | --- | --- |
| `LaunchMinijailChild` | process | W4 | yes (fork+exec) | no | minijail provisioning lands with the W4 mount-namespace + runner work. |
| `UsbipBind` | USBIP device routing | W6 | yes | no | live USBIP attach surface is W6; W3 ships only the firewall skeleton. |
| `UsbipUnbind` | USBIP device routing | W6 | yes | no | live USBIP detach surface is W6. |
| `UsbipProxyReconcile` | USBIP device routing | W6 | yes | no | the USBIP proxy DAG reconcile is W6. |
| `ReadSecretById` | secret store | W8 | no (read-only) | yes | secret backend (the only `secret: yes` family) is W8. |
| `InjectSecretById` | secret store | W8 | yes | yes | secret injection into VM payloads is W8. |
| `RotateSecretById` | secret store | W8 | yes | yes | secret rotation is W8. |
| `PauseBroker` | broker admin | W8 admin | partial (state transition) | no | broker-pause admin verb lands with W8 admin tooling. |
| `ResumeBroker` | broker admin | W8 admin | partial (state transition) | no | broker-resume admin verb lands with W8 admin tooling. |

The wire goldens under `tests/golden/broker-wire/` cover one canonical
encoding per deferred variant so W4/W6/W8 do not have to break wire
compatibility when their handlers ship. The canonical machine-
readable source for this catalog is the JSON schema under
[`docs/reference/schemas/v2/privileges.json`](schemas/v2/privileges.json)
(W3fu1 H3 bumped `schemaVersion` to `v2` and W3fu2 H6 corrected the
operation enum). The `v1` schema remains in tree as the frozen W2
baseline; consumers should validate against `v2`. The markdown above
is the human-readable index.

## Audit record schema (W3 baseline)

Every decision is one JSON object emitted via a pre-opened `O_APPEND`
fd. Common header:

```
{
  "ts": <iso-8601 utc>,
  "broker_version": <string>,
  "bundle_version": <"v2" | "v3" | "v4">,
  "bundle_hash": <"fnv1a64:...">,
  "operation": <enum-tag>,
  "public_operation_id": <stable-id>,
  "peer_uid": <integer>,
  "peer_gid": <integer>,
  "authz_result": "launcher" | "admin" | "deny",
  "subject_id": <opaque>,
  "scope_id": <opaque>,
  "decision": "allowed" | "denied-refused" | "denied-unknown" | "errored",
  "error_kind": <kebab-case or null>,
  "tracing_span_id": <opaque>,
  "operation_fields": { ... per-variant per the table above ... }
}
```

The shared broker `OperationFields` enum now serializes the live
non-bootstrap dispatch surface as typed per-op payloads:

- `Hello { client_version }`
- `ValidateBundle {}`
- `ExportBrokerAudit { since, filter }`
- `ApplyNftables { bundle_nft_intent_ref, scope_id, desired_hash, destroy }`
- `ApplyRoute { bundle_route_intent_ref, destination, via, destroy }`
- `ApplyNmUnmanaged { bundle_nm_intent_ref, scope_id, destroy }`
- `ApplySysctl { bundle_sysctl_intent_ref, key, destroy }`
- `UpdateHostsFile { bundle_hosts_intent_ref, destroy }`
- `OpenPidfd { pid, expected_start_time_ticks }`
- `SignalRunner { vm_id, role_id, signal }`
- `SpawnRunner { bundle_runner_intent_ref, vm_id, role_id, role, runtime_allocations }`
- `RunHostInstall { bundle_installer_intent_ref, enable, start, no_start }`
- `RunMigrate { bundle_migrate_intent_ref }`
- `RunActivation { bundle_activation_intent_ref, mode, vm }`
- `RunGc { bundle_gc_intent_ref, keep_generations }`
- `RunKeysRotate { bundle_keys_intent_ref, vm }`
- `RunHostKeyTrust { bundle_trust_intent_ref, vm }`
- `RunRotateKnownHost { bundle_rotate_known_host_intent_ref, vm }`
- `UsbipBind { bus_id, vm }`
- `UsbipUnbind { bus_id }`
- `UsbipProxyReconcile {}`
- `UsbipBindFirewallRule { bundle_usbip_firewall_intent_ref }`

### Runner roles (selected via `SpawnRunner.role`)

Each entry is a `RunnerRole` enum variant the broker dispatches to a
pure argv generator in `nixling_host`. Per-role minijail profile +
seccomp policy are listed below; full cap matrix is the
plan kernel-r2-4 table.

| Runner role | Replaces | Caps | Notes |
| --- | --- | --- | --- |
| `OtelHostBridge` | (obituary) singleton `nixling-otel-host-bridge.service` — **deleted** in P6 (`ph6-remove-systemd-emission`); replacement is the broker `SpawnRunner{role: OtelHostBridge, …}` row described here | empty | P1 decision 5 + security-2: bundle's `OtelHostBridge` runner intent MUST point at a VM whose `vm_name` equals `manifest._observability.vmName`; the broker refuses fail-closed via `Broker.OtelHostBridgeIntentInvalid` otherwise (closed-set). Pre-opened vsock fds only; `AF_VSOCK` / `AF_UNIX` socket(2) is denied by `w1-otel-host-bridge` seccomp policy. Bind set: alloy runtime dir (RW), obs VM CH vsock host UDS dir (RW). No `/dev` binds. Host-scoped profile `host-otel-host-bridge` (principal: `nixling-otel-bridge`, cgroup subtree: `nixling.slice/host/otel-host-bridge`). |


Sensitive path components are stable-hashed; user identity is stored
only as numeric `uid`/`gid` + the authz class; raw secrets are never
stored. Retention is daily rotation + a 14-day default deletion,
overridable via `nixling.site.audit.retentionDays` (W4a-H1).
**Reserved at W4a-H1**: broker prune-on-rotate is shipping, but the
NixOS option is not yet threaded into the broker invocation (W4 main
wave). The broker defaults to 14 days regardless of overrides until
that wiring lands.
See [`daemon-api.md`](daemon-api.md#audit) "Retention" for the prune
contract.

## P1 per-role device bind matrix

The P1 wave (daemon-only end-state) pins, per per-VM runner role, the
closed-set device-node bind list the broker opens via `OpenKvm` /
`OpenVhostNet` / `OpenFuse` / `OpenDevice` on behalf of the runner.
Every entry is grounded in the `DeviceClass` taxonomy
(`packages/nixling-host/src/devices.rs`); the broker refuses to open
any path absent from the role's bundle row, and the per-role minijail
profile (`nixos-modules/minijail-profiles.nix`) declares the bind set
via `mountPolicy.deviceBinds` so the runner's mount namespace cannot
see anything outside it.

### Gpu role

| Device | `DeviceClass` | Rationale |
| --- | --- | --- |
| `/dev/kvm` | `Kvm` | crosvm-gpu shares the runner's KVM fd for hypervisor coupling. |
| `/dev/dri/renderD128` | `Dri` | virgl/venus/cross-domain Wayland render node; carries the full `DRM_IOCTL_VIRTGPU_*` family (`GET_CAPS`, `CONTEXT_INIT`, `RESOURCE_CREATE`, `RESOURCE_CREATE_BLOB`, `SUBMIT_CMD`, `EXECBUFFER`, `WAIT`, `MAP`, `GETPARAM`) per `nixling_host::ioctl_policy::class_ioctls(DeviceClass::Dri)`. |
| `/dev/nvidiactl` | `NvidiaCtl` | NVIDIA control device — required for the Quadro T1000 driver context. |
| `/dev/nvidia0` | `NvidiaRender` | NVIDIA per-card primary device. **P1 framework device-taxonomy fix**: the original W3 enum mapped `NvidiaRender` to `/dev/nvidia-render`, which does not exist on real NVIDIA hosts; the correct path is `/dev/nvidia<N>` per the proprietary driver UAPI. Default path bumped to `/dev/nvidia0` in `DeviceClass::default_path`. |
| `/dev/nvidia-uvm` | `NvidiaUvm` | Unified-memory driver path used by VA-API NVDEC and Vulkan compute. |
| `/dev/udmabuf` | `Udmabuf` | Cross-domain dmabuf wrap path: cross-domain Wayland requires `UDMABUF_CREATE`/`UDMABUF_CREATE_LIST` to expose guest framebuffers to the host compositor without copy. **P1 framework device-taxonomy fix**: new `DeviceClass::Udmabuf` variant; previously absent from the enum and would have been refused by the broker dispatcher. |

In addition to the six device binds, the Gpu role's minijail profile
carries a single bind-mount mapping the host's per-user Wayland
socket into the role-local runtime dir so the runner can never
traverse `/run/user/<uid>`:

```
mountPolicy.bindMounts = [
  { src = "/run/user/<waylandUser-uid>/wayland-0";
    dst = "/run/nixling-gpu/<vm>/wayland-0"; }
];
```

- Caps: **empty** (per kernel-r2-4 corrected matrix — removed
  `CAP_SYS_NICE`; smoke proves no NICE needed at runtime).
- `seccompPolicyRef`: `w1-gpu` (closed-set syscall + ioctl allowlist
  derived from the device-bind set).
- `cgroupPlacement.subtree`: `nixling.slice/<vm>/gpu`.
- Validator: `tests/minijail-validator-gpu.sh` (positive
  `DRM_IOCTL_VIRTGPU_GET_CAPS` arm + negative `ptrace` arm; evidence
  at `/var/lib/nixling/validated/p1-gpu.json`; Layer-2 `NL_LIVE=1`
  hardware smoke on the host's Quadro T1000).
- Byte-parity golden: `tests/golden/runner-shape/gpu-argv-minimal.txt`
  via `nixling_host::gpu_argv::generate_gpu_argv`.

## P2 broker-op additions (daemon-only end-state)

The P2 wave of the daemon-only end-state migration (plan
`~/.copilot/session-state/<id>/plan.md` § "Phase 2: daemon-side
host-prep replaces per-VM systemd templates") **retired** the per-VM
systemd templates `microvm-tap-interfaces@<vm>.service`,
`microvm-set-booted@<vm>.service`,
`microvm-pci-devices@<vm>.service`,
`nixling-known-hosts-refresh@<vm>.service`,
`nixling-vfsd-watchdog@<vm>.{timer,service}`, and
`nixling-<vm>-store-sync.service` by folding their work into daemon
DAG nodes that dispatch to broker operations. **P6 sibling agent
`ph6-remove-systemd-emission` (branch `phase-p6-privileges-final`)
deleted the unit shells from `nixos-modules/`**; the canonical
daemon-only surface is now `nixlingd.service` +
`nixling-priv-broker.{service,socket}` + per-VM runners spawned
via broker `SpawnRunner`. The "Retired" column in the HostPrep DAG
table below carries the one-line obituary for each deleted unit.

This section documents the **new** broker ops the P2 daemon-side DAG
needs, plus the daemon-side preflights they ride beside. **StoreSync
is shipped** (broker wire variant + dispatcher + audit fields landed
in P2 wave 1, commit `bfe8c60`). Rows still marked **P2 (pending)**
are wire-reserved contracts that subsequent P2/P3 implementer agents
will land; they are NOT yet in `nixling_ipc::BrokerRequest` or
`packages/nixling-core/src/privileges.rs`. Until those land, the
`tests/privileges-matrix-completeness.sh` gate continues to pass
(it is one-directional: every broker-declared op must have a
rendered row; extra documented-but-undeclared rows are
permitted).

> **Spec correction (brief ↔ plan drift).** The integrator brief
> for this sub-agent named `BringUpTapInterface`,
> `SeedDnsmasqLease`, and `PreOpenVhostNetFd` as new typed
> HostPrep ops. The authoritative plan § "Per-unit retirement
> contract" (lines 199-207) does NOT introduce those names —
> instead, the P2 host-prep DAG composes the **existing W3
> broker ops** `CreateTapFd` / `CreatePersistentTap` (with
> `TUNSETOWNER`/`TUNSETGROUP` matching the exact runner uid/gid,
> including the graphics-VM `nixling-<vm>-gpu` owner),
> `SetBridgePortFlags`, `OpenDevice`, `ApplyNmUnmanaged`, and
> `ApplySysctl` in a fixed order. Following "existing code is
> canon" (AGENTS.md), this page documents what the plan actually
> contracts; the named ops from the brief are intentionally NOT
> rendered here.

### New broker ops

| Variant | Subject | Scope | Wave first delivered | Destructive | Secret access | Allowed groups | Audit | Default-for-unknown | Audit fields (in addition to common header) |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `StoreSync` | fs (per-VM hardlink farm + store-meta) | per VM / per generation | **P2 (shipped)** | yes (mkdir, chown, chmod, hardlink, gcroot symlinks, `current` symlink swap, `.marker` write) | no | `nixling-admin` (host install/apply path) + `nixling-launcher` (per-VM switch path) | yes | deny | `vm_id`, `generation`, `closure_hash`, `hardlink_farm_path_hash`, `store_meta_path_hash`, `gcroots_diff_hash`, `marker_present`, `acl_propagation_guard_result` |
| `SshKeygenProbe` | per-VM ssh control socket | per VM | P2 (pending) | no (read-only key fingerprint probe) | no | `nixling-launcher` + `nixling-admin` | yes | deny | `vm_id`, `socket_path_hash`, `keytype`, `fingerprint_hash`, `probe_result` |

### Per-operation cap usage (P2 additions)

| Operation | Capabilities used | Notes |
| --- | --- | --- |
| `StoreSync` | `CAP_FOWNER`, `CAP_DAC_OVERRIDE` | `CAP_FOWNER` for `fchmod`/`fchown` across the per-VM hardlink farm (broker runs as root but mutates `<vm>/store{,-meta}/` which is `nixlingd:users` 2770 `g+s`, so chown semantics require FOWNER); `CAP_DAC_OVERRIDE` for write under the trusted root-owned `/var/lib/nixling/vms/<vm>/` ancestor. The broker MUST NOT call `setfacl --recursive` across the hardlink farm — that propagates ACLs back into `/nix/store` paths through the hardlinks (the bug we hit on personal-dev today; see plan ph2-p2-ownership-matrix). The `acl_propagation_guard_result` audit field records the explicit "no recursive setfacl crossed into /nix/store" check. |
| `SshKeygenProbe` | — | empty bounding set: the op runs `ssh-keygen -F` / `-l` style fingerprint probes against the per-VM ssh control socket only. The broker dispatcher binds the probe target to `<vm>/sshd-host-keys/ssh_host_*_key.pub` derived from the bundle-pinned VM identity; no host-wide ssh-keygen surface is exposed. No `CAP_NET_*` because the probe runs over the pre-opened per-VM UDS, never a network socket. |

### HostPrep DAG (P2) — composition of existing W3 ops

The P2 host-prep DAG, executed daemon-side per VM start, dispatches
the W3 broker ops in a fixed canonical order. Each row below is a
**DAG node**, not a new broker variant. The `Retired (deleted in P6)`
column carries the one-line obituary for the legacy systemd template
the DAG node replaced; every named unit has been **deleted from
`nixos-modules/`** by P6 sibling agent `ph6-remove-systemd-emission`
(branch `phase-p6-privileges-final`).

| DAG node | Retired (deleted in P6) | Broker op(s) called | Ordering constraint |
| --- | --- | --- | --- |
| `host-prep.nm-unmanaged` | — (carry-over; previously activation-time) | `ApplyNmUnmanaged` | first — must precede tap create so NetworkManager does not claim the iface mid-creation |
| `host-prep.tap` | `microvm-tap-interfaces@<vm>.service` — **deleted P6**; replaced by `CreateTapFd` / `CreatePersistentTap` broker dispatch in this DAG node | `CreateTapFd` (fd handoff path) **or** `CreatePersistentTap` (with `TUNSETOWNER`/`TUNSETGROUP` set to the runner uid/gid — graphics VMs MUST use the `nixling-<vm>-gpu` uid, NOT `microvm`) | after `host-prep.nm-unmanaged`, before `host-prep.sysctl` |
| `host-prep.sysctl` | — (carry-over) | `ApplySysctl` (per-link IPv6-off + MTU) | after `host-prep.tap`, before `host-prep.bridge` |
| `host-prep.bridge` | `microvm-tap-interfaces@<vm>.service` (bridge-port subset) — **deleted P6**; replaced by `SetBridgePortFlags` broker dispatch | `SetBridgePortFlags` | after `host-prep.sysctl`, before `host-prep.spawn` |
| `host-prep.pci-devices` | `microvm-pci-devices@<vm>.service` — **deleted P6**; replaced by `OpenDevice` broker dispatch (device taxonomy extended to cover the PCI passthrough surface the retired unit handled) | `OpenDevice` | parallel with `host-prep.tap` chain; joins before `host-prep.spawn` |
| `host-prep.store-sync` | `nixling-<vm>-store-sync.service` + activation-time `nixling-store-sync` call from `store.nix` — **deleted P6**; replaced by `StoreSync` broker dispatch (P6 also retires the activation-time hook in favour of `nixling host install --apply`) | `StoreSync` (P2 new) | before any per-VM runner spawn; for the host-install/apply path, runs as part of `host install --apply` (which IS the activation hook from P6 onward) |
| `host-prep.known-hosts-refresh` | `nixling-known-hosts-refresh@<vm>.service` — **deleted P6**; replaced by `SshKeygenProbe` broker dispatch | `SshKeygenProbe` (P2 new) | after `vm.sshReady`, not in the cold-start chain |
| `vm.set-booted` | `microvm-set-booted@<vm>.service` — **deleted P6**; replaced by pure-daemon `supervisor::state::record_booted(<vm>, <closure>)` (no broker op) | — (pure daemon: `supervisor::state::record_booted(<vm>, <closure>)`) | after runner reports ready; no broker call |
| `host-prep.spawn` | — (final join) | `SpawnRunner` | after every preceding `host-prep.*` node completes; carries SCM_RIGHTS handoff of fds from `CreateTapFd` / `OpenDevice` / `OpenKvm` / etc. |

The daemon's `vfsd-watchdog` replacement is purely
`supervisor::pidfd` watching the virtiofsd pidfd and re-issuing
`SpawnRunner` on exit; no new broker op. The legacy
`nixling-vfsd-watchdog@<vm>.{timer,service}` pair was **deleted in
P6** (`ph6-remove-systemd-emission`).

### P2 preflights (daemon-side, no broker call)

The P2 daemon refuses to start a VM if any of the following
preflights fail. Each runs against `<vm>/`'s on-disk state with
`O_NOFOLLOW` and no privileged ops; they are documented here for
trust-boundary completeness, not because they are broker ops.

| Preflight | Subject | Capabilities | Refusal envelope | Plan reference |
| --- | --- | --- | --- | --- |
| `OwnershipMatrixCheck` | `/var/lib/nixling/vms/<vm>/` ownership matrix | — (pure `fstatat` traversal; the daemon already has `CAP_DAC_READ_SEARCH` for its state dir, no new caps) | refuses VM start with typed `daemon.ownership-matrix-drift` envelope citing the first drifted leaf (path, expected `owner:group mode`, observed) | `ph2-p2-ownership-matrix` (plan line 546); full matrix at plan §"Ownership matrix for `/var/lib/nixling/vms/<vm>/`" (lines 230-253) |
| `SshHostKeyPreflight` | `<vm>/sshd-host-keys/ssh_host_*_key` | — (`O_NOFOLLOW` `openat`, `fstat`) | refuses VM start with typed `daemon.ssh-host-key-drift` envelope on: symlink, owner/group != root, mode != `0400` | `ph2-p2-ssh-host-key-preflight` (plan line 548); security-r2-3 |
| `DnsmasqLeaseHashPreflight` (net VMs only) | `${dnsmasq_dir}/<env>.conf` (default `/var/lib/nixling/dnsmasq/<env>.conf`) vs bundle `hosts_intent` + `route_intent[env:<env>:*]` + `nft_intent[env:<env>]` | — (pure `read()` + SHA-256; the daemon already has read access to its state dir) | refuses net-VM start with typed `daemon.bundle-dnsmasq-drift` envelope (exit code `63`); covers `EnvMissing`, `ConfigMissing`, `ConfigReadFailed`, `HashMismatch`; remediation: re-render dnsmasq.conf (host singleton) and retry, or `nixos-rebuild switch`. See [`docs/reference/net-vm-bundle-gate.md`](./net-vm-bundle-gate.md). | `ph2-p2-net-vm-bundle-gate`; plan line 215; networking-r3-3 |
| `HostModuleMatrixPreflight` | trusted host kernel modules: `kvm_intel`/`kvm_amd`, `vhost`, `vhost_vsock`, `vhost_net`, `tun`, `bridge`, `nf_tables`, `nf_conntrack`, plus per-env `usbip-host`, plus `virtio_media` for video-enabled VMs | — (reads `/proc/modules`) | refuses VM start with `daemon.host-module-missing` envelope; remediation suggests `ModprobeIfAllowed` (broker op, separate path) | plan line 216; kernel-r2-4 |

The four preflights run in fixed order on every `nixling vm start
<vm> --apply`; `OwnershipMatrixCheck` runs first so a partially-
migrated host surfaces drift before any other check touches the VM
state.

### Cross-references (P2)

- Plan §"Phase 2: daemon-side host-prep replaces per-VM systemd
  templates" — authoritative contract.
- `tests/restart-policy-eval.sh` — per-phase migration; P2 begins
  swapping per-VM-unit assertions for absence + daemon-equivalent
  assertions.
- `tests/processes-json-drift.sh` — extended in P2 to assert no
  `nixling-<vm>-*` or `microvm-*@<vm>` references remain in
  `processes.json`.
- `tests/store-marker-eval.sh` — `<vm>/store-meta/.marker`
  presence regression gate (called from `StoreSync` audit).
- AGENTS.md "Critical subsystems — handle with care" rows for
  per-VM `/nix/store` hardlink farm and TPM state — `StoreSync`
  MUST honor both invariants.

## P3 broker-op additions (daemon-only end-state)

The P3 wave of the daemon-only end-state migration (plan
`~/.copilot/session-state/<id>/plan.md` § "Phase 3: replace host
singletons via broker `SpawnRunner` + daemon") replaces the
remaining host-singleton systemd units with broker `SpawnRunner`
runners + daemon-emitted telemetry. This section documents the
**new** broker-dispatch contracts the P3 wave adds; the underlying
broker variants (`SpawnRunner`, `ApplyNftables`, `ApplyRoute`,
`SetBridgePortFlags`, `ModprobeIfAllowed`, etc.) are already
delivered in P0/W3 and do not change wire shape.

### New broker-dispatch contracts (per-runner-role)

These rows extend the runner-role registry in the "Runner roles
(selected via `SpawnRunner.role`)" table above with the P3 dispatch
contracts. The cap matrix continues to be sourced from the
"Per-role minijail profile (P1)" table; this section pins the
**broker-dispatch contract** that the broker enforces fail-closed
before fork/exec.

| Runner role | Retired (deleted in P6) | Caps (steady-state) | Per-env scope | Broker-dispatch contract (P3) |
| --- | --- | --- | --- | --- |
| `OtelHostBridge` | `nixling-otel-host-bridge.service` — **deleted P6** (`ph6-remove-systemd-emission`); replaced by broker `SpawnRunner{role: OtelHostBridge, …}` dispatched per the contract column | empty | host-scoped (singleton — exactly one runner per host) | Broker refuses `SpawnRunner{role: OtelHostBridge, …}` fail-closed via `Broker.OtelHostBridgeIntentInvalid` (closed-set) unless the bundle's `OtelHostBridge` runner intent points at a VM whose `vm_name` equals `manifest._observability.vmName`. Readiness gate (per ph3-p3-otelbridge-readiness): alloy runtime dir exists with expected ownership, stale `host-egress.sock` removed, obs VM base `vsock.sock` exists; exponential backoff on host-OTLP unreachable. Broker waits the readiness gate before exec; `supervisor::pidfd` respawns on relay exit. Pre-opened vsock fds only; `socket(AF_VSOCK)` denied by `w1-otel-host-bridge` seccomp. |
| `Usbip` | per-env singletons `nixling-sys-<env>-usbipd-{proxy,backend}.{service,socket}` (3 envs × 4 units) — **deleted P6** (`ph6-remove-systemd-emission`); replaced by broker `SpawnRunner{role: Usbip, vm_id: sys-<env>-usbipd, …}` dispatched per the per-busid state machine in the contract column | `CAP_NET_RAW` (per-env usbipd proxy bind) | **per env** — one runner per USBIP-enabled env (`vm_id` = `sys-<env>-usbipd`) | Broker dispatches `SpawnRunner{role: Usbip, vm_id: sys-<env>-usbipd, …}` only inside the per-busid state machine (`ph3-p3-usbip-state-machine`): `ModprobeIfAllowed(usbip-host)` **first**, then acquire `/run/nixling/locks/usbip/<busid>` for the target env, then withhold non-owner-env `SpawnRunner`s for that busid, then `UsbipBindFirewallRule` carve-out, then start backend, then `UsbipBind(busid, vm)`, then proxy listen. Stop reverses order: proxy down → `UsbipUnbind` → firewall remove → release lock. Host kernel module is `usbip-host` (kernel-r2-7 correction: NOT `vhci_hcd`, which is the GUEST module). |

### Metrics endpoint (forthcoming — ph3-p3-prometheus-otlp-shape)

The P3 wave's sibling deliverable `ph3-p3-prometheus-otlp-shape`
pins the daemon-emitted metrics surface that retires
`nixling-ch-exporter.service`. The endpoint is daemon-served (not a
broker op) but its capability + sandbox posture is documented here
alongside the broker-dispatch contracts for trust-boundary
completeness.

| Endpoint | Served by | Transport | Capabilities | Sandbox posture | Notes |
| --- | --- | --- | --- | --- | --- |
| `http://127.0.0.1:9101/metrics` | `nixlingd` (daemon, **not** broker) | HTTP Prometheus exposition, no auth (loopback-only bind — same scrape contract as today's `nixling-ch-exporter`) | **empty** bounding set on `nixlingd.service` (the daemon already runs unprivileged; the metrics handler adds no new caps) | `NoNewPrivileges=true` on `nixlingd.service` (P0 invariant; the metrics handler inherits it). Listener is `127.0.0.1:9101` only — never `0.0.0.0`. | Forthcoming: handler implementation lands in `ph3-p3-prometheus-otlp-shape`; this row is the contract sibling agents may cite. Metric names preserved (`nixling_vm_ch_api_up`, `nixling_vm_running`, `nixling_vm_state`); cardinality budget per `ph3-p3-prometheus-otlp-shape`: `vm`/`env`/`role` labels only by default, topology labels opt-in. |
| `unix:///run/nixling/host-otlp.sock` | `nixlingd` (daemon) | OTLP/gRPC over `AF_UNIX` (matches today's alloy host-egress consumer) | **empty** bounding set | `NoNewPrivileges=true`; socket created with `0660 nixlingd:nixlingd` so only alloy (in `nixlingd` group) can connect. | Forthcoming with `ph3-p3-prometheus-otlp-shape`. Span/log attributes constrained by `ph3-p3-tracing-contract` (no secrets, no argv, no `/nix/store` paths). |

### Mutating recovery verb (P3)

The P3 daemon enters **degraded mode** rather than refusing to
serve when bridge/route self-check fails (per
`ph3-p3-net-route-degraded-mode`). Read-only `status` / `doctor` /
`audit` remain available; per-env starts are blocked. The **sole**
mutating recovery verb is `nixling host reconcile --network
--apply`, which the broker dispatches through the existing W3
host-prep ops (`CreateTapFd` / `CreatePersistentTap` /
`SetBridgePortFlags` / `ApplyNftables` / `ApplyRoute` /
`ApplySysctl` / `ApplyNmUnmanaged` / `UpdateHostsFile`) to recreate
bridges/routes **without** starting any VM. No new broker variant.

### Cross-references (P3)

- Plan §"Phase 3: replace host singletons via broker
  `SpawnRunner` + daemon" — authoritative contract.
- Sibling P3 deliverables: `ph3-p3-prometheus-otlp-shape`
  (metrics + OTLP endpoint), `ph3-p3-tracing-contract`
  (span attribute allowlist), `ph3-p3-loki-label-contract`
  (Loki label allowlist), `ph3-p3-otelbridge-readiness`
  (OtelHostBridge readiness gate), `ph3-p3-usbip-state-machine`
  (per-busid canonical order), `ph3-p3-net-route-degraded-mode`
  (degraded mode + reconcile verb), `ph3-p3-kernel-module-check`
  (daemon startup self-check), `ph3-p3-host-doctor-extended`
  (singleton liveness via pidfd).

## P3 host singleton retirements (deleted in P6)

The P3 wave retired three host-singleton systemd units in favour of
daemon-native equivalents; the unit shells remained in
`nixos-modules/` through P3..P5 implementation as scheduled-for-removal
artefacts. **P6 sibling agent `ph6-remove-systemd-emission` (branch
`phase-p6-privileges-final`) deleted every shell.** This table is the
final-pass obituary: the daemon-only replacement is the live surface
and the legacy unit name MUST NOT be cited in any consumer-facing
remediation.

| Retired singleton (deleted P6) | Replacement (daemon-only) | One-line obituary | Deliverable |
| --- | --- | --- | --- |
| `nixling-net-route-preflight.service` | Daemon startup self-check; on failure the daemon enters **degraded mode** (refuses per-env starts; keeps `status`/`doctor`/`audit` read-only available). The **sole** mutating recovery verb is `nixling host reconcile --network --apply` which the broker dispatches through existing W3 host-prep ops to recreate bridges/routes without starting any VM. | Deleted P6 (`ph6-remove-systemd-emission`); replaced by `nixlingd` startup self-check + broker `ApplyNftables` / `ApplyRoute` / `ApplySysctl` / `SetBridgePortFlags` dispatched via `nixling host reconcile --network --apply`. | `ph3-net-singletons` + `ph3-p3-net-route-degraded-mode` (plan line 276; networking-2, networking-r3-1) |
| `nixling-audit-check.service` + `nixling-audit-check.timer` | Daemon health endpoint that reads the broker `OpAuditRecord` daily files via `ExportBrokerAudit`; the Rust CLI `nixling audit` reads through the daemon. No separate systemd timer — `nixling host doctor` polls on demand. | Deleted P6 (`ph6-remove-systemd-emission`); both `.service` and `.timer` are gone; replaced by broker `ExportBrokerAudit` + `nixling host doctor`. | `ph3-p3-audit-check-retire` (plan line 277; security-6 audit parity) |
| `nixling-ch-exporter.service` | Daemon-emitted OTel metrics with preserved metric names (`nixling_vm_ch_api_up`, `nixling_vm_running`, `nixling_vm_state`) and bounded labels (`vm`/`env`/`role` only by default). Endpoint shape pinned by `ph3-p3-prometheus-otlp-shape` (Prometheus at `http://127.0.0.1:9101/metrics`, host OTLP at `unix:///run/nixling/host-otlp.sock`). | Deleted P6 (`ph6-remove-systemd-emission`); replaced by `nixlingd`-served Prometheus exposition at `127.0.0.1:9101` (no broker op — daemon-emitted directly). | `ph3-p3-ch-exporter-retire` (plan line 278; observability-1, observability-2, observability-r3-1) |
| `nixling-otel-host-bridge.service` | Broker `SpawnRunner{role: OtelHostBridge}` runner (host-scoped singleton). See "New broker-dispatch contracts (per-runner-role)" above and the Runner-roles table for the contract. | Deleted P6 (`ph6-remove-systemd-emission`); re-homed as broker `SpawnRunner{role: OtelHostBridge}` runner instead of a host singleton service. | `ph3-p3-otelbridge-readiness` |
| `nixling-sys-<env>-usbipd-proxy.service` + `nixling-sys-<env>-usbipd-proxy.socket` + `nixling-sys-<env>-usbipd-backend.service` + `nixling-sys-<env>-usbipd-backend.socket` (per USBIP-enabled env) | Broker `SpawnRunner{role: Usbip, vm_id: sys-<env>-usbipd}` runner per env, gated by the per-busid state machine. See the Runner-roles row above. | Deleted P6 (`ph6-remove-systemd-emission`); re-homed as broker `SpawnRunner{role: Usbip}` runners + `UsbipBindFirewallRule` / `UsbipBind` / `UsbipUnbind` / `UsbipProxyReconcile` broker ops. | `ph3-p3-usbip-state-machine` |

## P6 final-pass: comprehensive legacy systemd surface obituary

This section is the canonical post-P6 obituary index. Every legacy
systemd template and host singleton named here was **deleted from
`nixos-modules/`** by P6 sibling agent `ph6-remove-systemd-emission`
(branch `phase-p6-privileges-final`, base `phase-daemon-only @
29e37de`). After P6 the canonical surface is exactly:

- `nixlingd.service` (unprivileged daemon)
- `nixling-priv-broker.service` + `nixling-priv-broker.socket`
  (socket-activated privileged broker)
- per-VM / per-role runners spawned via broker `SpawnRunner` (no
  systemd unit per runner; lifecycle is daemon-supervised via
  pidfd)

The `tests/privileges-doc-completeness-eval.sh` Layer-1 gate enforces
that every legacy template still emitted by `nixos-modules/` either
has a live broker-op row in this document or appears below as a
deleted obituary — never both.

### Per-VM template obituaries (deleted P6)

| Legacy unit | Replacement (broker op + runner role) | Deliverable |
| --- | --- | --- |
| `nixling@<vm>.service` (pre-P6 host-wrapper.nix; retired in P6) | Daemon-supervised VM lifecycle: `nixlingd::supervisor::dag` orchestrates the 5-node DAG; broker `SpawnRunner{role: CloudHypervisor, vm_id: <vm>, …}` for the runner. | `ph6-remove-systemd-emission` (host-wrapper.nix deletion) |
| `microvm@<vm>.service` (upstream microvm.nix wrapper invoked by `nixling@<vm>`) | Replaced by direct broker `SpawnRunner{role: CloudHypervisor}` dispatch; the framework no longer composes the upstream template. | `ph6-remove-systemd-emission` |
| `microvm-tap-interfaces@<vm>.service` | `host-prep.tap` DAG node → `CreateTapFd` / `CreatePersistentTap` broker dispatch. See HostPrep DAG table above. | P2 `ph2-p2-tap-dag-contract`; deleted P6 |
| `microvm-set-booted@<vm>.service` | `vm.set-booted` DAG node → pure-daemon `supervisor::state::record_booted(<vm>, <closure>)` (no broker op). | P2; deleted P6 |
| `microvm-pci-devices@<vm>.service` | `host-prep.pci-devices` DAG node → `OpenDevice` broker dispatch. | P2; deleted P6 |
| `microvm-virtiofsd@<vm>.service` (upstream template) | Broker `SpawnRunner{role: Virtiofsd, vm_id: <vm>, …}` + `supervisor::pidfd` watchdog. | `ph6-remove-systemd-emission` (store.nix drop-in removed alongside) |
| `nixling-<vm>-gpu.service` (host-sidecars.nix) | Broker `SpawnRunner{role: Gpu, vm_id: <vm>, …}` per the P1 Gpu role matrix. | `ph6-remove-systemd-emission` (host-sidecars.nix deletion) |
| `nixling-<vm>-video.service` (components/video/host.nix) | Broker `SpawnRunner{role: Video, vm_id: <vm>, …}` per the P1 Video role matrix. | `ph6-remove-systemd-emission` (components/video/host.nix deletion) |
| `nixling-<vm>-snd.service` (components/audio/host.nix) | Broker `SpawnRunner{role: Audio, vm_id: <vm>, …}` per the P1 Audio role matrix. | `ph6-remove-systemd-emission` (components/audio/host.nix deletion) |
| `nixling-<vm>-swtpm.service` (host-sidecars.nix) | Broker `SpawnRunner{role: Swtpm, vm_id: <vm>, …}` (long-lived sidecar) + `SpawnRunner{role: SwtpmFlush, vm_id: <vm>}` (pre-start one-shot). | `ph6-remove-systemd-emission` |
| `nixling-<vm>-store-sync.service` | `host-prep.store-sync` DAG node → `StoreSync` broker dispatch. | P2 `ph2-p2-daemon-autostart`; deleted P6 |
| `nixling-known-hosts-refresh@<vm>.service` | `host-prep.known-hosts-refresh` DAG node → `SshKeygenProbe` broker dispatch. | P2; deleted P6 |
| `nixling-vfsd-watchdog@<vm>.service` + `nixling-vfsd-watchdog@<vm>.timer` | Pure-daemon `supervisor::pidfd` watch on the virtiofsd runner pidfd; re-issues `SpawnRunner` on exit. No broker op. | P2; deleted P6 |
| `nixling-otel-relay@<vm>.service` (host-otel-relay-acl.nix) | Broker `SpawnRunner{role: OtelHostBridge}` host singleton runner (one per host, not per VM). | `ph6-remove-systemd-emission` |

### Host singleton obituaries (deleted P6)

| Legacy unit | Replacement (broker op or daemon surface) | Deliverable |
| --- | --- | --- |
| `nixling-net-route-preflight.service` | Daemon startup self-check + degraded mode + `nixling host reconcile --network --apply` (broker `ApplyNftables` / `ApplyRoute` / `ApplySysctl` / `SetBridgePortFlags`). | P3 `ph3-p3-net-route-degraded-mode`; deleted P6 |
| `nixling-audit-check.service` + `nixling-audit-check.timer` | Broker `ExportBrokerAudit` + `nixling host doctor` on-demand poll; no timer. | P3 `ph3-p3-audit-check-retire`; deleted P6 |
| `nixling-ch-exporter.service` | `nixlingd` Prometheus exposition at `http://127.0.0.1:9101/metrics` (no broker op — daemon-emitted). | P3 `ph3-p3-ch-exporter-retire`; deleted P6 |
| `nixling-otel-host-bridge.service` | Broker `SpawnRunner{role: OtelHostBridge}` (host-scoped singleton, broker-supervised). | P3 `ph3-p3-otelbridge-readiness`; deleted P6 |
| `nixling-sys-<env>-usbipd-proxy.{service,socket}` + `nixling-sys-<env>-usbipd-backend.{service,socket}` (per USBIP-enabled env) | Broker `SpawnRunner{role: Usbip, vm_id: sys-<env>-usbipd}` + per-busid state machine (`UsbipBindFirewallRule` → `UsbipBind` → proxy listen). | P3 `ph3-p3-usbip-state-machine`; deleted P6 |

### Activation-time hooks retired in P6

| Hook | Replacement | Deliverable |
| --- | --- | --- |
| `nixling-store-sync` activation hook (store.nix) | `nixling host install --apply` invoking broker `StoreSync` dispatch through the daemon. | P6 `ph6-p6-cli-nix-migrations` (cli.nix consumer migration prior to package deletion) |
| Per-VM `desktopItems` generation (cli.nix `nixling-launch-<vm>`) | Daemon-native launcher module emitting `.desktop` wrappers calling `nixling vm start <vm> --apply`. | P4 `ph4-p4-desktop-wrapper`; cli.nix deleted P6 |

After P6 the only `systemd.services.*` declarations the framework
owns under `nixos-modules/` are `nixlingd`, `nixling-priv-broker`,
`nixling-load-store-db` (boot-time tmpfiles helper),
`nixling-load-host-keys` (boot-time helper), and the upstream
`pipewire` / `alloy` / `grafana` services the observability stack
delegates to. Every per-VM `nixling-<vm>-*` and `microvm-*@<vm>`
template, and every `nixling-{net-route-preflight,audit-check,
ch-exporter,otel-host-bridge}` singleton, is gone.

## Operations explicitly out of W3 scope

- `UsbipBind`, `UsbipUnbind`, `UsbipProxyReconcile` — live USBIP
  device routing. W3 ships only `UsbipBindFirewallRule` as the
  skeleton.
- Partition-root cpuset creation (`cpuset.cpus.partition=root`). W3
  forbids it without a panel-approved ADR.
- Threaded cgroups. Same forbidding rule as partition roots.

## Cross-references

- ADR 0011 (cgroup + pidfd), ADR 0012 (IPv6/IfName/bridge-port),
  ADR 0013 (firewall coexistence), ADR 0014 (modules + devices +
  runner-shape).
- [`docs/explanation/host-prepare.md`](../explanation/host-prepare.md) — conceptual model + recovery.
- [`docs/reference/error-codes.md`](error-codes.md) — typed
  exit-code catalog + W3 audit decision codes section.
- [`docs/reference/cgroup-delegation.md`](cgroup-delegation.md).
- [`docs/reference/inet-nixling-chains.md`](inet-nixling-chains.md).
- [`SECURITY.md`](../../SECURITY.md) § W3 trust-boundary delta.

## P0 broker CapabilityBoundingSet (canonical 8 caps)

The P0 wave narrowed the broker `CapabilityBoundingSet` to exactly the
following 8 capabilities. `CAP_SYS_PTRACE` is explicitly excluded.

| Capability | Rationale |
| --- | --- |
| `CAP_NET_ADMIN` | TAP interface creation, persistent TAP lifecycle, bridge port flags (`SetBridgePortFlags`), route programming (`ApplyRoute`), NetworkManager unmanaged drop-in writes. |
| `CAP_NET_RAW` | Raw socket required by the per-link sysctl IPv6-off sequencer (socket-based `ioctl` path on kernels that reject `sysctl` writes from non-root). |
| `CAP_DAC_OVERRIDE` | Write to root-owned files in trusted paths (audit log dir, tmpfiles-created directories, `/etc/hosts` nixling-managed block, NetworkManager conf.d drop-in). |
| `CAP_DAC_READ_SEARCH` | Directory traversal + open of cgroup subtree directories (required for `DelegateCgroupV2`/`OpenCgroupDir` before `fchown` to `nixlingd`). |
| `CAP_SYS_ADMIN` | cgroup v2 delegation: `open("/sys/fs/cgroup/nixling.slice", ...)` + `fchown` on delegated subdirs; mount namespace setup for `SetupMountNamespace`. |
| `CAP_SETUID` | Drop to `nixlingd` uid for `SpawnRunner` (the broker forks a runner child and drops privileges before exec). |
| `CAP_SETGID` | Drop to `nixlingd-launchers` gid as part of the same spawn descent. |
| `CAP_FOWNER` | `fchmod`/`fchown` on files the broker creates (audit log entries, socket ACL repair) when the effective uid is not the file owner. |

### Socket-activation contract

`nixling-priv-broker.socket` is socket-activated. systemd creates,
binds, listens on, and applies the ACL to the socket before the broker
process starts. The broker adopts the fd via `SD_LISTEN_FDS`.

**Invariants that must never be broken:**

- systemd **owns** the bind/listen/ACL lifecycle; the broker MUST NOT
  call `bind`, `fchmod`, or `fchown` on the socket path when
  `SD_LISTEN_FDS=1` is set.
- `LISTEN_FDNAMES` MUST equal `"priv.sock"`; any mismatch is a fatal
  startup error.
- The broker calls `sd_notify(READY=1)` only after the listener fd is
  adopted and the audit log is open — the systemd `notify` service type
  guarantees daemon readiness.
- `nixlingd.service` carries `Wants=nixling-priv-broker.socket` (not
  `Requires=`) so the daemon can serve even when the broker has idled.
- The socket group is `nixlingd`; `SO_PEERCRED` is used at accept time
  to authorise callers: only peers whose gid is `nixling-launcher` or
  `nixling-admin` are authorised; all others receive `denied-refused`.

### Per-operation cap usage

The table below maps each P0-delivered broker operation to the specific
capabilities it exercises. Operations that require no elevated capability
(pure `SO_PEERCRED` authz + file-descriptor passing) carry `—`.

| Operation | Capabilities used | Notes |
| --- | --- | --- |
| `ValidateBundle` | — | read-only bundle validation via the bundle resolver |
| `ExportBrokerAudit` | `CAP_DAC_READ_SEARCH` | open audit-log dir for export |
| `DelegateCgroupV2` | `CAP_SYS_ADMIN`, `CAP_DAC_READ_SEARCH` | open + `fchown` nixling.slice subtree |
| `OpenCgroupDir` | `CAP_DAC_READ_SEARCH` | open per-VM cgroup dir for fd-passing |
| `PrepareStateDir` / `PrepareRuntimeDir` | `CAP_DAC_OVERRIDE`, `CAP_FOWNER` | mkdir + chown + chmod under trusted paths |
| `OpenKvm` / `OpenVhostNet` / `OpenFuse` / `OpenDevice` | `CAP_DAC_OVERRIDE` | open device node on behalf of launcher |
| `CreateTapFd` / `CreatePersistentTap` | `CAP_NET_ADMIN` | `TUNSETIFF` + `TUNSETOWNER` on `/dev/net/tun` |
| `SetBridgePortFlags` | `CAP_NET_ADMIN` | `ioctl(SIOCSIFFLAGS)` on bridge port |
| `ApplyNftables` | `CAP_NET_ADMIN` | nftables ruleset load via netlink |
| `ApplyRoute` | `CAP_NET_ADMIN` | rtnetlink route add/del |
| `ApplySysctl` | `CAP_NET_RAW`, `CAP_NET_ADMIN` | per-link IPv6-off via `ioctl` or `sysctl` |
| `ApplyNmUnmanaged` | `CAP_DAC_OVERRIDE` | write NetworkManager conf.d drop-in |
| `UpdateHostsFile` | `CAP_DAC_OVERRIDE` | write `/etc/hosts` managed block |
| `ModprobeIfAllowed` | `CAP_SYS_ADMIN` | `finit_module` against trusted module matrix |
| `SpawnRunner` | `CAP_SETUID`, `CAP_SETGID` | drop to nixlingd uid/gid before exec |
| `SetupMountNamespace` | `CAP_SYS_ADMIN` | unshare + mount-bind inside mount namespace |

Reviewers auditing capability drift: the broker binary is built with
`#![forbid(unsafe_code)]` (quarantined exception: `src/sys.rs` for
`SCM_RIGHTS` fd-passing FFI). Any new operation that claims a capability
not listed above requires a panel-approved finding or a new ADR entry.

## Per-role minijail profile (P1)

Per the plan's P1 per-role capability matrix (kernel-r2-4 corrected)
and the closed-set seccomp / ioctl / cap / bind contract
(security-r2-4), every runner role ships a closed allowlist minijail
profile rendered by `nixos-modules/minijail-profiles.nix`. The table
below is the canonical operator-facing view; each row is enforced by
its per-role validator (`tests/minijail-validator-<role>.sh`) which
writes `/var/lib/nixling/validated/p1-<role>.json` on success.

| Role | Profile id pattern | Caps (steady-state) | Setup-time carve-out / device binds | Validator |
| --- | --- | --- | --- | --- |
| `cloud-hypervisor` | `vm-<vm>-cloud-hypervisor` | `CAP_NET_ADMIN` (transient — runner drops it after the SCM_RIGHTS tap-fd recv path before entering its main loop; static minijail allowlist cannot express "transient", so the profile declares the setup-time union) | `/dev/kvm`, `/dev/net/tun` (per the P1 device matrix; optional `/dev/dri/renderD128` + `/dev/nvidia0` when graphics/accelerator passthrough is bound to this runner) | `tests/minijail-validator-cloud-hypervisor.sh` |
| `virtiofsd` | `vm-<vm>-virtiofsd-<tag>` | empty (kernel-r2-4 steady-state) | startup carve-out: `CAP_SYS_ADMIN`, `CAP_SETPCAP`, `CAP_CHOWN`, `CAP_FOWNER`, `CAP_FSETID`, `CAP_SETUID`, `CAP_SETGID`, `CAP_DAC_OVERRIDE`, `CAP_MKNOD`, `CAP_SETFCAP` — required transiently during `virtiofsd --sandbox=namespace` setup before the daemon drops to an empty bounding set inside its own user namespace; ADR 0003 (`virtiofsdRootException` marker on the profile) | `tests/minijail-validator-virtiofsd.sh` (positive: `virtiofsd --version` under the carve-out profile; negative: `ptrace` probe under the `w1-virtiofsd` seccomp policy must exit with SIGSYS) |
| `swtpm` (long-lived sidecar) | `vm-<vm>-swtpm` | empty | **CRITICAL** RW bind of `/var/lib/nixling/vms/<vm>/swtpm` (TPM 2.0 NVRAM + EK seed) + `/run/swtpm/<vm>` (control socket). MUST be real RW bind, NOT tmpfs. Wiping/losing the bind forces Entra/Intune re-enrollment for work-aad (AGENTS.md critical-subsystem invariant). | `tests/minijail-validator-swtpm.sh` + `tests/swtpm-persistence-smoke.sh` (write/stop/daemon-restart/read-back persistence regression) |
| `swtpm-flush` (pre-start one-shot) | `vm-<vm>-swtpm-flush` | empty | Same `/var/lib/nixling/vms/<vm>/swtpm` + `/run/swtpm/<vm>` binds as the long-lived `swtpm` sidecar; runs `swtpm_ioctl -i` flush before the sidecar adopts state. | shares the swtpm validator + persistence smoke |
| `gpu` | `vm-<vm>-gpu` | empty (kernel-r2-4 corrected — previously `CAP_SYS_NICE`; per-role smoke proves virgl/venus/cross-domain run under SCHED_OTHER) | device binds: `/dev/kvm`, `/dev/dri/renderD128`, `/dev/nvidiactl`, `/dev/nvidia0`, `/dev/nvidia-uvm`, `/dev/udmabuf`; mount `/run/user/<uid>/wayland-0` → `/run/nixling-gpu/<vm>/wayland-0`; ioctls: full `DRM_IOCTL_VIRTGPU_*` family (via DeviceClass::Dri) | `tests/minijail-validator-gpu.sh` (positive: DRM_IOCTL_VIRTGPU_GET_CAPS under profile; negative: ptrace → SIGSYS) |
| `audio` | `vm-<vm>-audio` | `CAP_NET_RAW` (vhost-user-sound bind on PipeWire mediation path; AF_NETLINK for virtio-snd) | RO bind of `/run/user/<uid>/pipewire-0`; RW bind of `/run/nixling/vms/<vm>/snd.sock`; seccompPolicyRef = `w1-audio` | `tests/minijail-validator-audio.sh` |
| `video` | `vm-<vm>-video` | empty (kernel-r2-4) | RO bind of `/dev/dri/renderD128` for virtio-media decode (kernel-8 wire contract: `virtio_id=48`, 2×256 queues, 256 MiB SHM, `vring_base=0`); RW bind of `/run/nixling-video/<vm>/` (vhost-user-media socket dir); seccompPolicyRef = `w1-video`; principal shares `nixling-<vm>-gpu` uid for DRM access | `tests/minijail-validator-video.sh` |
| `vsock-relay` | `vm-<vm>-vsock-relay` | empty (kernel-r2-4 — pre-opened fds only, no AF_VSOCK socket creation in-role) | bind: per-VM `/var/lib/nixling/vms/<vm>/vsock.sock` (the inherited UDS); seccompPolicyRef = `w1-vsock-relay` (denies `socket(AF_VSOCK)` + `ptrace`) | `tests/minijail-validator-vsock-relay.sh` |
| `usbip` | `vm-<vm>-usbip` | `CAP_NET_RAW` (per-env usbipd proxy bind) | host module `usbip-host` (kernel-r2-7 correction: was incorrectly `vhci_hcd`, which is the GUEST module); broker's `ModprobeIfAllowed(usbip-host)` runs first in the per-busid sequence; per-busid state machine is P3 (`ph3-p3-usbip-state-machine`) | `tests/minijail-validator-usbip.sh` |
| `otel-host-bridge` | (host-scoped) `nixling-otel-host-bridge` | empty (observability-4 + decision 5: fd-only contract; no AF_VSOCK socket creation) | bind set: alloy runtime dir, CH vsock host socket, host-egress.sock (RW listen target); broker rejects bundle intent whose source VM ≠ `observability.vmName` | `tests/minijail-validator-otel-host-bridge.sh` |


## Related ADRs

- [ADR 0015: daemon-only clean break](../adr/0015-daemon-only-clean-break.md) — the architectural decision record that authorizes the P6 deletion sweep + collapse of the persistent root surface to nixlingd + nixling-priv-broker.
