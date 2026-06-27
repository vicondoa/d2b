# `bundle.json` schema reference

`bundle.json` is the private bundle index. It identifies the bundle version, records the artifact set that belongs to that version, and provides the hashes and compatibility metadata that `d2bd` and the broker use before trusting sibling artifacts.

Producer: `nixos-modules/manifest-bundle.nix` emits this artifact; `packages/d2b-core` parses it.

Schema: [`bundle.json`](./bundle.json) (forward reference; generated with `cargo xtask gen-schemas`).

## Fields

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `schemaVersion` | string | yes | Artifact schema version. This schema emits `v1`. |
| `bundleVersion` | integer | yes | Bundle-wide compatibility version; see [ADR 0006](../../../adr/0006-manifest-bundle-versioning.md). |
| `generatedAt` | string | yes | Deterministic generation timestamp or build metadata string supplied by the Nix emitter. Consumers must not use it for freshness decisions. |
| `producer` | object | yes | D2b producer identity: package/version metadata and the source revision when available. |
| `publicManifest` | object | yes | Location, `manifestVersion`, and hash for the sibling public `vms.json`. |
| `artifacts` | object | yes | Map of private artifact identifiers to paths, schema versions, modes, owners, and content hashes. |
| `hashAlgorithm` | string | yes | Hash algorithm used for every hash in this artifact set. The bundle uses one value across the artifact set. |
| `compatibility` | object | yes | Minimum/maximum supported `bundleVersion`, supported artifact `schemaVersion` values, and fail-closed policy text. |

## `producer`

| Field | Type | Description |
| --- | --- | --- |
| `name` | string | Producer name; this schema uses `d2b`. |
| `version` | string | D2b package or release version when available. |
| `sourceRevision` | string or null | Source revision for traceability. |
| `system` | string | Nix system that evaluated the bundle, such as `x86_64-linux`. |

## `publicManifest`

| Field | Type | Description |
| --- | --- | --- |
| `path` | string | Absolute path to `vms.json`. |
| `manifestVersion` | integer | Public manifest contract version. This schema preserves `2`. |
| `sha256` | string | Hash of the rendered `vms.json`. |

## `artifacts`

The artifact map is closed over these keys:

| Key | Target |
| --- | --- |
| `host` | `host.json` |
| `processes` | `processes.json` |
| `privileges` | `privileges.json` |
| `closures` | `closures/<vm>.json` entries |
| `minijailProfile` | `minijail-profile.json` |

Each artifact descriptor carries `path`, `schemaVersion`, `owner`,
`group`, `mode`, and `sha256`. Private descriptors must resolve to
root:`d2bd` `0640`; `vms.json` remains outside this private map.
