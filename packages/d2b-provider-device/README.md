# `d2b-provider-device`

This is the crate root for the `Device` resource type. It owns the type's
driver, its spec decoder, and the driver declaration the v3 resource plane
registers the type by.

The `Device` ResourceType is served by four hardware Providers, and the plane
keys exactly one driver per ResourceType, so the crate declares the type once
over the four declared rows:

| Row | Provider | Controller |
| --- | --- | --- |
| TPM | `Provider/device-tpm` | `Process/device-tpm-controller` |
| USBIP | `Provider/device-usbip` | `Process/device-usbip-controller` |
| Security key | `Provider/device-security-key` | `Process/device-security-key-controller` |
| GPU | `Provider/device-gpu` | `Process/device-gpu-controller` |

Each row's Provider identity is the realizer crate's exported `PROVIDER_REF`
constant, so a Provider rename is a compile-time change instead of a silent
string edit.

## Provider identity

| Field | Value |
| --- | --- |
| Provider name | n/a - this is a type crate, not a Provider |
| ResourceType | `Device` |
| Package | `packages/d2b-provider-device/` |
| Driver declaration | `device_descriptor` -> `DriverDescriptor` |

## Config schema

The type declares no provider config schema: a `Device` row carries the
declaring Provider's own extension schema (each realizer crate exports the
schema identifier its rows are validated against), and nothing outside the
row's `providerRef`-selected contract decodes at validate.

## Exported resource types

`Device` is not exportable. `ResourceExport` admits only qualified
`*.d2bus.org.*Service` types, so a device is never an export subject. The
declaration carries `exportable: false`.

## Controllers / services / workers / binaries

The crate ships one driver factory, not a standalone process. The driver
serves `Device` rows through the `ResourceDriver` verbs: `validate` decodes
the stored spec and admits only the four declared Providers, `recover` adopts
the Provider-side realization, `reconcile` runs the row's typed effect behind
`DeviceDriverEffects`, `finalize` drains owned children, and `delete` runs the
Provider's teardown stage before the owned subtree retires.

The Device rows declare no manager children of their own: the Zone bundle
declares the Device-owned worker rows (`Process/swtpm-<device>`,
`Process/gpu-<device>`) and each family's effect ensures its own
controller-created rows (the TPM state Volume) through the child surface. A
reconcile pass therefore never diffs - and never retires - the rows another
layer declared.

## Placement and dependencies

A `Device` row is reconciled on its containing Zone's Host; the guest-side
work is the worker rows the bundle declares and the providers' controllers
supervise. The daemon implements `DeviceDriverEffects` (the TPM controller,
the USBIP and security-key device admission, the GPU authority fence) and
passes its own per-resource `DeviceResourceState` back through the driver.

The crate depends on the four realizer crates for their exported identities,
on `d2b-provider-toolkit` for the shared driver machinery, and on
`d2b-resource-runtime`/`d2b-resource-types` for the driver contract.

## RBAC requirements

The driver holds no broker authority of its own: it runs inside the daemon's
per-zone plane, and every privileged mutation stays in the daemon's effect
implementation behind the port. A spec naming a Provider outside the four
rows is refused closed (`shared-provider-spec-invalid`).

## Security posture

The driver never invents a Provider, controller, or child identity from spec
text: the admitted Providers are the four declared rows, the device-class
admission stays in the daemon's effects, and a spec that fails to decode is a
terminal refusal rather than a best-effort teardown.

## State and telemetry

The per-resource `DeviceResourceState` (the TPM and GPU controllers plus the
GPU authority leases) lives with the driver instance and is never persisted:
after a restart the controllers rehydrate from fresh evidence exactly as the
old in-memory maps did. Status is in-memory only, and failures travel as
registered failure kinds on the structured failure surface.

## Build and test

```bash
cargo test -p d2b-provider-device
```

The hermetic suite drives the four rows over a recording Provider port; the
`integration/device_family.rs` scenario checks the registration shape the
plane consumes (one declaration for `Device`, four rows).
