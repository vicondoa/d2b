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
and the storage feasibility proof. Fourteen of 545 enumerated work items are `Merged`; 531
remain `Planned` across waves W2 through W7. The terminal wave W8 has no work items yet by
design: its contents are the delivery friction accumulated across the program and are
recorded at W7 close, so the program's final total exceeds 545.

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
   attested that a host recovery point exists, **Then** no further step executes and the
   reason is stated plainly.

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
  commits to anything.
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
  contract. Waves seal and merge in strict order; a wave MUST NOT seal or merge before its
  predecessor has sealed at full unanimity and merged. This ordering constrains **exit**
  only. It does not constrain when a wave may begin implementing; see FR-057.
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
  code-review gate across its quality aspects. Every CRITICAL finding from the verification
  gate, including every constitution conflict, MUST be resolved before the panel is
  dispatched.
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
- **Wave**: The delivery unit. Each wave has entry criteria, an immutable candidate
  snapshot, validation evidence, exactly one binding panel, a seal, and exit criteria.
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
[spec-coverage.md](./spec-coverage.md) and carried verbatim into tasks.

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

#### Scale and footprint

- **SC-011**: The resource plane sustains a 10,000-resource working set and 100 concurrent
  watchers while continuing to meet its readiness, latency, and footprint targets.
- **SC-012**: The Zone runtime whole-process resident memory stays at or below 24,576 KiB with
  no baseline subtraction - the target that currently measures 25,216 KiB - met by design
  change rather than by relaxing durability, authorization, or audit (FR-030).
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
  past its rollback boundary until the operator has attested that a host recovery point
  exists, and every such attestation is recorded.
- **SC-017**: Zero superseded control-plane units, command surfaces, or configuration
  namespaces scheduled for removal remain in the released tree, verified by their removal
  proofs.
- **SC-018**: The released version's notes are consumer-readable and contain zero internal
  wave, phase, follow-up, or finding markers.

#### Program completion

- **SC-019**: Every work item in the specification set is recorded as merged. That is the 545
  enumerated today (14 merged, 531 planned across W2 through W7) plus every item recorded for
  the terminal wave at W7 close. The count is read from the manifest at release time and is
  not fixed at 545.
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
- **SC-028**: Zero CRITICAL or HIGH findings are deferred in any wave. Every deferred
  LOW or MEDIUM finding appears in the deferred-findings register with a disposition,
  and zero deferred findings reach the release still marked open without an explicit
  withdrawal or schedule.
- **SC-029**: 100 percent of waves pass the pre-panel verification and code-review gates,
  scoped to the wave diff, with zero CRITICAL verification findings outstanding, before
  any panel lane is dispatched.

## Assumptions

- The ADR-046 specification set is Accepted and closed. This feature implements the
  specifications as written; it does not renegotiate them. Where implementation reveals a
  specification defect, the correction is made in the specification set through its own
  amendment path, which re-opens the affected validation evidence. The obligation this
  creates is a requirement, not merely an assumption; see FR-056.
- The ADR-046 delivery contract in `ADR-046-validation-and-delivery` governs how this work
  is delivered, and supersedes the repository's generic per-round phase gate for ADR-046
  work. This feature specification does not define a parallel process; the wave lifecycle,
  the once-per-wave binding ten-role panel, seal, and merge-eligibility rules apply
  unchanged.
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
- The deferred production storage engine and its watch consumer are delivered in W5 with the
  named design corrections that address the failed footprint measurement. Re-measuring
  against the corrected design is part of that wave's evidence.
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
- Effort and calendar duration are deliberately not estimated here. The remaining 531 work
  items span nine specifications' worth of resource types plus 27 Provider dossiers, and
  sequencing is governed by the dependency graph rather than by a date.

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
