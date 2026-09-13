# `d2b-provider-telemetry-binding`

This is the crate root for the `telemetry.d2bus.org.TelemetryBinding`
resource type. It owns the type's driver, its spec decoder, and the driver
declaration the v3 resource plane registers the type by.

The Binding is a local producer intent: it attaches a telemetry Service to a
same-Zone `Zone` or `Guest` producer. The Serving Provider declares the
child set (the host-placed collector, the optional vsock forwarder, and one
`Endpoint` per worker), Core owns the child body, and the Binding owns the
child lifecycle.

## Provider identity

| Field | Value |
| --- | --- |
| Provider name | `observability-otel` |
| ResourceType | `telemetry.d2bus.org.TelemetryBinding` |
| Package | `packages/d2b-provider-telemetry-binding/` |
| Driver declaration | `telemetry_binding_descriptor` -> `DriverDescriptor` |

## Config schema

The type declares no provider config schema of its own: a Binding row carries
the frozen telemetry Binding base from `d2b-contracts-provider`
(`semantic_services::telemetry`) - `providerRef`, `serviceRef`,
`producerRef`, `signals`, `quota`, and `policy`.

## Exported resource types

`telemetry.d2bus.org.TelemetryBinding` is not exportable: `ResourceExport`
admits only qualified `*.d2bus.org.*Service` types, so a binding is never an
export subject. The declaration carries `exportable: false`.

## Controllers / services / workers / binaries

The crate ships one driver factory, not a standalone process. The driver
serves Binding rows through the `ResourceDriver` verbs: `validate` decodes the
stored spec, `recover` adopts only a current owned child set, `reconcile`
ensures the provider-declared children and retires the ones the desired set no
longer derives, `finalize` drains the owned children, and `delete` leaves the
teardown to the manager's cascade.

`TelemetryBindingDriverFactory` is the registration surface;
`telemetry_binding_descriptor` carries it with the decoder, the type's verbs,
execution domains, reads, the `BUILTIN | STARTUP` allowed-source mask, and the
two declared child creations
(`TELEMETRY_BINDING_COLLECTOR_CREATION`, `TELEMETRY_BINDING_ENDPOINT_CREATION`).

## Placement and dependencies

`TelemetryBinding` names no placement anchor, so a Binding row is reconciled
on its containing Zone's Host domain, and the telemetry Provider declares
every child host-placed. The child shapes come from the Serving Provider
(`TelemetryBindingController::child_resources` in
`d2b-provider-observability-otel`) and the child body from Core
(`materialize_child_create_payload` in `d2b-core-controller`), so the closed
child set cannot drift from what the Provider commits.

## RBAC requirements

The driver holds no broker authority of its own: it runs inside the daemon's
per-zone plane, and every row it reads or writes is reached through the
manager with the plane's own caller identity. A Binding that names another
Provider, or whose Service/target relationship is malformed or dangling, is
fenced: no child mutation, `Degraded` projection.

## Security posture

The driver never invents a child: the resource refs, roles, placements, and
Process templates are the Provider's signed declaration, and an Endpoint child
is only ever produced by a Process the same declaration names. Teardown is
endpoint-first / process-last, and every effect is idempotent under retry.

## State and telemetry

The type publishes no durable status: `TelemetryBindingStatus` is in-memory
only (the plane's status rule), carrying the phase, the fence, the convergence
flag, and the desired child refs. Failures travel as
`TelemetryBindingDriverError` codes (`semantic-binding-resource-invalid`,
`semantic-binding-relationship-invalid`,
`semantic-binding-reconcile-failed`) and are classified retryable, exactly as
the preserved reconciler classified them.

## Build and test

```bash
cargo test -p d2b-provider-telemetry-binding
```

The unit tests drive validate, recover, reconcile, finalize, and delete over a
recording manager endpoint; the `registration` suite proves the declaration
registers the type through the provider registry with its decoder and factory,
that a duplicate registration is refused, and that the declared mask cannot
arrive after the plane opens.
