# Enable observability

> How-to: add metrics, logs, and traces to an existing `nixling`
> deployment without changing the network layout or introducing
> cross-VM SSH credentials.
>
> Reading time: ~8 minutes.
> Difficulty: intermediate.

## Goal

Add full observability (metrics, logs, and traces) to an existing
`nixling` deployment without restructuring the network or adding
cross-VM SSH credentials. The framework keeps the transport on vsock
plus Unix sockets, so your existing env topology and IP routing stay as
they are.

## Prerequisites

- A working `nixling` deployment, consumed via
  `inputs.nixling.url = "github:vicondoa/nixling/v0.2.0"` or later.
- Sufficient host resources for one extra microVM (roughly 2 GiB RAM
  plus persistent storage sized for your chosen retention windows; the
  stock v0.2.0 module does not impose a fixed obs-VM disk cap).
- Cloud Hypervisor and the vsock kernel module, already part of the
  nixling baseline.

## Step 1: Enable the framework

```nix
{ config, ... }: {
  nixling.observability.enable = true;
}
```

Rebuild. Expected outcome: `nixling list` shows a new auto-declared
`sys-obs-stack` VM. It is headless and autostarts.

## Step 2: Opt VMs in

```nix
nixling.vms.<your-vm> = {
  env = "work";
  index = 10;
  observability.enable = true;
};
```

Rebuild. Expected outcome: inside the VM, `systemctl status alloy`
shows the guest agent; on the host,
`systemctl status nixling-otel-relay@<vm>.service` shows the per-VM
relay; and Cloud Hypervisor exposes the vsock path for the guest.

## Step 3: Verify the data path

### On the host

```bash
systemctl status nixling-otel-relay@<vm>.service     # host relay
systemctl status nixling-otel-host-bridge.service    # host's own egress
systemctl status nixling-ch-exporter.service         # CH metrics
curl http://127.0.0.1:9101/metrics                   # spot-check the exporter
```

### Inside a monitored VM

```bash
systemctl status alloy
systemctl status nixling-otel-vsock-out.service
ls -l /run/nixling/otlp.sock                         # exists, owned by alloy
```

### Inside `sys-obs-stack`

```bash
systemctl status alloy grafana prometheus loki tempo
systemctl status nixling-otel-vsock-in.service
```

## Step 4: Open Grafana

- `http://10.40.0.10:3000` (the default
  `cfg.grafana.listenAddress:cfg.grafana.listenPort`; Grafana binds to
  the obs VM's LAN IP and is reachable from the host via the obs
  uplink).
- Default datasources: Prometheus (`http://localhost:9090`), Loki
  (`http://localhost:3100`), Tempo (`http://localhost:3200`). All three
  stay on loopback inside the obs VM.
- The shipped dashboards live in the Grafana **Nixling** folder:
  Nixling Overview, VM Resources, Lifecycle Traces, Logs, Per-VM
  Store, and Obs VM Health. The Lifecycle Traces dashboard is
  preprovisioned but stays empty on the stock setup unless you point
  `otel-cli` at a reachable OTLP collector.

### Step 4b: Configure alert notifications (optional)

The framework provisions 8 default alert rules but **deliberately
does not configure notification channels**. By default, alerts
visible in Grafana / Prometheus go nowhere when they fire — there
is no email, Slack, Pushover, or webhook integration.

To enable notifications, operators configure either:

1. **Prometheus Alertmanager** as a sibling service in the obs VM
   pointing at the provisioned rules.
2. **Grafana contact points** + notification policies via the
   Grafana UI or declarative provisioning.

Both routes are operator-owned to allow the choice of notification
backend without framework lock-in.

## Step 5: Disable individual signals (optional)

```nix
nixling.vms.<vm>.observability.scrapeJournal     = false;  # no guest logs
nixling.vms.<vm>.observability.scrapeNodeMetrics = false;  # no guest metrics
nixling.observability.cli.traces.enable          = false;  # no CLI spans
nixling.observability.ch.exporter.enable         = false;  # no CH metrics
```

## Step 6: Tune retention

```nix
nixling.observability.retention.metrics = "7d";
nixling.observability.retention.logs    = "3d";
nixling.observability.retention.traces  = "1d";
```

Defaults are conservative (`30d` / `14d` / `7d`). On a small host,
shrink them to keep the obs VM's disk usage in check.

## Step 7: Swap `socat` for a compatible relay package (optional)

```nix
nixling.observability.transport.relayPackage = pkgs.your-relay;
```

Use this only if your package exposes a `bin/socat`-compatible CLI.
The current transport still passes `socat`-specific arguments on the
host, guest, and obs-VM relay paths. The default stays `pkgs.socat`.
When a future stable relay interface lands, the socat-compatible path
will remain supported for at least one minor release so custom
`relayPackage` users have a clean migration window.
host, guest, and obs-VM relay paths. The default stays `pkgs.socat`.
v0.3.0 will define a stable relay-binary interface.
host, guest, and obs-VM relay paths, so that compatibility requirement
remains in force for now. When `nixling-otel-relay` lands, nixling
will add a dedicated relay interface first and keep `bin/socat`
compatibility for at least one minor release with CHANGELOG migration
notes before removing it.

## Step 8: Supply Grafana's secret key from sops-nix or agenix

By default (as of v0.2.0), the framework generates a per-install
Grafana `secret_key` at activation **on the host** and shares it
into `sys-obs-stack` read-only at
`/run/nixling-obs-secrets/grafana-secret-key`. The host-side
source path is `${nixling.site.stateDir}/observability/grafana-secret-key`
(default `/var/lib/nixling/observability/grafana-secret-key`, mode
0400 root:root). To source the secret from a declarative secrets
framework instead:

```nix
nixling.observability.grafana.secretKeyFile = config.sops.secrets."grafana/secret-key".path;
```

When this option is set, the framework's host-side generator
leaves that secret alone; the file you supply must be readable by
the Grafana service inside `sys-obs-stack` (sops-nix and agenix
guest-VM modules handle this).

## Step 9: Supply Grafana's admin password from sops-nix or agenix

Grafana logs in as user `nixling-admin`. By default (as of v0.2.0),
the framework generates a per-install admin password at activation
**on the host** at `${nixling.site.stateDir}/observability/grafana-admin-password`
(default `/var/lib/nixling/observability/grafana-admin-password`,
mode 0400 root:root) and shares it read-only into `sys-obs-stack`
at `/run/nixling-obs-secrets/grafana-admin-password`. Operators on
the host can therefore read it directly without any cross-VM SSH:

```bash
sudo cat /var/lib/nixling/observability/grafana-admin-password
```

To source the password from a declarative secrets framework
instead:

```nix
nixling.observability.grafana.adminPasswordFile = config.sops.secrets."grafana/admin-password".path;
```

When this option is set, the framework's host-side generator
leaves that secret alone.

## Step 10: (Optional) Anonymous Viewer mode

For single-host LAN deployments where unauthenticated dashboard
access is acceptable:

```nix
nixling.observability.grafana.anonymousViewer.enable = true;
```

This enables Grafana's anonymous-Viewer role for unauthenticated
dashboard access while **keeping the login form available** so
operators can still sign in as `nixling-admin` for admin tasks
(contact points, plugin install, ad-hoc query inspection).
**Only enable on trusted LANs** — anyone reachable to
`http://10.40.0.10:3000` from the host's primary LAN can read
all VM telemetry without auth, including any logs/traces that
may contain sensitive content from monitored VMs
(`vm.observability.scrapeJournal` defaults to `true`).

## Step 11: (Optional) Disable individual alerts

The framework provisions 8 default Prometheus alert rules
(see [`docs/reference/components-observability.md`](../reference/components-observability.md#alerts)).
Disable any of them individually:

```nix
nixling.observability.alerts.NixlingGuestTelemetryMissing.enable = false;
```

Disabled alerts are omitted from the generated rule file
entirely, so Prometheus never evaluates them.

## Troubleshooting

- **No data in Grafana.** Check `nixling-otel-relay@<vm>.service` and
  `nixling-otel-vsock-in.service` first.
- **VM won't start.** Check vsock CID assertions; `nixos-rebuild
  switch` should fail at eval time if CIDs collide.
- **Obs VM disk full.** Tune retention (Step 6) or wipe the obs VM
  state with
  `nixling vm stop sys-obs-stack --apply && rm -rf /var/lib/nixling/vms/sys-obs-stack/state`.
- **CLI traces not appearing in Tempo.** The stock v0.2.0 host setup
  keeps Alloy's OTLP receiver on a Unix socket, which `otel-cli`
  cannot dial directly. The preprovisioned dashboard stays empty unless
  you additionally point `OTEL_EXPORTER_OTLP_ENDPOINT` at a reachable
  OTLP collector. First confirm `otel-cli` is in the CLI runtime
  closure:
  `nix-store -q --requisites $(which nixling) | grep otel-cli`.

## See also

- [`docs/reference/components-observability.md`](../reference/components-observability.md)
  — option / port / CID reference.
- [`docs/explanation/design.md`](../explanation/design.md) — why the
  shipped v0.2.0 design uses vsock instead of reverse-SSH.
- `examples/with-observability/` — turn-key consumer flake.
