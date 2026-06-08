# Host prepare: cgroup v2 delegation

> Host-prepare fragment. The full
> `docs/how-to/host-prepare.md` page is assembled from the fragments
> under `docs/how-to/host-prepare.d/*.md`; this file is the cgroup
> section.

`nixling` runs every VM payload inside a per-VM/per-role cgroup leaf
beneath `/sys/fs/cgroup/nixling.slice`. The slice is created by the
small `nixling-priv-broker` and then delegated to the non-root
`nixlingd` daemon so the daemon never needs `CAP_SYS_ADMIN` on the
host cgroup tree at runtime.

This page covers operator-visible behavior. The full algorithm,
ownership model, and audit record shape are in
[`docs/reference/cgroup-delegation.md`](../../reference/cgroup-delegation.md).

## How to verify cgroup delegation prerequisites

Before running `nixling host prepare --apply`, confirm the host meets
the prerequisites:

```bash
# 1. Unified cgroup v2 hierarchy:
[ -f /sys/fs/cgroup/cgroup.controllers ] \
  && echo "ok: unified cgroup v2" \
  || echo "fail: legacy/hybrid cgroup layout"

# 2. Required controllers advertised on the root:
grep -wE 'cpu|memory|io|pids|cpuset' /sys/fs/cgroup/cgroup.controllers

# 3. nixlingd is non-root (delegation refuses uid 0):
id nixlingd
```

A host that boots with `systemd.unified_cgroup_hierarchy=0` or with the
legacy `cgroup` v1 mount option WILL fail `host check` with
`cgroup-v2-unified-not-present` (exit code 1).

## What `host check` reports for cgroup

`nixling host check` evaluates the following invariants in order; the
first failure is what the operator sees:

| Reported code | Meaning | Remediation |
| --- | --- | --- |
| `cgroup-v2-unified-not-present` | `/sys/fs/cgroup/cgroup.controllers` missing or unreadable. | Re-boot with the unified cgroup v2 hierarchy. NixOS: `boot.kernelParams = [ "systemd.unified_cgroup_hierarchy=1" ];`. |
| `cgroup-controllers-missing` | One of `cpu`, `memory`, `io`, `pids`, `cpuset` is absent from `cgroup.controllers`. | Confirm `systemd-cgls --all` works on the host; ensure the kernel exposes the missing controller. |
| `cgroup-delegation-refused` | Phase B (post-delegation) runtime mutation was attempted while the broker is still uid 0 — i.e., the broker failed to drop to `nixlingd` uid before the steady-state cgroup code path. Phase A privileged setup legitimately runs as root per ADR 0011. | Re-check the `nixlingd` user/group bootstrap and re-run `host prepare --apply`; verify the broker's drop-priv between Phase A and Phase B is wired correctly. |
| `cgroup-kill-on-ancestor-refused` | A broker-mediated `CgroupKill` op was requested on `nixling.slice` or an intermediate VM/host cgroup (i.e., `path_class: slice` or `vm-interior`). | This is a guard — the daemon re-requests `CgroupKill` against the specific leaf path instead. No operator action. |

Every check writes a record to the broker audit log at
`/var/lib/nixling/audit/broker-<utc-date>.jsonl` (root:nixlingd 0640),
keyed by `operation: "DelegateCgroupV2"` or `operation: "OpenCgroupDir"`.

## What `host prepare --apply` does for cgroup

For a successful apply, the broker performs the 8-step delegation
sequence documented in [`cgroup-delegation.md`][ref]:

1. probe the unified hierarchy;
2. assert `{cpu, memory, io, pids, cpuset}` are advertised;
3. ensure `cpuset.cpus`/`cpuset.mems` inherit from `.effective` on
   every ancestor before `+cpuset` is enabled;
4. enable `+cpu, +memory, +io, +pids, +cpuset` on `cgroup.subtree_control`
   in that strict order, verifying each enable by re-reading;
5. create `/sys/fs/cgroup/nixling.slice`;
6. keep `cpuset.cpus.partition` as `member` on `nixling.slice`
   and every nixling-created descendant (per-VM intermediate /
   per-role / host-scoped leaves); nixling does NOT read or
   write ancestor `cpuset.cpus.partition` (the cgroup v2 root
   is typically a partition root and that state is the host's
   concern, not nixling's);
7. fd-relative `fchown` the delegated subtree to `nixlingd:nixlingd`;
8. refuse Phase B (post-delegation) runtime mutation if the broker
   is still running as uid 0; Phase A privileged setup
   legitimately runs as root per ADR 0011 Decision item 2.

After the apply, `nixling.slice` is owned by `nixlingd` and the
delegated subtree carries every required controller in
`cgroup.subtree_control`. Threaded cgroups are forbidden.

`cgroup.kill` is permitted only via **broker-mediated** `CgroupKill`
on per-VM role leaves or host-scoped leaves during declared
teardown (v1.1+ — the broker is the sole writer per
[`docs/reference/cgroup-delegation.md`](../../reference/cgroup-delegation.md)
"Broker ops on the cgroup tree"; daemon NEVER writes `cgroup.kill`
directly). Asking the broker to kill an ancestor returns
`cgroup-kill-on-ancestor-refused`.

[ref]: ../../reference/cgroup-delegation.md

## Troubleshooting

### "cgroup-v2-unified-not-present"

The host is on a legacy or hybrid cgroup layout.

- **NixOS**: set `boot.kernelParams = [ "systemd.unified_cgroup_hierarchy=1" ];`
  and reboot. Most NixOS systems already run unified cgroup v2 by
  default; this only applies to hosts that explicitly opted out.
- **Ubuntu 24.04**: unified cgroup v2 is the default. If the probe
  fails, check `mount | grep cgroup` — the only mount under
  `/sys/fs/cgroup` should be `cgroup2`.

### "cgroup-controllers-missing"

The kernel is older than 6.6 or has one of the required controllers
disabled. Confirm `CONFIG_CPUSETS=y`, `CONFIG_MEMCG=y`,
`CONFIG_BLK_CGROUP=y`, `CONFIG_CGROUP_PIDS=y`, `CONFIG_CGROUP_SCHED=y`.

### "cgroup-delegation-refused" (uid 0)

The broker is supposed to enter the cgroup work path as the dropped
`nixlingd` user. If it reaches that path while still running as root,
something is wrong with the broker bootstrap. Re-check
`docs/explanation/host-prepare.md` § recovery.

### `kernel.modules_disabled=1`

Cgroup delegation does NOT load any kernel modules. This sysctl
does not block delegation. If you see `host-modules-locked` from
`host check`, that is a separate device-related preflight (scope s4),
not cgroup-related.
