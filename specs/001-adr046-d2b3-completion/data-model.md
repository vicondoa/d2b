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
| Generated artifact matches source | FR-031 | `make test-drift`, fail-closed |
| Capability with promised successor reaches parity | FR-041 | per-path removal proof + parity check |
| Capability without successor is listed and justified | FR-042 | explicit retirement list + release notes |

---

## 9. SC-002 Version 2 authority reference

This feature artifact is not the SC-002 protocol authority. The sole normative source is the
accepted Version 2 of
[`ADR-046-validation-and-delivery`](../../docs/specs/ADR-046-validation-and-delivery.md),
together with its generated schemas, fixtures, and generated traceability artifacts:
`docs/specs/ADR-046-validation-and-delivery-traceability.{json,md}`. The current external
specification is Version 1, so every consumer below remains blocked until Version 2 is
accepted, the generated artifacts are present, Gate 0 passes, and that commit is an ancestor
of the consumer base.

Version 2 and the generated traceability table MUST publish these stable identifiers:

| Identifier | Sole owned subject |
| --- | --- |
| `VD2-SC002-RECEIPT` | activation receipt, evidence-record reference, and close-stage validation |
| `VD2-SC002-PUBLICATION` | candidate-local publication, locking, retention, and crash recovery |
| `VD2-SC002-INCIDENT` | incident preimage, state, evidence, and redaction |
| `VD2-SC002-DISPOSITION` | successor freeze, authority request, signed disposition, apply, and admission |
| `VD2-SC002-RECOVERY` | inspectable states, emitted actions, exact invocations, exits, and convergence |
| `VD2-SC002-SOURCE-FLOOR` | installed source-generation compatibility evidence and capability consumption |
| `VD2-SC002-REGISTRIES` | independently authored fixture and poison registries with generated ownership traceability |
| `VD2-SC002-TRACEABILITY` | bijection from every identifier to schema, fixture, implementation owner, task, and gate |

The generated JSON is the machine authority and the generated Markdown is its review view.
Generation MUST fail on a missing, duplicate, extra, or ownerless identifier and drift gates
MUST compare both artifacts byte-for-byte. T589 implements only rows assigned to T589. T604
emits only its W6 acceptance evidence and T479 imports that exact-F6 result. T220 verifies
every generated Wave 5 row. No feature-local field list, count, digest recipe, state table,
fixture registry, or transition matrix may substitute for generated rows.

---

## 10. Installed source-floor evidence

`SourceGenerationCompatibilityFloorV1` is a stable type identifier, not a feature-local
schema. Its canonical encoding, fields, digests, signatures, capability rules, receipts,
fixtures, poison registries, and transitions are owned solely by accepted Version 2 through
`VD2-SC002-SOURCE-FLOOR`, `VD2-SC002-REGISTRIES`, and
`VD2-SC002-TRACEABILITY`.

The accepted external compatibility disposition names the producer/installer and typed
import/validation owners. T589 and T592 are read-only consumers of their generated
assignments and own no source-floor protocol. A missing, duplicate, stale, wrong-owner,
non-enforcing, non-ancestor, or failing generated row blocks dispatch with remediation to
accept Version 2, regenerate traceability, and pass Gate 0. No field list, digest recipe,
fixture census, registry count, or transition copy in this feature can satisfy that gate.

---

## 11. Retained Wave 5 request disposition

The retained Wave 5 binding request can be dispositioned only by an external
delivery-contract/tooling owner. That owner must land an accepted delivery-contract change
and its typed validator outside this feature before T219 may import anything. The external
record is evidence of allowed process, not panel sign-off and not a constitutional waiver.
No in-feature task produces, installs, or validates the authority.

`Wave5RetainedRequestDispositionV1` contains exactly:

| Field | Type and rule |
| --- | --- |
| `schemaVersion` | integer `1` |
| `kind` | literal `adr046w5-retained-request-disposition` |
| `program` | literal `ADR046` |
| `wave` | literal `adr046w5` |
| `authorityDispositionSha256` | digest of the accepted external authority artifact that authorizes this record; it is not a self-digest |
| `authorityCommitOid` | full commit object id of the accepted external delivery-contract change |
| `authorityTreeOid` | full tree object id of that change |
| `validatorArtifactSha256` | digest of the installed external typed validator |
| `deliveryContractVersion` | nonzero `u32` version accepted by the external change |
| `toolingVersion` | nonzero `u32` validator/tooling version accepted by the external change |
| `principleViAmendmentCommitOid` | full commit object id of the accepted FR-036 constitutional predecessor |
| `t072DispositionSha256` | digest of the one accepted T072 historical/current remedial disposition |
| `retainedCandidateId` | literal `d20267eec23f90b9cd6931e4bd322b66e259533849c8170617fbd002381493a4` |
| `retainedSnapshotSha256` | literal `7a04d9b86df6c8b8704b4bd79ddc25603fedae47d1a521f0b6fa420451816c3a` |
| `retainedRequestSha256` | digest of the retained binding request envelope |
| `retainedPanelRequestSha256` | digest of the byte-preserved `panel-request.json` |
| `retainedAttestationCount` | integer `0` |
| `retainedSealState` | literal `absent` |
| `finalCandidateId` | exact F candidate id |
| `finalCommitOid` | full commit object id of F |
| `finalTreeOid` | full tree object id of F |
| `closeAction` | one of `remain-blocked`, `abandon-without-merge`, or `recover-panel-without-new-request` |
| `requestPolicy` | literal `no-second-request` |
| `historyPolicy` | literal `preserve-retained-request-bytes` |
| `panelPolicy` | literal `unanimous-selected-roster-exact-final-candidate`, using the current thirteen-seat role domain and widen-only fix deltas |

The installed external validator is the sole import authority. It derives the Git and
delivery-state identities, verifies the authority and prerequisite commits are ancestors of
F, once-opens and hashes the retained request, compares every fixed field, rejects unknown
fields or enum values, and emits one immutable import result bound to the disposition digest
and F. A caller-supplied statement, feature-local receipt, phase-plan receipt, or self-named
authority is ineligible.

That `Wave5RetainedRequestDispositionImportV1` result contains exactly
`schemaVersion = 1`, `kind = "adr046w5-retained-request-disposition-import"`,
`recordSha256`, `authorityDispositionSha256`, `validatorArtifactSha256`,
`finalCandidateId`, `finalCommitOid`, `finalTreeOid`, `closeAction`, and
`verdict = "accepted"`. It is accepted only when every value equals the validated record;
`recordSha256` is computed over that complete record and is not stored inside it.

Disposition has these closed transitions:

```text
absent -> externally-accepted -> imported
imported + remain-blocked -> blocked
imported + abandon-without-merge -> abandoned-unmerged
imported + recover-panel-without-new-request -> panel-pending
panel-pending + exact unanimous selected-roster F-bound attestations -> panel-satisfied
panel-satisfied -> seal-eligible -> merge-eligible -> merged-byte-identical-F
panel-pending + completed/terminal panel with any missing role, recommendation,
  disagreement, or stale binding
  -> panel-refused
```

`blocked`, `abandoned-unmerged`, and `panel-refused` authorize no seal, merge, successor
wave, or release. `recover-panel-without-new-request` authorizes only the externally defined
recovery-attestation surface linked to the retained request; it creates no second request and
cannot itself satisfy the panel. The validator requires a candidate-bound selection from the
current thirteen-seat role domain, permits that selection only to widen over fix deltas,
requires `signoff = true` iff recommendations are empty, and requires identical
F/commit/tree/disposition bindings plus every constitutional predecessor. No action or field
can encode `waived`, partial, force, reduced roster, stale-candidate attestation, or panel
substitution. A content or history change after F, or any failed recovered panel, returns to
external escalation rather than admitting another feature-local request.
