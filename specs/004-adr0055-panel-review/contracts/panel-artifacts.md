# Panel Artifact Contract

All current artifacts carry a version and lifecycle identifier. Artifact paths
are generated under `.scratch/panel/<lifecycle>/`; accepted delivery records
continue to use existing candidate-addressed delivery state.

## Selection

Records the table version, candidate classification inputs, mandatory and
triggered seats, floor fill, profiles, ambiguity handling, and final ordered
roster.

## Discovery request and result

The request binds the full candidate, evidence, seat focus, and comprehensive
discovery instruction. Each source finding records seat, ordinal, severity,
impact, and recommendation.

## Ledger

Records stable `R` identifiers and complete source-to-issue mappings. The
orchestrator supplies deduplication groups; generation validates complete,
exactly-once mapping and deterministic identifier order.

## Implementation responses

Every ledger issue has one disposition, justification, changed-surface
declaration, and evidence. The allowed dispositions are Fixed, Intentionally
rejected, Deferred, Withdrawn, and Invalid.

## Verification request and result

Each selected seat receives the complete ledger, all responses and evidence,
the latest delta, full context, prior status, and its obligations. Results may
add only admitted late merge blockers or regressions. Blocking recommendations
remain strings compatible with existing delivery records.

## Legacy import

Legacy artifacts are detected by version before parsing. Source identity uses
record digest, legacy seat, and recommendation ordinal. Raw source text and
attribution are retained. Exact bracketed prefixes map to current severities;
all other legacy text receives migration-assigned MAJOR.

## Delivery request

New panel requests carry the selected ordered roster. Attestation and seal
require exactly one unanimous record per requested seat. Existing complete
ten-seat requests and records remain readable.
