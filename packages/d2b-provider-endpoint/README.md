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

## Serving the relationship

`binding_descriptor` is the registration surface for the `EndpointBinding`
type, and the same crate serves it: the endpoint owner mints the row and this
crate's own driver delivers it. The driver decodes the committed
`EndpointBindingSpec` through the contract's own strict wire decoder, fences
the row's name against `binding_row_name`, and enforces the committed
`BindingSourceDecision` before it touches the host: the arbitration must be the
shared one this family commits, the admitted rights must cover the right the
row's own attachment kind requests, and the realized facets must cover the
ones that kind needs and stay inside `endpoint_binding_support()`. It then
resolves the parent `Endpoint` row and the consumer row through the manager,
behind the same-Zone keys and the owner fence that make the endpoint the
minting owner.

`EndpointSpec` is locator-free by contract, so the exact socket's host path is
not a committed fact anywhere in the graph - it is a property of the producing
launch. The driver therefore holds no path derivation of its own: it asks the
declared `EndpointLocatorSource` whether this daemon privately realized the
endpoint, and every delivery verb names the endpoint reference, the purpose its
committed `Endpoint` row publishes, and the consumer reference. Those three are
what the daemon resolves into a host path and a host principal, over
`EndpointAccessSource`, `EndpointGrantSource`, and `EndpointRevokeSource`.
`reconcile` reads what the kernel applies and grants the admitted right only
when it is not already effective; `recover` observes and never mutates;
`pre_drain` drops the consumer's entry on the exact socket inode, which is both
the fence against new use and the release because the kernel enforces that
entry at `connect(2)`; `delete` repeats the same idempotent revoke. A grant
that lands but is not EFFECTIVE, a containing directory the consumer may
enumerate, and an `attach` to an endpoint that is not accepting are all
reported undelivered rather than delivered.

The driver declares no broker operation, no child creation, no startup step,
and no hosted service: the delivery effects ride the effect port, so a
`ServiceDecl` with no host behind it would be a surface nothing can reach.

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

The `Endpoint` type publishes no durable status: the in-memory
`EndpointDriverStatus` (`Realizing`, `Realized`) is the only status
projection, matching the plane's in-memory status rule.

The `EndpointBinding` type publishes no wire status projection at all. There is
no committed `EndpointBinding` status contract in `d2b-contracts-resource`, and
minting one here would be a second contract; the in-memory
`EndpointBindingDriverStatus` (`Delivered` with the pinned `(dev, ino)`, or
`Undelivered` with a closed reason) is the whole projection.

Failures travel as registered failure kinds on the structured failure surface,
which is what the daemon logs and what tests assert: the endpoint type's are
`endpoint-spec-invalid`, `endpoint-shape-unsupported`,
`endpoint-socket-effect-failed`, and `endpoint-drain-pending`; the
relationship type's are `binding-spec-invalid`, `binding-owner-mismatch`,
`binding-parent-unavailable`, `binding-plan-derivation-invalid`, and
`binding-serving-effect-failed`.

## Build and test

```bash
cargo test -p d2b-provider-endpoint
```

The unit tests drive validate, recover, reconcile, finalize, and delete over
a scripted effect port; the `registration` suite proves the declaration
registers the type through the provider registry with its decoder and
factory, that a duplicate registration is refused, and that the declared mask
cannot arrive after the plane opens.
