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

`EndpointBinding` is the second exported type and is not exportable either: a
relationship delivers one exact endpoint to one consumer, so exporting it would
export the delivery rather than the endpoint.

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

## Binding rows

The endpoint owner, not a consumer, mints the `EndpointBinding` rows its
committed `Endpoint` row implies. `canonical_binding_rows` takes the Zone, the
endpoint's own `EndpointSpec`, the endpoint reference, and the deliveries that
row declares, and commits exactly one canonical `EndpointBindingSpec` row per
delivery; the request the row was derived from travels beside the row bytes,
so the admission and the row a boundary reads back are one derivation rather
than two descriptions that can drift. Each row carries the source's own
`BindingSourceDecision`: the right the attachment kind requests, shared
arbitration, and the realized facets the kind's own `required_facets()` names
- a connect or a listen commits the endpoint descriptor alone, an attach
commits the descriptor and the private presentation. A source row that
declares no delivery derives no row, and a delivery the endpoint's own policy
does not admit is refused rather than committed. `binding_row_name` mints the
deterministic row name from the KTD3 slot address rather than a declaration
position, so one relationship keeps one identity across restarts.

The endpoint's own consumer policy is the source of the deliveries: a
subject the policy names is a consumer this row publishes the endpoint to, and
the endpoint's own operation allowlist and attachment capacity decide how that
consumer reaches it. `declared_endpoint_bindings` reads both off the committed
row, so nothing a consumer supplied reaches any of it.

`EndpointBindingDriverFactory` serves the committed rows. `validate` and
`reconcile` re-run every check the derivation performs - the row's derived
name, the committed decision, the owning `Endpoint` row behind its owner fence,
that row's own policy at its CURRENT generation, and the named consumer - and
then build the typed wire request in `endpoint_access_request`: the socket is
the row's own committed slot, the permission is the socket's own read/write
triple for the admitted right, and the authority binding is the wire's own
digest over the endpoint, the consumer, the Zone identity, the socket name, and
the VERB.

The delivery rides the declared `EndpointAccessDispatch` facet, which the
daemon implements over its authenticated broker socket. The pinned
`(device, inode)` and the effective rights that come back are the broker's own
answers, read from the kernel through the descriptor it held while it applied
or read the grant, and the driver reconciles against them rather than
recomputing either locally: a relationship short of the admitted right, an
ancestor that no longer applies a traverse bit, or a parent the consumer may
enumerate is refused rather than reported delivered, and an inode that changed
under a standing grant is reported as `EndpointReplaced` rather than as the
access that used to be there. A standing grant is observed before it is
re-applied, so a replaced inode and a nullified ACL mask are visible at all.

The delivery slot is derived from the endpoint's own committed identity rather
than from a caller or a label: it is the token the broker joins onto its own
endpoint directory, so one endpoint is one socket name and two endpoints in one
Zone can never collide on one socket.

## Placement and dependencies

`Endpoint` names no placement anchor, so an endpoint row is reconciled on its
containing Zone's Host. A realized producer may live in a Guest (a
guest-owned worker Process, or the Guest itself for `guest-control`); the
driver reaches that row through the declared facets and the manager, never
through its own placement.

The crate depends on `d2b-contracts-resource`, `d2b-contracts-broker`,
`d2b-resource-runtime`, `d2b-resource-types`, and `d2b-provider-toolkit`. The
broker wire dependency is the typed exact-endpoint request and answer only: the
crate builds the request and reconciles the reply, and no privileged dispatch,
socket, or path lives in it. The purpose derivations
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
