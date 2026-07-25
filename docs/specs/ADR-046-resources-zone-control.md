# ADR 0046 ResourceTypes: Zone, ZoneLink, Provider, Role, RoleBinding, Quota, EmergencyPolicy, ResourceExport, and ResourceImport

| Field | Value |
| --- | --- |
| Spec ID | `ADR-046-resources-zone-control` |
| Parent | ADR 0046 |
| Status | Accepted |
| Version | 1 |
| Baseline | `b5ddbed67867d9244bf33390868101bd9b053e49` |
| Normative | Yes |
| Owners | `d2b-contracts` (schemas), `d2b-core-controller` (handlers), Zone runtime, Nix resource compiler |
| Depends on | `ADR-046-decision-register`, `ADR-046-terminology-and-identities`, `ADR-046-resource-object-model`, `ADR-046-resource-api-and-authorization`, `ADR-046-resource-store-redb`, `ADR-046-core-controllers`, `ADR-046-provider-model-and-packaging`, `ADR-046-resource-reconciliation` |
| Supersedes | None |

## 1. Scope

This spec defines the complete normative contract for the nine core control
ResourceTypes: `Zone`, `ZoneLink`, `Provider`, `Role`, `RoleBinding`, `Quota`,
`EmergencyPolicy`, `ResourceExport`, and `ResourceImport` (the last two added by
D096 for cross-Zone sharing of scarce singleton resources).

For each type it provides:

- complete `metadata`/`spec`/`status` field schemas with bounds and defaults;
- provider/controller ownership;
- phase/condition/outcome transitions and timestamps;
- ownerRef/finalizer/deletion behavior;
- native RBAC and bootstrap authorization;
- Zone self-resource and parent/child ZoneLink semantics;
- core controller algorithms and async reconciliation triggers;
- security, audit, OTEL, and error requirements;
- normative Nix authoring examples;
- conformance requirements and exact tests;
- current-code fit tables;
- implementation work items.

This revision resolves all design choices for the covered ResourceTypes. New
design decisions are tracked in
[`docs/specs/ADR-046-decision-register.md`](ADR-046-decision-register.md).

All common resource fields (universal envelope, generation, revision, UID,
condition and outcome shapes, phase values, deletion protocol) are governed by
[`ADR-046-resource-object-model`](ADR-046-resource-object-model.md). This spec
extends that contract for these nine types only.

---

## 2. Zone

### 2.1 Role

`Zone` is the self-resource of a Zone store. Every Zone store contains exactly
one authoritative:

```text
Zone/<zone-name>
```

`<zone-name>` must equal the redb `store_meta.zone_name` key and the resource's
own `metadata.zone` field. There is no cross-Zone resource reference; a
`Zone/<name>` resource resolves only inside the store where `<name>` equals
the store's own `zone_name`. The Zone resource is not replicated, shadowed, or
referenced from other Zones.

**Controller/owner**: The `configuration publication` handler of the fixed
core-controller process (Provider/system-core) owns Zone reconciliation. No
other controller may update Zone spec or status.

### 2.2 Metadata

Zone metadata follows the common envelope. Additional Zone-specific rules:

| Field | Rule |
| --- | --- |
| `metadata.name` | Must equal `store_meta.zone_name`; validated at every open/upgrade; immutable after first commit |
| `metadata.zone` | Must equal `metadata.name` for the self resource |
| `metadata.uid` | Immutable; equals `store_meta.zone_uid`; generated once at store creation |
| `metadata.generation` | Starts at 1; increments only on spec change |
| `metadata.ownerRef` | Must be `null`; Zone cannot be owned by another resource |
| `metadata.finalizers` | Core may add `core.zone-drain` finalizer during shutdown/reset; no other controller may add a Zone finalizer |
| `metadata.deletionRequestedAt` | Set only by deliberate reset/shutdown; normal operations never delete Zone |

### 2.3 Spec

`Zone.spec` is `{}` - an empty object. Zone identity, API catalog, policy
revision, and configuration revision are entirely derived from the store
metadata and other installed resources (Provider, Role, RoleBinding, Quota,
EmergencyPolicy). The Zone resource is a pure identity anchor.

**D091 update policy.** The universal base spec carries `spec.updatePolicy` for
every Zone-control ResourceType (`Zone`, `ZoneLink`, `Provider`, `Role`,
`RoleBinding`, `Quota`, and `EmergencyPolicy`): disruptive changes default to
manual, while automatic non-disruptive upgrades are permitted by policy. A
`spec.provider` extension MAY add provider-specific knobs, but MUST NOT bypass
or weaken base `spec.updatePolicy`.

**D090 expedited reconcile.** Authorized `Create`, `UpdateSpec`, and `Delete`
calls on these ResourceTypes MAY set `waitForReconcile`. Under one mutation
ticket, `operationId`, and deadline, Core admission and the reserved-revision
redb commit run in parallel with the owning controller's preflight/plan, but the
controller MUST NOT perform external effects, finalizer release, or status
mutation until Core supplies `CommittedRevisionProof {resourceUid, generation,
revision, operationId}`; DB failure aborts with no effect. The API returns the
committed object plus one-pass projected layered status, `disposition`
(`Converged|Progressing|Blocked|UpgradeRequired|Failed`), `statusPersistence`
(`pending|committed`), and the last persisted status revision. The durable
commit is never rolled back on reconcile timeout or failure; effect idempotency
keys derive from `(UID,generation,revision,operationId)`, and the expedited pass
uses a bounded priority lane in the same per-resource single-flight.

```yaml
apiVersion: resources.d2bus.org/v3
type: Zone
metadata:
  name: dev
  zone: dev
  uid: <store-generated immutable>
  generation: 1
  revision: <opaque>
  ownerRef: null
  finalizers: []
  deletionRequestedAt: null
  createdAt: 2026-07-22T00:00:00.000Z
  updatedAt: 2026-07-22T00:00:00.000Z
spec: {}
status:
  observedGeneration: 1
  phase: Ready
  conditions: []
  lastReconciledAt: 2026-07-22T00:00:01.000Z
  startedAt: 2026-07-22T00:00:00.000Z
  completedAt: null
  outcome: null
  resource: {}
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

### 2.4 Status

#### Three-layer status shape (D088)

D088 freezes `Zone` status as three layers. The universal `ResourceStatus`
base (Layer 1) lives at top-level `status` and owns `observedGeneration`,
`phase`, `conditions`, `lastReconciledAt`, `startedAt`, `completedAt`, and
bounded `outcome`. The `Zone`-specific status fields documented in this
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

**D091 update currency.** Every Zone-control ResourceType includes universal
`status.update` with `state`
(`Current|UpdateAvailable|UpgradeRequired|Upgrading|Blocked|Unknown`), `reasons`
(`CoreGenerationChanged|ProviderGenerationChanged|ArtifactChanged|ImageOrSystemGenerationChanged|SpecChanged|DependencyChanged|SecurityPolicyChanged`),
bounded non-secret observed/target generation and digest IDs, `disruption`
(`None|Reload|Restart|Recycle|Replace`), `preserveState`, optional
`operationId`, `lastAssessedAt`, and bounded/truncated `owned:{count,refs}` and
`dependencies:{count,refs}`. ResourceType-specific currency refinements,
including Quota, EmergencyPolicy, Role, and RoleBinding currency, live in
`status.resource` and never in `status.provider`; controllers set
`status.update` via `assess_update` on core/provider/artifact/spec/dependency/
security-policy triggers and MUST report `UpgradeRequired` for disruptive
changes rather than applying them in place. A Zone-level rollout trigger reports
reason `CoreGenerationChanged`; Core aggregates self, owned, and dependency
currency for list/get.

Zone.status extends common status with:

| Field | Type | Rules |
| --- | --- | --- |
| `apiCatalogRevision` | u64 | Current Zone api_catalog revision from store_meta |
| `policyRevision` | u64 | Current authorization policy revision from store_meta |
| `configurationRevision` | u64 | Active configuration revision from store_meta |
| `coreControllerPhase` | phase enum | Aggregate phase of all mandatory core handlers |
| `handlers` | bounded list | Per-handler name/phase/lastReconciledAt (no resource names, no secrets) |
| `installedProviderCount` | u32 | Count of installed (non-Deleted) Provider resources |
| `readyProviderCount` | u32 | Count of Ready Provider resources |
| `totalResourceCount` | u32 | Total non-deleted resource count |
| `activeConfigurationGeneration` | u64 | Generation number of the active Nix-authored resource bundle |
| `generationCleanupPending` | bool | True while prior-generation config-owned resources are completing deletion |
| `cleanupPendingCount` | u32 | Count of config-owned resources from prior generations awaiting deletion |

`coreControllerPhase` is the stricter of all mandatory handler phases. Optional
handlers (ZoneLink maintenance, backup cleanup) may report Degraded without
changing `coreControllerPhase` to Failed.

`handlers` entries contain only: handler name (stable closed enum), phase, and
lastReconciledAt. No resource names, counts by type, provider diagnostics, or
store paths appear here.

### 2.5 Phase and conditions

| Phase | Meaning |
| --- | --- |
| `Pending` | Store open; core handlers initializing |
| `Ready` | All mandatory handlers Ready; Zone fully operational |
| `Degraded` | Core handlers Ready but optional handlers impaired, or config-owned resources are awaiting deletion (`GenerationCleanupPending=True`) |
| `Failed` | One or more mandatory handlers in Failed; Zone non-operational |
| `Unknown` | Store unavailable or Zone runtime cannot determine state |
| `Deleted` | Final revision event emitted; store closing/closed |

Zone never reaches `Succeeded` (it is long-lived, not a one-shot resource).

Closed condition types for Zone:

| Condition type | Meaning |
| --- | --- |
| `StoreReady` | Underlying redb database is open and healthy |
| `ConfigurationCurrent` | Active configuration generation is current |
| `ApiCatalogReady` | API catalog bound and valid |
| `AuthorizationReady` | Role/RoleBinding index ready and authorization operational |
| `ProvidersHealthy` | All required Provider processes are Ready |
| `CoreHandlerReady` | Fixed core-controller process is fully initialized |
| `GenerationCleanupPending` | Prior-generation config-owned resources have not yet completed deletion |
| `GenerationCleanupFailed` | One or more prior-generation config-owned resources are stuck awaiting deletion beyond threshold |

Condition `status` values follow the common `True|False|Unknown` rule.
Transition times are RFC 3339 UTC.

`GenerationCleanupPending=True` changes the Zone `phase` to `Degraded` while
any prior-generation config-owned resources are awaiting deletion; this is the
normal pending-cleanup posture and does not indicate a fault. `GenerationCleanupFailed=True`
is additionally set when a candidate is stuck beyond `cleanupStuckThreshold` (default
5 minutes) with no controller progress.

### 2.6 ownerRef, finalizers, and deletion

`metadata.ownerRef` is always `null`. Zone is the root resource; it cannot be
owned.

Core adds `core.zone-drain` finalizer to Zone only when an explicit reset/
shutdown request is in progress. During drain:

1. `metadata.deletionRequestedAt` is set.
2. Core stops admitting new resource/service requests.
3. Each non-Zone resource receives a delete request in reverse dependency order
   under finalizer protocol.
4. After all other resources are deleted, `core.zone-drain` is cleared.
5. Final transaction emits `phase=Deleted` event and closes the store.

A Zone resource is never deleted except during a deliberate destructive reset.
Normal daemon restart, upgrade, or Provider lifecycle changes do not delete Zone.

---

## 3. ZoneLink

### 3.1 Role

`ZoneLink/<name>` represents one parent/child Zone delegation as the child
Zone's local uplink. It is authored and stored in the **child** Zone. It carries
that local child identity, transport/session requirements, cursor tracking for
resource synchronization, and connection health. `spec.childZoneName` MUST
equal the enclosing Zone's self-name.

A parent accesses child Zone resources exclusively through the child's
`d2b.resource.v3` service over a ComponentSession routed via the transport
Provider named in ZoneLink. The parent never receives a database handle,
credential, token, or cross-Zone ResourceRef.

The provisioning parent allocator owns privileged listener creation, placement,
and route-namespace effects. The Nix compiler selects that allocator through
the enclosing Zone's compiler-only `parentZone` setting. The allocator binds
the child-local ZoneLink UID and child identity to that one parent edge in
sealed bootstrap state and keeps only allocator/route-engine state in the
parent; it does not create a reciprocal parent-store resource.
**Controller/owner**: the child Zone's `zone
link/delegation` handler in the fixed core-controller process owns the
ZoneLink resource and its status. It consumes allocator observations through
the authenticated session and has no direct privileged-effect path. No other
controller may update ZoneLink spec or status.

### 3.2 Metadata

| Field | Rule |
| --- | --- |
| `metadata.name` | Operator-assigned; locally unique in the child Zone; no structural constraint beyond ResourceName regex |
| `metadata.zone` | Child Zone self-name; ZoneLink is always local to that child |
| `metadata.ownerRef` | Optional same-Zone ref to the managing transport/runtime Provider; never a parent-Zone ref |
| `metadata.finalizers` | Core adds `core.zone-link-drain` before deletion; transport Provider may add its own finalizer |
| `metadata.deletionRequestedAt` | Normal deletion by operator or owning resource |

### 3.3 Spec

```yaml
apiVersion: resources.d2bus.org/v3
type: ZoneLink
metadata:
  name: guest-uplink
  zone: guest
  uid: <store-generated immutable>
  generation: 1
  revision: <opaque>
  ownerRef: null
  finalizers:
    - core.zone-link-drain
  deletionRequestedAt: null
  createdAt: 2026-07-22T00:00:00.000Z
  updatedAt: 2026-07-22T00:00:00.000Z
spec:
  childZoneName: guest
  transportProviderRef: "Provider/transport-unix"
  transportSettings: {}
  transportCredentials: []
  disabled: false
  limits:
    maxPendingIntents: 256
    maxActiveStreams: 32
    reconnectMaxAttempts: 10
    reconnectWindowSecs: 300
```

**`spec.transportProviderRef`** (required; no default):

- Must match `^Provider/transport-[a-z][a-z0-9-]*$`; any other form is rejected at admission.
- References a Provider installed in the same child Zone whose name begins with `transport-`; that Provider must be in `Ready` phase before any connection is attempted.
- The named transport Provider exports a ZoneLink schema extension (`transportSettings` schema); the resource compiler validates `spec.transportSettings` against that schema at build time.
- No transport Provider is pre-installed; an operator must install one before creating ZoneLinks that require it.

**`spec.transportSettings`** (required; default `{}`):

- Provider-specific configuration object; validated against the transport Provider's ZoneLink schema extension at build time (Phase 2) and at admission time (runtime).
- Must not contain credential bytes; use `spec.transportCredentials` for secrets.

**`spec.transportCredentials`** (list of Credential refs; default `[]`):

- Each entry is a string `"Credential/<name>"` in the same Zone.
- Max 8 entries; evaluated at ComponentSession establishment time.
- Referenced Credentials must be declared in `d2b.zones.<zone>.resources.*` with `type = "Credential"`.

**`spec.limits`**: ZoneLink connection and queue limits. All fields have normative defaults (shown above). Bounds enforced at admission:
- `maxPendingIntents`: max 1024
- `maxActiveStreams`: max 128

### 3.4 Status

#### Three-layer status shape (D088)

D088 freezes `ZoneLink` status as three layers. The universal `ResourceStatus`
base (Layer 1) lives at top-level `status` and owns `observedGeneration`,
`phase`, `conditions`, `lastReconciledAt`, `startedAt`, `completedAt`, and
bounded `outcome`. The `ZoneLink`-specific status fields documented in this
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


ZoneLink.status extends common status with:

| Field | Type | Rules |
| --- | --- | --- |
| `childZoneUid` | UID string or null | Immutable UID of this local child Zone as acknowledged by the parent allocation; null until first successful connection |
| `connected` | bool | Whether the allocator-bound ComponentSession to the parent route endpoint is established |
| `lastConnectedAt` | RFC 3339 UTC or null | Most recent successful session establishment |
| `lastDisconnectedAt` | RFC 3339 UTC or null | Most recent disconnect |
| `lastSentRevision` | u64 or null | Highest child advertisement/local intent revision sent to the parent |
| `lastAckedRevision` | u64 or null | Highest child revision acknowledged by the parent |
| `lastReceivedRevision` | u64 or null | Highest parent route/export advertisement revision received |
| `lastAppliedRevision` | u64 or null | Highest parent advertisement revision applied to child-local routing/import state |
| `linkEpoch` | u64 | Monotonic counter; increments on every reconnect establishing a new session generation |
| `pendingLocalIntents` | u32 | Count of locally queued intents while disconnected; bounded |
| `childAuthorized` | bool | Whether the parent allocator accepted and authorized this child Zone subject |

`childZoneUid` lets both ends detect that this child Zone was destroyed and
recreated. If it changes across reconnects, the child-local ZoneLink handler
clears local cursor state and the parent allocator discards the corresponding
route allocation before both sides relist/rewatch from revision 0.

Cursor fields (`lastSentRevision`, `lastAckedRevision`, `lastReceivedRevision`,
`lastAppliedRevision`) reflect the `zone_link_cursors` redb table entry for
this ZoneLink. They are bounded u64 monotonic values. Their exact semantics
are defined by the ZoneLink session and cursor protocol.

### 3.5 Phase and conditions

| Phase | Meaning |
| --- | --- |
| `Pending` | Transport not yet connected or child not yet authorized |
| `Ready` | ComponentSession established; child authorized; resources accessible |
| `Degraded` | Connected but some capability unavailable (e.g. watch quota exceeded) |
| `Failed` | Cannot connect after retry policy exhausted, or child permanently denied |
| `Unknown` | Transport state cannot be determined |

The parent Zone has no ZoneLink resource phase. The child-local ZoneLink alone
reports the edge's resource phase from authenticated allocator observations.

Closed condition types for ZoneLink:

| Condition type | Meaning |
| --- | --- |
| `TransportReachable` | Transport provider reports the endpoint is reachable |
| `SessionEstablished` | ComponentSession handshake completed successfully |
| `ChildAuthorized` | Parent allocator accepted the authenticated child subject and route allocation |
| `CursorSynchronized` | Parent/child revision cursors are within expected bounds |
| `LocalIntentsDrained` | No pending locally queued intents remain |
| `DisabledByOperator` | `spec.disabled: true`; link is intentionally inactive |

### 3.6 Local intents while disconnected

When the parent route endpoint is unreachable, the child may record bounded
local ZoneLink intents - operator intent that cannot be forwarded until
reconnection. These are not resource writes in the parent. They are persisted
in the child store as ZoneLink-owned metadata-only entries.

Rules:

- bounded count: `pendingLocalIntents` must not exceed 256; new intents are
  rejected with `backpressure` when at limit;
- intents are ordered FIFO; no deduplication or priority;
- on reconnect the ZoneLink handler applies pending intents in order under
  expected-revision preconditions;
- failed intent application clears the intent queue and records a
  `IntentApplicationFailed` condition; the operator must relist and retry;
- the queue is cleared on ZoneLink deletion or successful relist/resync.

### 3.7 ownerRef, finalizers, and deletion

`metadata.ownerRef` is optional and always same-Zone. A local transport/runtime
Provider may own the uplink it manages. A parent allocator, parent Provider, or
parent Zone resource can never be the owner because that would require a
cross-Zone ref. When the local owner is deleted, ZoneLink deletion follows
standard child-first finalizer protocol.

Core adds `core.zone-link-drain` to `metadata.finalizers` before processing a
delete request. The ZoneLink handler:

1. Closes the allocator-bound ComponentSession to the parent route endpoint.
2. Clears the cursor table entry.
3. Removes any pending local intents.
4. Clears `core.zone-link-drain`.
5. Clears any transport-Provider finalizer (by calling delete on the provider's
   owned endpoint resource if applicable under the resolved transport Provider
   contract).

After all finalizers clear, core removes the ZoneLink resource and its
`zone_link_cursors` entry in one transaction.

---

## 4. Provider

### 4.1 Role

`Provider/<name>` is the installed representation of one independently
buildable Provider package in a Zone. A `providerRef` resolves only if the
named Provider resource exists and is `Ready` in the same Zone.

Provider resources are always Zone-local. There are no cross-Zone Provider
refs. Provider installation is a Zone-local administrative act.

> **`unsafe-local` is NOT a Provider.** The current `kind = "unsafe-local"` Nix
> workload option (and the baseline `WorkloadProviderKind::UnsafeLocal` /
> `IsolationPosture::UnsafeLocal` in `d2b-realm-core/src/workload.rs`) maps to
> a **Host** resource with `spec.defaultDomain=user`, `spec.allowedDomains=[user]`,
> and `spec.defaultUserRef=User/<name>`, reconciled by `Provider/system-core`.
> No Provider resource with name or type `unsafe-local` exists in ADR 0046.
> Child processes of an unsafe-local Host use normal Process Providers. No-isolation
> posture and warnings are preserved in Host status, CLI output, and audit records.
> They must not appear in OTEL metric label values or structured log labels. See ADR046-zone-control-008 (§17) and §16.2 subsection
> "Workload posture and unsafe-local types".

**Controller/owner**: The `provider lifecycle` handler of the fixed
core-controller process owns Provider reconciliation. Only Provider/system-core
and Provider/system-minijail are fixed bootstrap exceptions with no Process
resource - all other Provider controllers are Process resources owned by their
Provider resources.

### 4.2 Metadata

| Field | Rule |
| --- | --- |
| `metadata.name` | Must equal the Provider's declared identity; no installer can select an arbitrary name independent of the package |
| `metadata.zone` | Zone where Provider is installed |
| `metadata.ownerRef` | `null` for directly installed Providers; may point to a managing resource for dependent Providers |
| `metadata.finalizers` | Core adds `core.provider-api-binding` before Provider deletion begins |
| `metadata.deletionRequestedAt` | Set by operator delete or owning resource delete |

Finalizer `core.provider-api-binding` ensures the API catalog handler
withdraws all exported ResourceType schemas and verifies no resources of those
types remain before the Provider resource is removed.

### 4.3 Spec

Provider.spec carries the complete desired installation. All digests are
`sha256:` prefixed lowercase hex. An empty or missing digest field is
rejected at admission.

```yaml
spec:
  artifactId: "runtime-cloud-hypervisor"  # bounded ID; references d2b.artifacts catalog
  # PackageIdentity fields (digest, manifestDigest, configSchemaDigest, signatureId,
  # trustEpoch, conformanceAttestationDigest, compatibility, support, etc.) are
  # resolved from the artifact catalog by the resource compiler; they do not appear
  # in operator-authored spec or in the canonical JSON envelope.
  config: {}                            # root config; validated against catalog's configSchemaDigest schema
  exports:
    resourceTypes:
      - name: Host                      # short Zone-unique ResourceType name (or vendor.qualified.Name)
        version: 1
        schemaDigest: "sha256:<64 hex>"
      - name: User
        version: 1
        schemaDigest: "sha256:<64 hex>"
  components:
    controllers:
      - id: host-controller
        binaryRef: d2b-core-controller
        resourceTypes: [Host, User]
        supportedHostProviderCapabilities: []
        supportedGuestProviderCapabilities: []
        allowedDomains: [system]
        specVerbs: [create, update-spec]
        statusVerbs: [update-status]
        finalizerIds: [core.host-teardown]
        watchSelectors:
          - resourceType: Host
          - resourceType: User
        dependencySelectors: []
        ownerChildTriggers: [owned-resource-changed]
        reconcileConcurrency: 8
        observeConcurrency: 4
        maxPendingResources: 256
        observeIntervalSeconds: null    # null = no periodic observe; non-null = seconds
        resyncPolicy: on-dependency-change
        resourceTypeDeadlineSeconds:
          Host: 30
          User: 10
    services: []
    workers: []
  dependencies:
    # aliases bound to Provider names; closed set defined per Provider manifest
    # example entries (actual aliases declared in signed manifest):
    # runtime: Provider/runtime-cloud-hypervisor
    # volume: Provider/volume-local
  permissionClaims:
    - verb: create
      resourceType: Host
    - verb: update-status
      resourceType: Host
    - verb: create
      resourceType: User
    - verb: update-status
      resourceType: User
  lifecycle:
    upgradePolicy: drain-then-replace   # drain-then-replace | rolling | immediate
    drainTimeoutSeconds: 120
    restartPolicy: on-failure
    maxRestarts: 5
    restartBackoffSeconds: 10
```

#### 4.3.1 Artifact identity and catalog fields

`spec.artifactId` is a bounded label (`^[a-z][a-z0-9-]*$`, max 128 chars)
referencing the `d2b.artifacts.<id>` catalog entry with `type = "provider"`.
All PackageIdentity sub-fields are resolved from the artifact catalog at build
time and are **not** present in the operator-authored spec or canonical JSON
envelope. They are validated by the resource compiler and stored in the private
artifact catalog; the resource store holds only `artifactId`.

| Artifact catalog field | Type | Rules |
| --- | --- | --- |
| `name` (from manifest) | string | Must equal Provider identity declared in manifest; must equal `metadata.name` |
| `version` (from manifest) | string | Semver `major.minor.patch`; informational; exact digest is binding |
| `digest` | sha256 | Content digest of signed Provider package; required; validated at build |
| `executableDigests` | map[name]sha256 | One entry per built binary; validated at build |
| `manifestDigest` | sha256 | Digest of the Provider's signed manifest artifact; validated at build |
| `configSchemaDigest` | sha256 | Digest of the root config JSON Schema; used to validate `spec.config` |
| `publisher` | string | Stable publisher/organization label; `^[a-z][a-z0-9-]*$` |
| `signatureId` | string | Stable opaque signature reference; non-empty |
| `trustEpoch` | u32 | Trust root epoch; must not be revoked |
| `revocationRef` | null or string | Stable revocation check token; null if no revocation mechanism |
| `conformanceAttestationDigest` | sha256 | Digest of Provider API conformance attestation |
| `compatibility.apiVersion` | u32 | Provider API major version; exact match required; no downgrade |

Unknown artifact catalog fields are rejected at build time. Store paths and
Nix closure metadata are private catalog implementation data and do not appear
in resource spec, status, audit records, or OTEL attributes.

#### 4.3.2 Root configuration

`spec.config` is an object validated against the JSON Schema identified by
the artifact catalog's `configSchemaDigest` for `spec.artifactId`. Validation
occurs at build time (resource compiler) and is re-validated at Provider spec
admission by the Zone runtime.

Rules:

- all validation is schema-strict; unknown fields in config are rejected unless
  the schema explicitly permits `additionalProperties: true` in a bounded
  vendor extension object;
- no credential bytes, raw host paths, PIDs, process arguments, or ambient
  authority values are permitted in `config`;
- secrets are `Credential/<name>` ResourceRefs; config values that need
  resolution reference Credentials or Volumes;
- config digest (derived from canonical JSON representation of `config`) is
  bound to every Process resource spawned from this Provider's components;
- config replacement (spec.generation increment) triggers all owned Process
  resources to be drained and replaced;
- the empty config `{}` is valid for Providers with no config.

#### 4.3.3 Component descriptors

A Provider may declare at most one set of each component type (controllers,
services, workers). Component IDs are unique within the Provider.

The following component bounds are normative and enforced at admission:

| Field | Maximum |
| --- | --- |
| Controllers per Provider | 8 |
| Services per Provider | 8 |
| Worker templates per Provider | 32 |
| ResourceTypes per controller | 16 |
| Watch selectors per controller | 32 |
| Dependency selectors per controller | 16 |
| Finalizer IDs per controller | 8 |

**Controller descriptor fields** (normative):


| Field | Type | Rules |
| --- | --- | --- |
| `id` | ComponentId | `^[a-z][a-z0-9-]*$`; unique within Provider |
| `binaryRef` | string | Key in `package.executableDigests`; identifies the built binary |
| `resourceTypes` | list of ResourceType names | ResourceTypes this controller reconciles; declared in `exports.resourceTypes`; max 16 |
| `supportedHostProviderCapabilities` | list of capability tokens | Host Provider capabilities required for this controller to operate |
| `supportedGuestProviderCapabilities` | list of capability tokens | Guest Provider capabilities required |
| `allowedDomains` | list of `system|user` | Process domains this controller instance supports |
| `specVerbs` | list of resource verbs | Verbs on spec subresource; bounded set from common verb enum |
| `statusVerbs` | list of resource verbs | Verbs on status subresource; controller is status owner if `update-status` present |
| `finalizerIds` | list of FinalizerId | Finalizers this controller adds/clears; `^[a-z][a-z0-9.-]*$` |
| `watchSelectors` | list of selector objects | Exact ResourceTypes and optional name/owner/zone filters this controller watches |
| `dependencySelectors` | list of selector objects | ResourceTypes watched as read-only dependencies |
| `ownerChildTriggers` | list of trigger reason tokens | Subset of common triggers; must include `owned-resource-changed` if controller uses owner index |
| `reconcileConcurrency` | u32 ≥ 1 | Max concurrent reconcile handlers for distinct resources |
| `observeConcurrency` | u32 ≥ 1 | Max concurrent observe handlers |
| `maxPendingResources` | u32 ≥ 1 | Max resources queued for reconcile before backpressure |
| `observeIntervalSeconds` | null or u32 ≥ 1 | Periodic observe period; null means no periodic observe |
| `resyncPolicy` | enum | `on-dependency-change | manual | period-only` |
| `resourceTypeDeadlineSeconds` | map[ResourceType]u32 | Per-type per-reconcile deadline; 0 means use global default |

**Service descriptor fields**:

| Field | Type | Rules |
| --- | --- | --- |
| `id` | ComponentId | Unique within Provider |
| `binaryRef` | string | Key in `package.executableDigests` |
| `serviceMethods` | list of method IDs | ttrpc method names this service exports |
| `serviceSchemaDigest` | sha256 | Digest of ttrpc service descriptor |
| `allowedDomains` | list of `system|user` | |
| `endpointRequirements` | object | ComponentSession purpose class/transport class required |
| `subjectConstraints` | list | Closed set of ResourceType subject classes allowed to connect |

**Worker descriptor fields**:

| Field | Type | Rules |
| --- | --- | --- |
| `id` | ComponentId | Unique within Provider |
| `binaryRef` | string | Key in `package.executableDigests` |
| `templateName` | string | Name of the Process template used for this worker |
| `allowedDomains` | list of `system|user` | |
| `configProjection` | list of config field paths | Config fields visible to this worker; bounded |
| `maxInstances` | u32 ≥ 1 | Cardinality; owner/controller enforces |
| `executionConstraints` | object | Allowed executionRef ResourceTypes (Host or Guest) for this worker |

#### 4.3.4 Dependencies

`spec.dependencies` is a flat map from alias to Provider ResourceRef:

```yaml
dependencies:
  runtime: Provider/runtime-cloud-hypervisor
  volume: Provider/volume-local
  network: Provider/network-local
  credential: Provider/credential-secret-service
  transport: Provider/transport-unix
```

Rules:

- alias names are defined in the Provider's signed manifest; the Zone config
  binds each alias to an exact installed Provider;
- alias resolution is request-time; the Provider lifecycle handler re-checks
  all aliases when a referenced Provider enters or leaves `Ready`;
- synchronous dependency cycles fail Provider configuration;
- an optional dependency's absence produces `Degraded` in the Provider's
  component that requires it; the Provider may still publish other components;
- a required dependency's absence prevents the Provider reaching `Ready`;
- alias keys must be `^[a-z][a-z0-9-]*$`; max 16 aliases per Provider.

#### 4.3.5 Permission claims

`spec.permissionClaims` is a bounded list of operations the Provider controller
or service requests. Each claim has:

```yaml
- verb: create               # resource verb or runtime verb
  resourceType: Host         # ResourceType name
  subresource: null          # null or bounded subresource name
  resourceNames: []          # [] = all; non-empty = explicit names
  zones: []                  # [] = this Zone only; non-empty = named linked Zones
  executionRefs: []          # [] = all; non-empty = explicit Host/Guest refs
```

Rules:

- the Zone API binding handler intersects permission claims with Zone policy
  derived from installed resources and static admission rules before granting
  any Role;
- a Provider cannot claim permissions not declared in its signed manifest;
- cross-Zone claims require an explicit Zone in `zones`; a ZoneLink must exist;
- wildcard permission claims (empty `resourceNames` and `executionRefs`) are
  only permitted for verified system Providers with explicit review;
- permission claims are inputs to generated Roles; they are not
  self-executing grants.

#### 4.3.6 Upgrade and lifecycle policy

| Field | Values | Default |
| --- | --- | --- |
| `lifecycle.upgradePolicy` | `drain-then-replace | rolling | immediate` | `drain-then-replace` |
| `lifecycle.drainTimeoutSeconds` | u32; 0 < t ≤ 3600 | 120 |
| `lifecycle.restartPolicy` | `on-failure | always | never` | `on-failure` |
| `lifecycle.maxRestarts` | u32; ≤ 20 | 5 |
| `lifecycle.restartBackoffSeconds` | u32; 1 ≤ b ≤ 3600 | 10 |

`drain-then-replace`: All controlled processes are gracefully stopped before
new version starts. No overlap window. Safe for stateful controllers.

`rolling`: Controller reconciliation continues; workers are replaced
concurrently under cardinality constraints. Not safe for controllers that hold
exclusive locks.

`immediate`: Old processes stop concurrently with new start. Only for
stateless workers.

### 4.4 Status

#### Three-layer status shape (D088)

D088 freezes `Provider` status as three layers. The universal `ResourceStatus`
base (Layer 1) lives at top-level `status` and owns `observedGeneration`,
`phase`, `conditions`, `lastReconciledAt`, `startedAt`, `completedAt`, and
bounded `outcome`. The `Provider`-specific status fields documented in this
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

Provider.status extends common status with:

| Field | Type | Rules |
| --- | --- | --- |
| `trustResult` | enum or null | `trusted | revoked | expired-epoch | attestation-failed | conformance-failed`; null until first check |
| `conformanceResult` | enum or null | `passed | failed | skipped`; null until first check |
| `lastTrustCheckedAt` | RFC 3339 UTC or null | |
| `activePackageDigest` | sha256 or null | Currently running package digest; differs from spec during upgrade |
| `activeConfigDigest` | sha256 or null | Currently running config digest |
| `providerGeneration` | u64 | Monotonic counter incremented by core on every trust+config+component re-validation |
| `components` | bounded list | Per-component status entries |
| `exportedResourceTypes` | list | ResourceTypes currently published to API catalog |
| `dependencyHealth` | map | alias → `Ready|Degraded|Missing`; no Provider details |
| `controllerLeases` | bounded list | Per-controller lease ID / phase / lastCheckpointAt |
| `stateSchemaMigration` | enum | `current | pending | in-progress | failed`; null if no state schema |
| `disabled` | bool | Whether Provider is operator-disabled |
| `quarantined` | bool | Whether core has quarantined Provider due to trust/conformance failure |

**Component status entry**:

```yaml
- componentId: host-controller
  type: controller                     # controller | service | worker
  phase: Ready                         # common phases
  processRef: Process/system-core-host-controller  # null for bootstrap exceptions
  lastReconciledAt: 2026-07-22T00:00:01.000Z
  conditions: []
```

`processRef` is null for `Provider/system-core` and `Provider/system-minijail`
components only. All other component `processRef` values must resolve to a
current Process resource owned by this Provider.

### 4.5 Phase and conditions

| Phase | Meaning |
| --- | --- |
| `Pending` | Provider spec admitted; trust/conformance/component checks not yet complete |
| `Ready` | All required components Ready; all exported ResourceTypes published |
| `Degraded` | Optional components impaired; required components Ready; exported types published |
| `Failed` | One or more required components Failed; exported types withdrawn |
| `Unknown` | Core cannot determine Provider state |

Provider never reaches `Succeeded` (long-lived resource).

Closed condition types for Provider:

| Condition type | Meaning |
| --- | --- |
| `PackageTrusted` | Package digest/signature/epoch/conformance verified |
| `ConfigValid` | Root config validated against configSchemaDigest schema |
| `DependenciesReady` | All required dependency aliases resolved to Ready Providers |
| `ComponentsReady` | All required component Processes in Ready phase |
| `ApiPublished` | All exported ResourceTypes bound in Zone API catalog |
| `ControllerLeaseActive` | At least one active controller lease exists |
| `DrainComplete` | Upgrade drain complete; `True` only during drain cycle |
| `Quarantined` | Trust/conformance failure has caused quarantine |

### 4.6 ownerRef, finalizers, and deletion

**ownerRef**: null for directly installed Providers. A managing resource
(e.g. a future Provider catalog manager) may own a Provider, making it a
generated child that is deleted when the owner is deleted.

**Finalizer `core.provider-api-binding`**: Added by core when a delete request
arrives. The API catalog handler:

1. Marks all exported ResourceTypes as `withdrawing`.
2. Verifies no resources of those types remain (returns Pending if they do,
   with a `PendingResourceDeletion` condition).
3. Atomically unbinds types from api_schemas.
4. Clears `core.provider-api-binding`.

**Additional Provider finalizers**: A Provider controller may add finalization
logic through its own finalizer ID (declared in the controller descriptor
`finalizerIds`). Core only coordinates finalizer ordering; it does not execute
Provider-specific finalization logic.

**Deletion sequence**:

1. Operator requests delete; `deletionRequestedAt` is set.
2. All owned child resources (Process, Volume, etc.) receive delete requests
   child-first.
3. `core.provider-api-binding` awaits ResourceType draining.
4. Provider-specific controller finalizers execute.
5. All finalizers cleared; `phase=Deleted` emitted; resource removed.

### 4.7 Bootstrap exceptions

`Provider/system-core` and `Provider/system-minijail` are the two Provider
bootstrap exceptions:

- they have no `processRef` in their component status;
- their Process binaries are embedded in or co-launched with the Zone runtime;
- they are pre-created in the store during zone initialization before any
  other resource exists;
- they are granted only the compiled bootstrap authorization (§6);
- they cannot be deleted while the Zone is operational;
- their trust/package/conformance fields must still pass validation; they
  are not exempt from trust checks, only from having Process resource children.

`Provider/system-minijail` is the fixed second bootstrap exception because:

- the first Process controller (`system-minijail`) cannot depend on a Process
  controller to launch itself;
- `system-minijail` reconciles all later Process resources including
  `system-systemd`; it is not a general-purpose minijail executor outside
  this bootstrap role.

After bootstrap both Providers share the same core-controller process process
boundary but use distinct authenticated subjects and closed RBAC grants.

### 4.8 Provider crate layout (normative)

Every Provider implementation lives in its own workspace member crate under
`packages/` with the name `d2b-provider-<base>-<implementation>` (following
AGENTS.md naming: base before implementation, alphanumerically sorted in the
workspace member list). The crate **MUST** contain all four of the following
paths or the workspace/package policy check fails the build:

#### 4.8.1 Required paths

| Path | Required contents | Missing → |
| --- | --- | --- |
| `src/` | All implementation source files and binaries. Colocated unit tests (`#[cfg(test)]` modules) live here. Must contain at least one `lib.rs`, `main.rs`, or named binary entry point. | policy failure |
| `tests/` | Hermetic Cargo integration tests. Must cover: ResourceType admission/lifecycle, controller reconcile and phase transitions, conformance checks, and one fault-injection test (e.g. trust failure, config-schema mismatch, component crash). No external I/O or container dependencies; runs inside the Nix sandbox build. | policy failure |
| `integration/` | Heavier fixtures and scenarios invoked by the existing test orchestration (`make test-integration` / `make test-host-integration`). Must cover at least one cross-process scenario (Provider process spawn + component lifecycle) and, if the Provider creates Host or Guest resources, one Host or Guest attachment scenario. Scenarios must declare which orchestration target invokes them (container or host-integration). May not import symbols from `src/` directly; communicates through the Zone API or test harness fixtures. | policy failure |
| `README.md` | Provider identity (name, publisher, version, trust/conformance attestation pointers); full `spec.config` schema with field descriptions and defaults; list of exported ResourceTypes with brief description; list of controllers, services, workers, and binaries (role, placement, resource quotas); placement constraints and dependencies; RBAC requirements (Roles and RoleBindings the Provider requires pre-installed); security posture (capabilities, namespace isolation, state roots, credential handling); state and telemetry (audit events emitted, OTEL span/metric names); build, unit-test, integration-test commands; note on future standalone-repo packaging if applicable. | policy failure |

#### 4.8.2 Policy enforcement

The workspace policy check (`packages/d2b-contract-tests/tests/policy_provider_crate_layout.rs`,
implemented in work item ADR046-pkg-001) walks every `packages/d2b-provider-*`
crate directory in the workspace and asserts all four paths exist. A missing
`src/`, `tests/`, `integration/`, or `README.md` in any Provider crate is a
**hard policy failure** that blocks `make test-policy` (and therefore
`make check`) on the same basis as a workspace member sort violation or a
`Command::new("bash")` site.

The check is gated by the same `D2B_FIXTURES` step in
`tests/tools/rust-workspace-checks.sh` as the existing crate-naming and
member-sort checks. It does not require building the crate; it inspects
the filesystem only.

#### 4.8.3 README minimum sections

A Provider `README.md` must contain all of the following section headings
(exact casing not required; matched case-insensitively after stripping `#`
whitespace):

1. `Provider identity`
2. `Config schema`
3. `Exported resource types`
4. `Controllers / services / workers / binaries`
5. `Placement and dependencies`
6. `RBAC requirements`
7. `Security posture`
8. `State and telemetry`
9. `Build and test`

The policy check verifies all nine headings are present. A missing heading
is a policy failure with the message:
`"d2b-provider-<name>/README.md: missing required section '<heading>'"`.

#### 4.8.4 Integration scenario declaration

Each file under `integration/` must declare its orchestration target in a
top-level comment or doc attribute within the first 20 lines:

```rust
//! integration-target: container          // runs under make test-integration
//! integration-target: host-integration   // runs under make test-host-integration
```

The policy check verifies every `integration/*.rs` file carries exactly one
such declaration. Missing or multiple declarations are a policy failure.

#### 4.8.5 Example skeleton

```
packages/d2b-provider-runtime-cloud-hypervisor/
├── Cargo.toml
├── README.md                          # §4.8.3 sections required
├── src/
│   ├── lib.rs                         # provider registration, component factory
│   ├── guest.rs                       # Guest ResourceType controller
│   ├── process.rs                     # Process ResourceType controller
│   └── bin/
│       └── d2b-provider-runtime-cloud-hypervisor   # provider agent binary
├── tests/
│   ├── guest_lifecycle.rs             # admission → Pending → Ready → deletion
│   ├── process_lifecycle.rs           # Process component phase transitions
│   ├── conformance.rs                 # check_provider_conformance/check_descriptor_conformance
│   └── fault_trust_failure.rs        # quarantine on trust failure
└── integration/
    ├── guest_spawn.rs                 # integration-target: host-integration
    └── guest_network_attach.rs        # integration-target: container
```

---

## 5. Role

### 5.1 Role and ownership

`Role/<name>` declares a bounded set of permission rules. RoleBindings grant
one Role to one or more subjects.

**Controller/owner**: The `authorization` handler of the fixed core-controller
process owns Role reconciliation. No other controller may update Role spec or
status.

### 5.2 Metadata

| Field | Rule |
| --- | --- |
| `metadata.name` | Zone-unique ResourceName |
| `metadata.ownerRef` | Optional; may be owned by a Provider (for generated Roles), by a ZoneLink (for core-generated relay Roles), or null for operator-created Roles |
| `metadata.finalizers` | Core adds `core.role-binding-drain` when a Role with active RoleBindings is deleted |
| `metadata.deletionRequestedAt` | Set on operator delete or owning resource delete |

### 5.3 Spec

The complete Role spec is the `rules` array:

```yaml
apiVersion: resources.d2bus.org/v3
type: Role
metadata:
  name: process-controller
  zone: dev
  uid: <store-generated>
  generation: 1
  revision: <opaque>
  ownerRef: null
  finalizers: []
  deletionRequestedAt: null
  createdAt: 2026-07-22T00:00:00.000Z
  updatedAt: 2026-07-22T00:00:00.000Z
spec:
  rules:
    - resourceTypes: [Process, EphemeralProcess]
      verbs: [get, list, watch, create, update-spec, update-status, update-finalizers, delete]
      subresources: []
      resourceNames: []
      zones: []
      executionRefs: []
      sessionVerbs: []
status:
  observedGeneration: 1
  phase: Ready
  conditions: []
  lastReconciledAt: 2026-07-22T00:00:01.000Z
  startedAt: null
  completedAt: null
  outcome: null
  resource: {}
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

#### 5.3.1 Rule schema

Each rule in `spec.rules` has:

| Field | Type | Default | Semantics |
| --- | --- | --- | --- |
| `resourceTypes` | list of ResourceType names | required, non-empty | ResourceTypes this rule covers; short Zone-unique names or qualified vendor names |
| `verbs` | list of resource verb tokens | `[]` = no resource verbs | Resource operation verbs (see §5.3.2); at least one of `verbs` or `sessionVerbs` must be non-empty |
| `subresources` | list of subresource or exact service/method selectors | `[]` = no subresource restriction | Empty means all subresources; non-empty means exactly those subresources or qualified service/method selectors |
| `resourceNames` | list of ResourceName | `[]` = all | Empty means all names; non-empty means exactly those names; max 64 |
| `zones` | list of Zone names | `[]` = this Zone only | Empty means only the evaluating Zone; non-empty means any listed Zone |
| `executionRefs` | list of ResourceRefs | `[]` = no restriction | Restricts to operations whose target resource has one of these executionRefs; `[]` means unrestricted |
| `sessionVerbs` | list of session verb tokens | `[]` = no session verbs | ComponentSession/service verbs granted to this rule (see §5.3.2) |

All list fields are deduplicated at admission. Order is not significant.

#### 5.3.2 Verb sets

**Resource verbs** (exact closed set):

| Verb | Semantics |
| --- | --- |
| `get` | Single resource read |
| `list` | Multi-resource read with pagination |
| `watch` | Streaming change subscription |
| `create` | Create a new resource |
| `update-spec` | Full spec replacement |
| `update-status` | Full status replacement (controller-only) |
| `update-metadata` | Bounded labels/annotations/ownerRef metadata change |
| `update-finalizers` | Add/remove finalizers (ownership constrained) |
| `delete` | Request resource deletion |
| `use-credential` | Invoke a Credential operation selected by one exact Credential allowed-operation subresource |
| `admin-credential` | Supplement matching ordinary Credential create/update-spec/delete authority using the same exact lifecycle subresource |

Unknown verbs are rejected at admission.
`use-credential` and `admin-credential` are valid only for `Credential` rules
with the exact subresources defined by the Credential contract.

**Session verbs** (exact closed set):

| Verb | Semantics |
| --- | --- |
| `connect` | Open a ComponentSession to a service |
| `invoke` | Call a ttrpc method |
| `open-stream` | Open a named stream |
| `relay` | Forward an already-admitted invocation or stream to one authorized next ZoneLink hop |
| `attach` | Transfer a local file descriptor |
| `cancel` | Cancel an in-progress operation |
| `observe` | Subscribe to service health/event notifications |
| `audit-export` | Invoke only `d2b.audit.v3.AuditService/Export` (admin-only) |
| `support-bundle` | Invoke only `d2b.support.v3.SupportService/GenerateBundle` (admin-only) |

Session verbs in a Role rule grant ComponentSession/d2b-bus access to the
services bound by the same rule's `resourceTypes` and `zones` constraints.
They are evaluated by the same native RBAC engine as resource verbs.
`audit-export` and `support-bundle` require exact qualified service/method
selectors in `subresources`, may appear only in `sessionVerbs`, and imply no
resource read or mutation verb.

`relay` is transport forwarding authority only. It permits the exact
authenticated ZoneLink/transport subject to forward an invocation or named
stream that has already passed admission to the next route-selected hop. The
same hop independently evaluates the invocation's original target verb; a
`relay` allow never satisfies that check. `relay` grants no resource read,
create, update, or delete operation, identity mapping, capability widening,
attachment right, credential access, or local lifecycle authority.

A relay-bearing rule is admitted only when all of the following hold:

- `relay` appears in `sessionVerbs`, never `verbs`;
- `resourceTypes`, `resourceNames`, and `zones` exactly bound the forwarded
  target; empty/all-name scope and every wildcard form are rejected. Named
  methods match one immutable resource name. Nameless `List`/`Watch` requests
  retain a non-empty exact `resourceNames` allowlist and bounded filters whose
  possible result set is a subset of that allowlist at every hop;
- the Role and RoleBinding are core-generated with `ownerRef` naming the
  governing `ZoneLink`, and the binding's trusted external-principal selector
  matches the exact enrolled adjacent-`Zone` transport subject; or an
  already-authorized local administrator explicitly permits the same bounded
  grant through the durable admin-policy path;
- the request payload, Provider descriptor, and Provider/operator-authored
  resource cannot assert core-generated or admin-policy provenance.

Permission to create a Role or RoleBinding is not itself permission to grant
`relay`. Missing `relay`, missing target-verb authority, stale policy, or
unavailable policy state denies forwarding. There is no implicit relay grant.

#### 5.3.3 Explicit wildcard form

The foundation specifies: "no implicit wildcard is granted; a reviewed explicit
wildcard may exist only for fixed core-controller roles."

An explicit wildcard in `resourceNames` is represented as a single-element
list `["*"]`. This form is:

- permitted only for fixed core-controller generated Roles;
- prohibited in operator-created or Provider-generated Roles (admission
  rejects at creation time);
- evaluated strictly as "all names in the current Zone catalog"; it does not
  grant future ResourceTypes not yet bound;
- recorded in the authorization audit log with the wildcard marker.

No other wildcard form exists. `[]` (empty list) means "all" for
`resourceNames` - this is equivalent to a wildcard for names but is the
default form available to all subjects. An `executionRefs: []` means
"unrestricted by executionRef", not a wildcard requiring `["*"]`.

The distinction between `resourceNames: []` (unrestricted, default) and
`resourceNames: ["*"]` (explicit reviewed wildcard, only for core-controller
Roles) must be enforced at admission. For non-core-controller Roles,
`resourceNames: ["*"]` is rejected with `resource-schema-invalid`.

#### 5.3.4 Bounds

| Field | Maximum |
| --- | --- |
| Rules per Role | 32 |
| `resourceTypes` per rule | 16 |
| `verbs` per rule | 16 (bounded by verb enum: currently 11 verbs) |
| `sessionVerbs` per rule | 9 (bounded by session verb enum: currently 9 verbs) |
| `subresources` per rule | 16 |
| `resourceNames` per rule | 64 |
| `executionRefs` per rule | 32 |
| `zones` per rule | 8 |

All bounds are enforced at admission. Exceeding any bound returns `resource-schema-invalid`.

### 5.4 Status

#### Three-layer status shape (D088)

D088 freezes `Role` status as three layers. The universal `ResourceStatus`
base (Layer 1) lives at top-level `status` and owns `observedGeneration`,
`phase`, `conditions`, `lastReconciledAt`, `startedAt`, `completedAt`, and
bounded `outcome`. The `Role`-specific status fields documented in this
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


Role.status contains only common fields. There are no Role-specific status
fields beyond:

| Field | Type | Rules |
| --- | --- | --- |
| `activeBindingCount` | u32 | Number of RoleBindings currently referencing this Role; maintained by authorization handler |
| `lastValidatedAt` | RFC 3339 UTC or null | Most recent time the authorization handler validated this Role |

`activeBindingCount` is informational only. Deletion is blocked (via finalizer)
when this count > 0.

### 5.5 Phase and conditions

| Phase | Meaning |
| --- | --- |
| `Pending` | Role spec admitted; authorization handler has not yet built index entry |
| `Ready` | Role validated; index built; authorization operational |
| `Degraded` | Role valid but one or more rules reference unresolvable executionRefs (warn only) |
| `Failed` | Role spec fails validation (invalid verb, unknown ResourceType, bad ref) |

Closed condition types for Role:

| Condition type | Meaning |
| --- | --- |
| `RuleSetValid` | All rules have valid verbs, ResourceType names, and ref formats |
| `IndexBuilt` | Authorization index entry is current |
| `ActiveBindings` | `True` when activeBindingCount > 0; informational |
| `PendingBindingDrain` | `True` during deletion while active bindings exist |

### 5.6 ownerRef, finalizers, and deletion

**ownerRef**: null for operator-created Roles; may be a Provider for
generated core-controller Roles.

**Finalizer `core.role-binding-drain`**: Added when deletion is requested and
`activeBindingCount > 0`. The authorization handler:

1. Sets `PendingBindingDrain=True` condition; the Role remains in its current phase (typically `Ready`) while awaiting binding drain.
2. Awaits all RoleBindings referencing this Role to be deleted first.
3. Clears the index entry.
4. Clears `core.role-binding-drain`.

A Role with active bindings cannot be deleted until all bindings are removed.
This prevents authorization gaps from dangling RoleBinding references.

---

## 6. RoleBinding

### 6.1 Role and ownership

`RoleBinding/<name>` grants a Role to one or more subjects. It may optionally
narrow the Role's rules and carries revocation state.

**Controller/owner**: The `authorization` handler of the fixed core-controller
process owns RoleBinding reconciliation. No other controller may update
RoleBinding spec or status.

### 6.2 Metadata

| Field | Rule |
| --- | --- |
| `metadata.name` | Zone-unique ResourceName |
| `metadata.ownerRef` | Optional; may be owned by the Provider that generates this binding, or by the governing ZoneLink for a core-generated relay binding |
| `metadata.finalizers` | No standard finalizer; core clears the binding on deletion without deferral (no downstream finalizer chain) |
| `metadata.deletionRequestedAt` | Set on operator delete or owning resource delete |

### 6.3 Spec

```yaml
apiVersion: resources.d2bus.org/v3
type: RoleBinding
metadata:
  name: process-controller-binding
  zone: dev
  uid: <store-generated>
  generation: 1
  revision: <opaque>
  ownerRef: null
  finalizers: []
  deletionRequestedAt: null
  createdAt: 2026-07-22T00:00:00.000Z
  updatedAt: 2026-07-22T00:00:00.000Z
spec:
  roleRef: Role/process-controller
  subjects:
    - Provider/system-minijail
  externalPrincipalSelector: null
  scopeNarrowing: null                   # see §6.3.5
status:
  observedGeneration: 1
  phase: Ready
  conditions: []
  lastReconciledAt: 2026-07-22T00:00:01.000Z
  startedAt: null
  completedAt: null
  outcome: null
  resource: {}
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

#### 6.3.1 roleRef

`spec.roleRef` is a canonical ResourceRef `Role/<name>` in the same Zone.

Rules:

- `roleRef` is required; an absent or empty roleRef is rejected at admission;
- the referenced Role must exist; if it does not, RoleBinding enters `Failed`
  with condition `RoleNotFound`;
- `roleRef` is immutable after creation; changing the role requires deleting
  and recreating the RoleBinding;
- authorization evaluates the referenced Role's current generation at decision
  time; the authorization cache is invalidated when the Role changes.

#### 6.3.2 Subjects

`spec.subjects` is a bounded ordered list of canonical same-Zone ResourceRefs:

```yaml
subjects:
  - Provider/system-minijail
  - User/alice
  - Host/host-system
  - Process/wayland-proxy
  - Guest/dev-vm
```

Rules:

- each subject is a `<ResourceType>/<resource_name>` ResourceRef in this Zone;
- supported subject ResourceTypes: `Zone`, `User`, `Provider`, `Host`, `Guest`,
  `Process`; a ResourceRef `Zone` subject can name only the store's self Zone,
  while an enrolled adjacent-Zone transport subject is matched by
  `externalPrincipalSelector`; other ResourceTypes are rejected at admission with
  `resource-schema-invalid`;
- subjects list must be non-empty except for a core-generated, ZoneLink-owned
  relay binding with one exact trusted `externalPrincipalSelector`;
- duplicate subjects are rejected at admission;
- a subject that does not currently exist as a resource causes `SubjectNotFound`
  condition (warning only, not Failed; the subject may be created later);
- the resolved subject UID is stored for change detection; if a subject is
  deleted and a new resource of the same name is created, the authorization
  handler detects the UID change, removes the old binding entry, and emits a
  `SubjectIdentityChanged` condition;
- max subjects per RoleBinding: 128.

#### 6.3.3 External principal selector

`spec.externalPrincipalSelector` is `null` or a bounded object generated by
trusted enrollment/config. It selects ComponentSession subjects identified by
external identity evidence (e.g. enrolled Noise KK keys, vsock CID, enrollment
token digests) rather than by local ResourceRef.

Rules:

- external selectors may only appear in RoleBindings generated by trusted
  enrollment config (configuration publication or Provider bootstrap);
  operator-created RoleBindings may use external selectors only when the
  operator has permission to create enrollment records;
- an external selector contains no credential bytes; it contains only opaque
  enrollment digests or stable external identity tokens;
- external selectors are evaluated by the ComponentSession authentication step
  before d2b-bus routing; they never appear in `subjects`;
- external selectors are bounded in size (max 512 bytes canonical JSON);
- `externalPrincipalSelector` and `subjects` may both be present; a request
  satisfies the binding if it matches either.
- a core-generated relay binding uses an exact adjacent-Zone enrollment
  selector and no broad subject-class selector; a peer cannot supply it in a
  request.

#### 6.3.4 (removed - no expiry field)

`RoleBinding` has no `spec.expiry` field. Revoke, update, or delete a
RoleBinding via normal resource lifecycle operations. Time-limited grants
are implemented by operator-scheduled deletion of the RoleBinding resource,
not by an expiry field inside the spec.

#### 6.3.5 Scope narrowing


`spec.scopeNarrowing` is null or a subset of the referenced Role's rules:

```yaml
scopeNarrowing:
  rules:
    - resourceTypes: [Process]
      verbs: [get, list, watch]
      executionRefs: [Host/host-system]
```

Rules:

- scope narrowing may only restrict; it cannot grant verbs or resourceTypes
  not present in the referenced Role;
- an attempt to grant a verb absent from the Role is rejected at admission
  with `resource-schema-invalid`;
- narrowed rules are the intersection of the Role rules and the narrowing
  rules;
- `sessionVerbs`, including `relay`, `audit-export`, and `support-bundle`, are
  intersected exactly like resource verbs; narrowing cannot add one or remove
  its exact target or service/method bounds;
- `scopeNarrowing: null` means the full Role is granted without restriction;
- scope narrowing affects only this RoleBinding; the referenced Role is
  unchanged.

### 6.4 Status

#### Three-layer status shape (D088)

D088 freezes `RoleBinding` status as three layers. The universal `ResourceStatus`
base (Layer 1) lives at top-level `status` and owns `observedGeneration`,
`phase`, `conditions`, `lastReconciledAt`, `startedAt`, `completedAt`, and
bounded `outcome`. The `RoleBinding`-specific status fields documented in this
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

| Field | Type | Rules |
| --- | --- | --- |
| `roleResolved` | bool | Whether `roleRef` currently resolves to a Ready Role |
| `subjectCount` | u32 | Number of `subjects` entries |
| `unresolvedSubjects` | bounded list | Subject refs that currently have no matching resource; max 8 entries shown; overflow shown as count |
| `revoked` | bool | Whether operator revocation is set |
| `revokedAt` | RFC 3339 UTC or null | When revocation was set |

### 6.5 Phase and conditions

| Phase | Meaning |
| --- | --- |
| `Pending` | Admitted; authorization handler not yet built index entry |
| `Ready` | Role resolved; at least one subject matches; index entry built |
| `Degraded` | Role resolved but some subjects unresolvable (warning) |
| `Failed` | Role not found or revocation set |

Closed condition types for RoleBinding:

| Condition type | Meaning |
| --- | --- |
| `RoleFound` | Referenced Role exists and is Ready |
| `SubjectsResolved` | All listed subjects resolve to current resources |
| `SubjectNotFound` | One or more listed subjects do not exist (warning) |
| `SubjectIdentityChanged` | A subject UID changed due to delete/recreate |
| `IndexBuilt` | Authorization index entry is current |
| `RoleBindingRevoked` | Operator revocation set |

### 6.6 ownerRef, finalizers, and deletion

**ownerRef**: null for operator-created bindings; may point to a Provider
that generated the binding.

**No standard finalizer**: RoleBinding deletion is immediate. When
`deletionRequestedAt` is set:

1. Authorization handler drains in-flight request contexts: ongoing requests
   that were admitted under this binding retain their original context until
   their deadline; no new authorizations are granted to these subjects under
   this binding.
2. Authorization caches for all subjects in this binding are invalidated.
3. Role's `activeBindingCount` decrements.
4. **Final atomic transaction**: remove the RBAC index entry, emit the
   `Deleted` revision event, and remove the resource row from the Zone store
   as one redb write transaction. No observable intermediate state exists
   between index removal and row removal.

There is no RoleBinding finalizer chain; the binding is not needed to safely
remove.

---

## 7. Quota

### 7.1 Role and ownership

`Quota/<name>` declares Zone-wide aggregate resource ceilings and optionally
per-resource-type limits. Host, Guest, and Process resources may carry a
`spec.quotaRef: "Quota/<name>"` pointing to the governing Quota, and may
declare inline resource requests (cpu, memory, storage) within that Quota's
bounds.

**Controller/owner**: The `quota` handler of the fixed core-controller process
owns Quota reconciliation. No other controller may update Quota spec or status.

### 7.2 Metadata

| Field | Rule |
| --- | --- |
| `metadata.name` | Zone-unique ResourceName |
| `metadata.ownerRef` | null for operator-created Quotas |
| `metadata.finalizers` | Core adds `core.quota-drain` when Quota is the subject of a quotaRef before deletion |
| `metadata.deletionRequestedAt` | Set on operator delete |

### 7.3 Spec

```yaml
apiVersion: resources.d2bus.org/v3
type: Quota
metadata:
  name: default-quota
  zone: dev
  uid: <store-generated>
  generation: 1
  revision: <opaque>
  ownerRef: null
  finalizers: []
  deletionRequestedAt: null
  createdAt: 2026-07-22T00:00:00.000Z
  updatedAt: 2026-07-22T00:00:00.000Z
spec:
  ceilings:
    maxResources: 4096            # max total non-Deleted resources in Zone
    maxResourcesPerType: 512      # max non-Deleted resources per ResourceType
    maxOwnerDepth: 8              # max owner chain depth
    maxCpu: null                  # optional: aggregate vCPU ceiling across all quotaRef resources (null = unlimited)
    maxMemoryMib: null            # optional: aggregate memory MiB ceiling (null = unlimited)
    maxStorageGib: null           # optional: aggregate storage GiB ceiling (null = unlimited)
  perTypeCeilings: {}             # map ResourceType → {maxResources, maxCpu, maxMemoryMib, maxStorageGib}; empty = use global ceilings
  scope: zone                     # "zone" (only value in v3; reserved for future sub-Zone scopes)
  enforcementPolicy: hard         # "hard" (reject over-quota) | "soft" (warn only, still admit)
```

**Spec field rules**:

- `ceilings.maxResources`: `1..=65536`; default 4096
- `ceilings.maxResourcesPerType`: `1..=65536`; default 512
- `ceilings.maxOwnerDepth`: `1..=32`; default 8
- `ceilings.maxCpu`, `ceilings.maxMemoryMib`, `ceilings.maxStorageGib`: null or positive integer; null means no ceiling
- `perTypeCeilings`: max 64 entries; ResourceType names must be resolvable in the Zone API catalog
- `scope`: exactly `"zone"` in v3
- `enforcementPolicy`: `"hard"` | `"soft"`

Multiple Quota resources may coexist in a Zone; individual Host/Guest/Process
resources reference exactly one Quota via `spec.quotaRef` (or none). The quota
handler aggregates usage by quotaRef independently. A resource with no quotaRef
is counted against all `scope: zone` Quotas for the `maxResources` ceiling check
but is not subject to cpu/memory/storage ceilings.

### 7.4 Status

#### Three-layer status shape (D088)

D088 freezes `Quota` status as three layers. The universal `ResourceStatus`
base (Layer 1) lives at top-level `status` and owns `observedGeneration`,
`phase`, `conditions`, `lastReconciledAt`, `startedAt`, `completedAt`, and
bounded `outcome`. The `Quota`-specific status fields documented in this
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

| Field | Type | Rules |
| --- | --- | --- |
| `usedResources` | u32 | Count of non-Deleted resources referencing this Quota |
| `usedCpu` | u32 or null | Aggregate vCPU in use; null if no cpu ceiling |
| `usedMemoryMib` | u32 or null | Aggregate memory MiB in use; null if no memory ceiling |
| `usedStorageGib` | u32 or null | Aggregate storage GiB in use; null if no storage ceiling |
| `overQuota` | bool | True if any ceiling is currently exceeded (only possible with enforcementPolicy=soft) |
| `overQuotaTypes` | bounded list | ResourceTypes currently over their perTypeCeiling; max 16 shown |
| `lastCheckedAt` | RFC 3339 UTC or null | Most recent quota check |
| `dependentCount` | u32 | Count of resources referencing this Quota via `spec.quotaRef`; 0 when safe to delete |

### 7.5 Phase and conditions

| Phase | Meaning |
| --- | --- |
| `Pending` | Admitted; quota handler initializing |
| `Ready` | Quota active; ceilings enforced |
| `Degraded` | Quota over soft limit (enforcementPolicy=soft), or deletion requested with dependents remaining |
| `Failed` | Spec invalid (bad ceiling value, unknown ResourceType in perTypeCeilings) |

Closed condition types:

| Condition type | Meaning |
| --- | --- |
| `CeilingsValid` | All ceiling values and perTypeCeilings entries are valid |
| `OverQuota` | One or more ceilings exceeded (soft enforcement only) |
| `QuotaDrainPending` | Deletion in progress; `dependentCount` resources still reference this Quota; waiting for authorized owners/operators to reassign or delete them |

### 7.6 ownerRef, finalizers, and deletion

Quota deletion with active `quotaRef` assignments uses `core.quota-drain`:

1. `deletionRequestedAt` set; Zone transitions to `Degraded` with `QuotaDrainPending` condition.
2. Quota controller sets `QuotaDrainPending=True` condition with message
   `"<N> resources still reference this Quota; reassign or delete them before deletion completes"`.
3. `status.dependentCount` reflects the current count of resources with `spec.quotaRef` pointing to this Quota.
4. The quota controller does NOT mutate any other resource's `spec.quotaRef`.
   Authorized owners/operators must manually reassign dependents to another Quota or
   delete them; the controller blocks `core.quota-drain` clearance until `dependentCount = 0`.
5. The quota controller re-checks `dependentCount` on every reconcile triggered by
   resource creation/deletion/update; it emits `quota-drain-complete` audit record when
   the count reaches 0, then clears `core.quota-drain`.
6. Final atomic transaction: emit `Deleted` revision event; remove resource row and index.

A Quota with `enforcementPolicy=hard` and active resource ceilings blocks
resource creation over-quota; it never blocks deletion.

### 7.7 Nix authoring

```nix
d2b.zones.dev.resources.default-quota = {
  type = "Quota";
  spec = {
    ceilings = {
      maxResources    = 4096;
      maxResourcesPerType = 512;
      maxOwnerDepth   = 8;
      maxCpu          = null;
      maxMemoryMib    = null;
      maxStorageGib   = null;
    };
    perTypeCeilings   = {};
    scope             = "zone";
    enforcementPolicy = "hard";
  };
};
```

Eval-time assertions: `ceilings.maxResources` in `1..=65536`; `ceilings.maxOwnerDepth` in `1..=32`; `scope` equals `"zone"`; `enforcementPolicy` in `{"hard","soft"}`. Phase 2: `perTypeCeilings` ResourceType names resolved against Zone Provider catalogs.

### 7.8 Controller algorithm

1. **Validate** `spec.ceilings` and `spec.perTypeCeilings`; set `CeilingsValid`.
2. **Build usage index**: scan all non-Deleted Zone resources; aggregate by `quotaRef` and ResourceType.
3. **Enforce**: on every resource admission (`create` verb), check current usage + 1 against ceilings; reject with `quota-exceeded` if over ceiling and `enforcementPolicy=hard`; warn if `soft`.
4. **Update status**: write `usedResources`, `usedCpu`, `usedMemoryMib`, `usedStorageGib`, `overQuota`, `overQuotaTypes`.
5. **Trigger**: on any resource creation, deletion, or `spec.quotaRef` change, trigger quota reconcile.

### 7.9 RBAC and audit

Quota creates/updates generate audit event kind `resource-mutated` with redacted spec. Quota admission checks generate `quota-check` audit event with resource type, name, result (admitted/rejected), and usage snapshot. No subject names, resource names, or store paths in audit payload.

---

## 8. EmergencyPolicy

### 8.1 Role and ownership

`EmergencyPolicy/<name>` declares a Zone-wide emergency disable scope,
permitted actions during disable, and current disable status. Multiple
EmergencyPolicy resources may coexist; their scopes are evaluated as
a union (most restrictive wins per-action).

**Controller/owner**: The `emergency-policy` handler of the fixed
core-controller process owns EmergencyPolicy reconciliation. Multiple
EmergencyPolicy resources with `enabled=true` may coexist; the handler
computes the most-restrictive union of all active scope flags: any scope
flag set to `true` in any enabled policy applies Zone-wide. The effective
`drainDeadlineSeconds` is the minimum across all enabled policies.

### 8.2 Metadata

| Field | Rule |
| --- | --- |
| `metadata.name` | Zone-unique ResourceName |
| `metadata.ownerRef` | null for operator-created EmergencyPolicies |
| `metadata.finalizers` | `core.emergency-drain` added when policy has `active=true` at deletion time |
| `metadata.deletionRequestedAt` | Set on operator delete |

### 8.3 Spec

```yaml
apiVersion: resources.d2bus.org/v3
type: EmergencyPolicy
metadata:
  name: zone-lockdown
  zone: dev
  uid: <store-generated>
  generation: 1
  revision: <opaque>
  ownerRef: null
  finalizers: []
  deletionRequestedAt: null
  createdAt: 2026-07-22T00:00:00.000Z
  updatedAt: 2026-07-22T00:00:00.000Z
spec:
  enabled: false                        # operator activates/deactivates disable
  scope:
    stopNewAdmissions: true             # block all new resource API admissions
    disconnectZoneLinks: true           # gracefully disconnect all ZoneLinks
    stopProviderProcesses: false        # stop all Provider Process resources
    drainOngoingOperations: true        # drain in-flight operations to deadline
  drainDeadlineSeconds: 30             # max seconds to drain; default 30
  reason: ""                            # operator-supplied rationale (max 256 chars); visible in resource spec and audit; never emitted in status, OTEL labels, or log labels
```

**Spec field rules**:

- `enabled`: boolean; `false` means policy exists but is not active
- `scope.stopNewAdmissions`: boolean; when true, new resource API admissions are rejected while active
- `scope.disconnectZoneLinks`: boolean; when true, ZoneLinks receive graceful disconnect signal
- `scope.stopProviderProcesses`: boolean; when true, the runtime suppresses new Provider component Process launches and signals running non-bootstrap Provider component Processes to stop; Process resources are **not** deleted and no `deletionRequestedAt` is set; on deactivation the provider lifecycle controller resumes reconciliation and re-launches stopped Processes
- `scope.drainOngoingOperations`: boolean; when true, in-flight operations drain to deadline before service stops
- `drainDeadlineSeconds`: `1..=300`; default 30
- `reason`: string max 256 chars; visible in resource spec; included in audit record body; must not appear in status fields, OTEL metric label values, or structured log labels

### 8.4 Status

#### Three-layer status shape (D088)

D088 freezes `EmergencyPolicy` status as three layers. The universal `ResourceStatus`
base (Layer 1) lives at top-level `status` and owns `observedGeneration`,
`phase`, `conditions`, `lastReconciledAt`, `startedAt`, `completedAt`, and
bounded `outcome`. The `EmergencyPolicy`-specific status fields documented in this
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

| Field | Type | Rules |
| --- | --- | --- |
| `active` | bool | Whether this policy is currently enforced (`enabled=true` and effects applied) |
| `activatedAt` | RFC 3339 UTC or null | When most recently activated |
| `deactivatedAt` | RFC 3339 UTC or null | When most recently deactivated |
| `drainCompletedAt` | RFC 3339 UTC or null | When in-flight drain completed after most recent activation |

### 8.5 Phase and conditions

| Phase | Meaning |
| --- | --- |
| `Pending` | Admitted; handler initializing |
| `Ready` | Policy spec valid; handler operational (regardless of `enabled` state; see `status.active` for enforcement state) |
| `Degraded` | Policy enabled and drain in progress, or partial scope effect failure |
| `Failed` | Spec invalid |
| `Unknown` | Handler cannot determine state |

Closed condition types:

| Condition type | Meaning |
| --- | --- |
| `PolicyValid` | Spec fields are valid |
| `Enforced` | `True` when `enabled=true` and all effective scope actions are applied |
| `DrainComplete` | Post-activation drain completed; `True` after drain, reset on next activation |
| `EmergencyDrainPending` | Deletion is requested and `active=true`; drain must complete before finalizer clearance |

### 8.6 ownerRef, finalizers, and deletion

`core.emergency-drain` is added when deletion is requested while `status.active=true`:

1. The controller sets `enabled=false` internally to begin deactivation and scope-effect reversal.
2. Drain completes per the effective `drainDeadlineSeconds` (minimum across all enabled policies, or this policy's own value if alone).
3. `core.emergency-drain` cleared; final atomic transaction emits `Deleted` revision event; resource row removed.

Deletion of an `enabled=false` EmergencyPolicy has no finalizer; the final
atomic transaction emits `Deleted` and removes the row immediately.

### 8.7 Nix authoring

```nix
d2b.zones.dev.resources.zone-lockdown = {
  type = "EmergencyPolicy";
  spec = {
    enabled = false;
    scope = {
      stopNewAdmissions     = true;
      disconnectZoneLinks   = true;
      stopProviderProcesses = false;
      drainOngoingOperations = true;
    };
    drainDeadlineSeconds = 30;
    reason = "";
  };
};
```

Eval-time assertions: `drainDeadlineSeconds` in `1..=300`; `reason` max 256 chars. `reason` is visible in the resource spec; it must not appear in status fields, OTEL metric label values, or structured log labels.

### 8.8 Controller algorithm

1. **Validate** spec; set `PolicyValid`.
2. **Compute effective scope union**: at each reconcile, iterate all EmergencyPolicy resources with `enabled=true` (including self). For each scope flag, set the effective flag to `true` if ANY enabled policy has it `true`. Effective `drainDeadlineSeconds` = minimum across all enabled policies. Apply/maintain the effective scope set.
3. **On `enabled` transition to `true`**: set `active=true`, `activatedAt=now()`, `Enforced=True`; apply this policy's contribution to the scope union:
   - `stopNewAdmissions`: signal API admission gate to reject new requests (if not already gated by another policy)
   - `disconnectZoneLinks`: emit graceful disconnect to all active ZoneLinks (if not already disconnected)
   - `stopProviderProcesses`: suppress new Provider component Process launches; signal running non-bootstrap Provider component Processes to stop without setting `deletionRequestedAt` on their Process resources
   - `drainOngoingOperations`: set effective drain deadline; await in-flight drain completion or deadline
4. **On `enabled` transition to `false`**: set `active=false`, `deactivatedAt=now()`, `Enforced=False`; recompute scope union from remaining enabled policies; if no other policy gates an action, restore that action (admit new requests, allow ZoneLinks to reconnect, resume Provider lifecycle reconciliation for stopped Processes).
5. **Update status** on every reconcile.

### 8.9 RBAC and audit

EmergencyPolicy activation/deactivation generates `emergency-policy-activated` /
`emergency-policy-deactivated` audit events carrying: policy ResourceRef/UID,
effective scope flags (union or individual), and outcome. `reason` is included
in the audit record body (it is a resource spec field); it must not appear in
audit span attribute labels, OTEL metric label values, or structured log labels.
Principal identity is carried at the ComponentSession authentication layer and
is not duplicated in the audit payload.

---

## 8A. ResourceExport and ResourceImport (D096)

`ResourceExport` and `ResourceImport` are the two standard ResourceTypes (D096)
that let a Provider's cross-Zone capability serve multiple Zones through one
authority, without a direct cross-Zone `ResourceRef` or duplicate open of a
physical device/backend. They carry the D089 three-layer spec and D088
three-layer status like every other standard type.

### 8A.1 General model

- Every exportable capability has a qualified semantic/provider-neutral
  `*Service` ResourceType and matching qualified semantic `*Binding`
  ResourceType. These are not Provider implementation namespaces and are not
  additions to the 19 standard ResourceTypes.
- The owner-Zone `*Service` is the one real authority. Its same-Zone spec
  references the local `Device`, `Endpoint`, or qualified semantic backend
  allowed by the Provider's signed projection factory. No consumer Zone opens
  that backing.
- There is **no cross-Zone `ResourceRef`**. The owner Zone declares
  `ResourceExport/<name>` whose `resourceRef` targets only the local owner
  `*Service`. It never targets a `Device`, `Endpoint`, or `*Binding`. The consumer
  Zone declares `ResourceImport/<name>` that references only its local
  `zoneLinkRef` plus a bounded remote `exportKey` and signed fingerprints.
- The core ZoneLink **export/import controller** (this spec, `d2b-core-controller`)
  owns routing and base lifecycle. It delegates semantic admission and
  observations to the selected Provider's signed **export/import adapter** and
  projection factory. A missing, unsigned, or mismatched factory fails closed.
- Core creates exactly one local projection **Service** per import. It has the
  same qualified `*Service` type as the owner Service and
  `metadata.ownerRef: ResourceImport/<name>`. Core never creates a Device,
  Endpoint, or Binding projection.
- Operator/Nix configuration authors one or more same-Zone matching `*Binding`
  resources. Each Binding references the projection's `serviceRef` plus a
  consuming `Guest`, `User`, or `Zone` allowed by the signed factory. Its
  spec is desired consumer intent only; all observations belong in `status`.
  Its Provider controller creates owned `Process`/`Endpoint` children. Binding is
  never exported and never auto-created, owned, or deleted by the import
  controller.
- Sharing is **opt-in on both sides**, over the **same Nix-compiled
  `parentZone` hierarchy and child-local ZoneLink only**. Every hop applies RBAC
  and a capability ceiling and requires an enrolled Noise_KK session. **No FD
  or resource grant crosses a Zone.** Payload bytes flow only over bounded,
  encrypted named streams with a
  per-import session generation, credits/backpressure, cancel, deadline, and
  idempotency (D096, ComponentSession/bus spec). Intermediate controllers see
  ciphertext only.
- Export removal or ZoneLink loss revokes outstanding leases and degrades the
  local projection Service; reconnect revalidates the remote generation and both
  fingerprints so no stale authority survives. D091 update currency propagates
  owner Service → export → import → projection Service → authored Binding → owned
  children. Per D092, leases, sessions, ceremonies, transfers, named streams,
  and stream handles stay internal/high-churn.

#### 8A.1.1 Signed projection factory

Every exportable Provider capability declares this metadata in its signed
descriptor:

| Field | Contract |
| --- | --- |
| `serviceType` | Exact qualified semantic/provider-neutral `*Service` type used by owner and projection |
| `bindingType` | Exact qualified semantic/provider-neutral `*Binding` type permitted to consume that Service |
| `allowedBackingRefTypes` | Closed same-Zone types the owner Service may reference (`Device`, `Endpoint`, or qualified semantic backend types) |
| `allowedBindingTargetRefTypes` | Closed subset of `Guest`, `User`, and `Zone` |
| `projectionSchema` | Strict deny-unknown semantic base schema for the local projection Service; contains standard `providerRef` plus semantic base/import fields and excludes `spec.provider`, implementation-specific fields, FD, secret, raw path/locator, credential, and payload bytes |
| `projectionSchemaFingerprint` | SHA-256 of the canonical projection schema |
| `factoryFingerprint` | SHA-256 binding all fields above and the semantic projection-protocol version, never Provider/adapter identity |

Provider install, Nix build, and API admission verify this metadata. The export's
Service type and fingerprints, the import's expectations, and the installed
consumer Provider factory must match exactly. There is no generic fallback and
no "project the exported resource's type" behavior. The selected implementation
remains the local `providerRef`; strict `spec.provider` contains only
implementation-specific settings on authored owner Services and Bindings.
Core-generated projection Services never contain `spec.provider`; their route
derives from the signed local Provider descriptor, `providerRef`, and
ResourceImport record, and implementation observation belongs only in
`status.provider`. Every conformant Provider MUST accept the
canonical minimal Service/Binding base without `spec.provider`. Base spec/status,
conditions, errors, and fingerprints never contain PipeWire, OTEL, USBIP,
CTAPHID, package, binary, or adapter details; `providerRef` is the sole opaque
implementation selector.

### 8A.2 ResourceExport

`ResourceExport/<name>` lives in the owner/authority Zone. The core
export/import controller owns its base lifecycle; the named Provider's export
adapter owns semantic admission and per-consumer arbitration.

#### 8A.2.1 Base spec (Layer 2, D089)

| Field | Type | Required | Default | Bounds/notes |
| --- | --- | --- | --- | --- |
| `providerRef` | ResourceRef | yes | - | `Provider/<name>` - the local authority Provider that mediates the resource |
| `resourceRef` | ResourceRef | yes | - | **Local qualified owner `*Service` only**; a Device, Endpoint, Binding, or other backing is rejected |
| `serviceType` | string | yes | - | Exact qualified type of `resourceRef`; must equal the factory `serviceType` |
| `projectionSchemaFingerprint` | string | yes | - | Must equal the signed factory fingerprint for the projection Service schema |
| `factoryFingerprint` | string | yes | - | Must equal the installed signed projection factory |
| `operations` | `[string]` | yes | - | Closed operation/capability set consumers may request; 0..64, deny-unknown |
| `arbitration` | enum | yes | - | `exclusive` \| `shared` \| `multiplexed` |
| `quota` | object | no | `{}` | fairness/quota/deadline knobs: `maxConsumers`, `perConsumerRate`, `fairness` (`fifo\|priority\|weighted`), `leaseDeadlineMs`; all bounded |
| `consumerZonePolicy` | object | yes | - | Allowed consumer-Zone selector (child Zones only) plus a capability ceiling that no import may exceed |
| `visibility` | enum | no | `child-zones` | `child-zones` \| `named-zones`; never host-global |
| `updatePolicy` | object | no | manual-disruptive | D091 update/revocation policy (manual disruptive default; auto non-disruptive permitted) |
| `revocationPolicy` | object | no | `{}` | grace window and forced-revoke behavior on export delete / ZoneLink loss |

`spec.provider = { schemaId, schemaVersion, settings }` (D089) adds strict,
deny-unknown, type-specific export policy. It never restates a base field and
never carries raw bytes, paths, device nodes, sockets, tokens, or a backing ref.

#### 8A.2.2 Base status (Layer 2, D088)

`status` base carries advertised/ready/revoking state, `exportGeneration`,
active and pending consumer counts, bounded per-consumer **lease summaries**
(consumer Zone, capability subset, lease state, monotonic lease id digest - no
bytes), owner Service readiness/generation, the two verified fingerprints, and
the D091 `status.update` currency object. `status.provider.details` carries
bounded, redacted implementation observations. **No backing ref, raw bytes,
path, device node, socket, token, or endpoint locator** is advertised.

#### 8A.2.3 Conditions, ownerRef, finalizer, deletion

Closed conditions: `ExportAdvertised`, `AuthorityReady` (owner Service Ready),
`ConsumersAdmitted`, `Revoking`. `metadata.ownerRef` may be the
authority Provider or owner Service. Finalizer ordering on delete: quiesce new
imports → revoke each active lease with the grace window → confirm the owner
Service released all internal per-consumer records/streams → withdraw the
advertisement → clear finalizer. The owner Service and its backing are not
deleted by export cleanup.

### 8A.3 ResourceImport

`ResourceImport/<name>` lives in the consumer (child) Zone. It never names a
remote resource; it names its local `ZoneLink` and a bounded `exportKey`.

#### 8A.3.1 Base spec (Layer 2, D089)

| Field | Type | Required | Default | Bounds/notes |
| --- | --- | --- | --- | --- |
| `providerRef` | ResourceRef | yes | - | **Local** Provider implementation whose import adapter builds the semantic projection |
| `zoneLinkRef` | ResourceRef | yes | - | **Local** `ZoneLink/<name>` to the parent/authority Zone; the only routing anchor |
| `exportKey` | string | yes | - | Bounded opaque key naming the remote export (not a Ref); ≤128 chars |
| `expectedServiceType` | string | yes | - | Expected qualified `*Service`; must equal both factories' `serviceType` |
| `expectedProjectionSchemaFingerprint` | string | yes | - | Must equal the export and local factory projection schema fingerprint |
| `expectedFactoryFingerprint` | string | yes | - | Must equal the export and installed local factory fingerprint |
| `projectionName` | string | yes | - | Stable local name of the one projection Service core creates |
| `requestedCapabilities` | `[string]` | yes | - | Subset of the export `operations`; bounded by the capability ceiling |
| `requestedQuota` | object | no | `{}` | Requested rate/weight/deadline, clamped to the export quota |
| `updatePolicy` | object | no | manual-disruptive | D091 propagation policy |
| `disconnectPolicy` | object | no | `{}` | Behavior on ZoneLink loss/revocation: `degrade` (default) vs `teardown` |

`spec.provider` adds strict type-specific import policy (deny-unknown). No remote
Ref, FD, path, or token appears anywhere.

#### 8A.3.2 Base status (Layer 2, D088)

`status` base: state (`pending\|reachable\|bound\|degraded\|revoked`), observed
remote `exportGeneration` and verified factory/projection fingerprints, local
projection-Service Ref, lease state/count, session generation digest, and
`status.update` currency. Degraded and revoked states propagate to the
projection Service, every Binding that references it, and their children. No raw
locator, backing ref, or bytes.

#### 8A.3.3 Projection ownership, conditions, finalizer

Core creates one `expectedServiceType` projection
(`ownerRef: ResourceImport/<name>`) through the local Provider's import adapter
and keeps its readiness synchronized with the lease. Closed conditions:
`ExportReachable`, `FactoryMatched`, `SchemaMatched`, `Bound`,
`ProjectionReady`, `BindingReferencesRemain`, `Degraded`.

On revoke/link loss, core first marks the projection Service draining/revoked
and refuses new sessions. Binding controllers observe that dependency change,
stop their owned Process/Endpoint children, and report degraded status; the
Binding rows remain because they are operator-owned. On import deletion, the
finalizer waits until all referencing Bindings are deleted or retargeted, then
releases the remote lease, deletes the projection Service and its remaining
provider-owned children, and clears only the import finalizer. It never deletes
or synthesizes a Binding. A remaining Binding produces visible pending cleanup,
not an implicit cascade. Reconnect revalidates generation and both fingerprints
before rebinding.

### 8A.4 Reconcile (core routing + provider adapter)

1. Core validates opt-in on both sides; resolves `resourceRef` to a same-Zone
   qualified owner Service; rejects Device/Endpoint/Binding targets; validates
   local `zoneLinkRef`; and rejects every cross-Zone Ref.
2. Core advertises the export over the ZoneLink to the exact `consumerZonePolicy`
   selector. The advertisement carries only export key, Service type, factory
   and projection fingerprints, operations, arbitration, and capability ceiling.
3. The importing Zone matches all three signed factory values. Missing metadata,
   a mismatch, or an unauthorized Zone fails closed before a lease or projection.
4. The selected Provider's export adapter admits the consumer (arbitration,
   quota, fairness, consent), issues an internal lease, and the import adapter
   builds/updates exactly one local projection Service. Bytes flow only over a
   bounded encrypted named stream.
5. Separately authored Bindings reconcile against that Service and own their
   Process/Endpoint children. D091 currency propagates owner Service → export →
   import → projection Service → Binding → children. Desired intent stays in
   Binding spec; observed realization stays only in status.

### 8A.5 Non-exportable defaults

Credential/token resources are **non-exportable by default**. Per D093, Entra ID
identity stays a same-Zone identity Guest; there is no `ResourceExport` for it
unless a future, explicitly reviewed export capability is added.

The frozen semantic pairs and initial implementing Providers are:

| Semantic pair | Initial Provider | Export policy |
| --- | --- | --- |
| `audio.d2bus.org.AudioService` / `audio.d2bus.org.AudioBinding` | `Provider/audio-pipewire` | exportable |
| `security-key.d2bus.org.SecurityKeyService` / `security-key.d2bus.org.SecurityKeyBinding` | `Provider/device-security-key` | exportable |
| `telemetry.d2bus.org.TelemetryService` / `telemetry.d2bus.org.TelemetryBinding` | `Provider/observability-otel` | exportable |
| `usb.d2bus.org.UsbService` / `usb.d2bus.org.UsbBinding` | `Provider/device-usbip` | policy-gated: Provider, Zone, export, and device policy all opt in |

Every other frozen Provider remains non-exportable. A matching Binding is always
non-exportable even when its Service is approved. Provider dossiers bind their
implementation and strict extensions to these names; they do not own or alias
the semantic namespaces.

### 8A.6 Nix authoring example

```nix
# The semantic AudioService name is Provider-independent. First declare the one
# real owner Service over local Device/Endpoint backing.
d2b.zones.local-root.resources.host-audio = {
  type = "audio.d2bus.org.AudioService";
  spec = {
    providerRef = "Provider/audio-pipewire";
    backingRefs = [ "Device/host-mic" "Endpoint/audio-local" ];
  };
};

# Export only that owner Service.
d2b.zones.local-root.resources.mic-export = {
  type = "ResourceExport";
  spec = {
    providerRef = "Provider/audio-pipewire";
    resourceRef = "audio.d2bus.org.AudioService/host-audio";
    serviceType = "audio.d2bus.org.AudioService";
    projectionSchemaFingerprint = "sha256:...";
    factoryFingerprint = "sha256:...";
    operations = [ "capture" ];
    arbitration = "exclusive";
    consumerZonePolicy = { zones = [ "Zone/work" ]; capabilityCeiling = [ "capture" ]; };
    visibility = "named-zones";
  };
};

# Consumer child Zone: declare the one local uplink used by imports. The
# compiler-only setting chooses the allocator owner but is never emitted into
# Zone.spec or the resource bundle.
d2b.zones.work.parentZone = "local-root";

# The child-local ZoneLink supplies transport and local route/session state;
# the selected parent allocator owns privileged listener and routing effects.
d2b.zones.work.resources.work-uplink = {
  type = "ZoneLink";
  spec = {
    childZoneName = "work";
    transportProviderRef = "Provider/transport-unix";
    transportSettings = {};
    transportCredentials = [];
    disabled = false;
  };
};

# Consumer Zone: import exactly one local AudioService projection.
d2b.zones.work.resources.mic-import = {
  type = "ResourceImport";
  spec = {
    providerRef = "Provider/audio-pipewire";
    zoneLinkRef = "ZoneLink/work-uplink";
    exportKey = "host/mic-export";
    expectedServiceType = "audio.d2bus.org.AudioService";
    expectedProjectionSchemaFingerprint = "sha256:...";
    expectedFactoryFingerprint = "sha256:...";
    projectionName = "host-audio";
    requestedCapabilities = [ "capture" ];
  };
};

# Operator-authored local consumption Binding. The import controller never
# creates this resource; its controller owns the resulting Process/Endpoint.
d2b.zones.work.resources.work-mic = {
  type = "audio.d2bus.org.AudioBinding";
  spec = {
    providerRef = "Provider/audio-pipewire";
    serviceRef = "audio.d2bus.org.AudioService/host-audio";
    targetRef = "Guest/workstation";
    mode = "capture";
  };
};
```

These declarations serialize to canonical ResourceEnvelopes with only local refs;
the resource compiler rejects a non-Service export target, cross-Zone Ref,
factory/schema mismatch, unauthorized consumer Zone, disallowed backing or Binding
target ref, or capability outside the export ceiling. Optional Nix sugar may
lower to these resources only when the canonical ResourceExport,
ResourceImport, projection-Service name, and Binding are stable and inspectable.

### 8A.7 Conformance and tests

Fast hermetic tests (fake ZoneLink/stream/clock/factory) MUST cover: signed
factory absent/mismatch/tamper fail-closed; only an owner Service is exportable;
exactly one same-type projection Service; no Device/Endpoint/Binding projection;
Binding is neither exported nor auto-created; Binding spec is intent-only and
observations are status-only; backing/target allowlists; stable canonical Nix
lowering; finalizer waiting on authored Bindings; update propagation;
classification (audio/security-key/observability approved, USBIP policy-gated,
all others forbidden); canonical minimal base acceptance; semantic-type
preservation across independently selected implementations; rejection of
implementation detail in the base; quota/fairness/deadline/reconnect/revocation;
and no FD/secret/path/raw locator. Slower integration tests use real bounded
encrypted streams for audio, security-key, observability, and policy-gated
USBIP, proving that intermediaries see ciphertext and high-churn sessions/
streams remain internal records.

---

## 8B. Core and singleton authorities (D097)

Per D097, every scarce or singleton backing declares a signed
`AuthorityDescriptor` (schema in
[`ADR-046-resource-object-model` §Authority and cardinality](ADR-046-resource-object-model.md)),
and **core owns the authority index** keyed by `(Zone/scope, authorityClass,
opaqueKeyDigest)`. Core rejects a conflicting authority Resource or Process with
the typed `duplicateConflict` before any external effect; config activation that
would create a second authority goes `Degraded` naming the exact incumbent owner
digest, and restart adopts the exact authority by `ownerProof` (ambiguity
quarantines).

The core/singleton control-plane authorities and their classification:

| Authority | Scope | Cardinality | Owning Resource/service | Exportability |
| --- | --- | --- | --- | --- |
| Zone self-resource | zone | exactly-one | `Zone` self-resource | forbidden |
| Resource store (redb) | zone | exactly-one | core resource store | forbidden |
| Resource API + runtime | zone | exactly-one | core resource API/runtime | forbidden |
| Core controller | zone | exactly-one | `d2b-core-controller` | forbidden |
| Bus `Endpoint` (d2b-bus) | zone | exactly-one | core bus listener `Endpoint` | forbidden |
| Privileged broker | host | exactly-one per Zone | fixed local-root broker | forbidden |
| **Daemon** audit authority | zone | exactly-one | core daemon audit writer | forbidden (Zone-local system of record) |
| **Broker** audit authority | host | exactly-one | broker audit writer (**separate** chain from the daemon) | forbidden (separate system of record) |
| Configuration publisher | zone | exactly-one | core configuration controller | forbidden |
| Artifact catalog | zone | exactly-one | Nix-emitted catalog | forbidden |
| Host substrate allocator / effect authority | host | exactly-one | `Host` + `Provider/system-core` | forbidden |
| Network authority (net Guest + DHCP/DNS/NAT) | zone | exactly-one **per `Network`** | `Network` net-VM authority | forbidden |
| Provider controller | zone | exactly-one per Zone (observability **at-most-one**/zero-or-one) | `Provider` | forbidden |
| `ResourceExport` authority | owner Zone | zero-or-one per exported owner Service | `ResourceExport` | n/a (is the export mechanism) |
| `Quota` scope | zone/scope | exactly-one per scope | `Quota` | forbidden |
| `EmergencyPolicy` scope | zone/scope | exactly-one per scope | `EmergencyPolicy` | forbidden |

The **daemon audit authority and the broker audit authority are two separate
authorities** (distinct writers, distinct chains); neither is transferable, and
exporting audit copies requires a separate explicit D096 export that transfers no
authority. **Provider controller cardinality** is `exactly-one` per Zone for most
Providers; the observability Provider is `at-most-one` (zero-or-one) per Zone.
The network authority is `exactly-one` per `Network` (the net Guest plus its
DHCP/DNS/NAT owner); a second DHCP/DNS/NAT owner on the same `Network` is a
`duplicateConflict`. `Quota`/`EmergencyPolicy` scope uniqueness is
`exactly-one`-per-scope; a second policy/quota claiming the same scope is a
`duplicateConflict`.

**Cross-Zone exportability is deny-by-default.** Core singletons and ordinary
Provider resources/backings are `exportability: forbidden`. D096
`ResourceExport`/`ResourceImport` are the sole typed bridge, and only a signed
factory-bound qualified owner Service may carry `explicit-export`. The initial
approved Provider families are audio, security-key, observability, and
policy-gated USBIP; all others remain forbidden. Their Devices, Endpoints,
Bindings, Credentials, secrets, and backend resources remain non-exportable. A
Zone needing another Zone's telemetry exports the observability owner Service,
which transfers capability/data but never audit-chain/store authority.

**Name disambiguation.** The Volume/virtiofs **`Export`** resource (the virtiofs
share lifecycle owner referenced by D092, which owns its `Endpoint`) is a
**distinct concept** from the D096 **`ResourceExport`** standard ResourceType
(the cross-Zone sharing declaration). They are never conflated: a virtiofs
`Export` is a local Volume-share owner; a `ResourceExport` is the cross-Zone
bridge. A virtiofs share or its Endpoint is not directly exportable; a future
reviewed Provider would need a qualified Service/Binding pair and signed
projection factory, and `ResourceExport` would target only that Service.

### 8B.2 D097 core-audit migration findings

The D097 core-singleton audit surfaced the following missing migration work.
Core-owned items are work items in §17 (ADR046-zone-control-021…023); items whose
implementation destination is a downstream (foreign) crate/spec are recorded here
as findings for that scope to convert into its own work item:

- **Process-global statics → per-Zone.** `USBIP_BACKGROUND_RECONCILE_ACTIVE`,
  `FORCE_SHUTDOWN_GENERATIONS`, and `activation_locks()` are today process-global
  and MUST move to per-Zone provider/resource status or a per-Zone coordinator
  (core-owned; ADR046-zone-control-021).
- **Configuration publisher per-VM → per-Zone staging.** The current per-VM
  configuration staging symbols move to per-Zone staging under the single
  configuration-publisher authority (core-owned; ADR046-zone-control-021).
- **ZoneLink cursor/adoption.** ZoneLink cursor persistence and restart adoption
  are an authority owned by the ZoneLink handler (core-owned;
  ADR046-zone-control-021; see also `ADR-046-zone-routing`).
- **Provider cardinality admission.** Admission MUST enforce Provider controller
  cardinality via the core authority index (core-owned;
  ADR046-zone-control-022; `ADR-046-resource-api-and-authorization`).
- **Quota and EmergencyPolicy implementation/tests.** Scope-uniqueness authority
  implementation and tests (core-owned; ADR046-zone-control-023).
- **`NetworkEffectPort`.** The per-`Network` DHCP/DNS/NAT authority needs a
  `NetworkEffectPort` (D077) rather than ad hoc effects - finding for the
  `resources-network`/`network-local` downstream scope.
- **activation-helper disposition.** The current activation-helper needs an
  explicit v3 disposition - finding for the `activation-nixos` downstream scope.
- **OTEL `vm`-label migration + `d2b-telemetry` bounded emitter** - finding for
  the `telemetry-audit-and-support`/`observability-otel` downstream scope.

Physical/scarce and per-user/session authorities (mic/speaker, security key,
GPU/render-node, video decoder, TPM/swtpm, NIC/uplink/macvtap, Wayland portal,
clipboard, notification sink, PipeWire mediator, Secret Service, systemd-user
manager, Entra login authority, shell supervisor, SigNoz ingest, cloud
subscription control) carry their qualified `AuthorityDescriptor` in the owning
Provider dossier; those per-Provider classifications are refined by evidence with
conservative existing-behavior defaults.

### 8B.1 Authority conformance tests

Hermetic (fake clock/index/adapter) fast tests and slower integration tests MUST
cover, provider-neutrally: a **duplicate race** (two concurrent authority
claimants → exactly one wins, the other gets `duplicateConflict`, no second
effect); a **config collision** (a second configuration-managed authority for the
same `(scope, authorityClass, opaqueKeyDigest)` → hard eval error / `Degraded`
activation naming the incumbent owner digest); each `arbitration` mode
(`exclusive` denies a second holder; `shared`/`multiplexed` admit bounded holders
through the single owner; `partitioned` isolates partitions); **adoption
ambiguity** (restart with two candidate owners or an unverifiable index entry →
quarantine, no open); **cross-Zone import** binds through the single owner with
no duplicate open; **non-exportable rejection** (an `exportability: forbidden`
authority declared as a `ResourceExport` target is rejected); **update drain**
(D091 upgrade drains consumers before recycling the authority); and **each
initial singleton** (Zone/store/API/bus/controller/broker/audit/config
publisher/allocator/net-VM-per-Network/`Quota`/`EmergencyPolicy` scope) admits
exactly one owner and rejects a second.

### 8B.3 Hardware singleton authorities (D097)

The D097 hardware audit classifies every scarce/physical/kernel backing. These
are **Host-global** authorities (keyed by `(Host, authorityClass,
opaqueKeyDigest)`): two Zones on the same host collide over one physical backing,
so the index admits exactly one owner across all Zones on the host.

| Hardware authority | Scope | Cardinality | Owner | Arbitration | Exportability |
| --- | --- | --- | --- | --- | --- |
| GPU/DRM full device (primary/VFIO) | physical-device (Host-global) | zero-or-one per GPU | `Device`/`Provider/device-gpu` | exclusive | forbidden (FD/local kernel) |
| GPU render-node | physical-device (Host-global) | bounded-many per render node | `Provider/device-gpu` | **shared** (explicitly) | forbidden |
| GPU-owned `udmabuf`/video subresources | subresource of the GPU authority | n/a (internal) | GPU authority | n/a | not a separate resource - declared **authority subresource/DeviceGrant**, never a new Provider |
| Per-Guest swtpm state + marker | physical-device (Host-global) | exactly-one per Guest | `Provider/device-tpm` | exclusive | forbidden; **state never wiped** (device-tampering signal) |
| Physical TPM | physical-device (Host-global) | exactly-one (host singleton) | `Provider/device-tpm` | exclusive | forbidden |
| Physical USB backing | physical-device (Host-global) | zero-or-one per Core-derived trusted identity digest | Any authority Service implemented by a USB or security-key Provider, initially `Provider/device-usbip` and `Provider/device-security-key` | exclusive through the identical `(Host, physical-usb-backing, opaqueKeyDigest)` tuple | forbidden directly; a policy-gated semantic USB Service or security-key Service may mediate it |
| `usbip-host` kernel module | host (Host-global) | exactly-one host-global | `Provider/device-usbip` | exclusive | forbidden |
| Per-Network USBIP listener + firewall | host (Host-global) | exactly-one per Core-derived Network UID/signed-policy-port digest | `Provider/device-usbip` relay `Endpoint` (never `Network` authority) | multiplexed; conflict `usbip-network-relay-authority-conflict` | forbidden |
| External NIC / macvtap `parentInterface` | physical-device (Host-global) | zero-or-one per interface | `Network`/`Provider/network-local` | `passthru` **globally exclusive across all Zones**; `bridge`/`private`/`vepa` per explicit policy | forbidden |
| Host-shared KVM (`/dev/kvm`) | host (Host-global) | shared grant, one grant authority | **`Provider/system-core`** (host substrate/effect authority) | shared | forbidden |
| Host-shared vhost-vsock (`/dev/vhost-vsock`) | host (Host-global) | shared grant, one grant authority | **`Provider/system-core`** | shared | forbidden |
| vsock CID allocation | host (Host-global) | globally-unique per CID | core allocator | exclusive | forbidden (CID never crosses a Zone) |
| Fixed listener port namespace | host (Host-global) | exactly-one per port | the listener's `Endpoint` | exclusive | forbidden (fixed ports are `Endpoint` resources) |
| Host Nix store | host (Host-global) | exactly-one | Host substrate | shared read | forbidden |
| Per-Guest store-view writer | physical-device (Host-global) | exactly-one writer per Guest | store-view writer | exclusive | forbidden |
| Network TAP / bridge | zone | exactly-one per TAP/bridge | `Network` authority | exclusive | forbidden |

**KVM/vhost-vsock ownership (no 28th Provider).** `/dev/kvm` and
`/dev/vhost-vsock` are **host-shared kernel devices owned by
`Provider/system-core`** (the existing Host substrate/effect authority, which
already declares the `kvm` `HostCapabilityClass`) and granted to runtime
Providers via the D077 EffectPort/LaunchTicket DeviceGrant. They are **not** a
`Device` `busClass` (the closed set stays `usb|hidraw|drm|pci|tpm`) and do **not**
require a `Provider/device-kvm` - any such reference resolves to a
`Provider/system-core` host-shared grant (finding for the foreign dossiers that
still name `Provider/device-kvm`). GPU-owned `udmabuf`/video and per-session
`vhost-vsock` tokens stay **authority subresources / DeviceGrants**, never new
Providers.

**D096 exportability of hardware.** GPU, KVM, physical/emulated TPM, host Nix
store, store-view writer, and macvtap/NIC `parentInterface` require an FD or
local-kernel authority and are **non-exportable** (`forbidden`). **USBIP capability is policy-gated exportable** only through the
factory-bound qualified USBIP Service/Binding pair and typed CTAPHID-free USBIP
protocol. The physical Device and fixed listener Endpoint remain non-exportable.
This supersedes any stale direct-hardware export claim.

### 8B.4 D097 hardware-audit consistency findings

- **TPM flush TTL** - corrected in `ADR-046-resources-device` to the D094
  canonical successful-EphemeralProcess TTL (`1h`), matching the device-tpm
  dossier; a shorter `15m` would need an explicit justified override.
- **`Provider/device-kvm` stale reference** - resolves to a
  `Provider/system-core` host-shared grant (no 28th Provider, no `kvm` busClass);
  finding for the foreign dossiers/validation that still name it.
- **`device-tpm` physical TPM** - the device-tpm dossier MUST explicitly
  implement or explicitly reject physical TPM (no second TPM Provider); finding
  for that downstream dossier.
- **macvtap `passthru`** conflict is Host-global across all Zones (covered by the
  Host-global index); **`Create`/`DeleteBridge`** must be `NetworkEffectPort`
  ops (finding for `resources-network`/`network-local`).
- **USBIP port 3240** multiplex behavior and the fixed port become an `Endpoint`
  resource; finding for the usbip/network downstream scope.
- **vsock CID hardcoded `2`** migration to the global CID allocation authority -
  corrected in `ADR-046-zone-routing`.
- **store-view gcroots path** code-vs-spec: **code wins**; finding for the store
  spec to align its prose to the implemented gcroots path.
- **ZoneLink range capacity/quota** - an explicit bounded quota; corrected in
  `ADR-046-zone-routing`.

---

## 9. Bootstrap authorization

Before any Role/RoleBinding resources exist, the Zone runtime has one compiled
non-configurable bootstrap authorization policy.

### 9.1 Subjects

Exactly two subjects are permitted under bootstrap:

- `Provider/system-core`
- `Provider/system-minijail`

Any other subject is denied. There is no bootstrap fallback for any other
Provider, controller, service, operator, or CLI connection.

### 9.2 Verbs

Bootstrap grants only:

| Verb | ResourceTypes | Scope |
| --- | --- | --- |
| `get`, `list` | all | Store recovery read |
| `create` | Zone | Initial self-resource creation |
| `create` | Provider | Initial system-core and system-minijail records |
| `create` | Host | First Host creation |
| `create` | User | Local User creation |
| `create` | Role | Initial core-controller Roles |
| `create` | RoleBinding | Initial core-controller RoleBindings |
| `update-spec`, `update-status` | Zone, Provider, Host, User | Initial reconciliation |
| `update-status` | Role, RoleBinding | Initial authorization index publication |
| `connect`, `invoke` | store-lifecycle, core-controller services | Core process launch only |

No wildcard Provider, resource, runtime authority, or cross-Zone access is
granted. No config field widens bootstrap authorization.

### 9.3 Properties

- Bootstrap policy is compiled into the Zone runtime binary; it is not
  configurable, overridable, or loaded from any file or resource at runtime.
- Bootstrap admission checks all structural constraints (Zone/session/route
  correctness, installed Provider/API binding, ref integrity, generation/revision,
  budget) as if they were normal requests.
- Every bootstrap action is structurally audited.
- Bootstrap authorization is superseded completely and immediately after the
  first full set of Role/RoleBinding resources is published and the
  authorization handler reports `IndexBuilt=True` for all initial bindings.
- After bootstrap supersession, any request using only bootstrap subjects/verbs
  that are not covered by stored Roles is denied.
- The transition from bootstrap to stored RBAC is atomic: the authorization
  handler swaps its in-memory policy in one transaction; there is no window
  where both policies are active.

### 9.4 Recovery

If the authorization store is corrupted or the Role/RoleBinding index is
unreadable:

- no new requests are admitted (fail-closed);
- the out-of-band safety path (see §12.6) allows a privileged local operator
  to trigger a destructive reset that re-enters bootstrap for a fresh
  initialization;
- the out-of-band path is authenticated by OS-level mechanisms (uid=0 or
  equivalent); it cannot be invoked remotely or through d2b-bus.

---

## 10. Zone self-resource and ZoneLink relationship

### 10.1 Zone self-resource invariant

Every Zone store contains exactly one `Zone/<zone-name>` resource where
`<zone-name>` equals `store_meta.zone_name`. This invariant is enforced:

- at store open: if `Zone/<zone-name>` does not exist, bootstrap creates it;
- at every mutation: create/update requests cannot change `metadata.name` or
  `metadata.zone` to violate the self-resource constraint;
- at every open: the Zone runtime reads and verifies the self resource before
  admitting any API request;
- at upgrade: if `store_meta.zone_uid` differs from the Zone resource UID,
  the store is quarantined (this indicates replacement or corruption);
- a second `Zone/<any-other-name>` resource in the same store cannot be
  created; the ResourceType `Zone` is restricted to cardinality 1 within a
  store.

### 10.2 No cross-Zone resource references

Ordinary resource refs (`<ResourceType>/<resource_name>`) always resolve in
the evaluating resource's own Zone. There is no URI scheme, Zone prefix, or
cross-Zone address in any ref field.

Prohibited:

```yaml
ownerRef: Zone/other-zone::Process/foo    # REJECTED
subjectRef: other-zone:User/alice         # REJECTED
```

A parent Zone accesses child Zone resources by calling the child Zone's
`d2b.resource.v3` API over the allocator-bound ComponentSession represented by
the child-local ZoneLink. The parent's
local resources (e.g. a local `Process` that represents a child Zone workload)
do not contain cross-Zone ResourceRefs; they contain only local refs valid in
the parent Zone.

### 10.3 ZoneLink parent/child access model

```
Parent Zone / allocator                 Child Zone
  private route allocation                Zone/guest
  (privileged listeners/routes)           ZoneLink/guest-uplink
         |                                (resource, cursor, intent state)
         | allocator-bound ComponentSession        |
         |------- d2b.resource.v3 Get ---------->  |
         |<------- response (child data) --------- |
```

The parent receives only data the child's authorization engine permits. The
parent's mapped subject in the child Zone is established through the
transport Provider's authenticated ComponentSession using
`spec.transportProviderRef`, `spec.transportSettings`, and resolved
`spec.transportCredentials`. The parent cannot read child resources it is not
authorized for; the child authorization engine is the sole arbiter. The
ZoneLink spec and all of its refs resolve in the child store. Parent-side
allocation/route state is not a resource and is not a reciprocal ZoneLink.
The enclosing child's compiler-only `parentZone` chooses which parent allocator
appears on the left side of the diagram; no ZoneLink field can select or
override that owner.

A disconnected child uplink:

- records outbound local ZoneLink intents in the child but does not claim
  parent resource state changed;
- on reconnect, re-authenticates and applies/rejects pending intents against
  the current parent revision;
- never lets either endpoint use cached remote resource state for authorization
  decisions.

---

## 11. Core controller algorithms

### 11.1 Zone controller algorithm

The `configuration publication` handler reconciles Zone:

1. **Startup relist**: List `Zone/<zone-name>`; verify `metadata.uid` matches
   `store_meta.zone_uid`; quarantine if mismatch.
2. **Spec validation**: Validate that `Zone.spec` is exactly `{}`; any
   non-empty object is rejected with `zone-spec-invalid` and `ConfigValid=False` condition (handler sets Phase `Failed` for this resource).
3. **Configuration activation**: Read integrity-pinned candidate bundle;
   validate all Provider packages/APIs/config/refs/owners/RBAC/budgets;
   stage inactive resources in bounded transactions; atomically activate one
   configuration revision; trigger affected resources/providers/controllers.
4. **Status update**: Write current `apiCatalogRevision`, `policyRevision`,
   `configurationRevision` from store_meta; aggregate mandatory handler phases
   into `coreControllerPhase`.
5. **Reconcile result**: `converged` when store_meta revisions match status and
   all mandatory handlers are Ready; `pending` during activation; `degraded` if
   optional handlers impaired.

The Zone controller never directly touches Provider, Role, RoleBinding, Host,
Guest, or Process resources. Those are owned by their respective handlers.

### 11.2 ZoneLink controller algorithm

The child Zone's `zone link/delegation` handler reconciles its local uplink:

1. **Startup relist**: List all ZoneLink resources; for each, read cursor from
   the child-local `zone_link_cursors` table; verify `childZoneName` equals the
   local Zone self-name and `childZoneUid` against the local Zone UID.
2. **Transport resolution**: Resolve `spec.transportProviderRef`; wait for
   that same-child Provider to be `Ready`. Submit the ZoneLink UID, child
   identity, and transport binding to the parent allocator selected by the
   compiler-only `parentZone` map. The allocator alone creates privileged
   listeners/route namespace and returns the pre-bound transport through
   sealed bootstrap authority.
3. **Authentication**: Establish the allocator-bound ComponentSession; perform
   the Noise handshake; authenticate the parent allocator and present the
   child-local subject.
4. **Child authorization check**: Require the parent to acknowledge that the
   child subject and requested ceiling fit the sealed allocation; set
   `ChildAuthorized`. Parent access to `d2b.resource.v3` is independently
   checked by the child authorization engine on every call.
5. **Cursor recovery**: Compare `childZoneUid` with the local Zone UID; if it
   changed, reset local cursor state and require a fresh parent allocation.
6. **Watch/relist**: Receive parent route/export advertisements from
   `lastAppliedRevision`; if the cursor expired, relist and re-watch.
7. **Status update**: Write connection state, cursor values, link epoch.
8. **Reconnect loop**: On disconnect, attempt reconnect with exponential backoff;
   write `lastDisconnectedAt`, `connected=false`, `Unknown` phase until
   reconnected.
9. **Drain on delete**: On `deletionRequestedAt`, gracefully close session,
   apply drain finalizer, update status to reflect drain.

ZoneLink does not copy parent or child resources across stores. The child store
contains only the local ZoneLink row/cursors/intents; parent allocator and
route-engine state is session/allocation scoped.

### 11.3 Provider lifecycle algorithm

The `provider lifecycle` handler reconciles Provider:

1. **Trust/conformance**: Resolve the artifact catalog entry for `spec.artifactId`;
   verify `digest`, `signatureId`, `trustEpoch` against the installed trust store;
   verify `conformanceAttestationDigest` against known attestation store.
   Set `PackageTrusted` and `trustResult`.
2. **Config validation**: Validate `spec.config` against the JSON Schema
   identified by the artifact catalog's `configSchemaDigest`. Set `ConfigValid`.
3. **Dependency resolution**: For each alias in `spec.dependencies`, resolve
   to a Ready Provider; verify service fingerprint compatibility. Set
   `DependenciesReady`.
4. **API binding**: Pass component descriptors and exported ResourceTypes to
   the API catalog handler; verify no name collisions; intersect permission
   claims with Zone policy; install schemas.
5. **Component launch**: For each component in `spec.components`, create or
   update the owned Process resource with the corresponding template/placement.
   Bootstrap exception components (`system-core`, `system-minijail`) skip this
   step.
6. **Readiness**: Once all required component Processes reach `Ready`, write
   Provider status to `Ready` with `ApiPublished=True`. Write `providerGeneration`.
7. **Upgrade**: On spec change (generation increment), execute lifecycle policy
   (`drainAndReplace`, `rolling`, `immediate`); update component Processes.
8. **Quarantine**: On trust or conformance failure, set `quarantined=true`; stop
   all component Processes; withdraw exported ResourceTypes; leave state Volumes
   intact for incident investigation.

### 11.4 Authorization algorithm (Role and RoleBinding)

The `authorization` handler reconciles Role and RoleBinding:

1. **Role reconciliation**:
   a. Validate rules (verbs, ResourceType names, ref formats, explicit wildcard
      restriction, and relay origin/ZoneLink/target bounds).
   b. Build index entry: for each (subject, Zone, ResourceType, verb, subresource,
      resourceNames, executionRefs) tuple, add an allow entry.
   c. Write `IndexBuilt=True`, `phase=Ready`.
   d. Invalidate authorization caches for all subjects covered by changed rules.

2. **RoleBinding reconciliation**:
   a. Resolve `roleRef` to current Role UID and generation.
   b. Resolve each subject in `subjects` to current UID; note unresolved.
   c. Resolve `externalPrincipalSelector` if present.
   d. Apply `scopeNarrowing` as an intersection with Role rules.
   e. For a relay-bearing Role, verify the exact adjacent-Zone enrollment
      selector, governing ZoneLink owner, and core-generated or explicit
      admin-policy provenance.
   f. Honor operator revocation state and any pending deletion request.
   g. Build index entry: for each subject × (narrowed rule) add allow entry.
   h. Increment Role `activeBindingCount`.
   i. Write `phase=Ready`, `IndexBuilt=True`, `roleResolved=true`.
   j. Invalidate authorization caches for all covered subjects.

3. **Index swap**: After every batch of Role/RoleBinding commits, the
   authorization handler atomically swaps the in-memory evaluator to the new
   generation. The swap is MVCC-safe: in-flight requests retain their original
   context; new requests use the new index.

4. **Cache invalidation**: The authorization cache (positive decisions cached
   under exact attributes and short expiry) is invalidated per subject upon:
   a. Any Role change affecting that subject.
   b. Any RoleBinding change affecting that subject.
   c. Zone policy revision change.
   d. Provider/API catalog revision change.
   Denials are never cached as allows.

5. **Deletion handling**:
   - **Role deletion**: Blocked by `core.role-binding-drain` until all
     RoleBindings referencing this Role are deleted. The handler decrements
     `activeBindingCount` on each binding deletion; clears the finalizer when
     count reaches 0.
   - **RoleBinding deletion**: The final atomic transaction removes the RBAC
     index entry, emits the `Deleted` revision event, and removes the resource
     row from the Zone store in one redb write; no observable intermediate state
     exists between index removal and row removal (§6.6).

### 11.5 Configuration generation cleanup algorithm

The `configuration publication` handler executes the following sequence when
activating a new Nix-authored resource bundle generation. See §14.11 for the
full normative cleanup contract including status/conditions/audit.

1. **Diff computation**: Compare the new bundle's config-owned resource set
   against all config-owned resources currently in the store. Resources present
   in the new bundle are upserted (created if new; spec-updated if changed).
   Resources that were config-owned in any prior active generation but absent
   from the new bundle are "cleanup candidates".

2. **Immediate generation activation**: The new bundle generation is written as
   the active configuration revision in `store_meta` atomically. The Zone runtime
   begins serving requests under the new generation without waiting for cleanup.
   `Zone.status.activeConfigurationGeneration` updates to the new value.
   `ConfigurationCurrent=True` is set.

3. **Async Delete for absent config-owned resources**: For each cleanup candidate:
   a. If `metadata.deletionRequestedAt` is null, set it to the current timestamp.
   b. Add the resource-type-specific core finalizer if not already present.
   c. Emit an async reconcile trigger to the owning handler.
   Deletes are issued concurrently, bounded by the store transaction limit.
   Resources with `managedBy=controller` (including bootstrap-created resources
   such as `Provider/system-core`, `Provider/system-minijail`, and the Zone
   self-resource) or `managedBy=api` are never cleanup-deleted by generation change.

4. **Controller-created resource preservation**: The handler tracks only bundle
   records. Controller-created children - process instances, ephemeral records,
   dynamic volumes - have no bundle entry and are never deleted by generation
   change. The owning controller reconciles its dynamic children when the parent
   resource's spec or phase changes via watch trigger.

5. **Owner controller child cascade**: When a config-owned resource has
   `deletionRequestedAt` set, its owning controller cascades deletion to owned children:
   - `Provider` awaiting deletion: drains component Processes per `upgradePolicy`; state
    Volumes preserved unless the policy specifies destruction.
   - `Role` awaiting deletion: blocked by `core.role-binding-drain` until all dependent
    RoleBindings are deleted or re-bound to another Role.
   - `RoleBinding` awaiting deletion: atomic RBAC index removal (see §6.6); resource row
    removed in same transaction.
   - `ZoneLink` awaiting deletion: graceful session close; pending-intent drain;
    `core.zone-link-drain` finalizer cleared.

6. **Cleanup status tracking**: The configuration publication handler tracks
   resources pending deletion via the standard watch mechanism:
   - While any remain: `generationCleanupPending=true`,
    `cleanupPendingCount=<N>`, `GenerationCleanupPending=True` condition with
    reason `PendingCleanup`; Zone.status.phase = `Degraded`.
   - When all complete: fields reset, condition cleared, Zone.status.phase
    reverts to the aggregate mandatory handler phase.

7. **Stuck-cleanup Degraded**: If any resource awaiting deletion has not
   completed finalizer drain within `cleanupStuckThreshold` (default 5 minutes),
   `GenerationCleanupFailed=True` is additionally set. Zone.status.phase remains
   `Degraded` until the stuck resource clears. The condition message names the
   ResourceType only (not the resource name or any spec content). The operator
   inspects the resource's finalizer list and owning controller conditions.

8. **Prior generation retention**: Prior bundle files are retained in the Zone
   store bundle directory up to the configured retention count (default 3, range
   1..16). When the count is exceeded, the oldest retained prior bundle beyond
   the retention window is de-referenced from the rollback target (resources
   with `deletionRequestedAt` set from that generation continue their individual
   finalizer drain but lose rollback association).

9. **Rollback**: An operator with `verb=zone.config-rollback` may restore the
   retained prior generation bundle atomically: re-creates deleted resources from
   the prior spec, issues async Deletes for superseded-generation additions. The
   rollback itself triggers a new cleanup cycle.

---

## 12. Async reconciliation triggers

### 12.1 Trigger reasons

These trigger reasons are relevant for Zone control ResourceTypes. All are
from the common closed set in `ADR-046-resource-reconciliation`.

| Trigger | Zone | ZoneLink | Provider | Role | RoleBinding |
| --- | --- | --- | --- | --- | --- |
| `spec-generation-changed` | ✓ | ✓ | ✓ | ✓ | ✓ |
| `owned-resource-changed` | ✓ | - | ✓ | - | - |
| `dependency-changed` | - | ✓ | ✓ | - | ✓ |
| `dependency-ready` | - | ✓ | ✓ | - | - |
| `deletion-requested` | ✓ | ✓ | ✓ | ✓ | ✓ |
| `finalizer-required` | ✓ | ✓ | ✓ | ✓ | - |
| `controller-generation-changed` | ✓ | ✓ | ✓ | ✓ | ✓ |
| `Provider-generation-changed` | - | - | - | - | - |
| `policy-changed` | ✓ | - | ✓ | - | - |
| `execution-status-changed` | - | - | - | - | - |
| `scheduled-observe` | ✓ | ✓ | ✓ | - | ✓ |
| `retry-due` | - | ✓ | ✓ | ✓ | ✓ |
| `manual-reconcile` | ✓ | ✓ | ✓ | ✓ | ✓ |
| `startup-relist` | ✓ | ✓ | ✓ | ✓ | ✓ |

`dependency-changed` for RoleBinding is triggered when the referenced Role
changes (subject resolved status changes). `dependency-ready` for Provider
is triggered when any alias dependency Provider transitions to Ready.

### 12.2 Cross-type triggers

| Event | Triggers reconcile of |
| --- | --- |
| Zone spec/status change | No automatic cross-type trigger; handlers watch their own types |
| ZoneLink status Connected | Provider lifecycle handler if transport Provider is ZoneLink-dependent |
| Provider reaches Ready | Every RoleBinding that includes this Provider as a subject |
| Provider quarantined | Zone status update; no cascade to Roles/RoleBindings |
| Role spec change | All RoleBindings referencing this Role (`dependency-changed`) |
| RoleBinding subject UID change | RoleBinding itself (`scheduled-observe` detects UID mismatch) |
| Role deletion finalizer drain | RoleBindings referencing it receive `deletion-requested` from core |

### 12.3 Convergence and suppression

Core may suppress a reconcile hint for a Zone control resource when:

- spec generation equals status `observedGeneration`;
- controller generation is current;
- no dependency/provider/policy generation change is pending;
- no deletion/finalizer/owner/retry cause exists;
- conditions do not require re-evaluation.

Core must NOT suppress:
- any trigger for `policy-changed` affecting Role/RoleBinding;
- any `deletion-requested` or `finalizer-required`;
- any scheduled-observe for RoleBinding subject re-resolution or revocation state refresh;
- any Provider trust expiry check.

### 12.4 Observe intervals

| ResourceType | `observeIntervalSeconds` | Rationale |
| --- | --- | --- |
| Zone | 60 | Periodic store health snapshot |
| ZoneLink | 30 | Transport reachability and cursor health |
| Provider | 300 | Trust epoch and conformance re-check |
| Role | null | No external drift; reconcile only on change |
| RoleBinding | 300 | Subject UID drift detection; revocation state refresh |

These are the declared `observeIntervalSeconds` values for core-controller
handlers. Provider controllers declare their own intervals.

---

## 13. Security, audit, OTEL, and errors

### 13.1 Authorization attributes for Zone control types

Every decision on a Zone/ZoneLink/Provider/Role/RoleBinding resource evaluates:

```text
Zone         (from authenticated session)
subject      (from AuthenticatedSubjectContext.subjectRef)
ResourceType (Zone | ZoneLink | Provider | Role | RoleBinding)
subresource  (spec | status | metadata | finalizers | deletion)
verb         (get | list | watch | create | update-spec | update-status | ...)
resource name
executionRef / domain / userRef scope  (not applicable to Zone control types)
Provider/controller generation
```

Zone control type modifications are restricted:

| Operation | Permitted subjects |
| --- | --- |
| Create/delete Zone | Bootstrap only (zone-drain is core-initiated) |
| Update Zone spec | Bootstrap; then stored RBAC via core-controller Role |
| Create/delete ZoneLink | Admin subjects per Zone policy |
| Create/delete Provider | Admin subjects per Zone policy |
| Create/delete Role | Admin subjects per Zone policy; core-generated Roles owned by system-core |
| Create/delete RoleBinding | Admin subjects per Zone policy; core-generated bindings owned by system-core |
| Update-status (any) | Owning core-controller handler only |
| Update-finalizers | Core only (no external finalizer owners for Zone control types except Provider controllers per §4.3.3) |

### 13.2 Audit records

Audit records for Zone control type operations contain:

- `subject` (ResourceRef/UID of the authenticated subject)
- `zone` (name)
- `resourceType` (Zone | ZoneLink | Provider | Role | RoleBinding)
- `resourceRef` (`ResourceType/name`) and `resourceUid` of the affected resource
- `verb`/`subresource`
- `expectedRevision`/`currentRevision`/`resultRevision`
- authorization decision (`allowed | denied`) and policy revisions consulted
- `operationId`/`correlationId`
- fixed outcome/error class

Audit records for these types MUST NOT contain:

- Role rule payloads (verbs/resourceTypes/resourceNames);
- RoleBinding subject lists;
- Provider package digests or config;
- Zone policy fields;
- ZoneLink transport credentials or cursor values;
- any credential byte, process data, or terminal byte.

The audit sink is the same fixed OTEL-separate audit path described in the
parent ADR. Zone control type audit records share no sink with runtime
operational audit.

### 13.3 OTEL telemetry

Metrics for Zone control types use closed label sets:

| Metric | Labels |
| --- | --- |
| `d2b.zone.reconcile.duration_seconds` | `handler`, `resource_type`, `outcome` |
| `d2b.zone.authorization.decisions_total` | `resource_type`, `verb`, `decision` |
| `d2b.zone.provider.component_phase` | `component_type`, `phase` |
| `d2b.zone.zoneLink.connected` | (no label; gauge per link reported as aggregate) |

Never label: subject names, resource names, provider package digests, Zone
names, RoleBinding subject counts, config values, or any credential-adjacent
string.

Spans for Zone control reconciliation:

- one span per reconcile attempt: `d2b.zone.reconcile`
- child spans for trust check, config validation, component launch, API binding
- no resource payload in span attributes; only stable codes and durations
- parent trace propagated from d2b-bus into reconcile context

### 13.4 Errors

Stable error codes for Zone control types:

| Error code | Meaning |
| --- | --- |
| `zone-self-resource-mismatch` | Store zone_name/zone_uid differs from Zone resource |
| `zone-spec-invalid` | Zone spec is not exactly `{}` |
| `zone-link-transport-unavailable` | Transport Provider unavailable or unreachable |
| `zone-link-child-auth-denied` | Parent allocator rejected the authenticated child subject or requested allocation ceiling |
| `zone-link-child-uid-changed` | Child Zone UID changed; cursor reset required |
| `provider-package-digest-mismatch` | Package content differs from declared digest |
| `provider-trust-revoked` | Trust epoch revoked or signature invalid |
| `provider-conformance-failed` | API conformance attestation check failed |
| `provider-config-invalid` | Config failed schema validation |
| `provider-api-name-collision` | Exported ResourceType name already bound by another Provider |
| `provider-dependency-cycle` | Provider dependency alias creates a cycle |
| `role-wildcard-denied` | Explicit wildcard attempted by non-core-controller Role |
| `role-unknown-verb` | Rule contains an unknown verb token |
| `role-relay-grant-restricted` | Relay is unbounded, self-asserted, lacks ZoneLink ownership/exact adjacent-Zone enrollment selector, or lacks explicit admin-policy provenance |
| `role-unknown-resource-type` | Rule names a ResourceType not in Zone API catalog |
| `rolebinding-role-not-found` | `roleRef` does not resolve to an existing Role |
| `rolebinding-subject-type-invalid` | Subject ResourceType is not a permitted subject type |
| `rolebinding-scope-exceeds-role` | scopeNarrowing attempts to grant more than Role allows |
| `bootstrap-subject-denied` | Non-bootstrap subject attempted a bootstrap-only verb |

All error messages are bounded (max 512 bytes), UTF-8 validated,
control-character sanitized, and must not contain secrets, credentials, paths,
process data, or any other sensitive content.

### 13.5 Authentication boundary

Zone control type mutations require:

- authenticated ComponentSession with verified `AuthenticatedSubjectContext`;
- the subject must be current (its resource UID must match the last seen UID
  for that subject ref in the authorization index);
- subject UID changes invalidate existing sessions - any session whose subject
  resource UID has changed is denied on next method call and must re-establish;
- bootstrap Providers (system-core, system-minijail) are authenticated by
  the compiled bootstrap policy through their fixed process identity, not by
  an external token.

### 13.6 Emergency disable

An emergency disable is declared by creating or updating an `EmergencyPolicy`
resource (§8) with `spec.enabled=true`. See §8 for the full EmergencyPolicy
spec and controller algorithm. When active, it applies to Zone control types as
follows:

- Zone emergency disable stops all new resource API admissions; ongoing bounded
  operations drain to their effective deadline (minimum `drainDeadlineSeconds` across
  all enabled policies);
- active ZoneLinks receive a graceful disconnect signal;
- Provider component Processes are stopped without deleting their Process resources;
  reconciliation resumes when all effective `stopProviderProcesses` policies are deactivated;
- Role/RoleBinding evaluation continues for ongoing admitted operations until
  deadline;
- Zone status reflects the admission stop via a fixed condition; it is not a Zone
  spec change and does not directly set Zone `phase`.

---

## 14. Nix authoring and resource bundle

This section is normative for the d2b 3.0 Nix configuration surface, including Quota and EmergencyPolicy resources.

**Unified authoring syntax**: All Zone control ResourceTypes use a single
uniform structure. `metadata.name` is derived from the resource attrset key,
`metadata.zone` from the Zone attrset key, and `apiVersion` defaults to
`"resources.d2bus.org/v3"`. `status` is omitted - it is read-only; the Zone runtime fills
all status fields. Core fills `uid`, `generation`, `revision`, `createdAt`,
`updatedAt`, `ownerRef` (where applicable), `finalizers`, and
`deletionRequestedAt`:

```nix
d2b.zones.<zone>.resources.<name> = {
  type = "<ResourceType>"; # "Zone"|"ZoneLink"|"Provider"|"Role"|"RoleBinding"|"Quota"|"EmergencyPolicy"|"Credential"|...
  spec = {
    # Exact ResourceType spec fields - mirrors ResourceSpec.spec for this type.
    # No renaming or re-nesting of field names.
  };
};
```

**Artifact catalog**: Derivation-valued inputs (Provider packages, NixOS
system closures, and similar build outputs) are configured exclusively in the
separate artifact catalog:

```nix
d2b.artifacts.<id> = {
  package = <derivation>;           # Nix derivation; required
  type    = "provider"              # "provider" | "nixos-system" | ...
            | "nixos-system" | ...; # closed enum; further types added via ADR
};
```

`<id>` is a bounded label (`^[a-z][a-z0-9-]*$`, max 128 chars). The resource
compiler builds/includes/hashes each derivation, validates the catalog for
type consistency and ID uniqueness, checks trust/conformance, and emits a
**private integrity-pinned artifact catalog** mapping each ID to its type,
content digest, and closure metadata. Store paths are private catalog
implementation data and are **never** exposed in resource spec fields, status
fields, audit records, or OTEL attributes.

ResourceSpecs remain **pure direct schema mirrors**: they reference artifacts
by plain bounded IDs. Provider spec uses `artifactId` (string); Guest system
spec uses `systemArtifactId` (string). These are not `*Ref` fields because
`Artifact` is not a ResourceType. A missing or wrong-type artifact ID is a
NixOS build failure.

**Schema-driven Nix options**: For each registered ResourceType, the Nix module
generator reads the corresponding ResourceTypeSchema JSON and emits Nix option
declarations (types, defaults, documentation strings) for every field in `spec`.
There is no second hand-authored Nix option vocabulary for resource spec fields.

**Build validation**: The resource compiler serializes each Nix `spec` attrset
to canonical JSON and validates it against the ResourceTypeSchema for the given
`type`. Provider `spec.config` is additionally validated against the exact signed
Provider's config schema (located via the artifact catalog's `configSchemaDigest`
for the named `artifactId`). The validated canonical JSON is the input to
per-resource digest computation and the Zone resource bundle.

See §14.8 for the canonical envelope format, §14.9 for the bundle format, and
§14.10 for the full three-phase validation pipeline.

### 14.1 Zone declaration

```nix
d2b.zones.local-root = {}; # parentZone is forbidden on this Zone

d2b.zones.dev = {
  parentZone = "local-root";
  resources = {};
};
```

`parentZone` is a compiler-only plain Zone name, not a `ResourceRef`. It is
required on every non-root Zone, forbidden on `local-root`, and never emitted
into a ResourceEnvelope or `Zone.spec`, which remains exactly `{}`. The
compiler resolves it to one declared parent and writes that edge only into the
sealed private allocator-bootstrap topology. It rejects a missing or unknown
parent, self-parenting, conflicting scalar definitions, cycles, and ancestry
paths longer than 16 Zone names.

The Zone self-resource name derives from the Zone attrset key. It is created by
the Zone runtime on first initialization with `managedBy=controller` and is
never included in the resource bundle. Authors do not declare a `type = "Zone"`
resource under `resources`.

### 14.2 ZoneLink declaration

```nix
d2b.zones.guest.parentZone = "local-root";

d2b.artifacts.transport-unix = {
  package = pkgs.d2b-provider-transport-unix;
  type    = "provider";
};

d2b.zones.guest.resources.transport-unix = {
  type = "Provider";
  spec = { artifactId = "transport-unix"; config = {}; };
};

d2b.zones.guest.resources.guest-uplink = {
  type = "ZoneLink";
  spec = {
    childZoneName = "guest";
    transportProviderRef = "Provider/transport-unix";
    transportSettings = {};
    transportCredentials = [];
    disabled = false;
  };
};
```


`metadata.name` (`guest-uplink`) and `metadata.zone` (`guest`) derive from the
attrset keys. Eval-time checks: `spec.childZoneName` matches
`^[a-z][a-z0-9-]*$` and equals the enclosing Zone key; the local root has no
uplink; at most one ZoneLink resource (enabled or disabled) exists per non-root
Zone; `spec.limits.*` remain within bounds. `parentZone` selects the allocator
owner and is compiled into sealed bootstrap topology; the child-local ZoneLink
supplies transport and local route/session state for that selected edge.

### 14.3 Provider installation

```nix
# Step 1: declare the derivation in the artifact catalog (global, not zone-local).
# Artifact IDs are zone-scoped at use, but the catalog is a top-level Nix attrset.
d2b.artifacts.runtime-cloud-hypervisor = {
  package = pkgs.d2b-provider-runtime-cloud-hypervisor;
  type    = "provider";
};

# Step 2: install the Provider in a Zone; spec.artifactId references the catalog
# by ID. All PackageIdentity sub-fields (digest, signatureId, trustEpoch,
# manifestDigest, conformanceAttestationDigest, configSchemaDigest) are resolved
# from the artifact catalog at build time - they do not appear in the spec.
d2b.zones.dev.resources.runtime-cloud-hypervisor = {
  type = "Provider";
  spec = {
    artifactId = "runtime-cloud-hypervisor"; # plain bounded ID; NOT a ResourceRef

    # spec.config: operator-authored; validated at build time against the config
    # schema identified by the artifact catalog's configSchemaDigest for this ID.
    config = {
      defaultCpuCount  = 2;
      defaultMemoryMib = 1024;
      # Secrets via Credential ref:
      # apiToken = d2b.zones.dev.credentialRef "api-token";
    };

    # spec.exports, spec.components, spec.dependencies, spec.permissionClaims,
    # spec.upgradePolicy, spec.restartPolicy: populated from the signed manifest
    # embedded in the artifact at build time. These are NOT writable Nix options;
    # the Nix module generator does not emit settable options for them.
    # Attempting to set them is an eval assertion error.
  };
};
```

Bootstrap Providers (`system-core`, `system-minijail`) are auto-generated by
the resource compiler from the Zone runtime's artifact catalog entries (type
`"provider"`, IDs `"system-core"` and `"system-minijail"`). Authoring
`d2b.zones.<zone>.resources.system-core` or `system-minijail` with
`type = "Provider"` is an eval assertion error:
`"system-core and system-minijail are bootstrap-only providers and cannot be
hand-authored"`.

### 14.4 Role authoring

```nix
d2b.zones.dev.resources.process-controller = {
  type = "Role";
  spec = {
    rules = [
      {
        resourceTypes = [ "EphemeralProcess" "Process" ];
        verbs         = [ "create" "delete" "get" "list" "update-finalizers"
                          "update-spec" "update-status" "watch" ];
        subresources  = [];
        resourceNames = [];    # empty = all names
        zones         = [];    # empty = this Zone only
        executionRefs = [];    # empty = unrestricted
        sessionVerbs  = [ "connect" "invoke" "open-stream" ];
      }
    ];
  };
};

d2b.zones.dev.resources.zone-reader = {
  type = "Role";
  spec = {
    rules = [
      {
        resourceTypes = [ "Guest" "Host" "Process" "Provider" "Zone" ];
        verbs         = [ "get" "list" "watch" ];
        subresources  = [];
        resourceNames = [];
        zones         = [];
        executionRefs = [];
        sessionVerbs  = [];
      }
    ];
  };
};
```

Eval-time validations: `spec.rules[*].verbs` against the closed verb enum;
`spec.rules[*].sessionVerbs` against the closed session verb enum;
`spec.rules[*].executionRefs` format if non-empty. `spec.rules[*].resourceTypes`
validation against the Zone API catalog is deferred to Phase 2 (catalog requires
loading Provider manifests; core types are known at eval time).
Generated option help lists all nine session verbs, describes `relay` as
ZoneLink-scoped forwarding only, and binds `audit-export` and `support-bundle`
to their exact admin-only service/method selectors without granting resource
authority. Nix recognizes all three as session-verb tokens, but Phase 2
admission still rejects an unbounded, wildcard, Provider-asserted, or ordinary
operator-authored relay grant unless explicit admin policy permits that exact
bounded grant.

### 14.5 RoleBinding authoring

```nix
d2b.zones.dev.resources.process-controller-binding = {
  type = "RoleBinding";
  spec = {
    roleRef                  = "Role/process-controller";
    subjects                 = [ "Provider/system-minijail" ];
    externalPrincipalSelector = null;
    scopeNarrowing            = null;
  };
};

d2b.zones.dev.resources.zone-reader-alice = {
  type = "RoleBinding";
  spec = {
    roleRef                  = "Role/zone-reader";
    subjects                 = [ "User/alice" ];
    externalPrincipalSelector = null;
    scopeNarrowing            = null;
  };
};

d2b.zones.dev.resources.zone-reader-bob = {
  type = "RoleBinding";
  spec = {
    roleRef                  = "Role/zone-reader";
    subjects                 = [ "User/bob" ];
    externalPrincipalSelector = null;
    scopeNarrowing            = null;
  };
};
```

Eval-time validations: `spec.roleRef` format; each `spec.subjects[*]` format
and ResourceType; no duplicate subjects.

### 14.6 Bootstrap provider records

`Provider/system-core` and `Provider/system-minijail` are generated
automatically by the Zone runtime on first initialization as `managedBy=controller`
resources. They are not included in the resource bundle and are never subject to
generation cleanup deletion.

Attempting to author `d2b.zones.<zone>.resources.system-core` or
`d2b.zones.<zone>.resources.system-minijail` with `type = "Provider"` emits the
eval assertion:
`"system-core and system-minijail are bootstrap-only providers and cannot be
hand-authored"`.

---

### 14.7 Credential references and secret handling

Secrets must never appear as inline string values in the resource bundle. The
Nix authoring surface uses Credential resources to hold secret references; any
Provider `config` field that requires a secret is expressed using the canonical
Credential ref object:

```json
{ "$credentialRef": "Credential/<name>" }
```

Nix authoring syntax:

```nix
# Declare a Credential resource using the unified syntax:
d2b.zones.dev.resources.api-token = {
  type = "Credential";
  spec = {
    source         = "systemd-credential"; # "systemd-credential"|"host-secret"|"derivation-secret"
    credentialName = "d2b-api-token";
  };
};

# Reference it in a Provider spec.config:
d2b.zones.dev.resources.some-provider = {
  type = "Provider";
  spec = {
    artifactId = "some-provider";
    config = {
      apiToken = d2b.zones.dev.credentialRef "api-token";
      # ↑ compiles to: { "$credentialRef": "Credential/api-token" }
    };
  };
};
```

Normative rules:

- `$credentialRef` is the only permitted `$`-prefixed key in config JSON; any
  other `$`-prefixed key is rejected at build time.
- The referenced Credential must be declared in `d2b.zones.<zone>.resources.*`
  with `type = "Credential"` within the same Zone; cross-Zone Credential refs
  are rejected at eval time.
- Credential resources are operator-authored and appear in the resource bundle;
  the Zone runtime creates them as `managedBy=configuration` resources subject to
  the generation cleanup contract (§14.11).
- The Zone runtime resolves Credential refs at activation time by reading the
  declared secret source. Resolved secret values are never written to the redb
  store, emitted in logs, audit records, or status fields.
- The resource compiler applies a heuristic inline-secret lint (PEM headers,
  base64 payload > 32 bytes, `sk-*`/`ecdsa-*` SSH key prefixes, UUIDs ≥ 128 bits
  as raw hex) and rejects the build when `--strict-secrets` is set or emits a
  warning otherwise. This lint is heuristic and does not replace the requirement
  to use Credential refs for all actual secrets.
- When a Credential is deleted by generation cleanup, the Zone runtime revokes
  the resolved secret binding before clearing the finalizer.

### 14.8 Canonical ResourceSpec JSON envelope format

All resources emitted by the Nix resource compiler follow this exact envelope
structure (normative).

**Nix-to-envelope mapping**: `type` in the Nix record maps to `resourceType`
in the envelope; `spec` maps to `spec`. `metadata.name` derives from the attrset
key; `metadata.zone` derives from the Zone attrset key; `apiVersion` defaults to
`"resources.d2bus.org/v3"`. `metadata.generation` is initialized to 1 for new resources and
incremented by the compiler whenever `spec` changes. All other metadata fields
and `status` are filled by the Zone runtime (null in the bundle).

```
{
  "apiVersion": "resources.d2bus.org/v3",            // fixed; not per-type versioned
  "resourceType": "<ResourceType>",       // PascalCase registered type name
  "metadata": {
    "name":                <string>,      // ResourceName; ^[a-z][a-z0-9-]*$; ≤128 chars
    "zone":                <string>,      // equals the Zone name this bundle is for
    "uid":                 null,          // null in bundle; set by runtime at first create
    "generation":          <u64>,         // 1 for new resources; compiler-incremented on spec change
    "revision":            null,          // null in bundle; set by runtime on each write
    "labels":              {},            // string→string; empty by default
    "annotations":         {},            // string→string; empty by default
    "ownerRef":            <string|null>, // "ResourceType/name" or null
    "finalizers":          [<string>],    // pre-set core finalizer strings (see per-type rules)
    "deletionRequestedAt": null,          // null in bundle; set by runtime on deletion
    "createdAt":           null,          // null in bundle; set by runtime on first create
    "updatedAt":           null           // null in bundle; set by runtime on each write
  },
  "spec":   { ... },  // type-specific; fully determined by Nix; see per-type examples
  "status": null      // always null in bundle; written by runtime only
}
```

Invariants:

- `metadata.zone` must equal the Zone name of the enclosing bundle.
- Runtime-assigned fields (`uid`, `revision`, `createdAt`, `updatedAt`) are `null`
  in the bundle; the runtime fills them on first activation and preserves stored
  values on subsequent generations.
- `metadata.generation` in the bundle is the spec-generation counter maintained
  by the resource compiler: starts at 1, incremented each time the operator
  changes the spec for that resource. The runtime preserves and independently
  increments this counter after activation.
- `metadata.finalizers` pre-populated in the bundle instruct the runtime to add
  those finalizers when the resource is first created. Values must be from the
  closed per-type core finalizer set.
- `spec` is serialized to canonical JSON (RFC 8785 key ordering: UTF-16 code-unit
  sort; no insignificant whitespace) by the resource compiler.
- Per-resource `digest` = `sha256(<canonical-JSON-of-spec>)` in lowercase hex
  prefixed `sha256:`.

#### Zone canonical envelope

```json
{
  "apiVersion": "resources.d2bus.org/v3",
  "resourceType": "Zone",
  "metadata": {
    "name": "dev", "zone": "dev", "uid": null, "generation": 1,
    "revision": null, "labels": {}, "annotations": {},
    "ownerRef": null, "finalizers": [],
    "deletionRequestedAt": null, "createdAt": null, "updatedAt": null
  },
  "spec": {},
  "status": null
}
```

Zone is the self-resource; it is created by the Zone runtime on initialization
(`managedBy=controller`) and is not included in the resource bundle.

#### ZoneLink canonical envelope

```json
{
  "apiVersion": "resources.d2bus.org/v3",
  "resourceType": "ZoneLink",
  "metadata": {
    "name": "guest-uplink", "zone": "guest", "uid": null, "generation": 1,
    "revision": null, "labels": {}, "annotations": {},
    "ownerRef": null, "finalizers": [],
    "deletionRequestedAt": null, "createdAt": null, "updatedAt": null
  },
  "spec": {
    "childZoneName": "guest",
    "transportProviderRef": "Provider/transport-unix",
    "transportSettings": {},
    "transportCredentials": [],
    "disabled": false,
    "limits": {
      "maxPendingIntents": 256,
      "maxActiveStreams": 32,
      "reconnectMaxAttempts": 10,
      "reconnectWindowSecs": 300
    }
  },
  "status": null
}
```

`childZoneUid` is `null` in the bundle; the child-local ZoneLink controller
writes the local Zone UID after the parent allocator acknowledges it and uses
it to detect replacement across reconnects.

#### Provider canonical envelope

```json
{
  "apiVersion": "resources.d2bus.org/v3",
  "resourceType": "Provider",
  "metadata": {
    "name": "runtime-cloud-hypervisor", "zone": "dev", "uid": null, "generation": 1,
    "revision": null, "labels": {}, "annotations": {},
    "ownerRef": null, "finalizers": ["core.provider-drain"],
    "deletionRequestedAt": null, "createdAt": null, "updatedAt": null
  },
  "spec": {
    "artifactId": "runtime-cloud-hypervisor",
    "config": { "defaultCpuCount": 2, "defaultMemoryMib": 1024 },
    "exports": [
      { "resourceType": "Guest", "schemaDigest": "sha256:445566..." }
    ],
    "components": [
      {
        "name": "primary", "role": "controller", "placement": "HostLocal",
        "processTemplate": {
          "executableRef": "d2b-provider-runtime-cloud-hypervisor",
          "args": ["--zone-bus-fd", "3"],
          "environmentPolicy": "isolated"
        },
        "required": true
      }
    ],
    "dependencies": [],
    "permissionClaims": [
      { "resourceType": "Guest",
        "verbs": ["create","update-spec","update-status","delete"],
        "level": "standard" }
    ],
    "upgradePolicy": "drain-then-replace",
    "restartPolicy": "on-failure"
  },
  "status": null
}
```

`exports`, `components`, `dependencies`, `permissionClaims`, `upgradePolicy`, and
`restartPolicy` are loaded from the signed manifest embedded in the artifact
identified by `artifactId`. The operator supplies only `artifactId` (a plain
bounded ID referencing the `d2b.artifacts.*` catalog) and `config`. The resource
compiler resolves all manifest-derived fields and PackageIdentity sub-fields from
the artifact catalog at build time. Store paths and raw closure metadata are
**private catalog implementation data** and do not appear in the spec, status,
audit records, or OTEL attributes.

#### Role canonical envelope

```json
{
  "apiVersion": "resources.d2bus.org/v3",
  "resourceType": "Role",
  "metadata": {
    "name": "process-controller", "zone": "dev", "uid": null, "generation": 1,
    "revision": null, "labels": {}, "annotations": {},
    "ownerRef": "Provider/system-minijail",
    "finalizers": ["core.role-binding-drain"],
    "deletionRequestedAt": null, "createdAt": null, "updatedAt": null
  },
  "spec": {
    "rules": [
      {
        "resourceTypes": ["EphemeralProcess", "Process"],
        "verbs": ["create","delete","get","list","update-finalizers",
                  "update-spec","update-status","watch"],
        "subresources": [],
        "resourceNames": [],
        "zones": [],
        "executionRefs": [],
        "sessionVerbs": ["connect","invoke","open-stream"]
      }
    ]
  },
  "status": null
}
```

Note: `verbs` and `resourceTypes` arrays in the canonical envelope are sorted
ascending by the resource compiler (RFC 8785 array-of-string sort).

#### RoleBinding canonical envelope

```json
{
  "apiVersion": "resources.d2bus.org/v3",
  "resourceType": "RoleBinding",
  "metadata": {
    "name": "process-controller-binding", "zone": "dev", "uid": null, "generation": 1,
    "revision": null, "labels": {}, "annotations": {},
    "ownerRef": null, "finalizers": [],
    "deletionRequestedAt": null, "createdAt": null, "updatedAt": null
  },
  "spec": {
    "externalPrincipalSelector": null,
    "roleRef": "Role/process-controller",
    "scopeNarrowing": null,
    "subjects": ["Provider/system-minijail"]
  },
  "status": null
}
```


### 14.9 Zone resource bundle: generation, canonical sort, and integrity pinning

The Nix resource compiler emits one `zone-resources.json` per configured Zone
per build. This is the Zone resource bundle.

```json
{
  "bundleVersion": "1",
  "zoneUid":       null,
  "zoneName":      "dev",
  "generation":    7,
  "generatedAt":   "2026-07-22T21:25:43.000Z",
  "nixRevision":   "abc123def456",
  "resources": [
    {
      "resourceType": "Credential", "name": "api-token",
      "digest": "sha256:...",
      "envelope": { "..." }
    },
    {
      "resourceType": "Provider",  "name": "runtime-cloud-hypervisor",
      "digest": "sha256:...",
      "envelope": { "..." }
    },
    {
      "resourceType": "Role",      "name": "process-controller",
      "digest": "sha256:...",
      "envelope": { "..." }
    },
    {
      "resourceType": "RoleBinding", "name": "process-controller-binding",
      "digest": "sha256:...",
      "envelope": { "..." }
    }
  ],
  "bundleDigest": "sha256:..."
}
```

| Bundle field | Type | Rules |
| --- | --- | --- |
| `bundleVersion` | string | Fixed `"1"`; the resource compiler's bundle schema version |
| `zoneUid` | string\|null | `null` at build time; the runtime validates against `store_meta.zone_uid` at activation; first activation stores the newly generated UID back into the bundle record in the store |
| `zoneName` | string | Must match the Zone self-resource `metadata.name` |
| `generation` | u64 | Monotonically increasing; compiler-incremented on every config change; starts at 1; rejected at runtime if not strictly greater than `store_meta.active_configuration_revision` |
| `generatedAt` | RFC 3339 UTC | Build-time timestamp from `builtins.currentTime` (impure); not used for security decisions |
| `nixRevision` | string | Git rev of the NixOS config, if available; opaque; traceability only |
| `resources` | array | Sorted ascending by `(resourceType, name)` lexicographically; contains only operator-authored resources (`managedBy=configuration`); bootstrap resources (`Provider/system-core`, `Provider/system-minijail`, `Zone/<name>`) are runtime-created and never present in the bundle |
| `digest` | sha256 hex | `sha256(<canonical-JSON-of-envelope.spec>)`; lowercase hex prefixed `sha256:` |
| `bundleDigest` | sha256 hex | `sha256(<canonical-JSON-of-resources-array>)` over the sorted array; computed by the compiler; verified by the runtime before applying any resource |

**Canonical sort order**: `(resourceType, name)` lexicographic ascending. Examples:
`Credential/api-token` < `Provider/runtime-cloud-hypervisor`
< `Role/process-controller` < `RoleBinding/process-controller-binding`.

**Integrity pinning**: The runtime verifies `bundleDigest` against the `resources`
array before applying any resource from the bundle. A digest mismatch rejects the
entire bundle; the prior generation remains active.

**Bundle file location**: Installed at
`/var/lib/d2b/zones/<zone>/bundles/generation-<N>.json` (path managed by the
Zone runtime's state directory). The `store_meta.active_configuration_revision`
pointer names the active generation. Retained prior generation bundles use the
same path scheme.

### 14.10 Nix eval and build-time validation pipeline

Three ordered phases validate the Nix configuration before a bundle is activated.

#### Phase 1 - NixOS eval (pure; runs on `nixos-rebuild` evaluation and `nix flake check`)

| Check | Mechanism | Failure mode |
| --- | --- | --- |
| Resource attrset key (→ `metadata.name`) matches `^[a-z][a-z0-9-]*$` | Nix `assert` | eval error |
| Name length ≤ 128 chars | Nix `assert` | eval error |
| `type` is a recognized core ResourceType or a non-empty string (non-core types validated at Phase 2) | Nix `assert` | eval error if empty or non-string |
| `spec.rules[*].verbs` tokens in closed verb enum (§5.3.2) - Role only | Nix `assert` | eval error |
| `spec.rules[*].sessionVerbs` tokens in closed session verb enum - Role only | Nix `assert` | eval error |
| `spec.roleRef` format `^Role/[a-z][a-z0-9-]*$` - RoleBinding only | Nix `assert` | eval error |
| `spec.subjects[*]` format `^[A-Za-z][A-Za-z0-9]*/[a-z][a-z0-9-]*$` - RoleBinding only | Nix `assert` | eval error |
| `spec.subjects[*]` ResourceType in permitted subject set (§6.3.2) - RoleBinding only | Nix `assert` | eval error |
| No duplicate entries in `spec.subjects` - RoleBinding only | Nix `assert` | eval error |
| No cross-Zone refs anywhere in `spec` (no `<Zone>/ResourceType/name` form) | Nix `assert` | eval error |
| `type = "Provider"` and name `system-core` or `system-minijail` rejected | Nix `assert` | eval error with message |
| Any authored `type = "Zone"` entry under `resources` | Nix `assert`; Zone self-resource is runtime-created | eval error |
| `parentZone` omitted on a non-root Zone or set on `local-root` | Nix `assert` over Zone options | eval error |
| `parentZone` does not resolve to a declared Zone or equals the child | Nix `assert` over Zone options | eval error |
| `parentZone` graph cycles or exceeds 16 Zone names on an ancestry path | Nix graph walk using retained `MAX_REALM_LABELS` bound | eval error |
| conflicting `parentZone` scalar definitions | Standard Nix module merge | eval error |
| ZoneLink: `spec.childZoneName ==` Zone attrset key | Nix `assert` | eval error |
| Non-root Zone has at most one ZoneLink resource; local root has none | Nix `assert` | eval error |
| `spec.transportProviderRef` matches `^Provider/transport-[a-z][a-z0-9-]*$` - ZoneLink only | Nix `assert` | eval error |
| `$credentialRef` target in any `spec.*` string must name a `type = "Credential"` resource in the same Zone | Nix `assert` | eval error |
| No `$`-prefixed keys in `spec` other than `$credentialRef` | Nix `assert` | eval error |
| `spec.limits.maxPendingIntents ≤ 1024` - ZoneLink only | Nix `assert` | eval error |
| `spec.limits.maxActiveStreams ≤ 128` - ZoneLink only | Nix `assert` | eval error |
| `spec.artifactId` format `^[a-z][a-z0-9-]*$`, max 128 chars - Provider only | Nix `assert` | eval error |
| `type = "Provider"` without a `spec.artifactId` field is rejected | Nix `assert` | eval error |
| `spec.artifactId` value must name an entry in `d2b.artifacts.*` (attrset key lookup at eval time) with `type = "provider"` - Provider only | Nix `assert` | eval error if ID absent from catalog |
| Manifest-derived Provider spec fields (`spec.exports`, `spec.components`, `spec.dependencies`, `spec.permissionClaims`, `spec.upgradePolicy`, `spec.restartPolicy`) not set by operator | Nix `assert` | eval error with message |

#### Phase 2 - Nix build (impure; runs on `nixos-rebuild build` and `nix build`)

| Check | Mechanism | Failure mode |
| --- | --- | --- |
| Each resource's `spec` validated against the ResourceTypeSchema for its `type`; canonical JSON compared against schema (`genSchemaOptions` emits Phase 1 Nix options; Phase 2 performs the definitive comparison against the committed schema JSON) | Resource compiler | build failure naming type and failing field |
| `spec.artifactId` names an existing `d2b.artifacts.<id>` catalog entry with `type = "provider"` - Provider only | Resource compiler | build failure: "artifact ID not found or wrong type" |
| Artifact catalog entry has required derivation outputs (manifest, config schema, executable) - Provider only | Resource compiler | build failure |
| Artifact catalog `configSchemaDigest` matches SHA-256 of schema file in derivation output - Provider only | Resource compiler | build failure |
| Operator `spec.config` passes JSON Schema validation against catalog-resolved config schema - Provider only | Resource compiler | build failure naming failing field and schema path |
| Artifact catalog `manifestDigest` matches SHA-256 of manifest file in derivation output - Provider only | Resource compiler | build failure |
| Artifact manifest signature chain valid against installed trust store - Provider only | Resource compiler | build failure |
| Artifact `conformanceAttestationDigest` present in known attestation store - Provider only | Resource compiler | build failure |
| No duplicate `d2b.artifacts.<id>` entries (IDs are unique across the catalog) | Resource compiler | build failure naming duplicate ID |
| All declared dependency aliases resolve within the same Zone's Providers - Provider only | Resource compiler | build failure naming unresolved alias |
| ResourceType short-name collision among installed Providers in the same Zone | Resource compiler | build failure naming conflicting Providers |
| `spec.rules[*].resourceTypes` resolved against installed Provider catalogs in the Zone bundle - Role only | Resource compiler | build failure |
| `spec.roleRef` names an existing `type = "Role"` resource in the same Zone bundle - RoleBinding only | Resource compiler | build failure |
| `spec.subjects[*]` names resolve to declared resources or known external principal types in the same Zone bundle - RoleBinding only | Resource compiler | build failure |
| Inline-secret heuristic lint on config string values | Resource compiler | build warning; build failure with `--strict-secrets` |
| `resources` array sorted by `(resourceType, name)` | Resource compiler | auto-sorted; not an operator error |
| Per-resource `digest` = `sha256(canonical-JSON-of-spec)` | Resource compiler | computed and written; mismatch is a compiler bug |
| `bundleDigest` over sorted `resources` array | Resource compiler | computed and written |

#### Phase 3 - Runtime activation (on daemon restart or `d2b zone config apply`)

| Check | Mechanism | Failure mode |
| --- | --- | --- |
| `bundleDigest` integrity re-verified against `resources` array | Zone runtime | bundle rejected; prior generation stays active |
| `zoneUid` consistency: null = first activation; non-null must match `store_meta.zone_uid` | Zone runtime | bundle rejected |
| `zoneName` must match Zone self-resource `metadata.name` | Zone runtime | bundle rejected |
| `generation` strictly greater than `store_meta.active_configuration_revision` | Zone runtime | bundle rejected (prevents replay/downgrade) |
| All resource `digest` values re-verified against `envelope.spec` | Zone runtime | bundle rejected |
| Provider `config` re-validated against the config schema identified by the artifact catalog entry for `spec.artifactId` | Zone runtime | affected Provider set to `Failed` with `ConfigValid=False` condition; generation proceeds for other resources |
| `zoneUid=null` only on store's first activation (no prior Zone UID stored) | Zone runtime | bundle rejected if non-null UID expected |

If Phase 3 rejects the bundle, no resources are applied atomically and the prior
generation remains active. A per-Provider `config` failure does not block other
resources from activating; the affected Provider is set to `Failed` with `ConfigValid=False` condition.

### 14.11 Configuration generation lifecycle and cleanup contract

This section is the normative specification for resource lifecycle across Nix
configuration generation changes.

#### Configuration ownership

The Zone runtime stores two internal resource metadata fields on every resource
in the redb store (not exposed in the public resource API envelope):
- `managedBy`: one of `configuration | controller | api` - set by the runtime at create time
- `configurationGeneration`: the bundle generation when this resource was last confirmed present in the bundle

Every resource present in the bundle is created (or updated) by the runtime as
`managedBy=configuration`. The Zone runtime also creates bootstrap resources
(`Provider/system-core`, `Provider/system-minijail`, and the Zone self-resource)
directly on initialization as `managedBy=controller`; these are not included in
the bundle. The cleanup authority is the stored `managedBy` and
`configurationGeneration` values on the resource, **never** inferred from bundle
absence, `ownerRef`, labels, or any other field.

| Class | `managedBy` | In bundle | Cleanup eligible |
| --- | --- | --- | --- |
| Config-owned | `configuration` | Yes | Yes - absent from new bundle (`configurationGeneration < new generation`) → async Delete |
| Bootstrap-created | `controller` | No | No - `managedBy=controller` resources untouched by generation cleanup |
| Controller-created | `controller` | No | No - `managedBy=controller` resources untouched by generation cleanup |
| API-created | `api` | No | No - `managedBy=api` resources untouched by generation cleanup |

The resource compiler must reject any operator attempt to name a controller-created
resource in config (e.g., placing `Process/provider-primary-0` under `d2b.zones.*`).
This is enforced by a build-time exclusion list of reserved name prefixes per
controller-managed ResourceType.

#### Generation activation sequence (non-blocking)

1. Resource compiler emits bundle with `generation=N+1`.
2. Zone runtime reads and integrity-verifies the bundle (Phase 3 checks, §14.10).
3. `store_meta.active_configuration_revision` advances to `N+1` atomically.
4. The Zone begins serving all requests under generation `N+1` immediately.
   No cleanup gate; no operator wait.
5. `Zone.status.activeConfigurationGeneration = N+1`.
6. `Zone.status.conditions[ConfigurationCurrent].status = "True"`.
7. Absent config-owned resources are identified and queued for async Delete
   (see below). Activation does not wait for them.

#### Absent resource deletion (normative)

After generation activation, for each resource in the store with
`managedBy=configuration` whose `configurationGeneration` does not match the
new bundle generation (i.e., absent from the new bundle):

1. `metadata.deletionRequestedAt` is set to the activation timestamp (if null).
2. The resource-type-specific core finalizer is added if absent:
   - `Provider`: `core.provider-drain`
   - `Role`: `core.role-binding-drain`
   - `ZoneLink`: `core.zone-link-drain`
   - `Credential`: `core.credential-revoke`
   - `RoleBinding`: no pre-set finalizer (immediate atomic deletion)
3. An async reconcile trigger is emitted to the owning controller handler.
4. `Zone.status.cleanupPendingCount` is incremented per candidate.
5. `Zone.status.generationCleanupPending = true`.
6. `Zone.status.conditions[GenerationCleanupPending]` is set with reason
   `PendingCleanup` and message `"<N> config-owned resources from generation <M>
   completing deletion"`.
7. Zone.status.phase transitions to `Degraded` (remains until cleanup completes).

The Zone runtime never force-removes finalizers. If an owning controller fails
to clear its finalizer, `GenerationCleanupFailed` is eventually set and Zone
remains `Degraded`. Resources with `managedBy=controller` or `managedBy=api`
are never touched by the generation cleanup path.

#### Controller-created child preservation (normative)

Controllers reconcile their dynamic children in response to parent resource
deletion requests (`deletionRequestedAt` set) and phase changes, not in response
to generation changes directly:

- A `Provider` with `deletionRequestedAt` set triggers the provider lifecycle
  handler to stop owned component Processes. State Volumes are preserved unless
  `upgradePolicy=immediate`.
- A `Role` with `deletionRequestedAt` set is blocked by `core.role-binding-drain`
  until all dependent RoleBindings are deleted or re-bound to another Role.
  The authorization handler triggers dependent RoleBinding deletion automatically.
- `RoleBinding` deletion is the atomic final transaction (§6.6): RBAC index
  removal + row removal + `Deleted` event in one redb write.
- `ZoneLink` deletion closes the session gracefully, drains pending intents, then
  clears `core.zone-link-drain`.
- Controller-created resources with `managedBy=controller` (e.g., `Process/provider-primary-0`,
  `Volume/provider-state`) are **never** automatically deleted on generation change,
  even if absent from the new Nix config. They are managed exclusively by their
  owning controller in response to parent resource phase transitions.

#### Cleanup status, conditions, and audit (normative)

**While cleanup is pending**:

```
Zone.status.generationCleanupPending = true
Zone.status.cleanupPendingCount      = <N>
Zone.status.conditions:
  GenerationCleanupPending:
    status:             "True"
    reason:             "PendingCleanup"
    message:            "<N> config-owned resources from generation <M> completing deletion"
    lastTransitionTime: <RFC 3339 UTC>
Zone.status.phase: "Degraded"  (transitions to Degraded while any cleanup is pending)
```

**After all cleanup candidates complete**:

```
Zone.status.generationCleanupPending = false
Zone.status.cleanupPendingCount      = 0
Zone.status.conditions[GenerationCleanupPending].status: "False"
Zone.status.conditions[GenerationCleanupFailed]:         absent or status="False"
Zone.status.phase: reverts to aggregate mandatory handler phase (typically "Ready")
```

**On stuck cleanup** (resource awaiting deletion with no controller progress for
`cleanupStuckThreshold`, default 5 minutes):

```
Zone.status.phase: "Degraded"  (already set from pending cleanup; remains Degraded)
Zone.status.conditions:
  GenerationCleanupFailed:
    status:             "True"
    reason:             "CleanupStuck"
    message:            "<ResourceType> resource has been awaiting deletion beyond threshold"
    lastTransitionTime: <RFC 3339 UTC>
```

The message names the ResourceType only; it does not include the resource name,
spec content, or any configuration value.

**Audit records** (all carry `zone_name` and `generation`; no resource names,
spec content, or secret values):

| Audit kind | Emitted when |
| --- | --- |
| `zone.config.generation.activate` | New bundle generation becomes active |
| `zone.config.cleanup.complete` | All cleanup candidates for a generation complete deletion |
| `zone.config.cleanup.stuck` | A cleanup candidate exceeds `cleanupStuckThreshold` |
| `zone.config.generation.rollback` | Prior generation is restored by operator |

**Error handling**: The configuration publication handler retries stuck finalizer
notifications with exponential backoff, bounded by `cleanupStuckThreshold`. After
threshold, it records the `GenerationCleanupFailed` condition without further
retries. The runtime never force-removes finalizers. An operator may resolve a
stuck cleanup by: (a) fixing or restarting the owning controller so it can
complete its finalizer, or (b) performing a full Zone reset.

#### Prior generation retention and rollback

Prior generation bundle files are retained in the Zone store bundle directory
up to the configured `retainedPriorGenerationCount` (default 3, range 1..16).
A bundle file for generation M is eligible for pruning when:

1. All resources from generation M with `managedBy=configuration` that were
   absent from generation M+1 have completed deletion; AND
2. No rollback lock is outstanding (`d2b zone config rollback-lock set`); AND
3. Retaining this file would exceed `retainedPriorGenerationCount`.

When the count is exceeded, the oldest eligible prior bundle file is pruned;
resources with `deletionRequestedAt` already set from that generation continue
their individual finalizer drain but lose rollback association.

An operator with `verb=zone.config-rollback` may restore the retained prior
generation:

```
d2b zone config rollback --zone dev --to-generation <N>
```

Rollback atomically:
1. Re-activates the prior bundle as the new active generation.
2. Re-creates any config-owned resources deleted by the superseded generation's
   cleanup, using the prior bundle's resource envelopes.
3. Issues async Deletes for resources that appeared in the superseded generation
   but not the prior generation.
4. Emits `zone.config.generation.rollback` audit record.
5. Triggers a new cleanup cycle for the superseded-generation additions.

---

## 15. Conformance and tests

### 15.1 Zone self-resource tests

| Test | Assertion |
| --- | --- |
| `zone-self-resource-enforced` | Store with no Zone resource fails to open |
| `zone-uid-mismatch-quarantine` | Opening a store where Zone UID differs from store_meta.zone_uid quarantines the store |
| `zone-name-mismatch-rejected` | Zone resource with `metadata.name != store_meta.zone_name` is rejected |
| `zone-cardinality-one` | Attempt to create a second Zone resource is rejected with `resource-already-exists` |
| `zone-cross-zone-ref-rejected` | Any resource with a ref containing a Zone prefix is rejected |
| `zone-owner-rejected` | Create request with non-null Zone `ownerRef` is rejected |
| `zone-deletion-only-on-drain` | Zone deletion without `core.zone-drain` finalizer path is rejected |

### 15.2 ZoneLink tests

| Test | Assertion |
| --- | --- |
| `zonelink-reconnect-child-uid-change` | On reconnect after local child Zone UID replacement, local cursor resets to 0, the parent allocation is recreated, and `childZoneUid` updates |
| `zonelink-disconnect-unknown-phase` | On transport disconnect ZoneLink phase becomes Unknown within one reconcile |
| `zonelink-intent-queue-limit` | Queuing more than 256 local intents returns `backpressure` |
| `zonelink-disabled-no-reconnect` | `spec.disabled=true` prevents reconnection and sets `DisabledByOperator` condition |
| `zonelink-child-auth-denied-failed` | Parent allocator rejecting the child subject/allocation sets `Failed` phase and `ChildAuthorizationDenied` condition |
| `zonelink-drain-closes-session` | Deletion drains local intents and closes session before removing resource |
| `zonelink-transport-provider-ref-required` | ZoneLink without `spec.transportProviderRef` fails admission |
| `zonelink-transport-ref-pattern-enforced` | `spec.transportProviderRef = "Provider/network-local"` fails admission because it does not match the required `transport-*` pattern |
| `zonelink-transport-credentials-max` | ZoneLink with 9 `transportCredentials` entries fails admission |
| `zonelink-child-name-matches-store` | ZoneLink whose `childZoneName` differs from its enclosing child Zone is rejected |
| `zonelink-one-child-local-uplink` | A second ZoneLink (even disabled) in one child Zone and any uplink in local root are rejected |
| `zonelink-parent-bootstrap-binding` | Child ZoneLink allocation uses exactly the parent chosen by compiler-only `parentZone`; no transport setting can override it |
| `zonelink-parent-has-no-reciprocal-row` | Reconcile creates parent allocator/route state but no parent-store ZoneLink resource |

### 15.3 Provider tests

| Test | Assertion |
| --- | --- |
| `provider-trust-check-required` | Provider with invalid signature is rejected before any component launch |
| `provider-conformance-check-required` | Provider without attestation digest is rejected |
| `provider-config-schema-validation` | Config object not matching configSchemaDigest schema is rejected |
| `provider-api-name-collision-rejected` | Two Providers exporting the same short ResourceType name: second rejected |
| `provider-dependency-cycle-rejected` | Provider with cyclic alias dependency is rejected |
| `provider-quarantine-on-trust-failure` | Trust check failure after install quarantines Provider and stops components |
| `provider-bootstrap-no-process` | system-core and system-minijail report no processRef; all others require processRef |
| `provider-wildcard-permission-restricted` | Non-bootstrap Provider with wildcard permission claim is rejected |
| `provider-upgrade-drain-then-replace` | `upgradePolicy=drain-then-replace` completes old component drain before new launch |
| `provider-component-bound-limits` | Provider with 9 controllers fails admission; 8 is the maximum |
| `provider-crate-layout-src-required` | A `d2b-provider-*` workspace crate without `src/` fails `make test-policy` with message naming the crate and missing path |
| `provider-crate-layout-tests-required` | A `d2b-provider-*` workspace crate without `tests/` fails `make test-policy` |
| `provider-crate-layout-integration-required` | A `d2b-provider-*` workspace crate without `integration/` fails `make test-policy` |
| `provider-crate-layout-readme-required` | A `d2b-provider-*` workspace crate without `README.md` fails `make test-policy` |
| `provider-readme-sections-all-present` | A Provider `README.md` missing any of the nine required headings (§4.8.3) fails policy with the exact missing heading name |
| `provider-readme-sections-partial-missing` | A Provider `README.md` with 8 of 9 sections fails policy; message names the one missing section |
| `provider-integration-target-declared` | An `integration/*.rs` file without an `integration-target:` declaration in the first 20 lines fails policy |
| `provider-integration-target-unique` | An `integration/*.rs` file with two `integration-target:` declarations fails policy |
| `provider-integration-target-valid-values` | An `integration-target:` value other than `container` or `host-integration` fails policy |
| `provider-crate-naming-convention` | A crate named `d2b-<implementation>-<base>` (implementation before base) fails the workspace member name policy |
| `provider-crate-layout-non-provider-exempt` | A non-`d2b-provider-*` workspace crate is exempt from the §4.8 layout check |

### 15.4 Role tests

| Test | Assertion |
| --- | --- |
| `role-unknown-verb-rejected` | Rule with an unknown verb token fails admission |
| `role-relay-core-zonelink-admitted` | Core-generated, ZoneLink-owned Role/RoleBinding with an exact adjacent-Zone enrollment selector and exact target bounds admits `relay` |
| `role-relay-missing-denied` | A forwarding hop without `relay` fails closed even when the target verb is allowed |
| `role-relay-target-verb-required` | `relay` alone cannot authorize the forwarded invocation/stream target verb |
| `role-relay-provider-self-assertion-rejected` | A Provider- or payload-asserted relay grant is rejected |
| `role-relay-wildcard-rejected` | Relay with empty/all-name or wildcard target scope is rejected, including for a core-generated Role |
| `role-unknown-resource-type-rejected` | Rule naming a ResourceType not in Zone API catalog is rejected |
| `role-wildcard-non-core-rejected` | Non-core-controller Role with `resourceNames: ["*"]` is rejected |
| `role-wildcard-core-permitted` | Core-generated Role with `resourceNames: ["*"]` is admitted |
| `role-index-built-before-ready` | Role does not reach Ready until `IndexBuilt=True` |
| `role-deletion-blocked-by-bindings` | Role with active bindings cannot be deleted until all bindings removed |
| `role-bounds-enforced` | Role with 33 rules fails admission; 32 is the maximum |

### 15.5 RoleBinding tests

| Test | Assertion |
| --- | --- |
| `rolebinding-role-not-found-failed` | RoleBinding with non-existent `roleRef` reaches `Failed` phase |
| `rolebinding-invalid-subject-type-rejected` | Subject with non-permitted ResourceType rejected at admission |
| `rolebinding-scope-exceeds-role-rejected` | `scopeNarrowing` with verb absent from Role rejected at admission |
| `rolebinding-subject-uid-change-detected` | Subject deleted/recreated triggers `SubjectIdentityChanged` condition |
| `rolebinding-deletion-immediate` | RoleBinding deletion is one atomic transaction: RBAC index removal, `Deleted` revision event, and row removal occur simultaneously; no intermediate state is observable |
| `rolebinding-subject-bounds-enforced` | RoleBinding with 129 subjects fails admission |

### 15.6 Bootstrap tests

| Test | Assertion |
| --- | --- |
| `bootstrap-only-system-core-minijail` | Any subject other than system-core/system-minijail is denied under bootstrap |
| `bootstrap-no-runtime-authority` | Bootstrap cannot grant exec/shell/process-outside-spec operations |
| `bootstrap-non-configurable` | No config field widens bootstrap authorization |
| `bootstrap-supersession-atomic` | After stored RBAC publishes, bootstrap is fully superseded with no overlap window |
| `bootstrap-recovery-out-of-band` | Corrupt authorization store triggers fail-closed; reset requires privileged local operator |

### 15.7 Cross-cutting tests

| Test | Assertion |
| --- | --- |
| `zone-control-audit-no-payload` | Audit records for Zone control type mutations contain no rule/subject/config/digest content |
| `zone-control-otel-no-sensitive-labels` | OTEL metrics carry no subject names, resource names, or credential-adjacent labels |
| `zone-control-error-bounded` | All Zone control error messages ≤ 512 bytes, UTF-8, no secrets |
| `zone-control-status-owner-only` | Only core-controller handler may update-status for any Zone control resource |
| `zone-control-cross-zone-ref-rejected` | Any ref with cross-Zone notation is rejected at admission |

### 15.8 Configuration generation and cleanup tests

#### Phase 1 - Nix eval tests

| Test | Assertion |
| --- | --- |
| `nix-eval-name-regex-enforced` | Zone/Provider/Role/RoleBinding with name `"Bad_Name"` fails eval with assertion error |
| `nix-eval-name-length-enforced` | Resource name of 129 characters fails eval |
| `nix-eval-verb-closed-enum` | Rule with verb `"delete-all"` (unknown) fails eval |
| `nix-eval-session-verb-closed-enum` | Rule with `sessionVerbs=["sudo"]` fails eval |
| `nix-eval-relay-session-verb-known` | `sessionVerbs=["relay"]` reaches Phase 2 as the canonical token; placing `relay` in `verbs` fails eval |
| `nix-build-relay-scope-restricted` | Unbounded, wildcard, or Provider/self-asserted relay configuration fails before bundle activation |
| `nix-eval-roleref-format` | `roleRef="role/foo"` (wrong case) fails eval |
| `nix-eval-subject-type-restricted` | Subject `"Device/foo"` (non-permitted type) fails eval |
| `nix-eval-no-duplicate-subjects` | Two identical subjects in one RoleBinding fails eval |
| `nix-eval-no-cross-zone-ref` | Subject `"dev/Role/foo"` (Zone-prefixed) fails eval |
| `nix-eval-bootstrap-provider-rejected` | `d2b.zones.dev.resources.system-core = { type = "Provider"; ... }` fails eval with named assertion |
| `nix-eval-provider-missing-artifact-id` | `d2b.zones.dev.resources.p = { type = "Provider"; spec = { config = {}; }; }` (no `artifactId`) fails eval |
| `nix-eval-artifact-id-not-in-catalog` | `spec.artifactId = "nonexistent"` for a Provider where `d2b.artifacts` has no `nonexistent` entry fails eval |
| `nix-eval-artifact-wrong-type` | `d2b.artifacts.foo = { package = pkgs.hello; type = "nixos-system"; }` used as `spec.artifactId` in a Provider fails eval (type mismatch) |
| `nix-eval-artifact-id-format` | `spec.artifactId = "Bad_ID"` fails eval with label regex assertion |
| `nix-eval-credentialref-declared` | `d2b.zones.dev.credentialRef "missing"` for undeclared Credential fails eval |
| `nix-eval-dollar-key-rejected` | Config `{ "$secret" = "x"; }` fails eval |
| `nix-eval-parent-zone-required-root-forbidden` | Missing `parentZone` on a non-root Zone and any definition on `local-root` fail eval |
| `nix-eval-parent-zone-resolves` | Unknown and self-valued `parentZone` settings fail eval |
| `nix-eval-parent-zone-one-parent` | Conflicting scalar `parentZone` module definitions fail through Nix merging |
| `nix-eval-parent-zone-cycle-rejected` | A two-Zone or longer `parentZone` cycle fails eval |
| `nix-eval-parent-zone-depth-bound` | A topology path containing 17 Zone names fails eval; 16 succeeds |
| `nix-eval-zonelink-child-name-mismatch-rejected` | Child-local ZoneLink with `childZoneName` unequal to its enclosing Zone key fails eval |
| `nix-eval-zonelink-second-uplink-rejected` | A second child-local uplink (even disabled), or any local-root uplink, fails eval |
| `nix-eval-zonelink-limits-maxpendingintents-bound` | `maxPendingIntents = 1025` fails eval |

#### Phase 2 - Build tests

| Test | Assertion |
| --- | --- |
| `nix-build-artifact-id-missing-from-catalog` | Provider `spec.artifactId = "unknown"` fails build with "artifact ID not found" |
| `nix-build-artifact-wrong-type-rejected` | Provider `spec.artifactId` pointing to a `nixos-system` catalog entry fails build |
| `nix-build-duplicate-artifact-id` | Two `d2b.artifacts` entries with the same ID fail build with duplicate error |
| `nix-build-artifact-store-path-absent-from-bundle` | Emitted bundle JSON contains no Nix store path strings |
| `nix-build-artifact-store-path-absent-from-config` | Emitted config JSON in bundle contains no Nix store path strings |
| `nix-build-config-schema-failure` | Provider config field of wrong type fails build with field path in error |
| `nix-build-schema-digest-mismatch` | Schema file with SHA-256 not matching `configSchemaDigest` in artifact catalog fails build |
| `nix-build-manifest-digest-mismatch` | Manifest file with SHA-256 not matching `manifestDigest` in artifact catalog fails build |
| `nix-build-resourcetype-collision` | Two Providers in the same Zone exporting the same short ResourceType name fail build |
| `nix-build-bundle-sorted` | Emitted `resources` array is sorted by `(resourceType, name)` ascending |
| `nix-build-bundle-digest-stable` | Same Nix config on two builds produces identical `bundleDigest` |
| `nix-build-per-resource-digest-correct` | Per-resource `digest` matches `sha256(canonical-JSON-of-spec)` |
| `nix-build-credential-ref-survives-build` | `{ "$credentialRef": "Credential/api-token" }` appears verbatim in emitted bundle |
| `nix-build-inline-secret-lint-warning` | Config string matching PEM header emits build warning |
| `nix-build-inline-secret-strict-failure` | Same with `--strict-secrets` fails build |

#### Phase 3 - Runtime activation tests

| Test | Assertion |
| --- | --- |
| `nix-runtime-bundledigest-integrity` | Bundle with tampered `bundleDigest` is rejected; prior generation stays active |
| `nix-runtime-generation-monotone` | Bundle with `generation ≤ active_configuration_revision` is rejected |
| `nix-runtime-zoneuid-mismatch-rejected` | Bundle with `zoneUid` not matching `store_meta.zone_uid` is rejected |
| `nix-runtime-zonename-mismatch-rejected` | Bundle with `zoneName != Zone self-resource name` is rejected |
| `nix-runtime-activation-nonblocking` | New generation activates and serves requests before cleanup of prior generation completes |
| `nix-runtime-provider-config-invalid-continues` | Provider `config` failing schema re-check sets `Failed` with `ConfigValid=False` condition but does not block other resources from activating |

#### Cleanup tests

| Test | Assertion |
| --- | --- |
| `cleanup-config-owned-absent-resource-deleted` | Provider present in generation N but absent from N+1 receives async Delete |
| `cleanup-controller-created-resource-preserved` | Controller-created `Process/provider-primary-0` (not in bundle) is NOT deleted on generation change |
| `cleanup-bootstrap-provider-preserved` | `Provider/system-core` absent from operator config is NOT deleted on any generation change |
| `cleanup-role-deletion-blocked-by-binding` | Config-owned Role awaiting cleanup is blocked by `core.role-binding-drain` until dependent RoleBinding is deleted |
| `cleanup-rolebinding-auto-deleted-when-role-deleted` | Authorization handler triggers RoleBinding deletion when parent Role has `deletionRequestedAt` set and is awaiting `core.role-binding-drain` clearance |
| `cleanup-provider-stops-processes-on-delete` | Config-owned Provider with `deletionRequestedAt` set stops owned component Processes via provider lifecycle handler |
| `cleanup-credential-revoke-on-delete` | Credential with `deletionRequestedAt` set triggers runtime to revoke resolved secret binding before finalizer clearance |
| `cleanup-status-pending-count-accurate` | `Zone.status.cleanupPendingCount` equals exact count of cleanup candidates with `deletionRequestedAt` set |
| `cleanup-zone-degraded-while-pending` | Zone.status.phase is Degraded while any cleanup candidate has `deletionRequestedAt` set; reverts when all complete |
| `cleanup-condition-clears-on-completion` | `GenerationCleanupPending` condition clears, count resets to 0, and Zone.status.phase reverts after all cleanup candidates complete |
| `cleanup-stuck-sets-degraded` | Resource awaiting deletion beyond `cleanupStuckThreshold` keeps Zone.status.phase=Degraded and additionally sets `GenerationCleanupFailed=True` |
| `cleanup-stuck-message-no-content` | `GenerationCleanupFailed` condition message contains ResourceType but no resource name, spec content, or secret value |
| `cleanup-audit-activate-emitted` | Generation activation emits `zone.config.generation.activate` audit record |
| `cleanup-audit-complete-emitted` | Cleanup completion emits `zone.config.cleanup.complete` audit record |
| `cleanup-audit-stuck-emitted` | Stuck cleanup emits `zone.config.cleanup.stuck` audit record |
| `cleanup-audit-no-resource-names` | All cleanup audit records contain no resource names, spec content, or secret values |
| `rollback-restores-prior-generation` | `d2b zone config rollback` re-creates deleted config-owned resources from prior bundle |
| `rollback-deletes-superseded-additions` | Rollback issues async Delete for resources added by the superseded generation but absent from prior bundle |
| `rollback-requires-prior-generation-retained` | Rollback fails if prior generation bundle has been pruned |
| `rollback-emits-audit-record` | Rollback emits `zone.config.generation.rollback` audit record |


### 15.9 Quota and EmergencyPolicy tests

| Test | Assertion |
| --- | --- |
| `quota-ceiling-hard-reject` | Resource creation over `maxResources` ceiling with `enforcementPolicy=hard` rejected with `quota-exceeded` |
| `quota-ceiling-soft-warn` | Same but `enforcementPolicy=soft`; resource admitted; `overQuota=true` in status |
| `quota-ceiling-pertype` | Resource creation over `maxResourcesPerType` ceiling for a specific ResourceType rejected |
| `quota-drain-blocks-on-dependents` | Quota deletion with `dependentCount > 0` keeps `core.quota-drain`; `QuotaDrainPending=True` condition message names the count; no quotaRef on other resources is modified |
| `quota-over-quota-status` | Quota status `usedResources` and `overQuota` reflect current usage |
| `quota-nix-eval-bounds` | `ceilings.maxOwnerDepth = 33` fails eval; `33 > 32` |
| `quota-nix-build-pertype-unknown-type` | `perTypeCeilings` entry for a ResourceType not in Zone API catalog fails build |
| `emergency-policy-activates-gate` | EmergencyPolicy with `enabled=true` and `stopNewAdmissions=true` causes new resource admissions to return admission-denied |
| `emergency-policy-disconnects-zonelinks` | EmergencyPolicy activation with `disconnectZoneLinks=true` triggers ZoneLink graceful disconnect |
| `emergency-policy-multiple-enabled-union` | Two enabled EmergencyPolicy resources with `stopNewAdmissions=true` and `disconnectZoneLinks=true` respectively produce a combined union where both effects apply Zone-wide |
| `emergency-policy-union-deactivate-partial` | Deactivating one of two enabled EmergencyPolicy resources removes only its contribution; the other policy's effects remain |
| `emergency-policy-deactivation-restores-gate` | Setting `enabled=false` restores normal admissions |
| `emergency-policy-stop-processes-no-delete` | EmergencyPolicy with `stopProviderProcesses=true` stops running Provider component Processes without setting `deletionRequestedAt`; Process resources remain; reconciliation resumes on deactivation |
| `emergency-policy-drain-finalizer-on-active-delete` | Deleting an EmergencyPolicy with `active=true` adds `core.emergency-drain`; drain completes before final atomic deletion |
| `emergency-reason-visible-in-spec` | EmergencyPolicy `reason` is readable in the resource spec via API; it does not appear in `status.*`, OTEL metric label values, or structured log labels |

---

## 16. Current-code fit

All seven Zone control ResourceTypes as stored resource objects are **`ADR-only`**:
no Zone, ZoneLink, Provider, Role, or RoleBinding resource schema, store table,
or controller handler exists in the v3 baseline (`b5ddbed6`). The subsections
below record every Realm-related baseline symbol that maps to Zone control
concepts, with exact evidence class and architectural mapping notes.

### 16.1 Why Realm→Zone is not a textual rename

ADR 0043 (`docs/adr/0043-realm-native-control-plane.md`, Accepted) defines a
Realm as a **runtime control-plane process pair**: each active realm boundary is
served by its own `d2bd` instance and its own privileged broker, with a distinct
socket, state directory, audit log, cgroup slice, nftables partition, and
identity key. A Realm is a running process boundary, not a data record.

A **Zone** (ADR 0046) is a typed ResourceType stored in a redb object store,
reconciled by the core controller. It is a resource object representing the
local isolation and policy domain of the store that owns it - one Zone per
store, cardinality-1 enforced. It is not itself a process; the Zone runtime
that hosts the store and core controller is the process, but the Zone resource
is just the authoritative metadata record in that runtime.

The key architectural differences that implementations must track:

| Dimension | Baseline Realm (ADR 0043) | Target Zone (ADR 0046) |
| --- | --- | --- |
| Identity model | `RealmPath(Vec<RealmId>)`, a tree path; realms may nest up to `MAX_REALM_LABELS=16` | `metadata.name` (single label); compiler-only `parentZone` selects the parent while a child-local ZoneLink supplies transport/route state |
| Process model | Each realm = one `d2bd` + one broker (multiple pairs on one host) | Zone is a resource record; Zone runtime is the surrounding process |
| Hierarchy representation | Tree path embedded in identity; parent realm is a routing ancestor in `RouteTreeEngine`/`RealmEntrypointTable` | `parentZone` compiles to sealed private allocator topology; the child store holds only its local ZoneLink; no path encoding or reciprocal parent row |
| Config loading | `realm-controllers.json` loaded at daemon startup (`load_realm_controllers_config` at `d2bd/src/lib.rs:1408`); routing remains "inert" but metadata is live | Zone self resource read from redb store; no external JSON config file for Zone identity |
| Access resolution | `RealmAccessResolverRequest/Response` + `RealmEntrypointTable` loaded from `/run/current-system/sw/share/d2b/realm-entrypoints.json`; reachable in CLI routing path (`d2b/src/target_routing.rs`) | ZoneLink resource with ComponentSession transport binding; access resolution is a resource-lifecycle operation |
| Provider placement | `RealmControllerPlacement` enum (`HostLocal`, `GatewayVm`, `CloudFullHost`, `ProviderController`, `ProviderAgent`) - runtime property of where realm's d2bd runs | Provider component `placement` field - descriptor in Provider resource spec; different semantics (component placement, not realm process placement) |
| Host-resource allocation | `LocalRootAllocatorEngine` in `d2b-realm-core/src/allocator_engine.rs:332`; typed leases for cgroup subtrees, nftables partitions, socket paths, bridges; leases tied to realm broker lifecycle | Resource store lifecycle + broker operations; allocator concepts move into Zone runtime internal mechanics, not ResourceTypes |
| Routing | `RouteTreeEngine`/`DescendantRoute`/`TreeRoutePath`/`RouteAdvertisement` (tree discovery); `OperationRouter` dedup; `RemoteNodeRegistry` (full-host nodes) - all `implemented-but-unwired` | Not scope of Zone control types; ZoneLink establishes point-to-point connection, not tree route advertisements |
| Auth/admission | Coarse `PeerRole::{Admin,Launcher,HostShutdown}` per-connection; `DaemonAccessPolicyRole::RealmAdmin` | Native RBAC: Role resource (rule table) + RoleBinding resource (subject binding); per-operation rather than per-connection |

### 16.2 Exhaustive symbol evidence matrix

Evidence classes:
- `implemented-and-reachable` - in live call path, not gated by dead-code allow
- `implemented-but-unwired` - compiles and is tested but not called from live daemon/CLI code
- `generated-or-eval-contract` - Nix emitter or generated JSON artifact; consumed at runtime
- `test-only-or-preview` - only in test harnesses

#### Identity and path types

| Symbol | File | Evidence class | ADR-0046 mapping |
| --- | --- | --- | --- |
| `RealmId` (label-shaped, `is_label()`) | `d2b-realm-core/src/ids.rs:211` | `implemented-and-reachable` | Zone `metadata.name`; ResourceName shape |
| `WorkloadId` (label-shaped) | `d2b-realm-core/src/ids.rs:225` | `implemented-and-reachable` | `metadata.name` for Guest or Host resources; `WorkloadProviderKind::LocalVm`/`QemuMedia` → Guest `metadata.name`; **`WorkloadProviderKind::UnsafeLocal` → Host `metadata.name` (user-only, `defaultDomain=user`, reconciled by `Provider/system-core`)** - NOT a Guest; `WorkloadProviderKind::ProviderManaged` → Guest or Host per provider semantics |
| `ProviderId` (label-shaped) | `d2b-realm-core/src/ids.rs:232` | `implemented-and-reachable` | Provider `metadata.name` |
| `NodeId` (label-shaped) | `d2b-realm-core/src/ids.rs:218` | `implemented-and-reachable` | Host `metadata.name` |
| `LABEL_PATTERN = "^[a-z][a-z0-9-]*$"`, `is_label()`, `MAX_ID_LEN = 128`, `IdError` | `d2b-realm-core/src/ids.rs:31,76,28,54` | `implemented-and-reachable` | ResourceName validator; extracted for ResourceRef parser |
| `RealmIdentityRef` (opaque, redacts debug) | `d2b-realm-core/src/ids.rs:312` | `implemented-and-reachable` | Opaque identity ref pattern; maps to Zone key-ref in ZoneLink key-pin model |
| `RealmPath(Vec<RealmId>)` | `d2b-realm-core/src/realm.rs:64` | `implemented-and-reachable` | **Not** a target type; encodes hierarchy structurally (path-in-identity). Compiler-only `parentZone` replaces the path topology; the child-local ZoneLink carries transport/route state. `RealmPath` depth (`MAX_REALM_LABELS=16`) maps to the compiled parent graph's ancestry bound |
| `MAX_REALM_LABELS = 16` | `d2b-realm-core/src/realm.rs:67` | `implemented-and-reachable` | Compiler-only `parentZone` ancestry bound |
| `RealmControllerPlacement::{HostLocal, GatewayVm, CloudFullHost, ProviderController, ProviderAgent}` | `d2b-realm-core/src/realm.rs:26` | `implemented-and-reachable` | **Partially reused**: these placement labels map to Provider component `placement` descriptor semantics, but the target is a Provider resource field, not a realm process property. Not a 1:1 rename. |
| `EntrypointMode::{HostResident, GatewayBacked}` | `d2b-realm-core/src/realm.rs:12` | `implemented-and-reachable` | Informs ZoneLink transport binding (host-local socket vs. remote transport); maps to the resolved `transportProviderRef` selector semantics |
| `RealmTarget { workload: WorkloadId, realm: RealmPath }` | `d2b-realm-core/src/target.rs:39` | `implemented-and-reachable` (CLI routing path) | `RealmTarget` is the current addressable unit (`<workload>.<realm>.d2b`); maps to `ResourceRef` (target scoped to Zone); NOT a Zone identity. `WorkloadTarget = RealmTarget` alias in `d2b-core/src/workload_identity.rs:55` |
| `TargetName`, `RealmTargetParser`, `RealmTargetParseError` | `d2b-realm-core/src/target.rs:122,373,280` | `implemented-and-reachable` | Maps to ResourceRef `<ResourceType>/<resource_name>` parser in ADR046-identities-001 |
| `LegacyNodeQualifiedTarget` | `d2b-realm-core/src/target.rs:242` | `implemented-and-reachable` | Migration artifact; removed after target format normalizes to ResourceRef |

#### Routing and access resolution types

| Symbol | File | Evidence class | ADR-0046 mapping |
| --- | --- | --- | --- |
| `RealmTreeEdge { parent: RealmPath, child: RealmPath }` | `d2b-realm-core/src/routing.rs:543` | `implemented-but-unwired` | Tree edge between two zones; ZoneLink resource replaces this structural edge with a stored resource object |
| `DescendantRoute`, `TreeRoutePath`, `TreeRouteHop` | `d2b-realm-core/src/routing.rs:581,854,798` | `implemented-but-unwired` | Multi-hop tree routing metadata; ADR 0046 replaces with ZoneLink cursor model (no multi-hop discovery in Zone control types) |
| `RouteAdvertisement`, `RouteAdvertisementEnvelope`, `RouteNamespaceAllocation` | `d2b-realm-core/src/routing.rs:609,697,716` | `implemented-but-unwired` | Route advertisement for peer discovery; no equivalent in Zone control types (ZoneLink uses ComponentSession, not route advertisements) |
| `RouteRealmClass::{LocalRoot, StaticConfigured, HostLocalPeer, GatewayBacked, CloudFullHost, ProviderManaged, EphemeralDiscovered, Unknown}` | `d2b-realm-core/src/routing.rs:1170` | `implemented-but-unwired` | Telemetry/audit realm class. Maps to ZoneLink low-cardinality transport class labels in OTEL metrics (§13); NOT a stored resource field |
| `RoutePlacementClass` | `d2b-realm-core/src/routing.rs:1201` | `implemented-but-unwired` | Low-cardinality placement label for telemetry; maps to Provider component placement OTEL label |
| `RouteAuditLabels`, `RouteTelemetryLabels`, `RouteTelemetrySample`, `RouteTelemetryBatch` | `d2b-realm-core/src/routing.rs:1244,1300,1313,1322` | `implemented-but-unwired` | Audit/telemetry shape precedent; maps to ZoneLink OTEL metrics label constraints in §13.2 |
| `RouteTreeEngine`, `RouteEngineEvent`, `RouteAdvertisementAdmission`, `DiscoveryQueueDecision`, `RoutePruneReport`, `DirectShortcutAuthorizationRequest` | `d2b-realm-core/src/route_engine.rs:161,30,37,51,70,78` | `implemented-but-unwired` | Live routing engine with dedup, pruning, shortcut auth; exported from `d2b_realm_core::lib.rs:122` but has **zero call sites in live daemon or CLI code** - only used in tests within `route_engine.rs`. ZoneLink does not use route advertisements; the `decide_route` algorithm is an unwired precedent only |
| `RealmEntrypointTable`, `RealmEntrypoint`, `DispatchTarget::{HostResident, GatewayBacked}`, `ResolveError` | `d2b-realm-router/src/target_resolver.rs:103,25,54` | `implemented-and-reachable` (CLI only) | The CLI (`d2b/src/lib.rs:5239`, `target_routing.rs:430`) loads the realm entrypoints file from `/run/current-system/sw/share/d2b/realm-entrypoints.json` (Nix-generated at `nixos-modules/host-daemon.nix:385`) and resolves targets. The daemon (`d2bd`) only uses it in `realm_stubs.rs` (dead code). Compiler-only `parentZone` topology plus child-local ZoneLink ComponentSession transport replace the entrypoint table |
| `OperationRouter<C>`, `RouteDecision`, `OperationRoutePlan`, `ReconcilableLease` | `d2b-realm-router/src/lib.rs:306,95,179,636` | `implemented-but-unwired` | In `d2bd` only via `realm_stubs.rs` (`#![allow(dead_code)]`; comment: "not called from the running daemon"); `OperationRouter` dedup model maps to resource API idempotency layer |
| `RemoteNodeRegistry`, `RemoteFullHostAdapter`, `RemotePeerClient`, `RemoteDispatchOutcome` | `d2b-realm-router/src/remote_node.rs:252,644,608,627` | `implemented-but-unwired` | Remote full-host node management; in d2bd only via realm_stubs dead-code. No Zone control type equivalent; future Host/node resource design |
| `RealmAccessTargetInput`, `RealmAccessAliasBinding`, `RealmAccessResolverRequest`, `RealmAccessResolverResponse`, `RealmAccessBinding`, `RealmTransportBinding`, `HostLocalPeerCredentialSemantics`, `AccessBindingRef` | `d2b-realm-core/src/access.rs:170,285,322,542,597,569,364,96` | `implemented-and-reachable` (CLI path) | Used actively in `d2b/src/target_routing.rs:21-26` which is called from live CLI routing. `RealmTransportBinding::{LocalUnixSocket,RemoteRealmTransport,ProviderRealmTransport}` maps to the resolved ZoneLink transport binding variants |
| `realm_access_resolver` module (`resolve_local_root_realm_access`, `host_local_capability_preflight`, `realm_controllers_config_generation`) | `d2bd/src/realm_access_resolver.rs:20,159,190` | `implemented-but-unwired` | `pub mod realm_access_resolver` in `d2bd/src/lib.rs:117`; no call sites in live daemon code (only within the module itself). Capability preflight model maps to Zone/ZoneLink capability surface in §13 |
| `realm_stubs` module (`ApiFrontend`, `ApiService`, `TargetResolver`, `PeerOperationRouter`, `ProviderExecutor`, `PeerDaemon`, `DaemonMode`, `SharedRouter`) | `d2bd/src/realm_stubs.rs` | `implemented-but-unwired` | Explicitly `#![allow(dead_code)]`; module comment: "not called from the running daemon - the local CLI→daemon path is unchanged (zero behavior change)". Seam for future gateway work. Maps to: `ApiFrontend`→ComponentSession framing; `ApiService`→Zone runtime API surface; `TargetResolver`→ZoneLink access |

#### Session types

| Symbol | File | Evidence class | ADR-0046 mapping |
| --- | --- | --- | --- |
| `PeerSession<C>` | `d2b-realm-router/src/session.rs` | `implemented-but-unwired` | No call sites in live d2bd/d2b code (d2bd imports `d2b_realm_router` only for `realm_stubs` dead code; d2b CLI imports only `RealmEntrypointTable` from router). Noise handshake/session framing model; reuse source for ComponentSession |
| `SecurePeerSession<C>` (Noise-based) | `d2b-realm-router/src/secure_session.rs` | `implemented-but-unwired` | Same reachability as `PeerSession`; Noise KK handshake implementation; **design precedent** for ComponentSession Noise KK (copy/adapt from main `a1cc0b2d`) |
| `MuxSession<C>` | `d2b-realm-router/src/mux_session.rs` | `implemented-but-unwired` | Stream multiplexing over a session; maps to ZoneLink named-stream framing model (copy/adapt from main `a1cc0b2d` d2b-bus) |
| `SessionLifecycle`, `SessionPhase` | `d2b-realm-router/src/session_lifecycle.rs` | `implemented-but-unwired` | FSM phases (current evidence: `Unknown`, `Connecting`, `Established`, `Failed`); design precedent for ZoneLink session reconnect loop; `Connecting`/`Established` are current baseline FSM states that map to ZoneLink `status.connected` detail field, not to `status.phase`; ZoneLink `status.phase` uses common Resource phases (`Pending`/`Ready`/`Degraded`/`Failed`/`Unknown`) |

#### Identity store and enrollment types

| Symbol | File | Evidence class | ADR-0046 mapping |
| --- | --- | --- | --- |
| `RealmIdentityStore`, `EnrollmentRecord`, `ChildKeyPin`, `ControllerGenerationId`, `RevocationList`, `RecoveryProcedure` | `d2b-realm-core/src/identity_store.rs:141,207,177,182,197,202` | `implemented-and-reachable` (d2b-realm-core is a live dependency) | Zone identity anchoring and child enrollment model. The identity store tracks enrolled realm key-pins, controller generations, and revocations. ZoneLink child key-pin binding adapts `ChildKeyPin` + `EnrollmentRecord`. **Not** a ResourceType; internal Zone runtime mechanism |
| `RealmIdentityConfigJson`, `RealmIdentityConfigEntry`, `RealmIdentityConfigInvariants`, `RealmIdentityConfigError` | `d2b-realm-core/src/identity_config.rs:19,69,89,124` | `implemented-and-reachable` | Deserialized by `load_realm_identity_config()` in live daemon startup (`d2bd/src/lib.rs:1425`); runtime trust sessions logged as "inert". Maps to Zone identity key-ref fields loaded from bundle |
| `RealmIdentityConfigRuntimeState::MetadataOnly` | `d2b-realm-core/src/identity_config.rs:63` | `implemented-and-reachable` | `runtimeState = "metadata-only"` invariant retained; all identity config validated as non-secret at startup |

#### Realm controller config types (generated-and-consumed)

| Symbol | File | Evidence class | ADR-0046 mapping |
| --- | --- | --- | --- |
| `RealmControllersJson`, `RealmControllerConfig`, `RealmControllerPlacement`, `RealmControllerLocalRuntime`, `RealmControllerLocalWorkload`, `RealmAllocatorBinding`, `RealmDaemonConfig`, `RealmBrokerConfig` | `d2b-core/src/realm_controller_config.rs:144,190,256,278,434,541,497,511` | `implemented-and-reachable` | Loaded in live daemon startup at `d2bd/src/lib.rs:1408` and in serve path at line `16741`; `WorkloadTargetIndex::build_from_controllers` is called live. Contains per-realm socket/state paths, placement, providers, workloads. Maps to: Zone self resource (the realm metadata row); compiler-only `parentZone` compiled into the private allocator binding (per-realm parent socket/edge data); child-local ZoneLink transport/route state; Provider (per-realm provider config rows); NOT replaced by a single file - replaced by resource store entries plus sealed allocator state |
| `WorkloadTargetIndex`, `TargetResolution`, `TargetResolutionError` | `d2bd/src/workload_target_index.rs:93,28,56` | `implemented-and-reachable` | Built from `RealmControllersJson` in live serve path (`d2bd/src/lib.rs:16745`); resolves `<workload>.<realm>.d2b` to VM name. Maps to resource API `resourceRef` lookup table in ADR 0046 API layer |

#### Allocator types

| Symbol | File | Evidence class | ADR-0046 mapping |
| --- | --- | --- | --- |
| `AllocatorLease`, `LeaseAllocationRequest`, `AllocatorLeaseState`, `LeaseOwner`, `AllocatorReasonCode`, `HostResourceKind`, `ResourceShareMode` | `d2b-realm-core/src/allocator.rs:274,151,241,71,310,41,84` | `implemented-and-reachable` | Core allocator data model. Leases typed by `HostResourceKind` (`HostFilePartition`, `CgroupSubtree`, `NftablesPartition`, `Bridge`, `NamespaceBoundary`). Maps to Zone runtime internal resource management; these are **mechanisms, not ResourceTypes** |
| `LocalRootAllocatorEngine` | `d2b-realm-core/src/allocator_engine.rs:332` | `implemented-and-reachable` | Test-facing fake engine; drives idempotency/reconciliation proof. Maps to Zone runtime allocator internals; NOT a ResourceType or Zone store table |
| `AllocatorEngineDecision`, `AllocatorEngineOutcome`, `AllocatorAllocationDecision`, `AllocatorReconciliationAction` | `d2b-realm-core/src/allocator_engine.rs:22,78,152,170` | `implemented-and-reachable` | Allocation decision types; maps to Zone runtime startup allocation invariants |

#### Provider trait types

| Symbol | File | Evidence class | ADR-0046 mapping |
| --- | --- | --- | --- |
| `HostSubstrateProvider`, `RuntimeProvider`, `WorkloadProvider`, `DurableExecutionProvider`, `InfrastructureProvider`, `NodeProvider`, `TransportProvider`, `RelayProvider`, `CredentialProvider`, `PersistentShellProvider`, `DisplayProvider` traits | `d2b-realm-provider/src/provider.rs:31,41,65,89,268,333,197,324,301,137,169` | `implemented-and-reachable` (`WorkloadProvider` is imported in live `d2bd/src/lib.rs:93` for ACA) | Provider trait surface. Each trait axis maps to a Provider component service definition; `WorkloadProvider` is the only one with a live call path (ACA gateway). Remaining traits are reachable via `d2b-realm-provider` dependency but have no live non-ACA call sites |
| `RuntimeCapabilitySet`, `WorkloadCapabilitySet`, `NodeCapabilitySet` | `d2b-realm-provider/src/capabilities.rs` | `implemented-and-reachable` | Maps to Provider `spec.components[].supportedCapabilities` |
| `workload_lists_and_advertises()`, `display_fails_closed_when_unsupported()` | `d2b-realm-provider/src/conformance.rs` | `implemented-and-reachable` | Provider conformance check behavior; reused in Provider trust check (ADR046-zone-control-003) |
| `ProviderError`, `ErrorKind`, `RetryHint`, `ProviderDiagnostic` | `d2b-realm-provider/src/error.rs` | `implemented-and-reachable` | Provider lifecycle error/retry schema; maps to Provider `status.conditions[].message` and error bounds |

#### Process and supervisor types

| Symbol | File | Evidence class | ADR-0046 mapping |
| --- | --- | --- | --- |
| `ProcessRole::{CloudHypervisorRunner, Virtiofsd, Swtpm, GpuRenderNode, Audio, Video, QemuMediaRunner, VsockRelay, OtelHostBridge, Usbip}` | `d2b-core/src/processes.rs:194` | `implemented-and-reachable` | Each `ProcessRole` variant maps to a declared Process or EphemeralProcess resource under a Provider component |
| `VmProcessDag`, `ProcessNode` | `d2b-core/src/processes.rs:20,57` | `implemented-and-reachable` | Provider component DAG shape; maps to Process/EphemeralProcess dependency graph |
| `DagExecutor`, `NodeRunner`, `topo_sort()`, `NodeOutcome`, `NodeBudget` | `d2bd/src/supervisor/dag.rs:302,181,222,38,148` | `implemented-and-reachable` | DAG lifecycle executor; maps to Process/EphemeralProcess launch ordering in Provider component lifecycle |

#### Authorization and admission types

| Symbol | File | Evidence class | ADR-0046 mapping |
| --- | --- | --- | --- |
| `PeerRole::{Admin, Launcher, HostShutdown}`, `PeerIdentity`, `authorize_peer()`, `verb_requires_admin()`, `verb_allowed_for_host_shutdown()` | `d2bd/src/admission.rs:16,10,37,123,157` | `implemented-and-reachable` | **Current bootstrap auth system**. Called in live daemon request path. Maps to: `PeerRole::Admin`→`system-core` subject; `PeerRole::Launcher`→`system-minijail` subject; `PeerRole::HostShutdown`→narrow bootstrap exception; verb tables → Role rule verb enum |
| `LocalUnixAllowlistRole::{Admin, Launcher, Denied}`, `DaemonAccessPolicyRole::RealmAdmin`, `DaemonAccessDecision::{Authorized,Denied}`, `DaemonAccessAdmissionSource`, `MappedDaemonAccessPrincipal::{LocalAdmin,LocalLauncher,LocalDenied}`, `map_local_unix_daemon_access()`, `map_remote_daemon_access()` | `d2b-daemon-access/src/lib.rs:361,370,390,415,468,487` | `implemented-and-reachable` | **Current local auth surface**. `LocalUnixAllowlistRole` determines `PeerRole` by group membership (`Admin`→`LocalAdmin`, `Launcher`→`LocalLauncher`). Maps to Role/RoleBinding RBAC engine (§6-§9). `DaemonAccessPolicyRole::RealmAdmin` maps to a future Role grant for realm operators; `MappedDaemonAccessPrincipal` maps to RoleBinding subject identity model |

#### Nix authoring and generated artifacts

| Symbol | File | Evidence class | ADR-0046 mapping |
| --- | --- | --- | --- |
| `d2b.realms.<realm>.*`, `providerType` submodule, `providerKind = "^[a-z][a-z0-9-]*$"`, `placementProvider`, `providerSpecificPlacement` | `nixos-modules/options-realms.nix:26,39,55,65,170` | `generated-or-eval-contract` | Authoring interface for realm/provider declarations. Label regex identical to `LABEL_PATTERN` in `ids.rs`. Adapted to `d2b.zones.<zone>.providers.*` in ADR046-zone-control-007 |
| `d2b.realms.<realm>.workloads.*`, `kind`, `placement`, `legacyVmName` | `nixos-modules/options-realms-workloads.nix` | `generated-or-eval-contract` | Workload authoring submodule; `kind = "local-vm"` / `"qemu-media"` → Guest resource Nix options; **`kind = "unsafe-local"` → Host resource Nix options** (`defaultDomain=user`, `allowedDomains=[user]`); `kind = "provider-placeholder"` → Guest/Host per provider semantics |
| `d2b.realms.<realm>.network.*` | `nixos-modules/options-realms-network.nix` | `generated-or-eval-contract` | Network/bridge authoring; maps to Network resource (not Zone control types) |
| `realm-controllers.json` (`RealmControllersJson`), `schemaVersion = "v2"`, `runtimeState = "metadata-only"`, per-realm rows | `nixos-modules/realm-controller-config-json.nix` (emitter); `d2b-core/src/realm_controller_config.rs:144` (deserializer) | `generated-or-eval-contract` (Nix); `implemented-and-reachable` (consumption in daemon) | **Loaded live in daemon** at `d2bd/src/lib.rs:1408,16741`. Runtime routing "remains inert" but `WorkloadTargetIndex` is built from it. Maps to: Zone self resource (identity row); compiler-only `parentZone` plus private parent allocator binding (topology/socket data); child-local ZoneLink transport/route state; Provider (provider config rows). The bundle artifact is replaced by child resource-store entries and sealed allocator state; until then, the schema is the live authority |
| `realm-identity.json` (`RealmIdentityConfigJson`), identity refs, fingerprints | `nixos-modules/realm-identity-config-json.nix` (emitter); `d2b-realm-core/src/identity_config.rs:19` (deserializer) | `generated-or-eval-contract` (Nix); `implemented-and-reachable` (consumption in daemon) | Loaded live at `d2bd/src/lib.rs:1425`; trust sessions "remain inert". Maps to Zone runtime identity key-ref state and store metadata, not Zone.spec |
| `allocator.json` resource request rows (per-realm: cgroup, nftables, bridges, socket paths), `LocalRootAllocatorEngine` | `nixos-modules/allocator-json.nix` (emitter); `d2b-realm-core/src/allocator_engine.rs:332` (engine) | `generated-or-eval-contract` (Nix); `implemented-and-reachable` (engine) | Allocator resource requests are generated per realm from `d2b.realms.*` Nix config and consumed by the allocator at runtime. Maps to compiler-only `parentZone` topology plus Zone runtime startup resource claiming. **Mechanism, not a ResourceType** |
| `realm-entrypoints.json`, loaded from `/run/current-system/sw/share/d2b/realm-entrypoints.json` | `nixos-modules/host-daemon.nix:385` (emitter); `d2b/src/lib.rs:5239` (loader) | `generated-or-eval-contract` (Nix); `implemented-and-reachable` (CLI consumption) | Loaded by `d2b` CLI in live routing path for realm-qualified targets. Replaced by the sealed `parentZone` topology plus child-local ZoneLink Resource/ComponentSession lookup |

#### Workload posture and unsafe-local types

> These symbols ground the `unsafe-local` → **Host** ResourceType mapping.
> `WorkloadProviderKind::UnsafeLocal` / `IsolationPosture::UnsafeLocal` do NOT
> map to a Guest resource or a Provider resource; they map exclusively to a
> user-domain Host resource reconciled by `Provider/system-core`.

| Symbol | File | Evidence class | ADR-0046 mapping |
| --- | --- | --- | --- |
| `WorkloadProviderKind::{LocalVm, QemuMedia, ProviderManaged, UnsafeLocal}` | `d2b-realm-core/src/workload.rs:13` | `implemented-and-reachable` | `LocalVm`/`QemuMedia` → Guest resource kind; **`UnsafeLocal` → Host resource (`defaultDomain=user`, `allowedDomains=[user]`)** - NOT a Provider, NOT a Guest; `ProviderManaged` → Guest or Host per provider semantics |
| `IsolationPosture::{VirtualMachine, ProviderManaged, UnsafeLocal}` | `d2b-realm-core/src/workload.rs:27` | `implemented-and-reachable` | `VirtualMachine` → Guest isolation; **`UnsafeLocal` → Host `spec.isolationPosture=no-isolation`**; no-isolation posture is preserved as a mandatory warning in Host status conditions, CLI/UI output, and audit records (`isolation=no-isolation`); it is never emitted as an OTEL metric label or span attribute |
| `WorkloadExecutionPosture { isolation, environment, display_environment, execution_identity, session_persistence }` | `d2b-realm-core/src/workload.rs:83` | `implemented-and-reachable` | Typed posture carried by launcher entries for CLI/desktop display. Canonical unsafe-local tuple at line 206-211: `isolation=unsafe-local`, `environment=systemd-user-manager-ambient`, `displayEnvironment=wayland-proxy-only`, `executionIdentity=authenticated-requester-uid`, `sessionPersistence=user-manager-lifetime`. Tuple maps to Host `status.observedPosture` and mandatory CLI/UI warnings; posture details appear in audit record body but are not OTEL labels |
| `EnvironmentPosture::{RuntimeManaged, SystemdUserManagerAmbient}` | `d2b-realm-core/src/workload.rs:39` | `implemented-and-reachable` | `SystemdUserManagerAmbient` is the unsafe-local environment posture; maps to Host `status.observedPosture.environment` field and audit record body; not emitted as an OTEL label |
| `ExecutionIdentityPosture::{WorkloadUser, ProviderManaged, AuthenticatedRequesterUid}` | `d2b-realm-core/src/workload.rs:61` | `implemented-and-reachable` | `AuthenticatedRequesterUid` is the unsafe-local identity posture; maps to Host `spec.defaultUserRef=User/<name>` and audit label `execution_identity=authenticated-requester-uid` |
| `SessionPersistencePosture::{RuntimeManaged, UserManagerLifetime}` | `d2b-realm-core/src/workload.rs:73` | `implemented-and-reachable` | `UserManagerLifetime` is the unsafe-local session posture; maps to Host status field noting session lifetime bound to the systemd user manager |
| `UnsafeLocalWorkloadsJson`, `UnsafeLocalWorkload`, `UnsafeLocalLauncherItem::{Exec, Shell}`, `UnsafeLocalExecItem`, `UnsafeLocalShellItem`, `UnsafeLocalShellPolicy` | `d2b-core/src/unsafe_local_workloads.rs:16,47,106,129,141,150` | `implemented-and-reachable` (consumed by `d2b-core/src/bundle_resolver.rs:85,106`) | Private configured-item contract loaded via bundle resolver. `UnsafeLocalWorkload.identity` → Host `metadata.name` + `spec.defaultUserRef`; `UnsafeLocalShellPolicy.{defaultName, maxSessions}` → Host `spec.shellPolicy`; `UnsafeLocalExecItem` → Host launcher item contract. Constants: `MAX_UNSAFE_LOCAL_WORKLOADS=256`, `MAX_LAUNCHER_ITEMS_PER_WORKLOAD=64`, `MAX_UNSAFE_LOCAL_SHELL_SESSIONS=64` → Host cardinality bounds |
| `HelperRegistry`, `bind_helper_socket`, `dispatch_launch`, `HelperReply`, `HelperSnapshot`, `active_generation()` | `d2bd/src/unsafe_local_helper.rs:41,62,149,167,176,208,221` | `implemented-and-reachable` (live in `d2bd/src/lib.rs:1346-1468`) | Per-uid launch broker for unsafe-local helper sessions, bound at daemon startup (`d2bd/src/lib.rs:1356`). Maps to Zone runtime Host/Process broker: `dispatch_launch` → Process launch request; `HelperRegistry.allowed_uids` → Host subject UID allowlist derived from `spec.defaultUserRef` |
| `d2b.realms.<realm>.policy.allowUnsafeLocal` | `nixos-modules/options-realms.nix:346` | `generated-or-eval-contract` | Gate option that permits `kind = "unsafe-local"` workloads in a realm; assertion at `nixos-modules/assertions.nix:730` blocks unsafe-local without this flag. Maps to a separate Host admission gate for user-domain unsafe-local resources, not Zone.spec |
| `kind = "unsafe-local"` enum value, doc "Host-user process runtime with no isolation boundary" | `nixos-modules/options-realms-workloads.nix:221,233` | `generated-or-eval-contract` | Nix workload `kind` enum value for unsafe-local Host resources. Maps to Host resource with `spec.defaultDomain=user` authored via `d2b.zones.<zone>.resources.<name> = { type = "Host"; spec = { ... }; };` (ADR046-zone-control-008) |
| Assertion: `!unsafeLocal \|\| realm.policy.allowUnsafeLocal` | `nixos-modules/assertions.nix:730` | `generated-or-eval-contract` | Eval-time gate; maps to Host admission enforcement for unsafe-local creation outside Zone.spec |

### 16.3 Required delta

None of the following exist in baseline:

- Zone, ZoneLink, Provider, Role, RoleBinding resource schemas (Rust structs + JSON schemas)
- redb store physical tables for all seven types
- Core-controller handlers for all seven types
- Native RBAC engine (Role index + RoleBinding evaluation)
- Bootstrap authorization (compiled constant policy superseded by stored RBAC)
- `d2b.zones.*` Nix options and resource compiler
- Audit/OTEL instrumentation per §13

### 16.4 Reuse path summary

| Baseline symbol | Reuse action | Destination |
| --- | --- | --- |
| `is_label()`, `LABEL_PATTERN`, `MAX_ID_LEN`, `IdError` | extract | `d2b-contracts/src/v3/resource_ref.rs` ResourceName validator |
| `RealmControllerPlacement` label set | adapt (semantics change: process placement → component placement) | Provider `spec.components[].placement` field enum |
| `EntrypointMode::{HostResident,GatewayBacked}` | adapt | ZoneLink transport binding selector |
| `RealmTransportBinding::{LocalUnixSocket,RemoteRealmTransport,ProviderRealmTransport}` | adapt | ZoneLink `spec.transportProviderRef` + transport settings variants |
| `SecurePeerSession<C>` (Noise KK) | copy/adapt via main `a1cc0b2d` | ComponentSession Noise KK handshake (ADR046-zone-control-018) |
| `SessionLifecycle`, `SessionPhase` | adapt | ZoneLink session reconnect loop and connection detail fields (`status.connected`, `status.lastConnectedAt`); `Connecting`/`Established` current evidence phases do not become `ZoneLink.status.phase` values |
| `RouteRealmClass`/`RoutePlacementClass` label strings | reuse label values | ZoneLink OTEL telemetry labels (§13.2) |
| `workload_lists_and_advertises()`, `display_fails_closed_when_unsupported()` | adapt | Provider trust/conformance check (ADR046-zone-control-003) |
| `RuntimeCapabilitySet`, `WorkloadCapabilitySet`, `NodeCapabilitySet` | adapt | Provider component `supportedCapabilities` fields |
| `ProviderError`, `ErrorKind`, `RetryHint` | adapt | Provider `status.conditions[].message` + error bounds |
| `ProcessRole` variants | adapt (each → Process/EphemeralProcess resource `spec.role`) | Provider component type identifiers |
| `authorize_peer()` verb table, `PeerRole` two-role model | adapt | Bootstrap authorization constant policy (ADR046-zone-control-006) |
| `map_local_unix_daemon_access()` SO_PEERCRED derivation | adapt | Role/RoleBinding subject identity derivation |
| `providerType` submodule, label regex, `placement` option | adapt | `d2b.zones.<zone>.providers.*` Nix option submodule |
| `WorkloadExecutionPosture` unsafe-local posture tuple | adapt | Host `status.observedPosture`; mandatory CLI/UI posture warnings; audit record body (no OTEL posture labels) |
| `UnsafeLocalShellPolicy.{defaultName, maxSessions}`, `MAX_UNSAFE_LOCAL_SHELL_SESSIONS=64`, `MAX_UNSAFE_LOCAL_WORKLOADS=256` | adapt | Host `spec.shellPolicy` bounds and per-Zone Host cardinality bounds |
| `HelperRegistry.dispatch_launch`, `bind_helper_socket`, `allowed_uids` | adapt | Zone runtime Host process launch broker; subject UID allowlist derived from `spec.defaultUserRef` |
| `d2b.realms.<realm>.policy.allowUnsafeLocal` option | adapt | `d2b.zones.<zone>.resources.*` Host ResourceType admission gate (separate opt-in required before `type = "Host"` with `defaultDomain=user` is permitted) |

### 16.5 Replacement and deletion

| Baseline artifact | Replacement trigger | Retention period |
| --- | --- | --- |
| `realm_stubs.rs` dead-code module | ADR046-zone-control-018 ComponentSession integration | Until gateway-mode work integrates live paths |
| `realm_access_resolver.rs` unwired module | ZoneLink resource + ComponentSession replaces entrypoint-table resolution | Same wave as ADR046-zone-control-002 |
| `PeerRole` coarse per-connection auth (`d2bd/admission.rs`) | Role/RoleBinding RBAC engine covers all verb gates | ADR046-zone-control-004/006 complete |
| `d2b-realm-provider` trait crate (all traits) | Provider resource + ComponentSession service definitions | ADR046-zone-control-017 integration complete |
| `realm-controllers.json` + `realm-identity.json` bundle artifacts | Zone self resource + ZoneLink resources in store | ADR046-zone-control-001/002 integration + Nix compiler (ADR046-zone-control-007) |
| `options-realms.nix`, `options-realms-workloads.nix`, `options-realms-network.nix` | `d2b.zones.*` Nix options | Purge wave (after all consumers migrate) |
| `realm-entrypoints.json` CLI config | ZoneLink access resolution via resource API | ADR046-zone-control-002 + CLI routing update |
| `unsafe-local-workloads.json` bundle artifact (`UnsafeLocalWorkloadsJson`) | Host resource store entries + Process Providers | ADR046-zone-control-008 complete |
| `d2bd/src/unsafe_local_helper.rs` `HelperRegistry` / `bind_helper_socket` | Zone runtime Host/Process broker | ADR046-zone-control-008 integration |
| `kind = "unsafe-local"` Nix enum value in `options-realms-workloads.nix` | `d2b.zones.<zone>.resources.<name> = { type = "Host"; spec = { defaultDomain = "user"; ... }; }` authoring | Purge wave (with full `options-realms*.nix` removal) |

---

## 17. Implementation work items

### ADR046-zone-control-001

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-zone-control-001` |
| Dependency/owner | ADR046-object-001 (resource envelope); ADR046-store-001 (redb store); ADR046-identities-001 (types) |
| Current source | `packages/d2b-realm-core/src/ids.rs` (`RealmId`, `LABEL_PATTERN = "^[a-z][a-z0-9-]*$"`, `is_label()`, `MAX_ID_LEN = 128`, `IdError` - `implemented-and-reachable`, baseline `b5ddbed6`); `packages/d2b-realm-core/src/realm.rs` (`RealmPath`, `MAX_REALM_LABELS = 16`, `RealmControllerPlacement`, `EntrypointMode` - `implemented-and-reachable`, baseline `b5ddbed6`); Zone resource schema: `ADR-only` |
| Reuse action | adapt |
| Destination | `packages/d2b-contracts/src/v3/zone.rs`; `packages/d2b-core-controller/src/zone.rs` |
| Detailed design | Zone ResourceType schema with `spec = {}`; self-resource enforcement; store_meta binding checks; phase/conditions; cardinality-1 enforcement; `zone-drain` finalizer; canonical JSON schema. Nix Zone options include compiler-only `parentZone`: required for every non-root Zone, forbidden on `local-root`, and never emitted into `Zone.spec` Primary reuse disposition: `adapt`. Preserved source-plan detail: extract and adapt (`RealmId`→`ResourceName` validators, `RealmPath` depth bound→ZoneLink depth, `RealmControllerPlacement`→Provider component placement); new (Zone resource schema/store table/handler). |
| Integration | Zone runtime open/upgrade verifies `Zone/<name>` self resource; configuration publication handler creates/reconciles Zone; resource API rejects cross-Zone refs |
| Data migration | Destructive d2b 3.0 reset; no v2/Realm Zone resource import |
| Validation | `zone-self-resource-enforced`, `zone-uid-mismatch-quarantine`, `zone-name-mismatch-rejected`, `zone-cardinality-one`, `zone-cross-zone-ref-rejected`, `zone-owner-rejected`, `zone-deletion-only-on-drain` |
| Removal proof | `d2b-realm-core` Realm struct removed only after Zone resource integration is live in Zone runtime |

### ADR046-zone-control-002

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-zone-control-002` |
| Dependency/owner | ADR046-zone-control-001; ADR046-zone-control-018 (ComponentSession) |
| Current source | `packages/d2b-realm-router/src/session.rs` (`PeerSession<C>` - `implemented-but-unwired`, baseline `b5ddbed6`; no live call sites in d2bd or d2b - only reachable through `realm_stubs.rs` dead code in d2bd); `packages/d2b-realm-router/src/secure_session.rs` (`SecurePeerSession<C>`, Noise-based - `implemented-but-unwired`); `packages/d2b-realm-router/src/mux_session.rs` (`MuxSession<C>` - `implemented-but-unwired`); `packages/d2b-realm-router/src/session_lifecycle.rs` (`SessionLifecycle`, `SessionPhase` - `implemented-but-unwired`); `packages/d2b-realm-core/src/route_engine.rs` (`RouteTreeEngine`, `admit_advertisement()`, `decide_route()`, `RoutePruneReport` - `implemented-but-unwired`; exported from `d2b_realm_core::lib.rs:122` but zero call sites in live daemon or CLI code, tests only); `packages/d2b-realm-core/src/identity_store.rs` (`RealmIdentityStore`, `EnrollmentRecord`, `ChildKeyPin` - `implemented-and-reachable`); `packages/d2b-realm-core/src/realm.rs` (`RealmPath`, `MAX_REALM_LABELS = 16` - `implemented-and-reachable`); `packages/d2b-realm-core/src/access.rs` (`RealmTransportBinding::{LocalUnixSocket,RemoteRealmTransport,ProviderRealmTransport}` - `implemented-and-reachable` in CLI routing path); ZoneLink resource schema and cursor tables: `ADR-only` |
| Reuse source | main `a1cc0b2d`: `packages/d2b-session/src/lifecycle.rs`, `d2b-session-unix/src/adapter.rs` for reconnect/transport precedents |
| Reuse action | adapt |
| Destination | `packages/d2b-contracts/src/v3/zone_link.rs`; `packages/d2b-core-controller/src/zone_link.rs` |
| Detailed design | Child-local ZoneLink schema with self-matching `spec.childZoneName` plus same-Zone `spec.transportProviderRef`, `spec.transportSettings`, and `spec.transportCredentials`; at most one uplink resource per non-root Zone and none in local root; child-store cursor/intent state; reconnect loop with exponential backoff; local intent queue (max 256 entries); local child UID change detection; drain finalizer; no reciprocal parent-store resource; compiler-only `parentZone` selects the one allocator owner, while that allocator owns privileged listener/placement/route allocation effects and exposes only a sealed bootstrap/allocation interface Primary reuse disposition: `adapt`. Preserved source-plan detail: extract and adapt (`SecurePeerSession` Noise model → ComponentSession Noise KK; `SessionLifecycle`/`SessionPhase` → ZoneLink session reconnect loop and connection detail fields; `Connecting`/`Established` current evidence phases drive `status.connected` and `status.phase` transitions to `Pending`/`Ready`, not direct phase values; `RouteTreeEngine.decide_route()` → cursor tracking; `RealmIdentityStore` enrollment → ZoneLink child key-pin). |
| Integration | child core-controller zone_link handler; child redb `zone_link_cursors` table; Nix-compiled `parentZone` bootstrap topology; selected parent allocator and route engine through sealed allocation authority; d2b-bus transport resolver; ComponentSession lifecycle |
| Data migration | Destructive reset; no v2 Realm peer migration |
| Validation | `zonelink-reconnect-child-uid-change`, `zonelink-disconnect-unknown-phase`, `zonelink-intent-queue-limit`, `zonelink-disabled-no-reconnect`, `zonelink-child-auth-denied-failed`, `zonelink-drain-closes-session`, `zonelink-child-name-matches-store`, `zonelink-one-child-local-uplink`, `zonelink-parent-bootstrap-binding`, `zonelink-parent-has-no-reciprocal-row` |
| Removal proof | `realm_stubs.rs` (`ApiFrontend`, `PeerOperationRouter`, `TargetResolver`) removed after ComponentSession integration (ADR046-zone-control-018); `realm_access_resolver.rs` module removed after ZoneLink replaces entrypoint-table resolution; gateway `PeerSession`/`SecurePeerSession` session types remain as dead code in d2b-realm-router until Provider session migration wave |

### ADR046-zone-control-003

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-zone-control-003` |
| Dependency/owner | ADR046-zone-control-001; ADR046-api-001; ADR046-zone-control-017 |
| Current source | `packages/d2b-realm-provider/src/provider.rs` (`HostSubstrateProvider`, `RuntimeProvider`, `WorkloadProvider`, `DurableExecutionProvider`, `InfrastructureProvider`, `NodeProvider` traits - `implemented-and-reachable`, baseline `b5ddbed6`); `packages/d2b-realm-provider/src/capabilities.rs` (`RuntimeCapabilitySet`, `WorkloadCapabilitySet`, `NodeCapabilitySet` - `implemented-and-reachable`); `packages/d2b-realm-provider/src/conformance.rs` (`workload_lists_and_advertises`, `display_fails_closed_when_unsupported` - `implemented-and-reachable`); `packages/d2b-realm-provider/src/error.rs` (`ProviderError`, `ErrorKind`, `RetryHint`, `ProviderDiagnostic` - `implemented-and-reachable`); `packages/d2b-realm-core/src/ids.rs` (`ProviderId`, label-shaped - `implemented-and-reachable`); `packages/d2b-core/src/processes.rs` (`ProcessRole::{CloudHypervisorRunner, Virtiofsd, Swtpm, GpuRenderNode, Audio, Video, QemuMediaRunner, VsockRelay, OtelHostBridge, Usbip}`, `VmProcessDag`, `ProcessNode` - `implemented-and-reachable`); Provider resource schema and API catalog: `ADR-only` |
| Reuse source | main `a1cc0b2d`: any `d2b-provider-toolkit` registry/descriptor patterns named in ADR046-zone-control-017 sub-items |
| Reuse action | adapt |
| Destination | `packages/d2b-contracts/src/v3/provider.rs`; `packages/d2b-core-controller/src/provider_lifecycle.rs`; `packages/d2b-core-controller/src/api_catalog.rs` |
| Detailed design | Provider resource schema with all spec fields from §4.3, including resolved component bounds (max 8 controllers, 8 services, 32 worker templates, 16 ResourceTypes per controller); trust/conformance/config validation; component descriptor validation; dependency alias resolution; API binding with permission intersection; lifecycle policies; Nix Provider installation options; Provider crate layout enforcement per §4.8 (see ADR046-pkg-001) Primary reuse disposition: `adapt`. Preserved source-plan detail: extract and adapt (`workload_lists_and_advertises`/`display_fails_closed_when_unsupported` conformance behavior; `RuntimeCapabilitySet`/`WorkloadCapabilitySet`/`NodeCapabilitySet` → Provider component `supportedCapabilities`; `ProviderError`/`RetryHint` → Provider lifecycle error schema; `ProviderId` → Provider `metadata.name` validator; `ProcessRole` variants → Provider component type identifiers). |
| Integration | Zone config publication installs Provider resources; API catalog handler binds exported ResourceTypes; Provider/system-core and Provider/system-minijail are bootstrap exceptions with pre-created records |
| Data migration | Full reset; Provider packages recompiled and re-registered per new schema |
| Validation | All §15.3 Provider tests including the resolved bounds checks |
| Removal proof | `d2b-realm-provider` trait crate removed per ADR046-zone-control-017 after Provider resource integration |

### ADR046-zone-control-004

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-zone-control-004` |
| Dependency/owner | ADR046-zone-control-001; ADR046-api-002 |
| Current source | `packages/d2bd/src/admission.rs` (`PeerRole::{Admin, Launcher, HostShutdown}`, `PeerIdentity`, `authorize_peer()`, `verb_requires_admin()`, `verb_allowed_for_host_shutdown()` - `implemented-and-reachable`, baseline `b5ddbed6`); `packages/d2b-daemon-access/src/lib.rs` (`LocalUnixAllowlistRole::{Admin, Launcher}`, `DaemonAccessPolicyRole::RealmAdmin`, `DaemonAccessDecision`, `MappedDaemonAccessPrincipal`, `map_local_unix_daemon_access()` - `implemented-and-reachable`); Role resource schema and RBAC index: `ADR-only` |
| Reuse action | adapt |
| Destination | `packages/d2b-contracts/src/v3/role.rs`; `packages/d2b-core-controller/src/authz.rs` |
| Detailed design | Role resource schema with rule schema from §5.3 and resolved bounds (32 rules, 16 resourceTypes per rule, 64 resourceNames, 32 executionRefs); separate closed resource/session verb enums including `relay`; core-generated ZoneLink ownership, exact adjacent-Zone enrollment selector, exact target bounds, and explicit admin-policy exception admission; explicit wildcard enforcement; index builder; phase/conditions; `role-binding-drain` finalizer; generated Nix Role option help; audit/OTEL instrumentation Primary reuse disposition: `adapt`. Preserved source-plan detail: extract and adapt (`verb_requires_admin()` verb table → Role verb enum; `DaemonAccessDecision` error types → Role admission errors; `MappedDaemonAccessPrincipal` → subject identity model). |
| Integration | Authorization evaluator (ADR046-api-002) reads Role index entries; core `authz` handler owns reconcile loop |
| Data migration | Initial Roles generated from Nix config; no v2 Role resource import |
| Validation | All §15.4 Role tests including closed-enum, relay origin/scope/target-verb, and resolved-bounds checks |
| Removal proof | `daemon-access` capability enum removed after RBAC Role engine covers all access decisions |

### ADR046-zone-control-005

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-zone-control-005` |
| Dependency/owner | ADR046-zone-control-004 |
| Current source | `packages/d2b-daemon-access/src/lib.rs` (`DaemonAccessDecision::Authorized { role: DaemonAccessPolicyRole }`, `DaemonAccessAdmissionSource`, `map_remote_daemon_access()` - `implemented-and-reachable`, baseline `b5ddbed6`; these implement the current coarse binding decision per-connection); RoleBinding resource schema: `ADR-only` |
| Reuse action | adapt |
| Destination | `packages/d2b-contracts/src/v3/role_binding.rs`; `packages/d2b-core-controller/src/authz.rs` (shared with Role handler) |
| Detailed design | RoleBinding resource schema with no expiry field and max 128 subjects; subject resolution and UID binding; external principal selector; scope narrowing intersection; revocation; immediate deletion with cache invalidation; Nix RoleBinding options Primary reuse disposition: `adapt`. Preserved source-plan detail: new (RoleBinding resource schema/store table/handler); adapt (`DaemonAccessAdmissionSource` identity fields → subject selector; `map_remote_daemon_access()` logic → subject UID-binding behavior). |
| Integration | Subject resolution uses store owner index; authorization evaluator reads combined Role+narrowing entry; revocation/update/delete flow uses the normal resource lifecycle |
| Data migration | Initial RoleBindings generated from Nix config |
| Validation | All §15.5 RoleBinding tests, including the 128-subject admission bound and no-expiry lifecycle model |
| Removal proof | Not applicable; new type |

### ADR046-zone-control-006

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-zone-control-006` |
| Dependency/owner | ADR046-zone-control-004, ADR046-zone-control-005; Zone runtime |
| Current source | `packages/d2bd/src/admission.rs` (`authorize_peer()`, `verb_requires_admin()`, `verb_allowed_for_host_shutdown()`, `PeerRole::{Admin, Launcher, HostShutdown}` - `implemented-and-reachable`, baseline `b5ddbed6`); `packages/d2b-daemon-access/src/lib.rs` (`LocalUnixAllowlistRole`, `DaemonAccessDecision::Authorized/Denied`, `DaemonAccessAdmissionSource`, `MappedDaemonAccessPrincipal`, `map_local_unix_daemon_access()` - `implemented-and-reachable`); `packages/d2bd/src/lib.rs` (`LoadedRealmControllersConfig`, `LoadedRealmIdentityConfig` startup state - `implemented-and-reachable`); compiled bootstrap constant policy: `ADR-only` |
| Reuse action | adapt |
| Destination | `packages/d2b-resource-api/src/bootstrap_authz.rs`; Zone runtime startup path |
| Detailed design | Compiled bootstrap authorization as described in §9; exact subjects (system-core, system-minijail); closed verb table; non-configurable enforcement; atomic supersession after stored RBAC publishes; out-of-band reset path Primary reuse disposition: `adapt`. Preserved source-plan detail: extract and adapt (`authorize_peer()`/`verb_requires_admin()` verb table → bootstrap constant policy verb table; `PeerRole` two-variant model → system-core/system-minijail bootstrap subjects; `map_local_unix_daemon_access()` SO_PEERCRED mapping → bootstrap subject derivation; `LoadedRealmControllersConfig` startup path → Zone runtime startup bootstrap init sequence). |
| Integration | Resource API authorization layer checks bootstrap policy before stored RBAC; supersession is triggered by first `IndexBuilt=True` event from authorization handler |
| Data migration | Bootstrap is always freshly compiled; no migration |
| Validation | All §15.6 bootstrap tests |
| Removal proof | `daemon-access` bootstrap stub removed after Zone runtime bootstrap authz integrates |

### ADR046-zone-control-007

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-zone-control-007` |
| Dependency/owner | ADR046-identities-002; ADR046-zone-control-001; ADR046-zone-control-002; ADR046-zone-control-003; ADR046-zone-control-004; ADR046-zone-control-005; ADR046-zone-control-009; ADR046-zone-control-010 |
| Current source | `nixos-modules/options-realms.nix` (`providerKind = "^[a-z][a-z0-9-]*$"` label regex matching `LABEL_PATTERN` in `ids.rs`; `providerType` submodule with `enable`/`kind`/`placement`/`freeformType` fields; `d2b.realms.<realm>.providers.*` attrset - `generated-or-eval-contract`, baseline `b5ddbed6`); `nixos-modules/options-realms-workloads.nix` (`d2b.realms.<realm>.workloads.*` submodule shape - `generated-or-eval-contract`); `d2b.zones.*` Nix options: `ADR-only` |
| Reuse action | adapt |
| Destination | `nixos-modules/options-zones.nix`; `nixos-modules/resources-zone-control.nix`; extended `nixos-modules/index.nix` |
| Detailed design | `d2b.zones.<zone>.*` Nix options for Zone/ZoneLink/Provider/Role/RoleBinding/Quota/EmergencyPolicy authoring; compiler-only scalar `parentZone` required on non-root Zones and forbidden on `local-root`; Nix eval-time validation of parent existence, one resolved parent, self/cycle rejection, 16-name ancestry depth, ResourceRefs, separate resource/session verb enums (including session-only `relay`), relay origin/scope restrictions, digest format, subject types, child-local ZoneLink self-name/one-uplink/local transport-ref constraints, and the no-expiry RoleBinding model; generated help carries the relay semantics; canonical JSON serialization; generation-bound resource bundle output; the compiler seals the validated parent map into allocator bootstrap topology without emitting `parentZone` into `Zone.spec` or reciprocal resources Primary reuse disposition: `adapt`. Preserved source-plan detail: adapt (`providerType` submodule → Provider option submodule; label regex `^[a-z][a-z0-9-]*$` retained unchanged; `placement` option → Provider component placement; `d2b.realms.*` option namespace → `d2b.zones.*` option namespace). |
| Integration | Nix compiler → sealed allocator topology for `parentZone`; resource compiler → configuration publication handler → Zone store; bootstrap Provider records auto-generated |
| Data migration | Full reset; Nix realm options (`d2b.realms.*`) remain until purge wave |
| Validation | nix-unit vectors for each Zone control type schema; closed resource/session verb and relay restriction vectors; cross-field constraint tests; rendered JSON contract tests (`make test-drift`) |
| Removal proof | `nixos-modules/options-realms.nix` and related realm options removed only after Zone resource Nix integration is live |

### ADR046-zone-control-008

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-zone-control-008` |
| Dependency/owner | ADR046-zone-control-001 (Zone resource); ADR046-zone-control-003 (Provider/system-core installed) |
| Current source | `d2b-realm-core/src/workload.rs:13,27,83` (`WorkloadProviderKind::UnsafeLocal`, `IsolationPosture::UnsafeLocal`, `WorkloadExecutionPosture` with canonical unsafe-local tuple at lines 206-211: `isolation=unsafe-local`, `environment=systemd-user-manager-ambient`, `executionIdentity=authenticated-requester-uid`, `sessionPersistence=user-manager-lifetime` - `implemented-and-reachable`, baseline `b5ddbed6`); `d2b-core/src/unsafe_local_workloads.rs:16,47,106,150` (`UnsafeLocalWorkloadsJson`, `UnsafeLocalWorkload`, `UnsafeLocalShellPolicy`, `MAX_UNSAFE_LOCAL_SHELL_SESSIONS=64`, `MAX_UNSAFE_LOCAL_WORKLOADS=256` - `implemented-and-reachable`); `d2bd/src/unsafe_local_helper.rs` (`HelperRegistry`, `bind_helper_socket`, `dispatch_launch` - `implemented-and-reachable`, live in `d2bd/src/lib.rs:1346-1468`); `nixos-modules/options-realms.nix:346` (`policy.allowUnsafeLocal` - `generated-or-eval-contract`); `nixos-modules/options-realms-workloads.nix:221,233` (`kind = "unsafe-local"`, doc "no isolation boundary" - `generated-or-eval-contract`) |
| Reuse action | adapt |
| Destination | `packages/d2b-contracts/src/v3/host.rs` (Host resource schema, user-domain variant); `packages/d2b-core-controller/src/host_user.rs` (reconciler owned by Provider/system-core); Nix Host authoring via `d2b.zones.<zone>.resources.<name> = { type = "Host"; spec = { ... }; };` (validated per ResourceTypeSchema; no separate `options-zones-hosts.nix` submodule) |
| Detailed design | Host ResourceType schema for user-domain variant: `spec.defaultDomain=user`, `spec.allowedDomains=[user]`, `spec.defaultUserRef=User/<name>`, `spec.shellPolicy` (adapted from `UnsafeLocalShellPolicy`), `spec.launcherItems` (adapted from `UnsafeLocalLauncherItem`). No-isolation posture recorded in `status.observedPosture`. A dedicated Host admission gate blocks unsafe-local Host creation without opt-in. Mandatory no-isolation warning in Host `status.conditions[0].message` and CLI/UI output for all Host commands. No-isolation posture included in audit record body (`isolation=no-isolation`); it is never emitted as an OTEL metric label value or span attribute. Child processes use standard Process Providers - no Provider resource with name or kind `unsafe-local` is created. Cardinality bounds: max 256 user-domain Hosts per Zone, max 64 launcher items per Host, max 64 shell sessions per Host. Primary reuse disposition: `adapt`. Preserved source-plan detail: adapt (`WorkloadExecutionPosture` unsafe-local posture tuple → Host `status.observedPosture`; posture details in audit record body, not OTEL labels; `UnsafeLocalShellPolicy.{defaultName,maxSessions}` → Host `spec.shellPolicy`; `HelperRegistry.dispatch_launch` → Zone runtime Host process launch broker; `policy.allowUnsafeLocal` → dedicated Host admission gate); new (Host ResourceType user-domain schema with `defaultDomain`, `allowedDomains`, `defaultUserRef`, `shellPolicy`, cardinality bounds). |
| Integration | Provider/system-core bootstrap exception creates Host resources from Zone config publication; Host controller reconciles `defaultUserRef` via User resource lookup; `HelperRegistry` in Zone runtime becomes per-uid launch broker for Host Process launch; Host admission layer enforces the `allowUnsafeLocal` gate before Host creation |
| Data migration | Destructive reset; `unsafe-local-workloads.json` bundle artifact and `HelperRegistry` replaced by Host resource store entries + Process Provider launch |
| Validation | `host-user-domain-no-isolation-warning-required`, `host-user-only-disallows-system-domain`, `host-allowUnsafeLocal-gates-creation`, `host-defaultUserRef-user-type-required`, `host-shell-policy-max-sessions-bound`, `host-launcher-item-max-count-bound`, `host-audit-body-isolation-label-present`, `host-otel-no-posture-label`, `host-cli-no-isolation-warning-present` |
| Removal proof | `d2bd/src/unsafe_local_helper.rs` `HelperRegistry` removed after Zone runtime Host/Process broker integration; `unsafe-local-workloads.json` bundle artifact removed after Host resource store replaces it; `kind = "unsafe-local"` Nix enum value removed in purge wave |


### ADR046-zone-control-009

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-zone-control-009` |
| Dependency/owner | ADR046-zone-control-001; Zone store (ADR046-store-001); Quota handler owner |
| Current source | `packages/d2b-realm-core/src/ids.rs` (`LABEL_PATTERN`, `MAX_ID_LEN` - ResourceName validation for quotaRef); `packages/d2b-core/src/unsafe_local_workloads.rs:16-164` (`MAX_UNSAFE_LOCAL_WORKLOADS=256`, etc. - bound evidence for quota ceiling defaults); no current quota ResourceType exists (`ADR-only`) |
| Reuse source | None |
| Reuse action | create |
| Destination | `packages/d2b-contracts/src/v3/quota.rs`; `packages/d2b-core-controller/src/quota.rs`; `packages/d2b-resource-api/src/quota_gate.rs` |
| Detailed design | Quota resource schema with all spec/status fields from §7; ceiling bounds enforcement at admission (hard policy: reject over-quota with `quota-exceeded`; soft policy: warn); usage index built from resource scan with `quotaRef` field; per-ResourceType ceiling in `perTypeCeilings`; `core.quota-drain` finalizer that blocks Quota deletion until all dependent resources with `spec.quotaRef` pointing to this Quota are reassigned or deleted by authorized owners/operators - the controller never issues spec-updates to clear `quotaRef` on other resources; `dependentCount` status field updated from dependency index; audit event `quota-check` per admission; Nix Quota options per §7.7 |
| Integration | Resource API admission gate (`packages/d2b-resource-api/src/quota_gate.rs`) called for every `create` verb; Zone controller triggers quota reconcile on resource-created/deleted/quotaRef-changed events; quota handler registered in core-controller process |
| Data migration | Full reset; no prior Quota resources exist |
| Validation | `quota-ceiling-hard-reject`, `quota-ceiling-soft-warn`, `quota-ceiling-pertype`, `quota-drain-blocks-on-dependents`, `quota-over-quota-status`, `quota-nix-eval-bounds`, `quota-nix-build-pertype-unknown-type` |
| Removal proof | Additive; no existing code removed |

### ADR046-zone-control-010

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-zone-control-010` |
| Dependency/owner | ADR046-zone-control-001; Zone store (ADR046-store-001); EmergencyPolicy handler owner |
| Current source | `packages/d2bd/src/lib.rs` admission gate (`authorize_peer()` - `implemented-and-reachable`, baseline `b5ddbed6`); `packages/d2b-daemon-access/src/lib.rs` (`DaemonAccessDecision::Denied` - basis for admission rejection pattern); no EmergencyPolicy ResourceType exists (`ADR-only`) |
| Reuse source | None |
| Reuse action | create |
| Destination | `packages/d2b-contracts/src/v3/emergency_policy.rs`; `packages/d2b-core-controller/src/emergency_policy.rs`; `packages/d2b-resource-api/src/emergency_gate.rs` |
| Detailed design | EmergencyPolicy resource schema from §8; union semantics: multiple `enabled=true` policies allowed simultaneously; effective scope = OR of all enabled policies' scope flags; effective `drainDeadlineSeconds` = minimum across enabled policies; scope flag evaluation: `stopNewAdmissions` signal to API admission gate, `disconnectZoneLinks` graceful signal to ZoneLink handler, `stopProviderProcesses` suppresses Process launch and sends stop signal to running Provider component Processes (does NOT set `deletionRequestedAt` on Process resources; reconciliation resumes on deactivation), `drainOngoingOperations` deadline drain; `core.emergency-drain` finalizer for enabled policies; audit events `emergency-policy-activated` / `emergency-policy-deactivated`; `reason` field stored in spec and included in audit record body (never in OTEL metric label values, structured log labels, or status fields); Nix EmergencyPolicy options per §8.7 |
| Integration | API admission gate checks union of enabled EmergencyPolicies before every admitted request; ZoneLink handler subscribes to EmergencyPolicy watch triggers; Provider process lifecycle listens for effective `stopProviderProcesses` and resumes launch on deactivation |
| Data migration | Full reset; no prior EmergencyPolicy resources exist |
| Validation | `emergency-policy-activates-gate`, `emergency-policy-disconnects-zonelinks`, `emergency-policy-union-most-restrictive`, `emergency-policy-multi-enabled-combined-scope`, `emergency-policy-stop-processes-no-delete`, `emergency-policy-deactivation-restores-gate`, `emergency-policy-drain-finalizer`, `emergency-nix-eval-drain-deadline-bound-tightest` |
| Removal proof | Replaces the inline `emergencyDisable` field from the proposed Zone.spec option B; that option was not implemented |

---

All items in this subsection copy or adapt from main commit `a1cc0b2da4a08ca3240a770a972fe4da6f912bef`
(the ADR 0045 in-progress implementation). Main is **not** current pre-ADR45 v3 behavior.
Citations use the prefix `main a1cc0b2d:` to distinguish from baseline `b5ddbed6` citations.
Evidence class for all: `main-reuse-source`.

### ADR046-zone-control-011

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-zone-control-011` |
| Dependency/owner | ADR046-identities-001; ADR046-store-001 |
| Current source | main `a1cc0b2d`: `packages/d2b-session/src/lifecycle.rs` (`SessionPhase::{Established,Disconnected,Reconnecting,Closing,Closed}`, `KeepaliveAction::{None,SendPing,Close}`, `SessionLifecycle::new/phase/on_activity/poll_keepalive/receive_pong/disconnect/begin_reconnect/reconnect_established/close` - lines 9-195); `packages/d2b-session/src/engine.rs` (`SessionEngine<T>`, `SessionEvent`, `establish_initiator/responder`, `reconnect_initiator/responder`, `call/complete_call/cancel_call`, `send_ttrpc/receive`, `open/send/grant_credit/close/reset_named_stream`, `send_attachments`, `drive_keepalive`, `fail_closed`, encode/decode helpers for keepalive/cancel/close/stream-control/attachment - lines 33-995); `packages/d2b-session/src/driver.rs` (`ComponentSessionDriver` trait 18 methods, `SessionDriverHandle`, `DriverQueues` 4 event queues + named-send FIFO, `DRIVER_COMMAND_CAPACITY=128`, `DRIVER_EVENT_CAPACITY=128`, backpressure on enqueue and dequeue, test `cancelled_immediate_receiver_restores_queued_event` - lines 20-718); `packages/d2b-session/src/streams.rs` (`StreamId`, `StreamPhase::{Open,HalfClosedLocal,HalfClosedRemote,Closed,Reset}`, `StreamEvent::{Data,RemoteClosed,Reset}`, `NamedStreamMux::open/reserve_send/grant_send_credit/refund_send_credit/receive_data/close_local/receive_close/reset/remove_terminal/active` - lines 7-237); `packages/d2b-session/src/transport.rs` (`OwnedTransport` trait, `TransportDescriptor`, `TransportPacket`, `TransportError::{Disconnected,Truncated,LimitExceeded,InvalidAttachment,Other}` - lines 9-106); `packages/d2b-session/src/error.rs` (`SessionError`, all `From<>` mappings covering `ContractError`, `BinaryError`, `SequenceError`, `FragmentSequenceError`, `HandshakeRejectReason`, `TransportError` - lines 10-143); supporting modules: `attachment.rs`, `bootstrap.rs`, `cancellation.rs`, `deadline.rs`, `fragmentation.rs`, `metrics.rs`, `record.rs`, `scheduler.rs`, `server.rs` |
| Reuse action | adapt |
| Destination | `packages/d2b-bus/src/{lifecycle,engine,driver,streams,transport,error}.rs` (new crate `d2b-bus`) |
| Detailed design | **Selected**: full `SessionLifecycle` FSM including keepalive ping-nonce/timeout close (`poll_keepalive` lines 81-124), nonce-exhaustion close (`NonceExhausted` remediation `ReplaceGeneration`), reconnect attempt counting + window expiry (`begin_reconnect` lines 147-174), generation increment on reconnect; `ComponentSessionDriver` 18-method trait as the d2b-bus driver contract; `SessionEngine` full frame encode/decode, keepalive, named-stream credit/half-close/reset; `NamedStreamMux` credit model with send/receive credit, phase transitions, terminal removal; `DriverQueues` backpressure (`QueueBackpressure`) on both deliver and receive paths; all `SessionError`/`From<>` mapping chains. **Excluded ADR45 assumptions**: `establish_initiator_with_generation_discovery()` (lines 102-123): ADR45 initiator-probes-server for current generation; in v3 the generation lives in the Zone resource store - use `establish_initiator()` directly. OTEL labels in `metrics.rs` reference ADR45 realm/workload dimensions - replace with Zone/ZoneLink labels per §13.2. Primary reuse disposition: `adapt`. Preserved source-plan detail: copy + adapt. |
| Integration | `d2b-bus` crate is the sole session-transport dependency for ZoneLink (ADR046-zone-control-018), Provider component Processes (ADR046-zone-control-017), and the Zone resource API service layer (ADR046-api-001); `ComponentSessionDriver` is the interface ZoneLink uses to send/receive ttrpc frames and named streams |
| Data migration | Not applicable; new crate |
| Validation | `session-lifecycle-reconnect-attempts-exhausted`, `session-lifecycle-keepalive-timeout-closes`, `session-lifecycle-nonce-exhausted-close-with-replace-generation-remediation`, `session-driver-cancelled-receiver-restores-event` (ported from test at line 707), `named-stream-credit-half-close-then-remote-close-transitions-closed`, `named-stream-reset-cancels-pending-send` |
| Removal proof | `d2b-realm-router/src/{session.rs,secure_session.rs,mux_session.rs,session_lifecycle.rs}` removed after d2b-bus integration wave |

### ADR046-zone-control-012

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-zone-control-012` |
| Dependency/owner | ADR046-zone-control-011 |
| Current source | main `a1cc0b2d`: `packages/d2b-session-unix/src/adapter.rs` (`UnixSeqpacketTransport::new`, `UnixStreamTransport::new`, `PeerIdentityPolicy::{Pathname{verifier},InheritedSocketpair{expected_peer}}`, `OwnedUnixAttachment::file`, `UnixAttachmentPayload`, `consume_peer_credentials()` credential strip+verify, `map_validation_error`/`map_transport_error`, tests `inherited_first_packet_requires_credentials`/`inherited_first_packet_rejects_wrong_credentials` - lines 23-852); `packages/d2b-session-unix/src/socket.rs` (`AncillaryCapacity::from_policy`, `OutboundPacket::new`, `SeqpacketSocket`, `prearmed_seqpacket_pair()`, `SendBurst`/`PacketBurst`/`SentPacket`, `recv_one` with `DONTWAIT|CMSG_CLOEXEC`, `send_one` with `DONTWAIT|NOSIGNAL`, SCM_RIGHTS+SCM_CREDENTIALS, fairness-budget burst loops, tests `raw_control_scanner_rejects_unknown_and_partial_headers`/`raw_control_scanner_accepts_exact_rights_shape` - lines 35-698); `packages/d2b-session-unix/src/pidfd.rs` (`PidfdEvidence::new`, `ProcPidfdIdentityVerifier::verify` with double-read race guard, `ProcSelfFdInfoSource::read_pid_from_fdinfo`, `DigestEvidenceCallback` - lines 8-159); `packages/d2b-session-unix/src/credit.rs` (`CreditPool::new/reserve`, `CreditScope::{Packet,Request,Operation,Session,Process,Host}`, `CreditScopeSet::reserve/reserve_ingress`, `CreditBundle::acquire_dispatch`, `ProcessCreditLimit::derive` from `RLIMIT_NOFILE` - lines 13-286); `packages/d2b-session-unix/src/descriptor.rs` (`ReceivedPacket::verify/verify_first_packet_credentials`, `DescriptorPolicy`, `ObjectIdentity`, `PeerCredentials`, `FirstPacketCredentials`, sealed memfd `F_SEAL_WRITE|GROW|SHRINK|SEAL` - lines 21-617); `packages/d2b-session-unix/src/error.rs` (`UnixSessionError` 21 variants, Display strings - lines 4-92); `packages/d2b-session-unix/src/systemd.rs` (`ActivatedSeqpacketListener`, `SystemdActivationError`) |
| Reuse action | adapt |
| Destination | `packages/d2b-bus-unix/src/{adapter,socket,pidfd,credit,descriptor,error,systemd}.rs` (new crate `d2b-bus-unix`) |
| Detailed design | **Selected**: `UnixSeqpacketTransport` seqpacket adapter implementing `OwnedTransport`; `PeerIdentityPolicy` SO_PEERCRED verification (pathname + inherited socketpair); `prearmed_seqpacket_pair()` for bootstrap inherited-socketpair path; `CreditPool`/`CreditScopeSet` six-scope credit model with per-packet/request/operation/session/process/host reservation; `ProcessCreditLimit::derive()` from `RLIMIT_NOFILE` minus `RESERVED_CONTROL_FDS`; `ProcPidfdIdentityVerifier` double-read race guard on `/proc/self/fdinfo/<fd>` with executable + cgroup digest callbacks; sealed memfd four-seal enforcement; `AncillaryCapacity` derived from `AttachmentPolicy`; `UnixSessionError` 21-variant Display strings preserved verbatim for audit log compatibility; `consume_peer_credentials()` strips-and-verifies SCM_CREDENTIALS exactly once on first packet. **Excluded ADR45 assumptions**: `vsock.rs` (`FramedVsockTransport`, `NativeVsockListener`, `NativeVsockTransport`): ADR45 guest-control vsock transport; v3 ZoneLink uses Unix sockets for host-local; vsock for Guest connections is separate work. Specific socket paths (`PUBLIC_SOCKET_PATH`, `BROKER_SOCKET_PATH`) are replaced by Zone-resource-managed paths. Primary reuse disposition: `adapt`. Preserved source-plan detail: copy + adapt. |
| Integration | `d2b-bus-unix` provides `OwnedTransport` impl consumed by `d2b-bus` `SessionEngine`; Zone runtime passes accepted seqpacket socket to `UnixSeqpacketTransport::new`; Process pidfd identity verifier integrates with Host/Process broker launch |
| Data migration | Not applicable; new crate |
| Validation | `seqpacket-inherited-missing-credentials-rejected` (ported from `inherited_first_packet_requires_credentials`), `seqpacket-inherited-wrong-credentials-rejected` (ported from `inherited_first_packet_rejects_wrong_credentials`), `pidfd-double-read-race-guard-detects-pid-reuse`, `sealed-memfd-partial-seal-rejected`, `credit-scope-six-levels-ordered`, `ancillary-capacity-from-disabled-policy-is-zero`, `raw-control-unknown-header-rejected` (ported from `raw_control_scanner_rejects_unknown_and_partial_headers`), `raw-control-valid-rights-parsed` (ported from `raw_control_scanner_accepts_exact_rights_shape`) |
| Removal proof | `d2b-realm-router/src/secure_session.rs` `SecurePeerSession` seqpacket layer removed after d2b-bus-unix provides the replacement |

### ADR046-zone-control-013

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-zone-control-013` |
| Dependency/owner | ADR046-identities-001 |
| Current source | main `a1cc0b2d`: `packages/d2b-contracts/src/v2_component_session.rs` (all protocol constants: `PREFACE_LEN=16`, `PREFACE_MAGIC=b"D2BCS2\r\n"`, `COMPONENT_SESSION_MAJOR=2`, `COMPONENT_SESSION_MINOR=0`, `MAX_HANDSHAKE_OFFER_BYTES=16384`, `HANDSHAKE_OFFER_CANONICAL_LEN=148`, `ENDPOINT_POLICY_IDENTITY_CANONICAL_LEN=140`, `MAX_PROTECTED_CIPHERTEXT_BYTES=65535`, `NOISE_TAG_BYTES=16`, `RECORD_LENGTH_BYTES=2`, `MAX_PROTECTED_PLAINTEXT_BYTES=65519`, `MAX_LOGICAL_MESSAGE_BYTES=1_048_576`, `MAX_ACTIVE_NAMED_STREAMS=128`, `MAX_PACKET_ATTACHMENTS=32`, `MAX_REQUEST_ATTACHMENTS=64`, `MAX_OPERATION_ATTACHMENTS=128`, `MAX_SESSION_ATTACHMENTS=256`, `MAX_PROCESS_ATTACHMENT_CREDITS=2048`, `MAX_HOST_ATTACHMENT_CREDITS=8192`, named-stream/control queue limits, credential size/flag constants; closed enums: `EndpointPurpose` 19 tags, `PurposeClass` 3 tags, `EndpointRole` 19 tags, `ServicePackage` 15 tags, `NoiseProfile` 3 tags, `IdentityEvidenceRequirement` 3 tags, `Locality` 4 tags, `TransportClass` 7 tags, `AttachmentPolicyKind`; structs: `ComponentSessionPreface::parse/encode`, `HandshakeOffer` 11 fields, `HandshakeAccept`, `HandshakeReject`, `HandshakeRejectReason` 21 tags, `LimitProfile` 15 fields with `local_default()`, `AttachmentPolicy::validate/disabled`, `BoundedVec<T,MIN,MAX>` - throughout); `packages/d2b-session/src/handshake.rs` (`HandshakeRole::{Initiator,Responder}`, `HandshakeCredentials::{Nn,Kk,IkPsk2Initiator,IkPsk2Responder}`, `NegotiatedOffer`, `NoiseHandshake::new/write_next/read_next/finish`, `EstablishedHandshake::transcript_hash/generation`, `encode_offer/negotiate_offer`, generation-discovery magics `b"D2BGD2Q\n"`/`b"D2BGD2A\n"`, `encode_generation_discovery_request/accept/response/decode_generation_discovery_response` - lines 39-433) |
| Reuse action | adapt |
| Destination | `packages/d2b-contracts/src/v3/component_session.rs` (new v3 namespace in existing contracts crate) |
| Detailed design | **Selected**: all numeric constants verbatim; all 21 `HandshakeRejectReason` tags; `ComponentSessionPreface::parse/encode` with exact magic/length layout; `HandshakeOffer` 11-field shape; `AttachmentPolicy::validate` transport constraints (packet-atomic requires seqpacket or inherited-socketpair); `LimitProfile` 15-field profile; `NoiseHandshake` for Nn/Kk/IkPsk2 profiles; `EstablishedHandshake::transcript_hash`; `BoundedVec<T,MIN,MAX>`. **Excluded ADR45 assumptions**: `EndpointPurpose` 19 tags and `ServicePackage` 15 tags encode ADR45-specific service families - v3 will append new tags for Zone API endpoints without renumbering existing ones. `IdentityEvidenceRequirement` + `GuestSessionCredentialV1` / `GuestBootstrapPsk` / `GUEST_SESSION_CREDENTIAL_MAGIC`: ADR45 guest bootstrap credential formats, excluded until v3 Guest bootstrap work item. Generation-discovery protocol (`encode_generation_discovery_request` lines 138-149): ADR45 initiator probes server for current generation; v3 generation lives in Zone resource store - generation-discovery excluded from initial d2b-bus copy. Primary reuse disposition: `adapt`. Preserved source-plan detail: copy + adapt. |
| Integration | All d2b-bus, d2b-bus-unix, Provider registry, and client packages import v3 protocol constants; v3 Zone API service layer uses `EndpointPurpose`/`ServicePackage` tags for Zone API endpoints |
| Data migration | Additive: new v3 tags appended to existing closed enums; ADR45 v2 tag values remain valid during coexistence |
| Validation | `preface-magic-exact-16-bytes`, `preface-offer-len-zero-rejected`, `preface-offer-len-over-max-rejected`, `preface-offer-len-canonical-accepted`, `handshake-21-reject-reasons-all-covered`, `attachment-policy-packet-atomic-requires-seqpacket-or-inherited`, `limit-profile-local-default-all-fields-positive` |
| Removal proof | `d2b-contracts/src/v2_component_session.rs` ADR45-specific tags retired after all v2 sessions decommissioned |

### ADR046-zone-control-018

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-zone-control-018` |
| Dependency/owner | ADR046-zone-control-011, ADR046-zone-control-012, ADR046-zone-control-013; ZoneLink session/admission foundation owner |
| Current source | main `a1cc0b2d`: `packages/d2b-session/src/lifecycle.rs` (`SessionPhase` 5-phase FSM, `begin_reconnect` window/attempt bounds - lines 9-195); `packages/d2b-realm-router/src/service_v2.rs` (`RealmSessionAuthority::local_controller/gateway_peer`, authority validation rejecting invalid combinations - lines 53-123; `RealmServiceServer::call` wire/request/generation/lifetime/attachment validation - lines 415-497; `RealmServiceLimits` 15 fields - lines 147-181; `RealmAuditEvent`/`RealmAuditOutcome` - lines 236-245); `packages/d2b-realm-router/src/session_lifecycle.rs` (`SessionPhase::{Allocating,TokenMinting,RelayConnecting,DisplayOpening,Running,Stopping,Stopped}`, `fail`/`stop`/`finish_stop`, tests `forward_sequence_reaches_running_then_refuses_to_advance`, `failure_mid_establishment_rolls_into_teardown_and_records_phase`, `stop_is_idempotent`, `finish_stop_without_stopping_is_a_no_op` - lines 31-220); `packages/d2b-client/src/session.rs` (`ComponentSessionConnector` trait - lines 79-86; `NamedStream` lifecycle/close/reset/send/receive - lines 426-576); `packages/d2b-daemon-access/src/component_session.rs` (`connect_component_session()` peer-UID verify + `HandshakeCredentials::Nn` + `TransportKind::LocalUnix` - lines 53-85) |
| Reuse action | adapt |
| Destination | `packages/d2b-core-controller/src/zone_link.rs` (ZoneLink handler); `packages/d2b-resource-api/src/admission.rs` (request admission) |
| Detailed design | **Selected**: `SessionPhase` 5-phase FSM (from main `d2b-session`) drives ZoneLink session state; `Established` state → `status.connected=true` + ZoneLink `status.phase=Ready`; `Disconnected`/`Reconnecting` states → `status.connected=false` + ZoneLink `status.phase=Pending` (or `Degraded` if degraded capability); session-internal phases are not exposed as `ZoneLink.status.phase` values - only the common Resource phases (`Pending|Ready|Degraded|Failed|Unknown`) appear in `status.phase` (§3.5); `begin_reconnect` window/attempt logic → ZoneLink reconnect loop; `RealmSessionAuthority` local vs gateway pattern → ZoneLink authority types for host-local vs transport-bridge sessions; `RealmServiceServer::call` wire validation (generation, request lifetime, attachment) → Zone API request admission; `RealmServiceLimits` 15 fields → ZoneLink `spec.limits`; `connect_component_session()` Nn peer-UID path → Zone runtime bootstrap ComponentSession; `NamedStream` lifecycle → ZoneLink named-stream operations; session-lifecycle tests ported as ZoneLink phase regression tests. **Excluded ADR45 assumptions**: `RealmSessionAuthority::gateway_peer()` (lines 72-87): gateway custody and `Locality::Remote` + `CredentialCustody::GatewayGuest` are ADR45 realm-gateway patterns; v3 ZoneLink transport is bound by the resolved `spec.transportProviderRef`, `spec.transportSettings`, and `spec.transportCredentials` contract instead. Realm 7-phase `SessionLifecycle` (`Allocating→…→Running→Stopping→Stopped`) is the ADR45 realm-specific lifecycle; ZoneLink uses the 5-phase d2b-session model. `GuestBootstrapPsk`/`GuestSessionCredentialV1`: ADR45 guest bootstrap, excluded. `realm_stubs.rs` `ApiService`/`ApiFrontend` dead code excluded (§16.2). |
| Integration | Zone runtime startup creates bootstrap ComponentSession using `HandshakeCredentials::Nn` + local domain socket + peer-UID verification (adapted from `connect_component_session`); the child-local ZoneLink handler opens its allocator-bound uplink session using d2b-bus `SessionEngine`, and the parent routes child calls over that session; resource API admission validates requests using the `RealmServiceServer::call` pattern |
| Data migration | Not applicable; new implementation |
| Validation | `zonelink-reconnect-child-uid-change`, `zonelink-disconnect-unknown-phase`, `zonelink-intent-queue-limit`, `zone-session-phase-forward-sequence-refuses-repeat` (ported from `forward_sequence_reaches_running_then_refuses_to_advance`), `zone-session-failure-records-phase` (ported from `failure_mid_establishment_rolls_into_teardown_and_records_phase`), `zone-session-stop-is-idempotent` (ported), `zone-bootstrap-session-nn-peer-uid-verified` |
| Removal proof | `d2b-realm-router/src/service_v2.rs` `RealmServiceServer` removed after Zone API service layer replaces realm v2 service; `d2b-realm-router/src/session_lifecycle.rs` removed after d2b-bus lifecycle replaces realm-specific one |

### ADR046-zone-control-017

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-zone-control-017` |
| Dependency/owner | ADR046-zone-control-011, ADR046-zone-control-013; Provider registry/toolkit foundation owner |
| Current source | main `a1cc0b2d`: `packages/d2b-provider/src/registry.rs` (`RegistryLimits`, `AdmissionOptions`, `ProviderRegistryBuilder::new/limits/register_factory/register_instance/register_constructed/finish`, `ProviderRegistry::lifecycle/snapshot/instance/admit/shutdown`, `InFlightPermit`, `AdmittedProvider`, `ProviderRegistryManager::new/current/publish` - lines 33-568; tests `shutdown_closes_final_permit_notify_race`, `finish_drain_closes_final_permit_notify_race` - lines 683-691); `packages/d2b-provider/src/rpc.rs` (`SessionIdentity`, `ProviderClock`/`SystemProviderClock`, `RpcCall`, `RpcPayload`, `RpcResponse`, `AuthenticatedProviderRpc`, `RpcProviderProxy::new/preflight/call*`, `session_identity_matches_placement` - lines 27-895; test `provider_and_user_agent_session_identities_are_placement_exact` - lines 923-959); `packages/d2b-provider-toolkit/src/adapter.rs` (`ProviderAgentAdapter::new/invoke_session/invoke` - lines 18-194); `packages/d2b-provider-toolkit/src/conformance.rs` (`ConformanceError`, `check_descriptor_conformance`, `check_provider_conformance` - lines 8-126); `packages/d2b-provider-toolkit/src/fixture.rs` (`DeterministicClock`, `Fixture::new/from_descriptor/operation/request/call_context/session_identity`, `FakeProvider`, `sample_lease_request` - lines 39-262); `packages/d2b-provider-toolkit/src/registration.rs` (`ToolkitError`, `register_exact_instances` - lines 12-107); `packages/d2b-provider-toolkit/src/server.rs` (`GeneratedProviderServiceServer::from_session_handle/new/shutdown/generated_services` - lines 59-176; test `rpc_statuses_retain_closed_actionable_reasons`); `packages/d2b-provider-toolkit/src/values.rs` (`ProviderValues::new/descriptor/health/plan/handle_from_request/handle_from_plan/observation` - lines 18-192); `packages/d2b-provider-toolkit/src/redaction.rs` (`Redacted<T>`, `Secret<T>` - lines 3-37) |
| Reuse action | adapt |
| Destination | `packages/d2b-provider/src/{registry,rpc}.rs` (new v3 Provider package); `packages/d2b-provider-toolkit/src/{adapter,conformance,fixture,registration,server,values,redaction}.rs` (new v3 toolkit) |
| Detailed design | **Selected**: `ProviderRegistry` lifecycle `Accepting→Draining→Retired` with drain-waiter notify-race safety (ported from `shutdown_closes_final_permit_notify_race` / `finish_drain_closes_final_permit_notify_race` tests); `InFlightPermit` + global + per-provider in-flight quota; `ProviderRegistryManager::publish` validates snapshot before swap; `RpcProviderProxy::preflight` cancellation-check, deadline-check, method/capability match, session-identity/placement exactness; `ProviderAgentAdapter` rejects attachments with missing descriptors or non-increasing indexes (lines 79-99); `GeneratedProviderServiceServer::new` single-service-per-agent requirement + shutdown via atomic accept-flag + idle notify + timeout; `check_provider_conformance` health/inspection/observability check sequence; `Fixture`/`FakeProvider`/`DeterministicClock` as conformance harness; `Redacted<T>`/`Secret<T>` retained unchanged. **Excluded ADR45 assumptions**: `TrustedFirstPartyInProcess` as the only accepted placement (in `session_identity_matches_placement` lines 577-598): v3 Provider resources support multiple placements per `RealmControllerPlacement` mapping (§16.2); in-process placement is retained but not exclusive. Generated ttrpc stubs in `d2b-contracts/src/generated_v2_services/` are v2-service-specific; v3 Provider processes compile their own stubs from v3 proto files. ACA workload adapter (`d2b-gateway-runtime/src/aca_workload.rs`) excluded. Primary reuse disposition: `adapt`. Preserved source-plan detail: copy + adapt. |
| Integration | Provider/system-core and Provider/system-minijail use `ProviderRegistryBuilder::register_constructed` as bootstrap exceptions; other Providers register instances from Process-spawned agents via `ProviderAgentAdapter`; Zone runtime hosts one `ProviderRegistry` per installed Provider component; `ProviderRegistryManager::publish` swaps registry on Provider update |
| Data migration | Provider descriptors re-registered on Zone store bootstrap; no v2 provider state migration |
| Validation | `provider-registry-drain-waiter-race-safe` (ported from both notify-race tests), `provider-registry-publish-validates-snapshot-before-swap`, `provider-rpc-proxy-placement-exact` (ported from `provider_and_user_agent_session_identities_are_placement_exact`), `provider-agent-adapter-rejects-non-monotone-attachment-indexes`, `provider-server-shutdown-drains-in-flight-requests`, `provider-conformance-health-inspection-observability-sequence` |
| Removal proof | `d2b-realm-provider` trait crate removed after v3 Provider resource + registry integration (§16.5) |

### ADR046-provider-agent-001

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-provider-agent-001` |
| Dependency/owner | ADR046-zone-control-011, ADR046-zone-control-017, ADR046-zone-control-018 |
| Current source | main `a1cc0b2d`: `packages/d2b-gateway-runtime/src/provider_agent.rs` (`ProviderAgentError::{UnregisteredAdapter,RegistryNotAccepting,RegistrationRejected,InvalidAuditCapacity,SessionClosed,ProtocolViolation}`, `ProviderAgentAuditOutcome`, `ProviderAgentAuditEvent`, `ProviderAgentProcess::from_registry/from_registry_with/provider_type/service_names/audit_snapshot/serve`, `run_registered`, bounded in-memory audit ring, frame dispatch loop: semaphore in-flight limit, service/method routing, negative-timeout guard, `SessionClosed` termination, `ProtocolViolation` audit + terminate - lines 31-452; tests `standalone_entrypoint_fails_without_registration`, `audit_capacity_is_bounded` - lines 454-486); `packages/d2b-contracts/src/provider_registry_v2.rs` (`ProviderRegistryV2` wire contract, `ProviderRegistryEntryV2::validate` with provider-id derivation rule, schema fingerprint, scope-digest, generation exactness, `TrustedFirstPartyInProcess` placement requirement, `ProviderIntentId` label rules `max 128 bytes`, `MAX_PROVIDER_REGISTRY_ENTRIES`, `MAX_PROVIDER_MAPPING_IDS=64`, `ProviderBindingV2` non-exhaustive + `UnsupportedProviderBindingV2` fallback, `ProviderRegistryV2::validate` sort/unique/count checks - lines 23-566; tests `validates_closed_local_runtime_mapping`, `validates_closed_local_observability_mapping`, `serializes_declared_mapping_axes_as_closed_variants`, `rejects_duplicate_or_unbounded_mapping_ids`, `local_storage_binding_realm_must_match_descriptor_placement`, `rejects_generation_and_exact_identity_mismatches`, `contradictory_binding_realm_json_is_unrepresentable`, `unknown_binding_axis_remains_rejected_on_the_wire`, `identity_mismatch_messages_name_the_failed_contract`, `accepts_explicit_empty_registry` - lines 722-1044) |
| Reuse action | adapt |
| Destination | `packages/d2b-provider/src/agent.rs` (v3 provider agent dispatch); `packages/d2b-contracts/src/v3/provider_registry.rs` (v3 provider registry wire contract) |
| Detailed design | **Selected**: `ProviderAgentProcess::serve` dispatch loop with semaphore in-flight limit; unsupported-service/method → ttrpc error; negative-timeout rejection; `SessionClosed` clean termination; `ProtocolViolation` audit-and-terminate path; bounded audit ring; `GeneratedProviderServiceServer` single-service-per-agent requirement; `ProviderRegistryV2` entry validation: provider-id derivation, schema fingerprint, scope digest, generation exactness; `MAX_PROVIDER_MAPPING_IDS=64` → Provider component mapping bound; `ProviderBindingV2` non-exhaustive + explicit `UnsupportedProviderBindingV2` fallback (never panics on unknown axis); `ProviderIntentId` `max 128 bytes` label rules → Provider component `spec.intentRef`; all 10 `provider_registry_v2.rs` tests ported. **Excluded ADR45 assumptions**: `aca_workload.rs` (`AcaGatewayWorkload`): ADR45 ACA external provider adapter, excluded entirely. `waypipe_display.rs` (`WaypipeDisplayProvider`): ADR45 display provider, excluded. `ProviderRegistryV2.registry_generation` / `configuration_fingerprint` bind to ADR45 bundle generation; v3 Provider resource version is tracked in redb store, not a JSON bundle. `run()` binary entrypoint uses the fixed `d2b-provider-agent` command; v3 provider processes use normal Zone runtime Process launch. `TrustedFirstPartyInProcess` is the only placement in v2; v3 Provider resources extend to `HostLocal`/`GatewayVm` etc. Primary reuse disposition: `adapt`. Preserved source-plan detail: copy + adapt. |
| Integration | Zone runtime spawns each Provider component Process via normal Process launch; Process binary calls `ProviderAgentProcess::from_registry` then `serve()` on established ComponentSession; on `SessionClosed` the process exits and Zone runtime observes `status.phase` transition |
| Data migration | Not applicable; new implementation |
| Validation | `provider-agent-dispatch-unsupported-service-returns-ttrpc-error`, `provider-agent-negative-timeout-rejected`, `provider-agent-session-closed-terminates-serve-loop`, `provider-agent-audit-ring-capacity-bounded` (ported from `audit_capacity_is_bounded`), `provider-registry-entry-fingerprint-generation-exact` (ported from `rejects_generation_and_exact_identity_mismatches`), `provider-registry-unknown-axis-fallback-non-exhaustive` (ported from `unknown_binding_axis_remains_rejected_on_the_wire`), `provider-registry-duplicate-ids-rejected` (ported from `rejects_duplicate_or_unbounded_mapping_ids`) |
| Removal proof | Not applicable; new implementation |

### ADR046-client-001

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-client-001` |
| Dependency/owner | ADR046-zone-control-011, ADR046-zone-control-012, ADR046-zone-control-013, ADR046-zone-control-018 |
| Current source | main `a1cc0b2d`: `packages/d2b-client/src/client.rs` (`WallClock`, `MetadataInput`, `RetryPolicy` 1..8 attempts, `CallOptions`, `CancellationToken`, `Client::new/with_clock/connect`, `ConnectedClient` methods incl. `session_generation/session_limits/service/invoke/invoke_with_attachments/named_stream/open_server_stream`, `prepare_typed_request/prepare_operation_context`, `can_retry/retryable_failure/validate_outbound_attachments/validate_reply/service_package/map_remote_kind/map_retry` - lines 35-921); `packages/d2b-client/src/session.rs` (`ComponentSessionConnector` trait, `NamedStream` lifecycle, `StreamDispatcher`, `SharedDriver`, aggregate-queue-bound test - lines 24-626); `packages/d2b-client/src/target.rs` (`ServiceOwner`, `TargetInput`, `TransportKind`, `RouteRecord`, `ResolvedTarget`, `TargetResolver`, `RouteTable` - lines 7-228); `packages/d2b-client/src/service.rs` (`ServiceKind`, `GeneratedClient`, `MethodHandle`, `ServiceHandle::new/kind/generated/proxy/method/invoke` - lines 21-184); `packages/d2b-client/src/daemon_service.rs` (`DaemonClient::new/session_generation/connected/resolve/inspect/lifecycle/open_terminal`, `DaemonTerminal`, `daemon_call_options`, `ensure_daemon_outcome`, `map_ttrpc_error`, test `redacted_terminal_debug_payload` - lines 29-689); `packages/d2b-client/src/host_socket.rs` (`HostSocketConnector::new/from_seqpacket_fd`, `local_daemon_endpoint_identity`, `ComponentSessionConnector::connect` - lines 252-383); `packages/d2b-client/src/error.rs` (`RemoteErrorKind`, `RetryClass`, `ClientError` - lines 5-128) |
| Reuse action | adapt |
| Destination | `packages/d2b-client/src/` (updated for v3 Zone API, replacing ADR45 daemon verbs with Zone resource operations) |
| Detailed design | **Selected**: `Client::connect()` target-resolve → ComponentSession-open → `ConnectedClient` lifecycle; `RetryPolicy` 1..8 bound + `retryable_failure()` safe-only retry detection; `NamedStream::send/receive/close_local/reset` lifecycle; `ComponentSessionConnector` trait as connector abstraction; `HostSocketConnector::from_seqpacket_fd` + `local_daemon_endpoint_identity` for Zone runtime socket connector; `RouteTable` ambiguous-route rejection; `ServiceHandle`/`GeneratedClient`/`MethodHandle` typed service client pattern; `map_ttrpc_error`/`validate_reply`/`map_retry` error-handling chain; `ClientError`/`RemoteErrorKind`/`RetryClass` error taxonomy; `DaemonClient` call-options and outcome helpers (infrastructure only). **Excluded ADR45 assumptions**: `DaemonMethod` enum (lines 29-56 of daemon_service.rs): ADR45 daemon verbs (`vm_start`, `vm_stop`, `list_realms`, etc.) - replaced with Zone API verbs. `GuestClient`/`guest_service.rs`: ADR45 guest operations; excluded until v3 Guest transport work item. Hardcoded socket path `PUBLIC_SOCKET_PATH = "/run/d2b/public.sock"` in `host_socket.rs`: replaced by Zone-resource-managed path. `TransportKind::LocalUnix` restriction in daemon-access: v3 allows multiple transport kinds per ZoneLink binding. Primary reuse disposition: `adapt`. Preserved source-plan detail: copy + adapt. |
| Integration | `d2b` CLI uses `d2b-client` to connect to Zone runtime via `HostSocketConnector`; a child Zone runtime uses `d2b-client` for its allocator-bound uplink while the parent route engine uses the established session for child calls; Provider toolkit conformance tests use `Fixture`/`FakeProvider` with `d2b-client` service handles |
| Data migration | Not applicable; updated in place |
| Validation | `client-retry-policy-max-8-attempts-enforced`, `client-named-stream-close-local-then-remote-close-transitions-closed`, `client-route-table-ambiguous-route-rejected`, `client-host-socket-peer-uid-verified-on-connect`, `client-retryable-failure-only-safe-mutations` |
| Removal proof | `DaemonMethod` v2 verb enum retired after all v2 daemon operations migrated to Zone API |

### ADR046-wire-001

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-wire-001` |
| Dependency/owner | ADR046-zone-control-013 |
| Current source | main `a1cc0b2d`: `packages/d2b-contracts/src/v2_services.rs` (`MethodSpec{mutating,stream_kind,...}`, `ServiceSpec`, `SERVICE_INVENTORY` covering 20+ services and all provider services, `service_schema_fingerprint`, `public_daemon_schema_fingerprint`, `direct_guest_schema_fingerprint`, `StrictWireMessage`, `decode_strict`, `encode_strict`, `admit_metadata`, `TerminalStreamValidator`, `ServerStreamLease`, `RedactedTerminalFrame`, stream-name validators - lines 204-1004; tests `public_endpoint_fingerprint_binds_both_services_dependencies_and_order`, `public_endpoint_fingerprint_binds_daemon_and_guest_method_descriptors`, `direct_guest_fingerprint_binds_activation_and_remains_separate_from_public_endpoint` - lines 872-1030); `packages/d2b-contracts/src/v2_state.rs` (constants: `STATE_SCHEMA_VERSION=2`, `STATE_SCHEMA_GENERATION=1`, `MAX_JSON_DOCUMENT_BYTES=1_048_576`, `MAX_INVENTORY_ROWS=4096`, `MAX_LOCKS=1024`, `MAX_LOCK_DEPENDENCIES=32`, `MAX_DISCOVERY_OBSERVATIONS=4096`, `MAX_AUDIT_RECORD_BYTES=8192`, `MAX_AUDIT_RECORDS_PER_SEGMENT=16384`, `MAX_AUDIT_SEGMENT_BYTES=64*1024*1024`, `MAX_AUDIT_RETENTION_DAYS=14`, `MAX_LOCK_DEADLINE_MS=300_000`; types: `Digest`, `Generation`, `OwnershipEpoch`, `SafeJsonInteger`, `StorageCategory`, `StateEnvelope<T>`, `CanonicalPayloadVerifier<T>`, `AtomicWritePhase`, `RunnerEvidence`, audit types incl. `AuditRecord/Segment/Checkpoint/Gap`, `detect_audit_gap`, `AuditRetentionPolicy`, lock types: `LockClass`, `LockSpec`, `SyncInventory`, `LeaseRecord`); `packages/d2b-contracts/src/v2_identity.rs` (`IdentityError` 13 variants, canonical name rules `^[a-z][a-z0-9-]*$` max 63 bytes start-lowercase-letter, `RealmPath` label/separator rules, `ProviderType::ALL` 11 types + `as_str()` - lines 11-250); `packages/d2b-contracts/src/v2_provider.rs` (bounded opaque IDs `[a-z][a-z0-9-]{0,63}`: `ImplementationId`, `OperationId`, `IdempotencyKey`, `PlanId`, `HandleId`, `LeaseId`, `TransferId`, `PROVIDER_CONTRACT_FINGERPRINT`, `ProviderContractError` 34 variants, `Fingerprint` 64 lowercase-hex chars - lines 18-219); `packages/d2b-contracts/src/generated_v2_services/` (all 40+ generated ttrpc client/server stubs for v2 services) |
| Reuse action | adapt |
| Destination | `packages/d2b-contracts/src/v3/{services,state,identity,provider}.rs` (new v3 namespace); generated stubs regenerated from v3 proto files in `packages/d2b-contracts/proto/v3/` |
| Detailed design | **Selected**: `MethodSpec`/`ServiceSpec`/fingerprinting infrastructure → v3 Zone API service schema fingerprinting; `StrictWireMessage`/`decode_strict`/`encode_strict`/`admit_metadata` → v3 wire decode/admit for all resource API requests; `CanonicalPayloadVerifier<T>` payload-digest binding → v3 resource store integrity checks; audit chain types `AuditRecord`/`AuditSegment`/`AuditCheckpoint`/`detect_audit_gap` + `MAX_AUDIT_RETENTION_DAYS=14`/`MAX_AUDIT_RECORD_BYTES=8192`/`MAX_AUDIT_SEGMENT_BYTES=64MiB` → v3 Zone audit (§13.3); lock types `LockSpec`/`SyncInventory`/`LeaseRecord` + `MAX_LOCKS=1024`/`MAX_LOCK_DEADLINE_MS=300_000` → v3 resource store lock layer; bounded opaque ID pattern `[a-z][a-z0-9-]{0,63}` → v3 `OperationId`/`HandleId`/`PlanId`/`LeaseId`; `Fingerprint` 64-hex-char → v3 Provider `spec.configFingerprint`; `ProviderContractError` 34 variants → v3 Provider operation error taxonomy; canonical name constraint `^[a-z][a-z0-9-]*$` max 63 bytes → same as `ids.rs::is_label()` shared validator (§16.2). **Excluded ADR45 assumptions**: `RealmLabel`/`WorkloadName`/`RealmPath` identity types: ADR45 workload/realm address format; replaced by `metadata.name` + Zone `ResourceRef`. `ProviderType::ALL` fixed 11-type closed enum: v3 Provider type is an open string field in the Provider resource spec. `STATE_SCHEMA_VERSION=2`/`STATE_SCHEMA_GENERATION=1`: v3 store schema uses redb table versioning, not a JSON schema version field. v2 service fingerprint tests reference ADR45-specific proto files; v3 fingerprints use different proto inputs but the same `service_schema_fingerprint` seeding mechanism. Generated stubs in `generated_v2_services/` are v2-specific and excluded; v3 uses regenerated stubs from `proto/v3/`. |
| Integration | v3 Zone API service layer uses `MethodSpec.mutating` + `StrictWireMessage` for admission and fingerprinting; `v3/state.rs` `Digest`/`StateEnvelope` integrate with redb store writes; `CanonicalPayloadVerifier` validates resource payloads loaded from store; audit types feed §13.3 Zone audit segment; lock types wire into ADR 0034 lock lifecycle |
| Data migration | v2 and v3 wire type namespaces coexist in the same contracts crate; no migration |
| Validation | `v3-canonical-name-matches-ids-is-label-regex`, `v3-service-fingerprint-changes-on-method-mutation` (behavior ported from `public_endpoint_fingerprint_binds_daemon_and_guest_method_descriptors`), `v3-audit-gap-detection-covers-missing-segment`, `v3-strict-wire-rejects-unknown-fields`, `v3-state-envelope-digest-mismatch-rejected`, `v3-canonical-payload-verifier-binding-holds-under-mutation` |
| Removal proof | `d2b-contracts/src/v2_{services,state,identity,provider}.rs` and `v2_component_session.rs` deprecated in contracts crate after all v2 clients decommissioned |

---

### ADR046-zone-control-014

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-zone-control-014` |
| Dependency/owner | ADR046-zone-control-001, ADR046-routing-011; Nix module owner |
| Current source | `nixos-modules/options-realms.nix` (option schema/assertion conventions); `nixos-modules/bundle-artifacts.nix` (bundle emit pattern); `nixos-modules/provider-registry-v2-json.nix` (provider-registry JSON emit); `nixos-modules/assertions.nix:730` (assertion style for unsafe-local); `packages/d2b-realm-core/src/ids.rs` (`LABEL_PATTERN`, `MAX_ID_LEN`, `is_label()` - eval-time name validation target) |
| Reuse source | None |
| Reuse action | create |
| Destination | `nixos-modules/options-zones.nix`, `nixos-modules/generated/resource-types.nix`, `nixos-modules/generated/options-zones-<ResourceType>.nix`, `nixos-modules/resource-type-validators.nix` |
| Detailed design | Consume the generator and registry established by ADR046-routing-011 to declare the unified `d2b.zones.<zone>.resources.<name>` option tree plus the Zone-level compiler-only `parentZone` scalar. The generated standard registry is exactly the canonical 19 types (`Zone`, `ZoneLink`, `Provider`, `Host`, `Guest`, `Process`, `EphemeralProcess`, `Network`, `Volume`, `Credential`, `Device`, `Endpoint`, `User`, `Role`, `RoleBinding`, `Quota`, `EmergencyPolicy`, `ResourceExport`, `ResourceImport`), and every standard type has a strict generated `spec` submodule carrying the schema's Nix types, defaults, bounds, and documentation. Installed Provider artifacts may append only signed qualified ResourceTypes whose strict schema has been verified and generated into the evaluated option set. `type` is selected from that closed combined registry: an unknown standard string, an unqualified extension, or a qualified type without an installed verified schema fails evaluation; there is no unrestricted string or free-form `spec` fallback. Require `parentZone` on every non-root Zone and forbid it on `local-root`; resolve it against declared Zone keys, reject self-parent/conflicting module definitions/cycles, and cap each ancestry path at 16 Zone names. Compile the validated map into sealed allocator bootstrap topology; never emit it into the resource bundle or `Zone.spec`. Declare the global `d2b.artifacts.<id>` catalog with `package` (types.package, required) and `type` (closed enum, required). Provider `spec.artifactId` is a plain catalog ID; the derivation is not a `spec` field. Implement the Phase 1 cross-resource assertions (§14.10 Phase 1 table); retain `credentialRef`, `resourceRef`, and `closedEnum` helpers; reject operator-authored `Zone`; and enforce child-local ZoneLink topology. The runtime creates `Zone/<name>`, `Provider/system-core`, and `Provider/system-minijail` with `managedBy=controller`; none is emitted or inferred from the configuration bundle. `allowUnsafeLocal` maps to the dedicated Host admission gate. Provider manifest-derived fields (`spec.exports`, `spec.components`, `spec.dependencies`, `spec.permissionClaims`, `spec.upgradePolicy`, `spec.restartPolicy`) are read-only and setting one is an eval error. |
| Integration | ADR046-routing-011 supplies the one canonical 19-type registry and generated option family; validated `parentZone` feeds the allocator bootstrap sealer; the closed `d2b.zones.<zone>.resources.*` tree is consumed by ADR046-zone-control-015; the Zone controller (ADR046-zone-control-001) reads the resulting bundle; Provider package conventions come from ADR046-zone-control-003 |
| Data migration | Replace `nixos-modules/options-realms.nix`-derived option trees once Zone controller is live and has reached parity |
| Validation | All Phase 1 eval tests in §15.8 (`nix-eval-name-regex-enforced`, `nix-eval-verb-closed-enum`, `nix-eval-session-verb-closed-enum`, `nix-eval-relay-session-verb-known`, `nix-eval-roleref-format`, `nix-eval-subject-type-restricted`, `nix-eval-no-duplicate-subjects`, `nix-eval-bootstrap-provider-rejected`, `nix-eval-provider-missing-artifact-id`, `nix-eval-artifact-id-not-in-catalog`, `nix-eval-artifact-wrong-type`, `nix-eval-artifact-id-format`, `nix-eval-credentialref-declared`, `nix-eval-dollar-key-rejected`, all five `nix-eval-parent-zone-*` vectors, `nix-eval-zonelink-child-name-mismatch-rejected`, `nix-eval-zonelink-second-uplink-rejected`, `nix-eval-zonelink-limits-maxpendingintents-bound`); Phase 2 runs `nix-build-relay-scope-restricted`; drift test asserts the standard registry and generated option modules cover exactly all 19 canonical types; negative evals reject unknown strings, unqualified extensions, unsigned or uninstalled qualified types, and unknown `spec` fields; a positive fixture admits an installed signed qualified type and validates its strict generated schema |
| Removal proof | `nixos-modules/options-realms.nix`, `nixos-modules/realm-controller-config-json.nix`, `nixos-modules/realm-identity-config-json.nix` deleted after Zone controller and resource compiler reach full parity; `nixos-modules/assertions.nix` lines referencing `allowUnsafeLocal`/realm names removed after Host admission validation replaces them |

### ADR046-zone-control-015

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-zone-control-015` |
| Dependency/owner | ADR046-zone-control-014; resource compiler owner |
| Current source | `nixos-modules/bundle-artifacts.nix` (artifact emit pattern); `nixos-modules/processes-json.nix` (canonical JSON serialization conventions); `packages/xtask/src/main.rs` `gen-schemas` subcommand (schema-from-derivation pattern); `packages/d2b-core/src/bundle.rs` (current bundle DTO shape for adaptation reference) |
| Reuse source | None |
| Reuse action | create |
| Destination | `packages/d2b-resource-compiler/src/{main,bundle,schema,validator,digest,sort,secret_lint,generation}.rs`; exposed as `pkgs.d2b-resource-compiler`; called from `nixos-modules/resource-compiler.nix` |
| Detailed design | Implement all Phase 2 build-time checks (§14.10 Phase 2 table): dispatch on `type` to look up ResourceTypeSchema; validate each resource's `spec` canonical JSON against the committed schema (build validation compares canonical rendered JSON against ResourceTypeSchema for each core type); compile the `d2b.artifacts.*` catalog: for each entry, build/include the derivation, verify `type` is a recognized value, compute `digest` over the derivation output, extract and hash manifest and config schema files, validate signature chain and conformance attestation; detect duplicate artifact IDs; for each Provider resource, look up `spec.artifactId` in the compiled catalog (build failure if absent or wrong type), verify `configSchemaDigest` matches schema SHA-256, validate operator `spec.config` against loaded JSON Schema using a pure-Rust validator bundled in the derivation, verify `manifestDigest` and signature chain, load manifest-derived fields (`exports`, `components`, `dependencies`, `permissionClaims`, `upgradePolicy`, `restartPolicy`) into the bundle envelope; emit private integrity-pinned artifact catalog (ID → type/digest/closure metadata) as a separate private file (never merged into the public resource bundle); check `spec.rules[*].resourceTypes` against installed Provider catalogs in the bundle (Role); verify `spec.roleRef` names an existing Role in the bundle (RoleBinding); verify `spec.subjects[*]` names resolve in bundle (RoleBinding); check ResourceType short-name collision across all Zone Providers; RFC 8785 canonical JSON serialization; per-resource `digest` computation; `bundleDigest` computation over sorted `resources` array; inline-secret heuristic lint (`--strict-secrets` flag); `generation` counter persistence in Nix module state; emit `zone-resources.json` bundle with all fields per §14.9 |
| Integration | Reads from `d2b.zones.<zone>.resources.*` (ADR046-zone-control-014); emits bundle consumed by ADR046-zone-control-001 configuration publication handler; generation counter stored as Nix module derivation input hash (hermetic) or in a NixOS state file (impure) - exact mechanism is implementation decision |
| Data migration | Full reset; no prior bundle state exists to carry forward |
| Validation | All Phase 2 build tests in §15.8 (`nix-build-artifact-id-missing-from-catalog`, `nix-build-artifact-wrong-type-rejected`, `nix-build-duplicate-artifact-id`, `nix-build-artifact-store-path-absent-from-bundle`, `nix-build-artifact-store-path-absent-from-config`, `nix-build-config-schema-failure`, `nix-build-schema-digest-mismatch`, `nix-build-manifest-digest-mismatch`, `nix-build-resourcetype-collision`, `nix-build-bundle-sorted`, `nix-build-bundle-digest-stable`, `nix-build-per-resource-digest-correct`, `nix-build-credential-ref-survives-build`, `nix-build-inline-secret-lint-warning`, `nix-build-inline-secret-strict-failure`) |
| Removal proof | No current equivalent; additive only; no prior code removed |

### ADR046-zone-control-016

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-zone-control-016` |
| Dependency/owner | ADR046-zone-control-015; ADR046-zone-control-001; configuration publication handler owner |
| Current source | `packages/d2bd/src/lib.rs` lines 1408 and 16741 (`RealmControllersJson` load - live active generation load pattern); `nixos-modules/realm-controller-config-json.nix` (current config bundle emit to `/etc/d2b/`); `packages/d2b-realm-core/src/allocator_engine.rs` (generation/activation pattern); `packages/d2b-core/src/unsafe_local_workloads.rs:16-164` (`MAX_UNSAFE_LOCAL_WORKLOADS=256`, etc. - bounds reference for Credential/Host cleanup) |
| Reuse source | main `a1cc0b2d`: `packages/d2b-session/src/lifecycle.rs` `begin_reconnect` exponential backoff logic (cleanup retry); `packages/d2b-state/` lock/lease types (ADR046-store-001 dependency for bundle file locking) |
| Reuse action | adapt |
| Destination | `packages/d2b-core-controller/src/configuration.rs` (Phase 3 activation, diff, delete dispatch); `packages/d2b-core-controller/src/cleanup.rs` (pending tracking, status, stuck detection, rollback verb handler) |
| Detailed design | Implement all Phase 3 runtime activation checks (§14.10 Phase 3 table); `bundleDigest` integrity verify; `zoneUid` consistency check; `generation` monotone check; per-resource `digest` re-verify; atomic generation advance in `store_meta`; diff computation (resources with `managedBy=configuration` absent from new bundle → async Delete; `managedBy=controller`/`managedBy=api` resources untouched); `managedBy` and `configurationGeneration` field maintenance on resources in redb store; `cleanupPendingCount` and `generationCleanupPending` maintenance; Zone.status.phase → Degraded while cleanup pending, reverts on completion; `GenerationCleanupPending`/`GenerationCleanupFailed` condition management; stuck-cleanup `GenerationCleanupFailed=True` at `cleanupStuckThreshold` (default 5 min) with exponential backoff retry; prior generation bundle retention and pruning up to configured `retainedPriorGenerationCount` (default 3, range 1..16); audit emission for all four cleanup audit kinds (§14.11); `zone.config-rollback` verb handler Primary reuse disposition: `adapt`. Preserved source-plan detail: extract exponential backoff from `begin_reconnect`. |
| Integration | Zone store / redb (ADR046-store-001); core-controller watch/trigger bus (ADR046-zone-control-011); Zone status writer (ADR046-zone-control-001); audit emitter (§13.2); Credential revocation hook (triggered when `deletionRequestedAt` is set on a Credential and `core.credential-revoke` finalizer is present; revocation completes before finalizer clearance) |
| Data migration | Full reset; no prior bundle activation state exists to carry forward |
| Validation | All Phase 3 runtime and cleanup tests in §15.8 (`nix-runtime-bundledigest-integrity`, `nix-runtime-generation-monotone`, `nix-runtime-zoneuid-mismatch-rejected`, `nix-runtime-zonename-mismatch-rejected`, `nix-runtime-activation-nonblocking`, `nix-runtime-provider-config-invalid-continues`, all `cleanup-*` and `rollback-*` tests) |
| Removal proof | `d2bd/src/lib.rs` config-load at lines 1408 and 16741 removed after Zone configuration publication handler reaches parity; `realm-controller-config-json.nix` and `realm-identity-config-json.nix` Nix bundle-emit removed after resource compiler reaches parity |

### ADR046-pkg-001

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-pkg-001` |
| Dependency/owner | ADR046-zone-control-003; workspace policy owner |
| Current source | `packages/d2b-contract-tests/tests/policy_contracts.rs` lines 5-6 (D2B_FIXTURES gate / workspace-checks integration pattern - `implemented-and-reachable`, baseline `b5ddbed6`); `packages/d2b-contract-tests/tests/static_invariants.rs` (hermetic policy test structure - `implemented-and-reachable`); `tests/tools/rust-workspace-checks.sh` (D2B_FIXTURES step shell harness - `implemented-and-reachable`); AGENTS.md "Naming conventions" section (`<base>-<implementation>` workspace sort rules - `implemented-and-reachable`); `packages/d2b-realm-core/src/ids.rs` `LABEL_PATTERN` / `MAX_ID_LEN` (name regex reused for crate name token validation - `implemented-and-reachable`) |
| Reuse source | None |
| Reuse action | create |
| Destination | `packages/d2b-contract-tests/tests/policy_provider_crate_layout.rs` (new file; gated under D2B_FIXTURES in existing `tests/tools/rust-workspace-checks.sh`) |
| Detailed design | Implement `policy_provider_crate_layout.rs` with the following test functions: (1) `every_provider_crate_has_src` - walk `packages/d2b-provider-*/` directories in the workspace, assert each contains `src/`; failure names crate and missing path; (2) `every_provider_crate_has_tests` - assert `tests/` present; (3) `every_provider_crate_has_integration` - assert `integration/` present; (4) `every_provider_crate_has_readme` - assert `README.md` present; (5) `every_provider_readme_has_required_sections` - read `README.md`, check for all nine section headings from §4.8.3 (case-insensitive, after stripping `#` and whitespace); failure names the missing heading(s); (6) `every_integration_file_has_target_declaration` - for each `integration/*.rs` file, scan first 20 lines for exactly one `//! integration-target: (container|host-integration)` declaration; failure names the file and the violation (missing/multiple/invalid value); (7) `non_provider_crates_exempt` - verify the check does not run on non-`d2b-provider-*` crates. All checks are filesystem-only (no compilation). Workspace member list is discovered by parsing `packages/Cargo.toml` `[workspace].members`. Gate: add the new test file to `tests/tools/rust-workspace-checks.sh` D2B_FIXTURES list alongside existing policy tests |
| Integration | `make test-policy` and `make check` both fail if any provider crate violates §4.8; consistent with existing `no-bash-ast-walker` and workspace-sort gates; ADR046-zone-control-003 references §4.8 for Provider package conventions |
| Data migration | Additive; no existing `d2b-provider-*` crates in the pre-ADR45 baseline; first Provider crate created must comply from inception |
| Validation | §15.3 layout conformance tests: `provider-crate-layout-src-required`, `provider-crate-layout-tests-required`, `provider-crate-layout-integration-required`, `provider-crate-layout-readme-required`, `provider-readme-sections-all-present`, `provider-readme-sections-partial-missing`, `provider-integration-target-declared`, `provider-integration-target-unique`, `provider-integration-target-valid-values`, `provider-crate-naming-convention`, `provider-crate-layout-non-provider-exempt` |
| Removal proof | No existing code removed; additive policy test only |

### ADR046-zone-control-019

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-zone-control-019` |
| Dependency/owner | ADR046-zone-control-001, ADR046-provider-004; ADR046-zonelink owner; `d2b-core-controller` + `d2b-contracts` owners |
| Current source | None - net-new ADR 0046 cross-Zone sharing model (D096); no pre-ADR45 baseline equivalent |
| Reuse source | ZoneLink reconcile/handler scaffolding (§3); `packages/d2b-session/src/streams.rs` `NamedStream` credit/backpressure (bounded encrypted stream carriage) |
| Reuse action | adapt |
| Destination | `packages/d2b-contracts/src/v3/{resource_export,resource_import}.rs` (base schemas); `packages/d2b-core-controller/src/export_import.rs` (core ZoneLink export/import routing controller); shared adapter trait in `packages/d2b-provider/src/share_adapter.rs` (`ExportAdapter`/`ImportAdapter` signed-capability traits) |
| Detailed design | Implement the `ResourceExport` and `ResourceImport` standard ResourceTypes per §8A plus signed Provider `ProjectionFactory` metadata binding qualified Service type, qualified Binding type, allowed owner-Service backing refs, allowed Binding target refs, projection schema/fingerprint, and aggregate factory fingerprint. Admission accepts only an owner Service as `ResourceExport.resourceRef`; matches export/import/local-factory type and fingerprints; and creates exactly one same-qualified-type projection Service (`ownerRef: ResourceImport/<name>`). It never projects Device/Endpoint/Binding and never creates Binding. Binding spec is desired consumer intent only; observations belong only in status. No cross-Zone Ref, FD, secret, path, locator, or resource grant crosses a Zone; payload bytes use bounded encrypted named streams and high-churn sessions/streams remain internal. Export removal/ZoneLink loss revokes leases and degrades the projection Service; reconnect revalidates generation and both fingerprints. D091 currency propagates Service → export → import → projection Service → authored Binding → children. Primary reuse disposition: `adapt`. Preserved source-plan detail: net-new (extend ZoneLink controller). |
| Integration | Zone store/redb (ADR046-store-001); shared D098 semantic base catalog (ADR046-provider-004); ZoneLink reconcile (§3); ComponentSession bounded encrypted named streams; signed projection factories/adapters for audio-pipewire, device-security-key, observability-otel, and policy-gated device-usbip; CLI graph rendering |
| Data migration | None - full d2b 3.0 reset; no prior cross-Zone sharing state |
| Validation | §8A.7: fast hermetic factory absent/mismatch/tamper, Service-only export target, exactly-one same-type projection Service, no Device/Endpoint/Binding projection, no auto-Binding, intent-only spec/status-only observations, backing/target allowlists, finalizer/update propagation, Provider classification, canonical Nix stability including compiler-only `d2b.zones.work.parentZone = "local-root"`, child-local `ZoneLink/work-uplink` in `d2b.zones.work.resources`, local `zoneLinkRef` resolution, quotas/reconnect/revoke, and no FD/secret/path tests; slower real encrypted-stream integration for audio/security-key/observability/policy-gated USBIP |
| Removal proof | Not applicable (new surface) |

### ADR046-zone-control-020

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-zone-control-020` |
| Dependency/owner | ADR046-zone-control-019; `d2b-core-controller` owner |
| Current source | None - net-new ADR 0046 cross-Zone sharing model (D096); the core owner/child reconcile machinery (§11) is prior-art design context only |
| Reuse source | None |
| Reuse action | create |
| Destination | `packages/d2b-core-controller/src/export_import_projection.rs` (local qualified Service projection lifecycle owned by `ResourceImport`) |
| Detailed design | Core creates exactly one same-qualified-type projection Service per `ResourceImport` and keeps it synchronized with the remote Service lease. Operators/Nix separately author same-Zone matching Binding resources with `serviceRef` plus an allowed Guest/User/Zone target; Binding specs hold desired intent only and Binding controllers write observations only to status while owning Process/Endpoint children. On revoke, mark the projection draining/revoked and let Binding controllers stop children. On delete, wait for Bindings to be deleted/retargeted (`BindingReferencesRemain`), release the lease, delete only the projection Service/provider-owned children, then clear the import finalizer. Never create/delete Binding or project Device/Endpoint. |
| Integration | ADR046-zone-control-019 controller; owner/dependency reconcile (§11, ADR046-reconcile-*); local semantic Provider import adapter |
| Data migration | None - full d2b 3.0 reset |
| Validation | Exactly one same-type Service projection owned by import; no Device/Endpoint/Binding projection; Binding never auto-created/deleted; Binding target allowlist; intent-only spec/status-only observations; owned Process/Endpoint child cleanup; pending finalizer while Binding refs remain; reconnect only after generation/factory/schema revalidation; hermetic fake-adapter + real-stream integration tiers |
| Removal proof | Not applicable (new surface) |

### ADR046-zone-control-021

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-zone-control-021` |
| Dependency/owner | ADR046-zone-control-001, ADR046-zone-control-016; `d2b-core-controller` owner |
| Current source | `packages/d2bd/src/` process-global statics `USBIP_BACKGROUND_RECONCILE_ACTIVE`, `FORCE_SHUTDOWN_GENERATIONS`, and `activation_locks()`; current per-VM configuration staging symbols; ZoneLink cursor persistence in `zone_link_cursors` |
| Reuse source | ZoneLink handler/cursor scaffolding (§3); core coordinator patterns |
| Reuse action | adapt |
| Destination | `packages/d2b-core-controller/src/{coordinator,configuration,zonelink}.rs` |
| Detailed design | Per D097 core-audit findings (§8B.2): move the process-global `USBIP_BACKGROUND_RECONCILE_ACTIVE`, `FORCE_SHUTDOWN_GENERATIONS`, and `activation_locks()` state into **per-Zone** provider/resource status or a per-Zone coordinator keyed by the authority index (no process-global singletons that ignore Zone boundaries). Migrate the configuration publisher's per-VM staging symbols to **per-Zone** staging under the single configuration-publisher authority. Make ZoneLink cursor persistence and restart adoption an authority owned by the ZoneLink handler (`ownerProof`; ambiguity quarantines). All coordinated through the core authority index; no direct broker path, no process-global lock. Primary reuse disposition: `adapt`. Preserved source-plan detail: `adapt` (move process-global state to per-Zone status/coordinator). |
| Integration | Core authority index (ADR046-zone-control-019); ZoneLink reconcile (§3); configuration publication handler (ADR046-zone-control-016) |
| Data migration | Full d2b 3.0 reset; no process-global state persisted across the cutover |
| Validation | Two Zones on one host do not share `USBIP_BACKGROUND_RECONCILE_ACTIVE`/`FORCE_SHUTDOWN_GENERATIONS`/activation-lock state; per-Zone configuration staging isolation; ZoneLink cursor adoption by `ownerProof` and quarantine on ambiguity; hermetic with fakes |
| Removal proof | The process-global statics and per-VM staging symbols are deleted after the per-Zone coordinator reaches parity; confirmed by `cargo check` and a no-process-global lint |

### ADR046-zone-control-022

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-zone-control-022` |
| Dependency/owner | ADR046-zone-control-019, ADR046-api-001; `d2b-core-controller` + resource API owners |
| Current source | None - net-new D097 admission (Provider cardinality) |
| Reuse source | Core authority index (ADR046-zone-control-019); resource API admission (`ADR-046-resource-api-and-authorization`) |
| Reuse action | adapt |
| Destination | `packages/d2b-core-controller/src/authority.rs`; resource API admission hook |
| Detailed design | Admission enforces **Provider controller cardinality** via the core authority index: most Providers are `exactly-one` per Zone; the observability Provider is `at-most-one` (zero-or-one). A `Create`/activation that would install a second controller for an `exactly-one` Provider (or a second observability Provider) is rejected with `duplicateConflict` naming the incumbent owner digest before any effect; config activation goes `Degraded`. |
| Integration | Core authority index; resource API admission; configuration activation |
| Data migration | None - full d2b 3.0 reset |
| Validation | Second Provider controller for an `exactly-one` Provider rejected with `duplicateConflict`; second observability Provider rejected; single controller admitted; `Degraded` config activation names the incumbent digest; hermetic |
| Removal proof | Not applicable (new surface) |

### ADR046-zone-control-023

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-zone-control-023` |
| Dependency/owner | ADR046-zone-control-019; `d2b-core-controller` + `d2b-contracts` owners |
| Current source | None - `Quota` and `EmergencyPolicy` scope-uniqueness are specified (§7, §8) but not implemented/tested at baseline |
| Reuse source | `Quota`/`EmergencyPolicy` schemas (§7, §8); core authority index |
| Reuse action | adapt |
| Destination | `packages/d2b-core-controller/src/{quota,emergency_policy}.rs`; `packages/d2b-contracts/src/v3/{quota,emergency_policy}.rs` |
| Detailed design | Implement `Quota` and `EmergencyPolicy` scope-uniqueness as `exactly-one`-per-scope authorities in the core authority index: a second `Quota`/`EmergencyPolicy` claiming the same scope is a `duplicateConflict`. Add the scope-uniqueness admission, status, and the test matrix (per D094 fast hermetic tests). Primary reuse disposition: `adapt`. Preserved source-plan detail: net-new (implementation + tests). |
| Integration | Core authority index (ADR046-zone-control-019); resource API admission |
| Data migration | None - full d2b 3.0 reset |
| Validation | Duplicate-scope `Quota`/`EmergencyPolicy` rejected with `duplicateConflict`; single-scope admitted; union/individual scope flags honored; fast hermetic tests |
| Removal proof | Not applicable (new implementation) |

### ADR046-zone-control-024

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-zone-control-024` |
| Dependency/owner | ADR046-zone-control-019, ADR046-zone-control-022; `d2b-core-controller` owner |
| Current source | None - net-new D097 hardware-audit contract; today physical/kernel backings are guarded per-Zone or per-process, not Host-global |
| Reuse source | Core authority index (ADR046-zone-control-019) |
| Reuse action | adapt |
| Destination | `packages/d2b-core-controller/src/authority.rs` (Host-global index scope + hardware admission) |
| Detailed design | Extend the core authority index so `host`, `physical-device`, `seat`, and `external-service` authorities are keyed **Host-global** (`(Host, authorityClass, opaqueKeyDigest)`), admitting exactly one owner across all Zones on the host, while `zone`-scoped authorities stay Zone-local. Enforce the §8B.3 hardware rows: GPU full-device exclusive vs render-node shared; per-Guest swtpm and physical TPM exclusive (state never wiped); one Core-derived `physical-usb-backing/v1` identity digest claimed through the exact `(Host, physical-usb-backing, opaqueKeyDigest)` tuple by every USB or security-key implementation before effects, plus the separate host-global `usbip-host` module and the Host-global `Provider/device-usbip` relay `Endpoint`, exactly one per Core-derived Network UID/signed-policy-port digest with multiplexed arbitration and `usbip-network-relay-authority-conflict` on a second owner; macvtap/NIC `parentInterface` `passthru` globally exclusive across all Zones; host-shared `/dev/kvm` and `/dev/vhost-vsock` as `Provider/system-core` grants (no 28th Provider, no `kvm` busClass); globally-unique vsock CID; other fixed listener ports as `Endpoint`s; host store + per-Guest store-view writer; Network TAP/bridge. A second Zone claiming the same physical backing receives `physical-usb-backing-conflict` before any open, bind, withhold, module, relay, or attachment effect; restart adopts by `ownerProof`; Guest-stop drains dependent leases. GPU-owned `udmabuf`/video and per-session `vhost-vsock` tokens stay authority subresources/DeviceGrants (not resources/Providers). Primary reuse disposition: `adapt`. Preserved source-plan detail: net-new (Host-global index scope for host/physical-device authorities). |
| Integration | Core authority index (ADR046-zone-control-019); Provider cardinality admission (ADR046-zone-control-022); `Provider/system-core` KVM/vhost-vsock grant; `Provider/device-*` and `Network` authorities |
| Data migration | None - full d2b 3.0 reset |
| Validation | Two Zones on one host cannot both claim one GPU/TPM/USB/`/dev/kvm`/passthru NIC/vsock CID/fixed port - second is `duplicateConflict`; security-key and USB implementations resolving the same physical token submit byte-identical `physical-usb-backing` tuple keys and the loser receives `physical-usb-backing-conflict` before any effect; a second USBIP relay `Endpoint` for one Core-derived Network UID/signed-policy-port digest receives `usbip-network-relay-authority-conflict`, while multiple admitted Services share the multiplexed owner and no `Network` authority owns the listener/firewall; Provider-private authority classes/digests cannot bypass the collision; render-node shared admits bounded holders; per-Guest swtpm exclusive and marker never wiped; host-global adoption by `ownerProof`; hardware D096 exportability (GPU/KVM/TPM/store/macvtap non-exportable; semantic USB policy-gated); fast hermetic with fakes |
| Removal proof | Not applicable (new surface) |

---

## 18. All decisions resolved

All design decisions for Zone, ZoneLink, Provider, Role, RoleBinding, Quota,
and EmergencyPolicy are resolved as of this revision. No unresolved design
blocks remain. New design decisions are tracked in
`docs/specs/ADR-046-decision-register.md`.
