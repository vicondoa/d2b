# Execution Manifest Binding

## Existing authority

`docs/reference/test-execution-manifest.md` and
`docs/reference/schemas/test-execution-manifest-v1.json` are authoritative.
ADR 0052 changes the executor, not that contract. This document adds no field,
identifier, status, or lifecycle rule.

## Adapter requirements

- Build Event Protocol results map to the exact eighteen existing surface IDs.
- A surface enters `completed_leaves` only after every command and companion
  required by its coverage row succeeds.
- Carrier failures map to `failed_surfaces` without collapsing several
  carriers into one result.
- Prior manifest evidence is invalidated before dispatch.
- Normal failure and handled interruption publish sorted partial evidence
  atomically and preserve the original command status.
- An uncatchable termination may publish nothing, but cannot leave an old
  success record in place.
- Fixture-backed IDs are emitted only by the unchanged Cargo/Nix fixture path.
- Executor name is migration metadata and is not added to schema v1.

## Equivalence

Cargo and Bazel evidence is comparable only when it refers to the same commit,
uses the same fixture mode, and validates against schema v1. Passing promotion
evidence contains all eighteen baseline IDs. A failed/interrupted manifest is
valid diagnostic evidence but cannot satisfy positive equivalence.

Static source or Bazel query inventory cannot substitute for execution
evidence.
