# Provider crate layout and the per-crate declarations

Every provider family keeps its vocabulary in crates that its own family owns.

under `packages/d2b-provider-*`. One declaration file per crate
(`resource-types.json` next to the crate's `Cargo.toml`) is the single source of
truth for what that crate provides. The generator aggregate derives every registry,
inventory, projection, and authority row from those declarations, and the layout
check verifies the result; so adding, renaming, or retiring a row in one crate
moves every consumer in the same change, and no consumer can drift from the
declaration set.

## The declaration file

All declaration files have the same shape; every provider crate carries one,
at its crate root:

```json
{
  "crate": "d2b-provider-<family>",
  "types": [
    {
      "resourceType": "<CamelCaseType>",
      "allowedSources": ["builtin", "startup"],
      "verbs": ["get", "list", "watch", "create", "update-spec", "update-status", "update-metadata", "update-finalizers", "delete"],
      "execution": ["host", "guest"],
      "exportable": false,
      "reads": ["<ResourceType>", "<Kind>"]
    }
  ],
  "provides": ["Provider/<name>", "Process/<name>"],
  "roles": [
    {
      "name": "<RoleName>",
      "description": "What the role means on the host.",
      "providerRef": "Provider/<owning-provider>",
      "operations": [],
      "principals": [],
      "storageRoots": [],
      "seccompClasses": [],
      "deviceClasses": [],
      "capabilityGrants": []
    }
  ],
  "principals": []
}
```

The declaration names the crate and every resource type the crate serves: the
types' verbs, execution classes (a list of `host`/`guest`), and the other
resource kinds each type reads; the crate-level `provides` list names the
provider and process references it realizes; and the crate-level `roles`
(and their principals, storage roots, seccomp classes, device classes,
and capability grants) name the vocation rows the crate declares. A role
row's optional `providerRef` names the Provider the role resolves to (the
role-to-provider mapping; a role no Provider serves omits it), and its
optional `description` is emitted as the generated role vocabulary's variant
documentation. It does
not name effects:the families that declare effect services - activation-nixos,
host, network-local,and process - serve their own effect implementation over
the declared facets(the daemon supplies the facet implementations), while in
the remaining family crates the production implementation of the declared port
still lives in the daemon. Either way the declaration names no daemon surface.

The resource-type authority (U4) aggregates the declared role vocabulary into
two committed consumers:

- `packages/d2b-core/src/generated/process_roles.rs` - the `ProcessRole`
  enum in `d2b-core`, `include!`d by `packages/d2b-core/src/processes.rs`,
  rendered from the declaring crate's role names and descriptions in
  declaration order.
- `nixos-modules/generated/process-role-providers.nix` - the role-to-provider
  map `nixos-modules/resources-zones-processes.nix` folds for the process
  compiler, rendered from the declared `providerRef` rows.

Both are drift-gated with the other authority artifacts. A declared role must
be spelled in its crate's descriptor sources (`ProcessRole::<Name>`), and a
declaration that widens an authority-bearing role fact - a role's operations,
principals, storage roots, seccomp classes, device classes, or capability
grants - beyond the crate's committed scope fails naming the widened fact
(R12). The declared service facets one method carries (its required
privileges, state cells, and descriptor-leg type/rights in `operations.json`)
carry the same committed per-crate bound (U4).
A second, smaller declaration (`registrations.json`, beside
`resource-types.json`) names the family's runtime registration: the
effect-service ids the family registers. The Provider identity that
registration is made under is **not** stated there - it is the crate's own
runtime identity in the per-crate identity authority (below). The daemon
composes the generated registration table instead of naming families, so a new
family is registered by declaring it in its own crate - no daemon edit and no
layout-ratchet row. A crate that registers no service carries an empty
`services` list:

```json
{
  "crate": "d2b-provider-<family>",
  "services": ["<family>.d2bus.org/<service>"]
}
```



## The Provider identity authority

`packages/d2b-provider-*/provider-identity.json` is the one place a provider
crate's Provider identities are stated. Every other declaration keeps its own
facts - a registration row states the effect services a runtime identity
registers, a session catalog row states the routing a session identity
answers, a packaging row states the artifact a product identity ships - and
each resolves its identity through this authority. Nothing reads an identity
out of a crate's directory name, and no count of any category is written down
here: the census is read from the declarations and the generated artifacts.

A crate declares one identity slot per surface, and a slot states either the
identity it owns or the closed reason it owns none. Each identity names the
production sources that name it, and a slot whose evidence is missing is a
refusal. The categories a declaration can fall into:

- **product** - the identity the packaging metadata and the Nix provider
  catalog ship. A crate in this category carries a packaging matrix row; a
  packaging row whose crate declares no product identity fails.
- **runtime** - the identity the daemon's composition root registers through
  the generated registration table. A crate that declares services but owns no
  runtime identity fails.
- **session** - the identity the session-plane service catalog routes to. A
  catalog row whose crate owns no session identity fails.
- **fixed-bootstrap** - a packaged product identity the deployment registers
  at startup, outside ordinary Process projection and outside the ProviderSet
  runtime registrations. It lives on the product surface because that is where
  its artifact ships; its startup is a deployment fact, so a fixed-bootstrap
  crate owns no runtime identity and never becomes a normal runtime row. A
  declaration also states whether the artifact it packages contains a binary,
  and that flag is what selects the catalog's one non-binary bootstrap entry:
  the deployment registers the non-binary identity as the root of its graph
  without materializing a process for it, so the flag belongs to a
  fixed-bootstrap crate owning a product identity, exactly one crate may state
  it, and the generator reads the identity from the declaration rather than
  naming one.
- **shared-driver** - the crate declares a driver another family's
  registration runs, so it owns the identity that driver serves. A crate in
  this category registers no Provider of its own.
- **blocked** - the crate records an external issue it is waiting on. A
  blocker is a record, never an exemption: it is read only out of a
  declaration that is present and complete.
- **no identity** - the crate implements other crates' surfaces and owns none
  of its own (a support crate, a test crate, a resource-vocabulary crate, a
  service-only crate, or a crate that deliberately owns no Provider). It
  states which of those classes it is and nulls every surface with its reason.

The three surfaces are independent: a crate routinely owns one identity on one
surface and none on another. `d2b-provider-process-systemd` ships a
product-plane artifact and registers a different runtime identity, and
`d2b-provider-endpoint` is registered at runtime without being product
packaging at all. An identity is written as the resource name the contracts
admit; the `Provider/<name>` reference form is derived from it and is never
authored, and every `Provider/<name>` reference a declaration emits resolves
against the authority, so a reference to an identity no crate declares is
refused at generation.

## What reads the declarations

The declarations drive everything the runtime and the Nix tree need to know
about the resource model:

- `generated/new-graph/v3_converted_resource_types.rs` - the committed
  resource-type authority the contracts layer serves (`include!`d by
  `packages/d2b-contracts/src/identity.rs`).

- `generated/new-graph/provider_registrations.rs` - the committed
  provider/service registration table the daemon composition root composes
  (`include!`d by `packages/d2bd/src/resource_plane_v3.rs`): one row per
  declaring crate, carrying the runtime identity the crate declares in the
  identity authority and the declared effect-service ids. The registration
  authority's parity gate refuses a registration whose crate declares no
  runtime identity, a declared service the crate's sources do not spell, and
  a service the crate spells or registers that the declaration omits. The
  identity itself is refused by the authority: a malformed identity, an
  identity no production source names, and an identity two crates declare are
  all failures before any generation runs.

- `generated/new-graph/service_provider_catalog.rs` - the committed
  service-to-provider catalog the zone-plane session contract serves
  (`include!`d by `packages/d2b-contracts-zone-session/src/v3/mod.rs`), so the
  bus resolves a service to its provider without naming a family.

The three Rust tables above are staged by `gen-new-graph` under the closure's
one committed location and compiled from there. There is no per-crate copy
beside the compiled one; `generated/new-graph/build_closure.json` records the
production source file each staged artifact is included by.

- `nixos-modules/generated/resource-types.nix` - the Nix type registry.

- `nixos-modules/generated/resource-inventories.nix` -the closed resource
  vocabularies the Nix authoring surface validates against (control types,
  Role/RoleBinding subjects, resource verbs, session verbs, and the
  committed schema pointers).
- `nixos-modules/generated/provider-projections.nix` - how a provider's rows
  project onto the zones it serves.
- `nixos-modules/generated/options-zones-Zone.nix`,
  `options-zones-ZoneLink.nix`, `provider-catalog-shape.nix`,
  `semantic-resource-types.nix`, and `zone-spec-canonical.nix` - the
  zone/zone-link/resource shapes the Nix host modules read.


- `nixos-modules/host-users.nix` - the host-user allocation rows, derived
  from `docs/reference/policy/principal-allocation.json` and the declarations' principals.
- the generated views of the broker operation catalog (`broker_operation_*`
  generated modules, `docs/reference/broker-operation-triage.md`), read
  `docs/reference/policy/broker-operations.json` and the per-crate
  `operations.json` declarations, and never restate a facet: a row cannot
  move without moving every view. The committed rows document is itself a
  generated view of the declarations plus the retained non-declared rows
  (U2/KTD3).

`gen-nix-inventories` and the other `gen-*` commands regenerate these files;
`tests/tools/generate-artifacts.sh` runs the whole set in one pass. The
generated files are committed, and the drift gates (Bazel `gen_*_drift`
targets) fail when a run would rewrite them.



## The layout gate

`cargo xtask check-provider-crate-layout` (or `xtask
check-provider-crate-layout`) reads the tree and the declarations and refuses:

- a resource driver declared in a shared crate (non-provider) module, unless a
  row in the exemption ratchet names the module and token;
- any family knowledge token in a shared crate the family ratchet does not
  already carry (a new occurrence fails the scan);
- any tokenless structural signal (a per-family branch, a hand-written Nix
  literal, a golden wire string, a type-name match arm, a service catalog
  case) the structural ratchet does not already carry;
- any provider crate carrying another family's identity token (zero-outside-
  edit: a crate may only spell its own family's identity, plus the
  reference forms `Provider/<name>`, `Process/<name>`, `Host/<name>`,
  `User/<name>`, and `Guest/<name>`);
- any row in either ratchet whose module no longer carries its token or signal
  (a stale row fails, so the inventory only shrinks).
- any committed inventory row naming a file the tree does not carry (the
  committed-scope proof).

The gate's `--fix` flag regenerates the generated Nix inventories from the
declarations.



##Carve-out convention

The two ratchets (`SHARED_FAMILY_KNOWLEDGE_RATCHET` and
`SHARED_STRUCTURAL_KNOWLEDGE_RATCHET` in
`packages/xtask/src/provider_crate_policy.rs`) hold every still-live guess
the tree carries in a shared crate. Each row names its module, its token or
signal class, and `retires_with`: the reason the knowledge cannot move, or the
step that was supposed to retire it. Rows that are permanent carry a
`permanent:...` reason naming the detector or test that refuses the move
 (the broker's provider-free pin, the dependency-direction detector, a
  golden wire-vector test, the shared-crate dependency rule), so a reader
 learns why the row stays without reading the crate. Rows debuted during the
census keep their census-step text as history: they remain live sites whose
tokens the tree still carries, anda stale row would fail the gate.



Per-provider crates keep their own smaller ratchet
(`PROVIDER_FAMILY_KNOWLEDGE_EXEMPTIONS`) for family-knowledge tokens the
crate legitimately carries (effect-id strings routing provider effects through
a shared driver, for example); the README-only integration ratchet records
scaffold crates whose integration surface is still so thin that their README
sections are their real documentation. Two framework driver declarations
(`MetadataDriver`, `MetadataDriverFactory`) record the drivers the metadata
framework exports, in shared `d2b-resource-runtime`; no others remain. The
blanket-allow table has one entry (the broker-composition audit tool's own
synchronous-path reads); everything else is a per-site row with reason.

##Renaming or adding a family

1. Move the vocabulary into the family's own crate(s) and declare the crate's
   `resource-types.json`: types with their verbs, execution classes, and
   reads, and the crate's provides, roles, principals, storage, security, and
   capability rows. Declare the crate's `provider-identity.json` beside them:
   it states which identity the crate owns on each of the product, runtime and
   session surfaces, and the closed reason for every surface it owns none on.
   If the family needs the daemon's composition root to register it, declare
   its `registrations.json` too: the generated registration table carries the
   crate into the daemon with no daemon edit.
2. Run `tests/tools/generate-artifacts.sh` so every generated view moves in
   the same change.
3. Run `xtask check-provider-crate-layout`; if it adds rows to the ratchets,
   carry them in the same change. If a shared-crate site cannot move, the
   row must carry its reason (and ideally a `permanent:...` reason naming
   the blocker), because any new unlisted occurrence fails.
4. Regenerate `nixos-modules/generated/*` and `host-users.nix` via the
   aggregate; re-verify the host-contract golden digest
   (`tests/unit/nix/cases/host-contract-digest.nix`) if the host contract's
   surface changed, and re-run the policy drift surfaces (the
   `privileges-json-drift` surface and the generator drift gates).

The policy documents themselves
(`docs/reference/policy/broker-operations.json`,
`docs/reference/policy/principal-allocation.json`) are committed: the
broker-operations document is generated output (the broker-operation
generator rewrites it from the per-crate `operations.json` declarations plus
the retained rows, and its drift target pins it byte-for-byte), while
principal-allocation.json is a committed input the aggregate reads but never
rewrites. Their drift surfaces are the generated views that must match them
byte-for-byte and (the host-contract golden digest case), which pins the
contract digest the host module derives from the allocation, the zone model,
and the bundle framing. A lane that moves a row in one of those documents
regenerates the consumers and re-pins its digest in the same change.

## The converted family's crate

The family's crate is the thing a conversion ships: `src/` carries the
driver surface the plane registers, the family's own effects implementation,
and the declared facets that daemon and tests supply.

- the effect port - the `pub trait <Family>Effects: Send + Sync + 'static`
  everything the driver needs arrives through. The family crate serves its
  own effect implementation over the daemon-supplied declared facets (hosted
  per zone as a declared service where the family declares one), so the
  crate holds no daemon state type and the dependency direction runs
  daemon-to-crate;
- the service - one concrete struct (`<Family>Driver`) holding
  `Arc<dyn <Family>Effects>`, built by the one public constructor over the
  facet-carried port and by nothing else;
- the registration surface - the spec decoder, the factory
  (`<Family>DriverFactory`), and the declaration (the shared
  `d2b_resource_types::DriverDescriptor` built by a family constructor such as
  `credential_descriptor`, `host_descriptor`, or `volume_descriptor`) the plane
  registers the type by, carrying the declared verbs, execution domains,
  exportability, reads, and allowed sources; and
- the scripting double - `pub mod test_support`, gated
  `#[cfg(any(test, feature = "test-support"))]`, so this crate's unit tests
  and other crates' tests (the daemon's plane tests, the integration crates)
  opt in through the `test-support` feature. The double scripts the seam
  hermetically: it records calls order-preservingly and can script outcomes
  and refusals.

Composition and scripting cross one seam - the declared facet set. The
daemon's composition root builds it from its runtime (for example
`ProcessProviderRuntime`), tests build it from the scripting double, both
through the same declared types, and the family's own implementation serves
over whichever set arrives: no second constructor and no separate scripting
surface. The composition root iterates the generated registration table, so
registering a new family needs no structural edit at that site; it still
names the families it carries when it wires them (the registered-drivers
match and the service factories spell each carried family, because the crate
reference and family id are the dependency itself), and a family is never
started by a bespoke hand-written site outside the table. A family that
serves effect services spells their ids in `registrations.json` and in its
descriptor sources, and nowhere else.

A reviewer rejects:

- a generic service with a second test-only constructor - a service generic
  over the port that grows a `cfg`-gated constructor for scripting. The
  production and test shapes diverge: composition builds one surface, tests
  exercise another, and nothing proves the tested shape is what runs;
- a discarded facet boundary - a constructor that accepts the declared
  facets and drops them (`let _ = facets;`). The declared seam exists on
  paper only: composition and scripting stop sharing a surface, and the
  next test author builds a private seam instead of using the declared one.
  Construction consumes the port it names;
- a scripting double that is not release-gated - a crate-private
  `#[cfg(test)]` module. Only the crate's own unit tests can reach it; the
  daemon's plane tests and the integration tests are other crates, so they
  cannot script the probe hermetically and fall back to the real machine.
  The double is `pub` behind `#[cfg(any(test, feature = "test-support"))]`;
- a registration or binding test that reads the real machine - an NSS
  lookup, a `/dev/kvm` probe, a proc scan in a registry test. Such a test
  fails for environmental reasons or, worse, passes without asserting
  anything when the environment degrades, and it can never prove the
  registration contract. Registration tests script the seam through the
  double and assert the registry behavior: the declared type is served, a
  second registration is refused, registration after the plane opens is
  refused;
- prose left describing the old shape - a README or doc comment the move
  falsified (a README claiming the daemon implements the effects while the
  crate now serves them itself, a sibling README naming a deleted adapter
  as the sole implementor, a doc comment describing the deleted adapter as
  the seam). A conversion is not complete while shipped prose contradicts
  the code.

A conversion completes when, beyond the steps above:

- the generated views are produced through the authority (the aggregate or
  `xtask check-provider-crate-layout --fix`) and committed in the same
  change - a hand edit of a generated file fails the drift gates;
- the change ships release notes: an entry under `## [Unreleased]` in
  `CHANGELOG.md`, or a `changelog.d/<branch-name>.md` fragment with the
  `### <Section>` headings Keep a Changelog requires. A bullet appended to
  the previous bullet's line is a review defect, not a fragment;
- every README and doc comment the move invalidated is updated in the same
  change; and
- newly added files end in a trailing newline (cosmetic, but review catches
  it on conversion lanes).
