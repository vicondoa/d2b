# Reference: broker privileged operation matrix

> Diataxis: reference. Stable catalog of every closed-enum operation
> the privileged broker (`d2b-priv-broker`) is allowed to perform
> on behalf of the unprivileged `d2bd` daemon. The wire-level
> source of truth is `d2b_contracts::BrokerRequest`; this page is the
> human-readable index keyed by operation name.

Every row carries three policy flags:

- **audit** — yes for the operations catalogued here; the broker
  writes one append-only JSON record per decision to
  `/var/lib/d2b/audit/broker-<utc-date>.jsonl`.
- **destructive** — `yes` for any operation whose audit decision can
  mutate persistent host state. Pure-`open` device handoffs are `no`
  because the broker only opens; the daemon owns the resulting fd.
- **secret** — `yes` for operations whose implementation reads secret
  material or whose audit record may reference secret-material
  identifiers. `redacted-only` rows carry only derived/redacted metadata:
  for example `GuestControlSign` records token-transcript metadata
  (`transcript_len`, `peer_cid_present`, `capabilities_hash_present`),
  and `UsbipBind` records normalized device identity plus serial HMAC
  correlations, never the per-VM token, signature bytes, raw serial, raw
  sysfs path, or device path.

Unknown variants and unknown fields in security-sensitive artifacts
are denied (`defaultForUnknown: deny`).

> **Authz-class vs system-group naming note.** The **Allowed groups**
> column below uses the broker's **authz class** identifiers
> `d2b-launcher` (singular) and `d2b-admin`, which are the
> classification outputs the **daemon's** authz layer produces per
> [ADR 0015](../adr/0015-daemon-only-clean-break.md). See also
> [naming-conventions.md § "Broker caller-role audit labels"](naming-conventions.md#broker-caller-role-audit-labels).
> The
> classification chain has two distinct sockets:
>
> 1. **Daemon public socket** at `/run/d2b/public.sock`
>    (owned `d2bd:d2b` mode `0660` per
>    `nixos-modules/host-daemon.nix`). The CLI user `connect(2)`s
>    here. The filesystem-permissions gate (group ownership + mode)
>    requires the connecting user to be in the `d2b`
>    Linux SYSTEM GROUP. At `accept(2)` time, `d2bd` reads the
>    peer's pid/uid/gid via `SO_PEERCRED` (`man 7 unix` — the Linux
>    SO_PEERCRED primitive returns ONLY pid+uid+gid, NOT
>    supplementary groups), then classifies the peer's uid via lookup
>    against the `launcherUsers` / `adminUsers` arrays in
>    `/etc/d2b/daemon-config.json` to produce one of three
>    authz-class outcomes: `d2b-launcher`, `d2b-admin`, or
>    `deny`. Membership in the `d2b` Linux system group
>    is the conventional way operators add users to `launcherUsers`;
>    the system group's mode-0660 connect gate is the filesystem-level
>    deny-by-default protection before `accept(2)` runs.
>
> 2. **Broker private socket** at `/run/d2b/priv.sock` (owned
>    `root:d2bd` mode `0660` per `nixos-modules/host-broker.nix`).
>    Only the `d2bd` uid can `connect(2)` here. The broker accepts
>    ONLY the `d2bd` peer; it does NOT directly classify launcher /
>    admin users. The daemon forwards the upstream authz-class
>    identifier to the broker with each per-op request; the broker
>    trusts the daemon's upstream classification and uses it as the
>    authz-class input to its per-op deny matrix in the table below.
>
> The authz classes named in the **Allowed groups** column
> (`d2b-launcher`, `d2b-admin`) are DISTINCT from the Linux
> SYSTEM GROUP `d2b` on the host: the system
> group gates **connect(2) reachability** to the daemon's public socket
> at the filesystem layer; the authz classes are the classification
> outputs the daemon produces by `launcherUsers`/`adminUsers` uid
> lookup and forwards to the broker. The authz-class name
> `d2b-launcher` shares a similar spelling for historical reasons,
> but it is an authz classification identifier, NOT a Linux group.

> **Relay and realm identity are not local authz principals.** The
> two-socket classification chain described above is the entirety of
> local lifecycle authorization: `SO_PEERCRED` + `d2b` group
> membership for daemon public-socket access, and `d2bd`-uid-only
> access for the broker private socket. Relay credentials used for
> remote daemon-access sessions are **node-management credentials**
> that authenticate the transport connection; they are never mapped
> to `d2b-launcher` or `d2b-admin` and never enter the
> `launcherUsers`/`adminUsers` uid lookup or the broker's authz
> chain. Realm or provider workload credentials are distinct from
> both: they remain in the exact credential-owning provider agent and are
> never placed on the host daemon or its config. Provider agents have no
> direct host-broker channel; only opaque, co-located credential leases may
> cross the provider contract.

> **Host shutdown exception.** The guarded host-shutdown hook is the only
> local lifecycle exception to the normal launcher/admin uid lookup. When
> systemd invokes `d2b host shutdown-hook --apply` from
> `d2bd.service` `ExecStop`, the daemon sees `SO_PEERCRED` uid `0` and
> classifies that connection as `HostShutdown`. That role is not an admin role:
> it is a narrow stop-only lifecycle authority for host shutdown and may issue
> only `vmStop`. It is denied for exec, USB attach/detach, host prepare/destroy,
> audit export, key rotation, config sync, and every other admin-only surface.
> The existing shutdown guard remains responsible for ensuring the hook exits
> without mutation during ordinary daemon restarts.

> **Configured launch is a public daemon operation, not a broker operation.**
> `launch` is authorized per workload/realm for the `d2b-launcher` and
> `d2b-admin` classes, is audited as a destructive runtime action, has no secret
> access, and does not require the privileged broker. It therefore appears in
> `PrivilegesJson.publicOperations` and the generated
> `OperationAuthz.operation` enum, but not in the broker-only catalog below.

## Operation catalog (PROTOCOL_VERSION = 2)

The currently implemented broker operation catalog. Every row carries
`audit: yes` and `defaultForUnknown: deny`.

| Variant | Subject | Scope | Status | Destructive | Secret access | Allowed groups | Audit | Default-for-unknown | Audit fields (in addition to common header) | Owner ADR |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `DelegateCgroupV2` | cgroup | global | live | no (chown only) | no | `d2b-admin` | yes | deny | `slice_path`, `controllers_enabled`, `owner_uid` | [0011](../adr/0011-cgroup-v2-delegation-and-pidfd-handoff.md) |
| `OpenCgroupDir` | cgroup | per VM / role | live | no | no | `d2b-launcher` + `d2b-admin` | yes | deny | `cgroup_id`, `path_class` | [0011](../adr/0011-cgroup-v2-delegation-and-pidfd-handoff.md) |
| `CgroupKill` | cgroup | per VM / role leaf | live | yes (writes `cgroup.kill` on a leaf) | no | `d2b-launcher` + `d2b-admin` | yes | deny | Request DTO (opaque IDs only): `vm_id`, `role_id`. Audit `operation_fields` (broker-derived after subject resolution): `cgroup_id`, `path_class` (one of `vm-role-leaf` / `host-scoped-leaf`; omitted on subject-resolution failure, which is recorded as `decision: denied-unknown` + `error_kind: unknown-subject` in the audit header). Refused with `cgroup-kill-on-ancestor-refused` if the resolved path's `path_class` is `slice` / `vm-interior`. The daemon invokes this op only as last-resort escalation after `pidfd_send_signal(SIGTERM)` does not drain the leaf within the role's grace period. | [0011](../adr/0011-cgroup-v2-delegation-and-pidfd-handoff.md) |
| `PrepareStateDir` | fs | global / per VM | live | yes (mkdir/chown/chmod) | no | `d2b-admin` | yes | deny | `base_dir_hash`, `vm_id_or_scope`, `created_paths_hash`, `mode`, `owner_uid`, `owner_gid`, `replace_or_create_result` | [0012](../adr/0012-w3-ipv6-off-sysctl-set-and-hash-ifname.md) |
| `PrepareRuntimeDir` | fs (`/run/d2b`) | global / per VM | live | yes (mkdir/chown/chmod) | no | `d2b-admin` | yes | deny | `base_dir_hash`, `vm_id_or_scope`, `created_paths_hash`, `mode`, `owner_uid`, `owner_gid`, `replace_or_create_result` | [0012](../adr/0012-w3-ipv6-off-sysctl-set-and-hash-ifname.md) |
| `ReconcileStorageScope` | fs (`storage.json`) | global / per VM / per role | live (static directories only for `apply`) | bounded (mkdir/chown/chmod for static directory specs only) | metadata-only | `d2b-admin` | yes | deny | `storage_ref`, `scope`, `kind`, `status`, `applied`, `path_hash` (no raw path) | [0034](../adr/0034-storage-lifecycle-restart-and-synchronization.md) |
| `ValidateLockSpec` | lock (`sync.json`) | global / per VM / per role | live (read-only) | no | metadata-only | `d2b-admin` | yes | deny | `lock_ref`, `scope`, `kind`, `cloexec_required`, `fd_passing_mechanism`, `order_key` | [0034](../adr/0034-storage-lifecycle-restart-and-synchronization.md) |
| `PrepareSwtpmDir` | fs (`${stateDir}/swtpm`) | per VM | live (`SpawnRunner` side-effect) | yes (mkdir/chown/chmod/setfacl + marker) | metadata-only | `d2b-admin` | yes | deny | `vm_id`, `base_dir_hash`, `result` (`created`/`reconciled`/`verified_clean`/`failed_closed`), `mode`, `owner_uid`, `owner_gid`, `marker_result` (`created`/`verified`/`failed_closed`), `fail_reason` (path-free slug, fail-closed only). NO raw `base_dir`/`tpm.sock`/state paths. | [0015](../adr/0015-daemon-only-clean-break.md) |
| `OpenKvm` | device | per role | live | no | no | `d2b-launcher` | yes | deny | `device_class`, `role_id` | [0014](../adr/0014-w3-modules-devices-runner-shape.md) |
| `OpenVhostNet` | device | per role | live | no | no | `d2b-launcher` | yes | deny | `device_class`, `role_id` | [0014](../adr/0014-w3-modules-devices-runner-shape.md) |
| `OpenFuse` | device | per role | live | no | no | `d2b-launcher` | yes | deny | `device_class`, `role_id` | [0014](../adr/0014-w3-modules-devices-runner-shape.md) |
| `OpenDevice` | device | per role | live | no | no | `d2b-launcher` | yes | deny | `device_class`, `role_id` | [0014](../adr/0014-w3-modules-devices-runner-shape.md) |
| `CreateTapFd` | network | per env / VM / TAP | live | possible (link create/destroy) | no | `d2b-admin` | yes | deny | `ifname_derived`, `role`, `flags_after`, `flags_diff` | [0012](../adr/0012-w3-ipv6-off-sysctl-set-and-hash-ifname.md) |
| `CreatePersistentTap` | network | per env / VM / TAP | live | possible (link create/destroy) | no | `d2b-admin` | yes | deny | `ifname_derived`, `role`, `flags_after`, `flags_diff` | [0012](../adr/0012-w3-ipv6-off-sysctl-set-and-hash-ifname.md) |
| `SetBridgePortFlags` | network | per env / VM / TAP | live | possible (flag flip) | no | `d2b-admin` | yes | deny | `ifname_derived`, `role`, `flags_after`, `flags_diff` | [0012](../adr/0012-w3-ipv6-off-sysctl-set-and-hash-ifname.md) |
| `ApplyNftables` | network host | global / per env | live | yes | no | `d2b-admin` | yes | deny | `table_hash_before`, `table_hash_after`, `coexistence_policy`, `manager_detected` | [0013](../adr/0013-w3-firewall-coexistence-policy.md) |
| `ApplyRoute` | routing | global / per env | live | yes | no | `d2b-admin` | yes | deny | `route_key`, `route_diff` | [0012](../adr/0012-w3-ipv6-off-sysctl-set-and-hash-ifname.md) |
| `ApplySysctl` | sysctl | per link / global | live | yes | no | `d2b-admin` | yes | deny | `sysctl_key`, `value_before`, `value_after` | [0012](../adr/0012-w3-ipv6-off-sysctl-set-and-hash-ifname.md) |
| `ApplyNmUnmanaged` | name resolution / NM | per ifname | live | yes | no | `d2b-admin` | yes | deny | `nm_file_path_hash`, `ifname_set` | [0012](../adr/0012-w3-ipv6-off-sysctl-set-and-hash-ifname.md) |
| `UpdateHostsFile` | name resolution | global | live | yes | no | `d2b-admin` | yes | deny | `managed_block_hash_before`, `managed_block_hash_after` | [0012](../adr/0012-w3-ipv6-off-sysctl-set-and-hash-ifname.md) |
| `BindUnixSocket` | socket | per VM / role | reserved | partial (replace stale only) | no | `d2b-admin` | yes | deny | `socket_path_hash`, `mode`, `acl_diff` | [0011](../adr/0011-cgroup-v2-delegation-and-pidfd-handoff.md) |
| `SetSocketAcl` | socket | per VM / role | reserved | partial (replace stale only) | no | `d2b-admin` | yes | deny | `socket_path_hash`, `mode`, `acl_diff` | [0011](../adr/0011-cgroup-v2-delegation-and-pidfd-handoff.md) |
| `RevokeSocketAclIfPresent` | socket | per VM / role | future work | yes (revoke) | no | `d2b-admin` | yes | deny | `socket_path_hash`, `groups_revoked`, `acl_diff` | [0018](../adr/0018-microvm-nix-removal.md) |
| `ModprobeIfAllowed` | kernel module | global / feature | live | yes | no | `d2b-admin` | yes | deny | `module_name`, `matrix_entry_id`, `modules_disabled_sysctl` | [0014](../adr/0014-w3-modules-devices-runner-shape.md) |
| `UsbipBind` | USBIP device routing | per busid / env | live | yes (driver bind + backend ACL grant) | redacted-only | `d2bd` | yes | deny | Request DTO (opaque IDs only): `bundle_usbip_bind_intent_ref`. Audit `operation_fields` (broker-derived after bundle resolution): `bus_id`, `vm`, optional `device_identity` with normalized VID/PID, `serial_observed`, and HMAC serial correlations; no raw sysfs path, serial, or device path. Refused before exposure if the selected physical device fails the bundle vendor/product or topology policy. | [0018](../adr/0018-microvm-nix-removal.md) |
| `UsbipUnbind` | USBIP device routing | per busid / env | live | yes (driver unbind + backend ACL revoke + optional host-session claim release) | no | `d2bd` | yes | deny | Request DTO (opaque IDs only): `bundle_usbip_bind_intent_ref`, `preserve_durable_claim`. Audit `operation_fields` (broker-derived after bundle resolution): `bus_id`; no raw sysfs path or topology. VM stop/restart sets `preserve_durable_claim = true`; explicit detach releases the matching broker-owned session claim after successful unbind. | [0018](../adr/0018-microvm-nix-removal.md) |
| `UsbipProxyReconcile` | USBIP proxy / backend ACL reconciliation | host / per env | live | yes (backend ACL reconciliation; no bind/unbind) | no | `d2bd` | yes | deny | Request DTO: `scope_id`. Audit `operation_fields`: `{}`; `subject_id` / `scope_id` carry the bounded reconcile scope, while per-busid expectations are re-derived from the trusted bundle. | [0018](../adr/0018-microvm-nix-removal.md) |
| `UsbipBindFirewallRule` | USBIP firewall | per busid | live | yes (applies nftables carve-out / live routing exposure) | no | `d2bd` | yes | deny | Request DTO (opaque IDs only): `bundle_usbip_firewall_intent_ref`. Audit `operation_fields`: `bundle_usbip_firewall_intent_ref`; `subject_id` carries the resolved busid and `scope_id` is `usbip-firewall`. The broker derives busid, source/destination scoping, and the nft batch hash from the trusted bundle before applying the `inet d2b` carve-out. | [0013](../adr/0013-w3-firewall-coexistence-policy.md), [0018](../adr/0018-microvm-nix-removal.md) |
| `QemuMediaEnroll` | qemu-media registry | per VM / media ref | live | yes (root-only registry + runtime udev ignore rules) | redacted-only | `d2b-admin` | yes | deny | `vm`, `media_ref`, `read_only`, `udev_rule_written`, `udev_reloaded`; no busid, by-id path, serial, or block path | [0015](../adr/0015-daemon-only-clean-break.md) |
| `QemuMediaRefreshRegistry` | qemu-media redacted registry | host | live | yes (redacted index + runtime udev ignore rules) | redacted-only | `d2b-admin` | yes | deny | `record_count`, `redacted_index_written`, `udev_rule_written`, `udev_reloaded`; no busid, by-id path, serial, block path, or registry path | [0015](../adr/0015-daemon-only-clean-break.md) |
| `QemuMediaAttach` | qemu-media hotplug | per VM / media ref | live | yes (live QMP media attach) | redacted-only | `d2b-admin` | yes | deny | `vm`, `media_ref`, `slot`, `read_only`, `qmp_commands`; no busid, by-id path, serial, or block path | [0015](../adr/0015-daemon-only-clean-break.md) |
| `QemuMediaBoot` | qemu-media boot media | per VM / media ref | live | yes (live QMP boot attach + continue) | redacted-only | `d2b-admin` | yes | deny | `vm`, `media_ref`, `slot`, `read_only`, `qmp_commands`; no busid, by-id path, serial, or block path | [0015](../adr/0015-daemon-only-clean-break.md) |
| `QemuMediaSystemPowerdown` | qemu-media lifecycle | per VM | live | yes (QMP system_powerdown) | redacted-only | `d2b-admin` | yes | deny | `vm`, `qmp_command`; no raw QMP response, socket path, guest output, busid, by-id path, serial, or block path | [0015](../adr/0015-daemon-only-clean-break.md), [0040](../adr/0040-graceful-vm-shutdown.md) |
| `QemuMediaQueryStatus` | qemu-media lifecycle status | per VM | live | no (QMP query-status) | redacted-only | `d2b-admin` | errors only | deny | `vm`, `shutdown_context`, typed status enum; suppresses success audit during polling and never records raw QMP JSON | [0040](../adr/0040-graceful-vm-shutdown.md) |
| `QemuMediaQuit` | qemu-media lifecycle | per VM | live | yes (QMP quit) | redacted-only | `d2b-admin` | yes | deny | `vm`, `qmp_command`; no raw QMP response, socket path, guest output, busid, by-id path, serial, or block path | [0015](../adr/0015-daemon-only-clean-break.md), [0040](../adr/0040-graceful-vm-shutdown.md) |
| `QemuMediaDetach` | qemu-media hotplug | per VM / media ref | live | yes (live QMP media detach) | redacted-only | `d2b-admin` | yes | deny | `vm`, `media_ref`, `slot`, `read_only`, `qmp_commands`; no busid, by-id path, serial, or block path | [0015](../adr/0015-daemon-only-clean-break.md) |

## Public USB operations

The public CLI/API rows in `privileges.json` are separate from the
daemon-to-broker rows above. The daemon enforces public authz before any
broker request is emitted.

| Public operation | Status | Destructive | Secret access | Allowed groups | Broker/audit path |
| --- | --- | --- | --- | --- | --- |
| `usb attach` | live | yes (host-session claim, driver bind, backend ACL, firewall/proxy convergence, guest import) | redacted-only (the broker-side bind may read the USB audit serial-HMAC key and records only redacted correlations) | `d2b-admin` | Emits public `usb attach` audit plus broker `UsbipBind`, `UsbipBindFirewallRule`, `SpawnRunner`/`OpenPidfd` as needed, and `UsbipProxyReconcile`; `UsbipBind.operation_fields` carries `bus_id`, `vm`, and optional redacted `device_identity`. |
| `usb detach` | live | yes (guest detach, firewall withdrawal/targeted stream cleanup when proven, then driver unbind, backend ACL revoke, optional host-session claim release) | no | `d2b-admin` | Emits public `usb detach` audit plus broker `UsbipUnbind` and related lifecycle rows. `UsbipUnbind.operation_fields` carries only the resolved `bus_id`; `preserve_durable_claim` is true for VM stop/restart cleanup and false for explicit detach. |
| `usb probe` | live diagnostic | no device-routing mutation (qemu-media redacted registry refresh and USBIP backend-ACL validation may run; it does not bind, unbind, import, detach, or release claims) | no | `d2b-launcher` + `d2b-admin` | Emits public `usb probe` audit; when USBIP intents exist it calls `UsbipProxyReconcile` with `scope_id = "host"` before rendering [`cli-output/usb-probe.md`](cli-output/usb-probe.md). |

## Utility and bootstrap variants

| Variant | Subject | Scope | Status | Destructive | Secret | Allowed groups | Audit | Default-for-unknown |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `ValidateBundle` | bundle | global | live | no | no | `d2b-launcher` + `d2b-admin` | yes | deny |
| `ExportBrokerAudit` | audit log | global | live | no (read-only export) | no | `d2b-admin` | yes | deny |
| `CreateOrReconcileUsersGroups` | user/group | global | bootstrap-only | yes | no | `d2b-admin` | yes | deny |

## Lifecycle variants

| Variant | Subject | Scope | Status | Destructive | Secret access | Allowed groups | Audit | Default-for-unknown | Audit fields (in addition to common header) |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `PrepareStoreView` | fs (store view) | per VM | live | yes | no | `d2b-launcher` + `d2b-admin` | yes | deny | `generation`, `hardlink_farm_path`, `target_view_path` |
| `StoreSync` | fs (hardlink farm) | per VM | live | yes (atomic `current` symlink swap) | no | `d2b-launcher` + `d2b-admin` | yes | deny | `bundle_closure_ref`, `generation`, `closure_count`, `hardlink_farm_path` |
| `GuestControlSign` | guest-control token | per VM | live | no | redacted-only | `d2bd` | yes | deny | `vm_id`, `role`, `purpose`, `transcript_len`, `peer_cid_present`, `capabilities_hash_present` |
| `SetupMountNamespace` | mount ns | per VM / role | live | partial (mount-root prep + bind target) | no | `d2b-launcher` + `d2b-admin` | yes | deny | `role_id`, `mount_root`, `mount_view_path`, `source_view_path` |
| `DeregisterRunnerPidfd` | process registry | per VM / role | live | no | no | `d2b-launcher` + `d2b-admin` | yes | deny | `vm_id`, `role_id`, `removed` |
| `DiskInit` | disk image | per VM | live | yes (create/format declared images; repair safe declared posture drift) | no | `d2bd` | yes | deny | `vm_id`, `ops_total`, `ops_created`, `ops_skipped`, `ops_repaired`, `ops_posture_repaired`, `target_paths_hash` |

## Variants reserved on the wire (`unknown-operation`)

These variants are present in the `BrokerRequest` enum so the wire
protocol stays stable, but the current broker dispatches them to an
`unknown-operation` refusal with an audit record.

| Variant | Subject | Current status | Destructive | Secret access | Notes |
| --- | --- | --- | --- | --- | --- |
| `LaunchMinijailChild` | process | future work | yes (fork+exec) | no | Handler work is tied to minijail provisioning and runner launch. |
| `ReadSecretById` | secret store | future work | no (read-only) | yes | Secret backend support is not implemented. |
| `InjectSecretById` | secret store | future work | yes | yes | Secret injection into VM payloads is not implemented. |
| `RotateSecretById` | secret store | future work | yes | yes | Secret rotation is not implemented. |
| `PauseBroker` | broker admin | future work | partial (state transition) | no | Broker pause tooling is not implemented. |
| `ResumeBroker` | broker admin | future work | partial (state transition) | no | Broker resume tooling is not implemented. |

The wire goldens under `tests/golden/broker-wire/` cover one canonical
encoding per reserved variant so handlers can be added without
breaking wire compatibility. The canonical machine-readable source for
this catalog is the JSON schema under
[`docs/reference/schemas/v2/privileges.json`](schemas/v2/privileges.json).
The `v1` schema remains in tree as the frozen baseline; consumers
should validate against `v2`. The markdown above is the human-readable
index.

## Audit record schema

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
- `DeregisterRunnerPidfd { vm_id, role_id }`
- `SpawnRunner { bundle_runner_intent_ref, vm_id, role_id, role, runtime_allocations }`
  — when the runner is the cloud-hypervisor role of a guest-control VM,
  spawning it also grants the unprivileged `d2bd` daemon uid a
  narrow ACL on the per-VM vsock transport (traversal `--x` on the
  non-public state-dir components, `rw` on `vsock.sock`) so the daemon
  can drive the authenticated guest-control bridge. The grant runs as a
  revoke-then-grant at each cloud-hypervisor (re-)spawn: any stale
  daemon grant left on a replaced/disabled socket inode is revoked
  first, then the traversal chain and the live-socket `rw` grant are
  (re-)established, so a disabled or replaced socket cannot retain a
  stale daemon grant. (A dedicated stop-time teardown revoke hook is
  future work: `SignalRunner` carries no socket path.) This ACL is
  a SpawnRunner **side-effect**, not a separate broker op. Both the grant
  and the revoke emit a hash-only audit record carrying `target_class`
  (`state-dir`, `ancestor`, or `vsock-socket`), the `daemon_principal`,
  and `acl_diff_hash` + `result` — never the raw socket / state-dir path.
  A **second** SpawnRunner side-effect on the same guest-control
  cloud-hypervisor spawn grants the cloud-hypervisor **runner** uid connect
  access to the cross-principal `d2b-gctl` token fs-share socket: `--x`
  traversal on the per-VM `guest-control` dir and `rw` (with an explicit
  `m::rw` mask) on the `gctlfs`-owned `d2b-gctl` socket inode, scoped to the
  live `(dev, ino)` (re-fstat after the fd-based setfacl) and retried until
  the socket is bound. This lifts the 0700 socket's `mask::---`, which
  otherwise masks out the inherited `default:u:<ch_uid>` grant and EACCESes
  cloud-hypervisor's vhost-user connect. It emits its own hash-only audit
  record carrying `target_class` (`gctlfs-dir` or `gctlfs-socket`), the
  `consumer_principal` (`cloud-hypervisor-runner`), and `acl_diff_hash` +
  `result` — never the raw socket / state-dir path or a uid-by-value.
- `RunHostInstall { bundle_installer_intent_ref, enable, start, no_start }`
- `RunMigrate { bundle_migrate_intent_ref }`
- `RunActivation { bundle_activation_intent_ref, mode, vm }`
- `RunGc { bundle_gc_intent_ref, keep_generations }`
- `RunKeysRotate { bundle_keys_intent_ref, vm }`
- `RunHostKeyTrust { bundle_trust_intent_ref, vm }`
- `RunRotateKnownHost { bundle_rotate_known_host_intent_ref, vm }`
- `UsbipBind { bus_id, vm, device_identity? }`
- `UsbipUnbind { bus_id }`
- `UsbipProxyReconcile {}`
- `UsbipBindFirewallRule { bundle_usbip_firewall_intent_ref }`
- `DiskInit { vm_id, ops_total, ops_created, ops_skipped, ops_repaired, ops_posture_repaired, target_paths_hash }`

### Runner roles (selected via `SpawnRunner.role`)

Each entry is a `RunnerRole` enum variant the broker dispatches to a
pure argv generator in `d2b_host`. Per-role minijail profile +
seccomp policy are listed below.

| Runner role | Replaces | Caps | Notes |
| --- | --- | --- | --- |
| `OtelHostBridge` | singleton `d2b-otel-host-bridge.service`; the current surface is broker `SpawnRunner{role: OtelHostBridge, …}` | empty | The bundle's `OtelHostBridge` runner intent MUST point at a VM whose `vm_name` equals `manifest._observability.vmName`; the broker refuses fail-closed via `Broker.OtelHostBridgeIntentInvalid` otherwise. Pre-opened vsock fds only; `AF_VSOCK` / `AF_UNIX` socket(2) is denied by `w1-otel-host-bridge` seccomp policy. Bind set: d2b OTel runtime dir `/run/d2b/otel` (RW), obs VM CH vsock host UDS dir (connect), and `/run/d2b/otel/host-egress.sock` (RW listen target). No `/dev` binds. Host-scoped profile `host-otel-host-bridge` (principal: `d2b-otel-bridge`, cgroup subtree: `d2b.slice/host/otel-host-bridge`). |
| `VsockRelay` | per-VM observability relay; the current surface is broker `SpawnRunner{role: VsockRelay, …}` | empty | Before spawn, the broker resolves the VM state directory from the trusted manifest and checks the bundle-resolved `UNIX-LISTEN` endpoint. It accepts only a direct child named `vsock.sock_<decimal-port>`, refuses active listeners, symlinks, non-sockets, and out-of-scope paths, and removes a proven stale socket so a normal VM restart can rebind the relay. This cleanup is a `SpawnRunner` side-effect, not a separate broker op. |


Sensitive path components are stable-hashed; user identity is stored
only as numeric `uid`/`gid` + the authz class; raw secrets are never
stored. Retention is daily rotation + a 14-day default deletion,
overridable via `d2b.site.audit.retentionDays`.
The broker prunes daily-rotated files older than the configured
retention window. If `d2b.site.audit.retentionDays` is unset, the
broker defaults to 14 days. See [`daemon-api.md`](daemon-api.md#audit)
"Retention" for the prune contract.

## Per-role device bind matrix

This section pins, per per-VM runner role, the closed-set device-node
bind list the broker opens via `OpenKvm` / `OpenVhostNet` / `OpenFuse`
/ `OpenDevice` on behalf of the runner. Every entry is grounded in the
`DeviceClass` taxonomy (`packages/d2b-host/src/devices.rs`); the
broker refuses to open any path absent from the role's bundle row, and
the per-role minijail profile (`nixos-modules/minijail-profiles.nix`)
declares the bind set via `mountPolicy.deviceBinds` so the runner's
mount namespace cannot see anything outside it.

### Per-role file-creation mask (umask)

The per-role minijail profile includes an optional
`umask: Option<u32>` field. The broker child closure calls `umask(2)`
with the role's mask immediately before `execve(2)`. Default is
`None` (inherit the broker's umask). Roles that bind shared Unix
sockets declare `umask = 0o007` so the resulting sockets get mode
0660:

| Role | umask | rationale |
| --- | --- | --- |
| `swtpm` (long-lived TPM sidecar) | `0o007` | `/run/d2b/vms/<vm>/tpm.sock` mode 0660 so cloud-hypervisor (named-user ACL grant) can connect via mask:rw |
| `audio` (vhost-user-sound sidecar) | `0o007` | `/run/d2b/vms/<vm>/snd.sock` mode 0660 — same rationale |
| `gpu` (crosvm-device-gpu sidecar) | `0o007` | `/run/d2b/vms/<vm>/gpu.sock` mode 0660 — same rationale |
| `video` (crosvm video-decoder sidecar) | `0o007` | `/run/d2b-video/<vm>/video.sock` mode 0770 with named-user ACL grants limited to the video and cloud-hypervisor UIDs |
| `cloud-hypervisor` (long-lived runner) | (none) | CH reads from these sockets; it does not bind any of its own that need ACL-mediated access |
| `virtiofsd` (per-share sidecar) | (none) | virtiofsd's `--sandbox=chroot` plus broker-pre-NS user-namespace handle access control; no shared sockets to harden |

The broker enforces `umask <= 0o777` and exits with
`CHILD_EXIT_INVALID_UMASK=75` for out-of-range values. The umask
syscall is invoked AFTER setuid/setgid/cap-drop/seccomp but BEFORE
execve, so the new process image inherits the configured mask.

### Gpu role

| Device | `DeviceClass` | Rationale |
| --- | --- | --- |
| `/dev/kvm` | `Kvm` | crosvm-gpu shares the runner's KVM fd for hypervisor coupling. |
| `/dev/dri/renderD128` | `Dri` | virgl/venus/cross-domain Wayland render node; carries the full `DRM_IOCTL_VIRTGPU_*` family (`GET_CAPS`, `CONTEXT_INIT`, `RESOURCE_CREATE`, `RESOURCE_CREATE_BLOB`, `SUBMIT_CMD`, `EXECBUFFER`, `WAIT`, `MAP`, `GETPARAM`) per `d2b_host::ioctl_policy::class_ioctls(DeviceClass::Dri)`. |
| `/dev/nvidiactl` | `NvidiaCtl` | NVIDIA control device — required for the Quadro T1000 driver context. |
| `/dev/nvidia0` | `NvidiaRender` | NVIDIA per-card primary device. The correct host path is `/dev/nvidia<N>` per the proprietary driver UAPI; `DeviceClass::default_path` uses `/dev/nvidia0`. |
| `/dev/nvidia-uvm` | `NvidiaUvm` | Unified-memory driver path used by VA-API NVDEC and Vulkan compute. |
| `/dev/udmabuf` | `Udmabuf` | Cross-domain dmabuf wrap path: cross-domain Wayland requires `UDMABUF_CREATE`/`UDMABUF_CREATE_LIST` to expose guest framebuffers to the host compositor without copy. `DeviceClass::Udmabuf` covers this device class. |

In addition to the six device binds, the Gpu role's minijail profile
carries a single bind-mount mapping the host's per-user Wayland
socket into the role-local runtime dir so the runner can never
traverse `/run/user/<uid>`:

```
mountPolicy.bindMounts = [
  { src = "/run/user/<waylandUser-uid>/wayland-0";
    dst = "/run/d2b-gpu/<vm>/wayland-0"; }
];
```

- Caps: **empty**. Per-role smoke proves no `CAP_SYS_NICE` is needed
  at runtime.
- `seccompPolicyRef`: `w1-gpu` (closed-set syscall + ioctl allowlist
  derived from the device-bind set).
- `cgroupPlacement.subtree`: `d2b.slice/<vm>/gpu`.
- Validator: `tests/minijail-validator-gpu.sh` (positive
  `DRM_IOCTL_VIRTGPU_GET_CAPS` arm + negative `ptrace` arm; evidence
  at `/var/lib/d2b/validated/p1-gpu.json`; hardware smoke on the
  host's Quadro T1000).
- Byte-parity golden: `tests/golden/runner-shape/gpu-argv-minimal.txt`
  via `d2b_host::gpu_argv::generate_gpu_argv`.

## Additional broker ops

This section documents broker ops used by the daemon-side host-prep
DAG and the daemon-side preflights that run beside them. The current
daemon-only surface is `d2bd.service` +
`d2b-priv-broker.{service,socket}` + per-VM runners spawned via
broker `SpawnRunner`.

`StoreSync` is live. `SshKeygenProbe` remains documented as future
work and is not part of the current `d2b_contracts::BrokerRequest` /
`packages/d2b-core/src/privileges.rs` surface.

> Host-prep composes the existing broker ops `CreateTapFd` /
> `CreatePersistentTap` (with `TUNSETOWNER`/`TUNSETGROUP` matching the
> exact runner uid/gid, including the graphics-VM `d2b-<vm>-gpu`
> owner), `SetBridgePortFlags`, `OpenDevice`, `ApplyNmUnmanaged`, and
> `ApplySysctl` in a fixed order. This catalog does not define separate
> `BringUpTapInterface`, `SeedDnsmasqLease`, or `PreOpenVhostNetFd`
> reference ops.

### Additional broker ops

| Variant | Subject | Scope | Status | Destructive | Secret access | Allowed groups | Audit | Default-for-unknown | Audit fields (in addition to common header) |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `StoreSync` | fs (per-VM hardlink farm + store-meta) | per VM / per generation | live | yes (mkdir, chown, chmod, hardlink, gcroot symlinks, `current` symlink swap, `.marker` write) | no | `d2b-admin` (host install/apply path) + `d2b-launcher` (per-VM switch path) | yes | deny | `vm_id`, `generation`, `closure_hash`, `hardlink_farm_path_hash`, `store_meta_path_hash`, `gcroots_diff_hash`, `marker_present`, `acl_propagation_guard_result` |
| `SshKeygenProbe` | per-VM ssh control socket | per VM | future work | no (read-only key fingerprint probe) | no | `d2b-launcher` + `d2b-admin` | yes | deny | `vm_id`, `socket_path_hash`, `keytype`, `fingerprint_hash`, `probe_result` |

### Per-operation cap usage

| Operation | Capabilities used | Notes |
| --- | --- | --- |
| `StoreSync` | `CAP_FOWNER`, `CAP_DAC_OVERRIDE`, `CAP_SYS_ADMIN` (store-view farm-build subprocess only) | `CAP_FOWNER` for `fchmod`/`fchown` across the per-VM hardlink farm (broker runs as root but mutates `<vm>/store{,-meta}/` which is `d2bd:users` 2770 `g+s`, so chown semantics require FOWNER); `CAP_DAC_OVERRIDE` for write under the trusted root-owned `/var/lib/d2b/vms/<vm>/` ancestor. The broker MUST NOT call `setfacl --recursive` across the hardlink farm — that would propagate ACLs back into `/nix/store` paths through the hardlinks. The `acl_propagation_guard_result` audit field records the explicit "no recursive setfacl crossed into /nix/store" check. **Store-view farm build:** on stock NixOS `/nix/store` is bind-mounted on itself, so a same-`st_dev` cross-vfsmount `link(2)` returns `EXDEV`. When an in-process build hits that case, the broker rebuilds the farm in a private mount namespace via a `d2b-activation-helper build-store-view-farm` subprocess launched as `unshare --mount --propagation private` + lazy `umount /nix/store` (`CAP_SYS_ADMIN`-scoped, namespace-local — the host `/nix/store` is never unmounted). Same-filesystem hosts never enter this path. |
| `SshKeygenProbe` | — | empty bounding set: the op runs `ssh-keygen -F` / `-l` style fingerprint probes against the per-VM ssh control socket only. The broker dispatcher binds the probe target to `<vm>/sshd-host-keys/ssh_host_*_key.pub` derived from the bundle-pinned VM identity; no host-wide ssh-keygen surface is exposed. No `CAP_NET_*` because the probe runs over the pre-opened per-VM UDS, never a network socket. |

### HostPrep DAG — composition of existing broker ops

The host-prep DAG executes daemon-side per VM start and dispatches
broker ops in a fixed canonical order. Each row below is a **DAG
node**, not a new broker variant. The `Legacy unit replaced` column
records the former systemd template that no longer exists.

| DAG node | Legacy unit replaced | Broker op(s) called | Ordering constraint |
| --- | --- | --- | --- |
| `host-prep.nm-unmanaged` | — (daemon-managed carry-over) | `ApplyNmUnmanaged` | first — must precede tap create so NetworkManager does not claim the iface mid-creation |
| `host-prep.tap` | `microvm-tap-interfaces@<vm>.service`; replaced by `CreateTapFd` / `CreatePersistentTap` broker dispatch in this DAG node | `CreateTapFd` (fd handoff path) **or** `CreatePersistentTap` (with `TUNSETOWNER`/`TUNSETGROUP` set to the runner uid/gid — graphics VMs MUST use the `d2b-<vm>-gpu` uid, NOT `microvm`) | after `host-prep.nm-unmanaged`, before `host-prep.sysctl` |
| `host-prep.sysctl` | — (daemon-managed carry-over) | `ApplySysctl` (per-link IPv6-off + MTU) | after `host-prep.tap`, before `host-prep.bridge` |
| `host-prep.bridge` | `microvm-tap-interfaces@<vm>.service` (bridge-port subset); replaced by `SetBridgePortFlags` broker dispatch | `SetBridgePortFlags` | after `host-prep.sysctl`, before `host-prep.spawn` |
| `host-prep.pci-devices` | `microvm-pci-devices@<vm>.service`; replaced by `OpenDevice` broker dispatch | `OpenDevice` | parallel with `host-prep.tap` chain; joins before `host-prep.spawn` |
| `host-prep.store-sync` | `d2b-<vm>-store-sync.service` + activation-time `d2b-store-sync` call from `store.nix`; replaced by `StoreSync` broker dispatch | `StoreSync` (live) | before any per-VM runner spawn; for the host-install/apply path, runs as part of `host install --apply` |
| `host-prep.known-hosts-refresh` | `d2b-known-hosts-refresh@<vm>.service`; current replacement is the planned `SshKeygenProbe` broker dispatch | `SshKeygenProbe` (future work) | after `vm.guest-control-health`, not in the cold-start chain |
| `vm.set-booted` | `microvm-set-booted@<vm>.service`; replaced by pure-daemon `supervisor::state::record_booted(<vm>, <closure>)` (no broker op) | — (pure daemon: `supervisor::state::record_booted(<vm>, <closure>)`) | after runner reports ready; no broker call |
| `host-prep.spawn` | — (final join) | `SpawnRunner` | after every preceding `host-prep.*` node completes; carries SCM_RIGHTS handoff of fds from `CreateTapFd` / `OpenDevice` / `OpenKvm` / etc. |

The daemon's `vfsd-watchdog` replacement is purely
`supervisor::pidfd` watching the virtiofsd pidfd and re-issuing
`SpawnRunner` on exit; the legacy
`d2b-vfsd-watchdog@<vm>.{timer,service}` pair no longer exists.

### Preflights (daemon-side, no broker call)

The daemon refuses to start a VM if any of the following preflights
fail. Each runs against `<vm>/`'s on-disk state with `O_NOFOLLOW` and
no privileged ops; they are documented here for trust-boundary
completeness, not because they are broker ops.

| Preflight | Subject | Capabilities | Refusal envelope | Notes |
| --- | --- | --- | --- | --- |
| `OwnershipMatrixCheck` | `/var/lib/d2b/vms/<vm>/` ownership matrix | — (pure `fstatat` traversal; the daemon already has `CAP_DAC_READ_SEARCH` for its state dir, no new caps) | refuses VM start with typed `daemon.ownership-matrix-drift` envelope citing the first drifted leaf (path, expected `owner:group mode`, observed) | Checks `/var/lib/d2b/vms/<vm>/` owner/group/mode invariants before start. |
| `SshHostKeyPreflight` | `<vm>/sshd-host-keys/ssh_host_*_key` | — (`O_NOFOLLOW` `openat`, `fstat`) | refuses VM start with typed `daemon.ssh-host-key-drift` envelope on: symlink, owner/group != root, mode != `0400` | Ensures host keys are regular root-owned `0400` files. |
| `DnsmasqLeaseHashPreflight` (net VMs only) | `${dnsmasq_dir}/<env>.conf` (default `/var/lib/d2b/dnsmasq/<env>.conf`) vs bundle `hosts_intent` + `route_intent[env:<env>:*]` + `nft_intent[env:<env>]` | — (pure `read()` + SHA-256; the daemon already has read access to its state dir) | refuses net-VM start with typed `daemon.bundle-dnsmasq-drift` envelope (exit code `63`); covers `EnvMissing`, `ConfigMissing`, `ConfigReadFailed`, `HashMismatch`. **Remediation**: re-render `dnsmasq.conf` and retry, or run `nixos-rebuild switch` (the standalone `d2b host prepare --apply` recovery path is not yet wired — it returns `daemon-down` (exit 1) today). See [`docs/reference/net-vm-bundle-gate.md`](./net-vm-bundle-gate.md). | Compares the rendered dnsmasq config to the bundle's host, route, and nft intents. |
| `HostModuleMatrixPreflight` | trusted host kernel modules: `kvm_intel`/`kvm_amd`, `vhost`, `vhost_vsock`, `vhost_net`, `tun`, `bridge`, `nf_tables`, `nf_conntrack`, plus per-env `usbip-host` | — (reads `/proc/modules`) | refuses VM start with `daemon.host-module-missing` envelope; remediation suggests `ModprobeIfAllowed` (broker op, separate path) | Reads `/proc/modules` and checks the trusted host module set before start. `virtio_media` is a guest driver for video-enabled VMs and is validated in the guest closure, not in host `/proc/modules`. |

The four preflights run in fixed order on every `d2b vm start
<vm> --apply`; `OwnershipMatrixCheck` runs first so a partially-
migrated host surfaces drift before any other check touches the VM
state.

### Cross-references

- `tests/restart-policy-eval.sh` — asserts daemon-equivalent behavior
  instead of legacy per-VM unit behavior.
- `tests/processes-json-drift.sh` — asserts no `d2b-<vm>-*` or
  `microvm-*@<vm>` references remain in `processes.json`.
- `tests/store-marker-eval.sh` — `<vm>/store-meta/.marker` presence
  regression gate (called from `StoreSync` audit).
- AGENTS.md "Critical subsystems — handle with care" rows for per-VM
  `/nix/store` hardlink farm and TPM state — `StoreSync` MUST honor
  both invariants.

## Broker-dispatch contracts

This section documents the broker-dispatch contracts for runner roles
and daemon-emitted telemetry. The underlying broker variants
(`SpawnRunner`, `ApplyNftables`, `ApplyRoute`, `SetBridgePortFlags`,
`ModprobeIfAllowed`, etc.) are already defined and do not change wire
shape.

### Per-runner-role dispatch contracts

These rows extend the runner-role registry in the "Runner roles
(selected via `SpawnRunner.role`)" table above. The cap matrix
continues to be sourced from the "Per-role minijail profile" table;
this section pins the **broker-dispatch contract** that the broker
enforces fail-closed before fork/exec.

| Runner role | Legacy unit replaced | Caps (steady-state) | Per-env scope | Broker-dispatch contract |
| --- | --- | --- | --- | --- |
| `OtelHostBridge` | `d2b-otel-host-bridge.service`; replaced by broker `SpawnRunner{role: OtelHostBridge, …}` | empty | host-scoped (singleton — exactly one runner per host) | Broker refuses `SpawnRunner{role: OtelHostBridge, …}` fail-closed via `Broker.OtelHostBridgeIntentInvalid` unless the bundle's `OtelHostBridge` runner intent points at a VM whose `vm_name` equals `manifest._observability.vmName`. Readiness gate: `/run/d2b/otel` exists with expected ownership, stale `host-egress.sock` is removed, and the obs VM base `vsock.sock` exists; exponential backoff applies on host-OTLP unreachable. Broker waits for the readiness gate before exec; `supervisor::pidfd` respawns on relay exit. Pre-opened vsock fds only; `socket(AF_VSOCK)` is denied by `w1-otel-host-bridge` seccomp. |
| `Usbip` | per-env singletons `d2b-sys-<env>-usbipd-{proxy,backend}.{service,socket}`; replaced by broker `SpawnRunner{role: Usbip, vm_id: sys-<env>-usbipd, …}` | backend: scoped host-root carve-out with `CAP_NET_RAW`; proxy: empty | **per env** — two runners per USBIP-enabled env (`vm_id` = `sys-<env>-usbipd`, roles `backend` and `proxy`) | `d2b usb attach --apply` dispatches `UsbipBind` with a bundle-resolved bind intent first so broker allowlist validation and the per-busid lock succeed before any listener is exposed, then applies `UsbipBindFirewallRule`, ensures the per-env backend (`usbipd -4 --tcp-port <backendPort>`) and bounded proxy (`socat TCP-LISTEN:3240,bind=<env.hostUplinkIp>,fork,max-children=4,reuseaddr ...`) are spawned and TCP-ready, then runs `UsbipProxyReconcile`. Host kernel module is `usbip-host` (not `vhci_hcd`, which is the guest module). |
| `CloudHypervisor` (guest-control VM) | `d2b@<vm>.service` runner; replaced by broker `SpawnRunner{role: CloudHypervisor, vm_id: <vm>, …}` | empty | per VM | On a guest-control-enabled VM the broker, as a SpawnRunner **side-effect**, grants the unprivileged `d2bd` daemon uid a minimal ACL on the per-VM vsock transport so the daemon can run the authenticated readiness/config-read bridge: traversal `--x` on every non-public component of the per-VM state dir and `rw` on `vsock.sock`. The grant is scoped to the **current** socket inode/dev (the broker re-fstats after the fd-based setfacl and aborts+retries if the path is replaced mid-grant), grants **only** that single daemon uid (no `g:`, no default, no blanket entry), and retries until bound while cloud-hypervisor finishes creating the socket. It runs as a revoke-then-grant at each cloud-hypervisor (re-)spawn — any stale daemon grant on a replaced/disabled socket inode is revoked first — so a disabled or replaced socket cannot retain a stale daemon grant; a dedicated stop-time teardown revoke hook is future work (`SignalRunner` carries no socket path). The shared traversal `--x` grants on non-public ancestors are retained, since the daemon also needs them for the per-VM api-socket and sibling VMs depend on them. Both grant and revoke emit hash-only audit records (`target_class` ∈ {`state-dir`, `ancestor`, `vsock-socket`}, `daemon_principal`, `acl_diff_hash`, `result`); raw socket / state-dir paths are never recorded. A **second** SpawnRunner side-effect grants the cloud-hypervisor **runner** uid connect access to the cross-principal `d2b-gctl` token fs-share socket (served by the narrower `gctlfs` principal, ADR 0021, so cloud-hypervisor does not own it as it owns its other fs-share sockets): `--x` traversal on the per-VM `guest-control` dir and `rw` with an explicit `m::rw` mask on the `gctlfs`-owned `d2b-gctl` socket inode. The explicit mask lifts the 0700 socket's `mask::---`, which otherwise masks out the inherited `default:u:<ch_uid>` grant and EACCESes cloud-hypervisor's vhost-user connect (hanging device-init at api-ready timeout). It is `(dev, ino)`-scoped (re-fstat after the fd-based setfacl), grants **only** the runner uid (no execute in the mask, no group/other broadening), and retries until the socket is bound. It emits its own hash-only audit record (`target_class` ∈ {`gctlfs-dir`, `gctlfs-socket`}, `consumer_principal` = `cloud-hypervisor-runner`, `acl_diff_hash`, `result`); raw paths and uids-by-value are never recorded. |

### Metrics endpoint

The daemon serves these metrics endpoints. They are not broker ops,
but their capability + sandbox posture is documented here alongside
the broker-dispatch contracts for trust-boundary completeness.

| Endpoint | Served by | Transport | Capabilities | Sandbox posture | Notes |
| --- | --- | --- | --- | --- | --- |
| `http://127.0.0.1:9101/metrics` | `d2bd` (daemon, **not** broker) | HTTP Prometheus exposition, no auth (loopback-only bind) | **empty** bounding set on `d2bd.service` (the daemon already runs unprivileged; the metrics handler adds no new caps) | `NoNewPrivileges=true` on `d2bd.service`. Listener is `127.0.0.1:9101` only — never `0.0.0.0`. | Metric names are preserved (`d2b_vm_ch_api_up`, `d2b_vm_running`, `d2b_vm_state`); default cardinality budget is `vm`/`env`/`role` labels only, with topology labels opt-in. |
| `unix:///run/d2b/otel/host-egress.sock` | broker-spawned `OtelHostBridge` | OTLP/gRPC over `AF_UNIX` into CH-vsock | **empty** bounding set | Host OTel collector connects to the d2b-owned runtime socket; the bridge principal gets only the required runtime path and obs-vsock socket access. | Span/log attributes are constrained to the tracing contract: no secrets, no argv, no `/nix/store` paths. |

### Mutating recovery verb

The daemon enters **degraded mode** rather than refusing to serve when
bridge/route self-check fails. Read-only `status` / `doctor` /
`audit` remain available; per-env starts are blocked. The **sole**
mutating recovery verb is `d2b host reconcile --network --apply`,
which the broker dispatches through the existing host-prep ops
(`CreateTapFd` / `CreatePersistentTap` / `SetBridgePortFlags` /
`ApplyNftables` / `ApplyRoute` / `ApplySysctl` /
`ApplyNmUnmanaged` / `UpdateHostsFile`) to recreate bridges/routes
**without** starting any VM. No new broker variant.

### Cross-references

- [`docs/reference/components-observability.md`](components-observability.md) — metrics and OTLP endpoint surface.
- [`docs/reference/doctor.md`](doctor.md) and [`docs/reference/cli-output/host-doctor.md`](cli-output/host-doctor.md) — degraded-mode and health output.
- [`docs/reference/usbip-state-machine.md`](usbip-state-machine.md) — per-busid canonical order.

## Host singleton retirements

The framework no longer emits these host-singleton systemd units. The
daemon-only replacements below are the live surface, and the legacy
unit names should not appear in operator-facing remediation.

| Retired singleton | Replacement (daemon-only) | Current state | Reference |
| --- | --- | --- | --- |
| `d2b-net-route-preflight.service` | Daemon startup self-check; startup failures are diagnostic so cold-boot net VMs can still run their host-prep DAG and recreate bridges/routes. Workloads degrade only if their env net VM actually fails to start. Focused repair is `d2b host reconcile --network --apply`, which the broker dispatches through existing host-prep ops without starting any VM. | not emitted | `d2b host reconcile --network --apply` |
| `d2b-audit-check.service` + `d2b-audit-check.timer` | Daemon health endpoint that reads the broker `OpAuditRecord` daily files via `ExportBrokerAudit`; the Rust CLI `d2b audit` reads through the daemon. No separate systemd timer — `d2b host doctor` polls on demand. | not emitted | `ExportBrokerAudit` |
| `d2b-ch-exporter.service` | Daemon-emitted scrape metrics with preserved metric names (`d2b_vm_ch_api_up`, `d2b_vm_running`, `d2b_vm_state`) and bounded labels (`vm`/`env`/`role` only by default). The loopback scrape endpoint is `http://127.0.0.1:9101/metrics`; host telemetry egress to the obs VM uses `unix:///run/d2b/otel/host-egress.sock`. | not emitted | [`docs/reference/components-observability.md`](components-observability.md) |
| `d2b-otel-host-bridge.service` | Broker `SpawnRunner{role: OtelHostBridge}` runner (host-scoped singleton). See the per-runner-role dispatch contract above. | not emitted | `SpawnRunner{role: OtelHostBridge}` |
| `d2b-sys-<env>-usbipd-proxy.service` + `d2b-sys-<env>-usbipd-proxy.socket` + `d2b-sys-<env>-usbipd-backend.service` + `d2b-sys-<env>-usbipd-backend.socket` (per USBIP-enabled env) | Broker `SpawnRunner{role: Usbip, vm_id: sys-<env>-usbipd}` runner per env, gated by the per-busid state machine. See the runner-role row above. | not emitted | [`docs/reference/usbip-state-machine.md`](usbip-state-machine.md) |

## Legacy systemd surface obituary

This section is the canonical legacy-systemd obituary index. The
canonical surface is exactly:

- `d2bd.service` (unprivileged daemon)
- `d2b-priv-broker.service` + `d2b-priv-broker.socket`
  (socket-activated privileged broker)
- per-VM / per-role runners spawned via broker `SpawnRunner` (no
  systemd unit per runner; lifecycle is daemon-supervised via pidfd)

The `tests/privileges-doc-completeness-eval.sh` gate enforces that
every legacy template still emitted by `nixos-modules/` either has a
live broker-op row in this document or appears below as an obituary —
never both.

### Per-VM template obituaries

| Legacy unit | Replacement (broker op + runner role) | Status |
| --- | --- | --- |
| `d2b@<vm>.service` (host-wrapper.nix) | Daemon-supervised VM lifecycle: `d2bd::supervisor::dag` orchestrates the 5-node DAG; broker `SpawnRunner{role: CloudHypervisor, vm_id: <vm>, …}` for the runner. | not emitted |
| `microvm@<vm>.service` (upstream microvm.nix wrapper invoked by `d2b@<vm>`) | Replaced by direct broker `SpawnRunner{role: CloudHypervisor}` dispatch; the framework no longer composes the upstream template. | not emitted |
| `microvm-tap-interfaces@<vm>.service` | `host-prep.tap` DAG node → `CreateTapFd` / `CreatePersistentTap` broker dispatch. See the HostPrep DAG table above. | not emitted |
| `microvm-set-booted@<vm>.service` | `vm.set-booted` DAG node → pure-daemon `supervisor::state::record_booted(<vm>, <closure>)` (no broker op). | not emitted |
| `microvm-pci-devices@<vm>.service` | `host-prep.pci-devices` DAG node → `OpenDevice` broker dispatch. | not emitted |
| `microvm-virtiofsd@<vm>.service` (upstream template) | Broker `SpawnRunner{role: Virtiofsd, vm_id: <vm>, …}` + `supervisor::pidfd` watchdog. | not emitted |
| `d2b-<vm>-gpu.service` (host-sidecars.nix) | Broker `SpawnRunner{role: Gpu, vm_id: <vm>, …}` per the Gpu role matrix. | not emitted |
| `d2b-<vm>-video.service` (components/video/host.nix) | Broker `SpawnRunner{role: Video, vm_id: <vm>, …}` per the Video role matrix. | not emitted |
| `d2b-<vm>-snd.service` / audio runner (components/audio/host.nix) | Broker `SpawnRunner{role: Audio, vm_id: <vm>, …}` per the Audio role matrix. | not emitted |
| `d2b-<vm>-swtpm.service` (host-sidecars.nix) | Broker `SpawnRunner{role: Swtpm, vm_id: <vm>, …}` (long-lived sidecar) + `SpawnRunner{role: SwtpmFlush, vm_id: <vm>}` (pre-start one-shot). | not emitted |
| `d2b-<vm>-store-sync.service` | `host-prep.store-sync` DAG node → `StoreSync` broker dispatch. | not emitted |
| `d2b-known-hosts-refresh@<vm>.service` | `host-prep.known-hosts-refresh` DAG node → `SshKeygenProbe` broker dispatch. | not emitted |
| `d2b-vfsd-watchdog@<vm>.service` + `d2b-vfsd-watchdog@<vm>.timer` | Pure-daemon `supervisor::pidfd` watch on the virtiofsd runner pidfd; re-issues `SpawnRunner` on exit. No broker op. | not emitted |
| `d2b-otel-relay@<vm>.service` (host-otel-relay-acl.nix) | Broker `SpawnRunner{role: OtelHostBridge}` host singleton runner (one per host, not per VM). | not emitted |

### Host singleton obituaries

| Legacy unit | Replacement (broker op or daemon surface) | Status |
| --- | --- | --- |
| `d2b-net-route-preflight.service` | Daemon startup self-check + net-VM autostart dependency gating + `d2b host reconcile --network --apply` (broker `ApplyNftables` / `ApplyRoute` / `ApplySysctl` / `SetBridgePortFlags`). | not emitted |
| `d2b-audit-check.service` + `d2b-audit-check.timer` | Broker `ExportBrokerAudit` + `d2b host doctor` on-demand poll; no timer. | not emitted |
| `d2b-ch-exporter.service` | `d2bd` Prometheus exposition at `http://127.0.0.1:9101/metrics` (no broker op — daemon-emitted). | not emitted |
| `d2b-otel-host-bridge.service` | Broker `SpawnRunner{role: OtelHostBridge}` (host-scoped singleton, broker-supervised). | not emitted |
| `d2b-sys-<env>-usbipd-proxy.{service,socket}` + `d2b-sys-<env>-usbipd-backend.{service,socket}` (per USBIP-enabled env) | Broker `SpawnRunner{role: Usbip, vm_id: sys-<env>-usbipd}` + per-busid state machine (`UsbipBind`, `UsbipBindFirewallRule`, proxy reconcile). | not emitted |

### Activation-time hooks no longer emitted

| Hook | Replacement | Status |
| --- | --- | --- |
| `d2b-store-sync` activation hook (store.nix) | `d2b host install --apply` invoking broker `StoreSync` dispatch through the daemon. | not emitted |
| Per-VM `desktopItems` generation (cli.nix `d2b-launch-<vm>`) | Daemon-native launcher module emitting `.desktop` wrappers calling `d2b vm start <vm> --apply`. | not emitted |

The root-visible `systemd.services.*` declarations the framework owns
under `nixos-modules/` are `d2bd`, `d2b-priv-broker`,
`d2b-load-store-db` (boot-time tmpfiles helper), and
`d2b-load-host-keys` (boot-time helper). The observability backend
declares native services inside the auto-declared `sys-obs` VM, not
root-visible framework singletons. Every per-VM `d2b-<vm>-*` and
`microvm-*@<vm>` template, and every
`d2b-{net-route-preflight,audit-check,ch-exporter,otel-host-bridge}`
singleton, is gone.

## Operations outside the current broker surface

- Partition-root cpuset creation (`cpuset.cpus.partition=root`). The
  broker forbids it without an ADR.
- Threaded cgroups. Same rule as partition roots.

## Public `config` operation (daemon-handled, no broker dispatch)

The `d2b config` verb group (`sync` / `diff` / `approve` /
`reject` / `status`) is a **public** operation handled entirely by the
unprivileged daemon and CLI. It dispatches **no** broker request
(`brokerRequired: false`), so it is not in the broker catalog above; it
is in the machine-readable public-operation matrix
([`schemas/v2/privileges.json`](schemas/v2/privileges.json),
`publicOperations[].operation = "config"`).

| Operation | Subject | Scope | Broker dispatch | Destructive | Secret access | Allowed authz | Audit | Default-for-unknown |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `config` | VM (guest-editable config layer) | per VM | none | no (staging/review only) | no | `d2b-launcher` + `d2b-admin` for the local review sub-verbs; `sync` is `d2b-admin` only (see notes) | yes | deny |

Notes:

- `sync` is **admin-only**: it dispatches the daemon's `ReadGuestConfig`
  verb, which is gated to the `d2b-admin` role at `SO_PEERCRED`
  accept time (`verb_requires_admin("readGuestConfig")`). A
  launcher-role caller is rejected with the typed `authz-not-admin`
  error (exit `75`) before any guest read. The local review sub-verbs
  (`diff` / `approve` / `reject` / `status`) operate on the host-side
  staging copy and dispatch no daemon verb, so a launcher can run
  them — which is why the schema models the `config` group as
  `d2b-launcher` + `d2b-admin`. `sync` reads the guest-editable
  config over the authenticated **guest-control** vsock (the daemon's
  `ReadGuestConfig` → `ReadGuestFile` path), not over SSH, and writes
  only into the host-side staging copy; it never evaluates or imports
  the guest bytes. It fails **closed**: a VM whose running generation
  predates guest-control (or that does not advertise `ReadGuestFile`)
  returns a typed error rather than silently falling back to SSH.
- `approve` / `reject` are the trust transition and are
  **host-operator-only** in the handler (the guest can never approve
  its own config); they write only the operator-named `--to` path.
- No new host mutation flows through the broker for any `config` verb,
  so there is no `OpAuditRecord`; the daemon logs the public-operation
  decision instead.

## Public `exec` operation (daemon-handled, no broker dispatch)

The `d2b vm exec` verb is a
**public** operation handled entirely by the unprivileged daemon and
CLI. It dispatches **no** broker request (`brokerRequired: false`): the
daemon holds an in-process exec **session table** and proxies typed exec
ops over the authenticated guest-control vsock to the VM's `guestd`. It
is in the machine-readable public-operation matrix
([`schemas/v2/privileges.json`](schemas/v2/privileges.json),
`publicOperations[].operation = "exec"`).

| Operation | Subject | Scope | Broker dispatch | Destructive | Secret access | Allowed authz | Audit | Default-for-unknown |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `exec` | VM (guest process) | per VM | none | yes (runs a guest command) | no | `d2b-admin` only | yes | deny |

Notes:

- `exec` is **admin-only**: the daemon checks the `SO_PEERCRED` admin
  role on the owner connection **before** any session lookup, slot
  reservation, transport connect, or `ExecCreate`. A launcher-role
  caller is rejected first, with no guest-control session established.
- The daemon emits **one** kind=critical session-establishment audit
  event per exec session carrying only redacted fields: `vm`, `peer_uid`,
  and the negotiated `tty` flag. The opaque `session_handle`, the guest
  `exec_id`/`guest_boot_id`, per-stream offsets, and the op stream are
  never audit labels.
- Redaction is fail-closed: argv (hash-only), env, cwd, paths, stdio
  bytes, nonces, tokens, capability tags, and session handles never
  appear in any `Debug`/trace/audit/metric surface.
- No host mutation flows through the broker for `exec`, so there is no
  `OpAuditRecord`; the daemon records the public-operation decision and
  the session-establishment event instead.

## Public `shell` operation (daemon-handled, no broker dispatch)

The `d2b shell` verb is a **public** operation handled by the
unprivileged daemon and CLI. It dispatches **no** broker request
(`brokerRequired: false`): the daemon proxies persistent-shell management and
terminal operations over the authenticated guest-control channel to guestd. It
is in the machine-readable public-operation matrix
([`schemas/v2/privileges.json`](schemas/v2/privileges.json),
`publicOperations[].operation = "shell"`).

| Operation | Subject | Scope | Broker dispatch | Destructive | Secret access | Allowed authz | Audit | Default-for-unknown |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `shell` | VM (persistent guest shell) | per VM | none | yes (`kill` terminates a guest shell session) | no | `d2b-admin` only | yes | deny |

Notes:

- `shell` is **admin-only**: the daemon checks the `SO_PEERCRED` admin role
  before guest-control contact or owner-session handoff.
- The daemon records attach, detach, and kill decisions in its daemon-owned JSONL
  event stream. Records carry only the VM name, peer uid, closed action/result
  enums, force flag when relevant, and a fixed shell correlation digest.
- Raw shell names, terminal session handles, terminal bytes, helper diagnostics,
  argv, env, paths, and guest-control nonces never appear in metric labels,
  traces, or daemon audit fields.
- No host mutation flows through the broker for `shell`, so there is no
  `OpAuditRecord`; the broker remains uninvolved.

## Cross-references

- ADR 0011 (cgroup + pidfd), ADR 0012 (IPv6/IfName/bridge-port),
  ADR 0013 (firewall coexistence), ADR 0014 (modules + devices +
  runner-shape).
- [`docs/explanation/host-prepare.md`](../explanation/host-prepare.md) — conceptual model + recovery.
- [`docs/reference/error-codes.md`](error-codes.md) — typed
  exit-code catalog + audit decision codes section.
- [`docs/reference/cgroup-delegation.md`](cgroup-delegation.md).
- [`docs/reference/inet-d2b-chains.md`](inet-d2b-chains.md).
- [`SECURITY.md`](../../SECURITY.md) § trust-boundary delta.

## Broker CapabilityBoundingSet (canonical 8 caps)

The broker `CapabilityBoundingSet` contains exactly the following 8
capabilities. `CAP_SYS_PTRACE` is explicitly excluded.

| Capability | Rationale |
| --- | --- |
| `CAP_NET_ADMIN` | TAP interface creation, persistent TAP lifecycle, bridge port flags (`SetBridgePortFlags`), route programming (`ApplyRoute`), NetworkManager unmanaged drop-in writes. |
| `CAP_NET_RAW` | Raw socket required by the per-link sysctl IPv6-off sequencer (socket-based `ioctl` path on kernels that reject `sysctl` writes from non-root). |
| `CAP_DAC_OVERRIDE` | Write to root-owned files in trusted paths (audit log dir, tmpfiles-created directories, `/etc/hosts` d2b-managed block, NetworkManager conf.d drop-in). |
| `CAP_DAC_READ_SEARCH` | Directory traversal + open of cgroup subtree directories (required for `DelegateCgroupV2`/`OpenCgroupDir` before `fchown` to `d2bd`). |
| `CAP_SYS_ADMIN` | cgroup v2 delegation: `open("/sys/fs/cgroup/d2b.slice", ...)` + `fchown` on delegated subdirs; mount namespace setup for `SetupMountNamespace`. |
| `CAP_SETUID` | Drop to `d2bd` uid for `SpawnRunner` (the broker forks a runner child and drops privileges before exec). |
| `CAP_SETGID` | Drop to `d2bd-launchers` gid as part of the same spawn descent. |
| `CAP_FOWNER` | `fchmod`/`fchown` on files the broker creates (audit log entries, socket ACL repair) when the effective uid is not the file owner. |

### Socket-activation contract

`d2b-priv-broker.socket` is socket-activated. systemd creates,
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
- `d2bd.service` carries `Wants=d2b-priv-broker.socket` (not
  `Requires=`) so the daemon can serve even when the broker has idled.
- The socket is the **broker private socket** at
  `/run/d2b/priv.sock`, owned `root:d2bd` with mode
  `0660` (declared in `nixos-modules/host-broker.nix`). ONLY the
  `d2bd` system user can `connect(2)` to this socket; the
  broker calls `getsockopt(SO_PEERCRED)` at `accept(2)` time
  (Linux SO_PEERCRED returns pid+uid+gid only, NOT supplementary
  groups) and rejects any peer whose effective uid is not the
  `d2bd` uid. The broker does NOT classify launcher / admin
  authz at this socket — peer classification into
  `d2b-launcher` / `d2b-admin` authz classes happens at
  the **daemon's public socket** (`/run/d2b/public.sock`,
  owned `d2bd:d2b` 0660 per
  `nixos-modules/host-daemon.nix`), where `d2bd` classifies
  the peer's uid against the `launcherUsers` / `adminUsers`
  arrays in `/etc/d2b/daemon-config.json` and forwards the
  resulting authz-class identifier to the broker over its own
  `d2bd→broker` priv.sock connection (the broker trusts
  `d2bd`'s upstream authz-class verdict, not the peer's
  direct credentials). The authz-class identifiers
  (`d2b-launcher` singular, `d2b-admin` singular) are
  the broker authz layer's classification outputs propagated
  from the daemon and are DISTINCT from the Linux SYSTEM GROUP
  `d2b` — see the authz-class-vs-system-
  group note at the top of this document. Authorisation
  outcomes other than the two `d2b-launcher` /
  `d2b-admin` authz classes (i.e., `deny` from the daemon's
  upstream classification) result in `denied-refused` audit
  emission at the broker.

The daemon receives broker `SCM_RIGHTS` messages with
`MSG_CMSG_CLOEXEC`. Its ancillary buffer covers Linux's full 253-descriptor
limit plus peer credentials. Received descriptors enter an owning parser before
any frame validation; malformed or still-truncated ancillary data fails closed,
and all installed descriptors are closed on every error path.

### Per-operation cap usage

The table below maps each broker operation to the specific
capabilities it exercises. Operations that require no elevated
capability (pure `SO_PEERCRED` authz + file-descriptor passing) carry
`—`.

| Operation | Capabilities used | Notes |
| --- | --- | --- |
| `ValidateBundle` | — | read-only bundle validation via the bundle resolver |
| `ExportBrokerAudit` | `CAP_DAC_READ_SEARCH` | open audit-log dir for export |
| `DelegateCgroupV2` | `CAP_SYS_ADMIN`, `CAP_DAC_READ_SEARCH` | open + `fchown` d2b.slice subtree |
| `OpenCgroupDir` | `CAP_DAC_READ_SEARCH` | open per-VM cgroup dir for fd-passing |
| `CgroupKill` | delegated leaf write-fd | leaf-only teardown signal via broker-mediated `cgroup.kill` write |
| `PrepareStateDir` / `PrepareRuntimeDir` | `CAP_DAC_OVERRIDE`, `CAP_FOWNER` | mkdir + chown + chmod under trusted paths |
| `PrepareSwtpmDir` | `CAP_DAC_OVERRIDE`, `CAP_FOWNER` | fd-safe mkdir + `fchown` + `fchmod` 0700 + ACL clear (`setfacl -b -k`) on the persistent swtpm state dir, ancestor traversal `+x` for the swtpm principal, and the identity-bound tamper marker create/verify under root-owned `/var/lib/d2b/swtpm-markers/` (issue #64) |
| `OpenKvm` / `OpenVhostNet` / `OpenFuse` / `OpenDevice` | `CAP_DAC_OVERRIDE` | open device node on behalf of launcher |
| `CreateTapFd` / `CreatePersistentTap` | `CAP_NET_ADMIN` | `TUNSETIFF` + `TUNSETOWNER` on `/dev/net/tun` |
| `SetBridgePortFlags` | `CAP_NET_ADMIN` | `ioctl(SIOCSIFFLAGS)` on bridge port |
| `ApplyNftables` | `CAP_NET_ADMIN` | nftables ruleset load via netlink |
| `ApplyRoute` | `CAP_NET_ADMIN` | rtnetlink route add/del |
| `ApplySysctl` | `CAP_NET_RAW`, `CAP_NET_ADMIN` | per-link IPv6-off via `ioctl` or `sysctl` |
| `ApplyNmUnmanaged` | `CAP_DAC_OVERRIDE` | write NetworkManager conf.d drop-in |
| `UpdateHostsFile` | `CAP_DAC_OVERRIDE` | write `/etc/hosts` managed block |
| `ReconcileStorageScope` | `CAP_CHOWN`, `CAP_FOWNER`, `CAP_DAC_OVERRIDE` | static directory spec reconciliation only (`mkdir`/`fchown`/`fchmod`); dynamic templates are check-only until scoped expansion lands |
| `ValidateLockSpec` | none | read-only sync contract validation |
| `ModprobeIfAllowed` | `CAP_SYS_ADMIN` | `finit_module` against trusted module matrix |
| `SpawnRunner` | `CAP_SETUID`, `CAP_SETGID` | drop to d2bd uid/gid before exec |
| `SetupMountNamespace` | `CAP_SYS_ADMIN` | unshare + mount-bind inside mount namespace |

For capability-drift auditing, the broker binary is built with
`#![forbid(unsafe_code)]` (quarantined exception: `src/sys.rs` for
`SCM_RIGHTS` fd-passing FFI). Any new operation that claims a
capability not listed above requires a new ADR entry.

## Per-role minijail profile

Every runner role ships a closed allowlist minijail profile rendered by
`nixos-modules/minijail-profiles.nix`. The table below is the
canonical operator-facing view; each row is enforced by its per-role
validator (`tests/minijail-validator-<role>.sh`) which writes
`/var/lib/d2b/validated/p1-<role>.json` on success.

| Role | Profile id pattern | Caps (steady-state) | Setup-time carve-out / device binds | Validator |
| --- | --- | --- | --- | --- |
| `cloud-hypervisor` | `vm-<vm>-cloud-hypervisor` | `CAP_NET_ADMIN` (transient — runner drops it after the SCM_RIGHTS tap-fd recv path before entering its main loop; static minijail allowlist cannot express "transient", so the profile declares the setup-time union) | `/dev/kvm`, `/dev/net/tun` (optional `/dev/dri/renderD128` + `/dev/nvidia0` when graphics/accelerator passthrough is bound to this runner) | `tests/minijail-validator-cloud-hypervisor.sh` |
| `qemu-media` | `vm-<vm>-qemu-media` | empty | fd-backed QMP/media runner: read-only root + read-only `/nix/store`, private PID/mount namespaces, masked `/dev`, no `/dev/bus/usb`, `/dev/net/tun`, or `/dev/vhost-net` path exposure, no media path binds, and writable access only to `/run/d2b/vms/<vm>` plus the per-VM state dir. `/dev/kvm` is classified as the only declared device class for focused ACL/fd handoff; vhost-net remains inherited-fd only. Focused Wayland/GTK display access uses `XDG_RUNTIME_DIR`, `WAYLAND_DISPLAY`, and host-session socket ACLs, not broad path binds. `seccompPolicyRef = w1-qemu-media`; no-new-privileges is installed by broker spawn before seccomp. | `tests/unit/nix/cases/external-vm-kind.nix` + `packages/d2b-contract-tests/tests/minijail_roles.rs` |
| `virtiofsd` | `vm-<vm>-virtiofsd-<tag>` | empty | ADR 0021 broker-pre-established single-entry user namespace; `requiresStartRoot = false`, zero host capabilities, `--sandbox=chroot --inode-file-handles=never`. Normal shares map to `d2b-<vm>-runner`; `d2b-gctl` maps to `d2b-<vm>-gctlfs` and is read-only. | `tests/minijail-validator-virtiofsd.sh` (positive: virtiofsd profile accepts the zero-host-capability user-NS shape; negative: `ptrace` probe under the `w1-virtiofsd` seccomp policy must exit with SIGSYS) |
| `swtpm` (long-lived sidecar) | `vm-<vm>-swtpm` | empty | **CRITICAL** RW bind of `/var/lib/d2b/vms/<vm>/swtpm` (TPM 2.0 NVRAM + EK seed) + `/run/d2b/vms/<vm>/` (the TPM socket + flush socket live here as `tpm.sock` and `tpm-flush.sock` respectively) (control socket). MUST be real RW bind, NOT tmpfs. Wiping or losing the bind forces Entra/Intune re-enrollment for work-aad. | `tests/minijail-validator-swtpm.sh` + `tests/integration/live/swtpm-persistence-smoke.sh` (write/stop/daemon-restart/read-back persistence regression) |
| `swtpm-flush` (pre-start one-shot) | `vm-<vm>-swtpm-flush` | empty | Same `/var/lib/d2b/vms/<vm>/swtpm` + `/run/d2b/vms/<vm>/` (the TPM socket + flush socket live here as `tpm.sock` and `tpm-flush.sock` respectively) binds as the long-lived `swtpm` sidecar; runs `swtpm_ioctl -i` flush before the sidecar adopts state. | shares the swtpm validator + persistence smoke |
| `gpu` | `vm-<vm>-gpu` | empty (per-role smoke proves virgl/venus/cross-domain run under SCHED_OTHER) | device binds: `/dev/kvm`, `/dev/dri/renderD128`, `/dev/nvidiactl`, `/dev/nvidia0`, `/dev/nvidia-uvm`, `/dev/udmabuf`; mount `/run/user/<uid>/wayland-0` → `/run/d2b-gpu/<vm>/wayland-0`; ioctls: full `DRM_IOCTL_VIRTGPU_*` family (via DeviceClass::Dri) | `tests/minijail-validator-gpu.sh` (positive: DRM_IOCTL_VIRTGPU_GET_CAPS under profile; negative: ptrace → SIGSYS) |
| `audio` | `vm-<vm>-audio` | `CAP_NET_RAW` (vhost-user-sound bind on PipeWire mediation path; AF_NETLINK for virtio-snd) | RO bind of `/run/user/<uid>/pipewire-0`; RW bind of `/run/d2b/vms/<vm>/snd.sock`; seccompPolicyRef = `w1-audio` | `tests/minijail-validator-audio.sh` |
| `video` | `vm-<vm>-video` | empty | `deviceBinds = [ "/dev/dri/renderD128" ]` by default for virtio-media decode (virtio-media wire contract: `virtio_id=48`, 2×256 queues, 256 MiB SHM, `vring_base=0`); `graphics.videoNvidiaDecode = true` additionally allows `/dev/nvidiactl`, `/dev/nvidia0`, and `/dev/nvidia-uvm` for the proprietary NVIDIA VA-API/NVDEC backend; `/dev` is masked and no broad bind is allowed; RW bind of `/run/d2b-video/<vm>/`; seccompPolicyRef = `w1-video`; principal is the dedicated `d2b-<vm>-video` uid | `tests/minijail-validator-video.sh` |
| `vsock-relay` | `vm-<vm>-vsock-relay` | empty (pre-opened fds only, no AF_VSOCK socket creation in-role) | bind: `<manifest VM stateDir>/vsock.sock_<port>` listener plus the observability VM's inherited CH-vsock UDS; seccompPolicyRef = `w1-vsock-relay` (denies `socket(AF_VSOCK)` + `ptrace`). Before spawn, the broker removes only a non-listening socket whose exact parent is the manifest-resolved VM state directory and whose basename is `vsock.sock_<decimal-port>`; active, non-socket, symlink, and out-of-scope paths fail closed. | `packages/d2b-priv-broker/src/runtime.rs` unit coverage + `tests/minijail-validator-vsock-relay.sh` |
| `usbip` backend | `vm-sys-<env>-usbipd-backend` | uid 0 carve-out + `CAP_NET_RAW` | host module `usbip-host` (not `vhci_hcd`, which is the guest module); long-lived per-env `usbipd` backend. `usbipd` must write the kernel `usbip_sockfd` sysfs attribute as host-root, so the broker gives this one runner a scoped root carve-out with a private PID namespace and fresh procfs; `/etc`, `/var`, `/home`, `/root`, `/run`, `/tmp`, `/boot`, `/mnt`, `/media`, `/srv`, and `/opt` masked; `/dev` masked; and only the currently locked `/dev/bus/usb/<bus>/<dev>` node(s) rebound writable. | `tests/minijail-validator-usbip.sh` |
| `usbip` proxy | `vm-sys-<env>-usbipd-proxy` | empty | self-binding TCP proxy from `<env.hostUplinkIp>:3240` to `127.0.0.1:<backendPort>`; no device access | `tests/minijail-validator-usbip.sh` |
| `otel-host-bridge` | (host-scoped) `d2b-otel-host-bridge` | empty (fd-only contract; no AF_VSOCK socket creation) | bind set: `/run/d2b/otel`, CH vsock host socket, `host-egress.sock` (RW listen target); broker rejects bundle intent whose source VM ≠ `observability.vmName` | `tests/minijail-validator-otel-host-bridge.sh` |


## Related ADRs

- [ADR 0015: daemon-only clean break](../adr/0015-daemon-only-clean-break.md) — the architectural decision record that defines the daemon-only root surface of `d2bd` + `d2b-priv-broker`.
