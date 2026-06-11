# `nixling.observability.*` / `nixling.vms.<vm>.observability.*`

Reference for the bundled native SigNoz observability component.

Option sources:

- [`nixos-modules/options-observability.nix`](../../nixos-modules/options-observability.nix)
- [`nixos-modules/options-vms.nix`](../../nixos-modules/options-vms.nix)

Implementation modules:

- [`nixos-modules/observability-vm.nix`](../../nixos-modules/observability-vm.nix)
- [`nixos-modules/components/observability/{host,guest,stack}.nix`](../../nixos-modules/components/observability/)

## Overview

Set `nixling.observability.enable = true` to auto-declare the `obs` env
and the `sys-obs` observability VM. Set
`nixling.vms.<vm>.observability.enable = true` on each workload VM that
should send telemetry.

The bundled backend is native SigNoz:

- ClickHouse stores telemetry.
- ClickHouse Keeper coordinates the single-node ClickHouse cluster used
  by SigNoz's replicated schema.
- SigNoz serves the UI and API.
- SigNoz OTel Collector ingests OTLP and writes logs, metrics, traces,
  and metadata to ClickHouse.

No Docker, Podman, Kubernetes, Helm, or compose deployment is emitted.

## Data path

```text
workload VM
  nixling-otel-collector.service
    -> /run/nixling/otel/otlp-egress.sock
    -> nixling-otel-vsock-out.service
    -> host CH-vsock relay
    -> sys-obs nixling-otel-vsock-in-<vm>.service
    -> signoz-otel-collector.service
    -> ClickHouse

host
  nixling-host-otel-collector.service
    -> /run/nixling/otel/host-egress.sock
    -> broker-spawned OtelHostBridge
    -> sys-obs nixling-otel-vsock-in-host.service
    -> signoz-otel-collector.service
    -> ClickHouse
```

Telemetry uses Unix sockets and vsock. It does not traverse workload env
LAN routing.

## Host-level options

| Option | Type | Default | Meaning |
| --- | --- | --- | --- |
| `nixling.observability.enable` | bool | `false` | Enable the bundled observability stack. |
| `nixling.observability.env` | str | `"obs"` | Auto-declared observability env name. |
| `nixling.observability.vmName` | str | `"sys-obs"` | Auto-declared observability VM name. |
| `nixling.observability.index` | int | `10` | LAN index for `sys-obs`. |
| `nixling.observability.lanSubnet` | str | `"10.40.0.0/24"` | Observability LAN CIDR. |
| `nixling.observability.uplinkSubnet` | str | `"203.0.113.0/30"` | Host↔obs point-to-point CIDR. |
| `nixling.observability.signoz.listenAddress` | str | derived obs IP | SigNoz UI bind address. |
| `nixling.observability.signoz.listenPort` | port | `8080` | SigNoz UI port. |
| `nixling.observability.signoz.otlpGrpcPort` | port | `4317` | SigNoz collector loopback OTLP gRPC port. |
| `nixling.observability.signoz.otlpHttpPort` | port | `4318` | SigNoz collector loopback OTLP HTTP port. |
| `nixling.observability.signoz.adminEmail` | str | `"admin@nixling.local"` | Root SigNoz admin email. |
| `nixling.observability.signoz.jwtSecretFile` | path or string or null | `null` | Optional host path for the SigNoz JWT/tokenizer secret. |
| `nixling.observability.signoz.rootPasswordFile` | path or string or null | `null` | Optional host path for the SigNoz root password. |
| `nixling.observability.signoz.clickhousePasswordFile` | path or string or null | `null` | Optional host path for the ClickHouse password used by SigNoz services. |
| `nixling.observability.transport.relayPackage` | package | `pkgs.socat` | Socat-compatible relay package for vsock bridges. |

Legacy Grafana/Tempo/Loki/Prometheus-specific options are retired or
kept only as migration shims. Do not use them for new configurations.
`retention.*` and `sampling.*` currently warn when changed and do not
configure SigNoz/ClickHouse TTL.

## Per-VM options

| Option | Type | Default | Meaning |
| --- | --- | --- | --- |
| `nixling.vms.<vm>.observability.enable` | bool | `false` | Enable telemetry collection for this VM. |
| `nixling.vms.<vm>.observability.scrapeJournal` | bool | `false` | Compatibility toggle reserved for future guest journald collection. |
| `nixling.vms.<vm>.observability.scrapeNodeMetrics` | bool | `true` | Enable guest hostmetrics collection. |

## Runtime services

Host:

- `nixling-host-otel-collector.service`
- broker-spawned `RunnerRole::OtelHostBridge` with process role
  `otel-host-bridge`

Workload VM:

- `nixling-otel-collector.service`
- `nixling-otel-vsock-out.service`

`sys-obs`:

- `clickhouse.service`
- `clickhouse-keeper.service`
- `signoz-schema-migrate-sync.service`
- `signoz-schema-migrate-async.service`
- `signoz.service`
- `signoz-otel-collector.service`
- `nixling-otel-vsock-in-host.service`
- `nixling-otel-vsock-in-<vm>.service` for each observed workload

## Host StoreSync observability export

The privileged broker emits a StoreSync-only telemetry export at:

```text
${nixling.site.stateDir}/observability/store-sync/store-sync-<utc-date>.jsonl
```

The host OTel collector tails `store-sync-*.jsonl` with a `filelog`
receiver and forwards those records through the same host→`sys-obs`
OTLP bridge as host metrics. This export is a positive allow-list
projection, not the broker audit log. Host-confidential fields
(`caller_principal`, retained generation lists, host/store paths,
`db.dump`, marker payloads) are redacted by construction in the broker.
The target VM/env stay in JSON content (`target_vm` / `target_env`) and
are not promoted to resource attributes.

The collector identity gets focused read/traverse ACLs on the StoreSync
export directory only. It is not added to the `nixlingd` group and gets
no access to the unified broker audit log, privileged daemon socket, or
other broker state. Static gates:

- [`tests/store-sync-export-eval.sh`](../../tests/store-sync-export-eval.sh)
- [`tests/loki-label-cardinality-eval.sh`](../../tests/loki-label-cardinality-eval.sh)

## Socket and port contract

| Resource | Value |
| --- | --- |
| Obs VM vsock CID | `1000` |
| Workload observability CID | `100 + envIndex * 100 + vm.index` |
| Host obs ingress vsock port | `14317` |
| Workload obs ingress vsock ports | `14318+`, one per observed VM |
| Host collector egress | `/run/nixling/otel/host-egress.sock` |
| Guest local OTLP | `/run/nixling/otel/otlp.sock` with compatibility symlink `/run/nixling/otlp.sock` |
| Guest relay handoff | `/run/nixling/otel/otlp-egress.sock` |
| SigNoz UI | `signoz.listenAddress:signoz.listenPort` |
| SigNoz OTLP gRPC | loopback `signoz.otlpGrpcPort` inside `sys-obs` |
| SigNoz OTLP HTTP | loopback `signoz.otlpHttpPort` inside `sys-obs` |

Only the SigNoz UI port is opened through the obs VM firewall by default.
ClickHouse, ClickHouse Keeper, OTLP, health, pprof, and zpages listeners
stay loopback or Unix-socket scoped.

## Secrets

Nixling generates SigNoz and ClickHouse credentials on the host under:

```text
/var/lib/nixling/observability/
```

The host directory is root-owned `0700`; files are root-owned `0444` so
guest-side systemd can read them through the read-only virtiofs secret
share at `/run/nixling-obs-secrets`. Secrets are consumed through
systemd credentials or environment files, not embedded as literals in the
Nix store.

## Default resources

`sys-obs` defaults are sized for a single-node SigNoz store:

| Resource | Default |
| --- | --- |
| vCPU | `4` |
| RAM | `8192` MiB |
| ClickHouse volume | `32768` MiB |
| ClickHouse Keeper volume | `2048` MiB |
| SigNoz volume | `4096` MiB |
| SigNoz collector volume | `2048` MiB |

## Migration notes

The old auto-declared VM name was `sys-obs-stack`; the new name is
`sys-obs`. If a host used the old default, upgrading creates a new VM
state directory. The old state is preserved for rollback and must be
retired intentionally.

Historical Prometheus/Loki/Tempo data and Grafana dashboard/alert state
do not automatically migrate to SigNoz.
