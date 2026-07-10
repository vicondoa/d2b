# Manifest bundle reference

**Diataxis category:** reference.

The manifest bundle is the private, daemon-facing contract. It lives
beside the existing public `vms.json` manifest and carries
host intent, process topology, privilege policy, closure metadata, and
sandbox profile metadata that must not be exposed through the
world-readable system profile.

## Artifact set

| Artifact | Visibility | Mode | Purpose |
| --- | --- | --- | --- |
| `vms.json` | public compatibility surface | world-readable, existing installation path | VM list and public capability metadata; see `docs/reference/manifest-schema.md` for the current `manifestVersion`. |
| `bundle.json` | private bundle index | root:`d2bd` `0640` | Bundle version, artifact paths, hashes, and compatibility policy. |
| `host.json` | private host intent | root:`d2bd` `0640` | Host requirements, network intent, runtime/provider catalog, kernel/device/fd requirements, and support tier. |
| `processes.json` | private supervisor intent | root:`d2bd` `0640` | Per-VM process DAG, readiness predicates, cgroup placement, and minijail profile IDs. |
| `storage.json` | private storage lifecycle contract | root:`d2bd` `0640` | Managed path inventory, restart/adoption policy, degraded-state taxonomy, cleanup/repair policy, and remediation IDs. |
| `sync.json` | private synchronization contract | root:`d2bd` `0640` | Lock inventory, OFD/fd-transfer policy, acquisition order, stale-owner policy, and lock degraded-state handling. |
| `allocator.json` | private allocator metadata | root:`d2bd` `0640` | Metadata-only local-root allocator plan rooted in `d2b.realms`: enabled realms, resource requests, path/socket partitions, provider placement, and env bridges. |
| `realm-controllers.json` | private realm controller metadata | root:`d2bd` `0640` | Metadata-only per-realm daemon, broker, socket, state, audit, allocator binding, provider placement, and direct-access plan rooted in `d2b.realms`. |
| `realm-workloads-launcher-v2.json` | public launcher metadata in the private bundle | root:`d2bd` `0640` | Argv-free provider, posture, realm color, and generic launcher-item metadata served to authorized clients through the public daemon API. |
| `unsafe-local-workloads.json` | private unsafe-local execution intent | root:`d2bd` `0640` | Normalized configured argv, workload identity, default item, and persistent-shell policy resolved only by `d2bd`. |
| `privileges.json` | private authorization policy | root:`d2bd` `0640` | Public API/CLI authorization matrix and private broker operation matrix. |
| `closures/<vm>.json` | private closure metadata | root:`d2bd` `0640` | Per-VM toplevel, closure paths, declared-runner parity data, and generation metadata. |
| `minijail-profile.json` | private sandbox profile catalog | root:`d2bd` `0640` | Typed minijail profile fields, mount policy, and bounded start-as-root exceptions. |

`vms.json` is the only world-readable artifact. All other artifacts are
daemon-owned bundle inputs, including public-safe launcher metadata that
`d2bd` exposes through its authorized API. The privileged boundary is described by
[ADR 0002](../adr/0002-non-root-daemon-and-privileged-broker.md).

## Versioning policy

| Version field | Scope | Bump rule |
| --- | --- | --- |
| `bundleVersion` | Entire private bundle | Bump for any breaking change that affects daemon or broker compatibility across the artifact set. |
| `schemaVersion` | One artifact schema | Bump for artifact-local schema evolution, including additive optional fields. |
| `_manifest.manifestVersion` | Public `vms.json` only | Bump for breaking public-manifest changes; private bundle versioning does not replace this public compatibility gate. |

The policy is defined by
[ADR 0006](../adr/0006-manifest-bundle-versioning.md). The current
schema directory is `docs/reference/schemas/v2/`; the bundle and
per-artifact schemas were bumped from `v1` to `v2` to land the
host-prepare additions; the current emitted
bundle keeps `schemaVersion = "v2"` and uses `bundleVersion = 10`
for the unsafe-local workload execution artifact.
Each artifact now carries a
matching v2 markdown companion beside the committed JSON schema.
`cargo xtask gen-schemas` regenerates the JSON files under
`schemas/v2/` from the Rust DTOs in `d2b-core` and
`d2b-contracts`; keep the markdown companions in sync in the same
commit whenever the schema changes.

## Drift policy

The committed schema files are derived from Rust DTOs in `d2b-core`
and `d2b-contracts`. The drift gate is:

```bash
cargo xtask gen-schemas
git diff --exit-code docs/reference/schemas/
```

Any diff is a contract drift. A valid schema change updates the Rust
DTOs, generated JSON Schemas, Nix emitters, prose docs, and tests in the
same wave integration.

## Public/private boundary

The public boundary is intentionally narrow:

- `vms.json` may contain VM names, public capability bits, public socket
  locations already required by the bash CLI, and non-secret topology
  metadata.
- `realm-workloads-launcher-v2.json` contains public-safe, argv-free metadata
  but remains daemon-owned `0640`; unprivileged consumers receive it through
  the authorized public daemon API.
- Private artifacts may contain command argv, broker-only paths,
  cgroup/device/fd requirements, sandbox profile internals, closure
  paths, qemu-media direct image-file paths authored in Nix config, and
  authorization policy.
- Secret material is never embedded in either boundary. Secret and key
  references use opaque key IDs only; path-bearing private-key fields are
  rejected by static gates.

Consumers that only need the compatibility manifest must read
[`manifest-schema.md`](./manifest-schema.md). Daemon and broker
implementations consume this bundle reference and the per-artifact schema
references below.

## Per-artifact and wire references

| Artifact | Prose reference | JSON Schema (current `v2` baseline) |
| --- | --- | --- |
| `bundle.json` | [`schemas/v2/bundle.md`](./schemas/v2/bundle.md) | `schemas/v2/bundle.json` |
| `host.json` | [`schemas/v2/host.md`](./schemas/v2/host.md) | `schemas/v2/host.json` |
| `processes.json` | [`schemas/v2/processes.md`](./schemas/v2/processes.md) | `schemas/v2/processes.json` |
| `storage.json` | [`schemas/v2/storage.md`](./schemas/v2/storage.md) | `schemas/v2/storage.json` |
| `sync.json` | [`schemas/v2/sync.md`](./schemas/v2/sync.md) | `schemas/v2/sync.json` |
| `allocator.json` | [`schemas/v2/allocator.md`](./schemas/v2/allocator.md) | `schemas/v2/allocator.json` |
| `realm-workloads-launcher-v2.json` | [`unsafe-local-provider.md`](./unsafe-local-provider.md) | `schemas/v2/realm-workloads-launcher-v2.json` |
| `unsafe-local-workloads.json` | [`unsafe-local-provider.md`](./unsafe-local-provider.md) | `schemas/v2/unsafe-local-workloads.json` |
| `privileges.json` | [`schemas/v2/privileges.md`](./schemas/v2/privileges.md) | `schemas/v2/privileges.json` |
| `closures/<vm>.json` | [`schemas/v2/closures.md`](./schemas/v2/closures.md) | `schemas/v2/closures.json` |
| `minijail-profile.json` | [`schemas/v2/minijail-profile.md`](./schemas/v2/minijail-profile.md) | `schemas/v2/minijail-profile.json` |
| `manifest_v04.json` | [`schemas/v2/manifest_v04.md`](./schemas/v2/manifest_v04.md) | `schemas/v2/manifest_v04.json` |
| `wire-protocol.json` | [`schemas/v2/wire-protocol.md`](./schemas/v2/wire-protocol.md) | `schemas/v2/wire-protocol.json` |
| unsafe-local helper protocol | [`unsafe-local-provider.md`](./unsafe-local-provider.md) | `schemas/v2/unsafe-local-helper-wire.json` |
| guest-control protocol | [`schemas/v2/guest-control.md`](./schemas/v2/guest-control.md) | `schemas/v2/guest-control.json` |
