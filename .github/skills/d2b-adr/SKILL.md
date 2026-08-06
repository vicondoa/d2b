---
name: d2b-adr
description: Author, review, and land a d2b architecture decision record. Runs standalone - draft, index row, supersession, ten-lane panel, PR. Use when a load-bearing design choice needs recording, before any feature that depends on it starts.
user-invocable: true
---

# Architecture decision record

```
/d2b-adr <the decision to record>
```

This runs to completion on its own and returns the merged ADR number.

## Why this is a separate process

An ADR is not a feature stage. It usually outlives the feature that provoked
it and may land before consumers are known, so tying it to a feature branch is
wrong for a document the whole repository reads.

A spec says *what* and *why* in product terms, not implementation. An ADR
decides *how* and is where the `nixos`, `rust`, `security` and `kernel` seats
argue. It gets its own run, panel, and PR. A feature that needs one cites its
merged ADR number like any committed contract; one that does not never
mentions ADRs.

Thus autopilot never decides whether an ADR is required: either the spec cites
an already merged one, or the work does not need one. A run that discovers a
mid-flight need parks and records it like any blocker.

## Procedure

### 1. Establish that a decision is actually needed

An ADR records a choice that constrains future work and a reasonable engineer
might make differently. A forced answer is documentation; a freely changeable
choice affecting one module is a code comment.

The repo bar is: does it change a contract, trust boundary, persistent surface,
wire or schema shape, or critical-subsystems invariant? If yes, it is an ADR.

### 2. Draft

Dispatch `d2b-architect` (`gpt-5.6-sol`, `xhigh`, `long_context` for the 1M
context tier). Number the ADR one above the highest in `docs/adr/`. The file is
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

Process markers are permitted: ADRs are dated historical records and may name
the producing wave or phase.

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
`/d2b-panel-round adr docs/adr/NNNN-*.md`. Ten lanes, `gpt-5.6-sol` at
`xhigh`, read-only.

ADR review prompts carry the draft, related or superseded records, and the
constrained code. Reviewers judge the decision and honest consequences, not
prose elegance.

Iterate until unanimous. `signoff` is `true` iff `recommendations` is `[]`.

### 5. Land

Set `Status: Accepted` with the date, add a `changelog.d/<branch>.md`
fragment, and open a PR to `v3`.

Validate before pushing:

```
make check-tier0
make test-changelog
```

The prose-heavy risk is the dash scan: only ASCII hyphen may spell a dash, and
nine codepoints are banned in tracked files.

## Feeding an ADR into execution

Once merged, run:

```
/d2b-autopilot docs/adr/NNNN-<slug>.md
```

The ADR is read into the spec step as **settled context**, so `/speckit-specify`
and `/speckit-plan` treat its decisions as given rather than reopening them.
