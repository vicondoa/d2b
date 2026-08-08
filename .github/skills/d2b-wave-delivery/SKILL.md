---
name: d2b-wave-delivery
description: Drive one d2b wave through the delivery gate - snapshot, validate-import, panel-request, panel-attest, seal, merge-target, merge-eligibility. Use to close a Track A wave, or to drive a single stage by hand for a parked or resumed run.
user-invocable: true
---

# Wave delivery

The binding gate. `d2b-panel-round` produces the verdicts; this skill binds
them to an immutable candidate and seals the wave.

```
/d2b-wave-delivery snapshot <wave>
/d2b-wave-delivery attest <wave>
/d2b-wave-delivery seal <wave>
/d2b-wave-delivery status <wave>
```

Autopilot runs these itself. They are exposed individually for parked or
resumed runs that drive one stage by hand.

## Wave identity

A wave is addressed by a **qualified token**: lowercase, program and wave
fused, no separator.

```
adr046w1      spec001w1      spec001w3fu2
```

The program is part of the token, not a path component: the delivery state
layout is `<state root>/<wave>/<candidate id>/...`, and the program is **not**
one. With two programs, each `w1` would name the same state directory. Fusing
them makes uniqueness intrinsic and survives use in artifact references,
commit subjects, panel records, or checkpoints.

**The legacy form keeps working, indefinitely.** `--program ADR046 --wave W1`
is valid, not deprecated or warned on, and has no timer. A bare `W0` through
`W8` still means program `ADR046` and writes to its existing state directory.
Existing snapshots, seals, records, and history proofs are never moved or
re-addressed, because re-addressing a wave would invalidate their candidate
digests. Only **new** programs use the qualified form.

A qualified token whose embedded program disagrees with explicit `--program` is
rejected as inconsistent.

## The command surface

Run from the repository root:

```
cargo run --manifest-path packages/Cargo.toml -p xtask -- delivery wave <stage> ...
```

Stages, in workflow order: `snapshot`, `validate-import`, `panel-request`,
`panel-attest`, `seal`, `merge-target`, `merge-eligibility`.

## Order is forced, not chosen

Read this before planning a wave; it determines the PR shape:

- `seal` requires **every work item in the current wave** to be merged.
- The wave exit boundary, covering the panel request, the seal, and merge
  eligibility, requires **every prior wave** to be merged.

Thus wave N+1 cannot open a panel request until wave N merges. Running every
wave and raising one PR at the end fails at the first seal.
The per-wave order is:

```
implement -> validate -> panel -> fix -> commit -> push -> PR -> CI -> merge -> seal
```

Track B work has no seal, so it is genuinely one PR for the whole feature.

## Procedure

### 1. Snapshot

Bind the wave's base and head commits into one immutable candidate. Downstream
steps use this address. Record the `candidate_id`, `content_id`, and
`snapshot_sha256` in `.scratch/panel/<round>/candidate.json` so the panel
record helper can join verdicts.

A content change after snapshot invalidates every wave record and requires a new
snapshot. This is mechanism, not policy: no override, force flag, or partial
pass exists.

The one exception is a **history-only rebase**. Review survives when content is
provably unchanged and matches on content identity rather than the full digest
triple. Validator evidence uses the opposite rule.

### 2. Panel request and selected lifecycle

`panel-request --selection PATH` requires the candidate-bound lifecycle
selection and stores its exact ordered roster, provider, model, and reasoning
effort. Then run `/d2b-panel-round work` against the same candidate: one
comprehensive discovery, one shared ledger, batch implementation responses and
self-verification, and scoped verification. The lifecycle roster may widen
for a fix delta but never narrows.

### 3. Attest

`panel-attest` validates a directory with exactly one strict current record per
role stored in the request, all bound to the same candidate. It enforces
unanimity for exactly that roster, `signoff` true iff `recommendations` is
empty, distinct provenance per seat, and pinned provider, model, and reasoning
effort. Current artifacts carry `panel_format_version: 1`; legacy fixed-ten
artifacts omit it and retain `rust`.

The panel model is deliberately not the coding model, so a lane cannot both
author a change and attest to it.

### 4. Seal, then merge eligibility

Seal after the wave's items are merged. Then `merge-eligibility` for the exit
boundary.

## What this skill does not do

It does not merge. `v3` and `main` are protected; merge is the point of no
return and belongs to a person with the panel verdict and CI result visible.

It does not render record content to stdout. Provider, model and
reasoning-effort fields live only inside the external delivery-state
directory and are deliberately kept out of Git, PR bodies and release
archives.
