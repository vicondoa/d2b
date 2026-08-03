# ADR 0052 Internal Interface Contracts

These documents constrain implementation and migration evidence. They are not
new public versioned APIs.

Existing authority wins:

1. ADR 0052 for architecture and mechanics.
2. `docs/reference/test-execution-manifest.md` and its v1 schema for execution
   evidence.
3. Committed Make, Rust gate, Layer-1 manifest, and policy code for the current
   baseline.
4. These documents for plan-level interfaces left to implementation.

Files:

- `make-target-compatibility.md` - contributor and workflow entry points.
- `coverage-map.md` - internal coverage artifact shape and invariants.
- `execution-manifest-binding.md` - executor-to-existing-contract binding.
- `shadow-promotion-evidence.md` - evidence and lifecycle gates.
- `cache-workflow-boundaries.md` - permissions, credentials, and generations.
- `recovery-deadline.md` - cleanup, shutdown, and deadline behavior.

If one of these conflicts with ADR 0052 or committed passing code, record the
drift and follow the higher authority.
