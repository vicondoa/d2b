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
| `bundle.json` | private bundle index | root:`nixlingd` `0640` | Bundle version, artifact paths, hashes, and compatibility policy. |
| `host.json` | private host intent | root:`nixlingd` `0640` | Host requirements, network intent, kernel/device/fd requirements, and support tier. |
| `processes.json` | private supervisor intent | root:`nixlingd` `0640` | Per-VM process DAG, readiness predicates, cgroup placement, and minijail profile IDs. |
| `privileges.json` | private authorization policy | root:`nixlingd` `0640` | Public API/CLI authorization matrix and private broker operation matrix. |
| `closures/<vm>.json` | private closure metadata | root:`nixlingd` `0640` | Per-VM toplevel, closure paths, declared-runner parity data, and generation metadata. |
| `minijail-profile.json` | private sandbox profile catalog | root:`nixlingd` `0640` | Typed minijail profile fields, mount policy, and bounded start-as-root exceptions. |

`vms.json` is the only public artifact. All other artifacts are trusted
inputs to `nixlingd` and the privileged broker described by
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
bundle keeps `schemaVersion = "v2"` and bumps `bundleVersion = 4`
for the additive USBIP `busIds` wiring). Each artifact now carries a
matching v2 markdown companion beside the committed JSON schema.
`cargo xtask gen-schemas` regenerates the JSON files under
`schemas/v2/` from the Rust DTOs in `nixling-core` and
`nixling-ipc`; keep the markdown companions in sync in the same
commit whenever the schema changes.

## Drift policy

The committed schema files are derived from Rust DTOs in `nixling-core`
and `nixling-ipc`. The drift gate is:

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
- Private artifacts may contain command argv, broker-only paths,
  cgroup/device/fd requirements, sandbox profile internals, closure
  paths, and authorization policy.
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
| `privileges.json` | [`schemas/v2/privileges.md`](./schemas/v2/privileges.md) | `schemas/v2/privileges.json` |
| `closures/<vm>.json` | [`schemas/v2/closures.md`](./schemas/v2/closures.md) | `schemas/v2/closures.json` |
| `minijail-profile.json` | [`schemas/v2/minijail-profile.md`](./schemas/v2/minijail-profile.md) | `schemas/v2/minijail-profile.json` |
| `manifest_v04.json` | [`schemas/v2/manifest_v04.md`](./schemas/v2/manifest_v04.md) | `schemas/v2/manifest_v04.json` |
| `wire-protocol.json` | [`schemas/v2/wire-protocol.md`](./schemas/v2/wire-protocol.md) | `schemas/v2/wire-protocol.json` |
| guest-control protocol | [`schemas/v2/guest-control.md`](./schemas/v2/guest-control.md) | `schemas/v2/guest-control.json` |
