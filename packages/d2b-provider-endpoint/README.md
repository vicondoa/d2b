# `d2b-provider-endpoint`

This is the crate root for the `Endpoint` resource type. It owns the type's
driver, its spec decoder, and the driver declaration the v3 resource plane
registers the type by.

The Endpoint family covers the endpoint shapes the plane realizes today:

- the binding-owned virtiofsd socket (transport unix, purpose `virtiofsd`),
  realized through the family's own effects implementation as a long
  effect;
- the guest-runtime control endpoints the Cloud Hypervisor provider's fixed
  child roles declare (`ch-api` on the guest's VMM Process, `guest-control` on
  the Guest), realized on the subject row's committed evidence;
- the Device TPM Provider's worker sockets (`swtpm-tpm-socket`,
  `swtpm-control-socket`), realized on the producer worker Process row's
  `Ready` status.

The family's driver effects are implemented by this crate itself
(`effects_service`): the purpose derivations classify one purpose onto the
realization the plane owns from the declaring providers' own vocabularies,
and the daemon-owned realization surfaces - the host socket effect for the
binding-owned virtiofsd socket and the two row-evidence probes - cross the
provider boundary as the declared `EndpointEffectFacets` the composition
root supplies. The daemon hosts the family's declared effects service
(`endpoint.d2bus.org/effects`) per zone from the family's registered
factory; no externally built port appears at any construction site.

## Provider identity

| Field | Value |
| --- | --- |
| Provider name | `endpoint` |
| ResourceType | `Endpoint` |
| Package | `packages/d2b-provider-endpoint/` |
| Driver declaration | `endpoint_descriptor` -> `DriverDescriptor` |

## Config schema

The type declares no provider config schema: `Endpoint` rows carry the closed
`EndpointSpec` contract from `d2b-contracts-resource`
(`providerRef`, `producerRef`, class, transport, purpose, locality,
visibility, attachment policy, consumer policy, lifecycle policy) and nothing
outside it decodes at validate.

## Exported resource types

`Endpoint` is not exportable. `ResourceExport` admits only qualified
`*.d2bus.org.*Service` types, so an endpoint is never an export subject. The
declaration carries `exportable: false`.

## Controllers / services / workers / binaries

The crate ships one driver factory, not a standalone process. The driver
serves `Endpoint` rows through the `ResourceDriver` verbs: `validate` decodes
the stored spec and admits only the closed realization set, `recover` probes
the endpoint's live evidence, `reconcile` spawns the realization as a long
effect, `finalize` drains owned children, and `delete` removes the socket
realization.

`EndpointDriverFactory` is the registration surface; `endpoint_descriptor`
carries it with the decoder, the type's verbs, execution domains, reads, and
the `BUILTIN | STARTUP` allowed-source mask.

## Placement and dependencies

`Endpoint` names no placement anchor, so an endpoint row is reconciled on its
containing Zone's Host. A realized producer may live in a Guest (a
guest-owned worker Process, or the Guest itself for `guest-control`); the
driver reaches that row through the declared facets and the manager, never
through its own placement.

The crate depends on `d2b-contracts-resource`, `d2b-resource-runtime`,
`d2b-resource-types`, and `d2b-provider-toolkit`. The purpose derivations
read the declaring providers' own vocabularies, so the crate also depends on
`d2b-provider-guest-cloud-hypervisor` (the child roles that declare the
guest-runtime control purposes) and `d2b-provider-device-tpm` (the declared
worker-socket purposes); the closed admission set cannot drift from the
children those providers commit.

## RBAC requirements

The driver holds no broker authority of its own: it runs inside the daemon's
per-zone plane, and every row it reads or writes is reached through the
manager with the plane's own caller identity. Classification of a stored spec
is refused closed (`endpoint-shape-unsupported`) for anything outside the
committed realization set.

## Security posture

The driver never invents a socket path, producer identity, or address from
spec text: the stored spec is decoded strictly, the admitted shapes are the
ones the declaring providers commit, and the socket path is resolved by the
daemon's own registry through the declared socket facet. Effects are
idempotent under retry, and a spec that fails to decode is a terminal refusal
rather than a best-effort teardown.

## State and telemetry

The type publishes no durable status: the in-memory `EndpointDriverStatus`
(`Realizing`, `Realized`) is the only status projection, matching the plane's
in-memory status rule. Failures travel as registered failure kinds
(`endpoint-spec-invalid`, `endpoint-shape-unsupported`,
`endpoint-socket-effect-failed`, `endpoint-drain-pending`) on the structured
failure surface, which is what the daemon logs and what tests assert.

## Build and test

```bash
cargo test -p d2b-provider-endpoint
```

The unit tests drive validate, recover, reconcile, finalize, and delete over
a scripted effect port; the `registration` suite proves the declaration
registers the type through the provider registry with its decoder and
factory, that a duplicate registration is refused, and that the declared mask
cannot arrive after the plane opens.
