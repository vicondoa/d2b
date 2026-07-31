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

One command runs every stage of every wave, including the seal and the memory
fold.

## The binding table

Every dispatch sets all four columns explicitly. An omitted `reasoning_effort`
silently runs the lane at the model default, because a subagent does **not**
inherit the session's effort.

| Role | `agent_type` | `model` | `reasoning_effort` | `context_tier` |
|---|---|---|---|---|
| architect | `d2b-architect` | `claude-opus-5` | `xhigh` | `long_context` |
| implementer | `d2b-implementer` | `gpt-5.6-sol` | `xhigh` | `long_context` |
| integrator | `d2b-integrator` | `gpt-5.6-sol` | `xhigh` | `long_context` |

The ten panel seats have their own table in `.github/skills/d2b-panel-round/SKILL.md`.
`scripts/copilot/check-bindings.mjs` validates both against the agent files
and against the xtask policy constants.

## Preconditions

Autopilot refuses to start unless all of these hold. Each is checked, not
assumed.

1. A spec directory exists with `spec.md`, `plan.md` and `tasks.md`.
2. `plan.md` records a track (see below) in its Constitution Check section.
3. The plan has passed `/d2b-panel-round plan` unanimously. **This is the gate
   that makes the rest safe to leave alone.** Autopilot does not review the
   plan itself; it executes it.
4. The worktree is clean, and it is not `v3` or `main` directly.
5. `node scripts/copilot/check-bindings.mjs` passes.

## Tracks

The track is recorded in the plan, not decided here.

**Track A** is for work that changes an architectural contract: a new broker
op, a wire or schema change, a trust-boundary move, a new persistent root
surface, anything touching the critical-subsystems index. It closes each wave
through the delivery tooling.

**Track B** is for contained work: a bug fix, a docs change, a test addition, a
contained refactor. No wave seal. One panel round on the finished diff, one PR.

A Track B feature that turns out to touch a critical subsystem is caught by
the panel and promoted rather than quietly shipping.

## Wave addressing

Every wave is a **qualified token**, lowercase, program and wave fused:
`spec001w1`, `adr046w1`, `spec001w3fu2`. It appears unchanged in the delivery
CLI, state paths, panel records, checkpoints, memory rows, and commit trailing
tags: `( spec001w1 )`, `( spec001w1fu2 H3 )`.

Legacy bare `W0` through `W8` remains valid and means program `ADR046`. Never
rewrite an existing legacy address.

## The per-wave loop

Track A runs all of it. Track B runs steps 1 through 6, once, then 7 and 8.

**1. Plan the slices.** Read the wave's tasks and the plan's file-ownership
map. Every slice gets a disjoint file list. Two slices that would write the
same file are serialised, not parallelised.

**2. Dispatch implementer lanes.** One `d2b-implementer` per slice, in a single
batch. Each prompt carries the task, the file-ownership list, and the
acceptance criteria. **Commit each slice as it lands**, staging only that
slice's paths. Do not accumulate several slices uncommitted, and never
`git add -A` while a gate is running: gates write scratch directories into the
worktree.

**3. Validate.** Run the smallest targeted command set that covers the change.
Escalate to the full gate only when targeted validation shows it is needed.
Record the exact commands and results; that record becomes the panel evidence.

Two traps to avoid asserting past:
- `test-rust` **excludes** the fixture-dependent contract crate. A green
  `test-rust` does not validate that layer.
- A job marked `"enforcement": "advisory"` in `tests/layer1-jobs.json` may
  legitimately skip, and **an advisory pass is not evidence**. Read the
  manifest; the split changes.

Heavy lanes go through the public gated targets so the two-slot semaphore is
respected.

**4. Panel.** `/d2b-panel-round work`. Ten read-only lanes on a staged diff.
Record the tip commit reviewed, because the next round is a delta against it.

**5. Fix.** If any seat returned findings, dispatch fix lanes **scoped strictly
to those findings**. A genuine defect found while fixing something else goes
to `/d2b-memory record`, not into this round. Revalidate, then run another
round. Any content change invalidates every prior sign-off in the phase.

**6. Advance only on unanimous panel plus green enforcing validation.**
Otherwise park.

**7. PR.** Push the branch, open the PR to `v3`. The body records the change,
the validation evidence, and substantive review outcomes. No AI, tool, or
model attribution anywhere.

**8. Merge.** See below.

**9. Seal.** `/d2b-wave-delivery seal <wave>`. Track A only.

**10. Record wave memory**, write the checkpoint, advance.

After the last wave, `/d2b-memory fold` and file the low-priority friction as
issues.

## One PR per wave, and why

The delivery tooling requires every item in the current wave to be merged
before a seal, and every prior wave to be merged before the next wave can open
a panel request. So wave N+1 cannot start its gate until wave N has merged.
Running every wave and raising one PR at the end fails at the first seal. This
is forced by the tooling, not chosen.

## The merge stop

`v3` is protected and the merge is the point of no return, so by default
autopilot pushes, opens the PR, waits for checks, and then **parks** with the
PR link, the check status and the panel verdict. The operator merges.
Autopilot resumes at the seal.

That is the right place for a person to be in the loop: the panel verdict and
the CI result are already in front of them.

`--auto-merge` raises the ceiling for a genuinely unattended run: once
required checks pass and the panel was unanimous, `gh pr merge --auto --squash`.
Off by default, because the repository's own convention is that the integrator
owns merge order and conflict resolution.

## Stopping

**Autopilot terminates on a mechanical condition, never on judgement.** The
documented top failure mode for autonomous coding agents is stopping at "looks
done", and per-step reliability compounds badly across a long run.

Terminate only when **all** of these hold:

- every `tasks.md` item is checked;
- the relevant **enforcing** Layer-1 jobs are green (never an advisory job);
- the worktree is clean and every slice is committed;
- every wave is merged, and for Track A, sealed.

Everything else is an **escalation**: pause, write the checkpoint, report the
reason, and stop. Escalate on:

- unresolved panel findings after the round budget;
- an enforcing gate still failing after a bounded number of attempts;
- ambiguous merge state, a conflict, or a failed rebase;
- a slice reporting a scope conflict, or a foreign file dirtied;
- a spec semantic that is missing or contradicts the plan;
- discovering mid-run that an architectural decision is needed. Park, record
  it, and let the operator run `/d2b-adr`.

An escalation never guesses. Reporting a blocker costs a message; guessing
wrong costs a wave.

## Complexity tiers

Budget the run so a small task is not over-resourced.

| Tier | Shape | Tool-call budget | Validation |
|---|---|---|---|
| trivial | one file, no behaviour change | 10 | targeted test or lint only |
| contained | one crate or module, tests included | 40 | that crate's tests |
| structural | multiple crates, schema or contract touched | 120 | targeted plus the enforcing lane that covers it |

Exceeding a budget is an escalation, not a licence to continue. It usually
means the task was mis-sized in the plan, which is information worth
surfacing.

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

A days-long run will cross the context budget. Wave boundaries are the
designed handoff points; `--resume` reads the latest checkpoint rather than
re-deriving state from the worktree.

Checkpoints carry addresses and outcomes only. No transcripts, no diffs, no
validation output beyond pass or fail, no store paths, no credentials.

## Headless

`scripts/copilot/autopilot.sh` runs this outside an interactive session with
the flags, ceilings and log directory pinned. Interactive work needs no
special launcher: per-lane binding works inside the ordinary session.
