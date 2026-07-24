# ADR specification sets

The files in this directory are focused normative specifications attached to an
architecture decision record. They keep implementation-level contracts
reviewable without turning the parent ADR into one monolithic document.

## ADR 0046 set

Every ADR 0046 specification is named:

```text
ADR-046-<spec-name>.md
```

The parent decision is
[`docs/adr/0046-d2b-3-provider-control-plane.md`](../adr/0046-d2b-3-provider-control-plane.md).
The parent and every manifest-listed spec form one atomic normative set:

- all files move from `Proposed` to `Accepted` together, and every member is
  `Proposed` today; this is a documentation-only set under user review;
- a content change to any member invalidates validation and panel evidence for
  the set;
- no spec may silently override another spec;
- cross-spec dependencies must name existing member Spec IDs and stay acyclic;
  the parent ADR is not itself a spec dependency;
- one spec owns each serialized contract, state machine, ResourceType,
  controller, Provider dossier, process model, and security invariant.

### Member index (55 specs)

The set has **55 normative member specs**: 28 foundation, resource,
cross-cutting, and closing specs, plus 27 Provider dossiers. The parent ADR,
this `README.md`, the `providers/README.md` index, the generated `*.json`
manifests, and the generated implementation-graph artifacts
(`ADR-046-implementation-graph.json`/`.md`) are **not** members.

**Foundation and platform (15):**

- [`ADR-046-decision-register`](ADR-046-decision-register.md) — resolved
  decisions (through D098)
- [`ADR-046-terminology-and-identities`](ADR-046-terminology-and-identities.md)
- [`ADR-046-resource-object-model`](ADR-046-resource-object-model.md)
- [`ADR-046-resource-store-redb`](ADR-046-resource-store-redb.md)
- [`ADR-046-resource-api-and-authorization`](ADR-046-resource-api-and-authorization.md)
- [`ADR-046-resource-reconciliation`](ADR-046-resource-reconciliation.md)
- [`ADR-046-primitive-resource-composition`](ADR-046-primitive-resource-composition.md)
- [`ADR-046-componentsession-and-bus`](ADR-046-componentsession-and-bus.md)
- [`ADR-046-zone-routing`](ADR-046-zone-routing.md)
- [`ADR-046-provider-model-and-packaging`](ADR-046-provider-model-and-packaging.md)
- [`ADR-046-provider-state`](ADR-046-provider-state.md)
- [`ADR-046-core-controllers`](ADR-046-core-controllers.md)
- [`ADR-046-components-processes-and-sandbox`](ADR-046-components-processes-and-sandbox.md)
- [`ADR-046-nix-configuration`](ADR-046-nix-configuration.md)
- [`ADR-046-current-code-migration-map`](ADR-046-current-code-migration-map.md)

**Resource catalog (6)** — the 19 standard ResourceTypes (`Zone`, `ZoneLink`,
`Provider`, `Role`, `RoleBinding`, `Quota`, `EmergencyPolicy`, `Host`, `Guest`,
`Process`, `EphemeralProcess`, `User`, `Volume`, `Network`, `Device`,
`Credential`, `Endpoint`, `ResourceExport`, `ResourceImport`) are owned across the foundation and these catalog
specs:

- [`ADR-046-resources-zone-control`](ADR-046-resources-zone-control.md) —
  `Zone`, `ZoneLink`, `Quota`, `EmergencyPolicy`
- [`ADR-046-resources-host-guest-process-user`](ADR-046-resources-host-guest-process-user.md) —
  `Host`, `Guest`, `Process`, `EphemeralProcess`, `User`, `Endpoint`
- [`ADR-046-resources-volume`](ADR-046-resources-volume.md) — `Volume`
- [`ADR-046-resources-network`](ADR-046-resources-network.md) — `Network`
- [`ADR-046-resources-device`](ADR-046-resources-device.md) — `Device`
- [`ADR-046-resources-credential`](ADR-046-resources-credential.md) — `Credential`

**Cross-cutting (3):**

- [`ADR-046-cli-and-operations`](ADR-046-cli-and-operations.md)
- [`ADR-046-telemetry-audit-and-support`](ADR-046-telemetry-audit-and-support.md)
- [`ADR-046-security-and-threat-model`](ADR-046-security-and-threat-model.md)

**Closing (4):**

- [`ADR-046-reset-and-cutover`](ADR-046-reset-and-cutover.md)
- [`ADR-046-feasibility-and-spikes`](ADR-046-feasibility-and-spikes.md)
- [`ADR-046-validation-and-delivery`](ADR-046-validation-and-delivery.md)
- [`ADR-046-streamline`](ADR-046-streamline.md)

**Provider dossiers (27)** — one dossier per installed `Provider/<name>`
resource, indexed with owned/exported ResourceTypes and component placement in
[`providers/README.md`](providers/README.md).

### Generated manifests

`ADR-046-spec-set.json` and `ADR-046-work-items.json` are deterministically
generated indexes, regenerated from the member Markdown and not themselves
members of the set.

- `ADR-046-spec-set.json` (`artifactKind: d2b-adr-spec-set`, `schemaVersion` 1)
  binds the exact 55 member files: for each member, its `specId`, `path`,
  `status`, `version`, resolved `dependsOn` edges (the `ADR-046-provider-*`
  dependency glob is expanded to every Provider dossier), `supersedes`, and the
  lowercase SHA-256 of the exact Markdown bytes. It records the parent path and
  the `v3` baseline commit and carries no timestamp or host path.
- `ADR-046-work-items.json` (`artifactKind: d2b-adr-work-items`, `schemaVersion`
  1) enumerates every implementation work item extracted from the member specs,
  sorted by `workItemId`, each bound to its `specId` and `specPath`. Every
  canonical required field is nonempty; `reuseSource` is `null` when a spec
  declares no reuse source. Work-item IDs are unique across the whole set.
- `ADR-046-implementation-graph.json` (`artifactKind:
  d2b-adr-implementation-graph`, `schemaVersion` 1) and its rendered human view
  `ADR-046-implementation-graph.md` are the D095 machine-readable
  implementation DAG. They map every one of the 55 member specs and every work
  item exactly once to a `W0`–`W7` launch wave and a file-disjoint parallel
  group, with typed edges, owner/destinations, entry contracts, prerequisites,
  blockers, exit gate, and topological rank. They are generated from the two
  manifests above plus the 8-wave topology in
  [`ADR-046-validation-and-delivery.md` §3](ADR-046-validation-and-delivery.md),
  are deterministic (no timestamps/host paths), and are **not** members of the
  set. They do not change the 55-member count. See
  [`ADR-046-implementation-graph.md`](ADR-046-implementation-graph.md).

## Required metadata

Each spec starts with this table:

| Field | Meaning |
| --- | --- |
| Spec ID | Stable `ADR-046-<spec-name>` identifier |
| Parent | `ADR 0046` |
| Status | `Proposed` or `Accepted` |
| Version | Monotonic integer |
| Baseline | Exact v3 commit analyzed |
| Normative | `Yes` |
| Owners | Contract and future implementation owners |
| Depends on | Exact ADR 0046 spec IDs |
| Supersedes | Prior spec/version, if any |

## Evidence and current-code fit

Current behavior is cited by exact v3 file, symbol, and baseline commit. Every
claim is classified as:

- `implemented-and-reachable`;
- `implemented-but-unwired`;
- `generated-or-eval-contract`;
- `test-only-or-preview`;
- `ADR-only`;
- `unknown-requires-spike`.

The protected pre-ADR45 v3 tree remains the sole current-state and ancestry
baseline. Main may be used freely as an implementation reuse source. A work
item that borrows from main must separately record:

- exact main commit, file, symbol, and tests;
- why that code is selected;
- which behavior is copied unchanged versus adapted;
- exact v3 destination and integration path;
- which surrounding ADR 0045 assumptions are deliberately excluded.

Borrowed main code never changes a v3 claim from ADR-only/unwired to
implemented-and-reachable.

Every design section ends with a current-code fit table:

| Item | Required content |
| --- | --- |
| Current anchor | Exact current source/artifact and evidence class |
| Behavior retained | Tested semantics preserved |
| Required delta | Behavior absent from v3 |
| Reuse path | Exact code/symbol copied, extracted, adapted, or wrapped |
| Replacement/deletion | Old owner removed only after a live successor |
| Feasibility proof | Existing proof or pre-acceptance spike |
| Future owner | Exact work item/crate/component |

Every ResourceType/Provider spec also contains a **Nix authoring and
configuration cleanup** section:

- user-facing direct schema mirror:
  `d2b.zones.<zone>.resources.<name> = { type = "..."; spec = { ... }; };`;
- exact canonical rendered ResourceSpec JSON;
- generated/committed ResourceTypeSchema and Provider settings schema;
- Nix eval/build validation of fields, bounds, ResourceRefs, Provider presence,
  Host/Guest/domain policy, ownership, conflicts, and schema fingerprints;
- canonical sorted integrity-pinned per-Zone resource bundle/generation;
- activation/publication path;
- removed configured-resource cleanup, status, audit, and tests.

Nix does not define a second resource vocabulary. `spec` field names, nesting,
types, defaults, bounds, and Provider extensions match the canonical
ResourceTypeSchema directly. metadata.name, metadata.zone, and apiVersion are
derived/defaulted. Users may author only metadata.ownerRef and bounded
presentation labels/annotations; status, UID, generation, revision, timestamps,
finalizers, managedBy, and configurationGeneration are core/controller-managed.
managedBy is `configuration`, `controller`, or `api`; API-managed resources
persist until explicitly deleted.

Nix derivations are the one value class that cannot appear in JSON
ResourceSpecs. They live in a separate named `d2b.artifacts.<id>` catalog.
ResourceSpecs use plain `artifactId`/`systemArtifactId` fields. Nix builds and
hashes each derivation and emits a private integrity-pinned ID-to-digest/closure
catalog; store paths never enter public spec/status/audit.

After a new generation activates, a previously Nix-owned resource absent from
the new configured set enters normal asynchronous finalizer-safe deletion. The
generation reports Degraded/pending-cleanup until deletion completes. Activation
does not block. Controller-created resources are not deleted merely because
they are absent from Nix; their owner controller governs them.

Every Provider dossier requires its crate's `src/`, `tests/`, `integration/`,
and `README.md` layout and assigns exact work items/files to each path.

## Resource terminology

A **resource** belongs to exactly one Zone.

A canonical resource reference is:

```text
<ResourceType>/<resource_name>
```

Examples:

```text
Zone/dev
Provider/system-core
Provider/system-systemd
Host/host-system
Guest/dev-vm
Process/wayland-proxy
User/alice
```

Rules:

- every field ending in `Ref` contains one canonical same-Zone resource
  reference;
- a plain enum or inline value never uses a `Ref` suffix;
- standard ResourceTypes use Zone-unique short names;
- vendor ResourceTypes use a qualified name such as `acme.io.Widget`;
- API binding rejects ResourceType collisions;
- cross-Zone resource references do not exist unless a later reviewed special
  case explicitly adds one.

Every resource has zero or one `ownerRef`. Any committed child mutation
triggers reconciliation of that owner through the reverse owner index.

ADR 0046 uses `ResourceType`, `ResourceTypeSchema`, and `ResourceSpec`
terminology. It does not expose Kubernetes-style ResourceKind/kind vocabulary.

Every resource has all three top-level objects:

- `metadata`, containing common name, Zone, UID, generation, revision,
  ownerRef/finalizer/deletion, and creation/update timestamps;
- `spec`, present as `{}` even when the ResourceType has no desired fields;
- `status`, present from creation and writable only through the authorized
  status subresource.

Common status contains numeric `observedGeneration`, phase
`Pending|Ready|Succeeded|Degraded|Failed|Deleted|Unknown`, bounded
conditions, RFC 3339 transition/reconcile/start/completion datetimes, and the
latest outcome with stable code, optional exitCode, detailed bounded/redacted
message, and retryability. ResourceType-specific status extends that shape.
Prior status is available from the Zone revision log until compaction rather
than retained in an unbounded per-resource history.

After finalizers complete, deletion emits one `phase=Deleted` revision event
and removes the resource immediately. The revision log is the only deletion
history.

## Decision-required protocol

Evidence may determine one answer. If it does not, the author stops the affected
spec and records `decision-required` rather than selecting an implicit default.
The decision entry contains:

- one focused question;
- the viable options and consequences;
- current-code evidence;
- a recommendation;
- blocked specs and work items.

The resolved choice is recorded in
[`ADR-046-decision-register.md`](ADR-046-decision-register.md) and propagated to
all affected specs. The set cannot enter pre-panel review with unresolved
decisions, placeholders, or implementation-defined behavior.

## Implementation work items

Each spec contains an **Implementation work items** section. Every item has:

| Field | Requirement |
| --- | --- |
| Work item ID | Stable `ADR046-<spec>-<ordinal>` ID |
| Dependency/owner | Prerequisites, future wave, crate/component, shared owner |
| Current source | Exact v3 paths, symbols, call sites, artifacts, and tests |
| Reuse source | Optional exact main commit/paths/symbols/tests used for copy/adaptation |
| Reuse action | `copy-unchanged`, `extract`, `adapt`, `wrap`, `replace`, or `delete-after-cutover` |
| Destination | Exact future crate/module/file and binary targets |
| Detailed design | Types, APIs, algorithms, state, limits, errors, security |
| Integration | Complete producer-to-consumer call/resource/process chain |
| Data migration | State/config/artifact/reset behavior |
| Validation | Exact test files/selectors and measurable acceptance |
| Removal proof | Live successor path and tests required before deletion |

Broad items such as “update d2bd” are invalid. A work item that copies current
behavior names the exact source symbols, the tests that move with them, the
adaptations required, and the exact point where callers switch.

## Parallel authoring

After the shared foundation is stable, file-disjoint specs are authored in
parallel. An agent owns only its assigned spec files and session evidence.
Shared parent/index/manifest/changelog/instruction files remain integrator-owned.
Agents return completed specs or `decision-required`; they do not make
cross-cutting choices or write implementation code.
