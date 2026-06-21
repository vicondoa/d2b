# Azure Relay transport

**Diataxis category:** reference.

`nixling-provider-relay` exposes Azure Relay Hybrid Connections as a
constellation byte transport. Relay provides reachability only: peer-session
handshake, authentication binding, frame caps, capability negotiation, and
named-stream authorization stay in the constellation router/mux layers.

## Credential roles

| Side | Relay role | Credential source |
| --- | --- | --- |
| Gateway listener | `listen` | Gateway-owned sealed credential envelope, unsealed inside the gateway guest. |
| Sandbox sender | `connect` | Managed-identity Entra bearer or a gateway-minted short-lived Relay Send SAS bearer. |

Long-lived Relay rule keys are never passed to provider-managed sandboxes.
`RelayCredential` and `RelayConnect` redact bearer material in `Debug`.

## Transport provider

`AzureRelayTransportProvider` implements the shared
`TransportProvider` trait:

- `connect()` dials Azure Relay as `connect` and wraps the WebSocket in a
  bounded in-memory `TransportSession`;
- `listen()` registers the `listen` control channel, accepts Relay
  rendezvous addresses, and queues accepted `TransportSession`s for callers;
- connection/auth failures map to typed provider errors without exposing
  relay tokens;
- adapters above it must still pass the shared
  [transport conformance matrix](./transport-conformance-matrix.md).

The provider accepts an optional extra CA bundle for sandbox egress-proxy
environments. Gateway-side listeners normally use the platform web PKI roots.
