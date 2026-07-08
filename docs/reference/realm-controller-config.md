# Realm controller configuration

**Diataxis category:** reference.

`/etc/d2b/realm-controllers.json` is a private, non-secret bundle
artifact that records the host-local realm controller plan derived from
`d2b.realms.<realm>`. It remains metadata only as a runtime contract: the
artifact describes which host-local daemon, broker, socket, user, group,
state, and audit surfaces NixOS materializes, but it is not itself an
activation command or router.

The artifact exists so later runtime code can consume one typed contract
instead of re-deriving names, paths, allocator bindings, and access metadata
from the public option tree.

## Top-level contract

The JSON root is `RealmControllersJson`:

| Field | Meaning |
| --- | --- |
| `schemaVersion` | Bundle schema version. Current value is `v2`. |
| `runtimeState` | Current runtime posture. The only current value is `metadata-only`. |
| `controllers[]` | One enabled realm controller record per enabled `d2b.realms.<realm>`. |
| `invariants` | Booleans that make the non-runtime boundary explicit. |

The current invariants are:

| Invariant | Required meaning |
| --- | --- |
| `metadataOnly` | The file is descriptive metadata, not a runtime activation command. |
| `noSystemdUnitsMaterialized` | Legacy double-negative kept for the v2 schema: `true` means every controller row is metadata-only with no emitted systemd unit/socket; `false` means at least one host-local realm materializes daemon/broker units or sockets. |
| `preservesGlobalDaemonBehavior` | Existing `d2bd.service`, `d2b-priv-broker.socket`, and `d2b-priv-broker.service` behavior is unchanged. |
| `preservesDirectUnixSocketSemantics` | Future realm clients are expected to authenticate to the owning realm socket directly, not through a host byte proxy. |

## Deterministic names

Each controller row reserves deterministic host-local names. Host-local rows
also materialize the matching control-plane surfaces described below:

| Shape | Source |
| --- | --- |
| Realm daemon user/group/socket group | `d2br-<hash>`, where `<hash>` is the first 16 hex characters of the SHA-256 hash of the realm path. |
| Realm daemon unit name | `d2b-realm-<hash>-daemon.service` |
| Realm broker socket unit name | `d2b-realm-<hash>-priv-broker.socket` |
| Realm broker service unit name | `d2b-realm-<hash>-priv-broker.service` |
| Realm daemon config path | `/etc/d2b/realms/<realm-id>/daemon-config.json` |
| Realm run directory | `/run/d2b/realms/<realm-id>` by default. |
| Realm public socket | `<runDir>/public.sock` by default. |
| Realm broker socket | `<runDir>/broker.sock` by default. |
| Realm state directory | `config.d2b.site.stateDir + "/realms/<realm-id>"` by default. |
| Realm audit directory | `/var/lib/d2b/audit/realms/<realm-id>` by default. |

For host-local realms, NixOS creates the deterministic principals, tmpfiles
paths, daemon service, and (when host mutation is enabled) broker socket/service.
The generated row marks those emitted surfaces with
`daemon.materializedService`, `broker.materializedSocket`, and
`broker.materializedService`. Provider-backed and disabled realms do not emit
host-local units.

Socket paths are validated against Linux AF_UNIX pathname limits before they
can enter the bundle.

## Direct realm socket authorization

The `access` block records host users and groups intended to receive direct
access to a future realm public socket:

- `allowedUsers` and `allowedGroups` come from the realm declaration;
- `inheritedAdminUsers` comes from `d2b.site.adminUsers`.

Host-local realms add `allowedUsers` to the deterministic realm socket-access
group. The live global control plane still authorizes local lifecycle requests
through `SO_PEERCRED` plus the canonical `d2b` group on the global public
socket.

The direct-access contract is important for future runtime work: a client that
is authorized for a realm should connect to that realm's public AF_UNIX socket
and be checked there. The global host daemon is not a byte proxy that forwards
opaque traffic between local users and realm daemons.

## Local-root allocator binding

The `allocator` block binds each realm controller to the private allocator
metadata:

| Field | Meaning |
| --- | --- |
| `kind` | Current value `local-root-metadata`. |
| `configPath` | `/etc/d2b/allocator.json`. |
| `rootSocket` | The reserved local-root allocator socket path from `allocator.json`. |
| `resourceRequestRefs[]` | Opaque resource ids requested by this realm. |

This is a resolver contract, not a byte proxy. Future realm brokers must
request typed host-resource leases from the local-root allocator and receive
opaque grants. They must not pass raw host paths, nftables snippets, interface
names, or command bytes through the host daemon to get work done.

## Identity lifecycle boundary

Realm controller rows reserve the host-local state and audit locations where a
future controller can persist identity lifecycle state. The identity contract
itself lives in
[`d2b-realm-core`](./realm-identity-lifecycle.md) and the generated
[`d2b-realm-core` schema companion](./schemas/v2/d2b-realm-core.md): identity
references, fingerprints, controller generations, enrollment records, rotation
plans, revocation lists, teardown directives, recovery procedures, and redacted
identity audit metadata.

`realm-controllers.json` does not contain private keys, public key bytes,
provider credentials, relay credentials, session secrets, or signed credential
material. It also does not enforce revocation or session teardown. Future
runtime code must load identity metadata from controller state, fail closed on
stale or revoked generations, and write bounded audit records to the realm
audit surface described by this file.

## State, locks, and audit separation

Realm controller metadata keeps three storage classes distinct:

| Class | Default location | Boundary |
| --- | --- | --- |
| Runtime/locks | `/run/d2b/realms/<realm-id>` | Ephemeral runtime metadata such as daemon locks. |
| State | `config.d2b.site.stateDir + "/realms/<realm-id>"` | Future persistent realm-controller state. |
| Audit | `/var/lib/d2b/audit/realms/<realm-id>` | Future per-realm audit stream location; also recorded as the broker audit directory. |

Audit and state separation is deliberate. The default audit tree is root-owned
and disjoint from daemon-owned mutable realm state, so a compromised realm daemon
cannot replace the audit directory with attacker-controlled contents. Future
audit records should remain append-oriented, bounded, and redacted; they must not
be treated as repair authority for mutable realm state. Conversely, controller
state must not be used as an audit substitute or mixed into the global broker
audit stream.

## Current implementation boundary

The committed implementation materializes host-local control-plane scaffolding:
deterministic users/groups, daemon config files, tmpfiles directories and ACLs,
realm daemon services, and broker socket/service units for host-local realms
whose broker is enabled. The artifact is still metadata-only for access,
routing, and identity behavior: it describes the local surfaces and allocator
bindings but does not make the global host daemon a realm router.

It does not implement:

- host-resource allocation or mutation through the allocator binding;
- identity enrollment, relay connectivity, route advertisement, or provider
  controller runtime;
- migration of existing VMs from `d2b.envs` to a realm-owned network model;
- a realm-local lifecycle API that replaces the existing global public socket.

Existing `d2b.envs` and `d2b.vms.<vm>.env` behavior remains the implemented
runtime substrate until those later runtime surfaces land.

## Related references

- [Realm option schema](./realm-options.md)
- [Realm access resolver contract](./realm-access-resolver.md)
- [Realm tree routing contract](./realm-routing.md)
- [Realm identity lifecycle contract](./realm-identity-lifecycle.md)
- [Local-root allocator contract](./local-root-allocator.md)
- [Realm policy](./realm-policy.md)
- [Realm core model reference](./realm-core.md)
