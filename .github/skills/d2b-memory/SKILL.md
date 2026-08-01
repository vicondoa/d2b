---
name: d2b-memory
description: Record, triage, fold, and file d2b delivery memory - deferred work, engineering friction, and debt. Use when a wave defers something, when the toolchain gets in the way, or at the end of a run to fold the open set into the next plan and file the rest as issues.
user-invocable: true
---

# Delivery memory

Three registers under `.specify/memory/`, one skill, four operations.

```
/d2b-memory record   <category> <what happened>
/d2b-memory triage
/d2b-memory fold
/d2b-memory file-issue <id>
```

The registers exist because the alternative is that every run rediscovers the
same friction, and every deferral is either forgotten or silently carried
forever. They are:

| Register | Holds |
|---|---|
| `.specify/memory/deferred-work.md` | work a wave consciously chose not to do |
| `.specify/memory/friction-log.md` | the engineering setup getting in the way |
| `.specify/memory/engineering-debt.md` | accepted shortcuts with a named cost |

## What may be recorded

**Classification metadata only.** Never transcripts, never validation output,
never attestation payloads, never diffs. An entry is a category, a wave
address, a one-line statement, a disposition, and an owner. If an entry needs
a paragraph of context to be actionable, it is a task, not a memory entry.

Categories, carried forward from the existing register taxonomy:
`signoff`, `build`, `test`, `merge`, `codegen`, `disk`.

Dispositions: `open`, `folded`, `filed`, `resolved`, `wontfix`.

## record

Append one row. The wave address uses the qualified token
(`spec001w1`, `adr046w3fu2`); a legacy bare `W1` remains valid for the
in-flight program.

```
| spec001w2 | test | 2026-02-14 | Contract lane needs a fixture build every run | open |  |
```

Record at the moment it happens, not at the end. A friction point noticed
during a fix round and not written down is lost, and it is the single most
common thing lost.

**A finding is not a memory entry.** Critical and high panel findings are
never deferrable and never auto-filed. They are fixed in the round that raised
them.

**A defect discovered while fixing something else goes here.** That is the
mechanism that lets a fix round stay scoped to the findings it answers without
losing the defect.

**A register never shrinks to nothing on its own.** Rows are dispositioned in
place rather than deleted, so a register with no rows means history was lost,
and `check-bindings.mjs` fails on it. If a register really is empty on purpose,
say so with a line reading exactly:

```
<!-- d2b-register: intentionally empty -->
```

outside any fenced block. It excuses an absent row, never an absent table: a
register with no header row still fails. Remove the marker when the first row
returns, because the gate refuses a register that declares itself empty and has
rows. That refusal is deliberate - a marker left behind would silently licence
the next truncation.

## triage

Read the open set and assign a disposition. Three rules decide it:

1. **A category recurring across three waves stops being friction and becomes
   a task.** Promote it into the plan. This is what keeps the register from
   becoming a graveyard, and it is not a judgement call: count the rows.
2. **Anything blocking the next wave folds now.** It is not memory; it is
   scope.
3. **Everything else that is genuinely low priority leaves the plan
   entirely** and becomes a GitHub issue. An item that stays `open` across
   three triages without being promoted or filed is being avoided; force it to
   one or the other.

## fold

Emit the open, foldable set as planning input for the next feature or wave:
a short list of concrete items, each with its category, its recurrence count,
and the wave that raised it. That output goes to the architect, who decides
whether each becomes a task.

Folding **does not** silently add tasks to a plan. It produces the input to
that decision, so the plan's task list stays something a person approved.

Mark folded rows `folded` with the target wave in the disposition column.

## file-issue

For a low-priority item that should leave the plan.

`<category>` below is substituted from the row's category column and MUST be one
of exactly `signoff`, `build`, `test`, `merge`, `codegen`, `disk`. Reject
anything else rather than passing it through; it reaches a shell.

**This operation mutates repository settings.** When a label is missing it
creates it, which is a wider privilege than filing an issue. A caller without
label-write permission cannot run it, and failing there is the correct outcome
rather than something to work around.

```bash
set -euo pipefail

existing=$(gh label list --limit 500 --json name --jq '.[].name')

ensure_label() {
  if ! grep -qxF "$1" <<<"$existing"; then
    gh label create "$1" --color "$2" --description "$3"
  fi
}

ensure_label delivery-memory 5319E7 'Raised by d2b delivery memory'
ensure_label '<category>'    BFD4F2 'd2b delivery memory category'

gh issue create \
  --title '<category>: <one-line statement>' \
  --label 'delivery-memory,<category>' \
  --body-file '<rendered body>'
```

`gh issue create` fails outright when a named label does not exist, so the labels
have to be ensured before it runs rather than assumed.

Five constraints on that block are load-bearing. Each one is here because
removing it reintroduces a specific defect, so treat them as prohibitions rather
than as style:

- **Do not drop `set -euo pipefail`.** It is what makes an authorisation denial,
  a rate limit, or a network error stop the run before `gh issue create` can
  report a misleading cause.
- **Do not guard the `gh label list` call.** It runs first and unguarded so that
  a caller without repository read access fails there, with the real error,
  rather than somewhere downstream. It is the authorisation probe.
- **Do not suppress errors** with `2>/dev/null` or `|| true`, and do not replace
  the query with an instruction to classify a failure. Suppression masks a
  permission denial and resurfaces it as a misleading "label not found" two
  commands later; an instruction is something an agent can skip. Querying first
  removes the failure rather than classifying it, so nothing depends on matching
  gh's error wording.
- **Do not use `--force`.** It updates the color and description of a label that
  already exists, so on a repository owning its own `test` or `build` label it
  would silently overwrite that label. Skipping creation when the name is
  present leaves it untouched.
- **Do not unquote any substituted value**, including the path passed to
  `--body-file`. Every placeholder above sits inside single quotes so that a
  value which somehow escaped the closed-set check, arbitrary text carried by
  the one-line statement, or a body path containing spaces cannot be expanded or
  word-split by the shell. This matters more for the statement than for the
  category: the category is drawn from a closed set of six words, while the
  statement is free text from the row. Under double quotes a statement holding
  `$PWD` would silently expand and publish a host path into a public issue
  title, and one holding an unbound name would trip `set -u` and echo that name
  into the transcript as the wrong error. The quotes belong to the command, not
  to the placeholder: substitute the bare value inside them, so `test` becomes
  `'test'` and never `''test''`. A statement containing a single quote must have
  it written as `'\''`, or be reworded; the body carries the detail anyway and
  reaches the command through `--body-file` rather than through an argument.

`--color` is optional; gh picks a random color when it is omitted. It is given so
a first creation is deterministic. `--limit 500` must exceed the repository's
label count; if it ever does not, a present label reads as absent, the create
fails, and `set -e` stops the run.

Body template:

```markdown
## What

<the one-line statement, expanded to two or three sentences>

## Where it came from

Raised during <qualified wave token>. Recurred <n> time(s).

## Why it is not in a plan

<the triage reason>
```

Then mark the row `filed` with the issue number.

**Never auto-file a critical or high finding.** Never include a transcript,
validation output, an attestation payload, a store path, or a real identifier
in an issue body. Redact any screenshot before attaching it: credentials,
tokens, real names, email addresses, host paths, and window titles naming a
real person or organisation all have to go, and a screenshot that cannot be
redacted without losing what it demonstrates should be replaced by a text
description.
