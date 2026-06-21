# Host egress policy

**Diataxis category:** reference.

Nixling hosts are a credential-free local control surface for realm-backed
workloads. Realm relay/provider credentials and relay sessions belong in
gateway guests.

The host publishes a static, redacted policy artifact at
`/etc/nixling/host-realm-relay-egress-policy.json`.
It lists the gateway interface classifications that may carry realm relay
traffic and records bounded diagnostic policy:

- diagnostics are redacted;
- diagnostics are rate-limited;
- payloads, headers, tokens, endpoints, and credentials are omitted.

The daemon does not mutate firewall or routing state at runtime. Any host
egress policy is declarative NixOS configuration plus broker-mediated host
preparation, and must fail before use if it cannot be rendered or applied.

## Runtime checks

Host runtime checks must verify that `nixlingd`, the broker, and host CLI
processes do not expose realm relay credentials in `/proc/<pid>/environ`,
`/proc/<pid>/cmdline`, or inherited file descriptors. Socket inspection
must confirm the host does not open realm Relay/WebSocket sessions.
