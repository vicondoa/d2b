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
  migration-conflict preflight with hard-bounded expiring isolated signals and
  fixed-cardinality telemetry health, exclusive migration execution capacity,
  controller-issued state-bound control identities, disjoint control and
  integrity reserves, direct tagged protected and recovery status,
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
| Operator | `ProtectedOperator`: protected operator identity resolved from peer evidence | `SubmitApprovalDecision`, `AbandonLifecycle`, `ResumeLifecycle`, `RescopeLifecycle`, `CreateSameScopeCurrentSchemaSuccessor`, `CreateReverificationSuccessor`, `PermanentlyCloseAbandonedLineage`, `ApplyLedgerMappingCorrection`, `IssueRiskOperationIntent`, `RequestNewRiskOperationIntent`, `AcceptMajorRisk`, `RevokeMajorRiskAcceptance`, `RevokeImplementationAssignment`, `ResumeProtectedAttempt`, `FenceProtectedAttempt`, `ReadLifecycleStatus`, `ReadRetentionRecoveryStatus`, `RunControllerRetentionCleanup`, `MigrateRetentionCapacity`, `ReadMigrationRecoveryStatus`, `RepairMigrationSinkAppend`, `CompleteMigrationAuditActivation`, `RepairMigrationControlReserve` |
| Assignment issuance | `ExactOriginatingAssignmentIssuancePrincipal`: exact controller-owned trusted implementation-dispatch principal or authoritative opaque-receipt resolver that owns the presented protected issuance evidence; evidence must be fresh to create a new assignment | `IssueImplementationAssignment` |
| Assignment completion | `ExactOriginatingAssignmentCompletionPrincipal`: exact trusted dispatch principal or authoritative resolver identity recorded by the originating issuance | `CompleteImplementationAssignment` |
| Issue reader | `OriginalIssueReaderPeer`: authenticated implementer peer presenting an opaque assignment handle, or resolved merge authority | `ResolveImplementationAssignment`, `ReadImplementerIssueView`, `ReadMergeAuthorityMajorIssueView` |
| Attempt status | `OriginalAttemptPeerOrProtectedOperator`: authenticated original peer for the named `AttemptIdentity`, or protected operator identity | `ReadProtectedAttemptStatus` |
| Recovery read | `OriginalAttemptPeerOrProtectedOperator`: authenticated original peer for the named `AttemptIdentity`, or protected operator identity | `ReadLifecycleRecoveryState`, `ReadLedgerRecoveryState`, `ReadAssignmentRecoveryState`, `ReadRiskRecoveryState`, `ReadArtifactRecoveryState`, `ReadRetentionRecoveryState`, `ReadPublicationRecoveryState`, `ReadOriginalRefusalRecoveryState` |
| Publisher | `ProtectedPublisher`: protected publisher identity | `ConsumePublicationManifest`, `RecordTrustedMergeCompletion`, `ReadPublicationStatus` |

`SubmitApprovalDecision` retains D17's closed
`{approve, revise, rescope, abort}` value. Approval and risk operations,
ledger-mapping mutation, lifecycle termination and permanent close are absent
from the orchestrator endpoint. Status reads do not mutate lifecycle,
assignment, or retention domain state, although section 13 still audits their
accepted attempts. `ReadImplementerIssueView` is not a status read.
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
mutate any domain. A protected operator receives the same redacted product,
not a privileged expansion. Lifecycle, ledger, assignment, risk, artifact,
retention, and publication recovery all use the corresponding operation in
the table rather than an invented generic read.
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
Every request frame carries an idempotency key, every operation is
candidate-bound where a candidate exists, and every endpoint uses the audit
and idempotency contract in section 13. Risk intents use the stronger
controller-issued-key rule in section 10.

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
ImplementationEvidenceConsumption {
  evidence_id,
  controller_private_assignment_id,
  assignment_binding_digest,
  state: Settled
}
```

The binding digest covers assignment kind, lifecycle, candidate, mapping
version, exact issue set, file-ownership digest when present, authenticated
implementer run, use limit, issuance, expiry, and the exact originating
principal identity. Issuance first acquires a fenced unique reservation for
`evidence_id`; activation atomically converts that reservation and the
assignment into the one consumption record in `Settled` state. Originating
evidence is then consumed for authority purposes: it can identify and replay
the settled assignment but cannot mint another one. Two concurrent issuances
therefore cannot both mint, and a crash can expose neither an unindexed
assignment nor a settled consumption without its assignment. Reissuing
byte-identical evidence and bindings returns the same assignment even under a
fresh idempotency key. Reusing that evidence with a different kind, issue set,
candidate, mapping version, implementer run, expiry, origin, use limit,
lifecycle, or file-ownership digest is
`implementation-assignment-evidence-conflict`, with a closed field code.
Different bytes under the same key are the section 13 protected replay
conflict and never reach evidence evaluation. A fresh key never bypasses the
evidence index. A genuinely new assignment requires fresh protected dispatch
or resolver evidence. Definite-no-append conversion under section 13 removes
an unactivated reservation and assignment together; evidence becomes
settled only when the audited issuance effect activates.

The controller-private assignment id and opaque capability handle never
appear in an error, log, audit event, status projection, or `Debug`. A
domain-separated `PresentedAssignmentAlias` is a non-capability digest used
for safe correlation; possessing it cannot resolve or use an assignment.

An assignment is either single-use or carries a closed use limit no greater
than the versioned controller maximum. Its controller-owned state is exactly:

```
ImplementationAssignmentState =
  Active { activated_uses, reserved_uses }
  | Completed { completion_event_id }
  | Revoked { revocation_event_id, reason_code }
  | Expired { expired_at }
  | Exhausted { activated_uses }
```

`IssueImplementationAssignment` creates `Active`.
`CompleteImplementationAssignment` moves `Active` to `Completed` only through
the assignment-completion endpoint. It requires fresh, single-consumption,
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
evidence digest, and full assignment-binding digest atomically with
`Completed`.

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

Assignment recovery is state-valid and never infers capability loss from
replay-payload eviction:

```
AssignmentRecoveryStatus =
  Active {
    presented_assignment_alias,
    activated_uses,
    reserved_uses,
    maximum_uses,
    expires_at,
    next: UseExistingCapability {
      resolve: IssueReader.ResolveImplementationAssignment,
      issue_view: IssueReader.ReadImplementerIssueView,
      caller: OriginalIssueReaderPeer
    }
  }
  | Completed {
      presented_assignment_alias,
      completion_event_id,
      next: RequestFreshAssignment {
        request: Orchestrator.RequestImplementationAssignment
          by OriginalOrchestratorPeer,
        issuance: AssignmentIssuance.IssueImplementationAssignment
          by ExactOriginatingAssignmentIssuancePrincipal,
        evidence: FreshProtectedImplementationEvidence
      }
    }
  | Revoked {
      presented_assignment_alias,
      revocation_event_id,
      reason_code,
      next: RequestFreshAssignment {
        request: Orchestrator.RequestImplementationAssignment
          by OriginalOrchestratorPeer,
        issuance: AssignmentIssuance.IssueImplementationAssignment
          by ExactOriginatingAssignmentIssuancePrincipal,
        evidence: FreshProtectedImplementationEvidence
      }
    }
  | Expired {
      presented_assignment_alias,
      expired_at,
      next: RequestFreshAssignment {
        request: Orchestrator.RequestImplementationAssignment
          by OriginalOrchestratorPeer,
        issuance: AssignmentIssuance.IssueImplementationAssignment
          by ExactOriginatingAssignmentIssuancePrincipal,
        evidence: FreshProtectedImplementationEvidence
      }
    }
  | Exhausted {
      presented_assignment_alias,
      activated_uses,
      maximum_uses,
      next: RequestFreshAssignment {
        request: Orchestrator.RequestImplementationAssignment
          by OriginalOrchestratorPeer,
        issuance: AssignmentIssuance.IssueImplementationAssignment
          by ExactOriginatingAssignmentIssuancePrincipal,
        evidence: FreshProtectedImplementationEvidence
      }
    }
```

An active assignment returned by issuance replay, completion-evidence replay
or conflict, protected-attempt replay, or post-eviction recovery remains the
same assignment. Its bound issue-reader peer uses the existing opaque
capability; the recovery product never returns, remints, or substitutes one.
A caller that consumed `ReadImplementerIssueView` first reads this assignment
state. If an active use remains, the same assignment may be used again. If
the final use activated, the returned variant is `Exhausted`. Completed,
revoked, expired, and exhausted variants require a new orchestrator
`RequestImplementationAssignment` and newly issued
`TrustedImplementationDispatch` or `OpaqueImplementationResolverReceipt`
evidence before `IssueImplementationAssignment` can create another
capability. Old issuance evidence, an old completion conflict, a tombstone,
or replay-payload eviction is never sufficient.

If an operator needs to abandon an active capability that may have been lost,
the operator first invokes `RevokeImplementationAssignment`. Only after the
revoked state is durable may the orchestrator request a new assignment and
the issuance principal present fresh protected evidence. The generated
assignment-state-to-action join admits `UseExistingCapability` only for
`Active`, admits `RequestFreshAssignment` only for the four terminal states,
checks both endpoint operations and caller classes against the endpoint
table, and rejects a new-capability action derived from replay or payload
availability.

`RevokeImplementationAssignment` is not present on either assignment endpoint.
Only the protected operator endpoint may request revocation. The controller
may also perform one closed internal invalidation transition, with no caller
operation, for exactly `CandidateChanged`, `MappingSuperseded`, or
`LifecycleTerminated`. Operator revocation and internal invalidation both
carry a closed reason code, append an audit event, and wait for every live use
reservation to resolve. The originating issuer or resolver alone cannot
revoke. The authoritative clock refuses a new reservation at or after expiry,
but does not invalidate a use reserved before expiry.
Settlement of the last live reservation moves the assignment to `Exhausted`
when the use limit is reached, otherwise to `Expired` when the expiry has
passed, and otherwise back to unreserved `Active`. These terminal states are
append-only and have no outgoing transition. Every transition uses the
assignment generation in a compare-and-swap, so completion, revocation,
expiry, exhaustion, and a concurrent read have one winner and no
last-writer-wins overwrite.

Resolution authenticates and binds the handle but does not consume a use.
`ReadImplementerIssueView` is a least-authority stateful read. A successful
distinct attempt reserves one available use by compare-and-swap. The
reservation is a quarantined authority effect committed with the terminal
journal, replay result, and outbox; section 13 activates the use atomically
with the other authority effects in the final transaction after audit
acknowledgement persistence. The final use activates `Exhausted`. A
byte-identical retry replays the original attempt and never reserves or
consumes again. A definite-no-append conversion releases the quarantined
reservation in the same replacement transaction. A concurrent fresh read that
finds all remaining uses reserved waits on the owning attempt's terminal
transition; it then either acquires a released use or
reaches the activated `Exhausted` state, rather than oversubscribing or
inventing a sixth lifecycle state. No uid equality,
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

The controller issues the idempotency key for each risk-operation intent
before it accepts the operation bytes. For each risk operation, the same key and
byte-identical request returns the original event and response while full
result bytes remain, and section 13's exact nested post-eviction recovery
variant afterwards; neither path re-executes. There are no separate
risk-state wire types.

Risk recovery is exactly the section 13 product:

```
ProtectedAttemptRecovery::Success::Operator::
  RiskOperationIntent<Outcome> {
    intent_id, candidate_id, intent_event_id,
    next: Invoke {
      endpoint: RecoveryRead,
      operation: ReadRiskRecoveryState,
      caller: OriginalAttemptPeerOrProtectedOperator
    }
  }

ProtectedAttemptRecovery::Success::Operator::
  NewRiskOperationIntent<Outcome> {
    intent_id, candidate_id, intent_event_id,
    next: Invoke {
      endpoint: RecoveryRead,
      operation: ReadRiskRecoveryState,
      caller: OriginalAttemptPeerOrProtectedOperator
    }
  }

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

`<Outcome>` is expanded by the generator into a distinct operation-and-outcome
wire tag; it is not a serialized state field. A risk refusal is the
corresponding
`ProtectedAttemptRecovery::OriginalRefusal::Operator::<Operation,Refusal>`
variant with the catalog's exact safe refusal product and a reachable
`RecoveryRead.ReadOriginalRefusalRecoveryState` next operation. The recovery
projection never substitutes a digest-only state envelope.

`ReadRiskRecoveryState` returns this exact tagged safe product directly:

```
RiskRecoveryStatus =
  AcceptanceIntentPending {
    intent_id,
    candidate_id,
    proposed_mutation_digest,
    next: Operator.AcceptMajorRisk by ProtectedOperator
  }
  | RevocationIntentPending {
      intent_id,
      candidate_id,
      acceptance_id,
      proposed_mutation_digest,
      next: Operator.RevokeMajorRiskAcceptance by ProtectedOperator
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

The tag owns its fields and action; state and action are not independently
selectable. Only `ClosedMutationPermitted`, after the controller proves that
no pending intent, live acceptance, or already-effective revocation forbids
another intent, can render `RequestNewRiskOperationIntent`.
`AcceptanceIntentPending` and `RevocationIntentPending` render only their
already-authorized state-valid mutation. Live acceptance and effective
revocation render their current state and `NoFurtherAction`; another
operation must first establish a closed mutation-permitted state.
`RequestNewRiskOperationIntent` is not a free-form key request: it accepts the
prior safe object alias and exact proposed mutation digest, rechecks the
closed state, and then executes the same controller-issued-key machinery as
`IssueRiskOperationIntent`. The same key with different request bytes is
`risk-operation-replay-conflict`. That conflict first invokes
`ReadRiskRecoveryState` and follows only the action owned by the returned
variant. A lost response or crash after durable admission therefore cannot
create a second live acceptance or a second revocation.

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
| `risk-operation-replay-conflict` | `RecoveryRead.ReadRiskRecoveryState` by `OriginalAttemptPeerOrProtectedOperator`, then only the exact action owned by the returned `RiskRecoveryStatus`; only `ClosedMutationPermitted` may render `Operator.RequestNewRiskOperationIntent` by `ProtectedOperator` |
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
| `AuditSinkReservation` | Sink-side durable capacity edge keyed by `AttemptIdentity`, with a monotonic generation that, once appendable, is authorized for exactly one event id and digest. It is ineligible from creation until an append tombstone exists or the controller proves that authoritative acceptance is permanently impossible. |
| `AuditConversionIntent`, `AuditSinkInvalidationProof`, and `AuditSinkRebindProof` | Audit floor and ineligible while the named audit conversion is pending. The records are keyed by `AttemptIdentity` and the old reservation generation. The replacement-activation transaction compacts their exact digests into an immutable `AuditConversionTombstone`; only then do their protected bytes become eligible round input. |
| `AuditConversionTombstone` | Immutable permanent controller audit floor. It binds the attempt identity, old and replacement reservation generations, replacement refusal event id and digest, and the intent, invalidation-proof, and rebind-proof digests. It contains no proof or event bytes. |
| `MigrationAuditRepairIntent`, migration `AuditSinkInvalidationProof`, and `MigrationAuditSinkRebindProof` | Audit floor and ineligible while migration-specific no-append repair is pending. They preserve the original migration success result, quarantined capacity-switch effect, outbox event id and digest, and `MigrationExecutionReserve` binding. |
| `MigrationAuditRepairTombstone` | Immutable permanent controller audit floor binding the migration attempt alias, old and replacement reservation generations, unchanged success event alias and digest, and repair intent, invalidation-proof, and rebind-proof digests. It contains no proof, event, result, or authority-effect bytes. |
| `AuditAppendTombstone` | Permanent append-sink idempotency floor for the sink namespace. It contains only audit event id and digest plus the original acknowledgement and has no event payload bytes. Raw sink event bytes remain under the sink's bounded rotation. |
| `MigrationPreflightSignal::ReplayConflict` | D17 round input in a stricter diagnostic subclass: exactly 256 reusable detailed slots in the active 15-minute window, forced compaction at rotation or generation cutover, and a hard 30-minute TTL even when compaction fails. It is never an audit floor or migration blocker. |
| `MigrationPreflightSignal::AggregateOverflow` and `MigrationPreflightSignalWindowSummary` | D17 round input in the same stricter diagnostic subclass: one reusable overflow slot in the active window and a controller-wide ring of exactly 96 compacted summary slots. A summary has a hard 24-hour TTL; rotation and generation cutover reuse slots and never wait for export. |
| `MigrationPreflightTelemetryHealth` | D17 current diagnostic state, not round history: exactly one current and one shadow slot, overwritten atomically on transition and reused across rotation and generation cutover. Protected audit records may retain exact transition generation under the ordinary audit period, but metric labels may not. |
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
conflicting request digest. Except for the migration preflight exception
below, that request is a distinct accepted and audited attempt with
`AttemptIdentity::Conflict`; it never reuses the base attempt's capacity,
journal, reservation, proof, event, result, marker, tombstone, worker, audit
record, or status. A same-key, different-request
`MigrateRetentionCapacity` is instead classified before accepted-attempt
registration. Its conflict id is safe correlation only and is never a durable
`AttemptIdentity`, accepted attempt, or charge against migration execution
capacity. The controller namespace is stable across restart and reviewed
storage migration. Permanent indexes address the base and every admitted
conflict tombstone by `AttemptIdentity` across journal compaction,
replay-payload eviction, and restart; eviction never makes an id reusable.

`AttemptIdentity` is the mandatory key for `AcceptancePrepare`,
`AcceptedAttemptJournal`, `IdempotencyReplayResult`, `AuditOutboxRow`,
`AuditSinkReservation`, accepted-journal and no-journal proofs,
`AttemptTombstone`, replay eviction markers, worker leases and recovery state,
audit-conversion records and tombstones, migration-audit-repair records and
tombstones, audit events, and `ReadProtectedAttemptStatus`. A schema that keys
any of those records by a
bare `ProtectedAttemptId` or allows a base and admitted conflict attempt to
share one record fails construction.

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
- `AuditAppendTombstone` is the append sink's permanent minimal deduplication
  authority. It is sufficient to return the original acknowledgement after
  raw sink event rotation and contains no protected request, response, or event
  payload bytes.

Before authoritative acceptance, the controller reserves one journal slot,
one outbox slot, one tombstone slot, the maximum two payload-eviction marker
slots, the bounded request and result budget, and the section 13 recovery
reserve. A closed operation selector reserves either one generic
audit-conversion tombstone plus the maximum bounded conversion intent and proof
bytes, or, only for `MigrateRetentionCapacity`, one
`MigrationAuditRepairTombstone` plus the maximum bounded migration-repair
intent and proof bytes. The two allocations are disjoint. A closed capacity
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
have versioned finite entry and byte maxima. If permanent controller or sink
tombstone capacity is exhausted, `replay-tombstone-store-full` or
`audit-append-tombstone-store-full` respectively refuses new acceptance;
tombstones are never evicted to make room. `MigrateRetentionCapacity` is the
only capacity escape. It requires a reviewed manifest and a closed reason of
`ReserveIntegrityRepair` or `VersionedBoundMigration`, copies every permanent
id and digest, recomputes every recovery reservation, verifies the complete
set, and atomically switches storage without changing the controller
namespace. General-store fullness alone does not authorize it. Resetting a
namespace or dropping an old key is forbidden. This is finite fail-closed
storage, not unbounded raw response retention.

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
`MigrateRetentionCapacity` is the sole exception: the same-key,
different-request check is preflight and returns
`retention-capacity-migration-replay-conflict` without accepted-attempt
registration or any permanent record allocation.

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
the matrix below. A state with several terminal values generates one variant
per value; it does not carry a separate `current_state` field. Each endpoint
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

Three state reads return a nested state-owned action rather than a second
independent `RecoveryNextAction`: `ReadAssignmentRecoveryState` returns
`AssignmentRecoveryStatus`, `ReadRiskRecoveryState` returns
`RiskRecoveryStatus`, and `ReadMigrationRecoveryStatus` returns
`MigrationRecoveryStatus`. Their generators join the exact tagged state to
the endpoint table, caller policy, reserve route where applicable, and any
fresh-evidence prerequisite. They reject an action owned by another state,
an action with a missing prerequisite, a terminal assignment that skips the
new orchestrator request, an active assignment that mints a capability, a
risk state that requests a new intent before
`ClosedMutationPermitted`, or a migration action routed through the wrong
reserve.

The following is the complete success-recovery schema input. `Stem<Outcome>`
means one distinct wire tag for every closed terminal outcome admitted for
that operation.

| Endpoint | Operation | Generated variant stem and exact owned safe fields | Exact next action |
| --- | --- | --- | --- |
| Orchestrator | `ProposeLifecycleStart` | `LifecycleStart<Outcome> { lifecycle_id, lifecycle_event_id }` | `RecoveryRead.ReadLifecycleRecoveryState` by `OriginalAttemptPeerOrProtectedOperator` |
| Orchestrator | `RequestPanelDispatch` | `PanelDispatch<Outcome> { lifecycle_id, dispatch_id, roster_manifest_digest }` | `RecoveryRead.ReadLifecycleRecoveryState` by `OriginalAttemptPeerOrProtectedOperator` |
| Orchestrator | `SubmitCandidateSnapshot` | `CandidateSnapshot<Outcome> { lifecycle_id, candidate_id, snapshot_digest }` | `NoFurtherAction` |
| Orchestrator | `SubmitLedgerSynthesisProposal` | `LedgerSynthesis<Outcome> { lifecycle_id, candidate_id, mapping_version, ledger_digest }` | `RecoveryRead.ReadLedgerRecoveryState` by `OriginalAttemptPeerOrProtectedOperator` |
| Orchestrator | `RequestImplementationAssignment` | `ImplementationAssignmentRequest<Outcome> { lifecycle_id, candidate_id, assignment_request_id }` | `RecoveryRead.ReadAssignmentRecoveryState` by `OriginalAttemptPeerOrProtectedOperator` |
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
| Operator | `IssueRiskOperationIntent` | `RiskOperationIntent<Outcome> { intent_id, candidate_id, intent_event_id }` | `RecoveryRead.ReadRiskRecoveryState` by `OriginalAttemptPeerOrProtectedOperator` |
| Operator | `RequestNewRiskOperationIntent` | `NewRiskOperationIntent<Outcome> { intent_id, candidate_id, intent_event_id }` | `RecoveryRead.ReadRiskRecoveryState` by `OriginalAttemptPeerOrProtectedOperator` |
| Operator | `AcceptMajorRisk` | `MajorRiskAcceptance<Outcome> { acceptance_id, candidate_id, issue_ids, acceptance_event_id }` | `NoFurtherAction` |
| Operator | `RevokeMajorRiskAcceptance` | `MajorRiskRevocation<Outcome> { acceptance_id, revocation_id, revocation_event_id }` | `NoFurtherAction` |
| Operator | `RevokeImplementationAssignment` | `AssignmentRevocation<Outcome> { presented_assignment_alias, revocation_event_id }` | `RecoveryRead.ReadAssignmentRecoveryState` by `OriginalAttemptPeerOrProtectedOperator` |
| Operator | `ResumeProtectedAttempt` | `ProtectedAttemptResume<Outcome> { target_attempt_identity, attempt_control_event_id }` | `AttemptStatus.ReadProtectedAttemptStatus` by `OriginalAttemptPeerOrProtectedOperator` |
| Operator | `FenceProtectedAttempt` | `ProtectedAttemptFence<Outcome> { target_attempt_identity, attempt_control_event_id }` | `AttemptStatus.ReadProtectedAttemptStatus` by `OriginalAttemptPeerOrProtectedOperator` |
| Operator | `ReadLifecycleStatus` | `OperatorLifecycleStatus<Outcome> { lifecycle_id, status_digest }` | `RecoveryRead.ReadLifecycleRecoveryState` by `OriginalAttemptPeerOrProtectedOperator` |
| Operator | `ReadRetentionRecoveryStatus` | `RetentionRecoveryStatus<Outcome> { capacity_generation, blocker_records, integrity_state, preflight_telemetry_health: MigrationPreflightTelemetryHealth }` | `RecoveryRead.ReadRetentionRecoveryState` by `OriginalAttemptPeerOrProtectedOperator` |
| Operator | `RunControllerRetentionCleanup` | `RetentionCleanup<Outcome> { capacity_generation, cleanup_event_id, eligible_bytes_reclaimed }` | `RecoveryRead.ReadRetentionRecoveryState` by `OriginalAttemptPeerOrProtectedOperator` |
| Operator | `MigrateRetentionCapacity` | `RetentionCapacityMigration<Outcome> { migration_attempt_identity, source_generation, destination_generation, migration_event_id }` | `RecoveryRead.ReadRetentionRecoveryState` by `OriginalAttemptPeerOrProtectedOperator` |
| Operator | `ReadMigrationRecoveryStatus` | `MigrationRecoveryStatusRead<Outcome> { status: MigrationRecoveryStatus }` | Exact action owned by the returned `MigrationRecoveryStatus` |
| Operator | `RepairMigrationSinkAppend` | `MigrationSinkRepair<Outcome> { migration_attempt_alias, repair_event_id, replacement_generation }` | `Operator.ReadMigrationRecoveryStatus` by `ProtectedOperator` |
| Operator | `CompleteMigrationAuditActivation` | `MigrationAuditActivation<Outcome> { migration_attempt_alias, activation_event_id }` | `Operator.ReadMigrationRecoveryStatus` by `ProtectedOperator` |
| Operator | `RepairMigrationControlReserve` | `MigrationControlReserveRepair<Outcome> { migration_attempt_alias, repaired_state_generation, integrity_event_id }` | `Operator.ReadMigrationRecoveryStatus` by `ProtectedOperator` |
| Assignment issuance | `IssueImplementationAssignment` | `ImplementationAssignmentIssued<Outcome> { presented_assignment_alias, assignment_event_id }` | `RecoveryRead.ReadAssignmentRecoveryState` by `OriginalAttemptPeerOrProtectedOperator` |
| Assignment completion | `CompleteImplementationAssignment` | `ImplementationAssignmentCompleted<Outcome> { presented_assignment_alias, completion_event_id }` | `NoFurtherAction` |
| Issue reader | `ResolveImplementationAssignment` | `ImplementationAssignmentResolved<Outcome> { presented_assignment_alias, assignment_summary_digest }` | `IssueReader.ReadImplementerIssueView` by `OriginalIssueReaderPeer` |
| Issue reader | `ReadImplementerIssueView` | `ImplementerIssueViewConsumed<Outcome> { presented_assignment_alias, consumed_use_ordinal }` | `RecoveryRead.ReadAssignmentRecoveryState` by `OriginalAttemptPeerOrProtectedOperator` |
| Issue reader | `ReadMergeAuthorityMajorIssueView` | `MergeAuthorityMajorIssueView<Outcome> { authority_alias, candidate_id, issue_id, view_digest }` | `IssueReader.ReadMergeAuthorityMajorIssueView` by `OriginalIssueReaderPeer` |
| Attempt status | `ReadProtectedAttemptStatus` | `ProtectedAttemptStatusRead<Outcome> { target_attempt_identity, status_digest }` | `AttemptStatus.ReadProtectedAttemptStatus` by `OriginalAttemptPeerOrProtectedOperator` |
| Recovery read | `ReadLifecycleRecoveryState` | `LifecycleRecoveryState<Outcome> { target_attempt_identity, lifecycle_id, state_digest }` | `NoFurtherAction` |
| Recovery read | `ReadLedgerRecoveryState` | `LedgerRecoveryState<Outcome> { target_attempt_identity, mapping_version, ledger_digest }` | `NoFurtherAction` |
| Recovery read | `ReadAssignmentRecoveryState` | `AssignmentRecoveryState<Outcome> { target_attempt_identity, status: AssignmentRecoveryStatus }` | Exact action owned by the returned `AssignmentRecoveryStatus` |
| Recovery read | `ReadRiskRecoveryState` | `RiskRecoveryState<Outcome> { target_attempt_identity, status: RiskRecoveryStatus }` | Exact action owned by the returned `RiskRecoveryStatus` |
| Recovery read | `ReadArtifactRecoveryState` | `ArtifactRecoveryState<Outcome> { target_attempt_identity, artifact_ids, artifact_digests }` | `NoFurtherAction` |
| Recovery read | `ReadRetentionRecoveryState` | `RetentionRecoveryState<Outcome> { target_attempt_identity, capacity_generation, blocker_records, integrity_state, preflight_telemetry_health: MigrationPreflightTelemetryHealth }` | `NoFurtherAction` |
| Recovery read | `ReadPublicationRecoveryState` | `PublicationRecoveryState<Outcome> { target_attempt_identity, lifecycle_id, candidate_id, state_digest }` | `NoFurtherAction` |
| Recovery read | `ReadOriginalRefusalRecoveryState` | `OriginalRefusalRecoveryState<Outcome> { target_attempt_identity, refusal_product: ExactOperationRefusalProduct, remedy_plan: TypedRemedyPlan }` | `NoFurtherAction` |
| Publisher | `ConsumePublicationManifest` | `PublicationManifestConsumed<Outcome> { lifecycle_id, candidate_id, publication_manifest_id }` | `RecoveryRead.ReadPublicationRecoveryState` by `OriginalAttemptPeerOrProtectedOperator` |
| Publisher | `RecordTrustedMergeCompletion` | `TrustedMergeCompletionRecorded<Outcome> { lifecycle_id, candidate_id, merge_event_id }` | `NoFurtherAction` |
| Publisher | `ReadPublicationStatus` | `PublicationStatusRead<Outcome> { lifecycle_id, candidate_id, status_digest }` | `RecoveryRead.ReadPublicationRecoveryState` by `OriginalAttemptPeerOrProtectedOperator` |

The generator rejects a row whose owned field can be absent for one of its
outcomes; that outcome requires another variant with its own exact product.
The generated matcher is total over every accepted
`(EndpointOperation, TerminalOutcome::{Success, Refusal})`. Migration
preflight refusals have no accepted attempt and therefore do not appear in
this recovery type. Adding an endpoint operation, closed success outcome, or
terminal refusal without exactly one generated variant is a schema and
coverage failure. Duplicate rows and wildcard matches also fail generation.
The projection never returns protected text or only opaque digests plus a
generic remedy.

Round-input cleanup runs through the controller's single cleanup entrypoint on
schedule and before admitting new bytes. Full attempt request and response
bytes become eligible only after the attempt is terminal, its sink
acknowledgement is persisted, and its tombstone is durable. A pending
`AcceptedAttemptJournal` or `AuditOutboxRow` is ineligible at every age. The
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
generation. For `MigrateRetentionCapacity`, the same definite-no-append observation
selects `PendingMigrationAuditRepair` instead. That blocker is internally
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
For `PendingMigrationAuditRepair`, that
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
combined independently. `PendingMigrationAuditRepair` has the same exact
nested-product rule and can render only its two protected operator operations.
If
the controller cannot prove that every blocker has the exact generated
reservation, it enters `RecoveryReserveIntegrityCorrupt`, stops normal
admission, and permits only authenticated status, already-reserved recovery,
and `MigrateRetentionCapacity`. It must not guess, reclaim a reservation, or
continue on warning.

The capacity system exposes six disjoint partitions. Items 1 through 4 are
generation-owned, item 5 is controller-namespace-wide and fixed-cardinality,
and item 6 is transient:

1. `MigrationExecutionReserve` is the complete maximum serialized allocation
   for exactly one executable, non-conflict `MigrateRetentionCapacity`
   logical operation. It includes its prepare, accepted journal, migration
   work and pause state, request and replay result, outbox, sink reservation
   and raw event, acknowledgement, controller and sink tombstones, both
   payload-eviction markers, and the maximum migration audit-repair intent,
   invalidation proof, rebind proof, and repair tombstone. It contains no
   generic refusal-conversion allocation. It also seals two disjoint sibling
   reserves. `MigrationControlReserve` is sufficient for the schema maxima of
   every resume, fence, sink-repair, and audit-activation operation that the
   one active migration can require. `MigrationIntegrityReserve` is sufficient
   for every `ReadMigrationRecoveryStatus` and
   `RepairMigrationControlReserve` operation, including a complete verified
   replacement of the control reserve. Neither reserve can address or borrow
   the other. Both are permanent-record capacity, not estimates or transient
   lanes.
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
5. `MigrationPreflightTelemetryHealthReserve` is a separate fixed-cardinality
   two-slot current-and-shadow allocation for the controller-wide
   `MigrationPreflightTelemetryHealth` state. It is reused across every window,
   rotation, and capacity-generation cutover. Signal, aggregate, exporter, and
   migration operations cannot allocate another health record.
6. The transient emergency migration lane holds only preflight ownership and
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
`RecoveryRead.ReadRiskRecoveryState` through the original attempt peer or a
protected operator. Only its `ClosedMutationPermitted` state renders
`Operator.RequestNewRiskOperationIntent`; pending intent, live acceptance,
effective revocation, and closed-forbidden states render only their exact
state-valid action or `NoFurtherAction`. No risk variant,
controller-issued-key operation, or unknown class can render a universal
fresh-caller-key retry.

Every active-migration control operation uses a controller-issued state-bound
identity:

```
MigrationControlOperation =
  ReadMigrationRecoveryStatus
  | ResumeProtectedAttempt
  | FenceProtectedAttempt
  | RepairMigrationSinkAppend
  | CompleteMigrationAuditActivation
  | RepairMigrationControlReserve

MigrationControlIdentity =
  digest(
    "d2b:panel:migration-control:v1",
    MigrationAttemptIdentity,
    MigrationControlOperation,
    migration_state_generation
  )
```

Caller keys and request digests are excluded. A fresh caller key therefore
selects the same identity and cannot allocate another record. The sealed
reserves contain exactly one slot for every operation and migration-state
generation that the generated state graph requires. An authenticated
byte-identical or concurrent duplicate coalesces on that slot and returns its
one result. Changed bytes return the closed
`migration-control-replay-conflict` product from the same slot; they do not
create an accepted conflict attempt, another identity, another audit record,
or another metric series. The slot retains the canonical request digest and
result, while the conflicting digest and closed field codes are compared and
rendered without durable per-conflict allocation. A changed operation or
state generation selects only its already sealed distinct slot. The generator
rejects caller-selected control identity, request-digest identity, fresh-key
allocation, an unsealed operation/state pair, or a conflict path that charges
`AcceptedConflictReserve`.

`MigrationRecoveryStatus` is a strict tagged type. `ReadMigrationRecoveryStatus`
returns the selected variant directly, not a state digest followed by another
generic read. Each row below is one wire variant; its safe fields, exact
state-valid endpoint operation, caller class, eligibility, and reserve route
are owned by that tag and cannot be substituted independently.

| `MigrationRecoveryStatus` variant | Exact safe fields | Exact operation, caller, eligibility, and reserve |
| --- | --- | --- |
| `AcceptedUnclaimedPrepared` | migration attempt alias, state generation, reservation alias and generation, deadline | `Operator.ResumeProtectedAttempt` by `ProtectedOperator`, immediately, through `MigrationControlReserve` |
| `AcceptedUnclaimedAcceptedBound` | migration attempt alias, state generation, reservation alias and generation, deadline | `Operator.ResumeProtectedAttempt` by `ProtectedOperator`, immediately, through `MigrationControlReserve` |
| `ProcessingLeaseLive` | migration attempt alias, state generation, owner epoch alias, lease until, deadline | `Operator.ReadMigrationRecoveryStatus` by `ProtectedOperator`, observation only, through `MigrationIntegrityReserve` |
| `ProcessingResumeEligible` | migration attempt alias, state generation, fenced owner epoch alias, lease expiry, deadline | `Operator.ResumeProtectedAttempt` by `ProtectedOperator`, immediately, through `MigrationControlReserve` |
| `ProcessingFenceEligible` | migration attempt alias, state generation, owner epoch alias, lease expiry, closed fence reason, deadline | `Operator.FenceProtectedAttempt` by `ProtectedOperator`, immediately, through `MigrationControlReserve` |
| `PausedRepairRequired` | migration attempt alias, state generation, owner epoch alias, safe pause reason, pause deadline | `Operator.ReadMigrationRecoveryStatus` by `ProtectedOperator`, observation until the named condition clears, through `MigrationIntegrityReserve` |
| `PausedResumeEligible` | migration attempt alias, state generation, fenced owner epoch alias, cleared pause reason, pause deadline | `Operator.ResumeProtectedAttempt` by `ProtectedOperator`, immediately, through `MigrationControlReserve` |
| `PausedFenceEligible` | migration attempt alias, state generation, owner epoch alias, safe pause reason, closed fence reason, pause deadline | `Operator.FenceProtectedAttempt` by `ProtectedOperator`, immediately, through `MigrationControlReserve` |
| `QuarantinedMigrationAudit` | migration attempt alias, state generation, owner epoch alias, reservation generation, success event alias and digest, deadline | `Operator.ResumeProtectedAttempt` by `ProtectedOperator`, immediately, through `MigrationControlReserve` |
| `MigrationAuditRepairIntentRecorded` | migration attempt alias, state generation, repair id, old reservation generation, success event alias and digest, deadline | `Operator.RepairMigrationSinkAppend` by `ProtectedOperator`, immediately, through `MigrationControlReserve` |
| `MigrationAuditRepairOldGenerationInvalidationPending` | migration attempt alias, state generation, repair id, old reservation generation, success event alias and digest, deadline | `Operator.RepairMigrationSinkAppend` by `ProtectedOperator`, immediately, through `MigrationControlReserve` |
| `MigrationAuditRepairOldGenerationInvalidatedRebindPending` | migration attempt alias, state generation, repair id, invalidated reservation generation, invalidation-proof digest, success event alias and digest, deadline | `Operator.RepairMigrationSinkAppend` by `ProtectedOperator`, immediately, through `MigrationControlReserve` |
| `MigrationAuditRepairSuccessGenerationBoundReplacementPending` | migration attempt alias, state generation, repair id, invalidated and replacement reservation generations, invalidation- and rebind-proof digests, success event alias and digest, deadline | `Operator.RepairMigrationSinkAppend` by `ProtectedOperator`, immediately, through `MigrationControlReserve` |
| `MigrationAuditRepairReplacementTupleInstalled` | migration attempt alias, state generation, repair id, repair-tombstone digest, replacement reservation generation, success event alias and digest, deadline | `Operator.RepairMigrationSinkAppend` by `ProtectedOperator`, immediately, through `MigrationControlReserve` |
| `MigrationSinkAcknowledgementPending` | migration attempt alias, state generation, appendable reservation generation, success event alias and digest, deadline | `Operator.RepairMigrationSinkAppend` by `ProtectedOperator`, immediately, through `MigrationControlReserve` |
| `MigrationActivationPending` | migration attempt alias, state generation, appendable reservation generation, audit-acknowledgement digest, deadline | `Operator.CompleteMigrationAuditActivation` by `ProtectedOperator`, immediately, through `MigrationControlReserve` |
| `ControlReserveIntegrityCorrupt` | migration attempt alias, state generation, underlying migration-state code, corrupt slot field codes, expected reserve digest | `Operator.RepairMigrationControlReserve` by `ProtectedOperator`, immediately, through `MigrationIntegrityReserve` |
| `Completed` | migration attempt alias, state generation, source generation, destination generation, migration event id and tombstone digest | `Operator.ReadMigrationRecoveryStatus` by `ProtectedOperator`, terminal observation with `NoFurtherAction`, through `MigrationIntegrityReserve` |

The status constructor partitions `Processing` and `Paused` by controller-owned
lease, repair, and fencing eligibility so no variant offers two incompatible
operations. A non-authoritative `AcceptancePreparePending` is not an active
migration and remains visible only through ordinary protected-attempt status.
Preflight refusals create no migration state.

`ReadMigrationRecoveryStatus` always uses `MigrationIntegrityReserve`.
`ResumeProtectedAttempt`, `FenceProtectedAttempt`,
`RepairMigrationSinkAppend`, and `CompleteMigrationAuditActivation` use only
`MigrationControlReserve`. `RepairMigrationControlReserve` uses only
`MigrationIntegrityReserve`. That integrity operation reads the canonical
migration journal and generated state graph, constructs a complete replacement
control reserve in the integrity reserve's pre-sealed repair allocation,
verifies every required operation/state slot and digest, and atomically
installs it while quarantining the corrupt reserve. It neither reads from nor
allocates in the corrupt reserve, does not advance migration state, and cannot
change the retained success tuple or capacity-switch effect. Failure leaves
the old reserve quarantined and the integrity reserve reusable for the same
repair identity. Status remains readable while control integrity is corrupt,
so repair never depends on the reserve it repairs.

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
  telemetry_health: MigrationPreflightTelemetryHealth
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

The separately reserved health state is:

```
MigrationPreflightTelemetryHealth =
  Healthy
  | DetailedDegraded
  | AggregateDegraded
  | ExporterDegraded
  | HealthUnavailable
```

It occupies the fixed current-and-shadow
`MigrationPreflightTelemetryHealthReserve`, not the signal, execution,
control, integrity, status, or accepted-conflict reserves. For simultaneous
failures, the deterministic observed precedence is `HealthUnavailable`,
`AggregateDegraded`, `DetailedDegraded`, `ExporterDegraded`, then `Healthy`.
A failed detailed write attempts the aggregate and records
`DetailedDegraded`; a failed aggregate or compaction write records
`AggregateDegraded`; a failed exporter write records `ExporterDegraded`.
Failure to read, verify, or update the health state yields
`HealthUnavailable` by observation even if that value cannot itself be
persisted. Health observation is therefore not circular and never affects
migration authorization, eligibility, execution, refusal selection, or
cutover.
`ReadRetentionRecoveryStatus` and `ReadRetentionRecoveryState` carry that
observation directly from the health reserve. Failure to obtain it substitutes
only `HealthUnavailable`; it cannot fail either status read or alter blocker,
integrity, or migration fields.

Metrics are
`migration_preflight_replay_conflict_signals_total`,
`migration_preflight_signal_overflow_buckets_total`, and
`migration_preflight_signal_reserve_used_ratio`, plus
`migration_preflight_telemetry_health`. Their exact label allowlist is:

```
MigrationPreflightMetricLabel =
  operation_class: MigrationReplayConflict
  | signal_outcome: Detailed | Aggregate | Unrecorded
  | reserve_role: Detailed | AggregateSummary | TelemetryHealth
  | health_state:
      Healthy
      | DetailedDegraded
      | AggregateDegraded
      | ExporterDegraded
      | HealthUnavailable
```

Each metric declares only the applicable subset of that closed list.
Capacity generation, signal id, request or key digest, attempt identity, peer,
path, and every other unbounded value are forbidden labels. Exact capacity
generation is absent from the refusal, detailed signal, aggregate, summary,
health state, log, and exporter product. It remains available only in
protected status and audit fields.
The two counters advance only when a detailed identity or overflow bucket first
becomes durable in its window; exact retries do not increment them.
Exporter failure cannot change durable counters and instead degrades health
when possible.

The original replay-conflict refusal is committed to the response path before
the controller attempts the detailed signal, aggregate, summary, health, or
exporter writes. Failure at any or all of those writes returns the same exact
refusal without delay. No telemetry failure consumes, delays, or refuses
`MigrationExecutionReserve`, `MigrationControlReserve`,
`MigrationIntegrityReserve`, or the transient migration lane.

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
route, copies every permanent replay and audit index, verifies the complete
copy, and atomically changes the capacity generation. An execution, storage,
or verification fault does not terminally consume the attempt or its one
emergency route. It durably enters `Paused` with a closed safe reason, bounded
deadline, owner epoch alias, and exact repair-and-resume action.
`ReadProtectedAttemptStatus` reports that state. Automatic recovery resumes by
the deadline when the condition clears; otherwise lease-expiry takeover or the
protected operator's audited `ResumeProtectedAttempt` or
`FenceProtectedAttempt` continues the same accepted attempt. Completion
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
- completion refusals carry exactly one
  `AssignmentCompletionRefusalProduct` with `CompletionEvidenceAlias`, and
  append-authorization refusals carry exactly one
  `AuditAppendAuthorizationRefusalProduct` with `SinkNamespaceAlias`,
  `SinkReservationAlias`, `AppendAuthorizationAlias`, and `AuditEventAlias`;
  local errors, the catalog, tombstones, recovery and retention projections,
  logs, audit, status, `Debug`, and fixtures cannot add
  `ControllerNamespaceAlias`, a raw identity, namespace, handle, path, or
  deployment id;
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
receives the ordinary exactly-one refusal audit. The sole stricter exception
is `MigrateRetentionCapacity`: its structural, authorization, eligibility,
same-key conflict, raw-capacity, execution-reserve, and transient-lane checks
are preflight by the re-entrant migration contract above. Their refusals are
not accepted attempts and consume neither the execution reserve nor emergency
lane.

Every authoritative event has:

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

The linear recoverable state is:

```
AcceptancePreparePending
-> AcceptedUnclaimed { sink_binding = Prepared }
-> AcceptedUnclaimed { sink_binding = AcceptedBound }
-> Processing { worker_epoch, lease_until }
-> Paused { worker_epoch, lease_until, safe_pause_reason, pause_deadline }
-> Processing { worker_epoch, lease_until }
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
definite-no-append result for a generic operation, one atomic compare-and-swap
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

`MigrateRetentionCapacity` is excluded from that conversion by operation
type. Its authenticated definite-no-append result atomically starts this
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
tombstone, and marks the attempt `Completed`. Migration instead requires
`CompleteMigrationAuditActivation` through `MigrationControlReserve`. No
effect, use, or response is visible before the applicable final transaction.
Thus a successful stateful read's use activation, terminal journal, replay
availability, and tombstone still commit atomically; its identical retry
cannot consume again.

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
  | Paused {
      attempt_identity, owner_epoch_alias, lease_until, safe_pause_reason,
      deadline, action: RepairPauseReasonThenResume
    }
  | QuarantinedPendingAudit {
      attempt_identity, owner_epoch_alias, lease_until,
      reservation_generation, authorized_event_id, authorized_event_digest,
      deadline, action: BindAndReplayAuthorizedAuditEvent
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
  | MigrationAuditRepairIntentRecorded {
      migration_attempt_alias, repair_id, old_reservation_generation,
      success_event_alias, success_event_digest, deadline,
      action: Operator.RepairMigrationSinkAppend by ProtectedOperator
    }
  | MigrationAuditRepairOldGenerationInvalidationPending {
      migration_attempt_alias, repair_id, old_reservation_generation,
      success_event_alias, success_event_digest, deadline,
      action: Operator.RepairMigrationSinkAppend by ProtectedOperator
    }
  | MigrationAuditRepairOldGenerationInvalidatedRebindPending {
      migration_attempt_alias, repair_id, invalidated_reservation_generation,
      invalidation_proof_digest, success_event_alias, success_event_digest,
      deadline,
      action: Operator.RepairMigrationSinkAppend by ProtectedOperator
    }
  | MigrationAuditRepairSuccessGenerationBoundReplacementPending {
      migration_attempt_alias, repair_id, invalidated_reservation_generation,
      invalidation_proof_digest, replacement_reservation_generation,
      rebind_proof_digest, success_event_alias, success_event_digest,
      deadline,
      action: Operator.RepairMigrationSinkAppend by ProtectedOperator
    }
  | MigrationAuditRepairReplacementTupleInstalled {
      migration_attempt_alias, repair_id, repair_tombstone_digest,
      replacement_reservation_generation, success_event_alias,
      success_event_digest, deadline,
      action: Operator.RepairMigrationSinkAppend by ProtectedOperator
    }
  | OrdinarySinkAcknowledgementPending {
      attempt_identity, appendable_reservation_generation,
      authorized_event_id, authorized_event_digest, deadline,
      action: QueryOrReplayAuthorizedAuditAppend
    }
  | OrdinaryActivationPending {
      attempt_identity, appendable_reservation_generation,
      audit_acknowledgement_digest, deadline,
      action: CompleteAtomicActivation
    }
  | MigrationSinkAcknowledgementPending {
      migration_attempt_alias, state_generation,
      appendable_reservation_generation, success_event_alias,
      success_event_digest, deadline,
      action: Operator.RepairMigrationSinkAppend by ProtectedOperator
              through MigrationControlReserve
    }
  | MigrationActivationPending {
      migration_attempt_alias, state_generation,
      appendable_reservation_generation, audit_acknowledgement_digest,
      deadline,
      action: Operator.CompleteMigrationAuditActivation by ProtectedOperator
              through MigrationControlReserve
    }
```

There are no optional status fields and no independent state or action
discriminants. Strict decoding denies an action, lease, pause reason, proof,
generation, or event field owned by another variant. Every deadline is
bounded by a versioned maximum. `InvalidatedReservationGeneration` cannot
convert to `AppendableReservationGeneration`; after invalidation, every
variant and action owns only the proof-bound replacement generation. It is
therefore impossible for a status action to replay an invalidated generation.
A pause auto-resumes when its closed condition clears, and at its deadline
either recovery takes over the expired lease or a protected operator uses
`ResumeProtectedAttempt` or `FenceProtectedAttempt`. Both are ordinary
accepted and audited protected operations, require the narrow operator
endpoint, bind the target `AttemptIdentity`, and cannot invent a result,
activate an effect, cancel an accepted attempt, or bypass sink audit.
The ordinary and migration sink-acknowledgement and activation tags are
different wire variants. Ordinary variants own only generic append replay and
atomic activation actions and cannot address a migration reserve. Migration
variants own only `RepairMigrationSinkAppend` and
`CompleteMigrationAuditActivation` through `MigrationControlReserve`.
Operation type is never consulted to reinterpret a generic status tag.

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
   operation and may not terminally consume the emergency lane;
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
    recovery performs its generic atomic transaction, while migration
    activation is reachable only through
    `CompleteMigrationAuditActivation` and `MigrationControlReserve`;
11. after completion but before delivery, identical retry returns the stored
   response and original acknowledgement; and
12. during or after payload eviction, the marker protocol above determines
   availability and replay returns operation-specific safe recovery without
   execution.

Startup and scheduled recovery scan every nonterminal prepare, journal,
reservation binding, outbox row, conversion intent, invalidation proof, and
rebind proof before the controller accepts normal work. Each recovery
transition is an epoch-and-generation compare-and-swap and consumes the
record's recovery reservation. A live paused worker is not an orphan merely
because recovery is running, but its bounded deadline guarantees takeover or
resume.

Timeout, disconnect, or lost acknowledgement is never proof that the sink did
not append. The controller may convert an authorized pending event only after an
authenticated definite-no-append result for its stable `AuditEventId`, digest,
and reservation generation. The result is accepted only while the journal is
in the exact matching ordinary or migration sink-acknowledgement state. For a generic
operation, conversion is itself recoverable:

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
the one stored terminal state selects exactly one of completed, revoked,
expired, or exhausted; a remaining lifecycle, candidate, or mapping mismatch
is a binding mismatch; and only an otherwise current active assignment can
reach cross-scope when the caller-supplied issue ids are not a subset of its
exact set. Cross-scope evaluation does not resolve which other assignment, if
any, owns an issue.

Completion refusal precedence is also fixed. After protected evidence
authentication, the controller first checks the exact originating principal
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

The refusal catalog and core plans are closed:

| Typed refusal | Causing ids | Ordered core `RemedyAction` plan |
| --- | --- | --- |
| `protected-authority-unavailable` | authority deployment, producer | `StartOrConfigureProtectedAuthority`, then `RetryProtectedPreflight` |
| `unauthorized-protected-operation` | endpoint, operation, peer alias | `UseAuthorizedEndpointIdentity`, then `RetryProtectedOperation` |
| `protected-operation-absent-from-endpoint` | endpoint, operation | `UseOperationOwningEndpoint` |
| `protected-operation-replay-conflict` | base and conflict attempt identities, accepted non-risk and non-migration endpoint operation, idempotency-key digest, request digests | `RetrySameProtectedOperationWithFreshIdempotencyKey` |
| `accepted-conflict-budget-exhausted` | exact `AcceptedConflictOperationClass`, including the risk target attempt identity when applicable, capacity generation, used and maximum conflict entries and bytes | `RiskOperation`: `RecoveryRead.ReadRiskRecoveryState` for the carried target attempt by `OriginalAttemptPeerOrProtectedOperator`, then only the exact action owned by the returned state, with `RequestNewRiskOperationIntent` limited to `ClosedMutationPermitted`; `CallerKeyOperation`: retry the named endpoint operation through its original authorized caller with a fresh caller key |
| `protected-operation-invalid-state` | lifecycle, operation, current state | `ReadLifecycleStatus`, then `UseStatePermittedOperation` |
| `accepted-attempt-crash-before-state` | endpoint, operation, attempt identity | `ReadProtectedAttemptStatus`, then `FollowOperationSpecificProtectedAttemptRecovery` |
| `audit-event-flush-failed` | endpoint, non-migration operation, attempt identity | `RestoreProtectedAuditSink`, then `ReadProtectedAttemptStatus`, then `FollowOperationSpecificProtectedAttemptRecovery` |
| `audit-event-id-conflict` | attempt identity, audit event alias, expected and actual event digests | `RepairAppendSinkIntegrity`, then `ReplayPendingAuditAppend` |
| `acceptance-prepare-digest-conflict` | sink namespace alias, attempt identity, reservation alias and generation, authoritative and presented prepare digests | `DiscardConflictingAcceptancePrepare`, then `ReplayAuthoritativeAcceptancePrepare` |
| `audit-append-authorization-invalid` | exact `AuditAppendAuthorizationRefusalProduct::Invalid` | `RequestFreshControllerAuditAppendAuthorization`, then `ReplayPendingAuditAppend` |
| `audit-append-authorization-cross-attempt` | exact `AuditAppendAuthorizationRefusalProduct::CrossAttempt` | `UseAttemptBoundAuditAppendAuthorization`, then `ReplayPendingAuditAppend` |
| `audit-sink-generation-stale` | exact `AuditAppendAuthorizationRefusalProduct::StaleGeneration` | `StopStaleAttemptWorker`, then `AttemptStatus.ReadProtectedAttemptStatus` by `OriginalAttemptPeerOrProtectedOperator` |
| `audit-append-authorization-unbound` | exact `AuditAppendAuthorizationRefusalProduct::Unbound` | `BindCurrentReservationGenerationToAuditEvent`, then `RequestFreshControllerAuditAppendAuthorization`, then `ReplayPendingAuditAppend` |
| `audit-append-authorization-event-mismatch` | exact `AuditAppendAuthorizationRefusalProduct::EventMismatch` | `RestoreCanonicalOutboxEvent`, then `RequestFreshControllerAuditAppendAuthorization`, then `ReplayPendingAuditAppend` |
| `audit-conversion-source-state-invalid` | safe attempt identity, presented state code, expected `OrdinarySinkAcknowledgementPending` code, old generation and event aliases and digests | `AttemptStatus.ReadProtectedAttemptStatus` by `OriginalAttemptPeerOrProtectedOperator`, then retry only the returned state-valid operation |
| `migration-audit-repair-tuple-mismatch` | migration attempt alias, repair state code, old or replacement generation, success event aliases and digests, closed field code | `Operator.ReadMigrationRecoveryStatus` by `ProtectedOperator`, then invoke only its returned state-valid operation; only a migration repair variant may render `Operator.RepairMigrationSinkAppend` |
| `migration-audit-repair-invalid-state` | migration attempt alias, current migration state code, requested repair operation | `Operator.ReadMigrationRecoveryStatus` by `ProtectedOperator` |
| `migration-control-replay-conflict` | controller-issued `MigrationControlIdentity`, migration attempt alias, control operation, state generation, canonical and conflicting request digests and closed conflicting field codes | `Operator.ReadMigrationRecoveryStatus` by `ProtectedOperator` through `MigrationIntegrityReserve`, then invoke only its returned state-valid operation; no caller key creates another identity |
| `migration-control-reserve-integrity-corrupt` | migration attempt alias, state generation, source generation, closed reserve field codes and expected reserve digest | `Operator.ReadMigrationRecoveryStatus` by `ProtectedOperator` through `MigrationIntegrityReserve`; only a returned `ControlReserveIntegrityCorrupt` renders `Operator.RepairMigrationControlReserve` by `ProtectedOperator` through `MigrationIntegrityReserve`, otherwise follow the returned state-valid action; normal admission and other migration control remain stopped until repair verifies and atomically installs the replacement reserve |
| `idempotency-result-evicted` | attempt identity, endpoint, operation, closed outcome, safe result ids, response and event digests | `ReturnOperationSpecificProtectedAttemptRecovery` |
| `protected-attempt-status-cross-peer` | attempt identity, presented peer safe alias | `UseOriginalAttemptPeerOrProtectedOperator` |
| `protected-recovery-read-cross-peer` | attempt identity, recovery-read operation, presented peer safe alias | `UseOriginalAttemptPeerOrProtectedOperator` |
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
| `implementation-assignment-evidence-conflict` | evidence digest and conflicting closed field codes | `RecoveryRead.ReadAssignmentRecoveryState` by `OriginalAttemptPeerOrProtectedOperator`, then follow only the returned `AssignmentRecoveryStatus`; a terminal state requires a new `Orchestrator.RequestImplementationAssignment` plus fresh protected issuance evidence |
| `implementation-assignment-self-asserted` | issuer endpoint, implementer run alias, supplied claim digest | `RequestTrustedImplementationDispatchOrResolverReceipt` |
| `implementation-assignment-completion-origin-mismatch` | exact `AssignmentCompletionRefusalProduct::OriginMismatch` | `RecoveryRead.ReadAssignmentRecoveryState` by `OriginalAttemptPeerOrProtectedOperator`; only `Active` renders `UseOriginatingAssignmentCompletionPrincipalAndEvidence`, then `RetryAssignmentCompletionWithFreshEvidence`; terminal variants render only their fresh-assignment plan |
| `implementation-assignment-completion-binding-mismatch` | exact `AssignmentCompletionRefusalProduct::BindingMismatch` | `RecoveryRead.ReadAssignmentRecoveryState` by `OriginalAttemptPeerOrProtectedOperator`; only `Active` renders `RequestFreshAssignmentBoundCompletionEvidence`, then `RetryAssignmentCompletion`; terminal variants render only their fresh-assignment plan |
| `implementation-assignment-completion-evidence-stale-or-expired` | exact `AssignmentCompletionRefusalProduct::StaleOrExpired` | `RecoveryRead.ReadAssignmentRecoveryState` by `OriginalAttemptPeerOrProtectedOperator`; only `Active` renders `RequestFreshAssignmentBoundCompletionEvidence`, then `RetryAssignmentCompletion`; terminal variants render only their fresh-assignment plan |
| `implementation-assignment-completion-evidence-replayed` | exact `AssignmentCompletionRefusalProduct::EvidenceReplay` | `RecoveryRead.ReadAssignmentRecoveryState` by `OriginalAttemptPeerOrProtectedOperator`, then follow only the exact returned `AssignmentRecoveryStatus` |
| `implementation-assignment-completion-evidence-conflict` | exact `AssignmentCompletionRefusalProduct::EvidenceConflict` | `RecoveryRead.ReadAssignmentRecoveryState` by `OriginalAttemptPeerOrProtectedOperator`, then follow only the returned `AssignmentRecoveryStatus`; conflict never proves capability loss |
| `implementation-assignment-revocation-unauthorized` | presented assignment alias and presented principal safe alias | `RecoveryRead.ReadAssignmentRecoveryState` by `OriginalAttemptPeerOrProtectedOperator`; only `Active` renders `UseProtectedOperatorEndpointIdentity`, then `RetryImplementationAssignmentRevocation`; terminal variants render no revocation retry |
| `implementation-assignment-replayed` | presented assignment alias and authenticated implementer peer or run safe alias | `RecoveryRead.ReadAssignmentRecoveryState` by `OriginalAttemptPeerOrProtectedOperator`; if `Active`, only the bound `OriginalIssueReaderPeer` may use the existing capability and no capability is issued |
| `implementation-assignment-completed` | presented assignment alias and completion event id | `RecoveryRead.ReadAssignmentRecoveryState` by `OriginalAttemptPeerOrProtectedOperator`, then the returned `Completed` variant requires `Orchestrator.RequestImplementationAssignment` plus fresh protected issuance evidence |
| `implementation-assignment-revoked` | presented assignment alias, revocation event id and reason code | `RecoveryRead.ReadAssignmentRecoveryState` by `OriginalAttemptPeerOrProtectedOperator`, then the returned `Revoked` variant requires `Orchestrator.RequestImplementationAssignment` plus fresh protected issuance evidence |
| `implementation-assignment-expired` | presented assignment alias and expiry | `RecoveryRead.ReadAssignmentRecoveryState` by `OriginalAttemptPeerOrProtectedOperator`, then the returned `Expired` variant requires `Orchestrator.RequestImplementationAssignment` plus fresh protected issuance evidence |
| `implementation-assignment-exhausted` | presented assignment alias, activated and maximum use counts | `RecoveryRead.ReadAssignmentRecoveryState` by `OriginalAttemptPeerOrProtectedOperator`, then the returned `Exhausted` variant requires `Orchestrator.RequestImplementationAssignment` plus fresh protected issuance evidence |
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
| `risk-operation-replay-conflict` | conflict attempt identity, exact risk operation, intent, acceptance or revocation safe id as applicable, idempotency-key and request digests | `RecoveryRead.ReadRiskRecoveryState` by `OriginalAttemptPeerOrProtectedOperator`, then only the action owned by its exact `RiskRecoveryStatus`; only `ClosedMutationPermitted` renders `Operator.RequestNewRiskOperationIntent` |
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
  controller-issued migration control identity, disjoint migration control
  and integrity reserves, direct tagged migration recovery status,
  migration-specific audit repair, bounded and expiring
  migration-preflight signals and summaries, fixed-cardinality telemetry
  health, low-cardinality metric labels, re-entrant capacity migration,
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
an endpoint operation lacks a refusal map or a terminal
`ProtectedAttemptRecovery` mapping, when a pending attempt state lacks its
status variant and exact action, when a migration state lacks its direct
status fields, operation, caller, eligibility, or reserve route, when any
recovery or remedy action lacks an exact eligible source-state join, when the
corpus is empty, or when a planted negative is accepted. At minimum, the
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
  protected-operator revocation, protected-attempt resume and fencing,
  protected-attempt status, retention-recovery status, cleanup, reviewed
  capacity migration, and migration-only direct status, sink repair, audit
  activation and control-reserve integrity repair, with cross-endpoint planted
  negatives proving that no assignment-issuance operation can derive a new
  capability from old evidence, that a generic issuer or resolver has no
  completion or revocation right, and that
  the orchestrator still has no approval, risk, mapping,
  assignment-lifecycle, attempt-control, recovery expansion, or retention
  mutation operation. Generation fails when any
  `ProtectedAttemptRecovery` action is not `NoFurtherAction` or does not name
  an operation in the endpoint table and one caller class authorized by that
  exact row;
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
- atomic single consumption of both protected assignment evidence variants;
  identical same-key and fresh-key reissuance returning one controller-private
  assignment; conflicting kind, issue set, candidate, mapping, run, use limit,
  lifecycle, file-ownership, origin or expiry reuse; originating evidence
  settled at audited activation; concurrent issuance, crash before and after
  activation, and proof that fresh keys cannot mint duplicates while fresh
  protected evidence can mint a genuinely new assignment;
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
  replay; exact current `AssignmentRecoveryStatus` after issuance replay,
  completion-evidence replay or conflict, protected-attempt replay, and
  replay-payload eviction, with every active case retaining the same
  capability and issuing no new one; `ReadImplementerIssueView` followed by a
  state read that returns the same active assignment when a use remains and
  `Exhausted` after the last activation; completed, revoked, expired, and
  exhausted states permitting a new assignment only after a new
  `Orchestrator.RequestImplementationAssignment` and fresh protected issuance
  evidence; operator revocation before abandoning a possibly lost active
  capability; absence of any alternate issuance or old-evidence eligibility
  surface; protected-operator revocation;
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
  transition selects only the terminal fresh-assignment plan and cannot retry
  completion or revocation, while only a still-`Active` state admits the
  applicable retry;
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
  issue and revocation, controller-issued idempotency key, identical retry,
  conflicting replay that first returns the exact current
  `RiskRecoveryStatus`, and response loss at every durable boundary;
  `AcceptanceIntentPending`, `RevocationIntentPending`, `AcceptanceLive`,
  `RevocationEffective`, `ClosedMutationPermitted`, and
  `ClosedMutationForbidden` with exact owned fields and actions; planted
  action substitutions proving only `ClosedMutationPermitted` can render
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
  or lease-expiry takeover; audited narrow `ResumeProtectedAttempt` and
  `FenceProtectedAttempt`; orphaned and expired work claimed once; a crash
  after acceptance and before the state transaction with no caller retry that
  recovers exactly one `accepted-attempt-crash-before-state` event except for
  resumable migration; every compare-and-swap and crash boundary through
  quarantined result, effect and outbox, sink fsync, acknowledgement
  persistence, ordinary or migration activation pending, atomic effect or
  assignment-use activation,
  response availability and completion; and pre-registration transport,
  parse, authentication and capacity failures that create no authoritative
  attempt or effect;
- exact one-event auditing of every accepted protected success and refusal;
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
  acknowledged success to refusal. Separate migration cases prove
  `MigrateRetentionCapacity` cannot enter generic conversion or
  `audit-event-flush-failed`; every definite-no-append crash boundary retains
  the same migration attempt, execution, control, and integrity reserves,
  quarantined capacity-switch effect, success result and event, rebinds only
  the sink generation, retries the unchanged success append, and activates
  through `CompleteMigrationAuditActivation`;
- `round-input-store-full` fixtures for every closed blocker:
  active lifecycle, unresolved lifecycle, retryable partial dispatch,
  unavailable partial dispatch, resumable abandoned capsule, unexpired
  approval receipt, unrecorded trusted merge completion, pending acceptance
  prepare, pending accepted attempt, pending outbox, and
  `PendingAuditConversion` and `PendingMigrationAuditRepair` at each of their
  five nested states. Each asserts its exact serialized blocker and blocker
  digest, recovery plan id, reservation alias and reservation digest, plus the
  configured bounds in the original refusal, then direct execution of that
  plan id without an unnecessary status read. Conversion fixtures prove the blocker
  key is exactly `AttemptIdentity` plus old reservation generation; intent,
  invalidation-proof, and rebind-proof bytes remain ineligible before
  replacement activation; replacement activation atomically creates the
  immutable conversion tombstone digests and makes those bytes eligible; and
  every crash resumes the exact state-specific action without replaying the
  invalidated generation. Migration repair fixtures prove that status uses
  `MigrationIntegrityReserve`, repair uses `MigrationControlReserve`, and both
  preserve the success tuple. From `MigrationActivationPending`,
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
  conversion, migration audit-repair, marker, tombstone, and bounded work
  record plus disjoint sibling `MigrationControlReserve` and
  `MigrationIntegrityReserve`; a separate bounded
  and duplicate-coalescing `ProtectedStatusReserve`; a separate bounded
  `AcceptedConflictReserve` for non-migration audited conflicts; a disjoint
  `MigrationPreflightSignalReserve` with its pre-sealed aggregate overflow; a
  fixed two-slot `MigrationPreflightTelemetryHealthReserve`; and the distinct
  transient execution lane. Exact-bound fixtures consume every migration
  record class at its serialized maximum, reject a generation whose exclusive
  reserve is one entry or byte short, and reject every attempt by another
  operation to allocate from that partition. Fixtures completely fill the normal,
  blocker-recovery, status, and accepted-conflict budgets, then issue
  arbitrarily repeated migration conflicts and status reads and prove that a
  valid non-conflict migration remains admissible from its untouched execution
  reserve. They also prove status attempts cannot borrow that reserve,
  migration conflicts create no accepted attempt or accepted-attempt permanent
  record, signal exhaustion coalesces into the bounded overflow without
  consuming or blocking execution, status budget exhaustion is isolated, and
  the active migration remains readable through its integrity reserve and
  resumable, fenceable, sink-repairable, and activation-completable through
  its control reserve after `ProtectedStatusReserve` is full. The complete
  generated `MigrationRecoveryStatus` variant table is exercised with exact
  safe fields, operation, caller, eligibility, and reserve route; every
  cross-state action substitution and control-versus-integrity misroute is
  refused. Corrupt control-reserve fixtures leave status readable, execute
  `RepairMigrationControlReserve` entirely from
  `MigrationIntegrityReserve`, verify every generated control slot, atomically
  install the replacement without advancing migration state, and then resume
  the original state-valid action. Unrelated status traffic cannot consume
  either reserve, and only the one accepted migration identity can retain the
  execution, control, and integrity allocations across pause, repair and
  resume. Every active control request derives
  `MigrationControlIdentity` from only migration attempt, operation, and state
  generation; byte-identical duplicates coalesce, changed bytes return the
  one closed conflict in that same slot, and arbitrarily many fresh caller
  keys and conflicting requests allocate no additional record or identity and
  cannot exhaust control capacity;
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
  retention blocker at any failure. `MigrationPreflightTelemetryHealth`
  covers `Healthy`, `DetailedDegraded`, `AggregateDegraded`,
  `ExporterDegraded`, and observed `HealthUnavailable` from its independent
  two-slot reserve. Health-read failure makes both retention status products
  report `HealthUnavailable` without failing the status read or changing
  migration execution. Protected identifiers are absent. Metric schema fixtures
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
  both a marker and a current list, and an unknown detail variant. No fixture or renderer contains
  an unparameterized blocker-specific remedy. An accepted migration pauses on execution and
  storage faults, reports status, resumes the same `AttemptIdentity`,
  succeeds, releases the transient lane, and leaves its permanent records only
  in the source generation's exclusive allocation;
- post-eviction identical replay and authenticated
  `ReadProtectedAttemptStatus` for the original peer and protected operator,
  cross-peer refusal, and generated exhaustive mapping of every operation in
  every endpoint for each accepted operation and closed terminal success
  outcome to exactly one nested operation-specific
  `ProtectedAttemptRecovery::Success` wire variant, and every operation and
  typed terminal refusal to exactly one nested `OriginalRefusal` variant with
  its exact safe product and reachable recovery-read operation. Every success
  next action is either mechanically justified `NoFurtherAction` or names an
  endpoint-table operation and caller class authorized by that exact row.
  Assignment recovery separately proves `UseExistingCapability` only for
  `Active` and the fresh request, evidence, and issuance sequence only for
  completed, revoked, expired, or exhausted states.
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
  unclaimed with sink `AcceptedBound`, processing, paused, quarantined pending
  audit, conversion intent recorded, old-generation invalidation pending, old
  generation invalidated with rebind pending, refusal generation bound with
  replacement tuple pending, replacement tuple installed, all five
  migration-audit-repair states,
  `OrdinarySinkAcknowledgementPending`,
  `OrdinaryActivationPending`, `MigrationSinkAcknowledgementPending`, and
  `MigrationActivationPending`.
  Each asserts exactly its owned safe fields, bounded
  deadline and exact closed action. Generated strict compile and parse
  negatives cover every action from another state, generic interpretation of a
  migration tag or migration interpretation of an ordinary tag, invalidated
  generation in a later action, missing or extra lease, pause, proof,
  generation, event or acknowledgement fields, an optional-field encoding,
  and an unknown or fallback variant. Auto-resume or lease-expiry takeover
  keeps every pause recoverable;
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
  and migration audit-repair refusal; and every migration preflight and signal
  overflow reason. Risk replay conflict and risk conflict-budget exhaustion
  first return exact current `RiskRecoveryStatus`; only
  `ClosedMutationPermitted` renders `RequestNewRiskOperationIntent`, while
  only `CallerKeyOperation` renders a fresh caller key.
  Remedy fixtures separately prove
  controller-unavailable orphan recovery restores the controller before
  requesting proof and retrying cancellation, invalid orphan proof repairs the
  controller/sink binding before requesting proof and retrying, current
  `round-input-store-full` and migration-ineligible complete details execute
  their named plan ids directly, only redacted or stale details trigger a
  status read, no unparameterized blocker remedy exists, and no context renders
  an offline migration action.

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
shape catch them.
The completion-specific failure is treating changed assignment bindings under
one settled evidence identity as replay. Full binding-digest precedence makes
that a conflict, but the conflict does not prove the existing capability was
lost. Exact assignment recovery returns the current state: active work keeps
the same capability, terminal work starts only through a new orchestrator
request and fresh protected evidence, and an operator who wants to abandon a
possibly lost active capability revokes it first. Completion and append
refusals expose fixed domain-separated products, so an error path cannot leak
the raw evidence identity, sink namespace, capability, handle, path, or
deployment id while claiming to be redacted. The append product also excludes
`ControllerNamespaceAlias` from every governed projection.

Human MAJOR acceptance remains powerful. A protected authority resolver,
separate typed operation, exact candidate binding, mandatory expiry,
revocation, and repeated validity checks make it attributable and
non-transferable. They do not make the accepted risk smaller.

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
capacity-switch edge. The sealed control reserve and migration audit-repair
state retain the same success and rebind only its sink generation. Separate
control identities prevent fresh caller keys or changed request bytes from
allocating another control record. The sibling integrity reserve keeps direct
status and control-reserve repair reachable without using corrupt control
capacity. Bounded preflight signals preserve conflict evidence without letting
signal, status, or conflict traffic consume execution or control capacity;
hard TTLs and reusable window slots prevent diagnostic accumulation. Detailed,
aggregate, health, and exporter failure still return the original refusal,
with fixed-cardinality health exposing degradation when observation remains
available and no capacity generation in metric labels.

Accepted attempts gain reconciliable prepares, common base-or-conflict
identity, sink reservations and generation authorizations, worker epochs,
immutable tombstones, and monotonic eviction markers. That is more durable
state. It prevents a live paused worker from being recovered twice, a sink
from expiring capacity below an accepted event, a delayed old-generation
append from reviving fenced success, and cleanup from claiming deleted replay
bytes still exist. Attempt response bytes remain bounded; post-eviction replay
and status retain exhaustive operation-specific safe recovery rather than
execution permission or an unusable digest-only answer.

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
