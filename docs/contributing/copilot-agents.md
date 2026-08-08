# Copilot agents, skills, and the autopilot process

Canonical definition surface for the repository's ADR, panel, and delivery
process. Everything is committed, so fresh clones behave identically without
local operator settings.

Copilot is the sole supported agent surface. The legacy integration was retired
with its tracked files removed; no second command path remains.

## What is here

```
.github/agents/          16 agents: 3 roles + 13 current panel seats
.github/skills/          d2b-adr, d2b-panel-round, d2b-wave-delivery,
                         d2b-memory, d2b-autopilot, d2b-caveman,
                         d2b-caveman-compress, d2b-spec-edit, plus speckit-*
scripts/copilot/         check-bindings.mjs, autopilot.sh
                         prompt-corpus.mjs and its checked-in manifest
.specify/memory/         deferred-work, friction-log, engineering-debt
```

## The two processes, and why they are separate

**An ADR is its own run.** `/d2b-adr` drafts a record, adds the index row that
has a coverage guard, updates anything it supersedes, runs the selected-roster panel,
and opens a PR. Its output is a merged ADR number. An architectural decision
usually outlives the feature that provoked it and often lands before anyone
knows which features will consume it, so coupling its lifetime to a feature
branch is wrong for a document the whole repository reads.

**A feature run cites merged contracts.** It never contains an ADR stage.
Either the spec cites a merged ADR, or the work did not need one. A run that
discovers mid-flight that it needs an architectural decision parks and records
it, like any other blocker.

## Authoring a feature

Interactive, because this is where judgment belongs. Track A, the full path:

```
/speckit-specify   <what you want built>      -> specs/NNN-slug/spec.md
/speckit-clarify                              -> resolves ambiguities
/speckit-plan                                 -> plan.md, research.md, contracts/
/speckit-tasks                                -> tasks.md, grouped into waves
/speckit-analyze                              -> cross-artifact consistency
/d2b-panel-round plan                         -> selected seats review the plan
```

Track B is spec-kit's documented shorter path and drops `clarify` and `analyze`:

```
/speckit-specify   <what you want built>
/speckit-plan
/speckit-tasks
/d2b-panel-round plan
```

Iterate until the panel is unanimous. **That gate makes the next step safe to
leave alone.**

The `/speckit-*` steps run in the parent session. A session bound to
`gpt-5.6-sol` at `xhigh` with the 1M `long_context` tier already carries the
architect binding. Otherwise dispatch `d2b-architect` explicitly for `specify`
and `plan`.

## Optional Caveman communication

`d2b-caveman` adapts pinned upstream communication rules to Copilot without
Anthropic, Claude CLI, Python, external install, network access, or third-party
content upload. Full communication is optional and applies only to transient
messages in these lanes:

| Surface | Default communication |
| --- | --- |
| `d2b-implementer` | `caveman-full-optional` |
| `d2b-integrator` | `caveman-full-optional` |
| all current `panel-*` seats | `caveman-full-optional` |
| `d2b-autopilot` and `d2b-panel-round` dispatches | `caveman-full-optional` |
| `d2b-architect` and `d2b-spec-edit` | `normal` |

An explicit `normal` or `off` request wins; no gate grades brevity. Persisted
code, commands, paths, identifiers, exact errors, negations, exceptions,
schemas, commits, release notes, contributor docs, ADRs, feature artifacts, and
panel JSON remain normal and exact. The only persisted-prose exception is the
checked-in governed prompt corpus.

The upstream files are provenance only:
`third_party/caveman/v1.10.0/UPSTREAM.json` pins the repository, tag, commit,
and hashes. `d2b-caveman-compress` works only on the manifest corpus, snapshots
under `.scratch/`, uses the current Copilot session, and requires a
side-by-side semantic audit. It never invokes an upstream runtime.

## Feature artifact ownership

`d2b-spec-edit` is user-invocable and dispatches `d2b-architect` at
`gpt-5.6-sol` / `xhigh` / `long_context` with `normal` communication. It
resolves one active directory under `specs/`, snapshots allowed paths, rejects
absolute paths, `..`, symlink escapes, ADRs, source, contributor docs, and
every other path outside that directory, verifies the changed-path set, and
never reverts foreign work. It accepts one batch and returns changed sections,
checklist transitions, changed files, deliberately untouched related files,
and requested validation.

The designated `speckit-*` commands may create only absent artifacts. After a
file exists, all feature-directory writes route through the editor. `clarify`
collects answers and submits one batch; `analyze` is read-only; `implement`
reports checkbox changes; `converge` prepares exact append content; `autopilot`
and memory fold route writes through the editor. No freshness sidecar, digest
chain, or artifact-state file is introduced.

## Prompt corpus

The checked-in manifest is an exact membership list for 35 files: three
`AGENTS.md` files, all eight `docs/contributing/*.md` files, all sixteen
`.github/agents/*.agent.md` files, and all eight `.github/skills/d2b-*/SKILL.md`
files. `prompt-corpus.mjs` verifies frontmatter, headings, fenced blocks, inline code,
links and URLs, list hierarchy and count, table shape, literals, normative
operators and negations, and exact JSON or output examples. It does not grade
style or token reduction. Imported `speckit-*` prose stays uncompressed except
for routing edits.

## Executing it

```
/d2b-autopilot
```

One command runs every stage of every wave, including the seal and the memory
fold. Per wave: dispatch implementer lanes per the file-ownership map, create
one selected-roster lifecycle, run one comprehensive discovery, merge the
shared ledger, hand batch responses and self-verification to implementation,
run scoped verification, then run the wave's validation and delivery gate.
Route only raised findings into scoped fix lanes, revalidate, commit with the
correct trailing tag, push, open the PR, wait for checks, merge, seal, record
wave memory, and advance. Between waves it writes a checkpoint, so `--resume`
continues after a context handoff.

It stops on a mechanical condition, never on judgement.

**One PR per wave, merged before the next wave starts.** This is forced by the
delivery tooling, not chosen: `seal` requires every item in the current wave
to be merged, and the wave exit boundary requires every prior wave to be
merged, so wave N+1 cannot open a panel request until wave N has merged. A
design that runs every wave and raises one PR at the end fails at the first
seal.

**The merge is the one designed stop.** `v3` is protected and the merge is the
point of no return, so autopilot parks with the PR link, the check status and
the panel verdict, and the operator merges. `--auto-merge` removes even that
stop, at the cost of the operator no longer seeing each wave before it lands.

## Wave identifiers

A wave is a **qualified token**: lowercase, program and wave fused, no
separator.

```
adr046w1      spec001w1      spec001w3fu2
```

The program is part of the token rather than a separate path component,
because the delivery state layout is `<state root>/<wave>/<candidate id>/...`
and the program is not a path component. With one program that is harmless;
with two, `w1` of each names the same state directory. Fusing them makes
uniqueness intrinsic to the token, so it survives being copied into an
artifact reference, a commit subject, a panel record, or a checkpoint, none of
which have a path structure to lean on.

A measured side effect: the qualified lowercase form passes the process-marker
scanner cleanly, while a bare `W1` is flagged and survives today only through
a narrow hardcoded exception plus the legacy path allowlist. New work in the
qualified form needs no exception at all.

**The legacy form keeps working, indefinitely.** `--program ADR046 --wave W1`
is valid, is not deprecated, is not warned on, and is not on a timer. A bare
`W0` through `W8` continues to mean program `ADR046` and continues to write to
its existing state directory. No existing snapshot, seal, record or history
proof is moved or re-addressed, because re-addressing a wave would invalidate
the candidate digests that bind its records. Only **new** programs use the
qualified form.

A qualified token whose embedded program disagrees with an explicit
`--program` is rejected as the inconsistency it is. The closed-set property is
preserved rather than loosened: the program component must match a strict
pattern and the ordinal must still be `0` through `8`, so no free-form
operator string can reach a state directory name or structured output.

## How the model binding actually works

This is the part that is easy to get wrong, and the failure is silent.

### Measured behaviour of Copilot CLI 1.0.75

Every claim below was verified against the installed CLI by creating real
files and observing actual behaviour. Several contradict published guidance,
so re-verify on every CLI upgrade.

| Mechanism | Result |
| --- | --- |
| `model:` in agent frontmatter | **Honoured.** |
| `tools:` in agent frontmatter | **Enforced.** A panel agent has no shell. |
| Task-tool `model`, `reasoning_effort`, `context_tier` at dispatch | **Causal.** Per lane, inside one session. |
| `effortLevel:` / `contextTier:` in frontmatter | Warned and ignored. |
| `reasoningEffort:` in frontmatter | **Accepted with no warning, and inert.** |
| `model` in repo-scope `.github/copilot/settings.json` | Not honoured. |
| `subagents` in repo-scope settings | Not honoured; the allowlist excludes it. |
| `--agent <name>` over ACP | **Ignored.** Works only in print mode. |
| Subagent inherits session reasoning effort | **No.** It falls to the model default. |
| Agent with no frontmatter `model`, dispatched bare | Inherits the **parent session's** model. |

### What follows from that

**Dispatch parameters are the binding.** They live in the committed skill
tables, which is why those tables are the configuration rather than
documentation of it.

**Nothing modifies the operator's settings.** Per-lane binding was measured
sufficient with no `subagents` block in either scope.

**Frontmatter `model` is kept even though the tables always pass it**, because
the fallback behaviours differ and one is dangerous. An agent that omits
`model` and is hand-invoked inherits the caller's model, so a panel seat could
run on an unrelated parent binding. An agent that pins it still runs the panel
model and only loses the effort, which the record helper catches. One line per
agent converts a false model attestation into something requiring two
independent mistakes.

**The residual risk is the silent downgrade.** An unpinned panel lane runs at
the model default while a record attests `xhigh`. That produces a
plausible-looking artifact rather than an error, which is the worst shape a
failure can take on an attestation gate. Three layers defend it:

1. the dispatch tables, which make it rarely happen;
2. `scripts/copilot/check-bindings.mjs`, which rejects a mispinned or illegal
   effort before a run;
3. the record helper, which takes the **observed** effort as input and fails
   closed rather than defaulting to the policy string.

New panel work uses `gpt-5.6-sol` at `xhigh`. Existing
`gemini-3.1-pro-preview` records at `high` remain readable as one exact
compatibility pair; mixed model and effort pairs are rejected.

### Running check-bindings

<!-- BEGIN PANEL-PREFLIGHT-COMMAND -->
```
node scripts/copilot/check-bindings.mjs
```
<!-- END PANEL-PREFLIGHT-COMMAND -->

It fails on: an agent with no binding row, an effort or tier a model does not
support, a panel row disagreeing with the delivery policy constants, a seat
missing from the roster or one not in it, a panel agent granted write tools,
any effort-like key in frontmatter, a frontmatter and table model
disagreement, and a repo-scope settings file carrying keys that scope cannot
honour.

<!-- BEGIN PANEL-PREFLIGHT-NOTICE -->
> **Future, not yet implemented.**
> [ADR 0053](../adr/0053-gascity-contributor-infrastructure.md) proposes
> replacing the operator spelling with a single `make panel-preflight` target
> that runs this checker plus a standalone harness receipt resolver and version
> check. That target does not exist yet. Until it lands, run the node command
> above. Whoever adds the target removes this notice and updates the command
> above in the same commit; leaving either half behind points contributors at
> the wrong preflight. The markers around this notice and the command block
> above are what that commit's policy lint reads; keep them.
<!-- END PANEL-PREFLIGHT-NOTICE -->

## Panel seats

The current thirteen seats each have a domain checklist anchored to this
repository's invariants. Selection is deterministic and request-bound; a
current panel excludes `rust`, whose depth is a `software` profile. They are
**read-only by construction**:
`tools: [view, grep, glob]` removes shell entirely, so they cannot run a
build. That is better than instructing them not to, and it keeps selected lanes off
the shared Nix store, the cargo target directory, and the heavy-gate
semaphore while implementation is still running.

Evidence is pre-staged so every reviewer in a round provably sees
byte-identical bytes:

```
bash .github/skills/d2b-panel-round/scripts/stage-diffs.sh <base> <prev-tip> <round> \
  --lifecycle <lifecycle-id> --selection <selection.json> \
  --candidate <current-candidate.json> \
  --ledger <discovery-ledger.json> --responses <responses.json> \
  --self-verification <self-verification.json>
```

Staging also writes `review-request.md`, `dispatch-prompt.txt`, and
`reviewer-notes/<seat>.md`. The integrator edits the evidence and any
seat-specific rebuttal, then dispatches every reviewer with the exact generated
prompt. It materializes supplied exact artifacts as round-local
`selection.json`, `current-candidate.json`, `discovery-ledger.json`,
`responses.json`, and `self-verification.json`. The dispatch prompt is usable
only when the round's `.complete` marker exists; an unmarked scratch directory
is non-authoritative and must be cleaned up before retrying. Later reviews fail
closed unless `<prev-tip>` matches the previous recorded tip and every seat's
prior verdict is available, so the incremental range and prior-finding
instructions cannot be replaced by a free-form summary.

After verification verdicts are collected, copy this sequence without changing
the canonical ledger path or any of the artifact paths:

```bash
ROUND=.scratch/panel/<round>
SELECTION="$ROUND/selection.json"
CANDIDATE="$ROUND/current-candidate.json"
LEDGER="$ROUND/discovery-ledger.json"
RESPONSES="$ROUND/responses.json"
SELF_VERIFICATION="$ROUND/self-verification.json"
VERIFICATION="$ROUND/verification-results.json"
APPROVAL="$ROUND/approval.json"
METRICS="$ROUND/metrics.json"

node .github/skills/d2b-panel-round/scripts/panel-lifecycle.mjs \
  adapt-verification "$LEDGER" "$ROUND/verdicts" "$VERIFICATION" \
  --selection "$SELECTION" --candidate "$CANDIDATE"
node .github/skills/d2b-panel-round/scripts/panel-lifecycle.mjs \
  approval "$SELECTION" "$LEDGER" "$RESPONSES" "$VERIFICATION" "$APPROVAL" \
  --candidate "$CANDIDATE"
node .github/skills/d2b-panel-round/scripts/panel-lifecycle.mjs \
  metrics --selection "$SELECTION" --ledger "$LEDGER" --responses "$RESPONSES" \
  --verification-results "$VERIFICATION" --output "$METRICS"
node .github/skills/d2b-panel-round/scripts/make-records.mjs "$ROUND" \
  --selection "$SELECTION" --ledger "$LEDGER" --responses "$RESPONSES" \
  --verification-results "$VERIFICATION" --approval "$APPROVAL"
```

The verification command that precedes this sequence requires
`--candidate`, `--prior-selection`, `--delta`, and `--full-context`; none of
those inputs has an empty default.

Independent selected reviewers are a deliberate cost. This repository's own
history is the argument: an early panel returned zero sign-offs with eleven
high findings that the static gate caught none of. A future producer must
preserve the selected roster rather than compressing it into a smaller
synthesis council.

## Delivery memory

Three registers under `.specify/memory/`, driven by `/d2b-memory`.
Classification metadata only: never transcripts, validation output, or
attestation payloads.

The rule that keeps them from becoming a graveyard: **a category recurring
across three waves stops being friction and becomes a task.** That is a count,
not a judgement.

BLOCKER findings are never deferred or auto-filed. A MAJOR may be Deferred
only with the recorded maintainer or merge-owner acceptance defined by ADR
0055; otherwise it remains blocking.

A defect discovered while fixing something else goes into a register, not into
the current fix round. That is the mechanism that lets a fix round stay scoped
to the findings it answers without losing the defect.

## spec-kit on Copilot

spec-kit 0.14.4 is installed for Copilot in skills mode. Use the
hyphenated skills such as `/speckit-specify`, `/speckit-plan`, and
`/speckit-tasks`; dotted integration commands are not a repository command
surface.

**Do not run `specify init` in this repository.** It was trialled on a scratch
copy, and it can replace `installed_integrations` and rewrite shared files
under `.specify/scripts/` and `.specify/templates/`. Those rewrites can
silently change the selected integration or reintroduce banned dash codepoints,
which fails `make check-tier0`. Keep the committed Copilot state and skills
authoritative instead.

`check-bindings.mjs` fails closed unless `.specify/integration.json` lists only
Copilot and selects Copilot as both the current and default integration. It
also rejects stale integration settings, malformed JSON, and any retired
integration value.

## Copilot-only cutover

The cutover is immediate: Copilot is solely authoritative, and the legacy
tracked surface is removed rather than retained as a compatibility path.
Historical delivery records remain records only; they do not make a retired
integration executable or authoritative.
