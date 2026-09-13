# `d2b-provider-volume`

This is the crate root for the `Volume` resource type. It owns the type's
driver, its spec decoder, and the driver declaration the v3 resource plane
registers the type by.

## Provider identity

| Field | Value |
| --- | --- |
| Provider name | `volume-local` (the fixed Provider a Volume row selects) |
| ResourceType | `Volume` |
| Package | `packages/d2b-provider-volume/` |
| Driver declaration | `volume_descriptor` -> `DriverDescriptor` |

## Config schema

The type declares no provider config schema: a `Volume` row carries the closed
`VolumeSpec` contract from `d2b-contracts-resource` (`source`, `kind`,
`layout`, `views`, `attachments`) and nothing outside it decodes at validate.
The stored envelope's `provider` extension travels to the layout effect
unchanged and is never interpreted here.

## Exported resource types

`Volume` is not exportable. `ResourceExport` admits only qualified
`*.d2bus.org.*Service` types, so a volume is never an export subject. The
declaration carries `exportable: false`.

## Controllers / services / workers / binaries

The crate ships one driver factory, not a standalone process. The driver
serves `Volume` rows through the `ResourceDriver` verbs: `validate` decodes
the stored spec and admits only the fixed `volume-local` Provider, `recover`
probes the durable layout state, `reconcile` runs the preserved layout effect
as a long effect and then derives and ensures one deterministic
`VolumeBinding` child per admitted virtiofs attachment, `finalize` drains
owned children, and `delete` removes the Volume's own layout state.

`VolumeDriverFactory` is the registration surface; `volume_descriptor`
carries it with the decoder, the type's verbs, execution domains, reads, the
`VolumeBinding` creation the driver may make, and the `BUILTIN | STARTUP`
allowed-source mask.

## Placement and dependencies

`Volume` names no placement anchor, so a volume row is reconciled on its
containing Zone's Host. A source reference or an attachment's execution
reference selects where a share is served, never where the row is reconciled;
a binding child targeting a Guest is reconciled by that child's own driver.

The crate depends on `d2b-contracts-resource`, `d2b-provider-volume-local`
(the layout intents and the preserved controller profile the daemon's effects
drive), `d2b-provider-volume-virtiofs` (the Provider reference the derived
binding children select), `d2b-resource-runtime`, and `d2b-resource-types`.
The layout effect and the durable layout probe arrive through
`VolumeDriverEffects`, so the crate carries no host state and no effect
implementation.

## RBAC requirements

The driver holds no broker authority of its own: it runs inside the daemon's
per-zone plane, and every row it reads or writes is reached through the
manager with the plane's own caller identity. A Volume row that selects any
Provider other than `volume-local` is refused closed
(`volume-provider-unsupported`) at validate, recover, and reconcile.

## Security posture

The driver never invents a layout path, marker, or identity from spec text:
the stored spec is decoded strictly, the admitted attachments are the ones
`volume-local` admits, and the layout effect runs behind the daemon's port,
which resolves anchored descriptors from trusted policy. Effects are
idempotent under retry: a repeated ensure derives the same child identities,
and a repeated delete removes layout state that is already absent without
failing. A spec that fails to decode is a terminal refusal rather than a
best-effort teardown.

## State and telemetry

The type publishes no durable status: the in-memory `VolumeDriverStatus`
(`EnsuringLayout`, `ServingChildren`) is the only status projection, matching
the plane's in-memory status rule. Failures travel as registered failure kinds
(`volume-spec-invalid`, `volume-provider-unsupported`,
`volume-layout-effect-failed`, `volume-layout-not-ready`,
`volume-child-mutation-failed`, `volume-child-derivation-invalid`, and the
shared `children-draining`) on the structured failure surface, which is what
the daemon logs and what tests assert.

## Build and test

```bash
cargo test -p d2b-provider-volume
```

The unit tests drive validate, recover, reconcile, finalize, and delete over a
scripted effect port and a recording manager endpoint; the `registration`
suite proves the declaration registers the type through the provider registry
with its decoder and factory, licenses exactly the derived `VolumeBinding`
creation, refuses a duplicate registration, and cannot arrive after the plane
opens.
