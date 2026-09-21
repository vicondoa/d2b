# `d2b-provider-user`

This is the crate root for the `User` resource type. It owns the type's
driver, its spec decoder, and the driver declaration the v3 resource plane
registers the type by.

`User` is a named host identity: UID/session observation and the subject of
user-domain policy. The driver discovers it and nothing else: it realizes no
target-local state, owns no child row, and spawns nothing.

## Provider identity

| Field | Value |
| --- | --- |
| Provider name | `system-core` |
| ResourceType | `User` |
| Package | `packages/d2b-provider-user/` |
| Driver declaration | `user_descriptor` -> `DriverDescriptor` |

## Config schema

The type declares no provider config schema: `User` rows carry the closed
`UserSpec` base contract from `d2b-contracts-resource` (the declared OS
username, its groups, and the display text) and nothing outside it decodes at
validate.

## Exported resource types

`User` is not exportable. `ResourceExport` admits only qualified
`*.d2bus.org.*Service` types, so a user is never an export subject. The
declaration carries `exportable: false`.

## Controllers / services / workers / binaries

The crate ships one driver factory, not a standalone process. The driver
serves `User` rows through the `ResourceDriver` verbs: `validate` decodes the
stored spec, `recover` adopts what the contract already establishes,
`reconcile` discovers the declared identity once per desired generation and
publishes the typed in-memory status, `finalize` drains owned children, and
`delete` converges without effects.

`UserDriverFactory` is the registration surface; `user_descriptor` carries it
with the decoder, the type's verbs, execution domain, reads, and the
`BUILTIN | STARTUP` allowed-source mask.

## Placement and dependencies

`User` names no placement anchor: the row is reconciled on the machine whose
local identity it names, so the plane drives it in the Host domain. The
crate's own bounded NSS probe (`src/probe.rs`) reads the local account
database through `nix`'s `getpwnam`/`getgrnam` surface, behind the
`UserDriverEffects` seam, over the preserved `UserReconciler`. No daemon
adapter implements discovery for this family, and the daemon supplies no
externally built port: the composition root hands the family's effects the
declared facet set, which carries the crate's own probe.

The crate depends on `d2b-contracts-resource`, `d2b-provider-system-core`
(the User reconciler the family's effects drive), `d2b-provider-toolkit`,
`d2b-resource-runtime`, `d2b-resource-types`, `nix` (the bounded account
reads), `sha2` (the identity digest), `serde_json`, and `tracing`. It
depends on no daemon runtime: every probe input is host state the crate
reads itself, and daemon state crosses the boundary only as declared
facets.

## RBAC requirements

The driver holds no broker authority of its own: it runs inside the daemon's
per-zone plane, and every row it reads or writes is reached through the
manager with the plane's own caller identity. It serves no broker operations.

## Security posture

The driver never resolves a uid, gid, home directory, shell, or username
itself: the stored spec is decoded strictly, discovery happens in the
crate's own bounded probe behind the effects seam, and only the opaque
identity digest and the closed set of verified bindings cross back into the
status. A spec that fails to decode is a terminal refusal, and the family
owns no spawn surface at all.

## State and telemetry

The type publishes no durable status: the in-memory `UserDriverStatus` (the
generation observed, plus the typed `UserStatusReport`) is the only status
projection, matching the plane's in-memory status rule. Failures travel as
registered failure kinds (`system-core-spec-invalid`,
`system-core-user-discovery-failed`, `system-core-drain-pending`) on the
structured failure surface, which is what the daemon logs and what tests
assert.

## Build and test

```bash
cargo test -p d2b-provider-user
```

The unit tests drive validate, recover, reconcile, finalize, and delete over
the scripted driver-effects double, script the family's probe through the
declared facet set for the factory and registry paths, and prove a User row
reaches its driver through the registry alone; the `registration` suite
proves the declaration registers the type with its decoder and factory, that
a duplicate registration is refused, and that the declared mask cannot
arrive after the plane opens.
