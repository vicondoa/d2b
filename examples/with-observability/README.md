# `examples/with-observability` — workload VM plus native SigNoz

This example is a complete, copy-pasteable NixOS configuration that
turns on nixling observability end-to-end. The host enables
`nixling.observability.enable = true`, `work-app` opts in with
`nixling.vms.work-app.observability.enable = true`, and nixling
auto-declares `sys-obs` with native SigNoz, ClickHouse, ZooKeeper, schema
migrations, and the SigNoz OTel Collector.

No container runtime is used.

## Topology

```text
host: demo
├─ work env (declared)
│  └─ work-app (10.20.0.10, obs vsock CID 1110)
└─ obs env (auto-declared)
   └─ sys-obs (SigNoz http://10.40.0.10:8080, obs vsock CID 1000)

work-app guest OTel collector
  → /run/nixling/otel/otlp-egress.sock
  → workload CH-vsock relay on host port 14317
  → sys-obs source-specific ingress port 14318
  → signoz-otel-collector
  → ClickHouse

host OTel collector
  → /run/nixling/otel/host-egress.sock
  → broker-spawned OtelHostBridge
  → sys-obs source-specific ingress port 14317
  → signoz-otel-collector
  → ClickHouse
```

## Pointers

| Item | Value |
| --- | --- |
| SigNoz URL | `http://10.40.0.10:8080` |
| `work-app` observability vsock CID | `1110` |
| Observability VM vsock CID | `1000` |
| Host obs ingress vsock port | `14317` |
| `work-app` obs ingress vsock port | `14318` |

## How to apply

1. Copy `examples/with-observability/` into your own consumer repository.
2. Replace the bootloader, `fileSystems."/"`, and `machine-id` stubs in
   `configuration.nix` with your real host hardware configuration.
3. Replace the `alice` placeholder user with your own login.
4. Swap `nixling.url = "path:../.."` in `flake.nix` for a real ref, for
   example `github:vicondoa/nixling/v1.0.0`.
5. Build and switch:

   ```bash
   sudo nixos-rebuild switch --flake .#demo
   ```

## Expected behavior after switch

- The auto-declared `obs` env and `sys-obs` microVM are materialized.
- `sys-obs` runs ClickHouse, ZooKeeper, SigNoz, SigNoz OTel Collector,
  and source-specific `nixling-otel-vsock-in-*` relay services.
- `work-app` runs `nixling-otel-collector.service` and
  `nixling-otel-vsock-out.service`.
- SigNoz is reachable from the host at `http://10.40.0.10:8080`.
- The generated SigNoz root password is host-local under
  `${nixling.site.stateDir}/observability/signoz-root-password`.

## Validation

This example is exercised by:

- `tests/examples-with-observability-eval.sh`
- the per-example flake-check loop in `tests/static.sh`

To re-run the dedicated gate from the repo root:

```bash
bash tests/examples-with-observability-eval.sh
```

To re-run the in-place flake check directly:

```bash
cd examples/with-observability \
  && nix flake check --no-build --all-systems --no-write-lock-file
```
