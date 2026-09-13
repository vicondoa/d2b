# `d2b-provider-provider`

This is the crate root for the `Provider` resource type. It owns the type's
driver, its spec decoder, and the driver declaration the v3 resource plane
registers the type by.

`Provider` is the identity registry type, and the one core type with behavior beyond convergence. The driver observes the Provider's owned controller `Process` rows and state `Volume` rows - plus the fixed `Host` and `Zone` dependency rows for the two internally hosted providers - and publishes the phase the pure provider policy (`providers`) projects.

## Provider identity

| Field | Value |
| --- | --- |
| Provider name | `provider` |
| ResourceType | `Provider` |
| Package | `packages/d2b-provider-provider/` |
| Driver declaration | `provider_descriptor` -> `DriverDescriptor` |

## Config schema

The type declares no provider config schema: `Provider` rows carry the JSON spec
object the core rows store, and nothing outside it decodes at validate.

## Exported resource types

`Provider` is not exportable. `ResourceExport` admits only qualified
`*.d2bus.org.*Service` types, so a `Provider` row is never an export subject. The
declaration carries `exportable: false`.

## Controllers / services / workers / binaries

The crate ships one driver factory, not a standalone process. The driver
serves `Provider` rows through the `ResourceDriver` verbs: `validate` decodes the
stored spec envelope and admits the JSON spec object every core row stores,
`recover` adopts the converged row, `reconcile` converges it as metadata,
`finalize` drains owned children, and `delete` retires the row.

`ProviderDriverFactory` is the registration surface; `provider_descriptor` carries
it with the decoder, the type's verbs, execution domains, reads, and the
`BUILTIN` allowed-source mask the plane's presence obligation reads.

## Placement and dependencies

`Provider` names no placement anchor, so a `Provider` row is reconciled on its
containing Zone's Host.

The crate depends on `d2b-contracts-resource`, `d2b-resource-runtime`,
`d2b-resource-types`, `d2b-contracts-provider`,
`d2b-contracts-zone-session`, and `d2b-controller-toolkit`, plus the two
provider crates that export the fixed provider identities the observation
pins (`d2b-provider-system-core::PROVIDER_REF`,
`d2b-provider-process-minijail::PROVIDER_REF`). The driver's live
session-evidence effect arrives through its port, which the daemon
implements, so the crate depends on no daemon type.

## RBAC requirements

The driver holds no broker authority of its own: it runs inside the daemon's
per-zone plane, and every row it reads or writes is reached through the
manager with the plane's own caller identity. A stored spec that does not
decode as the JSON spec object is a terminal refusal rather than a
best-effort pass.

## Security posture

The driver invents no identity from spec text: the stored envelope is decoded
strictly, a row whose spec is absent or undecodable fails closed, and the
drain is child-first so no dependents are orphaned. Every failure travels as a
registered failure kind on the structured failure surface.

## State and telemetry

The type publishes no durable status: the old plane's phase and
`observedGeneration` projections have no successor on the v3 surface, and the
driver keeps no in-memory status either. Failures travel as the registered
core failure kinds (`core-spec-invalid`, `core-dependency-read-failed`,
`core-drain-pending`) on the structured failure surface, which is what the
daemon logs and what tests assert.

## Build and test

```bash
cargo test -p d2b-provider-provider
```

The unit tests drive validate, recover, reconcile, finalize, and delete over
a scripted manager; the `registration` suite proves the declaration registers
the type through the provider registry with its decoder and factory, that a
duplicate registration is refused, and that the declared mask cannot arrive
after the plane opens.
