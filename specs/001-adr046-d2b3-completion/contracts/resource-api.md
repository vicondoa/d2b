# Contract: Resource API and ComponentSession

**Owning specs**: `ADR-046-resource-api-and-authorization`, `ADR-046-componentsession-and-bus`,
`ADR-046-zone-routing`, `ADR-046-resource-store-redb`

## What this surface is

The `d2b.resource.v3` service is how every in-Zone component reads and writes resources. It is
reached only through an authenticated ComponentSession admitted by the Zone registrar. There
is no unauthenticated path, no wildcard subscription, and no direct store handle.

## Current state

The production redb engine, watch primitive, in-process controller fan-in fixture, and a
daemon Zone-runtime skeleton now exist. The plane remains **unreachable by design** because
the production composition is incomplete:

- transport dispatch is still unregistered (`UnregisteredBusAdapter`, reachability constant
  `AwaitingAuthenticatedComponentSessionRouter`);
- the daemon public bridge has a verified peer role but no registrar-admitted
  ComponentSession and correctly refuses to treat that role as a resource subject;
- the daemon installs no policy, pins mutable policy/configuration/controller revisions to
  bootstrap constants when constructing store identity, and keeps policy readiness false;
- controller endpoint registration and authenticated watch admission exist only in fixtures
  or tests, not as owned production handles;
- `Provider/system-core` exists, but production readiness is not yet backed by one
  `d2b-core-controller`-owned registration plus the required live handler observations, and
  the unreleased v3 `ZoneHandlerName` enum still awaits T605's two approved values;
- production store construction deliberately disables the audit callback and retains the
  mutation outbox without a production drainer; and
- the core controller has no production durable effect/adoption ledger.

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
| RA-7 | Publish ResourceService only from a registrar-consumed authenticated ComponentSession, with the registrar's authoritative subject propagated internally and no request subject field | FR-066, SC-030 | W5 |
| RA-8 | Install and recover policy under `ZoneResourceRuntime`: consume one sealed non-Clone `PolicyBootstrapRead` for the first exact-revision envelope snapshot, then use authenticated Resource API reads/updates only; keep both stores policy-neutral | FR-067, FR-073 | W5 |
| RA-9 | Register the production controller endpoint and admit its watch through ResourceService, ZoneBus, the production store, and controller fan-in | FR-068, FR-069 | W5 |
| RA-10 | Persist every committed effect and cleanup intent before dispatch, replay/adopt it after restart, and complete cleanup only for the same UID and exact nonzero revision | FR-068, SC-031 | W5 |
| RA-11 | Drain mutation audit outboxes durably and idempotently before ordinary success; after commit-before-drain represent `CommittedPendingAudit` with the existing layered `ResourceStatus` composite, keep the Zone degraded/unpublished, and make same-ID retry observe one mutation and eventual final result | FR-070, SC-032 | W5 |
| RA-12 | Reopen advanced mutable revisions from durable metadata and isolate per-Zone startup/close failures without dropping later Zones | FR-071, SC-033 | W5 |
| RA-13 | Keep all RBAC DTO deserialization, compilation, and ownership outside both store crates | FR-073, D106 | W5 |
| RA-14 | Bind amended-plan resume to T603's external reconciliation and editor progress receipts, and bind W5 acceptance to exact-candidate production-boundary, operator activation/effect/cleanup, RSS, fan-in, restart, audit, removal, and reference evidence | FR-072, SC-034 | W5 |
| RA-15 | Make the readiness Provider member exactly the `d2b-core-controller`-owned `Provider/system-core` registration plus exactly one `Zone.status.handlers[]` record named `system-core-host` and one named `system-core-user`, each carrying phase/timestamp from the active, initialized, current `HostReconciler` or `UserReconciler`; reject duplicates, missing/wrong names, and `ProviderLifecycle` substitution; do not wait for other W6 dossiers | FR-069, SC-033 | W5 |
| RA-16 | Under Constitution 2.2.0, add the two omitted closed-enum values and move their Rust round-trip/list tests, compiler-derived public/private API snapshots, paired runtime reference, unchanged desired-Zone-schema proof, consumers/emitters, and exact-candidate evidence in the same Wave 5 PR | FR-072, SC-033, SC-034 | W5 |

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
- **Policy has one lifecycle owner.** `ZoneResourceRuntime` owns install/recovery and
  publication; `NativeAuthorizer` interprets an immutable installed set. Initial install and
  restart recovery consume one private, sealed, non-`Clone` `PolicyBootstrapRead` that can
  return only this Zone's policy-input envelopes at the exact live revision. It carries no
  public Resource API subject and has no general read, mutation, clone, or reuse path. After
  install, all policy access uses an authenticated Resource API session and normal revision
  rules. redb stores policy-neutral envelopes and revisions and never interprets RBAC.
- **Readiness is a projection of owners, not flags.** Store recovery, matching policy,
  registrar/router session, controller endpoint, watch cursor, audit catch-up, mandatory
  controller health, and the live `d2b-core-controller`-owned `Provider/system-core`
  registration are one aggregate. The status projection is the actual
  `Zone.status.handlers[]` list with exactly one `system-core-host` and one
  `system-core-user` record, each carrying `phase` and `lastReconciledAt` from active,
  initialized, current `HostReconciler` and `UserReconciler` instances. Duplicate, missing,
  or wrong-name records and the distinct `ProviderLifecycle` value fail closed. No partial
  route, detached status, or boolean is published; a failure degrades only that Zone.
- **The C1 version decision is closed but implementation is pending.**
  `ZoneHandlerName::SystemCoreHost` and `ZoneHandlerName::SystemCoreUser` use the existing
  kebab-case serialization. No field, operation, or desired Zone schema changes, and v3 is
  unreleased, so no `apiVersion`, `schemaVersion`, `manifestVersion`, `bundleVersion`, or
  wire-field version changes. T605 must still prove current API snapshots and a
  byte-identical generated desired Zone schema.
- **Effect and audit recovery precede publication.** A committed effect intent and a durable
  audit outbox survive restart; neither may be forgotten, bypassed, or treated as ready. A
  commit whose audit is pending returns semantic `CommittedPendingAudit` through the existing
  layered `ResourceStatus`: `ResourceStatus.phase = ResourcePhase::Degraded`;
  `ResourceStatus.outcome.code = StatusCode("committed-pending-audit")` with retryable safe
  remediation and no raw sink detail; `ResourceStatus.update.state = UpdateState::Blocked`;
  and `ResourceStatus.update.operation_id = Some(original_operation_id)`. Existing condition,
  outcome, and update fields remain bounded and redacted. `ResourceUpdateStatus` has no phase
  or status-code member, and this semantic state adds no enum variant, field, or schema
  version. Same-ID retry never reapplies the mutation and returns the one stored final result
  after drain recovery.
- **Mutable revisions are not identity constants.** Store/Zone UUIDs are immutable open
  checks. Policy, configuration, and controller revisions are recovered values.

## Acceptance

- A component with valid peer evidence completes registrar admission, performs an authorized
  operation and watch through the registered ZoneBus route, and is refused an unauthorized
  or cross-Zone operation, with the denial audited.
- A component presenting a self-named subject, a reused admission, or only a public daemon
  peer role is refused before ResourceService.
- Conformance evidence shows a registered backend mutates only through verified admission and
  exposes no independent write path, plus a recorded security review of each registered
  backend. The `adr046w5` seal must not close without both.
- Policy acceptance covers initial bootstrap installation, authenticated revision advance,
  and restart recovery. It proves the bootstrap capability is consumed, cannot clone or read
  a non-policy resource, carries no public subject, and leaves the Zone unpublished on every
  revision/identity/compile failure while D106 remains clean.
- Restart evidence covers every generation-commit/effect-ledger/dispatch/completion crash
  window and the stale, zero, wrong-UID, and ambiguous cleanup negatives.
- Audit evidence proves durable append and outbox completion precede ordinary mutation
  success. Commit-before-drain returns only the exact `ResourceStatus` composite above;
  same-ID retries apply once and converge on one final result, different-ID retries retain
  normal revision/conflict behavior, status remains visible across restart, and replay yields
  exactly one logical record by operation ID plus mutation ordinal.
- The full readiness projection publishes no partial path; one failed Zone is reported while
  later unrelated Zones still open and close. Removing the `Provider/system-core`
  registration or either required `Zone.status.handlers[]` record in turn degrades only that
  Zone; duplicates, a missing/wrong name, and `provider-lifecycle` substitution are rejected.
- All acceptance evidence names one exact candidate and uses production owners. T604 starts
  at the emitted operator Nix declaration/bundle, activates through the production daemon,
  observes a real owned effect or precise actionable refusal for the representative Guest,
  Volume, Network, and Device, then proves dependency-safe removal without affecting
  unrelated resources. Direct `WatchService`, `ProductionWatchHarness`, a fake endpoint, a
  fixed subject, or an older result artifact is ineligible.
- Before T589, T603 writes the closed external receipt, accounts for all T073-T218
  obligations, and binds current analysis plus a unanimous plan receipt at
  `adr046w5-r<n>` to the same amended artifact snapshot/tip and unambiguous
  branch/candidate. Only an identity-verified `/d2b-spec-edit` batch may then check
  T073-T218 and T603. T589 requires that editor receipt and those checkboxes. Before T219, no
  reconciled obligation remains open and both evidence manifests contain T604's
  exact-candidate result plus T605's variant/list, targeted-contract-test, current
  API-snapshot, paired-reference, and unchanged desired-schema results.
