# Feature Specification: Complete the ADR-046 Provider Control Plane (d2b 3.0)

**Feature Branch**: `001-adr046-d2b3-completion` (spec directory; implementation lands on per-wave stacks cut from `v3`)

**Created**: 2026-07-29

**Status**: Draft

**Input**: User description: "I want to create a spec for finishing implementation of ADR-046 (docs/adr) - d2b 3.0. W0-W1 have been implemented and merged into the v3 branch. there are detailed specs for it in docs/specs."

## Context

ADR 0046 and its 55-member normative specification set define d2b 3.0: a Zone-scoped,
resource-oriented control plane in which every host capability is a declared resource,
reconciled by controllers, and implemented by a pluggable Provider. The specification set
is Accepted; the parent ADR delivered documentation only and states that implementation
requires a separate request. This specification is that request.

Two of the nine delivery waves are complete and merged. Wave W0 landed the identity,
object-model, store-contract, and resource-API foundations. Wave W1 landed the
reconciliation toolkit, ComponentSession runtime, transport substrate, Zone message bus,
and the storage feasibility proof. At program opening, 14 of 545 enumerated work items were
`Merged` and 531 were `Planned` across waves W2 through W7. That 531-item initial scope is
preserved in the primary task set. At committed HEAD
`868469bf9c293cd48fff483717f14cb88c246821`, the authoritative manifest records 68 `Merged`
and 477 `Planned`. The terminal wave W8 has no work items yet by design: its contents are the
delivery friction accumulated across the program and are recorded at W7 close, so the
program's final total exceeds 545.

The decisive gap is that **none of the delivered foundation is reachable by an operator**.
Every W0/W1 crate is deliberately test-only and unwired from production: the bus
registration path denies every peer outside test builds, effect release depends on a
commit proof that only test code can issue, the resource API has no registered transport
dispatch, and the durable store has schema and codecs but no engine. No shipped binary
depends on any of it. Finishing ADR-046 means turning that foundation into a live control
plane an operator can actually use, replacing the pre-ADR-046 control plane, and shipping
the result as d2b 3.0.

One known blocking result carries forward: the storage feasibility spike passed six of
seven thresholds but missed its whole-process resident-memory budget by roughly 2.6
percent. The delivery contract requires a failed hard target to be resolved by changing the
design, never by weakening durability, authorization, or audit. The production storage
engine and its watch consumer were therefore deferred from W1 into W5 pending named design
corrections.

One process gap also carries forward: W0 and W1 merged through reviewed pull requests
without producing the delivery contract's sealed wave records, so no wave is currently
sealed. This specification accepts that as a one-time documented waiver and begins sealed
delivery at W2.

### Approved Wave 5 production-completion amendment (2026-08-06)

The preceding Context is retained as the feature's historical starting record. The committed
tree has moved beyond it: the production redb backend now exists in
`packages/d2b-resource-store-redb`, which directly depends on redb `=4.1.0`. The disposable
`proofs/redb-resource-store-spike` workspace separately retains its provisional `=4.1.0`
pin and quarantine under D128; that quarantine does not apply to the production crate. A
store watch primitive, a controller fan-in fixture, and a fail-closed daemon runtime skeleton
also exist. They do not make the resource plane production-reachable. The daemon still opens
the store with mutable revision identities pinned to bootstrap constants, installs no Zone
policy, registers no authenticated ComponentSession route or controller endpoint, admits no
production watch, and leaves the mutation audit outbox without a production drainer.
Existing RSS and watch fixtures exercise in-process services or a fixed fixture endpoint,
not the published daemon boundary.

The operator has approved the missing production wiring as Wave 5 work. It is not deferred
to W6 or W7, and a readiness bit, direct `WatchService` call, fake endpoint, disabled audit
callback, or test-only subject may not substitute for the real path. This is an explicit
scope amendment to the feature artifacts, not a rewrite of the earlier historical evidence.
It preserves ADR 0034 restart/adoption semantics, ADR 0046's Zone trust boundaries, D106's
store boundary, and the daemon-only end state. No new ADR is required because this amendment
assigns implementation ownership for already-decided boundaries rather than choosing a new
trust model.

**Approved C1 correction**: Constitution 2.2.0 permits an approved plan, specification, or
contract defect to be corrected in the same coordinated change as the affected contract
implementation. The accepted `ADR-046-provider-system-core` member specification currently
uses `system_core_host` and `system_core_user` for both internal telemetry labels and
serialized status names, while the committed v3 `ZoneHandlerName` closed enum uses kebab-case
wire serialization and omitted those variants. T605 owns the coordinated correction: add
`ZoneHandlerName::SystemCoreHost` and `ZoneHandlerName::SystemCoreUser`, serialize them only as
`system-core-host` and `system-core-user`, retain the underscore spellings only as internal
telemetry labels, bump both governing normative specification versions, and update every
paired compiler-derived API snapshot, Rust serialization and duplicate/underscore-rejection
test, lowest-layer contract/policy guard, and reference status surface. T595 consumes the two
variants in the production emitter, and T599 reconciles the remaining status consumers. All
paired normative specs and version metadata, Rust contract/tests, API snapshots, reference
status docs, consumers/emitters, generator no-drift proof, and panel evidence land in the
same Wave 5 PR.

The C1 correction itself adds no field or operation and changes no desired-state ResourceType
schema. Therefore it requires no `apiVersion`, JSON `schemaVersion`, `manifestVersion`,
`bundleVersion`, or C1-specific wire-field version bump.
The Zone desired-spec artifact
`docs/reference/schemas/v3/core.d2bus.org_Zone.schema.json` remains unchanged and T605 must
prove generator output is byte-identical rather than hand-editing it. C1 is resolved in these
feature artifacts but is not implemented. The plan is eligible for read-only cross-artifact
analysis at clean pre-validator base A and feature snapshot P0 and, if that has no HIGH or
CRITICAL finding, a unanimous plan panel bound to A/P0. Those gates authorize only T603's
validator implementation. T603 then lands validator-only commit V, freezes resume base B
exactly at V, and MUST rerun analysis and the plan panel at B/P before it may create the
reconciliation receipt or authorize any checkbox edit. T589 remains gated on those
post-validator receipts and T603 progress reconciliation.

## Clarifications

### Session 2026-07-29

- Q: When d2b 3.0 removes the v2 command surface, what should happen to the sibling desktop
  companions that consume d2b's public CLI and socket contracts? -> A: Coordinated sibling
  updates are a release blocker; 3.0 does not ship until compatible companion versions
  exist.
- Q: Is "every operator-facing capability that exists today can still be obtained after the
  program" a hard release gate or a best-effort goal? -> A: Enforced with exceptions.
  Parity is required wherever a successor was promised; a capability may be retired only if
  explicitly listed, justified, and documented in the release notes.
- Q: Before an operator runs the irreversible phase of the cutover, should the system
  require proof that a full host backup or snapshot exists? -> A: Require explicit
  attestation. The operator must confirm a recovery point exists before any step past the
  rollback boundary executes, and the attestation is recorded.
- Q: Which machine should the mandatory live-host and hardware validation run on before 3.0
  ships? -> A: The daily-driver host, the machine actually in use.
- Q: Should intermediate pre-releases be published during the program? -> A: No. Nothing
  ships until 3.0 final. Every wave must still land through a pull request that is merged
  only after that wave's gates pass.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Declare a capability and watch it become real (Priority: P1)

An operator describes what they want in their host configuration - an isolated Zone
containing a guest, a volume, a network, and a device - and activates it. The control
plane accepts the declaration, records it durably, reconciles each declared item toward
its desired state, and reports progress. When the operator removes an item from the
configuration, the control plane retires it safely rather than orphaning it. Nothing about
this requires the operator to edit framework source code or to know which component
implements the capability.

**Why this priority**: This is the entire premise of ADR-046. Until a declared resource
can travel from configuration to durable storage to a reconciling controller to a live
effect, none of the delivered foundation produces operator-visible value, and no later
story can be demonstrated. It is the smallest slice that turns the test-only foundation
into a working system.

**Independent Test**: Declare a Zone containing a small set of resources on a test host,
activate the configuration, and confirm each resource reaches a ready state and reports
accurate status. Remove one resource, reactivate, and confirm it is retired cleanly with
visible progress. This is demonstrable without any Provider family beyond the minimum
needed to satisfy the declared resources, and without performing a host cutover.

**Acceptance Scenarios**:

1. **Given** a host with no prior Zone state, **When** the operator activates a
   configuration declaring a Zone and its resources, **Then** the Zone initializes, every
   declared resource is durably recorded, and each reaches a ready state or reports a
   specific, actionable failure reason.
2. **Given** a Zone with live resources, **When** the operator removes one resource from
   the configuration and reactivates, **Then** the removed resource is retired in
   dependency-safe order, its cleanup status is visible while in progress, and unrelated
   resources are unaffected.
3. **Given** a Zone with live resources, **When** the host restarts, **Then** the Zone
   recovers its recorded state, resumes reconciliation, and re-establishes live resources
   without operator intervention and without losing durable state.
4. **Given** two resources where one depends on the other, **When** the dependency is not
   yet ready, **Then** the dependent resource waits and reports why, rather than failing
   permanently or acting on incomplete state.
5. **Given** a component that is not authorized for a resource, **When** it attempts to
   read or modify that resource, **Then** the attempt is refused and the refusal is
   recorded in the audit trail.

---

### User Story 2 - Get host capabilities through declarative Providers (Priority: P2)

An operator gains graphics, audio, storage, networking, device passthrough, credentials,
transport, clipboard, notifications, shells, and observability by declaring the
corresponding resources, not by toggling framework-internal feature switches. Each
capability is supplied by a Provider that the control plane installs, supervises, and
holds accountable for the state it owns. An operator can see which Provider owns a
capability and what state it reports.

**Why this priority**: Story 1 proves the plane works; this story is what makes it useful
enough to replace what operators have today. It is the largest single body of remaining
work and depends entirely on Story 1's contracts being settled first.

**Independent Test**: With the resource plane live, declare one resource from each Provider
family on a host that has the relevant hardware or services, and confirm the capability
functions end to end and reports ownership and status accurately. Each Provider family is
independently demonstrable; a missing family does not block the others.

**Acceptance Scenarios**:

1. **Given** a live Zone, **When** the operator declares a capability that a Provider
   implements, **Then** the Provider is installed and supervised by the control plane and
   the capability becomes usable without further manual steps.
2. **Given** a declared capability whose Provider fails to start or reconcile, **When** the
   operator inspects the resource, **Then** the reported status names the owning Provider
   and a specific failure reason, and the failure does not cascade to unrelated resources.
3. **Given** a Provider that owns durable state, **When** the host restarts or the Provider
   is restarted, **Then** the Provider re-adopts its state rather than recreating or
   destroying it.
4. **Given** a capability that requires privileged host mutation, **When** the Provider
   performs it, **Then** the mutation flows through the audited privileged path and is
   recorded, and the Provider never receives a raw host path or unmediated privilege.

---

### User Story 3 - Move an existing host onto 3.0 exactly once (Priority: P3)

An operator running the pre-ADR-046 control plane converts their host to the 3.0 control
plane through a single, deliberate, one-time procedure. They can preview exactly what will
be adopted, preserved, and destroyed before anything changes. They must give explicit
content-bound consent before any destructive step. Irreplaceable state - notably
device-identity material such as software TPM contents - is carried forward rather than
discarded. Up to a defined boundary the procedure can be rolled back.

**Why this priority**: Without this, 3.0 is only reachable by rebuilding a host from
scratch, and existing operators cannot adopt it. It depends on Stories 1 and 2 because the
cutover's destination must exist and be trustworthy before anyone is asked to cross to it.

**Independent Test**: On a host carrying representative pre-ADR-046 state, run the preview
mode and confirm the plan enumerates every affected artifact with its disposition. Then run
the procedure and confirm the host ends on the 3.0 control plane with preserved state
intact and no remnants of the superseded plane.

**Acceptance Scenarios**:

1. **Given** a host on the pre-ADR-046 control plane, **When** the operator requests a
   cutover preview, **Then** they receive a complete plan listing each affected artifact,
   its disposition, the preserved set, the rollback boundary, and the required consent
   text, and nothing is modified.
2. **Given** a cutover preview, **When** the operator does not supply the exact consent
   text and explicit apply intent, **Then** no destructive step executes.
3. **Given** an applied cutover, **When** it is inspected afterwards, **Then**
   identity-bearing state designated for preservation is intact and usable, and no
   superseded control-plane unit, command surface, or configuration namespace remains
   active.
4. **Given** a cutover interrupted before its rollback boundary, **When** the operator
   rolls back, **Then** the host returns to its prior working control plane.
5. **Given** an operator-set hold during a cutover window, **When** a destructive step is
   reached, **Then** it is blocked until the hold is cleared.
6. **Given** a cutover that has reached its rollback boundary, **When** the operator has not
   supplied one current, qualified recovery-point attestation bound to the exact candidate,
   commit, tree, preview inventory, and daily-driver host, **Then** no further step executes
   and the refusal names the missing or mismatched field and the action to create, verify,
   and attest a new recovery point.

---

### User Story 4 - Receive d2b 3.0 as a supported release (Priority: P4)

A consumer of d2b can adopt 3.0 the same way they adopt any other version: a tagged
release with summarized, consumer-readable notes, documentation that matches the shipped
behavior, and no half-migrated internals. The release does not carry both the superseded
control plane and its replacement. Adopting 3.0 does not degrade the operator's desktop:
the companion tools that sit on top of d2b's operator contracts have compatible versions
ready at release.

**Why this priority**: This converts completed work into something consumable. It is last
because it can only be evaluated against the final tree, after every preceding story has
landed and the superseded paths have been removed.

**Independent Test**: Inspect the released version for consumer-readable notes free of
internal process bookkeeping, documentation matching shipped behavior, and absence of the
superseded control-plane paths. Install the release on a clean host from published
artifacts, complete Story 1, and exercise each desktop companion against it.

**Acceptance Scenarios**:

1. **Given** the completed program, **When** the release is cut, **Then** it carries a new
   version entry summarized for consumers with all internal wave, phase, and finding
   markers removed.
2. **Given** the released tree, **When** it is searched for superseded control-plane paths
   scheduled for removal, **Then** none remain.
3. **Given** the released version, **When** a consumer follows the published documentation,
   **Then** the documented behavior matches the shipped behavior.
4. **Given** a host running the release candidate, **When** the operator uses each desktop
   companion that consumes d2b's public operator contracts, **Then** each works against the
   new contracts, and any companion that does not blocks the release rather than shipping
   broken.

---

### Edge Cases

- A declared resource references another resource that does not exist, is in a different
  Zone, or was deleted mid-reconcile.
- Two components attempt to modify the same resource concurrently, or a component submits a
  change based on a stale view.
- The host loses power during a durable write, or during an effect that has been decided
  but not yet released.
- A control-plane process restarts while resources are live: it must resume ownership of
  what already exists rather than duplicating, orphaning, or destroying it.
- A Provider crashes repeatedly, hangs, or reports success while its underlying capability
  is broken.
- A Provider is removed from configuration while resources still depend on it.
- Resource volume grows well beyond the expected working set, or many watchers observe the
  same resources at once.
- The durable store is corrupt, unreadable, was produced by a different identity, or is at
  an older schema version.
- A cutover is interrupted at each distinct phase, including after its rollback boundary.
- Preserved identity material is missing or altered at cutover time, which must fail closed
  rather than silently reinitialize a fresh identity.
- Resource-plane memory or latency budgets are exceeded under load.
- A wave's validation evidence, panel record, or work-item state does not match the exact
  tree being sealed.
- Every release condition is met except that one desktop companion still has no compatible
  version, so the release must hold rather than ship a degraded desktop.
- The operator cannot or will not attest that a recovery point exists once the cutover
  reaches its rollback boundary, leaving the host on the prior control plane indefinitely.
- A capability turns out to have no successor only after its superseded path is removed,
  which must surface as a parity failure rather than a silent disappearance.

## Requirements *(mandatory)*

### Functional Requirements

#### Live resource plane

- **FR-001**: The control plane MUST accept operator-declared resources from host
  configuration, record them durably, and make them retrievable with their observed status.
- **FR-002**: The control plane MUST reconcile every declared resource toward its declared
  state continuously, and MUST report progress, readiness, and failure reasons per resource.
- **FR-003**: The control plane MUST survive restart and power loss without losing
  committed state, and MUST resume reconciliation and re-adopt live resources on startup
  rather than recreating or destroying them.
- **FR-004**: The control plane MUST reject a change submitted against a stale view of a
  resource, and MUST resolve concurrent modification without silent data loss.
- **FR-005**: The control plane MUST retire a removed resource in dependency-safe order,
  MUST expose cleanup progress while it is in flight, and MUST NOT broadly sweep resources
  it did not create.
- **FR-006**: An external effect MUST NOT be released until the corresponding state change
  is durably committed and proven, including across restart, abort, and conflict.
- **FR-007**: Every resource operation MUST be authorized against the requesting
  component's proven identity, and every authorization decision that denies access MUST be
  recorded in the audit trail.
- **FR-008**: Components MUST obtain access only through an authenticated session bound to
  a single owner, and the control plane MUST refuse any component that names its own
  identity rather than proving it.
- **FR-009**: Resources MUST NOT be referenced across Zone boundaries except through the
  explicit, declared linking mechanism, and cross-Zone access MUST be default-denied.
- **FR-066**: Wave 5 MUST publish the production Resource API only through an authenticated,
  single-owner ComponentSession admitted by the authoritative Zone registrar and routed by
  the ZoneBus. The registrar MUST derive the subject from verified peer evidence in its
  private state and propagate that authoritative subject through every Resource API
  operation. Unix peer evidence MUST obtain the process descriptor directly from the accepted
  socket with `SO_PEERPIDFD`; opening a pidfd later from `SO_PEERCRED.pid` is forbidden.
  Credential, process-generation, cgroup, and liveness evidence MUST be verified against that
  exact `CLOEXEC` fd and consumed by one registrar-private issuer. Unavailable kernel support,
  numeric-PID reuse, dead-fd or evidence mismatch, or ambiguity denies admission. Public peer
  credentials/evidence accessors and bootstrap-identity construction, verification, cloning,
  or conversion paths are forbidden. A caller-supplied
  subject, daemon peer role treated as a resource subject,
  unauthenticated direct service call, fixed fixture endpoint, or readiness flag MUST NOT
  satisfy this requirement.
- **FR-067**: Wave 5 MUST establish `ZoneResourceRuntime` as the one Zone resource-policy
  owner in the daemon-owned Zone runtime. Initial installation and restart recovery MUST use
  one private, sealed, non-`Clone`, non-`Copy`, one-shot `PolicyBootstrapRead` capability
  owned by that runtime and minted only by one private issuer. It MUST expose no public
  constructor, field, accessor, `Default`, conversion, capability trait implementation, or
  reconstruction path, and compiler/API-surface/external compile-fail seals MUST enforce
  those absences. The capability MAY read only the Zone's policy-input resource envelopes at the
  exact durable nonzero policy revision needed to construct the first immutable `PolicySet`;
  it MUST carry no public Resource API subject, expose no general resource read or mutation
  operation, and become unusable when that installation attempt consumes it.
  `d2b-resource-api` MUST deserialize and compile those envelopes and install the resulting
  set in `NativeAuthorizer`; neither store crate may interpret them. After the first set is
  installed, every normal policy read and update MUST traverse an authenticated Resource API
  session and its revision checks. A revision advance MUST compile the exact committed
  revision before atomically replacing the installed set. Initial install, revision advance,
  and restart recovery publish policy readiness only when Zone UID and installed revision
  match live durable metadata. Missing, stale, cross-Zone, structurally invalid, or
  un-compilable bootstrap input MUST consume the attempt, leave the Zone unpublished and
  degraded, and name the policy remediation; it MUST NOT fall back to a constant, partial
  policy, caller claim, or reusable bootstrap reader. The resource store and redb backend
  MUST remain policy-neutral.
- **FR-068**: Wave 5 MUST register the production controller endpoint and fan-in, and MUST
  bind every committed controller effect and cleanup intent to one durable replay/adoption
  ledger before releasing the effect. The ledger identity MUST include the resource UID,
  controller generation, committed revision, operation identity, and effect ordinal.
  Restart after generation commit MUST replay or adopt every outstanding effect without
  losing cleanup intent. Cleanup completion MUST compare the same resource UID and an exact,
  nonzero expected revision; a stale revision, zero revision, UID mismatch, or ambiguous
  adoption MUST fail closed without releasing or completing the effect.
- **FR-069**: Wave 5 MUST admit watches through the authenticated, exact-Zone Resource API
  route, ZoneBus, production store, and registered controller fan-in without a replay/live
  gap. One authoritative readiness projection MUST cover store recovery, matching installed
  policy, authenticated session/router admission, registered controller endpoint, admitted
  watch cursor, caught-up durable audit, mandatory controller health, and the
  `d2b-core-controller`-owned registration for `Provider/system-core`. That Provider member
  is healthy only while `Zone.status.handlers[]` contains exactly one record whose `name` is
  `system-core-host` and exactly one record whose `name` is `system-core-user`. Each record
  carries `phase` and `lastReconciledAt` and is backed respectively by the live owned,
  active, initialized, current `HostReconciler` and `UserReconciler` handle.
  `ZoneHandlerName::ProviderLifecycle` remains a distinct aggregate handler name and cannot
  substitute for either required record. A missing, duplicate, wrong-name, inactive,
  uninitialized, or stale record MUST leave only that Zone unpublished and degraded with a
  specific remediation. Wave 5 does not wait for the remaining Wave 6 Provider dossiers.
  Partial publication, a bare readiness boolean, or a status value without the live owned
  registration and handler handles is forbidden.
- **FR-070**: Wave 5 MUST provide one production audit owner per Zone runtime. The same
  transaction that commits each privileged resource mutation MUST create an immutable
  authoritative journal row for each bounded mutation ordinal. Segment export completion is
  separate mutable state and MUST NOT delete or rewrite an unexported authority; deletion is
  permitted only after durable export completion plus the configured journal-retention
  interval. Journal, segment, and
  export records MUST use domain-separated fixed digests for operation, correlation,
  authoritative subject, Zone, and resource identifiers; raw values MUST remain private and
  absent from audit output, errors, logs, metrics, spans, and redacted `Debug`. Raw propagated
  trace context MUST remain private; an authoritative row or export may retain only its typed
  domain-separated fixed digest. Audit constructors MUST accept typed fixed digests rather
  than raw identifiers, and encoded records MUST reject bytes beyond the fixed limit. Replay
  after restart MUST be idempotent by fixed operation digest and mutation ordinal.
  `audit.retentionDays`, default 30 and range 1 through 3650, MUST govern segments and
  export-completed journal rows; `audit.maxRecordsPerSegment`, default 65536 and range 1
  through 1000000, and `audit.maxSegmentBytes`, default 67108864 and range 1048576 through
  1073741824, MUST be enforced at startup and rotation. Prune, limit, file-sync, or
  directory-sync failure MUST produce typed degraded health and block publication. The
  unprivileged Zone runtime MUST own drain sequencing but route every root-owned filesystem
  effect through one typed broker op carrying only fixed-digest bounded records. The root
  broker alone owns `SegmentWriter`; append, rotation, export, and prune MUST remain under one
  root-owned held directory fd with fd-relative operations. No service or unit is added.
  Mutation success MUST NOT be acknowledged until the required
  segment file and directory, export, and completion state are durable. If export cannot finish after
  the mutation and authoritative row commit, the API MUST NOT return ordinary success or imply
  rollback. It MUST return `CommittedPendingAudit` through the layered `ResourceStatus`
  composite: `ResourceStatus.phase` is
  `ResourcePhase::Degraded`; `ResourceStatus.outcome.code` is
  `StatusCode("committed-pending-audit")` with retryable, safe remediation and no raw sink
  detail; `ResourceStatus.update.state` is `UpdateState::Blocked`; and
  `ResourceStatus.update.operation_id` is `Some(original_operation_id)`. Existing bounded,
  redacted condition, outcome, and update fields carry only the semantic status and
  instructions to retry with the same ID or inspect status. They MUST NOT expose a subject,
  mutation payload, raw sink error, or a claim that the commit was undone. The affected Zone
  MUST remain unpublished and degraded until export completes. Before same-ID status or
  resumption, the implementation MUST match a persisted replay-binding digest over the
  registrar-derived subject, Zone, canonical semantic request, target, verb, exact expected
  revision, operation ID, and idempotency data. Cross-subject, cross-Zone, altered-request,
  target, verb, revision, idempotency, or restart mismatch MUST be denied and audited without
  observation or reapplication. An exact retry returns the same pending state while export is
  incomplete and its one stored final result after recovery. A different operation ID follows
  ordinary expected-revision and conflict semantics. Every mutation response, including
  `DeleteResponse` and batch ordinals, MUST represent the composite with the additive bounded
  protobuf `PendingAuditStatus`; ordinary success omits it. This changes the ResourceService
  schema fingerprint but not Resource JSON `apiVersion` or `schemaVersion`, and
  `ResourceUpdateStatus` does not acquire a phase or status-code member. An unavailable or
  disabled audit owner, missing authoritative row, incomplete export, dropped record, or
  unbound record MUST fail closed.
- **FR-071**: Persisted store, policy, active-configuration, and controller identities MUST
  reopen after their mutable revisions advance. Immutable store and Zone identity MAY be
  checked at open, but mutable revisions MUST be recovered from durable state rather than
  pinned to bootstrap constants. Startup and shutdown MUST visit every declared Zone:
  failure in one Zone MUST leave that Zone unpublished and visibly degraded while unrelated
  Zones continue, and a close failure MUST NOT silently drop later stores or their owners.
  Recovery and cleanup MUST retain ADR 0034's adopt-before-cleanup rule.
- **FR-072**: Before T219 may begin, Wave 5 MUST hold exact-candidate evidence for all of the
  following: authenticated cross-Zone denial and same-Zone watch delivery through production
  boundaries; restart crash windows for effect replay/adoption and cleanup stale, zero, and
  UID-mismatch refusals; durable audit drain and restart replay; whole-process RSS and
  single-owner fan-in at 10,000 resources and 100 watches; current removal proofs; and
  reference documentation compared with emitted behavior; and T605's exact enum
  round-trip, handler-list duplicate/missing/wrong-name, `ProviderLifecycle` non-substitution,
  API-snapshot, paired-reference, and unchanged Zone desired-schema drift results. T604 MUST
  additionally prove on that same candidate that an operator Nix declaration for the
  representative Guest, Volume, Network, and Device emits the installed per-Zone bundle,
  activates on initial startup and public declaration/removal NixOS switches without manual
  daemon restart or private reload, reaches a real owned effect and readiness for every
  supported representative resource, and removes one declaration with dependency-safe
  cleanup while unrelated resources remain ready and intact. Actionable refusals are separate
  negative cases and cannot satisfy this positive story. Direct `WatchService` calls, fixed
  or fake endpoints, test-only subject injection, stale evidence from an older tree, and
  historical proof artifacts are not evidence for this gate. T220 MUST converge all slice and
  integrator-owned generated-artifact changes, including T605/T595/T599 reconciliation and
  the full drift gate, before freezing final candidate F. Exact-candidate evidence MUST use
  this closed feature-local `EvidenceRecord.validation` set:
  `production-session-watch`, `effect-replay-cleanup`, `audit-drain-replay`,
  `system-core-handler-contract`, `operator-nix-activation-cleanup`,
  `resource-plane-rss-owner-fanin`, `wave5-removal-proofs`, and
  `cli-reference-conformance`. T600 exclusively owns the first five; T601 exclusively owns
  the final three. Before F freezes, T589 MUST implement one hermetic closed-profile validator
  used by panel-request, seal, and merge-eligibility, with negative tests for missing, extra,
  duplicate, unknown, wrong-lane, and conflated mappings. T602 MUST invoke that validator and
  MUST require all eight records to bind F and F's tree. T219 alone
  runs F's one binding panel, seal, and merge after T602; no content, evidence identity, or
  candidate change is permitted under F after its request, and F can never receive a second
  request. A nonunanimous F is retained as failed, its recommendations alone scope the fix
  round, and a distinct successor must repeat T220 and T600-T602 plus the delta/full-context
  follow-up panel before receiving its own one request. An external policy/tooling refusal is
  an integrator scope escalation, never a finding waiver. Before
  T603 implementation, clean base A and feature snapshot P0 MUST pass current cross-artifact
  analysis with no unresolved HIGH or CRITICAL finding and a unanimous plan panel that
  authorizes only `packages/xtask/src/delivery/{mod.rs,resume.rs}`. T603 MUST land one
  validator-only commit V with sole parent A and freeze resume base B exactly at V; feature
  snapshot P MUST be byte-identical to P0. Analysis over A..B plus the full feature artifacts
  and a plan panel bound to B/P MUST both rerun after V. Any finding or validator-code change
  invalidates B and both post-validator receipts. A source-only fix MUST create a new V/B and
  rerun both post-validator gates; a required feature-artifact edit MUST return to a fresh
  `/d2b-spec-edit` batch, establish a new A/P0, and rerun the complete two-pass sequence. Only
  then may T603 write the closed
  immutable external authorization receipt at
  `.scratch/autopilot/adr046w5/reconciliation.json`, account for exactly every T073-T218
  obligation against clean resume base B and delivery records, bind opaque project sentinel
  `7f6d0beab0ce4c13a89f6865d5ac42e2`, the Git-discovered root, and a
  repository-relative feature path without a hosting domain, account, remote URL, or checkout
  path, and bind both post-validator cross-artifact analysis
  and unanimous plan-panel receipts to B and pre-edit snapshot P. The validator MUST derive the
  sole authorized post-edit snapshot Q for the 147 checkbox changes. T605 appears only as
  future work after resume, never as a 147th obligation row or 148th checkbox transition. If
  any row is open, T603 remains unchecked and no checkbox changes. Only 146 satisfied rows,
  clean B/P identity, analysis with no unresolved HIGH or CRITICAL finding, and unanimous
  plan signoff authorize `/d2b-spec-edit`. The Wave 5 integrator alone owns dedicated checkbox
  commit C, whose sole parent MUST be B and whose only diff MUST be P-to-Q. The prepare,
  apply, and finalize protocol MUST resume safely from exact B/P, B/Q, or C/Q and refuse every other
  state. T589 MUST require the finalized progress receipt, clean HEAD C, and those checked
  boxes. Before F freezes, T589's hermetic closed-profile validator is wired into
  panel-request, seal, and merge-eligibility and its exact-eight positive plus missing, extra,
  duplicate, unknown, wrong-lane, and conflated negatives pass. T602 later invokes that same
  validator and validates B/P, C/Q, exact `C^ = B`, C as an ancestor of final candidate F
  frozen by T220, the exact eight-record T600/T601 closed set bound to F and F's tree, HEAD
  exactly F, and no staged, unstaged, or non-ignored untracked state. An absent, stale,
  ambiguous, structurally open,
  path-raced, or identity-mismatched receipt, any unaccounted prior obligation, or any
  unauthorized checkbox edit MUST block resume.
- **FR-073**: D106 remains binding in the completed production path.
  `d2b-resource-store` and `d2b-resource-store-redb` MUST NOT deserialize, import, compile,
  evaluate, or own `Role`, `RoleBinding`, `PolicySet`, or other RBAC policy DTOs. Policy
  interpretation stays in the Resource API and Zone policy owner. Store-owned validation
  MAY enforce policy-neutral envelope, schema, atomicity, revision, and structural
  invariants, and MAY only narrow an authorized mutation.
- **FR-074**: Wave 5 MUST reconcile the desktop-wrapper, companion, audio, USB, and
  security-key CLI reference promises with the exact emitted CLI and machine-readable
  behavior. A documented command or field MUST exist and pass its contract test. Candidate
  absence is a defect unless the same change follows the explicit parity or FR-042 retirement
  path with a named replacement, migration guidance, owner, restoring condition, release
  treatment, and contract coverage. A typed unavailable state is valid only when the frozen
  contract already defines it or that explicit path introduces it; candidate absence alone
  never authorizes rewriting the promise. Reference documentation MUST NOT invent an absent
  command, field, fallback, or production route. Pending-audit recovery MUST either conform to
  accepted `ADR-046-cli-and-operations` Version 1 or land T599's coordinated Version 2
  amendment with migration guidance, mandatory `zoneRef`/`schemaVersion`, DTO/schema and
  contract tests, release treatment, and closed remediation actions that contain no executable
  Zone/operation-ID argv or free-form command text.

#### Provider model

- **FR-010**: Every host capability in scope MUST be supplied by a Provider that the
  control plane installs, supervises, and holds accountable for the state it owns.
- **FR-011**: An operator MUST be able to obtain, inspect, and retire a capability purely by
  changing declared configuration, without editing framework source.
- **FR-012**: A Provider MUST NOT receive unmediated host privilege or a raw host path;
  every privileged host mutation MUST flow through the existing audited privileged path and
  be recorded.
- **FR-013**: A Provider failure MUST be attributed to that Provider in reported status and
  MUST NOT cascade to unrelated resources.
- **FR-014**: A Provider that owns durable state MUST re-adopt that state across its own
  restart and across host restart, and MUST fail closed rather than silently reinitialize
  when previously provisioned identity state is missing or altered.
- **FR-015**: Each Provider MUST be independently testable without requiring any other
  Provider to exist, compile, or be installed.

#### Operator surface and observability

- **FR-016**: Operators MUST be able to list and inspect resources, their owning Provider,
  their status, and the reason for any degraded or failed condition, from the operator
  command surface.
- **FR-017**: Reported failures MUST name a specific cause and at least one concrete operator
  action that can be taken next (a command, a configuration change, or a named artifact to
  inspect). A message that states only that something failed, or that offers only a generic
  retry, does not satisfy this requirement.
- **FR-018**: Telemetry and audit output MUST NOT contain secrets, credentials, command
  output, raw host paths, or personally identifying information, and MUST hold label
  cardinality within bounded, closed sets.
- **FR-019**: Reference documentation for a behavior MUST ship in the same increment as the
  behavior it describes, not deferred to a later increment.

#### Cutover and removal of the superseded plane

- **FR-020**: The system MUST provide a one-time, host-scoped cutover from the pre-ADR-046
  control plane, with a non-mutating preview that enumerates every affected artifact and its
  disposition before anything changes.
- **FR-021**: Destructive cutover steps MUST require explicit apply intent plus exact
  content-bound consent, and MUST be blockable by an operator-set hold.
- **FR-022**: The cutover MUST preserve designated irreplaceable state, including
  device-identity material, and MUST state a rollback boundary and support rollback up to
  that boundary.
- **FR-043**: The cutover MUST require the operator to explicitly attest that a host
  recovery point exists before executing any step past the rollback boundary, MUST refuse
  to proceed past that boundary without the attestation, and MUST record the attestation.
  The preview MUST state the rollback boundary and this obligation before the operator
  commits to anything. The W7 close path MUST require T580 to be complete and its passing,
  candidate-bound primary recovery-guard evidence to be current before panel request, seal,
  or merge, and MUST refuse each boundary when that evidence is absent, failed, stale,
  malformed, duplicated, or bound to any other candidate, commit, tree, preview, or host.

  A qualifying recovery point is an operator-owned, d2b-external full-host snapshot or
  restorable full-host backup. It MUST cover the boot and system configuration, the active
  NixOS generation, every artifact in the exact non-mutating cutover preview inventory, and
  all designated preserved identity state. It MUST target restoration to the same host, be
  retained read-only through its attestation expiration, have available restore instructions,
  and pass the external mechanism's non-mutating readback or integrity verification after
  capture. A d2b state export, a repository checkout, an unverified file copy, or a point
  covering only d2b paths does not qualify.

  The external canonical `d2b-recovery-point-attestation` version 1 record MUST contain
  exactly these fields: `artifactKind`, `schemaVersion`, `program`, `wave`, `candidateId`,
  `commitOid`, `treeOid`, `hostIdentitySha256`, `operatorSubjectSha256`, `previewSha256`,
  `recoveryPointKind`, `recoveryPointLocatorSha256`, `restoreInstructionsSha256`,
  `previewedAtUnix`, `capturedAtUnix`, `verifiedAtUnix`, `attestedAtUnix`,
  `retentionUntilUnix`, `expiresAtUnix`, `verificationMethod`, `verificationResult`,
  `qualification`, and `result`. `artifactKind` MUST equal
  `d2b-recovery-point-attestation`, `schemaVersion` MUST equal 1, `program` and `wave` MUST
  identify ADR046 and W7, and `recoveryPointKind` MUST be `full-host-snapshot` or
  `full-host-backup`. `verificationMethod` MUST be `snapshot-readback` or `backup-verify`;
  `verificationResult` and `result` MUST both be `passed`. `qualification` MUST contain only
  `bootAndSystemStateCovered`, `affectedArtifactInventoryCovered`,
  `preservedIdentityStateCovered`, `sameHostRestoreTarget`, and `readOnlyUntilExpiry`, all
  set to `true`. Canonical record bytes MUST be UTF-8 JSON serialized with the RFC 8785 JSON
  Canonicalization Scheme and no trailing bytes.

  `candidateId`, full `commitOid`, and full `treeOid` MUST equal the current frozen W7
  candidate (initially F7 and later only a distinct successor after durable failure).
  `previewSha256` MUST digest the exact canonical preview bytes used for that run.
  `hostIdentitySha256` MUST be the lowercase SHA-256 of the UTF-8 domain
  `d2b:recovery-host:v1`, one zero byte, and the lowercase contents of `/etc/machine-id` from
  the daily-driver host; the raw machine id MUST NOT enter the record.
  `operatorSubjectSha256` MUST use domain `d2b:recovery-operator:v1`, one zero byte, and the
  base-10 `SO_PEERCRED` uid. `recoveryPointLocatorSha256` MUST use domain
  `d2b:recovery-point-locator:v1`, one zero byte, and the opaque external locator.
  `restoreInstructionsSha256` MUST use domain `d2b:recovery-restore-instructions:v1`, one
  zero byte, and the exact external restore-instruction bytes. Each stores only the lowercase
  SHA-256, not a raw locator, recovery payload, restore text, username, or uid.

  Freshness is exact and bounded. Every timestamp field MUST decode directly from a JSON
  integer into one `RecoveryUnixSeconds` newtype whose closed range is 0 through
  253402300799. Negative, fractional, string, out-of-range, and non-canonical numeric forms
  MUST be refused. The validator MUST sample its current clock once per validation call into
  the same bounded type and require
  `previewedAtUnix <= capturedAtUnix <= verifiedAtUnix <= attestedAtUnix <= verifierNowUnix < expiresAtUnix`.
  It MUST compute `capturedAtUnix + 86,400` and `verifiedAtUnix + 86,400` with checked
  arithmetic that also remains within the newtype bound; overflow or an out-of-range result
  refuses the record. `expiresAtUnix` MUST equal the minimum of those two checked results and
  `retentionUntilUnix`. Import and every post-rollback boundary step, pre-panel dispatch,
  panel request, panel-attest, seal, merge-target registration, merge eligibility, and final
  merge check MUST invoke the same validator and occur strictly before `expiresAtUnix`.
  Candidate, commit, tree, preview, host or operator identity, restore-instruction binding,
  record bytes, future event time, clock order, or checked-expiration change invalidates the
  record.

  The validator MUST have one hermetic table-driven suite whose positive control is a valid
  canonical record and whose negative cases independently omit, duplicate, type-change, or
  alter every required top-level field, every qualification member, and every delivery
  binding. The matrix MUST include wrong `operatorSubjectSha256` and
  `restoreInstructionsSha256`, plus negative, fractional, future, out-of-range, and
  checked-add-overflow timestamp cases. Test listing MUST succeed, discover at least one
  matching non-ignored test, discover zero ignored matching tests, and execution MUST report
  no skip. Empty discovery is failure. A close stage MUST call this validator rather than
  copy a subset of its predicates.

  Expiration after a binding panel request durably fails that immutable candidate; evidence
  is not refreshed in place. Failure closure retains the request, panel, seal, and
  eligibility records and releases the candidate slot only after the closure is durable.
  The integrator then creates a distinct successor candidate, obtains a fresh canonical
  attestation bound to it, reruns complete candidate validation, and may issue that
  successor's single binding panel request. No predecessor attestation, panel, seal, or
  eligibility result transfers, even when commit and tree bytes are unchanged.

  T580 MUST import exactly one existing delivery `EvidenceRecord` with
  `validation = "recovery-point-attestation"` and `result = "passed"`, bound through its
  `candidate_id`, `content_id`, and `snapshot_sha256` to the current frozen W7 candidate.
  Its `output.sha256` and
  `output.bytes` MUST identify the exact canonical external attestation record, its
  `command` MUST name the verifier command without output, and its opaque `locator` MUST
  resolve the external record without embedding a raw host or recovery-point identifier.
  This feature specifies verification and refusal only. It does not implement, create,
  retain, or restore the external host snapshot or backup.
- **FR-023**: Each superseded path scheduled for removal MUST be removed only after its
  replacement is integrated and covered by tests, MUST pass an explicit removal proof, and
  MUST be removed in its own change separate from the change that introduced the
  replacement. This governs *how* a path is removed; FR-041 and FR-042 govern *whether* the
  capability it provided must survive.
- **FR-060**: The FR-023 removal-proof obligation binds the wave that **performs the
  removal**, not the wave that recorded the path in the migration map. A path whose recorded
  owning wave has already sealed and merged MUST NOT be treated as carrying an outstanding
  proof obligation against that closed wave, because a sealed wave cannot produce new
  evidence and its snapshot is immutable. Such a path acquires its proof obligation when a
  later wave removes it, and a path that no wave removes is not removed at all - so no proof
  is owed. This is a scoping rule, not a waiver: it changes *which* wave owes the proof, and
  never whether a removal needs one. A path being removed in the current wave always owes a
  proof under FR-023, regardless of which wave the map records as its owner.
- **FR-024**: The shipped release MUST NOT contain both the pre-ADR-046 control plane and
  its replacement.
- **FR-041**: Every operator-facing capability whose migration disposition promises a
  successor MUST be obtainable in 3.0 through that successor, and its parity MUST be
  verified before release. Removal mechanics are governed by FR-023.
- **FR-042**: A capability MAY be retired without a successor only if it appears in an
  explicit retirement list that states the justification, and it MUST be named in the
  consumer-facing release notes. A capability MUST NOT disappear silently or as an
  unremarked side effect of removing a superseded path.

#### Delivery and governance

- **FR-025**: Remaining work MUST be delivered wave by wave following the ADR-046 delivery
  contract. Each wave has two distinct panel gates: a unanimous `/d2b-panel-round plan`
  review bound to the exact implementation base and feature-artifact snapshot before any
  implementation or fix lane is dispatched, and a unanimous `/d2b-panel-round work` review
  of the integrated candidate after convergence and before advance. The work review does not
  substitute for the plan review. Waves seal and merge in strict order; a wave MUST NOT seal
  or merge before its predecessor has sealed at full unanimity and merged. This ordering
  constrains **exit** only. Pipelining may relax the predecessor-merge condition for
  implementation start under FR-048, but never relaxes the successor wave's own plan-review
  gate; see FR-057.
- **FR-048**: A wave's implementation MAY begin before its predecessor's panel completes,
  provided at least five of the predecessor's ten roster reviews have returned and the
  predecessor's integration tests pass on its converged tree.
- **FR-049**: A wave that started under FR-048 MUST NOT issue a panel request, produce a seal,
  or merge until its predecessor is sealed at 10/10 unanimity with zero recommendations and
  merged to the integration lineage. It MUST then rebase onto the updated integration lineage
  **before** its own panel runs, so the panel binds to a snapshot that already contains every
  predecessor finding.
- **FR-050**: Rework caused by a predecessor finding that invalidates in-flight work started
  under FR-048 MUST be absorbed by the wave that started early. Such rework MUST NOT be used
  as grounds to weaken, shorten, or partially accept the predecessor's panel.
- **FR-051**: Once a wave has completed eight panel rounds, a reviewer in round nine or later
  MAY classify a LOW or MEDIUM finding as deferred rather than blocking. CRITICAL and HIGH
  findings are never deferrable and MUST continue to block sign-off in every round.
- **FR-052**: A deferred finding MUST be moved out of `recommendations` and recorded in the
  deferred-findings register with its severity, subject area, wave, round, and reviewer role.
  The sign-off invariant is unchanged: `signoff` is `true` if and only if `recommendations`
  is empty. Deferring without recording, and re-ranking a CRITICAL or HIGH finding downward
  in order to defer it, are both process violations.
- **FR-053**: The program MUST maintain a deferred-findings register and a friction log as
  continuous planning inputs. Both MUST be reviewed at every wave close, MUST feed the
  terminal wave's triage, and MUST contain only classification metadata - never panel
  transcripts, validation command output, or attestation payloads.
- **FR-054**: After a wave's slices converge and its integration tests pass, and **before any
  panel lane is dispatched**, the wave MUST pass two read-only gates run in parallel: a
  verification gate against the specification artifacts and the constitution, and a
  code-review gate across its quality aspects. Every actionable content finding from either
  gate, at any severity, MUST be resolved before the panel is dispatched. A note that does
  not meet the finding bar is a nonblocking observation and MUST NOT enter the
  deferred-findings register. The round-nine LOW/MEDIUM deferral in FR-051 applies only to
  panel rounds, never to these pre-panel gates.
- **FR-055**: The code-review gate MUST be scoped to the wave's own diff against its actual
  base - the integration lineage, or the predecessor wave branch when stacked - and MUST NOT
  be scoped against the repository default branch, which does not share the integration
  lineage's history.
- **FR-026**: A wave MUST NOT be sealed unless every work item assigned to it is recorded as
  merged, its validation evidence is imported for the exact tree being sealed, and its
  binding ten-role panel has returned unanimous sign-off with zero outstanding
  recommendations against that exact snapshot.
- **FR-027**: Any content change to a wave's candidate tree MUST invalidate that wave's
  prior validation and panel evidence, except where a canonical proof establishes the
  content is byte-identical.
- **FR-028**: Independently ready, file-disjoint work MUST be launched in the same
  coordination cycle; a ready slice left unlaunched without a recorded blocker is a process
  failure to correct.
- **FR-059**: A destination file written by more than one parallel slice in the same wave is
  a contended file, and MUST NOT be edited concurrently by those slices. Before any of the
  claimant slices is dispatched, the integrator MUST land a shared-prep commit on the wave's
  root branch that establishes the contended symbol, module, or file once. Each claimant
  slice MUST then branch from that prep commit and write only disjoint regions of the
  resulting file, or the claimants MUST be internally ordered inside a single branch. Where
  the delivery contract assigns a contended path to the integrator alone - the workspace
  member list, the flake output list, the generated specification indexes, and the shared
  changelog block - a slice MUST NOT write that path at all, and MUST use the per-slice
  mechanism the contract names instead. Contention discovered during wave execution MUST be
  recorded in the same change that discovers it, and the wave MUST record its
  connected-component count, its launched-slice count, and any blocked slice with its exact
  blocker at wave entry and after every panel round. This binds immediately at W2, which has
  a single writer for `nixos-modules/assertions.nix`.
- **FR-029**: Every heavy validation lane MUST run through the single shared sole-use
  semaphore, with no second lock, retry loop, or per-crate guard.
- **FR-030**: A failed hard performance or footprint target MUST be resolved by changing the
  design, and MUST NOT be resolved by weakening durability, authorization, or audit, nor by
  adding a sleep, a timeout, or a test exclusion. The hard targets are:

  | Target | Threshold |
  | --- | --- |
  | Empty-store readiness | <= 500 ms |
  | p95 local point read and bounded list | <= 2 ms |
  | p95 crash-safe single-resource mutation | <= 10 ms |
  | p95 durable commit to controller handler start | <= 5 ms |
  | p95 ready process commit to launch-attempt start | <= 20 ms |
  | Whole-process resident memory, no baseline subtraction | <= 24,576 KiB |
  | Aggregate idle resident memory | <= 64 MiB |
  | Core system Provider / sandbox system Provider | <= 22 MiB / <= 12 MiB |
  | Per-Provider-crate hermetic suite, aggregate process-CPU p95 | <= 3 s |
  | Scale fixtures sustained while meeting the above | 10,000 resources; 100 live watches |

- **FR-031**: Generated artifacts, schemas, and specification indexes MUST remain in exact
  agreement with their sources, enforced by the existing fail-closed drift gates.
- **FR-032**: New test coverage MUST land at the lowest hermetic layer that can prove it,
  and MUST NOT introduce a new top-level shell gate.
- **FR-033**: Superseded test suites MUST be retired once their successor coverage passes,
  so that old and new suites do not run indefinitely.
- **FR-034**: Waves W0 and W1 MUST be recorded as delivered without the delivery contract's
  sealed wave records, by way of an explicit written waiver that names the missing artifacts
  (the ten panel receipts and the seal), states the evidence actually relied upon (all 14
  assigned work items recorded as merged, merged through reviewed pull requests), and is
  produced before W2 entry.
- **FR-035**: Sealed delivery MUST begin at W2. Every wave from W2 through W8 MUST produce a
  complete seal satisfying FR-026, and the FR-034 waiver MUST NOT be extended, reused, or
  cited as precedent for any wave from W2 onward.
- **FR-058**: The FR-034 waiver's scope MUST be read narrowly, and covers **only the absence
  of the W0 and W1 seal artifacts** - the ten panel receipts and the seal record for those
  two waves. It MUST NOT be read as waiving any work item's own completion obligation. In
  particular, the nine `ADR046-delivery-001` through `ADR046-delivery-009` work items are
  recorded as `Planned` and are assigned by the implementation graph to wave **W7**, not to
  W0 or W1. Their `Planned` state is therefore outside the waiver entirely: each MUST reach
  `Merged` under W7's own seal, evaluated against W7's snapshot under FR-026, and the waiver
  MUST NOT be cited as evidence for any of them. More generally, a work item owned by a wave
  later than W1 is never covered by the waiver regardless of its current implementation
  state, and no work item is recorded as complete on the strength of the waiver alone.
- **FR-036**: W2 entry MUST NOT be blocked by the absence of W0 and W1 seals. Under FR-048 a
  wave's implementation may also begin while its predecessor's items are not yet `Merged`;
  the predecessor-merged condition is enforced at the successor's **exit boundary** - panel
  request, seal, and merge eligibility (FR-049), not at its implementation start.
- **FR-057**: The program MUST distinguish **entry evidence** from **exit evidence**, and
  MUST NOT treat a requirement for one as a requirement for the other. Entry evidence is what
  a wave needs in order to **start implementing**: Gate 0 has passed, its destination paths
  carry no open contention flag, its stack is proposed against the exact named parent commit,
  the heavy-gate semaphore is available, and the fast hermetic suite passes on its entry tree.
  Exit evidence is what a wave needs in order to be **delivered** - sealed and merged: every
  assigned work item recorded as merged, validation evidence imported for the exact snapshot,
  and unanimous ten-role panel sign-off with zero outstanding recommendations against that
  snapshot. A missing or absent predecessor seal blocks the successor's **exit** and never its
  **entry**. FR-025's prohibition on partial-wave advance therefore means a wave is never
  *delivered* early and its evidence is never *accepted* early; it does not mean implementation
  must wait. This resolves the apparent conflict between FR-025 and FR-036, and matches the
  delivery contract's pipelined-start conditions restated in FR-048 through FR-050.
- **FR-044**: Every wave's work MUST land through pull requests opened against the
  integration lineage and merged only after that wave's gates pass: validation evidence
  imported for the exact snapshot, unanimous panel sign-off, a seal, and an eligible
  merge check. Work MUST NOT reach the integration lineage by direct push or by a local
  merge that bypasses those gates.
- **FR-045**: No intermediate release artifact MUST be published during the program. There
  is exactly one release, d2b 3.0, cut after the final wave satisfies the release gate.
  Wave merges are integration events, not releases, and MUST NOT be tagged or published as
  consumable versions. This does not forbid publishing the replacement *contracts* that
  companions adapt against; FR-061 defines the contract/artifact boundary and is the
  resolution of the apparent conflict with FR-039.
- **FR-046**: Where the specification set's prose and the generated implementation graph or
  work-item manifest disagree on wave assignment, destination paths, or work-item identity,
  the **generated manifests are authoritative**. Any such drift MUST be recorded and raised
  as a separate specification amendment, and MUST NOT be silently corrected inside an
  implementation wave, because amending a member spec re-opens that spec's validation and
  panel evidence.
- **FR-047**: Implementation MUST conform to every resolved decision in the ADR-046 decision
  register. A change that contradicts a decision is a specification amendment, not an
  implementation choice, and MUST follow FR-046's amendment path.
- **FR-056**: Gate 0, the manifest closure gate, MUST be re-evaluated - never waived -
  whenever any member of the ADR-046 specification manifest changes content after being
  marked Accepted. Any amendment to a member specification MUST re-open that specification's
  validation evidence and its panel evidence, and MUST re-trigger Gate 0 across the whole
  manifest, including the manifest digests, the work-item bijection, and the required human
  review gates. Gate 0 MUST pass again before any wave that depends on the amended
  specification may seal, and a wave holding evidence gathered before the amendment MUST
  regather that evidence rather than carry it forward. This is a standing program obligation
  for the full duration of W2 through W8, not a one-time precondition satisfied at program
  start.

#### Program scope

- **FR-037**: The program MUST deliver waves W2 through W8 inclusive, including the
  destructive host cutover and the removal of the superseded control plane.
- **FR-038**: The program MUST satisfy all six conditions of the release gate as evaluated
  against the final wave's candidate snapshot, and MUST then tag and publish d2b 3.0 from
  the `v3` integration lineage.
- **FR-039**: d2b 3.0 MUST NOT be released until a compatible version of every desktop
  companion that consumes d2b's public operator contracts exists and has been verified
  against the release candidate. The program MUST identify that companion set, publish the
  replacement contracts they depend on early enough for them to adapt, and treat an
  unadapted companion as a release blocker rather than as acceptable post-release breakage.
  FR-064 defines which candidates are members of that set; FR-065 defines what verification
  passes.
- **FR-040**: Companion compatibility MUST be verified by exercising each companion against
  the release candidate on a live host, not by inspection of its source or version number
  alone.
- **FR-061**: The FR-039 release blocker and the FR-045 no-intermediate-artifact rule are
  both retained in full, and the tension between them is resolved by **publishing contracts
  without publishing artifacts**. The distinction is binding, not editorial. A **contract**
  is committed reference text, a committed schema, or a committed typed definition, reachable
  at a public git ref, that a companion maintainer can read and implement against; publishing
  one is not a release. An **artifact** is anything a consumer's build could select or fetch
  as a version - a tag, a GitHub release, a binary archive, a substituter output, or a flake
  output pinned to a version; publishing one is a release and remains forbidden. The program
  MUST publish contracts early and MUST NOT publish artifacts, and the three stages MUST run
  in this order, each refusing rather than degrading:

  | Stage | Wave | Refusal if the stage is not met |
  | --- | --- | --- |
  | Publish the companion inventory and every replacement contract it names | W5 | The W5 exit refuses while any "surface consumed" cell in the inventory does not resolve to a committed reference document, schema, or typed definition at a public ref |
  | Companion maintainers adapt against the published contracts | W5 through W8, external to this program | No refusal here; this program does not control the schedule of a sibling repository, which is exactly why FR-062 records it as an unvalidated assumption rather than a plan step |
  | Verify each companion by exercising it against the release candidate on a live host | W8 | The release gate refuses while any inventory row lacks a live-host verification record naming the exact candidate, the companion revision, the surfaces exercised, and the result |

  Three things MUST NOT be accepted as verification evidence in the third stage: source
  inspection, a matching version number, and the publication of the contracts themselves.
  Contract publication is adaptation input, never compatibility evidence, and a reviewer who
  treats a published contract as a discharged verification has skipped the stage that FR-040
  exists to require.

  If adaptation stalls anyway, exactly two outcomes are lawful: **hold the release**, or
  **amend FR-045** through the specification-amendment path. Amending FR-045 is a
  specification change with its own evidence, never an integrator judgment call and never an
  unannounced preview. Publishing any artifact without that amendment is a violation of
  FR-045, not a pragmatic exception to it.
- **FR-062**: The assumption underlying FR-061 - that a companion maintainer can adapt from
  published contracts alone, with no artifact to build or test against - is **recorded as
  unvalidated**. This program cannot validate it: doing so requires evidence from repositories
  it does not own, and no such evidence has been gathered. The assumption MUST therefore be
  carried as a named risk with a stated mitigation and a stated detection point, and MUST NOT
  be restated as a fact anywhere in the program's artifacts.

  - **Mitigation**: the published contracts are the actionable interface shape, not a
    summary. Where a surface has a generated schema or a typed definition, the contract MUST
    point at that generated source rather than paraphrase it, so a maintainer implements
    against the same bytes the implementation validates against.
  - **Detection point**: the first live-host verification in W8. That is the first moment the
    assumption is tested, and it is late. A verification failure there is evidence the
    assumption was wrong, and the response is FR-061's two lawful outcomes, not a relaxation
    of FR-040.
  - **Escalation**: if the assumption is found wrong for any companion, that finding MUST be
    recorded against this requirement rather than absorbed into a wave's fix round, because
    it changes a program-level premise and not a wave's implementation.
- **FR-063**: Each companion surface named in the published inventory MUST be classified at
  W8 into exactly one of three outcomes, and the classification - not the impression a
  reviewer forms - decides whether the release proceeds.

  **First, the distinction the classification rests on.** A companion that reads a published
  capability key, finds the capability false, and declines to offer the action is **conforming
  to the contract**, not degrading. Capability discovery is the sanctioned way an operator's
  desktop shrinks: `runtime.operationCapabilities` is a committed manifest field, and
  `docs/reference/zone-cli-contract.md` already binds the shell client to check
  `runtime.operationCapabilities.guest.shell` before offering a shell action. Treating that as
  a defect would block the release on a companion doing exactly what d2b told it to do, and
  would make the capability surface pointless. **Degradation** is the different case: the
  surface is available and the companion cannot use it.

  | Outcome | Condition | Effect on the release |
  | --- | --- | --- |
  | **Conformant** | Every surface named in the row either works, or is unavailable through a published capability key or a typed refusal state the contract already names, and the companion refuses that action with an actionable message and takes no fallback | Ships |
  | **Blocked** | Anything else: absent, crashes, hangs, silently returns a wrong result, falls back to another transport or privilege path or a legacy shape, refuses without an actionable message, requires an undocumented workaround, or **cannot be classified** | Holds the release (FR-039) |
  | **Retired** | The operator has converted a Blocked surface into an explicit capability retirement under FR-042 **before the tag** | Ships, with the retirement named in the consumer-facing release notes |

  **A degraded required companion blocks.** SC-024 exists so that an operator's desktop is not
  degraded by adopting 3.0, and this requirement does not carve an exception into it. There is
  no tolerance band, no "mostly working" outcome, and no per-surface partial credit: a row with
  one Blocked surface is Blocked.

  **Fail closed on classification.** Any W8 outcome not positively classified as Conformant or
  Retired is Blocked. An exercise that was not run, was inconclusive, or produced a result the
  verifier could not place is Blocked, because an unclassifiable outcome and a broken one are
  indistinguishable from the release gate's position.

  **Refusal must be actionable, per FR-017.** A conformant refusal MUST name the capability key
  or refusal state that is false, and MUST name at least one concrete operator action: an option
  to set, a command to run, or an artifact to inspect. A bare "not supported", a generic retry
  prompt, a message naming only the companion, and a silently disabled or greyed control with no
  explanation are each **not** actionable, and a row whose refusal is unactionable is Blocked
  rather than Conformant.

  **Retirement is the only lawful ship-with-less path, and it is not a reclassification.** A
  Retired outcome requires an entry on the FR-042 retirement list with a stated justification,
  a named owner, the condition that would restore the surface, and a line in the consumer-facing
  release notes. It MUST be decided before the tag; a failed exercise MUST NOT be relabelled as
  a retirement after the fact. It is unavailable where FR-041 independently applies - if the
  capability's migration disposition promised a successor, that successor must be obtainable and
  no retirement may substitute for it. The published inventory row MUST NOT read as verified for
  a retired surface.

  **No staged deprecation applies.** FR-045 leaves exactly one release, and this repository
  deliberately retired its staged warning, fail-loud, and removal calendar at the clean break.
  A retirement is therefore an enumerated, release-note-named fact, never the first step of a
  multi-release timeline.
- **FR-064**: Membership in the release-blocking companion set MUST be decided by a two-limb
  test. A candidate is a member if and only if **both** limbs hold, and the decision MUST be
  recorded rather than argued.

  **Limb 1 - discovery.** The candidate appears in at least one of these sources, which are a
  closed list:

  1. the flake inputs of the validation host's own configuration - d2b targets a single trusted
     host with one operator, so the set that adopting 3.0 can break is what that host runs;
  2. the currently published inventory in `docs/reference/companion-contracts.md`, so the set
     can never shrink silently; or
  3. any repository that a d2b reference document, example, template, or how-to names as
     consuming a d2b surface.

  Prose in `README.md` or `AGENTS.md` MAY raise a candidate but MUST NOT settle membership,
  because it is measurably unreliable: `AGENTS.md` names no companion at all, and `README.md`
  names them once, in a sentence about colour output, listing three of the five published
  members under non-canonical short names alongside two upstream projects that are not members,
  and omitting two members entirely.

  **Limb 2 - consumed public surface.** The candidate consumes at least one surface from this
  closed list of public operator surfaces:

  - the public daemon socket wire (`docs/reference/daemon-api.md`);
  - the `d2b` CLI contract, including `--json` output and exit codes
    (`docs/reference/cli-contract.md`), and its v3 replacement
    (`docs/reference/zone-cli-contract.md`);
  - the public `vms.json` manifest (`docs/reference/manifest-schema.md` and its schema);
  - public presentation artifacts `/etc/d2b/ui-colors.json` and `/etc/d2b/ui-colors.css`
    (`docs/reference/ui-colors.md` and its schema);
  - the clipboard picker protocol over the inherited `socketpair()` file descriptor
    (`docs/reference/clipboard-picker-protocol.md`);
  - public launcher metadata served to authorized clients through the public daemon API
    (`realm-workloads-launcher-v2.json`, per `docs/reference/manifest-bundle.md`); and
  - the flake's public outputs: `nixosModules`, `packages.<system>`, `templates`, `overlays`.

  **Reading a private artifact is not membership; it is a defect.**
  `docs/reference/manifest-bundle.md` fixes the public/private boundary, and every private
  bundle artifact installs `root:d2bd` `0640`. A candidate found reading one MUST be reported
  as a defect and MUST NOT be admitted to the inventory on that basis, because admitting it
  would record an unauthorised read as a supported contract.

  **Evidence each row carries.** The repository, the exact revision pinned on the validation
  host as a commit rather than a tag or version string, the maintainer of record, the discovery
  source that raised it, and the specific surfaces from limb 2 that it consumes. A row without
  a pinned revision is not a row, because "which version blocks" would be undecidable.

  **Additions and removals.** An addition requires only both limbs. A **removal requires a
  negative determination**: recorded evidence that the candidate consumes no surface on the
  limb-2 list, at a named revision, on a named date. Removal by assertion, by absence from
  prose, or by an unrecorded judgement is not permitted.

  **Uncertain candidates fail closed into the set.** A candidate that satisfies limb 1 but whose
  limb-2 consumption cannot be determined is a **member** and blocks the release until a
  negative determination is recorded. The asymmetry is deliberate: wrongly including costs one
  determination, and wrongly excluding ships a broken desktop.
- **FR-065**: "A compatible version verified against the release candidate" (FR-039, SC-024)
  passes if and only if **all** of the following hold. Any one failing is a fail, and there is
  no aggregate or majority reading:

  1. the exercise ran on the daily-driver **live host**, not in a VM, a container, or a CI
     runner;
  2. it ran against the **exact release-candidate snapshot** that will be tagged, named by
     commit;
  3. the companion was at a **pinned revision**, named by commit;
  4. **every** surface named in that companion's inventory row was exercised, not a sample;
  5. every one of those surfaces classified **Conformant or Retired** under FR-063;
  6. **zero** surfaces classified Blocked, including zero that could not be classified; and
  7. the evidence was recorded in FR-063's shape.

  **None of these is a pass**: source inspection; a matching version number or tag; a
  successful documentation check; the publication of the replacement contracts; a successful
  build; a green CI run in the companion's own repository; an exercise against any d2b build
  other than the candidate; an exercise on a host that is not the live validation host; and a
  partial exercise of the row.

  **A moved candidate voids its verifications.** If the release-candidate snapshot changes for
  any reason, every companion verification recorded against the previous snapshot is void and
  MUST be re-run against the new one. This mirrors the rule that any content change invalidates
  prior panel sign-off, and it is what makes "verified against the release candidate"
  measurable rather than aspirational: without it, "the candidate" is whichever build was
  convenient at the time.

### Key Entities

- **Zone**: The unit of isolation, policy, routing, resource ownership, state, and audit.
  Owns exactly one resource store, one resource service, one authoritative self resource,
  and one core controller. Every resource belongs to exactly one Zone.
- **Resource**: A durably recorded, typed object with an operator-declared desired state and
  a controller-owned observed status, addressed by a Zone-scoped reference and carrying a
  revision for conflict detection.
- **Provider**: The installable, supervised unit that implements one or more resource types
  and is accountable for the state it owns. Providers are the extension point that replaces
  framework-internal capability switches.
- **Controller**: The component that continuously drives a resource from observed toward
  declared state, subject to ownership, dependency ordering, and fair admission.
- **Resource store**: The Zone-local durable record of resources and their revisions,
  supporting point reads, bounded listing, conflict-detecting commits, and change
  notification.
- **Component session**: The authenticated, single-owner association between a component and
  a Zone through which all resource access is admitted.
- **Cutover**: The one-time, host-scoped, previewable, consent-gated, partially reversible
  procedure that replaces the pre-ADR-046 control plane with a live Zone runtime, assigning
  every existing artifact a disposition of adopt, preserve, or destroy.
- **Wave**: The delivery unit. Each wave has entry criteria, immutable candidate snapshots,
  candidate-bound validation, at most one binding panel per candidate, one unanimously
  accepted candidate and seal, and exit criteria.
- **Work item**: The smallest tracked unit of implementation, bound to one owning
  specification with exact destination paths and required validation, and holding a state
  that must reach merged before its wave can seal.

## Delegation boundary

This specification states program-level outcomes, gates, and cross-cutting obligations. It
deliberately does **not** restate the per-ResourceType, per-Provider, and per-controller
contracts that the 55-member ADR-046 set already defines normatively. Restating them here
would create a second source of truth that no drift gate checks.

The boundary is explicit so that a delegated obligation is never mistaken for a missing one:

| Concern | Owner | Status here |
| --- | --- | --- |
| Field-level shape, validation, and state machine of each of the 19 ResourceTypes | The six owning resource specs | **Delegated.** The types are `Zone`, `ZoneLink`, `Provider`, `Role`, `RoleBinding`, `Quota`, `EmergencyPolicy`, `ResourceExport`, `ResourceImport`, `Host`, `Guest`, `Process`, `EphemeralProcess`, `User`, `Endpoint`, `Volume`, `Network`, `Device`, `Credential`. FR-001 through FR-009 apply to all of them uniformly. |
| Per-Provider behavior for all 27 Provider dossiers | The 27 dossier specs | **Delegated.** FR-010 through FR-015 apply to every Provider uniformly. |
| Controller algorithms, admission, dependency ordering | `core-controllers`, `resource-reconciliation` | **Delegated.** |
| Cutover phase mechanics, disposition matrix, the three reset scopes (Full Zone, Provider, Guest) | `reset-and-cutover` | **Delegated**, except the operator-facing guarantees stated in FR-020 through FR-024 and FR-043. |
| Wire formats, schemas, generated artifacts | Owning specs plus generated `docs/reference/schemas/v3/` | **Delegated**, enforced by FR-031. |
| Threat model, telemetry retention, streamline scope, remaining feasibility spikes | `security-and-threat-model`, `telemetry-audit-and-support`, `streamline`, `feasibility-and-spikes` | **Delegated** to their owning specs and to the wave that implements them, per FR-019. |
| The 129 frozen decisions | `decision-register` | **Binding.** See FR-047. |

Delegation is not omission. Every delegated obligation is enumerated in
[spec-coverage.md](./spec-coverage.md). Each manifest-backed task carries the authoritative
`workItemId` pointer; dispatch resolves that id to one complete 15-field manifest object and
carries the object verbatim rather than copying selected fields into the task row.

## Success Criteria *(mandatory)*

### Measurable Outcomes

#### Operator-visible capability

- **SC-001**: An operator can take a host with no prior Zone state, declare a Zone and its
  resources, activate, and reach a fully ready state with no manual intervention beyond the
  activation itself.
- **SC-002**: A newly declared resource becomes live within 2 seconds of activation for a
  single-Zone declaration of 10 to 20 resources (for example one guest, a volume, a network,
  and a device), and an operator observes progress rather than an opaque wait.
- **SC-003**: Every operator-facing capability whose migration disposition promises a
  successor is obtainable after the program, expressed as declared resources rather than
  framework-internal switches. Zero capabilities disappear silently: any deliberate
  retirement appears in the explicit retirement list and in the release notes.
- **SC-004**: 100 percent of resource failure conditions surfaced to an operator name a
  specific cause and an actionable next step.
- **SC-005**: An operator can determine which component owns any given capability, and its
  current state, in a single inspection command.

#### Durability, isolation, and correctness

- **SC-006**: No committed state is lost across process restart, host restart, or power loss
  in the restart and power-loss test scenarios.
- **SC-007**: No external effect is observed without a corresponding durable commit, under
  abort, conflict, restart, and crash injection.
- **SC-008**: Cross-Zone resource access is denied by default in 100 percent of tested
  attempts, with each denial recorded.
- **SC-009**: No component obtains access by naming its own identity; every admission is
  based on proven identity in 100 percent of tested attempts.
- **SC-010**: No secret, credential, command output, raw host path, or personally
  identifying value appears in telemetry, audit, logs, or error output across the full
  redaction test matrix.

#### Wave 5 production completion

- **SC-030**: On the exact Wave 5 candidate, every successful Resource API request and watch
  is traceable to one registrar-admitted ComponentSession and its authoritative subject, and
  100 percent of attempted cross-Zone or self-named-subject accesses are denied and audited.
  Unix admission obtains a live pidfd directly with `SO_PEERPIDFD` and verifies credentials,
  generation, cgroup, and liveness against that fd; restart obtains a new fd from the newly
  accepted socket, and unavailable support, numeric-only identity, PID reuse, dead fd,
  mismatch, and ambiguity are denied. API-surface and compile-fail seals expose no public
  registrar issuer, peer credential/evidence accessor, or bootstrap-identity mint path.
- **SC-031**: Crash injection at every boundary from generation commit through effect
  completion leaves zero lost effects and zero lost cleanup intents after restart. Every
  stale, zero, or wrong-UID cleanup completion is denied without changing durable state.
- **SC-032**: For every privileged mutation in the audit matrix, an immutable authoritative
  journal row commits transactionally with the mutation before any success-shaped effect.
  For every ordinary successful mutation, append-only segment file and directory sync, export,
  and its separate completion state are durable before success is returned. At every
  post-commit export crash
  boundary, the mutation instead returns `CommittedPendingAudit` through the additive
  protobuf status field as `ResourceStatus.phase = ResourcePhase::Degraded`,
  `ResourceStatus.outcome.code = StatusCode("committed-pending-audit")`,
  `ResourceStatus.update.state = UpdateState::Blocked`, and
  `ResourceStatus.update.operation_id = Some(original_operation_id)`. Its bounded, redacted
  condition, outcome, and update fields expose only safe same-ID retry/status remediation.
  It leaves the Zone unpublished and degraded and never reports rollback. Same-ID retries
  with an exact replay-binding match apply the mutation zero additional times and converge on
  one final result; cross-subject, cross-Zone, altered-request/target/verb/revision/
  idempotency, and restart mismatches are denied and audited; a different-ID retry obeys
  revision/conflict rules. Restart replay produces zero missing records and zero duplicate
  logical exports by fixed operation digest plus mutation ordinal. Raw operation,
  correlation, subject, Zone, resource, and trace canaries occur zero times in audit/export/
  error/log/metric/span/Debug output; constructors accept only typed fixed digests and
  oversize records refuse. Configured segment limits and post-export journal retention hold,
  early journal prune refuses, and any prune or file/directory-sync failure produces typed
  degraded health. The typed `InspectOperation` path returns the same durable pending/final
  state across restart and never observes a wrong binding.
- **SC-033**: Removing the `Provider/system-core` registration or either required
  `Zone.status.handlers[]` record in turn prevents publication of only the affected Zone.
  Acceptance also rejects duplicate records, a missing record, a wrong `name`, and an
  attempt to use the distinct `provider-lifecycle` record in place of either
  `system-core-host` or `system-core-user`. The two required records occur exactly once,
  carry `phase` and `lastReconciledAt`, and are backed by active, initialized, current live
  handlers; a boolean substitute fails the test. In the multi-Zone startup and shutdown
  matrix, every unrelated Zone is visited and remains operable, and every affected Zone
  reports a specific actionable refusal.
- **SC-034**: Clean pre-validator A/P0 analysis and plan-panel receipts authorize only T603's
  two validator source paths. Dedicated validator commit V has sole parent A and no other
  repository changes; resume base B equals V and feature snapshot P equals P0. Analysis over
  A..B plus the full feature artifacts and the plan panel are rerun and bound to B/P before
  T603 creates any authorization. A finding or later validator change invalidates B and both
  post-validator receipts. T603's immutable external authorization contains exactly the
  closed task-ID set T073-T218 with one `satisfied|open` row each, opaque project sentinel
  `7f6d0beab0ce4c13a89f6865d5ac42e2`, Git-root-relative feature path, B/tree, P, and
  validator-derived post-edit snapshot Q. Any open row
  leaves T603 unchecked and changes no checkbox. Only 146 satisfied rows, post-validator
  analysis with no unresolved HIGH or CRITICAL finding, and the post-validator unanimous
  plan panel at B/P authorize the sole `/d2b-spec-edit` progress batch. Dedicated checkbox
  commit C has exact parent B and exact P-to-Q diff; prepare/apply/finalize tests converge
  after a crash from B/P, B/Q, or C/Q. T589 refuses until the finalized editor receipt
  exists, HEAD is clean C, and T073-T218 plus T603 are checked. T605 remains future work and
  does not alter the 146 receipt rows or 147 checkbox transitions. T219 refuses unless C is
  an ancestor of final candidate F frozen after T220 convergence; the exact eight closed
  FR-072 evidence identifiers occur once each at their assigned T600/T601 owner and lane; all
  records name F and F's tree; HEAD equals F with no staged, unstaged, or non-ignored
  untracked state; `operator-nix-activation-cleanup` alone carries T604's all-positive
  public-switch activation/effect/cleanup result; `system-core-handler-contract` alone
  carries the coordinated T605 contract, T595 emitter, and T599 consumer result; production
  RSS is at or below 24,576 KiB with no baseline subtraction; owner fan-in is singular;
  current removal proofs pass; and checked reference behavior matches emitted CLI and wire
  output. T219 then runs F's exactly one binding panel and seal. F stays immutable and cannot
  receive another request. Nonunanimity fails F and routes scoped fixes through a distinct,
  fully revalidated successor; the successful merge preserves that candidate's tree.

#### Scale and footprint

- **SC-011**: The resource plane sustains a 10,000-resource working set and 100 concurrent
  watchers while continuing to meet its readiness, latency, and footprint targets.
- **SC-012**: The Zone runtime whole-process resident memory stays at or below 24,576 KiB with
  no baseline subtraction, met by design change rather than by relaxing durability,
  authorization, or audit (FR-030). Corrected disposable-proof and production-fixture
  measurements passed at their recorded tips; the completed production publication path has
  no current measurement until T601 measures final candidate F.
- **SC-013**: A Zone with an empty store becomes ready to serve within half a second.

#### Migration and release

- **SC-014**: An operator on the pre-ADR-046 control plane can preview a cutover and see
  100 percent of affected artifacts with an explicit disposition, with zero modification
  during preview.
- **SC-015**: Designated preserved state, including device identity material, survives
  cutover intact in 100 percent of cutover test scenarios, and a missing or altered
  identity artifact fails closed rather than reinitializing.
- **SC-016**: A cutover interrupted before its stated rollback boundary can be rolled back
  to a working prior control plane in 100 percent of tested interruption points.
- **SC-025**: In 100 percent of tested attempts, the cutover refuses to execute any step
  past its rollback boundary until the operator has supplied exactly one FR-043 version 1
  record for a qualified recovery point. The record's candidate, commit, tree, preview,
  daily-driver host digest, qualification fields, chronological order, 86,400-second
  freshness, and expiration all match, and every such attestation is recorded through the
  bound delivery `EvidenceRecord`. The candidate-bound primary recovery guard passes and
  T580 is complete before W7 panel request, seal, or merge; every missing, extra, duplicate,
  failed, malformed, wrong-host, wrong-candidate, wrong-commit, wrong-tree, wrong-preview,
  expired, or externally unresolvable record rejects each boundary.
- **SC-017**: Zero superseded control-plane units, command surfaces, or configuration
  namespaces scheduled for removal remain in the released tree, verified by their removal
  proofs.
- **SC-018**: The released version's notes are consumer-readable and contain zero internal
  wave, phase, follow-up, or finding markers.

#### Program completion

- **SC-019**: Every work item in the specification set is recorded as merged. The initial
  545-item census was 14 merged and 531 planned; the current manifest census at the receipt
  HEAD is 68 merged and 477 planned. Release also includes every item recorded for the
  terminal wave at W7 close. The count is read from the manifest at release time and is not
  fixed at 545.
- **SC-020**: Every wave from W2 through W8 carries a seal bound to its exact snapshot, with
  unanimous ten-role panel sign-off and zero outstanding recommendations. W0 and W1 carry a
  written delivered-without-seal waiver instead, and no wave from W2 onward relies on that
  waiver.
- **SC-021**: Zero foundation surfaces remain deliberately unwired from production at
  release: the capabilities delivered in W0 and W1 are reachable through the operator
  surface rather than only through tests.
- **SC-022**: Manual hardware, live-host, and cloud validation tiers have each been executed
  at least once against the final candidate with recorded external evidence, on the
  operator's daily-driver host carrying the real device set.
- **SC-023**: d2b 3.0 is tagged and published from the `v3` lineage with all six release-gate
  conditions satisfied against the final candidate snapshot.
- **SC-024**: 100 percent of identified desktop companions that consume d2b's public
  operator contracts have a compatible version verified against the release candidate on a
  live host before 3.0 is tagged, so an operator's desktop is not degraded by adopting 3.0.
  The identified set is fixed by FR-064's two-limb membership test; "verified" means exercised
  and classified under FR-063 and passing every condition of FR-065. A Blocked surface,
  including one that could not be classified, holds the release.
- **SC-026**: All seven remaining waves reach the integration lineage through a pull request
  whose gates passed first, with zero waves landing by direct push or by a gate-bypassing
  local merge, and zero intermediate versions published before 3.0.
- **SC-027**: Every wave seals at 10/10 unanimity and merges strictly after its predecessor
  did, in 100 percent of waves. Zero waves issue a panel request while their predecessor is
  unsealed, and zero waves panel against a snapshot that predates their post-merge rebase.
- **SC-028**: Zero CRITICAL or HIGH findings are deferred in any wave. Every LOW or MEDIUM
  finding deferred by a round-nine-or-later panel appears in the deferred-findings register
  with a disposition, and zero deferred findings reach the release still marked open without
  an explicit withdrawal or schedule. Pre-panel observations never enter this register.
- **SC-029**: 100 percent of waves pass the pre-panel verification and code-review gates,
  scoped to the wave diff, with zero CRITICAL verification findings outstanding, before
  any panel lane is dispatched.

## Assumptions

- The ADR-046 specification set is Accepted and closed. This feature implements the
  specifications as written; it does not renegotiate them. Where implementation reveals a
  specification defect, the correction is made in the specification set through its own
  amendment path, which re-opens the affected validation evidence. The obligation this
  creates is a requirement, not merely an assumption; see FR-056.
- The ADR-046 delivery contract in `ADR-046-validation-and-delivery` governs the binding
  work-panel, seal, and merge-eligibility surfaces; it does not supersede the repository's
  per-wave phase gate. Every wave retains one unanimous plan review before implementation
  dispatch and one unanimous work review after convergence. For already-dispatched W2-W4,
  the feature artifacts currently prove no contemporaneous plan-review receipt: historical
  compliance remains unproven, a current remedial plan review may guard only future dispatch,
  and the later work panel cannot repair or substitute for the missed historical gate.
  Wave 5's T603 plan review is the mandatory gate for resumed implementation after a valid
  T072 historical or current remedial entry disposition. W6-W8 must pass their prospective
  plan gates before their first implementation lane.
- The project constitution applies in full, in particular the audited-privilege boundary,
  the isolation-over-convenience rule, contract versioning, test-layer discipline, and the
  ban on internal process markers in shipped artifacts.
- Delivery proceeds in the specified wave order W2 through W8. Sealing and merging are
  strictly ordered and no partial-wave advance is permitted, but implementation start is
  pipelined; the entry-evidence versus exit-evidence distinction is stated in FR-057. The
  program terminates at the release of d2b 3.0, not at feature completeness: the release
  gate is evaluated against the final wave's snapshot, because gating earlier would release
  a candidate that a later wave still modifies.
- Waves W0 and W1 are accepted as delivered under a written waiver rather than being
  retroactively panelled and sealed. Their binding panel would otherwise have to run against
  a historical snapshot that no longer exists in a single canonical form. Per FR-057, every
  prior work item being recorded as merged is an **exit** condition, not an entry condition:
  it is tested at W2's exit boundary - panel request, seal, and merge eligibility - not at
  W2's implementation start. That exit
  condition is already satisfied - all 14 W0 and W1 work items are independently verified as
  `Merged` - so the waiver removes no check that W2's close would otherwise have to make.
  The waiver is a one-time, documented exception, not a precedent, and its scope is bounded
  by FR-058.
- The pre-ADR-046 control plane remains functional for operators throughout W2 through W6.
  It is replaced only by the cutover in W7 and removed under the release gate, so an
  operator's working host is not expected to be broken mid-program.
- Live-host, hardware, and cutover validation run on the operator's daily-driver host,
  because that is where the real GPU, TPM, and security-key devices are. This is a
  deliberate risk acceptance: the daily driver is the machine being put at risk, so the
  recovery-point attestation and rollback boundary (FR-043) are the primary safety net
  rather than a formality, and a recovery point must exist before each destructive run.
- The production storage engine and watch primitive now exist in committed code, but the
  approved Wave 5 work is not complete until FR-066 through FR-074 make them reachable
  through the authenticated production boundary. The historical measurements remain
  evidence for their named snapshots only; FR-072 requires new exact-candidate evidence.
- All 27 specified Provider dossiers are in scope, since each is an Accepted member of the
  specification set with assigned work items that must reach merged for its wave to seal.
- The integration lineage is `v3` rather than `main`. How work reaches it is a requirement,
  not an assumption; see FR-044 and FR-045.
- Desktop companion maintainers can adapt to the v3 surfaces from published contracts alone,
  without any artifact to build or test against. This is an **unvalidated** assumption held
  about repositories this program does not own, and it is carried as a named risk with a
  mitigation, a detection point, and an escalation path; see FR-062. It is stated here as an
  assumption and nowhere in this program's artifacts as a fact.
- Delivery state, panel transcripts, and attestation payloads remain outside the repository
  and are never committed.
- The target remains a single trusted host with one human operator. Multi-tenant isolation,
  a general-purpose container or VM manager, and support for non-NixOS hosts stay out of
  scope.
- Effort and calendar duration are deliberately not estimated here. The initial program
  scope was 531 work items; 477 remain `Planned` in the current manifest. They span nine
  specifications' worth of resource types plus 27 Provider dossiers, and sequencing is
  governed by the dependency graph rather than by a date.

## Out of Scope

- New architectural decisions beyond ADR-046 and its specification set.
- Feature work unrelated to ADR-046 that happens to touch the same tree.
- Backward compatibility with the pre-ADR-046 configuration namespace or wire protocol.
  The specification set mandates a destructive cutover with no compatibility layer and no
  in-place protocol migration.
- Multi-tenant trust boundaries, non-NixOS host support, and an X11 fallback.
- Implementing changes inside the sibling desktop-companion repositories. Their code is
  authored and released by their own maintainers. This program owns identifying the
  companion set, publishing the replacement contracts they need, and verifying them against
  the release candidate; a companion that has not adapted blocks the 3.0 release (FR-039,
  FR-040), and a companion that adapted only partly is classified and blocks on any Blocked
  surface (FR-063).
