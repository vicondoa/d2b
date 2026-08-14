# Specification coverage

**Feature**: [spec.md](./spec.md) | **Plan**: [plan.md](./plan.md) | **Tasks**: [tasks.md](./tasks.md)

This artifact maps product requirements to implementation areas and focused validation.
Existing ADR references below identify architectural rationale or the owner of existing code.

## Coverage rule

Every functional requirement has an implementation area, an observable acceptance condition,
and a focused validation path. A requirement is not satisfied by a skipped, advisory, status-
only, or unrelated test. Container, host, live, hardware, and performance checks are
conditional on the changed surface.

## Functional-requirement coverage

| Requirements | Product area | Implementation and evidence |
| --- | --- | --- |
| FR-001 - FR-011 | Resource objects, schemas, declaration, and routing | `d2b-contracts`, resource compiler, Nix emitters, Zone routing tests, schema and round-trip checks |
| FR-012 - FR-018 | Admission, authorization, audit, and redaction | ComponentSession registrar, broker operations, policy owner, audit journal/export tests, raw-identity negative fixtures |
| FR-019 | Documentation and public contracts | Affected reference pages, CLI/schema checks, companion inventory |
| FR-020 - FR-024 | Reset, cutover, removal, and recovery | Cutover broker, removal proofs, recovery validator, rollback and crash-window tests |
| FR-029 - FR-033 | Focused validation, determinism, performance, and test placement | Owning component tests, fixture contracts, runtime budgets, and conditional wider lanes |
| FR-037 - FR-038 | Product completion and release state | Release metadata, version checks, changelog, source/prebuilt fallback, publication identity |
| FR-039 - FR-042 | Desktop companion compatibility and capability parity | Companion inventory, exact release-tree exercises, explicit retirement and release-note checks |
| FR-043 | Recovery-point safety | Version-1 record validator, exact candidate/commit/tree/preview/host/operator/restore bindings, expiry and negative matrix |
| FR-045 | Release and preview boundaries | Public contract publication, no intermediate release, focused release validation |
| FR-047 | Architectural compatibility and implementation drift | Existing architectural references, committed code, and focused drift checks |
| FR-060 | Removal-proof scope | Removal proofs for paths actually retired, with successor coverage where promised |
| FR-061 - FR-065 | Provider completion and companion evidence | Provider implementation tests, operator acceptance identities, companion live checks |
| FR-066 - FR-075 | Production resource plane, handoff, audit, and readiness | Authenticated routes, pidfd identity, durable store, broker coordinator, exact handler names, pending-audit status, and host-continuity tests |

## Success-criteria coverage

| Criteria | Evidence |
| --- | --- |
| SC-001 - SC-004 | Operator-visible declaration, readiness, actionable refusal, and bounded latency tests |
| SC-005 - SC-011 | Durable store, replay, audit, redaction, and per-Zone isolation tests |
| SC-012 - SC-018 | Resource budgets, provider behavior, security boundaries, and typed EffectPort tests |
| SC-019 - SC-024 | Implementation completeness, companion inventory, and conditional live/hardware checks |
| SC-025 | Recovery-point qualification, expiry, binding, and fail-closed negative matrix |
| SC-026 - SC-035 | Cutover, compatibility, release metadata, removal proofs, and publication identity |

## Implementation ownership

- Provider and resource behavior is owned by the implementation task and technical contract
  named beside each task in `tasks.md`.
- T604 authors the operator-activation and daemon-restart fixtures; T479 executes the exact
  operator and Guest acceptance; T480 revalidates the resulting records and tree identity.
- The recovery validator is shared by every cutover stage and is not copied into local
  predicates.
- Generated schemas, API snapshots, Nix outputs, documentation, tests, and changelog entries
  move together when their owning contract changes.

## Validation policy

Focused tests are required for changed components. `make check` is optional and is not a
mandatory pre-PR or pre-review gate. Advisory jobs do not count as enforcing evidence.
