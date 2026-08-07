# ADR 0055: Discover, fix, and verify panel review

- Status: Proposed
- Date: 2026-08-06
- Partially supersedes: [ADR 0053](0053-gascity-contributor-infrastructure.md)
  D7's open-ended review and fix loop, D8's single blocking treatment of every
  recommendation in an admitted final set, and the D21 clauses that reject a
  severity ladder and keep separate per-seat finding state across repeated
  discovery rounds. D21's
  controller-owned roster selection, surface classifier, profile binding,
  reviewer identity, and candidate-bound evidence remain in force.
- Related: [ADR 0048](0048-copilot-native-agent-surface.md), whose
  Copilot-native surface, independent read-only reviewers, pinned bindings,
  helper-assembled records, and staged evidence remain in force. This record
  does not supersede ADR 0048.
- Scope: Panel review lifecycle, finding and final-verdict semantics,
  review evidence, convergence metrics, and migration of panel records.
- Non-scope: Implementing delivery tooling or rewriting contributor process
  documentation in this change.

## Context

The panel currently converges through repeated review, scoped fixes, and
another review. The first round can find real defects that tests miss, but the
same open-ended loop also permits each later round to become another discovery
pass. A candidate can be ready while new MINOR or NIT findings, style
preferences, and optional refactors keep invalidating sign-off.

ADR 0053 D21 improves roster selection and finding continuity. It does not
change the basic loop: a finding produces another content change, every content
change invalidates sign-off, and another panel can discover more pre-existing
findings. Its finding state is also per seat, so duplicate reports are separate
obligations even when they describe one defect.

The committed implementation is narrower than ADR 0053 D21. On this date,
`packages/xtask/src/delivery/panel.rs` still accepts the fixed ten-role roster,
requires `signoff == recommendations.is_empty()`, and admits only a unanimous
set with no recommendations. That passing code is the current behavior. This
record decides the replacement target; it does not describe the target as
already shipped.

The supersession boundary is narrow:

- ADR 0053 D7's repeated check, fix, and re-review loop becomes one discovery
  followed by constrained verification.
- D8's strict record, binding, provenance, and unanimity rules remain. Its
  final gate no longer treats every severity as one undifferentiated blocking
  class and no longer sees only an isolated final record set.
- D21's roster selection, profiles, controller authority, immutable payload
  binding, and reviewer identity remain. Its per-seat `held`,
  `prior_resolutions`, own-findings-only payload, rotation, rejection of a
  four-level severity contract, and no-cross-version-read clauses are replaced
  as stated below. Dependent prototype and acceptance items are superseded only
  where they assert those mechanics.
- ADR 0048 is not superseded. Its Copilot-native authority, read-only
  independent seats, pinned observed binding, helper assembly, and staged
  evidence are unchanged.

The desired process must keep the properties that make the panel trustworthy:
independent read-only reviewers, controller-owned roster selection, pinned
binding and observed attestation, immutable candidate evidence, candidate
digests, reviewer continuity, and the rule that green tests never waive review.
It must change where discovery ends and verification begins.

## Decision

### 1. One lifecycle has one discovery panel

A review lifecycle is identified independently of any one candidate snapshot.
It begins when a candidate first enters panel review and ends at sign-off,
abandonment, or supersession by migration. Content changes create new candidate
snapshots inside that lifecycle; they do not create another discovery phase.

The lifecycle is:

```
implementation
-> implementation self-review
-> discovery panel
-> deduplicated issue ledger
-> batch implementation
-> implementation self-verification
-> verification panel
-> batch fix and verification only for blocking failures or regressions
-> sign-off
```

The discovery panel runs exactly once per lifecycle. A zero-finding discovery
still proceeds through self-verification and verification; it skips a no-op
batch implementation.

### 2. Discovery is comprehensive, parallel, and exhaustive

The controller selects the discovery roster using ADR 0053 D21. Every selected
reviewer receives the full candidate, the immutable staged evidence and
digests, the applicable validation evidence, its controller-bound profile, and
access to repository context through its read-only tools.

Each discovery prompt MUST say all of the following explicitly:

- this is the one comprehensive discovery review;
- review the entire candidate, not only the seat's most obvious files;
- search repository context as needed to test assumptions and local
  invariants;
- work exhaustively rather than stopping after the first few findings; and
- report every actionable finding the reviewer can reasonably identify.

An actionable finding is grounded in a violated requirement, repository rule,
correctness property, or concrete maintainability defect. An unsupported style
preference is not made actionable by labeling it NIT.

There is no lifecycle-wide finding cap. A bounded record format MAY page one
reviewer's output into multiple digest-bound artifacts, but it MUST NOT
truncate findings or instruct a reviewer to stop at a count.

### 3. Findings have one four-level severity contract

Every discovery finding has exactly one severity:

- `BLOCKER`: merging can cause an unsafe or invalid result, including a
  security boundary violation, data loss, required-contract failure, or a
  correctness or reliability failure for which no responsible authority may
  accept the risk.
- `MAJOR`: a material correctness, security, reliability, product-contract,
  migration, or test-coverage defect that must be fixed unless the responsible
  authority explicitly accepts it.
- `MINOR`: a real, bounded defect whose remaining impact does not make the
  candidate unsafe to merge.
- `NIT`: a concrete, actionable local-quality defect with negligible behavior
  or risk impact. Personal taste and optional redesign are not findings.

Every finding MUST explain its impact and give a concrete recommendation. It
also carries the reporting seat and enough location or evidence to identify
the defect. A report missing impact or recommendation is malformed rather than
silently downgraded.

### 4. One deduplicated issue ledger is authoritative

After discovery, all raw findings are merged into one issue ledger. The
integrator proposes duplicate groups and the controller records them. The
ledger MUST account for every raw finding exactly once, either as the primary
report for an item or as a duplicate attribution on that item. A raw finding
cannot disappear during deduplication.

Each unique issue receives the next stable lifecycle identifier, `R1`, `R2`,
and so on. Identifiers are never renumbered, reused, or reassigned. Later
findings append new identifiers after the highest issued identifier. Duplicate
reports retain every reviewer attribution and use the highest reported
severity unless an explicit, audited severity correction records why a lower
severity is correct.

Each ledger item contains at least:

- stable issue id and severity;
- impact and concrete recommendation;
- location or evidence;
- all reporting reviewers and all raw-finding references;
- implementation disposition and justification;
- verification state and the reviewers that verified it; and
- any risk-acceptance or deferral reference.

The ledger and the raw-to-ledger mapping are immutable, append-only evidence.
Corrections append state transitions; they do not rewrite history.

### 5. Implementation receives the whole ledger once per batch

The first implementation pass after discovery is one batch over the complete
ledger, not one fix lane followed by one panel per finding. Parallel fix slices
remain allowed when their file ownership is disjoint, but they receive the
same ledger and integrate into one candidate before verification.

Before verification, every ledger item MUST have exactly one implementation
disposition:

- `Fixed`, with the implementing delta or commit reference;
- `Intentionally rejected`, with a concrete explanation of why the finding is
  incorrect, inapplicable, or not adopted; or
- `Deferred`, with a concrete explanation and a durable follow-up reference.

These are implementation responses, not approval states. In particular,
labeling a BLOCKER `Intentionally rejected` or `Deferred` does not permit
approval, and a MAJOR in either state still needs explicit risk acceptance.
No item may be dropped, left blank, or replaced by prose outside the ledger.

### 6. Self-verification precedes every verification panel

Implementation self-verifies the integrated candidate before the first
verification panel and after every later batch fix. It:

1. selects and runs applicable tests, lint, formatting, static analysis, and
   build commands from repository-supported entry points;
2. records every selected command and result;
3. records each category that was not applicable and why, rather than
   inventing or requiring a tool the repository does not have;
4. self-reviews the latest delta and the full candidate against the same
   rubric the panel receives; and
5. fixes mistakes introduced by the implementation before dispatching
   verification.

Applicability is evidence, not an escape hatch. A required repository gate or
build cannot be marked inapplicable merely because it is expensive. A
self-review finding discovered after the discovery panel is entered in the
late-discovery ledger before it is fixed, so discovery quality metrics cannot
be improved by fixing a miss silently. It is admitted into this lifecycle only
when it satisfies the same closed late-finding reasons as a reviewer finding;
otherwise it is filed outside the lifecycle and cannot delay approval.

### 7. Verification is resolution and regression review

Verification is not a reopened discovery pass. Every verification reviewer
receives:

- the complete prior issue ledger and raw-finding attribution;
- every implementation disposition and justification;
- applicable validation and build evidence;
- the latest fix delta;
- the full current candidate for context;
- the late-discovery ledger; and
- any durable MAJOR risk-acceptance records.

This full-ledger payload narrowly supersedes ADR 0053 D21's rule that a seat
receives only its own open findings. The controller still builds the bounded
payload and binds its digest into the trusted dispatch record.

Reviewers verify ledger resolutions, regressions introduced by fixes,
`Intentionally rejected` dispositions, `Deferred` dispositions, and MAJOR risk
acceptances. Every reviewer attributed to an issue MUST record a resolution
judgment for it. Deduplicated issues reported by several seats therefore retain
all of their original verification accountability while remaining one ledger
item.

Every verification prompt MUST state that it is resolution and
regression-focused, not another comprehensive discovery review. The prompt
MUST include the allowed-late-finding rule below verbatim in meaning.

### 8. Late findings are a closed exception

A verification reviewer may add a new finding only when at least one of these
closed reasons applies:

- `introduced_by_fix`: the implementation or a later fix introduced it;
- `missed_blocker_or_major`: it was present at discovery and is now assessed
  as BLOCKER or MAJOR; or
- `unsafe_to_approve`: correctness, security, data-loss, or reliability risk
  would make approval unsafe.

Verification MUST NOT add a pre-existing MINOR or NIT, style preference,
optional refactor, naming taste, documentation enhancement, theoretical
out-of-scope edge case, or defect in untouched code. Such an observation may
be filed outside this lifecycle and cannot delay its approval.

Every admitted late finding receives the next stable `R` identifier and is
added to both the issue ledger and a late-discovery ledger. The late-discovery
entry carries issue id, severity, reviewer, verification ordinal, allowed
reason, and a concrete explanation of why discovery missed it. The reviewer
field is a panel seat or the reserved `implementation-self-review` value.
Duplicate late reports merge into the existing item without incrementing the
unique late-finding metrics, while retaining attribution.

The controller refuses a late finding without one allowed reason. A reviewer
cannot evade the rule by restating an old MINOR as a new recommendation.

### 9. Roster selection and reviewer continuity remain controller-owned

ADR 0053 D21's pool, mandatory seats, surface classifier, trigger table,
profile activation, floor, over-selection direction, and no-model-selection
rules remain binding.

The lifecycle roster is monotonic:

```
lifecycle_roster =
    discovery_roster
    union select(each later full candidate)
    union select(each fix delta)
```

No discovery reviewer rotates out before sign-off. A specialist newly selected
because of a fix joins verification under the same restrictions as the other
reviewers. Each seat's provider, model, effort, prompt digest, and reviewer
identity are pinned from its first dispatch through lifecycle completion. This
narrowly replaces D21's release and rotation mechanism; the shared ledger
replaces per-seat `held` and `prior_resolutions` state.

Reviewers remain read-only and cannot attest their own authored work. Candidate
content, staged diffs, prompts, ledger versions, implementation responses,
validation evidence, risk acceptances, reviewer outputs, and final receipt are
digest-bound. A content change invalidates final verification sign-off, but it
does not erase the discovery ledger or its stable identifiers.

### 10. Approval is merge-ready, not perfect

A candidate is approved only when all of these are true:

1. every BLOCKER is `Fixed` and verified resolved;
2. every MAJOR is either `Fixed` and verified resolved or has a valid explicit
   risk acceptance;
3. every ledger item has an implementation disposition and the required
   verification coverage;
4. all required applicable validation passes;
5. the build succeeds where applicable;
6. verification finds no regression caused by a fix;
7. the final verification execution introduces no new BLOCKER or MAJOR; and
8. every reviewer on the final lifecycle roster signs off.

Only an unresolved BLOCKER, an unresolved or unaccepted MAJOR, a regression, a
failed required validation, an applicable build failure, or an incomplete
ledger obligation causes another batch implementation and verification loop.
MINOR and NIT items may remain intentionally rejected or deferred when their
justifications and references satisfy the ledger contract.

The responsible authority for accepting a MAJOR is the human who holds merge
authority for the protected target branch. Reviewers, implementation agents,
the integrator, and the orchestrator cannot invent or delegate that authority.
The authority records acceptance through the protected operator decision
surface: the ADR 0053 controller operator endpoint for a Gas City run, or an
equivalent operator-owned delivery input for a standalone run.

A MAJOR risk-acceptance record is durable and auditable. It binds the human
authority identity, timestamp, candidate and lifecycle ids, issue ids, scope,
rationale, follow-up reference, and an expiry or an explicit statement that no
expiry applies. It is digested into the final receipt and rendered in the
trusted review block. Reviewers validate that the record exists, matches the
issue and candidate, and does not claim broader authority than it carries.
They do not approve the risk on the authority's behalf.

Approval means the candidate is safe and complete enough to merge under these
criteria. It does not mean every possible improvement has been made.

### 11. Record-level sign-off stays exact; lifecycle approval gains a receipt

The record invariant is retained with ledger-state filtering:

```
PanelRecord.signoff == PanelRecord.recommendations.is_empty()
```

Final verification remains unanimous over the selected lifecycle roster. The
meaning of `recommendations` in a verification record is narrowed to an
unsatisfied merge-blocking condition under section 10 or a new finding allowed
by section 8. A resolved item, a validly accepted MAJOR, and a justified
non-blocking MINOR or NIT stay visible in the ledger and are not copied into
final blocking recommendations merely to keep them open forever.

Discovery output is evidence, not approval. A discovery record with no
findings does not sign off the candidate; its empty finding list only states
that the reviewer found none during discovery.

The final attestable object is a controller-derived panel lifecycle receipt.
It can exist only after a unanimous final verification and binds:

- the final candidate digests and lifecycle id;
- every roster and trusted dispatch record;
- discovery and verification prompt and payload digests;
- raw discovery records and the deduplicated issue ledger;
- implementation responses and verification judgments;
- validation and build evidence;
- late-discovery records and metrics;
- MAJOR risk-acceptance records; and
- the final per-seat records.

The seal and publication gate validate the receipt, not an isolated final set
that has lost its discovery history. Green tests remain evidence inside this
receipt and never substitute for panel approval.

This supersedes ADR 0053 D8 only where D8 gives every recommendation in the
admitted final set one undifferentiated blocking meaning and where the gate
sees no lifecycle. It preserves strict deserialization, closed enums, distinct
provenance, candidate binding, observed provider/model/effort checks, no
producer-asserted authority, and unanimous final sign-off.

### 12. Metrics have fixed counting semantics

The lifecycle receipt records:

- `initial_findings`: count of unique issue-ledger items originating in the
  discovery panel after deduplication;
- `late_findings`: count of unique issue-ledger items first entered after
  discovery, regardless of whether self-review or verification found them;
- `late_blocker_count`: late findings whose admitted severity is BLOCKER;
- `late_major_count`: late findings whose admitted severity is MAJOR;
- `review_iterations`: one for the admitted discovery panel plus one for each
  admitted verification panel execution, including an execution that blocks
  and excluding preflight failures or retries needed only to complete one
  roster's record set;
- `implementation_iterations`: each post-discovery batch that produces an
  integrated candidate delta and enters self-verification, excluding the
  original implementation and excluding a skipped no-op batch for an empty
  ledger; and
- `average_issues_fixed_per_implementation_iteration`: the number of unique
  ledger items that first reach verified `Fixed` state divided by
  `implementation_iterations`.

An issue is counted once in each origin and severity metric even when several
reviewers report it. If `implementation_iterations` is zero, the average is
defined as `0.0`; it is never NaN, infinity, null, or omitted. Re-fixing a
reopened item does not increment the numerator again.

These are process-quality signals, not approval thresholds. In particular, a
late BLOCKER or MAJOR forces correction and is measured, not suppressed to
protect the metric.

### 13. Cutover is clean for authority and compatible for audit

The implementation that first supports this ADR bumps the delivery schema and
declares a cutover revision. A candidate whose first panel lifecycle request is
created at or after that revision uses Discover -> Fix -> Verify. No new panel
request may be created under the old round workflow after cutover.

An in-flight candidate with old round records does not continue mixing record
semantics. Its old lifecycle is closed as `legacy-superseded`, its exact bytes
and digests are retained, and a new lifecycle starts with one discovery panel
against the latest candidate. Every unresolved legacy recommendation is
imported as a source into the new deduplication input, with an explicit
old-id-to-new-`R` crosswalk, so migration cannot erase a finding.

Completed historical reviews remain valid evidence for the candidate they
sealed and are never reopened, renumbered, or rewritten. Current tooling MUST
provide an audit-only, version-dispatched reader for historical request,
record, attestation, and round-history artifacts. Legacy artifacts may be
rendered and digest-checked; they may not satisfy a new-schema lifecycle or
seal. This audit-only compatibility narrowly supersedes ADR 0053 D21's claim
that no cross-version record is readable, while preserving its prohibition on
cross-version admission.

No historical record receives invented severities, metrics, ledger ids, or
acceptance state. Missing fields render as not recorded under that historical
schema.

### 14. Implementation obligations and affected surfaces

This ADR does not implement the process. The implementation must update, at
minimum:

- `packages/xtask/src/delivery/` for lifecycle identity, severity and ledger
  types, raw-finding coverage, deduplication evidence, roster continuity,
  late-finding admission, metrics, final receipt, historical audit reading,
  seal validation, and typed actionable errors;
- `.github/skills/d2b-panel-round/` for distinct discovery and verification
  staging, generated prompts, observed records, and no-truncation handling;
- `.github/agents/panel-*.agent.md` and
  `scripts/copilot/check-bindings.mjs` for the shared comprehensive-discovery
  and resolution-focused-verification prompt contracts without weakening
  read-only bindings;
- `.github/agents/d2b-integrator.agent.md` and delivery/autopilot skills for
  batch implementation, applicability-driven self-verification, ledger
  dispositions, and the restricted re-verification loop;
- the ADR 0053 controller and Gas City formulas for trusted lifecycle,
  payload, risk-acceptance, and receipt ownership; and
- `AGENTS.md`, `docs/contributing/panel-review.md`,
  `docs/contributing/copilot-agents.md`, ADR 0046 delivery specifications, and
  generated schemas or fixtures that describe the implemented contract.

Tooling MUST mechanically refuse:

- a second discovery panel for one lifecycle;
- a raw finding absent from the ledger mapping;
- a duplicate or renumbered `R` identifier;
- a ledger item without one implementation disposition;
- verification without recorded applicability decisions and required passing
  evidence;
- an ineligible late finding;
- a final receipt with unresolved approval criteria, missing verification
  coverage, non-unanimous final records, or stale digests; and
- a legacy artifact presented as current authority.

Contributor documentation must continue to describe current implemented
behavior until the tooling lands. The ADR may carry these future process
markers; shipped docs must not claim the process exists early.

## Consequences

The expected gain is fewer panel executions: discovery happens once, fixes are
batched, implementation catches its own mistakes before reviewers return, and
verification cannot reopen the candidate for ordinary pre-existing MINOR and
NIT findings.

The initial panel becomes more demanding. Reviewers must inspect the whole
candidate and repository context in one pass, which increases prompt size and
cognitive load. Exhaustiveness cannot be proven mechanically. The controls are
an explicit prompt, no truncation, complete raw-output retention, late-finding
metrics, and a late ledger that makes misses visible rather than hiding them.

Batching findings can create interacting fixes and a larger fix delta. The
applicability-driven self-verification and regression-focused verification
exist for that concrete failure. They do not make a large batch safe by
assertion.

The shared ledger adds controller state and a human deduplication judgment.
Two genuinely different defects could be merged incorrectly. The guard is the
raw-to-ledger mapping, retained reviewer attribution, highest-severity default,
and verification by every attributed reviewer.

The late-finding restriction deliberately leaves some real MINOR and NIT
defects for later work. That is the cost of defining merge-ready rather than
perfect. The unsafe-to-approve exception prevents convergence policy from
silencing a correctness, security, data-loss, or reliability risk.

Human MAJOR acceptance is powerful and can normalize debt if used casually.
Binding it to the protected-branch merge authority, the exact candidate and
issue ids, a durable rationale and follow-up, and the trusted publication block
makes the decision visible and attributable. It does not make the accepted risk
smaller.

Keeping the lifecycle roster monotonic costs reviewer sessions after an
irrelevant seat could previously rotate out. It prevents the cheaper and more
dangerous outcome: changing the surface or deduplicating a finding until the
reviewer who raised it disappears.

Historical audit support adds a read-only compatibility surface. It is
deliberately separate from admission so old records remain understandable
without becoming an authority over the new gate.

The concrete failure this design makes possible is an integrator silently
omitting an awkward finding while constructing the shared ledger. The
raw-finding coverage check catches it before implementation begins. The
concrete convergence failure it prevents is a verification reviewer adding a
pre-existing MINOR or NIT and forcing another full batch; the closed late-reason
enum and controller admission reject that finding from the lifecycle.

## Alternatives considered

### Keep repeated open-ended discovery rounds

Rejected. It preserves the simplest record model, but every fix reopens the
entire discovery surface. Peripheral findings can indefinitely move the gate
after the requested candidate is merge-ready.

### Run only one panel and merge after its fixes

Rejected. A fix can be incomplete or introduce a regression. Self-review is
not independent evidence, so one verification panel is the minimum safe close
of a batched implementation.

### Keep per-seat independent ledgers

Rejected. Duplicate reports become duplicate obligations with different ids
and can receive contradictory dispositions. One deduplicated ledger retains
all attribution while giving implementation and the final gate one state to
account for.

### Permit every late finding indefinitely

Rejected. That is the current open-ended model under a different name. The
closed exception admits fix regressions, missed BLOCKER and MAJOR findings, and
unsafe risks while refusing the classes that do not justify delaying approval.

### Let MINOR and NIT findings block until fixed

Rejected. It would make "merge-ready, not perfect" false and preserve the
iteration problem. They remain durable ledger items and require an explicit
disposition; they do not vanish merely because they do not block.

### Let reviewers accept MAJOR risk

Rejected. Reviewers identify and validate risk; they do not own the protected
branch decision. Giving any reviewer unilateral acceptance authority would
make roster composition an accidental authorization policy and would leave no
durable human accountability.

### Replace unanimous sign-off with a majority vote

Rejected. The process change narrows what verification may block; it does not
weaken the final panel. Every selected reviewer must still report no
merge-blocking recommendation, and the controller independently checks the
ledger and validation criteria.
