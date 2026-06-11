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
- ZooKeeper coordinates the single-node ClickHouse cluster used by
  SigNoz's replicated schema.
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
    -> sys-obs per-source vsock ingress
    -> signoz-otel-collector.service
    -> ClickHouse

host
  nixling-host-otel-collector.service
    -> /run/nixling/otel/host-egress.sock
    -> broker-spawned OtelHostBridge
    -> sys-obs per-source vsock ingress
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
| `nixling.observability.transport.relayPackage` | package | `pkgs.socat` | Socat-compatible relay package for vsock bridges. |

Legacy Grafana/Tempo/Loki/Prometheus-specific options are retired or
kept only as migration shims. Do not use them for new configurations.

## Per-VM options

| Option | Type | Default | Meaning |
| --- | --- | --- | --- |
| `nixling.vms.<vm>.observability.enable` | bool | `false` | Enable telemetry collection for this VM. |
| `nixling.vms.<vm>.observability.scrapeJournal` | bool | `true` | Compatibility toggle for guest log collection. |
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
- `zookeeper.service`
- `signoz-schema-migrate-sync.service`
- `signoz-schema-migrate-async.service`
- `signoz.service`
- `signoz-otel-collector.service`
- `nixling-otel-vsock-in.service`

## Socket and port contract

| Resource | Value |
| --- | --- |
| Obs VM vsock CID | `1000` |
| Workload observability CID | `100 + envIndex * 100 + vm.index` |
| Host collector egress | `/run/nixling/otel/host-egress.sock` |
| Guest local OTLP | `/run/nixling/otel/otlp.sock` with compatibility symlink `/run/nixling/otlp.sock` |
| Guest relay handoff | `/run/nixling/otel/otlp-egress.sock` |
| SigNoz UI | `signoz.listenAddress:signoz.listenPort` |
| SigNoz OTLP gRPC | loopback `signoz.otlpGrpcPort` inside `sys-obs` |
| SigNoz OTLP HTTP | loopback `signoz.otlpHttpPort` inside `sys-obs` |

Only the SigNoz UI port is opened through the obs VM firewall by default.
ClickHouse, ZooKeeper, OTLP, health, pprof, and zpages listeners stay
loopback or Unix-socket scoped.

## Secrets

Nixling generates SigNoz and ClickHouse credentials on the host under:

```text
/var/lib/nixling/observability/
```

Files are root-owned `0400` and shared read-only into `sys-obs` at
`/run/nixling-obs-secrets`. Secrets are consumed through systemd
credentials or environment files, not embedded as literals in the Nix
store.

## Migration notes

The old auto-declared VM name was `sys-obs-stack`; the new name is
`sys-obs`. If a host used the old default, upgrading creates a new VM
state directory. The old state is preserved for rollback and must be
retired intentionally.

Historical Prometheus/Loki/Tempo data and Grafana dashboard/alert state
do not automatically migrate to SigNoz.
