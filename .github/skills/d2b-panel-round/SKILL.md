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
| software | `panel-software` | `gemini-3.1-pro-preview` | `high` | `default` |
| test | `panel-test` | `gemini-3.1-pro-preview` | `high` | `default` |
| nixos | `panel-nixos` | `gemini-3.1-pro-preview` | `high` | `default` |
| networking | `panel-networking` | `gemini-3.1-pro-preview` | `high` | `default` |
| security | `panel-security` | `gemini-3.1-pro-preview` | `high` | `default` |
| rust | `panel-rust` | `gemini-3.1-pro-preview` | `high` | `default` |
| product | `panel-product` | `gemini-3.1-pro-preview` | `high` | `default` |
| docs | `panel-docs` | `gemini-3.1-pro-preview` | `high` | `default` |
| observability | `panel-observability` | `gemini-3.1-pro-preview` | `high` | `default` |
| kernel | `panel-kernel` | `gemini-3.1-pro-preview` | `high` | `default` |

**Never omit a parameter.** A subagent does not inherit the session's
reasoning effort. An omitted `reasoning_effort` silently runs the lane at the
model's own default, which is `medium`, while the resulting record would
attest `high`. That is a false attestation on the binding gate, and it
produces a plausible-looking record rather than an error, which is why it is
worth saying twice.

`gemini-3.1-pro-preview` supports `low`, `medium` and `high` only. A request
for `xhigh` on this model is invalid, not merely unusual.

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
`evidence.md`, and `address.json`.

The reviewers have no shell. Staging is what lets ten independent lanes see
byte-identical evidence, and it is what keeps them off the shared Nix store,
the cargo target directory, and the heavy-gate semaphore while implementation
is still running.

Write the integrator's validation evidence into `evidence.md` before
dispatching: the exact commands run and their pass or fail results. State what
was **not** covered too. A reviewer who cannot tell whether the change was
validated is required to raise that as a finding.

### 3. Dispatch all ten seats in one batch

Dispatch every row of the table in a single response so the lanes run in
parallel. Each lane's prompt carries:

- the paths `.scratch/panel/<ROUND>/delta.diff` and `full.diff`, and the
  instruction to read them with `view`;
- `.scratch/panel/<ROUND>/evidence.md`;
- for a round after the first, the commit that reviewer last reviewed and the
  instruction that **the delta is what they review**, with the full branch for
  context only;
- the phase deliverable, so findings stay confined to defects in the delta;
- any integrator rebuttal of a prior finding, stated with its evidence, and an
  explicit statement that the reviewer may withdraw an incorrect finding and is
  not required to withdraw a correct one;
- the instruction not to rerun tests, builds, evals, or long validations
  unless this specific reviewer is explicitly asked to.

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
    "model": "gemini-3.1-pro-preview",
    "reasoning_effort": "high",
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

**One carve-out, and it is what makes the gate terminate.** Appending the
closing round's own outcome to `.specify/memory/*.md` does not invalidate that
round. Without this the rule is non-terminating by construction: the last
round's friction can only be written after it closes, writing it would reopen
it, and the round after would have the same problem. The carve-out is narrow
and mechanical. It covers appended rows in `.specify/memory/` only, describing
a round that has already closed, in a commit that touches nothing else. Anything
bundled with it is a content change and reopens the phase, so land the
bookkeeping on its own.

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
