# ADR 0025: Host-jailed Wayland filter proxy role

- Status: Accepted (v1.3-dev)
- Date: 2026-06-10
- Related: ADR 0003 (minijail provisioning and sandbox interface),
  ADR 0011 (cgroup v2 delegation and pidfd handoff),
  ADR 0015 (daemon-only clean break),
  ADR 0021 (broker-pre-established user namespace for virtiofsd),
  ADR 0023 (runner-role lifecycle matrix)

## Context

Nixling graphics VMs currently use `wayland-proxy-virtwl --virtio-gpu`
inside the guest to bridge the virtio-gpu cross-domain channel to the
host Wayland compositor. This proxy runs inside the VM — inside the
security boundary — so it holds a direct connection to the host
compositor socket from the perspective of the host's ACL policy.

The crosvm GPU sidecar (`nixling-<vm>-gpu`) and render-node sidecar
(`nixling-<vm>-gpu`, render-node mode) both currently bind-mount the
real host compositor socket at `<gpuRuntimeDir>/wayland-0`. Any process
running as the GPU UID, or any process that can send malformed Wayland
messages through the cross-domain channel, has a path to the host
compositor.

The security goal is to remove the GPU runner's direct access to the
host compositor and interpose a host-owned, broker-spawned, minijailed
filter proxy that:

1. holds the only ACL grant on the real host compositor socket for this VM;
2. parses and filters Wayland traffic before forwarding to the compositor;
3. runs with empty host capabilities and mandatory seccomp;
4. has no PipeWire or Pulse socket access;
5. exposes a per-VM filter socket at `/run/nixling-wlproxy/<vm>/wayland-0`
   for crosvm to connect to instead of the real compositor socket.

## Decision

Add a new process role `wayland-proxy` to the nixling broker/daemon
role system. The role:

### Identity and isolation

- Principal: `nixling-<vm>-wlproxy` (dedicated system user + group).
- UID/GID: hash-derived via `stablePrincipalId` (same formula as all
  other per-VM principals).
- Host capabilities: **empty** (hard invariant; broker `live_spawn_runner`
  rejects spawns with non-empty caps for this role).
- Seccomp: **mandatory** — `seccompPolicyRef = "w1-wayland-proxy"`. The
  proxy parses untrusted guest Wayland bytes while holding the host
  compositor socket; spawning without a seccomp policy is rejected
  fail-closed in the broker `SpawnRunner` handler.
- PipeWire/Pulse access: **none**. The broker ACL refresh explicitly
  sets `u:<uid>:---` on `pipewire-0` and `pulse/native` for this role.

### Runtime directory and socket paths

- Dedicated per-VM runtime directory: `/run/nixling-wlproxy/<vm>`.
- Filter listen socket: `/run/nixling-wlproxy/<vm>/wayland-0` (where
  the crosvm GPU sidecar connects instead of the real compositor socket).
- In-jail upstream socket: `/run/nixling-wlproxy/<vm>/upstream` (where
  the real host compositor socket is bind-mounted read/write).
- `umask = 0o007` so the filter listen socket has mode `0660`; the
  per-VM runtime dir default ACL grants crosvm's named-user entry rw.

### Filesystem isolation

- Writable: `/run/nixling-wlproxy/<vm>` only.
- Bind-mount: real host compositor socket (`waylandHostSock`) → in-jail
  path `/run/nixling-wlproxy/<vm>/upstream`. Read/write.
- No `/dev` binds (pure AF_UNIX proxy; no device ioctls).
- `/nix/store` visible read-only.

### User namespace

The broker-pre-NS (ADR 0021) pattern is **not used** for this role.
The dedicated non-root host UID with empty capabilities is sufficient
for an AF_UNIX proxy. No `clone3(CLONE_NEWUSER)` pre-establishment is
required; the `userNamespace` field is absent in the minijail profile.

### DAG position and mid-life failure handling

- DAG edge (added in Wave 2 / Lane C):
  `wayland-proxy → <graphicsNodeId> → cloud-hypervisor`
  where `<graphicsNodeId>` is `gpu-render-node` when
  `graphics.renderNodeOnly = true` and `gpu` otherwise.
- Mid-life failure: the `nixlingd` pidfd watchdog is extended
  (`WlproxyWatchdogPolicy`) so unexpected wayland-proxy death triggers
  `StopRunnerRequested { runner_role: Gpu }`. The GPU runner silently
  blackholing Wayland traffic through a dead proxy socket is not
  acceptable — the VM must be torn down or the proxy restarted.
  `WlproxyWatchdogPolicy::stop_gpu_on_unexpected_exit` defaults to
  `true`.

### Lifecycle matrix (ADR 0023 template)

| # | Field | Value |
|---|-------|-------|
| 1 | **Fork model** | `clone3(CLONE_NEWCGROUP \| CLONE_INTO_CGROUP)` via `broker clone3_spawn_runner` |
| 2 | **Wait/reap owner** | `pidfd-handoff (broker → nixlingd)` via `SCM_RIGHTS` + `OpenPidfd` |
| 3 | **In-NS mount-action** | `apply` — writable `/run/nixling-wlproxy/<vm>`, bind-mount `waylandHostSock → upstream` |
| 4 | **Capability bounding set** | `empty` |
| 5 | **Ambient capability set** | `empty` |
| 6 | **Seccomp profile reference** | `w1-wayland-proxy` (mandatory; broker rejects absent policy) |
| 7 | **FD lifetime** | `close-on-exec` for all inherited broker fds; per-VM filter socket is `AF_UNIX` bound by the proxy after startup |
| 8 | **umask value** | `0o007` |
| 9 | **RLIMIT_NPROC value** | `inherit` (Wave 2 / Lane A may set explicit limit) |
| 10 | **oom_score_adj value** | `inherit` |
| 11 | **CLONE_INTO_CGROUP usage** | `yes — nixling.slice/<vm>/wayland-proxy` |

### What changes in Wave 2

This ADR covers the role contract, identity, isolation, and process
lifecycle. The following are deferred to Wave 2:

- Lane A: `nixling-wayland-filter` binary crate; filtering policy;
  app-id and title rewriting; a future filtering-policy ADR.
- Lane C: `processes-json.nix` and `host-activation.nix` integration —
  removing the real compositor socket from GPU runner bind-mounts,
  repointing `--wayland-sock`, and declaring the DAG edge.
- Lane D: guest `wl-cross-domain-proxy` guest service replacement and
  graphics.nix changes.

## Consequences

### Positive

- GPU runners no longer hold direct access to the host compositor socket.
- The security-critical Wayland filtering code runs in an isolated,
  minijailed process with empty capabilities and mandatory seccomp.
- Mid-life proxy death is detected and results in a predictable VM
  teardown rather than a silent blackhole.
- The role follows the established `stablePrincipalId` + broker
  `SpawnRunner` + `pidfd` handoff pattern consistently.

### Negative / trade-offs

- One additional process per graphics VM when `waylandFilter.enable = true`.
- The `nixling-<vm>-wlproxy` user/group are declared for all
  `graphics.enable` VMs regardless of `waylandFilter.enable`. The user
  is harmless when unused; this is consistent with how `nixling-<vm>-gpu`
  is declared for all graphics VMs including non-crossDomainTrusted ones.

## Non-decisions

- The filtering policy (which globals are allowed/denied, seccomp
  allowlist, RLIMIT_NOFILE value) belongs in a future filtering-policy
  ADR.
- The broker-pre-NS (ADR 0021) pattern was considered and rejected for
  this role: the proxy binds an AF_UNIX listen socket and has no need
  for fake-root semantics.
- PipeWire/Pulse access was considered and rejected. The proxy is a
  pure Wayland-protocol bridge; audio is orthogonal.
