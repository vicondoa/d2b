# Specification quality checklist: Complete the d2b Provider Control Plane

**Feature**: [spec.md](../spec.md)

This checklist verifies that product requirements are complete, testable, and traceable.

## Content quality

- [x] Technical detail is limited to product architecture, security, compatibility, recovery,
  and validation contracts.
- [x] The specification is focused on operator value and user-visible outcomes.
- [x] Implementers and operators can identify the owning component and failure behavior.
- [x] Mandatory specification sections are complete and contain no unresolved placeholders.

## Requirement completeness

- [x] Requirements are testable and unambiguous.
- [x] Success criteria are measurable and buildable.
- [x] Primary resource, Provider, cutover, recovery, and release scenarios are defined.
- [x] Edge cases cover stale state, replay, crash windows, partial readiness, failed export,
  capability mismatch, compatibility loss, and unavailable recovery evidence.
- [x] Scope, dependencies, assumptions, and external operator-owned backup behavior are clear.
- [x] Network/Host east-west behavior requires the double opt-in
  `Network.spec.isolation.allowEastWest && d2b.site.allowUnsafeEastWest`, with both inputs
  defaulting false and all four combinations tested.
- [x] Host-generation recovery is broker-coordinator-owned, transfers durably exactly once,
  survives broker and daemon failures through existing units, and never treats daemon identity
  or euid 0 as authorization.
- [x] Recovery-point evidence binds candidate, commit, tree, preview, host, operator, and
  restore instructions and fails closed on malformed, stale, expired, or mismatched records.
- [x] Companion compatibility, capability parity, explicit retirement, changelog treatment,
  and conditional live/hardware validation are specified.

## Feature readiness

- [x] Every functional requirement has an acceptance condition.
- [x] User scenarios cover the primary flows and negative outcomes.
- [x] Implementation-specific constraints are deliberate and traceable to a product contract
  or focused test.

## Validation notes

Focused tests for changed components are required. Container, host, live, hardware, and
performance lanes are conditional on the changed surface. `make check` is available as an
optional broader check, not a mandatory pre-PR or pre-review step. Advisory results do not
constitute enforcing evidence.
