# Enable observability

For constellation-wide inspection and `nixling op inspect`, see
[`../reference/constellation-observability.md`](../reference/constellation-observability.md).

> How-to: add OTel-native SigNoz telemetry to an existing `nixling`
> deployment without changing the network layout or introducing
> cross-VM SSH credentials.

## Goal

Enable the bundled native SigNoz observability stack. Nixling keeps the
transport on Unix sockets plus Cloud Hypervisor vsock; the workload env
LANs do not carry telemetry.

## Prerequisites

- A working nixling deployment.
- Enough capacity for the auto-declared `sys-obs` VM. Defaults are
  `vcpu = 4`, `mem = 8192` MiB, and about 40 GiB of persistent volumes:
  32 GiB ClickHouse, 2 GiB ClickHouse Keeper, 4 GiB SigNoz, and 2 GiB
  collector state.

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

## Optional: host journal and host OTLP ingest

The host edge collector always ships hostmetrics and the StoreSync audit
log. To bring it to parity with the guest collectors — host journal logs
and a host-local OTLP ingest endpoint — opt in (both default off):

```nix
{
  nixling.observability.host.scrapeJournal = true;       # host journal -> SigNoz
  nixling.observability.host.otlpIngest.enable = true;   # host apps push OTLP
  # nixling.observability.host.otlpIngest.clientGroup = "telemetry";
}
```

Host instrumentation then pushes OTLP to the Unix socket
`/run/nixling/otel/ingest/host-otlp.sock` (e.g.
`OTEL_EXPORTER_OTLP_ENDPOINT=unix:///run/nixling/otel/ingest/host-otlp.sock`).
There is no TCP listener; by default only root and the collector can write
the socket. To let a group emit, point `clientGroup` at an **existing**
group (declare it if needed):

```nix
{
  users.groups.telemetry = { };
  users.users.my-host-app.extraGroups = [ "telemetry" ];
  nixling.observability.host.otlpIngest.clientGroup = "telemetry";
}
```

> The host journal **and** host OTLP payloads can carry secrets (auth
> failures, sudo command lines, span attributes, log bodies). They are
> forwarded non-redacted over the host → `sys-obs` vsock bridge only, never
> a LAN. Enable them only when `sys-obs` is a trusted operator sink.
> Retention is governed by SigNoz/ClickHouse TTL inside `sys-obs`.

**Identity migration.** Enabling the framework stack already changes
host-origin telemetry identity: `vm.name` / `host.name` become the
hostname (`nixling.observability.host.identityName`, default
`networking.hostName`) instead of the literal `"host"`, even with the
receivers above left off. `vm.role` stays `"host"`. Set
`nixling.observability.host.identityName = "host"` if you depend on the
old labels in saved SigNoz queries.

## Step 3: Rebuild and restart affected VMs

On hosts where `nixling switch <vm> --apply` is unreliable, restart VMs with:

```bash
nixling vm stop <vm> --apply
nixling vm start <vm> --apply
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
systemctl status clickhouse-keeper.service
systemctl status signoz-schema-migrate-sync.service
systemctl status signoz.service
systemctl status signoz-otel-collector.service
systemctl status nixling-otel-vsock-in-host.service
systemctl status nixling-otel-vsock-in-<vm>.service
```

## Step 5: Open SigNoz

Default URL:

```text
http://10.40.0.10:8080
```

The address is derived from `nixling.observability.lanSubnet` and
`nixling.observability.index`. Only the SigNoz UI port is opened by
default; ClickHouse, ClickHouse Keeper, collector health, pprof, zpages,
and OTLP ports stay on loopback or Unix sockets inside `sys-obs`.

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

To source credentials from a declarative secrets system, pass host paths
as strings:

```nix
nixling.observability.signoz = {
  jwtSecretFile = "/run/secrets/nixling/signoz-jwt-secret";
  rootPasswordFile = "/run/secrets/nixling/signoz-root-password";
  clickhousePasswordFile = "/run/secrets/nixling/clickhouse-password";
};
```

The old `nixling.observability.grafana.*PasswordFile` options do not
affect native SigNoz authentication.

## Retention and disk budget

The `nixling.observability.retention.*` and `sampling.*` options are
compatibility shims from the retired Tempo/Loki stack. Changing them
emits a warning; they do not currently configure SigNoz or ClickHouse
TTL. Use SigNoz/ClickHouse retention controls and size the `sys-obs`
volumes for your workload before enabling high-volume telemetry.

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
