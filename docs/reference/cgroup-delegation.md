# Reference: cgroup v2 delegation

> W3 scope **s1** reference. Operator-facing how-to lives at
> [`docs/how-to/host-prepare.d/cgroup.md`](../how-to/host-prepare.d/cgroup.md).
> ADR with rationale and rejected alternatives:
> [`docs/adr/0011-cgroup-v2-delegation-and-pidfd-handoff.md`](../adr/0011-cgroup-v2-delegation-and-pidfd-handoff.md).

This page is the wire-stable contract for the `DelegateCgroupV2` and
`OpenCgroupDir` broker variants. The audit-record fields, error codes,
and ownership-table semantics here are the source of truth for the
broker, daemon, and CLI.

## Scope and invariants

W3 introduces **non-root nixlingd cgroup delegation**: the broker
runs the privileged setup once at host prepare, then `nixling.slice`
and its descendants belong to the `nixlingd` system user. Subsequent
runtime mutations (per-VM/per-role leaf creation, kill, attach) come
from `nixlingd` directly without going through the broker.

Hard invariants:

1. **Unified hierarchy required.** Presence of
   `/sys/fs/cgroup/cgroup.controllers` is the probe. Fail closed
   otherwise.
2. **Controller floor.** `cpu`, `memory`, `io`, `pids`, `cpuset` MUST
   all be present in the root `cgroup.controllers`. Optional
   controllers (`rdma`, `hugetlb`, `misc`, ...) are accepted but
   never required.
3. **Single slice name.** The slice is `nixling.slice` literally —
   not configurable. Per-VM leaves are `<vm-id>.scope` under it.
4. **`partition=member` everywhere.** `cpuset.cpus.partition` STAYS
   `member` on every ancestor and on `nixling.slice`. The
   `assert_partition_member_only` guard panics in debug builds and
   returns `cgroup-partition-root-forbidden` in release builds for
   any code path that tries to write the partition key.
5. **Threaded cgroups forbidden.** `cgroup.type=threaded` is refused
   with `cgroup-threaded-forbidden`. Removing this restriction
   requires a panel-approved ADR override.
6. **No internal processes.** `nixling.slice` and intermediate VM
   cgroup directories MUST be process-free. Leaf role cgroups are the
   only directories that carry processes.
7. **Kill scope.** `cgroup.kill` is allowed only on broker/daemon-owned
   VM or role leaves during declared teardown/cleanup. Ancestor
   `cgroup.kill` is refused with `cgroup-kill-on-ancestor-refused`.
8. **Non-root delegation.** Refuse delegation while running as uid 0.

## The 8-step algorithm

The algorithm runs through
[`nixling_host::cgroup`](../../packages/nixling-host/src/cgroup.rs):

| Step | Function | Failure code |
| --- | --- | --- |
| 1 | `probe_unified_hierarchy(root)` | `cgroup-v2-unified-not-present` |
| 2 | `require_controllers(root, REQUIRED)` | `cgroup-controllers-missing` |
| 3 | `prepare_cpuset_inheritance(<ancestor>)` before `+cpuset` | `cpuset-inheritance-failed` |
| 4 | `enable_subtree_controllers(<path>, ENABLE_ORDER)` per-controller, re-read after each | `cgroup-subtree-control-enable-failed` |
| 5 | `create_nixling_slice(...)` creates `nixling.slice`; `create_vm_subtree(...)` creates `<vm>.scope` leaves; `assert_no_internal_processes(<path>)` checks intermediate dirs | `cgroup-internal-processes-present` |
| 6 | `chown_subtree_to_nixlingd(<path>, uid, gid)` via fd-based `fchown` | `cgroup-io-error` |
| 7 | `cgroup_kill_leaf_only(<path>, leaf_set)` | `cgroup-kill-on-ancestor-refused` |
| 8 | `require_non_root_delegation()` (uid != 0 guard) | `cgroup-delegation-refused` |

`ENABLE_ORDER` is fixed at `+cpu, +memory, +io, +pids, +cpuset`. The
cpuset enable is the only one with a preceding inheritance step.

## Fd ownership table

The daemon owns one `Arc<OwnedFd>` per registered pidfd in
[`nixling_priv_broker` →
`nixlingd::supervisor::pidfd::PidfdTable`](../../packages/nixlingd/src/supervisor/pidfd.rs).

| Layer | What it holds | Lifetime |
| --- | --- | --- |
| Broker | The pidfd produced by `clone3(CLONE_PIDFD)` (or `fork`+`pidfd_open`) | Lives until SCM_RIGHTS send returns; broker drops its copy. |
| Kernel buffer | A duplicate fd inside the seqpacket message | Lives until the daemon `recvmsg(2)` consumes it. |
| Daemon table | `Arc<OwnedFd>` keyed by `(vm_id, role_id)` | Lives until `PidfdTable::deregister` is called (teardown or failed start). |
| Daemon poll loop | `tokio::io::unix::AsyncFd<RawFdView>` borrowing the table's `Arc<OwnedFd>` | Lives as long as the table entry. |

The daemon never holds a raw `pid_t` for control. Raw-pid kill/wait is
forbidden outside the reconciliation path, where pid +
`/proc/<pid>/stat` field 22 are both validated before the resulting
`pidfd_open` fd is accepted.

## Path-safety contract

All filesystem I/O against the cgroup tree follows W3 path-safety:

- fd-relative `openat`/`openat2` with `O_NOFOLLOW` on every open;
- `O_PATH | O_NOFOLLOW` followed by `fchown(fd, ...)` for the chown
  step — never path-based `chown`;
- the broker re-derives every operating path from the trusted bundle,
  never from caller input.

The L1c canary matrix in `tests/cgroup-delegation-oracle.sh` exercises
every refusal path.

## Audit records

Every `DelegateCgroupV2` and `OpenCgroupDir` decision emits one JSON
record to `/var/lib/nixling/audit/broker-<utc-date>.jsonl` (root:nixlingd 0640).

The common audit header is defined canonically in
[`docs/reference/privileges.md`](privileges.md) § "Audit record schema
(W3 baseline)". The cgroup variants reuse that header verbatim;
`authz_result` is the launcher/admin/deny class assigned by the broker
authz layer, and `decision` is the broker's per-operation
allowed/denied-refused/denied-unknown/errored verdict.

```json
{
  "ts": "...",
  "broker_version": "...",
  "bundle_version": "...",
  "bundle_hash": "...",
  "operation": "DelegateCgroupV2" | "OpenCgroupDir" | "CgroupKill",
  "public_operation_id": "...",
  "peer_uid": 0,
  "peer_gid": 0,
  "authz_result": "launcher" | "admin" | "deny",
  "subject_id": "...",
  "scope_id": "...",
  "decision": "allowed" | "denied-refused" | "denied-unknown" | "errored",
  "error_kind": "cgroup-delegation-refused" | ...,
  "tracing_span_id": "...",
  "operation_fields": { ... }
}
```

The broker's shared `OperationFields` enum now covers the live
non-bootstrap dispatcher as well as the cgroup paths documented here:

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
- `DelegateCgroupV2 { slice_path, controllers_enabled, owner_uid }`
- `OpenCgroupDir { cgroup_id, path_class }`
- `CgroupKill { cgroup_id, path_class }`

### `DelegateCgroupV2` `operation_fields`

| Field | Type | Notes |
| --- | --- | --- |
| `slice_path` | string | Canonical `/sys/fs/cgroup/nixling.slice`. Derived from the trusted bundle. |
| `controllers_enabled` | array<string> | Always `["cpu","memory","io","pids","cpuset"]` on success. |
| `owner_uid` | integer | The `nixlingd` uid the subtree is chowned to. |

### `OpenCgroupDir` `operation_fields`

| Field | Type | Notes |
| --- | --- | --- |
| `cgroup_id` | string | The canonical path the broker resolved from the subject. |
| `path_class` | string | `nixling-slice` / `vm-leaf` / `foreign` / `unknown-subject`. |

### `CgroupKill` (internal teardown path)

| Field | Type | Notes |
| --- | --- | --- |
| `cgroup_id` | string | The canonical leaf path. |
| `path_class` | string | Always `vm-leaf` on success; `unknown-subject` on bundle miss. |

## Forbidden surfaces

W3 explicitly forbids the following:

- writing `cpuset.cpus.partition` (everywhere stays `member`);
- creating threaded cgroups;
- holding non-leaf processes inside `nixling.slice` or an intermediate
  VM cgroup;
- `cgroup.kill` on `nixling.slice` or any ancestor;
- delegation while the broker is uid 0 (the broker drops privileges
  to `nixlingd` before entering the cgroup code path);
- libcgroup (rejected in the ADR — it cannot enforce the non-root
  delegation invariant);
- systemd `Slice=` direct delegation without the broker (rejected in
  the ADR — it cannot enforce bundle-derived paths or audit the
  decision).

Removing any of these requires a panel-approved ADR override.
