# Phase 1 Data Model: Complete the ADR-046 Provider Control Plane (d2b 3.0)

**Feature**: `001-adr046-d2b3-completion` | **Date**: 2026-07-29

This document summarizes the resource object model the program must make live. It is a
**navigational summary, not a redefinition**. The normative source for every field, rule, and
state machine is the ADR-046 specification set; each section cites its owning spec. Where this
document and a spec disagree, the spec wins.

**Completeness**: this file deliberately does not enumerate every field of every type - that
lives in the specs and in the generated `docs/reference/schemas/v3/` bytes. For the coverage
proof that every one of the 55 specs and 545 work items is accounted for, and for the binding
rule that `detailedDesign` and `validation` text is carried verbatim rather than paraphrased,
see [spec-coverage.md](./spec-coverage.md).

---

## 1. Containment model

```text
Zone  (isolation, policy, routing, resource ownership, state, audit)
 |
 +-- exactly one embedded resource store (redb)
 +-- exactly one resource service (d2b.resource.v3)
 +-- exactly one authoritative Zone/<zone-name> self resource
 +-- exactly one fixed core-controller process
 +-- Zone-local Providers, Hosts, Guests, controllers, policies, and ordinary resources
 |
 +-- ZoneLink/<name>  ---> a child Zone, accessed only through the child's Zone API
```

Invariants that acceptance tests must hold:

- Every resource belongs to **exactly one** Zone.
- `Zone.spec` is **empty**. Zone-wide ceilings and emergency controls are separate `Quota` and
  `EmergencyPolicy` resources with their own controllers and status.
- Ordinary resource references **never** cross Zones. A parent reaches a child only through a
  local `ZoneLink` and the child's own API (FR-009, SC-008).

Owning specs: `ADR-046-resource-object-model`, `ADR-046-resources-zone-control`,
`ADR-046-zone-routing`.

---

## 2. Resource shape

Every resource shares one envelope:

| Element | Ownership | Notes |
| --- | --- | --- |
| `apiVersion`, `kind` | derived | Qualified ResourceType, `.d2bus.org.` infix |
| `name` | operator or controller | Zone-scoped; unique per `(Zone, kind, name)` |
| `spec` | **operator-declared** | Nix mirrors this shape directly |
| `status` | **controller-owned** | Never authored by an operator |
| revision | store-owned | Monotonic; the basis for conflict detection (FR-004) |
| owner references | controller-owned | Drives dependency-safe deletion (FR-005) |
| finalizers | controller-owned | Gate deletion until cleanup completes |

Only `name`, Zone, and `apiVersion` are derived or defaulted from the Nix authoring form;
everything else in `spec` mirrors the canonical ResourceSpec one-to-one.

Owning spec: `ADR-046-resource-object-model`. Reference fields: `ADR-046-terminology-and-identities`.

---

## 3. The 19 standard ResourceTypes

Grouped by their **exclusive** owning spec. Foundation specs define shared contracts but do
not co-own a type.

| Owning spec | ResourceTypes |
| --- | --- |
| `ADR-046-resources-zone-control` | `Zone`, `ZoneLink`, `Provider`, `Role`, `RoleBinding`, `Quota`, `EmergencyPolicy`, `ResourceExport`, `ResourceImport` |
| `ADR-046-resources-host-guest-process-user` | `Host`, `Guest`, `Process`, `EphemeralProcess`, `User`, `Endpoint` |
| `ADR-046-resources-volume` | `Volume` |
| `ADR-046-resources-network` | `Network` |
| `ADR-046-resources-device` | `Device` |
| `ADR-046-resources-credential` | `Credential` |

### Types most load-bearing for User Story 1

- **`Guest`** - a workload VM. Absorbs the retired `WorkloadPlacement` into
  `spec.providerRef` plus provider-specific `spec.*` fields and `ZoneLink` routing.
- **`Process`** / **`EphemeralProcess`** - long-lived and one-shot execution. `EphemeralProcess`
  replaces the retired `DurableExecutionProvider` trait entirely; there is no separate Provider
  family for it.
- **`Volume`** - carries `stateSchema`, `persistenceClass`, and `sensitivityClass`; the anchor
  for Provider-owned durable state, incident hold, and unclaimed-volume GC.
- **`Host`** - carries the explicit no-isolation posture for unsafe-local in `status`,
  conditions, CLI warnings, and audit. This value **must not** be silently dropped and **must
  not** become a telemetry label, span attribute, or log field.

---

## 4. Lifecycle and state transitions

```text
declared (Nix activation)
   |
   v
recorded  --- durable commit, revision assigned
   |
   v
reconciling  --- controller drives observed toward declared
   |            (waits, with a stated reason, on unready dependencies)
   v
ready  ------------------------> degraded  (specific cause + actionable next step)
   |                                |
   | operator removes declaration   | operator repairs or removes
   v                                v
retiring --- finalizers run, reverse-dependency order, progress visible
   |
   v
gone
```

Rules acceptance tests must enforce:

- An **effect is never released before its durable commit is proven** - across restart, abort,
  and conflict (FR-006, SC-007). The commit proof is single-use and consumed.
- Removing a declared resource activates the new generation immediately and requests
  asynchronous owner- and finalizer-safe deletion with visible Degraded cleanup status.
  Dynamic controller-owned resources are **not** broadly swept (FR-005).
- On restart the runtime **re-adopts** live resources; it does not recreate or destroy them
  (FR-003). Ambiguity is quarantined and reported as degraded, never resolved by cleanup.

Owning specs: `ADR-046-resource-reconciliation`, `ADR-046-core-controllers`,
`ADR-046-provider-state`.

### 4a. Mutation operation identity and expiry

Mutation recovery is Zone-scoped:

| Element | Representation | Rule |
| --- | --- | --- |
| operation key | `(ZoneUid, OperationId)` | The selected Zone is mandatory for mutation, retry, and inspection. No host-global operation-ID index or reservation exists. |
| `OperationId` | 16 UUIDv7-layout bytes, rendered as lowercase 32-hex without separators | Opaque to callers. The same bytes are valid as independent operation identities in different Zones. |
| replay binding | typed fixed digest over the registrar-derived subject, Zone, semantic request, target, verb, expected revision, operation ID, and idempotency data | A mismatch within the selected Zone is non-observing and never reapplies. |
| `expiresAt` | checked UUIDv7 issuance time plus the fixed 30-day operation recovery retention | The active or final operation record may be pruned only at this boundary. |
| expired lookup | typed `operation-expired` refusal derived from UUIDv7 time and the durable per-Zone clock | Inspection and mutation both deny. No post-expiry tombstone or host-global index is required, and pruning never turns the old ID into a fresh mutation. |

The per-Zone durable retention clock is monotonic across restart. A malformed, future,
expired, overflowed, or clock-discontinuous ID is denied before observation or mutation.
Concurrent use of one ID in two Zones may commit once in each Zone; same-Zone response loss
and restart return the original pending or final result without another mutation.

Owning specs: `ADR-046-resource-store-redb`, `ADR-046-cli-and-operations`,
`ADR-046-telemetry-audit-and-support`.

---

## 5. Access and authorization

| Entity | Role |
| --- | --- |
| **Component session** | The authenticated, single-owner association through which all resource access is admitted. Admission evidence is consumed into one owner and cannot be cloned or replayed. |
| **Subject** | Resolved **only** by the Zone registrar from verified peer evidence. A caller-supplied subject claim is refused - this is the boundary that relocated six times during W1 and is now sealed in the compiler. |
| **`Role` / `RoleBinding`** | Zone-local RBAC. Every operation is authorized before it executes; every denial is audited (FR-007). |
| **`Quota` / `EmergencyPolicy`** | Zone-wide ceilings and emergency controls as first-class resources with their own controllers and status. |

Relay or transport credentials authenticate transport only. They are **never** mapped to a
local lifecycle role; `SO_PEERCRED` plus `d2b` group membership remains the sole local
authorization surface.

Owning specs: `ADR-046-resource-api-and-authorization`, `ADR-046-componentsession-and-bus`.

---

## 6. Provider model

| Element | Meaning |
| --- | --- |
| **`Provider` resource** | The installed, supervised unit implementing one or more ResourceTypes |
| **Dossier** | The per-Provider normative spec; 27 exist, one per installed Provider |
| **Owned state** | Provider-owned durable state lives in a `Volume` with declared persistence and sensitivity class |
| **Effect port** | The only way a Provider causes a host effect. A Provider never receives a raw host path or unmediated privilege (FR-012, D077) |

Provider families for W6 parallelism: credentials; interaction; storage/network/device;
system/host/guest; transport/observability/activation.

Owning specs: `ADR-046-provider-model-and-packaging`, `ADR-046-provider-state`,
`docs/specs/providers/ADR-046-provider-*`.

---

## 7. Delivery entities

These are program-tracking entities, not runtime resources, but acceptance criteria reference
them (FR-025 through FR-045).

| Entity | Key attributes | State |
| --- | --- | --- |
| **Wave** | id `W0`-`W8`, member specs, parallel groups, entry and exit criteria | entered, snapshotted, panelled, sealed, merged |
| **Work item** | `workItemId`, owning `specId`, exact destination paths, validation obligations, `reuseAction` | `Planned` -> `Merged` (68 of 545 at receipt HEAD; 477 remain `Planned`) |
| **Candidate snapshot** | `candidate_id`, `content_id`, `snapshot_sha256`, base and head OIDs, expected pull requests | immutable; any content change invalidates it |
| **Panel receipt** | one current `PanelRecord` per selected seat from the current thirteen-seat role domain, 15 fields including required `panel_format_version`, with pinned provider/model/reasoning effort; legacy historical records have 14 fields and no `panel_format_version` | `signoff` true iff `recommendations` is empty; candidate-bound selection may only widen over fix deltas; current writers and validators require 15 fields while legacy readers retain the 14-field compatibility path |
| **Seal** | binds candidate, content, and snapshot digests after all lanes and the panel pass | requires every wave work item `Merged` |

Delivery state lives outside the repository at `$XDG_STATE_HOME/d2b/delivery` and the tooling
refuses any root inside a git working tree, which enforces FR-027's "never committed" rule
structurally rather than by convention.

Owning spec: `ADR-046-validation-and-delivery`.

---

## 8. Validation rules traceable to requirements

| Rule | Requirement | Where enforced |
| --- | --- | --- |
| Resource name and Zone qualification regex | FR-001 | Nix eval assertions, schema |
| Stale-revision write is refused | FR-004 | store conflict detection |
| Effect requires consumed commit proof | FR-006 | controller toolkit |
| Cross-Zone ordinary reference refused | FR-009 | resource API authorization |
| Caller-supplied subject refused | FR-008 | Zone registrar, compile-time seal |
| No secret, path, or PII in telemetry or audit | FR-018 | redaction policy lints |
| Mutation identity is `(Zone, operation_id)` and old IDs fail closed after bounded expiry | FR-070 | store/CLI restart, cross-Zone concurrency, response-loss, and expiry tests |
| Raw Zone/resource/operation/correlation/trace identity is absent from telemetry and audit | FR-070 | typed-digest, redaction, cardinality, and no-relabel tests |
| Generated artifact matches source | FR-031 | `make test-drift`, fail-closed |
| Capability with promised successor reaches parity | FR-041 | per-path removal proof + parity check |
| Capability without successor is listed and justified | FR-042 | explicit retirement list + release notes |

---

## 9. Current delivery and foundation authority

The committed entry tree contains Version 1 of
[`ADR-046-validation-and-delivery`](../../docs/specs/ADR-046-validation-and-delivery.md)
and no `ADR-046-validation-and-delivery-traceability.{json,md}` artifacts. This feature
therefore does not claim a Version 2 protocol, identifier registry, schema, fixture census, or
transition matrix. Production delivery tooling and the committed Version 1 contract govern
candidate snapshots, evidence, panels, and seals.

Fifteen incomplete obligations whose retained source rows carry a W5 label are modeled as
prospective W6 foundation adoption, not as historical state transitions:

| W6 foundation | Adopted subject |
| --- | --- |
| T607 | Zone/CLI/system-core Host/User/bootstrap control foundation |
| T608 | Volume, export/import projection, and Host-global authority foundation |
| T609 | Durable audit and bounded telemetry foundation |

The exact adopted work-item identities are machine-readable in `tasks.md`. Their retained W5
checkboxes and delivery bytes never change. T606 separately owns the shared-contract and
thirteen-crate scaffold prep that makes those foundation tasks file-disjoint.

---

## 10. Installed source-floor evidence

`SourceGenerationCompatibilityFloorV1` is a stable type identifier, not a feature-local
schema. The accepted `ADR-046-provider-activation-nixos` specification and its
`ADR046-activation-001` and `ADR046-activation-006` rows own the canonical handoff and
carrier. Code canon contains no production implementation, so those rows run prospectively
in W6 after T607 and T609. No feature-local field list, digest recipe, fixture census,
registry count, or transition copy substitutes.

---

## 11. Immutable Wave 5 historical predecessor

<!-- RETIRED-READONLY-BEGIN -->

The former actionable retained-request disposition model is superseded. Constitution 3.1.0
supplies only the generic historical-process disposition. This feature and the exact delivery
validator/tooling contract accept the following state only as closed ADR-046 history through merged Wave 5 commit
`177235ed37188b3be87525e7f016fb43401574c5`:

| Attribute | Exact accepted value |
| --- | --- |
| Delivery address | `adr046w5` |
| Candidate | `d20267eec23f90b9cd6931e4bd322b66e259533849c8170617fbd002381493a4` |
| Embedded snapshot identity | `7a04d9b86df6c8b8704b4bd79ddc25603fedae47d1a521f0b6fa420451816c3a` |
| `snapshot.json` SHA-256 | `dcf4d71a572bdf0766de557dde6b8ede7fd680eb9f85572238575d2ab5c82149` |
| Head | `19b77dad63060bcadd41f1ef800978d2c53cc030` |
| `panel-request.json` SHA-256 | `15f49657490410f0fb5530513144c7c2392f567b211eb630551f3110b94633f7` |
| Candidate root entries | exactly `evidence/`, `panel-request.json`, and `snapshot.json` |
| Evidence root | exactly `evidence/local-host/` |
| Evidence tree SHA-256 | `7deb84943d36962493422407ac74342fd598b2fea4970ea1a162942e25cfd33d` over the sorted `(local-host/<name>, file SHA-256)` manifest |
| Attestations | `0` |
| Seal | absent |

The exact `evidence/local-host/` inventory is:

| File | SHA-256 |
| --- | --- |
| `check-inventory.json` | `785c51649f64bde6f4eff74468527b0702d0fafadf8dc6dc25eaee6a19f429fe` |
| `redb-full-scale-proof.json` | `2a62cc326b790d4427e061ae0bd078fca5d612d33b6dcd2afc9f85cd9d2f843a` |
| `redb-rss-spike-observation.json` | `937a9dc9082d96bb0e3662ba3f25c8c81810251587c91537eb80e6c0e403f4db` |
| `resource-api-external-seals.json` | `9b3ce360d61d0494ec3ff677fc2527cf19d60cd190e40caf0d0f77dda14fef84` |
| `resource-store-redb-seal-tests.json` | `9b9190f5d6a504b77e575f5e897abb19845512ade5a6287f9ee3c7b0ef913a30` |
| `test-changelog.json` | `99be704ff0e630220b07d5d218e278646a4396dd04b16e0ad7b4e986ccdf4188` |
| `test-drift.json` | `56d057624da3c74a2bff79851a4e43a0b989b996963223031793984c2201c9b9` |
| `test-fixture-contracts.json` | `4a219d8ddb0f376a54072697c3e8cfa98d0918ea7d2eb0962f257407e73c6490` |
| `test-flake.json` | `1c308adf388ad1c23b1e9135ed2791eadbec124ac3b84e313fac9b428070614f` |
| `test-lint.json` | `ce4d6477c8e97817d34c4ca02f2ade7dd67f867f44b8d16aef207439e549db3c` |
| `test-nix-unit.json` | `d2d7c5ae80dd8208ea63a2b44e365065630a7a58ac5ae4ce5da62155482d7d79` |
| `test-policy.json` | `1a09ca7847d704fb0bcd5a44d11da91ce8ef1b1da2f8efddb9c6d789ac6835f4` |
| `test-proofs.json` | `7486df6fdca37b631273bf2258dc7881b9bf99148f2ca5cfd6425def77f74c0a` |
| `test-runtime-ledger.json` | `8af159ac146bc577d5d831b40ead290fcc74a78062c2d45cdde0011cb4d3c3ac` |
| `test-rust-api-surface.json` | `6a5c25058ae63e6e805a01055d9b69a75929832e8644b68ec7d2b62c00ba718a` |
| `test-rust.json` | `d5e5b5ca2074f347bd3ee18fa3d516f812769228373c048ba0875c5e93e4ea60` |
| `tier0.json` | `cfdf94766f0814b53389c9f1a07db8b582e32fb2e26aa194742014586ba76317` |

The Wave 6 production guard accepts no equivalent-looking substitute. It requires the exact
fetched `origin/v3` base, the merged Wave 5 boundary, and the unique integration commit on
that base's first-parent lineage after the boundary whose tree contains the exact accepted
generic Constitution 3.1.0 bytes.
It matches every root entry and digest above, rejects every extra or missing artifact, and is
rechecked at Wave 6 snapshot/entry, panel request, seal, and merge eligibility.

No transition leaves this historical state. There is no recovery action, second request,
retroactive attestation, reconstructed seal, replacement candidate, or import record.
Historical T219 records the disposition only. T221 consumes the production guard result before
the ordinary prospective Wave 6 plan panel. The guard provides process-integrity and signoff
tracking; it is not authentication and does not establish a security boundary.

<!-- RETIRED-READONLY-END -->
