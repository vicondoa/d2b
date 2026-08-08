# Panel Artifact Contract

This contract defines contributor-tooling files only. It adds no protected
authority, service, principal, socket, capability, receipt resolver, or
migration controller.

## Version namespaces

The lifecycle selection is versioned independently from the unchanged delivery
artifacts:

| Contract | Current value | Rule |
| --- | --- | --- |
| Workspace delivery schema | `2` | `DELIVERY_SCHEMA_VERSION` does not change. |
| Selection table | `2` | Defines seat classes, triggers, floors, profiles, focus, and fill order. |
| Lifecycle selection artifact | `1` | Defines the candidate-bound selected roster consumed by both delivery consumers. |

Panel request, record, attestation, and seal serialization does not change.
There is no panel-specific delivery version or schema migration path.

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
is refused. Selection identity remains lifecycle-tooling state and is not
copied into delivery artifacts.

The two consumers are explicit:

```text
delivery wave panel-request --snapshot <path> --selection <path>
node .github/skills/d2b-panel-round/scripts/make-records.mjs \
  <round-dir> --selection <path>
```

`panel-request` requires the selection candidate triple, program, and wave to
equal the snapshot. `make-records.mjs` requires the same candidate triple as
`candidate.json` and verdict and observed-binding keys exactly equal to the
ordered roster. Both consumers accept only selection schema version `1` and
selection-table version `2`. Candidate, version, or roster disagreement fails
before output is written.

## Discovery request and result

The discovery request binds the full candidate, validation evidence, selected
roster, seat focus, and comprehensive discovery instruction.

Each selected seat must produce a result with:

- `seat`
- `complete: true`
- `findings`, an ordered array that may be empty

Every finding records the seat, source ordinal, severity, impact, and
recommendation. A missing selected-seat result is an error. The explicit
positive result `{ "complete": true, "findings": [] }` is accepted and is
never inferred from absence.

## Ledger

The ledger records stable `R` identifiers and complete source-to-issue
mappings. The orchestrator supplies deduplication groups; generation validates
complete, exactly-once mapping and deterministic identifier order.

Every ledger identifier must have exactly one implementation response.
Verification preparation refuses a missing response instead of dropping the
issue.

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

A recorded acceptance is an optional plain object containing the accepting
repository username, capacity `repository maintainer` or `merge owner`, and a
concrete justification. It is ordinary review data under existing repository
controls, not authentication or protected authorization.

Approval applies this matrix before reviewer sign-off:

| Severity | Approving response | Refusal |
| --- | --- | --- |
| `BLOCKER` | Fixed with evidence, or Invalid or Withdrawn with verified factual status and evidence | Intentionally rejected or Deferred, even if an acceptance object is present |
| `MAJOR` | Fixed with evidence, or any non-Fixed response with recorded acceptance by the repository maintainer or merge owner; Invalid and Withdrawn still require verified factual status | Any non-Fixed response without that acceptance, including Deferred |
| `MINOR` or `NIT` | Any complete supported response | Missing or incomplete response data |

Verification artifacts carry every response, the applicable validation and
self-review evidence, the latest delta, full candidate context, prior status,
and seat obligations. `signoff` remains true exactly when blocking
recommendations are empty.

## Existing schema-version-2 delivery artifacts

The delivery structs and serialized fields remain exactly as they are:

- `PanelRequest` keeps `artifact_kind`, `schema_version`, `program`, `wave`,
  `candidate_id`, `content_id`, `snapshot_sha256`, `provider`,
  `model_version`, `reasoning_effort`, `roles`, `record_artifact_kind`,
  `record_schema_version`, and `record_files`.
- `PanelRecord` keeps `artifact_kind`, `schema_version`, `role`,
  `candidate_id`, `content_id`, `snapshot_sha256`, `model_version`, `provider`,
  `reasoning_effort`, `run_id`, `receipt_locator`, `output_sha256`, `signoff`,
  and `recommendations`.
- `PanelAttestation` keeps `roles`, `records`, and `unanimous`; each record
  keeps `role`, `file`, `sha256`, and `run_id`.
- `SealRecord` keeps `artifact_kind`, `schema_version`, `program`, `wave`,
  `candidate_id`, `content_id`, `snapshot_sha256`, `material`, `panel`,
  `panel_request_sha256`, and `evidence`.

`panel-request --selection` parses one strict `PanelSelectionV1`, validates the
candidate identity and ordered roster, and places that roster in the existing
`roles` and `record_files` fields. The allowed role enum includes every
current seat and retains `rust`; current selection excludes `rust`.

`make-records.mjs` parses the same selection artifact and emits one existing
`PanelRecord` per selected role. It refuses candidate, selection-schema,
selection-table-version, verdict-roster, observed-binding-roster, and emitted
record-roster mismatches.

`panel-attest` loads the stored request and treats its `roles` and
`record_files` as the exact authority. It refuses missing, extra, duplicate,
misnamed, or out-of-order records and requires unanimity for exactly that
request roster. Attestation and seal validation use the existing fields and do
not consult a global current-roster count.

An existing ten-seat schema-version-2 request and its records therefore remain
readable without conversion, including the `rust` role. Focused compatibility
tests use existing builders and parsers, including for that ten-seat case. No
raw DTO family, serialization-golden family, alternate panel envelope, or
shared schema migration machinery is introduced.

## Legacy lifecycle import

Lifecycle import recognizes the existing round format inside
`panel-lifecycle.mjs`; it is not a delivery schema migration path. Source
identity uses record digest, legacy seat, and recommendation ordinal. Raw
source text and attribution are retained. Exact bracketed prefixes map to
current severities; all other legacy text receives migration-assigned MAJOR.

Complete legacy rounds serve as discovery input. Partial legacy rounds retain
every completed source before the lifecycle's one current discovery. Legacy
`rust` remains the source attribution and maps only its current verification
responsibility to `software` with the Rust profile.
