# `minijail-profile.json` schema reference

`minijail-profile.json` is the private sandbox profile catalog. It describes typed role profiles for `d2bd`, broker-facing helpers, Cloud Hypervisor, sidecars, and readiness helpers without embedding kernel-version-specific syscall allowlists.

Producer: `nixos-modules/manifest-minijail.nix` emits this artifact; `packages/d2b-core` parses it.

Schema: [`minijail-profile.json`](./minijail-profile.json) (forward reference; generated with `cargo xtask gen-schemas`).

## Top-level fields

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `schemaVersion` | string | yes | Artifact schema version. This schema emits `v1`. |
| `profiles` | array | yes | Closed set of named role profiles. |
| `requiresStartRootPolicy` | object | yes | Global policy for bounded start-as-root exceptions. |
| `seccompPolicies` | array | yes | References to generated or audited seccomp policy artifacts. |

## Profile fields

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `id` | string | yes | Stable profile ID referenced by `processes.json`. |
| `role` | string | yes | Role kind using the same vocabulary as `processes.json`. |
| `uid` / `gid` | integer | yes | Steady-state identity. Long-lived payloads must not run as uid 0. |
| `supplementaryGroups` | array | yes | Explicit group list; ambient groups are not inherited. |
| `capabilities` | array | yes | Exact capability set. Broad sets and undeclared caps are rejected. |
| `namespaces` | object | yes | Mount, pid, ipc, uts, net, cgroup, and user namespace decisions. |
| `noNewPrivs` | boolean | yes | Must be true for steady-state roles. |
| `seccompPolicy` | string | yes | Reference to a policy ID; syscall allowlists are not prose fields. |
| `mountPolicy` | object | yes | Readonly binds, writable binds, tmpfs mounts, propagation, and pivot/root policy. |
| `cgroupPlacement` | object | yes | Per-VM and per-role cgroup path under the delegated subtree. |
| `requiresStartRoot` | boolean | yes | Whether the profile may begin as root before dropping to `uid`/`gid`. |
| `startRootJustification` | string or null | yes | Required when `requiresStartRoot = true`; must cite an ADR-listed carve-out. |

## `requiresStartRoot` policy

`requiresStartRoot` is forbidden for long-lived roles unless the profile
names a bounded exception and the runtime verifies the drop before the
role is considered ready. Known carve-outs are limited to roles such
as `virtiofsd` when the implementation requires temporary root for
mount-namespace setup. Profiles requesting uid 0 as their steady-state
identity are rejected.

## Mount policy fields

| Field | Description |
| --- | --- |
| `readonlyBinds` | Host paths mounted read-only into the role. |
| `writableBinds` | Writable paths; every entry must be documented and role-scoped. |
| `tmpfs` | Ephemeral tmpfs mounts. |
| `root` | Start root or pivot root policy. |
| `propagation` | Explicit mount propagation setting. |
| `devices` | Device nodes are absent by default; privileged devices arrive as fds unless an ADR allows a node. |

## Seccomp policy references

The catalog references seccomp policy artifacts but does not enumerate
syscalls. Kernel-version-specific syscall and ioctl allowlists are owned
by later runtime waves and must be derived from typed role requirements.
