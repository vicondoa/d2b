# `d2b-provider-telemetry-service`

This is the crate root for the `telemetry.d2bus.org.TelemetryService`
resource type. It owns the type's driver, its spec decoder, and the driver
declaration the v3 resource plane registers the type by.

The Service is a Zone's telemetry ingest authority or a Core-owned projection
of a remote one. It realizes nothing on a target: its evidence is the durable
ingest `Endpoint` rows the driver re-reads, and it owns no child.

## Provider identity

| Field | Value |
| --- | --- |
| Provider name | `observability-otel` |
| ResourceType | `telemetry.d2bus.org.TelemetryService` |
| Package | `packages/d2b-provider-telemetry-service/` |
| Driver declaration | `telemetry_service_descriptor` -> `DriverDescriptor` |

## Config schema

The type declares no provider config schema of its own: a Service row carries
the frozen telemetry Service base from
`d2b-contracts-provider` (`semantic_services::telemetry`) - `providerRef`,
`serviceRole`, `ingestEndpointRefs`, `signals`, `quota`, and `policy`. OTEL,
OTLP, and backend-product choices belong only to a Provider's strict
`spec.provider` extension and never reach this driver.

## Exported resource types

`telemetry.d2bus.org.TelemetryService` is exportable: `ResourceExport` admits
qualified `*.d2bus.org.*Service` types, and this is exactly that shape. The
declaration carries `exportable: true`.

## Controllers / services / workers / binaries

The crate ships one driver factory, not a standalone process. The driver
serves Service rows through the `ResourceDriver` verbs: `validate` decodes the
stored spec, `recover` adopts (a Service has no target realization), and
`reconcile` projects the provider phase from the declared ingest routes.

`TelemetryServiceDriverFactory` is the registration surface;
`telemetry_service_descriptor` carries it with the decoder, the type's verbs,
execution domains, reads, and the `BUILTIN | STARTUP` allowed-source mask.

## Placement and dependencies

`TelemetryService` names no placement anchor, so a Service row is reconciled
on its containing Zone's Host domain. The crate depends only on
`d2b-contracts-provider`, `d2b-contracts-resource`, `d2b-resource-runtime`,
and `d2b-resource-types`; the Serving Provider's own controllers stay in
`d2b-provider-observability-otel`.

## RBAC requirements

The driver holds no broker authority of its own: it runs inside the daemon's
per-zone plane, and every row it reads is reached through the manager with
the plane's own caller identity. A `serviceRole` outside the admitted
`authority`/`projection` pair is reported `Degraded` with an empty projection
and mutates nothing.

## Security posture

The driver never fabricates a route: an ingest endpoint ref that does not
resolve to a live row leaves the Service `Pending`, and the readiness term is
fail-closed (`DEPENDENCY_READINESS_PROVEN`) until the driver surface carries a
dependency's observed status, so `Ready` is never claimed without evidence.
The projection carries only the closed `{serviceRole, serviceReadiness}` pair,
never spec text.

## State and telemetry

The type publishes no durable status: `TelemetryServiceStatus` is in-memory
only (the plane's status rule), carrying the phase, the projection, and the
present endpoint refs. Failures travel as `TelemetryServiceDriverError` codes
(`semantic-binding-resource-invalid`, `semantic-binding-reconcile-failed`) and
are classified retryable, exactly as the preserved reconciler classified them.

## Build and test

```bash
cargo test -p d2b-provider-telemetry-service
```

The unit tests drive validate, recover, reconcile, and delete over a recording
manager endpoint; the `registration` suite proves the declaration registers
the type through the provider registry with its decoder and factory, that a
duplicate registration is refused, and that the declared mask cannot arrive
after the plane opens.
