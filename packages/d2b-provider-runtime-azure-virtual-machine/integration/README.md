# Azure VM Provider integration

The normal suite is hermetic and uses injected effect and credential ports:

```text
cargo test -p d2b-provider-runtime-azure-virtual-machine
```

No test requires Azure credentials, ARM access, a host socket, or a running
daemon. Live ARM and VM enrollment checks are manual-only and must be gated by
the deployment harness.
