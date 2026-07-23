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
apiVersion: resources.d2bus.org/v3
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

Internal non-user-writable management metadata also records:

- `managedBy = configuration|controller|api`;
- optional `configurationGeneration` for Nix-owned roots;
- controller/Provider generation for dynamic children.

These fields are authority inputs set by the configuration/resource service,
not labels/annotations or caller-selected spec values.

`api` is assigned to resources created directly by an authorized API client;
they persist until explicit API deletion and are never swept by configuration
generation cleanup.

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

### Three-layer spec shape (D089)

`spec` mirrors the D088 three-layer status shape with symmetric frozen names.
Generic API/CLI/controllers author and reconcile only Layers 1 and 2; the
Layer-3 Provider extension builds on — never shadows, overrides, renames, or
duplicates — the fields below it.

| Layer | Location | Owner | Consumers |
| --- | --- | --- | --- |
| 1. Universal Resource envelope/metadata | `apiVersion`, `type`, `metadata`, `spec` presence rules | every resource | all generic tooling |
| 2. ResourceType base spec | top-level `spec.*` (incl. `spec.providerRef`) | the ResourceType schema (provider-neutral) | all implementations + generic tooling |
| 3. Provider-specific extension | optional `spec.provider` | the selected Provider (signed schema) | that Provider's implementation only |

```yaml
spec:
  # Layer 2 — ResourceType base spec (provider-neutral; includes providerRef)
  providerRef: Provider/<name>
  # ...exact typed base fields frozen by the ResourceType schema...
  # Layer 3 — optional canonical selected-Provider extension envelope
  provider:
    schemaId: <provider-name>.d2bus.org/<ResourceType>/spec
    schemaVersion: "1.0"
    settings:
      # strict, bounded, versioned, deny-unknown implementation-only desired settings
      ...
```

`spec.provider = { schemaId, schemaVersion, settings }` is the one canonical
provider-extension envelope. It replaces the former Host/Guest/Credential
`providerSettings`, Device `settings`, and every ad hoc per-ResourceType
extension. It is symmetric to `status.provider`: `spec.provider.settings` (the
desired implementation settings) mirrors `status.provider.details` (the observed
implementation state), and `spec.provider` omits `providerRef` and
`observedProviderGeneration` because `spec.providerRef` is a base field and spec
is desired, not observed. The selected `spec.provider` schema and the
`status.provider` schema align for the same Provider.

**Mapping convention.** Within any spec a reference to `spec.providerSettings`
(or, for Device, the former top-level `spec.settings`) denotes the canonical
`spec.provider.settings`; `spec.providerRef` and every other `spec.*` field is
ResourceType base.

**Base-schema conformance.** Every Provider `ResourceApiBinding` declares and
MUST implement the exact ResourceType base spec schema version/fingerprint and
pass the base lifecycle/status/error/finalizer/conformance suite. It MUST accept
the canonical minimal valid base Spec. It MAY reject an optional base capability
only via its signed standard capability matrix and a typed, provider-neutral
`unsupported-capability` result; it MUST NOT ignore, reinterpret, rename,
duplicate, or weaken the bounds of any base field, and MUST NOT require
extension data for base-required behavior.

**Provider extension.** `spec.provider` is optional unless the selected
Provider's signed schema marks it required. Its `settings` object is strict
deny-unknown, size/cardinality bounded, versioned/digested, and validated
against `spec.providerRef` at Nix build and at API admission. It carries only
implementation-only desired settings and MUST NOT shadow, restate, or override a
universal or ResourceType-base field. Any field or semantic shared across two or
more implementations is promoted to the ResourceType base and never carried in
`spec.provider`.

**Generic tooling.** Generic CLI and controllers operate on the base spec (and
base status); they never author or require `spec.provider`. Qualified
Provider-defined ResourceTypes carry the universal envelope, their own type-base
spec, and an optional implementation extension under the same `spec.provider`
envelope.

**Provider self-description exception.** The `Provider` resource's spec remains
`{ artifactId, config }` (D075) — the one documented exception to the
`providerRef`-plus-`spec.provider` shape, because a `Provider` cannot carry a
non-circular `spec.providerRef` to itself.

**Update policy (D091).** The base spec carries a provider-neutral
`spec.updatePolicy` (universal or ResourceType base): disruptive upgrades default
to `manual` (the controller reports `UpgradeRequired` and waits for an explicit
`resource upgrade ... --apply`), while automatic application of non-disruptive
changes MAY be permitted. A `spec.provider` extension MAY add implementation
knobs but MUST NOT bypass or weaken the base disruption policy.

## Status

Status is observed state and is a separately authorized subresource. It is the
**default durable observation and recovery surface** for a resource's bounded
non-secret operational state (D087): reconcile stage, opaque non-authorizing
external handles/IDs/digests, adoption observations after restart, bounded
counters, closed-enum detail, dependency-readiness observations, and last
successful checkpoints. Status is revisioned, optimistic-status-writer
controlled (every status write carries an expected revision), RBAC-readable,
redacted, and reverified against external reality by the owning controller
after a restart. The spec remains the desired-state authority; status is
observation only and is never a host-mutation or repair authority.

A controller keeps bounded non-secret operational state in `status` whenever
possible. Separate durable state storage (a Provider-owned Volume, see
`ADR-046-provider-state`) is used only when a payload is a secret or sensitive
private datum, is large/binary/file content, or is otherwise unsuitable for the
revisioned status API.

### Three-layer status shape (D088)

Every resource's `status` has a frozen three-layer shape. Generic API, CLI, and
controllers depend only on Layers 1 and 2; the Layer-3 Provider extension builds
on — never replaces, overrides, or duplicates — the fields below it.

| Layer | Location | Owner | Consumers |
| --- | --- | --- | --- |
| 1. Universal `ResourceStatus` base | top-level `status.*` common fields | every resource | all generic tooling |
| 2. ResourceType-common | `status.resource` | the ResourceType schema (provider-neutral) | all implementations + cross-resource/provider consumers |
| 3. Provider-specific extension | optional `status.provider` | the installed Provider (signed schema) | that Provider's own tooling only |

```yaml
status:
  # Layer 1 — universal ResourceStatus base (present on every resource)
  observedGeneration: 4
  phase: Ready
  conditions: [ ... ]
  lastReconciledAt: 2026-07-23T00:00:01Z
  startedAt: 2026-07-23T00:00:00Z
  completedAt: null
  outcome: null
  update:                              # universal currency object (D091)
    state: Current
    reasons: []
    observedGeneration: 4
    targetGeneration: 4
    disruption: None
    preserveState: true
    operationId: null
    lastAssessedAt: 2026-07-23T00:00:01Z
    owned: { count: 0, refs: [] }
    dependencies: { count: 0, refs: [] }
  # Layer 2 — ResourceType-common, provider-neutral (required across all implementations)
  resource:
    # exact typed fields frozen by the ResourceType schema
    ...
  # Layer 3 — optional Provider-specific extension
  provider:
    providerRef: Provider/<name>
    schemaId: <provider-name>.d2bus.org/<ResourceType>/status
    schemaVersion: "1.0"
    observedProviderGeneration: 7
    details:
      # strict, bounded, redacted, unknown-field-denied implementation observation
      ...
```

### Layer 1: universal ResourceStatus base

The universal base is present on every resource at `status` top level:

| Field | Rules |
| --- | --- |
| `observedGeneration` | Numeric spec generation accounted for by this status |
| `phase` | `Pending`, `Ready`, `Succeeded`, `Degraded`, `Failed`, `Deleted`, or `Unknown` |
| `conditions` | Bounded latest condition set keyed by condition type |
| `lastReconciledAt` | RFC 3339 UTC; set on completed reconcile attempt |
| `startedAt` | Optional RFC 3339 UTC; resource/effect start |
| `completedAt` | Optional RFC 3339 UTC; terminal completion |
| `outcome` | Optional latest bounded outcome |
| `update` | Universal provider-neutral currency object (D091); present on every resource |

### Universal currency object `status.update` (D091)

Every resource carries a bounded, provider-neutral `status.update` object in the
universal base. Because it is universal (or, for a ResourceType-specific
refinement, `status.resource`), its shared fields are never duplicated in a
`status.provider` extension (D088):

| Field | Rules |
| --- | --- |
| `state` | `Current`, `UpdateAvailable`, `UpgradeRequired`, `Upgrading`, `Blocked`, or `Unknown` |
| `reasons` | Bounded list from the closed enum `CoreGenerationChanged`, `ProviderGenerationChanged`, `ArtifactChanged`, `ImageOrSystemGenerationChanged`, `SpecChanged`, `DependencyChanged`, `SecurityPolicyChanged` |
| `observedGeneration` / `targetGeneration` | Numeric; and/or bounded non-secret observed/target digest IDs |
| `disruption` | `None`, `Reload`, `Restart`, `Recycle`, or `Replace` |
| `preserveState` | Bool: whether the planned upgrade preserves durable/state/secret Volumes and identity |
| `operationId` | Optional in-flight upgrade Operation id (core Operation ledger) |
| `lastAssessedAt` | RFC 3339 UTC of the last `assess_update` |
| `owned` | `{ count, refs }` bounded/truncated aggregate currency of owned resources |
| `dependencies` | `{ count, refs }` bounded/truncated aggregate currency of dependencies |

A controller assesses currency via `assess_update` on core/Provider-generation,
artifact/image/NixOS-generation, immutable-spec, dependency, or
security-policy change (D091). It MUST report `UpgradeRequired` rather than apply
a disruptive in-place change; non-disruptive changes reconcile normally. Core
aggregates self/owned/dependency currency for list/get. The latest bounded
upgrade plan/result lives here; the core Operation ledger owns
operation/idempotency/progress (never a second ledger in status).

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
bytes, argv/environment, state contents, or host/provider paths.

### Layer 2: ResourceType-common status (`status.resource`)

`status.resource` is the provider-neutral, typed object frozen by the
ResourceType schema. It holds the observation fields that every implementation
of that ResourceType and every cross-resource/provider consumer must be able to
read — for example Guest runtime readiness/capabilities, the Device claim base,
Credential lease metadata, or the Volume attachment base. It extends the
universal base (Layer 1) and never restates it. Generic API/CLI/controllers
depend only on the universal base plus `status.resource`. Any field shared
across two or more implementations MUST be promoted here and MUST NOT be copied
into individual Provider extensions.

### Layer 3: Provider-specific extension (`status.provider`)

`status.provider` is optional and carries implementation-only observation that
is not shared across implementations:

| Field | Rules |
| --- | --- |
| `providerRef` | `Provider/<name>` of the writing Provider |
| `schemaId` | Qualified, immutable status-extension schema ID (per the D080 grammar, e.g. `<provider-name>.d2bus.org/<ResourceType>/status`) |
| `schemaVersion` | Semver `MAJOR.MINOR` of the extension schema |
| `observedProviderGeneration` | Numeric `Provider/<name>` resource generation this observation reflects |
| `details` | Strict typed object: unknown-field-denied, size/cardinality bounded, redacted/non-secret |

The `status.provider.details` schema is signed into and registered with the
Provider package (see `ADR-046-provider-model-and-packaging`). A `status.provider`
whose `schemaId`/`schemaVersion` is not registered for the installed Provider, or
whose `details` carries an unknown field, is rejected. `status.provider` builds
on Layers 1 and 2 and MUST NOT replace, override, or duplicate any universal or
`status.resource` field.

**Atomic layered write.** The owning controller writes all present layers
(universal base, `status.resource`, and any `status.provider`) in one status
mutation with a single expected revision; the layers never diverge across
separate writes.

**Status-first state mapping (D087 + D088).** State shared across
implementations goes in `status.resource`; implementation-specific bounded
non-secret observation goes in `status.provider.details`; secret, large, or
private state goes in an optional Volume (`ADR-046-provider-state`), never in
status.

ResourceType schemas add typed `status.resource` fields and register any
`status.provider` extension; they do not replace the universal base.

### Status bounds

Status is a bounded observation surface, not a stream. The resource store
enforces the following caps on every status subresource write and rejects an
over-limit write with a typed `status-oversize` error (the write changes
nothing and the caller may re-read and retry with a smaller status):

| Bound | Limit |
| --- | --- |
| Total canonical serialized status object (all three layers) | 64 KiB |
| `status.resource` ResourceType-common typed detail | 32 KiB |
| `status.provider.details` Provider-specific extension | 32 KiB |
| `conditions` entries | 32 |
| Any status list or map field | 64 entries |
| Any single bounded status string (`message`, opaque handle, digest) | 4 KiB |

A controller writes status only on a **material change** in observed state.
Status never carries high-frequency byte streams, logs, metrics, command
output, or ring buffers; those stay in their owning surfaces (OTEL for
metrics/traces, the authoritative audit stream for security history, owning
process memory for content streams). Watches, revision compaction, and
backpressure remain bounded per `ADR-046-resource-store-redb`.

### Status prohibitions

Status MUST NOT contain secrets, raw tokens/keys/PSKs, any credential source
handle that confers authority, private endpoint/path/argv/environment/PID/unit
data, terminal/clipboard/CTAP bytes, raw cloud error bodies, large binary blobs,
unbounded collections, or any content whose churn would bloat revision history.
An opaque handle may appear in status only if it is bounded, non-secret,
non-authorizing, safe for authorized API readers, and independently revalidated
by the owning controller against external reality.

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

### Removed Nix configuration

After a newly validated Zone configuration generation activates, core diffs its
canonical configured resource set against the prior active set. Every prior
`managedBy=configuration` resource omitted from the new set receives normal
asynchronous Delete:

- activation succeeds without waiting for cleanup;
- generation status becomes Degraded/pending-cleanup while removals remain;
- owner children/finalizers complete through normal reconciliation;
- controller-created resources are never swept merely because absent from Nix;
- prior generation remains retained until cleanup/rollback policy permits
  pruning;
- failures are visible/audited and never reported as deleted.

## Minimal standard ResourceType catalog

Core control:

- Zone;
- ZoneLink;
- Provider;
- Role;
- RoleBinding.
- Quota;
- EmergencyPolicy.

Standard execution/shared:

- Host;
- Guest;
- Process;
- EphemeralProcess;
- Volume;
- Network;
- Device;
- User;
- Credential;
- Endpoint.

There are 17 standard ResourceTypes (Endpoint added by D092). Provider-specific
semantic ResourceTypes may extend the set through signed schemas/API bindings.
They use this same envelope/status/ownership contract.

## Entity promotion test and opaque-ID classification (D092)

A stable managed identity MUST be a proper ResourceType (not an opaque `*Id`/
`Handle`) when **any** of these holds:

- it is stable across a reconcile pass or restart;
- it is independently lifecycle/readiness/finalizer/lease managed;
- it is referenced by another resource, controller, or Provider;
- authorization/policy/audit applies to it independently;
- it is visible in resource status or CLI;
- the ownership/dependency graph needs it.

An opaque ID or handle is permitted **only** when it is confined to one
controller/process, not API/status-visible as a locator, has no independent
lifecycle/authorization, and/or is per-connection/high-churn. Every retained
public `*Id`/`Handle` documents its rationale. The frozen permitted-opaque set:

| Opaque handle | Why it is not a resource |
| --- | --- |
| pidfd | Process-local, non-persistent, never crosses the bus (D022) |
| fd index / inherited-fd slot | Per-launch LaunchTicket-local; not API/status-visible |
| named stream id (per ComponentSession) | Per-session/high-churn; internal to one session |
| transport byte-stream handle (`OwnedTransport`) | Per-session carriage handle held in one process (D081) |
| reconcile operation token / `operationId` | In-flight, single-controller, core Operation ledger owned |
| content/schema digest | A value, not a managed entity |

Stable cross-boundary endpoint identities do **not** qualify as opaque and are
the `Endpoint` ResourceType.

## Folded implementation detail

The following are not standalone ResourceTypes:

- budgets/cgroups;
- sandbox/namespace/seccomp/capability profiles;
- files/directories/ACLs/views/mounts outside Volume;
- process ports and telemetry bindings (stable endpoints are the `Endpoint`
  ResourceType, D092);
- controller instances;
- pidfds and per-session named-stream/transport handles;
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
| Detailed design | Implement strict ResourceEnvelope, metadata, the three-layer spec shape (universal envelope + ResourceType base `spec.*` incl. `spec.providerRef` + optional canonical `spec.provider` `{ schemaId, schemaVersion, settings }`), the three-layer status shape (universal base + `status.resource` + optional `status.provider` with `providerRef`/`schemaId`/`schemaVersion`/`observedProviderGeneration`/`details`), phase/condition/outcome, canonical JSON, per-layer bounds/redaction, ownerRef/UID fields |
| Integration | Store/API/SDK/Nix/codegen consume one contract |
| Data migration | Full d2b 3.0 reset; no v2 resource import |
| Validation | Golden JSON/protobuf vectors; serde unknown-field; three-layer spec shape round-trip; canonical minimal base-spec acceptance; base-schema version/fingerprint conformance; `spec.provider` deny-unknown/version-mismatch/shadow rejection and providerRef-binding; three-layer status shape round-trip; base-only projection (universal + `status.resource`) ignores/omits `status.provider`; `status.provider` unknown-field/version-mismatch rejection; status redaction/size/time/phase tests; `status.update` currency object round-trip (state/reasons/disruption/preserveState/owned+dependency refs bounded); `spec.updatePolicy` base round-trip |
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
