# Realm policy

**Diataxis category:** reference.

Realm policy is fail-closed and local by default. The local root retains its
local lifecycle boundary, while child realms are owned by their controller and
provider operations are admitted through typed in-process or provider-agent
implementations.

Current Nix support for `d2b.realms.<realm>` records the declaration,
emits private host-local controller metadata, and materializes host-local
control-plane scaffolding: deterministic daemon/broker units and sockets,
realm users and groups, tmpfiles-managed state/runtime/audit paths, config
metadata, and local socket access ACLs. Realm access-layer routing, policy
evaluation, identity/enrollment, and realm-owned network partitions remain
future work. Existing `d2b.envs` and `d2b.vms.<vm>.env` declarations remain
the implemented workload substrate.

## Policy ownership

| Placement | Authority | Credential boundary | Cross-realm default |
| --- | --- | --- | --- |
| `host-local` | The owning realm controller and its broker boundary. | No provider credential bytes in host Nix metadata. | Deny. |
| `provider-agent` | An authenticated provider agent bound to the exact realm, workload, and role. | Credentials remain co-located and cross provider boundaries only as opaque leases. | Deny. |

The host is not a global realm-policy singleton. The removed gateway option
surface is not a policy mode and does not provision an authority.

## Isolation rules

- Work, personal, and provider realms must not share an L2 bridge when they
  require separate network trust boundaries.
- A provider-agent placement is not itself a network isolation boundary.
  Operators must rely on declared realm/env topology and L3 isolation controls.
- Deployments must validate that host L3 forwarding cannot transit between
  realm bridges. Use explicit firewall/nftables drops or equivalent
  namespace/routing isolation before treating realms as isolated at L3. This
  reference page describes the policy contract; it does not claim code-level L3
  enforcement for hosts that have not installed those drops or isolation
  controls.
- Cross-realm operations and streams are denied unless a future reviewed typed
  policy explicitly allows a named operation or stream. There are no enabled
  default allow rules.
- SSH fallback, VPN/overlay links, raw relay pipes, raw TCP proxies, file
  descriptor tunnels, and generic tunnels are not policy escape hatches.

## Authorization and audit

Local host authorization remains `SO_PEERCRED` plus the canonical `d2b`
lifecycle group. Relay, provider-agent, and cross-realm identities never map
to local lifecycle roles.

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
- [Realm tree routing contract](./realm-routing.md)
- [Realm core model reference](./realm-core.md)
