---
name: d2b-panel-round
description: Run the standard Copilot Discover-Fix-Verify panel using the selected roster, one comprehensive discovery, a shared ledger, and scoped verification.
user-invocable: true
---

# Panel lifecycle

This skill runs one lifecycle, not a sequence of open-ended rediscovery
rounds. The lifecycle has one comprehensive discovery, a batch of fixes, and
scoped verification. Its roster starts with discovery selection and can only
widen when a full candidate or fix delta triggers another seat.

Usage:

```
/d2b-panel-round plan
/d2b-panel-round work
/d2b-panel-round adr <path>
```

The plan, work, and ADR entrypoints all use the same lifecycle helper. No
service, daemon, broker, socket, principal, authority protocol, signature, or
runtime surface is involved.

## Authoritative selection and dispatch table

`.github/skills/d2b-panel-round/selection-table.json` is the version-2 source
for seat classes, floors, fill order, focus, triggers, and profiles. The
orchestrator selects the roster from that table. Agents do not select their
own relevance.

| Seat | `agent_type` | `model` | `reasoning_effort` | `context_tier` | `communication` |
|---|---|---|---|---|---|
| software | `panel-software` | `gpt-5.6-sol` | `xhigh` | `default` | `caveman-full-optional` |
| test | `panel-test` | `gpt-5.6-sol` | `xhigh` | `default` | `caveman-full-optional` |
| product | `panel-product` | `gpt-5.6-sol` | `xhigh` | `default` | `caveman-full-optional` |
| docs | `panel-docs` | `gpt-5.6-sol` | `xhigh` | `default` | `caveman-full-optional` |
| security | `panel-security` | `gpt-5.6-sol` | `xhigh` | `default` | `caveman-full-optional` |
| observability | `panel-observability` | `gpt-5.6-sol` | `xhigh` | `default` | `caveman-full-optional` |
| simplicity | `panel-simplicity` | `gpt-5.6-sol` | `xhigh` | `default` | `caveman-full-optional` |
| reliability | `panel-reliability` | `gpt-5.6-sol` | `xhigh` | `default` | `caveman-full-optional` |
| agentic | `panel-agentic` | `gpt-5.6-sol` | `xhigh` | `default` | `caveman-full-optional` |
| nixos | `panel-nixos` | `gpt-5.6-sol` | `xhigh` | `default` | `caveman-full-optional` |
| networking | `panel-networking` | `gpt-5.6-sol` | `xhigh` | `default` | `caveman-full-optional` |
| kernel | `panel-kernel` | `gpt-5.6-sol` | `xhigh` | `default` | `caveman-full-optional` |
| build | `panel-build` | `gpt-5.6-sol` | `xhigh` | `default` | `caveman-full-optional` |

The current pool has seven mandatory seats. Code and operative configuration
have a floor of ten; documentation-only candidates have a floor of eight.
Every matching optional trigger is selected even when the floor is already
met. Fill order is `reliability`, `agentic`, `nixos`, `networking`, `kernel`,
then `build`. Ambiguity widens selection. Citation-only prose does not trigger
the build seat; an actual build contract or explicit build signal does.

Rust responsibility is a `software` profile. The retired legacy Rust seat
remains readable only while importing historical records; it is not dispatched
by this current table.

<!-- D2B-CAVEMAN-DISPATCH: caveman-full-optional -->
Resolve the caller's communication request before dispatch. Pass explicit
`normal` or `off` unchanged; either overrides optional
`caveman-full-optional`. Communication mode never changes persisted artifacts,
selection, ledger, verdict, or panel JSON.

## Lifecycle artifacts

Create one lifecycle selection for each reviewed candidate state:

```
node .github/skills/d2b-panel-round/scripts/panel-lifecycle.mjs \
  select <candidate.json> <lifecycle-id>
```

The helper renders selection schema version `1` at:

```
.scratch/panel/<lifecycle>/selections/<candidate-id>/<snapshot-sha256>.json
```

Generation is deterministic and create-or-compare. Existing different bytes
are a hard failure. The artifact binds the lifecycle, phase, program, wave,
candidate digest triple, selection-table version, classification inputs,
profiles, and ordered roster.

Verification selections use a phase component when a candidate address would
otherwise collide with discovery state.

Stage the candidate with the same lifecycle and selection:

```
bash .github/skills/d2b-panel-round/scripts/stage-diffs.sh \
  <base> <previous-tip> <round-id> --discovery-request <request.json> \
  --lifecycle <lifecycle-id> --selection <selection.json> \
  --candidate <current-candidate.json> \
  --evidence <finalized-evidence.md> \
  --reviewer-notes-dir <finalized-reviewer-notes>
```

For verification staging, supply the complete canonical handoff instead:

```
bash .github/skills/d2b-panel-round/scripts/stage-diffs.sh \
  <base> <previous-tip> <round-id> \
  --lifecycle <lifecycle-id> --selection <selection.json> \
  --candidate <current-candidate.json> \
  --evidence <finalized-evidence.md> \
  --reviewer-notes-dir <finalized-reviewer-notes> \
  --ledger <discovery-ledger.json> --responses <responses.json> \
  --self-verification <self-verification.json> \
  --verification-dir <verification-requests>
```

`address.json` records `lifecycle_id`. The selection path is passed unchanged
to both consumers:

```
node .github/skills/d2b-panel-round/scripts/make-records.mjs \
  <round-dir> \
  --selection <selection.json> \
  --ledger <round-dir>/discovery-ledger.json \
  --responses <round-dir>/responses.json \
  --verification-results <round-dir>/verification-results.json \
  --approval <round-dir>/approval.json
delivery wave panel-request --selection <selection.json>
```

Both consumers must refuse a candidate, selection schema, selection-table
version, or ordered-roster mismatch. Records are current workspace
schema-version `2` objects with `panel_format_version: 1`.

Staging materializes the supplied exact bytes for `selection.json`,
`current-candidate.json`, `discovery-ledger.json`, `responses.json`, and
`self-verification.json` when those artifacts are supplied. The discovery
request is intentionally different: `--discovery-request` supplies the
generated request before evidence binding, and staging derives the round-local
`discovery-request.json` by preserving that request and appending the exact
`evidence.md` SHA-256 and byte-count descriptor to `validation_evidence`.
Staging reuses an already matching descriptor and rejects a conflicting one.
The staged request is therefore the canonical evidence-bound request; it is
not claimed to be a byte-for-byte copy of the supplied request.

Finalize the non-empty validation evidence before staging and pass it with the
required `--evidence` argument. To supply integrator-authored notes, finalize
them before staging and pass `--reviewer-notes-dir`; that directory must
contain exactly one non-empty regular `<seat>.md` file per selected seat and no
other entries. Omitting the notes argument uses the generated defaults.
Discovery staging additionally requires a readable `--discovery-request`
artifact. Verification staging requires a readable complete per-seat
`--verification-dir`; it also requires the ledger, response, and
self-verification artifacts above.

All canonical artifacts, evidence, reviewer notes, and verification requests
are materialized before the round's `.complete` marker is published. That
marker byte-binds every reviewer-visible artifact. Once `.complete` exists,
the round is immutable: do not edit, replace, delete, or backfill a staged
artifact. Staging may only compare or reuse the existing bytes; changed
evidence or reviewer notes require a new qualified round. The generated
`dispatch-prompt.txt` is usable only when the round's `.complete` marker
exists; an unmarked scratch directory is non-authoritative.

Staging also snapshots each selected
`.github/agents/panel-<seat>.agent.md` at
`agent-definitions/panel-<seat>.agent.md` and byte-binds it in `.complete`.
Both `review-request.md` and `dispatch-prompt.txt` name that immutable copy and
carry the active phase's authoritative verdict contract. Discovery keeps its
bootstrap-compatible four-field schema and explicitly excludes
`verified_issue_statuses` and `late_findings`; verification requires those two
fields in its distinct six-field schema. An inactive-phase output example in
the bound agent definition does not override the staged phase contract.

The phase handoff uses a staged candidate, a discovery request, an immutable
ledger, a response envelope, verification requests, an approval artifact, and
a metrics artifact as its canonical inputs. The public command sequence covers
selection, verdict adaptation, discovery merging, response generation,
verification preparation, approval, metrics, and record generation. Staging
derives changed paths from its git range and records the selection digest beside
the phase and artifact names.

The following is the complete copyable handoff after the verification verdicts
are collected. Keep the ledger at the canonical round path; every later command
must consume that exact path and the exact bytes produced by the preceding
command:

```bash
ROUND=.scratch/panel/<round-id>

node .github/skills/d2b-panel-round/scripts/panel-lifecycle.mjs \
  adapt-verification "$ROUND/discovery-ledger.json" "$ROUND/verdicts" \
  "$ROUND/verification-results.json" \
  --selection "$ROUND/selection.json" --candidate "$ROUND/current-candidate.json"
node .github/skills/d2b-panel-round/scripts/panel-lifecycle.mjs \
  approval "$ROUND/selection.json" "$ROUND/discovery-ledger.json" \
  "$ROUND/responses.json" "$ROUND/verification-results.json" \
  "$ROUND/approval.json" --candidate "$ROUND/current-candidate.json"
node .github/skills/d2b-panel-round/scripts/panel-lifecycle.mjs \
  metrics --selection "$ROUND/selection.json" \
  --ledger "$ROUND/discovery-ledger.json" --responses "$ROUND/responses.json" \
  --verification-results "$ROUND/verification-results.json" \
  --output "$ROUND/metrics.json"
node .github/skills/d2b-panel-round/scripts/make-records.mjs "$ROUND" \
  --selection "$ROUND/selection.json" --ledger "$ROUND/discovery-ledger.json" \
  --responses "$ROUND/responses.json" \
  --verification-results "$ROUND/verification-results.json" \
  --approval "$ROUND/approval.json"
```

The `approval` command exits `0` when the valid artifact is approved, `3`
when the valid artifact is blocked, and `2` for an invalid invocation or
input. Exit `3` is a normal negative gate result, not a tooling failure.

## Discover once

The first staged request gives every selected seat the full candidate, full
context, validation evidence, and seat focus. Generate the request with:

```
node .github/skills/d2b-panel-round/scripts/panel-lifecycle.mjs \
  discovery-request <selection.json> <candidate.json> <request.json>
```

The discovery instruction is comprehensive: spend the effort now, report
every reasonably discoverable actionable finding, and do not save observations
for later rounds. Every seat returns exactly one reviewer verdict with exactly
these four top-level fields:

```json
{
  "engineer": "software",
  "signoff": false,
  "summary": "A source mapping issue was found.",
  "recommendations": [
    {
      "severity": "high",
      "where": "scripts/panel.js:1",
      "what": "A source can disappear.",
      "why": "The ledger would be incomplete.",
      "fix": "Validate source coverage."
    }
  ]
}
```

Each recommendation has exactly `severity`, `where`, `what`, `why`, and
`fix`. Severity is exactly `critical`, `high`, `medium`, or `low`. A
zero-finding verdict uses `signoff: true` and `recommendations: []`; otherwise
`signoff` is false. A missing verdict is an error, never an inferred
zero-finding result. Reviewers do not return `seat`, `complete`, or
`findings`; those are adapter-owned fields.

Adapt the canonical per-seat verdict directory without hand-copying or
aggregating reviewer output:

```bash
ROUND=.scratch/panel/<round-id>

node .github/skills/d2b-panel-round/scripts/panel-lifecycle.mjs \
  adapt-discovery "$ROUND/verdicts" "$ROUND/discovery-results.json" \
  --selection "$ROUND/selection.json" \
  --candidate "$ROUND/current-candidate.json"
```

The directory must contain exactly one regular `<seat>.json` file for every
selected seat and no other entries. Each verdict's declared seat must match
its filename; missing, unselected, mismatched, duplicate, or malformed entries
are errors. A malformed entry error names both its directory and filename.

`adapt-discovery` writes exactly one `d2b-panel/discovery-result` artifact:

```json
{
  "artifact_kind": "d2b-panel/discovery-result",
  "current_candidate": {
    "candidate_id": "candidate-1",
    "content_id": "content-1",
    "program": "SPEC004",
    "snapshot_sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    "wave": "spec004w1"
  },
  "lifecycle_id": "spec004w1",
  "phase": "discovery",
  "results": [
    {
      "complete": true,
      "findings": [
        {
          "attribution": "software",
          "impact": "The ledger would be incomplete.",
          "raw_text": "scripts/panel.js:1: A source can disappear.: The ledger would be incomplete.: Validate source coverage.",
          "recommendation": "Validate source coverage.",
          "seat": "software",
          "severity": "MAJOR",
          "source_id": "software:1",
          "source_ordinal": 1
        }
      ],
      "seat": "software"
    }
  ],
  "schema_version": 1,
  "selection_sha256": "0000000000000000000000000000000000000000000000000000000000000000"
}
```

The example abbreviates `results` to one seat; the real array contains exactly
one result per selected seat in roster order. The shown digest is a shape
placeholder; the adapter writes the SHA-256 of the exact selection file bytes.
It maps `critical`, `high`, `medium`, and `low` to `BLOCKER`, `MAJOR`,
`MINOR`, and `NIT`. It derives `source_id` and `source_ordinal` from
recommendation order, copies `why` to `impact`, copies `fix` to
`recommendation`, and constructs `raw_text` from `where`, `what`, `why`, and
`fix`. The envelope binds the lifecycle, exact selection bytes, and canonical
candidate address. `merge-ledger` validates all three, so an artifact from a
stale same-roster selection is not reusable.

The orchestrator supplies deduplication groups. The lifecycle helper validates
that every source finding maps to exactly one group, then assigns contiguous
stable lifecycle-local identifiers `R1`, `R2`, and so on:

```
node .github/skills/d2b-panel-round/scripts/panel-lifecycle.mjs \
  merge-ledger <selection.json> <discovery-results.json> \
  <dedup-groups.json> <ledger.json> --candidate <current-candidate.json>
```

All source attribution and source-to-issue mappings remain in the ledger.
Identical inputs produce identical bytes and conflicting regeneration is
refused.

Generate the response template and per-seat verification requests from the
same ledger:

```
node .github/skills/d2b-panel-round/scripts/panel-lifecycle.mjs \
  response-template <ledger.json> <responses.json>
node .github/skills/d2b-panel-round/scripts/panel-lifecycle.mjs \
  verification <selection.json> <ledger.json> <responses.json> \
  <self-verification.json> <verification-dir> \
  --candidate <current-candidate.json> \
  --prior-selection <prior-selection.json> \
  --prior-verdicts <prior-round>/verdicts \
  --delta <actual-delta.json>
```

## Fix and verify the ledger

Implementation receives the complete ledger at once. Every issue gets exactly
one disposition from this closed set:

- `Fixed`: changed surface and non-blank evidence;
- `Intentionally rejected`: concrete non-blank justification;
- `Deferred`: concrete non-blank justification;
- `Withdrawn`: verified factual status and non-blank evidence; or
- `Invalid`: verified factual status and non-blank evidence.

Missing responses and disposition-specific justification, evidence, or
factual status fail closed. A `BLOCKER` approves only as Fixed, or factually
verified Invalid or Withdrawn. A `MAJOR` also permits unresolved Intentionally
rejected or Deferred only with this exact plain recorded object:

```json
{
  "accepter": "claimed-repository-user",
  "capacity": "repository maintainer",
  "justification": "Recorded reason for accepting the remaining risk."
}
```

The object has exactly those three fields. Each value is a string;
`accepter` and `justification` are non-blank after trimming; `capacity` is
exactly `repository maintainer` or `merge owner`. It is shape-checked process
data only. No identity lookup, signature, GitHub API, service, or authority is
involved.

Before verification, record tests, lint, formatting, static analysis, build,
uncovered areas, and self-review. Fix scope is the ledger's declared changed
surface. Unrelated delta paths refuse verification and name a new or
explicitly rescoped lifecycle.

Rerun selection over the full current candidate and every fix delta. Union
each result with the prior lifecycle roster. A seat may join, but no seat
leaves.

Verification consumes the full ledger, every response, validation and
self-review evidence, latest delta, full candidate context, and each seat's
prior status. It checks resolution and regressions; it does not reopen
discovery. A late issue is admitted only for an introduced regression, a
previously missed BLOCKER or MAJOR, or an unsafe correctness, security,
data-loss, or reliability condition. Pre-existing MINOR, NIT, style, optional,
and theoretical out-of-scope observations remain non-blocking ledger history.

Every verification result must status every ledger issue exactly once. Only
`resolved` or `verified` is a passing status; `open`, `blocked`, `unresolved`,
`regression`, `accepted`, and every other status keep approval blocked.
The reviewer verdict always includes `verified_issue_statuses` and
`late_findings`, even when the latter is empty; discovery verdicts include
neither field.

The lifecycle records initial and late findings, late BLOCKER and MAJOR
counts, review and implementation iterations, and average fixed issues per
implementation iteration. A zero implementation-iteration denominator is
`0.0`. Metrics never affect approval.

## Legacy continuation

Import is version-first and never rewrites historical bytes:

```
node .github/skills/d2b-panel-round/scripts/panel-lifecycle.mjs \
  import-legacy <legacy-round-dir> [candidate.json] <import.json>
```

Complete legacy ten-seat records become discovery input without rerunning
discovery. Partial records retain every completed source and run one current
discovery. A source identity is the immutable record digest, legacy seat, and
recommendation ordinal. Raw recommendation text and attribution remain
unchanged.

Only an exact bracketed prefix at byte zero maps historical severity:
`[critical]` to `BLOCKER`, `[high]` to `MAJOR`, `[medium]` to `MINOR`, and
`[low]` to `NIT`, with ASCII case folding. Every other spelling is
migration-assigned `MAJOR`. Legacy `rust` remains attributed to `rust` while
current verification responsibility is `software` with the Rust profile.
Current candidate selection is unioned into the imported roster, including
`build` when a build contract is present.

## Dispatch and verdict

Dispatch only the seats in the current selection artifact. Panel agents are
read-only and must inspect staged evidence rather than run validation. The
exact discovery reviewer schema and recommendation schema are defined above.
A verification reviewer uses those same four base fields and adds
`verified_issue_statuses` plus `late_findings`:

```json
{
  "engineer": "software",
  "signoff": true,
  "summary": "What was reviewed and the overall posture.",
  "verified_issue_statuses": {
    "R1": "verified"
  },
  "late_findings": [],
  "recommendations": []
}
```

`verified_issue_statuses` has exactly one entry per ledger issue.
`late_findings` is an array. `signoff` is true if and only if
`recommendations` is empty. Generate records only after every selected seat
has a verdict and observed binding:

```
node .github/skills/d2b-panel-round/scripts/make-records.mjs \
  <round-dir> --selection <selection.json> \
  --ledger <round-dir>/discovery-ledger.json \
  --responses <round-dir>/responses.json \
  --verification-results <round-dir>/verification-results.json \
  --approval <round-dir>/approval.json
```

Record generation also consumes the approval artifact, exact response bytes,
exact adapted verification-result bytes, and immutable discovery ledger before
publication. It requires `--approval` and an explicit canonical `--ledger`
path; no ledger or verification artifact is inferred from an alternate
filename.

The metrics command reads and validates the canonical ledger, response
envelope, current selection, and adapted verification results. Final metrics
require complete verification and mark the output `status: "complete"` with
`degraded: false`; incomplete inputs are refused rather than presented as
complete.

Do not hand-copy findings or responses. Green validation never substitutes for
selected-roster verification.
