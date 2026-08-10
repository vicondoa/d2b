# Data Model: Panel Review Lifecycle

## Panel lifecycle

Fields:

- version
- lifecycle identifier
- phase: discovery, fix, or verification
- candidate address and staged evidence references
- current lifecycle-selection artifact reference
- discovery source findings
- issue ledger
- implementation responses
- verification iterations
- metrics

Rules:

- Discovery occurs at most once.
- The roster may widen after discovery but never narrow.
- Identical inputs render identical bytes.
- Existing conflicting bytes are never overwritten.
- Only a repository tree change creates a new candidate snapshot. Disposition,
  acceptance, response, and evidence-only updates retain the current candidate
  digest triple.

## Lifecycle selection artifact

Fields:

- artifact kind and selection schema version `1`
- lifecycle identifier and lifecycle phase
- program, wave, candidate id, content id, and snapshot SHA-256
- selection-table version `2`
- candidate classification inputs and ambiguity decision
- ordered selected roster and bound profiles

Rules:

- This is the one selected-roster artifact shape. Each reviewed candidate
  state has one immutable selection file; it is both candidate-bound and
  lifecycle-bound.
- xtask `delivery wave panel-request` and `make-records.mjs` consume the same
  artifact.
- The selection candidate digest triple must equal the xtask snapshot and the
  staged `current-candidate.json` digest triple.
- Only selection schema version `1` and selection-table version `2` are
  accepted.
- Request roles, verdict files, observed bindings, and emitted records must
  equal the artifact's ordered roster exactly.
- A mismatch in candidate, either version, or roster is refused before
  dispatch, request generation, record generation, or attestation.
- The selection is deterministic process evidence, not a signature,
  authentication claim, or cryptographic authority.

## Reviewer selection

Fields:

- table version
- candidate class
- changed paths and relevant content signals
- mandatory seats
- triggered optional seats
- floor-filled seats
- ordered final roster
- bound profiles
- ambiguity decision

Rules:

- Every mandatory and triggered seat is present.
- Code and operative configuration use a minimum of ten seats.
- Documentation-only changes use a minimum of eight seats.
- Ambiguity selects the wider result.
- The lifecycle roster is the set union of every accepted selection artifact
  in that lifecycle; it never narrows.

## Discovery reviewer verdict

Fields:

- `engineer`: selected reviewer seat
- `signoff`: boolean, true exactly when recommendations are empty
- `summary`: non-blank string
- `recommendations`: ordered array

Each recommendation contains exactly:

- `severity`: `critical`, `high`, `medium`, or `low`
- `where`
- `what`
- `why`
- `fix`

Rules:

- This is the exact reviewer-produced discovery layer.
- It contains no `seat`, `complete`, or `findings` field.
- The adapter validates the selected seat and closed schema before producing a
  discovery seat result.

## Discovery seat result

Fields:

- selected seat
- completion marker
- ordered source findings, possibly empty

Rules:

- This is adapter-produced normalized output, not reviewer output.
- Every seat in the selection artifact has exactly one complete discovery
  result.
- Absence is an error; it is never interpreted as zero findings.
- `{ complete: true, findings: [] }` is the explicit positive zero-finding
  result.
- A result for a seat outside the selected roster is refused.

## Source finding

Fields:

- immutable source identifier
- reviewer seat
- source ordinal
- raw finding text and attribution
- normalized severity
- impact
- recommendation
- migration-assigned severity flag

Rules:

- Every discovery finding maps to exactly one ledger issue.
- Raw legacy text and attribution remain unchanged.

## Ledger issue

Fields:

- stable lifecycle-local identifier `R1`, `R2`, and so on
- normalized description
- severity
- source finding identifiers
- implementation disposition
- justification
- evidence
- previous and current verification status
- late-discovery metadata when applicable

Rules:

- Identifiers never change or get reused.
- Every source appears exactly once across issue mappings.
- Every actionable discovery finding enters the ledger regardless of
  severity.
- Every issue has a disposition before verification.
- Late findings append new identifiers without renumbering prior issues.
- MINOR and NIT remain non-blocking only after complete response and
  verification processing.

## Implementation response

Fields:

- ledger issue identifier
- disposition: Fixed, Intentionally rejected, Deferred, Withdrawn, or Invalid
- concrete justification
- changed-surface declaration
- evidence
- verified factual status and supporting evidence when the disposition is
  Withdrawn or Invalid
- optional recorded `acceptance`, required for the unresolved MAJOR cases
  below, as a strict closed object containing exactly:
  - `accepter`: string, non-blank after whitespace trimming
  - `capacity`: string enum exactly `repository maintainer` or `merge owner`
  - `justification`: string, non-blank after whitespace trimming

Rules:

- Every ledger issue has exactly one response. A missing response is refused.
- Fixed requires the changed surface and non-empty evidence.
- Intentionally rejected and Deferred require a concrete justification.
- Withdrawn and Invalid require a verified factual status and non-empty
  supporting evidence.
- Recorded acceptance is ordinary repository process data. It is not a
  protected principal, authorization, capability, or separate service. Its
  claimed repository username and capacity are shape-checked but not resolved
  through signatures, GitHub API lookup, or another identity authority.
- A present `acceptance` is malformed unless it is a JSON object, not null or
  an array, with all three required fields and no extra fields. Every field
  must be a string. `accepter` and `justification` must remain non-empty after
  whitespace trimming, and `capacity` must equal one of its two enum values.
- A BLOCKER approves only as Fixed, Invalid, or Withdrawn. Invalid and
  Withdrawn must satisfy the verified-factual-status rule; Intentionally
  rejected and Deferred cannot approve a BLOCKER.
- A MAJOR is resolved as Fixed, or as Invalid or Withdrawn after satisfying the
  verified-factual-status rule. Those resolved factual dispositions require no
  acceptance.
- An unresolved Intentionally rejected or Deferred MAJOR requires recorded
  acceptance by the repository maintainer or merge owner. For either
  disposition, a missing acceptance; null, array, or scalar value; missing or
  extra field; non-string field; empty or whitespace-only string; or
  out-of-enum capacity cannot approve.
- MINOR and NIT responses remain in the ledger but do not block after their
  required response data is complete.
- Changing only a disposition, acceptance, response, or evidence does not
  create a new candidate snapshot. It does require a new qualified reviewer
  round when any completed reviewer packet input changes.

## Verification reviewer verdict

Fields:

- `engineer`
- `signoff`
- `summary`
- `verified_issue_statuses`
- `late_findings`
- `recommendations`

Rules:

- This is the exact reviewer-produced verification layer.
- It extends the four discovery base fields with exactly
  `verified_issue_statuses` and `late_findings`.
- `verified_issue_statuses` covers every ledger issue exactly once.
- `signoff` is true exactly when recommendations are empty.

## Verification result

Fields:

- reviewer seat and profiles
- verified issue statuses
- disposition validation
- regression findings
- admitted late issues
- blocking recommendations
- summary and sign-off

Rules:

- This is adapter-produced normalized output with exactly `seat`, `complete`,
  `summary`, `signoff`, `verified_issue_statuses`,
  `blocking_recommendations`, `recommendations`, and `late_findings`.
- `signoff` is true exactly when blocking recommendations are empty.
- Actionable late MINOR and NIT observations enter the ledger and become
  non-blocking only after complete processing.
- Approval evaluates the response rules above before reviewer unanimity.

## Completed reviewer packet

Fields:

- qualified round identifier
- `.complete` marker metadata
- exact relative path to SHA-256 map
- exact relative path to byte-count map
- candidate, selection, delta, and full-range bindings

Rules:

- The marker is created last and enumerates every reviewer-visible immutable
  packet input.
- Absence means the directory is incomplete and non-authoritative.
- A present marker requires every enumerated path to retain its exact bound
  bytes.
- Packet content is never edited in place. Any correction uses a new qualified
  round while preserving the completed prior packet.
- Packet digests are process-integrity checks, not signatures,
  authentication, secrecy, or adversarial same-UID protection.

## Publication

Rules:

- Single files use create-or-compare: create absent, accept byte-identical, and
  refuse conflict.
- Atomic sibling-directory rename is limited to the complete selected-seat
  verification request family and complete selected-seat delivery record
  family.
- No generic lock, fsync, raw-syscall, procfs-pinning, retention, quota, or
  filesystem transaction framework is part of the data model.

## Delivery panel format integration

Fields:

- workspace delivery schema version `2`
- current panel format version `1`
- current request fields, including `panel_format_version`, `roles`, and
  `record_files`
- current record fields, including `panel_format_version`, `role`, and the
  candidate digest triple
- current attestation fields: `panel_format_version`, `roles`, `records`, and
  `unanimous`
- the schema-version-2 seal fields, whose current embedded panel object carries
  `panel_format_version`
- strict legacy request, record, attestation, and seal DTOs whose panel objects
  omit `panel_format_version`

Rules:

- `DELIVERY_SCHEMA_VERSION` remains `2`.
- `panel-request --selection` validates selection schema version `1`,
  selection-table version `2`, candidate identity, and ordered roster, then
  populates `roles` and `record_files` and writes
  `panel_format_version: 1`.
- `make-records.mjs` validates the same selection artifact and emits the
  current `PanelRecord` shape with `panel_format_version: 1` for exactly its
  ordered roles.
- Every read is bounded by the existing delivery JSON limit. A shallow probe
  inspects the top-level discriminator for requests, records, and attestations,
  or `panel.panel_format_version` for seals, before any strict DTO is selected.
- An absent discriminator selects the legacy DTO. Integer value `1` selects
  the current DTO. A malformed, misplaced, or unknown value is refused.
  Failure to parse the selected strict DTO never falls back to the other
  family.
- Every DTO denies unknown fields. A request selects the family for all records,
  attestation, and its seal; mixing legacy and current artifacts is refused.
- `panel-attest` uses a current request's ordered roster as authority and
  rejects every missing, extra, duplicated, misnamed, or out-of-order current
  record.
- The current role domain is the thirteen-seat selection-table domain:
  `software`, `test`, `product`, `docs`, `security`, `observability`,
  `simplicity`, `reliability`, `agentic`, `nixos`, `networking`, `kernel`, and
  `build`. It excludes `rust`, whose current responsibility is a `software`
  profile.
- The legacy role domain and ordering remain exactly the historical fixed ten,
  including `rust`. Legacy validation does not accept a variable roster.
- Request, record, attestation, and seal gain no lifecycle identifier,
  selection digest, new workspace delivery schema, or top-level seal
  discriminator.
- Exactly two compact checked-in fixture bundles pin compatibility: one legacy
  fixed-ten set and one current variable-roster set. They cover request,
  records, attestation, and the seal's embedded panel object without creating
  a general migration or serialization-golden family.

## Metrics

Fields:

- initial unique findings
- late unique findings
- late BLOCKER count
- late MAJOR count
- review iterations
- implementation iterations
- average fixed issues per implementation iteration

Rules:

- A zero implementation-iteration denominator produces `0.0`.
- Metrics never affect approval.
