# ADR 0046 resources: Host, Guest, Process, EphemeralProcess, and User

| Field | Value |
| --- | --- |
| Spec ID | `ADR-046-resources-host-guest-process-user` |
| Parent | ADR 0046 |
| Status | Accepted |
| Version | 1 |
| Baseline | `b5ddbed67867d9244bf33390868101bd9b053e49` |
| Normative | Yes |
| Owners | `d2b-contracts`, `d2b-provider-system-core`, `d2b-provider-system-systemd`, `d2b-provider-system-minijail` |
| Depends on | `ADR-046-decision-register`, `ADR-046-terminology-and-identities`, `ADR-046-resource-object-model`, `ADR-046-resource-api-and-authorization`, `ADR-046-resource-reconciliation`, `ADR-046-components-processes-and-sandbox`, `ADR-046-provider-model-and-packaging`, `ADR-046-primitive-resource-composition`, `ADR-046-core-controllers` |
| Supersedes | Current `ProcessRole`/`VmProcessDag` as the public process model; current Realm workload/unsafe-local/execution DTOs; `WorkloadProviderKind`/`IsolationPosture` posture enums |

## Scope

This spec exhaustively defines:

- `Host` - physical/local host execution, policy, and budget parent;
- `Guest` - VM, sandbox, cloud, or remote execution parent;
- shared `ExecutionPolicy` inline schema used by both;
- `Process` - long-lived supervised process;
- `EphemeralProcess` - one-shot asynchronous process with terminal retention;
- shared `ExecutionSpec` inline schema used by both Process types;
- `User` - named host identity, UID/session observation, ACL/process subject.

Every field, type, default, bound, condition, error, security rule, RBAC verb,
reconcile step, finalizer step, Nix declaration example, and implementation
work item for these five ResourceTypes is defined here. ResourceTypes not in
scope (Volume, Network, Device, Credential, Zone, ZoneLink, Provider, Role,
RoleBinding) are referenced as target types and defined in their owning specs.

## ResourceSpec shape

### Three-layer spec shape (D089)

D089 freezes every ResourceSpec in this file as three layers. Layer 1 is the
universal Resource envelope and metadata. Layer 2 is the ResourceType base spec
at top-level `spec.*`, including `spec.providerRef`; all Host, Guest, Process,
EphemeralProcess, and User fields documented here are base unless explicitly
stated otherwise. Layer 3 is the optional canonical selected-Provider extension
`spec.provider = { schemaId, schemaVersion, settings }`; it is the only
Provider-specific desired extension and replaces the former Host/Guest
`providerSettings` shape. It omits `providerRef` and
`observedProviderGeneration`: `spec.providerRef` is base, and spec is desired
rather than observed.

**D091 update policy.** The universal base spec carries `spec.updatePolicy` for
every Host, Guest, Process, EphemeralProcess, and User resource: disruptive
changes default to manual, while automatic non-disruptive upgrades are permitted
by policy. A `spec.provider` extension MAY add provider-specific knobs, but MUST
NOT bypass or weaken base `spec.updatePolicy`.

**D090 expedited reconcile.** Authorized `Create`, `UpdateSpec`, and `Delete`
calls MAY set `waitForReconcile`. Under one mutation ticket, `operationId`, and
deadline, Core admission and the reserved-revision redb commit run in parallel
with the owning controller's preflight/plan, but the controller MUST NOT perform
external effects, finalizer release, or status mutation until Core supplies
`CommittedRevisionProof {resourceUid, generation, revision, operationId}`; DB
failure aborts with no effect. The API returns the committed object plus
one-pass projected layered status, `disposition`
(`Converged|Progressing|Blocked|UpgradeRequired|Failed`), `statusPersistence`
(`pending|committed`), and the last persisted status revision. The durable
commit is never rolled back on reconcile timeout or failure; effect idempotency
keys derive from `(UID,generation,revision,operationId)`, and the expedited pass
uses a bounded priority lane in the same per-resource single-flight.

Every Provider `ResourceApiBinding` MUST implement the exact base spec schema
version and fingerprint, accept the canonical minimal valid base Spec, and pass
base lifecycle/status/error/finalizer conformance. A Provider MAY reject an
optional base capability only through its signed standard capability matrix and
a typed provider-neutral `unsupported-capability` error; it MUST NOT ignore,
reinterpret, rename, duplicate, weaken, or require extension data for
base-required behavior. `spec.provider.settings` is strict deny-unknown,
bounded, schema-versioned and digested, validated against `spec.providerRef` at
Nix build and API admission, and fails with `spec-provider-schema-invalid` or
`spec-provider-shadow` when invalid or shadowing/restating/overriding/renaming/
duplicating a base field. Shared semantics are promoted to the ResourceType base
and never live in `spec.provider`; generic CLI/controllers operate on base spec
plus base status. For the same Provider, the `spec.provider` and
`status.provider` schemas align.

In Host, the no-isolation `isolationPosture` semantic is a promoted Host base
field at top-level `spec.isolationPosture`; it is never carried in the
`spec.provider` extension.

## Authority and cardinality (D097)

Host, Guest, Process, EphemeralProcess, and User carry D097 authority semantics
(schema in
[`ADR-046-resource-object-model` §Authority and cardinality](ADR-046-resource-object-model.md)):

- **Host** is an `exactly-one` host-scoped substrate **allocator/effect
  authority** (`authorityScope: host`, `arbitration: exclusive`, reconciled by
  `Provider/system-core`); it is the single owner of host allocation/effect for
  its node and is `exportability: forbidden`.
- **Guest** is `exactly-one` per Guest; **Process**/**EphemeralProcess** are
  `bounded-many` partitioned workloads placed under a Host/Guest authority.
- **User** is a per-user (`authorityScope: user`, `exactly-one` per user)
  authority; genuinely per-seat services (a Wayland portal, clipboard, or
  notification sink) are `authorityScope: seat|user` authorities owned by the
  relevant interaction Provider, not by this base type.
- Per-user/session Provider services (PipeWire mediator, Secret Service/keyring,
  systemd user manager, and a **shell supervisor per `ShellSession`** - never a
  global one) declare their qualified `AuthorityDescriptor` in the owning
  Provider dossier; this base spec only declares the requested share mode and
  cannot bypass the descriptor.

Core's authority index rejects a second authority for the same
`(Zone/scope, authorityClass, opaqueKeyDigest)` with `duplicateConflict` before
any effect; restart adopts by `ownerProof` and ambiguity quarantines.

### Fixed user-session authority (D097 desktop/session)

Display portal, the audio mediator, the notification sink, clipboard, Secret
Service, and the shell all depend on a **fixed user-session authority** that
pre-opens the compositor, PipeWire, and session-bus FDs for a login session.
This authority is **named here, not left as ambient prose**:

| Field | Value |
| --- | --- |
| `authorityScope` | `seat` (bound to a `Host` × `User` × login-session) |
| `authorityKey` | opaque class over `(Host, User, login-session/seat)` - never a raw socket path, XDG_RUNTIME_DIR, DISPLAY, or seat name |
| `cardinality` | `exactly-one` per `(Host, User, login-session/seat)` |
| `arbitration` | `exclusive` (the sole opener of the compositor/PipeWire/session-bus FDs) |
| Owner | a **core/user-agent session authority** (a per-user-session agent Process owned by `Provider/system-systemd` under the user's manager) - **not a new Provider** |
| Adoption | restart adopts by `ownerProof` (the agent Process identity + login-session id); ambiguity quarantines |
| Duplicate | a second session authority for the same `(Host, User, login-session)` is rejected with `duplicateConflict` before any FD open |
| `exportability` | `forbidden` (the session FDs never cross a Zone; desktop Providers receive them only via the D077 EffectPort/LaunchTicket) |

Every desktop/session Provider service binds to this single authority and
receives its compositor/PipeWire/session-bus FD through the EffectPort/
LaunchTicket; none opens the session FDs itself.

**Desktop/session authority classification** (per-Provider detail stays in each
dossier, refined by evidence; owner/cardinality named here):

| Class | Scope | Cardinality | Owner | Exportability |
| --- | --- | --- | --- | --- |
| Display controller + portal/login | zone | `exactly-one` per Zone | display Provider controller | forbidden |
| Compositor/session FD authority | seat | `exactly-one` per Host×User×session | core/user-agent session authority | forbidden |
| Clipboard host (`clipd-host`) | user | `exactly-one` per `User`; picker global-or-seat arbitration | clipboard Provider | policy-gated (default-denied) |
| Notification sink | user-session | `exactly-one` per User session | notification Provider | policy-gated (default-denied) |
| Audio authority (mediator) | seat/user | `exactly-one` per compositor user; **one `audio.d2bus.org.AudioBinding` per Guest** | audio Provider | explicit-export through the owner `audio.d2bus.org.AudioService` only (D096/D098) |
| systemd user manager | user | `exactly-one` per `User` × (`Host` or `Guest`) | `Provider/system-systemd` | forbidden |
| Entrablau login authority | guest | `exactly-one` per identity Guest/tenant | credential-entra (in the identity Guest) | forbidden (D093) |
| Secret Service / keyring | user-session | `exactly-one` per User session | credential-secret-service | forbidden |
| ShellPool / shell supervisor | shell-session | `exactly-one` per `ShellSession` (never global) | shell Provider | forbidden |
| Host input (`wl_seat`/pointer constraints) | seat | `at-most-one` per seat under the session authority | core/user-agent session authority | forbidden |

**Host input boundary.** The `wl_seat`/pointer-constraint surface is an explicit
`seat`-scoped authority owned by the fixed user-session authority (`at-most-one`
per seat); until an interaction Provider implements pointer-constraint/relative
input enforcement, that enforcement is a **declared unsupported boundary**
(input is not silently multiplexed - a second seat-input claimant is a
`duplicateConflict`).

**Admission conflict.** A second same-user Provider service or resource for any
`exactly-one` desktop authority (a second display portal, clipboard host,
notification sink, audio mediator, systemd user manager, Secret Service, or
session FD authority for the same `(Host, User, session)`) is rejected with
`duplicateConflict` naming the incumbent owner digest before any effect;
config activation goes `Degraded`. Multi-user/multi-seat is supported only up to
the **declared per-Host limit** (one authority per distinct `(Host, User, seat)`
tuple); anything beyond the declared limit is rejected.

**Guest-stop invalidation.** Stopping a Guest invalidates every user-session
authority and lease bound to that Guest across display, audio, notification,
credential, and shell in one dependency-aware cascade (D091): the session FD
authority and each dependent desktop authority are drained/recycled, their
`Endpoint`s revoked, and any `AudioBinding`/projection-Service/lease degraded - no stale
compositor/PipeWire/session-bus FD survives a Guest stop.

**Cross-Zone exportability (reconciled with D096).** Any prior claim of "no
cross-Zone sharing path" for desktop classes is superseded: D096
`ResourceExport`/`ResourceImport` are the sole typed bridge. Per class:
**display, Host input, and Secret Service are non-exportable** (`forbidden`) -
they never cross a Zone; **audio is a policy-gated explicit export** (already
supported, D096); **clipboard and notifications are structurally supportable
exports but default-denied** (policy-gated, opt-in only). Credentials/secrets
remain non-exportable by default (D093). This is an explicit per-class decision,
not a blanket omission.

## Shared field schemas

### ResourceName constraints

All resource names in this spec match `^[a-z][a-z0-9-]*$`. Length bounds:

| Field | Minimum | Maximum |
| --- | --- | --- |
| `metadata.name` | 1 | 63 |
| `spec.displayName` (User) | 1 | 128 |

### BudgetSpec

Inline schema embedded in Host, Guest, Process, and EphemeralProcess. Fields
are all optional; omitting a field means no d2b-level enforcement for that
resource. Zone capacity policy may still enforce totals.

```yaml
budget:
  cpu:
    request: "500m"       # millicpus; "0m" = no reservation; max "1024000m"
    limit: "2000m"        # millicpus; null = no limit; max "1024000m"
  memory:
    request: "128Mi"      # bytes with SI/IEC suffix; max "4Ti"
    limit: "512Mi"        # bytes with SI/IEC suffix; null = no limit
  pids:
    limit: 512            # integer [1, 65535]; null = no limit
  fds:
    limit: 1024           # integer [1, 1048576]; null = no limit
  ioWeight: 100           # cgroup blkio relative weight [1, 10000]
  networkEgressBps: null  # bytes/s; null = no limit; max 10^12
  threadLimit: null       # RLIMIT_NPROC equivalent; null = no limit
```

For Host and Guest, each field is the aggregate total across all Processes
placed on that execution target. For Process and EphemeralProcess, each field
applies to that single process's cgroup leaf. Process budget fields must not
exceed the Host or Guest total budget. Core rejects overcommit at spec
admission time using the aggregate reservation already committed in the store.

### ExecutionPolicy

Shared inline schema embedded in Host and Guest spec. All fields are optional
except `providerRef`.

| Field | Type | Default | Bound | Description |
| --- | --- | --- | --- | --- |
| `providerRef` | ResourceRef | required | `Provider/<name>` | The Provider owning this Host or Guest. Host must use `Provider/system-core`. Guest must use one of the four runtime Providers. |
| `defaultDomain` | `system\|user` | `system` | exact enum | Default Process domain when Process.domain is omitted. |
| `allowedDomains` | `[system\|user]` | `[system]` | 1..2 items, unique | Domains permitted for Processes under this Host or Guest. A Process with domain not in this list is rejected at spec admission. |
| `defaultUserRef` | ResourceRef? | `null` | `User/<name>` in same Zone | Used when Process.domain=user and Process.userRef is absent. Required when `allowedDomains` contains `user`. |
| `budget` | BudgetSpec | `{}` | see BudgetSpec | Aggregate budget for all Processes under this execution target. |
| `networkAttachments` | NetworkAttachmentList | `[]` | 0..64 items | Ordered list of Zone Networks that Processes under this Host or Guest may attach to. Each entry: `{networkRef: Network/<name>, default: bool}`. At most one entry may set `default: true`. |
| `deviceAttachments` | DeviceAttachmentList | `[]` | 0..64 items | Device refs available for Process device usage. Each entry: `{deviceRef: Device/<name>, exclusive: bool}`. |
| `volumeAttachmentDefaults` | VolumeAttachmentDefaultList | `[]` | 0..64 items | Default Volume attachment settings propagated to Processes that reference listed volumes. |
| `provider` | object? | `null` | canonical `{schemaId,schemaVersion,settings}` | Optional selected-Provider extension envelope (D089). `settings` carries implementation-only desired settings validated against the selected Provider's exported Host or Guest schema (`<provider>.d2bus.org/<Type>/spec`); strict deny-unknown, bounded. MUST NOT shadow or restate a base field. |

NetworkAttachmentList entry:

| Field | Type | Default | Description |
| --- | --- | --- | --- |
| `networkRef` | ResourceRef | required | `Network/<name>` in same Zone |
| `default` | bool | `false` | If true, this Network is the default for Processes that declare a network usage without an explicit networkRef |

DeviceAttachmentList entry:

| Field | Type | Default | Description |
| --- | --- | --- | --- |
| `deviceRef` | ResourceRef | required | `Device/<name>` in same Zone |
| `exclusive` | bool | `false` | If true, only one Process may hold this device at a time |

### SandboxSpec

Inline schema in Process and EphemeralProcess spec. Compiled by the selected
Process Provider to its implementation-specific form. No raw capability
numbers, seccomp BPF programs, minijail argument strings, or systemd unit
property fragments are accepted in spec. The Provider translates semantic
classes to exact implementation.

| Field | Type | Default | Bound | Description |
| --- | --- | --- | --- | --- |
| `namespaceClasses` | `[NamespaceClass]` | `[]` | 0..8 items, unique | Namespace isolation requests. Empty means inherits all parent namespaces. |
| `capabilityClasses` | `[CapabilityClass]` | `[]` | 0..16 items, unique | Semantic capability grants. Empty means no capabilities beyond user-domain base set. |
| `seccompClass` | `strict\|permissive\|allow-all\|<provider-class>` | `strict` | max 64 chars | Seccomp policy class. `strict` = minimal allow-list for the process class. `permissive` = log-only. `allow-all` = no filter; requires explicit carve-out in the controlling Provider's descriptor. `<provider-class>` = named profile from the Provider's compiled seccomp catalog. |
| `noNewPrivileges` | bool | `true` | - | If true, sets PR_SET_NO_NEW_PRIVS before exec. Must be true when `startRoot` is false. |
| `startRoot` | bool | `false` | - | If true, process starts as the in-namespace root UID before privilege drop. Requires explicit Provider justification in its descriptor. Cannot be true for user-domain Processes. |
| `environmentClass` | `minimal\|safe-inherited\|provider-defined` | `minimal` | - | `minimal` = only the fixed approved environment set. `safe-inherited` = inherits the declared safe subset from the owning Provider's component. `provider-defined` = exact environment from Provider's component template. |
| `readOnlyRoot` | bool | `true` | - | If true, rootfs is mounted read-only. |
| `umask` | string? | `"0022"` | octal 3-4 digits | File-creation mask installed before exec. |
| `oomScoreAdj` | int | `0` | -1000..1000 | OOM score adjustment. |
| `userNamespace` | UserNamespaceSpec? | `null` | - | If set, the process's effect adapter pre-establishes a single-entry user namespace before exec. Required for virtiofsd-class processes per ADR 0021. |

`NamespaceClass` enumeration:

| Value | Linux namespace |
| --- | --- |
| `user` | CLONE_NEWUSER |
| `pid` | CLONE_NEWPID |
| `mount` | CLONE_NEWNS |
| `ipc` | CLONE_NEWIPC |
| `uts` | CLONE_NEWUTS |
| `network` | CLONE_NEWNET |
| `cgroup` | CLONE_NEWCGROUP |
| `time` | CLONE_NEWTIME |

`CapabilityClass` enumeration (bounded; Provider adds no unlisted value without
a descriptor update):

| Value | Linux capability(ies) |
| --- | --- |
| `network-bind` | CAP_NET_BIND_SERVICE |
| `network-raw` | CAP_NET_RAW |
| `network-admin` | CAP_NET_ADMIN |
| `sys-time` | CAP_SYS_TIME |
| `sys-ptrace` | CAP_SYS_PTRACE |
| `sys-admin` | CAP_SYS_ADMIN (requires explicit carve-out) |
| `dac-override` | CAP_DAC_OVERRIDE |
| `fowner` | CAP_FOWNER |
| `chown` | CAP_CHOWN |
| `setuid` | CAP_SETUID |
| `setgid` | CAP_SETGID |
| `audit-write` | CAP_AUDIT_WRITE |
| `kill` | CAP_KILL |

`UserNamespaceSpec`:

| Field | Type | Default | Description |
| --- | --- | --- | --- |
| `mappingClass` | `MappingClass` | required | Semantic UID/GID mapping class. No numeric host UID/GID appears in the public spec. |

`MappingClass` enumeration (bounded; frozen):

| Value | Semantics |
| --- | --- |
| `process-principal-root` | Maps in-namespace UID/GID 0 to the host UID/GID of the Process's resolved principal - a stable `User/<name>` resource identified by the Process's owning template/ownerRef. This is the ADR 0021 virtiofsd-class mapped-root pattern. |

Core resolves the exact host UID/GID from the named principal and writes
`uid_map`/`gid_map` only into the private LaunchTicket/effect-adapter state at
launch time; the numeric values never appear in the public ResourceSpec,
status, audit, or API surface. The `user` NamespaceClass (CLONE_NEWUSER) is
unaffected by this change.

### ExecutionSpec

Shared inline fields embedded in both Process and EphemeralProcess spec.
All fields listed are part of the spec unless noted status-only.

| Field | Type | Default | Bound | Description |
| --- | --- | --- | --- | --- |
| `providerRef` | ResourceRef | required | `Provider/system-systemd` or `Provider/system-minijail` | Selects the Process Provider implementation. Must be installed and Ready in this Zone. |
| `executionRef` | ResourceRef | required | `Host/<name>` or `Guest/<name>` in same Zone | Target Host or Guest. Process is placed and supervised on this execution target. |
| `domain` | `system\|user` | inherited | inherits from `executionRef` target's `defaultDomain` | Execution domain. Must be in `executionRef.allowedDomains`. Specifying `user` without `userRef` and without a `defaultUserRef` on the target is a spec validation error. |
| `userRef` | ResourceRef? | `null` | `User/<name>` in same Zone | Required when `domain=user` and the target has no `defaultUserRef`, or to override the target default. Must resolve to a Ready User. |
| `processClass` | `controller\|service\|worker` | required | - | Process classification. `controller` = owns ResourceType reconcile loop via d2b-bus. `service` = serves typed ComponentSession methods; no reconcile ownership. `worker` = narrow Process with no controller/bus/dependency/CLI authority. |
| `template` | string | required | `^[a-z][a-z0-9-]*$`; max 63 chars | Plain component/process template ID. Resolved at runtime by the Provider registered as controller for the semantic owner resource identified by `metadata.ownerRef`. When `metadata.ownerRef` is absent, resolved through `spec.providerRef`. The controller maps this ID to an exact executable and content digest. `metadata.ownerRef` may be any resource type (Provider, Volume, Network, Device, or other); there is no restriction to Provider/<name> only. |
| `configRef` | ResourceRef? | `null` | `Volume/<name>` in same Zone | Sealed config Volume mounted read-only at the Provider-declared config path. |
| `credentialRefs` | `[ResourceRef]` | `[]` | 0..16 items | Credential refs the process may obtain leases from. Each must be `Credential/<name>` in same Zone. |
| `mounts` | `[MountSpec]` | `[]` | 0..64 items | Volume mounts declared by this process. |
| `sandbox` | SandboxSpec | `{}` | see SandboxSpec | Semantic sandbox requirements compiled by the selected Provider. |
| `budget` | BudgetSpec | `{}` | see BudgetSpec | Per-process resource limits. |
| `networkUsage` | NetworkUsageSpec? | `null` | - | Network access specification. |
| `deviceUsage` | `[DeviceUsageSpec]` | `[]` | 0..16 items | Device access specifications. |
| `telemetry` | TelemetrySpec | `{}` | - | Telemetry/observability bindings. |

`ProcessSpec` carries **no** inline `endpoints` field (D092). A stable endpoint a
Process produces is a separate owned `Endpoint` resource (see § Endpoint
ResourceType) with `producerRef: Process/<name>`; Core creates these from the
owning component descriptor's Endpoint templates, or the owning controller
creates them, and consumers reference `Endpoint/<name>`. Per-connection or
high-churn carriage (named streams, `OwnedTransport` handles, inherited fds)
remains internal and is not an Endpoint.

`MountSpec`:

| Field | Type | Default | Bound | Description |
| --- | --- | --- | --- | --- |
| `volumeRef` | ResourceRef | required | `Volume/<name>` in same Zone | Volume to mount. |
| `view` | string | required | max 63 chars | Named view declared by the Volume spec. |
| `mountPath` | string | required | absolute path; max 255 chars | Target path inside the process sandbox. |
| `access` | `read-only\|read-write` | `read-only` | - | Access level for this mount. |
| `required` | bool | `true` | - | If true, process start fails if Volume is not Ready. |

`NetworkUsageSpec`:

| Field | Type | Default | Bound | Description |
| --- | --- | --- | --- | --- |
| `networkRef` | ResourceRef? | `null` | `Network/<name>` in same Zone | Named Network. Null uses the target's default network. |
| `ports` | `[PortSpec]` | `[]` | 0..256 items | Ports declared for inbound traffic. |
| `allowEgress` | bool | `false` | - | If true, the process may initiate outbound connections subject to Network policy. |

`PortSpec`:

| Field | Type | Default | Description |
| --- | --- | --- | --- |
| `port` | u16 | required | Port number [1, 65535]. |
| `protocol` | `tcp\|udp\|sctp` | `tcp` | Transport protocol. |
| `purpose` | string | `""` | Stable bounded service label. Max 63 chars. |

`DeviceUsageSpec`:

| Field | Type | Default | Description |
| --- | --- | --- | --- |
| `deviceRef` | ResourceRef | required | `Device/<name>` in same Zone. |
| `access` | `shared\|exclusive` | `shared` | Exclusive requires the device's `exclusive` flag on the owning Host/Guest. |
| `purpose` | string | `""` | Bounded usage purpose. Max 63 chars. |

`EndpointSpec` (folded): there is no inline `EndpointSpec` on `ProcessSpec`.
Stable endpoints are the `Endpoint` ResourceType below; a former inline endpoint
`{ name, transport, purpose, publicKey }` becomes an `Endpoint` resource with
`producerRef` set to the producing Process. Ports are declared in
`NetworkUsageSpec.ports`; telemetry bindings in `TelemetrySpec`.

`TelemetrySpec`:

| Field | Type | Default | Description |
| --- | --- | --- | --- |
| `metricsEnabled` | bool | `true` | Export process metrics via the Zone observability Provider. |
| `tracingEnabled` | bool | `true` | Export traces. |
| `logLevel` | `off\|error\|warn\|info\|debug` | `info` | Log level hint. |
| `sensitiveLabels` | bool | `false` | If true, this process's telemetry is treated as sensitive and may be redacted in transit. |

---

## Host

### Purpose

`Host` represents one physical or local execution, policy, and budget parent
in the Zone. `Provider/system-core` is the sole reconciler. A Zone may declare
several Hosts with different policies, domains, and budgets, all mapping to the
same underlying OS instance.

The v3 successor to unsafe-local is a user-only Host (D042):
`defaultDomain=user`, `allowedDomains=[user]`, an explicit `defaultUserRef`.
Its explicit no-isolation posture is recorded in Host status and shown in
operator UI.

### Spec schema

```yaml
apiVersion: resources.d2bus.org/v3
type: Host
metadata:
  name: host-system            # required; ^[a-z][a-z0-9-]*$; max 63
  zone: dev                    # required; Zone self-name
spec:
  providerRef: Provider/system-core   # required; fixed value for Host
  defaultDomain: system               # system|user; default system
  allowedDomains: [system, user]      # [system|user]; 1..2 items, unique
  defaultUserRef: User/alice          # User/<name>; required when user in allowedDomains
  budget: {}                          # BudgetSpec; aggregate for all Processes on this Host
  networkAttachments: []              # 0..64 NetworkAttachmentList entries
  deviceAttachments: []               # 0..64 DeviceAttachmentList entries
  volumeAttachmentDefaults: []        # 0..64 VolumeAttachmentDefaultList entries
  provider:
    schemaId: system-core.d2bus.org/host-spec
    schemaVersion: "1.0"
    settings: {}                      # system-core Host extension schema; bounded
```

Full field table:

| Field | Type | Required | Default | Bound | Description |
| --- | --- | --- | --- | --- | --- |
| `providerRef` | ResourceRef | yes | - | `Provider/system-core` exactly | Reconciled by Provider/system-core. Any other providerRef is a spec error. |
| `defaultDomain` | `system\|user` | no | `system` | - | Default domain for Processes targeting this Host. |
| `allowedDomains` | `[system\|user]` | no | `[system]` | 1..2, unique | Domains allowed. Processes with unlisted domain are rejected at admission. |
| `defaultUserRef` | ResourceRef? | conditional | `null` | `User/<name>` | Required when `user` is in `allowedDomains`. |
| `budget` | BudgetSpec | no | `{}` | see BudgetSpec | Aggregate budget for Processes placed on this Host. |
| `networkAttachments` | list | no | `[]` | 0..64 | Network refs available to Processes on this Host. |
| `deviceAttachments` | list | no | `[]` | 0..64 | Device refs available to Processes on this Host. |
| `volumeAttachmentDefaults` | list | no | `[]` | 0..64 | Volume attachment defaults propagated to child Processes. |
| `isolationPosture` | string? | no | `null` | `null\|"none"` | Promoted Host base field (not a provider extension). `"none"` marks a user-only no-isolation Host and requires `defaultDomain=user`, `allowedDomains=["user"]`, and `defaultUserRef` set (and that tuple conversely requires `"none"`; `null` used to evade the no-isolation warning is rejected). Reflected in status. System processes are denied at admission when set to `"none"`. |
| `provider` | object? | no | `null` | canonical `{schemaId,schemaVersion,settings}` | Optional `Provider/system-core` extension envelope (D089), schema `system-core.d2bus.org/Host/spec`; see `spec.provider.settings` below. Strict deny-unknown; MUST NOT shadow a base field. |

`spec.provider.settings` for `Provider/system-core` (schemaId `system-core.d2bus.org/Host/spec`, schemaVersion `1.0`):

| Field | Type | Default | Description |
| --- | --- | --- | --- |
| `kernelVersionMin` | string? | `null` | Minimum required kernel version string, e.g. `"6.1"`. Reconcile fails if running kernel is older. |
| `capabilities` | `[HostCapabilityClass]` | `[]` | Capabilities this Host is expected to expose. Reconcile reports status.capabilities against this claim. |

The no-isolation `isolationPosture` field is a promoted Host base field at
top-level `spec.isolationPosture` (see the base spec table above); it is never a
`spec.provider.settings` field.

`HostCapabilityClass` enumeration:

| Value | Meaning |
| --- | --- |
| `kvm` | KVM hypervisor support |
| `pidfd` | pidfd_open(2) and CLONE_PIDFD kernel support |
| `cgroup-v2` | cgroup v2 delegation support |
| `user-namespace` | unprivileged user namespaces |
| `virtiofs` | virtiofsd/virtio-fs support |
| `audio-pipewire` | host PipeWire session manager running |
| `wayland` | host Wayland compositor socket present |
| `gpu-render` | render node present at /dev/dri/renderD* |
| `gpu-drm` | DRM primary node present |
| `tpm2` | TPM 2.0 device present |
| `usbip` | USBIP kernel module loadable |

`Provider/system-minijail` has a mandatory platform floor independent of the
optional Host setting: Linux **5.14 or newer**, cgroup v2 delegation, and a
writable `cgroup.kill` file on a delegated test leaf. Linux 5.14 is required
because intentional teardown uses `cgroup.kill` rather than PID/PGID ownership.
When any Process selects system-minijail, Host reconciliation performs this
probe before placement; `kernelVersionMin = null` cannot waive it, while a
higher configured minimum still applies. An older kernel fails with
`kernel-too-old`; missing or unusable `cgroup.kill` fails with
`cgroup-kill-unavailable`. Both keep the Provider/placement not Ready and
launch zero processes.

### Status schema

#### Three-layer status shape (D088)

D088 freezes `Host` status as three layers. The universal `ResourceStatus`
base (Layer 1) lives at top-level `status` and owns `observedGeneration`,
`phase`, `conditions`, `lastReconciledAt`, `startedAt`, `completedAt`, and
bounded `outcome`. The `Host`-specific status fields documented in this
section constitute the ResourceType-common `status.resource` (Layer 2) object
and never restate the universal base. Optional implementation-only observation
belongs in `status.provider` (Layer 3) with exactly `providerRef`, qualified
immutable `schemaId`, semver `MAJOR.MINOR` `schemaVersion`, numeric
`observedProviderGeneration`, and strict, bounded, redacted,
unknown-field-denied `details`. Generic API, CLI, and controllers MUST consume
only the base-only projection (`status` base plus `status.resource`).
Controllers MUST write all present layers atomically in one status mutation with
one expected revision. D087/D088 mapping is: shared observations go to
`status.resource`; implementation-specific bounded non-secret observations go to
`status.provider.details`; secret, large, or private observations go to an
optional Volume. D088 bounds apply: total status <= 64 KiB, `status.resource` <=
32 KiB, `status.provider.details` <= 32 KiB, 32 conditions, 64-entry lists/maps,
and 4 KiB strings; violations use `status-oversize`,
`status-provider-schema-invalid`, or `status-provider-overlap`.

Mapping convention: within this spec a reference to `status.<field>` denotes the ResourceType-common `status.resource.<field>` unless `<field>` is a universal base field (`observedGeneration`, `phase`, `conditions`, `lastReconciledAt`, `startedAt`, `completedAt`, `outcome`).

**D091 update currency.** Every Host, Guest, Process, EphemeralProcess, and User
resource includes universal `status.update` with `state`
(`Current|UpdateAvailable|UpgradeRequired|Upgrading|Blocked|Unknown`), `reasons`
(`CoreGenerationChanged|ProviderGenerationChanged|ArtifactChanged|ImageOrSystemGenerationChanged|SpecChanged|DependencyChanged|SecurityPolicyChanged`),
bounded non-secret observed/target generation and digest IDs, `disruption`
(`None|Reload|Restart|Recycle|Replace`), `preserveState`, optional
`operationId`, `lastAssessedAt`, and bounded/truncated `owned:{count,refs}` and
`dependencies:{count,refs}`. ResourceType-specific currency refinements live in
`status.resource` and never in `status.provider`; Core aggregates self, owned,
and dependency currency for list/get. Controllers set `status.update` via
`assess_update` on core/provider/artifact/image-or-system/spec/dependency/
security-policy triggers and MUST report `UpgradeRequired` for disruptive
changes rather than applying them in place. For Host, Guest, and Process-family
resources, disruptive image/system generation, provider generation, or immutable
spec changes use disruption `Recycle`, `Restart`, or `Replace`; upgrades recycle
realization and owned ephemeral Processes/endpoints while preserving
durable/state Volumes and Guest identity where possible.

```yaml
status:
  observedGeneration: 1
  phase: Ready                        # Pending|Ready|Succeeded|Degraded|Failed|Deleted|Unknown; long-lived resources do not steadily use Succeeded; Deleted is a terminal event-only phase (row removed after emit)
  conditions: []
  lastReconciledAt: "2026-07-22T00:00:01.000Z"
  startedAt: "2026-07-22T00:00:00.000Z"
  completedAt: null
  outcome: null
  # Host-specific:
  capabilities: []                    # HostCapabilityClass list observed on this host
  kernelRelease: ""                   # observed uname -r output; bounded; redacted in audit
  osName: ""                          # bounded OS identifier; max 128 chars
  userManagerAvailable: false         # true if systemd user manager is reachable
  activePid: null                     # not exposed; see security rules
  availableProcessSlots: null         # null = unknown; integer count
  isolationPosture: null              # null for normal Hosts; "none" for user-only no-isolation Host
  activeProcessCount: 0               # count of non-terminal Processes targeting this Host
```

Host-specific status fields:

| Field | Type | Description |
| --- | --- | --- |
| `capabilities` | `[HostCapabilityClass]` | Capabilities actually observed and verified by system-core. May differ from spec claim. |
| `kernelRelease` | string | Observed kernel release string from uname(2). Bounded to 64 chars. Not included in audit payloads. |
| `osName` | string | Bounded OS identifier string from /etc/os-release NAME. Max 128 chars. |
| `userManagerAvailable` | bool | True when the configured `defaultUserRef`'s systemd user manager is reachable and healthy. |
| `isolationPosture` | string? | Reflects `spec.isolationPosture`. `"none"` when this is a user-only no-isolation Host. Shown in operator UI as explicit no-isolation warning. Null/absent for all other Hosts. |
| `activeProcessCount` | u32 | Count of non-terminal Process/EphemeralProcess resources targeting this Host. Informational only. |

No PID, pidfd, socket path, internal node name, or raw host diagnostic is
exposed in public status or audit.

### Conditions

| Condition type | Ready = True when | Ready = False when | reason codes |
| --- | --- | --- | --- |
| `HostAvailable` | Host OS is reachable and matches `spec.provider.settings` | Host OS unreachable or kernel version requirement unmet | `host-unreachable`, `kernel-too-old`, `cgroup-unavailable` |
| `CapabilitiesVerified` | All `spec.provider.settings.capabilities` are observed | One or more claimed capabilities absent | `capability-absent-<class>` |
| `UserManagerReady` | User domain allowed and user manager reachable | User manager not running or unreachable | `user-manager-unavailable`, `user-manager-unknown` |
| `PolicyValid` | spec fields pass admission invariants | allowedDomains/defaultUserRef inconsistency detected | `spec-invalid-domains`, `spec-missing-default-user-ref` |
| `BudgetAdmitted` | Aggregate budget within Zone capacity | Overcommit detected | `budget-overcommit` |

Phase is `Ready` only when `HostAvailable` and `CapabilitiesVerified` are
True. `UserManagerReady` may be Degraded without affecting Ready for system-
only Hosts. `Failed` requires persistent unrecoverable error.

### RBAC

| Verb | Required rule | Restriction |
| --- | --- | --- |
| `get` | `{resourceTypes:[Host], verbs:[get]}` | Returns complete spec+status |
| `list` | `{resourceTypes:[Host], verbs:[list]}` | Bounded; snapshot revision returned |
| `watch` | `{resourceTypes:[Host], verbs:[watch]}` | Credit-bounded stream |
| `create` | `{resourceTypes:[Host], verbs:[create]}` | Provider/system-core bootstrap only during initial config; thereafter via config publication |
| `update-spec` | `{resourceTypes:[Host], verbs:[update-spec]}` | Config publication controller only |
| `update-status` | `{resourceTypes:[Host], verbs:[update-status]}` | Provider/system-core controller only, with expected revision and observedGeneration |
| `update-metadata` | `{resourceTypes:[Host], verbs:[update-metadata]}` | Bounded labels/annotations; no ownerRef changes |
| `delete` | `{resourceTypes:[Host], verbs:[delete]}` | Blocked while any non-terminal Process or EphemeralProcess targets this Host |

Structural checks additionally require:
- `providerRef` must resolve to `Provider/system-core` and be Ready;
- `defaultUserRef` must resolve to a `User/<name>` in the same Zone if set;
- `allowedDomains=[user]` and no `defaultUserRef` is rejected at admission;
- `isolationPosture = "none"` with `system` in `allowedDomains` is rejected.
- `allowedDomains = ["user"]` + `defaultDomain = "user"` + `defaultUserRef` set with `isolationPosture = null` is rejected (bidirectional).

### Reconcile

Provider/system-core reconcile loop for Host:

1. Receive trigger: `spec-generation-changed`, `dependency-changed`, `startup-relist`, or `scheduled-observe`.
2. Read fresh spec snapshot.
3. Validate spec invariants (domains, defaultUserRef, budget, capabilities claim).
4. Probe local OS availability: `uname(2)`, cgroup v2 mount, user namespace
   check, requested HostCapabilityClass probes. If system-minijail is installed
   or selected by a targeting Process, also enforce Linux ≥5.14 and probe a
   delegated leaf's writable `cgroup.kill`; failure is a platform-gate error,
   not a feature downgrade.
5. If `allowedDomains` contains `user`: contact fixed user supervisor to verify `defaultUserRef`'s user manager availability.
6. Compute aggregate Budget reservation from all non-terminal Processes targeting this Host via `List(executionRef=Host/<name>)`.
7. Check aggregate budget against Zone capacity policy.
8. Write Host status (capabilities, kernelRelease, osName, userManagerAvailable, activeProcessCount, conditions) via UpdateStatus with expected revision.
9. If spec-generation-changed and budget reduced: emit reconcile hints for non-terminal Processes over limit.
10. Return `converged` or `pending` with bounded requeue for next scheduled-observe.

Reconcile must complete in <=5 s to avoid the controller health timeout. Kernel
probes use bounded timeouts. User manager check uses a bounded IPC timeout.

### Finalize

Host uses no finalizer from system-core. Deletion is blocked by structural
check when active Processes exist. Operators must delete all Processes targeting
the Host before deletion succeeds. Core emits `phase=Deleted` event and removes
the Host immediately after the structural check passes.

---

## Guest

### Purpose

`Guest` represents one VM, sandbox, cloud, or remote execution parent in the
Zone. One of exactly four runtime Providers reconciles a Guest. The Guest
owns its child Process and EphemeralProcess resources and bootstrap
sub-resources.

The four accepted runtime Providers (D043):

| Provider | Execution substrate |
| --- | --- |
| `Provider/runtime-cloud-hypervisor` | Local NixOS VM via Cloud Hypervisor |
| `Provider/runtime-qemu-media` | Local QEMU media/physical-media VM |
| `Provider/runtime-azure-container-apps` | Azure Container Apps sandbox |
| `Provider/runtime-azure-virtual-machine` | Full-host Azure VM |

### Spec schema

```yaml
apiVersion: resources.d2bus.org/v3
type: Guest
metadata:
  name: dev-vm
  zone: dev
spec:
  providerRef: Provider/runtime-cloud-hypervisor   # required; one of four runtime Providers
  defaultDomain: system
  allowedDomains: [system, user]
  defaultUserRef: User/alice
  systemArtifactId: null  # artifact ID for the NixOS system closure; see d2b.artifacts catalog
  budget: {}
  networkAttachments: []
  deviceAttachments: []
  volumeAttachmentDefaults: []
  provider:
    schemaId: runtime-cloud-hypervisor.d2bus.org/guest-spec
    schemaVersion: "1.0"
    settings: {}        # selected runtime Provider's Guest schema extension
```

Full field table:

| Field | Type | Required | Default | Bound | Description |
| --- | --- | --- | --- | --- | --- |
| `providerRef` | ResourceRef | yes | - | One of four runtime Provider refs | Owning runtime Provider. Must be installed and Ready. |
| `defaultDomain` | `system\|user` | no | `system` | - | Default Process domain. |
| `allowedDomains` | `[system\|user]` | no | `[system]` | 1..2, unique | Allowed domains. |
| `defaultUserRef` | ResourceRef? | conditional | `null` | `User/<name>` | Required when `user` is in `allowedDomains`. |
| `budget` | BudgetSpec | no | `{}` | see BudgetSpec | Aggregate budget for all Processes on this Guest. |
| `networkAttachments` | list | no | `[]` | 0..64 | Networks available to Processes on this Guest. |
| `deviceAttachments` | list | no | `[]` | 0..64 | Devices available to Processes on this Guest. |
| `volumeAttachmentDefaults` | list | no | `[]` | 0..64 | Volume attachment defaults. |
| `systemArtifactId` | string? | no | `null` | `^[a-z][a-z0-9-]*$`; max 63 chars | Artifact ID for the NixOS system closure (kernel + initrd + rootfs). Must exist in `d2b.artifacts` with `type="nixos-system"`. Used by local VM Providers (e.g. `runtime-cloud-hypervisor`). `null` for cloud/remote Providers that do not boot a Nix-built system. |
| `provider` | object? | no | `null` | canonical `{schemaId,schemaVersion,settings}` | Optional selected-Provider extension envelope (D089). `settings` carries Provider-specific boot/identity/runtime settings validated against the selected runtime Provider's exported Guest schema (`<runtime-provider>.d2bus.org/Guest/spec`). Strict deny-unknown; MUST NOT shadow a base field. |

`spec.provider.settings` is the primary extension point for Provider-specific
behavior. Each runtime Provider exports its Guest schema extension through its
`ResourceApiExport`. Common fields across all four providers (informative;
exact fields owned by each Provider's dossier):

| Provider (schemaId) | Typical `spec.provider.settings` fields |
| --- | --- |
| `runtime-cloud-hypervisor` (`runtime-cloud-hypervisor.d2bus.org/Guest/spec`) | `vcpus`, `memoryMb`, `cmdline`, `vsockCid`, `machineType`, `consoleType`, `serialPort`, `pvpanic` |
| `runtime-qemu-media` (`runtime-qemu-media.d2bus.org/Guest/spec`) | `mediaSourceRef`, `mediaFormat`, `vcpus`, `memoryMb`, `machineType`, `displayOutput` |
| `runtime-azure-container-apps` (`runtime-azure-container-apps.d2bus.org/Guest/spec`) | `containerGroup`, `environmentId`, `revision`, `minReplicas`, `maxReplicas` |
| `runtime-azure-virtual-machine` (`runtime-azure-virtual-machine.d2bus.org/Guest/spec`) | `vmSize`, `imageRef`, `diskSku`, `adminUser`, `publicKeyRef`, `region` |

This spec does not define the exact `spec.provider.settings` field list for each
runtime Provider. That is owned by each Provider's dossier spec. API binding
rejects unknown `spec.provider.settings` fields not declared by the
installed Provider's schema.

### Status schema

#### Three-layer status shape (D088)

D088 freezes `Guest` status as three layers. The universal `ResourceStatus`
base (Layer 1) lives at top-level `status` and owns `observedGeneration`,
`phase`, `conditions`, `lastReconciledAt`, `startedAt`, `completedAt`, and
bounded `outcome`. The `Guest`-specific status fields documented in this
section constitute the ResourceType-common `status.resource` (Layer 2) object
and never restate the universal base. Optional implementation-only observation
belongs in `status.provider` (Layer 3) with exactly `providerRef`, qualified
immutable `schemaId`, semver `MAJOR.MINOR` `schemaVersion`, numeric
`observedProviderGeneration`, and strict, bounded, redacted,
unknown-field-denied `details`. Generic API, CLI, and controllers MUST consume
only the base-only projection (`status` base plus `status.resource`).
Controllers MUST write all present layers atomically in one status mutation with
one expected revision. D087/D088 mapping is: shared observations go to
`status.resource`; implementation-specific bounded non-secret observations go to
`status.provider.details`; secret, large, or private observations go to an
optional Volume. D088 bounds apply: total status <= 64 KiB, `status.resource` <=
32 KiB, `status.provider.details` <= 32 KiB, 32 conditions, 64-entry lists/maps,
and 4 KiB strings; violations use `status-oversize`,
`status-provider-schema-invalid`, or `status-provider-overlap`.

Mapping convention: within this spec a reference to `status.<field>` denotes the ResourceType-common `status.resource.<field>` unless `<field>` is a universal base field (`observedGeneration`, `phase`, `conditions`, `lastReconciledAt`, `startedAt`, `completedAt`, `outcome`).

`Guest` has multiple runtime implementations (`runtime-cloud-hypervisor`,
`runtime-qemu-media`, `runtime-azure-container-apps`, and
`runtime-azure-virtual-machine`). Runtime readiness, capability, observed
lifecycle phase, bootstrap readiness, identity-digest, and active-process
observations are frozen in `status.resource` and MUST be identical across all
implementations. Implementation-specific observation belongs only in that
implementation's `status.provider.details`; shared fields MUST NOT be duplicated
there.

```yaml
status:
  observedGeneration: 1
  phase: Ready                        # Pending|Ready|Succeeded|Degraded|Failed|Deleted|Unknown; long-lived resources do not steadily use Succeeded; Deleted is a terminal event-only phase (row removed after emit)
  conditions: []
  lastReconciledAt: "2026-07-22T00:00:01.000Z"
  startedAt: "2026-07-22T00:00:00.000Z"
  completedAt: null
  outcome: null
  # Guest-specific:
  providerPhase: ""                   # bounded provider-specific lifecycle phase; max 64 chars
  bootstrapReady: false
  guestIdentityDigest: ""             # opaque bounded digest of verified guest identity; max 128 chars
  activeProcessCount: 0
```

Guest-specific status fields:

| Field | Type | Description |
| --- | --- | --- |
| `providerPhase` | string | Bounded provider-specific phase label, e.g. `"starting"`, `"running"`, `"stopped"`. Max 64 chars. Stable lower-kebab-case value. |
| `bootstrapReady` | bool | True when the Guest's bootstrap sub-resources (first VMM Process, guest-control session, etc.) have reached Ready/Succeeded phase. |
| `guestIdentityDigest` | string | Opaque bounded hex digest of the verified guest identity material (boot token, generation, provider-specific). Max 128 chars. Not the guest IP/hostname. |
| `activeProcessCount` | u32 | Count of non-terminal Process/EphemeralProcess resources targeting this Guest. |

No CID, VSOCK address, SSH host, IP address, container name, Azure resource
path, or raw provider diagnostic is public status. Bounded `guestIdentityDigest`
and `providerPhase` are the only provider-observable identity fields.

### Conditions

| Condition type | Ready = True when | Ready = False when | reason codes |
| --- | --- | --- | --- |
| `GuestProvisioned` | Provider has provisioned the execution substrate | Substrate not provisioned or provider error | `provider-error`, `quota-exceeded`, `substrate-unavailable` |
| `BootstrapReady` | All required bootstrap sub-resources are Ready | One or more bootstrap resources Pending/Failed | `bootstrap-process-failed`, `bootstrap-process-pending` |
| `GuestReachable` | Guest passes the runtime Provider's authenticated health check | Health check failed or timed out | `health-check-failed`, `health-check-timeout` |
| `CapabilitiesVerified` | All declared device/network attachments reachable | Attachment missing or failed | `device-attachment-failed`, `network-attachment-failed` |
| `PolicyValid` | spec passes admission invariants | allowedDomains/defaultUserRef/`spec.provider.settings` error | `spec-invalid-domains`, `spec-provider-schema-invalid` |
| `BudgetAdmitted` | Aggregate budget within Zone capacity | Overcommit | `budget-overcommit` |

Phase is `Ready` only when `GuestProvisioned`, `BootstrapReady`, and
`GuestReachable` are all True.

### RBAC

| Verb | Required rule | Restriction |
| --- | --- | --- |
| `get` | `{resourceTypes:[Guest], verbs:[get]}` | Returns spec+status |
| `list` | `{resourceTypes:[Guest], verbs:[list]}` | Bounded snapshot |
| `watch` | `{resourceTypes:[Guest], verbs:[watch]}` | Credit-bounded |
| `create` | `{resourceTypes:[Guest], verbs:[create]}` | Config publication controller; runtime Provider bootstrap |
| `update-spec` | `{resourceTypes:[Guest], verbs:[update-spec]}` | Config publication controller |
| `update-status` | `{resourceTypes:[Guest], verbs:[update-status]}` | Owning runtime Provider controller only |
| `delete` | `{resourceTypes:[Guest], verbs:[delete]}` | Blocked while non-terminal Processes target this Guest |

Structural checks additionally require:
- `providerRef` must resolve to one of the four accepted runtime Providers;
- the runtime Provider must be installed and Ready;
- `defaultUserRef` required when `allowedDomains` contains `user`.

### Reconcile

Each runtime Provider controller instance runs the reconcile loop for its
Guests. Concrete reconcile behavior is defined in each Provider's dossier. The
common contract:

1. Receive trigger.
2. Read fresh Guest spec snapshot plus owned children snapshot.
3. Validate spec invariants and `spec.provider.settings` against the Provider schema.
4. Assert or create the required owned child Process/EphemeralProcess bootstrap
   graph (VMM, virtiofsd, guest-control, etc.) using the owner-child mechanism.
5. Wait for bootstrap children to reach Ready.
6. Run the Provider's authenticated health check against the Guest.
7. Write Guest status (providerPhase, bootstrapReady, guestIdentityDigest,
   conditions) via UpdateStatus with expected revision.
8. Return `converged`, `pending`, or `failed-retryable` per Provider policy.

The reconcile loop must not block the controller-wide queue on guest
provisioning. Long-running effects use async tasks; status is written
asynchronously with expected-revision commits.

### Finalize

The runtime Provider controller owns one finalizer: `runtime.<provider-name>/guest`.

Finalizer algorithm on `deletion-requested`:

1. Mark Guest as draining; stop admitting new Processes.
2. Delete all owned child Processes and EphemeralProcesses child-first per the
   owner graph, respecting their finalizers.
3. Ask the runtime Provider to deprovision the execution substrate.
4. Verify deprovisioning completion with bounded timeout.
5. Clear the `runtime.<provider-name>/guest` finalizer.
6. Return `finalized` after all children are deleted and substrate is gone.

If deprovisioning is ambiguous (network outage, partial state), the finalizer
returns `blocked` with a typed condition until the operator resolves or a
bounded timeout expires, at which point it returns `failed-terminal` with an
explicit audit record.

---

## Process

### Purpose

`Process` is a long-lived supervised process. One installed Process Provider
(`Provider/system-systemd` or `Provider/system-minijail`) manages its full
lifecycle: start, readiness, health, restart, adoption, and stop. Every
Process has exactly one executionRef (Host or Guest) and one process domain
(system or user).

### Spec schema

```yaml
apiVersion: resources.d2bus.org/v3
type: Process
metadata:
  name: wayland-proxy
  zone: dev
  ownerRef: Provider/display-wayland   # owning Provider; template is its signed component ID
spec:
  # ExecutionSpec common fields:
  providerRef: Provider/system-systemd
  executionRef: Host/host-system      # or Guest/<name>
  domain: system                      # system|user; default from executionRef
  userRef: null                       # User/<name>; required when domain=user without defaultUserRef
  processClass: service               # controller|service|worker
  template: wayland-proxy-main        # plain ID within owning Provider's component descriptor
  configRef: null
  credentialRefs: []
  mounts: []
  sandbox: {}
  budget: {}
  networkUsage: null
  deviceUsage: []
  telemetry: {}
  # Process-specific fields:
  desiredLifecycle: running           # running|stopped
  restartPolicy:
    class: on-failure                 # never|always|on-failure|on-crash
    backoffBase: "1s"
    backoffMax: "60s"
    backoffMultiplierMilli: 2000
    maxRestarts: null                 # null = unlimited; integer [1, 65535]
    resetAfter: "300s"
  readiness:
    initialDelay: "0s"
    timeout: "30s"
    failureThreshold: 3
    successThreshold: 1
    class: ready-condition            # ready-condition|provider-defined
  healthCheck:
    enabled: false
    interval: "30s"
    timeout: "5s"
    failureThreshold: 3
    class: provider-defined
  adoptionPolicy: adopt-on-restart    # adopt-on-restart|never-adopt
  drainTimeout: "30s"
```

Process-specific fields:

| Field | Type | Required | Default | Bound | Description |
| --- | --- | --- | --- | --- | --- |
| `desiredLifecycle` | `running\|stopped` | no | `running` | - | Desired steady-state lifecycle. `stopped` means the controller will not start the process but retains the resource and all status/finalizers. |
| `restartPolicy` | RestartPolicySpec | no | see below | - | Restart/backoff behavior. |
| `readiness` | ReadinessSpec | no | see below | - | Readiness probe settings. |
| `healthCheck` | HealthCheckSpec | no | disabled | - | Health check settings. |
| `adoptionPolicy` | `adopt-on-restart\|never-adopt` | no | `adopt-on-restart` | - | Whether the controller attempts to adopt a running process after controller restart. |
| `drainTimeout` | duration string | no | `"30s"` | `"0s".."3600s"` | Time to wait after exact-main SIGTERM before an unambiguous system-minijail subtree is terminated through its cgroup v2 `cgroup.kill`; no PGID fallback. |

`RestartPolicySpec`:

| Field | Type | Default | Bound | Description |
| --- | --- | --- | --- | --- |
| `class` | `never\|always\|on-failure\|on-crash` | `on-failure` | - | `never` = no restart. `always` = restart on any exit. `on-failure` = restart on non-zero exit. `on-crash` = restart only on signal/crash (not clean non-zero). |
| `backoffBase` | duration string | `"1s"` | `"0s".."60s"` | Initial backoff after first restart. |
| `backoffMax` | duration string | `"60s"` | `"1s".."3600s"` | Maximum backoff cap. |
| `backoffMultiplierMilli` | integer | `2000` | `1000..10000` | Exponential backoff multiplier x 1000 (integer fixed-point; D101 forbids JSON floats). |
| `maxRestarts` | u32? | `null` | `1..65535\|null` | Null means unlimited. When exceeded, phase becomes `Failed`. |
| `resetAfter` | duration string | `"300s"` | `"0s".."86400s"` | If process stays Running for this duration, restart counter resets to 0. |

`ReadinessSpec`:

| Field | Type | Default | Bound | Description |
| --- | --- | --- | --- | --- |
| `initialDelay` | duration string | `"0s"` | `"0s".."300s"` | Delay before first readiness probe after start. |
| `timeout` | duration string | `"30s"` | `"1s".."300s"` | Total readiness wait timeout from process start. |
| `failureThreshold` | u32 | `3` | `1..100` | Consecutive failures before marking not-ready. |
| `successThreshold` | u32 | `1` | `1..100` | Consecutive successes to declare ready from not-ready. |
| `class` | `ready-condition\|provider-defined` | `ready-condition` | - | `ready-condition` = process is ready when Ready condition becomes True. `provider-defined` = Provider uses a named readiness mechanism from the process template. |

`HealthCheckSpec`:

| Field | Type | Default | Bound | Description |
| --- | --- | --- | --- | --- |
| `enabled` | bool | `false` | - | Whether ongoing health checks run after readiness. |
| `interval` | duration string | `"30s"` | `"1s".."3600s"` | Interval between health checks. |
| `timeout` | duration string | `"5s"` | `"1s".."60s"` | Single health check timeout. |
| `failureThreshold` | u32 | `3` | `1..100` | Consecutive failures before Degraded/Failed. |
| `class` | `provider-defined` | `provider-defined` | - | Health check mechanism defined by the Provider template. |

### Status schema

#### Three-layer status shape (D088)

D088 freezes `Process` status as three layers. The universal `ResourceStatus`
base (Layer 1) lives at top-level `status` and owns `observedGeneration`,
`phase`, `conditions`, `lastReconciledAt`, `startedAt`, `completedAt`, and
bounded `outcome`. The `Process`-specific status fields documented in this
section constitute the ResourceType-common `status.resource` (Layer 2) object
and never restate the universal base. Optional implementation-only observation
belongs in `status.provider` (Layer 3) with exactly `providerRef`, qualified
immutable `schemaId`, semver `MAJOR.MINOR` `schemaVersion`, numeric
`observedProviderGeneration`, and strict, bounded, redacted,
unknown-field-denied `details`. Generic API, CLI, and controllers MUST consume
only the base-only projection (`status` base plus `status.resource`).
Controllers MUST write all present layers atomically in one status mutation with
one expected revision. D087/D088 mapping is: shared observations go to
`status.resource`; implementation-specific bounded non-secret observations go to
`status.provider.details`; secret, large, or private observations go to an
optional Volume. D088 bounds apply: total status <= 64 KiB, `status.resource` <=
32 KiB, `status.provider.details` <= 32 KiB, 32 conditions, 64-entry lists/maps,
and 4 KiB strings; violations use `status-oversize`,
`status-provider-schema-invalid`, or `status-provider-overlap`.

Mapping convention: within this spec a reference to `status.<field>` denotes the ResourceType-common `status.resource.<field>` unless `<field>` is a universal base field (`observedGeneration`, `phase`, `conditions`, `lastReconciledAt`, `startedAt`, `completedAt`, `outcome`).

```yaml
status:
  observedGeneration: 1
  phase: Ready                        # Pending|Ready|Succeeded|Degraded|Failed|Deleted|Unknown; long-lived resources do not steadily use Succeeded; Deleted is a terminal event-only phase (row removed after emit)
  conditions: []
  lastReconciledAt: "2026-07-22T00:00:01.000Z"
  startedAt: "2026-07-22T00:00:00.000Z"
  completedAt: null
  outcome: null
  # Process-specific:
  providerImplementation: ""          # bounded process Provider name; max 64 chars
  processIdentityDigest: ""           # opaque digest of verified process identity; max 128 chars
  waitReapOwner: ""                   # "d2b" or "systemd"; who owns wait/reap
  executionRef: ""                    # resolved executionRef at last reconcile
  domain: ""                          # system|user at last reconcile
  userRef: null                       # resolved userRef, if any
  configRevisionDigest: null          # digest of sealed config at last start; max 128 chars
  sandboxRevisionDigest: null         # digest of compiled sandbox at last start; max 128 chars
  lastExitClass: null                 # clean-exit|crash|signal|timeout|unknown
  restartCount: 0                     # total restart count since resource creation
  lastRestartAt: null                 # RFC 3339 UTC
  adoptionState: null                 # adopted|fresh|quarantined|adoption-failed
```

Process-specific status fields:

| Field | Type | Description |
| --- | --- | --- |
| `providerImplementation` | string | Stable Provider name that started this process (e.g., `system-systemd`). Max 64 chars. |
| `processIdentityDigest` | string | Opaque hex digest of the verified process identity material at last start (executable hash, template generation, cgroup path digest). Max 128 chars. Not the PID. |
| `waitReapOwner` | string | `"d2b"` for system-minijail means the privileged broker that called `clone3` and is the child parent; `"systemd"` for system-systemd. It never means the non-parent Provider controller. |
| `executionRef` | string | The resolved executionRef at last reconcile start. |
| `domain` | string | The resolved domain (`system` or `user`) at last reconcile. |
| `userRef` | string? | The resolved userRef used at last reconcile, if applicable. |
| `configRevisionDigest` | string? | Bounded hex digest of sealed config at last launch. |
| `sandboxRevisionDigest` | string? | Bounded hex digest of compiled sandbox at last launch. |
| `lastExitClass` | string? | Stable exit classification: `clean-exit`, `crash`, `signal`, `timeout`, `unknown`. |
| `restartCount` | u32 | Total restart count. |
| `lastRestartAt` | string? | RFC 3339 UTC timestamp of last restart. |
| `adoptionState` | string? | `adopted`: previously running process re-adopted after controller restart. `fresh`: new start. `quarantined`: ambiguous prior identity; process not adopted, not killed. `adoption-failed`: adoption attempted but failed; process stopped. |

No PID, pidfd file descriptor number, cgroup path, socket path, argv, raw
environment, systemd unit name, terminal bytes, or raw Provider diagnostic is
ever public Process status or audit payload.

### Conditions

| Condition type | Ready = True when | reason codes |
| --- | --- | --- |
| `Scheduled` | Process is assigned to a running controller instance | `controller-unavailable`, `no-available-controller` |
| `ProviderReady` | Process Provider (system-systemd/system-minijail) is installed and Ready | `provider-unavailable`, `provider-generation-mismatch` |
| `ExecutionReady` | executionRef target (Host or Guest) is in Ready phase | `execution-pending`, `execution-failed`, `execution-unknown` |
| `UserReady` | userRef (if required) resolves to a Ready User | `user-not-found`, `user-not-ready`, `user-manager-unavailable` |
| `DependenciesReady` | All required Volume/Network/Device dependencies are Ready | `volume-not-ready`, `network-not-ready`, `device-not-ready` |
| `Launching` | Process start attempt is underway | `launch-pending` |
| `Ready` | Process is running and has passed readiness check | `process-starting`, `process-crashed`, `process-timed-out`, `readiness-timeout` |
| `Healthy` | Most recent health check passed (if enabled) | `health-check-failed`, `health-check-timeout` |
| `Adopted` | Prior running process successfully adopted | `adoption-ambiguous`, `adoption-identity-mismatch`, `adoption-failed` |

### Pidfd rules

These rules are invariant for all Process Providers. Violation is a
`runtime-security-violation` audit event and the process is quarantined.

1. Every launched process has a local verified pidfd acquired by
   its lifecycle owner immediately after exec/launch; the Provider controller
   receives only an opaque identity/lease handle.
2. Pidfd is acquired only after ProviderSupervisor verifies the process's
   stable identity: executable hash, template generation, cgroup/scope
   placement, and provider-specific identity attributes. Any mismatch before
   pidfd open quarantines rather than adopts.
3. Pidfd is never serialized to disk, never written to the resource store,
   never sent over d2b-bus, and never exposed in public status or API. No
   Process Provider controller ever holds or imports a raw pidfd; it holds
   only the opaque handle ProviderSupervisor returns.
4. A ProviderSupervisor-held duplicate is closed and reacquired (with full
   re-verification) after every ProviderSupervisor restart. A Provider
   controller restart transfers no raw fd. For system-minijail, the still-live
   broker parent retains its original pidfd and wait/reap record until it reaps
   the child.
5. On adoption after restart, ProviderSupervisor locates the candidate
   process through the cgroup leaf, verifies all stable identity fields
   against the stored processIdentityDigest, and only then obtains a fresh
   verified pidfd handle and reports the outcome to the controller. For
   system-minijail, the original broker parent supplies a fresh duplicate; for
   system-systemd, ProviderSupervisor uses `pidfd_open(2)`.
   Ambiguous identity → `adoptionState: quarantined`.
6. For system-minijail: the broker calls
   `clone3(CLONE_PIDFD | CLONE_INTO_CGROUP)`, obtains the pidfd atomically, and
   remains the child's kernel parent. It alone calls `waitid(P_PIDFD, ...)`,
   collects exit status, and reaps. It may provide ProviderSupervisor a
   duplicate through a private local attachment for polling and signaling; the
   Provider controller never receives the raw fd.
7. For system-systemd: ProviderSupervisor reads the unit's MainPID after
   InvocationID + cgroup + start-time verification and calls `pidfd_open(2)`
   on that PID. systemd performs wait/reap.
8. A non-parent ProviderSupervisor may poll pidfd readability, but readability
   is only a terminal-liveness hint: it is not `waitid`, reap, or exit-status
   collection. For system-minijail, only the identity-bound terminal result
   relayed after the broker parent's successful wait/reap supplies
   `lastExitClass` or `outcome.exitCode`.
9. Any holder of a currently verified pidfd may use `pidfd_send_signal` for the
   exact main process; this syscall does not require parenthood. The Provider
   controller requests the effect through its opaque handle. The PID is never
   written to logs, status, audit, or metrics.
10. ProviderSupervisor maintains no PID/PGID ownership record and never sends
    descendant SIGKILL by process group. An unambiguous system-minijail subtree
    is terminated only by writing `1` to its anchored cgroup v2 `cgroup.kill`
    after the graceful exact-main signal/deadline.

### system-systemd conformance

A process launched under `Provider/system-systemd` must satisfy all of:

- Started as a non-forking transient systemd service or scope.
- Unit type must be `Type=exec` (or equivalent non-daemonizing `Type=simple`
  where `Type=exec` is not available). Daemonizing (`Type=forking`) is
  forbidden.
- ProviderSupervisor binds InvocationID, cgroup path, MainPID, and
  ExecMainStartTimestamp atomically from the unit's active state before
  opening pidfd, on behalf of the `system-systemd` Provider controller. The
  Provider controller itself never opens a systemd D-Bus connection or a
  pidfd directly.
- pidfd is opened from `MainPID` only after all four binding checks pass.
- systemd performs wait/reap. ProviderSupervisor does NOT call `waitpid`. It
  monitors process exit via the unit's active state transitions and reports
  transitions to the controller.
- No per-Provider static PID1 template unit may be used. All units are
  transient; they exist only while the Process resource is non-terminal.
- For user domain: a transient user scope is used via the fixed user supervisor.
  The exact authenticated `userRef` UID is verified by the user supervisor
  before creating the scope.
- Unit name alone is never treated as identity. Identity requires InvocationID +
  cgroup + MainPID + start-time tuple.
- On adoption after restart: ProviderSupervisor rediscovers the live unit by
  cgroup path, re-checks InvocationID, cgroup, MainPID, start-time against
  stored processIdentityDigest. Mismatch → quarantine.

### system-minijail conformance

A process launched under `Provider/system-minijail` must satisfy all of:

- The `system-minijail` Provider controller never imports or calls the broker
  itself. It resolves a `ProcessLaunchEffectPort.spawn` call with the opaque
  LaunchTicket; ProviderSupervisor dispatches that call to the privileged
  broker via `clone3(CLONE_PIDFD | CLONE_INTO_CGROUP)`, placing the process
  directly into its declared cgroup leaf.
- The process is born in its declared cgroup before any instruction executes.
- The broker that called `clone3` remains the process's parent and alone
  performs `waitid(P_PIDFD)`, exit-status collection, and reap via its retained
  pidfd. ProviderSupervisor may poll a verified duplicate for readability and
  use it for exact-main `pidfd_send_signal`, but polling is not wait/reap and
  cannot produce exit status. ProviderSupervisor relays only the broker's typed
  terminal result and adoption state back to the controller.
- The SandboxSpec's `namespaceClasses`, `capabilityClasses`, `seccompClass`,
  and `userNamespace` are compiled by the broker from the trusted bundle
  into a minijail/seccomp/namespace plan. The compiled plan digest is stored
  in `sandboxRevisionDigest`.
- The broker verifies: executable path/hash, template generation, declared UID/GID,
  compiled sandbox plan digest, and cgroup placement before exec.
- No environment variable, mount, or path from the caller resource payload
  reaches exec without passing through the trusted bundle compilation step.
- For user namespace processes (virtiofsd class): the broker pre-establishes
  the user namespace with `clone3(CLONE_NEWUSER)` + pipe sync + uid_map/gid_map
  writes before the process's first instruction. `UserNamespaceSpec.mappingClass:
  process-principal-root` resolves, only inside this private effect-adapter
  state, to the host UID/GID mapped to in-namespace UID/GID 0.
- On adoption after restart: ProviderSupervisor locates the process by cgroup
  leaf and verifies cgroup/PID/start-time/executable identity against
  processIdentityDigest, verifies the original broker-parent record, and
  obtains a fresh duplicate from that broker. Ambiguity or lost parent
  ownership → quarantine, never broad kill.
- Intentional stop/finalize sends SIGTERM to the exact main process, waits the
  bounded grace, then has the broker write `1` to the anchored leaf's
  `cgroup.kill`; it waits for broker-reaped main status and
  `cgroup.events: populated 0` before rmdir. There is no PID/PGID SIGKILL
  fallback.
- Linux ≥5.14 plus writable delegated-leaf `cgroup.kill` is a mandatory
  placement gate, not an optional capability downgrade.

### Fast path contract (D030)

After a Process resource is durably committed with all dependencies Ready:

- The store post-commit dispatcher emits a matching controller hint immediately
  after durable commit returns.
- p95 from durable commit to controller handler start: <=5 ms.
- p95 from durable commit to launch attempt start: <=20 ms.
- The controller launches the process in an independent async task without
  blocking the watch loop.
- The watch loop dispatches the next independent Process immediately.
- Status transitions (Queued → Launching → Ready) are async expected-revision
  writes; they do not hold the watch loop.
- Only per-resource ordering, explicit dependencies, declared concurrency
  limits, or active backpressure may delay dispatch. There is no polling
  interval, debounce window, or artificial delay.

### RBAC

| Verb | Required rule | Restriction |
| --- | --- | --- |
| `get` | `{resourceTypes:[Process], verbs:[get]}` | - |
| `list` | `{resourceTypes:[Process], verbs:[list]}` | - |
| `watch` | `{resourceTypes:[Process], verbs:[watch]}` | - |
| `create` | `{resourceTypes:[Process], verbs:[create], executionRefs:[Host/host-system]}` | Subject must have permission on the executionRef target |
| `update-spec` | `{resourceTypes:[Process], verbs:[update-spec]}` | Config publication or owning Provider controller |
| `update-status` | `{resourceTypes:[Process], verbs:[update-status]}` | Owning Process Provider controller only |
| `delete` | `{resourceTypes:[Process], verbs:[delete]}` | Blocked while finalizers exist |

Structural checks also enforce:
- `executionRef` must resolve to a Ready Host or Guest in the same Zone;
- `providerRef` must be `system-systemd` or `system-minijail` and be Ready;
- `domain` must be in `executionRef.allowedDomains`;
- `userRef` required when `domain=user` and no `defaultUserRef` on target;
- when `metadata.ownerRef` is set, the referenced resource must exist and its UID must be bound (no-cycle rule applies); owner phase (Ready/Degraded/Failed/etc.) does not block admission;
- budget fields must not exceed executionRef aggregate budget remaining.

### Reconcile

Process Provider reconcile loop:

1. Receive trigger (spec-generation-changed, execution-status-changed, owned-resource-changed, retry-due, etc.).
2. Validate ExecutionSpec fields: executionRef, domain, userRef, providerRef, template, sandbox, budget, mounts, dependencies.
3. Compile sandbox from SandboxSpec to provider-specific plan; compute sandboxRevisionDigest.
4. Compile all mounts from MountSpec array; verify all volumeRef targets are Ready.
5. If desiredLifecycle=stopped: stop any running process through the same
   provider-specific intentional-stop contract used by Finalize (including
   system-minijail `cgroup.kill` and broker wait/reap proofs); write
   Pending+stopped status; return converged.
6. If no running process: dispatch LaunchTicket to ProviderSupervisor asynchronously; write Launching condition.
7. ProviderSupervisor: verify ticket/resource/controller lease; spawn process via broker or systemd; obtain pidfd; return identity digest.
8. Record processIdentityDigest in status; write Ready condition via UpdateStatus.
9. If health check enabled: schedule periodic health checks.
10. On exit: classify exit (clean/crash/signal/timeout); apply restartPolicy; if restart, re-enter at step 6 with backoff; if maxRestarts exceeded, write Failed phase.
11. On execution-status-changed from Host/Guest: if executionRef becomes non-Ready, write Unknown condition; queue observation.

### Finalize

Owning Provider controller registers finalizer `process.<provider-name>/cleanup`.

Finalizer algorithm on `deletion-requested`:

1. Reverify the exact process/unit and its owned cgroup leaf. Ambiguity takes
   the quarantine path without a signal or subtree kill.
2. Signal the exact main process with SIGTERM and wait `drainTimeout`.
   A verified pidfd holder may use `pidfd_send_signal`; parenthood is not
   required. systemd-owned processes use the verified unit stop path.
3. For system-minijail, after main exit or the grace deadline, have the original
   broker parent write `1` to the anchored leaf's cgroup v2 `cgroup.kill`. This
   mandatory subtree operation replaces PID/PGID SIGKILL ownership and catches
   descendants that called `setsid(2)`.
4. Confirm system-minijail main exit only from the broker parent's relayed
   `waitid(P_PIDFD, ...)` result; pidfd poll readability is not proof. Also wait
   for `cgroup.events` to report `populated 0`. For system-systemd, require the
   verified unit's terminal transition and manager-owned subtree drain.
5. Release cgroup leaf (system-minijail) or unit (system-systemd) only after the
   applicable exit/subtree proofs.
6. Release any OFD locks/leases from this process.
7. Clear finalizer and return `finalized`.

On ambiguous state (pidfd closed, broker-parent ownership lost, cgroup identity
mismatch, `cgroup.kill` unavailable/failing, leaf still populated, or systemd
unit gone without a verified terminal transition), retain the finalizer and
write `Degraded`/`Unknown` with `process-exit-unconfirmed`. No broad kill,
signal, or `cgroup.kill` write targets an ambiguously owned candidate. A
success-shaped `finalized` result without the required proofs is prohibited.

---

## EphemeralProcess

### Purpose

`EphemeralProcess` is a one-shot asynchronous process that runs to terminal
state, retains its result for a configurable TTL, and is then cleaned up.
It shares the full `ExecutionSpec` with Process and adds one-shot-specific
fields. It does not reference or create a Process child.

### Spec schema (delta from ExecutionSpec)

```yaml
apiVersion: resources.d2bus.org/v3
type: EphemeralProcess
metadata:
  name: swtpm-pre-start-flush-abc123
  zone: dev
  ownerRef: Provider/device-tpm        # owning Provider; template is its signed component ID
spec:
  # All ExecutionSpec fields (same as Process):
  providerRef: Provider/system-minijail
  executionRef: Host/host-system
  domain: system
  userRef: null
  processClass: worker
  template: swtpm-flush               # plain ID within owning Provider's component descriptor
  configRef: null
  credentialRefs: []
  mounts: []
  sandbox: {}
  budget: {}
  networkUsage: null
  deviceUsage: []
  telemetry: {}
  # EphemeralProcess-specific fields:
  startDeadline: "60s"      # max time from spec commit to process start; default 60s
  runtimeDeadline: "300s"   # max process runtime after start; default 300s
  successfulTtl: "1h"       # retention after Succeeded; default 1h; D034
  failedTtl: "24h"          # retention after Failed; default 24h; D034
  incidentHold: false        # if true, cleanup is blocked pending explicit release
```

EphemeralProcess-specific fields:

| Field | Type | Required | Default | Bound | Description |
| --- | --- | --- | --- | --- | --- |
| `startDeadline` | duration string | no | `"60s"` | `"1s".."3600s"` | Maximum time from spec commit to process start. Expiry moves phase to Failed with `reason: start-deadline-exceeded`. |
| `runtimeDeadline` | duration string | no | `"300s"` | `"1s".."86400s"` | Maximum process wall-clock runtime after start. For system-minijail, expiry uses exact-main SIGTERM, bounded grace, then anchored leaf `cgroup.kill`; no PGID fallback. |
| `successfulTtl` | duration string | no | `"1h"` | `"0s".."7d"` | (D034) How long to retain the resource after Succeeded. TTL begins at `status.completedAt`. |
| `failedTtl` | duration string | no | `"24h"` | `"0s".."30d"` | (D034) How long to retain the resource after Failed. TTL begins at `status.completedAt`. |
| `incidentHold` | bool | no | `false` | - | If true, cleanup is blocked regardless of TTL until an authorized caller sets this to false. |

`processClass` for EphemeralProcess must be `worker`. A `controller` or
`service` EphemeralProcess is rejected at spec admission.

EphemeralProcess has no `restartPolicy`, `readiness`, `healthCheck`, or
`adoptionPolicy` fields. It runs once. If it fails to start or exits
non-zero, phase becomes `Failed` and the TTL begins at `completedAt`.

### Status schema

#### Three-layer status shape (D088)

D088 freezes `EphemeralProcess` status as three layers. The universal `ResourceStatus`
base (Layer 1) lives at top-level `status` and owns `observedGeneration`,
`phase`, `conditions`, `lastReconciledAt`, `startedAt`, `completedAt`, and
bounded `outcome`. The `EphemeralProcess`-specific status fields documented in this
section constitute the ResourceType-common `status.resource` (Layer 2) object
and never restate the universal base. Optional implementation-only observation
belongs in `status.provider` (Layer 3) with exactly `providerRef`, qualified
immutable `schemaId`, semver `MAJOR.MINOR` `schemaVersion`, numeric
`observedProviderGeneration`, and strict, bounded, redacted,
unknown-field-denied `details`. Generic API, CLI, and controllers MUST consume
only the base-only projection (`status` base plus `status.resource`).
Controllers MUST write all present layers atomically in one status mutation with
one expected revision. D087/D088 mapping is: shared observations go to
`status.resource`; implementation-specific bounded non-secret observations go to
`status.provider.details`; secret, large, or private observations go to an
optional Volume. D088 bounds apply: total status <= 64 KiB, `status.resource` <=
32 KiB, `status.provider.details` <= 32 KiB, 32 conditions, 64-entry lists/maps,
and 4 KiB strings; violations use `status-oversize`,
`status-provider-schema-invalid`, or `status-provider-overlap`.

Mapping convention: within this spec a reference to `status.<field>` denotes the ResourceType-common `status.resource.<field>` unless `<field>` is a universal base field (`observedGeneration`, `phase`, `conditions`, `lastReconciledAt`, `startedAt`, `completedAt`, `outcome`).

EphemeralProcess uses all common status fields plus:

```yaml
status:
  observedGeneration: 1
  phase: Succeeded                    # Pending|Ready|Succeeded|Degraded|Failed|Deleted|Unknown; EphemeralProcess steady lifecycle is Pending→Succeeded|Failed|Unknown; Deleted is a terminal event-only phase (row removed after emit)
  conditions: []
  lastReconciledAt: "2026-07-22T00:00:01.000Z"
  startedAt: "2026-07-22T00:00:02.000Z"
  completedAt: "2026-07-22T00:00:05.000Z"
  outcome:
    code: process-exited              # see outcome codes below
    exitCode: 0
    message: ""
    retryable: false
    occurredAt: "2026-07-22T00:00:05.000Z"
  # EphemeralProcess-specific:
  providerImplementation: ""
  processIdentityDigest: ""
  waitReapOwner: ""
  executionRef: ""
  domain: ""
  userRef: null
  sandboxRevisionDigest: null
  cleanupEligibleAt: null             # set after terminal phase + TTL; RFC 3339 UTC
  incidentHeld: false                 # mirrors spec.incidentHold at last reconcile
```

EphemeralProcess-specific status fields:

| Field | Type | Description |
| --- | --- | --- |
| `cleanupEligibleAt` | string? | RFC 3339 UTC timestamp when the resource becomes eligible for cleanup by the EphemeralProcess cleanup controller. Null until the terminal phase is entered and the TTL computed. Set after finalizers, incident holds, and owner deletion requirements are assessed. |
| `incidentHeld` | bool | True when spec.incidentHold=true is blocking cleanup. |

EphemeralProcess `outcome.code` stable values:

| Code | Meaning |
| --- | --- |
| `process-exited` | Process exited; exitCode reflects the actual exit code. |
| `process-crashed` | Process crashed with signal. exitCode may be absent. |
| `start-deadline-exceeded` | Process did not start within startDeadline. |
| `runtime-deadline-exceeded` | Process exceeded runtimeDeadline; was terminated. |
| `launch-failed` | ProviderSupervisor failed to spawn. |
| `execution-unavailable` | executionRef target became non-Ready before start. |
| `cancelled` | Explicitly deleted before terminal phase. |
| `unknown` | Exit not observable; controller restarted during run. |

EphemeralProcess steady-state lifecycle is strictly `Pending → (Succeeded | Failed | Unknown)`.
`Ready` and `Degraded` are not used in normal EphemeralProcess operation; they are
part of the shared schema but not expected in the EphemeralProcess lifecycle.
`Deleted` is terminal and event-only (row removed on emit).

### TTL and cleanup (D034)

EphemeralProcess terminal retention:

1. When `completedAt` is set (either Succeeded or Failed), the EphemeralProcess
   cleanup controller computes `cleanupEligibleAt`:
   - Succeeded: `completedAt + successfulTtl`
   - Failed: `completedAt + failedTtl`
2. If the EphemeralProcess has `incidentHold=true`, `cleanupEligibleAt` is set
   but cleanup is blocked until a caller sets `incidentHold=false`.
3. If the EphemeralProcess has active finalizers, cleanup waits for finalization.
4. If the EphemeralProcess has an `ownerRef` and the owner has `deletionRequestedAt`
   set, the owner's finalizer ordering takes precedence.
5. When `cleanupEligibleAt <= now()` and no holds exist, the cleanup controller
   calls the normal Delete API on the EphemeralProcess resource with an expected
   revision. It does not remove rows directly.
6. Pending and Unknown EphemeralProcesses never age out through TTL.
7. `cleanupEligibleAt` is written via UpdateStatus with expected revision;
   a conflict reloads and retries without loss.

### Conditions

| Condition type | Meaning | reason codes |
| --- | --- | --- |
| `Scheduled` | Assigned to a controller instance | `controller-unavailable` |
| `Launching` | Start attempt underway | `launch-pending`, `launch-failed` |
| `Running` | Process is actively running | `start-timed-out`, `runtime-deadline-nearing` |
| `CleanupPending` | Terminal; awaiting TTL expiry or incident hold release | `incident-held`, `ttl-pending`, `finalizer-pending` |

### RBAC

Same verbs and restrictions as Process, with `EphemeralProcess` as the ResourceType.
`update-spec.incidentHold` additionally requires the `incident-hold-release` sub-verb
on a dedicated Role rule or admin scope.

### Reconcile

EphemeralProcess Provider reconcile loop:

1. Receive trigger.
2. Validate ExecutionSpec fields; check startDeadline not yet exceeded.
3. Compile sandbox and mounts.
4. Dispatch LaunchTicket to ProviderSupervisor; write Launching condition.
5. ProviderSupervisor spawns process; obtains pidfd; returns identity digest.
6. Write Running condition and startedAt.
7. Monitor process exit via broker-relayed wait/reap status (system-minijail)
   or the verified systemd unit transition. Pidfd readability alone is not exit
   status.
8. On exit: classify exit code/class; write Succeeded/Failed phase, completedAt, outcome.
9. Compute cleanupEligibleAt and write it via UpdateStatus.
10. If runtimeDeadline exceeded before exit: run the Process finalizer's
    provider-specific intentional-stop sequence. For system-minijail this is
    exact-main SIGTERM, bounded grace, mandatory anchored-leaf `cgroup.kill`,
    broker wait/reap, and empty-leaf proof; then write
    Failed(runtime-deadline-exceeded).
11. If startDeadline exceeded before launch: write Failed(start-deadline-exceeded).

### Finalize

Same provider-specific teardown as Process. For system-minijail that includes
the broker-parent wait/reap result and mandatory cgroup v2 `cgroup.kill` for an
unambiguous intentional stop before empty-leaf proof and rmdir. EphemeralProcess
finalizers are cleared only after process exit and subtree drain are confirmed.
The resource is never force-deleted while the process may still be running or
ownership is ambiguous.

---

## User

### Purpose

`User` represents a named host identity in the Zone. `Provider/system-core`
discovers and reconciles Users from the host NSS/passwd/group database. User
spec contains the configured identity name used for NSS lookup. Status
reflects the discovered OS state. Users are referenced by Process `userRef`,
Volume ACL `ownerRef`/`groupRef`, and RoleBinding subjects.

User is always in the same Zone as the Hosts and Processes that reference it.
A User is not specific to one Host; multiple Hosts may refer to the same User.

### Spec schema

```yaml
apiVersion: resources.d2bus.org/v3
type: User
metadata:
  name: alice             # required; ^[a-z][a-z0-9-]*$; max 63; Zone-local resource name; used in userRef as User/alice
  zone: dev
spec:
  osUsername: alice       # required; the actual OS username presented to NSS getpwnam; validated by host OS username rules
  displayName: ""         # optional; bounded human-readable label; max 128 chars
  groups: []              # optional; additional group names to verify; 0..64 items
```

`metadata.name` is the canonical Zone-local resource identity used in all
`userRef: User/<name>` references throughout the API. It must satisfy the
ResourceName grammar (`^[a-z][a-z0-9-]*$`, max 63 chars).

`spec.osUsername` is the actual OS username passed to NSS `getpwnam`. It is
validated independently by the host OS username rules: bounded string (1..255
bytes), no NUL bytes, no control characters (U+0000-U+001F, U+007F), no path
separator characters (`/`, `\`). `spec.osUsername` does not need to satisfy
the ResourceName regex; it may contain underscores or other characters permitted
by the OS but excluded from the ResourceName grammar. The two fields may be
equal (common case) or differ (e.g. `metadata.name: alice-admin`,
`spec.osUsername: alice_admin`).

Full field table:

| Field | Type | Required | Default | Bound | Description |
| --- | --- | --- | --- | --- | --- |
| `osUsername` | string | yes | - | 1..255 bytes; no NUL/control/`/` | OS username passed to NSS `getpwnam` for UID/GID/home/shell/group resolution. Validated against host OS username rules, not ResourceName grammar. |
| `displayName` | string | no | `""` | max 128 chars | Human-readable display name shown in operator UI. Not used for NSS lookup. Bounded UTF-8; no control characters. |
| `groups` | `[string]` | no | `[]` | 0..64 items; each max 63 chars | Additional OS group names that the reconcile step verifies membership in. Each item matches `^[a-z_][a-z0-9_-]*$` at validation. Mismatch is surfaced in status conditions. |

User spec contains no credential material, SSH public key, PAM configuration,
or authentication token of any kind. Credentials are Credential resources.

### Status schema

#### Three-layer status shape (D088)

D088 freezes `User` status as three layers. The universal `ResourceStatus`
base (Layer 1) lives at top-level `status` and owns `observedGeneration`,
`phase`, `conditions`, `lastReconciledAt`, `startedAt`, `completedAt`, and
bounded `outcome`. The `User`-specific status fields documented in this
section constitute the ResourceType-common `status.resource` (Layer 2) object
and never restate the universal base. Optional implementation-only observation
belongs in `status.provider` (Layer 3) with exactly `providerRef`, qualified
immutable `schemaId`, semver `MAJOR.MINOR` `schemaVersion`, numeric
`observedProviderGeneration`, and strict, bounded, redacted,
unknown-field-denied `details`. Generic API, CLI, and controllers MUST consume
only the base-only projection (`status` base plus `status.resource`).
Controllers MUST write all present layers atomically in one status mutation with
one expected revision. D087/D088 mapping is: shared observations go to
`status.resource`; implementation-specific bounded non-secret observations go to
`status.provider.details`; secret, large, or private observations go to an
optional Volume. D088 bounds apply: total status <= 64 KiB, `status.resource` <=
32 KiB, `status.provider.details` <= 32 KiB, 32 conditions, 64-entry lists/maps,
and 4 KiB strings; violations use `status-oversize`,
`status-provider-schema-invalid`, or `status-provider-overlap`.

Mapping convention: within this spec a reference to `status.<field>` denotes the ResourceType-common `status.resource.<field>` unless `<field>` is a universal base field (`observedGeneration`, `phase`, `conditions`, `lastReconciledAt`, `startedAt`, `completedAt`, `outcome`).

```yaml
status:
  observedGeneration: 1
  phase: Ready                          # Pending|Ready|Succeeded|Degraded|Failed|Deleted|Unknown; long-lived resources do not steadily use Succeeded; Deleted is a terminal event-only phase (row removed after emit)
  conditions: []
  lastReconciledAt: "2026-07-22T00:00:01.000Z"
  startedAt: null
  completedAt: null
  outcome: null
  # User-specific:
  uid: null                             # discovered OS UID; u32; null if unknown
  gid: null                             # primary discovered OS GID; u32; null if unknown
  homeExists: false                     # true if home directory exists and is accessible
  shellValid: false                     # true if login shell is non-empty and exists
  sessionManagerAvailable: false        # true if systemd user manager is running for this user
  groupMembershipVerified: false        # true when spec.groups membership verified
  observedGroups: []                    # observed OS group names; 0..256 items; bounded strings
```

User-specific status fields:

| Field | Type | Description |
| --- | --- | --- |
| `uid` | u32? | Discovered OS UID from NSS getpwnam. Null if NSS lookup fails. |
| `gid` | u32? | Discovered primary GID. Null if NSS lookup fails. |
| `homeExists` | bool | True if the home directory reported by NSS exists and is accessible to d2b's check. |
| `shellValid` | bool | True if the login shell reported by NSS is non-empty and the path exists. |
| `sessionManagerAvailable` | bool | True if the systemd user manager for this UID is running and responding. Relevant to user-domain Processes. |
| `groupMembershipVerified` | bool | True when spec.groups are all present in the observed group membership. |
| `observedGroups` | `[string]` | Observed OS group membership (names only; no GIDs). Max 256 items; each max 63 chars. |

No UID or GID appears in audit payloads. Status fields containing numeric OS
identifiers are diagnostic only and must not be used as authorization input;
authorization uses the canonical Zone `User/<name>` subject reference.

### Conditions

| Condition type | Ready = True when | reason codes |
| --- | --- | --- |
| `UserFound` | NSS getpwnam(spec.osUsername) returns a record | `nss-lookup-failed`, `nss-lookup-timeout`, `user-not-found` |
| `HomeReady` | homeExists is true | `home-directory-missing`, `home-directory-inaccessible` |
| `ShellValid` | shellValid is true | `login-shell-missing`, `login-shell-invalid` |
| `GroupsVerified` | All spec.groups are present in observed membership | `group-membership-missing`, `group-not-found` |
| `SessionManagerReady` | sessionManagerAvailable is true | `session-manager-unavailable`, `session-manager-unknown` |

Phase is `Ready` when at least `UserFound` and `HomeReady` are True.
`SessionManagerReady` being False produces `Degraded` (not `Failed`) because
user-domain Processes cannot start but the user identity itself is valid.
`GroupsVerified` being False produces `Degraded` when `spec.groups` is
non-empty.

### RBAC

| Verb | Required rule | Restriction |
| --- | --- | --- |
| `get` | `{resourceTypes:[User], verbs:[get]}` | Numeric UID/GID in status may require elevated role |
| `list` | `{resourceTypes:[User], verbs:[list]}` | - |
| `watch` | `{resourceTypes:[User], verbs:[watch]}` | - |
| `create` | `{resourceTypes:[User], verbs:[create]}` | Config publication controller or Provider/system-core bootstrap only |
| `update-spec` | `{resourceTypes:[User], verbs:[update-spec]}` | Config publication controller |
| `update-status` | `{resourceTypes:[User], verbs:[update-status]}` | Provider/system-core controller only |
| `delete` | `{resourceTypes:[User], verbs:[delete]}` | Blocked while any Process has `userRef: User/<name>` or any Volume has `ownerRef: User/<name>` |

### Reconcile

Provider/system-core User reconcile loop:

1. Receive trigger: spec-generation-changed, dependency-changed, startup-relist, scheduled-observe.
2. Perform NSS `getpwnam(spec.osUsername)` with bounded timeout (default 5 s).
3. If NSS lookup fails: write UserFound=False, phase=Degraded/Failed (Failed only after consecutive failures exceed threshold); return requeue-at.
4. Record uid, gid, home directory from NSS record.
5. Check home directory existence/accessibility (non-blocking stat).
6. Check login shell existence.
7. Query observed group membership (bounded call to `getgrouplist` or equivalent).
8. Verify spec.groups subset.
9. If systemd user manager check is needed (any Process with domain=user targets this User): query user supervisor for session manager availability.
10. Write User status via UpdateStatus with expected revision.
11. Return `converged` or schedule next `scheduled-observe` at the configured interval.

User reconcile must not hold OS-user credentials or perform
authentication/login. It is purely observational.

### Finalize

User uses no controller-managed finalizer. Deletion is blocked by the
structural check when any Process `userRef` or Volume `ownerRef`/`groupRef`
references this User. The operator must remove or update all such references
before deletion succeeds.

---

## Endpoint

`Endpoint` is a standard ResourceType (D092): the promoted, provider-neutral
identity for a **stable managed endpoint** (a service/device/control/data/
transport attachment point) that is referenced across a resource/controller/
Provider boundary. Per-connection or high-churn carriage (named streams,
`OwnedTransport` handles, inherited fds, `operationId`) is not an Endpoint.

An `Endpoint` follows the D089 layered spec and D088 layered status. It carries
**no** raw path, address, CID, port, fd, or credential; Core/ProviderSupervisor
resolves the `Endpoint` resource to a private transport/FD only through the
EffectPort/LaunchTicket path.

### Endpoint base spec (Layer 2)

| Field | Type | Default | Description |
| --- | --- | --- | --- |
| `providerRef` | ResourceRef | required | `Provider/<name>` selecting the semantic Endpoint implementation/controller. |
| `producerRef` | ResourceRef | required | The producing resource: `Process/<name>`, `Device/<name>`, `Guest/<name>`, `Host/<name>`, or a qualified `<provider>.d2bus.org.<Type>/<name>`. |
| `endpointClass` | `service\|device\|transport\|control\|data` | required | Closed class of the endpoint. |
| `transport` | `unix\|vsock\|tcp\|fd-attachment\|opaque-carriage` | required | Closed transport class only - never a raw path/address/CID/port/fd. |
| `purpose` | string | required | Bounded stable purpose label; max 63 chars. |
| `serviceFingerprint` | string? | `null` | Bounded service/schema fingerprint or capability class; no raw schema bytes. |
| `locality` | `host-local\|guest-local\|cross-domain\|zone-local` | required | Closed locality/visibility class. |
| `visibility` | `owner\|provider\|zone` | `provider` | Closed visibility scope for consumers. These are the only accepted values; there are no aliases such as `private`, `provider-internal`, `zone-private`, or `authorized-consumers`. |
| `attachmentPolicy` | object | `{ supported: false }` | Bounded attachment support/policy (supported, max attachments); no locator. |
| `consumerPolicy` | object | `{}` | The only finer-grained consumer gate: bounded `allowedSubjects` (exact same-Zone ResourceRefs), `allowedProviderComponents` (exact signed component IDs), and `allowedOperations` (`resolve\|attach\|observe`). Omitted lists add no restriction beyond `visibility`; a present list is an allowlist, and all present dimensions must match. |
| `lifecyclePolicy` | `pinned\|recycle-with-producer\|recreate-on-generation` | `recycle-with-producer` | Closed lifecycle/recycle policy. |
| `provider` | ProviderExtension? | `null` | Optional canonical `spec.provider = { schemaId, schemaVersion, settings }` (D089); implementation-only, no locator. |

`visibility` is only the coarse candidate scope: `owner` admits the exact
`metadata.ownerRef` subject, `provider` admits authenticated Provider subjects
and signed Provider components in the Zone, and `zone` admits all other
same-Zone subjects. It never grants
resolution by itself. Normal Role/RoleBinding authorization and every
`consumerPolicy` allowlist still apply. `consumerPolicy` is strict and
deny-unknown; providers may not add visibility values or string/array policy
aliases.

### Endpoint base status (Layer 2)

On top of the universal base (including `status.update`, D091), the Endpoint
ResourceType base status carries:

| Field | Values |
| --- | --- |
| `readiness` | `Pending\|Ready\|Degraded\|Unavailable\|Unknown` |
| `observedProducerGeneration` | Numeric generation of the producer resource realized here |
| `observedResourceGeneration` | Numeric generation of this Endpoint resource observed |
| `endpointGeneration` | Numeric; bumped when the producer restarts/recycles so consumers re-resolve |
| `connectionAvailability` | Closed enum `available\|draining\|unavailable` |
| `leaseAvailability` | Closed enum `none\|available\|exhausted` |
| `capability` | Closed capability-class observations |
| `locality` | Observed closed locality class |
| `transport` | Observed closed transport class |
| `conditions` | Bounded standard conditions |

No raw locator (path/address/CID/port/fd/credential) appears in status. A
Provider-specific implementation extension uses the optional `status.provider`
(D088), never restating a base field.

### Ownership, resolution, and references

- `metadata.ownerRef` gives the Endpoint its lifecycle and child-first deletion
  (the owning component/Process/Device/Guest/Volume-`Export`). Core creates
  static component Endpoints from the signed manifest's Endpoint templates;
  dynamic controllers create their owned Endpoints. Static Endpoints may be
  Nix/API-authored only where the ResourceType schema permits.
- Consumers reference an endpoint by `Endpoint/<name>` ResourceRef and gain a
  dependency edge; a producer restart bumps `endpointGeneration` and fires the
  consumer's `dependency-changed` reconcile trigger.
- Resolving an `Endpoint` to a live transport/FD requires authorization; an
  unauthorized resolve is denied with a typed error and no locator is returned.
- Core admission first applies the closed `visibility` scope, then normal
  Role/RoleBinding authorization, then every present `consumerPolicy`
  allowlist. It authenticates the subject and signed Provider component rather
  than accepting either identity from the request body. A mismatch at any
  layer returns `endpoint-resolve-denied`.
- The virtiofs `Export` resource remains the attachment lifecycle owner and
  references its `Endpoint` where the exported endpoint is independently
  consumed (`ADR-046-resources-volume`).

---

## Cross-type security invariants

### Placement validation

Core enforces at every Process/EphemeralProcess spec create or update-spec:

1. `executionRef` resolves to an existing Host or Guest in the same Zone.
2. The resolved Host or Guest is not in `Deleted` or `Failed` phase.
3. `domain` is in `executionRef.spec.allowedDomains`.
4. If `domain=user` and no `defaultUserRef` on target: `userRef` is present.
5. `userRef` resolves to a `User/<name>` resource (if present).
6. `providerRef` resolves to an installed and Ready Process Provider.
7. No secret bytes, raw capability lists, cgroup paths, host paths, or minijail
   argument strings are in spec payload. Violation → `spec-security-violation` and rejection.
8. Process `budget` fields do not exceed the `executionRef` remaining aggregate budget.
9. When `spec.template` is set: the template ID is resolved at runtime by the Provider registered as controller for the semantic owner resource (`metadata.ownerRef`). If `metadata.ownerRef` is absent, the template is resolved through `spec.providerRef`. The controller verifies the template maps to a declared component in its signed descriptor before launch. `metadata.ownerRef` may be any resource type (Provider, Volume, Network, Device, or other).
10. `spec.template` may be set with any `metadata.ownerRef` type or with no `metadata.ownerRef`. When ownerRef is absent the providerRef itself is the resolver. No type restriction on ownerRef when template is set.
11. All `mounts[].volumeRef` and `credentialRefs[]` resolve in the same Zone.
12. All `deviceUsage[].deviceRef` are present in `executionRef.spec.deviceAttachments`.
13. `networkUsage.networkRef` is present in `executionRef.spec.networkAttachments` if set.

Violations are returned as `resource-schema-invalid` or `authorization-denied`
with a stable error code and a bounded redacted message. No partial mutation.

### User-only Host posture (no isolation)

When `Host.spec.isolationPosture = "none"`:

- `allowedDomains` must be exactly `[user]`. System processes rejected.
- `defaultDomain` must be `user`.
- `defaultUserRef` must be set.
- The Host status `isolationPosture = "none"` is surfaced in every operator CLI/UI view of this Host and its Processes as an explicit "no isolation boundary" warning.
- A `Process.executionRef` pointing to this Host must carry `domain=user`.
- The operator UI may not suppress or minimize this warning.
- `ProcessEffect` audit records (launch, stop, adopt, quarantine) for every child Process/EphemeralProcess on this Host must carry a stable `no_isolation=true` attribute so the no-isolation posture is recorded in the authoritative audit trail and cannot be absent from them. This attribute belongs on ProcessEffect records only; it must NOT appear on OTEL metric labels, span attributes, log fields, or audit records for other event kinds.

### Pidfd non-exportability

The pidfd file descriptor is strictly local lifecycle authority. A Provider
controller holds only an opaque handle. For system-minijail, the broker parent
retains the original pidfd and ProviderSupervisor may hold a verified duplicate:

- It is never stored in the resource store.
- It is never sent over d2b-bus, ComponentSession, or any ttrpc call.
- It is never written to any log, metric (with a name/PID label), audit record, or status field.
- It is never inferred from status by a caller.
- No API method accepts a pidfd from a caller.
- Every ProviderSupervisor restart reacquires its duplicate through
  re-verification; a Provider-controller restart transfers no raw fd and does
  not change the broker's parenthood.
- A non-parent holder may poll readability and call `pidfd_send_signal` for the
  exact process, but cannot call wait/reap or derive exit status from
  readability.
- Only the system-minijail broker that called `clone3` performs
  `waitid(P_PIDFD)`, collects exit status, and reaps; it relays a typed terminal
  result to ProviderSupervisor/controller.

### Process isolation rule

A Process resource spec must not contain:

- A raw numeric UID, GID, or supplementary group list (user identity comes from `userRef`);
- A raw cgroup path (placement is derived from Resource UID/Provider);
- A raw socket address or file path accessible from the host (these come from Volume/Network resource refs);
- A raw seccomp BPF bytecode program;
- A raw capability bitmask;
- A raw minijail argument string;
- A raw systemd unit property fragment;
- A raw broker operation name or parameters;
- An environment variable containing a credential, token, or secret byte.

### Status redaction

The following values MUST NOT appear in any Process, EphemeralProcess, Host,
Guest, or User status field, audit record, telemetry label, or log output:

- PID values;
- cgroup paths;
- socket file paths;
- argv or environment variable contents;
- credential bytes or token prefixes;
- SSH hostnames or IP addresses;
- terminal byte streams or partial output;
- raw Provider error messages with internal paths.

Bounded stable machine codes (`reason`, `code`) are always permitted.
Bounded redacted operator messages (max 512 chars per message, UTF-8 clean,
no control characters) are permitted in `status.conditions[].message` and
`status.outcome.message`.

---

## ProcessRole disposition table

Every current ProcessRole in `packages/d2b-core/src/processes.rs` at baseline
`b5ddbed6` is classified below. No role may be removed before its `Successor`
is integrated and all tests pass.

| ProcessRole | v3 classification | v3 ResourceType | v3 Provider | Successor condition |
| --- | --- | --- | --- | --- |
| `HostReconcile` | Controller logic, not a Process | - | `system-core` Host reconciler | Delete after Host reconcile handler parity (ADR046-exec-005) |
| `StoreVirtiofsPreflight` | Controller observation, not a Process | - | `volume-virtiofs` or `volume-local` Volume controller | Delete after Volume Provider parity (ADR046-primitives-003) |
| `SwtpmPreStartFlush` | `EphemeralProcess` | EphemeralProcess | `device-tpm` Provider | Delete after device-tpm EphemeralProcess integration |
| `Swtpm` | `Process` | Process | `device-tpm` Provider; owned by Guest | Delete after device-tpm Process integration |
| `Virtiofsd` | `Process` | Process | `volume-virtiofs` Provider; owned by Volume attachment | Delete after volume-virtiofs Process integration |
| `Video` | `Process` | Process | `device-gpu` Provider; owned by Guest | Delete after device-gpu Process integration |
| `Gpu` | `Process` | Process | `device-gpu` Provider; owned by Guest | Delete after device-gpu Process integration |
| `GpuRenderNode` | `Process` | Process | `device-gpu` Provider; owned by Guest | Delete after device-gpu Process integration |
| `Audio` | `Process` | Process | `audio-pipewire` Provider; owned by Guest or Host user | Delete after audio-pipewire Process integration |
| `CloudHypervisorRunner` | `Process` | Process | `runtime-cloud-hypervisor` Provider; owned by Guest | Delete after runtime-cloud-hypervisor Process integration |
| `QemuMediaRunner` | `Process` | Process | `runtime-qemu-media` Provider; owned by Guest | Delete after runtime-qemu-media Process integration |
| `VsockRelay` | `Process` | Process | `transport-vsock` Provider; owned by Guest | Delete after transport-vsock Process integration |
| `OtelHostBridge` | `Process` | Process | `observability-otel` Provider; owned by Host or Guest | Delete after observability-otel Process integration |
| `GuestSshReadiness` | Controller observation, not a Process | - | `runtime-cloud-hypervisor` controller | Delete after GuestControlHealth-equivalent in runtime-ch controller; SSH compat window retained |
| `GuestControlHealth` | Controller observation, not a Process | - | `runtime-cloud-hypervisor` controller | Delete after runtime-ch Guest health check parity |
| `Usbip` | `Process` / `EphemeralProcess` | Process + EphemeralProcess | `device-usbip` Provider; owned by Host or Guest | Delete after device-usbip integration |
| `SecurityKeyFrontend` | `Process` | Process | `device-security-key` Provider; owned by Guest, user domain | Delete after device-security-key Process integration |
| `WaylandProxy` | `Process` | Process | `display-wayland` Provider; owned by Guest or Host | Delete after display-wayland Process integration |

The ProcessRole/VmProcessDag/processes.json contract is retained until every
role's successor is integrated and passes the corresponding process conformance
test.

---

## Nix configuration, validation, and generation lifecycle

Bundle format, generation activation algorithm, configuration generation controller, audit events, and prior generation retention mechanics are defined in the Zone configuration lifecycle spec. This section covers only the Nix authoring shape, ResourceType-specific validation rules, canonical ResourceEnvelope JSON, per-type Nix examples, and type-specific deletion cascade behaviour.

### Option tree

```
# Artifact catalog - derivation-valued inputs; separate from and peer to zone resources
d2b.artifacts.<id> = {
  package = <derivation>;        # required; any Nix derivation
  type    = "<artifact-type>";   # required; "provider" | "nixos-system"
};

# Zone resources - pure data, no derivation values; artifact IDs are plain bounded strings
d2b.zones.<zone>.resources.<name> = {
  type     = "<ResourceType>";   # required; one of Host Guest Process EphemeralProcess User
  metadata = {                   # optional; author-settable metadata only
    ownerRef    = null;          # optional; "<Type>/<name>" ResourceRef; null = no owner
    labels      = { };           # optional; string-to-string attrs for selection/grouping
    annotations = { };           # optional; arbitrary string-to-string attrs
  };
  spec = { /* exact ResourceType spec fields; artifact IDs are plain bounded strings, not ResourceRefs */ };
};
```

All five ResourceTypes share a single flat `resources` attrset under each Zone. `metadata.name` is derived from the `resources.<name>` attrset key (satisfies `^[a-z][a-z0-9-]*$`, max 63 chars). `metadata.zone` is derived from the enclosing `d2b.zones.<zone>` key. `apiVersion` defaults to `"resources.d2bus.org/v3"`. Because `resources` is a flat attrset, no two entries may share the same `<name>` key regardless of `type`.

**Author-settable metadata fields** (`metadata` submodule; all optional):

| Nix field | Nix type | Notes |
| --- | --- | --- |
| `metadata.ownerRef` | `types.nullOr types.str` | ResourceRef (`<Type>/<name>`) of the owning resource. When the owner is deleted, its controller cascades Delete to resources with `metadata.ownerRef` pointing at it. Serialized to `metadata.ownerRef` in the bundle JSON; NOT a `spec` field. |
| `metadata.labels` | `types.attrsOf types.str` | Key-value string pairs. Keys: `^[a-z][a-z0-9-./]*$`, max 63 chars; values: max 256 chars. |
| `metadata.annotations` | `types.attrsOf types.str` | Arbitrary key-value string pairs. Keys: `^[a-zA-Z0-9-./]*$`, max 253 chars; values: max 4096 chars. |

**Derived and read-only** - absent from the Nix option; assigned by core or the resource store; not author-configurable:

| Field | Who sets it | Description |
| --- | --- | --- |
| `metadata.name` | Nix compiler | Derived from `resources.<name>` attrset key |
| `metadata.zone` | Nix compiler | Derived from enclosing `d2b.zones.<zone>` key |
| `apiVersion` | Nix compiler | Always `"resources.d2bus.org/v3"` |
| `metadata.managedBy` | Activation controller / core | Valid values: `"configuration"` (set by activation controller for all Nix-declared resources), `"controller"` (set by core for controller-created resources), `"api"` (set by core for API-created resources). Immutable after first Create; NOT author-settable |
| `metadata.configurationGeneration` | Activation controller | NixOS generation number recorded at activation; refreshed on each generation change even for unchanged specs; NOT author-settable |
| `metadata.uid` | Resource store | Stable opaque identity assigned at Create |
| `metadata.generation` | Resource store | Incremented on each spec update |
| `metadata.resourceVersion` | Resource store | Monotonically increasing revision stamp |
| `metadata.createdAt`, `metadata.updatedAt` | Resource store | RFC3339 timestamps |
| `metadata.deletionRequestedAt` | Resource store | RFC3339 timestamp set when a Delete is accepted; absent while the resource is not pending deletion; resource remains in its current phase (not a `Deleting` phase) until finalizers release and core performs atomic row/index removal |
| `metadata.finalizers` | Controllers | Managed by owner/provider controllers; not author-settable |
| `status` | Provider/core | Entirely absent from Nix option; never author-settable |

### Artifact catalog

`d2b.artifacts` is a top-level Nix option, peer to `d2b.zones`. It is the only place where derivation-valued inputs (Nix packages, NixOS system closures) appear in the d2b Nix configuration. ResourceSpecs never contain derivation values or store paths; they reference artifacts by plain bounded string IDs.

| Nix option | Type | Required | Notes |
| --- | --- | --- | --- |
| `d2b.artifacts.<id>` | submodule | - | One entry per artifact. `<id>` satisfies `^[a-z][a-z0-9-]*$`, max 63 chars |
| `.package` | derivation | yes | Any Nix derivation; built and hashed at eval/build time |
| `.type` | enum string | yes | `"provider"` \| `"nixos-system"` |

**Type semantics:**

- `"provider"` - a Provider binary/closure; referenced by `Provider.spec.artifactId` (defined in the Provider ResourceType spec, separate document)
- `"nixos-system"` - a complete NixOS system closure (kernel + initrd + rootfs); referenced by `Guest.spec.systemArtifactId` (top-level spec field) for local VM Providers

**Build-time catalog validation:**

- Duplicate `<id>` keys: rejected at Nix parse time (attrset key uniqueness); no explicit check required
- Missing catalog entry: `Guest.spec.systemArtifactId` referencing an absent `d2b.artifacts.<id>` → hard eval error (rule 17 below)
- Type mismatch: `Guest.spec.systemArtifactId` referencing a `type = "provider"` entry → hard eval error (rule 17)

**Private artifact catalog emission:**

At build time Nix emits a global private integrity-pinned artifact catalog installed at `/etc/d2b/artifact-catalog.json` (owned `root:d2bd`, mode `0640`). The catalog maps each `<id>` to its artifact type, SHA-256 digest, closure size, and store path. Store paths are in the catalog for trusted runtime use (resolving derivation closures) but must never appear in public resource bundle envelopes, resource spec/status fields, or audit records.

```json
{
  "catalogVersion": 1,
  "entries": {
    "<id>": { "sha256": "<hex>", "size": <bytes>, "storePath": "/nix/store/...", "type": "<artifact-type>" }
  }
}
```

The resource bundle references artifacts by plain `<id>` string in ResourceEnvelope `spec` fields and includes the catalog integrity anchor (`artifactCatalogDigest`) in the bundle envelope so the activation controller can verify the catalog matches the bundle at activation time, per `ADR-046-nix-configuration.md` Bundle contract (canonical) (D119).

**Referencing artifacts in ResourceSpecs:**

Use the plain bounded `<id>` string as the `Guest.spec.systemArtifactId` value (or `Provider.spec.artifactId` in the Provider spec). The `<id>` is neither a ResourceRef nor a store path:

```nix
# In artifact catalog (derivation-valued; separate from zone resources)
d2b.artifacts.dev-vm-system = { package = nixosSystemForDevVm.config.system.build.toplevel; type = "nixos-system"; };
d2b.artifacts.display-wayland-v1 = { package = pkgs.d2b-provider-display-wayland; type = "provider"; };

# In zone resources - plain string IDs only; no derivation values in spec
d2b.zones.dev.resources.dev-vm.spec.systemArtifactId = "dev-vm-system";
# Provider resource (separate spec): spec.artifactId = "display-wayland-v1";
```

### Nix resource option schema

Each entry in `d2b.zones.<zone>.resources` is a `types.submodule` with:

| Nix option | Nix type | Required | Notes |
| --- | --- | --- | --- |
| `type` | `types.enum ["EphemeralProcess" "Guest" "Host" "Process" "User"]` | yes | Selects the ResourceType |
| `metadata` | `types.submodule` | no | Author-settable metadata: `ownerRef`, `labels`, `annotations` only; all optional |
| `spec` | `types.submodule` (type-dependent) | yes | Exact ResourceType spec fields; auto-generated from `packages/d2b-contracts/src/v3/schemas/<Type>.json`; field names 1:1 with the spec tables above; no renaming |

`spec` submodule fields are generated from the committed ResourceTypeSchema JSON. For `spec.provider.settings`, sub-fields are additionally constrained by the Provider's `providerNixSettingsSchema` attribute; the schema version is recorded in the private bundle integrity metadata and verified by the activation controller (fail-closed on mismatch).

**Key `spec` constraints by type** (full rule list below):

| `type` | Key `spec` constraints enforced at eval time |
| --- | --- |
| `"Host"` | `spec.providerRef` must be a declared substrate Provider; `spec.allowedDomains` must include `spec.defaultDomain`; `spec.isolationPosture = "none"` implies `spec.allowedDomains == ["user"]`, `spec.defaultDomain == "user"`, and `spec.defaultUserRef` set (and vice-versa: that tuple implies `isolationPosture = "none"`) |
| `"Guest"` | `spec.providerRef` must be a declared runtime Provider; `spec.provider.settings` validated against Provider settings schema; `spec.systemArtifactId`, if set, must exist in `d2b.artifacts` with `type="nixos-system"` (rule 17) |
| `"Process"` | `spec.executionRef` resolves to a declared Host or Guest; `spec.domain ∈ executionRef.spec.allowedDomains`; `spec.template`, when set, is resolved at runtime by the controller of `metadata.ownerRef` (or `spec.providerRef`); no ownerRef type restriction |
| `"EphemeralProcess"` | Same as Process; `spec.startDeadline` and `spec.runtimeDeadline` required; no `spec.restartPolicy` |
| `"User"` | `spec.osUsername` required (1..255 bytes, no NUL/control/path separators); `spec.groups` items match `^[a-z_][a-z0-9_-]*$` |

### Eval-time validation rules

The Nix resource compiler enforces all of the following at `nixos-rebuild build` time. Each violation is a hard eval error with a stable rule code:

1. **ResourceType field**: `type` must be exactly one of the five valid values.
2. **ResourceName grammar**: every `resources.<name>` key matches `^[a-z][a-z0-9-]*$`, max 63 chars.
3. **ResourceRef resolution**: every `spec.*Ref` string field and `metadata.ownerRef` that names a resource in the same Zone resolves to a declared resource in `d2b.zones.<zone>.resources`. Cross-Zone refs are rejected. `metadata.ownerRef` may name any existing same-Zone resource subject to the no-self (rule 11) and no-cycle rules; no per-type owner restrictions apply. `spec.systemArtifactId` is a plain bounded string ID (not a ResourceRef) and is validated by rule 17 against the artifact catalog, not by this rule.
4. **Provider kind check**: `spec.providerRef` for `type="Host"` must be a substrate Provider; for `type="Guest"` must be a runtime Provider; for `type="Process"` or `type="EphemeralProcess"` must be a Process Provider.
5. **Domain inclusion**: `spec.defaultDomain ∈ spec.allowedDomains`; for Process/EphemeralProcess, `spec.domain ∈ executionRef.spec.allowedDomains`.
6. **User-domain userRef**: when `spec.domain == "user"` and `spec.executionRef` target has no `spec.defaultUserRef`, `spec.userRef` must be set.
7. **Isolation-posture constraints** (bidirectional): `spec.isolationPosture = "none"` implies `spec.allowedDomains == ["user"]`, `spec.defaultDomain == "user"`, and `spec.defaultUserRef` is set. Conversely, `spec.allowedDomains == ["user"]` + `spec.defaultDomain == "user"` + `spec.defaultUserRef` set implies `spec.isolationPosture == "none"`; `null` is rejected to prevent evasion of the no-isolation warning. Any value for `isolationPosture` other than `null` or `"none"` is a hard eval error.
8. **osUsername bounds**: `spec.osUsername` 1..255 bytes, no NUL/control/path-separator characters.
9. **Budget bounds**: `spec.budget.<dim>.request ≤ spec.budget.<dim>.limit` for every dimension where both are set.
10. **Mount uniqueness**: no two entries in `spec.mounts` have the same `mountPath`.
11. **No self-referential ownerRef**: `metadata.ownerRef` must not equal `"<type>/<name>"` of the resource itself.
12. **No inline secrets**: `spec.provider.settings` string fields with credential-suffix names (`password`, `token`, `secret`, `key`, `cert`, `credential`) are hard eval errors; credentials must be `Credential/<name>` refs.
13. **Provider settings schema**: each Provider declaring `providerNixSettingsSchema` has `spec.provider.settings` validated against that schema; SHA-256 recorded as `providerSchemaFingerprint`.
14. **Groups grammar**: each item in `spec.groups` matches `^[a-z_][a-z0-9_-]*$`.
15. **Template scoping**: `spec.template`, when set, satisfies `^[a-z][a-z0-9-]*$`, max 63 chars (grammar check). Template resolution is runtime: the registered controller of `metadata.ownerRef` (or `spec.providerRef` when ownerRef is absent) must declare this ID. No ownerRef type restriction at eval time.
16. **Flat namespace uniqueness**: no two entries in `d2b.zones.<zone>.resources` may share the same `<name>` key regardless of `type`.
17. **System artifact ID resolution**: `spec.systemArtifactId`, if set, must exist as a key in `d2b.artifacts` with `type = "nixos-system"`. Missing key or type mismatch is a hard eval error.

### Canonical ResourceEnvelope JSON shape

Every Nix-declared resource compiles to a `ResourceEnvelope` JSON object. `spec` is a direct 1:1 serialization of the Nix `spec` submodule (same field names, same nesting, same values; no renaming). `metadata.ownerRef` appears in the `metadata` object, NOT in `spec`. All JSON object keys are sorted alphabetically at every nesting level; order-significant arrays preserve declaration order; semantically unordered arrays are sorted.

```json
{
  "apiVersion": "resources.d2bus.org/v3",
  "metadata": {
    "annotations": {},
    "labels": {},
    "name": "<resource-name>",
    "ownerRef": null,
    "zone": "<zone-name>"
  },
  "spec": { /* all spec fields, keys sorted, defaults included; ownerRef is NOT a spec field */ },
  "type": "<ResourceType>"
}
```

`metadata.managedBy`, `metadata.configurationGeneration`, `metadata.deletionRequestedAt`, `metadata.uid`, `metadata.generation`, `metadata.resourceVersion`, `metadata.createdAt`, `metadata.updatedAt`, `metadata.finalizers`, and `status` are absent from the bundle envelope; they are set by the activation controller or resource store at runtime. Schema/Provider fingerprints and other bundle integrity metadata reside in the private bundle file alongside the resource envelope array; they are not included in individual ResourceEnvelopes. For the bundle envelope integrity fields (`contentHash`, `artifactCatalogDigest`, `schemaVersion`, `providerSchemaDigests`) and private integrity fields see the Zone configuration lifecycle spec and `ADR-046-nix-configuration.md` Bundle contract (canonical) (D119); there is no separate manifest wrapper, `resourceCount`, or `resourceTypeCounts`.

### Nix declaration examples with canonical JSON

#### Host - system execution node

```nix
d2b.zones.dev.resources.host-system = {
  type = "Host";
  spec = {
    providerRef = "Provider/system-core";
    defaultDomain = "system";
    allowedDomains = [ "system" "user" ];
    defaultUserRef = "User/alice";
    budget = { cpu.request = "2000m"; cpu.limit = "8000m"; memory.request = "1Gi"; memory.limit = "16Gi"; };
    provider = {
      schemaId = "system-core.d2bus.org/Host/spec";
      schemaVersion = "1.0";
      settings.capabilities = [ "kvm" "pidfd" "cgroup-v2" "user-namespace" "wayland" "audio-pipewire" ];
    };
  };
};
```

```json
{
  "apiVersion": "resources.d2bus.org/v3",
  "metadata": {
    "annotations": {}, "labels": {}, "name": "host-system",
    "ownerRef": null, "zone": "dev"
  },
  "spec": {
    "allowedDomains": ["system", "user"],
    "budget": { "cpu": { "limit": "8000m", "request": "2000m" }, "memory": { "limit": "16Gi", "request": "1Gi" } },
    "defaultDomain": "system", "defaultUserRef": "User/alice",
    "deviceAttachments": [], "isolationPosture": null, "networkAttachments": [],
    "provider": { "schemaId": "system-core.d2bus.org/Host/spec", "schemaVersion": "1.0", "settings": { "capabilities": ["audio-pipewire", "cgroup-v2", "kvm", "pidfd", "user-namespace", "wayland"] } },
    "providerRef": "Provider/system-core"
  },
  "type": "Host"
}
```

#### Host - user-only (unsafe-local successor)

```nix
d2b.zones.dev.resources.host-unsafe-local = {
  type = "Host";
  spec = {
    providerRef = "Provider/system-core";
    defaultDomain = "user";
    allowedDomains = [ "user" ];
    defaultUserRef = "User/alice";
    isolationPosture = "none";
  };
};
```

```json
{
  "apiVersion": "resources.d2bus.org/v3",
  "metadata": { "annotations": {}, "labels": {}, "name": "host-unsafe-local", "ownerRef": null, "zone": "dev" },
  "spec": {
    "allowedDomains": ["user"], "budget": null, "defaultDomain": "user",
    "defaultUserRef": "User/alice", "deviceAttachments": [], "isolationPosture": "none",
    "networkAttachments": [], "provider": null,
    "providerRef": "Provider/system-core"
  },
  "type": "Host"
}
```

#### Guest - cloud-hypervisor VM

```nix
# Artifact catalog entry (separate from zone resources; owns the derivation)
d2b.artifacts.dev-vm-system = {
  package = nixosSystemForDevVm.config.system.build.toplevel;
  type = "nixos-system";
};

d2b.zones.dev.resources.dev-vm = {
  type = "Guest";
  spec = {
    providerRef = "Provider/runtime-cloud-hypervisor";
    defaultDomain = "system";
    allowedDomains = [ "system" "user" ];
    defaultUserRef = "User/alice";
    systemArtifactId = "dev-vm-system";   # top-level spec field; not in spec.provider.settings
    budget = { cpu.request = "1000m"; cpu.limit = "4000m"; memory.request = "512Mi"; memory.limit = "4Gi"; };
    networkAttachments = [ { networkRef = "Network/dev-lan"; default = true; } ];
    provider = {
      schemaId = "runtime-cloud-hypervisor.d2bus.org/Guest/spec";
      schemaVersion = "1.0";
      settings = { vcpus = 4; memoryMb = 4096; vsockCid = 3; machineType = "q35"; consoleType = "virtio"; };
    };
  };
};
```

Resource bundle JSON (`systemArtifactId` is a top-level spec field; no store path in envelope):

```json
{
  "apiVersion": "resources.d2bus.org/v3",
  "metadata": { "annotations": {}, "labels": {}, "name": "dev-vm", "ownerRef": null, "zone": "dev" },
  "spec": {
    "allowedDomains": ["system", "user"],
    "budget": { "cpu": { "limit": "4000m", "request": "1000m" }, "memory": { "limit": "4Gi", "request": "512Mi" } },
    "defaultDomain": "system", "defaultUserRef": "User/alice", "deviceAttachments": [],
    "networkAttachments": [{ "default": true, "networkRef": "Network/dev-lan" }],
    "provider": { "schemaId": "runtime-cloud-hypervisor.d2bus.org/Guest/spec", "schemaVersion": "1.0", "settings": { "consoleType": "virtio", "machineType": "q35", "memoryMb": 4096, "vcpus": 4, "vsockCid": 3 } },
    "providerRef": "Provider/runtime-cloud-hypervisor",
    "systemArtifactId": "dev-vm-system",
    "volumeAttachmentDefaults": []
  },
  "type": "Guest"
}
```

Private artifact catalog entry (emitted alongside resource bundle; store path private):

```json
{ "dev-vm-system": { "sha256": "a3c1...", "size": 12456789, "type": "nixos-system" } }
```

#### Process - Wayland proxy sidecar

`metadata.ownerRef` is author-supplied in the `metadata` block; it is NOT a `spec` field.

```nix
d2b.zones.dev.resources.wayland-proxy = {
  type = "Process";
  metadata.ownerRef = "Provider/display-wayland";
  spec = {
    providerRef = "Provider/system-systemd";
    executionRef = "Host/host-system";
    domain = "system";
    processClass = "service";
    template = "wayland-proxy-main";
    sandbox = { namespaceClasses = [ "mount" "ipc" "uts" "network" ]; capabilityClasses = []; seccompClass = "strict"; readOnlyRoot = true; noNewPrivileges = true; };
    budget = { cpu.limit = "500m"; memory.limit = "128Mi"; };
    restartPolicy = { class = "on-failure"; backoffBase = "1s"; backoffMax = "60s"; };
    readiness = { timeout = "30s"; class = "ready-condition"; };
  };
};
```

```json
{
  "apiVersion": "resources.d2bus.org/v3",
  "metadata": { "annotations": {}, "labels": {}, "name": "wayland-proxy", "ownerRef": "Provider/display-wayland", "zone": "dev" },
  "spec": {
    "budget": { "cpu": { "limit": "500m", "request": null }, "memory": { "limit": "128Mi", "request": null } },
    "domain": "system", "executionRef": "Host/host-system", "mounts": [],
    "processClass": "service",
    "providerRef": "Provider/system-systemd",
    "readiness": { "class": "ready-condition", "timeout": "30s" },
    "restartPolicy": { "backoffBase": "1s", "backoffMax": "60s", "class": "on-failure" },
    "sandbox": { "capabilityClasses": [], "namespaceClasses": ["ipc", "mount", "network", "uts"], "noNewPrivileges": true, "readOnlyRoot": true, "seccompClass": "strict" },
    "telemetry": null, "template": "wayland-proxy-main", "userRef": null
  },
  "type": "Process"
}
```

#### EphemeralProcess - swtpm state flush

```nix
d2b.zones.dev.resources."swtpm-flush-dev-vm" = {
  type = "EphemeralProcess";
  metadata.ownerRef = "Provider/device-tpm";
  spec = {
    providerRef = "Provider/system-minijail";
    executionRef = "Host/host-system";
    domain = "system";
    processClass = "worker";
    template = "swtpm-flush";
    mounts = [ { volumeRef = "Volume/dev-vm-tpm-state"; view = "flush"; mountPath = "/state"; access = "read-write"; } ];
    startDeadline = "30s";
    runtimeDeadline = "60s";
    successfulTtl = "1h";
    failedTtl = "24h";
  };
};
```

```json
{
  "apiVersion": "resources.d2bus.org/v3",
  "metadata": { "annotations": {}, "labels": {}, "name": "swtpm-flush-dev-vm", "ownerRef": "Provider/device-tpm", "zone": "dev" },
  "spec": {
    "domain": "system", "executionRef": "Host/host-system", "failedTtl": "24h",
    "mounts": [{ "access": "read-write", "mountPath": "/state", "view": "flush", "volumeRef": "Volume/dev-vm-tpm-state" }],
    "processClass": "worker",
    "providerRef": "Provider/system-minijail",
    "runtimeDeadline": "60s", "sandbox": null, "startDeadline": "30s",
    "successfulTtl": "1h", "template": "swtpm-flush", "userRef": null
  },
  "type": "EphemeralProcess"
}
```

#### User

```nix
d2b.zones.dev.resources.alice = {
  type = "User";
  spec = { osUsername = "alice"; displayName = "Alice"; groups = [ "wheel" "audio" "video" "d2b" ]; };
};

# metadata.name (alice-admin) and spec.osUsername (alice_admin) are independent
d2b.zones.dev.resources.alice-admin = {
  type = "User";
  spec = { osUsername = "alice_admin"; displayName = "Alice (admin)"; groups = [ "wheel" "d2b" ]; };
};
```

```json
{
  "apiVersion": "resources.d2bus.org/v3",
  "metadata": { "annotations": {}, "labels": {}, "name": "alice", "ownerRef": null, "zone": "dev" },
  "spec": { "displayName": "Alice", "groups": ["audio", "d2b", "video", "wheel"], "osUsername": "alice" },
  "type": "User"
}
```

### Configuration management authority

The activation controller sets `metadata.managedBy = "configuration"` on every resource it creates from a bundle envelope, and sets `metadata.configurationGeneration` to the current NixOS generation number. `metadata.managedBy` is immutable after first Create and is NOT author-settable. `metadata.configurationGeneration` is core-updatable: the activation controller refreshes it on every generation activation, including for unchanged specs. The generation cleanup controller identifies the config-declared population by querying `metadata.managedBy = "configuration"`; it does not use any author-supplied label or `configOwned` field. Controller-created resources (`metadata.managedBy = "controller"`) and API-created resources (`metadata.managedBy = "api"`) are never touched by generation cleanup. See the Zone configuration lifecycle spec for the full activation algorithm and cleanup controller design.

### Removed-resource deletion cascade

When a resource is absent from the new bundle and has `metadata.managedBy = "configuration"`, the activation controller issues an async Delete. The resource's `metadata.deletionRequestedAt` is set and a `CleanupPending=True` condition is added with `reason=config-generation-cleanup`. The resource remains in its current phase (Pending or Degraded) while finalizers drain. When all finalizers release, the resource store commits a single atomic transaction: a `Deleted` revision event is written and the resource row and all indexes are removed. Following the committed transaction, the audit subsystem appends the deletion audit record using a dedup/exactly-once recovery key so retried recoveries never produce a duplicate audit entry. There is no `phase=Deleting`; the valid terminal deletion phase is `Deleted`. Owner controllers must release their finalizers. Cascade Delete responsibility by ResourceType:

- **Guest deleted**: the Guest controller issues Delete on all resources with `metadata.ownerRef = "Guest/<name>"` (Processes and EphemeralProcesses). Processes drain gracefully; EphemeralProcesses are allowed to complete or killed per provider policy. All child finalizers released before the Guest finalizer.
- **Host deleted**: config-declared Processes with `spec.executionRef = "Host/<name>"` have concurrent Delete operations in-flight from the same generation diff. Controller-created Processes with that `executionRef` are issued Delete by their owner controllers when the Host's `metadata.deletionRequestedAt` is set.
- **User deleted**: Delete is blocked if any Process has `spec.userRef = "User/<name>"`; this is a structural conflict requiring the operator to update or delete those Processes first. No implicit cascade.
- **Process or EphemeralProcess deleted**: the Process Provider drains and stops the process per drainTimeout policy, then releases the pidfd and cgroup leaf.

The generation cleanup controller issues Delete only on top-level config-declared resources and does not descend into controller-created children.

### Per-resource cleanup conditions and error codes

Resources deleted by generation cleanup carry:

| Condition type | True when | reason codes |
| --- | --- | --- |
| `CleanupPending` | Delete has been issued; `metadata.deletionRequestedAt` is set; finalizers draining | `config-generation-cleanup` |
| `CleanupBlocked` | Deletion is blocked by a finalizer or owned children beyond threshold | `finalizer-blocked`, `owner-child-blocked` |

Stable error codes:

| Code | Description |
| --- | --- |
| `config-bundle-integrity-failed` | `contentHash` mismatch; entire bundle activation aborted |
| `config-catalog-mismatch` | `artifactCatalogDigest` anchor in the bundle envelope does not match installed `/etc/d2b/artifact-catalog.json`; entire bundle activation aborted |
| `config-schema-mismatch` | One or more resource type schemas in the bundle do not match the currently committed ResourceTypeSchema; entire bundle activation aborted |
| `provider-schema-mismatch` | One or more Provider schema versions in the bundle do not match the installed Provider; entire bundle activation aborted |
| `config-collision` | New config-declared Create collides with an existing controller-created or api-created resource of the same name; per-item error; other intents continue |
| `config-activation-failed` | Activation returned a terminal error not covered by the above codes |
| `cleanup-finalizer-blocked` | Resource deletion blocked by unreleased finalizer beyond threshold |
| `cleanup-owner-blocked` | Resource deletion blocked by owned children beyond threshold |
| `cleanup-timeout` | Cleanup did not complete within the configured deadline |

### Prior generation retention

The zone configuration controller retains the N most recently activated, cleanup-complete bundle records. Default N=3; operator-configurable in range 1..16. No time-based TTL. A bundle record is eligible for release when its cleanup is complete and the retained count exceeds N (oldest-beyond-N released first). Rollback procedure and bundle GC-root management are defined in the Zone configuration lifecycle spec.

### Tests for Nix configuration and ResourceType-specific lifecycle

**Eval/build tests** (run by `make test-flake` / `nix-unit`):

1. `type = "Host"` with all `spec` fields renders exact golden JSON vector; `metadata.ownerRef = null`; `metadata.annotations = {}`; `metadata.labels = {}`; `spec` keys sorted; no `configOwned`, no `managedBy`, no `configurationGeneration`, no `schemaFingerprint`, no `providerSchemaFingerprint` in ResourceEnvelope.
2. `type = "Host"`, `spec.isolationPosture = "none"`, `spec.allowedDomains = ["system"]` → eval error; rule 7 code.
3. `type = "Host"`, `spec.isolationPosture = "none"`, `spec.defaultUserRef = null` → eval error; rule 7 code.
4. `type = "Guest"` with `spec.providerRef = "Provider/system-core"` (substrate, not runtime) → eval error; rule 4 code.
5. `type = "Process"`, `spec.domain = "system"`, `spec.executionRef` resolves to Host with `spec.allowedDomains = ["user"]` → eval error; rule 5 code.
6. `type = "Process"`, `spec.domain = "user"`, no `spec.userRef`, no `executionRef.spec.defaultUserRef` → eval error; rule 6 code.
7. `type = "User"`, `spec.osUsername = "alice/admin"` (path separator) → eval error; rule 8 code.
8. `type = "User"`, `spec.osUsername = "alice_admin"`, resource key `alice-admin`: valid; JSON `metadata.name = "alice-admin"`, `spec.osUsername = "alice_admin"`.
9. `type = "User"`, `spec.groups = ["WHEEL"]` → eval error; rule 14 code.
10. `type = "Process"`, `metadata.ownerRef = "Guest/dev-vm"`: JSON has `metadata.ownerRef = "Guest/dev-vm"` and `spec` contains no `ownerRef` field.
11. `metadata.ownerRef` referencing a non-existent resource in the same Zone → eval error; rule 3 code.
12. `metadata.ownerRef = "Bogus/nonexistent"` (unknown ResourceType) → eval error; rule 3 code. Also: `metadata.ownerRef = "Process/self"` on the same resource → eval error; rule 11 (no-self-ref) code. Also: `metadata.ownerRef` referencing a declared resource in a different Zone → eval error; rule 3 cross-Zone rejection. Also: `metadata.ownerRef = "Host/unresolved"` where `Host/unresolved` is not declared in `d2b.zones.<zone>.resources` → eval error; rule 3 unresolved ref code.
13. `d2b.artifacts.dev-vm-system = { package = <drv>; type = "nixos-system"; }`, Guest resource `spec.systemArtifactId = "dev-vm-system"` (top-level spec field) → valid; artifact catalog emits `"dev-vm-system": { "sha256": "...", "size": ..., "type": "nixos-system" }`; ResourceEnvelope contains `"systemArtifactId": "dev-vm-system"` in `spec` (not in `spec.provider.settings`) and no store path anywhere in the envelope; rule 17 passes.
14. Guest resource `spec.systemArtifactId = "missing-system"` with no `d2b.artifacts.missing-system` entry → hard eval error; rule 17 code.
15. Guest resource `spec.systemArtifactId = "display-wayland-v1"` where `d2b.artifacts."display-wayland-v1".type = "provider"` → hard eval error; rule 17 code (type mismatch).
16. Duplicate `d2b.artifacts.<id>` key in the same `d2b.artifacts` attrset → Nix parse-time attrset key collision error before any eval rule runs.
17. `type = "Host"`, `spec.isolationPosture = "none"`, `spec.defaultDomain = "user"`, `spec.allowedDomains = ["user"]`, `spec.defaultUserRef = "User/alice"` → valid; rendered JSON has top-level `"isolationPosture": "none"` in `spec` (not under `provider`); positive test for the user-only no-isolation Host configuration.
18. `type = "Host"`, `spec.isolationPosture = null`, `spec.defaultDomain = "user"`, `spec.allowedDomains = ["user"]`, `spec.defaultUserRef = "User/alice"` → eval error; rule 7 code (bidirectional: null cannot be used to evade the no-isolation posture declaration).

**Runtime/integration tests** (run by `make test-integration`):

19. New generation adds a Host → Create issued; stored resource has `metadata.managedBy = "configuration"` and `metadata.configurationGeneration` matches current NixOS generation; Host reaches `phase=Ready`; no `configOwned` field on stored resource; ResourceEnvelope contains no `schemaFingerprint`, `providerSchemaFingerprint`, or store path fields.
20. New generation removes a User not referenced by any Process → async Delete issued; `metadata.deletionRequestedAt` set; `CleanupPending=True` condition added; no `phase=Deleting`; finalizers release; deletion transaction atomically commits `Deleted` revision event + row/index removal; audit append follows committed revision with dedup/exactly-once recovery (no duplicate emit on recovery retry); Zone cleanup state empties.
21. New generation removes a Guest owning Processes → Guest Delete issued; Guest controller cascades Delete to all resources with `metadata.ownerRef = "Guest/<name>"`; Processes stop; each Process deletion atomically commits `Deleted` revision event + row/index removal; audit append follows committed revision with dedup/exactly-once recovery; Guest finalizer released; Guest deletion atomically commits `Deleted` revision event + row/index removal; audit append follows; `Deleted` Watch events observed for all.
22. New generation removes a User referenced by an active Process → Delete blocked; `cleanup-owner-blocked` error; after operator updates Process, Delete completes.
23. Controller-created EphemeralProcess (`metadata.managedBy = "controller"`) absent from new bundle → NOT deleted by generation cleanup; still present after activation.
24. system-minijail spawn → the broker that called `clone3` alone performs
    `waitid(P_PIDFD)` and reaps exactly once; ProviderSupervisor poll
    readability cannot supply `lastExitClass`/`outcome.exitCode`; the controller
    receives the broker-relayed typed status; a verified duplicate holder can
    still `pidfd_send_signal` the exact main process.
25. system-minijail intentional stop with a descendant that calls `setsid(2)`
    and an unrelated recycled-PGID decoy → exact-main SIGTERM/grace is followed
    by writing `1` to the anchored leaf's `cgroup.kill`; the owned leaf reaches
    `populated 0`, the decoy is untouched, and rmdir/finalizer clearing occurs
    only after broker-reaped main status. Ambiguous adoption performs no signal
    or `cgroup.kill`.
26. Linux <5.14 or a delegated cgroup v2 leaf without writable `cgroup.kill` →
    Host/system-minijail placement remains not Ready with
    `kernel-too-old`/`cgroup-kill-unavailable`, and the broker receives zero
    spawn requests.

---

## Current-code fit

| Item | Treatment |
| --- | --- |
| Current anchor (Host) | `packages/d2b-core/src/host_check.rs`: `HostCheckReport`, `HostCheckSummary`, `HostCheckFinding`, `HostCheckSeverity`; `packages/d2bd/src/kernel_module_check.rs`, `pidfs_probe.rs`; `nixos-modules/options-host.nix`; `packages/d2b-core/src/provider_capabilities.rs`; `packages/d2b-realm-core/src/ids.rs`: `HostResourceId`, `NodeId`; `packages/d2b-realm-core/src/node.rs`: `NodeKind` (`FullHost`/`Gateway`/`ProviderManaged`), `NodeSummary` (id/realm/kind/capabilities) - `NodeKind::FullHost` is the closest current analog for a v3 Host execution node |
| Host evidence class | `HostCheckReport`/`HostCheckSummary` implemented-and-reachable; `NodeSummary`/`NodeKind::FullHost` implemented-and-reachable (current node inventory concept); Host ResourceType is ADR-only |
| Current anchor (Guest) | `packages/d2b-realm-core/src/workload.rs`: `WorkloadProviderKind` (`LocalVm` → runtime-cloud-hypervisor Provider, `QemuMedia` → runtime-qemu-media Provider, `ProviderManaged` → ACA/relay Providers; `UnsafeLocal` → user-only **Host**, not Guest - this variant is cited here only for enum completeness; its evidence and target mapping are owned by the unsafe-local anchor row), `IsolationPosture` (`VirtualMachine` → Guest, `ProviderManaged` → Guest; `UnsafeLocal` → Host `isolationPosture="none"`, not Guest), `WorkloadExecutionPosture`, `WorkloadSummary`, `WorkloadState` (`Stopped`/`Starting`/`Running`/`Degraded`/`Failed`), `WorkloadSelector`; `packages/d2b-realm-core/src/ids.rs`: `RealmId`, `WorkloadId`, `NodeId`, `ProviderId`, `ExecutionId`, `GatewayId`; `packages/d2b-realm-core/src/realm.rs`: `RealmPath`, `RealmControllerPlacement`, `EntrypointMode`; `packages/d2b-realm-core/src/target.rs`: `RealmTarget`, `TargetName`, `RealmTargetParser` (current analog for `<ResourceType>/<name>` ResourceRef); `packages/d2b-core/src/workload_identity.rs`: `WorkloadIdentity`, `WorkloadTarget`, `WorkloadBackend` (`LocalVm`/`LocalQemuMedia`/`ProviderManaged` → Guest; `UnsafeLocal` → Host - not a Guest binding, not a v3 Provider; see unsafe-local anchor), `WorkloadRuntimeIntent`; `packages/d2b-core/src/realm_controller_config.rs`: `RealmControllerConfig`, `RealmControllerRuntimeProviderType`, `RealmControllerLocalWorkload`; `nixos-modules/options-realms-workloads.nix` (`d2b.realms.<realm>.workloads.<name>.kind`: local-vm/qemu-media → Guest; unsafe-local → Host, see unsafe-local anchor); `nixos-modules/options-realms.nix` (`d2b.realms`, `providerKind` regex `^[a-z][a-z0-9-]*$`) |
| Guest evidence class | `WorkloadSummary`/`WorkloadState`/`WorkloadProviderKind` implemented-and-reachable for local-vm/qemu-media paths; ACA (`d2b-provider-aca/`) and relay (`d2b-provider-relay/`) Providers have live runtime paths via `AcaWorkloadProvider`/`WorkloadProvider` trait; host Provider adapters in `d2b-host-providers/src/lib.rs` (`HostCheckSubstrateProvider`, `LocalMicroVmProvider`) are thin adapters, not fully wired to the daemon (see `packages/d2bd/src/realm_stubs.rs` `dead_code` note); Guest ResourceType is ADR-only; v3 runtime Provider resources are ADR-only |
| Current anchor (Guest runtime Providers) | `packages/d2b-realm-provider/src/provider.rs`: `HostSubstrateProvider`, `RuntimeProvider`, `WorkloadProvider`, `DurableExecutionProvider`, `GuestControlEndpointProvider`, `PersistentShellProvider`, `DisplayProvider`, `RelayProvider`, `NodeProvider` (traits); `packages/d2b-realm-provider/src/capabilities.rs`: `RuntimeCapabilitySet`, `WorkloadCapabilitySet`, `DisplayCapabilitySet`, `NodeCapabilitySet`, `HostSubstrateKind`; `packages/d2b-host-providers/src/lib.rs`: `HostCheckSubstrateProvider` (NIXOS_HOST_SUBSTRATE_PROVIDER_ID, GENERIC_LINUX_HOST_SUBSTRATE_PROVIDER_ID), `LocalMicroVmProvider` (CLOUD_HYPERVISOR_RUNTIME_PROVIDER_ID), LOCAL_QEMU_MEDIA_RUNTIME_PROVIDER_ID, LOCAL_CROSS_DOMAIN_WAYLAND_PROVIDER_ID; `packages/d2b-provider-aca/src/lib.rs`: `AcaWorkloadProvider` (ACA sandbox path, live); `packages/d2b-provider-relay/src/lib.rs` (relay transport, live) |
| Guest runtime Provider evidence class | ACA (`AcaWorkloadProvider`) and relay Provider are implemented-and-reachable with live data-plane REST and relay paths; `LocalMicroVmProvider`/`HostCheckSubstrateProvider` are implemented-but-unwired (`realm_stubs.rs` stubs are `dead_code`; gateway-mode wiring is incomplete); all v3 Provider resources are ADR-only |
| Current anchor (Process/EphemeralProcess) | `packages/d2b-core/src/processes.rs`: `ProcessRole` (18 variants), `VmProcessDag`, `ProcessNode`, `RoleProfile`, `NamespaceSet`, `MountPolicy`, `CgroupPlacement`, `ReadinessPredicate`; `packages/d2bd/src/supervisor/`: `DagExecutor`, `NodeOutcome`, `NodeHistory`, `NodeBudget`, `SplitReadinessMode`; `packages/d2bd/src/supervisor/pidfd_table.rs`: `PidfdTable`, `PidfdEntry`, `PidfdRegistration`, `WaitTermination`, `BrokerReapLog`; `packages/d2b-priv-broker/` SpawnRunner; `packages/d2b-guestd/src/exec.rs`, `exec_linux.rs`, `exec_pty.rs`, `detached.rs`; `packages/d2b-realm-core/src/execution.rs`: `ExecState`, `ExecAttachMode`, `ExecStartRequest`, `ExecAttachRequest` (current exec lifecycle analog for EphemeralProcess); `packages/d2b-realm-core/src/workload.rs`: `WorkloadProviderKind::UnsafeLocal` → user-only Host `isolationPosture="none"`, `IsolationPosture::UnsafeLocal` → Host `isolationPosture="none"` (these current enum variants are evidence for the no-isolation posture concept; they are retained in current evidence citations but not carried forward as target naming); `packages/d2b-core/src/workload_identity.rs`: `WorkloadBackend::UnsafeLocal` → Host `isolationPosture="none"` (same evidence classification) |
| Process evidence class | `ProcessRole`/`VmProcessDag`/`ProcessNode`/`RoleProfile`/`PidfdTable` implemented-and-reachable; `ExecState`/`ExecStartRequest` implemented-and-reachable for guest exec; Process/EphemeralProcess ResourceTypes are ADR-only |
| Current anchor (unsafe-local) | `packages/d2b-unsafe-local-helper/src/`: `lib.rs`, `protocol.rs` (`HelperClient`), `runtime.rs` (`ScopeRuntime`, `SupervisorSpec`), `systemd.rs`; `packages/d2bd/src/unsafe_local_helper.rs`; `packages/d2b-realm-core/src/workload.rs`: `WorkloadProviderKind::UnsafeLocal`, `IsolationPosture::UnsafeLocal`; `packages/d2b-core/src/workload_identity.rs`: `WorkloadBackend::UnsafeLocal`; `nixos-modules/options-realms-workloads.nix` (`d2b.realms.<realm>.workloads.<name>.kind = "unsafe-local"`) |
| unsafe-local evidence class | implemented-and-reachable (`HelperClient`/`ScopeRuntime`/`WorkloadProviderKind::UnsafeLocal` are all live); v3 user-only Host (`isolationPosture="none"`) is ADR-only |
| Current anchor (guestd) | `packages/d2b-guestd/src/`: `auth.rs`, `exec.rs` (`ExecPolicy`, `ExecState`, `ExecError`, `ExitOutcome`, `SpawnedProcess`, `RingChunk`), `exec_linux.rs`, `exec_pty.rs`, `detached.rs` (`ManagedUnit`, `UnitError`, `UnitIdentity`), `detached_registry.rs`, `service.rs`, `shell.rs` (`ShellRuntime`, `ShellRuntimeConfig`), `login_session.rs`; `packages/d2b-realm-core/src/execution.rs`: `ExecState`, `ExecAttachMode`, `ExecStartRequest` (guestd uses these DTOs in the vsock/ttrpc protocol) |
| guestd evidence class | implemented-and-reachable for guest exec; v3 EphemeralProcess/Process/ComponentSession is ADR-only |
| Current anchor (userd) | `packages/d2b-userd/src/lib.rs`: `UserdConfig`, `UserSessionIdentity`, `UserAttachRequest`, `UserdError`, `UserdTransport`, `UserExecSession` trait |
| userd evidence class | implemented-but-partially-wired; v3 User/Process domain is ADR-only |
| Current anchor (User) | `packages/d2b-realm-core/src/ids.rs`: `PrincipalId`, `RealmId`, `NodeId`, `WorkloadId` (identity foundation for Zone principal model); `packages/d2b-realm-core/src/access.rs`: `HostLocalPeerCredentialSemantics`, `HostLocalPeerCredentialSource`, `RealmAccessClientContract` (current host-local UID/credential resolution); `packages/d2bd/src/admission.rs` (`SO_PEERCRED` peer-uid); `nixos-modules/` host user configuration |
| User evidence class | `PrincipalId`/`HostLocalPeerCredentialSemantics` implemented-and-reachable for `SO_PEERCRED` authorization; OS UID/group NSS lookup reachable in broker/daemon; User ResourceType is ADR-only |
| Behavior retained | Fine-grained namespace/capability/seccomp/mount sandbox policy; pidfd identity/adoption; direct cgroup placement; systemd transient units; user-manager scope; typed readiness predicates; fail-closed identity checks; bounded audit/status messages; no-new-privileges enforcement; virtiofsd user namespace (ADR 0021) |
| Required delta | Host/Guest ResourceTypes; common Process/EphemeralProcess contract; ExecutionPolicy; User ResourceType; system-core reconcilers; Process Provider controllers; owner-child graph for Guests; TTL cleanup controller |
| Reuse path | Extract from exact sources named in work items below |
| Replacement/deletion | No ProcessRole/processes.json/workload DTO/unsafe-local-helper/guestd/userd path removed until successor ResourceType is integrated, all tests pass, and the exact disposition row is satisfied |
| Feasibility proof | system-core Host + User reconcile; system-systemd/system-minijail Process lifecycle; Guest with owned VMM Process; EphemeralProcess TTL; <=5 ms/<=20 ms fast path benchmarks |

---

## Implementation work items

### Provider crate standard layout

Every `packages/d2b-provider-<base>-<implementation>/` crate introduced by any work item in this spec or any downstream spec must satisfy the following directory and file layout. Absence of any required entry fails the workspace package-policy check (`make test-policy` / `cargo xtask delivery wave`).

| Path | Required | Contents |
| --- | --- | --- |
| `src/` | yes | Implementation source, binary entry points (`main.rs` / `bin/`), and colocated unit tests (`#[cfg(test)]` modules). All provider controller, reconcile, service, and worker logic lives here. |
| `tests/` | yes | Hermetic Cargo integration tests with no external services or live sockets: ResourceType lifecycle (create/update/delete/watch), controller reconcile/finalize state machines, conformance gate (all shared process conformance cases must pass), and fault injection (crash/restart/timeout/overload). Tests in this directory are run by bare `cargo test -p <crate>`. |
| `integration/` | yes | Heavier fixtures and scenarios that require containers, live Hosts or Guests, cross-process IPC, or real Provider subsystems. These are invoked by the existing test orchestration (`make test-integration`, `make test-host-integration`) and are not run by bare `cargo test`. Each scenario must include a `README.md` describing its prerequisites and invocation. |
| `README.md` | yes | Provider identity and descriptor (ID, version, supported ResourceTypes); `spec.provider.settings` config schema with all fields, types, defaults, and validation rules; ResourceTypes reconciled by this Provider and their lifecycle guarantees; controllers, services, workers, and binaries with their roles and cgroup/principal placement; Host/Guest placement rules and restrictions; dependencies, RBAC verbs, and Credential requirements; security posture, state ownership model, and canonical telemetry labels; `cargo build`, `cargo test`, and integration test invocation commands; future standalone-repo usage notes. |

A work item whose `Destination` row introduces a new `d2b-provider-*` crate must list all four required paths in that row and must include README content acceptance criteria in its `Validation` row. Work items that extend an existing crate (adding new source files to `src/` or new tests to `tests/`) inherit the layout from the introducing work item and need not repeat it.

### ADR046-exec-001

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-exec-001` |
| Dependency/owner | W0 shared contract root; `d2b-contracts` |
| Current source | `packages/d2b-core/src/processes.rs`: `ProcessRole` (18 variants), `ProcessNode`, `RoleProfile`, `NamespaceSet`, `MountPolicy`, `CgroupPlacement`, `ReadinessPredicate`; `packages/d2b-core/src/minijail_profile.rs`: `MinijailProfile`, `UserNamespaceProfile`, `NamespaceSet`, `MountPolicy`, `BindMount`, `CgroupPlacement`; `packages/d2b-core/src/storage.rs`: `StoragePathSpec`, `AclGrant`, `CleanupPolicy`, `RepairPolicy`; `packages/d2b-realm-core/src/ids.rs`: `RealmId`, `WorkloadId` (→ GuestRef), `NodeId` (→ HostRef), `ProviderId` (→ Provider ResourceRef), `ExecutionId` (→ EphemeralProcess exec identity), `PrincipalId` (→ User ResourceRef), `AllocatorLeaseId`, `ControllerGenerationId`; `packages/d2b-realm-core/src/workload.rs`: `WorkloadProviderKind` (`LocalVm`→runtime-cloud-hypervisor Provider, `QemuMedia`→runtime-qemu-media Provider, `ProviderManaged`→ACA/relay Providers, `UnsafeLocal`→user-only Host `isolationPosture="none"`), `IsolationPosture` (`VirtualMachine`→Guest, `ProviderManaged`→Guest, `UnsafeLocal`→Host `isolationPosture="none"`), `WorkloadExecutionPosture`, `WorkloadSummary`, `WorkloadState`; `packages/d2b-realm-core/src/target.rs`: `RealmTarget`, `TargetName`, `RealmTargetParser` (current analog for `<ResourceType>/<name>` ResourceRef parsing); `packages/d2b-realm-core/src/realm.rs`: `RealmPath`, `RealmControllerPlacement`, `EntrypointMode` (current Zone hierarchy analog); `packages/d2b-core/src/workload_identity.rs`: `WorkloadIdentity`, `WorkloadTarget` (= `RealmTarget`), `WorkloadBackend`, `WorkloadRuntimeIntent` (identity/backend separation reuse model for Host/Guest ResourceType split) |
| Reuse source | `packages/d2b-contracts/src/v3/` as destination; no equivalent main source for Host/Guest/Process ResourceType contracts |
| Reuse action | adapt |
| Destination | `packages/d2b-contracts/src/v3/host.rs`, `packages/d2b-contracts/src/v3/guest.rs`, `packages/d2b-contracts/src/v3/execution_policy.rs`, `packages/d2b-contracts/src/v3/process.rs`, `packages/d2b-contracts/src/v3/ephemeral_process.rs`, `packages/d2b-contracts/src/v3/user.rs`, `packages/d2b-contracts/src/v3/endpoint.rs` |
| Detailed design | Implement strict typed Rust structs for HostSpec, GuestSpec, ExecutionPolicy, ExecutionSpec, SandboxSpec, BudgetSpec, MountSpec, NetworkUsageSpec, DeviceUsageSpec, EndpointSpec, EndpointConsumerPolicy, TelemetrySpec, ProcessSpec, EphemeralProcessSpec, UserSpec; EndpointSpec accepts exactly `owner\|provider\|zone`, and EndpointConsumerPolicy owns the only finer gates (`allowedSubjects`, `allowedProviderComponents`, `allowedOperations`) with no schema aliases; strict serde deny_unknown_fields; bounds/redaction on all string fields; stable error types; `UserSpec.osUsername` validated as OS username (1..255 bytes, no NUL/control/path-separator), not ResourceName grammar Primary reuse disposition: `adapt`. Preserved source-plan detail: extract and adapt. |
| Integration | Provider dossiers, controller descriptors, Zone resource API, Nix resource compiler |
| Data migration | Full reset; no v2 resource import |
| Validation | Golden JSON vectors for each ResourceType; Endpoint vectors accept only `owner\|provider\|zone`, reject every legacy/private visibility alias, reject scalar/array `consumerPolicy` aliases, and cover each canonical consumer allowlist; a docs drift test parses every `type: Endpoint` YAML/Nix example and fails unless visibility is canonical and finer gates occur only under `consumerPolicy`; serde unknown-field rejection; bounds enforcement; `UserSpec.osUsername` OS-username validation (underscore allowed, NUL rejected) |
| Removal proof | Old DTO types removed only after owning Resource/Provider integrations are live |

### ADR046-exec-002

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-exec-002` |
| Dependency/owner | ADR046-exec-001; `d2b-contracts` |
| Current source | `packages/d2b-core/src/processes.rs`: `ReadinessPredicate`, `VmProcessInvariants`; `packages/d2b-core/src/processes.rs`: `SpawnRunnerPlanOp`; `packages/d2b-priv-broker/src/ops/` SpawnRunner |
| Reuse action | adapt |
| Destination | `packages/d2b-contracts/src/v3/process_provider.rs`: LaunchTicket, ProcessIdentityDigest, AdoptionCandidate, PidfdEvidence, WaitReapOwner, BrokerTerminalResult, ProcessOutcome, ExitClass |
| Detailed design | LaunchTicket (Process/EphemeralProcess ref/UID/revision/generation, owner Provider/component/template, executionRef/domain/userRef, providerRef, compiled sandbox/budget/mount/device/network/endpoint digest, inherited FD table, operation/deadline/cancellation, expected identity/readiness); ProcessIdentityDigest (opaque bounded hex string); AdoptionCandidate (cgroup leaf path relative to controller root, start-time token, executable hash); BrokerTerminalResult binds process identity/operation to the clone3 parent's exact-once wait/reap status and cannot be constructed from pidfd readability; all types zeroizing where credential-adjacent Primary reuse disposition: `adapt`. Preserved source-plan detail: extract and adapt. |
| Integration | ProviderSupervisor adapter; system-systemd and system-minijail Process Providers |
| Data migration | Full reset |
| Validation | Golden LaunchTicket and BrokerTerminalResult vectors; field redaction test; digest-binding test; duplicate/mismatched/non-parent terminal relay rejection |
| Removal proof | None - net-new types; no prior owner to remove |

### ADR046-exec-003

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-exec-003` |
| Dependency/owner | ADR046-exec-001; system-core Provider owner |
| Current source | `packages/d2b-core/src/host_check.rs`: `HostCheckReport`, `HostCheckSummary`, `HostCheckFinding`, `HostCheckSeverity`; `packages/d2bd/src/pidfs_probe.rs`; `packages/d2bd/src/kernel_module_check.rs`; `packages/d2b-core/src/provider_capabilities.rs`; `packages/d2b-realm-core/src/ids.rs`: `HostResourceId` (current host-identity handle), `NodeId` (execution node identity); `packages/d2b-realm-core/src/node.rs`: `NodeKind::FullHost`, `NodeSummary` (host node's capability advertisement - direct reuse model for Host status `capabilities[]`) |
| Reuse action | adapt |
| Destination | `packages/d2b-provider-system-core/src/host.rs`: HostReconciler; status/conditions/capability probe implementation; `packages/d2b-provider-system-core/tests/`: hermetic reconcile/conformance/fault tests; `packages/d2b-provider-system-core/integration/`: Host probe and lifecycle integration scenarios; `packages/d2b-provider-system-core/README.md`: Provider identity, `spec.provider.settings` schema, ResourceTypes, placement, RBAC, security posture, telemetry labels, build/test commands (provider crate standard layout - see §Provider crate standard layout) |
| Detailed design | Async Host reconcile loop per this spec's Reconcile section; HostCapabilityClass probe set (kvm/pidfd/cgroup-v2/user-namespace/wayland/audio-pipewire/gpu-render/tpm2/usbip); bounded OS probes with timeout; mandatory system-minijail placement gate for Linux ≥5.14 plus writable delegated-leaf `cgroup.kill` independent of optional `kernelVersionMin`; `isolationPosture` validation and status; aggregate budget reservation tracking via List; status write with expected revision Primary reuse disposition: `adapt`. Preserved source-plan detail: extract and adapt. |
| Integration | Provider/system-core fixed bootstrap process; ResourceClient Get/List/UpdateStatus |
| Data migration | New Host resources from Nix; no v2 state import |
| Validation | Multiple Hosts per Zone; system-only and user-only Hosts; capability probe mocks; Linux <5.14 and missing/unwritable `cgroup.kill` reject system-minijail before spawn; Linux ≥5.14 positive probe; `isolationPosture="none"` rejection of system processes; budget overcommit rejection; `tests/` all pass under `cargo test`; `integration/` scenario passes in container fixture; `README.md` present and covers all required sections (provider crate standard layout acceptance) |
| Removal proof | Current host capability checks in `d2bd` removed after Host reconcile parity |

### ADR046-exec-004

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-exec-004` |
| Dependency/owner | ADR046-exec-001; system-core Provider owner |
| Current source | `packages/d2b-userd/src/lib.rs`: `UserdConfig`, `UserSessionIdentity`; `packages/d2bd/src/admission.rs` (`SO_PEERCRED` UID/GID lookup); `packages/d2b-realm-core/src/ids.rs`: `PrincipalId` (current host-local principal identity); `packages/d2b-realm-core/src/access.rs`: `HostLocalPeerCredentialSemantics`, `HostLocalPeerCredentialSource`, `RealmAccessClientContract` (current host-local credential/uid resolution model; direct reuse precedent for v3 User → Process userRef resolution) |
| Reuse action | adapt |
| Destination | `packages/d2b-provider-system-core/src/user.rs`: UserReconciler; NSS lookup implementation |
| Detailed design | Async User reconcile loop per this spec's Reconcile section; NSS `getpwnam(spec.osUsername)` with bounded timeout (default 5 s); `spec.osUsername` validated on admission (1..255 bytes, no NUL/control/path-separator); `metadata.name` used only as Zone-local resource identity and `User/<name>` ref; uid/gid/home/shell/group discovery written to status; session manager availability check via fixed user supervisor; status write; phase Degraded on SessionManagerReady=False Primary reuse disposition: `adapt`. Preserved source-plan detail: extract and adapt. |
| Integration | Provider/system-core fixed bootstrap process; ResourceClient Get/List/UpdateStatus; other controllers resolve User status via ResourceClient |
| Data migration | New User resources from Nix; no v2 state import |
| Validation | User found/not-found; group membership; sessionManagerAvailable; multiple Users; deletion blocked by Process userRef; `spec.osUsername` with underscore succeeds NSS lookup where ResourceName grammar would reject it; `metadata.name` and `spec.osUsername` differ (e.g. `alice-admin` / `alice_admin`); `spec.osUsername` containing NUL/control/path-separator rejected at admission |
| Removal proof | Current local uid/group lookup in `d2bd/src/admission.rs` removed after User resource parity |

### ADR046-exec-005

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-exec-005` |
| Dependency/owner | ADR046-exec-001 + ADR046-exec-004; system-core Provider owner |
| Current source | `packages/d2bd/src/lib.rs` (daemon startup); `packages/d2b-core/src/host_check.rs`; `nixos-modules/host.nix` (host activation); `nixos-modules/options-host.nix` (host options) |
| Reuse action | adapt |
| Destination | `packages/d2b-provider-system-core/src/host.rs` (continued); bootstrap startup sequence |
| Detailed design | ProviderSystem-core fixed bootstrap: start with embedded Zone runtime and compiled bootstrap authorization; run Host reconcile before any Process Provider launches; create initial User resources from Nix; publish initial Role/RoleBinding from Nix config; hand off to stored RBAC |
| Integration | Zone runtime startup sequence; system-minijail bootstrap process launch |
| Data migration | Full v3 reset |
| Validation | Bootstrap without prior Host; User-before-Process ordering; compilation bootstrap authorization closed-set tests |
| Removal proof | Current d2bd initialization sequence removed after bootstrap parity |

### ADR046-exec-006

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-exec-006` |
| Dependency/owner | ADR046-exec-001 + ADR046-exec-002; system-systemd Process Provider owner |
| Current source | `packages/d2b-unsafe-local-helper/src/systemd.rs`; `packages/d2b-guestd/src/exec.rs`: `SystemdRunUnitManager`, `ManagedUnit`, `ExecPolicy`; `packages/d2b-guestd/src/exec_linux.rs`; `packages/d2bd/src/supervisor/` (transient unit management) |
| Reuse action | adapt |
| Destination | `packages/d2b-provider-system-systemd/src/`: launch.rs, adoption.rs, pidfd.rs, wait.rs, user_supervisor.rs; `packages/d2b-provider-system-systemd/tests/`: hermetic lifecycle/conformance/fault tests; `packages/d2b-provider-system-systemd/integration/`: transient-unit and user-supervisor integration scenarios; `packages/d2b-provider-system-systemd/README.md`: Provider identity, `spec.provider.settings` schema, ResourceTypes, placement, RBAC, security posture, telemetry labels, build/test commands (provider crate standard layout - see §Provider crate standard layout) |
| Detailed design | system-systemd Process/EphemeralProcess provider conformance per this spec's system-systemd conformance section; transient system unit (Type=exec); InvocationID+cgroup+MainPID+start-time binding before pidfd_open; systemd-owned wait/reap; user domain via fixed user supervisor; adoption re-verification; sandboxSpec compilation to systemd hardening directives; runtimeDeadline enforcement; drainTimeout SIGTERM/SIGKILL sequence Primary reuse disposition: `adapt`. Preserved source-plan detail: extract and adapt. |
| Integration | Zone-installed Provider/system-systemd; ProviderSupervisor LaunchTicket interface; ResourceClient UpdateStatus |
| Data migration | Current ProcessRole/systemd unit roles converted by ProcessRole disposition table after parity |
| Validation | Shared process conformance test matrix (lifecycle/readiness/crash/drain/adoption/user-domain/sandboxSpec/pidfd); system-specific InvocationID binding; no-static-unit test; `tests/` all pass under `cargo test`; `integration/` scenario passes in container fixture; `README.md` present and covers all required sections (provider crate standard layout acceptance) |
| Removal proof | ProcessRole roles using systemd (Audio, WaylandProxy, VsockRelay, etc.) removed per disposition table after system-systemd parity |

### ADR046-exec-007

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-exec-007` |
| Dependency/owner | ADR046-exec-001 + ADR046-exec-002; system-minijail Process Provider owner |
| Current source | `packages/d2b-core/src/processes.rs`: `ProcessNode`, `RoleProfile`, `NamespaceSet`, `MountPolicy`, `CgroupPlacement`; `packages/d2b-core/src/minijail_profile.rs`: full; `packages/d2b-priv-broker/src/ops/spawn_runner.rs` (if present at baseline); `packages/d2bd/src/supervisor/` pidfd/wait; `packages/d2b-core/src/process_builder.rs` |
| Reuse action | adapt |
| Destination | `packages/d2b-provider-system-minijail/src/`: sandbox_compiler.rs, launch.rs, adoption.rs, pidfd.rs, wait.rs, user_ns.rs; `packages/d2b-provider-system-minijail/tests/`: hermetic sandbox-compilation/lifecycle/conformance/fault tests; `packages/d2b-provider-system-minijail/integration/`: clone3/user-namespace and broker-spawn integration scenarios; `packages/d2b-provider-system-minijail/README.md`: Provider identity, `spec.provider.settings` schema, ResourceTypes, placement, RBAC, security posture (capabilities, namespaces, seccomp), telemetry labels, build/test commands (provider crate standard layout - see §Provider crate standard layout) |
| Detailed design | system-minijail Process/EphemeralProcess provider conformance per this spec's system-minijail conformance section; SandboxSpec-to-minijail plan compilation; Linux ≥5.14/cgroup.kill placement gate; broker clone3(CLONE_PIDFD|CLONE_INTO_CGROUP) parent retains sole waitid(P_PIDFD)/reap/exit-status ownership and relays a typed terminal result; ProviderSupervisor polls a verified duplicate and retains exact-main pidfd_send_signal semantics but never waits/reaps; adoption verifies original broker parent; runtimeDeadline/drainTimeout use graceful main signal then anchored leaf cgroup.kill, broker wait/reap, and empty-leaf proof; no PID/PGID fallback; EphemeralProcess one-shot launch Primary reuse disposition: `adapt`. Preserved source-plan detail: extract and adapt. |
| Integration | Zone-installed Provider/system-minijail fixed bootstrap process; ProviderSupervisor LaunchTicket; privileged broker effect adapter |
| Data migration | Current RoleProfile/NamespaceSet/MountPolicy/CgroupPlacement adapted to SandboxSpec/BudgetSpec |
| Validation | Shared process conformance test matrix; minijail-specific sandbox compilation tests; user namespace tests; clone3 parent-only wait/reap and broker-relay tests; poll-readability-not-status test; pidfd_send_signal duplicate-holder test; setsid descendant/recycled-PGID cgroup.kill teardown test; Linux 5.14/cgroup.kill platform-gate tests; adoption quarantine asserts no signal/cgroup.kill; `tests/` all pass under `cargo test`; `integration/` scenario passes; `README.md` present and covers all required sections (provider crate standard layout acceptance) |
| Removal proof | ProcessRole roles using minijail (Virtiofsd, Swtpm, SecurityKeyFrontend, etc.) removed per disposition table after system-minijail parity |

### ADR046-exec-008

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-exec-008` |
| Dependency/owner | ADR046-exec-006 + ADR046-exec-007; conformance test owner |
| Current source | `packages/d2b-core/src/processes.rs` test coverage; `packages/d2bd/src/supervisor/` tests; minijail/seccomp test vectors |
| Reuse action | adapt |
| Destination | `packages/d2b-process-conformance/src/`: shared conformance test matrix run against both system-systemd and system-minijail providers |
| Detailed design | Full process lifecycle tests: start/ready/crash/restart/maxRestarts/drain/stop/delete; EphemeralProcess start/succeed/fail/ttl/cleanup; adoption after controller restart (fresh/quarantine); sandboxSpec compilation contract; pidfd rules (never-serialized/never-exported/re-verified-after-supervisor-restart; clone3 broker parent alone waits/reaps; non-parent readability is not status; verified duplicate holder may pidfd_send_signal exact main); system-minijail intentional teardown uses anchored cgroup.kill against setsid descendant and recycled-PGID fixtures, with no kill on ambiguity; Linux ≥5.14/cgroup.kill platform gate; user domain (system-systemd only); desiredLifecycle=stopped; fast path latency gate (<=5ms/<=20ms p95); 1/10/100 concurrent Process start |
| Integration | system-systemd and system-minijail providers must both pass all shared tests |
| Data migration | Full d2b 3.0 reset; no v2 state/config import |
| Validation | Hard pass/fail per-test; latency gates enforced; no exception for partial conformance |
| Removal proof | None - permanent conformance tests; no prior owner to remove |

### ADR046-exec-009

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-exec-009` |
| Dependency/owner | ADR046-exec-001; unsafe-local migration owner |
| Current source | `packages/d2b-unsafe-local-helper/src/lib.rs`: `UserdConfig`, protocol traits; `packages/d2b-unsafe-local-helper/src/runtime.rs` (`ScopeRuntime`, `SupervisorSpec`); `packages/d2b-unsafe-local-helper/src/systemd.rs`; `packages/d2bd/src/unsafe_local_helper.rs`; `packages/d2b-realm-core/src/workload.rs`: `WorkloadProviderKind::UnsafeLocal`, `IsolationPosture::UnsafeLocal` (current evidence that the no-isolation posture exists and is classified separately from VM isolation - the exact semantics this spec's `Host.spec.isolationPosture="none"` preserves); `packages/d2b-core/src/workload_identity.rs`: `WorkloadBackend::UnsafeLocal`; `nixos-modules/options-realms-workloads.nix` (`d2b.realms.<realm>.workloads.<name>.kind = "unsafe-local"`) |
| Reuse action | adapt |
| Destination | `packages/d2b-provider-system-core/src/host.rs` (user-only no-isolation Host); `nixos-modules/options-zones.nix` (Nix unsafe-local Host declaration) |
| Detailed design | v3 unsafe-local migration: `kind = "unsafe-local"` in the current Nix Realm workload model becomes a Host resource with `providerRef: Provider/system-core`, `spec.isolationPosture: "none"`, `defaultDomain: user`, `allowedDomains: [user]`, `defaultUserRef: User/<name>`. This is a Host ResourceType, not a Guest and not a v3 Provider. Child Process and EphemeralProcess resources on this Host use the normal Process Providers (Provider/system-systemd for user-domain transient user scope; Provider/system-minijail is also valid for callers that explicitly request namespace isolation within the user session). No special unsafe-local-specific Provider is introduced. The explicit no-isolation posture and its warnings are preserved: Host status reflects `isolationPosture="none"` and this is surfaced in every operator CLI/UI view as an explicit "no isolation boundary" warning; `ProcessEffect` audit records (launch, stop, adopt, quarantine) for child Processes and EphemeralProcesses carry the stable `no_isolation=true` attribute; operator CLI/UI always shows the warning and may not suppress it. The `no_isolation=true` attribute belongs on ProcessEffect records only - it must NOT appear on OTEL metric labels, span attributes, log fields, or audit records for other event kinds. The legacy helper protocol (`d2b-unsafe-local-helper`) is not exposed as a v3 ComponentSession service. |
| Integration | Host resource reconcile; User resource; system-systemd user-domain Process launch |
| Data migration | Full reset; no unsafe-local session state migration |
| Validation | User-only no-isolation Host rejected for system processes; `isolationPosture="none"` Host rejected for `allowedDomains` containing `system`; `allowedDomains=["user"]`+`defaultDomain=user`+`defaultUserRef` set with `isolationPosture=null` rejected at eval time (bidirectional evasion test); posture warning visible in CLI/UI status; `no_isolation=true` attribute present on ProcessEffect launch/stop/adopt/quarantine audit records for child Processes/EphemeralProcesses; `no_isolation=true` absent from OTEL span attributes, metric labels, log fields, and non-ProcessEffect audit records; user-domain Process under user-only Host starts correctly with normal Process Provider |
| Removal proof | `d2b-unsafe-local-helper` helper binary and protocol removed after user-only Host + shell-terminal Provider parity; `options-realms-workloads.nix` unsafe-local kind removed in Nix reset |

### ADR046-exec-010

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-exec-010` |
| Dependency/owner | ADR046-exec-001 + ADR046-exec-007; guestd migration owner |
| Current source | `packages/d2b-guestd/src/exec.rs`: `ExecPolicy`, `ExecError`, `ExecState`, `ExitOutcome`, `ValidatedCommand`, `SpawnedProcess`, `RingChunk`, `ExecSnapshot`, `ExecCreateInput`; `packages/d2b-guestd/src/exec_linux.rs`; `packages/d2b-guestd/src/exec_pty.rs`; `packages/d2b-guestd/src/detached.rs`: `ManagedUnit`, `UnitError`, `UnitIdentity`; `packages/d2b-guestd/src/detached_registry.rs`; `packages/d2b-guestd/src/service.rs`; `packages/d2b-guestd/src/auth.rs`; `packages/d2b-guestd/src/login_session.rs` |
| Reuse action | adapt |
| Destination | `packages/d2b-provider-system-systemd/src/guest_exec.rs` (guest-domain EphemeralProcess launch via systemd-run inside guest); `packages/d2b-session/` (ComponentSession replacing ad-hoc guest ttrpc); runtime Provider guest bootstrap Process |
| Detailed design | Guest-side execution transitions to EphemeralProcess/Process resources under the owning Guest; guestd service becomes a fixed bootstrap Process owned by the runtime Provider controller; exec operations become EphemeralProcess creates via the Zone ResourceClient; detached sessions become EphemeralProcess with failedTtl=24h; shell sessions become shell-terminal Provider Processes; guestd auth.rs becomes ComponentSession via d2b-session (copy/adapt from ADR046-session-001 per ComponentSession spec) Primary reuse disposition: `adapt`. Preserved source-plan detail: extract and adapt. |
| Integration | runtime-cloud-hypervisor Guest controller creates guest-bootstrap Process; Zone ResourceClient in guest-bootstrap creates EphemeralProcess/Process on behalf of guest workloads |
| Data migration | Full reset; no guestd session state migration |
| Validation | Guest EphemeralProcess exec lifecycle; detached exec TTL; guestd auth replaced by ComponentSession; no SSH fallback |
| Removal proof | `d2b-guestd` binary removed after all guest-side behaviors have Process/EphemeralProcess/ComponentSession successors and tests pass |

### ADR046-exec-011

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-exec-011` |
| Dependency/owner | ADR046-exec-004 + ADR046-exec-010; userd migration owner |
| Current source | `packages/d2b-userd/src/lib.rs`: `UserdConfig`, `UserSessionIdentity`, `UserAttachRequest`, `UserOutputCursor`, `UserExecSession` trait, `UserdError`, `UserdTransport`, `UnixSocketOnly`, `validate_attach_request`; `packages/d2b-userd/src/main.rs` |
| Reuse action | adapt |
| Destination | guest-domain process attachment becomes a ComponentSession named stream to the EphemeralProcess running in the guest; `UserExecSession` trait reimplemented as a typed ResourceClient+ComponentSession attachment |
| Detailed design | `UserdConfig.socket_name` replaced by Zone-local EphemeralProcess ResourceRef and ComponentSession attach verb; `UserSessionIdentity.uid`/`gid` become Process `userRef` resolved against User resource status; `UserAttachRequest.exec_id`/`tty`/`initial_size` become ComponentSession method parameters on the EphemeralProcess attachment service; `UserdError` becomes stable v3 error codes; `validate_attach_request` tty/size validation retained in the ComponentSession session schema Primary reuse disposition: `adapt`. Preserved source-plan detail: adapt (logic retained; protocol replaced). |
| Integration | CLI/client uses ResourceClient to Get/Watch EphemeralProcess; attaches via d2b-bus ComponentSession attach verb |
| Data migration | Full reset; no session state import |
| Validation | Attach/detach; TTY size; stream close; error path parity with UserdError variants |
| Removal proof | `d2b-userd` binary removed after ComponentSession attach parity on EphemeralProcess; all d2b exec CLI paths must use new attach mechanism |

### ADR046-exec-012

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-exec-012` |
| Dependency/owner | ADR046-exec-001; Nix resource compiler owner |
| Current source | `nixos-modules/options-realms-workloads.nix`: `d2b.realms.<realm>.workloads.<name>.kind` = `local-vm`/`qemu-media` → `d2b.zones.<zone>.resources.<name>` (flat, `type="Guest"`); `unsafe-local` → `d2b.zones.<zone>.resources.<name>` (flat, `type="Host"`, user-only; see unsafe-local anchor); `nixos-modules/options-realms.nix`: top-level `d2b.realms` option shape including `providerKind` regex `^[a-z][a-z0-9-]*$` and `realmPath` regex `^[a-z][a-z0-9-]*(\\.[a-z][a-z0-9-]*)*$` (Zone hierarchy path encoding); `nixos-modules/processes-json.nix`; `nixos-modules/options-host.nix`; `nixos-modules/host.nix`; `packages/d2b-realm-core/src/realm.rs`: `RealmPath`, `RealmControllerPlacement` (current Zone declaration structure analog) |
| Reuse action | adapt |
| Destination | `nixos-modules/options-zones.nix`: `d2b.zones.<zone>.resources` option as `types.attrsOf (types.submodule resourceModule)` where each resource module has `type` (required enum), optional `metadata` submodule (`ownerRef`, `labels`, `annotations`), and `spec` (type-dependent, auto-generated submodule); `nixos-modules/zone-bundle.nix`: zone resource bundle emitter (see ADR046-exec-014); `nixos-modules/resource-schemas/`: generated per-type Nix option modules imported by `options-zones.nix` |
| Detailed design | `d2b.zones.<zone>.resources` is a flat attrset; each entry has `type`, optional `metadata` (`ownerRef`, `labels`, `annotations`), and `spec`. The attrset key is the resource name. `spec` submodule fields and their Nix types/defaults/docs are auto-generated from the committed `packages/d2b-contracts/src/v3/schemas/<Type>.json` via `xtask gen-resource-nix-options`; no second vocabulary and no renaming of `spec.*` fields. `spec.provider.settings` sub-fields are constrained to the specific Provider's `providerNixSettingsSchema` attribute if present. All 17 eval-time validation rules from the "Eval-time validation rules" section are enforced by `lib.assertMsg` on the flat resource attrset. The `spec` object in the emitted JSON is the direct 1:1 serialization of the `spec` submodule. `metadata.ownerRef`, `metadata.labels`, `metadata.annotations` are serialized into the `metadata` object of the ResourceEnvelope JSON. `metadata.managedBy` and `metadata.configurationGeneration` are NOT in the bundle; they are set by the activation controller at runtime. `metadata.name` is the attrset key; `metadata.zone` is the enclosing zone key. Eval errors carry stable rule codes (1-17). Status is never present in the Nix option. `Guest.spec.systemArtifactId` (top-level spec field, not in `spec.provider.settings`) is validated against `d2b.artifacts` by rule 17; the `spec` submodule never contains derivation values or store paths. |
| Integration | Zone Nix configuration → eval-time validation → Nix-to-ResourceEnvelope compilers → zone bundle emitter (ADR046-exec-014) → configuration publication controller (ADR046-exec-015) |
| Data migration | Full reset; Realm/Workload options removed in the Nix reset wave |
| Validation | nix-unit eval/build tests 1-18 from the "Tests" section; eval validation rule tests 1-17 with expected error messages and stable codes; `spec` fields in emitted JSON match `spec` submodule values exactly (1:1 invariant test); `type` field in JSON matches `type` option value; `metadata.name` = attrset key; `metadata.zone` = enclosing zone key; `Guest.spec.systemArtifactId` plain string in resource bundle JSON at top-level spec (no store path, not in `spec.provider.settings`); missing/wrong-type artifact ID raises rule 17 eval error |
| Removal proof | Realm/Workload Nix options removed only after Zone resource Nix option parity and successful eval tests |

### ADR046-exec-013

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-exec-013` |
| Dependency/owner | ADR046-exec-008; EphemeralProcess cleanup controller owner |
| Current source | No direct current equivalent; current processes.json has no TTL/cleanup concept |
| Reuse action | create |
| Destination | `packages/d2b-core-controller/src/cleanup.rs`: EphemeralProcess TTL cleanup controller handler |
| Detailed design | Cleanup controller handler as specified in core-controllers spec; watches EphemeralProcess resources for terminal phase; computes cleanupEligibleAt from successfulTtl/failedTtl + completedAt; handles incidentHold; respects finalizers; issues normal Delete via ResourceClient; does not remove rows directly; bounded requeue-at for TTL expiry |
| Integration | core-controller process; ResourceClient Watch(EphemeralProcess); ResourceClient Delete |
| Data migration | Full d2b 3.0 reset; no v2 state/config import |
| Validation | Succeeded TTL default 1h; Failed TTL default 24h; incidentHold blocking; finalizer blocking; Delete with expected revision; cleanup controller restart recovery |
| Removal proof | None - net-new controller; no prior owner to remove |

### ADR046-exec-014

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-exec-014` |
| Dependency/owner | ADR046-exec-001 + ADR046-exec-012; Nix resource compiler owner |
| Current source | `nixos-modules/processes-json.nix` (current Nix-to-JSON serialization pattern); `nixos-modules/manifest.nix` (current bundle manifest emitter with `bundleVersion`, `manifestVersion`, integrity-tracking pattern); `packages/xtask/src/main.rs` (`gen-schemas`) (current schema-fingerprint generation method); `packages/d2b-core/src/bundle.rs`: `Bundle`, `BundleVersion`, `BundleManifest`, `BundleArtifact` (current bundle integrity model) |
| Reuse action | adapt |
| Destination | `nixos-modules/zone-bundle.nix`: Zone resource bundle emitter; `nixos-modules/resource-schemas/`: generated per-type Nix option submodules; `packages/d2b-contracts/src/v3/resource_bundle.rs`: `ResourceBundle`, `ResourceEnvelope`, `BundleIntegrityPin` Rust types (the integrity fields `contentHash`, `artifactCatalogDigest`, and `providerSchemaDigests` live inline on the bundle envelope; there is no separate `BundleManifest` type per D119); `packages/xtask/src/gen_resource_schemas.rs`: `xtask gen-resource-schemas` (generates schema JSON and Nix option modules) |
| Detailed design | `zone-bundle.nix` iterates the flat `d2b.zones.<zone>.resources` attrset (all types share the same attrset; each entry has `type`, optional `metadata`, and `spec`). For each entry it serializes the `spec` submodule to canonical JSON 1:1 (field names unchanged; keys sorted at every level; order-significant arrays preserve declaration order; semantically unordered arrays sorted). `metadata.ownerRef`, `metadata.labels`, `metadata.annotations` are serialized from the author-supplied `metadata` submodule into the envelope's `metadata` object. `metadata.managedBy` and `metadata.configurationGeneration` are NOT in the bundle envelope; they are set by the activation controller at runtime. `metadata.name` is the attrset key; `metadata.zone` is the zone key. `xtask gen-resource-schemas` generates both the schema JSON files AND the per-type Nix option submodule files under `nixos-modules/resource-schemas/` from the Rust DTO definitions; both must be regenerated when any ResourceType spec or Rust struct changes (same drift-gate pattern as current `make test-drift` / `xtask gen-schemas`). The bundle sorts all envelopes by `type` then `metadata.name` alphabetically, computes `contentHash` from the canonical JSON of the `resources` array, and emits the bundle as a NixOS store artifact. No secret values, credentials, or OS paths not already declared in `spec` or `metadata` fields may appear in any envelope JSON. The bundle file includes private integrity metadata (resource type schema fingerprints, per-Provider schema fingerprints) alongside the `resources` array; these are bundle-level fields, never per-envelope fields. Alongside the resource bundle, the emitter installs the global private artifact catalog at `/etc/d2b/artifact-catalog.json` (root:d2bd 0640) mapping each `d2b.artifacts.<id>` to `{ "sha256", "size", "storePath", "type" }`; this catalog is never included in public resource bundle envelopes. The bundle envelope includes an `artifactCatalogDigest` anchor binding the catalog to the bundle for activation-time integrity verification. |
| Integration | Zone Nix configuration → ADR046-exec-012 compilers → ADR046-exec-014 bundle emitter → `/etc/d2b/zones/<zone>/resource-bundle.json` symlink → ADR046-exec-015 activation controller |
| Data migration | Full d2b 3.0 reset; new artifact replaces current `processes.json` and `manifest.json` pattern for Zone resources without importing v2 state |
| Validation | nix-unit test: bundle sort order (Host < Guest < Process; within type, names alphabetical); nix-unit test: `contentHash` recomputed from resources array matches recorded value; nix-unit test: schema fingerprints appear in private bundle file fields, not in any individual ResourceEnvelope `metadata` or `spec` object; nix-unit test: `artifactCatalogDigest` changes when any artifact derivation changes; nix-unit test: `schemaFingerprint` changes when ResourceTypeSchema JSON changes; nix-unit test: `providerSchemaFingerprint` changes when Provider settings schema changes and is null when Provider declares no schema; nix-unit test: no inline secret value passes through to bundle JSON; nix-unit test: identical configuration produces byte-identical bundle JSON across two builds; nix-unit test: artifact catalog JSON contains `storePath` field for each entry; nix-unit test: no envelope in `resources` array contains `storePath`, `nixSystem`, `schemaFingerprint`, or `providerSchemaFingerprint` |
| Removal proof | `nixos-modules/processes-json.nix` and current `manifest.nix` bundle artifact for Host/Guest/Process retained in parallel until Zone resource bundle replaces all roles; then removed |

### ADR046-exec-015

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-exec-015` |
| Dependency/owner | ADR046-exec-014 + ADR046-exec-001 + ADR046-exec-022; configuration generation controller owner |
| Current source | No direct current equivalent; current realm/workload configuration is applied at NixOS activation time directly, not through a Resource API generation controller |
| Reuse action | create |
| Destination | `packages/d2b-core-controller/src/configuration.rs`: `ZoneConfigController`, `GenerationState`, `PendingCleanup`, `BundleActivation`, `ActivationResult`, `ActivationError`, `GenerationDiff`, `DiffEntry`, `DiffKind`, `CleanupRecord`, `CleanupPhase`, `CleanupOutcome`, `RetentionPolicy`, `RetentionState`; `packages/d2b-core-controller/src/audit.rs`: audit event emission per the "Audit events" section |
| Detailed design | The Zone configuration controller (`packages/d2b-core-controller/src/configuration.rs`) runs as a fixed process in the `d2b-core-controller` crate. It watches `/etc/d2b/zones/<zone>/resource-bundle.json` (inotify or polling). On change: (1) read and verify `contentHash` integrity; fail closed on mismatch, emit `d2b.zone.config.activate.error` with `config-bundle-integrity-failed`, make no changes. (2) Compare candidate `contentHash` against the currently active bundle record; if identical, this is a no-op re-activation (return immediately). (3) Verify resource type schema fingerprints (from bundle private fields) against committed schemas at `packages/d2b-contracts/src/v3/schemas/<type>.json`; any mismatch fails the entire bundle closed with `config-schema-mismatch`, emits error, makes no changes. (4) Verify Provider schema fingerprints against installed Provider schemas; any mismatch fails the entire bundle closed with `provider-schema-mismatch`, emits error, makes no changes. (5) Verify the `artifactCatalogDigest` anchor against `/etc/d2b/artifact-catalog.json`; mismatch fails bundle closed. (6) Fetch current `metadata.managedBy="configuration"` resources via ResourceClient List (one call per type). (7) Compute `GenerationDiff` (new/changed/unchanged/removed) by type+name key. (8) Submit Create, UpdateSpec, and Delete intents concurrently with bounded async concurrency (default max 32 in-flight); activation returns after all intents are durably queued by the resource store, without waiting for reconcile loops to complete. For unchanged specs: issue UpdateConfigGeneration to refresh `metadata.configurationGeneration` to the new generation number; no controller reconcile triggered. For new resources: core sets `metadata.managedBy="configuration"` and `metadata.configurationGeneration=<nixGeneration>` on Create; if a same-name resource already exists with `metadata.managedBy="controller"` or `metadata.managedBy="api"`, record a per-item `config-collision` error for that resource without seizing it, emit error, continue other intents. For changed specs: submit UpdateSpec with `expectedRevision`; retry on optimistic lock conflict. For removed resources: submit Delete; set `metadata.deletionRequestedAt`. (9) Set Zone `phase=Pending` while create/update intents are outstanding. (10) Return after durable queue commit; do not block on reconcile completion. (11) The cleanup controller watches ResourceClient Watch streams for `Deleted` revision events (not polling GET) for each pending-cleanup resource by type+name+expectedRevision. When all finalizers release, the resource store commits the `Deleted` revision event atomically with row and index removal in a single transaction; following this commit, the audit subsystem appends the deletion audit record using a dedup/exactly-once recovery key (audit is NOT part of the atomic store transaction). Zone transitions to `phase=Degraded` immediately whenever any pending-cleanup item is outstanding. When all pending-cleanup items receive `Deleted` Watch events, `pendingCleanup` empties and Zone transitions to `phase=Ready`. Cleanup-stuck threshold: 10 min default; configurable; stuck resources remain Degraded without blocking later activations. Prior generation retention: controller retains the N most recently activated, cleanup-complete bundle records (default N=3; range 1..16; no time-based TTL). |
| Integration | Fixed process in `packages/d2b-core-controller`; ResourceClient (Create/UpdateSpec/UpdateConfigurationGeneration/UpdateStatus/Delete/List/Watch) per ADR046-exec-022; Zone resource status UpdateStatus (configurationGeneration, pendingCleanup, lastActivatedAt, lastActivationError); Zone `phase` transitions (Pending while outstanding intents; Degraded immediately when cleanup remains; Ready when create/update complete and cleanup empty); audit segment per ADR046-exec-014 audit events table |
| Data migration | Full d2b 3.0 reset; no v2 state/config import |
| Validation | Runtime/integration tests 19-23 from the "Tests for Nix configuration and ResourceType-specific lifecycle" section in this spec; additionally: `GenerationDiff` hermetic unit tests (new/changed/unchanged/removed classification); `contentHash` integrity failure aborts and emits correct audit event; `artifactCatalogDigest` mismatch aborts bundle; UpdateSpec optimistic lock conflict retried correctly; Watch `Deleted` revision events consumed (not polling GET) to track cleanup completion; Zone `phase=Pending` while intents outstanding; Zone `phase=Degraded` immediately when any cleanup outstanding (no grace window); Zone `phase=Ready` when complete; activation returns after durable queue commit, not after reconcile; same-name `managedBy=controller` OR `managedBy=api` collision emits per-item `config-collision` error without seizing resource, other intents continue; unchanged spec refreshes `configurationGeneration` without triggering controller reconcile; final deletion: atomic tx commits `Deleted` revision event + row/index removal only; audit append follows committed revision via dedup/exactly-once recovery (NOT part of atomic tx); recovery retry produces no duplicate audit record; prior bundle record released after cleanup-complete and retention count exceeded; activation with zero diff and identical `contentHash` is a no-op |
| Removal proof | None - net-new controller; no prior owner to remove |

### ADR046-user-session-001

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-user-session-001` |
| Dependency/owner | ADR046-zone-control-019 (authority index); `Provider/system-systemd` (user manager) + core/user-agent owner |
| Current source | None - the fixed user-session authority is today ambient prose across the display/audio/clipboard/notification/secret-service dossiers; no named owner exists |
| Reuse source | `Provider/system-systemd` user-manager scope; D077 EffectPort/LaunchTicket FD handoff |
| Reuse action | adapt |
| Destination | `packages/d2b-core-controller/src/user_session_authority.rs` (or a core/user-agent per-session agent Process under `Provider/system-systemd`); `AuthorityDescriptor` on the session authority |
| Detailed design | Name and implement the **fixed user-session authority** (D097 desktop/session): `authorityScope: seat` bound to `(Host, User, login-session/seat)`, opaque `authorityKey` (never a raw socket path/XDG_RUNTIME_DIR/DISPLAY/seat name), `cardinality: exactly-one` per `(Host, User, login-session)`, `arbitration: exclusive`, owner = a core/user-agent per-user-session agent Process (NOT a new Provider), adoption by `ownerProof` (agent Process identity + login-session id), `exportability: forbidden`. It is the sole opener of the compositor/PipeWire/session-bus FDs and hands them to desktop Providers only via the EffectPort/LaunchTicket. Core's authority index rejects a duplicate session authority (or a duplicate same-user display portal, clipboard host, notification sink, audio mediator, systemd user manager, Secret Service, or seat-input claimant) with `duplicateConflict` before any FD open; multi-user/seat is admitted only up to the declared per-Host limit. Guest-stop invalidates every session authority/lease bound to that Guest across display/audio/notification/credential/shell in one dependency-aware cascade (D091), with no stale FD surviving. Host input (`wl_seat`/pointer constraints) is an `at-most-one`-per-seat authority under this session authority; pointer-constraint enforcement is a declared boundary until an interaction Provider implements it. Primary reuse disposition: `adapt`. Preserved source-plan detail: net-new (name and implement the shared user-session authority). |
| Integration | Core authority index (ADR046-zone-control-019); `Provider/system-systemd` user manager; display/audio/clipboard/notification/secret-service/shell Provider services bind to this single authority for their FDs; D091 Guest-stop cascade |
| Data migration | None - full d2b 3.0 reset |
| Validation | Single session authority per `(Host, User, session)`; duplicate same-user session authority / desktop service rejected with `duplicateConflict`; multi-seat declared-limit enforcement; Guest-stop invalidates all bound desktop/audio/notification/credential/shell authorities and leases (no stale compositor/PipeWire/session-bus FD); seat-input second claimant rejected; adoption by `ownerProof` and quarantine on ambiguity; hermetic with fakes |
| Removal proof | Not applicable (net-new named authority; replaces ambient prose) |

---

## Bus and ComponentSession reuse work items

The following work items record exact reuse from main commit `a1cc0b2d` into the v3
implementation. The pre-ADR45 v3 baseline has no equivalent for any of these; all
are ADR-only until the corresponding destination crate is created. Main commit
code is the sole reuse source. No item may claim the behavior is already
implemented on the pre-ADR45 v3 baseline.

Each item records: exact main commit file and symbol set selected for reuse;
excluded ADR 0045 assumptions that must not be copied; v3 destination crate and
integration point.

### ADR046-exec-016

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-exec-016` |
| Dependency/owner | ADR046-exec-001; `d2b-session` v3 crate owner |
| Current source | No pre-ADR45 v3 baseline equivalent; ADR-only |
| Main reuse source | main `a1cc0b2d` `packages/d2b-session/src/`: `driver.rs` (`ComponentSessionDriver` trait, `SessionDriverHandle`); `engine.rs` (`SessionEngine<T>`, `SessionEvent`); `handshake.rs` (`NoiseHandshake`, `EstablishedHandshake`, `HandshakeRole`, `HandshakeCredentials`, `NegotiatedOffer`, all encode/decode/negotiate functions, generation-discovery functions, `GENERATION_DISCOVERY_REQUEST_LEN`, `GENERATION_DISCOVERY_RESPONSE_LEN`); `lifecycle.rs` (`SessionLifecycle`, `SessionPhase`, `KeepaliveAction`); `record.rs` (`ProtectedRecord`, `RecordProtector`); `fragmentation.rs` (`Fragment`, `Fragmenter`, `Reassembler`); `scheduler.rs` (`FairScheduler`, `OutboundFrame`, `QueueClass`); `streams.rs` (`NamedStreamMux`, `StreamEvent`, `StreamId`, `StreamPhase`); `cancellation.rs` (`Cancellation`, `RequestRegistry`); `deadline.rs` (`DeadlineBudget`); `bootstrap.rs` (`BootstrapPsk`, `AdmittedBootstrapPsk`, `BootstrapAdmission`, `Secret32`); `attachment.rs` (`AttachmentPayload` trait, `AttachmentValidationError`, `OwnedAttachment`); `transport.rs` (`OwnedTransport` trait, `TransportDescriptor`, `TransportError`, `TransportPacket`); `metrics.rs` (`MetricEvent`, `MetricsSink`, `NoopMetrics`); `server.rs` (`serve_ttrpc_services`, `SessionServerError`); `error.rs` (`Result`, `SessionError`); `tests/component_session.rs` (all 10+ tests: fixed negotiation, Noise profiles, record protection, fragmentation, deadline, cancellation, lifecycle/keepalive/reconnect, named stream credit/fairness, bootstrap, transport portability); `tests/noise_vectors.rs` |
| Reuse action | adapt |
| Destination | `packages/d2b-bus-session/src/`: all above modules verbatim; `packages/d2b-bus-session/tests/`: all above tests verbatim |
| Detailed design | The entire `d2b-session` portable ComponentSession runtime is transport-agnostic and contains no ADR 0045 realm-specific types. All Noise handshake parameters, record framing, fragmentation, named stream mux, fair scheduler, cancellation, deadlines, bootstrap PSK, and attachment bindings are directly reusable. `ComponentSessionDriver` trait and `serve_ttrpc_services` are the primary integration surface for every v3 bus service. The `sessions/lib.rs` re-export boundary (`pub use d2b_contracts::v2_component_session as contract`) must be updated to point at the v3 wire contract module. Primary reuse disposition: `adapt`. Preserved source-plan detail: copy verbatim; rename crate from `d2b-session` to `d2b-bus-session` or retain name. |
| Integration | All v3 bus service implementations (`d2b-zone-service`, `d2b-provider-agent`, `d2b-bus-client`) depend on this crate; EphemeralProcess attach service; Process Provider launch ticket channel |
| Data migration | Full d2b 3.0 reset; no v2 state/config import |
| Validation | Copy tests verbatim; all tests must pass on v3 baseline without modification; re-run `tests/noise_vectors.rs` golden vectors; add one v3-specific test: endpoint policy identity uses v3 `ZoneId`/zone-name binding rather than ADR45 `RealmId` |
| Removal proof | None - net-new; no prior owner to remove |
| Excluded ADR45 assumptions | `d2b_contracts::v2_component_session as contract` import alias points at v2 wire types; v3 must point at v3 bus wire types. No other realm/workload assumption exists in this crate. |

### ADR046-exec-017

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-exec-017` |
| Dependency/owner | ADR046-exec-016; `d2b-bus-session-unix` crate owner |
| Current source | `packages/d2b-session-unix/src/` at pre-ADR45 baseline `b5ddbed6` (partially equivalent: `d2b-session-unix` existed with `d2b-session-unix/src/adapter.rs` `UnixSeqpacketTransport`, `d2b-session-unix/src/descriptor.rs`, `d2b-session-unix/src/pidfd.rs`); main commit extends with credit pools, full pidfd identity verifier, and `host-socket` feature gate |
| Main reuse source | main `a1cc0b2d` `packages/d2b-session-unix/src/`: `adapter.rs` (`UnixSeqpacketTransport`, `UnixStreamTransport`, `UnixAttachmentPayload`, `OwnedUnixAttachment`, `PeerIdentityPolicy`, `DescriptorPolicyResolver`, `PathnamePeerVerifier`); `credit.rs` (`CreditPool`, `CreditScope`, `CreditScopeSet`, `CreditBundle`, `CreditError`, `ProcessCreditLimit`); `descriptor.rs` (`ReceivedPacket`, `VerifiedPacket`, `AcceptedAttachment`, `DescriptorPolicy`, `FirstPacketCredentials`, `ObjectIdentity`, `PeerCredentials`); `pidfd.rs` (`PidfdEvidence`, `PidfdIdentityVerifier`, `PidfdInfoSource`, `ProcPidfdIdentityVerifier`, `ProcSelfFdInfoSource`, `DigestEvidenceCallback`, `parse_pidfd_fdinfo`); `socket.rs`, `systemd.rs`, `vsock.rs`; `error.rs` (`UnixSessionError`); `tests/unix_session.rs` (20+ tests: seqpacket/stream transport, attachment transfer, SO_PEERCRED credential probing, pidfd identity verification, credit pool, full end-to-end session engine) |
| Reuse action | adapt |
| Destination | `packages/d2b-bus-session-unix/src/`: all above modules verbatim; `packages/d2b-bus-session-unix/tests/`: all above tests verbatim |
| Detailed design | Provides the Linux-specific `OwnedTransport` implementation for Unix seqpacket and stream sockets. `CreditPool`/`ProcessCreditLimit` enforces per-scope FD attachment budget (ADR45 constants: `MAX_PROCESS_ATTACHMENT_CREDITS = 2048`, `MAX_HOST_ATTACHMENT_CREDITS = 8192`, `RESERVED_CONTROL_FDS = 64`). `PidfdIdentityVerifier` provides the `/proc/<pid>/fdinfo/<fd>` parse path that Process controllers use to verify process identity before `pidfd_open(2)` - this is a direct dependency of system-systemd and system-minijail Process Providers (ADR046-exec-006, ADR046-exec-007). Primary reuse disposition: `adapt`. Preserved source-plan detail: copy verbatim; rename crate from `d2b-session-unix` to `d2b-bus-session-unix` or retain name. |
| Integration | v3 Zone runtime public socket listener; system-minijail/system-systemd pidfd identity verification; EphemeralProcess attach named stream |
| Data migration | Full d2b 3.0 reset; no v2 state/config import |
| Validation | Copy all 20+ tests verbatim; all must pass; add v3-specific test: `PathnamePeerVerifier` verifies against `d2b-zonert` daemon uid; credit pool per-Zone-runtime limits match v3 constants |
| Removal proof | If the crate is renamed, the superseded `packages/d2b-session-unix/` owner is removed or reduced to a compatibility wrapper after `packages/d2b-bus-session-unix/` passes copied and v3-specific tests; if the name is retained, no prior owner is removed. |
| Excluded ADR45 assumptions | `d2b-daemon-access` uses `d2b_contracts::v2_identity::RealmId/RealmPath/WorkloadName` for its route table - not present in session-unix itself; no exclusions needed in this crate |

### ADR046-exec-018

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-exec-018` |
| Dependency/owner | ADR046-exec-016; `d2b-bus-wire` contract owner |
| Current source | No pre-ADR45 v3 baseline equivalent for v3 bus wire types; ADR-only |
| Main reuse source | main `a1cc0b2d` `packages/d2b-contracts/src/v2_component_session.rs`: all protocol constants (`PREFACE_MAGIC`, `COMPONENT_SESSION_MAJOR=2`, `COMPONENT_SESSION_MINOR=0`, `MAX_HANDSHAKE_OFFER_BYTES`, `HANDSHAKE_OFFER_CANONICAL_LEN`, `MAX_PROTECTED_CIPHERTEXT_BYTES`, `NOISE_TAG_BYTES`, `RECORD_LENGTH_BYTES`, `MAX_LOGICAL_MESSAGE_BYTES=1MiB`, `MAX_ACTIVE_NAMED_STREAMS=128`, `MAX_PACKET_ATTACHMENTS=32`, `MAX_REQUEST_ATTACHMENTS=64`, `MAX_OPERATION_ATTACHMENTS=128`, `MAX_SESSION_ATTACHMENTS=256`, `MAX_PROCESS_ATTACHMENT_CREDITS=2048`, `MAX_HOST_ATTACHMENT_CREDITS=8192`, `RESERVED_CONTROL_FDS=64`, `MAX_NAMED_STREAM_QUEUE_BYTES=256KiB`, `MAX_AGGREGATE_NAMED_STREAM_QUEUE_BYTES=4MiB`, `MAX_TTRPC_CONTROL_QUEUE_BYTES=2MiB`, `MAX_SESSION_CONTROL_QUEUE_BYTES=64KiB`, `MAX_CLOCK_SKEW_MS=30000`, `MAX_REQUEST_LIFETIME_MS=900000`, `LOCAL_HANDSHAKE_DEADLINE_MS=5000`, `REMOTE_HANDSHAKE_DEADLINE_MS=15000`, `LOCAL_RECONNECT_DEADLINE_MS=5000`, `REMOTE_RECONNECT_DEADLINE_MS=30000`, `MAX_RECONNECT_ATTEMPTS=10`, `MAX_RECONNECT_WINDOW_MS=300000`, `MAX_KEEPALIVE_INTERVAL_MS=60000`, `MAX_KEEPALIVE_TIMEOUT_MS=30000`, `RECORD_HEADER_LEN=24`, `FRAGMENT_HEADER_LEN=24`, `GUEST_SESSION_CREDENTIAL_MAGIC`, `GUEST_SESSION_CREDENTIAL_SCHEMA_VERSION`); `AttachmentDescriptor`; `LimitProfile`; `EndpointRole`; `PurposeClass`; `Locality`; `ServicePackage`; `RequestId`; `SessionErrorCode`; `BoundedVec`; `EndpointPolicy`; `EndpointPolicyIdentity`; `CancelRequest`/`CancelAck`/`CancelResult` |
| Reuse action | adapt |
| Destination | `packages/d2b-bus-wire/src/session.rs`: v3 bus protocol constants and wire types; all numeric constants copied verbatim; `PREFACE_MAGIC` retained; `EndpointPolicy` and `EndpointPolicyIdentity` adapted to use v3 `ZoneId`/`ProviderId` instead of ADR45 `RealmId` in the policy identity fingerprint |
| Detailed design | All numeric constants (frame sizes, credit limits, deadline values, reconnect limits) are directly reusable without change - they are derived from protocol analysis, not from realm semantics. `LimitProfile::local_default()` is the source for `serve_ttrpc_services` capacity; retain the value. `EndpointPolicy` carries the Noise static key and schema fingerprint; the fingerprint computation does not embed realm names and is reusable. The `EndpointPolicyIdentity` type carries the zone runtime's static public key - update from ADR45 `RealmId` to v3 `ZoneId` string encoding. Primary reuse disposition: `adapt`. Preserved source-plan detail: copy and adapt. |
| Integration | `d2b-bus-session` imports constants from here; all v3 bus service and client crates import protocol constants from `d2b-bus-wire` |
| Data migration | Full d2b 3.0 reset; no existing constants module state to migrate |
| Validation | Compile-time assertions on all copied numeric constants matching source values; `EndpointPolicyIdentity` golden-vector test with v3 zone name encoding; `LimitProfile::local_default()` round-trip test |
| Removal proof | None - net-new; no prior owner to remove |
| Excluded ADR45 assumptions | `v2_identity.rs` `RealmId`/`RealmPath`/`WorkloadName` in the `EndpointPolicyIdentity` fingerprint must be replaced with v3 zone name string. `ServicePackage` enum variants referencing ADR45 realm/guest service names must be reviewed against v3 service inventory. `GUEST_SESSION_CREDENTIAL_*` constants are for guest-control bootstrap; adapt for v3 guest bootstrap credential if the format changes. |

### ADR046-exec-019

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-exec-019` |
| Dependency/owner | ADR046-exec-016 + ADR046-exec-018; `d2b-provider-runtime` crate owner |
| Current source | `packages/d2b-realm-provider/src/provider.rs` at baseline `b5ddbed6` (`HostSubstrateProvider`, `RuntimeProvider`, `WorkloadProvider` traits - baseline, unwired); ADR-only for the runtime registry and RPC proxy |
| Main reuse source | main `a1cc0b2d` `packages/d2b-provider/src/`: `registry.rs` (`ProviderRegistry`, `ProviderRegistryBuilder`, `ProviderRegistryManager`, `RegistryLimits`, `AdmittedProvider`, `InFlightPermit`, `AdmissionOptions`); `rpc.rs` (`SessionIdentity`, `ProviderClock`, `SystemProviderClock`, `RpcOperation`, `RpcPayload`, `RpcCall`, `RpcResponse`, `AuthenticatedProviderRpc` trait, `RpcProviderProxy`); `instance.rs` (`ProviderInstance`, `ProviderFactory`, `provider_capabilities_are_dispatchable`, `provider_inspection_method`, `provider_method_is_dispatchable`); `context.rs` (`OwnedOperationContext`, `CancellationToken`); `error.rs` (`FactoryError`, `ProviderRuntimeError`, `RegistryBuildError`, `RegistryShutdownReport`); `lib.rs` re-exports; re-exports from `d2b_contracts::v2_provider` (all Provider trait objects: `RuntimeProvider`, `StorageProvider`, `NetworkProvider`, `DeviceProvider`, `CredentialProvider`, `AudioProvider`, `DisplayProvider`, `InfrastructureProvider`, `ObservabilityProvider`, `SubstrateProvider`, `TransportProvider`) |
| Reuse action | adapt |
| Destination | `packages/d2b-provider-runtime/src/`: `registry.rs`, `rpc.rs`, `instance.rs`, `context.rs`, `error.rs`; provider trait objects moved to `d2b-bus-wire` or `d2b-provider-contracts` |
| Detailed design | `ProviderRegistry` manages in-flight permits, draining, and provider lifecycle. `RegistryLimits { total_in_flight, per_provider_in_flight }` is directly reusable. `RpcProviderProxy` wraps a `ComponentSessionDriver` and dispatches typed `RpcCall` to an `AuthenticatedProviderRpc` implementation - the proxy pattern is fully reusable for v3 Provider resource controllers. `SessionIdentity { peer_role, service, provider_id, provider_type, provider_generation }` maps directly to a v3 Provider session credential. `InFlightPermit` RAII guard is directly reusable. The provider trait object set (`RuntimeProvider`, `StorageProvider`, etc.) adapts to v3 Provider resource typed methods; the trait hierarchy is preserved but `ProviderMethod` enum variant names may be renamed to drop ADR45 workload terminology. `ProviderRegistry::MAX_PROVIDER_REGISTRY_ENTRIES` bound from v2_provider is retained. Primary reuse disposition: `adapt`. Preserved source-plan detail: copy and adapt. |
| Integration | Zone runtime provider-agent ComponentSession; system-core Provider controller; every v3 Provider resource controller |
| Data migration | Full d2b 3.0 reset; no v2 state/config import |
| Validation | Copy `d2b-provider` tests; `RegistryLimits::validate` enforces non-zero and per≤total; `InFlightPermit` RAII release; drain/retire state machine; `RpcProviderProxy` round-trip over `FakeProvider` (from toolkit ADR046-exec-020) |
| Removal proof | Supersedes the baseline `packages/d2b-realm-provider/src/provider.rs` runtime/provider trait owner after `packages/d2b-provider-runtime/` registry/RPC tests pass and no v3 registry path imports ADR45 workload terminology. |
| Excluded ADR45 assumptions | `ProviderRegistrySnapshot`/`ProviderRegistryUpdate` and the `d2b_contracts::v2_provider::RegistryLifecycle`/`RegistryDrainPolicy` protocol belongs to the ADR45 provider-agent registration handshake; v3 replaces with Provider resource lifecycle. `ProviderRegistryAxis` - not needed in v3 flat provider registry. `ProviderId`/`ProviderType` from `v2_identity` must be rebased on v3 `Provider/<name>` ResourceRef. |

### ADR046-exec-020

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-exec-020` |
| Dependency/owner | ADR046-exec-019; `d2b-provider-toolkit` owner |
| Current source | `packages/d2b-realm-provider/src/conformance.rs` at baseline `b5ddbed6` (`check_provider_conformance`); ADR-only for toolkit server and fixture |
| Main reuse source | main `a1cc0b2d` `packages/d2b-provider-toolkit/src/`: `adapter.rs` (`ProviderAgentAdapter` - wraps `ComponentSessionDriver` + `ProviderRegistry` into a session-driven dispatch loop); `server.rs` (`GeneratedProviderServiceServer` - wires generated ttrpc service stubs to `ProviderRegistry` dispatch); `conformance.rs` (`ConformanceError`, `check_descriptor_conformance`, `check_provider_conformance` - five conformance checks: Descriptor, CapabilityPublication, FixtureMismatch, Provider failure, ObservabilityQueryResult); `fixture.rs` (`DeterministicClock`, `FakeProvider`, `Fixture`, `sample_lease_request` - hermetic test fixture with all Provider trait implementations returning deterministic results); `redaction.rs` (`Redacted<T>`, `Secret<T>` - zero-copy redaction wrappers for audit/log outputs); `registration.rs` (`register_exact_instances`, `ToolkitError` - exact-instance registry validation); `values.rs` (`ProviderValues`); `lib.rs` re-exports |
| Main reuse source (provider-agent) | main `a1cc0b2d` `packages/d2b-gateway-runtime/src/provider_agent.rs`: `ProviderAgentProcess` (entry point for a provider-agent process: accepts a pre-registered `ProviderRegistry` + established `ComponentSessionDriver`, runs `MAX_DISPATCH_IN_FLIGHT=64` concurrent dispatch, bounded audit ring `DEFAULT_AUDIT_CAPACITY=1024`, shutdown within `SHUTDOWN_TIMEOUT=5s`); `run_registered`, `run`; `ProviderAgentError`, `ProviderAgentAuditEvent`, `ProviderAgentAuditOutcome` |
| Reuse action | adapt |
| Destination | `packages/d2b-provider-toolkit/src/`: retain all modules verbatim; adapt `ProviderAgentAdapter` to use v3 `ProviderRegistry` (ADR046-exec-019) and v3 bus wire types; adapt `GeneratedProviderServiceServer` to use v3 generated service stubs (ADR046-exec-021); `packages/d2b-provider-agent/src/`: adapted from `gateway-runtime/src/provider_agent.rs` |
| Detailed design | `ProviderAgentAdapter` is the core: it drives a `ComponentSessionDriver` receive loop, dispatches decoded ttrpc frames to `ProviderRegistry`, and forwards responses. `GeneratedProviderServiceServer` closes the loop by registering all generated service stubs with `serve_ttrpc_services`. `FakeProvider` implements every v2 Provider trait with deterministic outputs - adapt each trait method to v3 Provider resource semantics while retaining the fixture pattern. `Redacted<T>` / `Secret<T>` zero-copy wrappers are used in every audit log path; copy verbatim. Provider conformance check pattern is retained: descriptor validation, capability publication, fixture round-trip, observability query result. Primary reuse disposition: `adapt`. Preserved source-plan detail: copy and adapt. |
| Integration | Every v3 Provider resource controller uses `ProviderAgentAdapter` + `ProviderAgentProcess`; conformance tests gate Provider dossier acceptance; `FakeProvider` is used in all Provider controller hermetic tests |
| Data migration | Full d2b 3.0 reset; no v2 state/config import |
| Validation | Copy conformance tests verbatim; `check_descriptor_conformance` passes on a `FakeProvider` descriptor; `check_provider_conformance` covers all five `ConformanceError` variants; `ProviderAgentProcess` shutdown within deadline test; `MAX_DISPATCH_IN_FLIGHT` semaphore back-pressure test |
| Removal proof | Supersedes baseline `packages/d2b-realm-provider/src/conformance.rs` ownership only after `packages/d2b-provider-toolkit/` conformance coverage passes; ADR45 provider-agent registration behavior is not retained in the v3 provider-agent path. |
| Excluded ADR45 assumptions | `ProviderAgentAdapter` and `ProviderAgentProcess` use `d2b_contracts::v2_identity::ProviderId`/`ProviderType` and ADR45 `ProviderRegistrySnapshot` for registration - v3 replaces with Provider resource `spec.providerRef` and static registration. `GeneratedProviderServiceServer` registers ADR45 generated stubs; v3 must regenerate stubs from v3 service protobuf definitions (ADR046-exec-021). Audit ring in `provider_agent.rs` uses bounded `VecDeque` - retain size constants, adapt audit event type. |

### ADR046-exec-021

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-exec-021` |
| Dependency/owner | ADR046-exec-016 + ADR046-exec-018; v3 Zone service contract owner |
| Current source | No pre-ADR45 v3 baseline equivalent; ADR-only |
| Main reuse source | main `a1cc0b2d` `packages/d2b-contracts/src/generated_v2_services/`: all ttrpc stub files (`daemon.rs`, `daemon_ttrpc.rs`, `guest.rs`, `guest_ttrpc.rs`, `realm.rs`, `realm_ttrpc.rs`, `activation.rs`, `broker.rs`, `clipboard.rs`, `provider_storage.rs`, `provider_substrate.rs`, `provider_transport.rs`, `runtime_systemd_user.rs`, `security_key.rs`, `shell.rs`, `terminal.rs`, `tty.rs`, `user.rs`, `wayland.rs` and their `_ttrpc.rs` counterparts); `v2_services.rs` (`StrictWireMessage` trait, `MethodSpec`, `ServiceSpec`, `ServiceInventoryDocument`, `ServiceDocument`, `service_inventory_document`, `service_schema_fingerprint`, `public_daemon_schema_fingerprint`, `direct_guest_schema_fingerprint`, `ServerStreamLease`, `TerminalStreamValidator`, `TerminalFrameDirection`, `server_stream_name`/`parse_server_stream_name`, `admit_metadata`, `validate_terminal_open_response_for_request`, `validate_spawn_response_for_request`, `decode_spawn_response_for_request`, `validate_provider_response_for_method`); `v2_guest_services.rs`, `v2_guest_configured_launches.rs` (for guest exec message types); `d2bd/src/control_services/daemon.rs` (`DaemonServiceV2<H>`, `DaemonOperationHandler` trait, `DaemonCallContext`, `DaemonMethod` enum, `DaemonPeerRole`, `DaemonAdapter`, `daemon_endpoint_policy`, `daemon_channel_binding`, `DaemonSeqpacketTransport`); `d2bd/src/control_services/provider.rs`, `guest.rs`, `realm.rs`, `allocator.rs`, `broker.rs` - service handler skeletons |
| Reuse action | adapt |
| Destination | `packages/d2b-bus-contracts/src/generated_v3_services/`: v3 generated ttrpc stubs for Zone service methods (Resource CRUD, Watch, ComponentSession service verbs); `packages/d2b-zone-service/src/`: Zone runtime service handler adapted from `DaemonServiceV2<H>` pattern; `packages/d2b-zone-service/src/admission.rs`, `handler.rs`, `routing.rs` |
| Detailed design | `StrictWireMessage` trait (decode_strict, encode_strict) is directly reusable - it enforces deny-unknown-fields decode and schema-pinned fingerprint validation. `ServiceInventoryDocument` / `service_schema_fingerprint` pattern provides the service schema publication mechanism that v3 Provider resources use to advertise their ComponentSession service interface. `DaemonServiceV2<H>` / `DaemonOperationHandler` pattern becomes the v3 Zone service handler base: `DaemonCallContext` → v3 `ZoneCallContext` with `ZoneId`, principal `User/<name>`, operation deadline; `DaemonMethod` enum → v3 `ZoneMethod` (ResourceGet, ResourceList, ResourceWatch, ResourceCreate, ResourceUpdateSpec, ResourceUpdateStatus, ResourceDelete, BusAttach); `daemon_endpoint_policy` → v3 zone endpoint policy with v3 `ZoneId`-bound static key and schema fingerprint. `server_stream_name`/`parse_server_stream_name` for Watch stream naming is reusable verbatim. `TerminalStreamValidator` / `ServerStreamLease` for terminal byte stream safety is reusable for EphemeralProcess attach. Primary reuse disposition: `adapt`. Preserved source-plan detail: adapt; v3 generates new protobuf definitions and new ttrpc stubs; reuse message shaping, method dispatch patterns, and service inventory pattern. |
| Integration | Zone runtime ttrpc service over public socket ComponentSession; Provider resource controller attaches via `RpcProviderProxy`; ResourceClient Watch uses server stream naming convention |
| Data migration | Full d2b 3.0 reset; no v2 state/config import |
| Validation | `StrictWireMessage` decode rejects unknown fields on all v3 message types; `service_schema_fingerprint` is stable across builds; `ZoneCallContext` deadline enforcement test; `DaemonSeqpacketTransport` → `ZoneSeqpacketTransport` end-to-end roundtrip; generated v3 stub compile check |
| Removal proof | None - net-new; no prior owner to remove |
| Excluded ADR45 assumptions | ADR45 realm/workload/guest service operations (`realm_ttrpc.rs`, `guest_ttrpc.rs`, `v2_guest_services.rs`) map to ADR45 `WorkloadId`/`RealmId` - do NOT import these stub types directly into v3; regenerate from v3 proto definitions. ADR45 `DaemonMethod` variants that reference `ExecOp`/`ExecOpResponse`, workload-target routing, and realm management are replaced by v3 Resource verbs. `daemon_channel_binding(uid, gid)` channel binding token uses UID/GID directly - v3 uses `User/<name>` ResourceRef binding; adapt. |

### ADR046-exec-022

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-exec-022` |
| Dependency/owner | ADR046-exec-016 + ADR046-exec-021; `d2b-bus-client` crate owner |
| Current source | No pre-ADR45 v3 baseline equivalent; ADR-only |
| Main reuse source | main `a1cc0b2d` `packages/d2b-client/src/`: `client.rs` (`Client<R,C,W>` with `RetryPolicy`, `CallOptions`, `CancellationToken`, `MetadataInput`, `ConnectedClient`, `Response`, `WallClock`, `SystemClock`); `session.rs` (`ConnectedSession`, `SessionCall`, `SessionReply`, `NamedStream`, `SharedDriver`, `ComponentSessionConnector` trait, `SessionFailure`); `service.rs` (`ServiceHandle`, `MethodHandle`, `GeneratedClient`, `ServiceKind`); `daemon_service.rs` (`DaemonClient`, `DaemonLifecycleRequest`, `DaemonMethod`, `DaemonTerminal`, `daemon_call_options`); `guest_service.rs` (`GuestClient`, `GuestOperation`, `GuestCancelCall`, `GuestInspectCall`, `GuestRetainedLogCall`); `host_socket.rs` (`HostSocketConnector`, `local_daemon_endpoint_identity`); `target.rs` (`TargetResolver` trait, `RouteTable`, `ResolvedTarget`, `RouteRecord`, `TargetInput`, `TransportKind`, `TransportSelection`, `ServiceOwner`); `error.rs` (`ClientError`, `RemoteErrorKind`, `RetryClass`); `tests/client.rs` |
| Main reuse source (daemon-access) | main `a1cc0b2d` `packages/d2b-daemon-access/src/component_session.rs`: `LocalDaemonSession` (one authenticated local daemon session wrapping `DaemonClient`; connection via `connect_component_session` using `HostSocketConnector` + `local_daemon_endpoint_identity` + peer uid verification against `d2bd` uid); `connect_seqpacket` (blocking connect helper) |
| Reuse action | adapt |
| Destination | `packages/d2b-bus-client/src/`: all above modules; `DaemonClient` → `ZoneClient` (v3 Resource CRUD/Watch verbs); `GuestClient` → `ProcessAttachClient`; `HostSocketConnector` → `ZoneSocketConnector`; `LocalDaemonSession` → `LocalZoneSession` |
| Detailed design | `Client<R,C,W>` is the core typed async client with bounded retry, wall-clock injection, and cancellation. `ConnectedSession` wraps a `ComponentSessionDriver` and provides `call()`, `open_stream()`, and `close()`. `ComponentSessionConnector` trait decouples connection establishment from the client - v3 `ZoneSocketConnector` implements this for the local Zone runtime public socket. `TargetResolver`/`RouteTable` provides request routing to local vs remote Zone runtimes. `HostSocketConnector::local_daemon_endpoint_identity` provides the peer identity pinning that prevents MITM on the local socket - this is a security-critical invariant; copy verbatim, rename from `d2bd` to `d2b-zonert` uid. `DaemonClient` method table adapts to v3 Resource verbs: `ResourceGet`, `ResourceList`, `ResourceWatch` (streaming), `ResourceCreate`, `ResourceUpdateSpec`, `ResourceUpdateStatus`, `ResourceDelete`. `ServiceHandle`/`MethodHandle`/`GeneratedClient` provide the typed client stub generation pattern. `RetryPolicy`/`RetryClass`/`RemoteErrorKind` error classification is directly reusable. Primary reuse disposition: `adapt`. Preserved source-plan detail: copy and adapt. |
| Integration | CLI (`d2b` binary), external Zone API callers, Process/EphemeralProcess controller ResourceClient, all consumer-facing API paths |
| Data migration | Full d2b 3.0 reset; no v2 state/config import |
| Validation | Copy `tests/client.rs` verbatim; add v3-specific tests: `ResourceWatch` streaming teardown; `ZoneSocketConnector` peer-uid mismatch rejection; retry policy respects `RetryClass::Transient`/`Permanent`; `TargetInput`→`ResolvedTarget` for local-only v3 Zone; `local_daemon_endpoint_identity` returns correct v3 zone-rt uid |
| Removal proof | None - net-new; no prior owner to remove |
| Excluded ADR45 assumptions | `DaemonClient` methods reference `DaemonMethod` variants tied to ADR45 workload ops (`ExecOp`, `AllocatorOp`, workload lifecycle) - do NOT import ADR45 daemon service stubs; generate v3-specific client stubs from v3 service definitions. `GuestClient` / `GuestOperation` reference `WorkloadId`/`WorkloadName` scoping - replaced by `EphemeralProcess/<name>` ResourceRef in v3. `d2b-daemon-access` connects via hardcoded `d2bd` user lookup - adapt to Zone runtime `d2b-zonert` user. `RouteTable` dedup key uses `(realm, workload)` tuples - v3 uses `(zone, resource-type, resource-name)`. |

### ADR046-exec-023

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-exec-023` |
| Dependency/owner | ADR046-exec-021 + ADR046-exec-019; Zone bus routing owner |
| Current source | `packages/d2b-realm-router/src/` at baseline `b5ddbed6` (`OperationRouter`, `RouteDecision`, `OperationRoutePlan`, `ReconcilableLease` - unwired in daemon); ADR-only for Zone service routing with idempotency |
| Main reuse source | main `a1cc0b2d` `packages/d2b-realm-router/src/`: `lib.rs` (`OperationRouter`, dedup semantics: `(realm, principal, node, operation kind, idempotency key)` tuple namespace; tombstone after retention window; replay = same-key/same-request carries original `operation_id`; conflict = same-key/different-request; `IdempotencyKeyExpired` fail-closed after window); `service_v2.rs` (`RealmServiceServer`, `RealmServiceProcess`, `RealmSessionAuthority`, `RealmServiceLimits` (`DEFAULT_MAX_REALM_BINDINGS=256`, `DEFAULT_MAX_SHORTCUTS=256`, `DEFAULT_MAX_MUTATION_RECORDS=1024`, `DEFAULT_AUDIT_CAPACITY=1024`, `MAX_CONFIGURED_BOUND=4096`, `MAX_DISPATCH_IN_FLIGHT=64`), `RealmServiceError`, `RealmMethod`, `RealmAuditEvent`, `RealmAuditOutcome`, `CredentialCustody`, `REALM_SERVICE_NAME="d2b.realm.v2.RealmService"`); `session_lifecycle.rs`; `target_resolver.rs` (`TargetResolver` trait, `RealmEntrypointTable`, `RemoteFullHostAdapter`, `RemoteNodeRegistry`, `RemotePeerClient`); `remote_node.rs`; `execution.rs` (`DurableExecTable`, `DEFAULT_MAX_EXECUTIONS`); `d2bd/src/realm_stubs.rs` (dead-code wiring skeletons for `SharedRouter`, `PeerOperationRouter`, `ProviderExecutor`, `LocalExecutor`, `PeerDaemon`) |
| Reuse action | adapt |
| Destination | `packages/d2b-zone-router/src/`: `router.rs` (v3 `ZoneOperationRouter` - idempotency semantics copied verbatim; dedup key namespace adapted from `(realm, principal, node, kind, key)` to `(zone, resource-type, resource-name, verb, idempotency-key)`); `service.rs` (v3 `ZoneServiceLimits`, `ZoneServiceServer`, `ZoneAuditEvent`); `resolver.rs` (v3 `ZoneTargetResolver`, `ZoneEntrypointTable`) |
| Detailed design | The idempotency/dedup semantics from `OperationRouter` are security-critical and must be copied exactly: (1) dedup key is the full 5-tuple namespace - reusing a key under a different principal is a conflict, not a replay; (2) expired keys leave tombstones for a no-reuse horizon; (3) same-key/same-request returns the original `operation_id` and recorded result; (4) same-key/different-request returns conflict error fail-closed. These semantics apply to all v3 Resource mutation verbs (Create, UpdateSpec, UpdateStatus, Delete) that carry an idempotency key. `RealmServiceLimits` numeric bounds are copied verbatim: `MAX_DISPATCH_IN_FLIGHT=64` gates concurrent resource mutations per Zone session. `RealmSessionAuthority` principal-binding model (session principal MUST match request principal field, derived in trusted code from authenticated session) maps directly to v3 Zone RBAC: `ZoneCallContext` carries the authenticated `User/<name>` principal from `SO_PEERCRED`; no caller-supplied principal field is accepted. `DurableExecTable`/`DEFAULT_MAX_EXECUTIONS` from `execution.rs` provides the EphemeralProcess in-flight table bound for the Zone router. |
| Integration | Zone runtime ttrpc service (ADR046-exec-021); Zone ResourceClient (ADR046-exec-022); every Resource mutation verb |
| Data migration | Full d2b 3.0 reset; no v2 state/config import |
| Validation | Idempotency replay returns original result; conflict returns error; expired tombstone fails closed; `MAX_DISPATCH_IN_FLIGHT` semaphore back-pressure; principal-binding enforcement (mismatched principal returns auth-denied); `DurableExecTable` capacity limit; v3 5-tuple dedup key golden vector test |
| Removal proof | Supersedes the baseline `packages/d2b-realm-router/src/` routing owner after `packages/d2b-zone-router/` passes idempotency, dispatch-limit, and principal-binding tests; ADR45 realm/workload route tables are not imported into v3. |
| Excluded ADR45 assumptions | `OperationRequest` envelope format uses ADR45 `RealmId`/`PrincipalId`/`NodeId`/`OperationKind` from `d2b-realm-core` - v3 `ZoneOperationRequest` uses v3 `ResourceRef`, `User/<name>` principal, Resource verb enum; do NOT import `d2b-realm-core` types. `d2b-realm-router` `REALM_SERVICE_NAME = "d2b.realm.v2.RealmService"` - v3 uses `"d2b.zone.v3.ZoneService"`. `TargetResolver` resolves `RealmTarget` to node/provider - v3 `ZoneTargetResolver` resolves `ResourceRef` to local vs remote Zone runtime. `RemoteFullHostAdapter`/`RemotePeerClient` gateway routing is ADR45 constellation; v3 remote routing shape is out of scope for this spec. |
