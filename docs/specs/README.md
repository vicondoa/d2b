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

- all files move from `Proposed` to `Accepted` together;
- a content change to any member invalidates validation and panel evidence for
  the set;
- no spec may silently override another spec;
- cross-spec dependencies must be acyclic;
- one spec owns each serialized contract, state machine, ResourceType,
  controller, Provider dossier, process model, and security invariant.

`ADR-046-spec-set.json` and `ADR-046-work-items.json` are generated indexes.
They bind the exact member files, versions, statuses, dependency edges, content
digests, and implementation work items. They are generated only after the
initial member set exists.

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
