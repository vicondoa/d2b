# Zone-control Nix authoring

Companion to [Zone and Volume Nix authoring](./zone-volume-nix.md). That page
covers Volume authoring and Zone topology; this one covers the remaining
Zone-control resource classes and the exact refusals their compiler emits.

The authoring surface is the same:
`d2b.zones.<zone>.resources.<name>`. `nixos-modules/resources-zone-control.nix`
is the compiler seam for seven ResourceTypes:

`Zone`, `ZoneLink`, `Provider`, `Role`, `RoleBinding`, `Quota`, and
`EmergencyPolicy`.

**Field lists live in the generated schema, not here.** Per-type `spec` fields
come from `nixos-modules/generated/` and the committed JSON Schemas; this page
does not restate them, because a second copy of a generated field list is a
second thing to drift. What it does record is the eval-time behaviour an
operator meets: which authoring mistakes refuse, and what each refusal means.

## Every refusal is eval-time and fail-closed

These are `config.assertions`, so a violation fails the build before anything
is rendered, emitted, or applied. There is no warn-and-continue path and no
partial bundle. Every message is prefixed with the offending option path, so
the refusal names the exact attribute to edit.

## What refuses, by type

### Any Zone-control resource

| The refusal says | What it means |
| --- | --- |
| resource name must match `^[a-z][a-z0-9-]{0,62}$` | The attribute name under `resources` is the resource name and is bounded |
| `spec` must be an attribute set | A scalar or list was authored where a submodule belongs |
| `spec` contains a field outside the committed `<Type>` schema | The field is not in the generated schema for that type; check `nixos-modules/generated/` rather than guessing |
| `spec.<path>` must be a lowercase sha256 digest | Digest-shaped fields take `sha256:` plus 64 lowercase hex characters |

### `Zone`

| The refusal says | What it means |
| --- | --- |
| `spec` must be empty for the runtime-created Zone self-resource | The Zone self-resource is created by the runtime. Author Zone topology with the compiler-only `parentZone` scalar, never as a `Zone` resource body |

### `ZoneLink`

| The refusal says | What it means |
| --- | --- |
| `spec` must contain only the generated ZoneLink schema fields | The ZoneLink spec is frozen; an extra key is a refusal, not an extension point |
| `spec.childZoneName` must equal the enclosing Zone name | A ZoneLink is child-local; it cannot name another Zone |
| `spec.transportProviderRef` must be a same-Zone transport Provider ref | The ref must match `Provider/transport-*` and resolve inside this Zone |
| `spec.transportSettings` must be an attribute set | |
| `spec.transportCredentials` must contain at most 8 unique same-Zone Credential refs | Bounded, unique, and Zone-local |
| `spec.disabled` must be boolean | |
| `spec.limits` must contain only the generated ZoneLink limit fields | |
| `spec.limits.maxActiveStreams` must be between 1 and 128 | |
| `spec.limits.maxPendingIntents` must be between 0 and 1024 | |
| `spec.limits.reconnectMaxAttempts` must be positive | |
| `spec.limits.reconnectWindowSecs` must be positive | |

### `Provider`

| The refusal says | What it means |
| --- | --- |
| `spec` must contain only `artifactId` and `config` | |
| `spec.artifactId` must be a bounded plain artifact ID | Matches `^[a-z][a-z0-9-]{0,62}$`; it is an ID, not a store path |
| `spec.artifactId` must resolve to a provider artifact | The ID must name an entry of type `provider` in the artifact catalog |
| `spec.config` must be an attribute set | |
| `system-core` and `system-minijail` are bootstrap-only providers and cannot be hand-authored | These two are framework-declared. Declaring them by hand would shadow the bootstrap pair |

### `Role` and `RoleBinding`

| The refusal says | What it means |
| --- | --- |
| `spec` must contain only the generated Role fields | |
| `spec.rules` must contain at most 32 rules | |
| `spec.rules` must keep bounded non-empty ResourceTypes and permission lists | An empty ResourceType or permission list is a refusal, not a wildcard |
| `spec` must contain no expiry and only generated RoleBinding fields | |
| `spec` must not contain `expiry`, `expiresAt`, or `ttl` | RoleBindings have no expiry field. Revoke by removing the binding |
| `spec.roleRef` must resolve to a same-Zone Role | |
| `spec.subjects` must contain at most 128 unique same-Zone ResourceRefs | |
| `spec.externalPrincipalSelector` must be null or an attribute set | |
| `spec` must contain subjects unless an external principal selector is present | A binding that names nobody and selects nobody is refused rather than treated as empty |

The expiry refusal is worth reading twice, because the natural assumption is
the opposite. A time-limited grant would expire without anyone acting, which
makes revocation depend on a clock; d2b keeps revocation an explicit edit.

### `Quota`

| The refusal says | What it means |
| --- | --- |
| `spec` contains an unsupported Quota field | |
| `spec.ceilings` contains an unsupported field | |
| `spec.ceilings.maxResources` must be between 1 and 65536 | |
| `spec.ceilings.maxResourcesPerType` must be between 1 and 65536 | |
| `spec.ceilings.maxOwnerDepth` must be between 1 and 32 | |
| `spec.ceilings` optional limits must be null or positive integers | Zero is not "unlimited"; omit the field or set it null |
| `spec.perTypeCeilings` must contain at most 64 known ResourceType keys | An unknown ResourceType key refuses rather than being ignored |
| `spec.scope` must be `zone` | |
| `spec.enforcementPolicy` must be `hard` or `soft` | |

### `EmergencyPolicy`

| The refusal says | What it means |
| --- | --- |
| `spec` contains an unsupported EmergencyPolicy field | |
| `spec.enabled` must be boolean | |
| `spec.scope` must contain exactly the four boolean emergency controls | All four are required; a partial scope is refused so an omitted control never defaults to off |
| `spec.drainDeadlineSeconds` must be between 1 and 300 | |
| `spec.reason` must be a string of at most 256 bytes | The reason is bounded because it reaches audit |

## The sealed topology projection

The compiler publishes Zone parent topology to internal consumers as a
deliberately narrow projection: the parent map, its digest, and the per-Zone
bundle generation identity. It is marked sealed, and `parentZone` is never
written into the Zone self-resource or into a reciprocal row on the parent.

Operators do not author this projection and cannot widen it from
configuration. It is described here only so that a `parentZone` edit having no
visible effect on any emitted `Zone` resource reads as designed rather than as
a missing emission.

## See also

- [Zone and Volume Nix authoring](./zone-volume-nix.md) - Volume specs, layout
  and view anchoring, and Zone hierarchy authoring.
- [`zone-cli-contract.md`](./zone-cli-contract.md) - the operator-facing v3
  command surface these resources are administered through.
