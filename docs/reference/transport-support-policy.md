# Transport support policy

**Diataxis category:** reference.

Transports provide reachability for constellation peer sessions. They do
not authenticate local users, authorize operations, or multiplex named
streams; peer-session and stream-mux layers own those contracts.

| Transport | Status | Notes |
| --- | --- | --- |
| Azure Relay | Gateway transport | Uses gateway-owned credentials and Relay rendezvous sessions. |
| In-memory loopback | Test/conformance only | Hermetic byte transport; opens no sockets. |
| Local TCP | Test/conformance only | Loopback-only plaintext adapter that proves the transport trait is not Azure-specific. |
| QUIC | Planned | Must pass the same transport conformance matrix before support. |
| SSH | Explicit future transport/bootstrap only | There is no implicit fallback to SSH when a target transport is unavailable. |

Transport selection must be explicit. If the requested transport cannot
connect, listen, or authenticate at its own layer, callers receive a typed
transport error; d2b does not silently downgrade to another transport.

Tree route metadata does not create a transport by itself. The route contract
forbids treating discovery or direct shortcut metadata as a VPN, overlay, SSH
fallback, raw relay tunnel, raw TCP proxy, or file-descriptor tunnel; see
[realm tree routing](./realm-routing.md).
