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
consumed its once-per-wave request, one binding ten-role `/d2b-panel-round work` delivery
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
in the same Wave 5 PR. The plan is now eligible for a pre-T603 read-only cross-artifact
analysis and, if no HIGH or CRITICAL finding remains, a unanimous plan panel at clean base A
and feature snapshot P0. Those gates authorize only T603's validator implementation. T603 then
lands its dedicated validator commit V, freezes resume base B exactly at V, and reruns both
analysis and the plan panel at B/P before it may create a reconciliation receipt or authorize
the checkbox transition. Because dedicated checkbox commit C changes the reviewed feature
snapshot from P to Q, the B/P sign-off does not authorize T589. T589 remains blocked until
the receipt-bound progress reconciliation passes, a fresh cross-artifact analysis plus
unanimous plan review bind exact clean C/Q, and the externally accepted compatibility floor
has been installed in the source 3/1 generation.

**Current Wave 5 resume state:** at this amendment batch's committed input HEAD
`67f0ba8e32c4f91ebfcb4038aff77821d42b64b1`, the feature root is pre-T603 A/P0.
None of the 147 T603-authorized checkbox changes has occurred, so C/Q and the finalized
progress-editor receipt are future artifacts and are not claimed or required by the current
A/P0 analysis and plan-panel gate. That current gate requires the stated external
prerequisites and T072 disposition and authorizes only T603's validator-and-fragment commit
V. C/Q becomes a valid state only after the future V/B implementation, B/P gates,
reconciliation authorization, authorized editor checkbox transition, dedicated commit C,
and progress-receipt finalization. Only then does the fresh C/Q analysis and plan panel become
the T589 dispatch gate.

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
`SourceGenerationIdentityV1`, strict repository schemas, and checked-in golden vectors.
The compatibility producer/installer and import/validation authority implement and install
that accepted contract but do not own or redefine those repository artifacts. Both external
prerequisites must agree byte-for-byte before T589 dispatch.
caller-flake target entrypoint
remains unprivileged. Numeric protocol 4 without that exact negotiated fingerprint is the
bare committed protocol and refuses. T589 and all downstream Wave 5 implementation remain
blocked until an accepted external disposition lands, is installed in the source 3/1
generation, and proves the exact actor contract stated in FR-070. A target-only binary,
synthetic starting image, or prose compatibility claim is not that disposition.

## Technical Context

**Language/Version**: Rust 1.94.1 (pinned via `packages/rust-toolchain.toml`, components
`rustfmt` and `clippy`); Nix for the NixOS module surface

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

SC-002's 2,000 ms operator envelope is a separate end-to-end ceiling, not the sum of or a
replacement for those component budgets. Its monotonic clock starts when the public deployment
entrypoint durably commits the target-generation transition intent, before broker publication
or daemon ingestion, and stops at the later of real-effect observation and the production
operator projection reporting the new resource `Ready`. It includes handoff, automatic
activation ingestion, durable commit, controller dispatch, broker effect, status persistence,
and watch/read-model projection. Nix evaluation/build and pre-intent profile staging are
outside the clock. Every applicable component p95 and every qualifying SC-002 sample must pass
independently.

T604 owns SC-002 measurement collection in its Type-10 acceptance leg without claiming
implementation of the measured production path. It emits one separately versioned typed
`Sc002ActivationReceiptV1` defined in `data-model.md`, never free-form evidence. The
schema-v2 `EvidenceRecord` remains unchanged and references the receipt through its existing
`locator` as the candidate-relative content address
`evidence-sidecars/sc002/sha256/<digest>.json`. The receipt repeats the record's actual
`candidate_id`/`content_id`/`snapshot_sha256` triplet. Every stage resolves the locator
beneath the held candidate dirfd, verifies the exact byte digest before decode from the same
fd, and refuses replacement, traversal, absolute, URL, symlink, or hard-link inputs. The
receipt is schema version 1, at most 16,384 encoded bytes,
fixed-redacted under `Debug`, uses one `CLOCK_MONOTONIC` start tick, and contains the exact
three-sample census
keyed by `Volume/acceptance-state`, `Network/acceptance-net`, and
`Device/acceptance-tpm`. Each sample carries effect, production `Ready`, selected-stop, and
bounded progress observations whose repeated resource identity must equal the sample key.
Effect and Ready must name the same typed resource identity as the sample. The selected stop is the later effect/Ready tick, elapsed nanoseconds are its checked
difference from start and at most 2,000,000,000, and 1-32 production progress observations
fall strictly after start and no later than stop. T589 owns the type plus one validator used
unchanged at evidence import, durable reopen, panel-request/panel-attest, seal, and
merge-eligibility. A failed operator `EvidenceRecord` imports without a receipt but cannot
satisfy any close stage; only a passing record must resolve exactly one matching receipt.
T604 emits the receipt only as an external current-effective-uid `0600` validation output.
T600 imports the exact-F result only in `operator-nix-activation-cleanup` and supplies that
output through the T589-owned explicit `--sc002-receipt PATH` input; it may not supply
`--locator`. T589 hashes and validates the once-opened source before deriving the locator,
then installs the exact bytes as a current-effective-uid `0600` leaf beneath current-effective-
uid `0700` candidate directories using held dirfds, a create-exclusive temp, file `fsync`,
and `renameat2(RENAME_NOREPLACE)`. Before publishing the `EvidenceRecord`, it `fsync`s every
ancestor directory from `sha256` through `sc002`, `evidence-sidecars`, and the candidate
directory. Creation, publication, loser cleanup, and restart cleanup all acquire the same
verified candidate-scoped exclusive OFD write lock, held through parent `fsync`, the
applicable census, and record publication or return. There is no second cleanup lock or
lock-free path. A live importer retains that lock, so cleanup cannot inspect, rename, or
remove its temp; restart cleanup proceeds only after acquiring the released lock. Under the lock,
cleanup moves the opened temp with `renameat2(RENAME_NOREPLACE)` into a reserved quarantine
name, reopens it, requires the same device/inode plus owner/mode/link/digest identity, then
never calls `unlinkat` on a sidecar data leaf. An identity-preserving orphan moves
no-replace into
`evidence-sidecars/sc002/retired/sha256/<content-digest>/<retirement-id>.bin`, where the
domain-separated retirement id binds the candidate triplet, content digest, and recorded
device/inode without rendering raw inode identity. Cleanup reopens and revalidates the
retired leaf and syncs the leaf plus both directories. Two identical orphan leaves with
distinct inodes receive distinct names. A destination `EEXIST`, a 65th retired leaf, more
than 1,048,576 retired bytes, or an invalid retired census routes the source to the incident
transition instead of overwriting, reusing, deleting, or growing the set. An identity
mismatch never restores the temp. It first durably publishes immutable preimage-complete
metadata, moves the metadata-bound currently named suspect no-replace into
`evidence-sidecars/sc002/incidents/payload/sha256/<incident-digest>.bin`, syncs the old
parent, payload parent, and every changed ancestor, reopens and verifies the moved inode, and
then append-only publishes and syncs `parked` status. Only that fully verified quarantine is
terminal. A replacement, `ENOENT`, nonidentical `EEXIST`, or post-move identity mismatch
remains recovery-pending, preserves every name, publishes no parked status, and blocks record
publication and every close stage until restart completes the same protocol. Ordinary
winners/losers/refusals and terminal incidents leave both ephemeral namespaces empty;
recovery-pending is not reported as terminal.
T589's private `CandidateRetentionOwner` is a zero-mutation whole-scope retention guard. With
the shared lock held, it proves the candidate is terminal, every delivery transition is
terminal, every incident is absent or `successor-admitted`, every retained external
reference remains resolvable, both ephemeral namespaces are empty, the exact durable census
is valid and bounded, and the canonical candidate root plus all request, panel-record,
evidence-record, receipt, seal, eligibility, merge, incident, disposition, and status
history remain immutable. Verified orphans remain in the separately owned bounded
`evidence-sidecars/sc002/retired` subtree. No candidate descendant is automatically unlinked,
and the candidate root is never renamed, tombstoned, or deleted.
Crash recovery may reuse an
identical already-durable sidecar after a full reopen check but never replaces it; mismatched
bytes or binding refuse. T602 and T219
reopen it through that validator. Unknown fields or enum values, missing/duplicate/unrelated
samples, effect/Ready identity disagreement, mixed selected-stop/progress identities,
malformed or misordered ticks, a stale or wrong-triplet outer binding, progress-free evidence,
retirement collision/census failure, unauthorized retention cleanup, or an over-budget sample
blocks the
close. Remediation belongs to the existing production-path owner, must obey FR-030, and
requires a successor candidate plus rerun of T600-T602; T604 may not weaken the threshold,
add a sleep/timeout or exclusion, or claim the implementation.

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
parallel group, each in its own worktree; 10 read-only panel lanes on
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
|   |-- requirements.md  # Spec quality checklist (16/16 passing)
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
| W5 (`adr046w5` delivery address) | 7 | 146 + 17 local completion/resume tasks | 12 manifest groups + the serialized completion graph below | FR-036 external constitution amendment must disposition the unproven Wave 5 historical plan panel first; T072 disposition and fresh descendant-base A/P0 phase-plan review then gate T603, B/P phase-plan re-review follows V, and T220 may iterate nonbinding phase-plan fixes before final F. The retained Wave 5 binding delivery request consumed the once-per-wave request; T219 remains non-authorizing until accepted external disposition and cannot itself issue another. |
| W6 | 27 | 258 | 5 file-disjoint families | FR-036 external constitution amendment first; T221 plan panel before implementation; T479 requires exact-F6 `Provider/runtime-cloud-hypervisor` Guest acceptance; T480 revalidates it before close |
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
for each of the ten roles, all bound to the reviewed base, feature snapshot, and stated
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
disposition permits T603 until that external amendment is an ancestor of A. T603's unanimous
A/P0 `/d2b-panel-round plan` phase review is the plan gate for resumed Wave 5 implementation
only after that external prerequisite; it is not historical evidence and creates no delivery
request or reservation. V changes the implementation base, so B/P receives the required
nonbinding re-review.
Dedicated checkbox commit C then changes the reviewed feature snapshot from P to Q. That
content change invalidates B/P plan sign-off for implementation dispatch even though B/P
remains the authorization for the exact editor transition. A fresh analysis and unanimous
plan review bound to clean C/Q are therefore the final pre-T589 plan gate.
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
   cases and is independent of production enumeration. The prior mutation remains committed
   and audited; the refused edge and
   every successor have zero mutation count. The pidfd and executable fds are closed with the
   connection and are never serialized or persisted. Raw apply-peer PID/start identity and
   raw executable store path/NAR identity are internal comparison inputs only. Human, JSON,
   wire, error, log, span, audit, metric, and `Debug` surfaces contain none of them; where
   correlation is required they carry only typed fixed domain-separated digests, and metrics
   omit identity labels. Distinct PID, start, store-path, NAR, and executable canaries prove
   the prohibition.
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
   cross-disposition members refuse. The installed source receiver derives
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
An independent literal expected-set fixture owns the ids and cardinality. Unknown, duplicate,
missing, reordered, or dynamically skipped edges fail before any evidence is accepted.

#### Serial dependencies and file ownership

| Stage | Task(s) | Ownership and concurrency |
| --- | --- | --- |
| External Version 2 delivery-contract amendment | no feature task | Before T589 dispatch, a separate external specification-amendment workflow bumps accepted `ADR-046-validation-and-delivery` from Version 1 to Version 2. It normatively owns both contracts that T589 consumes: (1) the incident commands, exact twelve-line human projection, `Sc002IncidentDispositionV1` canonical encoding and Ed25519 authority/key/signature binding, immutable incident metadata and append-only status paths, file-and-ancestor-directory-durable publication/recovery protocol, separate durable-status and CLI-status schemas with deterministic remediation, the complete `CanonicalRetiredCensusV1` framing/tag/unavailable/sentinel/ordering contract and vectors, collision-safe retirement identity, private zero-mutation candidate-retention owner, and typed validator; and (2) the source-floor canonical JSON policy, `SourceGenerationIdentityV1`, complete digest/domain/length-framing and signature registry, unknown-field/tag/version refusal, strict schemas, and checked-in golden vectors. The amendment must receive the parent ADR's required pre-panel and post-panel approvals, regenerate `docs/specs/ADR-046-spec-set.json`, `ADR-046-work-items.json`, and `ADR-046-implementation-graph.{json,md}`, pass Gate 0 and drift validation on the exact amendment commit, and be an ancestor of T589's base. This row owns no feature artifact or implementation file. T589 cannot author, approve, regenerate, or substitute for it. |
| External source-floor prerequisite | no feature task | The accepted external compatibility disposition must name and deliver the `SourceGenerationCompatibilityFloorV1` producer/installer and typed import/validation authority. Its immutable four-record chain must reach `imported-for-exact-C/Q` for the exact source generation before T589 is ready. Those authorities produce, install, and validate objects against the already accepted Version 2 schemas and vectors; they do not own or redefine the repository encoding, digest, schema, or golden contract. This row owns no feature, source, test, Nix, normative, or changelog file and makes no implementation claim. |
| External retained-request disposition | no feature task | The external delivery-contract/tooling owner must land the contract and typed validator for `Wave5RetainedRequestDispositionV1`. T219 may consume its validated output only after F freezes. This row owns no repository file in this feature and cannot supply panel sign-off. |
| Resume reconciliation | T603 | FR-036's external constitution amendment is the first prerequisite and must be an ancestor of A. Exactly one valid T072 disposition is then required: exact historical attestation, or unchecked T072 plus one current `historical-entry-remediation-t072` record bound to A/P0 that claims no implementation completion. Fresh descendant-base analysis and plan panel at A/P0 then authorize exactly three repository paths: the two Rust files `packages/xtask/src/delivery/mod.rs` and `packages/xtask/src/delivery/resume.rs`, plus mandatory `changelog.d/delivery-resume-reconciliation.md`. T603 lands dedicated validator-and-fragment commit V with sole parent A and exactly those three paths; a missing or differently named fragment refuses. It freezes B exactly at V and P byte-identical to P0, revalidates the external prerequisite and T072 disposition, reruns analysis and the plan panel at B/P, then and only then writes the immutable authorization receipt at `.scratch/autopilot/adr046w5/reconciliation.json` and routes the sole receipt-bound checkbox transition through `/d2b-spec-edit`. It writes no feature prose; the Wave 5 integrator alone owns dedicated checkbox commit C. |
| Integrator prep | T589 | Its sole in-feature direct prerequisite is T603. Both external rows above are additional Wave 5 dispatch prerequisites. T589 does not start until accepted delivery-contract Version 2 and its approvals, regenerated manifests, Gate 0 receipt, and ancestor binding validate, and until the source-floor disposition has landed, its named producer/installer has atomically installed the source 3/1 floor, and its named typed import/validation authority has emitted the exact `SourceGenerationCompatibilityFloorV1` `imported-for-exact-C/Q` receipt for the migration generation. T589 only consumes both accepted external contracts; it authors neither. It checks the immutable source-floor receipt's dispatch binding and consumes it read-only; it is not its producer, installer, importer, or source-floor validator. Candidate-recovery v1 must be accepted on T589's actual base, but T008 is a separate historical W2 attestation and is not retroactively completed by T589. T589 remains blocked until the finalized editor progress receipt exists, T073-T218 and T603 are checked, HEAD is the clean dedicated checkbox commit, and both external prerequisites are satisfied. It lands the shared sealed capability, transactional audit-journal hook, mutation-response wire, and typed `InspectOperation` store/API/protobuf contract. It does not edit broker wire, StoreSync DTOs, their callers, broker protocol metadata, schemas, fingerprints, or snapshots. It also owns the `adr046w5` closed-evidence profile, the `--sc002-receipt` and incident command synopsis/catalog/help in `command.rs`, and wave-scoped strict state in `packages/xtask/src/delivery/{command,evidence,panel,seal,eligibility,history_proof,storage}.rs`: one fd-anchored, file-and-directory-durable, no-replace binding-request reservation for the program/wave; fixed redacted state/errors; fd-relative durable orphan retirement; a private `CandidateRetentionOwner`; exact final commit/tree/candidate/request/round binding; and the point-specific zero/zero-or-one/exactly-one crash oracle. In `evidence.rs`, it additionally owns SC-002 candidate-local ingestion. `wave validate-import --sc002-receipt PATH` is required only for the passing operator validation and rejects caller-supplied `--locator`; it once-opens a current-effective-uid `0600` single-link source, hashes before decode, derives `evidence-sidecars/sc002/sha256/<digest>.json`, validates the actual `candidate_id`/`content_id`/`snapshot_sha256` triplet, and durably publishes a current-effective-uid `0600` leaf beneath held current-effective-uid `0700` candidate dirfds with a create-exclusive temp, file `fsync`, `renameat2(RENAME_NOREPLACE)`, bottom-up `fsync` of every ancestor directory, and cleanup serialized by a verified candidate-scoped OFD lock. Every importer and cleanup worker acquires that same exclusive lock; no cleanup bypass exists. The live owner holds it through publication or verified quarantine/retirement cleanup, parent `fsync`, the applicable census, and `EvidenceRecord` publication or return; restart cleanup begins only after acquiring the released lock. Verified orphans retire under a candidate/content/inode-bound id into a 64-leaf/1,048,576-byte bounded census; two identical orphan leaves get distinct names, while destination collision or census failure transitions to incident without overwrite or unlink. Ordinary paths leave the ephemeral temp and cleanup-quarantine namespaces empty. An identity mismatch never unlinks or restores the suspect: it durably moves the currently named inode to `evidence-sidecars/sc002/incidents/sha256/<incident-digest>.bin`, or leaves an unmovable ambiguous name, and blocks publication and close with intentional incident residue. The private owner performs only a zero-mutation whole-scope retention guard after the exact terminal/reference/lock/census predicate passes; it preserves the canonical candidate root and all permanent delivery-history namespaces. Every reopen resolves beneath the held candidate dirfd and hashes before decode from the same fd. The accepted Version 2 contract's disposition validator authenticates canonical `Sc002IncidentDispositionV1` bytes against its pinned authority/key and consumes a private validated result by value. Same-candidate and alternate-candidate retries, reservation release, unrestricted successor admission, and every post-request move fail closed at panel, seal, and eligibility. Nonbinding `/d2b-panel-round plan` phase rounds create no reservation and may iterate before the final request. Crash injection covers every SC-002 source/hash/decode/temp/file-sync/no-replace/ancestor-directory-sync/lock/quarantine/retirement/incident/disposition/candidate-retention/cleanup/record boundary plus importer-cleanup and cleanup-cleanup overlap for same and different inputs, two identical orphans, forced retirement collision/census overflow, and replacement before quarantine, reopen, or retirement, as well as binding-request publication and terminal failed or successful disposition without enabling a second binding request. No implementation slice branches before this commit. |
| Serialized implementation | T590-T594, T605 | T590, T591, and T594 start together from T589. T592 starts only after T591 because both own `transaction.rs`; T593 starts after T592 and consumes its sealed broker peer-pidfd operation without another lockfile or dependency prerequisite; T605 starts after T593 and regenerates the shared API snapshots. T592 is the sole in-feature atomic broker-wire, target-broker, FFI-quarantine, and privilege-contract owner: it changes both StoreSync DTOs with every producer/consumer; defines the audit drain, `OpenPeerPidfdFromAcceptedSocket`, and versioned protocol-5 target host-generation adoption operation; consumes as immutable input the externally installed exact 13-member `SourceGenerationCompatibilityFloorV1` census, including the numeric-protocol-4 peers and exact negotiated `source-handoff-v1` catalogue fingerprint, but owns no source census member; owns only the target-v5 schema/catalogue/fingerprint/snapshot/fixture transition, target broker validation of the broker-managed target-object/GC-root and installed apply-object pins, the target coordinator adoption half, and exact source-to-target ownership transfer; binds source/target broker/daemon generations, numeric protocols, and exact catalogue digests; makes exact Hello mandatory before d2b-state publication; requires initial public-socket Admin evidence and a sealed durable handoff capability for every phase, while denying daemon-identity/euid0-only and every caller-claimed/admin-on-broker/launcher/root/HostShutdown/remote path; bumps `PROTOCOL_VERSION` 4 to 5; and updates `d2b-contracts`, `d2b-core` privilege sources, target broker protocol/runtime/bootstrap/`sys.rs` sources, target `d2bd` Hello/version and both ancillary-fd receive paths, `nixos-modules/privileges-json.nix`, `xtask` generators, Rust/Nix parity and policy tests, the target-v5 handoff schema/catalogue row, `wire-protocol.json`, `privileges.json`, target daemon/privilege catalogues, target fingerprints and snapshots, target-v5 fixtures, workspace lockfile, and standalone broker lockfile in one commit. The external prerequisite must already have installed the exact 13-member source census atomically; T592 refuses every `missing`, `duplicate`, `extra`, `empty`, `stale-generation`, `stale-digest`, and `cross-disposition` source member, plus a legacy or otherwise mismatched source contract, rather than regenerating it. Both receive paths require `MSG_CMSG_CLOEXEC`, complete ancillary parsing, exact-one-fd validation, and close-on-every-error behavior. A safe dependency may provide peer-pidfd acquisition only if it meets the exact contract; otherwise repository-authored unsafe is confined to approved `sys.rs` with per-block `SAFETY:`. T592 also owns `d2b-audit`; the authoritative `ZoneBundle`; duplicate-envelope retirement; and the one enforcing pidfd policy suite under `make test-fixture-contracts`, including a poison fixture for the forbidden `nix` 0.31.3 `PeerPidfd` wrapper. `make test-policy` must not duplicate or claim this suite. A new project FFI crate and a local session-crate syscall fallback are ineligible. T605 owns its Zone enum, governing specs, paired reference, and final API snapshots. T592's serialized commit may edit `d2bd/src/lib.rs`; after it lands, ownership transfers to T595. |
| Serial daemon composition | T595 | Sole writer after T592 for `d2bd/src/resource_runtime.rs`, `d2bd/src/lib.rs`, `d2bd/Cargo.toml`, new `d2b/src/bin/d2b-host-generation-deploy.rs`, `d2b/Cargo.toml`, `nixos-modules/{bundle-zones,host-daemon,host-broker,options-site}.nix`, the Type-1 `host-generation-rebuild-ref.nix` case, and Type-10 `host-generation-handoff.nix`; begins only after T590, T592, T594, and T605 converge. It owns startup ingestion, exact 4/2 consumption, the parameterized unprivileged validation/build/stage/public-socket-authorization/opaque-request-only target-closure entrypoint, target-generation installed apply behavior, and broker-before-daemon unit ordering. The caller-flake entrypoint resolves once while unprivileged and never runs under `sudo`. Privileged apply invokes the exact separately broker-pinned immutable object from trusted currently installed-generation metadata, receives no flake URI, installable, reference, target executable, path selection, command, or argv to reevaluate, retains at most 16,384 Nix-stderr bytes in memory, fails closed on overflow, drops the bytes, and emits only fixed redacted errors. Its command carries no intent selector or authority token. The broker enforces the durable single-nonterminal-intent rule, atomically selects and claims only the sole `authorized-pending` intent, and refuses zero, multiple, concurrent, stale-claim, or terminal selection before mutation. Its accepted connection is bound through a direct connection-scoped peer pidfd and live executable identity to that exact pin; exit, exec, PID reuse, mismatch, or ambiguity refuses, and no pidfd is persisted. A pre-mutation disconnect may release only a proven zero-mutation claim; post-mutation replacement is coordinator replay of the same intent and same pinned apply object, never caller selection. The accepted external source-generation disposition owns the exact nonempty 13-member `SourceGenerationCompatibilityFloorV1` census; its installed source broker owns the pre-transfer target-object/GC-root/apply-object pins, durable coordinator, stock profile publication, service transition, 3/1 bootstrap, rollback, and transfer. T592's target broker owns only post-transfer phases. T595 consumes that accepted external contract and wires target broker-before-daemon ordering; changing target `host-broker.nix` cannot by itself satisfy or install the source-generation prerequisite. Neither daemon nor entrypoint owns recovery, and no new unit or runtime unit override is permitted. The option is required `lib.types.strMatching` with exact `<flake-ref>#<configuration-name>` grammar, 2048-byte total limit, and 64-byte selector limit. `make nix-unit-pin` and Type-1 positive/boundary/missing/malformed/selector cases are required in addition to the VM crash/rollback matrix. |
| File-disjoint acceptance and docs | T596-T599, T604 | T599 retains its named CLI/reference ownership. T604 owns new `packages/d2b-contract-tests/tests/resource_operator_activation.rs`, `packages/d2bd/tests/resource_operator_activation.rs`, `tests/host-integration/resource-operator-activation.nix`, existing `tests/host-integration/daemon-restart-vm-survival.nix`, and only those checks' host-integration discovery/build recipe in `Makefile`. Its 3/1 leg starts from a source generation that has independently satisfied the external compatibility-floor prerequisite; it must not synthesize that actor from F. It resolves and authorizes one exact target store output, applies only through the separately broker-pinned installed apply object without Nix reevaluation, and rejects multi-output target resolution, target/apply executable substitution, GC-root replacement, changed installed symlinks, and apply-peer exit/exec/PID-reuse/identity mismatch or ambiguity before mutation. Its handoff matrix races two authorization calls, races two apply connections, injects a corrupt two-pending-intent census, retries after a pre-mutation disconnect, retries after a post-mutation disconnect, and invokes apply after completion. It requires one atomic winner only where exactly one pending intent exists, no caller selector or token, zero mutation by every refused contender, same-intent coordinator replay only after mutation, and no terminal replay. It kills the entrypoint and installed source compatibility actor before and after coordinator durability and every mutation/ownership-transfer boundary, requires the existing broker service to restart pre-transfer work, and covers target broker/daemon failure, exact profile/reference rollback, Nix stderr canary redaction, and the resource effect/cleanup story. It emits the SC-002 receipt only as a current-effective-uid `0600` external output; T600 alone passes that file to T589's explicit candidate-local import. Its evidence must enumerate and build both `vmChecks.x86_64-linux.resource-operator-activation` and `vmChecks.x86_64-linux.daemon-restart-vm-survival`; skip or empty discovery is ineligible. The latter proves public Ready/Stopped, same-runner fresh-pidfd adoption, PID-reuse/mismatch/ambiguity quarantine, and exact set equality after excluding only canonical `d2b.slice` from the full loaded `d2b*`/`microvm*` namespace. Separate injected unexpected-slice and unexpected-service cases must each remain after that exclusion and fail the equality check. The other tasks retain their named files. All five tasks may proceed together after T595 and share no file. |
| Integrator convergence and freeze | T220 | Merges every slice; reconciles generated manifests for in-wave amendments other than the externally owned delivery Version 2 transition; revalidates that Version 2, its approvals, Gate 0 receipt, and regenerated manifests were already complete on an ancestor of T589's base; and verifies coordinated normative/reference/test/schema/changelog treatment, including the authoritative single-owner 4/2 bundle contract/compiler/schema, poison guard, canonical digest, old/mixed/future refusals, replayable installed-host migration/rollback, closed runtime action, and command-only docs. T220 cannot perform or defer the pre-T589 delivery-contract transition. It folds fragments, rebases after W4, and records the panel base. It refuses to freeze F while the untouched external Network sole-opt-in contract conflicts with the feature's double-opt-in target: T070/T071 require the accepted external versioned correction/migration or the authoritative sole-opt-in historical disposition, and no feature status can bypass that blocker. It runs the closed-evidence profile plus the point-specific wave-reservation oracle, collision-safe bounded orphan retirement, all three census-byte and four kind-specific incident-id goldens, durable/CLI status-schema separation and remediation derivation, persisted status-kind agreement, private zero-mutation whole-scope retention guard with candidate-root and permanent-history preservation, canonical signed incident-disposition validator, concurrent first-request, duplicate/same-wave alternate-candidate, and post-request movement tests at panel, seal, and eligibility. It then runs integration, CI, and full drift and opens or updates the PR. Each integrated provisional candidate receives a nonbinding `/d2b-panel-round plan` phase review; findings route scoped fixes back through this row, followed by validation and a delta/full-context phase rerun. Only unanimous phase convergence freezes final F. Any later content/history change invalidates F and restarts T220 plus T600-T602 and the phase review, provided no binding request against F or its replacement provisional candidate has occurred. The retained historical request is not against F and does not disable this pre-disposition nonbinding convergence. |
| Frozen-candidate evidence | T600-T601 | Read-only evidence lanes run against F. They write delivery evidence only, not repository files, and emit the exact closed validation identifiers assigned below. T600 imports `operator-nix-activation-cleanup` by passing T604's explicit external `0600` receipt input to T589's candidate-local importer; the importer durably installs the sidecar before it publishes the record. They may run together subject to the heavy-gate limit. |
| Mechanical evidence convergence | T602 | Revalidates exactly one T072 historical/current remedial disposition without requiring or changing T072's checkbox, then verifies dependency closure, resume identities, clean F, T220's unanimous final phase-panel receipt, and the exact evidence-identifier multiset. T219 is blocked until it passes. |
| External disposition and conditional close | T219 | Revalidates the T072 disposition, T603 phase-plan chain, and T220 final nonbinding phase-plan receipt, but those process rounds do not replace or dispose the retained Wave 5 binding delivery request. T219 first requires the external owner to land the delivery-contract/tooling change and validator, then imports exactly one valid `Wave5RetainedRequestDispositionV1` bound to the byte-preserved request and exact F. Before that import, T219 authorizes no request, attestation, seal, merge-target registration, merge eligibility, or merge. `remain-blocked` stays blocked; `abandon-without-merge` is terminal and cannot advance; `recover-panel-without-new-request` permits only the external recovery-attestation surface, still requiring the complete ten-role unanimous exact-F panel before seal or merge. The record creates no second request and supplies no panel result or constitutional waiver. F and `adr046w5` delivery history stay immutable. |

The T589 ownership row is refined by the SC-002/source-floor closure in `tasks.md`.
The accepted `docs/specs/ADR-046-validation-and-delivery.md` Version 2 amendment and its
generated manifests belong exclusively to the external pre-T589 row; T589 does not edit
them. In addition to its prior implementation files, T589 owns
`docs/reference/schemas/delivery/sc002-incident-{status,cli-status,disposition}-v1.schema.json`,
`tests/golden/delivery/sc002-incident-{human,json}-v1.txt`, and
`tests/golden/delivery/sc002-incident-{id,disposition}-v1.json`. Its existing
`changelog.d/resource-api-production.md` fragment carries the incident-recovery entry, so the
fourteen-fragment map does not grow. It owns the exact inspect/apply/admit-successor delivery
commands, stable IDs, exits, durable status plus the distinct deterministic CLI JSON
remediation projection, private canonical-disposition validator, collision-safe bounded
retirement, candidate-retention owner, and focused tests. That
nonbinding successor flow never releases or creates a binding request and preserves Wave 5's
retained request byte-for-byte.

The accepted external Version 2 delivery amendment must pin the closed
`Sc002IncidentKindV1` enum, the four kind-specific domain-separated
`Sc002IncidentIdV1` preimages, persisted status `incidentKind`, the separate
`Sc002IncidentCliStatusV1` field order/remediation derivation, and the complete
`CanonicalRetiredCensusV1` byte contract before T589 dispatch. That census contract includes
the `0x01` version, `0x00` normal and `0xff` over-bound body tags, framed raw relative paths
and record count, unsigned-byte ordering, exact entry/observation/failure tag values, the
all-zero/`u64::MAX` unavailable tuple, and exact whole-census sentinel bytes `01ff`.
T589's incident-id golden has exactly one independently recomputed record for
`retirement-id-collision`, `retirement-census-exhausted`,
`retirement-census-invalid`, and `identity-ambiguity`. Collision binds separately observed
source/destination identities; the census kinds bind one source identity plus bounded census
evidence without fabricating a second tuple; ambiguity binds the closed reopen stage and
ordered before/after identities. The same golden has exactly three census vectors:
normal-empty, normal-sorted-mixed, and exact over-bound `01ff`. T220 independently encodes
and verifies all three census vectors, all four incident vectors, both strict status schemas,
the remediation projection table, and one-to-one payload-or-locator/status/id kind agreement
before F may freeze.

The external source-floor row now requires issuer-authenticated canonical receipts, strict
schemas/golden vectors at the external-owner paths in `data-model.md`, and a
disposition-pinned validator that returns T589 a nonserializable typed result. The 91-case
poison matrix visits all 13 roles for all seven classes with cardinality 13, recomputed
enclosing hashes, and valid test signatures; copied authority/proof chains refuse. Within
T589's SC-002 cleanup, the fixed stable OFD-lock inode and live-owner refusal precede all
namespace access. Verified orphans move under distinct retirement ids into the bounded
durable census. Incidents use immutable preimage-complete metadata, a no-replace moved and
revalidated payload outside the ephemeral namespaces, and append-only status; a raced
rename/reopen remains recovery-pending with every name preserved until restart. No sidecar
data leaf is unlinked. The private retention owner performs only the
whole-scope retention guard and proves the canonical candidate root plus every permanent
delivery-history namespace remains immutable. Overlap tests latch both orderings for
same/different importer, cleanup, and retention actors and recover two identical orphan
leaves without collision. T595/T604 own the independent closed 15-edge registry, six
pre-first transition cases, and exact 84-case post-first cross-product after the first
durable mutation, plus raw PID/start/store/derivation/NAR canary absence from
every state/output surface with digests only. These refinements make retirement
identity-bound and collision-safe, name the sole retention owner, keep successor admission
closed, and remove the former cleanup-by-deletion and output-only identity shorthand without
changing feature-task dependencies.

T592 has two mechanically closed subcontracts within its serialized scope. First, it consumes
the accepted external source floor as a read-only input and owns only target-v5 coordinator
adoption. Before T592 starts, the source generation must already have atomically installed
the exact nonempty 13-member `SourceGenerationCompatibilityFloorV1` census, with every role
present once under one disposition and generation, plus pre-transfer coordinator behavior
under the existing `d2b-priv-broker.service`. Bare
protocol 4 refuses. The existing service starts and restarts that installed actor before
transfer. T592's target broker authenticates the exact external source catalogue fingerprint
and durably adopts coordinator ownership once before daemon activation. Tests inject every
`missing`, `duplicate`, `extra`, `empty`, `stale-generation`, `stale-digest`, and
`cross-disposition` source member class, fingerprint mismatch, target/apply
executable and symlink substitution, every apply-peer identity transition before the first
and in the exact 84-case post-first registry, target broker startup failure, target daemon
startup/reconciliation failure, entrypoint death, every compatibility-broker crash boundary,
and both sides of ownership transfer. Recovery accepts only the pre-transfer external source
owner under the existing broker service or the post-transfer target broker owner, never
`d2bd`, Nix activation, the entrypoint, root/provenance/caller claims, or a new unit. T592
solely owns the target protocol-5 half and
`packages/d2b-priv-broker/tests/host_generation_coordinator_v5.rs` for this matrix; it neither
implements nor attests installation of the source-generation actor. Second,
T592 solely owns
`packages/d2b-contract-tests/tests/policy_peer_pidfd_quarantine.rs` and
`packages/d2b-contract-tests/tests/fixtures/peer_pidfd_quarantine/`. The nonempty source
policy and poison fixtures enforce exclusive safe-wrapper or approved `sys.rs` quarantine,
immediate `SAFETY:` comments, and explicit rejection of the `nix` 0.31.3 `PeerPidfd`
`MaybeUninit`/assert wrapper. The sole enforcing runner is
`D2B_ENABLE_FIXTURE_BUILD=1 make test-fixture-contracts`; `make test-policy` owns only its
existing meta-gates and must not run or duplicate this policy. T220 requires both
subcontracts before F can freeze.

This `/d2b-spec-edit` batch changes planning artifacts only. The implementation escalation is
exact and remains pending: no in-feature task owns the missing source-generation bootstrap
floor; its accepted external disposition and installation block T589. After that prerequisite,
T592 owns target-broker adoption, post-transfer coordinator phases, wire receive helpers,
pidfd policy, and broker tests; T595 owns the entrypoint and target existing-unit ordering;
T604 owns the substitution, crash-replay, redaction, and host acceptance tests; T220 owns only
convergence and generated-output reconciliation. No source, test, Nix,
normative specification, ADR, contributor guide, changelog, or panel artifact is changed by
this batch.

The T589 row has one additional dispatch guard after T603 completes: fresh analysis with no
unresolved HIGH or CRITICAL finding and a unanimous ten-role plan review MUST bind exact
clean C/Q. The P-to-Q checkbox content change invalidates B/P sign-off for implementation
dispatch, and any later content or history change invalidates the C/Q guard.

The implementation and close dependency chain is exactly:

```text
accepted external Principle VI constitution amendment -> every implementation/resume/fix/close/advance path below
reported W2 seal/merge -> {exact external delivery-record confirmation OR accepted external correction} -> T008/T028/T029 historical verification and adjudication only
adjudicated W2 history -> reported W3 seal/merge -> {exact external delivery-record confirmation OR accepted external correction} -> T030/T035/T036 historical verification and adjudication only
adjudicated W3 history -> reported W4 seal/merge -> {exact external delivery-record confirmation OR accepted external correction} -> T037/T070/T071 historical verification and adjudication only
adjudicated W4 history -> {T072 exact historical attestation OR current A/P0 remedial disposition}
T072 disposition -> pre-T603 analysis + plan panel at A/P0
pre-T603 analysis + plan panel at A/P0 -> T603 validator commit V
V = B -> post-T603 analysis + plan panel at B/P -> receipt/editor transition C
C/Q -> fresh analysis + unanimous plan panel at exact C/Q
accepted external source-generation compatibility disposition
  -> named external producer/installer
  -> named typed validator/importer
  -> SourceGenerationCompatibilityFloorV1 imported-for-exact-C/Q
{fresh C/Q analysis + plan panel, imported source-floor receipt} -> T589 -> {T590,T591,T594}
T591 -> T592
T592 -> T593 route -> T605
{T590,T592,T594,T605} -> T595
T595 -> {T596,T597,T598,T599,T604} -> T220 provisional candidate
T220 provisional candidate -> iterative /d2b-panel-round plan phase convergence -> freeze F
F -> {T600,T601} -> T602 T072-disposition and phase-receipt revalidation
retained Wave 5 binding delivery request
  -> external contract + validator
  -> imported Wave5RetainedRequestDispositionV1 X bound to F
{T602,X} -> T219 closed action
recover-panel-without-new-request -> exact unanimous ten-role F-bound attestations -> seal/merge eligibility
```

The host-generation sub-DAG inside `T592 -> T595 -> T604` is fixed:

```text
accepted external source-generation compatibility disposition
  -> source 3/1 generation atomically installs both numeric-protocol-4 peers, their exact
     negotiated `source-handoff-v1` catalogue fingerprint and matching generated contract
     set, plus the source apply object
  -> existing d2b-priv-broker.service supervises that exact installed actor
  -> T589 dispatch gate opens
  -> T592 protocol-5 Hello + generation identity + privilege/output parity
  -> T595 unprivileged resolution of one exact target executable store object
  -> T595 build target closure and immutable reference input with a 16,384-byte in-memory
     Nix-stderr ceiling
  -> file-and-directory-durable deployment intent
  -> unprivileged existing-public-socket Admin classification
  -> source daemon and broker negotiate numeric protocol 4 plus the exact
     `source-handoff-v1` catalogue fingerprint
  -> source daemon transfers exactly one accepted-socket evidence fd over that authenticated
     source-compatibility channel
  -> installed source broker consumes it into a broker-durable sealed handoff capability
  -> broker-managed GC root plus exact target-object pin and separately resolved installed
     apply-object pin
  -> under one coordinator lock, selector-free apply atomically claims the sole
     authorized-pending intent; zero, multiple, or concurrent claims refuse
  -> accepted apply connection binds that claim's direct peer pidfd and live executable
     identity to the apply-object pin, with immediate pre-mutation revalidation and no
     serialized token or persisted pidfd
  -> installed source broker under existing d2b-priv-broker.service durably owns the
     coordinator before first mutation
  -> existing service restarts the same installed source broker on pre-transfer failure
  -> broker-audited stock profile publication and target broker transition
  -> target broker staged-identity adoption audit
  -> durable coordinator ownership transfer to the target protocol-5 broker
  -> broker-audited target daemon service transition
  -> fresh target-daemon protocol-5 Hello while unready
  -> phase-attenuated authenticated opaque publication request
  -> broker-durable d2b pointer and stable-reference publication
  -> daemon reopens durable publication
  -> complete 4/2 ingestion and readiness acknowledgement
  -> T604 declaration/identical/removal deployments
```

Any failure after stock publication follows one recovery edge only:

```text
broker-owned durable coordinator reopens intent and capability
  -> before transfer: existing d2b-priv-broker.service restarts its installed source broker
  -> source-generation compatibility owner resumes or rolls back without the entrypoint
  -> after transfer: restarted existing d2b-priv-broker.service resumes or rolls back
  -> capability-authorized owner records durable rollback preparation
  -> prior d2b pointer and reference bytes-or-absence restoration
  -> target broker or supervised installed source broker audited stock rollback
  -> source system/broker/daemon/3/1 verification
```

No readiness, acknowledgement, repair, or rollback edge may skip its predecessor. The
Type-10 crash table injects immediately before and after every arrow, including target broker
startup failure, target daemon startup failure, every compatibility-broker exit boundary, and
both sides of durable ownership transfer. Before-transfer recovery accepts only the
externally installed source-generation compatibility owner under the existing broker service,
after-transfer recovery accepts only
the target broker owner, and neither requires a
new unit or a surviving entrypoint. It also authorizes target executable A and attempts apply through a different target
executable, a substituted installed apply object, and a changed installed symlink; all
substitutions must refuse before mutation while A remains eligible. Apply-connection tests
also force peer exit, exec, numeric PID reuse, executable-identity mismatch, and ambiguous
identity; each refuses before mutation and leaves no persisted pidfd. The done predicate is
one exact matching source or target tuple, one lifecycle owner, and zero mixed/unaudited
transition, not merely a running daemon.

T603 is the sole in-feature direct prerequisite of T589 and never treats code presence as
completion. FR-070's accepted and installed source-generation compatibility floor is a
separate external dispatch prerequisite.
Its gate is deliberately two-pass because the validator cannot attest to a base that predates
its own implementation:

1. **Pre-validator authorization.** Freeze clean commit A and the exact 28-file feature
   snapshot P0. Run `/speckit-analyze` against the feature artifacts at A/P0. Only a receipt
   with no unresolved HIGH or CRITICAL finding may proceed to the unanimous
   `/d2b-panel-round plan` review bound to A/P0. These receipts authorize only T603's three
   owned paths; they do not authorize a reconciliation receipt, checkbox edit, T589, or any
   other Wave 5 implementation.
2. **Validator implementation.** T603 changes exactly
   `packages/xtask/src/delivery/{mod.rs,resume.rs}` and its tests within those files and
   creates the mandatory unique fragment
   `changelog.d/delivery-resume-reconciliation.md`. It writes no feature artifact, generated
   output, source elsewhere, or tracked evidence. The integrator lands one dedicated
   validator-and-fragment commit V whose sole parent is A and whose diff is limited to those
   three paths. T603 validates the fragment at that exact path and leaves it unfolded; T220
   alone folds it after later convergence, and no T220 action is a prerequisite of T603.
3. **Post-validator authorization.** Freeze resume base B exactly at V and recompute feature
   snapshot P. P MUST be byte-identical to P0 because T603 has no feature-file ownership.
   Rerun `/speckit-analyze` over `A..B` plus the complete feature artifacts, then rerun
   `/d2b-panel-round plan`; both new receipts MUST bind B and P, and the panel request MUST
   expose the validator delta. This is a nonbinding phase-plan request and creates no delivery
   `panel-request.json` or binding reservation. A finding or any subsequent validator-code change abandons B.
   A source-only fix creates a new V/B and reruns both post-validator gates. A finding that
   requires a feature-artifact change returns to a fresh `/d2b-spec-edit` batch, establishes a
   new A/P0, and reruns the entire pre-validator and post-validator sequence. Neither receipts
   from the old A/P0 nor receipts from the abandoned B may be reused.
4. **Receipt/editor transition.** Only the passing post-validator B/P receipts permit T603 to
   create the immutable resume authorization, evaluate the 146 rows, and route the sole
   checkbox change through `/d2b-spec-edit`.
5. **Post-editor plan gate.** After the integrator creates and finalizes dedicated checkbox
   commit C, require clean HEAD C and exact feature snapshot Q. Run fresh
   `/speckit-analyze` against the complete feature artifacts at C/Q and require no unresolved
   HIGH or CRITICAL finding, then obtain a fresh unanimous ten-role
   `/d2b-panel-round plan` review whose records bind C and Q. The P-to-Q content change makes
   every B/P plan sign-off stale for T589 dispatch; B/P remains evidence only for the
   authorized editor transition. Any content or history change after the C/Q receipts
   invalidates them and blocks T589. A finding that requires a feature edit returns to a
   fresh `/d2b-spec-edit` batch and the applicable gate sequence; no prior sign-off transfers.

This sequence implements one reusable hermetic validator in
`packages/xtask/src/delivery/{mod.rs,resume.rs}` and writes an immutable resume-authorization
receipt outside Git at `.scratch/autopilot/adr046w5/reconciliation.json`. Final-candidate
evidence remains a separate T600/T601 concern. The authorization receipt has this closed
shape; every object rejects unknown fields:

- top level: `schema_version` exactly `2`; `wave` exactly `adr046w5`; `repository`;
  `feature_path`; `feature_files`; `pre_edit_feature_snapshot_sha256`;
  `authorized_post_edit_feature_snapshot_sha256`; `resume_base_commit`; `resume_base_tree`;
  nonempty `branch`; `analysis`; `plan_panel`; `changed_task_ids`; and `items`;
- `repository`: `project_id` exactly `7f6d0beab0ce4c13a89f6865d5ac42e2` and
  `object_format` exactly the format reported by Git. The project ID is an opaque, non-routing
  repository sentinel; no hosting domain, account, remote URL, or checkout path is receipt
  identity. `feature_path` is exactly the repository-relative path
  `specs/001-adr046-d2b3-completion`, never an absolute checkout path. The validator discovers
  the current checkout root with Git, verifies the compiled project sentinel at that Git root,
  does not consult `remote.origin.url`, and resolves `feature_path` beneath the held
  checkout-root directory fd;
- `analysis`: the post-validator result, whose closed values are `pass` and `fail`; the same
  pre-edit snapshot P and resume-base commit B as the top level; and one nonempty local receipt
  or session locator, with no transcript. An A/P0 receipt is stale for this field;
- `plan_panel`: the post-validator panel, with `round` matching
  `^adr046w5-r[1-9][0-9]*$`;
  `reviewed_feature_snapshot_sha256` equal to the pre-edit snapshot;
  `reviewed_resume_base_commit`; `unanimous`; and `record_locators`, an object with exactly
  one nonempty locator for each of `software`, `test`, `nixos`, `networking`, `security`,
  `rust`, `product`, `docs`, `observability`, and `kernel`, and no verdict text;
- `changed_task_ids`: exactly T073 through T218 followed by T603; and
- `items`: exactly 146 rows in numeric task order T073 through T218. Each row has only
  `task_id`, `status`, `obligation_id`, `evidence_kind`, and `evidence_locator`.
  `status` is exactly `satisfied` or `open`. A `satisfied` row uses evidence kind `commit` or
  `delivery-receipt` and a nonempty qualifying commit or local receipt locator. An `open` row
  uses `none` and a null locator. Task IDs and obligation identities are unique, and each
  obligation identity is the exact work-item identity named by that task.

`feature_files` is the following exact bytewise-sorted relative-path list:
`amendment-frozen-cross-zone-contracts.md`,
`amendment-provider-derivation-layout.md`, `amendment-spike-01-rerun.md`,
`amendment-w2-destination-drift.md`, `amendment-w5-destination-drift.md`,
`checklists/coverage.md`, `checklists/requirements.md`, `contracts/README.md`,
`contracts/companion-contracts.md`, `contracts/generated-artifacts.md`,
`contracts/nix-configuration.md`, `contracts/operator-cli.md`,
`contracts/resource-api.md`, `data-model.md`, `deferred-findings.md`, `friction-log.md`,
`gate0-reevaluation-spike-01-rss-rerun.md`, `gate0-reevaluation.md`,
`implementation-debt.md`, `plan.md`, `quickstart.md`, `removal-proof-inventory.md`,
`removal-proof-w5.md`, `research.md`, `spec-coverage.md`, `spec.md`, `tasks.md`, and
`waiver-w0-w1.md`. A missing, extra, symlinked, or non-regular member fails receipt creation.
The snapshot stream is, for each listed file in that order, an unsigned 64-bit big-endian
path-byte length, the UTF-8 relative-path bytes, an unsigned 64-bit big-endian content-byte
length, and the file bytes verbatim. `feature_snapshot_sha256` is the lowercase 64-hex SHA-256
of the concatenated stream; the receipt stores that value as
`pre_edit_feature_snapshot_sha256`, and the validator computes
`authorized_post_edit_feature_snapshot_sha256` from the same framing after the exact
authorized token changes.

Preparation fails on an unknown field, unknown status or evidence kind, task-ID duplicate,
missing or extra task, identity mismatch, stale feature hash, branch mismatch, dirty index or
worktree, relevant untracked path, analysis identity mismatch, panel base/snapshot mismatch,
malformed round address, non-unanimous or incomplete panel records when progress is requested,
use of a pre-validator A/P0 receipt in a B/P field, or a locator that is absent or not local
delivery state. The validator computes the
authorized post-edit snapshot in memory by applying exactly the 147 checkbox-token changes to
the pre-edit bytes; no editor-supplied post hash is trusted. The receipt contains only
addresses, classifications, and outcomes. It contains no diff, transcript, command or
validation output, secret, credential, Nix store path, or raw sink detail.
For this protocol, a relevant untracked path is any non-ignored path reported by
`git status --porcelain=v1 --untracked-files=all`; ignored external receipt state under
`.scratch/` is not candidate content.

All feature and receipt access is fd-anchored. The validator opens the checkout root,
feature directory, and receipt directory as `O_DIRECTORY|O_CLOEXEC` fds and uses `openat2`
with `RESOLVE_BENEATH|RESOLVE_NO_SYMLINKS|RESOLVE_NO_MAGICLINKS|RESOLVE_NO_XDEV`; lack of
`openat2` support is a fail-closed environment error naming the required kernel support.
Missing receipt-directory components are created one at a time with `mkdirat` from a held
parent dirfd at mode `0700`, then reopened and verified with the same `openat2` policy.
Every member is opened `O_RDONLY|O_CLOEXEC|O_NOFOLLOW`, verified as a regular file by
`fstat`, and hashed from that same fd. Ancestor and leaf path checks are never separated from
the read. The mode-`0700` receipt directory is opened and verified through its held fd.
Temporary files are created exclusively relative to the appropriate held dirfd with
`O_CREAT|O_EXCL|O_CLOEXEC|O_NOFOLLOW`, written completely, and `fsync`ed. Immutable receipts
with mode `0600` use `renameat2(..., RENAME_NOREPLACE)` and then `fsync` the receipt directory.
The `tasks.md` editor preserves the verified original owner and mode and holds the original
leaf fd and its device/inode identity. It creates and `fsync`s the replacement through the
held feature dirfd, reopens the original with `openat2` immediately before publication, and
requires the same device/inode. Publication uses
`renameat2(..., RENAME_EXCHANGE)` relative to that dirfd. The displaced leaf is reopened and
must be the held original inode before it is removed with `unlinkat`; a mismatch triggers a
validated exchange-back and hard failure rather than accepting a raced replacement.
`RENAME_EXCHANGE` unavailability fails closed. The replacement file and feature directory are
both `fsync`ed before completion is reported. No editor, reader, segment writer, exporter, or
pruner joins a pathname after acquiring its governing directory fd. A wrong owner, broader
receipt mode, mount crossing, replacement race, partial write, or failed file/directory sync
is a hard failure. Readers validate through the same opened fds; file existence alone is not
a receipt.

The checkbox transition is a three-step prepare/apply/finalize protocol:

1. **Prepare.** At clean exact post-validator `resume_base_commit` B = V and its tree, T603
   validates the post-validator analysis and plan panel bound to B/P, computes pre-edit
   snapshot P and authorized post-edit snapshot Q, and durably creates the immutable
   reconciliation receipt with `RENAME_NOREPLACE`. Any `open` row leaves T603 unchecked and
   creates no authorization.
2. **Apply.** `/d2b-spec-edit` reopens and validates the authorization. If HEAD is B and the
   complete tree is clean at P, it atomically replaces only `tasks.md` with the exact
   checkbox-token transition to Q. If HEAD is still B and the worktree is already at Q with
   exactly that one diff and no relevant untracked path, it treats the apply as complete
   rather than editing again. The diff may be wholly unstaged with an empty index or wholly
   staged with a clean worktree; mixed or partial staging refuses. The Wave 5 integrator, not
   the editor or an autopilot lane, stages only `tasks.md` when needed and owns one dedicated
   checkbox commit C. C MUST have B as its sole exact parent, and `B..C` MUST contain only the
   authorized 147 checkbox-token changes with feature snapshot Q.
3. **Finalize.** If a crash occurs after the edit or staging but before C, resume recognizes
   only an exact permitted B/Q state and creates C. If a crash occurs after C but before the
   progress receipt, resume recognizes only HEAD C with parent B, clean tree Q, and the exact
   authorized diff, then durably creates
   `.scratch/autopilot/adr046w5/progress-editor-receipt.json`. That closed receipt binds the
   authorization-receipt SHA-256, repository identity, relative feature path, B, C, C's exact
   parent B, P, Q, and the exact changed task-ID set. If the receipt already exists, it is
   reopened and fully revalidated rather than replaced.

The prepare/apply/finalize protocol authorizes only the exact progress transition. Because C
changes feature content from P to Q, it invalidates B/P plan sign-off for implementation
dispatch. Before T589, fresh analysis with no unresolved HIGH or CRITICAL finding and a fresh
unanimous ten-role plan review MUST both bind exact clean C/Q. No panel is run as part of the
editor transition itself.

T589 starts only with HEAD exactly C, a clean complete worktree, the finalized progress
receipt, all 147 checkboxes checked, and valid exact-C/Q analysis and unanimous plan-review
receipts. Later implementation commits do not stale the
authorization: T602 validates the immutable receipt against B/P, the finalized transition
against C/Q, requires C to be the exact child of B and an ancestor of final candidate F, and
validates T600/T601 separately against F and F's tree. This ancestry-and-snapshot chain is the
sole authorized progress transition; neither receipt is reinterpreted as evidence for the
final candidate.

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
| T603 | `changelog.d/delivery-resume-reconciliation.md` |

These paths supplement each task's owned-file list. No other task may write them. T220
requires all fourteen, including T603's exact path, before it reconciles
version/reference/test/schema treatment, then alone folds all fourteen into the shared
changelog. T603 is an integrator-owned validator prerequisite, not a slice, and owns exactly
three repository paths: its two Rust source files plus its mandatory unique fragment;
T603 completes its fragment obligation by creating and validating that still-unfolded file;
folding is exclusively T220's later convergence action and is not a T603 dependency.
T600-T602 and T219 write only external evidence or delivery state, and T220 only folds. A
missing, duplicate, differently named, or cross-owned fragment blocks both V and T220; prose
that an amendment has release treatment is not a substitute.

The T589 ownership-table row's older incident-path shorthand is superseded by
`data-model.md` and `tasks.md`: the sole terminal mismatch is immutable metadata, a
revalidated payload at
`evidence-sidecars/sc002/incidents/payload/sha256/<incident-id>.bin`, and a contiguous
append-only status prefix, all durably synced. A rename/reopen race is recovery-pending, not
an alternate "unmovable name" terminal. It preserves every name and blocks publication and
close until restart completes that same metadata-bound protocol. The same correction expands
the overlap matrix to importer, cleanup, and retention-guard actor pairs and replaces every
dynamic mutation-edge count with the independent 15-edge/six-transition/84-post-first
registry. No task dependency or file owner changes.

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
peer pidfd persists. Raw peer PID/start and executable store/NAR identity canaries remain
absent from every output surface; only typed fixed digests may correlate them, and metrics
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
| `operator-nix-activation-cleanup` | T600 | `local-host` | T604 exact 4/2 top-level audit carrier, audit-only generation identity change, empty ZoneSpec/no emitted Zone resource, and old/mixed/future refusals; Type-1 required/grammar/2048-byte/selector matrix with current nix-unit pin; accepted external compatibility-floor identity and exact nonempty 13-member `SourceGenerationCompatibilityFloorV1` census with `missing`, `duplicate`, `extra`, `empty`, `stale-generation`, `stale-digest`, and `cross-disposition` poison cases before parameterized fail-closed flake/configuration migration, with bare committed protocol 4 refusing and every preflight stopping before authorization or `sudo`; one unprivileged resolution and broker-managed pin of the exact target executable store object plus a separately broker-resolved and pinned installed apply object, followed by privileged apply only through the latter with no reevaluation; target/apply executable, changed-symlink, and GC-root substitution refusals; apply-connection direct peer-pidfd/executable binding with exit/exec/PID-reuse/mismatch/ambiguity refusal before the first mutation and every later mutation edge, no later mutation after refusal, no persisted pidfd, raw peer PID/start and executable store/NAR identity absent from every output surface, typed fixed correlation digests only, and no identity metric label; validation/build/stage/public-socket-authorization/opaque-request-only entrypoint; accepted public-socket evidence transferred as exactly one fd only after the authenticated source peers negotiate the exact source catalogue fingerprint and consumed by the installed source broker into a sealed nonfabricable durable handoff capability, with no root/provenance/caller-claim substitute; capability-authorized source/target-broker-only stock profile, service, 3/1 bootstrap, publication, and rollback mutations with immutable pre-mutation/outcome audit; broker-owned coordinator before first mutation; existing `d2b-priv-broker.service` start/restart ownership before transfer; compatibility-to-target-broker durable ownership transfer; entrypoint and compatibility-broker death recovery; target broker and daemon startup-failure rollback through existing units; and no daemon-owned recovery or new unit; target broker start, daemon Hello while unready, phase-attenuated authenticated publish request, durable broker publication, then ingestion/readiness; daemon-identity/euid0-only and all caller denials; fixed typed Nix-eval/build failures with raw stderr canaries absent from every output surface; externally owned source peer/schema/catalogue/fingerprint/snapshot/fixture atomicity plus T592-owned target-v5 wire/privilege schema/catalogue/parity/drift/lockfile proof; every compatibility-broker, ownership-transfer, profile, broker, daemon, pointer, reference, readiness, and rollback boundary; no unaudited or direct bypass mutation; closed identifier-free runtime action; runnable parameterized migration, stable-reference, and rollback authorization/apply commands only after the external prerequisite; initial, identical, declaration, and removal deployments; unchanged schema-v2 `EvidenceRecord` whose candidate-relative content-addressed locator names one version-1 bounded fixed-redacted typed `Sc002ActivationReceiptV1`; explicit current-effective-uid `0600` source input, hash-before-decode, current-effective-uid `0700` candidate directories, current-effective-uid `0600` destination, no-replace file-and-directory-durable sidecar publication before record publication, and crash/race recovery at import; one candidate-scoped exclusive OFD lock shared by importer and cleanup, live-owner exclusion, identity-preserving quarantine/reopen/no-replace bounded retirement for verified orphans with no sidecar-data unlink, leaf-and-directory sync, empty ephemeral namespaces on ordinary paths, a zero-mutation whole-scope retention guard preserving the candidate root and permanent delivery history, and durable incident quarantine plus publication/close denial without unlink on identity ambiguity; hash-before-decode at every reopen; actual `candidate_id`/`content_id`/`snapshot_sha256` triplet binding; replacement/traversal/URL/symlink refusal; closed three-resource census, same-identity effect/Ready pairs, valid selected-stop/elapsed/progress ordering, and every sample <=2,000 ms; failed-record-without-receipt import plus passed-record receipt requirements; import/reopen/panel/seal/eligibility negatives for unknown version/field/enum, 16,385-byte, missing/duplicate/mixed/effect-Ready-disagreeing/unrelated identity, stale, wrong-record, misordered, progress-free, and over-budget evidence; exact Provider installs/configs and positive owned effects/readiness for `Volume/acceptance-state`, `Network/acceptance-net`, and `Device/acceptance-tpm`; exact Device swtpm/flush cleanup, unresolvable Endpoint, finalizer clearance, and same-identity TPM state-Volume preservation; ready, identity-stable, unrecreated acceptance Volume/Network and unrelated resources; explicit Guest deferral to Wave 6 `Provider/runtime-cloud-hypervisor` with no Wave 5 Guest-success claim; candidate-bound FR-075 public lifecycle continuity including Ready/Stopped, fresh-pidfd adoption, PID reuse/mismatch/ambiguity quarantine, and exact set equality between the full loaded `d2b*`/`microvm*` namespace and the three ADR-0015 units after excluding only canonical `d2b.slice`, with injected unexpected-slice and unexpected-service negatives; and nonempty host output enumerating and building both `vmChecks.x86_64-linux.resource-operator-activation` and `vmChecks.x86_64-linux.daemon-restart-vm-survival` without skip |
| `resource-plane-rss-owner-fanin` | T601 | `local-host` | Whole-process RSS at 10,000 resources/100 authenticated watches with no baseline subtraction and store, policy, ResourceService, controller endpoint/fan-in, and audit journal/export owner counts exactly one; the system-core registration/list belongs only to `system-core-handler-contract` |
| `wave5-removal-proofs` | T601 | `github-ci` | Every manifest-label W5 removal predicate rerun against F |
| `cli-reference-conformance` | T601 | `github-ci` | Emitted CLI/help/JSON/wire behavior; accepted Version 2 amendment and Version 1 migration guidance; exact ID, exits, mandatory envelope fields, DTO/schema, retained `op inspect --deadline`/`--no-deadline` plus mutual-exclusion/cancellation coverage, identifier-free static human guidance, closed JSON remediation actions, and all T599 reference comparisons |

The five T600 identifiers and three T601 identifiers are exclusive ownership. A record may
contain several test cases only when all cases belong to that row's required-content cell; it
may not use one free-form validation name to satisfy two rows. T600 and T601 then establish:

- authenticated same-Zone request/watch and cross-Zone/self-named-subject denial, with direct
  `SO_PEERPIDFD` peer admission, private issuance, and unsupported/dead/numeric-only/reuse/
  credential/generation/cgroup/ambiguity refusals;
- restart at each generation-commit/effect-ledger/dispatch/completion window, plus stale,
  zero, wrong-UID, and ambiguous cleanup negatives;
- transactional authoritative-journal creation, fd-anchored segment file/directory durability,
  export completion, every crash boundary, logical deduplication by fixed operation digest
  plus mutation ordinal, fixed-digest/record limits, configured post-export journal and
  segment retention, and typed degraded health on prune or sync failure;
- export-pending failure returning `CommittedPendingAudit`, same-ID no-reapply and
  eventual-final-result behavior, cross-subject/Zone/request/restart replay-binding denial,
  different-ID revision/conflict behavior, typed durable operation inspection, raw identifier
  and trace canaries, and degraded status observability;
- 10,000 resources and 100 watches with whole-process RSS, one store owner, one watch fan-in,
  one controller endpoint, one `Provider/system-core` registration, healthy
  `Zone.status.handlers[]` records named `system-core-host` and `system-core-user`, and no
  baseline subtraction;
- T605's exact kebab-case round-trip, handler-list duplicate/missing/wrong-name and
  `ProviderLifecycle` non-substitution coverage, current API snapshots, paired reference
  wording, and byte-identical Zone desired-schema result;
- re-run removal proofs against the candidate rather than citing
  `removal-proof-w5.md`'s older snapshot; and
- emitted CLI/help/JSON/wire output compared with the desktop-wrapper, companion, audio, USB,
  security-key, Resource API, and error reference pages, including Version 2 migration,
  mandatory envelope fields, exact ID/exit/DTO schema, and absence of Zone/ID-bearing
  executable remediation; and
- T604's exact-candidate operator activation-to-effect-and-cleanup result, including the
  emitted bundle identity, deployment-entrypoint-triggered production daemon entry point,
  SC-002's separately versioned typed receipt referenced by the unchanged schema-v2
  `EvidenceRecord` through its candidate-relative digest locator, hash-before-decode at every
  stage, and the actual candidate/content/snapshot triplet, with the exact three-resource
  census, one common monotonic start,
  same-identity effect/Ready/selected-stop/progress observations, checked
  elapsed samples, and passing <=2,000 ms assertions, owned effect/readiness for
  `Volume/acceptance-state`, `Network/acceptance-net`, and `Device/acceptance-tpm`, exact
  swtpm/flush cleanup, unresolvable Endpoint, and preserved TPM state, and
  unchanged ready, identity-stable acceptance Volume/Network and unrelated resources. Guest
  runtime-effect acceptance remains a Wave 6 obligation and is not claimed by this record.

T602's done condition is mechanical: T603, every T589-T599 task, T604, T605, and T220 are complete;
`tasks.md` shows T073-T218 and T603 checked; the immutable reconciliation receipt validates
against resume base B and pre-edit snapshot P; the progress receipt validates dedicated
checkbox commit C, exact parent `C^ = B`, authorized post-edit snapshot Q, and exact diff; C
is an ancestor of final candidate F; and every T073-T218 receipt row is satisfied. The
T600/T601 record union names F and F's tree produced by T220 and its `(lane, validation)`
multiset passes T589's checked-in eight-row closed-profile validator exactly; T220 has proved
the same validator is wired to panel-request/panel-attest, seal, and merge-eligibility, and its missing,
extra, duplicate, unknown, wrong-lane, and conflated negative tests are green. T589's strict-binding suite also proves synchronized cross-candidate first requests for one
program/wave yield exactly one success and one durable fd-anchored reservation. Before
no-replace publication the recovery oracle is zero reservations; after publication but before
wave-directory `fsync` it is zero or one; after directory `fsync` it is exactly one and every
same-candidate or alternate-candidate request refuses permanently for that wave. Fd-relative
orphan cleanup leaves no temporary residue and durably syncs deletion. Injected crashes
around panel-request publication and terminal failed or successful disposition prove
idempotent ordering, a retained request record, and no retry, release-for-reuse, or duplicate
request. Post-request byte-identical history rebase and evidence refresh are rejected at
panel, seal, and merge-eligibility. Nonbinding `/d2b-panel-round plan` phase reviews remain
repeatable before the reservation and cannot create or consume it. A separate retained-state
fixture seeds Wave 5's already consumed `panel-request.json`, snapshots the complete delivery
state, runs a unanimous phase round and a finding-plus-rerun phase sequence, and then requires
the delivery state to be byte-identical. No binding reservation, request, candidate, or
request-disposition byte may be created, removed, renamed, or modified by either phase
sequence. This consumed-request fixture runs through the same T589 panel code and is required
again by T220 and T602. T604's
exact-candidate activation result
appears only in `operator-nix-activation-cleanup`; the coordinated T605 contract, T595 emitter,
and T599 consumer result appears only in `system-core-handler-contract`; and no record names a
direct/fake boundary. HEAD MUST equal F exactly. `git diff --cached --exit-code`,
`git diff --exit-code`, and
`git status --porcelain=v1 --untracked-files=all` MUST each show no staged, unstaged, or
non-ignored untracked state; ignored receipt state under `.scratch/` is outside the candidate.
Same-ID audit retry never reapplies a mutation and replay-binding mismatches deny; the pending
state uses the exact `ResourceStatus` composite carried by the additive protobuf field; the
system-core registration and both required handler records are healthy and unique; current
removal proofs are true; and the RSS value is at or below 24,576 KiB.
For SC-002, T602 and T219 must reopen that sole passed operator record and its referenced
`Sc002ActivationReceiptV1` through T589's validator and revalidate the unchanged
`EvidenceRecord` `candidate_id`/`content_id`/`snapshot_sha256` triplet for F. Its
candidate-relative locator must be the canonical
`evidence-sidecars/sc002/sha256/<digest>.json` content address; each stage resolves beneath
the held candidate dirfd and verifies the exact byte digest before decode from the same fd.
Replacement, traversal, absolute-path, URL, symlink, hard-link, and stale-triplet cases refuse.
The validator also revalidates unchanged schema-v2 decoding, receipt
schema/kind/size/content-digest bounds, the exact
three-resource census, one shared monotonic transition-intent start, same-identity
effect/Ready/selected-stop and progress observations, checked elapsed values, 1-32 correctly
ordered progress events per sample, and every sample at or below 2,000 ms. A failed operator
record remains importable without a receipt but is a false close conjunct. A missing,
duplicate, mixed, unrelated, effect/Ready-disagreeing, unknown-version/field/enum,
16,385-byte, malformed, misordered, stale, wrong-candidate, wrong-record, progress-free, or
over-budget sample is also a false conjunct and blocks close rather than becoming advisory
evidence.

This amendment changes the reviewed Wave 5 plan boundary. The first eligible action is the
pre-T603 read-only analysis at A/P0, followed by a unanimous plan panel at A/P0. That pair
authorizes only the T603 validator commit V. After V, T603 freezes B exactly at V and reruns
analysis plus `/d2b-panel-round plan` with qualified wave `adr046w5` and a round address of the
form `adr046w5-r<n>`, both bound to B/P. Only the no-HIGH/CRITICAL post-validator analysis,
unanimous post-validator plan receipt, and finalized editor progress receipt permit
`/d2b-autopilot --resume` to continue from the `adr046w5` checkpoint. T603 still reconciles
exactly T073-T218; it records T605 only as future work after resume and does not add a 147th
receipt row or a 148th checkbox transition.

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
linked to the retained request. That sequence still requires the complete ten-role panel,
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
| Earlier amendment prose assigned implementation ownership of Volume, Network, and Device to Wave 5. | The task graph assigns Network implementation and its close-blocking obligations to Wave 4 T061-T071. | The exact trio is `Volume/acceptance-state`, `Network/acceptance-net`, and `Device/acceptance-tpm`. Wave 5 proves their production-plane effects together but neither reopens nor claims Network implementation ownership. |
| Earlier feature prose asserted that W4 implemented Host/site plus Network double opt-in for east-west traffic. | Untouched external `ADR-046-resources-network` makes `Network.spec.isolation.allowEastWest` the sole opt-in and says no Zone-level gate is required. This feature cannot silently change that normative contract or claim different historical implementation. | The desired double-opt-in matrix still names all four Network/Host combinations, but W4 adjudication, T070, T071, and T220 remain blocked on an accepted external correction. Confirmation requires a versioned normative amendment and migration that predate actual F4 plus evidence for Network false/Host false, Network false/Host true, Network true/Host false, and Network true/Host true; otherwise the correction must preserve sole Network opt-in as W4's authoritative result and leave double opt-in prospectively unimplemented. No feature status or local matrix can unblock the gate. |
| AGENTS.md says the host exit census matching `d2b*`/`microvm*` returns three. | Committed code exposes canonical `d2b.slice` plus `d2bd.service`, `d2b-priv-broker.socket`, and `d2b-priv-broker.service`. | FR-075 enumerates the full loaded namespace, fails on listing error, excludes exactly `d2b.slice`, sorts, and compares the remainder with exactly the three service/socket names. The conforming raw census is four. Unexpected slice and service injections survive the sole exclusion and fail equality. This feature records but does not edit the external AGENTS.md drift. |
| Earlier handoff prose treated a broker-derived daemon principal and bootstrap euid 0 as independent authorization. | The existing lifecycle authorization chain is public-socket `SO_PEERCRED` plus current `d2b` group classification; broker-socket identity and root execution are not substitute operator authority. | Initial handoff admission transfers the accepted public-socket evidence only after the installed source peers negotiate numeric protocol 4 plus the exact `source-handoff-v1` catalogue fingerprint, then consumes that attachment into one nonfabricable intent-bound capability. No serialized caller/root/provenance/role claim substitutes. Every source or target broker phase consumes the capability or a phase attenuation; daemon identity, Hello, target-closure provenance, and euid 0 are integrity/eligibility checks only. |
| Earlier handoff prose claimed the target closure's compatibility broker could run under the existing broker service before profile publication and allowed the caller-flake target executable to run under `sudo`. | Committed `d2b_contracts::PROTOCOL_VERSION` is 4 and its `BrokerRequest` and operation-catalogue fingerprint have no host-generation handoff operation. Committed `nixos-modules/host-broker.nix` makes `d2b-priv-broker.service` execute the installed generation's `brokerPackage`, not a target-closure binary. Immutability does not make caller-selected executable code a trusted root entrypoint. | No executable authorized source actor exists at this base. T589 and downstream Wave 5 implementation are blocked on an accepted external source-generation compatibility disposition that atomically installs the exact nonempty 13-member `SourceGenerationCompatibilityFloorV1` census before migration. That external owner is outside this feature. T592 consumes the census and owns only target protocol-5 adoption and target artifacts; T595 owns target-generation behavior. The caller-flake target entrypoint runs only unprivileged. Only the separately pinned installed apply object runs under `sudo`, and its live connection peer pidfd/executable identity must match the pin before every mutation. Target/apply/GC-root substitution and peer exit/exec/PID reuse/mismatch/ambiguity refuse. Target-only code, a new unit or override, an entrypoint child or mutation path, daemon recovery, and a synthetic starting image are not accepted substitutes. |
| Earlier follow-up prose called the new source handoff row simply "protocol 4", assigned its compatibility artifacts to T592, and described the external source set with open-ended plurals. | The committed numeric protocol-4 peers negotiate a closed catalogue that does not contain the row; silently changing that catalogue under the same undifferentiated handshake is incompatible. | External scope escalation: the accepted source-generation disposition must install the exact nonempty 13-member `SourceGenerationCompatibilityFloorV1` census from `data-model.md`. Every role occurs once and every member binds the same disposition and source generation; `missing`, `duplicate`, `extra`, `empty`, `stale-generation`, `stale-digest`, and `cross-disposition` poison cases refuse. That external owner adds actual two-peer Hello negotiation whose `operation_catalogue_sha256` equals the exact `source-handoff-v1` catalogue fingerprint. Bare protocol 4 refuses. T592 consumes the census read-only and changes only target-v5 adoption and target outputs. No external source artifact, normative spec, source test, or implementation is edited by this batch. |
| Earlier SC-002 cleanup prose required an identity-mismatched inode to be restored while also requiring both reserved namespaces to be empty on every return. | The fail-closed rule forbids unlinking an inode whose identity is not proven; restoration makes the universal empty-census terminal state unattainable. | The two reserved temp/cleanup-quarantine namespaces are ephemeral and empty only on ordinary terminal paths. The sole mismatch terminal publishes preimage-complete metadata, moves the currently named suspect no-replace into durable incident payload quarantine outside those namespaces, reopens the moved inode, syncs both parents and every changed ancestor, then append-only publishes `parked` status. A replacement or rename/reopen ambiguity stays `recovery-pending`, preserves every name, publishes no parked status, and blocks record publication and every close stage until restart completes the same protocol. No SC-002 data leaf is unlinked. Import, cleanup, and the retention guard share one exclusive candidate OFD lock. Replacement and live-owner tests cover every name-consuming edge and all importer/cleanup/retention pair orderings. |
| Earlier SC-002 retention prose allowed the private owner to tombstone and delete the whole candidate after terminal-state checks. | Delivery requests, panel records, evidence records, seals, and eligibility history are permanent delivery history; SC-020 requires each applicable wave seal to remain available, and incident evidence is never automatically removed. Deleting the candidate root would erase those retained proofs. | `CandidateRetentionOwner` is a zero-mutation whole-scope guard. Verified orphans remain in the separately owned bounded `evidence-sidecars/sc002/retired` subtree; incident evidence and every request/record/receipt/seal/eligibility/merge artifact remain under the canonical candidate root. No candidate descendant is automatically unlinked, and the root is never renamed, tombstoned, or deleted. Tests poison candidate-root removal and any permanent-history mutation. |
| `ADR-046-provider-device-tpm` section 11.2 names the finalizer `device-tpm/cleanup`. | Committed `packages/d2b-provider-device-tpm/src/lib.rs` and `packages/d2b-contracts/src/v3/device.rs` both expose `device-tpm.d2bus.org/state-preserved`. | Existing code is canon. The exact T604 fixture uses `device-tpm.d2bus.org/state-preserved` while retaining the dossier's stop/wait/delete-process/delete-flush/retain-Volume/clear sequencing. Correcting the upstream dossier is external to this feature-only batch. |
| Feature-local prose treated the W0/W1 record and W2-W4 late remediation as authority to continue despite Constitution Principle VI, while T072 omitted Wave 5's contemporaneous plan-panel predicate. | The committed constitution permits no artifact-local waiver for these gaps, and existing Wave 5 implementation does not prove its historical entry gate. | FR-036 now makes a separate accepted Principle VI constitution amendment that expressly dispositions the W2-W5 plan-panel gap an external prerequisite for every implementation, resume, fix, close, merge, and advance path. T072 requires the exact retained Wave 5 plan-panel receipt to check; no current receipt is claimed, and current remediation remains evidence only. |
| Feature-local Wave 5 recovery prose allowed one binding delivery panel per candidate and a second request after a failed or content-invalidated binding result. | `ADR-046-validation-and-delivery` section 12.3 and `docs/contributing/panel-review.md` require the binding ten-role panel exactly once per wave. Delivery state already retains a Wave 5 `panel-request.json`; content invalidation does not reclassify or erase it. Iterative findings belong only to the nonbinding phase-plan surface. | T220 uses repeatable `/d2b-panel-round plan` phase reviews for scoped pre-close convergence; those rounds create no delivery request or reservation. T219 is non-authorizing until the external owner lands and validates one `Wave5RetainedRequestDispositionV1`. Only `recover-panel-without-new-request` can reach close, and only after a separate complete unanimous ten-role exact-F panel; the disposition cannot waive or supply that result. This feature batch does not authorize another request, seal, or merge; the contract/tooling owner remains external scope. |

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
outstanding ten-role panel request with imported validation evidence - including a
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
After the external amendment lands, Wave 5 may resume only under its stated conditions plus
the T072 disposition and fresh A/P0 plan panel, with B/P re-review after V. Neither current
panel is claimed as the missing contemporaneous Wave 5 receipt.
C1 is not a further exception: Constitution 2.2.0 authorizes its coordinated contract
correction, which is fully assigned to T605. No implementation is claimed, and implementation
remains gated first on FR-036's external amendment, then on the T072 disposition,
pre-T603 A/P0 analysis/plan panel, T603's
validator-and-fragment V/B commit, the post-T603 B/P analysis/plan panel, and the receipt/editor
transition.

### Program-local safety and delivery risks

These rows are not Constitution Principle VI deviations.

| Risk | Why Tracked | Guard and Rejected Alternative |
| --- | --- | --- |
| FR-043 (recovery-point attestation) is tracked program-local, outside the work-item manifest, so the manifest census alone cannot enforce it | FR-043 is locally added and **stricter** than `ADR-046-reset-and-cutover`, which permits proceeding past the rollback boundary without attestation. Creating a manifest work item would require amending that member spec, which re-opens its validation and panel evidence and re-triggers Gate 0. An unqualified "backup exists" assertion permits a partial, old, wrong-host, or unverifiable point to become success-shaped. | Keep it program-local, but close the safety gap at the W7 exit boundary. T548 owns one hermetic validator used unchanged by T580, T555, and T556. It decodes every timestamp through a bounded integer newtype, uses checked bounded expiration arithmetic, requires `previewed <= captured <= verified <= attested <= verifier-now < expires`, independently varies every receipt field and binding including operator and restore-instruction digests, and fails on listing failure, empty discovery, ignored tests, or skip. Before T580 records evidence, the integrator freezes the clean current W7 candidate and exact preview inventory. T580 accepts only one external version 1 record for a verified full-host snapshot or backup covering boot/system state, the active generation, the preview inventory, and preserved identity state. It binds candidate/commit/tree, preview, daily-driver host, operator, and restore instructions; imports only its digest and opaque locator through the existing `EvidenceRecord`; and rejects negative, fractional, future, out-of-range, overflow, stale, expired, or mismatched values. Every close stage invokes the same validator. Expiry before the binding request returns to prebinding convergence and requires fresh evidence plus another nonbinding phase review. Expiry after the wave's one binding request durably fails the close, retains its records, permits no successor request, and requires integrator scope escalation. The external operator-owned backup/snapshot and restore mechanism remains outside this feature; no host implementation is claimed. |
| Pipelined dispatch can create successor rework when a predecessor panel finding invalidates in-flight work | Constitution 2.0.0 expressly authorizes implementation of wave N+1 to begin at 5 of 10 wave-N panel returns plus green integration. It is therefore current policy, not a constitution deviation. | Unanimity, roster, seal ordering, and merge ordering remain unchanged. The successor rebases onto the merged predecessor before its own panel, so no panel reviews a tree built on unreviewed contracts. Strict serialization was rejected because it adds idle time without strengthening the exit gate. FR-050 forbids citing rework as grounds to shorten a panel. |
