# ADR 0046 Provider dossier: shell-terminal

| Field | Value |
| --- | --- |
| Spec ID | `ADR-046-provider-shell-terminal` |
| Parent | ADR 0046 |
| Status | Accepted |
| Version | 2 |
| Baseline | `b5ddbed67867d9244bf33390868101bd9b053e49` |
| Normative | Yes |
| Owners | `d2b-provider-shell-terminal`, Zone core-controller (ProviderDeployment/Process reconcile), Nix resource compiler |
| Depends on | `ADR-046-decision-register`, `ADR-046-terminology-and-identities`, `ADR-046-resource-object-model`, `ADR-046-resource-api-and-authorization`, `ADR-046-resource-reconciliation`, `ADR-046-components-processes-and-sandbox`, `ADR-046-provider-model-and-packaging`, `ADR-046-primitive-resource-composition`, `ADR-046-provider-state`, `ADR-046-componentsession-and-bus`, `ADR-046-core-controllers`, `ADR-046-resources-host-guest-process-user`, `ADR-046-resources-credential`, `ADR-046-telemetry-audit-and-support`, `ADR-046-nix-configuration`, `ADR-046-zone-routing` |
| Supersedes | v1 of this spec; `guestd/src/shell.rs` guest persistent-shell runtime (ADR 0038); `d2b-unsafe-local-helper` shell supervisor and wire protocol v2 (ADR 0044); `ShellOp`/`ShellOpResponse` seqpacket protocol (`d2b-contracts/src/public_wire.rs:1319,1394,1527`) |

## Scope

This dossier defines the complete pre-ADR-0045 baseline design for
`Provider/shell-terminal`.

It is authoritative for:

- `shell-terminal.d2bus.org.ShellPool`
- `shell-terminal.d2bus.org.ShellSession`
- controller and session-supervisor `Process` composition
- controller and supervisor ComponentSession contracts
- user-domain placement rules for Host and Guest targets
- PTY ownership, output-ring policy, restart adoption, stale-generation rejection,
  audit, telemetry, RBAC, Nix declarations, and implementation work items

This spec is intentionally baseline-scoped. It may reuse implementation patterns from
main-branch code and accepted baseline crates, but it explicitly excludes ADR 0045
session, realm, constellation, multi-hop controller, and child-realm assumptions. The
main approved reuse citation is `a1cc0b2da4a08ca3240a770a972fe4da6f912bef`; that reuse is
restricted to service-shape ideas, ring-buffer mechanics, and adoption probes. ADR 0045
transport, realm, and constellation assumptions are excluded.

In scope:

- provider identity, crate layout, binaries, and packaging rules
- policy-only pools
- per-session supervisors
- exact Process templates
- exact `status.phase` rules and typed `status.detail` rules
- bus routing and one-session generation identity
- Host `isolationPosture=none` warnings
- relay denial for Host user-domain access
- direct ComponentSession attach, detach, detach-all, kill, and status methods
- D087 status-first operational state in resource `status` plus the core Operation ledger

Out of scope:

- SSH
- direct-host fallback
- pool-wide supervisors
- management `EphemeralProcess` resources
- sealed output `Volume` resources for management responses
- controller-owned terminal transport
- cross-Zone shell federation beyond the K0/K1/K2/self vocabulary needed to define
  identity and stale-generation rejection

## Provider identity and crate layout

Provider identity is fixed to `Provider/shell-terminal`.

The provider owns exactly two binaries:

- `d2b-shell-terminal-controller`
- `d2b-shell-session-supervisor`

The controller is a system-domain controller and public shell service. The supervisor is a
dynamic user-domain `service` Process created exactly once per `shell-terminal.d2bus.org.ShellSession`.
It terminates a private typed ComponentSession endpoint (`shell-session-supervisor.v1`)
and the named terminal stream; it has no `ResourceClient`, no dependency or CLI portal,
and no involvement in resource-API operations. Host and Guest pools use the same
session-supervisor binary. Placement differs only by `executionRef` and `userRef`.

The canonical crate layout follows D012 and D059.

| Path | Requirement | Purpose |
| --- | --- | --- |
| `packages/d2b-provider-shell-terminal/src/` | Required | Provider controller, supervisor runtime, routing, audit, telemetry, error, schema, and support modules. |
| `packages/d2b-provider-shell-terminal/tests/` | Required | Hermetic Rust tests for schema, routing, stale-generation rejection, ring semantics, redaction, and policy checks. |
| `packages/d2b-provider-shell-terminal/integration/` | Required | Repository-driven integration fixtures for Host and Guest placement and restart-adoption coverage. |
| `packages/d2b-provider-shell-terminal/README.md` | Required | Provider overview and build or test entry point. |
| `packages/d2b-provider-shell-terminal/src/tests/integration/README.md` | Required by D059 | Scenario index for all integration fixtures. |

| Binary | Role | Placement | Principal class |
| --- | --- | --- | --- |
| `d2b-shell-terminal-controller` | controller and public lifecycle service | system-domain `Process` owned by `Provider/shell-terminal` | provider principal |
| `d2b-shell-session-supervisor` | per-session PTY owner and internal shell service | user-domain dynamic `service` `Process` owned by `shell-terminal.d2bus.org.ShellSession/<name>` | exact `User/<name>` from the pool |

No other binary is normative for this provider. In particular:

- `d2b-shell-pool-supervisor` is removed from the design.
- `d2b-guest-shell-runner` is not the managed shell session service.
- `guestd/src/shell.rs` is not the long-lived managed-session authority.
- the old unsafe-local helper supervisor protocol is superseded.

The provider exposes exactly two services:

- `shell-terminal.v3` on the controller
- `shell-session-supervisor.v1` on each supervisor

The provider declares no Provider state Volume. Controller reconciliation state is
bounded, non-secret operational data in `ShellPool` and `ShellSession` `status` plus
the core Operation ledger. Session supervisor PTY state, attach state, and output
bytes remain exclusively in supervisor process memory and are never persisted or
reported as status payload bytes. The `ProviderStateSet` is therefore empty.

## ResourceTypes overview

| ResourceType | Owner | Purpose | Creates provider worker? | Cardinality |
| --- | --- | --- | --- | --- |
| `shell-terminal.d2bus.org.ShellPool` | `Provider/shell-terminal` | capacity and policy for one execution target plus one user identity | No | one or more per Zone |
| `shell-terminal.d2bus.org.ShellSession` | `Provider/shell-terminal` | one persistent login-shell session plus exactly one supervisor `Process` | Yes, exactly one supervisor | zero or more per pool |

| Reference form | Example |
| --- | --- |
| pool `ResourceRef` | `shell-terminal.d2bus.org.ShellPool/dev-alice` |
| session `ResourceRef` | `shell-terminal.d2bus.org.ShellSession/dev-alice-main` |
| supervisor owner reference | `shell-terminal.d2bus.org.ShellSession/dev-alice-main` |
| Nix type string | `"shell-terminal.d2bus.org.ShellPool"` or `"shell-terminal.d2bus.org.ShellSession"` |

`status.phase` on both ResourceTypes uses only the common phase catalog:

- `Pending`
- `Ready`
- `Succeeded`
- `Degraded`
- `Failed`
- `Deleted`
- `Unknown`

Initialization, deletion, steady-state nuance, or terminal-cause detail belongs in the
resource-specific `status.resource.detail` object and conditions, not in ad hoc phase strings.

Per D088, `ShellPool` and `ShellSession` status use the universal
`ResourceStatus` base at top-level `status.*`; the typed pool/session fields
below are their ResourceType-common `status.resource` objects. Optional
`status.provider` carries only implementation-only observation (`providerRef`,
qualified immutable `schemaId`, semver `schemaVersion`, numeric
`observedProviderGeneration`, strict unknown-field-denied redacted `details`
≤32 KiB registered/signed in the Provider manifest) and never duplicates shared
fields. The controller writes all present layers atomically in one status
mutation.

Per D089, `ShellPool` and `ShellSession` typed desired specs are the
ResourceType base specs (Layer 2): top-level `spec.*`, including
`spec.providerRef` where applicable. Any implementation-variant desired
settings use only the canonical Layer 3 `spec.provider = { schemaId,
schemaVersion, settings }` envelope, whose `settings` are
manifest-registered/signed, deny-unknown, bounded, versioned/digested,
validated against `spec.providerRef`, and forbidden to shadow base fields;
shared fields are promoted into the base spec. `Provider/shell-terminal`
implements the exact base spec/status schema version/fingerprint, accepts the
canonical minimal base Spec, and rejects an unsupported optional base
capability only through its signed capability matrix plus typed
provider-neutral `unsupported-capability`. `spec.provider` aligns with
`status.provider`.

## `shell-terminal.d2bus.org.ShellPool` ResourceType

### Purpose

A pool is a policy object. It binds one execution target, one user identity, one
manifest-fixed login shell artifact reference, one output-ring default, and one pair of
capacity limits. A pool does not create or own a supervisor `Process`. A pool does not own
PTY state. A pool does not own attach streams.

A pool is responsible for:

1. validating `Host/<name>` or `Guest/<name>` existence in the same Zone
2. validating `User/<name>` existence and compatibility
3. validating the manifest-fixed `loginShellRef`
4. enforcing `maxSessions`
5. enforcing `maxAttached`
6. publishing aggregate counts and warnings
7. denying relay-authenticated Host user-domain access

### Canonical object shape

```yaml
apiVersion: resources.d2bus.org/v3
type: shell-terminal.d2bus.org.ShellPool
metadata:
  name: guest-alice-shell
  zone: dev
  ownerRef: Provider/shell-terminal
spec:
  providerRef: Provider/shell-terminal
  executionRef: Guest/work
  userRef: User/alice
  loginShellRef: artifact://shells/bash-login
  maxSessions: 8
  maxAttached: 1
  outputRingCapacity: 262144
status:
  observedGeneration: 1
  phase: Ready
  conditions:
    - type: Ready
      status: "True"
  lastReconciledAt: 2026-07-22T00:00:00.000Z
  startedAt: 2026-07-22T00:00:00.000Z
  completedAt: null
  outcome: null
  resource:
    detail:
      kind: CapacityReady
    activeSessions: 2
    attachedSessions: 1
    capacityRemaining: 6
    attachedCapacityRemaining: 0
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

### Spec schema

| Field | Type | Required | Default | Bound | Description |
| --- | --- | --- | --- | --- | --- |
| `apiVersion` | `string` | Yes | `resources.d2bus.org/v3` | Exact | Resource API version. |
| `type` | `string` | Yes | `shell-terminal.d2bus.org.ShellPool` | Exact | Vendor-qualified ResourceType identifier. |
| `metadata.name` | `ResourceName` | Yes | None | `^[a-z][a-z0-9-]*$`, max 63 | Pool resource name. |
| `metadata.zone` | `ResourceName` | Yes | None | Existing Zone | Owning Zone. |
| `metadata.ownerRef` | `ResourceRef` | No | `Provider/shell-terminal` | Provider or higher-level owner | Default owner is the provider. |
| `spec.providerRef` | `ResourceRef` | Yes | `Provider/shell-terminal` | Exact | Provider identity. |
| `spec.executionRef` | `ResourceRef` | Yes | None | `Host/<name>` or `Guest/<name>` in-zone | Execution target for all sessions in the pool. |
| `spec.userRef` | `ResourceRef` | Yes | None | `User/<name>` in-zone | Exact user identity that each supervisor must run as. |
| `spec.loginShellRef` | `string` | Yes | None | non-empty artifact ref, max 255 bytes | Manifest-fixed login shell artifact entry. Caller input never supplies a path. |
| `spec.maxSessions` | `u32` | No | `8` | `1..64` | Maximum live or retained sessions in the pool. The default carries forward the baseline `DEFAULT_SHELL_SESSIONS_PER_VM=8`. |
| `spec.maxAttached` | `u32` | No | `1` | `1..8` and `<= spec.maxSessions` | Maximum simultaneously attached terminal streams across child sessions. |
| `spec.outputRingCapacity` | `u64` bytes | No | `262144` | `4096..1048576` | Default ring capacity inherited by new sessions unless the session requests an equal or smaller value. |

`spec.executionRef`, `spec.userRef`, and `spec.loginShellRef` are immutable once a pool
has any child `shell-terminal.d2bus.org.ShellSession`. Mutation after session creation is rejected
with `PoolSpecFrozenByChildSessions`.

### Status schema

| Field | Type | Required | Default | Bound | Description |
| --- | --- | --- | --- | --- | --- |
| `status.observedGeneration` | `u64` | Yes | `0` | Monotonic | Last reconciled metadata generation. |
| `status.phase` | `enum` | Yes | `Pending` | `Pending|Ready|Succeeded|Degraded|Failed|Deleted|Unknown` | Common resource phase. Pools normally use `Pending`, `Ready`, `Degraded`, `Failed`, or `Deleted`. |
| `status.detail.kind` | `enum` | Yes | `Initializing` | See detail table | Typed pool-specific detail. |
| `status.detail.message` | `string` | No | Empty | redacted, max 256 bytes | Operator-readable summary with no usernames, paths, session names, or handles. |
| `status.conditions` | `[]Condition` | Yes | `[]` | closed provider-defined set | Boolean readiness and warning surfaces. |
| `status.executionRef` | `ResourceRef` | Yes | From spec | Exact | Resolved execution target snapshot. |
| `status.userRef` | `ResourceRef` | Yes | From spec | Exact | Resolved user snapshot. |
| `status.activeSessions` | `u32` | Yes | `0` | `0..64` | Count of child sessions not in `Deleted`. |
| `status.attachedSessions` | `u32` | Yes | `0` | `0..8` | Count of child sessions with one or more active attach streams. |
| `status.capacityRemaining` | `u32` | Yes | `0` | `0..64` | `maxSessions - activeSessions`, saturated at zero. |
| `status.attachedCapacityRemaining` | `u32` | Yes | `0` | `0..8` | `maxAttached - attachedSessions`, saturated at zero. |
| `status.isolationPosture` | `enum` | Yes | `isolated` | `isolated|none` | Execution-target posture for warnings. Host pools may expose `none`. |
| `status.lastReconciledAt` | `RFC3339 timestamp` | No | None | valid timestamp | Last successful reconcile timestamp. |

### `status.detail.kind` enum

| Value | Meaning |
| --- | --- |
| `Initializing` | Pool exists but target, user, login shell, and status writer validation are still in progress. |
| `CapacityReady` | Pool is fully reconciled and may admit new sessions subject to capacity. |
| `CapacityExhausted` | Pool is healthy but `maxSessions` or `maxAttached` has been reached. |
| `IsolationPostureWarning` | Pool is healthy, but a Host target reports `isolationPosture=none`. |
| `ExecutionTargetInvalid` | Referenced Host or Guest is missing or incompatible. |
| `UserInvalid` | Referenced `User/<name>` is missing or incompatible with the target. |
| `LoginShellArtifactMissing` | The manifest-fixed shell artifact reference does not resolve. |
| `Deleting` | Pool deletion is in progress and child sessions are being drained or blocked. |
| `Deleted` | Deletion completed. |

### Conditions

| Condition | True when | False/Unknown when |
| --- | --- | --- |
| `Ready` | The pool target, user, login shell, and status writer are valid; capacity counters are current. | Any validation or reconcile step is incomplete or failed. |
| `ExecutionTargetVerified` | The referenced Host or Guest exists and supports user-domain placement. | The target is absent, unresolved, or missing user-domain support. |
| `UserVerified` | The referenced `User/<name>` exists and is accepted by the target. | User lookup fails or the target rejects the user. |
| `CapacityAvailable` | At least one more `shell-terminal.d2bus.org.ShellSession` may be created. | `activeSessions >= maxSessions`. |
| `AttachCapacityAvailable` | At least one more live attach stream may be opened. | `attachedSessions >= maxAttached`. |
| `IsolationPostureWarning` | The pool targets a Host with `isolationPosture=none`; this is a warning, not an admission bypass. | The target is a Guest or a Host with an isolated posture. |
| `Deleting` | Metadata deletion timestamp is present and the finalizer has started. | The resource is not deleting. |

### Reconcile algorithm

1. Observe metadata generation and deletion intent.
2. Resolve `spec.executionRef`; require exactly one in-zone `Host/<name>` or
   `Guest/<name>`.
3. Resolve `spec.userRef`; require exactly one in-zone `User/<name>`.
4. Resolve `spec.loginShellRef` against the manifest-fixed artifact catalog.
5. Verify target capabilities:
   - Host pools require `Provider/system-systemd` user-domain placement support.
   - Guest pools require `spec.allowedDomains` containing `user` and a valid
     `spec.defaultUserRef` on the `Guest/<name>` resource.
6. Observe that the optional ProviderStateSet is empty for `shell-terminal`; there
   are no Provider state Volume prerequisites to gate pool readiness.
7. Enumerate child `shell-terminal.d2bus.org.ShellSession` resources by owner or `spec.poolRef`.
8. Count `activeSessions` and `attachedSessions`.
9. Compute remaining capacity.
10. Publish `status.isolationPosture` from the execution target.
11. If the pool targets a Host with `isolationPosture=none`, emit the warning condition
    and a pool-scoped audit event, but keep the pool eligible for local admin use.
12. Never create a supervisor `Process`; a pool is capacity-only.
13. Write common `status.phase`, `status.detail`, and conditions.
14. On validation failure, set `status.phase=Failed` for permanent schema issues or
    `status.phase=Degraded` for recoverable target disappearance.
15. Reconcile is idempotent; no side effect depends on prior controller memory.

### Error catalog

| Code | Phase | Retryable | Description |
| --- | --- | --- | --- |
| `ShellPoolAlreadyExists` | `Pending` | No | A conflicting pool already binds the same execution target and user identity. |
| `PoolSpecFrozenByChildSessions` | `Pending` | No | An immutable field changed after sessions existed. |
| `ExecutionTargetNotFound` | `Pending` | Yes | The referenced Host or Guest does not exist yet. |
| `ExecutionTargetWrongType` | `Pending` | No | The reference is not a Host or Guest resource. |
| `ExecutionTargetUserDomainUnsupported` | `Pending` | No | The target cannot host user-domain supervisors. |
| `UserRefNotFound` | `Pending` | Yes | The referenced user is absent. |
| `LoginShellArtifactMissing` | `Pending` | No | The manifest-fixed login shell artifact cannot be resolved. |
| `MaxAttachedOutOfRange` | `Pending` | No | The pool requested an invalid attached-session bound. |
| `OutputRingCapacityOutOfRange` | `Pending` | No | The pool ring capacity is outside the permitted bound. |
| `RelayHostUserDomainDenied` | `Ready` | No | A relay-authenticated caller attempted Host user-domain access. |
| `PoolDeleteBlockedBySessions` | `Deleting` | Yes | The pool still has child sessions that must finish finalization first. |

### Finalizer steps

1. Mark the pool `Deleting` condition true.
2. List child `shell-terminal.d2bus.org.ShellSession` resources.
3. If any child session is not in `Deleted`, block pool finalization with
   `PoolDeleteBlockedBySessions`.
4. Do not synthesize kill commands or management workers.
5. Clear pool-scoped route summaries and aggregate counters from status.
6. Remove the provider finalizer.
7. Allow the store to tombstone the pool and set `status.phase=Deleted`.

## `shell-terminal.d2bus.org.ShellSession` ResourceType

### Purpose

A session is the unit of persistent shell state. It owns exactly one user-domain session
supervisor `Process` named `shell-terminal--supervisor--<session-uid-short>`. The
supervisor owns exactly one PTY, exactly one login-shell process tree, exactly one bounded
merged-output ring in supervisor memory, the attach bookkeeping, and one private
ComponentSession endpoint. The controller owns lifecycle orchestration only.

### Canonical object shape

```yaml
apiVersion: resources.d2bus.org/v3
type: shell-terminal.d2bus.org.ShellSession
metadata:
  name: guest-alice-shell-main
  zone: dev
  ownerRef: shell-terminal.d2bus.org.ShellPool/guest-alice-shell
spec:
  providerRef: Provider/shell-terminal
  poolRef: shell-terminal.d2bus.org.ShellPool/guest-alice-shell
  executionRef: Guest/work
  userRef: User/alice
  loginShellRef: artifact://shells/bash-login
  sessionName: main
  outputRingCapacity: 262144
  desiredLifecycle: running
status:
  observedGeneration: 1
  phase: Ready
  conditions:
    - type: Ready
      status: "True"
  lastReconciledAt: 2026-07-22T00:00:00.000Z
  startedAt: 2026-07-22T00:00:00.000Z
  completedAt: null
  outcome: null
  resource:
    detail:
      kind: ReadyDetached
    supervisorRef: Process/shell-terminal--supervisor--0d3b0e42
    supervisorGeneration: 1
    attachCount: 0
    outputRingBytes: 8192
    outputRingEvictedBytes: 0
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

### Spec schema

| Field | Type | Required | Default | Bound | Description |
| --- | --- | --- | --- | --- | --- |
| `apiVersion` | `string` | Yes | `resources.d2bus.org/v3` | Exact | Resource API version. |
| `type` | `string` | Yes | `shell-terminal.d2bus.org.ShellSession` | Exact | Vendor-qualified ResourceType identifier. |
| `metadata.name` | `ResourceName` | Yes | Controller-generated or Nix-specified | `^[a-z][a-z0-9-]*$`, max 63 | Stable session resource name. |
| `metadata.zone` | `ResourceName` | Yes | None | Existing Zone | Owning Zone. |
| `metadata.ownerRef` | `ResourceRef` | Yes | `shell-terminal.d2bus.org.ShellPool/<name>` | pool reference | Owning pool. |
| `spec.providerRef` | `ResourceRef` | Yes | `Provider/shell-terminal` | Exact | Provider identity. |
| `spec.poolRef` | `ResourceRef` | Yes | None | `shell-terminal.d2bus.org.ShellPool/<name>` | Pool from which capacity and placement are derived. |
| `spec.executionRef` | `ResourceRef` | Yes on stored object | inherited from pool | `Host/<name>` or `Guest/<name>` | Controller copies the pool target into the session at creation time. |
| `spec.userRef` | `ResourceRef` | Yes on stored object | inherited from pool | `User/<name>` | Controller copies the pool user into the session at creation time. |
| `spec.loginShellRef` | `string` | Yes on stored object | inherited from pool | artifact catalog reference | Controller copies the pool login shell into the session at creation time. |
| `spec.sessionName` | `string` | No | controller-generated from metadata name | kebab-case, max 32 bytes | Operator-controlled display name. It is stored in spec only and never copied into telemetry or audit surfaces. |
| `spec.outputRingCapacity` | `u64` bytes | No | inherited from pool | `4096..1048576` and `<= pool.spec.outputRingCapacity` unless equal-by-default | Final ring capacity for this session. |
| `spec.desiredLifecycle` | `enum` | No | `running` | `running|stopped` | Whether the session should keep its supervisor running. `stopped` drains and stops the session without deleting the resource. |

The controller is the only writer of inherited fields during `OpenSession`. A direct
resource API create may specify them only if the caller is an authorized Nix emitter or a
controller principal. All other creates are rejected.

### Status schema

| Field | Type | Required | Default | Bound | Description |
| --- | --- | --- | --- | --- | --- |
| `status.observedGeneration` | `u64` | Yes | `0` | Monotonic | Last reconciled metadata generation. |
| `status.phase` | `enum` | Yes | `Pending` | `Pending|Ready|Succeeded|Degraded|Failed|Deleted|Unknown` | Common phase. Sessions may use all common phases except `Unknown` in normal flows. |
| `status.detail.kind` | `enum` | Yes | `PendingCreate` | See detail table | Typed lifecycle or failure detail. |
| `status.detail.message` | `string` | No | Empty | redacted, max 256 bytes | Operator-readable message without terminal bytes, usernames, PIDs, paths, or session names. |
| `status.conditions` | `[]Condition` | Yes | `[]` | closed provider-defined set | Boolean lifecycle indicators. |
| `status.supervisorRef` | `ResourceRef` | No | None | `Process/<name>` | Exact supervisor `Process` resource. |
| `status.supervisorGeneration` | `u64` | Yes | `0` | Monotonic | Per-session generation used in routing and stale-handle rejection. |
| `status.attachCount` | `u32` | Yes | `0` | `0..8` | Current number of attached terminal streams. |
| `status.outputRingBytes` | `u64` | Yes | `0` | `0..1048576` | Current ring fill in bytes. |
| `status.outputRingEvictedBytes` | `u64` | Yes | `0` | Monotonic | Total bytes evicted from the ring since session start. |
| `status.lastAttachedAt` | `RFC3339 timestamp` | No | None | valid timestamp | Most recent successful attach. |
| `status.lastExitedAt` | `RFC3339 timestamp` | No | None | valid timestamp | Set when the login shell exits. |
| `status.completedReason` | `enum` | No | None | `clean-exit|nonzero-exit|killed|supervisor-lost|operator-stop` | Terminal summary. |

`loginShellPid` is intentionally omitted from status. The supervisor owns the PTY and the
login shell, but PID values never cross the resource, audit, or telemetry boundary.
No terminal, clipboard, or notification bytes, secrets, paths, PIDs, unit names,
or authority-conferring handles appear in any status layer; terminal bytes stay
in process memory and named streams only.

D091 currency and upgrade: the shell-terminal controller implements
`assess_update`, `plan_upgrade`, and `execute_upgrade` for its qualified
ResourceTypes and semantic shell sessions. A `ProviderGenerationChanged`,
`ArtifactChanged`, `DependencyChanged`, or `SpecChanged` reason populates
universal `status.update` with
`UpdateAvailable` or `UpgradeRequired`; disruptive changes MUST return
`UpgradeRequired` rather than being applied in place, while non-disruptive
changes reconcile normally. These currency fields are universal/ResourceType
base fields, never `status.provider`. Upgrades recycle only the shell realization (owned
`Process` resources, endpoints, supervisors, and sessions) with `disruption`
set to `Reload`, `Restart`, or `Recycle`; durable config is preserved, dependent
sessions and attachments are drained and restarted by the dependency-aware
planner, and owned ephemeral session state remains process memory. No terminal
bytes, session bytes, clipboard bytes, notification content, secrets, paths, or
handles may appear in `status.update`.

D090 expedited reconcile: Create, UpdateSpec, and Delete requests that set
`waitForReconcile` perform no external effect, finalizer mutation, or status
mutation until Core supplies a typed `CommittedRevisionProof`
`{resourceUid,generation,revision,operationId}`. Abort produces no effect; a
durable commit is never rolled back on later reconcile timeout. The response is
the committed object plus one-pass projected layered status, a disposition
(`Converged`, `Progressing`, `Blocked`, `UpgradeRequired`, or `Failed`), and
`statusPersistence` (`pending` or `committed`); effect idempotency keys derive
from `(UID,generation,revision,operationId)` in the same per-resource
single-flight priority lane.

### `status.detail.kind` enum

| Value | Meaning |
| --- | --- |
| `PendingCreate` | The session resource exists but the supervisor `Process` has not yet been created. |
| `Initializing` | The supervisor exists and is starting, but readiness and route registration are incomplete. |
| `RouteRegistrationPending` | The supervisor is ready, but the d2b-bus route is not yet registered. |
| `ReadyDetached` | The session is ready and no client is attached. |
| `ReadyAttached` | The session is ready and at least one client is attached. |
| `Stopped` | The session has `desiredLifecycle=stopped` and the supervisor has been intentionally drained. |
| `ExitedCleanly` | The login shell exited with a success status; `status.phase=Succeeded`. |
| `ExitedError` | The login shell exited nonzero or due to a fatal runtime error; `status.phase=Failed`. |
| `SupervisorLost` | Controller re-verification failed or the supervisor disappeared; `status.phase=Degraded`. |
| `SupervisorAmbiguity` | Multiple candidate processes matched the expected slot; `status.phase=Degraded`. |
| `Deleting` | Finalization is in progress. |
| `Deleted` | Deletion completed. |

### Conditions

| Condition | True when | False/Unknown when |
| --- | --- | --- |
| `Ready` | The supervisor is verified, the route is registered, and the session is available for attach. | Any create, adopt, degrade, stop, or delete operation is incomplete. |
| `SupervisorReady` | The supervisor `Process` reports ready and matches the expected identity tuple. | The supervisor is absent, not ready, or mismatched. |
| `RouteRegistered` | The controller has an active d2b-bus route for the exact session generation. | Route registration has not completed or was invalidated. |
| `Attached` | `status.attachCount > 0`. | `attachCount == 0`. |
| `TerminalExited` | The login shell has exited and the supervisor reported terminal completion. | The login shell is still running or exit state is unknown. |
| `Deleting` | Metadata deletion timestamp is present and the finalizer is active. | The resource is not deleting. |
| `Degraded` | The session cannot currently guarantee identity-safe attach or kill behavior. | The session is healthy or terminally deleted. |

### Reconcile algorithm

1. Observe metadata generation, desired lifecycle, and deletion intent.
2. Resolve `spec.poolRef` and verify that the parent pool is `Ready` or `Degraded` with a
   recoverable cause.
3. Validate the inherited `spec.executionRef`, `spec.userRef`, and
   `spec.loginShellRef` against the parent pool snapshot.
4. Enforce pool capacity for total sessions.
5. Compute the canonical supervisor name from the session UID short form:
   `shell-terminal--supervisor--<session-uid-short>`.
6. If the session lacks a supervisor `Process`, create one using the canonical
   user-domain template.
7. Wait for the supervisor readiness condition.
8. On readiness, retrieve the supervisor InvocationID and component generation.
9. Compute or increment `status.supervisorGeneration`.
10. Register the d2b-bus route keyed by Zone, service, target session ref, stream or
    method, schema fingerprint, and `status.supervisorGeneration`.
11. Publish `status.supervisorRef`, ring metrics, and attach count.
12. If the login shell exits cleanly, set `status.phase=Succeeded` and
    `status.detail.kind=ExitedCleanly`.
13. If the login shell exits nonzero, or the supervisor crashes, set `status.phase=Failed`
    or `status.phase=Degraded` according to whether the controller can still prove the
    terminal outcome.
14. If `spec.desiredLifecycle=stopped`, request a graceful supervisor drain, invalidate
    the bus route, and keep the session retained in the store until explicit deletion.
15. On deletion, invalidate the route before any stop request, then stop the supervisor,
    then remove the finalizer.

### Error catalog

| Code | Phase | Retryable | Description |
| --- | --- | --- | --- |
| `SessionPoolNotFound` | `Pending` | Yes | The referenced pool is absent. |
| `SessionPoolNotReady` | `Pending` | Yes | The pool is present but not currently admitting sessions. |
| `SessionCapacityExceeded` | `Pending` | Yes | The parent pool has reached `maxSessions`. |
| `AttachedCapacityExceeded` | `Ready` | Yes | The parent pool has reached `maxAttached`; attach requests fail closed. |
| `SessionNameInvalid` | `Pending` | No | The supplied `spec.sessionName` fails kebab-case or length validation. |
| `SessionOutputRingCapacityOutOfRange` | `Pending` | No | The requested ring capacity is invalid or exceeds the pool limit. |
| `SessionInheritedSpecMismatch` | `Pending` | No | Stored inherited fields do not match the pool snapshot. |
| `SupervisorCreateFailed` | `Initializing` | Yes | The controller could not create the supervisor `Process`. |
| `SupervisorReadyTimeout` | `Initializing` | Yes | The supervisor did not report readiness before timeout. |
| `RouteRegistrationFailed` | `Initializing` | Yes | The controller could not register the bus route for the supervisor generation. |
| `SupervisorLost` | `Degraded` | No | The controller could not re-verify the expected supervisor identity. |
| `SupervisorAmbiguity` | `Degraded` | No | Multiple candidate processes matched the expected slot. |
| `StaleSessionGeneration` | `Ready` | No | The client presented an old generation during attach, detach, detach-all, or kill. |
| `SessionDeleteFailed` | `Deleting` | Yes | Supervisor stop or route invalidation did not complete. |

### Finalizer steps

1. Mark the session `Deleting` condition true.
2. Invalidate the d2b-bus route for the current `status.supervisorGeneration`.
3. Send a graceful stop to the supervisor only if the identity tuple still matches.
4. If the supervisor cannot be uniquely verified, do not guess; mark the session degraded
   and require explicit operator deletion or recreation.
5. Wait `drainTimeout` for an orderly supervisor exit.
6. Remove the owned supervisor `Process` if it still exists and still names the session as
   its `ownerRef`.
7. Clear `status.supervisorRef`, `status.attachCount`, and ring counters.
8. Remove the finalizer and allow the resource to reach `status.phase=Deleted`.

## Process templates

This provider uses two canonical `Process` templates only: one controller and one
per-session supervisor. The controller template is emitted by the core ProviderDeployment
handler. The supervisor template is emitted by the shell-terminal controller when it
creates a `shell-terminal.d2bus.org.ShellSession`.

### Controller `Process` template

```yaml
apiVersion: resources.d2bus.org/v3
type: Process
metadata:
  name: shell-terminal--controller
  zone: <zone>
  ownerRef: Provider/shell-terminal
spec:
  providerRef: Provider/system-systemd
  executionRef: Host/<host>
  domain: system
  processClass: controller
  template: shell-terminal-controller
  sandbox:
    seccompClass: strict
    noNewPrivileges: true
    startRoot: false
    readOnlyRoot: true
    namespaceClasses: [mount, ipc, uts, network]
    capabilityClasses: []
    environmentClass: minimal
  budget:
    memory: { limit: "128Mi" }
    pids:  { limit: 512 }
    fds:   { limit: 2048 }
  restartPolicy:
    class: on-failure
    backoffBase: "1s"
    backoffMax: "60s"
    backoffMultiplier: 2.0
    maxRestarts: null
    resetAfter: "300s"
  readiness:
    initialDelay: "0s"
    timeout: "30s"
    class: ready-condition
  adoptionPolicy: adopt-on-restart
  drainTimeout: "30s"
  desiredLifecycle: running
```

### Session supervisor `Process` template

```yaml
apiVersion: resources.d2bus.org/v3
type: Process
metadata:
  name: shell-terminal--supervisor--<session-uid-short>
  zone: <zone>
  ownerRef: shell-terminal.d2bus.org.ShellSession/<session-name>
spec:
  providerRef: Provider/system-systemd
  executionRef: Guest/<guest>   # or Host/<host> for Host pools
  domain: user
  userRef: User/<pool-user>     # from shell-terminal.d2bus.org.ShellPool spec.userRef
  processClass: service            # terminates a private ComponentSession endpoint; no ResourceClient/CLI portal
  template: shell-session-supervisor
  sandbox:
    seccompClass: strict
    noNewPrivileges: true
    startRoot: false
    readOnlyRoot: true
    namespaceClasses: []          # inherits user session cgroup/UTS; no extra NS
    capabilityClasses: []
    environmentClass: provider-defined
  budget:
    memory: { limit: "64Mi" }
    pids:   { limit: 32 }
    fds:    { limit: 256 }
  restartPolicy:
    class: never                  # crash = session terminal; no auto-restart
  readiness:
    initialDelay: "0s"
    timeout: "10s"
    class: ready-condition
  adoptionPolicy: adopt-on-restart
  drainTimeout: "5s"
  desiredLifecycle: running
```

### Canonical Nix form

```nix
d2b.zones.dev.resources.shell-terminal-controller = {
  type = "Process";
  spec = {
    providerRef = "Provider/system-systemd";
    executionRef = "Host/control";
    domain = "system";
    processClass = "controller";
    template = "shell-terminal-controller";
    sandbox = {
      seccompClass = "strict";
      noNewPrivileges = true;
      startRoot = false;
      readOnlyRoot = true;
      namespaceClasses = [ "mount" "ipc" "uts" "network" ];
      capabilityClasses = [ ];
      environmentClass = "minimal";
    };
    budget = {
      memory.limit = "128Mi";
      pids.limit = 512;
      fds.limit = 2048;
    };
    restartPolicy = {
      class = "on-failure";
      backoffBase = "1s";
      backoffMax = "60s";
      backoffMultiplier = 2.0;
      maxRestarts = null;
      resetAfter = "300s";
    };
    readiness = {
      initialDelay = "0s";
      timeout = "30s";
      class = "ready-condition";
    };
    adoptionPolicy = "adopt-on-restart";
    drainTimeout = "30s";
    desiredLifecycle = "running";
  };
};
```

```nix
d2b.zones.dev.resources.shell-session-supervisor-example = {
  type = "Process";
  spec = {
    providerRef = "Provider/system-systemd";
    executionRef = "Guest/work";
    domain = "user";
    userRef = "User/alice";
    processClass = "service";  # dynamic service: terminates shell-session-supervisor.v1 endpoint only
    template = "shell-session-supervisor";
    sandbox = {
      seccompClass = "strict";
      noNewPrivileges = true;
      startRoot = false;
      readOnlyRoot = true;
      namespaceClasses = [ ];
      capabilityClasses = [ ];
      environmentClass = "provider-defined";
    };
    budget = {
      memory.limit = "64Mi";
      pids.limit = 32;
      fds.limit = 256;
    };
    restartPolicy.class = "never";
    readiness = {
      initialDelay = "0s";
      timeout = "10s";
      class = "ready-condition";
    };
    adoptionPolicy = "adopt-on-restart";
    drainTimeout = "5s";
    desiredLifecycle = "running";
  };
};
```

```nix
d2b.zones.dev.resources.shell-terminal-service-endpoint = {
  type = "Endpoint";
  metadata.ownerRef = "Provider/shell-terminal";
  spec = {
    providerRef = "Provider/shell-terminal";
    producerRef = "Process/shell-terminal--controller";
    endpointClass = "service";
    transport = "unix";
    purpose = "shell-terminal.d2bus.org/controller-service";
    serviceFingerprint = "shell-terminal.d2bus.org/shell-terminal.v3";
    locality = "host-local";
    visibility = "zone";
    attachmentPolicy = "component-session";
    consumerPolicy = {
      allowedSubjects = [ "User/alice" ];
      allowedOperations = [ "resolve" ];
    };
    lifecyclePolicy = "recycle-with-producer";
  };
};
```

```nix
d2b.zones.dev.resources.shell-session-supervisor-endpoint = {
  type = "Endpoint";
  metadata.ownerRef = "shell-terminal.d2bus.org.ShellSession/example";
  spec = {
    providerRef = "Provider/shell-terminal";
    producerRef = "Process/shell-terminal--supervisor--<session-uid-short>";
    endpointClass = "service";
    transport = "unix";
    purpose = "shell-terminal.d2bus.org/session-supervisor";
    serviceFingerprint = "shell-terminal.d2bus.org/shell-session-supervisor.v1";
    locality = "guest-local";
    visibility = "zone";
    attachmentPolicy = "component-session";
    consumerPolicy = {
      allowedSubjects = [ "User/alice" ];
      allowedOperations = [ "resolve" ];
    };
    lifecyclePolicy = "recycle-with-producer";
  };
};
```

## Endpoint resources (D092)

`Provider/shell-terminal` declares standard `Endpoint` base-schema conformance.
Stable controller and shell-supervisor service identities are owned `Endpoint`
resources with `producerRef`; they are not inline `Process.spec` or Nix Process
fields. Consumers use `Endpoint/<name>` references. Endpoint spec/status/CLI/
audit/telemetry never include raw socket paths, PTY bytes, terminal output,
argv, environment, fds, or credentials. Resolution occurs only through an
authorized EffectPort/LaunchTicket; unauthorized resolution returns
`endpoint-resolve-denied`. Producer restart bumps
`Endpoint.status.endpointGeneration`, which triggers `dependency-changed` for
consumers.

## Retained opaque handles

- pidfds: Process supervision handles and not stable service identities.
- Per-connection/session handles: ShellSession handles, supervisor handles, and
  attach IDs are high-churn and scoped to one session generation.
- Named streams: terminal byte streams carry payload and attachment state behind
  authorization; they are not Endpoint identities.
- `OwnedTransport`: authenticated ComponentSession transport ownership remains
  an in-memory capability.
- fd indexes: PTY, stream, and pre-opened service descriptors are
  LaunchTicket-local slots and stay opaque under D092.

### YAML `Process` annotations field table

The canonical YAML templates above intentionally omit free-form annotations. The provider
must not depend on arbitrary annotations for correctness. If an implementation emits
optional `metadata.annotations`, only the closed keys below are permitted.

| Field | Type | Required | Default | Bound | Description |
| --- | --- | --- | --- | --- | --- |
| `metadata.annotations.d2bus.org/provider-role` | `string` | No | None | `shell-terminal-controller|shell-session-supervisor` | Human-stable template role marker. |
| `metadata.annotations.d2bus.org/provider-service` | `string` | No | None | `shell-terminal.v3|shell-session-supervisor.v1` | Declared primary service purpose. |
| `metadata.annotations.d2bus.org/routing-class` | `string` | No | None | `controller|session-supervisor` | Routing ownership class. |
| `metadata.annotations.d2bus.org/redaction-class` | `string` | No | None | `shell-terminal` | Indicates the shell-terminal redaction profile. |
| `metadata.annotations.d2bus.org/isolation-warning` | `string` | No | None | `host-none` | Optional warning tag for Host pools with `isolationPosture=none`. |

## ProviderStateSet

Per D087, `shell-terminal` declares **no Provider state Volume**. A
**ProviderStateSet** is the optional query-time set of declared Provider state
Volumes in a Zone whose `metadata.ownerRef` resolves to `Provider/shell-terminal`;
for this Provider the set is empty.

```text
ProviderStateSet(zone, "shell-terminal") = {}
```

The controller has no `/state` mount and no Volume lifecycle gate. The
session-supervisor has no Provider state Volume either; it owns live PTY fds, the
login-shell process tree, the bounded merged-output ring, and attachment state in
process memory only. Those bytes and handles are transient authority-bearing or
private runtime data and must never be persisted in a Provider state Volume,
resource status, logs, audit, or metrics.

Bounded non-secret operational state belongs in the owning resource status and
the core Operation ledger:

- `ShellPool.status` carries phase, conditions, capacity counters, target/user
  verification, warning details, and aggregate attachment counts.
- `ShellSession.status` carries supervisor reference, generation, phase,
  conditions, attach count, bounded ring counters, and adoption observations.
- Operation rows record lifecycle transitions, attach/detach/kill requests, and
  finalizer progress.

Status writes are revisioned, optimistic-status-writer controlled, RBAC-readable,
redacted, observation-only, and written only on material change. After restart,
the controller re-lists resources, verifies supervisor Process identity and
ComponentSession route reality, then republishes bounded status instead of
recovering from a private file.

Storage-need test rationale: shell-terminal has no durable secret recovery
payload, no large file content, no private data safe only outside authorized
status readers, and no bounded-but-revision-unsuitable data with a demonstrated
recovery need. PTY/ring/attach data is live process state, not durable Provider
state, so the ProviderStateSet remains empty.

## ComponentSession contracts


The provider defines two ComponentSession services.

| Service | Hosted by | Target resource | Purpose | Noise profile |
| --- | --- | --- | --- | --- |
| `shell-terminal.v3` | `Process/shell-terminal--controller` | `Provider/shell-terminal` | public lifecycle service for pools and sessions | KK or stronger per platform policy |
| `shell-session-supervisor.v1` | `Process/shell-terminal--supervisor--<session-uid-short>` | `shell-terminal.d2bus.org.ShellSession/<name>` | private per-session service for attach, detach, detach-all, kill, and status | KK |

### Public controller service: `shell-terminal.v3`

| Method | Target | Description | Required role |
| --- | --- | --- | --- |
| `OpenSession` | `shell-terminal.d2bus.org.ShellPool/<name>` | Create a new `shell-terminal.d2bus.org.ShellSession`, create its supervisor, and return the route data needed to attach. | `Role/shell-admin` or Zone-admin superset |
| `ListSessions` | `shell-terminal.d2bus.org.ShellPool/<name>` | List child session summaries for one pool. | `Role/shell-admin` or Zone-admin superset |
| `PoolStatus` | `shell-terminal.d2bus.org.ShellPool/<name>` | Return aggregate pool counts, phase, and warning state. | `Role/shell-admin` or Zone-admin superset |

#### `OpenSession` request

| Field | Type | Required | Default | Bound | Description |
| --- | --- | --- | --- | --- | --- |
| `poolRef` | `ResourceRef` | Yes | None | `shell-terminal.d2bus.org.ShellPool/<name>` | Target pool. |
| `sessionName` | `string` | No | controller-generated | kebab-case, max 32 bytes | Optional operator-friendly name copied into `spec.sessionName`. |
| `outputRingCapacity` | `u64` bytes | No | pool default | `4096..1048576` and `<= pool limit` | Optional session-specific ring size. |
| `attachImmediately` | `bool` | No | `true` | Fixed | Whether the caller plans to open the supervisor stream right away. |
| `terminalRows` | `u16` | No | `24` | `1..4096` | Initial PTY row hint. |
| `terminalCols` | `u16` | No | `80` | `1..4096` | Initial PTY column hint. |

#### `OpenSession` response

| Field | Type | Required | Default | Bound | Description |
| --- | --- | --- | --- | --- | --- |
| `sessionRef` | `ResourceRef` | Yes | None | `shell-terminal.d2bus.org.ShellSession/<name>` | Created session resource. |
| `supervisorRef` | `ResourceRef` | Yes | None | `Process/<name>` | Created supervisor process resource. |
| `supervisorGeneration` | `u64` | Yes | None | Monotonic | Generation that must accompany `Attach`, `Detach`, `DetachAll`, and `Kill`. |
| `service` | `string` | Yes | `shell-session-supervisor.v1` | Exact | Per-session service name. |
| `routeZone` | `ResourceName` | Yes | None | Existing Zone | K0 routing Zone for the supervisor. |
| `phase` | `enum` | Yes | None | Common phase | Initial session phase after creation. |

#### `ListSessions` response summary item

| Field | Type | Required | Default | Bound | Description |
| --- | --- | --- | --- | --- | --- |
| `sessionRef` | `ResourceRef` | Yes | None | `shell-terminal.d2bus.org.ShellSession/<name>` | Child session resource reference. |
| `phase` | `enum` | Yes | None | Common phase | Current session phase. |
| `detailKind` | `enum` | Yes | None | closed session detail set | Current typed detail kind. |
| `supervisorGeneration` | `u64` | Yes | None | Monotonic | Current attach generation. |
| `attachCount` | `u32` | Yes | None | `0..8` | Current live attachments. |
| `outputRingBytes` | `u64` | Yes | None | `0..1048576` | Current ring fill. |
| `outputRingEvictedBytes` | `u64` | Yes | None | Monotonic | Bytes evicted so far. |

#### `PoolStatus` response

| Field | Type | Required | Default | Bound | Description |
| --- | --- | --- | --- | --- | --- |
| `poolRef` | `ResourceRef` | Yes | None | `shell-terminal.d2bus.org.ShellPool/<name>` | Pool reference. |
| `phase` | `enum` | Yes | None | Common phase | Pool phase. |
| `detailKind` | `enum` | Yes | None | closed pool detail set | Pool detail kind. |
| `activeSessions` | `u32` | Yes | None | `0..64` | Current child-session count. |
| `attachedSessions` | `u32` | Yes | None | `0..8` | Current attached-session count. |
| `capacityRemaining` | `u32` | Yes | None | `0..64` | Remaining session capacity. |
| `attachedCapacityRemaining` | `u32` | Yes | None | `0..8` | Remaining attach capacity. |
| `isolationPosture` | `enum` | Yes | None | `isolated|none` | Execution-target posture. |

#### Public service error catalog

| Code | Phase | Retryable | Description |
| --- | --- | --- | --- |
| `NotAuthorized` | `Ready` | No | Caller lacks `Role/shell-admin` or a Zone-admin superset. |
| `RelayHostUserDomainDenied` | `Ready` | No | A relay-authenticated identity attempted Host user-domain access. |
| `PoolNotFound` | `Ready` | No | The requested pool does not exist. |
| `PoolNotReady` | `Ready` | Yes | The pool is not in a state that can admit session creation. |
| `SessionCapacityExceeded` | `Ready` | Yes | The pool has reached `maxSessions`. |
| `AttachedCapacityExceeded` | `Ready` | Yes | The pool has reached `maxAttached`. |
| `SessionNameInvalid` | `Ready` | No | The requested session display name is invalid. |
| `LoginShellArtifactMissing` | `Ready` | No | The pool login shell artifact cannot be resolved. |
| `OpenSessionTimedOut` | `Ready` | Yes | Supervisor creation or readiness did not complete in time. |

### Session supervisor service: `shell-session-supervisor.v1`

| Method or stream | Target | Description | Required role |
| --- | --- | --- | --- |
| `Attach` | `shell-terminal.d2bus.org.ShellSession/<name>` | Open a bidirectional terminal stream against the exact supervisor generation. | `Role/shell-admin` or Zone-admin superset |
| `Detach` | `shell-terminal.d2bus.org.ShellSession/<name>` | Detach the caller's current stream without killing the shell. | `Role/shell-admin` or Zone-admin superset |
| `DetachAll` | `shell-terminal.d2bus.org.ShellSession/<name>` | Detach all current streams from the exact session without killing the shell. | `Role/shell-admin` or Zone-admin superset |
| `Kill` | `shell-terminal.d2bus.org.ShellSession/<name>` | Terminate the exact session scope owned by the supervisor. | `Role/shell-admin` or Zone-admin superset |
| `SupervisorStatus` | `shell-terminal.d2bus.org.ShellSession/<name>` | Return redacted session status from the exact supervisor generation. | `Role/shell-admin` or Zone-admin superset |

#### `Attach` request

| Field | Type | Required | Default | Bound | Description |
| --- | --- | --- | --- | --- | --- |
| `sessionRef` | `ResourceRef` | Yes | None | `shell-terminal.d2bus.org.ShellSession/<name>` | Target session reference. |
| `expectedSupervisorGeneration` | `u64` | Yes | None | current generation | Fail closed if stale. |
| `tailBytes` | `u64` bytes | No | `65536` | `0..outputRingCapacity` | Ring tail to replay before live output. |
| `terminalRows` | `u16` | No | current PTY value | `1..4096` | Optional resize hint on attach. |
| `terminalCols` | `u16` | No | current PTY value | `1..4096` | Optional resize hint on attach. |

#### `Attach` response

| Field | Type | Required | Default | Bound | Description |
| --- | --- | --- | --- | --- | --- |
| `acceptedSupervisorGeneration` | `u64` | Yes | None | current generation | Echo of the generation admitted for the stream. |
| `ringBytesReplayed` | `u64` | Yes | `0` | `0..outputRingCapacity` | Tail bytes delivered before live output. |
| `streamName` | `string` | Yes | `terminal` | Exact | Named bidirectional terminal stream. |

#### `Detach`, `DetachAll`, `Kill`, and `SupervisorStatus` requests

| Field | Type | Required | Default | Bound | Description |
| --- | --- | --- | --- | --- | --- |
| `sessionRef` | `ResourceRef` | Yes | None | `shell-terminal.d2bus.org.ShellSession/<name>` | Target session. |
| `expectedSupervisorGeneration` | `u64` | Yes | None | current generation | Mandatory stale-handle protection field. |
| `reason` | `enum` | No | `operator-request` | `operator-request|stream-close|maintenance` | Redacted reason code for detach or kill. |
| `graceTimeoutMs` | `u32` | No | `5000` | `0..60000` | Kill grace period. Ignored by `Detach` and `SupervisorStatus`. |

#### `SupervisorStatus` response

| Field | Type | Required | Default | Bound | Description |
| --- | --- | --- | --- | --- | --- |
| `sessionRef` | `ResourceRef` | Yes | None | `shell-terminal.d2bus.org.ShellSession/<name>` | Target session. |
| `supervisorGeneration` | `u64` | Yes | None | Monotonic | Current generation. |
| `phase` | `enum` | Yes | None | Common phase | Current session phase. |
| `detailKind` | `enum` | Yes | None | closed detail set | Current session detail kind. |
| `attachCount` | `u32` | Yes | None | `0..8` | Current attachments. |
| `outputRingBytes` | `u64` | Yes | None | `0..1048576` | Current ring fill. |
| `outputRingEvictedBytes` | `u64` | Yes | None | Monotonic | Cumulative ring eviction count. |

#### Supervisor service error catalog

| Code | Phase | Retryable | Description |
| --- | --- | --- | --- |
| `NotAuthorized` | `Ready` | No | Caller lacks `Role/shell-admin` or a Zone-admin superset. |
| `RelayHostUserDomainDenied` | `Ready` | No | Relay-authenticated identities may not target Host user-domain supervisors. |
| `SessionNotReady` | `Ready` | Yes | The session is not in a state that can be attached. |
| `AttachedCapacityExceeded` | `Ready` | Yes | Pool-wide attach capacity has been reached. |
| `StaleSessionGeneration` | `Ready` | No | The request generation is older than `status.supervisorGeneration`. |
| `SessionAlreadyTerminal` | `Ready` | No | The login shell has already exited and the session is terminal. |
| `StreamAlreadyDetached` | `Ready` | No | The targeted stream was already detached. |
| `KillAlreadyInProgress` | `Ready` | Yes | A kill or drain operation is already executing. |
| `SupervisorIdentityMismatch` | `Ready` | No | The controller or bus route no longer maps to the expected supervisor identity. |

### Named terminal stream

The terminal stream is a bidirectional named ComponentSession stream named `terminal`.
It is not a generic byte pump held by the controller.

Flow:

1. The client calls `OpenSession` on `shell-terminal.v3`.
2. The controller creates the session and its supervisor, then returns `sessionRef`,
   `supervisorRef`, K0 route Zone, and `supervisorGeneration`.
3. The client opens a direct ComponentSession to `shell-session-supervisor.v1`.
4. The d2b-bus resolves the route by K0 Zone,
   `shell-terminal.d2bus.org.ShellSession/<name>`, service `shell-session-supervisor.v1`, named
   stream `terminal`, schema fingerprint, and `supervisorGeneration`.
5. The supervisor replays the requested tail from its in-memory ring, then switches to
   live PTY I/O.
6. Closing or detaching the terminal stream never kills the shell. `Kill` is a distinct
   typed method and targets only the exact verified supervisor generation.

| Frame | Direction | Description |
| --- | --- | --- |
| `Open` | client -> supervisor | Identifies the expected generation, requested ring-tail bytes, and optional terminal geometry. |
| `Input` | client -> supervisor | Opaque PTY input bytes. |
| `Resize` | client -> supervisor | Terminal rows and columns. |
| `Detach` | client -> supervisor | Voluntary detach for the current stream only. |
| `CreditGrant` | bidirectional | Byte credit update for backpressure. |
| `TailChunk` | supervisor -> client | Replay data from the in-memory ring before live output. |
| `OutputChunk` | supervisor -> client | Live merged stdout and stderr bytes. |
| `Detached` | supervisor -> client | Acknowledges a detach without killing the shell. |
| `Exit` | supervisor -> client | Signals login-shell termination and terminal outcome. |

Credit flow is mandatory. The sender may not exceed outstanding byte credit. Initial
credit is implementation-defined but must be bounded and documented. A conforming baseline
uses 64 KiB initial credit with bounded grant increments.

### Provider routing and d2b-bus resolution

| Routing dimension | Value |
| --- | --- |
| Zone | K0 Zone where the supervisor `Process` runs. |
| Target resource | `shell-terminal.d2bus.org.ShellSession/<name>` |
| Service | `shell-session-supervisor.v1` |
| Method or stream | `Attach`, `Detach`, `DetachAll`, `Kill`, `SupervisorStatus`, or `terminal` |
| Schema fingerprint | Versioned fingerprint of the service or stream schema. |
| Generation | `status.supervisorGeneration` of the target session. |

The controller never proxies PTY traffic. Once the route is registered, the bus sends the
client directly to the supervisor's enrolled KK endpoint. The controller participates only
in lifecycle: create session, create supervisor, register route, invalidate route, and
update resource status.

## PTY ownership and output ring

Each session supervisor owns exactly one PTY and exactly one login shell selected from the
manifest-fixed `spec.loginShellRef` artifact. The PTY master file descriptor lives only in
supervisor process memory. The controller never holds the PTY master, never buffers output
bytes, and never replays terminal data.

| Property | Value |
| --- | --- |
| PTY owner | Exactly one `d2b-shell-session-supervisor` process per session. |
| Login shell selection | Resolved from `spec.loginShellRef`; no caller-controlled path, argv, cwd, or environment. |
| Output channel model | Merged stdout plus stderr byte stream. |
| Ring location | Supervisor process memory only. |
| Ring default capacity | `262144` bytes (256 KiB). |
| Ring maximum capacity | `1048576` bytes (1 MiB). |
| Overflow policy | Evict oldest bytes and increment `status.outputRingEvictedBytes`. |
| Attach replay model | Deliver last `N` bytes from the ring, then switch to live output. |
| Seek support | No seek beyond current bounded ring contents. |

### Ring status publication

| Field | Meaning |
| --- | --- |
| `status.outputRingBytes` | Current bytes resident in the ring. |
| `status.outputRingEvictedBytes` | Total bytes evicted since session start. |
| `status.attachCount` | Current number of attached clients. |

### Redaction invariants for PTY and ring data

- terminal bytes never appear in audit events
- terminal bytes never appear in OTEL span attributes
- terminal bytes never appear in structured logs
- ring capacity and byte counts may appear as numeric counters
- PTY descriptors and shell-process identifiers never leave the supervisor-control path

## K0/K1/K2/self one-session identity and stale-generation rejection

Each session supervisor has one canonical identity tuple:

- Zone UID of the K0 Zone where the supervisor runs
- `shell-terminal.d2bus.org.ShellSession` UID
- Process supervisor generation, defined as the stored `status.supervisorGeneration`
  plus the currently verified `Process` generation and InvocationID binding

The externally visible generation is the monotonic `status.supervisorGeneration` u64. The
controller increments it every time a new supervisor becomes authoritative for a session,
including a fresh create or a verified post-restart adoption.

| Symbol | Meaning |
| --- | --- |
| `K0` | The Zone in which the session supervisor actually runs. |
| `K1` | The immediately enclosing or requesting Zone from which an attach is initiated. |
| `K2+` | Any outer routing hop vocabulary reserved by zone routing, not by shell-terminal logic. |
| `self` | The supervisor's own enrolled KK identity for `shell-session-supervisor.v1`. |

A client attach handle is valid only for one session generation. If a client presents a
lower generation than `status.supervisorGeneration`, the supervisor returns
`StaleSessionGeneration` and the bus route remains invalid for the stale generation. There
is no silent re-adoption of old handles. There is no automatic translation from a stale
handle to a new supervisor. The client must obtain a fresh handle from `OpenSession` or a
fresh `ListSessions` or `SupervisorStatus` read.

## Scope adoption and degraded-ambiguity rules

Controller restart is a continuation event, not a recreation event. The controller must
scan existing supervisor resources and re-register routes only after strict identity
verification.

### Restart adoption algorithm

1. List all `Process/shell-terminal--supervisor--*` resources in the Zone store.
2. For each candidate, require `ownerRef` to point at a `shell-terminal.d2bus.org.ShellSession/<name>`.
3. Resolve the owning session resource.
4. Verify the process name derived from the session UID short form.
5. Query provider and runtime identity evidence for the candidate process.
6. Verify InvocationID, cgroup leaf, Process generation, and supervisor-generation
   binding.
7. Require the session status to bind the same `supervisorRef` and a monotonic
   `status.supervisorGeneration` record.
8. If verification succeeds uniquely, re-register the exact d2b-bus routes for the
   verified generation.
9. If the process is missing, mark the session degraded with `SupervisorLost`.
10. If more than one candidate could satisfy the slot, mark the session degraded with
    `SupervisorAmbiguity` and do not guess.
11. Never recreate a supervisor solely because controller memory was lost.
12. Never attach, detach, detach-all, or kill through an unverified process handle.

| Case | Required outcome |
| --- | --- |
| unique verified process | Re-register route and keep the session `Ready` or terminal. |
| missing process | Set `status.phase=Degraded`, `status.detail.kind=SupervisorLost`, invalidate routes, and require operator action. |
| multiple plausible processes | Set `status.phase=Degraded`, `status.detail.kind=SupervisorAmbiguity`, invalidate routes, and require deletion or recreation. |
| stale route generation | Reject with `StaleSessionGeneration`; do not auto-forward. |

### Stale handle rejection rule

`status.supervisorGeneration` is the only generation field exposed to clients. The
controller may internally combine Process generation and InvocationID, but any request
presenting a generation lower than the stored field must fail closed with
`StaleSessionGeneration`. Higher generations are also rejected because they cannot be
validly registered.

## Session lifecycle state machine

This provider uses common phases only. The lifecycle names `Initializing` and `Deleting`
are represented in `status.detail.kind`, while the resource phase stays within the common
catalog.

| Conceptual state | `status.phase` | `detail.kind` | Entry event | Exit event |
| --- | --- | --- | --- | --- |
| `Pending` | `Pending` | `PendingCreate` | Session resource created. | Supervisor `Process` created. |
| `Initializing` | `Pending` | `Initializing` or `RouteRegistrationPending` | Supervisor creation started. | Supervisor ready and route registered. |
| `Ready` | `Ready` | `ReadyDetached` or `ReadyAttached` | Supervisor verified and route registered. | Detach, exit, degradation, stop, or delete. |
| `Succeeded` | `Succeeded` | `ExitedCleanly` | Login shell exits zero. | Deletion or retention. |
| `Failed` | `Failed` | `ExitedError` | Login shell exits nonzero or kill is terminal. | Deletion or retention. |
| `Degraded` | `Degraded` | `SupervisorLost` or `SupervisorAmbiguity` | Identity-safe routing can no longer be guaranteed. | Operator delete or explicit recreation. |
| `Deleting` | `Pending` or `Degraded` | `Deleting` | Deletion timestamp observed. | Finalizer completes. |
| `Deleted` | `Deleted` | `Deleted` | Finalizer removed and store tombstones resource. | Terminal state. |

### Transition rules

| From | Trigger | To | Notes |
| --- | --- | --- | --- |
| `Pending` | Supervisor `Process` created | `Initializing` | Controller writes the supervisor ref and waits for readiness. |
| `Initializing` | Supervisor ready and route registered | `Ready` | Exact generation becomes attachable. |
| `Ready` | Attach count rises above zero | `Ready` | Phase remains `Ready`; detail changes to `ReadyAttached`. |
| `Ready` | Attach count falls to zero | `Ready` | Phase remains `Ready`; detail changes to `ReadyDetached`. |
| `Ready` | Login shell clean exit | `Succeeded` | Supervisor publishes terminal completion. |
| `Ready` | Login shell nonzero exit | `Failed` | Supervisor publishes terminal failure. |
| `Ready` | Identity mismatch or missing supervisor | `Degraded` | No recreation by guesswork. |
| `Ready` | `spec.desiredLifecycle=stopped` | `Pending` with `Stopped` detail or terminal phase | Controller drains, stops, and retains the resource. |
| `Succeeded` or `Failed` | Delete request | `Deleting` | Routes are invalidated before any stop. |
| `Degraded` | Delete request | `Deleting` | Finalizer attempts verified cleanup only. |
| `Deleting` | Finalizer success | `Deleted` | Terminal store tombstone. |

### Login-shell exit handling

When the login shell exits, the supervisor updates the owning session through the
controller-owned status path. Clean exit yields `status.phase=Succeeded`; nonzero exit or
fatal runtime error yields `status.phase=Failed`. The supervisor then enters its
`drainTimeout`, stops accepting new attaches, and allows finalization to run. The session
resource may remain retained for inspection until deletion.

### `desiredLifecycle=stopped`

Setting `spec.desiredLifecycle=stopped` requests a graceful drain of the exact supervisor.
The controller invalidates the route, sends a stop request to the verified supervisor, and
retains the session resource in the store with terminal or stopped detail until explicit
delete. The controller does not kill on stream close, and it does not infer stop from a
missing client connection.

## Host-specific rules

Host pools replace the unsafe-local shell successor path, but they remain explicitly
non-isolating. They therefore carry stronger warning and authorization rules than Guest
pools.

| Rule | Requirement |
| --- | --- |
| isolation posture warning | If `spec.executionRef` targets `Host/<name>` and the Host reports `isolationPosture=none`, the pool must set `status.detail.kind=IsolationPostureWarning`, set the `IsolationPostureWarning` condition true, and emit a pool-scoped audit event. |
| user domain only | All Host session supervisors run in `domain=user` under `Provider/system-systemd`, never as a disguised system-domain process. |
| same-UID rule | The supervisor must run as exactly the UID named by `spec.userRef`; the provider verifies this before spawn or adopt. |
| relay denial | Relay-authenticated identities are denied all user-domain Host shell access under SR-3. |
| no isolation claim | Host shell status and audit must describe the posture as `none`; no wording may imply a sandbox boundary. |
| no fallback | There is no SSH, no direct host exec, and no bypass path around `Provider/system-systemd`. |

The pool-scoped audit event for Host posture warning is emitted when the pool first becomes
ready and every time the posture changes from `isolated` to `none`. The event must use a
redacted resource UID digest and posture enum only; it must not include usernames,
resource names, paths, or process identifiers.

## Guest-specific rules

Guest pools use the Guest's verified user manager through `Provider/system-systemd`.
Guest-specific requirements are strict because the prior draft incorrectly modeled guest
supervisors as system-domain workers or as non-service Processes.

| Rule | Requirement |
| --- | --- |
| allowed domains | The referenced `Guest/<name>` must advertise `spec.allowedDomains` containing `user`. |
| default user | The referenced `Guest/<name>` must publish `spec.defaultUserRef`; the shell pool `spec.userRef` must match an allowed guest workload user. |
| placement path | The supervisor is placed as a transient user scope through the Guest user manager exposed by `Provider/system-systemd`. |
| login shell selection | The supervisor resolves the shell binary from `spec.loginShellRef` only. No caller-supplied shell path is accepted. |
| capacity defaults | Pool defaults honor the baseline `DEFAULT_SHELL_SESSIONS_PER_VM=8` and `DEFAULT_SHELL_ATTACHED_SESSIONS_PER_VM=1`. |
| independent guestd limits removed | Guest-side shell limits are provider-policy only; there is no second independent guestd session-limit authority. |

Guest pools may be created only when the Guest resource proves that user-domain processes
are supported. A guest lacking `user` in `allowedDomains` is rejected at pool admission
with `ExecutionTargetUserDomainUnsupported`.

## RBAC and authorization

All shell verbs are admin-only. There is no anonymous shell use, no viewer role, and no
relay-authenticated exception for user-domain targets.

| Role | Scope | Description |
| --- | --- | --- |
| `Role/shell-admin` | Zone-scoped | Minimum role required for all shell-terminal service methods. |
| Zone-admin superset | Zone-scoped or broader | Any role explicitly documented as a superset of `Role/shell-admin`. |

| Verb | Target | Required role | Relay-authenticated identity allowed? |
| --- | --- | --- | --- |
| `OpenSession` | `shell-terminal.d2bus.org.ShellPool/<name>` | `Role/shell-admin` or Zone-admin superset | No for Host user-domain pools; policy may permit only local admin for Guest pools as well. |
| `ListSessions` | `shell-terminal.d2bus.org.ShellPool/<name>` | `Role/shell-admin` or Zone-admin superset | No for Host user-domain pools. |
| `PoolStatus` | `shell-terminal.d2bus.org.ShellPool/<name>` | `Role/shell-admin` or Zone-admin superset | No for Host user-domain pools. |
| `Attach` | `shell-terminal.d2bus.org.ShellSession/<name>` | `Role/shell-admin` or Zone-admin superset | No for Host user-domain pools. |
| `Detach` | `shell-terminal.d2bus.org.ShellSession/<name>` | `Role/shell-admin` or Zone-admin superset | No for Host user-domain pools. |
| `DetachAll` | `shell-terminal.d2bus.org.ShellSession/<name>` | `Role/shell-admin` or Zone-admin superset | No for Host user-domain pools. |
| `Kill` | `shell-terminal.d2bus.org.ShellSession/<name>` | `Role/shell-admin` or Zone-admin superset | No for Host user-domain pools. |
| `SupervisorStatus` | `shell-terminal.d2bus.org.ShellSession/<name>` | `Role/shell-admin` or Zone-admin superset | No for Host user-domain pools. |

### Admission rules

1. Only controller principals and authorized Nix emitters may create or mutate stored
   `shell-terminal.d2bus.org.ShellSession` objects directly.
2. External operators use only `OpenSession`, `ListSessions`, `PoolStatus`, `Attach`,
   `Detach`, `DetachAll`, `Kill`, and `SupervisorStatus`.
3. A relay-authenticated subject is never sufficient for user-domain Host shell access.
4. Anonymous or unauthenticated subjects are denied all shell service verbs.
5. Authorization is checked before capacity and before route lookup so that stale-handle
   rejection never reveals resource existence to unauthorized callers.

## Security invariants

SR-1: `shell-terminal.d2bus.org.ShellPool` is capacity-only. It never creates a pool-wide supervisor,
never owns PTY state, and never owns attach streams.

SR-2: Every managed shell session owns exactly one user-domain supervisor `Process`
executing as the exact `User/<name>` named by the pool.

SR-3: Relay-authenticated identities are denied user-domain Host shell access. No relay
credential maps to local shell authority.

SR-4: The controller never owns the PTY master, output bytes, attach state, or shell PID.
Those remain exclusively in supervisor process memory.

SR-5: `status.phase` uses only the common phases. Type-specific lifecycle nuance appears
only in `status.detail` and conditions.

SR-6: A client attach handle is bound to one `status.supervisorGeneration`. Stale or
future generations fail closed with `StaleSessionGeneration`.

SR-7: Controller restart is a continuation event. Re-adoption requires exact identity
verification by InvocationID, cgroup leaf, Process generation, and session binding. No
recreation by guesswork is permitted.

SR-8: No terminal bytes, argv, cwd, environment, paths, PIDs, unit names, usernames,
session names, socket paths, or opaque handles may appear in Debug, audit, metrics, or
span attributes.

SR-9: There is no SSH path, no direct-host fallback path, and no bypass path around the
user-domain `Provider/system-systemd` placement model.

SR-10: Host shells are explicitly marked with `isolationPosture=none` warnings in status
and audit; Guest shells may rely only on the Guest execution boundary actually declared by
the target resource.

## Nix configuration

Nix declarations use qualified type names exactly as stored.

```nix
d2b.zones.dev.resources.guest-alice-shell = {
  type = "shell-terminal.d2bus.org.ShellPool";
  spec = {
    providerRef = "Provider/shell-terminal";
    executionRef = "Guest/work";
    userRef = "User/alice";
    loginShellRef = "artifact://shells/bash-login";
    maxSessions = 8;
    maxAttached = 1;
    outputRingCapacity = 262144;
  };
};
```

```nix
d2b.zones.dev.resources.guest-alice-shell-main = {
  type = "shell-terminal.d2bus.org.ShellSession";
  spec = {
    providerRef = "Provider/shell-terminal";
    poolRef = "shell-terminal.d2bus.org.ShellPool/guest-alice-shell";
    executionRef = "Guest/work";
    userRef = "User/alice";
    loginShellRef = "artifact://shells/bash-login";
    sessionName = "main";
    outputRingCapacity = 262144;
    desiredLifecycle = "running";
  };
};
```

```nix
d2b.zones.dev.resources.work = {
  type = "Guest";
  spec = {
    allowedDomains = [ "system" "user" ];
    defaultUserRef = "User/alice";
  };
};
```

The Nix compiler must reject:

- non-vendor-qualified pool or session type strings
- session declarations whose `poolRef` does not name `shell-terminal.d2bus.org.ShellPool/<name>`
- Guest targets lacking `allowedDomains = [ ... "user" ... ]`
- session `outputRingCapacity` values larger than the parent pool's configured bound

## OTEL / audit / telemetry

Telemetry is closed-label, redacted, and resource-name-free. `d2b.zone` is an OTEL
resource attribute, not a metric label.

### Metrics

| Metric name | Type | Label set | Description |
| --- | --- | --- | --- |
| `d2b_shell_pool_sessions` | Gauge | `execution_kind={host,guest}` | Current active session count across one pool. |
| `d2b_shell_pool_attached_sessions` | Gauge | `execution_kind={host,guest}` | Current attached-session count across one pool. |
| `d2b_shell_session_attach_count` | Gauge | `execution_kind={host,guest}` | Current attach count for one session, exported without session name or user identity. |
| `d2b_shell_session_ring_bytes` | Gauge | `execution_kind={host,guest}` | Current ring fill bytes. |
| `d2b_shell_session_ring_evicted_bytes_total` | Counter | `execution_kind={host,guest}` | Total bytes evicted from session rings. |
| `d2b_shell_reconcile_total` | Counter | `resource_kind={pool,session}`, `outcome={success,retryable_error,terminal_error}` | Reconcile loop outcomes. |
| `d2b_shell_attach_total` | Counter | `execution_kind={host,guest}`, `outcome={success,stale_generation,capacity_denied,auth_denied,terminal}` | Attach attempts. |
| `d2b_shell_kill_total` | Counter | `execution_kind={host,guest}`, `outcome={success,stale_generation,auth_denied,terminal}` | Kill attempts. |

### Spans

| Span name | Required attributes | Forbidden attributes |
| --- | --- | --- |
| `shell.reconcile.pool` | `resource.kind`, `execution.kind`, `outcome` | No resource names, usernames, paths, session names, or handles. |
| `shell.reconcile.session` | `resource.kind`, `execution.kind`, `outcome` | No resource names, usernames, PIDs, or ring bytes. |
| `shell.attach` | `execution.kind`, `outcome` | No terminal bytes, session names, usernames, or path data. |
| `shell.kill` | `execution.kind`, `outcome` | No terminal bytes, session names, or PIDs. |
| `shell.adopt` | `execution.kind`, `outcome` | No raw InvocationID values, cgroup paths, or unit names. |

### Audit events

| Event type | When emitted | Permitted fields |
| --- | --- | --- |
| `shell-pool-created` | Pool first reaches readiness | `zone_uid_digest`, `resource_uid_digest`, `execution_kind`, `isolation_posture` |
| `shell-session-open` | A new session is created by `OpenSession` | `zone_uid_digest`, `resource_uid_digest`, `execution_kind`, `result` |
| `shell-session-attach` | An attach stream is successfully opened | `zone_uid_digest`, `resource_uid_digest`, `execution_kind`, `result` |
| `shell-session-detach` | A detach or detach-all completes | `zone_uid_digest`, `resource_uid_digest`, `execution_kind`, `result`, `detach_scope={self,all}` |
| `shell-session-kill` | A kill request completes | `zone_uid_digest`, `resource_uid_digest`, `execution_kind`, `result` |
| `shell-session-closed` | The login shell exits and the session becomes terminal | `zone_uid_digest`, `resource_uid_digest`, `execution_kind`, `result={clean-exit,nonzero-exit,killed}` |
| `shell-supervisor-degraded` | Supervisor identity or routing becomes unsafe | `zone_uid_digest`, `resource_uid_digest`, `execution_kind`, `result={lost,ambiguous}` |
| `shell-pool-isolation-warning` | A Host pool reports `isolationPosture=none` | `zone_uid_digest`, `resource_uid_digest`, `execution_kind=host`, `isolation_posture=none` |

### NEVER in any observable surface

The following are prohibited in metrics, span attributes, structured logs, and audit
records:

- terminal bytes
- login-shell argv
- cwd
- environment values
- filesystem paths
- PIDs
- unit names
- usernames
- session names
- socket paths
- opaque attach or supervisor handles
- raw InvocationID values

## Async reconcile patterns

| Pattern | Trigger | Idempotent action | Terminal condition |
| --- | --- | --- | --- |
| `controller-loop` | provider controller wakeup or resource watch event | Reconcile pools, then reconcile sessions, then refresh aggregate counts. | Observed generations recorded. |
| `supervisor-create` | session without `status.supervisorRef` | Create exactly one supervisor `Process` from the canonical template. | Supervisor resource exists or terminal error recorded. |
| `supervisor-adopt` | controller restart or runtime reconnect | Re-verify existing supervisor identity and re-register routes. | Verified route restored or session degraded. |
| `route-registration` | supervisor becomes ready | Register d2b-bus routes for the exact session generation. | Route visible and `RouteRegistered` condition true. |
| `capacity-check` | `OpenSession` or attach request | Read pool counts from store and reject if capacity is exhausted. | Request accepted or rejected without side effects. |
| `finalizer` | resource deletion timestamp set | Invalidate routes, stop verified supervisors, and remove finalizers. | Resource enters `Deleted`. |

Each reconcile pattern must survive controller restart with no in-memory-only dependency.
The only legitimate restart-sensitive shell state is the supervisor-owned PTY, ring, and
attach state; those are adopted by verifying the supervisor, not by copying bytes or
handles into the controller.

### Fast hermetic execution and test placement (D094)

Per D094 and `ADR-046-validation-and-delivery` §10.16, this Provider's `src/`
unit tests and `tests/*.rs` hermetic suite are fast, in-process, deterministic,
and parallel-safe: an individual normal test has an advisory wall-clock p95
diagnostic threshold of <=50 ms; gate enforcement is aggregate per-crate
process CPU only. There is no wall-clock
sleep, and `cargo test -p d2b-provider-shell-terminal --lib --tests` completes
in ≤2 s warm-cache execution time (compilation excluded). They use a
deterministic fake clock/RNG and the toolkit fakes/FakeEffectPort only - no
process spawn, container, network, DBus, systemd, broker daemon, Nix eval/build,
KVM, USB/GPU/TPM hardware, or live cloud, and no filesystem tree beyond tiny
temp fixtures. Any scenario needing those lives only in `integration/`, which
keeps a lane timeout/budget, parallel isolation, and fake external services by
default; such a need is re-placed into `integration/`, never given a sleep,
larger timeout, or `#[ignore]`. Bounded crypto/property tests are the only
classified exception, each named with a capped case count and a declared higher
per-test advisory threshold.

## Implementation work items

### ADR046-sterm-001
| Field | Value |
| --- | --- |
| Dependency/owner | Resource schemas area; owned by `d2b-provider-shell-terminal` resource modules. |
| Current source | None - net-new v3 qualified `ShellPool` and `ShellSession` resource schemas; superseded draft and legacy shell code do not define these canonical resources. |
| Reuse action | create |
| Destination | `packages/d2b-provider-shell-terminal/src/resources/{pool,session}.rs` |
| Detailed design | Implement `shell-terminal.d2bus.org.ShellPool` and `shell-terminal.d2bus.org.ShellSession` schemas with qualified names, common phases, and typed detail fields. |
| Integration | Nix resource compiler, resource API admission, controller reconcile, status writers, and d2b-bus routing all consume the qualified pool/session schemas. Integration path: `packages/d2b-provider-shell-terminal/integration/resource-shape/`. |
| Data migration | Full d2b 3.0 reset; no v2 state/config import. |
| Validation | `packages/d2b-provider-shell-terminal/tests/resource_schema.rs` |
| Removal proof | None - net-new; no prior owner to remove. |

### ADR046-sterm-002
| Field | Value |
| --- | --- |
| Dependency/owner | Controller binary area; owned by `d2b-provider-shell-terminal` controller and core Operation ledger integration. |
| Current source | None - net-new v3 controller; legacy guestd and unsafe-local helper shell paths are not the controller/state authority. |
| Reuse action | create |
| Destination | `packages/d2b-provider-shell-terminal/src/bin/d2b-shell-terminal-controller.rs` |
| Detailed design | Implement `d2b-shell-terminal-controller` with pool/session reconcile loops; assert ProviderStateSet is empty; publish bounded non-secret operational state to resource status and the core Operation ledger; no controller Provider state Volume or `/state` mount exists. Primary reuse disposition: `create`. Preserved source-plan detail: net-new controller; preserve status-first ProviderStateSet-empty rule. |
| Integration | Core ProviderDeployment starts the controller Process; controller reconciles ShellPool/ShellSession resources, writes status, registers routes, and records operations without a Provider state Volume. Integration path: `packages/d2b-provider-shell-terminal/integration/controller-restart/`. |
| Data migration | Full d2b 3.0 reset; no v2 state/config import. |
| Validation | `packages/d2b-provider-shell-terminal/tests/controller_reconcile.rs` |
| Removal proof | None - net-new controller; legacy controller-equivalent state owner does not exist. |

### ADR046-sterm-003
| Field | Value |
| --- | --- |
| Dependency/owner | Supervisor binary area; owned by `d2b-provider-shell-terminal` session supervisor runtime. |
| Current source | Reuse narrow ring/runtime ideas from `packages/d2b-guestd/src/shell.rs` and adoption-shape ideas from `packages/d2b-unsafe-local-helper/src/runtime.rs`; both legacy authorities are superseded. |
| Reuse source | `packages/d2b-guestd/src/shell.rs`; `packages/d2b-unsafe-local-helper/src/runtime.rs`. |
| Reuse action | adapt |
| Destination | `packages/d2b-provider-shell-terminal/src/bin/d2b-shell-session-supervisor.rs` |
| Detailed design | Implement `d2b-shell-session-supervisor` as the sole PTY owner for Host and Guest pools. Primary reuse disposition: `adapt`. Preserved source-plan detail: reuse narrow mechanics only; move PTY authority into per-session supervisor and exclude legacy protocols/identities/state storage. |
| Integration | Controller creates one user-domain supervisor Process per ShellSession; supervisor owns PTY, login shell, ring, attach bookkeeping, and private ComponentSession service. Integration path: `packages/d2b-provider-shell-terminal/integration/supervisor-host-guest/`. |
| Data migration | Full d2b 3.0 reset; no v2 shell state import; PTY/ring state is live process memory only. |
| Validation | `packages/d2b-provider-shell-terminal/tests/supervisor_runtime.rs` |
| Removal proof | Supersedes `guestd/src/shell.rs` managed runtime and unsafe-local helper shell supervisor; removed once successor supervisor coverage passes. |

### ADR046-sterm-004
| Field | Value |
| --- | --- |
| Dependency/owner | Process templates area; owned by Nix compiler plus shell-terminal controller. |
| Current source | Superseded draft templates included pool-wide/system-domain or management-worker concepts; canonical v3 templates are defined in this spec. |
| Reuse action | replace |
| Destination | `packages/d2b-provider-shell-terminal/src/process_templates.rs` |
| Detailed design | Teach the Nix compiler and controller to emit the canonical controller and user-domain supervisor `Process` templates. Primary reuse disposition: `replace`. Preserved source-plan detail: replace incorrect draft templates with canonical controller and user-domain supervisor Process templates. |
| Integration | Nix compiler emits controller Process/Endpoint resources; controller emits per-session user-domain supervisor Processes; Provider/system-systemd realizes them. Integration path: `packages/d2b-provider-shell-terminal/integration/process-placement/`. |
| Data migration | Full d2b 3.0 reset; no v2 state/config import. |
| Validation | `packages/d2b-provider-shell-terminal/tests/process_templates.rs` |
| Removal proof | Removes forbidden pool-wide supervisor, disguised system-domain supervisor, management EphemeralProcess, and sealed output Volume concepts from the template surface. |

### ADR046-sterm-005
| Field | Value |
| --- | --- |
| Dependency/owner | OpenSession lifecycle area; owned by controller service implementation. |
| Current source | None - net-new v3 `OpenSession` lifecycle; legacy shell protocols do not create ShellSession resources with inherited-field freeze. |
| Reuse action | create |
| Destination | `packages/d2b-provider-shell-terminal/src/service/open_session.rs` |
| Detailed design | Create sessions from pools, freeze inherited fields, and return `supervisorGeneration` to callers. |
| Integration | `shell-terminal.v3.OpenSession` validates pool capacity and policy, creates ShellSession and supervisor Process, registers route data, and returns session/supervisor references to clients. Integration path: `packages/d2b-provider-shell-terminal/integration/open-session/`. |
| Data migration | Full d2b 3.0 reset; no v2 state/config import. |
| Validation | `packages/d2b-provider-shell-terminal/tests/open_session.rs` |
| Removal proof | None - net-new resource lifecycle; no prior owner to remove. |

### ADR046-sterm-006
| Field | Value |
| --- | --- |
| Dependency/owner | PTY and ring area; owned by per-session supervisor runtime. |
| Current source | Ring buffer mechanics may reuse ideas from `packages/d2b-guestd/src/shell.rs`; management workers and sealed output Volumes from prior draft are removed. |
| Reuse source | `packages/d2b-guestd/src/shell.rs` bounded ring ideas only. |
| Reuse action | adapt |
| Destination | `packages/d2b-provider-shell-terminal/src/session/{pty,ring}.rs` |
| Detailed design | Implement the in-memory PTY owner model, bounded ring buffer, replay, and eviction counters; do not create any management worker or `EphemeralProcess`. Primary reuse disposition: `adapt`. Preserved source-plan detail: reuse ring semantics; keep bytes in supervisor memory and remove management worker/EphemeralProcess model. |
| Integration | Supervisor named terminal stream replays bounded ring tail then streams live PTY I/O; status publishes only ring byte counters and attach count. Integration path: `packages/d2b-provider-shell-terminal/integration/ring-overflow/`. |
| Data migration | Full d2b 3.0 reset; no terminal byte or ring-state import. |
| Validation | `packages/d2b-provider-shell-terminal/tests/ring_buffer.rs` |
| Removal proof | Proves no controller-owned PTY, management worker, EphemeralProcess, or sealed output Volume remains for shell output/management responses. |

### ADR046-sterm-007
| Field | Value |
| --- | --- |
| Dependency/owner | Adoption and routing area; owned by controller session adoption/routing module. |
| Current source | Reuse verification/adoption shape only from `packages/d2b-unsafe-local-helper/src/runtime.rs` `ScopeRuntime` and `PersistedScope`; exclude helper protocol and state storage. |
| Reuse source | `packages/d2b-unsafe-local-helper/src/runtime.rs` `ScopeRuntime` and `PersistedScope` adoption pattern. |
| Reuse action | adapt |
| Destination | `packages/d2b-provider-shell-terminal/src/session/adopt.rs` |
| Detailed design | Implement restart adoption, InvocationID plus cgroup verification, route registration, and stale-generation invalidation. Primary reuse disposition: `adapt`. Preserved source-plan detail: adapt identity-verification shape; do not reuse helper protocol, identities, or state storage assumptions. |
| Integration | Controller restart scans supervisor Processes, verifies owner/session/generation identity, re-registers exact d2b-bus routes, and rejects stale or ambiguous handles. Integration path: `packages/d2b-provider-shell-terminal/integration/adopt-after-restart/`. |
| Data migration | Full d2b 3.0 reset; no v2 state/config import. |
| Validation | `packages/d2b-provider-shell-terminal/tests/adoption.rs` |
| Removal proof | Supersedes unsafe-local helper adoption storage/protocol; proof is degraded-ambiguity and stale-generation tests with no helper state dependency. |

### ADR046-sterm-008
| Field | Value |
| --- | --- |
| Dependency/owner | Host rules area; owned by shell-terminal Host policy module. |
| Current source | Supersedes unsafe-local Host shell path from `packages/d2b-unsafe-local-helper/src/services/shell/` while preserving explicit non-isolation warning semantics. |
| Reuse action | replace |
| Destination | `packages/d2b-provider-shell-terminal/src/host_rules.rs` |
| Detailed design | Emit Host `isolationPosture=none` warnings, same-UID verification, and relay denial for Host user-domain pools. Primary reuse disposition: `replace`. Preserved source-plan detail: replace unsafe-local helper shell policy with resource-backed Host pool warnings and same-UID checks. |
| Integration | Pool reconcile reads Host posture and User identity, writes warning status/audit, and admission rejects relay-authenticated Host user-domain access. Integration path: `packages/d2b-provider-shell-terminal/integration/host-warning/`. |
| Data migration | Full d2b 3.0 reset; no unsafe-local shell state/config import. |
| Validation | `packages/d2b-provider-shell-terminal/tests/host_rules.rs` |
| Removal proof | Supersedes unsafe-local Host shell supervisor path; proof requires no SSH/direct-host fallback and relay denial coverage. |

### ADR046-sterm-009
| Field | Value |
| --- | --- |
| Dependency/owner | Guest rules area; owned by shell-terminal Guest policy module. |
| Current source | Supersedes guest persistent-shell runtime in `packages/d2b-guestd/src/shell.rs`; Guest user-domain placement moves to Provider/system-systemd. |
| Reuse action | replace |
| Destination | `packages/d2b-provider-shell-terminal/src/guest_rules.rs` |
| Detailed design | Require Guest `allowedDomains` to include `user`, require `defaultUserRef`, and place supervisors through the Guest user manager. Primary reuse disposition: `replace`. Preserved source-plan detail: replace guestd shell authority with Guest resource user-domain admission and supervisor placement. |
| Integration | Pool admission validates Guest capabilities and default user; controller creates user-domain supervisor Processes through the Guest user manager exposed by Provider/system-systemd. Integration path: `packages/d2b-provider-shell-terminal/integration/guest-user-domain/`. |
| Data migration | Full d2b 3.0 reset; no guestd shell runtime state/config import. |
| Validation | `packages/d2b-provider-shell-terminal/tests/guest_rules.rs` |
| Removal proof | Supersedes independent guestd session-limit/runtime authority; proof is rejection of Guests without `user` domain and no guestd-managed shell path. |

### ADR046-sterm-010
| Field | Value |
| --- | --- |
| Dependency/owner | RBAC and relay denial area; owned by shell-terminal authorization module. |
| Current source | Existing public-wire `ShellOp`/unsafe-local surfaces are superseded; v3 requires ComponentSession service authorization. |
| Reuse action | replace |
| Destination | `packages/d2b-provider-shell-terminal/src/authz.rs` |
| Detailed design | Gate all verbs on `Role/shell-admin` or Zone-admin superset and fail closed for relay-authenticated Host user-domain callers. Primary reuse disposition: `replace`. Preserved source-plan detail: replace legacy shell operation authorization with Role/shell-admin or Zone-admin service gates. |
| Integration | Controller and supervisor ComponentSession methods authorize before capacity or route lookup, preserving stale-handle non-disclosure and Host relay denial. Integration path: `packages/d2b-provider-shell-terminal/integration/authz/`. |
| Data migration | Full d2b 3.0 reset; no v2 state/config import. |
| Validation | `packages/d2b-provider-shell-terminal/tests/authz.rs` |
| Removal proof | Supersedes public-wire shell protocol authorization; proof requires no `ShellOp` or `ShellOpResponse` path remains. |

### ADR046-sterm-011
| Field | Value |
| --- | --- |
| Dependency/owner | Audit and telemetry area; owned by shell-terminal audit/telemetry modules. |
| Current source | None - net-new v3 closed-label/redacted observability for shell-terminal; legacy shell paths must not leak names, paths, PIDs, or terminal bytes. |
| Reuse action | create |
| Destination | `packages/d2b-provider-shell-terminal/src/{audit,telemetry}.rs` |
| Detailed design | Implement closed-label metrics, redacted spans, and audit events with no usernames, session names, paths, or terminal bytes. Primary reuse disposition: `create`. Preserved source-plan detail: net-new redacted observability. |
| Integration | Reconcile, OpenSession, Attach, Detach, Kill, terminal exit, degradation, and Host posture warnings emit only digest/enum surfaces consumed by audit and OTEL collectors. Integration path: `packages/d2b-provider-shell-terminal/integration/support-redaction/`. |
| Data migration | Full d2b 3.0 reset; no v2 audit/telemetry state import. |
| Validation | `packages/d2b-provider-shell-terminal/tests/redaction.rs` |
| Removal proof | None - net-new observability surface; legacy paths must be removed or adapted to pass redaction tests. |

### ADR046-sterm-012
| Field | Value |
| --- | --- |
| Dependency/owner | Baseline removal area; owned by migration/removal implementation for shell-terminal. |
| Current source | Superseded sources: `packages/d2b-guestd/src/shell.rs`, `packages/d2b-unsafe-local-helper/src/services/shell/`, and `packages/d2b-contracts/src/public_wire.rs` `ShellOp`/`ShellOpResponse`. |
| Reuse action | delete-after-cutover |
| Destination | `packages/d2b-provider-shell-terminal/src/migration.rs` |
| Detailed design | Delete superseded guestd shell runtime, unsafe-local helper shell supervisor, and public-wire `ShellOp` or `ShellOpResponse` shell protocol. Primary reuse disposition: `delete-after-cutover`. Preserved source-plan detail: delete superseded runtime/protocol surfaces after successor parity. |
| Integration | Removal runs after shell-terminal resource, supervisor, service, RBAC, and integration coverage prove parity; workspace manifests, CI shards, and pins are updated so old and new suites do not run indefinitely. Integration path: `packages/d2b-provider-shell-terminal/integration/migration-baseline/`. |
| Data migration | Full d2b 3.0 reset; no v2 shell state/config import. |
| Validation | `packages/d2b-provider-shell-terminal/tests/migration.rs` |
| Removal proof | Delete `packages/d2b-guestd/src/shell.rs` managed runtime flow, `packages/d2b-unsafe-local-helper/src/services/shell/`, and `ShellOp`/`ShellOpResponse`; update closed gate manifests, flake/matrix/Nix-unit pins, generated ledgers, and CI workflow shards. |

### ADR046-sterm-013
| Field | Value |
| --- | --- |
| Dependency/owner | Supervisor service area; owned by shell-terminal controller/supervisor service modules and ComponentSession contracts. |
| Current source | Reuse service-shape ideas only from main commit `a1cc0b2da4a08ca3240a770a972fe4da6f912bef` generated v2 shell services; exclude ADR 0045 session, realm, and constellation assumptions. |
| Reuse source | `a1cc0b2da4a08ca3240a770a972fe4da6f912bef` `packages/d2b-contracts/src/generated_v2_services/shell.rs` and `shell_ttrpc.rs`. |
| Reuse action | adapt |
| Destination | `packages/d2b-provider-shell-terminal/src/service/{controller,supervisor}.rs` |
| Detailed design | Define and implement `shell-terminal.v3` and `shell-session-supervisor.v1` ComponentSession services and the named `terminal` stream contract. Primary reuse disposition: `adapt`. Preserved source-plan detail: adapt service-shape ideas into v3 ComponentSession services and named terminal stream. |
| Integration | Controller service handles OpenSession/ListSessions/PoolStatus; supervisor service handles Attach/Detach/DetachAll/Kill/SupervisorStatus and terminal named stream routed by d2b-bus generation identity. Integration path: `packages/d2b-provider-shell-terminal/integration/service-contract/`. |
| Data migration | Full d2b 3.0 reset; no v2 service/session state import. |
| Validation | `packages/d2b-provider-shell-terminal/tests/service_contract.rs` |
| Removal proof | Supersedes public-wire shell protocol and helper supervisor protocol once ComponentSession service-contract tests pass. |

## Baseline reuse and removal

This rewrite deliberately reuses narrow primitives and removes broad, incorrect baseline
assumptions.

| Area | Reuse or removal | Source or target | Notes |
| --- | --- | --- | --- |
| ring buffer mechanics | Reuse | `packages/d2b-guestd/src/shell.rs` | Reuse bounded ring ideas only; move ownership into the per-session supervisor and keep bytes out of the controller. |
| scope adoption pattern | Reuse | `packages/d2b-unsafe-local-helper/src/runtime.rs` `ScopeRuntime` and `PersistedScope` | Reuse verification and adoption shape only; do not reuse protocol, helper identities, or state storage assumptions. |
| service starting point | Reuse | `a1cc0b2da4a08ca3240a770a972fe4da6f912bef` `packages/d2b-contracts/src/generated_v2_services/shell.rs` and `shell_ttrpc.rs` | Reuse service-shape ideas only; exclude ADR 0045 session, realm, and constellation assumptions. |
| Guest persistent-shell runtime | Remove | `packages/d2b-guestd/src/shell.rs` `ShellRuntimeConfig` and related managed runtime flow | Superseded by `shell-terminal.d2bus.org.ShellSession` plus per-session supervisors. |
| unsafe-local helper supervisor | Remove | `packages/d2b-unsafe-local-helper/src/services/shell/` | Superseded by `d2b-shell-session-supervisor` and `Provider/system-systemd` user-domain placement. |
| public wire shell protocol | Remove | `packages/d2b-contracts/src/public_wire.rs` `ShellOp` and `ShellOpResponse` | Superseded by ComponentSession services and named terminal streams. |
| pool-wide supervisor model | Remove | prior ADR 0046 draft text and templates | No pool-wide worker remains. |
| management workers and sealed output volumes | Remove | prior ADR 0046 draft management flow | List, detach, detach-all, and kill are typed methods, not worker jobs. |

### Explicit removals from the superseded draft

The following concepts are forbidden in the corrected spec and must not reappear in code
or documentation:

- pool-wide supervisors
- any supervisor in the system domain while impersonating a workload user
- management `EphemeralProcess` objects
- sealed output `Volume` objects for session management
- any non-vendor-qualified pool or session type names in resource surfaces
- `d2b-shell-pool-supervisor` as the managed guest or host shell worker
- `d2b-guest-shell-runner` as the managed session runtime process
- `supervisorPhase` as a top-level pool phase
- SSH or direct-host fallback
- controller-owned PTY or terminal bytes

Per D094, each replaced current-code test is retired with an explicit
keep/adapt/move/delete disposition and a removal gate: the minimum reusable
semantic assertions migrate into this crate's hermetic `tests/`, and the old
duplicate tests, shell gates, fixtures, static artifacts, CI jobs, and manifest
entries are deleted once successor coverage and the removal proof pass -
updating `tests/layer1-jobs.json`, the closed gate manifests, the
flake/matrix/Nix-unit pins, the generated ledgers, and the CI workflow shards.
Old and new suites never run in parallel indefinitely.

## Appendix A: canonical example resources

```yaml
apiVersion: resources.d2bus.org/v3
type: shell-terminal.d2bus.org.ShellPool
metadata:
  name: host-alice-shell
  zone: personal
  ownerRef: Provider/shell-terminal
spec:
  providerRef: Provider/shell-terminal
  executionRef: Host/workstation
  userRef: User/alice
  loginShellRef: artifact://shells/bash-login
  maxSessions: 4
  maxAttached: 1
  outputRingCapacity: 262144
status:
  observedGeneration: 1
  phase: Ready
  conditions:
    - type: Ready
      status: "True"
    - type: IsolationPostureWarning
      status: "True"
  lastReconciledAt: 2026-07-22T00:00:00.000Z
  startedAt: 2026-07-22T00:00:00.000Z
  completedAt: null
  outcome: null
  resource:
    detail:
      kind: IsolationPostureWarning
      message: host execution target reports non-isolated posture
    executionRef: Host/workstation
    userRef: User/alice
    activeSessions: 1
    attachedSessions: 1
    capacityRemaining: 3
    attachedCapacityRemaining: 0
    isolationPosture: none
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

```yaml
apiVersion: resources.d2bus.org/v3
type: shell-terminal.d2bus.org.ShellSession
metadata:
  name: host-alice-shell-main
  zone: personal
  ownerRef: shell-terminal.d2bus.org.ShellPool/host-alice-shell
spec:
  providerRef: Provider/shell-terminal
  poolRef: shell-terminal.d2bus.org.ShellPool/host-alice-shell
  executionRef: Host/workstation
  userRef: User/alice
  loginShellRef: artifact://shells/bash-login
  sessionName: main
  outputRingCapacity: 262144
  desiredLifecycle: running
status:
  observedGeneration: 3
  phase: Ready
  conditions:
    - type: Ready
      status: "True"
    - type: SupervisorReady
      status: "True"
    - type: RouteRegistered
      status: "True"
    - type: Attached
      status: "True"
  lastReconciledAt: 2026-07-22T00:00:00.000Z
  startedAt: 2026-07-22T00:00:00.000Z
  completedAt: null
  outcome: null
  resource:
    detail:
      kind: ReadyAttached
    supervisorRef: Process/shell-terminal--supervisor--0d3b0e42
    supervisorGeneration: 7
    attachCount: 1
    outputRingBytes: 65536
    outputRingEvictedBytes: 8192
  update:
    state: Current
    reasons: []
    observedGeneration: 3
    targetGeneration: 3
    disruption: None
    preserveState: true
    operationId: null
    lastAssessedAt: null
    owned: { count: 0, refs: [] }
    dependencies: { count: 0, refs: [] }
```

## Appendix B: controller and supervisor interaction sequence

| Step | Actor | Action |
| --- | --- | --- |
| 1 | CLI | Call `OpenSession` on `shell-terminal.v3` with a pool reference. |
| 2 | Controller | Validate pool capacity, resolve inherited execution and user fields, create `shell-terminal.d2bus.org.ShellSession`. |
| 3 | Controller | Create `Process/shell-terminal--supervisor--<session-uid-short>` in the user domain. |
| 4 | Supervisor | Spawn the manifest-fixed login shell, allocate the PTY, initialize the ring, and signal readiness. |
| 5 | Controller | Verify InvocationID and Process generation, increment `status.supervisorGeneration`, and register bus routes. |
| 6 | CLI | Open a direct ComponentSession to `shell-session-supervisor.v1` using the returned generation. |
| 7 | Supervisor | Replay ring tail and switch to live PTY streaming. |
| 8 | CLI | Later call `Detach`, `DetachAll`, or `Kill` directly on the supervisor service. |
| 9 | Controller | On restart, re-scan supervisors and re-register routes only after strict identity verification. |

## Appendix C: failure and recovery matrix

| Failure | Detection point | Resource outcome | Operator action |
| --- | --- | --- | --- |
| missing Host or Guest target | pool reconcile | Pool `Degraded` or `Failed` depending on permanence. | Restore or delete the target reference. |
| missing user reference | pool reconcile | Pool `Degraded` or `Failed`. | Restore or delete the user reference. |
| supervisor never becomes ready | session reconcile | Session remains `Pending` then `Failed` or retryable `Degraded`. | Inspect provider logs and retry. |
| supervisor disappears after readiness | adoption or live health watch | Session `Degraded` with `SupervisorLost`. | Delete and recreate the session. |
| two candidate supervisors match one session | adoption | Session `Degraded` with `SupervisorAmbiguity`. | Delete the session and clean foreign processes. |
| client uses stale generation | supervisor method admission | Request rejected with `StaleSessionGeneration`. | Refresh session info and reattach. |
| pool attach capacity exhausted | attach admission | Session remains `Ready`; attach denied. | Detach another session or raise pool limit. |
| Host posture becomes none | pool reconcile | Pool remains `Ready` with warning detail and audit event. | Acknowledge the warning or migrate to a Guest pool. |

## Appendix D: normative checklist

1. All stored type strings are qualified: `shell-terminal.d2bus.org.ShellPool` or `shell-terminal.d2bus.org.ShellSession`.
2. Every session supervisor is user-domain and names an exact `User/<name>`.
3. Exactly one supervisor `Process` exists per session.
4. No pool creates a supervisor `Process`.
5. No controller stores PTY or terminal bytes.
6. No management `EphemeralProcess` is used for list, detach, detach-all, or kill.
7. No sealed output `Volume` is used for shell output or management responses.
8. All attach or kill requests carry `status.supervisorGeneration`.
9. All shell verbs require `Role/shell-admin` or a Zone-admin superset.
10. Relay-authenticated Host user-domain access is denied.
11. Host pools surface `isolationPosture=none` warnings in status and audit.
12. Guest pools require `allowedDomains` to contain `user` and require `defaultUserRef`.
13. Metrics, logs, spans, and audit records exclude names, usernames, PIDs, paths, and terminal bytes.
14. No SSH or direct-host fallback exists.
15. The provider crate contains `src/tests/integration/README.md`.
