# ADR 0055: Discover, fix, and verify panel review

- Status: Proposed
- Date: 2026-08-06
- Partially supersedes: [ADR 0053](0053-gascity-contributor-infrastructure.md)
  D7's open-ended review and fix loop and own-findings-only dispatch payload;
  D8's single blocking treatment of every recommendation in an admitted final
  set; D9's publication refusal while any finding stands; D17's closed
  `{approve, revise, rescope, abort}` operator-operation surface, only to add
  the separate protected `AcceptMajorRisk` and `RevokeMajorRiskAcceptance`
  operations; and D21's per-seat `held` and `prior_resolutions` state,
  rotation, rejection of a severity ladder, and clean-break refusal to read or
  admit an earlier delivery schema. D21's controller-owned roster selection,
  surface classifier, profile binding, reviewer identity, and candidate-bound
  evidence remain in force. D7's three peer-separated endpoints remain in
  force; the new risk operations exist only on the operator endpoint.
- Related: [ADR 0048](0048-copilot-native-agent-surface.md), whose
  Copilot-native surface, independent read-only reviewers, pinned bindings,
  helper-assembled records, and staged evidence remain in force. This record
  does not supersede ADR 0048.
- Scope: Panel review lifecycle, finding and final-verdict semantics,
  compatibility migration, review evidence, retention, and convergence
  metrics.
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
the current behavior. This Proposed record decides a replacement target; it
does not describe the target as shipped.

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
new snapshot to it is refused.

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

The discovery panel runs exactly once. A zero-finding discovery still proceeds
through self-verification and verification; it skips a no-op batch
implementation.

### 2. Native discovery is comprehensive, parallel, and exhaustive

The controller selects the discovery roster under ADR 0053 D21. Every selected
reviewer receives the full candidate, immutable staged evidence and digests,
applicable validation evidence, its controller-bound profile, and read-only
repository context.

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

The effective severity of an issue is the highest uncorrected severity among
its sources. It may be lowered only when every seat that supplied a source at
each higher severity submits a candidate-bound `SeverityCorrection` through
trusted dispatch and at least one final-roster seat that did not implement the
candidate submits `severity_correction_verified`. The controller records both
events and their evidence digests. The integrator, orchestrator, operator, and
controller cannot originate a correction or lower severity by deduplication.
A dissenting or missing higher-severity source leaves the higher severity
effective. Closing a finding as invalid or withdrawn does not rewrite or
downgrade its historical severity. A content change makes a prior
severity-correction verification stale; the raw higher severity is effective
again until the correction is independently verified against the new
candidate.

### 4. The orchestrator synthesizes one stable issue ledger automatically

The orchestrating agent, not the operator, automatically assigns the next
stable identifiers `R1`, `R2`, and so on and synthesizes bounded issue
descriptions from the raw findings. The operator never copies recommendations,
chooses ids, or constructs a crosswalk.

Trusted tooling admits the synthesis only after validating:

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

The first admitted synthesis fixes the source-to-id mapping. Repeating
generation over the same input returns the same admitted artifact and digest.
A retry that proposes different bytes for an already admitted generation is
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

The orchestrator requests either event through the controller's protected
`ApplyLedgerMappingCorrection` operation. The controller validates exact
source coverage, candidate binding, monotonic id allocation, and idempotency
before appending it. Repeating the same correction returns the existing event.

The current effective mapping is derived by replaying mapping events. A source
maps to exactly one effective issue after every event. A correction invalidates
all affected implementation dispositions, verification judgments, severity
correction verifications, and risk acceptances. They must be re-established
against the corrected mapping and current candidate. Raw findings and their
historical events remain unchanged.

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

| Disposition | Adjudication needed to close | Otherwise |
| --- | --- | --- |
| `Fixed` | `verified_resolved` | issue stays open |
| `NoChangeClaimed` | `verified_invalid` or `verified_withdrawn` | issue stays open |
| `Deferred` | none; panel records `open` and verifies the stated evidence | severity rules decide approval |

An independently verified unresolved MINOR or NIT has complete verification
coverage even though it remains open. Verification completeness and issue
resolution are deliberately different facts.

### 7. Verification artifacts are generated, complete, and idempotent

Trusted tooling automatically generates every per-seat verification artifact.
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
bytes at the same identity are refused. A retry of one seat neither duplicates
an admitted judgment nor changes another seat's obligations.

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
is refused even if it is useful.

A genuine scope change uses the controller's protected `RescopeLifecycle`
operation. It terminates the source lifecycle as `superseded`, creates one
successor with a larger or different declared scope, atomically imports all
raw findings and every unresolved effective issue, and records a stable
old-id-to-successor-id crosswalk. The successor is a new current-schema
lifecycle and runs its own one comprehensive discovery panel. Imported
findings are prior obligations, not a substitute for that discovery.

`AbandonLifecycle` terminates without deleting findings. A later resume is a
new successor, never mutation of the abandoned lifecycle. `ResumeLifecycle`
and repeated `RescopeLifecycle` calls derive successor identity from the
source lifecycle and protected operation id, so retry returns the same
successor.

Successor creation, source import, crosswalk publication, and source
termination are one atomic transition. If complete import cannot commit, no
successor becomes usable and the source lineage remains terminal or parked.
There is no state in which a successor exists without all raw findings and
unresolved items. Abandonment, rescope, retry, or deduplication therefore
cannot erase an awkward finding.

### 10. MAJOR risk acceptance is a separate protected authority operation

A BLOCKER cannot be risk accepted. A MAJOR may remain open only under a valid
`MajorRiskAcceptance`.

`AcceptMajorRisk` is a distinct typed protected operation. It is not an
`approve` decision, cannot close a gate, and cannot be reached from ADR 0053's
orchestrator or publisher endpoints. It extends only the protected operator
endpoint. `RevokeMajorRiskAcceptance` is a second distinct operation on that
same endpoint.

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

Revocation is an append-only event. Validity means the acceptance is
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
operator operation set made this separate operation impossible. The
peer-separated endpoints, controller identity, append-only authority, and
publication approval remain unchanged.

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
unanimous over the monotonic lifecycle roster selected under ADR 0053 D21.
Newly selected specialists join verification; no discovery reviewer rotates
out.

The controller mints a `PanelLifecycleApprovalReceipt` only for `signed_off`.
It binds the final candidate, scope and lineage, every roster and trusted
dispatch, all source records and ledger events, dispositions, judgments,
validation evidence, migration origin, risk records, terminal metrics, and
final per-seat records. Abandonment and supersession mint terminal metric
records but never an approval receipt. Green tests are evidence in the receipt
and never substitute for panel approval.

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
- A partial legacy round is never discovery evidence and never a ledger
  source. Missing or invalid seats remain retry state for that same pinned old
  dispatch. No new old-schema round may be started after cutover.

Retrying missing seats does not mix schemas inside one round. If the old
dispatch cannot complete, the lifecycle stays in `legacy-round-retry`; an
operator is not asked to throw away a completed round or hand-build migration
state. A protected rescope may create a current-schema successor only through
section 9's atomic import rules.

The imported complete round is the lifecycle's migration discovery input. It
does not claim to be a native current-schema discovery panel. The terminal
receipt records `discovery_origin = legacy_imported`; native lifecycles record
`native`.

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

No legacy severity is invented. Every imported issue begins
`severity_origin = migration_untriaged`. Before implementation disposition can
satisfy approval, trusted dispatch obtains an explicit current-schema
re-triage and at least one independent final-roster verification of the
assigned severity. The resulting value is
`severity_origin = migration_assigned`; it is a current migration judgment,
not historical fact. Until every imported source participates in a verified
re-triage, the lifecycle fails closed.

Generation is idempotent. The same complete round, candidate, and accepted
grouping return the same source ids, `R` ids, descriptions, crosswalk, and
artifact digests. A changed grouping after admission is a dedup correction,
not regeneration. Repeated ingestion appends no duplicate sources, judgments,
metrics, or audit events.

After import, section 7 automatically generates every seat's verification
artifact with the full imported ledger and seat obligations. Legacy strings
are never hand-copied into reviewer notes.

### 13. Retention, redaction, audit, and terminal metrics are explicit

Every new artifact is in one ADR 0053 D17 retention class.

| Artifact | D17 class |
| --- | --- |
| Exact native and legacy reviewer bytes, prompts, generated per-seat bundles, full protected ledger pages, issue descriptions, source text, validation-output bytes, private acceptance rationale, protected authority mappings, and migration work records | Round input |
| Source and artifact digests, stable ids, source mapping and crosswalk events, dedup and severity events, closed disposition and judgment projections, roster and dispatch bindings, acceptance and revocation projections, lifecycle receipts, seals, and terminal metric records | Audit floor |

Round inputs retain D17's 30-day or 2-GiB bound after they become eligible.
Nothing belonging to an active lifecycle, an unresolved lifecycle, a partial
legacy round, or an unexpired merge-eligible receipt is eligible for eviction.
If the cap is reached and only protected ineligible records remain, admission
of new round bytes is refused. The implementation does not evict active state,
drop descriptions, or degrade to an incomplete reviewer payload.

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

The generic root-owned append sink receives one digest-only typed event for
each lifecycle transition, ledger correction, severity correction,
adjudication, risk operation, migration admission, receipt decision, seal,
publication check, and merge-eligibility check. It retains ADR 0053 D17's
append-only, write-once, daily-rotated, bounded, synchronously flushed shape.
The audit event is evidence; protected controller state remains authority.

Every terminal lifecycle writes exactly one typed
`TerminalLifecycleMetricRecord` with:

- outcome `signed_off`, `abandoned`, or `superseded`;
- completeness `complete` or `degraded`;
- discovery origin `native` or `legacy_imported`;
- final candidate, lineage, scope, ledger, and mapping digests;
- initial, late, severity, iteration, disposition, and adjudication counts;
- dedup split, merge, and alias counts;
- legacy source, imported issue, re-triaged issue, partial-round retry, and
  migration retry counts; and
- closed degraded-reason codes when completeness is `degraded`.

`signed_off` requires `complete`. `abandoned` and `superseded` still emit a
record even when evidence is incomplete; for them `degraded` identifies a
closed reason such as `partial_legacy_round`, `missing_verification`, or
`terminal_before_retriage`. Degraded state never satisfies approval.

Metric counting is fixed:

- `initial_findings` is the number of terminal effective issue classes whose
  earliest source is in the native or imported discovery input;
- `late_findings` is the number of terminal effective issue classes whose
  earliest source was admitted after discovery;
- `late_blocker_count` and `late_major_count` use those late classes and their
  terminal effective severities;
- native and migration-assigned severities are counted in separate fields, so
  no chart implies a legacy string carried historical severity;
- `review_iterations` counts the one native discovery execution or one
  imported complete legacy round, plus each admitted verification execution;
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
| `discovery-already-admitted` | lifecycle, discovery receipt | `ReturnToExistingLifecycle` |
| `terminal-lifecycle-reused` | lifecycle, terminal event | `CreateSuccessorWithAtomicImport` |
| `candidate-binding-stale` | lifecycle, expected and actual candidate | `RegenerateBoundArtifacts` |
| `artifact-binding-mismatch` | candidate, artifact | `RegenerateBoundArtifacts` |
| `raw-source-unmapped` | lifecycle, source ids | `RegenerateAutomaticLedger` |
| `raw-source-multiply-mapped` | source ids, issue ids | `RequestProtectedLedgerCorrection` |
| `issue-id-duplicate` | lifecycle, issue ids | `RegenerateAutomaticLedger` |
| `issue-id-reassigned` | issue id, old and proposed source digests | `RequestProtectedLedgerCorrection` |
| `ledger-synthesis-conflict` | lifecycle, ledger version, artifact digests | `ReturnToAdmittedLedger` |
| `ledger-correction-invalid` | correction, source ids, issue ids | `RetryProtectedAtomicLedgerOperation` |
| `ledger-correction-stale` | correction, expected and actual ledger version | `RegenerateLedgerCorrection`, then `RetryProtectedAtomicLedgerOperation` |
| `successor-import-incomplete` | source and successor lifecycle, source ids | `RetryProtectedAtomicLineageOperation` |
| `post-discovery-scope-expansion` | lifecycle, candidate, scope digest | `RequestProtectedRescope` |
| `issue-disposition-missing` | lifecycle, issue ids | `CompleteIssueDisposition` |
| `verification-coverage-incomplete` | candidate, issue ids, seat ids | `RedispatchVerificationObligations` |
| `verification-judgment-conflict` | candidate, issue ids, seat ids | `RedispatchDedicatedAdjudication` |
| `severity-correction-unauthorized` | candidate, issue ids, reporting seat ids | `RedispatchReportingSeatCorrection` |
| `severity-correction-unverified` | candidate, issue ids | `RedispatchIndependentVerifier` |
| `late-finding-ineligible` | candidate, source id, submitted reason | `FileFindingOutsideLifecycle` |
| `required-validation-missing` | candidate, validation job ids | `RunRequiredEnforcingValidation` |
| `required-validation-failed` | candidate, validation job ids | `ReturnToScopedBatchFix`, then `RunRequiredEnforcingValidation` |
| `advisory-validation-used-as-evidence` | candidate, validation job ids | `RunRequiredEnforcingValidation` |
| `required-validation-marked-inapplicable` | candidate, validation job ids | `RunRequiredEnforcingValidation` |
| `companion-validation-missing` | candidate, companion ids | `RunExplicitCompanionValidation` |
| `legacy-round-partial` | dispatch, missing seat ids | `CompletePinnedLegacyRound` |
| `legacy-source-unmapped` | lifecycle, legacy source ids | `RegenerateAutomaticLedger` |
| `legacy-retriage-incomplete` | lifecycle, issue ids, seat ids | `RedispatchLegacyRetriage` |
| `legacy-schema-version-unsupported` | artifact digest, found and supported versions | `InstallSupportedVersionDispatcher`, then `RetryLegacyImport` |
| `legacy-regeneration-conflict` | lifecycle, import and artifact digests | `ReturnToAdmittedLegacyImport` |
| `blocker-open` | candidate, issue ids | `ReturnToScopedBatchFix` |
| `round-input-store-full` | active lifecycle ids, configured bound | `ResolveRetentionCapacity` |
| `redaction-contract-violation` | artifact id, field code | `RegenerateBoundedRedactedArtifact` |
| `final-verification-nonunanimous` | candidate, seat ids | `RedispatchFinalVerification` |
| `lifecycle-receipt-invalid` | candidate, receipt id, failed invariant ids | `SatisfyReceiptPrerequisites`, then `RegenerateLifecycleReceipt` |

The risk-operation variants use section 10's more specific table and are part
of this same closed catalog. `ResolveRetentionCapacity` is itself a closed
choice over `CloseOrAbandonNamedLifecycles` and
`RaiseReviewedRetentionBound`; it never deletes ineligible records.

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

- `packages/xtask/src/delivery/` for lifecycle, lineage, scope, severity,
  source, ledger, correction, disposition, judgment, acceptance, migration,
  retention, terminal metric, receipt, seal, and typed remedy contracts;
- `.github/skills/d2b-panel-round/` for automatic discovery, compatibility,
  verification, and artifact generation;
- panel and integrator agents plus `scripts/copilot/check-bindings.mjs` for
  exhaustive discovery and constrained verification without weakening
  read-only bindings;
- Gas City formulas and ADR 0053's controller for protected operations,
  authority resolution, automatic import, retention, and audit;
- generated schemas and fixtures for every new closed type; and
- contributor and delivery documentation only when implementation lands, so
  current docs continue to describe current behavior until then.

The implementation maintains a machine-readable catalog of every invariant
and refusal in this record. Each catalog row names:

- the enforcing code path;
- at least one positive test;
- at least one planted negative that reaches the intended typed refusal rather
  than failing parse first;
- the validation job that executes those tests; and
- any explicit companion command required outside the normal harness.

Coverage fails when a catalog row, positive, or planted negative is missing,
when the corpus is empty, or when a planted negative is accepted. At minimum,
the corpus separately exercises:

- one native discovery and refusal of a second;
- automatic complete ledger generation, duplicate grouping, split, merge, and
  idempotent retry;
- false BLOCKER and MAJOR invalidation, withdrawal, severity correction,
  reporting-seat dissent, and missing independent coverage;
- every disposition and judgment combination, including
  `implementation-self-review`;
- automatic full-ledger per-seat artifacts, missing chunk, stale chunk,
  duplicate chunk, conflicting regeneration, and no hand-authored substitute;
- touched and untouched late findings for every allowed reason, plus refused
  pre-existing MINOR and NIT controls;
- ledger-scoped fixes, unrelated scope expansion, atomic rescope, crash and
  retry, abandonment, and successor import;
- every merge-authority evidence form, same-uid standalone refusal, acceptance
  issue and revocation, expiry at each of verification receipt, lifecycle
  receipt, seal, publication, and merge eligibility, and candidate or mapping
  mismatch;
- completed, in-flight, partial, retried, duplicate, malformed, and
  already-ingested legacy rounds with arbitrary recommendation strings;
- exact legacy-byte preservation, deterministic source ids, complete automatic
  crosswalk, migration re-triage, and no invented historical severity;
- active-retention refusal, both D17 bounds, digest-only synchronous audit,
  redaction and `Debug` controls, and all three terminal outcomes in complete
  and permitted degraded shapes;
- merge-ready MINOR and NIT treatment, unresolved blocking states, final
  unanimity, and green validation without panel approval; and
- every typed error and both producer-context remedy renderings.

Validation selection is derived at implementation time from
`tests/layer1-jobs.json`; this ADR does not freeze today's job list. A result
whose manifest entry is advisory cannot be cited as evidence.
Fixture-contract coverage is cited from the separate enforcing
`test-fixture-contracts` job rather than a Rust shard. Affected doctests and
`harness = false` companions run explicitly because they are not nextest
surfaces. An applicability record that omits one of those affected companions
is incomplete and blocks the receipt.

The ADR index coverage gate remains required. Evidence supplied for this
revision is:

```
make test-adr-index-coverage
PASS: 53 ADR files indexed in README.md
```

That authoring evidence does not satisfy any future implementation obligation.

## Consequences

The expected gain is fewer panel executions: native discovery happens once,
legacy work is imported instead of discarded, fixes are batched,
implementation catches mistakes before reviewers return, and ordinary
pre-existing MINOR and NIT findings cannot reopen discovery.

The initial panel becomes more demanding. Exhaustiveness cannot be proven.
Explicit prompts, complete raw-output retention, no truncation, late-finding
metrics, and the late ledger make misses visible rather than pretending they
cannot happen.

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
evidence capable of closing existing work.

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
trusted tooling validates.

### Let the integrator adjudicate a false finding or lower its severity

Rejected. The party implementing the fix cannot be the independent authority
that declares the finding false. Final-roster judgments clear false findings;
reporting seats alone authorize severity correction.

### Keep per-seat independent ledgers

Rejected. Duplicate reports become duplicate obligations with contradictory
state. One ledger retains every source and reporting-seat obligation.

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
