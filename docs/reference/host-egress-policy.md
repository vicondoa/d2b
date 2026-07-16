# Host egress policy

**Diataxis category:** reference.

D2b hosts are a credential-free local control surface for provider-backed
workloads. Realm Relay and provider credentials belong in the exact
credential-owning provider agent.

The host publishes a static, redacted policy artifact at
`/etc/d2b/host-realm-relay-egress-policy.json`.
The compatibility artifact records bounded diagnostic policy:

- diagnostics are redacted;
- diagnostics are rate-limited;
- payloads, headers, tokens, endpoints, and credentials are omitted.

The daemon does not mutate firewall or routing state at runtime. Any host
egress policy is declarative NixOS configuration plus broker-mediated host
preparation, and must fail before use if it cannot be rendered or applied.

## Runtime checks

Host runtime checks must verify that `d2bd`, the broker, and host CLI
processes do not expose realm relay credentials in `/proc/<pid>/environ`,
`/proc/<pid>/cmdline`, or inherited file descriptors. Socket inspection
must confirm the host does not open provider-owned Relay/WebSocket sessions.
