# Zone bus observability

The Zone bus emits bounded metrics through the shared `d2b-telemetry`
`BoundedEmitter`. Metrics are best-effort and never carry resource names,
subjects, paths, operation identifiers, payloads, or realm data.

## Metric inventory

| Metric | Kind | Labels |
| --- | --- | --- |
| `d2b_bus_route_total` | counter | `service`, `direction`, `outcome` |
| `d2b_bus_route_duration_seconds` | histogram | `service`, `direction` |
| `d2b_bus_session_active` | gauge | `transport` |
| `d2b_bus_registration_total` | counter | `direction`, `outcome` |
| `d2b_bus_stream_active` | gauge | `direction` |
| `d2b_bus_stream_total` | counter | `direction`, `outcome` |
| `d2b_bus_credit_bytes` | gauge | `direction` |
| `d2b_bus_backpressure_total` | counter | `direction`, `kind`, `reason` |
| `d2b_bus_rejection_total` | counter | `direction`, `outcome` |
| `d2b_bus_disconnect_total` | counter | `direction`, `outcome` |

`direction` is rendered only from the closed `BusDirection` enum:
`local`, `host`, `guest`, or `zone_link`. Service names are reduced to the
closed service catalog, with unknown values grouped into `bus`.

Route latency uses fixed boundaries:
`0.0001`, `0.0005`, `0.001`, `0.005`, `0.01`, `0.05`, and `0.1` seconds.
