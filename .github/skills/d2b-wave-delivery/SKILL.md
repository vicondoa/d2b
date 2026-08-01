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

Autopilot runs all of these itself. They are exposed individually because a
parked or resumed run needs to drive one stage by hand.

## Wave identity

A wave is addressed by a **qualified token**: lowercase, program and wave
fused, no separator.

```
adr046w1      spec001w1      spec001w3fu2
```

The program is deliberately part of the token rather than a separate path
component, because the delivery state layout is
`<state root>/<wave>/<candidate id>/...` and the program is **not** a path
component. With one program that is harmless; with two, `w1` of each names the
same state directory. Fusing them makes uniqueness intrinsic to the token, so
it survives being copied into an artifact reference, a commit subject, a panel
record, or a checkpoint, none of which have a path structure to lean on.

**The legacy form keeps working, indefinitely.** `--program ADR046 --wave W1`
is valid, is not deprecated, is not warned on, and is not on a timer. A bare
`W0` through `W8` continues to mean program `ADR046` and continues to write to
its existing state directory. Existing snapshots, seals, records and history
proofs are never moved or re-addressed, because re-addressing a wave would
invalidate the candidate digests that bind its records. Only **new** programs
use the qualified form.

A qualified token whose embedded program disagrees with an explicit
`--program` is rejected as the inconsistency it is.

## The command surface

Run from the repository root:

```
cargo run --manifest-path packages/Cargo.toml -p xtask -- delivery wave <stage> ...
```

Stages, in workflow order: `snapshot`, `validate-import`, `panel-request`,
`panel-attest`, `seal`, `merge-target`, `merge-eligibility`.

## Order is forced, not chosen

Read this before planning a wave, because it determines the PR shape:

- `seal` requires **every work item in the current wave** to be merged.
- The wave exit boundary, covering the panel request, the seal, and merge
  eligibility, requires **every prior wave** to be merged.

So wave N+1 cannot even open a panel request until wave N has merged. A design
that runs every wave and raises one PR at the end fails at the first seal.
The per-wave order is:

```
implement -> validate -> panel -> fix -> commit -> push -> PR -> CI -> merge -> seal
```

Track B work has no seal, so it is genuinely one PR for the whole feature.

## Procedure

### 1. Snapshot

Bind the wave's base and head commits into one immutable candidate. Everything
downstream binds to this address. Record the `candidate_id`, `content_id` and
`snapshot_sha256` into `.scratch/panel/<round>/candidate.json` so the panel
record helper can join verdicts to it.

A content change after the snapshot invalidates every record for the wave and
requires a new snapshot. That is the mechanism, not a policy: there is no
override, no force flag, and no partial pass.

The one exception is a **history-only rebase**. The review survives because
the reviewed content is provably unchanged, matched on content identity rather
than on the full digest triple. Validator evidence takes the opposite rule.

### 2. Panel request, then the round

`panel-request` writes the candidate-bound request naming exactly the ten
roles and the required provider, model and reasoning effort. Then run
`/d2b-panel-round work` against the same candidate.

### 3. Attest

`panel-attest` validates a directory holding exactly one strict record per
role, each bound to the same candidate. It enforces: ten of ten, `signoff`
true iff `recommendations` is empty, distinct provenance per seat, and the
pinned provider, model and reasoning effort.

The panel model is deliberately not the coding model, so a lane cannot both
author a change and attest to it.

### 4. Seal, then merge eligibility

Seal after the wave's items are merged. Then `merge-eligibility` for the exit
boundary.

## What this skill does not do

It does not merge. `v3` and `main` are protected and the merge is the point of
no return, so the merge is where a person belongs in the loop, with the panel
verdict and the CI result already in front of them.

It does not render record content to stdout. Provider, model and
reasoning-effort fields live only inside the external delivery-state
directory and are deliberately kept out of Git, PR bodies and release
archives.
