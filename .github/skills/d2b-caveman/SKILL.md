---
name: d2b-caveman
description: Optional full transient communication for selected d2b delivery and review lanes. Uses the current Copilot session only and never changes persisted artifact prose.
user-invocable: true
---

# Optional transient communication

Use this skill only when the caller selects Caveman communication for a lane
declaring `caveman-full-optional`. The mode is optional; when selected,
`full` is the default. An explicit `normal` or `off` request always wins and
restores normal professional communication.

## Scope

- Compress only free-form transient communication: status, handoff, review
  discussion, and other messages that are not persisted.
- Keep persisted code, comments, release notes, commits, pull request text,
  issue text, memory files, contributor docs, ADRs, feature artifacts, and
  consumer docs in normal professional prose.
- The one persisted exception is the governed prompt corpus admitted by
  `scripts/copilot/prompt-corpus-manifest.json` and checked by
  `scripts/copilot/prompt-corpus.mjs`.
- Never require Anthropic, Claude CLI, Python, an external install, network
  access, or repository-content upload. The current Copilot session is the only
  communication engine.

## Preservation

Compression never changes:

- code, commands, paths, URLs, identifiers, environment variables, versions,
  numbers, units, or exact errors;
- negations, exceptions, refusal rules, ordering, causal constraints, or
  security warnings;
- JSON, output examples, schemas, panel finding bars, verdict fields, or
  required formatting;
- the user's language or an explicitly requested normal response.

Do not grade brevity or claim a lane used compressed wording. A caller can
request `normal` or `off` at any point; that choice applies immediately.

## Safety boundary

Use normal prose for ambiguity, destructive or irreversible actions, security
warnings, clarification, or any message where compression could change meaning.
Resume optional transient compression only after the boundary is clear. Never
compress a file by invoking an upstream runtime or sending repository content
to a third party.
