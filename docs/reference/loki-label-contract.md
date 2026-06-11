# Retired Loki label contract

Status: historical reference for the pre-SigNoz observability backend.

Current nixling observability no longer emits a Loki backend or
`loki.source.*` Alloy pipeline. Logs flow through OpenTelemetry
Collector pipelines into the SigNoz OTel Collector and ClickHouse.

The live contract is:

- keep `vm.name`, `vm.env`, `vm.role`, `service.name`, and severity
  attributes low-cardinality;
- preserve trace and span IDs as OTel log record correlation fields, not
  metric labels;
- drop or demote path-like and unit-like attributes before telemetry
  reaches ClickHouse when they would create unbounded cardinality;
- scrub obvious token, secret, password, and API-key patterns at the
  edge collector before telemetry crosses a VM boundary.

See:

- [`components-observability.md`](components-observability.md)
- [`tracing-contract.md`](tracing-contract.md)

The old Loki label gate remains useful only for historical branches that
still carry the Grafana/Prometheus/Loki/Tempo stack.
