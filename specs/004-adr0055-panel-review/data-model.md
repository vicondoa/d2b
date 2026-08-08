# Data Model: Panel Review Lifecycle

## Panel lifecycle

Fields:

- version
- lifecycle identifier
- phase: discovery, fix, or verification
- candidate address and staged evidence references
- ordered selected roster
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
