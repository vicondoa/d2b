# `allocator.json` schema (`v2`)

Schema: [`allocator.json`](./allocator.json)

`allocator.json` is private, metadata-only local-root allocator configuration.
It is rooted in the normalized `d2b.realms` index and does not start a service,
bind a socket, mutate host state, or change current `d2b.envs` behavior.

## Top-level fields

- `schemaVersion` — schema version for this artifact.
- `allocator` — future allocator root socket/state/audit paths plus explicit
  `metadata-only` runtime state.
- `realms` — enabled realm rows selected from `d2b._index.realms`.
- `resourceRequests` — schema-friendly host-resource request metadata for
  path/socket partitions, network namespaces, shared env bridges, and optional
  host-mutation partitions.
- `pathPartitions` — per-realm state/run/audit/public-socket/broker-socket
  paths.
- `providerPlacements` — provider placement metadata copied from enabled realm
  provider rows. Optional provider `kind` values are bounded lowercase slugs.
- `envBridge` — transitional mapping from realm paths to existing env bridges and
  net VMs. `mode` is the closed realm network mode enum:
  `none`, `inherit-env`, `declared`, or `external`.
- `invariants` — booleans asserting this artifact is private metadata only and
  preserves the existing env runtime source of truth.

## Contract notes

- `runtimeState` is `metadata-only`; there is no allocator service/socket unit in
  this scope.
- Resource identifiers are opaque metadata. They are not host paths, interface
  names, nftables object names, file descriptors, or credentials.
- Every `realmPath` is a bounded DNS-style realm path: 1-16 lowercase
  dot-separated labels, maximum 255 bytes.
- Existing `d2b.envs` and `d2b.vms.<vm>.env` remain the runtime source of truth
  until a later allocator implementation lands.
