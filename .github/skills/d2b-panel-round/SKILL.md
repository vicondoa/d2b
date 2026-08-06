---
name: d2b-panel-round
description: Run one d2b panel review round. Stages the delta and full diffs, dispatches all ten reviewer seats as independent read-only lanes pinned to the panel model and effort, collects verdicts, and renders the round report. Use for a plan gate, a wave work gate, or an ADR review.
user-invocable: true
---

# Panel round

One ten-role panel gate round. A phase closes only on unanimous sign-off:
`signoff` is `true` **iff** `recommendations` is `[]`.

Usage:

```
/d2b-panel-round plan                 review the plan, before any implementation
/d2b-panel-round work                 review the integrated diff for a wave
/d2b-panel-round adr <path>           review an ADR draft
```

## The binding table

**This table is the configuration.** Every dispatch sets all four columns
explicitly. It is committed here for review against
`packages/xtask/src/delivery/model.rs`.

| Seat | `agent_type` | `model` | `reasoning_effort` | `context_tier` | `communication` |
|---|---|---|---|---|---|
| software | `panel-software` | `gpt-5.6-sol` | `xhigh` | `default` | `caveman-full-optional` |
| test | `panel-test` | `gpt-5.6-sol` | `xhigh` | `default` | `caveman-full-optional` |
| nixos | `panel-nixos` | `gpt-5.6-sol` | `xhigh` | `default` | `caveman-full-optional` |
| networking | `panel-networking` | `gpt-5.6-sol` | `xhigh` | `default` | `caveman-full-optional` |
| security | `panel-security` | `gpt-5.6-sol` | `xhigh` | `default` | `caveman-full-optional` |
| rust | `panel-rust` | `gpt-5.6-sol` | `xhigh` | `default` | `caveman-full-optional` |
| product | `panel-product` | `gpt-5.6-sol` | `xhigh` | `default` | `caveman-full-optional` |
| docs | `panel-docs` | `gpt-5.6-sol` | `xhigh` | `default` | `caveman-full-optional` |
| observability | `panel-observability` | `gpt-5.6-sol` | `xhigh` | `default` | `caveman-full-optional` |
| kernel | `panel-kernel` | `gpt-5.6-sol` | `xhigh` | `default` | `caveman-full-optional` |

**Never omit a parameter.** A subagent does not inherit session effort. An
omitted `reasoning_effort` silently uses the model default, `medium`, while the
record attests `high`: a plausible false attestation rather than an error.

Legacy records from `gemini-3.1-pro-preview` at `high` remain readable as an
exact compatibility pair. Never dispatch a new lane on it or mix one member
with the current binding.

`scripts/copilot/check-bindings.mjs` validates this table against agent files
and xtask policy constants. Run it after editing either.

<!-- D2B-CAVEMAN-DISPATCH: caveman-full-optional -->
Resolve the caller's communication request before dispatch. Pass explicit
`normal` or `off` unchanged; either overrides optional
`caveman-full-optional`. Do not score brevity or claim compressed wording in a
verdict or report.

## Procedure

### 1. Establish the round address

Every round uses a qualified wave token, lowercase, program and wave fused:
`adr046w1`, `spec001w1`, `spec001w3fu2`. Legacy bare `W0` through `W8` remain
valid for program `ADR046`; do not rewrite them.

Set `ROUND` to the qualified token plus the round ordinal, for example
`spec001w1-r2`.

### 2. Stage the evidence

```
bash .github/skills/d2b-panel-round/scripts/stage-diffs.sh <base> <prev-tip> <ROUND>
```

`<base>` is the branch base; `<prev-tip>` is the previous round's reviewed
commit, or the base for round 1. This writes `.scratch/panel/<ROUND>/`
containing `delta.diff`, `full.diff`, `evidence.md`, `address.json`,
`review-request.md`, `dispatch-prompt.txt`, and one file per seat under
`reviewer-notes/`.

For later rounds, the script requires `<prev-tip>` to match the immediately
previous recorded tip and every seat to have a prior verdict. That makes
`delta.diff` evidence rather than a caller-supplied range claim.

Reviewers have no shell. Staging gives ten lanes byte-identical evidence and
keeps them off the shared Nix store, cargo target, and heavy-gate semaphore
while implementation runs.

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

Do not summarise the change and ask reviewers to trust it. A prose summary is
intent; reading the delta catches silent scope changes.

### 4. Collect and record

Each lane returns one JSON verdict object. Write it to
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

That validates every verdict (`signoff` true iff `recommendations` empty, seat
in the closed roster, one record per seat, all ten present, reviewer text within
its length ceilings) and joins it to the candidate address. It takes the
**observed** model and effort and fails closed rather than defaulting to policy,
so a wrongly bound lane cannot be attested as correct.

### 5. Report and route

Render the round report with per-seat verdicts, findings by severity, and this
round's reviewed tip. **Record that tip** for the next delta.

If any seat returned findings, the round did not pass. Land scoped fixes,
rerun the smallest relevant validation, and run another round.

## Rules that bind the integrator, not the reviewers

**Any content change invalidates every prior sign-off in the phase**, including
from untouched seats. They re-report on the delta and may briefly confirm their
area is unaffected.

**A fix round addresses only the findings raised.** A genuine defect found
while fixing something else is out of scope; record it in the memory register
and land it separately. Otherwise each round grows the diff and the gate
recedes while the deliverable sits finished.

**Do not run `git add -A` while a gate is running.** Gates write scratch
directories into the worktree. Stage the specific paths the fix touched.

**Green tests never waive this gate.** The canonical precedent in this repo is
a panel round that returned zero sign-offs with eleven high findings that the
static gate caught none of.
