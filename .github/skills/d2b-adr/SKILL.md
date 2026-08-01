---
name: d2b-adr
description: Author, review, and land a d2b architecture decision record. Runs standalone - draft, index row, supersession, ten-lane panel, PR. Use when a load-bearing design choice needs recording, before any feature that depends on it starts.
user-invocable: true
---

# Architecture decision record

```
/d2b-adr <the decision to record>
```

This runs to completion on its own. Its output is a merged ADR number.

## Why this is a separate process

An ADR is not a stage inside a feature. An architectural decision usually
outlives the feature that provoked it and often lands before anyone knows
which features will consume it, so coupling its lifetime to a feature branch
is wrong for a document the whole repository reads.

A spec says *what* and *why* in product terms and should not name an
implementation. An ADR decides *how*, and is the thing the `nixos`, `rust`,
`security` and `kernel` seats argue about. So it gets its own run, its own
panel, and its own PR. A feature that needs one cites a merged ADR number the
way it cites any other committed contract. A feature that does not need one
never mentions ADRs at all.

The practical consequence is that autopilot never has to decide whether an ADR
is required. Either the spec cites one, in which case it is already merged and
readable, or the work does not need one. A run that discovers mid-flight that
it needs an architectural decision parks and records it, like any other
blocker.

## Procedure

### 1. Establish that a decision is actually needed

An ADR records a choice that constrains future work and that a reasonable
engineer might otherwise make differently. If the answer is forced, it is
documentation, not a decision. If it only affects one module and can be
changed freely later, it is a code comment.

The bar in this repo is roughly: does this change a contract, a trust
boundary, a persistent surface, a wire or schema shape, or an invariant in the
critical-subsystems index? If yes, it is an ADR.

### 2. Draft

Dispatch `d2b-architect` (`claude-opus-5`, `xhigh`, `long_context`). Number the
ADR one above the highest in `docs/adr/`. File name is
`NNNN-kebab-case-title.md`. Structure follows the existing records:

```markdown
# ADR NNNN: <Title>

- Status: Proposed
- Date: YYYY-MM-DD
- Related: ADR NNNN (short name), ADR NNNN (short name)

## Context

What is true today, what forces the decision, and what constraints are
non-negotiable. State the measured facts, not the assumptions.

## Decision

The choice, stated so it can be checked against code. Numbered items where the
decision has parts, because later records cite them individually.

## Consequences

What this makes easy, what it makes hard, and what it forecloses. Include the
costs honestly; a consequences section with no costs means the alternatives
were not taken seriously.

## Alternatives considered

Each with why it was rejected. This is the section a future reader actually
needs, because the obvious question about any decision is why not the other
thing.
```

Process markers are permitted here. ADRs are dated historical records and may
name the wave or phase that produced a decision.

### 3. Index and supersession

- Add the row to the `docs/adr/README.md` table. **This has a coverage guard**,
  so a missing row fails a gate rather than passing quietly.
- Update any ADR this one supersedes: mark its status and cross-reference the
  new number from it. Supersession that only points forward is half-done; a
  reader arriving at the old record must learn it is superseded.
- Cross-reference from the critical-subsystems index if the decision changes a
  listed subsystem's invariant.

### 4. Panel

An architectural decision earns the full roster. Run
`/d2b-panel-round adr docs/adr/NNNN-*.md`. Ten lanes, `gemini-3.1-pro-preview`
at `high`, read-only.

Panel prompts for an ADR review carry the draft, the records it supersedes or
relates to, and the code the decision constrains. Reviewers judge whether the
decision is correct and whether the consequences section is honest, not
whether the prose is elegant.

Iterate until unanimous. `signoff` is `true` iff `recommendations` is `[]`.

### 5. Land

Set `Status: Accepted` with the date. Add a `changelog.d/<branch>.md`
fragment. Open a PR to `v3`.

Validate before pushing:

```
make check-tier0
make test-changelog
```

The dash scan is the realistic risk in a prose-heavy change: only the ASCII
hyphen may spell a dash, and nine codepoints are banned across every tracked
file.

## Feeding an ADR into execution

Once merged:

```
/d2b-autopilot docs/adr/NNNN-<slug>.md
```

The ADR is read into the spec step as **settled context**, so `/speckit-specify`
and `/speckit-plan` treat its decisions as given rather than reopening them.
