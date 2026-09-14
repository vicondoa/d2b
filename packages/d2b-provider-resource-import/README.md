# `d2b-provider-resource-import`

This is the crate root for the `ResourceImport` resource type. It owns the type's identity
and the driver declaration the v3 resource plane registers the type by; the
driver itself, its spec decoder, and its factory are the shared declaration-only
metadata driver of `d2b-resource-runtime`.

`ResourceImport` declares that a remote resource is imported into this zone. The driver converges it as metadata once its desired state is admitted; the imported resource's own family realizes it.

## Provider identity

| Field | Value |
| --- | --- |
| Provider name | `resource-import` |
| ResourceType | `ResourceImport` |
| Package | `packages/d2b-provider-resource-import/` |
| Driver declaration | `resource_import_descriptor` -> `DriverDescriptor` |

## Config schema

The type declares no provider config schema: `ResourceImport` rows carry the JSON spec
object the core rows store, and nothing outside it decodes at validate.

## Exported resource types

`ResourceImport` is not exportable. `ResourceExport` admits only qualified
`*.d2bus.org.*Service` types, so a `ResourceImport` row is never an export subject. The
declaration carries `exportable: false`.

## Controllers / services / workers / binaries

The crate ships the type's declaration, not a standalone process. The shared
driver serves `ResourceImport` rows through the `ResourceDriver` verbs: `validate` decodes the
stored spec envelope and admits the JSON spec object every core row stores,
`recover` adopts the converged row, `reconcile` converges it as metadata,
`finalize` drains owned children, and `delete` retires the row.

The registration surface is `resource_import_descriptor`: it declares the type through the shared
declaration of the declaration-only metadata types, which carries the decoder,
the type's verbs, execution domains, reads, and the `BUILTIN` allowed-source
mask the plane's presence obligation reads.

## Placement and dependencies

`ResourceImport` names no placement anchor, so a `ResourceImport` row is reconciled on its
containing Zone's Host.

The crate depends only on `d2b-resource-types`, which carries the type's
declaration, and on `tokio` for its registration test.

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
cargo test -p d2b-provider-resource-import
```

The `registration` suite proves the declaration registers the type through the
provider registry with its decoder and factory, that a duplicate registration is
refused, and that the declared mask cannot arrive after the plane opens. The
driver behavior itself is covered once, by the shared driver's own tests in
`d2b-resource-runtime`.
