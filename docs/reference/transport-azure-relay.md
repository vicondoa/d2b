# Azure Relay transport

**Diataxis category:** reference.

`d2b-provider-relay` exposes Azure Relay Hybrid Connections as a
constellation byte transport. Relay provides reachability only: peer-session
handshake, authentication binding, frame caps, capability negotiation, and
named-stream authorization stay in the constellation router/mux layers.

## Credential roles

| Operation | Relay role | Credential source |
| --- | --- | --- |
| Provider-agent listener | `listen` | Distinct opaque credential lease resolved by the co-located relay control port. |
| Provider-agent connector | `connect` | Distinct opaque credential lease resolved by the co-located relay control port. |

Credential material remains in the credential-owning provider agent. Canonical
provider requests carry only lease and binding identifiers; they never carry
Relay keys or bearer bytes.

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

The production control port owns WebSocket and TLS behavior. The canonical
provider contract exposes no endpoint URL, CA bundle, credential, or free-form
transport payload.

The host production registry does not currently compose Azure Relay. It is
constructed only by the generic provider-agent composer with an agent-local
control port and co-located credential provider; realm Relay option fields are
metadata, not a deployment recipe.
