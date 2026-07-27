# ADR 0046 resource API and authorization

| Field | Value |
| --- | --- |
| Spec ID | `ADR-046-resource-api-and-authorization` |
| Parent | ADR 0046 |
| Status | Accepted |
| Version | 2 |
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
3. records the Provider's signed **standard capability matrix** - the exact set
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
Endpoint `visibility` is the closed enum `owner | provider | zone`; examples and
schemas use no other token. It is only a coarse ceiling. `consumerPolicy`
provides finer exact consumer bounds, and both visibility and consumer policy
must allow resolution.

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
`zero-or-one` authority - or one exceeding a `bounded-many` bound - with the
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

- the table is a `&'static` array in `packages/d2b-resource-api/src/authz.rs`
  and accepts no configuration, Nix, environment, or API input;
- its two phases are derived only from `store_meta.policy_revision` and the
  presence of the bootstrap Provider rows in `type_index`;
- exact subjects, local evidence, purpose, service, generations, and transport
  binding must match the compiled tuple;
- the exact method/type rows are those frozen by D105, with Provider creation
  narrowed to `system-core`/`system-minijail` and Zone creation to the compiled
  Zone name;
- UpdateSpec, UpdateMetadata, UpdateFinalizers, Delete, CommitBatch, Upgrade,
  and expedited waitForReconcile are always denied during bootstrap.

The first policy publication advances `policy_revision` from 0 to 1 in the same
redb transaction that installs the initial Role/RoleBinding set. The transition
is one-way; the table is never consulted at revision 1 or later, and reset
creates a new store identity rather than clearing the marker. Every bootstrap
action remains structurally validated/audited. A different subject, remote
route, Provider generation, or method fails closed.

### Admitted mutation boundary

Mutation admission can originate only from a successful native authorization
evaluation. The resulting sealed evidence carries the admitted mutation, exact
authorization attributes, policy/API-catalog/active-configuration/controller
revisions, request identity, and deadline. The native authorizer and store
identity are consumed into one private checked store, which verifies the
evaluator authority and exact store identity before passing a
`VerifiedMutation` to the backend. A caller therefore cannot mint admission
without a real allow or replay evidence against another store.

The seal ends at the backend boundary; it does not sandbox or attest the
backend implementation. A registered backend is trusted to mutate only from
the supplied `VerifiedMutation`, compare its captured revisions against live
`store_meta` in the same write transaction, preserve all structural and
atomicity checks, and expose no independent mutation path. Any mismatch aborts
without mutation as `authorization-denied`; the store never evaluates RBAC or
auto-retries, and the client must reissue through the evaluator. A production
backend is admitted to the trusted computing base only after security review
and conformance tests cover these obligations.

### Role

Role spec contains bounded rules:

```yaml
rules:
  - resourceTypes: [Process, Volume]
    verbs: [get, list, watch, create, update-spec]
    sessionVerbs: []
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
- exact subjects as canonical same-Zone refs: Zone, User, Provider, Host, Guest,
  or Process; a trusted external-principal selector binds the exact adjacent
  enrolled `Zone/*` transport subject for a core-generated relay binding;
- optional authenticated external-principal selector generated by trusted
  enrollment/config;
- bounded scope narrowing.

RoleBinding has no expiry field. Revocation uses normal spec update or deletion.
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

Resource verbs are the exact closed set:

- get;
- list;
- watch;
- create;
- update-spec;
- update-status;
- update-metadata;
- update-finalizers;
- delete;
- use-credential; and
- admin-credential.

`use-credential` is valid only for `Credential` rules with one or more exact
operation subresources from the Credential contract. `admin-credential` is
valid only for `Credential` rules with exact `create`, `update-spec`, or
`delete` subresources and is supplemental to the matching ordinary CRUD verb;
it grants no CRUD action by itself.

Runtime/session verbs are the exact closed set `connect`, `invoke`,
`open-stream`, `relay`, `attach`, `cancel`, `observe`, `audit-export`, and
`support-bundle`. They are mapped through the same engine but are not resource
mutations. `audit-export` binds only
`d2b.audit.v3.AuditService/Export`; `support-bundle` binds only
`d2b.support.v3.SupportService/GenerateBundle`. Both are admin-only,
session-only grants and imply no `get`, `list`, or other resource authority.
`relay` permits only an already-authenticated ZoneLink/transport subject to
forward an already-admitted invocation or stream to one route-selected next
hop. It grants no resource CRUD, identity mapping, capability widening,
attachment, credential, or local lifecycle authority.

Every forwarding hop evaluates two independent permissions: `relay` for the
authenticated adjacent-Zone transport subject and the forwarded operation's
target verb under the exact local Role/RoleBinding scope. Named methods carry
one immutable resource name. Nameless `List` and `Watch` carry no synthetic
name: their exact ResourceType, non-empty authorized `resourceNames` allowlist,
and bounded filters are evaluated as a set, and every hop preserves those
filters byte-for-byte. A filter that could select a name outside the local
intersection is denied rather than widened. The final target evaluates the
same target verb and selector again. Neither permission implies the other; a
missing grant or unavailable policy state fails closed. Relay-bearing
Roles/Bindings are core-generated and ZoneLink-scoped by default. Admission
rejects wildcard, self-asserted, Provider-authored, or ordinary
operator-authored relay grants unless an already-authorized local administrator
explicitly permits the exact bounded grant through durable admin policy.

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
- when it forwards again, requires both `relay` and the requested target verb
  for that authenticated adjacent-Zone subject;
- calls the child d2b.resource.v3 service;
- receives only child-authorized data/status.

The child commits to its own store. The parent receives no database handle,
credential, token, or cross-Zone ResourceRef.

A disconnected child may record a bounded outbound intent in its child-local
ZoneLink but cannot claim the parent resource changed. On reconnect the parent
reauthorizes and applies/rejects against current revision.

## Limits

Every bound below is normative and is enforced before any redb mutation or
Provider invocation. A rejection is total: no partial write, no Provider call,
and no revision allocation occurs. The Rust constants live in
`d2b-contracts::v3` and a policy lint asserts each declared value equals the
number in this table. Raising a bound is a compatible change and requires a new
decision register entry; lowering one breaks live Providers and is a breaking
change.

Each value is derived from an already frozen v3 bound rather than carried over
from a v2-era default. The derivation anchors are the frozen ComponentSession
transport ceilings in `ADR-046-security-and-threat-model` (1 MiB logical
message, 128 active named streams per session, 256 KiB queued plaintext per
named stream and 4 MiB aggregate in each direction, 64 request attachments,
`MAX_REQUEST_LIFETIME_MS` of 900,000 ms, 64 in-flight Provider agent
dispatches), the frozen status caps in `ADR-046-resource-object-model` (64 KiB
total status, 32 KiB per typed layer), and the frozen bounds in the decision
register (D073 Role and RoleBinding shape, D112 and D113 admission bounds, D117
store pool and queue capacities).

### Request, response, and batch

| Bound | Limit | Derived from | Over-limit class |
| --- | --- | --- | --- |
| Request canonical bytes | 512 KiB | Half the frozen 1 MiB transport logical-message ceiling, reserving the remainder for record and fragment headers, AEAD tags, up to 64 attachment descriptors, ttrpc control metadata, and the typed protobuf fields wrapping the canonical-JSON carrier | `resource-schema-invalid` |
| Response canonical bytes | 512 KiB | Same ceiling. A response that would exceed it is truncated at an envelope boundary and returns a page cursor rather than failing, so the byte cap and not the page count is the binding pagination constraint | `resource-schema-invalid` |
| One resource envelope | 256 KiB | Status is capped at 64 KiB, leaving 192 KiB for `metadata` and `spec`; two full envelopes still fit inside one response | `resource-schema-invalid` |
| Mutations in one `CommitBatch` | 32 | Twice the maximum group-commit batch of 16, so one batch never needs more than two commit groups | `resource-schema-invalid` |
| Distinct resources touched by one batch | 32 | One mutation targets exactly one resource | `resource-schema-invalid` |

### Identifier and reference lengths

| Bound | Limit | Derived from | Over-limit class |
| --- | --- | --- | --- |
| ResourceType local `<Type>` segment | 63 bytes | DNS-label shape; grammar `^[A-Z][A-Za-z0-9]{0,62}$` | `resource-schema-invalid` |
| ResourceType `<provider-name>` segment | 63 bytes | Resource-name grammar `^[a-z][a-z0-9-]*$` | `resource-schema-invalid` |
| Canonical `ResourceTypeName` | 63 bytes standard, 137 bytes qualified | Asserted from the two segment caps and the 11-byte `.d2bus.org.` separator, not separately configured | `resource-schema-invalid` |
| `ResourceName`, `ZoneId` | 63 bytes | DNS-label shape | `resource-schema-invalid` |
| Canonical `ResourceRef` string | 201 bytes | Asserted from 137 plus separator plus 63 | `resource-ref-invalid` |
| Reference nesting depth | 1 | A ref is exactly one type and one name and cannot nest, so owner-chain depth is the only depth bound in the resource plane | `resource-ref-invalid` |

### List and pagination

| Bound | Limit | Derived from | Over-limit class |
| --- | --- | --- | --- |
| ResourceTypes per request | 16 | The 16 ResourceTypes per Role rule (D073), so a List can never span more types than one rule can authorize | `resource-schema-invalid` |
| Page size | 500 maximum, 100 default | Secondary to the 512 KiB response cap, which truncates first whenever envelopes are large | `resource-schema-invalid` |
| Exact-match filters per request | 8 | Each filter is an indexed lookup inside one read transaction bounded to 250 ms (D117) | `resource-schema-invalid` |
| Values per filter | 64 | The 64 `resourceNames` per Role rule (D073) | `resource-schema-invalid` |
| Page cursor | 256 bytes, opaque | Bound to the snapshot revision and the request filter digest; a cursor presented with different filters is rejected, and an unreadable snapshot returns `revision-expired` | `resource-schema-invalid` |

### Watch

Watch memory is bounded by the aggregate queue ceiling, not by the watch count.
At the per-watch ceiling, 256 watches would queue 64 MiB, which is the entire
Zone idle-RSS budget, so the 4 MiB aggregate is the binding limit and the
per-watch value is only a fairness ceiling.

| Bound | Limit | Derived from | Over-limit class |
| --- | --- | --- | --- |
| Watches per session | 32 | One quarter of the frozen 128 active named streams per session, leaving stream headroom for non-watch work | `resource-schema-invalid` |
| Watches per Zone | 256 | 2.5 times the 100 live watches in the pinned performance fixture | `backpressure` |
| ResourceTypes per watch | 16 | Same rule as List | `resource-schema-invalid` |
| Filters per watch | 8 | Same rule as List | `resource-schema-invalid` |
| Outstanding credits | 1024 maximum, 128 default | Exhausted credits stall delivery and apply backpressure; an event is never dropped | `resource-schema-invalid` |
| Queued bytes per watch | 256 KiB | The frozen per-named-stream queue ceiling; a watch is carried on a named stream, so this is the transport's own cap and not a second budget | `backpressure` |
| Aggregate queued watch bytes per Zone | 4 MiB | The frozen aggregate named-stream queue ceiling, shared with named streams so watch and stream backpressure use one accounting | `backpressure` |

### Metadata, owners, and finalizers

| Bound | Limit | Derived from | Over-limit class |
| --- | --- | --- | --- |
| Owner-chain depth | 8 | The deepest chain any spec describes is about five resources; the walk runs inside the write transaction | `resource-owner-depth` |
| Resources visited per owner-hint propagation pass | 64 | Keeps hint fan-out bounded per commit | `backpressure` |
| Finalizers per resource | 8 | Unique and canonically sorted for the digest | `resource-schema-invalid` |
| Finalizer ID | 128 bytes | Admitted forms are `core.<name>` and `<namespace>.d2bus.org/<name>`, each segment 1 to 63 bytes | `resource-schema-invalid` |
| Labels, annotations | 32 each | Neither participates in authorization | `resource-schema-invalid` |
| Label or annotation key | 64 bytes | Printable ASCII; the optional `<namespace>/` prefix counts toward D101's canonical JSON object-key ceiling | `resource-schema-invalid` |
| Label value | 256 bytes | UTF-8, control-character free | `resource-schema-invalid` |
| Annotation value, aggregate annotations | 4 KiB, 16 KiB | Kept well inside the 256 KiB envelope cap | `resource-schema-invalid` |

Status bounds (64 KiB total, 32 KiB per typed layer, 32 conditions, 64 entries
per list or map, 4 KiB per bounded string) are frozen by
`ADR-046-resource-object-model` and rejected as `status-oversize`. Role and
RoleBinding bounds (32 rules; per rule 16 ResourceTypes, 16 verbs, 64 resource
names, 32 `executionRefs`; 128 subjects per binding) are frozen by D073 and
rejected as `resource-schema-invalid`.

### Concurrency, deadlines, and retry

| Bound | Limit | Derived from | Over-limit class |
| --- | --- | --- | --- |
| Concurrent reads per principal | 8 | The store runs a 4-thread read pool with at most 16 concurrent read transactions (D117), so two principals can saturate the pool and a third queues rather than starves | `backpressure` |
| Concurrent writes per principal | 4 | The store has a single writer with a group-commit batch of 16, so four principals fill one batch | `backpressure` |
| Concurrent in-flight Provider dispatches | 64 | The frozen semaphore-guarded Provider agent ceiling | `backpressure` |
| Request deadline | 900,000 ms maximum, 30,000 ms default | The frozen `MAX_REQUEST_LIFETIME_MS` | `resource-schema-invalid` |
| Expedited `waitForReconcile` deadline | 10,000 ms | Short enough that the priority lane cannot starve the ordinary queue | `expedited-quota-exceeded` |
| Expedited requests in flight per Zone | 8 | Same reason | `expedited-quota-exceeded` |
| `retryAfterMs` | 1 to 86,400,000 ms | The 24 h EphemeralProcess failed-TTL ceiling; `0` is rejected so absence has one spelling | `resource-schema-invalid` |
| Revision-log compaction trigger | 32 MiB, 100,000 entries, or 24 h, whichever is first | Compaction advances the durable floor in bounded write transactions deleting at most 1,000 batches each, so it never blocks the writer | not applicable |

## Errors

The stable class set is **closed** at exactly these 31 classes. Adding,
removing, or renaming one is a breaking wire change and requires a decision
register entry; a Provider MUST NOT invent a class.

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
- relay-denied;
- role-relay-grant-restricted;
- authorization-denied;
- revision-expired;
- backpressure;
- timeout;
- cancelled;
- resource-plane-unavailable;
- internal-integrity-failure.

Over-limit input maps onto this closed set and never introduces a new class:
byte, count, and length violations are `resource-schema-invalid`; reference
shape violations are `resource-ref-invalid`; owner-chain depth is
`resource-owner-depth`; status size is `status-oversize`; concurrency, queue,
and per-Zone capacity exhaustion is `backpressure`; expedited-lane exhaustion
is `expedited-quota-exceeded`.

An error is carried as a typed value with a closed `kind`, an optional
`currentRevision`, an optional `retryAfterMs`, a closed retry class, and a
`reason` string bounded at 512 bytes. The `reason` is UTF-8, control-character
free, and redacted: it never echoes caller input, a filesystem or store path,
argv, credential material, or an unauthorized resource body. Conflict returns
the current revision but does not return an unauthorized resource body, and
`currentRevision` is populated only for `resource-conflict` and
`revision-expired` and only when the caller is authorized to read that
revision.

The Rust homes and mapping boundary are frozen by D111.
`ResourceErrorKind`/`ResourceError` live in
`packages/d2b-contracts/src/v3/error.rs`;
`StoreErrorKind`/`StoreError` live in
`packages/d2b-resource-store/src/error.rs`; and the total one-way store-to-API
mapping lives in `packages/d2b-resource-api/src/error.rs`. The resource set is
the exact 31 strings above. The store set adds only
`store-integrity-failure`, `store-backpressure`, and `store-quarantined`.

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
| Current anchor | Native ResourceService DTOs, methods, errors, Role/RoleBinding evaluation, service-to-store calls, and an authenticated ttrpc adapter exist in `d2b-contracts` and `d2b-resource-api`. The adapter can register the complete ttrpc service for an already-authenticated session, but no production d2b-bus or Zone path dispatches it. Existing anchors remain public daemon/broker seqpacket auth, `d2b-daemon-access` admission types, `d2b-realm-router` principal/capability/idempotency checks, Realm access resolver, and strict DTOs |
| Evidence class | Resource API, native RBAC, service-to-store wiring, and the ttrpc adapter are `implemented-but-unwired`; production bus dispatch, Zone wiring, and a production store backend are absent. The adapter's existence is not production reachability. Local daemon auth is reachable, while daemon-access/Realm peer abstractions remain partly unwired |
| Behavior retained | SO_PEERCRED/local identity, typed denials, positive capabilities, no relay-to-local auth, strict bounds/unknown-field rejection |
| Required delta | Dispatch the existing ttrpc adapter from d2b-bus, connect a Zone runtime and production backend, and implement Provider API schemas/bindings, status ownership, and parent resource routing |
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
| Reuse action | adapt |
| Destination | `packages/d2b-contracts/proto/d2b-resource-v3.proto`; `packages/d2b-contracts/src/generated/d2b_resource_v3.rs`; `packages/d2b-resource-api/src/generated/`, `service.rs`, `client.rs`, `error.rs`; `packages/xtask/src/main.rs` codegen commands |
| Detailed design | Freeze the service as `d2b.resource.v3.ResourceService` and the D100 typed message set with one canonical-JSON bytes carrier. `xtask gen-resource-proto` emits message-only pure-Rust bindings into `d2b-contracts` from a service-stripped proto; `xtask gen-resource-ttrpc` emits async service/client bindings into `d2b-resource-api`. No `build.rs`, `google.protobuf.Any`, dynamically typed `oneof`, or domain error in transport status. Implement async methods, contexts, admitted-mutation preconditions, D112 contract constants, typed resource errors, status/finalizer separation, and batch API. Primary reuse disposition: `adapt`. Preserved source-plan detail: extract and adapt. |
| Integration | d2b-bus exact service → Zone auth → redb actor |
| Data migration | None; v3 clean break |
| Validation | Golden encoding/field-number vectors; generated-file drift tests for both outputs and byte-identical existing guest-proto output; no-build-script/Any/dynamic-oneof/transport-domain-error policy tests; D112 constant assertions; malformed/oversize/conflict/status-owner tests |
| Removal proof | Old command/resource-equivalent paths removed only per integration wave |
| Implementation state | Merged |
| Evidence | All destinations are present, including generated protobuf/ttrpc bindings and `packages/d2b-resource-api/src/{service,client,error,adapter}.rs`; contract, generated-drift, malformed-input, batch, and adapter tests are committed. |

### ADR046-api-002

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-api-001; authorization integrator |
| Current source | `d2bd` public admission; `d2b-daemon-access` policy evidence; `d2b-realm-core/src/access.rs`, `audit.rs` |
| Reuse action | adapt |
| Destination | `packages/d2b-resource-api/src/authz.rs`, `packages/d2b-core-controller/src/rbac.rs` |
| Detailed design | `packages/d2b-resource-api/src/authz.rs` defines ComponentSession subject mapping, parent-Zone access, canonical resource/session verb admission including ZoneLink-scoped `relay`, and independent per-hop relay plus target-verb checks. The W0 `packages/d2b-core-controller/src/rbac.rs` surface is limited to the stored-policy evaluator skeleton, cache keying, and revision invalidation; it defines no concrete Role or RoleBinding schema, which lands with the Zone-control work items in W5. |
| Integration | Every resource/runtime method invokes one native evaluator before structural checks |
| Data migration | Generate initial Roles/Bindings from Nix v3 config |
| Validation | W0 evaluator-skeleton, cache-key, revision-invalidation, subject-mapping, relay-origin/scope, relay-missing, and target-verb-missing fail-closed tests; concrete Role/RoleBinding schema and revocation vectors land with the W5 Zone-control work items |
| Removal proof | Legacy auth remains until every v3 route is covered |
| Implementation state | Merged |
| Evidence | Both destinations are present: `packages/d2b-resource-api/src/authz.rs` and `packages/d2b-core-controller/src/rbac.rs`, with native Role/RoleBinding evaluation, bootstrap, relay, cache, and revision tests. |
