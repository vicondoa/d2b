# Azure Relay Provider integration fixtures

The source integration fixtures exercise the Provider through its typed socket
and credential ports. They must not contain local Admin mapping, host relay
credentials, or unbounded stream buffers.

Hermetic tests use an injected credential port and a real in-process socket
object. No Azure account, credential, network, or daemon is required:

```text
cargo test -p d2b-provider-transport-azure-relay
```

Live Relay validation is manual-only and must be explicitly gated by the
deployment harness.
