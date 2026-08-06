---
name: d2b-spec-edit
description: Apply one approved batch of changes inside one active feature directory. Dispatches d2b-architect with normal communication and refuses every path outside the feature root.
user-invocable: true
---

# Feature artifact editor

<!-- D2B-SPEC-EDIT: exclusive-feature-root-v1 -->

`d2b-spec-edit` is the exclusive mutation route for an existing feature
artifact. It accepts one active feature directory and one batch of requested
changes, then dispatches `d2b-architect` with normal communication:

| Route | `agent_type` | `model` | `reasoning_effort` | `context_tier` | `communication` |
|---|---|---|---|---|---|
| editor | `d2b-architect` | `gpt-5.6-sol` | `xhigh` | `long_context` | `normal` |

## Input contract

The caller supplies:

- `FEATURE_DIR`: one existing directory under the repository's `specs/`
  tree, resolved to its canonical path;
- one batch containing the caller, reason, accepted decisions or clarification
  answers, target files, expected sections, and requested follow-up commands.

The editor may change any regular file below `FEATURE_DIR`, including
`spec.md`, `plan.md`, `tasks.md`, `checklists/`, `contracts/`, `research.md`,
`data-model.md`, `quickstart.md`, and feature-local evidence. It must never
write an ADR, source file, contributor document, changelog, or any path outside
the active feature root.

## Fail-closed path protocol

1. Resolve `FEATURE_DIR` once. Refuse a missing directory, a root outside
   `specs/`, an absolute target, an empty target, a target containing `..`, or
   a target whose existing path or parent resolves outside `FEATURE_DIR`.
2. Normalize every requested target relative to the feature root. Reject
   symlink escapes, path aliases, repository-root paths, and targets outside
   the declared batch.
3. Snapshot every allowed existing path and the starting changed-path set
   before dispatch. Store transient snapshots and comparisons under
   `.scratch/spec-edit/`, never beside feature artifacts.
4. Scope the architect prompt to the resolved root and batch. No other file is
   an allowed write.
5. After dispatch, compute the changed-path set again. Accept only paths below
   `FEATURE_DIR`; report any foreign change as a scope failure and never revert
   foreign work.

No freshness sidecar, digest chain, or artifact-state file is created. The
editor records requested text changes only; analyze and panel workflows keep
their existing review responsibilities.

## Batch completion receipt

Return a receipt with sections changed, checklist state transitions, feature
files changed, requested related files deliberately left unchanged, and
validation or follow-up commands requested by the caller. If no requested
change is safe or the root check fails, refuse without writing.

