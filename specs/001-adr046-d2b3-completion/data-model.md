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
after it acquires the released lock. The lock is one fixed candidate-relative regular
single-link current-effective-uid `0600` leaf opened with `O_CLOEXEC`. Every actor verifies
the same device/inode before taking `F_OFD_SETLK`; the leaf is never replaced, renamed, or
unlinked. A nonblocking lock attempt returning `EAGAIN` or `EACCES` proves a live owner and
requires zero namespace inspection or mutation before the caller returns the typed
`sc002-sidecar-owner-live` refusal.

While holding the lock, cleanup may inspect only the reserved temporary and quarantine
namespaces through the held leaf-parent fd. It opens the candidate temp, requires a regular
single-link current-effective-uid `0600` leaf, records its device/inode, owner, mode, link
count, digest, and bytes, and atomically moves the name into a unique reserved quarantine
name with `renameat2(RENAME_NOREPLACE)`. It then reopens the quarantine leaf. Cleanup never
calls `unlinkat` on a sidecar data leaf: Linux has no inode-qualified unlink, so a final
name check followed by `unlinkat` would retain a name/inode race.

If every recorded identity member matches, cleanup derives one `Sc002RetirementIdV1` as

```text
SHA-256(
  "d2b:sc002:retirement-id:v1\0" ||
  candidate_id[32] || content_id[32] || snapshot_sha256[32] ||
  content_digest[32] || u64be(st_dev) || u64be(st_ino)
)
```

where every named digest is decoded from its canonical lowercase 64-hex representation
before hashing. Only the resulting lowercase 64-hex retirement id is rendered; raw device
and inode values never enter an error, record, path component, or observability surface.
Cleanup moves the still-named leaf with `renameat2(RENAME_NOREPLACE)` into
`evidence-sidecars/sc002/retired/sha256/<content-digest>/<retirement-id>.bin`, reopens and
revalidates the same device/inode, owner, mode, link count, digest, bytes, candidate binding,
and path-derived retirement id, and `fsync`s the leaf plus both directory fds. Two
single-link orphan files with identical bytes have distinct device/inode identities and
therefore distinct retirement ids and durable names. A crash after the retirement rename is
recovered from the retired census; it is never recreated from a missing source name.

`EEXIST` at the retirement destination is not an idempotent-success signal while the source
name still exists. Cleanup opens and validates the existing destination without following
links, preserves both names, and moves the source to the incident path below with the fixed
`sc002-retirement-id-collision` refusal and
`Sc002IncidentKindV1 = "retirement-id-collision"`. The incident id binds the separately
observed source and existing-destination identity digests; it never treats the destination
as the source's second observation. Cleanup never overwrites, reuses, or unlinks either
leaf. The ordinary two-identical-orphan test must retire both leaves under different ids; a
forced retirement-id collision must take this incident transition, persist its typed status,
and block publication and close.

The retired census is bounded to at most 64 regular single-link current-effective-uid
`0600` leaves and at most 1,048,576 total encoded bytes beneath `retired/sha256`, with no
unknown directory level, leaf, or name. Every leaf must be at most 16,384 bytes and must
revalidate against both path digests and the candidate binding. Before adding a 65th leaf,
exceeding the byte bound, or accepting a malformed census, cleanup preserves the current
source through the incident transition and returns the fixed
`sc002-retirement-census-exhausted` or `sc002-retirement-census-invalid` refusal. It never
grows an unbounded retirement set. Exhaustion persists
`Sc002IncidentKindV1 = "retirement-census-exhausted"` and binds the verified source
identity, the valid pre-add census digest, and the current and prospective leaf/byte counts.
A malformed census persists
`Sc002IncidentKindV1 = "retirement-census-invalid"` and binds the verified source identity
and the bounded observed-census digest. Neither path invents a second identity tuple. A valid
retired orphan is immutable, non-authorizing residue and does not block retry or close.

The sole candidate-retention guard is T589's private
`packages/xtask/src/delivery/storage.rs::CandidateRetentionOwner`; no importer, restart
cleanup worker, panel stage, or generic filesystem sweep may mint or imitate it. It owns no
deletion authority. After acquiring the same exclusive candidate OFD lock, it performs one
zero-mutation whole-scope retention check. The full delivery-state census must prove all of
the following: the candidate is terminal `merged` or `abandoned-unmerged`; every request,
reservation, panel, seal, eligibility, and merge transition is terminal; every SC-002
incident is absent or `successor-admitted`; every retained external reference to the
candidate remains resolvable; both ephemeral namespaces are empty; and the receipt, retired,
incident, disposition, and status namespaces are exact, bounded, and valid. The guard also
proves that the candidate root remains at its canonical address and that all request,
reservation, panel-request, panel-record, evidence-record, receipt, seal, eligibility,
merge, incident, disposition, and status history, including authenticated absence where a
terminal state has no such artifact, remains immutable. The separately owned
`evidence-sidecars/sc002/retired` subtree retains every verified orphan under the bounded
census above. Neither that subtree nor any other candidate descendant is automatically
unlinked, and the candidate root is never renamed, tombstoned, or deleted. A failed predicate
or census performs zero mutation and returns `sc002-candidate-retention-blocked`. There is no
clock-driven, per-leaf, subtree, or candidate-root deletion.

Before ordinary success or an ordinary refusal returns, the still-held lock guards an exact
empty census of both ephemeral reserved namespaces and the bounded durable census above. No
joined path or broad sweep is allowed.

Identity ambiguity has one different, attainable terminal state. If the quarantine reopen or
the post-retirement reopen does not prove the recorded inode, cleanup MUST NOT restore the
suspect name to the temporary namespace or treat the retired name as verified. It records the
identity digest of the currently named leaf, derives the incident id, and atomically moves
that name with `renameat2(RENAME_NOREPLACE)` into the durable candidate-relative incident
payload namespace
`evidence-sidecars/sc002/incidents/payload/sha256/<incident-digest>.bin`, outside both
ephemeral reserved namespaces. It then reopens the payload and proves that the moved
device/inode, owner, mode, link count, digest, and bytes are the exact recorded current
identity before it can publish `parked` status. The rename is the name-consuming operation;
there is no check-then-`unlinkat` and no inode claim based only on the pre-rename lookup.
The incident digest is a fixed domain-separated digest of the candidate binding and both
observed identity tuples for
`Sc002IncidentKindV1 = "identity-ambiguity"`; raw device/inode values never enter an error
or observability surface.

A same-name replacement before the rename, a different identity after the rename, `ENOENT`,
or a nonidentical `EEXIST` is not a second terminal form. It leaves incident parking
`recovery-pending`, preserves every named leaf, publishes no `parked` status, and blocks
record publication and close. Restart recovery repeats only the exact metadata-bound move
under the same lock. The sole terminal mismatch result is a fully revalidated payload in
the durable incident namespace, with the old ephemeral source absent and both source and
incident directory updates durable. That state empties the two ephemeral namespaces but
intentionally leaves incident residue. It blocks `EvidenceRecord` publication and every
close stage, survives restart, and is never removed by automated cleanup. The fixed refusal
requires an operator incident disposition and a successor candidate. No SC-002 cleanup path
unlinks a sidecar data leaf.

`Sc002IncidentKindV1` is the closed enum
`retirement-id-collision | retirement-census-exhausted |
retirement-census-invalid | identity-ambiguity`. No other spelling is accepted. Let
`B = candidate_id[32] || content_id[32] || snapshot_sha256[32] ||
content_digest[32]`, with each digest decoded from canonical lowercase 64-hex. Let an
observed identity digest be

```text
I = SHA-256(
  "d2b:sc002:observed-identity:v1\0" ||
  u64be(st_dev) || u64be(st_ino) || u32be(st_uid) ||
  u32be(st_mode) || u64be(st_nlink) || observed_content_digest[32]
)
```

The bounded retired-census digest `C` is
`SHA-256("d2b:sc002:retired-census:v1\0" || u64be(canonical-census-length) ||
canonical-census)`. `canonical-census` is exactly `CanonicalRetiredCensusV1`; no JSON,
platform integer, errno, display path, or implementation iteration order enters these bytes.
Its complete byte grammar is:

```text
CanonicalRetiredCensusV1 =
    0x01 || 0x00 || u64be(record-count) || Record[record-count]
  | 0x01 || 0xff

Record =
  u32be(relative-path-length) || relative-path[relative-path-length] ||
  u8(entry-type) || u8(observation) || u8(failure) ||
  identity-digest[32] || u64be(encoded-size) || content-digest[32]
```

`0x01` is the census-grammar version. Body tag `0x00` means a complete bounded census.
The exact two bytes `0x01 0xff` are the sole over-bound representation; they carry no
partial records, count, reason, or host-dependent value. A normal empty census is exactly
`0x01 0x00` followed by eight zero bytes.

A relative path is the raw Unix name bytes below `retired/sha256`, with its one or two
components joined by the single byte `0x2f`; Unix forbids that byte inside a component.
The zero-length path is reserved solely for failure to enumerate `retired/sha256` itself.
Each valid content-digest directory is traversal structure and is not a record. Every
depth-two object is one terminal record. A depth-one object that is not a valid canonical
content-digest directory, an empty content-digest directory, an enumeration failure, or an
object at a forbidden depth is instead one terminal failure record. The validator never
follows a symlink. It sorts complete records by unsigned lexicographic comparison of
`relative-path`, with a shorter equal prefix first, before encoding them; duplicate paths
are malformed and never receive a tie-breaker.

The closed `entry-type` values are `0x00 = unavailable`, `0x01 = regular`,
`0x02 = directory`, `0x03 = symlink`, and `0x04 = other` (socket, FIFO, device, or any
remaining type). The closed `observation` values are `0x00 = unavailable` and
`0x01 = complete`. The closed `failure` values, selected by the first applicable step in
this listed validation order, are:

| Value | Meaning |
| --- | --- |
| `0x00` | no failure |
| `0x01` | invalid name, depth, or directory layout |
| `0x02` | no-follow metadata unavailable |
| `0x03` | invalid entry type |
| `0x04` | owner, mode, link count, or encoded-size metadata invalid |
| `0x05` | no-follow open unavailable |
| `0x06` | bounded content read unavailable |
| `0x07` | identity changed across observation |
| `0x08` | path digest, content digest, or candidate binding mismatch |
| `0x09` | directory enumeration unavailable |

Only two field combinations are legal. A stable fully read regular member uses
`entry-type = 0x01`, `observation = 0x01`, and either `failure = 0x00` or, when its complete
bytes expose a binding mismatch, `failure = 0x08`; it carries the `I` digest above, its exact
`u64be` encoded size, and the SHA-256 digest of those exact bytes. Every other failure uses
`observation = 0x00`, identity digest as 32 zero bytes, encoded size as eight `0xff` bytes,
and content digest as 32 zero bytes. That tuple is the sole unavailable representation and
cannot be confused with an available empty file. No other tag combination is accepted.

The observation charge is the terminal-record count plus the sum of exact encoded sizes of
complete readable regular members; structural digest directories add neither a record nor
content bytes. Before accepting a 65th terminal record or byte 1,048,577, the validator
stops without reading another member and replaces the entire would-be normal encoding with
the exact `0x01 0xff` sentinel. Thus traversal order cannot leak into an over-bound digest
and incident-id derivation cannot require an unbounded read. Prospective-add exhaustion
continues to digest the valid bounded pre-add census and binds the current/prospective
counts in the incident preimage below; an already over-bound observed tree uses the
sentinel digest and is malformed.

The incident digest is the stable `Sc002IncidentIdV1`: exactly 32 bytes rendered as 64
lowercase hexadecimal characters. It is derived by exactly one kind-specific preimage:

```text
retirement-id-collision =
  SHA-256(
    "d2b:sc002:incident-id:retirement-id-collision:v1\0" ||
    B || retirement_id[32] || source_I[32] || existing_destination_I[32]
  )

retirement-census-exhausted =
  SHA-256(
    "d2b:sc002:incident-id:retirement-census-exhausted:v1\0" ||
    B || source_I[32] || C[32] ||
    u64be(current_leaf_count) || u64be(current_encoded_bytes) ||
    u64be(prospective_leaf_count) || u64be(prospective_encoded_bytes)
  )

retirement-census-invalid =
  SHA-256(
    "d2b:sc002:incident-id:retirement-census-invalid:v1\0" ||
    B || source_I[32] || C[32]
  )

identity-ambiguity =
  SHA-256(
    "d2b:sc002:incident-id:identity-ambiguity:v1\0" ||
    B || u8(observation_stage) || before_I[32] || after_I[32]
  )
```

`observation_stage` is closed to `1 = quarantine-reopen` and
`2 = retirement-reopen`. Source/destination order, before/after order, kind domains, and
stage codes are not interchangeable. The census paths intentionally have one source
identity plus census evidence instead of a fabricated second identity. Raw device/inode,
uid, mode, link-count, name, and census bytes never enter an error or observability surface.

Every incident transition has one immutable `Sc002IncidentMetadataV1`. Its canonical JSON
rejects unknown, duplicate, missing, or reordered fields and records exactly these fields in
this order:

| Field | Type and rule |
| --- | --- |
| `schemaVersion` | integer `1` |
| `kind` | literal `sc002-incident-metadata` |
| `incidentKind` | exact `Sc002IncidentKindV1` |
| `incidentId` | lowercase 64-hex id recomputed from this metadata |
| `parkedCandidateId` | exact candidate id from `B` |
| `parkedContentId` | exact content id from `B` |
| `parkedSnapshotSha256` | exact snapshot digest from `B` |
| `contentDigest` | exact source content digest from `B` |
| `sourceLocator` | canonical candidate-relative source name opened before incident parking |
| `payloadLocator` | exact `incidents/payload/sha256/<incidentId>.bin` locator |
| `payloadIdentitySha256` | source identity for the three retirement kinds; `afterIdentitySha256` for identity ambiguity |
| `retirementId` | collision retirement id, otherwise null |
| `sourceIdentitySha256` | source identity for collision/census kinds, otherwise null |
| `existingDestinationIdentitySha256` | collision destination identity, otherwise null |
| `retiredCensusSha256` | census digest for census kinds, otherwise null |
| `currentLeafCount` | exhausted-census count, otherwise null |
| `currentEncodedBytes` | exhausted-census byte count, otherwise null |
| `prospectiveLeafCount` | exhausted-census count, otherwise null |
| `prospectiveEncodedBytes` | exhausted-census byte count, otherwise null |
| `observationStage` | identity-ambiguity stage `1` or `2`, otherwise null |
| `beforeIdentitySha256` | identity-ambiguity before digest, otherwise null |
| `afterIdentitySha256` | identity-ambiguity after digest, otherwise null |

Null is the only absent representation. The kind-specific non-null fields are exactly the
components of the corresponding incident-id preimage above; all other kind-specific fields
must be null. `sourceLocator` is accepted only from the closed temporary,
cleanup-quarantine, or retired namespaces, and it is never rendered in an error or CLI
projection. The metadata schema independently recomputes `B`, `I`, `C`, every count, stage,
and final incident id. The object is at most 8,192 bytes and uses canonical UTF-8 JSON with
no BOM, whitespace, or trailing newline, shortest ASCII string escaping, lowercase hex, and
unsigned base-10 integers with no leading zero. A metadata object that cannot reconstruct its
id or differs from decode-then-canonical-reencode is malformed.

The exact durable paths are:

```text
evidence-sidecars/sc002/incidents/metadata/sha256/<incident-id>.json
evidence-sidecars/sc002/incidents/payload/sha256/<incident-id>.bin
evidence-sidecars/sc002/incidents/status/sha256/<incident-id>/parked.json
evidence-sidecars/sc002/incidents/status/sha256/<incident-id>/disposition-validated.json
evidence-sidecars/sc002/incidents/status/sha256/<incident-id>/successor-admitted.json
```

Incident publication is one closed, idempotent protocol under the candidate lock:

1. Create-exclusively publish the canonical metadata, `fsync` the leaf, and `fsync` every
   held newly created or changed ancestor directory bottom-up through the candidate
   directory.
2. Move the metadata-bound source name to the payload with
   `renameat2(RENAME_NOREPLACE)`, `fsync` both the old source parent and payload parent, then
   reopen the payload and verify the exact metadata-bound identity, digest, and bytes.
3. Create-exclusively publish `parked.json`, `fsync` it, and `fsync` every changed status
   ancestor bottom-up through the candidate directory before returning the typed refusal.
4. Publish each later status transition as the next immutable state file with the same
   file-and-ancestor sync protocol. Never replace, truncate, or remove an earlier state.

An existing metadata, payload, or state path is idempotent success only after same-fd reopen
proves exact bytes and binding. Recovery accepts only these crash prefixes: metadata with
the original source still present and no payload; metadata plus the exact payload with the
source absent and no status; or a contiguous status prefix beginning at `parked`. It resumes
the next step and syncs every required ancestor. Source and payload both present, neither
present, status without metadata/payload, a skipped or duplicate state, nonidentical
`EEXIST`, or any identity change is `recovery-pending`, preserves all names, performs no
unlink, and blocks publication and close. This recovery rule applies at every create,
leaf-`fsync`, rename, old-parent-`fsync`, new-parent-`fsync`, ancestor-`fsync`, reopen, and
status-publication crash point for all four incident kinds.

Payload, metadata, the maximal contiguous status, and incident-id kind must agree one-to-one
on every restart and census. A missing, unknown, duplicate, cross-kind, noncontiguous, or
mismatched object blocks publication and close.
`tests/golden/delivery/sc002-incident-id-v1.json` contains exactly one canonical incident-id
vector for each of the four enum members and a `canonicalCensusV1` section with exactly three
byte vectors: normal-empty, normal-sorted-mixed (one complete member and one unavailable
member presented out of order), and over-bound (`01ff`). Every incident vector records the
decoded input components, identity and census sub-digests where applicable, exact
length-delimited preimage bytes, and expected lowercase incident id. Each census vector
records semantic inputs, exact canonical bytes, their framed length, and expected `C`.
Tests independently encode the census bytes, recompute every sub-digest and final id, and
reject version/body/tag substitution, incorrect framing or unsigned-byte ordering,
noncanonical unavailable fields, partial records attached to `01ff`, kind-domain
substitution, tuple-order substitution, stage substitution, omitted census evidence, and a
fabricated second census identity. Accepted `ADR-046-validation-and-delivery` Version 2 pins
this complete grammar, tag table, sentinel, ordering, the three census vectors, and all four
incident vectors before T589 dispatch.

`Sc002DispositionIdV1` has the same rendered form and is
`SHA-256("d2b:sc002:disposition-id:v1\0" || u64be(disposition-length) ||
canonical-authenticated-disposition)`. Each append-only immutable
`Sc002IncidentStatusV1` state record at the exact status path above records exactly these
fields in this order:
`schemaVersion = 1`, `kind = "sc002-incident-status"`, `incidentKind` as the exact
`Sc002IncidentKindV1` used to derive `incidentId`, `incidentId`, `parkedCandidateId`,
`parkedContentId`, `parkedSnapshotSha256`, `state`, `dispositionId`,
`successorCandidateId`, `successorContentId`, and `successorSnapshotSha256`.
`incidentKind` is immutable across every state transition. Each ID in the disposition and
successor positions is nullable but always present; the successor fields are either all null
or all non-null. Null is the sole absent representation; omission is malformed. Durable
status never contains a `remediation` field. The maximal valid contiguous state record is the
current durable state. Its closed transition is:

```text
parked -> disposition-validated -> successor-admitted
```

CLI JSON is deliberately a separate deterministic projection rather than the durable status
envelope. `Sc002IncidentCliStatusV1` records exactly the same fields in the same order except
that `kind` is the distinct literal `sc002-incident-cli-status`, followed by one final
required field `remediation`. Its closed values are `obtain-incident-disposition`,
`apply-incident-disposition`, `admit-successor`, and `none`. Projection reopens and validates
the durable status and disposition namespace under the same candidate lock, then selects:

| Durable state and census | `remediation` |
| --- | --- |
| `parked`, null disposition/successor IDs, no matching durable authenticated disposition | `obtain-incident-disposition` |
| `parked`, null disposition/successor IDs, exact matching durable authenticated disposition present after a publication-before-status crash | `apply-incident-disposition` |
| `disposition-validated`, non-null disposition ID, null successor triplet | `admit-successor` |
| `successor-admitted`, non-null disposition ID and successor triplet | `none` |

Every other state/field/census combination is malformed and produces no CLI status object.
The strict schemas are
`docs/reference/schemas/delivery/sc002-incident-metadata-v1.schema.json`,
`docs/reference/schemas/delivery/sc002-incident-status-v1.schema.json`, and
`docs/reference/schemas/delivery/sc002-incident-cli-status-v1.schema.json`. The status and
CLI JSON goldens prove that metadata reconstructs every incident id, statuses form only the
three append-only contiguous path prefixes, `remediation` is rejected from durable status,
required exactly once as the last CLI field, derived by the table rather than accepted from
a caller or stored status, and never free-form.

Human output is a separate exact line projection of the validated CLI status. It is always
these twelve newline-terminated lines in this order:

```text
incident-kind: <INCIDENT_KIND>
incident-id: <INCIDENT_ID>
parked-candidate-id: <PARKED_CANDIDATE_ID>
parked-content-id: <PARKED_CONTENT_ID>
parked-snapshot-sha256: <PARKED_SNAPSHOT_SHA256>
state: <STATE>
disposition-id: <DISPOSITION_ID_OR_NONE>
successor-candidate-id: <SUCCESSOR_CANDIDATE_ID_OR_NONE>
successor-content-id: <SUCCESSOR_CONTENT_ID_OR_NONE>
successor-snapshot-sha256: <SUCCESSOR_SNAPSHOT_SHA256_OR_NONE>
remediation: <REMEDIATION>
next-command: <NEXT_COMMAND>
```

The angle-bracket tokens describe bounded data fields; they are not printed literally.
Nullable IDs render exactly `none`. `NEXT_COMMAND` is selected only by `remediation`:
`obtain-incident-disposition` and `apply-incident-disposition` both render the static command
noun `sc002-incident-apply`, `admit-successor` renders
`sc002-successor-admit`, and `none` renders `none`. It never contains a path, flag, ID,
argument, shell fragment, executable path, or free-form sentence. The JSON projection
contains the same bounded values in the declared field order and no `nextCommand` or
guidance field.

`obtain-incident-disposition` directs the operator to the disposition authority/workflow
pinned by accepted Version 2; no repository command mints or self-signs that record.
`apply-incident-disposition` means replay `sc002-incident-apply` with the already obtained
record after a publication-before-status crash. `admit-successor` means invoke
`sc002-successor-admit` with the validated disposition id and fresh successor snapshot.

`Sc002IncidentDispositionV1` is the sole accepted disposition record. Its canonical JSON
object has exactly these fields in this order:

| Field | Type and rule |
| --- | --- |
| `schemaVersion` | integer `1` |
| `kind` | literal `sc002-incident-disposition` |
| `action` | literal `abandon-candidate-admit-successor` |
| `incidentId` | canonical `Sc002IncidentIdV1` |
| `parkedCandidateId` | lowercase 64-hex id equal to the parked status |
| `parkedContentId` | lowercase 64-hex id equal to the parked status |
| `parkedSnapshotSha256` | lowercase 64-hex digest equal to the parked status |
| `successorCandidateId` | lowercase 64-hex id distinct from the parked candidate id |
| `successorContentId` | lowercase 64-hex id for the fresh successor |
| `successorSnapshotSha256` | lowercase 64-hex digest for the fresh successor snapshot |
| `deliveryContractSpecSha256` | exact content digest for accepted `ADR-046-validation-and-delivery` Version 2 in the regenerated spec-set manifest |
| `authoritySha256` | exact disposition-authority digest pinned by that accepted Version 2 contract |
| `verificationKeySha256` | SHA-256 of the exact 32-byte Ed25519 public key pinned to that authority by the accepted Version 2 contract |
| `signatureEd25519` | lowercase 128-hex Ed25519 signature, final field |

The encoded record is canonical UTF-8 JSON with no BOM, whitespace, or trailing newline and
is at most 4,096 bytes. Fields occur only in the order above. Strings are ASCII with the
shortest JSON escaping, and integers use unsigned base-10 spelling with no leading zero.
Duplicate, missing, reordered, or unknown fields, alternate escapes, invalid UTF-8,
non-ASCII text, or bytes unequal to decode-then-canonical-reencode are malformed. The
signature covers

```text
"d2b:sc002:incident-disposition-signature:v1\0" ||
u64be(unsigned-canonical-length) ||
unsigned-canonical-object
```

where the unsigned object is the exact ordered object above with only the final
`signatureEd25519` field omitted. The validator first requires the Version 2 content digest,
authority digest, verification-key digest, and pinned key to match the accepted external
contract, then verifies the signature. A key or authority supplied by the disposition file
is never trusted as a selector.

`sc002-incident-apply` opens `--disposition PATH` exactly once with
`O_RDONLY|O_CLOEXEC|O_NOFOLLOW`, requires a regular single-link file owned by the current
effective uid with mode exactly `0600`, reads at most 4,097 bytes, hashes before decode, and
validates from those same bytes. T589 owns one private, nonserializable
`ValidatedSc002IncidentDisposition` result with no public constructor, fields, `Clone`,
`Copy`, `Default`, serde implementation, or conversion from the decoded DTO. Apply consumes
that result by value. While holding the candidate lock, it publishes the exact authenticated
bytes create-exclusively at
`evidence-sidecars/sc002/dispositions/sha256/<disposition-id>.json`, `fsync`s the leaf and
all created parent directories, and only then durably transitions status to
`disposition-validated` by publishing its append-only status file. An existing path is
accepted only when a same-fd reopen proves the exact bytes and binding; otherwise it is a
conflict. `sc002-successor-admit` reopens and revalidates those durable bytes through the same
validator before consuming the result and publishing `successor-admitted`. Each disposition
and status publication `fsync`s its leaf and every changed ancestor directory bottom-up
through the candidate directory. A crash before any required ancestor sync cannot advance
the maximal contiguous state; a crash after it replays the same transition idempotently.

The parked candidate remains permanently ineligible at every transition. A validated
disposition binds the incident, parked triplet, distinct successor triplet, accepted Version
2 contract digest, authority, key, and signature. It never authorizes unlink, incident
deletion, publication of the parked record, a binding panel request, reservation release,
or reuse of evidence. Successor admission requires a fresh snapshot with no copied SC-002
receipt, incident, retired, status, or disposition bytes. For `adr046w5`, it admits only
T220's nonbinding replacement-candidate and exact-candidate evidence path while preserving
the byte-identical retained request and reservation; T219's separate external
retained-request disposition remains mandatory. Any other wave whose binding request was
already consumed must stop for its external wave disposition rather than use this flow.

The strict schema is
`docs/reference/schemas/delivery/sc002-incident-disposition-v1.schema.json`. The canonical
signed fixture is
`tests/golden/delivery/sc002-incident-disposition-v1.json` and uses only a checked-in test
key; its accepted Version 2 contract digest, authority digest, key digest, unsigned signing
bytes, signature, and derived disposition id are asserted independently. Closed negatives
cover 4,097 bytes; unknown, missing, duplicate, and reordered fields; noncanonical JSON;
wrong version, kind, action, id width/case, incident, parked triplet, successor triplet, or
contract digest; equal parked/successor candidate ids; missing, unknown, copied, or wrong
authority/key; unsigned, malformed, wrong-domain, wrong-key, and post-signature-tampered
records; replay against another incident, candidate, content, snapshot, or contract
generation; replacement between open/hash/decode/publish/reopen; stale state; conflicting
same-id bytes; and copied SC-002 evidence in the successor. Every negative refuses before a
state transition, request, reservation release, incident deletion, or candidate admission.
The durable-status and CLI-status schemas plus human/JSON fixtures additionally exercise
every `Sc002IncidentKindV1`, require status `incidentKind` to match the id domain and durable
payload/locator, consume all four incident vectors and all three census vectors from
`tests/golden/delivery/sc002-incident-id-v1.json`, and cover every row of the deterministic
remediation table.

T589 owns the planned delivery CLI contract:
`wave sc002-incident-inspect --snapshot PATH --incident-id ID [--json]`,
`wave sc002-incident-apply --snapshot PATH --incident-id ID --disposition PATH [--json]`,
and
`wave sc002-successor-admit --snapshot PATH --incident-id ID --disposition-id ID
--successor-snapshot PATH [--json]`. Exit `0` means the requested read or transition
completed, exit `2` means invalid syntax or malformed input, exit `3` means the stable
incident/disposition/successor ID was not found, and exit `4` means stale state, conflict,
or blocked admission; no other stable exit is assigned. Reapplying the exact authenticated
disposition after `disposition-validated`, or readmitting the exact already admitted
successor, exits `0` after full durable revalidation and makes no write; a different
disposition, successor, or binding exits `4`. Human output is the exact twelve-line
projection above. JSON is the closed `Sc002IncidentCliStatusV1` projection above, not the
durable `Sc002IncidentStatusV1` envelope; its required final `remediation` field is derived
by the closed table and has no free-form counterpart. The original cleanup refusal and every
later refusal carry the same stable incident id as a bounded data field, so the operator can
invoke inspect without discovering an internal path. T589's focused parser,
state-transition, metadata/status-path, durable-status/CLI-schema, human/JSON golden,
disposition-schema/signature, exit, crash, stale-ID, replay, and no-request/no-unlink tests
own this contract. Its
existing `changelog.d/resource-api-production.md` fragment carries the operator-visible
delivery recovery entry, all three command nouns, the four exits, and the parked-candidate
successor requirement. Before T589 dispatch, a separate external specification-amendment
workflow must bump accepted `ADR-046-validation-and-delivery` from Version 1 to Version 2,
pin this complete command, census-byte/golden, durable-status/CLI-projection,
incident-metadata/status-path/publication/recovery, disposition-authority, canonical-record,
retention-owner, and validator contract, receive
the parent ADR's required pre-panel and post-panel approvals,
regenerate `ADR-046-spec-set.json`, `ADR-046-work-items.json`, and
`ADR-046-implementation-graph.{json,md}`, and pass Gate 0 on the exact amendment commit.
That commit must be an ancestor of T589's base. T589 does not own or edit that normative
amendment or its generated manifests; it implements the already accepted Version 2
contract. T220 later verifies generated help, schemas/goldens, and fragment fold without
substituting for this pre-T589 gate. This assigns future work only; these commands are not
claimed implemented at this planning base.

If a crash leaves the canonical sidecar durable before record publication, retry opens that
leaf through the same dirfd policy and accepts it only when type, effective-uid ownership,
mode `0600`, link count one, device/inode stability, digest, bytes, decode, and outer binding
all match; it never replaces the leaf. A different or malformed existing leaf refuses.
Crash injection covers source open, hash, decode, OFD-lock acquisition, temp write, file sync,
no-replace publication, each ancestor-directory sync, quarantine move/reopen, verified
retirement move/reopen, incident metadata publish and every ancestor sync, payload move,
old-parent sync, payload-parent sync, payload reopen, each append-only status publish and
every status ancestor sync, cleanup-parent sync,
ephemeral-residue census, and record publication.
Synchronized tests cover the complete importer/cleanup/retention actor matrix below,
temp replacement before quarantine move, quarantine replacement before reopen, replacement
before and after each retirement or incident move/reopen, same-bytes/same-record
idempotence, two identical orphan leaves retiring to distinct retirement ids, forced
retirement `EEXIST`, retirement-census exhaustion/corruption, and different-bytes or
wrong-binding races. Each overlap uses two independently opened descriptions of the same
verified lock inode and the named latch/owner orderings below. The loser must
receive the live-owner refusal before namespace access; after release, exactly one retry may
advance. Every case proves bounded completion with no deadlock, no sidecar-data unlink, and an
exact final census within 64 leaves and 1,048,576 bytes. An ordinary winner or loser leaves
both ephemeral namespaces empty.
Every identity-ambiguous terminal case instead proves the exact metadata/payload/parked-status
prefix is durable outside both ephemeral namespaces. A replacement-raced case remains
`recovery-pending` with every name preserved and no parked status until restart completes
that same protocol. Both states block publication and close.

The serialization oracle is the exact Cartesian product of actor pairs
`importer/cleanup`, `cleanup/cleanup`, `importer/retention-guard`, and
`cleanup/retention-guard`; same-input and different-input fixtures where the pair admits
both; first-actor and second-actor lock ownership; and latches at `temp-created`,
`temp-file-synced`, `quarantine-renamed`, `retirement-renamed`,
`incident-metadata-published`, `incident-payload-renamed`, and
`incident-status-published`. Every actor opens its own file description for the one verified
lock inode. A nonblocking contender must return `sc002-sidecar-owner-live` with
`namespace_open_count = 0`, `namespace_mutation_count = 0`, and
`critical_section_max = 1`; a blocking restart contender may enter only after release.
After release, exactly one retry linearizes after the winner. Tests assert the complete case
id set rather than counting dynamically generated cases, and each case records one of the
two allowed serial histories. No test may pass by timing out, skipping a latch, sharing an
open file description, or inspecting a live owner's namespace.

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
quarantine identity mismatch, cleanup-parent or incident metadata/payload/status leaf,
parent, or ancestor sync failure, unexpected ephemeral residue after an ordinary terminal,
or any durable incident entry also refuses. An identity mismatch instead passes only its
negative oracle: no sidecar-data unlink; either a complete durable
metadata/payload/parked-status terminal or a recovery-pending all-names-preserved prefix; and
publication/close denial in both states.
Compatibility tests decode
retained schema-v2 `EvidenceRecord` fixtures
byte-identically, import a failed operator record without a receipt, and prove that the same
failed record remains ineligible for every close stage.

---

## 10. Installed source-floor evidence

The source-generation compatibility floor is an unresolved external prerequisite, not a
Wave 5 implementation output. The accepted external disposition MUST name one concrete
source-generation compatibility producer/installer owner and one concrete import/validation
authority. The producer/installer owner emits the manifest and atomically installs the
manifested bytes in the source 3/1 generation. The import/validation authority is an
independently installed typed validator named by that same disposition; it validates the
installed generation and imports the accepted aggregate for the exact C/Q dispatch base.
Self-asserted owner names, prose, a directory census, a target-only fixture, or an in-feature
task are not authority. No task in this feature produces, installs, repairs, imports, or
accepts this floor. T589 is a read-only consumer at dispatch, and T592 remains a read-only
consumer during migration.

Every source-floor object uses one canonical UTF-8 JSON encoding. Objects contain fields in
the order listed here, arrays retain their stated canonical order, and output has no BOM,
whitespace, or trailing newline. Text is ASCII and uses the shortest JSON escaping; hex is
lowercase. Unsigned integers use base-10 digits with no sign, exponent, fraction, or leading
zero except the single digit `0`, and must fit their stated Rust width before conversion.
Duplicate, missing, reordered, or unknown fields; invalid UTF-8; non-ASCII text; alternate
escapes; and any byte sequence that differs from decode-then-canonical-reencode are refused.
The strict schemas set `additionalProperties: false` at every object level.

`SourceGenerationIdentityV1` is the canonical source-generation digest payload. It contains
exactly, in order, `schemaVersion = 1`,
`kind = "source-generation-identity"`, `baseSystemClosureNarSha256`,
`daemonContentSha256`, `brokerContentSha256`, `applyObjectContentSha256`,
`protocolVersion = 4`, and `operationCatalogueSha256`. Every digest field is lowercase
64-hex. The object contains no store path, derivation name, host identity, or mutable
generation number. `baseSystemClosureNarSha256` is
`SHA-256("d2b:source-floor:base-system-closure-nar:v1\0" ||
u64be(canonical-nar-length) || canonical-nar-bytes)` over the accepted source 3/1 system
closure before any compatibility-floor manifest or receipt is installed. Excluding those
objects prevents a digest from containing itself; their later installation is bound by the
installation and validation receipts.
`daemonContentSha256`, `brokerContentSha256`, `applyObjectContentSha256`, and
`operationCatalogueSha256` are exactly the member-content digests for
`source-daemon-peer`, `source-broker-peer`, `source-installed-apply-object`, and
`source-operation-catalogue`; they are not independently rehashed aliases.

Every source-floor digest uses
`SHA-256(domain || u64be(payload-length) || payload)`, where `domain` is the exact ASCII tag
including its terminating zero byte. There is no raw SHA-256, implicit serializer framing,
or untagged concatenation. The closed registry and payloads are:

| Identity or field | Exact domain tag | Exact payload |
| --- | --- | --- |
| accepted compatibility disposition | `d2b:source-floor:disposition:v1\0` | canonical accepted disposition bytes |
| `baseSystemClosureNarSha256` | `d2b:source-floor:base-system-closure-nar:v1\0` | exact canonical NAR bytes of the accepted source 3/1 system closure before compatibility-floor manifest/receipt installation |
| `sourceGenerationSha256` | `d2b:source-floor:source-generation:v1\0` | canonical `SourceGenerationIdentityV1` |
| producer authority | `d2b:source-floor:producer-authority:v1\0` | exact 32-byte pinned producer verification key |
| installer authority | `d2b:source-floor:installer-authority:v1\0` | exact 32-byte pinned installer verification key |
| import/validator authority | `d2b:source-floor:validator-authority:v1\0` | exact 32-byte pinned validator verification key |
| `verificationKeySha256` | `d2b:source-floor:verification-key:v1\0` | exact 32-byte Ed25519 verification key |
| member `contentSha256` | `d2b:source-floor:member-content:v1\0` | `u32be(role-length) || role || u32be(artifact-id-length) || artifactId || u64be(content-length) || exact installed bytes` |
| `manifestSha256` | `d2b:source-floor:manifest:v1\0` | canonical signed manifest object |
| `installationReceiptSha256` | `d2b:source-floor:installation-receipt:v1\0` | canonical signed installation receipt |
| `validationReceiptSha256` | `d2b:source-floor:validation-receipt:v1\0` | canonical signed validation receipt |
| import receipt content identity | `d2b:source-floor:import-receipt:v1\0` | canonical signed import receipt |
| `installedCensusSha256` / `validatedCensusSha256` | `d2b:source-floor:census:v1\0` | canonical full 13-member array |
| `validatedFloorSha256` | `d2b:source-floor:validated-floor:v1\0` | `u64be(manifest-length) || manifest || u64be(installation-length) || installation || u64be(validation-length) || validation` |
| aggregate typed floor identity | `d2b:source-floor:aggregate:v1\0` | canonical complete `SourceGenerationCompatibilityFloorV1` object |

Lengths count UTF-8 octets and are encoded before hashing. The role and artifact-id bytes in
member content framing are their exact canonical ASCII strings. The census payload is the
canonical full member array, not a concatenation of member hashes. The validated-floor
payload excludes the import receipt and therefore cannot contain itself. The aggregate
identity includes the import receipt and is bound by the private validated capability. No
digest uses serializer-dependent map order, pretty JSON, caller-supplied bytes, a platform
integer, or an unknown tag. A decoder encountering an unknown field, tag, schema version,
enum value, or trailing byte refuses; it never preserves or ignores extensions.

Issuer provenance is authenticated, not asserted by copying an authority digest. The
accepted external disposition pins one Ed25519 verification key to each producer, installer,
and import/validation authority. Each manifest or receipt ends with one closed
`SourceFloorIssuerProofV1` containing, in order, `authoritySha256`,
`verificationKeySha256`, and `signatureEd25519`; the key digest is 64 lowercase hex and the
64-byte signature is 128 lowercase hex. The proof signs
`signature-domain || u64be(unsigned-canonical-length) || unsigned-canonical-object`, where
the unsigned object is the same ordered object with only its final `issuerProof` field
omitted. The exact signature domains are
`d2b:source-floor:manifest-signature:v1\0`,
`d2b:source-floor:installation-signature:v1\0`,
`d2b:source-floor:validation-signature:v1\0`, and
`d2b:source-floor:import-signature:v1\0`. The authority and key must match the accepted
disposition before signature verification. Missing, copied, wrong-key, cross-transition,
or binding-stale proofs refuse before fd transfer, authorization, or mutation.

`SourceGenerationCompatibilityFloorV1` is one immutable aggregate with exactly these
top-level fields:

| Field | Type and rule |
| --- | --- |
| `schemaVersion` | integer `1` |
| `kind` | literal `source-generation-compatibility-floor` |
| `manifest` | one `SourceGenerationCompatibilityManifestV1` |
| `installationReceipt` | one `SourceGenerationCompatibilityInstallationV1` |
| `validationReceipt` | one `SourceGenerationCompatibilityValidationV1` |
| `importReceipt` | one `SourceGenerationCompatibilityImportV1` |

Every `*Sha256` field below is exactly 32 bytes rendered as 64 lowercase hexadecimal
characters. A receipt cannot authorize itself: each authority digest is compared with the
authority binding in the already accepted external disposition.

`SourceGenerationCompatibilityManifestV1` contains exactly:

| Field | Type and rule |
| --- | --- |
| `schemaVersion` | integer `1` |
| `kind` | literal `source-generation-compatibility-manifest` |
| `dispositionSha256` | digest of the accepted external compatibility disposition |
| `sourceGenerationSha256` | domain-separated digest of the exact source system closure and its installed daemon/broker generation |
| `producerAuthoritySha256` | disposition-pinned producer authority |
| `installerAuthoritySha256` | disposition-pinned installer authority |
| `importValidatorAuthoritySha256` | disposition-pinned typed import/validation authority |
| `memberCount` | integer `13` |
| `members` | exactly 13 `SourceGenerationCompatibilityMemberV1` values in the canonical order below |
| `issuerProof` | producer `SourceFloorIssuerProofV1`, final field |

Each `SourceGenerationCompatibilityMemberV1` has exactly `role`, `artifactId`,
`dispositionSha256`, `sourceGenerationSha256`, `byteLength`, and `contentSha256`.
`byteLength` is a canonical `u64`; zero is structurally encodable only so the authenticated
`empty` poison can reach the closed semantic refusal, and every accepted member requires
`1..=u64::MAX`. The two binding digests must equal the manifest values. `role` and
`artifactId` are bounded canonical ASCII strings at schema decode and must form the closed
pair from this table at semantic validation:

| Canonical role order | Exact `artifactId` |
| --- | --- |
| `source-daemon-peer` | `source-daemon-peer-v1` |
| `source-broker-peer` | `source-broker-peer-v1` |
| `source-wire-schema` | `source-handoff-wire-schema-v1` |
| `source-privilege-schema` | `source-handoff-privilege-schema-v1` |
| `source-operation-catalogue` | `source-handoff-operation-catalogue-v1` |
| `source-operation-catalogue-fingerprint` | `source-handoff-v1` |
| `source-compatibility-disposition` | `source-compatibility-disposition-v1` |
| `source-capability-api-fingerprint` | `source-capability-api-fingerprint-v1` |
| `source-serialization-snapshot` | `source-handoff-serialization-snapshot-v1` |
| `source-positive-fixture` | `source-handoff-positive-fixture-v1` |
| `source-bare-protocol-negative-fixture` | `source-bare-protocol-negative-fixture-v1` |
| `source-cross-fingerprint-negative-fixture` | `source-cross-fingerprint-negative-fixture-v1` |
| `source-installed-apply-object` | `source-installed-apply-object-v1` |

`SourceGenerationCompatibilityInstallationV1` contains, in exact order,
`schemaVersion = 1`, `kind = "source-generation-compatibility-installation"`,
`manifestSha256`, `dispositionSha256`, `sourceGenerationSha256`,
`installerAuthoritySha256`, `installedCensusSha256`, and the installer's final
`issuerProof`. The installer computes `installedCensusSha256` from the canonical ordered
tuples re-read from the immutable installed source generation after the atomic installation;
it cannot copy the manifest digest into that field without reading the installed bytes.

`SourceGenerationCompatibilityValidationV1` contains, in exact order,
`schemaVersion = 1`, `kind = "source-generation-compatibility-validation"`,
`manifestSha256`, `installationReceiptSha256`, `dispositionSha256`,
`sourceGenerationSha256`, `validatorAuthoritySha256`, `validatedCensusSha256`,
`verdict = "accepted"`, and the validator's final `issuerProof`. The typed validator
re-reads the installed census, recomputes every member and aggregate digest, and requires
`validatedCensusSha256` to equal the installation receipt's `installedCensusSha256`.

`SourceGenerationCompatibilityImportV1` contains, in exact order,
`schemaVersion = 1`, `kind = "source-generation-compatibility-import"`,
`validatedFloorSha256`, `manifestSha256`, `installationReceiptSha256`,
`validationReceiptSha256`, `dispositionSha256`, `sourceGenerationSha256`,
`executionCommitOid`, `featureSnapshotSha256`, `validatorAuthoritySha256`,
`verdict = "accepted"`, and the validator's final `issuerProof`.
`validatedFloorSha256` covers the canonical manifest, installation receipt, and validation
receipt only, avoiding a self-referential aggregate digest. `executionCommitOid` is the full
Git object id of exact clean C and `featureSnapshotSha256` is Q; neither may be abbreviated.
The external typed import/validation authority emits this receipt only after validating the
same installed source generation that the migration will use.

The accepted external `ADR-046-validation-and-delivery` Version 2 amendment owns the strict
JSON Schemas at
`docs/reference/schemas/delivery/source-floor-v1/{floor,source-generation-identity,manifest,member,issuer-proof,installation-receipt,validation-receipt,import-receipt}.schema.json`
and checked-in vectors under `tests/golden/delivery/source-floor-v1/`. The vectors contain
every canonical object and byte string, every length-framed preimage, every registry digest,
each verification key, each unsigned signing preimage, each signature, the validated-floor
preimage, and the aggregate identity. They include noncanonical field order, integer
spelling, text encoding, duplicate/unknown field, trailing byte, wrong domain, missing or
wrong frame, unknown tag/version, and copied-proof negatives. The Version 2 amendment,
approval, generated-manifest update, and Gate 0 receipt are external to this feature. The
separate source-generation producer/installer and import/validation authorities implement
and install objects conforming to those accepted schemas and vectors; they do not own or
silently redefine the repository contract artifacts. T589 does not deserialize a floor and
decide for itself. It invokes the exact disposition-pinned validator and consumes one private,
nonserializable `ValidatedSourceGenerationCompatibilityFloor` result by value. That type has
no public fields, constructor, serde implementation, `Clone`, `Copy`, `Default`, conversion,
or byte importer; it binds the validated floor digest, source generation, C/Q, and
authenticated issuer chain. A serialized receipt chain, even with copied matching authority
digests, is not a dispatch capability.

Acceptance is a closed append-only transition:

```text
absent
  -> manifest-produced
  -> atomically-installed
  -> installed-census-validated
  -> imported-for-exact-C/Q
```

Only the authority assigned to a transition may append its receipt. A skipped, reordered, or
repeated transition refuses. Any changed disposition, source generation, member bytes,
authority binding, receipt, C, or Q makes the aggregate stale and requires a new external
chain; no receipt is edited in place. T589 dispatch opens only from
`imported-for-exact-C/Q` after the disposition-pinned validator returns the nonserializable
typed result, and every later source-side handoff boundary revalidates the same aggregate.

The member-census rejection list is closed and always means all seven classes: `missing`,
`duplicate`, `extra`, `empty`, `stale-generation`, `stale-digest`, and
`cross-disposition`. Each refuses before accepted-socket fd transfer, authorization, or
mutation. Unknown versions, kinds, fields, authority bindings, transition order, or malformed
encodings are structural refusals and never weaken or replace that seven-class census.
The poison generator iterates all 13 canonical role/artifact pairs for all seven classes.
The exact case id is `source-floor/<class>/<role>`, and an independent expected-set fixture
enumerates the 91 ids from the literal seven-class list and literal 13-role table rather than
calling the poison generator. Every case keeps both the member array and declared
cardinality at 13. `missing/R` substitutes a second canonical successor pair for `R`;
`duplicate/R` substitutes a second `R` for its canonical successor; `extra/R` substitutes
one bounded syntactically valid `poison-extra-R` role/artifact pair for `R`; `empty/R`
installs zero bytes and records the correctly framed empty member digest and zero length;
`stale-generation/R` substitutes a distinct well-formed source-generation digest only in
that member; `stale-digest/R` keeps the installed bytes but records the framed digest and
length of a one-byte-different nonempty payload; and `cross-disposition/R` substitutes one
distinct well-formed disposition digest only in that member. Successor wraps from the last
role to the first.

After the one substitution, the fixture recomputes and re-signs in this exact order: member
content digest where the class changes role, artifact id, or bytes, canonical manifest and producer proof,
`manifestSha256`, installed census and installer proof,
`installationReceiptSha256`, validation receipt and validator proof,
`validationReceiptSha256`, `validatedFloorSha256`, exact-C/Q import receipt and validator
proof, import receipt identity, and aggregate typed floor identity. Test-only keys are pinned
independently of the generator. Thus every envelope, declared count, enclosing digest, and
signature is valid before the selected semantic invariant is checked. For the mathematically
overlapping `missing`, `duplicate`, and `extra` set cases, the oracle asserts the complete
expected census-error set rather than relying on first-error order. A fixture with a stale
enclosing hash, invalid signature, wrong cardinality, unvisited id, duplicate id, or a case
that fails before its intended semantic check makes the matrix itself fail and does not
count toward 91.

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
| `panelPolicy` | literal `unanimous-ten-role-exact-final-candidate` |

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
panel-pending + exact unanimous ten-role F-bound attestations -> panel-satisfied
panel-satisfied -> seal-eligible -> merge-eligible -> merged-byte-identical-F
panel-pending + completed/terminal panel with any missing role, recommendation,
  disagreement, or stale binding
  -> panel-refused
```

`blocked`, `abandoned-unmerged`, and `panel-refused` authorize no seal, merge, successor
wave, or release. `recover-panel-without-new-request` authorizes only the externally defined
recovery-attestation surface linked to the retained request; it creates no second request and
cannot itself satisfy the panel. The validator requires the repository's complete ten-role
roster, `signoff = true` iff recommendations are empty, identical F/commit/tree/disposition
bindings, and every constitutional predecessor. No action or field can encode `waived`,
partial, force, reduced roster, stale-candidate attestation, or panel substitution. A content
or history change after F, or any failed recovered panel, returns to external escalation
rather than admitting another feature-local request.
