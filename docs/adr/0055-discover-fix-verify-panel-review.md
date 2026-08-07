# ADR 0055: Discover, fix, and verify panel review

- Status: Proposed
- Date: 2026-08-06
- Partially supersedes: [ADR 0053](0053-gascity-contributor-infrastructure.md)
  D7's open-ended review and fix loop, own-findings-only dispatch payload, and
  Gas-City-specific use of the protected panel-and-approval controller; D8's
  single blocking treatment of every recommendation in an admitted final set;
  D9's publication refusal while any finding stands; and D17's closed
  endpoint operation sets and round-input eligibility rules, only as replaced
  by the closed endpoint table and receipt lifetime below. It also supersedes
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
  adds two least-authority endpoints, and replaces only the closed operation
  sets named below. Approval and risk operations remain absent from the
  orchestrator endpoint.
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
ledger, severity, approval and retention state. A deployment may expose
peer-authenticated Unix sockets or resolve opaque receipts from a protected
principal, but same-uid repository files, helper output and self-asserted
identity are never authoritative. If neither protected form is available, the
producer returns `protected-authority-unavailable` and does not dispatch.

This table narrowly replaces D7 and D17's closed endpoint operation sets. Each
endpoint has its own request enum and authentication policy. An operation
absent from an endpoint cannot be reached by presenting another endpoint's
request bytes.

| Endpoint | Authorized caller | Complete operation set |
| --- | --- | --- |
| Orchestrator | Candidate-bound standalone or future Gas City producer peer | `ProposeLifecycleStart`, `RequestPanelDispatch`, `SubmitCandidateSnapshot`, `SubmitLedgerSynthesisProposal`, `SubmitImplementationDisposition`, `SubmitImplementationSelfReviewFinding`, `SubmitValidationManifest`, `RequestGeneratedSeatArtifacts`, `ReadLifecycleStatus` |
| Reviewer | One controller-issued, candidate-bound trusted dispatch for the named seat | `SubmitNativeFindingPage`, `SubmitLateFinding`, `SubmitVerificationJudgment`, `SubmitLegacySourceTriage`, `SubmitLegacySourceTriageVerification`, `SubmitSeverityCorrection`, `SubmitSeverityCorrectionVerification`, `SubmitLedgerMappingConcurrence`, `SubmitRiskAcceptanceVerification`, `SubmitFinalSignoff` |
| Operator | Protected operator identity resolved from peer evidence | `SubmitApprovalDecision`, `AbandonLifecycle`, `ResumeLifecycle`, `RescopeLifecycle`, `CreateSameScopeCurrentSchemaSuccessor`, `CreateReverificationSuccessor`, `PermanentlyCloseAbandonedLineage`, `ApplyLedgerMappingCorrection`, `IssueRiskOperationIntent`, `AcceptMajorRisk`, `RevokeMajorRiskAcceptance`, `ReadLifecycleStatus` |
| Issue reader | A current candidate-bound implementation assignment or resolved merge authority | `ReadImplementerIssueView`, `ReadMergeAuthorityMajorIssueView` |
| Publisher | Protected publisher identity | `ConsumePublicationManifest`, `RecordTrustedMergeCompletion`, `ReadPublicationStatus` |

`SubmitApprovalDecision` retains D17's closed
`{approve, revise, rescope, abort}` value. Approval and risk operations,
ledger-mapping mutation, lifecycle termination and permanent close are absent
from the orchestrator endpoint. Status reads do not mutate protected state.
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
Every request frame carries an idempotency key, every operation is
candidate-bound where a candidate exists, and every endpoint uses the audit
and idempotency contract in section 13. Risk intents use the stronger
controller-issued-key rule in section 10.

The issue-reader endpoint is read-only and least-authority:

- `ReadImplementerIssueView` requires a current implementation assignment and
  returns only the assigned issue ids, protected descriptions, evidence,
  recommendations, and disposition obligations needed to fix that candidate.
  It cannot enumerate another assignment, obtain authority or identity
  mappings, or mutate ledger state.
- `ReadMergeAuthorityMajorIssueView` requires a current
  `MergeAuthorityResolver` result and returns only the requested effective
  MAJOR issue, its protected rationale, evidence, validation references,
  mapping version, and existing acceptance state for the exact candidate. It
  cannot inspect unrelated issues or perform acceptance.

Both reads refuse a lifecycle, candidate, mapping-version, issue, assignment
or authority binding mismatch as `issue-view-binding-mismatch`. Public,
generic status, log and audit views retain the redacted projections in
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

Closing a finding as invalid or withdrawn does not rewrite or downgrade its
historical severity. A content change makes a prior severity-correction
verification stale; the source's preceding current severity is effective
again until the correction is independently verified against the new
candidate. A split or merge replays source severity and correction events
without re-triage because source identity is unchanged.

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
original admitted artifact and digest. The same key with different bytes, or
different proposed bytes for an already admitted base generation, is
`ledger-synthesis-conflict`; it does not silently replace the ledger.

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
the event. Missing concurrence is `ledger-mapping-concurrence-missing`,
dissent is `ledger-correction-invalid`, and stale concurrence is
`ledger-mapping-concurrence-stale`. Repeating the identical correction returns
the existing event; a conflicting replay is
`protected-operation-replay-conflict`.

The current effective mapping is derived by replaying mapping events. A source
maps to exactly one effective issue after every event. A correction invalidates
only dependent issue-level verification and acceptance state whose subject set
changed: verification and adjudication judgments over the old grouping, and
risk or lifecycle approval state that named the old mapping. Those items must
be re-established against the corrected mapping and current candidate. Raw
findings, legacy source triage, source-level severity corrections and
implementation-disposition history replay unchanged. A split projects its
existing disposition onto each resulting source subset. A merge is admitted
only when the source issues have the same current disposition and
candidate-evidence digest; otherwise it is `ledger-correction-invalid` until
implementation submits compatible dispositions before the protected
correction.

Terminal metrics count effective issue classes at the terminal ledger version.
A split can increase and a merge can decrease the unique issue count; aliases
never count as additional issues. A fixed issue contributes once only if its
effective terminal issue reaches verified `Fixed` after the last correction.
Metric records bind the mapping version so a historical count is never
reinterpreted.

### 5. Implementation dispositions do not adjudicate reviewer truth

The first implementation pass after discovery is one batch over the complete
ledger. Parallel fix slices remain allowed when file ownership is disjoint,
but they receive the same ledger and integrate into one candidate before
verification.

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
named lineage, rescope it into a named successor, permanently close it, or
raise the reviewed bound. Ordinary abandonment does not free the capsule and
is never presented as a capacity remedy.

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
before it accepts the operation bytes. For either operation, the same key and
byte-identical request returns the original event and response; the same key
with different request bytes is `risk-operation-replay-conflict`. A lost
response or crash after durable admission therefore cannot create a second
live acceptance or a second revocation.

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
| `risk-operation-replay-conflict` | `RequestNewRiskOperationIntent` |
| `major-risk-duplicate-live` | `RevokeMajorRiskAcceptance` |
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
validation evidence, migration origin, risk records, terminal metrics, and
final per-seat records. Abandonment and supersession mint terminal metric
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
old round is incomplete, admission returns `legacy-round-partial`. If it
cannot complete because a reviewer is unavailable, the protected operator may
invoke `CreateSameScopeCurrentSchemaSuccessor`. This is not
`RescopeLifecycle`: declared scope and candidate stay exact, and completed
seats are not discarded or rerun merely to make the old round complete.

The transition retains every partial-round byte for audit, deterministically
creates a `LegacySourceId` for every recommendation in every well-formed
completed-seat record, and imports those sources as prior obligations. It
never labels the partial round discovery and never imports a malformed or
incomplete seat. The current-schema successor then runs exactly one fresh
native discovery over its selected roster. Native discovery findings and
imported legacy obligations enter the same proposed ledger synthesis without
losing either source identity.

Successor creation, completed-seat source import, old-to-new lifecycle and
issue crosswalks, source lifecycle termination, and the fresh-discovery
requirement are one atomic binding. The operation derives the successor and
crosswalk identity from its request idempotency key, source lifecycle, pinned
dispatch and completed-seat digest set. An identical retry returns the
original transition. A different completed-seat set or crosswalk under the
same key is `same-scope-successor-conflict`; a crash exposes neither a partial
successor nor a partially imported source set. This escape does not require a
genuine scope change and does not erase the completed seats.

The imported complete round is the lifecycle's migration discovery input. It
does not claim to be a native current-schema discovery panel. The terminal
receipt records `discovery_origin = legacy_imported`; native lifecycles record
`native`. A same-scope successor from a partial legacy round also records
`discovery_origin = native`, because its only discovery is fresh, and separately
records `migration_origin = partial_legacy_prior_obligations`.

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
implementation disposition can satisfy approval.

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

For a same-scope successor from a partial round, generation uses the completed
seat digest set as an additional identity input. The same set returns the same
legacy sources, successor and atomic crosswalk. It records
`legacy_partial_successor_count = 1`,
`legacy_partial_completed_seat_count`, and
`legacy_partial_imported_source_count`; missing-seat retries and response-loss
retries do not increment any of them.

After import, section 7 automatically generates every seat's verification
artifact with the full imported ledger and seat obligations. Legacy strings
are never hand-copied into reviewer notes.

### 13. Retention, redaction, audit, and terminal metrics are explicit

Every new artifact is in one ADR 0053 D17 retention class.

| Artifact | D17 class |
| --- | --- |
| Exact native and legacy reviewer bytes, prompts, generated per-seat bundles, full protected ledger pages, issue descriptions, source text, validation-output bytes, private acceptance rationale, protected authority mappings, migration work records, and `SuccessorImportCapsule` bytes | Round input |
| Source and artifact digests, stable ids, source mapping and crosswalk events, dedup and severity events, closed disposition and judgment projections, roster and dispatch bindings, acceptance and revocation projections, lifecycle receipts, seals, and terminal metric records | Audit floor |

Round inputs retain D17's 30-day or 2-GiB bound after they become eligible.
Nothing belonging to an active lifecycle, an unresolved lifecycle, a partial
legacy round, an unexpired merge-eligible receipt, or a resumable abandoned
lineage's `SuccessorImportCapsule` is eligible for eviction. Section 11's
trusted merge-completion event or receipt expiry determines eligibility for
terminal round inputs. Permanent close determines capsule eligibility. If the
cap is reached and only protected ineligible records remain, admission of new
round bytes is refused. The implementation does not evict active state, drop
descriptions, claim ordinary abandonment freed capacity, or degrade to an
incomplete reviewer payload.

All new durable and observable surfaces use declared bounded redacting types,
closed identifiers, closed enums, safe aliases, or digests:

- protected ledger and prompt views may reveal bounded issue text only to the
  dispatched seat;
- protected identity and rationale mappings are never public;
- public review and publication projections contain only safe aliases, issue
  ids, severities, closed dispositions and outcomes, bounded numerics,
  timestamps, and digests;
- logs and errors do not render raw recommendations, rationales, legacy
  strings, paths, branch names, user identities, run handles, or evidence
  bytes; and
- no governed type exposes those values through derived or handwritten
  `Debug`.

Every protected-operation attempt that reaches an endpoint produces exactly
one digest-only typed audit event, whether it succeeds or is refused. A
byte-identical transport retry with the same idempotency key is the same
logical attempt and returns the original result without duplicating the
event. A same-key, different-bytes request is a distinct typed conflict
attempt with its own deterministic conflict-attempt identity and exactly one
refusal event. Authentication refusal, endpoint-operation absence, stale
binding, read refusal, lifecycle transition, ledger or severity operation,
adjudication, risk operation, migration operation, receipt decision, seal,
publication check and merge-eligibility check all use this rule.

The controller uses a transactional outbox or an equivalent proven ordering.
For a mutating request, authoritative state and one pending audit event commit
atomically; no effect or success response becomes externally visible until the
generic root-owned append sink has synchronously flushed that event. Read and
refusal responses likewise wait for their event to flush. Recovery replays a
pending event before publishing the stored result. A crash before the state
transaction commits leaves neither effect nor event; a crash after it commits
but before flush exposes neither effect nor response; a crash after flush but
before response returns the stored response on retry without another event.
If audit flush cannot complete, the operation remains externally uncommitted
and any proposed authority effect remains quarantined. Recovery discards that
effect, finalizes the still-unflushed pending event as the one
`audit-event-flush-failed` refusal, and flushes it before returning the typed
failure. It does not append a second event for the same attempt.

The protected front door remains able to audit
`protected-authority-unavailable` when its state worker or authoritative
resolver is unavailable. A connection failure before the protected front door
accepts an authenticated request is a producer preflight failure and cannot
have an authoritative effect. The append sink retains ADR 0053 D17's
append-only, write-once, daily-rotated, bounded, synchronously flushed shape.
Audit is evidence; protected controller state remains authority.

Every terminal lifecycle writes exactly one typed
`TerminalLifecycleMetricRecord` with:

- outcome `signed_off`, `abandoned`, or `superseded`;
- completeness `complete` or `degraded`;
- discovery origin `native` or `legacy_imported`;
- migration origin `none`, `complete_legacy_discovery`, or
  `partial_legacy_prior_obligations`;
- final candidate, lineage, scope, ledger, and mapping digests;
- initial, late, severity, iteration, disposition, and adjudication counts;
- dedup split, merge, and alias counts;
- legacy source, imported issue, re-triaged issue, partial-round retry, and
  migration retry counts;
- same-scope partial-successor, completed legacy seat, and imported partial
  legacy source counts; and
- closed degraded-reason codes when completeness is `degraded`.

`signed_off` requires `complete`. `abandoned` and `superseded` still emit a
record even when evidence is incomplete; for them `degraded` identifies a
closed reason such as `partial_legacy_round`, `missing_verification`, or
`terminal_before_retriage`. Degraded state never satisfies approval.

Metric counting is fixed:

- `initial_findings` is the number of terminal effective issue classes whose
  earliest source is in the native or imported discovery input;
- `prior_obligation_findings` is the number whose earliest source came from a
  completed seat of a partial legacy round; those sources are not counted as
  discovery or late findings;
- `late_findings` is the number of terminal effective issue classes whose
  earliest source was admitted after discovery;
- `late_blocker_count` and `late_major_count` use those late classes and their
  terminal effective severities;
- native and migration-assigned severities are counted in separate fields, so
  no chart implies a legacy string carried historical severity;
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
| `protected-operation-replay-conflict` | endpoint, operation, idempotency-key digest, request digests | `IssueNewProtectedOperationIntent` |
| `protected-operation-invalid-state` | lifecycle, operation, current state | `ReadLifecycleStatus`, then `UseStatePermittedOperation` |
| `audit-event-flush-failed` | endpoint, operation, attempt id | `RestoreProtectedAuditSink`, then `RetryProtectedOperation` |
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
| `issue-view-binding-mismatch` | lifecycle, candidate, mapping version, issue, assignment or authority ids | `RequestCurrentCandidateBoundIssueView` |
| `raw-source-unmapped` | lifecycle, source ids | `RegenerateAutomaticLedger` |
| `raw-source-multiply-mapped` | source ids, issue ids | `RequestProtectedLedgerCorrection` |
| `issue-id-duplicate` | lifecycle, issue ids | `RegenerateAutomaticLedger` |
| `issue-id-reassigned` | issue id, old and proposed source digests | `RequestProtectedLedgerCorrection` |
| `ledger-synthesis-conflict` | lifecycle, ledger version, artifact digests | `ReturnToAdmittedLedger` |
| `ledger-correction-invalid` | correction, source ids, issue ids | `RetryProtectedAtomicLedgerOperation` |
| `ledger-correction-stale` | correction, expected and actual ledger version | `RegenerateLedgerCorrection`, then `RetryProtectedAtomicLedgerOperation` |
| `ledger-mapping-concurrence-missing` | correction, affected source and reporting seat ids | `CollectAffectedReporterConcurrence`, then `RetryProtectedAtomicLedgerOperation` |
| `ledger-mapping-concurrence-stale` | correction, candidate, expected and actual mapping versions | `RedispatchMappingConcurrence`, then `RetryProtectedAtomicLedgerOperation` |
| `successor-import-incomplete` | source and successor lifecycle, source ids | `RetryProtectedAtomicLineageOperation` |
| `same-scope-successor-conflict` | source lifecycle, dispatch, completed-seat set, idempotency-key digest | `ReturnToAdmittedSameScopeSuccessor` |
| `reverification-successor-ineligible` | lifecycle, receipt id and expiry | `WaitForReceiptExpiryOrUseCurrentReceipt` |
| `post-discovery-scope-expansion` | lifecycle, candidate, scope digest | `RequestProtectedRescope` |
| `post-discovery-change-unmapped` | lifecycle, candidate, changed-region ids | `MapChangeToLedgerIssueOrRequestProtectedRescope` |
| `issue-disposition-missing` | lifecycle, issue ids | `CompleteIssueDisposition` |
| `verification-coverage-incomplete` | candidate, issue ids, seat ids | `RedispatchVerificationObligations` |
| `verification-judgment-conflict` | candidate, issue ids, seat ids | `RedispatchDedicatedAdjudication` |
| `severity-correction-unauthorized` | candidate, source and reporting seat or successor ids | `RedispatchSourceAuthorizedSeverityCorrection` |
| `severity-correction-unverified` | candidate, source ids | `RedispatchIndependentVerifier` |
| `legacy-source-severity-unassigned` | lifecycle, legacy source ids | `RedispatchLegacySourceTriage` |
| `legacy-source-severity-correction-unauthorized` | candidate, legacy source, historical seat and successor ids | `DispatchVersionedAccountabilitySuccessor`, then `RedispatchIndependentVerifier` |
| `late-finding-ineligible` | candidate, source id, submitted reason | `FileFindingOutsideLifecycle` |
| `required-validation-missing` | candidate, validation job ids | `RunRequiredEnforcingValidation` |
| `required-validation-failed` | candidate, validation job ids | `ReturnToScopedBatchFix`, then `RunRequiredEnforcingValidation` |
| `advisory-validation-used-as-evidence` | candidate, validation job ids | `RunRequiredEnforcingValidation` |
| `required-validation-marked-inapplicable` | candidate, validation job ids | `RunRequiredEnforcingValidation` |
| `companion-validation-missing` | candidate, companion ids | `RunExplicitCompanionValidation` |
| `legacy-round-start-after-cutover` | dispatch, cutover revision, schema version | `StartCurrentSchemaLifecycle` |
| `legacy-round-partial` | dispatch, completed and missing seat ids | `CompletePinnedLegacyRound`, then on `reviewer_unavailable` `CreateSameScopeCurrentSchemaSuccessor` |
| `legacy-source-unmapped` | lifecycle, legacy source ids | `RegenerateAutomaticLedger` |
| `legacy-retriage-incomplete` | lifecycle, legacy source ids, seat ids | `RedispatchLegacySourceTriage` |
| `legacy-schema-version-unsupported` | artifact digest, found and supported versions | `InstallSupportedVersionDispatcher`, then `RetryLegacyImport` |
| `legacy-regeneration-conflict` | lifecycle, import and artifact digests | `ReturnToAdmittedLegacyImport` |
| `risk-operation-replay-conflict` | operation, acceptance or revocation id, idempotency-key and request digests | `RequestNewRiskOperationIntent` |
| `major-risk-duplicate-live` | lifecycle, candidate, acceptance ids | `RevokeMajorRiskAcceptance` |
| `blocker-open` | candidate, issue ids | `ReturnToScopedBatchFix` |
| `approval-receipt-expired` | lifecycle, candidate, receipt id and expiry | `CreateReverificationSuccessor` |
| `merge-completion-binding-mismatch` | receipt, expected and actual target and candidate ids | `ResolveTrustedMergeCompletion`, then `RetryRecordMergeCompletion` |
| `round-input-store-full` | active and resumable lineage ids, configured bound | `ResolveRetentionCapacity` |
| `redaction-contract-violation` | artifact id, field code | `RegenerateBoundedRedactedArtifact` |
| `final-verification-nonunanimous` | candidate, seat ids | `RedispatchFinalVerification` |
| `lifecycle-receipt-invalid` | candidate, receipt id, failed invariant ids | `SatisfyReceiptPrerequisites`, then `RegenerateLifecycleReceipt` |

The section 10 risk variants are rows in this same closed catalog, not a
separate open extension. `ResolveRetentionCapacity` is itself a closed choice
over `ResumeNamedAbandonedLineage`, `SupersedeNamedLineage`,
`PermanentlyCloseNamedAbandonedLineage`, and
`RaiseReviewedRetentionBound`. It never claims ordinary abandonment freed
state and never deletes ineligible records.

Every normative refusal site in this record names exactly one catalog row,
including every operation in the endpoint table. A machine-readable
operation-to-refusal map is total in both directions: an implementation
refusal with no row, a row with no reachable normative site, or an endpoint
operation with an unclassified refusal fails validation.

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
  source, ledger, correction, disposition, judgment, acceptance, migration,
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
an endpoint operation lacks a refusal map, when the corpus is empty, or when a
planted negative is accepted. At minimum, the corpus separately exercises:

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
  grouping, controller admission, identical retry, conflicting replay, split,
  merge, affected-reporting-reviewer concurrence, protected operator
  authorization, and refusal of an orchestrator mapping mutation;
- false BLOCKER and MAJOR invalidation, withdrawal, severity correction,
  reporting-seat dissent, retired-legacy-seat accountability succession, and
  missing independent coverage;
- every disposition and judgment combination, including
  `implementation-self-review`, disposition supersession by invalid or
  withdrawn adjudication, no-content-change closure, and candidate-change
  staleness;
- automatic full-ledger per-seat artifacts, missing chunk, stale chunk,
  duplicate chunk, conflicting identity regeneration, no hand-authored
  substitute, least-authority implementer and merge-authority issue views, and
  every issue-view binding mismatch;
- touched and untouched late findings for every allowed reason, plus refused
  pre-existing MINOR and NIT controls;
- ledger-scoped fixes, unrelated scope expansion, atomic rescope, crash and
  retry, abandonment, bounded `SuccessorImportCapsule`, refusal over its bound,
  resume while ineligible for eviction, atomic successor import, permanent
  close, permanent-closed-lineage reuse refusal, and each capacity remedy;
- every merge-authority evidence form, same-uid standalone refusal, acceptance
  issue and revocation, controller-issued idempotency key, identical retry,
  conflicting replay, response loss at every durable boundary, prohibited
  duplicate revocation, expiry at each of verification receipt, lifecycle
  receipt, seal, publication, and merge eligibility, and candidate or mapping
  mismatch;
- completed, in-flight, partial, retried, duplicate, malformed, and
  already-ingested legacy rounds with arbitrary recommendation strings,
  refusal to start an old-schema round after cutover, unavailable-reviewer
  same-scope succession, completed-seat prior-obligation import, one fresh
  native discovery, atomic crosswalk, response-loss retry, and exact metrics;
- exact legacy-byte preservation, deterministic source ids, complete automatic
  crosswalk, per-source migration triage, source-triage replay through split
  and merge, retired-seat correction, and no invented historical severity;
- approval-receipt seven-day cap, tighter MAJOR-acceptance cap, trusted merge
  completion, receipt-expiry merge refusal, terminal-input eligibility,
  eviction to audit-floor projections, and mandatory re-verification;
- active-retention refusal, both D17 bounds, exact one-event auditing of every
  protected success and refusal, transactional-outbox recovery and crashes
  before state commit, after state commit, after audit flush and before
  response, retry without audit duplication, redaction and `Debug` controls,
  and all three terminal outcomes in complete and permitted degraded shapes;
- merge-ready MINOR and NIT treatment, unresolved blocking states, final
  unanimity, and green validation without panel approval; and
- every typed error, every endpoint-operation/refusal mapping, and both
  producer-context remedy renderings, with mechanical parity between
  normative refusal sites and catalog rows.

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
Explicit prompts, complete raw-output retention, no truncation, late-finding
metrics, and the late ledger make misses visible rather than pretending they
cannot happen.

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
re-triage, complete automatic crosswalks, and idempotent generation bound that
machinery.

Human MAJOR acceptance remains powerful. A protected authority resolver,
separate typed operation, exact candidate binding, mandatory expiry,
revocation, and repeated validity checks make it attributable and
non-transferable. They do not make the accepted risk smaller.

The retention exception for active and unresolved work can fill the store.
The deliberate result is denial of new admission, not eviction of the only
evidence capable of closing existing work. Resumable abandonment has the same
cost: its bounded capsule remains protected until resume, supersession or
explicit permanent close, so abandonment cannot be misreported as reclaimed
capacity.

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
identified, preserved, re-triaged, and imported without pretending it already
had the new schema.

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
