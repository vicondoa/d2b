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
  `packages/d2b-core-controller/src/export_import.rs` (export and backing
  admission), `packages/xtask/src/semantic_service_schemas.rs`,
  `docs/reference/schemas/v3/*_projection_spec.schema.json`
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

**Measured: export admission cannot tell an authority from a projection.**
`ProjectionFactory::admits_export_target` in
`packages/d2b-contracts/src/v3/provider.rs` is, in full:

```rust
if resource_ref.resource_type() == &self.service_type { Ok(()) } else { Err(...) }
```

Its only argument is a `ResourceRef`, which is `<ResourceType>/<name>` and
carries no mode and no owner. So the function is mode-blind by construction, not
by oversight: it could not consult `spec.mode` or `metadata.ownerRef` if it
wanted to. Every projection Service has the same qualified type as the authority
Service it projects, so an import-owned projection is an accepted export target
today. `packages/d2b-contracts/src/v3/` contains no `resource_export.rs` or
`resource_import.rs`, so no admission layer above it compensates: those files
are `ADR046-zone-control-019`'s destination and do not exist yet. That is the
transit path decision 6 closes, and it is reachable from the empty backing set
by way of the "skip the check when empty" shape the Consequences section names.

**What forces the decision now.** `ADR046-zone-control-019` and `-020`
implement export admission and the import-owned projection lifecycle over
factory metadata. `ADR046-device-004` must publish this family's factory in its
signed descriptor. All three find three families that work and a fourth that
returns a typed error, and all three would inherit the mode-blind admission
shape above. `specs/001-adr046-d2b3-completion/implementation-debt.md` records
this at `### 13.3 Security-key cannot construct a signed projection factory at
all` and carries it forward at `### 19.8 The security-key semantic projection is
not invented` as a specification gap whose remedy is owner work, not slice work.
This record is that work.

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
`SemanticBackingDeclaration::NoBacking` (decision 3), which resolves to a closed
backing set that is present and empty.

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

### 3. The backing declaration has two states, and absence is a construction error

The backing declaration is one value with exactly two states, so a type set and
the base fields it applies to cannot drift apart:

```rust
/// A statically non-empty list of catalog constants.
///
/// The first element is a field, not an index, so an empty list has no
/// spelling. Const-constructible: `NonEmpty { first: "Device", rest: &[] }`.
pub(crate) struct NonEmpty<T: 'static> {
    first: T,
    rest: &'static [T],
}

/// What a semantic family declares about its owner Service's same-Zone
/// backing, as one value. There is no "we do not know" state: a family whose
/// dossier does not state its backing is not admitted to the catalog.
pub(crate) enum SemanticBackingDeclaration {
    /// The provider-neutral Service base names no backing resource, so the
    /// closed set is empty and admits nothing.
    NoBacking,
    /// The base names backing-reference fields, and this is their closed
    /// type set. Both lists are non-empty by construction.
    Constrained {
        types: NonEmpty<&'static str>,
        fields: NonEmpty<&'static str>,
    },
}
```

Per family: `Constrained { types: NonEmpty::of("Endpoint"), fields: NonEmpty::of("implementationEndpointRefs") }`
(audio), the same shape with `ingestEndpointRefs` (telemetry),
`NonEmpty::of("Device")` with `backingDeviceRef` (USB), and `NoBacking`
(security key).

**No `Undetermined`, and no `BackingRefTypesUndetermined`.** Both are deleted.
This catalog is a static, compile-time contract; a runtime "the specification
does not say" state has no place in it. A family whose dossier does not state
its backing set is not added to the catalog until it does, so the refusal
happens at construction, before publication, and is a build failure rather than
an error value a caller has to handle. Every catalog member therefore yields a
projection factory, with no fallible backing path at all.

**What this makes unrepresentable, with nothing left over.** A type set with no
field to apply it to, a declared backing field with no type set, an empty
`Constrained` that is a synonym for `NoBacking`, and an undetermined family in a
published catalog: none has a spelling. `NonEmpty` carries its first element as
a field rather than validating a slice length, so emptiness is a type error at
the declaration site and there is no later check to forget. Earlier revisions of
this record validated the degenerate `Constrained { types: &[], fields: &[] }`
at contract build and admitted that as a residual; that residual is gone, and
the build-time check for it goes with it.

**Grounding stays a check, because it cannot be a type.** Every name in
`Constrained.fields` must be a member of that family's `service_spec_allowed`,
which is a relation between two declarations and so is asserted over the catalog
rather than encoded. Decision 4 owns it.

The declaration is **not** a fingerprint input beyond the resolved type set that
D096 already names. `fields` never reaches `factory_fingerprint`: D096 does not
name it, and feeding it in would move fingerprints for a catalog-internal
consistency record.

### 4. `NoBacking` is permitted only when every base field is a classified non-reference field

Decision 3 makes the two halves of `Constrained` move together. It does not, by
itself, stop an author from declaring `NoBacking` for a family whose base *does*
name a backing resource. That coupling is restored here, mechanically.

**There is no authoritative reference-field list in the specification set, and
that is measured, not assumed.** Of the four families' backing fields, exactly
one is stated in a typed table: `ADR-046-provider-observability-otel.md` carries
`| ingestEndpointRefs | [ResourceRef] | authority only | ... |`. The audio and
USBIP dossiers state `implementationEndpointRefs` and `backingDeviceRef` only in
YAML examples and prose, with no type column. So a test cannot read an
authoritative list from the specs today, and a `*Ref` suffix heuristic is
rejected outright: it would classify `providerRef`, `serviceRef`,
`authorityRef`, `producerRef`, and `guestRef` as backings, all of which are
references to something other than a backing.

**The catalog carries the classification explicitly, and it must be total.** One
frozen catalog-level list names every Service base spec field that is *not* a
backing reference:

```rust
/// Every Service base spec field name in the catalog that is not a backing
/// reference. Total with the per-family `Constrained.fields` over the union
/// of all four families' `service_spec_allowed`.
const NON_BACKING_SERVICE_BASE_FIELDS: &[&str] = &[
    "accessPolicy", "authority", "authorityDescriptor", "backingAuthority",
    "mode", "operations", "policy", "providerRef", "quota", "serviceRole",
    "signals", "sourceSchemaFingerprint", "updatePolicy",
];
```

Three catalog-wide assertions:

- **Totality.** For every family, `service_spec_allowed` is a subset of
  `NON_BACKING_SERVICE_BASE_FIELDS` united with that family's declared backing
  fields. A new base field with a new spelling fails this until someone
  classifies it, so the classification cannot silently fall behind the field
  sets.
- **Disjointness.** No name appears in both `NON_BACKING_SERVICE_BASE_FIELDS`
  and any family's `Constrained.fields`. A field cannot be classified both ways.
- **Coupling.** A family declares `NoBacking` if and only if its
  `service_spec_allowed` is a subset of `NON_BACKING_SERVICE_BASE_FIELDS`, and
  declares `Constrained` if and only if it is not. This is the restored test:
  `NoBacking` is permitted exactly when no base field is a backing reference.

**Grounding.** Every name in `Constrained.fields` is a member of that family's
`service_spec_allowed`, which catches a base field rename that forgets the
declaration.

The classification is a catalog-internal record and never a fingerprint input.
Amendment F below adds the typed table rows to the three dossiers that lack
them, so the catalog's classification has a spec source rather than being the
only place the fact is written down.

### 5. `ProjectionFactory::new` accepts an empty backing set and keeps rejecting an empty target set

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

### 6. Export and backing admission take the stored resource, not a bare reference, and deny every import-owned row

This closes the transit path the first draft deferred. It is the load-bearing
security decision in this record.

A `ResourceRef` is `<ResourceType>/<name>` and carries no mode and no owner, so
a function that receives one **cannot** tell an owner authority from an
import-owned projection. `ProjectionFactory::admits_export_target` receives
exactly that today and is therefore mode-blind by construction, not by
oversight. The fix is to change what admission is given, not to add a
discriminant argument a caller could supply:

```rust
/// Decide whether an export may target this resource.
///
/// Takes the stored envelope so origin is read from committed metadata
/// rather than asserted by the caller. Rejects any row whose
/// `metadata.ownerRef` names a `ResourceImport`.
pub fn admits_export_target(
    &self,
    resource: &ResourceEnvelope,
) -> Result<(), ProviderContractError>;

/// Decide whether an owner Service may name this resource as its backing.
///
/// The allowlist is closed and the membership test is unconditional: an
/// empty allowlist admits nothing. Rejects any row whose
/// `metadata.ownerRef` names a `ResourceImport`.
pub fn admits_backing_ref(
    &self,
    backing: &ResourceEnvelope,
) -> Result<(), ProviderContractError>;
```

**The mechanically checkable invariant, stated without depending on a mode
spelling.** `spec.mode` is spelled `mode` in the security-key and USB families
and `serviceRole` in the audio and telemetry families, with three live authority
values, and those spellings are an open inference rather than a frozen contract.
So the discriminant is not the signal. The signal is ownership, which is
provider-neutral, single-spelled, and already normative in the zone-control
spec and every dossier:

> A resource is import-owned if and only if `metadata.ownerRef` names
> `ResourceImport/<name>`. An import-owned resource is never an export target
> and is never a backing reference.

`ResourceImport` is one of the frozen 19 standard ResourceTypes, so the test is
a type comparison against a name that cannot be redefined.

**What the two denials buy, concretely.** Denying an import-owned export target
stops a child Zone from re-exporting a capability it only holds on lease, so a
grandchild cannot obtain owner authority through a Zone that never had it.
Denying an import-owned backing stops a remote imported projection from being
presented as the local backing of a new authority claim, which would launder a
leased capability into an owned one and defeat the D097 single-authority index.

**The denial is its own typed error.** Both paths return a dedicated variant,
not the generic `ProjectionFactoryInvalid` that a wrong ResourceType returns, so
an operator and an audit record can tell "you named the wrong kind of thing"
from "you named a thing this Zone only holds on lease":

```rust
/// The named resource is owned by a `ResourceImport`, so it is a projection
/// of a capability this Zone holds on lease. It is never an export target
/// and never a backing reference.
ImportOwnedOriginRejected,
```

with the closed diagnostic label `provider-import-owned-origin-rejected`. The
variant is a bare discriminant: it carries no resource name, no owner ref, no
Zone, and no path, matching this enum's stated rule that a reason names the
class and not the value. The remedy is a property of the class, not of the
instance, so it belongs in `docs/reference/error-codes.md` and the CLI rendering
rather than in the payload: export the capability from the Zone that owns its
authority, or, for a backing, name a locally owned resource.

**The residual, named.** This reads committed metadata, so it is exact for any
row Core created correctly. A Core defect that created a projection without the
`ResourceImport` ownerRef would pass. That is `ADR046-zone-control-020`'s stated
contract ("always with `metadata.ownerRef: ResourceImport/<name>`") and gets its
own positive test there; this decision does not restate it as if it were free.

### 7. A family's backing set may not contain its own Service or Binding type

`ProjectionFactory::new` already rejects `service_type == binding_type` with
`ConflictingFields`. It additionally rejects a backing set containing either
`service_type` or `binding_type`.

This forbids a Service chaining onto another Service of the same family as its
"backing" at the declaration layer, complementing decision 6, which forbids it
at the admission layer for any type. Two independent guards, because the
declaration guard is static and the admission guard is per-row, and neither
subsumes the other. D096's allowance of "qualified semantic backend types" in a
backing set is preserved for a *different* family's type; only self-chaining is
closed.

### 8. Factory admission compares the protocol version first, then identity, then fingerprints

For any `serviceType` in the D098 catalog, a Provider descriptor's
`ProjectionFactory` is admitted at Provider install, Nix build, and API
admission only when every declared field equals the factory derived from
`d2b-contracts::v3::semantic_services`. A Provider cannot widen,
narrow, or restate the semantic factory; it may only add its own strict
extension schemas.

**The comparison order is normative, because it decides what the operator is
told.** Fingerprints are the last thing compared, not the first:

1. `projectionProtocolVersion` (decision 9). A mismatch returns
   `ProjectionProtocolVersionMismatch`, label
   `provider-projection-protocol-version-mismatch`. This is version skew and it
   is diagnosed as version skew.
2. `serviceType`, `bindingType`, `allowedBackingRefTypes`,
   `allowedBindingTargetRefTypes`, and `exportability`. A mismatch returns
   `ProjectionFactoryInvalid`.
3. `projectionSchemaFingerprint` and `factoryFingerprint`. A mismatch here, with
   every declared field already equal, means the descriptor's bytes do not hash
   to what it claims. Only then is `DescriptorFingerprintMismatch` the honest
   answer.

**`exportability` is in step 2 because it is declared but not hashed.** Measured:
`factory_fingerprint` binds `serviceType`, `bindingType`,
`allowedBackingRefTypes`, `allowedBindingTargetRefTypes`,
`projectionSchemaFingerprint`, and `projectionProtocolVersion`, and nothing
else. `exportability` is a `ProjectionFactory` field that no fingerprint covers,
so an explicit comparison is the *only* thing that catches a descriptor
advertising `explicit-export` for a capability the catalog declares
`forbidden`, or the reverse. Leaving it out of step 2 would have let a mismatch
through both step 2 and step 3 and been caught nowhere. Step 3 is therefore a
check on hashing integrity, not a backstop for undeclared fields, and the two
steps together are exhaustive over the factory's fields only because step 2
enumerates every unhashed one.

Without step 1 a descriptor built against protocol `1.0` fails at step 3 and
reads as tampering, which is the failure mode round 2 named. With it, the
operator is told the version differs and what to do about it.

**The remedy is addressed to whoever can actually act.** An operator cannot edit
or re-sign a third-party Provider descriptor, so a remedy that tells them to
regenerate one is not actionable. `docs/reference/error-codes.md` carries two
audiences:

- **Operator.** Obtain and install a Provider artifact built for this Core's
  declared projection-protocol version, then retry. The version this Core
  installs is a Core-side constant and may be named in the message; the version
  the descriptor declares is caller-supplied and may not be echoed. If no such
  artifact exists, the Provider is not usable with this Core and the capability
  stays unavailable rather than degrading.
- **Provider author.** Rebuild and re-sign the descriptor against the target
  Core's semantic catalog.

**There is no public command for the second audience today, and that is stated
rather than papered over.** In this repository the regeneration is
`cargo run -p xtask -- gen-semantic-service-schemas` and
`cargo run -p xtask -- gen-provider-packaging`, which write
`docs/reference/schemas/v3/` and `nixos-modules/generated/provider-catalog-shape.nix`.
Those are repository build commands, not an operator surface and not a
third-party surface: `gen-provider-packaging` writes into this tree. The `d2b`
CLI has no provider-packaging verb. So a third-party Provider author has no
supported way to rebuild a descriptor against a published Core protocol version.
That is a real gap, it is out of this record's scope, and it belongs to the
Provider packaging and distribution work rather than being invented here. Until
it is closed, every conformant Provider is built in-tree, and the operator
remedy above resolves to upgrading the d2b artifact set as a whole.

This is what makes decision 1 safe independent of decision 2: a descriptor
cannot reach the empty set for a family whose catalog entry is non-empty,
because every declared field has to match.

### 9. The projection-protocol version is a declared descriptor field, and it bumps to 1.1

The meaning of `allowedBackingRefTypes` changes: a value that was previously
unrepresentable becomes representable, and its reading is fixed as deny-all.
That is a change to the projection protocol, not to any schema's field set.

**The version becomes a first-class field, not only a hash input.** Today
`SEMANTIC_PROJECTION_PROTOCOL_VERSION` reaches the wire only through
`factory_fingerprint`, so a version difference is observable only as a hash that
does not match. That is the whole defect: a hidden input makes skew look like
tampering. D096's factory table gains a row and `ProjectionFactory` gains a
field:

```rust
pub struct ProjectionFactory {
    service_type: ResourceTypeName,
    binding_type: ResourceTypeName,
    projection_protocol_version: SemanticProjectionProtocolVersion, // new, declared
    allowed_backing_ref_types: BTreeSet<ResourceTypeName>,
    allowed_binding_target_ref_types: BTreeSet<BindingTargetType>,
    projection_schema_fingerprint: SchemaFingerprint,
    factory_fingerprint: SchemaFingerprint,
    exportability: Exportability,
}
```

**A 1.0 descriptor has no such field, so the parse rule is load-bearing.** A
required field would make every legacy descriptor fail at *deserialization*,
with a serde error that never reaches decision 8 step 1 and therefore never
produces the typed reason or the operator remedy that step exists to give. That
would defeat the whole point of declaring the version. The field is defaulted,
not required:

```rust
/// The protocol version a descriptor is assumed to declare when the field is
/// absent. A descriptor written before the field existed is a 1.0 descriptor.
/// This is a bounded constant, never caller text.
pub const LEGACY_ABSENT_PROTOCOL_VERSION: &str = "1.0";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Wire {
    // ...
    #[serde(default = "legacy_absent_protocol_version")]
    projection_protocol_version: SemanticProjectionProtocolVersion,
    // ...
}
```

`deny_unknown_fields` stays: the default covers a missing **known** field and
never an unknown one. `SemanticProjectionProtocolVersion` parses a bounded
`<major>.<minor>` grammar with one to three digits per component and a total
length ceiling, so a present-but-malformed value is a parse failure at the type
and never lands on the struct as free text. The three cases are distinct and
each has its own outcome:

| descriptor | deserialization | admission step 1 |
| --- | --- | --- |
| field absent | succeeds, value is `1.0` | `ProjectionProtocolVersionMismatch` with the operator remedy |
| field present, well-formed, not the installed version | succeeds | `ProjectionProtocolVersionMismatch` with the operator remedy |
| field present, malformed or over the ceiling | fails at the bounded type | not reached; the descriptor is malformed, which is a different class from skewed |

**Defaulting is fail-closed, and one invariant keeps it that way.** The default
can never manufacture a match, because the installed version is `1.1` and the
legacy default is `1.0`. That holds only while the two differ, so it is asserted
rather than assumed: `SEMANTIC_PROJECTION_PROTOCOL_VERSION` must never equal
`LEGACY_ABSENT_PROTOCOL_VERSION`. If a future change ever set the installed
version back to `1.0`, every field-absent descriptor would silently be admitted
as current, which is the one way this default could become a hole. The assertion
is the guard.

```rust
// packages/d2b-contracts/src/v3/semantic_services/mod.rs
pub const SEMANTIC_PROJECTION_PROTOCOL_VERSION: &str = "1.1"; // was "1.0"
```

**Why minor and not major, stated precisely about the right stage.** An old
descriptor's non-empty set means exactly what it meant before, every field keeps
its type and name, and with the default rule above a 1.0 descriptor still
deserializes. It is then **rejected at admission**, with a typed reason and a
remedy. Deserialization and admission are different stages and an earlier
revision of this record conflated them, claiming both that the field was
required and that nothing which parsed before fails to parse now; those cannot
both be true. What the minor bump claims is the narrower and correct thing: no
existing value changes meaning, and no descriptor becomes unparseable. Being
refused by an updated Core is the intended behaviour, not a compatibility break,
because that refusal is exactly what tells an operator to install a matching
artifact.

**Why not the base schema version.** `SEMANTIC_BASE_SCHEMA_MAJOR` and
`SEMANTIC_BASE_SCHEMA_MINOR` stay at `1` and `0`. No base or projection field
set changes, so moving them would move all sixteen base-layer fingerprints and
all four projection-schema fingerprints for a change that touched none of those
surfaces. The blast radius must match what actually moved.

**Measured: which fingerprints move, and which change causes each move.** Two
independent changes touch the `factory_fingerprint` preimage, and they are not
the same set of families. Decision 1 moves security key's
`allowedBackingRefTypes` from `null` to `[]` and touches no other family.
Decision 9 moves `projectionProtocolVersion` from `1.0` to `1.1` and touches
all four. Decomposed for security key by re-deriving each combination:

| security key preimage | `factoryFingerprint` |
| --- | --- |
| `null`, protocol `1.0` (committed today) | `sha256:444b0f9ba1d9997a392314a5aa9b49ca180a2e8c5d7541d1eb1846d2b3c460dc` |
| `[]`, protocol `1.0` (decision 1 alone) | `sha256:57f1d41aa8740a8f7012f42a1d06686454f33d4b4265cabee358b4a9519ead6e` |
| `null`, protocol `1.1` (decision 9 alone) | `sha256:a8c4d5f86894618950f08031803b246dc7e0ec2371b121c0f5e7a89393e3933b` |
| `[]`, protocol `1.1` (both, the target) | `sha256:8101ab8d17bac0cc1f57f957223fa531a3a5d231f93bc8e56e540dc499830027` |

So security key's fingerprint moves for **two** reasons and the other three move
for **one**. An earlier revision of this record claimed "no family was `None`",
which was wrong and contradicted its own table: security key **is** `None` in
the committed tree, which is exactly why its published
`x-d2b-allowed-backing-ref-types` is `null`. The corrected claim is narrower and
is about the Rust type rather than any family's value: after decision 1 no
family's declared value is absent, so **removing the `Option` wrapper from the
declaration type moves nothing**, because the only value that ever serialized as
`null` has already been replaced by `[]` under decision 1. The `null` to `[]`
move belongs to decision 1 and is counted there; decision 3's type change has no
preimage effect of its own.

Promoting the protocol version to a declared descriptor field likewise moves
nothing, because the hashed declaration already carried
`"projectionProtocolVersion"`. Adding the struct and wire field changes the
descriptor's shape, not its hash input. Adding `exportability` to decision 8's
step-2 comparison also moves nothing, because `exportability` is not in the
preimage at all; that is precisely why it needs an explicit comparison.

**What the bump buys.** The protocol version is an input to
`factory_fingerprint` and to nothing else, so bumping it moves **all four**
families' factory fingerprints, not just security key. That is the point. A
stale descriptor of any family now mismatches an updated Core, the mismatch is
caught at decision 8's step 1 by declared-field comparison before any hash is
examined, and the operator is told the protocol version differs rather than
being shown two hashes. Leaving the other three unchanged would have made
security key's move look like tampering and the others look fine.

**The generated artifact must publish every declared field, not a subset.**
Measured: `packages/xtask/src/semantic_service_schemas.rs` emits five extension
keys on each `*_projection_spec.schema.json` today, and `exportability` and
`allowedBindingTargetRefTypes` are not among them. Nix reads these artifacts to
validate a Provider descriptor at build time, so any declared field the artifact
omits is a field Nix cannot compare without recomputing a fingerprint, and
`exportability` is not in any fingerprint at all. That is the same blind spot
decision 8 step 2 closes at admission, left open one layer down. The generator
publishes all seven:

| key | source | in `factoryFingerprint`? |
| --- | --- | --- |
| `x-d2b-resource-type` | `serviceType` | yes |
| `x-d2b-binding-resource-type` | `bindingType` | yes |
| `x-d2b-projection-protocol-version` | `projectionProtocolVersion` | yes |
| `x-d2b-allowed-backing-ref-types` | `allowedBackingRefTypes` | yes |
| `x-d2b-allowed-binding-target-ref-types` | `allowedBindingTargetRefTypes` | yes |
| `x-d2b-exportability` | `exportability` | **no** |
| `x-d2b-projection-schema-fingerprint`, `x-d2b-factory-fingerprint` | both fingerprints | n/a |

The `exportability` row is the one that matters: it is the only declared field a
fingerprint comparison can never catch, at either layer. Publishing it is what
lets Nix reject a descriptor advertising `explicit-export` for a capability the
catalog declares `forbidden`.

**Regeneration and migration.** Regeneration is
`run_xtask gen-semantic-service-schemas`, gated by the enforcing `make
test-drift` lane; all four `*_projection_spec.schema.json` artifacts are
rewritten in one commit with the seven keys above. **There is no migration,
because there is no runtime consumer.** No Provider descriptor is signed, no
`ResourceExport` or `ResourceImport` type exists yet
(`packages/d2b-contracts/src/v3/` has no `resource_export.rs` or
`resource_import.rs`; they are `ADR046-zone-control-019`'s destination), and
every work item in this program carries `dataMigration: None; full reset`. The
versioning is therefore explicit rather than load-bearing today, and it is
recorded now so that the first signed descriptor is minted against a declared
protocol version instead of an implicit one.

**Adding artifact keys moves no fingerprint, and the reason is structural.**
Measured from the two derivation functions: `layer_fingerprint` takes only the
schema id, schema version, and the allowed and required field-name lists;
`factory_fingerprint` takes only the two ResourceTypes, both reference sets, the
projection schema fingerprint, and the protocol version. Neither reads the
emitted file. The artifact is an output of the catalog, never an input to it, so
no `x-d2b-*` key can feed back into a preimage. The four values pinned below are
unchanged by this decision's artifact work.

**The pinned values**, measured by re-deriving `factory_fingerprint` for every
family at `projectionProtocolVersion` `"1.1"`, with security key's backing set
at `[]` and the other three at their existing sets:

| Family | `factoryFingerprint` before | after | causes | `projectionSchemaFingerprint` |
| --- | --- | --- | --- | --- |
| audio | `sha256:80ef3d08378a61ac924944564efa136b0cfba314d1e48567680d16cc75ac4b38` | `sha256:67352424b92e8da62d2c39f664d9028c85fdede9c38f6a9e3e1423d3009a33a6` | decision 9 | unchanged |
| security key | `sha256:444b0f9ba1d9997a392314a5aa9b49ca180a2e8c5d7541d1eb1846d2b3c460dc` | `sha256:8101ab8d17bac0cc1f57f957223fa531a3a5d231f93bc8e56e540dc499830027` | decisions 1 and 9 | unchanged |
| telemetry | `sha256:de3ef22c8138fbe84c7905fa029d5b3c3f5bed364063b35affe8e0638ca26185` | `sha256:6e6c64a3e39554c76f7d745758a8faf2b81135556dbcd82ea085a073c7334218` | decision 9 | unchanged |
| USB | `sha256:72b5cafbd2409d187b523b1d6076094f8d6246d0a5714240d1b7bac775ed7b45` | `sha256:f73ce4a2ef7d6c21bfdf4f14da51be28d8ee7b53ecf85751f87c29df9a8d9115` | decision 9 | unchanged |

Security key's published `x-d2b-allowed-backing-ref-types` additionally moves
from `null` to `[]`, which is decision 1's visible half. Every
`projectionSchemaFingerprint` is unchanged because no projection
field set moves. The four generated artifacts are the only committed files that
pin any old value. The dossiers' Nix examples use placeholder tokens such as
`sha256:<security-key-projection-factory>`, not literals, so no prose pins one.

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
not cosmetic. Before decision 6, `ProjectionFactory::admits_export_target` took
a bare `ResourceRef` and so was type-only and mode-blind: a resource of the
family's own Service type was an accepted export target regardless of whether it
was an authority or an import-owned projection. A "skip when empty" backing
check plus a mode-blind export check is a path from a consumer Zone's imported
projection to a re-exported authority claim, which breaks Zone isolation rather
than degrading it.

Four guards, in order of strength. Decision 6 changes what admission is given,
so both the export target and the backing are read from committed metadata and
every import-owned row is denied at both ends of the path, with its own typed
variant so the denial is legible in an audit record. Decision 6 also gives
the membership test a typed method whose test is unconditional, so the branch
above has nowhere to live. Decision 7 removes the family's own Service type from
every backing set at declaration time, so the chain has no first link even if a
row's ownership were somehow unreadable. Decisions 3 and 4 stop a later author
from resolving the oddity by inventing a non-empty set: the type set and its
base fields are one value, and `NoBacking` is permitted only while every base
field stays in the classified non-reference list. The negative tests named below
assert that the security-key factory rejects `Device/x`, `Endpoint/x`, and
`security-key.d2bus.org.SecurityKeyService/x`, and that an import-owned row of
an otherwise-admissible type is denied for both export and backing with
`ImportOwnedOriginRejected` specifically.

**The second specific failure: a silent fingerprint move.** All four
`factoryFingerprint` values change. Any Provider descriptor, fixture, or example
that pins an old value will fail import expectation matching after this lands.
That failure is fail-closed and correct, and decision 9 makes it read as version
skew rather than tamper: the projection-protocol version moves from `1.0` to
`1.1`, so the mismatch has a declared cause. The guard is that both values are
pinned in both directions in decision 9, the sole committed pins are the four
generated artifacts, and `make test-drift` fails until they are regenerated. The
mechanically checkable condition is that after regeneration the four artifacts
carry `67352424...`, `8101ab8d...`, `6e6c64a3...`, and `f73ce4a2...`, and the
security-key artifact additionally carries `[]` for
`x-d2b-allowed-backing-ref-types`.

**What this makes easy.** All four families return `Ok` from
`projection_factory()`, so `ADR046-zone-control-019` and `-020` have one code
path with no family special case, and `ADR046-device-004` can publish this
family's factory in its signed descriptor. The audit's blocking entries at
implementation-debt 13.3 and 19.8 are discharged by an amendment, which is what
they always said the remedy was.

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

Decision 6 adds its own cost, stated plainly. Export and backing admission now
require the stored envelope rather than a reference, so a caller that holds only
a `ResourceRef` must read the row first. That is one extra store read on a path
that already reads the row to check `serviceType`, generation, and readiness, so
the cost is real but small, and it is the price of making the origin
unforgeable by the caller rather than asserted by it.

**What this forecloses.** It forecloses reading `allowedBackingRefTypes` as an
optional constraint anywhere in the tree. Any future family that wants "no
constraint" has no spelling for it and must enumerate. It also forecloses
re-export of a leased capability: a Zone may consume what it imports and may not
pass it on as authority. If a later decision wants grandchild chaining, it must
introduce an explicit delegated-export construct with its own capability
ceiling, rather than acquiring the behaviour by omission.

**Residual risk this decision does not remove.** Decision 8 makes Core compare a
Provider's advertised factory against the catalog. Nothing in this decision
proves that the catalog itself was derived from the dossier rather than from a
plausible reading of it; that is what the amendment in the next section is for,
and it is why the amendment is normative dossier text and not an ADR-only claim.
Decision 6's ownership test is exact for correctly created rows and does not
detect a projection that Core created without its `ResourceImport` ownerRef;
that positive obligation is `ADR046-zone-control-020`'s and is named in decision
6 rather than assumed away.

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

**Close the transit path by passing a mode discriminant to admission.** The
smaller-looking fix for decision 6: keep the `&ResourceRef` argument and add a
`ServiceMode` parameter the caller supplies. Rejected outright. A
caller-supplied discriminant is exactly the shape the repository already
learned to refuse at `ZoneRegistrar`, where a caller-supplied `subject_ref` is
"how a component would name itself something it is not". A controller that has
already made the mistake of treating a projection as an authority will pass
`ServiceMode::Authority` with complete sincerity. Origin has to be read from
committed state by the code that decides, which is why decision 6 takes the
envelope.

**Close it by freezing the per-family mode discriminant instead of ownership.**
Rejected on evidence. The discriminant is spelled `mode` in two families and
`serviceRole` in the other two, with three live authority values
(`mode: "authority"`, `serviceRole: "authority"`, `serviceRole: "owner"`), and
implementation-debt 13.4 records that divergence as an open inference rather
than a frozen contract. Making a security boundary depend on freezing four
spellings would both enlarge this record into a catalog-wide freeze and rest the
boundary on the least settled part of the contract. `metadata.ownerRef` has one
spelling, is already normative for projections in every dossier, and names a
type from the frozen 19.

**Bump the base schema version rather than the projection-protocol version.**
Rejected. No base or projection field set changes, so a base-version bump would
move sixteen base-layer fingerprints and four projection-schema fingerprints for
a change that touched none of them. The version that moves must be the one whose
contract moved.

**Retain `Undetermined` and `BackingRefTypesUndetermined` for a future family
whose dossier is silent.** This record argued for retention in its first
revision and reverses that here, because the argument was wrong in a way worth
recording. A silent dossier is not a state the catalog should be able to
publish; it is a reason not to add the family yet. Keeping the state put a
fallible path on every caller of `projection_factory()` in order to serve a
family that does not exist, and left a runtime error as the guard for something
a build refusal handles completely. The correct fail-closed behaviour for an
unstated backing set is that the catalog does not compile with that family in
it, which is strictly earlier and strictly louder than a typed error.

**Keep the protocol version only inside `factoryFingerprint`.** This is what the
record specified before this revision, on the reasoning that binding the version
into the hash was sufficient to make skew detectable. It is not: detectable and
diagnosable are different properties. A hash that does not match tells an
operator that two byte strings differ and nothing about why, which is
indistinguishable from tampering and points at no remedy. Rejected in favour of
a declared field compared before any hash, which costs one descriptor field and
buys an accurate diagnosis with an actionable remedy.

**Classify backing fields by a `*Ref` suffix heuristic instead of an explicit
list.** Rejected on the field names themselves. `providerRef`, `serviceRef`,
`authorityRef`, `producerRef`, and `guestRef` are all references and none is a
backing, so the heuristic misclassifies five of the names actually in play. It
also silently reclassifies any future field the moment someone renames it. The
explicit list costs thirteen strings and one totality assertion, and it fails
loudly when a new field appears rather than guessing.

**Make `projectionProtocolVersion` a required field.** This is what the record
specified before this revision, and it is self-defeating. A 1.0 descriptor has
no such field, so a required field means every legacy descriptor dies in serde
with an untyped deserialization error, never reaching decision 8 step 1 and
never producing the typed reason or the operator remedy that step exists to
give. The record simultaneously claimed the field was required and that nothing
which parsed before fails to parse now; both cannot hold. Rejected in favour of
a defaulted field whose default is a bounded constant.

**Make it `Option<SemanticProjectionProtocolVersion>` and treat `None` as
"unknown".** Rejected. It reintroduces the three-state shape decision 3 spent a
round removing, pushes a `None` case into every comparison site, and gains
nothing: a descriptor without the field is not a descriptor of unknown protocol,
it is a 1.0 descriptor, and saying so exactly is both truer and simpler than
saying it might be anything.

**Publish only the fingerprints in the generated artifact and let Nix
recompute.** Rejected twice over. Nix would have to reimplement canonical JSON
and the domain-separated digest to check anything, which duplicates a security
primitive in a second language; and it would still not catch an `exportability`
mismatch, because no fingerprint binds that field. Publishing the declared
fields costs seven keys per artifact and makes the comparison a string equality
Nix can actually perform.

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
> reference to another `SecurityKeyService`, and rejects any backing whose
> `metadata.ownerRef` names a `ResourceImport`. The physical `Device` and the
> relay `Endpoint` are implementation children named only in
> `spec.provider.settings`; the physical Device is backing inventory, not the
> service authority. The guarantee that an owner Service is backed by one real
> physical key is carried by the mandatory
> `(Host, physical-usb-backing, opaqueKeyDigest)` claim admitted before any
> DeviceGrant or hidraw open, by the D097 `AuthorityDescriptor`, and by the
> signed Provider extension schema, not by the projection factory.
>
> A projection `SecurityKeyService` is never an export target. Its
> `metadata.ownerRef` names its `ResourceImport`, so a consumer Zone cannot
> re-export the key to a grandchild Zone; the owner Zone remains the only
> exporter of the one authority Service.
>
> Provider install, Nix admission, API admission, export, and import fail closed
> if the factory, its signature, the Service/Binding pair, the allowed reference
> sets, or either fingerprint differs from the catalog-derived factory for this
> `serviceType`. The `factoryFingerprint` binds the semantic
> projection-protocol version, so a descriptor minted under an earlier protocol
> version mismatches by declared version skew rather than by tampering.

### B. `docs/specs/ADR-046-provider-model-and-packaging.md`

Replace the `allowedBackingRefTypes` row of the D096 factory table and add one
row above it:

> | `projectionProtocolVersion` | The semantic projection-protocol version this factory was minted against, declared explicitly. Compared before any other field, so a descriptor from an earlier protocol is diagnosed as version skew rather than as a fingerprint mismatch. Also bound into `factoryFingerprint`. |
> | `allowedBackingRefTypes` | Closed set of same-Zone `Device`, `Endpoint`, or qualified semantic backend types the owner Service may reference **in its provider-neutral base**. Empty when the family's base declares no backing-reference field. The empty set denies every backing reference and is never read as unconstrained. It may not contain the factory's own `serviceType` or `bindingType`. |

### C. `docs/specs/ADR-046-resources-zone-control.md`

Add the same `projectionProtocolVersion` row and replace the
`allowedBackingRefTypes` row of the 8A.1.1 factory table with the same text as
amendment B; append to the paragraph beginning "Provider install, Nix build, and
API admission verify this metadata":

> For a `serviceType` in the D098 semantic catalog, the advertised factory is
> admitted only when every declared field equals the catalog-derived factory.
> Comparison order is normative: `projectionProtocolVersion` first; then
> `serviceType`, `bindingType`, both reference sets, and `exportability`; then
> both fingerprints. `exportability` is compared explicitly because no
> fingerprint binds it. A descriptor minted under an earlier protocol version is
> therefore rejected as declared version skew, whose remedy is to install a
> Provider artifact built for the Core protocol in use, and never as a
> fingerprint mismatch that reads as tampering. A Provider adds strict extension
> schemas; it never restates, widens, or narrows the semantic factory.

and add to 8A.2 (`ResourceExport`), in the `resourceRef` row's notes and as a
normative sentence in 8A.2.1:

> `resourceRef` names a **local owner authority** Service. A resource whose
> `metadata.ownerRef` names a `ResourceImport` is an import-owned projection and
> is rejected as an export target: a Zone may consume what it imports and may
> not re-export it, so a grandchild Zone never acquires owner authority through
> an intermediary that held only a lease. Admission reads the stored resource;
> a bare ResourceRef is not sufficient evidence of origin. The same rule applies
> to any backing reference an owner Service names: an import-owned resource is
> never a backing.

### D. `docs/specs/ADR-046-decision-register.md`

No amendment required, and this is deliberate. The D096 row already says
"allowed backing/target refs" without stating a lower bound, so it is not
contradicted, and it already says `ResourceExport.resourceRef` "names only the
local owner Service", which amendment C makes mechanically checkable rather than
changing. The normative readings land in the two specs and the dossiers, which
are the documents an implementer reads.

### E. `docs/reference/error-codes.md`

Two closed variants gain rows, because both remedies are class-level and neither
may be carried in an error payload:

> | `provider-import-owned-origin-rejected` | The named resource is owned by a `ResourceImport`, so this Zone holds it on lease rather than owning its authority. Export the capability from the Zone that owns the authority Service; for a backing reference, name a locally owned resource. |
> | `provider-projection-protocol-version-mismatch` | The Provider artifact was built for a different semantic projection-protocol version than the one this Core installs. Install a Provider artifact built for this Core's protocol version and retry. If none is available, the Provider is not usable with this Core; the capability stays unavailable. Provider authors rebuild and re-sign the descriptor against the target Core's semantic catalog. |

The second message may name the version **this Core installs**, which is a
Core-side constant. It must not echo the version the descriptor declares, which
is caller-supplied. Neither message tells an operator to regenerate or re-sign a
descriptor: a descriptor is signed by its author, and an operator can do
neither.

### F. Typed base-field rows in three dossiers

Measured: of the four families' backing fields, only telemetry's is stated in a
typed table (`ADR-046-provider-observability-otel.md`, `| ingestEndpointRefs |
[ResourceRef] | authority only | ... |`). Audio's `implementationEndpointRefs`
and USBIP's `backingDeviceRef` appear only in YAML examples and prose, and the
security-key Service base has no backing field to state.

`ADR-046-provider-audio-pipewire.md` and `ADR-046-provider-device-usbip.md`
therefore gain a typed `Type` column for their Service base spec fields, matching
telemetry's shape, and `ADR-046-provider-device-security-key.md` gains the same
for its four base fields with an explicit line that none is a ResourceRef to a
backing. This is what gives decision 4's classification a spec source instead of
leaving the catalog as the only place the fact is written down.

### G. `docs/specs/ADR-046-nix-configuration.md`

The Nix build-time descriptor validation gains the two fields it could not
compare before. Add to the Provider descriptor validation section:

> Nix compares a Provider descriptor's projection factory against the committed
> `docs/reference/schemas/v3/<namespace>_projection_spec.schema.json` artifact
> for the declared `serviceType`. Every published `x-d2b-*` key is compared, not
> only the fingerprints: `x-d2b-resource-type`,
> `x-d2b-binding-resource-type`, `x-d2b-projection-protocol-version`,
> `x-d2b-allowed-backing-ref-types`, `x-d2b-allowed-binding-target-ref-types`,
> `x-d2b-exportability`, `x-d2b-projection-schema-fingerprint`, and
> `x-d2b-factory-fingerprint`. `x-d2b-exportability` is compared explicitly
> because no fingerprint binds it, so a descriptor advertising
> `explicit-export` for a capability the catalog declares `forbidden` would
> otherwise match on every hash and pass. A descriptor omitting
> `projectionProtocolVersion` is read as declaring the legacy version and is
> rejected against any newer installed version, with the operator remedy of
> installing a Provider artifact built for the Core protocol in use.

### H. `specs/001-adr046-d2b3-completion/implementation-debt.md`

Both cited sections exist at this branch's base, verified against the committed
blob rather than the working tree:

```text
$ git show origin/v3:specs/001-adr046-d2b3-completion/implementation-debt.md \
    | grep -n '^### 13.3\|^### 19.8'
942:### 13.3 Security-key cannot construct a signed projection factory at all
2019:### 19.8 The security-key semantic projection is not invented
```

The blob hashes `19e969d0a5cfe4091c9a25172c449342cf668e01a91978933ed1339b7f16e6cd`,
identical to the worktree copy, and this branch does not modify that file:
`git diff --name-only origin/v3..HEAD` lists only the three `docs/` files this
record touches. `origin/v3` is `2c665603`, which is the merge commit for PR #368
from `adr046-w5`, so the W5 work is merged into `v3` and is not an unmerged
branch. Both citations stand. Each section gains a closing line naming this ADR
and the amendment that discharges it. Ownership of that edit belongs to whichever
wave lands amendments A to G, not to this record, because the debt register
describes the tree and the tree does not change until then.

## Tests required to consume this decision

Each is mechanically checkable and named so a wave's stopping condition can cite
it.

In `packages/d2b-contracts/src/v3/semantic_services/security_key.rs`, replacing
`the_backing_ref_set_is_undetermined_and_fails_closed`:

- `the_backing_declaration_is_no_backing_and_the_factory_is_constructible` - the
  declaration is `NoBacking`, the resolved set is empty, and
  `projection_factory()` returns `Ok`.
- `the_empty_backing_set_admits_no_backing_reference` - the constructed factory
  rejects `Device/yubikey-primary`, `Endpoint/yubikey-primary-ctaphid-relay`,
  and `security-key.d2bus.org.SecurityKeyService/yubikey-primary`. Negative
  control: the USB factory admits a locally owned `Device/work-token`, so the
  assertion is capable of failing.

In `packages/d2b-contracts/src/v3/semantic_services/mod.rs`, over the whole
catalog:

- `an_empty_constrained_declaration_does_not_compile` - decision 3's static
  guarantee, as a `compile_fail` doctest rather than a runtime assertion.
  `Constrained { types: NonEmpty { first: ..., rest: &[] } }` is the only
  spelling, so a declaration attempting an empty list fails to build. This
  replaces the earlier `a_backing_declaration_is_well_formed_for_every_family`
  runtime check, which is deleted along with the state it validated. The repo
  already treats `compile_fail` doctests as capability seals and runs them
  outside nextest, so this obligation carries that companion run.
- `every_declared_backing_field_is_a_service_base_field` - decision 4's
  grounding invariant, which stays a runtime assertion because it relates two
  declarations and cannot be encoded in a type.
- `no_family_admits_its_own_service_or_binding_type_as_backing` - decision 7.
- `every_family_yields_a_projection_factory` - all four return `Ok`, replacing
  the current three-of-four state.
- `only_undetermined_fails_closed_as_undetermined` is **deleted**, along with
  the state and the error variant it exercised. Its replacement is
  `no_backing_declaration_state_is_undetermined`, a compile-visible assertion
  that `SemanticBackingDeclaration` has exactly the two variants and that
  `SemanticContractError` no longer carries `BackingRefTypesUndetermined`, so
  reintroducing a runtime "we do not know" state fails a test rather than
  passing quietly.
- `the_factory_fingerprint_binds_the_projection_protocol_version` - re-deriving
  a family's factory fingerprint with a different protocol-version string
  produces a different value, so decision 9's version is load-bearing in the
  hash as well as declared on the descriptor.
- `promoting_the_protocol_version_to_a_declared_field_moves_no_fingerprint` -
  the four values pinned in decision 9 are reproduced from the public
  accessors, so the claim that the descriptor gained a field without moving a
  hash is asserted rather than asserted-about.

For decision 4's restored coupling, over the whole catalog:

- `every_service_base_field_is_classified_exactly_once` - totality and
  disjointness. Each family's `service_spec_allowed` is a subset of
  `NON_BACKING_SERVICE_BASE_FIELDS` united with its own declared backing
  fields, and the two sets never intersect. **Two planted-violation controls,
  one per property, because one control cannot exercise both.** Totality: a
  local copy of a family's allowed set gains an unclassified field name and the
  assertion must reject it, so the scan cannot pass vacuously on an empty
  universe. Disjointness: a local copy of `NON_BACKING_SERVICE_BASE_FIELDS`
  gains `backingDeviceRef`, a name that is genuinely a declared backing field of
  the USB family, and the assertion must reject that too. Without the second
  control a disjointness check that never fires would look identical to one that
  passes, and misclassifying a real backing field as a non-reference is exactly
  the mistake that would let decision 4's coupling admit `NoBacking` for a
  family that has a backing.
- `no_backing_is_declared_exactly_when_no_base_field_is_a_backing_reference` -
  the coupling itself, both directions, over all four families. Positive
  controls: security key is `NoBacking` and every one of its four base fields is
  in the non-backing list; audio, telemetry, and USB are `Constrained` and each
  has exactly one base field outside it. Negative controls: a local declaration
  pairing security key's field set with `Constrained` is rejected, and one
  pairing USB's field set with `NoBacking` is rejected.
- `no_backing_classification_uses_a_ref_suffix_heuristic` - `providerRef`,
  `serviceRef`, and `authorityRef` appear in `NON_BACKING_SERVICE_BASE_FIELDS`
  or in no family's backing fields, proving the classification is explicit and
  a `*Ref` suffix rule would have misclassified them.

In `packages/d2b-contracts/src/v3/provider.rs`:

- `an_empty_backing_set_is_accepted_and_an_empty_target_set_is_not` - decision
  5's asymmetry, both halves.
- `a_factory_may_not_name_its_own_service_or_binding_type_as_backing` -
  decision 7 at the constructor.
- `an_import_owned_resource_is_never_an_export_target` - decision 6. Build one
  envelope of the family's Service type with `metadata.ownerRef` naming
  `ResourceImport/yubikey-primary` and one with no owner; assert the first is
  rejected with `ImportOwnedOriginRejected` specifically, not merely rejected,
  and the second admitted, for every family.
- `an_import_owned_resource_is_never_a_backing` - decision 6, same construction
  against `admits_backing_ref`, using a `Device` envelope for USB and an
  `Endpoint` envelope for audio and telemetry so the type is otherwise
  admissible and ownership is the only thing under test. Asserts
  `ImportOwnedOriginRejected` and not `ProjectionFactoryInvalid`, so the two
  denial classes cannot collapse into one.
- `a_wrong_type_and_an_import_owned_row_return_different_errors` - the
  discrimination test for decision 6's typed error. A `Volume` envelope returns
  `ProjectionFactoryInvalid`; an import-owned row of the right type returns
  `ImportOwnedOriginRejected`.
- `an_error_label_carries_no_caller_supplied_value` - both new variants render
  to their fixed labels and their `Debug` and `Display` output contains no
  resource name, owner ref, Zone, version string, or path.
- `factory_admission_reports_version_skew_before_fingerprint_mismatch` -
  decision 8's ordering. A descriptor equal in every field but declaring
  protocol `1.0` returns `ProjectionProtocolVersionMismatch`, not
  `DescriptorFingerprintMismatch`, even though its fingerprint also differs.
  This is the test that would have failed under the round-2 shape.
- `an_exportability_mismatch_is_caught_although_no_fingerprint_binds_it` -
  decision 8 step 2. A descriptor identical to the catalog factory in every
  hashed field but declaring a different `exportability` has a **matching**
  `factoryFingerprint`, so step 3 cannot see it; admission must still return
  `ProjectionFactoryInvalid`. The test asserts the fingerprints are equal before
  asserting the rejection, so it proves step 2 is doing the work rather than
  step 3 catching it incidentally.
- Round-trip: a factory with `allowedBackingRefTypes: []` and
  `projectionProtocolVersion: "1.1"` serializes and deserializes without loss;
  the deserializer still rejects an empty target set and now also rejects an
  absent `projectionProtocolVersion`.

At the zone-control export admission layer, in
`packages/d2b-core-controller/src/export_import.rs` and its test module, owned
by `ADR046-zone-control-019`:

- `an_export_of_an_import_owned_projection_is_denied` - the caller-facing
  negative test. A consumer Zone holding a projection Service created by its
  `ResourceImport` attempts to declare a `ResourceExport` naming it; admission
  denies it with `ImportOwnedOriginRejected`, so a grandchild Zone cannot obtain
  owner authority through a Zone that held only a lease.
- `an_owner_service_may_not_name_an_import_owned_backing` - an owner Service
  naming an import-owned resource of an allowed backing type is denied, so a
  leased capability cannot be laundered into a local authority claim.
- `an_empty_backing_set_denies_a_specified_backing` - the caller/export negative
  test, distinct from the unit test above because it runs through admission
  rather than the factory directly. An owner `SecurityKeyService` that names any
  backing reference at all is denied by an empty-set factory. Negative control
  in the same test: the same call shape against the USB factory with a locally
  owned `Device/work-token` is admitted, so an unconditionally denying admission
  path cannot pass this test vacuously.
- `an_export_of_a_locally_owned_authority_is_admitted` - the positive control,
  so the three denials above are not satisfied by an admission path that denies
  everything.

In `packages/d2b-provider-toolkit/tests/malicious_provider.rs`:

- A descriptor advertising a security-key factory with a non-empty backing set,
  or with the catalog's set but a recomputed fingerprint, is rejected by
  decision 8 at step 2 or 3.
- A descriptor minted under projection-protocol version `1.0` is rejected at
  step 1 with `ProjectionProtocolVersionMismatch`, and the assertion names that
  variant rather than accepting any error, so a regression that reorders the
  comparison is caught.

For decision 9's legacy parse rule, in
`packages/d2b-contracts/src/v3/provider.rs`:

- `a_descriptor_without_the_version_field_parses_as_the_legacy_version` - a
  descriptor JSON omitting `projectionProtocolVersion` deserializes, and the
  parsed value equals `LEGACY_ABSENT_PROTOCOL_VERSION`. This is the test that
  proves a 1.0 descriptor reaches admission rather than dying in serde.
- `a_legacy_descriptor_is_refused_with_the_typed_version_reason` - that same
  descriptor, passed to admission, returns `ProjectionProtocolVersionMismatch`
  and not a deserialization failure, a `DescriptorFingerprintMismatch`, or a
  generic `ProjectionFactoryInvalid`. The pair of tests together is the
  obligation; either alone would pass while the path stayed broken.
- `a_malformed_protocol_version_is_a_parse_failure_not_a_mismatch` - explicit
  values `""`, `"1"`, `"1.x"`, `"1.2.3"`, `"01.0"`, a 64-byte digit run, and a
  non-string JSON value each fail at the bounded type. Malformed and skewed are
  different classes and must not collapse into one.
- `a_well_formed_unsupported_version_is_a_mismatch_not_a_parse_failure` -
  `"2.0"` and `"0.9"` parse and are then refused at step 1, the mirror of the
  test above.
- `the_legacy_default_can_never_equal_the_installed_version` - asserts
  `SEMANTIC_PROJECTION_PROTOCOL_VERSION != LEGACY_ABSENT_PROTOCOL_VERSION`, so
  the one way defaulting could become a hole fails a test rather than shipping.
- `an_unknown_field_is_still_rejected` - the default covers a missing known
  field only; `deny_unknown_fields` still refuses an unknown one, so the
  defaulting change did not loosen the deserializer generally.

Artifact and drift:

- `run_xtask gen-semantic-service-schemas` regenerates all four
  `docs/reference/schemas/v3/*_projection_spec.schema.json` artifacts with the
  `factoryFingerprint` values pinned in decision 9, each publishing the seven
  extension keys in decision 9's table, and the security-key artifact
  additionally with `"x-d2b-allowed-backing-ref-types": []`. Enforced by
  `make test-drift`.
- `every_declared_factory_field_is_published_by_the_generator` - in
  `packages/xtask/src/semantic_service_schemas.rs`, over all four families:
  every field of `ProjectionFactory` other than the two fingerprints appears as
  an `x-d2b-*` key with the matching value. This is the test that makes the
  artifact's completeness structural rather than a list someone remembered to
  extend, and it is the same exhaustiveness obligation decision 8 step 2 carries
  one layer up.
- `an_artifact_missing_exportability_is_rejected` - a planted control removing
  `x-d2b-exportability` from a rendered artifact fails the assertion above, so
  the completeness check cannot pass vacuously.

Nix admission, owned by `ADR046-nix-configuration` and
`ADR046-zone-control-019`:

- `a_provider_descriptor_mismatching_any_published_field_is_rejected_at_eval` -
  a nix-unit case under `tests/unit/nix/cases/` that compares a Provider
  descriptor against the committed projection artifact and rejects a mismatch in
  each of the seven published keys, one planted mismatch per key. The
  `x-d2b-exportability` and `x-d2b-allowed-binding-target-ref-types` cases are
  the two this decision adds; without the artifact keys they were not
  expressible at eval time at all.
- `a_descriptor_with_a_matching_fingerprint_but_a_wrong_exportability_is_rejected` -
  the Nix-layer twin of the Rust step-2 test. The fingerprints match because no
  fingerprint binds `exportability`, and eval must still fail. This is the case
  that proves publishing the key bought something.

API surface, for decision 6's mint-surface widening and the new error variants:

- `admits_backing_ref` is a new public method on `ProjectionFactory`, a type
  already carried in `packages/d2b-bus/tests/approved-capability-api.txt`;
  `admits_export_target`'s signature changes; `ProjectionFactory` gains a
  `projection_protocol_version` accessor; `SemanticProjectionProtocolVersion`
  and `LEGACY_ABSENT_PROTOCOL_VERSION` are new public items; and
  `ProviderContractError` gains `ImportOwnedOriginRejected` and
  `ProjectionProtocolVersionMismatch` while `SemanticContractError` loses
  `BackingRefTypesUndetermined`. Every one of those is a two-way census entry,
  and the removal is as census-visible as the additions. The implementing wave
  runs `make api-surface-pin` to regenerate `tests/golden/api-surface/` and the
  approved capability list, and `make test-rust-api-surface` to prove the
  regenerated census matches. The regeneration is a deliberate trust-boundary
  change whose stated reason is this record; the census must not be pinned
  without citing it.

Lanes that must be green for the implementing wave: `make test-rust`,
`make test-rust-api-surface`, `make test-drift`, `make test-fixture-contracts`,
`make test-nix-unit`, `make check-tier0`, `make test-policy`.

## Invariants this decision creates

1. `allowedBackingRefTypes` is a closed allowlist and the empty allowlist admits
   nothing. No consumer branches on its emptiness.
2. A family's backing declaration is one value with two states. `NoBacking`
   carries nothing; `Constrained` carries a statically non-empty type list and a
   statically non-empty field list, so an empty `Constrained` has no spelling
   and needs no validation.
3. Every name in a family's `Constrained.fields` is a member of that family's
   `service_spec_allowed`.
4. The declared backing fields are never a fingerprint input; only the resolved
   type set D096 names reaches `factory_fingerprint`.
5. A factory's backing set never contains its own `serviceType` or
   `bindingType`.
6. `allowed_binding_target_ref_types` is always non-empty.
7. There is no undetermined state and no `BackingRefTypesUndetermined` error.
   Every catalog family declares `NoBacking` or `Constrained`; a family whose
   dossier does not state its backing is refused at construction, before
   publication, rather than carried as a runtime value.
8. A family declares `NoBacking` if and only if every name in its
   `service_spec_allowed` appears in `NON_BACKING_SERVICE_BASE_FIELDS`. That
   classification is total and disjoint over the catalog's base field universe,
   so an unclassified new base field fails a test rather than defaulting either
   way.
9. For a `serviceType` in the D098 catalog, an advertised `ProjectionFactory` is
   admitted only when every declared field equals the catalog-derived factory,
   and the comparison order is normative: declared protocol version, then
   identity, both reference sets, and `exportability`, then fingerprints. A
   protocol difference is never reported as a fingerprint mismatch, and
   `exportability` is compared explicitly because no fingerprint binds it.
10. A resource is import-owned if and only if its `metadata.ownerRef` names
    `ResourceImport/<name>`. An import-owned resource is never an export target
    and is never a backing reference. Export and backing admission read the
    stored resource; a bare `ResourceRef` is not sufficient evidence of origin,
    and no caller-supplied mode or origin discriminant is accepted. The denial
    has its own bounded, redacted variant, `ImportOwnedOriginRejected`, distinct
    from a wrong-type denial.
11. `projectionProtocolVersion` is a declared field of the signed factory
    descriptor as well as a `factoryFingerprint` input, so version skew is
    observable without recomputing a hash. The version is `1.1`; the base schema
    version stays `1.0` because no base or projection field set moved. The
    operator remedy for a mismatch is to install a Provider artifact built for
    the Core protocol in use; an operator is never told to regenerate or re-sign
    a descriptor, because a descriptor is signed by its author.
12. An absent `projectionProtocolVersion` deserializes as the bounded constant
    `LEGACY_ABSENT_PROTOCOL_VERSION`, never as caller text and never as a
    deserialization failure, so a legacy descriptor reaches admission and is
    refused there with a typed reason. That constant is never equal to
    `SEMANTIC_PROJECTION_PROTOCOL_VERSION`, so defaulting can never manufacture
    a match. `deny_unknown_fields` is unchanged: the default covers a missing
    known field only.
13. Each generated `*_projection_spec.schema.json` publishes every declared
    factory field as an `x-d2b-*` key, not only the fingerprints, so Nix can
    compare `exportability` and `allowedBindingTargetRefTypes` at eval time.
    `x-d2b-exportability` is the only published field no fingerprint binds, at
    either the Rust or the Nix layer.
14. The security-key provider-neutral base continues to reject `deviceRef`,
    `relayEndpointRef`, `authority` in a projection, and every physical
    selector.
