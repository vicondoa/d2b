# Realm option schema

**Diataxis category:** reference.

`d2b.realms.<realm>` is the public Nix option namespace for the
realm-native control-plane model. In the current release it is a schema
foundation only: defining a realm records intent and validates option
shape, but it does not spawn per-realm daemons or brokers, bind realm
sockets, allocate network resources, create users or groups, migrate VMs,
or change the active `d2b.envs` / `d2b.vms.<vm>.env` runtime model.

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
| `enable` | boolean | `true` | Includes the realm declaration in future realm-native planning. It has no runtime side effect today. |
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

`allowedUsers` lists host user names intended to receive direct access to
the realm's future local public socket. The option does not create those
users, groups, socket ACLs, or systemd units today. Local lifecycle
authorization for the current implementation remains the existing
`SO_PEERCRED` plus `d2b` group check on `/run/d2b/public.sock`.

The derived `paths.*` fields reserve future path shapes:

| Option | Default |
| --- | --- |
| `paths.stateDir` | `config.d2b.site.stateDir + "/realms/<realm>"` |
| `paths.auditDir` | `paths.stateDir + "/audit"` |
| `paths.runDir` | `/run/d2b/realms/<realm>` |
| `paths.publicSocket` | `paths.runDir + "/public.sock"` |
| `paths.brokerSocket` | `paths.runDir + "/broker.sock"` |

These paths are not created or bound by the schema-only declaration. Socket
paths are still checked against Linux AF_UNIX pathname sockets and must fit in
107 bytes, leaving the final `sockaddr_un.sun_path` byte for the terminating
NUL.

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
placeholders for future realm-local privileged broker work. Setting them in
the current schema records intent only; it does not start a broker, bind
`paths.brokerSocket`, or grant host mutation authority.

## Related references

- [ADR 0043 — Realm-native control plane](../adr/0043-realm-native-control-plane.md)
- [Realm core model reference](./realm-core.md)
- [Realm policy](./realm-policy.md)
- [Naming conventions](./naming-conventions.md)
