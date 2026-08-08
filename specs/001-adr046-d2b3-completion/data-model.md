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
MUST compare both artifacts byte-for-byte. T589 implements only rows assigned to T589; T604
emits only the acceptance evidence assigned to T604; T600 imports the exact candidate result;
and T220 verifies that every generated row is complete. No feature-local field list, count,
digest recipe, state table, or recovery matrix may substitute for those generated rows.

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
continuity-repair pre/evidence/watermark/intent/outcome records, continuity-compaction
pre-mutation/outcome records, immutable backup members, restoration private evidence,
pre-mutation, provenance, outcome, settlement and repair-resume records, plus backup-prune
pre-mutation and outcome records.

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
`PublishHostGenerationImmutableAuditContinuityReplayKeyV1`,
`RecycleHostGenerationImmutableAuditContinuityReplayKeyCandidateV1`,
`ReserveHostGenerationImmutableAuditCapacityV1`,
`BindHostGenerationImmutableAuditRetentionAnchorV1`,
`BindHostGenerationImmutableAuditContinuitySourceEvidenceV1`,
`ReconcileHostGenerationImmutableAuditContinuitySourcePrefixV1`,
`RepairHostGenerationImmutableAuditContinuityV1`,
`CompactHostGenerationImmutableAuditContinuityV1`,
`ReleaseHostGenerationImmutableAuditContinuityAttemptCapacityV1`,
`PublishHostGenerationImmutableAuditBackupV1`,
`RestoreHostGenerationImmutableAuditMemberV1`,
`PruneHostGenerationImmutableAuditBackupsV1`, and the already typed dispatch and
coordinator-pointer-repair operations. Each appends fixed digest/enum pre-mutation audit
before its first durable private mutation and one matching fixed outcome afterward. The
candidate recycler is an ensure-root subvisitor, source-prefix reconciliation is a
continuity-repair-pre subvisitor, and attempt-slice release is a compaction subvisitor.
They add no durable-record/boundary or lifecycle registry id; the exact 216 and 88 ids and
every existing id spelling remain unchanged. The
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
| continuity-repair body evidence and broker-to-source replay bindings | exactly 1 evidence record, 1 binding record, acquisition pre/outcome, association pre, and binding outcome per attempt, at most 141,312 encoded bytes together per attempt, and 256 simultaneously retained attempt slots per intent | included in 32,768 records and 536,870,912 bytes | retained through exact repair replay; a settled degraded attempt may use the safe reclamation mode below before governed-set absence, while final repaired-set compaction still requires governed-set absence and the outcome-specific export predicate |
| retention anchors, immutable watermarks, reservations, replay-key identity/candidate lifecycle, settlement prefixes, decision-intent and target-unlink commit witnesses, compaction metadata, and prune census state | 1 current anchor, 1 current replay key, at most 1 candidate commitment, 1 recycling prefix and 2 candidate-head banks within a fixed 12-record/98,304-byte reserve, at most 256 immutable watermarks, 1 reservation ledger, 256 retained repair/settlement/compaction prefixes, exactly 1 decision-intent witness and at most 4 target-unlink witnesses per retained attempt, and 256 per-member reduced-census records per intent | included in 32,768 records and 536,870,912 bytes | the current replay key and current watermark are not compactable; an absent-final candidate must finish sealed bounded recycling before resampling; final replaced-set cleanup requires governed-set absence, while attempt-local degraded reclamation may remove only its selected leaves after exact export and current-head proof; witness removal is ordered, successor-proved, parent-synced, and complete before source release or slot reuse |
| pending fixed-field audit staging for root operations | 8,192 records and 67,108,864 bytes total | included in 32,768 records and 536,870,912 bytes | append-only export to the existing immutable broker audit segment owner; staging removal only after segment file and directory durability and configured audit retention |

The root-wide record and byte ceilings include every class rather than counting the backup
subset alone. A reservation failure precedes mutation and is typed degradation. Root
operation audit rotates only through the existing append-only broker audit segment owner,
whose record/byte segment bounds and retention are enforced before old durable segments are
removed. No record is overwritten, truncated, or silently dropped to regain capacity.
Every accepted replacement reserves the complete worst-case retained prefix for all 256
admitted continuity-repair attempts, rather than one future attempt. This includes every
broker-to-source binding and source-prefix reconciliation pair, evidence record, optional
immutable watermark, repair and settlement decision-basis intent/basis/selection/record,
one decision-basis-intent commit witness, compaction selection/recovery/target
intent/receipt, up to four target-unlink commit witnesses, targets-compacted,
source-release, and attempt-slice release record, failed or successful prune
pre/outcome pair, and
per-member reduced-census record in `rootRecordDelta`/`rootEncodedByteDelta`; mandatory
continuity repair therefore does not depend on finding new general capacity at day 90.
The reservation ledger separately enforces the one-evidence-record/131,072-byte per-attempt
ceiling, one source-binding record per attempt, 256 simultaneously retained attempt slots
per intent, 256 simultaneously retained first-time member-absence proofs, and 512
simultaneously retained member-prune attempts. These are live-retention ceilings, not
lifetime attempt counters. A slot is reusable only after broker compaction, source release,
and the matching attempt-slice release outcome are all complete; reuse increments the
monotonic repair sequence and cannot reproduce an earlier private or audit identity. A checked subset
or aggregate conversion failure occurs before capacity pre-audit.

`ContinuityReplacementFutureChargeV1` is the fixed conservative charge added to every
accepted replacement. It is independent of the number of members actually present. The
first block reserves every retained per-attempt prefix for 256 attempts. The second block
reserves up to 256 degraded member-prune attempts plus up to 256 first-time
`Pruned | AlreadyPruned` absence proofs across those attempts; an already proven absence is
reused and may not append another `AlreadyPruned` record. A repair attempt that emits a
degraded member-prune outcome settles before another member is attempted. These bounds make
512 the exact maximum number of member-prune pre/outcome pairs and 256 the exact maximum
number of reduced-census records:

| Reserved future class | Multiplier | Records | Encoded bytes |
| --- | ---: | ---: | ---: |
| one broker-to-source replay-binding record, acquisition pre/outcome, association pre, binding outcome audit, and one recyclable source-prefix reconciliation pre/outcome pair | 256 | 1,792 | 3,670,016 |
| one maximum continuity-evidence body | 256 | 256 | 33,554,432 |
| one maximum immutable continuity watermark | 256 | 256 | 262,144 |
| continuity-repair fixed pre/outcome audit | 256 | 512 | 1,048,576 |
| settlement decision-basis intent, decision basis, decision selection, decision-pre, and exact-outcome intent | 256 | 1,280 | 2,621,440 |
| one decision-basis-intent commit witness and four target-unlink commit witnesses, each fixed at 2,048 encoded bytes | 256 | 1,280 | 2,621,440 |
| compaction pre/outcome audit, attempt-census reduction, four target intents, four target receipts, one recyclable recovery pre/outcome pair, one targets-compacted receipt, source-release pre/outcome, and attempt-slice release pre/ledger/outcome | 256 | 4,864 | 9,961,472 |
| compaction target-set, current-head, successor, and final-absence metadata | 256 | 1,024 | 2,097,152 |
| whole-set mandatory-prune target and proof metadata | 256 | 512 | 1,048,576 |
| member-prune pre/outcome audit pair | 512 | 1,024 | 2,097,152 |
| per-member reduced-census record | 256 | 256 | 524,288 |
| **Exact future charge** | - | **13,056** | **59,506,688** |

The reservation adds exactly 13,056 to `rootRecordDelta` and 59,506,688 to
`rootEncodedByteDelta`, using checked arithmetic, in addition to the replacement's present
backup and anchor charge. Later continuity repair debits this reservation and never repeats
general-capacity admission. Unrelated publication cannot consume reserved capacity.
Each admitted attempt receives a fixed 46-record/222,208-byte slice: 41 records and
211,968 bytes for the previously listed fixed rows plus one 2,048-byte decision-intent
witness and four 2,048-byte target-unlink witness slots. An attempt that uses fewer than
four unlink witnesses leaves the unused witness slots reserved rather than transferring
them to general capacity. Slice release returns the fixed 46/222,208 reservation only
after every witness that did become durable has completed the successor-proved,
parent-synced reclamation below; deleting a file does not itself decrement the reservation
ledger.
Target cardinality cannot vary this charge. Four independent read-literal general-capacity
probes use target cardinality `0`, `1`, `2`, and `3`. For each cardinality, separate
record-bound and byte-bound fixtures fill unrelated general capacity to the independently
calculated remaining boundary, accept that exact boundary, and refuse respectively one
record or one byte beyond it. Each starts a fresh process at attempt admission, after
targets compacted, after every applicable witness-reclamation prefix, after source
`Released`, and immediately before and after attempt-slice `CompletedReleased`. Every
pre-release checkpoint must still observe the complete 46-record/222,208-byte slice as
charged, including every unused target-witness slot. Only the exact attempt-slice
`CompletedReleased` transition returns all 46 records and 222,208 bytes atomically to
general capacity, and completed-response replay returns no additional capacity; no smaller
cardinality, witness absence, witness unlink, or source release returns a partial charge.
Each cardinality/boundary/restart assertion has its own hook and removal poison and does
not reuse the maximum four-target reclamation case.
The read-independent expected fixture literally enumerates the `0`, `1`, `2`, and `3`
target rows for both the record and byte bounds. For cardinality `c`, it independently
visits all `1 + c` reclamation names at before-unlink, after-unlink-before-parent-sync,
and after-parent-sync prefixes, then source `Released`, attempt-slice
`CompletedReleased`, and completed replay. No row is generated from the maximum-target
case or production cardinality. Every row observes the full 46-record/222,208-byte charge
until `CompletedReleased`, exactly one atomic 46-record/222,208-byte credit at that
transition, and no second credit at replay.
The one recovery pre/outcome pair is recyclable only after its prior outcome and every
completed-target intent and receipt are file-and-directory-durably exported and the
residual selection is durably superseded. Each missing prerequisite is a no-mutation
barrier: it may not unlink a recovery prefix, publish a new generation, or alter the
attempt census. Only the state with all three prerequisites complete may recycle the
prefix, and no more than one is live per attempt. Source-private pin/replay files are
charged by the source's separate 256-live
pair ceiling, while their broker audit and coordination records are included in this row.
Read-independent tests calculate every row literally rather than importing production
constants. They admit an exact record and byte boundary, refuse one record short and one
byte short, exhaust all unreserved root capacity after replacement, retain 255 independently
settled degraded prefixes with their failed member-prune records, and then complete the
256th repair, whole-set prune, settlement, and compaction solely from the reservation.
Separate exact-capacity fresh-process cases cover repeated success, degraded settlement,
partial prune, restart, later repair, final prune, settlement, compaction, audited source
release, witness reclamation, and slot reuse. One omission and one
multiplier/count-change poison for each of the eleven rows must fail before the lifecycle
case runs.

Attempt exhaustion is therefore a cleanup backlog, never a terminal epoch state. A settled
degraded attempt with no accepted watermark becomes eligible for
`CompactHostGenerationImmutableAuditContinuityV1::ReclaimDegradedAttempt` as soon as its
attempt-local binding, evidence state, repair, settlement, outcome, and any prune-history
records are file-and-directory-durably exported. This mode never unlinks a backup member,
changes the governed census, publishes a final-absence proof, advances a watermark, or
removes an epoch-level prune-history commitment. It compacts only that attempt's broker
evidence, replay binding, settlement prefix, and attempt census, then drives the audited
source release defined below. A degraded day-90 attempt with partial prune history first
folds each already exported history entry into the durable epoch history census before any
attempt-local record becomes a target. Repaired attempts and final replaced-set cleanup use
`FinalizeReplacedSet` and retain the governed-set-absence requirement.

Startup and the existing in-process idle wake always resume the oldest incomplete
broker-target compaction, source release, or attempt-slice release before admitting another
repair. At 256 retained slots, repair
admission first drives that ordered cleanup queue; after one complete release it admits the
next repair with a new sequence in the reclaimed reserved slice. If cleanup is still
failing, the request returns the exact owning stage and failure class - broker cleanup,
source lifecycle, or attempt-slice ledger - and performs no new source acquisition. A
generic `continuity-repair-attempt-limit` response is never substituted for that blocking
failure. `continuity-repair-attempt-limit` is only the private capacity-classifier trigger
for entering this cleanup path, not a public response variant; its paired internal
continuation label `resume-oldest-continuity-cleanup` is likewise never emitted as a public
action. An immutable degraded compaction outcome is not retried under the same
identity: after the named storage, audit, source, or head conflict is repaired, a
`ContinuityCompactionRecoveryGenerationV1` cites the prior attempt and outcome, every
completed target receipt, a freshly selected residual target set, and a fresh current-head
proof. It may continue only the residual targets and cannot reinterpret an earlier absence.
At most one recovery generation is active for an attempt.

The bounded-convergence fixture settles 256 degraded repairs while governed members remain,
crashes after every broker target unlink/census boundary and every source release
unlink/sync boundary, restarts a fresh broker, repairs each injected failure, and requires
256 ordered reclamations, 256 audited source releases, 256 attempt-slice releases, at least
one reclaimed-slot repair,
the eventual whole-set prune, final repaired settlement, and final compaction. The done
condition is a zero-length cleanup queue, zero source pins or replay bindings for compacted
attempts, at most one current replay key and watermark, all 256 slots reusable, and an exact
completed replay that performs zero writes. Any queue skip, slot reuse before source
release and attempt-slice release, second active recovery generation, changed attempt
sequence, or retained
source-private record fails with a dedicated hook and removal poison.

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
contract has one closed, crash-stable broker-to-source binding protocol, entered only after
the replay key is complete and that attempt's reserved-capacity slice is debited:

1. While holding the source's exclusive lifecycle lock, the broker performs the
   side-effect-free `admit_pin(ContinuitySourceBindingAttemptIdV1)` census check. The source
   admits at most 256 live acquisition/replay pairs for the intent and mutates no source
   state during this check.
2. The broker publishes the fixed source-acquisition pre-audit using only the audit
   projections defined below. Only after that record is durable may
   `pin_acquire(ContinuitySourceBindingAttemptIdV1)` create or exactly reopen one
   source-private no-replace acquisition record and return its one canonical evidence
   value. The pin file and containing directory are durable before the matching closed
   `Pinned | AlreadyPinned | Degraded` acquisition outcome is published.
3. `resume_pinned(ContinuitySourceBindingAttemptIdV1)` returns only that byte-identical
   pinned acquisition and cannot select current or newer evidence.
4. After the pin outcome, the broker derives the replay handle and publishes a second fixed
   replay-association pre-audit. Only then may
   `bind_replay(ContinuityEvidenceReplayBindingV1)` durably associate the broker-supplied
   opaque `ContinuityEvidenceReplayHandleV1` with that pinned acquisition, exact
   `authoritativeEvidenceSha256`, source authority/version, and binding attempt. It returns
   `ContinuityEvidenceReplayBindingReceiptV1` only after the binding file and directory are
   durable.
5. `replay(ContinuityEvidenceReplayHandleV1)` returns only the evidence named by that
   durable association.
6. `release_pinned(ContinuitySourceReleasePermitV1)` is the only removal API. It is admitted
   only after a matching durable `TargetsCompacted` receipt proves broker replay targets
   unnecessary and the required exports durable. It removes the replay association, syncs
   and records that absence, removes the pin, syncs and records that absence, and publishes
   a closed release outcome. Exact completed release is no-write replay.

The source cannot derive the HMAC handle and cannot accept a handle without the exact
source-private pinned acquisition. It may not implement "latest", caller-selected,
timestamp-selected, generic enumeration, fallback lookup, handle replacement, or rebinding.
It may not expose enumerate, unlink, rename, truncate, force-release, or generic filesystem
methods. The broker cannot call `release_pinned` with an incomplete, degraded, foreign, or
digest-only reconstructed compaction result.
The binding attempt is broker-derived before acquisition from the coordinator-private id,
disposition authority, retention epoch, prior watermark, and repair sequence, so restart
can resume a pin without knowing or selecting evidence bytes:

```text
ContinuitySourceBindingAttemptIdV1 =
  SHA-256("d2b:host-generation:audit-continuity-source-binding-attempt:v1\0" ||
  coordinator_private_id || disposition_authority_sha256 ||
  retention_epoch_sha256 || prior_watermark_sha256 ||
  continuity_repair_sequence_u64_be)

continuitySourceBindingAttemptSha256 =
  SHA-256("d2b:audit:host-generation:continuity-source-binding-attempt:v1\0" ||
  private_continuity_source_binding_attempt_id)

continuitySourceAuthorityAuditSha256 =
  SHA-256("d2b:audit:host-generation:continuity-source-authority:v1\0" ||
  source_authority_sha256)

continuitySourceBindingReceiptSha256 =
  SHA-256("d2b:audit:host-generation:continuity-source-binding-receipt:v1\0" ||
  continuity_source_binding_attempt_id ||
  continuity_evidence_replay_handle_sha256 ||
  authoritative_evidence_sha256 || source_authority_sha256 ||
  source_version_u32_be || source_durability_generation_u64_be)
```

Every digest or id member is 32 bytes, the sequence and durability generation are
eight-byte big-endian integers, and the source version is four-byte big-endian. The
source-acquisition pre contains only the fixed edge id,
`continuitySourceBindingAttemptSha256`, `continuitySourceAuthorityAuditSha256`, source
version, and reserved slot generation. It contains no raw attempt, private preimage, source
record, handle, receipt, evidence bytes, or ordinary source-authority digest. The
replay-association pre adds only `continuityEvidenceReplayHandleSha256`,
`authoritativeEvidenceSha256`, and the same audit-only authority projection. After the
source receipt, sealed typed operation
`BindHostGenerationImmutableAuditContinuitySourceEvidenceV1` publishes a
broker-private no-replace
`HostGenerationImmutableAuditContinuitySourceBindingV1` containing exactly the binding
attempt, handle digest, evidence digest, authority/version, durability generation, and
receipt digest, followed by the matching fixed outcome. It contains neither evidence bytes
nor the private handle. The broker binding is file-and-directory durable before
continuity-repair pre-audit or any call to `replay`; that pre-audit includes
`continuitySourceBindingReceiptSha256`. Replay must match both durable source and broker
records. A source-only pin or binding before broker final publication is resumed by binding
attempt and fixed broker pre; it can never authorize replay or a repair prefix by itself.
The source-acquisition pre/outcome, replay-association pre, and broker-binding final/outcome
are mandatory subrecords of the existing `continuity-repair-pre` boundary class and add no
registry id.

The source owns one stable root dirfd acquired from its disposition-pinned trusted ancestor
before admission. Every restart reacquires it with `O_DIRECTORY|O_CLOEXEC`, revalidates
mount/device/inode, owner, mode, and parent binding, and resolves every descendant with
`openat2(RESOLVE_BENEATH|RESOLVE_NO_SYMLINKS|RESOLVE_NO_MAGICLINKS|RESOLVE_NO_XDEV)`.
Pin and replay records use validated single-component direct-final names, unnamed
`O_TMPFILE|O_RDWR|O_CLOEXEC` preparation with creation mode `0600`, verified root
ownership, regular-file type, link count zero, expected mount/device, and no ACL or
extended security label before write, file sync, no-replace fd-relative link, exact final
reopen, and held-parent sync. Release uses only `unlinkat` on the selected one-component
leaf followed by anchored-parent reopen, identity revalidation, and `fsync`. Joined paths,
named temporaries, rename, replacement, cross-mount traversal, inherited descriptors, and
source-side broad cleanup are forbidden.

`ContinuitySourceBindingLifecycleV1` is exactly `Absent | AcquisitionPreAudited |
PinLinked | PinParentDurable | AcquisitionOutcomeDurable |
AssociationPreAudited | ReplayLinked | ReplayParentDurable | BrokerBindingDurable |
Complete | ReleasePreAudited | ReplayUnlinked | ReplayParentDurableAfterUnlink |
ReplayCensusCommitted | PinUnlinked | PinParentDurableAfterUnlink |
PinCensusCommitted | Released`. Every state is derived from durable source and broker
prefixes rather than accepted from the caller. `Complete` is the only replay-admitting
state. Release may begin only after broker compaction has published its durable
`ContinuityTargetsCompactedReceiptV1` and finished the bounded, successor-proved,
parent-synced witness reclamation that classifies the broker lifecycle as
`TargetsCompacted`; the receipt alone cannot mint the release permit. Source removal can
never create that receipt, complete witness reclamation, or make overall broker compaction
`Complete`. A crash at any release prefix resumes the first missing
revalidation, sync, census, or outcome step. `Released` is required before the broker may
release the attempt-local reserved slice. Source `Complete` means only binding completion;
it is not overall compaction completion. The ordered broker lifecycle is durably
`TargetsCompacted`, then source `Released`, then attempt-slice `CompletedReleased`, then
overall broker `Complete`. The source-release permit constructor accepts
`TargetsCompacted` and rejects both an earlier target prefix and an asserted overall
`Complete`; the latter cannot exist before release. Only final broker `Complete` frees the
live repair slot. Constructor and fresh-process negatives independently remove or
substitute each of the four ordered receipts.

The source release permit binds the intermediate broker-target receipt and these identities:

```text
releasedSourceCensusSha256 =
  SHA-256("d2b:host-generation:continuity-source-released-census:v1\0" ||
  live_acquisition_count_u16_be || live_replay_binding_count_u16_be)

ContinuitySourceReleaseAttemptIdV1 =
  SHA-256("d2b:host-generation:audit-continuity-source-release-attempt:v1\0" ||
  coordinator_private_id || continuity_source_binding_attempt_id ||
  broker_targets_compacted_sha256 ||
  source_durability_generation_u64_be)

continuitySourceReleaseAttemptSha256 =
  SHA-256("d2b:audit:host-generation:continuity-source-release-attempt:v1\0" ||
  private_continuity_source_release_attempt_id)

continuitySourceReleaseOutcomeRecordSha256 =
  SHA-256("d2b:host-generation:continuity-source-release-outcome:v1\0" ||
  private_continuity_source_release_attempt_id ||
  source_release_outcome_tag_u8 || released_source_census_sha256)

continuitySourceReleaseOutcomeSha256 =
  SHA-256("d2b:audit:host-generation:continuity-source-release-outcome:v1\0" ||
  continuity_source_release_attempt_sha256 ||
  continuity_source_release_outcome_record_sha256)
```

Source-release pre/outcome audit uses only
`continuitySourceReleaseAttemptSha256`,
`continuitySourceBindingAttemptSha256`,
`brokerTargetsCompactedSha256`, and closed target/prefix/outcome enums.
The source release outcome tag is exactly `0x01` released or `0x02` already released.
Both successful tags require `liveAcquisitionCount = 0`,
`liveReplayBindingCount = 0`, and a fresh recomputation of the exact
`releasedSourceCensusSha256`; both counts are unsigned two-byte big-endian integers.
Degradation is a separate source-lifecycle failure and has no successful outcome digest.
`brokerTargetsCompactedSha256` is the named audit projection of the immutable
`ContinuityTargetsCompactedReceiptV1` defined below. It exists only after every selected
broker target has its intent, unlink, parent durability, census reduction, and receipt
committed. Overall compaction completion, source release, and attempt-slice release are not
inputs to it. The source
receives the private attempt only inside the consumed sealed permit; no public DTO,
operator input, generic broker call, or digest reconstruction can mint one. Using the
source-binding and targets-compacted vectors and the source durability generation specified
in this section gives `brokerTargetsCompactedSha256 =
5f04253aa226aba438e4bc85e55041d8a581b56ccd0729161ccc9a3c5d7a6223`,
`ContinuitySourceReleaseAttemptIdV1 =
09d62866f414518292809355c84267085d398378f5be0a44ad5813dc8db36e30`
and `continuitySourceReleaseAttemptSha256 =
34fcc6edac2b7d885c4956bc44b734a8ecc9e5fa935df6a2b4d821dc42a82394`.
The literal source-release outcome input is
`liveAcquisitionCount = 0`, `liveReplayBindingCount = 0`, and
`sourceReleaseOutcomeTag = 0x01`. Its expected lowercase outputs are:

| Identity | Expected bytes |
| --- | --- |
| `releasedSourceCensusSha256` | `4d7dc751fd303dfb8c4da27759f38ec8fd3ff56b94183b7dfe83c9c904b8ad48` |
| `continuitySourceReleaseOutcomeRecordSha256` | `e352ce1ad797e20ec6caafe7ce3ef882996bd9af29f87c5e24d8603cbdb4112b` |
| `continuitySourceReleaseOutcomeSha256` | `303571fa69c60af7583cf6aee8574809211e172ac396f39c54dad7d856f32e88` |

An independent literal source-release vector perturbs the targets-compacted projection,
release private/audit domains, coordinator, binding attempt, target receipt, durability
generation and framing, both census counts and their two-byte framing, successful outcome
tag, and every field order. It substitutes the private attempt, audit attempt, target
receipt, census, outcome record, and audit outcome into every other same-width position.
Independent count-nonzero, census-substitution, tag-substitution, field-removal, and
downstream-proof substitution poisons cover each check.

The source-binding vector reuses the continuity vector's `coordinatorPrivateId`,
`dispositionAuthoritySha256`, `retentionEpochSha256`, `priorWatermarkSha256`,
`continuityRepairSequence`, `continuityEvidenceReplayHandleSha256`, and
`authoritativeEvidenceSha256`. It additionally sets
`sourceAuthoritySha256` to bytes `a0` through `bf`, `sourceVersion = 0x01020304`, and
`sourceDurabilityGeneration = 0x1112131415161718`. Its literal expected lowercase outputs
are:

| Identity | Expected bytes |
| --- | --- |
| `ContinuitySourceBindingAttemptIdV1` | `de2b87cb62f5d103a32dbaa773900525b7c417dd41b16e5c34a588c8a8feaf4c` |
| `continuitySourceBindingAttemptSha256` | `60718e796d53db55eb217c6dd1cef79d6260423ad7229ad685e826d0b2a18e94` |
| `continuitySourceAuthorityAuditSha256` | `b258671fd2aac779c588ad6cc4336f68dbb5d63936d202678a4400423cfc3013` |
| `continuitySourceBindingReceiptSha256` | `8d147d4920cd65c060f1bf44357e043b0b17f8e5d54876c394e800e9f4933fd2` |

The literal table, independently authored canonical-byte fixture, and production formula
are mutually read-independent. Negative vectors perturb each private and audit domain,
every 32-byte member, source version and four-byte framing, durability generation and
eight-byte framing, sequence, and field order. They substitute the private attempt, its
audit projection, the handle audit projection, evidence digest, authority digest, and
receipt into every other same-width field, and remove each member in turn. One poison per
vector, perturbation, substitution, removal, and framing assertion is mandatory.

Read-independent fresh-process cases crash before and after source pin creation, source
file sync, source directory sync, broker-binding pre-audit, handle association, association
file sync, association directory sync, every broker-binding-final publication boundary,
broker-binding outcome, compaction completion, source-release pre-audit, each replay/pin
unlink, each parent sync, each census commit, and release outcome. Each resumes the same
binding attempt, handle, evidence, receipt, and release generation. Missing, replaced,
rebound, partial, wrong-authority, wrong-version,
wrong-generation, handle-mismatched, evidence-mismatched, source-only, or broker-only state
is a distinct closed no-write degradation and admits neither replay nor continuity-repair
pre-audit. Source admission accepts exactly 256 live pairs. A 257th pair first drives the
oldest broker-target compaction, source release, and attempt-slice release; if blocked, it
returns that exact owning failure before acquisition pre-audit and never relabels the
condition `source-capacity`. It admits the exact 141,312-byte combined attempt ceiling,
maps a 141,313-byte refusal to authoritative-source-contract repair before acquisition
pre-audit, and admits the next pair only after complete audited source and attempt-slice
release. Independent
parent replacement, symlink, magic-link, cross-mount, wrong inode, final substitution,
descriptor-exec-leak, release-before-compaction, and release-receipt substitution cases
each have a hook and removal poison.

`ContinuitySourceReleasePermit<'coordinator>` is a private-field consumed capability. Its
only constructor is private to the exact coordinator module and is callable only after that
coordinator validates its own unnameable instance brand, invariant exclusive borrow, source
binding, target-compacted receipt, export set, and source durability generation. The
permit owns that brand and borrow for its full lifetime; `release_pinned` consumes it by
value and accepts no coordinator, receipt, digest, or generation argument. It has no public
constructor, struct literal, field, accessor, `Clone`, `Copy`, `Default`, `From`,
`TryFrom`, conversion, serde implementation, byte/digest/fd reconstruction, raw-fd view,
cross-coordinator transfer, lifetime widening or return, or second-dispatch path. Root,
direct broker clients, backup-administrator claims, source implementations, a foreign
coordinator, a digest-only target receipt, and a previously consumed permit cannot mint or
reuse one. Independent compile-fail and API-surface negatives cover every route and carry
one shrinkage poison each.

Storage availability repair never grants a human or independent process a source-prefix
mutation role. `ReconcileHostGenerationImmutableAuditContinuitySourcePrefixV1` is a sealed
broker suboperation entered only from startup/idle reconciliation or a selector-free local
Admin wake after external availability is restored. Its private permit binds the exact
coordinator brand, durable source acquisition/binding/release prefix, source root identity,
and first missing transition. That
`ContinuitySourcePrefixReconciliationPermitV1<'coordinator>` owns the invariant exclusive
borrow and is consumed by value. The operation publishes immutable fixed-field pre/outcome
audit, invokes
only the typed pin, bind, or release step for that prefix, and has the same no-construction,
clone, accessor, conversion, serde, cross-coordinator, lifetime-escape, and reuse seal as
the source release permit. It accepts no path, bytes, leaf, prefix, selector, role claim, or
repair body. Direct editing, copying, truncating, renaming, unlinking, recreating, or
reconciling the source root or lifecycle leaves by a site administrator is forbidden. At
most one reconciliation pre/outcome pair is live per attempt. The private reconciliation
identity binds the source-binding attempt, the exact first-missing lifecycle transition, a
checked unsigned 32-bit `recoveryGeneration`, and the prior reconciliation outcome digest
(canonical zero for generation zero). Its closed outcome is exactly
`Advanced { resultingPrefix } | RetrySelected { failureClass,
nextRecoveryGeneration }`. `Advanced` is valid only after the source lifecycle durably
passes the selected transition. `RetrySelected` is the durable supersession transition for
the same still-missing source prefix: it requires
`nextRecoveryGeneration = recoveryGeneration + 1`, preserves the failed pin, bind, or
release prefix unchanged, and authorizes no source mutation by itself. Overflow is a
no-mutation integrity incident, never generation wrap or pair accumulation. The literal
boundary matrix admits `recoveryGeneration = u32::MAX - 1`, durably selects
`nextRecoveryGeneration = u32::MAX`, exports and recycles the prior pair, and publishes
the maximum-generation pre-audit without wrap. A failed maximum-generation attempt cannot
select a successor: checked increment from `u32::MAX` returns the closed
`source-prefix-reconciliation/recovery-generation-overflow` integrity response before
next-generation pre-audit, pair unlink, parent sync, census-head mutation, source mutation,
or retained-record growth. Independent expected-generation, no-wrap, no-mutation,
constant-capacity, and no-pre-audit poisons cover both boundary cases.

The fixed reserved pair is recyclable only after its outcome is
file-and-directory-durably exported and either the lifecycle has durably advanced or the
exported outcome is the exact `RetrySelected` supersession. The next generation may publish
pre-audit only after the old pair's unlink, held-parent sync, and reduced census are
durable. That existing per-attempt source lifecycle census stores exactly one current
reconciliation head containing the exported prior outcome digest and selected next
generation; it is part of the already charged source pair state, not an append history or
additional broker-root record. The exported outcome is the durable mutation intent for
recycling its own pair, so pair absence before census is accepted only against that exact
exported outcome. A crash resumes the first missing export, supersession, pair-unlink,
parent-sync, census-head commit, or next-pre step. Repeated startup, idle, or Admin wakes
therefore retain one pair, monotonically advance only after a durably exported failure,
and converge on success without consuming another reserved pair. Withholding outcome
export, lifecycle advance, or same-prefix supersession performs no recycle, generation
advance, source mutation, or next pre-audit; successful recycling requires export plus
exactly one of advance or supersession. The read-independent matrix separately withholds
export while advance is durable, advance while export is durable, export while a
same-prefix `RetrySelected` supersession is durable, and supersession while export is
durable. It succeeds only after export plus advance in the success branch, or export plus
the exact next-generation supersession in the repeated-failure branch, and crashes a fresh
process before and after every recycle, parent-sync, census, and next-pre boundary.

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

Root creation also invokes the sealed typed suboperation
`PublishHostGenerationImmutableAuditContinuityReplayKeyV1`. Its capacity is charged before
its fixed pre-audit. Its private attempt and sole audit identity are:

```text
ContinuityReplayKeyPublicationAttemptIdV1 =
  SHA-256(
    "d2b:host-generation:audit-continuity-replay-key-publication-attempt:v1\0" ||
    coordinator_private_id || immutable_root_identity_sha256 ||
    root_generation_u64_be || operation_generation_u64_be)

continuityReplayKeyPublicationAttemptSha256 =
  SHA-256(
    "d2b:audit:host-generation:continuity-replay-key-publication-attempt:v1\0" ||
    private_continuity_replay_key_publication_attempt_id)
```

The fixed pre/outcome audit contains only
`continuityReplayKeyPublicationAttemptSha256`, root generation, and closed prefix/outcome
enums. It never contains the private attempt, its preimage, key bytes, a key digest, a
candidate commitment, inode identity, or an unqualified hash. Only after pre-audit does the
broker obtain exactly 32 bytes from the CSPRNG into an unnamed
`O_TMPFILE|O_RDWR|O_CLOEXEC` inode. It enforces a root-owned regular file, mode `0600`,
expected mount/device, no ACL or extended security label, and link count zero through
`InodeWritten`, `FileDurable`, and candidate commitment. It writes and file-syncs exactly
32 bytes.

Before final link, the broker durably publishes a broker-private no-replace
`ContinuityReplayKeyCandidateCommitmentV1` binding the private publication attempt,
candidate generation, expected mount/device/inode, exact length and posture, and
`keyCandidateSha256 =
SHA-256("d2b:host-generation:continuity-replay-key-candidate:v1\0" ||
candidate_generation_u64_be || key_bytes)`. This commitment and digest are private,
non-authorizing, never audited or exported, and included in the no-observable matrix. It
exists so an exact final can be bound to the selected inode and key after a crash; the
durable final name is never accepted from posture alone.

The broker then procfs-fd links the unnamed inode fd-relative to one no-replace leaf,
final-reopens it with `O_RDONLY|O_NOFOLLOW|O_CLOEXEC`, requires link count one, and
revalidates the same mount/device/inode, posture, exact bytes, and candidate commitment. It
syncs the held parent and only then publishes the fixed outcome. The broker-private root
census stores the accepted mount/device/inode identity, never a key digest, key bytes, or
candidate commitment.

`ContinuityReplayKeyPublicationPrefixV1` is exactly `Absent | PreAudited | InodeWritten |
FileDurable | CandidateCommitted | FinalLinked | FinalReopened | ParentDurable |
OutcomeDurable | CandidateCompacted | Complete`. Before `ParentDurable`, restart first
reopens the final. If the exact
commitment-bound final exists, it reuses that inode and key without CSPRNG use. If the final
is absent and the unnamed inode did not survive, restart durably marks only that candidate
`SupersededNoFinal` and must finish the bounded recycling machine below before incrementing
the candidate generation or resampling; it never treats the old commitment as a key or
silently overwrites it. Once `ParentDurable` was recorded,
absence is `replay-key-missing-after-parent-durable` integrity degradation and never permits
resampling. A nonidentical final at any prefix is preserved conflict. After parent
durability, exact-final replay finishes only the missing outcome and root-census commit.
That durable success moves the commitment to `PublishedFinal`; it is still non-authorizing
and is removed only by the typed candidate compaction mode below. `CandidateCompacted`
requires the fixed outcome, root census, exact final, and commitment removal all
file-and-directory durable. Only then may publication become `Complete`. An exact
`Complete` replay is zero-write. The root is unusable and source pin/bind/replay is denied
until `Complete`. Key pre/candidate/final/outcome and both candidate compaction modes are
mandatory independently hooked subrecords of the existing ensure-root boundary class and
add no registry id.

Candidate retention is a fixed-capacity lifecycle, not an append history. Before the first
replay-key pre-audit or candidate mutation, root creation reserves exactly 12 private
records of at most 8,192 encoded bytes each - 98,304 encoded bytes total - within the root
aggregate ceiling. The charge covers one live candidate commitment, one supersession,
one recycle selection, one recycle mutation intent, one target receipt, one reduced
census, both banks of the candidate-generation head, the recycle pre/outcome audit staging
pair, and the head-switch intent/receipt. General publication cannot consume this reserve,
and checked admission of the complete 12-record/98,304-byte charge precedes root mutation.
At every durable prefix there is at most one candidate commitment, one recycling
transaction, and two generation-head banks.
Admission with exactly 12 available records and 98,304 available encoded bytes succeeds.
Admission with 11 available records and 98,304 bytes, or 12 records and 98,303 bytes,
refuses before replay-key pre-audit, CSPRNG use, candidate creation, root census mutation,
or any covered audit append. Record and byte checks are independent, use literal test
values rather than production constants, and each has its own removal poison.

`ReplayKeyCandidateCompactionModeV1` is exactly `SupersededNoFinal |
PublishedFinal`. The mode is derived under the coordinator lock from the durable
publication prefix, never accepted from a caller. `SupersededNoFinal` requires the
committed final-name observation to be absent. `PublishedFinal` requires the exact final,
fixed publication outcome, and root-census identity to be durable and matching; it removes
only the now-redundant candidate commitment and never the replay key.
`RecycleHostGenerationImmutableAuditContinuityReplayKeyCandidateV1` is the only operation
that may compact either commitment state. Its private and audit identities are:

```text
ReplayKeyCandidateRecyclingAttemptIdV1 =
  SHA-256(
    "d2b:host-generation:audit-continuity-replay-key-candidate-recycling:v1\0" ||
    continuity_replay_key_publication_attempt_id ||
    candidate_generation_u64_be || key_candidate_sha256)

replayKeyCandidateRecyclingAttemptSha256 =
  SHA-256(
    "d2b:audit:host-generation:continuity-replay-key-candidate-recycling:v1\0" ||
    private_replay_key_candidate_recycling_attempt_id)
```

Its prefix is exactly `Absent | PreAudited | MutationIntentDurable |
CommitmentUnlinked | ParentDurable | CensusCommitted | ModeCommitted {
SupersededNoFinal: HeadSwitched | PublishedFinal: RootCensusRecorded } | Completed`.
Admission reopens the selected commitment and final-name census under the
coordinator lock and durably binds the derived mode, candidate identity, and either exact
final absence or the exact published-final outcome/root-census proof in the mutation
intent before `unlinkat`. Absence of the commitment after unlink is accepted only under
that exact intent. The operation then syncs the held parent and reduces the candidate
census. `SupersededNoFinal` additionally writes and syncs the inactive generation-head bank
with the next monotonic generation and completed recycle digest, switches the selected
bank, and removes and syncs the old bank before publishing the matching outcome.
`PublishedFinal` records the completed compaction digest in the existing root census and
does not advance the candidate generation or change either key-final or replay-key census
identity. It may not use rename, truncate, replacement, broad cleanup, or a second active
compaction. A fresh process resumes only the first missing prefix. A completed head
selection is derived from the unique highest valid generation whose predecessor digest
matches the other bank; while both banks exist, the older valid bank is the crash fallback.
Equal, disconnected, substituted, or independently advanced banks degrade before candidate
mutation.
`ReplayKeyCandidateRecyclingFailureClassV1` is exactly `hierarchy | write | file-sync |
link | reopen | unlink | directory-sync | census | conflict | audit-publication`.
No failure before a successful unlink may publish a failed outcome, settle, or supersede
the compaction. The original fixed identity remains pending at `PreAudited` or
`MutationIntentDurable`; after the named storage or audit repair, a fresh process retries
only the first missing step with that same identity. Once unlink succeeds, a later storage,
census, conflict, or outcome-audit-publication failure likewise cannot settle or supersede
the recycler: the original mutation intent remains pending and restart resumes that same
operation through parent revalidation, parent sync, census, any required head switch, and
outcome. No new candidate generation or recycler identity is legal until `Completed`.
A completed outcome whose audit segment file and directory are durable permits removal of
the transaction-local selection, intent, receipt, and census leaves plus the
`SupersededNoFinal` supersession leaf when present. In that mode the selected head bank
retains their digest and next generation; in `PublishedFinal` the root census retains the
compaction digest without a head advance. Only then is the complete 12-record reserve
reusable. Thus every commitment/crash/absent-final cycle returns to one head bank and zero
candidate/recycle records before resampling, while every successful publication reaches
zero candidate/recycle records before `Complete`; retained root state is constant for an
unbounded number of absent-final cycles.

The candidate vector sets `candidateGeneration = 0x0102030405060708`, uses key bytes `00`
through `1f`, and reuses the publication-attempt vector above. Its literal lowercase
outputs are:

| Identity | Expected bytes |
| --- | --- |
| `keyCandidateSha256` | `060408fedd99cd51b82d9a057b660e758c3ab51519fe15d1c07839831d7c4610` |
| `ReplayKeyCandidateRecyclingAttemptIdV1` | `54b77b17e5b35c0f92999aa2ccf9519f377c0ce5a0816d69feb1bf2ff6ed6ed0` |
| `replayKeyCandidateRecyclingAttemptSha256` | `65d880d2bd1904b0d29bf82ce8e67d4022596a3a1a8ce6d4e5eed9f9ffed45f5` |

The literal table, separately authored canonical-byte fixture, and production formulas are
mutually read-independent. Negatives perturb the candidate and recycle domains, key bytes,
candidate generation and eight-byte framing, publication attempt, candidate digest, field
order, and private-to-audit projection; substitute every 32-byte value into every other
position; and remove each check. One poison per vector, perturbation, substitution,
framing, order, and removal assertion is mandatory.

The bounded recycling fixture repeats commitment, crash, lost unnamed inode, absent final,
supersession, and recycling beyond the root record ceiling. A separate successful-final
fixture compacts `PublishedFinal` before `Complete`. Both crash at every recycle prefix;
the absent-final fixture also crashes at every generation-head bank transition, starts a
fresh broker each time, and requires eventual key publication with no capacity growth.
Every completed absent-final cycle has exactly one head bank, zero obsolete candidate
commitments, zero recycle prefix leaves, the original 12-record/98,304-byte reserve fully
reusable, and a strictly increasing generation. Every successful publication has zero
candidate commitments after `CandidateCompacted`, one exact replay-key final, and an
unchanged candidate generation.
Missing final-absence proof, reuse before parent sync/census/head switch/audit export,
simultaneous candidates, bank substitution, or retained obsolete state is a no-mutation
failure with its own hook and removal poison. Separate literal admission probes accept the
exact 12-record/98,304-byte reserve and refuse the one-record-short and one-byte-short
cases before any mutation. Recycler probes inject every class above at each reachable
prefix. Every pre-unlink case is repaired and retried under the original fixed identity;
every post-unlink case resumes the original intent in a fresh process. Neither may append a
new recycler generation. Published-final probes additionally reject a missing or
mismatched publication outcome, root census, exact final, or mode and prove that commitment
removal cannot alter the replay-key final.

The frozen replay-key publication-attempt vector uses the continuity vector's
`coordinatorPrivateId`, sets `immutableRootIdentitySha256` to bytes `00` through `1f`,
`rootGeneration = 0x0102030405060709`, and
`operationGeneration = 0x1112131415161719`. Its expected lowercase outputs are:

| Identity | Expected bytes |
| --- | --- |
| `ContinuityReplayKeyPublicationAttemptIdV1` | `233080fff66baf2ef3f8279c201c3e5dbb5e319c9f2f1b88b418ea8ed4ca4c5b` |
| `continuityReplayKeyPublicationAttemptSha256` | `94bde883bb3423f659b1df0c155a8e2ce86cf93e595d4e711bc11203159e0c6f` |

Independent literal-vector tests perturb both domains, every member, each eight-byte
generation framing, and field order; substitute the private attempt and audit digest into
every other 32-byte identity field; and remove each check separately.

The resulting `brokerPrivateContinuityReplayKey` is persistent across broker process death,
is never rotated while a retained epoch or continuity prefix exists, and is never exported,
audited, serialized, logged, displayed, included in a response, passed to the daemon, or
exposed by `Debug`. After any completed key publication, missing, short, long, duplicated,
hard-linked, replaced, wrong-inode, wrong-owner/group/mode/type/mount/device, symlink,
magic-link, ACL/label-bearing, partial-final, or identity-mismatched key state degrades the
root and admits no source pin, binding, replay, repair, prune, settlement, or compaction.
It is never recreated as recovery. Independent ensure-root first-run and fresh-process
restart cases cover capacity refusal, key pre-audit, CSPRNG failure, every prefix,
candidate commitment and supersession, outcome-publication pending, response loss,
secure-posture revalidation, exact completed reopen, zero pre-link/one post-reopen link
counts, exact-final reuse before parent durability, absent-final resampling before parent
durability only after bounded candidate recycling, repeated commitment/crash/absent-final
cycles with constant retained state and eventual completion, absent-final degradation
after parent durability, and every
missing/partial/replaced poison before descendant use. An audit reader
therefore has every audit projection and still cannot derive or test a private replay
handle.

The stable private replay and repair identities are:

```text
ContinuityEvidenceReplayHandleV1 =
  HMAC-SHA-256(
    key = broker_private_continuity_replay_key,
    message =
      "d2b:host-generation:audit-continuity-evidence-replay-handle:v1\0" ||
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
sequence. The handle is deterministically reconstructible only from the persistent sealed
key plus the disposition, epoch, prior watermark, sequence, and evidence digest. It is not
source-selected and is disclosed only as an opaque value to the disposition-pinned source
through the typed binding call; it is otherwise broker-private. The operation identity is
therefore reconstructible after a pre-only crash without recovering a request frame or
asking a changed source to choose bytes, while audit-only inputs are insufficient.

Before source binding or repair pre-audit, the already reserved replacement charge debits
that attempt's exact slice for one broker binding plus its pre/outcome pair, one continuity
evidence record and encoded bytes, one optional immutable watermark, fixed repair and
decision-selection/settlement records, target/successor/head/final-absence metadata,
per-target receipts, one recyclable recovery prefix, source release, and later compaction.
The operation
refuses through the capacity controller before source-binding pre-audit if the
one-record/131,072-byte evidence, one-binding-record, 256-live-attempt per-intent, or other
reserved subset is exceeded. An audited capacity refusal may append only its capacity
pre/outcome pair; it mutates no source binding, continuity prefix, watermark, prune state,
or covered operation. Standing-reserve exhaustion is the separate no-audit admission
refusal defined above.

The operation first publishes
`coordinator-immutable-audit-continuity-repair/pre-mutation` with exactly the fixed edge id,
`continuityRepairAttemptSha256`, `retentionEpochSha256`, `priorWatermarkSha256`,
`authoritativeEvidenceSha256`, `continuityEvidenceReplayHandleSha256`,
`continuitySourceBindingReceiptSha256`, `continuityRepairSequence`, and exactly one nested
deadline plan:

```text
ContinuityRepairDeadlinePlanV1 =
    BeforeDay90
  | Day90Reached { mandatoryPruneTargetSha256 }
```

There is no independent deadline flag, prune-required boolean, or nullable prune digest.
The broker constructs the plan from the original epoch deadline and current authoritative
lower bound under the lock. A before-day-90 plan cannot carry a prune target, and a
day-90-reached plan cannot omit one.

A day-90 target always binds the complete governed set, never one member. Members are sorted
by the unsigned byte order of their 32-byte `backupMemberSha256`, are unique, and number
from 1 through 256. The canonical whole-set census and target are:

```text
governedSetCensusSha256 =
  SHA-256("d2b:host-generation:immutable-audit-governed-census:v1\0" ||
  retention_epoch_sha256 || member_count_u16_be ||
  for each ordered member: 0x01 || backup_member_sha256)

mandatoryPruneTargetSha256 =
  SHA-256("d2b:audit:host-generation:continuity-mandatory-prune-target:v1\0" ||
  retention_epoch_sha256 || governed_set_initial_census_sha256 ||
  member_count_u16_be ||
  for each ordered member: 0x01 || backup_member_sha256)
```

Every digest member is 32 bytes, the count is an unsigned two-byte big-endian integer, and
`0x01` is the one-byte member framing tag. The initial census digest must equal both a
fresh canonicalization of the held durable census and the digest recorded in the deadline
plan's private target metadata. The digest field itself, any later prune result, a path,
serializer bytes, and a caller-selected order are excluded.

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

The redaction matrix gives `canonicalEvidenceBytes`, `evidenceRealtimeSeconds`,
`evidenceBootTimeNanoseconds`, `evidenceBootIdBytes`, and `authorityProofBytes` distinct
nonoverlapping canaries. Each canary is independently sought in audit, human and JSON
responses, public and private wire projections, errors and source chains, logs, metric
names/labels/values/exemplars, span names/attributes/events, panic text, `Debug`, and
`Display`. A shared canary or one aggregate body search cannot satisfy another cell. The
evidence type and every direct or transitive containing wrapper have compile-time negatives
for `Display`; each `Debug` implementation has an exact golden containing only its concrete
type name and `([REDACTED])`. One removal poison per sensitive member, observable surface,
`Display` negative, and redacted-`Debug` implementation must fail.

`authoritativeEvidenceSha256` is exactly
`SHA-256("d2b:host-generation:audit-continuity-authoritative-evidence:v1\0" ||
u32_be(len(canonicalEvidenceBytes)) || canonicalEvidenceBytes ||
evidenceRealtimeSeconds_u64_be || evidenceBootTimeNanoseconds_u64_be ||
u32_be(len(evidenceBootIdBytes)) || evidenceBootIdBytes ||
u32_be(len(authorityProofBytes)) || authorityProofBytes)`. The three bounded byte strings use
four-byte big-endian lengths, the clock values use eight-byte big-endian integers, and no
serializer output or implicit concatenation enters the formula. The evidence digest field
itself and `continuityRepairAttemptSha256` are not members of this preimage.

The frozen known-answer vector for all five continuity identities uses:

```text
brokerPrivateContinuityReplayKey =
  000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f
dispositionAuthoritySha256 =
  202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f
retentionEpochSha256 =
  404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f
priorWatermarkSha256 =
  606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f
coordinatorPrivateId =
  808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9f
continuityRepairSequence = 0x0102030405060708
canonicalEvidenceBytes = 000102ff
evidenceRealtimeSeconds = 0x0102030405060708
evidenceBootTimeNanoseconds = 0x1112131415161718
evidenceBootIdBytes = 626f6f742d41
authorityProofBytes = 70726f6f662d42
```

The expected lowercase hex outputs are:

| Identity | Expected bytes |
| --- | --- |
| `authoritativeEvidenceSha256` | `16c66c65b0705a846d962dd04326942d7789698ec5009872c3669080d6d244b9` |
| `ContinuityEvidenceReplayHandleV1` | `5b68255ad34dd5358fa6b405db221cc4c79b7736cd1ecf6a42e44f193d8b85fd` |
| `ContinuityRepairAttemptIdV1` | `b1b347d3c9fb2f1dc99c35a44d43c2492eb183bdfc5394dcfb2bbbb4abca9949` |
| `continuityEvidenceReplayHandleSha256` | `635fc5494e528163265784d3e8480d6227e90b48e2aa54748a6defd0f82b040e` |
| `continuityRepairAttemptSha256` | `0e7225df2517e961b428335533cd4cd1cb2a3e4db5999d0c46b89da8b6fdc0a0` |

One literal expected-value table, one separately authored fixture, and the production
implementation are mutually read-independent. Negative vectors perturb the HMAC key, each
private and audit domain byte, every 32-byte member, the sequence value and its eight-byte
framing, every evidence body member, every four-byte length, and the field order one at a
time. Changing excluded `schemaVersion`, `kind`,
`continuityRepairAttemptSha256`, or the stored `authoritativeEvidenceSha256` field while
leaving the authoritative-evidence preimage unchanged must not alter that digest. Removing
any vector, perturbation, excluded-field check, or framing check has its own poison.
Separate leakage negatives feed the complete audit projection to the test and prove that
neither private identity can be reconstructed or equality-tested without the sealed key;
key canaries are absent from every observable surface.

Fresh-process replay reconstructs the private handle and attempt from the durable pre fields,
coordinator-private id, and disposition, then invokes only the exact replay method. The
returned canonical record must reproduce the pre-bound digest, authority, epoch, prior
watermark, sequence, raw clock values, boot identity, and proof. Source unavailability is
`source-unavailable`; a changed source version, authority, handle binding, or any
nonidentical returned value is `source-conflict`. Neither may select a new attempt, evidence
digest, or deadline plan, and neither advances a watermark. Independent tests replace,
remove, and mutate the source after pre-only durability and require the same attempt identity
plus a closed failure, never newly selected bytes.

The continuity evidence is charged to the governed replaced set and remains through every
restart until one of the two initial compaction modes, including its sole recovery
continuation when required, reaches overall `Complete`. `FinalizeReplacedSet` is
outcome-specific; a watermark is mandatory only for a repaired terminal outcome and
forbidden as a substitute for a degraded terminal outcome:

In every row, "binding pre/final/outcome" is the complete source lifecycle export set:
source-acquisition pre and outcome, replay-association pre, broker binding final, and broker
binding outcome. "Settlement records" includes durable decision-basis intent, decision
basis, decision selection, decision-pre, outcome intent, and terminal outcome.

| Settled terminal outcome | Exact compaction prerequisite |
| --- | --- |
| `RepairedBeforeDay90` | matching source/broker replay binding, pre, exact durable evidence, immutable watermark final, repaired outcome, durable governed-set absence, final absence proof, and file-and-directory-durable export of the binding pre/final/outcome, repair pre, watermark, settlement records, outcome, every partial or later prune pre/outcome and reduced-census record, and final absence proof |
| `RepairedAfterMandatoryPrune` | matching source/broker replay binding, pre, exact durable evidence, whole-set mandatory-prune proof, immutable watermark final, repaired outcome, empty governed census, final absence proof, and file-and-directory-durable export of the binding pre/final/outcome, repair pre, target, every prune pre/outcome and reduced-census record, whole-set proof, watermark, settlement records, outcome, and final absence proof |
| `DegradedBeforeDay90` | matching source/broker replay binding and pre, evidence state proven as either never durable or exact durable, degraded outcome, zero accepted watermark, durable governed-set absence, final absence proof, and file-and-directory-durable export of the binding pre/final/outcome, repair pre, settlement records, outcome, every partial or later prune pre/outcome and reduced-census record, and final absence proof |
| `DegradedDay90BeforePrune` | matching source/broker replay binding and pre, evidence state proven as either never durable or exact durable, degraded outcome, zero accepted watermark, later durable governed-set absence, final absence proof, and file-and-directory-durable export of the binding pre/final/outcome, repair pre, target, settlement records, outcome, every prune pre/outcome and reduced-census record both before and after that degraded outcome, every failed prune outcome, and final absence proof |
| `DegradedDay90AfterPrune` | matching source/broker replay binding, pre, evidence state proven as either never durable or exact durable, whole-set mandatory-prune proof, degraded outcome, zero accepted watermark, empty governed census, final absence proof, and file-and-directory-durable export of the binding pre/final/outcome, repair pre, target, every prune pre/outcome and reduced-census record including failed attempts, whole-set proof, settlement records, outcome, and final absence proof |

`ReclaimDegradedAttempt` is admitted only for one settled degraded outcome with zero
accepted watermark, a complete durable source/broker binding, evidence proven never durable
or present exact, file-and-directory-durable export of every attempt-local record, and no
incomplete member mutation. Governed members may remain. Any durable prune-history entry is
first represented by the same epoch-level history census used by the eventual final-absence
proof. Its unlink target set is exactly present evidence, the broker replay binding, and
the attempt census; no backup member, current or superseded watermark, epoch census, prune
history census, or replay key is eligible. Completion drives source release and capacity
release before the slot is reusable. A repaired outcome, unexpected watermark, incomplete
settlement, unexported record, or prune entry absent from the epoch history census refuses
this mode before compaction pre-audit.

Immutable admission and transient observation are separate types.
`ContinuityEvidenceSelectionV1` is exactly `NeverDurable |
SelectedPresent { expectedLeafRecordSha256 }`. `NeverDurable` requires the durable repair
publication registry to prove no evidence final was ever linked; `SelectedPresent` requires
the exact final identity and canonical digest at selection. It is stored in the durable
target-set record and bound by the permit, private attempt identity, pre-audit, outcome, and
every recovery generation.

`ContinuityEvidenceObservationV1` is exactly `NeverExisted | PresentExact |
AbsentAfterSelectedUnlink { compactionOperationSha256, targetOrdinal,
targetMutationIntentSha256 }`. It is derived immediately before or after one target step
and is never an input to immutable selection or attempt identity. The absent variant is
legal only while resuming the named original operation and exact durable mutation intent
after that intent's selected unlink; no target receipt is required or legal at this
prefix. It still requires anchored-parent revalidation and sync before census reduction
and receipt publication. Evidence that was durable but is missing before that selected
unlink, absent under another operation, intent, or ordinal, mismatched, or unprovably never
durable is integrity degradation, not
`AlreadyCompacted`. `NeverDurable` still permits removal of replay binding and attempt
census. `AlreadyCompacted` is available only after a complete matching prior compaction
outcome, matching per-target receipts, matching reduced census, complete source release,
and matching capacity release.

Pending settlement, a missing terminal outcome, an incomplete repair, file-only export, a
missing or mismatched binding, pre, evidence, settlement
record, outcome, final absence proof, partial or later prune record, per-member
reduced-census record, required watermark, target, or mandatory-prune proof, an unexpected
watermark on a degraded branch, or any malformed replay-key state refuses before
`FinalizeReplacedSet` pre-audit. A present governed member also refuses that mode; it is
allowed only by the separately typed `ReclaimDegradedAttempt` prerequisites above, where it
is never a target. Fresh-process negatives independently cover missing, short, long,
partial, replaced, and identity-mismatched replay keys; evidence-never-durable,
evidence-durable-present, illicit missing durable evidence, mismatched evidence,
post-bound-unlink absence, and unbound absence; and first, intermediate, and final member
prune export omission or mismatch. Every negative has a separate hook and removal poison.

Only the typed sealed operation
`CompactHostGenerationImmutableAuditContinuityV1` may remove continuity evidence, replay
metadata, a no-longer-current watermark, or their census entries. The coordinator creates a
private
`CompactHostGenerationImmutableAuditContinuityPermit<'coordinator>` only after validating
one row above. The permit owns an invariant exclusive borrow and private instance brand,
binds the exact mode, terminal outcome, complete immutable unlink target set, complete
immutable current-head proof, evidence selection, export set, predecessor/successor proof,
and any prior completed-target receipts, and is consumed by value at dispatch. It has no public
constructor, field, accessor, `Clone`, `Copy`, `Default`, `From`, `TryFrom`, serde,
conversion, byte/digest/fd reconstruction, cross-coordinator use, lifetime escape, or
second-dispatch surface. Compiler and API negatives cover each route independently.

Its stable identity is:

```text
ContinuityCompactionAttemptIdV1 =
  SHA-256("d2b:host-generation:audit-continuity-compaction-attempt:v1\0" ||
  coordinator_private_id || continuity_repair_attempt_sha256 ||
  terminal_outcome_record_sha256 || governed_set_final_census_sha256 ||
  required_export_set_sha256 || continuity_compaction_target_set_sha256 ||
  current_continuity_head_proof_sha256)

continuityCompactionAttemptSha256 =
  SHA-256("d2b:audit:host-generation:continuity-compaction-attempt:v1\0" ||
  private_continuity_compaction_attempt_id)
```

That identity is the `FinalizeReplacedSet` identity retained by the frozen vector below.
The safe degraded mode and repair of a settled failed compaction use distinct canonical
identities rather than aliasing or retrying it:

```text
ContinuityDegradedAttemptReclamationIdV1 =
  SHA-256("d2b:host-generation:audit-continuity-degraded-reclamation:v1\0" ||
  coordinator_private_id || continuity_repair_attempt_sha256 ||
  terminal_outcome_record_sha256 || retained_governed_census_sha256 ||
  required_export_set_sha256 || continuity_compaction_target_set_sha256 ||
  current_continuity_head_proof_sha256)

ContinuityCompactionRecoveryGenerationIdV1 =
  SHA-256("d2b:host-generation:audit-continuity-compaction-recovery:v1\0" ||
  prior_compaction_operation_sha256 ||
  prior_compaction_outcome_audit_sha256 ||
  recovery_generation_u32_be || completed_target_receipt_set_sha256 ||
  residual_target_set_sha256 || current_continuity_head_proof_sha256)
```

The mode-specific projections and tagged common operation projection are:

```text
continuityDegradedAttemptReclamationSha256 =
  SHA-256("d2b:audit:host-generation:continuity-degraded-reclamation:v1\0" ||
  private_continuity_degraded_attempt_reclamation_id)

continuityCompactionRecoveryGenerationSha256 =
  SHA-256("d2b:audit:host-generation:continuity-compaction-recovery:v1\0" ||
  private_continuity_compaction_recovery_generation_id)

continuityCompactionOperationSha256 =
  SHA-256("d2b:audit:host-generation:continuity-compaction-operation:v1\0" ||
  compaction_operation_kind_tag_u8 || mode_specific_audit_sha256)
```

The tagged identity algebra is exact:

```text
ContinuityCompactionOperationV1 =
    FinalizeReplacedSet {
      privateId: ContinuityCompactionAttemptIdV1
    }
  | ReclaimDegradedAttempt {
      privateId: ContinuityDegradedAttemptReclamationIdV1
    }
  | RecoveryGeneration {
      privateId: ContinuityCompactionRecoveryGenerationIdV1
    }

(compaction_operation_kind_tag_u8, private_compaction_operation_id,
 mode_specific_audit_sha256) =
    FinalizeReplacedSet:
      (0x01, private_continuity_compaction_attempt_id,
       continuityCompactionAttemptSha256)
  | ReclaimDegradedAttempt:
      (0x02, private_continuity_degraded_attempt_reclamation_id,
       continuityDegradedAttemptReclamationSha256)
  | RecoveryGeneration:
      (0x03, private_continuity_compaction_recovery_generation_id,
       continuityCompactionRecoveryGenerationSha256)
```

The two initial admission modes are `FinalizeReplacedSet` and
`ReclaimDegradedAttempt`. `RecoveryGeneration` is the sole continuation kind and is
constructible only from a settled pre-unlink degraded predecessor; it is not a third
initial admission mode. The existing `continuityCompactionAttemptSha256` is the
mode-specific projection for `FinalizeReplacedSet`. Constructors reject every
kind/private-id/projection mismatch. Every target intent, receipt, recovery edge,
targets-compacted receipt, and final completion is carried by the sealed sum and binds
either its selected private operation id or `continuityCompactionOperationSha256`, as the
formula specifies, never an untagged digest relabeled from another kind. The selection
itself binds the kind tag plus the complete immutable mode-specific fields from which the
matching initial or recovery private identity is validated.

In every formula below, `private_compaction_operation_id` is exactly the 32-byte private id
selected by that sealed sum: `ContinuityCompactionAttemptIdV1` for tag `0x01`,
`ContinuityDegradedAttemptReclamationIdV1` for tag `0x02`, or
`ContinuityCompactionRecoveryGenerationIdV1` for tag `0x03`. The kind is independently
bound by `continuityCompactionOperationSha256` and
`continuityCompactionSelectionSha256`; no formula accepts a caller-supplied untagged id or
a private id from another variant. In a recovery identity,
`priorCompactionOperationSha256` is the tagged common projection of the immediately
settled predecessor, `priorCompactionOutcomeAuditSha256` is that predecessor's immutable
matching outcome projection, `completedTargetReceiptSetSha256` is the canonical
ordinal-prefix receipt set committed by that predecessor, and `residualTargetSetSha256`
is the canonical suffix of the predecessor's original target set beginning at its first
uncompleted ordinal. The recovery selection repeats that exact receipt set as
`priorCompletedTargetReceiptSetSha256`, binds the same original target set and terminal
record, and replaces only the freshly validated current-head proof. Constructors reject a
receipt outside the completed prefix, a residual target inside it, a gap, a substituted
selection, a predecessor outcome from another operation, and any finalize, reclamation, or
recovery identity relabeling.

The private operation record and every strict generated schema use the externally tagged
field `kind` with exactly `finalize-replaced-set | reclaim-degraded-attempt |
recovery-generation` and exactly one matching variant body. There is no independent
`mode`, nullable recovery body, generic 32-byte operation id, or caller-supplied kind tag.
Canonical encoding is the one-byte tag followed by the exact 32-byte private id selected
above. Schema, serde, and constructor negatives reject an unknown kind, two bodies, an
empty body, a body from another kind, or any mismatch among kind, private id, and
mode-specific audit projection.

The complete immutable selection and its audit projection are:

```text
continuityCompactionSelectionSha256 =
  SHA-256("d2b:host-generation:continuity-compaction-selection:v1\0" ||
  compaction_operation_kind_tag_u8 || terminal_outcome_record_sha256 ||
  selected_governed_census_sha256 || required_export_set_sha256 ||
  selected_target_set_sha256 || selected_current_head_proof_sha256 ||
  prior_completed_target_receipt_set_sha256)

continuityCompactionSelectionAuditSha256 =
  SHA-256("d2b:audit:host-generation:continuity-compaction-selection:v1\0" ||
  continuity_compaction_selection_sha256)

continuityCompactionTargetSetAuditSha256 =
  SHA-256("d2b:audit:host-generation:continuity-compaction-target-set:v1\0" ||
  continuity_compaction_target_set_sha256)

currentContinuityHeadProofAuditSha256 =
  SHA-256("d2b:audit:host-generation:continuity-current-head-proof:v1\0" ||
  current_continuity_head_proof_sha256)

requiredExportSetAuditSha256 =
  SHA-256("d2b:audit:host-generation:continuity-compaction-export-set:v1\0" ||
  required_export_set_sha256)

governedSetCensusAuditSha256 =
  SHA-256("d2b:audit:host-generation:immutable-audit-governed-census:v1\0" ||
  selected_governed_census_sha256)

terminalOutcomeRecordAuditSha256 =
  SHA-256("d2b:audit:host-generation:continuity-terminal-outcome-record:v1\0" ||
  terminal_outcome_record_sha256)
```

Every member is exactly 32 bytes except the one-byte operation tag and four-byte
big-endian recovery generation. Private identities, preimages, private selection, raw
target-set/head/export/census/terminal digests, complete selected leaf records, and
transient observations never enter audit. Audit may contain only the named audit
projections above and the closed tags defined here.

Target mutation, receipt, residual selection, and outcome use these canonical encodings.
The anchored parent and every post-unlink census are typed canonical values, not opaque
fixture digests:

```text
AnchoredParentIdentityV1 = {
  mountId: u64,
  deviceMajor: u32,
  deviceMinor: u32,
  inode: u64,
  uid: u32,
  gid: u32,
  mode: u32,
  linkCount: u64
}

ContinuityCompactionCensusTargetV1 = {
  kind: Evidence | SourceReplayBinding | SupersededWatermark | AttemptCensus,
  expectedLeafRecordSha256: [u8; 32]
}

ContinuityCompactionAttemptCensusV1 = {
  repairAttemptSha256: [u8; 32],
  selectionSha256: [u8; 32],
  remainingTargets: BoundedOrderedSet<ContinuityCompactionCensusTargetV1, 0, 4>
}

anchoredParentIdentitySha256 =
  SHA-256("d2b:host-generation:continuity-compaction-anchored-parent-identity:v1\0" ||
  parent_mount_id_u64_be ||
  parent_device_major_u32_be || parent_device_minor_u32_be ||
  parent_inode_u64_be || parent_uid_u32_be || parent_gid_u32_be ||
  parent_mode_u32_be || parent_link_count_u64_be)

continuityCompactionAttemptCensusSha256 =
  SHA-256("d2b:host-generation:continuity-compaction-attempt-census:v1\0" ||
  continuity_repair_attempt_sha256 || continuity_compaction_selection_sha256 ||
  remaining_target_count_u8 ||
  for each remaining target in target-kind order:
    target_kind_tag_u8 || expected_leaf_record_sha256)

continuityCompactionTargetMutationIntentSha256 =
  SHA-256("d2b:host-generation:continuity-compaction-target-mutation-intent:v1\0" ||
  private_compaction_operation_id || continuity_compaction_selection_sha256 ||
  target_ordinal_u8 || target_kind_tag_u8 || expected_leaf_record_sha256 ||
  pre_unlink_observation_tag_u8 || anchored_parent_identity_sha256)

continuityCompactionTargetReceiptSha256 =
  SHA-256("d2b:host-generation:continuity-compaction-target-receipt:v1\0" ||
  private_compaction_operation_id ||
  continuity_compaction_target_mutation_intent_sha256 ||
  target_ordinal_u8 || target_kind_tag_u8 || post_unlink_absence_tag_u8 ||
  anchored_parent_identity_sha256 || resulting_census_sha256)

completedTargetReceiptSetSha256 =
  SHA-256("d2b:host-generation:continuity-compaction-target-receipt-set:v1\0" ||
  receipt_count_u8 ||
  for each receipt in target-ordinal order:
    target_ordinal_u8 || continuity_compaction_target_receipt_sha256)

residualTargetSetSha256 =
  SHA-256("d2b:host-generation:continuity-compaction-residual-target-set:v1\0" ||
  original_target_set_sha256 || first_remaining_ordinal_u8 ||
  residual_target_count_u8 ||
  for each residual target in target-ordinal order:
    target_kind_tag_u8 || expected_leaf_record_sha256)

continuityCompactionOutcomeSha256 =
  SHA-256("d2b:host-generation:continuity-compaction-outcome:v1\0" ||
  private_compaction_operation_id || compaction_outcome_tag_u8 ||
  completed_target_receipt_set_sha256 || residual_target_set_sha256 ||
  outcome_variant_payload)

continuityCompactionOutcomeAuditSha256 =
  SHA-256("d2b:audit:host-generation:continuity-compaction-outcome:v1\0" ||
  continuity_compaction_operation_sha256 ||
  continuity_compaction_outcome_sha256)
```

Every identity member after the domain is fixed-width. All integer fields are unsigned
big-endian at the width named above; `mode` contains the complete file-type and permission
bits, and no serializer bytes, path, timestamp, padding, host-endian value, or implicit
length enters either formula. `ContinuityCompactionAttemptCensusV1` is a sealed typed
value, not a caller-supplied digest: its target set is unique, strictly target-kind ordered,
and encoded with tags `0x01 | 0x02 | 0x03 | 0x04` in the variant order above. The parent
identity is computed from the freshly reopened held dirfd and must match before unlink and
after absence;
`anchored_parent_identity_sha256` in the mutation and receipt formulas is exactly
`anchoredParentIdentitySha256`. The census is freshly
canonicalized from held descriptors after the named target is absent. In each target
receipt, `resulting_census_sha256` is exactly that
`continuityCompactionAttemptCensusSha256`; in
`ContinuityTargetsCompactedReceiptSha256`, `final_attempt_census_sha256` is exactly the
same formula with `remaining_target_count_u8 = 0` and no target entries. The count is one
byte because the closed target set contains at most four members. Unknown, duplicate,
missing, reordered, already-receipted, or unselected entries refuse rather than producing
another census.

Ordinals are zero-based one-byte integers. Target and operation tags are the closed values
defined in this section. Observation tag `0x01` is `PresentExact`; an intent cannot encode
absence. Post-unlink tag `0x01` is `AbsentAfterSelectedUnlink`. Outcome tags are `0x01`
compacted, `0x02` already compacted, and `0x03` degraded. Compacted and already-compacted
have the zero-length `outcome_variant_payload` and the canonical empty residual set.
For degraded, `outcome_variant_payload` is exactly
`target_ordinal_u8 || target_kind_tag_u8 || failure_class_tag_u8`; failure tags are `0x01`
head-changed, `0x02` target-changed, and `0x03` unlink. No other symbolic or
serializer-defined payload is permitted. A degraded outcome is legal only
while the named target is still present exact and before a successful unlink. Post-unlink
`hierarchy`, `write`, `file-sync`, `link`, `reopen`, `directory-sync`, `census`,
`conflict`, or `audit-publication` failures while publishing parent/census/receipt/outcome
state are pending prefixes of the original operation and have no degraded outcome tag.
Counts are one byte because the target set is capped at four.
Unknown, duplicate, missing, reordered, or out-of-range members are invalid rather than
alternative encodings.

`selected_governed_census_sha256`, `selected_target_set_sha256`,
`selected_current_head_proof_sha256`, and
`prior_completed_target_receipt_set_sha256` in the selection formula are exactly the
32-byte fields stored by the sealed selection record. In a recovery selection, the last
field is the predecessor's canonical completed ordinal-prefix receipt set; in either
initial mode it is the canonical empty receipt set. `prior_compaction_outcome_audit_sha256`
in a recovery identity is exactly the predecessor's
`continuityCompactionOutcomeAuditSha256`; it is not an arbitrary audit digest. These
aliases, the three operation variants, all fixed tags, every length/count/integer framing,
and the zero-length payloads are explicit schema and known-answer-vector inputs. The
finalize, reclamation, and recovery rows below each pin the private id, mode-specific
projection, tagged common operation projection, selection, and all reachable downstream
hashes; one missing variant row or one implementation-derived expected value fails the
vector fixture before lifecycle tests run.

The audit-only wrappers for completed receipts and residual selection are:

```text
completedTargetReceiptSetAuditSha256 =
  SHA-256("d2b:audit:host-generation:continuity-compaction-target-receipt-set:v1\0" ||
  completed_target_receipt_set_sha256)

residualTargetSetAuditSha256 =
  SHA-256("d2b:audit:host-generation:continuity-compaction-residual-target-set:v1\0" ||
  residual_target_set_sha256)
```

The canonical terminal digest is outcome-specific:

```text
terminalOutcomeRecordSha256 =
  SHA-256("d2b:host-generation:continuity-terminal-outcome-record:v1\0" ||
  continuity_repair_attempt_sha256 || terminal_outcome_tag_u8 ||
  deadline_plan_tag_u8 || deadline_plan_payload ||
  terminal_outcome_payload)

deadline_plan_payload =
    BeforeDay90: empty
  | Day90Reached: mandatory_prune_target_sha256

terminal_outcome_payload =
    RepairedBeforeDay90: repaired_watermark_sha256
  | RepairedAfterMandatoryPrune:
      mandatory_prune_proof_sha256 || repaired_watermark_sha256
  | DegradedBeforeDay90 | DegradedDay90BeforePrune:
      failure_branch_tag_u8 || failure_class_tag_u8
  | DegradedDay90AfterPrune:
      mandatory_prune_proof_sha256 ||
      failure_branch_tag_u8 || failure_class_tag_u8
```

Terminal outcome tags are exactly `0x01`, `0x02`, `0x81`, `0x82`, and `0x83` in the
variant order above. Deadline plan tags are exactly `0x01` before day 90 and `0x02` day 90
reached; only `0x02` carries the 32-byte mandatory target. Failure branch tags are exactly
`0x01` source, `0x02` publication,
and `0x03` retention. Source class tags are `0x01` unavailable and `0x02` conflict.
Terminal publication class tags are `0x01` hierarchy, `0x02` write, `0x03` file-sync,
`0x04` link, `0x05` reopen, `0x06` directory-sync, `0x07` conflict, and `0x08`
audit-publication. Terminal retention class tags are `0x01` clock-rollback, `0x02`
clock-watermark, `0x03` epoch-invalid, `0x04` clock-forward-discontinuity, `0x05`
clock-continuity-ambiguous, `0x06` clock-overflow, `0x07` unlink, `0x08`
directory-sync, `0x09` census, `0x0a` audit-publication, `0x0c`
standing-reserve-missing, `0x0d` standing-reserve-overdrawn, `0x0e`
standing-reserve-duplicated, and `0x0f` standing-reserve-unaccounted. Tag `0x0b` is
unassigned: settlement failure is never a terminal degradation.

For every nonterminal export member except the decision-basis intent and decision basis,
its `recordSha256` is:

```text
recordSha256 =
  SHA-256("d2b:host-generation:continuity-compaction-export-record:v1\0" ||
  record_kind_tag_u8 || canonical_record_length_u32_be ||
  canonical_record_bytes)
```

The decision-basis-intent and decision-basis members' `recordSha256` values are exactly
`continuityRepairDecisionBasisIntentSha256` and
`continuityRepairDecisionBasisSha256`; the terminal member's `recordSha256` is exactly
`terminalOutcomeRecordSha256`. None receives a second hash. Export record tags are
closed: `0x01` source-acquisition pre,
`0x06` source-acquisition outcome, `0x07` replay-association pre,
`0x02` source-binding final, `0x03` source-binding outcome, `0x04` repair pre,
`0x05` mandatory-prune target,
`0x10` prune pre, `0x11` prune outcome, `0x12`
reduced census, `0x13` mandatory-prune proof, `0x20` repaired watermark, `0x2d`
decision-basis intent, `0x2e` decision basis, `0x2f` decision selection, `0x30`
settlement decision-pre, `0x31` outcome intent, `0x32`
terminal outcome, and `0x40` final absence proof.

`requiredExportSetSha256` is
`SHA-256("d2b:host-generation:continuity-compaction-export-set:v1\0" ||
record_count_u16_be || for each protocol-ordered record:
record_kind_tag_u8 || record_sha256)` over exactly the row-specific records above; the
count is two-byte big-endian, tags are one byte, digests are 32 bytes, and duplicates or
reordering refuse. Protocol order is source-acquisition pre, source-acquisition outcome,
replay-association pre, binding final, binding outcome, repair pre, optional mandatory
target, all prune records by durable prune-history ordinal with each pre
immediately followed by its outcome and then its reduced census when one committed,
optional mandatory proof, optional repaired watermark, decision-basis intent, decision
basis, decision selection, settlement decision-pre, outcome intent, terminal
outcome, and final absence proof. Decision-basis intent precedes decision basis, which
precedes decision selection; reversal, omission, or substitution is invalid. This semantic
order includes prune
history produced after a degraded terminal outcome; wall-clock append order is not
substituted.

After the governed set becomes empty, the broker publishes one immutable
`ContinuityGovernedSetFinalAbsenceProofV1`:

```text
continuityPruneHistorySha256 =
  SHA-256("d2b:host-generation:continuity-prune-history:v1\0" ||
  history_count_u16_be ||
  for each durable prune-history ordinal:
    continuity_repair_attempt_sha256 || prune_attempt_sha256 ||
    prune_outcome_sha256 || resulting_census_sha256)

continuityFinalAbsenceProofSha256 =
  SHA-256("d2b:host-generation:continuity-final-absence-proof:v1\0" ||
  retention_epoch_sha256 || terminal_continuity_repair_attempt_sha256 ||
  governed_set_initial_census_sha256 || continuity_prune_history_sha256 ||
  governed_set_final_census_sha256)
```

Each history entry contains four 32-byte members; the count is two-byte big-endian. The
history includes every failed, partial, and later successful prune attempt across repair
attempts, in durable ordinal order, and the final census must freshly canonicalize as empty
for the same epoch. Omitting the first, intermediate, or final history entry, substituting a
later successful outcome for an earlier failure, or citing a nonempty/mismatched census
refuses compaction.

The frozen prune-history/final-absence vector reuses the watermark vector's retention
epoch, `continuityRepairAttemptSha256`, initial census, two prune attempt inputs, two prune
outcomes, intermediate census, and empty final census. Both history entries use the same
repair attempt; the first uses prune attempt bytes `00` through `1f`,
`pruneOutcomeSha256 = 744a24684dfe6c0b06fff23a8d59e40a60b0ab663f49d908dfc71f97ae7c7b35`,
and resulting census
`0261cd8a73bb28cacb8c80b321f7a5da0fca264f3113995f3b3046d73b4416ac`.
The second uses prune attempt bytes `20` through `3f`,
`pruneOutcomeSha256 = 866dd015f616fd3e521e0ca079baf40bc692b9da87c88ea64c338d81bbf0399a`,
and resulting empty census
`147f09b8db794ac55a5fc12ef6176fcfb94ba7284749d9848b72fce522245afa`.
The literal expected lowercase outputs are:

| Identity | Expected bytes |
| --- | --- |
| `continuityPruneHistorySha256` | `fbc997ef229a71e3d1d58a03b28dac0ed9317cece4e29c0597d155b3e92ccf21` |
| `continuityFinalAbsenceProofSha256` | `a04626962bda75cc81c99513797e94daa52f3d3789a1890c0ecd7c1ea8f823a9` |

The literal table, independently authored canonical-byte fixture, and production formulas
are mutually read-independent. Negative vectors perturb both domains, the two-byte history
count and its framing, each of the four fields at the first and final ordinal, entry order,
retention epoch, terminal attempt, initial census, history digest, and final census. They
substitute every 32-byte digest into each other field, omit the first or final entry,
duplicate either entry, replace the first failure/success result with the later outcome,
and replace the empty final census with the intermediate census. One removal poison per
vector, perturbation, substitution, omission, duplication, order, and framing check is
mandatory.

`ContinuityGovernedSetFinalAbsenceProofV1` uses the common
`HostGenerationImmutablePublicationV1` protocol. Its publication prefix is exactly
`Absent | HierarchyDurable | InodeWritten | FileDurable | FinalLinked | FinalReopened |
ParentDurable | AncestorsDurable | Complete`. Fresh-process restart resumes the first
missing boundary, accepts only the exact no-replace final, preserves a nonidentical final as
conflict, and returns a stored completed response with zero write after response loss.
The existing `continuity-repair-outcome` record-boundary ids therefore run an independently
hooked final-absence proof publication family at all nine boundaries. The complete pinned
set is exactly `decision-basis`, `decision-selection`, `decision-pre`,
`exact-outcome-intent`, `terminal-outcome`, and `final-absence-proof`. The
`decision-basis` subvisitor
independently visits its write-ahead intent and basis final at the named boundary. The
fixture pins all six subvisitor names for every id; one cannot satisfy another. Independent hierarchy,
write, file-sync, link, reopen, parent-sync, ancestor-sync, conflict, fresh-process, and
completed-no-write removal poisons are mandatory.

The complete target set and current-head proof are:

```text
continuityCompactionTargetSetSha256 =
  SHA-256("d2b:host-generation:continuity-compaction-target-set:v1\0" ||
  target_count_u8 ||
  for each target in target-kind order:
    target_kind_tag_u8 || expected_leaf_record_sha256)

currentContinuityHeadProofSha256 =
  SHA-256("d2b:host-generation:continuity-current-head-proof:v1\0" ||
  retention_epoch_sha256 || current_head_sequence_u64_be ||
  current_head_attempt_sha256 || current_head_watermark_sha256 ||
  target_attempt_sha256 || head_disposition_tag_u8 ||
  optional_target_watermark_sha256 ||
  current_head_successor_proof_sha256)

currentHeadSuccessorProofSha256 =
  SHA-256("d2b:host-generation:continuity-head-successor-proof:v1\0" ||
  link_count_u16_be ||
  for each target-to-head link in sequence order:
    predecessor_attempt_sha256 || predecessor_watermark_sha256 ||
    successor_attempt_sha256 || successor_watermark_sha256 ||
    successor_sequence_u64_be)
```

Target tags are exactly `0x01` evidence, `0x02` source replay binding, `0x03`
superseded watermark, and `0x04` attempt census, with no duplicate or unknown target.
Evidence is present in the set only for `PresentExact`; the replay binding and attempt
census are always present. The watermark target is present only when the proof carries
head-disposition `0x03`. Head-disposition is exactly `0x01` degraded attempt with no
attempt watermark, `0x02` repaired attempt whose watermark is still current and therefore
preserved, or `0x03` repaired attempt whose named strict successor is current and whose
validated predecessor chain contains the target watermark. The optional target watermark
is absent for `0x01` and present for `0x02 | 0x03`; `0x02` requires it to equal
`currentHeadWatermarkSha256`, while `0x03` requires a greater current sequence and a
byte-exact predecessor link. Dispositions `0x01 | 0x02` require the canonical zero-link
successor proof; `0x03` requires a nonzero, gap-free chain whose first predecessor is the
target attempt/watermark and whose final successor is the selected current
attempt/watermark/sequence. Every intermediate record is reopened and digest-validated
under the coordinator lock. A current watermark is never an unlink target. The durable
`ContinuityCompactionSelectionV1` stores the complete canonical target entries, evidence
selection, current head record, and all successor-chain entries; the target/head digests
are only its audit projections. The proof is created and validated under the same
coordinator lock from the complete durable head and census, never from a caller or mutable
pointer.

After all selected targets have matching intents, parent durability, reduced censuses, and
receipts, the broker publishes the intermediate authorization receipt:

```text
ContinuityTargetsCompactedReceiptSha256 =
  SHA-256("d2b:host-generation:continuity-targets-compacted-receipt:v1\0" ||
  private_compaction_operation_id || continuity_compaction_selection_sha256 ||
  completed_target_receipt_set_sha256 || final_attempt_census_sha256 ||
  targets_compacted_outcome_tag_u8)

brokerTargetsCompactedSha256 =
  SHA-256("d2b:audit:host-generation:continuity-targets-compacted:v1\0" ||
  continuity_compaction_operation_sha256 ||
  continuity_targets_compacted_receipt_sha256)
```

The outcome tag is exactly `0x01` targets compacted. This receipt has no degraded variant:
only a pre-unlink `head-changed`, `target-changed`, or failed `unlink` while the selected
target remains present may settle the operation's degraded outcome. Every failure after a
successful unlink leaves the original operation and mutation intent pending through parent
durability, census, receipt, and later targets; it cannot mint a degraded outcome, residual
selection, or recovery generation. The receipt is immutable and authorizes only source
release for the same coordinator/source binding. It is not overall compaction completion.

After source release, the sealed
`ReleaseHostGenerationImmutableAuditContinuityAttemptCapacityV1` releases only that
attempt's pre-reserved slice. Its proof and identities are:

```text
ContinuityAttemptReservedSliceReleaseProofSha256 =
  SHA-256(
    "d2b:host-generation:continuity-attempt-reserved-slice-release-proof:v1\0" ||
    broker_targets_compacted_sha256 ||
    continuity_source_release_outcome_sha256 ||
    reservation_attempt_id || continuity_repair_attempt_sha256)

ContinuityAttemptReservedSliceReleaseAttemptIdV1 =
  SHA-256(
    "d2b:host-generation:continuity-attempt-reserved-slice-release-attempt:v1\0" ||
    coordinator_private_id || continuity_repair_attempt_sha256 ||
    continuity_attempt_reserved_slice_release_proof_sha256 ||
    reservation_attempt_id || prior_reservation_ledger_sha256)

continuityAttemptReservedSliceReleaseAttemptSha256 =
  SHA-256(
    "d2b:audit:host-generation:continuity-attempt-reserved-slice-release-attempt:v1\0" ||
    private_continuity_attempt_reserved_slice_release_attempt_id)

continuityAttemptReservedSliceAppliedLedgerSha256 =
  SHA-256(
    "d2b:host-generation:continuity-attempt-reserved-slice-applied-ledger:v1\0" ||
    prior_reservation_ledger_sha256 || reservation_attempt_id ||
    continuity_repair_attempt_sha256 || reservation_generation_u64_be ||
    released_record_count_u32_be || released_encoded_byte_count_u64_be ||
    broker_targets_compacted_sha256 ||
    continuity_source_release_outcome_sha256)

continuityAttemptReservedSliceReleaseOutcomeRecordSha256 =
  SHA-256(
    "d2b:host-generation:continuity-attempt-reserved-slice-release-outcome:v1\0" ||
    private_continuity_attempt_reserved_slice_release_attempt_id ||
    attempt_slice_release_outcome_tag_u8 || applied_ledger_sha256)

continuityAttemptReservedSliceReleaseOutcomeSha256 =
  SHA-256(
    "d2b:audit:host-generation:continuity-attempt-reserved-slice-release-outcome:v1\0" ||
    continuity_attempt_reserved_slice_release_attempt_sha256 ||
    continuity_attempt_reserved_slice_release_outcome_record_sha256)
```

Its private ledger entry binds the exact repair sequence, reserved record/byte slice,
reservation generation, targets-compacted receipt, source-release outcome, prior ledger,
and resulting ledger. The resulting ledger digest is exactly
`continuityAttemptReservedSliceAppliedLedgerSha256`; the record count is unsigned
four-byte big-endian, the encoded-byte count and generation are unsigned eight-byte
big-endian, and checked subtraction must reproduce the persisted resulting ledger before
`LedgerApplied`. The sealed release constructor also requires
`ContinuityCompactionLifecycleV1::TargetsCompacted`, which proves the exact ordered
reclamation of the one decision-intent witness and every durable target-unlink witness;
the targets-compacted receipt alone cannot bypass that state. Its prefix is exactly
`Absent | PreAudited | LedgerApplied |
CompletedReleased`. Its outcome is exactly `Released { appliedLedgerSha256 } | Degraded {
class }`, where class is `ledger-conflict | census | audit-publication |
standing-reserve-missing | standing-reserve-overdrawn |
standing-reserve-duplicated | standing-reserve-unaccounted`. No generic capacity-release
reason or backup-set proof can substitute. Before `CompletedReleased`, the slot is not
reusable. Restart replays the exact proof and ledger transition, never double-releases, and
an exact completed response is zero-write. Successful outcome tag `0x01` is released;
degradation has no successful outcome digest. `census` and `audit-publication` preserve the
same ledger-safe release prefix; after the named repair procedure, typed reconciliation
resumes only that operation and checked subtraction. `ledger-conflict` and every
standing-reserve corruption class are terminal integrity incidents for this operation:
they preserve the ledger and charged slice, admit no successor release identity, free no
slot, and authorize no retry until an accepted security disposition exists. The public
integrity action is preservation and escalation, not cleanup retry. These variants never
inhabit `ContinuityCleanupPendingV1` and never serialize with a pending heading, pending
error kind, or pending settlement field. They use the distinct
`ContinuityCapacityIntegrityIncidentV1` projection defined below and the exact
`audit-continuity-capacity-integrity-incident` operator shape in
`contracts/operator-cli.md`.

Each of the five terminal classes has an independent fresh-process negative. Restart from
the durable incident must return the same class while preserving the byte-identical prior
and resulting ledger observations and the full charged slice. For each class, separate
hooks prove zero continuation, zero retry, zero successor release identity, zero source
acquisition or source mutation, zero next-cleanup dispatch, and zero slot reuse. Each hook
has its own removal poison, so one terminal class or one generic incident test cannot
satisfy another. These are subvisitors of the existing
`lifecycle/continuity/evidence-retention-export-compaction-restart` id; they add no registry
id.

Overall completion is then:

```text
brokerCompactionCompletionSha256 =
  SHA-256("d2b:audit:host-generation:continuity-compaction-completion:v1\0" ||
  continuity_compaction_operation_sha256 ||
  broker_targets_compacted_sha256 ||
  continuity_source_release_outcome_sha256 ||
  continuity_attempt_reserved_slice_release_outcome_sha256 ||
  compaction_operation_kind_tag_u8 || compaction_outcome_tag_u8)
```

It exists only after targets compacted, source `Released`, and attempt-slice
`CompletedReleased`. Kind tags are `0x01` finalize, `0x02` degraded reclamation, and
`0x03` recovery; completion outcome tags are `0x01` compacted and `0x02` already
compacted. This final receipt authorizes completed no-write replay and slot reuse, never
source release.

The frozen compaction vector uses the continuity vector's `coordinatorPrivateId`,
`continuityRepairAttemptSha256`, and `repairedWatermarkSha256`, the empty
`governedSetFinalCensusSha256` from the whole-set vector, and terminal outcome
`RepairedBeforeDay90` with deadline-plan tag `0x01`. Its ordinary export entries use
one-byte canonical record bodies. The decision-basis intent, decision basis, and terminal
outcome use their named 32-byte hashes directly as specified above. Entries are listed as
`(tag, body or direct hash)` in protocol order:

```text
(0x01, 01), (0x06, 06), (0x07, 07), (0x02, 02), (0x03, 03), (0x04, 04),
(0x10, 10), (0x11, 11), (0x12, 12),
(0x10, 20), (0x11, 21), (0x12, 22),
(0x20, 20),
(0x2d, continuityRepairDecisionBasisIntentSha256),
(0x2e, continuityRepairDecisionBasisSha256), (0x2f, 2f),
(0x30, 30), (0x31, 31),
(0x32, terminalOutcomeRecordSha256), (0x40, 40)
```

The target/head inputs are:

```text
evidenceExpectedLeafRecordSha256 =
  000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f
sourceBindingExpectedLeafRecordSha256 =
  202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f
targetWatermarkSha256 =
  ad091f50421f6e5ac3b7a5a33dad976aea3387dcc0fdad6f94a620b625450984
attemptCensusExpectedLeafRecordSha256 =
  606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f
currentHeadSequence = 0x0102030405060709
currentHeadAttemptSha256 =
  a0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebf
currentHeadWatermarkSha256 =
  c0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcdddedf
headDispositionTag = 0x03
successorLinkCount = 1
successorLink =
  (continuityRepairAttemptSha256, targetWatermarkSha256,
   currentHeadAttemptSha256, currentHeadWatermarkSha256, currentHeadSequence)
currentHeadSuccessorProofSha256 =
  e8f5b9fbc0b0a7f6689dfd9ba20f91b4a58861419b2200322705e23ca22ee09a
```

The expected lowercase hex outputs are:

| Identity | Expected bytes |
| --- | --- |
| `terminalOutcomeRecordSha256` | `109cead12b62bfb733f730f3dbb5eb2e2ac7796f3cf2469bdee8e72d56590bd0` |
| `continuityRepairDecisionBasisIntentSha256` | `235834839696200373f01cb604c94af584f4e42e8f36af229c831d2c81b90073` |
| `continuityRepairDecisionBasisSha256` | `528e76ab349b3af38c98ecc172dc7d8edd9534a11badf6ee2d868d6e143c3643` |
| `requiredExportSetSha256` | `ff6bb3a5241f2e33050619e1eb3b5de081b4d6171dcf70d971c652c1cb9ac7bf` |
| `continuityCompactionTargetSetSha256` | `7745a72f7c4b5a0bb93fa2ee4927c01c8391e7332c7a89486d64d5c63f4a5176` |
| `currentHeadSuccessorProofSha256` | `e8f5b9fbc0b0a7f6689dfd9ba20f91b4a58861419b2200322705e23ca22ee09a` |
| `currentContinuityHeadProofSha256` | `4f2c9a953db4c0f55761f9159c3db9b3159e7f65317bf871f4c53cc6ead518b2` |
| `ContinuityCompactionAttemptIdV1` | `f92e1886de033878cc6da9bdd6e2b9a1fb038567b80c59c45fe9c67100c363f3` |
| `continuityCompactionAttemptSha256` | `77537a9622c785ad33d65ad556af4077d69488df0848bd13c5720349019ccda1` |
| finalize `continuityCompactionOperationSha256` | `fc3dd07786e6980aa7df6b0e7740ecd16d733a00c782d3da48f83e1f21c9356f` |

The independent reclamation/receipt/recovery vector reuses the same coordinator, repair,
terminal outcome, export set, four-target set, and current-head proof; uses the initial
governed census `b744729a18a0ebe8069f92a741147a3be5749450e250e07f24adb6801c54b284`;
selects degraded reclamation; and begins with the canonical empty receipt set. Target zero
uses evidence leaf bytes `00` through `1f` and `PresentExact`. All four targets use the
same typed parent vector:

```text
parentMountId = 0x0102030405060708
parentDeviceMajor = 0x11121314
parentDeviceMinor = 0x21222324
parentInode = 0x3031323334353637
parentUid = 0
parentGid = 0
parentMode = 0x000041c0
parentLinkCount = 2
anchoredParentIdentitySha256 =
  117bbff60a288347a4001aabcca3245f5d84d742bcca74f8c96bdf386a4bb4d2
```

`parentMode` is exactly directory type plus mode `0700`; every integer is encoded at the
fixed width in the formula above. Under the reclamation selection, target-zero removal
canonically leaves target kinds `0x02`, `0x03`, and `0x04` and derives resulting census
`271698a1efb3fb87d5bc82a4037adc6fe250b1ab812cbd11d4f3a926ff53e1ed`.
Its first outcome is degraded at target zero/evidence/unlink while that target is still
present: the completed receipt set is empty, no target-zero receipt exists, and the
residual set contains all four original targets. This is a pre-unlink failed call, not a
post-unlink recovery vector. The independent post-unlink vector resumes the original
reclamation operation from its durable target intent through parent sync, census, and
target-zero receipt, with no degraded outcome or recovery identity. Recovery generation
one uses fresh
head-proof bytes `d0` through `ef` and completes evidence, source binding, watermark, and
attempt-census targets in order. Under the recovery selection, the remaining ordered
target-kind sets after ordinals zero through three are respectively
`[0x02,0x03,0x04]`, `[0x03,0x04]`, `[0x04]`, and `[]`. The formula above derives every
resulting census and the final attempt census from those literal members; no census digest
is an independent input.
The literal lowercase outputs are:

| Identity | Expected bytes |
| --- | --- |
| `ContinuityDegradedAttemptReclamationIdV1` | `ecc685c33bdbb04693e646992e1d15b33bfa17a9f47d7ad35c61d9ddd6cf738c` |
| `continuityDegradedAttemptReclamationSha256` | `61b73fa4a3b7abfd8e7a577aff38e71a49c675765aa512b7fac6e77b2d35f88b` |
| reclamation `continuityCompactionOperationSha256` | `6f298b102c19295a06b1fd8b25f9945cd1851ea1cf8d63182628ea19cb4e56d0` |
| empty `completedTargetReceiptSetSha256` | `60d7841d7d3f9aaabe89daab174460f26e202b4a5d1f72cb0ad6c6425a71416b` |
| reclamation `continuityCompactionSelectionSha256` | `5ed3a97a71bd92cf09dd9db8acd5a663a05e58d0591c015e4236796a7ee8f70c` |
| `continuityCompactionSelectionAuditSha256` | `c7ff7a11a644b0203fb3e70874773782d491fc84b9105d9955cff811d9c285e2` |
| `continuityCompactionTargetSetAuditSha256` | `d99c8eb5dfd9d847ff55333c27d2dba9e62bea0976629426b90f96f3183b59b9` |
| `currentContinuityHeadProofAuditSha256` | `497ab35d429a676fd9c4860b22ed5a382be121f0df1041cd38ca670baf628556` |
| `requiredExportSetAuditSha256` | `2fcaab94f26530d6a04d1c49eca2740ee7e1627bd0032cb1949efaaf1c97429b` |
| `governedSetCensusAuditSha256` | `e57290fe380d0f69a3fbfc9b3dbfc5c0e88d133d17d91eb3bd9c44188e07b16d` |
| `terminalOutcomeRecordAuditSha256` | `87b7039b567ff190a1d71a348a7352624634a186e7afcc9e042a5f86c3dcf039` |
| `anchoredParentIdentitySha256` | `117bbff60a288347a4001aabcca3245f5d84d742bcca74f8c96bdf386a4bb4d2` |
| reclamation target-zero mutation intent | `5c5e5bcf22985ab23b4957f693414c129f7ea58ce182d9f2706633f3c82eb773` |
| reclamation target-zero `resultingCensusSha256` | `271698a1efb3fb87d5bc82a4037adc6fe250b1ab812cbd11d4f3a926ff53e1ed` |
| reclamation post-unlink target-zero receipt | `258a3cbcefe16de55ba9ccd075c7fe441255cc6248fc19566544c3c2e95f4336` |
| all-target `residualTargetSetSha256` | `f30362b2624b0fdadc233be59064142f734641410ea6cbda9167f1b7ecc39ade` |
| degraded `continuityCompactionOutcomeSha256` | `51796179fa529afe3cd82232acd75699e97fbacb207cbcf8fd7b14cf726e4579` |
| `continuityCompactionOutcomeAuditSha256` | `c3b468dfb7eee9d1ec7fdbbb36caa711d680615f6062f5f8e54191d1896492e9` |
| `ContinuityCompactionRecoveryGenerationIdV1` | `d2b6477cd3a2dba78b6614729a44c32229d1b17b6384ad0e8b0cb8f92d47c2a0` |
| `continuityCompactionRecoveryGenerationSha256` | `d28be14591e64a7125c3a4db340f9c56b69d60c1def74f9c32718926bb6b0d3f` |
| recovery `continuityCompactionOperationSha256` | `036d7f61bce8e1ed6a97d276ba91ec595a64abc02dea0c0e693eed29157a87fc` |
| recovery `continuityCompactionSelectionSha256` | `58eed8fdfad20d115c849cfa7d8c9550307c7497d6a9312a2d3c5899a0027e04` |
| recovery `continuityCompactionSelectionAuditSha256` | `8fa9702d810e3c6fe937cfea336c04599069b631e041f2c9325d118303f99b5a` |
| recovery target-zero `resultingCensusSha256` | `07aeb6afbac3c35c30d1b003f0c6d2e1f282584990b3ba45371962c647b49aa9` |
| recovery target-one `resultingCensusSha256` | `40c192da6a4b4aedaaaf28e2e36c26fa22e3edd73bd651a693e5bec71c1b3882` |
| recovery target-two `resultingCensusSha256` | `5274840cf7e5190ed78142fb56868c4b337518883db2cd62d4a33f5a35ac69a3` |
| recovery target-three `resultingCensusSha256` and `finalAttemptCensusSha256` | `401851540f947c73929a84ecf89d3d0fcc5f23e4e0e9fccd39b50a2f0ac9a240` |
| recovery target-zero mutation intent | `4bdba91a8b752caa42f079b22ba63bc266a32131b4a57b19d26f1e181ed58b3d` |
| recovery target-zero receipt | `f1fbb6eee6a76f6b8325f67b81f2c172121ca100b0990842b5f06da3efca0a1f` |
| target-one mutation intent | `8eddd1fda840918dfb1e3f569a39a99592ecbbcad14cfae8b1fc17df1d7268a9` |
| target-one receipt | `1d1c7a5860efb8632cc51eca5488c48313c47d909f8f44907313aac06f7dab94` |
| target-two mutation intent | `0bc389a3664942fbe1be5345169a1a1ec3a8b07d11e122a79e5a0595857f2dd5` |
| target-two receipt | `5eeecb9eae7fe678dcf0a07ae79453cc0f666c72835e3bb670434fef2f29a5c1` |
| target-three mutation intent | `bf5468b64be4c81240e483c68fa6fe683f4c18f90acb461d7a872582ddb6a013` |
| target-three receipt | `102530d413ce2a0c79cbf2a7142be147004f7a94a6111bc2e6d64c9ae191aa1e` |
| all-target `completedTargetReceiptSetSha256` | `0cc056397bb6d95ca2477cf7a1a18b975acf928f0e3ecea875422863b7eaff57` |
| `completedTargetReceiptSetAuditSha256` | `79589216f18917039d2402a27b071826cbc318ec5751d327274782eca7110627` |
| `residualTargetSetAuditSha256` | `c0c6f13911c63c750b23af0bbd4c570c942dc50480b8dbb42b134c84c53a69b4` |
| `ContinuityTargetsCompactedReceiptSha256` | `2fbe82eaf643f4db47788ac5491525161261d7dbc796e9f9e392d3f77df5f66a` |
| `brokerTargetsCompactedSha256` | `5f04253aa226aba438e4bc85e55041d8a581b56ccd0729161ccc9a3c5d7a6223` |

The same fixture consumes the literal source-release output above rather than arbitrary
bytes: `continuitySourceReleaseOutcomeSha256 =
303571fa69c60af7583cf6aee8574809211e172ac396f39c54dad7d856f32e88`.
It sets `reservationAttemptId` to bytes `c0` through `df`,
`priorReservationLedgerSha256` to bytes `e0` through `ff`,
`reservationGeneration = 0x2122232425262728`, `releasedRecordCount = 46`, and
`releasedEncodedByteCount = 222208`. The count and byte values are the literal sum of the
fixed per-attempt rows in `ContinuityReplacementFutureChargeV1`, including one
decision-intent witness and four target-unlink witness slots, not opaque fixture digests.
Its literal expected lowercase outputs are:

| Identity | Expected bytes |
| --- | --- |
| `ContinuityAttemptReservedSliceReleaseProofSha256` | `f71b5b9cc75b4e9bb8be0f0903a804f4a06639d917cc5c7ee63cdf156381fe7c` |
| `ContinuityAttemptReservedSliceReleaseAttemptIdV1` | `8dd9af88d878776ac53fe392ca61e0af353bd8afa8da6f4dfe48c52e52173c1c` |
| `continuityAttemptReservedSliceReleaseAttemptSha256` | `9a91ecbe1af38030392df4578fd09468b606cbf55e458a9f46a59f6dac2c497f` |
| `continuityAttemptReservedSliceAppliedLedgerSha256` | `fd92ae28f0ab38aa284a920ad732d5e9f5ddff226e16691b91682d86921ad8f0` |
| `continuityAttemptReservedSliceReleaseOutcomeRecordSha256` | `bf22a4c9cb9a05f3c03b884d5ad29c60b57cb16dd5b65cc47bb3eb4d295a791c` |
| `continuityAttemptReservedSliceReleaseOutcomeSha256` | `649ef5691e93d9ddbbfe3d8d0247d46770cae75c1a86adc76525251c36e4d205` |
| `brokerCompactionCompletionSha256` | `73141aa555544aff146205af9e926cb6d2a2fbd533e98cde3f8114603c581382` |

Independent recalculation of the changed witness-inclusive release and downstream
completion chain, together with its parent/census predecessors, is a release obligation, not
an assertion inferred from either table. The recalculator must take exactly these literal
inputs:

```text
continuityRepairAttemptSha256 =
  0e7225df2517e961b428335533cd4cd1cb2a3e4db5999d0c46b89da8b6fdc0a0
reclamationPrivateOperationId =
  ecc685c33bdbb04693e646992e1d15b33bfa17a9f47d7ad35c61d9ddd6cf738c
reclamationSelectionSha256 =
  5ed3a97a71bd92cf09dd9db8acd5a663a05e58d0591c015e4236796a7ee8f70c
recoveryPrivateOperationId =
  d2b6477cd3a2dba78b6614729a44c32229d1b17b6384ad0e8b0cb8f92d47c2a0
recoveryOperationSha256 =
  036d7f61bce8e1ed6a97d276ba91ec595a64abc02dea0c0e693eed29157a87fc
recoverySelectionSha256 =
  58eed8fdfad20d115c849cfa7d8c9550307c7497d6a9312a2d3c5899a0027e04
parentTuple =
  (0x0102030405060708, 0x11121314, 0x21222324,
   0x3031323334353637, 0, 0, 0x000041c0, 2)
orderedTargets =
  (0x01, 000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f)
  (0x02, 202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f)
  (0x03, ad091f50421f6e5ac3b7a5a33dad976aea3387dcc0fdad6f94a620b625450984)
  (0x04, 606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f)
coordinatorPrivateId =
  808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9f
continuitySourceReleaseOutcomeSha256 =
  303571fa69c60af7583cf6aee8574809211e172ac396f39c54dad7d856f32e88
reservationAttemptId =
  c0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcdddedf
priorReservationLedgerSha256 =
  e0e1e2e3e4e5e6e7e8e9eaebecedeeeff0f1f2f3f4f5f6f7f8f9fafbfcfdfeff
reservationGeneration = 0x2122232425262728
releasedRecordCount = 46
releasedEncodedByteCount = 222208
```

It must independently encode the typed parent tuple, reclamation remaining set
`[0x02,0x03,0x04]`, and recovery remaining sets
`[0x02,0x03,0x04]`, `[0x03,0x04]`, `[0x04]`, and `[]`; then derive, rather than accept,
the parent hash, all five census hashes, all five mutation intents and receipts, the
four-receipt set, targets-compacted receipt/audit projection, attempt-slice release chain,
and final completion. Every derived byte string must equal the corresponding literal
expected output in the two tables above. The recalculator, the canonical-byte fixture, and
production must share no hash helper, encoder, generated expected value, or imported
constant other than these printed literals. Validation is incomplete unless the
recalculator prints one input/preimage/result row for every named identity and the
integrator runs
`(cd packages/d2b-priv-broker && cargo test --locked --test
host_generation_coordinator_v5
continuity_compaction_parent_census_downstream_vectors_recalculate -- --exact --nocapture)`
and records that successful command output in panel evidence outside this feature root.
The named function must remain a root test in that exact integration target, and the
captured output must contain `running 1 test` plus a successful result with exactly
`1 passed`; zero executed tests or a same-named test in another target is failure. A
changed formula or expected output requires a fresh complete recalculation and replacement
evidence from this exact command.

One literal expected-value table, one separately authored canonical-byte fixture, and
production are mutually read-independent. Negative vectors perturb each private/audit
domain, terminal outcome tag and variant member, deadline-plan tag/target pairing, export
record tag, record length framing, record body, record count, target count/tag/member, head
sequence/disposition/member, successor count/link/member, and every ordering boundary one
at a time. They substitute
each digest into every other named field and remove, duplicate, or reorder the first,
intermediate, and final export/target/successor member. One removal poison per vector,
perturbation, substitution, framing check, and order check must fail.

The reclamation/recovery fixture separately perturbs every new private/audit domain,
operation kind, outcome/failure/observation/absence tag, ordinal, count and integer framing,
selection/census/export/target/head/intent/receipt/residual/prior-outcome member, recovery
generation, released-source census counts, source-release record and outcome, reservation,
reservation generation, released record/byte counts, prior/applied ledger, slice-release
record and outcome, and final completion. It substitutes every 32-byte digest into every
other position, including source-release and attempt-slice outcomes in each downstream
proof/completion field, swaps reclamation and recovery operation projections,
removes/reorders/duplicates the first, intermediate, and final receipt or residual target,
and poisons each check independently. Independent anchored-parent vectors perturb the
domain, mount id, device major/minor, inode, uid, gid, complete mode bits, link count, every
fixed-width big-endian encoding, and field order. Independent census vectors perturb the
domain, repair attempt, selection, one-byte count, every target kind/digest, empty-census
framing, order, omission, duplication, and the `resultingCensusSha256` versus
`finalAttemptCensusSha256` alias. No arbitrary parent, census, or downstream digest bytes
are accepted as vector inputs. The literal fixture, expected table, and production
implementation are mutually read-independent; one removal poison per vector, perturbation,
substitution, framing, order, mode-tag, source-release chain, and ledger-release chain check is
mandatory.

The operation publishes fixed redacted
`coordinator-immutable-audit-continuity-compaction/pre-mutation` audit before unlink. It
contains only the fixed edge id, operation-kind tag,
`continuityCompactionOperationSha256`, `continuityRepairAttemptSha256`, terminal outcome
tag, `terminalOutcomeRecordAuditSha256`, `governedSetCensusAuditSha256`,
`requiredExportSetAuditSha256`, and
`continuityCompactionSelectionAuditSha256`. It never contains
`expectedLeafRecordSha256`, exact `ContinuityEvidenceSelectionV1`, a raw target/head/export
digest, or any complete selected record. The durable private pre record additionally stores the
complete immutable selection: mode, every target's kind/expected leaf identity/digest,
selected evidence state, full current-head record, full successor chain, export set, and
prior completed-target receipts. The matching outcome repeats the audit projections and
adds exactly `Compacted | AlreadyCompacted | Degraded { targetOrdinal, targetKind,
failure, completedTargetReceiptSetAuditSha256, residualTargetSetAuditSha256 }`, where failure is
exactly `head-changed | target-changed | unlink`, plus
`continuityCompactionOutcomeAuditSha256`. `head-changed` and `target-changed` occur before
mutation intent; `unlink` requires the selected target still present exact after the failed
call. A storage, `census`, `conflict`, or `audit-publication` error after successful unlink is
instead a typed cleanup-pending response carrying the original operation and
durable target prefix; it may not publish a degraded compaction outcome. Neither audit record
stores or accepts a transient presence/absence observation.

Immediately before every target mutation, the operation revalidates the complete target set
and current head from held root/coordinator descriptors, including each expected target
leaf, the current head record, and every predecessor/successor link. For each target in
target-kind order it fd-relatively reopens the exact expected final, revalidates identity
and digest, and durably publishes
`ContinuityCompactionTargetMutationIntentV1` binding the tagged private operation,
immutable selection digest, target ordinal/kind/expected leaf, exact `PresentExact`
pre-unlink observation, and anchored-parent identity. Only then may it call `unlinkat` on
one validated component, reopen and identity-revalidate its anchored parent, and call
`fsync` on that held parent. After that sync it publishes a private no-replace
`ContinuityCompactionTargetUnlinkCommitWitnessV1` in a coordinator-owned namespace
independent of the selected target leaf. The witness binds the exact operation, selection,
ordinal, target kind, mutation-intent digest, `AbsentAfterSelectedUnlink` observation, and
anchored-parent identity. It is one fixed record of at most 2,048 encoded bytes. Its final
and complete directory chain must be durable before it can be consumed. Every reopen
validates the complete tuple, the exact predecessor mutation intent, and the canonical
witness-record digest; a foreign record or a same-width substitution of operation,
selection, ordinal, mutation intent, anchored parent, or record digest cannot satisfy the
witness. It is a sealed recovery-classification capability, not an added
audit/export/receipt preimage member, so every frozen receipt and downstream hash remains
unchanged. The receipt constructor consumes the matching witness as well as the intent.
Before that witness is durable, a fresh process that sees the exact selected target present
revalidates the complete selection and retries only the same `unlinkat` idempotently; if it
sees absence, it repeats parent revalidation/sync and witness publication. Only a matching
durable witness makes later presence of that selected target a `target-changed`
reappearance. The broker then publishes the per-target receipt binding the same intent,
operation, selection, target, post-unlink absence, parent identity and durability, and
resulting census before advancing. An already absent target before receipt is accepted
only under that exact durable mutation intent; it still requires absence revalidation,
parent revalidation/sync, witness publication, census commit, and receipt publication.
Absence without the intent, under another operation/selection/ordinal, or before intent
durability is integrity degradation. Before retrying or publishing a missing witness, a
fresh process enumerates the complete attempt-local successor chain. No durable successor
permits the exact same-intent retry above. An exact target receipt is the only immediate
successor proof that may dominate an absent witness: it must byte-for-byte bind the same
operation, selection, ordinal, target kind, intent digest, parent identity, post-unlink
absence, and resulting census. In that case restart resumes from the receipt and never
recreates the witness. A later receipt or downstream prefix without that exact immediate
receipt is not proof and is the closed witness-integrity incident below. A present
nonidentical witness is never dominated by a receipt and never treated as absence.
After the unlink witness is durable and before census
or receipt publication, the broker recomputes
the complete `AnchoredParentIdentityV1` from the reopened held parent and recomputes every
remaining `ContinuityCompactionCensusTargetV1` from independently held leaf descriptors.
A changed parent member derives `conflict`; a changed, missing, substituted, or extra
remaining target derives `target-changed`. Either class keeps the original post-unlink
operation pending, publishes no census or receipt, performs no next unlink, and cannot
publish a degraded outcome, residual selection, recovery generation, targets-compacted
receipt, source release, or settlement.
The
operation revalidates the complete immutable target selection and complete current head
again after each committed target, before every next unlink, and immediately before the
final census/outcome commit. A head advance, predecessor-link change, target substitution,
or change to a not-yet-removed target stops before the next mutation with the exact closed
class; it never broadens the selection or authorizes deletion of a now-current watermark.

`ContinuityCompactionLifecycleV1` and its restart classifier are exactly `PreOnly |
Target { ordinal, SelectionValidated } |
Target { ordinal, MutationIntentDurable } | Target {
ordinal, AbsentUnderIntentParentUnconfirmed } | Target {
ordinal, UnlinkCommitWitnessDurableOldCensus } | Target {
ordinal, CensusCommittedReceiptPending } | Target { ordinal, ReceiptCommitted } |
TargetsCompactedReceiptPending | WitnessReclamationPending { nextWitness } |
TargetsCompacted | SourceReleasePending | AttemptSliceReleasePending | Complete`.
`SelectionValidated` requires the target present and may only publish its mutation intent.
`MutationIntentDurable` means the no-replace
`ContinuityCompactionTargetMutationIntentV1` final has been exactly reopened and its
complete directory chain is durable; an inode write, file sync, or link alone cannot enter
that state. It revalidates the target; present permits the one unlink, while absent permits
only `AbsentUnderIntentParentUnconfirmed` for that exact operation, selection, ordinal,
expected leaf, and durable intent.
`AbsentUnderIntentParentUnconfirmed` always performs the anchored parent revalidation and
sync, then publishes the independent unlink commit witness; it never infers durability
from absence. While that witness is absent, exact target presence retries the same unlink
instead of deriving `target-changed`, and exact absence repeats parent sync and witness
publication. `UnlinkCommitWitnessDurableOldCensus` requires the exact witness final and
complete directory chain plus fresh absence revalidation; only this state may classify
selected-target reappearance or commit the reduced census. A crash before intent retries
only intent publication, a crash after intent but before the witness revalidates and
idempotently unlinks only that target or repeats its parent sync and witness publication, a
crash after the witness commits only that target's census reduction, and a crash after
census commits the same target receipt. The receipt constructor consumes the matching
sealed witness and records the exact durable intent digest; no receipt can be constructed
from target absence alone.
Pre-receipt absence without that intent, with a merely linked but not directory-durable
intent, or under another operation/selection/ordinal is an integrity incident with zero
census or receipt mutation. A missing witness under the exact durable intent resumes the
same unlink/parent-sync/witness sequence only when the complete successor census proves
that no target receipt or later attempt prefix exists. Restart advances across a target
only with either its exact intent, witness, and receipt or its byte-identical immediate
receipt successor proof; it never uses a later prefix to bridge a missing or mismatched
predecessor.

Missing-witness precedence is closed. With no reduced census, receipt, or later attempt
prefix, the exact durable intent permits only the same-intent retry above. A durable
reduced census without its exact immediate receipt is
`ImmediateReceiptMissing`. An exact immediate receipt is the only proof-carrying
successor and resumes from that receipt; every later receipt, targets-compacted receipt,
reclamation prefix, release prefix, or completed prefix must retain that immediate
receipt byte-for-byte. If it does not, absence is `ImmediateReceiptMissing`; if its
predecessor differs, absence is `ImmediateReceiptPredecessorMismatch`. A present
nonidentical witness is always `WitnessMismatch` before successor consideration. Thus
every downstream census or receipt prefix either carries the one admitted successor proof
or fails closed with a typed incident; none falls back to witness publication, target
retry, or operation reselection.

The closed classification for a witness or predecessor that cannot satisfy either rule is:

```text
ContinuityCompactionWitnessIntegrityIncidentV1 =
    WitnessMismatch { ordinal, member }
  | ImmediateReceiptMissing { ordinal }
  | ImmediateReceiptPredecessorMismatch { ordinal }
```

`member` is exactly `operation | selection | ordinal | intent | parent | digest`. Every
variant poisons the attempt, preserves the original operation and every existing byte,
returns the fixed integrity action
`preserve-and-escalate-audit-integrity-incident`, and performs zero unlink, witness
publication, census, receipt, next-target, outcome, recovery-generation,
targets-compacted, source-release, attempt-slice-release, settlement, or slot-reuse
mutation. It has no retry or successor constructor.

Its public projection is total, terminal, and identifier-free. The response constructor
consumes only the sealed incident and derives exactly this closed failure class:

| Sealed incident | Public failure class |
| --- | --- |
| `WitnessMismatch { member: operation, .. }` | `witness-operation-mismatch` |
| `WitnessMismatch { member: selection, .. }` | `witness-selection-mismatch` |
| `WitnessMismatch { member: ordinal, .. }` | `witness-ordinal-mismatch` |
| `WitnessMismatch { member: intent, .. }` | `witness-intent-mismatch` |
| `WitnessMismatch { member: parent, .. }` | `witness-parent-mismatch` |
| `WitnessMismatch { member: digest, .. }` | `witness-digest-mismatch` |
| `ImmediateReceiptMissing { .. }` | `immediate-receipt-missing` |
| `ImmediateReceiptPredecessorMismatch { .. }` | `immediate-receipt-predecessor-mismatch` |

No constructor accepts a failure class, action, ordinal, member, operation, selection,
intent, parent, digest, or stage from a caller. The exact exit-`4` human and JSON response
is `audit-continuity-compaction-witness-integrity-incident` in
`contracts/operator-cli.md`; none of these variants is cleanup pending or wire-valid in
the cleanup-pending shape.

After the last target receipt the broker publishes the immutable
`ContinuityTargetsCompactedReceiptV1` and enters `WitnessReclamationPending`. Witness
reclamation is bounded to at most five attempt-local names in this order: the one
decision-basis-intent commit witness, then one target-unlink witness for each selected
target ordinal, at most zero through three.
Before each unlink, the broker reopens and validates the exact witness and its exact durable
successor - the byte-identical basis/downstream decision chain for the decision witness and
the matching target receipt for an unlink witness - and obtains a `PresentExact`
observation of the current witness. The unlink is unreachable unless that current witness
and successor are both present and exact. A missing witness under that exact
successor is already reclaimed and advances to the next name. A present foreign or
field-mismatched witness, or any missing/mismatched immediate successor, returns the
applicable closed integrity incident with zero mutation. An exact witness is unlinked
fd-relatively, its held coordinator parent is identity-revalidated and synced, and only
then may the next name be considered. A crash before unlink retries the same name; a crash
after unlink but before parent sync repeats parent revalidation and sync; a fresh process
observing absence advances only from the same exact successor proof. No new durable record,
registry id, receipt member, or hash input is introduced.

Only after every selected name is absent under its exact successor does the lifecycle enter
`TargetsCompacted`, then resume source release and attempt-slice release in that order.
`TargetsCompacted` is the only state that can mint
`ContinuitySourceReleasePermit<'coordinator>`; `Complete` is unavailable at that point and
is not a source-release prerequisite. `SourceReleasePending` advances only to
`AttemptSliceReleasePending` after the exact released-source outcome, and
`AttemptSliceReleasePending` advances only to `Complete` after
`CompletedReleased`. No earlier state can construct the final completion receipt or free
the live slot.
Response loss after `Complete` returns the stored result with zero write. Failure
preserves every not-yet-removed target and every confirmed prior absence, blocks later
mutation, and never weakens any capacity ceiling. A settled degraded outcome contains
exactly the receipt prefix before the still-present failed target, and its residual set
begins at that target. Only such a pre-unlink settled outcome may admit the recovery
generation defined above; repeating it is a no-write replay. After a successful unlink,
the target is absent only under its original durable mutation intent, is not yet part of
the completed receipt set, and no residual selection or recovery generation may be
published. Remediation and every fresh-process restart resume the original tagged
operation through parent revalidation/sync, census, receipt, later targets,
`TargetsCompacted`, source release, attempt-slice release, and `Complete`.

The read-independent lifecycle case has separate hooks and removal poisons for missing and
mismatched pre export, repaired missing and mismatched watermark export, degraded unexpected
watermark, missing and mismatched outcome export, missing and mismatched mandatory-prune
export, first/intermediate/final partial-prune export, reduced census, final absence proof,
file-only export, incomplete repair, governed-set-present finalization refusal and
degraded-reclamation preservation, each repaired and degraded
admission row including degraded-after-evidence with zero watermark, malformed replay-key
state, illicit missing or mismatched evidence, target-set omission/reorder/substitution,
current-head absence/substitution/stale-successor/predecessor-chain change/current-watermark
deletion, compaction
pre/outcome publication, each target present, each target absent with old census, each
target intent, unlink, and unlink commit witness, both per-target anchored-parent
revalidations, every parent sync, every per-target census and receipt commit, every
fresh-process target/ordinal prefix,
targets-compacted receipt, all selected ordered witness-reclamation names and their
unlink/parent-sync prefixes, source release, attempt-slice release, and completed no-write
replay. For each ordinal, independent post-unlink hierarchy/write/file-sync/link/reopen,
directory-sync/census/conflict, witness-publication, receipt-publication, and
outcome-audit-publication injections restart a fresh process on
the original operation and complete its parent, census, and receipt prefix without
publishing a degraded outcome or recovery generation. Companion pre-unlink
intent-write, intent-file-sync, intent-link, intent-reopen, and
intent-directory-durability crashes prove target absence is rejected until the exact
intent is complete; separate post-intent/pre-receipt absence cases prove that only the
matching intent resumes parent sync, witness, census, and receipt. For every ordinal,
independent fresh-process witness-substitution negatives run before census, before receipt,
before every later target, before the targets-compacted receipt, and before ordered witness
reclamation. They separately substitute a foreign witness and each of the six closed
members `operation`, `selection`, `ordinal`, `intent`, `parent`, and `digest`. Every case
must return `ContinuityCompactionWitnessIntegrityIncidentV1`, poison the attempt, preserve
the substituted bytes, and perform zero unlink, witness publication, census, receipt,
next-target, outcome, recovery-generation, targets-compacted, witness-reclamation,
source-release, attempt-slice-release, settlement, or slot-reuse mutation. Every
ordinal/member/consumer/forbidden-effect assertion has its own hook and removal poison; no
field substitution, consumer prefix, or ordinal may satisfy another.

For every ordinal, independent fresh-process witness-loss cases run after each consuming
prefix: reduced census, matching receipt, every later receipt prefix, and the
targets-compacted receipt. With the exact immediate target receipt, absence resumes only
from that receipt and never republishes the witness. A descendant without the exact
immediate receipt returns `ImmediateReceiptMissing`; a receipt whose predecessor differs
in any field returns `ImmediateReceiptPredecessorMismatch`. Both are terminal with zero
mutation. The reduced-census-only case independently proves that a durable census is not
a proof-carrying successor. The no-successor pre-census control
instead resumes only the original same-intent parent-sync/witness sequence. These three
classifications have independent hooks and poisons. For every ordinal,
independent fresh-process cases reinsert the exact selected target after successful unlink
but before the matching witness is durable. They must produce the same recovery as
crash-before-unlink: revalidate the complete selection, retry only the exact unlink
idempotently, publish the witness, and continue, with no `target-changed` classification,
new operation, residual selection, or recovery generation. Every no-witness retry,
witness-publication boundary, forbidden alternate classification, hook, and removal poison
is independent. Companion pre-unlink
`head-changed`, `target-changed`, and failed-`unlink` cases prove the failed target remains
present, the completed receipt set is the exact prior ordinal prefix, and the residual set
begins at that target before recovery is admitted. Every completed-receipt omission,
extra receipt, residual-prefix overlap/gap, foreign intent, and post-unlink recovery
substitution has an independent poison. For every ordinal, two additional fd-backed
fresh-process negative families run after the matching unlink commit witness is durable
and before census or receipt.
One changes each parent identity member in turn through a replacement parent descriptor.
The other changes each still-selected remaining target's type, identity, or canonical
digest through its held leaf descriptor and also contains two literal cases for every
ordinal: `post-unlink/<ordinal>/namespace-missing` makes the canonical namespace
observation incomplete, including omission of the required empty-census observation at
the final ordinal, and `post-unlink/<ordinal>/namespace-extra` injects one unselected
namespace member. An additional fd-backed selected-target reappearance matrix starts a
fresh process after the matching unlink commit witness is durable and before census or
receipt. At every ordinal, one case reinserts the just-unlinked selected target through an
independently held leaf descriptor before restart. At every ordinal with a durable prior
receipt prefix, separate cases reinsert each receipted selected target through its
independently held leaf descriptor before restart; an unselected namespace member cannot
satisfy either family. Every missing, extra, or witness-backed reappearance case derives
`target-changed`, stops at the named
ordinal with the prior receipt prefix intact, publishes no current receipt, performs no
next unlink, and publishes no census, outcome, recovery, targets-compacted, source-release,
or settlement record. Every ordinal, mutation member, missing/extra case, reappearing
selected target, forbidden-effect assertion, and poisoning check has an independent hook
and removal poison; a just-unlinked case cannot satisfy a receipted-target case or vice
versa.
An independent post-receipt matrix starts a fresh process immediately after every durable
target receipt and before the next target unlink, or after the final target receipt and
before `TargetsCompacted`. For each such receipt prefix, separate cases reinsert every
receipted selected target through its independently held leaf descriptor. Each case derives
`target-changed`, preserves every receipt byte-for-byte, and performs no unlink, census,
receipt, outcome, residual-selection, recovery-generation, targets-compacted,
source-release, attempt-slice-release, or settlement effect. Every
receipt-prefix/receipted-target/forbidden-effect assertion has an independent hook and
removal poison; no post-unlink/pre-census case can satisfy a post-receipt case. These
matrices are subcases of the existing compaction visitor and add no registry id. A source
`Released`/attempt-slice-pending case proves that no slot, reserved byte,
repair sequence, source acquisition, or next cleanup operation changes before the exact
slice-release outcome; the same fixture succeeds only after that final prerequisite. A
bounded witness-reclamation matrix uses the maximum four-target attempt and restarts a
fresh process before and after each of its five ordered unlinks and parent syncs. It proves
an exact successor is mandatory,
missing under that successor is accepted as already reclaimed, mismatch is an integrity
incident, no source release begins early, and the fixed 46-record/222,208-byte slice and
live slot remain charged until source `Released` and attempt-slice `CompletedReleased`.
Four separate target-cardinality probes use exactly `0`, `1`, `2`, and `3` targets, with
independent record-bound and byte-bound general-capacity saturation. Each runs in a fresh
process at attempt admission, targets compacted, every applicable witness-reclamation
prefix, source `Released`, and the boundaries immediately before and after attempt-slice
`CompletedReleased`. Read-literal ledger and exact-bound unrelated general-capacity
admissions prove that all 46 records and 222,208 bytes remain charged before that exact
release despite unused witness slots, that exactly the complete slice rather than a
cardinality-dependent subset returns afterward, and that completed replay returns no
second credit. Every cardinality, record/byte boundary, restart, premature-credit
assertion, hook, and removal poison is independent of the maximum four-target matrix.
Their literal hook namespace is
`continuity-reserve/cardinality/<0|1|2|3>/<records|bytes>/<checkpoint>`, where
`<checkpoint>` separately names attempt admission, targets compacted, each applicable
reclamation name before unlink, after unlink before parent sync, and after parent sync,
source released, slice release pending, slice completed released, and complete replay.
Each literal hook has independent missing-visit, premature-credit, wrong-credit, and
second-credit poisons.
Independent fresh processes also restart after source `Released`, attempt-slice
`CompletedReleased`, and `Complete`; each must reconstruct authorized witness absence from
the exact targets-compacted receipt, complete immediate-receipt chain, and ordered
coordinator-parent absences. Removing or changing any immediate successor at any of those
prefixes is an integrity incident with zero mutation, not permission to recreate a witness
or reselect an operation.
Each name, crash boundary, successor check, forbidden early effect, and removal poison is
independent. A
recovery-recycling matrix independently withholds prior outcome export, first/intermediate/
final target intent or receipt export, and residual-selection supersession. Every withheld
prerequisite preserves the old recovery prefix and performs zero unlink, census, generation,
or selection mutation; recycling succeeds only after all are durable. A dedicated
interleaving matrix advances
the current head, changes one predecessor link, and substitutes every not-yet-removed
target after each completed ordinal and immediately before final commit. Every case must
stop with the prior receipts and absences intact, no next unlink, no final success, and one
named poison. Restoring the selected state resumes only the original tagged operation from
its first missing validation or final commit; it cannot publish a degraded outcome,
residual selection, or recovery generation after any successful unlink. Only a complete
matching export, applicable mode prerequisite, target set,
successor-bound current-head proof, and prior receipt set reaches mutation.

For `ContinuityRepairDeadlinePlanV1::Day90Reached`, the same permit drives every member in
the pre-bound target through its own prune pre/outcome chain in target order and proves the
complete set absent before the watermark advances. Each successful member outcome carries
the canonical `pruneOutcomeSha256` defined below. The whole-set proof is:

```text
mandatoryPruneProofSha256 =
  SHA-256("d2b:audit:host-generation:continuity-mandatory-prune-proof:v1\0" ||
  mandatory_prune_target_sha256 || governed_set_initial_census_sha256 ||
  outcome_count_u16_be ||
  for each target-ordered outcome:
    0x01 || backup_member_sha256 || prune_outcome_sha256 ||
  governed_set_final_census_sha256 || completion_tag_u8)
```

The count is two-byte big-endian, each member and outcome digest is 32 bytes, the member tag
is `0x01`, and `completionTag` is exactly `0x01` for complete. A valid proof has the same
nonzero count and members as the target, one `Pruned | AlreadyPruned` outcome per member,
and `governedSetFinalCensusSha256` equal to a fresh canonical empty-census digest for the
same epoch. Partial, duplicate, reordered, foreign-target, foreign-epoch, wrong initial
census, missing member, extra member, degraded member outcome, nonempty final census, or
wrong completion tag refuses. A `BeforeDay90` attempt may not consume or cite any mandatory
proof.

Restart reconstructs the target and ordered proof from the durable initial census and exact
member outcome records. It resumes the first member without a completed matching outcome,
never re-prunes a completed member, and cannot advance the watermark until the aggregate
proof and a fresh empty census agree. Independent one-member, two-member, and 256-member
cases crash before and after every member pre, unlink, parent sync, census, and outcome;
multi-member reorder, omission, duplication, foreign outcome, intermediate-census, and
nonempty-final negatives each have a removal poison.

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

The watermark and its audit projection have these exact preimages:

```text
watermarkSha256 =
  SHA-256("d2b:host-generation:immutable-audit-continuity-watermark:v1\0" ||
  schema_version_u32_be ||
  kind_length_u16_be || kind_utf8 ||
  continuity_repair_attempt_sha256 || retention_epoch_sha256 ||
  prior_watermark_sha256 || authoritative_evidence_sha256 ||
  continuity_repair_sequence_u64_be ||
  greatest_realtime_seconds_u64_be ||
  greatest_boot_time_nanoseconds_u64_be || boot_id_sha256)

repairedWatermarkSha256 =
  SHA-256("d2b:audit:host-generation:continuity-repaired-watermark:v1\0" ||
  continuity_repair_attempt_sha256 || watermark_sha256)
```

`schemaVersion` is four-byte big-endian `1`; `kind` is the 52 UTF-8 bytes of
`host-generation-immutable-audit-continuity-watermark` preceded by unsigned two-byte
big-endian `52`; the sequence and both clock scalars are unsigned eight-byte big-endian
integers; all remaining members are 32 bytes. `watermarkSha256` and
`repairedWatermarkSha256` are excluded from their own preimages. No serializer output,
outcome tag, prune proof, path, or caller field enters either formula. The repaired digest
must bind the exact no-replace watermark final for the same attempt.

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

ContinuityRepairTerminalPublicationFailureClassV1 =
    hierarchy
  | write
  | file-sync
  | link
  | reopen
  | directory-sync
  | conflict
  | audit-publication

ContinuityRepairTerminalRetentionFailureClassV1 =
    clock-rollback
  | clock-watermark
  | epoch-invalid
  | clock-forward-discontinuity
  | clock-continuity-ambiguous
  | clock-overflow
  | unlink
  | directory-sync
  | census
  | audit-publication
  | standing-reserve-missing
  | standing-reserve-overdrawn
  | standing-reserve-duplicated
  | standing-reserve-unaccounted

ContinuityRepairTerminalFailureV1 =
    Source(ContinuityRepairSourceFailureClassV1)
  | Retention(ContinuityRepairTerminalRetentionFailureClassV1)
  | Publication(ContinuityRepairTerminalPublicationFailureClassV1)
```

Settlement preparation and publication failures are a disjoint algebra:

```text
ContinuityRepairSettlementPublicationBoundaryV1 =
    hierarchy
  | write
  | file-sync
  | link
  | reopen
  | directory-sync
  | conflict
  | audit-publication

ContinuityRepairSettlementV1 =
    PendingDecisionBasis {
      state: ContinuityRepairDecisionBasisPublicationStateV1
    }
  | PendingDecisionSelection {
      state: ContinuityRepairDecisionSelectionStateV1
    }
  | PreparationIncomplete {
      intendedOutcome: ContinuityRepairDecisionSelectionV1,
      stage: DecisionPreAudit,
      failure: audit-publication
    }
  | PendingIntentPublication {
      intendedOutcome: ContinuityRepairDecisionSelectionV1,
      state: ContinuityRepairOutcomeIntentPublicationStateV1
    }
  | PendingTerminalPublication {
      intendedOutcome: ContinuityRepairOutcomeIntentV1,
      state: ContinuityRepairTerminalOutcomePublicationStateV1
    }
```

No settlement variant converts to, is embedded by, or implements a conversion into
`ContinuityRepairTerminalFailureV1`. `pending-settlement`, intent publication, and terminal
outcome publication are not terminal retention or publication classes.
`ContinuityRepairDecisionBasisIntentIncompletePrefixV1`,
`ContinuityRepairDecisionBasisIncompletePrefixV1`,
`ContinuityRepairDecisionSelectionIncompletePrefixV1`,
`ContinuityRepairOutcomeIntentIncompletePrefixV1`, and
`ContinuityRepairTerminalOutcomeIncompletePrefixV1` are each exactly `Absent |
HierarchyDurable | InodeWritten | FileDurable | FinalLinked | FinalReopened |
ParentDurable | AncestorsDurable`. Neither `Conflict` nor `Complete` is a progress prefix.
`Complete` transitions to the next state or completed no-write replay; `Conflict` exists
only as the sealed publication-state variant below.

Every settlement publisher uses the same closed algebra:

```text
ContinuityRepairDecisionBasisIntentPublishingV1 =
    Progress {
      predecessor: ContinuityRepairDecisionCandidatePredecessorV1,
      candidateIntentSha256,
      prefix: ContinuityRepairDecisionBasisIntentIncompletePrefixV1
    }
  | Conflict {
      predecessor: ContinuityRepairDecisionCandidatePredecessorV1,
      existingIntentSha256,
      candidateIntentSha256
    }

ContinuityRepairDecisionBasisPublishingV1 =
    Progress {
      predecessor: ContinuityRepairDecisionBasisIntentV1,
      candidateBasisSha256,
      prefix: ContinuityRepairDecisionBasisIncompletePrefixV1
    }
  | Conflict {
      predecessor: ContinuityRepairDecisionBasisIntentV1,
      existingBasisSha256,
      candidateBasisSha256
    }

ContinuityRepairDurableSelectedDecisionV1(
  ContinuityRepairDecisionBasisV1
)

ContinuityRepairDecisionSelectionStateV1 =
    Progress {
      selected: ContinuityRepairDurableSelectedDecisionV1,
      candidateSelectionSha256,
      prefix: ContinuityRepairDecisionSelectionIncompletePrefixV1
    }
  | Conflict {
      selected: ContinuityRepairDurableSelectedDecisionV1,
      existingSelectionSha256,
      candidateSelectionSha256
    }

ContinuityRepairOutcomeIntentPublicationStateV1 =
    Progress {
      predecessor: ContinuityRepairOutcomeDecisionPreV1,
      candidateIntentSha256,
      prefix: ContinuityRepairOutcomeIntentIncompletePrefixV1
    }
  | Conflict {
      predecessor: ContinuityRepairOutcomeDecisionPreV1,
      existingIntentSha256,
      candidateIntentSha256
    }

ContinuityRepairTerminalOutcomePublicationStateV1 =
    Progress {
      predecessor: ContinuityRepairOutcomeIntentV1,
      candidateTerminalSha256,
      prefix: ContinuityRepairTerminalOutcomeIncompletePrefixV1
    }
  | Conflict {
      predecessor: ContinuityRepairOutcomeIntentV1,
      existingTerminalSha256,
      candidateTerminalSha256
    }

ContinuityRepairDecisionBasisIntentCommitWitnessV1 = {
  intentSha256: continuityRepairDecisionBasisIntentSha256
}
```

`ContinuityRepairDecisionBasisPublicationStateV1` is the sealed outer sum
`Intent(ContinuityRepairDecisionBasisIntentPublishingV1) |
Basis(ContinuityRepairDecisionBasisPublishingV1)`. The candidate predecessor is a sealed
borrow of the exact durable repair/source-binding/deadline/prune/watermark facts from which
the candidate is computed; it has no public constructor, fields, serde, or digest-only
reconstruction. Thus every `Conflict` carries its exact durable predecessor plus distinct
existing and candidate digests, and no conflict can overlap a progress prefix or omit the
existing final. `ContinuityRepairDurableSelectedDecisionV1` is likewise sealed and is a
newtype over exactly one `ContinuityRepairDecisionBasisV1`; it stores no duplicate selected
outcome or terminal-record digest. It is constructed only by reopening the exact
file-and-directory-durable basis and matching it byte-for-byte to its durable intent.
Private borrowing accessors derive `selected_outcome()` and
`terminal_outcome_record_sha256()` from that wrapped basis. For a degraded selection the
nested terminal failure branch and class are therefore derived from the wrapped basis;
no settlement constructor may discard them, replace them with a transient source failure,
or reconstruct them from current source state. Both selection `Progress` and selection
`Conflict` carry the same newtype value before the first vulnerable
selection-publication operation.

`ContinuityRepairDecisionBasisIntentCommitWitnessV1` is a broker-private fixed no-replace
record in a coordinator-owned namespace independent of the intent final. It is published
only after the exact intent final and its complete directory chain have been synced, and
its own final and complete directory chain must then be durable. Its encoded ceiling is
2,048 bytes and its one record/2,048-byte attempt slot is charged before publication. Its
sealed constructor accepts only the exact intent digest. Every consumer reopens the exact
witness, validates its canonical record digest and exact intent predecessor, and consumes
the sealed value rather than a digest copy. It is neither an export/audit member nor a new public
publication boundary, record-boundary subvisitor, registry id, or hash preimage. A basis
publisher may consume the intent predecessor only together with its matching sealed
witness. For the intent publisher only, `Progress { prefix: AncestorsDurable }` remains the
witness-pending state and derives the existing final-directory-sync boundary; `Complete`
is reachable only after the matching witness final and directory chain are durable.
The witness publisher has independent fresh-process fault cases after write, file sync,
no-replace link, exact reopen, coordinator-parent sync, and each required ancestor sync.
Their literal hooks are
`decision-intent-witness/after-write`,
`decision-intent-witness/after-file-sync`,
`decision-intent-witness/after-link`,
`decision-intent-witness/after-reopen`,
`decision-intent-witness/after-parent-sync`, and
`decision-intent-witness/after-ancestor-sync/<depth>`. A hook terminates before the next
publication operation. The two pre-link cases restart with no named final: the lost
unnamed inode is discarded and the same canonical witness is idempotently republished
beginning at write. The after-write poison proves file sync was not inferred, and the
after-file-sync poison proves link was not inferred. Every post-link case reopens the
byte-identical named final and resumes at the first missing reopen, parent-sync, or
ancestor-sync operation without rewriting or relinking it. At every observable prefix
whose complete witness directory chain is not yet durable, every consumer remains at
zero: basis construction and publication, decision selection, decision-pre, outcome
intent, terminal outcome, final-absence proof, compaction, settlement, and selected-outcome
projection. Only the final ancestor-sync case may classify the witness directory chain
complete and admit the basis consumer. The expected fixture separately pins
`<hook>/hook-removed` and `<hook>/wrong-resume` poisons. For every literal `<hook>` named
above, the closed ten-member per-consumer poison-path family is
`<hook>/consumer-ran/<basis-construction|basis-publication|decision-selection|decision-pre|outcome-intent|terminal-outcome|final-absence-proof|compaction|settlement|selected-outcome-projection>`.
Each member is pinned independently; an aggregate `<hook>/consumer-ran` assertion is
forbidden as a substitute, and no one per-consumer poison may satisfy another. The
wrong-resume assertion requires write after either
pre-link crash, reopen after link, parent sync after reopen, the first missing ancestor
sync after parent sync or an intermediate ancestor sync, and basis consumption only after
the final ancestor sync. Every fault hook, exact observable-prefix resume assertion,
forbidden consumer effect, and poison is independent; no earlier generic
absent-or-fully-durable case can satisfy this matrix. These are test hooks, not record
boundaries, registry ids, or public states.

Durable-final loss is a separate closed integrity algebra:

```text
ContinuityRepairDecisionDurabilityIncidentV1 =
    IntentFinalMissingAfterAncestorsDurable
  | BasisFinalMissingAfterAncestorsDurable
  | IntentCommitWitnessMissingAfterConsumption
  | IntentCommitWitnessMismatchAfterConsumption
  | IntentCommitWitnessPredecessorMismatchAfterConsumption
```

The first two variants map exactly to record
`decision-basis-intent | decision-basis`, durable boundary `ancestors-durable`, and failure
class `final-missing-after-durable-boundary`. The last three map only to record
`decision-basis-intent-commit-witness`, durable boundary
`consumed-by-durable-successor`, and respectively
`witness-missing-after-consumption | witness-mismatch-after-consumption |
predecessor-mismatch-after-consumption`. Every variant derives only its listed tuple and
`preserve-and-escalate-audit-integrity-incident`; no constructor accepts a caller-supplied
record, boundary, failure, or action. The intent variant is constructible only when the
intent final does not and either the matching independent commit witness survives or an
exact validated successor chain for the current lifecycle prefix proves that the witness
was consumed. The latter proof does not require pretending an in-progress unlink is
already durable. Immediately after decision-witness unlink and before coordinator-parent
sync, it consists of the exact current-name absence, identity-revalidated held coordinator
parent, and the durable successor chain; after parent sync it instead carries the exact
parent-synced absence. The pre-unlink `PresentExact` observation is a required guard on the
unlink operation, not durable evidence consumed by a fresh-process constructor. The
invariant chain consists of the byte-identical basis and downstream decision predecessors,
the exact targets-compacted receipt, the complete immediate target-receipt chain, and the
prefix-appropriate decision-witness removal observation. During target-witness reclamation
it also contains, in ordinal order, every completed target-witness parent-synced absence, the
exact absent-but-parent-sync-pending current name when applicable, and every exact
still-present witness/receipt pair for later names. At `TargetsCompacted` and later it
contains all ordered witness-parent absences plus the exact source-release and
attempt-slice prefix through the current checkpoint. Every durable member must reopen and
validate, and every current parent observation must be identity-revalidated, before the
chain can prove prior consumption. An `AncestorsDurable` progress prefix, prior in-memory
sync observation, later prefix without its complete prefix-appropriate chain, or asserted
witness absence cannot construct the incident. A
witness-consumption variant is constructible only from an exact
durable basis or later descendant proving that a witness was consumed. It preserves a
missing or nonidentical witness and any nonidentical intent predecessor, poisons the
attempt, and performs zero reconstruction, relink, witness publication, source access,
reselection, later publication, compaction, settlement, or cleanup mutation. This incident
is neither `PendingDecisionBasis` nor a publication
`Conflict`, terminal repair outcome, or settlement state. Its exact public human/JSON
response is fixed in `contracts/operator-cli.md`. There is no parent-durable incident wire
variant or constructor: a live `ParentDurable`-only reconciliation prefix still follows the
pre-`AncestorsDurable` absent-or-exact rule below, and no downstream consumer is reachable
before the intent commit witness is durable.

This is the write-ahead basis lifecycle, not a transient outcome-selection wrapper. The
complete typed outcome becomes selected only when the exact no-replace
`ContinuityRepairDecisionBasisIntentV1` final, its complete directory chain, and its
independent `ContinuityRepairDecisionBasisIntentCommitWitnessV1` are durable. Before that
commit, no selected decision exists. After it, every basis publication state carries that
durable intent under the matching sealed witness and a fresh process may not consult or
mutate the source, prune state, or watermark.
Until the basis and directory chain are durable, this state contains no public intended
outcome, terminal failure, or action; only the intrinsic publication state and derived
boundary may be projected.
Its exact public settlement token is `decision-basis-pending`.
None of the five publishing sums is constructible from a caller, schema, or independent
boundary. A `Progress` variant intrinsically derives exactly one first missing boundary
from its prefix; a `Conflict` intrinsically derives the `conflict` boundary. No constructor
accepts a second failure-boundary value, so state/boundary disagreement is
unrepresentable.

Restart after either decision-intent or basis `FinalLinked` always attempts an exact final
reopen before classifying the next step. At `FinalLinked`, `FinalReopened`, and
`ParentDurable`, and after the intent directory sync while no matching commit witness
exists, absence and the byte-identical exact final are the only valid results. Exact
survival resumes the first missing reopen, directory sync, or intent-witness publication.
Intent absence without the matching witness means no complete decision commit survived:
before discarding anything, the broker performs a complete attempt-local census for a
basis, decision selection, decision-pre, outcome intent, terminal outcome,
final-absence proof, compaction prefix, or completed prefix. Only a census with no basis or
later descendant permits the broker to discard the lost candidate and its incomplete
prefix, replay the sealed durable repair state, rerun candidate selection, and publish the
newly selected canonical intent. Any surviving consumer instead proves that the witness
was consumed and forbids reselection. Before authorized witness reclamation it returns the
matching witness-consumption incident with zero mutation; at or after decision-witness
reclamation, only the exact prefix-appropriate successor chain defined above may classify
the witness absence and resume that successor. If the intent final is also absent, that
complete validated chain instead constructs
`IntentFinalMissingAfterAncestorsDurable`; it never resumes, recreates, or reselects.
Precommit reselection may produce different intent bytes, terminal outcome, or degraded
branch/class; no prior candidate identity is preserved or projected, and no selected
decision existed to replace. Basis absence is recreated byte-identically only from the
exact durable intent, with no source access, evidence replay, or outcome reselection. A
`ParentDurable`-only prefix is not silently upgraded to directory-chain durability:
recreation repeats parent sync and every missing ancestor sync. No durable intent or basis
bytes are presumed to exist when the pre-`AncestorsDurable` final is absent.

Decision-witness precedence is likewise closed. An empty complete descendant census is
the only no-witness input that permits precommit reselection. Any surviving basis
`Progress | Conflict`, decision-selection `Progress | Conflict`, decision-pre,
outcome-intent `Progress | Conflict`, terminal `Progress | Conflict`, final-absence
prefix, compaction census or receipt, `PreparationIncomplete`,
`PendingIntentPublication`, `PendingTerminalPublication`, or completed settlement proves
consumption and forbids reselection. Before authorized reclamation, missing or mismatched
witness state returns the matching
`IntentCommitWitnessMissingAfterConsumption |
IntentCommitWitnessMismatchAfterConsumption |
IntentCommitWitnessPredecessorMismatchAfterConsumption` incident. During or after
authorized reclamation, only the exact prefix-appropriate successor chain above can replace
the witness as proof; an incomplete or corrupt chain returns its existing typed integrity
incident with zero mutation. Every durable basis, selection, and settlement prefix
therefore has exactly one of a surviving witness, a proof-carrying successor, or a
fail-closed incident.

Once the matching intent commit witness is durable, and at every state that has consumed
it, both the byte-identical intent final and the byte-identical witness are mandatory until
the ordered successor-proved reclamation after the targets-compacted receipt. After the
authorized decision-witness reclamation, the witness may be absent but the intent final
remains mandatory and the exact prefix-appropriate successor chain replaces the surviving
witness solely as proof that the intent was consumed. Intent absence then returns
`IntentFinalMissingAfterAncestorsDurable` and performs zero recreation, relink, source
access, reselection, decision-selection publication, or later settlement. Without that
witness and without a surviving consumer, intent absence follows the same precommit
recovery above regardless of whether a prior process had completed the ancestor sync. At
basis `AncestorsDurable`, the
byte-identical basis final remains mandatory and absence returns
`BasisFinalMissingAfterAncestorsDurable`.
A nonidentical final at any point or a predecessor mismatch remains the matching sealed
publication `Conflict`; it is not relabeled as absence. A nonidentical intent witness is
also preserved as the intent publisher's sealed `Conflict` before the first consumer.
After a basis or later descendant is durable, a missing witness, nonidentical witness, or
nonidentical witness-to-intent predecessor returns its exact witness-consumption incident
instead; every such conflict or incident performs zero mutation and poisons the attempt.
The intent selection freezes
only when its complete directory chain and matching independent commit witness are durable;
only after the basis also reaches `AncestorsDurable` may that frozen outcome enter
decision-selection publication.
A nonidentical intent, basis, selection, outcome-intent, or terminal final derives only the
matching sealed `Conflict` carrying predecessor/existing/candidate, preserves both values,
and performs no replacement or later settlement mutation.

The matching outcome repeats the exact pre identity and deadline plan and has exactly one
nested variant:

```text
ContinuityRepairOutcomeV1 =
    RepairedBeforeDay90 { repairedWatermarkSha256 }
  | RepairedAfterMandatoryPrune {
      mandatoryPruneProofSha256,
      repairedWatermarkSha256
    }
  | DegradedBeforeDay90(ContinuityRepairTerminalFailureV1)
  | DegradedDay90BeforePrune(ContinuityRepairTerminalFailureV1)
  | DegradedDay90AfterPrune {
      mandatoryPruneProofSha256,
      failure: ContinuityRepairTerminalFailureV1
    }
```

Constructors consume the pre variant and reachable prefix. They cannot construct day-90
success without the exact whole-set proof and empty census, before-day-90 success with any
prune proof, a before-prune failure carrying a whole-set proof, an after-prune failure
without that exact proof, success plus failure, or a deadline plan different from pre-audit.
`DegradedDay90BeforePrune` means before a complete whole-set proof; it remains the only
degraded day-90 variant during a partially completed member sequence.
Audit
projection, strict schema, wire snapshot, human/JSON goldens, and deserializers reject every
cross-pair and unknown variant. No outcome stores a caller-supplied action.

The closed outcome tags are `0x01` for `RepairedBeforeDay90`, `0x02` for
`RepairedAfterMandatoryPrune`, `0x81` for `DegradedBeforeDay90`, `0x82` for
`DegradedDay90BeforePrune`, and `0x83` for `DegradedDay90AfterPrune`. Once the reachable
terminal outcome is known, the broker first publishes broker-private immutable
`ContinuityRepairDecisionBasisIntentV1` under the already reserved settlement slice. This
is the write-ahead decision journal. It contains the one complete selected
`ContinuityRepairOutcomeV1`, including the exact failure branch/class for a degraded
outcome. After its file and complete directory chain are durable, the broker publishes the
independent matching `ContinuityRepairDecisionBasisIntentCommitWitnessV1`; both must be
durable before any basis hierarchy, basis final, decision-selection hierarchy, or response
carrying an intended outcome. The broker then publishes byte-identical
`ContinuityRepairDecisionBasisV1`. The basis constructor consumes the durable intent and
its sealed matching witness and rejects every field or digest mismatch. The selection-state
`Conflict` variant may be created only by revalidating a nonidentical no-replace selection
final against that durable basis. The intent, basis, and selection-state sum have no public
fields, construction, conversion, serde, or caller-supplied action.

Their identities and crash-stable publication are:

```text
continuityRepairDecisionBasisIntentSha256 =
  SHA-256("d2b:host-generation:continuity-repair-decision-basis-intent:v1\0" ||
  continuity_repair_attempt_sha256 ||
  continuity_source_binding_receipt_sha256 ||
  deadline_plan_tag_u8 || deadline_plan_payload ||
  terminal_outcome_record_sha256)

continuityRepairDecisionBasisSha256 =
  SHA-256("d2b:host-generation:continuity-repair-decision-basis:v1\0" ||
  continuity_repair_attempt_sha256 ||
  continuity_source_binding_receipt_sha256 ||
  deadline_plan_tag_u8 || deadline_plan_payload ||
  terminal_outcome_record_sha256)
```

The deadline tag and payload use the canonical terminal-record encoding above. The intent
and basis each use the common no-replace publication protocol and their distinct incomplete
prefix types and sealed `Progress | Conflict` publication states. Intent publication is the
write-ahead commit only after its final, complete directory chain, and independent matching
commit witness are durable; there is no state in which a selected choice precedes that
boundary. The broker may compute a candidate under the coordinator lock, but it does not
classify or return that candidate as selected before the witness is durable. A fresh process
before that durability returns only basis-pending. If the intent final is absent and no
matching witness survives, it first proves through a complete attempt-local census that no
basis or later descendant survives. Only then may it discard the lost uncommitted candidate
and rerun selection from the sealed durable repair state; it cannot project either the old
or new intended outcome or terminal failure. A surviving basis or later descendant is
proof of prior consumption and makes the same missing-witness observation the closed
decision-durability incident instead of a reselection input.

At `FinalLinked`, `FinalReopened`, and `ParentDurable` before ancestor-chain durability,
restart first reopens the named final and accepts absence or the exact candidate. An absent
pre-`AncestorsDurable` intent proves that no complete decision commit survived, so the
lost candidate and incomplete prefix are discarded. The broker replays the sealed durable
repair state, reruns candidate selection, and publishes that newly canonical intent; the
abandoned candidate has no retained identity, and this precommit reselection is not
replacement of a selected decision. The same recovery applies after ancestor-chain sync
when both the intent final and matching witness are absent only if the complete descendant
census is empty: that durable state is then indistinguishable from precommit absence. An
exact surviving intent without its witness and without any consumer resumes only witness
publication without reselection. Before its own `AncestorsDurable`, a
basis restart likewise accepts absence or the exact final and recreates absence
byte-identically only from the witness-backed durable intent, with no reselection. Once the
matching intent witness is durable, intent absence returns the closed
`IntentFinalMissingAfterAncestorsDurable` incident and never recreates or relinks. At basis
`AncestorsDurable`, basis absence returns the matching closed basis incident. The
witness-backed exact intent final is the frozen selection source. Every later basis state
requires both exact records and reconstructs solely from them. A surviving consumer with a
missing witness, nonidentical witness, or nonidentical witness predecessor returns the
closed witness-consumption incident, poisons the attempt, and performs zero mutation.

A nonidentical intent final becomes only the intent publisher's sealed `Conflict` and
preserves the predecessor plus both intent digests. A nonidentical basis final becomes only
the basis publisher's sealed `Conflict`, preserves the intent and both basis digests, and
authorizes no selection mutation. Before the first basis consumer, a foreign or
field-mismatched witness likewise becomes only the sealed intent-publisher `Conflict`;
after consumption it is the closed integrity incident instead. Failure at any write-ahead
intent or basis boundary
returns `PendingDecisionBasis` with the intrinsic state-derived boundary, no intended
outcome, and no terminal failure projection. Restart derives a public intended outcome only
after reopening the exact file-and-directory-durable basis and matching intent; it never
projects a transient selected failure. No selection hierarchy, selection final, new source
acquisition, prune, watermark, outcome intent, or terminal publication is legal while the
basis is pending.

The literal decision-basis vector reuses
`continuityRepairAttemptSha256 =
0e7225df2517e961b428335533cd4cd1cb2a3e4db5999d0c46b89da8b6fdc0a0`,
`continuitySourceBindingReceiptSha256 =
8d147d4920cd65c060f1bf44357e043b0b17f8e5d54876c394e800e9f4933fd2`,
deadline-plan tag `0x01` with an empty payload, and
`terminalOutcomeRecordSha256 =
109cead12b62bfb733f730f3dbb5eb2e2ac7796f3cf2469bdee8e72d56590bd0`.
Its expected lowercase outputs are
`continuityRepairDecisionBasisIntentSha256 =
235834839696200373f01cb604c94af584f4e42e8f36af229c831d2c81b90073`
and
`continuityRepairDecisionBasisSha256 =
528e76ab349b3af38c98ecc172dc7d8edd9534a11badf6ee2d868d6e143c3643`.
The expected table, an independently authored canonical-byte fixture, and production are
mutually read-independent. Domain, repair-attempt, source-binding receipt, deadline tag,
empty-payload framing, terminal-record, field-order, omission, and same-width substitution
poisons fail independently for both domains. The compaction export vector consumes these
exact expected hashes as the ordered `0x2d` decision-basis-intent and `0x2e`
decision-basis members; substituting arbitrary bytes, swapping them, or using the
decision-selection hash must change `requiredExportSetSha256` and fail its downstream
poison.

Only after `ContinuityRepairDecisionBasisV1` is file-and-directory durable does the broker
publish no-replace `ContinuityRepairDecisionSelectionV1` through the common publication
protocol. This is the first durable public decision state. It contains the exact complete typed outcome above,
`terminalOutcomeRecordSha256`, deadline plan, and matching repair/source-binding
identities, with no response action or sensitive body. Until its exact final and directory
chain are durable, failure is `PendingDecisionSelection` carrying
`ContinuityRepairDecisionSelectionStateV1`. Both its `Progress` and `Conflict` variants
carry the sealed `ContinuityRepairDurableSelectedDecisionV1` created before publication,
so the public projection preserves the basis-selected outcome and, for a degraded
selection, its exact nested terminal failure branch/class. The intrinsic prefix or sealed
conflict derives the publication boundary; no
preparation-incomplete or terminal/degraded result may be returned and restart performs no
new source acquisition, prune, watermark, or other repair mutation. It resumes only the
same selected canonical bytes from the durable basis. A nonidentical selection final is
preserved as the sealed selection-state `Conflict` variant with the durable selected
decision plus existing and candidate digests.

After selection is durable, the broker append-only publishes fixed
`ContinuityRepairOutcomeDecisionPreV1`, byte-identical in typed decision and digest. This is
the durable audit pre-intent journal state. The fixed audit record is charged before use.
If it cannot become durable, the result is `PreparationIncomplete` carrying the already
durable selected outcome; restart reloads that selection and retries only decision-pre
publication. No later failure, deadline, digest, posture, or action may be selected again.

Once decision-pre is durable, the broker publishes a broker-private no-replace
`ContinuityRepairOutcomeIntentV1` byte-identical in typed outcome to decision-pre. It is
charged as settlement, carries no response action, and is tested with decision-pre as
mandatory subrecords of the existing `continuity-repair-outcome` publication class rather
than adding registry classes or ids. Every hierarchy, write, file-sync, link, reopen,
directory-sync, conflict, and audit-publication boundary is a typed
`PendingIntentPublication` state. Its `Progress` variant carries only the progress prefix;
its `Conflict` carries the durable decision-pre predecessor plus existing and candidate
intent digests. Restart reconstructs the intended outcome from durable decision-pre and
retries only the exact missing intent boundary.

Only after that intent is complete may the broker publish the matching fixed terminal
outcome. Every terminal publication boundary is `PendingTerminalPublication`; restart
reloads the byte-identical intent and retries only that terminal projection. Neither
pending variant is a settled degraded failure, and neither may appear inside
`ContinuityRepairOutcomeV1`. Terminal `Progress` carries only its prefix; terminal
`Conflict` carries the durable outcome-intent predecessor plus existing and candidate
terminal digests.

If a hard source, hierarchy, write, file-sync, link, reopen, directory-sync, or conflict
failure occurs after pre, the intended outcome is the one reachable degraded variant and
advances no watermark. If a repaired watermark is complete, the intended outcome is the
matching repaired variant. Failure before decision-pre durability returns typed
`PendingDecisionBasis` until basis durability, `PendingDecisionSelection` until selection
durability, then typed `PreparationIncomplete`. Failure after decision-pre durability returns the applicable
typed pending form, blocks every later mutation, and restart reloads the exact selection
before acquiring evidence or dispatching another repair. It publishes only the unchanged
decision-pre, intent, and terminal outcome. In particular, a watermark-complete repaired decision can
never settle as degraded, and a degraded decision can never acquire or validate a
watermark during settlement. The watermark final is never republished or replaced.

Fresh-process source-change cases mutate source availability, version, authority, replay
binding, terminal failure, and returned bytes before every decision-basis and
decision-selection boundary.
Before write-ahead intent commit-witness durability, the only response is basis-pending
without an intended outcome or terminal failure. Those mutations cannot influence an exact
surviving pre-witness intent. An absent intent without a matching witness has no selected
identity to preserve: recovery discards its lost candidate, replays the sealed durable
repair state, and may reselect against the changed source while still returning only
basis-pending, but only after the complete attempt-local census proves that no basis or
later descendant survives. Independent fresh-process negatives retain in turn a basis,
decision-selection prefix, decision-pre, outcome-intent prefix, terminal prefix,
final-absence prefix, and every compaction prefix through the targets-compacted receipt
before witness reclamation while removing the intent and witness. Every case returns the
witness-missing-after-consumption integrity incident,
poisons the attempt, and proves zero reselection or other mutation; no descendant case may
satisfy another. At each intent and basis
after-link-before-reopen, after-reopen-before-parent-sync, and
after-parent-sync-before-ancestor-sync point, paired tests simulate both final absence and
exact-final survival. Intent absence proves the old candidate is discarded, permits one
fresh precommit selection from the sealed durable repair state, and never projects either
candidate; exact survival resumes without relink or reselection. Basis absence republishes
byte-identically only from the durable intent; exact survival resumes without relink. Both
parent-durable absence cases repeat parent sync and ancestor sync and cannot freeze
selection early. An additional intent pair starts after the complete ancestor sync but
before the independent witness is durable. Absence with no witness performs the same
discard-and-reselect recovery as every indistinguishable precommit absence; exact survival
publishes only the matching witness without source access or reselection. The pair has
independent hooks and poisons and cannot borrow an earlier prefix case. The absence half
also proves the complete descendant census is empty before reselection.
In addition to that pair, the decision-intent-witness publication matrix starts an
independent fresh process from every write, file-sync, link, reopen, parent-sync, and
individual ancestor-sync hook named above. Every incomplete case proves zero basis
construction or consumption and zero selected-outcome projection, then resumes exactly at
the first missing publication operation. The final ancestor-sync case alone may consume
the witness and continue to basis publication. Every prefix, forbidden-effect assertion,
resume point, hook, hook-removal poison, and early-consumer poison is independently
enumerated. For every literal `<hook>` named above, the closed ten-member per-consumer
poison-path family is
`<hook>/consumer-ran/<basis-construction|basis-publication|decision-selection|decision-pre|outcome-intent|terminal-outcome|final-absence-proof|compaction|settlement|selected-outcome-projection>`.
Each member is pinned independently; an aggregate `<hook>/consumer-ran` assertion is
forbidden as a substitute, and no one per-consumer poison may satisfy another.

Separate fresh-process removal-integrity cases physically delete the exact durable intent
before process start immediately after its matching independent commit witness is durable
and after every decision-basis `Progress` prefix and sealed `Conflict`. Each case returns the exact
`audit-continuity-repair-decision-durability-integrity-incident` for record
`decision-basis-intent` at boundary `ancestors-durable`, proves the intent remains
physically absent, and performs zero
reconstruction, relink, source access, outcome reselection, basis continuation or
publication, decision-selection publication, later publication, compaction, or settlement.
Each immediate or basis-prefix case, response assertion, forbidden effect, and removal
poison has an independent hook. Once the basis exists, separate fresh-process cases
physically delete the exact durable intent or basis final before process start at every
downstream durable prefix: each decision-selection publication prefix and
conflict, decision-pre durable, each outcome-intent publication prefix and conflict,
outcome-intent durable, each terminal publication prefix and conflict, terminal durable,
each final-absence-proof prefix and conflict, and completed replay. The immediate
pre-witness absence hooks do not satisfy any witness-backed ancestor-durable,
basis-publication, or downstream-removal case.
Each downstream removal must return its exact decision-durability integrity response with
the removed final still absent afterwards and zero reconstruction, relink, source access,
outcome reselection, decision-selection publication, decision-pre publication,
outcome-intent publication, terminal publication, final-absence publication, compaction,
or settlement hook. Every
record/downstream-prefix/forbidden-effect combination has a separate hook and removal
poison. A parallel fresh-process witness matrix runs before the basis consumer and after
every basis `Progress` prefix and `Conflict`, every decision-selection prefix and conflict,
decision-pre, every outcome-intent prefix and conflict, outcome-intent durable, every
terminal prefix and conflict, terminal durable, every final-absence prefix and conflict,
and every compaction prefix through the targets-compacted receipt before witness
reclamation. At each location, independent cases remove
the witness, substitute a foreign witness, change its intent predecessor, and change its
canonical digest. Before the first consumer, each nonidentical witness is the sealed
intent-publisher `Conflict` and missing under an exact surviving intent resumes only exact
witness publication. At and after the first consumer, each missing or mismatched case
returns its exact closed witness-consumption incident. Every case poisons the attempt and
proves zero reconstruction, relink, witness publication, source access, reselection,
decision-selection, decision-pre, outcome-intent, terminal, final-absence, compaction,
settlement, cleanup, or slot-reuse mutation. Every
prefix/substitution/forbidden-effect assertion has a separate hook and removal poison. No
pre-consumer conflict, no-witness reselection, later-prefix incident, or authorized
post-targets-compacted witness-reclamation case may satisfy another.
An independent decision-witness removal matrix starts a fresh process for every target
cardinality `0`, `1`, `2`, `3`, and `4` immediately before decision-witness unlink, after
that unlink but before coordinator-parent sync, and after that sync. Every cardinality
covers the decision witness, and each nonzero cardinality additionally covers every target
witness at its zero-based ordinal less than the cardinality. The maximum four-target case
is a member of this matrix and cannot satisfy any lower-cardinality or other-witness
member. The before-unlink case requires the current exact decision witness,
targets-compacted receipt, and complete durable basis/downstream successor chain to be
present. The after-unlink case requires a recorded `PresentExact` pre-unlink hook
assertion, exact current-name absence, and identity-revalidated held parent; the
fresh-process incident constructor consumes only the absence, parent, and
durable successor proof. It must not perform the pending parent sync after the intent final
is removed. The after-parent-sync case requires the exact synced absence. For each nonzero
target cardinality, the same three-prefix matrix applies to every target witness and
requires the current exact witness and matching immediate receipt before unlink. The
zero-target case proves no target witness or receipt can be synthesized; without the
injected removal its next state is directly `TargetsCompacted`, while the removal case
remains terminal.

At every one of those prefixes the case physically removes the intent final before process
start. The surviving witness or prefix-appropriate successor proof constructs
`IntentFinalMissingAfterAncestorsDurable` and returns the exact terminal response below
with zero reconstruction, relink, witness publication or reclamation, parent sync,
continuation, release, settlement, ledger mutation, or slot reuse. The literal hook family
is defined by a separately authored read-independent expected-hook literal before any case
runs. It enumerates the valid witness coordinates exactly as `decision/0`; `decision/1`,
`target-0/1`; `decision/2`, `target-0/2`, `target-1/2`; `decision/3`, `target-0/3`,
`target-1/3`, `target-2/3`; and `decision/4`, `target-0/4`, `target-1/4`, `target-2/4`,
`target-3/4`, each at each literal prefix `before-unlink`,
`after-unlink-before-parent-sync`, and `after-parent-sync`. It lists the full hooks rather
than generating them from the case visitor, production enumeration, cardinality, or target
collection. Every full hook has the shape
`decision-intent-final-removal/witness-reclamation/<witness>/<cardinality>/<prefix>`,
where `<witness>` is exactly `decision` or `target-<zero-based-ordinal>`,
`<cardinality>` is exactly `0`, `1`, `2`, `3`, or `4`, and `<prefix>` is one of the
three literal prefixes above.
Generated visits must be bijective with that literal: every valid full hook occurs exactly
once and no other hook occurs. For every full hook, an independent omission poison removes
only that visit while retaining the expected literal, and an independent duplication
poison repeats only that visit while retaining the expected literal; both must fail with
the exact missing or duplicate hook before semantic assertions. Each hook also has
independent current-witness, applicable current-receipt, successor-member,
terminal-response, forbidden-effect, and hook-removal poisons.

Four post-decision-witness-reclamation anchor cases start independent fresh processes at
the exact checkpoints `WitnessReclamationPending`, source `Released` under
`AttemptSliceReleasePending`, attempt-slice `CompletedReleased`, and `Complete`. The
`WitnessReclamationPending` anchor is after decision-witness parent sync and before the
current target-witness unlink; it requires the exact current target witness and matching
receipt. A separate zero-target control uses the immediately following `TargetsCompacted`
checkpoint. The matrix also removes the intent at every reachable target-witness
`WitnessReclamationPending { nextWitness }` prefix before unlink, after unlink before
parent sync, and after parent sync; at `TargetsCompacted`; at every
`SourceReleasePending` prefix; and at every `AttemptSliceReleasePending` prefix. Each valid
prefix-appropriate successor chain constructs
`IntentFinalMissingAfterAncestorsDurable` and returns exit `4` error
`audit-continuity-repair-decision-durability-integrity-incident` with record
`decision-basis-intent`, boundary `ancestors-durable`, failure
`final-missing-after-durable-boundary`, and action
`preserve-and-escalate-audit-integrity-incident`. It preserves the absent intent and
performs zero reconstruction, relink, witness publication or reclamation, parent sync,
source access or release, reselection, continuation, cleanup, settlement, response replay,
attempt-slice or ledger mutation, or slot reuse.

At every anchor and intervening prefix, independent cases remove, substitute, reorder, or
change the digest or predecessor of each required successor-chain member in turn. A corrupt
chain returns that member's exact existing closed terminal integrity result before any
mutation; it cannot construct the intent-final incident from incomplete proof and never
falls back to precommit reselection, cleanup continuation, pending settlement, or completed
response replay. The literal hooks are
`decision-intent-final-removal/<checkpoint>` and
`decision-intent-final-removal/<checkpoint>/successor/<member>/<poison>`. Every lifecycle
prefix, chain member, terminal-response assertion, forbidden-effect hook, and removal
poison is independent and introduces no lifecycle variant, registry id, or public state.

After the intent commit witness is durable, every fresh process must preserve its
selected outcome through every basis boundary. After basis durability, each case must
render and publish only the
basis-selected outcome and exact nested terminal failure or its sealed selection
`Conflict`. A fresh nonidentical selection final must remain `Conflict`; a changed source
or newly observed failure cannot replace either its existing or candidate digest. None may
project a transient failure, reacquire evidence, choose a different outcome after intent
durability, or advance a watermark. Separate conflict cases cover a foreign intent, basis,
or selection final before link, after link, and after every directory boundary, with zero
replacement and zero later publication. Each accepted-absence, exact-survival, conflict,
downstream-removal, and state/boundary check has an independent hook and removal poison.
The witness-pending and witness-backed cases are subhooks of the existing
`decision-basis` visitor and add no registry id, public boundary, export member, or hash
input.

Strict response schemas and human/JSON goldens pin completed repaired, completed degraded,
pending-decision-basis, pending-decision-selection, preparation-incomplete,
pending-intent, and pending-terminal forms. Progress prefixes reject `Conflict`; each
publisher's sealed `Conflict` requires predecessor, existing digest, and candidate digest,
and schema/constructor negatives reject omission or substitution of any member.
Decision-selection-pending has separate repaired and degraded projections: the repaired
projection cannot carry a failure, while the degraded projection must carry the exact
branch/class nested in the durable selected decision. Omitting, changing, adding, or
transiently recomputing that failure is a hard schema/constructor/golden failure.
Independent fresh-process cases cover every
decision-basis-intent, decision-basis, and decision-selection boundary,
decision-pre audit failure, and every outcome-intent and terminal publication boundary for
each repaired variant and each degraded variant, restart from every incomplete selection,
durable basis intent, durable basis, durable selection, decision-pre, or intent,
byte-identical basis-intent/basis/selection/intent/terminal publication, response loss, and
completed no-write replay. Separate cases pin repaired
settlement after immutable watermark completion and degraded settlement after evidence
durability with zero watermark. They poison any changed variant, failure, deadline plan,
prune proof, repaired watermark, publication stage, intrinsic state/boundary, or action.
Constructor negatives prove `Conflict` and `Complete` cannot inhabit a progress prefix and
no independent failure can disagree with a publication state. Table-driven hard-failure
tests inject every terminal
source/publication class and every disjoint settlement stage/boundary and reject fallback
strings, nullable sibling failures, terminal/settlement conversions, or class/action
substitutions.

The only valid successful prefixes are:

```text
Absent
  -> PreAudited
  -> EvidenceDurable
     BeforeDay90:
       -> WatermarkPublication(<exact prefix>)
       -> WatermarkApplied
       -> OutcomeDecisionBasisIntentDurable(RepairedBeforeDay90)
       -> OutcomeDecisionBasisDurable(RepairedBeforeDay90)
       -> OutcomeDecisionSelectionDurable(RepairedBeforeDay90)
       -> OutcomeDecisionPreDurable(RepairedBeforeDay90)
       -> OutcomeIntentPublication(<exact prefix>)
       -> OutcomeIntentDurable(RepairedBeforeDay90)
       -> CompletedRepairedBeforeDay90
     Day90Reached:
       -> MandatoryPruneWholeSetProofDurable
       -> WatermarkPublication(<exact prefix>)
       -> WatermarkApplied
       -> OutcomeDecisionBasisIntentDurable(RepairedAfterMandatoryPrune)
       -> OutcomeDecisionBasisDurable(RepairedAfterMandatoryPrune)
       -> OutcomeDecisionSelectionDurable(RepairedAfterMandatoryPrune)
       -> OutcomeDecisionPreDurable(RepairedAfterMandatoryPrune)
       -> OutcomeIntentPublication(<exact prefix>)
       -> OutcomeIntentDurable(RepairedAfterMandatoryPrune)
       -> CompletedRepairedAfterMandatoryPrune
```

`EvidenceDurable` advances directly to watermark publication only for `BeforeDay90`.
A post-pre failure may instead journal one reachable degraded decision with zero watermark
advance and settle that exact outcome, or return preparation-incomplete/pending if the
corresponding publication cannot complete. Restart reacquires the coordinator,
reconstructs the exact handle and attempt, revalidates the source/broker binding and any
durable evidence, prune, watermark, decision-basis intent, decision basis, decision
selection, decision-pre, and intent final, and resumes only the first missing step.
Pre-only, binding-only, evidence-only, prune-complete, every watermark publication
boundary, watermark-without-basis-intent, every decision-basis-intent and decision-basis boundary,
basis-without-selection, every decision-selection boundary,
selection-without-decision-pre, decision-pre-without-intent, every intent
publication boundary, intent-without-outcome, every terminal publication boundary,
decision-basis pending, decision-selection pending, preparation-incomplete, both later pending-settlement variants,
and completed-response-loss
prefixes are independent fresh-process cases. Binding without a source receipt, evidence
without binding/pre, watermark without pre/evidence, watermark before required prune,
day-90 success without durable whole-set proof and empty census, before-day-90 use of an
unrelated prune, duplicate binding/pre/evidence/watermark/basis-intent/basis/selection/
decision/intent/outcome,
nonidentical source replay, binding/sequence/predecessor/handle mismatch,
basis/selection/decision/intent/outcome mismatch, terminal/settlement failure substitution,
deadline/outcome cross-pair, or a second dispatch degrades the root and blocks later
mutation. The lifecycle malformed-prefix case has an independently named hook and removal
poison for each listed ordering or pairing conflict. Completed response loss returns the
stored outcome with zero write.

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
one failure. It also carries `resultingCensusSha256` and its own canonical digest:

```text
pruneOutcomeSha256 =
  SHA-256("d2b:audit:host-generation:immutable-audit-backup-prune-outcome:v1\0" ||
  prune_attempt_sha256 || retention_epoch_sha256 || backup_member_sha256 ||
  prior_census_sha256 || eligibility_tag_u8 || outcome_tag_u8 ||
  resulting_census_sha256)
```

Every digest member is 32 bytes. Eligibility tags remain `0x01` and `0x02`. Outcome tags
are exactly `0x01` for `Pruned`, `0x02` for `AlreadyPruned`, `0x80` for degraded `unlink`,
`0x81` for degraded `directory-sync`, `0x82` for degraded `census`, and `0x83` for degraded
`audit-publication`. `pruneOutcomeSha256` is excluded from its own preimage. A success must
bind the freshly validated census after that member's absence; a degraded result binds the
authoritative census at the failure prefix. No serializer output, nested enum spelling,
path, clock, errno, or caller value enters the formula.

The frozen watermark and whole-set-prune vector reuses the continuity vector's
`continuityRepairAttemptSha256`, `authoritativeEvidenceSha256`, retention epoch, prior
watermark, and sequence. It sets
`greatestRealtimeSeconds = 0x0102030405060708`,
`greatestBootTimeNanoseconds = 0x1112131415161718`,
`bootIdSha256 = a0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebf`,
and uses ordered members
`c0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcdddedf`
and
`e0e1e2e3e4e5e6e7e8e9eaebecedeeeff0f1f2f3f4f5f6f7f8f9fafbfcfdfeff`.
The first and second per-member `pruneAttemptSha256` inputs are respectively the byte
ranges `00` through `1f` and `20` through `3f`; both use eligibility tag `0x02`, with
outcomes `Pruned` then `AlreadyPruned`. Exact expected outputs are:

| Identity | Expected bytes |
| --- | --- |
| initial `governedSetCensusSha256` | `b744729a18a0ebe8069f92a741147a3be5749450e250e07f24adb6801c54b284` |
| one-member intermediate census | `0261cd8a73bb28cacb8c80b321f7a5da0fca264f3113995f3b3046d73b4416ac` |
| empty `governedSetCensusSha256` | `147f09b8db794ac55a5fc12ef6176fcfb94ba7284749d9848b72fce522245afa` |
| `mandatoryPruneTargetSha256` | `ccbf03e8d2386808b05af6fc9a143d0f39909200960605ddccfe5ae0c8346abf` |
| first `pruneOutcomeSha256` | `744a24684dfe6c0b06fff23a8d59e40a60b0ab663f49d908dfc71f97ae7c7b35` |
| second `pruneOutcomeSha256` | `866dd015f616fd3e521e0ca079baf40bc692b9da87c88ea64c338d81bbf0399a` |
| `mandatoryPruneProofSha256` | `7c7c0ff714045f711907af75f6de0f81972eb4cfd3e342701883bb8b15f824d3` |
| `watermarkSha256` | `ad091f50421f6e5ac3b7a5a33dad976aea3387dcc0fdad6f94a620b625450984` |
| `repairedWatermarkSha256` | `d21178b1675a8f736978b2e39ee4cfd16887d489a97991a728258bd369202146` |

The literal expected values and production formula are read-independent. Negatives perturb
every domain, member, count, length, framing tag, outcome tag, completion tag, sequence,
clock scalar, order, and initial/intermediate/final census one at a time. They substitute
each valid digest into every other named digest field and require refusal. Separate
self-field tests alter each excluded digest without changing its preimage and prove no
self-reference. Every vector, perturbation, substitution, and excluded-field assertion has
one removal poison.

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
The wire/CLI-emittable `CLOSED_RETENTION_CAPACITY_CLASS` subset is exactly
`RetentionCapacityClassV1` without `continuity-repair-attempt-limit`. The excluded member is
private trigger-only classification for ordered cleanup; it cannot construct a public
capacity response.
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
generation transition. For each wire-emittable variant, custom response serialization
derives the exact public `failureClass` and `action` pair. Custom response deserialization
constructs the variant from the closed public class, verifies that any wire action is
byte-identical to the derived action without storing or trusting it, and rejects a
missing/mismatched action, a class from another branch, or any illegal class/action
cross-product. Public response construction, serialization, deserialization, and schema
validation reject `continuity-repair-attempt-limit` and
`resume-oldest-continuity-cleanup`. The total public derived mapping is:

| `failureClass` | Exact `action` |
| --- | --- |
| `intent-member-limit`, `intent-byte-limit`, `root-intent-limit`, `root-member-limit`, `root-byte-limit`, `root-publication-record-limit`, `root-publication-byte-limit`, `restoration-record-limit`, `restoration-byte-limit`, `restoration-attempt-limit`, `pending-staging-record-limit`, `pending-staging-byte-limit` | `reconcile-immutable-audit-retention` |
| `continuity-evidence-record-limit`, `continuity-evidence-byte-limit` | `repair-continuity-authoritative-source-contract` |
| `standing-reserve-exhausted` | `repair-retention-audit-and-reconcile` |
| `clock-rollback`, `clock-watermark`, `epoch-invalid`, `clock-forward-discontinuity`, `clock-continuity-ambiguous` | `repair-retention-clock-discontinuity` |
| `clock-overflow` | `preserve-and-escalate-retention-clock-overflow` |
| `unlink`, `directory-sync` | `repair-retention-storage-and-reconcile` |
| `census` | `repair-retention-census-and-reconcile` |
| `audit-publication`, `pending-settlement` | `repair-retention-audit-and-reconcile` |
| `standing-reserve-missing`, `standing-reserve-overdrawn`, `standing-reserve-duplicated`, `standing-reserve-unaccounted` | `preserve-and-escalate-audit-integrity-incident` |

The separate private trigger mapping is
`continuity-repair-attempt-limit -> resume-oldest-continuity-cleanup`. It selects the
cleanup control path and is not a wire/CLI failure/action pair.

Private constructors, wire/schema snapshots, and table-driven negatives reject prune
success-plus-failure, degraded-without-failure, a retention class paired with another
action, a capacity class in the degraded or admission branch, an admission class in the
audited capacity branch, a degraded class in either capacity branch, caller-provided
action, unknown variants, multiple nested branches, and missing branches. Human and JSON
projections are generated only from validated wire-emittable variants. Independent schema,
wire, human/JSON golden, and lifecycle cases cover every public class and action, including
all five standing-reserve states and both the audited and no-write capacity refusal shapes,
and negatively cover the non-emittable trigger class and internal continuation label.

Continuity-specific source and publication projection derives these exact actions without
storing one:

| Continuity failure class | Exact `action` |
| --- | --- |
| `source-unavailable` | `repair-continuity-authoritative-source` |
| `source-conflict` | `preserve-and-escalate-continuity-source-conflict` |
| `hierarchy`, `write`, `file-sync`, `link`, `reopen`, `directory-sync` | `repair-retention-storage-and-reconcile` |
| `conflict` | `preserve-and-escalate-continuity-publication-conflict` |
| `audit-publication` | `repair-retention-audit-and-reconcile` |

The nested `Retention` branch uses the existing retention mapping above. Strict continuity
schemas, wire snapshots, human/JSON goldens, constructors, and deserializers reject a
source class in the publication branch, a publication class in the retention branch,
any settlement stage or boundary in a settled degraded outcome, pending without an exact
durable prefix, preparation-incomplete without the exact durable decision selection,
pending intent without decision-pre, pending terminal without intent, a `Complete` pending
prefix, an independently supplied boundary, any other
stage/prefix/settlement/action pair, a mismatched derived action, and every unknown or
multiple branch. Settlement never enters the terminal failure mapping. Its action is
derived from the effective intrinsic boundary:

| Settlement state/boundary | Exact `action` |
| --- | --- |
| decision-pre audit failure | `repair-retention-audit-and-reconcile` |
| decision-basis, decision-selection, outcome-intent, or terminal-outcome at `hierarchy`, `write`, `file-sync`, `link`, `reopen`, or `directory-sync` | `repair-retention-storage-and-reconcile` |
| decision-basis, decision-selection, outcome-intent, or terminal-outcome at `conflict` | `preserve-and-escalate-continuity-publication-conflict` |
| decision-basis, decision-selection, outcome-intent, or terminal-outcome at `audit-publication` | `repair-retention-audit-and-reconcile` |

Pre-repair continuity infrastructure has two additional closed failure domains.
`ContinuityReplayKeyFailureClassV1` is exactly `entropy-unavailable | hierarchy | write |
file-sync | link | reopen | directory-sync | conflict |
replay-key-missing-after-parent-durable | posture | audit-publication`.
`ContinuitySourceLifecycleStageV1` is exactly `pin-acquisition | replay-binding |
source-release | source-prefix-reconciliation`.
`ContinuitySourceLifecycleFailureClassV1`
is exactly `source-capacity | source-unavailable | source-conflict | hierarchy | write |
file-sync | link | reopen | unlink | directory-sync | census | conflict |
audit-publication | recovery-generation-overflow`.
Reserved-subset admission uses only the existing
`continuity-evidence-record-limit | continuity-evidence-byte-limit |
continuity-repair-attempt-limit` capacity classes and never relabels them as a source or
replay-key failure. Evidence size/count failures render authoritative-source-contract
repair. A live-attempt limit is the private trigger-only class: it first drives the oldest
ordered broker-target/source/attempt-slice cleanup and, if blocked, renders that exact
owning failure instead of the trigger or its internal continuation label. Replay-key
reservation uses the existing root-publication capacity or
standing-reserve admission class and likewise never enters the infrastructure enum.
The source's exact combined-attempt boundary accepts 141,312 encoded bytes and reports
`source-capacity -> repair-continuity-authoritative-source-contract` at 141,313 before
acquisition pre-audit. A 257th live source pair or repair slot is not `source-capacity` and
is not repairable by pruning alone: it drives broker targets, source release, and
attempt-slice release in that order and returns the exact first compaction,
source-lifecycle, or capacity-release blocker. The next source acquisition is
admitted only after all three cleanup phases complete.
Their total actions are:

| Infrastructure failure | Exact `action` |
| --- | --- |
| replay key `entropy-unavailable` | `repair-continuity-replay-key-generation` |
| replay key `hierarchy`, `write`, `file-sync`, `link`, `reopen`, `directory-sync` | `repair-retention-storage-and-reconcile` |
| replay key `conflict` | `preserve-and-escalate-continuity-publication-conflict` |
| replay key `replay-key-missing-after-parent-durable` or `posture` | `preserve-and-escalate-audit-integrity-incident` |
| replay key `audit-publication` | `repair-retention-audit-and-reconcile` |
| source binding `source-capacity` caused by evidence record/byte shape | `repair-continuity-authoritative-source-contract` |
| source binding `source-unavailable` | `repair-continuity-authoritative-source` |
| source binding `source-conflict` or `conflict` | `preserve-and-escalate-continuity-source-conflict` |
| source binding `hierarchy`, `write`, `file-sync`, `link`, `reopen`, `directory-sync` | `repair-continuity-source-storage-and-reconcile` |
| source release `unlink` | `repair-continuity-source-storage-and-reconcile` |
| source release `census` | `repair-retention-census-and-reconcile` |
| source binding `audit-publication` | `repair-retention-audit-and-reconcile` |
| source-prefix reconciliation `recovery-generation-overflow` | `preserve-and-escalate-audit-integrity-incident` |

Valid infrastructure stage/class pairs are closed. Replay-key
`audit-publication` is valid only while publishing its fixed outcome after parent
durability. Source `pin-acquisition` may use `source-capacity | source-unavailable |
source-conflict | hierarchy | write | file-sync | link | reopen | directory-sync |
conflict | audit-publication`; `replay-binding` may use the same set without
`source-capacity`; and `source-release` may use only `source-unavailable |
source-conflict | hierarchy | reopen | unlink | directory-sync | census | conflict |
audit-publication`. `source-prefix-reconciliation` admits only
`recovery-generation-overflow`. Every other stage/class pair is unrepresentable and rejected by strict
schemas, snapshots, constructors, deserializers, and human/JSON goldens.

Constructor, wire, human/JSON, and deserialization matrices cover every row and reject a
replay-key class in the source domain, a source class in settlement or capacity, a reserved
subset class in infrastructure, and every action substitution.

Continuity cleanup pending and terminal capacity integrity are disjoint public domains
rather than replay-key, source, or settled-repair failures:

```text
ContinuityCleanupPendingStageV1 =
    replay-key-candidate-recycling
  | broker-compaction
  | capacity-release

ContinuityCleanupPendingFailureClassV1 =
    head-changed
  | target-changed
  | hierarchy
  | write
  | file-sync
  | link
  | reopen
  | unlink
  | directory-sync
  | census
  | conflict
  | audit-publication

ContinuityCleanupPendingV1 = {
  stage: ContinuityCleanupPendingStageV1,
  class: ContinuityCleanupPendingFailureClassV1
}

ContinuityCapacityIntegrityIncidentV1 = {
  stage: capacity-release,
  class:
      ledger-conflict
    | standing-reserve-missing
    | standing-reserve-overdrawn
    | standing-reserve-duplicated
    | standing-reserve-unaccounted
}
```

Valid pending pairs are exactly
`replay-key-candidate-recycling/(hierarchy | write | file-sync | link | reopen | unlink |
directory-sync | census | conflict | audit-publication)`,
`broker-compaction/(head-changed | target-changed | hierarchy | write | file-sync | link |
reopen | unlink | directory-sync | census | conflict | audit-publication)`, and
`capacity-release/(census | audit-publication)`.
`head-changed | target-changed | conflict` map to
`preserve-and-escalate-continuity-publication-conflict`;
`hierarchy | write | file-sync | link | reopen | unlink | directory-sync` map to
`repair-retention-storage-and-reconcile`; `census` maps to
`repair-retention-census-and-reconcile`; and `audit-publication` maps to
`repair-retention-audit-and-reconcile`.

The terminal incident classes are exactly:

```text
ContinuityCapacityIntegrityIncidentClassV1 =
    ledger-conflict
  | standing-reserve-missing
  | standing-reserve-overdrawn
  | standing-reserve-duplicated
  | standing-reserve-unaccounted
```

Every terminal class maps only to
`preserve-and-escalate-audit-integrity-incident`, keeps the charged slice unavailable, and
has no successor or retry. Constructors, strict schemas, wire snapshots, human/JSON
goldens, and deserializers accept every listed pending pair and terminal incident class and
reject every cross-domain class, pending/incident relabeling, or action substitution.
Post-unlink broker-compaction and candidate recycler responses carry the original durable
operation prefix and resume that operation; they are never settled degraded outcomes.
Capacity-release `census` and `audit-publication` remain pending and resumable. A terminal
capacity incident is never cleanup-pending even though both domains identify the
`capacity-release` stage.

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
continuity evidence 1/2-record, 131,072/131,073-byte body,
141,312/141,313-byte combined source-attempt, and 256/257-live-attempt boundaries,
post-export retention/compaction restart, exactly-once outcomes, illegal DTO cross-products,
and redacted reports.

The no-observable private-identifier contract covers every identifier introduced by this
publication root: the raw publication-root operation id and private root reference,
`GoverningCapacityOperationIdV1`, `CapacityReservationAttemptIdV1`,
`CapacityReleaseAttemptIdV1`, `RetentionAnchorAttemptIdV1`,
`ContinuityReplayKeyPublicationAttemptIdV1`,
`ReplayKeyCandidateRecyclingAttemptIdV1`,
`ContinuitySourceBindingAttemptIdV1`, `ContinuityEvidenceReplayHandleV1`,
`ContinuitySourceReleasePermitV1`,
`ContinuitySourcePrefixReconciliationPermitV1`,
`ContinuitySourceReleaseAttemptIdV1`, `ContinuityRepairAttemptIdV1`,
`ContinuityRepairDecisionBasisIntentV1`,
`continuityRepairDecisionBasisIntentSha256`,
`ContinuityRepairDecisionBasisV1`,
`continuityRepairDecisionBasisSha256`,
`ContinuityRepairDecisionSelectionStateV1`,
`ContinuityCompactionAttemptIdV1`,
`ContinuityDegradedAttemptReclamationIdV1`,
`ContinuityCompactionRecoveryGenerationIdV1`,
`ContinuityCompactionTargetMutationIntentV1`,
`ContinuityTargetsCompactedReceiptV1`,
`ContinuityAttemptReservedSliceReleaseAttemptIdV1`,
`brokerPrivateContinuityReplayKey`, `keyCandidateSha256`,
`PruneAttemptIdV1`, `RestorationAttemptIdV1`,
`RestorationSettlementAttemptIdV1`, every complete preimage, and each unqualified
hexadecimal encoding. The complete source-private pin acquisition record, replay-binding
record, binding receipt, release permit, and candidate-commitment record each have a
distinct body canary even when no individual field is otherwise sensitive. Canaries inject
every value independently and require zero occurrences
in public DTO/schema/snapshot/example bytes, human or JSON output, error, `Display`, log,
trace/span, metric, panic, or `Debug`. Audit is the sole exception and may contain only the
named domain-separated fields `publicationRootOperationSha256`,
`publicationRootRefSha256`, `capacityReservationAttemptSha256`,
`capacityReleaseAttemptSha256`, `retentionAnchorAttemptSha256`,
`continuityReplayKeyPublicationAttemptSha256`,
`replayKeyCandidateRecyclingAttemptSha256`,
`continuitySourceBindingAttemptSha256`,
`continuitySourceAuthorityAuditSha256`,
`continuitySourceBindingReceiptSha256`,
`continuityEvidenceReplayHandleSha256`, `continuityRepairAttemptSha256`,
`continuityCompactionAttemptSha256`,
`continuityDegradedAttemptReclamationSha256`,
`continuityCompactionRecoveryGenerationSha256`,
`continuityCompactionOperationSha256`,
`continuityCompactionSelectionAuditSha256`,
`continuityCompactionTargetSetAuditSha256`,
`currentContinuityHeadProofAuditSha256`,
`requiredExportSetAuditSha256`,
`governedSetCensusAuditSha256`,
`terminalOutcomeRecordAuditSha256`,
`completedTargetReceiptSetAuditSha256`,
`residualTargetSetAuditSha256`,
`continuityCompactionOutcomeAuditSha256`,
`brokerTargetsCompactedSha256`,
`continuitySourceReleaseAttemptSha256`,
`continuitySourceReleaseOutcomeSha256`,
`brokerCompactionCompletionSha256`,
`continuityAttemptReservedSliceReleaseAttemptSha256`,
`continuityAttemptReservedSliceReleaseOutcomeSha256`,
`pruneAttemptSha256`, `restorationAttemptSha256`, and
`restorationSettlementAttemptSha256` with the exact formulas in this section. Each field is
allowed only on its named record family. Audit may not contain a raw id, raw preimage,
ordinary source-authority digest, source-private record or receipt bytes, candidate
commitment, `expectedLeafRecordSha256`, private target mutation intent or receipt, raw
decision-basis/selection/target/head/export/census/terminal/residual digest, unqualified hash, one domain's
digest relabeled as another, or a canonical
restoration attempt digest substituted for a settlement digest. A read-independent canary
visitor and one shrinkage poison per private identifier, complete preimage, source-private
record, unqualified encoding, named audit field, record-family allowlist, and observable
surface enforce the complete list.

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
sequence is unsigned eight-byte big-endian, and the closed settlement-state tag is one
byte. Tags are exactly `0x01 pre-audited`, `0x02 publishing`, `0x03 restored`,
`0x04 pre-only-awaiting-identical-resubmission`, `0x05 degraded-repair-required`,
`0x06 repair-resume-pre-audited`, and `0x07 pending-restart-settlement`. The first event has
sequence `0` and the all-zero 32-byte `priorSettlementSha256` sentinel. Every later event
increments the sequence by exactly one and names the immediately preceding
`restorationSettlementAttemptSha256`; the zero sentinel is invalid after sequence zero.

No serializer output or variable-length concatenation enters either formula. The event
contains only `restorationSettlementAttemptSha256`, the sequence,
`priorSettlementSha256`, and one typed state. A restoration pre, provenance, or outcome
digest cannot substitute for this settlement digest, and the settlement digest cannot
appear on those record families.

The frozen first-event vector uses
`privateRestorationAttemptId =
000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f`,
sequence `0`, the zero predecessor sentinel, and tag `0x01`. It yields
`RestorationSettlementAttemptIdV1 =
94d6e5a186f03d61a8f4034b0c13222d91330c5511ffcdf60d79bbfc0256b6dc`
and
`restorationSettlementAttemptSha256 =
9ea898ddb3c7299554111cf55278e78c8902238df2ada6cfdb30f3fb2a6925bc`.
Read-independent vectors change the restoration attempt, sequence, predecessor, every
state tag, private domain, and audit domain one at a time; reject sequence gaps, a nonzero
first predecessor, a zero later predecessor, a predecessor from another record family, and
every illegal chain edge; and pin every settlement schema, snapshot, audit record, and
golden. Each vector and substitution has a removal poison.
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
while the ledger and restoration operation are unchanged.
`CLOSED_RETENTION_CAPACITY_CLASS` excludes the trigger-only
`continuity-repair-attempt-limit`, so neither that class nor
`resume-oldest-continuity-cleanup` can inhabit this response. Pre-audit standing-reserve
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
The existing `prune-pre` and `prune-outcome` ids each run two independently hooked
subvisitors: backup-member prune and continuity compaction. Removing either subvisitor or
letting one satisfy the other's expected visit fails while the class list and its exact 216
ids remain unchanged. The continuity-compaction subvisitor contains the independent
pre-witness idempotent-unlink cases, witness-backed post-unlink classification cases, and
post-receipt reappearance matrix after every receipt before the next unlink or final
`TargetsCompacted`, plus the six-member witness-substitution/successor-precedence matrix
and maximum-five-name bounded witness reclamation; each retains its own hooks and poisons without
adding an id. The
existing `continuity-repair-outcome` ids independently visit
exactly these six named subvisitors at their named boundary: (1) `decision-basis`, (2)
`decision-selection`, (3) `decision-pre`, (4) `exact-outcome-intent`, (5)
`terminal-outcome`, and (6) `final-absence-proof`. `decision-basis` separately hooks both
its write-ahead intent and basis final without becoming a seventh subvisitor. The literal
read-independent pin contains all six names; a five-name registry, an omitted
`decision-basis`, a renamed member, or one visit satisfying another fails before any case
counts. Each subvisitor has an independent missing-visit, wrong-boundary, wrong-predecessor,
and poison-removal assertion. At every decision-basis intent and basis boundary, the
decision-basis subvisitor also pins the exact exit-`4` decision-basis-pending response
schema and human/JSON golden. Its paired pre-`AncestorsDurable`
absence/exact-survival hooks are independent for intent and basis at link, reopen, and
parent-durable prefixes. A separate pair runs after intent ancestor sync and before its
independent commit witness: absence takes the same discard-and-reselect precommit recovery
only after an empty complete descendant census, while exact survival publishes only the
matching witness. Independent controls retain every later descendant and require the
closed witness-consumption incident with zero mutation. Once that witness is durable,
independent fresh-process final-removal hooks physically delete the exact intent final
immediately before basis publication and after every decision-basis `Progress` prefix and
sealed `Conflict`. Each case returns the exact
`audit-continuity-repair-decision-durability-integrity-incident` for record
`decision-basis-intent` at boundary `ancestors-durable`, proves its continued physical
absence, and invokes zero
reconstruction, relink, source access, reselection, basis continuation or publication,
decision-selection publication, compaction, or settlement. Every case, response assertion,
forbidden effect, witness-presence assertion, and removal poison is independent. After
basis durability, the downstream final-removal hooks independently delete intent and basis
at every decision-selection,
decision-pre, outcome-intent,
terminal-outcome, final-absence-proof, and completed prefix. Every downstream case requires
the exact decision-durability integrity response, continued physical absence of the removed
final, and zero reconstruction, relink, reselection, later publication, compaction, or
settlement; each hook and response assertion has a separate removal poison. Separate
fresh-process witness hooks remove or substitute the witness, its intent predecessor, and
its canonical digest before the basis consumer and after every downstream prefix through
the targets-compacted receipt before witness reclamation. Authorized later absence is
covered separately by the exact successor/reclamation matrix.
Pre-consumer mismatch is the sealed conflict; every consumed-witness failure is its exact
closed integrity incident with zero mutation. Every prefix, substitution, and forbidden
effect has an independent poison. Separate
renderer/DTO assertions retain all five closed record/boundary/failure tuples from
`contracts/operator-cli.md`, exact exit `4`, and exact human/JSON bytes; no tuple is
inferred from another. The six-name pin, these subhooks, and
their renderer assertions add no registry id; the intent commit-witness cases remain
subhooks of `decision-basis` at the existing boundary.

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

The literal
`lifecycle/aggregate/continuity-repair-attempts-257-refused` id preserves the initial
private capacity-classifier refusal that enters ordered cleanup; `refused` does not name a
wire/CLI refusal. Its subvisitors require either one complete ordered release followed by
admission with a new sequence or the exact owning broker-cleanup, source-lifecycle, or
attempt-slice-ledger failure. They also require public response construction, schema, wire,
and human/JSON golden rejection of `continuity-repair-attempt-limit` and
`resume-oldest-continuity-cleanup`.

A separately authored literal 88-id constant, the fixture, and production visitors are
mutually read-independent. Each case must reach its named limit, prefix, failure injection,
permit negative, continuation source, or settlement branch. A `prefix-matrix` case has
independently named hook subcases for every prefix and one removal poison per hook; a grouped
permit case likewise owns separate compile-fail/API assertions and poisons for every token
named in its id. Each release malformed-prefix id independently hooks outcome-without-pre,
ledger-without-pre, completion-without-ledger, duplicate pre/outcome, wrong generation,
wrong prior ledger, wrong reason, wrong proof, and cross-release proof substitution. The
capacity reservation successful-prefix case independently calculates the eleven-row
13,056-record/59,506,688-byte future charge, tests exact admission and each one-short
boundary, exhausts unreserved capacity after replacement, retains 255 degraded prefixes
and their partial-prune history, and completes the 256th repair, final prune, settlement,
and compaction from the reservation. Its pinned subvisitor then settles 256 degradations
before governed-set absence, crashes and resumes broker compaction plus source release,
reclaims all 256 live slots, admits a new repair, and reaches a zero-backlog final state;
every row, multiplier, count, cleanup target, and release boundary has a poison. The
continuity malformed-prefix id independently hooks binding-without-source-receipt,
source-only and broker-only binding, evidence-without-binding/pre,
watermark-before-evidence, watermark-before-required-prune, before-day-90 unrelated prune,
day-90 success without the whole-set proof or empty census, duplicate
binding/pre/evidence/watermark/decision-basis/decision-selection/decision-pre/intent/outcome,
decision-basis/decision-selection/decision/intent/outcome
mismatch, source replay change, replay-binding/handle/sequence/predecessor mismatch,
terminal/settlement failure substitution, multi-member omission/reorder/substitution, and
every deadline/outcome cross-pair.
Every malformed hook has its own removal poison. The evidence-retention case independently
hooks missing and mismatched binding, pre, durable evidence, repaired watermark, settlement,
outcome, first/intermediate/final prune, reduced-census, final-absence, and mandatory-prune
exports; proven never-durable and post-bound-unlink evidence absence versus illicit missing
durable evidence; degraded zero-watermark admission and unexpected-watermark refusal;
file-only export; incomplete repair; governed-set-present finalization refusal; all repaired and degraded
finalization admissions plus degraded-attempt reclamation while the governed set remains;
complete target-set/current-head/successor proof and stale/current-watermark negatives;
compaction pre/outcome, recovery generation, every target
present/intent/absent-without-intent/absent-under-intent/old-census/unlink/parent-sync/census/
receipt prefix, restart at every target ordinal, head advance/predecessor change/target
substitution after every completed ordinal and before final commit, targets-compacted
receipt, source release, attempt-slice release, source-Released/slice-pending no-reuse, and
completed no-write replay,
again with one removal poison per hook. The same grouped case has independent fresh-process
source admission/pre-audit/pin/bind/broker-binding/release prefixes, anchored fd-relative
source races, source-prefix sealed reconciliation and source-release permit compiler/API
negatives, replay-key candidate/create/sync/recycling/outcome prefixes,
zero-pre-link/one-post-link counts, pre-parent exact reuse or bounded absent recycling
before resampling, repeated commitment/crash/absent-final constant-state completion,
post-parent absent degradation, and
secure-posture/missing/partial/replaced negatives. It also owns write-ahead
decision-basis-intent, decision-basis, decision-selection, decision-pre, outcome-intent,
terminal, and final-absence publication at every common boundary,
intrinsic incomplete-prefix actions, completed-prefix rejection, and repaired/degraded
exact settlement, including pre-witness absent/exact intent and basis replay, the
post-ancestor-sync intent-witness pair, source/failure change after witness-backed
write-ahead intent durability, sealed intent/basis/selection conflict, and independent
intent/basis removal at every downstream durable prefix with no reconstruction or later
publication. The grouped
continuity-permit cases run the full negative matrix separately for repair, compaction,
source release, and source-prefix reconciliation permits. The redaction case visits every
private identifier, replay key, source binding,
source pin/replay/release record, candidate commitment, continuity body member, audit-field
record-family allowlist, observable surface, `Display` negative, and exact redacted `Debug`
implementation independently, with one canary-removal poison per
identifier/member/surface/seal. The continuity identity vectors, source-binding
attempt/receipt vectors, replay-key attempt and candidate/recycling vectors,
watermark/prune vectors,
prune-history/final-absence vectors, terminal/export/target/successor/head/compaction
vectors, degraded-reclamation/target-intent/receipt/recovery/targets-compacted/
attempt-slice-release vectors, and settlement vectors, every member/domain/framing/order
perturbation, and every
cross-field substitution are literal and read-independent, with one removal poison each.
Each meta case
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
| `panelPolicy` | literal `unanimous-ten-role-exact-final-candidate`, retained only by this strict legacy record |

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
panel-pending + exact unanimous strict legacy fixed-ten F-bound attestations -> panel-satisfied
panel-satisfied -> seal-eligible -> merge-eligible -> merged-byte-identical-F
panel-pending + completed/terminal panel with any missing role, recommendation,
  disagreement, or stale binding
  -> panel-refused
```

`blocked`, `abandoned-unmerged`, and `panel-refused` authorize no seal, merge, successor
wave, or release. `recover-panel-without-new-request` authorizes only the externally defined
recovery-attestation surface linked to the retained request; it creates no second request and
cannot itself satisfy the panel. The validator requires the repository strict legacy fixed-ten
roster, `signoff = true` iff recommendations are empty, identical F/commit/tree/disposition
bindings, and every constitutional predecessor. No action or field can encode `waived`,
partial, force, reduced roster, stale-candidate attestation, or panel substitution. A content
or history change after F, or any failed recovered panel, returns to external escalation
rather than admitting another feature-local request.
