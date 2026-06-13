# `manifest_v04.json` schema (`v2` companion)

Schema: [`manifest_v04.json`](./manifest_v04.json)

`manifest_v04.json` captures the typed public `vms.json` contract consumed by
the Rust CLI, daemon, and broker.

## Top-level fields

- `_manifest` — manifest metadata. Contains `manifestVersion`, pinned to `5`
  in the schema and parser.
- `_observability` — reserved observability sentinel block.
- dynamic VM keys — every non-reserved top-level key is one VM row.

## Contract notes

- The JSON Schema models the reserved sentinel keys explicitly and leaves VM
  names as pattern-matched dynamic properties.
- Unknown reserved sentinels and unknown per-VM fields are rejected
  fail-closed. Public-manifest breaking changes require a manifest-version
  bump and matching schema update.
