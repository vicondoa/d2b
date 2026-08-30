# Manifest bundle reference

**Diataxis category:** reference.

The private bundle is a daemon-facing compatibility contract. It carries
typed, generated inputs for `d2bd`, the Guest controller, specialized
Providers, and `d2b-broker`. It is not a public lifecycle API.

## Artifact set

| Artifact | Visibility | Purpose |
| --- | --- | --- |
| `vms.json` | public compatibility projection | Bounded legacy inventory data for diagnostics and migration tooling. |
| `bundle.json` | private | Bundle version, artifact paths, digests, and compatibility metadata. |
| `host.json` | private | Host capabilities, Provider catalog, Network/Device requirements, and support tier. |
| `processes.json` | private | Controller and runner intent, readiness predicates, and delegated sandbox metadata. |
| `storage.json` | private | Anchored managed paths, restart adoption, cleanup, repair, and degraded states. |
| `sync.json` | private | OFD lock and fd-transfer policy, acquisition order, and stale-owner handling. |
| `allocator.json` | private | Zone-scoped resource allocation metadata and opaque host-resource leases. |
| `realm-controllers.json` | private compatibility artifact | Transitional metadata read by the current daemon bridge; it is not a product hierarchy or lifecycle owner. |
| `realm-identity.json` | private compatibility artifact | Transitional identity metadata; credential and session authority remains in Zone Resources. |
| `realm-workloads-launcher-v2.json` | private, daemon-served | Argv-free launcher metadata exposed only through the authorized daemon API. |
| `unsafe-local-workloads.json` | private | Validated unsafe-local Provider intent resolved by `d2bd`. |
| `privileges.json` | private | Public API and broker authorization policy. |
| `closures/<Guest>.json` | private | Per-Guest system closure and generation metadata. |
| `minijail-profile.json` | private | Typed sandbox profile metadata used by approved Providers. |

The daemon and broker own access to private artifacts. Secret bytes,
credentials, raw host paths, executable arguments, and private runtime
locators do not cross the public API boundary.

## Current ownership

Nix declares Zone resources and immutable artifacts. The Guest controller owns
the direct child Resource graph and lifecycle status. Specialized controllers
own Process, Endpoint, Volume, Network, Device, Credential, and Provider
effects. `d2b-broker` performs only approved typed host mutations.

The compatibility documents with `realm-` filenames may remain during host
migration, but they cannot create, discover, or authorize a Guest lifecycle.
The current authority is the Zone Resource store and authenticated session.
The current line is a clean break from v1/v2 host state: these artifacts do
not promise old-path adoption, data retention, or state conversion.

## Versioning

| Field | Scope | Rule |
| --- | --- | --- |
| `bundleVersion` | Private bundle | Bump for a breaking daemon/broker contract change. |
| `schemaVersion` | One artifact | Bump for artifact-local schema evolution. |
| `manifestVersion` | Public compatibility projection | Bump for a breaking public reader change. |

Update the Rust DTO, Nix emitter, generated schema, prose, fixture, and
changelog together. Do not hand-edit generated JSON.

## Generation and drift

```bash
bazel run //packages/xtask:xtask -- gen-schemas
make test-drift
```

Generated schemas live under [`schemas/v2/`](./schemas/v2/). The current
Zone-specific resource schemas live under [`schemas/v3/`](./schemas/v3/).

## Related references

- [`manifest-schema.md`](./manifest-schema.md) - public compatibility schema.
- [`zone-control-nix.md`](./zone-control-nix.md) - current Nix authoring.
- [`daemon-api.md`](./daemon-api.md) - daemon and session contract.
- [`schemas/v2/bundle.md`](./schemas/v2/bundle.md) - bundle DTO details.
- [`schemas/v2/host.md`](./schemas/v2/host.md) - host DTO details.
- [`schemas/v2/processes.md`](./schemas/v2/processes.md) - process intent.
- [`schemas/v2/storage.md`](./schemas/v2/storage.md) - storage lifecycle.
- [`schemas/v2/sync.md`](./schemas/v2/sync.md) - synchronization.
