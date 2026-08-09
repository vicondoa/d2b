# Panel Artifact Contract

This contract defines contributor-tooling process evidence only. Panel
artifacts are not a privilege, authentication, secrecy, hostile-input, or
adversarial same-UID security boundary. Digests and deterministic bytes detect
workflow mistakes and conflicting regeneration; they are not signatures,
cryptographic authority, or protection from a process that can already modify
the contributor's files.

The contract adds no protected authority, service, principal, socket,
capability, receipt resolver, or migration controller. Maintainer acceptance
is a plain recorded response under ordinary repository controls. Its claimed
username and capacity are not verified through signatures, GitHub API lookup,
another service, or an authoritative identity mechanism.

## Version namespaces

The lifecycle selection and panel format use independent namespaces inside the
unchanged workspace delivery schema:

| Contract | Current value | Rule |
| --- | --- | --- |
| Workspace delivery schema | `2` | `DELIVERY_SCHEMA_VERSION` does not change. |
| Selection table | `2` | Defines seat classes, triggers, floors, profiles, focus, and fill order. |
| Lifecycle selection artifact | `1` | Defines the candidate-bound selected roster consumed by both delivery consumers. |
| Current panel format | `1` | Required on current request, record, attestation, and the seal's embedded panel object. |
| Legacy panel format | field absent | Strict fixed-ten compatibility format, including `rust`. |

The current panel format adds only `panel_format_version`. The seal's top-level
field set and workspace schema version do not change. The lifecycle selection
remains version `1` and stays the single roster artifact consumed by both
delivery consumers.

## Lifecycle selection

The lifecycle helper writes one immutable selection for each reviewed candidate
state:

```text
.scratch/panel/<lifecycle>/selections/<candidate-id>/<snapshot-sha256>.json
```

The strict `PanelSelectionV1` DTO has these closed fields:

- `artifact_kind`: `d2b-panel/lifecycle-selection`
- `schema_version`: `1`
- `lifecycle_id`
- `phase`: `discovery` or `verification`
- `program`
- `wave`
- `candidate_id`
- `content_id`
- `snapshot_sha256`
- `selection_table_version`: `2`
- `candidate_class`
- `classification_inputs`
- `ambiguity_widened`
- `profiles`
- `roster`: the ordered lifecycle roster after monotonic union

Identical selection inputs render identical bytes, and conflicting regeneration
is refused. A selection is process evidence, not cryptographic authority.
Selection identity remains lifecycle-tooling state and is not copied into
delivery artifacts.

Only a repository tree change creates a new candidate snapshot and therefore a
new candidate-bound selection. A disposition change, acceptance update,
response correction, or evidence-only update keeps the existing candidate
digest triple. Such an update still requires a new qualified review round, but
it does not pretend that the reviewed tree changed.

The two consumers are explicit:

```text
delivery wave panel-request --snapshot <path> --selection <path>
node .github/skills/d2b-panel-round/scripts/make-records.mjs \
  <round-dir> \
  --selection <path> \
  --ledger <round-dir>/discovery-ledger.json \
  --responses <round-dir>/responses.json \
  --verification-results <round-dir>/verification-results.json \
  --approval <round-dir>/approval.json
```

`panel-request` requires the selection candidate triple, program, and wave to
equal the snapshot. `make-records.mjs` requires the same candidate triple as
the round-local staged `current-candidate.json` and verdict and
observed-binding keys exactly equal to the ordered roster. It also requires the
explicit canonical ledger path, response path, adapted verification-result
path, and approval path. Both consumers accept only selection schema version
`1` and selection-table version `2`. Candidate, version, roster, or
canonical-artifact disagreement fails before output is written.

## Discovery request and result

The discovery request binds the full candidate, validation evidence, selected
roster, seat focus, and comprehensive discovery instruction.

Each selected reviewer returns exactly this discovery verdict layer:

- `engineer`: the selected seat
- `signoff`: true if and only if `recommendations` is empty
- `summary`: a non-blank string
- `recommendations`: an ordered array that may be empty

Each recommendation has exactly `severity`, `where`, `what`, `why`, and `fix`.
The adapter validates the reviewer verdict and produces exactly this normalized
per-seat discovery result layer:

- `seat`
- `complete: true`
- `findings`, an ordered array derived from `recommendations`

Every normalized finding records the seat, source ordinal, raw text,
attribution, normalized severity, impact, and recommendation. Reviewers do not
return `seat`, `complete`, or `findings`; those fields exist only after the
adapter succeeds. A missing selected-seat verdict is an error. A reviewer
verdict with an empty `recommendations` array adapts to the explicit positive
result `{ "complete": true, "findings": [] }`; absence is never treated as
zero findings.

Verification reviewers return the same four base fields plus exactly
`verified_issue_statuses` and `late_findings`. The verification adapter emits
`seat`, `complete`, `summary`, `signoff`, `verified_issue_statuses`,
`blocking_recommendations`, `recommendations`, and `late_findings`. These
reviewer and adapter schemas are distinct and closed.

## Ledger

The ledger records stable `R` identifiers and complete source-to-issue
mappings. Every actionable discovery finding enters the ledger regardless of
severity. The orchestrator supplies deduplication groups; generation validates
complete, exactly-once mapping and deterministic identifier order.

Every ledger identifier must have exactly one implementation response.
Verification preparation refuses a missing response instead of dropping the
issue. MINOR and NIT are non-blocking only after they have a complete supported
response and verification has processed them; severity never permits omission
from the ledger or response envelope.

## Implementation responses and approval

The disposition enum remains exactly:

- Fixed
- Intentionally rejected
- Deferred
- Withdrawn
- Invalid

No Accepted disposition is added. A response contains the ledger identifier,
disposition, changed surface, justification, and evidence. Fixed requires a
declared change and non-empty evidence. Intentionally rejected and Deferred
require a concrete justification. Withdrawn and Invalid require a concrete
factual-status statement and non-empty evidence that verifies it.

A recorded `acceptance` is optional except where the approval matrix requires
it. When present, it is a strict closed JSON object with exactly these fields
and no others:

- `accepter`: a string naming the claimed repository username, non-blank after
  whitespace trimming;
- `capacity`: a string enum exactly `repository maintainer` or `merge owner`;
  and
- `justification`: a string, non-blank after whitespace trimming.

For an unresolved Intentionally rejected or Deferred MAJOR, a missing
`acceptance`; null, array, or scalar value; missing required field; extra
field; non-string field; empty or whitespace-only `accepter` or
`justification`; or empty, whitespace-only, or otherwise out-of-enum
`capacity` is malformed and cannot approve. The object is ordinary review data
under existing repository controls, not authentication or protected
authorization. Validation checks only this shape and content. It does not
verify the claimed identity or capacity, call GitHub, require a signature, or
consult another authority.

Approval applies this matrix before reviewer sign-off:

| Severity | Approving response | Refusal |
| --- | --- | --- |
| `BLOCKER` | Fixed with evidence, or Invalid or Withdrawn with verified factual status and evidence | Intentionally rejected or Deferred, even if an acceptance object is present |
| `MAJOR` | Fixed with evidence; Invalid or Withdrawn with verified factual status and evidence; or Intentionally rejected or Deferred with recorded maintainer or merge-owner acceptance | Unverified Invalid or Withdrawn, or Intentionally rejected or Deferred without recorded acceptance |
| `MINOR` or `NIT` | Any complete supported response | Missing or incomplete response data |

Verification artifacts carry every response, the applicable validation and
self-review evidence, the latest delta, full candidate context, prior status,
and seat obligations. After discovery, verification preparation requires an
explicit current candidate, prior selection, explicit actual delta, and
non-empty full candidate context. The actual delta may have no changed paths
when only dispositions, acceptance, responses, or evidence changed. Only
changed tree paths create a new candidate snapshot. `signoff` remains true
exactly when blocking recommendations are empty.

The approval artifact is bound to the exact UTF-8 bytes of the immutable
discovery ledger, implementation response envelope, and adapted
verification-result artifact. It records `discovery_ledger_sha256`,
`response_sha256`, and `verification_results_sha256`; approval and record
generation refuse any digest mismatch. Record generation recomputes approval
from those exact bytes rather than trusting a changed approval field.

An adapted verification-result artifact contains the current selection digest,
current candidate address, immutable ledger digest, and one complete result per
selected seat. A final issue status is passing only when it is exactly
`resolved` or `verified`; every other status is blocking even when the verdict
claims sign-off. The status map must cover every ledger issue exactly once.

## Reviewer packet completion and publication

The completed `.complete` marker byte-binds the immutable reviewer packet. Its
artifact digest and byte-count maps enumerate the reviewer-visible packet
files. The marker is written last. Before it exists, the directory is
incomplete and non-authoritative. After it exists, every enumerated packet file
must continue to match its bound bytes. Evidence, reviewer notes, agent
definitions, requests, diffs, selections, and other bound inputs are not
editable scratch.

Any reviewer packet content change requires a new qualified round identifier
and a new `.complete` marker. The prior completed packet remains unchanged.
This applies even when the tree candidate is unchanged because only a
disposition, response, acceptance, or evidence changed.

Ordinary single-file outputs, including selection files and `.complete`, use
simple create-or-compare publication: create an absent destination, accept an
existing byte-identical file, and refuse an existing conflicting file. The
only atomic family-directory publication is for the selected-seat verification
request family and the selected-seat delivery record family. Each family is
built in a unique sibling directory and renamed to an absent destination;
existing output is accepted only when membership and bytes match exactly.

This is a cooperative contributor workflow, not a filesystem security or
durability protocol. It deliberately rejects generic lock management, fsync
protocols, raw-syscall wrappers, procfs descriptor pinning, retention or quota
systems, and filesystem transaction frameworks. No such machinery may be
added to claim adversarial same-UID protection. A conflict fails with the
remedy to restore the expected bytes or use a new qualified round.

Final metrics validate the canonical selection, ledger, response envelope, and
adapted verification-result bytes. They require complete verification and emit
`status: "complete"`, `degraded: false`, and `verification_complete: true`.
Incomplete or absent verification is refused for final metrics rather than
reported as a complete round. Metrics remain descriptive and never become an
approval threshold.

## Panel delivery formats under workspace schema version 2

Current artifacts use panel format version `1`:

- Current `PanelRequest` carries `panel_format_version: 1` plus the existing
  `artifact_kind`, `schema_version`, `program`, `wave`, `candidate_id`,
  `content_id`, `snapshot_sha256`, `provider`, `model_version`,
  `reasoning_effort`, `roles`, `record_artifact_kind`,
  `record_schema_version`, and `record_files`.
- Current `PanelRecord` carries `panel_format_version: 1` plus the existing
  `artifact_kind`, `schema_version`, `role`, `candidate_id`, `content_id`,
  `snapshot_sha256`, `model_version`, `provider`, `reasoning_effort`, `run_id`,
  `receipt_locator`, `output_sha256`, `signoff`, and `recommendations`.
- Current `PanelAttestation` carries `panel_format_version: 1`, `roles`,
  `records`, and `unanimous`; each attested record retains `role`, `file`,
  `sha256`, and `run_id`.
- Current `SealRecord` retains its top-level `artifact_kind`, `schema_version`,
  `program`, `wave`, `candidate_id`, `content_id`, `snapshot_sha256`,
  `material`, `panel`, `panel_request_sha256`, and `evidence`. Its embedded
  `panel` object is the current attestation and therefore carries
  `panel_format_version: 1`.

`panel-request --selection` parses one strict `PanelSelectionV1`, validates the
candidate identity and ordered roster, and places that roster in the existing
`roles` and `record_files` fields. It writes current panel format version `1`.
The current role enum contains exactly the thirteen seats in selection-table
version `2`: `software`, `test`, `product`, `docs`, `security`,
`observability`, `simplicity`, `reliability`, `agentic`, `nixos`,
`networking`, `kernel`, and `build`. Current selection and current delivery
artifacts do not admit `rust`; Rust depth is the `software` Rust profile.

`make-records.mjs` parses the same selection artifact and emits one current
`PanelRecord` with `panel_format_version: 1` per selected role. It
refuses candidate, selection-schema, selection-table-version, verdict-roster,
observed-binding-roster, and emitted-record-roster mismatches.

`panel-attest` loads the stored request and treats its `roles` and
`record_files` as the exact authority. It refuses missing, extra, duplicate,
misnamed, or out-of-order records and requires unanimity for exactly that
request roster. A current request admits only current records and produces a
current attestation. Attestation and seal validation do not consult a global
current-roster count.

Legacy schema-version-2 request, record, attestation, and seal artifacts omit
`panel_format_version`. Their strict legacy DTOs retain the original field sets
listed above, the original role domain, and exactly the historical ordered ten
roles:

```text
software, test, nixos, networking, security, rust, product, docs,
observability, kernel
```

Legacy validation never treats absence as a variable current roster. An
existing legacy request may finish only with strict legacy records,
attestation, and an embedded legacy panel object. It remains readable without
conversion; no legacy artifact is rewritten.

### Version-first parsing

Every delivery artifact read is bounded by the existing delivery JSON byte
limit before format selection:

1. Probe only `panel_format_version` at the top level for a request, record, or
   attestation, and at `panel.panel_format_version` for a seal.
2. If the field is absent, select the strict legacy DTO. If it is the integer
   `1`, select the strict current DTO.
3. Refuse a null, non-integer, misplaced, or unknown value.
4. Deserialize exactly the selected DTO with unknown fields denied. A parse or
   validation failure never retries the other family.
5. Require one family across the request, every record, the attestation, and
   the seal's embedded panel object. Mixed-family data is refused.

This is explicit dispatch, not permissive `untagged` fallback. It prevents a
malformed current artifact from being accepted as legacy merely because one
field was omitted or misspelled.

Compatibility is pinned by exactly two compact checked-in fixture bundles:

- `packages/xtask/src/delivery/testdata/panel-legacy-ten.json`, containing one
  strict legacy request, ten ordered records including `rust`, its attestation,
  and its seal panel object; and
- `packages/xtask/src/delivery/testdata/panel-current-variable.json`,
  containing one strict current variable roster with an expanded-domain seat,
  its records, attestation, and seal panel object.

No third fixture family, broad artifact migration system, alternate panel
envelope, shared schema migration machinery, service, or authority is added.

## Legacy lifecycle import

Lifecycle import recognizes the existing round format inside
`panel-lifecycle.mjs`; it is not a delivery schema migration path. Source
identity uses record digest, legacy seat, and recommendation ordinal. Raw
source text and attribution are retained. Exact bracketed prefixes map to
current severities; all other legacy text receives migration-assigned MAJOR.

Complete legacy rounds serve as discovery input. Partial legacy rounds retain
every completed source before the lifecycle's one current discovery. Legacy
`rust` remains the source attribution and maps only its current verification
responsibility to `software` with the Rust profile. A JSON or object import is
partial by default, even when it contains all ten roles: it may suppress the
one current discovery only when exact record bytes and attestation digests
were supplied and validated. A directory import may be complete after strict
validation of the request, fixed-ten record filenames and bindings, exact
record bytes, and attestation digests. Without those proofs the import retains
all completed sources and requests exactly one current discovery.
