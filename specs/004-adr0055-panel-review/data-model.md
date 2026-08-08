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
  staged `candidate.json` digest triple.
- Only selection schema version `1` and selection-table version `2` are
  accepted.
- Request roles, verdict files, observed bindings, and emitted records must
  equal the artifact's ordered roster exactly.
- A mismatch in candidate, either version, or roster is refused before
  dispatch, request generation, record generation, or attestation.

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

## Discovery seat result

Fields:

- selected seat
- completion marker
- ordered source findings, possibly empty

Rules:

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
- Every issue has a disposition before verification.
- Late findings append new identifiers without renumbering prior issues.

## Implementation response

Fields:

- ledger issue identifier
- disposition: Fixed, Intentionally rejected, Deferred, Withdrawn, or Invalid
- concrete justification
- changed-surface declaration
- evidence
- verified factual status and supporting evidence when the disposition is
  Withdrawn or Invalid
- optional recorded acceptance containing a non-empty accepter identifier,
  capacity exactly `repository maintainer` or `merge owner`, and a non-empty
  acceptance justification

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
- A present acceptance object is malformed when the accepter identifier is
  empty, capacity is outside `repository maintainer` or `merge owner`, or the
  acceptance justification is empty.
- A BLOCKER approves only as Fixed, Invalid, or Withdrawn. Invalid and
  Withdrawn must satisfy the verified-factual-status rule; Intentionally
  rejected and Deferred cannot approve a BLOCKER.
- A MAJOR is resolved as Fixed, or as Invalid or Withdrawn after satisfying the
  verified-factual-status rule. Those resolved factual dispositions require no
  acceptance.
- An unresolved Intentionally rejected or Deferred MAJOR requires recorded
  acceptance by the repository maintainer or merge owner. Without it, or with a
  malformed acceptance object, the MAJOR cannot approve.
- MINOR and NIT responses remain in the ledger but do not block after their
  required response data is complete.

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

- `signoff` is true exactly when blocking recommendations are empty.
- Pre-existing late MINOR and NIT observations cannot become blockers.
- Approval evaluates the response rules above before reviewer unanimity.

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
