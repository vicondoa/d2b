# Contract: Resource API and ComponentSession

**Owning specs**: `ADR-046-resource-api-and-authorization`, `ADR-046-componentsession-and-bus`,
`ADR-046-zone-routing`, `ADR-046-resource-store-redb`

## What this surface is

The `d2b.resource.v3` service is how every in-Zone component reads and writes resources. It is
reached only through an authenticated ComponentSession admitted by the Zone registrar. There
is no unauthenticated path, no wildcard subscription, and no direct store handle.

## Current state

Landed but **unreachable by design**. The service implements all 13 RPC methods and a full
RBAC evaluator, but:

- transport dispatch is unregistered (`UnregisteredBusAdapter`, reachability constant
  `AwaitingAuthenticatedComponentSessionRouter`);
- the subject resolver denies every peer outside test builds;
- the upgrade dispatcher defaults to returning provider-unavailable;
- the only store backends are test fakes - the production engine does not exist.

Making this reachable is the core of User Story 1 and the precondition for SC-021.

## Obligations

| # | Obligation | Requirement | Wave |
| --- | --- | --- | --- |
| RA-1 | Register the resource service behind an authenticated ComponentSession router; retire `UnregisteredBusAdapter` and its reachability constant | FR-001, SC-021 | W2-W5 |
| RA-2 | Supply an authoritative subject resolver owned by the Zone registrar, consuming verified peer evidence only | FR-008, SC-009 | W2 |
| RA-3 | Wire the production store backend behind the corrected engine; remove the test-only commit-proof issuance path | FR-006, SC-007 | W5 |
| RA-4 | Deliver replay and live watch with one global bounded admission budget, typed backpressure, and deterministic slow-watcher eviction with cursor resume | FR-002 | W5 |
| RA-5 | Enforce exact, subject-bound, revision-bound, Zone-checked routing on every operation | FR-009, SC-008 | W2 |
| RA-6 | Audit every denial | FR-007 | W2-W5 |

## Invariants that must not regress

- **Admission evidence is single-owner and consumed.** No clone, no accessor, no reuse. The
  capability mint surface is sealed in the compiler and inventoried by an allowlist; widening
  either is a deliberate trust-boundary change requiring a stated reason.
- **`SessionAuthority` stays sealed** by its private supertrait. A foreign implementation is a
  direct path to minting a genuine admission.
- **No caller-supplied subject.** There must be no public subject-configuration type and no
  raw-claim registration path. Production currently fails closed here; that is correct until
  an authoritative resolver is wired, and "fixing" it by accepting caller claims is the exact
  defect the W1 hardening rounds closed repeatedly.
- **Zone equality is proven before every capability mint.**

## Acceptance

- A component with valid peer evidence completes admission, performs an authorized operation,
  and is refused an unauthorized one, with the denial audited.
- A component presenting a self-named subject is refused.
- Conformance evidence shows a registered backend mutates only through verified admission and
  exposes no independent write path, plus a recorded security review of each registered
  backend. The W5 seal must not close without both.
