# Realm workload with native SigNoz

This example enables the realm-owned `sys-obs.local-root.d2b` workload and a
`work-app.work.local-root.d2b` workload that imports the guest OpenTelemetry
component.

```text
local-root
├─ sys-obs
│  └─ SigNoz + ClickHouse + OpenTelemetry Collector
└─ work realm
   └─ work-app -> workload vsock relay -> sys-obs
```

Host and workload telemetry use Unix sockets and vsock, not a workload LAN.
Generated credentials and runtime endpoints live under canonical short-ID
realm/workload paths. The old `d2b.envs.obs` and `d2b.vms.sys-obs`
declarations are absent.

Validate the example through the repository's focused nix-unit cases:

```bash
make test-flake
```
