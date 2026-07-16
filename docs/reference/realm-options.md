# Realm, workload, and provider options

**Diataxis category:** reference.

D2b 2.0 has one public declaration tree:
`d2b.realms.<realm>`. The old VM, env, gateway, relay, provider-placeholder,
and provider-kind option paths are not declared and have no option aliases or
tombstones.

Every host configuration must explicitly set:

```nix
d2b.acceptDestructiveV2Cutover = true;
```

The default is `false`, which fails evaluation. The value acknowledges that
the separate reset procedure destroys d2b 1.x state; it never initiates that
procedure.

## Names

Realm, workload, and provider ids use lowercase labels matching
`^[a-z][a-z0-9-]*$`, bounded to 128 bytes. Implementation ids use the same
shape, bounded to 64 bytes. Realm paths are most-specific-first dot-separated
label sequences, such as `dev.work`.

## Realm options

| Option | Type | Default |
| --- | --- | --- |
| `enable` | boolean | `true` |
| `id` | label | attribute name |
| `name` | string | attribute name |
| `parent` | realm path or `null` | `null` |
| `path` | realm path | `id` or `<id>.<parent>` |
| `placement` | placement enum | `host-local` |
| `placementProvider` | provider name or `null` | `null` |
| `providerSpecificPlacement` | label or `null` | `null` |
| `allowedUsers`, `allowedGroups` | list of strings | `[]` |
| `defaultWorkloadNamespace` | realm path | `path` |

Enabled realm ids and paths must be unique. A non-null parent must name an
enabled realm path. Provider-backed placements require `placementProvider`;
local placements reject it. `providerSpecificPlacement` is required only for
`provider-specific`.

`network.mode` is one of `none`, `declared`, or `external`. There is no
inherit-env mode. Realm network settings, policy references, identity
references, access metadata, and broker intent remain typed under their
corresponding realm namespaces.

## Provider options

`providers.<provider>` is strict: unknown fields are rejected.

| Option | Type | Default |
| --- | --- | --- |
| `enable` | boolean | `true` |
| `id` | label | attribute name |
| `type` | primary authority enum | required |
| `implementationId` | implementation id | required |
| `placement` | placement enum or `null` | `null` |
| `capabilities` | list of ids | `[]` |
| `configRef` | string or `null` | `null` |

The closed primary authority set is `runtime`, `infrastructure`, `transport`,
`substrate`, `credential`, `display`, `network`, `storage`, `device`, `audio`,
and `observability`. `type` and `implementationId` preserve the provider
registry's authority/implementation factory key. There is no free-form `kind`
or placeholder provider.

## Workload options

`workloads.<workload>` is strict and has these base fields:

| Option | Type | Default |
| --- | --- | --- |
| `enable` | boolean | `true` |
| `id` | label | attribute name |
| `name` | string | attribute name |
| `provider` | provider attribute name | required |
| `config` | deferred Nix module | `{}` |
| `autostart` | boolean | `false` |
| `shell.*` | persistent-shell settings | disabled |
| `launcher.*` | provider-neutral launcher metadata | disabled |

An enabled workload's provider must name an enabled `runtime` provider in the
same realm. Workload declarations have no runtime-kind discriminator,
`legacyVmName`, legacy VM state path, or provider-placeholder shape.

See [Configure realms](../how-to/configure-realms.md) for a complete example.
