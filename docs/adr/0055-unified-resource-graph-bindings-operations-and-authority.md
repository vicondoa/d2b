# ADR 0055: Unified resource graph bindings, operations, and authorization-only roles

- Status: Accepted
- Date: 2026-09-30
- Related: [ADR 0046](0046-d2b-3-provider-control-plane.md) (d2b 3.0 Provider
  control plane) and its normative decision D035 in
  [`docs/specs/ADR-046-decision-register.md`](../specs/ADR-046-decision-register.md);
  [ADR 0015](0015-daemon-only-clean-break.md) (daemon-only clean break);
  [ADR 0034](0034-storage-lifecycle-restart-and-synchronization.md)
  (storage lifecycle, restart, and synchronization)
- Scope: the resource and authority contracts that model the five primitive
  binding relationships, the externally callable `Operation` declaration, the
  `ExecutionPolicy` and `SeccompProfile` confinement rows, and the `Role`
  authority contract. These are
  `packages/d2b-contracts-resource/src/v3/binding.rs`,
  `volume_binding.rs`, `device_binding.rs`, `endpoint_binding.rs`,
  `network_binding.rs`, `credential_binding.rs`, `operation.rs`,
  `execution_policy_resource.rs`, `seccomp_profile.rs`, and
  `packages/d2b-contracts-zone-session/src/v3/role.rs`, plus the per-crate
  provider declarations that state which type each provider serves.
- Non-scope: the qualified Provider Service/Binding projection families, the
  broker operation catalog, the Zone bundle and manifest compatibility
  projection, and the vendor ResourceType qualification grammar. ADR 0046's
  resource/provider separation is retained in full; this record changes only
  the resource and policy contracts it names.

## Context

ADR 0046 froze a model in which three authorities sat beside the graph.

A relationship between a source resource and a consumer was not a thing the
graph held. A device grant was a template name, a Volume attachment was a
field on the attachment, and a network membership was a derived runner
resource. The authority lived in a table a provider row or a template name
chose, so two readers of the same committed graph could derive two different
capabilities.

An `Operation` row did not say who answered it. The executable declaration sat
in a separate `Command` resource that a mutable row could name, and a provider
row that named an untrusted artifact could therefore introduce a compiled
privileged handler.

A `Role` row carried two unrelated things. Its rules granted verbs, and its
posture facet separately carried seccomp profile, principal, capabilities,
namespaces, mounts, umask, and user namespace authority. Execution and access
authority were reachable by selecting a row, which is a second grant path next
to the RBAC rules that already exist.

## Decision

### 1. A binding is two-sided, and both sides are committed rows

`BindingKind` is a closed vocabulary of five: `Volume`, `Device`, `Network`,
`Endpoint`, and `Credential`. Each kind materializes a row of its own
ResourceType, named by `BindingKind::resource_type`: `VolumeBinding`,
`DeviceBinding`, `EndpointBinding`, `NetworkBinding`, and `CredentialBinding`.

The **source side** is a request. A consumer publishes a typed binding request
naming the exact source, the requested rights, a stable consumer slot, and a
bounded destination. A request is not authority and has no constructor that
reaches admitted use from desired fields alone. No request names a raw host
source path, a numerical host principal, secret material, or a free-form
command line; the source side is always an exact typed reference plus its own
named view or function, and the consumer side is an exact typed reference plus
a bounded destination.

The **source decides**. `SourceAdmission` binds one exact `BindingKey`, the
rights the source admits, and how the source arbitrates that relationship.
`admit_binding_request` then evaluates the request against authorization
evidence the `Role` and `RoleBinding` contracts produced, the source's own
decision, and the realization the selected backend declares. Evaluation is
deterministic and stops at the first refusal, so a caller always sees one
enforcing stage. Nothing in it widens the request: a right the kind does not
admit was already refused by the constructor, a right the source does not
admit is refused, an exclusive claim the source is not arbitrating is refused,
a required presentation the selected realization cannot enforce is refused, and
an unfenced or empty admission is refused.

The **row side** is the committed relationship. The admitted relationship is
committed as a row of its own served ResourceType whose desired bytes are the
request itself, so a reader and the graph cannot disagree about what was
admitted. A dedicated provider crate serves each type: it registers the
driver, decodes the spec, and reconciles the row into the actual host, device,
network, or credential realization, releasing it on finalize or delete. Each
declares the types it serves in its `resource-types.json`; the authority for
the set of types the plane serves is the generated
`V3_CONVERTED_RESOURCE_TYPES` list, which the provider-crate layout check
regenerates from those declarations. This record deliberately does not restate
that list, because a hand-copied catalog of a generated file is a second
thing to drift.

A source ensures every binding row it derives and retires the ones it no
longer derives. Its readiness reads the child rows' phases rather than
overriding them, so a relationship that never committed is not reported as
served.

### 2. A realization is a declared capability, and an unenforced one is a refusal

`BindingRealizationFacet` is the closed vocabulary of what a presentation
actually is: a filesystem presentation carrying a named view, a block device in
a consumer device slot, a device attachment, a namespace interface, membership
in a provider-owned shared fabric, an endpoint descriptor, an endpoint
pathname, and credential delivery inside an admitted delivery session.

A source publishes the facets its selected backend realizes as
`BindingRealizationSupport`. A required facet the support set does not contain
is refused with the mandatory-facet reason. An unapplied mount policy, an
unclaimed device, and an unproven endpoint are not successes.

`BindingAdmission` is a fence, not a grant. It names the exact dependency
versions the admission was evaluated against, so an ownership, view, consumer,
provider-assignment, or policy change that does not advance a generation still
invalidates the earlier use. `SourceReservation` is identity evidence and
carries no capability: the broker-minted handle that realizes a delivery is
absent from the contract by design, so nothing in a committed row can be
replayed as access.

### 3. An `Operation` names a trusted implementation, not a resource

`OperationImplementation` has exactly two variants, `ProviderMethod` and
`TrustedExecutableTemplate`. Both name a declared `Provider` plus an identity
inside that provider's contract, and both refuse a reference to any other
ResourceType. An `Operation` row therefore cannot be answered by a resource a
mutable row could name, and a provider resource that names an untrusted
artifact cannot introduce a compiled privileged handler. Compiling a declared
implementation into a callable handler is the deployment's job.

`CallableOperation` is the whole declaration: payload and result schemas,
payload provenance, destructiveness, the secret-access ceiling, the audit facet
and its join, the authority facet, descriptor carriage, and bounds. It carries
no owning `Command` reference and no inherited wire discriminant, and its wire
mirror denies unknown fields, so a row carrying one is rejected rather than
decoded with its authority dropped.

### 4. `Role` is authorization-only

`AuthorizedRole` carries bounded `RoleRule` entries plus the `Operation` rows
the holder may create. The independent execution and access authority the
historical posture, path-grant, and command-reference facets carried is exactly
what this contract removes, and the wire mirror denies those fields, so a row
that still carries one is rejected rather than decoded with its authority
quietly dropped.

### 5. Confinement is stated apart from access

`ExecutionPolicy` states what an execution instance is allowed to be and
nothing more: the namespaces it runs under, its capability ceiling, whether
new privileges are forbidden, the identity it may resolve to, its
root-filesystem restrictions, the syscall filter it must load, and its umask.
It carries no volume, device, network, endpoint, credential, mount, or
host-path field, because those are separate typed binding relationships and
a policy that named them would be a second authority.

`SeccompProfile` states the syscall allowlist and its default action and
nothing else. Namespace isolation, cgroup confinement, device-node access, and
mount authority are not syscall-filter concerns.

`admit_execution` composes field-wise and treats a conflict as a refusal. A
missing required class, a capability outside the ceiling, a weakened mandatory
restriction, an unauthorized identity, a request over the admitted budget
ceiling, or a mandatory facet the target cannot enforce is refused with its
enforcing stage, never intersected into a successful but unconfined launch.
Selecting an `ExecutionPolicy` is itself a request: without `Role` and
`RoleBinding` evidence the selection is refused even when every other rule
would admit it.

## Supersedes

This record supersedes exactly two things in ADR 0046 and its register.

**D035's frozen standard catalog.** D035 froze nineteen standard,
unqualified, Zone-unique ResourceTypes and stated that other behavior is
inline or Provider-specific. That enumeration is no longer the catalog: a
binding relationship is a committed row of its own type rather than inline
behavior, and the execution model adds rows of its own. The generated
`V3_CONVERTED_RESOURCE_TYPES` list is the authority for what the plane serves,
and this record does not restate it.

Nothing else in ADR 0046 is withdrawn. The resource/provider separation, the
layered spec and status shapes, the ownership and deletion ordering, the
Provider installation model, and the standard-versus-qualified grammar all
stand as accepted.

## Consequences

`Operation`, `SeccompProfile`, and `ExecutionPolicy` are system-homed: only
the foundation plane may write them, and a zone-local plane refuses the write
terminally. That is the enforcement point in `d2bd`'s foundation seed.

An operator or consumer that needs a Volume, a device, a network, an endpoint,
or a credential now gets it by requesting a relationship and having the source
admit it, not by naming a template, a launch role, a device-node path, or a
posture row. Every one of those refusals is a named refusal with an enforcing
stage, not a downgrade.

The cost is that a relationship is now a durable object with its own
readiness, finalizer, and drain. A source that cannot retire its binding rows
reports `FinalizationBlocked` rather than leaking the realization, which is the
same drain rule the rest of the graph already follows.

## Rejected alternatives

**Keep the request inline and admit at use time.** Rejected: the graph would
hold no record of what was admitted, so readiness could not aggregate it and a
restart could not tell an applied relationship from a merely intended one.

**Let the consumer commit the binding row directly.** Rejected: that makes the
consumer the grant authority, which is the posture facet's mistake at a
different layer. The row is committed by the source from the admitted request,
so the committed bytes and the admission cannot disagree.

**Keep `Command` as the executable declaration and forbid mutable providers
from naming it.** Rejected: a prohibition is not an identity. Naming a declared
provider method makes the property structural, because there is no resource
left to name.

**Keep the `Role` posture facet and refuse it at admission.** Rejected: a
refusal path for a field the contract should not have is a field that still
needs a compatibility story. `Role` is authorization-only and the execution
model is named by the row that actually carries it.

**Fold `SeccompProfile` into `ExecutionPolicy`.** Rejected: a syscall filter is
reusable across policies, and a deployment that cannot honor a declared
restriction is refused at admission rather than at the filter.

## Invariants this decision creates

- No binding request converts itself into authority: admitted evidence requires
  authorization, a matching source decision, realized facets, and fenced
  dependencies.
- A relationship's identity is its Zone, source, consumer, kind, and stable
  consumer slot. Rights, destination, and presentation are outside it, so
  changing what a consumer asks for updates the relationship instead of
  minting a second one.
- No `Operation` row names a resource. Its implementation is a declared
  provider identity, and a non-`Provider` reference is refused.
- No `Role` row carries a posture, a mount, or a command reference.
- No `ExecutionPolicy` or `SeccompProfile` row grants resource access.
- A required realization facet the selected backend cannot enforce is a
  refusal, never a skip.
