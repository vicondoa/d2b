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
  `Accepted`; this is a documentation-only set;
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

- [`ADR-046-decision-register`](ADR-046-decision-register.md) - resolved
  decisions (through D118)
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

**Resource catalog (6)** - the 19 standard ResourceTypes (`Zone`, `ZoneLink`,
`Provider`, `Role`, `RoleBinding`, `Quota`, `EmergencyPolicy`, `Host`, `Guest`,
`Process`, `EphemeralProcess`, `Volume`, `Network`, `Device`, `User`,
`Credential`, `Endpoint`, `ResourceExport`, `ResourceImport`) have the
following exclusive ResourceType owners. Foundation specs define shared
contracts but do not co-own these types:

- [`ADR-046-resources-zone-control`](ADR-046-resources-zone-control.md) -
  `Zone`, `ZoneLink`, `Provider`, `Role`, `RoleBinding`, `Quota`,
  `EmergencyPolicy`, `ResourceExport`, `ResourceImport`
- [`ADR-046-resources-host-guest-process-user`](ADR-046-resources-host-guest-process-user.md) -
  `Host`, `Guest`, `Process`, `EphemeralProcess`, `User`, `Endpoint`
- [`ADR-046-resources-volume`](ADR-046-resources-volume.md) - `Volume`
- [`ADR-046-resources-network`](ADR-046-resources-network.md) - `Network`
- [`ADR-046-resources-device`](ADR-046-resources-device.md) - `Device`
- [`ADR-046-resources-credential`](ADR-046-resources-credential.md) - `Credential`

**Cross-cutting (3):**

- [`ADR-046-cli-and-operations`](ADR-046-cli-and-operations.md)
- [`ADR-046-telemetry-audit-and-support`](ADR-046-telemetry-audit-and-support.md)
- [`ADR-046-security-and-threat-model`](ADR-046-security-and-threat-model.md)

**Closing (4):**

- [`ADR-046-reset-and-cutover`](ADR-046-reset-and-cutover.md)
- [`ADR-046-feasibility-and-spikes`](ADR-046-feasibility-and-spikes.md)
- [`ADR-046-validation-and-delivery`](ADR-046-validation-and-delivery.md)
- [`ADR-046-streamline`](ADR-046-streamline.md)

**Provider dossiers (27)** - one dossier per installed `Provider/<name>`
resource, indexed with owned/exported ResourceTypes and component placement in
[`providers/README.md`](providers/README.md).

### Generated manifests

`ADR-046-spec-set.json` and `ADR-046-work-items.json` are deterministic indexes,
regenerated from the member Markdown and not themselves members of the set.
The checked-in generator exists: `packages/xtask/src/gen_spec_set.rs` emits both
manifests via `cargo run -p xtask -- spec-registry`, and
`packages/xtask/src/implementation_graph.rs` emits the implementation graph via
`cargo run -p xtask -- implementation-graph`. Both run under the fail-closed
`make test-drift` gate, which regenerates every ADR 0046 artifact and requires a
clean `git diff`. `ADR046-delivery-004` and `ADR046-delivery-009` own the
follow-on hardening of that generator and its fail-closed policy tests.

- `ADR-046-spec-set.json` (`artifactKind: d2b-adr-spec-set`, `schemaVersion` 3)
  binds the exact 55 member files: for each member, its `specId`, `path`,
  `status`, `version`, resolved `dependsOn` edges (the `ADR-046-provider-*`
  dependency glob is expanded to every Provider dossier), `supersedes`,
  sorted `workItemPrefixes` registry (an empty array for a member with no
  items), and the
  lowercase SHA-256 of the exact Markdown bytes. It records the parent path and
  the `v3` baseline commit and carries no timestamp or host path.
- `ADR-046-work-items.json` (`artifactKind: d2b-adr-work-items`, `schemaVersion`
  1) enumerates every implementation work item extracted from the member specs,
  sorted by `workItemId`, each bound to its `specId` and `specPath`. Every
  canonical required field is nonempty; `reuseSource` is `null` when a spec
  declares no reuse source. Work-item IDs satisfy the canonical ID contract
  below and are unique across the whole set. Generation fails closed on an
  incomplete, malformed, duplicate, or extra Markdown or manifest item.
- `ADR-046-implementation-graph.json` (`artifactKind:
  d2b-adr-implementation-graph`, `schemaVersion` 1) and its rendered human view
  `ADR-046-implementation-graph.md` are the D095 machine-readable
  implementation DAG. They map every one of the 55 member specs and every work
  item exactly once to a `W0`-`W7` launch wave and a file-disjoint parallel
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
- vendor ResourceTypes use a qualified name such as `acme.d2bus.org.Widget`;
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
| Work item ID | Declared by a level-three heading `### ADR046-<registered-prefix>-<ordinal>`, optionally followed by a title (see "Authoring shapes the registry tolerates"); an optional table row must match the ID exactly |
| Dependency/owner | Prerequisites, future wave, crate/component, shared owner |
| Current source | Exact v3 paths, symbols, call sites, artifacts, and tests |
| Reuse source | Optional exact main commit/paths/symbols/tests used for copy/adaptation; explicit `None` serializes as `null` |
| Reuse action | Exactly one canonical `reuseAction` value defined below |
| Destination | Exact future crate/module/file and binary targets |
| Detailed design | Types, APIs, algorithms, state, limits, errors, security |
| Integration | Complete producer-to-consumer call/resource/process chain |
| Data migration | State/config/artifact/reset behavior |
| Validation | Exact test files/selectors and measurable acceptance |
| Removal proof | Live successor path and tests required before deletion |

The exact work-item ID regex is
`^ADR046-[a-z0-9]+(?:-[a-z0-9]+)*-(?:00[1-9]|0[1-9][0-9]|[1-9][0-9]{2})$`.
`<registered-prefix>` is one entry in the owning member's
`workItemPrefixes` registry below; it is not required to equal the full Spec ID
suffix. `<ordinal>` is a three-digit value from `001` through `999`. A member
that owns work items registers a nonempty bytewise-sorted prefix list. Every
prefix is globally unique to exactly one member and is never inferred by
splitting an ID, Spec ID, or filename.

### Authoring shapes the registry tolerates

The generator parses the shapes the set actually uses, not an idealized
subset. An author does not have to normalize an existing spec to these rules,
but a tool that reads the set must handle all of them.

**The work-item heading may carry a title after the ID.** 356 headings are the
bare ID; the rest add a title introduced by a spaced hyphen (112), a colon
(51), or parentheses (24). All four forms declare the same ID.

```text
### ADR046-core-001
### ADR046-core-001 - Some title
### ADR046-core-001: Some title
### ADR046-core-001 (Some title)
```

Because a registered prefix may itself contain hyphens, ID extraction is
**anchored on the ID grammar and takes the shortest match**, never split on a
separator character. `ADR046-security-key-012 - Some title` yields
`ADR046-security-key-012`, and `ADR046-core-001-002` yields `ADR046-core-001`.
A parser that splits on the first hyphen, or that recognizes only one title
introducer, silently truncates or drops items.

**The enclosing section title varies.** `## Implementation work items` is the
majority spelling, but the set also uses `## Work items`, numbered forms such
as `## 17. Implementation work items`, and descriptive forms such as
`## Bus and ComponentSession reuse work items`. Anchor a scan on the work-item
heading pattern, not on the section title, and assert the expected total so a
miss fails closed rather than silently under-reporting.

**Table cells escape pipes as `\|`.** Unescape on read and re-escape on write,
or any field value containing a pipe is corrupted.

**Dependency cells reserve the word `through`.** It is a range-expansion
keyword: `ADR046-network-001 through ADR046-network-004` expands to four
dependency edges. Use `to`, or list the IDs, when a literal range is not
intended, and remember that a bare ordinal such as `004` carries no
`ADR046-` prefix and therefore does not parse as an ID at all.

| Normative member | Registered `workItemPrefixes` |
| --- | --- |
| `ADR-046-cli-and-operations` | `cli` |
| `ADR-046-components-processes-and-sandbox` | `process` |
| `ADR-046-componentsession-and-bus` | `bus`, `session` |
| `ADR-046-core-controllers` | `core` |
| `ADR-046-current-code-migration-map` | `[]` |
| `ADR-046-decision-register` | `decisions` |
| `ADR-046-feasibility-and-spikes` | `feasibility` |
| `ADR-046-nix-configuration` | `nix` |
| `ADR-046-primitive-resource-composition` | `primitives` |
| `ADR-046-provider-activation-nixos` | `activation` |
| `ADR-046-provider-audio-pipewire` | `audio` |
| `ADR-046-provider-clipboard-wayland` | `clipboard` |
| `ADR-046-provider-credential-entra` | `cred-entra` |
| `ADR-046-provider-credential-managed-identity` | `cred-mi`, `mi-topology` |
| `ADR-046-provider-credential-secret-service` | `cred-ss` |
| `ADR-046-provider-device-gpu` | `gpu` |
| `ADR-046-provider-device-security-key` | `security-key` |
| `ADR-046-provider-device-tpm` | `device-tpm` |
| `ADR-046-provider-device-usbip` | `usbip` |
| `ADR-046-provider-display-wayland` | `display` |
| `ADR-046-provider-model-and-packaging` | `provider` |
| `ADR-046-provider-network-local` | `nl` |
| `ADR-046-provider-notification-desktop` | `notify` |
| `ADR-046-provider-observability-otel` | `otel` |
| `ADR-046-provider-runtime-azure-container-apps` | `aca` |
| `ADR-046-provider-runtime-azure-virtual-machine` | `azure-vm` |
| `ADR-046-provider-runtime-cloud-hypervisor` | `ch` |
| `ADR-046-provider-runtime-qemu-media` | `qemu-media` |
| `ADR-046-provider-shell-terminal` | `sterm` |
| `ADR-046-provider-state` | `pstate` |
| `ADR-046-provider-system-core` | `system-core` |
| `ADR-046-provider-system-minijail` | `minijail` |
| `ADR-046-provider-system-systemd` | `systemd` |
| `ADR-046-provider-transport-azure-relay` | `transport-relay` |
| `ADR-046-provider-transport-unix` | `transport-unix` |
| `ADR-046-provider-transport-vsock` | `vsock` |
| `ADR-046-provider-volume-local` | `vl` |
| `ADR-046-provider-volume-virtiofs` | `vvfs`, `vvfs-export` |
| `ADR-046-reset-and-cutover` | `reset` |
| `ADR-046-resource-api-and-authorization` | `api` |
| `ADR-046-resource-object-model` | `object` |
| `ADR-046-resource-reconciliation` | `reconcile` |
| `ADR-046-resource-store-redb` | `store` |
| `ADR-046-resources-credential` | `credential` |
| `ADR-046-resources-device` | `device` |
| `ADR-046-resources-host-guest-process-user` | `exec`, `user-session` |
| `ADR-046-resources-network` | `network` |
| `ADR-046-resources-volume` | `volume` |
| `ADR-046-resources-zone-control` | `client`, `pkg`, `provider-agent`, `wire`, `zone-control` |
| `ADR-046-security-and-threat-model` | `security` |
| `ADR-046-streamline` | `streamline` |
| `ADR-046-telemetry-audit-and-support` | `audit`, `doctor`, `host-posture`, `reuse`, `telem` |
| `ADR-046-terminology-and-identities` | `identities` |
| `ADR-046-validation-and-delivery` | `delivery` |
| `ADR-046-zone-routing` | `routing` |

The registry also resolves the two formerly shared prefixes: `bus` belongs
only to `ADR-046-componentsession-and-bus`, so Nix integration items use
`nix`; `network` belongs only to `ADR-046-resources-network`, so
`ADR-046-provider-network-local` items use `nl`.

`reuseAction` is a closed scalar vocabulary:

| Value | Disposition |
| --- | --- |
| `create` | Implement a net-new destination without copying an implementation source; `reuseSource` is `null`. |
| `copy-unchanged` | Copy the named source while preserving its behavior and tests. |
| `extract` | Separate the named reusable subset into the destination without changing its behavior. |
| `adapt` | Reuse the named source with the contract or behavior changes stated in Detailed design. |
| `wrap` | Keep the named implementation intact behind a new adapter, port, or facade. |
| `replace` | Introduce and cut callers over to a named successor for an existing implementation or owner. |
| `delete-after-cutover` | Remove the named old implementation only after the referenced successor and cutover tests are live. |

Aliases, capitalization variants, free-form text, and compound values such as
`copy + adapt` are invalid; split work with different primary dispositions into
separate items.

Generation and validation fail closed unless every normative member's
Implementation work items section is complete:

- every work-item heading is exactly level three (`###`), matches the ID regex,
  uses one prefix registered by its owning member, and is unique across the
  set; `##` and `####` item declarations fail closed;
- every item has exactly one nonempty `Dependency/owner`, `Current source`,
  `Reuse action`, `Destination`, `Detailed design`, `Integration`,
  `Data migration`, `Validation`, and `Removal proof` field, with no duplicate
  fields; an optional `Work item ID` row exactly matches its heading and an
  optional `Reuse source` is nonempty;
- every heading prefix appears in the owning member's bytewise-sorted
  `workItemPrefixes`; every registered prefix belongs globally to exactly one
  member, and a member with no work items has an empty array in the generated
  spec set;
- an absent or explicit-none Reuse source serializes as `null`; `create`
  requires that null value;
- every Markdown item appears exactly once in `ADR-046-work-items.json`, every
  manifest item resolves to exactly one Markdown item, and the bound `specId`
  and `specPath` match the owning member; and
- manifest items are sorted bytewise by `workItemId`; any parse ambiguity,
  malformed `ADR046-` heading, count mismatch, or unconsumed item is an error.

Broad items such as “update d2bd” are invalid. A work item that copies current
behavior names the exact source symbols, the tests that move with them, the
adaptations required, and the exact point where callers switch.

## Parallel authoring

After the shared foundation is stable, file-disjoint specs are authored in
parallel. An agent owns only its assigned spec files and session evidence.
Shared parent/index/manifest/changelog/instruction files remain integrator-owned.
Agents return completed specs or `decision-required`; they do not make
cross-cutting choices or write implementation code.
