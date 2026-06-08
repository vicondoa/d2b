# Reference: cgroup v2 delegation

> Reference for the cgroup-delegation contract. Operator-facing
> how-to lives at
> [`docs/how-to/host-prepare.d/cgroup.md`](../how-to/host-prepare.d/cgroup.md).
> ADR with rationale and rejected alternatives:
> [`docs/adr/0011-cgroup-v2-delegation-and-pidfd-handoff.md`](../adr/0011-cgroup-v2-delegation-and-pidfd-handoff.md).

This page is the wire-stable contract for the `DelegateCgroupV2` and
`OpenCgroupDir` broker variants. The audit-record fields, error codes,
and ownership-table semantics here are the source of truth for the
broker, daemon, and CLI.

## Scope and invariants

Nixling uses **non-root `nixlingd` cgroup delegation**: the broker
runs the privileged Phase A setup once at host prepare, then
`nixling.slice` and its descendants belong to the `nixlingd` system
user. Subsequent runtime operations against the delegated subtree
split into two categories:

- **Daemon-direct** (read-only enumeration + pidfd-table
  registration): the daemon's `nixlingd` uid owns the subtree via
  Phase A `fchown` and READS delegated files using cgroup-directory
  fds obtained via the **broker's `OpenCgroupDir` op** (per the
  broker-ops table below — pidfds in the daemon's `PidfdTable` are
  for `pidfd_send_signal` + poll observability on processes ONLY;
  pidfds cannot read cgroup files like `cgroup.events`). The
  read-only cgroup files the daemon may need (e.g., `cgroup.events`
  `populated` field for liveness queries, per-leaf state for the
  `nixling status` verb) are accessed via dedicated cgroup-dir
  fds returned by `OpenCgroupDir`. The daemon does NOT perform
  process placement (`cgroup.procs` write), leaf mkdir, leaf
  rmdir, kill, or any other mutation — all mutations are
  broker-mediated per the table below. **The daemon NEVER writes
  to `cgroup.procs`** (the broker uses `clone3(CLONE_INTO_CGROUP)`
  for process placement at spawn time per
  [ADR 0011](../adr/0011-cgroup-v2-delegation-and-pidfd-handoff.md)
  Decision item 8 "Process-into-cgroup placement primitive";
  there is no daemon-side attach codepath in the current source).
- **Broker-mediated and audited** (the broker ops below): all
  cross-trust-boundary mutations on the cgroup tree are exposed
  as audited broker operations. The broker performs leaf mkdir
  (one-shot at Phase A), process placement (atomic on each
  `SpawnRunner` via `clone3(CLONE_INTO_CGROUP)`), leaf-only kill
  via `CgroupKill`, and any subsequent runtime state changes.

**Broker ops on the cgroup tree:**

| Broker op | Status | Audit | Owner of side-effect |
| --- | --- | --- | --- |
| `DelegateCgroupV2` | live | yes | Broker (one-shot Phase A: `+controllers` cascade, slice/leaf `mkdir`, `fchown` to `nixlingd` uid/gid; re-runnable on host re-prepare). |
| `OpenCgroupDir` | live | yes | Broker (fd-passing for delegated leaves; the daemon acquires fresh fds when needed). |
| `CgroupKill` | live | yes | Broker, broker-only. The broker holds the leaf write-fd via the Phase A `fchown` and is the **sole writer** of `cgroup.kill` files. The daemon NEVER writes `cgroup.kill` directly — daemon teardown of a runner uses `pidfd_send_signal(SIGTERM)` first; if SIGTERM does not drain the leaf within the role's documented grace period, the daemon escalates by issuing a `CgroupKill` broker request as a last resort. |

The **broker-only `cgroup.kill` writer invariant** is the canonical
rule for the leaf-kill code path (per ADR 0011 Decision item 6
"kill scope is leaf-only" + the audit-completeness requirement that
every cross-trust-boundary mutation generates an `OpAuditRecord`).
The corresponding error code
[`cgroup-kill-on-ancestor-refused`](error-codes.md#cgroup-kill-on-ancestor-refused)
applies to the broker's `CgroupKill` op (the only writer); there
are no daemon-side `cgroup.kill` write sites.

Hard invariants:

1. **Unified hierarchy required.** Presence of
   `/sys/fs/cgroup/cgroup.controllers` is the probe. Fail closed
   otherwise.
2. **Controller floor.** `cpu`, `memory`, `io`, `pids`, `cpuset` MUST
   all be present in the root `cgroup.controllers`. Optional
   controllers (`rdma`, `hugetlb`, `misc`, ...) are accepted but
   never required.
3. **Single slice name; per-VM-interior + per-role-leaf
   hierarchy.** The slice is `nixling.slice` literally — not
   configurable. Per-VM **intermediate** directories live at
   `nixling.slice/<vm-id>/` (process-free) with **per-role leaves**
   at `nixling.slice/<vm-id>/<role>/`. Host-scoped roles split
   into two patterns, both `path_class: host-scoped-leaf`:
   per-env (e.g., USBIP) at `nixling.slice/sys-<env>/<role>/`
   with `sys-<env>/` process-free interior; host singletons
   (e.g., otel-host-bridge) at `nixling.slice/host/<role>/`
   with `host/` process-free interior. The role-leaf model is
   required because the framework replaces the old per-VM
   systemd-template scope model with one broker-spawned child
   per role; each role needs its own pidfd / `cgroup.kill` scope.
4. **`partition=member` on nixling-created cgroups.** `cpuset.cpus.partition`
   STAYS `member` on `nixling.slice` and every nixling-created
   descendant (per-VM intermediate `<vm-id>/`, per-role leaves
   `<vm-id>/<role>/`, host-scoped `sys-<env>/<role>/`). The
   `assert_partition_member_only` guard panics in debug builds and
   returns `cgroup-partition-root-forbidden` in release builds for
   any code path that tries to write the partition key on a
   nixling-owned cgroup. The R10 kernel reviewer correctly noted
   that the cgroup v2 root is normally a partition root; the
   invariant therefore explicitly does NOT apply to the cgroup v2
   root or to any cgroup outside `nixling.slice` — nixling never
   reads or writes ancestor `cpuset.cpus.partition` values
   (partition-root state on the kernel root or distro-owned
   ancestors is a host concern, not a delegated-subtree concern).
   Reading the system root's partition state in the daemon's
   reconciliation path is permitted (for diagnostic
   observability only); writing to it is refused unconditionally.
5. **Threaded cgroups forbidden.** `cgroup.type=threaded` is refused
   with `cgroup-threaded-forbidden`. Removing this restriction
   requires a panel-approved ADR override.
6. **No internal processes.** `nixling.slice` and intermediate VM
   cgroup directories MUST be process-free. Leaf role cgroups are the
   only directories that carry processes.
7. **Kill scope.** `cgroup.kill` is allowed only on **broker-mediated**
   per-VM role leaves or host-scoped leaves during declared
   teardown/cleanup (the broker is the sole writer per cgroup-
   delegation.md "Broker ops on the cgroup tree" — daemon NEVER
   writes `cgroup.kill` directly; daemon requests broker escalation
   only after `pidfd_send_signal(SIGTERM)` grace expiry). Ancestor
   `cgroup.kill` is refused with `cgroup-kill-on-ancestor-refused`.
8. **Non-root delegation.** Refuse delegation **runtime mutation**
   while running as uid 0 — i.e., the `require_non_root_delegation`
   guard at step 8 is enforced on subsequent calls into the cgroup
   module **AFTER** the initial Phase A delegation completes
   (steps 1-6 are Phase A; they require root for `+controllers`,
   slice/leaf `mkdir`, and `fchown` to `nixlingd`'s uid/gid; the
   broker drops privileges between steps 6 and 7). See
   [ADR 0011](../adr/0011-cgroup-v2-delegation-and-pidfd-handoff.md)
   § Decision item 2 for the explicit Phase A vs Phase B split.

## The 8-step algorithm

The algorithm runs through
[`nixling_host::cgroup`](../../packages/nixling-host/src/cgroup.rs):

| Step | Function | Failure code |
| --- | --- | --- |
| 1 | `probe_unified_hierarchy(root)` | `cgroup-v2-unified-not-present` |
| 2 | `require_controllers(root, REQUIRED)` | `cgroup-controllers-missing` |
| 3 | `prepare_cpuset_inheritance(<ancestor>)` before `+cpuset` | `cpuset-inheritance-failed` |
| 4 | `enable_subtree_controllers(<path>, ENABLE_ORDER)` per-controller, re-read after each | `cgroup-subtree-control-enable-failed` |
| 5 | `create_nixling_slice(...)` creates `nixling.slice`; `create_vm_subtree(...)` creates per-VM intermediate `<vm-id>/` and per-role leaves `<vm-id>/<role>/` (current taxonomy per ADR 0011); `assert_no_internal_processes(<path>)` checks intermediate dirs | `cgroup-internal-processes-present` |
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
| Broker | The pidfd produced by `clone3(CLONE_PIDFD | CLONE_INTO_CGROUP)` (or, in the documented historical fallback, `fork`+`pidfd_open` per ADR 0011 Decision item 8). **The broker RETAINS this pidfd as the parent** of the SpawnRunner child for the lifetime of the child — per [ADR 0018](../adr/0018-microvm-nix-removal.md) § "broker-as-parent reaping model", only the broker (as parent) can `waitid(P_PIDFD)` to reap the child, so it MUST hold the pidfd until final reap. The R23 kernel reviewer flagged the prior "broker drops its copy" wording as incompatible with the broker-reaper invariant. | Broker holds until child exit + `waitid(P_PIDFD)` reap completes; only then drops. |
| Kernel buffer | A duplicate fd inside the seqpacket message (SCM_RIGHTS-sent DUP — broker's original pidfd is NOT transferred, only duplicated) | Lives until the daemon `recvmsg(2)` consumes it. |
| Daemon table | `Arc<OwnedFd>` of the DUP'd pidfd, keyed by `(vm_id, role_id)`. The daemon uses this pidfd ONLY for `pidfd_send_signal(2)` and pidfd-poll readiness observation (via tokio epoll); the daemon does NOT call `waitid(P_PIDFD)` on SpawnRunner children (the broker reaps, per ADR 0018) and does NOT use this pidfd to read cgroup files (the broker exposes `OpenCgroupDir` for cgroup-dir fd handoff — see the daemon-direct enumeration note above). | Lives until `PidfdTable::deregister` is called (teardown or failed start). |
| Daemon poll loop | `tokio::io::unix::AsyncFd<RawFdView>` borrowing the table's `Arc<OwnedFd>` (poll readiness only; on poll-readable the daemon emits an observability event but the broker is the one that reaps via OneShotComplete RPC per ADR 0018) | Lives as long as the table entry. |

The daemon never holds a raw `pid_t` for control. Raw-pid kill/wait is
forbidden outside the reconciliation path, where pid +
`/proc/<pid>/stat` field 22 are both validated before the resulting
`pidfd_open` fd is accepted.

## Path-safety contract

All filesystem I/O against the cgroup tree follows the documented
path-safety contract:

- fd-relative `openat`/`openat2` with `O_NOFOLLOW` on every open;
- For the chown step on `O_PATH` descriptors: use
  `fchownat(fd, "", uid, gid, AT_EMPTY_PATH)` — NOT `fchown(fd, ...)`,
  which is not the correct primitive for `O_PATH` descriptors
  (Linux `fchown` does not portably operate on `O_PATH` fds). The
  `AT_EMPTY_PATH` flag (Linux ≥ 2.6.39) directs the kernel to
  operate on the fd itself rather than a path component. As an
  alternative (when a non-`O_PATH` fd is acceptable), open the
  directory with `O_DIRECTORY | O_NOFOLLOW` (no `O_PATH`) and
  then call `fchown(fd, ...)`. The earlier
  `O_PATH | O_NOFOLLOW` + `fchown` combination is incorrect; the
  implementation MUST use `fchownat` with `AT_EMPTY_PATH` when
  working with `O_PATH` descriptors.
- the broker re-derives every operating path from the trusted bundle,
  never from caller input.

The L1c canary matrix in `tests/cgroup-delegation-oracle.sh` exercises
every refusal path.

## Audit records

Every `DelegateCgroupV2` and `OpenCgroupDir` decision emits one JSON
record to `/var/lib/nixling/audit/broker-<utc-date>.jsonl` (root:nixlingd 0640).

The common audit header is defined canonically in
[`docs/reference/privileges.md`](privileges.md) § "Audit record
schema". The cgroup variants reuse that header verbatim;
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
| `cgroup_id` | string | The canonical path the broker resolved from the subject. Omitted on subject-resolution failure. |
| `path_class` | string | One of `slice` / `vm-interior` / `vm-role-leaf` / `host-scoped-leaf` (the current four-value taxonomy per [ADR 0011](../adr/0011-cgroup-v2-delegation-and-pidfd-handoff.md) Decision item 1). Legacy `nixling-slice` / `vm-leaf` / `foreign` / `unknown-subject` values are retired. **Bundle-miss / unresolved-subject cases**: same convention as `CgroupKill` below — the `path_class` field is OMITTED from `operation_fields` on subject-resolution failure; the failure is recorded via `decision: "denied-unknown"` + `error_kind: "unknown-subject"` at the audit-header level. |

### `CgroupKill` (internal teardown path)

| Field | Type | Notes |
| --- | --- | --- |
| `cgroup_id` | string | The canonical leaf path. |
| `path_class` | string | Always `vm-role-leaf` (or `host-scoped-leaf` for host-scope roles) on success. **Bundle-miss / unresolved-subject cases**: the `path_class` field is **only populated after successful subject resolution**; on bundle miss the field is OMITTED from the audit record's `operation_fields` block and the failure is recorded via `decision: "denied-unknown"` + `error_kind: "unknown-subject"` at the audit-header level (the four-value enum `slice` / `vm-interior` / `vm-role-leaf` / `host-scoped-leaf` stays closed — no `unknown-subject` discriminant). |

## Forbidden surfaces

Nixling explicitly forbids the following:

- writing `cpuset.cpus.partition` on nixling-owned cgroups
  (`nixling.slice` and every nixling-created descendant stays
  `member` per invariant 4 above; the cgroup v2 root and other
  ancestor `cpuset.cpus.partition` values are out of scope —
  nixling never reads or writes them);
- creating threaded cgroups;
- holding non-leaf processes inside `nixling.slice` or an intermediate
  VM cgroup (`<vm-id>/` and `sys-<env>/` interiors stay
  process-free per the taxonomy in invariant 3);
- `cgroup.kill` on `nixling.slice` or any ancestor (including
  per-VM `<vm-id>/` and host-scope `sys-<env>/` interiors);
- **Phase B (post-delegation) runtime mutation while running as
  uid 0** — i.e., once Phase A (privileged setup: `+controllers`
  cascade, slice/leaf `mkdir`, `fchown` to `nixlingd`'s uid/gid;
  legitimately runs as root per
  [ADR 0011](../adr/0011-cgroup-v2-delegation-and-pidfd-handoff.md)
  Decision item 2) has completed and the broker has dropped
  privileges, all subsequent calls into the cgroup module
  (any potential daemon-side cgroup interaction — though the daemon
  performs read-only enumeration only per the
  scope-and-invariants section above) MUST run as `nixlingd`'s uid
  (`getuid() != 0`). Mutating operations (process placement via
  `clone3(CLONE_INTO_CGROUP)`, leaf kill via `CgroupKill`, leaf
  mkdir via Phase A `DelegateCgroupV2`) are broker-only and
  audited; there is no daemon-direct mutating codepath. Direct
  privilege escalation in the steady-state cgroup code path is what
  is forbidden, NOT the one-shot Phase A setup;
- libcgroup (rejected in the ADR — it cannot enforce the
  scoped non-root Phase B invariant);
- systemd `Slice=` direct delegation without the broker (rejected in
  the ADR — it cannot enforce bundle-derived paths or audit the
  decision).

Removing any of these requires a panel-approved ADR override.
