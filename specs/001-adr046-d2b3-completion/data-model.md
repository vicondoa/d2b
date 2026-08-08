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
exact form `evidence-sidecars/sc002/sha256/<typed-digest>.json`, where `<typed-digest>` is the lowercase
64-hex `Sc002ActivationReceiptContentSha256V1`:

```text
SHA-256(
  "d2b:sc002:activation-receipt-content:v1\0" ||
  u64be(canonical-receipt-length) || canonical-receipt-bytes
)
```

The length counts exact UTF-8 octets. Raw hashing, an unframed concatenation, native-width
integer framing, pretty JSON, or caller-selected domain bytes is ineligible. The locator and
its digest component are immutable. An absolute path, URL, query or fragment, empty or dot
component, `..`, alternate separator, noncanonical digest, or locator outside this namespace
is malformed. No field, enum variant, or version is added to `EvidenceRecord`, and no other
validation may reference this receipt type.

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

The receipt is canonical UTF-8 JSON with fields in the table order, no BOM, whitespace, or
trailing newline, ASCII strings with shortest JSON escaping, lowercase 64-hex digests, and
unsigned base-10 integers with no sign, exponent, fraction, or leading zero except `0`.
Every sample contains exactly, in order, `resourceIdentity`, `effect`, `ready`,
`selectedStop`, `elapsedNs`, and `progress`. Each effect and Ready observation contains
exactly `resourceIdentity`, then `tickNs`; selected stop contains those fields followed by
`source`; each progress observation contains exactly `resourceIdentity`, `tickNs`, then
`kind`. Duplicate, missing, reordered, or unknown fields, invalid UTF-8, non-ASCII text,
alternate escapes, and bytes unequal to decode-then-canonical-reencode refuse before the
content digest is accepted.

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
most 16,385 bytes, computes the typed domain-separated and length-framed receipt-content
digest over the exact opened bytes before decoding, and derives
`evidence-sidecars/sc002/sha256/<typed-digest>.json`. It decodes and validates from those same
bytes, including the outer candidate/content/snapshot binding, before candidate publication.
Through the already held candidate-directory fd it creates and verifies current-effective-uid
`0700` namespace directories, opens an unnamed `O_TMPFILE|O_RDWR|O_CLOEXEC` inode in the
final parent, sets and verifies current-effective-uid ownership and mode `0600`, writes the
exact validated bytes, and `fsync`s and revalidates the opened inode. Through one validated
procfs `/proc/self/fd` directory fd it capability-free links that exact opened inode directly
to the final no-replace leaf with `linkat(..., AT_SYMLINK_FOLLOW)`. It uses no
`AT_EMPTY_PATH`, linked temporary, or name-consuming publication rename. Before publishing
the `EvidenceRecord` carrying the derived locator, it final-reopens and matches the same
device/inode and exact bytes, `fsync`s the final parent, then `fsync`s each held ancestor
directory fd bottom-up: `sc002`, `evidence-sidecars`, and the candidate directory. Namespace
creation is therefore durable, not merely the final link. Before unnamed-inode creation or
recovery, every
importer and every cleanup worker acquires the same verified candidate-scoped exclusive OFD
write lock and retains it through publication or cleanup, parent `fsync`, the applicable
census, and `EvidenceRecord` publication or return. There is no second cleanup lock or
lock-free orphan path. A live importer owns that lock; a failed no-replace loser or restart
path cannot inspect, rename, or remove the live owner's unnamed inode. Restart cleanup begins only
after it acquires the released lock. The lock is one fixed candidate-relative regular
single-link current-effective-uid `0600` leaf opened with `O_CLOEXEC`. Every actor verifies
the same device/inode before taking `F_OFD_SETLK`; the leaf is never replaced, renamed, or
unlinked. A nonblocking lock attempt returning `EAGAIN` or `EACCES` proves a live owner and
requires zero namespace inspection or mutation before the caller returns the typed
`sc002-sidecar-owner-live` refusal.

The importer classifier is namespace- and inode-derived. Unsupported `O_TMPFILE`, invalid
procfs or target-mount identity, and unsupported procfs-fd direct linking return a typed
refusal with zero receipt leaf and zero `EvidenceRecord` mutation. A crash before inode
`fsync`, or after inode `fsync` but before the direct link, exposes no receipt leaf. A crash
after the direct link but before final-parent `fsync` may recover either absence or the exact
complete final inode. Restart either creates a fresh unnamed inode or final-reopens, verifies,
and syncs that exact final. A nonidentical `EEXIST` is a conflict and preserves the existing
leaf. An exact final whose parent and ancestors are durable is replay success with zero
write. No classifier admits a named temporary, a linked pre-final name, or process memory as
authority.

The reserved temporary and quarantine namespaces are legacy-observation surfaces only.
The direct-final importer, incident publisher, and request-output publisher never create a
named temporary or quarantine leaf. While holding the lock, cleanup may inspect a retained
legacy name through the held parent fd, once-open it, and record its device/inode, owner,
mode, link count, digest, and bytes. It never renames or unlinks that name. A same-uid
process can replace a candidate pathname without honoring the OFD lock, and Linux provides
no inode-qualified rename or unlink, so an advisory lock plus pre/post reopen cannot prove
kernel-enforced sole ownership. Every observed legacy name therefore enters the
identity-ambiguity incident path before any other sidecar namespace mutation. The complete
incident preimage is direct-final durable first; later payload or residue evidence is a
direct-final immutable copy from the retained source fd, and the original legacy name
remains frozen in the recursive evidence census. This is legacy-only recovery, never a
publication path for new records.

`CandidateNamespaceWriteOwner<'guard>` is a private negative authority borrowed only from
the same `SidecarCleanupOwner<'guard>`. It proves that existing-name consumption is
forbidden, not that an OFD lock excludes same-uid writers. Its only publication method may
prepare an unnamed inode from a retained source fd and direct-link that new inode to a final
no-replace evidence name after source and destination parent identities, the source-name
binding, procfs, mount, and final-leaf absence all validate. It exposes no `renameat2` or
`unlinkat` operation. Parent-boundary loss, source-name replacement, or destination-reopen
mismatch returns `sc002-namespace-write-ownership-unproven`, performs no later link, rename,
or unlink, and preserves every name not changed by the fault injector. The owner has no
public constructor, fields, accessors, serde, clone, copy, conversion, fd extraction, or
lifetime independent of the candidate guard. Independently pinned hermetic cases
`candidate-namespace-write-owner/parent-boundary-loss`,
`candidate-namespace-write-owner/source-replacement`, and
`candidate-namespace-write-owner/destination-reopen-mismatch` exercise all three failures
at the method boundary and assert the typed refusal, the complete before/after name census,
and zero subsequent link/rename/unlink calls. A separate
`candidate-namespace-write-owner/existing-name-move-hook-unreachable` poison fails if any
current rename hook exists.

The candidate lock serializes every cleanup worker, not only importer against cleanup.
Cleanup of the same leaf, cleanup of different leaves under the same candidate, incident
recovery, authenticated disposition, and the retention guard all acquire independently
opened descriptions of that one verified lock inode before opening any sidecar namespace.
A live cleanup owner therefore excludes every overlapping cleanup before namespace
inspection. A loser returns `sc002-sidecar-owner-live` with zero namespace opens and zero
namespace mutations; the sole post-release retry discards every pre-lock fd and observation,
opens fresh descriptions, and recensors under the lock. No per-leaf lock, optimistic census,
or incident-only bypass exists.

Successful lock acquisition yields one private `CandidateSidecarGuard` that solely owns the
`OwnedFd` carrying the locked open file description. The guard has no raw-fd accessor,
`AsRawFd`, `IntoRawFd`, duplication, transfer, serialization, `Clone`, or conversion
surface. Its private `Drop` closes that sole description and therefore releases the OFD
lock. The lock fd and every candidate, ancestor-directory, namespace, snapshot, request,
disposition, temporary, and reopened leaf fd are created with `O_CLOEXEC`; any received fd
uses `MSG_CMSG_CLOEXEC`, and no plain `dup` or non-CLOEXEC reconstruction is permitted.

Cleanup additionally requires exactly one private
`SidecarCleanupOwner<'guard>`. It is constructed only by
`CandidateSidecarGuard::enter_cleanup(&mut self)` and contains the exclusive mutable borrow
of that exact guard. It has no public fields, constructor, accessor, `Clone`, `Copy`,
`Default`, serde implementation, conversion, fd reconstruction, or independent lifetime.
Every current cleanup namespace observation, retained-legacy census, direct-final evidence
copy, and return is a method on that owner and reaches the held directory authority only
through its borrowed guard. Quarantine and retirement renames are historical input states,
not methods: the current owner exposes no rename, unlink, hardlink-existing-source, or
name-consuming hook.
Consequently an owner cannot outlive, be paired with a later, or remain usable after the OFD
guard that created it. A nonblocking loser never obtains either authority. Restart drops all
process-local values, independently reopens and validates the lock inode with `O_CLOEXEC`,
acquires the lock into a fresh guard, and only then may borrow a fresh cleanup owner and open
a namespace. Compile-fail/API-surface tests must reject owner return as `'static`, storage
beyond the guard, construction from any fd, guard extraction, guard replacement, fd
duplication, use after guard drop, and every attempted rename or unlink hook.

For every retained legacy retirement record, validation derives one
`Sc002RetirementIdV1` as

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
The retired subtree is read-only compatibility evidence: the new direct-final protocol
creates no named orphan and cleanup creates no new retired leaf. Existing retired leaves are
once-opened, fully revalidated against their path-derived id, and retained. A retained
legacy temporary or quarantine name is never moved into this subtree, even when its bytes
match an existing retired leaf.

`EEXIST` at a retained retirement destination is never an idempotent-success signal for a
separately named source. Validation preserves both names and classifies an already durable
historical collision as `Sc002IncidentKindV1 = "retirement-id-collision"` using its complete
direct-final preimage. A newly observed legacy source instead takes
`Sc002IncidentKindV1 = "identity-ambiguity"` before any existing-name mutation because sole
namespace ownership is unprovable. Tests retain historical collision and distinct-id census
validation, but independently prove that the current protocol emits no quarantine rename,
retirement rename, or sidecar-data unlink.

The retained retired census is bounded to at most 64 regular single-link current-effective-uid
`0600` leaves and at most 1,048,576 total encoded bytes beneath `retired/sha256`, with no
unknown directory level, leaf, or name. Every leaf must be at most 16,384 bytes and must
revalidate against both path digests and the candidate binding. No current path adds a
member. Encountering a 65th retained leaf, a byte-bound breach, or a malformed census
preserves every name and returns the fixed
`sc002-retirement-census-exhausted` or `sc002-retirement-census-invalid` refusal. It never
grows an unbounded retirement set. Exhaustion persists
`Sc002IncidentKindV1 = "retirement-census-exhausted"` and binds the verified source
identity, the valid pre-add census digest, and the current and prospective leaf/byte counts.
A malformed census persists
`Sc002IncidentKindV1 = "retirement-census-invalid"` and binds the verified source identity
and the bounded observed-census digest. Neither path invents a second identity tuple. A valid
retired orphan is immutable, non-authorizing compatibility residue and does not block retry
or close.

The sole candidate-retention guard is T589's private
`packages/xtask/src/delivery/storage.rs::CandidateRetentionOwner`; no importer, restart
cleanup worker, panel stage, or generic filesystem sweep may mint or imitate it. It owns no
deletion authority. After acquiring the same exclusive candidate OFD lock, it performs one
zero-mutation whole-scope retention check. The full delivery-state census must prove all of
the following: the candidate is terminal `merged` or `abandoned-unmerged`; every request,
reservation, panel, seal, eligibility, and merge transition is terminal; every SC-002
incident is absent or has exact primary or resolution `successor-admitted`; every retained external reference to the
candidate remains resolvable; both ephemeral namespaces are empty for every direct-final
ordinary path, while a terminal legacy incident has only the exact names bound by its frozen
recursive census; and the receipt, retired, incident, disposition, status, and resolution
namespaces are exact, bounded, and valid. The guard also
proves that the candidate root remains at its canonical address and that all request,
reservation, panel-request, panel-record, evidence-record, receipt, seal, eligibility,
merge, incident preimage, anchor, metadata, payload, residue-staging, residue, primary
status, resolution-evidence, resolution status, successor freeze, disposition request,
disposition, and successor-admission history, including authenticated absence where a
terminal state has no such artifact, remains immutable. The separately owned
`evidence-sidecars/sc002/retired` subtree retains every verified orphan under the bounded
census above. Neither that subtree nor any other candidate descendant is automatically
unlinked, and the candidate root is never renamed, tombstoned, or deleted. A failed predicate
or census performs zero mutation and returns `sc002-candidate-retention-blocked`. There is no
clock-driven, per-leaf, subtree, or candidate-root deletion.

The retention census recursively walks every listed durable namespace through the same
fd-relative node grammar used by the frozen primary-evidence scope. It requires every
incident copy of `Sc002IncidentPreimageV1` to be byte-identical to the immutable preimage
object and to contain all kind-specific collision, census/count, or ambiguity components.
It also requires every signed disposition to name the same durable successor freeze and
disposition request later consumed by apply and admission. A top-level directory identity,
opaque `incidentIdPreimageHex`, or status count cannot stand in for that recursive
path-and-content check.

Before direct-final ordinary success or an ordinary refusal with no legacy residue returns,
the still-held lock guards an exact empty census of both ephemeral reserved namespaces and
the bounded durable census above. A terminal legacy incident instead binds every retained
legacy name in its frozen recursive census. No joined path or broad sweep is allowed.

Identity ambiguity has one different, attainable terminal state. A retained legacy
temporary or quarantine name is ambiguous even when two observations match, because the
candidate OFD lock does not exclude a same-uid replacer. Cleanup MUST NOT move, restore, or
unlink that name. It records the before and after identity digests, derives the incident id,
and first direct-final publishes and syncs the complete incident preimage. It then copies
the exact bytes from the retained source fd into a new unnamed inode, file-syncs it, and
capability-free procfs-fd links that new inode directly to the durable candidate-relative
incident payload
`evidence-sidecars/sc002/incidents/payload/sha256/<incident-id>.bin` no-replace. Final reopen
must prove the copied digest, bytes, owner, mode, and incident binding before `parked` may
publish. The source name remains frozen and recursively censused; there is no existing-name
rename, hardlink, or unlink. The incident digest is a fixed domain-separated digest of the
candidate binding and both observed identity tuples for
`Sc002IncidentKindV1 = "identity-ambiguity"`; raw device/inode values never enter an error
or observability surface.

A same-name replacement before the copy, source identity loss, `ENOENT`, or a nonidentical
payload `EEXIST` never authorizes success or unlink. The already durable write-ahead
preimage leaves the stable incident in exactly one classified nonterminal recovery variant,
preserves every still-named leaf, publishes no `parked` status, and blocks record publication
and close. An exact metadata-bound source-fd-to-payload continuation is
`recovery-resumable`; identity loss, conflict, or ambiguity is
`recovery-irreconcilable`. Neither is a durable primary status or terminal cleanup result.
Restart recovery repeats only the exact direct-final copy under the same lock;
irreconcilable state requires the authenticated resolution path below.

There are two terminal incident entry states. A fully revalidated expected payload, with the
original legacy source retained in the frozen census and all required directory updates
durable, publishes `parked`.
When the expected identity can no longer be proved, an authenticated external disposition
may instead drive the no-unlink mismatch-retention protocol. That protocol leaves every
still-named sidecar data leaf at its observed name and direct-final publishes an immutable
evidence copy through the fixed durable staging namespace
`evidence-sidecars/sc002/incidents/residue-staging/sha256/<incident-id>/<source-slot>.bin`.
`source-slot` is the closed value `temporary`, `cleanup-quarantine`, `payload`,
`retired-source`, or `retired-existing-destination`. The two retired slots are derived only
from the exact source and existing-destination locators persisted in collision metadata;
they make an allowed retired source representable without treating an arbitrary retired
leaf as incident evidence. A kind/slot/locator table is closed: identity ambiguity admits
its recorded temporary, cleanup-quarantine, retired-source, or payload locator; collision
admits its recorded source plus the one recorded retired existing destination; either census
kind admits its one recorded source. No kind admits a second leaf under one slot. The slot is
selected from the held source-parent authority and immutable metadata, never accepted from a
caller. Each evidence copy is prepared in an unnamed inode from the retained source fd,
file-synced, procfs-fd linked directly to the staging final no-replace, and no-follow
final-reopened. The reopened copied content plus the retained source identity derives

```text
Sc002ResidueIdV1 =
  SHA-256(
    "d2b:sc002:residue-id:v1\0" ||
    incident_id[32] || u32be(source_slot_length) || source_slot ||
    reopened_I[32]
  )
```

and the same held authority then direct-final copies the staging bytes to
`evidence-sidecars/sc002/incidents/residue/sha256/<incident-id>/<residue-id>.bin`, reopens
it, and proves the path-derived id and exact copied bytes. A source replacement or final
conflict is reported as `recovery-resumable` only when one exact copy continuation remains;
otherwise it is irreconcilable. Original and staging names are never restored, renamed, or
unlinked. Only when every retained source and staging name is present in the frozen
recursive census, every residue revalidates, and the sorted residue-id census is
file-and-directory durable may the protocol append `mismatch-retained`. This is the coherent
terminal cleanup-mismatch state with retained legacy names. It remains permanently
ineligible, blocks its parked candidate's record publication and every close stage, survives
restart, and can advance only through the same authenticated external disposition and a
fresh successor. No SC-002 cleanup or incident path calls `unlinkat` on a sidecar data leaf.
No SC-002 cleanup or incident path performs a name-consuming rename. A pathname observation
never authorizes removal, terminal status, or success.

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

Every incident first constructs one canonical `Sc002IncidentPreimageV1`. Its JSON fields
occur exactly in this order:

`schemaVersion = 1`, `kind = "sc002-incident-preimage"`, `incidentKind`,
`parkedCandidateId`, `parkedContentId`, `parkedSnapshotSha256`, `contentDigest`,
`retirementId`, `sourceIdentitySha256`, `existingDestinationIdentitySha256`,
`retiredCensusSha256`, `currentLeafCount`, `currentEncodedBytes`,
`prospectiveLeafCount`, `prospectiveEncodedBytes`, `observationStage`,
`beforeIdentitySha256`, `afterIdentitySha256`, and `incidentIdPreimageHex`.

The parked triplet and content digest are always non-null. The collision variant requires
only `retirementId`, `sourceIdentitySha256`, and
`existingDestinationIdentitySha256`; exhausted census requires only
`sourceIdentitySha256`, `retiredCensusSha256`, and all four counts; invalid census requires
only `sourceIdentitySha256` and `retiredCensusSha256`; identity ambiguity requires only
`observationStage`, `beforeIdentitySha256`, and `afterIdentitySha256`. Every other
kind-specific field is null. The object independently reconstructs `B`, the exact
kind-specific suffix, the domain-terminated binary preimage below, and
`incidentIdPreimageHex`; disagreement is malformed. It is at most 8,192 bytes and follows
the canonical JSON rules used by incident metadata.

The exact bytes are the incident write-ahead record. Before any anchor, metadata, payload,
residue, status, resolution, freeze, request, disposition, or admission publication, the
publisher opens an unnamed `O_TMPFILE|O_RDWR|O_CLOEXEC` inode in the final preimage
directory, writes the complete canonical object, and `fsync`s and revalidates that opened
inode. The privilege-dropped target must have an empty effective capability set. It opens
and validates `/proc/self/fd` once as procfs, retains that directory fd, and calls
`linkat(proc_self_fd_dirfd, decimal_fd, preimage_parent_fd, final_name,
AT_SYMLINK_FOLLOW)`. This capability-free procfs-fd form links the exact opened inode
directly at
`evidence-sidecars/sc002/incidents/preimages/<incident-kind>/sha256/<incident-id>.json`;
it never uses `AT_EMPTY_PATH`, a deterministic temporary, or a name-consuming rename. The
link is no-replace. The publisher then final-reopens and matches the same device/inode,
`fsync`s the preimage parent, and syncs every changed ancestor.

There is no create-and-unlink capability probe and no named-partial fallback. Unsupported
`O_TMPFILE` or unsupported procfs-fd linking refuses with no incident name, sidecar-data
mutation, freeze, or request. A crash before inode `fsync` or after inode `fsync` but before
the direct link exposes no name. A crash after the direct link but before parent `fsync`
may recover either no final name or the exact complete final inode; restart recensors and
either retries from a new unnamed inode or reopens, verifies, and syncs that exact final.
A nonidentical `EEXIST` is `preimage-final-conflict` and preserves the existing name. Thus
every visible preimage is complete and file-synced, every replay decision is recoverable
from namespace plus inode state, and no classifier depends on process memory.

Immediately after acquiring the candidate lock and before observing any historical
quarantine or retirement name, cleanup validates the exact publication environment without
creating a namespace entry: effective uid/gid and empty effective capabilities, procfs identity and
`/proc/self/fd` magic-link semantics, target parent mount/device identity, `openat2`
resolution support, and `O_TMPFILE` support on that target filesystem. The actual
procfs-fd link is the first and only link support check and occurs only when a complete
preimage must be published. Its failure has zero incident or sidecar namespace mutation;
there is no probe link to remove.

Every anchor, metadata, primary
status, resolution, disposition request, disposition, successor freeze, and successor
admission record embeds the same complete `incidentPreimage` object byte-for-byte and names
the same `preimageLocator`. A record that carries only an incident id or opaque preimage hex
is incomplete and refuses. This duplication is deliberate replay evidence: any one valid
durable transition can reconstruct all common and kind-specific inputs, while equality
against the immutable preimage object prevents the copies from diverging.

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

`observation_stage` retains historical `1 = quarantine-reopen` and
`2 = retirement-reopen`, and adds `3 = legacy-named-source-unsealed`.
Source/destination order, before/after order, kind domains, and stage codes are not
interchangeable. The census paths intentionally have one source
identity plus census evidence instead of a fabricated second identity. Raw device/inode,
uid, mode, link-count, name, and census bytes never enter an error or observability surface.

Every incident transition first persists one immutable `Sc002IncidentAnchorV1`. Its
canonical JSON records exactly, in order, `schemaVersion = 1`,
`kind = "sc002-incident-anchor"`, `incidentKind`, `incidentId`,
`preimageLocator`, `incidentPreimage`, `incidentIdPreimageHex`, `parkedCandidateId`, `parkedContentId`,
`parkedSnapshotSha256`, and `contentDigest`. It is at most 16,384 bytes and uses the same
canonical JSON rules as metadata below. The complete preimage must recompute the id, kind,
parked triplet, and content digest, and both preimage representations must equal the exact
durable `Sc002IncidentPreimageV1` bytes. This anchor uses the unnamed-inode procfs-fd
direct-final protocol, is final-reopened, and is leaf/parent/every-ancestor synced before
metadata or payload-copy publication. Every restart classifier begins from it. An exact
anchor final with interrupted parent durability is `recovery-resumable`; a nonidentical
final anchor is `recovery-irreconcilable`. There is no anchor temporary. A missing expected
final is reconstructed only from the durable preimage, while a conflicting final is
identity-bound by the bounded-failure commitment rather than repaired or fabricated.
No durable incident status or resolution may omit or disagree with the exact anchor
preimage.

Every incident transition then has one immutable `Sc002IncidentMetadataV1`. Its canonical JSON
rejects unknown, duplicate, missing, or reordered fields and records exactly these fields in
this order:

| Field | Type and rule |
| --- | --- |
| `schemaVersion` | integer `1` |
| `kind` | literal `sc002-incident-metadata` |
| `incidentKind` | exact `Sc002IncidentKindV1` |
| `incidentId` | lowercase 64-hex id recomputed from this metadata |
| `preimageLocator` | exact `incidents/preimages/<incidentKind>/sha256/<incidentId>.json` locator |
| `incidentPreimage` | complete canonical `Sc002IncidentPreimageV1`, byte-identical to the immutable preimage object |
| `incidentIdPreimageHex` | lowercase hex of the complete kind-specific preimage, including the exact domain terminator and every fixed-width field |
| `parkedCandidateId` | exact candidate id from `B` |
| `parkedContentId` | exact content id from `B` |
| `parkedSnapshotSha256` | exact snapshot digest from `B` |
| `contentDigest` | exact source content digest from `B` |
| `anchorLocator` | exact `incidents/anchors/<incidentKind>/sha256/<incidentId>.json` locator |
| `metadataLocator` | exact `incidents/metadata/sha256/<incidentId>.json` locator |
| `sourceSlot` | exact closed source slot selected from held parent authority |
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
| `observationStage` | identity-ambiguity stage `1`, `2`, or `3`, otherwise null |
| `beforeIdentitySha256` | identity-ambiguity before digest, otherwise null |
| `afterIdentitySha256` | identity-ambiguity after digest, otherwise null |

Null is the only absent representation. The kind-specific non-null fields are exactly the
components of the corresponding incident-id preimage above; all other kind-specific fields
must be null. `preimageLocator`, `anchorLocator`, `metadataLocator`, `sourceSlot`, `sourceLocator`, and `payloadLocator` must
agree with the closed kind/slot/locator table and the already held parent-directory
authority. `sourceLocator` is accepted only from the recorded temporary,
cleanup-quarantine, payload, or exact retired source/destination namespaces, and it is never
rendered in an error or CLI projection. The metadata schema independently reconstructs the
complete preimage bytes from the structured object and the flattened kind-specific fields,
requires both forms and the immutable preimage object to be byte-identical, compares them
byte-for-byte with `incidentIdPreimageHex`, and
recomputes `B`, `I`, `C`, every count, stage, and final incident id. The object is at most
32,768 bytes and uses canonical UTF-8 JSON with no BOM, whitespace, or trailing newline,
shortest ASCII string escaping, lowercase hex, and unsigned base-10 integers with no leading
zero. A metadata object that cannot reconstruct its complete preimage and id or differs from
decode-then-canonical-reencode is malformed.

The exact durable paths are:

```text
evidence-sidecars/sc002/incidents/preimages/<incident-kind>/sha256/<incident-id>.json
evidence-sidecars/sc002/incidents/anchors/<incident-kind>/sha256/<incident-id>.json
evidence-sidecars/sc002/incidents/metadata/sha256/<incident-id>.json
evidence-sidecars/sc002/incidents/payload/sha256/<incident-id>.bin
evidence-sidecars/sc002/incidents/residue-staging/sha256/<incident-id>/<source-slot>.bin
evidence-sidecars/sc002/incidents/residue/sha256/<incident-id>/<residue-id>.bin
evidence-sidecars/sc002/incidents/status/sha256/<incident-id>/parked.json
evidence-sidecars/sc002/incidents/status/sha256/<incident-id>/mismatch-retained.json
evidence-sidecars/sc002/incidents/status/sha256/<incident-id>/disposition-validated.json
evidence-sidecars/sc002/incidents/status/sha256/<incident-id>/successor-admitted.json
evidence-sidecars/sc002/incidents/resolution-evidence/sha256/<incident-id>/<typed-digest>.bin
evidence-sidecars/sc002/incidents/resolution/sha256/<incident-id>/disposition-validated.json
evidence-sidecars/sc002/incidents/resolution/sha256/<incident-id>/successor-admitted.json
```

The `resolution` namespace is used only when the primary metadata/payload/status branch is
irreconcilable and therefore cannot safely receive another primary status. It never replaces
or repairs primary bytes. The resolution record and the canonical bytes it authenticates
MUST NOT be members of the census whose digest the resolution record embeds.

`CanonicalIncidentPrimaryEvidenceCensusV1` therefore has one frozen primary-evidence scope:
the incident's immutable structured preimage, anchor, metadata, payload, residue-staging,
residue, and primary status names
plus the closed metadata-recorded source slots. It excludes every resolution record,
resolution-evidence leaf, disposition leaf, receipt, and successor artifact. A name under an excluded
namespace is not silently ignored when it aliases or appears beneath a primary root; the
scope validator rejects the alias. The root set and source-slot table are selected from the
incident kind and complete incident-id preimage, never from a caller-provided path. When no
canonical preimage survives, the bounded-failure form scans all five closed source-slot roots
in source-slot enum order, records `record-invalid`, and relies on the authenticated
disposition's complete preimage before any resolution transition.

The leaf and scope digests are closed typed constructors:

```text
Sc002PrimaryEvidenceLeafContentDigestV1 =
  SHA-256(
    "d2b:sc002:primary-evidence-leaf-content:v1\0" ||
    u64be(content-length) || exact-once-opened-content
  )

Sc002IncidentPrimaryScopeIdentityDigestV1 =
  SHA-256(
    "d2b:sc002:incident-primary-scope-identity:v1\0" ||
    u64be(scope-payload-length) || scope-payload
  )

scope-payload =
  incident_id[32] || parked_candidate_id[32] || parked_content_id[32] ||
  parked_snapshot_sha256[32] || u32be(node-count) ||
  RecursiveNodeObservation[node-count]

RecursiveNodeObservation =
  u8(root-code) ||
  u8(root-instance-code) ||
  u32be(relative-path-length) || relative-path ||
  u8(node-kind) ||
  u64be(st_dev) || u64be(st_ino) ||
  i64be_twos_complement(st_ctime_sec) || u32be(st_ctime_nsec) ||
  u32be(st_mode) || u32be(st_uid) || u32be(st_gid) ||
  u64be(st_rdev) || u64be(st_size) || u64be(st_nlink) ||
  payload_digest[32]
```

Root codes are closed and ordered:
`0x01 = preimage`, `0x02 = anchor`, `0x03 = metadata`, `0x04 = payload`,
`0x05 = residue-staging`, `0x06 = residue`, `0x07 = primary-status`, and
`0x08 = source-slot`.
Root-instance code is `0x00` for each of the first seven roots and is closed for
`source-slot` to `0x01 = temporary`, `0x02 = cleanup-quarantine`, `0x03 = payload`,
`0x04 = retired-source`, and `0x05 = retired-existing-destination`. The required root table
therefore has exactly twelve ordered `(root-code, root-instance-code)` pairs; all-zero,
top-level directory identities, or one generic source-slot row cannot collapse those roots.

Serialized node kind is total over every representable no-follow observation:
`0x00 = absent`, `0x01 = directory`, `0x02 = regular-file`, `0x03 = symlink`,
`0x04 = block-device`, `0x05 = character-device`, `0x06 = fifo`, `0x07 = socket`,
`0x08 = mount`, and `0x09 = other`. Tag `0xff` is reserved and is rejected by every
complete-body and bounded-failure-body decoder even when every following field is zero. An
unavailable observation exists only as private
`DeniedSc002PrimaryEvidenceScopeV1`; it has no canonical node bytes, locator, or digest and
maps exclusively to denied scope. An absent root has an empty relative path and zero
identity/payload fields. A directory has a zero payload digest. A regular file carries
`Sc002PrimaryEvidenceLeafContentDigestV1` computed from its exact once-opened bytes.
For a symlink, the same length-framed constructor hashes the exact `readlinkat` bytes and
`node-kind` keeps that digest injectively distinct from a regular-file observation. Device
nodes carry exact unsigned `st_rdev`; other non-content nodes have a zero payload digest.
Every kind other than absent, directory, and regular-file is semantic failure evidence and
can occur only in the full-coverage bounded-failure body, never in a complete authorizing
body.
`st_mode` is the exact unsigned mode/type word returned by the no-follow stat and must agree
with `node-kind`; `st_uid` and `st_gid` are the exact unsigned owner values. All three are
fixed-width fields even for absent roots, where they are zero.
For a present root, the root itself appears first with an empty relative path, followed by
every descendant directory and regular file recursively in unsigned-byte relative-path
order. Invalid descendants remain observations at their exact sorted position. The relative
path is raw canonical Unix component bytes beneath the already selected root, never a caller
path; slash and NUL cannot occur in a component, and scanner-synthesized empty, `.`, `..`,
repeated-separator, or absolute forms refuse. Non-ASCII names are encoded as bytes and do not
alias or disappear.

Enumeration is fd-relative and recursive. Every component is opened beneath the held
candidate directory with
`openat2(RESOLVE_BENEATH|RESOLVE_NO_SYMLINKS|RESOLVE_NO_MAGICLINKS|RESOLVE_NO_XDEV)`;
the scanner never joins a path. It keeps each ancestor fd until its children have been
enumerated, opens each regular leaf exactly once for identity and content hashing, and
re-stats every directory and leaf after the scan. The complete form is limited to 128 node
observations and 2,097,152 present file bytes, including nodes below status, residue, and
source-slot directories. A nested unknown, duplicate, unstable, unreadable, or over-bound
entry therefore cannot disappear behind a valid top-level root identity.
`i64be_twos_complement` is exactly eight bytes in network order. `st_ctime_nsec` must be
`0..=999,999,999`; an out-of-range value refuses. Every root is
opened no-follow beneath the held candidate fd and every recursive node is observed twice
through independently reopened fd sets under the same candidate lock. A symlink is read
without following it; a mount or device is recorded without opening or traversing it. The
constructor accepts no domain string, display path, errno,
serializer object, or native-width integer. Relative path bytes enter only this typed
internal commitment and never an error, log, metric, span, CLI projection, or `Debug`.

The kind-bearing anchor parent lets restart recover the closed incident kind without decoding
a conflicting anchor. The candidate directory supplies the parked triplet. A conflicting
anchor is never trusted for either; an accepted disposition must supply and authenticate the
complete preimage before resolution can advance.

The complete census uses the same recursive node grammar as the scope-identity constructor;
there is no second flat locator/presence grammar:

```text
0x01 || 0x00 ||
incident_id[32] ||
parked_candidate_id[32] || parked_content_id[32] || parked_snapshot_sha256[32] ||
u32be(node-count) ||
RecursiveNodeObservation[node-count]
```

The observations are ordered first by root code, then root-instance code, then unsigned
canonical relative-path bytes. Every one of the twelve required root pairs occurs exactly
once as either the fixed-width all-zero absent-root observation or a present root node. A present directory root and every
descendant directory are explicit `node-kind = 0x01` members; every regular file is an
explicit `node-kind = 0x02` member carrying its exact encoded length and typed once-opened
payload digest. Directory observations carry their opened identity, mode, owner, rdev, size,
link count, and change time with a zero payload digest. A directory's complete sorted descendant
sequence and the enclosing node count bind membership; a valid directory observation cannot
stand in for omitted children. The complete form is limited to 128 node observations and
2,097,152 present regular-file bytes. Missing roots, duplicate root/path pairs, an absent
root with nonzero fields, a directory with a nonzero payload digest, a file with a zero or
wrong digest, any invalid/unavailable node kind, a descendant without every ancestor
directory, or any unlisted descendant refuses as a complete body.

A stable invalid or prospectively soft-over-bound primary scope that two complete walks can
cover uses the separate bounded-failure form, never an authorization sentinel:

```text
0x01 || 0x01 ||
incident_id[32] ||
parked_candidate_id[32] || parked_content_id[32] || parked_snapshot_sha256[32] ||
u8(failure-cause) ||
u64be(observed-record-count-saturated) ||
u64be(observed-present-bytes-saturated) ||
u8(failure-root-code) ||
u8(failure-root-instance-code) ||
failure_path_sha256[32] ||
u32be(observed-node-count) ||
RecursiveNodeObservation[observed-node-count] ||
scope_identity_before[32] || scope_identity_after[32]
```

The closed failure causes are
`0x01 = record-limit`, `0x02 = byte-limit`, `0x03 = enumeration-unavailable`,
`0x04 = unknown-name`, `0x05 = identity-unstable`, `0x06 = record-invalid`, and
`0x07 = depth-limit`. `failure-root-code` and `failure-root-instance-code` select the first
failing required root pair in fixed order, not filesystem iteration order.
`failure_path_sha256` is the typed
`Sc002PrimaryEvidenceFailurePathDigestV1`:

```text
SHA-256(
  "d2b:sc002:primary-evidence-failure-path:v1\0" ||
  u8(failure-root-code) ||
  u8(failure-root-instance-code) ||
  u32be(relative-path-length) || canonical-relative-path
)
```

It binds the canonical path of the first failing recursive node without rendering that path.
An absent-root failure uses an empty relative path. Directory enumeration retains at most
128 canonical child names; observing a 129th child fails at the containing directory, so
`failure_path_sha256` binds that directory rather than an iteration-dependent child.
Record and byte overflow encode exactly limit-plus-one (`129` and
`2,097,153` respectively); an unavailable count uses `u64::MAX`. Counts never wrap and no traversal-order prefix enters the commitment. The body contains the
complete canonical node sequence once. Both scope identities are independently recomputed
typed `Sc002IncidentPrimaryScopeIdentityDigestV1` values over the incident id, parked
triplet, and that entire ordered root/node sequence; both must equal each other and the
sequence embedded in the body. Raw filesystem identity is never rendered.

The same cause enum classifies non-authorizing
`DeniedSc002PrimaryEvidenceScopeV1`, but that private status value has no canonical census
bytes, locator, or digest. `enumeration-unavailable`, `identity-unstable`, `depth-limit`, and
record/byte overflow beyond the hard ceiling can occur only there. In particular no
all-zero `0xff` observation or failure-body prefix is serializable; an isolated decoder
negative must reject it before incident or parked binding is considered. These private
states cannot be copied into the serialized bounded-failure body.

The bounded-failure form is admission-capable only when two complete locked recursive walks
cover every descendant beneath every present root, yield the same ordered
`RecursiveNodeObservation` sequence, and compute equal before/after scope identities. It is a
bounded representation, not permission to commit a traversal prefix. The scanner may stream
the observations, but it must still cover the full scope. Its hard work ceiling is 4,096
nodes, 67,108,864 regular-file bytes, and depth 64, where each root is depth zero, a direct
child is depth one, and a node with exactly 64 descendant components is accepted. Observing
any depth-65 node, exceeding either other hard ceiling, losing read
or execute access to any directory, or observing any identity/member change between the two
walks produces no admission-capable commitment and denies request publication, disposition
apply, and successor admission.

Within that hard ceiling, `record-limit`, `byte-limit`, `unknown-name`, and
`record-invalid` may use the bounded-failure encoding only after the two complete stable
walks cover every descendant and the failure path names the first semantic failure in fixed
root-instance/path order. Stable symlink, device, fifo, socket, mount, other-kind, and
noncanonical-name observations use `record-invalid` or `unknown-name` and remain present in
that complete sequence. `enumeration-unavailable`, `identity-unstable`, `depth-limit`, and
either hard count/byte ceiling remain inspectable
irreconcilable causes but are never admission-capable evidence. The operator must restore
readability, stop the changing writer, or move only an injected unrecognized entry outside
the immutable candidate scope until the fixed hard ceiling can cover it, then rerun
`sc002-disposition-request`. Recognized incident evidence is never removed or rewritten.
The closed primary schema itself fits within the 128-observation and 2,097,152-byte complete
limits; reaching a hard ceiling therefore proves unrecognized injected descendants rather
than excessive valid incident history. T589 derives that recognized maximum from the closed
root/path schema and poisons any schema change that could exceed it without an accompanying
contract amendment.
The command recensors from fresh `O_CLOEXEC` fds and either produces a complete
coverage-bound request or returns exit `4` with the unchanged
`restore-primary-evidence-coverage` status projection. It never truncates the scope to make
the request signable.

For a soft bound exceeded inside one directory, the failure path is that directory, the full
walk still includes every descendant, and the saturated count is exact limit-plus-one. No
filesystem iteration prefix is an authorization input. A continuing mutation, changed
failure root/path/cause, incomplete walk, or inability to reproduce the same full recursive
failure exits `4` with the same stable incident projection and publishes nothing. At
successor admission the scanner repeats both complete walks and requires the failure cause,
root, canonical failing-path digest, saturated counts, full descendant set, and current
recursive scope identity to equal the committed values. Copying a failure body to another
incident or parked triplet, changing any primary descendant name or bytes, or replaying the
old body after a recursive scope-identity change blocks admission. The two-byte `0x01 0xff`
spelling is detection-only poison and is never a valid primary-evidence census, disposition
input, resolution record, or successor authority.

Duplicate locators, unknown names, unstable identities represented as a complete body,
native-width integers, joined-path traversal, included resolution/disposition leaves, or any
other encoding refuse. The typed
`Sc002IncidentPrimaryEvidenceCensusDigestV1` is

```text
SHA-256(
  "d2b:sc002:incident-primary-evidence-census:v1\0" ||
  u64be(canonical-census-length) || canonical-census
)
```

No serializer output, display path, errno, or caller-provided ordering enters the preimage.
The shared SC-002 domain-hash golden below covers complete zero-residue, complete mixed
present/absent, and bounded-failure commitment encodings. A disposition for an
irreconcilable incident binds the typed evidence kind and this digest. Consequently
`incident-names-absent` has a canonical complete zero-residue transition and does not
fabricate a nonempty residue census, while malformed metadata, conflicting status bytes,
invalid census structure, and unstable census observations remain retained and
identity-bound.

The exact canonical evidence bytes are durably published before the resolution status at
`evidence-sidecars/sc002/incidents/resolution-evidence/sha256/<incident-id>/<typed-digest>.bin`.
This path is outside the frozen primary-evidence scope. The publisher writes and `fsync`s an
unnamed `O_TMPFILE`, capability-free procfs-fd links that exact inode directly to the final
no-replace name, reopens and revalidates the typed digest and inode, and `fsync`s the parent
plus every changed ancestor through the candidate directory. Only then may the resolution
record embed the same typed digest.

Incident publication is one closed, idempotent protocol under the candidate lock:

1. Open or create and verify every required preimage, anchor, metadata, payload, status,
   resolution, resolution-evidence, residue-staging, and residue directory beneath the held
   candidate fd. `fsync` each newly created directory and every changed ancestor bottom-up
   through the candidate directory before a child name may become authoritative.
2. Publish the canonical incident preimage with the common direct-final protocol: prepare
   the complete canonical bytes in an unnamed `O_TMPFILE`, file-sync and revalidate the
   opened inode, capability-free procfs-fd link that exact inode directly to the final
   no-replace name, final-reopen and match it, then sync the parent and every changed
   ancestor. A crash before the direct link exposes no preimage name. After the link, restart
   accepts only absence or that exact complete final inode and finishes durability; a
   nonidentical final is `preimage-final-conflict`. There is no linked temporary,
   `preimage-publication-pending`, or publication rename.
3. Only after the final preimage is durable, publish the canonical anchor and metadata
   independently with the same unnamed-inode/procfs-fd direct-final protocol. An interrupted
   parent sync is `anchor-final-durability-pending` or
   `metadata-final-durability-pending`; a nonidentical final is
   `anchor-final-conflict` or `metadata-final-conflict`. No partial named leaf exists.
4. Durably precreate every missing payload ancestor and sync each new directory plus every
   changed ancestor bottom-up through the candidate directory. For a legacy named source,
   copy only from its retained verified fd into an unnamed inode and direct-final publish the
   payload; never rename, hardlink, or unlink the source name. Reopen the payload, verify the
   exact metadata-bound digest and bytes, and `fsync` it. A payload-sync failure cannot
   publish `parked` and remains a resumable prefix. The publisher does not create any status
   inode before this payload `fsync` succeeds. It retains the candidate OFD lock through
   payload sync, status and ancestor durability, and final revalidation.
5. Direct-final publish `parked.json`, or complete the no-move residue-copy protocol and
   direct-final publish `mismatch-retained.json`, before returning the typed refusal. Every
   immutable status, residue, freeze, request, disposition, and admission record uses the
   same unnamed-inode, file-sync, procfs-fd direct-final no-replace, final-reopen,
   parent-sync, and all-ancestor-sync protocol.
6. Publish each later primary status or irreconcilable resolution transition as the next
   immutable state file with that same protocol. Never replace, truncate, remove, rename, or
   skip an earlier state.

An existing metadata, payload, or state path is idempotent success only after an fd-relative
reopen through the same held directory authority proves exact bytes and binding. Recovery
has two distinct closed nonterminal variants:

- `recovery-resumable` proves one exact preimage/anchor/metadata-bound next step: a complete
  direct-final preimage, anchor, or metadata final whose parent or ancestor durability is
  interrupted; exact metadata with a retained source present and
  payload absent; exact metadata plus the same retained source name and exact payload whose
  file or directories still require sync; an exact authenticated residue-staging/residue prefix; or
  individually exact primary status leaves with one uniquely reconstructible missing
  contiguous predecessor. `sc002-incident-recover` is advertised only for this variant. It
  resumes that one step and replays the payload file sync plus every required parent and
  ancestor sync even when a final leaf already exists.
- `recovery-irreconcilable` covers a nonidentical final preimage, anchor, or metadata leaf,
  any original metadata-recorded legacy source name absent, source/payload conflict,
  all metadata-recorded names absent, payload identity or destination conflict,
  mutually exclusive or non-reconstructible status branches, and an invalid or unstable
  evidence census. No automatic recovery command is advertised. The authenticated
  disposition path preserves every byte and name, computes and binds the exact complete or
  bounded-failure `CanonicalIncidentPrimaryEvidenceCensusV1`, durably publishes those bytes
  outside the frozen scope, and append-only publishes the separate resolution
  `disposition-validated` record. It may retain verifiable named leaves through the residue
  protocol first, but a zero-residue `incident-names-absent` case transitions directly with
  the canonical absent census. It never requires a fabricated residue, repairs a conflicting
  primary branch, or deletes evidence.

The authenticated evidence-resolution branch is the only path that may advance an
irreconcilable source-absence prefix, and it never relabels that prefix resumable or
fabricates/restores the missing source. `parked` and `mismatch-retained` are terminal cleanup
statuses only when the frozen recursive census proves every original legacy source name
still present at its recorded locator. A resolution `successor-admitted` record remains a
separate authenticated resolution result and cannot satisfy or advertise that terminal
cleanup predicate.

Both variants preserve the same stable incident id, perform no unlink, and block publication
and close. Restart, inspect, recover, apply, and successor admission use one locked
classifier. Before selecting a branch, that classifier reopens the immutable preimage path,
requires every decodable anchor, metadata, primary status, resolution, disposition request,
disposition, freeze, and successor record to carry the byte-identical structured preimage,
and reconstructs the incident id from every common and kind-specific component. If a
primary record is malformed, only a signed disposition request/disposition carrying the
same complete object plus the frozen recursive primary-evidence commitment can drive the
resolution branch. A crash cannot move a state from irreconcilable back to resumable. The
classifier runs before and after every unnamed-inode create/write/file-`fsync`,
direct-final procfs-fd link, final reopen, parent sync, and ancestor sync, and at every later
directory-create, payload-copy, payload-file-`fsync`, residue-copy, primary-status
publication, and resolution publication crash point for all four incident kinds.

Payload or residue census, metadata, the maximal contiguous status, frozen primary-evidence
commitment, and incident-id kind
must agree one-to-one on every restart and census. A missing, unknown, duplicate, cross-kind,
noncontiguous, or mismatched object blocks publication and close but retains the stable
incident id and the closed recovery variant/cause/remediation projection below. A final
metadata conflict is addressed by the canonical incident-id path and exact frozen
primary-evidence census or bounded-failure commitment rather than returned as malformed CLI
syntax.
`tests/golden/delivery/sc002-incident-id-v1.json` contains exactly one canonical incident-id
vector for each of the four enum members and a `canonicalRetiredCensusV1` section with exactly three
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

The identity-ambiguity vector is the positive stage-`3`
`legacy-named-source-unsealed` incident and metadata golden. The independent
`tests/golden/delivery/sc002-observation-stage-case-ids.txt` fixture contains exactly, in
order, `observation-stage/historical-quarantine-1-compatible`,
`observation-stage/historical-retirement-2-compatible`,
`observation-stage/legacy-named-source-unsealed-3`,
`observation-stage/zero-refused`, `observation-stage/unknown-4-refused`,
`observation-stage/preimage-metadata-mismatch-refused`, and
`observation-stage/non-identity-nonnull-refused`. A separately authored literal seven-id
constant equals that file before any case runs. Stages `1` and `2` remain decode-compatible
historical inputs; new legacy observation emits only `3`. Wrong/unknown stages and a stage
that differs between the immutable preimage and metadata refuse before payload or status
publication.

`tests/golden/delivery/sc002-domain-hash-vectors-v1.json` is the one shared byte oracle for
every SC-002 typed hash used by a locator, incident, resolution, successor freeze, request,
or disposition. Its closed ordered `digests` array has exactly these nineteen ids:

```text
activation-receipt-content
retirement-id
observed-identity
retired-census
incident-id-retirement-id-collision
incident-id-retirement-census-exhausted
incident-id-retirement-census-invalid
incident-id-identity-ambiguity
residue-id
primary-evidence-leaf-content
primary-scope-identity
primary-evidence-failure-path
primary-evidence-complete
primary-evidence-bounded-failure
successor-freeze
disposition-request
disposition-authority
disposition-verification-key
disposition-id
```

Its closed `signatures` array has exactly one id,
`incident-disposition-signature`. Every digest entry records `id`, `domainAscii`,
`domainHex`, `payloadHex`, the applicable fixed-width framing fields, `preimageHex`, and
`digestHex`; the signature entry additionally records the test verification key, unsigned
canonical object, signing preimage, and signature. Consumers reconstruct each preimage from
semantic inputs and compare every byte. They never hash the stored preimage, import a domain
string from the vector, or define a second expected digest beside this file. The receipt
content-address fixture, all four incident vectors, complete and bounded-failure
primary-evidence vectors, disposition fixture, schemas, and human/JSON goldens refer to these
ids and must agree with the same oracle.

The accepted Version 2 gate rejects a missing, extra, duplicated, reordered, or unknown
vector id; wrong domain spelling or terminator; raw or unframed receipt hash; wrong
width/endian/length; tuple or incident-kind substitution; a primary-evidence vector that
includes any resolution leaf; the detection-only `01ff` body used as authority; or a digest
copied across incident, parked triplet, complete/bounded-failure kind, or domain. This shared
golden is normative input to T589, not generated from production constructors.

`Sc002DispositionIdV1` has the same rendered form and is
`SHA-256("d2b:sc002:disposition-id:v1\0" || u64be(disposition-length) ||
canonical-authenticated-disposition)`. Each append-only immutable
`Sc002IncidentStatusV1` state record at the exact status path above records exactly these
fields in this order:
`schemaVersion = 1`, `kind = "sc002-incident-status"`, `incidentKind` as the exact
`Sc002IncidentKindV1` used to derive `incidentId`, `incidentId`,
`preimageLocator`, `incidentPreimage`, `incidentIdPreimageHex`, `parkedCandidateId`,
`parkedContentId`, `parkedSnapshotSha256`, `state`, `cause`, `residueIds`, `dispositionId`,
`successorFreezeSha256`, `dispositionRequestSha256`,
`successorCandidateId`, `successorContentId`, and `successorSnapshotSha256`.
`incidentKind` is immutable across every state transition. Each ID in the disposition and
successor-binding positions is nullable but always present. Freeze, request, disposition,
and successor triplet are all null before disposition validation and all non-null from
`disposition-validated` onward; partial presence is malformed. `residueIds` is an always-present lexicographically sorted array of unique
`Sc002ResidueIdV1` values. It is empty on the verified-payload branch and contains every
retained mismatch residue on the mismatch branch; a later state repeats it byte-for-byte.
`incidentPreimage` and `incidentIdPreimageHex` are the same complete kind-specific preimage
and MUST reconstruct the incident id, parked triplet, content digest, and every applicable
collision, census, count, or ambiguity component byte-for-byte; they must also equal the
immutable object reopened at `preimageLocator`. A status never depends on an unpersisted
in-memory preimage or metadata remaining decodable. Null is the sole absent scalar
representation; omission is malformed.
Durable status never contains a `remediation` field. The maximal valid contiguous state
record is the current durable state. Its two closed transitions are:

```text
parked -> disposition-validated -> successor-admitted
mismatch-retained -> disposition-validated -> successor-admitted
```

`Sc002IncidentResolutionV1` is the separate append-only envelope for an irreconcilable
primary branch. It contains exactly, in order, `schemaVersion = 1`,
`kind = "sc002-incident-resolution"`, `incidentKind`, `incidentId`,
`preimageLocator`, `incidentPreimage`, `incidentIdPreimageHex`, `parkedCandidateId`, `parkedContentId`,
`parkedSnapshotSha256`, `recoveryClass =
"recovery-irreconcilable"`, the exact closed `cause`,
`primaryEvidenceKind`, `primaryEvidenceSha256`, `primaryEvidenceLocator`,
`primaryEvidenceFailureCause`, sorted unique `residueIds`, `state`, `dispositionId`,
`successorFreezeSha256`, `dispositionRequestSha256`,
`successorCandidateId`, `successorContentId`, and `successorSnapshotSha256`. Its only
contiguous transition is `disposition-validated -> successor-admitted`; the freeze, request,
disposition, and successor triplet are already all non-null in the first record and repeat
byte-for-byte in the second. Admission state, not a late triplet, is the only changed field.
It is accepted only when
the authenticated disposition binds the same incident and parked/successor triplets and the
locked primary-evidence bytes and typed digest revalidate exactly.
`primaryEvidenceKind` is closed to `complete-census` or
`bounded-failure-commitment`; `primaryEvidenceFailureCause` is null for a complete census
and otherwise the exact closed bounded-failure cause. `primaryEvidenceLocator` is the exact
candidate-relative resolution-evidence path derived from the typed digest, never a
caller-supplied or raw-hash locator. An existing resolution record is idempotent only for
exact bytes and the same persisted evidence object. A changed primary name or scope identity
after disposition validation is a conflict and cannot admit a successor.
Primary status remains immutable and need not be structurally valid for this resolution
branch; its exact raw bytes and locators are retained in the frozen primary-evidence
commitment.
The resolution's structured preimage, hex preimage, and locator must agree exactly even
when primary anchor or metadata bytes are malformed; the authenticated disposition supplies
the same complete object and cannot omit any kind-specific component. Resolution JSON uses the same canonical UTF-8, field-order, integer, lowercase-hex,
unknown-field, create-exclusive, leaf-sync, no-replace, final-reopen, parent-sync, and
all-ancestor-sync rules as primary status and is at most 32,768 bytes.

`Sc002IncidentCauseV1` is closed to the four `Sc002IncidentKindV1` spellings plus
`preimage-final-durability-pending`, `preimage-final-conflict`,
`anchor-final-durability-pending`, `anchor-final-conflict`,
`metadata-final-durability-pending`, `metadata-final-conflict`,
`source-present-payload-absent`, `payload-file-sync-pending`,
`payload-present-status-absent`, `source-payload-conflict`, `incident-names-absent`,
`payload-identity-mismatch`, `payload-destination-conflict`,
`status-prefix-repairable`, `status-branch-conflict`, `residue-staging-pending`,
`residue-status-absent`, `evidence-census-conflict`, and the bounded
`primary-evidence-coverage:<failure-class>:<root-class>` family defined below. A
verified-payload status keeps its incident kind as `cause`.
A mismatch status freezes the recovery cause that selected external retention and carries a
nonempty residue census. An irreconcilable resolution may instead carry an empty residue
census only for `incident-names-absent`; its primary-evidence census proves the exact authenticated
absence. A nonterminal CLI projection derives exactly one recovery class and one recovery
cause from the locked census in the listed first-applicable order; it never accepts a caller
class, cause, or free-form explanation.

Coverage denial never collapses to the generic `evidence-census-conflict` cause. Its
operator-safe cause is the bounded string
`primary-evidence-coverage:<failure-class>:<root-class>`, where `failure-class` is exactly
`enumeration-unavailable`, `identity-unstable`, `depth-limit`,
`node-hard-ceiling`, or `byte-hard-ceiling`, and `root-class` is exactly `preimage`,
`anchor`, `metadata`, `payload`, `residue-staging`, `residue`, `primary-status`, or
`source-slot`. The root class intentionally omits the source-slot instance, relative path,
identity, errno, count, and bytes. The private denied scope supplies both enum values; no
caller or serialized bounded-failure body does.

CLI JSON is deliberately a separate deterministic projection rather than the durable status
envelope. `Sc002IncidentCliStatusV1` records, in order, `schemaVersion`,
`kind = "sc002-incident-cli-status"`, `incidentKind`, `incidentId`,
`parkedCandidateId`, `parkedContentId`, `parkedSnapshotSha256`, `state`, `cause`,
`residueIds`, `dispositionId`, the three successor IDs, nullable
`resolutionEvidenceKind`, nullable `resolutionEvidenceSha256`, and final required
`remediation`. It deliberately omits `preimageLocator`, `incidentPreimage`,
`successorFreezeSha256`, `dispositionRequestSha256`, and
`incidentIdPreimageHex` from operator output while
every durable status or resolution persists that complete structured preimage. The two resolution
evidence fields are both null outside an irreconcilable resolution flow and otherwise expose
only the bounded kind plus typed digest needed by the external disposition workflow; no raw
locator or primary bytes are rendered. Its `state` is the exact durable primary/resolution
state when one exists and otherwise one of the distinct closed `recovery-resumable` or
`recovery-irreconcilable` variants. The remediation closed values are
`obtain-incident-disposition`, `resume-incident-recovery`,
`restore-primary-evidence-coverage`, `apply-incident-disposition`, `admit-successor`, and
`none`. `restore-primary-evidence-coverage` is selected when unreadability, instability,
depth 65, or either hard work ceiling prevented two complete equal walks. In that projection
both resolution-evidence fields are null because no signable evidence exists yet.
Projection reopens the immutable anchor and metadata and, where canonical, validates their
complete matching preimage; it also reopens the payload/source/residue crash prefix, durable status, resolution,
and disposition namespace under the same candidate lock, then selects. When the primary
anchor or metadata is structurally invalid, projection does not decode it as authority: it
revalidates the authenticated disposition and
resolution incident kind/preimage, recomputes the stable id and parked triplet, and binds the
exact raw metadata/path bytes through the locked frozen primary-evidence census or
identity-bearing bounded-failure commitment:

| Validated state and census | `cause` | `remediation` |
| --- | --- | --- |
| exact `recovery-resumable` prefix: directly linked complete preimage final with interrupted parent durability, incomplete anchor or metadata publication, source only, payload file/directory sync pending, exact payload without base status, exact residue prefix, or uniquely repairable status prefix | exact locked resumable cause | `resume-incident-recovery` |
| `recovery-irreconcilable`, including `evidence-census-conflict`, with no disposition matching the current complete census or bounded-failure commitment | exact locked irreconcilable cause | `obtain-incident-disposition` |
| same `recovery-irreconcilable` state with an exact matching durable authenticated disposition | exact locked irreconcilable cause | `apply-incident-disposition` |
| `recovery-irreconcilable` whose recursive scan is unreadable, unstable, depth 65, or beyond either hard work ceiling | exact `primary-evidence-coverage:<failure-class>:<root-class>` | `restore-primary-evidence-coverage` |
| `parked`, null disposition/successor IDs, no matching durable authenticated disposition | incident kind | `obtain-incident-disposition` |
| `parked`, null disposition/successor IDs, exact matching durable authenticated disposition present after a publication-before-status crash | incident kind | `apply-incident-disposition` |
| `mismatch-retained`, null disposition/successor IDs | frozen recovery cause | `apply-incident-disposition` |
| `disposition-validated`, non-null disposition/freeze/request IDs and non-null signed successor triplet | inherited incident kind or frozen recovery cause | `admit-successor` |
| `successor-admitted`, non-null disposition ID and successor triplet | inherited incident kind or frozen recovery cause | `none` |
| resolution `disposition-validated`, exact persisted complete census or bounded-failure commitment, non-null disposition/freeze/request IDs, and non-null signed successor triplet | frozen irreconcilable cause | `admit-successor` |
| resolution `successor-admitted`, exact persisted complete census or bounded-failure commitment and successor triplet | frozen irreconcilable cause | `none` |

The grouped rows above do not leave individual causes implicit. Every closed cause has this
inspect/action/status/successor path:

| Cause | Inspect state | Required action | Durable status reached | Successor path |
| --- | --- | --- | --- | --- |
| `retirement-id-collision` | `parked` | request and apply the exact disposition | primary `disposition-validated` | admit the freeze/request-bound successor |
| `retirement-census-exhausted` | `parked` | request and apply the exact disposition | primary `disposition-validated` | admit the freeze/request-bound successor |
| `retirement-census-invalid` | `parked` | request and apply the exact disposition | primary `disposition-validated` | admit the freeze/request-bound successor |
| `identity-ambiguity` | `parked` | request and apply the exact disposition | primary `disposition-validated` | admit the freeze/request-bound successor |
| `preimage-final-durability-pending` | `recovery-resumable` | run incident recover | complete durable preimage, then `parked` or the next exact prefix | continue its projected request/apply/admit action |
| `preimage-final-conflict` | `recovery-irreconcilable` | request/apply against complete recursive evidence while retaining both expected and conflicting bytes | resolution `disposition-validated` | admit the same frozen successor |
| `anchor-final-durability-pending` | `recovery-resumable` | run incident recover | `parked` | request/apply, then admit |
| `anchor-final-conflict` | `recovery-irreconcilable` | request/apply against complete recursive evidence | resolution `disposition-validated` | admit the same frozen successor |
| `metadata-final-durability-pending` | `recovery-resumable` | run incident recover | `parked` | request/apply, then admit |
| `metadata-final-conflict` | `recovery-irreconcilable` | request/apply against complete recursive evidence | resolution `disposition-validated` | admit the same frozen successor |
| `source-present-payload-absent` | `recovery-resumable` | run incident recover | `parked` | request/apply, then admit |
| `payload-file-sync-pending` | `recovery-resumable` | run incident recover and replay every required sync | `parked` | request/apply, then admit |
| `payload-present-status-absent` | `recovery-resumable` | run incident recover | `parked` | request/apply, then admit |
| `source-payload-conflict` | `recovery-irreconcilable` | request/apply against complete recursive evidence | resolution `disposition-validated` | admit the same frozen successor |
| `incident-names-absent` | `recovery-irreconcilable` | request/apply the complete zero-residue census | resolution `disposition-validated` | admit the same frozen successor |
| `payload-identity-mismatch` | `recovery-irreconcilable` | retain representable names, otherwise request/apply complete recursive evidence | primary or resolution `disposition-validated` | admit the same frozen successor |
| `payload-destination-conflict` | `recovery-irreconcilable` | retain representable names, otherwise request/apply complete recursive evidence | primary or resolution `disposition-validated` | admit the same frozen successor |
| `status-prefix-repairable` | `recovery-resumable` | run incident recover | maximal contiguous primary state | continue its projected request/apply/admit action |
| `status-branch-conflict` | `recovery-irreconcilable` | request/apply against complete recursive evidence without editing either branch | resolution `disposition-validated` | admit the same frozen successor |
| `residue-staging-pending` | `recovery-resumable` | run incident recover | `mismatch-retained` | apply, then admit |
| `residue-status-absent` | `recovery-resumable` | run incident recover | `mismatch-retained` | apply, then admit |
| `evidence-census-conflict` with complete stable descendant coverage | `recovery-irreconcilable` | request/apply the complete census or admission-capable bounded commitment | resolution `disposition-validated` | admit only after full recursive replay |
| exact `primary-evidence-coverage:<failure-class>:<root-class>` | `recovery-irreconcilable` | complete the exact coverage-repair procedure below, then rerun inspect; do not run disposition request while this cause remains | unchanged nonterminal projection with null resolution evidence until two complete equal walks fit the hard ceiling | request/apply/admit only after coverage; admission is denied before then |

Every row is exercised independently. Inspect always exits `0` with the same stable human
or JSON projection. The action column is selected only from the locked state: an absent
matching disposition projects `obtain-incident-disposition`, a matching durable disposition
projects `apply-incident-disposition`, and `disposition-validated` projects
`admit-successor`. A request refusal caused by incomplete descendant coverage exits `4`,
retains the same incident id/cause/state, publishes no freeze or request, and names the
specific remediation above. No row ends at an uninspectable parked prefix, generic retry,
or terminal state without either successor admission or an explicit coverage guard that
denies it.

`restore-primary-evidence-coverage` is a procedure selector, not a signing command. The
exact procedure is selected only by the bounded failure class:

| Failure class | Exact repair procedure | Required completion check |
| --- | --- | --- |
| `enumeration-unavailable` | `restore-primary-evidence-access`: the registered owner of the projected root class restores the prior owner/mode/mount read and execute contract without editing recognized evidence | rerun `sc002-incident-inspect`; the same coverage cause must disappear |
| `identity-unstable` | `quiesce-primary-evidence-writer`: the registered owner stops the non-d2b writer changing that root class and leaves recognized evidence byte-identical | rerun `sc002-incident-inspect`; two fresh walks must match |
| `depth-limit` | `relocate-unrecognized-primary-evidence-subtree`: the registered owner moves only the injected unrecognized depth-65 subtree outside the immutable candidate root | rerun `sc002-incident-inspect`; every remaining descendant must fit depth 64 |
| `node-hard-ceiling` | `relocate-unrecognized-primary-evidence-subtree`: the registered owner moves only injected unrecognized entries outside the immutable candidate root | rerun `sc002-incident-inspect`; the full walk must fit 4,096 nodes |
| `byte-hard-ceiling` | `relocate-unrecognized-primary-evidence-subtree`: the registered owner moves only injected unrecognized entries outside the immutable candidate root | rerun `sc002-incident-inspect`; the full walk must fit 67,108,864 bytes |

The incident CLI never performs these owner repairs and never removes, rewrites, or
re-owns recognized evidence. If the registered owner cannot complete its named procedure,
the operator escalates to that owner with the stable incident id and bounded root class;
there is no force flag. Only a later inspect projection with
`obtain-incident-disposition` may advertise or permit `sc002-disposition-request`.

The `parked` row's cause is its incident kind. Every locked incident census maps to exactly
one row. Invalid CLI syntax or a noncanonical caller-supplied ID exits `2`; an absent
canonical stable id exits `3`. A complete direct-final metadata or anchor whose parent or
ancestor durability was interrupted is `metadata-final-durability-pending` or
`anchor-final-durability-pending`; a complete directly linked preimage final in that state is
`preimage-final-durability-pending`; a nonidentical final preimage is
`preimage-final-conflict`; a nonidentical final anchor is
`anchor-final-conflict`; a nonidentical final metadata leaf is `metadata-final-conflict`;
individually exact status leaves with one uniquely
reconstructible missing predecessor are `status-prefix-repairable`; mutually exclusive,
nonidentical, or ambiguous branches are `status-branch-conflict`. Stored corruption is never
collapsed into syntax exit `2` or a path-only diagnostic.
The strict schemas are
`docs/reference/schemas/delivery/sc002-incident-preimage-v1.schema.json`,
`docs/reference/schemas/delivery/sc002-incident-anchor-v1.schema.json`,
`docs/reference/schemas/delivery/sc002-incident-metadata-v1.schema.json`,
`docs/reference/schemas/delivery/sc002-incident-status-v1.schema.json`,
`docs/reference/schemas/delivery/sc002-incident-resolution-v1.schema.json`, and
`docs/reference/schemas/delivery/sc002-incident-cli-status-v1.schema.json`. The anchor, status, and
CLI JSON goldens prove that anchor and metadata reconstruct every incident id, statuses form only the
two allowed append-only contiguous primary branches, resolutions form only the
irreconcilable two-state branch, `remediation` is rejected from durable status and
resolution,
required exactly once as the last CLI field, derived by the table rather than accepted from
a caller or stored status, and never free-form. They also prove that every recovery cause is
reachable by one fixture, has exactly one of the two nonterminal state variants, and projects
the same stable incident id in human and JSON modes.

Human output is a separate exact line projection of the validated CLI status. It is always
these thirteen newline-terminated lines in this order:

```text
incident-kind: <INCIDENT_KIND>
incident-id: <INCIDENT_ID>
parked-candidate-id: <PARKED_CANDIDATE_ID>
parked-content-id: <PARKED_CONTENT_ID>
parked-snapshot-sha256: <PARKED_SNAPSHOT_SHA256>
state: <STATE>
cause: <CAUSE>
disposition-id: <DISPOSITION_ID_OR_NONE>
successor-candidate-id: <SUCCESSOR_CANDIDATE_ID_OR_NONE>
successor-content-id: <SUCCESSOR_CONTENT_ID_OR_NONE>
successor-snapshot-sha256: <SUCCESSOR_SNAPSHOT_SHA256_OR_NONE>
remediation: <REMEDIATION>
next-command: <NEXT_COMMAND>
```

The angle-bracket tokens describe bounded data fields; they are not printed literally.
Nullable IDs render exactly `none`. `NEXT_COMMAND` is selected only by `remediation`:
`resume-incident-recovery` renders `sc002-incident-recover`,
`obtain-incident-disposition` renders `sc002-disposition-request`,
`apply-incident-disposition` renders `sc002-incident-apply`, `admit-successor` renders
`sc002-successor-admit`, `restore-primary-evidence-coverage` renders `none`, and `none`
renders `none`. It never contains a path, flag, ID,
argument, shell fragment, executable path, or free-form sentence. The JSON projection
contains the same bounded values in the declared field order and no `nextCommand` or
guidance field.

`resume-incident-recovery` means invoke `sc002-incident-recover` with the stable incident id.
That command acquires the same candidate lock and resumes only `recovery-resumable`. It
cannot accept a replacement path, identity, disposition, successor, or deletion request. It
exits `0` only after durably reaching and revalidating `parked`, `mismatch-retained`, or the
uniquely repaired contiguous status prefix, including payload file sync and all ancestor
syncs. If a fresh locked census reclassifies the incident as irreconcilable, it exits `4`,
prints the same stable projection with `obtain-incident-disposition` or
`apply-incident-disposition`, and performs no mutation or unlink. Recover is never the
advertised command for `recovery-irreconcilable`.
`obtain-incident-disposition` directs the operator to the disposition authority/workflow
pinned by accepted Version 2; the operator first runs inspect with `--json`, then runs
`sc002-disposition-request` with one clean successor snapshot and submits the exact canonical
request output. That request includes the same locked incident projection, nullable typed
resolution-evidence pair, durable successor freeze, complete structured incident preimage,
and exact successor triplet. Inspect output alone is not a signing request. No repository
command mints or self-signs the disposition record.
`apply-incident-disposition` means run or replay `sc002-incident-apply` with the already
obtained record and the same successor snapshot used to create the signed request. From
`parked` it appends the authenticated transition. From a
recovery-irreconcilable prefix with representable named leaves it first completes durable
no-unlink residue retention and publishes `mismatch-retained`, then appends the same
authenticated transition. For zero-residue, `anchor-final-conflict`, malformed-metadata,
conflicting-status, invalid-census, or unstable-census
states it durably publishes the exact complete primary-evidence census or identity-bearing
bounded-failure commitment and then publishes the separate resolution
`disposition-validated` record without modifying primary evidence. A stale disposition or a
scope that changes during apply exits `4`, emits a fresh inspect projection, and writes
nothing. Every successful apply therefore moves the prescribed state to a terminal
authenticated disposition from which successor admission is reachable.
`admit-successor` means invoke
`sc002-successor-admit` with the validated disposition id and fresh successor snapshot.

Successor selection is frozen before any authority signs. The operator runs
`sc002-disposition-request` with the parked snapshot, stable incident id, and one clean
successor snapshot. Under the candidate lock, the command derives the successor
candidate/content/snapshot triplet from that exact immutable snapshot, rejects equality with
the parked candidate, and recursively proves that it contains no copied SC-002 receipt,
retired, preimage, anchor, metadata, payload, residue, status, resolution, freeze, request,
or disposition bytes. It then direct-final publishes one canonical
`Sc002SuccessorFreezeV1` at
`evidence-sidecars/sc002/incidents/successor-freezes/sha256/<incident-id>/<freeze-sha256>.json`.
The freeze contains exactly, in order, `schemaVersion = 1`,
`kind = "sc002-successor-freeze"`, `incidentKind`, `incidentId`, `preimageLocator`,
`incidentPreimage`, the parked triplet, the successor triplet, nullable
`resolutionEvidenceKind`, nullable `resolutionEvidenceSha256`, and
`deliveryContractSpecSha256`. Its typed digest is:

```text
Sc002SuccessorFreezeDigestV1 =
  SHA-256(
    "d2b:sc002:successor-freeze:v1\0" ||
    u64be(canonical-freeze-length) || canonical-freeze
  )
```

  The canonical freeze is at most 16,384 bytes.

After final reopen plus leaf/parent/every-ancestor sync, the same command publishes the
canonical unsigned `Sc002IncidentDispositionRequestV1` at
`evidence-sidecars/sc002/incidents/disposition-requests/sha256/<incident-id>/<request-sha256>.json`
before publishing an operator output. Its canonical JSON has exactly these 19 fields in this
order:

| Ordinal | Request field | Rule |
| --- | --- | --- |
| 1 | `schemaVersion` | integer `1` |
| 2 | `kind` | literal `sc002-incident-disposition-request` |
| 3 | `action` | literal `abandon-candidate-admit-successor` |
| 4 | `incidentKind` | exact `Sc002IncidentKindV1` |
| 5 | `incidentId` | canonical `Sc002IncidentIdV1` |
| 6 | `preimageLocator` | exact immutable preimage locator |
| 7 | `incidentPreimage` | complete canonical `Sc002IncidentPreimageV1` |
| 8 | `incidentIdPreimageHex` | exact lowercase-hex preimage that recomputes the incident id |
| 9 | `parkedCandidateId` | exact parked candidate id |
| 10 | `parkedContentId` | exact parked content id |
| 11 | `parkedSnapshotSha256` | exact parked snapshot digest |
| 12 | `successorCandidateId` | exact distinct frozen successor candidate id |
| 13 | `successorContentId` | exact frozen successor content id |
| 14 | `successorSnapshotSha256` | exact frozen successor snapshot digest |
| 15 | `resolutionEvidenceKind` | null, `complete-census`, or `bounded-failure-commitment` |
| 16 | `resolutionEvidenceSha256` | null with field 15, otherwise its exact typed digest |
| 17 | `deliveryContractSpecSha256` | exact accepted Version 2 content digest |
| 18 | `successorFreeze` | complete canonical `Sc002SuccessorFreezeV1` object |
| 19 | `successorFreezeSha256` | exact typed digest of field 18 |

It contains no `dispositionRequestSha256`, authority, verification-key, or signature field
and therefore cannot claim authentication. The canonical request is at most 65,536 bytes.
It uses the same canonical UTF-8, no-BOM/no-whitespace/no-trailing-byte, field-order,
shortest ASCII escaping, lowercase-hex, unsigned-integer, duplicate/missing/unknown-field,
and decode-then-reencode equality rules as the disposition. A request unequal to its
canonical re-encoding is malformed before its digest is accepted.
Its typed digest is:

```text
Sc002DispositionRequestDigestV1 =
  SHA-256(
    "d2b:sc002:disposition-request:v1\0" ||
    u64be(canonical-request-length) || canonical-request
  )
```

  The authority workflow accepts only this complete request and performs one exact
  transformation. It independently verifies the embedded freeze and field 19, the incident
  projection or fully descendant-covering primary-evidence commitment, accepted contract
  digest, and complete structured preimage. The disposition copies request field 1; substitutes
  only field 2 with `kind = "sc002-incident-disposition"`; copies request fields 3 through 17
  byte-for-byte in the same order; omits the embedded field-18 freeze object after reopening
  and verifying it; copies request field 19 as disposition field
  `successorFreezeSha256`; inserts the independently recomputed request digest as
  `dispositionRequestSha256`; inserts the disposition-pinned `authoritySha256` and
  `verificationKeySha256`; and appends `signatureEd25519`. No other field is synthesized,
  renamed, reordered, omitted, or copied from a caller. The signature covers that exact
  22-field disposition with only its final signature field omitted. The disposition validator
  performs the inverse comparison against the reopened request and freeze before signature
  acceptance. No prose request, inspect output without a freeze, caller-supplied successor
  triplet, or post-signing successor selection is eligible. Regenerating a request after any
  incident scope or successor triplet change yields a different request digest and requires a
  new signature.

  Request-output preparation begins before candidate-internal publication and creates no
  output name. `PATH` is resolved once from an `O_PATH|O_DIRECTORY|O_CLOEXEC` fd for `.`
  when relative or `/` when absolute. Every parent is opened with
  `openat2(RESOLVE_BENEATH|RESOLVE_NO_SYMLINKS|RESOLVE_NO_MAGICLINKS)` and
  `O_PATH|O_DIRECTORY|O_CLOEXEC`; the final component must be one canonical nonempty
  component, and `.`, `..`, repeated separators, NUL, symlink parents, and a symlink leaf
  refuse. No joined path is reopened after resolution. The command verifies the exact
  privilege-dropped credentials, requires an empty effective capability set, validates
  `/proc/self/fd` as procfs through a retained directory fd, records the output parent
  mount/device/inode, and opens `O_TMPFILE|O_RDWR|O_CLOEXEC` in that parent. It sets and
  verifies current-effective-uid ownership and mode `0600`, writes the exact canonical
  bytes, `fsync`s the unnamed inode, and revalidates its bytes and identity. Unsupported
  open, invalid procfs, nonzero effective capabilities, or any prepublication mismatch
  refuses with zero output namespace and zero freeze/request mutation.

  Only after that complete unnamed output inode exists does the command durably publish and
  sync the candidate-internal successor freeze, request leaf, and every changed candidate
  ancestor. It then performs the first and only link-support check by calling
  `linkat(proc_self_fd_dirfd, decimal_fd, output_parent_fd, final_name,
  AT_SYMLINK_FOLLOW)`. This capability-free procfs-fd form links the exact opened inode
  directly to the final no-replace output name. It does not use `AT_EMPTY_PATH`, a linked
  temporary, `renameat2`, or a create-and-unlink preflight. An unsupported-link result is an
  ordinary idempotent output failure: it creates no output name, exits `4`, and retains the
  already durable candidate-internal freeze/request unchanged. This ordering is deliberate;
  an unsupported-link case cannot both occur after candidate durability and prove zero
  freeze/request mutation.

  After a successful direct link, the command reopens the final leaf with
  `O_RDONLY|O_CLOEXEC|O_NOFOLLOW`, requires the same device/inode as the still-open unnamed
  file description plus exact owner/mode/link/digest/bytes, and `fsync`s the output parent.
  Before returning, it re-resolves the canonical parent components from the same
  root/cwd anchor with the same openat2 policy and requires the same mount/device/inode. A
  renamed or replaced parent is a conflict even though all data remains preserved. It never
  truncates, replaces, follows, or name-consumingly renames an output.

  Exact replay is crash-safe. An exact already-published final leaf returns `0` after reopen,
  full verification, and parent `fsync`. If the final leaf is absent, replay prepares a new
  unnamed inode and reuses the already durable candidate-internal freeze/request. A
  nonidentical final leaf, unexpected type/owner/mode/link count, output-parent replacement,
  or final inode mismatch exits `4` and preserves the existing name. A crash before unnamed
  inode `fsync`, after that `fsync` but before candidate publication, or after candidate
  publication but before the direct link exposes no output name. A crash after the direct
  link but before parent `fsync` may recover either no final or the exact complete final;
  replay handles both without changing the internal request digest. Request-output failure
  never rolls back or edits an already durable candidate-internal freeze/request, so
  rerunning the identical command reproduces the exact authority input without minting
  another request digest.

`Sc002IncidentDispositionV1` is the sole accepted disposition record. The accepted Version 2
contract also pins two closed typed constructors:

```text
Sc002DispositionAuthorityDigestV1 =
  SHA-256(
    "d2b:sc002:disposition-authority:v1\0" ||
    u64be(canonical-authority-binding-length) ||
    canonical-authority-binding
  )

Sc002DispositionVerificationKeyDigestV1 =
  SHA-256(
    "d2b:sc002:disposition-verification-key:v1\0" ||
    u64be(32) || verification_key[32]
  )
```

`canonical-authority-binding` is the exact canonical UTF-8 JSON object with fields, in order,
`schemaVersion = 1`, `kind = "sc002-disposition-authority"`, bounded ASCII `authorityId`,
`deliveryContractSpecSha256`, and `verificationKeySha256`. It excludes
`authoritySha256` and signatures, so it cannot contain itself. It is not a display name or a
caller-selected selector. `authorityId` is 1-64 lowercase ASCII letters, digits, or hyphens,
begins with a letter, and ends with a letter or digit. Both constructors are present in the shared SC-002 golden and
accept no caller-provided domain.

The disposition's canonical JSON object has exactly these fields in this order:

| Field | Type and rule |
| --- | --- |
| `schemaVersion` | integer `1` |
| `kind` | literal `sc002-incident-disposition` |
| `action` | literal `abandon-candidate-admit-successor` |
| `incidentKind` | exact `Sc002IncidentKindV1` |
| `incidentId` | canonical `Sc002IncidentIdV1` |
| `preimageLocator` | exact immutable kind-bearing preimage locator for this incident |
| `incidentPreimage` | complete canonical `Sc002IncidentPreimageV1`, including every applicable kind-specific component |
| `incidentIdPreimageHex` | complete lowercase-hex kind-specific preimage that recomputes `incidentId` |
| `parkedCandidateId` | lowercase 64-hex id equal to the parked status |
| `parkedContentId` | lowercase 64-hex id equal to the parked status |
| `parkedSnapshotSha256` | lowercase 64-hex digest equal to the parked status |
| `successorCandidateId` | lowercase 64-hex id distinct from the parked candidate id |
| `successorContentId` | lowercase 64-hex id for the fresh successor |
| `successorSnapshotSha256` | lowercase 64-hex digest for the fresh successor snapshot |
| `resolutionEvidenceKind` | null for a valid primary terminal; otherwise `complete-census` or `bounded-failure-commitment` |
| `resolutionEvidenceSha256` | null for a valid primary terminal; otherwise the exact lowercase 64-hex `Sc002IncidentPrimaryEvidenceCensusDigestV1` selected by `resolutionEvidenceKind` |
| `deliveryContractSpecSha256` | exact content digest for accepted `ADR-046-validation-and-delivery` Version 2 in the regenerated spec-set manifest |
| `successorFreezeSha256` | exact `Sc002SuccessorFreezeDigestV1` for the pre-signing immutable successor freeze |
| `dispositionRequestSha256` | exact `Sc002DispositionRequestDigestV1` for the authority request whose fields this record preserves |
| `authoritySha256` | exact `Sc002DispositionAuthorityDigestV1` pinned by that accepted Version 2 contract |
| `verificationKeySha256` | exact `Sc002DispositionVerificationKeyDigestV1` of the Ed25519 public key pinned to that authority |
| `signatureEd25519` | lowercase 128-hex Ed25519 signature, final field |

The encoded record is canonical UTF-8 JSON with no BOM, whitespace, or trailing newline and
is at most 32,768 bytes. Fields occur only in the order above. Strings are ASCII with the
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
contract, then verifies the signature. It independently decodes
`incidentIdPreimageHex`, requires its domain to equal `incidentKind`, recomputes the incident
id and parked triplet, compares them with the signed fields and complete embedded
`incidentPreimage`, reopens the durable freeze and request by their typed digests, and
requires every request field and both triplets to match byte-for-byte. A key, authority,
successor, freeze, or request supplied only by the disposition file is never trusted as a
selector.

`sc002-incident-apply` opens `--disposition PATH` exactly once with
`O_RDONLY|O_CLOEXEC|O_NOFOLLOW`, requires a regular single-link file owned by the current
effective uid with mode exactly `0600`, reads at most 32,769 bytes, hashes before decode, and
validates from those same bytes. T589 owns one private, nonserializable
`ValidatedSc002IncidentDisposition` result with no public constructor, fields, `Clone`,
`Copy`, `Default`, serde implementation, or conversion from the decoded DTO. Apply also
once-opens `--successor-snapshot PATH`, derives its triplet, and requires it to equal the
durable freeze, request, and signed disposition before consuming that result by value.
While holding the candidate lock, it direct-final publishes the exact authenticated bytes at
`evidence-sidecars/sc002/dispositions/sha256/<disposition-id>.json` through the same
unnamed-inode, file-sync, procfs-fd no-replace link, final-reopen, parent-sync, and
all-ancestor-sync protocol.
From `parked` it may then transition directly. From `recovery-irreconcilable` with
representable named evidence it must first complete and sync the residue protocol and append
`mismatch-retained`; from zero-residue or unusable primary anchor/metadata/status it instead
requires the disposition-bound complete primary-evidence census or identity-bearing
bounded-failure commitment, publishes and syncs those exact canonical bytes outside the
frozen primary scope, and publishes the separate resolution `disposition-validated` record.
Only then may it expose authenticated external disposition. An existing path is
accepted only when an fd-relative reopen through the same held directory authority proves
the exact bytes and binding; otherwise it is a
conflict. `sc002-successor-admit` reopens and revalidates the same preimage, freeze, request,
disposition, and successor snapshot through the same validator before consuming the result
and publishing `successor-admitted`. Each disposition
and status or resolution publication `fsync`s its leaf and every changed ancestor directory bottom-up
through the candidate directory. A crash before any required ancestor sync cannot advance
the maximal contiguous state; a crash after it replays the same transition idempotently.
Every `disposition-validated` primary status or resolution already persists the freeze
digest, request digest, and signed successor triplet. Admission repeats those values
byte-for-byte and changes only the state, so no restart gap can select a different successor.
An irreconcilable apply additionally requires a non-null matching resolution-evidence kind
and digest before it may publish the resolution record; a primary-terminal apply requires
both null. A raw `0x01 0xff` sentinel, stale or copied bounded-failure commitment, fabricated
zero-residue proof, included resolution leaf, or cross-branch value refuses without changing
either branch. Successor admission revalidates the persisted canonical evidence bytes,
and current scope identity before it appends `successor-admitted`.

The incident candidate remains permanently ineligible at every transition. A validated
disposition binds the incident, complete structured preimage, parked triplet, pre-signing
successor freeze, disposition request, distinct successor triplet, accepted Version 2
contract digest, authority, key, and signature. Apply and admit both rederive that same
successor triplet from the same immutable successor snapshot; neither accepts a later
substitution. It never authorizes unlink, incident
deletion, publication of the parked record, a binding panel request, reservation release,
or reuse of evidence. Successor admission requires a fresh snapshot with no copied SC-002
receipt, incident, retired, status, or disposition bytes. For `adr046w5`, it admits only
T220's nonbinding replacement-candidate and exact-candidate evidence path while preserving
the byte-identical retained request and reservation; T219's separate external
retained-request disposition remains mandatory. Any other wave whose binding request was
already consumed must stop for its external wave disposition rather than use this flow.

The strict schemas are
`docs/reference/schemas/delivery/sc002-successor-freeze-v1.schema.json`,
`docs/reference/schemas/delivery/sc002-incident-disposition-request-v1.schema.json`, and
`docs/reference/schemas/delivery/sc002-incident-disposition-v1.schema.json`. The canonical
freeze, request, and signed disposition fixtures are
`tests/golden/delivery/sc002-successor-freeze-v1.json`,
`tests/golden/delivery/sc002-incident-disposition-request-v1.json`, and
`tests/golden/delivery/sc002-incident-disposition-v1.json`; they use only a checked-in test
key. The accepted Version 2 contract digest, authority digest, key digest, unsigned signing
bytes, signature, and derived disposition id are asserted independently. Closed negatives
cover 32,769 bytes; unknown, missing, duplicate, and reordered fields; noncanonical JSON;
wrong version, envelope kind, action, incident kind, incident preimage/domain/framing,
id width/case, incident, parked triplet, successor triplet, resolution evidence kind/digest,
successor freeze, disposition request, or contract
digest; equal parked/successor candidate ids; successor snapshot changed before apply or
admit; request produced before the final locked scope; missing, unknown, copied, or wrong
authority/key; unsigned, malformed, wrong-domain, wrong-key, and post-signature-tampered
records; replay against another incident, candidate, content, snapshot, request, freeze, or contract
generation; replacement between open/hash/decode/publish/reopen; stale state; conflicting
same-id bytes; copied SC-002 evidence in the successor; and post-signing successor
substitution at apply or admission. Every negative refuses before a
state transition, request, reservation release, incident deletion, or candidate admission.
The durable-status, resolution, and CLI-status schemas plus human/JSON fixtures additionally exercise
every `Sc002IncidentKindV1`, require status `incidentKind` to match the id domain and durable
payload, residue, or primary-evidence commitment, consume all four incident vectors and every
retired-census vector from `tests/golden/delivery/sc002-incident-id-v1.json`, consume the
shared typed digest oracle in
`tests/golden/delivery/sc002-domain-hash-vectors-v1.json`, and cover every row of the deterministic
cause/remediation table, every terminal primary/resolution branch, and every recovery cause.

T589 owns the planned delivery CLI contract:
`wave sc002-incident-inspect --snapshot PATH --incident-id ID [--json]`,
`wave sc002-incident-recover --snapshot PATH --incident-id ID [--json]`,
`wave sc002-disposition-request --snapshot PATH --incident-id ID
--successor-snapshot PATH --request-out PATH [--json]`,
`wave sc002-incident-apply --snapshot PATH --incident-id ID --disposition PATH
--successor-snapshot PATH [--json]`,
and
`wave sc002-successor-admit --snapshot PATH --incident-id ID --disposition-id ID
--successor-snapshot PATH [--json]`. Exit `0` means the requested read or transition
completed, exit `2` means invalid syntax or malformed input, exit `3` means the stable
incident/disposition/successor ID was not found, and exit `4` means stale state, conflict,
or blocked admission; no other stable exit is assigned. Reapplying the exact authenticated
disposition after `disposition-validated`, recreating the exact freeze/request to an
identical mode-`0600` output, recovering an already parked or later incident,
or readmitting the exact already admitted successor exits `0` after full durable
revalidation and makes no write; a different disposition, successor, or binding exits `4`.
Inspect exits `0` and projects every `recovery-resumable`,
`recovery-irreconcilable`, `parked`, `mismatch-retained`,
`disposition-validated`, or `successor-admitted` primary or resolution state with its stable
incident id, cause, and deterministic remediation. Recover is offered only for
`recovery-resumable` and advances the uniquely determined durable step; for an
irreconcilable cause it exits `4`, preserves every name, and emits the same inspect
projection directing the authenticated disposition path. Apply completes the no-unlink
residue transition when possible or binds the exact complete census or bounded-failure
commitment when primary state is unusable, then publishes authenticated disposition. Every prescribed next command either
advances to its advertised terminal state or returns the same stable projection after a
concurrent state change; repeated exact commands converge idempotently. Every exit `4`
reached after a valid incident id emits the same human or JSON status projection; invalid
CLI syntax or a noncanonical caller ID exits `2`, an absent stable id exits `3`, and stored
metadata/status corruption is an inspectable irreconcilable state rather than malformed
input.
Human output is the exact thirteen-line
projection above. JSON is the closed `Sc002IncidentCliStatusV1` projection above, not the
durable `Sc002IncidentStatusV1` envelope; its required final `remediation` field is derived
by the closed table and has no free-form counterpart. The original cleanup refusal and every
later refusal carry the same stable incident id, cause, and remediation as bounded data
fields, so the operator can invoke inspect without discovering an internal path. T589's focused parser,
state-transition, metadata/status/resolution-path, durable-status/resolution/CLI-schema, human/JSON golden,
disposition-schema/signature, exit, crash, stale-ID, replay, and no-request/no-unlink tests
own this contract. Its
existing `changelog.d/resource-api-production.md` fragment carries the operator-visible
delivery recovery entry, all five command nouns, the four exits, and the parked-candidate
successor requirement. Before T589 dispatch, a separate external specification-amendment
workflow must bump accepted `ADR-046-validation-and-delivery` from Version 1 to Version 2,
pin this complete command, both census byte grammars/goldens, typed canonical receipt hash,
durable-status/resolution/CLI projection, complete incident metadata/preimage/path,
payload-file and all-ancestor durability, residue/publication/recovery/resolution, closed
state/cause/remediation table, every primary and resolution branch, disposition authority,
canonical record, one-lock cleanup exclusion, retention owner, and validator contract,
receive
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
Crash injection covers source open, hash, decode, OFD-lock acquisition, every incident
namespace create/sync, and every preimage, anchor, metadata, payload-copy, residue-copy,
resolution-evidence, status, resolution, successor-freeze, request, disposition, and
admission unnamed-inode create/write/file-sync/direct-final-link/final-reopen/parent-sync/
ancestor-sync boundary, including unsupported procfs and link outcomes after an earlier
durable write-ahead record. It also covers recursive scope walk/replay, cleanup-parent sync,
ephemeral-residue census, and record publication.
It also injects parent-boundary loss, source replacement, and destination-reopen mismatch
before the legacy-source evidence-copy boundary. No current hook performs a quarantine
rename, retirement rename, payload rename, residue rename, or sidecar-data unlink.
Synchronized tests cover the complete candidate-writer/cleanup/retention actor matrix below,
legacy source replacement or disappearance before evidence copy, parent-boundary loss,
destination-reopen mismatch, same-bytes/same-record idempotence, retained historical
retirement collision/census validation, and different-bytes or wrong-binding races. Each
overlap uses two independently opened descriptions of the same
verified lock inode and the named latch/owner orderings below. The loser must
receive the live-owner refusal before namespace access; after release, exactly one retry may
advance only after reopening and recensoring every namespace; it may not retain a pre-lock
leaf fd or identity observation. Every case proves bounded completion with no deadlock, no existing-name rename, no
sidecar-data unlink, and an exact final census within 64 leaves and 1,048,576 bytes. A
direct-final ordinary winner or loser leaves both ephemeral namespaces empty; a legacy
incident retains only names included in its frozen recursive census.
Every identity-ambiguous verified terminal case proves the exact
preimage/anchor/metadata/payload/parked-status prefix is durable and every legacy source
name is retained. Every terminal cleanup-mismatch case proves preimage plus anchor and
metadata, a complete frozen census of retained ephemeral and staging names, exact durable
residue ids, and `mismatch-retained`. A replacement-raced case is classified exactly once as `recovery-resumable` or
`recovery-irreconcilable`, with every still-named leaf preserved and no false terminal status
until its advertised recovery or authenticated disposition path completes. Zero-residue,
retired-source, malformed-metadata, conflicting-status, invalid-census, and unstable-census
cases prove exact complete/bounded-failure primary-evidence binding and
resolution-to-successor convergence. All primary and resolution incident states
block publication and close for the incident candidate.

The independent fault case
`legacy-source/absent-after-metadata-before-payload-copy` durably publishes exact metadata,
removes only the original legacy source through the injector, leaves payload absent, and
restarts. It must classify `recovery-irreconcilable`, advertise no recover command, preserve
every remaining name, append no terminal cleanup status, and permit progress only through
the authenticated evidence-resolution branch.

The serialization oracle uses the closed writer set `importer`, `cleanup`,
`incident-recover`, `disposition-request`, `incident-apply`, and `successor-admit`, plus
`retention-guard`. It contains every `writer/cleanup` and `cleanup/writer` pair, the distinct
`cleanup/cleanup` pair, and every `writer/retention-guard` and
`retention-guard/writer` pair; same-input and different-input fixtures are present where the
pair admits both. This explicitly serializes cleanup against every live owner rather than
testing importer ownership as a proxy. It covers first-actor and second-actor lock ownership
and latches at `unnamed-inode-created`,
`unnamed-inode-file-synced`, `direct-final-linked`,
`incident-preimage-published`, `incident-anchor-published`, `incident-metadata-published`,
`incident-payload-copied`,
`incident-payload-file-synced-before-status-inode`,
`incident-residue-staged`, `incident-residue-finalized`, and
`incident-status-published`, `resolution-evidence-published`,
`incident-resolution-published`, `successor-freeze-published`,
`disposition-request-published`, and `successor-status-published`. Every actor opens its own
file description for the one verified lock inode. This includes two cleanup workers targeting the same leaf and two cleanup
workers targeting different candidate leaves beneath the same candidate; there is no
per-leaf cleanup lock. A nonblocking contender must return `sc002-sidecar-owner-live` with
`namespace_open_count = 0`, `namespace_mutation_count = 0`, and
`critical_section_max = 1`; a blocking restart contender may enter only after release.
After release, exactly one retry acquires a fresh description, recensors under lock, and
linearizes after the winner. Tests assert the complete case id set rather than counting
dynamically generated cases, and each case records one of the two allowed serial histories.
No test may pass by timing out, skipping a latch, sharing an open file description, retaining
a pre-lock namespace fd, inspecting a live owner's namespace, or constructing,
serializing, cloning, returning beyond its guard, pairing with another guard, or
reconstructing `SidecarCleanupOwner<'guard>` from an fd.

The exact latch `incident-payload-file-synced-before-status-inode` fires only after the
reopened payload fd has completed `fsync` and its content still matches metadata, while no
parked-status unnamed inode has been created. Creating any parked-status inode before that
latch or firing the latch before payload `fsync` completes fails the ordering oracle.

At every durable reopen, the validator resolves the locator beneath the already held
candidate-directory fd with
`openat2(RESOLVE_BENEATH|RESOLVE_NO_SYMLINKS|RESOLVE_NO_MAGICLINKS|RESOLVE_NO_XDEV)`, opens
the leaf once with `O_RDONLY|O_CLOEXEC|O_NOFOLLOW`, requires a regular single-link file owned
by the current effective uid with mode exactly `0600` and stable device/inode identity,
computes the exact typed receipt-content digest, and compares it with the locator's typed
digest before decoding from the same opened fd. A replacement between lookup, digest,
decode, or later-stage reopen is a
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
legacy named-source identity mismatch, cleanup-parent or incident anchor/metadata/payload/status leaf,
resolution-evidence or resolution leaf/parent/ancestor sync failure, payload-file sync failure, unexpected
ephemeral residue after an ordinary terminal,
or any durable incident entry also refuses. An identity mismatch instead passes only its
negative oracle: no sidecar-data unlink; either a complete durable
anchor/metadata/payload/parked-status terminal, a complete durable
anchor/metadata/residue/mismatch-retained terminal, a disposition-bound irreconcilable resolution,
or an inspectable resumable/irreconcilable all-names-preserved prefix; stable
cause/remediation projection and publication/close denial hold in every state.
Compatibility tests decode
retained schema-v2 `EvidenceRecord` fixtures
byte-identically, import a failed operator record without a receipt, and prove that the same
failed record remains ineligible for every close stage.

The receipt and census negative registries are flat, checked-in, and independent of
production enumeration:

- `tests/golden/delivery/sc002-activation-receipt-negative-case-ids.txt` contains exactly
  these 61 newline-terminated ids in this order:

  ```text
  receipt/size-over
  receipt/invalid-utf8
  receipt/bom
  receipt/whitespace
  receipt/trailing-newline
  receipt/non-ascii
  receipt/alternate-escape
  receipt/field-missing
  receipt/field-duplicate
  receipt/field-reordered
  receipt/field-unknown
  receipt/version
  receipt/kind
  receipt/candidate
  receipt/content
  receipt/snapshot
  receipt/validation
  receipt/outcome
  receipt/clock
  receipt/integer-negative
  receipt/integer-fractional
  receipt/integer-exponent
  receipt/integer-leading-zero
  receipt/integer-out-of-range
  receipt/sample-missing
  receipt/sample-duplicate
  receipt/sample-extra
  receipt/sample-reordered
  receipt/sample-unknown-identity
  receipt/effect-identity
  receipt/ready-identity
  receipt/selected-stop-identity
  receipt/selected-stop-source
  receipt/selected-stop-not-later
  receipt/selected-stop-tie-not-ready
  receipt/elapsed-mismatch
  receipt/elapsed-overflow
  receipt/elapsed-over-budget
  receipt/progress-empty
  receipt/progress-over-32
  receipt/progress-identity
  receipt/progress-kind
  receipt/progress-at-start
  receipt/progress-after-stop
  receipt/progress-misordered
  receipt/outer-binding-stale
  receipt/locator-digest-mismatch
  receipt/absent-input-on-passed
  receipt/input-on-failed
  receipt/input-on-other-validation
  receipt/caller-locator
  receipt/source-absolute
  receipt/source-traversal
  receipt/source-url
  receipt/source-symlink
  receipt/source-hardlink
  receipt/source-owner
  receipt/source-mode
  receipt/source-replacement
  receipt/duplicate-durable-leaf
  receipt/failed-record-positive-receipt
  ```

-   `tests/golden/delivery/sc002-census-negative-case-ids.txt` contains exactly these 73
  newline-terminated ids in this order:

  ```text
  retired-census/version
  retired-census/body-tag
  retired-census/record-count
  retired-census/path-length
  retired-census/path-order
  retired-census/duplicate-path
  retired-census/entry-tag
  retired-census/observation-tag
  retired-census/failure-tag
  retired-census/unavailable-identity
  retired-census/unavailable-size
  retired-census/unavailable-content
  retired-census/partial-over-bound
  primary-census/version
  primary-census/body-tag
  primary-census/truncated-header
  primary-census/incident-mismatch
  primary-census/parked-triplet-mismatch
  primary-census/anchor-missing
  primary-census/anchor-mismatch
  primary-census/record-count
  primary-census/locator-length
  primary-census/locator-truncated
  primary-census/locator-outside-scope
  primary-census/resolution-leaf-included
  primary-census/disposition-leaf-included
  primary-census/duplicate-locator
  primary-census/locator-order
  primary-census/presence-tag
  primary-census/absent-length
  primary-census/absent-digest
  primary-census/present-length
  primary-census/present-digest
  primary-census/unknown-primary-name
  primary-census/symlink
  primary-census/hardlink
  primary-census/unstable-as-complete
  primary-census/over-record-limit-as-complete
  primary-census/over-byte-limit-as-complete
  primary-census/failure-cause
  primary-census/failure-commitment-partial
  primary-census/failure-scope-identity
  primary-census/failure-saturation
  primary-census/failure-copied-cross-incident
  primary-census/raw-01ff-authority
  primary-census/root-missing
  primary-census/root-duplicate
  primary-census/root-instance-tag
  primary-census/source-slot-root-missing
  primary-census/source-slot-root-duplicate
  primary-census/source-slot-root-order
  primary-census/directory-node-missing
  primary-census/directory-digest-nonzero
  primary-census/file-digest-zero
  primary-census/node-kind-tag
  primary-census/node-kind-mode-mismatch
  primary-census/unavailable-node-tag
  primary-census/unavailable-node-fields
  primary-census/symlink-target-digest
  primary-census/device-rdev
  primary-census/device-as-complete
  primary-census/fifo-as-complete
  primary-census/socket-as-complete
  primary-census/mount-as-complete
  primary-census/descendant-parent-missing
  primary-census/descendant-omitted
  primary-census/failure-node-observation
  primary-census/failure-node-list-omitted
  primary-census/bounded-failure-node-order
  primary-census/bounded-failure-incomplete-descendants
  primary-census/bounded-failure-hard-node-ceiling
  primary-census/bounded-failure-hard-byte-ceiling
  primary-census/depth-65-hard-ceiling
  ```

The retained `locator-*` and `presence-*` spellings are stable negative-case ids, not a
second wire grammar. Their exact meanings under the recursive grammar are:
`record-count` tests `node-count`; `locator-length` and `locator-truncated` test recursive
relative-path framing; `locator-outside-scope` tests a root/path outside the closed root
table; `duplicate-locator` tests a duplicate root-code/root-instance/relative-path tuple; `locator-order`
tests root-code, root-instance, then unsigned-path order; `presence-tag` tests `node-kind`; `absent-length`
tests an absent root with a nonempty path or nonzero size; `absent-digest` tests any nonzero
absent-root identity/content field; `present-length` tests regular-file length against the
once-opened bytes; and `present-digest` tests the typed regular-file content digest. Tests
must not implement those ids by decoding the retired flat locator/presence shape.

`tests/golden/delivery/sc002-primary-census-v1.json` is the independently authored byte
oracle for this grammar. It contains exactly eight ordered vectors:
`all-twelve-roots-absent`, `mixed-recursive-valid`, `all-invalid-node-kinds`,
`stable-soft-bound-full-coverage`, `enumeration-unavailable-denied`,
`identity-unstable-denied`, `depth-64-accepted`, and `depth-65-denied`. The invalid-node
vector has distinct symlink, block-device, character-device, fifo, socket, mount, and other
observations; nonzero `st_uid`, `st_gid`, and both device `st_rdev` values; and a
symlink-target payload digest. The enumeration-unavailable vector has no serialized node or
bounded body and must fail the isolated all-zero-`0xff` decoder poison. Each vector records semantic inputs, exact
`RecursiveNodeObservation` bytes, the complete or bounded body when one is eligible, both
scope identities, the framed typed digest when eligible, and an explicit
`admissionCapable` boolean. The two denied vectors and depth-65 vector have no resolution
evidence digest. Tests reconstruct every byte from semantic fields and compare against this
file; production reads none of it.

A separately authored literal array in the test module must equal each file before any
negative can count. The receipt encoder, census encoders, poison builders, and production
validators may read neither file nor the literal arrays. Every id must reach its named
semantic check after all earlier canonical/authentication predicates that are not under test
pass. Missing, duplicate, extra, reordered, unknown, dynamically skipped, or unvisited ids,
an early unrelated failure, or a generated expectation fails the enforcing runner. A
separate post-resolution mutation case changes a primary name after a valid bounded-failure
commitment and must block successor admission; it is a state-transition negative, not a
malformed encoding and therefore is not conflated with the 73 malformed census ids.

The retained filename
`tests/golden/delivery/sc002-request-output-negative-case-ids.txt` is the closed direct-final
publication registry. It contains exactly these 35
newline-terminated ids in order:

```text
request-output/path-empty
request-output/path-dot
request-output/path-dotdot
request-output/path-repeated-separator
request-output/path-nul
request-output/parent-symlink
request-output/parent-replaced
request-output/leaf-symlink
preimage-publication/unsupported-open
preimage-publication/unsupported-proc-fd-link
receipt-import/unsupported-open
receipt-import/proc-fd-mount-invalid
receipt-import/unsupported-proc-fd-link
receipt-import/foreign-eexist
receipt-import/crash-before-inode-fsync
receipt-import/crash-after-inode-fsync-before-final-link
receipt-import/crash-after-final-link-before-parent-fsync
receipt-import/post-link-final-reopen-inode-mismatch
receipt-import/exact-replay-after-parent-fsync
request-output/unsupported-open
request-output/unsupported-proc-fd-link
request-output/proc-fd-mount-invalid
request-output/final-type
request-output/final-owner
request-output/final-mode
request-output/final-hardlink
request-output/final-bytes
request-output/final-inode-mismatch
request-output/foreign-eexist
request-output/crash-before-inode-fsync
request-output/crash-after-inode-fsync-before-internal-request
request-output/crash-after-internal-request-before-final-link
request-output/crash-after-final-link-before-parent-fsync
request-output/exact-replay-after-parent-fsync
request-output/descriptor-exec-leak
```

A separately authored literal 35-id constant equals this file before any publication test
runs. The importer, path resolver, output publisher, crash injector, descriptor tracker, and
production code read neither expectation. Every case reaches only its named openat2,
ownership, mode, link, content, inode, no-replace, durability, replay, or CLOEXEC check. Each
exact replay case is a positive idempotence oracle and must perform zero writes after full
final-leaf and parent durability verification. Missing, duplicate, reordered, skipped,
early-failing, or production-derived cases fail the enforcing runner.

The two preimage refusal cases prove no incident name, sidecar mutation, freeze, or request.
The nine receipt-import cases are read-independent from the request-output cases.
Unsupported open, invalid procfs/mount identity, and unsupported direct link prove zero
receipt leaf and zero `EvidenceRecord` mutation. The crash cases prove no visible leaf before
the link, only absence or the exact complete final after the link, conflict preservation,
and zero-write replay after final-parent durability. The importer-specific
`post-link-final-reopen-inode-mismatch` case replaces the destination only after the direct
link and before the importer's final reopen; the importer must refuse, preserve the observed
destination and source, perform no repair/unlink/relink, and publish zero `EvidenceRecord`
mutation.
The request-output unsupported-open and invalid-procfs cases run before internal
publication and prove zero output namespace plus zero freeze/request mutation. The
request-output unsupported-link case runs only after the candidate-internal freeze/request
and all candidate ancestors are durable; it proves zero output namespace mutation and exact
retention of that internal pair. Treating it as a zero-freeze/request case would contradict
the candidate-first durability invariant and fails the crash matrix.

`tests/golden/delivery/sc002-recovery-forbidden-values.tsv` is the independent closed
redaction registry for the incident/request/output path. It contains exactly these seventeen
tab-separated, newline-terminated rows and no header:

```text
request-out-path	/tmp/d2b-sc002-request-canary.json
request-temp-name	.d2b-sc002-request-canary.tmp
successor-snapshot-path	/tmp/d2b-sc002-successor-canary
disposition-input-path	/tmp/d2b-sc002-disposition-canary.json
preimage-locator	evidence-sidecars/sc002/incidents/preimages/canary
source-locator	evidence-sidecars/sc002/canary-source
payload-locator	evidence-sidecars/sc002/incidents/payload/canary
resolution-evidence-locator	evidence-sidecars/sc002/incidents/resolution-evidence/canary
failure-relative-path	canary/private/descendant
filesystem-device	818181
filesystem-inode	828282
filesystem-ctime-sec	838383
filesystem-uid	848484
filesystem-gid	858585
filesystem-rdev	868686
symlink-target-bytes	d2b-sc002-symlink-target-canary-v1
canonical-request-body	d2b-sc002-canonical-request-body-canary-v1
```

Each literal is injected independently through inspect, recover, request, request-output
replay, apply, admission, cleanup, and retention owners. It must be absent from human and
JSON status, wire diagnostics, error and `Display`, log fields/messages, audit,
tracing events and span attributes, metric name/help/label/value/exemplar, panic, and every
`Debug`. The exact `--request-out` file, exact disposition input fixture, candidate-private
canonical request leaf, and the test's private injection buffer are the only scan
exclusions; no path, prefix, process, directory, or broad evidence-sidecar exclusion is
allowed. Bounded incident/disposition/successor ids may appear only in their declared
operator status fields. Metrics contain none of those ids and no recovery-state path. A
missing visit, changed literal, duplicate row, omitted captured surface, unexpected
projection, or production read of this fixture fails before evidence acceptance. Raw
`st_uid`, `st_gid`, `st_rdev`, and symlink-target bytes therefore have no observability
exception: they may enter only the typed internal census preimage and the private injection
buffer, never a rendered or captured surface. Each of the seventeen literals is injected
separately into every captured surface named above; a generic stat or payload canary cannot
stand in for either field-specific row.

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
The table above is the complete digest registry. Implementations select a typed registry
member before hashing; they never accept a caller-provided domain string. `u32be` and
`u64be` are fixed-width unsigned big-endian encodings, every text length covers the exact
canonical ASCII or UTF-8 octets that follow it, and concatenated objects retain the listed
order. The accepted Version 2 vectors contain one positive preimage/digest pair for every
registry row plus boundary vectors for zero and maximum in-range integer spellings, text
length measured in octets, reordered objects, unknown tags/fields/versions, missing or
duplicated frames, trailing bytes, and cross-domain substitution. A vector consumer
recomputes the framed bytes independently rather than hashing a stored expected preimage.
The exact checked-in file is
`tests/golden/delivery/source-floor-v1/hash-vectors-v1.json`. It has a closed ordered
`digests` array of exactly 15 entries, one for each registry row above, and a closed ordered
`signatures` array of exactly four entries, one for each signature domain below. Every digest
entry records `id`, `domainAscii`, `domainHex`, `payloadHex`, `payloadLength`,
`payloadLengthU64BeHex`, `preimageHex`, and `digestHex`; every signature entry records the
same domain and unsigned-object framing fields plus `verificationKeyHex`,
`signingPreimageHex`, and `signatureHex`. The ASCII domain must encode to the listed hex and
end in exactly one `00`; `preimageHex` must equal domain bytes, the eight-byte length frame,
and payload bytes with no omitted or duplicate frame. Consumers reconstruct both arrays from
semantic inputs and compare every byte and id. They do not hash the stored `preimageHex` or
use the vector file as a production registry. Wrong vector count/order/id, domain spelling or
terminator, payload encoding, length width/endian/value, preimage concatenation, digest,
verification key, or signature fails the external Version 2 gate.

The digest ids, in order, are `accepted-disposition`, `base-system-closure-nar`,
`source-generation`, `producer-authority`, `installer-authority`, `validator-authority`,
`verification-key`, `member-content`, `manifest`, `installation-receipt`,
`validation-receipt`, `import-receipt`, `member-census`, `validated-floor`, and
`aggregate-floor`. The signature ids, in order, are `manifest`, `installation`,
`validation`, and `import`. No alias, repeated id, omitted row, or extension id is accepted.

`tests/golden/delivery/source-floor-v1/receipt-negative-case-ids.txt` independently pins the
complete structural and transition negative registry. It contains exactly these 32
newline-terminated ids in this order:

```text
source-floor-receipt/floor-version
source-floor-receipt/floor-kind
source-floor-receipt/field-missing
source-floor-receipt/field-duplicate
source-floor-receipt/field-reordered
source-floor-receipt/field-unknown
source-floor-receipt/noncanonical-json
source-floor-receipt/invalid-utf8
source-floor-receipt/trailing-byte
source-floor-receipt/integer-negative
source-floor-receipt/integer-fractional
source-floor-receipt/integer-out-of-range
source-floor-receipt/integer-leading-zero
source-floor-receipt/text-non-ascii
source-floor-receipt/text-length-mismatch
source-floor-receipt/digest-width
source-floor-receipt/digest-case
source-floor-receipt/domain-unknown
source-floor-receipt/frame-missing
source-floor-receipt/frame-duplicated
source-floor-receipt/frame-width
source-floor-receipt/frame-endian
source-floor-receipt/frame-length
source-floor-receipt/cross-domain
source-floor-receipt/manifest-count
source-floor-receipt/transition-skipped
source-floor-receipt/transition-reordered
source-floor-receipt/transition-repeated
source-floor-receipt/authority-transition
source-floor-receipt/binding-stale
source-floor-receipt/import-wrong-c
source-floor-receipt/import-wrong-q
```

A separately authored literal 32-id constant must equal this file. Neither input may be
generated from or read by the floor decoder, transition machine, schema, hash-vector
consumer, poison generator, or 91-case role matrix. Each fixture recomputes every unaffected
enclosing digest and signature and must reach only its named decoder, framing, transition,
authority, or binding check. This registry, the independent 13-row role matrix, the 91-case
semantic poison registry, the five-case copied-issuer registry, the 26-case
issuer-authentication/capability registry, the 21-case hash-vector registry, and the
15-digest/four-signature oracle are seven independent expectations; success in one cannot supply the
expected ids, cardinality, bytes, or visits of another.

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
`d2b:source-floor:import-signature:v1\0`. The authority digest, verification-key digest, and actual verifier key must all match the
accepted disposition before signature verification; the object never selects its verifier.
Validation and import use fresh signatures under their distinct domains even when the
accepted disposition pins the same validator key. Missing, copied, wrong-key,
cross-transition, or binding-stale proofs refuse before fd transfer, authorization, or
mutation.

Copied-digest rejection is a closed five-case matrix:
`source-floor/copied-issuer/manifest`,
`source-floor/copied-issuer/installation`,
`source-floor/copied-issuer/validation`,
`source-floor/copied-issuer/import`, and
`source-floor/copied-issuer/all`. Each attacked proof copies both expected authority and
verification-key digests into an otherwise canonical object, signs the exact correct-domain
preimage with a different valid test key, recomputes every enclosing digest, and re-signs
every unaffected outer object with its independently pinned legitimate test key. The
single-transition cases must pass canonical decoding, framing, every enclosing hash, and all
unaffected issuer checks before failing signature verification under the disposition-pinned
key for the named transition. The `all` case must report the complete four-transition
issuer-failure set. A stale enclosing hash, invalid outer proof, wrong transition domain, or
early structural failure makes the poison fixture invalid and cannot count. A copied digest,
including one enclosed by otherwise valid outer hashes and signatures, is never issuer
provenance. T589's independent expected-id file is
`tests/golden/delivery/source-floor-v1/issuer-proof-negative-case-ids.txt`; neither the
production validator nor the poison generator may read it.

`tests/golden/delivery/source-floor-v1/issuer-authentication-negative-case-ids.txt`
closes every remaining issuer/capability negative. It contains exactly these 26
newline-terminated ids in order:

```text
source-floor-auth/missing-proof/manifest
source-floor-auth/missing-proof/installation
source-floor-auth/missing-proof/validation
source-floor-auth/missing-proof/import
source-floor-auth/wrong-key/manifest
source-floor-auth/wrong-key/installation
source-floor-auth/wrong-key/validation
source-floor-auth/wrong-key/import
source-floor-auth/cross-domain/manifest
source-floor-auth/cross-domain/installation
source-floor-auth/cross-domain/validation
source-floor-auth/cross-domain/import
source-floor-auth/rebound/execution-commit
source-floor-auth/rebound/feature-snapshot
source-floor-auth/rebound/source-generation
source-floor-auth/direct-decoded-chain
source-floor-auth/protected-origin-serialize
source-floor-auth/protected-origin-clone
source-floor-auth/protected-origin-copy
source-floor-auth/protected-origin-replay
source-floor-auth/issuer-provenance-serialize
source-floor-auth/issuer-provenance-clone
source-floor-auth/validated-floor-serialize
source-floor-auth/validated-floor-clone
source-floor-auth/validated-floor-repeated-mint
source-floor-auth/handoff-serialized-revalidation
```

A separately authored literal 26-id constant must equal the file. The twelve
transition-specific cases retain canonical bytes, correct framing, and every unaffected
proof; each reaches only its named pinned-key/domain check. The three rebound cases use a
fully signed valid chain from another C, Q, or source generation and fail final binding.
The final eleven are compile-fail/API-surface and state-machine cases proving that direct
decode cannot mint any private authority, the protected origin cannot be serialized,
cloned, copied, or replayed, neither later private result can be serialized or cloned, one
origin cannot mint a second validated floor, and a handoff cannot revalidate serialized
evidence. Production validators, DTO decoders, poison builders, and API-surface discovery
may not read the expected-id file.

`tests/golden/delivery/source-floor-v1/hash-vector-negative-case-ids.txt` closes the byte
oracle negatives. It contains exactly these 21 newline-terminated ids in order:

```text
source-floor-vector/digest-id-missing
source-floor-vector/digest-id-extra
source-floor-vector/digest-id-duplicate
source-floor-vector/digest-id-reordered
source-floor-vector/digest-id-unknown
source-floor-vector/signature-id-missing
source-floor-vector/signature-id-extra
source-floor-vector/signature-id-duplicate
source-floor-vector/signature-id-reordered
source-floor-vector/signature-id-unknown
source-floor-vector/domain-spelling
source-floor-vector/domain-terminator
source-floor-vector/payload
source-floor-vector/frame-width
source-floor-vector/frame-endian
source-floor-vector/frame-length
source-floor-vector/preimage
source-floor-vector/digest
source-floor-vector/verification-key
source-floor-vector/signing-preimage
source-floor-vector/signature
```

A separately authored literal 21-id constant equals the fixture before vectors run. Each
poison changes exactly its named oracle member, reconstructs every unaffected semantic
input independently, and reaches the intended count/order/id/domain/frame/preimage/digest/
key/signature check. The vector consumer, production registry, and fixture are mutually
read-independent; a generated expected list or early schema failure does not count.

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

The table is independently pinned by
`tests/golden/delivery/source-floor-v1/role-artifact-matrix.tsv`: exactly 13 nonempty
newline-terminated ASCII rows in the order above, each encoded as
`<role>\t<artifactId>`. It has no header, comment, blank row, escaping, duplicate, or
trailing column. The production role registry, manifest builder, poison generator, and this
expected matrix are four separately constructed inputs. Production code may not read the
fixture. Tests require exact ordered equality between all four, and a missing, extra,
duplicate, reordered, or changed role/artifact pair fails before the 91-case poison matrix
can count as visited.

`tests/golden/delivery/source-floor-v1/poison-case-ids.txt` is itself pinned rather than
generated from the role table or class list. It contains exactly these 91
newline-terminated ids in this class-major, role-major order:

```text
source-floor/missing/source-daemon-peer
source-floor/missing/source-broker-peer
source-floor/missing/source-wire-schema
source-floor/missing/source-privilege-schema
source-floor/missing/source-operation-catalogue
source-floor/missing/source-operation-catalogue-fingerprint
source-floor/missing/source-compatibility-disposition
source-floor/missing/source-capability-api-fingerprint
source-floor/missing/source-serialization-snapshot
source-floor/missing/source-positive-fixture
source-floor/missing/source-bare-protocol-negative-fixture
source-floor/missing/source-cross-fingerprint-negative-fixture
source-floor/missing/source-installed-apply-object
source-floor/duplicate/source-daemon-peer
source-floor/duplicate/source-broker-peer
source-floor/duplicate/source-wire-schema
source-floor/duplicate/source-privilege-schema
source-floor/duplicate/source-operation-catalogue
source-floor/duplicate/source-operation-catalogue-fingerprint
source-floor/duplicate/source-compatibility-disposition
source-floor/duplicate/source-capability-api-fingerprint
source-floor/duplicate/source-serialization-snapshot
source-floor/duplicate/source-positive-fixture
source-floor/duplicate/source-bare-protocol-negative-fixture
source-floor/duplicate/source-cross-fingerprint-negative-fixture
source-floor/duplicate/source-installed-apply-object
source-floor/extra/source-daemon-peer
source-floor/extra/source-broker-peer
source-floor/extra/source-wire-schema
source-floor/extra/source-privilege-schema
source-floor/extra/source-operation-catalogue
source-floor/extra/source-operation-catalogue-fingerprint
source-floor/extra/source-compatibility-disposition
source-floor/extra/source-capability-api-fingerprint
source-floor/extra/source-serialization-snapshot
source-floor/extra/source-positive-fixture
source-floor/extra/source-bare-protocol-negative-fixture
source-floor/extra/source-cross-fingerprint-negative-fixture
source-floor/extra/source-installed-apply-object
source-floor/empty/source-daemon-peer
source-floor/empty/source-broker-peer
source-floor/empty/source-wire-schema
source-floor/empty/source-privilege-schema
source-floor/empty/source-operation-catalogue
source-floor/empty/source-operation-catalogue-fingerprint
source-floor/empty/source-compatibility-disposition
source-floor/empty/source-capability-api-fingerprint
source-floor/empty/source-serialization-snapshot
source-floor/empty/source-positive-fixture
source-floor/empty/source-bare-protocol-negative-fixture
source-floor/empty/source-cross-fingerprint-negative-fixture
source-floor/empty/source-installed-apply-object
source-floor/stale-generation/source-daemon-peer
source-floor/stale-generation/source-broker-peer
source-floor/stale-generation/source-wire-schema
source-floor/stale-generation/source-privilege-schema
source-floor/stale-generation/source-operation-catalogue
source-floor/stale-generation/source-operation-catalogue-fingerprint
source-floor/stale-generation/source-compatibility-disposition
source-floor/stale-generation/source-capability-api-fingerprint
source-floor/stale-generation/source-serialization-snapshot
source-floor/stale-generation/source-positive-fixture
source-floor/stale-generation/source-bare-protocol-negative-fixture
source-floor/stale-generation/source-cross-fingerprint-negative-fixture
source-floor/stale-generation/source-installed-apply-object
source-floor/stale-digest/source-daemon-peer
source-floor/stale-digest/source-broker-peer
source-floor/stale-digest/source-wire-schema
source-floor/stale-digest/source-privilege-schema
source-floor/stale-digest/source-operation-catalogue
source-floor/stale-digest/source-operation-catalogue-fingerprint
source-floor/stale-digest/source-compatibility-disposition
source-floor/stale-digest/source-capability-api-fingerprint
source-floor/stale-digest/source-serialization-snapshot
source-floor/stale-digest/source-positive-fixture
source-floor/stale-digest/source-bare-protocol-negative-fixture
source-floor/stale-digest/source-cross-fingerprint-negative-fixture
source-floor/stale-digest/source-installed-apply-object
source-floor/cross-disposition/source-daemon-peer
source-floor/cross-disposition/source-broker-peer
source-floor/cross-disposition/source-wire-schema
source-floor/cross-disposition/source-privilege-schema
source-floor/cross-disposition/source-operation-catalogue
source-floor/cross-disposition/source-operation-catalogue-fingerprint
source-floor/cross-disposition/source-compatibility-disposition
source-floor/cross-disposition/source-capability-api-fingerprint
source-floor/cross-disposition/source-serialization-snapshot
source-floor/cross-disposition/source-positive-fixture
source-floor/cross-disposition/source-bare-protocol-negative-fixture
source-floor/cross-disposition/source-cross-fingerprint-negative-fixture
source-floor/cross-disposition/source-installed-apply-object
```

The separately authored literal 91-id test constant must equal this file before any poison
runs. It repeats these bytes directly; it may not form a Cartesian product at runtime or
read `role-artifact-matrix.tsv`.

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
wrong frame, unknown tag/version, and copied-proof negatives.
`hash-vectors-v1.json` is the exact 15-digest/four-signature byte oracle defined above.
The five copied-digest provenance ids are independently pinned and each must pass
canonical-envelope, framing, enclosing-digest, and unaffected-proof validation before
failing issuer authentication at its named transition. The Version 2 amendment,
approval, generated-manifest update, and Gate 0 receipt are external to this feature. The
separate source-generation producer/installer and import/validation authorities implement
and install objects conforming to those accepted schemas and vectors; they do not own or
silently redefine the repository contract artifacts. T589 does not deserialize a floor and
decide for itself. The installed source coordinator acquires one exclusive OFD claim on the
durable unconsumed origin record for the exact source generation, C, Q, and import-receipt
identity and returns one private `ProtectedSourceFloorOrigin`. Acquisition does not consume
the durable origin. The claim uses one preprovisioned lock inode beneath the coordinator's
anchored nonreplaceable root-owned namespace. The held parent is opened
`O_PATH|O_DIRECTORY|O_NOFOLLOW|O_CLOEXEC`; the lock leaf is opened
`O_RDWR|O_NOFOLLOW|O_CLOEXEC`, is a regular root-owned mode-`0600` single-link file, and is
never replaced, renamed, or unlinked. Before and after `F_OFD_SETLK`, acquisition verifies
the held parent and lock device/inode identities against the durable origin binding. A
pathname replacement, second link, parent replacement, no-follow failure, or identity
change refuses before floor authentication or dispatch. The owner retains that exact locked
open file description through final dispatch-file reopen and every parent/ancestor
directory sync.

The owner holds the validated installed-generation fd anchors, the sole locked open file
description, and a claim-incarnation digest. It has no public fields,
constructor, accessor, serde implementation, `Clone`, `Copy`, `Default`, conversion, byte
importer, fd extraction, or reconstruction from a digest or serialized chain. A concurrent
open cannot obtain the OFD claim. After owner death the kernel releases the sole claim; a
new coordinator may reacquire only the same exact origin after proving no durable dispatch
publication consumed it. Copying fields or replaying serialized evidence cannot produce an
owner. The lock description is close-on-exec and is never duplicated or transferred. An
exec-leak test holds the parent claim while a child execs and proves the child inherited no
descriptor; after parent death the child cannot retain the claim. A replacement-race test
holds the claim through dispatch publication and proves a replacement attempt cannot create
an independently lockable inode or a second validated capability.

The exact disposition-pinned validator consumes that process-local protected owner by value while
authenticating all four issuer proofs against the keys selected only by the accepted
disposition and returns one private `AuthenticatedSourceFloorIssuerProvenance` by value.
That intermediate result carries the live origin claim forward and has the same
closed surface. It binds the exact manifest, installation, validation, and import
canonical-byte digests, all four signature domains, the disposition, source generation, C,
and Q. The semantic floor validator consumes it by value and only then may return one
private, nonserializable `ValidatedSourceGenerationCompatibilityFloor` result by value. The
final type has the same closed surface and additionally binds the validated aggregate floor
digest and exact 13-member census. One live claim can therefore produce at most one
validated-floor capability at a time.

Durable consumption occurs only when that final capability is consumed into the one
create-exclusive, file-and-directory-durable handoff dispatch publication. That same record
is also the origin-consumed marker; no second file or split transaction exists. A
canonical/schema, issuer-authentication, semantic, C/Q, or
final-capability-construction error drops the process-local owner and leaves the origin
unconsumed. A crash after claim acquisition, after issuer authentication, after semantic
validation, or after final capability creation but before commit publication releases the
OFD claim and permits exact-origin reacquisition after owner death. A crash after the commit
replays that durable dispatch and never mints another validated-floor capability.
Independently pinned fault-injection cases cover every boundary and assert
`critical_section_max = 1`, one durable dispatch at most, no permanent pre-publication
consumption, no concurrent capabilities, no lock-path replacement, no descriptor exec leak,
and no serialized-evidence revalidation.

Every immutable single-use host-generation publication uses one
`HostGenerationImmutablePublicationV1` protocol: the origin-consuming handoff dispatch,
coordinator-pointer repair and ensure-root pre-mutation/outcome records, reservation and both
release pre-mutation/outcome pairs, retention-anchor pre/candidate/outcome records,
immutable backup members, restoration private evidence, pre-mutation, provenance, outcome,
settlement and repair-resume records, plus backup-prune pre-mutation and outcome records.

Activation owns ordering, not a direct privileged `mkdir`. The existing broker executes the
typed sealed operation `EnsureHostGenerationImmutablePublicationRootV1` before any operation
may resolve a source or target publication descendant. The accepted external source-generation
producer/installer owns the source-generation invocation and orders it before the installed
source broker advertises `source-handoff-v1`. T595 owns the target `host-broker.nix` invocation
and orders it before the target broker accepts adoption and before target `d2bd.service`.
Neither invocation adds a unit or allows Nix activation, the daemon, or a shell helper to
create the root directly. The external source-generation invocation remains an unresolved
prerequisite; target-generation wiring cannot substitute for it.

`EnsureHostGenerationImmutablePublicationRootV1` receives only a private root reference and
the closed activation side `source-generation | target-generation`. Before any audit or
namespace mutation, the broker opens a disposition-sealed trusted ancestor directory,
validates its root ownership, mode, filesystem type, mount id, device, inode, and expected
private reference, and walks every existing parent component from that held dirfd with
`openat2(RESOLVE_BENEATH|RESOLVE_NO_SYMLINKS|RESOLVE_NO_MAGICLINKS|RESOLVE_NO_XDEV)` and
`O_DIRECTORY|O_CLOEXEC`. The exact creation parent is therefore opened and identity-validated
before `mkdirat`; no unanchored path walk, current working directory, joined path, or
post-creation-only validation may select it. After process death the broker discards every
numeric descriptor, reacquires a fresh trusted-ancestor dirfd from the sealed reference,
repeats the complete walk, and revalidates the creation parent before classifying or
continuing the attempt.

It first appends a fixed-field pre-mutation record to the already existing broker audit sink
outside the publication root. That record carries only the typed
`publicationRootOperationSha256 =
SHA-256("d2b:audit:host-generation:publication-root-operation:v1\0" ||
private_operation_id)` and
`publicationRootRefSha256 =
SHA-256("d2b:audit:host-generation:publication-root-ref:v1\0" ||
private_publication_root_ref)`, activation side, expected owner/mode/type/link-count
posture, and prior state `absent | exact`. It then uses `mkdirat`
on the held validated parent to create the root at mode `0700`, or reopens it relative to
that parent, validates the root-owned directory and the unchanged ancestor/parent
mount/device/inode chain, syncs the root, syncs the held parent, reopens and revalidates the
root and parent chain, and appends the matching outcome `created | reopened | degraded`.
No raw operation id, path, timestamp, uid, gid, device, inode, or errno is rendered in
either audit record. The root is usable only after the outcome and both directory syncs are
durable. A second activation run must return `reopened` with zero namespace mutation. Wrong
trusted ancestor, replaced parent, wrong type, owner, mode, link count, parent identity,
symlink, magic-link, cross-mount traversal, or mount identity refuses and preserves the
observed object.

The source and target activation suites independently cover first run, second run, crash
after pre-audit, after root creation, after root sync, after parent sync, after final reopen,
after outcome, and response loss. Every fresh-process restart reacquires the trusted
ancestor, reopens and revalidates the exact parent chain, and either finishes the one attempt
or returns the prior completed response with zero write. A literal
`host-generation-immutable-publication-root-case-ids.txt` fixture and a separately authored
constant enumerate both activation sides at every boundary; production root creation,
fixture enumeration, and the expected constant are mutually read-independent. Missing,
duplicate, reordered, skipped, or unvisited side/boundary cases fail before source or target
broker use.

The fixture contains exactly these 20 newline-terminated ids in this order:

```text
publication-root/source/first-run-created
publication-root/source/second-run-reopened-zero-write
publication-root/source/crash-after-pre-before-mkdir
publication-root/source/crash-after-mkdir-before-root-sync
publication-root/source/crash-after-root-sync-before-parent-sync
publication-root/source/crash-after-parent-sync-before-final-reopen
publication-root/source/crash-after-final-reopen-before-outcome
publication-root/source/crash-after-outcome-before-response
publication-root/source/wrong-parent-refused
publication-root/source/wrong-root-posture-refused
publication-root/target/first-run-created
publication-root/target/second-run-reopened-zero-write
publication-root/target/crash-after-pre-before-mkdir
publication-root/target/crash-after-mkdir-before-root-sync
publication-root/target/crash-after-root-sync-before-parent-sync
publication-root/target/crash-after-parent-sync-before-final-reopen
publication-root/target/crash-after-final-reopen-before-outcome
publication-root/target/crash-after-outcome-before-response
publication-root/target/wrong-parent-refused
publication-root/target/wrong-root-posture-refused
```

The wrong-parent and wrong-root posture cases independently poison trusted-ancestor
identity, creation-parent replacement, type, owner, mode, link count, symlink, magic-link,
cross-mount traversal, and mount identity beneath one case-family visitor; a missing poison
visit fails the fixture. The source and target visitors must each reach both durable
ensure-root record classes and every acquisition/create/sync/reopen/outcome/response
boundary. One shrinkage poison per record class and boundary fails before broker use. The
first-run and every crash case require durable pre/outcome audit, and the second-run case
requires fresh descriptor reacquisition plus zero namespace and audit write after exact
completed replay.

Every operation holds that stable root dirfd and revalidates its mount/device/inode identity
before and after publication. Every descendant is resolved fd-relative with
`openat2(RESOLVE_BENEATH|RESOLVE_NO_SYMLINKS|RESOLVE_NO_MAGICLINKS|RESOLVE_NO_XDEV)`;
every name passed to `mkdirat`, `linkat`, `openat2`, or `unlinkat` is one validated
single-component leaf, and every opened descriptor carries `O_CLOEXEC`.

Every durable private-state change beneath or governing this root is a typed broker operation
or a private sealed suboperation reached from one. The closed set is
`EnsureHostGenerationImmutablePublicationRootV1`,
`ReserveHostGenerationImmutableAuditCapacityV1`,
`BindHostGenerationImmutableAuditRetentionAnchorV1`,
`RepairHostGenerationImmutableAuditContinuityV1`,
`PublishHostGenerationImmutableAuditBackupV1`,
`RestoreHostGenerationImmutableAuditMemberV1`,
`PruneHostGenerationImmutableAuditBackupsV1`, and the already typed dispatch and
coordinator-pointer-repair operations. Each appends fixed digest/enum pre-mutation audit
before its first durable private mutation and one matching fixed outcome afterward. The
reservation, retention anchor and watermark, restoration evidence, effective provenance,
and prune census have no direct filesystem writer, public DTO, daemon shortcut, or activation
shortcut. Canonical member bodies, restoration artifacts, private evidence, root paths, and
clock samples remain broker-private and never enter audit, responses, logs, metrics, spans,
panic, or `Debug`.

Capacity is reserved before every append, not only before the first backup. The publication
root has these simultaneous subset and aggregate ceilings:

| Class | Per-attempt or per-intent ceiling | Root ceiling | Bounded lifecycle |
| --- | --- | --- | --- |
| immutable backup members | 256 records and 16,777,216 encoded bytes per intent | 64 intents, 4,096 members, 268,435,456 bytes | current intent unprunable; replaced intent day 30 through mandatory day 90 |
| restoration private evidence, pre, provenance, settlement, and outcome | 8 records and 1,048,576 bytes per restoration attempt; 256 attempts per intent | included in 32,768 records and 536,870,912 bytes | body-bearing private evidence follows its replaced backup set and is absent by day 90 after a complete exported digest audit |
| continuity-repair body evidence | exactly 1 evidence record and at most 131,072 encoded bytes per attempt; 256 attempts per intent | included in 32,768 records and 536,870,912 bytes | retained through repair replay; removed only after the governed set is durably pruned and the matching pre, watermark, outcome, and prune digest audit export is file-and-directory durable |
| retention anchors, immutable watermarks, reservations, continuity replay metadata, and prune census state | 1 current anchor, 1 current watermark, 1 reservation ledger, and 2,048 records per intent | included in 32,768 records and 536,870,912 bytes | compact only after the governed set is durably pruned and the fixed audit export is durable |
| pending fixed-field audit staging for root operations | 8,192 records and 67,108,864 bytes total | included in 32,768 records and 536,870,912 bytes | append-only export to the existing immutable broker audit segment owner; staging removal only after segment file and directory durability and configured audit retention |

The root-wide record and byte ceilings include every class rather than counting the backup
subset alone. A reservation failure precedes mutation and is typed degradation. Root
operation audit rotates only through the existing append-only broker audit segment owner,
whose record/byte segment bounds and retention are enforced before old durable segments are
removed. No record is overwritten, truncated, or silently dropped to regain capacity.
Every accepted replacement reserves its worst-case later continuity-repair evidence,
immutable watermark, fixed audit, settlement, and compaction charge in
`rootRecordDelta`/`rootEncodedByteDelta`; mandatory continuity repair therefore does not
depend on finding new general capacity at day 90. The reservation ledger also enforces the
continuity evidence one-record/131,072-byte and 256-attempt subset ceilings. A checked
subset or aggregate conversion failure occurs before capacity pre-audit.

Capacity-control audit cannot recursively reserve itself. Initial root creation therefore
atomically installs and charges a broker-private standing capacity-control reserve of eight
fixed records and 65,536 encoded bytes inside the root-wide ceilings before the root becomes
usable. Those bytes are unavailable to backup, restoration, retention, or prune
publication. A reservation or release operation consumes exactly two standing slots for
its pre/outcome pair, never invokes `ReserveHostGenerationImmutableAuditCapacityV1` for
those records, and refuses before mutation when two slots are not free. Export completion
replenishes slots only after the audit segment file and directory are durable; a crash
reconstructs used/free slots from the immutable audit prefix and export state. Missing,
overdrawn, duplicated, or unaccounted standing-reserve state degrades the root and blocks
all mutation. Root-create, pre-only, outcome-publication, export-replenishment,
standing-reserve exhaustion, and restart reconstruction have literal independent cases.
Fewer than two free slots is the pre-audit
`CapacityAdmissionRefusalV1::StandingReserveExhausted`. It appends no pre or outcome,
changes neither the reservation ledger nor the covered operation, consumes no slot, and
does not create or advance a reservation generation. Exact retry reconstructs the same
governing operation and generation from durable state; unchanged reserve state returns the
same no-write refusal, while a later durable export replenishment may admit that same
generation. The missing, overdrawn, duplicated, and unaccounted states are four distinct
closed no-write degradation classes. None is representable as an audited
`CapacityReservationOutcomeV1`; their actions are derived from the total tables below
rather than stored or accepted from a caller.

Under the one owning lock, hierarchy replay walks from the stable root. For each missing
component it calls `mkdirat` on the held parent, opens the result with the same `openat2`
policy and `O_DIRECTORY|O_CLOEXEC`, verifies the expected root ownership, mode, mount,
device, inode, and parent binding, syncs the new directory, then syncs its parent before
descending. An already present component is reopened and revalidated identically. A crash
after `mkdirat` but before either sync therefore leaves no ambiguous state: replay reopens
the partial hierarchy from the stable root, rejects any identity or posture mismatch, and
repeats the child and parent syncs. No record inode write begins until the complete final
parent hierarchy has been reopened, identity-revalidated, and synced bottom-up.

The publisher then prepares complete canonical bytes in an unnamed
`O_TMPFILE|O_RDWR|O_CLOEXEC` inode opened from the held final-parent dirfd, writes,
file-syncs, and revalidates that inode, procfs-fd links it fd-relative directly to the final
no-replace single-component name, final-reopens it fd-relative with
`O_RDONLY|O_NOFOLLOW|O_CLOEXEC`, verifies the same inode and exact bytes, then syncs the
final parent and every changed ancestor through held dirfds. No joined path, named
temporary, replacement, truncation, rename, or unlink is a publication step.

Restart classification is identical for every record class. A crash during hierarchy
creation replays the reopen/revalidate/sync procedure above. A crash before file sync or
after file sync but before link leaves no authoritative name and replay starts with a fresh
unnamed inode. A crash after link but before final reopen, parent sync, or ancestor sync
accepts only absence or the exact complete final: absence restarts publication and an exact
final is reopened, identity-revalidated, and completes the remaining directory syncs. A
nonidentical, wrong-inode, wrong-binding, wrong-predecessor, parent-replaced, symlink,
magic-link, or cross-mount final is a typed conflict and is preserved without replacement.
After all directory syncs, exact-final replay advances to the next ordered record or returns
the previously committed response with zero write. Response loss after a complete terminal
record is therefore completed no-write replay, not a second append.

Independently authored fault tests exercise after-hierarchy-`mkdirat`/before-sync,
after-hierarchy-sync/before-inode-write, after-write/before-file-sync,
after-file-sync/before-link, after-link/before-final-reopen,
after-final-reopen/before-parent-sync, after-parent-sync/before-ancestor-sync,
after-final-directory-sync, and response-loss for each dispatch, repair-audit, backup,
restoration-private-evidence, restoration-pre, restoration-provenance,
restoration-outcome, prune-pre, and prune-outcome class; a test for one class cannot satisfy
another. Parent replacement, symlink ancestor, procfs magic-link substitution, cross-device
mount, final-reopen identity change, and child-exec descriptor inheritance are independent
negative axes for both publication and pruning. Every `mkdirat`, open, write, `fsync`,
link, reopen, `unlinkat`, and directory-sync failure has an injected case; a separately
pinned record-class/boundary visitor and shrinkage poisons make a skipped class or boundary
fail before any case can count.

Validation order is canonical/schema/framing and enclosing-digest reconstruction, then
disposition/key selection and all four issuer signatures, then transition and member
semantics, then exact-C/Q aggregate acceptance. A proof carrying copied matching authority
and verification-key digests but signed by any unpinned key never produces
`AuthenticatedSourceFloorIssuerProvenance`, even when all outer objects are recomputed and
legitimately re-signed. A serialized receipt chain, decoded DTO, copied digest tuple, or the
intermediate issuer result is not a dispatch capability. Every later source-side handoff
boundary borrows the one validated floor and may create only a lifetime-bound,
operation-attenuated `SourceFloorBoundaryPermit<'floor>` that cannot outlive or be detached
from that borrow. No later boundary reparses or revalidates serialized floor evidence, and
no permit can mint a second validated floor.

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
typed result and the same claimed owner atomically commits origin consumption with the
durable dispatch publication. Every later source-side handoff boundary borrows and
attenuates that exact result until publication consumes it; restart continues only from the
durable dispatch. It never revalidates the serialized aggregate or mints another capability.

The member-census rejection list is closed and always means all seven classes: `missing`,
`duplicate`, `extra`, `empty`, `stale-generation`, `stale-digest`, and
`cross-disposition`. Each refuses before accepted-socket fd transfer, authorization, or
mutation. Unknown versions, kinds, fields, authority bindings, transition order, or malformed
encodings are structural refusals and never weaken or replace that seven-class census.
The poison generator iterates all 13 canonical role/artifact pairs for all seven classes.
The exact case id is `source-floor/<class>/<role>`, and an independent expected-set fixture
enumerates the 91 ids from the literal seven-class list and the separate exact
`role-artifact-matrix.tsv` rather than calling the poison generator or production registry.
The expected-id file, role/artifact matrix, poison generator, and production registry are
mutually read-independent. Every case keeps both the member array and declared
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

The 13-role floor is independently pinned four ways: a literal 13-row test constant, the
checked-in `role-artifact-matrix.tsv`, the production registry, and the poison visitor. None
is generated from or imported by another. The production validator must report exact ordered
equality with the literal constant before any positive or poison case can count. The exact
91-id fixture is compared with the Cartesian product of a separately literal seven-class
test axis and that literal 13-row constant, never with a production count. In particular all
39 `missing`, `stale-digest`, and `cross-disposition` role cases preserve array and declared
cardinality 13, recompute and re-sign every enclosing manifest, installation, validation,
import, and aggregate receipt, and reach only their named semantic refusal.
`tests/golden/delivery/source-floor-v1/matrix-meta-negative-case-ids.txt` contains exactly
four newline-terminated ids in this order:
`source-floor-meta/production-role-removed`,
`source-floor-meta/fixture-role-removed`,
`source-floor-meta/visitor-hook-removed`, and
`source-floor-meta/enclosing-receipt-not-recomputed`. Each poison must make the enforcing
fixture-contract runner fail for its named reason; a shrunken shared count, early structural
failure, or unvisited poison cannot pass.

The host-generation apply verifier has three additional independent registries. The exact 15
ordered mutation ids are pinned one per newline in
`tests/golden/delivery/host-generation-mutation-edge-ids.txt`. The exact 90 case ids remain
separately pinned in `host-generation-apply-peer-case-ids.txt`; neither file is generated
from or readable by the production transition registry. Tests compare production order with
both a separately authored literal 15-id test constant and the 15-id fixture, independently
form the six pre-first and 84 post-first expectations from literal test axes, and compare
those with the 90-id fixture. No expected count reads `mutation_edge_count`, registry
length, discovered hooks, or another runtime count. Thus production and expectation cannot
omit the same edge by sharing enumeration.

The mutation-edge fixture contains exactly:

```text
host-generation.source-bootstrap-publish
host-generation.target-profile-publish
host-generation.target-broker-service-transition
host-generation.coordinator-transfer-to-target
host-generation.target-daemon-service-transition
host-generation.target-pointer-publish
host-generation.target-reference-publish
host-generation.target-pointer-repair
host-generation.target-reference-repair
host-generation.rollback-target-daemon-service
host-generation.rollback-pointer-restore
host-generation.rollback-reference-restore
host-generation.rollback-profile-publish
host-generation.rollback-source-broker-service
host-generation.rollback-source-daemon-service
```

The apply-peer fixture is also literal, not a runtime Cartesian product. It contains exactly:

```text
apply-peer/pre-first/peer-exit
apply-peer/pre-first/peer-exec
apply-peer/pre-first/peer-pid-reuse
apply-peer/pre-first/peer-start-identity-mismatch
apply-peer/pre-first/peer-executable-identity-mismatch
apply-peer/pre-first/peer-identity-ambiguity
apply-peer/post-first/host-generation.target-profile-publish/peer-exit
apply-peer/post-first/host-generation.target-profile-publish/peer-exec
apply-peer/post-first/host-generation.target-profile-publish/peer-pid-reuse
apply-peer/post-first/host-generation.target-profile-publish/peer-start-identity-mismatch
apply-peer/post-first/host-generation.target-profile-publish/peer-executable-identity-mismatch
apply-peer/post-first/host-generation.target-profile-publish/peer-identity-ambiguity
apply-peer/post-first/host-generation.target-broker-service-transition/peer-exit
apply-peer/post-first/host-generation.target-broker-service-transition/peer-exec
apply-peer/post-first/host-generation.target-broker-service-transition/peer-pid-reuse
apply-peer/post-first/host-generation.target-broker-service-transition/peer-start-identity-mismatch
apply-peer/post-first/host-generation.target-broker-service-transition/peer-executable-identity-mismatch
apply-peer/post-first/host-generation.target-broker-service-transition/peer-identity-ambiguity
apply-peer/post-first/host-generation.coordinator-transfer-to-target/peer-exit
apply-peer/post-first/host-generation.coordinator-transfer-to-target/peer-exec
apply-peer/post-first/host-generation.coordinator-transfer-to-target/peer-pid-reuse
apply-peer/post-first/host-generation.coordinator-transfer-to-target/peer-start-identity-mismatch
apply-peer/post-first/host-generation.coordinator-transfer-to-target/peer-executable-identity-mismatch
apply-peer/post-first/host-generation.coordinator-transfer-to-target/peer-identity-ambiguity
apply-peer/post-first/host-generation.target-daemon-service-transition/peer-exit
apply-peer/post-first/host-generation.target-daemon-service-transition/peer-exec
apply-peer/post-first/host-generation.target-daemon-service-transition/peer-pid-reuse
apply-peer/post-first/host-generation.target-daemon-service-transition/peer-start-identity-mismatch
apply-peer/post-first/host-generation.target-daemon-service-transition/peer-executable-identity-mismatch
apply-peer/post-first/host-generation.target-daemon-service-transition/peer-identity-ambiguity
apply-peer/post-first/host-generation.target-pointer-publish/peer-exit
apply-peer/post-first/host-generation.target-pointer-publish/peer-exec
apply-peer/post-first/host-generation.target-pointer-publish/peer-pid-reuse
apply-peer/post-first/host-generation.target-pointer-publish/peer-start-identity-mismatch
apply-peer/post-first/host-generation.target-pointer-publish/peer-executable-identity-mismatch
apply-peer/post-first/host-generation.target-pointer-publish/peer-identity-ambiguity
apply-peer/post-first/host-generation.target-reference-publish/peer-exit
apply-peer/post-first/host-generation.target-reference-publish/peer-exec
apply-peer/post-first/host-generation.target-reference-publish/peer-pid-reuse
apply-peer/post-first/host-generation.target-reference-publish/peer-start-identity-mismatch
apply-peer/post-first/host-generation.target-reference-publish/peer-executable-identity-mismatch
apply-peer/post-first/host-generation.target-reference-publish/peer-identity-ambiguity
apply-peer/post-first/host-generation.target-pointer-repair/peer-exit
apply-peer/post-first/host-generation.target-pointer-repair/peer-exec
apply-peer/post-first/host-generation.target-pointer-repair/peer-pid-reuse
apply-peer/post-first/host-generation.target-pointer-repair/peer-start-identity-mismatch
apply-peer/post-first/host-generation.target-pointer-repair/peer-executable-identity-mismatch
apply-peer/post-first/host-generation.target-pointer-repair/peer-identity-ambiguity
apply-peer/post-first/host-generation.target-reference-repair/peer-exit
apply-peer/post-first/host-generation.target-reference-repair/peer-exec
apply-peer/post-first/host-generation.target-reference-repair/peer-pid-reuse
apply-peer/post-first/host-generation.target-reference-repair/peer-start-identity-mismatch
apply-peer/post-first/host-generation.target-reference-repair/peer-executable-identity-mismatch
apply-peer/post-first/host-generation.target-reference-repair/peer-identity-ambiguity
apply-peer/post-first/host-generation.rollback-target-daemon-service/peer-exit
apply-peer/post-first/host-generation.rollback-target-daemon-service/peer-exec
apply-peer/post-first/host-generation.rollback-target-daemon-service/peer-pid-reuse
apply-peer/post-first/host-generation.rollback-target-daemon-service/peer-start-identity-mismatch
apply-peer/post-first/host-generation.rollback-target-daemon-service/peer-executable-identity-mismatch
apply-peer/post-first/host-generation.rollback-target-daemon-service/peer-identity-ambiguity
apply-peer/post-first/host-generation.rollback-pointer-restore/peer-exit
apply-peer/post-first/host-generation.rollback-pointer-restore/peer-exec
apply-peer/post-first/host-generation.rollback-pointer-restore/peer-pid-reuse
apply-peer/post-first/host-generation.rollback-pointer-restore/peer-start-identity-mismatch
apply-peer/post-first/host-generation.rollback-pointer-restore/peer-executable-identity-mismatch
apply-peer/post-first/host-generation.rollback-pointer-restore/peer-identity-ambiguity
apply-peer/post-first/host-generation.rollback-reference-restore/peer-exit
apply-peer/post-first/host-generation.rollback-reference-restore/peer-exec
apply-peer/post-first/host-generation.rollback-reference-restore/peer-pid-reuse
apply-peer/post-first/host-generation.rollback-reference-restore/peer-start-identity-mismatch
apply-peer/post-first/host-generation.rollback-reference-restore/peer-executable-identity-mismatch
apply-peer/post-first/host-generation.rollback-reference-restore/peer-identity-ambiguity
apply-peer/post-first/host-generation.rollback-profile-publish/peer-exit
apply-peer/post-first/host-generation.rollback-profile-publish/peer-exec
apply-peer/post-first/host-generation.rollback-profile-publish/peer-pid-reuse
apply-peer/post-first/host-generation.rollback-profile-publish/peer-start-identity-mismatch
apply-peer/post-first/host-generation.rollback-profile-publish/peer-executable-identity-mismatch
apply-peer/post-first/host-generation.rollback-profile-publish/peer-identity-ambiguity
apply-peer/post-first/host-generation.rollback-source-broker-service/peer-exit
apply-peer/post-first/host-generation.rollback-source-broker-service/peer-exec
apply-peer/post-first/host-generation.rollback-source-broker-service/peer-pid-reuse
apply-peer/post-first/host-generation.rollback-source-broker-service/peer-start-identity-mismatch
apply-peer/post-first/host-generation.rollback-source-broker-service/peer-executable-identity-mismatch
apply-peer/post-first/host-generation.rollback-source-broker-service/peer-identity-ambiguity
apply-peer/post-first/host-generation.rollback-source-daemon-service/peer-exit
apply-peer/post-first/host-generation.rollback-source-daemon-service/peer-exec
apply-peer/post-first/host-generation.rollback-source-daemon-service/peer-pid-reuse
apply-peer/post-first/host-generation.rollback-source-daemon-service/peer-start-identity-mismatch
apply-peer/post-first/host-generation.rollback-source-daemon-service/peer-executable-identity-mismatch
apply-peer/post-first/host-generation.rollback-source-daemon-service/peer-identity-ambiguity
```

The literal 90-id test constant repeats this order directly and may not call a product,
generator, or shared matrix helper.
`tests/golden/delivery/host-generation-mutation-edge-meta-negative-case-ids.txt` contains
exactly three newline-terminated ids in order:
`mutation-edge-meta/production-edge-removed`,
`mutation-edge-meta/expected-edge-removed`, and
`mutation-edge-meta/verification-hook-removed`. The first poisons production catalogue
equality, the second poisons literal/fixture equality, and the third preserves both 15-edge
lists while removing one pre-mutation verification visit. Each must fail through the
enforcing runner before evidence acceptance.

`tests/golden/delivery/host-generation-post-first-negative-case-ids.txt` contains exactly
these 15 newline-terminated ids in order:

```text
post-first-negative/missing-edge
post-first-negative/duplicate-edge
post-first-negative/unknown-edge
post-first-negative/reordered-edge
post-first-negative/empty-edge-set
post-first-negative/missing-transition
post-first-negative/duplicate-transition
post-first-negative/unknown-transition
post-first-negative/unvisited-case
post-first-negative/dynamic-case-skipped
post-first-negative/verification-hook-missing
post-first-negative/selected-edge-mutated
post-first-negative/successor-mutated
post-first-negative/durable-prefix-changed
post-first-negative/first-audit-missing
```

A separately authored literal 15-id constant must equal the file. Each poison preserves
every prerequisite needed to reach only its named edge/transition/visit/hook/mutation/audit
check. Empty production and expected sets, a dynamically skipped case, an early structural
failure, or a count learned from production is a failure, not a visit. This file, the
15-edge file, the 90-case file, the three-case meta registry, production enumeration, and
all literal expectations are mutually read-independent.

`tests/golden/delivery/host-generation-pre-start-case-ids.txt` closes the pre-mutation and
root-invocation surface that the edge matrix does not cover. It contains exactly these 15
newline-terminated ids in order:

```text
pre-start/unprivileged-authorize-positive
pre-start/root-authorize-bootstrap-refused
pre-start/root-authorize-stable-reference-refused
pre-start/root-authorize-rollback-refused
pre-start/apply-without-authorization
pre-start/apply-before-source-daemon-ready
pre-start/apply-before-source-broker-ready
pre-start/apply-before-source-hello
pre-start/apply-before-catalogue-match
pre-start/apply-before-capability-seal
pre-start/apply-before-target-object-pin
pre-start/apply-before-installed-apply-pin
pre-start/apply-before-gc-root
pre-start/apply-before-coordinator-durable
pre-start/apply-before-existing-broker-unit-active
```

A separately authored literal constant equals this file. Each refusal has zero mutation
edges, zero pre-mutation audit rows claiming an attempted mutation, no fd transfer before
the named prerequisite, and the closed remediation for that exact prerequisite. The three
root cases execute the documented bootstrap, stable-reference, and rollback authorization
forms as uid 0 and must fail before public-socket authorization or `sudo`. A skipped,
unvisited, reordered, duplicate, production-derived, or early unrelated refusal fails the
matrix.

The independently authored
`tests/golden/delivery/host-generation-unit-census-case-ids.txt` contains exactly these 27
newline-terminated ids in order:

```text
unit-census/positive
unit-census/enumeration-error
unit-census/empty
unit-census/missing-d2bd-service
unit-census/missing-broker-socket
unit-census/missing-broker-service
unit-census/unexpected-d2b-service
unit-census/unexpected-d2b-socket
unit-census/unexpected-d2b-slice
unit-census/unexpected-d2b-path
unit-census/unexpected-d2b-timer
unit-census/unexpected-microvm-template
unit-census/unexpected-microvm-instance
unit-census/malformed-row
unit-census/skip-marker
unit-census/pre-start-positive
unit-census/post-start-positive
unit-census/post-restart-positive
unit-census/post-stop-positive
unit-census/unexpected-d2b-target
unit-census/unexpected-d2b-template
unit-census/unexpected-d2b-instance
unit-census/unexpected-microvm-socket
unit-census/unexpected-microvm-slice
unit-census/unexpected-microvm-target
unit-census/unexpected-microvm-path
unit-census/unexpected-microvm-timer
```

`positive` remains the candidate-bound acceptance census. The four phase-specific positives
repeat exact full-namespace equality before VM start, after public start, after daemon
restart/adoption, and after public stop. Each added poison loads exactly one named unit:
`d2b-unexpected.target`, `d2b@.service`, `d2b@unexpected.service`,
`microvm-unexpected.socket`, `microvm-unexpected.slice`,
`microvm-unexpected.target`, `microvm-unexpected.path`, or
`microvm-unexpected.timer`. Together with the retained service/socket/slice/path/timer and
microvm template/instance cases, every allowed systemd unit kind and both governed prefixes
are exercised. A separate literal 27-id constant equals this fixture; the production filter,
expected-three-unit array, fixture, and literal are mutually read-independent.

`tests/golden/delivery/host-generation-apply-peer-forbidden-values.tsv` is the closed literal
observability canary registry. It covers every raw input read by apply-peer admission and
identity verification. It has exactly these fifteen tab-separated, newline-terminated
rows and no header:

```text
peer-pidfd-number	9090
peer-pid	424242
peer-start-identity	998877665544
peer-socket-uid	61616
peer-socket-gid	62626
peer-cgroup-path	/sys/fs/cgroup/d2b-apply-peer-canary.scope
peer-proc-path	/proc/515151/exe
executable-store-path	/nix/store/00000000000000000000000000000000-d2b-apply-peer-canary/bin/d2b-host-generation-deploy
executable-derivation	/nix/store/11111111111111111111111111111111-d2b-apply-peer-canary.drv
executable-nar-identity	d2b-nar-identity-canary-v1
executable-nar-sha256	2222222222222222222222222222222222222222222222222222222222222222
executable-content-sha256	3333333333333333333333333333333333333333333333333333333333333333
executable-device	737373
executable-inode	747474
executable-mount-id	757575
```

The verifier injects each literal independently before the first mutation and at every
post-first case, then scans coordinator state, receipt/evidence bytes, human and JSON output,
wire payloads, error and `Display`, logs, tracing event fields and span attributes, metric
name/help/label key/label value/exemplar, audit, panic, and `Debug`. The injection fixture
and the test's private expected-value buffer are the only scan exclusions. No broad file,
directory, prefix, or process exclusion is allowed. Every raw literal must be absent.
Only the PID/start pair may produce `ApplyPeerProcessInstanceDigestV1`, and only the
NAR/content-digest pair may produce `ApplyPeerExecutableIdentityDigestV1`; pidfd number,
socket uid/gid, cgroup path, proc path, store path, derivation, NAR name, device, inode, and
mount id have no allowed digest projection. Where one of the two correlation classes is
required, its independently computed fixed digest must be present; metrics must contain
neither raw values nor peer-identity digests or labels. A
missing canary visit, duplicate registry row, unknown class, changed literal, captured-surface
omission, or production read of this fixture fails the matrix.

Where non-metric audit correlation is required, the only allowed encodings are closed typed
digests over fixed binary inputs:

```text
ApplyPeerProcessInstanceDigestV1 =
  SHA-256(
    "d2b:apply-peer:process-instance:v1\0" ||
    u64be(peer-pid) || u64be(peer-start-identity)
  )

ApplyPeerExecutableIdentityDigestV1 =
  SHA-256(
    "d2b:apply-peer:executable-identity:v1\0" ||
    executable-nar-sha256[32] || executable-content-sha256[32]
  )
```

The two input digests are decoded from canonical lowercase 64-hex before hashing. No raw
path, derivation, NAR name, pidfd number, socket credential, cgroup, proc path,
device/inode/mount identity, serializer output, native-width integer, or caller-selected tag
enters either preimage. Tests reconstruct both digests independently
and reject wrong domain, terminator, width, endian, field order, hex decoding, or cross-class
substitution. Metrics contain neither raw values nor either digest.

`HostGenerationHandoffStatusV1` is a closed validated enum, not a freely constructible
seven-field struct. It projects the current intent, whether active or terminal, in the
current source-generation coordinator. The unprivileged
`d2b-host-generation-deploy --inspect-authorized-handoff [--json]` entrypoint reaches it
through the existing public socket; it accepts no intent id, generation selector, path,
authority token, or root invocation. JSON contains exactly, in order,
`schemaVersion = 1`, `kind = "host-generation-handoff-status"`, `state`, `phase`, `owner`,
`action`, and `successorStates`. `phase` is null or one exact closed mutation-edge id.
`owner` is only `none`, `apply-peer-live`, `source-broker-recovery`, or
`target-broker-recovery`. It never contains a pid, uid, unit path, generation, store path,
intent id, digest, or executable identity. `successorStates` is a sorted unique array from
the same closed state set, not free-form guidance. The internal variant, durable state,
phase nullability, owner, action, and exact successor array must match one row below; no
cross-product tuple validates or serializes.

| Validated variant | `state` | `phase` | `owner` | `action` | Exact sorted `successorStates` |
| --- | --- | --- | --- | --- | --- |
| `AuthorizedPending` | `authorized-pending` | null | `none` | `run-authorized-apply` | `["apply-claimed"]` |
| `ApplyClaimedPeerLive` | `apply-claimed` | null | `apply-peer-live` | `wait-for-live-apply` | `["authorized-pending","mutating"]` |
| `MutatingPeerLive` | `mutating` | exact current edge | `apply-peer-live` | `wait-for-live-apply` | `["completed","recovery-pending","transfer-pending"]` |
| `RecoveryPendingSourceActive` | `recovery-pending` | exact interrupted edge | `source-broker-recovery` | `wait-for-broker-recovery` | `["completed","mutating","recovery-irreconcilable","rolled-back","transfer-pending"]` |
| `RecoveryPendingSourceFailed` | `recovery-pending` | exact interrupted edge | `source-broker-recovery` | `restart-existing-broker` | `["completed","mutating","recovery-irreconcilable","rolled-back","transfer-pending"]` |
| `RecoveryPendingTargetActive` | `recovery-pending` | exact interrupted edge | `target-broker-recovery` | `wait-for-broker-recovery` | `["completed","mutating","recovery-irreconcilable","rolled-back","transfer-pending"]` |
| `RecoveryPendingTargetFailed` | `recovery-pending` | exact interrupted edge | `target-broker-recovery` | `restart-existing-broker` | `["completed","mutating","recovery-irreconcilable","rolled-back","transfer-pending"]` |
| `TransferPendingSourceActive` | `transfer-pending` | `host-generation.coordinator-transfer-to-target` | `source-broker-recovery` | `wait-for-broker-recovery` | `["completed","recovery-irreconcilable","recovery-pending","rolled-back"]` |
| `TransferPendingSourceFailed` | `transfer-pending` | `host-generation.coordinator-transfer-to-target` | `source-broker-recovery` | `restart-existing-broker` | `["completed","recovery-irreconcilable","recovery-pending","rolled-back"]` |
| `RollbackSourceActive` | `recovery-irreconcilable` | exact failed edge | `source-broker-recovery` | `wait-for-broker-rollback` | `["rolled-back"]` |
| `RollbackSourceFailed` | `recovery-irreconcilable` | exact failed edge | `source-broker-recovery` | `restart-existing-broker-for-rollback` | `["rolled-back"]` |
| `RollbackTargetActive` | `recovery-irreconcilable` | exact failed edge | `target-broker-recovery` | `wait-for-broker-rollback` | `["rolled-back"]` |
| `RollbackTargetFailed` | `recovery-irreconcilable` | exact failed edge | `target-broker-recovery` | `restart-existing-broker-for-rollback` | `["rolled-back"]` |
| `CompletedTerminal` | `completed` | null | `none` | `none` | `[]` |
| `RolledBackTerminal` | `rolled-back` | null | `none` | `rebuild-host-generation` | `["authorized-pending"]` |

The source coordinator durably maintains one authenticated `current-intent` pointer and a
strictly increasing internal sequence. Inspection selects that pointer, never a timestamp,
directory order, caller selector, or "latest" heuristic. A sole nonterminal record that does
not equal the pointer, multiple nonterminal records, a pointer to a missing record, or a
nonterminal record after a higher terminal sequence is invalid coordinator state. When the
pointer names `completed` or `rolled-back`, that exact terminal remains inspectable until a
new authorization atomically installs the next `authorized-pending` pointer. This is the
only cross-intent transition represented by the rolled-back row.

A state may be classified `recovery-irreconcilable` only when immutable coordinator and
audit history prove the exact prior profile, source/target broker and daemon service states,
pointer bytes or authenticated absence, reference bytes or authenticated absence, every
pre-mutation row for the applied prefix, every matching outcome row, and one contiguous
reverse rollback plan ending at that tuple. Removing, duplicating, reordering, or changing
any member; omitting an outcome; adding an unaudited mutation; or proving only some restored
members returns `invalid-coordinator`, performs no mutation, and never serializes a rollback
variant. Active rollback uses `wait-for-broker-rollback`; a failed existing broker unit uses
`restart-existing-broker-for-rollback`. A transfer-pending broker failure is likewise its
own validated restart row and cannot retain the active wait action.

Human inspection is exactly five newline-terminated lines:

```text
state: <STATE>
phase: <PHASE_OR_NONE>
owner: <OWNER>
action: <ACTION>
successor-states: <SORTED_COMMA_LIST_OR_NONE>
```

The bracketed values are closed bounded enums. No line carries an identifier, path, command
argument, or free-form sentence. The apply command and broker recovery return the same
projection on a valid refusal or concurrent transition through the same typed renderer; no
second apply/recovery serializer exists.

The exit and error contract is exact. Inspecting any valid active or terminal row exits `0`.
Invalid syntax, any forbidden selector/path/token option, or root invocation exits `2`;
human output is exactly `host generation handoff inspection refused\n` followed by
`action: inspect-without-selectors\n`, and JSON is exactly
`{"schemaVersion":1,"kind":"host-generation-handoff-error","error":"invalid-invocation","action":"inspect-without-selectors"}`.
An absent current pointer whose locked coordinator census is exactly empty is
`clean-absence` and exits `3`; human output is exactly
`host generation handoff not found\n` followed by
`action: begin-host-generation-deploy\n`, and JSON is exactly
`{"schemaVersion":1,"kind":"host-generation-handoff-error","error":"not-found","action":"begin-host-generation-deploy"}`.
An absent pointer with exactly one fully valid authenticated active or terminal intent and
its complete immutable transition matrix is `repairable-absence` and exits `4`; human output
is exactly
`host generation handoff pointer repair required\naction: repair-authorized-handoff\n`,
and JSON is exactly
`{"schemaVersion":1,"kind":"host-generation-handoff-error","error":"pointer-repair-required","action":"repair-authorized-handoff"}`.
Invalid coordinator or incomplete rollback proof exits `4` with zero mutation; human output
is exactly `host generation handoff coordinator invalid\n` followed by
`action: preserve-and-escalate-invalid-coordinator\n`, and JSON is exactly
`{"schemaVersion":1,"kind":"host-generation-handoff-error","error":"invalid-coordinator","action":"preserve-and-escalate-invalid-coordinator"}`.
Apply or broker recovery encountering a valid concurrent or terminal state exits `4` and
renders that same valid five-line or seven-field status rather than an error envelope.

The repair action maps to the exact selector-free unprivileged command
`d2b-host-generation-deploy --repair-authorized-handoff`. T595 owns its CLI/parser and
public-socket client. T592 owns the typed broker operation
`RepairHostGenerationCurrentIntentV1`, coordinator mutation, privilege row, and audit edge.
The CLI traverses the existing public socket and consumes only its accepted-socket `Admin`
capability. `Launcher`, workload, Zone, unauthenticated, direct broker-socket, and root
invocations are denied before coordinator access. The broker acquires the one coordinator
lock, authenticates the current pointer or its absence and every immutable matrix member,
and may repair only a uniquely reconstructible authenticated `current-intent` pointer. It
cannot rewrite an intent, audit member, service observation, profile, reference, or mutation
outcome.

Pointer absence has a closed three-way classifier. `clean-absence` means the locked
coordinator census contains zero intent, audit, transition, pointer, or restoration members;
inspect and repair both exit `3` with the existing exact `not-found` envelope and perform no
write. `repairable-absence` means the pointer alone is absent and exactly one fully valid
authenticated active or terminal intent, one contiguous sequence, one complete immutable
transition matrix, and zero competing intent reconstruct the final pointer bytes. Every
other census is `invalid-coordinator`, including two otherwise valid competing intents, a
malformed intent, an unauthenticated intent, an orphan audit/transition/restoration member,
an incomplete matrix, or any unknown entry. None of those invalid censuses is clean or
repairable. Independent inspect and repair tests cover the empty, exactly-one-valid,
competing-valid, malformed, unauthenticated, orphan-member, and incomplete-matrix cases.

Repair publishes only from `repairable-absence`. Under the lock, the broker first appends
and durably syncs the immutable
`coordinator-pointer-repair/pre-mutation` audit member. It then writes the exact pointer
bytes to an unnamed `O_TMPFILE|O_RDWR|O_CLOEXEC` inode in the final pointer directory,
file-syncs and revalidates it, procfs-fd links that exact inode directly to the final
no-replace name, final-reopens and matches it, and syncs the final parent. There is no named
temporary or name-consuming rename. It then appends and durably syncs the matching
`coordinator-pointer-repair/outcome` member before returning the normal five-line status.
The pre/outcome records contain only the fixed edge id, a domain-separated attempt digest,
the selected intent digest, the immutable-matrix digest, the closed prior-pointer state,
and the closed outcome; no path, intent bytes, generation, uid, pid, errno, or free-form
value appears. Prior-pointer state is the literal `absent`; outcome is exactly
`published | pointer-conflict`. The pre row omits outcome and the outcome row
repeats every digest and prior-state field before appending its one outcome. Fixtures cover
published success, completed no-write replay from that existing pair, and conflict failure;
missing, duplicate,
reordered, mismatched, or unknown repair audit members refuse.

```text
HostGenerationCoordinatorRepairAttemptDigestV1 =
  SHA-256(
    "d2b:host-generation:coordinator-pointer-repair-attempt:v1\0" ||
    selected_intent_digest[32] || immutable_matrix_digest[32]
  )
```

Both inputs are decoded from canonical lowercase 64-hex typed digests before hashing. No
serializer bytes, display member, native-width integer, or caller value enters the preimage.

Restart classification is exact. Before the pre-mutation audit is durable, no repair
started. After that audit but before the direct link, the final is absent and replay creates
a fresh unnamed inode. After the link but before parent sync, restart accepts only final
absence or the exact complete inode; it recreates or reopens and syncs accordingly. After
parent sync but before outcome audit, the exact final is retained and replay appends only
the matching outcome. A nonidentical final is `pointer-repair-conflict`, preserves it, exits
`4`, and appends the failure outcome without pointer mutation. After the complete audit pair,
a second run final-reopens the exact pointer and returns the normal status with zero pointer,
audit, or coordinator write. A crash after the outcome and before response has the same
no-write replay. An already exact authenticated pointer created by the ordinary handoff path
likewise returns normal status with zero write after its existing publication audit
provenance validates; it does not fabricate a coordinator-repair pair. No classifier uses
process memory.

`pointer-repair-conflict` human output is exactly
`host generation handoff pointer repair conflict\naction: preserve-and-escalate-pointer-conflict\n`.
Its JSON is exactly
`{"schemaVersion":1,"kind":"host-generation-handoff-error","error":"pointer-repair-conflict","action":"preserve-and-escalate-pointer-conflict"}`.
It never suggests retry, force, replacement, or deletion.

Repair accepts only optional `--json`. An intent or generation selector, path, authority
token, root invocation, extra positional argument, or `--force` exits `2`, performs zero
public-socket or coordinator mutation, and emits exactly
`host generation handoff repair refused\naction: repair-without-selectors\n` in human mode
or
`{"schemaVersion":1,"kind":"host-generation-handoff-error","error":"invalid-invocation","action":"repair-without-selectors"}`
in JSON.

If one immutable member is missing, mismatched, unauthenticated, or noncontiguous, repair
exits `4` with zero pointer mutation and identifies only the bounded closed member id and
failure class. Human output is exactly:

```text
host generation handoff immutable audit restoration required
member: <CLOSED_MEMBER>
failure-class: <MISSING_OR_MISMATCH_OR_UNAUTHENTICATED_OR_NONCONTIGUOUS>
action: restore-immutable-audit-backup
```

JSON contains exactly, in order, `schemaVersion = 1`,
`kind = "host-generation-handoff-error"`,
`error = "audit-restoration-required"`, `member`, `failureClass`, and
`action = "restore-immutable-audit-backup"`. `member` is the first failing member in the
independently pinned canonical rollback/audit/transition order, never a caller-selected or
free-form value; one rerun may expose the next failure only after authenticated restoration
of the displayed member. An unaudited extra mutation is not restoration
eligible. It exits `4` with zero mutation and human output exactly
`host generation handoff audit integrity incident\nfailure-class: unaudited-extra-mutation\naction: preserve-and-escalate-audit-integrity-incident\n`;
JSON contains exactly `schemaVersion`, `kind`, `error = "audit-integrity-incident"`,
`failureClass = "unaudited-extra-mutation"`, and
`action = "preserve-and-escalate-audit-integrity-incident"` in that order.

The binding backup owner is the broker-private
`HostGenerationImmutableAuditBackupOwner`, transferred with coordinator ownership and never
held by the daemon. It is a private linear capability whose fields and constructors are
private. It implements no `Clone`, `Copy`, `Default`, serde/serialization, `From`,
`TryFrom`, conversion, byte importer, digest reconstruction, or independent mint surface.
Coordinator transfer consumes the old wrapper by value and moves the same owner into the
new wrapper; using the old wrapper after transfer is a compile error. Compile-fail and
API-surface tests reject construction, field access, clone/copy/default, serialization,
conversion, reconstruction, duplication during transfer, and reuse after transfer.

Before any covered mutation becomes durable, that owner append-only publishes one
authenticated `HostGenerationImmutableAuditBackupV1` member containing exactly, in order,
`schemaVersion = 1`, `kind = "host-generation-immutable-audit-backup"`, `member`,
`canonicalMemberSha256`, `canonicalMemberBytes`, `predecessorSha256`, `authoritySha256`,
`verificationKeySha256`, and `signatureEd25519`. `canonicalMemberBytes` is at most 65,536
bytes and the complete canonical backup envelope is at most 131,072 bytes. The disposition
pins the backup authority and verification key. The broker refuses the covered mutation
unless the exact backup final and its parent/ancestor durability have completed under
`HostGenerationImmutablePublicationV1`.

One intent's retained backup set admits at most 256 members and at most 16,777,216 encoded
bytes. Across the stable publication root, all current and replaced retained sets together
admit at most 64 intents, 4,096 members, and 268,435,456 encoded bytes. The exact accepted
and refused boundaries are therefore 256/257 members and 16,777,216/16,777,217 bytes per
intent, plus 64/65 intents, 4,096/4,097 members, and
268,435,456/268,435,457 bytes at the root. Before a handoff can append its first backup, the broker invokes typed
`ReserveHostGenerationImmutableAuditCapacityV1` under the coordinator lock and durably
reserves the exact prospective set's intent/member/encoded-byte counts plus the aggregate
record/byte charge for every private publication class. The sealed broker first binds the
reservation to one unique durable `GoverningCapacityOperationIdV1` for the covered
mutation. That id is never caller supplied and two independent operations remain distinct
even when their intent and charge bytes are equal.

Each governing operation has a durable `reservationGeneration:u64`. Generation zero is its
first cycle. A later cycle is permitted only after an exact completed zero-mutation refusal
or a matching completed release, and is bound to that predecessor outcome and the current
ledger digest. Exact replay keeps the same generation; only the coordinator's typed
retry-after-changed-capacity transition advances it. Reservation and release identities are:

- `CapacityReservationAttemptIdV1 =
  SHA-256("d2b:host-generation:audit-capacity-reservation-attempt:v1\0" ||
  governing_capacity_operation_id || reservation_generation_u64_be ||
  prior_cycle_outcome_sha256 || prior_ledger_sha256 || intent_sha256 ||
  prospective_charge_canonical_bytes)`.
- `CapacityReleaseAttemptIdV1 =
  SHA-256("d2b:host-generation:audit-capacity-release-attempt:v1\0" ||
  reservation_attempt_id || reservation_generation_u64_be || release_reason_tag ||
  governing_proof_sha256 || prior_ledger_sha256)`.

The generation is eight-byte big-endian; every id or digest field in these formulas is
exactly 32 bytes, and `prior_cycle_outcome_sha256` is the all-zero 32-byte sentinel only for
generation zero. No serializer output or variable-length concatenation enters either
formula.

`prospective_charge_canonical_bytes` is not serializer output. It is the fixed-width
big-endian tuple `intentDelta:u16 || memberDelta:u32 || backupEncodedByteDelta:u64 ||
rootRecordDelta:u32 || rootEncodedByteDelta:u64`. `release_reason_tag` is one byte:
`0x01` for `durable-prune` and `0x02` for `immutable-zero-mutation`. Checked conversion or
addition failure refuses before pre-audit.

Audit contains only the corresponding typed
`capacityReservationAttemptSha256 =
SHA-256("d2b:audit:host-generation:capacity-reservation-attempt:v1\0" ||
private_attempt_id)` or
`capacityReleaseAttemptSha256 =
SHA-256("d2b:audit:host-generation:capacity-release-attempt:v1\0" ||
private_attempt_id)`, the reservation generation, exact bounded charge or release reason,
governing proof digest, prior-cycle outcome digest, prior-ledger digest, and closed outcome.
It never contains a raw governing operation, reservation, release, or other private attempt
id.

Reservation completion is a total nested enum:

```text
CapacityReservationOutcomeV1 =
    Reserved { appliedLedgerSha256 }
  | RefusedZeroMutation {
      class: RetentionCapacityClassV1,
      unchangedLedgerSha256
    }

CapacityReservationPrefixV1 =
    Absent
  | PreAudited
  | LedgerApplied
  | CompletedReserved
  | CompletedRefusedZeroMutation

CapacityAdmissionRefusalV1 =
    StandingReserveExhausted
```

Its only valid transitions are
`Absent -> PreAudited -> LedgerApplied -> CompletedReserved` and
`Absent -> PreAudited -> CompletedRefusedZeroMutation`. `Absent` may append one pre record
only after atomically claiming two slots from the nonrecursive standing capacity-control
reserve. `StandingReserveExhausted` is classified before `Absent` and therefore is not a
prefix or an audited reservation outcome.
`PreAudited` has no ledger mutation and replays the exact governing operation, generation,
predecessor, and charge from durable fixed fields. `LedgerApplied` exists only on the
successful branch and means the mutation is durable but its matching `Reserved` outcome is
absent. `CompletedRefusedZeroMutation` requires the unchanged prior-ledger digest and can
never be classified as `LedgerApplied`. Either completed variant replays its stored response
with zero ledger or audit write, including response loss.

The retained wire name `RefusedZeroMutation` has one narrow meaning: zero reservation-ledger
mutation and zero mutation by the covered handoff/restoration/prune operation. It still
requires exactly one durable capacity-reservation pre/outcome audit pair and consumes then
releases its two standing slots through export reconciliation. It never promises zero audit
mutation. In contrast, `StandingReserveExhausted`, malformed charge encoding, checked
conversion failure, and corrupt standing-reserve admission are pre-audit refusals with zero
ledger, covered-operation, and audit mutation. Response loss on a pre-audit refusal reruns
the read-only admission classifier at the same generation; it does not synthesize a
completed audited refusal or advance `prior_cycle_outcome_sha256`.

Each release reason has the separate total prefix enum
`Absent | PreAudited | LedgerApplied | CompletedReleased`; release is permitted only after
either the corresponding set is durably pruned or an immutable zero-mutation outcome proves
that no backup, private evidence, audit member, or covered mutation became durable. The two
release reasons remain exactly `durable-prune | immutable-zero-mutation`; they use distinct
attempt ids and cannot substitute for each other. The durable-prune machine accepts only
the exact completed prune outcome digest as `governing_proof_sha256`; the
immutable-zero-mutation machine accepts only the exact completed covered-operation
zero-mutation outcome digest. A reason-tag or proof substitution, including a valid proof
from the other release machine, is an impossible prefix rather than a release.

A ledger mutation without pre-audit, a refused completion after ledger mutation, a reserved
completion without ledger mutation, an outcome without the exact mutation, duplicate
pre/outcome, reused governing-operation/generation pair, a mismatched
charge/reason/proof/predecessor/digest, or any other impossible prefix degrades the root and
blocks all later mutation. Restart reconstructs every governing operation, generation,
reservation, release, and standing-reserve charge from the immutable root census and exact
prefixes before admission. Literal tests independently cover equal-charge operations,
successful and refused prefixes, refusal crashes, completed response loss, a typed retry
after capacity changes, and release followed by same-intent/same-charge retry. Each requires
cycle-unique ids, one ledger apply at most, one outcome at most, no capacity leak, and no
double release. The two release machines each have read-independent malformed-prefix hooks
for outcome-without-pre, ledger-without-pre, completion-without-ledger, duplicate pre,
duplicate outcome, wrong generation, wrong prior-ledger digest, wrong reason tag, wrong
governing proof, and cross-release proof substitution, with one removal poison per hook.

No backup for the current intent is prune-eligible. Immediately after authenticated pointer
replacement is durable, the broker completes the same effective replacement transition with
typed `BindHostGenerationImmutableAuditRetentionAnchorV1`. It derives a stable private
anchor-attempt id from the durable replacement audit, prior watermark digest, and exact
epoch sequence:
`RetentionAnchorAttemptIdV1 =
SHA-256("d2b:host-generation:audit-retention-anchor-attempt:v1\0" ||
replacement_audit_sha256 || prior_watermark_sha256 || epoch_sequence_u64_be)`.
Audit uses only
`retentionAnchorAttemptSha256 =
SHA-256("d2b:audit:host-generation:retention-anchor-attempt:v1\0" ||
private_anchor_attempt_id)`. It first appends fixed-field retention-anchor pre-audit
containing that typed domain-separated attempt digest,
replacement digest, intent digest, and epoch sequence, but no clock sample or candidate
digest. Only after that pre record is durable may it take one trusted clock sample and
publish the sample as the broker-private, non-authorizing anchor candidate. The transition
does not become effective until the anchor candidate, pointer replacement, matching
replacement outcome, and anchor outcome are all durable. A pre-only crash proves no sample
became durable and permits the fresh process to take the first sample. Once an exact
candidate final exists, every replay reuses it and never samples a new timestamp; a
nonidentical candidate is preserved as conflict. The private immutable
`HostGenerationImmutableAuditBackupRetentionEpochV1` fields are exactly
`schemaVersion = 1`,
`kind = "host-generation-immutable-audit-backup-retention-epoch"`,
`replacementAuditSha256`, `intentSha256`, `epochSequence`, private `bootIdSha256`,
`clock = "CLOCK_REALTIME+CLOCK_BOOTTIME"`, `epochUnixSeconds`, and
`epochBootTimeNanoseconds`. Only the outcome carries the digest of the now-durable anchor
candidate. Neither timestamp nor boot identity is permitted in pre/outcome audit. Fresh
process tests crash after pre-audit and every candidate write/file-sync/link/directory-sync
boundary, proving zero samples before pre, exactly one durable candidate, and no resampling
after any candidate final. A nonidentical candidate under the same anchor attempt is a
preserved conflict and can never be treated as a later sample or replacement epoch.

A root-owned durable private clock watermark stores the greatest accepted real-time second,
boot-time nanosecond, boot identity digest, and anchor sequence. On the same boot,
eligibility advances only by `CLOCK_BOOTTIME`; `CLOCK_REALTIME` must be nondecreasing and its
delta may differ from the boot-time delta by at most 300 seconds. A larger positive
difference is `clock-forward-discontinuity`, not elapsed retention age. A changed boot
identity has no crash-stable monotonic continuity and is
`clock-continuity-ambiguous` until the broker validates authoritative non-caller continuity
evidence and consumes the sealed continuity-repair permit. The Admin request cannot publish
an anchor. Neither class can make a member eligible. Every eligibility
calculation uses checked unsigned addition for 2,592,000 seconds (30 days) and 7,776,000
seconds (90 days). Backward time, forward discontinuity, changed-boot ambiguity, overflow,
an invalid anchor, or an unpersistable watermark is typed degradation and quarantines age.
At 30 trusted elapsed days a member becomes prune-eligible; at 90 trusted elapsed days it is
mandatory to prune. A long same-boot process crash or suspend uses the bound boot-time delta;
a real-time jump with little boot-time advance and every changed-boot restart are explicit
fail-closed tests.

Continuity repair accepts no caller timestamp, boot identity, anchor, deadline, proof,
digest, artifact, or operation id. A selector-free unprivileged public-socket `Admin`
request is only a wake signal. Under the coordinator lock, the broker obtains continuity
from its configured authoritative non-caller source, validates that source against the
disposition-pinned authority and prior private watermark, and only then creates the sealed
typed operation `RepairHostGenerationImmutableAuditContinuityV1`. The configured source
contract has two closed methods: acquire one canonical evidence value, and replay that exact
value by a broker-derived `ContinuityEvidenceReplayHandleV1`. It may not implement
"latest", caller-selected, timestamp-selected, or fallback lookup.

The coordinator method returns one private
`BindHostGenerationImmutableAuditContinuityRepairPermit<'coordinator>` that owns an
invariant exclusive borrow of that exact coordinator instance, its private unnameable
instance brand, and the validated evidence.
The only dispatch method is private, consumes the permit by value, and operates through the
borrow it contains; it accepts no second coordinator argument. The permit and evidence have
no public constructor, field, accessor, `Clone`, `Copy`, `Default`, `From`, `TryFrom`,
conversion, serde, raw-fd view, byte/digest reconstruction, or lifetime escape. Compiler
and API-surface negatives independently reject struct literals, field/accessor use, every
listed trait or conversion, serialize/deserialize, reconstruction from bytes/digest/fd,
cross-coordinator use, lifetime widening or return, and a second dispatch after consumption.
Each negative has its own compile-fail or API assertion and shrinkage poison. Direct broker,
root, backup-administrator claims, caller-supplied evidence, and previously consumed permits
cannot mint another permit.

The stable private replay and repair identities are:

```text
ContinuityEvidenceReplayHandleV1 =
  SHA-256("d2b:host-generation:audit-continuity-evidence-replay-handle:v1\0" ||
  disposition_authority_sha256 || retention_epoch_sha256 ||
  prior_watermark_sha256 || continuity_repair_sequence_u64_be ||
  authoritative_evidence_sha256)

ContinuityRepairAttemptIdV1 =
  SHA-256("d2b:host-generation:audit-continuity-repair-attempt:v1\0" ||
  coordinator_private_id || retention_epoch_sha256 || prior_watermark_sha256 ||
  continuity_repair_sequence_u64_be || authoritative_evidence_sha256 ||
  continuity_evidence_replay_handle)

continuityEvidenceReplayHandleSha256 =
  SHA-256("d2b:audit:host-generation:continuity-evidence-replay-handle:v1\0" ||
  private_continuity_evidence_replay_handle)

continuityRepairAttemptSha256 =
  SHA-256("d2b:audit:host-generation:continuity-repair-attempt:v1\0" ||
  private_continuity_repair_attempt_id)
```

Every id and digest preimage member is exactly 32 bytes except the eight-byte big-endian
sequence. The handle is deterministically reconstructible from the disposition, epoch,
prior watermark, sequence, and evidence digest; it is not source-selected and remains
broker-private. The operation identity is therefore reconstructible after a pre-only crash
without recovering a request frame or asking a changed source to choose bytes.

Before pre-audit, the already reserved replacement charge must cover exactly one continuity
evidence record, its encoded bytes, one immutable watermark publication, fixed audit and
settlement records, and later compaction. The operation refuses through the capacity
controller before continuity pre-audit if the one-record/131,072-byte per-attempt or
256-attempt per-intent subset is exceeded. An audited capacity refusal may append only its
capacity pre/outcome pair; it mutates no continuity prefix, watermark, prune state, or
covered operation. Standing-reserve exhaustion is the separate no-audit admission refusal
defined above.

The operation first publishes
`coordinator-immutable-audit-continuity-repair/pre-mutation` with exactly the fixed edge id,
`continuityRepairAttemptSha256`, `retentionEpochSha256`, `priorWatermarkSha256`,
`authoritativeEvidenceSha256`, `continuityEvidenceReplayHandleSha256`,
`continuityRepairSequence`, and exactly one nested deadline plan:

```text
ContinuityRepairDeadlinePlanV1 =
    BeforeDay90
  | Day90Reached { mandatoryPruneTargetSha256 }
```

There is no independent deadline flag, prune-required boolean, or nullable prune digest.
The broker constructs the plan from the original epoch deadline and current authoritative
lower bound under the lock. A before-day-90 plan cannot carry a prune target, and a
day-90-reached plan cannot omit one.

It then publishes broker-private sealed
`HostGenerationImmutableAuditContinuityRepairEvidenceV1`. Its canonical fields are exactly,
in order, `schemaVersion = 1`,
`kind = "host-generation-immutable-audit-continuity-repair-evidence"`,
`continuityRepairAttemptSha256`, `authoritativeEvidenceSha256`,
`canonicalEvidenceBytes`, `evidenceRealtimeSeconds`,
`evidenceBootTimeNanoseconds`, `evidenceBootIdBytes`, and
`authorityProofBytes`. The complete canonical record is at most 131,072 encoded bytes.
`canonicalEvidenceBytes`, both raw clock values, `evidenceBootIdBytes`, and
`authorityProofBytes` are sensitive body members. The evidence type and every owner,
replay, error, and settlement wrapper containing it implement no `Display`; their custom
`Debug` output is exactly the type name plus `([REDACTED])` and renders no field. Only
`authoritativeEvidenceSha256` may enter continuity audit. No watermark or prune mutation
may precede the pre record and durable evidence.

`authoritativeEvidenceSha256` is exactly
`SHA-256("d2b:host-generation:audit-continuity-authoritative-evidence:v1\0" ||
u32_be(len(canonicalEvidenceBytes)) || canonicalEvidenceBytes ||
evidenceRealtimeSeconds_u64_be || evidenceBootTimeNanoseconds_u64_be ||
u32_be(len(evidenceBootIdBytes)) || evidenceBootIdBytes ||
u32_be(len(authorityProofBytes)) || authorityProofBytes)`. The two bounded byte strings use
four-byte big-endian lengths, the clock values use eight-byte big-endian integers, and no
serializer output or implicit concatenation enters the formula. The evidence digest field
itself and `continuityRepairAttemptSha256` are not members of this preimage.

Fresh-process replay reconstructs the private handle and attempt from the durable pre fields,
coordinator-private id, and disposition, then invokes only the exact replay method. The
returned canonical record must reproduce the pre-bound digest, authority, epoch, prior
watermark, sequence, raw clock values, boot identity, and proof. Source unavailability is
`source-unavailable`; a changed source version, authority, handle binding, or any
nonidentical returned value is `source-conflict`. Neither may select a new attempt, evidence
digest, or deadline plan, and neither advances a watermark. Independent tests replace,
remove, and mutate the source after pre-only durability and require the same attempt identity
plus a closed failure, never newly selected bytes.

The continuity evidence is charged to the governed replaced set. It remains through every
restart and cannot be compacted while its repair is incomplete, while its governed set
exists, or before the matching continuity pre, watermark, outcome, and any mandatory prune
digest audit have been exported with both segment file and directory durability. The sealed
prune owner then removes the evidence with fd-relative `unlinkat`, parent `fsync`, and a
durable reduced census. A crash before unlink retries; after unlink but before parent sync it
revalidates absence and syncs; after parent sync but before census commit it commits only
the reduction; a completed compaction is no-write replay. Failure preserves the remaining
evidence or absence, blocks later mutation, and never weakens the one-record, byte, attempt,
or root aggregate ceilings. A read-independent lifecycle case owns pre-export retention,
file-only export refusal, directory-durable export admission, unlink, parent sync, census
commit, restart at every compaction prefix, and completed no-write replay hooks with one
removal poison per hook.

For `ContinuityRepairDeadlinePlanV1::Day90Reached`, the same permit must complete the prune pre/outcome
chain and durable absence before the watermark advances. More precisely, the
`Day90Reached` variant requires the exact `mandatoryPruneTargetSha256` to reach a matching
durable `Pruned | AlreadyPruned` outcome and durable absence. A `BeforeDay90` attempt may
not consume or cite an unrelated prune outcome.

The advanced watermark is not one mutable write. It is one immutable
`HostGenerationImmutableAuditContinuityWatermarkV1` final containing exactly
`schemaVersion = 1`,
`kind = "host-generation-immutable-audit-continuity-watermark"`,
`continuityRepairAttemptSha256`, `retentionEpochSha256`,
`priorWatermarkSha256`, `authoritativeEvidenceSha256`,
`continuityRepairSequence`, private greatest accepted real-time second,
boot-time nanosecond and boot identity digest, and `watermarkSha256`. It is published under
`HostGenerationImmutablePublicationV1` to a sequence-derived no-replace leaf. The current
watermark is the greatest contiguous completed repair whose exact watermark final and
matching outcome both validate; no mutable pointer, replacement, truncation, or partial
prefix can select it.

`ContinuityWatermarkPublicationPrefixV1` is exactly `Absent | HierarchyDurable |
InodeWritten | FileDurable | FinalLinked | FinalReopened | ParentDurable |
AncestorsDurable | Complete`. Reconciliation uses the common publication protocol:
pre-link prefixes discard any unnamed inode and republish; post-link prefixes accept only
absence or the exact final, reopen and identity-revalidate it, finish the first missing
directory sync, and never relink or replace it. Only `Complete` is `WatermarkApplied`.
The independent record-boundary registry visits the watermark class at
hierarchy-after-mkdir-before-sync, hierarchy-after-sync-before-inode-write,
after-write-before-file-sync, after-file-sync-before-link,
after-link-before-final-reopen, after-final-reopen-before-parent-sync,
after-parent-sync-before-ancestor-sync, after-final-directory-sync, and
completed-response-loss-no-write. Each boundary has a separate production hook and
shrinkage poison. An exact completed response-loss replay performs no evidence, prune,
watermark, outcome, census, or audit write.

Continuity publication and source failures are closed:

```text
ContinuityRepairSourceFailureClassV1 =
    source-unavailable
  | source-conflict

ContinuityRepairPublicationFailureClassV1 =
    hierarchy
  | write
  | file-sync
  | link
  | reopen
  | directory-sync
  | conflict
  | audit-publication
  | outcome-publication

ContinuityRepairFailureV1 =
    Source(ContinuityRepairSourceFailureClassV1)
  | Retention(RetentionDegradedClassV1)
  | Publication(ContinuityRepairPublicationFailureClassV1)
```

The matching outcome repeats the exact pre identity and deadline plan and has exactly one
nested variant:

```text
ContinuityRepairOutcomeV1 =
    RepairedBeforeDay90 { repairedWatermarkSha256 }
  | RepairedAfterMandatoryPrune {
      pruneOutcomeSha256,
      repairedWatermarkSha256
    }
  | DegradedBeforeDay90(ContinuityRepairFailureV1)
  | DegradedDay90BeforePrune(ContinuityRepairFailureV1)
  | DegradedDay90AfterPrune {
      pruneOutcomeSha256,
      failure: ContinuityRepairFailureV1
    }
```

Constructors consume the pre variant and reachable prefix. They cannot construct day-90
success without the exact mandatory prune, before-day-90 success with any prune, a
before-prune failure carrying a prune outcome, an after-prune failure without that exact
outcome, success plus failure, or a deadline plan different from pre-audit. Audit
projection, strict schema, wire snapshot, human/JSON goldens, and deserializers reject every
cross-pair and unknown variant. No outcome stores a caller-supplied action.

If a hard source, hierarchy, write, file-sync, link, reopen, directory-sync, or conflict
failure occurs after pre, the broker appends the one reachable degraded outcome when audit
storage permits and advances no watermark. If the outcome itself cannot become durable,
the immutable prefix is
`ContinuityRepairSettlementV1::PendingOutcomePublication { deadlinePlan, failure }`.
It returns only typed `Pending`, blocks every later mutation, and restart settles that exact
attempt before acquiring new evidence or dispatching another repair. A watermark-complete
prefix with no outcome remains pending settlement and never republishes the watermark.
`outcome-publication` is therefore representable only in pending settlement until its
matching degraded outcome is durable. Strict response schemas and human/JSON goldens pin
completed repaired, completed degraded, and pending forms; table-driven hard-failure tests
inject every source/publication class at evidence, watermark, and outcome publication and
reject fallback strings, nullable sibling failures, or class/action substitutions.

The only valid successful prefixes are:

```text
Absent
  -> PreAudited
  -> EvidenceDurable
     BeforeDay90:
       -> WatermarkPublication(<exact prefix>)
       -> WatermarkApplied
       -> CompletedRepairedBeforeDay90
     Day90Reached:
       -> MandatoryPruneDurable
       -> WatermarkPublication(<exact prefix>)
       -> WatermarkApplied
       -> CompletedRepairedAfterMandatoryPrune
```

`EvidenceDurable` advances directly to watermark publication only for `BeforeDay90`.
A post-pre failure may instead append one reachable completed degraded outcome with zero
watermark advance, or enter pending settlement if that append cannot complete. Restart
reacquires the coordinator, reconstructs the exact handle and attempt, revalidates any
durable evidence, prune, and watermark final, and resumes only the first missing step.
Pre-only, evidence-only, prune-complete, every watermark publication boundary,
watermark-without-outcome, outcome-publication, pending-settlement, and
completed-response-loss prefixes are independent fresh-process cases. Evidence without
pre, watermark without pre/evidence, watermark before required prune, day-90 success without
durable prune absence, before-day-90 use of an unrelated prune, duplicate
pre/evidence/watermark/outcome, nonidentical source replay, sequence/predecessor/handle
mismatch, deadline/outcome cross-pair, or a second dispatch degrades the root and blocks
later mutation. The lifecycle malformed-prefix case has an independently named hook and
removal poison for each listed ordering or pairing conflict. Completed response loss
returns the stored outcome with zero write.

Repair advances the existing epoch's trusted lower bound; it never creates a replacement
epoch or resets either the day-30 threshold or original day-90 deadline. Startup performs
this typed operation and mandatory catch-up before accepting handoff work, and the internal
wake repeats it without Admin traffic. A reboot, discontinuity, delayed repair, repeated
Admin wake, or broker restart therefore cannot extend retention past the original deadline.
Missing or invalid authoritative evidence keeps the root degraded and blocks mutation; it
never accepts caller evidence or publishes a prune-governing anchor. Tests cross day 90 on
same boot, reboot, forward discontinuity, delayed repair, startup, and idle wake and require
either deadline-preserving mandatory prune before watermark/outcome from authoritative
evidence or continued fail-closed unavailability with no new anchor.

Pruning is the typed broker operation
`PruneHostGenerationImmutableAuditBackupsV1`. It is not callable by the daemon, a public
role, root, or a direct broker client as a privileged mutation. A selector-free Admin
retention-reconciliation request may only wake the broker's coordinator; the broker's
sealed coordinator owner must still attenuate one
`PruneHostGenerationImmutableAuditPermit<'coordinator>` and consume it by value at
dispatch. The permit has private fields and is created only by the private coordinator
method after exact epoch, watermark, census, reservation, and audit-prefix validation. It
implements no `Clone`, `Copy`, `Default`, `From`, `TryFrom`, serde serialization or
deserialization, conversion, public constructor, public field, accessor, raw-fd view, byte
import, digest reconstruction, or independent lifetime. No digest, wire field, request,
caller claim, decoded DTO, or previously consumed permit can reconstruct it. Compile-fail
and API-surface negatives cover struct literals, field access, every listed trait and
conversion, serialize/deserialize round trips, clone/copy/duplicate, reconstruction from
bytes/digest/fd, cross-coordinator use, lifetime escape, and second dispatch with one permit.
Under that capability and the same coordinator lock, pruning
reserves and append-only publishes
`coordinator-immutable-audit-backup-prune/pre-mutation` before `unlinkat`. The pre record
contains only the fixed edge id, `pruneAttemptSha256`, `retentionEpochSha256`,
`backupMemberSha256`, `priorCensusSha256`, and closed `eligibility = day-30-through-day-89 |
day-90-mandatory`. Its one private and one audit identity are:

```text
PruneAttemptIdV1 =
  SHA-256("d2b:host-generation:immutable-audit-backup-prune-attempt:v1\0" ||
  coordinator_private_id || retention_epoch_sha256 || backup_member_sha256 ||
  prior_census_sha256 || eligibility_tag_u8)

pruneAttemptSha256 =
  SHA-256("d2b:audit:host-generation:immutable-audit-backup-prune-attempt:v1\0" ||
  private_prune_attempt_id)
```

Every id or digest preimage member is exactly 32 bytes and `eligibility_tag_u8` is exactly
`0x01` for `day-30-through-day-89` or `0x02` for `day-90-mandatory`. No serializer output,
caller value, path, clock sample, or variable-length concatenation enters either formula.
Every prune pre/outcome projection uses only the canonical `pruneAttemptSha256`; no second
prune identity or relabeled continuity/restoration digest is accepted. It then reopens the
exact leaf from the stable root by the common
fd-relative `openat2` policy, revalidates identity, calls `unlinkat` with a
single-component leaf, syncs the held parent dirfd, durably commits the reduced census, and
append-only publishes the matching outcome. The outcome repeats those fields and adds only
one nested `PruneOutcomeV1 = Pruned | AlreadyPruned |
Degraded(PruneFailureClassV1)`, where `PruneFailureClassV1` is exactly
`unlink | directory-sync | census | audit-publication`. There is no nullable sibling
failure class and no constructor can represent a success plus failure or degraded without
one failure.

Restart classifies pre-only, unlinked-but-unsynced, synced-but-old-census,
new-census-without-outcome, and complete prefixes. It never repeats an already reflected
unlink, never reduces the census before parent durability, and appends exactly one matching
outcome. Loss of the completed response is no-write replay. If outcome publication itself
cannot become durable, the operation remains typed `pending-settlement`; restart settles
the immutable prefix before another handoff mutation. A prune, clock, capacity, or
settlement failure blocks the next handoff mutation.

The existing broker starts one internal retention catch-up before accepting handoff work and
arms an internal wake for the next day-30 or day-90 boundary. This is an in-process duty of
`d2b-priv-broker.service`, not a new timer or service and not dependent on an Admin request.
Startup and wake catch-up acquire the coordinator lock, mint the private permit, and either
complete an audited mandatory prune or publish typed retention degradation. An Admin
reconciliation request is an additional wake only. Tests advance beyond day 90 with no Admin
traffic and require the complete prune pre/outcome chain and absence on same boot, after
reboot, and after a forward discontinuity. The latter two paths must first validate
authoritative non-caller continuity and consume the sealed repair permit; ambiguity may
block service before that validation but may not reset the epoch, accept caller evidence,
publish repair, or retain the set once the authoritative lower bound reaches the original
day-90 deadline.

`RetentionCapacityClassV1` is exactly
`intent-member-limit | intent-byte-limit | root-intent-limit | root-member-limit |
root-byte-limit | root-publication-record-limit | root-publication-byte-limit |
restoration-record-limit | restoration-byte-limit | restoration-attempt-limit |
continuity-evidence-record-limit | continuity-evidence-byte-limit |
continuity-repair-attempt-limit | pending-staging-record-limit |
pending-staging-byte-limit`.
`RetentionDegradedClassV1` is exactly
`clock-rollback | clock-watermark | epoch-invalid | clock-forward-discontinuity |
clock-continuity-ambiguous | clock-overflow | unlink | directory-sync | census |
audit-publication | pending-settlement | standing-reserve-missing |
standing-reserve-overdrawn | standing-reserve-duplicated |
standing-reserve-unaccounted`.

The audited capacity, no-write admission-refusal, and degraded DTOs each contain exactly one
validated nested variant carrying one class from its own enum. Neither stores a sibling
action. `CapacityAdmissionRefusalV1` contains only
`StandingReserveExhausted`; it has no reservation attempt digest, prefix, outcome, or
generation transition. Custom serialization derives the exact public `failureClass` and
`action` pair from the variant. Custom deserialization constructs the variant from the
closed class, verifies that any wire action is byte-identical to the derived action without
storing or trusting it, and rejects a missing/mismatched action, a class from another
branch, or any illegal class/action cross-product. The total derived mapping is:

| `failureClass` | Exact `action` |
| --- | --- |
| `intent-member-limit`, `intent-byte-limit`, `root-intent-limit`, `root-member-limit`, `root-byte-limit`, `root-publication-record-limit`, `root-publication-byte-limit`, `restoration-record-limit`, `restoration-byte-limit`, `restoration-attempt-limit`, `continuity-evidence-record-limit`, `continuity-evidence-byte-limit`, `continuity-repair-attempt-limit`, `pending-staging-record-limit`, `pending-staging-byte-limit` | `reconcile-immutable-audit-retention` |
| `standing-reserve-exhausted` | `repair-retention-audit-and-reconcile` |
| `clock-rollback`, `clock-watermark`, `epoch-invalid`, `clock-forward-discontinuity`, `clock-continuity-ambiguous` | `repair-retention-clock-discontinuity` |
| `clock-overflow` | `preserve-and-escalate-retention-clock-overflow` |
| `unlink`, `directory-sync` | `repair-retention-storage-and-reconcile` |
| `census` | `repair-retention-census-and-reconcile` |
| `audit-publication`, `pending-settlement` | `repair-retention-audit-and-reconcile` |
| `standing-reserve-missing`, `standing-reserve-overdrawn`, `standing-reserve-duplicated`, `standing-reserve-unaccounted` | `preserve-and-escalate-audit-integrity-incident` |

Private constructors, wire/schema snapshots, and table-driven negatives reject prune
success-plus-failure, degraded-without-failure, a retention class paired with another
action, a capacity class in the degraded or admission branch, an admission class in the
audited capacity branch, a degraded class in either capacity branch, caller-provided
action, unknown variants, multiple nested branches, and missing branches. Human and JSON
projections are generated only from validated variants. Independent schema, wire,
human/JSON golden, and lifecycle cases cover every class and action, including all five
standing-reserve states and both the audited and no-write capacity refusal shapes.

Continuity-specific source and publication projection derives these exact actions without
storing one:

| Continuity failure class | Exact `action` |
| --- | --- |
| `source-unavailable` | `repair-continuity-authoritative-source` |
| `source-conflict` | `preserve-and-escalate-continuity-source-conflict` |
| `hierarchy`, `write`, `file-sync`, `link`, `reopen`, `directory-sync` | `repair-retention-storage-and-reconcile` |
| `conflict` | `preserve-and-escalate-continuity-publication-conflict` |
| `audit-publication`, `outcome-publication` | `repair-retention-audit-and-reconcile` |

The nested `Retention` branch uses the existing retention mapping above. Strict continuity
schemas, wire snapshots, human/JSON goldens, constructors, and deserializers reject a
source class in the publication branch, a publication class in the retention branch,
pending without `outcome-publication`, a mismatched derived action, and every unknown or
multiple branch.

The sealed private age-anchor state is the only wall-clock carrier. No response or audit
projection carries a path, member bytes, artifact bytes, wall-clock value, errno, uid, pid,
or free-form value. Tests use a hermetic injected clock and pin day 29 refusal, day 30
eligibility, day 90 mandatory absence, pre-audit before the first clock sample,
fresh-process pre-only sampling, restart-stable candidates with no resampling after any
candidate publication boundary, same-boot long crash, safe same-boot advance, unsafe
forward step, changed-boot ambiguity, authoritative non-caller continuity validation,
Admin-wake-only repair, original-deadline preservation across reboot/discontinuity, and
post-day-90 prune before repair outcome. They also pin invalid-anchor,
watermark-persistence, rollback, overflow refusal, every per-intent and root aggregate
limit, standing capacity-control reserve creation/exhaustion/export reconstruction,
reservation before handoff, every reservation/release prefix, release after durable prune,
release after immutable zero-mutation, sealed prune-capability admission and all
compiler/API negatives, immutable nested pre/outcome audit,
every unlink/directory-sync/census/audit failure, every crash prefix, completed
response-loss replay, parent/symlink/magic-link/cross-device/final-identity races, descriptor
exec-leak absence, internal idle wake and startup catch-up without Admin traffic,
continuity evidence 1/2-record, 131,072/131,073-byte and 256/257-attempt boundaries,
post-export retention/compaction restart, exactly-once outcomes, illegal DTO cross-products,
and redacted reports.

The no-observable private-identifier contract covers every identifier introduced by this
publication root: the raw publication-root operation id and private root reference,
`GoverningCapacityOperationIdV1`, `CapacityReservationAttemptIdV1`,
`CapacityReleaseAttemptIdV1`, `RetentionAnchorAttemptIdV1`,
`ContinuityEvidenceReplayHandleV1`, `ContinuityRepairAttemptIdV1`,
`PruneAttemptIdV1`, `RestorationAttemptIdV1`,
`RestorationSettlementAttemptIdV1`, every complete preimage, and each unqualified
hexadecimal encoding. Canaries inject every value independently and require zero occurrences
in public DTO/schema/snapshot/example bytes, human or JSON output, error, `Display`, log,
trace/span, metric, panic, or `Debug`. Audit is the sole exception and may contain only the
named domain-separated fields `publicationRootOperationSha256`,
`publicationRootRefSha256`, `capacityReservationAttemptSha256`,
`capacityReleaseAttemptSha256`, `retentionAnchorAttemptSha256`,
`continuityEvidenceReplayHandleSha256`, `continuityRepairAttemptSha256`,
`pruneAttemptSha256`, `restorationAttemptSha256`, and
`restorationSettlementAttemptSha256` with the exact formulas in this section. Each field is
allowed only on its named record family. Audit may not contain a raw id, raw preimage,
unqualified hash, one domain's digest relabeled as another, or a canonical restoration
attempt digest substituted for a settlement digest. A read-independent canary visitor and
one shrinkage poison per private identifier, complete preimage, unqualified encoding, named
audit field, and observable surface enforce the complete list.

The continuity evidence body has a second independent leakage matrix. Distinct canaries for
`canonicalEvidenceBytes`, `evidenceRealtimeSeconds`,
`evidenceBootTimeNanoseconds`, `evidenceBootIdBytes`, and `authorityProofBytes` must each be
absent from public DTO/schema/snapshot/example bytes, response, human/JSON output, error,
`Display`, audit, log, trace/span, metric, panic, and custom `Debug`. A test for one member
or surface cannot satisfy another. Every sensitive member and every surface has its own
visitor hook and removal poison; removing the custom redacted `Debug` implementation or
adding `Display` is an API/compiler failure before runtime cases.

Restoration requires a signed `HostGenerationImmutableAuditRestorationV1` from the
disposition-pinned backup authority. Its canonical fields are exactly, in order,
`schemaVersion = 1`, `kind = "host-generation-immutable-audit-restoration"`, `member`,
`failureClass`, `canonicalMemberSha256`, `backupMemberSha256`, `predecessorSha256`,
`observedMemberSha256`, `authoritySha256`, `verificationKeySha256`, and
`signatureEd25519`. `failureClass` is exactly
`missing | mismatch | unauthenticated | noncontiguous`;
`observedMemberSha256` is null only for `missing` and otherwise commits the preserved
original member. The signature covers the exact canonical object under the restoration
domain selected by the disposition. A valid signature is integrity evidence only and never
grants caller authority.

The disposition-pinned external procedure
`host-generation-immutable-audit-backup-acquisition-v1`, owned by the named backup
authority, is the only artifact-acquisition procedure. It accepts the exact bounded
`audit-restoration-required` diagnostic plus authenticated coordinator evidence, selects no
caller-provided path/member override, and returns exactly one canonical signed restoration
artifact as a regular single-link current-user mode-`0600` file of at most 131,072 bytes.
An acquisition failure changes no coordinator state. The repository-owned submission
command is exactly
`d2b-host-generation-deploy --restore-immutable-audit-backup PATH [--json]`. T595 owns the
unprivileged parser and public-socket client. It opens `PATH` once with
`O_RDONLY|O_NOFOLLOW|O_CLOEXEC`, requires a regular single-link current-user mode-`0600`
file, reads at most 131,073 bytes, rejects overflow and noncanonical bytes, and sends the
same opened bytes. `PATH` is never sent to the daemon or rendered. The command accepts
exactly one path and optional `--json`; member/failure selectors, authority/key/token
arguments, `--force`, and an extra positional argument are invalid. Root invocation is the
distinct authorization refusal defined below, not the misleading one-artifact shape error.

T592 owns the shared wire DTO and broker operation
`RestoreHostGenerationImmutableAuditMemberV1`. Its request is exactly
`RestoreHostGenerationImmutableAuditMemberRequestV1 { schemaVersion = 1,
kind = "restore-host-generation-immutable-audit-member", operationId,
restorationArtifactBytes }`, where `operationId` is the first 16 bytes of
`SHA-256("d2b:host-generation:audit-restoration-request:v1\0" ||
artifact_sha256[32])` and the artifact byte string is 1 through 131,072 bytes. Unknown
fields are denied. No request field carries a path, selector, uid, pid, authority token,
member override, failure override, or free-form value.

The response is the closed nested enum
`RestoreHostGenerationImmutableAuditMemberResponseV1`. It is not a struct with sibling
`error`, nullable `failureClass`, settlement, or action fields:

```text
RestoreHostGenerationImmutableAuditMemberResponseV1 =
    Completed(RestorationCompletedV1)
  | Refused(RestorationRefusalV1)
  | Pending(RestorationPendingV1)
  | Degraded(RestorationDegradedV1)
```

`Completed` contains `operationId`, `member`, and
`outcome = restored | already-restored`. `Refused` contains `operationId` and exactly one
nested refusal:

```text
RestorationRefusalV1 =
    InvalidRequest(RestorationInvalidRequestClassV1)
  | RootRefused
  | Unauthorized
  | ArtifactInvalid(RestorationArtifactInvalidClassV1)
  | Conflict(RestorationConflictClassV1)
  | RetentionCapacity(RetentionCapacityClassV1)
  | RetentionDegraded(RetentionDegradedClassV1)
```

`RestorationInvalidRequestClassV1` is exactly
`missing-schema-version | wrong-schema-version | missing-kind | wrong-kind | unknown-field |
missing-operation-id | operation-id-length | operation-id-digest-mismatch |
missing-artifact-bytes | empty-artifact-bytes | over-limit-artifact-bytes | path-field |
selector-field | uid-field | pid-field | authority-token-field | member-override-field |
failure-override-field | free-form-field`.
`RestorationArtifactInvalidClassV1` is exactly
`canonical | size | signature | domain | key | authority | member | failure-class |
predecessor | backup-binding | observed-member-binding`.
`RestorationConflictClassV1` is exactly
`provenance | member | operation-id | duplicate-supersession`.
The two retention enums are the applicable closed domains in the retention table.

`Pending` contains `operationId` and exactly one nested
`RestorationPendingReasonV1`:

```text
RestorationPendingReasonV1 =
    Publication(RestorationPublicationFailureClassV1)
  | SettlementReplay(RestorationPublicationFailureClassV1)
```

The variant totally derives settlement state `restart-settlement-pending` and the public
failure class from its carried publication class. `Degraded` contains `operationId`, the closed
`RestorationPublicationFailureClassV1`, and settlement state `repair-required`.
`RestorationPublicationFailureClassV1` is total over every post-pre publication fault:
`hierarchy | write | file-sync | link | reopen | directory-sync | outcome-publication`.
Hierarchy is never represented by null or collapsed into write.

The strict wire representation preserves that nesting: the envelope has only
`schemaVersion`, `kind`, `operationId`, and exactly one of `completed`, `refused`, `pending`,
or `degraded`. Each nested object has its own discriminant and per-error class domain.
Custom serialization derives every public error token, action, and settlement projection
from the validated variant. Custom deserialization rejects more than one nested branch, a
missing branch, a class from another branch, an action or settlement value that differs from
the variant's derived value, a settlement value in a refusal, success plus failure, pending
without a reason or required carried class, degraded without a class, or any other illegal
cross-product. No action or settlement input is stored as independent state. `RootRefused` and
`Unauthorized` cannot substitute for one another. `operationId` is null only when
`InvalidRequest` proves the request lacked one valid 16-byte id. The broker-private
`RestorationAttemptIdV1` and every raw or encoded form of it are absent from all public
response variants and wire fields.

This operation never falls back to `BrokerErrorResponse.message`, another free-form broker
error string, or an unrecognized enum. The renderer maps wire artifact `size` to public
`input-size`; its local pre-wire input checks alone produce `input-type`, `input-owner`,
`input-mode`, or `input-link-count`. The generated strict JSON schema, wire snapshot, shared
CLI renderer, human/JSON goldens, schema-drift tests, and table-driven illegal-cross-product
tests must all carry this same nested enum. Leakage canaries inject the private attempt id,
its raw preimage, and its unqualified hexadecimal encoding and require zero occurrences in
response, schema examples, human/JSON output, error, `Display`, audit, log, trace/span,
metric, panic, or `Debug`.

The operation is admitted exclusively from the existing public socket's consumed `Admin`
capability. Every other local role, including `Launcher`, workload, Zone, and
`HostShutdown`, plus root, nonmember, unauthenticated-local, direct-broker-socket, and remote
callers is denied before coordinator or backup access. Signature verification cannot
upgrade any denied caller.

Before taking the coordinator lock, the broker may perform only state-independent request
framing, canonical decoding, size checks, digest reconstruction, and restoration signature
verification. It may not accept a coordinator identity, backup member, predecessor,
observed state, capacity reservation, or artifact-to-backup binding there. Under the lock it
reopens the stable coordinator root and every required backup leaf through the common
fd-relative policy, revalidates coordinator and backup mount/device/inode identities,
rehashes the exact request bytes, validates member/failure/predecessor/observed-state and
artifact-to-backup bindings, and reserves retention capacity immediately before the first
mutation. Any state change between preparse and lock acquisition refuses with zero
coordinator, backup, restoration, or audit mutation. A latch race replaces the coordinator
or backup after signature verification and proves this under-lock revalidation is
mandatory.

After that under-lock validation, the broker derives one
`RestorationAttemptIdV1` by the one canonical formula
`SHA-256("d2b:host-generation:restoration-attempt:v1\0" ||
operation_id_16_bytes || restoration_artifact_sha256_32_bytes)`. The same artifact always
addresses the same attempt; different artifact bytes under the operation id are conflict.
This identifier remains broker-private. Every restoration pre, provenance, and outcome
audit projection uses only
`restorationAttemptSha256 =
SHA-256("d2b:audit:host-generation:restoration-attempt:v1\0" ||
private_restoration_attempt_id_32_bytes)`; no audit or settlement record carries the raw
attempt id. Restoration pre, provenance, and outcome records use that exact audit digest;
settlement records use the separate canonical settlement identity below. Those formulas are
the only restoration-attempt definitions. Independent fixed vectors change the operation
id, artifact digest, private domain, and audit domain one at a time and require every pre,
provenance, outcome, schema, and golden consumer to reconstruct the same bytes.
After the separately pre-audited capacity reservation completes, and before any body-bearing
private evidence, provenance, effective member, or settlement mutation, the broker appends
the durable fixed-field
`coordinator-immutable-audit-restoration/pre-mutation` audit record. It then append-only
publishes one broker-private, non-observable
`HostGenerationImmutableAuditRestorationEvidenceV1` preparatory record. That private record
contains the authenticated `canonicalMemberBytes` and complete signed
`restorationArtifact` plus their named digests. It is readable only by the broker's sealed
restoration/replay owner, is never exported as audit, and has no public accessor,
serialization response, `Debug`, `Display`, log, metric, or span projection. By itself it
authorizes and changes nothing in the coordinator, backup census, effective audit view, or
restored member. A process death after pre audit but before private evidence loses the
request frame. The durable pre-only prefix therefore blocks every later coordinator
mutation and remains broker-private `pre-only-awaiting-identical-resubmission`; it never
reconstructs or publishes a body on restart and synthesizes no response without a caller.
Only a new unprivileged public-socket `Admin` submission of the
byte-identical artifact may resume it. That submission is reauthorized, reverified, and
fully revalidated under the lock, must reconstruct the exact operation id and private
attempt id, and then may publish evidence. A different artifact or binding is preserved as
conflict. No restoration body may become durable before its matching pre audit.

The separate audit provenance record is
`HostGenerationImmutableAuditRestoredMemberV1` with fields exactly, in order,
`schemaVersion = 1`, `kind = "host-generation-immutable-audit-restored-member"`,
`restorationAttemptSha256`, `member`, `failureClass`, `canonicalMemberSha256`,
`backupMemberSha256`, `predecessorSha256`, `observedMemberSha256`,
`restorationArtifactSha256`, and `privateEvidenceSha256`. It contains no member or artifact
body. The original mismatched, unauthenticated, or noncontiguous member is never replaced
or deleted; the effective audit view accepts the private evidence plus fixed-digest
provenance as its single append-only supersession only when the complete
evidence/pre/provenance/outcome chain validates. Missing uses a null observed digest.
Multiple or conflicting supersessions are invalid coordinator state. Finally the broker
appends the matching `coordinator-immutable-audit-restoration/outcome` record. Every append
uses `HostGenerationImmutablePublicationV1`.

The restoration pre-mutation record contains exactly the fixed edge id,
`restorationAttemptSha256`, `member`, `failureClass`, `backupMemberSha256`,
`canonicalMemberSha256`, `predecessorSha256`, closed
`observedState = absent | mismatch | unauthenticated | noncontiguous`, nullable
`observedMemberSha256`, and `restorationArtifactSha256`. The outcome repeats every field and
adds one nested outcome:
`Restored`, `Conflict(Provenance | Member)`, or
`PublicationDegraded(RestorationPublicationFailureClassV1)`. No nullable sibling failure
class exists. The total publication failure class includes `hierarchy`, `write`,
`file-sync`, `link`, `reopen`, `directory-sync`, and `outcome-publication`. The attempt
digest is always the canonical `restorationAttemptSha256` formula above. The fixed-width
member/failure tags and listed record digests are authenticated fields but are not an
alternative attempt-digest preimage. No serializer output, path, raw member bytes, artifact
bytes, caller identity, wall-clock value, errno, or free-form text enters either attempt
formula. Authorization, input-shape, signature/domain/key/authority, member/failure,
predecessor, backup-binding, state-race, capacity, and size refusals occur before
restoration pre-mutation append and have zero coordinator, backup, restoration, provenance,
or member mutation. Noncapacity refusals and standing-reserve admission refusal also have
zero audit mutation. An audited retention-capacity refusal may append only the capacity
controller's exact pre/outcome pair and leaves every restoration record absent.

Once pre-mutation is durable, every private-evidence, provenance, hierarchy, or effective
member syscall failure is a post-pre failure and must settle to exactly one matching outcome
event when storage permits. A pre-only state is internally pending but has no caller and no
response to replay. The first byte-identical authorized resubmission itself drives
continuation and returns completed, conflict, or the actual later publication failure. If a
later outcome event cannot become durable, the response is `Pending` with a typed
`Publication` or `SettlementReplay` reason and the same artifact resubmission drives
settlement before another mutation. A durable `PublicationDegraded` outcome is explicitly
nonterminal. It creates settlement state
`repair-required`, preserves the complete append-only attempt prefix, and permits only the
same operation id plus byte-identical restoration artifact to resume after
`host-generation-restoration-storage-repair-v1`.

The append-only settlement chain is:

```text
pre-audited -> publishing -> restored
pre-audited -> pre-only-awaiting-identical-resubmission -> publishing
publishing -> degraded-repair-required -> repair-resume-pre-audited -> publishing
publishing -> pending-restart-settlement -> publishing | degraded-repair-required
```

Each event has one private settlement identity and one audit projection:

```text
RestorationSettlementAttemptIdV1 =
  SHA-256("d2b:host-generation:restoration-settlement-attempt:v1\0" ||
  private_restoration_attempt_id || settlement_sequence_u64_be ||
  prior_settlement_sha256 || settlement_state_tag_u8)

restorationSettlementAttemptSha256 =
  SHA-256("d2b:audit:host-generation:restoration-settlement-attempt:v1\0" ||
  private_restoration_settlement_attempt_id)
```

The private restoration attempt and prior settlement digest are exactly 32 bytes, the
sequence is eight-byte big-endian, and the closed settlement-state tag is one byte. No
serializer output or variable-length concatenation enters either formula. The event
contains only `restorationSettlementAttemptSha256`, the strictly increasing settlement
sequence, `priorSettlementSha256`, and one typed state. A restoration pre, provenance, or
outcome digest cannot substitute for this settlement digest, and the settlement digest
cannot appear on those record families. Independent vectors change the restoration
attempt, sequence, predecessor, state tag, private domain, and audit domain one at a time
and pin every settlement schema, snapshot, audit record, and golden.
`repair-resume-pre-audited` is a new
fixed-field audit event under the same operation id, not a new restoration attempt or a
second provenance append. Under the coordinator lock, replay revalidates the same artifact,
predecessor, backup, observed-state, reservation, and repaired storage hierarchy; it then
continues at the first missing record and appends a final `Restored` outcome. The durable
degraded event remains history but no longer controls current settlement after the
contiguous repaired event. An exact completed chain returns `already-restored` with zero
write. A different artifact, operation id, predecessor, or observed state cannot resume the
attempt. Signed private evidence and fixed-digest provenance remain append-only for the
lifetime of coordinator history.

Restart classifies every hierarchy, pre/evidence/provenance/outcome/settlement write, file-sync, link,
final-reopen, parent-sync, ancestor-sync, final-directory-sync, and response-loss boundary
through the shared publication protocol. A durable evidence-only prefix revalidates the
byte-identical artifact only as an invalid pre-audit ordering and never appends a missing
pre record after the fact. A durable pre-only prefix has no recoverable request frame,
admits no caller-free settlement, and remains blocked until the same Admin-authorized
byte-identical artifact is resubmitted. A durable pre-plus-evidence prefix has its body and
may settle without caller input. Durable provenance without outcome appends only the
matching outcome.
An existing exact complete chain returns `already-restored` with zero write, including
response-loss replay. A nonidentical final or same operation id with different artifact
bytes is preserved and returns conflict. Independent fault tests cover all four restoration
failure classes, every record/boundary pair, every write/`fsync`/link/reopen/directory-sync
failure after pre-mutation, provenance/member conflict, pending and degraded settlement,
hierarchy failure, fresh-process pre-only blocking, byte-identical authorized resubmission,
different-artifact conflict, degraded-to-repair-resume-to-restored convergence, restart
after every degraded and repaired threshold, exactly-one effective outcomes, and completed
no-write replay. Distinct canary member bytes, artifact bytes, private attempt ids, and
unqualified attempt encodings must be absent from response, human/JSON output, error,
`Display`, audit, log, trace/span, metric, panic, and `Debug`; the process-local bounded
request frame and post-pre private evidence record are the only intentional body carriers.

Successful human output is exactly
`host generation handoff immutable audit restoration complete\noutcome: <RESTORED_OR_ALREADY_RESTORED>\nsettlement: restored\nmember: <CLOSED_MEMBER>\naction: rerun-repair-authorized-handoff\n`; the outcome token is lowercase
`restored | already-restored`. JSON is exactly
`{"schemaVersion":1,"kind":"host-generation-immutable-audit-restoration-result","outcome":"<RESTORED_OR_ALREADY_RESTORED>","settlement":"restored","member":"<CLOSED_MEMBER>","action":"rerun-repair-authorized-handoff"}`.
Both outcomes exit `0`. Invalid invocation exits `2` with
`host generation handoff audit restoration refused\naction: restore-with-one-artifact\n`
or exact JSON
`{"schemaVersion":1,"kind":"host-generation-handoff-error","error":"invalid-invocation","action":"restore-with-one-artifact"}`.
The release-sealed client cannot construct any of the broker's nineteen
`InvalidRequest` classes. Receiving one after a locally valid invocation is therefore a
client/broker contract-integrity failure, not invalid operator syntax. Each exits `4` with
the exact class-specific form
`host generation handoff audit restoration request rejected\nfailure-class: <INVALID_REQUEST_CLASS>\naction: repair-restoration-client-broker-contract\n`
or
`{"schemaVersion":1,"kind":"host-generation-handoff-error","error":"audit-restoration-invalid-request","failureClass":"<INVALID_REQUEST_CLASS>","action":"repair-restoration-client-broker-contract"}`.
There is one golden for each of the nineteen literal class values; no catch-all golden or
generic broker message is accepted.
Root invocation exits `4` with
`host generation handoff audit restoration requires unprivileged local Admin\naction: use-unprivileged-local-admin-restoration-session\n`
or exact JSON
`{"schemaVersion":1,"kind":"host-generation-handoff-error","error":"audit-restoration-root-refused","action":"use-unprivileged-local-admin-restoration-session"}`.
Authorization refusal exits `4` with
`host generation handoff audit restoration unauthorized\naction: use-local-admin-public-socket\n` or exact JSON
`{"schemaVersion":1,"kind":"host-generation-handoff-error","error":"audit-restoration-unauthorized","action":"use-local-admin-public-socket"}`.
Artifact refusal exits `4`, exposes only closed
`failureClass = input-type | input-owner | input-mode | input-link-count | input-size |
canonical | signature | domain | key | authority | member | failure-class | predecessor |
backup-binding | observed-member-binding`, and is exactly
`host generation handoff audit restoration artifact invalid\nfailure-class: <CLOSED_FAILURE_CLASS>\naction: reacquire-immutable-audit-backup\n` or
`{"schemaVersion":1,"kind":"host-generation-handoff-error","error":"audit-restoration-artifact-invalid","failureClass":"<CLOSED_FAILURE_CLASS>","action":"reacquire-immutable-audit-backup"}`.
A durable conflict exits `4` with exactly
`host generation handoff audit restoration conflict\naction: preserve-and-escalate-audit-restoration-conflict\n` or
`{"schemaVersion":1,"kind":"host-generation-handoff-error","error":"audit-restoration-conflict","action":"preserve-and-escalate-audit-restoration-conflict"}`.
A retention-capacity refusal exits `4` with exactly
`host generation handoff immutable audit retention capacity unavailable\nfailure-class: <CLOSED_RETENTION_CAPACITY_CLASS>\naction: <ACTION_FROM_RETENTION_CAPACITY_TABLE>\n`
or
`{"schemaVersion":1,"kind":"host-generation-handoff-error","error":"audit-backup-retention-capacity","failureClass":"<CLOSED_RETENTION_CAPACITY_CLASS>","action":"<ACTION_FROM_RETENTION_CAPACITY_TABLE>"}`.
This is the audited `RefusedZeroMutation` shape: its capacity pre/outcome pair is durable,
while the ledger and restoration operation are unchanged. Pre-audit standing-reserve
exhaustion instead exits `4` with exactly
`host generation handoff immutable audit capacity admission unavailable\nfailure-class: standing-reserve-exhausted\naction: repair-retention-audit-and-reconcile\n`
or
`{"schemaVersion":1,"kind":"host-generation-handoff-error","error":"audit-capacity-admission-refused","failureClass":"standing-reserve-exhausted","action":"repair-retention-audit-and-reconcile"}`.
It carries no capacity attempt digest or generation transition and appends no audit.
A retention degradation exits `4` with exactly
`host generation handoff immutable audit retention degraded\nfailure-class: <CLOSED_RETENTION_DEGRADED_CLASS>\naction: <ACTION_FROM_RETENTION_TABLE>\n`
or
`{"schemaVersion":1,"kind":"host-generation-handoff-error","error":"audit-backup-retention-degraded","failureClass":"<CLOSED_RETENTION_DEGRADED_CLASS>","action":"<ACTION_FROM_RETENTION_TABLE>"}`.
A restoration publication that has no durable outcome yet exits `4` with exactly
`host generation handoff immutable audit restoration settlement pending\nsettlement: restart-settlement-pending\nfailure-class: <CLOSED_PUBLICATION_CLASS>\naction: resubmit-same-restoration-artifact\n`
or
`{"schemaVersion":1,"kind":"host-generation-handoff-error","error":"audit-restoration-publication-pending","settlement":"restart-settlement-pending","failureClass":"<CLOSED_PUBLICATION_CLASS>","action":"resubmit-same-restoration-artifact"}`.
A durable degraded restoration outcome exits `4` with exactly
`host generation handoff immutable audit restoration degraded\nsettlement: repair-required\nfailure-class: <CLOSED_PUBLICATION_CLASS>\naction: repair-restoration-storage-and-resubmit\n`
or
`{"schemaVersion":1,"kind":"host-generation-handoff-error","error":"audit-restoration-publication-degraded","settlement":"repair-required","failureClass":"<CLOSED_PUBLICATION_CLASS>","action":"repair-restoration-storage-and-resubmit"}`.
`CLOSED_PUBLICATION_CLASS` is the total
`hierarchy | write | file-sync | link | reopen | directory-sync |
outcome-publication` domain for both `Pending` and `Degraded`; no pending reason can
serialize without exactly one carried class. `repair-required` is nonterminal: after the named storage repair,
resubmitting the byte-identical artifact resumes the same operation id and restoration
attempt, appends the repair-resume chain, and must converge to the success form above.
For either pending class, byte-identical Admin-authorized resubmission drives settlement;
there is no automatic-settlement wait or separate status trigger.

If the real CLI finishes writing the bounded request but receives EOF, reset, or another
transport close before one complete typed broker response, it cannot infer whether the
durable prefix is absent, pre-only, pending, or complete. It exits `4` and renders exactly:

```text
host generation handoff immutable audit restoration response lost
action: resubmit-same-restoration-artifact
```

JSON is exactly
`{"schemaVersion":1,"kind":"host-generation-handoff-error","error":"audit-restoration-response-lost","action":"resubmit-same-restoration-artifact"}`.
This local transport projection carries no `failureClass` or `settlement`, never invents a
publication class, and directs immediate byte-identical Admin resubmission. A disconnect
before any broker response, including the pre-only crash, has dedicated real-binary
human/JSON goldens and exit-`4` tests. The resubmission, not daemon restart or automatic
settlement, drives recovery.

All human and JSON forms have dedicated goldens and expose no path, member or artifact
bytes, signature, private attempt id, unqualified digest, uid, pid, role, wall-clock value,
errno, or free-form value.

After restoration the operator reruns `--repair-authorized-handoff`. The separate
audit-integrity incident preserves the coordinator and backup artifacts for security
adjudication and never advertises restoration as remediation. The named external procedures
`host-generation-invalid-coordinator-escalation-v1`,
`host-generation-pointer-conflict-escalation-v1`,
`host-generation-audit-restoration-conflict-escalation-v1`, and
`host-generation-audit-integrity-escalation-v1` are owned by the site's security incident
authority. Each accepts only the matching fixed human/JSON error plus an authenticated
forensic acquisition performed outside this CLI, preserves all coordinator/backup bytes,
and authorizes no repair, copy, delete, replace, retry, or force operation.
The site access administrator owns
`host-generation-unprivileged-local-admin-restoration-session-v1` for the root refusal. The
site backup administrator owns
`host-generation-immutable-audit-retention-reconciliation-v1`,
`host-generation-retention-clock-discontinuity-repair-v1` only for repairing the configured
authoritative time source and issuing the selector-free Admin wake,
`host-generation-retention-storage-repair-v1`,
`host-generation-retention-census-repair-v1`,
`host-generation-retention-audit-repair-v1`, and
`host-generation-restoration-storage-repair-v1`, plus the site package administrator owns
`host-generation-restoration-client-broker-contract-repair-v1`. The latter verifies and
reinstalls one matching release-sealed client/broker generation before the operator may
resubmit the artifact; it never edits a request. Each accepts only its matching fixed
error, performs no direct filesystem mutation, and reaches continuity publication, pruning,
or settlement only through the broker's authoritative non-caller evidence validation,
typed broker op, and sealed coordinator capability. Clock overflow uses the
site-security-owned `host-generation-retention-clock-overflow-escalation-v1`.

T589 owns the strict schema plus
`tests/golden/delivery/host-generation-handoff-status-v1.{json,txt}` and the independent
`host-generation-handoff-status-case-ids.txt`, plus the read-independent
`host-generation-handoff-{rollback-members,audit-members,transition-edges}.tsv` fixtures,
the strict
`host-generation-immutable-audit-{backup,restoration,restoration-evidence}-v1.schema.json`
schemas, the
`host-generation-immutable-audit-backup-retention-epoch-v1.schema.json` schema, and matching
read-independent JSON goldens, the
`host-generation-immutable-audit-restored-member-v1.schema.json` schema, restoration
wire success/error schema and snapshot, human/JSON goldens, the two-row restoration-audit
edge fixture, and the independent restoration broker-case registry;
T595 owns the private variant constructors, current-pointer classifier, shared renderer,
focused hermetic tests, and the dedicated Type-10 `host-generation-handoff.nix` VM test that
proves real service failure/restart and rollback effects. T604 later exercises the public
handoff as part of exact-candidate operator acceptance without owning that dedicated test.
The fixture and literal test constant cover every row, every allowed and forbidden edge,
every forbidden inspect input, source/target active/failed partitions, transfer-pending
failure, every independently enumerated rollback-proof member and edge, terminal pointer
selection and replacement, all four exits, terminal idempotence, and raw apply-peer canary
absence in both forms.

Rollback proof expectations are pinned by three checked-in fixtures that are mutually
read-independent from production, the status case file, and their separately authored test
constants. `host-generation-handoff-rollback-members.tsv` contains exactly seven rows:

```text
prior-profile
source-broker-service
source-daemon-service
target-broker-service
target-daemon-service
current-pointer
current-reference
```

`host-generation-handoff-audit-members.tsv` contains exactly 32 rows: one independently
enumerated pre-mutation/outcome pair member for each of the 15 handoff transition edges plus
the distinct out-of-band coordinator pointer-repair audit edge:

```text
source-bootstrap-publish/pre-mutation
source-bootstrap-publish/outcome
target-profile-publish/pre-mutation
target-profile-publish/outcome
target-broker-service-transition/pre-mutation
target-broker-service-transition/outcome
coordinator-transfer-to-target/pre-mutation
coordinator-transfer-to-target/outcome
target-daemon-service-transition/pre-mutation
target-daemon-service-transition/outcome
target-pointer-publish/pre-mutation
target-pointer-publish/outcome
target-reference-publish/pre-mutation
target-reference-publish/outcome
target-pointer-repair/pre-mutation
target-pointer-repair/outcome
target-reference-repair/pre-mutation
target-reference-repair/outcome
coordinator-pointer-repair/pre-mutation
coordinator-pointer-repair/outcome
rollback-target-daemon-service/pre-mutation
rollback-target-daemon-service/outcome
rollback-pointer-restore/pre-mutation
rollback-pointer-restore/outcome
rollback-reference-restore/pre-mutation
rollback-reference-restore/outcome
rollback-profile-publish/pre-mutation
rollback-profile-publish/outcome
rollback-source-broker-service/pre-mutation
rollback-source-broker-service/outcome
rollback-source-daemon-service/pre-mutation
rollback-source-daemon-service/outcome
```

`host-generation-handoff-transition-edges.tsv` contains exactly these 15 tab-separated rows.
Column one is the edge id and column two is its required immediate predecessor; `origin`
exists only for the first edge:

```text
source-bootstrap-publish	origin
target-profile-publish	source-bootstrap-publish
target-broker-service-transition	target-profile-publish
coordinator-transfer-to-target	target-broker-service-transition
target-daemon-service-transition	coordinator-transfer-to-target
target-pointer-publish	target-daemon-service-transition
target-reference-publish	target-pointer-publish
target-pointer-repair	target-reference-publish
target-reference-repair	target-pointer-repair
rollback-target-daemon-service	target-reference-repair
rollback-pointer-restore	rollback-target-daemon-service
rollback-reference-restore	rollback-pointer-restore
rollback-profile-publish	rollback-reference-restore
rollback-source-broker-service	rollback-profile-publish
rollback-source-daemon-service	rollback-source-broker-service
```

The case-id file contains exactly these 156 newline-terminated ids in order:

```text
handoff-status/authorized-pending
handoff-status/apply-claimed-peer-live
handoff-status/mutating-peer-live
handoff-status/recovery-pending-source-active
handoff-status/recovery-pending-source-failed
handoff-status/recovery-pending-target-active
handoff-status/recovery-pending-target-failed
handoff-status/transfer-pending-source-active
handoff-status/transfer-pending-source-failed
handoff-status/rollback-source-active
handoff-status/rollback-source-failed
handoff-status/rollback-target-active
handoff-status/rollback-target-failed
handoff-status/completed-terminal
handoff-status/rolled-back-terminal
handoff-status/tuple-phase-nullability
handoff-status/tuple-phase-edge
handoff-status/tuple-owner
handoff-status/tuple-action
handoff-status/tuple-successor-missing
handoff-status/tuple-successor-extra
handoff-status/tuple-successor-duplicate
handoff-status/tuple-successor-unsorted
handoff-status/inspect-intent-id
handoff-status/inspect-generation-selector
handoff-status/inspect-path
handoff-status/inspect-authority-token
handoff-status/inspect-root
handoff-status/inspect-extra-positional
handoff-status/repair-intent-id
handoff-status/repair-generation-selector
handoff-status/repair-path
handoff-status/repair-authority-token
handoff-status/repair-root
handoff-status/repair-extra-positional
handoff-status/repair-force
handoff-status/selection-absent-pointer
handoff-status/selection-missing-record
handoff-status/selection-multiple-nonterminal
handoff-status/selection-nonterminal-pointer-mismatch
handoff-status/selection-stale-terminal-sequence
handoff-status/selection-terminal-replaced
handoff-status/rollback-missing-member/prior-profile
handoff-status/rollback-missing-member/source-broker-service
handoff-status/rollback-missing-member/source-daemon-service
handoff-status/rollback-missing-member/target-broker-service
handoff-status/rollback-missing-member/target-daemon-service
handoff-status/rollback-missing-member/current-pointer
handoff-status/rollback-missing-member/current-reference
handoff-status/rollback-mismatch-member/prior-profile
handoff-status/rollback-mismatch-member/source-broker-service
handoff-status/rollback-mismatch-member/source-daemon-service
handoff-status/rollback-mismatch-member/target-broker-service
handoff-status/rollback-mismatch-member/target-daemon-service
handoff-status/rollback-mismatch-member/current-pointer
handoff-status/rollback-mismatch-member/current-reference
handoff-status/rollback-missing-audit/source-bootstrap-publish/pre-mutation
handoff-status/rollback-missing-audit/source-bootstrap-publish/outcome
handoff-status/rollback-missing-audit/target-profile-publish/pre-mutation
handoff-status/rollback-missing-audit/target-profile-publish/outcome
handoff-status/rollback-missing-audit/target-broker-service-transition/pre-mutation
handoff-status/rollback-missing-audit/target-broker-service-transition/outcome
handoff-status/rollback-missing-audit/coordinator-transfer-to-target/pre-mutation
handoff-status/rollback-missing-audit/coordinator-transfer-to-target/outcome
handoff-status/rollback-missing-audit/target-daemon-service-transition/pre-mutation
handoff-status/rollback-missing-audit/target-daemon-service-transition/outcome
handoff-status/rollback-missing-audit/target-pointer-publish/pre-mutation
handoff-status/rollback-missing-audit/target-pointer-publish/outcome
handoff-status/rollback-missing-audit/target-reference-publish/pre-mutation
handoff-status/rollback-missing-audit/target-reference-publish/outcome
handoff-status/rollback-missing-audit/target-pointer-repair/pre-mutation
handoff-status/rollback-missing-audit/target-pointer-repair/outcome
handoff-status/rollback-missing-audit/target-reference-repair/pre-mutation
handoff-status/rollback-missing-audit/target-reference-repair/outcome
handoff-status/rollback-missing-audit/coordinator-pointer-repair/pre-mutation
handoff-status/rollback-missing-audit/coordinator-pointer-repair/outcome
handoff-status/rollback-missing-audit/rollback-target-daemon-service/pre-mutation
handoff-status/rollback-missing-audit/rollback-target-daemon-service/outcome
handoff-status/rollback-missing-audit/rollback-pointer-restore/pre-mutation
handoff-status/rollback-missing-audit/rollback-pointer-restore/outcome
handoff-status/rollback-missing-audit/rollback-reference-restore/pre-mutation
handoff-status/rollback-missing-audit/rollback-reference-restore/outcome
handoff-status/rollback-missing-audit/rollback-profile-publish/pre-mutation
handoff-status/rollback-missing-audit/rollback-profile-publish/outcome
handoff-status/rollback-missing-audit/rollback-source-broker-service/pre-mutation
handoff-status/rollback-missing-audit/rollback-source-broker-service/outcome
handoff-status/rollback-missing-audit/rollback-source-daemon-service/pre-mutation
handoff-status/rollback-missing-audit/rollback-source-daemon-service/outcome
handoff-status/rollback-mismatch-audit/source-bootstrap-publish/pre-mutation
handoff-status/rollback-mismatch-audit/source-bootstrap-publish/outcome
handoff-status/rollback-mismatch-audit/target-profile-publish/pre-mutation
handoff-status/rollback-mismatch-audit/target-profile-publish/outcome
handoff-status/rollback-mismatch-audit/target-broker-service-transition/pre-mutation
handoff-status/rollback-mismatch-audit/target-broker-service-transition/outcome
handoff-status/rollback-mismatch-audit/coordinator-transfer-to-target/pre-mutation
handoff-status/rollback-mismatch-audit/coordinator-transfer-to-target/outcome
handoff-status/rollback-mismatch-audit/target-daemon-service-transition/pre-mutation
handoff-status/rollback-mismatch-audit/target-daemon-service-transition/outcome
handoff-status/rollback-mismatch-audit/target-pointer-publish/pre-mutation
handoff-status/rollback-mismatch-audit/target-pointer-publish/outcome
handoff-status/rollback-mismatch-audit/target-reference-publish/pre-mutation
handoff-status/rollback-mismatch-audit/target-reference-publish/outcome
handoff-status/rollback-mismatch-audit/target-pointer-repair/pre-mutation
handoff-status/rollback-mismatch-audit/target-pointer-repair/outcome
handoff-status/rollback-mismatch-audit/target-reference-repair/pre-mutation
handoff-status/rollback-mismatch-audit/target-reference-repair/outcome
handoff-status/rollback-mismatch-audit/coordinator-pointer-repair/pre-mutation
handoff-status/rollback-mismatch-audit/coordinator-pointer-repair/outcome
handoff-status/rollback-mismatch-audit/rollback-target-daemon-service/pre-mutation
handoff-status/rollback-mismatch-audit/rollback-target-daemon-service/outcome
handoff-status/rollback-mismatch-audit/rollback-pointer-restore/pre-mutation
handoff-status/rollback-mismatch-audit/rollback-pointer-restore/outcome
handoff-status/rollback-mismatch-audit/rollback-reference-restore/pre-mutation
handoff-status/rollback-mismatch-audit/rollback-reference-restore/outcome
handoff-status/rollback-mismatch-audit/rollback-profile-publish/pre-mutation
handoff-status/rollback-mismatch-audit/rollback-profile-publish/outcome
handoff-status/rollback-mismatch-audit/rollback-source-broker-service/pre-mutation
handoff-status/rollback-mismatch-audit/rollback-source-broker-service/outcome
handoff-status/rollback-mismatch-audit/rollback-source-daemon-service/pre-mutation
handoff-status/rollback-mismatch-audit/rollback-source-daemon-service/outcome
handoff-status/rollback-transition-mismatch/source-bootstrap-publish
handoff-status/rollback-transition-mismatch/target-profile-publish
handoff-status/rollback-transition-mismatch/target-broker-service-transition
handoff-status/rollback-transition-mismatch/coordinator-transfer-to-target
handoff-status/rollback-transition-mismatch/target-daemon-service-transition
handoff-status/rollback-transition-mismatch/target-pointer-publish
handoff-status/rollback-transition-mismatch/target-reference-publish
handoff-status/rollback-transition-mismatch/target-pointer-repair
handoff-status/rollback-transition-mismatch/target-reference-repair
handoff-status/rollback-transition-mismatch/rollback-target-daemon-service
handoff-status/rollback-transition-mismatch/rollback-pointer-restore
handoff-status/rollback-transition-mismatch/rollback-reference-restore
handoff-status/rollback-transition-mismatch/rollback-profile-publish
handoff-status/rollback-transition-mismatch/rollback-source-broker-service
handoff-status/rollback-transition-mismatch/rollback-source-daemon-service
handoff-status/rollback-duplicate-audit
handoff-status/rollback-reordered-audit
handoff-status/rollback-noncontiguous
handoff-status/rollback-unaudited-extra-mutation
handoff-status/selection-pointer-unauthenticated
handoff-status/inspect-repairable-absence
handoff-status/repair-clean-absence-not-found
handoff-status/repair-pointer-positive
handoff-status/repair-pointer-conflict
handoff-status/repair-crash-after-pre-audit
handoff-status/repair-crash-after-inode-fsync
handoff-status/repair-crash-after-final-link
handoff-status/repair-crash-after-parent-fsync
handoff-status/repair-crash-after-outcome-audit
handoff-status/repair-second-run-no-write
handoff-status/repair-audit-restoration-required
handoff-status/repair-audit-integrity-incident
handoff-status/meta-production-member-removed
handoff-status/meta-fixture-member-removed
handoff-status/meta-transition-edge-removed
handoff-status/meta-poison-visitor-removed
```

A separately authored literal 156-id test constant equals the file before any case runs.
Production variant and pointer registries, the fixture, the human/JSON goldens, and the
literal constant are mutually read-independent. Each of the seven rollback members and each
of the 32 audit members has independent missing and mismatch poison. Each transition edge
has its own mismatch poison that changes its edge or predecessor cell while every other row
remains exact. The unaudited-extra-mutation and unauthenticated-pointer cases
are independent and require exit `4`, the exact integrity-incident or invalid-coordinator
output, and zero mutation. The repair cases independently prove all seven forbidden inputs,
clean absence, the distinct repairable-absence inspect projection, exact selector-free
repair success, competing/malformed/unauthenticated invalid censuses, foreign-final conflict, every
pre-audit/unnamed-inode/direct-link/parent-sync/outcome crash boundary, second-run no-write
success, bounded audit-restoration diagnostics, and separate unaudited-mutation escalation.
The four meta-negatives remove one production member, fixture
member, transition edge, or poison visit independently; shrinking a shared registry or
expected count cannot false-green. Each poison reaches its named tuple, selection, input, or
rollback-proof check; an early generic decode failure cannot count.

The 156-case handoff-status registry closes projection and pointer-repair behavior only; it
does not count as broker restoration authorization, artifact, backup, retention, or
publication coverage. Those boundaries use three read-independent fixtures.
`tests/golden/delivery/host-generation-immutable-audit-restoration-edges.tsv` contains
exactly:

```text
coordinator-immutable-audit-restoration/pre-mutation	origin
coordinator-immutable-audit-restoration/outcome	coordinator-immutable-audit-restoration/pre-mutation
```

Here `origin` is the start of one already authorized, artifact-validated restoration
attempt, not the handoff transition graph's global origin. The pre record's fixed
`predecessorSha256` still binds the actual immutable audit predecessor.
Production edge code, the fixture, and a separately authored literal two-row constant are
mutually read-independent. Missing, duplicate, reordered, unknown, or predecessor-substituted
rows fail before a restoration case may count.

`tests/golden/delivery/host-generation-immutable-audit-prune-edges.tsv` contains exactly:

```text
coordinator-immutable-audit-backup-prune/pre-mutation	origin
coordinator-immutable-audit-backup-prune/outcome	coordinator-immutable-audit-backup-prune/pre-mutation
```

Here `origin` is one sealed-capability, epoch-validated prune attempt. Production prune-edge
code, this fixture, and a separately authored literal two-row constant are mutually
read-independent. Missing, duplicate, reordered, unknown, or predecessor-substituted rows
fail before any retention case may count.

`tests/golden/delivery/host-generation-immutable-audit-restoration-broker-case-ids.txt`
contains exactly these 168 newline-terminated ids in order:

```text
restoration/authorization/local-launcher-denied
restoration/authorization/local-workload-denied
restoration/authorization/local-zone-denied
restoration/authorization/local-host-shutdown-denied
restoration/authorization/local-root-denied
restoration/authorization/local-nonmember-denied
restoration/authorization/local-unauthenticated-denied
restoration/authorization/direct-broker-denied
restoration/authorization/remote-denied
restoration/request/missing-schema-version-denied
restoration/request/wrong-schema-version-denied
restoration/request/missing-kind-denied
restoration/request/wrong-kind-denied
restoration/request/unknown-field-denied
restoration/request/missing-operation-id-denied
restoration/request/operation-id-length-denied
restoration/request/operation-id-digest-mismatch-denied
restoration/request/missing-artifact-bytes-denied
restoration/request/empty-artifact-bytes-denied
restoration/request/over-limit-artifact-bytes-denied
restoration/request/path-field-denied
restoration/request/selector-field-denied
restoration/request/uid-field-denied
restoration/request/pid-field-denied
restoration/request/authority-token-field-denied
restoration/request/member-override-field-denied
restoration/request/failure-override-field-denied
restoration/request/free-form-field-denied
restoration/artifact/noncanonical-denied
restoration/artifact/over-limit-denied
restoration/artifact/signature-denied
restoration/artifact/domain-denied
restoration/artifact/key-denied
restoration/artifact/authority-denied
restoration/artifact/member-binding-denied
restoration/artifact/failure-class-binding-denied
restoration/artifact/predecessor-binding-denied
restoration/artifact/backup-binding-denied
restoration/artifact/observed-member-binding-denied
restoration/class/missing-append
restoration/class/mismatch-append-only-supersession
restoration/class/unauthenticated-append-only-supersession
restoration/class/noncontiguous-append-only-supersession
restoration/conflict/provenance-preserved
restoration/conflict/member-preserved
restoration/conflict/operation-id-different-artifact
restoration/conflict/duplicate-supersession
restoration/backup/absent-before-covered-mutation
restoration/backup/not-durable-before-covered-mutation
restoration/backup/durable-before-covered-mutation
restoration/retention/current-intent-prune-denied
restoration/retention/day-29-prune-denied
restoration/retention/day-30-prune-eligible
restoration/retention/day-90-prune-mandatory
restoration/retention/epoch-restart-stable
restoration/retention/epoch-invalid-degraded
restoration/retention/clock-watermark-degraded
restoration/retention/clock-rollback-degraded
restoration/retention/clock-overflow-degraded
restoration/retention/count-256-accepted
restoration/retention/count-257-refused
restoration/retention/bytes-16777216-accepted
restoration/retention/bytes-16777217-refused
restoration/retention/root-intents-64-accepted
restoration/retention/root-intents-65-refused
restoration/retention/root-members-4096-accepted
restoration/retention/root-members-4097-refused
restoration/retention/root-bytes-268435456-accepted
restoration/retention/root-bytes-268435457-refused
restoration/retention/reservation-before-handoff
restoration/retention/reservation-reconstructed-on-restart
restoration/retention/reservation-released-after-durable-prune
restoration/retention/reservation-released-after-immutable-zero-mutation
restoration/retention/sealed-prune-capability-required
restoration/retention/pre-audit-before-unlink
restoration/retention/success-outcome-exactly-once
restoration/retention/unlink-failure-degraded
restoration/retention/parent-sync-failure-degraded
restoration/retention/census-failure-degraded
restoration/retention/audit-publication-failure-pending
restoration/retention/crash-after-pre-before-unlink
restoration/retention/crash-after-unlink-before-parent-sync
restoration/retention/crash-after-parent-sync-before-census
restoration/retention/crash-after-census-before-outcome
restoration/retention/completed-response-loss-no-write
restoration/retention/parent-replacement-denied
restoration/retention/symlink-ancestor-denied
restoration/retention/magiclink-ancestor-denied
restoration/retention/cross-device-denied
restoration/retention/exec-descriptor-leak-denied
restoration/retention/final-reopen-identity-mismatch-denied
restoration/retention/degraded-report-redacted
restoration/publication/backup/hierarchy-after-mkdir-before-sync
restoration/publication/backup/hierarchy-after-sync-before-inode-write
restoration/publication/backup/after-write-before-file-sync
restoration/publication/backup/after-file-sync-before-link
restoration/publication/backup/after-link-before-final-reopen
restoration/publication/backup/after-final-reopen-before-parent-sync
restoration/publication/backup/after-parent-sync-before-ancestor-sync
restoration/publication/backup/after-final-directory-sync
restoration/publication/backup/completed-response-loss-no-write
restoration/publication/evidence/hierarchy-after-mkdir-before-sync
restoration/publication/evidence/hierarchy-after-sync-before-inode-write
restoration/publication/evidence/after-write-before-file-sync
restoration/publication/evidence/after-file-sync-before-link
restoration/publication/evidence/after-link-before-final-reopen
restoration/publication/evidence/after-final-reopen-before-parent-sync
restoration/publication/evidence/after-parent-sync-before-ancestor-sync
restoration/publication/evidence/after-final-directory-sync
restoration/publication/evidence/completed-response-loss-no-write
restoration/publication/pre/hierarchy-after-mkdir-before-sync
restoration/publication/pre/hierarchy-after-sync-before-inode-write
restoration/publication/pre/after-write-before-file-sync
restoration/publication/pre/after-file-sync-before-link
restoration/publication/pre/after-link-before-final-reopen
restoration/publication/pre/after-final-reopen-before-parent-sync
restoration/publication/pre/after-parent-sync-before-ancestor-sync
restoration/publication/pre/after-final-directory-sync
restoration/publication/pre/completed-response-loss-no-write
restoration/publication/provenance/hierarchy-after-mkdir-before-sync
restoration/publication/provenance/hierarchy-after-sync-before-inode-write
restoration/publication/provenance/after-write-before-file-sync
restoration/publication/provenance/after-file-sync-before-link
restoration/publication/provenance/after-link-before-final-reopen
restoration/publication/provenance/after-final-reopen-before-parent-sync
restoration/publication/provenance/after-parent-sync-before-ancestor-sync
restoration/publication/provenance/after-final-directory-sync
restoration/publication/provenance/completed-response-loss-no-write
restoration/publication/outcome/hierarchy-after-mkdir-before-sync
restoration/publication/outcome/hierarchy-after-sync-before-inode-write
restoration/publication/outcome/after-write-before-file-sync
restoration/publication/outcome/after-file-sync-before-link
restoration/publication/outcome/after-link-before-final-reopen
restoration/publication/outcome/after-final-reopen-before-parent-sync
restoration/publication/outcome/after-parent-sync-before-ancestor-sync
restoration/publication/outcome/after-final-directory-sync
restoration/publication/outcome/completed-response-loss-no-write
restoration/publication/prune-pre/hierarchy-after-mkdir-before-sync
restoration/publication/prune-pre/hierarchy-after-sync-before-inode-write
restoration/publication/prune-pre/after-write-before-file-sync
restoration/publication/prune-pre/after-file-sync-before-link
restoration/publication/prune-pre/after-link-before-final-reopen
restoration/publication/prune-pre/after-final-reopen-before-parent-sync
restoration/publication/prune-pre/after-parent-sync-before-ancestor-sync
restoration/publication/prune-pre/after-final-directory-sync
restoration/publication/prune-pre/completed-response-loss-no-write
restoration/publication/prune-outcome/hierarchy-after-mkdir-before-sync
restoration/publication/prune-outcome/hierarchy-after-sync-before-inode-write
restoration/publication/prune-outcome/after-write-before-file-sync
restoration/publication/prune-outcome/after-file-sync-before-link
restoration/publication/prune-outcome/after-link-before-final-reopen
restoration/publication/prune-outcome/after-final-reopen-before-parent-sync
restoration/publication/prune-outcome/after-parent-sync-before-ancestor-sync
restoration/publication/prune-outcome/after-final-directory-sync
restoration/publication/prune-outcome/completed-response-loss-no-write
restoration/audit/missing-pre-refused
restoration/audit/outcome-without-provenance-refused
restoration/audit/mismatched-outcome-fields-refused
restoration/audit/unknown-outcome-refused
restoration/meta/fixture-member-removed
restoration/meta/poison-visitor-removed
restoration/meta/request-shape-member-removed
restoration/meta/publication-record-class-removed
restoration/meta/publication-boundary-removed
restoration/meta/retention-boundary-removed
restoration/meta/prune-pre-record-class-removed
restoration/meta/prune-outcome-record-class-removed
restoration/meta/reservation-lifecycle-member-removed
```

A separately authored literal 168-id constant equals the file before any case runs.
Production authorization, signature verification, publication, backup, pruning, and
restoration registries read neither expectation. The nine authorization cases, nineteen
request-shape cases, and eleven artifact cases assert zero coordinator, audit, backup,
private-evidence, provenance, and member mutation. Conflict cases may append only their
exact pre/outcome pair after authorization and under-lock state revalidation, preserve every
original and conflicting final, and append no restored provenance member. The four
restoration classes independently prove exact predecessor/member/failure binding and
append-only supersession. Backup-order cases prove no covered mutation precedes a
file-and-directory-durable backup.

The existing 42 retention ids and 63 publication ids prove only the cases literally present
in this 168-id fixture. They do not stand in for aggregate-root storage, anchor staging,
continuity repair, internal no-Admin catch-up, the full prune-permit seal, settlement,
repair-resume, ensure-root, reservation, or release coverage.

`tests/golden/delivery/host-generation-immutable-audit-record-boundary-case-ids.txt`
is a separate literal 216-id registry. Every id is exactly
`record-boundary/<CLASS>/<BOUNDARY>` in the class order and then boundary order below:

```text
CLASS =
dispatch
pointer-repair-pre
pointer-repair-outcome
ensure-root-pre
ensure-root-outcome
reservation-pre
reservation-outcome
release-durable-prune-pre
release-durable-prune-outcome
release-zero-mutation-pre
release-zero-mutation-outcome
retention-anchor-pre
retention-anchor-candidate
retention-anchor-outcome
continuity-repair-pre
continuity-repair-evidence
continuity-repair-watermark
continuity-repair-outcome
provenance
restoration-outcome
settlement
repair-resume
prune-pre
prune-outcome

BOUNDARY =
hierarchy-after-mkdir-before-sync
hierarchy-after-sync-before-inode-write
after-write-before-file-sync
after-file-sync-before-link
after-link-before-final-reopen
after-final-reopen-before-parent-sync
after-parent-sync-before-ancestor-sync
after-final-directory-sync
completed-response-loss-no-write
```

The fixture and separately authored literal 216-id constant spell all rows; neither may
generate this product at runtime or read production classes, hooks, registry length, or
another expectation. Production owns a third independently authored visitor. One
family-specific shrinkage poison for each of the 24 classes and each of the nine boundaries
must fail before a case runs. Every case reaches its named class and fault hook, so one
record's test cannot satisfy another, and settlement or repair-resume cannot borrow
restoration-outcome coverage. The unchanged 168-id registry remains the independent owner
of the legacy backup, restoration-private-evidence, and restoration-pre class/boundary rows
omitted here; replacing those three duplicate supplemental classes with continuity-repair
pre/evidence/watermark/outcome produces exactly 24 by nine = 216 and makes the immutable
watermark's partial-prefix reconciliation independently mandatory. Neither registry may
borrow a visitor, expected set, or case result from the other.

`tests/golden/delivery/host-generation-immutable-audit-lifecycle-case-ids.txt`
contains exactly these 88 newline-terminated ids in order:

```text
lifecycle/aggregate/root-records-32768-accepted
lifecycle/aggregate/root-records-32769-refused
lifecycle/aggregate/root-bytes-536870912-accepted
lifecycle/aggregate/root-bytes-536870913-refused
lifecycle/aggregate/restoration-records-8-accepted
lifecycle/aggregate/restoration-records-9-refused
lifecycle/aggregate/restoration-bytes-1048576-accepted
lifecycle/aggregate/restoration-bytes-1048577-refused
lifecycle/aggregate/restoration-attempts-256-accepted
lifecycle/aggregate/restoration-attempts-257-refused
lifecycle/aggregate/continuity-evidence-records-1-accepted
lifecycle/aggregate/continuity-evidence-records-2-refused
lifecycle/aggregate/continuity-evidence-bytes-131072-accepted
lifecycle/aggregate/continuity-evidence-bytes-131073-refused
lifecycle/aggregate/continuity-repair-attempts-256-accepted
lifecycle/aggregate/continuity-repair-attempts-257-refused
lifecycle/aggregate/pending-staging-records-8192-accepted
lifecycle/aggregate/pending-staging-records-8193-refused
lifecycle/aggregate/pending-staging-bytes-67108864-accepted
lifecycle/aggregate/pending-staging-bytes-67108865-refused
lifecycle/standing-reserve/created-and-charged
lifecycle/standing-reserve/nonrecursive-pre-outcome
lifecycle/standing-reserve/exhausted-blocks-before-mutation
lifecycle/standing-reserve/export-replenishes-after-durability
lifecycle/standing-reserve/missing-degraded
lifecycle/standing-reserve/overdrawn-degraded
lifecycle/standing-reserve/duplicated-degraded
lifecycle/standing-reserve/unaccounted-degraded
lifecycle/capacity/reservation/successful-prefix-matrix
lifecycle/capacity/reservation/malformed-prefix-degraded
lifecycle/capacity/reservation/completed-response-loss-no-write
lifecycle/capacity/reservation/equal-charge-independent-governing-operations
lifecycle/capacity/reservation/refusal-prefix-crash-matrix-zero-mutation
lifecycle/capacity/reservation/refusal-completed-zero-mutation
lifecycle/capacity/reservation/refusal-response-loss-no-write
lifecycle/capacity/reservation/retry-after-capacity-change-new-generation
lifecycle/capacity/release-durable-prune/prefix-crash-matrix
lifecycle/capacity/release-durable-prune/malformed-prefix-degraded
lifecycle/capacity/release-durable-prune/completed-response-loss-no-write
lifecycle/capacity/release-zero-mutation/prefix-crash-matrix
lifecycle/capacity/release-zero-mutation/malformed-prefix-degraded
lifecycle/capacity/release-zero-mutation/release-then-same-intent-retry-refusal-response-loss
lifecycle/retention-anchor/pre-and-candidate-prefix-no-resample
lifecycle/retention-anchor/outcome-response-loss-no-resample
lifecycle/retention-anchor/nonidentical-candidate-conflict
lifecycle/retention-anchor/day-29-refused
lifecycle/retention-anchor/day-30-eligible
lifecycle/retention-anchor/day-90-mandatory
lifecycle/retention-anchor/discontinuity-does-not-reset-epoch
lifecycle/continuity/admin-wake-caller-input-denied
lifecycle/continuity/invalid-broker-evidence-refused
lifecycle/continuity/authoritative-evidence-sealed-permit
lifecycle/continuity/pre-and-evidence-prefix-replay
lifecycle/continuity/malformed-prefix-degraded
lifecycle/continuity/post-day-90-prune-before-watermark
lifecycle/continuity/watermark-applied-before-outcome-replay
lifecycle/continuity/completed-response-loss-no-write
lifecycle/continuity/evidence-retention-export-compaction-restart
lifecycle/continuity/reboot-preserves-original-day-90
lifecycle/continuity/startup-no-admin-catch-up
lifecycle/continuity/idle-no-admin-catch-up
lifecycle/continuity-permit/construction-field-accessor-denied
lifecycle/continuity-permit/clone-copy-default-denied
lifecycle/continuity-permit/conversion-serde-reconstruction-denied
lifecycle/continuity-permit/cross-coordinator-lifetime-escape-denied
lifecycle/continuity-permit/second-dispatch-denied
lifecycle/prune-permit/construction-field-accessor-denied
lifecycle/prune-permit/clone-copy-default-denied
lifecycle/prune-permit/conversion-serde-denied
lifecycle/prune-permit/byte-digest-fd-reconstruction-denied
lifecycle/prune-permit/cross-coordinator-lifetime-escape-denied
lifecycle/prune-permit/second-dispatch-denied
lifecycle/settlement/pre-only-no-caller-free-recovery
lifecycle/settlement/pre-only-byte-identical-admin-resubmission
lifecycle/settlement/pre-only-different-artifact-conflict
lifecycle/settlement/transport-response-loss-resubmit-no-class
lifecycle/settlement/pending-resubmission-converges
lifecycle/settlement/degraded-repair-resume-converges
lifecycle/settlement/completed-response-loss-no-write
lifecycle/redaction/all-private-publication-identifiers-observable-surface-canaries
lifecycle/meta/aggregate-member-removed
lifecycle/meta/capacity-prefix-member-removed
lifecycle/meta/retention-anchor-member-removed
lifecycle/meta/continuity-member-removed
lifecycle/meta/prune-permit-negative-member-removed
lifecycle/meta/settlement-member-removed
lifecycle/meta/boundary-visitor-removed
lifecycle/meta/shrinkage-poison-removed
```

A separately authored literal 88-id constant, the fixture, and production visitors are
mutually read-independent. Each case must reach its named limit, prefix, failure injection,
permit negative, continuation source, or settlement branch. A `prefix-matrix` case has
independently named hook subcases for every prefix and one removal poison per hook; a grouped
permit case likewise owns separate compile-fail/API assertions and poisons for every token
named in its id. Each release malformed-prefix id independently hooks outcome-without-pre,
ledger-without-pre, completion-without-ledger, duplicate pre/outcome, wrong generation,
wrong prior ledger, wrong reason, wrong proof, and cross-release proof substitution. The
continuity malformed-prefix id independently hooks evidence-without-pre,
watermark-before-evidence, watermark-before-required-prune, before-day-90 unrelated prune,
day-90 success without prune, duplicate pre/evidence/watermark/outcome, source replay
change, replay-handle/sequence/predecessor mismatch, and every deadline/outcome cross-pair.
Every malformed hook has its own removal poison. The evidence-retention case independently
hooks pre-export retention, file-only export refusal, directory-durable export admission,
unlink, parent sync, census commit, restart at every compaction prefix, and completed
no-write replay, again with one removal poison per hook. The redaction case visits every
private identifier, every continuity body member, and every observable surface
independently, with one canary-removal poison per identifier/member/surface. Each meta case
deletes exactly one member or visit from one family while all other bytes remain valid. Unknown,
duplicate, reordered, skipped, dynamically generated, early-generic-refusal, or
production-derived expectations fail before any case counts. Count reconciliation is
therefore exact: the original broker registry remains 168 ids unchanged, while the
supplemental registries add 216 durable-record/boundary ids and 88 lifecycle ids; the 20-id
ensure-root fixture and 156-id status fixture retain their existing independent purposes.

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
