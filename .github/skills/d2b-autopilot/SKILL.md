---
name: d2b-autopilot
description: Run a completed d2b plan end to end, unattended. Executes every wave - implement, validate, feedback, fix, bind, PR, merge, seal, memory - and stops only on a mechanical condition. Use once a spec and plan exist and the plan has passed its panel gate.
user-invocable: true
---

# Autopilot

```
/d2b-autopilot                        from the current spec directory
/d2b-autopilot docs/adr/0047-*.md     seeded from a merged ADR
/d2b-autopilot <free-text goal>       no ADR at all
/d2b-autopilot --resume               continue from the last checkpoint
/d2b-autopilot --auto-merge           opt in to merging without a human stop
```

One command runs every stage of every wave, including seal and memory fold.

## The binding table

Every dispatch sets all four columns explicitly. An omitted `reasoning_effort`
silently runs the lane at the model default, because a subagent does **not**
inherit the session's effort.

| Role | `agent_type` | `model` | `reasoning_effort` | `context_tier` | `communication` |
|---|---|---|---|---|---|
| architect | `d2b-architect` | `gpt-5.6-sol` | `xhigh` | `long_context` | `normal` |
| implementer | `d2b-implementer` | `gpt-5.6-luna` | `max` | `long_context` | `caveman-full-optional` |
| integrator | `d2b-integrator` | `gpt-5.6-luna` | `max` | `long_context` | `caveman-full-optional` |

The thirteen current panel seats have their own selection table in
`.github/skills/d2b-panel-round/SKILL.md`. `scripts/copilot/check-bindings.mjs`
validates both tables against agent files and xtask policy constants.

<!-- D2B-CAVEMAN-DISPATCH: caveman-full-optional -->
`communication` is a dispatch parameter, not a style requirement. Pass an
operator's explicit `normal` or `off` request unchanged; either overrides the
optional `caveman-full-optional` default. Do not score brevity or claim a lane
used compressed wording.

<!-- D2B-FEATURE-ARTIFACT-ROUTING: d2b-spec-edit-exclusive-v1 -->
All writes under an active feature directory, including task checkbox, specification,
plan, checklist, contract, research, data-model, quickstart, and evidence
changes, go through one `/d2b-spec-edit` batch. Initial creation exceptions
remain in the eight `speckit-*` markers; autopilot never edits an existing
feature artifact.

## Preconditions

Autopilot refuses to start unless all of these checked conditions hold.

1. A spec directory exists with `spec.md`, `plan.md` and `tasks.md`.
2. `plan.md` records a track (see below) in its Constitution Check section.
3. The plan has passed `/d2b-panel-round plan` unanimously. **This gate makes
   the rest safe to leave alone.** Autopilot executes the plan; it does not
   review it.
4. The worktree is clean, and it is not `v3` or `main` directly.
5. `node scripts/copilot/check-bindings.mjs` passes.

## Tracks

The track is recorded in the plan, not decided here.

**Track A** changes an architectural contract: a new broker op, wire or schema
change, trust-boundary move, persistent root surface, or critical-subsystems
index entry. It closes each wave through delivery tooling.

**Track B** is contained work: bug fix, docs change, test addition, or contained
refactor. No wave seal; one panel round on the finished diff and one PR.

A Track B feature touching a critical subsystem is promoted by the panel rather
than shipped quietly.

## Wave addressing

Every wave is a **qualified token**: lowercase, program and wave fused:
`spec001w1`, `adr046w1`, `spec001w3fu2`. It appears unchanged in the delivery CLI, state paths, panel records,
checkpoints, memory rows, and commit trailing tags: `( spec001w1 )`,
`( spec001w1fu2 H3 )`.

Legacy bare `W0` through `W8` remains valid and means program `ADR046`. Never
rewrite an existing legacy address.

## The per-wave loop

Track A runs the feedback lifecycle, binding delivery, PR/merge, and seal.
Track B runs one feedback lifecycle followed by one PR/merge and has no seal.

**1. Plan the slices.** Read the wave tasks and plan's file-ownership map. Give every
slice disjoint files; serialize slices that would write the same file.

Prep commits and worktree slice commits are integrated into an owned
feature/integration branch. Never commit, merge, or push them directly to
protected `main` or `v3`; the owned branch reaches those targets only through
the required PR flow.

**2. Dispatch implementer lanes.** Send one `d2b-implementer` task subagent per
slice in one batch from the exact reviewed worktree. Each prompt carries the
task, file-ownership list, and acceptance criteria. Never substitute a
different agent type and never spawn a nested `copilot` CLI session. If the
current session registry cannot supply every selected exact agent definition,
park with a restart-in-worktree instruction before dispatch.
**Commit each slice as it lands**, staging only its paths. Do not accumulate
slices or run `git add -A` while a gate writes scratch.

**3. Validate.** Run the smallest command set covering the change; escalate to
the full gate only when needed. Record exact commands and results as panel
evidence.

Two traps to avoid asserting past:
- `test-rust` **excludes** the fixture-dependent contract crate. A green
  `test-rust` does not validate that layer.
- A job marked `"enforcement": "advisory"` in `tests/layer1-jobs.json` may
  legitimately skip, and **an advisory pass is not evidence**. Read the
  manifest; the split changes.

Heavy lanes use public gated targets so the two-slot semaphore is respected.

**4. Nonbinding Discover-Fix-Verify.** `/d2b-panel-round work`: create one
lifecycle selection, dispatch one comprehensive discovery to its selected
read-only roster, merge the shared ledger, and hand every issue to
implementation with batch response and self-verification templates. Reselect
over the full candidate and every fix delta, unioning the roster without
narrowing it. Record the reviewed tip and supplied evidence for scoped
verification. This is feedback only: do not create the delivery
`panel-request`, final records, or attestation while content may still change.

**5. Continue fixes and verification.** If verification returns findings, run
the exact continuation sequence below. The advance command publishes the
immutable ledger and blank/partial response template; copy that template to a
distinct completed-response file and fill only the copy:

```bash
ROUND=.scratch/panel/<round>
NEXT=.scratch/panel/<next-handoff>

node .github/skills/d2b-panel-round/scripts/panel-lifecycle.mjs \
  advance-verification "$ROUND/selection.json" "$ROUND/discovery-ledger.json" \
  "$ROUND/responses.json" "$ROUND/verification-results.json" "$NEXT" \
  --candidate "$ROUND/current-candidate.json"
cp "$NEXT/responses.json" "$NEXT/responses-completed.json"
# Fill and save only "$NEXT/responses-completed.json".
node .github/skills/d2b-panel-round/scripts/panel-lifecycle.mjs \
  finalize-handoff "$NEXT/discovery-ledger.json" \
  "$NEXT/responses-completed.json" "$NEXT/handoff.json"
FIX_DELTA=.scratch/panel/<next-fix-delta>.json
CURRENT_CANDIDATE=.scratch/panel/<next-current-candidate>.json
NEXT_SELECTION=$(node .github/skills/d2b-panel-round/scripts/panel-lifecycle.mjs \
  select "$CURRENT_CANDIDATE" <lifecycle-id> --phase verification \
  --previous-selection "$ROUND/selection.json" --fix-delta "$FIX_DELTA")
node .github/skills/d2b-panel-round/scripts/panel-lifecycle.mjs \
  verification "$NEXT_SELECTION" "$NEXT/discovery-ledger.json" \
  "$NEXT/responses-completed.json" "$NEXT/self-verification.json" \
  "$NEXT/verification" --candidate "$CURRENT_CANDIDATE" \
  --prior-selection "$ROUND/selection.json" \
  --prior-verdicts "$ROUND/verdicts" --delta "$FIX_DELTA" \
  --handoff "$NEXT/handoff.json"
bash .github/skills/d2b-panel-round/scripts/stage-diffs.sh \
  <base> <previous-tip> <next-round-id> --selection "$NEXT_SELECTION" \
  --candidate "$CURRENT_CANDIDATE" --ledger "$NEXT/discovery-ledger.json" \
  --responses "$NEXT/responses-completed.json" \
  --handoff "$NEXT/handoff.json" \
  --self-verification "$NEXT/self-verification.json" \
  --verification-dir "$NEXT/verification" --lifecycle <lifecycle-id> \
  --evidence <finalized-evidence.md> \
  --reviewer-notes-dir <finalized-reviewer-notes>
```

Never edit `$NEXT/responses.json`; it remains the immutable blank/partial
template. A verification whose prior selection is discovery is the one
marker-free exception. Dispatch fix lanes **scoped strictly to those findings**
as proper task subagents from the exact reviewed worktree. A genuine defect
found while fixing something else goes to `/d2b-memory record`, not this
lifecycle. Revalidate and run scoped verification again. Any content change
invalidates every prior phase sign-off; do not reopen comprehensive discovery.
A blocked continuation requires a fresh self-verification artifact for the new
candidate and an explicit lifecycle when staging. Never use a parent-worktree
or legacy agent definition as a fallback.

**6. Freeze and bind the final delivery packet (Track A only).** After the
feedback lifecycle is unanimously approved and no content-changing fix
remains, create the final
snapshot and candidate-bound selection. Then create the sole delivery
`panel-request`, run `make-records` from that final packet, and run
`panel-attest` against the same final candidate. This feedback approval is
nonbinding and does not authorize a PR merge. Never issue the request before
the content-changing lifecycle; if content changes after a request, stop with
an invalid candidate rather than creating a second request. A successor wave
may do this only after its predecessor has completed the required panel, seal,
and merge.

**7. PR.** Push the owned feature/integration branch and open the PR to the
protected integration target (`v3` or `main`, as applicable). Record the change,
validation evidence, and substantive review outcomes; no AI, tool, or model
attribution.

**8. Merge.** See below.

**9. Seal.** `/d2b-wave-delivery seal <wave>`. Track A only, after the wave's
items have merged through the PR flow.

**10. Record wave memory**, write the checkpoint, advance.

After the last wave, run `/d2b-memory fold` and file low-priority friction as
issues.

## One PR per wave, and why

Delivery tooling requires every item in the current wave to merge before seal and
every prior wave to merge before the next can open a panel request. Thus wave
N+1 cannot start until wave N merges; one PR at the end fails at the first
seal. The tooling forces this order.

## The merge stop

`v3` is protected and merge is the point of no return. By default autopilot
pushes, opens the PR, waits for checks, then **parks** with the PR link, check
status, and panel verdict. The operator merges; autopilot resumes at seal.

That is the right human stop: panel verdict and CI result are already visible.

`--auto-merge` permits an unattended run after the final packet is attested,
required checks pass, and the panel is unanimous:
`gh pr merge --auto --squash`. It is off by default because the integrator owns
merge order and conflict resolution.

## Stopping

**Autopilot terminates on a mechanical condition, never on judgement.**
Stopping at "looks done" is the documented failure mode, and per-step
reliability compounds across a long run.

Terminate only when **all** of these hold:

- every `tasks.md` item is checked;
- the relevant **enforcing** Layer-1 jobs are green (never an advisory job);
- the worktree is clean and every slice is committed;
- every wave is merged, and for Track A, sealed.

Everything else is an **escalation**: pause, write the checkpoint, report the
reason, and stop. Escalate for:

- unresolved panel findings after the round budget;
- an enforcing gate still failing after a bounded number of attempts;
- ambiguous merge state, a conflict, or a failed rebase;
- a slice reporting a scope conflict, or a foreign file dirtied;
- a spec semantic that is missing or contradicts the plan;
- discovering mid-run that an architectural decision is needed. Park, record
  it, and let the operator run `/d2b-adr`.

An escalation never guesses. Reporting a blocker costs a message; a wrong guess
costs a wave.

## Complexity tiers

Budget the run so small tasks are not over-resourced.

| Tier | Shape | Tool-call budget | Validation |
|---|---|---|---|
| trivial | one file, no behaviour change | 10 | targeted test or lint only |
| contained | one crate or module, tests included | 40 | that crate's tests |
| structural | multiple crates, schema or contract touched | 120 | targeted plus the enforcing lane that covers it |

Exceeding a budget is an escalation, not permission to continue. It usually
means the plan mis-sized the task.

## Checkpoints

Write `.scratch/autopilot/<wave>/checkpoint.json` at every wave boundary and
after every panel round:

```json
{
  "wave": "spec001w2",
  "stage": "panel",
  "round": 2,
  "tip": "<sha>",
  "reviewed_tip": "<sha>",
  "tasks_done": ["T012", "T013"],
  "tasks_open": ["T014"],
  "validation": [{"command": "make test-rust", "result": "pass"}],
  "parked_reason": null
}
```

A days-long run crosses the context budget. Wave boundaries are handoff
points; `--resume` reads the latest checkpoint instead of re-deriving state.

Checkpoints carry addresses and outcomes only. No transcripts, no diffs, no
validation output beyond pass or fail, no store paths, no credentials.

## Headless

`scripts/copilot/autopilot.sh` runs this outside an interactive session with
the flags, ceilings and log directory pinned. Interactive work needs no
special launcher: per-lane binding works inside the ordinary session.
