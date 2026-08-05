---
name: d2b-panel-round
description: Run one d2b panel review round. Stages the delta and full diffs, dispatches all ten reviewer seats as independent read-only lanes pinned to the panel model and effort, collects verdicts, and renders the round report. Use for a plan gate, a wave work gate, or an ADR review.
user-invocable: true
---

# Panel round

One round of the ten-role panel gate. A phase closes only on unanimous
sign-off: `signoff` is `true` **iff** `recommendations` is `[]`.

Usage:

```
/d2b-panel-round plan                 review the plan, before any implementation
/d2b-panel-round work                 review the integrated diff for a wave
/d2b-panel-round adr <path>           review an ADR draft
```

## The binding table

**This table is the configuration.** Every dispatch sets all four columns
explicitly. It is committed here so it is diffable and so a reader can check
it against the policy constants in `packages/xtask/src/delivery/model.rs`.

| Seat | `agent_type` | `model` | `reasoning_effort` | `context_tier` |
|---|---|---|---|---|
| software | `panel-software` | `gpt-5.6-sol` | `xhigh` | `default` |
| test | `panel-test` | `gpt-5.6-sol` | `xhigh` | `default` |
| nixos | `panel-nixos` | `gpt-5.6-sol` | `xhigh` | `default` |
| networking | `panel-networking` | `gpt-5.6-sol` | `xhigh` | `default` |
| security | `panel-security` | `gpt-5.6-sol` | `xhigh` | `default` |
| rust | `panel-rust` | `gpt-5.6-sol` | `xhigh` | `default` |
| product | `panel-product` | `gpt-5.6-sol` | `xhigh` | `default` |
| docs | `panel-docs` | `gpt-5.6-sol` | `xhigh` | `default` |
| observability | `panel-observability` | `gpt-5.6-sol` | `xhigh` | `default` |
| kernel | `panel-kernel` | `gpt-5.6-sol` | `xhigh` | `default` |

**Never omit a parameter.** A subagent does not inherit the session's
reasoning effort. An omitted `reasoning_effort` silently runs the lane at the
model's own default, which is `medium`, while the resulting record would
attest `high`. That is a false attestation on the binding gate, and it
produces a plausible-looking record rather than an error, which is why it is
worth saying twice.

Legacy records from `gemini-3.1-pro-preview` at `high` remain readable as an
exact compatibility pair. Never dispatch a new lane on that binding, and
never mix one member of the legacy pair with the current binding.

`scripts/copilot/check-bindings.mjs` validates this table against the agent
files and against the xtask policy constants. Run it after editing either.

## Procedure

### 1. Establish the round address

Every round is addressed by a qualified wave token, lowercase, program and
wave fused: `adr046w1`, `spec001w1`, `spec001w3fu2`. Legacy bare `W0` through
`W8` remain valid and continue to mean program `ADR046`; do not rewrite them.

Set `ROUND` to the qualified token plus the round ordinal, for example
`spec001w1-r2`.

### 2. Stage the evidence

```
bash .github/skills/d2b-panel-round/scripts/stage-diffs.sh <base> <prev-tip> <ROUND>
```

`<base>` is the branch base, `<prev-tip>` is the commit the previous round
reviewed, or the base again for round 1. This writes
`.scratch/panel/<ROUND>/` containing `delta.diff`, `full.diff`,
`evidence.md`, `address.json`, `review-request.md`,
`dispatch-prompt.txt`, and one file per seat under `reviewer-notes/`.

For a later round, the script fails unless `<prev-tip>` is the tip recorded by
the immediately previous round and every seat has a prior verdict. That check
is what makes `delta.diff` incremental evidence rather than a caller-supplied
claim about the range.

The reviewers have no shell. Staging is what lets ten independent lanes see
byte-identical evidence, and it is what keeps them off the shared Nix store,
the cargo target directory, and the heavy-gate semaphore while implementation
is still running.

Write the integrator's validation evidence into `evidence.md` before
dispatching: the exact commands run and their pass or fail results. State what
was **not** covered too. A reviewer who cannot tell whether the change was
validated is required to raise that as a finding.

Edit `reviewer-notes/<seat>.md` only when that seat needs an integrator
rebuttal or an explicit reviewer-specific validation request. Do not put a
change summary there.

### 3. Dispatch all ten seats in one batch

Dispatch every row of the table in a single response so the lanes run in
parallel. Use the exact contents of
`.scratch/panel/<ROUND>/dispatch-prompt.txt` as every task prompt. Do not
hand-author, summarize, shorten, or supplement reviewer prompts.

The generated `review-request.md` is the complete shared instruction source.
It names the exact delta and full ranges, the validation evidence, the phase
deliverable, the seat-specific notes, the finding bar, the no-rerun rule, and,
after the first round, the prior verdict each seat must verify. The task prompt
has one job: direct the reviewer to that complete request.

Do not summarise the change and ask reviewers to trust the summary. A prose
summary is a statement of intent. A fix that silently touched something the
summary omitted is exactly what a delta review exists to catch.

### 4. Collect and record

Each lane returns one JSON verdict object. Write each to
`.scratch/panel/<ROUND>/verdicts/<seat>.json`.

Then write `.scratch/panel/<ROUND>/observed.json`, recording what each lane
**actually** ran at:

```json
{
  "security": {
    "model": "gpt-5.6-sol",
    "reasoning_effort": "xhigh",
    "run_id": "...",
    "receipt_locator": "github-copilot://..."
  }
}
```

**Take these values from the harness, never from the reviewer.** The
dispatch result reports the model the lane resolved to; that is the
authoritative record. A reviewer's own statement about which model it is
running is **confabulated and must not be used**: in the round that
introduced this skill, five of ten seats named a model other than the one
the harness reported for them, including two that named a different vendor
entirely. Models cannot introspect their own binding. Asking them to is a
plausible-looking source of exactly the false attestation the observed
table exists to prevent.

```
node .github/skills/d2b-panel-round/scripts/make-records.mjs .scratch/panel/<ROUND>
```

That validates every verdict (`signoff` true iff `recommendations` empty,
seat name in the closed roster, exactly one record per seat, all ten present,
reviewer text within its length ceilings) and joins it to the candidate
address to produce attestable records. It takes the **observed** model and
effort as input and fails closed rather than defaulting to the policy string,
so a lane that ran at the wrong effort cannot be attested as if it had not.

### 5. Report and route

Render the round report: per-seat verdict, the finding list grouped by
severity, and the tip commit this round reviewed. **Record that tip**, because
the next round is scoped against it.

If any seat returned findings, the round did not pass. Land scoped fixes,
rerun the smallest relevant validation, and run another round.

## Rules that bind the integrator, not the reviewers

**Any content change invalidates every prior sign-off in the phase**,
including from seats the change did not touch. Those seats re-report, scoped
to the delta, and may confirm briefly that their area is unaffected.

**A fix round addresses only the findings raised.** A genuine defect
discovered while fixing something else is still out of scope for that round;
record it in the memory register and land it separately. Otherwise every round
reviews a larger diff, offering more to find, and the gate recedes while the
deliverable sits finished.

**Do not run `git add -A` while a gate is running.** Gates write scratch
directories into the worktree. Stage the specific paths the fix touched.

**Green tests never waive this gate.** The canonical precedent in this repo is
a panel round that returned zero sign-offs with eleven high findings that the
static gate caught none of.
