# Enable observability

> How-to: add OTel-native SigNoz telemetry to an existing `nixling`
> deployment without changing the network layout or introducing
> cross-VM SSH credentials.

## Goal

Enable the bundled native SigNoz observability stack. Nixling keeps the
transport on Unix sockets plus Cloud Hypervisor vsock; the workload env
LANs do not carry telemetry.

## Prerequisites

- A working nixling deployment.
- Enough capacity for the auto-declared `sys-obs` VM. The SigNoz stack
  includes ClickHouse and is heavier than the retired Grafana stack; use
  the default `sys-obs` resources unless you have sized your own
  ClickHouse memory and disk budget.

## Step 1: Enable the framework-level stack

```nix
{ ... }: {
  nixling.observability.enable = true;
}
```

Rebuild. Expected result: nixling auto-declares:

- `nixling.envs.obs`
- `nixling.vms.sys-obs`

`sys-obs` runs native NixOS services for ClickHouse, SigNoz, the SigNoz
OTel Collector, and schema migrations. No container runtime is used.

## Step 2: Opt workload VMs in

```nix
nixling.vms.work-app = {
  env = "work";
  index = 10;
  observability.enable = true;
};
```

Each opted-in VM runs a guest OTel collector and a relay that forwards
OTLP over vsock. The host runs a nixling-owned OTel collector for host
metrics and a broker-spawned host bridge into `sys-obs`.

## Step 3: Rebuild and restart affected VMs

On hosts where `nixling switch <vm>` is unreliable, restart VMs with:

```bash
nixling down <vm> --apply
nixling up <vm> --apply
```

When changing the nixling checkout or bundle contract, restart the daemon
before runtime validation:

```bash
sudo systemctl restart nixlingd.service
```

## Step 4: Verify the data path

Host:

```bash
systemctl status nixling-host-otel-collector.service
nixling host doctor --read-only
```

Workload VM:

```bash
systemctl status nixling-otel-collector.service
systemctl status nixling-otel-vsock-out.service
```

Observability VM:

```bash
systemctl status clickhouse.service
systemctl status zookeeper.service
systemctl status signoz-schema-migrate-sync.service
systemctl status signoz.service
systemctl status signoz-otel-collector.service
systemctl status nixling-otel-vsock-in.service
```

## Step 5: Open SigNoz

Default URL:

```text
http://10.40.0.10:8080
```

The address is derived from `nixling.observability.lanSubnet` and
`nixling.observability.index`. Only the SigNoz UI port is opened by
default; ClickHouse, ZooKeeper, collector health, pprof, zpages, and OTLP
ports stay on loopback or Unix sockets inside `sys-obs`.

## First-run admin

The bundled stack provisions a root SigNoz admin from host-generated
credentials. The default email is:

```text
admin@nixling.local
```

The root password is generated on the host at:

```text
/var/lib/nixling/observability/signoz-root-password
```

Read it with `sudo` on the host. Do not copy it into a world-readable
Nix store file.

## Alert notifications

Nixling may seed default SigNoz alerts, but notification channels remain
operator-owned. Configure email, Slack, webhook, PagerDuty, or other
SigNoz channels in the SigNoz UI or with a site-local declarative
provisioning layer.

## Migration from the old stack

Older nixling versions used `sys-obs-stack` with
Grafana/Prometheus/Loki/Tempo/Alloy.

The new default VM name is `sys-obs`. Historical Prometheus, Loki, Tempo,
Grafana dashboard state, and Grafana alert state are not migrated into
SigNoz automatically.

Recommended low-risk rollout:

1. Preserve `/var/lib/nixling/vms/sys-obs-stack`.
2. Bring up `sys-obs`.
3. Verify host and workload telemetry appears in SigNoz.
4. Only then retire or wipe old `sys-obs-stack` state.

Rollback is clean only while the old `sys-obs-stack` state remains.
