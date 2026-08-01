# Panel review

The panel sign-off contract: the phase gate, how fix rounds are scoped, the
default ten-role roster and each role's focus, and the harness notes for
running the panel under swarm or unattended.

The binding rules are in [`../../AGENTS.md`](../../AGENTS.md) under "Panel
review": a phase closes only on unanimous sign-off, `signoff` is `true` iff
`recommendations` is `[]`, and green tests never waive the gate. This file
carries the detail behind those rules.

For the once-per-wave binding panel enforced in code, see
`packages/xtask/src/delivery/panel.rs` and
[`../specs/ADR-046-validation-and-delivery.md`](../specs/ADR-046-validation-and-delivery.md)
section 12.3.

## Phase gate

Multi-phase plans MUST pass a panel sign-off gate at each phase
boundary. The integrator MUST NOT begin the next phase until every
reviewer on the selected roster returns `signoff: true` (N/N for the
plan's panel size; the default roster below is 10).

For plan-driven work, a "phase" is usually one wave from the plan's
parallelization graph (`Wave 0`, `Wave 1`, ...). For tiny plans that
touch fewer than three files, a single phase covering the whole plan is
acceptable.

For each phase:

1. **Plan review** - panel reviews the plan; iterate until N/N
   sign-off. The integrator may not dispatch implementation subagents
   until this gate passes.
2. **Implementation** - dispatch subagents in parallel per the
   dependency graph.
3. **Integration** - integrator merges subagent output.
4. **Work review** - panel reviews the integrated diff; iterate via
   fix-subagents until N/N sign-off.
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

A panel round after the first is a **delta review**, and its prompt MUST
carry two explicit ranges rather than only the full branch diff:

- `git diff <the commit that reviewer last reviewed>..HEAD` - the delta,
  which is what the reviewer actually reviews. It is the only thing that
  can have introduced a new defect or failed to close an old one.
- `git diff <base>..HEAD` - the full branch, for context when the delta
  touches something whose correctness depends on code outside it.

The integrator therefore MUST record the tip commit each round reviewed, so
the next round can be scoped against it. A prose summary of what changed is
a statement of intent, not evidence: prompts MUST instruct reviewers to read
the delta themselves rather than trust the summary, because a fix that
silently touched something the summary omits is exactly what a delta review
exists to catch. Prompts MUST also instruct reviewers to verify their own
prior findings against the tree by inspection rather than marking them
closed because the prompt says they were fixed.

Where the integrator disputes a finding, the prompt MUST state the rebuttal
and its evidence and ask the reviewer to judge it on the merits - explicitly
permitting withdrawal of an incorrect finding, and explicitly not requiring
it. An unfounded finding drives a wrong change into the tree, so sustaining
one to save face is worse than admitting the error; equally, a reviewer must
not withdraw a valid finding merely because the integrator pushed back.

Any content change to the reviewed tree invalidates every prior sign-off in
that phase, including sign-offs from reviewers whose focus the change did
not touch. Those reviewers still re-report, but their prompt should scope
them to the delta and permit a short confirmation that their area is
unaffected.

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
Otherwise, `recommendations[]` carries the actionable findings. If any
reviewer returns findings, the integrator spawns follow-up
implementation agents, lands the fixes, reruns the tests, and starts
another panel round. Green tests do not waive this gate; a phase closes
only on unanimous sign-off.

## Fix rounds are scoped to the findings

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
each prompt author to honour. Every `.github/agents/panel-*.agent.md`
carries a `## The bar for a finding` section, and
`scripts/copilot/check-bindings.mjs` requires all ten to be
**byte-identical**. Editing one seat's copy fails `make test-lint` until
the other nine match.

The enforcement exists because the prose version did not hold. The bar
was written once and then restated per seat, and it diverged into ten
different thresholds: two seats carried the full rule, four carried a
partial variant each excluding a different thing, one substituted its own
test, and **three carried no threshold at all**. A seat with no stated bar
treats anything it notices as blocking, and because `signoff` is `true`
iff `recommendations` is `[]`, each of those cost a full extra round
across all ten seats. That is the mechanism behind the drift toward
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
Make it in all ten files in one commit; the gate will not let you do
otherwise.

Escape hatches are narrow:

- **Swarm-driven work** satisfies the per-round gate with swarm's
  five-seat phase council instead of a ten-role panel round. See
  [Running the panel under swarm](#running-the-panel-under-swarm). The
  substitution covers only the per-round gate; the binding wave panel is
  untouched.
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

## Default panel

| Engineer          | Focus |
|-------------------|-------|
| `software`        | Shell + Nix shape of every new module, daemon instrumentation, idempotency of sidecars, error handling in metric exporters. |
| `test`            | Coverage of new option schema, vsock CID collision cases, restart-policy gates, manifest schema drift, and what could regress invisibly. |
| `nixos`           | Module wiring, `lib.mkForce` / `lib.mkDefault` correctness, option declarations, systemd unit composition, and activation ordering. |
| `networking`      | Network surface changes, firewall posture across envs, DHCP/DNS regressions, bridge isolation, and routing invariants. |
| `security`        | Attack surface, host-relay trust posture, capability sets / syscall filters, authz boundaries, telemetry-label PII review, and retention defaults. |
| `rust`            | Rust API shape, error propagation, unsafe/FFI boundaries, schema generation, workspace dependency direction, and testability. |
| `product`         | Operator UX, naming surface, migration/deprecation policy, default-off opt-in shape, and actionable error messages. |
| `docs`            | Diataxis adherence in `docs/{reference,how-to,explanation}/`, CHANGELOG entries, schema md↔json drift, and AGENTS.md updates landing with load-bearing changes. |
| `observability`   | Cardinality of metric labels, span attribute hygiene (no secrets/cmd output/store paths), log/audit shape, retention, and dashboard/exporter correctness. |
| `kernel`          | pidfd, cgroup, namespace, mount, signal, ioctl, and filesystem semantics; kernel-version assumptions and Linux API edge cases. |

Older commits and [CHANGELOG.md](CHANGELOG.md) entries may reference
the historical six-engineer security-hardening roster (`nixos`, `rust`,
`software`, `test`, `networking`, `security`) or the earlier
observability-specific roster. The unified default panel above
supersedes both for new work.

Host-local roster files under `/etc/nixos/scripts/` are operator
configuration and are out of scope for this repository; keep repo docs
focused on the review contract rather than paydro-specific files.

## Running the panel under swarm

There are three review surfaces in this repository and they are strictly
ranked. Read this ordering before wiring any harness.

1. **The binding ten-role panel** - `cargo run --manifest-path
   packages/Cargo.toml -p xtask -- delivery wave panel-request` /
   `panel-attest` / `seal`. This is the authority for an ADR 0046 wave.
   It runs **once, at wave close**, against the wave's one immutable
   snapshot, and it is enforced in code by
   `packages/xtask/src/delivery/panel.rs`: exactly one record per role
   for all ten roles, `signoff` true iff `recommendations` is `[]`,
   unanimous ten of ten, every record bound to the same
   `candidate_id`/`content_id`/`snapshot_sha256`, and provider/model/
   reasoning effort pinned to `github-copilot` /
   `gemini-3.1-pro-preview` / `high`. The panel model is deliberately
   not the coding model, so a lane cannot both author a change and
   attest to it. There is no override, no force flag, and no partial
   pass.
   See [`docs/specs/ADR-046-validation-and-delivery.md`](../specs/ADR-046-validation-and-delivery.md)
   section 12.3.
2. **The per-round phase panel** - the [Phase gate](#phase-gate) rule
   above. Where ADR 0046 restricts the *binding* panel to one per wave,
   this rule allows a panel per implementation round. This is the loop
   swarm automates.
3. **Swarm's five-seat phase council** - the per-round gate whenever
   swarm drives the work. It stands in for surface 2 and has no bearing
   on surface 1.

**Swarm runs surface 2, not surface 1.** Under swarm the five-seat
council is the per-round gate: no ten-role panel round is required
between implementation rounds, which is the whole point of running the
harness. Surface 1 is unchanged, because ADR 0046 section 12.3 already
restricts the binding panel to exactly one run at wave close and never
per implementation round. A green phase council is therefore not a
sealed wave, and `phase_complete` passing is not `delivery wave seal`
passing.

**The 10 roles at wave close.** The ten-role roster is no longer run
every round. It runs once, at wave close, to produce the records
surface 1 consumes: dispatch one read-only lane per roster role via
`dispatch_lanes_async`, seeded with that role's focus cell from the
table above plus the integrator's validation evidence. Lanes are
read-only by contract, which keeps them off the shared Nix store, cargo
target directory, and heavy gate semaphore. Lane ids are free-form, so
all 10 roles vote independently and each lane's verdict maps one-to-one
onto a `panel-attest` record.

To keep those records attestable, the reviewing agents must run on the
pinned panel binding. The `panel` entry under `agent` in
`.opencode/opencode.json` pins them to
`github-copilot/gemini-3.1-pro-preview` at reasoning effort `high` and
denies the write, edit, patch, and bash tools, matching the read-only
lane contract above. A lane on any other model produces a record
`panel-attest` will reject, so do not let model fallback silently
downgrade a panel lane, and do not dispatch a panel lane through the
`general` agent - that one is pinned to the coding model
`github-copilot/gpt-5.6-sol` and its records are rejected by design.

**The per-round council, and what it costs.**
`submit_phase_council_verdicts` has a closed five-member roster
(`critic`, `reviewer`, `sme`, `test_engineer`, `explorer`) and
deduplicates by member, so ten distinct votes cannot be cast against it.
Each seat carries the concerns of the roster roles nearest it:

| Seat            | Covers                          |
|-----------------|---------------------------------|
| `reviewer`      | `software`, `rust`              |
| `test_engineer` | `test`                          |
| `sme`           | `nixos`, `networking`, `kernel` |
| `critic`        | `security`, `product`           |
| `explorer`      | `docs`, `observability`         |

A seat MUST NOT return `APPROVE` while any concern it covers is open.
Accept the tradeoff knowingly: five synthesizers can agree where ten
independent reviewers would have dissented, and the observability
precedent above is exactly that failure shape. That is why this council
gates a round and not a wave, and why the ten-role panel still runs
before the seal.

**Verdict rule.** Swarm's default is more permissive than this file: a
`CONCERNS` verdict carrying only MEDIUM/LOW findings still passes. The
repository rule, and the rule `panel.rs` enforces, is `signoff: true`
iff `recommendations` is `[]`. Set
`council.phaseConcernsAllowComplete: false` so `CONCERNS` blocks like
`REJECT`; that is a required part of the project config.

**Gate wiring.** Enable the gates before the QA profile locks
(`set_qa_gates` is ratchet-tighter and rejects all writes once critic
approval or drift evidence locks it):

```
phase_council, final_council, drift_check,
hallucination_guard, critic_pre_plan, sme_enabled
```

`phase_complete` then refuses to close a phase without
`.swarm/evidence/<phase>/phase-council.json`.

**Plan review.** Swarm has no gate that blocks dispatch on a
phase-scoped plan panel; `critic_pre_plan` is a single critic, once,
project-wide. Encode the plan gate as work instead: make task `N.1` of
every phase the plan-review task, declare the plan itself as its
acceptance criteria via `declare_council_criteria`, and give every
implementation task in that phase a `depends` edge on it. Per-task
council then enforces the plan gate before any coder is dispatched.

**Waves and file ownership.** `epic_decide_phase` followed by
`epic_plan_waves` is the direct implementation of the parallelization
graph, and a `declare_scope` call per task is the file-ownership map
described in [Integrator-prep-first pattern](#integrator-prep-first-pattern-w3-onwards).
Record `epic_record_divergence` after each task completes; declared
scope versus files actually touched is calibration data the manual
process never captured.

## Unattended multi-day runs

Long plans are expected to run for days with the operator away. Two
things make that work, and one thing makes "zero interaction"
unachievable.

**Removing the routine prompts.** Set `execution_profile.auto_proceed:
true` on the plan to drop the phase-boundary confirmation, and enable
Full-Auto (`full_auto.enabled: true`, `mode: "supervised"`) so safe
in-scope operations stop asking. Writes to protected paths still route
through the read-only `critic_oversight` agent rather than blocking.

**Escalation is a pause, not a stop-the-world.** Keep
`full_auto.escalation_mode: "pause"` and `full_auto.denials.on_limit:
"pause"`. `terminate` kills a multi-day run outright; `pause` parks it
recoverably, and `.swarm/` state survives process restarts.

**Zero user interaction is not achievable, by design on both sides.**
`full_auto.escalation_mode` admits only `pause` and `terminate`, there
is no autonomous mode, and `council.escalateOnMaxRounds` is declared
but not implemented - exhausting `council.maxRounds` without an
`APPROVE` surfaces a message for the operator and refuses to
auto-advance. Surface 1 is stricter still: a wave cannot seal without
ten human-attested records, so the binding panel is a deliberate
human-in-the-loop stop that no configuration removes. That matches this
file's own rule that green tests never waive the gate. Plan for
**batched escalation**: the run parks on unresolved disagreement,
`/swarm status` reports why, and the operator services the queue when
convenient. Raising `council.maxRounds` to 5 lets more disagreements
self-resolve before parking; it does not remove the park.

**Context.** A days-long session will cross the context budget's
critical threshold. Treat phase boundaries as the handoff points rather
than fighting the guard mid-phase.

**Heavy lanes.** Advisory panel lanes are read-only and take no heavy
gate slot. Any reviewer explicitly asked to run a validation is subject
to the normal two-slot semaphore in [Heavy lanes](#heavy-lanes), and an
unattended run must not exceed it.

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

The panel contract is implementation-neutral: any harness that
preserves the roster, the unanimity rule, the no-rerun discipline, and
the two gates per phase is acceptable.

The in-repo reference implementation is the Copilot surface: ten
`.github/agents/panel-<role>.agent.md` files, one per roster role, each
carrying its own domain checklist and `tools: [view, grep, glob]`, driven
by `.github/skills/d2b-panel-round/`. The binding table in that skill is
the tracked, reviewable surface for panel behaviour, and
`scripts/copilot/check-bindings.mjs` enforces that it agrees with both the
agents and the delivery policy constants. See
[copilot-agents.md](./copilot-agents.md). Change those files in the same
commit as any change to this section.

`.opencode/opencode.json` is the **frozen legacy binding**, retained
byte-identical for the in-flight ADR 0046 program. Its `agent` table pins
`panel` to the reviewing binding and `general`/`explore` to the coding
binding. It is not modified during the overlap, and where the two surfaces
disagree the legacy one wins until the cutover named in
[copilot-agents.md](./copilot-agents.md).

The ADR 0046 program does not run swarm. Where this section describes
swarm's five-seat council, treat it as documenting an available harness
rather than the configuration in use; the per-round gate is run
directly, and the binding wave panel is dispatched as ten read-only
`panel` lanes.

A second, host-local implementation lives in
`/etc/nixos/scripts/panel-review.{md,sh}` and
`/etc/nixos/scripts/panel-aggregate.sh`. That tooling is paydro's
host-specific implementation, not an upstream d2b dependency. In it the
roster is selected per plan via `ENGINEERS_FILE` and each engineer's
focus file comes from `panel-roles/<engineer>.md`.

