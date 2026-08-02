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

### 3. One enum replaces the two independently coupled declaration fields

The backing declaration is a single three-state value, so a type set and the
base fields it applies to cannot drift apart:

```rust
/// What a semantic family declares about its owner Service's same-Zone
/// backing, as one value.
pub(crate) enum SemanticBackingDeclaration {
    /// The specification does not state this family's set. No factory is
    /// derivable; this is the only state that yields
    /// `BackingRefTypesUndetermined`.
    Undetermined,
    /// The provider-neutral Service base names no backing resource, so the
    /// closed set is empty and admits nothing.
    NoBacking,
    /// The base names backing-reference fields, and this is their closed
    /// type set. Both slices are non-empty.
    Constrained {
        types: &'static [&'static str],
        fields: &'static [&'static str],
    },
}
```

Per family: `Constrained { types: ["Endpoint"], fields: ["implementationEndpointRefs"] }`
(audio), `Constrained { types: ["Endpoint"], fields: ["ingestEndpointRefs"] }`
(telemetry), `Constrained { types: ["Device"], fields: ["backingDeviceRef"] }`
(USB), and `NoBacking` (security key).

**What this makes unrepresentable.** A type set with no field to apply it to,
and a declared backing field with no type set, are both gone: neither has a
spelling. There is no longer a pair of fields that a later edit can move
independently, which was the exact defect the first draft carried.

**The one degenerate spelling that remains, and its guard.**
`Constrained { types: &[], fields: &[] }` is still constructible by a careless
author and is a synonym for `NoBacking`. `SemanticPairContract::build` rejects
it, alongside a `Constrained` whose slices disagree in emptiness, before any
contract is observable. This is a build-time rejection on a compile-time
constant, and every family's `contract()` is exercised, so the failure surfaces
as a test failure rather than in production. Claiming the enum makes this state
unrepresentable would overstate what Rust's slice types give us, so it is stated
as what it is.

**Grounding.** Every name in `Constrained.fields` is a member of that family's
`service_spec_allowed`, asserted over the whole catalog. That catches a base
field rename that forgets the declaration.

The declaration is **not** a fingerprint input beyond the resolved type set that
D096 already names. `fields` never reaches `factory_fingerprint`: D096 does not
name it, and feeding it in would move fingerprints for a catalog-internal
consistency record.

### 4. `Undetermined` is the only state that fails closed as undetermined

`SemanticContractError::BackingRefTypesUndetermined` is returned for
`Undetermined` and for nothing else. `NoBacking` yields a factory whose backing
set is empty.

A future family whose dossier genuinely does not state its set must still fail
closed with that typed error rather than default to `NoBacking`, because "we do
not know" and "there is none" are different claims and only one of them is safe
to sign. After this decision no shipped family is `Undetermined`; the state is
retained for the next gap, not deleted as dead code.

Consequently `null` and `[]` remain distinct fingerprint inputs. They are
distinct claims, so they must hash differently.

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

### 9. The projection-protocol version bumps to 1.1, so downstream drift is version skew and not tamper

The meaning of `allowedBackingRefTypes` changes: a value that was previously
unrepresentable becomes representable, and its reading is fixed as deny-all.
That is a change to the projection protocol, not to any schema's field set. So
the versioned surface that moves is the protocol version, and only it:

```rust
// packages/d2b-contracts/src/v3/semantic_services/mod.rs
pub const SEMANTIC_PROJECTION_PROTOCOL_VERSION: &str = "1.1"; // was "1.0"
```

**Why minor and not major.** An old descriptor's non-empty set means exactly
what it meant before, and every field keeps its type and name, so nothing that
parsed before fails to parse now. What changes is the admitted value domain,
which widens. A widening with unchanged meaning for existing values is a minor
bump.

**Why not the base schema version.** `SEMANTIC_BASE_SCHEMA_MAJOR` and
`SEMANTIC_BASE_SCHEMA_MINOR` stay at `1` and `0`. No base or projection field
set changes, so moving them would move all sixteen base-layer fingerprints and
all four projection-schema fingerprints for a change that touched none of those
surfaces. The blast radius must match what actually moved.

**What the bump buys.** The protocol version is an input to
`factory_fingerprint` and to nothing else, so bumping it moves **all four**
families' factory fingerprints, not just security key. That is the point. A
stale descriptor of any family now mismatches an updated Core's expectation, and
the mismatch is attributable to a declared protocol version rather than
presenting as a fingerprint that differs for no stated reason. Leaving the other
three unchanged would have made security key's move look like tampering and the
others look fine, which is precisely the failure mode this finding named.

**Regeneration and migration.** Regeneration is
`run_xtask gen-semantic-service-schemas`, gated by the enforcing `make
test-drift` lane; all four `*_projection_spec.schema.json` artifacts are
rewritten in one commit. **There is no migration, because there is no runtime
consumer.** No Provider descriptor is signed, no `ResourceExport` or
`ResourceImport` type exists yet (`packages/d2b-contracts/src/v3/` has no
`resource_export.rs` or `resource_import.rs`; they are
`ADR046-zone-control-019`'s destination), and every work item in this program
carries `dataMigration: None; full reset`. The versioning is therefore explicit
rather than load-bearing today, and it is recorded now so that the first signed
descriptor is minted against a stated protocol version instead of an implicit
one.

**The pinned values**, measured by re-deriving `factory_fingerprint` for every
family with `projectionProtocolVersion` at `"1.1"` and security key's backing
set moved from `null` to `[]`:

| Family | `factoryFingerprint` before | after | `projectionSchemaFingerprint` |
| --- | --- | --- | --- |
| audio | `sha256:80ef3d08378a61ac924944564efa136b0cfba314d1e48567680d16cc75ac4b38` | `sha256:67352424b92e8da62d2c39f664d9028c85fdede9c38f6a9e3e1423d3009a33a6` | unchanged |
| security key | `sha256:444b0f9ba1d9997a392314a5aa9b49ca180a2e8c5d7541d1eb1846d2b3c460dc` | `sha256:8101ab8d17bac0cc1f57f957223fa531a3a5d231f93bc8e56e540dc499830027` | unchanged |
| telemetry | `sha256:de3ef22c8138fbe84c7905fa029d5b3c3f5bed364063b35affe8e0638ca26185` | `sha256:6e6c64a3e39554c76f7d745758a8faf2b81135556dbcd82ea085a073c7334218` | unchanged |
| USB | `sha256:72b5cafbd2409d187b523b1d6076094f8d6246d0a5714240d1b7bac775ed7b45` | `sha256:f73ce4a2ef7d6c21bfdf4f14da51be28d8ee7b53ecf85751f87c29df9a8d9115` | unchanged |

Security key's `x-d2b-allowed-backing-ref-types` additionally moves from `null`
to `[]`. Every `projectionSchemaFingerprint` is unchanged because no projection
field set moves. Exactly one committed file pins any old value: the four
generated artifacts themselves. The dossiers' Nix examples use placeholder
tokens such as `sha256:<security-key-projection-factory>`, not literals, so no
prose pins one.

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
every import-owned row is denied at both ends of the path. Decision 6 also gives
the membership test a typed method whose test is unconditional, so the branch
above has nowhere to live. Decision 7 removes the family's own Service type from
every backing set at declaration time, so the chain has no first link even if a
row's ownership were somehow unreadable. Decision 3 stops a later author from
resolving the oddity by inventing a non-empty set, because the type set and its
base fields are now one value. The negative tests named below assert that the
security-key factory rejects `Device/x`, `Endpoint/x`, and
`security-key.d2bus.org.SecurityKeyService/x`, and that an import-owned row of
an otherwise-admissible type is denied for both export and backing.

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

Replace the `allowedBackingRefTypes` row of the D096 factory table:

> | `allowedBackingRefTypes` | Closed set of same-Zone `Device`, `Endpoint`, or qualified semantic backend types the owner Service may reference **in its provider-neutral base**. Empty when the family's base declares no backing-reference field. The empty set denies every backing reference and is never read as unconstrained. It may not contain the factory's own `serviceType` or `bindingType`. |

### C. `docs/specs/ADR-046-resources-zone-control.md`

Replace the `allowedBackingRefTypes` row of the 8A.1.1 factory table with the
same text as amendment B; append to the paragraph beginning "Provider install,
Nix build, and API admission verify this metadata":

> For a `serviceType` in the D098 semantic catalog, the advertised factory is
> admitted only when it equals the catalog-derived factory field for field,
> including both reference sets and both fingerprints. A Provider adds strict
> extension schemas; it never restates, widens, or narrows the semantic factory.
> The `factoryFingerprint` binds the semantic projection-protocol version, so a
> descriptor minted under an earlier protocol version mismatches by version skew
> and not by tampering.

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
changing. The normative readings land in the two specs and the dossier, which
are the documents an implementer reads.

### E. `specs/001-adr046-d2b3-completion/implementation-debt.md`

Verified by reading the file at this branch's base: the two sections are
`### 13.3 Security-key cannot construct a signed projection factory at all`
(line 942) and
`### 19.8 The security-key semantic projection is not invented` (line 2019).
Both exist, both record this gap as open, and both citations in this record are
correct. Each gains a closing line naming this ADR and the amendment that
discharges it. Ownership of that edit belongs to whichever wave lands amendments
A to C, not to this record, because the debt register describes the tree and the
tree does not change until then.

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

- `a_backing_declaration_is_well_formed_for_every_family` - decision 3's
  build-time rejection of `Constrained` with either slice empty, exercised by
  constructing the degenerate declarations locally and asserting the contract
  build refuses each.
- `every_declared_backing_field_is_a_service_base_field` - decision 3's
  grounding invariant.
- `no_family_admits_its_own_service_or_binding_type_as_backing` - decision 7.
- `every_family_yields_a_projection_factory` - all four return `Ok`, replacing
  the current three-of-four state.
- `only_undetermined_fails_closed_as_undetermined` - a locally constructed
  `Undetermined` declaration yields `BackingRefTypesUndetermined`, and no
  shipped family does, so decision 4's retained state is exercised rather than
  left as unreachable code.
- `the_factory_fingerprint_binds_the_projection_protocol_version` - re-deriving
  a family's factory fingerprint with a different protocol-version string
  produces a different value, so decision 9's version is load-bearing rather
  than decorative.

In `packages/d2b-contracts/src/v3/provider.rs`:

- `an_empty_backing_set_is_accepted_and_an_empty_target_set_is_not` - decision
  5's asymmetry, both halves.
- `a_factory_may_not_name_its_own_service_or_binding_type_as_backing` -
  decision 7 at the constructor.
- `an_import_owned_resource_is_never_an_export_target` - decision 6. Build one
  envelope of the family's Service type with `metadata.ownerRef` naming
  `ResourceImport/yubikey-primary` and one with no owner; assert the first is
  rejected and the second admitted, for every family.
- `an_import_owned_resource_is_never_a_backing` - decision 6, same construction
  against `admits_backing_ref`, using a `Device` envelope for USB and an
  `Endpoint` envelope for audio and telemetry so the type is otherwise
  admissible and ownership is the only thing under test.
- Round-trip: a factory with `allowedBackingRefTypes: []` serializes and
  deserializes without loss, and the deserializer still rejects an empty target
  set.

At the zone-control export admission layer, in
`packages/d2b-core-controller/src/export_import.rs` and its test module, owned
by `ADR046-zone-control-019`:

- `an_export_of_an_import_owned_projection_is_denied` - the caller-facing
  negative test. A consumer Zone holding a projection Service created by its
  `ResourceImport` attempts to declare a `ResourceExport` naming it; admission
  denies it, so a grandchild Zone cannot obtain owner authority through a Zone
  that held only a lease.
- `an_owner_service_may_not_name_an_import_owned_backing` - an owner Service
  naming an import-owned resource of an allowed backing type is denied, so a
  leased capability cannot be laundered into a local authority claim.
- `an_empty_backing_set_denies_a_specified_backing` - the caller/export negative
  test this finding requires, distinct from the unit test above because it runs
  through admission rather than the factory directly. An owner
  `SecurityKeyService` that names any backing reference at all is denied by an
  empty-set factory. Negative control in the same test: the same call shape
  against the USB factory with `Device/work-token` is admitted, so an
  unconditionally denying admission path cannot pass this test vacuously.
- `an_export_of_a_locally_owned_authority_is_admitted` - the positive control,
  so the three denials above are not satisfied by an admission path that denies
  everything.

In `packages/d2b-provider-toolkit/tests/malicious_provider.rs`:

- A descriptor advertising a security-key factory with a non-empty backing set,
  or with the catalog's set but a recomputed fingerprint, or minted under
  projection-protocol version `1.0`, is rejected by decision 8's equality check.

Artifact and drift:

- `run_xtask gen-semantic-service-schemas` regenerates all four
  `docs/reference/schemas/v3/*_projection_spec.schema.json` artifacts with the
  `factoryFingerprint` values pinned in decision 9, and the security-key
  artifact additionally with `"x-d2b-allowed-backing-ref-types": []`. Enforced
  by `make test-drift`.

API surface, for decision 6's mint-surface widening:

- `admits_backing_ref` is a new public method on `ProjectionFactory`, a type
  already carried in `packages/d2b-bus/tests/approved-capability-api.txt`, and
  `admits_export_target`'s signature changes. Both are two-way census entries,
  so the implementing wave runs `make api-surface-pin` to regenerate
  `tests/golden/api-surface/` and the approved capability list, and
  `make test-rust-api-surface` to prove the regenerated census matches. The
  regeneration is a deliberate trust-boundary change whose stated reason is this
  record; the census must not be pinned without citing it.

Lanes that must be green for the implementing wave: `make test-rust`,
`make test-rust-api-surface`, `make test-drift`, `make test-fixture-contracts`,
`make check-tier0`, `make test-policy`.

## Invariants this decision creates

1. `allowedBackingRefTypes` is a closed allowlist and the empty allowlist admits
   nothing. No consumer branches on its emptiness.
2. A family's backing declaration is one value with three states. `NoBacking`
   is declarable only because the provider-neutral Service base declares no
   backing-reference field; `Constrained` carries a non-empty type set and a
   non-empty field set together, and a `Constrained` with either slice empty is
   rejected at contract build.
3. Every name in a family's `Constrained.fields` is a member of that family's
   `service_spec_allowed`.
4. The declared backing fields are never a fingerprint input; only the resolved
   type set D096 names reaches `factory_fingerprint`.
5. A factory's backing set never contains its own `serviceType` or
   `bindingType`.
6. `allowed_binding_target_ref_types` is always non-empty.
7. `Undetermined` means "the specification does not state this set" and is the
   only state that fails closed with `BackingRefTypesUndetermined`. It is never
   a synonym for `NoBacking`, and `null` and `[]` hash differently.
8. For a `serviceType` in the D098 catalog, an advertised `ProjectionFactory` is
   admitted only when equal field for field to the catalog-derived factory.
9. A resource is import-owned if and only if its `metadata.ownerRef` names
   `ResourceImport/<name>`. An import-owned resource is never an export target
   and is never a backing reference. Export and backing admission read the
   stored resource; a bare `ResourceRef` is not sufficient evidence of origin,
   and no caller-supplied mode or origin discriminant is accepted.
10. `factoryFingerprint` binds the semantic projection-protocol version, so a
    fingerprint mismatch across a protocol change is attributable version skew
    rather than tampering. The version is `1.1`; the base schema version stays
    `1.0` because no base or projection field set moved.
11. The security-key provider-neutral base continues to reject `deviceRef`,
    `relayEndpointRef`, `authority` in a projection, and every physical
    selector.
