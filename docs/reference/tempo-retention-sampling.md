# Retired Tempo retention and sampling policy

Status: historical reference for the pre-SigNoz observability backend.

Current d2b observability no longer emits Tempo tenants,
Tempo-specific block retention, or Grafana Tempo datasources. Traces flow
through OpenTelemetry Collector pipelines into the SigNoz OTel Collector
and ClickHouse.

Current policy:

- edge collectors do not sample;
- edge collectors preserve the critical-decision attributes needed by
  central trace policy;
- the central SigNoz collector computes RED/span metrics from the
  unsampled stream before any trace-storage sampling branch;
- `kind=critical` remains an OTel attribute, but the old
  Tempo-tenant-specific retention model is not automatically migrated;
- ClickHouse/SigNoz retention is the backend-retention control surface.
  The legacy `d2b.observability.retention.*` and `sampling.*`
  options are compatibility shims and currently warn when changed; they
  do not configure ClickHouse TTL.

Historical Tempo data is not migrated to SigNoz automatically. Preserve
the old `sys-obs-stack` state until the new `sys-obs` stack is validated
and rollback is no longer required.

See:

- [`components-observability.md`](components-observability.md)
- [`enable observability`](../how-to/enable-observability.md)
