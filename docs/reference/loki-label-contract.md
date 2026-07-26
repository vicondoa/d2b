# Retired Loki label contract

Status: historical reference for the pre-SigNoz observability backend.

Current d2b observability no longer emits a Loki backend or
`loki.source.*` Alloy pipeline. Logs flow through OpenTelemetry
Collector pipelines into the SigNoz OTel Collector and ClickHouse.

The live contract is codified by
[`packages/d2b-contract-tests/tests/policy_observability.rs`](../../packages/d2b-contract-tests/tests/policy_observability.rs).
That policy is advisory until the fixture-contract lane is enabled and
promoted. It covers the OTel resource-attribute contract for the native SigNoz
path:

- Resource attribute keys in d2b-managed collector configs are
  allowlisted to:
  - `deployment.environment`
  - `host.name`
  - `service.name`
  - `service.namespace`
  - `source`
  - `vm.env`
  - `vm.name`
  - `vm.role`
- `vm.name`, `vm.env`, and `vm.role` are stamped authoritatively by the
  collector path. Workload-supplied values cannot override the
  source-specific `sys-obs` receiver identity.
- `service.name` is preserved for workload OTLP telemetry. Collector
  self-metrics and host StoreSync export records use dedicated pipelines
  that set their own `service.name`.
- `source = "store-sync-audit"` is reserved for the host StoreSync
  observability export. Target VM/env stay in JSON content as
  `target_vm` / `target_env`, not resource attributes.
- Resource attribute keys must not carry secrets, argv/cmdline text,
  command output, or `/nix/store` paths.

See:

- [`components-observability.md`](components-observability.md)
- [`tracing-contract.md`](tracing-contract.md)
- [`daemon-metrics.md`](daemon-metrics.md)

The old Loki label contract is retained here only for migration context:
historical branches that still carry the Grafana/Prometheus/Loki/Tempo
stack should use their branch-local Loki label gate. New d2b changes
must follow the OTel resource-attribute contract above.
