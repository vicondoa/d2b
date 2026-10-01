# `d2b-provider-endpoint-binding`

This is the crate root for the `EndpointBinding` resource type. It owns the
row's driver, its spec decoder, the read-side helpers over a stored binding
row, and the driver declaration the v3 resource plane registers the type by.

## Provider identity

| Field | Value |
| --- | --- |
| Provider name | `endpoint-binding` (the Provider that serves a binding row) |
| ResourceType | `EndpointBinding` |
| Source ResourceType | `Endpoint` |
| Package | `packages/d2b-provider-endpoint-binding/` |
| Driver declaration | `binding_descriptor` -> `DriverDescriptor` |

## Config schema

The type declares no provider config schema. An `EndpointBinding` row carries
the closed `EndpointBindingSpec` contract from `d2b-contracts-resource`
(`endpointRef`, `executionRef`, `attachment`, `slot`) as its base spec, and
nothing outside it decodes at validate. The row contract's own `new()`
refuses a source that is not an `Endpoint` and a consumer the binding kind
does not admit, which for this kind is any consumer except a `Host`; a
helper `Process` and a `Guest` are both admitted, and which of them is
permitted is still the endpoint's own consumer policy to decide. A row may
carry a `providerRef` in its envelope layer naming the endpoint family's
Provider; a row naming a Provider outside this family is refused, and a row
naming none is served by the declaring Provider, which is this type's own
driver.

The exact endpoint a row delivers is resolved from the committed `Endpoint`
row, never from the binding's own text: the row has no path field and no
purpose of its own - the purpose the delivery rides is the endpoint row's -
so nothing in the desired state can be rewritten into a different socket.

The row is read through exactly one seam in this crate, and no row field is
copied into a local struct: the type the driver reasons over is the
contract's own.

## Exported resource types

`EndpointBinding` is not exportable. `ResourceExport` admits only qualified
`*.d2bus.org.*Service` types, so a binding is never an export subject. The
declaration carries `exportable: false` and licenses no child creation
(`ENDPOINT_BINDING_CREATIONS` is empty): the `Endpoint` a binding delivers is
the row the binding is owned by, and a minted endpoint would be a second,
unauthorized one - exactly the neighbouring endpoint the family exists to
keep a consumer away from. The exact endpoint is declared as a read
(`ENDPOINT_BINDING_READS = [Endpoint]`).

## Controllers / services / workers / binaries

The crate ships one driver factory, not a standalone process. The driver
serves `EndpointBinding` rows through the `ResourceDriver` verbs:

- `validate` (`validate_spec`) applies the exact-endpoint selector rule - a
  missing, non-string, wildcard, ambiguous, cross-Zone, or wrong-type
  selector is refused - then resolves the named `Endpoint` row through the
  manager under the owner fence, and applies the endpoint's own consumer
  policy, operation allowlist, and attachment ceiling.
- `recover` (`observe`) re-verifies the exact endpoint and re-adopts it; a
  restart never re-delivers an endpoint the consumer already holds.
- `reconcile` observes what the kernel actually applies for this consumer
  (the pinned `(dev, ino)`, the effective rights, the effective traverse
  bit, the containing directory's listability, and whether the endpoint is
  accepting), delivers only what that observation proves, re-delivers when
  the endpoint's inode was replaced underneath the row, registers the
  endpoint and consumer dependency watches, and publishes the fenced
  readiness projection.
- `pre_drain` blocks new use for the relationship ahead of its release.
- `delete` observes whether the consumer is still attached and refuses
  retryably while it is, then releases the delivery idempotently.

`EndpointBindingDriverFactory` is the registration surface;
`binding_descriptor` carries it with the decoder, the type's verbs, execution
domains, reads, the empty creation set, and the `BUILTIN | STARTUP`
allowed-source mask.

## Placement and dependencies

`EndpointBinding` names no placement anchor, so a binding row is reconciled
on its containing Zone's Host. The admitted consumer runs where that consumer
runs; the delivery itself is realized by the Endpoint family's effect
adapter, which owns the endpoint's locator and the consumer launch's
descriptor slots.

The crate depends on `d2b-contracts-resource`, `d2b-provider-endpoint` (the
closed `EndpointSpec`, the endpoint's own admission vocabulary, the
`EndpointAccessObservation` a verification answers with, and the delivery
form a request declares), `d2b-provider-toolkit`, `d2b-resource-runtime`, and
`d2b-resource-types`. The family's driver effects are implemented by this
crate itself (`effects_service`); the daemon-owned half - the exact endpoint
verification, the delivery, the pre-drain fence, the attachment observation,
and the release - crosses the provider boundary as the declared
`EndpointBindingEffectFacets` the composition root supplies, so the crate
carries no host path, socket name, or locator of its own. The daemon hosts
the family's declared effects service
(`endpoint-binding.d2bus.org/effects`) per zone from the family's registered
factory.

## RBAC requirements

The driver holds no broker authority of its own: it runs inside the daemon's
per-zone plane, and every row it reads or writes is reached through the
manager with the plane's own caller identity. A binding whose named
`Endpoint` is the row the manager reports but with another owner uid is
refused closed (`binding-owner-mismatch`) rather than silently re-parented.

## Security posture

The driver realizes a row only under the source's own committed decision: the
right the row's attachment kind claims must be one `spec.source.admittedRights`
admits, and every facet that delivery depends on must be one
`spec.source.realizedFacets` declares. Both checks run in the row seam, so no
verb can act on a decision it never verified. They are distinct from the
endpoint family's declared support, which answers what the implementation can
realize at all, and from the endpoint's own consumer policy, which answers
what this endpoint admits; a row must satisfy all three.

The driver never invents a locator: every value it hands the effect port is a
committed-row fact, and the endpoint's locator stays private to the adapter
that resolves it. Readiness is the verified descriptor, never an assumed one -
a pass that cannot prove the effective access, the ancestor traverse bits, the
non-listable containing directory, or - for an `attach` - an accepting
endpoint delivers nothing and publishes the stable not-ready code. A
delivery whose pinned inode no longer matches the observed one is dropped and
re-established rather than kept. The teardown gate fails closed: a consumer
still attached keeps the durable deleting mark and the delivery. Effects are
idempotent under retry.

## State and telemetry

The type publishes no durable status of its own; the fenced
`EndpointBindingStatusResource` projection the actor writes is the
wire-visible readiness, and `EndpointBindingDriverStatus` (`Delivering`,
`Unproven`, `Recovered`, `Rejected`) is the in-memory projection. Failures
travel as registered failure kinds (`binding-spec-invalid`,
`binding-provider-unsupported`, `binding-owner-mismatch`,
`binding-parent-unavailable`, `binding-parent-spec-invalid`,
`binding-plan-derivation-invalid`, `binding-serving-effect-failed`, and the
shared `children-draining`).

## Build and test

```bash
cargo test -p d2b-provider-endpoint-binding --features test-support
```

The unit tests drive validate, recover, reconcile, pre_drain, and delete over
a scripted serving port and a recording manager endpoint with one shared
ordered log, so the exact endpoint's verification, delivery, and release are
asserted as the manager and the effect adapter record them. The `registration`
suite proves the declaration registers the type through the provider registry
with its decoder and factory, licenses no child creation while reading the
exact endpoint, refuses a duplicate registration, and cannot arrive after the
plane opens.
