# `d2b-provider-device-binding`

This is the crate root for the `DeviceBinding` resource type. It owns the
type's driver, its spec decoder, the read-side helpers over a committed
binding row, the family's driver effects, and the driver declaration the v3
resource plane registers the type by.

## Provider identity

| Field | Value |
| --- | --- |
| Provider name | `device-binding` (the Provider that serves a binding row) |
| ResourceType | `DeviceBinding` |
| Package | `packages/d2b-provider-device-binding/` |
| Driver declaration | `binding_descriptor` -> `DriverDescriptor` |
| Effects service | `device-binding.d2bus.org/effects` |

## Config schema

The type declares no provider config schema. A `DeviceBinding` row carries the
closed `DeviceBindingSpec` contract from `d2b-contracts-resource`
(`deviceRef`, `executionRef`, `slot`, `function`, `claim`) and nothing outside
it decodes at validate. The consumer is whatever the binding kind admits, not a
fixed `Guest`. The physical authority key is not authored here: it comes from
the Device provider's trusted inventory, so device permission is never derived
from a seccomp name, a launch role, or a host path.

## Exported resource types

`DeviceBinding` is not exportable. `ResourceExport` admits only qualified
`*.d2bus.org.*Service` types, so a binding is never an export subject. The
declaration carries `exportable: false`.

## Controllers / services / workers / binaries

The crate ships one driver factory, not a standalone process. The driver
serves `DeviceBinding` rows through the `ResourceDriver` verbs:

- `validate` decodes the stored envelope and the canonical request strictly,
  and applies the owner fence against the declared parent `Device` row;
- `recover` adopts the realization the mediation still holds after a restart
  and reports missing when it did not survive, so a restart never attaches a
  consumer twice;
- `reconcile` resolves the parent `Device`, watches it, drives the claim and
  its attachment through the effect port, and publishes the fenced readiness
  projection plus the in-memory status;
- `pre_drain` removes the attachment the consumer was holding before the row's
  own teardown;
- `delete` observes the drain gate before anything is released, then tears
  down attachment-first and releases the device slot last.

`DeviceBindingDriverFactory` is the registration surface; `binding_descriptor`
carries it with the decoder, the type's verbs, execution domains, the read
set, the empty creation set, and the `BUILTIN | STARTUP` allowed-source mask.

The driver reads the parent `Device` the row names (`DEVICE_BINDING_READS` is
`[Device]`): a Device that has been deleted, re-created under another owner,
or replaced by a row that does not decode leaves a binding whose claim can no
longer be proven, so the row defers or refuses instead of keeping it. The
named capability itself is still resolved against the Device provider's
trusted inventory through the effect port.

The family declares no child creation (`DEVICE_BINDING_CREATIONS` is empty).
The realized attachment is a mediation over the Device provider's own trusted
inventory, whose worker rows the Zone bundle declares and whose Endpoints the
Device driver declares; a binding row that minted an attachment surface of its
own would be a second authority over the same realization.

## Placement and dependencies

`DeviceBinding` names no placement anchor, so a binding row is reconciled on
its containing Zone's Host. The request's consumer reference names who the
capability reaches, never where the binding row itself is reconciled.

The crate depends on `d2b-contracts-resource`, `d2b-provider-toolkit`,
`d2b-resource-runtime`, and `d2b-resource-types`. The family's driver effects
are implemented by this crate itself (`effects_service`); the device
mediation and the observation over the realized attachment cross the provider
boundary as the declared `DeviceBindingEffectFacets` the composition root
supplies, so the crate carries neither a device node path nor a host
permission bit of its own. The daemon hosts the family's declared effects
service (`device-binding.d2bus.org/effects`) per zone from the family's
registered factory.

## RBAC requirements

The driver holds no broker authority of its own: it runs inside the daemon's
per-zone plane, and every effect it drives is named by the row's own committed
request. A row the trusted adapter refuses as unauthorized or stale is
terminal (`driver-refused`) and drives nothing further, rather than realizing
a capability the source could not prove.

## Security posture

The driver never invents a device node path, serial, bus id, numeric
principal, or host permission bit: the stored spec is decoded strictly, the
attachment identity is derived from the committed request and the row's own
fence, and the physical authority stays with the Device provider's trusted
inventory. The drain gate fails closed: an attachment the consumer still holds
keeps the durable deleting mark and the claim rather than yanking a live
consumer off a capability. Both releases are idempotent under retry.

## State and telemetry

The type publishes no durable status of its own; the fenced
`BindingReadiness` projection the actor writes is the wire-visible readiness,
and `BindingDriverStatus` (`Realized`, `Recovered`, `Rejected`) is the
in-memory projection of the attachment the pass drove. Failures travel as
registered failure kinds (`binding-spec-invalid`, `driver-refused`,
`driver-not-yet`, `binding-serving-effect-failed`,
`binding-parent-unavailable`, `binding-owner-mismatch`,
`binding-parent-spec-invalid`, and the shared `children-draining` the generic
finalization raises).

## Build and test

```bash
cargo test -p d2b-provider-device-binding
```

The unit tests drive validate, recover, reconcile, pre-drain, and delete over a
scripted mediation port and a recording manager endpoint, so the drain gate
and the attachment-first release are asserted as the two record them. The
`registration` suite proves the declaration registers the type through the
provider registry with its decoder and factory, licenses no child creation,
refuses a duplicate registration, and cannot arrive after the plane opens.