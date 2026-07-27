# ADR 0046 primitive ResourceSpec composition

| Field | Value |
| --- | --- |
| Spec ID | `ADR-046-primitive-resource-composition` |
| Parent | ADR 0046 |
| Status | Accepted |
| Version | 1 |
| Baseline | `b5ddbed67867d9244bf33390868101bd9b053e49` |
| Normative | Yes |
| Owners | System/Host/Guest/Process/Volume Provider contracts |
| Depends on | `ADR-046-resource-object-model`, `ADR-046-resource-reconciliation` |
| Supersedes | Current implementation-shaped ProcessRole composition |

## ResourceType threshold

A behavior is a separate ResourceType only when it needs at least one:

- independent identity;
- independent lifecycle;
- independent controller/status;
- independent owner/finalizer;
- sharing by several parents/consumers;
- Provider substitution.

Otherwise it is a field or usage edge inside its parent ResourceSpec.

## Frozen standard catalog

Core control:

| ResourceType | Responsibility |
| --- | --- |
| Zone | Zone self identity, policy, API/store status |
| ZoneLink | Child-local uplink delegation, transport, cursor/health; compiler-only `parentZone` selects the parent allocator |
| Provider | Installed package/config/controllers/schemas/services/status |
| Role | Bounded native RBAC rules |
| RoleBinding | Subjects to Role with narrowing; no time-based expiry |
| Quota | Zone-wide/shared aggregate ceilings and observed usage |
| EmergencyPolicy | Disable scopes/actions and emergency status |

Standard execution/shared:

| ResourceType | Responsibility |
| --- | --- |
| Host | Physical/local host execution/policy/budget parent |
| Guest | VM, sandbox, cloud, or remote execution/policy/budget parent |
| Process | Long-lived supervised process |
| EphemeralProcess | One-shot asynchronous process with result retention |
| Volume | Shareable storage, layout/ACL/views, Host/Guest attachments |
| Network | Independently shared network fabric |
| Device | Inventoried/exclusive-or-shared device arbitration |
| User | Named identity, UID/session observations, ACL/process subject |
| Credential | Opaque rotating credential/lease lifecycle |
| Endpoint | Stable managed service/device/transport/control/data attachment point (D092); no raw locator |
| ResourceExport | Cross-Zone share of a scarce singleton resource via a single Provider authority; no cross-Zone Ref (D096) |
| ResourceImport | Cross-Zone share of a scarce singleton resource via a single Provider authority; no cross-Zone Ref (D096) |

Provider-specific semantic ResourceTypes may extend this set, always qualified
`<provider-name>.d2bus.org.<Type>` (for example `display-wayland.d2bus.org.WaylandSession`).
Standard types above are always unqualified; a Provider-specific type name is
never bare/unqualified.

Whether a given entity is a standard/qualified ResourceType or a permitted
opaque handle is decided by the entity promotion test in
`ADR-046-resource-object-model` § Entity promotion test (D092): a stable,
cross-boundary, independently-managed identity is a ResourceType; a
controller-internal or high-churn handle (pidfd, fd index, per-session named
stream, `OwnedTransport`, `operationId`) stays an opaque ID.

## ResourceSpec shape

### Three-layer spec shape (D089)

D089 freezes every primitive ResourceSpec as three layers. Layer 1 is the
universal Resource envelope and metadata. Layer 2 is the ResourceType base spec
at top-level `spec.*`, including `spec.providerRef`; provider-neutral required
and optional fields plus shared semantics live there. Layer 3 is the optional
canonical selected-Provider extension
`spec.provider = { schemaId, schemaVersion, settings }`; it is the only
Provider-specific desired extension for primitive ResourceTypes and for any
qualified Provider-defined ResourceType. It omits `providerRef` and
`observedProviderGeneration`: `spec.providerRef` is base, and spec is desired
rather than observed.

Every Provider `ResourceApiBinding` MUST implement the exact base spec schema
version and fingerprint for each ResourceType it serves, accept the canonical
minimal valid base Spec, and pass base lifecycle/status/error/finalizer
conformance. A Provider MAY reject an optional base capability only through its
signed standard capability matrix and a typed provider-neutral
`unsupported-capability` error; it MUST NOT ignore, reinterpret, rename,
duplicate, weaken, or require extension data for base-required behavior.
`spec.provider.settings` is strict deny-unknown, bounded, schema-versioned and
digested, validated against `spec.providerRef` at Nix build and API admission,
and fails with `spec-provider-schema-invalid` or `spec-provider-shadow` when
invalid or shadowing/restating/overriding/renaming/duplicating a base field.
Shared fields and semantics are promoted to the ResourceType base and never live
in `spec.provider`; generic CLI/controllers operate on base spec plus base
status. For the same Provider, the `spec.provider` and `status.provider` schemas
align.

The Provider ResourceType is the single D075 exception: it keeps
`spec.artifactId` and `spec.config` as its self-description because a Provider
has no non-circular `spec.providerRef` and therefore does not use the
`spec.provider` extension envelope for its install spec.

## Folded fields

### Host and Guest shared ExecutionPolicy

Inline:

- Host/Guest Provider-specific type/settings;
- defaultDomain/allowedDomains/defaultUserRef;
- optional `quotaRef` plus inline requested CPU/memory/pids/fds/I/O/storage/
  network amounts;
- network attachments;
- device attachments;
- Volume attachment defaults;
- provider-specific settings;
- boot/identity/capability requirements.

### Process and EphemeralProcess

Inline:

- sandbox namespaces/seccomp/capabilities/LSM;
- CPU/memory/pids/fds/I/O/thread/concurrency budgets (canonical nested `budget`);
- plain `template` ID and sealed `configRef`/`credentialRefs` (never a
  command/binary/argv field);
- Volume mounts by volumeRef/view/mountPath/access;
- `networkUsage`/`deviceUsage` (never `network`/`devices`);
- `endpoints`;
- bus/telemetry bindings;
- readiness/health/deadlines;
- user-domain portals.

### Volume

Inline:

- files/directories;
- state/tmp/cache;
- owner/group/mode;
- access/default ACLs;
- no-follow/inheritance/recursive;
- create/repair/cleanup/restart/adoption;
- named views;
- same-Zone Host/Guest attachments and transport settings.

### Internal-only

Not ResourceTypes:

- controller instances;
- pidfds;
- redb/OFD/controller locks;
- transient leases that have no independent external lifecycle;
- broker operations/syscalls;
- sandbox compiler fragments;
- process-group/cgroup implementation handles;
- route/session implementation handles.

## Host

Physical/local execution is an explicit Host:

```yaml
apiVersion: resources.d2bus.org/v3
type: Host
metadata:
  name: host-system
  zone: dev
spec:
  providerRef: Provider/system-core
  defaultDomain: system
  allowedDomains: [system, user]
  defaultUserRef: User/alice          # required when user is in allowedDomains (D116)
  budget: { ... }
status:
  observedGeneration: 1
  phase: Ready
  ...
  update:
    state: Current
    reasons: []
    observedGeneration: 1
    targetGeneration: 1
    disruption: None
    preserveState: true
    operationId: null
    lastAssessedAt: null
    owned: { count: 0, refs: [] }
    dependencies: { count: 0, refs: [] }
```

A Zone may have several Hosts:

- system-default Host accepting system and selected user processes;
- user-default Host for one configured defaultUserRef that rejects
  system processes;
- additional policy/budget-separated Hosts.

The v3 successor to unsafe-local is a user-only Host:

```yaml
providerRef: Provider/system-core
defaultDomain: user
allowedDomains: [user]
defaultUserRef: User/alice
```

Its status/UI preserves an explicit no-isolation posture. It cannot satisfy an
isolated/managed/credential-separated policy and never falls back from another
Host/Guest Provider. It is not a separate Provider.

## Guest

VM/sandbox/remote Guests select their own installed Provider.
They use the same ExecutionPolicy fields as Host and may add Provider-specific
boot/identity/runtime settings.

## Process common execution spec

Process and EphemeralProcess share:

```yaml
providerRef: Provider/system-systemd
executionRef: Host/host-system
domain: system # optional; defaults from Host/Guest ExecutionPolicy
userRef: null  # required/inherited for user
processClass: controller # controller | service | worker
template: controller-main
configRef: Volume/example-config
credentialRefs: []
mounts:
  - volumeRef: Volume/example-state
    view: controller
    mountPath: /state
    access: read-write
sandbox: { ... }
budget: { ... }
networkUsage: null
deviceUsage: []
telemetry: { ... }
# no inline endpoints (D092): stable endpoints are owned Endpoint resources with producerRef
```

The owning semantic Provider is metadata.ownerRef; its signed component/
process template supplies the executable package/digest. `template` is a plain
bounded ID, not a ResourceRef. No free-form executable, raw host path, numeric
UID/GID, raw seccomp program,
ambient capability list, caller-selected broker op, credential bytes, or
arbitrary socket address is accepted. Package/template/provider schemas resolve
those implementation details. These are the exact frozen common field names
(see `ADR-046-resources-host-guest-process-user`); they are never renamed to
`network`, `devices`, a command/binary/argv field, or an endpoint kind/path/
service field.

## Process

Adds:

- desired lifecycle;
- readiness/health;
- restart/backoff;
- adoption/drain;
- long-lived retention;
- provider-specific process identity.

Status includes mandatory locally held pidfd evidence (never the fd/PID in
public status), wait/reap owner, phase/conditions, ready/exit observations, and
resource revisions.

## EphemeralProcess

Adds:

- one-shot input;
- start/runtime deadlines;
- terminal output handles/digests;
- successfulTtl default `1h`;
- failedTtl default `24h`.

It is the one-shot process itself, not a Job that references a Process.
Pending/running does not expire. Terminal cleanup begins at completedAt, respects
finalizers/incident hold, and writes cleanupEligibleAt.

## Process Providers

### system-systemd

- non-forking transient system unit/scope or user scope;
- unit InvocationID+cgroup+MainPID/start-time binding;
- opens/verifies mandatory pidfd;
- systemd owns wait/reap;
- exact userRef for user scope;
- no per-Provider static PID1 template unit.

### system-minijail

- Process controller calls the `ProcessLaunchEffectPort` (ProviderSupervisor)
  with the resource UID and compiled sandbox/resource digests; the effect
  adapter resolves the plan and the broker performs the spawn;
- clone3(CLONE_PIDFD);
- d2b owns wait/reap;
- pidfd/cgroup/process start identity;
- no direct Provider broker access - the Provider process never imports or
  calls the broker itself.

Both implement identical ResourceTypes and status/error conformance.

## system-core bootstrap

The fixed core-controller process is also Provider/system-core. It and the
fixed Provider/system-minijail controller are the two Provider bootstrap
exceptions. system-core:

- reconciles Host resources;
- discovers/reconciles local User resources/status;
- runs before the first Host exists.

It does not implement Process, EphemeralProcess, Volume, Network, Device, or
Credential.

Every Provider/controller other than those two bootstrap controllers is
represented by a Process under a Host or Guest.

## Volume

Volume spec preserves fine-grained current storage policy:

```yaml
providerRef: Provider/volume-local
source:
  executionRef: Host/host-system
  settings:
    kind: local-path
    sourcePolicyId: <opaque ID bound to a Provider-declared allowlist policy entry>
layout:
  - path: state
    type: directory
    ownerRef: User/example-system
    groupRef: User/example-system
    mode: "0700"
    accessAcl: []
    defaultAcl: []
    noFollow: true
    recursive: false
    sensitivity: private
    createPolicy: create-if-never-provisioned
    repairPolicy: exact-owner
    cleanupPolicy: owner-controlled
views:
  controller:
    path: state
    rights: [read, write, create, delete, traverse]
attachments:
  - executionRef: Guest/dev-vm
    transport: virtiofs
    mountPath: /mnt/state
    view: controller
    access: read-write
```

`source.settings` never carries a raw host path in the authored spec.
`sourcePolicyId` is an opaque bounded string ID; the Provider and its
controller see the ID and semantic settings only. Raw path resolution is
private Nix/bundle/effect authority (see `ADR-046-resources-volume`). The
`transport: virtiofs` attachment is served by the separate `volume-virtiofs`
Provider, which owns only the attachment lifecycle/status and its owned
virtiofsd Process - never the Volume's own `providerRef`/layout/ownership
fields.

All layout paths are relative to the anchored Volume root. A raw host path is
never an authored spec field; the `source.settings.sourcePolicyId` on the
`local-path`/`block-image` source kinds resolves, only inside the Volume
Provider's private Nix/bundle/effect authority, to the actual host path
against that Provider's allowlisted root policy. It never appears in public
status/audit, and it never reaches the Provider process as a literal path.

Virtiofs attachment controller may create:

- owned Host EphemeralProcess/Process for setup/virtiofsd;
- target Guest Provider attachment update;
- status per export and guest mount.

Process mounts independently select Volume view/access for sandbox exposure.

## Composite ownership example

Cloud Hypervisor Guest owns:

- VMM Process under Host;
- virtiofsd Process per Volume attachment;
- audio/video/GPU worker Processes where required;
- Provider-specific endpoint/device resources only when they satisfy the
  separate ResourceType threshold.

If any child mutates, Guest owner reconcile runs and reasserts the complete
child graph.

Leaf Process Providers create no child merely to represent their internal
systemd unit/minijail state; they write Process status.

## Current-code fit

| Item | Treatment |
| --- | --- |
| Current anchor | `d2b-core/src/processes.rs`, `storage.rs`, `sync.rs`; Nix processes/storage/sync emitters; d2bd DAG; broker SpawnRunner; unsafe-local scopes; guest exec |
| Evidence class | Current process/storage contracts are reachable/generated; unified minimal resources are ADR-only |
| Behavior retained | Fine-grained ownership/ACL/no-follow, minijail profiles, pidfd/adoption, systemd user scopes, typed role/readiness, state fail-closed |
| Required delta | Host/Guest parent model, common Process/EphemeralProcess, generic Volume, Provider implementations, owner child graphs |
| Reuse path | Exact ProcessRole/storage-path work items copy/adapt current algorithms/tests |
| Replacement/deletion | No current role/path is removed without a resource/Provider successor |
| Feasibility proof | Host system/user Process; non-Host Process; virtiofs host-path→guest-mount Volume |
| Future owner | Resource/Provider dossier work items |

## Implementation work items

### ADR046-primitives-001

| Field | Value |
| --- | --- |
| Dependency/owner | W0; resource contracts |
| Current source | `packages/d2b-core/src/processes.rs`, `minijail_profile.rs`, `storage.rs`; `d2b-contracts/src/broker_wire.rs` |
| Reuse action | adapt |
| Destination | `packages/d2b-contracts/src/v3/host.rs`, `guest.rs`, `execution_policy.rs`, `process.rs`, `volume.rs`, `user.rs`, `network.rs`, `device.rs`, `credential.rs` |
| Detailed design | Complete minimal ResourceType schemas and shared execution/Volume sub-schemas Primary reuse disposition: `adapt`. Preserved source-plan detail: extract and adapt. |
| Integration | Provider dossiers/controller descriptors bind exact types |
| Data migration | Full reset |
| Validation | Schema vectors and folded-field/no-duplicate-type policy tests |
| Removal proof | Old DTOs removed only by owning future slices |

### ADR046-primitives-002

| Field | Value |
| --- | --- |
| Dependency/owner | Process contracts; system Provider slices |
| Current source | broker SpawnRunner/pidfd; d2bd supervisor; unsafe-local helper; guest exec runner |
| Reuse action | adapt |
| Destination | `packages/d2b-provider-system-systemd/`, `packages/d2b-provider-system-minijail/`, shared neutral process conformance library |
| Detailed design | Common Process/EphemeralProcess, provider-specific launch/pidfd/wait/adoption/status Primary reuse disposition: `adapt`. Preserved source-plan detail: extract and adapt. |
| Integration | Process controller registration under Host/Guest; d2b-bus ResourceClient/status |
| Data migration | Current ProcessRoles converted by exact disposition table |
| Validation | Shared conformance plus Host/Guest/user integration |
| Removal proof | Role branches removed only after successor Provider tests |

### ADR046-primitives-003

| Field | Value |
| --- | --- |
| Dependency/owner | Volume contract; Volume Provider slices |
| Current source | `storage-json.nix`, `d2b-core/src/storage.rs`, store/TPM/runtime path owners, virtiofsd argv/ProcessRole |
| Reuse action | adapt |
| Destination | `packages/d2b-provider-volume-*/`; `nixos-modules/resources-volume.nix` |
| Detailed design | Fine-grained Volume layout/views, host-path policy, virtiofs attachments/status/owned Process Primary reuse disposition: `adapt`. Preserved source-plan detail: extract and adapt. |
| Integration | Host/Guest/Process refs and Volume controller |
| Data migration | Full reset; Provider-specific state export only where separately specified |
| Validation | ACL/no-follow/marker, sharing/views, virtiofs host/guest mount tests |
| Removal proof | storage.json rows removed only after Volume successor parity |
