# Realm option schema

**Diataxis category:** reference.

`d2b.realms.<realm>` is the public Nix option namespace for the
realm-native control-plane model. In the current release it records intent,
validates option shape, emits private host-local controller metadata in
`/etc/d2b/realm-controllers.json`, and materializes host-local scaffolding:
deterministic daemon/broker units and sockets, realm users and groups,
tmpfiles-managed state/runtime/audit paths, config metadata, and local
socket access ACLs. It does not allocate realm-owned network resources,
migrate VMs, start provider adapters, or enable realm routing/identity/access
policy beyond that inert host-local scaffolding.

Existing `d2b.envs` declarations remain the implemented network substrate.
Workload VMs still join that substrate with `d2b.vms.<vm>.env = "<env>"`.
Realm declarations may point at existing envs as transition metadata so
future runtime support can map realm policy to the current bridge, net-VM,
NAT, DHCP, and firewall substrate without changing today's behavior.

## Minimal declaration

```nix
d2b.realms.work = {
  placement = "host-local";
  allowedUsers = [ "alice" ];
  env = "work";
  network.envs = [ "work" ];
};

d2b.envs.work = {
  lanSubnet = "10.44.0.0/24";
  uplinkSubnet = "192.0.2.0/30";
};

d2b.vms.laptop.env = "work";
```

The realm above does not move `laptop` into a new runtime namespace. The
active VM placement remains `d2b.vms.laptop.env = "work"` until future
runtime support consumes the realm declaration.

## Identifier and path fields

| Option | Type / values | Default | Meaning |
| --- | --- | --- | --- |
| `enable` | boolean | `true` | Includes the realm declaration in realm metadata and, for enabled host-local realms, host-local scaffolding materialization. Disabled realms are omitted. |
| `id` | lowercase label matching `^[a-z][a-z0-9-]*$` | attribute name | Stable realm id used for derived paths. |
| `name` | string | attribute name | Human-readable realm name. |
| `parent` | realm path or `null` | `null` | Optional parent realm path. Parent/child validation and routing are future runtime work. |
| `path` | realm path | `id`, or derived from `parent` | Canonical realm path, written most-specific first for targets such as `builder.dev.d2b`. |
| `defaultWorkloadNamespace` | realm path | `id` | Namespace future target resolution will use for unqualified workload declarations. Current VM names are unchanged. |

Realm path labels use the same lowercase label shape as other
realm-core identifiers. See [Naming conventions](./naming-conventions.md)
and [Realm core model reference](./realm-core.md) for target grammar and
identifier families.

## Placement

`placement` selects the intended controller placement:

| Value | Intended placement |
| --- | --- |
| `host-local` | Realm controller as an isolated host-local service. |
| `gateway-vm` | Realm controller inside a dedicated local gateway VM. |
| `cloud-full-host` | Realm controller on a cloud VM running full d2b. |
| `provider-controller` | Provider-supported controller environment named by `placementProvider`. |
| `provider-agent` | Agent inside or adjacent to a managed provider sandbox named by `placementProvider`. |
| `provider-specific` | Adapter-defined placement named by `placementProvider` plus `providerSpecificPlacement`. |

`placementProvider` is required for `provider-controller`,
`provider-agent`, and `provider-specific`, and must be null for
`host-local`, `gateway-vm`, and `cloud-full-host`. `providerSpecificPlacement`
is meaningful only with `placement = "provider-specific"`. Both values are
inert metadata in the current schema.

## Local access metadata

`allowedUsers` lists host user names that receive membership in the
realm's deterministic socket-access group for the local public socket.
`allowedGroups` does the same for existing host groups. Both are emitted into
`realm-controllers.json` as access metadata, along with inherited
`d2b.site.adminUsers`, and host-local materialization creates the required
realm users/groups, socket ACLs, and systemd units. Local lifecycle
authorization for existing VM operations remains the `SO_PEERCRED` plus `d2b`
group check on `/run/d2b/public.sock`; the realm access layer is scaffolded
but not yet a routing or identity authority.

Future realm access is direct socket access: an authorized local user connects
to the owning realm's public AF_UNIX socket and is checked there. The global
host daemon is not a byte proxy for realm public sockets.

The derived `paths.*` fields define the materialized host-local path shapes:

| Option | Default |
| --- | --- |
| `paths.stateDir` | `config.d2b.site.stateDir + "/realms/<realm>"` |
| `paths.auditDir` | `/var/lib/d2b/audit/realms/<realm>` |
| `paths.runDir` | `/run/d2b/realms/<realm>` |
| `paths.publicSocket` | `paths.runDir + "/public.sock"` |
| `paths.brokerSocket` | `paths.runDir + "/broker.sock"` |

These paths are emitted into the private realm controller artifact. Host-local
realm controllers materialize the state, audit, and runtime directories through
tmpfiles. The default audit path is not nested under daemon-owned realm state,
preserving the root-owned append-only audit boundary. Runtime directories are
root-owned and not group-writable; the daemon gets an explicit ACL for socket
creation while authorized local users get traversal to known socket paths. Socket
paths are still checked against Linux AF_UNIX pathname sockets and must fit in
107 bytes, leaving the final `sockaddr_un.sun_path` byte for the terminating NUL.

The realm controller metadata also defines deterministic unit names:

| Reserved name | Shape |
| --- | --- |
| Realm daemon | `d2b-realm-<hash>-daemon.service` |
| Realm broker socket | `d2b-realm-<hash>-priv-broker.socket` |
| Realm broker service | `d2b-realm-<hash>-priv-broker.service` |

Host-local realm controllers emit the daemon service and, when host mutation is
enabled, the broker socket/service. Provider-backed realms keep these names as
reservations and carry `materializedService = false` /
`materializedSocket = false` in the private artifact.

## Env and network substrate bridge

| Option | Type / values | Default | Current effect |
| --- | --- | --- | --- |
| `env` | existing env name or `null` | `null` | Records the primary existing `d2b.envs.<env>` association for the realm. |
| `network.envs` | list of env names | `[]` | Records additional existing envs associated with the realm. |
| `network.mode` | `none`, `inherit-env`, `declared`, `external` | `none` | Placeholder for the future realm network model. |
| `network.cidrRefs` | list of strings | `[]` | Opaque references to future realm-owned address allocation records. |

The safe default is `network.mode = "none"`: declaring a realm claims no
network resources. Even when `env` or `network.envs` is set, `network.nix`
continues to materialize bridges, net VMs, NAT/DHCP, and workload taps from
`d2b.envs` and `d2b.vms.<vm>.env`.

## Provider declarations

`providers.<provider>` entries describe provider metadata owned by the
realm:

| Option | Type / values | Default | Meaning |
| --- | --- | --- | --- |
| `enable` | boolean | `true` | Whether this provider declaration is active for future planning. |
| `id` | string | provider attribute name | Stable provider identifier within the realm. |
| `kind` | string or `null` | `null` | Provider family or adapter name, such as `aca`. |
| `placement` | placement enum or `null` | `null` | Optional provider placement override; `null` inherits the realm placement. |
| `capabilityRefs` | list of strings | `[]` | Opaque references to capability bundles or advertisements. |
| `configRef` | string or `null` | `null` | Opaque reference to non-secret provider configuration. |

Provider declarations do not start provider adapters or daemons yet. Keep
credentials out of `configRef`; use external enrollment and key references
instead.

## Relay, discovery, policy, and keys

Realm declarations use opaque references for material that must not be
stored directly in Nix:

| Namespace | Options | Notes |
| --- | --- | --- |
| `relay` | `enable`, `mode`, `endpoints`, `credentialRef` | `mode` is `disabled`, `static`, or `discovery`. The default opens no listeners and connects no relays. `credentialRef` must point outside the host Nix store. |
| `discovery` | `enable`, `domain`, `configRef` | Non-secret discovery metadata. |
| `policy` | `bundleRef`, `bundlePath`, `defaultDeny` | Policy starts from `defaultDeny = true`. `bundlePath` must be an absolute path when set. |
| `keys` | `controllerKeyRef`, `trustBundleRef`, `enrollmentRef`, `rotationPolicyRef` | Opaque references to controller, trust, enrollment, and rotation material held outside Nix expressions. |

Relay identity is transport metadata, not local authorization. Future
cross-realm operations still require realm identity, capability checks,
policy, idempotency, and bounded audit.

## Broker placeholder

`broker.enable` and `broker.hostMutation` are deliberately default-off
controls for realm-local privileged broker scaffolding. Enabling the broker
for a host-local realm materializes the deterministic broker socket/service
metadata and units, but it does not grant host mutation authority or route
free-form host operations through the realm.

The private controller artifact records each realm's local-root allocator
binding to `/etc/d2b/allocator.json`. That binding is a typed resolver
contract for future host-resource leases; it is not permission for a realm
broker to send raw commands, raw host paths, or free-form network rules through
the host daemon.

## Related references

- [ADR 0043 — Realm-native control plane](../adr/0043-realm-native-control-plane.md)
- [Realm core model reference](./realm-core.md)
- [Realm controller configuration](./realm-controller-config.md)
- [Realm policy](./realm-policy.md)
- [Naming conventions](./naming-conventions.md)
