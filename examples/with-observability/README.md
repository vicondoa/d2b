# `examples/with-observability` — one workload VM plus the observability stack

This example shows the consumer-side toggle pair for nixling observability: the host enables `nixling.observability.enable = true`, one workload VM opts in with `nixling.vms.work-app.observability.enable = true`, and nixling auto-declares the dedicated `sys-obs-stack` VM that fronts the Grafana/Prometheus/Loki/Tempo stack described in the observability reference. The example keeps the default `nixling.observability.vmName`, so `nix flake check` exercises the shipped v0.2.0 naming surface directly.

## Topology

```text
host: demo
├─ work env
│  ├─ work-app (10.20.0.10, obs vsock CID 210)
│  └─ host relay: nixling-otel-relay@work-app.service
├─ obs env (auto-declared by nixling.observability.enable)
│  └─ sys-obs-stack (Grafana http://10.40.0.10:3000, obs vsock CID 1000)
└─ host-local CH exporter: 127.0.0.1:9101

work-app guest Alloy/journald/node exporter
  → /run/nixling/otlp.sock
  → AF_VSOCK port 14317 via the host relay
  → observability-stack Alloy → Prometheus / Loki / Tempo / Grafana
```

## Pointers

| Item | Value | Notes |
|---|---|---|
| Grafana URL | `http://10.40.0.10:3000` | Default `nixling.observability.grafana.{listenAddress,listenPort}` |
| `work-app` observability vsock CID | `210` | `lib.attrNames config.nixling.envs = [ "obs" "work" ]`, so `work` is `envIndex = 1` and the manifest formula is `100 + 1*100 + 10` |
| Observability stack obs-vsock CID | `1000` | Fixed framework-reserved obs-stack CID from `_observability.obsVsockCid`, used by `sys-obs-stack` |
| Vsock service port | `14317` | Guest→host→obs relay port |
| CH exporter | `127.0.0.1:9101` | Default host-loopback exporter port |

`work-app` lands in the declared `work` env. Turning on host observability also auto-declares the separate `obs` env plus the dedicated `sys-obs-stack` VM, so the host still reaches Grafana over the obs env while telemetry itself stays on the vsock path.

## Disabling individual signals

- Journal scraping: `nixling.vms.work-app.observability.scrapeJournal = false;`
- Node metrics: `nixling.vms.work-app.observability.scrapeNodeMetrics = false;`
- CLI traces: `nixling.observability.cli.traces.enable = false;`
- CH exporter: `nixling.observability.ch.exporter.enable = false;`

## Swapping `socat` for a compatible relay

Set `nixling.observability.transport.relayPackage = pkgs.your-relay;` to replace the default `pkgs.socat` relay package across the observability transport. The package must expose a `bin/socat`-compatible CLI today; v0.3.0 will define a stable relay-binary interface.

## See also

- [`../../docs/reference/components-observability.md`](../../docs/reference/components-observability.md)
- [`../../docs/how-to/enable-observability.md`](../../docs/how-to/enable-observability.md)
