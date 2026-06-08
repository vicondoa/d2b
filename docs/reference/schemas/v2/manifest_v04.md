# `manifest_v04.json` schema (`v2` companion)

Schema: [`manifest_v04.json`](./manifest_v04.json)

`manifest_v04.json` captures the typed public `vms.json` contract the CLI,
daemon, and broker still accept for v0.4.x compatibility.

## Top-level fields

- `_manifest` — manifest metadata (`schemaVersion`, emitter metadata, and
  other reserved sentinels).
- `_observability` — reserved observability sentinel block.
- dynamic VM keys — every non-reserved top-level key is one VM row.

## Contract notes

- The JSON Schema models the reserved sentinel keys explicitly and leaves VM
  names as pattern-matched dynamic properties.
- Future public-manifest breaking changes require a new manifest schema, not
  an in-place mutation of this companion.
