# ADR 0046 Provider dossier: shell-terminal

| Field | Value |
| --- | --- |
| Spec ID | `ADR-046-provider-shell-terminal` |
| Parent | ADR 0046 |
| Status | Proposed |
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

- `shell.d2b.io.ShellPool`
- `shell.d2b.io.ShellSession`
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
- core ProviderDeployment-created `Volume` resources per component with provider-state extension schema

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
dynamic user-domain `service` Process created exactly once per `shell.d2b.io.ShellSession`.
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
| `d2b-shell-session-supervisor` | per-session PTY owner and internal shell service | user-domain dynamic `service` `Process` owned by `shell.d2b.io.ShellSession/<name>` | exact `User/<name>` from the pool |

No other binary is normative for this provider. In particular:

- `d2b-shell-pool-supervisor` is removed from the design.
- `d2b-guest-shell-runner` is not the managed shell session service.
- `guestd/src/shell.rs` is not the long-lived managed-session authority.
- the old unsafe-local helper supervisor protocol is superseded.

The provider exposes exactly two services:

- `shell-terminal.v3` on the controller
- `shell-session-supervisor.v1` on each supervisor

The provider declares two stateful components (controller and session supervisor). The
core ProviderDeployment lifecycle handler creates the corresponding `Volume` resources
before the component Processes start and deletes them after the component Processes
finish. The shell-terminal controller does not own, create, or delete its prerequisite
Volumes; it does not add `Volume` to its exported ResourceTypes. `Provider/volume-local`
is the sole Volume reconciler. The controller only consumes its required view through a
mount. The session-supervisor Volume is declared in the session-supervisor component
descriptor; core ProviderDeployment creates it when the `shell.d2b.io.ShellSession`
resource and its supervisor `Process` are provisioned. PTY state, attach state, and
output bytes remain exclusively in session-supervisor process memory and are never
written to any Volume.

## ResourceTypes overview

| ResourceType | Owner | Purpose | Creates provider worker? | Cardinality |
| --- | --- | --- | --- | --- |
| `shell.d2b.io.ShellPool` | `Provider/shell-terminal` | capacity and policy for one execution target plus one user identity | No | one or more per Zone |
| `shell.d2b.io.ShellSession` | `Provider/shell-terminal` | one persistent login-shell session plus exactly one supervisor `Process` | Yes, exactly one supervisor | zero or more per pool |

| Reference form | Example |
| --- | --- |
| pool `ResourceRef` | `shell.d2b.io.ShellPool/dev-alice` |
| session `ResourceRef` | `shell.d2b.io.ShellSession/dev-alice-main` |
| supervisor owner reference | `shell.d2b.io.ShellSession/dev-alice-main` |
| Nix type string | `"shell.d2b.io.ShellPool"` or `"shell.d2b.io.ShellSession"` |

`status.phase` on both ResourceTypes uses only the common phase catalog:

- `Pending`
- `Ready`
- `Succeeded`
- `Degraded`
- `Failed`
- `Deleted`
- `Unknown`

Initialization, deletion, steady-state nuance, or terminal-cause detail belongs in the
resource-specific `status.detail` object and conditions, not in ad hoc phase strings.

## `shell.d2b.io.ShellPool` ResourceType

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
apiVersion: resources.d2b.io/v3
type: shell.d2b.io.ShellPool
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
  phase: Ready
  detail:
    kind: CapacityReady
  activeSessions: 2
  attachedSessions: 1
  capacityRemaining: 6
  attachedCapacityRemaining: 0
```

### Spec schema

| Field | Type | Required | Default | Bound | Description |
| --- | --- | --- | --- | --- | --- |
| `apiVersion` | `string` | Yes | `resources.d2b.io/v3` | Exact | Resource API version. |
| `type` | `string` | Yes | `shell.d2b.io.ShellPool` | Exact | Vendor-qualified ResourceType identifier. |
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
has any child `shell.d2b.io.ShellSession`. Mutation after session creation is rejected
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
| `Initializing` | Pool exists but target, user, or controller Volume validation is still in progress. |
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
| `Ready` | The pool target, user, login shell, and controller Volume resources are all valid and counted. | Any validation or reconcile step is incomplete or failed. |
| `ExecutionTargetVerified` | The referenced Host or Guest exists and supports user-domain placement. | The target is absent, unresolved, or missing user-domain support. |
| `UserVerified` | The referenced `User/<name>` exists and is accepted by the target. | User lookup fails or the target rejects the user. |
| `CapacityAvailable` | At least one more `shell.d2b.io.ShellSession` may be created. | `activeSessions >= maxSessions`. |
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
6. Observe that the component `Volume` resources in the ProviderStateSet report
   `phase=Ready` and `stateSchemaPhase=current`. These Volumes are owned and managed by
   core ProviderDeployment and `Provider/volume-local`; the shell-terminal controller
   does not create or delete them, only blocks the pool on their absence.
7. Enumerate child `shell.d2b.io.ShellSession` resources by owner or `spec.poolRef`.
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
| `ControllerStateVolumeMissing` | `Pending` | Yes | The component `Volume` resource(s) in the ProviderStateSet are absent or not yet Ready; core ProviderDeployment provisions them. |
| `RelayHostUserDomainDenied` | `Ready` | No | A relay-authenticated caller attempted Host user-domain access. |
| `PoolDeleteBlockedBySessions` | `Deleting` | Yes | The pool still has child sessions that must finish finalization first. |

### Finalizer steps

1. Mark the pool `Deleting` condition true.
2. List child `shell.d2b.io.ShellSession` resources.
3. If any child session is not in `Deleted`, block pool finalization with
   `PoolDeleteBlockedBySessions`.
4. Do not synthesize kill commands or management workers.
5. Confirm that the controller `Volume` resource for this pool's execution target is still
   present; if absent, log the discrepancy and continue (the pool finalizer is not
   responsible for Volume lifecycle, which outlives individual pool resources).
6. Clear pool-scoped route summaries and aggregate counters.
7. Remove the provider finalizer.
8. Allow the store to tombstone the pool and set `status.phase=Deleted`.

## `shell.d2b.io.ShellSession` ResourceType

### Purpose

A session is the unit of persistent shell state. It owns exactly one user-domain session
supervisor `Process` named `shell-terminal--supervisor--<session-uid-short>`. The
supervisor owns exactly one PTY, exactly one login-shell process tree, exactly one bounded
merged-output ring in supervisor memory, the attach bookkeeping, and one private
ComponentSession endpoint. The controller owns lifecycle orchestration only.

### Canonical object shape

```yaml
apiVersion: resources.d2b.io/v3
type: shell.d2b.io.ShellSession
metadata:
  name: guest-alice-shell-main
  zone: dev
  ownerRef: shell.d2b.io.ShellPool/guest-alice-shell
spec:
  providerRef: Provider/shell-terminal
  poolRef: shell.d2b.io.ShellPool/guest-alice-shell
  executionRef: Guest/work
  userRef: User/alice
  loginShellRef: artifact://shells/bash-login
  sessionName: main
  outputRingCapacity: 262144
  desiredLifecycle: running
status:
  phase: Ready
  detail:
    kind: ReadyDetached
  supervisorRef: Process/shell-terminal--supervisor--0d3b0e42
  supervisorGeneration: 1
  attachCount: 0
  outputRingBytes: 8192
  outputRingEvictedBytes: 0
```

### Spec schema

| Field | Type | Required | Default | Bound | Description |
| --- | --- | --- | --- | --- | --- |
| `apiVersion` | `string` | Yes | `resources.d2b.io/v3` | Exact | Resource API version. |
| `type` | `string` | Yes | `shell.d2b.io.ShellSession` | Exact | Vendor-qualified ResourceType identifier. |
| `metadata.name` | `ResourceName` | Yes | Controller-generated or Nix-specified | `^[a-z][a-z0-9-]*$`, max 63 | Stable session resource name. |
| `metadata.zone` | `ResourceName` | Yes | None | Existing Zone | Owning Zone. |
| `metadata.ownerRef` | `ResourceRef` | Yes | `shell.d2b.io.ShellPool/<name>` | pool reference | Owning pool. |
| `spec.providerRef` | `ResourceRef` | Yes | `Provider/shell-terminal` | Exact | Provider identity. |
| `spec.poolRef` | `ResourceRef` | Yes | None | `shell.d2b.io.ShellPool/<name>` | Pool from which capacity and placement are derived. |
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
creates a `shell.d2b.io.ShellSession`.

### Controller `Process` template

```yaml
apiVersion: resources.d2b.io/v3
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
  endpoints:
    - name: shell-service
      transport: unix
      purpose: shell-terminal.v3
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
apiVersion: resources.d2b.io/v3
type: Process
metadata:
  name: shell-terminal--supervisor--<session-uid-short>
  zone: <zone>
  ownerRef: shell.d2b.io.ShellSession/<session-name>
spec:
  providerRef: Provider/system-systemd
  executionRef: Guest/<guest>   # or Host/<host> for Host pools
  domain: user
  userRef: User/<pool-user>     # from shell.d2b.io.ShellPool spec.userRef
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
  endpoints:
    - name: supervisor-session
      transport: unix
      purpose: shell-session-supervisor.v1
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
    endpoints = [
      { name = "shell-service"; transport = "unix"; purpose = "shell-terminal.v3"; }
    ];
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
    endpoints = [
      { name = "supervisor-session"; transport = "unix"; purpose = "shell-session-supervisor.v1"; }
    ];
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

### YAML `Process` annotations field table

The canonical YAML templates above intentionally omit free-form annotations. The provider
must not depend on arbitrary annotations for correctness. If an implementation emits
optional `metadata.annotations`, only the closed keys below are permitted.

| Field | Type | Required | Default | Bound | Description |
| --- | --- | --- | --- | --- | --- |
| `metadata.annotations.d2b.io/provider-role` | `string` | No | None | `shell-terminal-controller|shell-session-supervisor` | Human-stable template role marker. |
| `metadata.annotations.d2b.io/provider-service` | `string` | No | None | `shell-terminal.v3|shell-session-supervisor.v1` | Declared primary service purpose. |
| `metadata.annotations.d2b.io/routing-class` | `string` | No | None | `controller|session-supervisor` | Routing ownership class. |
| `metadata.annotations.d2b.io/redaction-class` | `string` | No | None | `shell-terminal` | Indicates the shell-terminal redaction profile. |
| `metadata.annotations.d2b.io/isolation-warning` | `string` | No | None | `host-none` | Optional warning tag for Host pools with `isolationPosture=none`. |

## ProviderStateSet

A **ProviderStateSet** is the query-time set of all `Volume` resources in a Zone whose
`metadata.ownerRef` resolves to `Provider/shell-terminal`. It is not a ResourceType and
not a stored artifact:

```
ProviderStateSet(zone, "shell-terminal") =
  { v : Volume | v.metadata.zone == zone
              && v.metadata.ownerRef == "Provider/shell-terminal" }
```

Core ProviderDeployment creates every Volume in the ProviderStateSet before the
component Processes start, and deletes them after the component Processes finish and
their finalizers complete. The shell-terminal controller does not own, create, or delete
any Volume in the set. `Provider/volume-local` is the sole Volume reconciler for all
Volumes in the set.

### Component state namespaces

The shell-terminal provider has two semantic components with declared state namespaces:

| Component | Namespace ID | Schema ID | `kind` | `persistenceClass` | Payload | View |
| --- | --- | --- | --- | --- | --- | --- |
| `controller` | `reconcile-state` | `io.d2b.shell-terminal/controller/reconcile-state` | `state` | `persistent` | Empty; `migrationPolicy: none`; reconcile authority is the Zone store (ShellPool/ShellSession resources and core Operation ledger) | read-only |
| `session-supervisor` | `supervisor-state` | `io.d2b.shell-terminal/session-supervisor/supervisor-state` | `state` | `persistent` | Empty; `migrationPolicy: none`; PTY/ring/attach state lives in supervisor process memory only | read-only |

Both namespaces use `kind: state`, `persistenceClass: persistent`, `migrationPolicy: none`,
and read-only views. The Volumes are durable: they survive component process exit,
controller restart, daemon restart, and host reboot, and they participate in the upgrade,
destroy, and reset protocol. Neither Volume stores any payload — the controller uses the
Zone store (ShellPool/ShellSession resources plus the core Operation ledger) as its
reconcile authority, not a private state file. Both Volumes carry a nonzero base
`quotaBytes`, `sourcePolicyId`, and a broker-maintained identity marker.

### Volume naming convention

Volumes follow the `ADR-046-provider-state` naming rule:
`<provider-name>--<component-id>--<namespace-id>--<execution-ref-short>`

| Volume name | Owner component | Execution scope |
| --- | --- | --- |
| `shell-terminal--controller--reconcile-state--<host-short>` | `controller` | One per installed Host target |
| `shell-terminal--supervisor--supervisor-state--<session-uid-short>` | `session-supervisor` | One per live `shell.d2b.io.ShellSession` |

### Controller Volume declaration

Core ProviderDeployment creates the controller Volume before the controller Process
starts. The payload schema is empty; `migrationPolicy: none`; no migration worker is
ever created. The controller mounts a read-only view; it does not write to the Volume.

```yaml
apiVersion: resources.d2b.io/v3
type: Volume
metadata:
  name: shell-terminal--controller--reconcile-state--host-system
  zone: dev
  ownerRef: Provider/shell-terminal
spec:
  providerRef: Provider/volume-local
  kind: state
  persistenceClass: persistent
  sensitivityClass: private
  sourcePolicyId: io.d2b.shell-terminal/controller/reconcile-state
  stateSchema:
    schemaId: io.d2b.shell-terminal/controller/reconcile-state
    schemaVersion: "1.0"
    schemaDigest: sha256:<hex>
    migrationPolicy: none
  quotaBytes: 65536           # 64 KiB base quota; nonzero required; payload is empty
  sealingCredentialRef: null
  source:
    executionRef: Host/host-system
    settings: {}
  layout:
    - path: state
      type: directory
      ownerRef: User/shell-terminal-system
      groupRef: User/shell-terminal-system
      mode: "0700"
      sensitivity: private
      createPolicy: create-if-never-provisioned
      repairPolicy: exact-owner
      cleanupPolicy: owner-controlled
      noFollow: true
  views:
    main:
      path: state
      rights: [read, traverse]      # read-only; controller does not write payload
  identityMarker:
    class: broker-maintained
    markerRoot: provider-state-markers
```

The controller `Process` mounts the controller Volume read-only with `required: true`:

```yaml
mounts:
  - volumeRef: Volume/shell-terminal--controller--reconcile-state--host-system
    view: main
    mountPath: /state
    access: read-only
    required: true
```

The controller receives only this local view dirfd. No other component mounts the
controller Volume. No cross-component Volume sharing is permitted.

### Session-supervisor Volume declaration

Core ProviderDeployment creates one per-session Volume before the supervisor `Process`
starts for each `shell.d2b.io.ShellSession`, and deletes it after the supervisor Process
finishes. The payload schema is empty; `migrationPolicy: none`; no migration worker is
ever created. The supervisor mounts a read-only view; it does not write to the Volume.

```yaml
apiVersion: resources.d2b.io/v3
type: Volume
metadata:
  name: shell-terminal--supervisor--supervisor-state--<session-uid-short>
  zone: dev
  ownerRef: Provider/shell-terminal
spec:
  providerRef: Provider/volume-local
  kind: state
  persistenceClass: persistent
  sensitivityClass: private
  sourcePolicyId: io.d2b.shell-terminal/session-supervisor/supervisor-state
  stateSchema:
    schemaId: io.d2b.shell-terminal/session-supervisor/supervisor-state
    schemaVersion: "1.0"
    schemaDigest: sha256:<hex>
    migrationPolicy: none
  quotaBytes: 65536           # 64 KiB base quota; nonzero required; payload is empty
  sealingCredentialRef: null
  source:
    executionRef: Host/host-system    # or Guest/<name> for Guest pools
    settings: {}
  layout:
    - path: state
      type: directory
      ownerRef: User/<pool-user>      # Nix-preprovisioned User matching ShellPool spec.userRef
      groupRef: User/<pool-user>
      mode: "0700"
      sensitivity: private
      createPolicy: create-if-never-provisioned
      repairPolicy: exact-owner
      cleanupPolicy: owner-controlled
      noFollow: true
  views:
    main:
      path: state
      rights: [read, traverse]      # read-only; supervisor does not write payload
  identityMarker:
    class: broker-maintained
    markerRoot: provider-state-markers
```

The session supervisor `Process` mounts the supervisor Volume read-only with
`required: true`:

```yaml
mounts:
  - volumeRef: Volume/shell-terminal--supervisor--supervisor-state--<session-uid-short>
    view: main
    mountPath: /state
    access: read-only
    required: true
```

The supervisor receives only its own local view dirfd. PTY file descriptors, output ring
bytes, and attach state are held in the supervisor's process memory; nothing is written
to the mounted Volume.

The layout `ownerRef` and `groupRef` bind `User/<pool-user>` — the Nix-preprovisioned
user matching `ShellPool.spec.userRef`. The volume-local Provider validates the inode
owner against this reference before exposing the view. No cross-user Volume is permitted.

### ProviderStateSet invariants

- The set is identified by query: every `Volume` whose `ownerRef == Provider/shell-terminal`
  in the Zone.
- Core ProviderDeployment creates and deletes all Volumes in the set. The shell-terminal
  controller does not own, create, or delete any Volume and does not export `Volume` as
  a ResourceType. `Provider/volume-local` is the sole Volume reconciler.
- Core ProviderDeployment creates the per-session supervisor Volume before the supervisor
  `Process` starts and deletes it after the supervisor `Process` finishes. This is the
  dynamic service state Volume lifecycle for each `shell.d2b.io.ShellSession`.
- No two components share a Volume. Each component mounts only its own declared view
  (local view dirfd only) with `required: true`.
- Both views are read-only (`rights: [read, traverse]`). Neither the controller nor the
  supervisor writes any payload to its Volume. The controller's reconcile authority is
  the Zone store (ShellPool/ShellSession resources and the core Operation ledger).
- Both Volumes carry `sourcePolicyId`, a nonzero base `quotaBytes` (64 KiB), and a
  broker-maintained identity marker. `migrationPolicy: none` on both; no migration
  worker is ever created for either namespace.
- Both Volumes use `kind: state` and `persistenceClass: persistent`. They survive
  component process exit, controller restart, daemon restart, and host reboot, and
  participate in the upgrade, destroy, and reset protocol. Their lifecycle integrity
  (identity marker, markerStatus) is tracked; a `markerStatus: missing` or `replaced`
  causes the dependent component `Process` to enter `Degraded`.
- PTY state, output ring bytes, and attach state are never written to any Volume.

## ComponentSession contracts

The provider defines two ComponentSession services.

| Service | Hosted by | Target resource | Purpose | Noise profile |
| --- | --- | --- | --- | --- |
| `shell-terminal.v3` | `Process/shell-terminal--controller` | `Provider/shell-terminal` | public lifecycle service for pools and sessions | KK or stronger per platform policy |
| `shell-session-supervisor.v1` | `Process/shell-terminal--supervisor--<session-uid-short>` | `shell.d2b.io.ShellSession/<name>` | private per-session service for attach, detach, detach-all, kill, and status | KK |

### Public controller service: `shell-terminal.v3`

| Method | Target | Description | Required role |
| --- | --- | --- | --- |
| `OpenSession` | `shell.d2b.io.ShellPool/<name>` | Create a new `shell.d2b.io.ShellSession`, create its supervisor, and return the route data needed to attach. | `Role/shell-admin` or Zone-admin superset |
| `ListSessions` | `shell.d2b.io.ShellPool/<name>` | List child session summaries for one pool. | `Role/shell-admin` or Zone-admin superset |
| `PoolStatus` | `shell.d2b.io.ShellPool/<name>` | Return aggregate pool counts, phase, and warning state. | `Role/shell-admin` or Zone-admin superset |

#### `OpenSession` request

| Field | Type | Required | Default | Bound | Description |
| --- | --- | --- | --- | --- | --- |
| `poolRef` | `ResourceRef` | Yes | None | `shell.d2b.io.ShellPool/<name>` | Target pool. |
| `sessionName` | `string` | No | controller-generated | kebab-case, max 32 bytes | Optional operator-friendly name copied into `spec.sessionName`. |
| `outputRingCapacity` | `u64` bytes | No | pool default | `4096..1048576` and `<= pool limit` | Optional session-specific ring size. |
| `attachImmediately` | `bool` | No | `true` | Fixed | Whether the caller plans to open the supervisor stream right away. |
| `terminalRows` | `u16` | No | `24` | `1..4096` | Initial PTY row hint. |
| `terminalCols` | `u16` | No | `80` | `1..4096` | Initial PTY column hint. |

#### `OpenSession` response

| Field | Type | Required | Default | Bound | Description |
| --- | --- | --- | --- | --- | --- |
| `sessionRef` | `ResourceRef` | Yes | None | `shell.d2b.io.ShellSession/<name>` | Created session resource. |
| `supervisorRef` | `ResourceRef` | Yes | None | `Process/<name>` | Created supervisor process resource. |
| `supervisorGeneration` | `u64` | Yes | None | Monotonic | Generation that must accompany `Attach`, `Detach`, `DetachAll`, and `Kill`. |
| `service` | `string` | Yes | `shell-session-supervisor.v1` | Exact | Per-session service name. |
| `routeZone` | `ResourceName` | Yes | None | Existing Zone | K0 routing Zone for the supervisor. |
| `phase` | `enum` | Yes | None | Common phase | Initial session phase after creation. |

#### `ListSessions` response summary item

| Field | Type | Required | Default | Bound | Description |
| --- | --- | --- | --- | --- | --- |
| `sessionRef` | `ResourceRef` | Yes | None | `shell.d2b.io.ShellSession/<name>` | Child session resource reference. |
| `phase` | `enum` | Yes | None | Common phase | Current session phase. |
| `detailKind` | `enum` | Yes | None | closed session detail set | Current typed detail kind. |
| `supervisorGeneration` | `u64` | Yes | None | Monotonic | Current attach generation. |
| `attachCount` | `u32` | Yes | None | `0..8` | Current live attachments. |
| `outputRingBytes` | `u64` | Yes | None | `0..1048576` | Current ring fill. |
| `outputRingEvictedBytes` | `u64` | Yes | None | Monotonic | Bytes evicted so far. |

#### `PoolStatus` response

| Field | Type | Required | Default | Bound | Description |
| --- | --- | --- | --- | --- | --- |
| `poolRef` | `ResourceRef` | Yes | None | `shell.d2b.io.ShellPool/<name>` | Pool reference. |
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
| `Attach` | `shell.d2b.io.ShellSession/<name>` | Open a bidirectional terminal stream against the exact supervisor generation. | `Role/shell-admin` or Zone-admin superset |
| `Detach` | `shell.d2b.io.ShellSession/<name>` | Detach the caller's current stream without killing the shell. | `Role/shell-admin` or Zone-admin superset |
| `DetachAll` | `shell.d2b.io.ShellSession/<name>` | Detach all current streams from the exact session without killing the shell. | `Role/shell-admin` or Zone-admin superset |
| `Kill` | `shell.d2b.io.ShellSession/<name>` | Terminate the exact session scope owned by the supervisor. | `Role/shell-admin` or Zone-admin superset |
| `SupervisorStatus` | `shell.d2b.io.ShellSession/<name>` | Return redacted session status from the exact supervisor generation. | `Role/shell-admin` or Zone-admin superset |

#### `Attach` request

| Field | Type | Required | Default | Bound | Description |
| --- | --- | --- | --- | --- | --- |
| `sessionRef` | `ResourceRef` | Yes | None | `shell.d2b.io.ShellSession/<name>` | Target session reference. |
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
| `sessionRef` | `ResourceRef` | Yes | None | `shell.d2b.io.ShellSession/<name>` | Target session. |
| `expectedSupervisorGeneration` | `u64` | Yes | None | current generation | Mandatory stale-handle protection field. |
| `reason` | `enum` | No | `operator-request` | `operator-request|stream-close|maintenance` | Redacted reason code for detach or kill. |
| `graceTimeoutMs` | `u32` | No | `5000` | `0..60000` | Kill grace period. Ignored by `Detach` and `SupervisorStatus`. |

#### `SupervisorStatus` response

| Field | Type | Required | Default | Bound | Description |
| --- | --- | --- | --- | --- | --- |
| `sessionRef` | `ResourceRef` | Yes | None | `shell.d2b.io.ShellSession/<name>` | Target session. |
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
   `shell.d2b.io.ShellSession/<name>`, service `shell-session-supervisor.v1`, named
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
| Target resource | `shell.d2b.io.ShellSession/<name>` |
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
- `shell.d2b.io.ShellSession` UID
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
2. For each candidate, require `ownerRef` to point at a `shell.d2b.io.ShellSession/<name>`.
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
| `OpenSession` | `shell.d2b.io.ShellPool/<name>` | `Role/shell-admin` or Zone-admin superset | No for Host user-domain pools; policy may permit only local admin for Guest pools as well. |
| `ListSessions` | `shell.d2b.io.ShellPool/<name>` | `Role/shell-admin` or Zone-admin superset | No for Host user-domain pools. |
| `PoolStatus` | `shell.d2b.io.ShellPool/<name>` | `Role/shell-admin` or Zone-admin superset | No for Host user-domain pools. |
| `Attach` | `shell.d2b.io.ShellSession/<name>` | `Role/shell-admin` or Zone-admin superset | No for Host user-domain pools. |
| `Detach` | `shell.d2b.io.ShellSession/<name>` | `Role/shell-admin` or Zone-admin superset | No for Host user-domain pools. |
| `DetachAll` | `shell.d2b.io.ShellSession/<name>` | `Role/shell-admin` or Zone-admin superset | No for Host user-domain pools. |
| `Kill` | `shell.d2b.io.ShellSession/<name>` | `Role/shell-admin` or Zone-admin superset | No for Host user-domain pools. |
| `SupervisorStatus` | `shell.d2b.io.ShellSession/<name>` | `Role/shell-admin` or Zone-admin superset | No for Host user-domain pools. |

### Admission rules

1. Only controller principals and authorized Nix emitters may create or mutate stored
   `shell.d2b.io.ShellSession` objects directly.
2. External operators use only `OpenSession`, `ListSessions`, `PoolStatus`, `Attach`,
   `Detach`, `DetachAll`, `Kill`, and `SupervisorStatus`.
3. A relay-authenticated subject is never sufficient for user-domain Host shell access.
4. Anonymous or unauthenticated subjects are denied all shell service verbs.
5. Authorization is checked before capacity and before route lookup so that stale-handle
   rejection never reveals resource existence to unauthorized callers.

## Security invariants

SR-1: `shell.d2b.io.ShellPool` is capacity-only. It never creates a pool-wide supervisor,
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
  type = "shell.d2b.io.ShellPool";
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
  type = "shell.d2b.io.ShellSession";
  spec = {
    providerRef = "Provider/shell-terminal";
    poolRef = "shell.d2b.io.ShellPool/guest-alice-shell";
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
- session declarations whose `poolRef` does not name `shell.d2b.io.ShellPool/<name>`
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

## Implementation work items

| ID | Area | Description | src path | tests path | integration path |
| --- | --- | --- | --- | --- | --- |
| `ADR046-sterm-001` | Resource schemas | Implement `shell.d2b.io.ShellPool` and `shell.d2b.io.ShellSession` schemas with qualified names, common phases, and typed detail fields. | `packages/d2b-provider-shell-terminal/src/resources/{pool,session}.rs` | `packages/d2b-provider-shell-terminal/tests/resource_schema.rs` | `packages/d2b-provider-shell-terminal/integration/resource-shape/` |
| `ADR046-sterm-002` | Controller binary | Implement `d2b-shell-terminal-controller` with pool/session reconcile loops; observe (but do not create) component `Volume` resources provisioned by core ProviderDeployment; block pool readiness on `ControllerStateVolumeMissing`; mount controller Volume view only. | `packages/d2b-provider-shell-terminal/src/bin/d2b-shell-terminal-controller.rs` | `packages/d2b-provider-shell-terminal/tests/controller_reconcile.rs` | `packages/d2b-provider-shell-terminal/integration/controller-restart/` |
| `ADR046-sterm-003` | Supervisor binary | Implement `d2b-shell-session-supervisor` as the sole PTY owner for Host and Guest pools. | `packages/d2b-provider-shell-terminal/src/bin/d2b-shell-session-supervisor.rs` | `packages/d2b-provider-shell-terminal/tests/supervisor_runtime.rs` | `packages/d2b-provider-shell-terminal/integration/supervisor-host-guest/` |
| `ADR046-sterm-004` | Process templates | Teach the Nix compiler and controller to emit the canonical controller and user-domain supervisor `Process` templates. | `packages/d2b-provider-shell-terminal/src/process_templates.rs` | `packages/d2b-provider-shell-terminal/tests/process_templates.rs` | `packages/d2b-provider-shell-terminal/integration/process-placement/` |
| `ADR046-sterm-005` | OpenSession lifecycle | Create sessions from pools, freeze inherited fields, and return `supervisorGeneration` to callers. | `packages/d2b-provider-shell-terminal/src/service/open_session.rs` | `packages/d2b-provider-shell-terminal/tests/open_session.rs` | `packages/d2b-provider-shell-terminal/integration/open-session/` |
| `ADR046-sterm-006` | PTY and ring | Implement the in-memory PTY owner model, bounded ring buffer, replay, and eviction counters; do not create any management worker or `EphemeralProcess`. | `packages/d2b-provider-shell-terminal/src/session/{pty,ring}.rs` | `packages/d2b-provider-shell-terminal/tests/ring_buffer.rs` | `packages/d2b-provider-shell-terminal/integration/ring-overflow/` |
| `ADR046-sterm-007` | Adoption and routing | Implement restart adoption, InvocationID plus cgroup verification, route registration, and stale-generation invalidation. | `packages/d2b-provider-shell-terminal/src/session/adopt.rs` | `packages/d2b-provider-shell-terminal/tests/adoption.rs` | `packages/d2b-provider-shell-terminal/integration/adopt-after-restart/` |
| `ADR046-sterm-008` | Host rules | Emit Host `isolationPosture=none` warnings, same-UID verification, and relay denial for Host user-domain pools. | `packages/d2b-provider-shell-terminal/src/host_rules.rs` | `packages/d2b-provider-shell-terminal/tests/host_rules.rs` | `packages/d2b-provider-shell-terminal/integration/host-warning/` |
| `ADR046-sterm-009` | Guest rules | Require Guest `allowedDomains` to include `user`, require `defaultUserRef`, and place supervisors through the Guest user manager. | `packages/d2b-provider-shell-terminal/src/guest_rules.rs` | `packages/d2b-provider-shell-terminal/tests/guest_rules.rs` | `packages/d2b-provider-shell-terminal/integration/guest-user-domain/` |
| `ADR046-sterm-010` | RBAC and relay denial | Gate all verbs on `Role/shell-admin` or Zone-admin superset and fail closed for relay-authenticated Host user-domain callers. | `packages/d2b-provider-shell-terminal/src/authz.rs` | `packages/d2b-provider-shell-terminal/tests/authz.rs` | `packages/d2b-provider-shell-terminal/integration/authz/` |
| `ADR046-sterm-011` | Audit and telemetry | Implement closed-label metrics, redacted spans, and audit events with no usernames, session names, paths, or terminal bytes. | `packages/d2b-provider-shell-terminal/src/{audit,telemetry}.rs` | `packages/d2b-provider-shell-terminal/tests/redaction.rs` | `packages/d2b-provider-shell-terminal/integration/support-redaction/` |
| `ADR046-sterm-012` | Baseline removal | Delete superseded guestd shell runtime, unsafe-local helper shell supervisor, and public-wire `ShellOp` or `ShellOpResponse` shell protocol. | `packages/d2b-provider-shell-terminal/src/migration.rs` | `packages/d2b-provider-shell-terminal/tests/migration.rs` | `packages/d2b-provider-shell-terminal/integration/migration-baseline/` |
| `ADR046-sterm-013` | Supervisor service | Define and implement `shell-terminal.v3` and `shell-session-supervisor.v1` ComponentSession services and the named `terminal` stream contract. | `packages/d2b-provider-shell-terminal/src/service/{controller,supervisor}.rs` | `packages/d2b-provider-shell-terminal/tests/service_contract.rs` | `packages/d2b-provider-shell-terminal/integration/service-contract/` |

## Baseline reuse and removal

This rewrite deliberately reuses narrow primitives and removes broad, incorrect baseline
assumptions.

| Area | Reuse or removal | Source or target | Notes |
| --- | --- | --- | --- |
| ring buffer mechanics | Reuse | `packages/d2b-guestd/src/shell.rs` | Reuse bounded ring ideas only; move ownership into the per-session supervisor and keep bytes out of the controller. |
| scope adoption pattern | Reuse | `packages/d2b-unsafe-local-helper/src/runtime.rs` `ScopeRuntime` and `PersistedScope` | Reuse verification and adoption shape only; do not reuse protocol, helper identities, or state storage assumptions. |
| service starting point | Reuse | `a1cc0b2da4a08ca3240a770a972fe4da6f912bef` `packages/d2b-contracts/src/generated_v2_services/shell.rs` and `shell_ttrpc.rs` | Reuse service-shape ideas only; exclude ADR 0045 session, realm, and constellation assumptions. |
| Guest persistent-shell runtime | Remove | `packages/d2b-guestd/src/shell.rs` `ShellRuntimeConfig` and related managed runtime flow | Superseded by `shell.d2b.io.ShellSession` plus per-session supervisors. |
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

## Appendix A: canonical example resources

```yaml
apiVersion: resources.d2b.io/v3
type: shell.d2b.io.ShellPool
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
  detail:
    kind: IsolationPostureWarning
    message: host execution target reports non-isolated posture
  conditions:
    - type: Ready
      status: "True"
    - type: IsolationPostureWarning
      status: "True"
  executionRef: Host/workstation
  userRef: User/alice
  activeSessions: 1
  attachedSessions: 1
  capacityRemaining: 3
  attachedCapacityRemaining: 0
  isolationPosture: none
```

```yaml
apiVersion: resources.d2b.io/v3
type: shell.d2b.io.ShellSession
metadata:
  name: host-alice-shell-main
  zone: personal
  ownerRef: shell.d2b.io.ShellPool/host-alice-shell
spec:
  providerRef: Provider/shell-terminal
  poolRef: shell.d2b.io.ShellPool/host-alice-shell
  executionRef: Host/workstation
  userRef: User/alice
  loginShellRef: artifact://shells/bash-login
  sessionName: main
  outputRingCapacity: 262144
  desiredLifecycle: running
status:
  observedGeneration: 3
  phase: Ready
  detail:
    kind: ReadyAttached
  conditions:
    - type: Ready
      status: "True"
    - type: SupervisorReady
      status: "True"
    - type: RouteRegistered
      status: "True"
    - type: Attached
      status: "True"
  supervisorRef: Process/shell-terminal--supervisor--0d3b0e42
  supervisorGeneration: 7
  attachCount: 1
  outputRingBytes: 65536
  outputRingEvictedBytes: 8192
```

## Appendix B: controller and supervisor interaction sequence

| Step | Actor | Action |
| --- | --- | --- |
| 1 | CLI | Call `OpenSession` on `shell-terminal.v3` with a pool reference. |
| 2 | Controller | Validate pool capacity, resolve inherited execution and user fields, create `shell.d2b.io.ShellSession`. |
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

1. All stored type strings are qualified: `shell.d2b.io.ShellPool` or `shell.d2b.io.ShellSession`.
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
