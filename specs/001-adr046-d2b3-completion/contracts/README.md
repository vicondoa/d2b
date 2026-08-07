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
| [companion-contracts.md](./companion-contracts.md) | What the four revision-2 desktop companions consume | Sibling repositories | W5 publish, W8 verify |
| [Candidate recovery prerequisite v1](#candidate-recovery-prerequisite-v1) | Immutable-candidate failure closure and external disposition | Plan integrator, delivery tooling, panel process | Historical W2 entry; requalified at close if unattested |

## Candidate recovery prerequisite v1

**Contract id**: `adr046-candidate-recovery-prerequisite/v1`

**Owner**: T008 and the ADR046 plan integrator

**Status**: historical W2 entry attestation; fail-closed remedial disposition required before
W2 close when contemporaneous entry evidence is absent

The feature-local sequencing contract is:

1. one immutable candidate receives at most one binding request;
2. a nonunanimous candidate is durably closed as failed and retains its request and records;
3. the active candidate slot is not released until that closure is durable;
4. no same-wave successor may receive another binding request; an accepted external
   disposition is required and may authorize only a non-request close action; and
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
failure closure; successor admission before or after durable release; missing or stale recovery
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
11. **Amended-plan resume is receipt-bound.** T603 is the sole in-feature direct prerequisite
    of T589; FR-070's accepted and installed source-generation compatibility floor is an
    additional external dispatch prerequisite. That external floor atomically owns the exact
    nonempty 13-member `SourceGenerationCompatibilityFloorV1` census in `data-model.md`.
    Its accepted disposition names the external producer/installer and typed
    import/validation authorities; the versioned manifest, installation, validation, and
    exact-C/Q import receipts must form the closed append-only chain before T589. Every role
    occurs once under one accepted disposition and source generation; `missing`, `duplicate`,
    `extra`, `empty`, `stale-generation`, `stale-digest`, and `cross-disposition` members
    refuse. The separately accepted external `ADR-046-validation-and-delivery` Version 2
    amendment owns the canonical JSON/digest/domain/framing policy, strict schemas, and
    checked-in vectors; the compatibility authorities produce and validate conforming
    objects but do not redefine those artifacts. Bare protocol 4
    refuses. T592 consumes those source artifacts read-only and
    owns only target-v5 adoption and target outputs. The caller-flake target executable stays
    unprivileged; only the separately broker-pinned installed apply object runs under `sudo`,
    and its connection-scoped peer pidfd/executable identity must remain an exact live match
    through each mutation with no persisted pidfd. Tests use the independent exact registry:
    six pre-first cases and 84 literal post-first cases across the fourteen later members of
    the closed 15-edge set; refusal occurs before the selected edge and all successors. Every
    raw apply-peer input in the complete fifteen-row registry remains absent from human,
    JSON, wire, error, log, span, metric, audit, panic, and `Debug` output. Only the typed
    process-instance and executable-identity correlation digests are permitted outside
    metrics, and metrics carry no identity. T603's pre-validator analysis and plan panel
    at A/P0 authorize only its two validator source
    paths plus `changelog.d/delivery-resume-reconciliation.md`. Validator-and-fragment commit
    V becomes B, P remains byte-identical to P0, and analysis
    plus the plan panel rerun at B/P before T603 writes immutable authorization R using
    repository identity plus a relative feature path. If all rows and post-validator
    analysis/panel identities pass, only the validator-derived P-to-Q `/d2b-spec-edit` batch
    may check T073-T218 and T603. The Wave 5 integrator owns exact child commit C; T589
    requires finalized progress receipt E, clean HEAD C, the checked task set, and fresh
    analysis plus unanimous plan review bound to exact C/Q. The P-to-Q content change makes
    B/P sign-off stale for T589 dispatch. T602 later
    validates the B-to-C ancestry/snapshots and separate final-candidate F/tree evidence
    (FR-072, SC-034).
12. **Operator activation is acceptance evidence.** T604 starts from the emitted Nix
    resource declaration and per-Zone bundle, activates on startup and public declaration and
    removal switches without manual restart, observes the spec-pinned Provider/config/effect
    and readiness for `Volume/acceptance-state`, `Network/acceptance-net`, and
    `Device/acceptance-tpm`, then removes only the Device and proves its state-preserving
    cleanup without disturbing the ready, identity-stable, unrecreated acceptance
    Volume/Network or unrelated resources. Network implementation remains owned by Wave 4.
    This one denied-east-west sample is not double-opt-in evidence. The untouched external
    Network specification remains sole-opt-in canon; W4 adjudication, T070, T071, and T220
    require its accepted versioned correction/migration plus all four Network/Host cases, and
    no feature-local status can unblock them.
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
    wrong-lane, or conflated identifier. Wave 5's retained `panel-request.json` has already
    consumed its binding surface. T219 performs no binding action and may perform
    only a non-request close action expressly authorized by an accepted external disposition
    that preserves the historical bytes. F and delivery history remain immutable.
15. **SC-002 evidence is typed and census-closed.** The schema-v2 `EvidenceRecord` remains
    unchanged. A passing `operator-nix-activation-cleanup` record uses its existing opaque
    locator to reference exactly one separately versioned `Sc002ActivationReceiptV1`. Its
    version-1, 16,384-byte-bounded, fixed-redacted receipt contains one common monotonic start
    and exactly one sample for each
    of `Volume/acceptance-state`, `Network/acceptance-net`, and
    `Device/acceptance-tpm`. Effect, production Ready, selected-stop, and bounded progress
    observations repeat the sample identity, and effect plus Ready must name the same typed
    resource identity. A failed operator record remains importable without a receipt but
    cannot satisfy a close stage; a failed record with a positive receipt is malformed.
    T604 emits only an external regular single-link receipt owned by the current effective uid
    with mode `0600`. T600 supplies it through T589's explicit
    `wave validate-import --sc002-receipt PATH` input and supplies no locator. T589 once-opens
    the source, computes only the typed domain-separated and length-framed receipt-content
    digest before decode, derives the typed content-address locator, validates the outer triplet, then
    installs the exact bytes beneath held current-effective-uid `0700` candidate dirfds as a
    current-effective-uid `0600` leaf. The importer uses a create-exclusive temp, file
    `fsync`, and `renameat2(RENAME_NOREPLACE)`, then `fsync`s every ancestor directory from
    `sha256` through the candidate directory before it publishes the `EvidenceRecord`.
    Every importer, cleanup worker, incident transition, successor admission, and retention
    guard holds the same verified candidate-scoped exclusive OFD lock through publication or
    return. Successful acquisition yields one private `CandidateSidecarGuard` that solely
    owns the locked `OwnedFd`; cleanup borrows it into
    `SidecarCleanupOwner<'guard>`. Every namespace open or cleanup mutation is a method on
    that borrow, so it cannot outlive, be paired with a later guard, or remain usable after
    lock release. Neither type exposes construction, raw-fd extraction, duplication,
    transfer, serialization, clone, conversion, or `'static` storage. A live owner cannot be cleaned; a nonblocking cleanup loser returns before any
    namespace open or mutation, and restart
    cleanup begins only after lock acquisition, moves the opened temp to a reserved
    quarantine name, reopens and verifies the same device/inode and full identity, derives
    the candidate/content/device/inode-bound retirement id, and moves the leaf no-replace
    into the bounded `evidence-sidecars/sc002/retired` subtree. It then reopens and
    revalidates the retired leaf and `fsync`s the leaf, both parents, and every changed
    ancestor. No sidecar data leaf is unlinked. Every name-consuming operation is
    `renameat2(RENAME_NOREPLACE)` followed by an fd-relative reopen and full moved-inode
    identity check; no check-then-unlink or name-only inode claim exists.

    An identity mismatch never unlinks or restores the suspect. It durably publishes the
    structured `Sc002IncidentPreimageV1` containing every applicable kind-specific
    component as a complete unnamed-inode/file-synced write-ahead record before it
    capability-free procfs-fd links that exact opened inode directly to the final
    no-replace preimage or publishes any other incident leaf. Only after durable preimage publication may it
    publish the kind-bearing incident anchor and complete metadata, then move the
    metadata-bound currently named inode to the
    typed incident payload address, reopens and verifies it, syncs the payload fd, both
    parents, and every changed ancestor, and append-only publishes `parked`. A replacement
    or rename/reopen mismatch is exactly `recovery-resumable` when one continuation remains
    and otherwise `recovery-irreconcilable`; every name is preserved, inspect returns the
    stable id/cause/remediation, and no parked status is fabricated. Recover is offered only
    for the resumable variant. Authenticated apply handles the irreconcilable variant by
    retaining representable names as durable residue or by publishing a complete recursively
    enumerated frozen primary-evidence census or identity-bearing bounded-failure commitment,
    then appending
    the separate resolution. The one recursive grammar contains every absent root, directory,
    regular-file, symlink, device, fifo, socket, mount, and other member under twelve exact
    root/root-instance pairs. Unavailable state is private denied scope only and all-zero
    `0xff` serialized observations refuse. It binds `st_uid`, `st_gid`, `st_rdev`, and
    symlink-target identity internally without rendering them. An admission-capable bounded
    failure embeds every descendant in two equal stable walks within the hard ceiling;
    unreadable, unstable, incomplete, depth-65, or over-hard-ceiling scope has null evidence,
    projects `restore-primary-evidence-coverage`, and denies request, apply, and admission
    until a fresh complete scan.
    The frozen primary scope binds every descendant path/content identity plus the canonical
    failure-path digest and excludes every resolution,
    resolution-evidence, successor-freeze, disposition-request, disposition, receipt, and successor leaf, so no digest contains
    itself. A raw `01ff` sentinel, copied commitment, or changed primary scope never
    authorizes successor admission. Invalid and unstable census causes remain inspectable and
    actionable through inspect `--json`, signed apply, and fresh-successor admission.
    Before signing, `sc002-disposition-request` durably freezes the clean successor triplet
    and emits the exact 19-field canonical authority request. The authority performs only
    the closed 19-to-22 transformation in `data-model.md`.     Unnamed output preparation precedes candidate publication; candidate request durability
    precedes capability-free procfs-fd linking of that exact opened inode directly to the
    final no-replace output name, final-inode verification, and parent sync. Unsupported open
    has zero internal mutation; unsupported link retains the internal pair. Exact replay is
    crash-safe at every anonymous and direct-final boundary, and every descriptor is
    CLOEXEC. Apply and admission rederive that same
    snapshot triplet and require the same freeze, request, and signed disposition. Ordinary paths and terminal incidents leave both ephemeral namespaces empty; neither
    nonterminal variant claims a terminal empty census.
    T589's private `CandidateRetentionOwner` is a zero-mutation recursive whole-scope
    retention guard: it preserves the canonical candidate root and all request, panel-record,
    evidence-record, receipt, seal, eligibility, merge, incident preimage/anchor/metadata/
    payload/residue/status, resolution-evidence/resolution, successor-freeze,
    disposition-request, disposition, and admission history. Every record repeats the same
    complete structured preimage and all kind-specific components. It never renames, tombstones, or deletes the candidate root or automatically
    unlinks any candidate descendant. Crash retry may
    reuse only an identical fully revalidated durable leaf; a different existing leaf or
    concurrent wrong-byte/binding input refuses.
    T589's one validator runs at import, durable reopen, panel-request/panel-attest, seal, and
    merge-eligibility and retains schema-v2 decoding while rejecting every missing,
    malformed, unknown-version/field/enum, over-bound, misordered, stale, progress-free,
    over-budget, missing-sample, duplicate-sample, mixed-identity, effect/Ready-disagreeing,
    unrelated-sample, wrong-owner/mode, pre-durability record publication, or crash/race case.
    Independent literal fixtures pin the complete receipt, retired-census,
    primary-evidence-census, source-floor 32-id receipt, 26-id issuer-authentication/
    capability, 21-id hash-vector, mutation-edge, 15-id post-mutation,
    15-id pre-start/root, 27-id unit-census, 26-id request-output, and both forbidden-value
    registries. The SC-002 census set has 73 ids and includes root-instance injectivity,
    invalid-node totality, depth-64 acceptance, depth-65 denial, directory completeness, and
    full-descendant bounded-failure refusal. The request-output set has 26 ids, and the
    recovery redaction set has seventeen rows including raw `st_uid`, `st_gid`, `st_rdev`,
    and symlink-target bytes.
    Source-floor signature validation
    consumes one private non-clonable `ProtectedSourceFloorOrigin` through private
    `AuthenticatedSourceFloorIssuerProvenance` into one validated-floor result. Later
    boundaries borrow/attenuate it; copied authority/key digests, origin replay, repeated
    mint, and serialized revalidation cannot produce authority. One shared
    nineteen-digest/one-signature SC-002 domain-hash
    golden is the oracle for every typed locator, incident, resolution, and disposition
    digest; raw SHA-256 locator definitions are ineligible.
16. **Recovery is never a status-only dead end.** Every closed SC-002 cause maps to one
    inspect/action/status/successor row. Incomplete descendant coverage remains inspectable
    with the exact bounded failure/root cause,
    `restore-primary-evidence-coverage`, null evidence, and `next-command: none`. It maps to
    one owner repair procedure and denies request/apply/admission until a fresh complete scan
    succeeds. The separate
    selector-free `HostGenerationHandoffStatusV1` projection gives each
    active, transfer-pending, recovery, and terminal handoff variant one exact
    state/phase/owner/action/successor tuple. Active and failed broker owners, including
    transfer-pending and rollback, are distinct. Terminal selection uses the authenticated
    current-intent pointer. Its 135 independent cases enumerate seven rollback members, 30
    audit members, and 15 transition edges plus mismatch, extra-mutation, pointer-auth, and
    shrinkage poisons. Selector-free pointer repair is distinct from immutable-audit
    restoration escalation. Recovery uses only the existing broker unit.
    Human/JSON schemas and both redaction registries remain synchronized; no path, fd,
    generation, raw identity, request body, or free-form remediation enters logs, audit,
    metrics, spans, errors, panic, or `Debug`.
