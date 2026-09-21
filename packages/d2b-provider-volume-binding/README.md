# `d2b-provider-volume-binding`

This is the crate root for the `VolumeBinding` resource type. It owns the
type's driver, its spec decoder, the read-side helpers over a stored binding
row, and the driver declaration the v3 resource plane registers the type by.

## Provider identity

| Field | Value |
| --- | --- |
| Provider name | `volume-virtiofs` (the Provider that owns a binding row) |
| ResourceType | `VolumeBinding` |
| Package | `packages/d2b-provider-volume-binding/` |
| Driver declaration | `binding_descriptor` -> `DriverDescriptor` |

## Config schema

The type declares no provider config schema: a `VolumeBinding` row carries the
closed `VolumeBindingSpec` contract from `d2b-contracts-resource`
(`volumeRef`, `executionRef`, `view`, `access`, `mountPath`) and nothing
outside it decodes at validate. The view a binding selects is resolved from
the owning `Volume` row, never from the binding's own text.

## Exported resource types

`VolumeBinding` is not exportable. `ResourceExport` admits only qualified
`*.d2bus.org.*Service` types, so a binding is never an export subject. The
declaration carries `exportable: false`.

## Controllers / services / workers / binaries

The crate ships one driver factory, not a standalone process. The driver
serves `VolumeBinding` rows through the `ResourceDriver` verbs: `validate`
decodes the stored spec, checks the serving Provider, and applies the owner
fence against the declared parent `Volume` row; `recover` re-derives the
worker plan and adopts the owned-child realization; `reconcile` derives the
launch plan, ensures the worker Process child and then the Endpoint child,
retires obsolete children endpoint-first, registers the dependency watches,
and publishes the fenced readiness projection; `finalize` drains owned
children; `delete` observes the guest mount before anything is removed and
tears the binding down endpoint-first / process-last.

`BindingDriverFactory` is the registration surface; `binding_descriptor`
carries it with the decoder, the type's verbs, execution domains, reads, the
worker Process and Endpoint creations the driver may make, and the
`BUILTIN | STARTUP` allowed-source mask. The two minted children select their
providers from those crates' own references (`Provider/system-minijail` for
the worker Process, `Provider/volume-virtiofs` for the Endpoint).

## Placement and dependencies

`VolumeBinding` names no placement anchor, so a binding row is reconciled on
its containing Zone's Host. The declared `executionRef` names the Guest that
consumes the share; the worker itself executes on the Host, exactly as the
signed `virtiofsd-worker` template the launch ticket resolves binds it.

The crate depends on `d2b-contracts-resource`, `d2b-provider-process-minijail`
(the worker Provider reference), `d2b-provider-volume-virtiofs` (the binding
row contract, the frozen worker plan, and the binding Provider reference),
`d2b-resource-runtime`, and `d2b-resource-types`. The family's driver effects
are implemented by this crate itself (`effects_service`); the daemon-owned
reads - the serving-socket probe, the socket removal, and the guest-mount
observation - cross the provider boundary as the declared `BindingEffectFacets`
the composition root supplies, so the crate carries neither a socket path nor
a mount observation of its own. The daemon hosts the family's declared
effects service (`volume-binding.d2bus.org/effects`) per zone from the
family's registered factory.

## RBAC requirements

The driver holds no broker authority of its own: it runs inside the daemon's
per-zone plane, and every row it reads or writes is reached through the
manager with the plane's own caller identity. A binding whose declared parent
`Volume` is the row the manager reports but with another owner uid is refused
closed (`binding-owner-mismatch`) rather than silently re-parented.

## Security posture

The driver never invents a socket path, worker argv, or mount source: the
stored spec is decoded strictly, the launch plan comes from the frozen
`VirtiofsdWorkerPlan` contract, and the worker child is minted argv-free so
the Process controller composes its launch from the committed binding and
Volume rows. The drain gate fails closed: a guest mount the observation
cannot see keeps the durable deleting mark and the owned children rather
than force-clearing a share that is still mounted. Effects are idempotent under
retry.

## State and telemetry

The type publishes no durable status of its own; the fenced
`VolumeBindingStatusResource` projection the actor writes is the wire-visible
readiness, and `BindingDriverStatus` (`ServingChildren`, `RecoveredPlan`,
`Rejected`) is the in-memory projection of the derived plan. Failures travel
as registered failure kinds (`binding-spec-invalid`,
`binding-provider-unsupported`, `binding-owner-mismatch`,
`binding-parent-unavailable`, `binding-parent-spec-invalid`,
`binding-plan-derivation-invalid`, `binding-serving-effect-failed`,
`binding-child-mutation-failed`, and the shared `children-draining`).

## Build and test

```bash
cargo test -p d2b-provider-volume-binding
```

The unit tests drive validate, recover, reconcile, finalize, and delete over a
scripted serving port and a recording manager endpoint with one shared ordered
log, so commit-before-spawn and endpoint-first teardown are asserted as the
manager records them. The `registration` suite proves the declaration
registers the type through the provider registry with its decoder and factory,
licenses exactly the worker Process and Endpoint creations with their
providers and ranks, refuses a duplicate registration, and cannot arrive after
the plane opens.
