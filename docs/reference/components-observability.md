# Observability component

The bundled observability stack is the realm workload
`sys-obs.local-root.d2b`. It runs native SigNoz, ClickHouse, ClickHouse Keeper,
and the SigNoz OpenTelemetry Collector without a container runtime.

## Resource model

`nixos-modules/realm-observability-rows.nix` emits canonical short-ID rows for:

- the observability workload and its host bridge role;
- host-ingest, host-egress, and stack-vsock endpoints;
- configuration, state, secret, runtime, and StoreSync projection paths; and
- authoritative ingress-source projections.

All host paths are scoped below:

```text
/etc/d2b/r/<realm-id>/w/<workload-id>
/var/lib/d2b/r/<realm-id>/w/<workload-id>
/run/d2b/r/<realm-id>/w/<workload-id>
```

The realm broker is the declared creator and repair owner. There is no
`/var/lib/d2b/vms/sys-obs` resource tree and no host-singleton VM declaration.

## Provider binding

The component consumes the existing `local-observability` registry entry for
the local-root realm. It emits no provider-registry fragment or alternate
registration path.

The frozen binding limits are:

| Limit | Value |
| --- | ---: |
| records | 64 |
| bytes | 32768 |
| time window | 86400000 ms |

Results use a positive allowlist. Raw audit, raw repair state, credentials,
argv, environment, command output, secrets, and host paths are excluded.

## Data path

```text
workload collector
  -> workload Unix socket
  -> workload vsock relay
  -> sys-obs source receiver
  -> signoz-otel-collector
  -> ClickHouse

host collector
  -> canonical host-egress Unix socket
  -> realm-owned host bridge role
  -> sys-obs host receiver
```

OTLP does not traverse workload LANs. The optional host ingest endpoint is an
AF_UNIX socket and remains disabled by default.

## Credentials

SigNoz JWT, SigNoz root-password, and ClickHouse password sources are
realm-broker-owned workload resource rows. Generated values are created
outside the Nix store and mounted read-only inside the observability workload.

## Guest opt-in

Import `nixos-modules/components/observability/guest.nix` in a workload's
deferred `config` module. The guest collector may scrape node metrics and the
guest journal; both are controlled by its `d2b.observability` guest options.
