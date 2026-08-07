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
| **Panel receipt** | one per role, 10 roles, 14 fields, pinned provider/model/reasoning effort | `signoff` true iff `recommendations` is empty |
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

## 9. Typed SC-002 delivery receipt

The delivery schema-v2 `EvidenceRecord` remains byte-for-byte unchanged and retains its v2
decoder. A passing `validation = "operator-nix-activation-cleanup"` record uses its existing
`locator` field to reference exactly one separately encoded `Sc002ActivationReceiptV1`. For
this validation only, the locator is a candidate-directory-relative content address with the
exact form `evidence-sidecars/sc002/sha256/<digest>.json`, where `<digest>` is the lowercase
64-hex SHA-256 of the exact encoded sidecar bytes. The locator and its digest component are
immutable. An absolute path, URL, query or fragment, empty or dot component, `..`, alternate
separator, noncanonical digest, or locator outside this namespace is malformed. No field,
enum variant, or version is added to `EvidenceRecord`, and no other validation may reference
this receipt type.

A failed `operator-nix-activation-cleanup` `EvidenceRecord` remains importable with no SC-002
receipt. It cannot satisfy the closed evidence profile, panel request, seal, or merge
eligibility. This design chooses no typed failure receipt: a failed record that references a
positive SC-002 receipt is malformed. Only a passing record must resolve the positive receipt
at import and every close-stage reopen.

The serialized object rejects unknown fields and has this closed shape:

| Field | Type and bound |
| --- | --- |
| `schemaVersion` | integer exactly `1` |
| `kind` | string exactly `sc002-activation-live` |
| `candidateId` | exact outer-record `candidate_id` |
| `contentId` | exact outer-record `content_id` |
| `snapshotSha256` | exact outer-record `snapshot_sha256` |
| `validation` | string exactly `operator-nix-activation-cleanup` |
| `outcome` | string exactly `passed` |
| `clock` | string exactly `CLOCK_MONOTONIC` |
| `startTickNs` | unsigned 64-bit monotonic tick |
| `samples` | exactly three `Sc002ResourceSampleV1` values in canonical Volume, Network, Device order |

Each sample contains `resourceIdentity`, `effect`, `ready`, `selectedStop`, `elapsedNs`, and
`progress`. `resourceIdentity` is one typed closed enum value:
`Volume/acceptance-state`, `Network/acceptance-net`, or `Device/acceptance-tpm`. `effect`,
`ready`, and `selectedStop` each repeat that identity and carry an unsigned 64-bit
`tickNs`; the effect and Ready identities MUST equal each other and the sample identity.
`selectedStop` also carries the closed source enum `effect` or `ready`.
`progress` contains 1-32 observations, each repeating the sample identity, carrying a
monotonic tick, and using only the closed kind enum `ingestion`, `commit`, `dispatch`,
`effect`, `status`, or `projection`.

The encoded receipt is at most 16,384 bytes; 16,384 is accepted and 16,385 is refused.
T604 produces the receipt as an external validation output; it does not place bytes in a
candidate directory. T600 supplies that file explicitly to
`wave validate-import --sc002-receipt PATH` while importing the
`operator-nix-activation-cleanup` record. That option is required exactly for a passing
record with this validation and forbidden for every other validation or failed result;
caller-supplied `--locator` is forbidden in the same invocation because the importer derives
the content address.

The importer opens the explicit source once with `O_RDONLY|O_CLOEXEC|O_NOFOLLOW`, requires a
regular single-link file owned by the current effective uid with mode exactly `0600`, reads at
most 16,385 bytes, hashes the exact opened bytes before decoding, and derives
`evidence-sidecars/sc002/sha256/<digest>.json`. It decodes and validates from those same
bytes, including the outer candidate/content/snapshot binding, before candidate publication.
Through the already held candidate-directory fd it creates and verifies current-effective-uid
`0700` namespace directories, creates a current-effective-uid `0600` temporary leaf with
`O_CREAT|O_EXCL|O_CLOEXEC|O_NOFOLLOW`, writes the exact validated bytes, `fsync`s the leaf,
and publishes with `renameat2(RENAME_NOREPLACE)`. Before publishing the `EvidenceRecord`
carrying the derived locator, it `fsync`s each held ancestor directory fd bottom-up:
`sha256`, `sc002`, `evidence-sidecars`, then the candidate directory. Namespace creation is
therefore durable, not only the final leaf rename. Before temp creation or recovery, every
importer and every cleanup worker acquires the same verified candidate-scoped exclusive OFD
write lock and retains it through publication or cleanup, parent `fsync`, the applicable
census, and `EvidenceRecord` publication or return. There is no second cleanup lock or
lock-free orphan path. A live importer owns that lock; a failed no-replace loser or restart
path cannot inspect, rename, or remove the live owner's temp. Restart cleanup begins only
after it acquires the released lock.

While holding the lock, cleanup may inspect only the reserved temporary and quarantine
namespaces through the held leaf-parent fd. It opens the candidate temp, requires a regular
single-link current-effective-uid `0600` leaf, records its device/inode, owner, mode, link
count, digest, and bytes, and atomically moves the name into a unique reserved quarantine
name with `renameat2(RENAME_NOREPLACE)`. It then reopens the quarantine leaf and requires all
recorded identity members to match before a final fd-relative name check and `unlinkat`.
Successful unlink is followed by parent `fsync`; before ordinary success or an ordinary
refusal returns, the still-held lock guards an exact empty census of both ephemeral reserved
namespaces. No joined path or broad sweep is allowed.

Identity ambiguity has one different, attainable terminal state. If the quarantine reopen or
the final pre-unlink name check does not prove the recorded inode, cleanup MUST NOT call
`unlinkat` and MUST NOT restore the suspect name to the temporary namespace. It atomically
moves the currently named suspect with `renameat2(RENAME_NOREPLACE)` into the durable
candidate-relative incident namespace
`evidence-sidecars/sc002/incidents/sha256/<incident-digest>.bin`, outside both ephemeral
reserved namespaces, then `fsync`s the incident directory and the old leaf parent. The
incident digest is a fixed domain-separated digest of the candidate binding and both observed
identity tuples; raw device/inode values never enter an error or observability surface. The
move empties the two ephemeral namespaces but intentionally leaves the durable incident
entry. That entry is not zero residue: it blocks `EvidenceRecord` publication and every
close stage, survives restart, and is never removed by automated cleanup. The fixed refusal
requires an operator incident disposition and a successor candidate. If the suspect name
cannot itself be moved without ambiguity, cleanup leaves it in place, records no success,
and blocks publication and close; it still never unlinks an unverified inode.

If a crash leaves the canonical sidecar durable before record publication, retry opens that
leaf through the same dirfd policy and accepts it only when type, effective-uid ownership,
mode `0600`, link count one, device/inode stability, digest, bytes, decode, and outer binding
all match; it never replaces the leaf. A different or malformed existing leaf refuses.
Crash injection covers source open, hash, decode, OFD-lock acquisition, temp write, file sync,
no-replace publication, each ancestor-directory sync, quarantine move/reopen,
the final pre-unlink identity check, race-loser/orphan unlink, incident move and both incident
directory syncs, cleanup-parent sync, ephemeral-residue census, and record publication.
Synchronized tests cover importer versus cleanup and cleanup versus cleanup for both the same
input and different inputs, temp replacement before quarantine move, quarantine replacement
before reopen, replacement immediately before unlink, same-bytes/same-record idempotence, and
different-bytes or wrong-binding races. An ordinary winner or loser leaves both ephemeral
namespaces empty. Every identity-ambiguous case instead proves no unverified inode was
unlinked, the durable incident entry or still-ambiguous name remains, restart redetects it,
and publication and close remain blocked.

At every durable reopen, the validator resolves the locator beneath the already held
candidate-directory fd with
`openat2(RESOLVE_BENEATH|RESOLVE_NO_SYMLINKS|RESOLVE_NO_MAGICLINKS|RESOLVE_NO_XDEV)`, opens
the leaf once with `O_RDONLY|O_CLOEXEC|O_NOFOLLOW`, requires a regular single-link file owned
by the current effective uid with mode exactly `0600` and stable device/inode identity,
hashes those exact bytes, and compares the hash with the locator digest before decoding from
the same opened fd. A replacement between lookup, hash, decode, or later-stage reopen is a
hard failure rather than a second read. The validator then requires exactly one sample for
each closed identity, every repeated identity to match its sample key, effect and Ready ticks
not earlier than start, selected stop to equal the later effect/Ready tick and its source
(`ready` wins an exact tie), `elapsedNs` to equal the checked stop-minus-start difference,
elapsed to be at most 2,000,000,000 ns, and every progress tick to fall in `(start, stop]`.
The receipt repeats and must match the unchanged outer `EvidenceRecord`
`candidate_id`/`content_id`/`snapshot_sha256` triplet, validation, and outcome;
the immutable snapshot resolves the commit/tree binding without adding either field to
`EvidenceRecord`. Reopening against another triplet is stale.
`EvidenceRecord`, receipt, and validation-error `Debug` output is fixed and redacted: it
exposes only type/version, sample count, and pass/refuse class, never ticks, identities, host
data, paths, commands, argv, or free-form text.

T589 owns the type and one validator invoked unchanged at evidence import, durable reopen,
panel-request/panel-attest, seal, and merge-eligibility. The negative census is closed:
passed record with a missing or duplicate receipt; receipt on a failed or wrong-validation
record; unknown/malformed version, kind, field, enum, locator, content digest, or size; absolute,
traversal, URL, symlink, hard-link, wrong-owner, non-`0600`, and replacement-race inputs;
caller-supplied locator with an SC-002 input; absent SC-002 input for a passed operator record;
SC-002 input for a failed or other-validation record; crash before and after every
sidecar/file-sync/directory-sync/record-publication boundary; same-name concurrent imports
with different bytes or bindings; missing,
duplicate, mixed, or unrelated resource samples; effect/Ready identity disagreement;
selected-stop/progress identity mismatch; arithmetic overflow or event misordering; stale
binding; zero progress; more than 32 progress observations; and any over-budget sample all
refuse. Missing ancestor sync, non-fd-relative cleanup, cleanup outside the reserved temp
namespace, missing or replaceable candidate OFD lock, cleanup against a live lock owner,
quarantine identity mismatch, cleanup-parent or incident-directory sync failure, unexpected
ephemeral residue after an ordinary success/refusal/race/restart, or any durable incident
entry also refuses. An identity mismatch instead passes only its negative oracle: no
unverified unlink, durable incident preservation, and publication/close denial.
Compatibility tests decode
retained schema-v2 `EvidenceRecord` fixtures
byte-identically, import a failed operator record without a receipt, and prove that the same
failed record remains ineligible for every close stage.

---

## 10. Installed source-floor evidence

The source-generation compatibility floor is external to this feature, but Wave 5 consumes
one closed evidence object. `SourceGenerationCompatibilityFloorV1` contains a nonempty exact
13-member census. Each member carries one accepted external disposition ID, source generation
identity, canonical artifact identity, and fixed content digest; no member is inferred from a
directory listing or prose claim.

| Member role | Exact member |
| --- | --- |
| `source-daemon-peer` | installed numeric-protocol-4 daemon peer |
| `source-broker-peer` | installed numeric-protocol-4 broker peer under the existing broker service |
| `source-wire-schema` | source handoff wire schema |
| `source-privilege-schema` | source handoff privilege schema |
| `source-operation-catalogue` | catalogue containing the source handoff row |
| `source-operation-catalogue-fingerprint` | exact `source-handoff-v1` `operation_catalogue_sha256` value |
| `source-compatibility-disposition` | accepted external compatibility disposition |
| `source-capability-api-fingerprint` | source capability/API fingerprint |
| `source-serialization-snapshot` | source serialization snapshot |
| `source-positive-fixture` | exact negotiated source-handoff positive fixture |
| `source-bare-protocol-negative-fixture` | bare protocol-4 refusal fixture |
| `source-cross-fingerprint-negative-fixture` | mismatched-peer catalogue refusal fixture |
| `source-installed-apply-object` | immutable broker-managed privileged apply object |

The set is exact: an absent, duplicate, extra, empty, stale-generation, stale-digest, or
cross-disposition member refuses before accepted-socket fd transfer, authorization, or
mutation. Both installed peers, all eleven remaining members, and the evidence object itself
must name the same disposition and source generation. T592 and all in-feature tasks consume
this object read-only; producing or repairing any member remains an external scope
escalation.
