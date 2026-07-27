# ADR 0046 terminology and identities

| Field | Value |
| --- | --- |
| Spec ID | `ADR-046-terminology-and-identities` |
| Parent | ADR 0046 |
| Status | Accepted |
| Version | 1 |
| Baseline | `b5ddbed67867d9244bf33390868101bd9b053e49` |
| Normative | Yes |
| Owners | `d2b-contracts`, Zone runtime, Nix resource compiler |
| Depends on | `ADR-046-decision-register` |
| Supersedes | Public Realm terminology selected by ADR 0043 for d2b v3 |

## Purpose

This spec defines the names, identities, references, and scoping rules used by
every ADR 0046 specification.

## Terms

### Zone

A Zone is the d2b 3.0 resource, policy, authorization, routing, state, and audit
boundary. Every resource belongs to exactly one Zone and is stored in that
Zone's redb database.

Every Zone store contains exactly one authoritative:

```text
Zone/<zone-name>
```

The resource's metadata.zone equals `<zone-name>`. Each non-root child stores
one child-local `ZoneLink/<name>` uplink. Its compiler-only `parentZone`
setting selects the parent allocator and is never emitted as a resource.
Parent access uses the child Zone API; resources and ordinary refs are not
copied across Zones.

`Realm` remains current v3 baseline terminology and migration evidence. New d2b
3.0 public schemas, CLI, APIs, errors, and docs use `Zone`.

### ResourceType and ResourceSpec

A ResourceType is a Zone-bound schema/controller contract. A ResourceSpec is
one resource's desired-state object under that type.

The public envelope field is `type`. ADR 0046 does not use Kubernetes
ResourceKind/kind terminology.

Standard ResourceTypes use a short Zone-unique name:

```text
Host
Guest
Process
Volume
```

Vendor ResourceTypes use a qualified name:

```text
acme.d2bus.org.Widget
```

ResourceApiBinding rejects type-name collisions. A type cannot be selected by
an ambiguous short name.

### Resource reference

Every field ending in `Ref` contains:

```text
<ResourceType>/<resource_name>
```

Examples:

```text
Zone/dev
Provider/system-core
Host/host-system
Process/wayland-proxy
User/alice
Volume/work-state
```

Rules:

- refs resolve only in the caller/resource's Zone;
- ResourceType and name are both required;
- `resource_name` matches `^[a-z][a-z0-9-]*$`;
- the serialized form has no scheme, Zone prefix, query, fragment, relative
  segment, or implicit default type;
- a plain enum/inline value never has a `Ref` suffix;
- a ref resolves both canonical type/name and the target's immutable UID;
- a deleted/recreated object with the same type/name has a different UID and
  does not silently inherit old ownership/operation state.

Cross-Zone ResourceRefs do not exist in the initial contract. A future need is
a decision-required architecture change.

### Provider

A Provider is installed as:

```text
Provider/<name>
```

Its resource binds exact package/config/schema/controller/service/process/
state/trust generations and status. A providerRef cannot resolve merely because
a package exists in the Nix store or catalog; the Provider resource must exist
and be Ready in the Zone.

One Provider maps to one independently buildable crate/package. It may contain
several separately sandboxed process binaries.

### Host

A Host is a physical/local host execution, policy, and budget parent:

```text
Host/<name>
```

Provider/system-core reconciles Host and local User. A Zone may define several
Hosts for separate system/user policy and budgets.

### Guest

A Guest is a non-host VM, sandbox, cloud, or remote execution parent:

```text
Guest/<name>
```

Each Guest selects an installed runtime Provider such as Cloud Hypervisor,
QEMU, ACA, or Azure VM.

### ExecutionPolicy

Host and Guest share:

- `providerRef`;
- `defaultDomain: system|user`;
- `allowedDomains`;
- `defaultUserRef` when `allowedDomains` contains `user`.
- budgets;
- Volume/Network/Device attachment defaults.

### Process domain

Process and EphemeralProcess placement consists of:

- required `executionRef`;
- optional `domain: system|user`, defaulting from the referenced Host/Guest;
- `userRef` when user domain does not inherit its default.

executionRef must resolve to Host or Guest. Remote/nested are Guest Provider/
Zone properties, not Process domains or duplicate Process ResourceTypes.

### Process implementation

Process and EphemeralProcess use `providerRef` to select an installed
implementation such as:

```text
Provider/system-systemd
Provider/system-minijail
```

Both implement one common execution schema and mandatory local pidfd
conformance. The pidfd is local ephemeral controller authority, not a resource
or ref.

### AuthenticatedSubjectContext

The shared identity/authorization seam is:

```text
AuthenticatedSubjectContext {
  subjectRef
  subjectUid
  zoneRef
  evidenceClass
  executionRef?
  providerRef?
  processRef?
  controllerGeneration?
  providerGeneration?
  sessionPurpose
  service
  schemaFingerprint
  transportBinding
  reconnectGeneration
  transcriptHash
}
```

This spec owns the field contract. ComponentSession owns mapping trusted
Unix/Noise/bootstrap/vsock/enrollment evidence into it. The resource API and
d2b-bus consume it when building native authorization attributes. Peers cannot
self-assert or mutate it.

### Owner

Each resource has zero or one `metadata.ownerRef`. It is same-Zone and resolves
to canonical ref plus immutable UID.

Ownership means:

- a child mutation triggers owner reconciliation;
- owner deletion orders child finalization first;
- owner cycles fail at commit;
- unrelated dependencies use ordinary typed refs, not ownerRef.

### Generation, revision, and UID

- `metadata.uid` is immutable store-generated identity.
- `metadata.generation` starts at 1 and increments only on spec change.
- `metadata.revision` is the opaque Zone-local optimistic concurrency/watch
  token of the resource's latest committed mutation.
- `status.observedGeneration` is the latest spec generation the controller has
  observed and accounted for.

None is a timestamp. Human-readable generation names are not part of the common
contract.

### Time

Persistent datetimes are exactly `YYYY-MM-DDTHH:MM:SS.sssZ`: 24 ASCII bytes,
UTC, uppercase `T` and `Z`, and exactly three fractional digits. Other RFC 3339
spellings, offsets, leap seconds, and fractional widths fail closed. Wall time
never extends an already-admitted monotonic deadline. Common metadata/status
times are described in
[`ADR-046-resource-object-model`](ADR-046-resource-object-model.md).

## Canonical process identities

Fixed Zone bootstrap process:

```text
z-<zone-id>@<process-name>
```

System-domain Host/Guest Process:

```text
s-<execution-id>@<process-name>
```

User-domain Host/Guest Process:

```text
u-<execution-id>-<user-id>@<process-name>
```

The logical name is diagnostic only. Adoption also verifies Provider,
component/template, executable/config/sandbox generations, executionRef/domain,
cgroup/scope, and provider-specific process identity.

## Current-code fit

| Item | Treatment |
| --- | --- |
| Current anchor | `d2b-realm-core/src/ids.rs`, `realm.rs`, `target.rs`, `workload.rs`; `nixos-modules/options-realms*.nix`; `index.nix` |
| Evidence class | Current Realm/Workload IDs are implemented-and-reachable; Zone/Host/Guest/ResourceRef/Provider resources are ADR-only |
| Behavior retained | Bounded fail-closed IDs, canonical target parsing, opaque token redaction, stable current Workload identity |
| Required delta | Zone term/type, universal ResourceRef, UID/generation/revision, Host/Guest split, Process domains |
| Reuse path | Adapt current ID validators/serde/redaction; map current Workload/Realm only where evidence says reachable |
| Replacement/deletion | Realm public types/options remain until the v3 cutover work item supplies Zone successors |
| Feasibility proof | Golden ref/ID vectors shared by Rust/Nix/other SDKs; collision and UID-recreate tests |
| Future owner | Work items below |

## Implementation work items

### ADR046-identities-001

| Field | Value |
| --- | --- |
| Dependency/owner | W0 shared contract root; `d2b-contracts` |
| Current source | `packages/d2b-realm-core/src/ids.rs`, `realm.rs`, `target.rs`, `workload.rs` |
| Reuse action | adapt |
| Destination | `packages/d2b-contracts/src/v3/identity.rs`, `packages/d2b-contracts/src/v3/resource_ref.rs` |
| Detailed design | Add ZoneId, ResourceTypeName, ResourceName, ResourceUid, ResourceRef, generation/revision newtypes, exact parsing/serde/Debug/redaction, and golden vectors. `ResourceUid` is store-generated canonical lowercase UUIDv4 only. ResourceType uses the exact standard/qualified 63-byte segment and 137-byte total bounds; ResourceRef is bounded to 201 bytes. Define `AuthenticatedSubjectContext` and its validated component newtypes/enums exactly as frozen in D109: no `Deserialize`, no public field mutation, whole-struct redacted `Debug`, four closed evidence classes, bounded `SessionPurpose`/`ServiceName`, typed locality/binding digest, nonzero reconnect/controller generations, and redacted transcript hash. Primary reuse disposition: `adapt`. Preserved source-plan detail: extract and adapt. |
| Integration | Resource API/store/controllers/SDK/Nix import only these canonical types |
| Data migration | Destructive d2b 3.0 reset; no RealmRef parser compatibility |
| Validation | Rust property/vector tests; pure-Nix vector parity; malformed/collision/UID-recreate tests; UUIDv4 canonical-form and CSPRNG failure vectors; exact ResourceType/ResourceRef bounds; `AuthenticatedSubjectContext` no-Deserialize/no-public-mutation and redacted-Debug policy tests |
| Removal proof | Old public Realm target types removed only after all v3 callers consume Zone/ResourceRef |
| Implementation state | Merged |
| Evidence | Both destinations are present: `packages/d2b-contracts/src/v3/identity.rs` and `packages/d2b-contracts/src/v3/resource_ref.rs`, with their inline contract/vector tests. |

### ADR046-identities-002

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-identities-001; Nix integrator |
| Current source | `nixos-modules/options-realms.nix`, `options-realms-workloads.nix`, `index.nix` |
| Reuse action | adapt |
| Destination | `nixos-modules/options-zones.nix`, `nixos-modules/resources.nix`, `nixos-modules/index.nix` |
| Detailed design | Validate Zone names, ResourceTypes/names/refs, the shared Host/Guest ExecutionPolicy option shape, and canonical sorted resource identities. This work item defines no Rust ExecutionPolicy DTO and renders no Host or Guest resource. |
| Integration | Nix serializes only the W0-owned Zone names, ResourceType names, ResourceNames, ResourceRefs, and canonical `(type, name)` order; Rust-to-Nix ExecutionPolicy parity starts after the Rust type lands in ADR046-W2. |
| Data migration | Full reset and new Zone declarations |
| Validation | W0 nix-unit vectors for accepted and rejected option shapes; Rust-to-Nix rendered contract parity is deferred to ADR046-W2 |
| Removal proof | Realm-facing declarations removed only in the reset/purge wave |
| Implementation state | Merged |
| Evidence | All destinations are present: `nixos-modules/options-zones.nix`, `nixos-modules/resources.nix`, and `nixos-modules/index.nix`, with the W0 nix-unit option vectors. |
