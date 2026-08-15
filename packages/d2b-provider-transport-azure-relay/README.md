# `d2b-provider-transport-azure-relay`

Canonical implementation of `Provider/transport-azure-relay`.

## Provider identity

The implementation identifier is `azure-relay`. It carries opaque
ComponentSession byte streams and owns no ResourceType.

## Config schema

`RelayTransportConfig` requires a gateway Guest and Network. The signed
transport settings schema accepts only bare namespace and entity identifiers;
Credential refs are separate from settings.

## Exported resource types

No ResourceType is exported. ZoneLink desired state is interpreted by Core and
the Provider returns only an opaque carriage connection.

## Controllers / services / workers / binaries

`AzureRelayTransportProvider` opens bounded sender or listener connections
through `RelayCredentialPort` and `RelaySocketConnector`. Reconnect and
backpressure are explicit typed helpers.

## Placement and dependencies

Relay credentials and endpoint coordinates remain inside the gateway Guest.
The Host is an opaque intermediary and never terminates the enrolled KK
ComponentSession.

## RBAC requirements

Credentials are acquired for one role and one bounded deadline. Relay
authentication is carriage evidence only and never maps to local Admin.

## Security posture

Secrets, frames, endpoint coordinates, and lease diagnostics are redacted.
Bootstrap IKpsk2 continuation must be rejected until durable enrollment and a
distinct enrolled KK session are established.

## State and telemetry

Credit windows bound aggregate buffering. Reconnect delays are capped and
reset after a stable connection. Audit and metric labels are closed semantic
sets.

## Build and test

```text
cargo test -p d2b-provider-transport-azure-relay
```

Tests use in-process socket objects and do not contact Azure Relay.
