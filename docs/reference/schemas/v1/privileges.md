# `privileges.json` schema reference

`privileges.json` is the closed-world authorization artifact. It enumerates every public CLI/API operation and every private broker enum variant with the groups, audit behavior, broker requirement, and secret-access posture required to authorize or deny the request.

Producer: `nixos-modules/manifest-privileges.nix` emits this artifact; `packages/d2b-core` parses it.

Schema: [`privileges.json`](./privileges.json) (forward reference; generated with `cargo xtask gen-schemas`).

## Top-level fields

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `schemaVersion` | string | yes | Artifact schema version. This schema emits `v1`. |
| `publicOperations` | array | yes | Complete public CLI/API authorization matrix. |
| `brokerOperations` | array | yes | Complete private privileged-broker enum authorization matrix. |
| `groups` | object | yes | Canonical group names for launcher/admin policy. |
| `defaultForUnknown` | string | yes | Must be `deny`; unknown operations fail closed. |
| `secretReferencePolicy` | object | yes | Opaque key-ID-only secret/key reference invariant. |

Each row in both operation arrays has the same shape.

| Field | Type | Description |
| --- | --- | --- |
| `operation` | string | Stable operation identifier. |
| `subject` | string | Target kind: VM, env, host, key, secret, broker, or audit log. |
| `scope` | string | Scope selector such as `vm`, `env`, `host`, `global`, or `self`. |
| `allowedGroups` | array | Unix groups whose members may request the operation. |
| `destructive` | boolean | Whether the operation can stop workloads, delete state, rotate trust, or mutate host policy. |
| `secretAccess` | enum | `none`, `opaqueKeyId`, `readSecret`, `writeSecret`, or `rotateSecret`. Runnable operations use no raw secret access. |
| `brokerRequired` | boolean | Whether successful execution requires a privileged broker request. |
| `audit` | enum | `allowAndDeny`, `denyOnly`, or `none`. Security-sensitive rows use `allowAndDeny`. |
| `defaultForUnknown` | enum | Row-local default; this schema uses `deny` for all rows. |

## Public CLI/API matrix

| Operation | Subject | Scope | Allowed groups | Destructive | Secret access | Broker required | Audit |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `ListVms` | VM inventory | global | `d2b-launcher`, `d2b-admin` | false | none | false | denyOnly |
| `GetStatus` | VM or host status | vm/global | `d2b-launcher`, `d2b-admin` | false | none | false | denyOnly |
| `CheckBridges` | host network status | host | `d2b-launcher`, `d2b-admin` | false | none | false | denyOnly |
| `StartVm` | VM | vm | `d2b-launcher`, `d2b-admin` | false | none | true | allowAndDeny |
| `StartVmDetached` | VM | vm | `d2b-launcher`, `d2b-admin` | false | none | true | allowAndDeny |
| `StopVm` | VM | vm | `d2b-launcher`, `d2b-admin` | true | none | true | allowAndDeny |
| `RestartVm` | VM | vm | `d2b-launcher`, `d2b-admin` | true | none | true | allowAndDeny |
| `ForceStopNetVm` | env net VM | env | `d2b-admin` | true | none | true | allowAndDeny |
| `OpenConsole` | VM console | vm | `d2b-launcher`, `d2b-admin` | false | none | true | allowAndDeny |
| `AttachUsb` | VM USBIP | vm | `d2b-launcher`, `d2b-admin` | false | none | true | allowAndDeny |
| `DetachUsb` | VM USBIP | vm | `d2b-launcher`, `d2b-admin` | false | none | true | allowAndDeny |
| `BuildVm` | VM closure | vm | `d2b-launcher`, `d2b-admin` | false | none | false | allowAndDeny |
| `SwitchVm` | VM closure | vm | `d2b-launcher`, `d2b-admin` | true | opaqueKeyId | true | allowAndDeny |
| `BootVm` | VM closure | vm | `d2b-launcher`, `d2b-admin` | true | opaqueKeyId | true | allowAndDeny |
| `TestVm` | VM closure | vm | `d2b-launcher`, `d2b-admin` | true | opaqueKeyId | true | allowAndDeny |
| `RollbackVm` | VM closure | vm | `d2b-admin` | true | opaqueKeyId | true | allowAndDeny |
| `GcGenerations` | generations | vm/global | `d2b-admin` | true | none | true | allowAndDeny |
| `TrustHostKey` | known-hosts | vm | `d2b-admin` | true | opaqueKeyId | true | allowAndDeny |
| `RotateKnownHost` | known-hosts | vm | `d2b-admin` | true | none | true | allowAndDeny |
| `ListKeys` | keys | global | `d2b-launcher`, `d2b-admin` | false | opaqueKeyId | false | denyOnly |
| `ShowKey` | key public material | vm | `d2b-launcher`, `d2b-admin` | false | opaqueKeyId | false | denyOnly |
| `RotateKey` | key | vm | `d2b-admin` | true | rotateSecret | true | allowAndDeny |
| `Audit` | audit report | host/global | `d2b-launcher`, `d2b-admin` | false | none | true | allowAndDeny |
| `AudioStatus` | audio grant state | global | `d2b-launcher`, `d2b-admin` | false | none | false | denyOnly |
| `AudioSet` | audio grant state | vm | `d2b-launcher`, `d2b-admin` | true | none | true | allowAndDeny |
| `HostPrepare` | host resources | host | `d2b-admin` | true | none | true | allowAndDeny |
| `ActivateBundle` | trusted bundle | host | `d2b-admin` | true | none | true | allowAndDeny |

This schema does not implement secret/key flows. Reserved rows may exist so
matrix completeness gates can fail closed, but runnable behavior for
`ReadSecretById`, `InjectSecretById`, and `RotateSecretById` is deferred.

## Private broker enum matrix

| Operation | Subject | Scope | Allowed caller | Destructive | Secret access | Audit |
| --- | --- | --- | --- | --- | --- | --- |
| `OpenKvm` | `/dev/kvm` fd | vm | `d2bd` uid | false | none | allowAndDeny |
| `OpenTap` | TAP fd | vm/env | `d2bd` uid | true | none | allowAndDeny |
| `OpenVhostNet` | `/dev/vhost-net` fd | vm | `d2bd` uid | false | none | allowAndDeny |
| `ConfigureBridge` | bridge/TAP flags | env | `d2bd` uid | true | none | allowAndDeny |
| `ConfigureIpv6Off` | per-link sysctls | env | `d2bd` uid | true | none | allowAndDeny |
| `ApplyNftables` | `inet d2b` table | host/env | `d2bd` uid | true | none | allowAndDeny |
| `WriteNetworkManagerUnmanaged` | NM config | host | `d2bd` uid | true | none | allowAndDeny |
| `UpdateHostsBlock` | `/etc/hosts` block | host | `d2bd` uid | true | none | allowAndDeny |
| `PrepareCgroupDelegation` | cgroup v2 | host | `d2bd` uid | true | none | allowAndDeny |
| `OpenCgroupDir` | cgroup dirfd | vm/role | `d2bd` uid | false | none | allowAndDeny |
| `SetDeviceAcl` | device node ACL | role | `d2bd` uid | true | none | allowAndDeny |
| `OpenRuntimeSocket` | pre-bound Unix socket | role | `d2bd` uid | false | none | allowAndDeny |
| `LoadKernelModule` | kernel module | host | `d2bd` uid | true | none | allowAndDeny |
| `BindUsbipDevice` | USBIP busid | host/env | `d2bd` uid | true | none | allowAndDeny |
| `UnbindUsbipDevice` | USBIP busid | host/env | `d2bd` uid | true | none | allowAndDeny |
| `ReadSecretById` | secret | vm/host | `d2bd` uid | false | readSecret | allowAndDeny |
| `InjectSecretById` | secret | vm | `d2bd` uid | true | writeSecret | allowAndDeny |
| `RotateSecretById` | secret | vm/host | `d2bd` uid | true | rotateSecret | allowAndDeny |
| `PauseBroker` | broker | host | `d2bd` uid with admin-authorized public op | true | none | allowAndDeny |
| `ResumeBroker` | broker | host | `d2bd` uid with admin-authorized public op | true | none | allowAndDeny |

## Cgroup v2 delegation operation

`PrepareCgroupDelegation` is required and fail-closed. It creates
`/sys/fs/cgroup/d2b.slice`, writes `cgroup.subtree_control` on each
ancestor needed for delegation, chowns the delegated subtree to the
`d2bd` uid/gid, returns a dirfd or verified delegated path, and
aborts the entire host-prepare operation if any step cannot be verified.
See [ADR 0002](../../../adr/0002-non-root-daemon-and-privileged-broker.md).

## Secret-reference invariant

Artifacts reference secrets and keys by opaque key IDs only. They must
not contain private-key paths, token paths, or arbitrary secret file
paths. `secretAccess` documents authorization semantics; it is not a
location field.
