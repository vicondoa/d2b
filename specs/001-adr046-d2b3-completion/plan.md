# Implementation Plan: Complete the ADR-046 Provider Control Plane (d2b 3.0)

**Branch**: `001-adr046-d2b3-completion` | **Date**: 2026-07-29 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/001-adr046-d2b3-completion/spec.md`

## Summary

Turn the ADR-046 foundation delivered in waves W0 and W1 - currently 52k lines of
deliberately test-only, production-unwired code that no shipped binary depends on - into a
live Zone-scoped resource control plane, replace the pre-ADR-046 control plane through a
one-time destructive cutover, and release the result as d2b 3.0.

The design work is already done and Accepted: 55 normative specs, a 600-node implementation
graph, and 545 work items with exact destination paths and validation obligations. This plan
therefore does **not** re-derive architecture. It preserves the 531 work items that were
`Planned` at program opening as the primary W2-W7 task scope. At committed HEAD
`868469bf9c293cd48fff483717f14cb88c246821`, the authoritative manifest records 68 `Merged`
and 477 `Planned`; 54 of the initial 531 have moved to `Merged`, while the 14 W0/W1 items were
already `Merged` before this plan. The plan resolves the four unknowns that block delivery
(next-wave scope, the failed footprint gate, the companion release blocker, and the
parity/retirement split) and defines how each wave passes its gate.

The approach is delivery-shaped, not design-shaped: launch each wave's file-disjoint parallel
groups together only after that wave's unanimous plan review, converge every repository change
before exact-candidate evidence, and iterate scoped findings through nonbinding
`/d2b-panel-round plan` phase reviews before selecting the final candidate. Then gate the
integrated work on imported validation evidence plus, only when the wave has not already
consumed its once-per-wave request, one binding selected-roster `/d2b-panel-round work` delivery
request bound to that immutable snapshot. Entry-plan review, iterative phase-plan review, and
the binding delivery panel are separate surfaces and none substitutes for another. A retained
delivery request is never reclassified as a phase-plan round. Merge an attested tree through a
pull request without a post-attestation content change, and cut exactly one release at the end.

Wave 5 now includes an approved production-completion graph in addition to its 146 manifest
items. The graph wires the store, policy, authenticated ComponentSession route, controller
endpoint, watch fan-in, durable effect/adoption ledger, and mutation-audit drainer into one
daemon-owned Zone runtime. T220 then converges every slice and integrator-owned generated
artifact, iterates the nonbinding phase panel, and freezes final candidate F only on unanimous
convergence; T600-T602 regather and check production-boundary evidence against F. Wave 5
already has a retained binding delivery request, so T219 is non-authorizing until an accepted
external delivery-contract/tooling disposition reconciles that consumed request with the
amended candidate without deleting or reclassifying the historical record. T219 does not
independently authorize another request, seal, or merge. The earlier
backend, watch, and RSS results remain historical inputs; none substitutes for this final
wiring or its exact-candidate evidence.

The C1 contract defect is approved and fully assigned under Constitution 2.2.0. The accepted
provider-system-core member specification is authority for the stable internal handler names,
while the committed unreleased v3 `ZoneHandlerName` enum omitted their status values. T605
adds `ZoneHandlerName::SystemCoreHost` and `ZoneHandlerName::SystemCoreUser`, serialized as
`system-core-host` and `system-core-user`, and owns its paired snapshots, focused tests,
drift proof, and reference status surface. T595 consumes the variants in the production
emitter and T599 reconciles the remaining consumers; all paired artifacts and evidence land
in the same Wave 5 PR. T603 is now an editor-only accounting gate. At one clean base, cross-artifact analysis and a current selected-roster plan lifecycle bind the complete feature snapshot. If every T073-T218 obligation is satisfied, one `/d2b-spec-edit` batch checks exactly those rows plus T603 and the integrator records only that checkbox transition in a dedicated commit. The editor receipt and Git commit are the sole authority; no Rust resume validator, changelog fragment, scratch receipt, digest chain, or sidecar exists. Because the edit changes feature content, T589 requires fresh analysis and a fresh selected-roster lifecycle bound to the resulting clean commit and snapshot.

**Current installed-host bootstrap state: BLOCKED.** Committed protocol 4 has no
host-generation handoff operation, while committed `nixos-modules/host-broker.nix` makes the
source generation's `d2b-priv-broker.service` execute that generation's `brokerPackage`.
Consequently the target closure's proposed compatibility mode is not an executable,
supervised actor before profile publication. The constraints correctly forbid solving that
cycle with a new unit, runtime override, child process, mutating entrypoint, or daemon recovery
owner. This plan therefore escalates the missing source-generation compatibility floor rather
than claiming T592/T595 can implement it inside the one Wave 5 target transition. That
external owner must atomically install the exact nonempty 13-member
`SourceGenerationCompatibilityFloorV1` census defined in `data-model.md`: both protocol-4
source peers, source wire and privilege schemas, operation catalogue,
`source-handoff-v1` catalogue fingerprint, compatibility disposition, capability/API
fingerprint, serialization snapshot, positive fixture, bare-protocol negative fixture,
cross-fingerprint negative fixture, and exact immutable broker-managed apply object used
under `sudo`. Every member binds one disposition and source generation; missing, duplicate,
extra, empty, stale-generation, stale-digest, and cross-disposition members refuse. The
external disposition must name the concrete source producer/installer owner and the concrete
typed import/validation authority. Acceptance is the exact append-only
manifest-produced -> atomically-installed -> installed-census-validated ->
imported-for-exact-C/Q chain in `data-model.md`; T589 dispatch requires its final immutable
import receipt bound to clean C/Q and the source generation used for migration. No
feature-local task owns a producer, installer, validator, importer, or repair step. The
separately accepted external `ADR-046-validation-and-delivery` Version 2 amendment owns the
canonical source-floor object encoding, closed digest/domain/framing registry,
`SourceGenerationIdentityV1`, strict repository schemas, exact 15-digest/four-signature
vectors, and disposition-pinned issuer verification plus copied-digest rejection.
The compatibility producer/installer and import/validation authority implement and install
that accepted contract but do not own or redefine those repository artifacts. Both external
prerequisites must agree byte-for-byte before T589 dispatch.
The caller-flake target entrypoint
remains unprivileged. Numeric protocol 4 without that exact negotiated fingerprint is the
bare committed protocol and refuses. T589 and all downstream Wave 5 implementation remain
blocked until an accepted external disposition lands, is installed in the source 3/1
generation, and proves the exact actor contract stated in FR-070. A target-only binary,
synthetic starting image, or prose compatibility claim is not that disposition.

## Technical Context

**Language/Version**: Rust from `packages/rust-toolchain.toml` (currently 1.97.0, with
`rustfmt` and `clippy`); Nix for the NixOS module surface. The pin, not this plan, is
authoritative for the compiler version used by enforcing gates.

**Primary Dependencies**: redb `=4.1.0` is a direct dependency of the production
`packages/d2b-resource-store-redb` backend. The disposable
`proofs/redb-resource-store-spike` workspace separately retains its provisional `=4.1.0`
pin and remains quarantined under D128; that quarantine does not apply to the production
crate. Other primary dependencies are ttrpc/protobuf for the resource service, Noise
handshakes for ComponentSession, and Cloud Hypervisor and crosvm as runtime backends. No
new toolchain, linter, formatter, or nixpkgs overlay is introduced.

**Storage**: One embedded redb database per Zone, opened by owned fd, with full crash-safe
durability - one fsync per write transaction, no reduced-durability mode. Write queue 256,
group-commit batch 16, read pool 4, concurrent reads 16, read lifetime 250 ms.

**Testing**: Existing closed layer set - nix-unit eval cases, Rust unit and binary integration
tests, rendered-artifact contract tests, policy lints, and flake checks at Layer 1; podman
containers and `runNixOSTest` at Layer 2; hardware, live-host, and cloud tiers manual. No new
top-level shell gate. Every heavy lane runs through the two-slot `xtask heavy-gate` semaphore.

**Target Platform**: `x86_64-linux` NixOS host with KVM, single trusted user. Graphics paths
are x86_64-only by existing platform gate.

**Project Type**: NixOS module framework plus a multi-crate Rust control plane (58 workspace
members at committed HEAD c758a377703c523edd88a987e48a6f30034e1912, plus two deliberately
excluded standalone workspaces)

**Performance Goals**: Empty-store readiness <=500 ms; p95 local Get and bounded List <=2 ms;
p95 crash-safe single-resource mutation <=10 ms; p95 durable commit to controller handler
start <=5 ms; p95 ready Process commit to launch-attempt start <=20 ms

SC-002 performance and evidence semantics are owned solely by accepted Version 2 `ADR-046-validation-and-delivery` and generated `VD2-SC002-RECEIPT` plus `VD2-SC002-TRACEABILITY` rows. T604 measures the assigned operator outcome, T600 imports it, and T220 verifies the generated ownership bijection. This plan does not copy the clock, receipt, digest, census, publication, or recovery protocol. Until the Version 2 rows exist and Gate 0 passes, SC-002 is a blocking unimplemented prerequisite.

**Constraints**: Whole-process RSS <=24,576 KiB with **no baseline subtraction** - historical
production fixtures passed at their recorded tips, but the completed production publication
path remains unmeasured until T601 measures final candidate F frozen by T220; aggregate idle
RSS <=64 MiB; per-component budgets 22 MiB for `Provider/system-core` and 12 MiB for
`Provider/system-minijail`; per-Provider-crate hermetic suite aggregate process-CPU p95 <=3 s

**Scale/Scope**: initial program scope 531 work items across 53 specs and 7 waves; current
manifest state 477 `Planned` work items across 43 specs and 68 `Merged` total; 27 Provider
crates; 19 standard ResourceTypes; hard fixtures at 10,000 resources and 100 concurrent
watches

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-checked after Phase 1 design.*

| Principle | Assessment | Status |
| --- | --- | --- |
| **I. Daemon-Only Control Plane** | ADR-046 adds per-Zone runtimes as **parent-spawned processes**, not PID1 units, and DELETEs the three per-realm units. Unit count does not grow; the `systemctl list-units` exit criterion is unchanged. Restart remains a continuation event via FR-003. | PASS (see research R5) |
| **II. Broker-Mediated Audited Privilege** | FR-012 keeps every privileged host mutation on the audited broker path; D077 forbids any Provider process importing the broker, enforced by a policy lint. FR-070 adds a daemon-owned resource-mutation audit drainer, not a new service, and requires audit durability before success. `SO_PEERCRED` plus group membership at the public socket stays the sole initial local lifecycle authz surface and is never treated as a Resource API subject. Host-generation continuation consumes a sealed durable capability minted only after that classification; daemon identity, broker-socket credentials, and euid 0 never independently authorize. | PASS |
| **III. Reasonable Isolation Over Convenience** | FR-009 default-denies cross-Zone reference; FR-014 fails closed on missing identity state rather than reinitializing; FR-066 requires authoritative registrar-derived subjects; FR-069 forbids partial publication; FR-071 isolates a failed Zone without making it ready. virtiofsd zero-capability and per-VM store-farm invariants are untouched. | PASS |
| **IV. Contract-Driven Compatibility** | 3.0 is a deliberate major-version clean break with v3 schemas, versioned artifacts, and fail-closed drift gates (FR-031). Constitution 2.2.0 authorizes this coordinated correction of an approved contract defect before the first d2b 3.0/v3 release. T605 owns the two omitted `ZoneHandlerName` values plus its normative, test, API-snapshot, guard, and reference artifacts; T595 owns emission, T599 owns downstream consumers, and T220 owns generated-manifest reconciliation and the full drift gate. Those stages land together in one Wave 5 PR. The Zone desired-state schema is unchanged. | PASS - C1 resolved in artifacts, implementation pending |
| **V. Test-Layer Discipline** | FR-032 pins coverage to the lowest hermetic layer and forbids a new top-level shell gate; FR-029 routes every heavy lane through the single semaphore; FR-033 retires superseded suites. | PASS |
| **VI. Panel-Gated Multi-Phase Work** | W0/W1 lack their required panel receipts and seals, and W2-W5 have implementation whose contemporaneous plan panels are unproven. The feature-local historical record and late-remediation receipts preserve evidence but cannot waive or amend Principle VI. Before any implementation, resume, fix, work-panel, seal, merge, or advance boundary, FR-036 requires a separate accepted Principle VI constitution amendment on the integration lineage that expressly dispositions both gaps and is an ancestor of the exact execution base. Only after that external prerequisite may the ordinary T008/T030/T037/T072 dispositions and T221/T481/T558 prospective plan gates operate. | **BLOCKED - external constitution amendment required** |
| **VII. Traceable, Marker-Free Shipped Artifacts** | Wave tags stay in commits and planning artifacts; SC-018 requires the release notes carry zero process markers; FR-019 lands docs with their behavior. ASCII-hyphen rule observed throughout. | PASS |

**Gate result**: **BLOCKED for every implementation, resume, fix, work-panel, seal, merge,
and advance boundary**. Read-only analysis may describe the tree but cannot produce an
authorizing receipt while FR-036 is open. W0/W1's record and W2-W5's remedial paths are
historical or current evidence only. A separate accepted Principle VI constitution amendment
must expressly disposition both gaps and be an ancestor of the exact execution base. After
that external prerequisite lands, all affected analysis and plan-panel evidence must be
gathered or rerun against its descendant base before T603 or any other source-writing task
may proceed. C1 remains a Constitution-2.2.0 coordinated contract correction, not a Principle
VI waiver.

**Execution model**: this plan is executed by a coding agent dispatching subagents. Wide
parallel fan-out is a positive obligation, not an optimization - the delivery contract fails
wave entry when a ready, file-disjoint slice is left unlaunched. One write-capable subagent per
parallel group, each in its own worktree; the lifecycle-selected read-only panel lanes on
`gpt-5.6-sol` at `xhigh`. Heavy validation is capped at 2 concurrent lanes by the OFD-locked
semaphore regardless of how many implementation subagents are running. See tasks.md
"Parallel subagent execution model".

**Post-amendment re-check (2026-08-06)**: **FAIL on Principle VI until the external FR-036
constitution amendment lands; other principles remain unchanged**. The
completion graph introduces no new unit, privileged path, top-level test gate, toolchain,
overlay, compatibility shim, or store-owned policy interpreter. The audit drainer and
controller ledger are Zone-runtime owners inside `d2bd`, and production publication consumes
the existing ComponentSession/ZoneBus boundary. T605 corrects the omitted closed status-enum
values through the approved coordinated path. `spec-coverage.md` binds the local tasks to the
unchanged generated-manifest census.

### C1 correction and version impact

The accepted `docs/specs/providers/ADR-046-provider-system-core.md` member spec currently uses
`system_core_host` and `system_core_user` both for internal telemetry labels and for the
serialized `Zone.status.handlers` names, while the committed
`packages/d2b-contracts/src/v3/zone.rs` closed enum uses kebab-case wire serialization. T605
resolves that defect in favor of the committed serialization rule: the only serialized Zone
handler names are `system-core-host` and `system-core-user`. The underscore spellings remain
internal closed telemetry-label values only and MUST NOT appear in serialized
`Zone.status.handlers[]`. T605 adds `ZoneHandlerName::SystemCoreHost` and
`ZoneHandlerName::SystemCoreUser`; readiness consumes exactly one status record for each.
`ProviderLifecycle` remains a separate aggregate enum value and cannot satisfy either record.

T605 bumps the `Version` metadata of both governing normative specifications and corrects
their handler-name language in the same commit as the Rust enum, unit/serialization and
closed-list tests, compiler-derived public and private API snapshots regenerated only by
`make api-surface-pin`, the existing lowest-layer contract/policy guard, and
`docs/reference/resource-plane-runtime.md`. No `apiVersion`, JSON `schemaVersion`,
`manifestVersion`, or `bundleVersion` bump is made because no desired-state field or
ResourceType schema changes. The generated
`docs/reference/schemas/v3/core.d2bus.org_Zone.schema.json` remains byte-identical after its
existing generator and drift gate run. T595 consumes the new variants only after T605 merges.

## Project Structure

### Documentation (this feature)

```text
specs/001-adr046-d2b3-completion/
├── plan.md              # This file
├── research.md          # Phase 0 output - resolves R1-R7, records RK-1..RK-6
├── spec-coverage.md     # Phase 1 output - COMPLETENESS PROOF: all 55 specs and all 545
│                        #   work items enumerated; cross-cutting obligations; binding rule
├── data-model.md        # Phase 1 output - Zone/Resource model and the 19 ResourceTypes
├── quickstart.md        # Phase 1 output - wave lifecycle and operator validation runbook
├── deferred-findings.md # Deferred LOW/MEDIUM panel findings (constitution 2.1.0)
├── friction-log.md      # Delivery friction, categorized for terminal-wave triage
├── contracts/           # Phase 1 output - the contract surfaces this program must deliver
│   ├── README.md
│   ├── resource-api.md
│   ├── operator-cli.md
│   ├── nix-configuration.md
│   ├── generated-artifacts.md
│   └── companion-contracts.md
|-- checklists/
|   |-- requirements.md  # Spec quality checklist (33/33 passing)
|   \-- coverage.md      # Upstream coverage gate; CHK054 closes only the C1
|                        #   specification-quality ambiguity, not implementation
\-- tasks.md             # Phase 2 output - NOT created by /speckit-plan
```

## Specification coverage and the no-detail-loss rule

The ADR-046 set is the design. This plan sequences and gates it; it does **not** restate it,
and it must not lose it.

**The manifests are authoritative.** Every work item is a 15-field object. `tasks.md`
deliberately does not copy those fields: each manifest-backed row carries a stable
`workItemId` pointer plus non-authoritative orientation labels. At dispatch, retrieve the
complete object **verbatim** from `docs/specs/ADR-046-work-items.json` and carry all 15 fields
unchanged in the work prompt. The implementing change is checked against those canonical
bytes; a task-row label, paraphrase, condensation, or selective quotation cannot satisfy an
obligation that the wave seal will later assert.

**Why this plan does not inline the spec text.** Copying 545 items of design and validation
prose into planning artifacts would create a second source of truth that no drift gate checks,
which is a worse failure than referencing the authoritative bytes. Instead:

- [spec-coverage.md](./spec-coverage.md) enumerates **all 55 specs and all 545 work items**,
  accounting for each exactly once, generated from the committed manifests so it cannot drift.
- Full text for any item is one command away:
  `jq --arg id <ID> '.items[] | select(.workItemId==$id)' docs/specs/ADR-046-work-items.json`
- `spec-coverage.md` also captures the set-wide obligations that belong to no single item and
  are therefore easiest to lose: the standing Gate 0 conditions, the 129 frozen decisions, the
  exclusive ResourceType ownership map, the ten hard numeric targets, the contended-file
  prep rules, and the three-part deletion obligation.
- It closes with a detail-preservation checklist to run against `tasks.md` before
  implementation starts.

**Completeness reconciles**: the initial 545-item census was 14 `Merged` and 531 `Planned`.
At the receipt HEAD it is 68 `Merged` and 477 `Planned`, splitting
8/6/19/4/31/146/258/73 across W0-W7 with W8 recorded at W7 close. The graph moved
`ADR046-process-002` from W4 to W6 and leaves it `Planned`; T039 follows that state and wave
without renumbering. Every item carries a non-empty `removalProof`, so FR-023's per-path
proof obligation is already itemized rather than needing to be invented.

A `tasks.md` that does not cover every `Planned` id in `spec-coverage.md` is incomplete by
definition.

### Source Code (repository root)

The program writes into the existing tree. Paths below are the real destinations named by the
implementation graph, grouped by wave ownership.

```text
packages/
├── d2b-contracts/src/v3/          # W2 adds host, guest, execution_policy, process, volume,
│                                  #   user, network, device, credential, zone_routing,
│                                  #   zone_session (W0/W1 modules already present)
├── d2b-resource-store/            # engine-neutral contract (W0, present)
├── d2b-resource-store-redb/       # W5 adds actor, transaction, revision_log, backup,
│                                  #   migration - the corrected production engine
├── d2b-resource-api/              # W5 adds watch.rs; registration path wired in W2-W5
├── d2b-controller-toolkit/        # W1 present; W5 adds the real-backend reaction benchmark
├── d2b-core-controller/           # W2 adds zone_links.rs, configuration.rs
├── d2b-session/  d2b-session-unix/ d2b-bus/
│                                  # W1 present; W2 adds bus session/, transport/,
│                                  #   zone_route.rs, relay.rs
├── d2b-zone-routing/              # NEW in W2 - engine, resolver, service, vectors, benches
├── d2b-resource-client/           # NEW in W2
├── d2b-provider/  d2b-provider-toolkit/
│                                  # W2 adapts in place; W3 owns the Provider contract
├── d2b-process/  d2b-provider-supervisor/
│                                  # W4 (NOT W2 - see drift note below)
├── d2b-provider-system-{core,systemd,minijail}/
├── d2b-provider-volume-{local,virtiofs}/
├── d2b-provider-network-local/  d2b-provider-credential-*/  d2b-provider-device-*/
│                                  # schema halves in W2/W4/W5, implementations in W6
├── d2b-telemetry/  d2b-audit/     # W5
├── d2b/                           # operator CLI - W5
└── xtask/                         # gen-zone-schemas, gen-zone-nix-options; delivery tooling

nixos-modules/
├── options-zones.nix              # present; W2 restructures as the generated base
├── generated/                     # NEW - resource-types.nix, options-zones-<Type>.nix
├── zone-resources-json.nix        # NEW in W2
├── resources-*.nix                # per-ResourceType emitters, W2/W5
├── assertions.nix                 # W2 adds Zone assertions (single writer in W2)
└── bundle-artifacts.nix           # W2 adds the per-Zone resource-bundle.json row

docs/
├── reference/schemas/v3/          # NEW - per-ResourceType JSON schemas
├── reference/                     # per-behavior docs land with their wave (FR-019)
└── specs/ADR-046-*                # normative set - amended only via its own path

proofs/redb-resource-store-spike/  # disposable; hosts the RSS correction prototype (RK-1)
tests/                             # extends existing closed layer set; no new top-level gate
```

**Structure Decision**: No new top-level structure is introduced. The program extends the
existing `packages/` workspace, `nixos-modules/`, `docs/reference/`, and `tests/` trees at the
exact destinations the implementation graph names. New crates follow the established
`d2b-provider-<base>-<implementation>` layout so the existing provider-crate-layout policy lint
applies without modification.

### Wave sequencing

| Wave | Specs | Items | Parallel groups | Gate note |
| --- | --- | --- | --- | --- |
| W2 | 2 | 19 | 2, file-disjoint, zero overlap edges | Delivery state reports sealed/merged; T008, T028, and T029 are historical verification/adjudication only. Exact external delivery-record confirmation or an accepted external correction is required; no new binding panel, seal, or merge is scheduled. |
| W3 | 1 | 4 | 1, strictly serial | Delivery state reports sealed/merged; T030, T035, and T036 are historical verification/adjudication only under the same external-confirmation-or-correction rule. |
| W4 | 5 | 31 | 6 parallel | Delivery state reports sealed/merged; T037, T070, and T071 are historical verification/adjudication only under the same external-confirmation-or-correction rule. |
| W5 (`adr046w5` delivery address) | 7 plus pulled-forward `provider-network-local` | 146 + 17 local completion tasks + T336-T355 pulled forward | 12 manifest groups + the serialized completion graph + one Network production group | FR-036 and T072 gate T603's exclusive-editor reconciliation. T336-T355 then land the accepted double-opt-in production path before T604. T220 uses the selected-roster lifecycle before final F. The retained binding request remains externally dispositioned only. |
| W6 | 27 manifest dossiers, 26 remaining | 258 manifest items, 238 remaining after T336-T355 | 5 families, with `provider-network-local` already merged | FR-036 first; T221 selected-roster plan lifecycle before remaining implementation; T479 requires exact-F6 `Provider/runtime-cloud-hypervisor` Guest acceptance; T480 revalidates it before close |
| W7 | 5 | 73 | 5 parallel | FR-036 external constitution amendment first; T481 plan panel before implementation; T555 work panel after convergence |
| W8 | 0 | determined by T557 at W7 close | friction closure | FR-036 external constitution amendment first; T558 plan panel after triage and before implementation; T565 work panel after convergence |

#### W2-W6 host-continuity close gate

FR-075 is a requirement, not an assumption. The converged exact candidate for each of W2
through W6 must have enumerated and successfully built
`vmChecks.x86_64-linux.daemon-restart-vm-survival` through the existing heavy-gated
`make test-host-integration` target with no skip. The case must prove public `d2b vm`
start/status/stop, an explicit `Ready` observation before restart, guest reachability,
`d2bd.service` restart, same runner PID/start-time adoption through a newly acquired pidfd,
continued reachability, and an explicit `Stopped` observation after stop. It must query
`systemctl list-units --all` over the complete loaded `d2b*`/`microvm*` namespace, exclude
exactly the canonical `d2b.slice`, sort every remaining unit name, and require exact set
equality with the three lifecycle units below. A nonzero listing result is fatal before filtering; no
downstream pipeline stage may convert failed enumeration into an empty or successful census.
The required set is `d2bd.service`,
`d2b-priv-broker.socket`, and `d2b-priv-broker.service`. A query containing only those
expected names is ineligible because it cannot observe an unexpected lifecycle unit. No
other slice, target, service, socket, timer, path, or template is filtered. Separate negative
cases use the code-canon raw set: committed `d2b.slice` plus the required three
service/socket units. The stale AGENTS.md exit-criterion count of three omits that slice.
This plan therefore compares exactly four raw names only after full-namespace enumeration,
excludes exactly `d2b.slice`, and compares exactly three remaining names; it does not edit the
external contributor guidance in this feature batch.
Separate negative
cases inject loaded `d2b-unexpected.slice` and `d2b-unexpected.service`; each remains after
the sole exclusion and must fail equality. The same case injects PID reuse,
pidfd/start-identity mismatch, and multiple-plausible-runner ambiguity and proves each is
quarantined with no adoption, cleanup, or signal against an unproven process. Passing
evidence must record nonempty discovery, the exact enumerated and successfully built attr,
command success, and zero `SKIP` result; status-only output is not execution evidence.
For W2-W4, T028/T035/T070 only verify that evidence in the retained historical candidate;
they do not adapt the test or rerun the command. A missing historical result requires accepted
external correction. The evidence map is closed:

| Wave candidate | Evidence owner/verifier | Close owner/adjudicator | Candidate-bound evidence |
| --- | --- | --- | --- |
| historical F2 | T028 | T029 | exactly one `local-host` `EvidenceRecord.validation = "pre-adr046-host-continuity"` result |
| historical F3 | T035 | T036 | exactly one `local-host` `EvidenceRecord.validation = "pre-adr046-host-continuity"` result |
| historical F4 | T070 | T071 | exactly one `local-host` `EvidenceRecord.validation = "pre-adr046-host-continuity"` result |
| F | T220/T604 | T219 | the result appears only in T600's existing `operator-nix-activation-cleanup` record and is revalidated by T602 |
| F6 | T479 | T480 | the result appears only in the existing `w6-cloud-hypervisor-guest-acceptance` record |

Missing, duplicate, wrong-candidate, empty, skipped, status-only, private-hook, missing
Ready/Stopped state, non-fresh-pidfd adoption, incomplete unit enumeration, or stale
continuity evidence blocks historical close confirmation for W2-W4 and any externally
authorized W5 close action; it blocks W6's prospective work-panel request, seal, merge
eligibility, and merge. It never licenses a new W2-W5 binding request. No new W5 evidence
identifier is introduced. W7's explicit cutover is the only point that ends this gate.

#### W6 Guest acceptance ownership

| Scope | Owner | Exact files or evidence surface | Validation lane and done condition |
| --- | --- | --- | --- |
| `Provider/runtime-cloud-hypervisor` implementation family | T384-T390 (`ADR046-ch-001` through `ADR046-ch-007`) | `packages/d2b-provider-runtime-cloud-hypervisor/src/{controller,bootstrap_graph,vmm_argv,health,adoption,metrics,audit,state}.rs`, `packages/d2b-provider-runtime-cloud-hypervisor/{nix,tests}/`, only T387's generated Guest option extension under `nixos-modules/`, plus T384's `tests/host-integration/runtime-cloud-hypervisor-guest-acceptance.nix` and only its discovery/build recipe in `Makefile` | Each manifest row's own validation; T384 is the acceptance owner because its authoritative validation requires end-to-end VMM boot with real KVM and a guest-control session through `make test-host-integration` |
| Exact-F6 Guest acceptance | T479 | No repository write; exactly one delivery `EvidenceRecord.validation = "w6-cloud-hypervisor-guest-acceptance"` bound to F6 | Heavy-gated `make test-host-integration`; nonempty enumeration and successful no-skip builds of exact attrs `vmChecks.x86_64-linux.runtime-cloud-hypervisor-guest-acceptance` and `vmChecks.x86_64-linux.daemon-restart-vm-survival`; declared Guest reaches the real Provider-owned Cloud Hypervisor process effect, authenticated guest-control session, and ready state; the same record carries FR-075 public lifecycle continuity |
| Close revalidation | T480 | Same immutable T479 evidence record; no alternate record or Provider family | Reinvoke the same closed predicate at every close boundary through merge |

ACA, Azure VM, and qemu-media cannot satisfy this acceptance, and no four-family matrix is
required. This keeps the full-US1 proof bounded to one Provider family, one numbered evidence
owner, one candidate, and one heavy validation lane.

### Per-wave plan-panel and work-panel boundaries

FR-036 is a program-wide predecessor of every row below. The external constitution amendment
must be accepted, committed on the integration lineage, expressly disposition the W0/W1 and
W2-W5 Principle VI gaps, and be an ancestor of the exact execution base. No feature-local
historical record, remedial plan receipt, candidate record, or task checkbox can satisfy it.

The two panel surfaces are deliberately different. `/d2b-panel-round plan` is the
nonbinding phase surface: it reviews the exact feature snapshot and clean implementation base
before dispatch, and it is also the iterative surface for integrated-candidate finding-fix
rounds before a final candidate is selected. A valid phase receipt contains exactly one record
for every role recorded by the lifecycle selection artifact, all bound to the reviewed base, feature snapshot, and stated
integrated tree, with `signoff: true` and no recommendations. A content, plan-artifact, or
implementation-base change invalidates the applicable receipt and requires a
delta/full-context rerun. `/d2b-panel-round work` is the binding delivery surface and is
requested at most once per wave. For a prospective wave with an unconsumed request, only the
final converged immutable candidate receives it. A finding on that binding request leaves the
wave unsealed; it never opens a second binding request for the same or a successor candidate.
The wave entry task refuses dispatch without its entry receipt, and the wave close task
refuses advance unless the reviewed base is an ancestor of every implementation head and the
final phase receipt still matches the selected tree and feature snapshot.

Pipelining changes only whether the predecessor must already be merged. It never permits a
successor to implement before the successor's own plan panel. The work panel and its binding
candidate request cannot be cited as plan-review evidence.

At committed HEAD `d89636d212d2989c19b6a1cf3fc86308c9daa28f`, implementation already exists
downstream of W2-W4, but these feature artifacts cite no contemporaneous plan-panel receipt for
their first dispatch. Their historical plan-review status is therefore **unproven**, not
passed. T008, T030, and T037 remain unchecked unless exact retained receipts prove the
original gate. External delivery state reports all three waves sealed and merged, so no
current remedial plan panel, convergence fix, candidate freeze, binding panel, seal, or merge
is scheduled. T028/T029, T035/T036, and T070/T071 require exact external delivery-record
confirmation or an accepted external correction and perform historical
verification/adjudication only. Current evidence cannot rewrite the missed entry boundary or
manufacture a new close.

Wave 5 also has existing implementation whose original entry predicates are owned by T072.
At committed HEAD `e6bece5d9debebef467e0c553a4d911701f6223e`, these feature artifacts cite no
exact contemporaneous Wave 5 plan-panel receipt bound to the actual first-dispatch base and
feature snapshot. T072 is therefore unproven and may be checked only if that retained receipt,
plus every other historical predicate, is produced. Otherwise it stays unchecked and one
`historical-entry-remediation-t072` record requalifies the current clean A/P0 base without
claiming any T073-T218 obligation complete or curing the missed historical plan gate. FR-036's
external amendment must expressly disposition the Wave 5 gap together with W2-W4. Exactly one
T072 historical/current remedial disposition must validate before T603, but neither
disposition permits T603 until that external amendment is an ancestor of A. T603's current
selected-roster plan lifecycle at A is not historical evidence and creates no delivery
request or reservation. The exclusive editor batch and dedicated checkbox-only commit C
change the feature snapshot, so fresh analysis and a new selected-roster lifecycle at C are
required for implementation dispatch and form the final pre-T589 plan gate.
T602 and T219 revalidate the T072 disposition at close, but T219 remains non-authorizing
until the external owner has landed the delivery-contract/tooling change and validator and
that validator imports one `Wave5RetainedRequestDispositionV1` for the retained Wave 5
binding request and exact F. W6-W8 are prospective and their entry
tasks T221, T481, and T558 refuse their first implementation dispatch until their own plan
panels pass.

### Approved `adr046w5` production-completion graph

This section is the explicit amendment required by the 2026-08-06 operator decision. It does
not alter the 146 manifest work items or mark any of them complete. It assigns the missing
production composition that the feature artifacts previously implied but did not own.
`W5` remains the historical manifest wave label; every current delivery address, panel round,
checkpoint, and commit-tag instruction for this graph uses qualified lowercase `adr046w5`.

#### Production data flow and ownership

1. The broker resolves the opaque Zone store id and returns the owned database descriptor.
   The Zone runtime verifies immutable store and Zone identity, then reads mutable policy,
   active-configuration, and controller revisions from durable state. Reopen never supplies
   mutable revisions from constants.
2. `ZoneResourceRuntime` is the single lifecycle owner of the Zone policy. On initial install
   and every restart it owns one private, sealed, non-`Clone`, non-`Copy`, one-shot
   `PolicyBootstrapRead` minted only by one private issuer. It has no public constructor,
   conversion, `Default`, field, accessor, capability trait implementation, or reconstruction
   path. After immutable Zone/store identity is verified, that capability
   reads only the Zone's policy-input envelopes at the exact live durable nonzero revision.
   It carries no Resource API subject, has no general read or mutation method, and is consumed
   by the one installation attempt. `d2b-resource-api` compiles those envelopes into the
   first immutable `PolicySet` and installs it in `NativeAuthorizer`; redb never parses an
   RBAC DTO. Missing, stale, cross-Zone, or invalid input consumes the attempt and leaves the
   Zone unpublished and degraded. After installation, policy reads and revision advances
   use only the authenticated Resource API. A committed new revision is compiled completely
   before atomic replacement, and readiness advances only when the installed revision equals
   live durable metadata. This bootstrap-to-normal transition breaks the startup cycle
   without weakening authentication or D106.
3. The registrar consumes verified transport evidence into one ComponentSession, derives the
   authoritative subject from registrar-private state, and registers both ResourceService and
   the controller endpoint on the exact ZoneBus route. Unix admission obtains the peer process
   descriptor directly from the accepted socket with `SO_PEERPIDFD`; opening a pidfd later from
   the `SO_PEERCRED` numeric PID is forbidden because PID reuse can redirect that lookup. The
   kernel floor must provide `SO_PEERPIDFD`; unavailable support, a non-`CLOEXEC` descriptor,
   or any mismatch between `SO_PEERCRED` and the credential, process-generation, cgroup, or
   liveness evidence verified against that exact pidfd refuses admission. A daemon restart
   acquires a new peer pidfd from the newly accepted socket and never revives persisted
   numeric-PID evidence. The public daemon bridge may request registration but may not
   construct or pass a subject claim. `VerifiedUnixPeer` exposes no credentials or evidence
   accessor, `ZoneBootstrapIdentity` exposes no public issuer, constructor, verifier, clone, or
   identity accessor, and one registrar-private issuer consumes the complete pidfd evidence.
   The session adapter, descriptor, bus Unix transport, and session seam consume that same
   accepted-socket evidence and expose no caller-supplied verifier or credential constructor.
   Peer-pidfd acquisition uses T592's typed
   `OpenPeerPidfdFromAcceptedSocket` broker operation. The daemon transfers only the accepted
   Unix socket with `SCM_RIGHTS`; the request carries no raw descriptor number, credential
   tuple, or numeric PID, and the response returns only an `OwnedFd` pidfd with
   `FD_CLOEXEC` over `SCM_RIGHTS`. Both ancillary receive paths - broker receipt of the
   accepted socket and daemon receipt of the returned pidfd - call `recvmsg` with
   `MSG_CMSG_CLOEXEC`, reject `MSG_CTRUNC`, parse the complete control-message set, take
   ownership of every received fd immediately, require exactly one fd of the expected type,
   verify `FD_CLOEXEC`, and close every fd on count, type, index, decode, or later validation
   failure. An unexpected extra fd is never ignored. Descriptor-count and exec probes cover
   success, malformed payload, missing fd, extra fds, truncated control data, and errors
   after fd receipt. The sole raw `getsockopt(SO_PEERPIDFD)` call is consolidated
   in the already approved `packages/d2b-priv-broker/src/sys.rs` FFI quarantine. It uses a
   narrow item-level unsafe allowance and a `SAFETY:` justification on every unsafe block,
   passes and validates exact `optlen`, assumes ownership of every nonnegative returned fd
   before checking the syscall result or later invariants, and closes it on every short,
   oversized, malformed, syscall, missing-CLOEXEC, or later failure without assert, panic, or
   leak. The `nix` 0.31.3 `PeerPidfd` `MaybeUninit`/assert wrapper, a new repository-authored
   FFI crate, and any local `d2b-session-unix` syscall fallback are ineligible.
4. The admitted ResourceService watch opens through that registered route, replays from the
   durable checkpoint without a replay/live gap, and feeds the registered controller fan-in.
   Before any EffectPort call, the core controller records an outstanding effect in the
   existing per-Zone durable store through the engine-neutral store contract. The core
   controller alone interprets ledger bytes. The key binds Zone, controller generation,
   resource UID, committed revision, operation id, and effect ordinal. Restart adopts or
   idempotently replays pending entries before cleanup. Cleanup completion is a compare against
   the same UID and exact nonzero revision.
5. The Zone runtime owns the mutation-audit drainer. The same redb transaction that commits
   each privileged mutation also creates its immutable authoritative journal rows, one per
   mutation ordinal; export completion is separate mutable state and can never delete or
   rewrite an unexported row. Audit constructors accept typed fixed 32-byte,
   domain-separated digests for operation, correlation, subject, Zone, resource, replay
   binding, and any retained trace correlation; no constructor accepts their raw string
   equivalents. Raw values stay only in bounded private operation/replay state with redacted
   `Debug`, and raw trace context is excluded from authoritative rows and exports. The
   unprivileged Zone runtime owns the drain state machine, but every root-owned filesystem
   effect crosses one typed broker op carrying only fixed-digest records and bounded rotation
   policy. The root broker is the sole `SegmentWriter` owner and holds the root-owned segment
   directory fd; segment append, rotation, export, and prune use fd-relative
   `openat2`/`openat`/`unlinkat`, never joined paths. No service or unit is added. Export
   completion may advance only after the broker response proves the segment file and its
   directory have both been `fsync`ed and the opened segment inode has been revalidated. A
   normal successful mutation response is
   released only after the required append-only segment export and its completion state are
   durable. If export remains incomplete after commit, the API returns
   semantic `CommittedPendingAudit` through the layered `ResourceStatus` composite:
   `ResourceStatus.phase` is
   `ResourcePhase::Degraded`; `ResourceStatus.outcome.code` is
   `StatusCode("committed-pending-audit")` with retryable, safe remediation and no raw sink
   detail; `ResourceStatus.update.state` is `UpdateState::Blocked`; and
   `ResourceStatus.update.operation_id` is `Some(original_operation_id)`. Existing bounded,
   redacted condition, outcome, and update fields carry only safe same-ID retry/status
   instructions. The additive protobuf `PendingAuditStatus` field makes that composite
   representable on every mutation response, including `DeleteResponse`; it changes the
   ResourceService schema fingerprint but no Resource JSON `apiVersion` or `schemaVersion`.
   The result neither reports ordinary success nor implies rollback. The Zone is unpublished
   and degraded until export recovery. Same-ID observation or resumption first matches a
   persisted replay-binding digest over the registrar-derived subject, Zone, semantic request,
   target, verb, expected revision, and idempotency data. A mismatch is denied and audited.
   An exact retry returns the pending or one stored final result without reapplication; a
   different ID follows normal revision/conflict semantics. A typed `InspectOperation`
   ResourceService request/response carries this lookup through the store, generated protobuf
   and ttrpc bindings, method catalogue, authorization/router, daemon client, and CLI; no
   in-memory-only or CLI-local status path is eligible. Restart deduplicates by fixed operation
   digest plus mutation ordinal and produces one logical exported record. The one
   configuration carrier is compiler-only `d2b.zones.<zone>.audit`, emitted as the
   required top-level `audit` object in that Zone's `resource-bundle.json`, outside every
   ResourceSpec and the controller-created empty `Zone.spec`. `audit.retentionDays`, default
   30 and range 1 through 3650, governs both exported segments and journal rows, but a journal
   row becomes prune-eligible only after durable export completion plus that retention
   interval. `audit.maxRecordsPerSegment`, default 65536 and range 1 through 1000000, and
   `audit.maxSegmentBytes`, default 67108864 and range 1048576 through 1073741824, bound
   rotation. This header change moves the only accepted resource-bundle pair from
   `schemaVersion: 3` / `bundleVersion: 1` to `schemaVersion: 4` /
   `bundleVersion: 2`; v4 `contentHash` covers canonical `{audit,resources}`, so an audit-only
   change cannot reuse a generation identity. Missing, old/mixed, malformed, misplaced, or
   unenforceable policy and any journal or segment prune failure produce typed degraded Zone
   health and block publication.
   Every sensitive audit or broker DTO and owner uses fixed redacted `Debug`, including
   `StoreSyncRequest`, `StoreSyncResponse`, the drain request, dispatcher errors,
   `SegmentWriter`, sink, exporter, root directory owner, and opaque storage handle owner.
   StoreSync wire fields, producers, consumers, schemas, and snapshots use only sealed typed
   digests or opaque handles. Present trace context becomes only its typed domain-separated
   digest, absence stays absent, and malformed input is denied before mutation; another
   digest class is never fabricated or relabelled as trace correlation.
6. One readiness projection is computed from store recovery, policy match, authenticated
   session/router admission, controller registration, watch admission, audit catch-up,
   mandatory controller health, and the `d2b-core-controller`-owned
   `Provider/system-core` registration. The minimum Provider handler set is the active,
   initialized, current `HostReconciler` and `UserReconciler`, observed through exactly one
   `Zone.status.handlers[]` record named `system-core-host` and exactly one named
   `system-core-user`. Each record carries `phase` and `lastReconciledAt`.
   `ProviderLifecycle` is a distinct aggregate handler name and cannot substitute for either
   record. No Wave 6 Provider dossier is an `adr046w5` readiness member, and no duplicate,
   missing, wrong-name, boolean, or detached-status substitute may satisfy this member. No
   component publishes itself. Startup and close collect per-Zone outcomes and visit every
   Zone; a missing or unhealthy system-core registration/handler degrades only that Zone and
   never aborts or silently drops later owners.
7. Installed-host migration begins with the target closure's
   `system.build.d2bHostGenerationDeploy` entrypoint and one durable, replayable handoff
   binding source/target system, broker, daemon, numeric protocol, negotiated operation
   catalogue fingerprint, bundle-pointer,
   complete bundle-set, stable-reference, and deployment-intent digests. The first 3/1-to-4/2
   migration obtains that entrypoint from an explicit target installable and never reads a
   target-generated stable file. Unprivileged resolution produces exactly one canonical Nix
   store output and caller-flake executable. Before authorization succeeds, the broker
   verifies that target object, creates a broker-managed GC root, and durably pins its
   canonical store identity, NAR hash, deployment-executable digest, and staged intent. It
   separately resolves only trusted installed-generation metadata and pins the canonical
   store identity, NAR hash, and executable digest of one broker-managed apply object. The
   caller-flake executable performs the unprivileged authorization only. Privileged apply
   invokes only the separately pinned installed apply object and performs no Nix eval, build,
   `nix run`, installable resolution, or symlink lookup; the broker reopens both pins and
   refuses any target/apply substitution, changed symlink target, digest mismatch, missing GC
   root, or cross-intent replay before mutation. On the accepted apply connection the broker
   obtains a connection-scoped peer pidfd directly from the accepted socket, binds the live
   peer's executable store/NAR/digest identity to the apply-object pin, and revalidates
   liveness, process start identity, and current executable identity immediately before each
   mutation. Peer exit, exec, PID reuse, mismatch, or ambiguous identity refuses. After
   allowing `host-generation.source-bootstrap-publish`, the acceptance matrix injects every
   one of the six exact transitions independently before each of the fourteen later ids in
   the closed registry from `quickstart.md`. The literal expected set has 84 post-first
   cases and is independent of production enumeration. Separate literal fixtures pin the
   ordered 15 mutation ids and the 90 pre/post case ids; production reads neither, and tests
   compare both against a separately authored literal 15-id test constant, never
   `mutation_edge_count` or another discovered count. Exact meta-poisons remove one production
   edge, one expected edge, and one pre-mutation verification hook; all must fail before
   evidence acceptance. Tests reject missing, duplicate, unknown, reordered, or unvisited
   edge/transition cases before accepting evidence. The prior mutation remains committed and audited; the refused edge and
   every successor have zero mutation count. The pidfd and executable fds are closed with the
   connection and are never serialized or persisted. Raw apply-peer pidfd/PID/start,
   socket uid/gid, cgroup/proc path, executable store/derivation/NAR/content, and
   device/inode/mount identity are internal comparison inputs only. The closed fifteen-row
   literal forbidden-value registry in `data-model.md` covers every one of those classes. It injects each canary
   independently and scans coordinator state,
   receipts/evidence, human, JSON, wire, error/`Display`, log, tracing event/span, metric
   name/help/label/value/exemplar, audit, panic, and `Debug`. Only the registry and private
   injection buffer are excluded. Where correlation is required, tests independently
   reconstruct only the exact fixed-binary process-instance or executable-identity digest
   preimage from `data-model.md`; wrong tag, framing, endian, order, or cross-class
   substitution refuses. Metrics omit both raw and digested peer identity.
   Because `--apply-authorized-handoff` carries neither an intent selector nor an authority
   token, the source broker keeps exactly zero or one durable nonterminal handoff intent per
   source generation. Authorization and apply selection share the coordinator's exclusive
   lock. Authorization refuses while any authorized, claimed, mutating, recovery-pending, or
   transfer-pending intent exists. Apply selects only the sole `authorized-pending` intent
   and atomically claims it for the kernel-derived connection identity; zero, multiple, and
   already-claimed intents refuse without waiting or choosing by age. A pre-mutation
   disconnect can release the claim only after a durable zero-mutation proof. After mutation,
   only durable coordinator replay of the same intent can accept a replacement connection
   from the same pinned apply object after the old peer is proven dead. Terminal intents are
   never selected or replayed by a later command.
   T595's unprivileged selector-free
   `d2b-host-generation-deploy --inspect-authorized-handoff [--json]` exposes the exact
   `HostGenerationHandoffStatusV1` projection from `data-model.md`. Every
   `recovery-pending` row names whether the live apply peer, source broker, or target broker
   owns progress, one closed wait/restart-existing-broker action, and the exact allowed
   successors. `recovery-irreconcilable` is valid only when immutable pre-mutation/outcome
   audit proves one complete rollback; restarting the existing broker unit drives only that
   rollback to `rolled-back`. Root inspection, an intent selector, a path, a token, a daemon
   recovery owner, or a new unit is forbidden. Human/JSON status contains only bounded
   state/phase/owner/action/successor enums and no identity, generation, path, or apply-peer
   canary.
   The entrypoint may build and verify the target closure, durably stage immutable bytes, and
   submit one opaque intent; it cannot publish a profile, control a service, initiate
   rollback, or select a path, unit, generation, command, or argv.
   A protocol-5 installed broker accepts the typed request after transfer. Before transfer,
   only the externally installed source-generation compatibility peers may receive exactly
   one accepted public-socket evidence fd after their authenticated daemon/broker Hello has
   matched both numeric protocol 4 and Hello `operation_catalogue_sha256` equal to the exact
   `source-handoff-v1` operation-catalogue fingerprint. Bare committed protocol 4 omits the
   field or advertises a different catalogue and refuses; it
   cannot route authority to a target-closure mode. Before fd transfer, the exact nonempty
   13-member `SourceGenerationCompatibilityFloorV1` census from `data-model.md` must contain
   one of every closed role, no extra or duplicate role, and one common accepted disposition
   and source generation. Missing, empty, stale-generation, stale-digest, or
   cross-disposition members refuse. A separately literal 13-row test constant, the
   independent role/artifact fixture, production registry, and poison visitor must agree
   exactly without deriving a count from another. The 91 poison ids, four matrix-meta
   negatives, and five issuer-proof poison ids must match their separately authored
   registries exactly. All 39 missing/stale-digest/cross-disposition cases preserve
   cardinality 13 and recompute every enclosing receipt and signature before the named
   semantic refusal. Each manifest, installation, validation, and import proof
   verifies with the transition key pinned by the accepted disposition; copied authority/key
   digests signed by an unpinned valid key refuse after canonical, framing, enclosing-hash,
   and unaffected-proof validation. The installed source receiver derives
   authority only from that consumed fd and its broker-sealed staged-intent binding. It
   accepts no serialized uid, gid, role, provenance, root, daemon, or caller claim.
   Before either broker acts, the unprivileged operator must pass the existing public-socket
   `SO_PEERCRED` plus current `d2b`-group Admin classification. The broker consumes that
   one-shot evidence into one durably sealed, nonfabricable capability bound to the complete
   staged intent and pinned store object; process/socket credentials, daemon identity, Hello,
   target-closure provenance, and bootstrap euid 0 are integrity or eligibility inputs only.
   The installed source broker before transfer and target broker after transfer advance a
   phase only by consuming that capability or a
   broker-issued phase attenuation and emit immutable pre-mutation and outcome audit rows for
   every system-profile, service, bootstrap, publication, repair, and rollback phase.

   The existing `d2b-priv-broker.service`, reached through
   `d2b-priv-broker.socket`, is the sole executable lifecycle owner before and after transfer.
   Its existing `Restart=on-failure` path starts or restarts the externally installed source
   broker, whose ordinary `serve` startup reopens the durable coordinator; the deployment
   entrypoint and `d2bd` never supervise it,
   and no transient, template, path, timer, or additional service is created. The broker
   durably owns the coordinator before the first mutation, including the exact lifecycle
   owner and pinned bootstrap generation. The installed source broker retains that ownership until the
   authenticated target protocol-5 broker durably adopts it exactly once. Target broker
   activation and durable coordinator transfer precede target daemon activation. Killing the
   entrypoint or compatibility process at any pre-transfer boundary must cause the existing
   broker unit to reopen the same coordinator and resume or roll back idempotently. The target
   daemon starts, completes fresh exact-generation protocol-5 Hello while explicitly unready,
   then presents a phase attenuation in the authenticated opaque publication request. Only
   after the broker durably publishes and audits the matching d2b pointer and stable reference
   may daemon ingestion and readiness proceed. On failure the broker-owned coordinator, not
   the entrypoint, daemon, or a new supervisor, reopens the durable handoff. Before transfer,
   only the capability-authorized compatibility mode under the existing broker service may
   resume or roll back its phase. After
   transfer, the existing `d2b-priv-broker.service` reopens the coordinator after broker
   restart and completes or rolls back even when target daemon startup or reconciliation
   fails. The broker restores
   prior pointer/reference bytes or verified absence before performing typed stock rollback
   and source-service restoration. Runtime refusal carries only
   `rebuild-host-generation`. Documentation gives fail-closed parameterized
   authorization/apply pairs for first bootstrap, stable-reference use, and rollback; every
   preflight stops before public-socket authorization or `sudo`, every privileged apply names
   the already authorized exact store executable, and runtime output contains neither command
   nor reference. Runnable documentation redirects Nix evaluation and build stderr directly
   to `/dev/null`; it creates no diagnostic file and emits only the fixed stage-specific
   `fail` literals shown in `quickstart.md`. The production entrypoint may retain at most
   16,384 stderr bytes in memory, never on disk; overflow is a fail-closed stage failure, all
   raw bytes are dropped before return, and only the closed identifier-free typed
   failure/remediation is emitted. Canary
   tests require raw evaluator/builder stderr to be absent from human, JSON, wire, log, audit,
   metric, span, and `Debug` output.

The concrete failures this permits are a committed generation whose process dies after its
effect intent becomes durable but before the effect completes, and an audit segment export
that fails after the mutation and its immutable authoritative journal row commit together.
The durable ledger makes the first recoverable; the operation-bound pending-audit result makes
the second observable without lying about success or rollback. The restart crash-window
matrices catch a lost, duplicated, ambiguous, or stale effect/export record, while the
transactional journal prevents an unaudited committed privilege change. The aggregate
readiness projection prevents the recovered store from becoming success-shaped while policy,
route, watch, audit export, controller, or the exact system-core Provider ownership is absent.

The apply-peer matrix uses the closed 15-member mutation-edge registry in `quickstart.md`,
not production self-enumeration. The first edge is
`host-generation.source-bootstrap-publish`; the remaining fourteen exact ids cover target
profile, broker service, coordinator transfer, daemon service, pointer/reference publish,
pointer/reference repair, and the six rollback/service-restore edges. With the closed six
peer-transition ids, the post-first expected set is exactly 84
`apply-peer/post-first/<edge>/<transition>` cases, plus the separate six pre-first cases.
Independent literal fixtures own the ordered 15 edge ids, the 90 case ids, and the negative
matrix. A separately authored literal 15-id test constant must equal both the fixture and
production order; neither cardinality nor expected visits may read a discovered registry
count. The three mutation-edge meta-poisons remove one production edge, one expected edge,
or one verification hook, and the 15 post-first negatives include explicit empty-set,
dynamic-skip, and missing-hook
case. Unknown, duplicate, missing, reordered, dynamically skipped, unvisited, selected-edge
mutation, successor mutation, durable-prefix change, or missing first audit fails before any
evidence is accepted. T589 freezes the independent flat expected-set files
`tests/golden/delivery/host-generation-mutation-edge-ids.txt`,
`tests/golden/delivery/host-generation-apply-peer-case-ids.txt`,
`tests/golden/delivery/host-generation-mutation-edge-meta-negative-case-ids.txt`,
`tests/golden/delivery/host-generation-post-first-negative-case-ids.txt`,
`tests/golden/delivery/host-generation-apply-peer-forbidden-values.tsv`,
`tests/golden/delivery/source-floor-v1/role-artifact-matrix.tsv`,
`tests/golden/delivery/source-floor-v1/poison-case-ids.txt`,
`tests/golden/delivery/source-floor-v1/matrix-meta-negative-case-ids.txt`,
`tests/golden/delivery/source-floor-v1/issuer-proof-negative-case-ids.txt`,
`tests/golden/delivery/source-floor-v1/issuer-authentication-negative-case-ids.txt`,
`tests/golden/delivery/source-floor-v1/hash-vector-negative-case-ids.txt`,
`tests/golden/delivery/source-floor-v1/receipt-negative-case-ids.txt`,
`tests/golden/delivery/sc002-sidecar-lock-case-ids.txt`,
`tests/golden/delivery/sc002-activation-receipt-negative-case-ids.txt`,
`tests/golden/delivery/sc002-census-negative-case-ids.txt`,
`tests/golden/delivery/sc002-request-output-negative-case-ids.txt`, and
`tests/golden/delivery/sc002-recovery-forbidden-values.tsv`; production enumerators and poison
generators may consume none of them. The forbidden-value registry is input only to test
injection and captured-surface comparison, never to production filtering. `data-model.md`
lists the complete literal 15-edge, 90-case, and 91-case source-floor fixture contents rather
than leaving their bytes to a runtime Cartesian product. T604 separately owns
`tests/golden/delivery/{host-generation-pre-start-case-ids,host-generation-unit-census-case-ids}.txt`;
their independent literal 15-case and 27-case constants must agree before the host test can
run. The source
floor and hermetic matrix-meta negatives run through
`D2B_ENABLE_FIXTURE_BUILD=1 make test-fixture-contracts`; the mutation and unit cases are
also required in T604's no-skip `make test-host-integration` result. `tasks.md` pins every
literal axis, row, negative id, ordering rule, runner, and cardinality for these files. A generated
expected set, runtime count, fixture that imports the production registry, or filter that
learns its forbidden values from production is ineligible.
`tests/golden/delivery/sc002-domain-hash-vectors-v1.json` is the separate shared typed-byte
oracle; it is reconstructed from semantic inputs and is never a production registry or a
source for test case enumeration.

#### Serial dependencies and file ownership

**Round 24 correction to the long external-amendment row below:** its current
SC-002 counts are six closed remediation values, exact 61/73/35
receipt/census/direct-final-publication registries, and seventeen recovery-redaction rows. The external
owner must also adopt the complete
preimage/request unnamed-inode protocols, total root-instance/node grammar, coverage-repair
projection, and handoff contract assigned below. The row remains one external prerequisite;
no task or ownership edge is added.

| Stage | Task(s) | Ownership and concurrency |
| --- | --- | --- |
| External Version 2 delivery-contract amendment | no feature task | Before T589, the external owner accepts Version 2 of `ADR-046-validation-and-delivery` and generates `ADR-046-validation-and-delivery-traceability.{json,md}`. The generated bijection must own `VD2-SC002-RECEIPT`, `VD2-SC002-PUBLICATION`, `VD2-SC002-INCIDENT`, `VD2-SC002-DISPOSITION`, `VD2-SC002-RECOVERY`, `VD2-SC002-SOURCE-FLOOR`, `VD2-SC002-REGISTRIES`, and `VD2-SC002-TRACEABILITY`, mapping each to exact schemas, fixtures, implementation owners, tasks, and gates. Required approvals, manifest regeneration, Gate 0, and ancestor checks remain mandatory. No feature task authors or substitutes for that contract. |
| External source-floor prerequisite | no feature task | The accepted external compatibility disposition must name and deliver the `SourceGenerationCompatibilityFloorV1` producer/installer and typed import/validation authority. Its immutable four-record chain must reach `imported-for-exact-C/Q` for the exact source generation before T589 is ready. The installed coordinator atomically consumes the exact one-time origin record into private nonserializable, non-clonable `ProtectedSourceFloorOrigin`; the disposition-pinned validator consumes it while authenticating all four proofs into private `AuthenticatedSourceFloorIssuerProvenance`, rejects copied authority/key digests without producing it, and consumes the intermediate by value into one private validated-floor result. Later boundaries borrow/attenuate that result and never revalidate serialized evidence. Copy, replay, or repeated mint refuses. Those authorities produce, install, and validate objects against the already accepted Version 2 schemas and vectors; they do not own or redefine the repository encoding, digest, schema, or golden contract. This row owns no feature, source, test, Nix, normative, or changelog file and makes no implementation claim. |
| External retained-request disposition | no feature task | The external delivery-contract/tooling owner must land the contract and typed validator for `Wave5RetainedRequestDispositionV1`. T219 may consume its validated output only after F freezes. This row owns no repository file in this feature and cannot supply panel sign-off. |
| Feature-editor reconciliation | T603 | After the accepted FR-036 predecessor and one valid T072 disposition, run analysis and one current selected-roster plan lifecycle at clean base A. If all T073-T218 obligations are satisfied, one `/d2b-spec-edit` batch checks exactly those 146 rows plus T603 and the integrator creates dedicated checkbox-only commit C. The editor receipt and C are the sole authority. T603 owns no source, changelog fragment, scratch receipt, digest chain, or sidecar. Fresh analysis and a new selected-roster lifecycle bound to clean C are required before T589. |
| Integrator prep | T589 | Depends on T603 plus accepted Version 2 and the installed source-floor disposition. Owns the shared registrar, policy-bootstrap, operation-status, audit hook, exact-eight evidence validator, and only implementation/schema/fixture/API rows assigned to T589 by generated `VD2-SC002-*` traceability. It verifies but does not produce the external source floor. No feature-local SC-002 or source-floor copy is authority. |
| Serialized implementation | T590-T594, T605 | T590, T591, and T594 start together from T589. T592 starts only after T591 because both own `transaction.rs`; T593 starts after T592 and consumes its sealed broker peer-pidfd operation without another lockfile or dependency prerequisite; T605 starts after T593 and regenerates the shared API snapshots. T592 is the sole in-feature atomic broker-wire, target-broker, FFI-quarantine, and privilege-contract owner: it changes both StoreSync DTOs with every producer/consumer; defines the audit drain, `OpenPeerPidfdFromAcceptedSocket`, and versioned protocol-5 target host-generation adoption operation; consumes as immutable input the externally installed exact 13-member `SourceGenerationCompatibilityFloorV1` census, including the numeric-protocol-4 peers and exact negotiated `source-handoff-v1` catalogue fingerprint, but owns no source census member; owns only the target-v5 schema/catalogue/fingerprint/snapshot/fixture transition, target broker validation of the broker-managed target-object/GC-root and installed apply-object pins, the target coordinator adoption half, and exact source-to-target ownership transfer; binds source/target broker/daemon generations, numeric protocols, and exact catalogue digests; makes exact Hello mandatory before d2b-state publication; requires initial public-socket Admin evidence and a sealed durable handoff capability for every phase, while denying daemon-identity/euid0-only and every caller-claimed/admin-on-broker/launcher/root/HostShutdown/remote path; bumps `PROTOCOL_VERSION` 4 to 5; and updates `d2b-contracts`, `d2b-core` privilege sources, target broker protocol/runtime/bootstrap/`sys.rs` sources, target `d2bd` Hello/version and both ancillary-fd receive paths, `nixos-modules/privileges-json.nix`, `xtask` generators, Rust/Nix parity and policy tests, the target-v5 handoff schema/catalogue row, `wire-protocol.json`, `privileges.json`, target daemon/privilege catalogues, target fingerprints and snapshots, target-v5 fixtures, workspace lockfile, and standalone broker lockfile in one commit. The external prerequisite must already have installed the exact 13-member source census atomically; T592 refuses every `missing`, `duplicate`, `extra`, `empty`, `stale-generation`, `stale-digest`, and `cross-disposition` source member, plus a legacy or otherwise mismatched source contract, rather than regenerating it. Both receive paths require `MSG_CMSG_CLOEXEC`, complete ancillary parsing, exact-one-fd validation, and close-on-every-error behavior. A safe dependency may provide peer-pidfd acquisition only if it meets the exact contract; otherwise repository-authored unsafe is confined to approved `sys.rs` with per-block `SAFETY:`. T592 also owns `d2b-audit`; the authoritative `ZoneBundle`; duplicate-envelope retirement; and the one enforcing pidfd policy suite under `make test-fixture-contracts`, including a poison fixture for the forbidden `nix` 0.31.3 `PeerPidfd` wrapper. `make test-policy` must not duplicate or claim this suite. A new project FFI crate and a local session-crate syscall fallback are ineligible. T605 owns its Zone enum, governing specs, paired reference, and final API snapshots. T592's serialized commit may edit `d2bd/src/lib.rs`; after it lands, ownership transfers to T595. |
| Serial daemon composition | T595 | Sole writer after T592 for `d2bd/src/resource_runtime.rs`, `d2bd/src/lib.rs`, `d2bd/Cargo.toml`, new `d2b/src/bin/d2b-host-generation-deploy.rs`, `d2b/Cargo.toml`, `nixos-modules/{bundle-zones,host-daemon,host-broker,options-site}.nix`, the Type-1 `host-generation-rebuild-ref.nix` case, and Type-10 `host-generation-handoff.nix`; begins only after T590, T592, T594, and T605 converge. It owns startup ingestion, exact 4/2 consumption, the parameterized unprivileged validation/build/stage/public-socket-authorization/opaque-request-only target-closure entrypoint, target-generation installed apply behavior, and broker-before-daemon unit ordering. The caller-flake entrypoint resolves once while unprivileged and never runs under `sudo`. Privileged apply invokes the exact separately broker-pinned immutable object from trusted currently installed-generation metadata, receives no flake URI, installable, reference, target executable, path selection, command, or argv to reevaluate, retains at most 16,384 Nix-stderr bytes in memory, fails closed on overflow, drops the bytes, and emits only fixed redacted errors. Its command carries no intent selector or authority token. The broker enforces the durable single-nonterminal-intent rule, atomically selects and claims only the sole `authorized-pending` intent, and refuses zero, multiple, concurrent, stale-claim, or terminal selection before mutation. Its accepted connection is bound through a direct connection-scoped peer pidfd and live executable identity to that exact pin; exit, exec, PID reuse, mismatch, or ambiguity refuses, and no pidfd is persisted. A pre-mutation disconnect may release only a proven zero-mutation claim; post-mutation replacement is coordinator replay of the same intent and same pinned apply object, never caller selection. The accepted external source-generation disposition owns the exact nonempty 13-member `SourceGenerationCompatibilityFloorV1` census; its installed source broker owns the pre-transfer target-object/GC-root/apply-object pins, durable coordinator, stock profile publication, service transition, 3/1 bootstrap, rollback, and transfer. T592's target broker owns only post-transfer phases. T595 consumes that accepted external contract and wires target broker-before-daemon ordering; changing target `host-broker.nix` cannot by itself satisfy or install the source-generation prerequisite. Neither daemon nor entrypoint owns recovery, and no new unit or runtime unit override is permitted. The option is required `lib.types.strMatching` with exact `<flake-ref>#<configuration-name>` grammar, 2048-byte total limit, and 64-byte selector limit. `make nix-unit-pin` and Type-1 positive/boundary/missing/malformed/selector cases are required in addition to the VM crash/rollback matrix. |
| Pulled-forward Network production path | T336-T355 | These rows execute after T595 and before T604. Existing code keeps `NetworkEffectPort` in `d2b-provider-network-local::controller`. T336 records the generated destination drift and implements the production adapter at `packages/d2bd/src/network_effect_adapter.rs`, with serialized post-T595 ownership of `d2bd/{Cargo.toml,src/lib.rs,src/resource_runtime.rs}`. The adapter uses only typed broker operations. The accepted contract fixes `effectiveEastWest = Network.spec.isolation.allowEastWest && d2b.site.allowUnsafeEastWest`, both default false. T346 carries the site gate to production and T350 proves all four cases. |
| File-disjoint acceptance and docs | T596-T599, T604 | T596-T599 start after T595. T604 additionally waits for pulled-forward T336-T355. T604 owns only its three acceptance tests, two host-generation fixtures, and their Makefile recipe; it consumes only its generated `VD2-SC002-*` rows and cannot implement the Network adapter. T599 owns the CLI contract, public recovery runbook, generated action mapping, and client-side operation-ID response-loss proof. |
| Integrator convergence and freeze | T220 | Merges every slice and generated artifact, verifies accepted Version 2 and generated `VD2-SC002-*` traceability, folds exactly thirteen fragments, requires the T599 runbook/action map, and runs integration, CI, and drift. It cannot freeze F until T336-T355, the exact `effectiveEastWest` double-opt-in migration, real adapter, and all four production cases pass. Each provisional candidate uses one current selected-roster lifecycle; every selected seat must approve before F. Historical sole Network opt-in cannot close this row. |
| Frozen-candidate evidence | T600-T601 | Read-only evidence lanes run against F. They write delivery evidence only, not repository files, and emit the exact closed validation identifiers assigned below. T600 imports `operator-nix-activation-cleanup` by passing T604's explicit external `0600` receipt input to T589's candidate-local importer; the importer durably installs the sidecar before it publishes the record. They may run together subject to the heavy-gate limit. |
| Mechanical evidence convergence | T602 | Revalidates exactly one T072 historical/current remedial disposition without requiring or changing T072's checkbox, then verifies dependency closure, resume identities, clean F, T220's unanimous final phase-panel receipt, and the exact evidence-identifier multiset. T219 is blocked until it passes. |
| External disposition and conditional close | T219 | Revalidates the T072 disposition, T603 phase-plan chain, and T220 final nonbinding phase-plan receipt, but those process rounds do not replace or dispose the retained Wave 5 binding delivery request. T219 first requires the external owner to land the delivery-contract/tooling change and validator, then imports exactly one valid `Wave5RetainedRequestDispositionV1` bound to the byte-preserved request and exact F. Before that import, T219 authorizes no request, attestation, seal, merge-target registration, merge eligibility, or merge. `remain-blocked` stays blocked; `abandon-without-merge` is terminal and cannot advance; `recover-panel-without-new-request` permits only the external recovery-attestation surface, still requiring the complete strict legacy fixed-ten unanimous exact-F panel before seal or merge. The record creates no second request and supplies no panel result or constitutional waiver. F and `adr046w5` delivery history stay immutable. |

Accepted Version 2 `ADR-046-validation-and-delivery` and generated `ADR-046-validation-and-delivery-traceability.{json,md}` solely own every SC-002 and source-floor protocol row. T589, T592, T595, T600, T604, and T220 consume only their generated assignments. The generated bijection must map every `VD2-SC002-*` identifier to exact schemas, fixtures, implementation owners, tasks, and gates and must pass Gate 0 before T589. Historical feature-local encodings, counts, state tables, and recovery matrices are non-authoritative. The host-generation path remains broker-owned and uses only the three existing root-visible units.

T603 remains the sole in-feature direct prerequisite of T589, but it is an editorial accounting gate rather than a second delivery protocol. At clean base A, one cross-artifact analysis and one current selected-roster plan lifecycle bind the complete feature snapshot. If every T073-T218 obligation is satisfied, one `/d2b-spec-edit` batch checks exactly T073-T218 and T603 and the integrator records that exact change as dedicated commit C. The editor receipt and C are the only authority; T603 owns no source, changelog fragment, scratch receipt, digest chain, resume sidecar, or custom atomic replacement behavior. Any open row leaves all 147 boxes unchanged. Because C changes feature content, T589 additionally requires fresh analysis and a fresh selected-roster plan lifecycle bound to clean C and the new snapshot. Final-candidate T600/T601 evidence remains separate.

T589 may touch files later owned by a serialized successor because its purpose is to
establish the contracts those successors implement. No two parallel tasks own the same file.
Cargo workspace membership, generated spec manifests, flake outputs, shared changelog, and
feature artifacts remain integrator-only.

Every source-writing completion slice has disjoint, exact fragment ownership:

| Slice owner | Sole fragment |
| --- | --- |
| T589 | `changelog.d/resource-api-production.md` |
| T590 | `changelog.d/resource-policy-bootstrap.md` |
| T591 | `changelog.d/store-policy-neutrality.md` |
| T592 | `changelog.d/resource-bundle-audit-carrier.md` |
| T593 | `changelog.d/componentsession-peer-admission.md` |
| T594 | `changelog.d/controller-effect-ledger.md` |
| T595 | `changelog.d/zone-runtime-production.md` |
| T596 | `changelog.d/authenticated-publication-acceptance.md` |
| T597 | `changelog.d/effect-replay-acceptance.md` |
| T598 | `changelog.d/audit-acceptance.md` |
| T599 | `changelog.d/cli-operation-recovery.md` |
| T604 | `changelog.d/operator-resource-activation.md` |
| T605 | `changelog.d/system-core-handlers.md` |

These paths supplement each task owned-file list. T220 requires exactly these thirteen source-writing fragments and folds them only after their coordinated version, reference, test, schema, and release treatment is complete. T603 owns no changelog fragment or source file. T600-T602 and T219 write only external evidence or delivery state.

T589 and downstream tasks implement only their generated Version 2 traceability rows. No feature-local incident, source-floor, registry, encoding, or recovery detail is implementation authority. T220 blocks F on exact generated traceability and drift equality.

#### Wave 5 validation and evidence

The implementation tasks run focused hermetic tests while writing their files. Before T595,
T605 runs the `d2b-contracts` unit/serialization cases, the existing
`policy_contracts.rs` Layer-1 policy surface, `make api-surface-pin` followed by the API
snapshot check, and the targeted Zone desired-schema generator comparison. Its schema proof compares
`docs/reference/schemas/v3/core.d2bus.org_Zone.schema.json` with the T589 base and requires no
byte change after generator execution; the schema is never hand-edited. T605's completion is
bounded to its owned enum, tests, guard, normative/reference pages, and API snapshots. It does
not require the later T595 emitter or T599 consumers and does not run the full `make
test-drift`, because generated spec manifests are integrator-owned. After T595, T596-T599 and
T604 fan out together. T596-T598 add production-boundary tests that enter through the
daemon/session boundary and the registered ZoneBus route, T599 reconciles the T595 emitter
and its downstream reference/status consumers with T605's contract, and T604 adds the missing
operator boundary. T220 then reconciles the integrator-owned generated spec manifests,
requires the generated broker wire/privilege outputs and parity tests plus
`make nix-unit-pin`, and runs the full drift gate before F is frozen. T604 first verifies the emitted Nix resource
bundle in
`packages/d2b-contract-tests/tests/resource_operator_activation.rs`. Its lowest feasible
production-boundary leg is the Type-3
`packages/d2bd/tests/resource_operator_activation.rs`, which consumes those exact generation
bytes through the daemon startup/change-ingestion entry and production store/controller path
without calling ResourceService directly. T595 separately owns the Type-1 option case and
Type-10 handoff test; T604 does not replace either. Run the lower legs through
`D2B_ENABLE_FIXTURE_BUILD=1 make test-fixture-contracts` and `make test-rust`. Because a real
owned host effect requires systemd, broker mutation, and a booted NixOS system, the canonical
Type-10 destination is `tests/host-integration/resource-operator-activation.nix`, run only by
the public heavy-gated `make test-host-integration` target. It declares the spec's exact
Zone `acceptance` fixture through Nix. The three acceptance resources, selected Providers,
and decisive outcomes are closed:

| Acceptance resource | Selected Provider and exact distinguishing config | Positive effect/readiness and removal obligation |
| --- | --- | --- |
| `Volume/acceptance-state` | `Provider/volume-local`; Provider artifact `volume-local-provider`; `controllerExecutionRef = Host/host-system`; sole `state-root` local-path source policy for `state`; resource uses that opaque policy, one private `0700` no-follow root owned by `User/d2bd`, one controller view, no attachment/quota | Broker-backed root and identity marker are provisioned/adopted and read back; resource and layout are `Ready`/`Current`. It remains unchanged throughout Device removal. |
| `Network/acceptance-net` | `Provider/network-local`; Provider artifact `provider-network-local`; `controllerExecutionRef = Host/host-system`; `10.20.0.0/24`, `192.0.2.0/30`, mandatory host blocklist, east-west denied, empty DNS forwarders/attachments, mDNS off, `net-vm-base` nixos-system artifact | Both derived bridges, IPv6 suppression, and this Network's firewall projection are real and read back; `FabricReady`, `FirewallReady`, `ConfigVolumeReady`, `NetVmReady`, and `DhcpReady` plus both bridge phases are ready. Network-owned Guest dependencies do not count as independent Guest acceptance. It remains unchanged throughout Device removal. |
| `Device/acceptance-tpm` | `Provider/device-tpm`; Provider artifact `d2b-provider-device-tpm`; `controllerExecutionRef = Host/host-system`, `logLevel = 20`; exclusive emulated Device owned by `Guest/acceptance-vm`, empty selector, schema `device-tpm.d2bus.org/Device/spec` version `1.0.0`, setting `logLevel = 20` | Controller-managed TPM state Volume and marker validate, mandatory flush completes, real swtpm Process runs, typed TPM Endpoint publishes, and Device is `Ready`/`Current`, present, and healthy. Removal stops and waits for then deletes swtpm, deletes any non-terminal flush process, preserves the same state Volume identity/marker, releases its Volume references, clears the finalizer, deletes only the Device, and leaves the Endpoint unresolvable. |

Support Host/User/Guest/system Provider resources may be present but cannot substitute for one
of these three. This names acceptance scope only; Network implementation remains owned by
Wave 4. The Type-10 test starts only after the accepted external compatibility floor is
atomically installed in the 3/1 source generation and its exact nonempty 13-member census
passes. Every role in `SourceGenerationCompatibilityFloorV1` occurs once and every member
binds the same disposition and generation. Bare
committed protocol 4 or a source-peer fingerprint mismatch is a refusal case, not the
positive control. It obtains `d2bHostGenerationDeploy`
from the explicit target installable and reaches
4/2 through that entrypoint without reading an absent stable reference. It resolves the
target executable once while unprivileged, requires the broker-managed GC root and immutable
target-object pin before authorization returns, separately pins the installed apply object
from trusted installed-generation metadata, and applies only through that object without
privileged reevaluation. The test substitutes the target executable and apply object, changes
the installed symlink, and forces apply-peer exit, exec, PID reuse, identity mismatch, and
ambiguity before the first mutation. After one mutation succeeds, the test repeats every
transition in all 84 literal post-first cases over the fourteen exact later edge ids and
proves refusal before that edge, zero later mutations, and preservation of the first
mutation's audit. No
peer pidfd persists. All fifteen raw apply-peer canaries remain absent from every output
surface; only the typed process-instance and executable-identity digests may correlate their
two classes, and metrics
carry no identity label. Accepted
public-socket evidence crosses as exactly one fd only after exact source catalogue
negotiation, and no root, provenance, daemon, or caller claim substitutes for it. The source
broker coordinator is
durable before first mutation and the existing `d2b-priv-broker.service` starts/restarts its
installed source broker across
entrypoint or compatibility-process death. The coordinator then transfers durably to the
target broker before daemon activation, with no new unit. Raw Nix stderr canaries never
escape the fixed typed error surface. The new broker must start before the new daemon. The
daemon completes exact protocol-5 Hello
while unready, submits the authenticated opaque publication request, and becomes ready only
after broker-durable d2b-state publication and complete ingestion. Subsequent
declaration/removal deployments
use the stable-reference entrypoint without a manual daemon restart or private reload and
require every one of those three resources to reach its real owned effect and
readiness. The target must discover a
nonempty `vmChecks` set, enumerate and successfully build
`vmChecks.x86_64-linux.resource-operator-activation`, and report no skip; skipped or empty
output is ineligible evidence. It must also enumerate and successfully build
`vmChecks.x86_64-linux.daemon-restart-vm-survival` with no skip and bind that FR-075 result to
the same F through `operator-nix-activation-cleanup`. Typed refusal cases are separate
negative tests and cannot satisfy the positive story. The test removes only
`Device/acceptance-tpm`, switches again, and observes the exact finalizer outcome above while
`Volume/acceptance-state`, `Network/acceptance-net`, and unrelated resources remain ready,
intact, and unrecreated. Guest runtime-effect ownership is absent from the Wave 5 system-core owner and is
deferred specifically to Wave 6 `Provider/runtime-cloud-hypervisor`; Guest emission, ingestion, status, or refusal cannot
satisfy this positive criterion. A direct
ResourceService or `WatchService` call, `ProductionWatchHarness`, fixed subject, fake
endpoint, or manually set readiness field may remain useful unit coverage but is explicitly
ineligible as T219 evidence.

T589 implements one hermetic `adr046w5` closed-evidence profile in the delivery validator
before any final candidate can be frozen. The profile compares the imported record multiset
to the following table byte-for-byte and is invoked at panel-request/panel-attest, seal, and
merge-eligibility, not only by T602 prose. Its table-driven negative suite must independently
reject a missing row, an extra row, a duplicate pair, an unknown identifier, a right
identifier in the wrong lane, and one record conflating two required rows. T220 reruns that
suite before freezing F. After T220 completes every repository change and freezes F, T600 and
T601 import generic `EvidenceRecord` objects bound to F. T602 invokes the same validator and
adds the receipt, ancestry, and clean-tree checks; it does not implement or substitute a
second validator.

| Closed `validation` identifier | Owner | Delivery lane | Required content |
| --- | --- | --- | --- |
| `production-session-watch` | T600 | `github-ci` | Same-Zone request/watch through the production session/router plus cross-Zone and self-named subject denial; one accepted-socket evidence object shared by adapter, descriptor, bus Unix transport, and session seam; T592's typed broker `OpenPeerPidfdFromAcceptedSocket` operation transfers only the accepted socket and returned pidfd with `SCM_RIGHTS`; both receive paths set `MSG_CMSG_CLOEXEC`, reject truncation, own every received fd immediately, require exactly one expected fd, and close all received fds on count/type/index/decode/later-validation errors; descriptor-count and exec-leak matrices cover success, missing, extra, malformed, truncated, and post-receive failure; a safe dependency satisfies the exact contract or exact-optlen validation, immediate ownership of any returned fd, and failure closure live only in the approved broker `sys.rs` FFI quarantine with narrow allowances and immediate per-block `SAFETY:` comments; no repository-authored unsafe outside that quarantine; injected short-result fd-count, unsupported/malformed/missing-CLOEXEC/leak/dead/numeric-only/reuse/credential/generation/cgroup/ambiguity refusals; the sole enforcing pidfd policy runner is `D2B_ENABLE_FIXTURE_BUILD=1 make test-fixture-contracts`, whose poison suite rejects the `nix` 0.31.3 `PeerPidfd` wrapper, a new project FFI crate, and local syscall/raw-fd fallback; no caller-supplied verifier/credential constructor or re-export; private registrar issuance; and a fresh restart peer pidfd |
| `effect-replay-cleanup` | T600 | `github-ci` | Every generation/effect-ledger crash window, replay/adoption, and stale/zero/wrong-UID/ambiguous cleanup refusal |
| `audit-drain-replay` | T600 | `github-ci` | Transactional authoritative rows; fd-anchored file/directory-durable export and restart replay; digest-plus-ordinal deduplication; durable operation inspection; replay-binding denials; fixed-digest/record limits; valid-present/absent/malformed trace behavior with typed correlation and no fabrication/relabel; typed `StoreSyncRequest`/`StoreSyncResponse` producers, consumers, schema and snapshots; fixed redacted `Debug` for every migrated producer, both StoreSync wire DTOs, broker-drain DTO, SegmentWriter/sink/export/directory/opaque-handle owner; post-export journal retention and prune health; and raw identifier/trace/path/handle-canary absence |
| `system-core-handler-contract` | T600 | `github-ci` | T605 enum/list/API/reference/schema proof, T595 emission, T599 consumer agreement, exact live handler readiness, non-substitution, and multi-Zone isolation |
| `operator-nix-activation-cleanup` | T600 | `local-host` | T604 real Nix-to-daemon-to-controller-to-broker activation and cleanup for the exact Volume, Network, and TPM Device resources; accepted Network/Host double opt-in with all four production cases; no-skip FR-075 continuity; and every T600-owned generated `VD2-SC002-*` row. Feature-local protocol detail and historical counts are not evidence. |
| `resource-plane-rss-owner-fanin` | T601 | `local-host` | Whole-process RSS at 10,000 resources/100 authenticated watches with no baseline subtraction and store, policy, ResourceService, controller endpoint/fan-in, and audit journal/export owner counts exactly one; the system-core registration/list belongs only to `system-core-handler-contract` |
| `wave5-removal-proofs` | T601 | `github-ci` | Every manifest-label W5 removal predicate rerun against F |
| `cli-reference-conformance` | T601 | `github-ci` | Emitted CLI/help/JSON/wire behavior; accepted Version 2 amendment and Version 1 migration guidance; exact ID, exits, mandatory envelope fields, DTO/schema, retained `op inspect --deadline`/`--no-deadline` plus mutual-exclusion/cancellation coverage, identifier-free static human guidance, closed JSON remediation actions, and all T599 reference comparisons |

The five T600 identifiers and three T601 identifiers are exclusive. Their required content is the table above plus the accepted generated traceability rows; no free-form validation name or copied protocol may satisfy another row.

T602 has a mechanical done condition: T603, T589-T599, T604, T605, and T220 are complete; the editor receipt and checkbox-only commit validate; C is an ancestor of F; the exact eight T600/T601 records bind F and its tree; the checked-in closed-profile validator passes at every named delivery boundary; generated `VD2-SC002-*` traceability and drift checks pass; and HEAD is clean at F. T602 does not reopen a feature-local SC-002 copy. Any missing, duplicate, stale, wrong-owner, wrong-candidate, non-enforcing, or ungenerated traceability row blocks close.

The T603 edit is authorized only by one clean-base selected-roster lifecycle, the exclusive editor batch receipt, and the dedicated checkbox-only commit. Fresh analysis and a new selected-roster lifecycle on the changed snapshot gate T589. No validator source, scratch receipt, digest chain, sidecar, or custom resume protocol exists.

At wave close, T220 converges all content and iterates the nonbinding
`/d2b-panel-round plan` phase surface over provisional integrated candidates. A finding routes
only its scoped fixes through T220, validation, and a delta/full-context phase-panel rerun.
Only unanimous phase convergence freezes final F before exact-candidate evidence. These
phase-plan rounds create no delivery candidate request, no `panel-request.json`, and no
wave-scoped binding reservation; they cannot replace or dispose a
`/d2b-panel-round work` delivery request. T220 never invokes a binding delivery request.

Wave 5 already retains such a delivery request from the pre-amendment candidate. That request
consumed the wave's once-per-wave request even though it has no attestations or seal. T219
therefore remains non-authorizing until the external delivery-contract/tooling owner lands
the contract and validator for `Wave5RetainedRequestDispositionV1`, and that validator
imports exactly one record binding the retained bytes, the accepted constitutional
predecessors, and exact F. Feature-local planning does not issue a second request or silently
reclassify the first. The record's action is closed: `remain-blocked` stays blocked;
`abandon-without-merge` is terminal without Wave 6 or release advancement; and
`recover-panel-without-new-request` permits only one external recovery-attestation sequence
linked to the retained request. That retained-request sequence still requires the complete strict legacy fixed-ten panel,
all records bound to F/commit/tree/disposition, and `signoff = true` exactly when
recommendations are empty. The disposition is neither an attestation nor a waiver. Missing,
partial, stale, nonunanimous, reduced-roster, or recommendation-bearing panel state enters
terminal `panel-refused` and authorizes no seal or merge. Only `panel-satisfied` may proceed
to seal and byte-identical-F merge eligibility. Any permitted successful merge preserves F's
tree; afterward W6 rebases its own branch while F and `adr046w5` delivery history remain
immutable.

### Spec corrections

| Prose drift | Canon kept | Planning correction |
| --- | --- | --- |
| Earlier feature prose required T604 to prove a positive owned Guest effect in Wave 5. | Committed `packages/d2b-provider-system-core/src/ownership.rs` does not own Guest runtime effects, and the four Guest-capable runtime families are assigned to Wave 6 and absent at this Wave 5 base. The authoritative `ADR046-ch-001` validation already names real-KVM end-to-end Guest boot through `make test-host-integration`. | Wave 5 positive operator acceptance covers exactly `Volume/acceptance-state`, `Network/acceptance-net`, and `Device/acceptance-tpm` as a partial US1 production-plane checkpoint. Full US1 acceptance is mechanically bounded to `Provider/runtime-cloud-hypervisor`: T384 owns the controller and authoritative real-KVM/guest-control integration obligation, T384-T390 own the exact family files, and T479/T480 require exact-F6 candidate-bound host-integration evidence. Guest emission, ingestion, status, or an actionable refusal is not evidence. |
| Earlier amendment prose treated W4 Network work as sufficient for the Wave 5 positive. | W4 landed the Provider trait and hermetic fake-port behavior, but the real production adapter and several network-local paths remain in T336-T355; `NetworkEffectPort` is in the Provider crate, contrary to the generated destination. | Preserve W4 history, record the destination drift, and pull T336-T355 forward before T604. T604 remains acceptance-only. T220 requires the accepted double-opt-in migration, real adapter path, and all four production Network/Host cases. |
| Earlier feature prose asserted that W4 implemented Host/site plus Network double opt-in for east-west traffic. | Untouched external `ADR-046-resources-network` makes `Network.spec.isolation.allowEastWest` the sole opt-in and says no Zone-level gate is required. Existing code places `NetworkEffectPort` in `d2b-provider-network-local::controller`, while no production adapter composes it to the broker. This feature cannot silently claim a different historical implementation. | T070 and T071 may record W4's historical sole-opt-in result, but that result cannot close Wave 5. Before T604 or T220, an accepted versioned network amendment and migration must require Network plus Host/site double opt-in; T336-T355 are pulled forward as production prerequisites and must implement the real adapter and network-local path; and all four Network/Host combinations must pass through the actual emitter/controller/broker/net-VM path. Preserving sole Network opt-in, a fake port, or a declaration-only fixture leaves T604 and T220 blocked. |
| AGENTS.md says the host exit census matching `d2b*`/`microvm*` returns three. | Committed code exposes canonical `d2b.slice` plus `d2bd.service`, `d2b-priv-broker.socket`, and `d2b-priv-broker.service`. | FR-075 enumerates the full loaded namespace, fails on listing error, excludes exactly `d2b.slice`, sorts, and compares the remainder with exactly the three service/socket names. The conforming raw census is four. Unexpected slice and service injections survive the sole exclusion and fail equality. This feature records but does not edit the external AGENTS.md drift. |
| Earlier handoff prose treated a broker-derived daemon principal and bootstrap euid 0 as independent authorization. | The existing lifecycle authorization chain is public-socket `SO_PEERCRED` plus current `d2b` group classification; broker-socket identity and root execution are not substitute operator authority. | Initial handoff admission transfers the accepted public-socket evidence only after the installed source peers negotiate numeric protocol 4 plus the exact `source-handoff-v1` catalogue fingerprint, then consumes that attachment into one nonfabricable intent-bound capability. No serialized caller/root/provenance/role claim substitutes. Every source or target broker phase consumes the capability or a phase attenuation; daemon identity, Hello, target-closure provenance, and euid 0 are integrity/eligibility checks only. |
| Earlier handoff prose claimed the target closure's compatibility broker could run under the existing broker service before profile publication and allowed the caller-flake target executable to run under `sudo`. | Committed `d2b_contracts::PROTOCOL_VERSION` is 4 and its `BrokerRequest` and operation-catalogue fingerprint have no host-generation handoff operation. Committed `nixos-modules/host-broker.nix` makes `d2b-priv-broker.service` execute the installed generation's `brokerPackage`, not a target-closure binary. Immutability does not make caller-selected executable code a trusted root entrypoint. | No executable authorized source actor exists at this base. T589 and downstream Wave 5 implementation are blocked on an accepted external source-generation compatibility disposition that atomically installs the exact nonempty 13-member `SourceGenerationCompatibilityFloorV1` census before migration. That external owner is outside this feature. T592 consumes the census and owns only target protocol-5 adoption and target artifacts; T595 owns target-generation behavior. The caller-flake target entrypoint runs only unprivileged. Only the separately pinned installed apply object runs under `sudo`, and its live connection peer pidfd/executable identity must match the pin before every mutation. Target/apply/GC-root substitution and peer exit/exec/PID reuse/mismatch/ambiguity refuse. Target-only code, a new unit or override, an entrypoint child or mutation path, daemon recovery, and a synthetic starting image are not accepted substitutes. |
| Earlier follow-up prose called the new source handoff row simply "protocol 4", assigned its compatibility artifacts to T592, and described the external source set with open-ended plurals. | The committed numeric protocol-4 peers negotiate a closed catalogue that does not contain the row; silently changing that catalogue under the same undifferentiated handshake is incompatible. | External scope escalation: the accepted source-generation disposition must install the exact nonempty 13-member `SourceGenerationCompatibilityFloorV1` census from `data-model.md`. Every role occurs once and every member binds the same disposition and source generation; `missing`, `duplicate`, `extra`, `empty`, `stale-generation`, `stale-digest`, and `cross-disposition` poison cases refuse. That external owner adds actual two-peer Hello negotiation whose `operation_catalogue_sha256` equals the exact `source-handoff-v1` catalogue fingerprint. Bare protocol 4 refuses. T592 consumes the census read-only and changes only target-v5 adoption and target outputs. No external source artifact, normative spec, source test, or implementation is edited by this batch. |
| Historical, non-authorizing SC-002 cleanup prose required an identity-mismatched inode to be restored while also requiring both reserved namespaces to be empty on every return. | The fail-closed rule forbids unlinking an inode whose identity is not proven; restoration makes the universal empty-census terminal state unattainable, and Linux has no inode-qualified unlink that could safely remove a checked name later. | The temporary and cleanup-quarantine namespaces are legacy-observation-only, and current publication creates neither. A verified, direct-final copied and payload-file-synced payload reaches `parked` while every original legacy source name remains at its frozen locator. Every nonterminal census is inspectable as exactly `recovery-resumable` or `recovery-irreconcilable` under the same stable incident id and closed cause/remediation/exit projection. Recover is offered only for a unique exact continuation. Authenticated apply never mutates an existing name: it direct-final publishes immutable payload or residue evidence from retained fds and publishes `mismatch-retained` only against the exact frozen retained-name and residue censuses, or binds the complete recursively enumerated frozen primary-evidence census or identity-bearing bounded-failure commitment in a separate resolution branch when names, metadata, primary status, or census stability is irreconcilable. The scope binds every descendant and canonical failure-path digest and excludes resolution/request/disposition/freeze leaves; raw `01ff`, copied commitments, and post-resolution primary changes block admission. Every branch reaches fresh-successor admission without restoration, fabricated residue, primary-branch repair, or unlink. Import, cleanup, incident transitions, successor admission, and the retention guard share one exclusive candidate OFD lock; every live owner excludes cleanup before namespace access, successful cleanup additionally requires private `SidecarCleanupOwner`, and the sole retry recensors with fresh fds after release. Only a direct-final ordinary terminal requires both ephemeral namespaces empty; a terminal legacy incident instead retains every original source name at its frozen locator under the exact frozen recursive census. The typed receipt locator, shared SC-002 hash oracle, structured complete preimage/status/path contract, pre-signing successor request binding, payload and all-ancestor durability, resolution schema, exact receipt/census/source-floor/post-mutation/unit negative registries, and complete apply-peer registry still require the separately accepted Version 2 normative amendment; this feature-only batch records that escalation and does not edit it. |
| Earlier SC-002 retention prose allowed the private owner to tombstone and delete the whole candidate after terminal-state checks. | Delivery requests, panel records, evidence records, seals, and eligibility history are permanent delivery history; SC-020 requires each applicable wave seal to remain available, and incident evidence is never automatically removed. Deleting the candidate root would erase those retained proofs. | `CandidateRetentionOwner` is a zero-mutation whole-scope guard. Verified orphans remain in the separately owned bounded `evidence-sidecars/sc002/retired` subtree; incident evidence and every request/record/receipt/seal/eligibility/merge artifact remain under the canonical candidate root. No candidate descendant is automatically unlinked, and the root is never renamed, tombstoned, or deleted. Tests poison candidate-root removal and any permanent-history mutation. |
| `ADR-046-provider-device-tpm` section 11.2 names the finalizer `device-tpm/cleanup`. | Committed `packages/d2b-provider-device-tpm/src/lib.rs` and `packages/d2b-contracts/src/v3/device.rs` both expose `device-tpm.d2bus.org/state-preserved`. | Existing code is canon. The exact T604 fixture uses `device-tpm.d2bus.org/state-preserved` while retaining the dossier's stop/wait/delete-process/delete-flush/retain-Volume/clear sequencing. Correcting the upstream dossier is external to this feature-only batch. |
| Feature-local prose treated the W0/W1 record and W2-W4 late remediation as authority to continue despite Constitution Principle VI, while T072 omitted Wave 5's contemporaneous plan-panel predicate. | The committed constitution permits no artifact-local waiver for these gaps, and existing Wave 5 implementation does not prove its historical entry gate. | FR-036 now makes a separate accepted Principle VI constitution amendment that expressly dispositions the W2-W5 plan-panel gap an external prerequisite for every implementation, resume, fix, close, merge, and advance path. T072 requires the exact retained Wave 5 plan-panel receipt to check; no current receipt is claimed, and current remediation remains evidence only. |
| Feature-local Wave 5 recovery prose allowed repeated binding requests. | Current panels use the candidate-bound lifecycle selection artifact and require every selected seat; strict fixed-ten including `rust` is readable only for legacy records. Wave 5 already retains one strict legacy fixed-ten `panel-request.json`, and content invalidation does not erase it. | T220 uses repeatable selected-roster nonbinding plan lifecycles for convergence and creates no request. T219 may reach close only through accepted external disposition of the retained request and the strict legacy fixed-ten recovery-attestation set that request requires. No current lifecycle may drop selected seats or reinterpret the legacy record. |
| The accepted external `ADR-046-validation-and-delivery` contract remains Version 1 and therefore does not yet own the complete Wave 5 incident/source-floor byte contract. | Feature-local planning artifacts can specify the required shape but cannot amend the accepted external normative contract, generated manifests, schemas, source, or tests. | External scope escalation: Version 2 must pin the thirteen-line cause/remediation projection; distinct resumable/irreconcilable variants; inspect/request/apply/successor convergence for `evidence-census-conflict`; a pre-signing durable successor freeze and canonical authority request whose exact triplet persists through apply/admit; structured durable incident preimages containing every kind-specific component and repeated across all status/path records; recursively enumerated frozen primary-evidence and retention scopes with identity-bearing bounded-failure replay and raw `01ff` non-authorizing; typed receipt locator plus the nineteen-digest SC-002 hash golden; payload-file plus all-ancestor durability; one-lock cleanup exclusion plus private `SidecarCleanupOwner`; residue-backed `mismatch-retained`; one non-clonable protected source-floor origin consumed through authenticated issuer provenance into one private validated-floor result with copied-digest, replay, repeated-mint, and serialized-revalidation rejection; exact receipt, malformed-census, source-floor 32/26/21, 15-case post-mutation, and unit negative registries; and the complete fifteen-row apply-peer registry. Its approvals, regenerated manifests, Gate 0 receipt, and ancestor binding remain pre-T589 prerequisites. Feature `spec.md`, `data-model.md`, `quickstart.md`, `contracts/README.md`, and `tasks.md` now agree on that planned requirement, but do not supply external acceptance or implementation authority. |
| Earlier feature planning left the disposition request as a prose prefix transform, retained a second flat primary-census grammar, allowed partial bounded-failure traversal authority, did not tie cleanup authority to the OFD guard lifetime, and described request output as merely create-exclusive. | The accepted external Version 1 contract, source, tests, schemas, normative/reference docs, and panel artifacts remain unchanged and cannot be silently amended by this feature-only batch. | External scope escalation: Version 2 must add the exact 19-field request and 19-to-22 transformation; successor-freeze and request-digest continuity; one recursive absent-root/directory/regular-file grammar; full-descendant coverage or admission denial with hard work ceilings; `CandidateSidecarGuard` plus `SidecarCleanupOwner<'guard>` lifetime/API seals; all-descriptor CLOEXEC and exec-leak proof; anchored openat2, deterministic temp, file-sync, no-replace, final-inode verification, parent-sync, and exact replay for `--request-out`; every cause and handoff recovery-pending/irreconcilable inspect/action/status/successor row; the literal 15-edge, 90-case, and 91-case matrices; the 15 pre-start/root and 27 unit cases; 56 census and 25 output ids; and the thirteen-row SC-002 redaction registry. External schemas, reference/status prose, tests, source, Nix, changelog, contributor guidance, and panel artifacts must move only in their owning workflow before T589. |
| Round 22 found that incomplete/hard-ceiling SC-002 scans still projected an unusable signing action; named partial preimage/request temporaries could poison replay; recursive node encoding did not injectively represent source-slot roots, symlinks, or devices; handoff status remained a constructible tuple with no terminal-pointer rule or failed-transfer partition; and raw `st_uid`/`st_gid` lacked canaries. | No committed source/test/schema or accepted external Version 1 normative artifact implements the planned correction, so feature prose cannot claim it does. The daemon-only three-unit architecture, broker-only recovery, no-unlink evidence rule, and existing 605 task ids remain binding. | The Round 22 batch added complete unnamed-inode write-ahead publication, six remediation values with null-evidence coverage repair, twelve root/root-instance pairs and total node encoding, full-sequence stable bounded failures, exact depth 64/65, its then-current 72 census and 26 output ids, fifteen recovery canaries, payload-sync-before-status ordering under the candidate lock, and a closed handoff variant/current-pointer/error contract split across T589/T595/T604. The concrete failures were a linked partial record authorizing replay, a symlink/device aliasing absence, a failed transfer projecting wait forever, or incomplete rollback being labeled recoverable. External Version 2, source, tests, schemas, normative/reference docs, ADRs, constitution, contributor guidance, changelog, and panel artifacts remained escalated to their owners and unedited by that batch. |
| Round 23 found that `AT_EMPTY_PATH` requires capability unavailable to the privilege-dropped target; request-output support refusal conflicted with candidate-first durability; replaceable names weakened cleanup/publication identity; unavailable nodes could enter a serialized bounded body; source-floor bytes could mint repeated capabilities; coverage and coordinator recovery labels advertised unrealizable actions; handoff, redaction, and remediation matrices were incomplete. | No committed source/test/schema or accepted external Version 1 normative artifact implements these corrections. The daemon-only three-unit architecture, broker-only recovery, no-unlink rule, 605 task ids, and all external non-authorizing dispositions remain binding. | Round 23 planning, retained only as non-authorizing history, used zero-effective-capability procfs-fd exact-inode direct-final linking with no `AT_EMPTY_PATH`, linked temporary, or create-and-unlink probe; preserved request candidate-first durability and treated unsupported link as retained-internal output failure; required a guarded namespace write owner for existing-name cleanup moves; rejected serialized unavailable nodes; consumed one protected source-floor origin into one validated capability with borrow-only attenuation; exposed bounded coverage failure/root classes with exact owner procedures and no signing command until repair; defined selector-free coordinator pointer repair plus immutable-audit escalation; and pinned its then-current 61/73/26 SC-002 registries, seventeen recovery canaries, all six remediation rows, and exact 135-case handoff registry over seven rollback members, 30 audit members, and 15 transitions. External Version 2, source, tests, schemas, normative/reference docs, ADRs, constitution, contributor guidance, changelog, and panel artifacts remained escalated to their owners and unedited by that batch. |
| Round 24 found that live importer/preimage/request publication still conflicted with direct-final rules; cleanup relied on same-uid-cooperative name moves; source-floor claims could be consumed before fallible validation; pointer repair lacked restart, privilege, audit, and backup contracts; and live registry counts omitted importer and repair cases. | Committed source, tests, accepted external Version 1 artifacts, external blockers, and non-authorizing dispositions remain unchanged. The daemon-only three-unit architecture, broker-only privileged mutation, fail-closed defaults, and exact 605 task ids remain binding. | Current planning uses unnamed-inode zero-capability procfs-fd direct-final publication for every new SC-002 immutable record, treats named cleanup state as legacy-only, performs no existing-name rename/unlink, and pins 35 publication cases including importer support/crash/replay and post-link final-reopen mismatch. Source-origin consumption commits only with durable dispatch and is reacquirable after pre-publication owner death under one anchored stable OFD-lock inode. T592 owns Admin-only typed pointer repair/restoration broker ops, coordinator lock, 32 closed handoff audit members plus a separate two-edge restoration audit fixture, immutable pre/outcome audit, and bounded authenticated backup; T595 owns repair and restoration clients. Exact-empty clean absence, separately projected repairable absence, invalid competing/malformed/unauthenticated censuses, every crash boundary, conflict, second-run no-write, forbidden inputs, bounded restoration output, and separate integrity escalation expand the handoff registry to 156 cases while the independent restoration broker registry has 62 cases and transition edges remain 15. External Version 2 and every other external owner/disposition remain escalated and unmodified. |

The Round 24 row is preserved verbatim as dated history; its phrase "Current planning" and
62-case restoration value mean current at that round and are non-authorizing now. The active
R26 values are 156 handoff cases, 35 SC-002 publication cases, and 145 restoration broker
cases.

R27 preserves that historical R26 census and expands the active restoration broker registry
to 168 literal ids: 18 independent prune-pre/prune-outcome publication-boundary ids, restart
reservation reconstruction, immutable-zero-mutation reservation release, and three
prune/reservation shrinkage poisons. Active plan, task, contract, and checklist references use
168; the dated R26 row remains historical.

R30 leaves those 168 literal ids unchanged and narrows their active claim to the cases they
actually enumerate. Active plan, task, contract, quickstart, coverage, and completion
checklist references additionally require the read-independent 216-id durable-record/
boundary and 88-id lifecycle registries with the responsibilities pinned in
`data-model.md`; neither supplemental registry may be replaced by the 168-id broker or
156-id status registry. Task ids remain exactly 605, and every external blocker and
disposition above remains unchanged.

### Recorded drift

`ADR-046-validation-and-delivery` §3.2 lists `packages/d2b-process/` and
`packages/d2b-provider-supervisor/` under W2. No W2 work item targets either path; the owning
item `ADR046-process-001` is W4. Per "existing code is canon" the machine-readable graph wins.
This plan follows the graph. Correcting the prose is a specification amendment that re-opens
that spec's evidence, so it is raised to the integrator rather than fixed mid-wave.

`ADR-046-validation-and-delivery` section 12.3 and the repository panel guidance agree that
the binding delivery panel runs exactly once per wave. Earlier feature prose incorrectly
placed scoped fix and follow-up rounds after a binding request and treated each successor
candidate as eligible for another request. This plan keeps iterative process review on the
nonbinding `/d2b-panel-round plan` phase surface, which creates neither a delivery
`panel-request.json` nor a binding reservation. The retained pre-amendment Wave 5 request has
already consumed the delivery surface; T219 cannot reserve it again for final F. An accepted
external disposition must reconcile those facts without deleting or reclassifying history.
T589's wave-scoped reservation work cannot govern W2-W4 retroactively.
The versioned feature-local
`contracts/README.md#candidate-recovery-prerequisite-v1` therefore made T008 the intended W2
entry owner. The committed history now contains downstream W2, W3, and W4 implementation while
T008, T030, and T037 remain unchecked, and the delivery state reports all three waves already
sealed and merged. That is historical drift, not authority to schedule replacement closes.
T008, T030, and T037 remain historical entry attestations: only exact retained evidence from
the actual first-dispatch base may check them. T028/T029, T035/T036, and T070/T071 are now
historical close verification/adjudication only. They require exact external delivery-record
confirmation binding the actual candidate, binding panel, seal, and merge, or an accepted
external correction. They do not freeze a new candidate, issue or rerun a binding panel,
attest, reseal, remerge, or claim a new close. A current rerun never masquerades as historical
compliance. One canonical hermetic `candidate_recovery_v1` implementation owns
both receipt variants and all five sequencing invariants. Its asserted table-driven inventory
independently mutates every receipt field and delivery binding; request/candidate/program/wave/
round/commit/tree/recommendation/convergence/validation identity; exclusivity condition; and
post-request content/history/evidence movement. T029, T036, and T071 may invoke that validator
only to adjudicate retained records or an accepted correction, never to create a new close. A
local predicate, happy-path-only test, missing case, ignored/empty discovery, or different
acceptance between waves leaves historical confirmation open. T589 consumes accepted v1 on its own actual base and adds the stricter
wave-scoped `adr046w5` storage profile;
it does not close T008 retroactively. This batch records the external scope escalation in
`friction-log.md` but does not edit the external ADR, ADR index, normative specification,
delivery source, `AGENTS.md`, contributor guidance, or panel tooling.

The feature-local census also drifted after initial task generation. The initial program
scope remains 531 primary work-item tasks, but the authoritative manifest at
`868469bf9c293cd48fff483717f14cb88c246821` records 68 `Merged` and 477 `Planned`.
The regenerated feature census checks the 54 newly merged primary tasks, leaves all 477
planned primary tasks open, moves T039 with `ADR046-process-002` to its authoritative W6
group, and treats completed process tasks that merely cite work-item ids as non-primary.

`ADR-046-telemetry-audit-and-support` work item `ADR046-reuse-005` required the
`observability-otel` Provider to emit authoritative `SessionConnect` records "via `d2b-audit`".
That obligation is not dischargeable from a Provider crate and contradicts two committed,
passing surfaces. `ALLOWED_WORKSPACE_DEPS` in
`packages/d2b-contract-tests/tests/policy_provider_crates.rs` admits only `d2b-contracts`,
`d2b-controller-toolkit`, `d2b-core`, `d2b-process-conformance`, `d2b-provider` and
`d2b-provider-toolkit`, and `packages/d2b-provider-toolkit/src/audit.rs` states in code that the
Provider agent ring is "diagnostic, never the authority for what happened". The authoritative
writer already exists at `packages/d2b-session/src/audit.rs` and belongs to `ADR046-audit-003`.
Per "existing code is canon" the code wins, and per the ruling class established in
`implementation-debt.md` §16.1 this is a manifest defect rather than permission for a
slice-local workaround. The correction was authored in the member spec and regenerated:
the Provider is the subject of the record, not its author; the crate takes no `d2b-audit` and
no direct `d2b-telemetry` dependency; the closed `METRIC_LABEL_POLICY` data is single-sourced
in `d2b-contracts` and re-exported by both sides. `ADR046-telem-006`'s validation phrase was
corrected in the same pass to distinguish the table-driven four-variant test of the one shared
ingress gate, which is owed now, from live `otlp_unix` / `otlp_vsock` / `import_stream`
adapters, which that item's own Removal proof sequences after the OTLP exporter.

The W5 removal-proof inventory originally grouped eleven Rust crates together. The
implementation graph and live dependency tree permit only three removals in W5:
`d2b-daemon-access`, `d2b-host-providers` with its sole
`d2b-host::runtime_provider` consumer, and the already-retired `d2b-userd` stub. The realm
session crates are retained for the W7 Provider-session migration, `d2b-provider-aca` and
`d2b-provider-relay` are W6 Provider surfaces, `d2b-unsafe-local-helper` is reused by W7, and
`d2b-guestd` is the live guest-control service rather than a legacy stub. The work-item wave
ownership and committed runtime wiring force this boundary; deleting the later-wave surfaces
in W5 would remove their only current implementations.

ADR 0051 Amendment F describes the audio dossier's `implementationEndpointRefs` as appearing
only in YAML examples and prose. At the c62e57ce integration tip, the committed
`AudioService.spec` table already has a typed `Type` column for that field and every other
base field. Per "existing code is canon", this wave leaves the already-correct audio table
unchanged and adds only the missing typed Service tables to the security-key and USBIP
dossiers.

ADR 0051 Amendment E names two provider admission variants that are not present in the
current `d2b_core::error::Kind` catalog. This docs-only scope cannot add Rust error records,
and `gen-error-codes` owns the auto-generated table, so the two normative rows are recorded
in a separate provider projection admission table rather than hand-editing generated output.
The generated runtime catalog remains unchanged and its drift gate stays authoritative. The
corresponding `ProviderContractError` variants and the protocol-version field are also absent
from the current Rust implementation; those implementation changes belong to the downstream
ADR 0051 consumer slice.

Amendment A is likewise intentionally ahead of the current implementation: the committed
`semantic_services/security_key.rs` still models `allowed_backing_ref_types: None` and
`BackingRefTypesUndetermined`, while `ProjectionFactory::new` still rejects an empty backing
set. This docs-only slice records the accepted deny-all contract and leaves that Rust
implementation drift for the consumer slice rather than editing out-of-scope code.

ADR 0051 Amendment G is outside this W5 request, which consumes amendments A-F and H.
`ADR-046-nix-configuration.md` is therefore deliberately left unchanged.

`ADR046-zone-control-001` authorizes removing the legacy `Realm` model but does not make the
whole `d2b-realm-core` crate mechanically replaceable. In particular,
`d2b-contracts::v3::resource_status::ResourceUpdateStatus` still uses the realm-core string
`OperationId`, while the v3 ComponentSession contract owns a distinct fixed-width, redacted
`OperationId`. Choosing the v3 status wire representation is an architectural decision and is
not inferred by a removal slice. This blocks eventual realm-core retirement, not the three
W5-owned stub removals above.

The SPIKE-01 RSS rerun amendment landed against
[`amendment-spike-01-rerun.md`](./amendment-spike-01-rerun.md) and produced three drift
records worth carrying forward.

The draft prescribes pinning the policy fingerprint to the wording
`6,148 KiB below 24,576 KiB`. The committed result artifact,
`proofs/redb-resource-store-spike/RESULTS-rerun-2026-08-02.md`, does not emit that phrase; its
canonical measurement cell reads ``Median `18,428 KiB`, `6,148 KiB` below the threshold``. Per
"existing code is canon" the artifact wins, and the artifact is out of scope for this change.
The lint therefore pins the artifact's actual wording as the canonical fingerprint and pins the
draft's phrasing as the *derived* prose the specifications restate, with the `24,576 KiB` value
bound separately through the row's threshold key. Both numbers and the gate are still pinned;
only the sentence that carries them differs from the draft.

The draft's verbatim section 3.2 replacement text says "remain W5 implementation work". The surrounding
table uses the `ADR046-W<n>` namespace throughout, so the applied text says `ADR046-W5`. This is
a preserved legacy member-spec namespace normalization, not a current delivery address or a
scope change.

`plan.md`'s **Constraints** line formerly read "currently MEASURED-FAIL at 25,216 KiB". That
figure was superseded first by the corrected disposable proof and then by production-fixture
measurements. This approved amendment updates the line without rewriting either historical
result: the unchanged constraint is `<=24,576 KiB` with no baseline subtraction, the prior
results remain bound to their recorded commits, and the completed publication path is
unmeasured until T601 runs against final candidate F frozen by T220.

Eleven W5 work items and two already-`Merged` items name destination crates that do not
exist, where a committed crate covers the same obligation under a different name. The full
adjudication is [`amendment-w5-destination-drift.md`](./amendment-w5-destination-drift.md);
the summary is that FR-046 does not decide this class, because both sides of the
disagreement are the generated manifest rather than prose against a manifest.
`ADR046-exec-016` names `packages/d2b-bus-session/` while `ADR046-session-001`, `Merged` in
W1, names `packages/d2b-session/`, and both are rows in `ADR-046-work-items.json`. "Existing
code is canon" decides it, and the committed crate is the destination:
`d2b-session`/`d2b-session-unix`, `d2b-bus`, `d2b-zone-routing`, `d2b-resource-client`,
`d2b-resource-api`, and `d2b-provider`. The session-crate pair is not drift at all - the
member spec's own text says "rename crate ... or retain name". Two obligations are genuinely
absent rather than relocated, `ProcessAttachClient` and
`nixos-modules/options-volumes.nix`, and both stay outstanding against their `Planned`
items. No item is marked complete on file presence.

The three W5 crate removals now carry real FR-023 proofs in
[`removal-proof-w5.md`](./removal-proof-w5.md), replacing the rationale paragraph above as
the evidence of record. Two migration-map rows moved to W5 under FR-060, one row was retired
as naming a path that does not exist at the map's own baseline, and the
`d2b-daemon-access` ADAPT disposition is raised as drift rather than corrected, because the
migration map is a member specification.

### Gate status

Gate 0 has been re-evaluated a second time under FR-056; the record is
[`gate0-reevaluation-spike-01-rss-rerun.md`](./gate0-reevaluation-spike-01-rss-rerun.md). The
mechanical half is discharged: four member digests moved, seven work-item strings changed, and
`ADR-046-implementation-graph.md` is byte-identical, so the specification-to-work-item bijection
is provably untouched.

The human-review half is **not** empty this time. The legacy `ADR046-W5` delivery state holds an
outstanding strict legacy fixed-ten panel request with imported validation evidence - including a
`redb-rss-spike-observation` record - gathered before the amendment. FR-056 requires that
evidence not to authorize amended bytes. The request is nevertheless a retained binding
delivery request and consumed Wave 5's once-per-wave slot; it is not deleted or silently
reclassified as a phase-plan round. T219 remains non-authorizing until an accepted external
delivery-contract/tooling disposition reconciles the consumed request, amended candidate,
and historical record. Until then there is no new binding request, attestation, seal,
merge-target registration, merge eligibility, or merge.

Delivery state reports W2-W4 sealed and merged. The open task rows for those waves are
historical verification/adjudication only: each requires exact external delivery-record
confirmation or an accepted external correction and cannot schedule a replacement binding
panel, seal, or merge. This feature batch claims no new W2-W4 close.

Waves W6 through W8 are unaffected. `ADR046-store-002`, `ADR046-store-004`, `ADR046-store-005`,
and `ADR046-reconcile-003` remain `Planned` in W5: the passing rerun measures a disposable proof
crate, not `packages/d2b-resource-store-redb`, and supplies none of those items' production
evidence.

That caveat remains load bearing rather than ceremonial, and it became more so once the production
backend landed. `packages/d2b-resource-store-redb/src/{actor,transaction,revision_log,backup}.rs`
were added by `0a080828` on 2026-07-31, 349 commits before this amendment's base `c3e15b66`.
The code exists, and the production measurement gap described above was closed for the
exercised hard fixtures at their recorded snapshot. The then-current-tip reruns recorded in
`proofs/redb-resource-store-spike/RESULTS-production-2026-08-03.md` and
`proofs/redb-resource-store-spike/RESULTS-production-watch-2026-08-03.md` were run at HEAD
`da9295e7ff370b22cdd6c413e8d82b33936f285e` through the public heavy-gate slot. The backend-only
fixture passed with RSS values 18,784 / 18,788 / 19,160 KiB and a 18,788 KiB median against the
24,576 KiB threshold; the production watch fixture passed with RSS values 20,724 / 20,788 /
21,072 KiB and a 20,788 KiB median against the same threshold. The watch harness exercises the
production `WatchService`, authenticated `ZoneBus`, and controller fan-in in-process, not test
no-op ports.

Those results corrected the claim that production RSS had never been measured. They are now
historical because the approved publication path changes the measured owner graph. They do not
discharge the broader authenticated publication, restart, audit, disconnect, fan-in, and
compaction matrices. All four items therefore stay `Planned` in the manifest until their complete
validation obligations are met; T601 must measure the amended candidate and T602 rejects either
2026-08-03 artifact as current evidence.

The backend commit predates the amendment base, so it is outside this focused Gate 0 review and
is not reverted or re-adjudicated here.

## Complexity Tracking

### Constitution Principle VI disposition

| Recorded violation | Why It Cannot Be Reconstructed | Required Disposition |
| --- | --- | --- |
| W0 and W1 delivered without the panel receipts and seals that Principle VI requires; W2-W5 later received implementation while contemporaneous plan-panel evidence remained unproven | The historical W0/W1 candidate snapshots do not exist in one canonical form, and a current W2-W5 panel cannot become evidence about an earlier dispatch boundary. Retroactive attestation would be false evidence. | Keep the feature-local records as evidence only. Before any further implementation, resume, fix, work-panel, seal, merge, or advance, land a separate Principle VI constitution amendment that expressly dispositions both gaps, including Wave 5, and states the continuation conditions. Its commit must be an ancestor of the exact execution base. |

No feature-local constitution deviation is accepted. W0/W1 and W2-W5 are different evidence
classes, but both require the external FR-036 constitutional disposition before the program
can continue. Their historical tasks still require exact retained evidence; absent that
evidence, current remedial records preserve the present gate without rewriting history.
After the external amendment lands, Wave 5 may resume only after the T072 disposition, one clean-base analysis, and one current selected-roster plan lifecycle. T603 then routes the exact all-satisfied checkbox transition through `/d2b-spec-edit`; the editor receipt and dedicated Git commit are the only authority. Fresh analysis and a new selected-roster lifecycle on the changed snapshot gate T589. C1 remains fully assigned to T605 and creates no exception.

### Program-local safety
### Program-local safety and delivery risks

These rows are not Constitution Principle VI deviations.

| Risk | Why Tracked | Guard and Rejected Alternative |
| --- | --- | --- |
| FR-043 (recovery-point attestation) is tracked program-local, outside the work-item manifest, so the manifest census alone cannot enforce it | FR-043 is locally added and **stricter** than `ADR-046-reset-and-cutover`, which permits proceeding past the rollback boundary without attestation. Creating a manifest work item would require amending that member spec, which re-opens its validation and panel evidence and re-triggers Gate 0. An unqualified "backup exists" assertion permits a partial, old, wrong-host, or unverifiable point to become success-shaped. | Keep it program-local, but close the safety gap at the W7 exit boundary. T548 owns one hermetic validator used unchanged by T580, T555, and T556. It decodes every timestamp through a bounded integer newtype, uses checked bounded expiration arithmetic, requires `previewed <= captured <= verified <= attested <= verifier-now < expires`, independently varies every receipt field and binding including operator and restore-instruction digests, and fails on listing failure, empty discovery, ignored tests, or skip. Before T580 records evidence, the integrator freezes the clean current W7 candidate and exact preview inventory. T580 accepts only one external version 1 record for a verified full-host snapshot or backup covering boot/system state, the active generation, the preview inventory, and preserved identity state. It binds candidate/commit/tree, preview, daily-driver host, operator, and restore instructions; imports only its digest and opaque locator through the existing `EvidenceRecord`; and rejects negative, fractional, future, out-of-range, overflow, stale, expired, or mismatched values. Every close stage invokes the same validator. Expiry before the binding request returns to prebinding convergence and requires fresh evidence plus another nonbinding phase review. Expiry after the wave's one binding request durably fails the close, retains its records, permits no successor request, and requires integrator scope escalation. The external operator-owned backup/snapshot and restore mechanism remains outside this feature; no host implementation is claimed. |
| Historical pipeline records allowed successor work after 5 of 10 legacy panel returns. | Current delivery tooling instead requires the predecessor selected-roster lifecycle, seal, and merge before successor implementation starts. Fixed-ten including `rust` is legacy data only. | Do not dispatch a successor from a partial legacy count. The prior merged seal is the mechanical guard; strict sequential delivery is current authority. |
