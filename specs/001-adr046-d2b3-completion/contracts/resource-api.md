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
| RA-7 | Publish ResourceService only from a registrar-consumed authenticated ComponentSession, with the accepted socket transferred over `SCM_RIGHTS` to the typed broker `OpenPeerPidfdFromAcceptedSocket` operation and only an `OwnedFd` pidfd returned; keep raw `SO_PEERPIDFD` solely in the approved broker `sys.rs` FFI quarantine with narrow allowances and per-block `SAFETY:` comments; verify credential/generation/cgroup/liveness against that fd; retain one private registrar issuer, no public evidence accessor or bootstrap mint, authoritative subject propagation, and no request subject field; reject the `nix` wrapper, a new project FFI crate, numeric-PID lookup, and any local session fallback | FR-066, SC-030 | W5 |
| RA-8 | Install and recover policy under `ZoneResourceRuntime`: consume one private-issuer, compiler/API-sealed, non-fabricable one-shot `PolicyBootstrapRead` for the first exact-revision envelope snapshot, then use authenticated Resource API reads/updates only; keep both stores policy-neutral | FR-067, FR-073 | W5 |
| RA-9 | Register the production controller endpoint and admit its watch through ResourceService, ZoneBus, the production store, and controller fan-in | FR-068, FR-069 | W5 |
| RA-10 | Persist every committed effect and cleanup intent before dispatch, replay/adopt it after restart, and complete cleanup only for the same UID and exact nonzero revision | FR-068, SC-031 | W5 |
| RA-11 | Commit an immutable authoritative audit journal row transactionally with each mutation, export through a root-owned fd-anchored segment owner by typed fixed digest plus ordinal with file/directory durability, prune journal rows only after durable export plus bounded retention, represent export-pending `CommittedPendingAudit` on every mutation response including delete, expose a typed durable `InspectOperation` method, and require exact replay-binding before same-ID observation/resumption | FR-070, SC-032 | W5 |
| RA-12 | Reopen advanced mutable revisions from durable metadata and isolate per-Zone startup/close failures without dropping later Zones | FR-071, SC-033 | W5 |
| RA-13 | Keep all RBAC DTO deserialization, compilation, and ownership outside both store crates | FR-073, D106 | W5 |
| RA-14 | Bind amended-plan reconciliation to one clean-base analysis and current selected-roster lifecycle, one `/d2b-spec-edit` batch that changes exactly T073-T218 plus T603 when every row is satisfied, one dedicated checkbox-only Git commit, and fresh post-edit analysis plus a new selected-roster lifecycle before T589. The editor receipt and Git history are the sole authority; T603 owns no source, fragment, sidecar, digest chain, or resume receipt. Before T220 freezes F, install the hermetic exact-seven Wave 5 evidence validator at panel-request/panel-attest/seal/merge-eligibility with all six negative classes; then require those seven identifiers before T219 evaluates the accepted external disposition. `operator-nix-activation-cleanup` remains T604-owned W6 evidence imported only by T479. | FR-072, SC-034 | W5 |
| RA-15 | Make the readiness Provider member exactly the `d2b-core-controller`-owned `Provider/system-core` registration plus exactly one `Zone.status.handlers[]` record named `system-core-host` and one named `system-core-user`, each carrying phase/timestamp from the active, initialized, current `HostReconciler` or `UserReconciler`; reject duplicates, missing/wrong names, and `ProviderLifecycle` substitution; do not wait for other W6 dossiers | FR-069, SC-033 | W5 |
| RA-16 | Under Constitution 2.2.0, add the two omitted closed-enum values with exact kebab-case Zone wire names, retain underscore spellings only for internal telemetry labels, complete T605 on its owned normative/test/API/reference artifacts, reconcile the T595 emitter and T599 consumers, then reconcile generated manifests/full drift at T220 so all coordinated artifacts and exact-candidate evidence land in the same Wave 5 PR | FR-072, SC-033, SC-034 | W5 |

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
- **Unix process identity is socket-derived pidfd-bound.** `SO_PEERCRED` supplies attributes,
  not a durable process identity. Admission obtains the pidfd directly from the accepted
  socket with `SO_PEERPIDFD`; `pidfd_open(SO_PEERCRED.pid)` is forbidden. Credentials,
  generation, cgroup, and liveness are verified against and consumed with that exact fd by one
  registrar-private issuer. Unsupported kernels, numeric-PID reuse, dead fd, mismatch, or
  ambiguity refuse. The only syscall boundary is T592's wrapper in the approved broker
  `sys.rs` FFI quarantine. Its typed operation accepts exactly one accepted-socket ancillary
  fd and returns only an `OwnedFd` pidfd, validates exact `optlen` and `FD_CLOEXEC`, assumes
  ownership of any returned fd before later checks, and closes every failure path. The `nix`
  `MaybeUninit`/assert wrapper, a new project FFI crate, and any `d2b-session-unix` fallback
  are ineligible. Public peer evidence accessors and bootstrap-identity mint paths remain
  sealed.
- **Zone equality is proven before every capability mint.**
- **Policy has one lifecycle owner.** `ZoneResourceRuntime` owns install/recovery and
  publication; `NativeAuthorizer` interprets an immutable installed set. Initial install and
  restart recovery consume one private-issuer, sealed, non-`Clone`, non-`Copy`
  `PolicyBootstrapRead` that can
  return only this Zone's policy-input envelopes at the exact live revision. It carries no
  public Resource API subject and has no public construction, default, conversion, capability
  reconstruction, general read, mutation, clone, or reuse path. After
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
  `ZoneHandlerName::SystemCoreHost` and `ZoneHandlerName::SystemCoreUser` serialize only as
  `system-core-host` and `system-core-user`; `system_core_host` and `system_core_user` remain
  internal telemetry labels only. T605 bumps both governing normative specification versions.
  No desired Zone field or schema changes, and v3 is unreleased, so no `apiVersion`, JSON
  `schemaVersion`, `manifestVersion`, or `bundleVersion` changes. T605 must still prove
  current API snapshots and a byte-identical generated desired Zone schema.
- **Effect and audit recovery precede publication.** A committed effect intent and an
  immutable authoritative audit journal row survive restart; neither may be forgotten,
  bypassed, or treated as ready. Export completion is separate. Audit row constructors accept
  typed fixed digests; raw identifiers and trace context are excluded from rows and exports.
  The unprivileged runtime owns drain sequencing, but a typed broker op carrying only bounded
  fixed-digest records routes every root-owned mutation. The root broker alone owns the
  held-dirfd-relative segment append/rotation/export/prune path, and completion waits for file
  and directory durability. Exported rows become prune-eligible only after
  `audit.retentionDays`; configured record/byte limits and prune health are readiness inputs.
  A commit whose export is
  pending returns semantic `CommittedPendingAudit` through the layered `ResourceStatus`:
  `ResourceStatus.phase = ResourcePhase::Degraded`;
  `ResourceStatus.outcome.code = StatusCode("committed-pending-audit")` with retryable safe
  remediation and no raw sink detail; `ResourceStatus.update.state = UpdateState::Blocked`;
  and `ResourceStatus.update.operation_id = Some(original_operation_id)`. Existing condition,
  outcome, and update fields remain bounded and redacted. Every mutation response carries the
  composite, when pending, in additive protobuf `PendingAuditStatus`; `DeleteResponse` is not
  an exception. The ResourceService fingerprint changes, while Resource JSON versioning does
  not. Same-ID observation/resumption first matches the original authoritative subject, Zone,
  semantic request, target, verb, expected revision, and idempotency binding; mismatch denies,
  and an exact retry never reapplies the mutation.
- **Mutable revisions are not identity constants.** Store/Zone UUIDs are immutable open
  checks. Policy, configuration, and controller revisions are recovered values.

## Acceptance

- A component with valid peer evidence completes registrar admission, performs an authorized
  operation and watch through the registered ZoneBus route, and is refused an unauthorized
  or cross-Zone operation, with the denial audited.
- A component presenting a self-named subject, a reused admission, or only a public daemon
  peer role is refused before ResourceService. Unsupported `SO_PEERPIDFD`, numeric-only
  identity, PID reuse, dead fd, credential/generation/cgroup mismatch, ambiguity, and a stale
  pre-restart pidfd are also refused. Missing or extra ancillary fds, a raw-fd field, unsafe
  outside broker `sys.rs`, a missing per-block `SAFETY:` justification, a new project FFI
  crate, or a local session syscall blocks T593. API-surface/compile-fail checks expose no
  public issuer, verifier, clone, or peer evidence accessor.
- Conformance evidence shows a registered backend mutates only through verified admission and
  exposes no independent write path, plus a recorded security review of each registered
  backend. The `adr046w5` seal must not close without both.
- Policy acceptance covers initial bootstrap installation, authenticated revision advance,
  and restart recovery. It proves the bootstrap capability is consumed, cannot clone or read
  a non-policy resource, cannot be publicly constructed/defaulted/converted/reconstructed,
  carries no public subject, and leaves the Zone unpublished on every revision/identity/
  compile failure while compiler/API/external seals and D106 nonempty/poison checks remain
  clean.
- Restart evidence covers every generation-commit/effect-ledger/dispatch/completion crash
  window and the stale, zero, wrong-UID, and ambiguous cleanup negatives.
- Audit evidence proves the immutable authoritative row commits with each mutation and segment
  file/directory durability plus export completion precede ordinary success. Export-pending
  returns only the exact
  protobuf-represented `ResourceStatus` composite above; same-ID retries with matching replay
  bindings apply once and converge on one final result, cross-subject/Zone/request/restart
  mismatches deny, different-ID retries retain normal revision/conflict behavior, status
  remains visible across restart, multi-mutation replay yields one export per fixed digest
  plus ordinal, raw identifier/trace canaries never escape, fixed-digest and record-size
  constructor seals hold, configured segment and post-export journal retention limits hold,
  and prune/sync failure degrades health. `InspectOperation` traverses the typed durable
  backend and preserves wrong-binding indistinguishability across restart.
- The full readiness projection publishes no partial path; one failed Zone is reported while
  later unrelated Zones still open and close. Removing the `Provider/system-core`
  registration or either required `Zone.status.handlers[]` record in turn degrades only that
  Zone; duplicates, a missing/wrong name, and `provider-lifecycle` substitution are rejected.
- All acceptance evidence names one exact candidate and uses production owners. T604 starts
  at the emitted operator Nix declaration/bundle, activates on initial startup and public
  declaration/removal switches without manual daemon restart, observes the exact
  Provider/config/real-effect/readiness contract for `Volume/acceptance-state`,
  `Network/acceptance-net`, and `Device/acceptance-tpm`, then proves the pinned
  state-preserving Device removal without affecting the ready, identity-stable, unrecreated
  acceptance Volume/Network or unrelated resources. Network implementation remains owned by
  Wave 4. Guest runtime-effect acceptance
  remains specifically a Wave 6 `Provider/runtime-cloud-hypervisor` T384/T479/T480
  obligation; Guest emission, status, or refusal cannot
  satisfy this partial US1 production-plane checkpoint. Refusals are
  separate negative cases. Direct `WatchService`, `ProductionWatchHarness`, a fake endpoint, a fixed
  subject, or an older result artifact is ineligible.
- Before T589, analysis and one current selected-roster plan lifecycle bind clean base A and the complete feature snapshot. If all T073-T218 obligations are satisfied, one `/d2b-spec-edit` batch checks those rows plus T603 and the integrator creates dedicated checkbox-only commit C. The editor receipt and C are the sole authority. Fresh analysis and a new selected-roster lifecycle bind C and the changed snapshot; T603 owns no source, fragment, scratch receipt, sidecar, digest chain, or resume protocol.

  Before T219, no reconciled obligation
  remains open, T220 has converged and frozen clean exact F, C is an ancestor of F, and the
  F/tree-bound evidence union contains exactly the seven closed Wave 5 validation
  identifiers, as
  accepted by T589's pre-freeze validator that also guards panel-request, seal, and
  merge-eligibility and rejects missing, extra, duplicate, unknown, wrong-lane, and conflated
  fixtures.
  T604's W6 result appears only as `operator-nix-activation-cleanup`, is imported by T479,
  and is excluded from this union; the coordinated T605
  contract, T595 emitter, and T599 consumers appear only as
  `system-core-handler-contract`. Wave 5's retained request already consumed its binding
  surface. T219 performs no binding action; it may perform only a non-request
  close action expressly authorized by an accepted external disposition that preserves the
  historical request. F and its delivery history remain immutable.
  The refusal names the remediation: obtain an accepted external delivery-contract/tooling
  disposition identifying the retained request and an authorized non-request close action.
  It never recommends a successor, replacement candidate, second request, or feature-local
  status edit.
