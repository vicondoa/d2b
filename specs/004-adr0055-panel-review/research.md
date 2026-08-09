# Research: Pragmatic Panel Review

## Decision: One atomic Track B cutover

**Rationale**: Selection, prompts, lifecycle artifacts, records, delivery
validation, and compatibility must agree at merge. A split delivery would
create the unsupported half-converted workflow that ADR 0055 rejects.

**Alternatives considered**:

- Multiple waves: rejected because the intermediate state would be unusable.
- A new service or crate: rejected as unnecessary for contributor tooling.

## Decision: Existing script and xtask surfaces

**Rationale**: The current panel already stages immutable completed reviewer
packets, dispatches read-only agents, records observed bindings, and validates
delivery records. One focused JavaScript lifecycle helper plus small changes
to the existing shell and Rust validators is the shortest complete path.

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

## Decision: Reviewer verdicts and normalized adapter results stay distinct

**Rationale**: Reviewers already return the exact
`engineer/signoff/summary/recommendations` discovery schema. Lifecycle tooling
derives the exact `seat/complete/findings` discovery result. Naming both layers
prevents prompts from asking reviewers to manufacture internal normalization
fields and lets complete-result coverage remain mechanical.

**Alternatives considered**:

- Ask reviewers for normalized lifecycle objects: rejected because it couples
  the human review seam to internal ledger representation.
- Treat both shapes as interchangeable: rejected because permissive parsing
  hides missing seats and schema drift.

## Decision: Process evidence, not a security boundary

**Rationale**: Panel files document feedback, fixes, and verification under
ordinary repository controls. Deterministic bytes, digests, and immutable
completed packets catch mistakes and conflicting retries. They do not confer
privilege, authenticate a person, keep data secret, accept hostile inputs, or
protect against an adversarial process with the same UID.

Use create-or-compare for single files. Use one sibling-directory rename only
for the complete selected-seat verification request family and complete
selected-seat delivery record family.

**Alternatives considered**:

- Generic locks, fsync protocols, raw-syscall wrappers, procfs descriptor
  pinning, retention or quota systems, and filesystem transaction frameworks:
  rejected because they create a false security claim and unnecessary
  contributor-tooling machinery.
- Cryptographic signatures or identity lookup: rejected because panel review
  is a process gate, not authentication or authorization.

## Decision: Candidate snapshots track tree content only

**Rationale**: A candidate snapshot names reviewed repository content.
Disposition, acceptance, response, and evidence-only changes affect the review
packet, not the tree. They therefore use a new qualified round with the same
candidate digest triple. A tree change creates the new snapshot.

**Alternatives considered**:

- Mint a snapshot for every process-artifact update: rejected because it
  falsely represents evidence changes as code changes.
- Edit a completed packet in place: rejected because `.complete` byte-binds
  the immutable packet reviewed in that round.

## Decision: Informational metrics only

**Rationale**: Counts help improve discovery prompts but cannot prove review
quality. Approval remains based on resolved merge blockers, validation, no
regressions, and unanimous selected-roster sign-off.

**Alternatives considered**:

- Quality thresholds or reviewer scoring: rejected because they incentivize
  gaming an imperfect LLM process.
