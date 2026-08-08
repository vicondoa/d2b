# Panel review

Panel sign-off contract: phase gate, Discover-Fix-Verify lifecycle, selected
roster and focus, and future producer parity.

Binding rules are in [`../../AGENTS.md`](../../AGENTS.md) under "Panel review":
a phase closes only on unanimous sign-off, `signoff` is `true` iff
`recommendations` is `[]`, and green tests never waive the gate. This file
provides the detail.

For the once-per-wave binding panel enforced in code, see
`packages/xtask/src/delivery/panel.rs` and
[`../specs/ADR-046-validation-and-delivery.md`](../specs/ADR-046-validation-and-delivery.md)
section 12.3.

## Phase gate

Multi-phase plans MUST pass a panel sign-off gate at each phase boundary. The
integrator MUST NOT begin the next phase until every selected reviewer returns
`signoff: true` (N/N for the selected roster size). The current role domain is
the thirteen-seat selection table; the lifecycle selection artifact chooses
the ordered roster and the roster may only widen.

For plan-driven work, a "phase" is usually one wave from the plan's graph
(`Wave 0`, `Wave 1`, ...). For plans touching fewer than three files, one phase
covering the whole plan is acceptable.

For each phase:

1. **Plan review** - panel runs one comprehensive discovery over the plan,
   batches any fixes, and performs scoped verification until N/N sign-off.
   The integrator may not dispatch implementation subagents until this gate
   passes.
2. **Implementation** - dispatch subagents in parallel per the
   dependency graph.
3. **Integration** - integrator merges subagent output.
4. **Work review** - panel runs one comprehensive discovery over the
   integrated candidate, batches fixes, and performs scoped verification until
   N/N sign-off.
5. **Advance** - only now may the integrator begin the next phase's
   plan review.

Panel prompts MUST include the validation evidence the integrator already
ran for the phase (commands and pass/fail results) and MUST instruct
reviewers not to rerun tests, builds, evals, or other long validations
unless the integrator explicitly requests that reviewer to do so.
Reviewers should inspect the plan or diff, reason over the supplied
evidence, and call out missing or insufficient validation as a finding
rather than duplicating the validation themselves. This keeps panel
review from stampeding the shared Nix store, cargo target, and git
worktrees while parallel implementation agents are still active.

A verification iteration after discovery MUST carry two explicit ranges:

- `git diff <the commit that reviewer last reviewed>..HEAD` - the fix delta,
  used to verify ledger resolutions and regressions.
- `git diff <base>..HEAD` - the full branch, for context when the delta
  touches something whose correctness depends on code outside it.

The integrator records each reviewed tip so verification can be scoped against
it. A prose summary is intent, not evidence: reviewers inspect the fix delta,
the full candidate, the shared ledger, and the implementation responses.

Where the integrator disputes a finding, the prompt MUST state the rebuttal
and its evidence and ask the reviewer to judge it on the merits - explicitly
permitting withdrawal of an incorrect finding, and explicitly not requiring
it. An unfounded finding drives a wrong change into the tree, so sustaining
one to save face is worse than admitting the error; equally, a reviewer must
not withdraw a valid finding merely because the integrator pushed back.

The Copilot panel stages one canonical `review-request.md` for the whole
selected roster and one `reviewer-notes/<seat>.md` file per reviewer. The request names
the exact delta and full ranges, validation evidence, deliverable, finding
threshold, no-rerun rule, and prior-verdict obligation. The integrator
dispatches the exact generated `dispatch-prompt.txt` to every seat rather than
reconstructing those instructions in free-form prompts. For a later
review, staging fails unless the supplied previous tip matches the prior
`address.json` and every seat's prior verdict is present.

Any content change invalidates sign-off for that candidate. The lifecycle
roster remains selected, may widen, and verifies the new candidate without
running a second discovery.

Selected panel lanes may use optional full transient communication through the
`d2b-caveman` contract. An explicit `normal` or `off` request wins. This is a
communication choice only: reviewers remain read-only, the shared finding bar
stays byte-identical, verdict JSON stays exact, `signoff` still means
`recommendations` is empty, and optional communication never waives or changes
the normal panel gate.

Each engineer returns a JSON sign-off record shaped like:

```json
{
  "engineer": "software",
  "signoff": true,
  "summary": "What was reviewed and the overall posture.",
  "recommendations": []
}
```

By policy, `signoff` is `true` iff `recommendations` is `[]`.
Otherwise, `recommendations[]` carries merge-blocking conditions. Discovery
findings enter the shared ledger, implementation resolves them in a batch,
and the selected lifecycle roster performs scoped verification. Green tests
do not waive this gate; a phase closes only on unanimous sign-off.

## Discover-Fix-Verify lifecycle

ADR 0055 makes one comprehensive discovery the start of a lifecycle. The
orchestrator creates the versioned selection artifact, dispatches every
selected seat with the full candidate and supplied validation evidence, and
requires an explicit complete result from every seat. `{ complete: true,
findings: [] }` is a valid zero-finding result; an absent seat result is not.

Discovery findings are merged into one shared ledger with deterministic `R`
identifiers and complete source attribution. Implementation receives the full
ledger and records exactly one supported disposition, justification, changed
surface, and evidence for every issue. The integrator then reruns selection
over the full candidate and every fix delta, unions each result with the
lifecycle roster, and never removes a selected seat.

Verification receives the complete ledger, all responses, validation and
self-review evidence, the latest delta, full candidate context, and each
reviewer's prior obligations. It checks resolutions, dispositions, evidence,
regressions, and unsafe late `BLOCKER` or `MAJOR` findings. A pre-existing late
`MINOR` or `NIT` remains non-blocking history. Sign-off remains true exactly
when the blocking recommendation list is empty.

The current selection table defines these thirteen seats:
`software`, `test`, `product`, `docs`, `security`, `observability`,
`simplicity`, `reliability`, `agentic`, `nixos`, `networking`, `kernel`, and
`build`. Code and configuration floors are ten, documentation floor is eight,
and ambiguity widens selection. Build-system and build-contract changes
select `build`; citation-only prose does not. Rust review is a `software`
profile. Historical fixed-ten delivery artifacts remain readable separately
and retain `rust`.

The lifecycle selection is the one roster authority:

```text
.scratch/panel/<lifecycle>/selections/<candidate-id>/<snapshot-sha256>.json
```

`stage-diffs.sh`, `make-records.mjs`, and xtask `panel-request` consume the
same artifact. Current delivery request, record, attestation, and embedded
seal panel objects carry `panel_format_version: 1`; legacy fixed-ten objects
omit it. The workspace delivery schema remains version `2`.

## Fix passes are scoped to the ledger

A fix round MUST address the findings the panel actually raised, and
nothing else. Do not take a finding as licence to harden the surrounding
area, add coverage the panel did not ask for, or fix an unrelated defect
noticed in passing. File those separately.

This rule exists because the alternative does not converge. Every
unrequested change is new content, new content invalidates the round's
evidence, and the next round reviews a larger diff that offers more to
find - so the gate recedes while the actual deliverable sits finished and
unmerged. The observed failure mode is a phase gate whose findings drift
from "the specification contradicts the shipped code" to progressively
more peripheral tooling nits, several rounds after the deliverable was
ready.

Two consequences worth stating outright:

- A genuine defect discovered while fixing something else is still out of
  scope for that fix round. Record it and land it separately, so the
  round's diff stays reviewable against the findings it answers.
- An integrator MUST NOT run `git add -A` while a build, test, or gate is
  running. Those write scratch directories into the worktree, and a
  catch-all add commits them. Stage the specific paths the fix touched.
  The gitignore is a backstop, not the control - it can only cover
  scratch patterns someone already thought of.

Panel prompts SHOULD state the phase's deliverable and instruct reviewers
to confine findings to defects in the delta that would cause incorrect
behaviour or mask a regression, rather than proposing speculative
robustness work. A reviewer who wants additional hardening should say so
as an observation in the summary, not as a blocking recommendation.

### The bar is one shared, gate-enforced block

That paragraph is a SHOULD, and for the Copilot panel it is not left to
each prompt author to honour. Every current `.github/agents/panel-*.agent.md`
carries a `## The bar for a finding` section, and
`scripts/copilot/check-bindings.mjs` requires them to be
**byte-identical**. Editing one seat's copy fails `make test-lint` until
the current set matches.

The enforcement exists because the prose version did not hold. The bar
was written once and then restated per seat, and it diverged into ten
different thresholds: two seats carried the full rule, three carried a
partial variant each excluding a different thing, one substituted its own
test, and **four carried no threshold at all**. A seat with no stated bar
treats anything it notices as blocking, and because `signoff` is `true`
iff `recommendations` is `[]`, each of those cost another selected-roster
verification iteration. That is the mechanism behind the drift toward
peripheral nits described above: not reviewers being pedantic, but
reviewers correctly applying ten different thresholds because that is what
they were given.

The block also carries two rules that came out of observed misses in this
repo, and both belong to every seat rather than to one:

- **Report the class, not the instance.** A finding named one substituted
  position; the fix closed exactly that one and left two others, and the
  round after found them. A finding that names the class closes it once.
- **Prose asserting a property is not evidence of it.** A seat that missed
  a real defect explained afterwards that the surrounding prose asserted the
  property held, so it read as established and was not re-checked. Where the
  delta claims a property, check the property.

A change to the bar is a deliberate change to what the panel blocks on.
Make it in every current panel agent file in one commit; the gate will not let
you do otherwise.

Escape hatches are narrow:

- **Future Gas City orchestration** may drive the gate only after it consumes
  the same selection table and lifecycle artifacts and produces the same
  selected-roster result. No smaller council substitutes for the current
  standard Copilot implementation.
- **Trivial fixes** (typo, one-line, no semantic change) may skip the
  panel gate.
- **Time-critical hotfixes** (production breakage) may skip the
  pre-fix panel, but MUST run a post-fix panel before the incident is
  considered closed.
- **Documentation-only changes** may skip the panel gate unless the doc
  change describes a load-bearing behavior.

Autopilot prompts encourage "bias to action." That is in tension with
the panel gate. When in doubt, run the panel. A two-hour panel that
catches one HIGH finding is cheaper than re-doing two days of
integration.

Canonical precedent: an early observability Wave-1 panel returned
0/8 sign-offs with 11 HIGH findings. `tests/static.sh` caught none of
them. This is the canonical "you can't test your way out of needing a
panel" data point.

## Concurrent slices share one worktree, so destructive git is banned

Parallel slices in a wave write to the same checkout. A slice therefore
sees uncommitted files it does not own, and MUST treat them as read-only
evidence rather than as its own stray edits.

Two commands are prohibited inside a slice:

- `git checkout -- <path>` and `git restore <path>` on any path the slice
  does not own. Uncommitted work has no reflog entry and no dangling blob,
  so this is an unrecoverable delete of a sibling's work. If a slice
  believes it dirtied a file it does not own, it MUST report that rather
  than revert it.
- A package-wide or workspace-wide formatter. `cargo fmt -p <pkg>`
  reformats every file in the package, not the slice's file, which makes
  the slice's diff look like it touched files it never opened - and that
  false signal is what motivates the revert above. Format the single file
  instead.

The integrator MUST commit each slice's output as it lands rather than
accumulating several slices' work uncommitted, so a mistake costs one
`git checkout` of committed content instead of a rewrite. Where work is
already lost, check the rebase autostash before concluding it is gone: a
rebase run during the wave captures the whole dirty tree, and that has
already recovered one slice's uncommitted output in this program.

## Selection-table role domain

| Engineer          | Focus |
|-------------------|-------|
| `software`        | Correctness, control flow, error propagation, APIs, unsafe and FFI boundaries, dependency direction, and testability. |
| `test`            | Coverage, invisible regressions, planted negatives, gate placement, and validation evidence. |
| `product`         | Scope, operator UX, naming, migration, contracts, and actionable errors. |
| `docs`            | Documentation placement, changelog and ADR coverage, terminology, links, schema drift, process markers, and ASCII dashes. |
| `security`        | Exploitability, attacker model, authorization, trust boundaries, secrets, and PII. |
| `observability`   | Metrics, spans, logs, audit shape, cardinality, redaction, retention, and diagnostics. |
| `simplicity`      | Reuse, deletion, abstraction count, dependency adoption, indirection, and unnecessary machinery. |
| `reliability`    | Ownership, cleanup, restart and adoption, idempotency, ordering, concurrency, partial failure, and durable state. |
| `agentic`         | Agent profiles, prompt contracts, instruction layering, orchestration, handoffs, and mechanical enforcement. |
| `nixos`           | NixOS module and option semantics, priorities, activation ordering, assertions, and evaluated configuration. |
| `networking`      | Reachability, firewall, address and port allocation, routing, MTU and MSS, and coexistence. |
| `kernel`          | Syscalls, descriptor and lock semantics, signals, mounts, filesystems, races, and kernel-version assumptions. |
| `build`           | Build graphs, CI scheduling, toolchains, targets, hermeticity, caches, dependencies, packaging, and release artifacts. |

Older commits and [CHANGELOG.md](CHANGELOG.md) entries may reference
the historical ten-seat or six-engineer rosters. The selection-table domain
above supersedes them for current work. Legacy delivery artifacts remain
readable under their strict fixed-ten compatibility format, including `rust`.

Host-local roster files under `/etc/nixos/scripts/` are operator
configuration and are out of scope for this repository; keep repo docs
focused on the review contract rather than paydro-specific files.

## Future Gas City parity

Gas City panel orchestration is not implemented in this repository. A future
producer may replace the standard Copilot dispatcher only if it consumes the
same versioned selection table and lifecycle artifacts, produces the same
ordered roster for the same inputs, requires one explicit complete result per
selected seat, and emits the same ledger, response, verification, and delivery
record formats. A five-seat council or other compressed roster is not
equivalent and cannot satisfy the gate.

## Commit-tag mapping

The tag examples in [Commit conventions](#commit-conventions) use this
mapping, and every commit that comes out of a panel-fix round MUST
carry the relevant tag:

- `Wn` = wave / phase number from the plan's parallelization graph
- `Wnfu` = first follow-up round on wave `n` after the first panel
  findings land
- `Wnfu<M>` = follow-up round `M` on wave `n` when a specific
  follow-up round must be named (for example `W5fu1`)
- `CN`, `HN`, `MN`, `LN` = finding ordinal `N`, prefixed by the
  severity letter from the JSON output (`critical` → `C`, `high` →
  `H`, `medium` → `M`, `low` → `L`)

Example: `( W1fu1 H3 )` means "wave 1, follow-up round 1,
addresses finding ranked HIGH-3."

Inline references to a specific commit in prose elsewhere may
use the compact form `(W2fu4 H10)` for readability - that's
shorthand for citing a commit, not the literal trailing tag
that the commit subject must end with. The trailing-tag form
in the commit subject itself always uses the spaced canonical
form (e.g. `... ( W2fu4 H10 )`).

## Tooling note

The panel contract is producer-neutral only at the defined artifact seam. A
future producer must preserve deterministic selection, the complete-result
rule, the shared ledger, scoped verification, unanimity, and both gates.

The in-repo implementation is the Copilot surface: current
`.github/agents/panel-<role>.agent.md` files, one per role domain entry, each
carrying its own domain checklist and `tools: [view, grep, glob]`, driven
by `.github/skills/d2b-panel-round/`. The binding table in that skill is
the tracked, reviewable surface for panel behaviour, and
`scripts/copilot/check-bindings.mjs` enforces that it agrees with both the
agents and the delivery policy constants. See
[copilot-agents.md](./copilot-agents.md). Change those files in the same
commit as any change to this section.

Host-local panel scripts are operator configuration and are not an upstream
d2b implementation or compatibility target.
