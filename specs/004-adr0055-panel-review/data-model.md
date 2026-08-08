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
- optional recorded acceptance naming the repository maintainer or merge
  owner, that capacity, and the acceptance justification

Rules:

- Every ledger issue has exactly one response. A missing response is refused.
- Fixed requires the changed surface and non-empty evidence.
- Intentionally rejected and Deferred require a concrete justification.
- Withdrawn and Invalid require a verified factual status and non-empty
  supporting evidence.
- Recorded acceptance is ordinary repository process data. It is not a
  protected principal, authorization, capability, or separate service.
- A BLOCKER approves only as Fixed, Invalid, or Withdrawn. Invalid and
  Withdrawn must satisfy the verified-factual-status rule; Intentionally
  rejected and Deferred cannot approve a BLOCKER.
- A MAJOR approves when Fixed. Any non-Fixed MAJOR also requires recorded
  acceptance by the repository maintainer or merge owner; Deferred without
  that acceptance cannot approve.
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

## Existing delivery schema integration

Fields:

- the existing schema-version-2 panel request fields, including `roles` and
  `record_files`
- the existing schema-version-2 panel record fields, including `role` and the
  candidate digest triple
- the existing panel attestation fields: `roles`, `records`, and `unanimous`
- the existing schema-version-2 seal fields and embedded panel attestation

Rules:

- `DELIVERY_SCHEMA_VERSION` remains `2`.
- `panel-request --selection` validates selection schema version `1`,
  selection-table version `2`, candidate identity, and ordered roster, then
  populates the request's existing `roles` and `record_files`.
- `make-records.mjs` validates the same selection artifact and emits the
  existing `PanelRecord` shape for exactly its ordered roles.
- `panel-attest` uses the stored request as the roster authority. It rejects
  every missing, extra, duplicated, misnamed, or out-of-order record.
- Request, record, attestation, and seal gain no lifecycle identifier,
  selection digest, panel schema version, or other new delivery field.
- The allowed `PanelRole` domain retains `rust` for existing ten-seat data.
  Current lifecycle selection excludes `rust` and assigns Rust review through
  the `software` profile.
- Focused compatibility tests use existing helpers, including for the ten-seat
  legacy case; no raw DTO or serialization-golden families are introduced.

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
