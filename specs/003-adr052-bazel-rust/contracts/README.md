# ADR 0052 Internal Interface Contracts

These documents constrain implementation and migration evidence. They are not
new public versioned APIs.

Existing authority wins:

1. The amended ADR 0052 for architecture and mechanics.
2. `docs/reference/test-execution-manifest.md` and its v1 schema for execution
   evidence.
3. Committed Make, Rust gate, Layer-1 manifest, and policy code for the current
   baseline.
4. These documents for plan-level interfaces left to implementation.

Files:

- `make-target-compatibility.md` - contributor and workflow entry points.
- `coverage-map.md` - internal coverage artifact shape, cardinality, and the
  split between analysis-time, in-test, and out-of-test invariants.
- `runner-environment.md` - child environment, per-case result document,
  filesystem semantics, and the scope of the no-shell rule.
- `workspace-and-tool-pinning.md` - startup options, workspace boundary, the
  four dependency hubs and their locks, the repository-owned commands that
  regenerate each committed lock and validate the yanked snapshot, the exact
  operator-facing recovery text every refusal must carry, and permitted tool
  acquisition.
- `execution-manifest-binding.md` - executor-to-existing-contract binding.
- `shadow-promotion-evidence.md` - qualification records, evidence, and
  lifecycle gates.
- `cache-workflow-boundaries.md` - permissions, credentials, key inputs,
  trimming, and generations.
- `recovery-deadline.md` - cleanup, shutdown, and deadline behavior.

If one of these conflicts with the amended ADR 0052 or committed passing code,
record the drift and follow the higher authority.
