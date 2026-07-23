# ADR 0046: d2b 3.0 Provider control plane

- Status: Proposed
- Date: 2026-07-22
- Baseline: protected `v3` commit
  `b5ddbed67867d9244bf33390868101bd9b053e49`
- Related: [ADR 0028](0028-guest-control-plane-over-vsock.md)
  (typed guest control), [ADR 0034](0034-storage-lifecycle-restart-and-synchronization.md)
  (storage/restart ownership), [ADR 0037](0037-local-hypervisor-runtime-seam.md)
  (runtime Provider seam), [ADR 0043](0043-realm-native-control-plane.md)
  (pre-v3 Realm architecture), and
  [ADR 0044](0044-unsafe-local-runtime-provider.md)
  (unsafe-local Provider)

## Status and output

This ADR defines the d2b v3 architecture from the protected pre-ADR-0045
branch. Main-branch ADR 0045 is not ancestry or current v3 behavior, but its
code may be copied/adapted without restriction when an exact work item proves
the source, selected behavior, destination, integration, tests, and excluded
ADR 0045 assumptions.

The decision is a concise parent for the normative
[`docs/specs/ADR-046-*`](../specs/README.md) specification set. The set, its
indexes, and this ADR are accepted as one unit.

This decision PR delivers documentation only. It does not create v3 crates,
dependencies, Nix modules, services, controllers, Providers, state stores, or
reset behavior. Future implementation requires a separate request after the
set is Accepted.

## Context

The v3 baseline has useful foundations:

- typed Provider traits in `d2b-realm-provider`;
- production ACA and Relay integrations;
- codec-neutral Realm operations, idempotency, targets, capabilities, and
  stream state machines;
- typed guest control over ttrpc/vsock;
- typed process DAGs, minijail profiles, broker effects, pidfd adoption, and
  generated storage/synchronization contracts;
- normalized Nix indexes and integrity-pinned bundle artifacts;
- user-session process/scope precedents in unsafe-local;
- separate journald/OTEL and authoritative audit paths.

It does not have a live generic Provider registry, universal ComponentSession,
Provider toolkit/client, native resource API/store, generic Provider state, or
production Realm peer frontend. Several Provider/transport/codec/client crates
are test-only or unwired. Realm artifacts and the allocator remain explicitly
metadata-only.

A kcp feasibility spike proved useful object semantics—typed spec/status,
revisions, watches, optimistic conflicts, owner/finalizer behavior, hierarchy,
and controller clients—but measured approximately 490 MiB RSS and a 176 MiB
executable. That footprint is unsuitable for recursive host/Guest/Zone use.

## Decision

### Zones and resources

`Zone` replaces Realm as the v3 public isolation, policy, routing, resource,
state, and audit unit. Every Zone owns:

- one Zone runtime;
- one embedded redb database and `d2b.resource.v3` service;
- one authoritative `Zone/<zone-name>` self resource;
- one fixed core-controller process;
- Zone-local Provider, Host, Guest, controller, policy, and ordinary resources.

Every resource belongs to one Zone. A parent represents a child with a local
`ZoneLink/<name>` resource and accesses the child's resources through the child
Zone API. Ordinary resource references never cross Zones.

### Resource references and ownership

Every `*Ref` field is:

```text
<ResourceType>/<resource_name>
```

It resolves a named resource in the same Zone. Plain enums and inline values do
not use a `Ref` suffix. Standard ResourceTypes are short and Zone-unique;
vendor ResourceTypes are qualified; API binding rejects collisions.

Every resource has zero or one `ownerRef`. Any committed child mutation
triggers reconciliation of the owner through a reverse owner index. The owner
controller relists its children and restores the complete desired child set and
configuration. Owner cycles fail, deletion is child-first/finalizer-aware, and
immutable UIDs prevent delete/recreate name confusion.

Every resource uses native `ResourceType`/`ResourceSpec` terminology and has:

```yaml
apiVersion: ...
type: ...
metadata:
  name: ...
  zone: ...
  uid: ...
  generation: ...
  revision: ...
  ownerRef: ...
  createdAt: ...
  updatedAt: ...
spec: {}
status:
  observedGeneration: ...
  phase: ...
  conditions: []
  lastReconciledAt: ...
  startedAt: ...
  completedAt: ...
  outcome: ...
```

`spec` and `status` are always present. Common identity, Zone, ownership,
revision, and timestamps remain in metadata. Status is a separately authorized
controller-owned subresource. It stores the latest bounded observation:
numeric `observedGeneration`, closed phase, conditions and transition/reconcile
datetimes, stable outcome code, optional process exit code, detailed bounded
redacted message, retryability, and ResourceType-specific fields. Earlier
status versions are revision-log history, not an embedded unbounded array.
The common phase is
`Pending|Ready|Succeeded|Degraded|Failed|Deleted|Unknown`; conditions carry
starting, deleting, retrying, and other transition detail.

### Providers and controllers

A Provider must be installed as a Zone-local `Provider/<name>` resource before
it can be selected. Every Provider is one independently buildable crate and
signed package. It may contain several separately sandboxed controller,
service, and worker binaries, but it declares one Provider identity.

Controller processes own their watch/coalescing queues and retry decisions.
Core validates signed watch plans, filters by API/scope/ownership/dependencies,
suppresses irrelevant or already-converged changes, coalesces revisions, and
delivers bounded reconcile hints immediately after durable commit. All
reconciliation APIs are asynchronous. A dedicated watch task keeps reading
while per-resource tasks reconcile; independent resources run in parallel and
long-running Process effects do not block the next ready Process. There is no
fixed poll, debounce, or inter-resource sleep. All controllers implement the
standard reconciliation contract and commit optimistic
`ResourceMutationBatch` values.

### Hosts, Guests, and process placement

Physical/local host execution contexts are `Host/<name>` resources reconciled
by Provider/system-core. A Zone may declare several policy/budget-separated
Hosts. VM, sandbox, cloud-host, and remote execution contexts are
`Guest/<name>` resources reconciled by their installed runtime Providers.

Host and Guest share one `ExecutionPolicy`:

- `providerRef`;
- `defaultDomain = system|user`;
- `allowedDomains`;
- `defaultUserRef` when its default domain is user.
- budgets and Volume/Network/Device attachment defaults.

Current unsafe-local becomes a user-only Host under
Provider/system-core, not a v3 Provider. Its no-isolation warning remains
explicit in status and UI.

Every ordinary `Process` declares:

- `providerRef`, selecting an installed Process Provider;
- `executionRef`, selecting `Host/<name>` or `Guest/<name>`;
- optional `domain`, inheriting from the referenced ExecutionPolicy;
- `userRef` when required by user domain;
- exact sandbox, principal, budget, state, filesystem, endpoint, network,
  device, config, and telemetry resource refs.

There are no Host/Guest-specific Process ResourceTypes. Provider controller
descriptors declare which Host/Guest Provider capabilities and domains their
instances support.

### System Providers and pidfd

The initial system Provider family is:

- `Provider/system-core` for Host reconciliation and local User
  discovery/status only;
- `Provider/system-systemd` for systemd-backed Process/scope lifecycle;
- `Provider/system-minijail` for broker/minijail-backed Process lifecycle.

The fixed core-controller process is also the `Provider/system-core`
controller. `Provider/system-minijail` is a second fixed bootstrap controller
because the first Process controller cannot be launched by itself. It
reconciles all later Process resources, including the system-systemd controller.
Every other Provider/controller is represented by a Process resource under a
Host or Guest.

Both Process Providers implement the same common execution schema and
conformance for long-lived `Process` and one-shot `EphemeralProcess`.
EphemeralProcess reports its terminal outcome directly and does not reference a
Process child. It retains successful results for `successfulTtl` (default
`1h`) and failed results for `failedTtl` (default `24h`), measured from
status.completedAt, before revision/finalizer-safe cleanup. Every Process
Provider must obtain and locally retain a verified pidfd.
Minijail receives it from `clone3(CLONE_PIDFD)` and owns wait/reap. The systemd
implementation binds a non-forking transient unit/scope by
InvocationID+cgroup+MainPID/start-time and opens a pidfd, while systemd owns
wait/reap. Pidfds never persist or cross d2b-bus.

### Resource plane

Each Zone embeds one redb database in its Zone runtime. redb supplies ACID
transactions, one writer, concurrent MVCC readers, crash safety, and B-tree
storage. d2b owns:

- ResourceType schemas/API exports/bindings;
- resources and derived indexes;
- native Role/Binding authorization;
- revisions/change log/watches;
- operations/idempotency;
- Zone-link cursors;
- compaction, backup, restore, upgrade, and quarantine;
- controller registration and reconcile hinting.

Concurrent clients are supported. Writes serialize through a bounded fair
coordinator. Non-conflicting queued mutations may use bounded group commit:
one redb transaction/fsync and Zone revision with ordered mutation ordinals,
while each caller receives its own result. Every mutation has expected-revision
preconditions; stale writes return conflict and never merge silently.

The aggregate Zone resource service/store plus mandatory system-core and
system-minijail controller processes must satisfy the normative footprint/
startup/latency gates, including
p95 durable commit-to-controller-handler <=5 ms and p95 ready Process
commit-to-launch-attempt <=20 ms.

### Communication

All controller control-plane traffic uses:

```text
ResourceClient
  -> d2b-bus
  -> ComponentSession
  -> selected d2b transport
  -> owning Zone d2b.resource.v3
  -> redb coordinator
```

This applies to local, user, guest, remote, and nested controller registration,
get/list/watch, mutations, status/finalizers, reconcile hints/checkpoints, and
conflicts. Providers/controllers never receive a redb handle/path, direct store
client, HTTP control plane, or alternate ambient socket.

ComponentSession uses v3 Noise-based authentication and record protection.
Authenticated peers map to canonical Zone-local resource subjects. The same
native Role/RoleBinding engine authorizes session connect, service invoke,
stream open, and resource verbs. A handshake cannot self-assert roles;
authorization leases bind policy revisions and revoke when RBAC changes.

### Primitive ResourceSpecs

A primitive is a complete standard low-level ResourceSpec. The model is kept
small: Host; Guest; Process/EphemeralProcess; Volume; and only independently
lifecycle-managed/shared/substitutable User, Network, Device, and Credential
resources approved by the normative resource catalog.

Budgets, cgroups, sandbox, namespaces, capabilities, environment, mounts,
endpoints, ports, and telemetry are fields of Host/Guest/Process specs. Files,
directories, state, ACLs, views, and mount lifecycle are one Volume spec.
Locks/leases remain internal transaction/controller mechanics unless a later
independent shared lifecycle proves a ResourceType is required.

Volume preserves fine-grained storage policy: anchored relative layout entries,
owner/group refs, mode, access/default ACLs, no-follow/inheritance/repair/
cleanup rules, named views, and Process mounts. Same-Zone Volume attachments
can bridge an authorized Host source to a Guest mount using virtiofs; the
Volume controller owns any virtiofsd Process and reports per-attachment status.

Helpers, syscalls, broker operations, pidfds, and sandbox fragments are
implementation mechanisms, not primitives.

Semantic Provider controllers compose behavior by creating owned primitive
resources. They do not call spawn, systemd, minijail, broker, filesystem,
network, or device effects directly.

## Rejected alternatives

### kcp/etcd/Kubernetes API

Rejected for runtime footprint and unnecessary workload-cluster machinery.
Useful resource semantics are retained in the native API.

### One global resource database

Rejected because Zone ownership, backup, failure, authorization, and state
must remain independently bounded.

### Separate Process ResourceTypes for Host and Guest

Rejected because location belongs to executionRef/provider/domain selection.
Duplicated Process schemas would drift and prevent implementation substitution.

### Direct controller store access

Rejected because it bypasses ComponentSession identity, d2b-bus routing, native
authorization, audit, limits, and cross-Host/Guest transport parity.

### One Provider process or dynamic library

Rejected because components need distinct UIDs, Hosts/Guests, user domains,
sandboxes, resources, and failure boundaries.

## Consequences

Benefits:

- one resource/controller model for physical hosts, users, guests, sandboxes,
  remote hosts, and nested Zones;
- independently packaged and later independently hosted Provider repositories;
- exact ownership-driven reconciliation after child drift;
- implementation-neutral Process resources with systemd/minijail substitution;
- smaller control-plane target than kcp;
- language-neutral contracts and one authenticated communication channel.

Costs:

- d2b must implement and maintain resource storage, watches, RBAC, schemas,
  backup/upgrade, controller scheduling support, and conformance;
- many current implementation-shaped roles require explicit migration work
  items;
- every ResourceType, core controller, and Provider needs a complete normative
  spec;
- more processes and controller/resource status surfaces increase operational
  complexity;
- the d2b 3.0 cutover remains destructive.

## Normative specifications

The authoritative set is indexed by
[`docs/specs/README.md`](../specs/README.md). The decision register is
[`ADR-046-decision-register.md`](../specs/ADR-046-decision-register.md).

Foundation specs are authored first. Once stable, all ready file-disjoint
resource, core-controller, cross-cutting, and Provider dossier specs are
authored in parallel. No dependent spec may invent a missing foundation choice.

## Review and acceptance

The same ADR/spec PR has two required human review gates:

1. approval before the immutable final panel snapshot;
2. approval after unanimous panel signoff.

Any content change invalidates validation and panel evidence. The changed
candidate is revalidated, repaneled, and reviewed again.

The set cannot become Accepted while it contains:

- an unresolved decision;
- a missing ResourceType/core-controller/Provider dossier;
- an undefined ref, owner, controller, process, state, limit, error, or test;
- a work item without exact v3 source and future destination paths;
- a claim that proposed v3 implementation is already live.

## Current-code fit

| Item | ADR 0046 treatment |
| --- | --- |
| Current anchor | ADR 0043 Realm model; `d2b-realm-*`; live CLI/daemon/guest-control; process DAG/broker; normalized Nix/bundle/storage contracts; ACA/Relay/gateway; unsafe-local user scopes |
| Evidence class | Mixed; exact classifications live in the authoring evidence ledger and owning specs |
| Behavior retained | Typed/fail-closed contracts, positive capabilities, idempotency, bounded streams, pidfd identity, broker mediation, generated storage ownership, argv/secret redaction, OTEL/audit separation |
| Required delta | Entire native Zone resource plane, ComponentSession/d2b-bus production path, Provider resources/toolkit, reconciliation, Primitive ResourceSpecs, Provider process packaging, Provider state, Zone routing, and reset |
| Reuse path | Copy/extract/adapt exact symbols named by each spec work item; current code is canon for current behavior |
| Replacement/deletion | No current path is removed until its resource/controller/Provider successor is integrated and tested |
| Feasibility proof | redb/resource/reconciliation/process/bus/package/state/route/security spikes specified by the normative set |
| Future owner | Exact `ADR046-*` work items in the spec-set manifest |
