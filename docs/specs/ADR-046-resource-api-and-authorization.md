# ADR 0046 resource API and authorization

| Field | Value |
| --- | --- |
| Spec ID | `ADR-046-resource-api-and-authorization` |
| Parent | ADR 0046 |
| Status | Proposed |
| Version | 1 |
| Baseline | `b5ddbed67867d9244bf33390868101bd9b053e49` |
| Normative | Yes |
| Owners | `d2b-resource-api`, Zone authorization engine |
| Depends on | `ADR-046-terminology-and-identities`, `ADR-046-resource-object-model`, `ADR-046-resource-store-redb` |
| Supersedes | None |

## Service

`d2b.resource.v3` is a language-neutral protobuf/ttrpc service routed through
d2b-bus over ComponentSession and a d2b transport.

The resource API is asynchronous end-to-end. Rust clients expose async methods;
synchronous redb/filesystem work remains behind bounded blocking adapters in
the Zone runtime.

## Methods

| Method | Semantics |
| --- | --- |
| `Get` | Exact ResourceRef; returns current envelope/revision |
| `List` | Exact ResourceTypes and bounded filters/pagination; returns one MVCC snapshot revision |
| `Watch` | Exact authorized filters after revision; returns named stream |
| `Create` | Create-absent precondition and complete envelope spec |
| `UpdateSpec` | Full spec replacement with expected revision |
| `UpdateStatus` | Full status replacement with expected revision; controller/status-owner only |
| `UpdateMetadata` | Bounded labels/annotations/ownerRef/deletion-neutral metadata with expected revision |
| `UpdateFinalizers` | Exact finalizer add/remove with expected revision and ownership checks |
| `Delete` | Request deletion with expected revision/preconditions |
| `CommitBatch` | Atomic bounded set of the above in one Zone transaction |
| `ResolveRef` | Validate canonical ref/type/UID and return bounded identity metadata |
| `InspectSchema` | Read bound ResourceType schema/export/Provider identity |
| `Upgrade` | Authorized `assess_update`/`plan_upgrade`/`execute_upgrade` on a ResourceRef; plan-by-default, apply with explicit intent; optional `--recursive` owned/dependency planning (D091) |

`Create`, `UpdateSpec`, and `Delete` accept an authorized expedited option
`waitForReconcile` with a bounded deadline (D090; see § Expedited reconcile).

There is no arbitrary path/header/query/table API, JSON patch, server-side
apply, direct redb access, exec/port-forward, secret byte read, or generic
Provider command.

## Expedited reconcile (D090)

`Create`/`UpdateSpec`/`Delete` may set `waitForReconcile` with a bounded
deadline. Under one **mutation ticket** carrying an `operationId` and the
deadline, Core admission plus the reserved-revision redb transaction run in
parallel with the owning controller's preflight/plan:

1. The controller MAY validate and plan before commit but performs **no**
   external effect, finalizer release, or status mutation until Core supplies a
   typed `CommittedRevisionProof { resourceUid, generation, revision, operationId }`.
2. A DB failure sends `Abort`; no effect occurs. The API returns success only if
   the initial resource mutation durably committed. A durable commit is never
   rolled back because reconcile later fails or times out.
3. Commit enqueues the ordinary reconcile hint; the expedited request enters a
   bounded priority lane into the SAME per-resource single-flight reconciler
   (`ADR-046-resource-reconciliation`). Any normally queued reconcile stays
   queued and runs after the expedited pass. All effect IDs/idempotency keys
   derive from `(UID, generation, revision, operationId)`; a normal re-entry
   observes converged/progressing state and no-ops/rejoins, never duplicating.

The API waits in parallel for durable commit plus **one expedited reconcile
pass** and returns:

```yaml
resource: <committed resource object>          # base spec + committed metadata
status:   <post-pass projected layered status> # universal + resource + optional provider
disposition: Converged | Progressing | Blocked | UpgradeRequired | Failed
statusPersistence: pending | committed
lastPersistedStatusRevision: <revision>
reconcileProjection: <bounded>
```

Status persistence is asynchronous; the response need not wait for the status
write, so no uncommitted status is represented as durable. `Delete` returns an
event-only Deleted projection / not-found outcome. Expedited completion means
one pass reached a `Converged|Progressing|Blocked|UpgradeRequired|Failed`
disposition, not long-running external Ready. A timeout/cancel after commit
returns a typed committed-but-reconcile-pending response while the ordinary
queue continues. Priority quotas/fairness prevent starvation/DoS. Only
authorized UX mutations and core may request expedited mode; the admin
`resource reconcile` CLI action reuses the same lane.

## Request context

The caller cannot provide authenticated identity or authorization outcome.
d2b-bus/ComponentSession supplies the shared `AuthenticatedSubjectContext`.
The API derives request attributes from it plus:

- operation/idempotency/correlation/trace IDs;
- issue/deadline/cancellation;
- policy/API/config/controller revisions.

The resource payload supplies only target refs, desired body, preconditions,
pagination/watch filters, and method-specific options.

## ResourceType schemas and API exports

Every Provider resource binds:

- package/signature/trust identity;
- exported ResourceType names/versions;
- exact ResourceTypeSchema digests;
- controller descriptors and supported methods;
- maximum permission claims;
- service descriptors.

The Zone self resource binds accepted API exports and short ResourceType names.
Binding:

1. verifies Provider/package/schema signatures;
2. rejects a short ResourceType collision;
3. validates compatibility/fingerprint;
4. intersects permission claims with Zone policy;
5. installs the schema into api_schemas;
6. advances API catalog revision.

Providers cannot mutate api_schemas directly.

### ResourceApiBinding base-schema conformance (D089)

Each Provider `ResourceApiBinding` for a ResourceType declares and MUST
implement the exact ResourceType **base spec** and **base status** schema
version/fingerprint and pass the base lifecycle/status/error/finalizer
conformance suite. Binding:

1. records the declared base schema version/fingerprint and rejects a binding
   whose declared fingerprint does not match the installed ResourceType base
   schema;
2. records the Provider's signed `spec.provider` extension schema
   (`schemaId`/`schemaVersion`/`settings` JSON Schema) and the aligned
   `status.provider` extension schema, both deny-unknown, bounded, and digested;
3. records the Provider's signed **standard capability matrix** — the exact set
   of optional base capabilities it does and does not support.

A bound Provider MUST accept the canonical minimal valid base Spec for the
ResourceType. It MAY reject an optional base capability only when its signed
capability matrix declares that capability unsupported, returning a typed,
provider-neutral `unsupported-capability` result naming the base capability; it
MUST NOT ignore, reinterpret, rename, duplicate, or weaken the bounds of any
base field, and MUST NOT require `spec.provider` data for base-required
behavior. Generic API/CLI/controllers author and read only the base spec and
base status; `spec.provider`/`status.provider` are provider-scoped and never
required for a generic Get/List/Watch/UpdateSpec of the base.

`UpdateSpec` validates the canonical `spec.provider` envelope
(`{ schemaId, schemaVersion, settings }`) against the installed Provider named
by `spec.providerRef`: an unregistered/version-mismatched `schemaId`/
`schemaVersion` is rejected with `spec-provider-schema-invalid`, an unknown
field in `settings` is denied, a `settings` that restates/overrides a base field
is rejected with `spec-provider-shadow`, and an over-limit envelope is rejected
with the spec bounds error. The same validation runs at Nix build time.

### Endpoint resource resolution (D092)

`Endpoint` is a standard ResourceType and uses the ordinary
`Get`/`List`/`Watch`/`UpdateSpec`/`UpdateStatus`/`Delete`/`Upgrade` verbs and
Role/RoleBinding authorization. Its spec/status never carry a raw locator, so a
`Get` returns only closed class/transport/locality/purpose values and bounded
fingerprints. Resolving an `Endpoint` to a live transport/FD is **not** a public
API verb: Core/ProviderSupervisor performs it privately through the
EffectPort/LaunchTicket path when wiring an authorized consumer Process. A
consumer that is not authorized (by RoleBinding and the Endpoint's
`consumerPolicy`) to resolve an `Endpoint` is denied with `endpoint-resolve-denied`
and receives no locator. A producer restart bumps `status.update`/
`endpointGeneration`, firing the consumer's `dependency-changed` reconcile.

### ResourceExport and ResourceImport (D096)

`ResourceExport` and `ResourceImport` are standard ResourceTypes and use the
ordinary `Create`/`Get`/`List`/`Watch`/`UpdateSpec`/`UpdateStatus`/`Delete`/
`Upgrade` verbs plus Role/RoleBinding authorization. Cross-Zone advertisement
and import are still per-hop: native RBAC, the export's consumer-Zone policy,
the ZoneLink relationship, and the export capability ceiling must all allow the
operation.

For every exportable capability, the installed Provider descriptor supplies a
signed projection factory binding the exact qualified semantic/provider-neutral
`*Service` and `*Binding` types, allowed owner-Service backing ref types, allowed
Binding target ref types, the strict projection-Service schema/fingerprint, and
an aggregate semantic factory fingerprint. Admission fails closed when the
metadata is absent, unsigned, or mismatched. Provider/adapter identity is not
part of the semantic fingerprint: local `providerRef` independently selects the
implementation. Authored owner Services and Bindings may carry that
implementation's strict `spec.provider`; a Core-generated projection never
does. Its route derives from the signed local Provider descriptor,
`providerRef`, and ResourceImport record, and implementation observation may
appear only in `status.provider`.
`*State`, `stateType`, and `allowedStateTargetRefTypes` are not compatibility
aliases; strict schema admission rejects them.

Core enforces all of these rules before advertisement, lease creation, or local
projection:

1. `ResourceExport.resourceRef` resolves in the export Zone to the owner
   qualified `*Service` declared by the factory. A Device, Endpoint, Binding, or
   any other resource is rejected, even if it is a Service backing.
2. `ResourceImport` contains only a local `zoneLinkRef`, bounded `exportKey`,
   expected qualified Service type, and expected projection/factory
   fingerprints; it contains no remote Ref.
3. The advertised, expected, and locally installed factory values match exactly,
   the Provider accepts the canonical minimal base without `spec.provider`, and
   `requestedCapabilities` is within the export ceiling.
4. Core creates exactly one same-qualified-type local projection Service with
   `ownerRef: ResourceImport/<name>`, `providerRef`, and only semantic
   base/import fields. It rejects `spec.provider` and never creates a Device,
   Endpoint, or Binding projection.
5. An authored matching Binding is admitted only in the same Zone, with
   `serviceRef` targeting that Service and its consuming `Guest`, `User`, or
   `Zone` target type allowed by the factory. Binding spec contains desired
   intent only; observed realization is written only to status. Binding is
   non-exportable and cannot claim remote authority.

RoleBindings separately authorize export/import mutation, Service use, Binding
creation, and target use. Possessing a local projection-Service Ref does not
grant access to its remote authority or stream; the current lease/capability
check remains mandatory. Leases, ceremonies, sessions, and streams are internal
records rather than API resources.

Admission also rejects a semantic base schema/status, condition, error, or
fingerprint containing implementation-specific behavior or protocol fields.
The base `providerRef` is the sole opaque implementation selector; PipeWire,
CTAPHID, OTEL, and USBIP details belong only to that selected Provider's
registered strict extension, never the base API.

### Authority index admission (D097)

Core keeps a unique **authority index** keyed by `(Zone/scope, authorityClass,
opaqueKeyDigest)` derived from each authority Resource/Process's signed
`AuthorityDescriptor`. On `Create`/`UpdateSpec` of an authority-bearing Resource
(or launch of an authority owner Process), admission consults the index **before
any external effect** and rejects a second claimant for an `exactly-one`/
`zero-or-one` authority — or one exceeding a `bounded-many` bound — with the
typed `duplicateConflict` error naming the exact incumbent owner digest. The
`authorityKey` is internal and non-authorizing: it is never an authorization
principal and never appears as a locator in spec/status/audit. Authorization for
authority operations still flows through native Role/RoleBinding; the
`resource authorities` read surface (list authorities/holders and any conflict)
requires ordinary `get`/`list` verbs on the owning ResourceType.

## Native RBAC resources

### Bootstrap authorization

Before a reset/empty store has Role/RoleBinding resources, the Zone runtime has
one compiled, non-configurable bootstrap policy:

- exact subjects: Provider/system-core and Provider/system-minijail;
- exact local ComponentSession purposes/services;
- only store recovery, schema/config publication, initial Host/User/Provider/
  Role/RoleBinding creation, and first Process-controller launch verbs;
- no wildcard Provider/resource/runtime authority;
- no config field can widen it;
- normal stored RBAC governs all non-bootstrap work after publication.

Every bootstrap action remains structurally validated/audited. A different
subject, remote route, Provider generation, or method fails closed.

### Role

Role spec contains bounded rules:

```yaml
rules:
  - resourceTypes: [Process, Volume]
    verbs: [get, list, watch, create, update-spec]
    subresources: []
    resourceNames: []
    zones: [dev]
    executionRefs: [Host/host-system]
```

Rules use exact values; no implicit wildcard is granted. A reviewed explicit
wildcard may exist only for fixed core-controller roles and remains narrowed by
Zone/Provider/controller structural checks.

### RoleBinding

RoleBinding spec contains:

- `roleRef: Role/<name>`;
- exact subjects as canonical same-Zone refs, such as User, Provider, Host,
  Guest,
  or Process;
- optional authenticated external-principal selector generated by trusted
  enrollment/config;
- expiry/revocation;
- bounded scope narrowing.

A request body cannot select/override its subject.

## Authorization attributes

Every decision evaluates:

```text
Zone
subject
ResourceType
subresource/service
verb
resource name
executionRef/domain/userRef scope
Provider/controller generation
```

Resource verbs:

- get;
- list;
- watch;
- create;
- update-spec;
- update-status;
- update-metadata;
- update-finalizers;
- delete.

Runtime service verbs such as invoke/connect/attach/cancel/observe are mapped
through the same engine but are not resource mutations.

Native RBAC allow is necessary but not sufficient. Core structural checks also
enforce:

- correct Zone/session/route;
- installed Provider/API binding;
- ResourceType/controller/status-owner match;
- ownerRef/UID/ref integrity;
- Host/Guest executionRef/domain/user placement;
- generation/revision;
- budget/quota/cardinality;
- process/sandbox/resource policy;
- broker/FD/locality constraints.

Structural checks may narrow an allow and never override a deny.

## Status ownership

Each ResourceType schema identifies:

- spec writer roles/controllers;
- status owner Provider/controller;
- finalizer owners;
- fields core alone sets.

Only the current exact controller lease/generation may update status. Status
updates carry expected revision and observedGeneration. A Host/Guest/link/
controller disconnect cannot write success; status becomes Unknown through the
authorized observer/core rule.

Status is the default durable observation and recovery surface for bounded
non-secret operational state (D087) and has a frozen three-layer shape (D088):
the universal `ResourceStatus` base, the ResourceType-common `status.resource`
object, and an optional Provider-specific `status.provider` extension. The
owning controller writes all present layers in one `UpdateStatus` mutation with
a single expected revision; the layers never diverge across separate writes.

`UpdateStatus` is bounded and validated per layer: the resource store rejects a
status replacement whose total canonical serialized size exceeds 64 KiB, whose
`status.resource` typed detail exceeds 32 KiB, whose `status.provider.details`
exceeds 32 KiB, or whose condition/list/map cardinality exceeds the frozen
limits (see `ADR-046-resource-object-model` § Status bounds) with a typed
`status-oversize` error. A `status.provider` whose `providerRef`/`schemaId`/
`schemaVersion` is not registered for the installed Provider, or whose `details`
carries an unknown field, is rejected with `status-provider-schema-invalid`; the
write changes nothing and the caller re-reads and retries. A `status.provider`
that restates, overrides, or duplicates a universal or `status.resource` field
is rejected with `status-provider-overlap`.

Generic API/CLI/controllers request and depend on a **base-only projection**
(the universal base plus `status.resource`); the optional `status.provider`
extension is ignored by base-only consumers and never required for a Watch or
Get to succeed. Status never carries secrets, authority-conferring handles,
private path/argv/environment/PID/unit data, terminal/clipboard/CTAP bytes, raw
cloud error bodies, or high-frequency streams; those stay in their owning
surfaces. Controllers write status only on a material change in observed state,
and never treat status as a host-mutation or repair authority.

## OwnerRef authorization

Create/reparent requires:

- permission on the child;
- get/use permission on the owner;
- scope compatibility;
- no cycle/depth violation;
- permission for both old/new owners on reparent.

A child mutation's owner hint is generated by core after commit and cannot be
suppressed by the child.

## Authorization cache

Positive decisions may be cached only under:

- exact subject;
- exact authorization attributes;
- Role/RoleBinding/Provider/API/Zone policy revisions;
- short expiry.

Relevant resource revisions invalidate caches immediately after durable commit.
Denials may be cached briefly but never become allows.

If authorization/store state is unavailable:

- no new resource/runtime operation is admitted;
- admitted bounded operations retain their original context until deadline;
- long-lived streams require short authorization leases and close on expiry;
- local emergency disable remains available through the fixed out-of-band
  safety path.

## Parent/child Zone access

A parent:

- authenticates over a ZoneLink ComponentSession;
- maps to a child-local subject/RoleBinding;
- calls the child d2b.resource.v3 service;
- receives only child-authorized data/status.

The child commits to its own store. The parent receives no database handle,
credential, token, or cross-Zone ResourceRef.

A disconnected parent may record a local ZoneLink intent but cannot claim the
child resource changed. On reconnect the child reauthorizes and applies/rejects
against current revision.

## Limits

The API spec freezes bounds for:

- request/response/batch bytes;
- batch mutation/resource count;
- ResourceType/name/ref depth/length;
- list page/filter count;
- watch count/filter complexity/rate/credit;
- Role rules/bindings/subjects;
- conditions/status/error strings;
- finalizers/owner depth;
- concurrent reads/writes per principal/controller;
- deadlines and retry-after.

Over-limit input is rejected before redb mutation or Provider invocation.

## Errors

Stable classes include:

- resource-not-found;
- resource-already-exists;
- resource-conflict;
- resource-schema-invalid;
- resource-ref-invalid;
- resource-owner-cycle;
- resource-owner-depth;
- resource-finalizer-denied;
- resource-provider-unavailable;
- resource-controller-mismatch;
- resource-status-owner-mismatch;
- status-oversize;
- status-provider-schema-invalid;
- status-provider-overlap;
- spec-provider-schema-invalid;
- spec-provider-shadow;
- unsupported-capability;
- expedited-not-authorized;
- expedited-quota-exceeded;
- expedited-reconcile-pending;
- upgrade-required;
- endpoint-resolve-denied;
- authorization-denied;
- revision-expired;
- backpressure;
- timeout;
- cancelled;
- resource-plane-unavailable;
- internal-integrity-failure.

Error messages are bounded/redacted. Conflict returns current revision but does
not return an unauthorized resource body.

## Audit

Audit records:

- subject/Zone;
- ResourceType/name or bounded digest;
- verb/subresource;
- expected/current/result revision;
- authorization decision and policy revisions;
- operation/correlation;
- fixed outcome/error/retry class.

It excludes spec/status payloads, Provider diagnostics, host paths, credentials,
process data, and terminal bytes.

## Current-code fit

| Item | Treatment |
| --- | --- |
| Current anchor | Public daemon/broker seqpacket auth; `d2b-daemon-access` admission types; `d2b-realm-router` principal/capability/idempotency checks; Realm access resolver; strict DTOs |
| Evidence class | Local daemon auth is reachable; daemon-access/Realm peer abstractions are partly unwired; native RBAC/API are ADR-only |
| Behavior retained | SO_PEERCRED/local identity, typed denials, positive capabilities, no relay-to-local auth, strict bounds/unknown-field rejection |
| Required delta | Entire resource API, Provider API schemas/bindings, Role/RoleBinding engine, status ownership, parent resource routing |
| Reuse path | Extract exact admission/error/id/ref validators and router authorization derivation |
| Replacement/deletion | Old public wire remains until CLI/controllers consume new services |
| Feasibility proof | Multi-process local/vsock/Zone resource calls, immediate revocation, conflict/no-leak tests |
| Future owner | Work items below |

## Implementation work items

### ADR046-api-001

| Field | Value |
| --- | --- |
| Dependency/owner | W0; resource API integrator |
| Current source | `packages/d2b-contracts/src/public_wire.rs`, `broker_wire.rs`; `d2b-daemon-access/src/lib.rs`; `d2b-realm-router/src/lib.rs` |
| Reuse action | extract and adapt |
| Destination | `packages/d2b-contracts/proto/d2b-resource-v3.proto`, `packages/d2b-resource-api/src/service.rs`, `client.rs` |
| Detailed design | Async methods, contexts, preconditions, limits, errors, status/finalizer separation, batch API |
| Integration | d2b-bus exact service → Zone auth → redb actor |
| Data migration | None; v3 clean break |
| Validation | Protocol vectors; malformed/oversize/conflict/status-owner tests |
| Removal proof | Old command/resource-equivalent paths removed only per integration wave |

### ADR046-api-002

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-api-001; authorization integrator |
| Current source | `d2bd` public admission; `d2b-daemon-access` policy evidence; `d2b-realm-core/src/access.rs`, `audit.rs` |
| Reuse action | adapt |
| Destination | `packages/d2b-resource-api/src/authz.rs`, `packages/d2b-core-controller/src/rbac.rs` |
| Detailed design | Role/RoleBinding schemas/evaluator/cache/revision invalidation, ComponentSession subject mapping, parent Zone access |
| Integration | Every resource/runtime method invokes one native evaluator before structural checks |
| Data migration | Generate initial Roles/Bindings from Nix v3 config |
| Validation | Decision matrix/property tests; revocation/cache/outage/parent-child tests |
| Removal proof | Legacy auth remains until every v3 route is covered |
