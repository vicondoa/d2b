# ADR 0046 Provider dossier: Provider/system-minijail

| Field | Value |
| --- | --- |
| Spec ID | `ADR-046-provider-system-minijail` |
| Parent | ADR 0046 |
| Status | Proposed |
| Version | 1 |
| Baseline | `b5ddbed67867d9244bf33390868101bd9b053e49` |
| Main reuse | `a1cc0b2da4a08ca3240a770a972fe4da6f912bef` |
| Normative | Yes |
| Owners | `d2b-provider-system-minijail` crate owner |
| Depends on | `ADR-046-terminology-and-identities`, `ADR-046-resource-object-model`, `ADR-046-resource-api-and-authorization`, `ADR-046-resource-reconciliation`, `ADR-046-components-processes-and-sandbox`, `ADR-046-provider-model-and-packaging`, `ADR-046-primitive-resource-composition`, `ADR-046-resources-host-guest-process-user`, `ADR-046-core-controllers`, `ADR-046-componentsession-and-bus`, `ADR-046-nix-configuration`, `ADR-046-telemetry-audit-and-support` |
| Supersedes | Current `d2b-priv-broker` SpawnRunner, `d2b-core` minijail profile, and `d2bd` supervisor pidfd/wait paths for minijail-spawned processes |
| Related ADRs | ADR 0021 (broker user namespace for virtiofsd), ADR 0011 (cgroup v2 delegation and pidfd handoff), ADR 0003 (minijail provisioning and sandbox interface), ADR 0034 (storage lifecycle for this Provider's zero state) |

---

## 1. Scope

This dossier defines the complete Provider/system-minijail specification: its
purpose, independently buildable crate, bootstrap exception, implemented
ResourceTypes, binaries and component inventory, root config schema, compiled
SandboxSpec contract (namespaces, capability classes, seccomp, mounts, cgroup
placement, user namespace pre-establishment), Process and EphemeralProcess
lifecycle, pidfd ownership and d2b wait/reap, adoption and quarantine rules,
restart and stop/finalize, effect port surface (MinijailProcessEffectPort),
d2b-bus RBAC, errors, status additions, audit events, telemetry labels, Nix
authoring examples, hard bounds and performance gates, current-code reuse
ledger, implementation work items, test inventory, and removal proof for every
superseded path.

Provider/system-minijail and Provider/system-systemd are the two Process
Provider implementations in the initial system Provider family. They implement
identical ResourceTypes — `Process` and `EphemeralProcess` — against one common
schema and one shared conformance suite. The compiled sandbox is the only
implementation-specific surface.

**D089 spec extension contract:** this Provider's implementation-only desired
configuration is carried in `spec.provider.settings` under
`system-minijail.d2b.io/Process/spec` or
`system-minijail.d2b.io/EphemeralProcess/spec`; each schema is registered/signed
in the manifest, deny-unknown, bounded, versioned, and validated against
`spec.providerRef` at Nix build and API admission. Base fields stay at `spec.*`;
shared semantics are promoted to the Process/EphemeralProcess base and never
placed in `spec.provider`. This Provider implements the exact base spec/status
schema version/fingerprint, accepts the canonical minimal valid base Spec, and
rejects an unsupported optional base capability only through its signed
capability matrix plus provider-neutral `unsupported-capability`.
`spec.provider` aligns with `status.provider` for `Provider/system-minijail`.

---

## 2. Provider identity

| Field | Value |
| --- | --- |
| `providerRef` | `Provider/system-minijail` |
| Crate | `packages/d2b-provider-system-minijail/` |
| Package identity | `io.d2b.system-minijail` |
| Publisher | `io.d2b` (first-party) |
| Implemented ResourceTypes | `Process`, `EphemeralProcess` |
| Supported Host/Guest Provider capabilities | `pidfd`, `cgroup-v2`, `user-namespace`, `minijail-seccomp` |
| Supported domains | `system` on any Host or Guest; `user` domain only if the Provider descriptor's conformance extension declares `user-domain-supported: true` for that Host/Guest placement |
| Bootstrap role | Fixed bootstrap controller — one of the two Providers without a Process resource |
| Wait/reap ownership | `d2b` (the Provider controller); not systemd |
| Artifact catalog type | `provider` |

The Provider crate mandatory layout:

```
packages/d2b-provider-system-minijail/
  src/                  # controller logic, sandbox_compiler, launch, adoption, pidfd, wait, user_ns
  tests/                # hermetic Cargo integration tests
  integration/          # container/Host/Guest/broker integration scenarios
  README.md             # required Provider dossier — this file's canonical prose summary
```

Workspace policy rejects any of these four paths missing (`src/`, `tests/`,
`integration/`, `README.md`). A nested `integration/README.md` is not a
separate workspace-policy requirement. No other Provider
crate may import `d2b-provider-system-minijail` internals outside the declared
public API. The crate may not import another Provider's implementation
internals.

---

## 3. Bootstrap exception

Provider/system-minijail is one of exactly two Provider controllers in a Zone
that are not represented by a `Process` resource. The other is
`Provider/system-core`.

The bootstrap boundary is closed:

- Zone runtime and embedded store/resource API/bus endpoint start first.
- `Provider/system-core` (fixed core-controller) starts as the first process.
- `Provider/system-minijail` (fixed minijail controller) starts as the second
  process.
- Both use the compiled bootstrap authorization.
- system-minijail then launches every other Provider/controller/service/worker
  as a `Process` resource — including `Provider/system-systemd`.

This is the fixed bootstrap exception because no Process controller exists yet
to launch the first Process controller.

### Bootstrap authorization scope

system-minijail does not create Process or EphemeralProcess resources.
Those are created by owning controllers (e.g., `Provider/volume-virtiofs`
creates virtiofsd `Process` resources) or by the configuration publication
handler. system-minijail watches and reconciles existing resources where
`spec.providerRef = Provider/system-minijail`.

The compiled bootstrap authorization — not a stored Role/RoleBinding — grants
system-minijail exactly:

- `Process` get, list, and watch on the local Zone, restricted to resources
  whose `spec.providerRef` equals `Provider/system-minijail`;
- `Process` update-status and update-finalizers on those same resources;
- `EphemeralProcess` get, list, and watch on the local Zone, restricted to
  resources whose `spec.providerRef` equals `Provider/system-minijail`;
- `EphemeralProcess` update-status and update-finalizers on those same
  resources;
- `LaunchTicket` privilege from the fixed ProviderSupervisor;
- effect port calls via the injected `MinijailProcessEffectPort` (opaque
  Process/LaunchTicket/profile/resource IDs only; no broker service/client/DTO
  imported by the Provider crate).

It does not grant:

- `create` or `update-spec` on any ResourceType;
- any resource verb on a remote or parent Zone;
- any `Provider` create/update/delete;
- any `Role` or `RoleBinding` create/update/delete;
- any broker operation beyond what the `MinijailProcessEffectPort` privately
  authorizes; direct access to any broker service, client, or DTO is
  prohibited for the Provider crate;
- any host path, socket, or file descriptor outside the inherited bootstrap FD
  set.

The bootstrap authorization is non-configurable. No config field can widen it.
All bootstrap actions are structurally validated and audited. A wrong subject,
remote route, Provider generation, method, or purpose fails closed.

After the first stored Role/RoleBinding generation is activated, system-minijail
operates under the same native RBAC engine as all other controllers.

---

## 4. Component inventory

Provider/system-minijail contains one controller component and no service or
worker components.

### 4.1 `minijail-controller` (controller)

| Field | Value |
| --- | --- |
| Component ID | `minijail-controller` |
| Type | controller |
| Binary | `d2b-provider-system-minijail` (single executable) |
| Exported ResourceTypes | `Process`, `EphemeralProcess` |
| Domain | `system` (default); `user` when descriptor declares `user-domain-supported` |
| Cardinality | 1 per Zone |
| Process placement | Fixed bootstrap; no Process resource parent |
| Config projection | Provider `spec.config` (fixed empty; no configurable fields) |
| State | None — `Provider/system-minijail` declares no Provider state Volume; `ProviderStateSet(zone, "system-minijail")` is empty. Bounded non-secret operational state (reconcile stage, per-Process launch/adoption observations, counters, closed-enum error detail) lives in the owning resource's `status` subresource and the core Operation ledger (D087); persisted restart/backoff/checkpoints are core `Process`/`EphemeralProcess` status and the core Operation ledger; running units are re-adopted from declared cgroup leaves and fresh pidfds. Live pidfds/FDs are process-local and non-persistent. The controller declares no state namespace, mounts no state Volume, and needs no dedicated state-layout `User/<name>` principal (D086 superseded by D087) |
| Permission claims | `Process` get/list/watch/update-status/update-finalizers (where `providerRef=Provider/system-minijail`); `EphemeralProcess` get/list/watch/update-status/update-finalizers (where `providerRef=Provider/system-minijail`); effect port calls via the injected `MinijailProcessEffectPort` (opaque IDs; no broker service/client/DTO imported) |
| Readiness | Ready when bootstrap authorization active, redb connection established, all pending adopted processes verified |
| Drain | Stop dispatching LaunchTickets; wait for inflight ProviderSupervisor operations; close ComponentSession |

There are no service, worker, or separate component binaries in this Provider.
The controller is the only binary entry point.

---

## 5. Root config schema

`Provider/system-minijail` has a fixed empty, non-configurable `spec.config =
{}`. There are no operator-settable fields.

`Provider/system-minijail` declares **no** Provider state Volume for its
`minijail-controller` component. `ProviderStateSet(zone, "system-minijail")` is
the optional, query-time grouping of the *declared* Volume resources carrying
`ownerRef: Provider/system-minijail`; it is not a ResourceType or stored
artifact and is empty. Bounded non-secret operational state belongs in the
owning resource's `status` subresource and the core Operation ledger by default
(D087): persisted restart counts, backoff state, and operational checkpoints are
core resource/operation state (Process status and checkpoint records owned by
the resource API), not Provider-owned private state. Live pidfds and in-flight
FDs are process-local and non-persistent.

Because the `minijail-controller` component holds no durable payload that passes
the storage-need test — its operational state is fully derivable from spec,
`status`, the core Operation ledger, and external observation (running Processes
re-adopted from declared cgroup leaves and fresh pidfds) — it declares no state
namespace, no state Volume, no state-view mount, and no dedicated state-layout
`User/<name>` principal. There is no empty identity-only Volume.

### 5.1 No bootstrap-state exception

`Provider/system-minijail` is a fixed bootstrap controller that starts before
`Provider/volume-local` is ready. Because it declares no state Volume and
reaches Ready from resource `status`, the core Operation ledger, and external
process observation, it needs no state Volume before volume-local is ready — so
there is no bootstrap state-Volume cycle, no closed bootstrap storage mechanism,
no bootstrap `dirfd` delivery, and no bootstrap-storage exception (D086,
superseded by D087; see "No bootstrap state Volume" in
`ADR-046-components-processes-and-sandbox`). There is no hidden bootstrap store,
and no new public resource type, d2b-bus service, or broker operation is
introduced.

Process lifecycle defaults (drain timeout, restart backoff base/max) and
resource-level bounds (per-process `startDeadline`, `runtimeDeadline`, TTL
limits) are declared in the `Process` or `EphemeralProcess` spec by the owning
controller or configuration author. The fixed signed manifest owns concurrency
bounds and capability constraints; they are not operator-configurable.

No executable path, UID/GID, seccomp BPF program, minijail argument string,
cgroup path, socket address, or credential byte is a Provider config field.

---

## 6. Implemented ResourceTypes

### 6.1 `Process`

Provider/system-minijail implements the full `Process` ResourceType defined in
`ADR-046-resources-host-guest-process-user`. The common spec, common status,
common conditions, and common reconcile/finalize algorithm all apply without
modification. This section documents only minijail-specific behavior.

Selecting `Provider/system-minijail`:

```yaml
spec:
  providerRef: Provider/system-minijail
  executionRef: Host/host-system   # or Guest/<name>
  domain: system
  template: virtiofsd              # plain component template ID
  sandbox:
    namespaceClasses: [user, mount, pid]
    capabilityClasses: []
    seccompClass: strict
    noNewPrivileges: true
    startRoot: false
    environmentClass: minimal
    readOnlyRoot: true
    userNamespace:
      mappingClass: process-principal-root
```

`waitReapOwner` in status is always `"d2b"` for processes under this Provider.

### 6.2 `EphemeralProcess`

Provider/system-minijail implements the full `EphemeralProcess` ResourceType.
`processClass` must be `worker`. All EphemeralProcess spec/status fields are
as defined in `ADR-046-resources-host-guest-process-user`. This section
documents only minijail-specific behavior.

`waitReapOwner` in status is always `"d2b"`.

EphemeralProcess does not use `adoptionPolicy` or `adoptionState`. On a
controller restart, the controller attempts **continuation recovery**: it
locates the running one-shot by cgroup leaf membership, reverifies its identity,
opens a fresh pidfd, and attaches a waiter to observe the exact exit. If
identity verification passes, the EphemeralProcess remains in `Running` phase
and the fresh pidfd is used to await the real exit (no relaunch). If the
candidate is ambiguous or identity verification fails, the EphemeralProcess is
written to `Unknown` phase and quarantined; it is never auto-TTL-cleaned while
the process may still be live or ambiguous. The operator resolves ambiguity
through normal resource management or a full Zone reset.

---

## 7. SandboxSpec compilation

The `SandboxSpec` in a Process or EphemeralProcess spec declares semantic
requirements. Provider/system-minijail compiles these into a verified
implementation-specific plan before spawning. The compiled plan's digest is
stored in `status.sandboxRevisionDigest`.

No raw capability bitmask, seccomp BPF bytecode, minijail argument string,
mount table row, or cgroup path fragment escapes the compilation step into any
public resource field, status, audit payload, log line, or metric label.

### 7.1 Namespace classes

`SandboxSpec.namespaceClasses` selects which Linux namespaces are new for the
spawned process. An empty list inherits all parent namespaces.

| `NamespaceClass` value | Linux namespace | Notes |
| --- | --- | --- |
| `user` | `CLONE_NEWUSER` | Requires `SandboxSpec.userNamespace` to be set; see §7.7. Cannot combine with `startRoot: false` on a plain system-domain process unless `userNamespace` is set. |
| `pid` | `CLONE_NEWPID` | Spawned process is PID 1 inside the namespace. |
| `mount` | `CLONE_NEWNS` | Required for read-only root or custom mount table. |
| `ipc` | `CLONE_NEWIPC` | Isolates SysV IPC and POSIX message queues. |
| `uts` | `CLONE_NEWUTS` | Isolates hostname and NIS domain. |
| `network` | `CLONE_NEWNET` | Isolates network interfaces; used only for fully network-isolated workers. Not used when the Process has a `networkUsage` ref to an active Network resource. |
| `cgroup` | `CLONE_NEWCGROUP` | New cgroup namespace. Not used when the broker must place the process into a pre-delegated cgroup leaf. |
| `time` | `CLONE_NEWTIME` | New time namespace. Supported on kernel ≥5.6. |

Combinations that the compiler rejects at spec admission:

- `user` without a `userNamespace` block;
- `network` combined with a non-null `networkUsage.networkRef`;
- `cgroup` when `Host.spec.provider.settings.capabilities` does not include
  `cgroup-v2`.

### 7.2 Capability classes

`SandboxSpec.capabilityClasses` selects semantic capability grants. The compiler
translates each class to the smallest Linux capability set needed. An empty
class list means no capabilities beyond the user-domain base set.

The capability class enumeration is closed. The Provider adds no value to this
list without a descriptor update approved in the Provider's provider-dossier
change.

| `CapabilityClass` value | Compiled to | Restriction |
| --- | --- | --- |
| `network-bind` | `CAP_NET_BIND_SERVICE` | Permitted for service processes needing ports <1024. |
| `network-raw` | `CAP_NET_RAW` | Requires explicit Provider descriptor carve-out. |
| `network-admin` | `CAP_NET_ADMIN` | Requires explicit Provider descriptor carve-out. Denied in user domain. |
| `sys-time` | `CAP_SYS_TIME` | Requires explicit Provider descriptor carve-out. |
| `sys-ptrace` | `CAP_SYS_PTRACE` | Requires explicit Provider descriptor carve-out. |
| `sys-admin` | `CAP_SYS_ADMIN` | Requires explicit Provider descriptor carve-out. Denied in user domain. Requires `startRoot: true`. |
| `dac-override` | `CAP_DAC_OVERRIDE` | Permitted for processes needing file access beyond DAC. |
| `fowner` | `CAP_FOWNER` | Permitted for file ownership operations. |
| `chown` | `CAP_CHOWN` | Permitted for chown. |
| `setuid` | `CAP_SETUID` | Permitted for privilege drop after exec. Denied in user domain. |
| `setgid` | `CAP_SETGID` | Permitted for privilege drop after exec. Denied in user domain. |
| `audit-write` | `CAP_AUDIT_WRITE` | Permitted only for system-domain worker processes with explicit carve-out. |
| `kill` | `CAP_KILL` | Permitted for narrow inter-process signal use. |

For virtiofsd-class processes (those with `userNamespace` set), the compiled
capability set is always empty in the host capability set. All required
capabilities run inside the user namespace as namespace-scoped grants, not as
host capabilities. This preserves the ADR 0021 zero-host-capability invariant.
See §7.7.

### 7.3 Seccomp classes

`SandboxSpec.seccompClass` selects the seccomp policy. Values:

| Value | Meaning |
| --- | --- |
| `strict` | Minimal allow-list compiled from the process class (`controller`, `service`, `worker`) and the owning Provider component's declared syscall profile. Default for all processes. |
| `permissive` | Log-only; all syscalls permitted but audited. Requires explicit Provider descriptor carve-out. Never used in production without carve-out approval. |
| `allow-all` | No seccomp filter. Requires explicit Provider descriptor carve-out and is rejected unless the descriptor declares `seccomp-allow-all-permitted: true`. |
| `<provider-class>` | Named profile from the Provider's compiled seccomp catalog (e.g., `virtiofsd`, `swtpm`, `security-key`). Resolved at compilation time to a versioned seccomp plan digest. |

Raw BPF programs are not accepted. The compiled seccomp plan is a broker-owned
artifact addressed by its digest. The digest is stored in
`status.sandboxRevisionDigest` together with the namespace and capability plan
digests.

### 7.4 Mount compilation

`Process.spec.mounts` declares Volume mounts. For each `MountSpec` entry:

1. The controller verifies the `volumeRef` target is `Ready`.
2. The `view` field selects a named view from the Volume's declared view table.
3. The `mountPath` is an absolute path inside the sandbox.
4. The `access` field (`read-only` or `read-write`) is enforced at mount time.

The broker translates the compiled mount table into a set of bind-mount
operations applied after namespace setup. No caller-supplied absolute host path
reaches the broker. All source paths come from the Volume Provider's
implementation through the trusted ProviderSupervisor ticket. A mount whose
Volume is not Ready at launch time and whose `required: true` aborts the launch
with `volume-not-ready`.

### 7.5 Environment classes

`SandboxSpec.environmentClass` selects what environment the process receives:

| Value | Meaning |
| --- | --- |
| `minimal` | Fixed approved environment set only. Enforced at the broker exec site. No inherited variables. Default. |
| `safe-inherited` | Inherits the declared safe subset from the owning Provider's component template. The safe subset is a static allow-list signed into the component descriptor. |
| `provider-defined` | The Provider's component template defines the exact environment. No caller extension is accepted. |

No environment variable from a caller resource payload reaches exec without
passing through the trusted bundle compilation step. Credential bytes, raw
paths, and socket addresses are not environment variables.

### 7.6 cgroup placement

The broker places the process directly into its declared cgroup leaf using
`CLONE_INTO_CGROUP`. This means the process is born in its final cgroup before
any instruction executes. The cgroup leaf path follows the shape defined in
`ADR-046-components-processes-and-sandbox`:

```text
z-<zone-id>/
  executions/
    e-<execution-id>/
      system/
        providers/
          p-<provider-id>/
            components/
              c-<component-id>/
                process/
```

Intermediate cgroup nodes are process-free. The cgroup leaf is created by the
broker under the delegated cgroup subtree before clone3 is called. After process
exit and pidfd-confirmed reap, the broker removes the leaf.

The compiled cgroup path is never a public resource field, status field, log
line, audit payload, or metric label.

### 7.7 User namespace pre-establishment (ADR 0021 model)

For processes whose `SandboxSpec.userNamespace` is set, the broker
pre-establishes a single-entry user namespace before the process's first
instruction runs. This implements the ADR 0021 zero-host-capability contract
for virtiofsd-class processes.

Pre-establishment sequence:

1. Broker calls `clone3(CLONE_NEWUSER | CLONE_PIDFD | CLONE_INTO_CGROUP)` with
   the target cgroup leaf FD and `CLONE_PIDFD` to obtain the pidfd atomically.
2. The child process blocks on a pipe sync (reading before exec).
3. The effect port resolves the exact host principal UID from `mappingClass`
   (e.g., `process-principal-root` maps to the component principal UID declared
   in the Provider descriptor for this process) and writes
   `/proc/<child-pid>/uid_map` mapping that UID to in-namespace UID 0. The
   Provider crate never observes the numeric host UID.
4. Likewise for `/proc/<child-pid>/gid_map`: the effect port resolves the GID
   from `mappingClass` and writes it privately. The resolved GID must not be 0
   (host root); the effect port enforces this before any write.
5. The broker writes the pipe sync byte, unblocking the child.
6. The child proceeds to exec the target binary.

The result: the process runs as in-namespace UID/GID 0 and may hold
in-namespace capabilities without holding any host capabilities. The host
capability set for this process is zero.

`UserNamespaceSpec.mappingClass` is validated at spec admission:

- `process-principal-root` is the only defined value in the initial enumeration.
  Additional values require a descriptor update.
- At spawn time, the effect port resolves the exact host UID/GID from
  `mappingClass` by looking up the component principal declared in the Provider
  descriptor. The resolved UID/GID must not be 0 (host root); the effect port
  enforces this invariant before writing uid_map/gid_map. The Provider crate
  never receives or stores the numeric host UID or GID; numeric IDs are confined
  to the effect port implementation (core/ProviderSupervisor).

The parent name-to-inode bindings for the uid_map/gid_map writes are
revalidated: the broker does not follow symlinks and rejects any interposition
attempt between the `/proc` open and the writes.

This model applies universally to all processes requesting `user` in
`namespaceClasses`. Non-user-namespace processes never receive a user namespace,
regardless of any `SandboxSpec` combination.

---

## 8. Process lifecycle

### 8.1 LaunchTicket

The Process controller authenticates a `LaunchTicket` from the
ProviderSupervisor. The ticket is bound to:

- `Process`/`EphemeralProcess` ResourceRef, UID, spec generation, revision;
- owning Provider/component/template name and digest;
- `executionRef`, domain, and resolved `userRef`;
- selected Process Provider (`Provider/system-minijail`);
- compiled sandbox plan digest (covering namespace, capability, seccomp, mount,
  environment, userNamespace, rlimit, umask, oom classes);
- compiled budget/cgroup placement digest;
- compiled mount table digest;
- compiled network/device/endpoint configuration digest;
- inherited FD table (only the fixed bootstrap or explicitly authorized set);
- operation ID, deadline, and cancellation token;
- expected process identity and readiness predicate.

The ProviderSupervisor:

1. Verifies the ticket against the current Process/EphemeralProcess resource
   generation and controller lease.
2. Resolves only trusted package/template/resource outputs. No caller payload
   field reaches exec unvalidated.
3. Calls the injected `MinijailProcessEffectPort` with opaque
   Process/LaunchTicket/profile IDs to request process spawn.
4. Returns the stable `processIdentityDigest` to the controller.

### 8.2 Spawn via MinijailProcessEffectPort

The minijail controller calls the injected `MinijailProcessEffectPort` with
opaque Process/LaunchTicket/profile/resource IDs. The Provider crate imports no
broker service, client, or DTO. The effect port, owned by core/ProviderSupervisor,
privately resolves these IDs and delegates to the privileged broker, which
remains the sole executor and audit owner of all privileged effects. The broker:

1. Validates the request against the compiled sandbox plan digest and the
   trusted bundle.
2. Verifies the executable path, executable hash, template generation, declared
   UID/GID, and cgroup placement before any exec call.
3. Creates the cgroup leaf under the delegated subtree.
4. Calls `clone3(CLONE_PIDFD | CLONE_INTO_CGROUP)` with the exact cgroup leaf
   FD to place the process directly in its cgroup. `CLONE_PIDFD` ensures the
   pidfd is obtained atomically at spawn time with no PID-reuse race.
5. For user-namespace processes: additionally passes `CLONE_NEWUSER` and
   performs the uid_map/gid_map pre-establishment (§7.7) before releasing
   the pipe sync.
6. Returns the pidfd to ProviderSupervisor via the effect port. The pidfd FD
   number is never written to any log, status, audit record, or metric.

The broker rejects any request that does not match the precompiled and
broker-verified plan digest. No environment variable, mount, capability, or
argument fragment from the caller resource payload reaches exec without passing
through this compilation step.

### 8.3 Pidfd ownership and wait/reap

d2b (the Provider controller) owns wait/reap for all processes spawned under
this Provider.

Pidfd rules (invariant across all Process Providers; violation is a
`runtime-security-violation` audit event):

1. Every launched process has a local verified pidfd obtained atomically from
   `clone3(CLONE_PIDFD)`.
2. The pidfd is acquired only after the effect port (via the broker) verifies
   stable process identity: executable hash, template generation, cgroup
   placement, and provider-specific identity attributes.
3. The pidfd is never serialized to disk, never written to the resource store,
   never sent over d2b-bus, and never exposed in public status or audit payload.
4. The pidfd is closed and reopened (with full re-verification) after every
   controller restart. No pidfd is valid across a controller process restart.
5. On adoption, the controller locates the candidate process by cgroup leaf and
   verifies all stable identity fields against the stored
   `processIdentityDigest`, then opens a new pidfd with `pidfd_open(2)`. See §8.5.
6. The PID contained within the pidfd is used internally only for
   `pidfd_send_signal` and process group management. It is never written to any
   log, status, audit record, or metric with a resource-name label.

The controller calls `waitid(P_PIDFD, ...)` on the pidfd to receive the terminal
event. There is no polling interval. The wait is driven by the async runtime's
fd readability notification via `AsyncFd` (or equivalent); the controller
never calls a blocking `waitid` variant directly on the watch-loop task.

All operations that involve blocking or latency-unbounded syscalls — including
`pidfd_open(2)`, `/proc/<pid>/stat` reads, executable hash computation, and
cgroup filesystem enumeration (leaf existence, occupant detection) — are
dispatched through a bounded blocking adapter (`spawn_blocking` or equivalent)
with an explicit timeout, so the controller watch loop is never blocked.
Adapter timeouts are treated as adoption failures or launch errors, not
silent hangs.

On wait completion:

- Exit code is recorded internally as the basis for `lastExitClass` and
  `outcome.exitCode` (EphemeralProcess).
- The cgroup leaf is released.
- The pidfd is closed.
- The restart or finalize path is dispatched.

systemd does not own wait/reap for any process under this Provider.

### 8.4 Restart and backoff

For `Process` resources with `restartPolicy: always` or `restartPolicy:
on-failure`:

1. On process exit, classify exit: `clean-exit`, `crash`, `signal`,
   `timeout`, `unknown`.
2. Apply `restartPolicy` logic: `on-failure` skips restart on `clean-exit`.
3. Apply exponential backoff: starting at `restartPolicy.backoffBase` (Process
   spec field; e.g., `"1s"`), doubling on each consecutive crash, capped at
   `restartPolicy.backoffMax` (Process spec field; e.g., `"5m"`; maximum `"1h"`).
4. Backoff is reset to zero after a process has been running for at least one
   backoff period without exiting.
5. If `maxRestarts` is set and exceeded: write `Failed` phase;
   `reason: max-restarts-exceeded`; stop restarting.
6. Each restart increments `status.restartCount` and updates
   `status.lastRestartAt`.

`status.lastExitClass` and `status.adoptionState` are updated on each restart.

For `restartPolicy: never`: no restart. Write final phase `Succeeded` (if
`clean-exit`) or `Failed` (any other).

### 8.5 Adoption after controller restart

When the controller restarts, it performs adoption for each `Process` resource
whose `adoptionPolicy: adopt-on-restart` and current phase is non-terminal.

Adoption algorithm:

1. Locate the candidate process by cgroup leaf path. The cgroup leaf path is
   derived from the stable UID/generation/zone identifiers, not stored on disk.
2. Via a bounded blocking adapter with an explicit timeout: read
   `/proc/<pid>/stat` bytes to obtain the start-time token and PID; verify
   cgroup membership (no migration during adoption window).
3. Verify that the start-time token, executable identity, and cgroup membership
   match the `processIdentityDigest` stored in the resource status.
4. Via a bounded blocking adapter: compute the executable content hash and
   verify it against the Provider template/bundle digest.

All steps 2–4 run outside the watch-loop task. A blocking-adapter timeout is
treated as an adoption failure (ambiguous result), not a clean success.

If all checks pass: via a bounded blocking adapter, call `pidfd_open(2)` to
open a fresh pidfd. Set `adoptionState: adopted`. Continue supervising.

If any check fails or is ambiguous:

- Set `adoptionState: quarantined`.
- Do **not** attempt to kill the process.
- Do **not** reuse the PID or cgroup leaf.
- Write `Degraded` phase with reason `adoption-ambiguous` or
  `adoption-identity-mismatch`.
- Emit a `runtime-security-violation` audit event (see §12).
- Await operator review.

A quarantined process is invisible to the controller. The controller does not
send signals, claim the cgroup leaf, or allocate new resources under the
quarantined identity. Quarantine **cannot** be resolved by deleting and
recreating the Process resource while the process may still be alive. Before
the controller will accept a new finalizer registration or reuse the cgroup
leaf, the operator must establish — through means external to d2b — that the
process is definitively absent (e.g., by confirming via OS-level inspection
that no process occupies the cgroup leaf and the leaf is empty). Alternatively,
the operator may perform a destructive full Zone reset, which terminates all
resident Zone processes. d2b never sends any signal to a quarantined or
ambiguous process identity.

When `adoptionPolicy: never-adopt`, no adoption attempt is made. A prior
running process whose cgroup leaf still exists after restart is quarantined
automatically per the ambiguous-identity path above (the controller will not
claim it as fresh).

### 8.6 Stop and finalize

The owning Provider controller registers finalizer
`process.system-minijail/cleanup` on every Process and EphemeralProcess it
manages.

Finalizer algorithm on `deletion-requested`:

1. Send `SIGTERM` via `pidfd_send_signal`. Wait up to `drainTimeout` (Process
   spec field; default `"10s"`; maximum `"300s"`).
2. If the process has not exited: send `SIGKILL` via `pidfd_send_signal`.
3. Confirm process exit via `waitid(P_PIDFD, ...)`.
4. Release the cgroup leaf: remove the leaf directory under the delegated
   subtree.
5. Release any OFD locks/leases held by this process (through the Volume
   Provider, not directly by this Provider).
6. Clear the finalizer.

On ambiguous state (pidfd closed unexpectedly, cgroup leaf empty, exit not
confirmed): the finalizer is **retained**; the resource is written to `Degraded`
or `Unknown` phase with condition `process-exit-unconfirmed`. The finalizer is
never cleared without exact exit proof confirmed via pidfd. The resource remains
in this state until exact exit is confirmed or the operator performs a full Zone
reset. Recording a success-shape finalized result without exit proof is
prohibited.

---

## 9. EphemeralProcess one-shot lifecycle

An `EphemeralProcess` under Provider/system-minijail follows the same spawn
path as a `Process` — LaunchTicket, ProviderSupervisor, broker
`clone3(CLONE_PIDFD | CLONE_INTO_CGROUP)`, pidfd ownership — but has no
restart, no adoption, and a terminal TTL.

One-shot lifecycle:

1. Spec admitted and committed: phase `Pending`.
2. All dependencies Ready; `startDeadline` countdown starts.
3. LaunchTicket dispatched: condition `Launching`.
4. Process starts: condition `Running`.
5. Process exits within `runtimeDeadline`:
   - exit 0 → phase `Succeeded`, `outcome.code: process-exited`,
     `outcome.exitCode: 0`.
   - non-zero → phase `Failed`, `outcome.code: process-exited`,
     `outcome.exitCode: <N>`.
   - killed by signal → `outcome.code: process-crashed`.
6. `startDeadline` exceeded without start: phase `Failed`,
   `outcome.code: start-deadline-exceeded`.
7. `runtimeDeadline` exceeded while running: SIGTERM then SIGKILL; phase
   `Failed`, `outcome.code: runtime-deadline-exceeded`.
8. After terminal phase: cleanup controller computes `cleanupEligibleAt`:
   - `Succeeded`: `completedAt + successfulTtl`.
   - `Failed`: `completedAt + failedTtl`.
9. When `cleanupEligibleAt <= now()`, no `incidentHold`, no active finalizers:
   normal `Delete` called with expected revision.
10. The resource row and index entry are removed atomically; the `ResourceDeleted`
    audit event is appended afterward with dedup (so the audit record is the
    final observable event, appended after removal, not the trigger for removal).

Controller restart during a running EphemeralProcess triggers **continuation
recovery**: the controller locates the one-shot by cgroup leaf membership,
reverifies its process identity, opens a fresh pidfd, and attaches a waiter to
observe the exact exit. The EphemeralProcess remains in `Running` phase while
verification succeeds (no relaunch). If the candidate is ambiguous or
verification fails, the EphemeralProcess is written to `Unknown` phase and
quarantined; it is **never** auto-TTL-cleaned while the process may still be
live or ambiguous. `adoptionPolicy`/`adoptionState` do not apply; the term for
this recovery is **continuation recovery**, not adoption.

---

## 10. d2b-bus and ComponentSession

Provider/system-minijail communicates exclusively through d2b-bus over
ComponentSession. It does not hold a direct redb handle, an HTTP control plane,
or an ambient non-bus socket.

### 10.1 Session profile

The minijail controller uses the enrolled KK (`Noise_KK_25519_ChaChaPoly_SHA256`)
session profile for all post-bootstrap d2b-bus connections:

- Both static public keys known before handshake.
- Local private key is sealed/zeroizing.
- Static key registry maps the authenticated key to the `Provider/system-minijail`
  Zone-local subject.
- Prologue binds purpose, service package and schema fingerprint, route,
  limits, and reconnect generation.

During bootstrap, the one-time IKpsk2 (`Noise_IKpsk2_25519_ChaChaPoly_SHA256`)
session is used to authenticate the initial connect before enrollment:

- Single-use PSK bound to operation ID, replay nonce, expected subject, and
  expiry.
- PSK is consumed exactly once.
- Successful enrollment replaces bootstrap with an enrolled KK session.

### 10.2 Services used

| Service | Method/stream | Purpose |
| --- | --- | --- |
| `d2b.resource.v3` | `Watch`, `List`, `Get` | Watch/list Process and EphemeralProcess resources assigned to this controller instance |
| `d2b.resource.v3` | `UpdateStatus` | Write Process/EphemeralProcess status transitions |
| `d2b.resource.v3` | `UpdateFinalizers` | Clear finalizer after process exit |
| `d2b.resource.v3` | `Delete` | Delete EphemeralProcess after TTL expiry |
| `d2b.resource.v3` | `CommitBatch` | Batch status + finalizer updates in one Zone transaction |
| `d2b.controller.v3` | `RegisterController` | Register controller descriptor and watch plan |
| `d2b.controller.v3` | `ReportCheckpoint` | Report watch high-water mark |
| `d2b.supervisor.v3` | `IssueLaunchTicket`, `ReportSpawnResult`, `ReportExitEvent` | ProviderSupervisor ticket/result/exit protocol |

### 10.3 Fast path contract

After a Process resource is durably committed with all dependencies Ready:

- Store post-commit dispatcher emits a controller hint immediately after
  commit returns.
- p95 from durable commit to controller handler start: ≤5 ms.
- p95 from durable commit to launch attempt start: ≤20 ms.
- The controller launches the process in an independent async task without
  blocking the watch loop.
- The watch loop dispatches the next independent Process immediately.
- Status transitions (hint received → Launching → Ready) are async
  `UpdateStatus` calls with expected-revision preconditions; they do not hold
  the watch loop.

---

## 11. RBAC and broker operations

### 11.1 RBAC verbs on managed ResourceTypes

| Verb | ResourceType | Required grant | Notes |
| --- | --- | --- | --- |
| `get` | Process | `{resourceTypes:[Process], verbs:[get]}` | — |
| `list` | Process | `{resourceTypes:[Process], verbs:[list]}` | — |
| `watch` | Process | `{resourceTypes:[Process], verbs:[watch]}` | Controller watch stream |
| `create` | Process | `{resourceTypes:[Process], verbs:[create], executionRefs:[Host/<n>]}` | Config publication handler or owning controller only; **not** a bootstrap grant for system-minijail itself |
| `update-spec` | Process | `{resourceTypes:[Process], verbs:[update-spec]}` | Config publication handler or owning controller only; **not** a bootstrap grant for system-minijail itself |
| `update-status` | Process | `{resourceTypes:[Process], verbs:[update-status]}` | system-minijail controller only |
| `update-finalizers` | Process | `{resourceTypes:[Process], verbs:[update-finalizers]}` | system-minijail controller only |
| `delete` | Process | `{resourceTypes:[Process], verbs:[delete]}` | Blocked while finalizer exists |
| `get` | EphemeralProcess | `{resourceTypes:[EphemeralProcess], verbs:[get]}` | — |
| `list` | EphemeralProcess | `{resourceTypes:[EphemeralProcess], verbs:[list]}` | — |
| `watch` | EphemeralProcess | `{resourceTypes:[EphemeralProcess], verbs:[watch]}` | — |
| `create` | EphemeralProcess | `{resourceTypes:[EphemeralProcess], verbs:[create], executionRefs:[Host/<n>]}` | Owning controller only; **not** a bootstrap grant for system-minijail itself |
| `update-status` | EphemeralProcess | `{resourceTypes:[EphemeralProcess], verbs:[update-status]}` | system-minijail controller only |
| `update-finalizers` | EphemeralProcess | `{resourceTypes:[EphemeralProcess], verbs:[update-finalizers]}` | system-minijail controller only |
| `delete` | EphemeralProcess | `{resourceTypes:[EphemeralProcess], verbs:[delete]}` | Cleanup controller after TTL; blocked while `incidentHold` or active finalizers |

The `incident-hold-release` sub-verb is required to set `spec.incidentHold: false`
on an `EphemeralProcess` in `Failed` phase.

### 11.2 Effect port and broker operations

The minijail controller calls the injected `MinijailProcessEffectPort` with
opaque IDs; it does not hold a `d2b.broker.v3` connection and imports no broker
service, client, or DTO. The effect port implementation, owned by
core/ProviderSupervisor, privately invokes the following broker operations. The
broker remains the sole executor and audit owner:

| Operation | Purpose | Authority |
| --- | --- | --- |
| `SpawnRunner` | Clone3-spawn of a Process/EphemeralProcess with a pre-compiled sandbox plan | Scoped to the zone-delegated cgroup subtree and pre-verified plan digest |
| User namespace uid_map/gid_map write | Write UID/GID mapping for user-namespace processes | Broker-internal; always part of SpawnRunner when `userNamespace` is set |
| Cgroup leaf create/observe | Create and manage the cgroup leaf for each process | Delegated cgroup subtree only; broker validates path against `z-<zone-id>/` prefix |
| Cgroup leaf release | Remove cgroup leaf on process exit | Same delegation scope |

No direct path exists from the Provider crate to the broker socket. The
`MinijailProcessEffectPort` enforces the boundary: all spawn effects are carried
by opaque identifiers, and the effect port resolves them privately.

The broker exposes no arbitrary host-global operations through the effect port.
The broker's host-global mutation authority (firewall, network, device, storage)
is not available to this Provider.

The minijail controller writes status only on `Process` and `EphemeralProcess`
resources. `Provider/system-minijail` resource status is aggregated by core from
checkpoint and health events reported via `d2b.controller.v3`; the minijail
controller has no `Provider` update-status grant.

---

## 12. Errors

All stable error codes. No raw path, PID, argv, or capability bitmask appears
in any error message field. Message fields are redacted to a bounded safe
description.

### 12.1 Spec admission errors

| Code | Meaning |
| --- | --- |
| `user-namespace-missing-spec` | `namespaceClasses` includes `user` but `userNamespace` is null |
| `user-namespace-mapping-class-unknown` | `userNamespace.mappingClass` is not a recognized semantic class value |
| `network-namespace-with-network-ref` | `namespaceClasses` includes `network` and `networkUsage.networkRef` is non-null |
| `cgroup-namespace-unsupported` | `namespaceClasses` includes `cgroup` but Host lacks `cgroup-v2` capability |
| `user-domain-not-supported` | `domain: user` but the Provider component descriptor does not declare user-domain support for this Host/Guest placement |
| `capability-class-denied-user-domain` | `capabilityClasses` contains a class denied in user domain (`network-admin`, `sys-admin`, `setuid`, `setgid`) |
| `seccomp-allow-all-not-permitted` | `seccompClass: allow-all` without descriptor carve-out |
| `start-root-user-domain` | `startRoot: true` combined with `domain: user` |
| `start-root-without-carve-out` | `startRoot: true` without explicit Provider descriptor carve-out |
| `provider-class-unknown` | `seccompClass` names a class not in the Provider's compiled catalog |
| `budget-exceeds-execution-target` | Per-process budget fields exceed the executionRef aggregate remainder |
| `volume-domain-mismatch` | A `MountSpec` Volume's `sensitivityClass` is incompatible with the process domain/userRef |
| `execution-ref-not-ready` | `executionRef` target is not in Ready phase at admission time |
| `provider-not-ready` | `Provider/system-minijail` is not in Ready phase |
| `template-not-found` | `template` ID does not resolve in the owning Provider's component descriptor |

### 12.2 Launch errors

| Code | Meaning |
| --- | --- |
| `sandbox-plan-digest-mismatch` | Compiled plan digest at launch time differs from the digest at ticket issue time |
| `executable-hash-mismatch` | Binary content hash does not match the bundle-pinned template digest |
| `cgroup-leaf-create-failed` | Broker could not create the cgroup leaf under the delegated subtree |
| `clone3-failed` | Kernel returned an error from `clone3` |
| `user-ns-uid-map-failed` | Writing `uid_map` failed during user namespace setup |
| `user-ns-gid-map-failed` | Writing `gid_map` failed during user namespace setup |
| `broker-spawn-denied` | Broker refused the SpawnRunner request (admission check failed) |
| `volume-not-ready` | A required Volume mount is not Ready at launch time |
| `launch-ticket-expired` | LaunchTicket deadline exceeded before spawn completed |
| `launch-ticket-revoked` | LaunchTicket revoked by controller generation change |

### 12.3 Runtime and adoption errors

| Code | Meaning |
| --- | --- |
| `start-deadline-exceeded` | EphemeralProcess did not start within `startDeadline` |
| `runtime-deadline-exceeded` | Process or EphemeralProcess exceeded `runtimeDeadline` |
| `max-restarts-exceeded` | Process reached `restartPolicy.maxRestarts` |
| `adoption-ambiguous` | Multiple processes found in the cgroup leaf on adoption |
| `adoption-identity-mismatch` | Process identity does not match stored `processIdentityDigest` |
| `adoption-failed` | Adoption attempted but could not open pidfd after verification |
| `runtime-security-violation` | Pidfd invariant violated; emits audit event and quarantines |
| `process-exit-unconfirmed` | Process exit could not be confirmed via pidfd during finalize; finalizer retained; resource reports `Degraded`/`Unknown` pending operator resolution or full Zone reset |

---

## 13. Process and EphemeralProcess status additions

The following fields are specific to `Provider/system-minijail` as the
implementation. Per D088, ResourceType-common Process/EphemeralProcess
observation written by system-minijail lives in `status.resource`, while
bounded non-secret minijail-only observation lives in `status.provider.details`
with `providerRef: Provider/system-minijail`, qualified schema IDs
`system-minijail.d2b.io/Process/status` or
`system-minijail.d2b.io/EphemeralProcess/status`, `schemaVersion` (semver MAJOR.MINOR),
`observedProviderGeneration`, and a strict unknown-field-denied, ≤32 KiB,
redacted schema registered and signed in the Provider manifest. The controller
writes all present layers atomically in one status mutation, and shared fields
are promoted to `status.resource` rather than duplicated into
`status.provider`.

### Process status values

| Layer/path | Field | Written value |
| --- | --- | --- |
| `status.resource` | `providerImplementation` | `"system-minijail"` when required by cross-provider Process consumers |
| `status.resource` | `waitReapOwner` | `"d2b"` |
| `status.provider.details` | `sandboxRevisionDigest` | Opaque hex digest of the compiled sandbox plan (namespace + capability + seccomp + mount + environment + userNamespace + rlimit + umask classes and version). Max 128 chars. |
| `status.provider.details` | `adoptionState` | One of `adopted`, `fresh`, `quarantined`, `adoption-failed`. |

### EphemeralProcess status values

| Layer/path | Field | Written value |
| --- | --- | --- |
| `status.resource` | `providerImplementation` | `"system-minijail"` when required by cross-provider EphemeralProcess consumers |
| `status.resource` | `waitReapOwner` | `"d2b"` |
| `status.provider.details` | `sandboxRevisionDigest` | Same as Process. |
| `status.provider.details` | `cleanupEligibleAt` | Set after terminal phase + TTL; RFC 3339 UTC. |
| `status.provider.details` | `incidentHeld` | Mirrors `spec.incidentHold` at last reconcile. |

No PID, pidfd file descriptor number, cgroup leaf path, mount table entry,
socket address, argv, environment variable, capability bitmask, seccomp BPF
program fragment, or raw broker diagnostic appears in any status field.

The minijail controller writes status only on `Process` and `EphemeralProcess`
resources. `Provider/system-minijail` resource status is aggregated by core;
the minijail controller has no `Provider` update-status grant.

---

## 14. Audit events

All audit records are committed before the operation they describe completes.
Audit is distinct from OTEL telemetry (§15). No OTEL pipeline carries audit
payloads.

### 14.1 Event catalog

| Event kind | Trigger | Required fields |
| --- | --- | --- |
| `ProcessLaunched` | Process transitions to Running (pidfd obtained) | `zone`, `resource_ref`, `resource_uid`, `resource_generation`, `provider`, `component`, `operation_id`, `subject_digest`, `execution_ref`, `domain`, `sandbox_plan_digest`, `adoption_state` |
| `ProcessExited` | Wait-confirmed process exit | `zone`, `resource_ref`, `resource_uid`, `provider`, `operation_id`, `exit_class`, `restart_count` |
| `ProcessAdopted` | Successful adoption after controller restart | `zone`, `resource_ref`, `resource_uid`, `provider`, `operation_id`, `subject_digest`, `adoption_state: adopted` |
| `ProcessQuarantined` | Ambiguous or mismatched adoption | `zone`, `resource_ref`, `resource_uid`, `provider`, `operation_id`, `reason` (one of `adoption-ambiguous`, `adoption-identity-mismatch`) |
| `ProcessFinalized` | Finalizer cleared after SIGTERM/SIGKILL/wait sequence | `zone`, `resource_ref`, `resource_uid`, `provider`, `operation_id`, `exit_confirmed: bool` |
| `EphemeralProcessLaunched` | EphemeralProcess transitions to Running | Same fields as `ProcessLaunched` |
| `EphemeralProcessCompleted` | EphemeralProcess reaches terminal phase | `zone`, `resource_ref`, `resource_uid`, `provider`, `operation_id`, `outcome_code`, `exit_code?` (only when `outcome_code: process-exited`) |
| `EphemeralProcessCleanupInitiated` | EphemeralProcess cleanup controller calls Delete | `zone`, `resource_ref`, `resource_uid`, `operation_id`, `cleanup_eligible_at` |
| `SandboxPlanCompiled` | Sandbox plan compiled before LaunchTicket issue | `zone`, `resource_ref`, `resource_uid`, `provider`, `sandbox_plan_digest`, `namespace_classes`, `seccomp_class` (class name only — no BPF), `user_namespace: bool`, `subject_digest` |
| `runtime-security-violation` | Any pidfd invariant violated | `zone`, `resource_ref`, `resource_uid`, `provider`, `violation_class`, `subject_digest`, `operation_id` |
| `BootstrapAuthorizationUsed` | Bootstrap authorization used for a resource verb | `zone`, `provider`, `operation_id`, `verb`, `resource_type`, `subject_digest` |

### 14.2 Redaction rules

The following fields must never appear in any audit record, log line, OTEL span
attribute, or metric label:

- PID or pidfd file descriptor number;
- cgroup leaf path;
- executable path or argv;
- raw seccomp BPF program bytes;
- capability bitmask;
- environment variable name or value;
- mount source path;
- uid_map/gid_map raw content;
- credential bytes;
- socket address or file descriptor number;
- resource name combined with subject name in a single audit field.

The `sandbox_plan_digest` field is an opaque hex string. It identifies the
compiled plan version without exposing implementation details.

The `exit_code` field is included only in `EphemeralProcessCompleted` when
`outcome_code: process-exited`. It is a bounded integer. It is never included
in metric labels.

---

## 15. Telemetry

### 15.1 SDK placement

Provider/system-minijail uses the lightweight bounded emitter (`tracing` crate
plus bounded in-process ring) to push frames over the Zone's local private
OTEL socket. It carries no `opentelemetry_sdk` or `opentelemetry-otlp`
dependency. Frames are drained and forwarded by `Provider/observability-otel`
when installed.

If `Provider/observability-otel` is absent or unready: the emitter ring fills
and oldest frames are dropped. `d2b_telemetry_drop_total` increments. Audit is
unaffected.

### 15.2 OTEL resource attributes

All v3 target additions are advisory; re-stamped authoritatively at the trusted
ingress boundary:

| Attribute | Value | Rule |
| --- | --- | --- |
| `service.name` | `d2b-provider-system-minijail` | Required |
| `service.version` | `CARGO_PKG_VERSION` | Required |
| `d2b.zone` | Zone name string | Advisory; not a metric label value |
| `d2b.provider` | `system-minijail` | Required for Provider processes |
| `d2b.component` | `minijail-controller` | Required |

The existing baseline attributes (`vm.name`, `vm.env`, `vm.role`, `host.name`,
`service.namespace`) are preserved in the allowlist and may be set by processes
supervised by this Provider.

No attribute outside the allowlist defined in
`ADR-046-telemetry-audit-and-support` may be stamped by this Provider.

### 15.3 Metric labels (closed set)

Metrics exposed by the minijail controller use only closed label sets.
**No metric uses a resource name, process name, subject name, PID, path,
capability, or argv as a label value.**

| Metric | Labels | Description |
| --- | --- | --- |
| `d2b_minijail_process_starts_total` | `{domain, outcome}` | Total process start attempts; `outcome` ∈ `{success, launch-failed, rejected}` |
| `d2b_minijail_process_restarts_total` | `{domain, exit_class}` | Total process restarts; `exit_class` ∈ `{clean-exit, crash, signal, timeout, unknown}` |
| `d2b_minijail_process_adoptions_total` | `{domain, adoption_state}` | Total adoption outcomes; `adoption_state` ∈ `{adopted, quarantined, adoption-failed, fresh}` |
| `d2b_minijail_process_active` | `{domain}` | Gauge: current non-terminal Process count |
| `d2b_minijail_ephemeral_starts_total` | `{domain, outcome_code}` | EphemeralProcess start attempts; `outcome_code` per §9 |
| `d2b_minijail_sandbox_compile_duration_seconds` | `{seccomp_class, user_namespace}` | Histogram: sandbox plan compilation latency |
| `d2b_minijail_launch_latency_ms` | `{domain}` | Histogram: hint-to-launch-attempt latency; gates ≤20 ms p95 |
| `d2b_minijail_hint_latency_ms` | `{component}` | Histogram: commit-to-hint latency; gates ≤5 ms p95 |
| `d2b_minijail_concurrent_launches` | `{component}` | Gauge: current inflight LaunchTickets |
| `d2b_telemetry_drop_total` | `{component}` | Telemetry ring overflow drops |

Label value constraints:

- `domain`: `system` or `user`.
- `outcome`, `adoption_state`, `exit_class`, `outcome_code`: closed enumerations as defined in §12 and §9.
- `seccomp_class`: class name string from the closed `seccompClass` enumeration or `<provider-class>` name (bounded identifier).
- `user_namespace`: `true` or `false`.
- `component`: `minijail-controller`.

No label contains a resource name, subject name, PID, capability bitmask, path,
or any compound identifier.

### 15.4 Async latency gate

The following latency thresholds are enforced as test pass/fail gates in the
integration test suite (see §18):

| Gate | Threshold | Metric |
| --- | --- | --- |
| p95 commit-to-handler-start | ≤5 ms | `d2b_minijail_hint_latency_ms` |
| p95 handler-to-launch-attempt | ≤20 ms | `d2b_minijail_launch_latency_ms` |

---

## 16. Nix configuration

### 16.1 Installing Provider/system-minijail

Provider/system-minijail is a system Provider. It is fixed in the bootstrap
sequence and is present by default in every Zone runtime. The `Provider/system-minijail`
resource itself is **runtime-created** by the core-controller during Zone
bootstrap (`managedBy: controller`); it is never authored in Nix by the
operator.

The operator's only Nix declaration is the artifact catalog entry, which
supplies the package derivation:

```nix
d2b.artifacts.system-minijail = {
  package = packages.d2b-provider-system-minijail;
  type    = "provider";
};
```

`Provider.spec.config` is fixed empty for this Provider. The operator does not
set any config fields. Process lifecycle defaults (drain timeout, restart
backoff base/max) are set in the `Process` or `EphemeralProcess` spec; fixed
manifest bounds are not operator-configurable.

The `d2b.artifacts.system-minijail` catalog entry is validated at build time:
`type` must be `"provider"`. The rendered artifact reference in the Provider
resource contains only the bounded `artifactId` string — not any Nix store path.

### 16.2 Selecting Provider/system-minijail for a Process

```nix
d2b.zones.dev.resources.virtiofsd-work = {
  type = "Process";
  spec = {
    providerRef   = "Provider/system-minijail";
    executionRef  = "Host/host-system";
    domain        = "system";
    processClass  = "worker";
    template      = "virtiofsd";
    ownerRef      = "Provider/volume-virtiofs";   # set via metadata not spec; shown here for clarity
    sandbox = {
      namespaceClasses  = ["user" "mount" "pid"];
      capabilityClasses = [];
      seccompClass      = "virtiofsd";
      noNewPrivileges   = true;
      startRoot         = false;
      environmentClass  = "minimal";
      readOnlyRoot      = true;
      userNamespace = {
        mappingClass = "process-principal-root";
      };
    };
    budget = {
      memory = { request = "32Mi"; limit = "128Mi"; };
      pids   = { limit = 32; };
    };
    mounts = [
      {
        volumeRef  = "Volume/work-store";
        view       = "ro-store";
        mountPath  = "/store";
        access     = "read-only";
        required   = true;
      }
    ];
  };
};
```

Rendered canonical JSON (excerpt):

```json
{
  "apiVersion": "resources.d2b.io/v3",
  "type": "Process",
  "metadata": {
    "name": "virtiofsd-work",
    "zone": "dev"
  },
  "spec": {
    "providerRef": "Provider/system-minijail",
    "executionRef": "Host/host-system",
    "domain": "system",
    "processClass": "worker",
    "template": "virtiofsd",
    "sandbox": {
      "namespaceClasses": ["user", "mount", "pid"],
      "capabilityClasses": [],
      "seccompClass": "virtiofsd",
      "noNewPrivileges": true,
      "startRoot": false,
      "environmentClass": "minimal",
      "readOnlyRoot": true,
      "userNamespace": { "mappingClass": "process-principal-root" }
    },
    "budget": {
      "memory": { "request": "32Mi", "limit": "128Mi" },
      "pids": { "limit": 32 }
    },
    "mounts": [
      {
        "volumeRef": "Volume/work-store",
        "view": "ro-store",
        "mountPath": "/store",
        "access": "read-only",
        "required": true
      }
    ]
  }
}
```

No store path appears in the rendered JSON. The `template` field is a plain
bounded ID. No raw capability list, seccomp BPF, minijail argument string,
cgroup path, or socket address appears.

### 16.3 EphemeralProcess example

```nix
d2b.zones.dev.resources.swtpm-flush-abc123 = {
  type = "EphemeralProcess";
  spec = {
    providerRef      = "Provider/system-minijail";
    executionRef     = "Host/host-system";
    domain           = "system";
    processClass     = "worker";
    template         = "swtpm-flush";
    sandbox = {
      namespaceClasses  = ["pid" "mount"];
      capabilityClasses = [];
      seccompClass      = "swtpm";
      noNewPrivileges   = true;
      startRoot         = false;
      environmentClass  = "minimal";
      readOnlyRoot      = true;
    };
    startDeadline    = "60s";
    runtimeDeadline  = "120s";
    successfulTtl    = "1h";
    failedTtl        = "24h";
    incidentHold     = false;
  };
};
```

### 16.4 Eval-time validation rules

The Nix compiler enforces at eval time:

1. `providerRef` resolves to a declared `Provider/system-minijail` resource in
   the same Zone.
2. `executionRef` resolves to a declared `Host/<name>` or `Guest/<name>` in the
   same Zone.
3. `domain` is in `executionRef.allowedDomains`.
4. When `domain: user`, either `userRef` is set or the execution target has
   `defaultUserRef` set.
5. `sandbox.namespaceClasses` contains `user` only if `sandbox.userNamespace`
   is set.
6. `sandbox.userNamespace.mappingClass` is a recognized semantic class value
   (closed enumeration; currently only `process-principal-root`).
7. `sandbox.seccompClass` is one of the closed enum values or a named class
   identifier (bounded string).
8. No inline secret byte, raw host path, capability bitmask, or socket address
   appears in any spec field.
9. `processClass: controller` or `service` on an EphemeralProcess is rejected.

Missing required fields produce actionable eval errors with source location.

### 16.5 Build-time validation

The build:

1. Renders the canonical JSON ResourceSpec.
2. Validates it against the committed ResourceTypeSchema
   (`docs/reference/schemas/v3/Process.json` and `EphemeralProcess.json`).
3. Validates `spec.sandbox` against the signed Provider schema extension for
   minijail-specific fields.
4. Verifies no Nix store path appears in any rendered field.
5. Verifies two identical configs produce byte-identical generation IDs.

---

## 17. Hard bounds

| Bound | Value | Enforced by |
| --- | --- | --- |
| Maximum concurrent inflight LaunchTickets per Zone | 64 (fixed manifest bound; not operator-configurable) | Controller semaphore; excess queued |
| LaunchTicket TTL | `spec.startDeadline` (1s..3600s; default 60s) | Controller ticker |
| Maximum runtimeDeadline | `spec.runtimeDeadline` max 86400s | Spec admission |
| Maximum failedTtl | 30 days | Spec admission |
| Maximum successfulTtl | 7 days | Spec admission |
| Maximum `drainTimeout` (Process spec field) | 300s | Spec admission; fixed bound |
| Maximum `restartPolicy.backoffMax` (Process spec field) | 1h | Spec admission; fixed bound |
| `processIdentityDigest` length | 128 chars | Status field |
| `sandboxRevisionDigest` length | 128 chars | Status field |
| `namespaceClasses` items | 0..8 unique | Spec admission |
| `capabilityClasses` items | 0..16 unique | Spec admission |
| `seccompClass` string length | 64 chars | Spec admission |
| `mounts` items | 0..64 | Spec admission |
| `template` length | 63 chars | Spec admission |
| `userNamespace.mappingClass` string length | 64 chars | Spec admission |

---

## 18. Test inventory

Each test path corresponds to a required file within the Provider crate layout
defined in `ADR-046-provider-model-and-packaging` and enforced by workspace
policy.

### 18.1 `src/` — colocated unit tests

Every module in `src/` includes `#[cfg(test)]` unit tests for:

- `sandbox_compiler.rs`: SandboxSpec → compiled plan round-trips; every
  NamespaceClass, CapabilityClass, and SeccompClass combination; user namespace
  block with valid/invalid mappingClass; every rejection condition in §12.1.
- `launch.rs`: LaunchTicket construction and verification; digest binding;
  expired/revoked ticket paths.
- `adoption.rs`: fresh adoption, successful identity match, ambiguous/multiple
  candidates, identity mismatch, quarantine path; blocking-adapter timeout →
  adoption-failed; quarantine reuse rejected without externally established proof.
- `pidfd.rs`: pidfd close/reopen on controller restart; never-serialized
  invariant assertion; mock broker pidfd return; `pidfd_open` via blocking
  adapter with timeout.
- `wait.rs`: async wait completion via `AsyncFd` fd readability; no blocking
  `waitid` on watch-loop task; clean-exit/crash/signal classification; SIGKILL
  fallback on SIGTERM timeout.
- `user_ns.rs`: uid_map/gid_map write sequence; pipe sync ordering; host UID 0
  rejection.
- `metrics.rs`: no `zone` label in any emitted metric; closed label set
  enforcement; label value bounds.

### 18.2 `tests/` — hermetic Cargo integration tests

Files:

```
tests/
  sandbox_compilation.rs    # full SandboxSpec → plan round-trips against golden vectors
  lifecycle.rs              # Process: start → ready → crash → restart → stop
  ephemeral_lifecycle.rs    # EphemeralProcess: start → succeed/fail → ttl → cleanup
  conformance.rs            # shared conformance matrix (run against fake broker/supervisor)
  fault_injection.rs        # broker failures, pidfd errors, cgroup errors, user-ns errors
  redaction.rs              # no PID/path/cap/argv in status/audit/metrics; no zone label
  schema.rs                 # rendered JSON validates against ResourceTypeSchema
  fast_path.rs              # ≤5 ms hint / ≤20 ms launch latency gates (1/10/100 concurrent)
  adoption_quarantine.rs    # adoption identity mismatch → quarantine, no kill; blocking-adapter timeout → adoption-failed; quarantine reuse rejected without external proof
  bootstrap_authz.rs        # bootstrap authorization scope; no widening; wrong subject fails
  status_state.rs           # status-first operational state: controller declares no state Volume; bounded observations written to status/core ledger within status bounds; no secret/path/argv/PID/unit content; restart re-derives observed state from cgroup leaves + fresh pidfds
  blocking_adapter.rs       # /proc reads, pidfd_open, cgroup ops via bounded blocking adapter; timeout → error, not hang
```

All tests pass under `cargo test -p d2b-provider-system-minijail`.

### 18.3 `integration/` — container and broker integration scenarios

Files:

```
integration/
  clone3_pidfd/             # clone3 CLONE_PIDFD | CLONE_INTO_CGROUP on a real cgroup leaf
  user_namespace/           # effect port user namespace pre-establishment; virtiofsd fixture
  adoption_restart/         # controller restart → adopt live process → verify digest; blocking-adapter path
  quarantine_scenario/      # identity mismatch on restart → quarantine, no signal; external proof required before reuse
  ephemeral_ttl/            # EphemeralProcess TTL and cleanup in real broker fixture
  concurrent_launch/        # fixed concurrency bound semaphore; 100 parallel launches
  latency_gate/             # ≤20 ms p95 launch-attempt start gate with real broker
  user_domain/              # user-domain Process via user supervisor (if descriptor declares support)
  status_state_restart/     # controller starts with no state Volume; reaches Ready from status/core ledger; restart re-derives observed process state from declared cgroup leaves + fresh pidfds; no state-Volume mount
```

Each integration scenario:

- is invoked by the existing repository test orchestration (`make
  test-integration`);
- declares its fixture dependencies within its scenario directory;
- does not modify global host state or mount namespaces outside its declared
  fixture scope.

### 18.4 Required conformance coverage (shared with Provider/system-systemd)

The shared process conformance suite (`packages/d2b-process-conformance/src/`)
is run against both system-minijail and system-systemd providers:

| Scenario | system-minijail assertion |
| --- | --- |
| Start → Ready | pidfd obtained atomically via `clone3(CLONE_PIDFD)` |
| Crash → restart | `waitReapOwner: "d2b"`; backoff applies |
| Drain: SIGTERM → exit | drainTimeout enforced; pidfd-confirmed exit |
| Drain: SIGTERM → SIGKILL | SIGKILL via pidfd after timeout |
| Adoption: matching identity | `adoptionState: adopted`; new pidfd opened via blocking adapter |
| Adoption: mismatched identity | `adoptionState: quarantined`; no signal; external proof required before reuse |
| EphemeralProcess: Succeeded TTL | `cleanupEligibleAt` set; row removed on Delete |
| EphemeralProcess: Failed TTL | `failedTtl` applied |
| SandboxSpec virtiofsd profile | user namespace pre-established; zero host caps |
| Fast path: 1 process | p95 ≤20 ms |
| Fast path: 100 concurrent | p95 ≤20 ms; no queue starvation |
| PID never in status | No PID/pidfd in any status/audit/log |
| No static template units | No PID1 unit for any process |
| No zone metric labels | No `zone` label on any emitted metric; Zone is OTEL resource attribute only |
| Blocking-adapter isolation | `/proc` read, executable hash, cgroup enum, `pidfd_open` never block watch loop |
| Effect port boundary | Provider crate imports no broker service/client/DTO; all spawn effects via `MinijailProcessEffectPort` with opaque IDs |
| Provider status by core | Minijail controller writes no `Provider` resource status; core aggregates from checkpoint/health events |
| No state Volume | The minijail controller declares no Provider state Volume; bounded non-secret operational state lives in `status`/the core Operation ledger (D087); no bootstrap state Volume, no bootstrap storage mechanism, and no bootstrap-storage exception (D086 superseded by D087); running units re-adopted from cgroup leaves + fresh pidfds on restart |

---

## 19. Current-code reuse ledger

All evidence classes use the definitions from
`ADR-046-current-code-migration-map` (§0 Purpose and Notation).

The baseline is `b5ddbed67867d9244bf33390868101bd9b053e49`.

| Current symbol / path | Evidence class | Action | Destination |
| --- | --- | --- | --- |
| `packages/d2b-core/src/processes.rs` — `ProcessRole` (18 variants), `ProcessNode`, `RoleProfile`, `NamespaceSet`, `MountPolicy`, `CgroupPlacement`, `ReadinessPredicate` | production-reachable | EXTRACT/ADAPT | `packages/d2b-provider-system-minijail/src/sandbox_compiler.rs` — namespace/cap/mount class compilation; `packages/d2b-process/src/` — common spec types |
| `packages/d2b-core/src/minijail_profile.rs` — `MinijailProfile`, `UserNamespaceProfile`, `NamespaceSet`, `MountPolicy`, `BindMount`, `CgroupPlacement` | production-reachable | EXTRACT/ADAPT | `packages/d2b-provider-system-minijail/src/sandbox_compiler.rs` — compiled plan types; preserve typed fail-closed profile verification |
| `packages/d2b-core/src/process_builder.rs` | production-reachable | ADAPT | `packages/d2b-provider-system-minijail/src/launch.rs` — LaunchTicket builder adapted to v3 ticket contract |
| `packages/d2b-priv-broker/src/ops/spawn_runner.rs` | production-reachable | ADAPT | Broker-side: retained as internal broker op invoked by `MinijailProcessEffectPort` implementation (owned by core/ProviderSupervisor); Provider-side: `packages/d2b-provider-system-minijail/src/launch.rs` calls `MinijailProcessEffectPort` with opaque IDs; Provider crate imports no broker service/client/DTO |
| `packages/d2bd/src/supervisor/pidfd_table.rs` — `PidfdTable`, `PidfdEntry`, `PidfdRegistration`, `WaitTermination`, `BrokerReapLog` | production-reachable | EXTRACT/ADAPT | `packages/d2b-provider-system-minijail/src/pidfd.rs`, `wait.rs` — pidfd ownership, async wait, never-serialized invariant |
| `packages/d2bd/src/supervisor/*.rs` — `DagExecutor`, `NodeOutcome`, `NodeHistory`, `NodeBudget`, `SplitReadinessMode` | production-reachable | ADAPT | `packages/d2b-provider-system-minijail/src/adoption.rs` — adoption/quarantine algorithm adapted from DAG supervisor restart logic |
| `packages/d2b-priv-broker/src/ops/swtpm_dir.rs` — user namespace uid_map/gid_map write sequence | production-reachable | ADAPT | `packages/d2b-provider-system-minijail/src/user_ns.rs` — pre-establishment sequence; preserve pipe sync, O_NOFOLLOW, re-validation |
| `packages/d2b-realm-core/src/ids.rs` — `RealmId`, `WorkloadId`, `PrincipalId` | production-reachable | ADAPT | Use v3 `ZoneId`, `ResourceRef`, `UserRef` from `d2b-contracts/src/v3/identity.rs` (ADR046-identities-001) |
| `packages/d2b-realm-core/src/workload.rs` — `WorkloadProviderKind`, `IsolationPosture`, `WorkloadExecutionPosture` | production-reachable | DELETE at cutover | Replaced by `Host`/`Guest`/`ExecutionPolicy`; evidence for `UnsafeLocal` → user-only Host mapping retained in migration map |
| `packages/d2b-core/src/storage.rs` — `StoragePathSpec` | production-reachable | Not consumed | Provider/system-minijail declares no state Volume; bounded non-secret operational state lives in `status`/the core Operation ledger (D087); no state-Volume creation or reconciliation on any path |
| `packages/d2b-realm-router` session types | dead-reachable | DELETE | Replaced by ComponentSession (`d2b-session`, `a1cc0b2d` reuse) |
| `packages/d2b-realm-transport` `LocalTcpTransport` | test-only | DELETE | No live socket; test conformance vectors replaced by v3 session tests |
| `packages/d2bd/src/realm_stubs.rs` | dead-reachable (explicitly dead_code-allowed) | DELETE | Stubs removed after v3 ComponentSession/bus integration |
| Main reuse `a1cc0b2d`: `packages/d2b-session/src/{handshake,bootstrap,record,engine,scheduler,streams,lifecycle,transport}.rs` | — (main, not baseline) | COPY/ADAPT | `packages/d2b-session/` — KK enrolled session for post-bootstrap bus; IKpsk2 for bootstrap; exact vectors preserved |
| Main reuse `a1cc0b2d`: `packages/d2b-session-unix/src/{adapter,socket,descriptor,pidfd,vsock,credit}.rs` | — (main, not baseline) | COPY/ADAPT | `packages/d2b-session-unix/` — Unix peer evidence, CLOEXEC FD validation |

No symbol from `d2b-realm-router` implementation types or `d2b-realm-transport`
live sockets is carried into v3 as architecture. Main `a1cc0b2d` ADR 0045
Provider types, endpoint roles, service inventory, realm process model, and
delivery assumptions are not copied.

---

## 20. Implementation work items

### ADR046-minijail-001 (Dependency: ADR046-process-001, ADR046-provider-001)

| Field | Value |
| --- | --- |
| Dependency/owner | `ADR046-process-001` (common spec/status types); `ADR046-provider-001` (toolkit/contracts); system-minijail Provider owner |
| Current source | `d2b-core/src/minijail_profile.rs`; `d2b-core/src/processes.rs` (NamespaceSet, MountPolicy, CgroupPlacement); `d2b-priv-broker/src/ops/spawn_runner.rs` |
| Reuse action | EXTRACT/ADAPT |
| Destination | `packages/d2b-provider-system-minijail/src/sandbox_compiler.rs` |
| Detailed design | Accept `SandboxSpec` from common contracts; compile NamespaceClass/CapabilityClass/SeccompClass/UserNamespaceSpec/mount/environment/rlimit/umask into a versioned `CompiledSandboxPlan`; compute `sandboxRevisionDigest`; all rejection conditions from §12.1; no raw bitmask/BPF/argv/path in any output type; golden round-trip test vectors |
| Integration | LaunchTicket builder (ADR046-minijail-002); effect port integration (ADR046-minijail-003) |
| Data migration | Full reset; current `MinijailProfile` not import-compatible with v3 SandboxSpec |
| Validation | `tests/sandbox_compilation.rs`; `tests/schema.rs`; golden vectors |
| Removal proof | Current `MinijailProfile`/`NamespaceSet` types in `d2b-core` removed after all callers migrate to SandboxSpec |

### ADR046-minijail-002 (Dependency: ADR046-minijail-001, ADR046-process-001)

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-minijail-001; common `LaunchTicket` contract |
| Current source | `d2b-core/src/process_builder.rs`; `d2bd/src/supervisor/*.rs` (ticket generation) |
| Reuse action | ADAPT |
| Destination | `packages/d2b-provider-system-minijail/src/launch.rs` |
| Detailed design | LaunchTicket construction with compiled sandbox/budget/mount digests; ticket verification on ProviderSupervisor receipt; `d2b.supervisor.v3/IssueLaunchTicket` service call; expired/revoked/malformed ticket rejection |
| Integration | `ProviderSupervisor` local adapter; minijail controller (ADR046-minijail-005) |
| Data migration | None |
| Validation | `tests/lifecycle.rs`; `tests/fault_injection.rs`; `tests/fast_path.rs` |
| Removal proof | Current `process_builder.rs` removed after parity |

### ADR046-minijail-003 (Dependency: ADR046-minijail-001)

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-minijail-001; broker integration owner |
| Current source | `d2b-priv-broker/src/ops/spawn_runner.rs`; `d2b-priv-broker/src/sys.rs` (`clone3_spawn_runner`, user namespace setup) |
| Reuse action | ADAPT |
| Destination | Broker-side: `d2b-priv-broker` retains `SpawnRunner` op, invoked by the `MinijailProcessEffectPort` implementation owned by core/ProviderSupervisor; Provider-side: `packages/d2b-provider-system-minijail/src/launch.rs` calls `MinijailProcessEffectPort` with opaque Process/LaunchTicket/profile IDs; `user_ns.rs` implements the user namespace pre-establishment protocol |
| Detailed design | `clone3(CLONE_PIDFD | CLONE_INTO_CGROUP)` with pre-declared cgroup leaf FD; user namespace pre-establishment sequence (§7.7) when `userNamespace` set; host UID 0 rejection; parent name-to-inode re-validation; zero-host-capability invariant (ADR 0021); `MinijailProcessEffectPort` privately maps opaque IDs to SpawnRunner/OpenDevice/clone3/uid-map/FD effects; Provider crate imports no broker service/client/DTO |
| Integration | ADR046-minijail-002 (LaunchTicket); real cgroup/broker fixture in `integration/clone3_pidfd/` and `integration/user_namespace/` |
| Data migration | None |
| Validation | `tests/fault_injection.rs`; `integration/clone3_pidfd/`; `integration/user_namespace/` |
| Removal proof | Old broker `SpawnRunner` direct-caller paths in `d2bd` removed after system-minijail Provider integration |

### ADR046-minijail-004 (Dependency: ADR046-minijail-003)

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-minijail-003; wait/pidfd owner |
| Current source | `d2bd/src/supervisor/pidfd_table.rs` (PidfdTable, WaitTermination, BrokerReapLog) |
| Reuse action | EXTRACT/ADAPT |
| Destination | `packages/d2b-provider-system-minijail/src/pidfd.rs`; `packages/d2b-provider-system-minijail/src/wait.rs` |
| Detailed design | Async `waitid(P_PIDFD)` via `AsyncFd` fd readability; no blocking `waitid` on watch-loop task; `pidfd_open(2)` dispatched through bounded blocking adapter with explicit timeout; pidfd never serialized; pidfd close/reopen after controller restart; exit class classification (clean-exit/crash/signal/timeout/unknown); SIGTERM/SIGKILL via pidfd_send_signal; drainTimeout enforcement |
| Integration | Controller restart → adoption (ADR046-minijail-005); finalize (§8.6) |
| Data migration | None |
| Validation | `tests/lifecycle.rs`; `tests/redaction.rs` (PID never in log/status/audit); `tests/blocking_adapter.rs` (pidfd_open via adapter; timeout → error) |
| Removal proof | Old `PidfdTable` in `d2bd` supervisor removed after Provider integration |

### ADR046-minijail-005 (Dependency: ADR046-minijail-002, ADR046-minijail-004, ADR046-session-001, ADR046-bus-001)

| Field | Value |
| --- | --- |
| Dependency/owner | All ADR046-minijail-00{1..4}; ComponentSession/d2b-bus (ADR046-session-001, ADR046-bus-001); bootstrap authz |
| Current source | `d2bd/src/supervisor/*.rs` (DagExecutor, NodeOutcome); `d2bd/src/supervisor/pidfd_table.rs`; `d2b-realm-core/src/allocator_engine.rs` (adoption/identity concepts) |
| Reuse action | ADAPT |
| Destination | `packages/d2b-provider-system-minijail/src/` — controller binary entry point; reconcile loop; adoption; quarantine; bootstrap authz; health/status; restart; finalize |
| Detailed design | Full Process/EphemeralProcess reconcile algorithm (§8); fast path ≤5/≤20 ms gates; spawn via `MinijailProcessEffectPort` (opaque IDs; no broker DTO imported); adoption algorithm (§8.5) with `/proc` reads and cgroup enumeration via bounded blocking adapters; quarantine on ambiguity; quarantine reuse blocked until externally established process-absence proof or full Zone reset; no signal to quarantined/ambiguous identity; restart/backoff; finalize (§8.6); EphemeralProcess continuation recovery (§9); bootstrap authz scope (§3); post-bootstrap RBAC; metric label closed-set enforcement (no `zone` label); controller writes status only on Process/EphemeralProcess resources; Provider resource status aggregated by core; the controller declares no Provider state Volume and mounts none — its bounded non-secret operational state lives in `status`/the core Operation ledger (§5.1, D087) and running units are re-adopted from cgroup leaves + fresh pidfds on restart |
| Integration | Zone runtime startup (bootstrap); all v3 ResourceClient/bus/session paths |
| Data migration | Full reset; current DAG/role snapshot import not required |
| Validation | `tests/lifecycle.rs`; `tests/ephemeral_lifecycle.rs`; `tests/conformance.rs`; `tests/adoption_quarantine.rs`; `tests/bootstrap_authz.rs`; `tests/fast_path.rs`; `tests/blocking_adapter.rs`; `integration/adoption_restart/`; `integration/quarantine_scenario/`; `integration/latency_gate/`; shared conformance suite in `d2b-process-conformance` |
| Removal proof | Current `d2bd` DAG executor and direct spawn paths removed only after all ProcessRoles in the role-disposition table (ADR-046-components-processes-and-sandbox, §Representative baseline mapping) reach parity under system-minijail or system-systemd |

### ADR046-minijail-006 (Dependency: ADR046-minijail-005)

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-minijail-005; Nix integrator; test infrastructure owner |
| Current source | `nixos-modules/processes-json.nix`; `nixos-modules/minijail-profiles.nix`; `packages/d2b-contract-tests/tests/policy_observability.rs` |
| Reuse action | ADAPT |
| Destination | `nixos-modules/` — v3 Nix `Process`/`EphemeralProcess` resource authoring; Provider catalog entry; `docs/reference/schemas/v3/Process.json`; `docs/reference/schemas/v3/EphemeralProcess.json`; `make test-drift` schema drift gate |
| Detailed design | Nix module accepts `d2b.zones.<zone>.resources.<name>` with `type = "Process"` or `"EphemeralProcess"`; eval-time validation rules (§16.4); build-time JSON validation (§16.5); artifact catalog integration; cleanup contract tests (§16.5) |
| Integration | `d2b.artifacts` catalog; Zone bundle emission; `make test-drift` |
| Data migration | Current `nixos-modules/processes-json.nix` and minijail profile Nix removed at cutover |
| Validation | `nix-unit` eval cases for every validation rule; schema drift gate; `tests/schema.rs` |
| Removal proof | `processes-json.nix`, `minijail-profiles.nix`, and `programs-json.nix` removed after v3 Nix parity |

---

## 21. Removal proof

No current production path is removed until the exact Process Provider successor
is integrated and tested.

| Current path | Removed when |
| --- | --- |
| `d2bd` DAG executor direct minijail spawn paths | After ADR046-minijail-005 full lifecycle test parity (all ProcessRoles in disposition table) |
| `d2b-core/src/minijail_profile.rs` module | After ADR046-minijail-001 SandboxSpec compilation covers all current `MinijailProfile` callers |
| `d2b-core/src/process_builder.rs` | After ADR046-minijail-002 LaunchTicket builder replaces all current callers |
| `d2bd/src/supervisor/pidfd_table.rs` | After ADR046-minijail-004 wait/pidfd replaces all current callers |
| `nixos-modules/processes-json.nix` | After ADR046-minijail-006 Nix Process resource authoring replaces all ProcessRole Nix emissions |
| `nixos-modules/minijail-profiles.nix` (virtiofsdProfiles) | After virtiofsd Process resources (Provider/volume-virtiofs) are fully validated under system-minijail |
| `d2b-realm-router` session implementation types | After ComponentSession (ADR046-session-001) replaces all Realm PeerSession routes |
| `d2bd/src/realm_stubs.rs` dead scaffolding | After bus/ComponentSession integration lands |
| `d2b-realm-core` WorkloadProviderKind/IsolationPosture public enums | After all consumers migrate to `Host`/`Guest`/`ExecutionPolicy` ResourceTypes at cutover |

Each removal requires a separate work item or disposition commit that
demonstrates test parity before deletion. No removal may occur as part of the
same commit as a new feature unless the feature directly replaces the removed
symbol with verified test coverage.

---

## 22. Security invariants

The following invariants must hold at all times. Violation of any invariant
is a `runtime-security-violation` audit event and triggers quarantine or
process termination.

1. **Zero host capabilities for user-namespace processes.** Any process with
   `sandbox.userNamespace` set holds zero capabilities in the host capability
   set. In-namespace capabilities are namespace-scoped and do not grant host
   privilege. This preserves the ADR 0021 model for virtiofsd-class and
   comparable processes.

2. **No PID reuse in pidfd window.** The pidfd is obtained atomically from
   `clone3(CLONE_PIDFD)`. No window exists between clone and pidfd acquisition
   during which a PID could be reused.

3. **Cgroup-before-exec.** With `CLONE_INTO_CGROUP`, the process is placed in
   its cgroup leaf before any instruction executes. No window exists for the
   process to escape into an ancestor cgroup.

4. **No broad kill on quarantine; externally established proof required.**
   An ambiguous adoption or ambiguous finalize never issues any signal to the
   candidate process. Quarantine cannot be resolved by deleting/recreating the
   resource while the process may live. Cgroup leaf reuse and finalizer
   re-registration require externally established proof of process absence (OS
   inspection confirming the cgroup leaf is empty) or a destructive full Zone
   reset. The operator performs all process-absence verification through means
   external to d2b.

5. **Bootstrap authorization is non-configurable and contains no create verbs.**
   No operator config field widens the bootstrap authorization scope. The
   bootstrap grants cover only `get/list/watch/update-status/update-finalizers`
   on resources where `providerRef=Provider/system-minijail` — `create` verbs on
   any ResourceType are excluded. The `Provider/system-minijail` resource itself
   is runtime-created by the core-controller (`managedBy: controller`), not by
   system-minijail. A wrong subject, purpose, method, or Provider generation
   fails the bootstrap connection closed.

6. **Sandbox plan digest binding.** The compiled sandbox plan digest is bound
   into the LaunchTicket and re-verified by the broker at exec time. Any
   change between ticket issue and exec fails the spawn.

7. **UID/GID map write — effect port resolves principal; Provider never
   sees numeric IDs.** `userNamespace.mappingClass: process-principal-root`
   is the only public SandboxSpec field for user namespace identity. The core
   effect port resolves the exact host UID/GID for the declared Process
   component principal and enforces non-zero (non-root). It validates the
   parent name-to-inode binding for `/proc/<pid>/uid_map` and
   `/proc/<pid>/gid_map` writes with `O_NOFOLLOW` and re-verification before
   writing. No symlink or replacement can intercept the write. The Provider
   crate never holds or observes numeric host UID/GID values.

8. **No credential bytes in resource fields.** No credential byte, raw token,
   or secret appears in any Process/EphemeralProcess spec, status, audit
   record, log line, or metric label.

9. **Redaction before any external surface.** PID, pidfd number, cgroup path,
   argv, capability bitmask, mount source path, environment variable, and
   socket address are redacted from all Debug formatting, log events, audit
   records, and metric labels before reaching any external I/O path.

10. **Bootstrap provider cannot widen its own authorization.** system-minijail
    cannot grant itself additional RBAC verbs by creating Role or RoleBinding
    resources. Only the core-controller handles Role/RoleBinding creation, and
    it validates that no subject grants itself escalating verbs.

11. **Provider crate carries no broker service, client, or DTO.** The minijail
    controller crate imports no `d2b.broker.v3` service, client type, or broker
    DTO. All spawn effects flow exclusively through the injected
    `MinijailProcessEffectPort` with opaque identifiers. A compile-time
    dependency audit enforces this boundary; the effect port implementation
    remains owned by core/ProviderSupervisor and is the sole path to privileged
    broker operations.

12. **Volume ownership and reconciliation: core ProviderDeployment creates;
    volume-local reconciles; minijail controller only consumes.** Core
    ProviderDeployment creates the component state Volume (`kind: state`,
    `persistenceClass: persistent`, `migrationPolicy: none`, minimal nonzero
    quota) before the component Process starts and deletes it after the
    component Process is gone. `Provider/volume-local` is the sole Volume
    reconciler. The minijail controller does not create, own, add Volume to its
    exported ResourceTypes, or reconcile Volume resources; it only consumes the
    required `dirfd` view delivered by core. `ProviderStateSet(zone,
    "system-minijail")` is a query-time set — not a ResourceType. Each
    component Volume is `kind: state`, `persistenceClass: persistent`,
    `migrationPolicy: none`; never ephemeral, never zero-quota; identity marker
    always provisioned; no migration EphemeralProcess or worker ever created.
    Layout principals are Nix-preprovisioned `User/<name>` (no
    `ComponentPrincipal` ResourceRefs); no Volume is shared across components.
    Live pidfds and FDs are process-local and non-persistent; they must not be
    serialized, stored, or re-used across controller restarts without full
    re-verification.
