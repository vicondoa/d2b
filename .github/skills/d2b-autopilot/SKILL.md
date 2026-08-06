---
name: d2b-autopilot
description: Run a completed d2b plan end to end, unattended. Executes every wave - implement, validate, panel, fix, commit, PR, merge, seal, memory - and stops only on a mechanical condition. Use once a spec and plan exist and the plan has passed its panel gate.
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

The ten panel seats have their own table in
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

Track A runs all steps. Track B runs steps 1 through 6 once, then 7 and 8.

**1. Plan the slices.** Read the wave tasks and plan's file-ownership map. Give every
slice disjoint files; serialize slices that would write the same file.

**2. Dispatch implementer lanes.** Send one `d2b-implementer` per slice in one
batch. Each prompt carries the task, file-ownership list, and acceptance criteria.
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

**4. Panel.** `/d2b-panel-round work`: ten read-only lanes on a staged diff.
Record the reviewed tip for the next delta round.

**5. Fix.** If a seat returns findings, dispatch fix lanes **scoped strictly
to those findings**. A genuine defect found while fixing something else goes to
`/d2b-memory record`, not this round. Revalidate and run another round. Any
content change invalidates every prior phase sign-off.

**6. Advance only on a unanimous panel and green enforcing validation.**
Otherwise park.

**7. PR.** Push the branch and open the PR to `v3`. Record the change,
validation evidence, and substantive review outcomes; no AI, tool, or model
attribution.

**8. Merge.** See below.

**9. Seal.** `/d2b-wave-delivery seal <wave>`. Track A only.

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

`--auto-merge` permits an unattended run after required checks pass and the
panel is unanimous: `gh pr merge --auto --squash`. It is off by default because
the integrator owns merge order and conflict resolution.

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
