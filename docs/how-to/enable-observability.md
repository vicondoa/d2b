# Enable observability

## Goal

Run the bundled native SigNoz stack as a realm-owned workload and forward
host and workload telemetry to it without exposing OTLP on a realm LAN.

## Enable the stack

```nix
{
  d2b.acceptDestructiveV2Cutover = true;
  d2b.observability.enable = true;
}
```

This adds `sys-obs.local-root.d2b` to the local-root workload graph. Its state,
runtime sockets, credentials, and host bridge use canonical short-ID resource
paths below `/var/lib/d2b/r/<realm-id>/w/<workload-id>` and
`/run/d2b/r/<realm-id>/w/<workload-id>`. The old host-wide `d2b.vms.sys-obs`
composition no longer exists.

## Enable collection in a workload

Import the guest component in the workload module:

```nix
d2b.realms.work = {
  path = "work.local-root";
  providers.runtime-local = {
    type = "runtime";
    implementationId = "cloud-hypervisor";
  };
  workloads.work-app = {
    provider = "runtime-local";
    autostart = true;
    config.imports = [
      (inputs.d2b + "/nixos-modules/components/observability/guest.nix")
    ];
  };
};
```

The guest collector sends OTLP through its workload vsock relay. The stack
collector assigns authoritative realm/workload identity at ingress.

## Optional host inputs

Host metrics and the bounded StoreSync projection are enabled with the stack.
Host journal and arbitrary host OTLP remain default-off:

```nix
{
  d2b.observability.host.scrapeJournal = true;
  d2b.observability.host.otlpIngest.enable = true;
  # d2b.observability.host.otlpIngest.clientGroup = "telemetry";
}
```

Journal and application OTLP payloads can contain secrets. Enable these inputs
only when the observability workload is a trusted operator sink.

## Verify

```bash
systemctl status d2b-host-otel-collector.service
d2b workload inspect sys-obs.local-root.d2b
d2b workload inspect work-app.work.local-root.d2b
```

Inside the observability workload, verify `clickhouse`,
`clickhouse-keeper`, `signoz`, and `signoz-otel-collector`.

The local observability provider returns only bounded, positive-allowlist
projections. It does not read raw repair state, own audit records, or create a
second provider registration path.
