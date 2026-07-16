# `provider-registry-v2.json` schema (`v2`)

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
- `local-runtime` bindings map normalized host-local Cloud Hypervisor and
  qemu-media workloads to the authoritative VM-start and runner intents emitted
  by the workload process rows. Descriptor IDs remain workload-scoped
  `runtime-<workloadId>` adapter identities even when several workloads select
  one configured runtime provider; the normalized provider binding validates
  same-realm enabled authority and implementation but is not serialized as the
  descriptor ID.
- `local-observability` bindings carry only bounded `maxRecords`, `maxBytes`,
  and `maxTimeWindowMs` limits. They carry no target or cardinality IDs;
  descriptor placement is the realm authority. The host emits these rows only
  for enabled host-local root realms and exposes closed aggregate
  metrics/audit-health projections without repair authority.
- The serialized binding union is closed. Its additional declared mappings are:
  - `local-transport` — a bounded, non-empty set of unique
    `transportBindingIds`;
  - `local-substrate` — no repeated target data; descriptor placement and the
    implementation ID select the host check profile;
  - `local-display` — `workloadId`, `ownerRoleId`, and four distinct generated
    endpoint IDs (`wayland`, `crossDomain`, `waypipe`, and `proxy`);
  - `network` — generated network, allocator lease, bridge, TAP, net-VM role,
    policy, optional external-attachment, and resource-generation IDs;
  - `local-storage` — `realmId`, `workloadId`, generated local-state,
    disk-set, store-view, closure-sync, and media-set IDs, plus resource
    generation. The repeated realm ID is required by the frozen local-storage
    factory binding and must equal descriptor placement;
  - `local-device` — a bounded set of unique generated `deviceResourceIds`;
    an enabled device provider may have no requested device resources;
  - `local-audio` — `workloadId`, `roleId`, and generated process, endpoint,
    state-storage, lock-storage, mediation-storage, and lease IDs.
- These mapping bindings contain neither host paths nor argv. Descriptor
  placement remains authoritative for realm placement and provider ID. The
  local-storage binding repeats its realm ID only for its frozen factory
  adapter, and validation rejects disagreement with descriptor placement.
- The Rust enum and its consumer view are non-exhaustive for downstream code,
  but JSON decoding still rejects any undeclared axis or field. Consumers must
  explicitly register an adapter for each declared mapping; an unregistered
  mapping fails closed.
- An explicit zero-row artifact is valid. A missing artifact is not.
- Azure VM implementations, credential providers, and `RuntimeExecute` are not
  live host registrations.
