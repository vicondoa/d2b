# ADR 0046 resource object model

| Field | Value |
| --- | --- |
| Spec ID | `ADR-046-resource-object-model` |
| Parent | ADR 0046 |
| Status | Proposed |
| Version | 1 |
| Baseline | `b5ddbed67867d9244bf33390868101bd9b053e49` |
| Normative | Yes |
| Owners | `d2b-contracts`, Zone resource API/store |
| Depends on | `ADR-046-decision-register`, `ADR-046-terminology-and-identities` |
| Supersedes | None |

## Universal envelope

Every resource has:

```yaml
apiVersion: resources.d2b.io/v3
type: Host
metadata:
  name: host-system
  zone: dev
  uid: <store-generated>
  generation: 1
  revision: <opaque>
  ownerRef: null
  finalizers: []
  deletionRequestedAt: null
  createdAt: 2026-07-22T00:00:00Z
  updatedAt: 2026-07-22T00:00:00Z
spec: {}
status:
  observedGeneration: 0
  phase: Pending
  conditions: []
  lastReconciledAt: null
  startedAt: null
  completedAt: null
  outcome: null
```

`metadata`, `spec`, and `status` are always present. A type with no desired
fields uses `spec: {}`.

## Metadata

| Field | Rules |
| --- | --- |
| `name` | Required ResourceName; unique for the bound ResourceType in this Zone |
| `zone` | Required plain Zone name; must equal the store's `Zone/<name>` self resource |
| `uid` | Immutable, generated at create, never caller-selected |
| `generation` | Starts at 1; increments only when spec changes |
| `revision` | Opaque Zone-local latest-commit/watch token |
| `ownerRef` | Optional singular canonical same-Zone ResourceRef |
| `ownerUid` | Internal store binding, not caller-writable/portable JSON input |
| `finalizers` | Bounded unique typed finalizer IDs owned by installed controllers |
| `deletionRequestedAt` | Core-set RFC 3339 UTC timestamp; null before delete |
| `createdAt` | Core-set RFC 3339 UTC timestamp |
| `updatedAt` | Core-set RFC 3339 UTC timestamp on every mutation |

Labels/annotations are optional bounded presentation metadata. They never
select authorization, provider/controller ownership, path, process identity,
or implicit relationships. A ResourceType may declare a closed set of indexed
exact-match metadata fields.

## Spec

Spec is the desired state. Rules:

- strict schema with unknown fields denied unless the signed ResourceType
  explicitly declares a bounded vendor extension object;
- deterministic defaults applied before storage;
- canonical JSON representation and digest;
- all references use canonical ResourceRef values;
- no secrets, raw credentials, or authority decisions;
- provider-specific settings are schema-bound to the exact installed Provider;
- spec mutation requires expected revision;
- full replacement is the default; no field-level server-side apply or silent
  merge;
- a successful spec replacement increments generation and revision.

## Status

Status is observed state and is a separately authorized subresource.

### Common fields

| Field | Rules |
| --- | --- |
| `observedGeneration` | Numeric spec generation accounted for by this status |
| `phase` | `Pending`, `Ready`, `Succeeded`, `Degraded`, `Failed`, `Deleted`, or `Unknown` |
| `conditions` | Bounded latest condition set keyed by condition type |
| `lastReconciledAt` | RFC 3339 UTC; set on completed reconcile attempt |
| `startedAt` | Optional RFC 3339 UTC; resource/effect start |
| `completedAt` | Optional RFC 3339 UTC; terminal completion |
| `outcome` | Optional latest bounded outcome |

Condition:

```yaml
type: Ready
status: "True" # True | False | Unknown
reason: process-ready
message: bounded redacted operator detail
observedGeneration: 3
lastTransitionAt: 2026-07-22T00:00:01Z
```

Outcome:

```yaml
code: process-exited
exitCode: 1
message: bounded redacted error detail
retryable: true
retryAfter: 5s
occurredAt: 2026-07-22T00:00:01Z
```

`code` and `reason` are stable lower-kebab-case machine values. `message` may
contain actionable Provider detail but is bounded, UTF-8/control-character
validated, and must not contain secrets, tokens, credential material, terminal
bytes, argv/environment, state contents, or host/provider paths. ResourceType
schemas add typed status fields; they do not replace the common fields.

Status retains only the latest conditions/outcome. Prior status versions remain
in revision_log until compaction.

### Phase use

- `Pending`: desired state exists but is not yet ready/terminal.
- `Ready`: long-lived resource is healthy and current.
- `Succeeded`: one-shot/finite desired work completed successfully.
- `Degraded`: usable but one or more declared conditions are impaired.
- `Failed`: current desired generation cannot complete under the current
  retry/terminal policy.
- `Deleted`: final status event after finalizers, immediately before row/index
  removal.
- `Unknown`: owning controller/Host/Guest/link cannot currently prove state.

Starting, reconciling, retrying, draining, and deleting are condition/reason
details, not additional common phases.

## Ownership and child-triggered reconciliation

Each resource has zero or one ownerRef. Create/reparent:

1. resolves canonical owner type/name in the same Zone;
2. stores owner UID binding;
3. rejects self/cycles/excessive depth;
4. updates owner_index atomically with the resource and revision event.

Every committed child spec/status/metadata/finalizer/delete mutation produces a
coalesced `owned-resource-changed` hint for the current owner. The owner
controller relists owner_index, compares children with desired state, creates
missing children, corrects drift through expected-revision writes, and removes
no-longer-desired children under finalizer policy.

Child status updates still trigger the owner even when a controller's own
status-only event would otherwise be suppressed.

## Generation and revision

- create: generation 1, one Zone revision;
- status-only/metadata/finalizer update: generation unchanged, revision changes;
- spec update: generation increments exactly once, revision changes;
- a multi-resource mutation batch receives one Zone revision and ordered
  per-resource ordinals;
- stale expected revision changes nothing and returns conflict/current revision.

## Deletion

1. Authorized delete sets deletionRequestedAt and emits a revision.
2. Controllers complete exact finalizers child-first.
3. Final transaction emits a `phase=Deleted` change event and removes the
   resource plus indexes immediately.
4. GET returns not found. revision_log is the only deletion history.

No retained resource tombstone exists.

## Minimal standard ResourceType catalog

Core control:

- Zone;
- ZoneLink;
- Provider;
- Role;
- RoleBinding.

Standard execution/shared:

- Host;
- Guest;
- Process;
- EphemeralProcess;
- Volume;
- Network;
- Device;
- User;
- Credential.

Provider-specific semantic ResourceTypes may extend the set through signed
schemas/API bindings. They use this same envelope/status/ownership contract.

## Folded implementation detail

The following are not standalone ResourceTypes:

- budgets/cgroups;
- sandbox/namespace/seccomp/capability profiles;
- files/directories/ACLs/views/mounts outside Volume;
- process endpoints/ports/telemetry bindings;
- controller instances;
- pidfds;
- locks/leases internal to transactions/controllers;
- syscalls/broker operations.

## Current-code fit

| Item | Treatment |
| --- | --- |
| Current anchor | `d2b-realm-core/src/ids.rs`, `workload.rs`, `allocator.rs`; `d2b-core/src/storage.rs`, `processes.rs`; daemon operation/readiness/status DTOs |
| Evidence class | Current DTOs are mixed reachable/generated; universal resource envelope is ADR-only |
| Behavior retained | Strict serde, bounded IDs/messages, typed status/error enums, generation-bound exec/shell attach, storage owner/repair metadata |
| Required delta | Universal metadata/spec/status, ResourceType schemas, owner index/triggers, revisions, conditions/outcome, native deletion |
| Reuse path | Extract validators/redaction/error constants and storage lifecycle fields named in work items |
| Replacement/deletion | Existing manifest/process/storage DTOs remain until owning ResourceType integrations are live |
| Feasibility proof | Schema golden vectors, owner property tests, optimistic conflicts, status redaction, deletion event/removal |
| Future owner | Work items below |

## Implementation work items

### ADR046-object-001

| Field | Value |
| --- | --- |
| Dependency/owner | W0 shared contract root; `d2b-contracts` |
| Current source | `packages/d2b-realm-core/src/ids.rs`, `workload.rs`, `error.rs`; `packages/d2b-core/src/storage.rs`, `processes.rs` |
| Reuse action | extract and adapt |
| Destination | `packages/d2b-contracts/src/v3/resource.rs`, `resource_status.rs`, `resource_schema.rs` |
| Detailed design | Implement strict ResourceEnvelope, metadata, spec/status values, phase/condition/outcome, canonical JSON, bounds/redaction, ownerRef/UID fields |
| Integration | Store/API/SDK/Nix/codegen consume one contract |
| Data migration | Full d2b 3.0 reset; no v2 resource import |
| Validation | Golden JSON/protobuf vectors; serde unknown-field; status redaction/size/time/phase tests |
| Removal proof | Old DTOs removed per owning ResourceType wave only after rendered/runtime consumers move |

### ADR046-object-002

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-object-001; native resource store |
| Current source | `packages/d2b-realm-core/src/allocator_engine.rs`, `d2b-realm-router/src/lib.rs` shared ownership/idempotency precedents |
| Reuse action | adapt |
| Destination | `packages/d2b-resource-store-redb/src/ownership.rs`, `packages/d2b-controller-toolkit/src/owner_hints.rs` |
| Detailed design | Singular ownerRef resolution/UID binding, cycle/depth property checks, reverse index, owner hints, child-first deletion |
| Integration | Every store mutation updates owner index and hint dispatcher atomically |
| Data migration | None after reset |
| Validation | Property tests for cycles/reparent/name reuse; integration tests for child drift repair and owner cascades |
| Removal proof | Not applicable |
