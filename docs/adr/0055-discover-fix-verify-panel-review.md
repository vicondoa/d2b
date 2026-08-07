# ADR 0055: Discover, fix, and verify panel review

- Status: Proposed
- Date: 2026-08-06
- Partially supersedes: [ADR 0053](0053-gascity-contributor-infrastructure.md)
  D7's open-ended review and fix loop, own-findings-only dispatch payload, and
  Gas-City-specific use of the protected panel-and-approval controller; D8's
  single blocking treatment of every recommendation in an admitted final set;
  D9's publication refusal while any finding stands; and D17's closed
  endpoint operation sets, round-input eligibility rules, and accepted-attempt
  replay and audit ordering, only as replaced by the closed endpoint table,
  receipt lifetime, common base-or-conflict attempt identity, fenced worker
  and sink recovery, reconciliable accepted-attempt records, immutable
  permanent replay floor, append-only payload eviction, durable generation
  fenced sink reservation, recovery reserve with plan-id binding,
  migration-conflict preflight with hard-bounded expiring isolated signals,
  fixed-cardinality migration-control conflict signals, sticky
  integrity-checked telemetry health, exclusive migration execution capacity,
  fixed reusable parent-attempt child-command slots, controller epochs,
  reserve incarnations, disjoint control and integrity reserves, direct
  current-state protected and recovery status with one audit event per
  successful migration-status disclosure,
  cross-owner assignment-issuance prepares that bind complete revocation
  capacity before activation, dedicated assignment-revocation audit recovery
  plus mandatory finalization and proof-backed release of unused issuance-time
  capacity before any terminal result, generated per-counter rekey drain headroom,
  a representable identity-free would-cross-threshold drain state,
  identity-free initial and alias-only resumed counter-independent epoch
  rekey recovery, durable immutable child-command capacity prepare, protected
  recovery from every non-healthy telemetry state, a durable pre-barrier
  telemetry failure latch, closed audit-settled telemetry recovery success and
  failure, mandatory deployment-keyed protected-operator attribution in
  status and telemetry audit events, exact caller-disjoint risk recovery, and
  operator-redacted migration audit events,
  exact redacted refusal products, migration-specific audit repair, and
  idempotent append contract below. It also supersedes
  D21's per-seat `held` and `prior_resolutions` state, rotation, rejection of a
  severity ladder, and clean-break refusal to read or admit an earlier
  delivery schema. It narrowly supersedes D21's closed twelve-role pool and
  version 1 selection table with the closed thirteen-role pool and version 2
  table below, adding only optional `build`. D21's seven mandatory seats,
  surface-dependent ten-seat and eight-seat floors, select-every-trigger rule,
  conservative classifier, profile binding, deterministic roster mechanics,
  reviewer identity, and candidate-bound evidence remain in force. D7's
  protected-principal, peer-separated, append-only authority boundary remains
  in force. This record generalizes that boundary for the standalone producer,
  adds protected reviewer, assignment-issuance, assignment-completion, and
  least-authority issue-reader endpoints, and replaces only the closed
  operation sets named below. Approval and risk operations remain absent from
  the orchestrator endpoint.
- Related: [ADR 0048](0048-copilot-native-agent-surface.md), whose
  Copilot-native surface, independent read-only reviewers, pinned bindings,
  helper-assembled records, and staged evidence remain in force. This record
  does not supersede ADR 0048; and
  [ADR 0052](0052-bazel-rust-build-and-test.md), whose Bazel build authority is
  reviewed by the new `build` seat but is not changed here.
- Scope: Panel pool and roster selection, producer ordering, panel review
  lifecycle, finding and final-verdict semantics, compatibility migration,
  review evidence, retention, and convergence metrics.
- Non-scope: Implementing delivery tooling or changing contributor process
  documentation in this change.

## Context

The panel currently converges through repeated review, scoped fixes, and
another review. The first round can find real defects that tests miss, but the
same open-ended loop permits every later round to become another discovery
pass. A candidate can be merge-ready while new MINOR or NIT findings, style
preferences, and optional refactors keep invalidating sign-off.

ADR 0053 D21 improves roster selection and finding continuity. It does not
change the basic loop: a finding produces another content change, every
content change invalidates sign-off, and another panel can discover more
pre-existing findings. Its finding state is also per seat, so duplicate
reports are separate obligations even when they describe one defect.

The committed implementation is narrower than both records. At candidate
`e4635981`, `packages/xtask/src/delivery/panel.rs` still accepts the fixed
ten-role roster, requires `signoff == recommendations.is_empty()`, and admits
only a unanimous set with no recommendations. `PanelRecord.recommendations`
contains arbitrary strings with no issue id or severity. That passing code is
the current behavior. Gas City is not implemented. This Proposed record
decides a replacement target and its implementation order; neither the
thirteen-role roster nor the Gas City producer is described as shipped.

The replacement must not strand a panel already in flight at cutover. An old
complete round and its fixes are completed work, not debris. Compatibility
therefore has to be automatic, version-dispatched, and candidate-bound. It
must preserve exact old bytes without pretending those strings had issue ids
or severities that did not exist.

The desired process keeps independent read-only reviewers, controller-owned
roster selection, pinned binding and observed attestation, immutable candidate
evidence, exhaustive discovery, reviewer continuity, unanimous final sign-off,
and the rule that green tests never waive review. It changes where discovery
ends, how findings are accounted for, and how old work enters that lifecycle.

## Decision

### Panel pool, selection guidance, and producer order

This section is the complete narrow supersession of ADR 0053 D21's pool and
selection table. It does not change D21's substantive remit for an existing
seat. The closed pool has thirteen seats:

| Class | Seats |
| --- | --- |
| Mandatory, on every panel | `software`, `test`, `product`, `docs`, `security`, `observability`, `simplicity` |
| Optional, selected by trigger or floor fill | `reliability`, `agentic`, `nixos`, `networking`, `kernel`, `build` |

`build` is the single canonical lowercase seat name. It is the Bazel and
build-systems expert, not a Bazel-only reviewer. Like every other optional
seat, it is subject to the same candidate binding, pinned reviewer identity,
lifecycle-roster continuity, deterministic selection reasons, per-seat payload
digest, and final-roster unanimity rules.

The code or operative-configuration floor remains ten seats: all seven
mandatory seats and at least three optional seats. The documentation-only
floor remains eight seats: all seven mandatory seats and at least one optional
seat. Every optional trigger that fires selects its seat even when the floor
is already met. The version 2 floor-fill order is
`reliability`, `agentic`, `nixos`, `networking`, `kernel`, `build`. Appending
`build` preserves D21's existing fill outcome when no build rule fires. A
triggered `build` counts toward the floor; it is never displaced by an earlier
fill seat. Ambiguous classification or matching selects the wider result. An
over-bound surface selects all thirteen seats and every software profile, then
retains D21's separate `selection-surface-over-bound` refusal at seal and
publication.

Selection is derived from one bounded change surface. It contains both sides
of every rename, every added and deleted path, added and deleted changed
lines, and controller or harness facts such as the current interpreter,
recognized continuous-integration job kind, and generated-artifact kind.
Path rules inspect both rename sides. Content rules inspect added and deleted
lines. Except for the explicit normative build-contract rule below, content
rules run only on paths classified code or operative under D21. An undecidable
fact over-selects; it never removes a seat.

The following is the normative human selection guidance. Focus text tells an
always-selected seat what to examine; it is not a relevance escape hatch.
Optional rows are selected when any listed rule matches.

| Seat | Class | Focus | When selected |
| --- | --- | --- | --- |
| `software` | Mandatory | Correctness-first control-flow review, error propagation, structure, local conventions, and measured performance. Apply every controller-bound Rust, Python, shell, and Nix profile. The Rust profile retains unsafe, FFI, public API, Cargo SemVer, and workspace dependency-direction depth; build-graph mechanics belong to `build`. | Always. Changed paths and interpreter facts bind all applicable language profiles, including every profile on a mixed-language diff; they never remove the seat. |
| `test` | Mandatory | Coverage of new behavior and failure paths, invisible regression risk, planted negatives, gate placement, and whether cited validation actually proves the change. | Always. Use the staged change surface and validation evidence to decide which behaviors and companions require scrutiny. |
| `product` | Mandatory | Scope and gap analysis, CLI and exit-code behavior, external wire and artifact contracts, schema and version discipline, and operator migration and upgrade experience. | Always. A controller-bound product profile may add contract-specific scope but cannot remove the seat. |
| `docs` | Mandatory | Diataxis placement, changelog and ADR-index coverage, prose-to-schema drift, process-marker and ASCII-dash rules, intra-document coherence, terminology, ambiguity, and links. | Always. Review documentation impact even when no documentation file changed. |
| `security` | Mandatory | Adversarial review of capability and authorization boundaries, privilege separation, sandboxing, secrets and PII, audit shape, and concrete exploit paths under a stated attacker model. | Always. Security-sensitive paths or facts deepen the review; they do not control selection. |
| `observability` | Mandatory | Metric label cardinality, span attributes, log and audit shape, retention, redaction, exporter behavior, and whether failure remains diagnosable. | Always. Review both changed telemetry and observability lost or required by other changes. |
| `simplicity` | Mandatory | The smallest maintainable code and decision surface, reuse rather than reinvention, deletion where it lowers risk, and avoidance of duplicated contracts, dependency sprawl, or complexity laundering. | Always. Apply the code lens to implementations and the artifact lens to ADRs, specifications, and plans. |
| `reliability` | Optional | Resource ownership and cleanup on error or crash, restart and adoption, idempotency, cross-component ordering and concurrency, partial failure, degraded state, and on-disk migration. | Select on D21's `reliability-paths` or `reliability-tokens`: delivery, daemon, broker, resource-store, store, lifecycle, state, session, shutdown, restart, pool, adopt, lock, lease, sync, reconcile, supervisor, or cleanup paths; or operative changed lines involving `Drop for`, spawned tasks or threads, synchronization or atomics, unwind handling, rename and fsync durability, temporary-file publication, schema versioning, `deny_unknown_fields`, or `EBUSY`. |
| `agentic` | Optional | Copilot agents, instructions, prompts, skills, context construction, Gas City formulas and packs, orchestration durability and handoffs, and replacement of prompt-only assurances with mechanical gates. | Select on D21's `agentic-paths`, extended in version 2 for this contract: `.github/agents/**`, prompts, instructions, skills and Copilot instructions; `scripts/copilot/**`; `.gc/**`; any `AGENTS.md`; formula, pack, or prompt-template files; the Copilot and panel contributor pages; ADR 0053 and its prompt-source contract; and ADR 0055. |
| `nixos` | Optional | NixOS module declarations and types, merge priority and `mkDefault` or `mkForce` semantics, assertions, RFC 42 option structure, activation ordering, and the three-root-unit invariant. General Nix expression quality stays with `software`. | Select on D21's `nix-sources`: any `.nix` path, `flake.lock`, or a change under `nixos-modules/**`, `nix/**`, `pkgs/**`, `templates/**`, or `examples/**`, using both sides of a rename. |
| `networking` | Optional | Bridge isolation, firewall posture, DHCP and DNS, routing and MTU or MSS invariants, socket exposure, and coexistence with host networking. | Select on D21's `net-paths` or `net-tokens`: network modules and provider, routing, realm-router or transport paths; basenames naming firewall, nftables, bridge, vsock, DHCP, DNS, resolver, route, interface name, egress, MTU, or NetworkManager; or operative changed lines naming the D21 socket, address, bind, listen, route, NAT, forwarding, resolver, gateway, MTU, MSS, or netlink token set. |
| `kernel` | Optional | Syscall and kernel-interface semantics, pidfd, cgroup v2, namespaces, mounts, signals, ioctl and filesystem behavior, errno handling, and kernel-version assumptions. | Select on D21's `kernel-paths` or `kernel-tokens`: minijail, privileged-broker, or guest-shell-runner paths; or operative changed lines naming pidfd, cgroup, namespace, seccomp, ioctl, `openat2`, resolution or mount flags, procfs, cgroupfs, signals, close-on-exec, locks, mounts, `statx`, `renameat2`, or the D21 errno set. |
| `build` | Optional | Bazel and build systems: build graphs, scheduler and orchestration behavior, toolchains and target triples, hermeticity, runfiles, sandboxes, cross-target builds, local and remote cache boundaries, remote execution, dependency authority, continuous-integration scheduling, and packaging or build integration. | Select when any version 2 `build-*` rule matches: Bazel files or Starlark (`BUILD`, `BUILD.bazel`, `MODULE.bazel`, `MODULE.bazel.lock`, `WORKSPACE*`, `.bzl`, `.bazelrc`, and registered Bazel module, lock, registry, repository, or vendor surfaces); Make targets or build scheduler and orchestration paths; a harness-derived continuous-integration job fact for a changed job that builds, tests, packages, or publishes; toolchain files, target triples, cross-compilation, Cargo, Bazel, or Nix build-authority and integration paths; runfiles, build sandbox, cache, or remote-execution paths or operative changed-line tokens; dependency-hub or lock generation; packaging, release, or artifact-production paths and facts; or a registered normative ADR, specification, or operative-doc build contract. |

The `build` contract rule is deliberately narrower than a prose search. The
table registers eligible build-contract paths and marked regions, initially
the build decisions in ADR 0052 and this section of ADR 0055 plus their
registered specification surfaces. Within one of those regions, or on another
ADR or specification path, a changed added or deleted line selects `build`
only when it contains both a versioned build-contract term and a versioned
normative operator. A pure rename of a registered build-contract path also
selects it. Operative documentation uses the ordinary code-operative content
rule. Path eligibility alone does not select the seat, and a bare mention or
link containing `Bazel` in non-operative prose does not select it. Deletion
and rename cases use the same inputs as additions, so deleting a normative
build contract or renaming a Bazel file cannot evade the seat.

There is one machine-readable selection-rule authority:
`.github/skills/d2b-panel-round/selection-table.json`. Version 2 contains the
pool, classes, floors, fill order, bounds, profiles, exact trigger operands,
fact enums, registered normative paths, and the human guidance rows above.
The table is data, not a second implementation hidden in a script. The
rendered selection-guidance block in
`.github/skills/d2b-panel-round/SKILL.md` is generated from it or checked
byte-for-byte against the same rendering. Agent files carry seat-specific
rubrics; they never choose whether their seat is relevant.

The standard Copilot skill is the first delivery target and the first
producer. Repository-owned staging derives `change-surface.json` and the
proposed selected roster without a caller-supplied seat list. It proposes a
`roster-manifest.json` binding the table version and digest, candidate and
lineage, surface and evidence digests, class, matched rules, selected roster,
profiles, reviewer identities, and each generated per-seat review-artifact
digest. The protected authority independently re-derives and admits the
surface, roster and manifest before dispatch iterates exactly the admitted
manifest and only its selected seats.
The operator, user, and orchestrating agent have no input that can omit a
triggered seat, replace the roster with a smaller one, or hand-author a
per-seat artifact. The orchestrator still synthesizes the shared issue ledger
and assigns stable `R` identifiers under section 4; selection tooling does not
assign issue ids.

Repository-owned staging and delivery helpers are not authority. They run as
the contributor uid and can derive a surface, propose a roster, synthesize
descriptions, and assign proposed `R` ids, but they cannot admit a lifecycle,
roster, ledger mapping, severity event, approval, risk event, or terminal
state. The standard skill is a client of the protected standalone authority
defined below. It fails closed before dispatch when that authority or its
authoritative receipt resolver is unavailable.

Gas City remains deferred and is not a current producer. Its future controller
must consume the same table bytes, change-surface schema, roster-manifest
schema, and generated per-seat artifact contract. It may wrap dispatch in
protected provenance, but it may neither fork a rule nor produce a different
core roster or core artifact for identical inputs. The standard skill does not
wait for Gas City. A future Gas City producer uses the same protected authority
contract and endpoint operation sets; no Gas City implementation is part of
the first delivery.

### Protected authority and closed endpoint operation sets

The authority called `controller` in this record is the protected
panel-and-approval controller boundary from ADR 0053 D7, generalized so the
standalone skill can use it without Gas City. It runs as a principal distinct
from the contributor or agent uid and owns authoritative lifecycle, roster,
ledger, implementation-assignment, severity, approval, accepted-attempt,
replay, outbox and retention state. A deployment may expose peer-authenticated
Unix sockets or resolve opaque receipts from a protected principal, but
same-uid repository files, helper output and self-asserted identity are never
authoritative. If neither protected form is available, the producer returns
`protected-authority-unavailable` and does not dispatch.

This table narrowly replaces D7 and D17's closed endpoint operation sets. Each
endpoint has its own request enum and authentication policy. An operation
absent from an endpoint cannot be reached by presenting another endpoint's
request bytes.

| Endpoint | Authorized caller | Complete operation set |
| --- | --- | --- |
| Orchestrator | `OriginalOrchestratorPeer`: candidate-bound standalone or future Gas City producer peer | `ProposeLifecycleStart`, `RequestPanelDispatch`, `SubmitCandidateSnapshot`, `SubmitLedgerSynthesisProposal`, `RequestImplementationAssignment`, `SubmitImplementationDisposition`, `SubmitImplementationSelfReviewFinding`, `SubmitValidationManifest`, `RequestGeneratedSeatArtifacts`, `ReadLifecycleStatus` |
| Reviewer | `TrustedDispatchedReviewer`: one controller-issued, candidate-bound trusted dispatch for the named seat | `SubmitNativeFindingPage`, `SubmitLateFinding`, `SubmitVerificationJudgment`, `SubmitLegacySourceTriage`, `SubmitLegacySourceTriageVerification`, `SubmitSeverityCorrection`, `SubmitSeverityCorrectionVerification`, `SubmitLedgerMappingConcurrence`, `SubmitRiskAcceptanceVerification`, `SubmitFinalSignoff` |
| Operator | `ProtectedOperator`: protected operator identity resolved from peer evidence | `SubmitApprovalDecision`, `AbandonLifecycle`, `ResumeLifecycle`, `RescopeLifecycle`, `CreateSameScopeCurrentSchemaSuccessor`, `CreateReverificationSuccessor`, `PermanentlyCloseAbandonedLineage`, `ApplyLedgerMappingCorrection`, `IssueRiskOperationIntent`, `RequestNewRiskOperationIntent`, `AcceptMajorRisk`, `RevokeMajorRiskAcceptance`, `RevokeImplementationAssignment`, `ResumeProtectedAttempt`, `FenceProtectedAttempt`, `ReadLifecycleStatus`, `ReadRetentionRecoveryStatus`, `RunControllerRetentionCleanup`, `MigrateRetentionCapacity`, `ReadMigrationRecoveryStatus`, `RepairMigrationSinkAppend`, `CompleteMigrationAuditActivation`, `RepairMigrationControlReserve`, `RekeyMigrationControllerEpoch`, `RecoverMigrationTelemetryHealth` |
| Assignment issuance | `ExactOriginatingAssignmentIssuancePrincipal`: exact controller-owned trusted implementation-dispatch principal or authoritative opaque-receipt resolver that owns the presented protected issuance evidence; evidence must be fresh to create a new assignment | `IssueImplementationAssignment` |
| Assignment completion | `ExactOriginatingAssignmentCompletionPrincipal`: exact trusted dispatch principal or authoritative resolver identity recorded by the originating issuance | `CompleteImplementationAssignment` |
| Issue reader | `OriginalIssueReaderPeer`: authenticated implementer peer presenting an opaque assignment handle, or resolved merge authority | `ResolveImplementationAssignment`, `ReadImplementerIssueView`, `ReadMergeAuthorityMajorIssueView` |
| Attempt status | `OriginalAttemptPeerOrProtectedOperator`: authenticated original peer for the named `AttemptIdentity`, or protected operator identity | `ReadProtectedAttemptStatus` |
| Recovery read | `OriginalAttemptPeerOrProtectedOperator`: authenticated original peer for the named `AttemptIdentity`, or protected operator identity | `ReadLifecycleRecoveryState`, `ReadLedgerRecoveryState`, `ReadAssignmentRecoveryState`, `ReadArtifactRecoveryState`, `ReadRetentionRecoveryState`, `ReadPublicationRecoveryState`, `ReadOriginalRefusalRecoveryState` |
| Recovery read, risk | `ProtectedOperator`: protected operator identity, freshly authenticated for this read | `ReadRiskRecoveryState` |
| Publisher | `ProtectedPublisher`: protected publisher identity | `ConsumePublicationManifest`, `RecordTrustedMergeCompletion`, `ReadPublicationStatus` |

`SubmitApprovalDecision` retains D17's closed
`{approve, revise, rescope, abort}` value. Approval and risk operations,
ledger-mapping mutation, lifecycle termination and permanent close are absent
from the orchestrator endpoint. Status reads do not mutate lifecycle,
assignment, or retention domain state. Ordinary status reads remain accepted
attempts under section 13. `ReadMigrationRecoveryStatus` is the one
authenticated, bounded, serialized per-disclosure-audit exception and is
never an accepted attempt. `ReadImplementerIssueView` is not a status read.
Retention-capacity migration, attempt resume and fencing, assignment
revocation, and cleanup are also absent from the orchestrator endpoint.
Recovery-capacity reservation is not a separable caller operation: the
controller creates it atomically inside each transition that creates an
ineligible record. The attempt-status endpoint authenticates against the
original attempt peer or protected operator before revealing even its safe
projection.
The recovery-read endpoint is narrower still. Every request names one
`AttemptIdentity`; it returns only the domain-separated aliases, closed state,
bounded numerics, timestamps, field codes, and digests that the named
post-eviction recovery variant permits. The original peer cannot enumerate
another attempt, recover protected response bytes, obtain a capability, or
mutate any domain. A protected operator receives the same redacted product except on the
caller-disjoint risk row. Only a freshly authenticated protected operator can
invoke `ReadRiskRecoveryState` and receive its handle-bearing pending variant;
the original peer receives only the exact handle-free
`OriginalPeerRiskRecovery` state-and-action variant defined in section 10.
Lifecycle, ledger, assignment, risk, artifact, retention, and
publication recovery all use the corresponding operation in the table rather
than an invented generic read.
The future Gas City producer does not gain a wider operation set.

Every orchestrator operation is proposal, evidence intake, artifact request or
status. None exposes a direct protected-state mutation. The controller
re-derives and validates any resulting internal transition under its own
principal, and refusal leaves authoritative state unchanged.

Admission is controller-owned and is not another caller operation.
`SubmitLedgerSynthesisProposal` can cause an internal
`ledger_synthesis_admitted` transition only after all section 4 checks pass.
`SubmitSeverityCorrection` is intake; the internal
`severity_correction_admitted` transition occurs only after the source
authorization and independent verification in section 3 are both present.
`SubmitLegacySourceTriage` and
`SubmitLegacySourceTriageVerification` analogously cause an internal
`legacy_source_triage_admitted` transition only as section 12 permits.
Reviewer concurrence is accepted only through the trusted reviewer endpoint.
`RequestImplementationAssignment` is likewise a proposal. Only
`IssueImplementationAssignment`, authenticated on the assignment-issuance
endpoint as the controller-owned trusted dispatch principal or authoritative
resolver that owns the presented originating evidence, can cause the
controller's internal `implementation_assignment_issued` transition.
Every ordinary request frame carries an idempotency key, every operation is
candidate-bound where a candidate exists, and every ordinary endpoint
operation uses the accepted-attempt audit and idempotency contract in section
13. Risk intents use the stronger controller-issued-key rule in section 10.
The five active-migration controls `ResumeProtectedAttempt`,
`FenceProtectedAttempt`, `RepairMigrationSinkAppend`,
`CompleteMigrationAuditActivation`, and `RepairMigrationControlReserve`, plus
the separately sealed `RekeyMigrationControllerEpoch` integrity operation and
controller-wide `RecoverMigrationTelemetryHealth` operation, are
authenticated typed operations. Only the five controls are
`MigrationControlCommand` child transitions of the one accepted migration.
Rekey and telemetry recovery use their dedicated durable state machines.
`ReadMigrationRecoveryStatus` is observational. Those eight operations use
their explicit fixed-slot, dedicated-recovery-record, or serialized
per-disclosure contracts in section 13 and never create a standalone
`ProtectedAttemptId`, accepted journal, replay payload, or attempt tombstone.

The controller owns every `ImplementationAssignment`. The orchestrator may
request one but cannot supply, edit, or attest the assignment that results.
Issuance accepts exactly one protected evidence variant:

- `TrustedImplementationDispatch`, resolved from a controller-owned dispatch
  record; or
- `OpaqueImplementationResolverReceipt`, resolved by an authoritative
  resolver whose principal is distinct from the contributor or agent uid.

Both variants bind the authoritative dispatch or resolver digest, authenticated
implementer run identity, lifecycle, candidate, current ledger mapping version,
exact issue set, assignment kind, issuance and expiry, and a controller-bounded
use limit. The closed assignment kinds are:

- `PrimaryBatch`, whose exact issue set is the complete current ledger and
  whose issue view may contain that complete ledger; and
- `ParallelFixSlice`, whose exact issue set and file-ownership digest are one
  disjoint projection of a controller-validated partition.

Every slice in one partition is pairwise issue-disjoint and file-disjoint.
The union of the slices is exactly the issue set the primary batch delegated.
An overlap, omission, or issue outside the primary assignment is refused
before any slice assignment is issued.

Protected evidence is linear independently of request idempotency. The
controller derives an `ImplementationEvidenceId` from the evidence kind and
the immutable controller dispatch or resolver-receipt identity, excluding all
caller-proposed assignment fields. Its private evidence-consumption index is:

```
ImplementationEvidenceConsumption =
  Reserved {
    evidence_id,
    issuance_prepare_identity,
    issuance_prepare_incarnation,
    accepted_attempt_identity,
    assignment_binding_digest
  }
  | Settled {
      evidence_id,
      controller_private_assignment_id,
      assignment_binding_digest
    }
```

The binding digest covers assignment kind, lifecycle, candidate, mapping
version, exact issue set, file-ownership digest when present, authenticated
implementer run, use limit, issuance, expiry, and the exact originating
principal identity. Issuance first acquires a fenced unique reservation for
`evidence_id`. The ordinary accepted attempt preallocates its assignment
success result and canonical audit event before it creates a prepare:

```
AssignmentIssuancePrepareIncarnation =
  MonotonicallyIncreasingNonZeroU64PerAssignmentRequest

AssignmentIssuanceCanonicalAuditTuple = {
  attempt_identity: AttemptIdentity,
  event_id: AuditEventId,
  event_digest,
  canonical_event_bytes_digest
}

AssignmentIssuancePrepareIdentity =
  digest(
    "d2b:panel:implementation-assignment-issuance-prepare:v1",
    AttemptIdentity,
    AssignmentRequestId,
    ImplementationEvidenceId,
    AssignmentBindingDigest,
    AssignmentIssuancePrepareIncarnation,
    AssignmentIssuanceCanonicalAuditTuple
  )

AssignmentIssuancePrepareBinding = {
  accepted_attempt_identity,
  assignment_request_id,
  protected_evidence_digest,
  assignment_binding_digest,
  prepare_incarnation,
  canonical_issuance_audit_tuple
}

PreparedCancellationPreProofBindingDigest =
  digest(
    "d2b:panel:prepared-cancellation-pre-proof-binding:v1",
    accepted_attempt_identity,
    issuance_prepare_identity_digest,
    prepare_incarnation,
    sink_reservation_alias,
    sink_activation_proof_digest,
    cancellation_reason_code,
    next_prepare_incarnation,
    cancellation_refusal_audit_tuple_digest,
    evidence_reservation_binding_digest,
    request_reservation_binding_digest,
    successor_eligibility_digest,
    controller_capacity_binding_digest
  )

PreparedCancellationActivationBindingDigest =
  digest(
    "d2b:panel:prepared-cancellation-activation-binding:v1",
    prepared_cancellation_pre_proof_binding_digest,
    sink_non_creatable_fence_proof_digest,
    sink_absence_or_cancellation_proof_digest
  )

AssignmentIssuancePrepareState =
  SinkReservationCreateOrAdoptPending {
    controller_reservation_alias,
    revocation_capacity_binding_digest
  }
  | SinkReservationActivationPending {
      controller_reservation_alias,
      sink_reservation_alias,
      sink_reservation_generation,
      sink_prepare_proof_digest
    }
  | PreparedForOrdinaryAudit {
      controller_reservation_alias,
      sink_reservation_alias,
      sink_activation_proof_digest
    }
  | SinkReservationNonCreatableFencePending {
      controller_reservation_alias,
      revocation_capacity_binding_digest,
      cancellation_reason_code
    }
  | SinkReservationProofCancellationPending {
      controller_reservation_alias,
      revocation_capacity_binding_digest,
      sink_non_creatable_fence_proof_digest,
      cancellation_reason_code
    }
  | ControllerReservationReleasePending {
      controller_reservation_alias,
      sink_non_creatable_fence_proof_digest,
      sink_absence_or_cancellation_proof_digest,
      cancellation_reason_code
    }
  | PreparedAttemptCancellationSinkFencePending {
      controller_reservation_alias,
      sink_reservation_alias,
      sink_activation_proof_digest,
      cancellation_reason_code,
      next_prepare_incarnation,
      prepared_cancellation_pre_proof_binding_digest,
      cancellation_refusal_audit_tuple_digest
    }
  | PreparedAttemptCancellationSinkProofPending {
      controller_reservation_alias,
      sink_reservation_alias,
      sink_activation_proof_digest,
      sink_non_creatable_fence_proof_digest,
      cancellation_reason_code,
      next_prepare_incarnation,
      prepared_cancellation_pre_proof_binding_digest,
      cancellation_refusal_audit_tuple_digest
    }
  | PreparedAttemptCancellationRefusalInstallPending {
      controller_reservation_alias,
      sink_reservation_alias,
      sink_non_creatable_fence_proof_digest,
      sink_absence_or_cancellation_proof_digest,
      cancellation_reason_code,
      next_prepare_incarnation,
      prepared_cancellation_activation_binding_digest,
      cancellation_refusal_audit_tuple_digest
    }
```

The prepare identity is controller-derived from the exact ordinary
`AttemptIdentity`, immutable request, evidence and assignment bindings,
monotonic prepare incarnation, and the canonical issuance event id, digest,
and bytes digest. No caller supplies any of them. The transaction that enters
`IssuancePending` first stores the immutable
`AssignmentIssuancePrepareBinding`, evidence reservation, and dedicated
controller revocation reservation, including permanent sink-fence capacity.
The sink creates or adopts only the reservation whose authorization matches
the exact accepted attempt, prepare identity and incarnation, assignment
binding, and canonical issuance audit tuple. The controller records the sink
prepare proof, the sink durably activates that reservation for the same
binding, and only then may the controller enter
`PreparedForOrdinaryAudit`.

`PreparedForOrdinaryAudit` is not assignment activation. It permits only the
ordinary accepted-attempt handler to install the already bound issuance result
as its quarantined effect, replay result, and canonical outbox tuple. The
attempt then traverses `OrdinarySinkAcknowledgementPending`. On the normal
path, only the original acknowledgement for that ordinary generation, event
id, digest, and canonical bytes may enter
`OrdinaryActivationPending::AssignmentIssuance`.
`ActivatePreparedImplementationAssignment` has exactly two disjoint
authorized sources:

- normal `OrdinaryActivationPending::AssignmentIssuance`, carrying that
  original acknowledgement; or
- repair
  `AssignmentIssuanceAuditRepairState::ReplacementAcknowledgedActivationPending`,
  carrying the proof-bound final replacement acknowledgement and either the
  durable `AssignmentIssuanceAuditRepairTombstone` or its exact durable
  tombstone-preparation tuple.

Both sources must match the accepted `AttemptIdentity`, issuance prepare
identity and current incarnation, sink activation proof, assignment binding
digest, evidence and request reservations, successor eligibility, both
capacity reservations, and canonical issuance event id, digest, and bytes
digest. The normal
source must additionally match the original ordinary reservation generation
and original acknowledgement and must carry no repair binding. The repair
source must additionally match the repair identity, initial and final
reservation generations, accumulator root, saturating retry count, unchanged
event, proof-bound final acknowledgement, and exact tombstone or
tombstone-preparation digest and must carry no ordinary activation variant.
Any cross-source field, acknowledgement, generation, or proof substitution is
refused without mutation.

From either source, one controller compare-and-swap installs `Active`, moves
the exact initial or successor eligibility to `Issued`, converts the evidence
reservation to `Settled`, makes the replay result available, creates the
immutable `AttemptTombstone`, and marks the accepted attempt `Completed`. On
the repair source it also verifies an already durable repair tombstone or
materializes the exact preparation as that tombstone, binding the latest
committed accumulator root and retry count. Until that transaction commits,
no assignment capability, active state, issued response, settled evidence, attempt
tombstone, or final repair-floor state is visible. Folded-cycle proof
eligibility remains governed by each earlier compaction boundary. On the
repair source, retained repair-intent and any remaining current-cycle source
bytes become ordinary eligible round input only in the committed final
activation state and only after that state contains the durable permanent
repair tombstone; a durable preparation alone does not make them eligible.

An accepted-attempt crash after `PreparedForOrdinaryAudit` is an
issuance-specific recovery condition, not a generic handler failure. After
fencing the expired worker epoch, one compare-and-swap over the accepted
attempt generation and prepare generation enters
`AssignmentIssuancePreparedRecovery::ResumePreparedHandlerPending`. The
record carries the exact accepted `AttemptIdentity`, prepare identity and
incarnation, sink activation proof, assignment binding digest, and canonical
issuance audit tuple digest. `ResumePreparedAssignmentIssuanceHandler`
revalidates that complete immutable join. If it is valid, it runs only the
original ordinary handler transaction and installs the same issuance success,
quarantined assignment effect, replay result, and canonical issuance outbox.
It cannot synthesize a crash refusal or a different audit tuple.

If that join has a closed integrity failure, a second atomic
compare-and-swap records the canonical
`implementation-assignment-issuance-prepared-recovery-cancelled` refusal
tuple, reserves `next_prepare_incarnation`, and enters
`PreparedAttemptCancellationSinkFencePending`. This is the only cancellation
edge from a prepared accepted attempt. It advances linearly through
`PreparedAttemptCancellationSinkProofPending` and
`PreparedAttemptCancellationRefusalInstallPending`. The sink first
persists a permanent non-creatable fence for the exact prepare identity and
incarnation, then returns proof that the activated issuance reservation was
cancelled. Only those two proofs permit one controller compare-and-swap to
install the unchanged canonical cancellation refusal and ordinary outbox as a
no-effect `QuarantinedPendingAudit` tuple. That transaction does not release
controller capacity, release or restore evidence, restore the assignment
request, expose the reserved next prepare incarnation, or terminalize the old
attempt. Those effects remain quarantined through append and durable
acknowledgement. `ActivatePreparedAssignmentIssuanceCancellation` has exactly two disjoint
authorized sources: normal
`OrdinaryActivationPending::AssignmentIssuanceCancellation` with the original
cancellation-refusal acknowledgement, or repair
`AssignmentIssuanceCancellationAuditRepairState::ReplacementAcknowledgedActivationPending`
with the proof-bound final replacement acknowledgement and either the durable
`AssignmentIssuanceCancellationAuditRepairTombstone` or its exact durable
tombstone-preparation tuple. Both sources must match the accepted
`AttemptIdentity`, prepare identity and incarnation, permanent sink-fence and
proof-cancellation digests, cancellation reason, reserved next incarnation,
request and evidence reservations, successor eligibility, controller capacity,
complete `PreparedCancellationActivationBindingDigest`, and canonical
cancellation-refusal event id, digest, and bytes digest. The normal source must
match its original ordinary generation and acknowledgement and carry no repair
binding. The repair source must match its repair identity, initial and final
generations, accumulator root, saturating retry count, unchanged refusal,
final acknowledgement, and exact tombstone or preparation digest and carry no
ordinary activation variant. Any mismatch is a no-mutation refusal.

The transition into `PreparedAttemptCancellationSinkFencePending` constructs
only `PreparedCancellationPreProofBindingDigest`. It binds the recorded
operands available before either cancellation proof exists and cannot contain
a placeholder fence proof, cancellation proof, or partial
`PreparedCancellationActivationBindingDigest`.
`PreparedAttemptCancellationSinkProofPending` carries that same pre-proof
digest plus the durable fence proof. Only after the sink cancellation proof is
durable does one controller transaction verify the pre-proof operands and both
proof digests, derive the complete
`PreparedCancellationActivationBindingDigest`, and enter
`PreparedAttemptCancellationRefusalInstallPending`. The refusal outbox,
ordinary activation state, and cancellation-repair workspace carry only that
complete digest. Neither pre-proof state can carry or decode the complete
digest, and no later state accepts the pre-proof digest in its place.

From either source, one controller compare-and-swap proof-releases the matching
controller reservation, restores the evidence and request reservations,
returns the same assignment request to `RequestPending` with the reserved
strictly greater prepare incarnation, exposes that fresh-incarnation
eligibility, makes the replay result available, creates the immutable
`AttemptTombstone`, and marks the old accepted attempt `Completed`. On the
repair source it also verifies or materializes the exact cancellation-repair
tombstone binding the latest committed accumulator root and retry count.
On that repair source, retained repair-intent and any remaining current-cycle
source bytes become ordinary eligible round input only in the committed final
activation state and only after that state contains the durable permanent
cancellation-repair tombstone; a durable preparation alone does not make them
eligible.
Thus no terminal attempt can coexist with live issuance prepare, evidence,
controller capacity, or sink capacity. Time, worker loss, a missing
observation, or a failed resume is not a fence or cancellation proof.
The fenced and cancelled sink reservation is the issuance-time revocation
reservation. The accepted attempt's distinct ordinary-audit reservation
remains bound to the cancellation refusal tuple.
A crash before the cancellation intent leaves
`ResumePreparedHandlerPending`. A crash after the intent, sink fence request,
fence proof, sink cancellation request, or cancellation proof resumes only
the exact recorded cancellation state and action. The final controller
transaction is indivisible: restart observes either
`CancellationRefusalInstallPending`, `QuarantinedPendingAudit`,
`OrdinarySinkAcknowledgementPending`, or the specialized
`OrdinaryActivationPending`, or one exact
`AssignmentIssuanceCancellationAuditRepairState`, with all controller,
evidence, and request reservations still owned, or the completed old attempt
and restored `RequestPending` state together. It never observes released
capacity, restored evidence, or fresh-incarnation eligibility before the
refusal acknowledgement.

Originating evidence is then consumed for authority purposes: it can identify
and replay the settled assignment but cannot mint another one. Two concurrent
issuances therefore cannot both mint, and a crash can expose neither an
unindexed assignment nor a settled consumption without its assignment.
Reissuing byte-identical evidence and bindings returns the same assignment
even under a fresh idempotency key. Reusing that evidence with a different
kind, issue set, candidate, mapping version, implementer run, expiry, origin,
use limit, lifecycle, or file-ownership digest is
`implementation-assignment-evidence-conflict`, with a closed field code.
Different bytes under the same key are the section 13 protected replay
conflict and never reach evidence evaluation. A fresh key never bypasses the
evidence index. A genuinely new assignment requires fresh protected dispatch
or resolver evidence.

Startup scans every nonterminal issuance prepare before admitting assignment
use. It queries both capacity owners by the immutable accepted-attempt and
prepare binding, adopts an exact matching controller or sink reservation,
resumes sink activation, prepared-handler recovery, prepared-attempt
cancellation, or ordinary audit, or marks the prepare non-adoptable and
enters `SinkReservationNonCreatableFencePending`. The sink must first
persist a permanent fence tombstone for that exact prepare identity and
incarnation. Once fenced, no delayed creation authorization can create,
adopt, or activate capacity for that incarnation. Only the resulting durable
non-creatable proof permits
`SinkReservationProofCancellationPending` to obtain a sink proof that no
reservation exists or that the exact reservation was cancelled. Both the
non-creatable and cancellation proofs bind the accepted attempt, prepare
identity and incarnation, assignment binding, and canonical issuance audit
tuple.
`ControllerReservationReleasePending` then releases controller capacity and
returns the evidence and request to their pre-prepare eligibility inside the
same accepted attempt. Its next retry allocates the next prepare incarnation;
it never restores or reuses the fenced incarnation. Time, worker loss, or an
absent controller observation is neither a non-creatable fence nor
cancellation proof. A sink reservation cannot be created without a durable
controller prepare, and no prepare is deleted before the permanent sink fence,
proof-cancellation, and controller release. Thus startup closes crashes before
sink creation, after sink creation but before adoption, after adoption, after
sink activation, and at every cancellation boundary without exposing an
under-reserved assignment, leaving an orphan reservation, or allowing stale
authorization to revive capacity.

The controller-private assignment id and opaque capability handle never
appear in an error, log, audit event, status projection, or `Debug`. A
domain-separated `PresentedAssignmentAlias` is a non-capability digest used
for safe correlation; possessing it cannot resolve or use an assignment.

An assignment is either single-use or carries a closed use limit no greater
than the versioned controller maximum. Its controller-owned state is exactly:

```
ImplementationAssignmentState =
  Active {
    activated_uses,
    reserved_uses,
    revocation_capacity_binding_digest,
    controller_reservation_alias,
    sink_reservation_alias,
    sink_activation_proof_digest
  }
  | RevocationPending {
      reason,
      revocation_identity_alias,
      revocation_event_id,
      revocation_event_digest,
      reserved_uses,
      audit: AssignmentRevocationAuditState
    }
  | RevocationReadyToFinalize {
      reason,
      revocation_identity_alias,
      revocation_event_id,
      acknowledgement_digest,
      reserved_uses: 0
    }
  | RevocationCapacityReleasePending {
      terminal_intent: AssignmentTerminalIntent,
      release: AssignmentRevocationCapacityReleaseState
    }
  | Completed { completion_event_id }
  | Revoked { revocation_event_id, reason_code }
  | Expired { expired_at }
  | Exhausted { activated_uses }

AssignmentTerminalIntent =
  Complete { completion_event_id }
  | Expire { expired_at }
  | Exhaust { activated_uses }

AssignmentRevocationCapacityReleaseState =
  SinkProofCancellationPending {
    controller_reservation_alias,
    sink_reservation_alias,
    terminal_intent_digest
  }
  | ControllerCapacityReleasePending {
      controller_reservation_alias,
      sink_cancellation_proof_digest,
      terminal_intent_digest
    }
  | TerminalInstallPending {
      sink_cancellation_proof_digest,
      controller_release_proof_digest,
      terminal_intent_digest
    }

AssignmentRevocationAuditState =
  RevocationAuditOutboxPending {
    sink_reservation_generation
  }
  | RevocationAuditSinkAcknowledgementPending {
      appendable_reservation_generation
    }
  | RevocationAuditOldGenerationInvalidationPending {
      old_reservation_generation
    }
  | RevocationAuditOldGenerationInvalidatedRebindPending {
      invalidated_reservation_generation,
      invalidation_proof_digest
    }
  | RevocationAuditReplacementBoundAppendPending {
      replacement_reservation_generation,
      invalidation_proof_digest,
      rebind_proof_digest
    }
  | RevocationAuditAcknowledged {
      acknowledgement_digest
    }

AssignmentRevocationAuditSafeState =
  AuditOutboxPending
  | SinkAcknowledgementPending { reservation_generation }
  | OldGenerationInvalidationPending { reservation_generation }
  | OldGenerationInvalidatedRebindPending {
      invalidated_reservation_generation,
      invalidation_proof_digest
    }
  | ReplacementBoundAppendPending {
      replacement_reservation_generation,
      rebind_proof_digest
    }
  | AuditAcknowledged { acknowledgement_digest }

AssignmentRevocationAuditWorkSafeState =
  AuditOutboxPending
  | SinkAcknowledgementPending { reservation_generation }
  | OldGenerationInvalidationPending { reservation_generation }
  | OldGenerationInvalidatedRebindPending {
      invalidated_reservation_generation,
      invalidation_proof_digest
    }
  | ReplacementBoundAppendPending {
      replacement_reservation_generation,
      rebind_proof_digest
    }

AssignmentRevocationAuditEventId =
  digest(
    "d2b:panel:implementation-assignment-revocation-audit:v1",
    ControllerPrivateAssignmentId,
    AssignmentRevocationIdentity
  )
```

`IssueImplementationAssignment` cannot create `Active` until the issuance
prepare has sealed and verified one dedicated controller reservation,
revocation outbox, and activated sink reservation at the schema maximum, and
the exact ordinary issuance audit acknowledgement is durable.
These issuance-time reservations cannot be borrowed, resized, or recreated
from ordinary capacity. `RevocationPending` refuses
completion with its exact pending state and cannot return to `Active`.
`CompleteImplementationAssignment` freezes a `Complete` terminal intent and
enters `RevocationCapacityReleasePending`; it reaches `Completed` only after
the unused dedicated capacity is proof-cancelled and released.
`RevokeImplementationAssignment` first
authenticates the protected operator, resolves the exact active assignment,
strictly validates its closed reason, and verifies that dedicated capacity.
An authorization, decoding, resolution, state, or capacity preflight failure
creates no revocation identity, event, outbox, or pending state. After those
checks, one controller transaction creates a controller-issued revocation
identity and immutable canonical event bytes, binds the dedicated capacity,
and enters `RevocationPending::RevocationAuditOutboxPending` while atomically
closing new use reservations. Even when `reserved_uses` is zero, the
assignment remains pending until audit acknowledgement is durable.

The revocation event is never handled by ordinary quarantined-effect
conversion. The dedicated audit substates append and fsync the same event id,
digest, and canonical bytes. Timeout or unknown append outcome retries the
same authorized append. An authenticated definite-no-append proof advances
only through the dedicated old-generation invalidation, proof-bound rebind,
and replacement append substates above. It never restores `Active`, installs
`audit-event-flush-failed`, or terminalizes to a generic refusal. The event id
is bound to its immutable digest before the first append, so no retry can use
the same event id with different bytes.

Already reserved uses may settle in every revocation audit substate. The
settlement transaction decrements only `reserved_uses`. If audit
acknowledgement arrives while uses remain, the assignment stays
`RevocationPending::RevocationAuditAcknowledged`; settlement of the last use
then enters `RevocationReadyToFinalize`. If the last use settles before audit
acknowledgement, the assignment remains in its exact audit-work substate and
the later acknowledgement enters `RevocationReadyToFinalize`. Concurrent
last-use settlement and acknowledgement has the same compare-and-swap result.
Neither ordering installs `Revoked`; only `FinalizeAssignmentRevoked` from
that exact intermediate state does so. Startup scans every pending
revocation before admitting assignment use or completion, fences a stale
revocation worker by record digest, adopts the exact outbox or acknowledgement
state, and resumes only the recorded audit action. A crash before the initial
transaction leaves `Active`; a crash after it can reveal only
`RevocationPending`; a crash after sink fsync replays the same append and
obtains the original acknowledgement; and a crash after acknowledgement but
before finalization recovers the exact ready state and atomic finalization
when the use count is zero. Completion, expiry, exhaustion, duplicate revocation, use
settlement, and restart all compare-and-swap the same assignment record, so
none can bypass the two-condition finalization or reactivate authority.

`Complete`, `Expire`, and `Exhaust` intents reached without revocation have a
separate mandatory capacity-release path. The winning completion, expiry, or
final-use transaction closes new use and revocation admission, freezes the
exact `AssignmentTerminalIntent`, and enters
`RevocationCapacityReleasePending::SinkProofCancellationPending`; it does not
install the terminal state yet. The sink must durably proof-cancel the sealed
unused revocation reservation before the controller releases its matching
reservation. The controller then records the sink cancellation proof and its
own release proof before `TerminalInstallPending` atomically installs the
frozen `Completed`, `Expired`, or `Exhausted` state. Time, lease expiry, and an
assumption that revocation never started are not cancellation proof.

Startup scans this release state before terminal cleanup or successor
admission. It replays only the exact proof-cancellation, controller-release,
or terminal-install action owned by the recorded variant. A crash before the
first transaction leaves `Active`; a crash after it leaves the immutable
terminal intent and sealed reservations; and a crash after either release
proof resumes the next state without double release. No terminal cleanup,
successor eligibility, or reuse of either reservation is visible until the
terminal install commits. Internal `CandidateChanged`,
`MappingSuperseded`, and `LifecycleTerminated` invalidations never use this
cancellation path: they enter the same auditable `RevocationPending` state
machine as operator revocation and consume the sealed capacity.

Completion requires
fresh, single-consumption,
assignment-bound completion evidence from the exact trusted dispatch principal
or authoritative resolver identity recorded by issuance. That evidence binds
the exact originating principal identity, originating assignment-issuance
evidence identity, controller-private assignment id, lifecycle, candidate,
mapping version, final issue set, implementer run, closed completion result,
authoritative issuance time, mandatory finite expiry, and its own declared
evidence identity. The controller resolves the protected evidence record and
re-derives every one of those fields; no caller-supplied field establishes a
binding.

Completion validation uses these closed field codes:

```
AssignmentCompletionOriginCode =
  OriginatingPrincipal
  | OriginatingIssuanceEvidence

AssignmentCompletionBindingFieldCode =
  AssignmentId
  | Lifecycle
  | Candidate
  | MappingVersion
  | FinalIssueSet
  | ImplementerRun
  | CompletionResult
  | IssuedAt
  | ExpiresAt
  | EvidenceIdentity

AssignmentCompletionFreshnessCode =
  Stale
  | Expired
```

A different originating principal or originating issuance evidence selects
the exact `AssignmentCompletionOriginCode`. A mismatch in any remaining bound
field selects the exact `AssignmentCompletionBindingFieldCode`. Evidence that
is not yet valid or otherwise stale selects `Stale`; evidence past its expiry
selects `Expired`. Both are a separate refusal. The controller derives
`AssignmentCompletionEvidenceId`
from the evidence kind, originating principal identity, originating issuance
evidence identity, controller-private assignment id, declared evidence
identity, and domain separator, explicitly excluding the mutable evidence
digest and bound completion fields. The full
`AssignmentCompletionBindingDigest` covers every bound field listed above,
including the exact originating principal and originating issuance evidence.
The single-consumption index stores the internal evidence id, immutable
evidence digest, and full assignment-binding digest atomically with the
`RevocationCapacityReleasePending` completion intent. It does not record or
report `Completed` until terminal installation commits.

The internal evidence id is never serialized. Its only public correlate is:

```
CompletionEvidenceAlias =
  digest(
    "d2b:panel:completion-evidence-alias:v1",
    AssignmentCompletionEvidenceId
  )
```

`CompletionEvidenceAlias` is domain separated, cannot resolve evidence, and
confers no completion authority. Every completion refusal, including replay
and conflict, uses exactly one variant of this product:

```
AssignmentCompletionRefusalProduct =
  OriginMismatch {
    presented_assignment_alias,
    completion_evidence_alias,
    presented_principal_alias,
    origin_code: AssignmentCompletionOriginCode
  }
  | BindingMismatch {
      presented_assignment_alias,
      completion_evidence_alias,
      field_code: AssignmentCompletionBindingFieldCode
    }
  | StaleOrExpired {
      presented_assignment_alias,
      completion_evidence_alias,
      issued_at,
      expires_at,
      freshness_code: AssignmentCompletionFreshnessCode
    }
  | EvidenceReplay {
      presented_assignment_alias,
      completion_evidence_alias,
      immutable_evidence_digest,
      stored_assignment_binding_digest,
      replay_code: ExactEvidenceIdentityAndBinding
    }
  | EvidenceConflict {
      presented_assignment_alias,
      completion_evidence_alias,
      conflict: CompletionEvidenceConflict
    }

CompletionEvidenceConflict =
  AssignmentBindingDigest {
    authoritative_assignment_binding_digest,
    presented_assignment_binding_digest
  }
  | ImmutableEvidenceDigest {
      authoritative_immutable_evidence_digest,
      presented_immutable_evidence_digest
    }
```

The local refusal, canonical refusal audit event, refusal catalog row,
`AttemptTombstone`, `ProtectedAttemptRecovery::OriginalRefusal`, status and
retention projections, logs, derived or handwritten `Debug`, and fixtures
serialize that same product without additions or subtractions. They never
serialize a raw completion-evidence identity,
originating issuance evidence identity, controller-private assignment id,
protected principal mapping, evidence bytes, capability handle, path, or
deployment id.

A settled reuse is replay only when the immutable completion-evidence identity,
immutable evidence digest, and full stored assignment-binding digest all match.
The same evidence identity with a changed full binding digest is
`implementation-assignment-completion-evidence-conflict`, never replay and
never an ordinary one-field binding mismatch. With the binding digest equal,
a changed immutable evidence digest is the other conflict code. Only equality
of both digests is replay. Neither replay nor conflict reaches a state
transition. Both return the exact current safe assignment state through
`ReadAssignmentRecoveryState`; neither authorizes a new assignment.

Assignment recovery is context-specific and never infers capability loss from
replay-payload eviction. The top-level tag says why recovery was requested;
the controller never interprets one generic state differently according to a
source operation. `ReadAssignmentRecoveryState` names one closed context and
the source attempt or assignment alias that authorizes it; a mismatch is a
typed refusal, and the response repeats that same top-level context:

```
AssignmentRecoveryContext =
  StatusUse(AssignmentStatusUseRecovery)
  | CompletionRecovery(AssignmentCompletionRecovery)
  | RevocationRecovery(AssignmentRevocationRecovery)
  | IssueViewRecovery(AssignmentIssueViewRecovery)
  | FreshAssignmentFlow(AssignmentFreshFlowRecovery)

AssignmentStatusUseRecovery =
  ActiveUsable {
      presented_assignment_alias,
      activated_uses,
      reserved_uses: 0,
      maximum_uses,
      expires_at,
      next: UseExistingCapability {
        resolve: IssueReader.ResolveImplementationAssignment,
        issue_view: IssueReader.ReadImplementerIssueView,
        caller: OriginalIssueReaderPeer
      }
    }
  | ActiveUsesTemporarilyReserved {
      presented_assignment_alias,
      activated_uses,
      reserved_uses,
      maximum_uses,
      expires_at,
      next: RecoveryRead.ReadAssignmentRecoveryState {
        context: StatusUse,
        caller: OriginalAttemptPeerOrProtectedOperator
      }
    }
  | RevocationPendingNoNewUses {
      presented_assignment_alias,
      reason,
      revocation_identity_alias,
      revocation_event_id,
      audit_state: AssignmentRevocationAuditSafeState,
      reserved_uses,
      next: RecoveryRead.ReadAssignmentRecoveryState {
        context: RevocationRecovery,
        caller: OriginalAttemptPeerOrProtectedOperator
      }
    }
  | RevocationCapacityReleasePending(
      AssignmentRevocationCapacityReleaseRecovery
    )
  | Terminal(AssignmentTerminalRecovery)

AssignmentCompletionRecovery =
  ActiveCompletionRetry {
      presented_assignment_alias,
      retry:
        UseOriginatingAssignmentCompletionPrincipalAndFreshEvidence {
          next: AssignmentCompletion.CompleteImplementationAssignment
            by ExactOriginatingAssignmentCompletionPrincipal
        }
        | RequestFreshAssignmentBoundCompletionEvidence {
            next: AssignmentCompletion.CompleteImplementationAssignment
              by ExactOriginatingAssignmentCompletionPrincipal
          }
    }
  | ActiveCompletionWaitingForReservedUses {
      presented_assignment_alias,
      reserved_uses,
      next: RecoveryRead.ReadAssignmentRecoveryState {
        context: CompletionRecovery,
        caller: OriginalAttemptPeerOrProtectedOperator
      }
    }
  | RevocationPendingCompletionRefused {
      presented_assignment_alias,
      reason,
      revocation_identity_alias,
      revocation_event_id,
      audit_state: AssignmentRevocationAuditSafeState,
      reserved_uses,
      next: RecoveryRead.ReadAssignmentRecoveryState {
        context: RevocationRecovery,
        caller: OriginalAttemptPeerOrProtectedOperator
      }
    }
  | RevocationCapacityReleasePending(
      AssignmentRevocationCapacityReleaseRecovery
    )
  | Terminal(AssignmentTerminalRecovery)

AssignmentRevocationRecovery =
  ActiveRevocationRequired {
      presented_assignment_alias,
      declared_reason: CapabilityUnavailable | CapabilityAbandoned,
      next: Operator.RevokeImplementationAssignment by ProtectedOperator
    }
  | RevocationAuditWorkPending {
      presented_assignment_alias,
      reserved_uses,
      reason,
      revocation_identity_alias,
      revocation_event_id,
      audit_state: AssignmentRevocationAuditWorkSafeState,
      next: ResumeAssignmentRevocationAudit
    }
  | RevocationAuditAcknowledgedUsesReserved {
      presented_assignment_alias,
      reserved_uses: NonZeroBoundedUseCount,
      reason,
      revocation_identity_alias,
      revocation_event_id,
      acknowledgement_digest,
      next: RecoveryRead.ReadAssignmentRecoveryState {
        context: RevocationRecovery,
        caller: OriginalAttemptPeerOrProtectedOperator
      }
    }
  | RevocationReadyToFinalize {
      presented_assignment_alias,
      reserved_uses: 0,
      reason,
      revocation_identity_alias,
      revocation_event_id,
      acknowledgement_digest,
      next: FinalizeAssignmentRevoked
    }
  | RevocationCapacityReleasePending(
      AssignmentRevocationCapacityReleaseRecovery
    )
  | Terminal(AssignmentTerminalRecovery)

AssignmentIssueViewRecovery =
  ActiveIssueViewUsable {
      presented_assignment_alias,
      activated_uses,
      reserved_uses: 0,
      maximum_uses,
      expires_at,
      next: IssueReader.ReadImplementerIssueView by OriginalIssueReaderPeer
    }
  | ActiveIssueViewUseReserved {
      presented_assignment_alias,
      reserved_use_ordinal,
      next: RecoveryRead.ReadAssignmentRecoveryState {
        context: IssueViewRecovery,
        caller: OriginalAttemptPeerOrProtectedOperator
      }
    }
  | RevocationPendingNoIssueViewUse {
      presented_assignment_alias,
      reason,
      revocation_identity_alias,
      revocation_event_id,
      audit_state: AssignmentRevocationAuditSafeState,
      reserved_uses,
      next: RecoveryRead.ReadAssignmentRecoveryState {
        context: RevocationRecovery,
        caller: OriginalAttemptPeerOrProtectedOperator
      }
    }
  | RevocationCapacityReleasePending(
      AssignmentRevocationCapacityReleaseRecovery
    )
  | Terminal(AssignmentTerminalRecovery)

AssignmentRevocationCapacityReleaseRecovery =
  SinkProofCancellationPending {
      presented_assignment_alias,
      terminal_intent: AssignmentTerminalIntent,
      sink_reservation_alias,
      next: ProofCancelUnusedAssignmentRevocationSinkCapacity
    }
  | ControllerCapacityReleasePending {
      presented_assignment_alias,
      terminal_intent: AssignmentTerminalIntent,
      sink_cancellation_proof_digest,
      next: ReleaseUnusedAssignmentRevocationControllerCapacity
    }
  | TerminalInstallPending {
      presented_assignment_alias,
      terminal_intent: AssignmentTerminalIntent,
      sink_cancellation_proof_digest,
      controller_release_proof_digest,
      next: InstallAssignmentTerminalIntent
    }

AssignmentTerminalRecovery =
  Completed {
      presented_assignment_alias,
      completion_event_id,
      successor: AssignmentSuccessorEligibility
    }
  | Revoked {
      presented_assignment_alias,
      revocation_event_id,
      reason_code,
      successor: AssignmentSuccessorEligibility
    }
  | Expired {
      presented_assignment_alias,
      expired_at,
      successor: AssignmentSuccessorEligibility
    }
  | Exhausted {
      presented_assignment_alias,
      activated_uses,
      maximum_uses,
      successor: AssignmentSuccessorEligibility
    }

AssignmentIssuancePrepareRecovery =
  SinkReservationCreateOrAdoptPending {
    issuance_prepare_alias,
    prepare_incarnation,
    protected_evidence_digest,
    canonical_issuance_audit_tuple_digest,
    next: CreateOrAdoptAssignmentIssuanceSinkReservation
  }
  | SinkReservationActivationPending {
      issuance_prepare_alias,
      prepare_incarnation,
      protected_evidence_digest,
      canonical_issuance_audit_tuple_digest,
      sink_reservation_alias,
      sink_reservation_generation,
      next: ActivateAssignmentIssuanceSinkReservation
    }
  | PreparedForOrdinaryAudit {
      issuance_prepare_alias,
      prepare_incarnation,
      protected_evidence_digest,
      sink_activation_proof_digest,
      canonical_issuance_audit_tuple_digest,
      next: AttemptStatus.ReadProtectedAttemptStatus
        by OriginalAttemptPeerOrProtectedOperator
    }
  | SinkReservationNonCreatableFencePending {
      issuance_prepare_alias,
      prepare_incarnation,
      canonical_issuance_audit_tuple_digest,
      cancellation_reason_code,
      next: FenceAssignmentIssuanceSinkIncarnation
    }
  | SinkReservationProofCancellationPending {
      issuance_prepare_alias,
      prepare_incarnation,
      canonical_issuance_audit_tuple_digest,
      sink_non_creatable_fence_proof_digest,
      cancellation_reason_code,
      next: ProofCancelAssignmentIssuanceSinkReservation
    }
  | ControllerReservationReleasePending {
      issuance_prepare_alias,
      prepare_incarnation,
      canonical_issuance_audit_tuple_digest,
      sink_non_creatable_fence_proof_digest,
      sink_absence_or_cancellation_proof_digest,
      cancellation_reason_code,
      next: ReleaseAssignmentIssuanceControllerReservation
    }
  | PreparedAttemptCancellationSinkFencePending {
      issuance_prepare_alias,
      prepare_incarnation,
      canonical_issuance_audit_tuple_digest,
      cancellation_reason_code,
      next_prepare_incarnation,
      prepared_cancellation_pre_proof_binding_digest,
      cancellation_refusal_audit_tuple_digest,
      next: FencePreparedAssignmentIssuanceForCancellation
    }
  | PreparedAttemptCancellationSinkProofPending {
      issuance_prepare_alias,
      prepare_incarnation,
      canonical_issuance_audit_tuple_digest,
      sink_non_creatable_fence_proof_digest,
      cancellation_reason_code,
      next_prepare_incarnation,
      prepared_cancellation_pre_proof_binding_digest,
      cancellation_refusal_audit_tuple_digest,
      next: ProofCancelPreparedAssignmentIssuanceSinkReservation
    }
  | PreparedAttemptCancellationRefusalInstallPending {
      issuance_prepare_alias,
      prepare_incarnation,
      canonical_issuance_audit_tuple_digest,
      sink_non_creatable_fence_proof_digest,
      sink_absence_or_cancellation_proof_digest,
      cancellation_reason_code,
      next_prepare_incarnation,
      prepared_cancellation_activation_binding_digest,
      cancellation_refusal_audit_tuple_digest,
      next: InstallPreparedAssignmentIssuanceCancellationRefusal
    }

PreparedAssignmentIssuanceCancellationReasonCode =
  AcceptedAttemptBindingMismatch
  | PrepareIncarnationMismatch
  | SinkActivationProofInvalid
  | AssignmentBindingIntegrityInvalid
  | CanonicalIssuanceAuditTupleIntegrityInvalid

AssignmentIssuanceCapacityUnavailable =
  ControllerBeforePrepare {
    assignment_request_id,
    required_capacity,
    available_capacity
  }
  | SinkAfterPrepare {
      assignment_request_id,
      issuance_prepare_alias,
      required_capacity,
      available_capacity
    }

AssignmentFreshFlowRecovery =
  InitialAssignment {
      partition_authority_alias,
      eligibility: InitialAssignmentEligibility
    }
  | TerminalSuccessor {
      predecessor_assignment_alias,
      predecessor_terminal_state:
        Completed | Revoked | Expired | Exhausted,
      eligibility: AssignmentSuccessorEligibility
    }

InitialAssignmentEligibility =
  InitialAvailable {
      next: Orchestrator.RequestImplementationAssignment
        by OriginalOrchestratorPeer
    }
  | InitialRequestPending {
      assignment_request_id,
      canonical_request_digest,
      next_prepare_incarnation,
      next: AssignmentIssuance.IssueImplementationAssignment {
        caller: ExactOriginatingAssignmentIssuancePrincipal,
        evidence: FreshProtectedImplementationEvidence
      }
    }
  | InitialIssuancePending {
      assignment_request_id,
      prepare: AssignmentIssuancePrepareRecovery
    }
  | InitialIssued {
      assignment_request_id,
      assignment_alias,
      assignment_event_id,
      next: RecoveryRead.ReadAssignmentRecoveryState {
        context: StatusUse,
        caller: OriginalAttemptPeerOrProtectedOperator
      }
    }

AssignmentSuccessorEligibility =
  Available {
      next: Orchestrator.RequestImplementationAssignment
        by OriginalOrchestratorPeer
    }
  | RequestPending {
      assignment_request_id,
      canonical_request_digest,
      next_prepare_incarnation,
      next: AssignmentIssuance.IssueImplementationAssignment {
        caller: ExactOriginatingAssignmentIssuancePrincipal,
        evidence: FreshProtectedImplementationEvidence
      }
    }
  | IssuancePending {
      assignment_request_id,
      prepare: AssignmentIssuancePrepareRecovery
    }
  | Issued {
      assignment_request_id,
      successor_assignment_alias,
      assignment_event_id,
      next: RecoveryRead.ReadAssignmentRecoveryState {
        context: StatusUse,
        caller: OriginalAttemptPeerOrProtectedOperator
      }
    }
```

`InitialAssignmentEligibility` has the same four linear states and actions as
`AssignmentSuccessorEligibility`, but is anchored in the controller-owned
disjoint partition authority rather than a terminal predecessor. Every
terminal predecessor owns exactly one
`AssignmentSuccessorEligibility`. Its first accepted request atomically moves
`Available` to `RequestPending`, binds one `assignment_request_id`, and
records its first `next_prepare_incarnation`. A proof-backed restoration
returns to that same state with a strictly newer recorded incarnation.
Concurrent duplicates, byte-identical retries, changed caller keys, and fresh
caller keys for that predecessor all replay that same request id; none can
allocate another request. `IssueImplementationAssignment` consumes only that
`RequestPending` request and fresh
`TrustedImplementationDispatch` or `OpaqueImplementationResolverReceipt`
evidence and uses exactly its recorded `next_prepare_incarnation`. Acceptance
moves it to `IssuancePending` only after the immutable
issuance prepare, evidence reservation, and controller revocation reservation
are durable. Its exact nested prepare state owns creation or adoption, sink
activation, ordinary-audit continuation, sink-incarnation fencing,
proof-cancellation, or controller release. Verified sink activation permits
only `PreparedForOrdinaryAudit`. On the normal path, durable original
acknowledgement then permits `ActivatePreparedImplementationAssignment` from
the exact ordinary attempt's
`OrdinaryActivationPending::AssignmentIssuance`; the only other authorized
source is the proof-bound issuance-repair
`ReplacementAcknowledgedActivationPending` described in section 13. The
source-specific checks and one atomic authority-effect transaction above alone
move the eligibility to `Issued` and expose `Active`. A retry before cancellation
continues only the same prepare incarnation. A retry restored after the
permanent non-creatable fence, proof-cancellation, and controller release
allocates the next incarnation for the same pending request. For the
prepared-attempt cancellation branch, the next incarnation is reserved but
not eligible when cancellation starts. Request and evidence restoration,
controller release, fresh-incarnation eligibility, and old-attempt completion
occur together only in the specialized post-acknowledgement activation
transaction from its normal original-acknowledgement source or its
proof-bound cancellation-repair source. A fresh accepted attempt may then
consume only the restored same request and recorded newer incarnation. An old retry
after `Issued` returns the issued successor state without minting another
capability.

Intentional parallel fix slices never consume predecessor successor
eligibility. They remain exclusively under the existing controller-owned
pairwise-disjoint `ImplementationAssignmentPartition` authority; each
partition member has its own initial eligibility and no slice can create a
second successor for another member.

An active assignment returned by issuance replay, completion-evidence replay
or conflict, protected-attempt replay, or post-eviction recovery remains the
same assignment. Its bound issue-reader peer uses the existing opaque
capability; recovery never returns, remints, or substitutes that capability.
`ActiveUsesTemporarilyReserved` and
`ActiveIssueViewUseReserved` expose the exact wait-and-reread action while a
reserved use settles. If another use remains after settlement, the applicable
active-usable variant returns; if the final use activated, the exact
`RevocationCapacityReleasePending` variant is returned until the unused
revocation reservations are proof-cancelled and released, after which
`Exhausted` returns with its one successor eligibility. A
`RevocationPending*` variant never returns a use, completion, or new
revocation action.

Payload eviction proves only that protected response bytes are unavailable.
When the caller explicitly declares an active capability unavailable or
abandoned, the controller returns
`RevocationRecovery::ActiveRevocationRequired`; the only action is
`Operator.RevokeImplementationAssignment` by `ProtectedOperator`. After the
authorization and dedicated-capacity preflight above, the revocation
compare-and-swap atomically enters authoritative `RevocationPending` and
rejects every new use reservation. Already reserved uses may only settle.
Their settlement decrements the pending count. Settlement of the last one
enters `RevocationReadyToFinalize` when
`RevocationAuditAcknowledged` is already durable; otherwise it remains in its
exact `RevocationAuditWorkPending` state. If acknowledgement arrives with
live uses, it becomes `RevocationAuditAcknowledgedUsesReserved`; settlement
of the last use then enters `RevocationReadyToFinalize`. If acknowledgement
arrives last with zero uses, it enters the same state. Only its exact
finalization action installs `Revoked`. Expiry or the use limit reaching its
bound meanwhile cannot bypass that acknowledged-zero-use intermediate.
No recovery, restart, completion, expiry, or use settlement can return
`RevocationPending` to `Active`. Recovery returns `RevocationAuditWorkPending` while any append,
acknowledgement, invalidation, rebind, or replacement-append work remains;
that variant alone owns `ResumeAssignmentRevocationAudit`.
`RevocationAuditAcknowledgedUsesReserved` owns only wait and reread.
`RevocationReadyToFinalize` is constructible only with durable
acknowledgement and zero reserved uses and alone owns
`FinalizeAssignmentRevoked`. There is no universal audit-resume action.
Only
the resulting durable `Revoked` predecessor exposes
`AssignmentSuccessorEligibility::Available`. Old issuance evidence, an old
completion conflict, a tombstone, payload eviction, caller-key change, or a
loss declaration is never fresh-assignment eligibility.

The generated assignment-context-to-action join admits use only from the two
usable active contexts, completion retry only from
`ActiveCompletionRetry`, first revocation only from
`ActiveRevocationRequired`, revocation audit work only from
`RevocationAuditWorkPending`, wait and reread only from
`RevocationAuditAcknowledgedUsesReserved`, finalization only from
`RevocationReadyToFinalize`, release reconciliation only from the exact
`AssignmentRevocationCapacityReleaseRecovery` variant, and the four
fresh-flow actions only from their matching eligibility states. It checks endpoint, operation, caller,
authoritative assignment state, partition, predecessor, request id, and
fresh-evidence prerequisites. Every nested issuance prepare state admits only
its exact create-or-adopt, sink-activation, ordinary-audit read,
sink-incarnation fence, proof-cancellation, controller-release,
prepared-handler resume, prepared-incarnation cancellation fence,
prepared-reservation proof-cancellation, refusal-install, or specialized
post-audit activation action. Every nested issuance success or cancellation
audit-repair state admits only its exact invalidation, invalidation replay,
unchanged-event rebind or generation rollover, accumulator-folding replacement
install, replacement append, repeated definite-no-append loop,
acknowledgement, or specialized post-audit activation action. Assignment
activation is admitted from exactly two disjoint source classes: the matching
ordinary accepted attempt's outcome-specific `OrdinaryActivationPending`
variant with its original acknowledgement, or that same attempt's
outcome-specific repair `ReplacementAcknowledgedActivationPending` with its
proof-bound final acknowledgement and durable repair tombstone or preparation.
`InitialAssignmentEligibility` and every
terminal predecessor's `AssignmentSuccessorEligibility` remain exactly one
request and one issuance: `Available -> RequestPending -> IssuancePending ->
Issued`. The join rejects a context inferred from the source operation, a
generic active or terminal action, a status-only state paired with a mutation,
any state/action cross-product, and any duplicate capability path.

`RevokeImplementationAssignment` is not present on either assignment endpoint.
Only the protected operator endpoint may request revocation. The controller
may also perform one closed internal invalidation transition, with no caller
operation, for exactly `CandidateChanged`, `MappingSuperseded`, or
`LifecycleTerminated`. Operator revocation and internal invalidation both
carry a closed reason code and event id, use the same dedicated revocation
audit state machine and pre-sealed capacity, atomically enter
`RevocationPending`, and reject every later reservation. With no live
reservation they still remain pending until the revocation audit is
acknowledged; there is no unaudited direct transition to `Revoked`. The
originating issuer or resolver alone cannot revoke. The
authoritative clock refuses a new reservation at or after expiry, but does not
invalidate a use reserved before expiry.
`Active` is constructible only with the complete immutable controller
reservation, activated sink reservation, and sink activation proof recorded
in the state above. A transient capacity-owner outage leaves those verified
reservations complete but denies new use and revocation until the owner is
reachable. If any binding or proof fails integrity verification, the record is
quarantined and cannot decode or project as authoritative `Active`; no use,
completion, or revocation action is admitted until the exact dedicated
capacity binding is restored. An internal invalidation never continues to
authorize new use merely because its audit capacity needs repair.
Outside `RevocationPending`, settlement of the last live reservation moves the
assignment to `RevocationCapacityReleasePending` with an `Exhaust`,
`Expire`, or `Complete` terminal intent when the corresponding terminal
predicate wins, and otherwise back to unreserved `Active`. Inside
`RevocationPending`, the last settlement remains in its exact audit-work state
when acknowledgement is not durable and enters
`RevocationReadyToFinalize` when it is. It never moves directly to `Revoked`.
Only `FinalizeAssignmentRevoked` installs that terminal. Only
after proof-backed sink cancellation and controller-capacity release does the
non-revocation path install an append-only terminal state with no outgoing
transition. Every transition uses the assignment generation in a
compare-and-swap, so completion, revocation, expiry, exhaustion, use
settlement, restart reconciliation, and a concurrent read have one winner and
no last-writer-wins overwrite.

Resolution authenticates and binds the handle but does not consume a use.
`ReadImplementerIssueView` is a least-authority stateful read. A successful
distinct attempt reserves one available use by compare-and-swap. The
reservation is a quarantined authority effect committed with the terminal
journal, replay result, and outbox; section 13 activates the use atomically
with the other authority effects in the final transaction after audit
acknowledgement persistence. A non-final use returns the assignment to
unreserved `Active`. Final-use activation instead atomically enters
`RevocationCapacityReleasePending { Exhaust }`; it never installs
`Exhausted` directly. A byte-identical retry replays the original attempt and
never reserves or consumes again. A definite-no-append conversion releases
the quarantined reservation in the same replacement transaction. A concurrent
fresh read that finds all remaining uses reserved waits on the owning
attempt's terminal transition; it then either acquires a released use or
receives the exact pending sink-cancellation, controller-release, or
terminal-install action. Only after those proof-backed actions complete can a
later read observe `Exhausted` and its successor eligibility. No uid equality,
local file, environment value, run name, caller-provided issue set, or
self-asserted assignment claim is evidence.

The issue-reader endpoint is least-authority:

- `ResolveImplementationAssignment` consumes the opaque handle plus
  authenticated implementer peer evidence and returns only a safe assignment
  summary. It never returns the trusted-dispatch or resolver mapping.
- `ReadImplementerIssueView` requires the resolved current assignment and
  returns the complete ledger for `PrimaryBatch`, or only the exact assigned
  issue projection for `ParallelFixSlice`, including the protected
  descriptions, evidence, recommendations, and disposition obligations needed
  for that candidate. It cannot enumerate another assignment, widen a slice,
  obtain authority or identity mappings, or mutate ledger state.
- `ReadMergeAuthorityMajorIssueView` requires a current
  `MergeAuthorityResolver` result and returns only the requested effective
  MAJOR issue, its protected rationale, evidence, validation references,
  mapping version, and existing acceptance state for the exact candidate. It
  cannot inspect unrelated issues or perform acceptance.

Both reads refuse a lifecycle, candidate, mapping-version, assignment, or
authority binding mismatch as `issue-view-binding-mismatch`. The merge reader
uses that refusal for an issue outside its resolved authority. The implementer
reader instead uses `implementation-assignment-cross-scope` for caller-supplied
issue ids outside its otherwise current active assignment.
Assignment self-assertion, capability replay by a different authenticated
peer or run, each terminal assignment state, and a request outside the
presented assignment's exact issue set are disjoint typed refusals in section
14. A cross-scope refusal carries only the presented non-capability alias and
the requested issue ids already supplied by the caller. It never looks up or
reveals a foreign owning assignment or capability handle.
Public, generic status, log and audit views retain the redacted projections in
section 13.

### 1. Lifecycle, lineage, scope, and candidate identity are controller-owned

A panel lifecycle is identified by a controller-issued `ReviewLifecycleId`.
It belongs to one controller-issued `CandidateLineageId`, one
`DeclaredScopeDigest`, and a sequence of immutable `CandidateContentId`
snapshots. None of those values may be asserted by an implementation agent,
reviewer, integrator, or free-form operator input.

The declared scope binds the approved deliverable, base and target identities,
and the bounded change surface. A candidate snapshot binds the exact content
and evidence under review. Content changes create a new snapshot inside the
same lifecycle; they do not create a second discovery phase.

A lifecycle ends in exactly one terminal outcome:

- `signed_off`;
- `abandoned`; or
- `superseded`.

Terminal state is append-only. Reusing a terminal lifecycle id or attaching a
new snapshot to it is `terminal-lifecycle-reused`.

For a native current-schema candidate the lifecycle is:

```
implementation
-> implementation self-review
-> one discovery panel
-> automatic issue-ledger synthesis
-> batch implementation
-> implementation self-verification
-> constrained verification panel
-> batch fix and verification only for blocking failures
-> unanimous sign-off
```

The discovery panel runs exactly once; a second admission is
`discovery-already-admitted`. A zero-finding discovery still proceeds through
self-verification and verification; it skips a no-op batch implementation.

### 2. Native discovery is comprehensive, parallel, and exhaustive

The controller selects the discovery roster under ADR 0053 D21 as narrowly
superseded by the pool and version 2 table above. Every selected reviewer
receives the full candidate, immutable staged evidence and digests, applicable
validation evidence, its controller-bound profile, and read-only repository
context.

Every discovery prompt MUST state all of the following:

- this is the lifecycle's one comprehensive discovery review;
- review the entire candidate, not only the seat's most obvious files;
- inspect repository context needed to test local invariants;
- work exhaustively rather than stopping after the first findings; and
- report every actionable finding the reviewer can reasonably identify.

An actionable finding is grounded in a violated requirement, repository rule,
correctness property, or concrete maintainability defect. An unsupported style
preference is not made actionable by labeling it NIT.

There is no lifecycle-wide finding cap. A bounded record MAY use
content-addressed pages, but its manifest must prove complete ordered coverage.
Truncation, sampling, or instructing a reviewer to stop at a count is refused.
The controller refuses a missing, duplicated, out-of-order, truncated or
otherwise incomplete page set as `discovery-page-incomplete`. A native finding
missing any required closed field, carrying an unknown severity, exceeding a
bound, or disagreeing with its dispatch and candidate identity is
`malformed-native-finding`; it is never silently dropped from an otherwise
admitted page.

### 3. Raw findings and severity are closed, immutable evidence

Every native raw finding has exactly one severity:

- `BLOCKER`: merging can cause an unsafe or invalid result, including a
  security-boundary violation, data loss, required-contract failure, or a
  correctness or reliability failure for which no authority may accept risk.
- `MAJOR`: a material correctness, security, reliability, product-contract,
  migration, or test-coverage defect that must be fixed unless the protected
  merge authority explicitly accepts it.
- `MINOR`: a real, bounded defect whose remaining impact does not make the
  candidate unsafe to merge.
- `NIT`: a concrete, actionable local-quality defect with negligible behavior
  or risk impact. Personal taste and optional redesign are not findings.

A native raw finding carries the reporting seat, impact, concrete
recommendation, location or evidence, candidate binding, output digest, and
recommendation ordinal. Missing impact or recommendation is malformed, not a
reason to downgrade.

Raw findings are immutable. A correction appends an event and never changes
the original bytes, severity, seat, or recommendation.

Severity state is source-owned rather than issue-owned. A native source begins
with its raw severity. A legacy source begins with the migration-assigned
source triage in section 12. The effective severity of an issue is the highest
current severity of the sources mapped to it at the current mapping version.

A `SeverityCorrection` targets exactly one native `SourceId` or
`LegacySourceId`. For a native source, only its reporting seat may submit the
candidate-bound correction through trusted dispatch. For a legacy source whose
reporting seat was retired, section 12's versioned accountability successor
submits it. At least one final-roster seat that neither reported that source
nor implemented the candidate must submit
`SubmitSeverityCorrectionVerification`. The controller admits the correction
only when both records bind the same source, candidate and proposed severity.
The integrator, orchestrator, operator, and controller cannot originate a
correction or lower severity by deduplication. A dissenting or missing
higher-severity source leaves the higher severity effective.

The authorization predicates are disjoint. The generic
`severity-correction-unauthorized` predicate accepts and reports only a native
`SourceId`. Every `LegacySourceId`, whether its historical role is current or
retired, is evaluated only by
`legacy-source-severity-correction-unauthorized`. A source cannot be
reclassified between those predicates to obtain the other remedy. Missing or
stale independent verification is partitioned by the same identifier type:
`severity-correction-unverified` accepts only a native `SourceId`, while
`legacy-source-severity-correction-unverified` accepts only a
`LegacySourceId`.

Closing a finding as invalid or withdrawn does not rewrite or downgrade its
historical severity. A content change makes a prior severity-correction
verification stale; the source's preceding current severity is effective
again until the correction is independently verified against the new
candidate. A split or merge replays source severity and correction events
without repeating source triage because source identity is unchanged.

### 4. The orchestrator synthesizes one stable issue ledger automatically

The orchestrating agent, not the operator, automatically assigns the next
stable identifiers `R1`, `R2`, and so on and synthesizes bounded issue
descriptions from the raw findings. The operator never copies recommendations,
chooses ids, or constructs a crosswalk.

The orchestrator calls `SubmitLedgerSynthesisProposal` with its assigned
stable `R` ids, bounded descriptions, grouping, source mapping, proposal
idempotency key, and the base ledger and source-set digests. The controller,
not the orchestrator, admits the result. It validates:

- every raw source maps to exactly one effective issue;
- every issued `R` id is unique, monotonic, never reused, and never
  renumbered;
- every issue description is present and bounded;
- every source recommendation remains reachable from the issue;
- duplicate attribution is complete;
- the ledger, source records, scope, lineage, and candidate bindings agree;
  and
- the synthesis was produced for the latest admitted source set.

Issue descriptions and recommendation text are protected fields. The ledger
stores their bounded redacting types and digests; it does not place them in
public output or generic `Debug` rendering.

The first admitted synthesis fixes the source-to-id mapping. The orchestrator
still assigns the proposed ids; admission does not replace, renumber or invent
them. An identical retry with the same key and request bytes returns the
original admitted artifact and digest. The same key with different bytes is
`protected-operation-replay-conflict`. A fresh key carrying different proposed
bytes for an already admitted base generation is
`ledger-synthesis-conflict`; neither path silently replaces the ledger.

Each issue carries, directly or by digest-bound reference:

- stable issue id and effective severity;
- protected description, impact, recommendation, and location or evidence;
- every raw source and reporting reviewer;
- implementation disposition and justification;
- all verification judgments and the derived adjudication;
- validation evidence references;
- any severity-correction, risk-acceptance, or dedup-correction events; and
- the ledger version and exact candidate binding.

#### Deduplication corrections

Deduplication is a fallible judgment, so its correction is append-only:

- `SplitIssue` leaves the oldest issue id with a declared primary source
  subset and assigns new, next-monotonic ids to the separated subsets.
- `MergeIssues` keeps the oldest id as the effective id. Every other id remains
  a permanent resolvable alias and is never reassigned.

The orchestrator endpoint cannot request either event. Every reporting
reviewer whose source mapping would change, or its versioned accountability
successor for a retired legacy seat, first submits
`SubmitLedgerMappingConcurrence` through trusted dispatch. Each concurrence
binds the candidate, current mapping version, complete proposed mapping digest,
and that reviewer's affected source ids. The protected operator then invokes
`ApplyLedgerMappingCorrection` on the operator endpoint. The controller
validates protected operator authorization, complete candidate-bound
concurrence from every affected reporting reviewer, exact source coverage,
candidate binding, monotonic id allocation, and idempotency before appending
the event. Missing concurrence is `ledger-mapping-concurrence-missing`, an affected
reporter's explicit dissent is `ledger-correction-reporter-dissent`, and stale
concurrence is `ledger-mapping-concurrence-stale`. A proposed event whose
source partition, alias, monotonic-id, or exact-coverage structure is invalid
is `ledger-correction-structurally-invalid`. Repeating the identical correction
returns the existing event; a conflicting replay is
`protected-operation-replay-conflict`.

The current effective mapping is derived by replaying mapping events. A source
maps to exactly one effective issue after every event. A correction invalidates
only dependent issue-level verification and acceptance state whose subject set
changed: verification and adjudication judgments over the old grouping, and
risk or lifecycle approval state that named the old mapping. Those items must
be re-established against the corrected mapping and current candidate. Raw
findings, legacy source triage, source-level severity corrections and
implementation-disposition history replay unchanged. A split projects its existing disposition onto each resulting source subset.
A merge is admitted only when the source issues have the same current
disposition and candidate-evidence digest; otherwise it is
`ledger-correction-dispositions-incompatible` until implementation submits
compatible dispositions before the protected correction.

Terminal metrics count effective issue classes at the terminal ledger version.
A split can increase and a merge can decrease the unique issue count; aliases
never count as additional issues. A fixed issue contributes once only if its
effective terminal issue reaches verified `Fixed` after the last correction.
Metric records bind the mapping version so a historical count is never
reinterpreted.

### 5. Implementation dispositions do not adjudicate reviewer truth

The first implementation pass after discovery is one batch over the complete
ledger. Its controller-issued `PrimaryBatch` assignment may expose that
complete ledger. Parallel fix slices remain allowed when file ownership is
disjoint, but each receives only its controller-issued `ParallelFixSlice`
projection. A slice cannot read the rest of the ledger or another slice's
issues. The disjoint slices integrate into one candidate before verification.

Before verification, every issue has exactly one closed implementation
disposition:

- `Fixed`, with a candidate-bound delta or commit reference;
- `NoChangeClaimed`, with reason `incorrect` or `inapplicable` and a concrete
  protected explanation; or
- `Deferred`, with a protected explanation and durable follow-up reference.

These values state what implementation did. They do not decide whether the
finding was right, whether a fix works, or whether the candidate may merge.
In particular, `NoChangeClaimed` is not an invalid-finding adjudication,
`Deferred` is not risk acceptance, and neither value changes severity.

Verification judgments are separately closed:

- `resolved`: the defect was applicable and is fixed in the bound candidate;
- `invalid`: the asserted defect is factually wrong or inapplicable;
- `withdrawn`: the reporting seat withdraws its own recommendation through
  trusted dispatch;
- `unresolved`: the issue remains applicable and unresolved.

Only final-roster panel seats may author those judgments. Implementation
self-review, the integrator, orchestrator, controller, operator, and merge
authority are not panel adjudicators.

The controller derives one issue adjudication:

- `verified_resolved`;
- `verified_invalid`;
- `verified_withdrawn`; or
- `open`.

`verified_invalid` requires two agreeing final-roster seats that did not
implement the candidate. At least one must be a non-reporting seat when one
exists. If a reporting seat judges it unresolved, the controller issues a
separate adjudication obligation to two non-dissenting final-roster seats,
preferring non-reporting seats. A seat that also supplied a duplicate source
may satisfy that dedicated obligation only when the roster has too few
non-reporting seats; its new candidate-bound adjudication is recorded
separately from its raw finding. This fallback keeps an issue reported by the
whole roster adjudicable without adding an off-roster authority.
`verified_withdrawn` requires a candidate-bound withdrawal from every
reporting seat whose source remains on the effective issue and a separately
recorded independent final-roster verification. `verified_resolved` requires
the reporting seats and an independent panel verifier to accept the fix; if a
reporting seat dissents, the same two-seat dedicated adjudication rule may
independently establish resolution.

All disagreement remains in the ledger. Until one rule above is satisfied the
adjudication is `open`. Once an invalid or withdrawn adjudication satisfies its
rule, a historical BLOCKER or MAJOR is clear without severity downgrade or
risk acceptance. A reviewer may block on evidence that the adjudication rule
or candidate binding was violated, but not merely by restating the already
adjudicated raw recommendation.

`verified_invalid` and `verified_withdrawn` close the effective issue
regardless of whether its historical implementation disposition was `Fixed`,
`NoChangeClaimed`, or `Deferred`. The controller retains that disposition and
its justification unchanged and appends the derived state
`disposition_superseded_by_adjudication`. No content change is required merely
to make a historical disposition agree with the later adjudication.

Every adjudication and derived supersession is candidate-bound. A later
candidate snapshot makes it stale when admitted, retains it as history, and
returns the current issue to `open` until the required seats re-adjudicate the
new candidate. Re-establishing `verified_invalid` or `verified_withdrawn`
again requires no content change. If the new candidate makes the source
applicable, the ordinary disposition, fix and verification rules apply.

### 6. Verification coverage is total and independent

Implementation self-verifies the integrated candidate before the first
verification panel and after every later blocking batch fix. It records
every selected command and result for supported tests, lint, formatting,
static analysis, and builds, plus every category found inapplicable and the
concrete reason. It then self-reviews the latest delta and full candidate. It
cannot mark a required repository gate inapplicable because the gate is
expensive.

Every issue and every implementation disposition receives panel verification:

1. every original panel reporting seat that remains dispatchable submits a
   candidate-bound judgment for every issue carrying one of its sources;
2. at least one final-roster seat that did not implement the candidate
   verifies the disposition and evidence for every issue;
3. a finding originating from the reserved
   `implementation-self-review` source receives at least one final-roster
   panel judgment, because self-review is not panel review; and
4. invalid, withdrawn, resolved-with-dissent, severity-correction, and risk
   acceptance cases satisfy their additional independent coverage rules.

An original reporting seat remains accountable even when its source is a
duplicate. Deduplication never releases its judgment obligation. A retired
legacy seat follows section 12's explicit accountability-successor rule; its
source attribution is never relabeled. Missing, duplicate, stale, or
contradictory coverage blocks approval.

Disposition and adjudication combine as follows:

| Historical disposition | Current adjudication | Result |
| --- | --- | --- |
| `Fixed` | `verified_resolved` | closed as resolved |
| `Fixed`, `NoChangeClaimed`, or `Deferred` | `verified_invalid` or `verified_withdrawn` | closed; append `disposition_superseded_by_adjudication` |
| `Fixed` | `open` | issue stays open |
| `NoChangeClaimed` | anything else | issue stays open |
| `Deferred` | `open` | disposition coverage is complete; severity rules decide approval |
| any | stale adjudication after candidate change | issue returns to `open` pending candidate-bound re-verification |

An independently verified unresolved MINOR or NIT has complete verification
coverage even though it remains open. Verification completeness and issue
resolution are deliberately different facts.

### 7. Verification artifacts are generated, complete, and idempotent

The protected authority automatically generates every per-seat verification
artifact from admitted inputs.
No operator, integrator, or orchestrator copies findings into reviewer notes.
There is no hand-authored reviewer-notes migration surface.

Every seat dispatched after an admitted discovery or verification round
automatically receives:

- the full prior ledger, or a bounded content-addressed manifest whose chunks
  are collectively complete;
- every issue id and protected issue description from the last complete
  round;
- prior recommendation sources and reporting seats;
- implementation dispositions and justifications;
- verification judgments and current adjudications;
- applicable validation evidence and its enforcement class;
- the latest delta and full candidate context; and
- a seat-specific obligation view naming exactly what that seat must judge.

The bounded representation may page or chunk but may not summarize away an
issue, source, description, disposition, judgment, or evidence reference.
Manifest coverage, chunk order, and digests are validated before dispatch.

Artifact identity is a total function of schema version, lifecycle, lineage,
candidate content, ledger version, verification ordinal, and seat. Retrying
generation with the same inputs returns byte-identical artifacts. Different
bytes at the same identity are refused as
`verification-artifact-identity-conflict`. A retry of one seat neither
duplicates an admitted judgment nor changes another seat's obligations.

Dispatch accepts only the authority-generated artifact whose digest appears in
the admitted roster manifest. A caller-supplied file, edited reviewer note,
substituted seat bundle, or digest-equivalent wrapper is
`manual-per-seat-artifact-substitution`, even when its visible text matches.
The only protected issue-text reads outside a generated reviewer artifact are
the two least-authority issue-reader operations defined above.

Reviewers remain read-only and cannot attest their own authored work. Each
seat's provider, model, effort, prompt digest, and reviewer identity are pinned
from first dispatch through lifecycle completion. Candidate content, prompts,
ledger versions, dispositions, validation evidence, risk records, reviewer
outputs, and the final receipt are digest-bound.

### 8. Late findings are a closed exception, including unsafe untouched code

Verification is resolution and regression review, not reopened discovery. A
new finding is admitted only under one of these closed reasons:

- `introduced_by_fix`: an implementation or later fix introduced it;
- `missed_blocker_or_major`: it existed at discovery and is now assessed as
  BLOCKER or MAJOR; or
- `unsafe_to_approve`: a correctness, security, data-loss, or reliability risk
  makes approval unsafe.

The untouched-code exclusion applies only to an ordinary pre-existing MINOR or
NIT. `missed_blocker_or_major` and `unsafe_to_approve` override touched status:
an unsafe finding is admitted even when its code was untouched and outside a
reviewer's usual seat focus.

A pre-existing MINOR or NIT, style preference, optional refactor, naming
taste, or merely desirable documentation enhancement is filed outside the
lifecycle and cannot delay approval. A reviewer cannot evade the closed
reason by relabeling an old MINOR without evidence for the higher severity.

Every admitted late source receives the next stable id or maps as a duplicate
to an existing issue. The late record carries its allowed reason, reviewer or
reserved `implementation-self-review` source, verification ordinal, and a
protected explanation of the discovery miss. It then receives the same
disposition, coverage, correction, and adjudication treatment as every other
issue.

### 9. Post-discovery change is ledger-scoped; rescope preserves lineage

After discovery, every content change must be mapped to one or more ledger
issues and may only implement, validate, or correct those issues. A regression
or self-review defect is entered as an allowed late issue before its fix is
admitted. An unrelated cleanup, feature, hardening change, or scope expansion
is refused even if it is useful. An unmapped change is
`post-discovery-change-unmapped`; a genuine wider scope is
`post-discovery-scope-expansion`.

`SubmitCandidateSnapshot` therefore carries a complete changed-region to
effective-issue map against the current ledger version. A changed region with
no mapping is `post-discovery-change-unmapped`; mapping it only to an alias,
stale ledger version, or unrelated issue does not satisfy the operation.

A genuine scope change uses the controller's protected `RescopeLifecycle`
operation. It terminates the source lifecycle as `superseded`, creates one
successor with a larger or different declared scope, atomically imports all
raw findings and every unresolved effective issue, and records a stable
old-id-to-successor-id crosswalk. The successor is a new current-schema
lifecycle and runs its own one comprehensive discovery panel. Imported
findings are prior obligations, not a substitute for that discovery.

`AbandonLifecycle` terminates without deleting findings. In the same atomic
transition the controller creates one `SuccessorImportCapsule`, bounded to
1 MiB of canonical serialized protected state per abandoned lineage. It
contains only what an atomic resume needs without the full round bytes:
source ids and protected source-view payloads, current source severities and
correction events, effective issue ids and protected description,
recommendation and evidence payloads, current mapping and crosswalk versions,
and unresolved disposition and verification obligation ids. A
content-addressed payload may be stored separately only when its bytes count
toward the same 1 MiB transitive bound and its lifetime is pinned to the
capsule. It contains no prompt, diff, validation-output bytes, reviewer output
bytes or public identity mapping.

The capsule is required state, not an audit convenience. From abandonment
until resume, rescope or permanent close, it has no age expiry and is
ineligible for D17's size eviction because the lineage remains resumable. If
the bounded capsule cannot represent the lineage completely,
`AbandonLifecycle` refuses as `successor-import-capsule-over-bound` and leaves
the lifecycle active.

A later resume is a new successor, never mutation of the abandoned lifecycle.
`ResumeLifecycle` and repeated `RescopeLifecycle` calls derive successor
identity from the source lifecycle and protected operation id, so retry
returns the same successor. Resume consumes the capsule atomically with source
import and crosswalk creation; a crash cannot expose a successor without the
complete import.

`PermanentlyCloseAbandonedLineage` applies only to an abandoned lineage; any
other state is `permanent-close-ineligible`. It atomically marks the lineage
nonresumable, forbids any same-lineage or same-candidate restart, retains
audit-floor digests and closed projections, and makes the capsule eligible for
ordinary round-input eviction. Repeating
the same operation returns the original close event; a conflicting retry is
`protected-operation-replay-conflict` and an attempted reuse is
`permanent-closed-lineage-reuse`. Retention-capacity recovery may resume a
named lineage, rescope it into a named successor, or permanently close it.
Ordinary abandonment does not free the capsule and is never presented as a
capacity remedy. Raising a bound is not a blocker remedy; section 13 permits
the single reviewed `MigrateRetentionCapacity` escape only for detected
recovery-reserve corruption or a versioned bound migration.

Successor creation, source import, crosswalk publication, and source
termination are one atomic transition. If complete import cannot commit, no
successor becomes usable and the source lineage remains terminal or parked.
There is no state in which a successor exists without all raw findings and
unresolved items. Abandonment, rescope, retry, or deduplication therefore
cannot erase an awkward finding.

### 10. MAJOR risk acceptance is a separate protected authority operation

A BLOCKER cannot be risk accepted. A MAJOR may remain open only under a valid
`MajorRiskAcceptance`.

`IssueRiskOperationIntent` first returns the controller-issued idempotency key
for exactly one proposed acceptance or revocation digest. `AcceptMajorRisk` is
a distinct typed protected operation. It is not an
`approve` decision, cannot close a gate, and cannot be reached from ADR 0053's
orchestrator or publisher endpoints. It extends only the protected operator
endpoint. `RevokeMajorRiskAcceptance` is a second distinct operation on that
same endpoint.

The controller also issues an opaque `RiskOperationHandle` for each admitted
risk-operation intent. The handle is the bound operation key or resolves that
key through controller authority state; it is never a caller-selected key.
It binds the candidate, intent kind, proposed mutation digest, and acceptance
id where applicable. Only an authenticated `ProtectedOperator` may receive or
present it. The raw handle is carried only in the immediate protected intent
response, a pending context returned by operator-authenticated
`RecoveryRead.ReadRiskRecoveryState`, and the corresponding protected mutation
request. It is absent from generic `ProtectedAttemptRecovery`,
`ReadProtectedAttemptStatus`, and every response available under original-peer
authentication alone. Logs, audit events, metrics, tombstones, refusal
products, derived or handwritten `Debug`, and public artifacts contain at
most a domain-separated non-capability handle alias.

The controller issues the idempotency key and handle for each risk-operation
intent before it accepts the mutation bytes. For each risk operation, the same
key and byte-identical request returns the original event and response while
full result bytes remain. If the intent response is lost or its replay payload
is evicted, an original peer sees only the exact handle-free pending
`OriginalPeerRiskRecovery` variant and its
`RecoveryRead.ReadRiskRecoveryState` action requiring fresh protected-operator
authentication.
Operator-authenticated `ReadRiskRecoveryState` then idempotently reissues the
same `RiskOperationHandle` from the still-pending authority state; it never
issues a replacement handle or a new intent. Neither path re-executes. The
safe and handle-bearing schemas are caller-disjoint and strict; one cannot be
decoded as the other.

Risk recovery is caller-disjoint and exactly tagged. The original-peer outer
type has one wire variant per exact safe state and action:

```
OriginalPeerRiskRecovery =
  AcceptanceIntentPending {
    intent_id,
    candidate_id,
    proposed_mutation_digest,
    risk_operation_handle_alias,
    next: RecoveryRead.ReadRiskRecoveryState {
      caller: ProtectedOperator,
      authentication: FreshForThisRead
    }
  }
  | RevocationIntentPending {
      intent_id,
      candidate_id,
      acceptance_id,
      proposed_mutation_digest,
      risk_operation_handle_alias,
      next: RecoveryRead.ReadRiskRecoveryState {
        caller: ProtectedOperator,
        authentication: FreshForThisRead
      }
    }
  | AcceptanceLive {
      acceptance_id,
      candidate_id,
      issue_ids,
      expires_at,
      acceptance_event_id,
      next: NoFurtherAction
    }
  | RevocationEffective {
      acceptance_id,
      candidate_id,
      revocation_id,
      revocation_event_id,
      next: NoFurtherAction
    }
  | ClosedMutationPermitted {
      candidate_id,
      prior_object_alias,
      closed_reason_code,
      next: RecoveryRead.ReadRiskRecoveryState {
        caller: ProtectedOperator,
        authentication: FreshForThisRead
      }
    }
  | ClosedMutationForbidden {
      candidate_id,
      prior_object_alias,
      closed_reason_code,
      next: NoFurtherAction
    }

ProtectedAttemptRecovery::Success::Operator::
  RiskOperationIntent<Outcome, OriginalPeerRiskRecovery>

ProtectedAttemptRecovery::Success::Operator::
  NewRiskOperationIntent<Outcome, OriginalPeerRiskRecovery>

ProtectedAttemptRecovery::Success::Operator::
  MajorRiskAcceptance<Outcome> {
    acceptance_id, candidate_id, issue_ids, acceptance_event_id,
    next: NoFurtherAction
  }

ProtectedAttemptRecovery::Success::Operator::
  MajorRiskRevocation<Outcome> {
    acceptance_id, revocation_id, revocation_event_id,
    next: NoFurtherAction
  }
```

The immediate authenticated success response from
`IssueRiskOperationIntent` or `RequestNewRiskOperationIntent` is a distinct
`ProtectedRiskOperationIntentResponse` containing the newly issued
`RiskOperationHandle` and only its bound acceptance or revocation mutation.
It is not `ProtectedOperatorRiskRecoveryContext`. The
`ProtectedAttemptRecovery` variants above are the later handle-free generic
recovery products; they never substitute for that immediate protected
response. `ProtectedOperatorRiskRecoveryContext` exists only as the immediate
result of a freshly authenticated `ReadRiskRecoveryState`.

`<Outcome>` is expanded by the generator into a distinct operation-and-outcome
wire tag; it is not a serialized state field. A risk refusal is the
corresponding
`ProtectedAttemptRecovery::OriginalRefusal::Operator::<Operation,Refusal>`
variant with the catalog's exact safe refusal product and a reachable
`RecoveryRead.ReadOriginalRefusalRecoveryState` next operation. Its risk
portion is likewise one exact `OriginalPeerRiskRecovery` outer tag, not a
generic risk-safe-state field plus an independently chosen action. The recovery
projection never substitutes a digest-only state envelope and never carries a
raw `RiskOperationHandle`.

Generic attempt recovery and `ReadProtectedAttemptStatus` generate only the
six exact `OriginalPeerRiskRecovery` variants above. Each variant owns its
handle-free safe fields and its state-valid action directly; no broad
risk-safe-state envelope or independently selected action is a generator
input. Pending and closed-permitted variants name
`RecoveryRead.ReadRiskRecoveryState` by a freshly authenticated protected
operator. Live, effective-revocation, and closed-forbidden variants own
`NoFurtherAction`. Generic original-peer recovery never owns or serializes a
`RiskOperationHandle`.

The immediate intent response is closed and distinct:

```
ProtectedRiskOperationIntentResponse =
  AcceptanceIntentIssued {
    intent_id,
    candidate_id,
    proposed_mutation_digest,
    risk_operation_handle: RiskOperationHandle,
    next: Operator.AcceptMajorRisk {
      caller: ProtectedOperator,
      handle: ThisRiskOperationHandle
    }
  }
  | RevocationIntentIssued {
      intent_id,
      candidate_id,
      acceptance_id,
      proposed_mutation_digest,
      risk_operation_handle: RiskOperationHandle,
      next: Operator.RevokeMajorRiskAcceptance {
        caller: ProtectedOperator,
        handle: ThisRiskOperationHandle
      }
    }
```

Only `RecoveryRead.ReadRiskRecoveryState` on the caller-disjoint
`ProtectedOperator` row, after fresh authentication for that read, returns this
handle-bearing tagged recovery context:

```
ProtectedOperatorRiskRecoveryContext =
  AcceptanceIntentPending {
    intent_id,
    candidate_id,
    proposed_mutation_digest,
    risk_operation_handle: RiskOperationHandle,
    next: Operator.AcceptMajorRisk {
      caller: ProtectedOperator,
      handle: ThisRiskOperationHandle
    }
  }
  | RevocationIntentPending {
      intent_id,
      candidate_id,
      acceptance_id,
      proposed_mutation_digest,
      risk_operation_handle: RiskOperationHandle,
      next: Operator.RevokeMajorRiskAcceptance {
        caller: ProtectedOperator,
        handle: ThisRiskOperationHandle
      }
    }
  | AcceptanceLive {
      acceptance_id,
      candidate_id,
      issue_ids,
      expires_at,
      acceptance_event_id,
      next: NoFurtherAction
    }
  | RevocationEffective {
      acceptance_id,
      candidate_id,
      revocation_id,
      revocation_event_id,
      next: NoFurtherAction
    }
  | ClosedMutationPermitted {
      candidate_id,
      prior_object_alias,
      closed_reason_code,
      next: Operator.RequestNewRiskOperationIntent by ProtectedOperator
    }
  | ClosedMutationForbidden {
      candidate_id,
      prior_object_alias,
      closed_reason_code,
      next: NoFurtherAction
    }
```

Each outer recovery tag owns its exact safe variant and action, and each protected
operator context tag owns its fields, handle, and action. State, handle, and
action are not independently selectable. Only `ClosedMutationPermitted`,
after the controller
proves that no pending intent, live acceptance, or already-effective
revocation forbids another intent, can render
`RequestNewRiskOperationIntent`. `AcceptanceIntentPending` and
`RevocationIntentPending` reissue only their same handle and render only their
exact `AcceptMajorRisk` or `RevokeMajorRiskAcceptance` mutation. A handle for
one pending variant cannot construct the other operation. Live acceptance and
effective revocation render their current state and `NoFurtherAction`;
another operation must first establish a closed mutation-permitted state.
`RequestNewRiskOperationIntent` is not a free-form key request: it accepts the
prior safe object alias and exact proposed mutation digest, rechecks the
closed state, and then executes the same controller-issued-key machinery as
`IssueRiskOperationIntent`. The same key with different request bytes is
`risk-operation-replay-conflict`. That conflict first returns the exact
handle-free `OriginalPeerRiskRecovery` variant to an original peer; only a
freshly authenticated protected operator may invoke `ReadRiskRecoveryState`
and follow the handle-bearing action owned by the returned variant. Lost or
evicted intent responses recover the same handle. Closed states return only `NoFurtherAction` or their exact
state-valid new-intent action. A lost response or crash after durable
admission therefore cannot create a second live acceptance or a second
revocation.
An original peer that invokes the risk read without fresh protected-operator
authentication receives `risk-recovery-operator-authentication-required` with
the exact safe-state-only `OriginalPeerRiskRecovery` variant. Authentication
failure therefore returns `NoFurtherAction` for live, effective-revocation,
and closed-forbidden state. Pending or closed-permitted state instead returns
its exact `OriginalPeerRiskRecovery` variant naming
`ReadRiskRecoveryState` by `ProtectedOperator` with
`FreshForThisRead`.
Authentication failure cannot reveal whether a raw handle is currently
recoverable. Strict generation rejects every cross-action encoding, including
a mutation action inside an `OriginalPeerRiskRecovery` variant, an original-peer variant with
a handle, an acceptance handle paired with revocation, a pending state paired
with new-intent creation, and any live or forbidden state paired with a
mutation.

The accepting identity is resolved as current merge authority for the
protected target from trusted peer evidence and an authoritative
`MergeAuthorityResolver`. A typed name, uid equality with an agent session,
environment value, local file, or producer assertion is not authority.

The two admitted evidence forms are:

- `ControllerPeerMergeAuthority`, resolved from the authenticated operator
  peer by the controller-owned resolver; and
- `StandaloneProtectedMergeAuthorityReceipt`, an opaque receipt issued by a
  protected identity separate from the standalone agent or contributor uid
  and resolved by an authoritative resolver.

There is no same-uid standalone fallback. Without a supported protected
resolver, standalone work must fix the MAJOR or configure the protected
authority path before acceptance is available.

Risk-operation recovery is fixed at the contract level:

| Typed refusal | Ordered `RemedyAction` plan |
| --- | --- |
| `major-risk-resolver-missing` | `ConfigureProtectedMergeAuthorityResolver`, then `RetryMajorRiskOperation` |
| `major-risk-peer-unauthorized` | `RequestResolvedMergeAuthority`, then `RetryMajorRiskOperation` |
| `major-risk-same-uid-standalone` | `ConfigureProtectedMergeAuthorityResolver`, then `RetryMajorRiskOperation` |
| `major-risk-acceptance-missing` | `RequestNewCandidateBoundRiskAcceptance` |
| `major-risk-candidate-mismatch` | `RequestNewCandidateBoundRiskAcceptance` |
| `major-risk-expired` | `RequestNewCandidateBoundRiskAcceptance` |
| `major-risk-revoked` | `ReturnToScopedBatchFix` |
| `major-risk-ledger-mapping-stale` | `ReverifyCorrectedIssue`, then `RequestNewCandidateBoundRiskAcceptance` |
| `risk-operation-replay-conflict` | original peer: return the exact handle-free `OriginalPeerRiskRecovery` state-and-action variant; freshly authenticated protected operator: `RecoveryRead.ReadRiskRecoveryState`, then only the exact handle and action owned by its immediate `ProtectedOperatorRiskRecoveryContext`; only pending or closed-permitted original-peer variants name that protected read, and only protected `ClosedMutationPermitted` may render `Operator.RequestNewRiskOperationIntent` |
| `major-risk-duplicate-live` | `Operator.RevokeMajorRiskAcceptance` by `ProtectedOperator` |
| `blocker-risk-acceptance-forbidden` | `ReturnToScopedBatchFix` |
| `nonblocking-risk-acceptance-unnecessary` | `ContinueWithDispositionAndVerification` |

The implementation generates producer-specific command text from those
actions and tests both renderings. It does not substitute a same-uid record,
severity downgrade, or generic "contact an administrator" message.

An acceptance binds:

- a stable acceptance id and authority alias or digest;
- lifecycle, lineage, declared scope, target branch, and exact
  `CandidateContentId`;
- the effective MAJOR issue ids and ledger mapping version;
- bounded protected rationale and durable follow-up reference;
- issue-description, evidence, and validation digests;
- issuance time and mandatory finite expiry; and
- the trusted resolver and peer-evidence digests.

Protected identity and rationale mappings stay nonpublic. Public review output
contains only safe aliases, closed states, expiry class, issue ids, and
digests.

Revocation is an append-only event. Its target is the logical acceptance
identity plus every prohibited duplicate the controller can prove has the same
authority, lifecycle, candidate, mapping version, issue set and request
digest. If recovery or a prior faulty implementation left more than one such
event live, one revocation invalidates the whole duplicate set; it never
selects one duplicate to remain effective. Legitimate distinct acceptances
with a different request digest are unaffected.

Validity means the acceptance is
candidate-exact, authority resolution still applies, the mapping version and
issue set still match, it is unexpired at the checking clock, and no revocation
event precedes that check. Validity is re-evaluated independently when the
issue verification receipt is admitted, when the lifecycle approval receipt
is created, at seal, at publication, and whenever merge eligibility is read.
An acceptance that expires or is revoked after verification therefore blocks
the later stage rather than being grandfathered.

Panel reviewers verify the acceptance's binding and current validity. They do
not accept the risk on the authority's behalf. A valid acceptance leaves the
issue adjudication `open` and records that this particular candidate may
proceed despite it; it does not rewrite the finding as resolved.

This section narrowly supersedes ADR 0053 D7 and D17 where their closed
operator operation set made this separate operation impossible. The endpoint
table above is the complete replacement; controller identity, append-only
authority, and publication approval remain unchanged.

### 11. Approval is merge-ready, unanimous, and sign-off-only

A candidate is approved only when:

1. every BLOCKER is `verified_resolved`, `verified_invalid`, or
   `verified_withdrawn`;
2. every open MAJOR has a currently valid risk acceptance;
3. every issue has one implementation disposition and complete required panel
   verification;
4. all required applicable enforcing validation passes;
5. applicable builds pass;
6. no admitted late issue that blocks under these rules remains untreated;
7. every artifact and acceptance binds the final candidate and current ledger
   mapping; and
8. every reviewer on the final lifecycle roster signs off.

An unresolved or unaccepted BLOCKER or MAJOR, incomplete coverage, failed
required validation, applicable build failure, stale binding, invalid
acceptance, or non-unanimity causes another scoped batch fix or refusal.

MINOR and NIT issues do not create endless verification cycles. Each still
requires a disposition and independent panel judgment once. They may remain
open under `Deferred`, or be invalid or withdrawn, without another content
change or verification execution. A MINOR or NIT introduced by a fix is
admitted and measured, but its severity does not become blocking merely
because its origin is a regression.

The record invariant remains exact:

```
PanelRecord.signoff == PanelRecord.recommendations.is_empty()
```

In final verification, `recommendations` contains only an unsatisfied
merge-blocking condition under this section, an allowed new finding, or a
contract failure in evidence or adjudication. A resolved issue, a verified
invalid or withdrawn issue, a validly accepted MAJOR, and a completely judged
nonblocking MINOR or NIT remain visible in the ledger but are not copied into
blocking recommendations.

Discovery output is evidence, not approval. Final verification remains
unanimous over the monotonic lifecycle roster selected under ADR 0053 D21 as
narrowly superseded by the pool and version 2 table above. Newly selected
specialists join verification; no discovery reviewer rotates out.

The controller mints a `PanelLifecycleApprovalReceipt` only for `signed_off`.
It binds the final candidate, scope and lineage, every roster and trusted
dispatch, all source records and ledger events, dispositions, judgments,
validation evidence, the `SignedOff` terminal metric payload, risk records,
and final per-seat records. Abandonment and supersession mint terminal metric
records but never an approval receipt. Green tests are evidence in the receipt
and never substitute for panel approval.

Every approval receipt has mandatory finite expiry. The versioned constant is
`APPROVAL_RECEIPT_MAX_AGE = 7 days`, and
`receipt.expires_at` is the earliest of issuance plus that constant and every
MAJOR risk-acceptance expiry on which the receipt depends. No configuration or
operator input may extend it beyond either cap.

An unexpired receipt is merge-eligible only while every ordinary invariant
continues to hold. `RecordTrustedMergeCompletion` accepts only an
authoritatively resolved provider event for the exact target and candidate.
Any other binding is `merge-completion-binding-mismatch`. Its admission makes
the terminal round inputs eligible for retention
immediately. Without that event, receipt expiry invalidates merge eligibility
as `approval-receipt-expired` and makes those inputs eligible. Eviction leaves
only audit-floor digests and closed projections.

An expired receipt cannot be renewed from its old sign-offs. The operator must
invoke `CreateReverificationSuccessor`, which creates a same-scope successor
for the exact candidate and requires fresh candidate-bound verification and a
new unanimous receipt. Retained issue inputs may be imported atomically. If
they were already evicted, the successor runs a fresh native discovery before
verification rather than reconstructing protected text from audit digests.
Before expiry the operation is `reverification-successor-ineligible`. Expiry
never turns an old receipt back into merge evidence.

The seal, publication gate, and merge-eligibility reader validate the
lifecycle receipt rather than an isolated final record set. This supersedes
ADR 0053 D8 and D9 only where they see an isolated final set and refuse
publication while any finding of any severity exists.

### 12. Cutover uses an automatic version-dispatched compatibility adapter

The first implementation bumps the delivery schema and declares a cutover
revision. A native lifecycle first created at or after cutover uses the current
schema and exactly one native discovery panel. Compatibility does not add a
second discovery to a native lifecycle.

The reader envelope version-dispatches before strict schema parsing:

- current artifacts use the current strict reader;
- each supported historical schema uses its own strict historical reader; and
- unknown versions fail with a typed version error and generated remedy.

Historical readers preserve and digest exact bytes. Their diagnostics and
renderers use schema-specific redacting projections, never raw arbitrary
strings or generic `Debug`.

#### Completed and in-flight legacy rounds

A compatibility import uses the latest complete legacy round for the active
candidate lineage:

- If a completed legacy round already exists and fixes are underway, the
  adapter ingests it immediately. Existing fix content is not discarded. A
  disposition may cite an already-produced candidate delta when immutable
  orchestration evidence maps it automatically. Otherwise the code remains
  intact and the generated ledger is sent through the ordinary implementation
  disposition step. The operator never supplies the source crosswalk.
- If a legacy dispatch is already in flight, every seat in that dispatch may
  finish that one complete round under the old schema. The adapter ingests it
  only after the whole roster is complete.
- A partial legacy round is never discovery evidence. Missing or invalid seats
  remain retry state for that same pinned old dispatch, and no new old-schema
  round may be started after cutover; an attempt is
  `legacy-round-start-after-cutover`.

Retrying missing seats does not mix schemas inside one round. If the pinned
old round is incomplete and every missing pinned reviewer remains
dispatchable, admission returns `legacy-round-partial-retryable`. Its only
linear remedy is to complete that pinned round. If protected dispatch
resolution proves at least one missing pinned reviewer unavailable, admission
instead returns `legacy-round-reviewer-unavailable`. Its only linear remedy is
`CreateSameScopeCurrentSchemaSuccessor`; retrying the unchanged old dispatch
is not offered. This is not `RescopeLifecycle`: declared scope and candidate
stay exact, and completed seats are not discarded or rerun merely to make the
old round complete.

While the round is `legacy-round-partial-retryable`, every source
partial-round byte is ineligible for cleanup. The bytes remain ineligible
after `legacy-round-reviewer-unavailable` until the same-scope successor
transition commits. That transition reads those exact bytes,
deterministically creates a `LegacySourceId` for every recommendation in every
well-formed completed-seat record, and imports the protected source views as
prior obligations into the successor's admitted prior-obligation source set.
It never labels the partial round discovery, never imports a malformed or
incomplete seat, and does not treat the unavailable reviewer as dispatchable.
The current-schema successor then runs exactly one fresh native discovery.
Only after that discovery do native findings and imported legacy obligations
enter the same proposed ledger synthesis without losing either source
identity.

The successor's initial roster is the union of:

- the normal version 2 native roster selected for its unchanged candidate; and
- every current-pool role that reported an imported completed-seat source, or
  that role's versioned accountability successor when the role is retired.

The union is controller-derived, de-duplicated, and monotonic. Thus an imported
`networking` or `kernel` source keeps that role on the successor even when
native selection would omit it. A fresh agent instance dispatched for a
current role may satisfy the reporting-role obligation when the old pinned run
is unavailable; continuity binds the role and immutable source attribution,
not the unavailable agent process. The controller binds the current versioned
role profile, reviewer identity, and trusted dispatch to every such
accountability seat. A retired role uses only the versioned accountability
successor table and never relabels the historical source.

Successor creation, completed-seat source import, old-to-new lifecycle and
issue crosswalks, source lifecycle termination, and the fresh-discovery
requirement are one atomic binding. Their stable logical identity is:

```
LogicalSuccessorImportId =
  digest(
    "d2b:panel:logical-successor-import:v1",
    source_lifecycle_id,
    pinned_legacy_dispatch_id,
    canonical_completed_seat_digest_set,
    CandidateContentId,
    declared_scope_digest,
    compatibility_schema_version
  )
```

The successor lifecycle id and crosswalk identity are independently
domain-separated derivations of `LogicalSuccessorImportId`. No protected
attempt id, idempotency key, worker epoch, reservation, or retry ordinal is an
input. The controller derives every field from immutable source state and
refuses a caller-supplied mismatch.

Each `CreateSameScopeCurrentSchemaSuccessor` protected attempt targets this
logical import. Byte-identical replay of one attempt returns that attempt's
original success or refusal. A terminal `successor-import-incomplete` attempt
therefore continues to replay its refusal, but a fresh protected attempt may
execute the same `LogicalSuccessorImportId`; success still creates the same
successor and crosswalk identity. The source lifecycle admits exactly one
logical tuple. A request that changes its pinned dispatch, completed-seat
digest set, candidate, declared scope, compatibility schema, proposed
successor, or proposed crosswalk is `same-scope-successor-conflict`, whether
the earlier attempt failed or succeeded. It returns the admitted logical
identity and, only when one exists, the admitted successor safe id. A crash
exposes neither a partial successor nor a partially imported source set. This
escape does not require a genuine scope change and does not erase the
completed seats.

At the same atomic commit, the source lifecycle becomes terminal
`superseded`. Its partial-round byte objects then become eligible immediately
for ordinary D17 round-input cleanup; they do not wait for the successor to
sign off. The successor no longer references those source objects. Everything
needed to continue is in its admitted protected prior-obligation source set,
then its ledger state after synthesis, and later in its
`SuccessorImportCapsule` if it is abandoned. Permanent audit-floor digests
bind the source dispatch, completed-seat set, imported source ids, successor,
and crosswalk. If the atomic import fails, the source remains
`UnavailablePartialDispatch`, its bytes remain ineligible, the unavailable
reviewer is not relabeled as dispatchable, and no successor is usable. Only a
fresh protected `CreateSameScopeCurrentSchemaSuccessor` attempt targeting that
same `LogicalSuccessorImportId` is retryable; completing or redispatching the
pinned legacy round is no longer a remedy. The accepted failed attempt is
`successor-import-incomplete`; its one linear remedy is
`RetryLogicalSuccessorImportWithFreshProtectedAttempt`. Replaying the failed
attempt still returns the original refusal.

If the successor terminates after import but before fresh discovery, after
fresh discovery but before ledger synthesis, or after ledger admission,
section 13 emits the corresponding top-level closed progress variant. Every
variant reachable by this successor carries its exact imported
`LegacySourceId` source count and exact admitted current and stale-or-missing
triage counts; termination cannot collapse partial progress to a generic
no-discovery value.

The imported complete round is the lifecycle's migration discovery input. It
does not claim to be a native current-schema discovery panel. A same-scope
successor from a partial legacy round has native discovery plus imported prior
obligations. Section 13's top-level terminal metric enum records these cases
without independent origin fields.

#### Legacy source identity, ids, descriptions, and severity

Legacy `PanelRecord.recommendations` are arbitrary strings with no id or
severity. For each recommendation the adapter creates:

```
LegacySourceId =
  digest(
    "d2b:panel:legacy-source:v1",
    immutable_record_digest,
    seat,
    recommendation_ordinal
  )
```

The ordinal is its zero-based position in the immutable legacy array. Exact
record bytes are retained under the retention rules below. Equal strings in
one or several seats remain distinct raw sources because their record digest,
seat, or ordinal differs.

The orchestrator automatically groups those sources, assigns new stable `R`
ids in deterministic group order, and synthesizes issue descriptions. For a
single-source group the legacy string is copied mechanically into the
protected source view; for a duplicate group every original string remains
available beside the synthesized description. No operator transcribes text or
constructs an old-to-new crosswalk.

Trusted tooling refuses admission until every source maps exactly once, every
description exists, ids are unique and monotonic, the immutable source digest
matches, and the legacy candidate and current lineage bindings agree.
Duplicate recommendations may map to one `R` id but never disappear.

A legacy reporting role remains immutable source attribution. If that role is
still in the current pool, it retains the normal reporting-seat judgment
obligation. If D21 retired it, the adapter applies a versioned deterministic
accountability-successor table without relabeling the source. The initial table
maps legacy `rust` to current `software` with the Rust profile D21 assigns it.
The successor submits the reporting obligation and a second non-reporting
final-roster seat supplies independent coverage. A legacy source cannot be
withdrawn on behalf of a retired seat; a false source is closed through the
independent `verified_invalid` rule instead.

No legacy severity is invented. Every `LegacySourceId`, not every synthesized
issue, begins `severity_origin = migration_untriaged`. Trusted dispatch obtains
one explicit current-schema `SubmitLegacySourceTriage` per source and at least
one `SubmitLegacySourceTriageVerification` by a final-roster seat that neither
reports that source nor implemented the candidate. The resulting source value is
`severity_origin = migration_assigned`; it is a current migration judgment,
not historical fact. Until every imported source has verified triage, no
implementation disposition can satisfy approval. The controller first
computes two disjoint sets. A source with no submitted triage is in
`missing_source_triage`; a source with submitted triage but no current
independent verification is in
`present_unverified_or_stale_source_triage`. If the first set is nonempty it
returns `legacy-source-triage-missing` with exactly that set. Only when the
first is empty may it return
`legacy-source-triage-unverified-or-stale` with exactly the second set. No
source can satisfy both predicates, and no generic triage refusal exists.

Effective issue severity is then derived from the current mapping and the
current severity of each mapped source under section 3. A split or merge
replays the same source triage and source correction events automatically.
It invalidates only dependent issue-level verification, adjudication and
acceptance state whose effective source set changed; disposition history
replays under section 4 and it does not request a second migration triage.

If the historical reporting seat still exists, it alone may submit a
correction to its migration-assigned source severity. If that seat was
retired, the versioned accountability successor submits
`SubmitSeverityCorrection` and an independent non-reporting final-roster seat
submits `SubmitSeverityCorrectionVerification`. Requiring the unavailable
historical seat is forbidden. The correction remains a current migration
judgment over the immutable `LegacySourceId`; it never edits historical bytes
or claims the historical recommendation carried a severity.

Generation is idempotent. The same complete round, candidate, and accepted
grouping return the same source ids, `R` ids, descriptions, crosswalk, and
artifact digests. A changed grouping after admission is a dedup correction,
not regeneration. Repeated ingestion appends no duplicate sources, judgments,
metrics, or audit events.

For a same-scope successor from a partial round, generation uses
`LogicalSuccessorImportId`. The same complete logical tuple returns the same
legacy sources, successor and atomic crosswalk across fresh protected
attempts. Its
top-level terminal progress and, after ledger admission,
`CompleteAdmittedDiscovery::NativeDiscovery` payload record one imported
partial-legacy prior-obligation set with the exact completed-seat,
`LegacySourceId`, imported-effective-issue, and three-way source-triage
counts.
`partial_round_retry_count` counts distinct admitted missing-seat redispatches,
and `migration_retry_count` counts distinct admitted import attempts.
Byte-identical request replay, response loss, and idempotent regeneration do
not increment either count or any source, seat, issue, or successor count.

After import, section 7 automatically generates every seat's verification
artifact with the full imported ledger and seat obligations. Legacy strings
are never hand-copied into reviewer notes.

### 13. Retention, redaction, audit, and terminal metrics are explicit

Every new artifact is in one ADR 0053 D17 retention class. This section
narrowly replaces D17 where accepted-attempt replay requires a permanent
controller floor rather than permanent raw response bytes.

| Artifact or authoritative record | D17 class and cleanup |
| --- | --- |
| Exact native and legacy reviewer bytes, prompts, generated per-seat bundles, full protected ledger pages, issue descriptions, source text, validation-output bytes, private acceptance rationale, protected authority mappings, migration work records, `SuccessorImportCapsule` bytes, and full protected accepted-request or response bytes | Round input. Retain for 30 days or within the 2-GiB bound after eligibility, whichever binds first. |
| `AcceptancePrepare` | Non-authoritative controller recovery state. It is ineligible until atomically promoted to an `AcceptedAttemptJournal` or cancelled with a sink-verifiable proof that promotion is permanently impossible. |
| `AcceptedAttemptJournal` | Audit floor while pending and ineligible. After audit acknowledgement is persisted and terminal activation completes, compact its safe identity and digests into the permanent `AttemptTombstone`; any separate protected request bytes are ordinary eligible round input. |
| `IdempotencyReplayResult` | Its bounded full protected response bytes are round input. Its closed outcome, response digest, and safe result ids are copied into the permanent `AttemptTombstone`. |
| Pending `AuditOutboxRow` | Audit floor and ineligible until the append sink's original acknowledgement is persisted by the controller. Then compact its event id, event digest, and acknowledgement digest into the permanent `AttemptTombstone`; the sink owns its separately bounded audit copy. |
| `AttemptTombstone` | Immutable permanent controller audit and replay floor for the lifetime of the controller namespace. It contains endpoint, operation, closed outcome, closed refusal code and safe causing or result identifiers and digests, but no protected request or response bytes and no mutable availability field. |
| `ReplayPayloadEvictionPrepared` and `ReplayPayloadEvicted` | Append-only monotonic replay-payload records. They never restore availability or alter the base tombstone. |
| `AuditSinkReservation` | Sink-side durable capacity edge keyed by `AttemptIdentity`, with a monotonic generation that, once appendable, is authorized for exactly one event id and digest. Assignment-issuance repair uses the non-repeating epoch-and-counter generation below so a finite counter rolls to a new epoch rather than wrapping. It is ineligible from creation until an append tombstone exists or the controller proves that authoritative acceptance is permanently impossible. |
| `AuditConversionIntent`, `AuditSinkInvalidationProof`, and `AuditSinkRebindProof` | Audit floor and ineligible while the named audit conversion is pending. The records are keyed by `AttemptIdentity` and the old reservation generation. The replacement-activation transaction compacts their exact digests into an immutable `AuditConversionTombstone`; only then do their protected bytes become eligible round input. |
| `AuditConversionTombstone` | Immutable permanent controller audit floor. It binds the attempt identity, old and replacement reservation generations, replacement refusal event id and digest, and the intent, invalidation-proof, and rebind-proof digests. It contains no proof or event bytes. |
| `MigrationAuditRepairIntent`, migration `AuditSinkInvalidationProof`, and `MigrationAuditSinkRebindProof` | Audit floor and ineligible while migration-specific no-append repair is pending. They preserve the original migration success result, quarantined capacity-switch effect, outbox event id and digest, and `MigrationExecutionReserve` binding. |
| `MigrationAuditRepairTombstone` | Immutable permanent controller audit floor binding the migration attempt alias, old and replacement reservation generations, unchanged success event alias and digest, and repair intent, invalidation-proof, and rebind-proof digests. It contains no proof, event, result, or authority-effect bytes. |
| `AssignmentIssuancePrepareState`, its accepted-attempt and canonical-audit binding, prepared-handler recovery, evidence reservation, controller reservation, sink reservation, and sink prepare, activation, non-creatable-fence, or cancellation proof | Current recovery floor while issuance is pending. An exact matching sink reservation is adopted. A non-adoptable incarnation is first made permanently non-creatable at the sink, then proof-cancelled before controller release; its sink fence tombstone is a permanent floor. A prepared-attempt cancellation remains ineligible after the exact incarnation is fenced and proof-cancelled: refusal installation retains controller capacity, evidence, the request reservation, and newer-incarnation eligibility in quarantine. Its final transaction is admitted only from normal ordinary activation with the original refusal acknowledgement or cancellation repair with the proof-bound final acknowledgement and durable repair tombstone or preparation. The success branch has the same exact two-source shape for its own original or repaired acknowledgement. Only the source-specific final activation atomically installs its authority and replay effects and attempt tombstone. No age-only cleanup is permitted. |
| `AssignmentIssuanceAuditRepairState`, its fixed repair workspace, unchanged success tuple, accumulator, current-cycle proofs, and acknowledgement | Audit floor while an issuance-success definite-no-append repair is pending. It retains the prepared assignment effect, evidence, request, and both capacity reservations. Before a cycle fold, that cycle's definite-no-append, invalidation, and rebind-or-rollover proof bytes are all ineligible. The atomic fold is their durable compaction boundary: after the new accumulator root and retry count commit, exactly those three folded proof records are ordinary eligible round input and all three fixed proof slots are reusable. Unfolded current-cycle proofs remain ineligible. Final activation requires a proof-bound acknowledgement and a durable repair tombstone or preparation whose root is the committed accumulated root. Retained repair-intent and any remaining current-cycle source bytes stay ineligible until final activation commits with the permanent repair tombstone durable; a preparation alone is insufficient. |
| `AssignmentIssuanceAuditRepairTombstone` | Bounded immutable permanent controller audit floor keyed by the exact `AttemptIdentity`. Its minimal redacted fields bind the issuance prepare identity digest and incarnation, initial and final repair generations, unchanged issuance event id and digest, final repair accumulator root, saturating retry count, and final acknowledgement digest. It contains no raw proof, canonical event bytes, result bytes, evidence, request, assignment capability, or authority effect. |
| `AssignmentIssuanceCancellationAuditRepairState`, its fixed repair workspace, unchanged canonical cancellation refusal, accumulator, current-cycle proofs, and acknowledgement | Audit floor while a prepared-cancellation refusal definite-no-append repair is pending. It retains the fenced and proof-cancelled prepare plus quarantined controller capacity, evidence, request reservation, newer-incarnation eligibility, replay result, and old attempt. Before a cycle fold, that cycle's definite-no-append, invalidation, and rebind-or-rollover proof bytes are all ineligible. The atomic fold commits the new accumulator root and retry count, makes exactly those three folded proof records ordinary eligible round input, and permits reuse of all three fixed proof slots. Unfolded current-cycle proofs remain ineligible. Retained repair-intent and any remaining current-cycle source bytes stay ineligible until final activation commits with the permanent cancellation-repair tombstone durable; a preparation alone is insufficient. It can never construct `audit-event-flush-failed`. |
| `AssignmentIssuanceCancellationAuditRepairTombstone` | Bounded immutable permanent controller audit floor keyed by the exact `AttemptIdentity`. It binds the issuance prepare identity digest and incarnation, initial and final repair generations, unchanged cancellation-refusal event id and digest, final repair accumulator root, saturating retry count, and final acknowledgement digest. It contains no raw proof, event bytes, evidence, request, or authority effect. Final cancellation activation verifies or materializes this binding before installing terminal effects. |
| `AssignmentRevocationAuditState`, its dedicated outbox, sink reservation, invalidation proof, and rebind proof | Audit floor and ineligible while the assignment remains `RevocationPending`. Definite-no-append repair retains the unchanged revocation event and can neither restore `Active` nor create a generic refusal. After acknowledgement and zero reserved uses reach the exact ready state and atomic finalization installs `Revoked`, proof bytes become ordinary audit-period input while the revocation event projection remains at the audit floor. |
| `AssignmentRevocationCapacityReleaseState`, unused sink cancellation proof, and controller release proof | Recovery state and audit floor while a non-revocation `Complete`, `Expire`, or `Exhaust` terminal intent is pending. The sink proof-cancels its issuance-time reservation before controller capacity is released. Proof bytes become ordinary audit-period input only after atomic terminal install; no age-only cleanup or successor admission is permitted. |
| `AuditAppendTombstone` | Permanent append-sink idempotency floor for the sink namespace. It contains only audit event id and digest plus the original acknowledgement and has no event payload bytes. Raw sink event bytes remain under the sink's bounded rotation. |
| `MigrationPreflightSignal::ReplayConflict` | D17 round input in a stricter diagnostic subclass: exactly 256 reusable detailed slots in the active 15-minute window, forced compaction at rotation or generation cutover, and a hard 30-minute TTL even when compaction fails. It is never an audit floor or migration blocker. |
| `MigrationPreflightSignal::AggregateOverflow` and `MigrationPreflightSignalWindowSummary` | D17 round input in the same stricter diagnostic subclass: one reusable overflow slot in the active window and a controller-wide ring of exactly 96 compacted summary slots. A summary has a hard 24-hour TTL; rotation and generation cutover reuse slots and never wait for export. |
| `MigrationControlConflictSignal::Detailed` | D17 diagnostic round input in a separate fixed-cardinality subclass: exactly 64 reusable slots in the active 15-minute window and a hard 30-minute TTL. It is never an audit floor, accepted attempt, mutation-slot record, or migration blocker. |
| `MigrationControlConflictSignal::AggregateOverflow` and `MigrationControlConflictWindowSummary` | D17 diagnostic round input in that same separate subclass: one reusable overflow slot per active window and a controller-wide ring of exactly 96 summaries. A summary has a hard 24-hour TTL; rotation and control-reserve incarnation change reuse slots and never wait for export. |
| `MigrationTelemetryHealthMarker`, `MigrationTelemetryFailureLatch`, current failure-alias record, telemetry recovery slot, reusable last-result slot, and matching normal health | D17 current diagnostic and recovery state, not round history: one integrity-checked current-and-shadow marker, separately reserved core-metadata latch, independently sealed current server-issued failure alias, matching fixed normal-health storage, separately sealed recovery and audit workspace, and one fixed last-result slot that remains readable during a later recovery and is overwritten only by that later settled outcome. Normal closure records proof but only successful `RecoveryBarrier` completion clears the latch. A closed failure rotates the alias and permits a fresh recovery cycle only after its canonical failure event is acknowledged and `LastResult::Failed` is installed. Protected recovery events retain exact marker sequences under the ordinary audit period, but metric labels may not. |
| `MigrationControlCommandSlot`, integrity-command slot, and their current or last result | Current controller recovery state for the one accepted migration, not round history. Each fixed primary and refusal slot durably enters `PreparingAuditCapacity` before any child-audit reservation. The slots are reused after audited settlement and retain only the current command or last exact safe result plus its audit-event digest. They never create a permanent child-command tombstone. |
| `MigrationChildAuditOutbox`, `MigrationChildAuditSinkReservation`, and append acknowledgement | Fixed pre-sealed controller and sink workspace. Every reservation binds one immutable command and fixed-slot prepare identity and excludes mutable worker ownership. The outbox is ineligible until acknowledgement, and the slots are reusable only after proof-backed adoption or cancellation and settlement. No child command may borrow ordinary accepted-attempt outbox or tombstone capacity. |
| `MigrationControllerRekeyRecord` | Fixed non-resettable current recovery state. It survives the epoch install it authorizes, retains requested, prepared, audit-pending, acknowledged-install-pending, or terminal installed state under one controller-issued identity, the primary outcome nonce, distinct nonces for any other admitted terminal outcomes, and bounded counter-independent continuation records. It is never reconstructed with the six resettable counters. |
| `MigrationChildCommandAuditEvent`, `MigrationEpochRekeyOutcomeEvent`, and `MigrationTelemetryHealthRecoveryEvent` | D17 append-only audit evidence under the ordinary bounded audit period. The sink retains canonical event bytes and its bounded idempotency index only for that period; expiry never makes an old epoch, generation, incarnation, sequence, rekey identity, or telemetry failure current again. |
| `MigrationStatusAccessAuditEvent` | D17 bounded disclosure evidence. Every successful status disclosure has a fresh controller-issued identity and distinct event containing the current state digest and mandatory deployment-keyed `ProtectedOperatorAuditDigest`, never `protected_operator_alias`. The fixed integrity-reserve slot and its disclosure-identity-bound reservations serialize one disclosure and retain no per-operator history; bounded sink retention owns history. The digest exists only in canonical audit bytes. It is not an accepted attempt or response cache. |
| Source and artifact digests, stable ids, source mapping and crosswalk events, dedup and severity events, closed disposition and judgment projections, roster and dispatch bindings, acceptance and revocation projections, lifecycle receipts, seals, and terminal metric records | Audit floor under D17's ordinary audit period unless another row, such as `AttemptTombstone` or `AuditAppendTombstone`, sets a longer lifetime. |

Attempt identity is controller-derived and mandatory:

```
ProtectedAttemptId =
  digest(
    "d2b:panel:protected-attempt:v1",
    controller_namespace,
    endpoint_discriminant,
    operation_discriminant,
    authenticated_stable_peer_identity_digest,
    idempotency_key_digest
  )

ConflictAttemptId =
  digest(
    "d2b:panel:protected-attempt-conflict:v1",
    ProtectedAttemptId,
    conflicting_request_digest
  )

AttemptIdentity =
  Base(ProtectedAttemptId)
  | Conflict(ConflictAttemptId)
```

`ProtectedAttemptId` explicitly excludes request bytes and their digest.
Changing peer, endpoint, or operation therefore creates a different protected
attempt even when an idempotency key is reused. A changed request under the
same protected attempt creates the one `ConflictAttemptId` for that
conflicting request digest. Except for the migration preflight, active-migration child-command, and
current-state migration-status exceptions below, that request is a distinct
accepted and audited attempt with
`AttemptIdentity::Conflict`; it never reuses the base attempt's capacity,
journal, reservation, proof, event, result, marker, tombstone, worker, audit
record, or status. A same-key, different-request
`MigrateRetentionCapacity` is instead classified before accepted-attempt
registration. Its conflict id is safe correlation only and is never a durable
`AttemptIdentity`, accepted attempt, or charge against migration execution
capacity. A `MigrationControlCommand` does not derive either attempt id: its
identity is a child of the one active migration and its caller key is
nonsemantic. `ReadMigrationRecoveryStatus` derives no request identity at all.
The controller namespace is stable across restart and reviewed
storage migration. Permanent indexes address the base and every admitted
conflict tombstone by `AttemptIdentity` across journal compaction,
replay-payload eviction, and restart; eviction never makes an id reusable.

`AttemptIdentity` is the mandatory key for `AcceptancePrepare`,
`AcceptedAttemptJournal`, `IdempotencyReplayResult`, `AuditOutboxRow`,
`AuditSinkReservation`, accepted-journal and no-journal proofs,
`AttemptTombstone`, replay eviction markers, worker leases and recovery state,
audit-conversion records and tombstones, migration-audit-repair records and
tombstones, both assignment-issuance audit-repair record families and
tombstones, audit events, and `ReadProtectedAttemptStatus`. A schema that keys
any of those records by a
bare `ProtectedAttemptId` or allows a base and admitted conflict attempt to
share one record fails construction.

That mandatory-key list applies only to ordinary accepted attempts and the
accepted migration itself. It explicitly excludes
`RevokeImplementationAssignment`,
`ResumeProtectedAttempt`, `FenceProtectedAttempt`,
`RepairMigrationSinkAppend`, `CompleteMigrationAuditActivation`,
`RepairMigrationControlReserve`, `RekeyMigrationControllerEpoch`, and
`RecoverMigrationTelemetryHealth` when they target the active capacity
migration or controller telemetry health, as well as
`ReadMigrationRecoveryStatus`. None can construct an
`AcceptancePrepare`, `AcceptedAttemptJournal`, `IdempotencyReplayResult`,
ordinary `AuditOutboxRow`, `AttemptTombstone`, replay-payload marker, generic
worker lease, or generic protected-attempt recovery variant of its own.

The authoritative attempt and append records have closed roles:

- `AcceptedAttemptJournal` is the controller's durable statement that an
  authenticated request crossed the acceptance boundary. It binds the
  `AttemptIdentity`, base `ProtectedAttemptId`, endpoint, operation,
  authenticated peer digest,
  idempotency-key digest, request digest, acceptance time, reserved capacity,
  sink reservation, and one linear fenced state.
- `IdempotencyReplayResult` is the bounded terminal response record. It binds
  the result kind, safe result ids, exact protected response digest, and, only
  while retained, the full protected response bytes.
- `AuditOutboxRow` is the exact canonical audit event awaiting the generic
  append sink. It binds `AuditEventId`, event kind, exact event bytes and
  digest, and the sink acknowledgement once known.
- `AttemptTombstone` is the permanent minimal replay authority. It binds the
  `AttemptIdentity`, base attempt and request digests, endpoint, operation,
  closed terminal result kind and outcome, closed refusal code when applicable,
  the operation-specific exact safe refusal product or success identifiers,
  audit event alias and digest, and original acknowledgement digest. Completion
  and append-authorization refusals embed the exact products defined below,
  never a widened generic map. It is immutable after creation,
  is sufficient to refuse re-execution and conflicting reuse, and is not a
  reconstruction of protected response bytes.
- `ReplayPayloadEvictionPrepared` and `ReplayPayloadEvicted` are immutable
  append-only markers keyed by `AttemptIdentity`. Availability is derived
  from the tombstone, the replay-result row, and absence of either marker; it
  is never a boolean rewritten inside the tombstone.
- `AuditSinkReservation` is the sink's durable reservation of the maximum
  bounded raw event and append-tombstone capacity for one `AttemptIdentity`.
  Each monotonically increasing appendable reservation generation binds
  exactly one authorized `AuditEventId` and event digest; prepared capacity is
  non-appendable.
- `AuditConversionTombstone` is the controller's permanent proof that one old
  sink generation was invalidated and rebound to one replacement refusal
  event before replacement activation. Its three source-record digests are
  immutable and do not make the protected proof bytes permanent.
- Issuance-success and prepared-cancellation audit repair share one fixed
  workspace shape and one accumulator contract:

  ```
  AssignmentIssuanceRepairGeneration = {
    sink_reservation_epoch: SinkReservationEpochId,
    reservation_generation: NonZeroU64
  }

  AssignmentIssuanceRepairWorkspace = {
    repair_id,
    outcome: IssuanceSuccess | PreparedCancellationRefusal,
    attempt_identity: AttemptIdentity,
    issuance_prepare_identity_digest,
    prepare_incarnation,
    prepared_activation_binding_digest,
    initial_generation: AssignmentIssuanceRepairGeneration,
    current_generation: AssignmentIssuanceRepairGeneration,
    unchanged_event_id,
    unchanged_event_digest,
    unchanged_event_bytes_digest,
    repair_intent_digest,
    accumulator_root,
    retry_count: SaturatingBoundedCount
  }
  ```

  For `IssuanceSuccess`, `prepared_activation_binding_digest` covers the sink
  activation proof, assignment binding digest, evidence and request
  reservations, successor eligibility, and both capacity reservations. For
  `PreparedCancellationRefusal`, the field has the concrete type
  `PreparedCancellationActivationBindingDigest` and covers the permanent
  sink-fence proof, sink proof-cancellation digest, cancellation reason,
  reserved next incarnation, request and evidence reservation bindings,
  successor eligibility, and controller capacity binding. The distinct
  `PreparedCancellationPreProofBindingDigest` is not a valid workspace value
  and cannot be widened, padded, or substituted for the complete digest. In
  both cases
  it also covers the canonical event bytes digest, so a workspace cannot be
  transplanted between the two activation kinds or between prepared effects.
  A definite-no-append result enters the workspace only after authenticating
  the sink principal and matching the exact attempt, repair identity,
  `current_generation`, append authorization, unchanged event id, digest, and
  bytes digest. A result for any earlier, future, other-attempt, other-repair,
  or changed-event tuple is the family-specific repair tuple mismatch and
  changes neither the workspace nor the sink.

  The route reserves this workspace, one current-cycle
  definite-no-append proof slot, one invalidation-proof slot, one rebind-or-
  rollover-proof slot, one tombstone-preparation slot, and their schema
  maxima before ordinary acceptance. It creates no per-retry controller row
  and cannot borrow generic conversion or migration capacity. For completed
  invalidation/rebind cycle `n`:

  ```
  accumulator_root[0] =
    digest(
      "d2b:panel:assignment-issuance-repair-accumulator-start:v1",
      repair_id,
      attempt_identity,
      issuance_prepare_identity_digest,
      prepare_incarnation,
      prepared_activation_binding_digest,
      initial_generation,
      unchanged_event_id,
      unchanged_event_digest,
      repair_intent_digest
    )

  accumulator_root[n + 1] =
    digest(
      "d2b:panel:assignment-issuance-repair-accumulator-step:v1",
      accumulator_root[n],
      repair_id,
      attempt_identity,
      issuance_prepare_identity_digest,
      prepare_incarnation,
      prepared_activation_binding_digest,
      invalidated_generation,
      replacement_generation,
      authenticated_definite_no_append_proof_digest,
      invalidation_proof_digest,
      rebind_or_rollover_proof_digest,
      unchanged_event_id,
      unchanged_event_digest
    )
  ```

  The tuple-commit transaction authenticates all three current-cycle proof
  operands, folds them into the root, advances `current_generation`, increments
  the saturating retry count, and commits one durable compaction boundary. The
  root therefore commits every invalidation and rebind even after the count
  saturates, while controller state remains fixed size. The count is the
  number of completed invalidation/rebind cycles, including the first move
  from `initial_generation`; saturation loses no proof history because the
  root continues to advance.

  Before that transaction commits, the prior root and retry count remain
  authoritative, the current cycle's raw definite-no-append, invalidation,
  and rebind-or-rollover proof bytes all remain ineligible, and all three
  fixed proof slots remain occupied. After it commits, the new root and retry
  count are authoritative, exactly those three folded proof records become
  ordinary eligible round input, and all three fixed proof slots may be
  reused by the next cycle. No other record changes retention class at this
  boundary. Recovery
  never infers the side of the boundary from slot contents: a crash before
  commit resumes the proof-bound pre-fold state, while a crash after commit
  resumes the installed replacement tuple and never folds that cycle again.
  `RunControllerRetentionCleanup` may reclaim only the eligible folded-cycle
  proof bytes under the ordinary 30-day or 2-GiB rule. It cannot reclaim any
  unfolded current-cycle proof, retained repair-intent, or remaining
  current-cycle source bytes before final activation commits with the
  permanent repair tombstone durable, and it cannot alter the committed
  accumulator.

  The `NonZeroU64` generation component never wraps. When its next value is
  not representable, proof persistence enters the typed
  `ReplacementGenerationRolloverPending` state. Its state-owned rollover
  action uses the same workspace and reserved capacity, creates a new
  controller-derived `SinkReservationEpochId`, binds generation one in that
  epoch to the unchanged event, and returns one authenticated rollover-and-
  rebind proof for the accumulator step. The prior epoch is permanently
  non-appendable. The new epoch id is:

  ```
  digest(
    "d2b:panel:assignment-issuance-repair-sink-epoch:v1",
    repair_id,
    attempt_identity,
    issuance_prepare_identity_digest,
    current_generation,
    accumulator_root,
    unchanged_event_id,
    unchanged_event_digest
  )
  ```

  The rollover proof binds that derivation, both epochs, the exhausted
  generation, replacement generation one, the unchanged event, and the
  current-cycle no-append and invalidation proofs. Replaying it after a crash
  returns the same proof; a different epoch, event, or proof tuple is a repair
  tuple mismatch. `SinkReservationEpochId` is a digest identity, not a finite
  counter. This reservation counter is not one of the six resettable
  migration-controller counters, so `RekeyMigrationControllerEpoch` is not
  invoked; the typed sink-epoch rollover is the required no-wrap route.

- `AssignmentIssuanceAuditRepairTombstone` is a bounded permanent controller
  audit-floor record keyed by exact `AttemptIdentity`:

  ```
  AssignmentIssuanceAuditRepairTombstone = {
    key: AttemptIdentity,
    issuance_prepare_identity_digest,
    prepare_incarnation,
    initial_generation: AssignmentIssuanceRepairGeneration,
    final_generation: AssignmentIssuanceRepairGeneration,
    unchanged_issuance_event_id,
    unchanged_issuance_event_digest,
    repair_accumulator_root,
    retry_count: SaturatingBoundedCount,
    final_acknowledgement_digest
  }
  ```

  These are its complete fields. It is created only in the final
  post-acknowledgement assignment-activation transaction or is already durable
  with exactly those bytes after an idempotent tombstone-materialization
  retry. The only alternative activation prerequisite is a durable
  tombstone-preparation tuple with exactly those fields and its own digest.
  The tombstone binds the final committed accumulator root and retry count.
  Folded-cycle proof eligibility is established only at each earlier atomic
  fold and is not delayed until final activation; an unfolded cycle cannot
  construct this tombstone or its preparation. Final activation makes the
  retained repair-intent and any remaining current-cycle source bytes
  ordinary eligible round input only after this permanent tombstone is
  durable; the preparation does not establish that eligibility.
- `AssignmentIssuanceCancellationAuditRepairTombstone` is the corresponding
  bounded permanent controller audit floor for an unchanged canonical
  prepared-cancellation refusal:

  ```
  AssignmentIssuanceCancellationAuditRepairTombstone = {
    key: AttemptIdentity,
    issuance_prepare_identity_digest,
    prepare_incarnation,
    initial_generation: AssignmentIssuanceRepairGeneration,
    final_generation: AssignmentIssuanceRepairGeneration,
    unchanged_cancellation_refusal_event_id,
    unchanged_cancellation_refusal_event_digest,
    repair_accumulator_root,
    retry_count: SaturatingBoundedCount,
    final_acknowledgement_digest
  }
  ```

  These are its complete fields. Its final cancellation activation accepts
  only the exact durable tombstone or exact durable tombstone-preparation
  tuple binding the final committed accumulator root and retry count.
  Folded-cycle proof eligibility remains controlled by the earlier atomic
  folds; an unfolded cycle cannot construct either final record. Final
  activation makes the retained repair-intent and any remaining current-cycle
  source bytes ordinary eligible round input only after this permanent
  tombstone is durable; the preparation does not establish that eligibility.

  ```
  AssignmentIssuanceRepairTombstonePreparation =
    IssuanceSuccess {
      exact_tombstone_fields: AssignmentIssuanceAuditRepairTombstone,
      preparation_digest:
        digest(
          "d2b:panel:assignment-issuance-repair-tombstone-prepare:v1",
          exact_tombstone_fields
        )
    }
    | PreparedCancellationRefusal {
        exact_tombstone_fields:
          AssignmentIssuanceCancellationAuditRepairTombstone,
        preparation_digest:
          digest(
            "d2b:panel:assignment-issuance-cancellation-repair-tombstone-prepare:v1",
            exact_tombstone_fields
          )
      }
  ```

  The preparation is durable recovery state, not the permanent audit floor.
  It is valid only when its exact tombstone fields rederive the current
  workspace, final generation, accumulator root, retry count, unchanged event,
  and proof-bound final acknowledgement. Final activation either finds the
  byte-identical permanent tombstone or materializes these exact fields; a
  tombstone or preparation from the other outcome is a binding mismatch.
- `AuditAppendTombstone` is the append sink's permanent minimal deduplication
  authority. It is sufficient to return the original acknowledgement after
  raw sink event rotation and contains no protected request, response, or event
  payload bytes.

Before authoritative acceptance of an ordinary attempt, the controller
reserves one journal slot, one outbox slot, one attempt-tombstone slot, the
maximum two payload-eviction marker slots, the bounded request and result
budget, and the section 13 recovery reserve. The permanent controller
tombstone budget separately includes every
`AssignmentIssuanceAuditRepairTombstone` and
`AssignmentIssuanceCancellationAuditRepairTombstone` entry and byte maximum.
A closed operation selector reserves the maximum complete mutually exclusive
route: generic conversion and its `AuditConversionTombstone`; for
`IssueImplementationAssignment`, either generic conversion for a
non-prepared refusal or one dedicated success or prepared-cancellation repair
including its permanent repair tombstone, fixed reusable accumulator and
current-cycle proof workspace, tombstone preparation, and sink-generation
rollover proof;
or, only for `MigrateRetentionCapacity`, one
`MigrationAuditRepairTombstone` plus the maximum bounded migration-repair
intent and proof bytes. These allocations are disjoint and no selected route
may borrow another route's tombstone slot. A closed capacity
selector charges a base ordinary
attempt to ordinary capacity, an accepted conflict including a status
operation to
`AcceptedConflictReserve`, a base protected status or recovery read to
`ProtectedStatusReserve`, and the one non-conflict migration only to
`MigrationExecutionReserve`. The marker-driven retention read and active
migration control exceptions use only their explicitly sealed reserves. No
class may fall through to another partition.
Cross-store reservation then uses this reconciliable protocol:

1. In one controller transaction, create a non-authoritative
   `AcceptancePrepare` keyed by `AttemptIdentity`. It binds the base
   `ProtectedAttemptId`, request and peer digests, endpoint and operation,
   every controller-local capacity reservation, and a canonical prepare
   digest. It is visible to protected attempt status but is not an accepted
   attempt and authorizes no operation processing.
2. The sink durably creates `Prepared` capacity for that exact
   `AttemptIdentity`, base `ProtectedAttemptId`, controller namespace, prepare
   digest, reservation id, and reservation generation. Repeating the same
   prepare returns the same reservation. A different prepare digest for that
   identity is `acceptance-prepare-digest-conflict` and creates or changes
   nothing.
3. In one controller transaction, compare-and-swap the exact
   `AcceptancePrepare` to `AcceptedAttemptJournal`, binding the sink
   reservation id, digest, and generation. This commit, and no earlier step,
   is authoritative acceptance.
4. The controller presents an unforgeable `AcceptedJournalProof` over the
   exact journal and reservation. The sink compare-and-swaps `Prepared` to
   `AcceptedBound`. A worker may not claim the accepted attempt until this
   binding exists.

No step assumes a transaction shared by the two stores. Recovery is total at
every boundary. A controller-only prepare is completed or cancelled. A
sink-side `Prepared` reservation with no accepted journal is either promoted
by the still-valid controller prepare or cancelled only after the controller
atomically marks that prepare non-promotable and issues
`NoAcceptedJournalProof`. An accepted journal whose sink remains `Prepared`
causes recovery to replay `AcceptedJournalProof` until the sink binds it; it is
never treated as an orphan or leaked. An `AcceptedBound` sink state necessarily
names the accepted-journal proof that authorized it. Repeating any completed
step is idempotent.

The sink cannot reclaim a `Prepared` or `AcceptedBound` reservation by age.
`NoAcceptedJournalProof` binds the controller namespace, `AttemptIdentity`,
base `ProtectedAttemptId`, prepare digest, reservation id, and exact
generation. The controller may issue it only in the same authority transaction
that makes the prepare permanently non-promotable after proving that no
accepted journal binds the reservation. A proof for one cancelled generation
cannot release a later generation. If the controller is unavailable, the
reservation remains and reclamation returns
`audit-sink-orphan-proof-controller-unavailable`. If a presented proof is
invalid or its controller and sink bindings disagree, the reservation remains
and reclamation returns `audit-sink-orphan-proof-invalid`. Beneath an accepted
journal the reservation remains until the stable `AuditEventId` has its
`AuditAppendTombstone`; there is no time-only expiry or orphan guess.

The pending-record, controller tombstone, and append-sink tombstone budgets
have versioned finite entry and byte maxima. The permanent controller budget
counts `AttemptTombstone`, `AuditConversionTombstone`,
`MigrationAuditRepairTombstone`, `AssignmentIssuanceAuditRepairTombstone`,
and `AssignmentIssuanceCancellationAuditRepairTombstone` by their exact
serialized entry and byte maxima. If permanent controller or sink tombstone
capacity is exhausted, `replay-tombstone-store-full` or
`audit-append-tombstone-store-full` respectively refuses new acceptance;
tombstones are never evicted to make room. `MigrateRetentionCapacity` is the
only capacity escape. It requires a reviewed manifest and a closed reason of
`ReserveIntegrityRepair` or `VersionedBoundMigration`, copies every permanent
id and digest including both assignment-issuance repair tombstone families,
recomputes every permanent entry and byte maximum and every recovery
reservation, verifies the complete set, and atomically switches storage
without changing the controller namespace. A destination schema or reviewed
manifest that omits either repair tombstone family, its exact
`AttemptIdentity` key, or its serialized maximum is invalid and cannot switch.
General-store fullness alone does not authorize it. Resetting a namespace or
dropping an old key is forbidden. This is finite fail-closed storage, not
unbounded raw response retention.

An identical retry has one authority behavior before and after full-result
eviction: it never executes the operation again and never appends another
audit event. Response-byte availability is the only difference.
Before eviction, the controller returns the byte-identical stored response and
original append acknowledgement. After eviction, it returns the deterministic
typed `idempotency-result-evicted` replay result containing the safe attempt
identity, endpoint, operation, closed outcome, safe result identifiers, event and
response digests, and the operation-specific recovery projection described
below. It never treats absence of response bytes as permission to execute. A
same-key, different-request-digest retry selects exactly one conflict variant
before or after eviction: `risk-operation-replay-conflict` for
`IssueRiskOperationIntent`, `RequestNewRiskOperationIntent`,
`AcceptMajorRisk`, and `RevokeMajorRiskAcceptance`, and
`protected-operation-replay-conflict` for every other accepted operation.
Each distinct accepted conflicting request digest uses its
`AttemptIdentity::Conflict(ConflictAttemptId)` and crosses the same
acceptance-prepare, sink-reservation, accepted-journal, audit, replay,
eviction, and tombstone protocol independently and charges only the bounded
`AcceptedConflictReserve`. The base attempt is not rewritten or charged for
that refusal. Repeating those same conflict bytes replays that conflict
attempt's refusal from its own permanent tombstone.
`MigrateRetentionCapacity` is the accepted-attempt preflight exception: the same-key,
different-request check is preflight and returns
`retention-capacity-migration-replay-conflict` without accepted-attempt
registration or any permanent record allocation. Active-migration child
commands instead use the fixed child-command protocol below, and
`ReadMigrationRecoveryStatus` always performs a new current-state evaluation;
neither participates in this replay rule.

`ReadProtectedAttemptStatus` authenticates the original stable peer identity
bound into the named `AttemptIdentity` or a protected operator. Cross-peer
access is `protected-attempt-status-cross-peer` and reveals no outcome or
result id. Its safe result is reconstructed from the immutable tombstone and
current authoritative state.

Post-eviction recovery is a nested tagged type. Operation, outcome, state, and
action are never independent fields:

```
ProtectedAttemptRecovery =
  Success(ProtectedAttemptSuccessRecovery)
  | OriginalRefusal(ProtectedAttemptOriginalRefusal)

ProtectedAttemptSuccessRecovery =
  Orchestrator(OrchestratorSuccessRecovery)
  | Reviewer(ReviewerSuccessRecovery)
  | Operator(OperatorSuccessRecovery)
  | AssignmentIssuance(AssignmentIssuanceSuccessRecovery)
  | AssignmentCompletion(AssignmentCompletionSuccessRecovery)
  | IssueReader(IssueReaderSuccessRecovery)
  | AttemptStatus(AttemptStatusSuccessRecovery)
  | RecoveryRead(RecoveryReadSuccessRecovery)
  | Publisher(PublisherSuccessRecovery)

ProtectedAttemptOriginalRefusal =
  Orchestrator(OrchestratorOriginalRefusal)
  | Reviewer(ReviewerOriginalRefusal)
  | Operator(OperatorOriginalRefusal)
  | AssignmentIssuance(AssignmentIssuanceOriginalRefusal)
  | AssignmentCompletion(AssignmentCompletionOriginalRefusal)
  | IssueReader(IssueReaderOriginalRefusal)
  | AttemptStatus(AttemptStatusOriginalRefusal)
  | RecoveryRead(RecoveryReadOriginalRefusal)
  | Publisher(PublisherOriginalRefusal)

RecoveryNextAction =
  NoFurtherAction
  | Invoke {
      endpoint: Endpoint,
      operation: EndpointOperation,
      caller: AuthorizedCallerClass
    }
```

Each endpoint success enum has one generated wire variant for each valid
`(operation, closed terminal success outcome)` pair. The variant tag identifies
both values and owns only the exact safe fields and one exact next action in
the matrix below. Each risk-intent variant owns one exact
`OriginalPeerRiskRecovery` state-and-action variant; its raw handle exists
solely in the distinct immediate protected intent response and the
caller-disjoint operator-authenticated risk read from section 10. A state with several terminal values
generates one variant per value; it does not carry a separate `current_state`
field. Each endpoint
refusal enum likewise has one generated variant for each valid
`(operation, typed refusal)` pair. Its variant owns only that refusal's exact
safe product and exactly:

```
Invoke {
  endpoint: RecoveryRead,
  operation: ReadOriginalRefusalRecoveryState,
  caller: OriginalAttemptPeerOrProtectedOperator
}
```

That read returns the current typed remedy plan. There is no optional
field, generic `safe_ids`, generic state, generic action, `Other`, or fallback
variant.

An `Invoke` is constructible only when the endpoint table contains the named
operation and its authorization policy contains the named caller class.
`OriginalAttemptPeerOrProtectedOperator` is valid only on `AttemptStatus` and
`RecoveryRead`; `ExactOriginatingAssignmentIssuancePrincipal` is valid only on
`AssignmentIssuance`; and every other caller class is checked against its one
table row. The generator rejects an absent operation, wrong endpoint, wrong
caller class, recovery read for another attempt, or action string without all
three fields. `NoFurtherAction` is permitted only when the safe accepted
product is already sufficient and no authority, protected bytes, or capability
must be recovered.
The only generic action for a risk variant is its inline `Invoke` of
`RecoveryRead.ReadRiskRecoveryState` by
`ProtectedOperator` with `FreshForThisRead`. It is constructible only in the
pending and closed-permitted `OriginalPeerRiskRecovery` variants, resolves
only to the protected-operator risk row, and never treats original-peer
authentication as protected authorization.

One accepted state read returns a nested context-owned action rather than a
second independent `RecoveryNextAction`:
`ReadAssignmentRecoveryState` returns `AssignmentRecoveryContext`. Its
generator joins the exact tagged state to
the endpoint table, caller policy, reserve route where applicable, and any
fresh-evidence prerequisite. It rejects an action owned by another state,
an action with a missing prerequisite, a terminal assignment that skips the
linear successor request, or an active assignment that mints a capability.
The caller-disjoint risk generator separately rejects a context that omits or
substitutes its bound handle, any action encoded inside
an original-peer safe-field group, any handle in an original-peer outer variant, and a
risk state that requests a new intent before `ClosedMutationPermitted`. The
migration status generator rejects an action routed through the wrong reserve.
The immediate caller-disjoint response of operator-authenticated
`ReadRiskRecoveryState` is `ProtectedOperatorRiskRecoveryContext`, but its own
generic post-eviction protected-attempt recovery is again
an exact handle-free `OriginalPeerRiskRecovery` tag owning its state-valid
action. Thus no recursive or generic recovery surface
carries the handle.
`ReadMigrationRecoveryStatus` is generated separately from accepted-attempt
recovery. Its current-state schema owns its action and reserve route exactly as
defined below.

The following is the complete success-recovery schema input. `Stem<Outcome>`
means one distinct wire tag for every closed terminal outcome admitted for
that operation.

| Endpoint | Operation | Generated variant stem and exact owned safe fields | Exact next action |
| --- | --- | --- | --- |
| Orchestrator | `ProposeLifecycleStart` | `LifecycleStart<Outcome> { lifecycle_id, lifecycle_event_id }` | `RecoveryRead.ReadLifecycleRecoveryState` by `OriginalAttemptPeerOrProtectedOperator` |
| Orchestrator | `RequestPanelDispatch` | `PanelDispatch<Outcome> { lifecycle_id, dispatch_id, roster_manifest_digest }` | `RecoveryRead.ReadLifecycleRecoveryState` by `OriginalAttemptPeerOrProtectedOperator` |
| Orchestrator | `SubmitCandidateSnapshot` | `CandidateSnapshot<Outcome> { lifecycle_id, candidate_id, snapshot_digest }` | `NoFurtherAction` |
| Orchestrator | `SubmitLedgerSynthesisProposal` | `LedgerSynthesis<Outcome> { lifecycle_id, candidate_id, mapping_version, ledger_digest }` | `RecoveryRead.ReadLedgerRecoveryState` by `OriginalAttemptPeerOrProtectedOperator` |
| Orchestrator | `RequestImplementationAssignment` | `ImplementationAssignmentRequest<Outcome> { lifecycle_id, candidate_id, predecessor_or_partition_alias, assignment_request_id }` | `RecoveryRead.ReadAssignmentRecoveryState { context: FreshAssignmentFlow }` by `OriginalAttemptPeerOrProtectedOperator` |
| Orchestrator | `SubmitImplementationDisposition` | `ImplementationDisposition<Outcome> { lifecycle_id, candidate_id, issue_ids, disposition_event_ids }` | `NoFurtherAction` |
| Orchestrator | `SubmitImplementationSelfReviewFinding` | `ImplementationSelfReviewFinding<Outcome> { lifecycle_id, candidate_id, source_ids }` | `NoFurtherAction` |
| Orchestrator | `SubmitValidationManifest` | `ValidationManifest<Outcome> { lifecycle_id, candidate_id, validation_manifest_id, manifest_digest }` | `NoFurtherAction` |
| Orchestrator | `RequestGeneratedSeatArtifacts` | `GeneratedSeatArtifacts<Outcome> { lifecycle_id, candidate_id, mapping_version, artifact_ids, artifact_digests }` | `Orchestrator.RequestGeneratedSeatArtifacts` by `OriginalOrchestratorPeer` |
| Orchestrator | `ReadLifecycleStatus` | `OrchestratorLifecycleStatus<Outcome> { lifecycle_id, status_digest }` | `RecoveryRead.ReadLifecycleRecoveryState` by `OriginalAttemptPeerOrProtectedOperator` |
| Reviewer | `SubmitNativeFindingPage` | `NativeFindingPage<Outcome> { lifecycle_id, candidate_id, seat_id, page_ids }` | `NoFurtherAction` |
| Reviewer | `SubmitLateFinding` | `LateFinding<Outcome> { lifecycle_id, candidate_id, seat_id, source_ids }` | `NoFurtherAction` |
| Reviewer | `SubmitVerificationJudgment` | `VerificationJudgment<Outcome> { lifecycle_id, candidate_id, seat_id, issue_ids, judgment_event_ids }` | `NoFurtherAction` |
| Reviewer | `SubmitLegacySourceTriage` | `LegacySourceTriage<Outcome> { lifecycle_id, candidate_id, legacy_source_ids, triage_event_ids }` | `NoFurtherAction` |
| Reviewer | `SubmitLegacySourceTriageVerification` | `LegacySourceTriageVerification<Outcome> { lifecycle_id, candidate_id, legacy_source_ids, verification_event_ids }` | `NoFurtherAction` |
| Reviewer | `SubmitSeverityCorrection` | `SeverityCorrection<Outcome> { lifecycle_id, candidate_id, source_ids, correction_event_ids }` | `NoFurtherAction` |
| Reviewer | `SubmitSeverityCorrectionVerification` | `SeverityCorrectionVerification<Outcome> { lifecycle_id, candidate_id, source_ids, verification_event_ids }` | `NoFurtherAction` |
| Reviewer | `SubmitLedgerMappingConcurrence` | `LedgerMappingConcurrence<Outcome> { lifecycle_id, candidate_id, mapping_version, correction_id, concurrence_event_id }` | `RecoveryRead.ReadLedgerRecoveryState` by `OriginalAttemptPeerOrProtectedOperator` |
| Reviewer | `SubmitRiskAcceptanceVerification` | `RiskAcceptanceVerification<Outcome> { lifecycle_id, candidate_id, acceptance_id, verification_event_id }` | `NoFurtherAction` |
| Reviewer | `SubmitFinalSignoff` | `FinalSignoff<Outcome> { lifecycle_id, candidate_id, seat_id, signoff_event_id }` | `NoFurtherAction` |
| Operator | `SubmitApprovalDecision` | `ApprovalDecision<Outcome> { lifecycle_id, candidate_id, approval_event_id }` | `RecoveryRead.ReadLifecycleRecoveryState` by `OriginalAttemptPeerOrProtectedOperator` |
| Operator | `AbandonLifecycle` | `LifecycleAbandonment<Outcome> { lifecycle_id, lineage_id, lifecycle_event_id }` | `RecoveryRead.ReadLifecycleRecoveryState` by `OriginalAttemptPeerOrProtectedOperator` |
| Operator | `ResumeLifecycle` | `LifecycleResume<Outcome> { source_lifecycle_id, successor_lifecycle_id, lifecycle_event_id }` | `RecoveryRead.ReadLifecycleRecoveryState` by `OriginalAttemptPeerOrProtectedOperator` |
| Operator | `RescopeLifecycle` | `LifecycleRescope<Outcome> { source_lifecycle_id, successor_lifecycle_id, lifecycle_event_id }` | `RecoveryRead.ReadLifecycleRecoveryState` by `OriginalAttemptPeerOrProtectedOperator` |
| Operator | `CreateSameScopeCurrentSchemaSuccessor` | `SameScopeSuccessor<Outcome> { source_lifecycle_id, successor_lifecycle_id, logical_successor_import_id }` | `RecoveryRead.ReadLifecycleRecoveryState` by `OriginalAttemptPeerOrProtectedOperator` |
| Operator | `CreateReverificationSuccessor` | `ReverificationSuccessor<Outcome> { source_lifecycle_id, successor_lifecycle_id, lifecycle_event_id }` | `RecoveryRead.ReadLifecycleRecoveryState` by `OriginalAttemptPeerOrProtectedOperator` |
| Operator | `PermanentlyCloseAbandonedLineage` | `PermanentLineageClose<Outcome> { lineage_id, permanent_close_event_id }` | `RecoveryRead.ReadLifecycleRecoveryState` by `OriginalAttemptPeerOrProtectedOperator` |
| Operator | `ApplyLedgerMappingCorrection` | `LedgerMappingCorrection<Outcome> { lifecycle_id, candidate_id, correction_id, mapping_version, ledger_digest }` | `RecoveryRead.ReadLedgerRecoveryState` by `OriginalAttemptPeerOrProtectedOperator` |
| Operator | `IssueRiskOperationIntent` | one exact `RiskOperationIntent<Outcome, OriginalPeerRiskRecovery::<StateActionTag>>` outer variant | Exact action owned by that outer tag; pending and closed-permitted invoke `RecoveryRead.ReadRiskRecoveryState` by `ProtectedOperator` with `FreshForThisRead`, while live, effective-revocation, and closed-forbidden own `NoFurtherAction` |
| Operator | `RequestNewRiskOperationIntent` | one exact `NewRiskOperationIntent<Outcome, OriginalPeerRiskRecovery::<StateActionTag>>` outer variant | Exact action owned by that outer tag under the same rule |
| Operator | `AcceptMajorRisk` | `MajorRiskAcceptance<Outcome> { acceptance_id, candidate_id, issue_ids, acceptance_event_id }` | `NoFurtherAction` |
| Operator | `RevokeMajorRiskAcceptance` | `MajorRiskRevocation<Outcome> { acceptance_id, revocation_id, revocation_event_id }` | `NoFurtherAction` |
| Operator | `ReadLifecycleStatus` | `OperatorLifecycleStatus<Outcome> { lifecycle_id, status_digest }` | `RecoveryRead.ReadLifecycleRecoveryState` by `OriginalAttemptPeerOrProtectedOperator` |
| Operator | `ReadRetentionRecoveryStatus` | `RetentionRecoveryStatus<Outcome> { capacity_generation, blocker_records, integrity_state, migration_telemetry_health: MigrationTelemetryHealthObservation }` | `RecoveryRead.ReadRetentionRecoveryState` by `OriginalAttemptPeerOrProtectedOperator` |
| Operator | `RunControllerRetentionCleanup` | `RetentionCleanup<Outcome> { capacity_generation, cleanup_event_id, eligible_bytes_reclaimed }` | `RecoveryRead.ReadRetentionRecoveryState` by `OriginalAttemptPeerOrProtectedOperator` |
| Operator | `MigrateRetentionCapacity` | `RetentionCapacityMigration<Outcome> { migration_attempt_identity, source_generation, destination_generation, migration_event_id }` | `RecoveryRead.ReadRetentionRecoveryState` by `OriginalAttemptPeerOrProtectedOperator` |
| Assignment issuance | `IssueImplementationAssignment` | `ImplementationAssignmentIssued<Outcome> { assignment_request_id, presented_assignment_alias, assignment_event_id }` | `RecoveryRead.ReadAssignmentRecoveryState { context: FreshAssignmentFlow }` by `OriginalAttemptPeerOrProtectedOperator` |
| Assignment completion | `CompleteImplementationAssignment` | `ImplementationAssignmentCompletionAccepted<Outcome> { presented_assignment_alias, completion_event_id, terminal_intent_digest }` | `RecoveryRead.ReadAssignmentRecoveryState { context: CompletionRecovery }` by `OriginalAttemptPeerOrProtectedOperator`; the returned capacity-release variant owns the pending proof-cancel, controller-release, or terminal-install action until `Completed` is installed |
| Issue reader | `ResolveImplementationAssignment` | `ImplementationAssignmentResolved<Outcome> { presented_assignment_alias, assignment_summary_digest }` | `IssueReader.ReadImplementerIssueView` by `OriginalIssueReaderPeer` |
| Issue reader | `ReadImplementerIssueView` | `ImplementerIssueViewConsumed<Outcome> { presented_assignment_alias, consumed_use_ordinal }` | `RecoveryRead.ReadAssignmentRecoveryState { context: IssueViewRecovery }` by `OriginalAttemptPeerOrProtectedOperator` |
| Issue reader | `ReadMergeAuthorityMajorIssueView` | `MergeAuthorityMajorIssueView<Outcome> { authority_alias, candidate_id, issue_id, view_digest }` | `IssueReader.ReadMergeAuthorityMajorIssueView` by `OriginalIssueReaderPeer` |
| Attempt status | `ReadProtectedAttemptStatus` | `ProtectedAttemptStatusRead<Outcome> { target_attempt_identity, status: SafeProtectedAttemptStatus }`; each risk outcome uses one exact handle-free `OriginalPeerRiskRecovery` state-and-action variant | Exact action owned by the returned outer status tag; a risk pending or closed-permitted tag invokes `RecoveryRead.ReadRiskRecoveryState` by `ProtectedOperator` with `FreshForThisRead`, a risk live, effective-revocation, or closed-forbidden tag owns `NoFurtherAction`, and a non-risk tag owns `AttemptStatus.ReadProtectedAttemptStatus` by `OriginalAttemptPeerOrProtectedOperator` only when reread is its state-valid action |
| Recovery read | `ReadLifecycleRecoveryState` | `LifecycleRecoveryState<Outcome> { target_attempt_identity, lifecycle_id, state_digest }` | `NoFurtherAction` |
| Recovery read | `ReadLedgerRecoveryState` | `LedgerRecoveryState<Outcome> { target_attempt_identity, mapping_version, ledger_digest }` | `NoFurtherAction` |
| Recovery read | `ReadAssignmentRecoveryState` | `AssignmentRecoveryState<Outcome> { target_attempt_identity, context: AssignmentRecoveryContext }` | Exact action owned by the returned `AssignmentRecoveryContext` |
| Recovery read, risk | `ReadRiskRecoveryState` | a freshly authenticated protected operator receives exactly one immediate `ProtectedOperatorRiskRecoveryContext` tag; an original peer is refused with exactly one handle-free `OriginalPeerRiskRecovery` state-and-action variant | Exact action owned by the returned caller-disjoint tag; no broad safe-state field or fixed universal action exists |
| Recovery read | `ReadArtifactRecoveryState` | `ArtifactRecoveryState<Outcome> { target_attempt_identity, artifact_ids, artifact_digests }` | `NoFurtherAction` |
| Recovery read | `ReadRetentionRecoveryState` | `RetentionRecoveryState<Outcome> { target_attempt_identity, capacity_generation, blocker_records, integrity_state, migration_telemetry_health: MigrationTelemetryHealthObservation }` | `NoFurtherAction` |
| Recovery read | `ReadPublicationRecoveryState` | `PublicationRecoveryState<Outcome> { target_attempt_identity, lifecycle_id, candidate_id, state_digest }` | `NoFurtherAction` |
| Recovery read | `ReadOriginalRefusalRecoveryState` | `OriginalRefusalRecoveryState<Outcome> { target_attempt_identity, refusal_product: ExactOperationRefusalProduct, remedy_plan: TypedRemedyPlan }` | `NoFurtherAction` |
| Publisher | `ConsumePublicationManifest` | `PublicationManifestConsumed<Outcome> { lifecycle_id, candidate_id, publication_manifest_id }` | `RecoveryRead.ReadPublicationRecoveryState` by `OriginalAttemptPeerOrProtectedOperator` |
| Publisher | `RecordTrustedMergeCompletion` | `TrustedMergeCompletionRecorded<Outcome> { lifecycle_id, candidate_id, merge_event_id }` | `NoFurtherAction` |
| Publisher | `ReadPublicationStatus` | `PublicationStatusRead<Outcome> { lifecycle_id, candidate_id, status_digest }` | `RecoveryRead.ReadPublicationRecoveryState` by `OriginalAttemptPeerOrProtectedOperator` |

The generator rejects a row whose owned field can be absent for one of its
outcomes; that outcome requires another variant with its own exact product.
The generated matcher is total over every ordinary accepted
`(EndpointOperation, TerminalOutcome::{Success, Refusal})`. Migration
preflight refusals, `RevokeImplementationAssignment`, the five migration
child-command operations, `RekeyMigrationControllerEpoch`,
`RecoverMigrationTelemetryHealth`, and `ReadMigrationRecoveryStatus` have no
standalone accepted attempt and therefore do not appear in this recovery type.
Their dedicated recovery products are respectively assignment revocation
recovery, command recovery, the rekey record, the telemetry health-recovery
slot, and the serialized disclosure slot. Adding an ordinary endpoint
operation, closed success outcome, or
terminal refusal without exactly one generated variant is a schema and
coverage failure. Duplicate rows and wildcard matches also fail generation.
The projection never returns protected text or only opaque digests plus a
generic remedy.

Round-input cleanup runs through the controller's single cleanup entrypoint on
schedule and before admitting new bytes. Full attempt request and response
bytes become eligible only after the attempt is terminal, its sink
acknowledgement is persisted, and its tombstone is durable. A pending
`AcceptedAttemptJournal` or `AuditOutboxRow` is ineligible at every age. The
three raw current-cycle proof records in either assignment-issuance repair
family become eligible only at their atomic accumulator fold. Retained
repair-intent and any remaining current-cycle source bytes remain ineligible
until final activation commits with the applicable permanent repair tombstone
durable; a tombstone preparation is not enough. The
cleanup first compacts acknowledged terminal records, then applies the
30-day and 2-GiB rules only to eligible bytes. When response bytes share the
controller transaction store, deletion and append of
`ReplayPayloadEvicted` commit atomically. A crash before commit leaves bytes
available and no marker; a crash after commit leaves the marker and no bytes.
The controller cannot observe either half-state.

If response bytes must live in storage that cannot join that transaction,
cleanup first appends `ReplayPayloadEvictionPrepared`. Replay treats prepared
payload as unavailable immediately and never serves it. Cleanup then deletes
the bytes and appends `ReplayPayloadEvicted`. A crash before prepare leaves
verified bytes available, a crash after prepare but before deletion leaves
unservable residual bytes, and a crash after deletion leaves at least the
prepared marker, which cannot claim bytes exist. Recovery may finish deletion
and append the final marker but cannot remove either marker. Cleanup cannot
delete or rewrite a tombstone.
The sink's own cleanup may rotate eligible raw event bytes but cannot delete
or rewrite an `AuditAppendTombstone`.

Other round inputs retain D17's 30-day or 2-GiB bound after they become
eligible. The ineligible-record classification is a closed sum type:

```
RetentionBlockerRecord {
  blocker,
  blocker_digest,
  recovery_plan_id,
  recovery_reservation_alias,
  recovery_reservation_digest
}

RetentionBlocker =
  ActiveLifecycle { lifecycle_id }
  UnresolvedLifecycle { lifecycle_id, obligation_ids }
  RetryablePartialDispatch { lifecycle_id, dispatch_id, missing_seat_ids }
  UnavailablePartialDispatch { lifecycle_id, dispatch_id, missing_seat_ids }
  ResumableAbandonedCapsule { lineage_id, capsule_id }
  UnexpiredApprovalReceipt { lifecycle_id, receipt_id, expires_at }
  UnrecordedTrustedMergeCompletion { lifecycle_id, receipt_id, merge_event_id }
  PendingAcceptancePrepare { attempt_identity, prepare_id }
  PendingAcceptedAttempt { attempt_identity }
  PendingAuditOutbox { attempt_identity, audit_event_id }
  PendingAuditConversion {
    attempt_identity,
    reservation_generation,
    conversion: PendingAuditConversionRetention
  }
  PendingAssignmentIssuanceAuditRepair {
    attempt_identity,
    issuance_prepare_alias,
    prepare_incarnation,
    prepared_activation_binding_digest,
    initial_generation,
    current_generation,
    repair_accumulator_root,
    retry_count: SaturatingBoundedCount,
    repair: PendingAssignmentIssuanceAuditRepairRetention
  }
  PendingAssignmentIssuanceCancellationAuditRepair {
    attempt_identity,
    issuance_prepare_alias,
    prepare_incarnation,
    prepared_cancellation_activation_binding_digest,
    initial_generation,
    current_generation,
    repair_accumulator_root,
    retry_count: SaturatingBoundedCount,
    repair: PendingAssignmentIssuanceCancellationAuditRepairRetention
  }
  PendingMigrationAuditRepair {
    migration_attempt_alias,
    reservation_generation,
    repair: PendingMigrationAuditRepairRetention
  }

PendingAuditConversionRetention =
  IntentRecorded { conversion_id }
  | OldGenerationInvalidationPending { conversion_id }
  | OldGenerationInvalidatedRebindPending { conversion_id, invalidation_proof_digest }
  | RefusalGenerationBoundReplacementPending {
      conversion_id,
      invalidation_proof_digest,
      rebind_proof_digest,
      replacement_generation
    }
  | ReplacementTupleInstalled {
      conversion_id,
      conversion_tombstone_digest,
      replacement_generation,
      replacement_event_id
    }

PendingMigrationAuditRepairRetention =
  IntentRecorded { repair_id }
  | OldGenerationInvalidationPending { repair_id }
  | OldGenerationInvalidatedRebindPending {
      repair_id,
      invalidation_proof_digest
    }
  | SuccessGenerationBoundReplacementPending {
      repair_id,
      invalidation_proof_digest,
      rebind_proof_digest,
      replacement_generation
    }
  | ReplacementTupleInstalled {
      repair_id,
      repair_tombstone_digest,
      replacement_generation,
      success_event_alias
    }

PendingAssignmentIssuanceAuditRepairRetention =
  IntentRecorded { repair_id }
  | OldGenerationInvalidationPending {
      repair_id,
      authenticated_definite_no_append_proof_digest
    }
  | OldGenerationInvalidatedRebindPending {
      repair_id,
      authenticated_definite_no_append_proof_digest,
      invalidation_proof_digest
    }
  | ReplacementGenerationRolloverPending {
      repair_id,
      authenticated_definite_no_append_proof_digest,
      invalidation_proof_digest,
      exhausted_generation
    }
  | SuccessGenerationBoundReplacementPending {
      repair_id,
      authenticated_definite_no_append_proof_digest,
      invalidation_proof_digest,
      rebind_or_rollover_proof_digest,
      replacement_generation
    }
  | ReplacementTupleInstalled {
      repair_id,
      replacement_generation,
      unchanged_success_event_id
    }
  | ReplacementSinkAcknowledgementPending {
      repair_id,
      replacement_generation,
      unchanged_success_event_id
    }
  | ReplacementAcknowledgedActivationPending {
      repair_id,
      replacement_generation,
      unchanged_success_event_id,
      final_acknowledgement_digest,
      repair_floor:
        DurableRepairTombstone { tombstone_digest }
        | DurableRepairTombstonePreparation { preparation_digest }
    }

PendingAssignmentIssuanceCancellationAuditRepairRetention =
  IntentRecorded { repair_id }
  | OldGenerationInvalidationPending {
      repair_id,
      authenticated_definite_no_append_proof_digest
    }
  | OldGenerationInvalidatedRebindPending {
      repair_id,
      authenticated_definite_no_append_proof_digest,
      invalidation_proof_digest
    }
  | ReplacementGenerationRolloverPending {
      repair_id,
      authenticated_definite_no_append_proof_digest,
      invalidation_proof_digest,
      exhausted_generation
    }
  | RefusalGenerationBoundReplacementPending {
      repair_id,
      authenticated_definite_no_append_proof_digest,
      invalidation_proof_digest,
      rebind_or_rollover_proof_digest,
      replacement_generation
    }
  | ReplacementTupleInstalled {
      repair_id,
      replacement_generation,
      unchanged_cancellation_refusal_event_id
    }
  | ReplacementSinkAcknowledgementPending {
      repair_id,
      replacement_generation,
      unchanged_cancellation_refusal_event_id
    }
  | ReplacementAcknowledgedActivationPending {
      repair_id,
      replacement_generation,
      unchanged_cancellation_refusal_event_id,
      final_acknowledgement_digest,
      repair_floor:
        DurableRepairTombstone { tombstone_digest }
        | DurableRepairTombstonePreparation { preparation_digest }
    }
```

There is no `Other` blocker. Every ineligible round-input record maps to
exactly one variant; a record that does not is
`retention-classification-incomplete` and stops admission. Atomic lineage
transitions expose no durable half-import record. Specialized variants take
precedence: a partial dispatch is never also `ActiveLifecycle`, a resumable
capsule is never `UnresolvedLifecycle`, a trusted but unrecorded merge is
`UnrecordedTrustedMergeCompletion` rather than
`UnexpiredApprovalReceipt`, and an attempt record moves from
`PendingAcceptancePrepare` to `PendingAcceptedAttempt` only at authoritative
promotion, then to `PendingAuditOutbox` when its terminal state transaction
commits. A durable conversion intent takes precedence over
`PendingAuditOutbox` and changes that same blocker key to
`PendingAuditConversion`, but only after the exact old tuple has already
reached `OrdinarySinkAcknowledgementPending`; no retention transition exists from
`QuarantinedPendingAudit`. Its key is exactly the `AttemptIdentity` and old
reservation generation. The conversion state advances monotonically through
the nested variants above. The replacement-activation transaction creates the
immutable `AuditConversionTombstone`, makes the intent and proof bytes
eligible, and advances the blocker to `ReplacementTupleInstalled`. The next
append transition changes it to `PendingAuditOutbox` under the replacement
generation. For `IssueImplementationAssignment`, the same
definite-no-append observation selects
`PendingAssignmentIssuanceAuditRepair` for a prepared success or
`PendingAssignmentIssuanceCancellationAuditRepair` for the canonical
prepared-cancellation refusal. Each retains its exact prepared state,
evidence, request, and controller and sink capacity bindings and can never be
represented as `PendingAuditConversion`. The blocker remains specialized
through replacement append, acknowledgement persistence, and final
activation. An authenticated definite-no-append result for any replacement
generation moves the same blocker back to its invalidation/rebind cycle with
the same fixed workspace, accumulator, and reserved capacity. A finite
generation at its no-wrap boundary uses only the nested rollover state and
action. The final activation accepts only the exact proof-bound
acknowledgement and durable repair tombstone or preparation, materializes the
tombstone when necessary, and requires that floor to bind the latest committed
accumulator root and retry count. Each earlier fold has already made exactly
its own definite-no-append, invalidation, and rebind-or-rollover proof bytes
eligible; the blocker keeps all three proofs of any unfolded current cycle
ineligible and reserved. The blocker remains through final activation;
retained repair-intent and any remaining current-cycle source bytes become
eligible only after that activation has made the applicable permanent repair
tombstone durable. For
`MigrateRetentionCapacity`, the same observation selects
`PendingMigrationAuditRepair` instead. That blocker is internally
keyed by the exact migration `AttemptIdentity` and old reservation generation
but serializes only the safe migration-attempt alias. It retains its sealed
execution and control reserves and can never be represented as
`PendingAuditConversion`.
`ActiveLifecycle` covers other nonterminal work actively advancing;
`UnresolvedLifecycle` covers other parked named obligations before terminal
transition. The section 12 source partial-round bytes are
`RetryablePartialDispatch` or `UnavailablePartialDispatch` until atomic
successor import, then become eligible immediately. Section 11's trusted
merge-completion event or receipt expiry makes receipt-bound terminal inputs
eligible. Permanent close makes a resumable capsule eligible.

Every operation that can create one of these variants must first reserve a
bounded `RecoveryCapacityReservation` in the same authority transaction. The
reservation binds the blocker key and variant, blocker digest, schema version,
exact closed recovery-plan id, and a mechanically generated vector of maximum
additional record slots and bytes needed for that plan to reach eligibility.
For `PendingAssignmentIssuanceAuditRepair` and
`PendingAssignmentIssuanceCancellationAuditRepair`, the reservations are
bound to and preallocated with the issuance prepare before ordinary append
authorization, including the applicable permanent repair-tombstone slot; the
generic conversion reserve cannot be substituted. For
`PendingMigrationAuditRepair`, that
`RecoveryCapacityReservation` is a pre-sealed component of the active
migration's `MigrationControlReserve`; it never allocates from the general
recovery partition after the migration has accepted.
`recovery_reservation_digest` covers that allocation vector and blocker
binding before the plan id is inserted. The exact `recovery_plan_id` is then
derived from the domain separator
`d2b:panel:retention-recovery-plan:v1`, schema version, blocker digest,
recovery-reservation digest, and the complete ordered closed operation vector.
The reservation stores that plan id, and `RetentionBlockerRecord` serializes
the same blocker digest, plan id, domain-separated non-capability reservation
alias, and reservation digest. This avoids a digest cycle while binding the
plan to both the exact blocker and exact reserved capacity.
The
generator sums the serialized maxima of every state-valid journal, outbox,
metric, capsule, crosswalk, marker, and tombstone schema on the route; an
unbounded or unknown schema makes the operation inadmissible. A transition to
another blocker atomically consumes the current reservation and installs the
recomputed reservation for the next variant. Eligibility releases any
remainder.

The plan cannot assume that a future discovery produces a bounded total
finding set. It may roll capacity through a finite list of already named
obligations, or reserve the complete explicit
`AbandonLifecycle`-then-`PermanentlyCloseAbandonedLineage` route when that is
the only bounded state-valid path. That irreversible route is named in the
blocker record and requires the protected operator operation; abandonment
alone never satisfies the plan or releases capacity. If the operator declines
the named bounded route, new admission remains denied.

Recovery capacity is physically or transactionally partitioned from ordinary
admission. Normal work cannot borrow it, cleanup cannot count it as free
general capacity, and a general store at its byte and entry maxima still
admits the reservation-bound recovery transitions. Creation without the full
reservation is `recovery-reserve-unavailable` and creates neither the
ineligible record nor its authority effect.

`round-input-store-full` carries the complete sorted list of
`RetentionBlockerRecord` values with all listed safe ids, each reservation id
and exact plan id, and the configured general and recovery bounds. "Reservation
id" in this sentence and every projection means the serialized
`recovery_reservation_alias`; a raw reservation handle is never emitted. Its normal
closed remedy executes those named plan ids in order and then runs cleanup; it
does not first ask the operator to rediscover blockers. If a later
post-eviction or public projection is marked `BlockerDetailsRedacted` or
`BlockerDetailsStale`, and only then, its generated plan first calls
`ReadRetentionRecoveryStatus`, takes the returned current named plan ids, and
executes them. A projection lacking either complete current details or one of
those two closed markers is invalid.

| Blocker | Ordered capacity remedy |
| --- | --- |
| `ActiveLifecycle` | `ExecuteNamedReservedLifecycleEligibilityPlan`, then `RunControllerRetentionCleanup` |
| `UnresolvedLifecycle` | `ExecuteNamedReservedLifecycleEligibilityPlan`, then `RunControllerRetentionCleanup` |
| `RetryablePartialDispatch` | `CompletePinnedLegacyRound`, then `RunControllerRetentionCleanup` |
| `UnavailablePartialDispatch` | `CreateSameScopeCurrentSchemaSuccessor`, then `RunControllerRetentionCleanup` |
| `ResumableAbandonedCapsule` | `ChooseResumableLineageDisposition { resume, supersede, permanently_close }`, then `ApplyChosenResumableLineageDisposition`, then `RunControllerRetentionCleanup` |
| `UnexpiredApprovalReceipt` | `WaitForApprovalReceiptExpiry`, then `RunControllerRetentionCleanup` |
| `UnrecordedTrustedMergeCompletion` | `RecordResolvedTrustedMergeCompletion`, then `RunControllerRetentionCleanup` |
| `PendingAcceptancePrepare` | `CompleteOrCancelNamedAcceptancePrepare`, then `RunControllerRetentionCleanup` |
| `PendingAcceptedAttempt` | `RecoverPendingAcceptedAttempt`, then `RunControllerRetentionCleanup` |
| `PendingAuditOutbox` | `RestoreProtectedAuditSink`, then `ReplayPendingAuditAppend`, then `RunControllerRetentionCleanup` |
| `PendingAuditConversion` | `ExecuteNamedPendingAuditConversionPlan`, whose exact next action is selected by its nested state, then `RunControllerRetentionCleanup` |
| `PendingAssignmentIssuanceAuditRepair` | execute only the exact action nested in `PendingAssignmentIssuanceAuditRepairRetention` through unchanged-success acknowledgement and `ActivatePreparedImplementationAssignment`, then `RunControllerRetentionCleanup` |
| `PendingAssignmentIssuanceCancellationAuditRepair` | execute only the exact action nested in `PendingAssignmentIssuanceCancellationAuditRepairRetention` through unchanged-refusal acknowledgement and `ActivatePreparedAssignmentIssuanceCancellation`, then `RunControllerRetentionCleanup` |
| `PendingMigrationAuditRepair` | `Operator.RepairMigrationSinkAppend` by `ProtectedOperator` until append acknowledgement, then `Operator.CompleteMigrationAuditActivation` by `ProtectedOperator`, then `RunControllerRetentionCleanup` |

The named pending-conversion plan is closed and state-specific:

| Pending conversion state | Exact action |
| --- | --- |
| `IntentRecorded` | `IssueNamedOldGenerationInvalidation` |
| `OldGenerationInvalidationPending` | `ReplayNamedOldGenerationInvalidation` |
| `OldGenerationInvalidatedRebindPending` | `BindNamedReplacementRefusalGeneration` |
| `RefusalGenerationBoundReplacementPending` | `CommitNamedReplacementRefusalTuple` |
| `ReplacementTupleInstalled` | `ReplayNamedReplacementRefusalAppend` |

Migration audit-repair state is also closed and state-specific:

| Pending migration repair state | Reachable operation and caller |
| --- | --- |
| `IntentRecorded` | `Operator.RepairMigrationSinkAppend` by `ProtectedOperator`, issuing the named old-generation invalidation |
| `OldGenerationInvalidationPending` | `Operator.RepairMigrationSinkAppend` by `ProtectedOperator`, replaying the named invalidation |
| `OldGenerationInvalidatedRebindPending` | `Operator.RepairMigrationSinkAppend` by `ProtectedOperator`, binding the unchanged success event to the replacement generation |
| `SuccessGenerationBoundReplacementPending` | `Operator.RepairMigrationSinkAppend` by `ProtectedOperator`, committing the proof-bound replacement tuple |
| `ReplacementTupleInstalled` | `Operator.RepairMigrationSinkAppend` by `ProtectedOperator`, replaying the unchanged success append |

Assignment-issuance audit-repair state is closed and state-specific:

| Pending issuance repair state | Exact action |
| --- | --- |
| `IntentRecorded` | `IssueAssignmentIssuanceOldGenerationInvalidation` |
| `OldGenerationInvalidationPending` | `ReplayAssignmentIssuanceOldGenerationInvalidation` |
| `OldGenerationInvalidatedRebindPending` | `BindAssignmentIssuanceSuccessReplacementGeneration` |
| `ReplacementGenerationRolloverPending` | `RolloverAssignmentIssuanceSuccessReplacementGeneration` |
| `SuccessGenerationBoundReplacementPending` | `CommitAssignmentIssuanceSuccessReplacementTuple` |
| `ReplacementTupleInstalled` | `ReplayAssignmentIssuanceSuccessAppend` |
| `ReplacementSinkAcknowledgementPending` | `QueryOrReplayAssignmentIssuanceSuccessAppend`; persist the proof-bound acknowledgement, or on authenticated definite-no-append re-enter the same invalidation/rebind loop |
| `ReplacementAcknowledgedActivationPending` | `ActivatePreparedImplementationAssignment` with only the state-bound final acknowledgement, accumulator, retry count, and durable repair tombstone or preparation |

Prepared-cancellation audit-repair state is closed and state-specific:

| Pending prepared-cancellation repair state | Exact action |
| --- | --- |
| `IntentRecorded` | `IssueAssignmentIssuanceCancellationOldGenerationInvalidation` |
| `OldGenerationInvalidationPending` | `ReplayAssignmentIssuanceCancellationOldGenerationInvalidation` |
| `OldGenerationInvalidatedRebindPending` | `BindAssignmentIssuanceCancellationRefusalReplacementGeneration` |
| `ReplacementGenerationRolloverPending` | `RolloverAssignmentIssuanceCancellationRefusalReplacementGeneration` |
| `RefusalGenerationBoundReplacementPending` | `CommitAssignmentIssuanceCancellationRefusalReplacementTuple` |
| `ReplacementTupleInstalled` | `ReplayAssignmentIssuanceCancellationRefusalAppend` |
| `ReplacementSinkAcknowledgementPending` | `QueryOrReplayAssignmentIssuanceCancellationRefusalAppend`; persist the proof-bound acknowledgement, or on authenticated definite-no-append re-enter the same invalidation/rebind loop |
| `ReplacementAcknowledgedActivationPending` | `ActivatePreparedAssignmentIssuanceCancellation` with only the state-bound final acknowledgement, accumulator, retry count, and durable cancellation-repair tombstone or preparation |

After the acknowledgement is persisted,
`Operator.CompleteMigrationAuditActivation` by `ProtectedOperator` performs
the normal atomic capacity-switch activation. Both operations are scoped to
the one active migration and consume only its sealed
`MigrationControlReserve`.

Each generic conversion action owns the blocker record's `AttemptIdentity`, old reservation
generation, conversion id, and any replacement generation or proof digest
present in that one state. No action accepts a caller-supplied generation.
After an invalidation proof exists, the old generation is represented by a
non-appendable `InvalidatedReservationGeneration` type; only the proof-bound
`ReplacementReservationGeneration` can construct a later append action.
Each assignment-issuance repair action owns the exact `AttemptIdentity`,
prepare identity and incarnation, unchanged success or cancellation-refusal
event, fixed workspace, initial and current generations, accumulator root,
saturating retry count, and only the current-cycle proof digests,
proof-bound final acknowledgement, or repair-floor digest in its nested
state. No caller can supply or substitute any of them. The replacement append
query has exactly three outcomes: unknown remains in the same state,
acknowledged enters its activation-pending state with the exact acknowledgement
and tombstone preparation, and authenticated definite-no-append enters the
same family's invalidation loop without allocating another workspace or
capacity reservation.
Each migration repair action instead owns the blocker record's migration
attempt alias, repair id, unchanged success event alias and digest, and only
the generations and proof digests present in its exact nested state.

For `UnavailablePartialDispatch`,
`CreateSameScopeCurrentSchemaSuccessor` means creating a fresh protected
attempt that targets the blocker record's exact
`LogicalSuccessorImportId`. It never redispatches the unavailable reviewer,
changes the blocker back to `RetryablePartialDispatch`, or reuses the failed
attempt identity.

The error never defaults to a reviewed bound increase or ordinary
abandonment. It does not evict active state, drop descriptions, or degrade to
an incomplete reviewer payload. Every blocker must name its own remedy before
admission is retried.

`ReadRetentionRecoveryStatus` and
`RecoveryRead.ReadRetentionRecoveryState` return the complete current
`RetentionBlockerRecord` products when details are current: safe blocker ids,
blocker digests, exact plan ids, reservation aliases and digests, schema
versions, reserved and consumed bounded numerics, and an integrity state. If
the blocker is `PendingAuditConversion`, its returned blocker is the exact
nested retention variant above and its plan expands to only that variant's
exact action; conversion state, generation, proof fields, and action cannot be
combined independently. Both assignment-issuance repair blockers have the
same exact nested-product rule and render only their state-owned success or
cancellation action. `PendingMigrationAuditRepair` has the same rule and can
render only its two protected operator operations.
If
the controller cannot prove that every blocker has the exact generated
reservation, it enters `RecoveryReserveIntegrityCorrupt`, stops normal
admission, and permits only authenticated status, already-reserved recovery,
and `MigrateRetentionCapacity`. It must not guess, reclaim a reservation, or
continue on warning.

The capacity system exposes seven disjoint partitions. Items 1 through 4 are
capacity-generation-owned, items 5 and 6 are controller-wide and
fixed-cardinality, and item 7 is transient:

1. `MigrationExecutionReserve` is the complete maximum serialized allocation
   for exactly one executable, non-conflict `MigrateRetentionCapacity`
   logical operation. It includes its prepare, accepted journal, migration
   work and pause state, request and replay result, outbox, sink reservation
   and raw event, acknowledgement, controller and sink tombstones, both
   payload-eviction markers, and the maximum migration audit-repair intent,
   invalidation proof, rebind proof, and repair tombstone. It contains no
   generic refusal-conversion allocation. It also seals two disjoint sibling
   reserves. Each `MigrationControlReserve` incarnation contains exactly one
   fixed reusable command slot for each resume, fence, sink-repair, and
   audit-activation operation that the one active migration can require, plus
   fixed child-audit outbox and sink-reservation workspace.
   `MigrationIntegrityReserve` contains one fixed reusable command slot for
   control-reserve repair, one separately sealed non-child rekey recovery
   record, one fixed reusable per-disclosure status-audit slot, and the repair
   command's fixed child-audit workspace. It contains no per-state,
   per-request, per-cycle, or permanent child-command allocation.
   `MigrationIntegrityReserve` is sufficient for every
   `ReadMigrationRecoveryStatus` and
   `RepairMigrationControlReserve` operation, every required
   `RekeyMigrationControllerEpoch`, and a complete verified replacement of
   the control reserve. Neither reserve can address or borrow the other. The
   command slots, child-audit workspace, status-audit slot, non-resettable
   rekey recovery record, its generated maximum continuation inventory and
   outcome-event workspace, and integrity-repair workspace are fixed capacity,
   not estimates,
   append-only request history, or transient lanes.
2. `ProtectedStatusReserve` is a separate bounded allocation for
   `ReadLifecycleStatus`, `ReadProtectedAttemptStatus`,
   `ReadRetentionRecoveryStatus`, `ReadPublicationStatus`, and every
   recovery-read operation, except the exact marker-driven retention recovery
   read charged to its blocker reservations and active-migration status charged
   to `MigrationIntegrityReserve`. Byte-identical
   retries and concurrent duplicates for one authenticated peer, operation,
   target, idempotency key, and observed state coalesce onto one accepted
   status attempt. A distinct status attempt consumes only this partition.
   Exhaustion is the preflight refusal `protected-status-budget-exhausted`;
   it never borrows migration or blocker-recovery capacity.
3. `AcceptedConflictReserve` is the separate bounded permanent-record
   allocation for audited `AttemptIdentity::Conflict` attempts of every
   operation except `MigrateRetentionCapacity` and the active migration's
   control operations. Identical conflict retries
   coalesce on their tombstone. Exhaustion is the preflight refusal
   `accepted-conflict-budget-exhausted`; it cannot charge the base attempt,
   status reserve, recovery reserve, or migration execution reserve.
4. `MigrationPreflightSignalReserve` is a separate bounded durable diagnostic
   allocation for migration replay-conflict signals. It cannot be addressed by
   accepted attempts, status reads, blocker recovery, migration execution, or
   either migration control or integrity reserve.
5. `MigrationControlConflictSignalReserve` is a separate controller-wide
   fixed-cardinality
   durable diagnostic allocation for authenticated stale-generation and
   changed-byte migration-control refusals. It cannot be addressed by control
   slots, integrity repair, accepted attempts, status, blocker recovery,
   migration execution, or the preflight signal reserve.
6. `MigrationTelemetryHealthMarkerReserve` is a controller-wide, independently
   integrity-checked fixed current-and-shadow marker, separately reserved
   durable failure latch, independently sealed current failure-alias record,
   and separately sealed telemetry-recovery slot with fixed audit workspace
   for controller-wide migration telemetry health.
   Marker sequence numbers increase monotonically and the same slots are
   reused across every signal window, control-reserve incarnation, rotation,
   and capacity cutover. No signal, aggregate, exporter, status, or migration
   operation can allocate another marker, latch, or recovery slot.
7. The transient emergency migration lane holds only preflight ownership and
   the live execution lease. It creates no substitute for any permanent
   allocation.

`accepted-conflict-budget-exhausted` carries exactly one closed operation
class:

```
AcceptedConflictOperationClass =
  RiskOperation {
    target_attempt_identity,
    operation:
      IssueRiskOperationIntent
      | RequestNewRiskOperationIntent
      | AcceptMajorRisk
      | RevokeMajorRiskAcceptance
  }
  | CallerKeyOperation {
      endpoint,
      operation
    }
```

`CallerKeyOperation` is constructible only for an endpoint operation whose
schema permits a caller-selected idempotency key. Its remedy invokes that
same endpoint operation through its original authorized caller with a fresh
key. `RiskOperation` instead invokes
`RecoveryRead.ReadRiskRecoveryState` through an authenticated protected
operator; an original attempt peer must reauthenticate as that class before a
pending handle is returned. Only its `ClosedMutationPermitted` state renders
`Operator.RequestNewRiskOperationIntent`; pending intent, live acceptance,
effective revocation, and closed-forbidden states render only their exact
state-valid action or `NoFurtherAction`. No risk variant,
controller-issued-key operation, or unknown class can render a universal
fresh-caller-key retry.

Migration control is a typed child-command protocol of the one accepted
migration. It uses fixed reusable slots, not standalone accepted attempts,
per-state identities, or permanent child-command history:

```
MigrationControlCommand =
  Mutation(
    ResumeProtectedAttempt
    | FenceProtectedAttempt
    | RepairMigrationSinkAppend
    | CompleteMigrationAuditActivation
  )
  | Integrity(RepairMigrationControlReserve)

MigrationControlCommandRequest = {
  migration_attempt_alias,
  controller_epoch,
  expected_migration_state_generation,
  operation: MigrationControlCommand,
  canonical_operation_payload,
  caller_key: IgnoredNonsemanticTransportField
}

MigrationControlCommandId =
  digest(
    "d2b:panel:migration-control-child:v1",
    MigrationAttemptIdentity,
    MigrationControllerEpoch,
    MigrationControlReserveIncarnation,
    MigrationControlCommand,
    pre_state_generation,
    child_sequence
  )

MigrationChildCommandAuditEventId =
  digest(
    "d2b:panel:migration-control-child-audit:v1",
    MigrationAttemptIdentity,
    MigrationControllerEpoch,
    MigrationControlReserveIncarnation,
    MigrationControlCommand,
    pre_state_generation,
    child_sequence,
    child_audit_sequence
  )

MigrationChildAuditPrepareIdentity =
  digest(
    "d2b:panel:migration-child-audit-prepare:v1",
    MigrationAttemptIdentity,
    MigrationControllerEpoch,
    MigrationControlReserveIncarnation,
    FixedCommandSlotRole,
    MigrationControlCommandId,
    ControllerIssuedPrepareNonce
  )
```

The four mutation slots live in `MigrationControlReserve`. The
control-reserve repair slot lives in `MigrationIntegrityReserve`; repairing
the control reserve therefore never depends on a slot inside the reserve being
repaired. Epoch rekey is not a `MigrationControlCommand`, does not claim a
command or refusal subslot, and does not consume child sequence, child audit
sequence, slot record generation, or child-audit capacity. It uses the
dedicated non-resettable rekey record defined below. Every child-command slot
has the same reusable lifecycle:

```
MigrationControlCommandSlot = {
  slot_record_generation,
  primary: MigrationControlCommandSubstate,
  changed_bytes_refusal: FixedRefusalSubslot,
  last: None | MigrationControlCommandLastResult
}

MigrationControlCommandSubstate =
  Available
  | PreparingAuditCapacity {
      command_id,
      prepare_identity,
      controller_epoch,
      reserve_incarnation,
      pre_state_generation,
      child_sequence,
      child_audit_sequence,
      canonical_request_digest,
      owner_epoch,
      lease_until,
      deadline,
      required_capacity,
      prepare_digest
    }
  | Claimed {
      command_id,
      controller_epoch,
      reserve_incarnation,
      pre_state_generation,
      child_sequence,
      child_audit_sequence,
      canonical_request_digest,
      owner_epoch,
      lease_until,
      deadline,
      controller_outbox_reservation_alias,
      sink_reservation_alias
    }
  | ExternalEffectPending {
      claimed_fields,
      external_effect_kind,
      external_effect_identity
    }
  | ExternalEffectObserved {
      claimed_fields,
      external_effect_kind,
      external_effect_identity,
      observation_digest
    }
  | AuditAppendPending {
      claimed_fields,
      exact_success_or_refusal,
      audit_event_id,
      audit_event_digest,
      child_audit_outbox_slot,
      child_audit_sink_reservation
    }
  | AuditAcknowledged {
      claimed_fields,
      exact_success_or_refusal,
      audit_event_id,
      audit_event_digest,
      acknowledgement_digest
    }

MigrationControlCommandLastResult = {
  command_id,
  controller_epoch,
  reserve_incarnation,
  operation,
  pre_state_generation,
  canonical_request_digest,
  exact_success_or_refusal,
  audit_event_digest
}
```

`FixedRefusalSubslot` has the same claim, worker, audit-pending,
audit-acknowledged, and last-result fields, including
`PreparingAuditCapacity`, but can settle only
`migration-control-replay-conflict`. It lets one changed-byte command be
audited without overwriting a live primary command. An identical conflict
coalesces there. A further distinct conflict waits for that fixed subslot or
is refused before child-command admission as
`migration-control-refusal-subslot-occupied`; it cannot allocate another slot
and cannot render an outbox or sink-capacity repair. Its only remedy is to wait
for the current refusal settlement and reread current migration status. Stale
and ordinary prerequisite refusals use the primary slot when it is available.
Caller keys never select a slot, affect identity, or survive in authority
state.

Before a request becomes a child command, the controller authenticates
`ProtectedOperator`, strictly decodes the bounded operation, resolves the
active migration, and validates the slot's required fixed controller and sink
capacity shape and current availability. Failure at any of those steps creates
no slot prepare, child command, reservation, or privileged effect. Current
capacity unavailability returns
`MigrationChildAuditCapacityRecovery::PrePrepareCapacityUnavailable`, which
contains no prepare id, alias, or digest and routes through fresh current
migration status. It then atomically reserves the
next child and child-audit sequences and compare-and-swaps the owning primary
or refusal subslot to `PreparingAuditCapacity`, binding the command identity,
immutable `MigrationChildAuditPrepareIdentity`, owner epoch, lease, deadline,
exact required capacity, canonical request digest, and prepare digest before
contacting either capacity owner. The controller-outbox and sink reservations
bind only the immutable command id, fixed primary-or-refusal slot role,
prepare identity, and prepare digest:

```
MigrationChildAuditReservationBinding = {
  command_id,
  fixed_slot_role,
  prepare_identity,
  prepare_digest
}
```

Worker epoch, lease, deadline, slot record generation, and every later
takeover value are excluded from that reservation binding. They govern who
may execute the prepare, not what capacity belongs to it. No child-audit
capacity can exist without that durable immutable prepare owner.

After both reservations are verified, one compare-and-swap changes the same
prepare to `Claimed`. A reservation failure leaves the prepare nonterminal
until reconciliation either adopts all matching reservations or obtains
proof-backed cancellation for every reservation that was created and returns
the slot to `Available`. While that work remains, recovery returns
`PrepareReconciliationPending` with only the non-capability prepare alias,
prepare digest, and exact reservation-state codes. After adoption completes,
the command continues from the same prepare. After proof-backed cancellation,
recovery returns `ReconciledAndAvailable` and the slot is available. It never
admits a privileged effect from a cancelled prepare. Time alone is not
cancellation proof.
Startup and lease-expiry recovery scan every primary and refusal prepare,
query both capacity owners by prepare digest, adopt matching orphan
reservations, or first mark the prepare non-adoptable and proof-cancel each
orphan. Takeover changes only execution ownership. The new worker presents a
controller proof of the same immutable prepare and adopts the same
reservations without rebinding them. Reservation creation and adoption also
require the current execution-ownership proof; capacity created by a stale
worker after takeover is non-adoptable and must be proof-cancelled. A stale
worker cannot reserve, adopt, cancel, promote, or make its reservation usable
after the ownership compare-and-swap. Thus a crash before either reservation,
between reservations, or after both reservations but before `Claimed` has one
durable prepare identity and one recovery path.

```
MigrationChildAuditCapacityRecovery =
  RefusalSubslotOccupied {
    migration_attempt_alias,
    controller_epoch,
    safe_operation,
    refusal_subslot_generation,
    current_refusal_alias,
    deadline,
    next: RemedyPlan [
      WaitForMigrationRefusalSubslotSettlement,
      ReadCurrentMigrationRecoveryStatus
    ]
  }
  | PrePrepareCapacityUnavailable {
    migration_attempt_alias,
    controller_epoch,
    safe_operation,
    unavailable_capacity_role,
    required_capacity,
    available_capacity,
    next: Operator.ReadMigrationRecoveryStatus
  }
  | PrepareReconciliationPending {
      migration_attempt_alias,
      controller_epoch,
      safe_operation,
      prepare_alias,
      prepare_digest,
      controller_reservation_state,
      sink_reservation_state,
      next: ReconcileMigrationChildAuditPrepare
    }
  | ReconciledAndAvailable {
      migration_attempt_alias,
      controller_epoch,
      safe_operation,
      cancelled_prepare_alias,
      cancellation_proof_digest,
      next: Operator.ReadMigrationRecoveryStatus
    }
```

After `RefusalSubslotOccupied`, the declared two-action `RemedyPlan` first
waits for that exact subslot generation to settle, then renders
`ReadCurrentMigrationRecoveryStatus` as
`Operator.ReadMigrationRecoveryStatus` with the causing migration alias.
After either
`PrePrepareCapacityUnavailable` or `ReconciledAndAvailable`, the caller must
use the operation returned by the fresh status tag. No generated remedy
blindly resubmits the old canonical request bytes. Only
`PrepareReconciliationPending` can invoke prepare adoption or cancellation.

After the `Claimed` boundary, every successful or refused
privileged command has one distinct canonical append-only
`MigrationChildCommandAuditEvent`, including stale-generation, ineligible,
tuple-mismatch, prerequisite, changed-byte, and integrity-repair outcomes. Its
event id uses the migration attempt, controller epoch, reserve
incarnation, operation, pre-state generation, and the fixed-slot child and
child-audit sequences above. Rekey uses only
`MigrationEpochRekeyOutcomeEventId` and its dedicated state machine. The
controller sends no child success or refusal response
until the event or its outbox is durable. The sink keeps the event under
D17's bounded audit retention; the controller keeps no permanent tombstone per
child command.

The controller then orders an admitted child command as follows:

1. integrity-check the current migration journal, controller epoch, reserve
   incarnation, slot record generation, and the applicable fixed slot;
2. reserve the next child and child-audit sequences and compare-and-swap the
   slot to `PreparingAuditCapacity`, bind and verify both prepare-owned
   reservations, then compare-and-swap that exact prepare to `Claimed`;
3. compare the presented epoch and expected generation with current state.
   An old epoch is `migration-control-epoch-stale`. A lower or higher
   generation is `migration-control-state-stale` with `Past` or `Future`.
   Both are exact audited child refusals and return current status as the only
   reconstruction source;
4. at a matching generation, validate that the operation is owned by the
   exact tagged state and validate every prerequisite and safe tuple field.
   Ineligible, tuple-mismatch, and prerequisite failures are exact audited
   child refusals. A corrected request claims the reused slot with a new child
   sequence; it is not a replay attempt;
5. for `ResumeProtectedAttempt`, `FenceProtectedAttempt`, and
   `CompleteMigrationAuditActivation`, the slot claim and internal mutation
   may be one transaction, proceeding directly to `AuditAppendPending`.
   `RepairMigrationSinkAppend` and `RepairMigrationControlReserve` must use the
   explicit
   `ExternalEffectPending` and `ExternalEffectObserved` states with an
   idempotent effect identity before they can enter `AuditAppendPending`;
6. append or recover the one child audit event, persist its acknowledgement,
   and compare-and-swap `AuditAcknowledged` to `Available`, copying only the
   current last result and audit digest into `last`; and
7. only after audit settlement return the exact result. Telemetry is the
   separate best-effort follow-up below and never delays this response.

An identical concurrent request joins the current command and receives the
same result. Different bytes while the primary command owns the same epoch,
incarnation, operation, and pre-state generation use the fixed refusal
subslot and return audited `migration-control-replay-conflict`. After a state
advance, even byte-identical old bytes are audited
`migration-control-state-stale`, not replay. After response loss, resubmitting
the exact request consults the current or last fixed-slot result and returns
exactly one of:

```
MigrationControlCommandRecovery =
  InProgress { progress: MigrationControlCommandProgressProjection }
  | AlreadyApplied {
      command_alias,
      exact_last_result,
      audit_event_digest,
      current_migration_status
    }
  | CurrentRefusal {
      command_alias,
      exact_current_refusal,
      audit_event_digest,
      current_migration_status
    }
  | SupersededByCurrentState { current_migration_status }

MigrationControlCommandProgressProjection =
  Preparing {
    command_alias,
    safe_operation,
    lease_until,
    deadline,
    state_generation_relation: MigrationStateGenerationRelation
  }
  | Claimed {
      command_alias,
      safe_operation,
      lease_until,
      deadline,
      state_generation_relation: MigrationStateGenerationRelation
    }
  | ExternalEffectPending {
      command_alias,
      safe_operation,
      lease_until,
      deadline,
      state_generation_relation: MigrationStateGenerationRelation
    }
  | ExternalEffectObserved {
      command_alias,
      safe_operation,
      lease_until,
      deadline,
      state_generation_relation: MigrationStateGenerationRelation,
      effect_digest
    }
  | AuditAppendPending {
      command_alias,
      safe_operation,
      lease_until,
      deadline,
      state_generation_relation: MigrationStateGenerationRelation,
      audit_digest
    }
  | AuditAcknowledged {
      command_alias,
      safe_operation,
      lease_until,
      deadline,
      state_generation_relation: MigrationStateGenerationRelation,
      audit_digest
    }

MigrationStateGenerationRelation =
  AtExpectedGeneration
  | CurrentGenerationPastCommand
  | CurrentGenerationFutureCommand
```

It never invokes generic protected-attempt replay, reconstructs a replay
payload, or creates a tombstone. `AlreadyApplied` is returned only when the
current journal proves the named command's post-state; otherwise recovery
returns the exact current refusal or current state and action.
`ReadMigrationRecoveryStatus` independently returns that current state and
action on every call.

`MigrationControlCommandProgressProjection` is the complete public progress
surface. It has no `state_tag` field and no optional digest. The effect digest
is owned only by `ExternalEffectObserved`; the audit digest is owned only by
`AuditAppendPending` and `AuditAcknowledged`. Strict construction rejects a
digest in any other tag, either digest omitted from its owning tag, or any
cross-variant field substitution. A raw command id, owner epoch, controller or
sink reservation, external-effect identity, outbox identity or bytes, sink
handle, fencing value, slot record generation, and every unlisted field are
absent from callers, logs, status, refusal recovery, and `Debug`.

Every non-available substate carries the original owner epoch, lease, deadline,
and slot record generation. Startup scans all five command slots and their
refusal subslots before normal control traffic. A live lease is left alone.
After expiry, recovery compare-and-swaps the exact slot generation to a new
owner epoch, queries an idempotent external effect by
`external_effect_identity` when applicable, and resumes only the recorded
next substate. A stale owner cannot write any effect observation, migration
state, audit row, acknowledgement, or result. A crash after claim therefore
cannot strand a slot, and a crash at every external-effect, outbox, sink-fsync,
acknowledgement, or settlement boundary has one compare-and-swap continuation.

The public worker-fenced refusal does not expose either owner epoch, any lease
token, or `slot_record_generation`. It carries the command alias, the closed
safe relation `PresentedWorkerSupersededByCurrentOwnership`, and a digest of
the current continuation. That relation and digest are sufficient to stop the
stale worker and re-read recovery without publishing a fencing value.

The exact resettable counter set is closed:

```
MigrationResettableCounterKind =
  MigrationStateGeneration
  | ControlReserveIncarnation
  | ChildSequence
  | ChildAuditSequence
  | SlotRecordGeneration
  | TelemetryMarkerSequence
```

Each is an unsigned 64-bit integer and never wraps. `ChildSequence` and
`ChildAuditSequence` are controller-owned migration-wide counters within an
epoch, not caller values or per-slot counters. `SlotRecordGeneration` covers
the generation on every primary and refusal subslot. No seventh resettable
counter or alternate name is permitted. Their current values are interpreted
only with `MigrationControllerEpoch`, which is a controller-calculated 256-bit
chain digest over the migration attempt, previous epoch, successful rekey
audit event, and primary outcome nonce. Old-epoch requests are stale even
when every reset counter value matches.

Every command, command audit, audit-repair, control-repair, conflict signal,
status disclosure, telemetry barrier, worker, and recovery identity that
contains any of those six values also contains `MigrationControllerEpoch`.
Generated schemas reject a resettable counter outside that epoch binding.
`RekeyIdentity`, `RekeyOutcomeNonce`, the rekey record compare-and-swap
digest, status-disclosure identity, and telemetry-recovery identity are not
derived from, sequenced by, or reset with any of the six.

Counter exhaustion is prevented with generated drain headroom rather than a
one-increment sentinel. For each exact counter kind `k`, generation computes:

```
REKEY_DRAIN_HEADROOM[k] =
  max(
    required_increments_to_quiesce_or_migrate(configuration, k)
    for configuration in GeneratedLegalLiveRekeyConfigurations
  )

REKEY_ADMISSION_THRESHOLD[k] =
  u64::MAX - REKEY_DRAIN_HEADROOM[k]
```

`GeneratedLegalLiveRekeyConfigurations` is the exhaustive bounded product of
all five primary child slots, all five refusal subslots, the reusable
status-audit slot, every migration-audit-repair and activation state, every
assignment-issuance prepare, assignment-revocation audit and capacity-release
state, every telemetry marker, latch, barrier, and recovery state, and rekey
preparation itself.
Each state graph edge declares its six-component maximum increment vector.
The generator computes the maximum component-wise cost to settle a legal
simultaneous live configuration or move each live continuation into the
rekey record. Rekey preparation is present in the census with a zero vector
because the dedicated protocol uses no resettable counter. An absent state,
unknown edge cost, overflow in the calculation, zero headroom for a counter
whose graph can increment, or hand-written constant fails generation.

The transaction that first makes any counter equal to its
`REKEY_ADMISSION_THRESHOLD` atomically enters
`DrainOnlyThresholdReached`. Before a multi-increment request can charge any
counter, the controller adds its generated six-component increment vector to
the current six-component counter vector with checked arithmetic. If any
component would cross its threshold, the same transaction refuses the
increment and durably enters `DrainOnlyWouldCrossThreshold` with the
pre-request current values, the triggering increment vector, the exact
would-cross counter kinds, thresholds, and remaining budgets. It creates no
rekey identity or alias.

New ordinary child prepares, semantic migration transitions, telemetry
updates, and migration-status claims are then forbidden. Only an
already-admitted drain action whose generated counter-budget vector fits the
remaining reserved headroom, or the counter-independent rekey protocol, may
run. Each permitted drain transition atomically charges its vector; no drain
action can consume a component reserved for the worst remaining legal
configuration. The closed inventory and component-wise budget proof make
wrapping any one of the exact six counters unreachable.

Legacy or corrupt values above a threshold do not attempt a counter-using
drain. Their remaining budget for the affected component is zero rather than
an underflowing subtraction. Startup or the observing transaction first
installs `DrainOnlyCounterExhausted` without a rekey identity. The controller
then uses fixed, counter-independent
`RekeyContinuationRecord` entries inside the non-resettable rekey record:

```
RekeyContinuationRecord = {
  continuation_alias: ControllerIssued256BitNonCapability,
  continuation_kind: CounterIndependentRekeyContinuationKind,
  canonical_continuation_digest,
  source_record_digest,
  migration_proof_digest
}

CounterIndependentRekeyContinuationKind =
  ChildOrRefusalSlot
  | MigrationAuditRepairOrActivation
  | StatusDisclosure
  | AssignmentIssuanceOrRevocationOrCapacityRelease
  | TelemetryUpdateOrRecoveryBarrier
  | SemanticMigrationContinuation
```

These records contain none of the six resettable counters. A typed
counter-independent drain action freezes the exact source record, proves that
the old continuation can no longer write, and atomically installs its
continuation record. This route is available for every quiescence blocker,
including migration-audit activation and telemetry barrier migration. A
continuation is never dropped because its legacy counter is already too high.

The gate state is closed:

```
MigrationRekeyAdmissionGates =
  Open
  | DrainOnlyThresholdReached {
      trigger_counter_kinds,
      current_counter_values,
      admission_thresholds,
      remaining_counter_budget,
      new_child_prepares: Closed,
      semantic_transitions: Closed,
      telemetry_updates: Closed,
      migration_status_claims: Closed
    }
  | DrainOnlyWouldCrossThreshold {
      would_cross_counter_kinds,
      current_counter_values,
      triggering_increment_vector,
      admission_thresholds,
      remaining_counter_budget,
      new_child_prepares: Closed,
      semantic_transitions: Closed,
      telemetry_updates: Closed,
      migration_status_claims: Closed
    }
  | DrainOnlyCounterExhausted {
      exhausted_counter_kinds,
      current_counter_values,
      admission_thresholds,
      integrity_reason,
      new_child_prepares: Closed,
      semantic_transitions: Closed,
      telemetry_updates: Closed,
      migration_status_claims: Closed
    }
  | DrainOnlyRekeyActive {
      rekey_identity_alias,
      old_controller_epoch,
      new_child_prepares: Closed,
      semantic_transitions: Closed,
      telemetry_updates: Closed,
      migration_status_claims: Closed
    }
```

An initial rekey request atomically changes `Open` or one exact pre-request
drain-only variant to `DrainOnlyRekeyActive` and
`RequestedQuiescencePending`. That transition is the first point at which a
rekey identity exists. Existing status
disclosure, command, outbox, audit, revocation, and telemetry work may settle
within its declared budget or use its typed counter-independent continuation
migration. No new claim can race into the old epoch. Install is permitted only
after every blocker is either quiescent or represented by exactly one verified
continuation record. The gates reopen only in the atomic epoch-install
transaction.

`RekeyMigrationControllerEpoch` is a dedicated durable protocol in the
separately sealed, non-resettable `MigrationControllerRekeyRecord`, not a
child command. Its request is:

```
RekeyMigrationControllerEpochRequest =
  Initial {
    migration_attempt_alias,
    expected_old_controller_epoch,
    requested_replacement_digest
  }
  | Resume {
      migration_attempt_alias,
      rekey_identity_alias
    }
```

The initial request carries neither raw `RekeyIdentity` nor an alias. In the
same transaction that admits it and enters
`DrainOnlyRekeyActive` and `RequestedQuiescencePending`, the controller mints
the internal 256-bit `RekeyIdentity` and the independent primary
`RekeyOutcomeNonce`. A `DrainOnlyWouldCrossThreshold` record therefore always
precedes identity creation for the request that triggered it, and its only
next action is this identity-free initial request. Later status
and resume expose or accept only the server-resolved, non-capability
`rekey_identity_alias`. Generated commands never accept a raw rekey identity.
A byte-identical repeated initial request joins the active record. Changed
initial bytes while that record is active are a dedicated audited refusal and
cannot replace its immutable fields; a `Resume` alias must resolve to that
same record and supplies no replacement fields to alter.

```
MigrationControllerRekeyState =
  RequestedQuiescencePending {
    old_controller_epoch,
    rekey_identity: ControllerIssued256Bit,
    primary_outcome_nonce: ControllerIssued256Bit,
    exact_counter_kinds,
    exact_counter_values,
    remaining_counter_budget,
    semantic_state_digest,
    requested_replacement_digest,
    continuations: BoundedRekeyContinuationRecords
  }
  | NewEpochPrepared {
      requested_fields,
      new_controller_epoch,
      quiescence_or_continuation_proof_digest,
      replacement_control_digest,
      replacement_integrity_digest,
      replacement_telemetry_digest,
      success_event_id,
      success_event_digest,
      canonical_success_event_bytes_digest
    }
  | RekeyAuditOutboxPending {
      prepared_fields,
      audit_outbox_digest,
      sink_reservation_alias
    }
  | AuditAcknowledgedInstallPending {
      prepared_fields,
      audit_acknowledgement_digest
    }
  | Installed {
      old_controller_epoch,
      new_controller_epoch,
      rekey_identity,
      primary_outcome_nonce,
      success_event_id,
      success_event_digest,
      audit_acknowledgement_digest,
      installed_state_digest
    }

MigrationControllerRekeySafeState =
  RequestedQuiescencePending {
    rekey_identity_alias,
    exact_counter_kinds,
    exact_counter_values,
    remaining_counter_budget,
    quiescence_blocker_codes,
    migrated_continuation_aliases
  }
  | NewEpochPrepared {
      rekey_identity_alias,
      new_controller_epoch_alias,
      quiescence_or_continuation_proof_digest,
      success_event_digest
    }
  | RekeyAuditOutboxPending {
      rekey_identity_alias,
      new_controller_epoch_alias,
      success_event_digest
    }
  | AuditAcknowledgedInstallPending {
      rekey_identity_alias,
      new_controller_epoch_alias,
      success_event_digest,
      audit_acknowledgement_digest
    }

MigrationControllerRekeyTerminalResult =
  Installed {
    rekey_identity_alias,
    new_controller_epoch_alias,
    success_event_digest,
    audit_acknowledgement_digest,
    installed_state_digest
  }

MigrationControllerRekeyRecovery =
  InProgress(MigrationControllerRekeySafeState)
  | Terminal(MigrationControllerRekeyTerminalResult)
```

`Installed` is terminal result, never an in-progress safe state. Construction,
verification, allocation, storage, fsync, and retryable install failure retain
the same nonterminal state, identity, and primary outcome nonce. The dedicated
claim, recovery identity, and audit authorization contain the migration
identity, old controller epoch, rekey identity, outcome nonce, record digest,
and closed outcome kind only. They contain no child sequence, child-audit
sequence, slot record generation, migration-state generation,
control-reserve incarnation, telemetry-marker sequence, reservation
generation, or other resettable counter.

Every admitted terminal success or refusal of the dedicated protocol receives
its own freshly minted counter-independent `RekeyOutcomeNonce` and therefore
its own event id:

```
MigrationEpochRekeyOutcomeEventId =
  digest(
    "d2b:panel:migration-controller-epoch-rekey-outcome:v1",
    MigrationAttemptIdentity,
    OldMigrationControllerEpoch,
    RekeyIdentity,
    RekeyOutcomeNonce,
    RekeyTerminalOutcomeKind
  )
```

The primary nonce minted on entry is reserved for the eventual install
success. A changed-request or other admitted terminal refusal mints a distinct
nonce and event without changing the primary continuation. Exact retries
replay the same outcome. Nonterminal failures mint no terminal outcome. The
event preimage and canonical audit bytes exclude all resettable counters and
all child, slot, telemetry, and reservation identities.

Rekey can advance to `NewEpochPrepared` only after one transaction verifies
the old-epoch gate and all of the following under one expected record digest:

- each live primary or refusal child slot has settled, or its exact immutable
  continuation has been fenced and moved into a
  `RekeyContinuationRecord`;
- every migration result outbox, acknowledgement, audit-repair state, and
  activation has settled or has an exact counter-independent continuation;
- an existing migration-status disclosure has been acknowledged and cleared,
  or its matching disclosure-identity-bound reservations and response have
  been fenced and migrated; no new disclosure claim is admitted;
- each assignment revocation and unused-capacity release is settled or
  represented by its exact continuation; and
- the semantic migration state and retained success tuple are stable or have
  exact continuation records. The rekey record is excluded from the reset set
  and is never its own blocker.

Telemetry does not have to become `Healthy`, or even `Stable`, before rekey.
A verified degraded marker, `UpdatePending`, `RecoveryBarrier`, corrupt
marker, armed latch, or live recovery slot is frozen with its exact marker,
latch, normal-health, and recovery-slot digests into a
`TelemetryUpdateOrRecoveryBarrier` continuation. The new epoch installs the
same non-healthy observation and a counter-independent recovery continuation;
explicit telemetry recovery proceeds afterward. Rekey may not reinterpret it
as healthy or clear the latch.

The transition to `NewEpochPrepared` freezes the replacement bytes, every
continuation, and canonical success event bytes. Because all four admission
gates are already closed, a late child, status, semantic, or telemetry claim
is refused rather than racing the prepare. No effect, outbox,
acknowledgement, audit-repair state, revocation, status disclosure, telemetry
barrier, or corrupt marker can be silently dropped.

`NewEpochPrepared` freezes one canonical success event byte string for the
primary outcome event id. Every later retry must reproduce the stored digest
byte for byte; a mismatch is an integrity fault and appends nothing.
`RekeyAuditOutboxPending` retries only those frozen bytes. The sink must fsync
and the controller must durably enter `AuditAcknowledgedInstallPending` before
epoch install. A crash after audit acknowledgement resumes install, not event
construction and not a new identity.

The install transaction compares the complete old semantic, gate,
quiescence-or-continuation, and audit-acknowledgement digests; creates the new
epoch; resets all and only the exact six counters to zero; preserves semantic
state and the retained success tuple; installs available slots for settled
work and counter-independent continuation adapters for migrated work; carries
every non-healthy telemetry marker, latch, and recovery continuation without
claiming recovery; advances the separate rekey record to terminal
`Installed`; and reopens all four gates. It cannot reset, delete, or
reconstruct its own recovery record.

A future rekey replaces the terminal record only after the installed epoch is
authoritative and mints a fresh rekey identity and fresh primary outcome
nonce. Thus two consecutive rekeys cannot share identity or outcome events. A
crash at requested, prepared, outbox-pending,
acknowledged-install-pending, or terminal install resumes that exact record.
Success audit is durable before install, and neither threshold exhaustion nor
legacy corruption can force counter reuse or wrap.

`MigrationRecoveryStatus` is a strict tagged type. `ReadMigrationRecoveryStatus`
returns the selected variant directly, not a state digest followed by another
generic read. Each row below is one wire variant; its safe fields, exact
state-valid endpoint operation, caller class, eligibility, and reserve route
are owned by that tag and cannot be substituted independently.
Every variant owns the common fields `migration_attempt_alias`,
`controller_epoch`, `control_reserve_incarnation`, and
`migration_state_generation`; the table lists its additional exact fields.

Audit-sink failure is not a generic pause. It is this dedicated nested state:

```
MigrationAuditRepairState =
  AuditFailureDetected {
    owner_epoch_alias,
    old_reservation_generation,
    repair_identity,
    success_event_alias,
    success_event_digest,
    deadline
  }
  | IntentRecorded {
      repair_identity,
      old_reservation_generation,
      success_event_alias,
      success_event_digest,
      deadline
    }
  | OldGenerationInvalidationPending {
      repair_identity,
      old_reservation_generation,
      success_event_alias,
      success_event_digest,
      deadline
    }
  | OldGenerationInvalidatedRebindPending {
      repair_identity,
      invalidated_reservation_generation,
      invalidation_proof_digest,
      success_event_alias,
      success_event_digest,
      deadline
    }
  | SuccessGenerationBoundReplacementPending {
      repair_identity,
      invalidated_reservation_generation,
      replacement_reservation_generation,
      invalidation_proof_digest,
      rebind_proof_digest,
      success_event_alias,
      success_event_digest,
      deadline
    }
  | ReplacementTupleInstalled {
      repair_identity,
      repair_tombstone_digest,
      replacement_reservation_generation,
      success_event_alias,
      success_event_digest,
      deadline
    }
  | SinkAcknowledgementPending {
      repair_identity,
      appendable_reservation_generation,
      success_event_alias,
      success_event_digest,
      deadline
    }
```

| `MigrationRecoveryStatus` variant | Additional exact safe fields | Exact operation, caller, eligibility, and reserve |
| --- | --- | --- |
| `AcceptedUnclaimedPrepared` | reservation alias and generation, deadline | `Operator.ResumeProtectedAttempt` by `ProtectedOperator`, immediately, through the resume child slot in `MigrationControlReserve` |
| `AcceptedUnclaimedAcceptedBound` | reservation alias and generation, deadline | `Operator.ResumeProtectedAttempt` by `ProtectedOperator`, immediately, through the resume child slot |
| `ProcessingLeaseLive` | owner epoch alias, lease until, deadline | `Operator.ReadMigrationRecoveryStatus` by `ProtectedOperator`, observation only, through `MigrationIntegrityReserve` |
| `ProcessingResumeEligible` | fenced owner epoch alias, lease expiry, deadline | `Operator.ResumeProtectedAttempt` by `ProtectedOperator`, immediately, through the resume child slot |
| `ProcessingFenceEligible` | owner epoch alias, lease expiry, closed fence reason, deadline | `Operator.FenceProtectedAttempt` by `ProtectedOperator`, immediately, through the fence child slot |
| `PausedSelfClearingWait` | owner epoch alias, exact `MigrationSelfClearingWaitV1`, pause deadline | controller waits for that exact prerequisite and then resumes the same migration; operator observation is `Operator.ReadMigrationRecoveryStatus` |
| `PausedOperatorRepairRequired` | owner epoch alias, exact `MigrationOperatorRepairPlanV1`, pause deadline | satisfy the carried prerequisite, then `Operator.ResumeProtectedAttempt` through the resume child slot; no audit-sink or control-integrity action is constructible |
| `PausedFenceEligible` | owner epoch alias, exact retained wait or generic repair plan, closed fence reason, pause deadline | `Operator.FenceProtectedAttempt` by `ProtectedOperator`, immediately, through the fence child slot |
| `MigrationAuditRepair` | exact `MigrationAuditRepairState` including repair identity, retained success tuple, generations, proofs, owner where applicable, and deadline | `Operator.RepairMigrationSinkAppend` by `ProtectedOperator`, immediately, through the sink-repair child slot |
| `MigrationActivationPending` | appendable reservation generation, audit-acknowledgement digest, deadline | `Operator.CompleteMigrationAuditActivation` by `ProtectedOperator`, immediately, through the audit-activation child slot |
| `ControlReserveIntegrityCorrupt` | underlying migration-state code, quarantined reserve incarnation, epoch-and-incarnation-bound repair identity, corrupt slot field codes, expected reserve digest | `Operator.RepairMigrationControlReserve` by `ProtectedOperator`, immediately, through the integrity repair child slot |
| `MigrationControllerRekeyRequired` | exact subset of the six `MigrationResettableCounterKind` values at their generated `REKEY_ADMISSION_THRESHOLD`, current values, thresholds, and remaining drain budgets; no rekey identity or alias | initial `Operator.RekeyMigrationControllerEpoch` by `ProtectedOperator` with no rekey identity or alias, through the separately sealed non-child rekey record |
| `MigrationControllerRekeyWouldCrossThreshold` | exact `DrainOnlyWouldCrossThreshold` pre-request state, including all six current counter values, the triggering six-component increment vector, exact would-cross counter kinds, thresholds, and remaining drain budgets; no rekey identity or alias | initial `Operator.RekeyMigrationControllerEpoch` by `ProtectedOperator` with no rekey identity or alias, through the separately sealed non-child rekey record |
| `MigrationControllerCounterExhausted` | exact subset of the six counter kinds above threshold or otherwise legacy/corrupt, current values, thresholds, and integrity reason; no rekey identity or alias | initial `Operator.RekeyMigrationControllerEpoch` by `ProtectedOperator` with no rekey identity or alias, through the separately sealed non-child rekey record |
| `Completed` | source generation, destination generation, migration event id and tombstone digest | `Operator.ReadMigrationRecoveryStatus` by `ProtectedOperator`, terminal observation with `NoFurtherAction`, through `MigrationIntegrityReserve` |

`MigrationControllerRekeyRecovery` is separate from
`MigrationRecoveryStatus`. A pre-request drain-only refusal returns the exact
threshold, would-cross, or exhausted status product above and only the
identity-free initial action. Once `DrainOnlyRekeyActive` closes new
migration-status claims, only `RekeyMigrationControllerEpoch::Resume` with
the server-resolved alias returns its exact in-progress or terminal rekey recovery tag.
`MigrationControllerRekeySafeState` cannot contain `Installed`; `Installed`
is returned only as `MigrationControllerRekeyRecovery::Terminal`. After
atomic install, ordinary migration status is evaluated in the new epoch under
the reopened gates.

The versioned closed pause plans are:

```
MigrationSelfClearingWaitV1 =
  TransientSourceReadRetry {
    prerequisite: SourceReadServiceHealthy,
    next: ControllerResumeSameAttempt
  }
  | TransientDestinationWriteRetry {
      prerequisite: DestinationWriteServiceHealthy,
      next: ControllerResumeSameAttempt
    }
  | TransientVerificationServiceRetry {
      prerequisite: VerificationServiceHealthy,
      next: ControllerResumeSameAttempt
    }

MigrationOperatorRepairPlanV1 =
  ProvisionRawDestinationCapacityThenResume {
    prerequisite: RequiredRawDestinationCapacityProvisionedWithoutAuthorityAccess,
    next: Operator.ResumeProtectedAttempt by ProtectedOperator
  }
  | RepairSourceStorageThenResume {
      prerequisite: SourceStorageIntegrityVerified,
      next: Operator.ResumeProtectedAttempt by ProtectedOperator
    }
  | RepairDestinationStorageThenResume {
      prerequisite: DestinationStorageIntegrityVerified,
      next: Operator.ResumeProtectedAttempt by ProtectedOperator
    }
  | RepairManifestVerificationThenResume {
      prerequisite: ReviewedManifestVerificationInputsInstalled,
      next: Operator.ResumeProtectedAttempt by ProtectedOperator
    }
  | RepairDestinationVerificationThenResume {
      prerequisite: CompleteDestinationVerificationPasses,
      next: Operator.ResumeProtectedAttempt by ProtectedOperator
    }
```

The controller verifies the named prerequisite while executing the resume
child command. A missing prerequisite is a typed audited child-command refusal
that returns the same exact plan. A successful generic operator repair
and authenticated `ResumeProtectedAttempt` advances directly out of the pause;
there is no independently resumable operator-repair state. Audit-sink failure transitions
directly to `MigrationAuditRepair`; control-reserve corruption transitions
directly to `ControlReserveIntegrityCorrupt`. Neither condition can be encoded
inside `PausedOperatorRepairRequired` or `MigrationOperatorRepairPlanV1`.
Unknown pause reasons, a generic `RepairThenResume`, and a plan whose next
operation does not match its variant fail strict construction and decoding.

The status constructor partitions `Processing` and `Paused` by
controller-owned lease, wait, repair, resume, and fencing eligibility so no
variant offers incompatible operations. A non-authoritative
`AcceptancePreparePending` is not an active migration and remains visible only
through ordinary protected-attempt status. Preflight refusals create no
migration state.

`ReadMigrationRecoveryStatus` always uses `MigrationIntegrityReserve`.
It is an observational current-state read: it ignores every caller key,
acquires no mutation slot, and evaluates the current journal, state
generation, controller epoch, pause or dedicated repair state,
control-reserve incarnation, command-slot state, counter exhaustion, and
integrity state on
every call. A retry after a state transition therefore returns the new state,
never a cached or stale status. It is outside `ProtectedAttemptId`,
accepted-journal, tombstone, replay-payload, and
`ProtectedAttemptRecovery` generation.

The read authenticates `ProtectedOperator` and uses one fixed reusable
integrity-reserve status-audit slot:

```
MigrationStatusAuditSlot =
  Available
  | Claimed {
      disclosure_identity: ControllerIssued256Bit,
      migration_attempt_alias,
      controller_epoch,
      owner_epoch,
      lease_until,
      deadline,
      required_capacity,
      controller_reservation_alias,
      sink_reservation_alias
    }
  | AuditOutboxPending {
      claimed_fields,
      response_state_digest,
      event_id,
      event_digest,
      sink_reservation_alias
    }
  | AuditAcknowledgedReturnPending {
      claimed_fields,
      response_state_digest,
      event_id,
      event_digest,
      acknowledgement_digest,
      sealed_response
    }

MigrationStatusAccessAuditEventId =
  digest(
    "d2b:panel:migration-status-disclosure-audit:v1",
    MigrationAttemptIdentity,
    MigrationControllerEpoch,
    DisclosureIdentity
  )

MigrationStatusAuditReservationBinding = {
  disclosure_identity,
  migration_attempt_alias,
  controller_epoch
}

MigrationStatusAccessAuditEvent =
  {
    event_id,
    disclosure_identity,
    migration_attempt_alias,
    controller_epoch,
    response_state_digest,
    protected_operator_audit_digest: ProtectedOperatorAuditDigest
  }

ProtectedOperatorAuditDigest =
  keyed_digest(
    DeploymentAuditKey,
    "d2b:panel:protected-operator-audit:v1",
    ProtectedOperatorIdentity
  )
```

Every successful disclosure is a distinct serialized operation. After
authentication, the controller must claim `Available` with a fresh
controller-issued 256-bit disclosure identity, lease, deadline, and exact
controller and sink capacity before constructing the response. Both
reservations bind the immutable disclosure identity, migration alias, and
controller epoch, never operator history or mutable worker ownership. It then
evaluates current state, seals the exact response, binds those reservations to
the one event id and digest, and appends and fsyncs one event containing the
current state digest. Only after acknowledgement is durable may it return the
sealed response; it then clears the slot. A second read by the same operator
in the same epoch gets a fresh identity and event. A different operator or a
new epoch also serializes through the slot and gets a distinct identity,
reservations, and event. The fixed slot retains no per-operator history; the
sink's bounded D17 retention owns event history.

The event omits `protected_operator_alias` and always carries the fixed
deployment-keyed `ProtectedOperatorAuditDigest`. The claim transaction places
that digest directly into the preallocated canonical audit prefix, so crash
recovery never needs an operator field in the slot. The digest is permitted only inside canonical
audit event bytes. It is non-reversible outside the deployment and is forbidden from
logs, errors, status, refusal products, metrics, `Debug`, reservation
bindings, and every non-audit schema.

A concurrent read waits for the slot or receives
`migration-status-audit-capacity-unavailable` before any state is constructed
or disclosed. No successful response, safe state digest, rekey blocker,
telemetry observation, or other migration detail is released without claimed
capacity and a durable acknowledgement. Startup adopts the exact outbox or
acknowledgement of a stale claim. If no response was returned, it clears the
settled slot and a retry creates a new disclosure identity and event; it never
replays the old response as a successful new disclosure. A reservation that
matches the immutable disclosure binding is adopted even after worker
takeover. A reservation that does not match or cannot be adopted is
proof-cancelled before the slot clears; time alone never clears it. The status slot
cannot consume a child command slot or ordinary status capacity.

While any drain-only gate variant is active, no new status claim is admitted. An
existing claim must reach acknowledgement and clear, or be fenced and moved
to the rekey record by the typed disclosure-continuation action. Dedicated
rekey resume reads its own record and does not claim this slot.

`ResumeProtectedAttempt`, `FenceProtectedAttempt`,
`RepairMigrationSinkAppend`, and `CompleteMigrationAuditActivation` use only
their fixed child slots in `MigrationControlReserve`.
`RepairMigrationControlReserve` uses only its fixed child slot in
`MigrationIntegrityReserve`. `RekeyMigrationControllerEpoch` uses only the
separately sealed non-child rekey record. The repair identity is:

```
MigrationControlReserveRepairIdentity =
  digest(
    "d2b:panel:migration-control-reserve-repair:v1",
    MigrationAttemptIdentity,
    MigrationControllerEpoch,
    CurrentMigrationControlReserveIncarnation
  )
```

The integrity operation reads the canonical migration journal and generated
state graph, constructs a complete replacement containing exactly the four
available fixed mutation slots in the integrity reserve's pre-sealed repair
workspace, and verifies its schema, digest, controller epoch, state
generation, and operation set. The
install transaction keeps the old reserve quarantined, installs the
replacement, and increments `MigrationControlReserveIncarnation` atomically.
It does not advance migration state and cannot change the retained success
tuple or capacity-switch effect.

A construction, verification, fsync, or installation failure leaves the old
reserve quarantined, leaves `ControlReserveIntegrityCorrupt` readable, returns
the exact same epoch-and-incarnation-bound repair identity, and resets the
fixed integrity workspace for retry. No failed repair installs an incarnation
or creates a second identity; the same fixed slot and identity are retryable.
After a successful install, any later corruption, including at the same
migration state generation, names the incremented incarnation and therefore a
new repair identity. Status remains available while control
integrity is corrupt, so repair never depends on the reserve it repairs.

Unrelated status or control traffic cannot name either migration reserve.
Exhausting `ProtectedStatusReserve` therefore cannot hide, strand, resume,
fence, repair, activate, or integrity-repair the active migration, and
migration control cannot borrow any unrelated partition.

A capacity generation cannot become active unless the schema generator proves
and the controller transactionally seals the full
`MigrationExecutionReserve`, including its sibling
`MigrationControlReserve` and `MigrationIntegrityReserve`.
Normal admission, blocker recovery, unrelated status reads, cleanup, migration
preflight refusals, migration preflight signals, and every operation's accepted
conflict attempts are structurally unable to address either reserve. Migration
conflicts are not accepted at all. Once a valid non-conflict migration accepts, its
allocation and transient lane remain bound to the same `AttemptIdentity`
through every pause, takeover, resume, audit repair, and completion. The
destination generation carries a fresh sealed execution reserve before
cutover; the source generation retains the completed migration's permanent
records. Thus any number of conflict requests or status requests can at most
exhaust their own signal, accepted-conflict, or status budgets and cannot make
a structurally valid, authorized, eligible non-conflict migration inadmissible
or make an active migration uncontrollable. Control-reserve corruption cannot
consume or disable the independent status and integrity-repair route.

`retention-capacity-migration-ineligible` owns this closed detail type:

```
MigrationIneligibleBlockerDetails =
  CurrentBlockerDetails {
    blockers: CompleteSortedRetentionBlockerRecords
  }
  | BlockerDetailsRedacted
  | BlockerDetailsStale
```

`CompleteSortedRetentionBlockerRecords` is the complete current bounded list
of `RetentionBlockerRecord` values, including every exact plan id. The
`CurrentBlockerDetails` remedy directly executes those named plan ids in
order and runs cleanup. It never reads status first. Only
`BlockerDetailsRedacted` or `BlockerDetailsStale` renders
`ReadRetentionRecoveryStatus`, followed by execution of the exact plan ids
returned by that read and cleanup. No variant renders an unparameterized
blocker-specific remedy.

That marker-driven `ReadRetentionRecoveryStatus` is itself a named reserved
recovery transition. Each blocker reservation includes its share of one
coalescing read product for the exact capacity generation and current complete
blocker-set digest. It therefore consumes neither `ProtectedStatusReserve` nor
ordinary capacity, and unrelated status traffic cannot address it. An
identical or concurrent marker read replays the one product. The only refusal
is a proved `RecoveryReserveIntegrityCorrupt`, whose reviewed
`ReserveIntegrityRepair` migration reason is eligible. Redaction, staleness,
and a full ordinary status budget therefore cannot create a dead end.

`MigrateRetentionCapacity` has a mandatory preflight before
acceptance-prepare. The controller verifies the authenticated protected
operator, strict manifest structure and signature, one closed reason of
`ReserveIntegrityRepair` or `VersionedBoundMigration`, eligibility against the
current generation, and complete source and destination bounds. A structurally
invalid, unauthorized, or ineligible request is a preflight refusal: it
creates no
`AcceptancePrepare`, accepted journal, or authoritative attempt, and releases
any transient preflight hold. A plain `round-input-store-full`, an operator
preference, or a desire to avoid a blocker-specific remedy is
`retention-capacity-migration-ineligible`.

For idempotency conflict, the structural, authorization, and domain-eligibility
checks run first, but conflict classification runs before transient lane
or permanent-reserve acquisition. A valid same-key different-request
migration returns `retention-capacity-migration-replay-conflict` as a
preflight refusal. It creates no `AcceptancePrepare`, accepted
`AttemptIdentity::Conflict`, audit outbox, replay result, sink reservation, or
tombstone. It attempts one bounded durable diagnostic signal from
`MigrationPreflightSignalReserve`; signal failure does not alter the refusal.
That signal is not an accepted-attempt audit event and cannot authorize state.

```
MigrationPreflightSignalId =
  digest(
    "d2b:panel:migration-preflight-signal:v1",
    capacity_generation,
    domain_separated_base_migration_alias,
    domain_separated_conflicting_request_alias
  )

MigrationReplayConflictRefusalProduct = {
  signal_id,
  reason: SameKeyDifferentRequest
}

MigrationPreflightSignal =
  ReplayConflict {
    signal_id,
    reason: SameKeyDifferentRequest,
    first_seen_time_bucket
  }
  | AggregateOverflow {
      reason: DistinctSignalCapacityExhausted,
      first_overflow_time_bucket,
      occupied_bucket_bitmap: FixedBoundedBitmap,
      approximate_distinct_bucket_count: SaturatingBoundedCount
    }

MigrationPreflightSignalWindowSummary = {
  window_time_bucket,
  detailed_signal_count: SaturatingBoundedCount,
  overflow_bucket_bitmap: FixedBoundedBitmap,
  telemetry_health: MigrationTelemetryHealthObservation
}
```

The aliases used in the identity preimage are internal domain-separated
non-capabilities and are not serialized. `signal_id` is likewise a
non-capability correlation digest and cannot address controller state. The
refusal product is computed before any telemetry write and always has the
same fields whether telemetry is healthy or absent. A signal or summary
carries no `AttemptIdentity`, protected attempt id, peer or principal id,
idempotency-key digest, request digest, sink namespace, path, handle,
deployment id, or protected text. Repetition of the same conflict
idempotently finds the same record and changes no durable field; it never
consumes another slot. When
every distinct-signal slot is occupied, the one pre-sealed
`AggregateOverflow` record maps each internal signal identity to a fixed
versioned bucket and atomically sets that bit. A repeated identity sets the
same bit and changes nothing. Collisions deliberately aggregate; the bounded
count is the population count of the bitmap, not a claim about exact requests.
The refusal remains `MigrationReplayConflictRefusalProduct`; aggregate
availability never substitutes another refusal shape.

The diagnostic subclass has hard bounds. The active 15-minute window owns
exactly 256 detailed slots and one `AggregateOverflow` slot. Rotation compacts
that window to one `MigrationPreflightSignalWindowSummary` in a controller-wide
96-slot ring, clears every detailed and overflow slot, and reuses those same
slots for the next window. Detailed records expire after 30 minutes even if
compaction fails. Summaries expire after 24 hours. A capacity-generation
cutover performs the same best-effort compaction, then clears and reuses the
active slots for the destination generation. A full summary ring overwrites
the oldest expired slot, or the oldest slot when none is expired; it never
delays a refusal, holds the transient lane, consumes migration execution or
control capacity, or blocks cutover. Compaction failure drops the diagnostic
input after recording the health transition when possible. These signals and
summaries cannot become retention blockers or accumulate by generation.

Authenticated migration-control conflicts use a different bounded diagnostic
reserve:

```
MigrationControlConflictKind =
  ChangedBytesWhileCurrentCommand
  | StaleExpectedGeneration

MigrationControlConflictSignalId =
  digest(
    "d2b:panel:migration-control-conflict-signal:v1",
    domain_separated_migration_attempt_alias,
    MigrationControllerEpoch,
    migration_control_reserve_incarnation,
    MigrationControlCommand,
    MigrationControlConflictKind,
    expected_migration_state_generation,
    domain_separated_request_digest_alias
  )

MigrationControlConflictSignal =
  Detailed {
    signal_id,
    operation: MigrationControlCommand,
    conflict_kind: MigrationControlConflictKind,
    first_seen_time_bucket
  }
  | AggregateOverflow {
      operation: MigrationControlCommand,
      conflict_kind: MigrationControlConflictKind,
      first_overflow_time_bucket,
      occupied_bucket_bitmap: FixedBoundedBitmap,
      approximate_distinct_bucket_count: SaturatingBoundedCount
    }

MigrationControlConflictWindowSummary = {
  window_time_bucket,
  operation: MigrationControlCommand,
  conflict_kind: MigrationControlConflictKind,
  detailed_signal_count: SaturatingBoundedCount,
  overflow_bucket_bitmap: FixedBoundedBitmap,
  telemetry_health: MigrationTelemetryHealthObservation
}
```

The `MigrationControlConflictSignalReserve` owns exactly 64 reusable detailed
slots and one overflow slot in an active 15-minute window, plus a
controller-wide 96-slot summary ring. A detailed signal expires after 30
minutes and a summary after 24 hours. Window rotation and control-reserve
incarnation change compact best effort, clear the active slots, and reuse
them. A full ring replaces the oldest expired summary or otherwise its oldest
summary. No signal or summary becomes a retention blocker.

The same signal id in a window is one idempotent record. After the detailed
slots fill, the overflow slot maps the id to one fixed versioned bit; repeats
set the same bit and collisions deliberately aggregate. Counters advance only
for the first durable detailed identity or first setting of an overflow bit.
The signal carries no caller key, raw request or digest, capability, peer,
path, deployment id, or unbounded label. It uses neither a mutation slot nor
control or integrity capacity.

Only `migration-control-state-stale` after successful authentication and
target resolution, or `migration-control-replay-conflict` after the complete
current-state prerequisite validation, attempts this signal. An unauthorized
request emits none. A current-generation request whose operation or
prerequisites are ineligible emits none even when its bytes differ from a
slot's retained bytes. Thus authorization and eligibility cannot be probed by
the conflict diagnostic.

Migration telemetry health is sticky and independently integrity checked:

```
MigrationTelemetryHealthState =
  Healthy
  | DetailedDegraded
  | AggregateDegraded
  | ExporterDegraded

MigrationTelemetryHealthMarker =
  StableHealthy {
    controller_epoch,
    marker_sequence,
    normal_health_digest
  }
  | StableDetailedDegraded {
      controller_epoch,
      marker_sequence,
      normal_health_digest,
      telemetry_failure_alias
    }
  | StableAggregateDegraded {
      controller_epoch,
      marker_sequence,
      normal_health_digest,
      telemetry_failure_alias
    }
  | StableExporterDegraded {
      controller_epoch,
      marker_sequence,
      normal_health_digest,
      telemetry_failure_alias
    }
  | UpdatePending {
      controller_epoch,
      marker_sequence,
      expected_prior_sequence,
      prior_stable_digest,
      telemetry_operation_class,
      update_plan_digest,
      telemetry_failure_alias
    }
  | RecoveryBarrier {
      controller_epoch,
      marker_sequence,
      expected_failed_sequence,
      prior_failure_digest,
      required_probe_set_digest,
      telemetry_failure_alias
    }

MigrationTelemetryHealthObservation =
  Healthy {
    marker_sequence,
    next: NoFurtherAction
  }
  | DetailedDegraded {
      marker_sequence,
      telemetry_failure_alias,
      next: Operator.RecoverMigrationTelemetryHealth
    }
  | AggregateDegraded {
      marker_sequence,
      telemetry_failure_alias,
      next: Operator.RecoverMigrationTelemetryHealth
    }
  | ExporterDegraded {
      marker_sequence,
      telemetry_failure_alias,
      next: Operator.RecoverMigrationTelemetryHealth
    }
  | HealthUnavailable {
      reason: HealthUnavailableReason,
      telemetry_failure_alias,
      next: Operator.RecoverMigrationTelemetryHealth
    }
  | UpdatePending {
      marker_sequence,
      telemetry_operation_class,
      telemetry_failure_alias,
      next: Operator.RecoverMigrationTelemetryHealth
    }
  | RecoveryBarrier {
      marker_sequence,
      telemetry_failure_alias,
      next: Operator.RecoverMigrationTelemetryHealth
    }
  | CorruptMarker {
      integrity_reason: MarkerIntegrityReason,
      telemetry_failure_alias,
      next: Operator.RecoverMigrationTelemetryHealth
    }
  | ArmedFailureLatch {
      marker_tag,
      latch_digest,
      telemetry_failure_alias,
      next: Operator.RecoverMigrationTelemetryHealth
    }

HealthUnavailableReason =
  MarkerSequenceExhausted
  | NormalHealthDigestMismatch
  | TelemetryRecoveryIncomplete
  | TelemetryFollowupUnavailable

TelemetryFailureAlias = ControllerIssued256BitNonCapability

MigrationTelemetryCurrentFailureAliasRecord =
  NoneHealthy
  | Current {
      telemetry_failure_alias,
      controller_epoch,
      source_marker_digest,
      source_latch_digest,
      source_state_tag
    }

MigrationTelemetryFailureLatch =
  Clear {
    last_recovery_event_digest
  }
  | ArmPending {
      telemetry_write_identity: ControllerIssued256Bit,
      controller_epoch,
      expected_marker_digest,
      telemetry_operation_class,
      update_plan_digest,
      telemetry_failure_alias
    }
  | Armed {
      arm_pending_fields,
      armed_digest
    }
  | StableClosureProven {
      telemetry_write_identity,
      controller_epoch,
      stable_marker_digest,
      normal_health_digest,
      telemetry_failure_alias,
      closure_digest
    }

MigrationTelemetryHealthRecoveryRequestBinding = {
  recovery_identity: ControllerIssued256Bit,
  telemetry_failure_alias,
  controller_epoch,
  expected_marker_tag,
  expected_marker_digest,
  expected_latch_tag,
  expected_latch_digest,
  expected_failure_alias_record_digest,
  canonical_request_digest
}

MigrationTelemetryHealthRecoverySlot =
  Available
  | RecoveryRequested {
      request_binding: MigrationTelemetryHealthRecoveryRequestBinding,
      owner_epoch,
      lease_until,
      deadline,
      required_capacity,
      canonical_audit_prefix_digest
    }
  | RecoveryBarrierInstalled {
      request_binding,
      recovery_barrier_digest,
      required_probe_set_digest
    }
  | SuccessAuditOutboxPending {
      request_binding,
      recovery_barrier_digest,
      probe_result_digest,
      prepared_stable_state,
      prepared_normal_health_digest,
      recovery_event_id,
      recovery_event_digest,
      canonical_recovery_event_bytes: BoundedCanonicalAuditBytes,
      sealed_response: BoundedMigrationTelemetryRecoveryResponse,
      sink_reservation_alias
    }
  | FailureAuditOutboxPending {
      request_binding,
      source_failure_digest,
      failed_recovery_state_tag,
      failed_recovery_state_digest,
      closed_failure_code,
      proposed_next_telemetry_failure_alias,
      recovery_event_id,
      recovery_event_digest,
      canonical_recovery_event_bytes: BoundedCanonicalAuditBytes,
      sealed_response: BoundedMigrationTelemetryRecoveryResponse,
      sink_reservation_alias
    }
  | SuccessAuditSinkAcknowledgementPending {
      success_outbox_fields,
      appendable_reservation_generation
    }
  | FailureAuditSinkAcknowledgementPending {
      failure_outbox_fields,
      appendable_reservation_generation
    }
  | SuccessAuditAcknowledgedInstallPending {
      success_outbox_fields,
      audit_acknowledgement_digest
    }
  | FailureAuditAcknowledgedSettlePending {
      failure_outbox_fields,
      audit_acknowledgement_digest
    }

MigrationTelemetryHealthRecoveryLastResult =
  Succeeded {
    accepted_telemetry_failure_alias,
    request_binding_digest,
    source_failure_digest,
    installed_stable_state,
    recovery_event_id,
    recovery_event_digest,
    audit_acknowledgement_digest,
    sealed_response: BoundedMigrationTelemetryRecoveryResponse
  }
  | Failed {
      accepted_telemetry_failure_alias,
      request_binding_digest,
      source_failure_digest,
      failed_recovery_state_tag,
      failed_recovery_state_digest,
      closed_failure_code,
      current_telemetry_failure_alias,
      recovery_event_id,
      recovery_event_digest,
      audit_acknowledgement_digest,
      sealed_response: BoundedMigrationTelemetryRecoveryResponse
    }

MigrationTelemetryHealthRecoveryLastResultSlot =
  Empty
  | Settled(MigrationTelemetryHealthRecoveryLastResult)

MigrationTelemetryHealthRecoveryEventId =
  digest(
    "d2b:panel:migration-telemetry-health-recovery-audit:v1",
    MigrationControllerEpoch,
    MigrationTelemetryHealthRecoveryIdentity
  )

MigrationTelemetryHealthRecoveryEvent =
  Success {
    recovery_event_id,
    controller_epoch,
    telemetry_failure_alias,
    source_failure_digest,
    probe_result_digest,
    prepared_stable_state,
    normal_health_digest,
    protected_operator_audit_digest: ProtectedOperatorAuditDigest
  }
  | Failure {
      recovery_event_id,
      controller_epoch,
      telemetry_failure_alias,
      source_failure_digest,
      failed_recovery_state_tag,
      failed_recovery_state_digest,
      closed_failure_code,
      protected_operator_audit_digest: ProtectedOperatorAuditDigest
    }
```

The fixed current-and-shadow `MigrationTelemetryHealthMarkerReserve` is not
the preflight signal, control-conflict signal, execution, control, integrity,
status, or accepted-conflict reserve. Its failure latch is separately reserved
controller core metadata, not a field hidden inside the marker. The current
failure-alias record is a third independently sealed fixed record. It stores
the controller-issued, non-capability alias for the exact current non-healthy
cycle, including a corrupt marker that cannot safely carry its own alias.
Startup reconciles this fixed record before status admission; when marker
corruption has no trustworthy prior alias, it mints and fsyncs a fresh alias in
the independent record without rewriting the marker. Thus even the corrupt
marker observation has a current server-issued failure alias. If that fixed
record cannot be made durable, status admission fails closed before returning
a telemetry observation; no non-healthy tag is ever emitted without its
alias.

The durable admission boundary for any fallible detailed, aggregate, summary,
normal-health, or exporter write is the fsynced transaction that installs
`ArmPending` and `MigrationTelemetryCurrentFailureAliasRecord::Current` with
fresh controller-issued write and failure identities. Before that commit no
telemetry operation is admitted and no telemetry write may be attempted. Once
it commits, failure at every point before `UpdatePending`, including failure
to advance or verify `Armed`, leaves `ArmPending`, `Armed`, or an integrity
fault plus the independently durable current failure alias. Restart therefore
returns `ArmedFailureLatch`, `CorruptMarker`, or `HealthUnavailable`; it can
never return the old or shadow `Healthy`.

Only an exact verified `Armed` value may install the preallocated
`UpdatePending` marker with the same failure alias, controller epoch, next
monotonic marker sequence, expected prior sequence and digest, and update-plan
digest. Only then may normal telemetry bytes be touched. Successful normal
closure writes and verifies normal health, compare-and-swaps that exact
`UpdatePending` marker to one exact stable tag, and atomically records
`StableClosureProven` with the same alias and stable and normal-health digests.
Normal closure proves completion but does not clear the latch. A stable
degraded tag therefore remains recoverable and cannot drift back to healthy
through later ordinary telemetry success.

The exact normal transitions are:

```
FailureLatch: Clear -> ArmPending -> Armed -> StableClosureProven
Marker:       Stable* -> UpdatePending -> Stable*
```

There is no direct `ArmPending` or `Armed` to stable edge. Any crash or write
failure after `UpdatePending` leaves `UpdatePending`, the armed latch, or an
integrity mismatch durable. Startup never rolls back to a shadow healthy
marker. `DetailedDegraded`, `AggregateDegraded`, `ExporterDegraded`,
`HealthUnavailable`, `UpdatePending`, `RecoveryBarrier`, `CorruptMarker`, and
`ArmedFailureLatch` each return the current server-issued
`TelemetryFailureAlias` and the exact
`Operator.RecoverMigrationTelemetryHealth` action. `Healthy` alone owns
`NoFurtherAction`.

`RecoverMigrationTelemetryHealth` authenticates `ProtectedOperator` and uses
only the separately sealed `MigrationTelemetryHealthRecoverySlot` and its
fixed audit capacity. Its request carries the server-issued
`TelemetryFailureAlias` from the causing observation or retryable product.
The controller resolves that one accepted identifier against the active
request binding and reusable last-result slot before treating it as the
current alias for a fresh cycle. A bound active alias resolves to the exact
recovery identity, epoch, marker, latch, normal-health, failure-alias record,
and recovery-slot digests. A settled alias plus the same canonical request
digest returns the sealed last result. Only the current alias with no matching
active or settled request may create a fresh controller-issued recovery
identity, store the bounded
`MigrationTelemetryHealthRecoveryRequestBinding`, seal the mandatory
operator-attribution prefix in the fixed audit workspace, and enter
`RecoveryRequested`. The binding contains only that accepted failure alias,
epoch, marker, latch, failure-alias-record, and canonical request digests shown
above. A byte-identical retry with the same alias joins that identity and
state. An unrelated stale alias or changed bytes cannot replace the identity,
request binding, barrier, probes, event, or result. Authentication, decoding,
alias-resolution, or sealed capacity failure creates no recovery identity or
health change. No request variant accepts a recovery-identity parameter.

Recovery performs two semantic marker compare-and-swaps, with the durable
outbox and acknowledgement substates between them. First it advances only the
recovery slot to `RecoveryBarrierInstalled` and advances the marker to
`RecoveryBarrier` with the same failure alias; the latch remains its exact
source latch state. `RecoveryBarrierInstalled` is a recovery-slot variant
only. It is not a marker or latch variant. The controller then verifies both
marker copies, latch, closure, failure-alias record, and the complete detailed,
aggregate, summary, and exporter probe set. It freezes the prepared
normal-health value, exact stable tag, one success event id, canonical event
bytes, and exact sealed response in `SuccessAuditOutboxPending`. A closed
recovery failure instead freezes its exact state tag and digest,
`closed_failure_code`, proposed next failure alias, failure event id,
canonical failure-event bytes, and exact sealed response in
`FailureAuditOutboxPending`. The success and failure event variants are
disjoint; failure cannot populate success-only probe or stable-state fields.
Retry appends only the frozen bytes for the chosen outcome. Unknown append
outcome remains in the matching
`SuccessAuditSinkAcknowledgementPending` or
`FailureAuditSinkAcknowledgementPending` state until the sink returns the
durable acknowledgement for those exact bytes.

Only `SuccessAuditAcknowledgedInstallPending` may install the prepared
higher-sequence stable marker, matching normal-health digest,
`FailureLatch::Clear`, and either `NoneHealthy` or a new current failure-alias
record for a probed degraded result. Every probe passing installs
`StableHealthy`; otherwise it installs the exact stable degraded tag and keeps
recovery available under its new server-issued failure alias. That same
transaction writes `LastResult::Succeeded` to the separate fixed reusable
last-result slot with the request-binding digest, event and acknowledgement
digests, and sealed response, then makes the recovery work slot reusable.

Only `FailureAuditAcknowledgedSettlePending` may rotate the current telemetry
failure alias to the proposed next alias, write `LastResult::Failed` with the
exact `closed_failure_code`, event and acknowledgement digests, and sealed
response to that same last-result slot, and make the recovery work slot
reusable for a fresh recovery cycle. A closed
failure therefore cannot rotate an alias, return its response, or release the
slot before its failure audit settles. Audit construction, storage, append,
acknowledgement, or final-settlement failure remains nonterminal under the
same recovery identity and same canonical event bytes.

Startup reconciles every success and failure state and the separate
last-result slot. It adopts the durable request binding and matching sink
reservation, replays only the frozen canonical bytes, obtains or reuses the
sink acknowledgement, and finishes the one success install or failure
settlement. A crash after acknowledgement resumes only that final transition.
A crash before success install cannot expose healthy; a crash before failure
settlement cannot rotate the alias or reuse the work slot. The last-result
slot remains readable while a later recovery is active and is overwritten
only when that later outcome settles. A byte-identical retry matched by its
accepted `TelemetryFailureAlias` and request-binding digest returns the exact
stored sealed response from that slot without another event.

Both recovery audit variants omit `protected_operator_alias` and always carry
the deployment-keyed `ProtectedOperatorAuditDigest`. The authenticated
request transaction places that digest directly in the preallocated canonical
audit prefix, not in recovery-slot fields. All other surfaces use neither
operator alias nor audit digest. The recovery event id is independent of
marker sequence and is assigned once per internal recovery identity. Each
success or failure event carries the accepted `TelemetryFailureAlias`, not an
external recovery-identity alias. Each outbox freezes one canonical byte
string; the same id with different bytes is refused before append.

Both normal and recovery closure compare controller epoch, expected prior
marker sequence, marker digest and tag, write or recovery identity, exact latch
digest and tag, and the current failure-alias record digest. A delayed
`UpdatePending` or `RecoveryBarrier` closure is
`migration-telemetry-health-barrier-stale` and changes no marker, latch,
failure alias, normal telemetry, or recovery slot. It therefore cannot
overwrite a newer detailed, aggregate, exporter, unavailable, or corrupt
degradation. The generated telemetry threshold uses
`REKEY_ADMISSION_THRESHOLD[TelemetryMarkerSequence]`; reaching it closes
ordinary telemetry updates while budgeted closure or counter-independent
barrier migration remains available. A legacy value above threshold is moved
through the rekey continuation protocol rather than incremented.

For simultaneous telemetry conditions, observed precedence is
`CorruptMarker`, `ArmedFailureLatch`, `RecoveryBarrier`, `UpdatePending`,
`HealthUnavailable`, `AggregateDegraded`, `DetailedDegraded`,
`ExporterDegraded`, then `Healthy`. A detailed-capacity fallback may be
committed as `DetailedDegraded`, an aggregate-capacity fallback as
`AggregateDegraded`, and an exporter refusal as `ExporterDegraded` only when
the complete barrier-protected update succeeds. Marker or normal-health
failure selects the applicable unavailable or corrupt tag and remains so
until explicit recovery.
`ReadRetentionRecoveryStatus`, `ReadRetentionRecoveryState`, and
`ReadMigrationRecoveryStatus` derive the observation by verifying both marker
slots and the matching normal-health digest. A read or verification failure
selects `CorruptMarker` when marker integrity is classifiable and otherwise
`HealthUnavailable`, always with the current independently stored failure
alias and recovery action. After failure-alias reconciliation it cannot fail
the status read or alter blocker, integrity, migration, or refusal fields; an
unrecoverable alias record is the earlier fail-closed status-admission case.

Metrics are
`migration_preflight_replay_conflict_signals_total`,
`migration_preflight_signal_overflow_buckets_total`, and
`migration_preflight_signal_reserve_used_ratio`, plus
`migration_control_conflict_signals_total`,
`migration_control_conflict_overflow_buckets_total`,
`migration_control_conflict_signal_reserve_used_ratio`, and
`migration_telemetry_health`. Their exact label allowlist is:

```
MigrationTelemetryMetricLabel =
  operation_class:
      MigrationReplayConflict
      | Resume
      | Fence
      | SinkRepair
      | AuditActivation
      | ControlReserveRepair
      | ControllerEpochRekey
      | TelemetryHealthRecovery
  | conflict_kind:
      ReplayConflict
      | ChangedBytes
      | StaleGeneration
  | signal_outcome: Detailed | Aggregate | Unrecorded
  | reserve_role:
      PreflightDetailed
      | PreflightAggregate
      | ControlDetailed
      | ControlAggregate
      | AggregateSummary
      | TelemetryHealth
      | TelemetryHealthRecovery
  | health_state:
      Healthy
      | DetailedDegraded
      | AggregateDegraded
      | ExporterDegraded
      | UpdatePending
      | RecoveryBarrier
      | CorruptMarker
      | ArmedFailureLatch
      | HealthUnavailable
```

Each metric declares only the applicable subset of that closed list.
Capacity generation, signal id, request or key digest, attempt identity, peer,
path, and every other unbounded value are forbidden labels. Exact capacity
generation is absent from the refusal, detailed signal, aggregate, summary,
health state, log, and exporter product. It remains available only in
protected status and audit fields.
Each signal counter advances only when a detailed identity or overflow bucket
first becomes durable in its own window; exact retries do not increment it.
Exporter failure cannot change durable counters and instead degrades health
when possible.

Every migration preflight or control refusal response uses:

```
MigrationRefusalEnvelope = {
  original_refusal: ExactMigrationPreflightOrControlRefusalProduct,
  telemetry_health_observation: MigrationTelemetryHealthObservation
}
```

For an admitted child command, the original refusal product and its distinct
child-command audit event settle before the priority response frame is
released. For migration preflight, the refusal frame is fixed before any
diagnostic work. In both cases the original frame is made available before
the controller attempts a detailed signal, aggregate, summary, marker,
normal-health, or exporter write. The health field is a
separate closed observation and is never inserted into or substituted for the
original product. The transport makes the priority refusal frame available to
the caller before the telemetry follow-up; telemetry has a bounded
nonblocking attempt and otherwise supplies
`HealthUnavailable { reason: TelemetryFollowupUnavailable,
telemetry_failure_alias, next: Operator.RecoverMigrationTelemetryHealth }`.
A blocked signal,
aggregate, marker, normal-health, or exporter path therefore proves that the
original refusal returns first. Failure at any or all of those writes returns
the same exact refusal. No telemetry failure consumes, delays, or refuses
`MigrationExecutionReserve`, `MigrationControlReserve`,
`MigrationIntegrityReserve`, or the transient migration lane.

Authenticated stale-generation and changed-byte child commands always attempt
their bounded fixed-cardinality signal after their audited refusal has been
released. The signal uses only
`MigrationControlConflictSignalReserve`; its health barrier and observation
use only `MigrationTelemetryHealthMarkerReserve`. Neither can address a child
command slot, child-audit outbox, sink-repair workspace, or integrity
workspace. Detailed-slot exhaustion uses the one overflow bitmap, summary-ring
pressure applies the already settled oldest-slot replacement rule, and D17
TTL expiry reuses storage; none changes the child result.

Simultaneous-failure precedence is first the original structural,
authorization, eligibility, stale-generation, tuple-mismatch, or
changed-byte refusal selected by its operation's normal order; then the
separate health observation precedence above; then `Detailed`, `Aggregate`,
or `Unrecorded` only as diagnostic outcome. Telemetry can never mask or
upgrade the original refusal. Unauthorized and current-state-ineligible
requests attempt no conflict signal, aggregate, exporter, or marker
transition; their envelopes carry only the independently derived current
health observation.

Repetition re-derives the same safe conflict correlation and the same signal
identity from the durable base request digest and still allocates no accepted
attempt record. Structural, authorization, and eligibility validation precede
conflict classification. A request that fails any of them remains that earlier
preflight refusal even when its caller key and bytes also conflict; it emits no
detailed signal, aggregate mutation, metric, exporter write, or health
transition.

A valid non-conflict request then atomically acquires the transient preflight
hold and binds the sealed `MigrationExecutionReserve`. Missing raw destination capacity is
`retention-capacity-migration-raw-capacity-unavailable`. An accepted migration
already holding the lane is `retention-capacity-migration-already-active` and
returns that safe `AttemptIdentity` for status and resume. Either preflight
refusal releases the caller's transient hold without charging the execution
reserve.

After acceptance, migration is one recoverable nonterminal logical operation
under the same `AttemptIdentity`. It recomputes all reservations from the old
and new bounded schemas, proves that every existing blocker has a complete
route, copies every permanent replay and audit index including every
`AssignmentIssuanceAuditRepairTombstone` and
`AssignmentIssuanceCancellationAuditRepairTombstone` keyed by exact
`AttemptIdentity`, verifies their entry counts, byte counts, fields, and
digests against the reviewed destination manifest, and atomically changes the
capacity generation. The destination permanent-tombstone capacity includes
the exact serialized maxima of both record types before the switch is
admissible. An execution, storage, or verification fault does not terminally
consume the attempt or its one emergency route. It durably enters `Paused`
with a closed safe reason, bounded deadline, owner epoch alias, and exact
repair-and-resume action.
`ReadProtectedAttemptStatus` reports that state. Automatic recovery resumes by
the deadline only for `PausedSelfClearingWait` and only after its exact
controller-verified prerequisite clears. Every
`PausedOperatorRepairRequired` remains pending across deadlines, restart, and
lease-expiry takeover until its exact prerequisite is verified and an
authenticated, audited `ResumeProtectedAttempt` succeeds. The protected
operator's `FenceProtectedAttempt` may fence an expired worker but preserves
that repair plan; it cannot clear the prerequisite or resume the attempt.
Completion
releases the transient lane after every reserved permanent record is either
durable or proven unnecessary by the closed terminal path. The completed
records remain charged only to the source generation's execution allocation.
A preflight refusal leaves both allocations reusable.
There is no terminal execution-failure variant that consumes the only
migration edge and forces a second migration.

There is no offline controller mutation entrypoint. If the controller cannot
open the emergency lane because physical destination capacity is absent, a
deployment administrator may provision raw empty storage or capacity without
reading, interpreting, copying, or mutating any controller record. The
controller remains the only reader and writer of authority state. After raw
capacity exists, the authenticated protected operator submits the same
reviewed manifest to the normal `MigrateRetentionCapacity` endpoint, which
uses accepted-attempt auditing and the resumable operation above. The
preflight refusal is `retention-capacity-migration-raw-capacity-unavailable`;
its remedy names raw provisioning and then the normal endpoint, never an
offline migration. The controller never silently rebuilds reserve accounting
from the full store.

Ordinary abandonment creates or preserves a resumable capsule and therefore
does not release general state; only a later explicit permanent close can make
that capsule eligible.

All new durable and observable surfaces use declared bounded redacting types,
closed identifiers, closed enums, safe aliases, or digests:

- protected ledger and prompt views may reveal bounded issue text only to the
  dispatched seat;
- protected identity and rationale mappings are never public;
- public review and publication projections contain only safe aliases, issue
  ids, severities, closed dispositions and outcomes, bounded numerics,
  timestamps, and digests;
- an assignment refusal, audit event, log, status result, or `Debug` projection
  may carry only the presented assignment's non-capability
  `PresentedAssignmentAlias` and issue ids already supplied by that caller; it
  never carries a foreign owning assignment identity, foreign safe alias, or
  either assignment's opaque capability handle;
- a raw `RiskOperationHandle` appears only in the protected intent response,
  a pending `ProtectedOperatorRiskRecoveryContext` returned by
  operator-authenticated `RecoveryRead.ReadRiskRecoveryState`, and that
  handle's exact protected mutation request. It is never present in generic
  `ProtectedAttemptRecovery` or `ReadProtectedAttemptStatus`, logged, audited,
  exported, included in a metric, tombstone, refusal, public status, derived
  or handwritten `Debug`, or exposed through an original-peer recovery read;
- completion refusals carry exactly one
  `AssignmentCompletionRefusalProduct` with `CompletionEvidenceAlias`, and
  append-authorization refusals carry exactly one
  `AuditAppendAuthorizationRefusalProduct` with `SinkNamespaceAlias`,
  `SinkReservationAlias`, `AppendAuthorizationAlias`, and `AuditEventAlias`;
  local errors, the catalog, tombstones, recovery and retention projections,
  logs, audit, status, `Debug`, and fixtures cannot add
  `ControllerNamespaceAlias`, a raw identity, namespace, handle, path, or
  deployment id;
- migration-status-disclosure and both telemetry-recovery outcome audit events
  omit `protected_operator_alias` and carry a mandatory deployment-keyed
  `ProtectedOperatorAuditDigest` only in canonical audit event bytes. The
  digest is forbidden from logs, errors, status, refusal products,
  reservations, metrics, `Debug`, and every other schema;
- logs and errors do not render raw recommendations, rationales, legacy
  strings, paths, branch names, user identities, run handles, or evidence
  bytes; and
- no governed type exposes those values through derived or handwritten
  `Debug`.

The exactly-one authoritative audit boundary is durable accepted-attempt
registration, not socket accept and not state mutation. The protected front
door first authenticates the peer, parses the bounded envelope, checks
the endpoint and operation discriminants are syntactically bounded, reserves
controller capacity, and derives the mandatory `ProtectedAttemptId` and
`AttemptIdentity`. It then runs the four-step `AcceptancePrepare`, sink
`Prepared`, accepted-journal promotion, and sink-binding protocol above. Only
the promoted `AcceptedAttemptJournal` is accepted, and operation processing
waits for `AcceptedBound`. From the accepted-journal commit, the attempt must
recover to exactly one typed success or refusal event even if the caller never
retries.

A connection failure, malformed frame, authentication failure, unavailable
front door, or capacity failure before durable accepted-attempt registration
is a transport or preflight event. It has no authoritative effect, is not an
accepted endpoint attempt, does not enter terminal attempt metrics, and does
not claim the exactly-one authoritative audit guarantee. A peer that
authenticates and submits a bounded request for an absent or unauthorized
operation is durably registered before that policy check runs; it then
receives the ordinary exactly-one refusal audit. There are six narrow
exception classes. `MigrateRetentionCapacity` performs structural, authorization,
eligibility, same-key conflict, raw-capacity, execution-reserve, and
transient-lane checks as preflight by the re-entrant migration contract above;
their refusals are not accepted attempts and consume neither the execution
reserve nor emergency lane. `ReadMigrationRecoveryStatus` is an authenticated
observational read whose fixed slot emits one distinct event per successful
disclosure rather than using a caller key; it always evaluates current state
and never enters mutation idempotency or a mutation slot. The five
`MigrationControlCommand` operations are authenticated parent-attempt child
transitions. They use fixed slots and one distinct bounded child audit event
per admitted success or refusal, not standalone accepted-attempt
registration. `RevokeImplementationAssignment` uses the assignment's
dedicated pending, outbox, no-append rebind, acknowledgement, and settlement
state rather than ordinary quarantined conversion.
`RekeyMigrationControllerEpoch` uses the counter-independent rekey record and
event identity rather than a child slot.
`RecoverMigrationTelemetryHealth` uses the separately sealed health-recovery
slot, failure latch, recovery barrier, and recovery audit. These last four
mutation classes authorize state change only through their exact sealed
records; none can fall through to generic accepted-attempt conversion.

Every ordinary accepted-attempt event has:

```
AuditEventId =
  digest(
    "d2b:panel:audit-event:v1",
    AttemptIdentity,
    audit_event_kind
  )
```

The event kind is a closed success or refusal variant. A conflict request uses
`AttemptIdentity::Conflict`, so its event cannot collide with the base
attempt's event. Event bytes are canonical and digest-only.
Child-command event identity is instead
`MigrationChildCommandAuditEventId` above; status access uses
`MigrationStatusAccessAuditEvent`, revocation uses its assignment-bound event,
rekey uses `MigrationEpochRekeyOutcomeEventId`, and telemetry recovery uses its
recovery-slot event. None is an `AuditEventId` over a standalone
`AttemptIdentity`.

The append sink retains ADR 0053 D17's root-owned, append-only, write-once,
daily-rotated, bounded, synchronously flushed shape and adds atomic idempotent
append. In one durable operation it records the canonical event and an index
from `AuditEventId` to event digest, location, and original
`AuditAppendAcknowledgement`, creates the permanent `AuditAppendTombstone`,
and consumes the bound `AuditSinkReservation`, then fsyncs before returning
that acknowledgement. The same id and byte-identical event returns the
original acknowledgement without appending, even after acknowledgement loss
or raw event rotation. The same id with different bytes is
`audit-event-id-conflict` and appends nothing.

That permanent append tombstone rule applies to ordinary accepted-attempt
events. A migration child event uses the child-audit sink reservation sealed
inside the parent migration reserve, an authorization bound to the parent
migration, controller epoch, reserve incarnation, fixed slot, child sequence,
child audit sequence, pre-state generation, event id, and digest, and a bounded D17
sink idempotency row. The controller's fixed slot retains the current or last
result and audit digest. After D17 expiry the sink may release that event and
idempotency row; no permanent child tombstone is created, and the old epoch,
incarnation, generation, and sequence remain stale. Status-disclosure events
use the analogous fixed integrity-reserve slot and bounded sink row, but each
successful disclosure receives a fresh controller-issued identity and event.

Every appendable `AuditSinkReservation` generation has exactly one authorized
`AuditEventId` and event digest. An append request carries the reservation id,
generation, event id and digest, canonical event bytes, and an unforgeable
controller `AuditAppendAuthorization` binding that complete tuple to the
accepted journal. The sink verifies the authorization independently and
rejects every invalid tuple without writing bytes.

Raw sink namespaces, authorization bytes or handles, reservation handles,
event handles, paths, and deployment ids never appear in an append refusal or
any governed projection of it. The only alias types admitted to the product
are these domain-separated non-capabilities:

```
SinkNamespaceAlias =
  digest("d2b:panel:sink-namespace-alias:v1", internal_sink_namespace_id)

SinkReservationAlias =
  digest("d2b:panel:sink-reservation-alias:v1", internal_reservation_id)

AppendAuthorizationAlias =
  digest("d2b:panel:append-authorization-alias:v1", authorization_identity)

AuditEventAlias =
  digest("d2b:panel:audit-event-alias:v1", AuditEventId)
```

`ControllerNamespaceAlias` remains a separate safe correlate for
controller-namespace capacity refusals such as
`replay-tombstone-store-full`; it is not an append-refusal field:

```
ControllerNamespaceAlias =
  digest("d2b:panel:controller-namespace-alias:v1", controller_namespace)
```

Every append-authorization refusal and every governed serialization of that
refusal uses exactly one variant of:

```
AuditAppendAuthorizationRefusalProduct =
  Invalid {
    sink_namespace_alias,
    append_authorization_alias,
    validation_code: AppendAuthorizationValidationCode
  }
  | CrossAttempt {
      sink_namespace_alias,
      append_authorization_alias,
      request_attempt_identity,
      authorization_attempt_identity,
      sink_reservation_alias,
      presented_generation
    }
  | StaleGeneration {
      sink_namespace_alias,
      request_attempt_identity,
      sink_reservation_alias,
      presented_generation,
      current_generation,
      generation_code: Past | Future
    }
  | Unbound {
      sink_namespace_alias,
      request_attempt_identity,
      sink_reservation_alias,
      current_generation,
      append_authorization_alias,
      binding_code: CurrentGenerationHasNoAuthorizedEvent
    }
  | EventMismatch {
      sink_namespace_alias,
      request_attempt_identity,
      sink_reservation_alias,
      current_generation,
      authorized_event_alias,
      presented_event_alias,
      authorized_event_digest,
      presented_event_digest,
      mismatch_code: EventId | EventDigest | EventIdAndDigest
    }
```

This tagged product is the allowlist. The local error, canonical refusal audit
event, refusal catalog row, `AttemptTombstone`,
`ProtectedAttemptRecovery::OriginalRefusal`, protected status, retention
projection, log, derived or handwritten `Debug`, and test fixture serialize
the canonical bytes of this exact product with no wrapper-owned field. A field
addition, removal, substitution, alias widening, or product-specific metadata
is invalid on every surface. In particular, `ControllerNamespaceAlias` is
absent everywhere in this product family.

Refusal evaluation is ordered and disjoint:

1. a forged, malformed, incorrectly signed, or otherwise unverifiable
   authorization is `audit-append-authorization-invalid`;
2. a valid authorization naming an `AttemptIdentity` other than the request
   is `audit-append-authorization-cross-attempt`;
3. a valid same-attempt authorization naming any past or future reservation
   generation is `audit-sink-generation-stale`;
4. only at the current generation, absence of an authorized event is
   `audit-append-authorization-unbound`;
5. only at the current generation with one authorized event, a wrong event id,
   digest, or both is `audit-append-authorization-event-mismatch`; and
6. only the remaining exact tuple may append.

A future generation is therefore always stale and can never be unbound. The
local refusal, audit event, catalog row, `AttemptTombstone`,
`ProtectedAttemptRecovery::OriginalRefusal`, status and retention projections,
logs, `Debug`, and fixtures use the identical product above.
A higher-precedence predicate cannot fall through to a lower one or reveal
that lower predicate's expected binding.
Preparing capacity and binding an accepted journal authorize no append; the
controller binds the generation's one event only after the complete
quarantined tuple exists.

Attempt processing is fenced. The controller issues a monotonically increasing
`WorkerEpoch` per `AttemptIdentity` and stores a generation on every state. A
worker claims only by compare-and-swap and every later write supplies both
epoch and generation. Lease renewal and a deliberate pause are durable
transitions. A stale worker write is `attempt-worker-fenced` and cannot alter
the journal, sink authorization, effect, result, outbox, or response.

The three assignment-issuance recovery products are closed. Every variant of
each repair state below carries the complete
`AssignmentIssuanceRepairWorkspace`; the displayed fields are its
phase-specific fixed slots:

```
AssignmentIssuancePreparedRecoveryState =
  ResumePreparedHandlerPending {
    issuance_prepare_alias,
    prepare_incarnation,
    sink_activation_proof_digest,
    assignment_binding_digest,
    prepared_activation_binding_digest,
    canonical_issuance_audit_tuple_digest,
    next: ResumePreparedAssignmentIssuanceHandler
  }
  | CancellationSinkFencePending {
      issuance_prepare_alias,
      prepare_incarnation,
      cancellation_reason_code,
      next_prepare_incarnation,
      prepared_cancellation_pre_proof_binding_digest,
      cancellation_refusal_audit_tuple_digest,
      next: FencePreparedAssignmentIssuanceForCancellation
    }
  | CancellationSinkProofPending {
      issuance_prepare_alias,
      prepare_incarnation,
      sink_non_creatable_fence_proof_digest,
      cancellation_reason_code,
      next_prepare_incarnation,
      prepared_cancellation_pre_proof_binding_digest,
      cancellation_refusal_audit_tuple_digest,
      next: ProofCancelPreparedAssignmentIssuanceSinkReservation
    }
  | CancellationRefusalInstallPending {
      issuance_prepare_alias,
      prepare_incarnation,
      sink_non_creatable_fence_proof_digest,
      sink_absence_or_cancellation_proof_digest,
      cancellation_reason_code,
      next_prepare_incarnation,
      prepared_cancellation_activation_binding_digest,
      cancellation_refusal_audit_tuple_digest,
      next: InstallPreparedAssignmentIssuanceCancellationRefusal
    }

AssignmentIssuanceAuditRepairState =
  IntentRecorded {
    repair_id,
    old_reservation_generation,
    authenticated_definite_no_append_proof_digest,
    next: IssueAssignmentIssuanceOldGenerationInvalidation
  }
  | OldGenerationInvalidationPending {
      repair_id,
      old_reservation_generation,
      authenticated_definite_no_append_proof_digest,
      next: ReplayAssignmentIssuanceOldGenerationInvalidation
    }
  | OldGenerationInvalidatedRebindPending {
      repair_id,
      invalidated_reservation_generation,
      authenticated_definite_no_append_proof_digest,
      invalidation_proof_digest,
      next: BindAssignmentIssuanceSuccessReplacementGeneration
    }
  | ReplacementGenerationRolloverPending {
      repair_id,
      invalidated_reservation_generation,
      authenticated_definite_no_append_proof_digest,
      invalidation_proof_digest,
      next: RolloverAssignmentIssuanceSuccessReplacementGeneration
    }
  | SuccessGenerationBoundReplacementPending {
      repair_id,
      invalidated_reservation_generation,
      authenticated_definite_no_append_proof_digest,
      invalidation_proof_digest,
      replacement_reservation_generation,
      rebind_or_rollover_proof_digest,
      next: CommitAssignmentIssuanceSuccessReplacementTuple
    }
  | ReplacementTupleInstalled {
      repair_id,
      replacement_reservation_generation,
      next: ReplayAssignmentIssuanceSuccessAppend
    }
  | ReplacementSinkAcknowledgementPending {
      repair_id,
      replacement_reservation_generation,
      next: QueryOrReplayAssignmentIssuanceSuccessAppend
    }
  | ReplacementAcknowledgedActivationPending {
      repair_id,
      replacement_reservation_generation,
      final_acknowledgement_digest,
      repair_floor:
        DurableRepairTombstone { tombstone_digest }
        | DurableRepairTombstonePreparation { preparation_digest },
      next: ActivatePreparedImplementationAssignment
    }

AssignmentIssuanceCancellationAuditRepairState =
  IntentRecorded {
    repair_id,
    old_reservation_generation,
    authenticated_definite_no_append_proof_digest,
    next: IssueAssignmentIssuanceCancellationOldGenerationInvalidation
  }
  | OldGenerationInvalidationPending {
      repair_id,
      old_reservation_generation,
      authenticated_definite_no_append_proof_digest,
      next: ReplayAssignmentIssuanceCancellationOldGenerationInvalidation
    }
  | OldGenerationInvalidatedRebindPending {
      repair_id,
      invalidated_reservation_generation,
      authenticated_definite_no_append_proof_digest,
      invalidation_proof_digest,
      next: BindAssignmentIssuanceCancellationRefusalReplacementGeneration
    }
  | ReplacementGenerationRolloverPending {
      repair_id,
      invalidated_reservation_generation,
      authenticated_definite_no_append_proof_digest,
      invalidation_proof_digest,
      next:
        RolloverAssignmentIssuanceCancellationRefusalReplacementGeneration
    }
  | RefusalGenerationBoundReplacementPending {
      repair_id,
      invalidated_reservation_generation,
      authenticated_definite_no_append_proof_digest,
      invalidation_proof_digest,
      replacement_reservation_generation,
      rebind_or_rollover_proof_digest,
      next: CommitAssignmentIssuanceCancellationRefusalReplacementTuple
    }
  | ReplacementTupleInstalled {
      repair_id,
      replacement_reservation_generation,
      next: ReplayAssignmentIssuanceCancellationRefusalAppend
    }
  | ReplacementSinkAcknowledgementPending {
      repair_id,
      replacement_reservation_generation,
      next: QueryOrReplayAssignmentIssuanceCancellationRefusalAppend
    }
  | ReplacementAcknowledgedActivationPending {
      repair_id,
      replacement_reservation_generation,
      final_acknowledgement_digest,
      repair_floor:
        DurableRepairTombstone { tombstone_digest }
        | DurableRepairTombstonePreparation { preparation_digest },
      next: ActivatePreparedAssignmentIssuanceCancellation
    }
```

The linear recoverable state is:

```
AcceptancePreparePending
-> AcceptedUnclaimed { sink_binding = Prepared }
-> AcceptedUnclaimed { sink_binding = AcceptedBound }
-> Processing { worker_epoch, lease_until }
-> Paused {
     worker_epoch,
     lease_until,
     reason:
       SelfClearing(MigrationSelfClearingWaitV1)
       | OperatorRepair(MigrationOperatorRepairPlanV1),
     pause_deadline
   }
-> Processing { worker_epoch, lease_until }
or, only for `IssueImplementationAssignment` after
`PreparedForOrdinaryAudit` and before a handler tuple:
-> AssignmentIssuancePreparedRecovery {
     state: AssignmentIssuancePreparedRecoveryState
   }
-> QuarantinedPendingAudit {
     worker_epoch,
     lease_until,
     closed_result,
     quarantined_authority_effect,
     replay_result,
     outbox,
     response = Unavailable
   }
-> OrdinarySinkAcknowledgementPending {
     reservation_generation,
     authorized_event_id,
     authorized_event_digest
   }
-> OrdinaryActivationPending {
     reservation_generation,
     audit_acknowledgement = Persisted
   }
or, only for an issuance success with authenticated definite-no-append:
-> AssignmentIssuanceAuditRepair {
     unchanged_success_event,
     prepare_incarnation,
     state: AssignmentIssuanceAuditRepairState
   }
-> Completed {
     assignment_issuance_repair_tombstone = Durable,
     authority_effect = Activated,
     response = PayloadAvailable,
     tombstone = Durable,
     retained_repair_intent = OrdinaryEligibleRoundInput,
     remaining_current_cycle_source_bytes = OrdinaryEligibleRoundInput
   }
or, only for a prepared-cancellation refusal with authenticated
definite-no-append:
-> AssignmentIssuanceCancellationAuditRepair {
     unchanged_cancellation_refusal_event,
     prepare_incarnation,
     state: AssignmentIssuanceCancellationAuditRepairState
   }
-> Completed {
     assignment_issuance_cancellation_repair_tombstone = Durable,
     authority_effect = None,
     response = PayloadAvailable,
     tombstone = Durable,
     retained_repair_intent = OrdinaryEligibleRoundInput,
     remaining_current_cycle_source_bytes = OrdinaryEligibleRoundInput
   }
or, only for `MigrateRetentionCapacity`:
-> MigrationSinkAcknowledgementPending {
     reservation_generation,
     success_event_alias,
     success_event_digest
   }
-> MigrationActivationPending {
     reservation_generation,
     audit_acknowledgement = Persisted
   }
-> Completed {
     authority_effect = ActivatedOrNone,
     assignment_use = ActivatedOrNone,
     response = PayloadAvailable,
     tombstone = Durable
   }
-> Completed { response = EvictionPrepared }
-> Completed { response = PayloadEvicted }
```

`AcceptancePreparePending` is non-authoritative; promotion creates
`AcceptedUnclaimed`. The pause branch is optional. After an authenticated
definite-no-append result for an ordinary tuple other than a prepared
assignment-issuance success or prepared-cancellation refusal, one atomic
compare-and-swap
starts conversion from the exact
`OrdinarySinkAcknowledgementPending { old_reservation_generation,
authorized_event_id, authorized_event_digest }` state and exact old tuple. It
cannot start from `QuarantinedPendingAudit`, a generic `PendingAuditOutbox`
without the matching authorized tuple, or any later state. The closed generic
branch is:

```
OrdinarySinkAcknowledgementPending { old_reservation_generation, old_event }
-> AuditConversionIntentRecorded
-> AuditConversionOldGenerationInvalidationPending
-> AuditConversionOldGenerationInvalidatedRebindPending
-> AuditConversionRefusalGenerationBoundReplacementPending
-> AuditConversionReplacementTupleInstalled
-> OrdinarySinkAcknowledgementPending { replacement_generation }
```

A prepared `IssueImplementationAssignment` success, its canonical
prepared-cancellation refusal, and `MigrateRetentionCapacity` are excluded
from that conversion by stored operation, outcome, and prepared-cancellation
binding. An issuance success enters its dedicated repair state and retains
its prepared assignment effect. A prepared-cancellation refusal enters its
separate dedicated repair state and retains all quarantined release and
restoration effects. An issuance refusal with no prepared effect remains an
ordinary no-effect tuple. Both assignment repair families use this same
fixed-workspace loop, with outcome-specific action names:

```
IntentRecorded { current_generation, accumulator_root, retry_count }
-> OldGenerationInvalidationPending
-> OldGenerationInvalidatedRebindPending
-> SuccessOrRefusalGenerationBoundReplacementPending
or, when the finite generation would wrap:
-> ReplacementGenerationRolloverPending
-> SuccessOrRefusalGenerationBoundReplacementPending
-> ReplacementTupleInstalled {
     current_generation = replacement_generation,
     accumulator_root = Folded,
     retry_count = SaturatingIncrement,
     folded_definite_no_append_proof = OrdinaryEligibleRoundInput,
     folded_invalidation_proof = OrdinaryEligibleRoundInput,
     folded_rebind_or_rollover_proof = OrdinaryEligibleRoundInput,
     fixed_definite_no_append_proof_slot = Reusable,
     fixed_invalidation_proof_slot = Reusable,
     fixed_rebind_or_rollover_proof_slot = Reusable
   }
-> ReplacementSinkAcknowledgementPending
-> ReplacementAcknowledgedActivationPending {
     proof_bound_final_acknowledgement,
     durable_tombstone_or_preparation
   }
or, on authenticated definite-no-append for that replacement:
-> OldGenerationInvalidationPending {
     current_generation = failed_replacement_generation
   }
```

The loop never constructs a generic conversion tuple and never allocates
another repair workspace or capacity reservation. A migration success
atomically starts this
nonterminal audit-repair branch from the same exact source state and old tuple:

```
MigrationSinkAcknowledgementPending {
  old_reservation_generation,
  unchanged_success_event
}
-> MigrationAuditRepairIntentRecorded
-> MigrationAuditRepairOldGenerationInvalidationPending
-> MigrationAuditRepairOldGenerationInvalidatedRebindPending
-> MigrationAuditRepairSuccessGenerationBoundReplacementPending
-> MigrationAuditRepairReplacementTupleInstalled {
     replacement_generation,
     unchanged_success_event
   }
-> MigrationSinkAcknowledgementPending {
     replacement_generation,
     unchanged_success_event
   }
```

The migration attempt identity, `MigrationExecutionReserve`, sibling
`MigrationControlReserve` and `MigrationIntegrityReserve`, quarantined
capacity-switch effect, success result, replay result, and canonical outbox
event remain unchanged and nonterminal through that branch. Only the sink
reservation generation changes. No migration state or crash recovery path can
construct `audit-event-flush-failed`.

Capacity migration is also the one operation whose execution or storage fault
must pause rather than create a
terminal fault result. A handler transaction atomically commits the
quarantined result, authority effect or none, replay result, exact outbox
event, and journal transition. For
`ReadImplementerIssueView`, the authority effect is its already-reserved
assignment use. A refusal and a genuinely stateless read use `None`; they
still commit the result and outbox together.

The controller next binds the current reservation generation to the outbox's
one event and durably enters `OrdinarySinkAcknowledgementPending` or
`MigrationSinkAcknowledgementPending` before sending the authorized append.
After the sink returns its original acknowledgement, one controller
transaction persists it, marks the outbox acknowledged, and enters the
matching `OrdinaryActivationPending` or `MigrationActivationPending`. The
ordinary final authority transaction activates the quarantined effect and
assignment use, advances the replay result to available, creates the immutable
tombstone, and marks the attempt `Completed`. For a prepared successful
`IssueImplementationAssignment`, that transaction is
`ActivatePreparedImplementationAssignment`. Its normal source is
`OrdinaryActivationPending::AssignmentIssuance` with the original
acknowledgement, matching prepare identity and incarnation, sink activation
proof, assignment binding, and canonical issuance audit tuple. Its only other
source is issuance-repair
`ReplacementAcknowledgedActivationPending` with those same prepared bindings,
the unchanged canonical event, proof-bound final acknowledgement, accumulator
root and retry count, and durable repair tombstone or preparation. From either
source it also installs `Active`, `Issued`, and settled evidence in the same
atomic authority transaction. For a final
`ReadImplementerIssueView` use, the same transaction installs
`RevocationCapacityReleasePending { Exhaust }`, never `Exhausted`; issue
readers and every assignment recovery context expose only the exact pending
release action until terminal installation. Migration instead requires
the `CompleteMigrationAuditActivation` child command through its fixed
`MigrationControlReserve` slot. No
effect, use, or response is visible before the applicable final transaction.
Thus a successful stateful read's use activation, terminal journal, replay
availability, and tombstone still commit atomically; its identical retry
cannot consume again.

A prepared-cancellation refusal's normal source uses the
`AssignmentIssuanceCancellation` nested ordinary activation instead of
`Generic`. Its only other source is cancellation-repair
`ReplacementAcknowledgedActivationPending`. Both carry the matching prepare
identity and incarnation, complete
`PreparedCancellationActivationBindingDigest`, fence and cancellation proof
digests, cancellation reason, reserved next incarnation, and canonical
refusal tuple digest; neither carries or accepts
`PreparedCancellationPreProofBindingDigest`. The normal source carries the
original acknowledgement, while the repair source carries the accumulator
root and retry count, proof-bound final acknowledgement, and durable repair
tombstone or preparation. From either
source `ActivatePreparedAssignmentIssuanceCancellation`, and no earlier
refusal-install, append, acknowledgement, or repair-loop transaction,
releases controller capacity, restores the evidence and request reservations,
returns the same request, exposes fresh-incarnation eligibility, makes refusal
replay available, creates the attempt tombstone, and terminalizes the old
attempt. Any normal-source transaction or storage failure leaves the exact
specialized ordinary activation state unchanged and returns only
`implementation-assignment-issuance-cancellation-activation-retryable`; a
repair-source failure stays in the repair state and returns only the dedicated
cancellation-audit-repair retryable refusal.

Every nonterminal status is a closed tagged
`PendingProtectedAttemptStatus`. Each variant owns all and only its safe fields
and its one exact action:

```
PendingProtectedAttemptStatus =
  AcceptancePreparePending {
    attempt_identity, prepare_id, deadline,
    action: CompleteOrCancelAcceptancePrepare
  }
  | AcceptedUnclaimedPrepared {
      attempt_identity, reservation_id, reservation_generation, deadline,
      action: BindAcceptedSinkReservation
    }
  | AcceptedUnclaimedAcceptedBound {
      attempt_identity, reservation_id, reservation_generation, deadline,
      action: ClaimAcceptedAttempt
    }
  | Processing {
      attempt_identity, owner_epoch_alias, lease_until, deadline,
      action: WaitForLeaseOrLeaseExpiryTakeover
    }
  | PausedSelfClearingWait {
      attempt_identity, owner_epoch_alias, lease_until,
      wait: MigrationSelfClearingWaitV1, deadline,
      action: WaitForExactPrerequisiteThenControllerResume
    }
  | PausedOperatorRepairRequired {
      attempt_identity, owner_epoch_alias, lease_until,
      repair_plan: MigrationOperatorRepairPlanV1, deadline,
      action: ExactFirstActionOwnedByRepairPlan
    }
  | QuarantinedPendingAudit {
      attempt_identity, owner_epoch_alias, lease_until,
      reservation_generation, authorized_event_id, authorized_event_digest,
      deadline, action: BindAndReplayAuthorizedAuditEvent
    }
  | AssignmentIssuancePreparedRecovery {
      attempt_identity, owner_epoch_alias,
      issuance_prepare_alias, prepare_incarnation,
      canonical_issuance_audit_tuple_digest, deadline,
      state: AssignmentIssuancePreparedRecoveryState
    }
  | AssignmentIssuanceAuditRepair {
      attempt_identity, issuance_prepare_alias, prepare_incarnation,
      unchanged_success_event_id, unchanged_success_event_digest,
      canonical_issuance_audit_tuple_digest, deadline,
      state: AssignmentIssuanceAuditRepairState
    }
  | AssignmentIssuanceCancellationAuditRepair {
      attempt_identity, issuance_prepare_alias, prepare_incarnation,
      unchanged_cancellation_refusal_event_id,
      unchanged_cancellation_refusal_event_digest,
      cancellation_refusal_audit_tuple_digest, deadline,
      state: AssignmentIssuanceCancellationAuditRepairState
    }
  | AuditConversionIntentRecorded {
      attempt_identity, conversion_id, old_reservation_generation,
      replacement_event_id, replacement_event_digest, deadline,
      action: IssueNamedOldGenerationInvalidation
    }
  | AuditConversionOldGenerationInvalidationPending {
      attempt_identity, conversion_id, old_reservation_generation,
      replacement_event_id, replacement_event_digest, deadline,
      action: ReplayNamedOldGenerationInvalidation
    }
  | AuditConversionOldGenerationInvalidatedRebindPending {
      attempt_identity, conversion_id, invalidated_reservation_generation,
      invalidation_proof_digest, replacement_event_id,
      replacement_event_digest, deadline,
      action: BindNamedReplacementRefusalGeneration
    }
  | AuditConversionRefusalGenerationBoundReplacementPending {
      attempt_identity, conversion_id, invalidated_reservation_generation,
      invalidation_proof_digest, replacement_reservation_generation,
      rebind_proof_digest, replacement_event_id, replacement_event_digest,
      deadline, action: CommitNamedReplacementRefusalTuple
    }
  | AuditConversionReplacementTupleInstalled {
      attempt_identity, conversion_id, conversion_tombstone_digest,
      replacement_reservation_generation, replacement_event_id,
      replacement_event_digest, deadline,
      action: ReplayNamedReplacementRefusalAppend
    }
  | MigrationAuditRepair {
      migration_attempt_alias, controller_epoch,
      control_reserve_incarnation, state_generation,
      state: MigrationAuditRepairState,
      action: Operator.RepairMigrationSinkAppend by ProtectedOperator
              through the fixed sink-repair child slot
    }
  | OrdinarySinkAcknowledgementPending {
      attempt_identity, appendable_reservation_generation,
      authorized_event_id, authorized_event_digest, deadline,
      action: QueryOrReplayAuthorizedAuditAppend
    }
  | OrdinaryActivationPending {
      attempt_identity, appendable_reservation_generation,
      audit_acknowledgement_digest, deadline,
      activation:
        Generic {
          action: CompleteAtomicActivation
        }
        | AssignmentIssuance {
            issuance_prepare_alias,
            prepare_incarnation,
            sink_activation_proof_digest,
            assignment_binding_digest,
            canonical_issuance_audit_tuple_digest,
            action: ActivatePreparedImplementationAssignment
          }
        | AssignmentIssuanceCancellation {
            issuance_prepare_alias,
            prepare_incarnation,
            sink_non_creatable_fence_proof_digest,
            sink_absence_or_cancellation_proof_digest,
            cancellation_reason_code,
            next_prepare_incarnation,
            prepared_cancellation_activation_binding_digest,
            cancellation_refusal_audit_tuple_digest,
            action: ActivatePreparedAssignmentIssuanceCancellation
          }
    }
  | MigrationActivationPending {
      migration_attempt_alias, controller_epoch,
      control_reserve_incarnation, state_generation,
      appendable_reservation_generation, audit_acknowledgement_digest,
      deadline,
      action: Operator.CompleteMigrationAuditActivation by ProtectedOperator
              through the fixed audit-activation child slot
    }
  | ControlReserveIntegrityCorrupt {
      migration_attempt_alias, controller_epoch, state_generation,
      underlying_migration_state_code, quarantined_reserve_incarnation,
      repair_identity, corrupt_slot_field_codes, expected_reserve_digest,
      action: Operator.RepairMigrationControlReserve by ProtectedOperator
              through the fixed integrity repair child slot
    }
  | MigrationControllerRekeyRequired {
      migration_attempt_alias, controller_epoch, state_generation,
      counter_kinds, current_counter_values, admission_thresholds,
      remaining_counter_budgets,
      action: initial Operator.RekeyMigrationControllerEpoch
              by ProtectedOperator with no rekey identity or alias
              through the separately sealed non-child rekey record
    }
  | MigrationControllerRekeyWouldCrossThreshold {
      migration_attempt_alias, controller_epoch, state_generation,
      would_cross_counter_kinds, current_counter_values,
      triggering_increment_vector, admission_thresholds,
      remaining_counter_budgets,
      action: initial Operator.RekeyMigrationControllerEpoch
              by ProtectedOperator with no rekey identity or alias
              through the separately sealed non-child rekey record
    }
  | MigrationControllerCounterExhausted {
      migration_attempt_alias, controller_epoch, state_generation,
      counter_kinds, current_counter_values, admission_thresholds,
      integrity_reason,
      action: initial Operator.RekeyMigrationControllerEpoch
              by ProtectedOperator with no rekey identity or alias
              through the separately sealed non-child rekey record
    }
  | MigrationControllerRekeyInProgress {
      migration_attempt_alias, controller_epoch, state_generation,
      rekey_identity_alias,
      state: MigrationControllerRekeySafeState,
      action: Operator.RekeyMigrationControllerEpoch::Resume
              by ProtectedOperator with only the server-resolved alias
              through the separately sealed
              non-child rekey record
    }
```

There are no optional status fields and no independent state or action
discriminants. Strict decoding denies an action, lease, pause reason, proof,
generation, or event field owned by another variant. Every deadline is
bounded by a versioned maximum. `InvalidatedReservationGeneration` cannot
convert to `AppendableReservationGeneration`; after invalidation, every
variant and action owns only the proof-bound replacement generation. It is
therefore impossible for a status action to replay an invalidated generation.
Only `PausedSelfClearingWait` polls its exact prerequisite and auto-resumes
when that prerequisite clears. `PausedOperatorRepairRequired` always carries
the exact repair prerequisite and endpoint operation from
`MigrationOperatorRepairPlanV1`; every such plan ends in resume and it cannot
render audit repair, integrity repair, rekey, or observation as its sole
remedy. At the bounded deadline, recovery may fence an expired worker epoch,
but it preserves the same repair plan and cannot pretend an operator
prerequisite cleared. `MigrationAuditRepair`,
`ControlReserveIntegrityCorrupt`, and all three pre-request counter-limit
variants are
dedicated status products with exact threshold and budget fields, no
pre-request rekey identity or alias, and only their dedicated-record action.
`MigrationControllerRekeyInProgress` is the
dedicated status product for every requested, prepared, audit-pending, and
acknowledged-install-pending rekey state. `ResumeProtectedAttempt` and
`FenceProtectedAttempt` are
authenticated audited child commands of the accepted migration, require the
narrow operator endpoint, and cannot invent a result, activate an effect,
cancel an accepted attempt, or bypass child audit.
The ordinary and migration sink-acknowledgement and activation tags are
different wire variants. The ordinary activation tag owns exactly one of
three closed nested activation variants: ordinary no-effect refusals and
non-issuance attempts use
`CompleteAtomicActivation`; a prepared assignment-issuance success carries
its prepare identity and incarnation, sink activation proof, assignment
binding, original acknowledgement, canonical audit tuple digest, and
`ActivatePreparedImplementationAssignment`; and a prepared-cancellation
refusal carries its complete
`PreparedCancellationActivationBindingDigest`, original acknowledgement, and
`ActivatePreparedAssignmentIssuanceCancellation`; a pre-proof digest cannot
parse in that slot. Those two nested
variants are the normal activation sources only. Each activator's one other
authorized source is its outcome-specific repair
`ReplacementAcknowledgedActivationPending` with the exact fields defined
above; no generic status tag can represent that source. The cancellation variant
cannot expose controller release, evidence or request restoration,
fresh-incarnation eligibility, replay availability, or old-attempt
terminalization until durable acknowledgement has constructed that exact
variant. Operation type cannot reinterpret it as generic or assignment
activation. Neither
ordinary variant can address a migration reserve. Migration
variants own only `RepairMigrationSinkAppend` and
`CompleteMigrationAuditActivation` through their fixed child slots.
Operation type is never consulted to reinterpret a generic status tag.
`AssignmentIssuancePreparedRecovery` and
both assignment-issuance repair variants likewise own their nested state and
exact next action. Prepared recovery can only resume the immutable prepared
handler or advance the fenced proof-cancellation path. Success repair can only
invalidate, rebind, install, append, acknowledge, or activate the unchanged
issuance success tuple. Cancellation repair can do the same only for the
unchanged canonical cancellation refusal, ending in specialized cancellation
activation. None can parse as generic audit conversion, emit
`accepted-attempt-crash-before-state` or `audit-event-flush-failed`, or carry
an action from another nested state.
Each `Cancellation*` journal projection is atomically joined to the
corresponding `PreparedAttemptCancellation*` prepare state and identical
prepare incarnation, reason, proof digests, next incarnation, and refusal
tuple digest. A cross-pair or mixed-generation projection fails strict
decoding and cannot run either state's action.

Crash handling is closed at every boundary:

1. before `AcceptancePrepare`, no attempt, effect, reservation, or
   authoritative event exists;
2. after controller prepare but before sink prepare, recovery completes or
   cancels the controller prepare;
3. after sink `Prepared` but before journal promotion, recovery promotes the
   valid prepare or cancels it with `NoAcceptedJournalProof`;
4. after accepted-journal promotion but before sink binding, recovery replays
   `AcceptedJournalProof`; it never cancels or leaks the accepted reservation;
5. after acceptance and sink binding but before a claim, recovery claims
   `AcceptedUnclaimed` with a new epoch;
6. while a processing or paused lease is live, recovery does nothing; after
   expiry it fences the old epoch, claims the attempt, and if no handler
   transaction exists atomically creates the one
   `accepted-attempt-crash-before-state` refusal result and outbox, except that
   accepted `MigrateRetentionCapacity` resumes its nonterminal logical
   operation and may not terminally consume the emergency lane. An accepted
   `IssueImplementationAssignment` with any durable issuance prepare resumes
   that exact prepare instead. Specifically, `PreparedForOrdinaryAudit` enters
   `AssignmentIssuancePreparedRecovery`. A valid exact
   accepted-attempt, prepare-incarnation, sink-proof, assignment-binding, and
   canonical-audit join resumes the original handler transaction. A closed
   integrity failure advances only through the permanent sink fence,
   proof-cancellation, and cancellation-refusal installation while retaining
   controller capacity, evidence, the request reservation, and
   newer-incarnation eligibility in quarantine. Only durable refusal
   acknowledgement permits the specialized final activation to release and
   restore them and terminalize the old attempt;
7. a crash during the handler transaction leaves either the prior processing
   state or the complete quarantined tuple, never a partial effect, result, or
   event;
8. after quarantine but before event authorization, recovery binds that exact
   event and generation; after authorization but before sink fsync, it resends
   the same generation, id, digest, bytes, and authorization;
9. after sink fsync but before controller acknowledgement persistence, the
   controller still sees the exact ordinary or migration
   sink-acknowledgement variant and resends the same authorized append; the
   sink returns the original acknowledgement;
10. a crash after acknowledgement persistence leaves
    `OrdinaryActivationPending` or `MigrationActivationPending`; ordinary
    recovery performs the one exact nested activation, with assignment
    issuance success permitted from normal
    `OrdinaryActivationPending::AssignmentIssuance` only through
    `ActivatePreparedImplementationAssignment` and prepared cancellation from
    normal `OrdinaryActivationPending::AssignmentIssuanceCancellation` only
    through `ActivatePreparedAssignmentIssuanceCancellation`. A dedicated
    issuance success or cancellation repair instead remains in its other
    authorized source, exact `ReplacementAcknowledgedActivationPending` with
    proof-bound acknowledgement and durable repair tombstone or preparation,
    until that same operation-specific activation commits. Recovery
    revalidates every source-specific binding and never converts one source
    into the other. In either assignment repair family, a crash before an
    accumulator fold preserves the prior root and all three current-cycle
    proof records as ineligible, while a crash after the fold preserves the
    new root and retry count and eligibility of exactly the definite-no-append,
    invalidation, and rebind-or-rollover records. A crash before final
    activation preserves retained repair-intent and any remaining current-cycle
    source bytes as ineligible; a crash after activation observes their
    eligibility only with the permanent repair tombstone durable. Migration
    activation is reachable
    only through
    the `CompleteMigrationAuditActivation` child command and its fixed
    audit-activation slot;
11. after completion but before delivery, identical retry returns the stored
   response and original acknowledgement; and
12. during or after payload eviction, the marker protocol above determines
   availability and replay returns operation-specific safe recovery without
   execution.

Startup and scheduled recovery scan every nonterminal prepare, including
every assignment issuance prepare, prepared-handler recovery, prepared
cancellation state, issuance success and cancellation audit-repair state,
intent, current-cycle proof, accumulator root and retry count, generation
rollover, replacement append, repeated definite-no-append, acknowledgement,
tombstone preparation or materialization, and activation, journal,
reservation binding, outbox row, generic conversion intent, invalidation
proof, and rebind proof, every pending assignment revocation or
revocation-capacity
release, all five reusable migration
child slots and their fixed refusal subslots including every
`PreparingAuditCapacity`, the status-disclosure slot, rekey record, telemetry
failure latch, current failure-alias record, and telemetry-recovery slot,
before the controller accepts
normal work. Each ordinary
attempt recovery
transition is an epoch-and-generation compare-and-swap and consumes the
record's recovery reservation. A live paused worker is not an orphan merely
because recovery is running. A bounded deadline guarantees worker fencing and
takeover eligibility; only `PausedSelfClearingWait` can then resume
automatically, while `PausedOperatorRepairRequired` still requires its exact
prerequisite and authenticated resume. Child slots use their separately
specified owner-epoch,
slot-generation, lease, deadline, external-effect, and audit recovery
protocol.

Timeout, disconnect, or lost acknowledgement is never proof that the sink did
not append. The controller may convert an authorized pending event only after an
authenticated definite-no-append result for its stable `AuditEventId`, digest,
and reservation generation. The result is accepted only while the journal is
in the exact matching ordinary or migration sink-acknowledgement state. For a generic
remaining tuple, conversion is itself recoverable:

1. one controller transaction compares the exact old
   `OrdinarySinkAcknowledgementPending` tuple, durably records
   `AuditConversionIntent` binding that old generation and the one replacement
   refusal event and digest, and installs the
   `PendingAuditConversion::IntentRecorded` blocker. A mismatch changes
   nothing, and no transition exists from `QuarantinedPendingAudit`;
2. before sending the invalidation request, the controller durably enters
   `OldGenerationInvalidationPending`. The sink then invalidates the exact old
   generation and returns an unforgeable `AuditSinkInvalidationProof`. From
   that sink commit every delayed old-generation append is
   `audit-sink-generation-stale`. Persisting the proof enters
   `OldGenerationInvalidatedRebindPending`;
3. only that proof can authorize a sink rebind to the next monotonic
   generation and exactly the refusal event and digest. The sink returns an
   unforgeable `AuditSinkRebindProof`; persisting it enters
   `RefusalGenerationBoundReplacementPending`;
4. after the sink durably rebinds, one replacement-activation controller
   transaction replaces the
   quarantined result, effect, assignment-use reservation, replay bytes, and
   outbox with `audit-event-flush-failed`, no effect, no use, and the rebound
   refusal event. The same transaction creates the immutable
   `AuditConversionTombstone` from the exact intent, invalidation-proof, and
   rebind-proof digests; makes the protected intent and proof bytes eligible
   round input; and enters `ReplacementTupleInstalled`; and
5. normal authorized append and activation continue only on the replacement
   generation. Sending that append enters the ordinary
   `OrdinarySinkAcknowledgementPending` variant with the replacement
   generation.

A crash at any conversion boundary resumes from the exact tagged status and
the same `PendingAuditConversion` blocker. A crash before intent leaves the
old `OrdinarySinkAcknowledgementPending` tuple, not
`QuarantinedPendingAudit`. A crash
after intent, invalidation request,
invalidation proof, rebind proof, or replacement activation respectively
replays only that state's exact action. No action after the invalidation proof
can carry the old generation. Unknown old append state remains pending for
idempotent replay and cannot enter conversion. A delayed fenced worker that
submits the old success after invalidation is rejected by the sink; the
controller also rejects its stale epoch and fails closed without recording
that success, acknowledgement, replay result, or effect.
`audit-event-id-conflict` is a fail-closed integrity fault and never activates
the quarantined effect. Audit is evidence; protected controller state remains
authority.

For an issuance success, the same authenticated definite-no-append proof
enters `AssignmentIssuanceAuditRepair` instead of generic conversion:

1. one controller transaction compares the exact
   `OrdinarySinkAcknowledgementPending` source tuple, operation type,
   accepted `AttemptIdentity`, prepare identity and incarnation, sink
   activation proof, assignment binding, and canonical issuance event id,
   digest, and bytes digest. It records an
   `AssignmentIssuanceAuditRepairState::IntentRecorded` and the fixed
   `AssignmentIssuanceRepairWorkspace` binding the initial and current
   generation, authenticated definite-no-append proof, initial accumulator
   root, zero saturating retry count, and unchanged success tuple. The
   prepared assignment effect, replay result, outbox, evidence reservation,
   request reservation, and both revocation-capacity reservations remain
   unchanged and nonterminal;
2. `IssueAssignmentIssuanceOldGenerationInvalidation` durably enters
   `OldGenerationInvalidationPending`; the sink invalidates exactly the old
   ordinary-audit generation, and
   `ReplayAssignmentIssuanceOldGenerationInvalidation` persists the proof.
   The same state and action are reused after any later replacement generation
   receives authenticated definite-no-append;
3. only that exact definite-no-append and invalidation proof pair permits
   `BindAssignmentIssuanceSuccessReplacementGeneration` to bind the next
   generation to the same issuance event id, digest, and canonical bytes and
   persist the rebind proof. If the finite generation would wrap, proof
   persistence instead enters `ReplacementGenerationRolloverPending`, and
   `RolloverAssignmentIssuanceSuccessReplacementGeneration` creates the next
   sink epoch and binds its first generation to the same event using the same
   workspace and reserved capacity;
4. `CommitAssignmentIssuanceSuccessReplacementTuple` installs the
   proof-bound replacement tuple and atomically folds the definite-no-append,
   invalidation, and rebind-or-rollover proof digests into the accumulator,
   advances the current generation, and increments the saturating retry count.
   It does not change the prepared effect, result, evidence, request, capacity,
   or authority eligibility. That commit is the cycle's durable compaction
   boundary: exactly its raw definite-no-append, invalidation, and
   rebind-or-rollover proofs become ordinary eligible round input and all
   three fixed proof slots become reusable. Before commit the prior root and
   count, ineligibility of all three proof records, and occupancy of all three
   slots remain authoritative; after commit the new root and count,
   eligibility of exactly those three proof records, and reuse of all three
   slots remain authoritative;
5. `ReplayAssignmentIssuanceSuccessAppend` enters
   `ReplacementSinkAcknowledgementPending` on the replacement generation.
   `QueryOrReplayAssignmentIssuanceSuccessAppend` can only recover the same
   append. Unknown outcome remains in that state. Authenticated
   definite-no-append atomically binds the proof to the current generation and
   re-enters `OldGenerationInvalidationPending` in the same workspace; it
   cannot allocate another repair or convert the result. A proof-bound
   acknowledgement advances to the final step; and
6. acknowledgement persistence constructs the exact tombstone-preparation
   tuple and enters `ReplacementAcknowledgedActivationPending` with the
   unchanged event, initial and final generations, accumulator root, retry
   count, final acknowledgement, and either that durable preparation or the
   already materialized matching tombstone. From this repair source, the exact
   `ActivatePreparedImplementationAssignment` binding checks and atomic effects
   are those stated in the issuance decision above. Its committed post-state
   makes retained repair-intent and any remaining current-cycle source bytes
   ordinary eligible round input only after the permanent repair tombstone is
   durable. No ordinary
   `OrdinaryActivationPending::AssignmentIssuance` record can be substituted.

A crash before repair intent leaves the exact old issuance-success
`OrdinarySinkAcknowledgementPending` source. Crashes after intent, invalidation request,
invalidation proof, ordinary rebind or rollover proof, accumulator fold,
replacement installation, any replacement sink fsync, repeated
definite-no-append, acknowledgement persistence, tombstone preparation or
materialization, or activation resume only the exact nested action with the
same workspace and capacity. Startup never restarts at the first generation
when a later current generation or accumulator root is durable. A delayed
invalidated-generation append is stale. A tuple mismatch
returns only
`implementation-assignment-issuance-audit-repair-tuple-mismatch`; a request
from any other source state returns only
`implementation-assignment-issuance-audit-repair-invalid-state`. A retryable
storage, invalidation, rebind, rollover, append, acknowledgement, accumulator,
tombstone, or activation failure returns only
`implementation-assignment-issuance-audit-repair-retryable` with the current
safe repair state, accumulator root, retry count, and exact next action. None
of those conditions changes the prepared success or enters generic
conversion. A tombstone construction or final activation failure remains
`ReplacementAcknowledgedActivationPending`; prior folded-cycle proofs retain
ordinary eligibility, while all three proofs of any unfolded current cycle,
the retained repair-intent, and any remaining current-cycle source bytes
remain ineligible. Specifically, a crash before a fold preserves the prior
root and all three current-cycle proof records as ineligible, while a crash
after the fold preserves the new root and retry count and eligibility of
exactly those three records. A crash before final activation leaves retained
repair-intent and remaining source bytes ineligible; a crash after activation
observes both the durable permanent repair tombstone and their eligibility.
Generic `audit-event-flush-failed` is unconstructible for an
issuance success.

For the canonical prepared-cancellation refusal, the authenticated
definite-no-append proof enters
`AssignmentIssuanceCancellationAuditRepair` instead of generic conversion:

1. one controller transaction compares the exact
   `OrdinarySinkAcknowledgementPending` source tuple, accepted
   `AttemptIdentity`, prepare identity and incarnation, permanent sink-fence
   and proof-cancellation digests, cancellation reason, reserved next
   incarnation, and canonical cancellation-refusal event id, digest, and
   bytes digest. It records
   `AssignmentIssuanceCancellationAuditRepairState::IntentRecorded` binding
   the fixed workspace, initial and current generation, authenticated
   definite-no-append proof, initial accumulator root, zero saturating retry
   count, and unchanged refusal. The controller reservation, evidence, request
   reservation, newer-incarnation eligibility, replay result, and old attempt
   remain quarantined and nonterminal;
2. `IssueAssignmentIssuanceCancellationOldGenerationInvalidation` durably
   enters `OldGenerationInvalidationPending`; the sink invalidates exactly the
   old ordinary-audit generation, and
   `ReplayAssignmentIssuanceCancellationOldGenerationInvalidation` persists
   the proof. Any later replacement-generation definite-no-append result
   returns to this same fixed cycle;
3. only the exact definite-no-append and invalidation proof pair permits
   `BindAssignmentIssuanceCancellationRefusalReplacementGeneration` to bind
   the next generation to the unchanged canonical cancellation-refusal event
   id, digest, and bytes and persist the rebind proof. At the no-wrap boundary,
   `ReplacementGenerationRolloverPending` owns
   `RolloverAssignmentIssuanceCancellationRefusalReplacementGeneration`,
   which creates the next sink epoch and proof-binds its first generation to
   the same refusal using no new workspace or capacity;
4. `CommitAssignmentIssuanceCancellationRefusalReplacementTuple` installs
   only the proof-bound replacement tuple while folding the current proof
   triple into the accumulator, advancing current generation, and incrementing
   the saturating retry count atomically. It does not release, restore, expose
   authority eligibility, or terminalize. The same atomic commit is the
   durable compaction boundary: exactly that cycle's raw definite-no-append,
   invalidation, and rebind-or-rollover proofs become ordinary eligible round
   input and all three fixed proof slots become reusable. A crash before it
   preserves the prior root and all three proof records as ineligible with all
   three slots occupied; a crash after it preserves the new root and retry
   count, eligibility of exactly those three proof records, and reuse of all
   three slots;
5. `ReplayAssignmentIssuanceCancellationRefusalAppend` enters
   `ReplacementSinkAcknowledgementPending`.
   `QueryOrReplayAssignmentIssuanceCancellationRefusalAppend` can recover only
   that unchanged append. Unknown outcome remains pending; authenticated
   definite-no-append binds the current generation and re-enters the same
   invalidation loop; a proof-bound acknowledgement advances to the final
   step; and
6. acknowledgement persistence constructs the exact cancellation-repair
   tombstone preparation and enters
   `ReplacementAcknowledgedActivationPending` with the unchanged refusal,
   initial and final generations, accumulator root, retry count, final
   acknowledgement, and exact durable preparation or already materialized
   tombstone. From this repair source, the exact
   `ActivatePreparedAssignmentIssuanceCancellation` checks and atomic effects
   are those stated in the prepared-cancellation decision above. That repair
   activation is complete only when the permanent cancellation-repair
   tombstone is durable; the committed final state then makes retained
   repair-intent and any remaining current-cycle source bytes ordinary
   eligible round input. No
   `OrdinaryActivationPending::AssignmentIssuanceCancellation` record can be
   substituted.

A crash before cancellation-repair intent leaves the exact old refusal
`OrdinarySinkAcknowledgementPending` source. A crash after any intent, invalidation, rebind,
rollover, accumulator fold, replacement, repeated definite-no-append, append,
acknowledgement, tombstone preparation or materialization, or activation
boundary resumes only the exact nested state and action with the same
workspace and capacity. A delayed invalidated-generation append is stale. A
tuple mismatch, invalid source state, or retryable invalidation, rebind,
rollover, accumulator, append, acknowledgement, tombstone, storage, or
activation failure returns only its dedicated
`implementation-assignment-issuance-cancellation-audit-repair-*` refusal and
current safe state, accumulator root, retry count, and exact next action. No
such condition releases capacity or evidence, restores the request, exposes
the newer incarnation, terminalizes the old attempt, changes the canonical
cancellation refusal, allocates another repair workspace, or enters generic
conversion. A crash before a fold preserves the prior root and all three
current-cycle proof records as ineligible, while a crash after the fold
preserves the new root and retry count and eligibility of exactly those three
records. A crash before final activation leaves retained repair-intent and
remaining source bytes ineligible; a crash after activation observes both the
durable permanent cancellation-repair tombstone and their eligibility.
Generic `audit-event-flush-failed` is unconstructible for this
refusal.

For `MigrateRetentionCapacity`, the same definite-no-append proof enters
`PendingMigrationAuditRepair` instead:

1. the source-state compare-and-swap records `MigrationAuditRepairIntent`
   over the old tuple and the unchanged success event, while retaining the
   quarantined capacity-switch effect, success and replay results, outbox,
   execution reserve, control reserve, and transient lane;
2. `RepairMigrationSinkAppend` invalidates the exact old generation and
   persists its proof;
3. only that proof rebinds the next generation to the same success event id,
   digest, and canonical bytes, and persists the rebind proof;
4. one transaction installs the replacement tuple and immutable
   `MigrationAuditRepairTombstone` without replacing or terminalizing the
   success result; and
5. `RepairMigrationSinkAppend` retries that success append. After
   acknowledgement persistence, `CompleteMigrationAuditActivation` activates
   the same capacity-switch effect and completes the same attempt.

Every step is idempotent and consumes only the migration's sealed control
reserve. A crash before the repair intent remains at the old
`MigrationSinkAcknowledgementPending` tuple. Crashes after intent, invalidation request,
invalidation proof, rebind proof, replacement installation, replacement sink
fsync, acknowledgement persistence, or activation resume only the exact
tagged step. A delayed old-generation append is stale, a wrong success event
is an event mismatch, and neither can replace the retained canonical success
tuple. Generic refusal conversion rejects a migration attempt by type before
writing an intent.

Every terminal lifecycle writes exactly one typed
`TerminalLifecycleMetricRecord`. Outcome, completeness, degraded reason,
discovery progress, and discovery metrics are not independent wire fields.
They are one top-level tagged enum:

```
TerminalLifecycleMetricRecord =
  SignedOff {
    final_candidate_id,
    lineage_digest,
    scope_digest,
    discovery: CompleteAdmittedDiscovery
  }
  | Abandoned {
      final_candidate_id,
      lineage_digest,
      scope_digest,
      progress: ClosedProgressSnapshot,
      degraded_reason: None | ClosedDegradedReason
    }
  | Superseded {
      final_candidate_id,
      lineage_digest,
      scope_digest,
      progress: ClosedProgressSnapshot,
      degraded_reason: None | ClosedDegradedReason
    }

AdmittedDiscoveryMetrics {
  final_ledger_digest,
  final_mapping_digest,
  late_and_severity_counts,
  review_and_implementation_iteration_counts,
  disposition_and_adjudication_counts,
  split_merge_and_alias_counts
}

PartialLegacyProgress {
  source_lifecycle_id,
  dispatch_id,
  completed_seat_count,
  imported_legacy_source_count,
  missing_source_triage_count,
  present_unverified_or_stale_source_triage_count,
  verified_legacy_source_triage_count,
  partial_round_retry_count,
  migration_retry_count
}

AdmittedDiscoveryInput =
  NativeDiscovery {
    native_source_count,
    imported_partial_legacy: None | PartialLegacyProgress
  }
  | CompleteLegacyDiscoveryImport {
      source_lifecycle_id,
      completed_seat_count,
      legacy_source_count,
      missing_source_triage_count,
      present_unverified_or_stale_source_triage_count,
      verified_legacy_source_triage_count,
      migration_retry_count
    }

CompleteAdmittedDiscovery =
  NativeDiscovery {
    input: AdmittedDiscoveryInput::NativeDiscovery,
    admitted: AdmittedDiscoveryMetrics,
    native_initial_effective_issue_count,
    imported_partial_effective: None | {
      imported_effective_issue_count,
      prior_obligation_effective_issue_count
    }
  }
  | CompleteLegacyDiscoveryImport {
      input: AdmittedDiscoveryInput::CompleteLegacyDiscoveryImport,
      admitted: AdmittedDiscoveryMetrics,
      imported_effective_issue_count
    }

ClosedProgressSnapshot =
  BeforeDiscovery {
    partial_legacy_source: None | {
        dispatch_id,
        completed_seat_count,
        completed_recommendation_count,
        retry_count
      }
  }
  | PartialLegacyObligationsImported {
      imported: PartialLegacyProgress
    }
  | DiscoveryAdmittedLedgerPending {
      discovery: AdmittedDiscoveryInput
    }
  | LedgerAdmitted {
      discovery: CompleteAdmittedDiscovery
    }
```

For every `PartialLegacyProgress`, the missing,
present-unverified-or-stale, and verified triage counts are disjoint and sum
exactly to `imported_legacy_source_count`. The source ids remain protected,
but the counts are exact projections of the imported `LegacySourceId` set.
Every successor terminal point therefore records the same exact imported
source count and its current triage partition: immediately after import,
after fresh discovery but before ledger synthesis, and after ledger
admission.
The complete-legacy triage counts obey the same partition over
`legacy_source_count`. `imported_partial_effective` exists if and only if the
native input carries `imported_partial_legacy`; effective issue counts do not
appear before ledger admission. Native and complete-legacy effective issue
counts likewise exist only in `CompleteAdmittedDiscovery`.

There are no independent outcome, completeness, degraded-reason,
discovery-origin, migration-origin, legacy-source, imported-issue,
partial-successor, completed-seat, or issue-level retriage fields. The
top-level and nested enum payloads own every count that can exist only for
their variants, so a native lifecycle cannot claim complete legacy discovery,
a pre-discovery lifecycle cannot claim a ledger, and a signed-off record
cannot carry a degraded reason or no-discovery progress.
Generated code uses private constructors over this tagged enum, and strict
deserialization denies unknown, missing, or cross-variant fields. Contradictory
combinations therefore fail construction or parsing rather than reaching
metric emission.

`SignedOff` can own only `CompleteAdmittedDiscovery`; completeness is implied
and no degraded field exists in that variant. `Abandoned` and `Superseded`
always own one closed progress snapshot and may own one closed degraded
reason. A source partial lifecycle superseded into a same-scope successor
records `BeforeDiscovery` with `partial_legacy_source`; it never counts its
partial round as discovery. The successor begins at
`PartialLegacyObligationsImported`, advances to
`DiscoveryAdmittedLedgerPending` after its one fresh native discovery, and
advances to `LedgerAdmitted` only after synthesis admission. A lifecycle
terminated before any discovery records `BeforeDiscovery`. No abandoned or
superseded projection can be presented as approval.

For a signed-off complete legacy import,
`verified_legacy_source_triage_count == legacy_source_count`. For a signed-off
native successor with imported partial obligations, that verified count equals
`imported_legacy_source_count`. Earlier terminal outcomes record the exact
three-way triage partition without converting it to an issue count.

Metric counting is fixed:

- `initial_findings` is a derived projection, not an independent serialized
  field. It exists only for `SignedOff` or `LedgerAdmitted` and is the number
  of terminal effective issue classes whose earliest source is in the native
  or complete imported discovery input:
  `native_initial_effective_issue_count` for `NativeDiscovery`, or
  `imported_effective_issue_count` for `CompleteLegacyDiscoveryImport`;
- `prior_obligation_findings` is likewise derived from
  `imported_partial_effective.prior_obligation_effective_issue_count` after
  ledger admission. It counts classes whose earliest source came from a
  completed seat of a partial legacy round; those sources are not counted as
  discovery or late findings;
- `late_findings` is the number of terminal effective issue classes whose
  earliest source was admitted after discovery;
- `late_blocker_count` and `late_major_count` use those late classes and their
  terminal effective severities;
- native and migration-assigned severities are counted in separate fields, so
  no chart implies a legacy string carried historical severity;
- `verified_legacy_source_triage_count` is the number of distinct exact
  `LegacySourceId` values with an admitted triage and independent verification
  at the terminal candidate and mapping. It is never an issue count, and there
  is no `re-triaged issue` metric;
- partial-round and migration retry counts include only distinct accepted
  retry attempt identities that reached their named stage. Identical request
  replay, response loss, preflight refusal, and idempotent regeneration do not
  increment them;
- `review_iterations` counts the one native discovery execution or one
  imported complete legacy round, plus each admitted verification execution;
- a partial legacy successor counts its one fresh native discovery and never
  counts the partial old round as an execution;
- partial rounds, missing-seat retries, preflight failures, and idempotent
  regeneration do not increment review iterations;
- `implementation_iterations` counts each post-discovery batch that produces a
  candidate delta and enters self-verification;
- average issues fixed divides effective issues first reaching terminal
  verified `Fixed` after the latest mapping correction by implementation
  iterations, and is `0.0` when the denominator is zero; and
- every unique issue is counted once at the terminal effective mapping,
  regardless of source count or aliases.

The approval receipt remains sign-off-only. Terminal metric records for
abandonment and supersession are not approval receipts and cannot be presented
to seal, publication, or merge eligibility.

### 14. Refusals have typed causes and deterministic recovery

Every refusal introduced by this record is a closed error variant carrying the
safe causing identifiers: applicable lifecycle, candidate, issue, source,
seat, acceptance, ledger version, or validation job ids. It never carries the
protected text those ids address.

Assignment refusal evaluation is ordered and disjoint. Missing authoritative
evidence or a caller-built claim is self-assertion; a real handle presented by
an authenticated peer or implementer run other than the bound one is replay;
authoritative `RevocationPending` selects its exact pending refusal before any
use, issue-view, completion, expiry, or duplicate-revocation predicate; the
one stored terminal state selects exactly one of completed, revoked, expired,
or exhausted; a remaining lifecycle, candidate, or mapping mismatch is a
binding mismatch; and only an otherwise current active assignment can reach
cross-scope when the caller-supplied issue ids are not a subset of its exact
set. Cross-scope evaluation does not resolve which other assignment, if any,
owns an issue.

Accepted-attempt abort selection is also ordered and disjoint. Recovery first
checks the exact journal generation and whether a handler tuple exists. With
no handler tuple, `IssueImplementationAssignment` joined to any durable
issuance prepare resumes that prepare before the generic crash predicate.
`PreparedForOrdinaryAudit` specifically selects
`AssignmentIssuancePreparedRecovery`. Its valid complete tuple resumes the
prepared handler; its closed integrity failure selects the single fenced
prepared-cancellation path. An issuance attempt with no durable prepare may
still select the generic no-state refusal because it has no issuance
reservation, evidence reservation, or prepared effect to release.
`MigrateRetentionCapacity` next selects its existing nonterminal resume path.
Only every remaining accepted operation can select
`accepted-attempt-crash-before-state`.

Definite-no-append selection first requires the exact authorized
sink-acknowledgement source tuple. It then dispatches by the already stored
operation and outcome type: a prepared `IssueImplementationAssignment`
success selects `AssignmentIssuanceAuditRepair`; the canonical refusal bound
to `AssignmentIssuancePreparedRecovery` selects
`AssignmentIssuanceCancellationAuditRepair`;
`MigrateRetentionCapacity` selects `PendingMigrationAuditRepair`; and only a
remaining ordinary tuple, including an issuance refusal with no prepared
effect, selects generic refusal conversion. Prepared-cancellation identity is
tested before the generic no-effect-refusal predicate. A tuple mismatch or
invalid source state is returned before any branch writes an intent. An
issuance success or cancellation repair failure retains its dedicated branch
and unchanged event; neither can fall through to
`audit-event-flush-failed`. These precedence joins are generated, not an
ordered string comparison in a handler.

Completion refusal precedence is also fixed. After endpoint authentication and
assignment resolution, `RevocationPending` and terminal assignment states
return their exact state refusal before completion-evidence evaluation. For an
active assignment, after protected evidence authentication, the controller
first checks the exact originating principal
and originating issuance evidence and selects the corresponding
`AssignmentCompletionOriginCode`. It then checks the
single-consumption index. For a settled internal evidence identity, a different
full `AssignmentCompletionBindingDigest` is conflict with
`AssignmentBindingDigest`; if that digest matches but the immutable evidence
digest differs, conflict is `ImmutableEvidenceDigest`; only equality of the
identity and both full digests is replay. A multi-fault settled reuse therefore
cannot be classified as an ordinary binding mismatch, and replay can never
mask a changed binding. Unsettled fresh evidence is compared one field at a
time in this order:
`AssignmentId`, `Lifecycle`, `Candidate`, `MappingVersion`, `FinalIssueSet`,
`ImplementerRun`, `CompletionResult`, `IssuedAt`, `ExpiresAt`, and
`EvidenceIdentity`. The first mismatch selects exactly that
`AssignmentCompletionBindingFieldCode`. Only fully bound evidence reaches the
authoritative freshness check, which selects the separate stale-or-expired
reason. Only fresh evidence can complete. The generated one-field mutation
matrix uses fresh unconsumed evidence, so an exact field mismatch cannot be
masked by replay or conflict. Revocation first requires the protected operator
endpoint; an originating issuer or resolver never passes that check merely
because it issued the assignment.

At sink prepare, a missing reservation is created, the same
`AttemptIdentity` and prepare digest is idempotent, and the same identity with
a different canonical prepare digest is only
`acceptance-prepare-digest-conflict`. It cannot fall through to protected
request replay, orphan-proof, or append-authorization errors. Append
authorization precedence is the six-step order in section 13: invalid,
cross-attempt, any non-current generation as stale, current generation with no
authorized event as unbound, current authorized event with wrong id or digest
as event mismatch, and append. Those rows are disjoint, a future generation
cannot be unbound, and no generic sink row overlaps them. Each refusal emits
its exact `AuditAppendAuthorizationRefusalProduct` variant and nothing else.

Legacy triage is likewise partitioned: any source without a submitted triage
selects `legacy-source-triage-missing`; only when that set is empty can a
source with a present but absent or stale independent verification select
`legacy-source-triage-unverified-or-stale`. Generic native-source severity
authorization and legacy-source severity authorization remain disjoint by
identifier type before caller authorization is evaluated; their independent
verification refusals use the same identifier-type partition.

Ledger-correction refusal order is also fixed. A base-ledger version mismatch
is `ledger-correction-stale`; against the current base, an invalid source
partition, alias, monotonic id, or coverage shape is
`ledger-correction-structurally-invalid`; a structurally valid merge with
incompatible dispositions is
`ledger-correction-dispositions-incompatible`; and only then is each required
concurrence classified as explicit dissent, stale, or missing. An explicit
dissent wins over stale or missing concurrence, stale wins over missing, and
the causing source and seat sets are disjoint projections. The partial-round
states are similarly linear: all missing reviewers proven dispatchable is
`legacy-round-partial-retryable`; any reviewer proven unavailable is
`legacy-round-reviewer-unavailable`; after that proof, an import failure is
only `successor-import-incomplete` and cannot revert to either earlier
predicate.

Recovery is generated by a total function:

```
remedies(error, producer_context) -> RemedyPlan
```

`producer_context` is closed as `GasCity { stage }` or
`Standalone { operation }`. `RemedyPlan` is an ordered sequence of closed
`RemedyAction` values. Callers cannot populate free-form advice. Gas City
actions name the deterministic stage retry or protected controller operation;
standalone actions name the corresponding standalone operation. Exact CLI
spelling may remain implementation-defined, but it must be generated from the
typed action, tested, and actionable.

The migration, telemetry, assignment-issuance, and assignment-revocation
additions use only these new closed `RemedyAction` values:

```
RemedyAction =
  ...
  | ReadCurrentMigrationRecoveryStatus
  | InvokeReturnedMigrationOperation
  | ReturnAuditedMigrationChildRefusal
  | AttemptMigrationControlConflictSignal { StaleGeneration | ChangedBytes }
  | CorrectMigrationControlTuple
  | VerifyMigrationPausePrerequisite
  | SubmitStateCurrentMigrationChildCommand
  | RestoreMigrationChildAuditCapacity { ControllerOutbox | Sink }
  | ReconcileMigrationChildAuditPrepare
  | WaitForMigrationRefusalSubslotSettlement
  | StopStaleMigrationChildWorker
  | RetryMigrationControlReserveRepairSameIdentity
  | StartMigrationControllerRekeyIdentityFree
  | ResumeMigrationControllerRekeyWithCurrentAlias
  | WaitForMigrationRekeyQuiescence
  | RetryMigrationControllerRekeySameIdentity
  | WaitForMigrationStatusAuditSlot
  | DiscardStaleMigrationTelemetryClosure
  | ReadCurrentMigrationTelemetryHealth
  | FollowCurrentMigrationTelemetryHealthRemedy
  | RecoverMigrationTelemetryHealth
  | ReadAssignmentRevocationRecovery
  | InvokeReturnedAssignmentRecoveryAction
  | RestoreAssignmentRevocationAuditCapacity
  | ResumeAssignmentRevocationAudit
  | WaitForAssignmentRevocationSettlement
  | FinalizeAssignmentRevoked
  | ProofCancelUnusedAssignmentRevocationSinkCapacity
  | ReleaseUnusedAssignmentRevocationControllerCapacity
  | InstallAssignmentTerminalIntent
  | RestoreAssignmentIssuanceCapacity { Controller | Sink }
  | RetrySamePendingAssignmentIssuance
  | ReadCurrentAssignmentIssuanceRecovery
  | InvokeReturnedAssignmentIssuanceRecoveryAction
  | CreateOrAdoptAssignmentIssuanceSinkReservation
  | ActivateAssignmentIssuanceSinkReservation
  | FenceAssignmentIssuanceSinkIncarnation
  | ActivatePreparedImplementationAssignment
  | ProofCancelAssignmentIssuanceSinkReservation
  | ReleaseAssignmentIssuanceControllerReservation
  | ResumePreparedAssignmentIssuanceHandler
  | FencePreparedAssignmentIssuanceForCancellation
  | ProofCancelPreparedAssignmentIssuanceSinkReservation
  | InstallPreparedAssignmentIssuanceCancellationRefusal
  | ActivatePreparedAssignmentIssuanceCancellation
  | IssueAssignmentIssuanceOldGenerationInvalidation
  | ReplayAssignmentIssuanceOldGenerationInvalidation
  | BindAssignmentIssuanceSuccessReplacementGeneration
  | RolloverAssignmentIssuanceSuccessReplacementGeneration
  | CommitAssignmentIssuanceSuccessReplacementTuple
  | ReplayAssignmentIssuanceSuccessAppend
  | QueryOrReplayAssignmentIssuanceSuccessAppend
  | IssueAssignmentIssuanceCancellationOldGenerationInvalidation
  | ReplayAssignmentIssuanceCancellationOldGenerationInvalidation
  | BindAssignmentIssuanceCancellationRefusalReplacementGeneration
  | RolloverAssignmentIssuanceCancellationRefusalReplacementGeneration
  | CommitAssignmentIssuanceCancellationRefusalReplacementTuple
  | ReplayAssignmentIssuanceCancellationRefusalAppend
  | QueryOrReplayAssignmentIssuanceCancellationRefusalAppend
```

Their generated protected command mapping is exact:

| `RemedyAction` | Generated command |
| --- | --- |
| `ReadCurrentMigrationRecoveryStatus` | `Operator.ReadMigrationRecoveryStatus` with the causing migration alias |
| `InvokeReturnedMigrationOperation` | the one endpoint operation, caller class, reserve route, and identity owned by the returned status tag |
| `SubmitStateCurrentMigrationChildCommand` | the one child operation and current epoch and state generation owned by the causing product |
| `WaitForMigrationRefusalSubslotSettlement` | wait only for the exact refusal-subslot generation in the causing occupied product; it cannot read status or repair controller outbox or sink capacity |
| `StartMigrationControllerRekeyIdentityFree` | initial `Operator.RekeyMigrationControllerEpoch` from an exact threshold-reached, would-cross-threshold, or exhausted pre-request product, with no rekey identity or alias |
| `ResumeMigrationControllerRekeyWithCurrentAlias` or `RetryMigrationControllerRekeySameIdentity` | `Operator.RekeyMigrationControllerEpoch::Resume` only from an active rekey product carrying the current server-resolved non-capability rekey alias |
| `ReadCurrentMigrationTelemetryHealth` | the health projection nested in `Operator.ReadMigrationRecoveryStatus` |
| `RecoverMigrationTelemetryHealth` | `Operator.RecoverMigrationTelemetryHealth` with the causing server-issued `TelemetryFailureAlias`, whether it is current in a non-healthy observation or bound in a retryable recovery product |
| `ReadAssignmentRevocationRecovery` | `RecoveryRead.ReadAssignmentRecoveryState { context: RevocationRecovery }` with the causing assignment alias |
| `ResumeAssignmentRevocationAudit` | the exact append, query, invalidation, rebind, or acknowledgement action owned only by `RevocationAuditWorkPending` |
| `WaitForAssignmentRevocationSettlement` | wait and reread owned only by `RevocationAuditAcknowledgedUsesReserved` |
| `FinalizeAssignmentRevoked` | atomic finalization owned only by `RevocationReadyToFinalize` |
| `RestoreAssignmentIssuanceCapacity { Controller | Sink }` | restore only the named dedicated capacity owner from the exact `AssignmentIssuanceCapacityUnavailable` variant; it cannot create a prepare or assignment |
| `RetrySamePendingAssignmentIssuance` | `AssignmentIssuance.IssueImplementationAssignment` for the same `RequestPending` request after controller-capacity restoration, or the same `IssuancePending` prepare after sink-capacity restoration; after proof-backed restoration it allocates the next prepare incarnation, never the fenced incarnation, and no fresh request or assignment identity is allowed |
| `ReadCurrentAssignmentIssuanceRecovery` | `RecoveryRead.ReadAssignmentRecoveryState { context: FreshAssignmentFlow }` for the exact initial partition or terminal predecessor bound to the causing assignment request |
| `InvokeReturnedAssignmentIssuanceRecoveryAction` | invoke only the action nested in the returned `AssignmentIssuancePreparedRecoveryState`, `AssignmentIssuanceAuditRepairState`, `AssignmentIssuanceCancellationAuditRepairState`, or specialized assignment-issuance `OrdinaryActivationPending` variant; no caller may supply a state or action discriminator |
| `CreateOrAdoptAssignmentIssuanceSinkReservation` | create or adopt only the sink reservation bound to `SinkReservationCreateOrAdoptPending`, its exact ordinary accepted attempt, prepare identity and incarnation, assignment binding, and canonical issuance audit tuple |
| `ActivateAssignmentIssuanceSinkReservation` | activate only the adopted sink reservation owned by `SinkReservationActivationPending` with that same incarnation and binding |
| `FenceAssignmentIssuanceSinkIncarnation` | durably install the permanent non-creatable sink fence for the exact non-adoptable prepare identity and incarnation before any proof-cancellation action is constructible |
| `ActivatePreparedImplementationAssignment` | from exactly one of two disjoint sources: normal `OrdinaryActivationPending::AssignmentIssuance` with the original acknowledgement, or issuance-repair `ReplacementAcknowledgedActivationPending` with its proof-bound final acknowledgement and exact durable `AssignmentIssuanceAuditRepairTombstone` or tombstone preparation. Both bind the accepted attempt, prepare identity and incarnation, prepared activation binding covering sink proof, assignment, evidence, request, eligibility, and capacity, and canonical event id, digest, and bytes; normal additionally binds the original generation and acknowledgement with no repair fields, while repair additionally binds repair identity, initial and final generations, accumulator root, saturating retry count, unchanged event, final acknowledgement, and repair-floor digest with no ordinary variant. One atomic controller transaction installs `Active`, exact `Issued`, settled evidence, replay result, attempt tombstone, and completed attempt; on repair that same transaction verifies or materializes the exact permanent repair tombstone binding the latest committed accumulator root and retry count, then makes retained repair-intent and any remaining current-cycle source bytes ordinary eligible round input only with that tombstone durable. |
| `ProofCancelAssignmentIssuanceSinkReservation` | obtain sink absence or cancellation proof only from `SinkReservationProofCancellationPending` with the exact incarnation's durable non-creatable fence proof; time alone cannot satisfy it |
| `ReleaseAssignmentIssuanceControllerReservation` | release controller capacity and restore the same accepted attempt's request and evidence eligibility only from `ControllerReservationReleasePending` with both sink proofs; its retry must allocate a newer incarnation |
| `ResumePreparedAssignmentIssuanceHandler` | from only `ResumePreparedHandlerPending`, replay the original ordinary handler transaction with the exact accepted attempt, prepare incarnation, sink activation proof, assignment binding, and canonical issuance audit tuple |
| `FencePreparedAssignmentIssuanceForCancellation` | from only `CancellationSinkFencePending`, verify its `PreparedCancellationPreProofBindingDigest` and persist the permanent non-creatable fence for the exact prepared incarnation and recorded cancellation reason; no complete activation digest is accepted or constructed |
| `ProofCancelPreparedAssignmentIssuanceSinkReservation` | from only `CancellationSinkProofPending`, verify the same pre-proof binding plus the durable fence proof and obtain absence or cancellation proof for the exact fenced activated sink reservation |
| `InstallPreparedAssignmentIssuanceCancellationRefusal` | from only `CancellationRefusalInstallPending`, whose transition has derived the complete `PreparedCancellationActivationBindingDigest` from the pre-proof binding and both durable proofs, install the unchanged no-effect cancellation refusal and outbox while retaining controller capacity, evidence, the request reservation, newer-incarnation eligibility, and the old attempt in quarantine |
| `ActivatePreparedAssignmentIssuanceCancellation` | from exactly one of two disjoint sources: normal `OrdinaryActivationPending::AssignmentIssuanceCancellation` with the original refusal acknowledgement, or cancellation-repair `ReplacementAcknowledgedActivationPending` with its proof-bound final acknowledgement and exact durable cancellation-repair tombstone or preparation. Both carry the complete `PreparedCancellationActivationBindingDigest` covering sink fence, proof-cancellation, reason, next incarnation, evidence, request, eligibility, controller capacity, accepted attempt, prepare identity and incarnation, and canonical refusal id, digest, and bytes; neither accepts `PreparedCancellationPreProofBindingDigest`. Normal additionally binds the original generation and acknowledgement with no repair fields, while repair additionally binds repair identity, initial and final generations, accumulator root, saturating retry count, unchanged refusal, final acknowledgement, and repair-floor digest with no ordinary variant. One atomic controller transaction proof-releases controller capacity, restores evidence and request reservations, returns the same request with its recorded newer incarnation, exposes fresh-incarnation eligibility, makes replay available, creates the attempt tombstone, and completes the old attempt; on repair that same transaction verifies or materializes the exact permanent cancellation-repair tombstone binding the latest committed accumulator root and retry count, then makes retained repair-intent and any remaining current-cycle source bytes ordinary eligible round input only with that tombstone durable. |
| `IssueAssignmentIssuanceOldGenerationInvalidation` | from only issuance repair `IntentRecorded`, durably enter invalidation pending and invalidate the exact old ordinary-audit generation |
| `ReplayAssignmentIssuanceOldGenerationInvalidation` | from only issuance repair `OldGenerationInvalidationPending`, replay the same invalidation and persist its proof |
| `BindAssignmentIssuanceSuccessReplacementGeneration` | from only issuance repair `OldGenerationInvalidatedRebindPending`, bind the next generation to the unchanged issuance success event and persist the rebind proof |
| `RolloverAssignmentIssuanceSuccessReplacementGeneration` | from only issuance repair `ReplacementGenerationRolloverPending`, create the next sink epoch, bind its first generation to the unchanged issuance success event, and persist the combined rollover-and-rebind proof using the same workspace and capacity |
| `CommitAssignmentIssuanceSuccessReplacementTuple` | from only issuance repair `SuccessGenerationBoundReplacementPending`, atomically fold the current definite-no-append, invalidation, and rebind-or-rollover proof digests into the fixed accumulator, increment the saturating retry count, install the proof-bound replacement tuple, make exactly those three raw proof records ordinary eligible round input, and free all three fixed proof slots without changing issuance effect, evidence, request, or capacity |
| `ReplayAssignmentIssuanceSuccessAppend` | from only issuance repair `ReplacementTupleInstalled`, replay the unchanged success event on the replacement generation |
| `QueryOrReplayAssignmentIssuanceSuccessAppend` | from only issuance repair `ReplacementSinkAcknowledgementPending`, query or replay the unchanged success event; unknown remains pending, a proof-bound acknowledgement creates the exact tombstone preparation and enters activation pending, and authenticated definite-no-append binds that current generation and re-enters the same fixed invalidation/rebind loop |
| `IssueAssignmentIssuanceCancellationOldGenerationInvalidation` | from only prepared-cancellation repair `IntentRecorded`, durably enter invalidation pending and invalidate the exact old ordinary-audit generation |
| `ReplayAssignmentIssuanceCancellationOldGenerationInvalidation` | from only prepared-cancellation repair `OldGenerationInvalidationPending`, replay the same invalidation and persist its proof |
| `BindAssignmentIssuanceCancellationRefusalReplacementGeneration` | from only prepared-cancellation repair `OldGenerationInvalidatedRebindPending`, bind the next generation to the unchanged canonical cancellation-refusal event and persist the rebind proof |
| `RolloverAssignmentIssuanceCancellationRefusalReplacementGeneration` | from only prepared-cancellation repair `ReplacementGenerationRolloverPending`, create the next sink epoch, bind its first generation to the unchanged canonical cancellation refusal, and persist the combined rollover-and-rebind proof using the same workspace and capacity |
| `CommitAssignmentIssuanceCancellationRefusalReplacementTuple` | from only prepared-cancellation repair `RefusalGenerationBoundReplacementPending`, atomically fold the current definite-no-append, invalidation, and rebind-or-rollover proof digests into the fixed accumulator, increment the saturating retry count, install the proof-bound unchanged refusal tuple, make exactly those three raw proof records ordinary eligible round input, and free all three fixed proof slots without releasing, restoring, or terminalizing |
| `ReplayAssignmentIssuanceCancellationRefusalAppend` | from only prepared-cancellation repair `ReplacementTupleInstalled`, replay the unchanged canonical cancellation refusal on the replacement generation |
| `QueryOrReplayAssignmentIssuanceCancellationRefusalAppend` | from only prepared-cancellation repair `ReplacementSinkAcknowledgementPending`, query or replay only that unchanged refusal; unknown remains pending, a proof-bound acknowledgement creates the exact tombstone preparation and enters activation pending, and authenticated definite-no-append binds that current generation and re-enters the same fixed invalidation/rebind loop |
| every remaining value above | the named internal controller action with all parameters taken from the causing refusal product |

No renderer accepts free-form command text or caller-supplied recovery
parameters. Gas City and standalone renderings may differ in transport
spelling, but both are generated from this same action and command mapping.

The refusal catalog and core plans are closed:

| Typed refusal | Causing ids | Ordered core `RemedyAction` plan |
| --- | --- | --- |
| `protected-authority-unavailable` | authority deployment, producer | `StartOrConfigureProtectedAuthority`, then `RetryProtectedPreflight` |
| `unauthorized-protected-operation` | endpoint, operation, peer alias | `UseAuthorizedEndpointIdentity`, then `RetryProtectedOperation` |
| `protected-operation-absent-from-endpoint` | endpoint, operation | `UseOperationOwningEndpoint` |
| `protected-operation-replay-conflict` | base and conflict attempt identities, accepted non-risk and non-migration endpoint operation, idempotency-key digest, request digests | `RetrySameProtectedOperationWithFreshIdempotencyKey` |
| `accepted-conflict-budget-exhausted` | exact `AcceptedConflictOperationClass`, including the risk target attempt identity when applicable, capacity generation, used and maximum conflict entries and bytes | `RiskOperation`: return the current exact handle-free `OriginalPeerRiskRecovery` state-and-action variant; only `RecoveryRead.ReadRiskRecoveryState` by `ProtectedOperator` with `FreshForThisRead` returns `ProtectedOperatorRiskRecoveryContext`, with `RequestNewRiskOperationIntent` limited to `ClosedMutationPermitted`; `CallerKeyOperation`: retry the named endpoint operation through its original authorized caller with a fresh caller key |
| `protected-operation-invalid-state` | lifecycle, operation, current state | `ReadLifecycleStatus`, then `UseStatePermittedOperation` |
| `accepted-attempt-crash-before-state` | endpoint, non-migration operation, attempt identity; assignment issuance is eligible only when no durable issuance prepare exists | `ReadProtectedAttemptStatus`, then `FollowOperationSpecificProtectedAttemptRecovery` |
| `audit-event-flush-failed` | endpoint, non-migration ordinary tuple and attempt identity; a prepared assignment-issuance success and its canonical prepared-cancellation refusal are excluded | `RestoreProtectedAuditSink`, then `ReadProtectedAttemptStatus`, then `FollowOperationSpecificProtectedAttemptRecovery` |
| `audit-event-id-conflict` | attempt identity, audit event alias, expected and actual event digests | `RepairAppendSinkIntegrity`, then `ReplayPendingAuditAppend` |
| `acceptance-prepare-digest-conflict` | sink namespace alias, attempt identity, reservation alias and generation, authoritative and presented prepare digests | `DiscardConflictingAcceptancePrepare`, then `ReplayAuthoritativeAcceptancePrepare` |
| `audit-append-authorization-invalid` | exact `AuditAppendAuthorizationRefusalProduct::Invalid` | `RequestFreshControllerAuditAppendAuthorization`, then `ReplayPendingAuditAppend` |
| `audit-append-authorization-cross-attempt` | exact `AuditAppendAuthorizationRefusalProduct::CrossAttempt` | `UseAttemptBoundAuditAppendAuthorization`, then `ReplayPendingAuditAppend` |
| `audit-sink-generation-stale` | exact `AuditAppendAuthorizationRefusalProduct::StaleGeneration` | `StopStaleAttemptWorker`, then `AttemptStatus.ReadProtectedAttemptStatus` by `OriginalAttemptPeerOrProtectedOperator` |
| `audit-append-authorization-unbound` | exact `AuditAppendAuthorizationRefusalProduct::Unbound` | `BindCurrentReservationGenerationToAuditEvent`, then `RequestFreshControllerAuditAppendAuthorization`, then `ReplayPendingAuditAppend` |
| `audit-append-authorization-event-mismatch` | exact `AuditAppendAuthorizationRefusalProduct::EventMismatch` | `RestoreCanonicalOutboxEvent`, then `RequestFreshControllerAuditAppendAuthorization`, then `ReplayPendingAuditAppend` |
| `audit-conversion-source-state-invalid` | safe attempt identity, presented state code, expected `OrdinarySinkAcknowledgementPending` code, old generation and event aliases and digests | `AttemptStatus.ReadProtectedAttemptStatus` by `OriginalAttemptPeerOrProtectedOperator`, then retry only the returned state-valid operation |
| `migration-audit-repair-tuple-mismatch` | migration attempt alias, repair state code, old or replacement generation, success event aliases and digests, closed field code | `ReadCurrentMigrationRecoveryStatus`, then `InvokeReturnedMigrationOperation` |
| `migration-audit-repair-invalid-state` | migration attempt alias, current migration state code, requested repair operation | `ReadCurrentMigrationRecoveryStatus` |
| `migration-control-epoch-stale` | migration attempt alias, presented and current controller epochs, command operation and pre-state generation | `ReturnAuditedMigrationChildRefusal`, then `ReadCurrentMigrationRecoveryStatus`, then `InvokeReturnedMigrationOperation` |
| `migration-control-state-stale` | migration attempt alias, controller epoch, child command, expected and current state generations, `Past` or `Future`, child audit digest | `ReturnAuditedMigrationChildRefusal`, then `AttemptMigrationControlConflictSignal { StaleGeneration }`, then `ReadCurrentMigrationRecoveryStatus`, then `InvokeReturnedMigrationOperation` |
| `migration-control-operation-ineligible` | migration attempt alias, controller epoch, current state generation and state code, requested child command, child audit digest | `ReadCurrentMigrationRecoveryStatus`, then `InvokeReturnedMigrationOperation` |
| `migration-control-prerequisite-tuple-mismatch` | migration attempt alias, controller epoch, current state generation, child command, closed tuple kind `SinkRepair`, `AuditAcknowledgement`, `PauseRepair`, or `DestinationVerification`, current safe expected tuple, conflicting field codes, child audit digest | `CorrectMigrationControlTuple`, then `SubmitStateCurrentMigrationChildCommand` |
| `migration-control-pause-prerequisite-unsatisfied` | migration attempt alias, controller epoch, state generation, exact generic `MigrationOperatorRepairPlanV1` variant, missing prerequisite code, child audit digest | `VerifyMigrationPausePrerequisite`, then `SubmitStateCurrentMigrationChildCommand` |
| `migration-control-replay-conflict` | migration attempt alias, controller epoch, control-reserve incarnation, fixed-slot operation, current state generation, canonical and conflicting request digests, closed conflicting field codes, child audit digest | `ReturnAuditedMigrationChildRefusal`, then `AttemptMigrationControlConflictSignal { ChangedBytes }`, then `ReadCurrentMigrationRecoveryStatus` |
| `migration-control-refusal-subslot-occupied` | exact `MigrationChildAuditCapacityRecovery::RefusalSubslotOccupied`, including fixed refusal-subslot generation, current refusal alias, and deadline; no capacity-owner failure code | `WaitForMigrationRefusalSubslotSettlement`, then `ReadCurrentMigrationRecoveryStatus`; outbox or sink-capacity repair is not constructible |
| `migration-control-child-audit-preprepare-capacity-unavailable` | exact `MigrationChildAuditCapacityRecovery::PrePrepareCapacityUnavailable`; no prepare id, alias, or digest | `RestoreMigrationChildAuditCapacity { causing_role }`, then `ReadCurrentMigrationRecoveryStatus`, then `InvokeReturnedMigrationOperation` |
| `migration-control-child-audit-prepare-reconciliation-pending` | exact `MigrationChildAuditCapacityRecovery::PrepareReconciliationPending`, including immutable prepare alias and digest | `ReconcileMigrationChildAuditPrepare`, then `ReadCurrentMigrationRecoveryStatus`; invoke only the operation returned by current status |
| `migration-control-child-audit-reconciled-and-available` | exact `MigrationChildAuditCapacityRecovery::ReconciledAndAvailable` | `ReadCurrentMigrationRecoveryStatus`, then `InvokeReturnedMigrationOperation`; never resubmit stale request bytes |
| `migration-control-child-worker-fenced` | migration attempt alias, controller epoch, command operation, command alias, `PresentedWorkerSupersededByCurrentOwnership`, and current continuation digest | `StopStaleMigrationChildWorker`, then `ReadCurrentMigrationRecoveryStatus` |
| `migration-control-reserve-integrity-corrupt` | migration attempt alias, controller epoch, state generation, source generation, quarantined reserve incarnation, epoch-and-incarnation-bound repair identity, closed reserve field codes and expected reserve digest | `ReadCurrentMigrationRecoveryStatus`, then `InvokeReturnedMigrationOperation` |
| `migration-control-reserve-repair-failed` | migration attempt alias, controller epoch, state generation, quarantined reserve incarnation, unchanged repair identity, closed construction, verification, fsync, audit, or install failure code | `RetryMigrationControlReserveRepairSameIdentity` |
| `migration-controller-rekey-required` | migration attempt alias, controller epoch, exact subset of the six counter kinds at generated admission threshold, current values, thresholds, and remaining drain budgets; no rekey identity or alias | `StartMigrationControllerRekeyIdentityFree` |
| `migration-controller-rekey-would-cross-threshold` | migration attempt alias, controller epoch, exact `DrainOnlyWouldCrossThreshold` state with all six pre-request current counter values, triggering six-component increment vector, exact would-cross counter kinds, thresholds, and remaining drain budgets; no rekey identity or alias | `StartMigrationControllerRekeyIdentityFree` |
| `migration-controller-counter-exhausted` | migration attempt alias, controller epoch, exact subset of the six counter kinds above threshold or legacy/corrupt, current values, thresholds, and integrity reason; no rekey identity or alias | `StartMigrationControllerRekeyIdentityFree` through counter-independent continuation migration |
| `migration-controller-rekey-quiescence-pending` | migration attempt alias, unchanged controller epoch, rekey identity alias, exact `MigrationControllerRekeyState` safe tag and quiescence blocker codes | `WaitForMigrationRekeyQuiescence`, then `ResumeMigrationControllerRekeyWithCurrentAlias` |
| `migration-controller-rekey-request-conflict` | migration attempt alias, unchanged controller epoch, active rekey identity alias, immutable and conflicting request digests, distinct refusal event id and event digest | `RetryMigrationControllerRekeySameIdentity` with only the active server-resolved alias |
| `migration-controller-rekey-failed` | migration attempt alias, unchanged controller epoch, rekey identity alias, exact nonterminal rekey state tag, closed construction, storage, fsync, audit, acknowledgement, or install failure code | `RetryMigrationControllerRekeySameIdentity` with only the server-resolved alias |
| `migration-status-claim-closed-for-rekey-required` | migration attempt alias, unchanged controller epoch, exact `DrainOnlyThresholdReached`, `DrainOnlyWouldCrossThreshold`, or `DrainOnlyCounterExhausted` fields; no rekey identity or alias | `StartMigrationControllerRekeyIdentityFree` |
| `migration-status-claim-closed-for-active-rekey` | migration attempt alias, unchanged controller epoch, active rekey identity alias, `DrainOnlyRekeyActive` gate code | `ResumeMigrationControllerRekeyWithCurrentAlias` |
| `migration-status-audit-capacity-unavailable` | migration attempt alias, controller epoch, status-audit slot state, bounded required and available capacity | `WaitForMigrationStatusAuditSlot`, then `ReadCurrentMigrationRecoveryStatus` |
| `migration-telemetry-health-barrier-stale` | controller epoch, presented and current marker sequences and digests, presented and current marker tags, presented and current latch tags and digests | `DiscardStaleMigrationTelemetryClosure`, then `ReadCurrentMigrationTelemetryHealth`, then `FollowCurrentMigrationTelemetryHealthRemedy` |
| `migration-telemetry-health-recovery-required` | exact non-healthy `MigrationTelemetryHealthObservation`: degraded, unavailable, update pending, recovery barrier, corrupt marker, or armed latch, with its current server-issued failure alias | `RecoverMigrationTelemetryHealth` |
| `migration-telemetry-health-recovery-retryable` | controller epoch, bound server-issued `TelemetryFailureAlias`, exact nonterminal recovery slot state, request-binding, marker, latch, and failure-alias-record digests, and closed retryable failure code; no recovery-identity parameter | `RecoverMigrationTelemetryHealth` with that bound `TelemetryFailureAlias` |
| `migration-telemetry-health-recovery-cycle-failed` | exact settled `MigrationTelemetryHealthRecoveryLastResult::Failed`, failed recovery state tag and digest, closed failure code, failure event digest, audit acknowledgement digest, and newly current server-issued failure alias | `RecoverMigrationTelemetryHealth` to start a fresh recovery cycle |
| `idempotency-result-evicted` | attempt identity, endpoint, operation, closed outcome, safe result ids, response and event digests | `ReturnOperationSpecificProtectedAttemptRecovery` |
| `protected-attempt-status-cross-peer` | attempt identity, presented peer safe alias | `UseOriginalAttemptPeerOrProtectedOperator` |
| `protected-recovery-read-cross-peer` | attempt identity, recovery-read operation, presented peer safe alias | `UseOriginalAttemptPeerOrProtectedOperator` |
| `risk-recovery-operator-authentication-required` | target attempt identity, exact risk state-and-action tag, presented original-peer alias | return only the matching handle-free `OriginalPeerRiskRecovery` variant; pending and closed-permitted name `RecoveryRead.ReadRiskRecoveryState` by `ProtectedOperator` with `FreshForThisRead`, while live, effective-revocation, and closed-forbidden own `NoFurtherAction`; never serialize the raw handle |
| `protected-status-budget-exhausted` | ordinary non-migration, non-marker status or recovery-read operation, safe target id, capacity generation, used and maximum status entries and bytes | `MigrateRetentionCapacity { VersionedBoundMigration }`, then retry the same authorized status operation |
| `attempt-worker-fenced` | attempt identity, presented and current worker epochs and generations | `StopStaleAttemptWorker` |
| `audit-sink-reservation-unavailable` | sink namespace alias, attempt identity, required entries and bytes | `RestoreProtectedAuditSinkCapacity`, then `RetryProtectedPreflight` |
| `audit-sink-orphan-proof-controller-unavailable` | sink namespace alias, attempt identity, reservation alias and generation | `RestoreProtectedController`, then `RequestNoAcceptedJournalProof`, then `RetrySinkReservationCancellation` |
| `audit-sink-orphan-proof-invalid` | sink namespace alias, attempt identity, reservation alias, generation and proof reason code | `RepairControllerSinkReservationBinding`, then `RequestNoAcceptedJournalProof`, then `RetrySinkReservationCancellation` |
| `replay-tombstone-store-full` | controller namespace alias, used and maximum entries and bytes | `MigrateRetentionCapacity { VersionedBoundMigration }`, then `RetryProtectedPreflight` |
| `audit-append-tombstone-store-full` | sink namespace alias, used and maximum entries and bytes | `MigrateRetentionCapacity { VersionedBoundMigration }`, then `RetryProtectedPreflight` |
| `recovery-reserve-unavailable` | blocker key and variant, schema and plan ids, required and available entries and bytes | `CompleteNamedReservedRecoveries`, then `RetryProtectedPreflight` |
| `recovery-reserve-integrity-corrupt` | capacity generation, affected blocker and reservation ids, integrity reason codes | `MigrateRetentionCapacity { ReserveIntegrityRepair }` |
| `retention-capacity-migration-structurally-invalid` | manifest digest and closed structural field codes | `InstallReviewedValidMigrationManifest`, then `RetryRetentionCapacityMigrationPreflight` |
| `retention-capacity-migration-unauthorized` | protected operator endpoint and presented peer safe alias | `UseProtectedOperatorEndpointIdentity`, then `RetryRetentionCapacityMigrationPreflight` |
| `retention-capacity-migration-replay-conflict` | exact `MigrationReplayConflictRefusalProduct`; no telemetry availability field, protected attempt, key, request, peer, path, handle, or deployment identifier | retry `Operator.MigrateRetentionCapacity` by `ProtectedOperator` only with a fresh caller key and otherwise identical reviewed migration intent; failure of detailed, aggregate, summary, health, or exporter writes does not change or suppress this refusal |
| `retention-capacity-migration-ineligible` | requested migration reason, capacity generation, general and recovery bounds, and `MigrationIneligibleBlockerDetails` | `CurrentBlockerDetails`: execute the carried exact plan ids, then `Operator.RunControllerRetentionCleanup` by `ProtectedOperator`; `BlockerDetailsRedacted` or `BlockerDetailsStale`: `Operator.ReadRetentionRecoveryStatus` by `ProtectedOperator`, then execute the returned exact plan ids, then `Operator.RunControllerRetentionCleanup` by `ProtectedOperator` |
| `retention-capacity-migration-already-active` | active migration attempt identity and capacity generation | `Operator.ReadMigrationRecoveryStatus` by `ProtectedOperator`, then invoke only its returned state-valid operator operation |
| `retention-capacity-migration-raw-capacity-unavailable` | requested destination capacity class and bounded required and available numerics | `ProvisionRawMigrationCapacityWithoutControllerAccess`, then `RetryRetentionCapacityMigrationPreflight` |
| `retention-classification-incomplete` | record id, retention class and state code | `InstallCorrectedRetentionClassifier`, then `RetryProtectedPreflight` |
| `selection-surface-over-bound` | candidate, measured path or byte count, table version | `SplitCandidateOrInstallReviewedSelectionTable` |
| `discovery-already-admitted` | lifecycle, discovery receipt | `ReturnToExistingLifecycle` |
| `discovery-page-incomplete` | lifecycle, seat, page-manifest and page ids, reason enum | `RedispatchCompleteDiscoveryPages` |
| `malformed-native-finding` | lifecycle, candidate, seat, finding ordinal, field code | `RedispatchCorrectedNativeFinding` |
| `terminal-lifecycle-reused` | lifecycle, terminal event | `CreateSuccessorWithAtomicImport` |
| `permanent-closed-lineage-reuse` | lineage, candidate, permanent-close event | `StartNewLineageWithNewCandidate` |
| `permanent-close-ineligible` | lineage, lifecycle state | `ResolvePermanentCloseEligibility` |
| `successor-import-capsule-over-bound` | lifecycle, measured and maximum bytes | `ContinueNamedLifecycleOrReviewedRescope`, then `RetryAbandonLifecycle` |
| `candidate-binding-stale` | lifecycle, expected and actual candidate | `RegenerateBoundArtifacts` |
| `artifact-binding-mismatch` | candidate, artifact | `RegenerateBoundArtifacts` |
| `verification-artifact-identity-conflict` | candidate, artifact identity, expected and actual digests | `ReturnToAuthorityGeneratedArtifact` |
| `manual-per-seat-artifact-substitution` | lifecycle, seat, expected and supplied artifact digests | `RegenerateAuthoritySeatArtifact` |
| `issue-view-binding-mismatch` | lifecycle, candidate, mapping version, issue ids and presented non-capability assignment or authority alias | `RequestCurrentCandidateBoundIssueView` |
| `implementation-assignment-evidence-conflict` | evidence digest and conflicting closed field codes | `RecoveryRead.ReadAssignmentRecoveryState` by `OriginalAttemptPeerOrProtectedOperator`, then follow only the returned `AssignmentRecoveryContext`; only the exact fresh-flow eligibility state can request or issue a successor |
| `implementation-assignment-self-asserted` | issuer endpoint, implementer run alias, supplied claim digest | `RequestTrustedImplementationDispatchOrResolverReceipt` |
| `implementation-assignment-recovery-context-mismatch` | presented assignment alias, source attempt operation, requested and required recovery context tags | retry `RecoveryRead.ReadAssignmentRecoveryState` with the required closed context; no generic state or action is returned |
| `implementation-assignment-completion-origin-mismatch` | exact `AssignmentCompletionRefusalProduct::OriginMismatch` | read `AssignmentRecoveryContext::CompletionRecovery`; only `ActiveCompletionRetry::UseOriginatingAssignmentCompletionPrincipalAndFreshEvidence` may retry completion, a reserved-use state waits and rereads, and a terminal context follows only its linear successor eligibility |
| `implementation-assignment-completion-binding-mismatch` | exact `AssignmentCompletionRefusalProduct::BindingMismatch` | read `AssignmentRecoveryContext::CompletionRecovery`; only `ActiveCompletionRetry::RequestFreshAssignmentBoundCompletionEvidence` may retry completion, a reserved-use state waits and rereads, and a terminal context follows only its linear successor eligibility |
| `implementation-assignment-completion-evidence-stale-or-expired` | exact `AssignmentCompletionRefusalProduct::StaleOrExpired` | read `AssignmentRecoveryContext::CompletionRecovery`; only `ActiveCompletionRetry::RequestFreshAssignmentBoundCompletionEvidence` may retry completion, a reserved-use state waits and rereads, and a terminal context follows only its linear successor eligibility |
| `implementation-assignment-completion-evidence-replayed` | exact `AssignmentCompletionRefusalProduct::EvidenceReplay` | `RecoveryRead.ReadAssignmentRecoveryState` by `OriginalAttemptPeerOrProtectedOperator`, then follow only the exact returned `AssignmentRecoveryContext` |
| `implementation-assignment-completion-evidence-conflict` | exact `AssignmentCompletionRefusalProduct::EvidenceConflict` | `RecoveryRead.ReadAssignmentRecoveryState` by `OriginalAttemptPeerOrProtectedOperator`, then follow only the returned `AssignmentRecoveryContext`; conflict never proves capability loss |
| `implementation-assignment-revocation-unauthorized` | presented assignment alias and presented principal safe alias | `ReadAssignmentRevocationRecovery` |
| `implementation-assignment-capability-unavailable` | presented assignment alias and caller declaration `CapabilityUnavailable` or `CapabilityAbandoned` | `ReadAssignmentRevocationRecovery`, then `InvokeReturnedAssignmentRecoveryAction` |
| `implementation-assignment-issuance-capacity-unavailable` | exact `AssignmentIssuanceCapacityUnavailable::ControllerBeforePrepare` or `SinkAfterPrepare`, with assignment request id, bounded required and available capacity, and issuance prepare alias only for the sink case | `RestoreAssignmentIssuanceCapacity { causing_role }`, then `RetrySamePendingAssignmentIssuance`; no free-form advice, fresh request, fresh prepare, or assignment activation is constructible |
| `implementation-assignment-issuance-prepared-recovery-retryable` | accepted attempt identity, issuance prepare alias and incarnation, exact `AssignmentIssuancePreparedRecoveryState`, canonical issuance audit tuple digest, and closed fence, cancellation, refusal-install, storage, or transaction failure code | `AttemptStatus.ReadProtectedAttemptStatus` by `OriginalAttemptPeerOrProtectedOperator`, then `InvokeReturnedAssignmentIssuanceRecoveryAction` |
| `implementation-assignment-issuance-prepared-recovery-cancelled` | accepted attempt identity, assignment request id, issuance prepare alias and fenced incarnation, closed cancellation reason code, canonical cancellation refusal audit tuple digest, and restored next prepare incarnation | `ReadCurrentAssignmentIssuanceRecovery`, then `RetrySamePendingAssignmentIssuance`; constructible only after durable refusal acknowledgement and atomic cancellation activation, so the terminal refusal proves sink cancellation, controller release, evidence and request restoration, fresh-incarnation eligibility, and old-attempt terminalization and cannot reactivate the fenced incarnation |
| `implementation-assignment-issuance-activation-source-invalid` | activation kind `IssuanceSuccess` or `PreparedCancellationRefusal`, accepted attempt identity, current state code, and the exact two expected source codes: matching ordinary activation pending or matching repair replacement-acknowledged activation pending | `AttemptStatus.ReadProtectedAttemptStatus` by `OriginalAttemptPeerOrProtectedOperator`, then invoke only the action returned by that exact state; no generic activation or conversion is constructible |
| `implementation-assignment-issuance-activation-binding-mismatch` | activation kind, accepted attempt identity, source code, expected and presented prepare identity and incarnation, the outcome-typed complete activation binding digest, source-specific generation and acknowledgement digests, canonical event id, digest, and bytes-digest tuples, and closed mismatching field codes. Success uses its sink-proof and assignment binding. Prepared cancellation requires `PreparedCancellationActivationBindingDigest` covering the pre-proof operands plus both durable cancellation proofs; `PreparedCancellationPreProofBindingDigest`, a placeholder, or a partial complete digest is always mismatch. Repair sources additionally carry expected and presented repair identity, initial and final generations, accumulator root, retry count, unchanged event, and tombstone-or-preparation digest | `AttemptStatus.ReadProtectedAttemptStatus` by `OriginalAttemptPeerOrProtectedOperator`, then `InvokeReturnedAssignmentIssuanceRecoveryAction`; caller-supplied replacement fields are never accepted |
| `implementation-assignment-issuance-cancellation-activation-retryable` | accepted attempt identity, issuance prepare alias and incarnation, canonical cancellation refusal audit tuple digest, original acknowledgement digest, exact `OrdinaryActivationPending::AssignmentIssuanceCancellation` source, and closed controller-release, evidence-restoration, request-restoration, replay, tombstone, storage, or transaction failure code | `AttemptStatus.ReadProtectedAttemptStatus` by `OriginalAttemptPeerOrProtectedOperator`, then `InvokeReturnedAssignmentIssuanceRecoveryAction`; retry only the same atomic normal-source post-audit activation |
| `implementation-assignment-issuance-audit-repair-tuple-mismatch` | accepted attempt identity, issuance prepare alias and incarnation, repair state code, initial and current repair generations, expected and presented canonical issuance audit tuple digests, accumulator roots, retry counts, current-cycle proof digests, repair-floor digests when present, and closed field code | `AttemptStatus.ReadProtectedAttemptStatus` by `OriginalAttemptPeerOrProtectedOperator`, then `InvokeReturnedAssignmentIssuanceRecoveryAction` |
| `implementation-assignment-issuance-audit-repair-invalid-state` | accepted attempt identity, issuance prepare alias and incarnation, current attempt state code, requested issuance repair action | `AttemptStatus.ReadProtectedAttemptStatus` by `OriginalAttemptPeerOrProtectedOperator`, then `InvokeReturnedAssignmentIssuanceRecoveryAction` |
| `implementation-assignment-issuance-audit-repair-retryable` | accepted attempt identity, issuance prepare alias and incarnation, unchanged issuance event id and digest, initial and current generations, accumulator root, saturating retry count, exact `AssignmentIssuanceAuditRepairState`, and closed invalidation, rebind, generation-rollover, accumulator, append, acknowledgement, tombstone, activation, or storage failure code | `AttemptStatus.ReadProtectedAttemptStatus` by `OriginalAttemptPeerOrProtectedOperator`, then `InvokeReturnedAssignmentIssuanceRecoveryAction`; reuse only the same fixed repair workspace and reserved capacity |
| `implementation-assignment-issuance-cancellation-audit-repair-tuple-mismatch` | accepted attempt identity, issuance prepare alias and incarnation, repair state code, initial and current repair generations, expected and presented canonical cancellation refusal tuple digests, accumulator roots, retry counts, current-cycle proof digests, repair-floor digests when present, and closed field code | `AttemptStatus.ReadProtectedAttemptStatus` by `OriginalAttemptPeerOrProtectedOperator`, then `InvokeReturnedAssignmentIssuanceRecoveryAction` |
| `implementation-assignment-issuance-cancellation-audit-repair-invalid-state` | accepted attempt identity, issuance prepare alias and incarnation, current attempt state code, requested cancellation repair action | `AttemptStatus.ReadProtectedAttemptStatus` by `OriginalAttemptPeerOrProtectedOperator`, then `InvokeReturnedAssignmentIssuanceRecoveryAction` |
| `implementation-assignment-issuance-cancellation-audit-repair-retryable` | accepted attempt identity, issuance prepare alias and incarnation, unchanged cancellation refusal event id and digest, initial and current generations, accumulator root, saturating retry count, exact `AssignmentIssuanceCancellationAuditRepairState`, and closed invalidation, rebind, generation-rollover, accumulator, append, acknowledgement, tombstone, activation, or storage failure code | `AttemptStatus.ReadProtectedAttemptStatus` by `OriginalAttemptPeerOrProtectedOperator`, then `InvokeReturnedAssignmentIssuanceRecoveryAction`; reuse only the same fixed repair workspace and reserved capacity |
| `implementation-assignment-revocation-audit-capacity-unavailable` | presented assignment alias, active assignment generation, dedicated controller or sink capacity role, bounded required and available capacity | `RestoreAssignmentRevocationAuditCapacity`, then `ReadAssignmentRevocationRecovery` |
| `implementation-assignment-revocation-pending` | presented assignment alias, pending reason, revocation identity alias, event id, exact audit safe state, and reserved-use count | `ReadAssignmentRevocationRecovery`, then only the exact returned action: audit work resumes, acknowledged live uses wait and reread, or acknowledged zero uses finalize |
| `implementation-assignment-revocation-audit-retry-failed` | presented assignment alias, revocation identity alias, unchanged event id and digest, exact `AssignmentRevocationAuditWorkSafeState`, and closed append, invalidation, rebind, acknowledgement, or storage failure code | `ResumeAssignmentRevocationAudit`, then `ReadAssignmentRevocationRecovery` |
| `implementation-assignment-revocation-capacity-release-pending` | presented assignment alias, exact frozen terminal intent and `AssignmentRevocationCapacityReleaseState` | invoke only its proof-cancel, controller-release, or terminal-install action, then reread assignment recovery |
| `implementation-assignment-replayed` | presented assignment alias and authenticated implementer peer or run safe alias | read `AssignmentRecoveryContext::StatusUse`; only `ActiveUsable` permits the bound `OriginalIssueReaderPeer` to use the existing capability, reserved uses wait, and no capability is issued |
| `implementation-assignment-completed` | presented assignment alias and completion event id | read `AssignmentRecoveryContext::FreshAssignmentFlow`, then follow the predecessor's exact `AssignmentSuccessorEligibility` state |
| `implementation-assignment-revoked` | presented assignment alias, revocation event id and reason code | read `AssignmentRecoveryContext::FreshAssignmentFlow`, then follow the predecessor's exact `AssignmentSuccessorEligibility` state |
| `implementation-assignment-expired` | presented assignment alias and expiry | read `AssignmentRecoveryContext::FreshAssignmentFlow`, then follow the predecessor's exact `AssignmentSuccessorEligibility` state |
| `implementation-assignment-exhausted` | presented assignment alias, activated and maximum use counts | read `AssignmentRecoveryContext::FreshAssignmentFlow`, then follow the predecessor's exact `AssignmentSuccessorEligibility` state |
| `implementation-assignment-cross-scope` | presented assignment alias and caller-supplied requested issue ids | `RequestCorrectImplementationAssignment` |
| `implementation-assignment-partition-invalid` | primary assignment safe alias, slice proposal ids, overlapping, omitted or foreign issue ids | `RegenerateDisjointImplementationPartition`, then `Orchestrator.RequestImplementationAssignment` by `OriginalOrchestratorPeer` |
| `raw-source-unmapped` | lifecycle, source ids | `RegenerateAutomaticLedger` |
| `raw-source-multiply-mapped` | source ids, issue ids | `RequestProtectedLedgerCorrection` |
| `issue-id-duplicate` | lifecycle, issue ids | `RegenerateAutomaticLedger` |
| `issue-id-reassigned` | issue id, old and proposed source digests | `RequestProtectedLedgerCorrection` |
| `ledger-synthesis-conflict` | lifecycle, ledger version, artifact digests | `ReturnToAdmittedLedger` |
| `ledger-correction-reporter-dissent` | correction, affected source ids and dissenting reporting seat ids | `ReviseProposedMappingAfterReporterDissent`, then `CollectAffectedReporterConcurrence`, then `RetryProtectedAtomicLedgerOperation` |
| `ledger-correction-dispositions-incompatible` | correction, source issue ids and disposition digests | `SubmitCompatibleImplementationDispositions`, then `RetryProtectedAtomicLedgerOperation` |
| `ledger-correction-structurally-invalid` | correction, source ids, issue ids and structural reason codes | `RegenerateStructurallyValidLedgerCorrection`, then `CollectAffectedReporterConcurrence`, then `RetryProtectedAtomicLedgerOperation` |
| `ledger-correction-stale` | correction, expected and actual ledger version | `RegenerateLedgerCorrection`, then `RetryProtectedAtomicLedgerOperation` |
| `ledger-mapping-concurrence-missing` | correction, affected source and reporting seat ids | `CollectAffectedReporterConcurrence`, then `RetryProtectedAtomicLedgerOperation` |
| `ledger-mapping-concurrence-stale` | correction, candidate, expected and actual mapping versions | `RedispatchMappingConcurrence`, then `RetryProtectedAtomicLedgerOperation` |
| `successor-import-incomplete` | logical successor import id, failed attempt identity, source lifecycle, pinned dispatch, completed-seat set and source ids | `RetryLogicalSuccessorImportWithFreshProtectedAttempt` |
| `same-scope-successor-conflict` | source lifecycle, admitted logical successor import id, optional admitted successor, and expected and proposed logical-input digests | `ReturnToAdmittedLogicalSuccessorImport` |
| `reverification-successor-ineligible` | lifecycle, receipt id and expiry | `WaitForReceiptExpiryOrUseCurrentReceipt` |
| `post-discovery-scope-expansion` | lifecycle, candidate, scope digest | `RequestProtectedRescope` |
| `post-discovery-change-unmapped` | lifecycle, candidate, changed-region ids | `MapChangeToLedgerIssueOrRequestProtectedRescope` |
| `issue-disposition-missing` | lifecycle, issue ids | `CompleteIssueDisposition` |
| `verification-coverage-incomplete` | candidate, issue ids, seat ids | `RedispatchVerificationObligations` |
| `verification-judgment-conflict` | candidate, issue ids, seat ids | `RedispatchDedicatedAdjudication` |
| `severity-correction-unauthorized` | candidate, native `SourceId` and native reporting seat id | `RedispatchNativeReportingSeatSeverityCorrection` |
| `severity-correction-unverified` | candidate and native `SourceId` values | `RedispatchIndependentVerifier` |
| `legacy-source-triage-missing` | lifecycle and exact `LegacySourceId` values with no submitted triage | `SubmitMissingLegacySourceTriage`, then `RedispatchLegacySourceTriageVerification` |
| `legacy-source-triage-unverified-or-stale` | lifecycle and exact `LegacySourceId` values with present but unverified or stale triage | `RedispatchLegacySourceTriageVerification` |
| `legacy-source-severity-correction-unauthorized` | candidate, `LegacySourceId`, historical role and current accountability role | `DispatchLegacySourceAuthorizedSeverityCorrection`, then `RedispatchIndependentVerifier` |
| `legacy-source-severity-correction-unverified` | candidate and exact `LegacySourceId` values | `RedispatchIndependentVerifier` |
| `late-finding-ineligible` | candidate, source id, submitted reason | `FileFindingOutsideLifecycle` |
| `required-validation-missing` | candidate, validation job ids | `RunRequiredEnforcingValidation` |
| `required-validation-failed` | candidate, validation job ids | `ReturnToScopedBatchFix`, then `RunRequiredEnforcingValidation` |
| `advisory-validation-used-as-evidence` | candidate, validation job ids | `RunRequiredEnforcingValidation` |
| `required-validation-marked-inapplicable` | candidate, validation job ids | `RunRequiredEnforcingValidation` |
| `companion-validation-missing` | candidate, companion ids | `RunExplicitCompanionValidation` |
| `legacy-round-start-after-cutover` | dispatch, cutover revision, schema version | `StartCurrentSchemaLifecycle` |
| `legacy-round-partial-retryable` | lifecycle, dispatch, completed and missing seat ids | `CompletePinnedLegacyRound` |
| `legacy-round-reviewer-unavailable` | lifecycle, dispatch, completed and unavailable seat ids | `CreateSameScopeCurrentSchemaSuccessor` |
| `legacy-source-unmapped` | lifecycle, legacy source ids | `RegenerateAutomaticLedger` |
| `legacy-schema-version-unsupported` | artifact digest, found and supported versions | `InstallSupportedVersionDispatcher`, then `RetryLegacyImport` |
| `legacy-regeneration-conflict` | lifecycle, import and artifact digests | `ReturnToAdmittedLegacyImport` |
| `risk-operation-replay-conflict` | conflict attempt identity, exact risk operation, intent, acceptance or revocation safe id as applicable, idempotency-key and request digests | original peer receives only the exact caller-disjoint `OriginalPeerRiskRecovery` state-and-action variant; only `RecoveryRead.ReadRiskRecoveryState` by `ProtectedOperator` with `FreshForThisRead` returns the handle-bearing `ProtectedOperatorRiskRecoveryContext`; pending states reissue the same handle and only `ClosedMutationPermitted` renders `Operator.RequestNewRiskOperationIntent` |
| `major-risk-duplicate-live` | lifecycle, candidate, acceptance ids | `Operator.RevokeMajorRiskAcceptance` by `ProtectedOperator` |
| `blocker-open` | candidate, issue ids | `ReturnToScopedBatchFix` |
| `approval-receipt-expired` | lifecycle, candidate, receipt id and expiry | `CreateReverificationSuccessor` |
| `merge-completion-binding-mismatch` | receipt, expected and actual target and candidate ids | `ResolveTrustedMergeCompletion`, then `RetryRecordMergeCompletion` |
| `round-input-store-full` | complete sorted `RetentionBlockerRecord` values with blocker digests, exact recovery plan ids, reservation aliases and reservation digests, and configured general and recovery bounds | current complete details: `ExecuteNamedRetentionRecoveryPlansInOrder` with the serialized plan ids, then `Operator.RunControllerRetentionCleanup` by `ProtectedOperator`; redacted or stale projection only: `Operator.ReadRetentionRecoveryStatus` by `ProtectedOperator`, then execute the returned exact plan ids, then `Operator.RunControllerRetentionCleanup` by `ProtectedOperator` |
| `terminal-metric-variant-invalid` | metric record id, variant and forbidden or missing field codes | `RegenerateTerminalMetricFromLifecycleReplay` |
| `redaction-contract-violation` | artifact id, field code | `RegenerateBoundedRedactedArtifact` |
| `final-verification-nonunanimous` | candidate, seat ids | `RedispatchFinalVerification` |
| `lifecycle-receipt-invalid` | candidate, receipt id, failed invariant ids | `SatisfyReceiptPrerequisites`, then `RegenerateLifecycleReceipt` |

The section 10 risk variants are rows in this same closed catalog, not a
separate open extension. The `round-input-store-full` plan is the exact
state-specific concatenation defined in section 13. Its original error already
carries the exact blocker records, blocker and reservation digests,
reservation aliases and plan ids, so
`ReadRetentionRecoveryStatus` appears only in a later projection explicitly
marked redacted or stale. It has no generic raise-the-bound or
ordinary-abandonment fallback.
`retention-capacity-migration-ineligible` uses the same closed detail rule:
complete current blocker records execute their carried plan ids directly, and
only `BlockerDetailsRedacted` or `BlockerDetailsStale` reads status before
executing the returned plan ids. There is no
unparameterized blocker-remedy action. The capsule row is the only one that
offers permanent close, and only for an already abandoned resumable lineage.
No producer context or remedy action can name offline migration, offline
controller access, or mutation of controller records by a deployment
administrator.

Every normative refusal site in this record names exactly one catalog row,
including every operation in the endpoint table. A machine-readable
operation-to-refusal map is total in both directions: an implementation
refusal with no row, a row with no reachable normative site, or an endpoint
operation with an unclassified refusal fails validation.
Completion and append-authorization rows additionally fail unless their local
error, audit event, catalog product, tombstone, protected recovery, status,
retention, log, `Debug`, and fixture surfaces all resolve to the same exact
tagged field product. The migration-preflight signal catalog is separately
total over `ReplayConflict`, `AggregateOverflow`, window summary, telemetry
health, hard TTL, compaction, rotation, generation-cutover reuse, metric
labels, and telemetry-failure fallback; a missing bound or
protected-identifier negative fails validation. The recovery-action join
against the endpoint table and caller policy is also total and has no
free-form action escape. Its eligibility relation additionally binds every
recovery and remedy action to the exact tagged source states, prerequisites,
and migration reserve routes that admit it. An action admitted from an extra
state, omitted from an eligible state, or executable without its fresh
evidence or integrity prerequisite fails generation.

After the core plan, a Gas City renderer appends
`RetryGasCityStage { stage }` only when the core action makes retry safe. A
standalone renderer analogously appends
`RerunStandaloneOperation { operation }`. A protected action remains first in
both contexts and cannot be replaced by a local edit. No remedy suggests
editing generated reviewer artifacts, hand-writing a migration crosswalk,
lowering severity, accepting BLOCKER risk, deleting protected records, or
bypassing a gate.

### 15. Validation and implementation obligations are mechanically covered

This ADR does not implement the process. The implementation must update, at
minimum:

- `.github/skills/d2b-panel-round/selection-table.json` as the sole version 2
  selection authority, with generated or byte-checked human guidance in the
  panel skill;
- the standard Copilot panel staging and dispatch path first, so it derives
  the change surface, proposed roster manifest and per-seat artifacts, submits
  them to the protected standalone authority, and dispatches only the admitted
  manifest roster without a manual omission surface;
- a protected standalone deployment or authoritative receipt resolver for the
  generalized ADR 0053 panel-and-approval controller contract, with the closed
  endpoints above and no Gas City dependency;
- `packages/xtask/src/delivery/` for lifecycle, lineage, scope, severity,
  source, ledger, correction, implementation assignment, disposition,
  judgment, acceptance, migration, logical successor import identity,
  acceptance prepare, accepted-attempt journal, idempotency result, common
  base-or-conflict attempt identity, fenced worker and sink-generation state,
  outbox, immutable tombstone, replay-payload eviction marker, sink
  reservation and authorization, exact redacted completion and append-refusal
  products, recovery reserve and plan-id binding, exhaustive protected
  recovery, narrow recovery-read endpoint, pending protected status,
  caller-disjoint safe-facts and handle-bearing risk recovery, durable
  assignment-issuance preparation, dedicated assignment-revocation audit
  recovery and unused-capacity release, fixed reusable assignment-issuance
  audit-repair workspace, accumulator, and sink-generation rollover, fixed
  reusable parent-migration child-command slots, durable immutable
  child-audit capacity prepare, child owner fencing, closed tagged redacted
  child progress, bounded child audit, controller epochs, the exact six finite
  counters, generated drain headroom, and identity-free-initial
  counter-independent rekey state machine,
  disjoint migration control and integrity reserves, observational direct
  tagged migration recovery status with one audit event per disclosure,
  migration-specific audit repair, bounded and expiring
  migration-preflight signals and summaries, fixed-cardinality telemetry
  health, durable pre-barrier failure latch and current failure alias,
  protected recovery from every non-healthy telemetry state with closed
  audit-settled success and failure, mandatory deployment-keyed
  protected-operator digest in operator-redacted migration audit events,
  low-cardinality metric labels,
  re-entrant capacity migration,
  retention, terminal metric, receipt, seal, shared selection-artifact
  validation, and typed remedy contracts;
- `.github/skills/d2b-panel-round/` for automatic discovery, compatibility,
  verification, and artifact generation;
- panel and integrator agents plus `scripts/copilot/check-bindings.mjs` for
  the closed thirteen-seat pool, including `panel-build`, exhaustive discovery
  and constrained verification without weakening read-only bindings;
- generated schemas and fixtures for every new closed type; and
- contributor and delivery documentation only when implementation lands, so
  current docs continue to describe current behavior until then.

The first delivery is atomic across the standard skill, protected standalone
authority, table, schemas, bindings, agents, staging, dispatch and verifier.
Until that cutover completes, the committed fixed ten-seat behavior remains
current. Gas City formulas and a Gas City transport adapter are a later
delivery because Gas City is not implemented. That adapter consumes the same
protected operations, authority resolution, import, retention and audit
contract and does not add another selector or authority.

The implementation maintains a machine-readable catalog of every invariant
and refusal in this record. Each catalog row names:

- the enforcing code path;
- at least one positive test;
- at least one planted negative that reaches the intended typed refusal rather
  than failing parse first;
- the validation job that executes those tests; and
- any explicit companion command required outside the normal harness.

Coverage fails when a catalog row, positive, or planted negative is missing,
when a normative refusal site and catalog row lack bidirectional parity, when
an ordinary accepted endpoint operation lacks a refusal map or terminal
`ProtectedAttemptRecovery` mapping, when a migration child command lacks its
fixed-slot result and audit mapping, when the observational migration status
read appears in accepted-attempt recovery, when a pending attempt state lacks its
status variant and exact action, when an assignment-issuance prepared-recovery
or audit-repair state lacks its exact accepted-attempt, prepare-incarnation,
audit-tuple, source-state, and action join, when prepared-handler recovery can
reach a generic crash refusal or prepared-success audit repair can reach a
generic flush refusal, when a migration state lacks its direct
status fields, operation, caller, eligibility, or reserve route, when any
recovery or remedy action lacks an exact eligible source-state join, when the
closed six-counter census differs, when the stale-contract governed-input
census differs or is empty, when the corpus is empty, or when a planted
negative is accepted. At minimum, the
corpus separately exercises:

- exact version 2 table integrity: all seven mandatory seats, all six optional
  seats including `build`, the ten-seat and eight-seat floors, the appended
  fill order, every-trigger selection beyond either floor, candidate binding,
  selected-reviewer identity, and order-independent byte-identical selection;
- a generated selector behavior matrix that enumerates every closed trigger
  operand and every continuous-integration fact enum in the versioned table,
  rather than sampled representatives. Every operand has a positive fixture
  and every operand whose predicate admits an exclusion has an appropriate
  negative fixture. The matrix explicitly includes `build`, `test`, `package`
  and `publish` facts and every registered Bazel module, lock, registry,
  repository and vendor surface. An operand or enum with no generated behavior
  case fails coverage;
- planted citation-only negatives where `Bazel` appears in non-operative
  prose, plus positive and planted-negative pairs for both rename sides,
  deleted Bazel paths, deleted build-token lines and deleted normative
  build-contract lines;
- mixed Rust, shell, Nix and build-system diffs that bind every applicable
  software profile and select `build`, without letting a profile select a seat
  or letting `build` replace `software`, `test`, `product`, or `nixos`;
- code and documentation floor-fill fixtures with no build trigger, with a
  build trigger below the floor, and with a build trigger after the floor is
  already met; the exact roster is asserted in every case and every fired
  optional remains selected;
- ambiguity and both over-bound limits, with the wider bounded result and the
  exact all-thirteen over-bound roster asserted before the separate seal and
  publication refusals;
- standard-skill staging that writes a candidate- and table-digest-bound
  roster proposal and all selected per-seat artifacts, obtains protected
  admission, dispatches every and only admitted seat, and rejects protected
  authority absence, a caller-supplied smaller roster, a removed triggered
  seat, a substituted reviewer identity, a stale artifact, a hand-edited
  per-seat artifact, and a manifest or dispatch disagreement;
- exact closed endpoint ownership for assignment issuance and completion,
  fresh orchestrator assignment requests, narrow recovery reads,
  protected-operator revocation, parent-migration child-command resume and
  fencing,
  protected-attempt status, retention-recovery status, cleanup, reviewed
  capacity migration, and migration-only direct status, sink repair, audit
  activation, control-reserve integrity repair, controller-epoch rekey, and
  telemetry-health recovery,
  with cross-endpoint planted
  negatives proving that no assignment-issuance operation can derive a new
  capability from old evidence, that a generic issuer or resolver has no
  completion or revocation right, and that
  the orchestrator still has no approval, risk, mapping,
  assignment-lifecycle, attempt-control, recovery expansion, or retention
  mutation operation. Generation fails when any ordinary
  `ProtectedAttemptRecovery` action is not `NoFurtherAction` or an operation
  in the endpoint table with one caller class authorized by that exact row.
  Risk recovery must be one exact `OriginalPeerRiskRecovery` state-and-action
  variant, and only its eligible variants may name the freshly authenticated
  protected risk read. It also fails if
  `RevokeImplementationAssignment`, any of the five migration child commands,
  `RekeyMigrationControllerEpoch`, `RecoverMigrationTelemetryHealth`, or
  `ReadMigrationRecoveryStatus` appears as a standalone protected attempt, or
  if the original-peer risk schema can carry a raw handle;
- generated or byte-identical selection guidance in `SKILL.md` covering every
  seat, and a planted agent or skill rule that attempts to self-select or
  carries guidance that drifts from the table;
- the first panel fix round after discovery with the same generated ledger,
  orchestrator-assigned proposed `R` ids, controller admission, scoped batch
  fix, verification obligations and final unanimity as the lifecycle sections
  above; selection staging may change its roster and artifacts but may not
  skip or restart that round;
- a reusable identical-input parity fixture for the future Gas City consumer.
  When Gas City lands, its controller and the standard skill must produce
  byte-identical core change surfaces, roster manifests and per-seat artifacts
  for that fixture; a forked rule, smaller roster, reordered core artifact or
  provenance field inserted into the core schema is a planted failure;
- one native discovery and refusal of a second;
- complete paged discovery, plus truncated, missing, duplicated and
  out-of-order page sets and malformed native findings;
- `SubmitLedgerSynthesisProposal` with orchestrator-assigned ids, duplicate
  grouping, controller admission, identical retry, same-key protected replay
  conflict and fresh-key admitted-generation synthesis conflict, split, merge,
  affected-reporting-reviewer concurrence, protected operator authorization,
  refusal of an orchestrator mapping mutation, and separate reporter-dissent,
  incompatible-disposition and structural-invalidity correction refusals
  whose remedies establish their prerequisites before retry;
- false BLOCKER and MAJOR invalidation, withdrawal, severity correction,
  reporting-seat dissent, the native-only and `LegacySourceId`-only
  unauthorized and unverified predicates, retired-legacy-seat accountability
  succession, and missing independent coverage;
- every disposition and judgment combination, including
  `implementation-self-review`, disposition supersession by invalid or
  withdrawn adjudication, no-content-change closure, and candidate-change
  staleness;
- automatic full-ledger per-seat artifacts, missing chunk, stale chunk,
  duplicate chunk, conflicting identity regeneration, no hand-authored
  substitute, least-authority implementer and merge-authority issue views,
  every issue-view binding mismatch, and controller-owned
  `ImplementationAssignment` request, issue, complete, revoke and resolve
  operations;
- atomic single consumption of both protected assignment evidence variants,
  the immutable `AssignmentIssuancePrepareIdentity`, its exact ordinary
  accepted `AttemptIdentity`, canonical issuance audit tuple, monotonic
  prepare incarnation, and every
  `AssignmentIssuancePrepareState` from controller reservation through sink
  create-or-adopt, sink activation, ordinary audit, and final assignment
  activation. An undersized,
  unavailable, or integrity-invalid controller or sink reservation refuses
  issuance or new use as specified and cannot be repaired by borrowing
  ordinary capacity. Controller-capacity refusal creates no prepare;
  sink-capacity refusal retains the same prepare. Both render only the closed
  capacity-restore action followed by retry of the same pending issuance.
  Crashes before sink creation, after sink creation but before controller
  adoption, after adoption, after sink activation, before and after the
  non-creatable fence, and during proof-cancellation resume the exact
  prepare. Orphan reservations are adopted only on the exact accepted-attempt,
  incarnation, assignment, and audit binding. A non-adoptable incarnation is
  durably fenced non-creatable at the sink before proof-cancellation and
  controller release. Its restored retry uses the next incarnation, and
  planted delayed create, adopt, activate, and cancel authorizations for the
  fenced incarnation all fail without capacity mutation. `Active`, `Issued`,
  settled evidence, replay availability, the capability, and the attempt
  tombstone remain unconstructible until sink activation is complete, the
  canonical issuance event is acknowledged durably, and the attempt is in
  exactly one of its two authorized sources: normal
  `OrdinaryActivationPending::AssignmentIssuance` with the original
  acknowledgement, or issuance-repair
  `ReplacementAcknowledgedActivationPending` with the proof-bound final
  acknowledgement and exact durable repair tombstone or preparation. Planted
  direct assignment activation from `PreparedForOrdinaryAudit`, a
  prepared-cancellation source, any other repair state, a different accepted
  attempt, a cross-source field, a mismatched prepare, sink proof, assignment
  binding, evidence or request reservation, eligibility or capacity binding,
  event tuple, generation, accumulator, retry count,
  acknowledgement, or repair floor, or an unacknowledged audit fails without
  mutation. Each of the two valid
  `ActivatePreparedImplementationAssignment` source fixtures
  installs `Active`, the exact `Issued` successor state, settled evidence,
  replay result, tombstone, and completed attempt atomically. The complete
  accepted-attempt crash matrix includes every boundary after
  `PreparedForOrdinaryAudit` and before the ordinary handler transaction. An
  exact valid accepted-attempt, prepare-incarnation, sink-proof,
  assignment-binding, and canonical-audit tuple resumes the same issuance
  success and never constructs `accepted-attempt-crash-before-state`.
  One-field integrity-corruption fixtures for each controller-owned join
  operand instead enter the single prepared cancellation graph. Generated
  transitions cover cancellation intent with only
  `PreparedCancellationPreProofBindingDigest`, permanent sink fence with that
  same pre-proof binding, sink proof-cancellation, atomic derivation of the
  complete `PreparedCancellationActivationBindingDigest`, refusal tuple
  installation with controller capacity, evidence, request reservation, and
  the reserved newer incarnation still quarantined, refusal append
  acknowledgement, and
  both authorized final sources: normal
  `OrdinaryActivationPending::AssignmentIssuanceCancellation` with its
  original acknowledgement, and cancellation-repair
  `ReplacementAcknowledgedActivationPending` with its proof-bound final
  acknowledgement and exact durable tombstone or preparation. Each valid
  source performs the same atomic controller release, evidence and request
  restoration, fresh-incarnation eligibility, replay availability, attempt
  tombstone, and old-attempt terminalization, with a crash and restart before
  and after every boundary.
  Strict construction and parse negatives reject a complete activation digest
  in either pre-proof state, a pre-proof digest in refusal-install, ordinary
  activation, or repair state, a missing proof operand, a zero or placeholder
  proof, a partial complete digest, and any fence-proof or cancellation-proof
  substitution. No negative may synthesize the complete digest before both
  proofs are durable.
  Independently produced and pinned literal known-answer vectors cover both
  cancellation formulas. The vector oracle must not import or call the
  production digest helper or production event encoder, and a test that only
  asks the implementation under test to compute its own expected digest is
  invalid. For `PreparedCancellationPreProofBindingDigest`, a generated
  one-operand-at-a-time mutation matrix changes the
  `d2b:panel:prepared-cancellation-pre-proof-binding:v1` domain-separator
  bytes and each of
  `accepted_attempt_identity`, `issuance_prepare_identity_digest`,
  `prepare_incarnation`, `sink_reservation_alias`,
  `sink_activation_proof_digest`, `cancellation_reason_code`,
  `next_prepare_incarnation`, `cancellation_refusal_audit_tuple_digest`,
  `evidence_reservation_binding_digest`,
  `request_reservation_binding_digest`, `successor_eligibility_digest`, and
  `controller_capacity_binding_digest`, with every other operand held to the
  known-answer bytes. For `PreparedCancellationActivationBindingDigest`, the
  same matrix separately changes the
  `d2b:panel:prepared-cancellation-activation-binding:v1` domain-separator
  bytes and each of
  `prepared_cancellation_pre_proof_binding_digest`,
  `sink_non_creatable_fence_proof_digest`, and
  `sink_absence_or_cancellation_proof_digest`. Every mutation must assert the
  exact closed mismatch field, exact state-valid mismatch reason, and exact
  typed refusal, plus byte-for-byte no mutation of controller state, sink
  state, repair workspace, reservations, eligibility, or capacity.
  The test generator forms a fail-closed cross-product between every mutation
  above and every transition that derives, stores, carries, verifies, or
  consumes the corresponding digest. The governed transition manifest is
  versioned, nonempty, and exact. For the pre-proof digest it includes initial
  derivation, entry to and recovery of `CancellationSinkFencePending`, carry
  through `PreparedAttemptCancellationSinkProofPending`, and derivation of the
  complete activation digest. For the complete activation digest it includes
  entry to and recovery of `CancellationRefusalInstallPending`, refusal tuple
  installation, normal
  `OrdinaryActivationPending::AssignmentIssuanceCancellation`, repair
  `ReplacementAcknowledgedActivationPending`, and both final activation
  commands. Every matrix cell asserts the exact refusal and byte-identical
  controller, sink, workspace, reservation, eligibility, and capacity state.
  A transition present in code or generated schemas but absent from the
  manifest, a manifest transition not discovered, an empty discovered set, or
  a mutation tested against only one producer or consumer fails coverage.
  Each case asserts that the old incarnation cannot create, adopt, activate,
  append, or mint; every pre-acknowledgement state still owns controller
  capacity, evidence, and the request reservation and exposes no
  fresh-incarnation eligibility; no terminal old attempt coexists with a live
  prepare, evidence reservation, controller reservation, or sink reservation;
  the post-acknowledgement transaction restores the request with exactly the
  recorded next incarnation; and no direct edge skips a proof, refusal audit,
  or specialized terminal activation. Planted transactions that release
  controller capacity, restore evidence or the request, expose the newer
  incarnation, make replay available, create the attempt tombstone, or
  terminalize the old attempt before acknowledgement fail atomically.
  Retryable cancellation failures expose only the exact nested action, while
  the settled terminal refusal maps only to
  `ReadCurrentAssignmentIssuanceRecovery` followed by retry of that same
  request; the complete
  `Available -> RequestPending -> IssuancePending -> Issued`
  flow for every terminal predecessor. Concurrent requests, byte-identical
  retries, changed caller keys, and fresh caller keys all consume
  `Available` once and replay one `assignment_request_id`; issuance consumes
  only that request and fresh protected evidence, resumes only the same
  accepted pending issuance after a crash, uses a newer incarnation only after
  proof-backed restoration, and activates one successor capability. Old
  evidence, a second request, a stale-incarnation authorization, or a delayed
  issuance cannot mint another. Separate fixtures prove intentional parallel
  slices use only the controller-owned disjoint partition authority and never
  predecessor successor eligibility;
- assignment self-assertion, cross-peer or cross-run replay, active to
  completed, revoked, expired and exhausted transitions, transition races,
  exact retry, use exhaustion and cross-scope access; authoritative
  trusted-dispatch and opaque-resolver issuance; binding to implementer run,
  lifecycle, candidate, mapping version and exact issue set; a full-ledger
  `PrimaryBatch`; and pairwise disjoint `ParallelFixSlice` projections with
  planted overlap, omission and foreign issue failures; completion with fresh
  assignment-bound evidence from the exact originating dispatch principal or
  resolver; cross-resolver, cross-assignment, stale, expired, replayed and
  conflicting completion-evidence refusals; same internal evidence identity
  plus a changed full assignment-binding digest as conflict rather than
  replay; every top-level `AssignmentRecoveryContext` and every nested exact
  state/action combination for status/use, completion, revocation, issue-view,
  and fresh-assignment flow after issuance replay, completion-evidence replay
  or conflict, protected-attempt replay, and replay-payload eviction. Active
  usable, active completion retry, active revocation required,
  `RevocationPending` in status, completion, revocation, and issue-view
  contexts, active temporarily reserved uses, all four terminal states, and
  all four successor eligibility states are covered without source-operation
  interpretation.
  `ReadImplementerIssueView` followed by a state read returns the same active
  assignment when a use remains, a reserved-use wait while settlement is
  pending, `RevocationCapacityReleasePending { Exhaust }` after final
  activation, and
  `Exhausted` only after proof-backed release. Payload eviction alone
  never selects loss. An explicit unavailable or abandoned capability
  declaration permits only protected operator revocation. Unauthorized,
  malformed, unresolved, ineligible, or dedicated-capacity preflight failures
  leave `Active` unchanged and create no pending state; the capacity-integrity
  case separately denies new use until repair. Accepted revocation atomically enters
  `RevocationPending`, freezes one event id, digest, and byte string, rejects
  every racing new use reservation, and exercises crashes before and after
  outbox persistence, sink append, acknowledgement persistence, each
  definite-no-append invalidation and rebind state, use settlement, and final
  transition. Append and storage failures remain nonterminal under the same
  revocation identity; definite-no-append never restores `Active` or creates a
  generic refusal. Separate races settle the last use before audit
  acknowledgement, acknowledge audit before the last use, and do both
  concurrently. Audit-work states expose only resume-audit, acknowledged
  nonzero-use state exposes only wait and reread, and acknowledged zero-use
  state exposes only finalization. Durable acknowledgement plus zero reserved
  uses enters `RevocationReadyToFinalize`; only
  `FinalizeAssignmentRevoked` enters `Revoked` and exposes successor
  `Available`. Restart, completion, expiry,
  exhaustion, duplicate revocation, and use-settlement races never return it
  to `Active`. Both orderings of final-use settlement and audit
  acknowledgement enter `RevocationReadyToFinalize`, and only
  `FinalizeAssignmentRevoked` installs `Revoked`;
  completion, expiry, and exhaustion without revocation atomically enter each
  exact `AssignmentRevocationCapacityReleaseState`, proof-cancel the sink
  reservation, release the controller reservation, and install the frozen
  terminal intent only afterward. Crashes before and after sink cancellation,
  controller release, and terminal install resume without double release,
  time-only clearing, premature cleanup, or early successor eligibility.
  Completion success and final-use recovery expose the exact
  `RevocationCapacityReleasePending` action until terminal installation;
  neither can serialize `Completed` or `Exhausted` early. A generated
  transition negative rejects every direct final-use edge to `Exhausted`;
  successor eligibility is absent until proof-backed sink cancellation,
  controller release, and terminal installation have all completed.
  `CandidateChanged`, `MappingSuperseded`, and `LifecycleTerminated`
  invalidations instead traverse the same audited `RevocationPending` graph as
  operator revocation. Dedicated-capacity undersize and unavailability,
  internal invalidation, and release-cancellation races are planted cases;
  absence of any alternate issuance or old-evidence eligibility surface;
  protected-operator revocation;
  refusal of resolver-only revocation; and only the closed candidate, mapping
  and lifecycle internal invalidations. Planted negatives prove that an old
  evidence digest, completion conflict, tombstone, payload-eviction marker,
  fresh caller key, terminal alias, or issuer identity alone cannot mint a
  capability;
- a generated assignment-completion mutation matrix starting from one valid,
  fresh, unconsumed protected completion-evidence fixture. It mutates exactly
  one of originating principal, originating issuance evidence, assignment id,
  lifecycle, candidate, mapping version, final issue set, implementer run,
  completion result, issuance, expiry, and evidence identity in each case.
  Every case asserts the exact `AssignmentCompletionOriginCode` or
  `AssignmentCompletionBindingFieldCode`, no transition to `Completed`, no
  consumption-index mutation, no private assignment or principal information
  in error, audit, log, status, or `Debug`, and no masking by another field
  predicate. Separate and multi-fault precedence cases cover stale or expired
  otherwise-bound evidence, exact identity plus immutable-evidence and full
  binding-digest equality as replay, changed full binding digest as
  `AssignmentBindingDigest` conflict even when another field is also wrong,
  and changed immutable digest with equal binding as
  `ImmutableEvidenceDigest` conflict. Every local error, canonical audit
  event, catalog record, tombstone, recovery, status, retention, log, derived
  and handwritten `Debug`, and fixture is byte-identical to its exact
  `AssignmentCompletionRefusalProduct`; field-addition, field-removal, and
  same-type field-substitution negatives fail the parity join; raw evidence
  identities, handles, paths and deployment ids are absent. Every completion
  and revocation remedy re-reads assignment state; a concurrent terminal
  transition selects only its current linear successor eligibility and cannot
  retry completion or revocation, while only
  `CompletionRecovery::ActiveCompletionRetry` or
  `RevocationRecovery::ActiveRevocationRequired` admits the applicable first
  mutation, while `RevocationAuditWorkPending`,
  `RevocationAuditAcknowledgedUsesReserved`, and
  `RevocationReadyToFinalize` admit only their exact resume, wait-and-reread,
  or finalize action. Every
  `AssignmentRevocationCapacityReleaseRecovery` variant admits only its exact
  proof-cancel, controller-release, or terminal-install action. The
  generated action-state eligibility matrix covers pending initial and
  successor request, pending issuance, active completion, active revocation,
  pending revocation, and every terminal successor action exactly, with
  planted status-only and cross-action negatives;
- `ReadImplementerIssueView` use reservation and activation as a quarantined
  authority effect, including audit failure rollback, definite-no-append
  replacement, acknowledgement loss, concurrent final-use reads, and
  byte-identical replay without a second use;
- cross-assignment error, log, audit, status, and derived and handwritten
  `Debug` fixtures proving that only the presented non-capability assignment
  alias and caller-supplied issue ids appear, while no foreign assignment id,
  safe alias, or opaque handle appears; the remedy is always
  `RequestCorrectImplementationAssignment`;
- touched and untouched late findings for every allowed reason, plus refused
  pre-existing MINOR and NIT controls;
- ledger-scoped fixes, unrelated scope expansion, atomic rescope, crash and
  retry, abandonment, bounded `SuccessorImportCapsule`, refusal over its bound,
  resume while ineligible for eviction, atomic successor import, permanent
  close, and permanent-closed-lineage reuse refusal;
- every merge-authority evidence form, same-uid standalone refusal, acceptance
  issue and revocation, controller-issued idempotency key and opaque
  `RiskOperationHandle`, identical retry, conflicting replay that first
  returns the exact current handle-free `OriginalPeerRiskRecovery`
  state-and-action variant, and response loss or replay payload eviction at
  every durable boundary. Generic
  `ProtectedAttemptRecovery`, `ReadProtectedAttemptStatus`, and original-peer
  recovery return no raw handle and no broad risk-state envelope. Each
  caller-disjoint original-peer variant owns its exact safe fields and either
  the freshly authenticated protected risk read or `NoFurtherAction`.
  Freshly operator-authenticated `ReadRiskRecoveryState` alone returns
  `ProtectedOperatorRiskRecoveryContext`; pending contexts reissue the same
  handle only there. Acceptance and
  revocation handles cannot substitute for each other, and raw handles are
  absent from audit, log, metric, tombstone, refusal, public status, and every
  `Debug` projection;
  `AcceptanceIntentPending`, `RevocationIntentPending`, `AcceptanceLive`,
  `RevocationEffective`, `ClosedMutationPermitted`, and
  `ClosedMutationForbidden` with exact owned fields and actions; planted
  cross-action negatives reject an action not owned by the exact
  `OriginalPeerRiskRecovery` variant, a raw handle in any generic or
  original-peer variant, either pending handle
  paired with the other mutation, pending state paired with new-intent
  creation, live or effective state paired with mutation, and safe-state or
  action substitution across caller classes. Only
  `ClosedMutationPermitted` can render
  `RequestNewRiskOperationIntent`, while pending intents render only their
  bound mutation and live acceptance or effective revocation never
  unconditionally creates a new intent; prohibited duplicate revocation,
  expiry at each of verification receipt, lifecycle receipt, seal,
  publication, and merge eligibility, and candidate or mapping mismatch;
- completed, in-flight, partial, retried, duplicate, malformed, and
  already-ingested legacy rounds with arbitrary recommendation strings,
  refusal to start an old-schema round after cutover, the separate retryable
  and reviewer-unavailable partial states and their linear remedies,
  unavailable-reviewer same-scope succession, completed-seat prior-obligation
  import, one fresh native discovery, atomic crosswalk, and exact metrics;
  derivation of `LogicalSuccessorImportId` from exactly source lifecycle,
  pinned legacy dispatch, completed-seat digest set, candidate, declared scope
  and compatibility schema; independence from every protected attempt and
  idempotency key; one terminal failed attempt that permanently replays its
  refusal followed by a fresh protected attempt that reaches the same
  successor and crosswalk; and refusal of every conflicting logical input;
- a partial legacy round whose completed optional `networking` and `kernel`
  reporting roles are omitted by normal native selection, proving the
  successor roster is their union, that fresh current-role agent instances
  receive bound profiles and trusted dispatch, and that the roster remains
  monotonic;
- source partial-round bytes ineligible while retryable, an atomic same-scope
  import failure that leaves them `UnavailablePartialDispatch`, keeps the
  reviewer unavailable, and permits only a fresh protected attempt for the
  same logical import;
  immediate ordinary D17 eligibility after successful import and source
  supersession; termination at imported, discovery-admitted, and
  ledger-admitted successor progress with exact `LegacySourceId` source and
  triage counts; and continuation from successor ledger or capsule state after
  those source bytes are evicted;
- exact legacy-byte preservation, deterministic source ids, complete automatic
  crosswalk, per-source migration triage, source-triage replay through split
  and merge, exact verified `LegacySourceId` triage counts, retired-seat
  correction, no invented historical severity, and exhaustive partition cases
  for missing triage versus present-but-unverified or stale triage, including a
  mixed input that first selects only the missing predicate and then only the
  unverified-or-stale predicate;
- approval-receipt seven-day cap, tighter MAJOR-acceptance cap, trusted merge
  completion, receipt-expiry merge refusal, terminal-input eligibility,
  eviction to audit-floor projections, and mandatory re-verification;
- controller-derived domain-separated `ProtectedAttemptId` and
  `ConflictAttemptId` derivation and the closed `AttemptIdentity`, including
  request-byte exclusion and cross-peer, endpoint, operation,
  conflicting-request, restart, compaction, and post-eviction addressability
  cases; one base and multiple conflict attempts with independent acceptance
  prepares, journals, sink reservations and proofs, replay results, eviction
  markers, tombstones, worker recovery, audit events and status, with planted
  key-collision or shared-state constructions refused; and the migration-only
  exception proving that a same-key different-request migration conflict is a
  preflight refusal with a stable bounded preflight signal, no accepted
  conflict attempt or accepted-attempt permanent record, repeat coalescing,
  and aggregate overflow isolation;
- acceptance prepare, accepted-attempt journal, full replay-result, pending
  outbox, immutable controller tombstone, append-only replay-payload eviction
  markers, durable sink reservation and append-sink tombstone retention and
  cleanup; both D17 round-input bounds; every atomic and two-phase eviction
  crash half-state; terminal full-result and raw sink-event eviction followed
  by identical-request no-reexecution and same-key conflicting-request
  refusal; finite tombstone-capacity refusals; and proof that cleanup never
  rewrites or deletes either replay tombstone;
- the complete inter-store acceptance protocol: crash before and after
  controller `AcceptancePrepare`, sink `Prepared`, controller atomic promotion,
  and sink `AcceptedJournalProof` binding; completion or proof-backed
  cancellation at each boundary; accepted-journal recovery from a still
  `Prepared` sink state; refusal of time-only cancellation; separate
  controller-unavailable and invalid-proof remedies; and no leaked
  reservation or accepted attempt without its bound capacity; plus the same
  `AttemptIdentity` and same prepare digest idempotent case and a conflicting
  prepare digest that returns only `acceptance-prepare-digest-conflict`,
  creates or changes nothing, and cannot be masked by a generic replay or
  orphan-proof refusal;
- durable acceptance before state processing; epoch-and-generation processing
  claim, renewal, pause and fencing; bounded pause deadlines, automatic resume
  only for `PausedSelfClearingWait` after its exact prerequisite, and
  lease-expiry takeover that preserves every
  `PausedOperatorRepairRequired` plan until prerequisite verification plus
  authenticated `ResumeProtectedAttempt`; authenticated fixed-slot child
  commands for
  `ResumeProtectedAttempt` and `FenceProtectedAttempt` under the same accepted
  migration; orphaned and expired work claimed once; a crash
  after acceptance and before the state transaction with no caller retry that
  recovers exactly one `accepted-attempt-crash-before-state` event except for
  resumable migration and assignment issuance with a durable issuance
  prepare; every compare-and-swap and crash boundary through
  quarantined result, effect and outbox, sink fsync, acknowledgement
  persistence, ordinary or migration activation pending, atomic effect or
  assignment-use activation,
  response availability and completion; and pre-registration transport,
  parse, authentication and capacity failures that create no authoritative
  attempt or effect;
- exact one-event auditing of every ordinary accepted protected success and
  refusal;
  deterministic `AuditEventId`; transactional-outbox recovery after state
  commit; crash after sink fsync but before controller acknowledgement
  persistence; idempotent same-id same-bytes replay returning the original
  acknowledgement; same-id different-bytes conflict; crash after audit
  acknowledgement and before activation or response; one unforgeably
  authorized event id and digest per monotonically increasing sink generation;
  the exact precedence and safe projections for forged or invalid
  authorization, cross-attempt identity, any past or future generation as
  stale, current generation with no authorized event as unbound, and current
  authorized event id or digest mismatch; a planted future-generation plus
  unbound multi-fault that must remain stale; planted
  multi-fault cases proving the higher-precedence reason cannot leak or be
  masked by a lower one; byte-identical
  `AuditAppendAuthorizationRefusalProduct` use in the local error, canonical
  audit event, catalog, tombstone, protected recovery, status, retention,
  log, derived and handwritten `Debug`, and fixture surfaces, with only
  `SinkNamespaceAlias`, `SinkReservationAlias`,
  `AppendAuthorizationAlias`, and `AuditEventAlias` and no
  `ControllerNamespaceAlias`, raw namespace, handle, path or deployment id.
  One generated parity join compares the canonical bytes at all governed
  surfaces, and field-addition, field-removal, and same-type field-substitution
  negatives fail every projection rather than only the fixture; generic
  definite-no-append transition atomically from the exact old
  `OrdinarySinkAcknowledgementPending` tuple, never
  `QuarantinedPendingAudit`, followed by old-generation invalidation and
  refusal-event rebind; intent recorded, old-generation
  invalidation pending, old generation invalidated with rebind pending,
  refusal generation bound with replacement tuple pending, replacement tuple
  installed, replacement append pending, and
  `OrdinaryActivationPending` crash cases;
  a delayed fenced worker's stale success append refused and never recorded by
  the controller; and no duplicate append, replay of an invalidated
  generation, success-bytes-plus-refusal-audit state, or conversion of an
  acknowledged success to refusal. Separate assignment-issuance cases start
  from the exact issuance
  `OrdinarySinkAcknowledgementPending` tuple and generate every
  `AssignmentIssuanceAuditRepairState` transition and crash boundary:
  intent, invalidation pending, invalidation proof, unchanged-success rebind,
  generation rollover, accumulator fold, replacement installation,
  replacement append pending, repeated authenticated definite-no-append,
  acknowledgement persistence, tombstone preparation or prior
  materialization, and `ActivatePreparedImplementationAssignment`. Cases with
  zero, one, two, and enough replacement no-append results to saturate the
  bounded retry count prove that every generation is invalidated before the
  next is bound, every accumulator step changes the root and commits the exact
  no-append, invalidation, and rebind-or-rollover proof digests, and the
  serialized controller record, record count, fixed workspace, and reserved
  bytes never grow. Every case retains the same
  accepted attempt, prepare identity and incarnation, sink activation proof,
  assignment binding, canonical success event and bytes, prepared effect,
  replay result, evidence, request, and controller and sink revocation
  reservations; only the ordinary-audit sink generation, accumulator root, and
  saturating retry count change. For every folded cycle, paired crash fixtures
  prove that a crash before commit preserves the prior root and retry count,
  keeps that cycle's raw definite-no-append, invalidation, and
  rebind-or-rollover proof records ineligible, and leaves all three fixed
  proof slots occupied, while a crash after commit preserves the new root and
  retry count, makes exactly those three proof records ordinary eligible
  round input, leaves all three slots reusable, and never refolds the cycle.
  Cleanup fixtures reclaim folded-cycle proof bytes only under the ordinary
  retention bounds and refuse current or unfolded proof bytes, retained
  repair-intent, and remaining current-cycle source bytes before final
  activation has committed with the permanent repair tombstone durable,
  without changing recovery state or the accumulator. The no-wrap fixture
  starts at the maximum generation, enters
  `ReplacementGenerationRolloverPending`, advances to a new sink epoch and its
  first generation using the same workspace and capacity, rejects the prior
  epoch, and never invokes `RekeyMigrationControllerEpoch`.
  One-field
  tuple mutations and wrong-source states select their exact issuance repair
  refusals and state-owned remedies. Generated negatives prove that a prepared
  issuance success cannot enter `PendingAuditConversion`, replace success with
  `audit-event-flush-failed`, release evidence or capacity, activate before
  acknowledgement, replay an invalidated generation, allocate per-retry
  controller state or capacity, reset the accumulator on restart, or pair one
  nested state with another state's action. Exact tombstone fixtures prove
  `AssignmentIssuanceAuditRepairTombstone` is keyed by the exact
  `AttemptIdentity`, has only the prepare identity digest and incarnation,
  initial and final generations, unchanged event id and digest, accumulator
  root, saturating retry count, and final acknowledgement digest, is charged
  to permanent controller tombstone capacity, and is either exact and durable
  or materialized from the exact preparation before final activation exposes
  any authority effect. Its root and count must equal the latest committed
  accumulator boundary; it neither delays nor changes prior folded-cycle proof
  eligibility. Final-activation fixtures prove retained repair-intent and any
  remaining current-cycle source bytes stay ineligible through tombstone
  preparation and become ordinary eligible round input only in the committed
  state containing the permanent repair tombstone. Missing, extra, cross-attempt,
  wrong-incarnation, wrong-initial- or final-generation, wrong-event,
  wrong-accumulator, wrong-count, wrong-acknowledgement, and
  tombstone-versus-preparation substitution fields fail.
  Separate prepared-cancellation cases start from the exact canonical refusal
  `OrdinarySinkAcknowledgementPending` tuple and generate every
  `AssignmentIssuanceCancellationAuditRepairState` and crash boundary:
  intent, old-generation invalidation pending and proof, unchanged-refusal
  rebind, generation rollover, accumulator fold, replacement installation,
  replacement append pending, repeated authenticated definite-no-append,
  acknowledgement persistence, repair-tombstone preparation or
  materialization, and `ActivatePreparedAssignmentIssuanceCancellation`.
  Zero, one, many, saturating-count, and no-wrap rollover cases make the same
  constant-state, proof-chain, same-capacity, and restart assertions as
  issuance success, including paired before-fold and after-fold crash fixtures,
  exact eligibility of the definite-no-append, invalidation, and
  rebind-or-rollover proof records, reuse of all three fixed proof slots, and
  cleanup refusal for current or unfolded proofs. They assert the refusal
  event id, digest, and bytes never change; controller capacity, evidence, the
  request reservation, newer-incarnation eligibility, replay result, and old
  attempt remain quarantined through acknowledgement; prior folded-cycle
  proofs retain ordinary eligibility while current or unfolded proofs remain
  ineligible; the cancellation-repair tombstone binds the latest committed
  accumulator root and retry count; and the
  final activation from its exact normal or repair source releases, restores,
  exposes eligibility, creates the attempt tombstone, verifies or materializes
  the permanent repair tombstone when applicable, makes retained repair-intent
  and any remaining current-cycle source bytes ordinary eligible round input
  only after that tombstone is durable, and terminalizes atomically. One-field
  tuple mutations,
  wrong-source states, and every nested state/action substitution select only
  the dedicated cancellation repair refusal and remedy. Generated precedence
  negatives prove the prepared-cancellation refusal cannot enter
  `PendingAuditConversion`, become `audit-event-flush-failed`, use the success
  repair path, replay an invalidated generation, allocate another repair
  workspace, or expose any final effect before acknowledgement and the exact
  repair floor. Separate migration cases prove
  `MigrateRetentionCapacity` cannot enter generic conversion or
  `audit-event-flush-failed`; every definite-no-append crash boundary retains
  the same migration attempt, execution, control, and integrity reserves,
  quarantined capacity-switch effect, success result and event, rebinds only
  the sink generation, retries the unchanged success append, and activates
  through the fixed-slot `CompleteMigrationAuditActivation` child command;
- explicit absence of `ProtectedAttemptId`, acceptance prepare, accepted
  journal, generic replay result, attempt tombstone, replay-payload marker, and
  generic `ProtectedAttemptRecovery` for all five migration child commands,
  the dedicated rekey operation, telemetry-health recovery, and migration
  status disclosure.
  Every primary and refusal child slot atomically enters
  `PreparingAuditCapacity` with command identity, immutable fixed-slot prepare
  identity, owner epoch, lease, deadline, required capacity, and prepare
  digest before either controller or sink reservation. Reservations bind the
  immutable command, slot role, prepare identity, and digest, never worker
  epoch. Crashes before, between, and after those reservations prove startup
  adoption of matching orphans and proof-backed cancellation of non-adoptable
  orphans. Takeover changes execution ownership only, adopts the same
  reservations through proof, and cannot make a reservation created by a
  stale worker usable. Planted cases cover stale-worker reserve, adopt,
  cancel, and promote attempts, a reservation without a durable owning
  prepare, crash before takeover, crash during adoption, and crash after
  rebind proof; no time-only cancellation is accepted. Every successful or refused privileged child effect then emits a
  distinct append-only
  sink audit event derived from migration attempt, controller epoch, reserve
  incarnation, operation, pre-state generation, child sequence, and audit
  sequence. Repeated cycles at the same semantic state produce distinct sink
  events while the controller reuses one slot and retains only current or last
  result plus audit digest. The three child-audit capacity outcomes are
  exhaustive: pre-prepare unavailable carries no prepare id, alias, or digest;
  reconciliation-pending carries the immutable prepare alias and digest and
  permits only adoption or proof-cancellation; reconciled-and-available first
  reads current migration status and invokes only the operation returned
  there. A planted renderer that resubmits stale bytes fails. Response-loss
  recovery returns in-progress, already-applied,
  exact current refusal, or superseding current state, never generic attempt
  replay. `InProgress` fixtures enumerate the six closed
  `MigrationControlCommandProgressProjection` variants and assert only each
  variant's owned command alias, safe operation, lease, deadline,
  state-generation relation, and required effect or audit digest. Planted
  cross-variant parse negatives reject a `state_tag`, every optional digest,
  a missing owned digest, an effect digest outside
  `ExternalEffectObserved`, an audit digest outside `AuditAppendPending` or
  `AuditAcknowledged`, raw owner
  epoch, external-effect identity, controller or sink reservation, outbox,
  sink handle, fencing generation, slot record generation, an effect digest
  on the wrong state, and an audit digest on the wrong state. Worker-fenced
  refusal fixtures admit only the safe superseded relation and current
  continuation digest and reject owner epochs and slot record generation;
- `round-input-store-full` fixtures for every closed blocker:
  active lifecycle, unresolved lifecycle, retryable partial dispatch,
  unavailable partial dispatch, resumable abandoned capsule, unexpired
  approval receipt, unrecorded trusted merge completion, pending acceptance
  prepare, pending accepted attempt, pending outbox, and
  `PendingAuditConversion` and `PendingMigrationAuditRepair` at each of their
  five nested states, plus `PendingAssignmentIssuanceAuditRepair` and
  `PendingAssignmentIssuanceCancellationAuditRepair` at every state in an
  enum-derived fail-closed census. The generator reads each closed repair-state
  enum and requires an exact one-to-one fixture and recovery-action mapping for
  its derived variants; a missing, extra, duplicate, unknown, or unmapped
  variant fails before fixtures run. The current derived census has eight
  variants in each family:
  `IntentRecorded`, `OldGenerationInvalidationPending`,
  `OldGenerationInvalidatedRebindPending`,
  `ReplacementGenerationRolloverPending`, the outcome-specific
  `SuccessGenerationBoundReplacementPending` or
  `RefusalGenerationBoundReplacementPending`, `ReplacementTupleInstalled`,
  `ReplacementSinkAcknowledgementPending`, and
  `ReplacementAcknowledgedActivationPending`. Each
  asserts its exact serialized blocker and blocker
  digest, recovery plan id, reservation alias and reservation digest, plus the
  configured bounds in the original refusal, then direct execution of that
  plan id without an unnecessary status read. In both eight-state censuses,
  the fold-edge fixtures assert pre-fold ineligibility and occupied slots for
  the definite-no-append, invalidation, and rebind-or-rollover proof records,
  then post-fold eligibility of exactly those three records and reuse of all
  three slots. The `ReplacementAcknowledgedActivationPending` fixtures assert
  that retained repair-intent and any remaining current-cycle source bytes
  stay ineligible with only a tombstone preparation and become eligible only
  after final activation has made the permanent repair tombstone durable.
  Conversion fixtures prove the blocker
  key is exactly `AttemptIdentity` plus old reservation generation; intent,
  invalidation-proof, and rebind-proof bytes remain ineligible before
  replacement activation; replacement activation atomically creates the
  immutable conversion tombstone digests and makes those bytes eligible; and
  every crash resumes the exact state-specific action without replaying the
  invalidated generation. Assignment-issuance repair fixtures prove both
  blockers remain specialized through replacement append,
  acknowledgement, and activation. For every enum-derived state in both repair
  families, a dedicated fixture first fills the general store and then proves
  that state's pre-reserved action succeeds without borrowing general
  capacity; this explicitly includes
  `ReplacementGenerationRolloverPending` and both final activation states.
  Those fixtures also preserve exact three-proof folded-cycle eligibility,
  three-slot reuse, current-cycle ineligibility, and final
  repair-intent/source-byte gating across recovery. Migration repair fixtures
  prove that status uses `MigrationIntegrityReserve`, repair uses
  `MigrationControlReserve`, and both preserve the success tuple. From
  `MigrationActivationPending`,
  `CompleteMigrationAuditActivation` succeeds using only
  `MigrationControlReserve` while the general, blocker-recovery, protected
  status, accepted-conflict, signal, telemetry-health, and integrity
  partitions are all full; a planted route through any other partition is
  refused without changing state. Redacted and stale post-eviction
  projections alone first use `ReadRetentionRecoveryStatus`. The general
  store is actually full, with planted failures for omitted blockers,
  reservation use by normal admission, generic bound increase and ordinary
  abandonment;
- recovery-reserve creation and roll-forward for every blocker-creating
  operation, corrupt and undersized reserve integrity states, fail-closed
  normal admission and reserved recovery from a full general store;
  schema-maximum accounting and transactional sealing of the exclusive
  `MigrationExecutionReserve`, including every controller, sink, replay,
  conversion, migration audit-repair, child-command slot, child-audit outbox
  and sink reservation, per-disclosure status audit, non-resettable epoch
  rekey record and maximum continuation inventory, telemetry marker, failure
  latch, current failure-alias record, telemetry-recovery slot,
  tombstone, and bounded work record plus disjoint sibling
  `MigrationControlReserve` and `MigrationIntegrityReserve`; all seven
  partitions, with partitions 5 and 6
  controller-wide fixed-cardinality and partition 7 transient; a separate bounded
  and duplicate-coalescing `ProtectedStatusReserve`; a separate bounded
  `AcceptedConflictReserve` for non-migration audited conflicts; a disjoint
  `MigrationPreflightSignalReserve` with its pre-sealed aggregate overflow; a
  separate fixed-cardinality `MigrationControlConflictSignalReserve` with its
  overflow and summary ring; an independently integrity-checked fixed
  `MigrationTelemetryHealthMarkerReserve` with current, shadow, separately
  reserved failure latch, current failure-alias record, and separately sealed
  recovery and audit workspace;
  and the distinct transient execution lane. Exact-bound fixtures
  consume every migration
  record class at its serialized maximum, reject a generation whose exclusive
  reserve is one entry or byte short, and reject every attempt by another
  operation to allocate from that partition. Capacity-generation fixtures
  include `AssignmentIssuanceAuditRepairTombstone` and
  `AssignmentIssuanceCancellationAuditRepairTombstone` in permanent
  controller entry and byte maxima and include one fixed accumulator,
  current-cycle proof, rollover-proof, and tombstone-preparation workspace in
  each mutually exclusive repair route. Repeating definite-no-append through
  and beyond retry-count saturation leaves those entry and byte counts
  unchanged. Migration copies every exact
  `AttemptIdentity` key and every minimal field and digest for both families,
  and a
  reviewed manifest, destination bound, copied count, copied digest, or
  recovery reservation omitting either family fails before the atomic switch.
  Fixtures completely fill the normal,
  blocker-recovery, status, and accepted-conflict budgets, then issue
  arbitrarily repeated migration conflicts and status reads and prove that a
  valid non-conflict migration remains admissible from its untouched execution
  reserve. They also prove status attempts cannot borrow that reserve,
  migration preflight conflicts create no accepted attempt or
  accepted-attempt permanent record, signal exhaustion coalesces into the
  bounded overflow without
  consuming or blocking execution, status budget exhaustion is isolated, and
  the active migration remains readable through its integrity reserve and
  resumable, fenceable, sink-repairable, and activation-completable through
  its control reserve after `ProtectedStatusReserve` is full. The complete
  generated `MigrationRecoveryStatus` variant table is exercised with exact
  safe fields, operation, caller, eligibility, and reserve route; every
  cross-state action substitution and control-versus-integrity misroute is
  refused. `ReadMigrationRecoveryStatus` fixtures reuse arbitrary caller keys
  and the same caller key across state changes, always evaluate the current
  journal, never replay stale state, occupy no mutation slot, accepted-attempt
  record, or replay schema. Two successful reads by the same operator in the
  same epoch append two distinct disclosure events; a second operator and a
  new epoch each append another distinct event. Every event contains the
  digest of the currently returned state and omits
  `protected_operator_alias`. Every event carries
  `ProtectedOperatorAuditDigest` in its canonical audit bytes, and logs,
  status, errors, metrics, refusal
  products, reservation bindings, and `Debug` reject it.
  Concurrent reads serialize on the one reusable slot or receive
  `migration-status-audit-capacity-unavailable` before disclosure. Capacity
  exhaustion, sink failure, and crashes at claim, response construction,
  outbox, fsync, acknowledgement, return, and clear disclose no unaudited
  state. Controller and sink reservations bind immutable disclosure identity,
  not operator or worker history. Crash recovery adopts only matching
  reservations and proof-cancels every nonmatching or non-adoptable
  reservation before clear; a planted time-only clear fails. Two operators
  serialize and receive distinct events, and the same sequence after a new
  controller epoch receives another distinct event. The slot retains no
  per-operator history. Corrupt
  control-reserve fixtures leave status readable and execute
  `RepairMigrationControlReserve` entirely from
  `MigrationIntegrityReserve`. Construction, verification, fsync, and install
  failures each leave the old incarnation quarantined, make the fixed
  workspace reusable, and retry the same controller-epoch-and-incarnation-bound
  repair identity. Successful install
  increments the incarnation atomically without advancing migration state; a
  later corruption at that same state produces a new identity. Unrelated
  status traffic cannot consume either reserve, and only the one accepted
  migration identity can retain the execution, control, and integrity
  allocations across pause, repair and resume. Each incarnation has exactly
  one fixed reusable slot for each of the four mutation operations, while the
  integrity reserve has one fixed repair slot and one separately sealed
  non-child rekey record.
  Byte-identical concurrent requests coalesce while their expected generation
  remains current, changed bytes return the audited closed conflict through
  the fixed refusal subslot, `Past`, `Future`, and matching generations are
  all tested, and corrected bytes after an audited tuple or prerequisite
  refusal reclaim the slot with a new child sequence.
  `RefusalSubslotOccupied` carries exactly the two-action `RemedyPlan`
  `WaitForMigrationRefusalSubslotSettlement`, then
  `ReadCurrentMigrationRecoveryStatus`; the renderer maps the latter to
  `Operator.ReadMigrationRecoveryStatus` with the causing migration alias.
  A composite wait-and-read action, either missing action, reversed order,
  extra repair action, or a wait action that itself performs the read fails
  product, enum, mapping, and catalog coverage. Repeated
  pause and resume cycles run beyond every former generated-state bound and
  reuse those slots without permanent controller growth while producing
  distinct bounded sink audit events. Fresh caller keys remain nonsemantic
  and cannot exhaust control capacity. Every primary and refusal slot is
  crashed before and after `PreparingAuditCapacity`, each controller and sink
  reservation, promotion to claim, internal mutation, external-effect request
  and observation, outbox persistence, sink fsync, acknowledgement
  persistence, and settlement;
  startup and lease-expiry recovery fence the old owner and compare-and-swap
  the exact continuation so no slot is stranded. Audit-capacity reservation
  success and exhaustion are both covered. The exact counter census is
  `MigrationStateGeneration`, `ControlReserveIncarnation`, `ChildSequence`,
  `ChildAuditSequence`, `SlotRecordGeneration`, and
  `TelemetryMarkerSequence`. Generation enumerates every legal live primary
  and refusal child slot, status-audit slot, migration-audit repair and
  activation, assignment revocation and capacity release, telemetry marker,
  latch, barrier and recovery slot, semantic continuation, and rekey
  preparation. It computes and pins each counter's exact
  `REKEY_DRAIN_HEADROOM` and `REKEY_ADMISSION_THRESHOLD`; an omitted state,
  unknown increment vector, changed inventory digest, wrong maximum, zero
  required headroom, or seventh resettable counter fails. Boundary fixtures
  run every counter immediately below, at, and above its own threshold and at
  `u64::MAX`, plus multi-component increments that would cross one or several
  thresholds. They prove the triggering request charges no component,
  atomically installs `DrainOnlyWouldCrossThreshold` with the exact
  pre-request values and triggering vector before any rekey identity exists,
  and renders only the identity-free initial rekey action. They separately
  distinguish threshold-reached and exhausted pre-request drain-only states
  from `DrainOnlyRekeyActive`, whose only rekey action carries the current
  server alias. Only vector-budgeted drain or the state-valid rekey action is
  permitted, and no counter wraps.

  Rekey fixtures start with every live primary and refusal slot state,
  stale-worker takeover, `PreparingAuditCapacity`, external effect, child or
  migration outbox, acknowledgement pending, migration-audit repair and
  activation, an active or racing status claim, every assignment-issuance
  prepare, assignment-revocation and capacity-release state, and every
  semantic continuation. After rekey is
  requested, planted new child prepares, semantic transitions, telemetry
  updates, and status claims are refused while existing disclosure and
  command/audit work drains. Each blocker either settles within its generated
  budget or moves through its typed counter-independent continuation record.
  Legacy and corrupt above-threshold fixtures use only that continuation
  route and never increment a resettable counter. Install is refused until
  every blocker is quiescent or has exactly one verified continuation.

  Telemetry rekey cases include stable detailed, aggregate, and exporter
  degradation, `UpdatePending`, `RecoveryBarrier`, corrupt marker,
  `ArmPending`, `Armed`, every recovery-slot state, and a closed failed
  recovery cycle. Each migrates its exact non-healthy marker, latch,
  failure-alias, and recovery continuation into the new epoch and remains
  recoverable afterward; none is required to become healthy and none may be
  reinterpreted as healthy. The rekey record itself never appears as its own
  blocker and is not reset.

  Initial request fixtures reject raw `RekeyIdentity`, any rekey alias, and
  any generated command parameter for raw identity. Admission atomically
  mints identity and primary outcome nonce. Later status and resume accept
  only the server-resolved non-capability alias. Byte-identical initial retry
  joins the record; changed bytes while it is active produce a separate
  audited terminal refusal with a distinct counter-independent outcome nonce
  and event id and leave the primary request unchanged. Every terminal outcome
  has a unique nonce and event. Dedicated claim and audit schemas reject all
  six resettable counters and every child, slot, reservation, and telemetry
  identity. Two complete consecutive rekeys mint distinct identities,
  primary nonces, epochs, and events, while delayed old-epoch requests remain
  stale.

  Crash fixtures cover `RequestedQuiescencePending`, `NewEpochPrepared`,
  `RekeyAuditOutboxPending`, `AuditAcknowledgedInstallPending`, atomic install,
  and `Installed`. Construction, storage, fsync, audit, acknowledgement, and
  retryable install failures resume the same controller-issued 256-bit rekey
  identity and primary outcome nonce through its alias. `Installed` is tested
  only as `MigrationControllerRekeyTerminalResult`, never an in-progress safe
  state. The success event id and canonical bytes
  remain identical on every retry, no same event id with changed bytes
  reaches the sink, success audit acknowledgement precedes install, and a
  crash after acknowledgement installs the prepared epoch without another
  identity. Atomic install resets all and only the exact six counters,
  restores continuation adapters, carries non-healthy telemetry, reopens all
  admission gates, and cannot run before durable success audit;
- refusal of every offline controller mutation, raw-capacity provisioning
  that cannot read or mutate controller records, and the only reviewed
  `MigrateRetentionCapacity` reasons for reserve repair and versioned bound
  migration; structurally invalid, unauthorized, same-key conflicting,
  ineligible, already-active and raw-capacity-unavailable migration preflight
  refusals create no accepted attempt and leave transient and permanent
  execution capacity reusable. A request with simultaneous structural,
  authorization, eligibility, and replay-conflict failures returns the first
  applicable earlier preflight refusal and emits no detailed signal,
  aggregate mutation, summary, health transition, exporter write, or metric.
  Replay conflicts always return the same exact
  `MigrationReplayConflictRefusalProduct`, including when detailed,
  aggregate, summary, health-state, or exporter writes fail. Detailed success,
  detailed-write failure, aggregate-write failure, health-state failure, and
  exporter failure are separate fixtures. Replay conflicts otherwise create
  one idempotent `MigrationPreflightSignal` with stable identity and safe
  fields; exact repeats coalesce and distinct-signal exhaustion uses
  `AggregateOverflow`. Retention fixtures assert exactly 256 reusable
  detailed slots and one overflow slot per 15-minute window, detailed hard
  expiry at 30 minutes, a 96-slot summary ring with 24-hour hard expiry,
  compaction and slot reuse at window rotation and generation cutover, and no
  retention blocker at any failure. Separate migration-control fixtures cover
  authenticated changed-byte and stale-generation signals, stable idempotent
  identity, exactly 64 reusable detailed slots plus one overflow slot per
  15-minute window, the separate 96-slot summary ring, hard expiry, rotation,
  incarnation-change reuse, aggregation collisions, and failure isolation
  from child-command, child-audit, control, and integrity capacity. The
  audited original child refusal is returned before any signal or health
  follow-up, overflow and retention follow the already settled bounded rules,
  and repeated telemetry failures never change the result.
  Unauthorized-plus-conflicting
  bytes and current-state-ineligible-plus-conflicting bytes are distinct
  negative cases and emit no detailed signal, aggregate, summary, exporter
  write, marker transition, or metric; stale authenticated requests retain
  their separately required stale signal.

  `MigrationTelemetryHealthObservation` fixtures cover `Healthy`,
  `DetailedDegraded`, `AggregateDegraded`, `ExporterDegraded`,
  `HealthUnavailable`, `UpdatePending`, `RecoveryBarrier`, `CorruptMarker`,
  and `ArmedFailureLatch`. `Healthy` alone owns `NoFurtherAction`; every other
  tag carries a current server-issued `TelemetryFailureAlias` and only
  `Operator.RecoverMigrationTelemetryHealth`. Cross-state action, missing
  alias, stale alias, and healthy-with-recovery parse negatives fail.
  Every detailed, aggregate, summary, normal-health, and exporter write proves
  the durable atomic `ArmPending` plus current failure-alias record and then
  verified `Armed` precede the write. Planted direct-write negatives and
  crashes before and after admission, `ArmPending`, `Armed`,
  `UpdatePending`, normal-health write, `StableClosureProven`, marker closure,
  recovery-slot `RecoveryBarrierInstalled`, marker `RecoveryBarrier`, probe
  verification, recovery audit outbox, acknowledgement, and final closure are
  covered. A failure before `UpdatePending` can never be restart-visible
  healthy; an untrustworthy marker receives a fresh alias in the independent
  record without marker rewrite. Later ordinary telemetry success cannot
  return healthy, and normal closure cannot clear the latch.

  Protected endpoint fixtures prove
  `RecoverMigrationTelemetryHealth` is the only operation that can install
  recovery-slot `RecoveryBarrierInstalled` and marker `RecoveryBarrier`,
  verify marker, latch, failure-alias record, detailed, aggregate, summary, and
  exporter integrity, append and fsync the protected-operator recovery event,
  and atomically install a new exact stable marker plus `Clear`.
  `RecoveryBarrierInstalled` is rejected from marker and latch schemas.
  `MigrationTelemetryHealthRecoveryEvent` is generated as the closed
  `Success | Failure` union. Success owns probe, prepared-stable, and
  normal-health fields; failure owns failed-state tag and digest plus
  `closed_failure_code`. Both carry the accepted
  `TelemetryFailureAlias`; neither carries a recovery-identity alias.
  `MigrationTelemetryHealthRecoveryLastResult` carries the same accepted
  failure alias used by the request binding. Missing, extra, cross-outcome,
  optional, and unattributed encodings are rejected.
  Byte-identical
  retry through `RecoverMigrationTelemetryHealth` uses the bound
  `TelemetryFailureAlias`, joins one internal recovery identity, and returns
  its fixed last result after response loss. Every retryable state and refusal
  carries that alias, and both renderers pass it to the existing operation.
  A recovery-identity command parameter, missing alias, substituted current
  alias, changed bytes, or unrelated stale alias cannot replace the request
  binding. Crashes at every success or failure recovery-slot state resume the
  same request binding, identity, outcome, and canonical event bytes. A crash
  after success acknowledgement recovers stable install; a crash after
  failure acknowledgement recovers failure settlement. A same event id with changed
  bytes is refused before append. A closed failure cannot rotate its alias,
  return its sealed response, or release the slot before audit
  acknowledgement. Settlement installs `LastResult::Failed`, rotates to the
  fresh failure alias, and permits a fresh recovery identity and cycle; a
  planted implementation that permanently retries only the failed identity is
  rejected. Response loss for either outcome returns the byte-identical sealed
  last result without a second event. Startup, normal telemetry, and
  every other endpoint have planted negatives for committing
  `RecoveryBarrier` followed by a stable marker. Rekey may migrate, but not
  clear, the exact barrier continuation.

  Stale normal and recovery closures race a newer marker or latch and are
  rejected by controller epoch, sequence, marker digest and tag, write or
  recovery identity, latch digest and tag, and failure-alias-record digest
  compare-and-swap without changing telemetry. They cannot overwrite a newer
  degradation. Separate
  simultaneous-failure cases prove original-refusal precedence and
  `HealthUnavailable` over aggregate, detailed, exporter, and healthy
  observations. A deliberately blocked signal, aggregate, marker,
  normal-health, and exporter path proves the priority original-refusal frame
  is delivered first with unchanged bytes and the separate observation falls
  back to `TelemetryFollowupUnavailable`. Protected identifiers are absent.
  Status-disclosure and telemetry-recovery events omit
  `protected_operator_alias`; every status event and both telemetry outcome
  events require `ProtectedOperatorAuditDigest` in canonical audit bytes and
  reject it
  from logs, status, `Debug`, metrics, refusal products, and reservations.
  Independently produced and pinned known-answer vectors, not the production
  digest helper or event encoder, verify for status events and both telemetry
  event variants that
  `ProtectedOperatorAuditDigest = keyed_digest(DeploymentAuditKey,
  "d2b:panel:protected-operator-audit:v1",
  ProtectedOperatorIdentity)`. The vector set includes two distinct
  authenticated operators under one deployment key and one operator under
  two distinct deployment keys. Planted constant-digest, swapped-operator,
  and wrong-key implementations must each fail exact event validation.
  Metric schema fixtures
  admit only the closed operation-class, signal-outcome, health-state, and
  reserve-role labels, reject capacity generation and every unbounded label,
  and keep exact generation only in protected status and audit fields, with
  already-active returning the live attempt
  status action and the raw-capacity case rendering only raw provisioning
  followed by the normal endpoint. Migration-ineligible fixtures separately
  cover complete current sorted blocker records with blocker and reservation
  digests, reservation aliases and exact plan ids, which
  execute directly, and `BlockerDetailsRedacted` and
  `BlockerDetailsStale`, which alone read retention status before executing the
  returned plan ids. Those marker reads still succeed from the blocker-bound
  coalescing read allocation after `ProtectedStatusReserve` is full; unrelated
  status cannot consume that allocation. Strict parsing rejects an unsorted, incomplete, or
  plan-id-free or digest-unbound current list, a marker with blocker fields,
  both a marker and a current list, and an unknown detail variant. No fixture
  or renderer contains an unparameterized blocker-specific remedy. Every
  `MigrationSelfClearingWaitV1` and `MigrationOperatorRepairPlanV1` variant
  has a positive exact-prerequisite and exact-action case plus missing,
  cross-plan, and generic-polling negatives: transient source read,
  destination write and verification waits; raw destination capacity
  provisioning; source and destination storage repair; manifest and
  destination verification repair. Every generic operator-repair plan ends
  only in `ResumeProtectedAttempt`. Separate fixtures prove audit-sink failure
  enters dedicated `MigrationAuditRepair` with its full repair identity,
  generations, proofs, success tuple, and sink-repair action, while
  control-reserve corruption enters `ControlReserveIntegrityCorrupt` with its
  epoch-and-incarnation-bound identity and integrity repair action. Neither
  dedicated condition can parse as `PausedOperatorRepairRequired`. An
  accepted migration follows every generic and dedicated path under the same
  `AttemptIdentity`, succeeds, releases the transient lane, and leaves its
  permanent records only in the source generation's exclusive allocation;
- post-eviction identical replay and authenticated
  `ReadProtectedAttemptStatus` for the original peer and protected operator,
  cross-peer refusal, and generated exhaustive mapping of every ordinary
  accepted operation in every endpoint for each closed terminal success
  outcome to exactly one nested operation-specific
  `ProtectedAttemptRecovery::Success` wire variant, and every ordinary
  operation and typed terminal refusal to exactly one nested
  `OriginalRefusal` variant with
  its exact safe product and reachable recovery-read operation. Every success
  next action is mechanically justified `NoFurtherAction` or names an
  endpoint-table operation and caller class authorized by that exact row.
  Risk variants additionally prove that only pending and closed-permitted
  states name the freshly authenticated protected risk read. Separate absence fixtures prove all five
  child commands, assignment revocation, rekey, telemetry-health recovery, and
  observational `ReadMigrationRecoveryStatus` have no generic recovery
  variant.
  Assignment recovery separately proves each top-level context owns only its
  nested state-valid action and that the successor request, issuance, and
  issued sequence is linear for completed, revoked, expired, or exhausted
  predecessors.
  Strict compile and parse negatives cover
  every cross-operation state, action or field substitution, missing or extra
  field, unknown outcome, optional-field encoding, generic safe-id map,
  duplicate mapping, wildcard, and fallback variant. Protected text remains
  absent, and no operation, state, action, or outcome can be selected
  independently. A generated endpoint-action join fails on an absent
  operation, wrong endpoint, wrong caller class, unauthorized recovery read,
  wrong migration reserve, or unjustified `NoFurtherAction`. A second
  action-to-state eligibility join covers every `RecoveryNextAction`,
  assignment, risk, and migration status action, and every ordered
  `RemedyAction` in the refusal catalog. It requires an exact source tagged
  state or refusal, endpoint operation, caller class, reserve route when
  applicable, and fresh-evidence prerequisite when applicable. Coverage fails
  if any recovery or remedy action has no eligible source, is eligible from an
  extra state, skips a prerequisite, or has no planted state-substitution
  negative;
- `ReadProtectedAttemptStatus` for every nonterminal variant:
  acceptance prepare, accepted unclaimed with sink `Prepared`, accepted
  unclaimed with sink `AcceptedBound`, processing,
  `PausedSelfClearingWait`, every `PausedOperatorRepairRequired` nested plan,
  quarantined pending audit, conversion intent recorded, old-generation
  invalidation pending, old
  generation invalidated with rebind pending, refusal generation bound with
  replacement tuple pending, replacement tuple installed, every nested
  `AssignmentIssuanceAuditRepairState`,
  `AssignmentIssuanceCancellationAuditRepairState`, and
  `MigrationAuditRepairState`,
  `OrdinarySinkAcknowledgementPending`,
  every `OrdinaryActivationPending` nested variant including prepared
  cancellation, `MigrationActivationPending`,
  `ControlReserveIntegrityCorrupt`, `MigrationControllerRekeyRequired`,
  `MigrationControllerRekeyWouldCrossThreshold`, and
  `MigrationControllerCounterExhausted`, plus every nested
  `MigrationControllerRekeySafeState`.
  Each asserts exactly its owned safe fields, bounded
  deadline and exact closed action. Generated strict compile and parse
  negatives cover every action from another state, generic interpretation of a
  migration tag or migration interpretation of an ordinary tag, invalidated
  generation in a later action, missing or extra lease, pause, proof,
  generation, event or acknowledgement fields, an optional-field encoding,
  and an unknown or fallback variant. Auto-resume is admitted only for a
  self-clearing wait; an operator-repair pause retains its exact plan through
  lease-expiry takeover and never degrades to polling, audit repair, or
  integrity repair;
- every top-level `TerminalLifecycleMetricRecord` variant,
  `BeforeDiscovery`, `PartialLegacyObligationsImported`,
  `DiscoveryAdmittedLedgerPending`, and `LedgerAdmitted` progress, the source
  partial lifecycle and its native successor as separate records,
  complete-legacy exact source and three-way source-triage counts, and
  compile-time construction or strict parse refusal for signed-off degraded,
  signed-off no-discovery, cross-progress, and every other contradictory wire
  shape;
- redaction and `Debug` controls, `SignedOff` only with complete admitted
  discovery, and `Abandoned` and `Superseded` with closed progress and only
  optional closed degraded reasons;
- merge-ready MINOR and NIT treatment, unresolved blocking states, final
  unanimity, and green validation without panel approval; and
- every typed error, including every new assignment, audit, retention,
  partial-round, source-triage, severity-predicate and ledger-correction
  partition; every endpoint-operation/refusal mapping; and both
  producer-context remedy renderings, with mechanical parity between
  normative refusal sites and catalog rows. The corpus has a positive,
  one-reason negative, and multi-reason precedence case for every assignment
  completion origin, binding, freshness, replay and conflict reason; conflicting
  acceptance-prepare digest; invalid, cross-attempt, past and future stale
  generation, current unbound, and current event-id, event-digest and combined
  event-mismatch append authorization; status-budget and operation-classed
  accepted-conflict-budget exhaustion; every generic conversion source-state
  and migration audit-repair refusal; every issuance-success and
  prepared-cancellation repair tuple-mismatch, invalid-state, rollover,
  accumulator, and retryable refusal; both activation-source-invalid and
  activation-binding-mismatch variants for the exact normal and repair source
  union; the normal prepared-cancellation activation-retryable refusal and
  repair-source activation failure under the dedicated repair retryable
  refusal;
  multi-fault precedence proving prepared cancellation wins over generic
  no-effect conversion; and every migration preflight and signal overflow
  reason. Risk replay conflict and risk conflict-budget exhaustion
  first return one exact handle-free `OriginalPeerRiskRecovery` tag; pending
  contexts reissue only their same protected handle through the
  `ProtectedOperatorRiskRecoveryContext` returned immediately by a fresh
  protected-operator `ReadRiskRecoveryState`, and only
  `ClosedMutationPermitted` renders `RequestNewRiskOperationIntent`, while
  only `CallerKeyOperation` renders a fresh caller key. Generated compile and
  parse negatives reject every risk variant field, outer-action, caller-class,
  handle, and protected-operation cross-variant substitution. The equivalent
  full cross-product negatives for
  `MigrationControlCommandProgressProjection` reject every tag, field, and
  digest substitution rather than sampling one optional-field case.
  Remedy fixtures separately prove
  controller-unavailable orphan recovery restores the controller before
  requesting proof and retrying cancellation, invalid orphan proof repairs the
  controller/sink binding before requesting proof and retrying, current
  `round-input-store-full` and migration-ineligible complete details execute
  their named plan ids directly, only redacted or stale details trigger a
  status read, no unparameterized blocker remedy exists, and no context renders
  an offline migration action. Every new migration, telemetry, status-audit,
  child-prepare, rekey, assignment-issuance, and assignment-revocation refusal
  row contains only closed `RemedyAction` values, including
  `RecoverMigrationTelemetryHealth`; both producer renderers generate the
  command mapping above, and a prose action, unknown action, missing command
  mapping, caller-supplied parameter, or cross-state action fails coverage;
  and
- a stale-contract absence check first discovers a sorted governed-input
  census of exact file paths, generated schema/type edges, refusal-catalog
  rows, and renderer mappings from a versioned manifest. That checked-in
  manifest is an exact enumerated inventory, not a glob, directory default, or
  best-effort discovery rule. The test asserts the
  discovered list, count, and digest exactly equal the manifest and asserts
  `count > 0`; zero discovery fails with
  `stale-contract-governed-input-empty` rather than passing vacuously, while a
  missing or extra governed input fails the census before phrase scanning.
  Over that exact nonempty census it rejects the phrases and schema edges
  removed by this decision: standalone-attempt treatment of child controls,
  child-command attempt identities, journals, replay payloads, or permanent
  tombstones, rekey as a child command or resettable slot, status-audit
  coalescing or operator-history reservation binding, handle-bearing generic
  risk recovery, a broad serialized risk-safe-state envelope beside one fixed
  action, independently selected risk state and action, optional progress
  digests or `state_tag`, worker-fenced slot generation, child reservations
  bound to worker epoch, status-only assignment mutation actions, generic
  conversion, unaudited terminalization of assignment revocation, terminal
  assignment cleanup before unused revocation-capacity cancellation,
  assignment activation before its source-specific original acknowledgement
  or proof-bound replacement acknowledgement and repair floor, reuse of a
  fenced assignment-prepare incarnation, prepared-cancellation controller
  release, evidence or request restoration, fresh-incarnation eligibility, or
  old-attempt terminalization before durable refusal acknowledgement,
  generic definite-no-append conversion of the canonical prepared-cancellation
  refusal, an assignment-repair activation surface with fewer or more than
  the exact normal and repair sources, one-shot-only replacement append
  handling, per-retry assignment-repair controller records or capacity,
  generation wrap, accumulator reset, assignment-repair definite-no-append,
  invalidation, or rebind-or-rollover proof eligibility or slot reuse before
  its cycle fold, loss of any of those three proof records' eligibility after
  that fold, repair-intent or remaining current-cycle source eligibility
  before final activation has made the permanent repair tombstone durable, or
  a repair tombstone without its
  initial and final generations, accumulator root, saturating retry count, and
  final acknowledgement digest, direct final-use installation of
  `Exhausted`,
  universal revocation-audit resume, automatic resume of operator-repair
  pauses, telemetry write before the failure latch, healthy-only telemetry
  recovery, `RecoveryBarrierInstalled` as a marker or latch,
  a telemetry recovery-identity command parameter, a composite
  refusal-subslot wait-and-read action,
  `protected_operator_alias` in status-disclosure or telemetry-recovery audit,
  a one-step-from-overflow rekey sentinel, raw rekey identity input, `Installed` as an
  in-progress rekey state, the two retired generic audit and integrity pause
  variants, the old partition count, and controller-issued per-state
  migration-control identities.

Validation selection is derived at implementation time from
`tests/layer1-jobs.json`; this ADR does not freeze today's job list. A result
whose manifest entry is advisory cannot be cited as evidence.
Fixture-contract coverage is cited from the separate enforcing
`test-fixture-contracts` job rather than a Rust shard. Affected doctests and
`harness = false` companions run explicitly because they are not nextest
surfaces. An applicability record that omits one of those affected companions
is incomplete and blocks the receipt.

The ADR index coverage gate remains required for this record. Authoring
validation is recorded in panel evidence and does not satisfy any future
implementation obligation.

## Consequences

The expected gain is fewer panel executions: native discovery happens once,
legacy work is imported instead of discarded, fixes are batched,
implementation catches mistakes before reviewers return, and ordinary
pre-existing MINOR and NIT findings cannot reopen discovery.

The initial panel becomes more demanding. Exhaustiveness cannot be proven.
Explicit prompts, complete raw output while a lifecycle needs it, bounded
cleanup after eligibility, no truncation, late-finding metrics, and the late
ledger make misses visible rather than pretending they cannot happen.

Build-system changes gain an optional specialist without raising either
minimum floor. The concrete new failure is a harmless-looking scheduler,
runfiles, cache, cross-target, dependency-authority, or packaging edit reaching
the ordinary software and test seats without anyone reviewing the build graph
that gives it effect. Version 2 build triggers and exact-roster fixtures catch
that omission. The opposite failure, selecting `build` for a prose citation,
is bounded by the registered-contract and normative-operator rule and its
planted negatives.

Shipping the standard skill first creates one usable implementation rather
than waiting for an absent Gas City. It does not make same-uid repository
helpers authoritative. The concrete failure is a contributor replacing a
staged roster, ledger or lifecycle file and asking the standard skill to admit
it. The protected standalone authority re-derives and admits those contracts,
and its absence stops before dispatch. A later Gas City controller returning a
different roster remains a drift risk; shared authority operations, table
bytes, schemas, byte-identical core artifacts and the identical-input parity
fixture make that fork mechanically visible.

The shared ledger and dedup corrections add controller state. Two defects can
be merged incorrectly or one defect split twice. Immutable sources,
append-only correction events, stable aliases, invalidation of dependent
judgments, and complete source mapping catch that failure without rewriting
history.

Independent invalid adjudication can overrule a reporting reviewer. That is
intentional: otherwise a factually wrong BLOCKER becomes permanent unless the
reporter saves face. Two independent panel judgments, a non-reporting-seat
requirement, retained dissent, and unanimous review of the adjudication
provide the guard. The integrator and operator never adjudicate the technical
truth.

Automatic compatibility is more machinery than a clean break. The concrete
failure it prevents is an active operator having to discard completed review
and fix progress. Exact old bytes, deterministic source identities, verified
source triage, complete automatic crosswalks, and idempotent generation bound
that machinery.

Implementation assignments become real protected capabilities rather than
copied dispatch metadata. The concrete failures are concurrent or fresh-key
issuance consuming one dispatch twice, and a cross-scope refusal disclosing
the capability that owns a requested issue. A third is a generic resolver
terminating an assignment it did not originate. The evidence-consumption
index, exact-origin completion evidence, operator-only revocation, assignment
state machine, stateful-read use transaction, and presented-alias-only refusal
shape catch them. A fourth failure is exposing an active assignment after the
controller reservation commits but before the sink reservation is adopted and
activated. The immutable issuance prepare, startup reconciliation, and
proof-backed cancellation make that intermediate durable without exposing a
capability or leaving an orphan reservation. A fifth failure is a generic
accepted-attempt abort or definite-no-append conversion terminalizing issuance
while its prepare, evidence, or capacity remains live. The
prepared-handler recovery join, fenced proof-cancellation path, and
dedicated unchanged-success and unchanged-cancellation-refusal audit repairs
exclude both generic refusals and make their state-action coverage generated.
A sixth failure is making retry capacity or a newer prepare incarnation
visible before the canonical cancellation refusal is durably acknowledged.
The prepared-cancellation refusal-install transaction retains controller
capacity, evidence, the request reservation, eligibility, and the old attempt
in quarantine; only its specialized final activation releases and restores
them atomically. Permanent issuance-repair tombstones bind the exact attempt,
prepare, initial and final generations, unchanged event, accumulator root,
saturating retry count, and final acknowledgement. Each accumulator fold is
the durable compaction boundary that makes exactly its raw definite-no-append,
invalidation, and rebind-or-rollover proofs ordinary eligible round input and
permits reuse of all three fixed proof slots. Final activation makes retained
repair-intent and any remaining current-cycle source bytes eligible only after
the permanent repair tombstone is durable. Repeated definite-no-append reuses
one fixed workspace and reserved capacity, and rolls the sink generation epoch
rather than wrapping or converting the
unchanged result. Reviewed capacity migration copies and budgets those floors.
The completion-specific failure is treating changed assignment bindings under
one settled evidence identity as replay. Full binding-digest precedence makes
that a conflict, but the conflict does not prove the existing capability was
lost. Exact assignment recovery returns the current state: active work keeps
the same capability, terminal work starts only through a new orchestrator
request and fresh protected evidence, and an operator who wants to abandon a
possibly lost active capability revokes it first. `RevocationPending`
atomically closes new reservations while existing ones settle. Its dedicated
audit state retries the unchanged event and never converts to a generic
refusal. Either ordering of audit acknowledgement and the last use enters
`RevocationReadyToFinalize`; only its explicit finalization installs
`Revoked`. Restart or a completion, expiry, exhaustion, audit, or use race
therefore cannot reactivate authority or expose an unaudited successor.
Unused issuance-time revocation capacity also cannot leak: completion, expiry,
and exhaustion remain in proof-backed release state until sink cancellation,
controller release, and terminal install reconcile. Their success and
recovery products expose that pending action rather than an early terminal.
Completion and append
refusals expose fixed domain-separated products, so an error path cannot leak
the raw evidence identity, sink namespace, capability, handle, path, or
deployment id while claiming to be redacted. The append product also excludes
`ControllerNamespaceAlias` from every governed projection.

Human MAJOR acceptance remains powerful. A protected authority resolver,
separate typed operation, exact candidate binding, mandatory expiry,
revocation, and repeated validity checks make it attributable and
non-transferable. Caller-disjoint recovery also prevents possession of an
original attempt identity from recovering a raw risk handle without fresh
protected-operator authentication. Exact outer variants prevent a safe state
from being paired with another state's action. These controls do not make the accepted
risk smaller.

The retention exception for active and unresolved work can fill the store.
The deliberate result is denial of new admission with every safe blocker id
and a pre-reserved state-specific remedy that still runs when the general
store is actually full, not eviction of the only evidence capable of closing
existing work. Resumable abandonment has the same cost: its bounded capsule
remains protected until resume, supersession or explicit permanent close, so
ordinary abandonment cannot be misreported as reclaimed capacity. Corrupt
reserve accounting stops normal admission and requires the one reviewed
online capacity migration. Its transient lane is not consumed by preflight
refusal, and an accepted execution fault pauses and resumes the same attempt
rather than destroying the only repair edge.
The migration-specific audit failure is more dangerous: converting a
definite-no-append success to `audit-event-flush-failed` would discard the only
capacity-switch edge. The sealed control reserve and migration audit-repair state retain the same
success and rebind only its sink generation. Four fixed reusable mutation
slots per reserve incarnation and one fixed integrity-repair command slot make
caller keys nonsemantic, reject stale epochs and generations, and prevent
repeated pause cycles or changed request bytes from allocating permanent
control history. Durable immutable command-and-slot prepare ownership prevents
an orphan controller or sink reservation across worker takeover, and the
closed tagged progress projection
keeps owner, effect, reservation, outbox, sink, and fencing identities private.
Each admitted child success or refusal still emits a distinct bounded sink
audit event, while owner epochs, leases, deadlines, recoverable
external-effect states, and startup compare-and-swap reconciliation prevent a
crashed slot claim from stranding control. The sibling integrity reserve keeps
newly evaluated current-state status and epoch-and-incarnation-bound
control-reserve repair reachable without using corrupt control capacity.
Every successful status disclosure has its own audit event rather than a
per-operator history in the reusable slot. Its reservations bind disclosure
identity. Neither it nor telemetry-recovery audit exposes the operator alias;
both require the deployment-keyed protected-operator audit digest in canonical
audit bytes and forbid it everywhere else. Finite counters never wrap: generated per-counter drain headroom closes
ordinary admission early, while budgeted drain and counter-independent
continuation migration remain available. The separate non-child rekey record
uses none of the six counters, requires every blocker to quiesce or migrate,
makes its success audit durable first, and resets all and only those six under
a new controller-issued epoch without changing semantic migration state or
resetting its own recovery record. Non-healthy telemetry safely crosses the
epoch and recovers afterward. Separate
bounded preflight and migration-control
conflict signals preserve conflict evidence without letting signal, status, or
conflict traffic consume execution, control, or integrity capacity; hard TTLs
and reusable window slots prevent diagnostic accumulation. Detailed,
aggregate, marker, normal-health, and exporter failure still return the
original refusal first. The separately reserved failure latch is durable
before `UpdatePending` and before any telemetry write, so a latch failure or a
crash before `UpdatePending` cannot reveal old healthy state. Only protected
`RecoverMigrationTelemetryHealth` can commit marker `RecoveryBarrier`, audit
the verified recovery, install a new exact stable marker, and clear the latch.
Every non-healthy state carries a current `TelemetryFailureAlias`, and a
closed failed recovery can begin a fresh cycle only after its failure event is
acknowledged, its alias rotates, and its exact last result is durable. Metric
labels contain no capacity generation.

Ordinary accepted attempts gain reconciliable prepares, common base-or-conflict
identity, sink reservations and generation authorizations, worker epochs,
immutable tombstones, and monotonic eviction markers. That is more durable
state. It prevents a live paused worker from being recovered twice, a sink
from expiring capacity below an accepted event, a delayed old-generation
append from reviving fenced success, and cleanup from claiming deleted replay
bytes still exist. Attempt response bytes remain bounded; post-eviction replay
and status retain exhaustive operation-specific safe recovery rather than
execution permission or an unusable digest-only answer.
Migration child commands, assignment revocation, epoch rekey, telemetry-health
recovery, and observational migration status are deliberate typed exceptions:
none creates another accepted attempt, replay payload, or permanent controller
tombstone.

The late-finding restriction leaves some real MINOR and NIT defects for later
work. That is the cost of merge-ready rather than perfect. Unsafe findings
remain admissible regardless of touched status, so convergence policy cannot
silence a release-blocking risk.

## Alternatives considered

### Keep repeated open-ended discovery rounds

Rejected. Every fix reopens the entire discovery surface, so peripheral
findings can indefinitely move the gate after the candidate is merge-ready.

### Cut over by discarding every in-flight old round

Rejected. It is operationally avoidable data loss. A complete old round can be
identified, preserved, given verified current source triage, and imported
without pretending it already had the new schema.

### Ask the operator to assign ids or copy reviewer notes

Rejected. Manual copying is incomplete, non-idempotent, and makes the operator
the accidental author of reviewer evidence. The orchestrator synthesizes and
assigns proposed ids; the protected controller validates and admits.

### Let repository helpers be standalone authority

Rejected. They run as the contributor uid that can author the candidate and
replace their files. They may derive and propose, but only the protected
panel-and-approval controller can admit lifecycle, roster, ledger, severity or
approval state.

### Let the integrator adjudicate a false finding or lower its severity

Rejected. The party implementing the fix cannot be the independent authority
that declares the finding false. Final-roster judgments clear false findings;
reporting seats alone authorize severity correction.

### Keep per-seat independent ledgers

Rejected. Duplicate reports become duplicate obligations with contradictory
state. One ledger retains every source and reporting-seat obligation.

### Make the new seat Bazel-only

Rejected. Bazel behavior depends on the build graph, toolchains, runfiles,
cache and remote-execution boundaries, cross-target scheduling, dependency
authority, and packaging integration around it. A Bazel-only charter would
split one causal system across seats and miss failures at those seams.

### Let each producer or operator choose its roster

Rejected. Manual relevance turns a triggered specialist into an optional cost
and gives the future Gas City path a second rule set. One versioned table,
generated manifests and exact dispatch remove the smaller-roster input while
leaving issue synthesis and `R` id assignment with the orchestrator.

### Permit every late finding indefinitely

Rejected. That recreates open-ended discovery. The closed reasons admit fix
regressions, missed BLOCKER and MAJOR findings, and unsafe risks, including in
untouched code.

### Let MINOR and NIT findings block until fixed

Rejected. It makes merge-ready false and preserves the convergence failure.
They remain durable, disposed, and independently judged without forcing a
content loop.

### Accept MAJOR risk from a same-uid standalone session

Rejected. A session that can author the candidate cannot authenticate itself
as independent merge authority by writing another local record. Without a
protected resolver the MAJOR is fixed, not accepted.

### Replace unanimous sign-off with a majority vote

Rejected. This decision narrows what verification may block; it does not
weaken the final panel. Every selected reviewer still signs off, and the
controller independently checks ledger, validation, acceptance, and lineage
criteria.
