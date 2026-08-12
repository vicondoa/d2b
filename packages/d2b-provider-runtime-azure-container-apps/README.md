# `d2b-provider-runtime-azure-container-apps`

This is the canonical crate root for
`Provider/runtime-azure-container-apps`. It is a compile-safe scaffold;
semantic Provider behavior is intentionally not present here.

See [Create a Provider](../../docs/how-to/create-provider.md) and the
[runtime-azure-container-apps dossier](../../docs/specs/providers/ADR-046-provider-runtime-azure-container-apps.md)
for the implementation contract.

## Provider identity

| Field | Value |
| --- | --- |
| Provider name | `runtime-azure-container-apps` |
| Provider reference | `Provider/runtime-azure-container-apps` |
| Package | `packages/d2b-provider-runtime-azure-container-apps/` |

## Config schema

The Provider-specific configuration is defined by the
runtime-azure-container-apps dossier. This scaffold does not publish a
configuration schema.

## Exported resource types

The resource types are defined by the dossier. This scaffold exports no
resource implementation.

## Controllers / services / workers / binaries

None are implemented in this scaffold. Controllers, services, workers, and
binaries belong to the owning Provider implementation.

## Placement and dependencies

No runtime placement is declared, and the scaffold has no workspace
dependencies.

## RBAC requirements

The scaffold requests no permissions and performs no resource or effect
operations.

## Security posture

No host, broker, filesystem, network, process, credential, or device effect is
reachable from this scaffold.

## State and telemetry

The scaffold owns no state and emits no telemetry.

## Build and test

```bash
cargo check -p d2b-provider-runtime-azure-container-apps
cargo test -p d2b-provider-runtime-azure-container-apps
```

The current test targets are structural compile checks. Executable scenarios
belong to the owning implementation.
