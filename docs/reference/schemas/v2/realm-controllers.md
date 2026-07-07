# `realm-controllers.json` schema (`v2`)

Schema: [`realm-controllers.json`](./realm-controllers.json)

`realm-controllers.json` is private realm controller configuration. It records
deterministic per-realm daemon, broker, socket, state, audit, allocator,
provider-placement, and access metadata rooted in `d2b.realms`; host-local
realms use the same rows to materialise users, groups, systemd services, and
sockets.

## Top-level fields

- `schemaVersion` — schema version for this artifact.
- `runtimeState` — closed runtime state enum. The value in this scope remains
  `metadata-only`.
- `controllers` — enabled realm controller rows, sorted deterministically by
  normalized realm path.
- `invariants` — booleans asserting the artifact preserves the existing global
  daemon/broker behavior and keeps direct Unix socket semantics.

## Controller fields

- `realmName`, `realmId`, and `realmPath` — identifiers copied from the
  normalized realm index.
- `placement` and `providerPlacement` — closed controller placement metadata
  plus the optional provider binding for provider-backed placements.
- `daemon` — deterministic daemon user/group, public socket group,
  service/config/lock names, and materialization flags.
- `broker` — deterministic broker socket/service names, broker socket
  path, audit directory, host-mutation intent, and materialization flags.
- `paths` and `sockets` — state/run/audit directories plus public and broker
  socket paths.
- `allocator` — metadata-only binding to `/etc/d2b/allocator.json`, the
  local-root allocator socket path, and the realm's allocator resource request
  references.
- `access` — declared direct-access users/groups plus inherited host admin
  users for socket ACL planning.
- `providers` — provider declarations copied from the realm index.

## Contract notes

- `materializedService` and `materializedSocket` are true only for emitted
  host-local systemd units. Gateway and provider-backed realms remain metadata
  rows until their controller placement is implemented.
- The daemon/broker socket paths remain ordinary AF_UNIX pathnames. Future
  runtime work must preserve direct `SO_PEERCRED` and `SCM_RIGHTS` semantics
  rather than proxying authority through an unrelated transport.
- Principal names are deterministic metadata derived from the realm path; the
  NixOS host module creates them for host-local realms.
