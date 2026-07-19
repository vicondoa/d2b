# `provider-registry-v2.json` schema (`v2`)

**Diataxis category:** reference.

Schema: [`provider-registry-v2.json`](./provider-registry-v2.json)

This private artifact binds canonical provider descriptors to closed,
per-axis configuration rows. Descriptors carry canonical provider and realm
IDs; bindings carry only target identity that placement cannot express plus
opaque intent IDs resolved through the integrity-verified bundle. They never
carry argv, host paths, or credential material.

## Top-level fields

- `schemaVersion` — schema version for this artifact.
- `registryGeneration` — generation shared by the registry and every provider
  descriptor.
- `configurationFingerprint` — SHA-256 fingerprint of the schema version,
  registry generation, and provider entries.
- `publishedAtUnixMs` — publication time in Unix milliseconds.
- `providers` — canonical provider descriptor and binding entries, sorted by
  provider ID.

## Contract notes

- `schemaVersion` is `v2`.
- `registryGeneration` must match every descriptor generation.
- Provider entries are bounded, unique, and sorted by canonical provider ID.
- `ProviderBindingV2` wire decoding remains strict to variants declared by the
  current schema. Downstream consumers must retain an explicit unsupported
  fallback so a newly declared variant cannot silently activate behavior.
- Each local-runtime descriptor's provider ID is re-derived from the realm in
  descriptor placement and the workload ID in its binding. Its configuration
  fingerprint and scope digest are recomputed from those closed fields, while
  startup factory composition verifies exact placement, implementation, and
  capabilities.
- **Specification correction:** `LocalRuntimeProviderBindingV2` does not repeat
  `realmId`. Descriptor placement is the sole realm authority, making
  contradictory realm JSON unrepresentable. `workloadId` remains in the
  binding because a realm-scoped trusted in-process descriptor does not identify
  the target workload.
- `local-runtime` bindings map explicit local-VM and qemu-media workloads to
  matching VM-start and runner intents.
- `local-observability` bindings carry only bounded `maxRecords`, `maxBytes`,
  and `maxTimeWindowMs` limits. They carry no target or cardinality IDs;
  descriptor placement is the realm authority. The host emits these rows only
  for enabled host-local root realms and exposes closed aggregate
  metrics/audit-health projections without repair authority.
- An explicit zero-row artifact is valid. A missing artifact is not.
- Azure VM implementations, credential providers, and `RuntimeExecute` are not
  live host registrations.
