# ADR 0051: Security-key semantic backing set and the empty backing allowlist

- Status: Proposed
- Date: 2026-08-02
- Related: [ADR 0046](0046-d2b-3-provider-control-plane.md) (d2b 3.0 Provider
  control plane) and its normative decisions D089, D096, D097, and D098 in
  [`docs/specs/ADR-046-decision-register.md`](../specs/ADR-046-decision-register.md);
  the `device-security-key` dossier
  [`docs/specs/providers/ADR-046-provider-device-security-key.md`](../specs/providers/ADR-046-provider-device-security-key.md)
- Scope: `packages/d2b-contracts/src/v3/provider.rs`,
  `packages/d2b-contracts/src/v3/semantic_services/`,
  `packages/xtask/src/semantic_service_schemas.rs`,
  `docs/reference/schemas/v3/security-key.d2bus.org_projection_spec.schema.json`
- Unblocks: `ADR046-device-004` (T124, semantic half),
  `ADR046-zone-control-019` (T177), `ADR046-zone-control-020` (T178)

## Context

D098 freezes four semantic Service/Binding families. D096 requires every
exportable capability to publish a signed projection factory, one of whose
fields is `allowedBackingRefTypes`: the closed set of same-Zone types the owner
Service may reference. Three families derive that set from a base spec field.
The fourth cannot, and the W5 audit stopped rather than guess.

**Measured, at `2c665603` on `v3`.** The four catalog declarations in
`packages/d2b-contracts/src/v3/semantic_services/`:

| Family | Service base field naming a backing | `allowed_backing_ref_types` |
| --- | --- | --- |
| audio | `implementationEndpointRefs` | `Some(["Endpoint"])` |
| telemetry | `ingestEndpointRefs` | `Some(["Endpoint"])` |
| USB | `backingDeviceRef` | `Some(["Device"])` |
| security key | none | `None` |

`SECURITY_KEY` `SERVICE_SPEC_ALLOWED` is exactly
`["providerRef", "updatePolicy", "mode", "authority"]`. The dossier places
`deviceRef` and `relayEndpointRef` inside
`spec.provider.settings`, the implementation's strict D089 extension, and the
committed test `the_physical_device_selector_is_not_a_base_field` pins that
`deviceRef` is rejected from the base. So no semantic base field of this family
names a backing resource, and there is nothing to derive a closed set from.

`SemanticProjectionBinding::projection_factory` therefore returns
`Err(SemanticContractError::BackingRefTypesUndetermined)` for security key and
`Ok` for the other three. That is the correct fail-closed reading of a document
that does not state the set; it is also a hard block.

**Measured: the empty set is not an available spelling today.**
`ProjectionFactory::new` in `packages/d2b-contracts/src/v3/provider.rs` rejects
an empty backing set. Constructed against the live security-key contract with
`std::iter::empty()` as the backing argument:

```text
EMPTY_FACTORY_RESULT = Err(BoundExceeded)
```

So the catalog had exactly two options: invent a non-empty set, or carry `None`
and a typed error. It carried `None`.

**Measured: the committed artifact publishes a fingerprint for a factory that
cannot exist.** `docs/reference/schemas/v3/security-key.d2bus.org_projection_spec.schema.json`
carries `"x-d2b-allowed-backing-ref-types": null` alongside
`"x-d2b-factory-fingerprint": "sha256:444b0f9b..."`. A published factory
fingerprint for a family whose factory constructor fails is a live drift, not a
cosmetic one: an importing Zone compares `expectedFactoryFingerprint` against
that published value and would match a factory nobody can build.

**What forces the decision now.** `ADR046-zone-control-019` and `-020`
implement export admission and the import-owned projection lifecycle over
factory metadata. `ADR046-device-004` must publish this family's factory in its
signed descriptor. All three find three families that work and a fourth that
returns a typed error. `specs/001-adr046-d2b3-completion/implementation-debt.md`
records this at 13.3 and carries it forward at 19.8 as a specification gap whose
remedy is owner work, not slice work. This record is that work.

**The constraint that rules out the obvious answer.** `Device` is the plausible
guess. It is wrong for two independent reasons, and both are stated in the
specification set rather than inferred.

1. D096 requires the factory fingerprint to bind "the semantic factory fields
   plus the projection-protocol version; never Provider or adapter identity or
   implementation detail." For this family the physical `Device` *is* the
   implementation detail: it lives in `spec.provider.settings`. Putting `Device`
   in a provider-neutral fingerprint puts an implementation choice in the one
   value that is required not to carry one.
2. D096 also requires that "export/import preserves `serviceType` exactly even
   when owner and consumer select different conformant implementations through
   local `providerRef`", and the dossier states that "a future Provider
   implements these same exact semantic types through a different `providerRef`
   and its own signed extension". The catalog already exercises exactly that,
   with a second implementation named `security-key-alternate`, in
   `every_implementation_passes_the_identical_base_fixture`. An alternate
   implementation that backs its authority onto an `Endpoint` rather than a
   hidraw `Device` would be excluded by a `Device`-only semantic set, or would
   force a fingerprint divergence that breaks cross-implementation import. The
   backing type is not a property of the semantic family; it is a property of
   the implementation.

The dossier says the same thing in one line, in the D096/D097 section:

> The physical Device is backing inventory, not the service authority.

The authority is the Service. The `authority` base field is the D097
`AuthorityDescriptor` whose `authorityRef` points at the Service itself. There
is no backing to name at the semantic layer because, for this family, there is
no semantic backing.

**Non-negotiable constraints this decision must preserve.** Zone isolation and
no cross-Zone ResourceRef, FD, path, locator, or grant; the single
`LeaseId`-guarded ceremony authority; opaque session, CID, and relay data on
every public and broker surface; no host credential ownership; and fail-closed
behaviour on every cross-Zone and reference path.

## Decision

### 1. The security-key semantic backing set is empty, and that is determinate

`packages/d2b-contracts/src/v3/semantic_services/security_key.rs` declares

```rust
const ALLOWED_BACKING_REF_TYPES: &[&str] = &[];
// ...
allowed_backing_ref_types: Some(ALLOWED_BACKING_REF_TYPES),
```

This is a positive statement, not an absence: the provider-neutral
`security-key.d2bus.org.SecurityKeyService` names **no** same-Zone backing
resource in its base, and therefore admits none. It is not "undetermined", and
it is not "any".

### 2. An empty backing allowlist denies every backing reference

Normative, and binding on every consumer of `allowedBackingRefTypes`:

> `allowedBackingRefTypes` is a closed allowlist. The empty allowlist is the
> minimum of the permission lattice: it admits no backing reference of any
> type. It is never read as "unconstrained", "not applicable", or "skip the
> check".

An empty set is therefore always safe on the wire. A signed descriptor that
carries an empty set for a family whose base *does* declare a backing-reference
field makes that family inert, which is a denial and not an escalation. There is
no reading of an empty set under which a reference is admitted that a non-empty
set would have refused.

### 3. `ProjectionFactory::new` accepts an empty backing set and keeps rejecting an empty target set

The lower bound moves from one to zero for `allowed_backing_ref_types` only.
`allowed_binding_target_ref_types` keeps its non-empty requirement, and
`MAX_PROJECTION_REF_TYPES` keeps its upper bound.

The asymmetry is deliberate and is the reason this is not a general loosening.
An empty **target** set means no Binding can name any consumer, so no consumer
intent can ever be authored and the family is dead rather than restricted: an
inert factory row is a manifest defect, exactly as an `Exportability::Forbidden`
factory already is. An empty **backing** set means the owner Service names its
backing outside the semantic layer, which is a coherent and already-shipped
shape.

### 4. `None` survives as the undetermined spelling; security key stops using it

`SemanticContractError::BackingRefTypesUndetermined` and the `Option` in
`SemanticPairDeclaration` stay. A future family whose dossier genuinely does not
state its set must still fail closed with a typed error rather than default to
the empty set, because "we do not know" and "there is none" are different claims
and only one of them is safe to sign. After this decision no shipped family uses
`None`; the branch is retained for the next gap, not deleted as dead code.

Consequently `null` and `[]` remain distinct fingerprint inputs. They are
distinct claims, so they must hash differently.

### 5. The catalog declares which base fields carry a backing ref, and the coupling is asserted

`SemanticPairDeclaration` gains one field:

```rust
/// The Service base spec field names that carry a same-Zone backing
/// ResourceRef. Empty for a family whose authority names no backing
/// resource in its provider-neutral base.
pub(crate) service_backing_ref_fields: &'static [&'static str],
```

with values `["implementationEndpointRefs"]` (audio),
`["ingestEndpointRefs"]` (telemetry), `["backingDeviceRef"]` (USB), and `[]`
(security key).

Two catalog-wide invariants, asserted over every family:

- **Coupling.** When `allowed_backing_ref_types` is `Some(set)`,
  `set.is_empty()` holds if and only if `service_backing_ref_fields.is_empty()`
  holds. A family may declare the empty backing set only because its base
  declares no backing-reference field, and a family that declares one may not
  declare the empty set.
- **Grounding.** Every name in `service_backing_ref_fields` is a member of that
  family's `service_spec_allowed`.

`service_backing_ref_fields` is **not** a fingerprint input. It is a
catalog-internal consistency declaration, D096 does not name it, and feeding it
into `factory_fingerprint` would move all four fingerprints for no contract
reason.

This is the guard that stops the empty set from being "fixed" later by someone
adding `Device` to the list without adding a base field for it to apply to.

### 6. Backing admission is an unconditional membership test behind a typed method

`ProjectionFactory` gains

```rust
/// Decide whether an owner Service may name the supplied backing reference.
///
/// The allowlist is closed and the test is unconditional: an empty allowlist
/// admits nothing.
pub fn admits_backing_ref(
    &self,
    resource_ref: &ResourceRef,
) -> Result<(), ProviderContractError>;
```

`ADR046-zone-control-019` calls this method. It does not read
`allowed_backing_ref_types()` and branch on emptiness. Section "Consequences"
names the specific bug this forecloses.

### 7. A family's backing set may not contain its own Service or Binding type

`ProjectionFactory::new` already rejects `service_type == binding_type` with
`ConflictingFields`. It additionally rejects a backing set containing either
`service_type` or `binding_type`.

This forbids a Service chaining onto another Service of the same family as its
"backing", which is the shape that would let an imported projection Service in a
consumer Zone be presented as local backing authority. D096's allowance of
"qualified semantic backend types" in a backing set is preserved for a
*different* family's type; only self-chaining is closed.

### 8. Core admits a Provider-advertised factory only if it equals the catalog-derived factory

For any `serviceType` in the D098 catalog, a Provider descriptor's
`ProjectionFactory` is admitted at Provider install, Nix build, and API
admission only when it is equal field for field, including
`allowedBackingRefTypes`, `allowedBindingTargetRefTypes`,
`projectionSchemaFingerprint`, and `factoryFingerprint`, to the factory derived
from `d2b-contracts::v3::semantic_services`. A Provider cannot widen, narrow, or
restate the semantic factory; it may only add its own strict extension schemas.

This is what makes decision 1 safe independent of decision 2: a descriptor
cannot reach the empty set for a family whose catalog entry is non-empty,
because the whole factory has to match.

### 9. The fingerprint consequence, pinned

Measured against the live contract by re-deriving `factory_fingerprint` with an
empty backing set in place of `None`:

| Value | Before | After |
| --- | --- | --- |
| security-key `projectionSchemaFingerprint` | `sha256:8b63b67989477970d9a0196695aa040b806061a13bacc79e6e71fbc274006d6f` | unchanged |
| security-key `factoryFingerprint` | `sha256:444b0f9ba1d9997a392314a5aa9b49ca180a2e8c5d7541d1eb1846d2b3c460dc` | `sha256:57f1d41aa8740a8f7012f42a1d06686454f33d4b4265cabee358b4a9519ead6e` |
| security-key `x-d2b-allowed-backing-ref-types` | `null` | `[]` |
| audio, telemetry, USB projection artifacts | `80ef3d08...`, `de3ef22c...`, `72b5cafb...` | byte-identical |

The projection *schema* fingerprint does not move because the projection field
set does not move. Only the factory fingerprint moves, because only the backing
set is an input to it.

Exactly one committed file pins the old value: the generated artifact itself.
Regeneration is `run_xtask gen-semantic-service-schemas`, gated by the enforcing
`make test-drift` lane. The dossier's Nix examples use placeholder tokens
(`sha256:<security-key-projection-factory>`), not literals, so no prose pins it.

### 10. Where the physical-backing guarantee actually lives

The projection factory is not, and never was, what proves an owner
`SecurityKeyService` is backed by a real physical key. Recorded here so that
nobody reads the empty set as a lost check. The guarantee is carried by three
surfaces that this decision does not touch:

1. The mandatory Core-derived claim on the exact Host-global tuple
   `(Host, physical-usb-backing, opaqueKeyDigest)`, admitted before Core permits
   the relay DeviceGrant or hidraw open, and shared with every USB Provider so a
   second claimant on the same token loses with
   `physical-usb-backing-conflict`.
2. The D097 `AuthorityDescriptor` on the Service, with
   `cardinality: zero-or-one`, `arbitration: exclusive`, and
   `ownerProof: service-and-relay-process-identity` for restart adoption.
3. The signed, strict, deny-unknown `spec.provider` extension schema that
   validates `deviceRef` and `relayEndpointRef` against `spec.providerRef`, plus
   the deny-unknown semantic base that already rejects `deviceRef` from
   `spec`.

## Consequences

**The specific bug this design makes possible, and the guard that catches it.**
Three of four families have a non-empty backing set and one does not. The
natural shape for `ADR046-zone-control-019` to write is

```rust
// the defect this decision forecloses
if !factory.allowed_backing_ref_types().is_empty() {
    require_member(backing_ref.resource_type(), factory.allowed_backing_ref_types())?;
}
```

Under that shape the empty set means "skip the check", and an owner
`SecurityKeyService` naming any backing reference at all is admitted. That is
not cosmetic. `ProjectionFactory::admits_export_target` is type-only and
mode-blind, so a resource of the family's own Service type is an accepted export
target regardless of whether it is an authority or an import-owned projection. A
"skip when empty" backing check plus a mode-blind export check is a path from a
consumer Zone's imported projection to a re-exported authority claim, which
breaks Zone isolation rather than degrading it.

Three guards, in order of strength. Decision 6 gives the check a typed method
whose membership test is unconditional, so the branch above has nowhere to live.
Decision 7 removes the family's own Service type from every backing set, so the
chain has no first link. Decision 5 stops a later author from resolving the
oddity by inventing a non-empty set. The negative test named below asserts that
the security-key factory rejects `Device/x`, `Endpoint/x`, and
`security-key.d2bus.org.SecurityKeyService/x`.

**The second specific failure: a silent fingerprint move.** The security-key
`factoryFingerprint` changes. Any Provider descriptor, fixture, or example that
pins `444b0f9b...` will fail import expectation matching after this lands. That
failure is fail-closed and correct, but it reads as a tamper error rather than a
version skew. The guard is that the value is pinned in both directions in
decision 9, the sole committed pin is the generated artifact, and `make
test-drift` fails until it is regenerated. The mechanically checkable condition
is that after regeneration the security-key artifact carries
`57f1d41a...` and `[]`, and the other three artifacts are unchanged in
`git diff`.

**What this makes easy.** All four families return `Ok` from
`projection_factory()`, so `ADR046-zone-control-019` and `-020` have one code
path with no family special case, and `ADR046-device-004` can publish this
family's factory in its signed descriptor. The audit's blocking entry at
implementation-debt 13.3 and 19.8 is discharged by an amendment, which is what
it always said the remedy was.

**What this makes hard, honestly.** A future security-key implementation that
*does* want a semantic backing reference, for example one that backs onto a
first-class `Endpoint` visible to Core rather than to the Provider, cannot get
one by editing a factory. It has to add a base spec field to the frozen
security-key Service base, which moves that base's schema fingerprint, its
factory fingerprint, and both committed artifacts, and requires re-signing every
descriptor that pinned them. That is the correct cost for changing a frozen
provider-neutral base, and it is higher than it would have been if we had
speculatively admitted `Device` and `Endpoint` up front. We are choosing the
smaller design that can be extended over the larger one that anticipates.

**What this forecloses.** It forecloses reading `allowedBackingRefTypes` as an
optional constraint anywhere in the tree. Any future family that wants "no
constraint" has no spelling for it and must enumerate.

**What this deliberately does not decide.** Whether a `ResourceExport` may
target an import-owned projection Service, that is, whether a capability may be
re-exported to a grandchild Zone. `admits_export_target` accepts any resource of
the Service type and does not consult `spec.mode`, and the zone-control spec's
"every hop applies RBAC and a capability ceiling" reads as though chaining is
contemplated. That is a distinct question about export admission, not about the
backing set, and deciding it inside this record would be scope creep. It is
recorded here so it is not lost, and it belongs to `ADR046-zone-control-019`
as a separate finding.

**Residual risk this decision does not remove.** Decision 8 makes Core compare a
Provider's advertised factory against the catalog. Nothing in this decision
proves that the catalog itself was derived from the dossier rather than from a
plausible reading of it; that is what the amendment in the next section is for,
and it is why the amendment is normative dossier text and not an ADR-only claim.

## Alternatives considered

**Declare `allowedBackingRefTypes: ["Device"]`.** The plausible guess, and the
one the W5 audit named and refused. Rejected on the two grounds in Context: it
puts an implementation detail into a fingerprint D096 requires to carry none,
and it excludes the alternate implementation the dossier promises and the
catalog already tests. It also constrains nothing, because there is no base
field for it to apply to, so its only observable effect would be the fingerprint
it changes.

**Declare `["Device", "Endpoint"]`.** Strictly worse than the above. It is the
same fingerprint pollution, plus it is a guess about a union rather than a
guess about a single type, and a wider allowlist is the wrong direction for a
fail-closed surface.

**Ground the set in `spec.authority.authorityRef`, giving
`["security-key.d2bus.org.SecurityKeyService"]`.** Superficially attractive
because `authorityRef` is a real base field carrying a real ResourceRef, and
D096 permits "qualified semantic backend types". Rejected on three counts.
`authorityRef` names the authority owner, which for an owner Service is itself;
a self-reference is not a backing. The catalog deliberately does not freeze the
interior of the `authority` field, so grounding a closed set in
`authority.authorityRef` would freeze a member name the dossier states only in
an example. And it creates precisely the Service-chaining hazard that decision 7
closes.

**Rule that a family with no semantic backing has no projection factory, and
make security key non-exportable.** This is the literal reading of the second
option the audit offered. Rejected because it contradicts an Accepted normative
dossier at version 6 across a large surface: `exportability: explicit-export` in
the authority descriptor, the entire `mode: projection` branch, the
`security-key-projection-controller` Role, the import lease states in
`status.resource.import`, the `security-key-import-invalid` error class, the
ResourceImport finalizer ordering, and the Nix authoring examples for a consumer
Zone. It also removes real capability: one physical security key shared from an
owner Zone to child Zones is among the strongest motivations for D096 existing
at all. Deleting a designed feature to avoid stating a one-word fact is the
wrong trade.

**Keep `None` and special-case security key in `ADR046-zone-control-019`.**
Rejected. It pushes a permanent branch into export admission, which is exactly
the surface where a branch becomes a "skip the check" bug, and it leaves the
generated artifact publishing a factory fingerprint for a factory that cannot be
constructed.

**Make `allowedBackingRefTypes` optional on the wire, `Option<BTreeSet<_>>` in
`ProjectionFactory`.** Rejected. It moves the ambiguity from the catalog into
the signed descriptor, where a missing field and an empty field would then have
to be distinguished by every verifier, and where the "absent means unconstrained"
misreading is far more likely than it is behind a typed method.

## Normative amendments this decision requires

These are the exact changes that consume the decision. They are drafted here and
land with the implementing wave, not with this record.

### A. `docs/specs/providers/ADR-046-provider-device-security-key.md`

Under "Security-key authority and cross-Zone sharing (D096/D097)", after the
"Service export/import (D096)" paragraph, insert a subsection matching the shape
the `device-usbip` and `audio-pipewire` dossiers already use:

> ### Signed D096 projection factory
>
> The Provider descriptor carries exactly one signed factory:
>
> | Field | Security-key value |
> | --- | --- |
> | `serviceType` | `security-key.d2bus.org.SecurityKeyService` |
> | `bindingType` | `security-key.d2bus.org.SecurityKeyBinding` |
> | `allowedBackingRefTypes` | empty; this family's provider-neutral base names no backing resource |
> | `allowedBindingTargetRefTypes` | `Guest`, `User` |
> | `projectionSchema` | strict same-type projection schema with `providerRef` and the observed `mode`; no `spec.provider`, `deviceRef`, `authority`, physical selector, DeviceGrant, or raw locator/path/credential/fd/bytes |
> | `projectionSchemaFingerprint` | SHA-256 of the canonical committed projection schema |
> | `factoryFingerprint` | SHA-256 binding the semantic factory fields plus the projection-protocol version; never Provider or ExportAdapter/ImportAdapter identity or version |
>
> The empty `allowedBackingRefTypes` is a closed deny-all, not an absent
> constraint. Core rejects an owner `SecurityKeyService` that names any
> same-Zone backing reference in its provider-neutral base, including a
> reference to another `SecurityKeyService`. The physical `Device` and the relay
> `Endpoint` are implementation children named only in
> `spec.provider.settings`; the physical Device is backing inventory, not the
> service authority. The guarantee that an owner Service is backed by one real
> physical key is carried by the mandatory
> `(Host, physical-usb-backing, opaqueKeyDigest)` claim admitted before any
> DeviceGrant or hidraw open, by the D097 `AuthorityDescriptor`, and by the
> signed Provider extension schema, not by the projection factory.
>
> Provider install, Nix admission, API admission, export, and import fail closed
> if the factory, its signature, the Service/Binding pair, the allowed reference
> sets, or either fingerprint differs from the catalog-derived factory for this
> `serviceType`.

### B. `docs/specs/ADR-046-provider-model-and-packaging.md`

Replace the `allowedBackingRefTypes` row of the D096 factory table:

> | `allowedBackingRefTypes` | Closed set of same-Zone `Device`, `Endpoint`, or qualified semantic backend types the owner Service may reference **in its provider-neutral base**. Empty when the family's base declares no backing-reference field. The empty set denies every backing reference and is never read as unconstrained. It may not contain the factory's own `serviceType` or `bindingType`. |

### C. `docs/specs/ADR-046-resources-zone-control.md`

Replace the `allowedBackingRefTypes` row of the 8A.1.1 factory table with the
same text as amendment B, and append to the paragraph beginning "Provider
install, Nix build, and API admission verify this metadata":

> For a `serviceType` in the D098 semantic catalog, the advertised factory is
> admitted only when it equals the catalog-derived factory field for field,
> including both reference sets and both fingerprints. A Provider adds strict
> extension schemas; it never restates, widens, or narrows the semantic factory.

### D. `docs/specs/ADR-046-decision-register.md`

No amendment required, and this is deliberate. The D096 row already says
"allowed backing/target refs" without stating a lower bound, so it is not
contradicted. The normative reading of the empty set lands in the two specs and
the dossier, which are the documents an implementer reads.

### E. `specs/001-adr046-d2b3-completion/implementation-debt.md`

Sections 13.3 and 19.8 record this gap as open. Both gain a closing line naming
this ADR and the amendment that discharges it. Ownership of that edit belongs to
whichever wave lands amendments A to C, not to this record, because the debt
register describes the tree and the tree does not change until then.

## Tests required to consume this decision

Each is mechanically checkable and named so a wave's stopping condition can cite
it.

In `packages/d2b-contracts/src/v3/semantic_services/security_key.rs`, replacing
`the_backing_ref_set_is_undetermined_and_fails_closed`:

- `the_backing_ref_set_is_empty_and_the_factory_is_constructible` - the backing
  set is `Some` and empty, and `projection_factory()` returns `Ok`.
- `the_empty_backing_set_admits_no_backing_reference` - the constructed factory
  rejects `Device/yubikey-primary`, `Endpoint/yubikey-primary-ctaphid-relay`,
  and `security-key.d2bus.org.SecurityKeyService/yubikey-primary`. Negative
  control: the USB factory admits `Device/work-token`, so the assertion is
  capable of failing.

In `packages/d2b-contracts/src/v3/semantic_services/mod.rs`, over the whole
catalog:

- `a_backing_set_is_empty_exactly_when_the_base_names_no_backing_field` -
  decision 5's coupling invariant, both directions.
- `every_declared_backing_field_is_a_service_base_field` - decision 5's
  grounding invariant.
- `no_family_admits_its_own_service_or_binding_type_as_backing` - decision 7.
- `every_family_yields_a_projection_factory` - all four return `Ok`, replacing
  the current three-of-four state.
- `the_undetermined_spelling_still_fails_closed` - a locally constructed
  declaration with `allowed_backing_ref_types: None` still yields
  `BackingRefTypesUndetermined`, so decision 4's retained branch is exercised
  rather than left as unreachable code.

In `packages/d2b-contracts/src/v3/provider.rs`:

- `an_empty_backing_set_is_accepted_and_an_empty_target_set_is_not` - decision
  3's asymmetry, both halves.
- `a_factory_may_not_name_its_own_service_or_binding_type_as_backing` -
  decision 7 at the constructor.
- Round-trip: a factory with `allowedBackingRefTypes: []` serializes and
  deserializes without loss, and the deserializer still rejects an empty target
  set.

In `packages/d2b-provider-toolkit/tests/malicious_provider.rs`:

- A descriptor advertising a security-key factory with a non-empty backing set,
  or with the catalog's set but a recomputed fingerprint, is rejected by
  decision 8's equality check.

Artifact and drift:

- `run_xtask gen-semantic-service-schemas` regenerates
  `docs/reference/schemas/v3/security-key.d2bus.org_projection_spec.schema.json`
  with `"x-d2b-allowed-backing-ref-types": []` and
  `"x-d2b-factory-fingerprint": "sha256:57f1d41aa8740a8f7012f42a1d06686454f33d4b4265cabee358b4a9519ead6e"`,
  and leaves the audio, telemetry, and USB artifacts byte-identical. Enforced by
  `make test-drift`.

Lanes that must be green for the implementing wave: `make test-rust`,
`make test-drift`, `make test-fixture-contracts`, `make check-tier0`,
`make test-policy`.

## Invariants this decision creates

1. `allowedBackingRefTypes` is a closed allowlist and the empty allowlist admits
   nothing. No consumer branches on its emptiness.
2. A semantic family declares the empty backing set if and only if its
   provider-neutral Service base declares no backing-reference field.
3. Every name in a family's `service_backing_ref_fields` is a member of that
   family's `service_spec_allowed`.
4. `service_backing_ref_fields` is never a fingerprint input.
5. A factory's backing set never contains its own `serviceType` or
   `bindingType`.
6. `allowed_binding_target_ref_types` is always non-empty.
7. `None` means "the specification does not state this set" and continues to
   fail closed with `BackingRefTypesUndetermined`. It is never a synonym for the
   empty set, and `null` and `[]` hash differently.
8. For a `serviceType` in the D098 catalog, an advertised `ProjectionFactory` is
   admitted only when equal field for field to the catalog-derived factory.
9. The security-key provider-neutral base continues to reject `deviceRef`,
   `relayEndpointRef`, `authority` in a projection, and every physical selector.
