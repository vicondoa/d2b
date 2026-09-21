# `d2b-provider-credential`

This is the crate root for the `Credential` resource type. It owns the
type's driver, its spec decoder, its driver declaration, and the revocation
and session vocabulary the type's teardown binds.

The Credential family covers the three Credential Providers the plane
realizes:

- `credential-secret-service` (user-domain credentials on a Host or Guest
  execution target);
- `credential-entra` (Guest-executing, no user domain);
- `credential-managed-identity` (Host- or Guest-executing, no user domain;
  the one Provider with a co-located agent Process child).

## Provider identity

| Field | Value |
| --- | --- |
| Provider name | `credential` |
| ResourceType | `Credential` |
| Package | `packages/d2b-provider-credential/` |
| Driver declaration | `credential_descriptor` -> `DriverDescriptor` |

The three realizing Providers keep their own identities, crates, and
dossiers (`d2b-provider-credential-secret-service`, `-entra`,
`-managed-identity`). This crate consumes their exported `PROVIDER_REF` and
`PROVIDER_KIND` constants, so the admission set cannot drift from the
Providers that declare them.

## Config schema

The type declares no provider config schema: `Credential` rows carry the
closed `CredentialSpec` contract from `d2b-contracts-provider`
(`providerRef`, controller identity, scope with `executionRef` and
`domainFilter`, allowed operations, audience, and secret-shape policy) and
nothing outside it decodes at validate. Per-Provider scope admission is
closed: secret-service requires the user domain and a user reference,
entra requires a Guest execution target, and managed-identity admits Host
and Guest targets without the user domain.

## Exported resource types

`Credential` is not exportable. `ResourceExport` admits only qualified
`*.d2bus.org.*Service` types, so a credential is never an export subject.
The declaration carries `exportable: false`.

## Controllers / services / workers / binaries

The crate ships one driver factory, not a standalone process. The driver
serves `Credential` rows through the `ResourceDriver` verbs: `validate`
decodes the stored spec and applies the per-Provider scope checks, `recover`
adopts an already-serving managed-identity agent, `reconcile` reports
Provider readiness and - for managed-identity - mints the declared agent
Process child, `finalize` drains owned children, and `delete` revokes the
lease before marking any owned child deleting.

`CredentialDriverFactory` is the registration surface; `credential_descriptor`
carries it with the decoder, the type's verbs, execution domains, reads, the
`BUILTIN | STARTUP` allowed-source mask, and the one declared `ChildCreation`
(the agent Process under the minijail Provider's exported reference).

## Placement and dependencies

A Credential names a Host or a Guest execution target in its own scope; the
driver reaches the Provider row, the target row, and its owned Process
children through the family's own effects implementation and the manager,
never through its own placement.

The crate depends on `d2b-contracts-provider`, `d2b-contracts-resource`,
`d2b-resource-runtime`, `d2b-resource-types`, and the three Credential
realizer crates' exported vocabulary, plus the minijail Process Provider's
exported reference. The daemon's runtime is deliberately not a dependency:
the Provider facts, lease facts, agent probe, and session arrive through the
declared facets this crate defines, the daemon supplies their
implementations through the composition root, and the family serves its own
effects over them (U8), hosted per zone as the declared
`credential.d2bus.org/effects` service.

## RBAC requirements

The driver holds no broker authority of its own: it runs inside the daemon's
per-zone plane, and every row it reads or writes is reached through the
manager with the plane's own caller identity. Revocation runs through the
authenticated Provider session the daemon hands in, which binds the exact
zone, Provider generation, controller generation, and session generation; a
missing or stale binding is refused (`credential-resource-invalid`) or
reported uncertain, never guessed.

## Security posture

The driver never invents a provider identity, session generation, or
credential identity from spec text: the stored spec is decoded strictly, only
the three declared Providers are admitted, and the revocation request is
built from the durable row plus the live session facts. `CredentialRevocationRequest`
and `CredentialRevocationEvidence` redact their identity fields in `Debug`.
Cleanup fails closed: an unconfirmed revocation leaves the row and its owned
children in place (R28), and the agent Process is minted through the manager
(commit-before-spawn) rather than spawned by the driver.

## State and telemetry

The type publishes no durable status: the in-memory `CredentialDriverStatus`
(`ProviderUnavailable`, `AgentPending`, `AgentUnavailable`, `AgentDraining`,
`Ready`, `LeaseRevoked`, `RevocationUncertain`) is the only status
projection, matching the plane's in-memory status rule. Confirmed
revocations are observable through the journal record the driver emits
(credential, operation id, outcome, session generation). Failures travel as
registered failure kinds (`credential-spec-invalid`,
`credential-provider-unsupported`, `credential-provider-unavailable`,
`credential-agent-unavailable`, `credential-child-mutation-failed`,
`credential-revocation-identity-rejected`,
`credential-revocation-unconfirmed`) on the structured failure surface.

## Build and test

```bash
cargo test -p d2b-provider-credential
```

The unit tests drive validate, recover, reconcile, finalize, and delete over
a scripted effect port and a recording manager, observing revocation
ordering, child minting, and the fail-closed session binding; the
`registration` suite proves the declaration registers the type through the
provider registry with its decoder, factory, and declared creation, that a
duplicate registration is refused, and that the declared mask cannot arrive
after the plane opens.
