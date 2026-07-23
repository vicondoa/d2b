# ADR 0046 core controllers

| Field | Value |
| --- | --- |
| Spec ID | `ADR-046-core-controllers` |
| Parent | ADR 0046 |
| Status | Proposed |
| Version | 1 |
| Baseline | `b5ddbed67867d9244bf33390868101bd9b053e49` |
| Normative | Yes |
| Owners | Fixed core-controller process, `Provider/system-core` |
| Depends on | `ADR-046-resource-store-redb`, `ADR-046-resource-api-and-authorization`, `ADR-046-resource-reconciliation`, `ADR-046-provider-model-and-packaging` |
| Supersedes | Current daemon-global hard-coded orchestration ownership |

## Process model

Each Zone has one fixed core-controller process. It hosts multiple isolated
controller handlers in one binary/runtime to reduce memory/process overhead.

It is also the `Provider/system-core` controller. Provider/system-core and the
fixed Provider/system-minijail controller are the two Provider bootstrap
exceptions. It starts after the embedded store/resource API/bus endpoint is
ready and connects to those services through the same local
d2b-bus/ComponentSession path used by other controllers. Only the non-controller
store actor touches redb directly.

The process:

- has zero ambient host capability;
- cannot invoke arbitrary broker operations;
- owns no Provider payload state;
- uses typed internal core effect ports only for store lifecycle/emergency
  operations that cannot be resources;
- has reserved resource API/bus capacity;
- runs handlers concurrently but serializes each resource;
- exposes aggregate health plus per-handler status.

All other Provider controllers are Process resources under Hosts or Guests.

## Handler catalog

### Configuration publication

Owns:

- configured root resource generation;
- validation/staging/activation;
- prior generation rollback window.

Algorithm:

1. read integrity-pinned candidate bundle;
2. validate Provider packages/APIs/config/refs/owners/RBAC/budgets;
3. stage inactive resources in bounded transactions;
4. atomically activate one configuration revision;
5. trigger affected resources/providers/controllers;
6. retain prior generation for bounded rollback/drain;
7. prune only after ownership/finalizer checks.

It does not overwrite controller-created children merely because root config
changes.

### API catalog/binding

Owns ResourceTypeSchema, ResourceApiExport, and ResourceApiBinding internal
control resources.

It verifies:

- Provider/package/signature;
- ResourceType short-name collision;
- exact schema/descriptor fingerprint;
- additive compatibility;
- permission claims narrowed by policy;
- controller implementation exists;
- API withdrawal has no unresolved resources/finalizers.

It atomically swaps the in-memory validator/catalog after commit.

### Authorization

Owns Role/RoleBinding validation and revision-bound authorization indexes.

It:

- validates subjects/refs/scope/verbs;
- builds/switches indexes after durable commit;
- invalidates session/API authorization caches/leases immediately;
- triggers affected Provider/controller/resource reconcilers;
- never turns a deny into allow through fallback.

### Provider lifecycle

Owns Provider resources and aggregate status.

It:

- verifies package/trust/config/conformance;
- validates controller/service/worker graph;
- creates owned Volume/Process/EphemeralProcess and other required children;
- waits for required components/dependencies;
- publishes exported ResourceTypes/services only after ready;
- drains/withdraws/revokes components on update/disable/delete;
- aggregates exact child status without spawning directly.

Provider/system-core is handled internally without a Process child. All others
use the same owned child-resource algorithm.

### Controller registration and hints

Owns:

- signed descriptor validation;
- authenticated controller lease;
- ResourceType/provider/Host/Guest/domain binding;
- watch plan;
- reverse dependency indexes;
- hint filtering/coalescing;
- checkpoint/high-water tracking;
- lease withdrawal/replacement.

It implements the <=5 ms commit-to-handler path with the store post-commit
dispatcher and d2b-bus.

### Ownership/finalizer

Owns generic owner graph integrity and deletion ordering, not Provider-specific
finalizer logic.

It:

- validates singular ownerRef/UID/cycle/depth;
- emits owner hints for every child mutation;
- tracks child-first deletion;
- dispatches finalizer owners;
- reports blocked/ambiguous conditions;
- never clears another controller's finalizer or invents cleanup success.

### Revision/watch maintenance

Owns:

- watch registration/high-water handoff;
- revision-log compaction policy;
- expired-cursor/relist;
- watch/stream quotas;
- owner/dependency live dispatch.

The store actor performs redb transactions; this controller schedules policy and
reports status.

### EphemeralProcess cleanup

Owns terminal retention cleanup:

- Succeeded uses successfulTtl, default 1h;
- Failed uses failedTtl, default 24h;
- starts at completedAt;
- writes/validates cleanupEligibleAt;
- respects finalizers, owner deletion, incident holds, expected revision;
- calls normal Delete; does not remove rows directly.

### Zone link/delegation

Owns ZoneLink:

- parent/child Zone identity and transport/session requirements;
- child-local RoleBinding/authorization expectation;
- route/resource cursor status;
- reconnect/resync;
- local intents while disconnected;
- disable/revocation.

It never stores child resources or credentials in the parent.

### Budget/emergency policy

Budgets are shared Host/Guest ExecutionPolicy and Process fields, not
ResourceTypes. This handler:

- validates hierarchical Host/Guest allocations against Zone capacity/policy;
- tracks aggregate reservations/observations;
- blocks overcommit;
- narrows child Zone capacity;
- handles digest/Provider/Host/Guest/Zone/global emergency disable;
- revokes routes/sessions/grants and stops child Processes via normal resources;
- preserves incident-held Volume/Provider state.

### Store lifecycle

Coordinates:

- backup request/admission;
- staged restore/internal schema upgrade;
- compaction;
- corruption quarantine;
- health/metrics;
- reset inventory.

Actual redb transaction/file publication stays in the embedded store actor/
storage owner. Normal controllers cannot call these internals.

### system-core Host

Provider/system-core reconciles Host:

- validates it represents the local OS attached to this Zone runtime;
- validates defaultDomain/allowedDomains/defaultUserRef/budget/policy;
- reports local host availability/capabilities;
- exposes Host Provider capabilities used by Process/Volume/Network/Device
  controller descriptors;
- never starts a Process directly.

A Zone may have multiple policy/budget-separated Hosts, all
mapping to the same physical OS but accepting different Process domains/users.

### system-core User

Provider/system-core reconciles local User:

- configured identity/name;
- NSS/passwd/group lookup or explicitly supported local source;
- observed UID/GID/groups/home/session manager availability;
- status only, no credential bytes;
- detects UID/name/group drift;
- does not create arbitrary users unless a later reviewed Provider implements
  that behavior.

User status supports Process userRef and Volume ACL owner/group refs.

## Controller health/status

Each handler exposes:

- observed configuration/API/policy generation;
- phase/conditions/outcome;
- queue/running counts;
- last watch revision/checkpoint;
- lastReconciledAt;
- degraded/failed stable code;
- retry/backpressure;
- latency histogram without resource-name labels.

The core-controller aggregate becomes Ready only when mandatory store/API/auth/
configuration/Provider/controller/ownership handlers are Ready. Optional link/
backup cleanup work can report Degraded without false total failure.

## Startup

1. Zone runtime validates/opens store.
2. Resource service and local d2b-bus/ComponentSession endpoint start.
3. Fixed system-core and system-minijail controller processes start and
   authenticate as their exact Provider subjects.
4. The compiled bootstrap authorization grants only the closed verbs to exact
   Provider/system-core and Provider/system-minijail subjects.
5. Handlers list/recover/checkpoint concurrently.
6. Configuration publishes/recovers active generation.
7. system-core Hosts/Users reconcile.
8. Other Provider controllers/processes launch through resources.
9. Zone readiness publishes after mandatory handlers are current.

No handler requires all optional Providers to be ready.

## Restart

The core controller stores no authoritative private ledger outside resources/
operations/revision log. On restart it:

- authenticates a new ComponentSession generation;
- relists owned resources;
- resumes from durable checkpoints where valid;
- revalidates Provider/controller leases;
- does not clean up before Process/Host/Guest/Volume owners observe/adopt;
- preserves Unknown/ambiguous states.

## Security boundary

Keeping handlers in one process does not union arbitrary Provider privilege:

- only fixed core ResourceTypes/verbs;
- no Provider payload config/secrets/state;
- no broker socket/raw host paths;
- typed store-lifecycle/emergency internal port;
- all resource work still uses Role/RoleBinding and expected revisions;
- handler panic/failure is contained, surfaced, and may restart process, but
  store remains independently owned by Zone runtime.

## Current-code fit

| Item | Treatment |
| --- | --- |
| Current anchor | d2bd startup/config/provider/process/operation/adoption owners; Realm access/allocator engines; Nix config generation; storage/sync contracts |
| Evidence class | Current owners are mixed reachable/generated; generic core-controller process is ADR-only |
| Behavior retained | Single owners, deterministic config, typed policy, idempotency, recover-before-cleanup, pidfd non-persistence, bounded audit |
| Required delta | Separate fixed core controller, ResourceType handlers, resource-only authority, async watches/checkpoints |
| Reuse path | Extract pure controller algorithms/validators from exact current/main sources per work item |
| Replacement/deletion | d2bd monolithic branches remain until each handler and Provider successor is integrated |
| Feasibility proof | One process with all handlers over local ComponentSession, fast hint path, crash/relist/revoke tests |
| Future owner | Work items below |

## Implementation work items

### ADR046-core-001

| Field | Value |
| --- | --- |
| Dependency/owner | W0/W1a; core-controller owner |
| Current source | `packages/d2bd/src/lib.rs`, `provider_registry.rs` if present on source ref, supervisor state, operations; `d2b-realm-core/src/{allocator_engine,identity_store}.rs` |
| Reuse source | Useful pure handler/toolkit code from main named in implementation sub-items |
| Reuse action | extract and adapt |
| Destination | `packages/d2b-core-controller/src/{main,configuration,api_catalog,authz,providers,controllers,ownership,watches,cleanup,zone_links,budgets,store}.rs` |
| Detailed design | One fixed process, isolated handlers, async ResourceClient, health/startup/restart |
| Integration | Zone runtime local bus/session; Provider/system-core resource identity |
| Data migration | Full reset |
| Validation | Per-handler unit/property tests plus multi-process startup/restart |
| Removal proof | Current daemon branches removed after handler/Provider parity |

### ADR046-core-002

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-core-001; system-core Provider owner |
| Current source | Nix host/Realm options/index; host check/provider code; user/group lookup and unsafe-local eligibility |
| Reuse source | Any main local-host/user-provider code selected by exact sub-items |
| Reuse action | extract/adapt |
| Destination | `packages/d2b-provider-system-core/src/{host,user}.rs` linked into fixed core controller |
| Detailed design | Host and User schemas/reconcile/status/capabilities |
| Integration | Bootstrap Provider/system-core; other controllers target Host/Guest/User refs |
| Data migration | New v3 resources from Nix |
| Validation | Multiple Hosts, system/user restrictions, UID/session drift |
| Removal proof | Host grouping/user helper policy removed only after resource parity |
