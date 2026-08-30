# Manifest schema

**Diataxis category:** reference.

The committed JSON Schema is
[`manifest-schema.json`](./manifest-schema.json). It describes the public
compatibility manifest consumed by host diagnostics and migration tooling.
It is not the authority for current Guest lifecycle.

## Current authority

Current configuration declares Zone-owned Resources and immutable artifacts:

```text
Zone/<zone>
Guest/<name>
Provider/<name>
Process/<name>
```

`d2bd` resolves those Resources from the Zone store. The Guest controller owns
child creation, readiness, restart adoption, and finalization. The broker
resolves private runtime identity from the authenticated Zone and Guest
incarnations.

Private bundle documents and Provider manifests carry the inputs required by
the daemon and broker. See [`manifest-bundle.md`](./manifest-bundle.md) and
[`zone-control-nix.md`](./zone-control-nix.md).

## Compatibility document

The manifest schema remains versioned and strict for readers that still need
the public compatibility projection. Its fields may contain historical
workload terminology because the compatibility format is frozen. Consumers
must not infer lifecycle ownership, host paths, credentials, or Provider
authority from those fields.

Unknown fields, malformed digests, invalid names, and inconsistent generation
metadata fail closed. A compatibility projection never authorizes a host
mutation and never replaces the Zone Resource API.

## Generated artifact rule

The schema is generated from the canonical Rust DTOs:

```bash
bazel run //packages/xtask:xtask -- gen-schemas
```

Update the DTO, generator, prose, fixtures, and changelog together when the
compatibility contract changes. Do not hand-edit the generated JSON.

## Historical references

Older migration pages may call this document a VM manifest or use retired
hierarchy names. Those pages explain upgrade history only. New integrations
must use Zone and ResourceRefs.
