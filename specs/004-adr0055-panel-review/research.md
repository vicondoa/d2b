# Research: Pragmatic Panel Review

## Decision: One atomic Track B cutover

**Rationale**: Selection, prompts, lifecycle artifacts, records, delivery
validation, and compatibility must agree at merge. A split delivery would
create the unsupported half-converted workflow that ADR 0055 rejects.

**Alternatives considered**:

- Multiple waves: rejected because the intermediate state would be unusable.
- A new service or crate: rejected as unnecessary for contributor tooling.

## Decision: Existing script and xtask surfaces

**Rationale**: The current panel already stages immutable diffs, dispatches
read-only agents, records observed bindings, and validates delivery records.
One focused JavaScript lifecycle helper plus small changes to the existing
shell and Rust validators is the shortest complete path.

**Alternatives considered**:

- Put the whole lifecycle in Rust: rejected as larger than the workflow needs.
- Keep orchestration entirely in prose: rejected because automatic artifacts,
  deterministic selection, and compatibility need executable validation.

## Decision: Request-bound selected roster

**Rationale**: The delivery request is already the candidate-bound statement of
what reviewers must attest. Recording the selected ordered roster there lets
attestation and seal validation require exactly that roster without a second
source of truth.

**Alternatives considered**:

- Keep a fixed global ten-seat roster: rejected by ADR 0055.
- Let each reviewer choose relevance: rejected because selection must be
  deterministic and shared.

## Decision: Version-first legacy transform

**Rationale**: Complete and partial legacy rounds already exist. Dispatching on
artifact version before parsing preserves their raw evidence and avoids making
the current parser accept two ambiguous shapes.

**Alternatives considered**:

- Force in-flight reviews to restart: rejected by the compatibility goal.
- Rewrite legacy records in place: rejected because history must remain
  readable and unchanged.

## Decision: Orchestrator supplies deduplication

**Rationale**: Deduplicating natural-language findings requires judgement. The
orchestrator assigns stable `R` identifiers and supplies source groupings;
tooling validates deterministic order and complete exactly-once mapping.

**Alternatives considered**:

- Automatic semantic clustering: rejected as unnecessary and less predictable.
- Manual operator copy/paste: rejected because it drops context and
  attribution.

## Decision: Informational metrics only

**Rationale**: Counts help improve discovery prompts but cannot prove review
quality. Approval remains based on resolved merge blockers, validation, no
regressions, and unanimous selected-roster sign-off.

**Alternatives considered**:

- Quality thresholds or reviewer scoring: rejected because they incentivize
  gaming an imperfect LLM process.
