# Feature Specification: Complete the ADR-046 Provider Control Plane (d2b 3.0)

**Feature Branch**: `001-adr046-d2b3-completion` (spec directory; implementation lands on per-wave stacks cut from `v3`)

**Created**: 2026-07-29

**Status**: Specification reconciled - Wave 6 entry approval pending

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
delivery friction accumulated across the program and are triaged and recorded only after W7
is sealed, merged, and cleaned up, so the program's final total exceeds 545.

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
sealed. The legacy-named `waiver-w0-w1.md` records that historical failure and its weaker
evidence only. It does not waive Constitution Principle VI, ratify the missing gates, or
authorize W2 entry.

**Accepted constitutional disposition**: Constitution 3.1.0 now provides a generic
historical-process disposition. This feature's exact delivery validator/tooling contract
instantiates it once and only for ADR-046 history through merged Wave 5 commit
`177235ed37188b3be87525e7f016fb43401574c5`, the missing W0/W1 panel receipts and seals, the
unproven contemporaneous W2-W5 plan panels, and the exact retained Wave 5 state named in
FR-036. This is a closed historical governance deviation, not a finding that those gates
passed. The retained candidate, request, evidence, zero-attestation state, and absent seal
remain immutable. No Wave 5 recovery, replacement candidate, second request, retroactive
attestation, reconstructed seal, or actionable Wave 5 close is authorized.

The disposition permits only prospective Wave 6 entry. Before any Wave 6 implementation
dispatch, T221 must prove the fetched exact `origin/v3` entry base, the accepted first-parent
integration commit carrying the exact generic Constitution 3.1.0 bytes after the Wave 5
merge, the
exact retained state and evidence inventory, and the focused delivery guard. It must then
obtain the ordinary unanimous selected-roster plan result against that exact base and current
feature snapshot with zero recommendations. Every prospective Wave 6 validation, binding
panel, protected PR, seal, and merge gate remains unchanged.

All later future-tense Wave 5 completion requirements and success criteria in this artifact
are retained as historical design evidence. They are not executable recovery instructions,
do not block T221, and are not claims that unchecked tasks or validation passed. The exact
retained bytes, rather than a reconstructed ideal candidate, are the only Wave 5 input to
prospective continuation.

### Approved Wave 5 production-completion amendment (2026-08-06)

<!-- RETIRED-READONLY-BEGIN: historical Wave 5 amendment -->

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

The operator has approved the missing production-plane wiring in FR-066 through FR-074 as
Wave 5 work except for the settled Network implementation and operator-positive boundaries.
T336-T355 retain the Network production path and four-case matrix in W6 under T221, while
prospective activation/cleanup acceptance follows those rows. Wave 5 must instead remove
every current-facing sole Network-opt-in path and freeze the double-opt-in migration plus
that W6 ownership. A readiness bit, direct `WatchService` call, fake endpoint, disabled audit
callback, or test-only subject may not substitute for any real path. This is an explicit
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
wire serialization and still omits those variants. Retired T605 is historical only.
Prospective W6 T423 owns the coordinated correction: add
`ZoneHandlerName::SystemCoreHost` and `ZoneHandlerName::SystemCoreUser`, serialize them only as
`system-core-host` and `system-core-user`, retain the underscore spellings only as internal
telemetry labels, bump both governing normative specification versions, and update every
paired compiler-derived API snapshot, Rust serialization and duplicate/underscore-rejection
test, lowest-layer contract/policy guard, and reference status surface. The historical plan
assigned emitter and consumer work to T595/T599; those assignments are retired. Prospective
the then-proposed owner held the enum, emitter, consumer, generated-artifact, and focused
evidence correction before later acceptance.

The C1 correction itself adds no field or operation and changes no desired-state ResourceType
schema. Therefore it requires no `apiVersion`, JSON `schemaVersion`, `manifestVersion`,
`bundleVersion`, or C1-specific wire-field version bump.
The Zone desired-spec artifact
`docs/reference/schemas/v3/core.d2bus.org_Zone.schema.json` remains unchanged and T423 must
prove generator output is byte-identical rather than hand-editing it. C1 is resolved in these
feature artifacts but is not implemented. The former T603/T589 accounting and review sequence
is read-only historical planning evidence and is not reconstructed.

<!-- RETIRED-READONLY-END -->

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

**Wave checkpoint**: Wave 5 is only a partial US1 production-plane checkpoint and does not
claim the three-resource operator activation positive. The exact acceptance set is
`Volume/acceptance-state`, `Network/acceptance-net`, and `Device/acceptance-tpm` in Zone
`acceptance`; no other resource may substitute for one of those three. The authoritative
prospective acceptance row is the active feature-local T604 coordination/completion task; it
exercises that
set as W6 acceptance after T221 and merged authoritative T336-T355, consuming their
double-opt-in production Network path and four-case matrix. Full US1 completion occurs only
after that result and the Wave 6 `Provider/runtime-cloud-hypervisor` family supply positive
runtime-effect acceptance for the declared Guest. Missing, skipped, status-only,
fake-boundary, or refusal evidence leaves US1 incomplete.

#### Exact Wave 6 operator acceptance fixture

The authoritative fixture MAY contain the Host, User, Guest, system Provider, and artifact-catalog
prerequisites that these three resources require, but those support objects are not acceptance
resources and their effects cannot substitute for an effect below. The three selected Provider
resources and their artifact-catalog entries are exact:

| Provider resource | Artifact entry | Exact `spec.config` |
| --- | --- | --- |
| `Provider/volume-local` | `volume-local-provider`, type `provider`, candidate package `d2b-provider-volume-local` | `controllerExecutionRef = "Host/host-system"`; `sourcePolicies = [{ id = "state-root"; class = "local-path"; volumeKinds = ["state"]; }]`; the test-owned backing-root binding remains private compiler/broker data and no raw path enters the Provider or Volume resource |
| `Provider/network-local` | `provider-network-local`, type `provider`, candidate package `d2b-provider-network-local` | `controllerExecutionRef = "Host/host-system"` |
| `Provider/device-tpm` | `d2b-provider-device-tpm`, type `provider`, candidate package `d2b-provider-device-tpm` | `controllerExecutionRef = "Host/host-system"` and `logLevel = 20` |

`d2b.artifacts.net-vm-base` is additionally present with type `nixos-system` and the
candidate generic net-VM system. The exact acceptance resources are:

| Resource | Exact authored configuration | Required real effect and readiness |
| --- | --- | --- |
| `Volume/acceptance-state` | `providerRef = "Provider/volume-local"`; `source.executionRef = "Host/host-system"`; `source.settings = { kind = "local-path"; sourcePolicyId = "state-root"; }`; `kind = "state"`; one root directory layout entry owned and grouped by `User/d2bd`, mode `0700`, `createPolicy = "create-if-never-provisioned"`, `repairPolicy = "exact-owner"`, `cleanupPolicy = "never"`, `sensitivity = "private"`, and `noFollow = true`; one `controller` view over `""` with `read`, `write`, `create`, `delete`, and `traverse`; no attachments and no quota | The fixed Core adapter and broker resolve the opaque source policy, provision or adopt the root plus identity marker, and read back the declared owner, mode, type, and no-follow posture. The universal resource phase is `Ready`, `status.update.state` is `Current`, and every declared layout entry is `Ready`; a status-only assignment or a fake effect port is ineligible. |
| `Network/acceptance-net` | `providerRef = "Provider/network-local"`; `lanCidr = "10.20.0.0/24"`; `uplinkCidr = "192.0.2.0/30"`; `mtu = null`; `mssClamp = false`; `isolation.allowEastWest = false`; `routing.hostBlocklist = ["10.0.0.0/8", "169.254.0.0/16", "172.16.0.0/12", "192.168.0.0/16"]`; `dhcp = { domain = null; ignoreClientNames = true; }`; `dns = { forwarders = []; cacheSize = 1000; }`; `externalAttachment = null`; `mdns = { enable = false; reflector = true; dnsmasqLocal = false; dnsmasqLocalPort = 53530; publishWorkstation = false; }`; `netVmNameOverride = null`; `netVmSystemArtifactId = "net-vm-base"`; and `attachments = []` | The production Network controller creates or adopts both derived bridges, reapplies and reads back IPv6 suppression, installs only this Network's firewall ownership projection, and converges its config Volume, owned net-VM, and agent dependencies. The universal resource phase is `Ready`; `FabricReady`, `FirewallReady`, `ConfigVolumeReady`, `NetVmReady`, and `DhcpReady` are true; both bridge status phases are `Ready`. Network-owned Guest dependencies establish only Network readiness and MUST NOT be reported as independent Guest acceptance. |
| `Device/acceptance-tpm` | `metadata.ownerRef = "Guest/acceptance-vm"`; `providerRef = "Provider/device-tpm"`; `deviceClass = "emulated"`; `arbitration = "exclusive"`; `maxConcurrentClaims = 1`; `inventory.selector = {}`; `provider = { schemaId = "device-tpm.d2bus.org/Device/spec"; schemaVersion = "1.0.0"; settings.logLevel = 20; }` | The production Device controller creates or adopts its controller-managed TPM state Volume, verifies its tamper marker, completes the mandatory pre-start flush, starts the broker-supervised long-lived swtpm Process, and publishes the typed TPM Endpoint. The universal resource phase and Provider phase are `Ready`, `status.update.state` is `Current`, and Device status reports `present = true` and `health = healthy`; a manually assigned phase, refusal, or fake worker is ineligible. |

This single denied-east-west acceptance case does not establish Host/Network double opt-in.
The untouched external `ADR-046-resources-network` and committed absence of a production
adapter make the historical sole-opt-in result nonconforming and non-authorizing. T070 and
T071 cannot complete by ratifying it. The required contract is
`effectiveEastWest = Network.spec.isolation.allowEastWest && d2b.site.allowUnsafeEastWest`;
both inputs default false. The former freeze predicate is historical only. T221 requires
the accepted versioned correction and migration to remove every current-facing sole
Network-opt-in path and retain T336-T355 plus all four Network/Host combinations as W6 work.
It requires the migration and ownership on
the fetched Wave 6 base, and T480 revalidates them before every prospective close boundary.
Prospective acceptance ownership resolves only from authoritative member specs and generated
manifests after T336-T355 merge. A feature-local status,
declaration-only fixture, fake effect port, historical W4 record, stale sole Network opt-in,
or reassignment of T336-T355 outside W6 cannot unblock any boundary.

The removal generation deletes only `Device/acceptance-tpm`. Its
`device-tpm.d2bus.org/state-preserved` finalizer MUST set the owned swtpm Process to stopped,
wait for its terminal phase, delete that Process, delete any non-terminal flush
`EphemeralProcess`, preserve the controller-created TPM state Volume with the same resource
identity and marker, release its Volume references, clear the finalizer, and allow the Device
row to disappear. The typed TPM Endpoint MUST no longer resolve after deletion.
`Volume/acceptance-state`, `Network/acceptance-net`, their live effects, and unrelated
resources MUST remain `Ready`, retain their resource identities, and show no recreation.

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
- **FR-066**: Code canon lacks the accepted-socket peer-pidfd operation. The authoritative
  member spec and generated manifest MUST publish the production Resource API only through
  an authenticated, single-owner ComponentSession admitted by the authoritative Zone
  registrar and routed by the ZoneBus. The registrar MUST derive the subject from verified peer evidence in its
  private state and propagate that authoritative subject through every Resource API
  operation. Unix peer evidence MUST obtain the process descriptor directly from the accepted
  socket with `SO_PEERPIDFD`; opening a pidfd later from `SO_PEERCRED.pid` is forbidden.
  Acquisition MUST use a typed broker operation that receives only the accepted socket over
  `SCM_RIGHTS` and returns only an `OwnedFd` pidfd with `FD_CLOEXEC` over `SCM_RIGHTS`; no
  numeric PID, raw descriptor integer, or credential tuple is serializable. A safe dependency
  API MAY be used only if it satisfies the exact no-panic and fd-ownership contract.
  Otherwise the sole raw `getsockopt(SO_PEERPIDFD)` wrapper MUST live in the approved
  `packages/d2b-priv-broker/src/sys.rs` FFI quarantine with narrow item-level unsafe
  allowances and an immediate `SAFETY:` justification on every unsafe block. No
  repository-authored unsafe is permitted outside that quarantine. The wrapper MUST validate exact
  `optlen`, take ownership of every nonnegative returned fd before checking syscall outcome
  or later invariants, and close it on every failure. The `nix` 0.31.3 `PeerPidfd`
  `MaybeUninit`/assert wrapper, a new repository-authored FFI crate, and a
  `d2b-session-unix` syscall fallback are ineligible.
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
  absent from audit output, telemetry, logs, metrics, spans, redacted `Debug`, and unrelated
  error context. The sole output exception is a direct Version 2 operator CLI/JSON status or
  recovery response: bounded `zoneRef` and `operationId` values that the same operator
  supplied or received MAY appear there as required recovery coordinates. Those fields MUST
  remain confined to that response and MUST NOT become telemetry labels, span attributes,
  exported audit identities, or unrelated error context. No other raw identity uses this
  exception. Raw propagated
  trace context MUST remain private; an authoritative row or export may retain only its typed
  domain-separated fixed digest. Audit constructors MUST accept typed fixed digests rather
  than raw identifiers, and encoded records MUST reject bytes beyond the fixed limit. Replay
  after restart MUST be idempotent by fixed operation digest and mutation ordinal.
  Metrics and OTEL resource attributes MUST carry no Zone, resource, operation, correlation,
  or trace identity, raw or digested. Logs and spans MAY carry a typed fixed digest only when
  their accepted contract requires correlation, and MUST use a distinct domain-separated
  digest type for each identity class. A digest type MUST NOT be relabelled as another class.
  Before T182-T205 may dispatch, the accepted
  `ADR-046-telemetry-audit-and-support` specification and generated work-item manifests MUST
  be versioned and amended to remove every raw identity field and attribute and to assign the
  corresponding redaction/cardinality tests. This uses the existing audit and telemetry
  owners; it creates no secrets service or runtime boundary.
  `audit.retentionDays`, default 30 and range 1 through 3650, MUST govern segments and
  export-completed journal rows;
  `audit.maxRecordsPerSegment`, default 65536 and range 1 through 1000000, and
  `audit.maxSegmentBytes`, default 67108864 and range 1048576 through 1073741824, MUST be
  enforced at startup and rotation. Prune, limit, file-sync, or
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
  `ResourceStatus.update.operation_id` is `Some(original_operation_id)`. That response field
  and its bounded Version 2 CLI projection are recovery coordinates under the sole direct
  operator-response exception above, not telemetry or audit identity. Existing bounded,
  redacted condition, outcome, and update fields carry only the semantic status and
  instructions to retry with the same ID or inspect status. They MUST NOT expose a subject,
  mutation payload, raw sink error, or a claim that the commit was undone. The affected Zone
  MUST remain unpublished and degraded until export completes. Before same-ID status or
  resumption, the implementation MUST match a persisted replay-binding digest over the
  registrar-derived subject, Zone, canonical semantic request, target, verb, exact expected
  revision, operation ID, and idempotency data. Mutation identity is exactly
  `(Zone, operation_id)`: inspection MUST name the Zone, the same opaque operation ID MAY be
  used concurrently in different Zones, and no host-global operation-ID reservation or index
  may be introduced. Cross-subject or altered-request, target, verb, revision, idempotency,
  or restart mismatch within that Zone MUST be denied and audited without observation or
  reapplication. An exact same-Zone retry returns the same pending state while export is
  incomplete and its one stored final result after recovery. A different operation ID follows
  ordinary expected-revision and conflict semantics.
  The 16-byte ID MUST use UUIDv7 byte layout and remain externally opaque, rendered as
  lowercase 32-hex without separators. Its embedded issuance time plus the fixed 30-day
  operation recovery retention defines checked `expiresAt`; operation state MAY be pruned only at that
  boundary. The existing durable per-Zone retention clock MUST never move backwards. A
  future, malformed, expired, or clock-discontinuous ID MUST fail closed as
  `operation-expired` or invalid before mutation or inspection and MUST NOT become a new
  mutation after its prior state is pruned.
  Every mutation response, including
  `DeleteResponse` and batch ordinals, MUST represent the composite with the additive bounded
  protobuf `PendingAuditStatus`; ordinary success omits it. This changes the ResourceService
  schema fingerprint but not Resource JSON `apiVersion` or `schemaVersion`, and
  `ResourceUpdateStatus` does not acquire a phase or status-code member. An unavailable or
  disabled audit owner, missing authoritative row, incomplete export, dropped record, or
  unbound record MUST fail closed.
  Installed-host generation handoff remains broker-only and fail-closed. The target-closure
  entrypoint runs unprivileged and may submit only an opaque intent after local lifecycle
  authorization. Before ownership transfer, the installed source broker under the existing
  `d2b-priv-broker.socket` and `d2b-priv-broker.service` owns the durable coordinator and
  every privileged mutation; after one durable transfer, the target broker owns continuation.
  No new unit, daemon-owned rollback path, caller-selected privileged executable, serialized
  authority, or root/provenance shortcut is permitted. Target, apply-object, and live apply
  peer identities are pinned and revalidated before every mutation; exit, exec, PID reuse,
  mismatch, ambiguity, or recovery-owner uncertainty refuses before mutation.

  All source-generation floor schemas, encodings, digests, receipts, capability transitions,
  fixtures, poison registries, and handoff transition matrices are owned solely by accepted
  Version 2 `ADR-046-validation-and-delivery` through `VD2-SC002-SOURCE-FLOOR`,
  `VD2-SC002-REGISTRIES`, and `VD2-SC002-TRACEABILITY`. Retired feature-local ownership has no
  prospective effect. Current ownership resolves only from authoritative member specs and
  generated manifests. Feature-local field lists, counts, or transition copies are not authority.
- **FR-071**: Persisted store, policy, active-configuration, and controller identities MUST
  reopen after their mutable revisions advance. Immutable store and Zone identity MAY be
  checked at open, but mutable revisions MUST be recovered from durable state rather than
  pinned to bootstrap constants. Startup and shutdown MUST visit every declared Zone:
  failure in one Zone MUST leave that Zone unpublished and visibly degraded while unrelated
  Zones continue, and a close failure MUST NOT silently drop later stores or their owners.
  Recovery and cleanup MUST retain ADR 0034's adopt-before-cleanup rule.
- **FR-072**: The retained Wave 5 history records exact-candidate evidence for all of the
  following: authenticated cross-Zone denial and same-Zone watch delivery through production
  boundaries; restart crash windows for effect replay/adoption and cleanup stale, zero, and
  UID-mismatch refusals; durable audit drain and restart replay; whole-process RSS and
  single-owner fan-in at 10,000 resources and 100 watches; current removal proofs; and
  reference documentation compared with emitted behavior. The former enum/list evidence was
  planned but is absent from code canon; current ownership resolves only from authoritative
  member specs and generated manifests. The exact three-resource operator activation positive
  is not Wave 5 evidence: its authoritative prospective row follows T221 and merged T336-T355, and
  T479/T480 bind it to F6 together with the
  `Provider/runtime-cloud-hypervisor` Guest result. Actionable refusals remain separate
  negative cases and cannot satisfy either positive story.
  The historical Wave 5 plan assigned convergence and the following closed
  `EvidenceRecord.validation` set:
  `production-session-watch`, `effect-replay-cleanup`, `audit-drain-replay`,
  `system-core-handler-contract`, `resource-plane-rss-owner-fanin`, `wave5-removal-proofs`, and
  `cli-reference-conformance`. Retired rows remain fenced historical records and authorize no current validator
  run, convergence, evidence import, host-continuity execution, or close action.

  Wave 5's retained `panel-request.json` consumed its sole binding request with zero
  attestations and no seal. The exact delivery validator/tooling contract, under the generic
  Constitution 3.1.0 disposition, accepts that state only as closed history through merged
  Wave 5 commit
  `177235ed37188b3be87525e7f016fb43401574c5`. The feature-owned record preserves that disposition and
  performs no binding or close action. No retained-state fixture, current panel, replacement
  candidate, second request, retroactive attestation, reconstructed seal, or recovery
  transition may mutate or complete Wave 5. Historical planning and accounting text
  remains evidence only and is not a prerequisite for the one-time Wave 6 predecessor
  disposition. T221 requires the accepted
  double-opt-in Network contract and settled T336-T355 W6 ownership on its exact fetched
  base; T479/T480 own the prospective implementation evidence and close checks.

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
  accepted `ADR-046-cli-and-operations` Version 1. The former Version 2 amendment plan is
  historical and authorizes no current implementation.
- **FR-075**: The pre-ADR-046 operator lifecycle MUST remain functional on every exact
  prospective candidate that closes W6. Historical W2-W5 results remain immutable evidence
  and are not rerun or reconstructed. Before the W6 work-panel request,
  the candidate MUST enumerate and successfully build
  `vmChecks.x86_64-linux.daemon-restart-vm-survival` through the existing heavy-gated
  `make test-host-integration` target with no skip. The case MUST use the public `d2b vm`
  surface to start the configured VM, observe the explicit `Ready` state and guest
  reachability, restart `d2bd.service`, prove the same runner PID/start-time identity was
  adopted through a newly acquired pidfd and remained reachable, stop the VM, and observe the
  explicit `Stopped` state. Enumeration MUST query the complete loaded `d2b*` and
  `microvm*` namespace with `systemctl list-units --all`, extract every returned unit name,
  exclude exactly the canonical `d2b.slice`, sort the remainder, and require exactly these
  three lifecycle units: `d2bd.service`, `d2b-priv-broker.socket`, and
  `d2b-priv-broker.service`. No additional unit may remain after that sole exclusion.
  Code canon makes the raw matched set four entries on a conforming host: committed
  `d2b.slice` plus those three service/socket units. Therefore "exactly three" in this
  requirement always means the sorted post-exclusion comparison, never the raw
  `systemctl` count asserted by the stale AGENTS.md exit-criterion prose. This feature records
  that external drift but does not edit AGENTS.md.
  A nonzero `systemctl list-units --all` result MUST refuse before filtering;
  a later pipeline stage may not turn failed enumeration into an empty or successful census.
  No other slice, target, service, socket, timer, path, or template is excluded. Querying only
  those three names is not enumeration and cannot detect an unexpected lifecycle unit. The
  negative matrix MUST inject an unexpected loaded
  `d2b-unexpected.slice` and, separately, an unexpected loaded
  `d2b-unexpected.service`; both survive the sole `d2b.slice` exclusion and MUST fail exact
  equality. PID reuse, pidfd/start-identity mismatch, and multiple-plausible-runner
  cases MUST quarantine without adoption, cleanup, or signal. Prospective command execution
  is limited to T479 for W6. Historical T028/T035/T070 and retained Wave 5 evidence are read
  only and MUST NOT rerun the target. Current source ownership of the host VM case and its
  discovery/build recipe resolves only from authoritative objects; every current or later close
  reuses that
  candidate-bound predicate. Passing evidence MUST name the enumerated and successfully built
  attr, command success, and no `SKIP` result. Missing, empty, skipped, stale,
  wrong-candidate, status-only, private-hook, incomplete unit enumeration, missing
  Ready/Stopped, or non-fresh-pidfd evidence blocks that wave's close. This requirement
  expires only at W7's
  explicit cutover; a fail-closed continuity gate is not permission to weaken any W2-W6
  security or delivery gate.

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
  provisional candidate. A distinct provisional identity is allowed only before the wave's
  binding request, after a nonbinding phase finding or pre-request evidence expiry.
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

  Expiration after the wave's binding panel request durably fails that immutable candidate;
  evidence is not refreshed in place. Failure closure retains the request, panel, seal, and
  eligibility records. Because the binding panel is exactly once per wave, no successor may
  receive another binding request. The integrator MUST stop and escalate the external
  disposition rather than transfer approval, refresh evidence, or waive the failed close.

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
  contract. Each wave has a nonbinding phase-panel surface and exactly one binding delivery
  panel. `/d2b-panel-round plan` reviews the exact implementation base and feature-artifact
  snapshot before implementation dispatch and handles iterative finding-fix rounds over
  provisional integrated candidates before the final candidate is selected. Every content
  change invalidates that phase result and requires a delta/full-context rerun. Only after
  phase-panel convergence may the final immutable candidate receive the wave's single binding
  `/d2b-panel-round work` request. A binding finding leaves the wave unsealed and cannot be
  followed by a second binding request for any candidate. The binding review does not
  substitute for the entry plan review. Waves seal and merge in strict order; a wave MUST NOT seal
  or merge before its predecessor has sealed at full unanimity and merged. This ordering
  constrains **exit** only. Pipelining may relax the predecessor-merge condition for
  implementation start under FR-048, but never relaxes the successor wave's own plan-review
  gate; see FR-057. The exact ADR-046 delivery validator/tooling contract instantiates the
  generic Constitution 3.1.0 disposition as exactly one historical predecessor exception:
  for Wave 6 only, merged Wave 5 commit
  `177235ed37188b3be87525e7f016fb43401574c5` replaces a Wave 5 seal as predecessor evidence.
  It does not create that seal and does not weaken Wave 6's own prospective gates.
- **FR-048**: A wave's implementation MAY begin before its predecessor's panel completes,
  provided at least five of the predecessor's selected-roster reviews have returned and the
  predecessor's integration tests pass on its converged tree. This permission does not apply
  to W8 triage or entry: T557 and T558 wait until W7 is sealed, merged, and cleaned up.
- **FR-049**: A wave that started under FR-048 MUST NOT issue a panel request, produce a seal,
  or merge until its predecessor is sealed at N/N unanimity for its selected roster with zero recommendations and
  merged to the integration lineage. It MUST then rebase onto the updated integration lineage
  **before** its own panel runs, so the panel binds to a snapshot that already contains every
  predecessor finding.
- **FR-050**: Rework caused by a predecessor finding that invalidates in-flight work started
  under FR-048 MUST be absorbed by the wave that started early. Such rework MUST NOT be used
  as grounds to weaken, shorten, or partially accept the predecessor's panel.
- **FR-051**: Every current lifecycle MUST follow Constitution 3.0
  Discover-Fix-Verify: one comprehensive discovery from every selected seat, one stable
  shared ledger, batched implementation responses and self-verification, and scoped
  verification against the ledger and full candidate.
- **FR-052**: Verification MUST keep pre-existing late MINOR and NIT observations as
  nonblocking history and MUST keep admitted late BLOCKER and MAJOR findings blocking.
  There is no round-count threshold and no later transition from blocking to nonblocking.
- **FR-053**: The program MUST maintain the friction log as a continuous planning input,
  review it at every wave close, and feed it to the terminal wave's triage only after W7 is
  sealed, merged, and cleaned up so the triage includes actual close and cleanup friction. The legacy
  deferred-findings register is historical compatibility data and MUST NOT receive current
  lifecycle findings. Neither register may contain panel transcripts, validation command
  output, or attestation payloads.
- **FR-054**: After a wave's slices converge and its integration tests pass, and **before any
  panel lane is dispatched**, the wave MUST pass two read-only gates run in parallel: a
  verification gate against the specification artifacts and the constitution, and a
  code-review gate across its quality aspects. Every actionable content finding from either
  gate, at any severity, MUST be resolved before the panel is dispatched. A note that does
  not meet the finding bar is a nonblocking observation and MUST NOT enter the
  legacy deferred-findings register. No round-count threshold makes an actionable content
  finding nonblocking at these pre-panel gates.
- **FR-055**: The code-review gate MUST be scoped to the wave's own diff against its actual
  base - the integration lineage, or the predecessor wave branch when stacked - and MUST NOT
  be scoped against the repository default branch, which does not share the integration
  lineage's history.
- **FR-026**: A wave MUST NOT be sealed unless every work item assigned to it is recorded as
  merged, its validation evidence is imported for the exact tree being sealed, and its
  binding selected-roster panel has returned unanimous sign-off with zero outstanding
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

  SC-002's 2,000 ms activation-to-live interval is the outer operator outcome over this
  component system, not an additional component row and not a sum of the p95 rows. Its
  monotonic start/stop events and included ingestion/effect/projection stages are defined in
  SC-002. The FR-030 no-weakening rule applies to it. A run must satisfy that end-to-end
  ceiling and every applicable table row independently.

- **FR-031**: Generated artifacts, schemas, and specification indexes MUST remain in exact
  agreement with their sources, enforced by the existing fail-closed drift gates.
- **FR-032**: New test coverage MUST land at the lowest hermetic layer that can prove it,
  and MUST NOT introduce a new top-level shell gate.
- **FR-033**: Superseded test suites MUST be retired once their successor coverage passes,
  so that old and new suites do not run indefinitely.
- **FR-034**: Waves W0 and W1 MUST have a historical deviation record that names the missing
  artifacts (the ten panel receipts and the seal for each wave) and the weaker evidence
  actually available (all 14 assigned work items recorded as merged through reviewed pull
  requests). The record is evidence only. It MUST state that it does not waive Constitution
  Principle VI, cure the missing gates, authorize W2 entry, or permit any later phase to
  dispatch, resume, close, or advance.
- **FR-035**: The feature-owned exact delivery validator/tooling contract applies the generic
  Constitution 3.1.0 disposition to close the exact historical deviations through merged
  Wave 5 without reconstructing missing evidence or a Wave 5 seal. Every prospective wave from W6
  through W8 MUST produce its own complete seal satisfying FR-026. The FR-034 historical
  record and the one-time Wave 5 predecessor disposition MUST NOT be extended, reused, or
  cited as authorization or precedent for another program or wave.
- **FR-058**: The FR-034 historical record's scope MUST be read narrowly, and covers **only the absence
  of the W0 and W1 seal artifacts** - the ten panel receipts and the seal record for those
  two waves. It MUST NOT be read as waiving any work item's own completion obligation. In
  particular, the nine `ADR046-delivery-001` through `ADR046-delivery-009` work items are
  recorded as `Planned` and are assigned by the implementation graph to wave **W7**, not to
  W0 or W1. Their `Planned` state is therefore outside the historical record entirely: each MUST reach
  `Merged` under W7's own seal, evaluated against W7's snapshot under FR-026, and the record
  MUST NOT be cited as evidence for any of them. More generally, a work item owned by a wave
  later than W1 is never covered by the record regardless of its current implementation
  state, and no work item is recorded as complete on the strength of the record alone.
- **FR-036**: The generic Constitution 3.1.0 historical-process disposition contains no
  ADR-046 identifier, wave, candidate, request, or hash. This feature and its exact delivery
  validator/tooling contract bind only the ADR-046 history through merged Wave 5 commit
  `177235ed37188b3be87525e7f016fb43401574c5`. The retained Wave 5 state MUST remain
  candidate `d20267eec23f90b9cd6931e4bd322b66e259533849c8170617fbd002381493a4`,
  snapshot identity `7a04d9b86df6c8b8704b4bd79ddc25603fedae47d1a521f0b6fa420451816c3a`,
  head `19b77dad63060bcadd41f1ef800978d2c53cc030`, retained `panel-request.json`
  SHA-256 `15f49657490410f0fb5530513144c7c2392f567b211eb630551f3110b94633f7`,
  the exact accepted evidence inventory and digests, zero attestations, and no seal. Those
  bytes are immutable historical evidence. They MUST NOT be described as passed gates,
  changed, supplemented, re-attested, replaced, recovered, or sealed.

  Before T221 may dispatch any Wave 6 implementation, it MUST fetch `origin/v3` and use its
  exact resolved commit as the entry base. The production historical-predecessor guard MUST
  identify on that base's first-parent lineage, after the Wave 5 merge, the accepted
  integration commit whose tree carries the exact generic Constitution 3.1.0 bytes; require that
  commit and the Wave 5 merge to be ancestors of the Wave 6 base and head; and match the exact
  retained candidate root, request, snapshot file, evidence inventory, and every digest.
  Missing, extra, changed, partial, non-ancestor, non-first-parent, unfetched, or substituted
  state MUST refuse. The focused delivery-guard validation MUST pass before the ordinary T221
  plan lifecycle runs. T221 then MUST obtain unanimous sign-off from its exact selected
  roster against that exact base and current feature snapshot with zero recommendations.
  The production guard runs at the Wave 6 snapshot/entry boundary and again at panel request,
  seal, and merge eligibility. It is process-integrity and signoff tracking, not
  authentication or a security boundary.
- **FR-057**: After FR-036's historical disposition is matched, the program
  MUST distinguish **entry evidence** from **exit evidence**, and
  MUST NOT treat a requirement for one as a requirement for the other. Entry evidence is what
  a wave needs in order to **start implementing**: Gate 0 has passed, its destination paths
  carry no open contention flag, its stack is proposed against the exact named parent commit,
  the heavy-gate semaphore is available, and the fast hermetic suite passes on its entry tree.
  Exit evidence is what a wave needs in order to be **delivered** - sealed and merged: every
  assigned work item recorded as merged, validation evidence imported for the exact snapshot,
  and unanimous selected-roster panel sign-off with zero outstanding recommendations against that
  snapshot. For ordinary prospective pipelining, a missing or absent predecessor seal blocks
  the successor's **exit** and not its **entry**. The sole exception is ADR-046 Wave 6: the
  exact delivery validator/tooling contract instantiates the generic Constitution 3.1.0
  disposition with the merged Wave 5 boundary and retained immutable state, without creating
  a Wave 5 seal. FR-025's prohibition on
  partial-wave advance therefore means a wave is never
  *delivered* early and its evidence is never *accepted* early; it does not mean implementation
  must wait. This matches the delivery contract's ordinary pipelined-start conditions
  restated in FR-048 through FR-050 while keeping T221 and every prospective Wave 6 exit gate.
  W8 is the explicit exception: its T557 triage and T558 entry MUST wait for W7 seal, merge,
  and cleanup, then use the updated `v3` lineage.
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
  resolution of the apparent conflict with FR-039. A push or merge to `v3` MUST NOT publish.
  Before F8 freezes, the release binary/package versions and flake package versions MUST
  equal the changelog's d2b 3.0.0 version, and F8 MUST contain either a matching complete
  prebuilt manifest or the existing explicit source-fallback manifest shape. T573 MUST first
  resolve the current merged `v3` HEAD, prove its tree is byte-identical to the sealed F8
  tree, and bind publication to that merged HEAD commit and release state. It MUST NOT require
  the merged commit OID to equal the sealed feature-tip commit OID; a squash or merge commit
  is eligible only when its tree equals the sealed F8 tree. Only the manual publication
  dispatch for that merged `v3` HEAD may create the immutable tag and release.
  A post-tag manifest commit or pull request MUST NOT be treated as repairing the tag.
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
  start. Retired amendment text has no prospective effect.

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
  desktop shrinks: the revision-2 companion inventory binds `d2b-wlterm` to the qualified
  ShellSession Resource lifecycle and ProcessAttachClient named streams, and the replacement
  references define typed unavailability and refusal states for those surfaces. Treating an
  actionable refusal in one of those published states as a defect would block the release on
  a companion doing exactly what d2b told it to do. **Degradation** is the different case: the
  replacement surface is available and the companion cannot use it.

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
  names them once, in a sentence about colour output, listing three of the four revision-2
  members under non-canonical short names alongside two upstream projects that are not members,
  and omitting `d2b-toolkit`. Revision 2 separately records the negative determination that
  excludes `weezterm`; it is not a fifth member.

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
- **Wave**: The delivery unit. Each wave has entry criteria, provisional candidate snapshots
  reviewed iteratively on the nonbinding phase-panel surface, candidate-bound validation, and
  exactly one binding delivery-panel request for one final immutable candidate. A binding
  finding leaves the wave without a seal and never authorizes a second binding request.
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
  single-Zone declaration of 10 to 20 resources, with at least one production progress event
  after the transition-intent commit and no later than the later of the real owned effect or
  production `Ready` observation. Nix evaluation, build, and profile staging completed before
  that durable start are excluded. The outer 2,000 ms ceiling and every applicable FR-030
  component budget pass independently.

  The authoritative prospective acceptance row collects the exact-F6 operator sample and emits only its assigned evidence
  by accepted Version 2. T479 imports `operator-nix-activation-cleanup`; that identifier is not
  a member of the Wave 5 exact-seven profile. The sole authority for receipt shape,
  publication, incident handling, disposition, recovery, source-floor evidence, fixture and
  poison registries, and traceability is accepted Version 2
  `ADR-046-validation-and-delivery` plus generated `VD2-SC002-RECEIPT`,
  `VD2-SC002-PUBLICATION`, `VD2-SC002-INCIDENT`, `VD2-SC002-DISPOSITION`,
  `VD2-SC002-RECOVERY`, `VD2-SC002-SOURCE-FLOOR`, `VD2-SC002-REGISTRIES`, and
  `VD2-SC002-TRACEABILITY` rows. Until those rows are accepted, generated, ancestor-bound,
  and passing Gate 0, every consumer fails closed. No feature-local field list, digest recipe,
  fixture census, registry count, or transition matrix can satisfy SC-002.
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
  identifying value, and no raw Zone, resource, operation, correlation, or trace identity,
  appears in telemetry, audit, logs, or unrelated error output across the full redaction
  test matrix. Metrics and OTEL resources carry no identity dimension; correlation fields
  use only their typed domain-separated fixed digest. Direct Version 2 operator CLI/JSON
  status and recovery responses may return only the bounded `zoneRef` and `operationId`
  recovery coordinates supplied or received by that operator, and tests prove those fields
  do not propagate into telemetry, spans, exported audit, or unrelated errors.

#### Wave 5 production completion

- **SC-030**: On the exact Wave 5 candidate, every successful Resource API request and watch
  is traceable to one registrar-admitted ComponentSession and its authoritative subject, and
  100 percent of attempted cross-Zone or self-named-subject accesses are denied and audited.
  Unix admission obtains a live pidfd directly with `SO_PEERPIDFD` and verifies credentials,
  generation, cgroup, and liveness against that fd; restart obtains a new fd from the newly
  accepted socket, and unavailable support, numeric-only identity, PID reuse, dead fd,
  mismatch, and ambiguity are denied. The typed broker operation and approved `sys.rs` FFI
  quarantine pass exact-optlen, no-panic, returned-fd cleanup, `FD_CLOEXEC`, ancillary-fd
  count, and `OwnedFd` ownership tests. A new project FFI crate, `nix` wrapper, or local
  session fallback fails the criterion. API-surface and compile-fail seals expose no public
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
  one final result; cross-subject and altered-request/target/verb/revision/idempotency and
  restart mismatches within the selected Zone are denied and audited; the same ID in a
  different Zone is an independent operation, and a different-ID retry obeys
  revision/conflict rules. Inspection always names the Zone. UUIDv7 issuance time and the
  fixed 30-day operation recovery retention bound recovery records; malformed, future, expired, and
  clock-discontinuous IDs are denied before observation or mutation, and pruning an expired
  record never makes its ID reusable. Restart replay produces zero missing records and zero duplicate
  logical exports by fixed operation digest plus mutation ordinal. Raw operation,
  correlation, subject, Zone, resource, and trace canaries occur zero times in audit/export/
  internal-error/log/metric/span/Debug output; constructors accept only typed fixed digests and
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
- **SC-034**: The historical Wave 5 plan described a clean-base analysis, selected-roster
  lifecycle, all-or-nothing editor batch, checkbox-only commit C, and fresh post-edit review.
  Those unchecked records are read-only history and are not reconstructed. The feature-owned
  record preserves the exact one-time Wave 5 disposition and no seal; T221 is the next executable
  gate.

- **SC-035**: The exact W6 close candidate has one candidate-bound passing FR-075 result for
  `vmChecks.x86_64-linux.daemon-restart-vm-survival`, with the exact attr enumerated and built
  and no skip. W2-W4 and Wave 5 retain their exact historical evidence without rerun or
  reconstruction. W6 carries the prospective result only inside
  `w6-cloud-hypervisor-guest-acceptance`, and T480 revalidates it before panel request, seal,
  merge eligibility, and merge.

#### Scale and footprint

- **SC-011**: The resource plane sustains a 10,000-resource working set and 100 concurrent
  watchers while continuing to meet its readiness, latency, and footprint targets.
- **SC-012**: The Zone runtime whole-process resident memory stays at or below 24,576 KiB with
  no baseline subtraction, met by design change rather than by relaxing durability,
  authorization, or audit (FR-030). Corrected disposable-proof and production-fixture
  measurements passed at their recorded tips. Current exact-candidate measurement ownership
  resolves only from authoritative generated rows.
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
  terminal wave after W7 is sealed, merged, and cleaned up. The count is read from the
  manifest at release time and is not fixed at 545.
- **SC-020**: Every prospective wave from W6 through W8 carries a seal bound to its exact
  snapshot, with unanimous selected-roster panel sign-off and zero outstanding
  recommendations. W0/W1 and the retained Wave 5 state remain historical evidence, not
  substitute seals. The release is ineligible unless the integration commit carrying the
  accepted generic Constitution 3.1.0 disposition is on the release lineage and the exact
  feature-owned Wave 5 predecessor guard passed at Wave 6 entry.
- **SC-021**: Zero foundation surfaces remain deliberately unwired from production at
  release: the capabilities delivered in W0 and W1 are reachable through the operator
  surface rather than only through tests.
- **SC-022**: Manual hardware, live-host, and cloud validation tiers have each been executed
  at least once against the final candidate with recorded external evidence, on the
  operator's daily-driver host carrying the real device set.
- **SC-023**: d2b 3.0 is tagged and published from the current merged `v3` HEAD whose tree
  equals the sealed F8 tree, with all six release-gate conditions satisfied against the final
  candidate snapshot.
- **SC-024**: 100 percent of identified desktop companions that consume d2b's public
  operator contracts have a compatible version verified against the release candidate on a
  live host before 3.0 is tagged, so an operator's desktop is not degraded by adopting 3.0.
  The identified set is fixed by FR-064's two-limb membership test; "verified" means exercised
  and classified under FR-063 and passing every condition of FR-065. A Blocked surface,
  including one that could not be classified, holds the release.
- **SC-026**: All seven remaining waves reach the integration lineage through a pull request
  whose gates passed first, with zero waves landing by direct push or by a gate-bypassing
  local merge, and zero intermediate versions published before 3.0.
- **SC-027**: Every prospective wave seals at N/N unanimity for its selected roster and
  merges strictly after its predecessor did. The sole historical predecessor exception is
  ADR-046 Wave 6, whose T221 entry uses the exact merged Wave 5 boundary and immutable
  retained state without claiming a Wave 5 seal. Wave 6's own panel, PR, seal, and
  merge-eligibility gates remain complete. W8 begins neither triage nor entry until W7 seal,
  merge, and cleanup are complete.
- **SC-028**: Every current lifecycle completes one comprehensive discovery, one stable
  shared ledger, batched fixes and self-verification, and scoped verification. Zero findings
  become nonblocking because of a round count: pre-existing late MINOR and NIT observations
  remain nonblocking history, while admitted BLOCKER and MAJOR findings remain blocking.
- **SC-029**: 100 percent of waves pass the pre-panel verification and code-review gates,
  scoped to the wave diff, with zero actionable content findings outstanding, before
  any panel lane is dispatched.

## Assumptions

- The ADR-046 specification set is Accepted and closed. This feature implements the
  specifications as written; it does not renegotiate them. Where implementation reveals a
  specification defect, the correction is made in the specification set through its own
  amendment path, which re-opens the affected validation evidence. The obligation this
  creates is a requirement, not merely an assumption; see FR-056.
- The ADR-046 delivery contract in `ADR-046-validation-and-delivery` governs the binding
  work-panel, seal, and merge-eligibility surfaces; it does not supersede the repository's
  per-wave phase gate. Every current wave uses the candidate-bound lifecycle selection
  artifact and requires every selected seat through discovery and verification. For
  already-dispatched W2-W5, contemporaneous plan-review compliance remains unproven.
  The exact delivery validator/tooling contract applies the generic Constitution 3.1.0
  disposition to that fact only as closed history through merged Wave 5 commit
  `177235ed37188b3be87525e7f016fb43401574c5`; it does not make a later work panel substitute
  for a missed historical gate. T072 and the retired planning sequence remain historical
  evidence and are not executable recovery instructions. W6-W8 must pass their prospective
  plan gates before their first implementation lane, beginning with the exact T221 gate.
- The project constitution applies in full, in particular the audited-privilege boundary,
  the isolation-over-convenience rule, contract versioning, test-layer discipline, and the
  ban on internal process markers in shipped artifacts. The feature cannot amend or waive it;
  FR-036 consumes the accepted generic Constitution 3.1.0 disposition and owns the exact
  ADR-046 validator/tooling instantiation rather than placing specifics in the constitution.
- Delivery proceeds in the specified wave order W2 through W8. Sealing and merging are
  strictly ordered and no partial-wave advance is permitted, but implementation start is
  pipelined; the entry-evidence versus exit-evidence distinction is stated in FR-057. The
  W8 terminal-friction wave is the one entry exception: its triage and plan gate wait for W7
  seal, merge, and cleanup so they operate on actual observed friction.
  program terminates at the release of d2b 3.0, not at feature completeness: the release
  gate is evaluated against the final wave's snapshot, because gating earlier would release
  a candidate that a later wave still modifies.
- Waves W0 and W1 are historically recorded as merged without the required panel receipts
  and seals. Their binding panels cannot now be recreated against one canonical delivered
  snapshot. The legacy-named `waiver-w0-w1.md` remains only the FR-034 evidence record. The
  exact delivery validator/tooling contract applies the generic Constitution 3.1.0
  disposition to those exact deviations together with the W2-W5 history through the merged
  Wave 5 boundary. It does not authorize reconstruction or create missing seals.
- FR-075 requires the pre-ADR-046 operator lifecycle to remain functional throughout W2
  through W6 and makes exact-candidate continuity evidence a wave-close gate. It is replaced
  only by the explicit cutover in W7 and removed under the release gate.
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
