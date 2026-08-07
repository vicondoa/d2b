# Contracts: Complete the ADR-046 Provider Control Plane (d2b 3.0)

**Feature**: `001-adr046-d2b3-completion` | **Date**: 2026-07-29

d2b exposes contracts to operators, to guest components, to Nix consumers, and to sibling
desktop companions. This directory records **which contract surfaces this program must
deliver, change, or retire**, and what "done" means for each.

These files are an index and an obligation list. They do not restate field-level schemas: the
normative definitions live in the ADR-046 specification set, and the machine-readable form is
generated into `docs/reference/schemas/v3/` by `xtask`. Duplicating them here would create a
third source of truth that the drift gates do not check.

## Contract surfaces

| File | Surface | Consumers | Wave |
| --- | --- | --- | --- |
| [resource-api.md](./resource-api.md) | The `d2b.resource.v3` service and ComponentSession admission | In-Zone components, controllers, Providers | W2-W5 |
| [operator-cli.md](./operator-cli.md) | The `d2b` command surface | Human operators, companion tools | W5 |
| [nix-configuration.md](./nix-configuration.md) | `d2b.zones.<zone>.resources.*` option schema | Host configurations | W2, W5 |
| [generated-artifacts.md](./generated-artifacts.md) | Schemas, per-Zone bundles, UI colors, delivery artifacts | Broker, daemon, companions, drift gates | W2-W7 |
| [companion-contracts.md](./companion-contracts.md) | What the five desktop companions consume | Sibling repositories | W5 publish, W8 verify |
| [Candidate recovery prerequisite v1](#candidate-recovery-prerequisite-v1) | Immutable-candidate failure closure and successor admission | Plan integrator, delivery tooling, panel process | Historical W2 entry; requalified at close if unattested |

## Candidate recovery prerequisite v1

**Contract id**: `adr046-candidate-recovery-prerequisite/v1`

**Owner**: T008 and the ADR046 plan integrator

**Status**: historical W2 entry attestation; fail-closed remedial disposition required before
W2 close when contemporaneous entry evidence is absent

The feature-local sequencing contract is:

1. one immutable candidate receives at most one binding request;
2. a nonunanimous candidate is durably closed as failed and retains its request and records;
3. the active candidate slot is not released until that closure is durable;
4. only a distinct successor with the failed predecessor's complete recommendation,
   convergence, and candidate-bound validation identities may be admitted; and
5. same-candidate retry, two active candidates, stale or cross-wave recovery evidence, and
   any post-request content, history, or evidence move fail closed.

One canonical hermetic `candidate_recovery_v1` validator owns those five invariants and both
entry-disposition receipt shapes. Historical receipts contain exactly `historicalTask`,
`entryBaseCommit`, `entryBaseTree`, `firstDispatchCommit`, `recordedAtUnix`, the closed
prerequisite-locator set, and the closed original-check result set. Remedial receipts contain
exactly `historicalTask`, `historicalEntry: "unproven"`, `candidateId`, `candidateCommit`,
`candidateTree`, `recordedAtUnix`, the closed prerequisite-locator set, and the closed current
requalification-check result set. The validator rejects an unknown task, unknown or duplicate
field, duplicate receipt, both receipt variants, neither receipt variant, a missing or failed
check, a non-ancestor prerequisite or implementation head, a wrong entry/candidate
commit/tree, and a historical label attached to current evidence.

Its table-driven suite starts with one valid state and one valid receipt variant, then changes
exactly one field, binding, or transition at a time. The case inventory is itself asserted and
must cover every required and forbidden field in both receipt variants; candidate, program,
wave, request, round, commit, tree, recommendations, convergence, and validation bindings;
same-candidate second request; alternate candidate while active; release before durable
failure closure; successor admission before durable release; missing or stale recovery
evidence; cross-candidate and cross-wave evidence; and post-request content, history, or
evidence movement. A prose assertion or one happy-path test is not this matrix.

This contract does not amend the external ADR or tooling by assertion. Before T008 may
complete, the ADR046 plan integrator owns a separate external scope escalation that must
merge all of the following as one accepted policy generation:

- a new or superseding ADR plus its `docs/adr/README.md` index row;
- the coordinated `docs/specs/ADR-046-validation-and-delivery.md` and generated spec-manifest
  amendment;
- delivery implementation and tests under `packages/xtask/src/delivery/`; and
- matching `AGENTS.md` and `docs/contributing/` panel/delivery guidance.

T008's evidence record must name accepted commit locators for all four scopes (locators may
coincide when one commit owns multiple scopes) and prove each is an ancestor of its W2 base.
It must also record successful, nonempty execution of:

```bash
set -euo pipefail
listed="$(cargo test --manifest-path packages/Cargo.toml -p xtask \
  candidate_recovery_v1 -- --list)"
ignored="$(cargo test --manifest-path packages/Cargo.toml -p xtask \
  candidate_recovery_v1 -- --list --ignored)"
test "$(printf '%s\n' "$listed" |
  awk '/candidate_recovery_v1.*: test$/ { n += 1 } END { print n + 0 }')" -ge 1
test "$(printf '%s\n' "$ignored" |
  awk '/candidate_recovery_v1.*: test$/ { n += 1 } END { print n + 0 }')" -eq 0
cargo test --manifest-path packages/Cargo.toml -p xtask candidate_recovery_v1
make test-adr-index-coverage
make test-lint
```

`set -e` makes either Cargo listing failure fatal before a count is evaluated; no `grep -c`
pipeline may turn a failed listing into a numeric result. An ignored or skipped test, zero
discovered `candidate_recovery_v1` tests, an unmerged scope, or wording that still permits
only one request for the whole wave leaves T008 open. Downstream W2 work now exists while T008
remains unchecked, so a successful current rerun cannot be presented as historical entry
evidence. T008 may close only from a retained receipt bound to the actual W2 entry base and
first dispatch.

If that receipt does not exist, T008 remains unchecked. Exact frozen F2 must instead carry one
passing `EvidenceRecord` with validation `historical-entry-remediation-t008`; its external
receipt records `historicalEntry: "unproven"`, binds the F2 candidate/commit/tree, names the
accepted prerequisite commits, proves every W2 implementation head is an ancestor, and
records successful nonempty execution of the command set above plus current lineage,
destination, cleanliness, and fast-suite checks. T029 refuses panel request, seal, or merge
when neither the exact historical receipt nor this single remedial record exists, or when
either record is duplicated, malformed, failed, or bound to another commit or tree. This
requalification does not assert that original W2 entry complied.

T589 later hardens accepted v1 with the `adr046w5` strict storage profile. It must see accepted
v1 on its own actual base, but it does not retroactively complete T008, T030, or T037.
T029, T036, and T071 invoke this same validator, not local predicates, before pre-panel
dispatch, panel request, panel-attest, seal, merge-target registration, merge eligibility,
and merge. Any matrix case that would pass at one of those boundaries leaves the
corresponding wave open.

## Recovery-point attestation validator v1

**Contract id**: `d2b-recovery-point-attestation/v1`

**Owner**: T548; consumers T580, T555, and T556

One hermetic validator owns canonical JSON shape, every FR-043 field and delivery binding,
bounded integer timestamps, checked expiration arithmetic, external locator resolution, and
candidate/commit/tree/preview/host/operator/restore-instruction matching. T580 uses it for
import; T555 and T556 invoke the same implementation at every close boundary. A stage-local
predicate copy is ineligible.

The table-driven negative suite begins with one valid record and varies exactly one field or
binding at a time. It covers every required top-level field and qualification member,
including wrong `operatorSubjectSha256` and `restoreInstructionsSha256`, as well as missing,
duplicate, extra, malformed, wrong-type, per-timestamp negative/fractional/out-of-range,
future-event, checked-add-overflow, stale, expired, and unresolvable cases. The prerequisite
is:

```bash
set -euo pipefail
listed="$(cargo test --manifest-path packages/Cargo.toml -p xtask \
  recovery_point_attestation_v1 -- --list)"
ignored="$(cargo test --manifest-path packages/Cargo.toml -p xtask \
  recovery_point_attestation_v1 -- --list --ignored)"
test "$(printf '%s\n' "$listed" |
  awk '/recovery_point_attestation_v1.*: test$/ { n += 1 } END { print n + 0 }')" -ge 1
test "$(printf '%s\n' "$ignored" |
  awk '/recovery_point_attestation_v1.*: test$/ { n += 1 } END { print n + 0 }')" -eq 0
cargo test --manifest-path packages/Cargo.toml -p xtask \
  recovery_point_attestation_v1
```

Cargo listing failure, zero discovery, any ignored matching test, any skip, or a failing
per-field case blocks import, pre-panel dispatch, panel request, panel-attest, seal,
merge-target registration, merge eligibility, and merge.

## Rules that apply to every surface

1. **Versioned, not silently changed.** Adding, removing, or renaming a field or operation
   bumps the relevant version, updates the paired schema and prose, and lands in the same
   change as the emitter (FR-031, constitution Principle IV).
2. **Generated artifacts are the contract.** The committed bytes under
   `docs/reference/schemas/v3/` are authoritative; `make test-drift` regenerates and requires
   a clean diff. A hand-edited schema is a gate failure, not a shortcut.
3. **Documentation ships with behavior.** A reference page affected by a change lands in the
   same wave, never deferred (FR-019).
4. **No compatibility layer.** 3.0 is a clean break. A surface being removed is removed, with
   a removal proof, in its own commit, after its successor is integrated and tested (FR-023).
5. **Retirement is explicit.** A capability may only disappear if it is on the retirement list
   with a justification and named in the release notes (FR-042).
6. **Nothing leaks.** No secret, credential, command output, raw host path, or PII crosses any
   of these surfaces into telemetry, audit, logs, or errors (FR-018).
7. **Production evidence crosses production boundaries.** Resource-plane acceptance enters
   through a registrar-admitted, pidfd-bound ComponentSession and the published ZoneBus route.
   Restart uses a fresh pidfd, and PID reuse, mismatch, `ESRCH`, or ambiguity denies. A direct
   ResourceService or `WatchService` call, fixed subject, fake endpoint, independent readiness
   flag, disabled audit owner, or result from another commit cannot satisfy the
   `adr046w5` gate
   (FR-066-FR-072).
8. **One readiness projection, no partial publication.** Store, matching policy, session/router,
   controller endpoint, watch admission, audit catch-up, and the
   `d2b-core-controller`-owned `Provider/system-core` registration publish together with
   exactly one `Zone.status.handlers[]` record named `system-core-host` and one named
   `system-core-user`, each carrying `phase` and `lastReconciledAt` from its live handler, or
   the affected Zone refuses with remediation. `ProviderLifecycle` is distinct and cannot
   substitute. No other Wave 6 Provider dossier gates this wave (FR-069).
9. **Policy bootstrap is private and one-shot.** `ZoneResourceRuntime` may consume one sealed,
   private-issuer, non-`Clone`, non-`Copy` `PolicyBootstrapRead` for the first exact-revision
   policy-envelope snapshot. Compiler/API/external seals forbid public construction, default,
   conversion, reconstruction, or reuse. It carries no public Resource API subject or general
   read/mutation surface; all later policy access uses authenticated Resource API revision
   rules, and both store crates remain policy-neutral (FR-067, FR-073).
10. **A committed mutation is never unaudited or reported as rolled back.** Its immutable
    authoritative journal row commits in the same transaction. Until separate segment export
    and completion finish, return operation-bound `CommittedPendingAudit` through the layered
    `ResourceStatus`:
    `phase = ResourcePhase::Degraded`,
    `outcome.code = StatusCode("committed-pending-audit")`,
    `update.state = UpdateState::Blocked`, and
    `update.operation_id = Some(original_operation_id)`. Existing condition, outcome, and
    update detail stays bounded and redacted. Additive protobuf `PendingAuditStatus` carries
    the composite on every mutation response, including delete. Keep the Zone unpublished,
    require exact subject/Zone/request/target/verb/revision/idempotency replay binding, and
    make same-ID retry observe rather than reapply the mutation. Audit/export identifiers are
    fixed domain-separated digests, and retention/prune failure is typed degraded health
    (FR-070).
11. **Amended-plan resume is receipt-bound.** T603 is the sole direct prerequisite of T589
    but pre-validator analysis and plan panel at A/P0 authorize only its two validator source
    paths plus `changelog.d/delivery-resume-reconciliation.md`. Validator-and-fragment commit
    V becomes B, P remains byte-identical to P0, and analysis
    plus the plan panel rerun at B/P before T603 writes immutable authorization R using
    repository identity plus a relative feature path. If all rows and post-validator
    analysis/panel identities pass, only the validator-derived P-to-Q `/d2b-spec-edit` batch
    may check T073-T218 and T603. The Wave 5 integrator owns exact child commit C; T589
    requires finalized progress receipt E, clean HEAD C, and the checked task set. T602 later
    validates the B-to-C ancestry/snapshots and separate final-candidate F/tree evidence
    (FR-072, SC-034).
12. **Operator activation is acceptance evidence.** T604 starts from the emitted Nix
    resource declaration and per-Zone bundle, activates on startup and public declaration and
    removal switches without manual restart, observes the spec-pinned Provider/config/effect
    and readiness for `Volume/acceptance-state`, `Network/acceptance-net`, and
    `Device/acceptance-tpm`, then removes only the Device and proves its state-preserving
    cleanup without disturbing the ready, identity-stable, unrecreated acceptance
    Volume/Network or unrelated resources. Network implementation remains owned by Wave 4.
    Guest runtime-effect acceptance
    is deferred specifically to Wave 6 `Provider/runtime-cloud-hypervisor` T384/T479/T480;
    Guest emission, status, or refusal cannot
    satisfy this partial US1 production-plane checkpoint. Refusals are
    separate negative cases. The exact
    candidate result is emitted once by T600 as
    `operator-nix-activation-cleanup` and is required by T602 and T219.
13. **C1 is a coordinated unreleased-v3 correction.** Constitution 2.2.0 authorizes T605 to
    add `ZoneHandlerName::SystemCoreHost` and `ZoneHandlerName::SystemCoreUser`, serialized only
    as `system-core-host` and `system-core-user`; underscore spellings remain internal
    telemetry labels. Both governing normative specs and their version metadata move with
    targeted Rust/contract tests, compiler-derived public/private API snapshots, paired
    reference status text, and byte-identical Zone desired-schema proof. T605 completes on
    those owned pre-consumer artifacts; T595 consumes the variants, T599 reconciles other
    consumers, and T220 reconciles generated spec manifests plus the full drift gate in the
    same Wave 5 PR. C1 changes no desired Zone field or JSON schema version. Implementation
    remains pending.
14. **Exact-candidate evidence and close are closed.** T220 converges every repository change
    before freezing F. T600 owns exactly `production-session-watch`,
    `effect-replay-cleanup`, `audit-drain-replay`, `system-core-handler-contract`, and
    `operator-nix-activation-cleanup`; T601 owns exactly
    `resource-plane-rss-owner-fanin`, `wave5-removal-proofs`, and
    `cli-reference-conformance`. T602 rejects any unknown, duplicate, missing, extra,
    wrong-lane, or conflated identifier. T219 alone runs F's one binding panel, seal, and
    tree-preserving merge. F cannot change or receive a second request. Nonunanimity retains F
    as failed and routes scoped fixes through a fully revalidated successor and
    delta/full-context follow-up panel before that candidate's one request.
15. **SC-002 evidence is typed and census-closed.** Only
    `operator-nix-activation-cleanup` carries
    `EvidencePayload::Sc002ActivationLiveV1`. Its version-1, 16,384-byte-bounded,
    fixed-redacted payload contains one common monotonic start and exactly one sample for each
    of `Volume/acceptance-state`, `Network/acceptance-net`, and
    `Device/acceptance-tpm`. Effect, production Ready, selected-stop, and bounded progress
    observations repeat the sample identity. T589's one validator runs at import, durable
    reopen, panel-request, seal, and merge-eligibility and rejects every missing, malformed,
    misordered, stale, progress-free, over-budget, missing-sample, duplicate-sample,
    mixed-identity, or unrelated-sample case.
