# `examples/with-observability` — one workload VM plus the observability stack

This example is a complete, copy-pasteable NixOS configuration that
turns on the nixling observability subsystem end-to-end. The host
enables `nixling.observability.enable = true`, one workload VM
opts in with `nixling.vms.work-app.observability.enable = true`,
and nixling auto-declares the dedicated `sys-obs-stack` VM that
fronts the Grafana/Prometheus/Loki/Tempo/Alloy stack described in
the observability reference. The example pins
`nixling.observability.vmName = "sys-obs-stack"` (the framework
default) so `nix flake check` exercises the shipped v1.0.0 naming
surface directly.

## Files

| File | Role |
|---|---|
| `flake.nix` | Thin flake wrapper — pins `nixling.url = "path:../.."` and wires `nixling.nixosModules.default` + `./configuration.nix` into `nixosConfigurations.demo`. |
| `configuration.nix` | Operator-facing NixOS config: host stubs, `nixling.site`, `nixling.observability`, `nixling.envs.work`, and the `work-app` workload VM with observability enabled. |

## Topology

```text
host: demo
├─ work env (declared)
│  ├─ work-app (10.20.0.10, obs vsock CID 1110)
│  └─ host relay: broker-spawned VsockRelay runner (RunnerRole::VsockRelay)
├─ obs env (auto-declared by nixling.observability.enable)
│  └─ sys-obs-stack (Grafana http://10.40.0.10:3000, obs vsock CID 1000)
└─ host-local CH exporter: 127.0.0.1:9101

work-app guest Alloy/journald/node exporter
  → /run/nixling/otlp.sock
  → AF_VSOCK port 14317 via the host relay
  → sys-obs-stack Alloy → Prometheus / Loki / Tempo / Grafana
```

## Pointers

| Item | Value | Notes |
|---|---|---|
| Grafana URL | `http://10.40.0.10:3000` | Default `nixling.observability.grafana.{listenAddress,listenPort}`. |
| `work-app` observability vsock CID | `1110` | `lib.attrNames config.nixling.envs = [ "obs" "work" ]`, so `work` is `envIndex = 1` and the manifest formula is `100 + 1*1000 + 10`. |
| Observability stack obs-vsock CID | `1000` | Fixed framework-reserved obs-stack CID from `_observability.obsVsockCid`, used by `sys-obs-stack`. |
| Vsock service port | `14317` | Guest→host→obs relay port. |
| CH exporter | `127.0.0.1:9101` | Default host-loopback exporter port. |

## How to apply

1. Copy `examples/with-observability/` into your own consumer
   repository (or evaluate it in place against the in-tree
   framework — see "Validation" below).
2. Replace the bootloader / `fileSystems."/"` / `machine-id`
   stubs in `configuration.nix` with the real values from your
   `hardware-configuration.nix`.
3. Replace the `alice` placeholder user with your own login.
4. Swap `nixling.url = "path:../.."` in `flake.nix` for a
   real ref, e.g. `github:vicondoa/nixling/v1.0.0`.
5. Build and switch:

   ```bash
   sudo nixos-rebuild switch --flake .#demo
   ```

## Expected behaviour after `switch`

* The auto-declared `obs` env's bridges (`br-obs-up`,
  `br-obs-lan`) and the `sys-obs-stack` microVM come up via the
  daemon-spawned broker runners (per ADR 0015).
* The host-side OTLP relay (`broker-spawned VsockRelay runner`)
  starts when `work-app` boots and forwards guest telemetry to
  `sys-obs-stack` over AF_VSOCK port 14317.
* Grafana becomes reachable from the host at
  `http://10.40.0.10:3000`. Initial credentials (`nixling-admin`
  + per-install random password) live on the host at
  `${nixling.site.stateDir}/observability/grafana-admin-password`
  (mode `0400 root:root`); read them with `sudo cat`.
* The "Nixling" Grafana folder contains the 6 default dashboards
  and the 8 default Prometheus alert rules (see
  `docs/reference/components-observability.md`).
* `work-app`'s guest Alloy agent scrapes journald + node-exporter
  metrics and ships them through the relay path above.

## Validation

This example is exercised by:

* `tests/examples-with-observability-eval.sh` — asserts the
  example evaluates cleanly via its own `flake.nix`, that the
  expected observability + VM toggles are set in
  `configuration.nix`, and that the auto-declared
  `sys-obs-stack` VM and `obs` env materialise.
* The per-example flake-check loop in `tests/static.sh` — runs
  `nix flake check --no-build --all-systems --no-write-lock-file`
  in every `examples/*/` directory.

To re-run the dedicated gate from the repo root:

```bash
bash tests/examples-with-observability-eval.sh
```

To re-run the in-place flake check directly:

```bash
cd examples/with-observability \
  && nix flake check --no-build --all-systems --no-write-lock-file
```

## Disabling individual signals

* Journal scraping: `nixling.vms.work-app.observability.scrapeJournal = false;`
* Node metrics: `nixling.vms.work-app.observability.scrapeNodeMetrics = false;`
* CLI traces: `nixling.observability.cli.traces.enable = false;`
* CH exporter: `nixling.observability.ch.exporter.enable = false;`

## Swapping `socat` for a compatible relay

Set `nixling.observability.transport.relayPackage = pkgs.your-relay;`
to replace the default `pkgs.socat` relay package across the
observability transport. The package must expose a `bin/socat`-
compatible CLI for the current transport.

## See also

* [`../../docs/reference/components-observability.md`](../../docs/reference/components-observability.md)
* [`../../docs/how-to/enable-observability.md`](../../docs/how-to/enable-observability.md)
