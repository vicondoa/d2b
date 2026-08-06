# Spec 003 Internal Contracts

These files constrain implementation and migration evidence. They are not new
public APIs.

Authority order:

1. ADR 0052 as amended by ADR 0054.
2. Committed passing code for the current baseline.
3. The execution-manifest v1 reference and schema.
4. This amended Spec 003 artifact set.

Contracts:

- `workspace-and-tool-pinning.md` - product and walker workspace, lock, hub,
  repin, module refresh, yanked authority, selected-context oracle,
  local-socket/external-egress boundary, package-policy equivalence,
  source-census, Nix selection, generated ownership, release wiring, and
  tool-pinning contract, including the Nix-patched Bazel Linux sandbox and
  exact four-row artifact baseline.
- `coverage-map.md` - eighteen-surface carrier coverage, native first-party
  targets, selected-context censuses, and guard placement.
- `make-target-compatibility.md` - shadow, contributor mutation, promotion,
  versioned diagnostic transition, and retirement command surface.
- `runner-environment.md` - test topology, provider, environment, and
  sanitized bounded per-case evidence contract, compile-sealed verified
  executable, immutable static C execution supervisor, safe Rust command-fd
  mapping, typed ownership/error transport, retention classes, and canonical
  closed complete/degraded evidence.
- `execution-manifest-binding.md` - Bazel carrier results bound to existing
  execution-manifest v1.
- `shadow-promotion-evidence.md` - qualification, the typed qualification
  validator, dual-architecture, promotion evidence, cache counts, broker
  repetition, stable-head rules, typed sealed-merge promotion binding, and
  complete-transient/bounded-persisted post-promotion run-unit derivation.
- `cache-workflow-boundaries.md` - permissions, credentials, key inputs,
  trimming, and cache generations.
- `recovery-deadline.md` - cleanup, shutdown, deadline, and recovery behavior.

If an implementation detail disagrees with older Spec 003 prose, follow the
authority above and record the correction in `plan.md`.
