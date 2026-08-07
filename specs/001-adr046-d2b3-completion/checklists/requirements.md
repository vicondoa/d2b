# Specification Quality Checklist: Complete the ADR-046 Provider Control Plane (d2b 3.0)

**Purpose**: Validate specification completeness and quality before proceeding to planning
**Created**: 2026-07-29
**Feature**: [spec.md](../spec.md)

## Content Quality

- [x] Technical detail is limited to binding architecture, security, delivery, and validation contracts
- [x] Focused on operator value, user-visible outcomes, and program completion
- [x] Written for the technical implementers and operators who must apply the contracts
- [x] All mandatory sections completed

## Requirement Completeness

- [x] No [NEEDS CLARIFICATION] markers remain
- [x] Requirements are testable and unambiguous
- [x] Success criteria are measurable
- [x] Success criteria are buildable and name technical bindings where mechanical proof requires them
- [x] All acceptance scenarios are defined
- [x] Edge cases are identified
- [x] Scope is clearly bounded
- [x] Dependencies and assumptions identified
- [x] Wave 5 Nix acceptance names exactly `Volume/acceptance-state`, `Network/acceptance-net`,
  and `Device/acceptance-tpm`; each effect and production `Ready` projection is bound to that
  same identity, while Guest runtime-effect acceptance is explicitly deferred to Wave 6
  `Provider/runtime-cloud-hypervisor` T384/T479/T480
- [x] SC-002 uses a separately versioned typed receipt referenced by an unchanged schema-v2
  `EvidenceRecord`; failed operator records import without a receipt but cannot close
- [x] SC-002 recovery has closed inspect/recover/request/apply/successor transitions for parked,
  resumable, irreconcilable, and `evidence-census-conflict` states; its frozen primary scope
  uses one total injective root-instance/node grammar and recursively binds every absent,
  directory, file, symlink, device, fifo, socket, mount, and other descendant; unavailable
  state is private denied scope and an all-zero `0xff` serialized observation refuses;
  an incomplete, unstable, unreadable, depth-65, or over-hard-ceiling scan exposes null
  evidence, an exact bounded failure/root class, and
  `restore-primary-evidence-coverage` with no next command until its owner repair procedure
  restores coverage; it denies admission; the scope excludes
  resolution/request/disposition/freeze leaves, and raw `01ff` cannot authorize
- [x] SC-002 persists one structured incident preimage with every kind-specific component
  as a complete unnamed-inode/file-synced/procfs-fd direct-final-linked write-ahead record before every other
  incident publication, repeats it byte-identically in preimage/anchor/metadata/status/
  resolution/freeze/request/disposition/admission records, and classifies every crash boundary
- [x] Successor selection is durably frozen before signing; the canonical authority request,
  signed disposition, apply, and admit all bind the same successor triplet
- [x] The request has an exact 19-field schema and closed 19-to-22 disposition transform;
  `--request-out` uses anchored openat2, zero-capability/procfs validation, unnamed-inode
  file sync before candidate publication, exact-inode direct-final no-replace linking after
  candidate durability, final inode verification, parent sync, and exact replay; unsupported
  open has zero internal mutation and unsupported link retains the internal pair; every
  descriptor is CLOEXEC
- [x] Cleanup authority is `SidecarCleanupOwner<'guard>` borrowing the exact private
  `CandidateSidecarGuard`, and compile/API seals prevent stale-owner lifetime, fd
  reconstruction, duplication, transfer, serialization, or cross-guard use
- [x] Every SC-002 cause and every active or terminal host-generation variant has an exact
  inspect/state/phase/owner/action/successor tuple; failed transfer and rollback are separate
  restart variants, terminal selection uses the authenticated current pointer, and there is
  no daemon recovery owner or new unit
- [x] Literal expectations pin the 15 mutation edges, all 90 apply-peer ids, all 91
  source-floor poison ids, 15 pre-start/root ids, 27 unit ids, 73 census ids, 34
  direct-final publication ids, and seventeen recovery redaction rows independently from
  production
- [x] Recursive census goldens cover depth 64 success, depth 65 denial, every invalid node
  kind, unstable denial, `st_uid`/`st_gid`/`st_rdev`, and symlink-target identity
- [x] Handoff goldens and independent cases pin every valid tuple, exact human/JSON/errors,
  exits `0|2|3|4`, forbidden inspect inputs, current/terminal pointer selection, selector-free
  pointer repair restart/conflict/no-write behavior, bounded immutable-audit diagnostics,
  separate integrity escalation, and the exact 155 cases over seven rollback members, 32
  audit members, 15 transition edges, mismatches, extra mutation, pointer authentication,
  and shrinkage; Type-1 option eval cannot substitute for Type-10 VM proof
- [x] Cleanup is serialized against every importer, cleanup, incident, successor, and
  retention live owner before namespace access, and namespace operations require the private
  lifetime-bound `SidecarCleanupOwner<'guard>`; named legacy state is never renamed or
  unlinked
- [x] Source-floor issuer validation returns private nonserializable authenticated
  provenance from one non-clonable OFD-claimed protected origin, commits durable consumption
  only with dispatch publication, and permits exact-origin reacquisition after
  pre-publication owner death; copied digests, concurrent origin replay, repeated mint, and
  serialized revalidation cannot mint authority
- [x] Source-floor 32/26/21 and 91-case, mutation-edge 15/90 plus post-mutation,
  pre-start/root, unit-census, SC-002 output/census, and both forbidden-value expectations
  are independently pinned
- [x] Host-generation recovery is broker-coordinator-owned before first mutation, transfers
  durably from bootstrap broker to target broker, survives broker/daemon startup failures
  through existing units only, and never treats daemon identity or euid 0 as authorization
- [x] W4/T070/T071/T220 remain blocked on the external Network normative
  correction/version/migration and all four Network/Host opt-in cases
- [x] T219 performs only externally authorized historical adjudication, emits an actionable
  external-disposition refusal, and offers no successor or second-request path

## Feature Readiness

- [x] All functional requirements have clear acceptance criteria
- [x] User scenarios cover primary flows
- [x] Feature meets measurable outcomes defined in Success Criteria
- [x] Every implementation-specific detail is deliberate and traceable to a binding contract or fail-closed gate

## Validation Notes

**Iteration 1 (2026-07-29)** - 15 of 16 passing; two scope-class [NEEDS CLARIFICATION]
markers outstanding (terminal milestone, and the W0/W1 seal gap). Both surfaced to the user
rather than guessed, because either answer materially changes the size of the program.

**Iteration 2 (2026-07-29)** - all 16 items passing. Both clarifications answered and folded
in:

- **Terminal milestone**: full program, W2 through W8, six-condition release gate satisfied
  against the final candidate, d2b 3.0 tagged from `v3`. Landed as FR-037 and FR-038, a new
  SC-023, and an expanded delivery assumption noting that gating earlier would release a
  candidate a later wave still modifies.
- **W0/W1 seal gap (historical Iteration 2 disposition, superseded)**: Iteration 2 treated a
  feature-local written record as authority to begin sealed delivery at W2. The 2026-08-06
  analysis correction rejects that constitutional interpretation. FR-034 now makes the file
  historical evidence only, and FR-036 requires a separate accepted Principle VI
  constitution amendment before any implementation, resume, fix, close, merge, or advance.

Final shape: 38 functional requirements, 23 success criteria, 4 prioritized user stories,
9 key entities, 12 assumptions, explicit Out of Scope. No bracketed placeholder tokens
remain. Verified free of the non-ASCII dash codepoints the project constitution bans.

Ready for `/speckit-plan`.

**Iteration 3 (2026-07-29, post-clarify)** - all 16 items still passing after five
clarifications were integrated. No regressions.

Five decisions were recorded and applied:

- **Desktop companions block the release.** Previously out of scope entirely; now FR-039 and
  FR-040 require identifying the companion set, publishing replacement contracts early, and
  verifying each against the release candidate on a live host. Added SC-024, a fourth US4
  acceptance scenario, and rewrote the contradictory Out of Scope bullet so it scopes to
  companion source code rather than to companion compatibility.
- **Capability parity is enforced with exceptions.** SC-003 was absolute and would have
  contradicted the migration map's 15 DELETE rows. Now FR-041 enforces parity wherever a
  successor was promised, and FR-042 permits retirement only with an explicit listing,
  justification, and release-note entry.
- **Recovery-point attestation gates the irreversible cutover phase.** Added FR-043,
  SC-025, and a sixth US3 acceptance scenario. FR-043 now closes CHK019 with an exact
  full-host qualification, F7 candidate/commit/tree and daily-driver host binding, closed
  attestation fields, 86,400-second freshness and expiration, digest-bound evidence import,
  and fail-closed negative matrix. External snapshot/backup implementation remains outside
  the feature.
- **Live and hardware validation runs on the daily-driver host.** SC-022 now names the
  target, and a new assumption records this as deliberate risk acceptance that makes FR-043
  the primary safety net rather than a formality.
- **No intermediate releases; every wave lands by gated pull request.** Added FR-044 and
  FR-045 and SC-026. The former assumption about work landing through pull requests was
  promoted from an assumption to a requirement, since it is now enforced rather than
  presumed.

Three edge cases were added to cover the failure modes these requirements introduce: a
companion with no compatible version at release time, an operator who cannot attest to a
recovery point, and a capability discovered to have no successor only after its superseded
path is removed.

Shape at the end of Iteration 3: 45 functional requirements, 26 success criteria, 4
prioritized user stories, 15 edge cases, 9 key entities, 12 assumptions, and 5 recorded
clarifications. Those counts are an historical checkpoint and are superseded by Iteration 4.
At that checkpoint the artifacts were verified free of duplicate requirement ids,
placeholder tokens, banned dash codepoints, and statements contradicted by the
clarifications.

Ready for `/speckit-plan`.

**Iteration 4 (2026-08-06, current artifact reconciliation)** - all 33 current checklist rows
pass under the completion-program scope reflected by the current specification. The original
16-item baseline remains fully passing; 17 reconciliation checks were added after that baseline.
The current shape is **75 functional requirements and 35 buildable success criteria**. The
earlier 45/26 shape records the end of Iteration 3; it is not the current census.

Lifecycle state: **specification reconciled - plan approval pending**. Passing these 33 current
specification-quality checks means the artifacts are ready to request the required analysis
and plan-review gates. It does not record plan approval, implementation completion, or
permission to bypass the later exact-C/Q gate before T589.

This is a technical completion and delivery contract, not a technology-agnostic greenfield
product brief. Exact APIs, paths, commands, protocol fields, timing bounds, candidate
bindings, and validation procedures are present where removing them would make an
architecture, security, or fail-closed delivery obligation ambiguous or untestable. The
content-quality checks above therefore reject accidental or gratuitous implementation detail,
while accepting the deliberate technical contract detail required by the current 75 FRs and
35 SCs. This reconciliation changes planning prose only, preserves all 605 task IDs, and
records no implementation completion.

## Notes

- Items marked incomplete require spec updates before `/speckit-clarify` or `/speckit-plan`
