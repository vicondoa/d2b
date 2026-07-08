# Realm policy

**Diataxis category:** reference.

Realm policy is fail-closed and local by default. The host keeps the local
fast path for bare VM names and the reserved `local` realm. Gateway-backed
realms are fronted by a dedicated gateway guest in a separate d2b env/L2
segment.

Current Nix support for `d2b.realms.<realm>` records the declaration,
emits private host-local controller metadata, and materializes host-local
control-plane scaffolding: deterministic daemon/broker units and sockets,
realm users and groups, tmpfiles-managed state/runtime/audit paths, config
metadata, and local socket access ACLs. Realm access-layer routing, policy
evaluation, identity/enrollment, and realm-owned network partitions remain
future work. Existing `d2b.envs` and `d2b.vms.<vm>.env` declarations remain
the implemented workload substrate.

## Policy modes

| Mode | Authority | Credential boundary | Cross-realm default |
| --- | --- | --- | --- |
| `host-resident` | Local host daemon for local workloads only. | No realm relay/provider credentials. | Deny. |
| `gateway-backed` | The owning gateway guest for that realm. | Credentials are enrolled direct-to-guest or by opaque passthrough; the host never parses or stores them. | Deny. |

The host is not a global realm-policy singleton. It publishes local gateway
entrypoints and routes operators to the right gateway VM, but remote/provider
policy storage and evaluation live in the owning gateway guest.

## Isolation rules

- `local` is always host-resident and cannot be declared as gateway-backed.
- Work, personal, and provider realms never share a gateway guest or L2 bridge.
- Default routes inside a gateway guest are not an isolation boundary by
  themselves; operators must rely on the dedicated gateway/env topology and L3
  isolation controls below.
- Deployments must validate that host L3 forwarding cannot transit between
  realm bridges. Use explicit firewall/nftables drops or equivalent
  namespace/routing isolation before treating realms as isolated at L3. This
  reference page describes the policy contract; it does not claim code-level L3
  enforcement for hosts that have not installed those drops or isolation
  controls.
- Cross-realm operations and streams are denied unless a future reviewed typed
  policy explicitly allows a named operation or stream. There are no enabled
  default allow rules.
- SSH fallback and generic tunnels are not policy escape hatches.

## Authorization and audit

Local host authorization remains `SO_PEERCRED` plus the canonical `d2b`
lifecycle group. Relay, gateway, and cross-realm identities never map to local
lifecycle roles.

Future host-local realm access uses direct realm socket authorization. A local
user or group listed for a realm is intended to connect to that realm's public
AF_UNIX socket directly and be authorized there. The global host daemon must not
act as a byte proxy that forwards opaque traffic between local users and realm
controllers.

Default-deny decisions are operator-visible through typed errors and bounded
audit events. Audit and error surfaces carry only low-cardinality realm,
operation or stream kind, decision, and reason labels. They must not contain
payload bytes, argv, stdout/stderr, credentials, tokens, provider headers, full
endpoints, host paths, or PII.

Identity lifecycle metadata for enrollment, controller generations, rotation,
revocation, teardown directives, and recovery is documented in
[Realm identity lifecycle contract](./realm-identity-lifecycle.md). Those DTOs
are designed for future policy/session enforcement, but the current runtime
does not yet enforce live revocation or route sessions through per-realm
controllers.

## Related references

- [Realm option schema](./realm-options.md)
- [Realm controller configuration](./realm-controller-config.md)
- [Realm identity lifecycle contract](./realm-identity-lifecycle.md)
- [Realm core model reference](./realm-core.md)
